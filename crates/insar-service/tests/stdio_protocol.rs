use std::collections::BTreeSet;
use std::fs;
use std::io::{BufReader, Cursor, Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    AsfCacheDecisionKind, AsfCacheDecisionResponse, AsfNormalizeResponse, AsfRemoteSearchRequest,
    AsfRemoteSearchResponse, AsfSearchStats, ContractError, OrbitMatchReport, OrbitParseResponse,
    ProviderResult, SarSearchCountRequest, SarSearchCountResponse, SidecarOperation,
    SidecarRequest, SidecarResponse, V3_CONTRACT_VERSION, V3_PROTOCOL_MINOR, V3_PROTOCOL_MIN_MINOR,
};
use insar_service::{
    serve, serve_with_limits, CancellationRegistry, DispatchClass, Dispatcher, LiveAsfSearch,
    RequestDispatcher, MAX_MESSAGE_BYTES,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

fn request(request_id: &str, operation: SidecarOperation, payload: Value) -> SidecarRequest {
    SidecarRequest {
        protocol_major: 1,
        protocol_minor: 4,
        request_id: request_id.to_string(),
        operation,
        payload: payload.as_object().cloned().unwrap_or_default(),
    }
}

fn response_lines(output: &[u8]) -> Vec<SidecarResponse> {
    String::from_utf8(output.to_vec())
        .expect("protocol output is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout line is a response"))
        .collect()
}

struct BlockingDispatcher {
    health_seen: AtomicBool,
    work_finished_after_health: AtomicBool,
    shutting_down: AtomicBool,
}

impl BlockingDispatcher {
    fn new() -> Self {
        Self {
            health_seen: AtomicBool::new(false),
            work_finished_after_health: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
        }
    }

    fn response(request_id: String, result: Value) -> SidecarResponse {
        SidecarResponse {
            protocol_major: 1,
            protocol_minor: 4,
            request_id,
            ok: true,
            result: result.as_object().cloned().unwrap_or_default(),
            error: None,
            terminal: true,
        }
    }
}

impl RequestDispatcher for BlockingDispatcher {
    fn dispatch(&self, request: SidecarRequest) -> SidecarResponse {
        match request.operation {
            SidecarOperation::Health => {
                self.health_seen.store(true, Ordering::SeqCst);
                Self::response(request.request_id, json!({"status": "healthy"}))
            }
            SidecarOperation::Shutdown => {
                self.shutting_down.store(true, Ordering::SeqCst);
                Self::response(request.request_id, json!({"status": "shutting_down"}))
            }
            _ => {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while !self.health_seen.load(Ordering::SeqCst) {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "control request was blocked"
                    );
                    std::thread::yield_now();
                }
                self.work_finished_after_health
                    .store(true, Ordering::SeqCst);
                Self::response(request.request_id, json!({"status": "worked"}))
            }
        }
    }

    fn classify(&self, operation: &SidecarOperation) -> DispatchClass {
        match operation {
            SidecarOperation::Health | SidecarOperation::Cancel | SidecarOperation::Shutdown => {
                DispatchClass::Control
            }
            _ => DispatchClass::Work,
        }
    }

    fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }
}

#[test]
fn dispatcher_negotiates_supported_versions_and_reports_health() {
    for minor in V3_PROTOCOL_MIN_MINOR..=V3_PROTOCOL_MINOR {
        let dispatcher = Dispatcher::new();
        let mut handshake = request(
            &format!("handshake-{minor}"),
            SidecarOperation::Handshake,
            json!({}),
        );
        handshake.protocol_minor = minor;

        let response = dispatcher.dispatch(handshake);

        assert!(response.ok);
        assert_eq!(response.result["protocol_major"], 1);
        assert_eq!(response.result["protocol_minor"], V3_PROTOCOL_MINOR);
        assert_eq!(
            response.result["minimum_protocol_minor"],
            V3_PROTOCOL_MIN_MINOR
        );
        assert_eq!(response.result["contract_version"], V3_CONTRACT_VERSION);
        let mut expected_capabilities = if minor == 4 {
            json!([
                "orbit_parse",
                "orbit_match",
                "asf_normalize",
                "asf_cache_decide",
                "asf_search"
            ])
        } else if minor == 5 {
            json!([
                "orbit_parse",
                "orbit_match",
                "asf_normalize",
                "asf_cache_decide",
                "asf_search",
                "store_search_batch",
                "store_create_task",
                "store_update_task",
                "store_append_task",
                "store_archive_task",
                "store_delete_history",
                "store_restore"
            ])
        } else if minor == 6 {
            json!([
                "orbit_parse",
                "orbit_match",
                "asf_normalize",
                "asf_cache_decide",
                "asf_search",
                "store_search_batch",
                "store_create_task",
                "store_update_task",
                "store_append_task",
                "store_archive_task",
                "store_delete_history",
                "store_restore",
                "store_update_workbench"
            ])
        } else if minor == 7 {
            json!([
                "orbit_parse",
                "orbit_match",
                "asf_normalize",
                "asf_cache_decide",
                "asf_search",
                "store_search_batch",
                "store_create_task",
                "store_update_task",
                "store_append_task",
                "store_archive_task",
                "store_delete_history",
                "store_restore",
                "store_update_workbench",
                "asf_search_cached"
            ])
        } else if minor == 8 {
            json!([
                "orbit_parse",
                "orbit_match",
                "asf_normalize",
                "asf_cache_decide",
                "asf_search",
                "store_search_batch",
                "store_create_task",
                "store_update_task",
                "store_append_task",
                "store_archive_task",
                "store_delete_history",
                "store_restore",
                "store_update_workbench",
                "asf_search_cached",
                "sar_search",
                "provider_capabilities"
            ])
        } else {
            json!([
                "orbit_parse",
                "orbit_match",
                "asf_normalize",
                "asf_cache_decide",
                "asf_search",
                "store_search_batch",
                "store_create_task",
                "store_update_task",
                "store_append_task",
                "store_archive_task",
                "store_delete_history",
                "store_restore",
                "store_update_workbench",
                "asf_search_cached",
                "sar_search",
                "provider_capabilities",
                "sar_count"
            ])
        };
        if minor >= 10 {
            expected_capabilities
                .as_array_mut()
                .expect("capabilities are an array")
                .push(json!("store_update_task_view"));
        }
        if minor >= 14 {
            expected_capabilities
                .as_array_mut()
                .expect("capabilities are an array")
                .push(json!("asset_execution_policy_v1"));
        }
        if minor >= 15 {
            expected_capabilities
                .as_array_mut()
                .expect("capabilities are an array")
                .push(json!("asf_cached_preview_metadata_v1"));
        }
        if minor >= 16 {
            expected_capabilities
                .as_array_mut()
                .expect("capabilities are an array")
                .push(json!("store_startup_readiness_v1"));
        }
        if minor >= 19 {
            expected_capabilities
                .as_array_mut()
                .expect("capabilities are an array")
                .extend([
                    json!("supports_catalog_result_limit_v1"),
                    json!("supports_application_search_progress_v2"),
                ]);
        }
        assert_eq!(response.result["capabilities"], expected_capabilities);
        assert!(!response.result["capabilities"]
            .as_array()
            .expect("capabilities are an array")
            .contains(&json!("resolved_asset_transfer_v1")));
        assert!(response.validate().is_ok());
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let store_path = std::env::temp_dir().join(format!(
        "insar-service-health-{}-{nonce}.sqlite3",
        std::process::id()
    ));
    let dispatcher = Dispatcher::try_new_with_store_path(store_path.clone()).unwrap();
    let mut health_request = request(
        "健康-001",
        SidecarOperation::Health,
        json!({"probe": "startup", "require_store": true}),
    );
    health_request.protocol_minor = 16;
    let health = dispatcher.dispatch(health_request);
    assert!(health.ok);
    assert_eq!(health.request_id, "健康-001");
    assert_eq!(
        health.result,
        Map::from_iter([("status".to_string(), json!("healthy"))])
    );
    drop(dispatcher);
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", store_path.display(), suffix));
    }
}

#[test]
fn dispatcher_classifies_control_and_work_operations() {
    let dispatcher = Dispatcher::new();
    for operation in [
        SidecarOperation::Handshake,
        SidecarOperation::Health,
        SidecarOperation::Cancel,
        SidecarOperation::Shutdown,
    ] {
        assert_eq!(dispatcher.classify(&operation), DispatchClass::Control);
    }
    for operation in [
        SidecarOperation::OrbitParse,
        SidecarOperation::OrbitMatch,
        SidecarOperation::AsfNormalize,
        SidecarOperation::AsfCacheDecide,
        SidecarOperation::AsfSearch,
        SidecarOperation::SarCount,
    ] {
        assert_eq!(dispatcher.classify(&operation), DispatchClass::Work);
    }
}

#[test]
fn dispatcher_exposes_production_typed_sources_and_routes_burst_through_rust() {
    let dispatcher = Dispatcher::new();
    let mut capabilities_request = request(
        "sar-capabilities",
        SidecarOperation::ProviderCapabilities,
        json!({}),
    );
    capabilities_request.protocol_minor = 8;
    let capabilities = dispatcher.dispatch(capabilities_request);
    assert!(capabilities.ok);
    assert_eq!(capabilities.result["provider_id"], "asf.catalog");
    // ERS, JERS-1 and RADARSAT-1 are retired (ASF removed them, d8a8bfd8).
    assert_eq!(capabilities.result["datasets"].as_array().unwrap().len(), 4);
    let sources = capabilities.result["sources"].as_array().unwrap();
    let alos1 = sources
        .iter()
        .find(|source| source["selector"] == "ALOS PALSAR")
        .unwrap();
    assert_eq!(alos1["search_kind"], "legacy_asf_search");
    assert_eq!(alos1["filters"]["legacy_dataset"], "ALOS PALSAR");
    let alos2 = sources
        .iter()
        .find(|source| source["selector"] == "ALOS-2")
        .unwrap();
    assert_eq!(alos2["search_kind"], "typed_catalog_profile");
    assert_eq!(
        alos2["logical_dataset"],
        "sar.alos-2.palsar-2.scansar.l1.1.v1"
    );
    assert_eq!(alos2["native_collection_id"], "C3315903479-ASF");
    assert_eq!(alos2["available"], true);
    assert_eq!(alos2["disabled_reason"], Value::Null);
    let nisar = sources
        .iter()
        .find(|source| source["selector"] == "NISAR")
        .unwrap();
    assert_eq!(nisar["search_kind"], "typed_catalog_profile");
    assert_eq!(nisar["logical_dataset"], "sar.nisar.l.rslc.provisional.v1");
    assert_eq!(nisar["native_collection_id"], "C2853145197-ASF");
    assert_eq!(nisar["available"], true);
    assert_eq!(nisar["disabled_reason"], Value::Null);
    let burst_capabilities = capabilities.result["datasets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|dataset| dataset["dataset"] == "SLC-BURST")
        .unwrap();
    assert_eq!(burst_capabilities["supports_burst"], true);

    let dispatcher = Dispatcher::with_asf_search(Arc::new(SarWireFixture));

    let mut burst_request = request(
        "sar-burst-official-adapter",
        SidecarOperation::SarSearch,
        json!({
            "query": {
                "dataset": "SLC-BURST",
                "burst": {"full_burst_id": "048_101784_IW2"}
            }
        }),
    );
    burst_request.protocol_minor = 8;
    let burst = dispatcher.dispatch(burst_request);
    assert!(burst.ok);

    let mut burst_count = request(
        "sar-burst-rust-count",
        SidecarOperation::SarCount,
        json!({
            "query": {
                "dataset": "SLC-BURST",
                "burst": {"full_burst_id": "048_101784_IW2"}
            }
        }),
    );
    burst_count.protocol_minor = 9;
    let count = dispatcher.dispatch(burst_count);
    assert!(count.ok);
    assert_eq!(count.result["total_count"], 1);
}

#[test]
fn dispatcher_rejects_unsupported_versions_and_unknown_cancellation() {
    let dispatcher = Dispatcher::new();
    let mut incompatible = request("bad-version", SidecarOperation::Handshake, json!({}));
    incompatible.protocol_major = 2;
    let rejected = dispatcher.dispatch(incompatible);
    assert!(!rejected.ok);
    assert_eq!(
        rejected.error.unwrap().code,
        "V3_PROTOCOL_VERSION_UNSUPPORTED"
    );
    let mut too_old = request("old-version", SidecarOperation::Handshake, json!({}));
    too_old.protocol_minor = 2;
    assert_eq!(
        dispatcher.dispatch(too_old).error.unwrap().code,
        "V3_PROTOCOL_VERSION_UNSUPPORTED"
    );

    let cancelled = dispatcher.dispatch(request(
        "cancel-001",
        SidecarOperation::Cancel,
        json!({"request_id": "not-running"}),
    ));
    assert!(cancelled.ok);
    assert_eq!(cancelled.result["cancelled"], false);
}

#[test]
fn dispatcher_executes_typed_asf_operations() {
    let dispatcher = Dispatcher::new();
    let normalized = dispatcher.dispatch(request(
        "asf-normalize",
        SidecarOperation::AsfNormalize,
        json!({
            "query": {
                "version": 1, "dataset": "SENTINEL-1", "bbox": null, "aoi": "",
                "aoi_geojson": null, "start": "", "end": "", "product_type": "SLC",
                "beam_mode": "IW", "polarization": "DV", "orbit_direction": "",
                "relative_orbit": null, "frame": null, "max_results": null
            },
            "features": [{
                "type": "Feature",
                "properties": {
                    "sceneName": "S1B_IW_SLC__1SDV_20240206T100000_20240206T100027_034175_064BBB_ABCD"
                },
                "geometry": null
            }]
        }),
    ));
    assert!(normalized.ok);
    let normalized: AsfNormalizeResponse =
        serde_json::from_value(Value::Object(normalized.result)).unwrap();
    assert_eq!(normalized.scenes.len(), 1);
    assert_eq!(normalized.scenes[0].orbit_direction, None);

    let cached = dispatcher.dispatch(request(
        "asf-cache",
        SidecarOperation::AsfCacheDecide,
        json!({
            "query": {
                "version": 1, "dataset": "SENTINEL-1", "bbox": null, "aoi": "",
                "aoi_geojson": null, "start": "", "end": "", "product_type": "SLC",
                "beam_mode": "IW", "polarization": "DV", "orbit_direction": "",
                "relative_orbit": null, "frame": null, "max_results": 1
            },
            "entries": []
        }),
    ));
    assert!(cached.ok);
    let cached: AsfCacheDecisionResponse =
        serde_json::from_value(Value::Object(cached.result)).unwrap();
    assert_eq!(cached.kind, AsfCacheDecisionKind::Miss);
}

struct FixtureLiveSearch {
    calls: AtomicUsize,
}

struct SarWireFixture;

impl LiveAsfSearch for SarWireFixture {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        unreachable!("the SAR wire fixture only services SarSearch")
    }

    fn search_sar(
        &self,
        _request: &insar_core::SarRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<insar_core::SarRemoteSearchResponse> {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/v3_contracts/protocol_1_16_sar_provider_result.json"
        ))
        .map_err(|error| {
            ContractError::new("FIXTURE_SAR_WIRE", error.to_string(), Map::new()).unwrap()
        })
    }

    fn count_sar(
        &self,
        _request: &SarSearchCountRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarSearchCountResponse> {
        Ok(SarSearchCountResponse {
            total_count: Some(1),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenProviderAssetV313 {
    asset_key: Option<String>,
    stable_asset_key: Option<String>,
    access_descriptor: Option<Value>,
    role: String,
    url: Option<String>,
    file_name: Option<String>,
    media_type: Option<String>,
    size_bytes: Option<i64>,
}

#[test]
fn protocol_1_16_sar_wire_projects_catalogue_only_asset_fields_for_old_readers() {
    let dispatcher = Dispatcher::with_asf_search(Arc::new(SarWireFixture));
    let mut request = request(
        "sar-wire-compat",
        SidecarOperation::SarSearch,
        json!({"query": {"dataset": "SENTINEL-1"}}),
    );
    request.protocol_minor = 16;

    let response = dispatcher.dispatch(request);

    assert!(response.ok);
    let wire = Value::Object(response.result.clone());
    assert!(wire.pointer("/products/0/assets/0/native_roles").is_none());
    assert_eq!(
        wire.pointer("/products/0/native_metadata/native_roles"),
        Some(&json!(["preserve-product-metadata-role"]))
    );
    assert_eq!(
        wire.pointer("/products/0/native_metadata/nested/native_roles"),
        Some(&json!(["preserve-nested-metadata-role"]))
    );
    assert!(!wire.to_string().contains("oauth_bearer_https"));
    assert!(!wire.to_string().contains("CatalogSearchResponse"));

    let frozen_asset: FrozenProviderAssetV313 = serde_json::from_value(
        wire.pointer("/products/0/assets/0")
            .expect("fixture response has one asset")
            .clone(),
    )
    .expect("frozen 3.13 asset reader rejects no unknown fields");
    assert_eq!(frozen_asset.asset_key.as_deref(), Some("fixture-asset"));
    assert_eq!(
        frozen_asset.stable_asset_key.as_deref(),
        Some("ASF::SENTINEL-1::fixture-sar::ASSET::fixture-asset")
    );
    assert!(frozen_asset.access_descriptor.is_none());
    assert_eq!(frozen_asset.role, "primary");
    assert_eq!(
        frozen_asset.url.as_deref(),
        Some("https://example.invalid/fixture-sar.zip")
    );
    assert_eq!(frozen_asset.file_name.as_deref(), Some("fixture-sar.zip"));
    assert_eq!(frozen_asset.media_type.as_deref(), Some("application/zip"));
    assert_eq!(frozen_asset.size_bytes, Some(1));

    let current: insar_core::SarRemoteSearchResponse =
        serde_json::from_value(wire).expect("current reader accepts the old-compatible projection");
    assert_eq!(
        current.products[0].assets[0].native_roles,
        Vec::<String>::new()
    );
}

impl FixtureLiveSearch {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl LiveAsfSearch for FixtureLiveSearch {
    fn search(
        &self,
        request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let requested = request.query.max_results.unwrap_or(500);
        Ok(AsfRemoteSearchResponse {
            scenes: Vec::new(),
            errors: Vec::new(),
            stats: AsfSearchStats {
                requested_limit: requested,
                query_limit: requested,
                metadata_request_count: None,
                total_count: Some(0),
                candidate_count: 0,
                returned_count: 0,
                source: "ASF".to_string(),
                cmr_enriched: None,
                matched_count: None,
                fetched_record_count: None,
                replayed_record_count: None,
                mapped_record_count: None,
                mapping_error_count: None,
                aggregated_asset_record_count: None,
                unique_provider_product_count: None,
                asset_count: None,
                identity_conflict_count: None,
                page_size: None,
                truncated: None,
                item_errors: Vec::new(),
                warnings: Vec::new(),
            },
        })
    }
}

fn live_search_payload() -> Value {
    json!({
        "query": {
            "version": 1, "dataset": "SENTINEL-1", "bbox": null, "aoi": "",
            "aoi_geojson": null, "start": "", "end": "", "product_type": "SLC",
            "beam_mode": "IW", "polarization": "DV", "orbit_direction": "",
            "relative_orbit": null, "frame": null, "max_results": 1
        },
        "allow_cmr_fallback": true
    })
}

#[test]
fn dispatcher_exposes_typed_live_search_on_protocol_1_4_compatibility() {
    let search = Arc::new(FixtureLiveSearch::new());
    let dispatcher = Dispatcher::with_asf_search(search.clone());
    let response = dispatcher.dispatch(request(
        "asf-search",
        SidecarOperation::AsfSearch,
        live_search_payload(),
    ));

    assert!(response.ok);
    let response: AsfRemoteSearchResponse =
        serde_json::from_value(Value::Object(response.result)).unwrap();
    assert_eq!(response.stats.source, "ASF");
    assert_eq!(search.calls.load(Ordering::SeqCst), 1);

    let mut old_request = request(
        "asf-search-old-protocol",
        SidecarOperation::AsfSearch,
        live_search_payload(),
    );
    old_request.protocol_minor = 3;
    let rejected = dispatcher.dispatch(old_request);
    assert!(!rejected.ok);
    assert_eq!(
        rejected.error.expect("capability error").code,
        "V3_PROTOCOL_VERSION_UNSUPPORTED"
    );
    assert_eq!(search.calls.load(Ordering::SeqCst), 1);
}

struct WaitingLiveSearch {
    entered: Arc<AtomicBool>,
}

impl LiveAsfSearch for WaitingLiveSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        self.entered.store(true, Ordering::SeqCst);
        while !cancelled() {
            std::thread::yield_now();
        }
        Err(ContractError::new("DL001", "ASF 检索已停止", Map::new()).unwrap())
    }
}

#[test]
fn dispatcher_cancel_reaches_a_running_live_search() {
    let entered = Arc::new(AtomicBool::new(false));
    let dispatcher = Arc::new(Dispatcher::with_asf_search(Arc::new(WaitingLiveSearch {
        entered: entered.clone(),
    })));
    let worker_dispatcher = dispatcher.clone();
    let worker = std::thread::spawn(move || {
        worker_dispatcher.dispatch(request(
            "running-asf-search",
            SidecarOperation::AsfSearch,
            live_search_payload(),
        ))
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !entered.load(Ordering::SeqCst) {
        assert!(std::time::Instant::now() < deadline, "search did not start");
        std::thread::yield_now();
    }

    let cancelled = dispatcher.dispatch(request(
        "cancel-running-search",
        SidecarOperation::Cancel,
        json!({"request_id": "running-asf-search"}),
    ));
    let result = worker.join().expect("search worker joins");

    assert_eq!(cancelled.result["cancelled"], true);
    assert!(!result.ok);
    assert_eq!(result.error.unwrap().code, "DL001");
}

#[test]
fn dispatcher_executes_typed_orbit_operations() {
    let dispatcher = Dispatcher::new();
    let parsed = dispatcher.dispatch(request(
        "orbit-parse",
        SidecarOperation::OrbitParse,
        json!({
            "file_names": [
                "S1A_OPER_AUX_POEORB_OPOD_20240102T120000_V20240101T225942_20240103T005942.EOF"
            ]
        }),
    ));
    assert!(parsed.ok);
    let parsed: OrbitParseResponse = serde_json::from_value(Value::Object(parsed.result)).unwrap();
    assert!(parsed.errors.is_empty());

    let matched = dispatcher.dispatch(request(
        "orbit-match",
        SidecarOperation::OrbitMatch,
        json!({
            "scenes": [{
                "scene_id": "service-scene",
                "platform": "S1A",
                "acquisition_unix_seconds": 1704196800
            }],
            "orbit_files": parsed.orbit_files
        }),
    ));
    assert!(matched.ok);
    let matched: OrbitMatchReport = serde_json::from_value(Value::Object(matched.result)).unwrap();
    assert_eq!(matched.matched, 1);
}

#[test]
fn cancellation_registry_sets_and_removes_request_flags() {
    let registry = CancellationRegistry::default();
    let flag = registry.register("work-001");
    assert!(!flag.load(Ordering::SeqCst));
    assert!(registry.cancel("work-001"));
    assert!(flag.load(Ordering::SeqCst));
    registry.finish("work-001");
    assert!(!registry.cancel("work-001"));
}

struct ChunkedReader {
    inner: Cursor<Vec<u8>>,
    chunk_size: usize,
}

impl Read for ChunkedReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let limit = buffer.len().min(self.chunk_size);
        self.inner.read(&mut buffer[..limit])
    }
}

#[test]
fn serve_handles_fragmented_unicode_and_multiple_messages() {
    let requests = [
        request("健康-分片", SidecarOperation::Health, json!({})),
        request("关闭", SidecarOperation::Shutdown, json!({})),
    ];
    let input = requests
        .iter()
        .map(|item| serde_json::to_string(item).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let reader = BufReader::with_capacity(
        2,
        ChunkedReader {
            inner: Cursor::new(input.into_bytes()),
            chunk_size: 3,
        },
    );
    let mut output = Vec::new();

    serve(reader, &mut output, &Dispatcher::new()).unwrap();

    let responses = response_lines(&output);
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].request_id, "健康-分片");
    assert_eq!(responses[1].request_id, "关闭");
    assert!(responses.iter().all(|item| item.terminal));
}

#[test]
fn concurrent_serve_keeps_control_requests_responsive_during_work() {
    let requests = [
        request("blocked-work", SidecarOperation::AsfNormalize, json!({})),
        request("health-during-work", SidecarOperation::Health, json!({})),
        request("shutdown-after-work", SidecarOperation::Shutdown, json!({})),
    ];
    let input = requests
        .iter()
        .map(|item| serde_json::to_string(item).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let dispatcher = BlockingDispatcher::new();
    let mut output = Vec::new();

    serve_with_limits(
        BufReader::new(Cursor::new(input)),
        &mut output,
        &dispatcher,
        1,
        1,
    )
    .unwrap();

    assert!(dispatcher.health_seen.load(Ordering::SeqCst));
    assert!(dispatcher.work_finished_after_health.load(Ordering::SeqCst));
    let responses = response_lines(&output);
    assert_eq!(responses.len(), 3);
    assert!(responses.iter().all(|response| response.ok));
    assert_eq!(
        responses
            .iter()
            .map(|response| response.request_id.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["blocked-work", "health-during-work", "shutdown-after-work"])
    );
}

#[test]
fn concurrent_serve_rejects_saturated_work_without_blocking_health() {
    let requests = [
        request("work-1", SidecarOperation::AsfNormalize, json!({})),
        request("work-2", SidecarOperation::AsfNormalize, json!({})),
        request("work-3", SidecarOperation::AsfNormalize, json!({})),
        request("health-after-full", SidecarOperation::Health, json!({})),
        request("shutdown-after-full", SidecarOperation::Shutdown, json!({})),
    ];
    let input = requests
        .iter()
        .map(|item| serde_json::to_string(item).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let dispatcher = BlockingDispatcher::new();
    let mut output = Vec::new();

    serve_with_limits(
        BufReader::new(Cursor::new(input)),
        &mut output,
        &dispatcher,
        1,
        1,
    )
    .unwrap();

    let responses = response_lines(&output);
    assert_eq!(responses.len(), 5);
    assert!(responses.iter().any(|response| {
        response.error.as_ref().map(|error| error.code.as_str()) == Some("V3_SIDECAR_BUSY")
    }));
    assert!(responses.iter().any(|response| {
        response.request_id == "health-after-full" && response.result["status"] == "healthy"
    }));
}

#[test]
fn serve_returns_bounded_errors_for_malformed_and_oversized_lines() {
    let shutdown = serde_json::to_string(&request(
        "shutdown-after-errors",
        SidecarOperation::Shutdown,
        json!({}),
    ))
    .unwrap();
    let mut input = b"\n{\"protocol_major\":1,\"protocol_minor\":1,\"request_id\":\"unknown-op\",\"operation\":\"download\",\"payload\":{}}\n{not-json}\n".to_vec();
    let oversized_prefix =
        br#"{"protocol_major":1,"protocol_minor":1,"request_id":"oversized-workbench","operation":"health","payload":{"padding":"#;
    input.extend_from_slice(oversized_prefix);
    input.extend(std::iter::repeat_n(
        b'x',
        MAX_MESSAGE_BYTES + 1 - oversized_prefix.len(),
    ));
    input.extend_from_slice(br#""}}"#);
    input.push(b'\n');
    input.extend_from_slice(shutdown.as_bytes());
    input.push(b'\n');
    let mut output = Vec::new();

    serve(
        BufReader::new(Cursor::new(input)),
        &mut output,
        &Dispatcher::new(),
    )
    .unwrap();

    let responses = response_lines(&output);
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0].request_id, "unknown-op");
    assert_eq!(
        responses[0].error.as_ref().unwrap().code,
        "V3_PROTOCOL_UNKNOWN_OPERATION"
    );
    assert_eq!(
        responses[1].error.as_ref().unwrap().code,
        "V3_PROTOCOL_INVALID_JSON"
    );
    assert_eq!(
        responses[2].error.as_ref().unwrap().code,
        "V3_PROTOCOL_MESSAGE_TOO_LARGE"
    );
    assert_eq!(responses[2].request_id, "oversized-workbench");
    assert_eq!(responses[3].request_id, "shutdown-after-errors");
    assert!(output.len() < 8_192);
}

#[test]
fn serve_accepts_a_multi_megabyte_frozen_workbench_request() {
    let large = request(
        "large-workbench",
        SidecarOperation::Health,
        json!({"padding": "x".repeat(2 * 1024 * 1024)}),
    );
    let shutdown = request(
        "shutdown-after-large",
        SidecarOperation::Shutdown,
        json!({}),
    );
    let input = format!(
        "{}\n{}\n",
        serde_json::to_string(&large).unwrap(),
        serde_json::to_string(&shutdown).unwrap(),
    );
    let mut output = Vec::new();

    serve(
        BufReader::new(Cursor::new(input.into_bytes())),
        &mut output,
        &Dispatcher::new(),
    )
    .unwrap();

    let responses = response_lines(&output);
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].request_id, "large-workbench");
    assert!(responses[0].ok);
}

#[test]
fn binary_stdout_contains_only_protocol_json() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_insar-service"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sidecar starts");
    let mut stdin = child.stdin.take().unwrap();
    for item in [
        request("spawn-handshake", SidecarOperation::Handshake, json!({})),
        request("spawn-health", SidecarOperation::Health, json!({})),
        request("spawn-shutdown", SidecarOperation::Shutdown, json!({})),
    ] {
        writeln!(stdin, "{}", serde_json::to_string(&item).unwrap()).unwrap();
    }
    drop(stdin);

    let output = child.wait_with_output().expect("sidecar exits");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let responses = response_lines(&output.stdout);
    assert_eq!(responses.len(), 3);
    assert_eq!(
        responses
            .iter()
            .map(|item| item.request_id.as_str())
            .collect::<Vec<_>>(),
        vec!["spawn-handshake", "spawn-health", "spawn-shutdown"]
    );
    let unique = responses
        .iter()
        .map(|item| item.request_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), 3);
}
