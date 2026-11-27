use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use insar_core::{AsfRemoteSearchRequest, AsfSearchQuery, ContractError, ProviderResult};
use insar_provider::asf::cmr::{cmr_entry_to_feature, collection_ids};
use insar_provider::asf::search::{AsfProvider, MetadataTransport};
use serde_json::{json, Map, Value};

#[derive(Default)]
struct ScriptedTransport {
    counts: Mutex<VecDeque<ProviderResult<Option<u64>>>>,
    asf: Mutex<VecDeque<ProviderResult<Value>>>,
    cmr: Mutex<VecDeque<ProviderResult<Value>>>,
    asf_calls: Mutex<Vec<BTreeMap<String, String>>>,
    cmr_calls: Mutex<Vec<Vec<(String, String)>>>,
    cancel_after_asf: Option<Arc<AtomicBool>>,
}

impl MetadataTransport for ScriptedTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.asf_calls.lock().unwrap().push(params.clone());
        let result = self
            .asf
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted ASF response");
        if let Some(flag) = &self.cancel_after_asf {
            flag.store(true, Ordering::SeqCst);
        }
        result
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        self.counts.lock().unwrap().pop_front().unwrap_or(Ok(None))
    }

    fn cmr_json(
        &self,
        params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.cmr_calls.lock().unwrap().push(params.to_vec());
        self.cmr
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted CMR response")
    }
}

fn error() -> ContractError {
    ContractError::new("ASF002", "scripted remote failure", Map::new()).unwrap()
}

fn query(overrides: Value) -> AsfSearchQuery {
    let mut value = json!({
        "version": 1, "dataset": "SENTINEL-1", "bbox": null, "aoi": "",
        "aoi_geojson": null, "start": "", "end": "", "product_type": "SLC",
        "beam_mode": "IW", "polarization": "DV", "orbit_direction": "",
        "relative_orbit": null, "frame": null, "max_results": 10
    });
    value
        .as_object_mut()
        .unwrap()
        .extend(overrides.as_object().unwrap().clone());
    serde_json::from_value(value).unwrap()
}

fn request(query: AsfSearchQuery, allow_cmr_fallback: bool) -> AsfRemoteSearchRequest {
    AsfRemoteSearchRequest {
        query,
        allow_cmr_fallback,
    }
}

fn feature(index: usize, inside: bool) -> Value {
    let (west, south, east, north) = if inside {
        (0.2, 0.2, 0.8, 0.8)
    } else {
        (2.0, 2.0, 3.0, 3.0)
    };
    json!({
        "type": "Feature",
        "properties": {
            "sceneName": format!(
                "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_{:06}_064ABC_{:04X}",
                52_000 + index,
                index
            ),
            "pathNumber": "88", "frameNumber": "456", "flightDirection": "ASCENDING"
        },
        "geometry": {"type": "Polygon", "coordinates": [[
            [west, south], [east, south], [east, north], [west, north], [west, south]
        ]]}
    })
}

fn collection(features: Vec<Value>) -> Value {
    json!({"type": "FeatureCollection", "features": features})
}

fn grd_feature(resolution: &str, polarization: &str, index: usize) -> Value {
    json!({
        "type": "Feature",
        "properties": {
            "sceneName": format!(
                "S1A_IW_GRD{resolution}_1S{polarization}_20240101T100000_20240101T100027_{:06}_064ABC_{:04X}",
                52_000 + index,
                index
            ),
            "pathNumber": "88",
            "frameNumber": "456",
            "flightDirection": "ASCENDING"
        },
        "geometry": null
    })
}

fn make_provider(transport: ScriptedTransport) -> AsfProvider<ScriptedTransport> {
    AsfProvider::with_transport(transport, "2026-07-13T00:00:00Z")
}

#[test]
fn direct_search_counts_normalizes_and_reports_reliable_metadata() {
    let transport = ScriptedTransport::default();
    transport.counts.lock().unwrap().push_back(Ok(Some(27)));
    transport
        .asf
        .lock()
        .unwrap()
        .push_back(Ok(collection(vec![feature(0, true)])));
    let provider = make_provider(transport);

    let response = provider
        .search(&request(query(json!({})), true), &|| false)
        .expect("direct search");

    assert_eq!(response.scenes.len(), 1);
    assert_eq!(response.scenes[0].path, Some(88));
    assert_eq!(response.scenes[0].frame, Some(456));
    assert_eq!(
        response.scenes[0].orbit_direction.as_deref(),
        Some("ASCENDING")
    );
    assert_eq!(response.stats.total_count, Some(27));
    assert_eq!(response.stats.candidate_count, 27);
    assert_eq!(response.stats.source, "ASF");
    assert_eq!(response.stats.cmr_enriched, None);
}

#[test]
fn detailed_grd_search_filters_by_filename_resolution_and_polarization() {
    let candidates = [
        ("GRD_HD", "H", "DV"),
        ("GRD_HS", "H", "SV"),
        ("GRD_MD", "M", "DH"),
        ("GRD_MS", "M", "SH"),
        ("GRD_FD", "F", "DV"),
    ];
    for (expected_level, _, _) in candidates {
        let transport = ScriptedTransport::default();
        transport.asf.lock().unwrap().push_back(Ok(collection(
            candidates
                .iter()
                .enumerate()
                .map(|(index, (_, resolution, polarization))| {
                    grd_feature(resolution, polarization, index)
                })
                .collect(),
        )));
        let provider = make_provider(transport);

        let response = provider
            .search(
                &request(
                    query(json!({
                        "product_type": expected_level,
                        "beam_mode": "",
                        "polarization": "",
                        "max_results": 10
                    })),
                    false,
                ),
                &|| false,
            )
            .unwrap_or_else(|error| panic!("{expected_level}: {error}"));

        assert_eq!(response.scenes.len(), 1, "{expected_level}");
        let params = provider.transport().asf_calls.lock().unwrap();
        assert_eq!(params[0]["processingLevel"], expected_level);
    }
}

#[test]
fn detailed_grd_levels_share_the_reviewed_grd_cmr_collections() {
    let broad = collection_ids("GRD").expect("broad GRD collections");
    for level in ["GRD_HD", "GRD_HS", "GRD_MD", "GRD_MS", "GRD_FD"] {
        assert_eq!(collection_ids(level), Some(broad), "{level}");
    }
}

#[test]
fn aoi_underfill_refills_without_changing_the_requested_limit() {
    let transport = ScriptedTransport::default();
    transport.counts.lock().unwrap().push_back(Ok(Some(500)));
    transport.asf.lock().unwrap().extend([
        Ok(collection(
            (0..120).map(|index| feature(index, index < 27)).collect(),
        )),
        Ok(collection(
            (0..240).map(|index| feature(index, index < 30)).collect(),
        )),
    ]);
    let provider = make_provider(transport);
    let query = query(json!({
        "bbox": {"west": -10.0, "east": 10.0, "south": -10.0, "north": 10.0, "crs": "EPSG:4326"},
        "aoi_geojson": {"type": "Polygon", "coordinates": [[[0,0],[1,0],[1,1],[0,1],[0,0]]]},
        "max_results": 30
    }));

    let response = provider.search(&request(query, true), &|| false).unwrap();

    assert_eq!(response.scenes.len(), 30);
    assert_eq!(response.stats.requested_limit, 30);
    assert_eq!(response.stats.query_limit, 240);
    let limits = provider
        .transport()
        .asf_calls
        .lock()
        .unwrap()
        .iter()
        .map(|params| params["maxResults"].clone())
        .collect::<Vec<_>>();
    assert_eq!(limits, ["120", "240"]);
}

#[test]
fn large_truncated_direct_search_switches_to_date_windows() {
    let transport = ScriptedTransport::default();
    transport.counts.lock().unwrap().push_back(Ok(Some(6_000)));
    transport.asf.lock().unwrap().extend([
        Ok(collection(vec![])),
        Ok(collection(vec![feature(1, true), feature(0, true)])),
    ]);
    let provider = make_provider(transport);
    let query = query(json!({
        "bbox": {"west": 0.0, "east": 1.0, "south": 0.0, "north": 1.0, "crs": "EPSG:4326"},
        "start": "2024-01-01", "end": "2024-01-02", "max_results": 2500
    }));

    let response = provider.search(&request(query, false), &|| false).unwrap();

    assert_eq!(response.stats.source, "ASF_BATCH");
    assert_eq!(response.scenes.len(), 2);
    assert!(response.scenes[0].scene_id.contains("052001"));
    assert!(response.scenes[1].scene_id.contains("052000"));
    let calls = provider.transport().asf_calls.lock().unwrap();
    assert_eq!(calls[0]["maxResults"], "3500");
    assert_eq!(calls[1]["maxResults"], "2000");
    assert_eq!(calls[1]["start"], "2024-01-01T00:00:00Z");
    assert_eq!(calls[1]["end"], "2024-01-02T23:59:59Z");
}

#[test]
fn cmr_fallback_is_explicit_enriched_when_bounded_and_honest_when_large() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/v3_asf/asf_cmr_cases.json");
    let fixture: Value = serde_json::from_str(&fs::read_to_string(fixture_path).unwrap()).unwrap();
    let entry = fixture["cases"][0]["entry"].clone();

    let transport = ScriptedTransport::default();
    transport
        .asf
        .lock()
        .unwrap()
        .extend([Err(error()), Ok(collection(vec![feature(0, true)]))]);
    transport
        .cmr
        .lock()
        .unwrap()
        .push_back(Ok(json!({"feed": {"entry": [entry.clone()]}})));
    let provider = make_provider(transport);
    let response = provider
        .search(
            &request(
                query(json!({
                    "max_results": 1,
                    "beam_mode": "EW",
                    "polarization": "DH"
                })),
                true,
            ),
            &|| false,
        )
        .unwrap();
    assert_eq!(response.stats.source, "CMR");
    assert_eq!(response.stats.cmr_enriched, Some(true));
    assert_eq!(response.scenes.len(), 1);

    let transport = ScriptedTransport::default();
    transport.counts.lock().unwrap().push_back(Ok(None));
    transport.asf.lock().unwrap().push_back(Err(error()));
    transport
        .cmr
        .lock()
        .unwrap()
        .push_back(Ok(json!({"feed": {"entry": [entry]}})));
    let provider = make_provider(transport);
    let response = provider
        .search(
            &request(
                query(json!({
                    "max_results": 600,
                    "beam_mode": "EW",
                    "polarization": "DH",
                    "orbit_direction": "ASCENDING"
                })),
                true,
            ),
            &|| false,
        )
        .unwrap();
    assert_eq!(response.stats.cmr_enriched, Some(false));
    assert_eq!(
        response.scenes.len(),
        1,
        "large CMR does not claim direction parity"
    );
}

#[test]
fn bounded_grd_cmr_enrichment_queries_base_and_product_identifiers() {
    let scene = "S1A_IW_GRDH_1SDV_20240101T100000_20240101T100027_052000_064ABC_1234";
    let entry = json!({
        "producer_granule_id": scene,
        "links": []
    });
    let transport = ScriptedTransport::default();
    transport.asf.lock().unwrap().extend([
        Err(error()),
        Ok(collection(vec![json!({
            "type": "Feature",
            "properties": {"sceneName": scene, "pathNumber": 88},
            "geometry": null
        })])),
    ]);
    transport
        .cmr
        .lock()
        .unwrap()
        .push_back(Ok(json!({"feed": {"entry": [entry.clone()]}})));
    let raw = cmr_entry_to_feature(entry.as_object().unwrap()).expect("valid GRD CMR entry");
    let normalized = insar_core::normalize_asf_features(&insar_core::AsfNormalizeRequest {
        query: query(json!({"product_type": "GRD", "max_results": null})),
        features: vec![raw],
    })
    .unwrap();
    assert_eq!(normalized.scenes.len(), 1, "{:?}", normalized.errors);
    let provider = make_provider(transport);

    let response = provider
        .search(
            &request(
                query(json!({
                    "product_type": "GRD",
                    "max_results": 1
                })),
                true,
            ),
            &|| false,
        )
        .unwrap();

    assert_eq!(response.scenes.len(), 1);
    let calls = provider.transport().asf_calls.lock().unwrap();
    let ids = &calls[1]["granule_list"];
    assert!(ids.contains(scene));
    assert!(ids.contains(&format!("{scene}-GRD")));
}

#[test]
fn disabled_fallback_and_post_request_cancellation_are_not_hidden() {
    let transport = ScriptedTransport::default();
    transport.asf.lock().unwrap().push_back(Err(error()));
    let provider = make_provider(transport);
    let error = provider
        .search(&request(query(json!({"max_results": 1})), false), &|| false)
        .unwrap_err();
    assert_eq!(error.code, "ASF002");
    assert!(provider.transport().cmr_calls.lock().unwrap().is_empty());

    let flag = Arc::new(AtomicBool::new(false));
    let transport = ScriptedTransport {
        cancel_after_asf: Some(Arc::clone(&flag)),
        ..ScriptedTransport::default()
    };
    transport
        .asf
        .lock()
        .unwrap()
        .push_back(Ok(collection(vec![feature(0, true)])));
    let provider = make_provider(transport);
    let error = provider
        .search(&request(query(json!({"max_results": 1})), true), &|| {
            flag.load(Ordering::SeqCst)
        })
        .unwrap_err();
    assert_eq!(error.code, "DL001");
}

#[test]
fn cmr_adapter_matches_python_v219_fixture() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/v3_asf/asf_cmr_cases.json");
    let fixture: Value = serde_json::from_str(&fs::read_to_string(fixture_path).unwrap()).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let feature = cmr_entry_to_feature(case["entry"].as_object().unwrap()).unwrap();
        let query = query(json!({"max_results": null, "beam_mode": "", "polarization": ""}));
        let normalized = insar_core::normalize_asf_features(&insar_core::AsfNormalizeRequest {
            query,
            features: vec![feature],
        })
        .unwrap();
        assert_eq!(
            normalized.scenes.len(),
            1,
            "{}: {:?}",
            case["name"],
            normalized.errors
        );
        let actual = serde_json::to_value(&normalized.scenes[0]).unwrap();
        let expected = case["expected"].as_object().unwrap();
        // This fixture owns v2.1.9 fields; optional v3 metadata has focused contract tests.
        for (key, expected_value) in expected {
            assert_eq!(
                actual.get(key),
                Some(expected_value),
                "{}: {key}",
                case["name"]
            );
        }
    }
}
