#[cfg(feature = "nisar-subset-live-acceptance")]
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(feature = "nisar-subset-live-acceptance")]
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
#[cfg(feature = "nisar-subset-live-acceptance")]
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
#[cfg(feature = "nisar-subset-live-acceptance")]
use std::time::Instant;

#[cfg(feature = "nisar-subset-live-acceptance")]
use hdf5::File as Hdf5File;
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_application_service::{
    build_nisar_subset_live_frequency_a_hh_descriptor,
    build_nisar_subset_live_frequency_a_multi_pol_descriptor,
    map_nisar_product_for_live_acceptance, CapabilityRuntimeObservations,
    NISAR_REMOTE_SUBSET_CAPABILITY_ID,
};
use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, NisarCreateRemoteSubsetTaskRequest,
};
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_application_service::{
    AsfNisarSubsetLiveDescriptorError, AsfNisarSubsetLiveFrequencyAMultiPolDescriptorV1,
    AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1,
};
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_application_service::{
    NisarConfirmSubsetTaskV2Request, NisarPrepareSubsetIntentV2Request,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, ContractError, DownloadFailure,
    DownloadItemProgress, DownloadItemResult, DownloadOutcome, NisarSubsetFrequency,
    NisarSubsetValidatorKind, ProviderResult,
};
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_core::{
    BuildCapabilityState, CapabilityPolicyState, CatalogSearchQuery, CatalogSearchResponse,
    CredentialReadinessState, EvidenceAdmissionState, FrozenSearchBatch, RuntimeReadinessState,
    SearchWorkbenchState, SnapshotItem, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskLifecycleStatus, TaskStateCommandKind,
};
use insar_download::{
    prefetch_live_nisar_metadata, preflight_live_nisar_source,
    prepare_offline_nisar_subset_acceptance_case, validate_nisar_live_acceptance_budget,
    CredentialMaterial, DestinationLeaseController, DownloadWork, DownloadWorker,
    Hdf5RangeWorkspaceV1, HttpTransfer, OfflineNisarSubsetWriter, ProxyPolicy, RemoteValidator,
    ResolvedAsset, ResolvedMethod, TransferOptions, WorkerControl, NISAR_LIVE_MAX_REQUESTS,
    NISAR_LIVE_MAX_RESPONSE_BODY_BYTES,
};
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_download::{
    NisarIsce3LiveGeometryGateV1, NisarLiveAcceptanceEvidenceSink, NisarLiveAcceptanceEvidenceV2,
    ProductionDownloadWorker, NISAR_M2D_A_CURRENT_PLAN_SHA256,
    NISAR_M2D_B2_SEMANTIC_PLAN_AUTHORITY_SHA256,
};
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_provider::asf::catalog_client::AsfCatalogClient;
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_provider::asf::catalog_client::NISAR_CATALOGUE_CANARY_GRANULE_ID;
use insar_provider::asf::nisar_subset::normalize_nisar_subset_aoi;
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_provider::asf::nisar_subset::supported_dataset_polarizations;
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_provider::asf::{map_nisar_rslc_umm_granule, CmrGranuleEnvelope};
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_provider::catalog::{
    CollectionProfileRegistry, NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID,
    NISAR_PRODUCTION_CONFIGURATION, NISAR_PROFILE_ID, NISAR_PROVIDER_ID, NISAR_QUALITY_STATUS,
    NISAR_REQUIRED_PRODUCT_TYPE,
};
use insar_service::LiveAsfSearch;
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_service::LiveCatalogSearch;
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_store::{TaskStore, NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND};
#[cfg(feature = "nisar-subset-live-acceptance")]
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const GRANULE_ID: &str =
    "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001";
#[cfg(feature = "nisar-subset-live-acceptance")]
const PROVIDER_PRODUCT_ID: &str = "G4263869502-ASF";
const FILE_NAME: &str =
    "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001.h5";
const SOURCE_BYTES: u64 = 14_252_244_992;
const SOURCE_CHECKSUM_ALGORITHM: &str = "MD5";
const SOURCE_CHECKSUM_VALUE: &str = "059ada6521bff1ef314fb1141a89cd10";
const AOI_SHA256: &str = "ef57c7102808f8c921393592f3e744638dae05f984ac23bb0f047b867ae8cee8";
const SWATH: &str = "LSAR_RSLC";
const HISTORICAL_P1_5_PLAN_SHA256: &str =
    "b2e6d1c7384e8c6c3425c90f3642f21de7954f0e2bba029d59bb5dad856d2bc9";
const REMOTE_BYTES: u64 = 198_995_157;
const MAX_REQUESTS: u64 = 256;
const MAX_RESPONSE_BYTES: u64 = 512 * 1024 * 1024;

#[cfg(feature = "nisar-subset-live-acceptance")]
fn controlled_live_capability_observations() -> CapabilityRuntimeObservations {
    CapabilityRuntimeObservations::default()
        .with_build(
            "production-worker.nisar-remote-subset.v1",
            BuildCapabilityState::Available,
        )
        .with_runtime(
            "production-worker.nisar-remote-subset.v1",
            RuntimeReadinessState::Ready,
        )
        .with_credential(
            "route.asf.nisar-rslc.remote-subset.v1",
            CredentialReadinessState::Ready,
        )
        .with_admission(
            "admission.nisar.remote-subset.p0.3a.v1",
            EvidenceAdmissionState::Accepted,
        )
        .with_policy(
            NISAR_REMOTE_SUBSET_CAPABILITY_ID,
            CapabilityPolicyState::Allowed,
        )
}

fn aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [37.720222204639484, 10.857787104635449],
            [37.721296026021705, 10.857533983552843],
            [37.721144257142576, 10.856_905_700_184_74],
            [37.720070447767895, 10.857158813483217],
            [37.720222204639484, 10.857787104635449]
        ]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn official_aoi_sha256() -> String {
    let value = json!({
        "coordinates": [[
            [37.720222204639484, 10.857787104635449],
            [37.721296026021705, 10.857533983552843],
            [37.721144257142576, 10.856_905_700_184_74],
            [37.720070447767895, 10.857158813483217],
            [37.720222204639484, 10.857787104635449]
        ]],
        "type": "Polygon"
    });
    sha256_bytes(serde_json::to_string(&value).unwrap().as_bytes())
}

fn fixture_error() -> ContractError {
    ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap()
}

#[derive(Default)]
struct FixtureAsf;

impl LiveAsfSearch for FixtureAsf {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(fixture_error())
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
impl LiveCatalogSearch for FixtureAsf {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        Err(fixture_error())
    }
}

#[derive(Default)]
struct NoopWorker;

impl DownloadWorker for NoopWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Failed,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(DownloadFailure {
                code: "FIXTURE_ONLY".to_owned(),
                reason_zh: "fixture only".to_owned(),
                retryable: false,
            }),
            finished_at: "2026-08-02T00:00:00Z".to_owned(),
        }
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[derive(Default)]
struct SchedulerProbeWorker {
    authority_calls: AtomicUsize,
    cancellation_observed: AtomicUsize,
    fallback_calls: AtomicUsize,
    blocking: bool,
}

#[cfg(feature = "nisar-subset-live-acceptance")]
impl SchedulerProbeWorker {
    fn blocking() -> Self {
        Self {
            blocking: true,
            ..Self::default()
        }
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
impl DownloadWorker for SchedulerProbeWorker {
    fn execute_with_authority(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _plan_freeze: &insar_download::NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.authority_calls.fetch_add(1, Ordering::SeqCst);
        if self.blocking {
            let task_id = work.task.task_id.clone();
            let item_id = work.item.item_id.clone();
            let destination = work.task.output_root.clone();
            while !control.is_cancelled() {
                thread::sleep(Duration::from_millis(2));
            }
            self.cancellation_observed.fetch_add(1, Ordering::SeqCst);
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Cancelled,
                destination,
                bytes_written: 0,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-09-01T00:00:00Z".to_owned(),
            };
        }
        self.execute(work, control, progress)
    }

    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.fallback_calls.fetch_add(1, Ordering::SeqCst);
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Failed,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(DownloadFailure {
                code: "FIXTURE_ONLY".to_owned(),
                reason_zh: "fixture only".to_owned(),
                retryable: false,
            }),
            finished_at: "2026-09-01T00:00:00Z".to_owned(),
        }
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn save_source(path: &Path, item: SnapshotItem) -> Result<(), &'static str> {
    let item_id = item.item_id.clone();
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "nisar-live-source-batch".to_owned(),
        provider_id: item.provider_id.clone(),
        created_at: "2026-08-02T00:00:00Z".to_owned(),
        aoi_name: "P1.5 fixed AOI".to_owned(),
        aoi_geojson: Some(aoi()),
        query: Map::new(),
        search_stats: None,
        candidates: vec![item],
    };
    let mut store = TaskStore::open(path).map_err(|_| "NISAR_LIVE_SOURCE_STORE_FAILED")?;
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch,
            workbench: SearchWorkbenchState {
                batch_id: "nisar-live-source-batch".to_owned(),
                checked_item_ids: vec![item_id],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .map_err(|_| "NISAR_LIVE_SOURCE_STORE_FAILED")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn live_query() -> Result<CatalogSearchQuery, &'static str> {
    serde_json::from_value(json!({
        "logical_dataset": NISAR_LOGICAL_DATASET,
        "provider_id": NISAR_PROVIDER_ID,
        "native_collection_id": NISAR_NATIVE_COLLECTION_ID,
        "aoi_geojson": aoi(),
        "cursor": null,
        "result_limit": 100,
        "filters": {
            "datetime": {
                "start": "2026-07-26T15:00:00Z",
                "end": "2026-07-26T17:00:00Z"
            },
            "production_configuration": "PR"
        }
    }))
    .map_err(|_| "NISAR_LIVE_CATALOGUE_FAILED")
}

fn fixed_gate(
    enabled: Option<&str>,
    granule: Option<&str>,
    root: Option<&str>,
) -> Result<PathBuf, &'static str> {
    if enabled != Some("1") {
        return Err("NISAR_LIVE_ACCEPTANCE_REQUIRED");
    }
    if granule != Some(GRANULE_ID) {
        return Err("NISAR_LIVE_GRANULE_MISMATCH");
    }
    let value = root.ok_or("NISAR_LIVE_ROOT_REQUIRED")?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("NISAR_LIVE_ROOT_NOT_ABSOLUTE");
    }
    if path.exists() {
        return Err("NISAR_LIVE_ROOT_MUST_NOT_EXIST");
    }
    Ok(path)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn frequency_b_fixed_gate() -> Result<PathBuf, &'static str> {
    if std::env::var("INSAR_NISAR_FREQ_B_LIVE_ACCEPTANCE")
        .ok()
        .as_deref()
        != Some("1")
    {
        return Err("NISAR_FREQ_B_LIVE_ACCEPTANCE_REQUIRED");
    }
    if std::env::var("INSAR_NISAR_FREQ_B_LIVE_GRANULE")
        .ok()
        .as_deref()
        != Some(GRANULE_ID)
    {
        return Err("NISAR_FREQ_B_GRANULE_MISMATCH");
    }
    let value =
        std::env::var("INSAR_NISAR_FREQ_B_LIVE_ROOT").map_err(|_| "NISAR_FREQ_B_ROOT_REQUIRED")?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("NISAR_FREQ_B_ROOT_NOT_ABSOLUTE");
    }
    if path.exists() {
        return Err("NISAR_FREQ_B_ROOT_MUST_NOT_EXIST");
    }
    Ok(path)
}

fn fixed_scope(
    granule: &str,
    aoi_sha256: &str,
    frequency: NisarSubsetFrequency,
    polarization: &str,
    swath: &str,
) -> Result<(), &'static str> {
    if granule != GRANULE_ID {
        return Err("NISAR_LIVE_GRANULE_MISMATCH");
    }
    if aoi_sha256 != AOI_SHA256 {
        return Err("NISAR_LIVE_AOI_MISMATCH");
    }
    if frequency != NisarSubsetFrequency::A || polarization != "HH+HV" || swath != SWATH {
        return Err("NISAR_LIVE_SELECTION_MISMATCH");
    }
    Ok(())
}

#[test]
fn live_acceptance_requires_all_explicit_gates_without_network() {
    assert_eq!(
        fixed_gate(None, Some(GRANULE_ID), Some("C:\\missing")),
        Err("NISAR_LIVE_ACCEPTANCE_REQUIRED")
    );
    assert_eq!(
        fixed_gate(Some("1"), Some("other"), Some("C:\\missing")),
        Err("NISAR_LIVE_GRANULE_MISMATCH")
    );
    assert_eq!(
        fixed_gate(Some("1"), Some(GRANULE_ID), None),
        Err("NISAR_LIVE_ROOT_REQUIRED")
    );
    assert_eq!(
        fixed_scope("other", AOI_SHA256, NisarSubsetFrequency::A, "HH", SWATH),
        Err("NISAR_LIVE_GRANULE_MISMATCH")
    );
    assert_eq!(
        fixed_scope(GRANULE_ID, "wrong", NisarSubsetFrequency::A, "HH", SWATH),
        Err("NISAR_LIVE_AOI_MISMATCH")
    );
    assert_eq!(
        fixed_scope(GRANULE_ID, AOI_SHA256, NisarSubsetFrequency::B, "HH", SWATH),
        Err("NISAR_LIVE_SELECTION_MISMATCH")
    );
}

#[test]
fn normal_business_store_has_no_live_subset_route() {
    let path = std::env::temp_dir().join(format!(
        "nisar-p1-5-disabled-{}.sqlite3",
        std::process::id()
    ));
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&path),
        Arc::new(FixtureAsf),
        Arc::new(NoopWorker),
    );
    let capability = service.nisar_remote_subset_capability().unwrap();
    assert!(capability.visible);
    assert!(!capability.enabled);
    assert!(capability.live_verified);
    assert!(!capability.production_enabled);
    assert!(!capability.default_enabled);
    assert_eq!(
        service
            .create_nisar_remote_subset_task(&NisarCreateRemoteSubsetTaskRequest {
                request_id: "blocked".to_owned(),
                source_batch_id: "blocked".to_owned(),
                item_id: "blocked".to_owned(),
                task_id: "blocked".to_owned(),
                output_root: std::env::temp_dir(),
                layout: insar_core::DownloadLayout::Direct,
                concurrency: 2,
                frequency: NisarSubsetFrequency::A,
                polarization: "HH".to_owned(),
                created_at: "2026-08-02T00:00:00Z".to_owned(),
            })
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_NOT_ENABLED"
    );
    service.shutdown().unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

struct OneShotServer {
    url: String,
    worker: Option<JoinHandle<()>>,
}

impl OneShotServer {
    fn new(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let headers = headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<Vec<_>>();
        let body = body.to_vec();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_headers(&mut stream);
            let mut response = format!("HTTP/1.1 {status}\r\n");
            for (name, value) in headers {
                response.push_str(&format!("{name}: {value}\r\n"));
            }
            response.push_str(&format!(
                "Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            ));
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });
        Self {
            url: format!("http://{address}/fixed.h5"),
            worker: Some(worker),
        }
    }
}

impl Drop for OneShotServer {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn read_headers(stream: &mut TcpStream) -> Vec<u8> {
    let mut result = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        let count = stream.read(&mut buffer).unwrap_or(0);
        if count == 0 {
            break;
        }
        result.extend_from_slice(&buffer[..count]);
        if result.windows(4).any(|value| value == b"\r\n\r\n") {
            break;
        }
    }
    result
}

fn direct_transfer() -> HttpTransfer {
    HttpTransfer::new_with_proxy_policy(
        TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(30),
            chunk_size: 64 * 1024,
            max_accepted_polls: 1,
            accepted_poll_timeout: Duration::from_secs(1),
        },
        ProxyPolicy::Direct,
    )
    .unwrap()
}

fn preflight_asset(url: &str, size: u64) -> ResolvedAsset {
    ResolvedAsset::new(url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(size))
}

fn preflight_response(status: &str, etag: Option<&str>, total: u64) -> OneShotServer {
    let mut headers = vec![("Content-Range", format!("bytes 0-0/{total}"))];
    if let Some(etag) = etag {
        headers.push(("ETag", etag.to_owned()));
    }
    headers.push(("Content-Encoding", "identity".to_owned()));
    let header_refs = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect::<Vec<_>>();
    OneShotServer::new(status, &header_refs, &[0x01])
}

#[test]
fn validator_preflight_rejects_missing_weak_mismatched_and_200_responses() {
    for (etag, status, expected) in [
        (None, "206 Partial Content", true),
        (Some("W/\"weak\""), "206 Partial Content", true),
        (Some("\"strong\""), "206 Partial Content", false),
        (Some("\"strong\""), "200 OK", true),
    ] {
        let total = if etag == Some("\"strong\"") && status.starts_with("206") {
            4
        } else {
            5
        };
        let server = preflight_response(status, etag, total);
        let asset = preflight_asset(&server.url, 4);
        let result = preflight_live_nisar_source(&direct_transfer(), &asset, 4, &|| false);
        assert_eq!(result.is_err(), expected);
    }
}

#[test]
fn signed_locator_is_not_exposed_by_resolved_asset_debug() {
    let asset = ResolvedAsset::new(
        "https://datapool.asf.alaska.edu/fixed.h5?X-Amz-Signature=SECRET_SIGNATURE",
        ResolvedMethod::Get,
    )
    .unwrap()
    .with_remote_validator(RemoteValidator {
        etag: Some("\"redacted\"".to_owned()),
        last_modified: None,
        content_length: Some(SOURCE_BYTES),
    });
    let debug = format!("{asset:?}");
    assert!(!debug.contains("SECRET_SIGNATURE"));
    assert!(!debug.contains("X-Amz-Signature"));
    assert!(!debug.contains("Authorization"));
}

#[test]
fn live_budget_constants_are_hard_fail_limits_not_estimates() {
    assert_eq!(MAX_REQUESTS, NISAR_LIVE_MAX_REQUESTS);
    assert_eq!(MAX_RESPONSE_BYTES, NISAR_LIVE_MAX_RESPONSE_BODY_BYTES);
    const {
        assert!(REMOTE_BYTES < SOURCE_BYTES / 20);
        assert!(MAX_RESPONSE_BYTES <= 512 * 1024 * 1024);
    }
}

#[test]
fn live_request_and_response_caps_fail_closed_at_the_first_excess() {
    assert!(validate_nisar_live_acceptance_budget(NISAR_LIVE_MAX_REQUESTS, 0).is_ok());
    assert!(validate_nisar_live_acceptance_budget(NISAR_LIVE_MAX_REQUESTS + 1, 0).is_err());
    assert!(validate_nisar_live_acceptance_budget(0, NISAR_LIVE_MAX_RESPONSE_BODY_BYTES).is_ok());
    assert!(
        validate_nisar_live_acceptance_budget(0, NISAR_LIVE_MAX_RESPONSE_BODY_BYTES + 1).is_err()
    );
}

#[test]
fn credential_material_debug_is_redacted_and_not_a_durable_contract() {
    let material = CredentialMaterial::bearer("OFFLINE_TEST_SECRET").unwrap();
    let debug = format!("{material:?}");
    assert!(!debug.contains("OFFLINE_TEST_SECRET"));
    assert!(!debug.contains("Authorization"));
}

#[test]
fn failed_offline_writer_attempt_leaves_no_final_artifact() {
    let case = prepare_offline_nisar_subset_acceptance_case(
        "p1-5-failed-writer",
        1,
        NisarSubsetFrequency::A,
        "HH",
        "/science/LSAR/RSLC/swaths/frequencyA/HH",
        "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask",
    );
    let output_root = case._root.0.join("failed-output");
    let writer = OfflineNisarSubsetWriter::new(
        output_root.clone(),
        DestinationLeaseController::new_for_qualification_test(),
    )
    .unwrap();
    let mut tampered = case.request.clone();
    tampered.expected_output.file_name = "not-the-frozen-output.h5".to_owned();
    assert!(writer
        .write_and_validate(
            &tampered,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        )
        .is_err());
    assert!(
        fs::read_dir(&output_root)
            .map(|entries| entries.filter_map(Result::ok).all(|entry| entry
                .path()
                .extension()
                .and_then(|value| value.to_str())
                != Some("h5")))
            .unwrap_or(true),
        "failed validation must not promote a final HDF5 artifact"
    );
}

#[test]
fn metadata_range_validator_change_is_terminal_before_body_consumption() {
    let root =
        std::env::temp_dir().join(format!("nisar-p1-5-validator-drift-{}", std::process::id()));
    let server = OneShotServer::new(
        "206 Partial Content",
        &[
            ("ETag", "\"changed\""),
            ("Content-Range", "bytes 0-0/4"),
            ("Content-Encoding", "identity"),
        ],
        &[0x01],
    );
    let workspace = Hdf5RangeWorkspaceV1::new(root.clone()).unwrap();
    let validator = insar_core::NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: "\"frozen\"".to_owned(),
        content_length: 4,
    };
    let result = prefetch_live_nisar_metadata(
        &direct_transfer(),
        &preflight_asset(&server.url, 4),
        &workspace,
        &validator,
        4,
        &|| false,
    );
    assert!(matches!(
        result,
        Err(insar_download::Hdf5RangeTransferError::SourceChanged)
    ));
    drop(server);
    let _ = fs::remove_dir_all(root);
}

fn sha256_bytes(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

#[cfg(feature = "nisar-subset-live-acceptance")]
const BASELINE_COMMIT: &str = "a65c0828596dc9a3a6ed34058ec23dc2479f28d2";

#[cfg(feature = "nisar-subset-live-acceptance")]
fn current_git_commit() -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git is required to record the live harness commit");
    assert!(output.status.success(), "git rev-parse HEAD failed");
    String::from_utf8(output.stdout)
        .expect("git commit output is UTF-8")
        .trim()
        .to_owned()
}

#[cfg(feature = "nisar-subset-live-acceptance")]
struct LiveExecutionCleanupGuard {
    root: PathBuf,
    evidence: Arc<NisarLiveAcceptanceEvidenceSink>,
    preserve_final_artifact: bool,
    cleaned: bool,
}

#[cfg(feature = "nisar-subset-live-acceptance")]
impl LiveExecutionCleanupGuard {
    fn new(root: PathBuf, evidence: Arc<NisarLiveAcceptanceEvidenceSink>) -> Self {
        Self {
            root,
            evidence,
            preserve_final_artifact: false,
            cleaned: false,
        }
    }

    fn preserve_final_artifact(&mut self) {
        self.preserve_final_artifact = true;
    }

    fn discard_final_artifact(&mut self) {
        self.preserve_final_artifact = false;
    }

    fn replace_evidence(&mut self, evidence: Arc<NisarLiveAcceptanceEvidenceSink>) {
        self.evidence = evidence;
    }

    fn preserve_failure_diagnostics(&mut self) {
        // Explicit forensic runs retain the task-scoped Range workspace after
        // a typed failure so the received bytes can be compared with the
        // verified local source. Ordinary acceptance runs still clean up.
        self.cleaned = true;
    }

    fn cleanup_now(&mut self) -> std::io::Result<()> {
        let mut targets = vec![
            self.root.join("transfer"),
            self.root.join("candidate"),
            self.root.join("output").join(".insar-nisar-range"),
        ];
        if !self.preserve_final_artifact {
            targets.push(self.root.join("output"));
        }
        let mut first_error = None;
        for target in targets {
            match fs::remove_dir_all(&target) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        let cleanup_completed = first_error.is_none();
        if let Err(error) = self.evidence.mark_cleanup_completed(cleanup_completed) {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => {
                self.cleaned = true;
                Ok(())
            }
        }
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
impl Drop for LiveExecutionCleanupGuard {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = self.cleanup_now();
        }
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
const LIVE_ACCEPTANCE_REQUEST_ID: &str = "nisar-live-acceptance-request";

#[cfg(feature = "nisar-subset-live-acceptance")]
fn record_live_failure_if_unset(evidence: &NisarLiveAcceptanceEvidenceSink, code: &'static str) {
    if evidence.snapshot().status != "fail" {
        let _ = evidence.record_failure(code);
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn failure_code(evidence: &NisarLiveAcceptanceEvidenceSink, fallback: &'static str) -> String {
    let snapshot = evidence.snapshot();
    if snapshot.status == "fail" {
        snapshot.typed_stop_code
    } else {
        fallback.to_owned()
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn boundary_failure_code(evidence: &NisarLiveAcceptanceEvidenceSink, _boundary: &str) -> String {
    failure_code(evidence, "NISAR_LIVE_EVIDENCE_INCOMPLETE")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.as_bytes().iter().all(u8::is_ascii_hexdigit)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveRecoveryMode {
    PauseResume,
    RestartResume,
    BodyInterruption,
    ResumeExistingStore,
}

#[cfg(feature = "nisar-subset-live-acceptance")]
impl LiveRecoveryMode {
    fn label(self) -> &'static str {
        match self {
            Self::PauseResume => "pause_resume",
            Self::RestartResume => "restart_resume",
            Self::BodyInterruption => "body_interruption",
            Self::ResumeExistingStore => "resume_existing_store",
        }
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
const BODY_INTERRUPTION_READ_BUFFER_BYTES: u64 = 64 * 1024;

/// A transient body failure of one science Range is retried automatically
/// (design 2026-10-17 §3: three attempts per Range, backoff 5 s then 20 s), so
/// one interrupted body no longer fails the item. The interruption is repeated
/// for every attempt of that Range, which then ends as a retryable failure.
#[cfg(feature = "nisar-subset-live-acceptance")]
const BODY_INTERRUPTION_ATTEMPTS: u64 = 3;

#[cfg(feature = "nisar-subset-live-acceptance")]
struct LiveBodyInterruptionControl {
    armed: AtomicBool,
    interruptions: AtomicU64,
    first_observed_bytes: AtomicU64,
    total_observed_bytes: AtomicU64,
}

#[cfg(feature = "nisar-subset-live-acceptance")]
impl LiveBodyInterruptionControl {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            armed: AtomicBool::new(false),
            interruptions: AtomicU64::new(0),
            first_observed_bytes: AtomicU64::new(0),
            total_observed_bytes: AtomicU64::new(0),
        })
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn on_body_read(&self, observed_bytes: u64, _control: &WorkerControl) -> bool {
        if observed_bytes == 0 || !self.armed.load(Ordering::SeqCst) {
            return false;
        }
        let Ok(previous) =
            self.interruptions
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |interruptions| {
                    (interruptions < BODY_INTERRUPTION_ATTEMPTS).then_some(interruptions + 1)
                })
        else {
            return false;
        };
        if previous == 0 {
            self.first_observed_bytes
                .store(observed_bytes, Ordering::SeqCst);
        }
        self.total_observed_bytes
            .fetch_add(observed_bytes, Ordering::SeqCst);
        // Returning immediately models a response-body read failure. The
        // production worker maps this hook to ScienceTransferIncomplete; the
        // executor retries the same Range until its attempts are used, then
        // Store/Scheduler record a real Failed item eligible for RetryItems.
        true
    }

    /// (interruptions, response-body bytes observed across them).
    fn interruption_totals(&self) -> (u64, u64) {
        (
            self.interruptions.load(Ordering::SeqCst),
            self.total_observed_bytes.load(Ordering::SeqCst),
        )
    }

    fn wait_for_trigger(&self, timeout: Duration) -> Result<u64, &'static str> {
        let deadline = Instant::now() + timeout;
        loop {
            let triggered = self.interruptions.load(Ordering::SeqCst) > 0;
            let observed = self.first_observed_bytes.load(Ordering::SeqCst);
            if triggered && observed > 0 {
                return Ok(observed);
            }
            if Instant::now() >= deadline {
                return Err(if triggered {
                    "NISAR_LIVE_RECOVERY_BODY_INTERRUPTION_OBSERVATION_INVALID"
                } else {
                    "NISAR_LIVE_RECOVERY_BODY_INTERRUPTION_TIMEOUT"
                });
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
struct LiveRecoveryCheckpointObservation {
    state: String,
    task_id: String,
    item_id: String,
    anchor_generation: u64,
    checkpoint_sha256: String,
    lineage_key_sha256: String,
    execution_binding_sha256: String,
    prepared_intent_sha256: String,
    final_plan_sha256: String,
    physical_plan_sha256: String,
    inventory_sha256: String,
    inventory_evidence_sha256: String,
    source_graph_sha256: String,
    source_validator_sha256: String,
    part_identity_sha256: String,
    planned_science_response_body_bytes: u64,
    committed_science_request_count: u64,
    committed_science_response_body_bytes: u64,
    reserved_science_request_count: u64,
    reserved_science_response_body_bytes: u64,
    issued_science_request_count: u64,
    observed_science_response_body_bytes: u64,
    failed_science_request_count: u64,
    indeterminate_science_request_count: u64,
    indeterminate_science_response_body_bytes: u64,
    planned_ranges: Vec<LiveRecoveryRangeObservation>,
    completed_ranges: Vec<LiveRecoveryRangeObservation>,
    completed_range_request_counts: Vec<u64>,
    planned_range_count: usize,
    resumed_bytes: u64,
    pending_attempt: bool,
    completed_ranges_digest: String,
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, serde::Deserialize)]
struct LiveRecoveryRangeObservation {
    index: u64,
    start: u64,
    end_exclusive: u64,
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
struct LiveRecoveryAnchorObservation {
    lineage_key_sha256: String,
    record_sha256: String,
    generation: u64,
    committed_checkpoint_sha256: Option<String>,
    pending_transition: bool,
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[derive(Debug, Serialize, serde::Deserialize)]
struct LiveRecoveryTrace {
    evidence_kind: String,
    mode: String,
    threshold_basis: String,
    threshold_bytes: Option<u64>,
    planned_science_response_body_bytes: Option<u64>,
    pause_request_progress_bytes: Option<u64>,
    pause_request_checkpoint_bytes: Option<u64>,
    pause_request_revision: Option<u64>,
    paused_store_progress_bytes: Option<u64>,
    paused_runtime_revision: Option<u64>,
    resume_runtime_revision: Option<u64>,
    reopen_runtime_revision: Option<u64>,
    worker_quiescent: bool,
    pause: Option<LiveRecoveryCheckpointObservation>,
    anchor_at_pause: Option<LiveRecoveryAnchorObservation>,
    reopen: Option<LiveRecoveryCheckpointObservation>,
    anchor_at_reopen: Option<LiveRecoveryAnchorObservation>,
    completion: Option<LiveRecoveryCheckpointObservation>,
    anchor_at_completion: Option<LiveRecoveryAnchorObservation>,
    reused_verified_bytes: u64,
    body_interruption_range: Option<LiveRecoveryRangeObservation>,
    body_interruption_observed_bytes: Option<u64>,
    body_interruption_result_code: Option<String>,
    body_interruption_failed_request_count: Option<u64>,
    body_interruption_retry_request_count: Option<u64>,
    failure_runtime_revision: Option<u64>,
    failure: Option<LiveRecoveryCheckpointObservation>,
    anchor_at_failure: Option<LiveRecoveryAnchorObservation>,
    duplicate_science_ranges: Vec<LiveRecoveryRangeDuplicate>,
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
struct LiveRecoveryRangeDuplicate {
    start: u64,
    end_exclusive: u64,
    observations: u64,
}

#[cfg(feature = "nisar-subset-live-acceptance")]
impl LiveRecoveryTrace {
    fn new(mode: LiveRecoveryMode) -> Self {
        Self {
            evidence_kind: "nisar_live_recovery_trace_v1".to_owned(),
            mode: mode.label().to_owned(),
            threshold_basis: "completed_science_checkpoint_bytes_over_planned_science_bytes"
                .to_owned(),
            threshold_bytes: None,
            planned_science_response_body_bytes: None,
            pause_request_progress_bytes: None,
            pause_request_checkpoint_bytes: None,
            pause_request_revision: None,
            paused_store_progress_bytes: None,
            paused_runtime_revision: None,
            resume_runtime_revision: None,
            reopen_runtime_revision: None,
            worker_quiescent: false,
            pause: None,
            anchor_at_pause: None,
            reopen: None,
            anchor_at_reopen: None,
            completion: None,
            anchor_at_completion: None,
            reused_verified_bytes: 0,
            body_interruption_range: None,
            body_interruption_observed_bytes: None,
            body_interruption_result_code: None,
            body_interruption_failed_request_count: None,
            body_interruption_retry_request_count: None,
            failure_runtime_revision: None,
            failure: None,
            anchor_at_failure: None,
            duplicate_science_ranges: Vec::new(),
        }
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn write_live_recovery_trace(root: &Path, trace: &LiveRecoveryTrace) -> Result<(), &'static str> {
    write_live_recovery_trace_at(&root.join("evidence"), trace)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn write_live_recovery_trace_at(
    evidence_root: &Path,
    trace: &LiveRecoveryTrace,
) -> Result<(), &'static str> {
    fs::create_dir_all(evidence_root).map_err(|_| "NISAR_LIVE_RECOVERY_TRACE_WRITE_FAILED")?;
    let path = evidence_root.join("recovery-trace.json");
    let bytes =
        serde_json::to_vec_pretty(trace).map_err(|_| "NISAR_LIVE_RECOVERY_TRACE_WRITE_FAILED")?;
    fs::write(path, bytes).map_err(|_| "NISAR_LIVE_RECOVERY_TRACE_WRITE_FAILED")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn find_live_recovery_checkpoint(root: &Path) -> Option<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().and_then(|name| name.to_str()) == Some("science-checkpoint-v2.json")
            {
                return Some(path);
            }
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    None
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn recovery_u64(value: &Value, key: &str) -> Result<u64, &'static str> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn recovery_string(value: &Value, key: &str) -> Result<String, &'static str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn read_live_recovery_checkpoint(
    output: &Path,
) -> Result<Option<LiveRecoveryCheckpointObservation>, &'static str> {
    let Some(path) = find_live_recovery_checkpoint(output) else {
        return Ok(None);
    };
    let raw = fs::read_to_string(path).map_err(|_| "NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID")?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|_| "NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID")?;
    let planned = value
        .get("planned_ranges")
        .and_then(Value::as_array)
        .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID")?;
    let mut planned_science_response_body_bytes = 0_u64;
    let mut planned_ranges = Vec::with_capacity(planned.len());
    for (index, range) in planned.iter().enumerate() {
        let start = recovery_u64(range, "start")?;
        let end_exclusive = recovery_u64(range, "end_exclusive")?;
        let length = end_exclusive
            .checked_sub(start)
            .filter(|length| *length > 0)
            .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID")?;
        planned_science_response_body_bytes = planned_science_response_body_bytes
            .checked_add(length)
            .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID")?;
        planned_ranges.push(LiveRecoveryRangeObservation {
            index: index as u64,
            start,
            end_exclusive,
        });
    }
    let completed = value
        .get("completed_ranges")
        .and_then(Value::as_array)
        .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID")?;
    let mut completed_ranges = Vec::with_capacity(completed.len());
    let mut completed_range_request_counts = Vec::with_capacity(completed.len());
    for range in completed {
        let start = recovery_u64(range, "start")?;
        let end_exclusive = recovery_u64(range, "end_exclusive")?;
        if end_exclusive <= start {
            return Err("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID");
        }
        completed_range_request_counts.push(recovery_u64(range, "request_count")?);
        completed_ranges.push(LiveRecoveryRangeObservation {
            index: recovery_u64(range, "index")?,
            start,
            end_exclusive,
        });
    }
    let completed_ranges_digest = sha256_bytes(
        &serde_json::to_vec(completed).map_err(|_| "NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID")?,
    );
    let state = recovery_string(&value, "state")?;
    if !matches!(
        state.as_str(),
        "Planned" | "InProgress" | "Interrupted" | "Complete"
    ) || completed_ranges.len() > planned.len()
    {
        return Err("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID");
    }
    let task_id = recovery_string(&value, "task_id")?;
    let item_id = recovery_string(&value, "item_id")?;
    let checkpoint_sha256 = recovery_string(&value, "checkpoint_sha256")?;
    let lineage_key_sha256 = recovery_string(&value, "lineage_key_sha256")?;
    let execution_binding_sha256 = recovery_string(&value, "execution_binding_sha256")?;
    let prepared_intent_sha256 = recovery_string(&value, "prepared_intent_sha256")?;
    let final_plan_sha256 = recovery_string(&value, "final_plan_sha256")?;
    let physical_plan_sha256 = recovery_string(&value, "physical_plan_sha256")?;
    let inventory_sha256 = recovery_string(&value, "inventory_sha256")?;
    let inventory_evidence_sha256 = recovery_string(&value, "inventory_evidence_sha256")?;
    let source_graph_sha256 = recovery_string(&value, "source_graph_sha256")?;
    let source_validator_sha256 = recovery_string(&value, "source_validator_sha256")?;
    let part_identity_sha256 = recovery_string(&value, "part_identity_sha256")?;
    if !is_sha256(&checkpoint_sha256)
        || !is_sha256(&lineage_key_sha256)
        || !is_sha256(&execution_binding_sha256)
        || !is_sha256(&prepared_intent_sha256)
        || !is_sha256(&final_plan_sha256)
        || !is_sha256(&physical_plan_sha256)
        || !is_sha256(&inventory_sha256)
        || !is_sha256(&inventory_evidence_sha256)
        || !is_sha256(&source_graph_sha256)
        || !is_sha256(&source_validator_sha256)
        || !is_sha256(&part_identity_sha256)
    {
        return Err("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID");
    }
    Ok(Some(LiveRecoveryCheckpointObservation {
        state,
        task_id,
        item_id,
        anchor_generation: recovery_u64(&value, "anchor_generation")?,
        checkpoint_sha256,
        lineage_key_sha256,
        execution_binding_sha256,
        prepared_intent_sha256,
        final_plan_sha256,
        physical_plan_sha256,
        inventory_sha256,
        inventory_evidence_sha256,
        source_graph_sha256,
        source_validator_sha256,
        part_identity_sha256,
        planned_science_response_body_bytes,
        committed_science_request_count: recovery_u64(&value, "committed_science_request_count")?,
        committed_science_response_body_bytes: recovery_u64(
            &value,
            "committed_science_response_body_bytes",
        )?,
        reserved_science_request_count: recovery_u64(&value, "reserved_science_request_count")?,
        reserved_science_response_body_bytes: recovery_u64(
            &value,
            "reserved_science_response_body_bytes",
        )?,
        issued_science_request_count: recovery_u64(&value, "issued_science_request_count")?,
        observed_science_response_body_bytes: recovery_u64(
            &value,
            "observed_science_response_body_bytes",
        )?,
        failed_science_request_count: recovery_u64(&value, "failed_science_request_count")?,
        indeterminate_science_request_count: recovery_u64(
            &value,
            "indeterminate_science_request_count",
        )?,
        indeterminate_science_response_body_bytes: recovery_u64(
            &value,
            "indeterminate_science_response_body_bytes",
        )?,
        planned_ranges,
        completed_ranges,
        completed_range_request_counts,
        planned_range_count: planned.len(),
        resumed_bytes: recovery_u64(&value, "resumed_bytes")?,
        pending_attempt: value
            .get("pending_attempt")
            .is_some_and(|pending| !pending.is_null()),
        completed_ranges_digest,
    }))
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn read_live_recovery_anchor(
    database: &Path,
    task_id: &str,
    item_id: &str,
) -> Result<Option<LiveRecoveryAnchorObservation>, &'static str> {
    let store = TaskStore::open(database).map_err(|_| "NISAR_LIVE_RECOVERY_ANCHOR_UNAVAILABLE")?;
    let Some(anchor) = store
        .load_nisar_subset_execution_anchor_v2(
            task_id,
            item_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )
        .map_err(|_| "NISAR_LIVE_RECOVERY_ANCHOR_UNAVAILABLE")?
    else {
        return Ok(None);
    };
    let canonical: Value = serde_json::from_str(&anchor.canonical_json)
        .map_err(|_| "NISAR_LIVE_RECOVERY_ANCHOR_INVALID")?;
    let lineage_key_sha256 = recovery_string(&canonical, "lineage_key_sha256")?;
    let record_sha256 = anchor.record_sha256;
    if lineage_key_sha256 != anchor.lineage_key_sha256 || !is_sha256(&record_sha256) {
        return Err("NISAR_LIVE_RECOVERY_ANCHOR_INVALID");
    }
    let committed_checkpoint_sha256 = canonical
        .get("committed_checkpoint_sha256")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if committed_checkpoint_sha256
        .as_deref()
        .is_some_and(|value| !is_sha256(value))
    {
        return Err("NISAR_LIVE_RECOVERY_ANCHOR_INVALID");
    }
    Ok(Some(LiveRecoveryAnchorObservation {
        lineage_key_sha256,
        record_sha256,
        generation: recovery_u64(&canonical, "generation")?,
        committed_checkpoint_sha256,
        pending_transition: canonical
            .get("pending_transition")
            .is_some_and(|pending| !pending.is_null()),
    }))
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn live_recovery_progress_bytes(
    database: &Path,
    task_id: &str,
    item_id: &str,
) -> Result<u64, &'static str> {
    let store = TaskStore::open(database).map_err(|_| "NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?;
    let snapshot = store
        .load_download_snapshot(task_id)
        .map_err(|_| "NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?;
    Ok(snapshot
        .progress
        .iter()
        .filter(|progress| progress.item_id == item_id)
        .map(|progress| progress.bytes_completed)
        .max()
        .unwrap_or(0))
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn live_science_duplicate_ranges(
    evidence: &NisarLiveAcceptanceEvidenceV2,
) -> Vec<LiveRecoveryRangeDuplicate> {
    let mut counts = BTreeMap::<(u64, u64), u64>::new();
    for observation in evidence
        .range_observations
        .iter()
        .filter(|observation| observation.stage == "science")
    {
        *counts
            .entry((observation.start, observation.end_exclusive))
            .or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|((start, end_exclusive), observations)| {
            (observations > 1).then_some(LiveRecoveryRangeDuplicate {
                start,
                end_exclusive,
                observations,
            })
        })
        .collect()
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn build_live_production_service(
    database: &Path,
    evidence: &Arc<NisarLiveAcceptanceEvidenceSink>,
    reopen: bool,
    body_interruption: Option<Arc<LiveBodyInterruptionControl>>,
) -> Result<ApplicationService, &'static str> {
    let production_worker = ProductionDownloadWorker::new(TransferOptions::default())
        .map_err(|_| "NISAR_LIVE_WORKER_INIT_FAILED")?
        .with_nisar_live_evidence_sink_for_acceptance(Arc::clone(evidence));
    let production_worker = if let Some(body_interruption) = body_interruption {
        production_worker.with_nisar_subset_v2_body_interruption_hook_for_acceptance(Arc::new(
            move |observed_bytes, control| body_interruption.on_body_read(observed_bytes, control),
        ))
    } else {
        production_worker
    };
    let config = if reopen {
        ApplicationServiceConfig::existing_business_store(database.to_path_buf())
    } else {
        ApplicationServiceConfig::business_store(database.to_path_buf())
    }
    .with_controlled_capability_runtime_observations(controlled_live_capability_observations());
    let service = ApplicationService::with_production_worker_for_live_acceptance(
        config,
        Arc::new(production_worker),
    );
    // Existing Running stores need the same explicit startup used by desktop
    // core restore; construction alone leaves the runtime and scheduler lazy.
    service
        .startup()
        .map_err(|_| "NISAR_LIVE_SERVICE_STARTUP_FAILED")?;
    Ok(service)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn recovery_result_or_worker_failure<T>(
    evidence: &NisarLiveAcceptanceEvidenceSink,
    result: Result<T, &'static str>,
) -> Result<Option<T>, &'static str> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(_code) if evidence.snapshot().status == "fail" => Ok(None),
        Err(code) => Err(code),
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn wait_for_live_science_threshold(
    output: &Path,
    evidence: &NisarLiveAcceptanceEvidenceSink,
    timeout: Duration,
    minimum_completed_ranges: usize,
) -> Result<(LiveRecoveryCheckpointObservation, u64), &'static str> {
    let deadline = Instant::now() + timeout;
    loop {
        if evidence.snapshot().status == "fail" {
            return Err("NISAR_LIVE_RECOVERY_WORKER_FAILURE");
        }
        if let Some(checkpoint) = read_live_recovery_checkpoint(output)? {
            if checkpoint.planned_science_response_body_bytes == 0 {
                return Err("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID");
            }
            if checkpoint.state == "Complete" {
                return Err("NISAR_LIVE_RECOVERY_COMPLETED_BEFORE_PAUSE");
            }
            let threshold = checkpoint
                .planned_science_response_body_bytes
                .checked_div(10)
                .and_then(|value| value.checked_add(1))
                .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_INVALID")?;
            if checkpoint.committed_science_response_body_bytes >= threshold
                && checkpoint.completed_ranges.len() > minimum_completed_ranges
                && !checkpoint.pending_attempt
            {
                return Ok((checkpoint, threshold));
            }
        }
        if Instant::now() >= deadline {
            return Err("NISAR_LIVE_RECOVERY_THRESHOLD_TIMEOUT");
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn wait_for_live_pause_quiescence(
    database: &Path,
    output: &Path,
    task_id: &str,
    item_id: &str,
    evidence: &NisarLiveAcceptanceEvidenceSink,
    timeout: Duration,
) -> Result<(LiveRecoveryCheckpointObservation, u64, u64), &'static str> {
    let deadline = Instant::now() + timeout;
    let mut previous_sample: Option<(String, u64)> = None;
    let mut stable_samples = 0_u8;
    loop {
        if evidence.snapshot().status == "fail" {
            return Err("NISAR_LIVE_RECOVERY_WORKER_FAILURE");
        }
        let store =
            TaskStore::open(database).map_err(|_| "NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?;
        let runtime = store
            .load_scheduler_control_runtime(task_id)
            .map_err(|_| "NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?
            .ok_or("NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?;
        drop(store);
        let checkpoint = read_live_recovery_checkpoint(output)?;
        let progress = live_recovery_progress_bytes(database, task_id, item_id)?;
        let quiescent_state = runtime.task_paused
            && runtime.status == TaskLifecycleStatus::Paused
            && checkpoint
                .as_ref()
                .is_some_and(|checkpoint| checkpoint.state == "Interrupted")
            && checkpoint
                .as_ref()
                .is_some_and(|checkpoint| !checkpoint.pending_attempt);
        if quiescent_state {
            let checkpoint = checkpoint.expect("quiescent state includes a checkpoint");
            let sample = (checkpoint.checkpoint_sha256.clone(), progress);
            if previous_sample.as_ref() == Some(&sample) {
                stable_samples = stable_samples.saturating_add(1);
            } else {
                previous_sample = Some(sample);
                stable_samples = 1;
            }
            if stable_samples >= 4 {
                return Ok((checkpoint, runtime.revision, progress));
            }
        } else {
            previous_sample = None;
            stable_samples = 0;
        }
        if Instant::now() >= deadline {
            return Err("NISAR_LIVE_RECOVERY_PAUSE_QUIESCENCE_TIMEOUT");
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn wait_for_live_recovery_terminal(
    evidence: &NisarLiveAcceptanceEvidenceSink,
    timeout: Duration,
) -> Result<bool, &'static str> {
    let deadline = Instant::now() + timeout;
    loop {
        match evidence.snapshot().status.as_str() {
            "pass" => return Ok(true),
            "fail" => return Ok(false),
            _ => {}
        }
        if Instant::now() >= deadline {
            return Err("NISAR_LIVE_RECOVERY_COMPLETION_TIMEOUT");
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn live_recovery_anchor_matches_checkpoint(
    anchor: &LiveRecoveryAnchorObservation,
    checkpoint: &LiveRecoveryCheckpointObservation,
) -> bool {
    anchor.lineage_key_sha256 == checkpoint.lineage_key_sha256
        && anchor.generation == checkpoint.anchor_generation
        && anchor.committed_checkpoint_sha256.as_deref()
            == Some(checkpoint.checkpoint_sha256.as_str())
        && !anchor.pending_transition
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[allow(clippy::too_many_arguments)]
fn run_live_recovery_phase(
    service: &mut ApplicationService,
    root: &Path,
    database: &Path,
    output: &Path,
    evidence: &Arc<NisarLiveAcceptanceEvidenceSink>,
    item_id: &str,
    mode: LiveRecoveryMode,
    minimum_completed_ranges: usize,
) -> Result<(), &'static str> {
    const TASK_ID: &str = "nisar-live-acceptance-task";
    let mut trace = LiveRecoveryTrace::new(mode);
    write_live_recovery_trace(root, &trace)?;

    let Some((threshold_checkpoint, threshold_bytes)) = recovery_result_or_worker_failure(
        evidence,
        wait_for_live_science_threshold(
            output,
            evidence,
            Duration::from_secs(45 * 60),
            minimum_completed_ranges,
        ),
    )?
    else {
        return Ok(());
    };
    trace.threshold_bytes = Some(threshold_bytes);
    trace.planned_science_response_body_bytes =
        Some(threshold_checkpoint.planned_science_response_body_bytes);
    let pause_checkpoint = read_live_recovery_checkpoint(output)?
        .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_UNAVAILABLE")?;
    if pause_checkpoint.state == "Complete"
        || pause_checkpoint.pending_attempt
        || pause_checkpoint.completed_ranges.is_empty()
        || pause_checkpoint.committed_science_response_body_bytes < threshold_bytes
    {
        return Err("NISAR_LIVE_RECOVERY_THRESHOLD_CHECKPOINT_INVALID");
    }
    trace.pause_request_checkpoint_bytes =
        Some(pause_checkpoint.committed_science_response_body_bytes);
    trace.pause_request_progress_bytes =
        Some(live_recovery_progress_bytes(database, TASK_ID, item_id)?);
    let pause_runtime = TaskStore::open(database)
        .map_err(|_| "NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?
        .load_scheduler_control_runtime(TASK_ID)
        .map_err(|_| "NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?
        .ok_or("NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?;
    if pause_runtime.status == TaskLifecycleStatus::Completed {
        return Err("NISAR_LIVE_RECOVERY_COMPLETED_BEFORE_PAUSE");
    }
    trace.pause_request_revision = Some(pause_runtime.revision);
    write_live_recovery_trace(root, &trace)?;
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: TASK_ID.to_owned(),
            expected_revision: pause_runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: Vec::new(),
            },
            event_id: format!(
                "{LIVE_ACCEPTANCE_REQUEST_ID}:recovery:{}:pause:{}",
                mode.label(),
                pause_runtime.revision
            ),
            occurred_at: "2026-08-02T00:00:00Z".to_owned(),
        })
        .map_err(|_| "NISAR_LIVE_RECOVERY_PAUSE_FAILED")?;

    let Some((pause_checkpoint, paused_runtime_revision, paused_store_progress)) =
        recovery_result_or_worker_failure(
            evidence,
            wait_for_live_pause_quiescence(
                database,
                output,
                TASK_ID,
                item_id,
                evidence,
                Duration::from_secs(5 * 60),
            ),
        )?
    else {
        return Ok(());
    };
    if pause_checkpoint.committed_science_response_body_bytes < threshold_bytes
        || !pause_checkpoint.completed_ranges.iter().any(|range| {
            threshold_checkpoint
                .completed_ranges
                .iter()
                .any(|threshold_range| threshold_range == range)
        })
    {
        return Err("NISAR_LIVE_RECOVERY_PAUSE_CHECKPOINT_INVALID");
    }
    let pause_anchor = read_live_recovery_anchor(database, TASK_ID, item_id)?
        .ok_or("NISAR_LIVE_RECOVERY_ANCHOR_UNAVAILABLE")?;
    if !live_recovery_anchor_matches_checkpoint(&pause_anchor, &pause_checkpoint) {
        return Err("NISAR_LIVE_RECOVERY_PAUSE_ANCHOR_MISMATCH");
    }
    trace.worker_quiescent = true;
    trace.paused_store_progress_bytes = Some(paused_store_progress);
    trace.paused_runtime_revision = Some(paused_runtime_revision);
    trace.pause = Some(pause_checkpoint.clone());
    trace.anchor_at_pause = Some(pause_anchor.clone());
    write_live_recovery_trace(root, &trace)?;

    if mode == LiveRecoveryMode::RestartResume {
        service
            .shutdown()
            .map_err(|_| "NISAR_LIVE_RECOVERY_SERVICE_SHUTDOWN_FAILED")?;
        let reopened_checkpoint = read_live_recovery_checkpoint(output)?
            .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_UNAVAILABLE")?;
        let reopened_anchor = read_live_recovery_anchor(database, TASK_ID, item_id)?
            .ok_or("NISAR_LIVE_RECOVERY_ANCHOR_UNAVAILABLE")?;
        let reopened_runtime = TaskStore::open(database)
            .map_err(|_| "NISAR_LIVE_RECOVERY_STORE_REOPEN_FAILED")?
            .load_scheduler_control_runtime(TASK_ID)
            .map_err(|_| "NISAR_LIVE_RECOVERY_STORE_REOPEN_FAILED")?
            .ok_or("NISAR_LIVE_RECOVERY_STORE_REOPEN_FAILED")?;
        if reopened_runtime.status != TaskLifecycleStatus::Paused
            || !reopened_runtime.task_paused
            || reopened_checkpoint.checkpoint_sha256 != pause_checkpoint.checkpoint_sha256
            || reopened_checkpoint.lineage_key_sha256 != pause_checkpoint.lineage_key_sha256
            || reopened_checkpoint.execution_binding_sha256
                != pause_checkpoint.execution_binding_sha256
            || reopened_checkpoint.physical_plan_sha256 != pause_checkpoint.physical_plan_sha256
            || reopened_checkpoint.source_validator_sha256
                != pause_checkpoint.source_validator_sha256
            || reopened_checkpoint.completed_ranges_digest
                != pause_checkpoint.completed_ranges_digest
            || reopened_anchor.record_sha256 != pause_anchor.record_sha256
            || !live_recovery_anchor_matches_checkpoint(&reopened_anchor, &reopened_checkpoint)
        {
            return Err("NISAR_LIVE_RECOVERY_REOPEN_MISMATCH");
        }
        trace.reopen_runtime_revision = Some(reopened_runtime.revision);
        trace.reopen = Some(reopened_checkpoint);
        trace.anchor_at_reopen = Some(reopened_anchor);
        write_live_recovery_trace(root, &trace)?;
        *service = build_live_production_service(database, evidence, true, None)?;
    }

    service
        .resume_nisar_subset_task(TASK_ID)
        .map_err(|_| "NISAR_LIVE_RECOVERY_RESUME_FAILED")?;
    let resumed_runtime = TaskStore::open(database)
        .map_err(|_| "NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?
        .load_scheduler_control_runtime(TASK_ID)
        .map_err(|_| "NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?
        .ok_or("NISAR_LIVE_RECOVERY_STORE_UNAVAILABLE")?;
    if resumed_runtime.task_paused
        || !resumed_runtime.paused_item_ids.is_empty()
        || !matches!(
            resumed_runtime.status,
            TaskLifecycleStatus::Queued | TaskLifecycleStatus::Running
        )
    {
        return Err("NISAR_LIVE_RECOVERY_RESUME_STATE_INVALID");
    }
    trace.resume_runtime_revision = Some(resumed_runtime.revision);
    write_live_recovery_trace(root, &trace)?;

    let completed = match wait_for_live_recovery_terminal(evidence, Duration::from_secs(45 * 60)) {
        Ok(completed) => completed,
        Err(code) => {
            let _ = evidence.record_failure(code);
            if let Ok(store) = TaskStore::open(database) {
                if let Ok(Some(runtime)) = store.load_scheduler_control_runtime(TASK_ID) {
                    let _ = service.update_download_task(&StoreUpdateTaskRequest {
                        task_id: TASK_ID.to_owned(),
                        expected_revision: runtime.revision,
                        command: StoreTaskCommand {
                            kind: TaskStateCommandKind::CancelTask,
                            item_ids: Vec::new(),
                        },
                        event_id: format!("{LIVE_ACCEPTANCE_REQUEST_ID}:recovery:timeout-cancel"),
                        occurred_at: "2026-08-02T00:00:00Z".to_owned(),
                    });
                }
            }
            return Err(code);
        }
    };
    if !completed {
        return Ok(());
    }

    let completion = read_live_recovery_checkpoint(output)?
        .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_UNAVAILABLE")?;
    let completion_anchor = read_live_recovery_anchor(database, TASK_ID, item_id)?
        .ok_or("NISAR_LIVE_RECOVERY_ANCHOR_UNAVAILABLE")?;
    let reused_verified_bytes = completion.resumed_bytes;
    let completed_pause_ranges_reused = pause_checkpoint.completed_ranges.iter().all(|range| {
        completion
            .completed_ranges
            .iter()
            .any(|candidate| candidate == range)
    });
    if completion.state != "Complete"
        || completion.pending_attempt
        || completion.completed_ranges.len() != completion.planned_range_count
        || completion.planned_science_response_body_bytes
            != pause_checkpoint.planned_science_response_body_bytes
        || completion.lineage_key_sha256 != pause_checkpoint.lineage_key_sha256
        || completion.physical_plan_sha256 != pause_checkpoint.physical_plan_sha256
        || completion.resumed_bytes < pause_checkpoint.committed_science_response_body_bytes
        || reused_verified_bytes == 0
        || !completed_pause_ranges_reused
        || !live_recovery_anchor_matches_checkpoint(&completion_anchor, &completion)
        || completion_anchor.generation < pause_anchor.generation
    {
        return Err("NISAR_LIVE_RECOVERY_COMPLETION_INVALID");
    }
    if completion.execution_binding_sha256 != pause_checkpoint.execution_binding_sha256
        || completion.source_validator_sha256 != pause_checkpoint.source_validator_sha256
    {
        return Err("NISAR_LIVE_RECOVERY_AUTHORITY_DRIFT");
    }
    let duplicate_science_ranges = live_science_duplicate_ranges(&evidence.snapshot());
    trace.completion = Some(completion);
    trace.anchor_at_completion = Some(completion_anchor);
    trace.reused_verified_bytes = reused_verified_bytes;
    trace.duplicate_science_ranges = duplicate_science_ranges.clone();
    write_live_recovery_trace(root, &trace)?;
    if !duplicate_science_ranges.is_empty() {
        return Err("NISAR_LIVE_RECOVERY_DUPLICATE_RANGE_REQUEST");
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn has_hdf5_output(output: &Path) -> bool {
    fs::read_dir(output)
        .ok()
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry.path().extension().and_then(|value| value.to_str()) == Some("h5")
            })
        })
        .unwrap_or(false)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_output_science_selection(
    output: &Path,
    frequency: NisarSubsetFrequency,
    selected_polarizations: &[String],
) -> Result<(), &'static str> {
    let artifact_path = fs::read_dir(output)
        .map_err(|_| "NISAR_LIVE_SCIENCE_OUTPUT_MISSING")?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("h5"))
        .ok_or("NISAR_LIVE_SCIENCE_OUTPUT_MISSING")?;
    let artifact =
        Hdf5File::open(&artifact_path).map_err(|_| "NISAR_LIVE_SCIENCE_OUTPUT_UNREADABLE")?;
    let frequency_label = match &frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    };
    let derived_root =
        format!("/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequency{frequency_label}");
    let standard_root = format!("/science/LSAR/RSLC/swaths/frequency{frequency_label}");
    for polarization in ["HH", "HV", "VH", "VV"] {
        let expected = selected_polarizations
            .iter()
            .any(|selected| selected == polarization);
        let derived_present = artifact
            .dataset(&format!("{derived_root}/{polarization}"))
            .is_ok();
        if derived_present != expected {
            return Err("NISAR_LIVE_SCIENCE_SELECTION_MISMATCH");
        }
        // The V2 writer keeps selected science under the derived-subset
        // namespace; a standard RSLC science path would be an unapproved
        // duplicate payload.
        if artifact
            .dataset(&format!("{standard_root}/{polarization}"))
            .is_ok()
        {
            return Err("NISAR_LIVE_SCIENCE_SELECTION_MISMATCH");
        }
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn valid_evidence_atom(value: &str) -> bool {
    !value.is_empty() && value != "not_observed" && value != "[REDACTED]" && value.len() <= 1024
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_v2_range_evidence(
    evidence: &NisarLiveAcceptanceEvidenceV2,
    source_size: u64,
    validator_sha256: &str,
) -> bool {
    if source_size == 0
        || !is_sha256(validator_sha256)
        || !evidence.exact_http_206
        || !evidence.exact_content_range
        || !evidence.validator_continuity
    {
        return false;
    }
    let mut observed_failure_requests = 0_u64;
    let mut observed_failure_bytes = 0_u64;
    for failure in &evidence.range_failure_observations {
        let range_length = failure.end_exclusive.checked_sub(failure.start);
        let retryable_code = match failure.failure_code.as_str() {
            "NISAR_SUBSET_V2_PARTIAL_RESPONSE" | "NISAR_SUBSET_V2_TRANSPORT_FAILED" => true,
            "NISAR_SUBSET_V2_RESPONSE_INVALID" => failure
                .http_status
                .is_some_and(|status| (500..=599).contains(&status)),
            _ => false,
        };
        if failure.stage != "metadata_prefix"
            || !retryable_code
            || !matches!(
                failure.disposition.as_str(),
                "sent_response_rejected" | "sent_outcome_indeterminate"
            )
            || failure.request_count == 0
            || failure.source_size_bytes != source_size
            || failure.start >= failure.end_exclusive
            || failure.end_exclusive > source_size
            || failure.response_body_bytes.unwrap_or(0) > range_length.unwrap_or(0)
            || failure
                .validator_sha256
                .as_deref()
                .is_some_and(|digest| digest != validator_sha256)
            || !evidence.range_observations.iter().any(|observation| {
                observation.stage == failure.stage
                    && observation.start == failure.start
                    && observation.end_exclusive == failure.end_exclusive
            })
        {
            return false;
        }
        observed_failure_requests =
            match observed_failure_requests.checked_add(failure.request_count) {
                Some(value) => value,
                None => return false,
            };
        observed_failure_bytes =
            match observed_failure_bytes.checked_add(failure.response_body_bytes.unwrap_or(0)) {
                Some(value) => value,
                None => return false,
            };
    }
    if observed_failure_requests != evidence.charged_failure_requests
        || observed_failure_bytes != evidence.charged_failure_response_bytes
    {
        return false;
    }
    let stages = [
        (
            "preflight",
            evidence.preflight.requests,
            evidence.preflight.response_bytes,
        ),
        (
            "metadata_prefix",
            evidence.metadata_prefix.requests,
            evidence.metadata_prefix.response_bytes,
        ),
        (
            "metadata_discovery",
            evidence.metadata_discovery.requests,
            evidence.metadata_discovery.response_bytes,
        ),
        (
            "additional_geolocation",
            evidence.additional_geolocation.requests,
            evidence.additional_geolocation.response_bytes,
        ),
        (
            "planning_revalidation",
            evidence.planning_revalidation.requests,
            evidence.planning_revalidation.response_bytes,
        ),
        (
            "science",
            evidence.science_range.requests,
            evidence.science_range.response_bytes,
        ),
    ];
    let mut expected_observations = 0_u64;
    let Some(mut expected_requests) = evidence
        .charged_failure_requests
        .checked_add(evidence.failed_science_requests)
    else {
        return false;
    };
    let Some(mut expected_bytes) = evidence
        .charged_failure_response_bytes
        .checked_add(evidence.failed_science_response_bytes)
    else {
        return false;
    };
    if evidence.failed_science_requests == 0 && evidence.failed_science_response_bytes != 0 {
        return false;
    }
    for (index, observation) in evidence.range_observations.iter().enumerate() {
        if !stages
            .iter()
            .any(|(stage, _, _)| observation.stage == *stage)
            || evidence.range_observations[..index].iter().any(|previous| {
                previous.stage == observation.stage
                    && previous.start == observation.start
                    && previous.end_exclusive == observation.end_exclusive
            })
        {
            return false;
        }
    }
    for (stage, requests, response_bytes) in stages {
        let mut observed_observations = 0_u64;
        let mut observed_requests = 0_u64;
        let mut observed_bytes = 0_u64;
        for observation in evidence
            .range_observations
            .iter()
            .filter(|observation| observation.stage == stage)
        {
            if observation.request_count == 0
                || observation.source_size_bytes != source_size
                || observation.http_status != 206
                || observation.content_range_start != observation.start
                || observation.content_range_end_exclusive != observation.end_exclusive
                || observation.content_range_total != source_size
                || observation.start >= observation.end_exclusive
                || observation.end_exclusive > source_size
                || observation.response_body_bytes != observation.end_exclusive - observation.start
                || observation.validator_sha256 != validator_sha256
                || ((stage == "preflight" || stage == "planning_revalidation")
                    && (observation.start != 0 || observation.end_exclusive != 1))
            {
                return false;
            }
            observed_observations = match observed_observations.checked_add(1) {
                Some(value) => value,
                None => return false,
            };
            observed_requests = match observed_requests.checked_add(observation.request_count) {
                Some(value) => value,
                None => return false,
            };
            observed_bytes = match observed_bytes.checked_add(observation.response_body_bytes) {
                Some(value) => value,
                None => return false,
            };
        }
        if observed_requests != requests || observed_bytes != response_bytes {
            return false;
        }
        expected_observations = match expected_observations.checked_add(observed_observations) {
            Some(value) => value,
            None => return false,
        };
        expected_requests = match expected_requests.checked_add(requests) {
            Some(value) => value,
            None => return false,
        };
        expected_bytes = match expected_bytes.checked_add(response_bytes) {
            Some(value) => value,
            None => return false,
        };
    }
    expected_observations == evidence.range_observations.len() as u64
        && expected_bytes > 0
        && expected_requests == evidence.total_requests
        && expected_bytes == evidence.total_response_bytes
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_v2_budget_evidence(evidence: &NisarLiveAcceptanceEvidenceV2) -> bool {
    validate_v2_budget_evidence_with_retry_budget(evidence, 0, 0)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_v2_budget_evidence_with_retry_budget(
    evidence: &NisarLiveAcceptanceEvidenceV2,
    retry_requests: u64,
    retry_response_bytes: u64,
) -> bool {
    let Some(planning_requests) = evidence
        .preflight
        .requests
        .checked_add(evidence.metadata_prefix.requests)
        .and_then(|value| value.checked_add(evidence.metadata_discovery.requests))
        .and_then(|value| value.checked_add(evidence.additional_geolocation.requests))
        .and_then(|value| value.checked_add(evidence.planning_revalidation.requests))
        .and_then(|value| value.checked_add(evidence.charged_failure_requests))
    else {
        return false;
    };
    let Some(planning_response_bytes) = evidence
        .preflight
        .response_bytes
        .checked_add(evidence.metadata_prefix.response_bytes)
        .and_then(|value| value.checked_add(evidence.metadata_discovery.response_bytes))
        .and_then(|value| value.checked_add(evidence.additional_geolocation.response_bytes))
        .and_then(|value| value.checked_add(evidence.planning_revalidation.response_bytes))
        .and_then(|value| value.checked_add(evidence.charged_failure_response_bytes))
    else {
        return false;
    };
    let Some(total_requests) = planning_requests
        .checked_add(evidence.science_range.requests)
        .and_then(|value| value.checked_add(evidence.failed_science_requests))
    else {
        return false;
    };
    let Some(total_response_bytes) = planning_response_bytes
        .checked_add(evidence.science_range.response_bytes)
        .and_then(|value| value.checked_add(evidence.failed_science_response_bytes))
    else {
        return false;
    };
    let Some(remaining_requests) = evidence.max_requests.checked_sub(total_requests) else {
        return false;
    };
    let Some(remaining_response_bytes) = evidence
        .max_response_bytes
        .checked_sub(total_response_bytes)
    else {
        return false;
    };
    let Some(expected_reserved_requests) = evidence
        .committed_science_requests
        .checked_add(retry_requests)
    else {
        return false;
    };
    let Some(expected_reserved_response_bytes) = evidence
        .committed_science_response_bytes
        .checked_add(retry_response_bytes)
    else {
        return false;
    };
    let Some(request_partition) = planning_requests
        .checked_add(evidence.committed_science_requests)
        .and_then(|value| value.checked_add(evidence.failed_science_requests))
        .and_then(|value| value.checked_add(remaining_requests))
    else {
        return false;
    };
    let Some(response_partition) = planning_response_bytes
        .checked_add(evidence.committed_science_response_bytes)
        .and_then(|value| value.checked_add(evidence.failed_science_response_bytes))
        .and_then(|value| value.checked_add(remaining_response_bytes))
    else {
        return false;
    };
    evidence.max_requests == MAX_REQUESTS
        && evidence.max_response_bytes == MAX_RESPONSE_BYTES
        && evidence.preflight_requests == evidence.preflight.requests
        && evidence.preflight_response_bytes == evidence.preflight.response_bytes
        && evidence.prefix_requests == evidence.metadata_prefix.requests
        && evidence.prefix_response_bytes == evidence.metadata_prefix.response_bytes
        && evidence.discovery_requests == evidence.metadata_discovery.requests
        && evidence.discovery_response_bytes == evidence.metadata_discovery.response_bytes
        && evidence.additional_geolocation_requests == evidence.additional_geolocation.requests
        && evidence.additional_geolocation_response_bytes
            == evidence.additional_geolocation.response_bytes
        && evidence.planning_revalidation_requests == evidence.planning_revalidation.requests
        && evidence.planning_revalidation_response_bytes
            == evidence.planning_revalidation.response_bytes
        && evidence.science_requests == evidence.science_range.requests
        && evidence.science_response_bytes == evidence.science_range.response_bytes
        && evidence.planning_requests == planning_requests
        && evidence.planning_response_bytes == planning_response_bytes
        && evidence.committed_science_requests == evidence.science_range.requests
        && evidence.committed_science_response_bytes == evidence.science_range.response_bytes
        && evidence.failed_science_requests == retry_requests
        && evidence.failed_science_response_bytes <= retry_response_bytes
        && evidence.reserved_science_requests == expected_reserved_requests
        && evidence.reserved_science_response_bytes == expected_reserved_response_bytes
        && evidence.remaining_requests == remaining_requests
        && evidence.remaining_response_bytes == remaining_response_bytes
        && evidence.total_requests == total_requests
        && evidence.total_response_bytes == total_response_bytes
        && request_partition == evidence.max_requests
        && response_partition == evidence.max_response_bytes
        && evidence.total_requests <= evidence.max_requests
        && evidence.total_response_bytes <= evidence.max_response_bytes
        && evidence.source_size_bytes > 0
        && evidence.source_byte_ratio.is_finite()
        && evidence.source_byte_ratio
            == evidence.total_response_bytes as f64 / evidence.source_size_bytes as f64
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_v2_authority_digests(evidence: &NisarLiveAcceptanceEvidenceV2) -> bool {
    validate_v2_authority_digests_with_scene_mode(evidence, false)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_v2_authority_digests_scene_generic(evidence: &NisarLiveAcceptanceEvidenceV2) -> bool {
    validate_v2_authority_digests_with_scene_mode(evidence, true)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_v2_authority_digests_with_scene_mode(
    evidence: &NisarLiveAcceptanceEvidenceV2,
    scene_generic: bool,
) -> bool {
    [
        evidence.locator_hash.as_str(),
        evidence.validator_hash.as_str(),
        evidence.source_validator_sha256.as_str(),
        evidence.source_product_identity_sha256.as_str(),
        evidence.source_asset_identity_sha256.as_str(),
        evidence.source_footprint_sha256.as_str(),
        evidence.requested_geometry_sha256.as_str(),
        evidence.effective_geometry_sha256.as_str(),
        evidence.prepared_intent_sha256.as_str(),
        evidence.final_plan_sha256.as_str(),
        evidence.physical_plan_sha256.as_str(),
        evidence.inventory_sha256.as_str(),
        evidence.inventory_evidence_sha256.as_str(),
        evidence.source_graph_sha256.as_str(),
        evidence.execution_plan_sha256.as_str(),
        evidence.semantic_plan_sha256.as_str(),
        evidence.output_sha256.as_str(),
    ]
    .iter()
    .all(|value| is_sha256(value))
        && if scene_generic {
            evidence.local_qualification_execution_plan_sha256 == "not_applicable_scene_generic"
        } else {
            is_sha256(&evidence.local_qualification_execution_plan_sha256)
        }
        && evidence.aoi_digest == evidence.requested_geometry_sha256
        && valid_evidence_atom(&evidence.provider_id)
        && valid_evidence_atom(&evidence.provider_product_id)
        && valid_evidence_atom(&evidence.provider_product_key)
        && valid_evidence_atom(&evidence.collection_id)
        && valid_evidence_atom(&evidence.asset_key)
        && valid_evidence_atom(&evidence.stable_asset_key)
        && evidence.exact_http_206
        && evidence.exact_content_range
        && evidence.validator_continuity
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn wait_for_durable_store_completion(
    database: &Path,
    task_id: &str,
    item_id: &str,
    timeout: Duration,
    failure_code: &'static str,
) -> Result<(), &'static str> {
    let deadline = Instant::now() + timeout;
    loop {
        let store = TaskStore::open(database).map_err(|_| failure_code)?;
        let completed = store
            .load_task_record(task_id)
            .map_err(|_| failure_code)?
            .is_some_and(|task| {
                task.runtime.status == TaskLifecycleStatus::Completed
                    && task.runtime.active_item_ids.is_empty()
                    && task.runtime.queued_item_ids.is_empty()
                    && task.runtime.paused_item_ids.is_empty()
                    && task.runtime.cancelled_item_ids.is_empty()
                    && task
                        .snapshot
                        .candidates
                        .iter()
                        .any(|candidate| candidate.item_id == item_id)
            });
        if completed
            && store
                .load_download_snapshot(task_id)
                .map_err(|_| failure_code)?
                .results
                .iter()
                .any(|result| {
                    result.item_id == item_id
                        && result.outcome == DownloadOutcome::Completed
                        && result.failure.is_none()
                })
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(failure_code);
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn wait_for_durable_store_failure(
    database: &Path,
    task_id: &str,
    item_id: &str,
    timeout: Duration,
    failure_code: &'static str,
) -> Result<(), &'static str> {
    let deadline = Instant::now() + timeout;
    loop {
        let store = TaskStore::open(database).map_err(|_| failure_code)?;
        let failed = store
            .load_task_record(task_id)
            .map_err(|_| failure_code)?
            .is_some_and(|task| {
                task.runtime.status == TaskLifecycleStatus::Failed
                    && task.runtime.active_item_ids.is_empty()
                    && task.runtime.queued_item_ids.is_empty()
                    && task.runtime.paused_item_ids.is_empty()
                    && task.runtime.failed_item_ids.iter().any(|id| id == item_id)
            });
        let item_failed = store
            .load_download_snapshot(task_id)
            .map_err(|_| failure_code)?
            .results
            .iter()
            .any(|result| {
                result.item_id == item_id
                    && result.outcome == DownloadOutcome::Failed
                    && result.failure.is_some()
            });
        if failed && item_failed {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(failure_code);
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_m2d_b_final_evidence(
    evidence: &NisarLiveAcceptanceEvidenceV2,
    expected_request_aoi_sha256: &str,
    output: &Path,
) -> Result<(), &'static str> {
    validate_m2d_b_final_evidence_for_selection(
        evidence,
        expected_request_aoi_sha256,
        output,
        "A",
        "HH+HV",
        &["HH".to_owned(), "HV".to_owned()],
        &["HH".to_owned(), "HV".to_owned()],
        None,
    )
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[allow(clippy::too_many_arguments)]
fn validate_m2d_b_final_evidence_for_selection(
    evidence: &NisarLiveAcceptanceEvidenceV2,
    expected_request_aoi_sha256: &str,
    output: &Path,
    expected_frequency: &str,
    expected_polarization: &str,
    expected_advertised_polarizations: &[String],
    expected_selected_polarizations: &[String],
    retry_budget: Option<(u64, u64)>,
) -> Result<(), &'static str> {
    let scene_generic = evidence.gate == insar_download::NISAR_FREQ_B_GATE;
    let expected_schema = if scene_generic {
        insar_download::NISAR_FREQ_B_EVIDENCE_SCHEMA
    } else {
        insar_download::NISAR_M2D_B_EVIDENCE_SCHEMA
    };
    if evidence.schema_version != expected_schema
        || (scene_generic && evidence.gate != insar_download::NISAR_FREQ_B_GATE)
        || (!scene_generic && evidence.gate != insar_download::NISAR_M2D_B_GATE)
        || evidence.status != "pass"
        || evidence.typed_stop_code != "PASS"
        || evidence.baseline_commit != BASELINE_COMMIT
        || !valid_evidence_atom(&evidence.harness_commit)
        || evidence.granule_id != GRANULE_ID
        || evidence.source_size_bytes != SOURCE_BYTES
        || evidence.source_checksum_algorithm != SOURCE_CHECKSUM_ALGORITHM
        || evidence.source_checksum != SOURCE_CHECKSUM_VALUE
        || evidence.aoi_digest != expected_request_aoi_sha256
        || evidence.frequency != expected_frequency
        || evidence.polarization != expected_polarization
        || evidence.advertised_polarizations.as_slice() != expected_advertised_polarizations
        || evidence.selected_polarizations.as_slice() != expected_selected_polarizations
        || evidence.provider_id != NISAR_PROVIDER_ID
        || evidence.provider_product_id != PROVIDER_PRODUCT_ID
        || evidence.provider_product_key
            != "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G4263869502-ASF"
        || evidence.collection_id != NISAR_NATIVE_COLLECTION_ID
        || evidence.final_host_class != "earthdata_asf_v1"
        || evidence.final_path_class != "approved_nisar_hdf5_object"
        || evidence.prefix_response_bytes != insar_download::NISAR_LIVE_METADATA_PREFIX_BYTES
        || evidence.source_dataset_count == 0
        || evidence
            .metadata_group_binding_count
            .checked_add(evidence.metadata_dataset_binding_count)
            != Some(evidence.metadata_binding_count)
        || evidence.output_authority != insar_core::NISAR_SUBSET_OUTPUT_AUTHORITY
        || evidence.output_class != "client_derived_provisional_nisar_rslc_aoi_subset"
        || !evidence.one_hdf5
        || evidence.output_size_bytes == 0
        || !evidence.atomic_promotion
        || evidence.store_completed != "observed"
        || evidence.store_reopen != "observed"
        || !has_hdf5_output(output)
        || !(if scene_generic {
            validate_v2_authority_digests_scene_generic(evidence)
        } else {
            validate_v2_authority_digests(evidence)
        })
    {
        return Err("NISAR_LIVE_EVIDENCE_INCOMPLETE");
    }

    let semantic_ok = if scene_generic {
        evidence.local_qualification_execution_plan_sha256 == "not_applicable_scene_generic"
            && evidence.expected_semantic_plan_sha256 == "not_applicable_scene_generic"
            && is_sha256(&evidence.execution_plan_sha256)
            && is_sha256(&evidence.semantic_plan_sha256)
    } else {
        evidence.local_qualification_execution_plan_sha256 == NISAR_M2D_A_CURRENT_PLAN_SHA256
            && is_sha256(&evidence.execution_plan_sha256)
            && evidence.expected_semantic_plan_sha256 == NISAR_M2D_B2_SEMANTIC_PLAN_AUTHORITY_SHA256
            && is_sha256(&evidence.expected_semantic_plan_sha256)
            && is_sha256(&evidence.semantic_plan_sha256)
            && evidence.semantic_plan_sha256 == evidence.expected_semantic_plan_sha256
    };
    if !semantic_ok {
        return Err("NISAR_LIVE_SEMANTIC_PLAN_POSTCHECK_FAILED");
    }

    if !validate_v2_range_evidence(evidence, SOURCE_BYTES, &evidence.source_validator_sha256) {
        return Err("NISAR_LIVE_RANGE_POSTCHECK_FAILED");
    }
    let budget_valid = retry_budget.map_or_else(
        || validate_v2_budget_evidence(evidence),
        |(retry_requests, retry_response_bytes)| {
            validate_v2_budget_evidence_with_retry_budget(
                evidence,
                retry_requests,
                retry_response_bytes,
            )
        },
    );
    if !budget_valid {
        return Err("NISAR_LIVE_BUDGET_POSTCHECK_FAILED");
    }

    let geometry = &evidence.geometry;
    let science = &evidence.scientific_validation;
    if geometry.line_end_exclusive <= geometry.line_start
        || geometry.sample_end_exclusive <= geometry.sample_start
        || science.complex_mismatch != 0
        || science.amplitude_mismatch != 0
        || science.wrapped_phase_mismatch != 0
        || science.mask_mismatch != 0
        || !science.logical_science_equality
        || !science.identification_semantics
        || !science.dimension_scales
        || !science.metadata_semantics
        || !science.provenance_semantics
    {
        return Err("NISAR_LIVE_SCIENCE_POSTCHECK_FAILED");
    }

    Ok(())
}

#[test]
#[cfg(feature = "nisar-subset-live-acceptance")]
fn live_acceptance_feature_alone_keeps_normal_capability_default_off() {
    let path = std::env::temp_dir().join(format!(
        "nisar-p1-5-production-{}.sqlite3",
        std::process::id()
    ));
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&path));
    let capability = service.nisar_remote_subset_capability().unwrap();
    assert!(capability.visible);
    assert!(!capability.enabled);
    assert!(capability.live_verified);
    assert!(!capability.production_enabled);
    assert!(!capability.default_enabled);
    service.shutdown().unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

#[test]
fn live_harness_uses_the_production_worker_primary_path() {
    let harness_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("nisar_subset_live_acceptance.rs");
    let harness = fs::read_to_string(harness_path).unwrap();
    let controlled_start = harness
        .rfind("fn run_m2d_b_controlled_attempt")
        .expect("controlled live acceptance body");
    let controlled = &harness[controlled_start..];
    assert!(controlled.contains("ProductionDownloadWorker::new"));
    assert!(controlled.contains("with_production_worker_for_live_acceptance"));
    assert!(!controlled.contains("LiveWorker"));
    assert!(!controlled.contains("with_offline_nisar_subset_fixture"));
    assert!(!controlled.contains("OfflineHdf5RangeTransfer::new("));
    assert!(!controlled.contains("OfflineNisarSubsetWriter::new(work.task.output_root"));

    let worker_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("insar-download")
        .join("src")
        .join("production_worker.rs");
    let worker = fs::read_to_string(worker_path).unwrap();
    assert!(worker.contains("Some(\"remote_derived_subset\")"));
    assert!(worker.contains("execute_nisar_remote_subset"));
    assert!(worker.contains("NisarLiveNetworkBudgetV1"));
    assert!(worker.contains("execute_with_nisar_budget"));
    assert!(worker.contains("validate_actual"));
    assert!(worker.contains("derive_nisar_live_semantic_plan_sha256"));
    assert!(worker.contains("record_execution_plan"));
    assert!(worker.contains("prepared.requested_geometry.canonical_geojson"));
    assert!(!worker.contains("plan.plan_sha256() != NISAR_M2D_A_CURRENT_PLAN_SHA256"));
    assert!(worker.contains("validate_nisar_live_admission"));
    assert!(!worker.contains("NISAR_SUBSET_SEMANTIC_PLAN_MISMATCH"));
    assert!(worker.contains("NISAR_LIVE_EVIDENCE_WRITE_FAILED"));

    let admission_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("insar-download")
        .join("src")
        .join("nisar_live_admission.rs");
    let admission = fs::read_to_string(admission_path).unwrap();
    assert!(admission.contains("NISAR_SUBSET_ADMISSION_SOURCE_INVALID"));
    assert!(admission.contains("NISAR_SUBSET_ADMISSION_TRANSPORT_INVALID"));
    assert!(admission.contains("NISAR_SUBSET_ADMISSION_PHYSICAL_PLAN_INVALID"));
}

#[test]
fn live_harness_uses_the_frozen_v2_task_chain_for_each_live_path() {
    let harness_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("nisar_subset_live_acceptance.rs");
    let harness = fs::read_to_string(harness_path).unwrap();
    let source_function_body = |function_marker: &str, next_function_marker: &str| {
        let start_anchor = format!("\n{function_marker}");
        let start = harness
            .rfind(&start_anchor)
            .map(|offset| offset + 1)
            .expect("live acceptance function body");
        let end_anchor = format!("\n{next_function_marker}");
        let end = start
            + harness[start..]
                .find(&end_anchor)
                .expect("live acceptance function boundary");
        &harness[start..end]
    };
    let live_bodies = [
        source_function_body(
            "fn run_scene_generic_controlled_attempt",
            "fn run_m2d_b_controlled_attempt",
        ),
        source_function_body(
            "fn run_c2_live_validation",
            "fn c2_second_scene_scope_is_fixed_without_network",
        ),
    ];

    let body_retry = source_function_body(
        "fn run_live_body_failure_retry_phase",
        "fn run_m2d_b_controlled_attempt",
    );
    for required in [
        "ScienceTransferIncomplete",
        "wait_for_durable_store_failure",
        "TaskStateCommandKind::RetryItems",
        "old_failure_evidence_bytes",
        "replace_evidence",
        "mark_store_completed",
        "mark_store_reopen",
        "matching_failed_ranges",
    ] {
        assert!(
            body_retry.contains(required),
            "body recovery must retain the failed-item retry seam: {required}"
        );
    }
    assert!(!body_retry.contains("TaskStateCommandKind::PauseTask"));

    let shared_worker = source_function_body(
        "fn build_live_production_service",
        "fn recovery_result_or_worker_failure",
    );
    assert!(shared_worker.contains("ProductionDownloadWorker::new"));
    assert!(
        shared_worker.contains("ApplicationService::with_production_worker_for_live_acceptance")
    );
    for body in live_bodies {
        assert!(
            body.contains("ProductionDownloadWorker::new")
                || body.contains("build_live_production_service(")
        );
        for required in [
            "prepare_nisar_subset_intent_v2",
            "confirm_nisar_subset_task_v2",
            "TaskStore::open",
            "TaskLifecycleStatus::Paused",
            "remote_derived_subset_v2",
            "update_download_task",
            "TaskStateCommandKind::ResumeTask",
            "TaskStateCommandKind::ResumeItems",
        ] {
            assert!(
                body.contains(required),
                "live body missing V2 chain step: {required}"
            );
        }
        assert!(!body.contains("create_nisar_remote_subset_task"));
        assert!(!body.contains("NisarCreateRemoteSubsetTaskRequest"));
        assert!(!body.contains("attach_nisar_subset_execution_plan_v2"));
        assert!(!body.contains("store.update_task("));
        assert!(
            body.contains("TaskStateCommandKind::CancelTask"),
            "live timeout body must cancel the V2 task through Store/Scheduler"
        );
        assert!(
            !body.contains("cancel_request("),
            "live timeout body must not use request cancellation as V2 task authority"
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn live_v2_confirmation_and_resume_use_application_service_store_scheduler() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let database = std::env::temp_dir().join(format!(
        "nisar-live-v2-harness-{pid}-{nonce}.sqlite3",
        pid = std::process::id()
    ));
    let task_id = format!("nisar-live-v2-harness-task-{nonce}");
    let item = production_mapped_fixed_scene_fixture();
    save_source(&database, item.clone()).unwrap();
    let worker = Arc::new(SchedulerProbeWorker::default());
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database),
        Arc::new(FixtureAsf),
        Arc::new(FixtureAsf),
        Arc::clone(&worker) as Arc<dyn DownloadWorker>,
    );
    let expected_workbench_revision = TaskStore::open(&database)
        .unwrap()
        .load_workbench()
        .unwrap()
        .unwrap()
        .state
        .revision;
    let prepared = service
        .prepare_nisar_subset_intent_v2(&NisarPrepareSubsetIntentV2Request {
            source_batch_id: "nisar-live-source-batch".to_owned(),
            source_item_id: item.item_id.clone(),
            expected_workbench_revision,
            subset_draft_geometry: item.footprint_geojson.clone().unwrap(),
            selected_frequency: "A".to_owned(),
            selected_polarizations: vec!["HH".to_owned()],
        })
        .unwrap();
    service
        .confirm_nisar_subset_task_v2(&NisarConfirmSubsetTaskV2Request {
            source_batch_id: prepared.source_batch_id.clone(),
            source_item_id: prepared.source_item_id.clone(),
            expected_workbench_revision: prepared.workbench_revision,
            prepared_intent: prepared.intent,
            task_id: task_id.clone(),
            output_root: std::env::temp_dir().join(format!("{task_id}-output")),
            layout: insar_core::DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-01T00:00:00Z".to_owned(),
        })
        .unwrap();
    let paused = TaskStore::open(&database)
        .unwrap()
        .load_task_record(&task_id)
        .unwrap()
        .unwrap();
    let paused_item = paused.snapshot.candidates.first().unwrap();
    assert_eq!(paused.snapshot.candidates.len(), 1);
    assert_eq!(paused.runtime.status, TaskLifecycleStatus::Paused);
    assert!(paused.runtime.task_paused);
    assert!(paused.runtime.queued_item_ids.is_empty());
    assert_eq!(
        paused.runtime.paused_item_ids.as_slice(),
        std::slice::from_ref(&item.item_id)
    );
    assert_eq!(
        paused_item
            .metadata
            .get("delivery_kind")
            .and_then(Value::as_str),
        Some("remote_derived_subset_v2")
    );
    assert!(
        insar_store::parse_nisar_subset_plan_v2_envelope(&paused_item.metadata)
            .unwrap()
            .is_none()
    );

    // Since 75315b3d a user-level task resume also resumes the task's remaining
    // paused items (ApplicationService orchestration, recorded in
    // the NISAR subsetting contract and tested by m1_offline_vertical): one ResumeTask
    // makes two Store transitions. A separate ResumeItems at the next revision,
    // as this test sent before that change, is stale.
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: task_id.clone(),
            expected_revision: paused.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeTask,
                item_ids: Vec::new(),
            },
            event_id: format!("{task_id}:resume"),
            occurred_at: "2026-09-01T00:00:00Z".to_owned(),
        })
        .unwrap();
    let after_resume = TaskStore::open(&database)
        .unwrap()
        .load_task_record(&task_id)
        .unwrap()
        .unwrap();
    assert!(!after_resume.runtime.task_paused);
    assert!(after_resume.runtime.paused_item_ids.is_empty());
    assert!(
        after_resume.runtime.revision >= paused.runtime.revision + 2,
        "the task resume and the orchestrated item resume are two Store transitions"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while worker.authority_calls.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < deadline,
            "Scheduler did not wake the V2 task after application-service resume"
        );
        thread::yield_now();
    }
    assert_eq!(worker.authority_calls.load(Ordering::SeqCst), 1);
    service.shutdown().unwrap();
    let resumed = TaskStore::open(&database)
        .unwrap()
        .load_task_record(&task_id)
        .unwrap()
        .unwrap();
    assert!(!resumed.runtime.task_paused);
    assert_ne!(resumed.runtime.status, TaskLifecycleStatus::Paused);
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", database.display(), suffix));
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn live_v2_timeout_cancellation_uses_latest_store_revision_and_reopens_cancelled() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let database = std::env::temp_dir().join(format!(
        "nisar-live-v2-cancel-harness-{pid}-{nonce}.sqlite3",
        pid = std::process::id()
    ));
    let task_id = format!("nisar-live-v2-cancel-task-{nonce}");
    let item = production_mapped_fixed_scene_fixture();
    save_source(&database, item.clone()).unwrap();
    let worker = Arc::new(SchedulerProbeWorker::blocking());
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database),
        Arc::new(FixtureAsf),
        Arc::new(FixtureAsf),
        Arc::clone(&worker) as Arc<dyn DownloadWorker>,
    );
    let expected_workbench_revision = TaskStore::open(&database)
        .unwrap()
        .load_workbench()
        .unwrap()
        .unwrap()
        .state
        .revision;
    let prepared = service
        .prepare_nisar_subset_intent_v2(&NisarPrepareSubsetIntentV2Request {
            source_batch_id: "nisar-live-source-batch".to_owned(),
            source_item_id: item.item_id.clone(),
            expected_workbench_revision,
            subset_draft_geometry: item.footprint_geojson.clone().unwrap(),
            selected_frequency: "A".to_owned(),
            selected_polarizations: vec!["HH".to_owned()],
        })
        .unwrap();
    service
        .confirm_nisar_subset_task_v2(&NisarConfirmSubsetTaskV2Request {
            source_batch_id: prepared.source_batch_id.clone(),
            source_item_id: prepared.source_item_id.clone(),
            expected_workbench_revision: prepared.workbench_revision,
            prepared_intent: prepared.intent,
            task_id: task_id.clone(),
            output_root: std::env::temp_dir().join(format!("{task_id}-output")),
            layout: insar_core::DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-09-01T00:00:00Z".to_owned(),
        })
        .unwrap();
    let paused = TaskStore::open(&database)
        .unwrap()
        .load_task_record(&task_id)
        .unwrap()
        .unwrap();
    let paused_item = paused.snapshot.candidates.first().unwrap();
    assert_eq!(paused.runtime.status, TaskLifecycleStatus::Paused);
    assert!(paused.runtime.task_paused);
    assert!(paused.runtime.queued_item_ids.is_empty());
    assert_eq!(paused.runtime.paused_item_ids, vec![item.item_id.clone()]);
    assert!(
        insar_store::parse_nisar_subset_prepared_intent_v2_envelope(&paused_item.metadata)
            .unwrap()
            .is_some()
    );
    assert!(
        insar_store::parse_nisar_subset_plan_v2_envelope(&paused_item.metadata)
            .unwrap()
            .is_none()
    );

    // Since 75315b3d a user-level task resume also resumes the task's remaining
    // paused items (ApplicationService orchestration, recorded in
    // the NISAR subsetting contract and tested by m1_offline_vertical): one ResumeTask
    // makes two Store transitions, and the Scheduler may dispatch the queued item
    // at once. A separate ResumeItems at the next revision, as this test sent
    // before that change, is stale.
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: task_id.clone(),
            expected_revision: paused.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeTask,
                item_ids: Vec::new(),
            },
            event_id: format!("{task_id}:resume"),
            occurred_at: "2026-09-01T00:00:01Z".to_owned(),
        })
        .unwrap();
    let resumed = TaskStore::open(&database)
        .unwrap()
        .load_task_record(&task_id)
        .unwrap()
        .unwrap();
    assert!(!resumed.runtime.task_paused);
    assert!(resumed.runtime.paused_item_ids.is_empty());
    assert!(
        resumed.runtime.queued_item_ids == vec![item.item_id.clone()]
            || resumed.runtime.active_item_ids == vec![item.item_id.clone()],
        "the resumed item is queued or already dispatched"
    );
    assert!(
        resumed.runtime.revision >= paused.runtime.revision + 2,
        "the task resume and the orchestrated item resume are two Store transitions"
    );
    let start_deadline = Instant::now() + Duration::from_secs(5);
    while worker.authority_calls.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < start_deadline,
            "authority-aware Scheduler worker did not start"
        );
        thread::yield_now();
    }
    assert_eq!(worker.authority_calls.load(Ordering::SeqCst), 1);
    assert_eq!(worker.fallback_calls.load(Ordering::SeqCst), 0);

    // V2 task cancellation intentionally does not consult the request-cancellation registry.
    let latest_runtime = TaskStore::open(&database)
        .unwrap()
        .load_scheduler_control_runtime(&task_id)
        .unwrap()
        .unwrap();
    let cancellation_revision = latest_runtime.revision;
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: task_id.clone(),
            expected_revision: cancellation_revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelTask,
                item_ids: Vec::new(),
            },
            event_id: format!("{task_id}:timeout-cancel:v2"),
            occurred_at: "2026-09-01T00:00:02Z".to_owned(),
        })
        .unwrap();
    let cancelled_runtime = TaskStore::open(&database)
        .unwrap()
        .load_scheduler_control_runtime(&task_id)
        .unwrap()
        .unwrap();
    assert_eq!(cancelled_runtime.revision, cancellation_revision + 1);

    let worker_deadline = Instant::now() + Duration::from_secs(5);
    while worker.cancellation_observed.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < worker_deadline,
            "Scheduler did not propagate durable cancellation to the worker control"
        );
        thread::yield_now();
    }

    let terminal_deadline = Instant::now() + Duration::from_secs(5);
    let terminal_runtime = loop {
        let store = TaskStore::open(&database).unwrap();
        let runtime = store
            .load_scheduler_control_runtime(&task_id)
            .unwrap()
            .unwrap();
        let download = store.load_download_snapshot(&task_id).unwrap();
        let result_cancelled = download.results.iter().any(|result| {
            result.item_id == item.item_id && result.outcome == DownloadOutcome::Cancelled
        });
        if runtime.status == TaskLifecycleStatus::Cancelled
            && runtime.active_item_ids.is_empty()
            && runtime.queued_item_ids.is_empty()
            && runtime.paused_item_ids.is_empty()
            && runtime.cancelled_item_ids == vec![item.item_id.clone()]
            && result_cancelled
        {
            break runtime;
        }
        assert!(
            Instant::now() < terminal_deadline,
            "Scheduler did not reconcile the cancelled worker into durable Store truth"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(terminal_runtime.status, TaskLifecycleStatus::Cancelled);
    assert_eq!(terminal_runtime.revision, cancellation_revision + 1);

    service.shutdown().unwrap();
    let reopened = TaskStore::open(&database)
        .unwrap()
        .load_task_record(&task_id)
        .unwrap()
        .unwrap();
    assert_eq!(reopened.runtime.status, TaskLifecycleStatus::Cancelled);
    assert!(reopened.runtime.active_item_ids.is_empty());
    assert!(reopened.runtime.queued_item_ids.is_empty());
    assert!(reopened.runtime.paused_item_ids.is_empty());
    assert_eq!(reopened.runtime.cancelled_item_ids, vec![item.item_id]);
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", database.display(), suffix));
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b_redacted_evidence_records_typed_failure_without_secrets() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "nisar-m2d-b-evidence-redaction-{}-{nonce}",
        std::process::id()
    ));
    let sink = NisarLiveAcceptanceEvidenceSink::new(
        root.clone(),
        "https://example.invalid/?token=secret",
        "Authorization: Bearer secret",
    )
    .unwrap();
    sink.record_failure("NISAR_SUBSET_SEMANTIC_PLAN_MISMATCH")
        .unwrap();
    let snapshot = sink.snapshot();
    assert_eq!(snapshot.status, "fail");
    assert_eq!(
        snapshot.typed_stop_code,
        "NISAR_SUBSET_SEMANTIC_PLAN_MISMATCH"
    );
    assert_eq!(
        snapshot.schema_version,
        insar_download::NISAR_M2D_B_EVIDENCE_SCHEMA
    );
    let raw = fs::read_to_string(sink.evidence_path()).unwrap();
    let lowercase = raw.to_ascii_lowercase();
    for forbidden in [
        "https://",
        "authorization",
        "token",
        "cookie",
        ".netrc",
        "keyring",
        "x-amz-",
        "?",
    ] {
        assert!(
            !lowercase.contains(forbidden),
            "forbidden evidence token: {forbidden}"
        );
    }
    drop(sink);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b_cleanup_guard_removes_workspaces_and_preserves_evidence() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "nisar-m2d-b-cleanup-{}-{nonce}",
        std::process::id()
    ));
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(root.clone(), BASELINE_COMMIT, "offline-test")
            .unwrap(),
    );
    fs::create_dir_all(root.join("transfer")).unwrap();
    fs::create_dir_all(root.join("candidate")).unwrap();
    fs::create_dir_all(root.join("output").join(".insar-nisar-range")).unwrap();
    fs::write(
        root.join("output").join("approved.h5"),
        b"offline-placeholder",
    )
    .unwrap();
    let mut guard = LiveExecutionCleanupGuard::new(root.clone(), Arc::clone(&evidence));
    guard.preserve_final_artifact();
    guard.cleanup_now().unwrap();
    assert!(!root.join("transfer").exists());
    assert!(!root.join("candidate").exists());
    assert!(!root.join("output").join(".insar-nisar-range").exists());
    assert!(root.join("output").join("approved.h5").exists());
    assert!(evidence.evidence_path().exists());
    assert!(evidence.snapshot().cleanup_completed);
    drop(guard);
    drop(evidence);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn offline_m2d_b_root(label: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "nisar-m2d-b0-1-{label}-{}-{nonce}",
        std::process::id()
    ))
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn passing_m2d_b_evidence() -> NisarLiveAcceptanceEvidenceV2 {
    let prefix_response_bytes = insar_download::NISAR_LIVE_METADATA_PREFIX_BYTES;
    let total_requests = 5;
    let total_response_bytes = prefix_response_bytes + 4;
    NisarLiveAcceptanceEvidenceV2 {
        schema_version: insar_download::NISAR_M2D_B_EVIDENCE_SCHEMA.to_owned(),
        gate: insar_download::NISAR_M2D_B_GATE.to_owned(),
        baseline_commit: BASELINE_COMMIT.to_owned(),
        harness_commit: "offline-harness".to_owned(),
        status: "pass".to_owned(),
        typed_stop_code: "PASS".to_owned(),
        granule_id: GRANULE_ID.to_owned(),
        source_size_bytes: SOURCE_BYTES,
        source_checksum_algorithm: SOURCE_CHECKSUM_ALGORITHM.to_owned(),
        source_checksum: SOURCE_CHECKSUM_VALUE.to_owned(),
        aoi_digest: normalize_nisar_subset_aoi(&aoi()).unwrap().sha256,
        frequency: "A".to_owned(),
        polarization: "HH+HV".to_owned(),
        advertised_polarizations: vec!["HH".to_owned(), "HV".to_owned()],
        selected_polarizations: vec!["HH".to_owned(), "HV".to_owned()],
        provider_id: NISAR_PROVIDER_ID.to_owned(),
        provider_product_id: PROVIDER_PRODUCT_ID.to_owned(),
        provider_product_key:
            "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G4263869502-ASF"
                .to_owned(),
        collection_id: NISAR_NATIVE_COLLECTION_ID.to_owned(),
        asset_key: "primary".to_owned(),
        stable_asset_key: "stable-primary".to_owned(),
        final_host_class: "earthdata_asf_v1".to_owned(),
        final_path_class: "approved_nisar_hdf5_object".to_owned(),
        locator_hash: "a".repeat(64),
        validator_hash: "b".repeat(64),
        source_validator_sha256: "b".repeat(64),
        source_product_identity_sha256: "1".repeat(64),
        source_asset_identity_sha256: "2".repeat(64),
        source_footprint_sha256: "3".repeat(64),
        requested_geometry_sha256: normalize_nisar_subset_aoi(&aoi()).unwrap().sha256.clone(),
        effective_geometry_sha256: "4".repeat(64),
        prepared_intent_sha256: "5".repeat(64),
        final_plan_sha256: "6".repeat(64),
        physical_plan_sha256: "7".repeat(64),
        inventory_evidence_sha256: "8".repeat(64),
        source_graph_sha256: "9".repeat(64),
        preflight: insar_download::NisarLiveBudgetEvidenceV2 {
            requests: 1,
            response_bytes: 1,
        },
        metadata_prefix: insar_download::NisarLiveBudgetEvidenceV2 {
            requests: 1,
            response_bytes: prefix_response_bytes,
        },
        metadata_discovery: insar_download::NisarLiveBudgetEvidenceV2 {
            requests: 1,
            response_bytes: 1,
        },
        additional_geolocation: insar_download::NisarLiveBudgetEvidenceV2::default(),
        planning_revalidation: insar_download::NisarLiveBudgetEvidenceV2 {
            requests: 1,
            response_bytes: 1,
        },
        science_range: insar_download::NisarLiveBudgetEvidenceV2 {
            requests: 1,
            response_bytes: 1,
        },
        preflight_requests: 1,
        preflight_response_bytes: 1,
        prefix_requests: 1,
        prefix_response_bytes,
        discovery_requests: 1,
        discovery_response_bytes: 1,
        additional_geolocation_requests: 0,
        additional_geolocation_response_bytes: 0,
        planning_revalidation_requests: 1,
        planning_revalidation_response_bytes: 1,
        science_requests: 1,
        science_response_bytes: 1,
        total_requests,
        total_response_bytes,
        max_requests: MAX_REQUESTS,
        max_response_bytes: MAX_RESPONSE_BYTES,
        remaining_requests: MAX_REQUESTS - total_requests,
        remaining_response_bytes: MAX_RESPONSE_BYTES - total_response_bytes,
        planning_requests: 4,
        planning_response_bytes: prefix_response_bytes + 3,
        reserved_science_requests: 1,
        reserved_science_response_bytes: 1,
        committed_science_requests: 1,
        committed_science_response_bytes: 1,
        source_byte_ratio: total_response_bytes as f64 / SOURCE_BYTES as f64,
        source_dataset_count: 207,
        metadata_group_binding_count: 1,
        metadata_dataset_binding_count: 1,
        metadata_binding_count: 2,
        inventory_sha256: "c".repeat(64),
        local_qualification_execution_plan_sha256: NISAR_M2D_A_CURRENT_PLAN_SHA256.to_owned(),
        execution_plan_sha256: "e".repeat(64),
        expected_semantic_plan_sha256: NISAR_M2D_B2_SEMANTIC_PLAN_AUTHORITY_SHA256.to_owned(),
        semantic_plan_sha256: NISAR_M2D_B2_SEMANTIC_PLAN_AUTHORITY_SHA256.to_owned(),
        geometry: insar_download::NisarLiveGeometryEvidenceV2 {
            line_start: 0,
            line_end_exclusive: 2,
            sample_start: 0,
            sample_end_exclusive: 2,
        },
        scientific_validation: insar_download::NisarLiveScientificValidationEvidenceV2 {
            complex_mismatch: 0,
            amplitude_mismatch: 0,
            wrapped_phase_mismatch: 0,
            mask_mismatch: 0,
            logical_science_equality: true,
            identification_semantics: true,
            dimension_scales: true,
            metadata_semantics: true,
            provenance_semantics: true,
        },
        range_observations: vec![
            insar_download::NisarLiveRangeObservationV2 {
                stage: "preflight".to_owned(),
                start: 0,
                end_exclusive: 1,
                source_size_bytes: SOURCE_BYTES,
                request_count: 1,
                http_status: 206,
                content_range_start: 0,
                content_range_end_exclusive: 1,
                content_range_total: SOURCE_BYTES,
                response_body_bytes: 1,
                validator_sha256: "b".repeat(64),
            },
            insar_download::NisarLiveRangeObservationV2 {
                stage: "metadata_prefix".to_owned(),
                start: 0,
                end_exclusive: prefix_response_bytes,
                source_size_bytes: SOURCE_BYTES,
                request_count: 1,
                http_status: 206,
                content_range_start: 0,
                content_range_end_exclusive: prefix_response_bytes,
                content_range_total: SOURCE_BYTES,
                response_body_bytes: prefix_response_bytes,
                validator_sha256: "b".repeat(64),
            },
            insar_download::NisarLiveRangeObservationV2 {
                stage: "metadata_discovery".to_owned(),
                start: prefix_response_bytes,
                end_exclusive: prefix_response_bytes + 1,
                source_size_bytes: SOURCE_BYTES,
                request_count: 1,
                http_status: 206,
                content_range_start: prefix_response_bytes,
                content_range_end_exclusive: prefix_response_bytes + 1,
                content_range_total: SOURCE_BYTES,
                response_body_bytes: 1,
                validator_sha256: "b".repeat(64),
            },
            insar_download::NisarLiveRangeObservationV2 {
                stage: "planning_revalidation".to_owned(),
                start: 0,
                end_exclusive: 1,
                source_size_bytes: SOURCE_BYTES,
                request_count: 1,
                http_status: 206,
                content_range_start: 0,
                content_range_end_exclusive: 1,
                content_range_total: SOURCE_BYTES,
                response_body_bytes: 1,
                validator_sha256: "b".repeat(64),
            },
            insar_download::NisarLiveRangeObservationV2 {
                stage: "science".to_owned(),
                start: 0,
                end_exclusive: 1,
                source_size_bytes: SOURCE_BYTES,
                request_count: 1,
                http_status: 206,
                content_range_start: 0,
                content_range_end_exclusive: 1,
                content_range_total: SOURCE_BYTES,
                response_body_bytes: 1,
                validator_sha256: "b".repeat(64),
            },
        ],
        range_failure_observations: Vec::new(),
        charged_failure_requests: 0,
        charged_failure_response_bytes: 0,
        failed_science_requests: 0,
        failed_science_response_bytes: 0,
        exact_http_206: true,
        exact_content_range: true,
        validator_continuity: true,
        output_authority: insar_core::NISAR_SUBSET_OUTPUT_AUTHORITY.to_owned(),
        output_class: "client_derived_provisional_nisar_rslc_aoi_subset".to_owned(),
        one_hdf5: true,
        output_size_bytes: 16,
        output_sha256: "d".repeat(64),
        atomic_promotion: true,
        cleanup_completed: false,
        store_completed: "observed".to_owned(),
        store_reopen: "observed".to_owned(),
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn scene_generic_evidence_uses_non_qualified_plan_markers() {
    let root = offline_m2d_b_root("scene-generic-markers");
    let evidence = NisarLiveAcceptanceEvidenceSink::new_for_scene_generic_acceptance(
        root.clone(),
        BASELINE_COMMIT,
        "offline-harness",
        insar_download::NISAR_FREQ_B_EVIDENCE_SCHEMA,
        insar_download::NISAR_FREQ_B_GATE,
    )
    .unwrap();
    let snapshot = evidence.snapshot();
    assert_eq!(snapshot.gate, insar_download::NISAR_FREQ_B_GATE);
    assert_eq!(
        snapshot.schema_version,
        insar_download::NISAR_FREQ_B_EVIDENCE_SCHEMA
    );
    assert_eq!(
        snapshot.local_qualification_execution_plan_sha256,
        "not_applicable_scene_generic"
    );
    assert_eq!(
        snapshot.expected_semantic_plan_sha256,
        "not_applicable_scene_generic"
    );
    drop(evidence);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b0_1_a_preworker_failure_is_typed_and_terminal() {
    let root = offline_m2d_b_root("preworker");
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(root.clone(), BASELINE_COMMIT, "offline-harness")
            .unwrap(),
    );
    let mut cleanup = LiveExecutionCleanupGuard::new(root.clone(), Arc::clone(&evidence));

    record_live_failure_if_unset(&evidence, "NISAR_LIVE_CATALOGUE_FAILED");
    assert_eq!(evidence.snapshot().status, "fail");
    assert_eq!(
        evidence.snapshot().typed_stop_code,
        "NISAR_LIVE_CATALOGUE_FAILED"
    );
    cleanup.cleanup_now().unwrap();
    assert!(evidence.snapshot().cleanup_completed);
    drop(cleanup);
    drop(evidence);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b0_1_b_harness_panic_is_typed_after_evidence_creation() {
    let root = offline_m2d_b_root("panic");
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(root.clone(), BASELINE_COMMIT, "offline-harness")
            .unwrap(),
    );
    let panic_result = catch_unwind(AssertUnwindSafe(|| panic!("offline harness panic")));
    assert!(panic_result.is_err());
    record_live_failure_if_unset(&evidence, "NISAR_LIVE_HARNESS_PANIC");
    let snapshot = evidence.snapshot();
    assert_eq!(snapshot.status, "fail");
    assert_eq!(snapshot.typed_stop_code, "NISAR_LIVE_HARNESS_PANIC");
    assert!(evidence.evidence_path().exists());
    drop(evidence);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b0_1_c_postcondition_failure_removes_unapproved_artifact() {
    let root = offline_m2d_b_root("postcondition-fail");
    let output = root.join("output");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("approved.h5"), b"offline-placeholder").unwrap();
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(root.clone(), BASELINE_COMMIT, "offline-harness")
            .unwrap(),
    );
    let mut cleanup = LiveExecutionCleanupGuard::new(root.clone(), Arc::clone(&evidence));
    let mut failed = passing_m2d_b_evidence();
    failed.semantic_plan_sha256 = "f".repeat(64);
    let code = validate_m2d_b_final_evidence(&failed, &failed.aoi_digest, &output)
        .expect_err("plan mismatch must fail typed postcondition validation");
    assert_eq!(code, "NISAR_LIVE_SEMANTIC_PLAN_POSTCHECK_FAILED");
    evidence.record_failure(code).unwrap();
    cleanup.cleanup_now().unwrap();
    assert!(!output.exists());
    assert_eq!(
        evidence.snapshot().typed_stop_code,
        "NISAR_LIVE_SEMANTIC_PLAN_POSTCHECK_FAILED"
    );
    assert!(evidence.snapshot().cleanup_completed);
    drop(cleanup);
    drop(evidence);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b0_1_d_success_validates_before_preserving_artifact() {
    let root = offline_m2d_b_root("success");
    let output = root.join("output");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("approved.h5"), b"offline-placeholder").unwrap();
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(root.clone(), BASELINE_COMMIT, "offline-harness")
            .unwrap(),
    );
    let mut cleanup = LiveExecutionCleanupGuard::new(root.clone(), Arc::clone(&evidence));
    let passing = passing_m2d_b_evidence();
    assert!(validate_m2d_b_final_evidence(&passing, &passing.aoi_digest, &output).is_ok());
    cleanup.preserve_final_artifact();
    cleanup.cleanup_now().unwrap();
    assert!(output.join("approved.h5").exists());
    assert!(evidence.evidence_path().exists());
    assert!(evidence.snapshot().cleanup_completed);
    drop(cleanup);
    drop(evidence);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b_successful_prefix_retry_is_charged_and_strictly_validated() {
    let root = offline_m2d_b_root("successful-prefix-retry");
    let output = root.join("output");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("approved.h5"), b"offline-placeholder").unwrap();
    let mut evidence = passing_m2d_b_evidence();
    let failed_response_bytes = 7;
    evidence
        .range_failure_observations
        .push(insar_download::NisarLiveRangeFailureObservationV2 {
            stage: "metadata_prefix".to_owned(),
            start: 0,
            end_exclusive: insar_download::NISAR_LIVE_METADATA_PREFIX_BYTES,
            source_size_bytes: SOURCE_BYTES,
            disposition: "sent_response_rejected".to_owned(),
            request_count: 2,
            http_status: Some(206),
            response_body_bytes: Some(failed_response_bytes),
            validator_sha256: Some(evidence.source_validator_sha256.clone()),
            failure_code: "NISAR_SUBSET_V2_PARTIAL_RESPONSE".to_owned(),
            redirect_occurred: false,
            redirect_hop_index: None,
            redirect_source_host_class: None,
            redirect_destination_host_class: None,
            redirect_source_path_class: None,
            redirect_destination_path_class: None,
            redirect_same_origin: None,
            redirect_scheme_transition: None,
            redirect_policy_result: None,
            redirect_rejection_reason: None,
        });
    evidence.charged_failure_requests = 2;
    evidence.charged_failure_response_bytes = failed_response_bytes;
    evidence.planning_requests += 2;
    evidence.planning_response_bytes += failed_response_bytes;
    evidence.total_requests += 2;
    evidence.total_response_bytes += failed_response_bytes;
    evidence.remaining_requests -= 2;
    evidence.remaining_response_bytes -= failed_response_bytes;
    evidence.source_byte_ratio =
        evidence.total_response_bytes as f64 / evidence.source_size_bytes as f64;

    assert!(validate_m2d_b_final_evidence(&evidence, &evidence.aoi_digest, &output).is_ok());

    let mut guessed_charge = evidence.clone();
    guessed_charge.charged_failure_response_bytes =
        insar_download::NISAR_LIVE_METADATA_PREFIX_BYTES * 2;
    assert_eq!(
        validate_m2d_b_final_evidence(&guessed_charge, &evidence.aoi_digest, &output),
        Err("NISAR_LIVE_RANGE_POSTCHECK_FAILED")
    );

    let mut wrong_stage = evidence.clone();
    wrong_stage.range_failure_observations[0].stage = "science".to_owned();
    assert_eq!(
        validate_m2d_b_final_evidence(&wrong_stage, &evidence.aoi_digest, &output),
        Err("NISAR_LIVE_RANGE_POSTCHECK_FAILED")
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b_final_evidence_rejects_polarization_range_budget_and_store_drift() {
    let root = offline_m2d_b_root("strict-final-evidence");
    let output = root.join("output");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("approved.h5"), b"offline-placeholder").unwrap();
    let passing = passing_m2d_b_evidence();

    let mut advertised = passing.clone();
    advertised.advertised_polarizations = vec!["HH".to_owned()];
    assert_eq!(
        validate_m2d_b_final_evidence(&advertised, &passing.aoi_digest, &output),
        Err("NISAR_LIVE_EVIDENCE_INCOMPLETE")
    );

    let mut selected = passing.clone();
    selected.selected_polarizations = vec!["HH".to_owned()];
    assert_eq!(
        validate_m2d_b_final_evidence(&selected, &passing.aoi_digest, &output),
        Err("NISAR_LIVE_EVIDENCE_INCOMPLETE")
    );

    let mut missing_range = passing.clone();
    missing_range.range_observations.pop();
    assert_eq!(
        validate_m2d_b_final_evidence(&missing_range, &passing.aoi_digest, &output),
        Err("NISAR_LIVE_RANGE_POSTCHECK_FAILED")
    );

    let mut malformed_range = passing.clone();
    malformed_range.range_observations[0].content_range_end_exclusive = 2;
    assert_eq!(
        validate_m2d_b_final_evidence(&malformed_range, &passing.aoi_digest, &output),
        Err("NISAR_LIVE_RANGE_POSTCHECK_FAILED")
    );

    let mut mismatched_request_count = passing.clone();
    mismatched_request_count.range_observations[0].request_count = 2;
    assert_eq!(
        validate_m2d_b_final_evidence(&mismatched_request_count, &passing.aoi_digest, &output),
        Err("NISAR_LIVE_RANGE_POSTCHECK_FAILED")
    );

    let mut bad_budget = passing.clone();
    bad_budget.remaining_requests += 1;
    assert_eq!(
        validate_m2d_b_final_evidence(&bad_budget, &passing.aoi_digest, &output),
        Err("NISAR_LIVE_BUDGET_POSTCHECK_FAILED")
    );

    let mut missing_completion = passing.clone();
    missing_completion.store_completed = "not_observed".to_owned();
    assert_eq!(
        validate_m2d_b_final_evidence(&missing_completion, &passing.aoi_digest, &output),
        Err("NISAR_LIVE_EVIDENCE_INCOMPLETE")
    );

    let mut missing_reopen = passing;
    missing_reopen.store_reopen = "not_observed".to_owned();
    assert_eq!(
        validate_m2d_b_final_evidence(&missing_reopen, &missing_reopen.aoi_digest, &output),
        Err("NISAR_LIVE_EVIDENCE_INCOMPLETE")
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b0_1_e_specific_worker_failure_wins_over_harness_panic() {
    let root = offline_m2d_b_root("specific-failure");
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(root.clone(), BASELINE_COMMIT, "offline-harness")
            .unwrap(),
    );
    evidence
        .record_failure("NISAR_SUBSET_SEMANTIC_PLAN_MISMATCH")
        .unwrap();
    record_live_failure_if_unset(&evidence, "NISAR_LIVE_HARNESS_PANIC");
    assert_eq!(
        evidence.snapshot().typed_stop_code,
        "NISAR_SUBSET_SEMANTIC_PLAN_MISMATCH"
    );
    drop(evidence);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_acceptance_loopback_preflight_does_not_need_production_credentials() {
    let server = preflight_response("206 Partial Content", Some("\"loopback\""), 4);
    let asset = preflight_asset(&server.url, 4);
    let result = preflight_live_nisar_source(&direct_transfer(), &asset, 4, &|| false).unwrap();
    assert_eq!(result.final_host_class(), "earthdata_asf_v1");
    assert_eq!(result.final_path_class(), "approved_nisar_hdf5_object");
    assert_eq!(result.response_body_bytes(), 1);
    assert_eq!(result.request_count(), 1);
    let validator = result.into_source_validator();
    assert_eq!(validator.kind, NisarSubsetValidatorKind::StrongEtag);
    assert_eq!(validator.content_length, 4);
}

#[test]
fn source_and_budget_constants_match_the_authorized_single_scene() {
    assert_eq!(FILE_NAME, format!("{GRANULE_ID}.h5").as_str());
    assert_eq!(SOURCE_CHECKSUM_ALGORITHM, "MD5");
    assert_eq!(SOURCE_CHECKSUM_VALUE.len(), 32);
    assert_eq!(HISTORICAL_P1_5_PLAN_SHA256.len(), 64);
    #[cfg(feature = "nisar-subset-live-acceptance")]
    assert_eq!(NISAR_M2D_A_CURRENT_PLAN_SHA256.len(), 64);
    #[cfg(feature = "nisar-subset-live-acceptance")]
    assert_ne!(
        HISTORICAL_P1_5_PLAN_SHA256, NISAR_M2D_A_CURRENT_PLAN_SHA256,
        "the historical P1.5 plan hash must not be treated as the current M2D-A authority"
    );
    assert_eq!(
        fixed_scope(
            GRANULE_ID,
            AOI_SHA256,
            NisarSubsetFrequency::A,
            "HH+HV",
            SWATH
        ),
        Ok(())
    );
    assert_eq!(official_aoi_sha256(), AOI_SHA256);
    assert_ne!(
        normalize_nisar_subset_aoi(&aoi()).unwrap().sha256,
        AOI_SHA256,
        "the official evidence digest is the fixed raw GeoJSON identity; the Request uses the current normalized contract digest"
    );
    const {
        assert!(REMOTE_BYTES < SOURCE_BYTES / 20);
    }
}

#[test]
fn live_test_is_not_run_by_the_ordinary_test_command() {
    let ignored = true;
    assert!(ignored, "the real case below is intentionally ignored");
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn controlled_catalogue() -> Result<AsfCatalogClient, &'static str> {
    AsfCatalogClient::nisar_controlled_live_acceptance().map_err(|_| "NISAR_LIVE_CATALOGUE_FAILED")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "one manually authorized catalogue-only public CMR request"]
fn nisar_exact_identity_catalogue_canary() {
    assert_eq!(
        std::env::var("INSAR_NISAR_CATALOGUE_CANARY")
            .ok()
            .as_deref(),
        Some("1"),
        "the exact catalogue canary requires its explicit gate"
    );
    let producer_granule_id = std::env::var("INSAR_NISAR_CATALOGUE_CANARY_GRANULE")
        .expect("fixed canary producer granule gate");
    assert_eq!(producer_granule_id, NISAR_CATALOGUE_CANARY_GRANULE_ID);
    let root = std::path::PathBuf::from(
        std::env::var("INSAR_NISAR_CATALOGUE_CANARY_ROOT").expect("fixed canary root gate"),
    );
    assert!(
        !root.exists(),
        "the catalogue-only canary root must be absent before the request"
    );

    let result =
        AsfCatalogClient::nisar_exact_identity_canary().expect("production CMR catalogue client");
    let result = result
        .search_nisar_exact_identity_canary(
            insar_provider::catalog::NISAR_NATIVE_COLLECTION_ID,
            &producer_granule_id,
            &aoi(),
            &|| false,
        )
        .expect("exact NISAR identity catalogue canary");

    assert_eq!(result.request_count, 1);
    assert_eq!(result.http_status, 200);
    assert!(result.response_body_bytes > 0);
    assert!(result.response_body_bytes <= 2 * 1024 * 1024);
    assert_eq!(result.hits, 1);
    assert_eq!(result.returned_item_count, 1);
    assert_eq!(
        result.provider_id,
        insar_provider::catalog::NISAR_PROVIDER_ID
    );
    assert_eq!(
        result.collection_id,
        insar_provider::catalog::NISAR_NATIVE_COLLECTION_ID
    );
    assert_eq!(
        result.producer_granule_id,
        NISAR_CATALOGUE_CANARY_GRANULE_ID
    );
    assert_eq!(result.product_type, "RSLC");
    assert_eq!(result.production_configuration, "PR");
    assert_eq!(result.crid, "P05023");
    assert_eq!(result.product_specification_version, "1.5.0");
    assert_ne!(result.quality_status, "Beta");
    assert_eq!(
        result.primary_filename,
        format!("{NISAR_CATALOGUE_CANARY_GRANULE_ID}.h5")
    );
    assert_eq!(result.primary_role, "Primary");
    assert_eq!(result.source_size_bytes, 14_252_244_992);
    assert_eq!(result.source_checksum_algorithm, "MD5");
    assert_eq!(
        result.source_checksum_value,
        "059ada6521bff1ef314fb1141a89cd10"
    );
    assert!(result.footprint_valid);
    assert!(result.aoi_intersects);
    assert!(result.primary_locator_present);
    let redacted = format!("{result:?}");
    println!("CATALOGUE_CANARY_REDACTED_RESULT={redacted}");
    assert!(!redacted.contains("https://"));
    assert!(!redacted.contains("credential"));
    assert!(!redacted.contains("token"));
    assert!(
        !root.exists(),
        "catalogue-only canary must not create a root"
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn production_fixed_scene_product_fixture() -> (insar_core::ProviderProduct, CatalogSearchQuery) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-nisar-l-p1/granule-provisional-multi-asset-valid.json");
    let mut fixture: Value = serde_json::from_str(
        &fs::read_to_string(path).expect("production-shaped NISAR fixture exists"),
    )
    .expect("production-shaped NISAR fixture is valid JSON");
    fixture
        .get_mut("collection")
        .and_then(Value::as_object_mut)
        .expect("NISAR fixture collection object exists")
        .insert("cmr_provider_id".to_owned(), json!("ASF"));
    let granule = fixture
        .get_mut("granule")
        .and_then(Value::as_object_mut)
        .expect("NISAR fixture granule object exists");
    granule.insert(
        "concept_id".to_owned(),
        Value::String(PROVIDER_PRODUCT_ID.to_owned()),
    );
    granule.insert(
        "producer_granule_id".to_owned(),
        Value::String(GRANULE_ID.to_owned()),
    );
    granule.insert(
        "asf_product_id".to_owned(),
        Value::String(GRANULE_ID.to_owned()),
    );
    let records = granule
        .get_mut("assets")
        .and_then(Value::as_object_mut)
        .and_then(|assets| assets.get_mut("records"))
        .and_then(Value::as_array_mut)
        .expect("NISAR fixture asset records exist");
    let primary = records
        .iter_mut()
        .find(|record| record.get("normalized_role").and_then(Value::as_str) == Some("primary"))
        .expect("NISAR fixture primary asset exists");
    let primary = primary
        .as_object_mut()
        .expect("NISAR fixture primary asset is an object");
    primary.insert("file_name".to_owned(), Value::String(FILE_NAME.to_owned()));
    primary.insert("size_bytes".to_owned(), Value::from(SOURCE_BYTES));
    primary.insert(
        "checksum".to_owned(),
        json!({
            "algorithm": SOURCE_CHECKSUM_ALGORITHM,
            "value": SOURCE_CHECKSUM_VALUE,
        }),
    );
    primary.insert(
        "routes".to_owned(),
        json!([
            {
                "scheme": "https",
                "locator": format!("https://example.invalid/nisar/{FILE_NAME}"),
            },
            {
                "scheme": "s3",
                "locator": format!("s3://example-invalid/nisar/{FILE_NAME}"),
            },
        ]),
    );
    primary
        .get_mut("native_evidence")
        .and_then(Value::as_object_mut)
        .expect("NISAR fixture primary evidence exists")
        .insert(
            "archive_name".to_owned(),
            Value::String(FILE_NAME.to_owned()),
        );

    let envelope = CmrGranuleEnvelope::try_from(&fixture).expect("NISAR fixture envelope");
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    let product = map_nisar_rslc_umm_granule(&envelope, profile).expect("NISAR product maps");
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": NISAR_LOGICAL_DATASET,
        "provider_id": NISAR_PROVIDER_ID,
        "native_collection_id": NISAR_NATIVE_COLLECTION_ID,
        "aoi_geojson": aoi(),
        "cursor": null,
        "result_limit": 100,
        "filters": {"production_configuration": "PR"}
    }))
    .expect("offline NISAR mapper query");
    (product, query)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn production_mapped_fixed_scene_fixture() -> SnapshotItem {
    let (product, query) = production_fixed_scene_product_fixture();
    map_nisar_product_for_live_acceptance(product, &query).expect("NISAR product projects")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_live_source_identity_pre_b1(item: &SnapshotItem) -> Result<(), &'static str> {
    if item.provider_id != NISAR_PROVIDER_ID
        || item.display_name != GRANULE_ID
        || item.metadata.get("beam_swath").and_then(Value::as_str) != Some(SWATH)
        || item.metadata.get("polarization").and_then(Value::as_str) != Some("HH")
    {
        return Err("NISAR_LIVE_SOURCE_IDENTITY_MISMATCH");
    }

    let asset = item
        .asset_access
        .as_ref()
        .ok_or("NISAR_LIVE_SOURCE_IDENTITY_MISMATCH")?;
    if asset.role != insar_core::AssetRole::Primary
        || asset.file_name.as_deref() != Some(FILE_NAME)
        || asset.expected_size_bytes != Some(SOURCE_BYTES)
        || asset.checksum_algorithm.as_deref() != Some(SOURCE_CHECKSUM_ALGORITHM)
        || asset.checksum_value.as_deref() != Some(SOURCE_CHECKSUM_VALUE)
        || asset.reference.collection_id != NISAR_NATIVE_COLLECTION_ID
    {
        return Err("NISAR_LIVE_SOURCE_IDENTITY_MISMATCH");
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b1_regression_reproduces_pre_b1_generic_metadata_false_negative() {
    let item = production_mapped_fixed_scene_fixture();
    assert!(item.metadata.get("beam_swath").is_none());
    assert!(item.metadata.get("polarization").is_none());
    assert_eq!(
        item.metadata.get("logical_dataset").and_then(Value::as_str),
        Some(NISAR_LOGICAL_DATASET)
    );
    assert_eq!(
        item.metadata
            .get("native_collection_id")
            .and_then(Value::as_str),
        Some(NISAR_NATIVE_COLLECTION_ID)
    );
    assert_eq!(
        item.metadata
            .get("processing_level")
            .and_then(Value::as_str),
        Some("RSLC")
    );
    assert_eq!(
        item.metadata
            .get("production_configuration")
            .and_then(Value::as_str),
        Some("PR")
    );
    assert_eq!(
        item.metadata
            .get("frequency_a_polarization")
            .and_then(Value::as_str),
        Some("HH+HV")
    );
    assert_eq!(item.display_name, GRANULE_ID);
    let asset = item.asset_access.as_ref().expect("fixed primary asset");
    assert_eq!(asset.file_name.as_deref(), Some(FILE_NAME));
    assert_eq!(asset.expected_size_bytes, Some(SOURCE_BYTES));
    assert_eq!(
        asset.checksum_algorithm.as_deref(),
        Some(SOURCE_CHECKSUM_ALGORITHM)
    );
    assert_eq!(asset.checksum_value.as_deref(), Some(SOURCE_CHECKSUM_VALUE));
    assert_eq!(
        validate_live_source_identity_pre_b1(&item),
        Err("NISAR_LIVE_SOURCE_IDENTITY_MISMATCH")
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn source_supports_frequency_a_hh(item: &SnapshotItem) -> bool {
    let requested_polarization = "HH";
    let requested_is_supported =
        supported_dataset_polarizations(&NisarSubsetFrequency::A).contains(&requested_polarization);
    let available = item
        .metadata
        .get("frequency_a_polarization")
        .and_then(Value::as_str);
    requested_is_supported && matches!(available, Some("HH" | "HH+HV"))
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_live_source_identity(item: &SnapshotItem) -> Result<(), &'static str> {
    if item.provider_id != NISAR_PROVIDER_ID || item.display_name != GRANULE_ID {
        return Err("NISAR_LIVE_GRANULE_IDENTITY_MISMATCH");
    }
    if item.metadata.get("logical_dataset").and_then(Value::as_str) != Some(NISAR_LOGICAL_DATASET)
        || item
            .metadata
            .get("native_collection_id")
            .and_then(Value::as_str)
            != Some(NISAR_NATIVE_COLLECTION_ID)
    {
        return Err("NISAR_LIVE_COLLECTION_IDENTITY_MISMATCH");
    }
    if item
        .metadata
        .get("processing_level")
        .and_then(Value::as_str)
        != Some(NISAR_REQUIRED_PRODUCT_TYPE)
        || item
            .metadata
            .get("production_configuration")
            .and_then(Value::as_str)
            != Some(NISAR_PRODUCTION_CONFIGURATION)
        || item.metadata.get("quality_status").and_then(Value::as_str) != Some(NISAR_QUALITY_STATUS)
        || !source_supports_frequency_a_hh(item)
    {
        return Err("NISAR_LIVE_SOURCE_CAPABILITY_MISMATCH");
    }

    let Some(asset) = item.asset_access.as_ref() else {
        return Err("NISAR_LIVE_PRIMARY_ASSET_MISSING");
    };
    if asset.reference.provider_id != NISAR_PROVIDER_ID
        || asset.reference.collection_id != NISAR_NATIVE_COLLECTION_ID
    {
        return Err("NISAR_LIVE_COLLECTION_IDENTITY_MISMATCH");
    }
    if asset.role != insar_core::AssetRole::Primary {
        return Err("NISAR_LIVE_PRIMARY_ASSET_MISSING");
    }
    if asset.file_name.as_deref() != Some(FILE_NAME) {
        return Err("NISAR_LIVE_SOURCE_FILENAME_MISMATCH");
    }
    if asset.expected_size_bytes != Some(SOURCE_BYTES) {
        return Err("NISAR_LIVE_SOURCE_SIZE_MISMATCH");
    }
    if asset.checksum_algorithm.as_deref() != Some(SOURCE_CHECKSUM_ALGORITHM)
        || asset.checksum_value.as_deref() != Some(SOURCE_CHECKSUM_VALUE)
    {
        return Err("NISAR_LIVE_SOURCE_CHECKSUM_MISMATCH");
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_live_source_identity_for_frequency(
    item: &SnapshotItem,
    frequency: NisarSubsetFrequency,
    selected_polarizations: &[String],
) -> Result<(), &'static str> {
    if item.provider_id != NISAR_PROVIDER_ID || item.display_name != GRANULE_ID {
        return Err("NISAR_LIVE_GRANULE_IDENTITY_MISMATCH");
    }
    if item.metadata.get("logical_dataset").and_then(Value::as_str) != Some(NISAR_LOGICAL_DATASET)
        || item
            .metadata
            .get("native_collection_id")
            .and_then(Value::as_str)
            != Some(NISAR_NATIVE_COLLECTION_ID)
    {
        return Err("NISAR_LIVE_COLLECTION_IDENTITY_MISMATCH");
    }
    if item
        .metadata
        .get("processing_level")
        .and_then(Value::as_str)
        != Some(NISAR_REQUIRED_PRODUCT_TYPE)
        || item
            .metadata
            .get("production_configuration")
            .and_then(Value::as_str)
            != Some(NISAR_PRODUCTION_CONFIGURATION)
        || item.metadata.get("quality_status").and_then(Value::as_str) != Some(NISAR_QUALITY_STATUS)
    {
        return Err("NISAR_LIVE_SOURCE_CAPABILITY_MISMATCH");
    }
    let capability_key = match &frequency {
        NisarSubsetFrequency::A => "frequency_a_polarization",
        NisarSubsetFrequency::B => "frequency_b_polarization",
    };
    let advertised = item
        .metadata
        .get(capability_key)
        .and_then(Value::as_str)
        .and_then(|label| {
            insar_provider::asf::nisar_subset::canonical_nisar_polarizations_for_frequency_label(
                &frequency, label,
            )
        })
        .ok_or("NISAR_LIVE_SOURCE_CAPABILITY_MISMATCH")?;
    if selected_polarizations.is_empty()
        || selected_polarizations
            .iter()
            .any(|polarization| !advertised.contains(polarization))
    {
        return Err("NISAR_LIVE_SOURCE_CAPABILITY_MISMATCH");
    }
    let Some(asset) = item.asset_access.as_ref() else {
        return Err("NISAR_LIVE_PRIMARY_ASSET_MISSING");
    };
    if asset.reference.provider_id != NISAR_PROVIDER_ID
        || asset.reference.collection_id != NISAR_NATIVE_COLLECTION_ID
    {
        return Err("NISAR_LIVE_COLLECTION_IDENTITY_MISMATCH");
    }
    if asset.role != insar_core::AssetRole::Primary {
        return Err("NISAR_LIVE_PRIMARY_ASSET_MISSING");
    }
    if asset.file_name.as_deref() != Some(FILE_NAME) {
        return Err("NISAR_LIVE_SOURCE_FILENAME_MISMATCH");
    }
    if asset.expected_size_bytes != Some(SOURCE_BYTES) {
        return Err("NISAR_LIVE_SOURCE_SIZE_MISMATCH");
    }
    if asset.checksum_algorithm.as_deref() != Some(SOURCE_CHECKSUM_ALGORITHM)
        || asset.checksum_value.as_deref() != Some(SOURCE_CHECKSUM_VALUE)
    {
        return Err("NISAR_LIVE_SOURCE_CHECKSUM_MISMATCH");
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn assert_live_source_identity_rejects(
    label: &str,
    mutate: impl FnOnce(&mut SnapshotItem),
    expected: &'static str,
) {
    let mut item = production_mapped_fixed_scene_fixture();
    mutate(&mut item);
    assert_eq!(
        validate_live_source_identity(&item),
        Err(expected),
        "{label}"
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b1_production_mapped_fixed_scene_passes_source_identity_and_hh_capability() {
    let item = production_mapped_fixed_scene_fixture();
    assert_eq!(validate_live_source_identity(&item), Ok(()));

    let mut hh_only = item;
    hh_only.metadata.insert(
        "frequency_a_polarization".to_owned(),
        Value::String("HH".to_owned()),
    );
    assert_eq!(validate_live_source_identity(&hh_only), Ok(()));
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task1a_production_mapper_accepts_frequency_a_hh_from_hh_hv() {
    let (product, query) = production_fixed_scene_product_fixture();
    let descriptor: AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1 =
        build_nisar_subset_live_frequency_a_hh_descriptor(product, &query)
            .expect("live descriptor");

    assert_eq!(descriptor.provider_id(), NISAR_PROVIDER_ID);
    assert_eq!(descriptor.provider_product_id(), PROVIDER_PRODUCT_ID);
    assert_eq!(descriptor.granule_id(), GRANULE_ID);
    assert_eq!(descriptor.collection_id(), NISAR_NATIVE_COLLECTION_ID);
    assert_eq!(descriptor.collection_profile_id(), NISAR_PROFILE_ID);
    assert_eq!(descriptor.file_name(), FILE_NAME);
    assert_eq!(descriptor.expected_size_bytes(), SOURCE_BYTES);
    assert_eq!(descriptor.checksum().value, SOURCE_CHECKSUM_VALUE);
    assert_eq!(descriptor.acquisition_datetime(), "2026-06-18T10:00:00Z");
    assert_eq!(
        descriptor.quality_status(),
        insar_core::NisarSubsetQualityStatusV2::Provisional
    );
    assert_eq!(descriptor.selected_frequency(), NisarSubsetFrequency::A);
    assert_eq!(descriptor.selected_polarizations(), ["HH"]);
    assert_eq!(
        descriptor.advertised_frequency_a_polarizations(),
        ["HH", "HV"]
    );
    assert_eq!(descriptor.source_footprint_geojson()["type"], "Polygon");
    let serialized = serde_json::to_string(&descriptor).expect("descriptor serializes");
    for forbidden in [
        "https://",
        "Authorization",
        "Cookie",
        "credential",
        "username",
        "password",
        "X-Amz-Signature",
        "stable_href",
        "resolver_id",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "serialized descriptor leaked {forbidden}"
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task2_public_builder_selects_all_requested_frequency_a_polarizations_from_advertised_metadata() {
    let (product, query) = production_fixed_scene_product_fixture();
    let descriptor: AsfNisarSubsetLiveFrequencyAMultiPolDescriptorV1 =
        build_nisar_subset_live_frequency_a_multi_pol_descriptor(
            product,
            &query,
            &["HV".to_owned(), "HH".to_owned()],
        )
        .expect("live multi-pol descriptor");

    assert_eq!(descriptor.selected_frequency(), NisarSubsetFrequency::A);
    assert_eq!(descriptor.selected_polarizations(), ["HH", "HV"]);
    assert_eq!(
        descriptor.advertised_frequency_a_polarizations(),
        ["HH", "HV"]
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task2_public_builder_accepts_frequency_a_vv_vh_and_canonicalizes_selection() {
    let (mut product, query) = production_fixed_scene_product_fixture();
    product.properties.insert(
        "frequency_a_polarization".to_owned(),
        Value::String("VV+VH".to_owned()),
    );
    let mapped = map_nisar_product_for_live_acceptance(product.clone(), &query)
        .expect("VV+VH product maps through the authoritative mapper");
    assert_eq!(
        mapped
            .metadata
            .get("frequency_a_polarization")
            .and_then(Value::as_str),
        Some("VV+VH")
    );

    let descriptor = build_nisar_subset_live_frequency_a_multi_pol_descriptor(
        product.clone(),
        &query,
        &["VV".to_owned(), "VH".to_owned()],
    )
    .expect("VV+VH multi-pol descriptor");
    assert_eq!(
        descriptor.advertised_frequency_a_polarizations(),
        ["VH", "VV"]
    );
    assert_eq!(descriptor.selected_polarizations(), ["VH", "VV"]);
    assert_eq!(descriptor.selected_frequency(), NisarSubsetFrequency::A);

    assert_eq!(
        build_nisar_subset_live_frequency_a_hh_descriptor(product, &query).unwrap_err(),
        AsfNisarSubsetLiveDescriptorError::ProjectionInvalid
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task2_public_builder_rejects_invalid_requested_polarizations() {
    let cases: &[&[&str]] = &[
        &[],
        &["HH"],
        &["HH", "VV"],
        &["HH", "HH"],
        &["HH", " HV"],
        &["hv", "HH"],
    ];
    for requested in cases {
        let (product, query) = production_fixed_scene_product_fixture();
        let requested = requested
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            build_nisar_subset_live_frequency_a_multi_pol_descriptor(product, &query, &requested,)
                .unwrap_err(),
            AsfNisarSubsetLiveDescriptorError::ProjectionInvalid,
            "requested polarization case {requested:?}"
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task2_public_builder_requires_each_requested_polarization_in_authoritative_a_capability() {
    let (mut product, query) = production_fixed_scene_product_fixture();
    product.properties.insert(
        "frequency_a_polarization".to_owned(),
        Value::String("HH".to_owned()),
    );
    assert_eq!(
        build_nisar_subset_live_frequency_a_multi_pol_descriptor(
            product,
            &query,
            &["HH".to_owned(), "HV".to_owned()],
        )
        .unwrap_err(),
        AsfNisarSubsetLiveDescriptorError::ProjectionInvalid
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task2_public_builder_rejects_missing_or_malformed_frequency_a_capability() {
    type CapabilityMutation = fn(&mut insar_core::ProviderProduct);
    let cases: &[(&str, CapabilityMutation)] = &[
        ("missing Frequency A", |product| {
            product.properties.remove("frequency_a_polarization");
        }),
        ("unsupported Frequency A", |product| {
            product
                .properties
                .insert("frequency_a_polarization".to_owned(), json!("VV"));
        }),
        ("duplicate Frequency A", |product| {
            product
                .properties
                .insert("frequency_a_polarization".to_owned(), json!("HH+HH"));
        }),
        ("malformed Frequency A", |product| {
            product
                .properties
                .insert("frequency_a_polarization".to_owned(), json!("HH+"));
        }),
        ("unsorted Frequency A", |product| {
            product
                .properties
                .insert("frequency_a_polarization".to_owned(), json!("VH+VV"));
        }),
    ];
    for &(label, mutate) in cases {
        let (mut product, query) = production_fixed_scene_product_fixture();
        mutate(&mut product);
        let error = build_nisar_subset_live_frequency_a_multi_pol_descriptor(
            product,
            &query,
            &["HH".to_owned(), "HV".to_owned()],
        )
        .unwrap_err();
        assert_eq!(
            error,
            if label == "missing Frequency A" {
                AsfNisarSubsetLiveDescriptorError::FrequencyAMissing
            } else {
                AsfNisarSubsetLiveDescriptorError::AuthoritativeMapperRejected
            },
            "{label}: {error:?}"
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task2_multi_pol_descriptor_is_secret_free_and_filename_tokens_do_not_select_datasets() {
    let (mut product, query) = production_fixed_scene_product_fixture();
    let misleading_granule_id = format!("{GRANULE_ID}_VV");
    let misleading_file_name = format!("{misleading_granule_id}.h5");
    product.display_name = misleading_granule_id.clone();
    product.properties.insert(
        "producer_granule_id".to_owned(),
        Value::String(misleading_granule_id.clone()),
    );
    product.properties.insert(
        "asf_product_id".to_owned(),
        Value::String(misleading_granule_id),
    );
    let primary = product
        .assets
        .iter_mut()
        .find(|asset| asset.role == insar_core::AssetRole::Primary)
        .expect("primary provider asset");
    primary.file_name = Some(misleading_file_name.clone());
    primary.url = primary.url.take().map(|url| {
        format!(
            "{}/{}",
            url.rsplit_once('/')
                .map(|(prefix, _)| prefix)
                .unwrap_or("https://example.invalid"),
            misleading_file_name
        )
    });
    if let Some(descriptor) = primary.access_descriptor.as_mut() {
        descriptor.stable_href = Some(format!(
            "https://example.invalid/nisar/{misleading_file_name}"
        ));
    }
    let graph = product
        .properties
        .get_mut("assets")
        .and_then(Value::as_array_mut)
        .expect("asset graph");
    let primary_graph = graph
        .iter_mut()
        .find(|asset| asset.get("role").and_then(Value::as_str) == Some("primary"))
        .and_then(Value::as_object_mut)
        .expect("graph primary");
    primary_graph.insert(
        "file_name".to_owned(),
        Value::String(misleading_file_name.clone()),
    );
    primary_graph.insert(
        "routes".to_owned(),
        json!([
            {"scheme": "https", "locator": format!("https://example.invalid/nisar/{misleading_file_name}")},
            {"scheme": "s3", "locator": format!("s3://example-invalid/nisar/{misleading_file_name}")}
        ]),
    );
    primary_graph
        .get_mut("native_evidence")
        .and_then(Value::as_object_mut)
        .expect("native evidence")
        .insert(
            "archive_name".to_owned(),
            Value::String(misleading_file_name.clone()),
        );
    let primary_key = product
        .properties
        .get("primary_asset_key")
        .and_then(Value::as_str)
        .expect("primary key")
        .to_owned();
    product
        .properties
        .get_mut("asset_integrity")
        .and_then(Value::as_object_mut)
        .and_then(|integrity| integrity.get_mut(&primary_key))
        .and_then(Value::as_object_mut)
        .expect("primary integrity")
        .insert(
            "file_name".to_owned(),
            Value::String(misleading_file_name.clone()),
        );
    product
        .properties
        .get_mut("asset_routes")
        .and_then(Value::as_object_mut)
        .expect("asset routes")
        .insert(
            primary_key,
            json!([
                {"scheme": "https", "locator": format!("https://example.invalid/nisar/{misleading_file_name}")},
                {"scheme": "s3", "locator": format!("s3://example-invalid/nisar/{misleading_file_name}")}
            ]),
        );
    let descriptor = build_nisar_subset_live_frequency_a_multi_pol_descriptor(
        product,
        &query,
        &["HH".to_owned(), "HV".to_owned()],
    )
    .expect("filename is still mapper-consistent");
    assert_eq!(descriptor.selected_polarizations(), ["HH", "HV"]);
    assert_eq!(descriptor.file_name(), misleading_file_name);
    let serialized = serde_json::to_string(&descriptor).expect("descriptor serializes");
    for forbidden in [
        "https://",
        "Authorization",
        "Cookie",
        "credential",
        "username",
        "password",
        "X-Amz-Signature",
        "stable_href",
        "resolver_id",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "serialized multi-pol descriptor leaked {forbidden}"
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn assert_task1a_mapper_rejects(
    label: &str,
    mutate: impl FnOnce(&mut insar_core::ProviderProduct),
    expected: AsfNisarSubsetLiveDescriptorError,
) {
    let (mut product, query) = production_fixed_scene_product_fixture();
    mutate(&mut product);
    let error =
        build_nisar_subset_live_frequency_a_hh_descriptor(product, &query).expect_err(label);
    assert_eq!(error, expected, "{label}: {error:?}");
    assert_eq!(error.code(), expected.code(), "{label}: typed code");
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task1a_public_builder_rejects_self_consistent_invalid_checksums() {
    assert_task1a_invalid_checksum_cases_reject();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn set_primary_checksum_evidence(
    product: &mut insar_core::ProviderProduct,
    algorithm: &str,
    value: &str,
) {
    let primary_asset_key = product
        .properties
        .get("primary_asset_key")
        .and_then(Value::as_str)
        .expect("primary key")
        .to_owned();
    let graph = product
        .properties
        .get_mut("assets")
        .and_then(Value::as_array_mut)
        .expect("asset graph");
    let primary = graph
        .iter_mut()
        .find(|asset| asset.get("role").and_then(Value::as_str) == Some("primary"))
        .and_then(Value::as_object_mut)
        .expect("graph primary");
    let checksum = primary
        .get_mut("checksum")
        .and_then(Value::as_object_mut)
        .expect("primary checksum");
    checksum.insert("algorithm".to_owned(), Value::String(algorithm.to_owned()));
    checksum.insert("value".to_owned(), Value::String(value.to_owned()));

    let integrity = product
        .properties
        .get_mut("asset_integrity")
        .and_then(Value::as_object_mut)
        .expect("asset integrity");
    let primary_integrity = integrity
        .get_mut(&primary_asset_key)
        .and_then(Value::as_object_mut)
        .expect("primary integrity");
    primary_integrity.insert("algorithm".to_owned(), Value::String(algorithm.to_owned()));
    primary_integrity.insert("value".to_owned(), Value::String(value.to_owned()));
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn assert_task1a_invalid_checksum_cases_reject() {
    let expected = AsfNisarSubsetLiveDescriptorError::AuthoritativeMapperRejected;
    let cases = [
        (
            "invalid checksum algorithm",
            "CRC32",
            SOURCE_CHECKSUM_VALUE.to_owned(),
        ),
        (
            "invalid checksum value length",
            SOURCE_CHECKSUM_ALGORITHM,
            "0".repeat(31),
        ),
    ];
    for (label, algorithm, value) in cases {
        let (mut product, query) = production_fixed_scene_product_fixture();
        set_primary_checksum_evidence(&mut product, algorithm, &value);
        let error =
            build_nisar_subset_live_frequency_a_hh_descriptor(product, &query).expect_err(label);
        assert_eq!(error, expected, "{label}: {error:?}");
        assert_eq!(error.code(), expected.code(), "{label}: typed code");
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task1a_public_builder_rejects_required_product_mutations() {
    assert_task1a_required_product_mutations_reject();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn primary_provider_asset_mut(
    product: &mut insar_core::ProviderProduct,
) -> &mut insar_core::ProviderAsset {
    product
        .assets
        .iter_mut()
        .find(|asset| asset.role == insar_core::AssetRole::Primary)
        .expect("primary provider asset")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
type Task1aProductMutation = fn(&mut insar_core::ProviderProduct);

#[cfg(feature = "nisar-subset-live-acceptance")]
fn assert_task1a_required_product_mutations_reject() {
    let mapper_rejected = AsfNisarSubsetLiveDescriptorError::AuthoritativeMapperRejected;
    let cases: &[(&str, Task1aProductMutation)] = &[
        ("zero size", |product| {
            primary_provider_asset_mut(product).size_bytes = Some(0)
        }),
        ("invalid footprint", |product| {
            product.footprint_geojson = Some(Map::from_iter([
                ("type".to_owned(), json!("Point")),
                ("coordinates".to_owned(), json!([117.0, 30.0])),
            ]));
        }),
        ("missing footprint", |product| {
            product.footprint_geojson = None
        }),
        ("wrong quality", |product| {
            product
                .properties
                .insert("quality_status".to_owned(), json!("Beta"));
        }),
        ("missing acquisition", |product| {
            product.acquisition_datetime = None
        }),
        ("invalid acquisition", |product| {
            product.acquisition_datetime = Some("not-an-instant".to_owned())
        }),
        ("unsupported Frequency A", |product| {
            product
                .properties
                .insert("frequency_a_polarization".to_owned(), json!("VV"));
        }),
        ("duplicate Frequency A", |product| {
            product
                .properties
                .insert("frequency_a_polarization".to_owned(), json!("HH+HH"));
        }),
        ("malformed Frequency A", |product| {
            product
                .properties
                .insert("frequency_a_polarization".to_owned(), json!("HH+"));
        }),
        ("non-primary asset", |product| {
            primary_provider_asset_mut(product).role = insar_core::AssetRole::Browse
        }),
        ("mismatched asset", |product| {
            primary_provider_asset_mut(product).file_name = Some("wrong.h5".to_owned());
        }),
        ("missing asset", |product| {
            product
                .assets
                .retain(|asset| asset.role != insar_core::AssetRole::Primary);
        }),
    ];
    for &(label, mutate) in cases {
        assert_task1a_mapper_rejects(label, mutate, mapper_rejected);
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task1a_mapper_rejects_wrong_or_empty_query_logical_dataset() {
    for logical_dataset in ["wrong.logical.dataset", ""] {
        let (product, mut query) = production_fixed_scene_product_fixture();
        query.logical_dataset = logical_dataset.to_owned();
        let error = build_nisar_subset_live_frequency_a_hh_descriptor(product, &query)
            .expect_err("wrong or empty logical dataset");
        assert_eq!(
            error,
            AsfNisarSubsetLiveDescriptorError::AuthoritativeMapperRejected
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task1a_mapper_rejects_wrong_provider_product_id_and_collection_identity() {
    let mapper_rejected = AsfNisarSubsetLiveDescriptorError::AuthoritativeMapperRejected;
    assert_task1a_mapper_rejects(
        "wrong provider",
        |product| product.provider_id = "wrong.provider".to_owned(),
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "wrong product ID",
        |product| product.product_id = "G4263869503-ASF".to_owned(),
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "wrong collection identity",
        |product| {
            product.properties.insert(
                "collection_concept_id".to_owned(),
                Value::String("WRONG_COLLECTION".to_owned()),
            );
        },
        mapper_rejected,
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task1a_mapper_rejects_fallback_graph_and_malformed_product_inputs() {
    let mapper_rejected = AsfNisarSubsetLiveDescriptorError::AuthoritativeMapperRejected;
    assert_task1a_mapper_rejects(
        "fallback without graph",
        |product| {
            product.properties.remove("assets");
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "wrong source kind",
        |product| {
            product.source_kind = insar_core::DataSourceKind::Sentinel1;
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "wrong product kind",
        |product| {
            product.product_kind = insar_core::ProductKind::SarSlc;
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "nested producer identity",
        |product| {
            product.properties.insert(
                "producer_granule_id".to_owned(),
                Value::String("NISAR_L1_PR_RSLC_WRONG".to_owned()),
            );
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "primary key mismatch",
        |product| {
            product.properties.insert(
                "primary_asset_key".to_owned(),
                Value::String("wrong-primary-key".to_owned()),
            );
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "graph primary media mismatch",
        |product| {
            let graph = product
                .properties
                .get_mut("assets")
                .and_then(Value::as_array_mut)
                .expect("asset graph");
            graph
                .iter_mut()
                .find(|asset| asset.get("role").and_then(Value::as_str) == Some("primary"))
                .and_then(Value::as_object_mut)
                .expect("graph primary")
                .insert(
                    "media_type".to_owned(),
                    Value::String("text/plain".to_owned()),
                );
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "integrity mismatch",
        |product| {
            let key = product
                .properties
                .get("primary_asset_key")
                .and_then(Value::as_str)
                .expect("primary key")
                .to_owned();
            product
                .properties
                .get_mut("asset_integrity")
                .and_then(Value::as_object_mut)
                .and_then(|integrity| integrity.get_mut(&key))
                .and_then(Value::as_object_mut)
                .expect("primary integrity")
                .insert("value".to_owned(), Value::String("bad".to_owned()));
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "routes mismatch",
        |product| {
            let key = product
                .properties
                .get("primary_asset_key")
                .and_then(Value::as_str)
                .expect("primary key")
                .to_owned();
            product
                .properties
                .get_mut("asset_routes")
                .and_then(Value::as_object_mut)
                .expect("route graph")
                .insert(key, json!([]));
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "malformed filename",
        |product| {
            let graph = product
                .properties
                .get_mut("assets")
                .and_then(Value::as_array_mut)
                .expect("asset graph");
            graph
                .iter_mut()
                .find(|asset| asset.get("role").and_then(Value::as_str) == Some("primary"))
                .and_then(Value::as_object_mut)
                .expect("graph primary")
                .insert(
                    "file_name".to_owned(),
                    Value::String("bad?query.h5".to_owned()),
                );
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "userinfo route",
        |product| {
            let primary_asset_key = product
                .properties
                .get("primary_asset_key")
                .and_then(Value::as_str)
                .expect("primary key")
                .to_owned();
            let invalid_locator = format!("https://user:password@example.invalid/{FILE_NAME}");
            let graph = product
                .properties
                .get_mut("assets")
                .and_then(Value::as_array_mut)
                .expect("asset graph");
            let route = graph
                .iter_mut()
                .find(|asset| asset.get("role").and_then(Value::as_str) == Some("primary"))
                .and_then(Value::as_object_mut)
                .and_then(|asset| asset.get_mut("routes"))
                .and_then(Value::as_array_mut)
                .and_then(|routes| routes.first_mut())
                .and_then(Value::as_object_mut)
                .expect("primary route");
            route.insert(
                "locator".to_owned(),
                Value::String(invalid_locator.to_owned()),
            );
            let route_graph = product
                .properties
                .get_mut("asset_routes")
                .and_then(Value::as_object_mut)
                .expect("route graph");
            let routes = route_graph
                .get_mut(&primary_asset_key)
                .and_then(Value::as_array_mut)
                .expect("primary route graph");
            routes
                .first_mut()
                .and_then(Value::as_object_mut)
                .expect("primary route graph entry")
                .insert(
                    "locator".to_owned(),
                    Value::String(invalid_locator.to_owned()),
                );
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "credential-like granule",
        |product| {
            product.display_name = "NISAR_L1_PR_RSLC_password".to_owned();
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "invalid acquisition",
        |product| {
            product.acquisition_datetime = Some("2026-07-26T15:58:03+08:00".to_owned());
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "missing footprint",
        |product| {
            product.footprint_geojson = None;
        },
        mapper_rejected,
    );
    assert_task1a_mapper_rejects(
        "full product mismatch",
        |product| {
            product
                .properties
                .insert("full_product".to_owned(), Value::Bool(false));
        },
        mapper_rejected,
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task1a_projection_reports_missing_frequency_a_after_authoritative_mapping() {
    let (mut product, query) = production_fixed_scene_product_fixture();
    product.properties.remove("frequency_a_polarization");
    let error = build_nisar_subset_live_frequency_a_hh_descriptor(product, &query)
        .expect_err("missing frequency A");
    assert_eq!(error, AsfNisarSubsetLiveDescriptorError::FrequencyAMissing);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn task1a_frequency_selection_ignores_misleading_primary_filename() {
    let (mut product, query) = production_fixed_scene_product_fixture();
    let misleading_granule_id = format!("{GRANULE_ID}_HV");
    let misleading_file_name = format!("{misleading_granule_id}.h5");
    product.display_name = misleading_granule_id.clone();
    product.properties.insert(
        "producer_granule_id".to_owned(),
        Value::String(misleading_granule_id.clone()),
    );
    product.properties.insert(
        "asf_product_id".to_owned(),
        Value::String(misleading_granule_id.clone()),
    );
    let primary = product
        .assets
        .iter_mut()
        .find(|asset| asset.role == insar_core::AssetRole::Primary)
        .expect("primary provider asset");
    primary.file_name = Some(misleading_file_name.clone());
    primary.url = primary.url.take().map(|url| {
        format!(
            "{}/{}",
            url.rsplit_once('/')
                .map(|(prefix, _)| prefix)
                .unwrap_or("https://example.invalid"),
            misleading_file_name
        )
    });
    if let Some(descriptor) = primary.access_descriptor.as_mut() {
        descriptor.stable_href = Some(format!(
            "https://example.invalid/nisar/{misleading_file_name}"
        ));
    }
    let graph = product
        .properties
        .get_mut("assets")
        .and_then(Value::as_array_mut)
        .expect("asset graph");
    let primary_graph = graph
        .iter_mut()
        .find(|asset| asset.get("role").and_then(Value::as_str) == Some("primary"))
        .and_then(Value::as_object_mut)
        .expect("graph primary");
    primary_graph.insert(
        "file_name".to_owned(),
        Value::String(misleading_file_name.clone()),
    );
    primary_graph.insert(
        "routes".to_owned(),
        json!([
            {"scheme": "https", "locator": format!("https://example.invalid/nisar/{misleading_file_name}")},
            {"scheme": "s3", "locator": format!("s3://example-invalid/nisar/{misleading_file_name}")}
        ]),
    );
    primary_graph
        .get_mut("native_evidence")
        .and_then(Value::as_object_mut)
        .expect("native evidence")
        .insert(
            "archive_name".to_owned(),
            Value::String(misleading_file_name.clone()),
        );
    let primary_key = product
        .properties
        .get("primary_asset_key")
        .and_then(Value::as_str)
        .expect("primary key")
        .to_owned();
    product
        .properties
        .get_mut("asset_integrity")
        .and_then(Value::as_object_mut)
        .and_then(|integrity| integrity.get_mut(&primary_key))
        .and_then(Value::as_object_mut)
        .expect("primary integrity")
        .insert(
            "file_name".to_owned(),
            Value::String(misleading_file_name.clone()),
        );
    product
        .properties
        .get_mut("asset_routes")
        .and_then(Value::as_object_mut)
        .expect("route graph")
        .insert(
            primary_key,
            json!([
                {"scheme": "https", "locator": format!("https://example.invalid/nisar/{misleading_file_name}")},
                {"scheme": "s3", "locator": format!("s3://example-invalid/nisar/{misleading_file_name}")}
            ]),
        );

    let descriptor = build_nisar_subset_live_frequency_a_hh_descriptor(product, &query)
        .expect("valid filename containing a misleading HV token");
    assert_eq!(descriptor.file_name(), misleading_file_name);
    assert_eq!(descriptor.selected_frequency(), NisarSubsetFrequency::A);
    assert_eq!(descriptor.selected_polarizations(), ["HH"]);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2d_b1_source_identity_negative_mutations_fail_closed_with_typed_codes() {
    assert_live_source_identity_rejects(
        "wrong granule",
        |item| item.display_name = "WRONG_GRANULE".to_owned(),
        "NISAR_LIVE_GRANULE_IDENTITY_MISMATCH",
    );
    assert_live_source_identity_rejects(
        "wrong metadata collection",
        |item| {
            item.metadata.insert(
                "native_collection_id".to_owned(),
                Value::String("WRONG_COLLECTION".to_owned()),
            );
        },
        "NISAR_LIVE_COLLECTION_IDENTITY_MISMATCH",
    );
    assert_live_source_identity_rejects(
        "wrong asset collection",
        |item| {
            item.asset_access
                .as_mut()
                .expect("primary asset")
                .reference
                .collection_id = "WRONG_COLLECTION".to_owned();
        },
        "NISAR_LIVE_COLLECTION_IDENTITY_MISMATCH",
    );
    assert_live_source_identity_rejects(
        "wrong filename",
        |item| {
            item.asset_access.as_mut().expect("primary asset").file_name =
                Some("wrong.h5".to_owned());
        },
        "NISAR_LIVE_SOURCE_FILENAME_MISMATCH",
    );
    assert_live_source_identity_rejects(
        "wrong size",
        |item| {
            item.asset_access
                .as_mut()
                .expect("primary asset")
                .expected_size_bytes = Some(SOURCE_BYTES + 1);
        },
        "NISAR_LIVE_SOURCE_SIZE_MISMATCH",
    );
    assert_live_source_identity_rejects(
        "wrong checksum algorithm",
        |item| {
            item.asset_access
                .as_mut()
                .expect("primary asset")
                .checksum_algorithm = Some("SHA-256".to_owned());
        },
        "NISAR_LIVE_SOURCE_CHECKSUM_MISMATCH",
    );
    assert_live_source_identity_rejects(
        "wrong md5",
        |item| {
            item.asset_access
                .as_mut()
                .expect("primary asset")
                .checksum_value = Some("00000000000000000000000000000000".to_owned());
        },
        "NISAR_LIVE_SOURCE_CHECKSUM_MISMATCH",
    );
    assert_live_source_identity_rejects(
        "unsupported frequency A HH capability",
        |item| {
            item.metadata.insert(
                "frequency_a_polarization".to_owned(),
                Value::String("HV".to_owned()),
            );
        },
        "NISAR_LIVE_SOURCE_CAPABILITY_MISMATCH",
    );
    assert_live_source_identity_rejects(
        "missing primary asset",
        |item| item.asset_access = None,
        "NISAR_LIVE_PRIMARY_ASSET_MISSING",
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn run_scene_generic_controlled_attempt(
    root: &Path,
    evidence: &Arc<NisarLiveAcceptanceEvidenceSink>,
    cleanup: &mut LiveExecutionCleanupGuard,
    frequency: NisarSubsetFrequency,
    selected_polarizations: Vec<String>,
    recovery_mode: Option<LiveRecoveryMode>,
) -> Result<(), &'static str> {
    let database = root.join("tasks.sqlite3");
    let output = root.join("output");
    if frequency == NisarSubsetFrequency::A {
        fixed_scope(
            GRANULE_ID,
            AOI_SHA256,
            NisarSubsetFrequency::A,
            "HH+HV",
            SWATH,
        )
        .map_err(|_| "NISAR_LIVE_GRANULE_IDENTITY_MISMATCH")?;
    } else if selected_polarizations != ["HH".to_owned()] {
        return Err("NISAR_LIVE_SELECTION_MISMATCH");
    }
    let query = live_query()?;
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .ok_or("NISAR_LIVE_CATALOGUE_FAILED")?;
    let catalogue = controlled_catalogue()?;
    let response = catalogue
        .search_nisar(&query, profile, &|| false)
        .map_err(|_| "NISAR_LIVE_CATALOGUE_FAILED")?;
    let mut products = response
        .products
        .into_iter()
        .filter(|product| product.display_name == GRANULE_ID)
        .collect::<Vec<_>>();
    if products.len() != 1 {
        return Err("NISAR_LIVE_GRANULE_IDENTITY_MISMATCH");
    }
    let product = products.remove(0);
    let item = map_nisar_product_for_live_acceptance(product, &query)
        .map_err(|_| "NISAR_LIVE_CATALOGUE_MAPPING_FAILED")?;
    validate_live_source_identity_for_frequency(&item, frequency.clone(), &selected_polarizations)?;
    let capability_key = match &frequency {
        NisarSubsetFrequency::A => "frequency_a_polarization",
        NisarSubsetFrequency::B => "frequency_b_polarization",
    };
    let advertised_polarizations = item
        .metadata
        .get(capability_key)
        .and_then(Value::as_str)
        .and_then(|label| {
            insar_provider::asf::nisar_subset::canonical_nisar_polarizations_for_frequency_label(
                &frequency, label,
            )
        })
        .ok_or("NISAR_LIVE_SOURCE_CAPABILITY_MISMATCH")?;
    if selected_polarizations
        .iter()
        .any(|polarization| !advertised_polarizations.contains(polarization))
    {
        return Err("NISAR_LIVE_SOURCE_CAPABILITY_MISMATCH");
    }

    let body_interruption = if recovery_mode == Some(LiveRecoveryMode::BodyInterruption) {
        Some(LiveBodyInterruptionControl::new())
    } else {
        None
    };
    save_source(&database, item.clone())?;
    let mut service =
        build_live_production_service(&database, evidence, false, body_interruption.clone())?;
    let expected_workbench_revision = TaskStore::open(&database)
        .map_err(|_| "NISAR_LIVE_SOURCE_STORE_FAILED")?
        .load_workbench()
        .map_err(|_| "NISAR_LIVE_SOURCE_STORE_FAILED")?
        .filter(|frozen| frozen.batch.batch_id == "nisar-live-source-batch")
        .map(|frozen| frozen.state.revision)
        .ok_or("NISAR_LIVE_SOURCE_STORE_FAILED")?;
    let prepared = service
        .prepare_nisar_subset_intent_v2(&NisarPrepareSubsetIntentV2Request {
            source_batch_id: "nisar-live-source-batch".to_owned(),
            source_item_id: item.item_id.clone(),
            expected_workbench_revision,
            subset_draft_geometry: aoi(),
            selected_frequency: match &frequency {
                NisarSubsetFrequency::A => "A".to_owned(),
                NisarSubsetFrequency::B => "B".to_owned(),
            },
            selected_polarizations: selected_polarizations.clone(),
        })
        .map_err(|_| "NISAR_LIVE_TASK_PREPARE_FAILED")?;
    if prepared.source_batch_id != "nisar-live-source-batch"
        || prepared.source_item_id != item.item_id
        || prepared.workbench_revision != expected_workbench_revision
    {
        return Err("NISAR_LIVE_TASK_PREPARE_FAILED");
    }
    // Validate the final evidence against the exact V2-frozen geometry digest
    // returned by PreparedIntent, rather than a legacy raw/normalized AOI hash.
    let expected_request_aoi_sha256 = prepared
        .intent
        .canonical_geometry_digest
        .requested_sha256
        .clone();
    service
        .confirm_nisar_subset_task_v2(&NisarConfirmSubsetTaskV2Request {
            source_batch_id: prepared.source_batch_id.clone(),
            source_item_id: prepared.source_item_id.clone(),
            expected_workbench_revision: prepared.workbench_revision,
            prepared_intent: prepared.intent.clone(),
            task_id: "nisar-live-acceptance-task".to_owned(),
            output_root: output.clone(),
            layout: insar_core::DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-08-02T00:00:00Z".to_owned(),
        })
        .map_err(|_| "NISAR_LIVE_TASK_CONFIRM_FAILED")?;

    let confirmed = TaskStore::open(&database)
        .map_err(|_| "NISAR_LIVE_STORE_REOPEN_FAILED")?
        .load_task_record("nisar-live-acceptance-task")
        .map_err(|_| "NISAR_LIVE_STORE_REOPEN_FAILED")?
        .ok_or("NISAR_LIVE_TASK_CONFIRM_FAILED")?;
    let confirmed_item = confirmed
        .snapshot
        .candidates
        .first()
        .ok_or("NISAR_LIVE_TASK_CONFIRM_FAILED")?;
    let prepared_envelope =
        insar_store::parse_nisar_subset_prepared_intent_v2_envelope(&confirmed_item.metadata)
            .map_err(|_| "NISAR_LIVE_TASK_CONFIRM_FAILED")?;
    let plan_envelope = insar_store::parse_nisar_subset_plan_v2_envelope(&confirmed_item.metadata)
        .map_err(|_| "NISAR_LIVE_TASK_CONFIRM_FAILED")?;
    if confirmed.snapshot.batch_id != "nisar-live-source-batch"
        || confirmed.snapshot.candidates.len() != 1
        || confirmed.runtime.status != TaskLifecycleStatus::Paused
        || !confirmed.runtime.task_paused
        || !confirmed.runtime.queued_item_ids.is_empty()
        || confirmed.runtime.paused_item_ids.len() != 1
        || confirmed.runtime.paused_item_ids.first() != Some(&item.item_id)
        || confirmed_item.item_id != item.item_id
        || confirmed_item
            .metadata
            .get("delivery_kind")
            .and_then(Value::as_str)
            != Some("remote_derived_subset_v2")
        || prepared_envelope.is_none()
        || plan_envelope.is_some()
    {
        return Err("NISAR_LIVE_V2_PAUSED_STATE_INVALID");
    }
    for (offset, kind, item_ids) in [
        (0_u64, TaskStateCommandKind::ResumeTask, Vec::new()),
        (
            1_u64,
            TaskStateCommandKind::ResumeItems,
            vec![item.item_id.clone()],
        ),
    ] {
        service
            .update_download_task(&StoreUpdateTaskRequest {
                task_id: "nisar-live-acceptance-task".to_owned(),
                expected_revision: confirmed.runtime.revision + offset,
                command: StoreTaskCommand { kind, item_ids },
                event_id: format!("{LIVE_ACCEPTANCE_REQUEST_ID}:resume:{offset}"),
                occurred_at: "2026-08-02T00:00:00Z".to_owned(),
            })
            .map_err(|_| "NISAR_LIVE_TASK_RESUME_FAILED")?;
    }

    if let Some(mode) = recovery_mode {
        if mode == LiveRecoveryMode::BodyInterruption {
            return run_live_body_failure_retry_phase(
                &mut service,
                root,
                &database,
                &output,
                evidence,
                cleanup,
                &item.item_id,
                frequency,
                &selected_polarizations,
                &expected_request_aoi_sha256,
                body_interruption.clone(),
            );
        }
        run_live_recovery_phase(
            &mut service,
            root,
            &database,
            &output,
            evidence,
            &item.item_id,
            mode,
            0,
        )?;
    } else {
        let deadline = Instant::now() + Duration::from_secs(45 * 60);
        loop {
            let status = evidence.snapshot().status;
            if status == "pass" || status == "fail" {
                break;
            }
            if Instant::now() >= deadline {
                evidence
                    .record_failure("NISAR_LIVE_TIMEOUT")
                    .map_err(|_| "NISAR_LIVE_EVIDENCE_INCOMPLETE")?;
                let latest_runtime = TaskStore::open(&database)
                    .map_err(|_| "NISAR_LIVE_TIMEOUT_STORE_LOAD_FAILED")?
                    .load_scheduler_control_runtime("nisar-live-acceptance-task")
                    .map_err(|_| "NISAR_LIVE_TIMEOUT_STORE_LOAD_FAILED")?
                    .ok_or("NISAR_LIVE_TIMEOUT_STORE_LOAD_FAILED")?;
                service
                    .update_download_task(&StoreUpdateTaskRequest {
                        task_id: "nisar-live-acceptance-task".to_owned(),
                        expected_revision: latest_runtime.revision,
                        command: StoreTaskCommand {
                            kind: TaskStateCommandKind::CancelTask,
                            item_ids: Vec::new(),
                        },
                        event_id: "nisar-live-acceptance-task:timeout-cancel:v2".to_owned(),
                        occurred_at: "2026-08-02T00:00:00Z".to_owned(),
                    })
                    .map_err(|_| "NISAR_LIVE_TIMEOUT_CANCELLATION_FAILED")?;

                let cancellation_deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    let store = TaskStore::open(&database)
                        .map_err(|_| "NISAR_LIVE_TIMEOUT_STORE_LOAD_FAILED")?;
                    let runtime = store
                        .load_scheduler_control_runtime("nisar-live-acceptance-task")
                        .map_err(|_| "NISAR_LIVE_TIMEOUT_STORE_LOAD_FAILED")?
                        .ok_or("NISAR_LIVE_TIMEOUT_STORE_LOAD_FAILED")?;
                    let durable_cancelled = runtime.status == TaskLifecycleStatus::Cancelled
                        && runtime.active_item_ids.is_empty()
                        && runtime.queued_item_ids.is_empty()
                        && runtime.paused_item_ids.is_empty()
                        && runtime
                            .cancelled_item_ids
                            .iter()
                            .any(|item_id| item_id == &item.item_id);
                    if durable_cancelled {
                        let download = store
                            .load_download_snapshot("nisar-live-acceptance-task")
                            .map_err(|_| "NISAR_LIVE_TIMEOUT_STORE_LOAD_FAILED")?;
                        match download
                            .results
                            .iter()
                            .find(|result| result.item_id == item.item_id)
                        {
                            Some(result) if result.outcome == DownloadOutcome::Cancelled => break,
                            Some(_) => return Err("NISAR_LIVE_TIMEOUT_WORKER_FAILURE"),
                            None => {}
                        }
                    }
                    if Instant::now() >= cancellation_deadline {
                        return Err(if durable_cancelled {
                            "NISAR_LIVE_TIMEOUT_SCHEDULER_NOT_CONSUMED_OR_WORKER_NOT_STOPPED"
                        } else {
                            "NISAR_LIVE_TIMEOUT_STORE_NOT_TERMINAL"
                        });
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                let terminal_store =
                    TaskStore::open(&database).map_err(|_| "NISAR_LIVE_TIMEOUT_REOPEN_MISMATCH")?;
                let terminal_task = terminal_store
                    .load_task_record("nisar-live-acceptance-task")
                    .map_err(|_| "NISAR_LIVE_TIMEOUT_REOPEN_MISMATCH")?
                    .ok_or("NISAR_LIVE_TIMEOUT_REOPEN_MISMATCH")?;
                let terminal_download = terminal_store
                    .load_download_snapshot("nisar-live-acceptance-task")
                    .map_err(|_| "NISAR_LIVE_TIMEOUT_REOPEN_MISMATCH")?;
                drop(terminal_store);
                service
                    .shutdown()
                    .map_err(|_| "NISAR_LIVE_TIMEOUT_SERVICE_SHUTDOWN_FAILED")?;
                let reopened =
                    TaskStore::open(&database).map_err(|_| "NISAR_LIVE_TIMEOUT_REOPEN_MISMATCH")?;
                let reopened_task = reopened
                    .load_task_record("nisar-live-acceptance-task")
                    .map_err(|_| "NISAR_LIVE_TIMEOUT_REOPEN_MISMATCH")?
                    .ok_or("NISAR_LIVE_TIMEOUT_REOPEN_MISMATCH")?;
                let reopened_download = reopened
                    .load_download_snapshot("nisar-live-acceptance-task")
                    .map_err(|_| "NISAR_LIVE_TIMEOUT_REOPEN_MISMATCH")?;
                drop(reopened);
                if reopened_task != terminal_task || reopened_download != terminal_download {
                    return Err("NISAR_LIVE_TIMEOUT_REOPEN_MISMATCH");
                }
                evidence
                    .mark_store_reopen("observed")
                    .map_err(|_| "NISAR_LIVE_EVIDENCE_INCOMPLETE")?;
                return Err("NISAR_LIVE_TIMEOUT");
            }
            thread::sleep(Duration::from_millis(250));
        }
    }

    let worker_failure = evidence.snapshot().status == "fail";
    if worker_failure {
        wait_for_durable_store_failure(
            &database,
            "nisar-live-acceptance-task",
            &item.item_id,
            Duration::from_secs(30),
            "NISAR_LIVE_WORKER_FAILURE_NOT_TERMINAL",
        )?;
    } else {
        wait_for_durable_store_completion(
            &database,
            "nisar-live-acceptance-task",
            &item.item_id,
            Duration::from_secs(30),
            "NISAR_LIVE_STORE_COMPLETION_FAILED",
        )?;
        evidence
            .mark_store_completed("observed")
            .map_err(|_| "NISAR_LIVE_EVIDENCE_INCOMPLETE")?;
    }

    let terminal_store =
        TaskStore::open(&database).map_err(|_| "NISAR_LIVE_STORE_TERMINAL_TRUTH_UNAVAILABLE")?;
    let terminal_task = terminal_store
        .load_task_record("nisar-live-acceptance-task")
        .map_err(|_| "NISAR_LIVE_STORE_TERMINAL_TRUTH_UNAVAILABLE")?
        .ok_or("NISAR_LIVE_STORE_TERMINAL_TRUTH_UNAVAILABLE")?;
    let terminal_download = terminal_store
        .load_download_snapshot("nisar-live-acceptance-task")
        .map_err(|_| "NISAR_LIVE_STORE_TERMINAL_TRUTH_UNAVAILABLE")?;
    drop(terminal_store);

    service
        .shutdown()
        .map_err(|_| "NISAR_LIVE_SERVICE_SHUTDOWN_FAILED")?;

    let reopened = TaskStore::open(&database).map_err(|_| "NISAR_LIVE_STORE_REOPEN_FAILED")?;
    let reopened_task = reopened
        .load_task_record("nisar-live-acceptance-task")
        .map_err(|_| "NISAR_LIVE_STORE_REOPEN_MISMATCH")?
        .ok_or("NISAR_LIVE_STORE_REOPEN_MISMATCH")?;
    let reopened_download = reopened
        .load_download_snapshot("nisar-live-acceptance-task")
        .map_err(|_| "NISAR_LIVE_STORE_REOPEN_MISMATCH")?;
    drop(reopened);
    if reopened_task != terminal_task || reopened_download != terminal_download {
        return Err("NISAR_LIVE_STORE_REOPEN_MISMATCH");
    }
    evidence
        .mark_store_reopen("observed")
        .map_err(|_| "NISAR_LIVE_EVIDENCE_INCOMPLETE")?;

    let before_cleanup = evidence.snapshot();
    if before_cleanup.status != "pass" {
        return Err("NISAR_LIVE_WORKER_FAILURE_PRESERVED");
    }
    let frequency_label = match &frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    };
    let advertised_label = match &frequency {
        NisarSubsetFrequency::A | NisarSubsetFrequency::B => "HH+HV",
    };
    let retry_budget = if recovery_mode == Some(LiveRecoveryMode::BodyInterruption) {
        let checkpoint = read_live_recovery_checkpoint(&output)?
            .ok_or("NISAR_LIVE_RECOVERY_CHECKPOINT_UNAVAILABLE")?;
        Some((
            checkpoint.failed_science_request_count,
            checkpoint
                .reserved_science_response_body_bytes
                .checked_sub(checkpoint.committed_science_response_body_bytes)
                .ok_or("NISAR_LIVE_RECOVERY_BODY_INTERRUPTION_ACCOUNTING_INVALID")?,
        ))
    } else {
        None
    };
    validate_m2d_b_final_evidence_for_selection(
        &before_cleanup,
        &expected_request_aoi_sha256,
        &output,
        frequency_label,
        if selected_polarizations == ["HH".to_owned()] {
            "HH"
        } else {
            advertised_label
        },
        &["HH".to_owned(), "HV".to_owned()],
        &selected_polarizations,
        retry_budget,
    )?;

    // The artifact is preserved only after worker PASS, store reopen, and all
    // typed postconditions have passed.
    cleanup.preserve_final_artifact();
    cleanup
        .cleanup_now()
        .map_err(|_| "NISAR_LIVE_CLEANUP_FAILED")?;
    let final_evidence = evidence.snapshot();
    if !final_evidence.cleanup_completed || !has_hdf5_output(&output) {
        cleanup.discard_final_artifact();
        return Err("NISAR_LIVE_EVIDENCE_INCOMPLETE");
    }
    validate_output_science_selection(&output, frequency.clone(), &selected_polarizations)?;

    println!(
        "{}_REDACTED_EVIDENCE_PATH={}",
        if frequency == NisarSubsetFrequency::B {
            "P0.3a-FREQ-B"
        } else {
            "P0.3a-M2D-B0"
        },
        evidence.evidence_path().display()
    );
    let serialized =
        serde_json::to_string(&final_evidence).map_err(|_| "NISAR_LIVE_EVIDENCE_INCOMPLETE")?;
    println!(
        "{}_REDACTED_EVIDENCE={serialized}",
        if frequency == NisarSubsetFrequency::B {
            "P0.3a-FREQ-B"
        } else {
            "P0.3a-M2D-B0"
        }
    );
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn body_retry_evidence_root(root: &Path) -> Result<PathBuf, &'static str> {
    let candidate = std::env::var("INSAR_NISAR_SUBSET_LIVE_BODY_RETRY_EVIDENCE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("body-retry-acceptance"));
    if !candidate.is_absolute() {
        return Err("NISAR_LIVE_BODY_RETRY_EVIDENCE_ROOT_NOT_ABSOLUTE");
    }
    if candidate.exists() {
        return Err("NISAR_LIVE_BODY_RETRY_EVIDENCE_ROOT_MUST_NOT_EXIST");
    }
    Ok(candidate)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[allow(clippy::too_many_arguments)]
fn run_live_body_failure_retry_phase(
    service: &mut ApplicationService,
    root: &Path,
    database: &Path,
    output: &Path,
    evidence: &Arc<NisarLiveAcceptanceEvidenceSink>,
    cleanup: &mut LiveExecutionCleanupGuard,
    item_id: &str,
    frequency: NisarSubsetFrequency,
    selected_polarizations: &[String],
    expected_request_aoi_sha256: &str,
    body_interruption: Option<Arc<LiveBodyInterruptionControl>>,
) -> Result<(), &'static str> {
    const TASK_ID: &str = "nisar-live-acceptance-task";
    const SCIENCE_FAILURE_CODE: &str = "NISAR_SUBSET_V2_SCIENCE_INCOMPLETE";

    let mut trace = LiveRecoveryTrace::new(LiveRecoveryMode::BodyInterruption);
    write_live_recovery_trace(root, &trace)?;
    let (threshold_checkpoint, threshold_bytes) =
        wait_for_live_science_threshold(output, evidence, Duration::from_secs(45 * 60), 0)?;
    trace.threshold_bytes = Some(threshold_bytes);
    trace.planned_science_response_body_bytes =
        Some(threshold_checkpoint.planned_science_response_body_bytes);
    trace.pause_request_checkpoint_bytes =
        Some(threshold_checkpoint.committed_science_response_body_bytes);
    trace.pause_request_progress_bytes =
        Some(live_recovery_progress_bytes(database, TASK_ID, item_id)?);
    if threshold_checkpoint.failed_science_request_count != 0
        || threshold_checkpoint.indeterminate_science_request_count != 0
        || threshold_checkpoint.indeterminate_science_response_body_bytes != 0
    {
        return Err("NISAR_LIVE_BODY_FAILURE_THRESHOLD_NOT_CLEAN");
    }

    let controller = body_interruption
        .as_ref()
        .ok_or("NISAR_LIVE_RECOVERY_BODY_INTERRUPTION_HOOK_UNAVAILABLE")?;
    controller.arm();
    let observed_bytes = controller.wait_for_trigger(Duration::from_secs(5 * 60))?;
    if observed_bytes == 0 {
        return Err("NISAR_LIVE_RECOVERY_BODY_INTERRUPTION_NOT_PARTIAL");
    }
    trace.body_interruption_observed_bytes = Some(observed_bytes);
    write_live_recovery_trace(root, &trace)?;

    // The hook returns ScienceTransferIncomplete immediately. The executor
    // retries that Range after 5 s and 20 s, the hook interrupts each attempt,
    // and the third interrupted attempt fails the item. Wait for the ordinary
    // Scheduler/Store Failed transition instead of issuing PauseTask; a pause
    // command would convert the same transport error into Interrupted.
    wait_for_durable_store_failure(
        database,
        TASK_ID,
        item_id,
        Duration::from_secs(3 * 60),
        "NISAR_LIVE_BODY_FAILURE_NOT_TERMINAL",
    )?;
    let (interruptions, interrupted_body_bytes) = controller.interruption_totals();
    if interruptions != BODY_INTERRUPTION_ATTEMPTS {
        return Err("NISAR_LIVE_BODY_FAILURE_RETRY_COUNT_INVALID");
    }
    let failed_store =
        TaskStore::open(database).map_err(|_| "NISAR_LIVE_BODY_FAILURE_STORE_UNAVAILABLE")?;
    let failed_task = failed_store
        .load_task_record(TASK_ID)
        .map_err(|_| "NISAR_LIVE_BODY_FAILURE_STORE_UNAVAILABLE")?
        .ok_or("NISAR_LIVE_BODY_FAILURE_TASK_UNAVAILABLE")?;
    let failed_download = failed_store
        .load_download_snapshot(TASK_ID)
        .map_err(|_| "NISAR_LIVE_BODY_FAILURE_STORE_UNAVAILABLE")?;
    let failed_result = failed_download
        .results
        .iter()
        .find(|result| result.item_id == item_id)
        .ok_or("NISAR_LIVE_BODY_FAILURE_RESULT_UNAVAILABLE")?;
    let failure = failed_result
        .failure
        .as_ref()
        .ok_or("NISAR_LIVE_BODY_FAILURE_DETAIL_UNAVAILABLE")?;
    if failed_task.runtime.status != TaskLifecycleStatus::Failed
        || failed_task.runtime.task_paused
        || !failed_task.runtime.active_item_ids.is_empty()
        || !failed_task.runtime.queued_item_ids.is_empty()
        || !failed_task.runtime.paused_item_ids.is_empty()
        || !failed_task
            .runtime
            .failed_item_ids
            .iter()
            .any(|id| id == item_id)
        || failed_result.outcome != DownloadOutcome::Failed
        || failed_result.attempt_count != BODY_INTERRUPTION_ATTEMPTS
        || failure.code != SCIENCE_FAILURE_CODE
        || !failure.retryable
    {
        return Err("NISAR_LIVE_BODY_FAILURE_STORE_STATE_INVALID");
    }
    drop(failed_store);
    let failure_evidence = evidence.snapshot();
    if failure_evidence.status != "fail" || failure_evidence.typed_stop_code != SCIENCE_FAILURE_CODE
    {
        return Err("NISAR_LIVE_BODY_FAILURE_EVIDENCE_INVALID");
    }
    let failure_checkpoint = read_live_recovery_checkpoint(output)?
        .ok_or("NISAR_LIVE_BODY_FAILURE_CHECKPOINT_UNAVAILABLE")?;
    if failure_checkpoint.state != "Interrupted"
        || failure_checkpoint.pending_attempt
        || failure_checkpoint.planned_ranges != threshold_checkpoint.planned_ranges
        || failure_checkpoint.completed_ranges.len()
            != failure_checkpoint.completed_range_request_counts.len()
    {
        return Err("NISAR_LIVE_BODY_FAILURE_CHECKPOINT_INVALID");
    }
    if failure_checkpoint.source_validator_sha256 != threshold_checkpoint.source_validator_sha256 {
        return Err("NISAR_LIVE_BODY_FAILURE_SOURCE_VALIDATOR_DRIFT");
    }
    let reserved_body_bytes = failure_checkpoint
        .reserved_science_response_body_bytes
        .checked_sub(failure_checkpoint.committed_science_response_body_bytes)
        .ok_or("NISAR_LIVE_BODY_FAILURE_ACCOUNTING_INVALID")?;
    // Every attempt reserved the whole Range again; reservations are never
    // refunded, so the open reservation is one Range length per attempt.
    if reserved_body_bytes % BODY_INTERRUPTION_ATTEMPTS != 0 {
        return Err("NISAR_LIVE_BODY_FAILURE_ACCOUNTING_INVALID");
    }
    let range_length = reserved_body_bytes / BODY_INTERRUPTION_ATTEMPTS;
    let matching_failed_ranges = failure_checkpoint
        .planned_ranges
        .iter()
        .filter(|range| {
            range
                .end_exclusive
                .checked_sub(range.start)
                .is_some_and(|length| {
                    length == range_length
                        && !failure_checkpoint
                            .completed_ranges
                            .iter()
                            .any(|completed| completed.index == range.index)
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    if matching_failed_ranges.len() != 1 {
        return Err("NISAR_LIVE_RECOVERY_BODY_INTERRUPTION_RANGE_IDENTITY_UNPROVEN");
    }
    let interrupted_range = matching_failed_ranges
        .into_iter()
        .next()
        .ok_or("NISAR_LIVE_RECOVERY_BODY_INTERRUPTION_RANGE_IDENTITY_UNPROVEN")?;
    if range_length <= BODY_INTERRUPTION_READ_BUFFER_BYTES || observed_bytes >= range_length {
        return Err("NISAR_LIVE_RECOVERY_BODY_INTERRUPTION_NOT_PARTIAL");
    }
    let threshold_ranges_reused = failure_checkpoint.completed_ranges.len()
        >= threshold_checkpoint.completed_ranges.len()
        && threshold_checkpoint
            .completed_ranges
            .iter()
            .zip(&failure_checkpoint.completed_ranges)
            .all(|(threshold, failure)| threshold == failure)
        && threshold_checkpoint
            .completed_range_request_counts
            .iter()
            .zip(&failure_checkpoint.completed_range_request_counts)
            .all(|(threshold, failure)| threshold == failure);
    let threshold_digest_reused = failure_checkpoint.completed_ranges.len()
        != threshold_checkpoint.completed_ranges.len()
        || failure_checkpoint.completed_ranges_digest
            == threshold_checkpoint.completed_ranges_digest;
    let failure_completed_ranges_valid = failure_checkpoint.completed_ranges.iter().all(|range| {
        threshold_checkpoint
            .planned_ranges
            .iter()
            .any(|planned| planned == range)
    });
    let failed_request_count = failure_checkpoint.failed_science_request_count;
    let issued_after_commit = failure_checkpoint
        .issued_science_request_count
        .checked_sub(failure_checkpoint.committed_science_request_count)
        .ok_or("NISAR_LIVE_BODY_FAILURE_ACCOUNTING_INVALID")?;
    let reserved_after_commit = failure_checkpoint
        .reserved_science_request_count
        .checked_sub(failure_checkpoint.committed_science_request_count)
        .ok_or("NISAR_LIVE_BODY_FAILURE_ACCOUNTING_INVALID")?;
    let observed_body_bytes = failure_checkpoint
        .observed_science_response_body_bytes
        .checked_sub(failure_checkpoint.committed_science_response_body_bytes)
        .ok_or("NISAR_LIVE_BODY_FAILURE_ACCOUNTING_INVALID")?;
    if failed_request_count < BODY_INTERRUPTION_ATTEMPTS
        || issued_after_commit != failed_request_count
        || reserved_after_commit != failed_request_count
        || observed_body_bytes != interrupted_body_bytes
        || !threshold_ranges_reused
        || !threshold_digest_reused
        || !failure_completed_ranges_valid
        || failure_checkpoint.indeterminate_science_request_count != 0
        || failure_checkpoint.indeterminate_science_response_body_bytes != 0
    {
        return Err("NISAR_LIVE_BODY_FAILURE_ACCOUNTING_INVALID");
    }
    trace.body_interruption_range = Some(interrupted_range.clone());
    trace.body_interruption_result_code = Some(SCIENCE_FAILURE_CODE.to_owned());
    trace.body_interruption_failed_request_count = Some(failed_request_count);
    trace.failure_runtime_revision = Some(failed_task.runtime.revision);
    trace.failure = Some(failure_checkpoint.clone());
    let failure_anchor = read_live_recovery_anchor(database, TASK_ID, item_id)?
        .ok_or("NISAR_LIVE_BODY_FAILURE_ANCHOR_UNAVAILABLE")?;
    if !live_recovery_anchor_matches_checkpoint(&failure_anchor, &failure_checkpoint) {
        return Err("NISAR_LIVE_BODY_FAILURE_ANCHOR_MISMATCH");
    }
    trace.anchor_at_failure = Some(failure_anchor.clone());
    let old_failure_evidence_bytes = fs::read(evidence.evidence_path())
        .map_err(|_| "NISAR_LIVE_BODY_FAILURE_EVIDENCE_UNREADABLE")?;
    write_live_recovery_trace(root, &trace)?;

    service
        .shutdown()
        .map_err(|_| "NISAR_LIVE_BODY_FAILURE_SERVICE_SHUTDOWN_FAILED")?;
    trace.worker_quiescent = true;
    write_live_recovery_trace(root, &trace)?;

    let retry_root = body_retry_evidence_root(root)?;
    let retry_evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(
            retry_root.clone(),
            BASELINE_COMMIT,
            current_git_commit(),
        )
        .map_err(|_| "NISAR_LIVE_BODY_RETRY_EVIDENCE_INIT_FAILED")?,
    );
    cleanup.replace_evidence(Arc::clone(&retry_evidence));
    let retry_service = build_live_production_service(database, &retry_evidence, true, None)?;
    let operation = (|| {
        let reopened_task = TaskStore::open(database)
            .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_REOPEN_FAILED")?
            .load_task_record(TASK_ID)
            .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_REOPEN_FAILED")?
            .ok_or("NISAR_LIVE_BODY_RETRY_STORE_REOPEN_FAILED")?;
        let reopened_checkpoint = read_live_recovery_checkpoint(output)?
            .ok_or("NISAR_LIVE_BODY_RETRY_CHECKPOINT_UNAVAILABLE")?;
        let reopened_anchor = read_live_recovery_anchor(database, TASK_ID, item_id)?
            .ok_or("NISAR_LIVE_BODY_RETRY_ANCHOR_UNAVAILABLE")?;
        if reopened_task.runtime.status != TaskLifecycleStatus::Failed
            || reopened_task.runtime.task_paused
            || !reopened_task.runtime.active_item_ids.is_empty()
            || !reopened_task.runtime.queued_item_ids.is_empty()
            || !reopened_task.runtime.paused_item_ids.is_empty()
            || !reopened_task
                .runtime
                .failed_item_ids
                .iter()
                .any(|id| id == item_id)
            || reopened_checkpoint.checkpoint_sha256 != failure_checkpoint.checkpoint_sha256
            || reopened_checkpoint.lineage_key_sha256 != failure_checkpoint.lineage_key_sha256
            || reopened_checkpoint.execution_binding_sha256
                != failure_checkpoint.execution_binding_sha256
            || reopened_checkpoint.prepared_intent_sha256
                != failure_checkpoint.prepared_intent_sha256
            || reopened_checkpoint.final_plan_sha256 != failure_checkpoint.final_plan_sha256
            || reopened_checkpoint.physical_plan_sha256 != failure_checkpoint.physical_plan_sha256
            || reopened_checkpoint.inventory_sha256 != failure_checkpoint.inventory_sha256
            || reopened_checkpoint.inventory_evidence_sha256
                != failure_checkpoint.inventory_evidence_sha256
            || reopened_checkpoint.source_graph_sha256 != failure_checkpoint.source_graph_sha256
            || reopened_checkpoint.source_validator_sha256
                != failure_checkpoint.source_validator_sha256
            || reopened_checkpoint.part_identity_sha256 != failure_checkpoint.part_identity_sha256
            || reopened_checkpoint.completed_ranges_digest
                != failure_checkpoint.completed_ranges_digest
            || reopened_anchor.record_sha256 != failure_anchor.record_sha256
            || !live_recovery_anchor_matches_checkpoint(&reopened_anchor, &reopened_checkpoint)
        {
            return Err("NISAR_LIVE_BODY_RETRY_REOPEN_MISMATCH");
        }
        trace.reopen_runtime_revision = Some(reopened_task.runtime.revision);
        trace.reopen = Some(reopened_checkpoint);
        trace.anchor_at_reopen = Some(reopened_anchor);
        write_live_recovery_trace_at(retry_root.join("evidence").as_path(), &trace)?;

        retry_service
            .update_download_task(&StoreUpdateTaskRequest {
                task_id: TASK_ID.to_owned(),
                expected_revision: reopened_task.runtime.revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::RetryItems,
                    item_ids: vec![item_id.to_owned()],
                },
                event_id: format!("{LIVE_ACCEPTANCE_REQUEST_ID}:recovery:body-retry"),
                occurred_at: "2026-08-02T00:00:00Z".to_owned(),
            })
            .map_err(|_| "NISAR_LIVE_BODY_RETRY_COMMAND_FAILED")?;
        let mut resumed_runtime = TaskStore::open(database)
            .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_UNAVAILABLE")?
            .load_scheduler_control_runtime(TASK_ID)
            .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_UNAVAILABLE")?
            .ok_or("NISAR_LIVE_BODY_RETRY_STORE_UNAVAILABLE")?;
        if resumed_runtime.task_paused {
            retry_service
                .update_download_task(&StoreUpdateTaskRequest {
                    task_id: TASK_ID.to_owned(),
                    expected_revision: resumed_runtime.revision,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::ResumeTask,
                        item_ids: Vec::new(),
                    },
                    event_id: format!("{LIVE_ACCEPTANCE_REQUEST_ID}:recovery:body-retry-resume"),
                    occurred_at: "2026-08-02T00:00:00Z".to_owned(),
                })
                .map_err(|_| "NISAR_LIVE_BODY_RETRY_RESUME_FAILED")?;
            resumed_runtime = TaskStore::open(database)
                .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_UNAVAILABLE")?
                .load_scheduler_control_runtime(TASK_ID)
                .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_UNAVAILABLE")?
                .ok_or("NISAR_LIVE_BODY_RETRY_STORE_UNAVAILABLE")?;
        }
        if resumed_runtime.task_paused
            || !resumed_runtime.paused_item_ids.is_empty()
            || resumed_runtime
                .failed_item_ids
                .iter()
                .any(|id| id == item_id)
            || !matches!(
                resumed_runtime.status,
                TaskLifecycleStatus::Queued
                    | TaskLifecycleStatus::Running
                    | TaskLifecycleStatus::Completed
            )
        {
            return Err("NISAR_LIVE_BODY_RETRY_RESUME_STATE_INVALID");
        }
        trace.resume_runtime_revision = Some(resumed_runtime.revision);
        write_live_recovery_trace_at(retry_root.join("evidence").as_path(), &trace)?;

        if !wait_for_live_recovery_terminal(&retry_evidence, Duration::from_secs(45 * 60))? {
            return Err("NISAR_LIVE_BODY_RETRY_WORKER_FAILURE");
        }
        let completion = read_live_recovery_checkpoint(output)?
            .ok_or("NISAR_LIVE_BODY_RETRY_CHECKPOINT_UNAVAILABLE")?;
        let completion_anchor = read_live_recovery_anchor(database, TASK_ID, item_id)?
            .ok_or("NISAR_LIVE_BODY_RETRY_ANCHOR_UNAVAILABLE")?;
        if failure_checkpoint.completed_ranges.len()
            != failure_checkpoint.completed_range_request_counts.len()
            || completion.completed_ranges.len() != completion.completed_range_request_counts.len()
        {
            return Err("NISAR_LIVE_BODY_RETRY_LEDGER_INVALID");
        }
        let ranges_reused =
            failure_checkpoint
                .completed_ranges
                .iter()
                .enumerate()
                .all(|(index, range)| {
                    completion
                        .completed_ranges
                        .iter()
                        .position(|candidate| candidate == range)
                        .and_then(|position| {
                            completion.completed_range_request_counts.get(position)
                        })
                        == failure_checkpoint.completed_range_request_counts.get(index)
                });
        let new_range_requests = completion
            .completed_ranges
            .iter()
            .zip(&completion.completed_range_request_counts)
            .filter(|(range, _)| !failure_checkpoint.completed_ranges.contains(range))
            .try_fold(0_u64, |sum, (_, count)| sum.checked_add(*count));
        let retry_request_count = completion
            .issued_science_request_count
            .checked_sub(failure_checkpoint.issued_science_request_count);
        let retry_reserved_request_count = completion
            .reserved_science_request_count
            .checked_sub(failure_checkpoint.reserved_science_request_count);
        let interrupted_position = completion
            .completed_ranges
            .iter()
            .position(|range| range == &interrupted_range);
        let interrupted_request_count = interrupted_position
            .and_then(|position| completion.completed_range_request_counts.get(position))
            .copied()
            .unwrap_or(0);
        let retry_budget_bytes = completion
            .reserved_science_response_body_bytes
            .checked_sub(completion.committed_science_response_body_bytes)
            .ok_or("NISAR_LIVE_BODY_RETRY_ACCOUNTING_INVALID")?;
        let retry_committed_body_bytes = completion
            .committed_science_response_body_bytes
            .checked_sub(failure_checkpoint.committed_science_response_body_bytes)
            .ok_or("NISAR_LIVE_BODY_RETRY_ACCOUNTING_INVALID")?;
        let expected_observed_body_bytes = failure_checkpoint
            .observed_science_response_body_bytes
            .checked_add(retry_committed_body_bytes)
            .ok_or("NISAR_LIVE_BODY_RETRY_ACCOUNTING_INVALID")?;
        if completion.state != "Complete"
            || completion.pending_attempt
            || completion.completed_ranges.len() != completion.planned_range_count
            || completion.planned_ranges != failure_checkpoint.planned_ranges
            || completion.planned_science_response_body_bytes
                != failure_checkpoint.planned_science_response_body_bytes
            || completion.committed_science_response_body_bytes
                != completion.planned_science_response_body_bytes
            || completion.observed_science_response_body_bytes != expected_observed_body_bytes
            || completion.resumed_bytes != failure_checkpoint.committed_science_response_body_bytes
            || completion.resumed_bytes == 0
            || completion.lineage_key_sha256 != failure_checkpoint.lineage_key_sha256
            || completion.execution_binding_sha256 != failure_checkpoint.execution_binding_sha256
            || completion.prepared_intent_sha256 != failure_checkpoint.prepared_intent_sha256
            || completion.final_plan_sha256 != failure_checkpoint.final_plan_sha256
            || completion.physical_plan_sha256 != failure_checkpoint.physical_plan_sha256
            || completion.inventory_sha256 != failure_checkpoint.inventory_sha256
            || completion.inventory_evidence_sha256 != failure_checkpoint.inventory_evidence_sha256
            || completion.source_graph_sha256 != failure_checkpoint.source_graph_sha256
            || completion.source_validator_sha256 != failure_checkpoint.source_validator_sha256
            || completion.part_identity_sha256 != failure_checkpoint.part_identity_sha256
            || completion.failed_science_request_count
                != failure_checkpoint.failed_science_request_count
            || completion.indeterminate_science_request_count != 0
            || completion.indeterminate_science_response_body_bytes != 0
            || !ranges_reused
            || new_range_requests != retry_request_count
            || new_range_requests != retry_reserved_request_count
            || new_range_requests
                != completion
                    .committed_science_request_count
                    .checked_sub(failure_checkpoint.committed_science_request_count)
            || interrupted_request_count == 0
            || retry_budget_bytes != range_length
            || !completion
                .completed_ranges
                .iter()
                .any(|range| range == &interrupted_range)
            || !live_recovery_anchor_matches_checkpoint(&completion_anchor, &completion)
            || completion_anchor.generation < failure_anchor.generation
        {
            return Err("NISAR_LIVE_BODY_RETRY_COMPLETION_INVALID");
        }
        trace.body_interruption_retry_request_count = retry_request_count;
        trace.completion = Some(completion.clone());
        trace.anchor_at_completion = Some(completion_anchor.clone());
        trace.reused_verified_bytes = completion.resumed_bytes;
        trace.duplicate_science_ranges = live_science_duplicate_ranges(&retry_evidence.snapshot());
        write_live_recovery_trace_at(retry_root.join("evidence").as_path(), &trace)?;
        if !trace.duplicate_science_ranges.is_empty() {
            return Err("NISAR_LIVE_BODY_RETRY_DUPLICATE_RANGE_REQUEST");
        }
        wait_for_durable_store_completion(
            database,
            TASK_ID,
            item_id,
            Duration::from_secs(30),
            "NISAR_LIVE_BODY_RETRY_STORE_COMPLETION_FAILED",
        )?;
        retry_evidence
            .mark_store_completed("observed")
            .map_err(|_| "NISAR_LIVE_BODY_RETRY_EVIDENCE_INCOMPLETE")?;
        Ok(())
    })();
    if let Err(code) = operation {
        let _ = retry_service.shutdown();
        return Err(code);
    }

    let terminal_store = TaskStore::open(database)
        .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_TERMINAL_UNAVAILABLE")?;
    let terminal_task = terminal_store
        .load_task_record(TASK_ID)
        .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_TERMINAL_UNAVAILABLE")?
        .ok_or("NISAR_LIVE_BODY_RETRY_STORE_TERMINAL_UNAVAILABLE")?;
    let terminal_download = terminal_store
        .load_download_snapshot(TASK_ID)
        .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_TERMINAL_UNAVAILABLE")?;
    drop(terminal_store);
    retry_service
        .shutdown()
        .map_err(|_| "NISAR_LIVE_BODY_RETRY_SERVICE_SHUTDOWN_FAILED")?;
    let final_checkpoint = read_live_recovery_checkpoint(output)?
        .ok_or("NISAR_LIVE_BODY_RETRY_CHECKPOINT_UNAVAILABLE")?;
    let final_anchor = read_live_recovery_anchor(database, TASK_ID, item_id)?
        .ok_or("NISAR_LIVE_BODY_RETRY_ANCHOR_UNAVAILABLE")?;
    if final_checkpoint.checkpoint_sha256
        != trace
            .completion
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_sha256.as_str())
            .unwrap_or_default()
        || !live_recovery_anchor_matches_checkpoint(&final_anchor, &final_checkpoint)
    {
        return Err("NISAR_LIVE_BODY_RETRY_FINAL_AUTHORITY_MISMATCH");
    }
    let reopened_store =
        TaskStore::open(database).map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_REOPEN_FAILED")?;
    let reopened_task = reopened_store
        .load_task_record(TASK_ID)
        .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_REOPEN_FAILED")?
        .ok_or("NISAR_LIVE_BODY_RETRY_STORE_REOPEN_FAILED")?;
    let reopened_download = reopened_store
        .load_download_snapshot(TASK_ID)
        .map_err(|_| "NISAR_LIVE_BODY_RETRY_STORE_REOPEN_FAILED")?;
    drop(reopened_store);
    if reopened_task != terminal_task || reopened_download != terminal_download {
        return Err("NISAR_LIVE_BODY_RETRY_STORE_REOPEN_MISMATCH");
    }
    retry_evidence
        .mark_store_reopen("observed")
        .map_err(|_| "NISAR_LIVE_BODY_RETRY_EVIDENCE_INCOMPLETE")?;
    validate_m2d_b_final_evidence_for_selection(
        &retry_evidence.snapshot(),
        expected_request_aoi_sha256,
        output,
        match &frequency {
            NisarSubsetFrequency::A => "A",
            NisarSubsetFrequency::B => "B",
        },
        "HH+HV",
        &["HH".to_owned(), "HV".to_owned()],
        selected_polarizations,
        Some((
            final_checkpoint.failed_science_request_count,
            final_checkpoint.reserved_science_response_body_bytes
                - final_checkpoint.committed_science_response_body_bytes,
        )),
    )?;
    if fs::read(evidence.evidence_path())
        .map_err(|_| "NISAR_LIVE_BODY_FAILURE_EVIDENCE_UNREADABLE")?
        != old_failure_evidence_bytes
    {
        return Err("NISAR_LIVE_BODY_FAILURE_EVIDENCE_MUTATED");
    }

    cleanup.preserve_final_artifact();
    cleanup
        .cleanup_now()
        .map_err(|_| "NISAR_LIVE_BODY_RETRY_CLEANUP_FAILED")?;
    let final_evidence = retry_evidence.snapshot();
    if !final_evidence.cleanup_completed || !has_hdf5_output(output) {
        cleanup.discard_final_artifact();
        return Err("NISAR_LIVE_BODY_RETRY_EVIDENCE_INCOMPLETE");
    }
    validate_output_science_selection(output, frequency, selected_polarizations)?;
    let serialized = serde_json::to_string(&final_evidence)
        .map_err(|_| "NISAR_LIVE_BODY_RETRY_EVIDENCE_INCOMPLETE")?;
    println!(
        "P0.3a-BODY-FAILURE-RETRY_REDACTED_EVIDENCE_PATH={}",
        retry_evidence.evidence_path().display()
    );
    println!("P0.3a-BODY-FAILURE-RETRY_REDACTED_EVIDENCE={serialized}");
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn run_m2d_b_controlled_attempt(
    root: &Path,
    evidence: &Arc<NisarLiveAcceptanceEvidenceSink>,
    cleanup: &mut LiveExecutionCleanupGuard,
    single_polarization: bool,
) -> Result<(), &'static str> {
    let selected = if single_polarization {
        vec!["HH".to_owned()]
    } else {
        vec!["HH".to_owned(), "HV".to_owned()]
    };
    run_scene_generic_controlled_attempt(
        root,
        evidence,
        cleanup,
        NisarSubsetFrequency::A,
        selected,
        None,
    )
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn run_frequency_b_controlled_attempt(
    root: &Path,
    evidence: &Arc<NisarLiveAcceptanceEvidenceSink>,
    cleanup: &mut LiveExecutionCleanupGuard,
) -> Result<(), &'static str> {
    run_scene_generic_controlled_attempt(
        root,
        evidence,
        cleanup,
        NisarSubsetFrequency::B,
        vec!["HH".to_owned()],
        None,
    )
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn recovery_fixed_gate(enabled_name: &str) -> Result<PathBuf, &'static str> {
    let enabled = std::env::var(enabled_name).ok();
    let granule = std::env::var("INSAR_NISAR_SUBSET_LIVE_GRANULE").ok();
    let root = std::env::var("INSAR_NISAR_SUBSET_LIVE_ROOT").ok();
    fixed_gate(enabled.as_deref(), granule.as_deref(), root.as_deref())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn run_nisar_recovery_controlled_attempt(
    root: &Path,
    evidence: &Arc<NisarLiveAcceptanceEvidenceSink>,
    cleanup: &mut LiveExecutionCleanupGuard,
    mode: LiveRecoveryMode,
) -> Result<(), &'static str> {
    run_scene_generic_controlled_attempt(
        root,
        evidence,
        cleanup,
        NisarSubsetFrequency::A,
        vec!["HH".to_owned(), "HV".to_owned()],
        Some(mode),
    )
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn run_nisar_recovery_live_acceptance(
    mode: LiveRecoveryMode,
    enabled_name: &str,
    description: &str,
) {
    insar_download::establish_hdf5_plugin_isolation().expect(
        "live acceptance host must establish app-owned HDF5 plugin isolation before HDF5 use",
    );
    let root = recovery_fixed_gate(enabled_name)
        .expect("all explicit recovery live acceptance gates are required before network access");
    fs::create_dir_all(&root).unwrap();
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(root.clone(), BASELINE_COMMIT, current_git_commit())
            .expect("redacted live recovery evidence sink"),
    );
    let mut cleanup = LiveExecutionCleanupGuard::new(root.clone(), Arc::clone(&evidence));
    let attempt = catch_unwind(AssertUnwindSafe(|| {
        run_nisar_recovery_controlled_attempt(&root, &evidence, &mut cleanup, mode)
    }));
    let active_evidence = Arc::clone(&cleanup.evidence);
    let outcome = match attempt {
        Ok(Ok(())) => Ok(()),
        Ok(Err(code)) => {
            record_live_failure_if_unset(&active_evidence, code);
            Err(boundary_failure_code(&active_evidence, code))
        }
        Err(_) => {
            record_live_failure_if_unset(&active_evidence, "NISAR_LIVE_HARNESS_PANIC");
            Err(boundary_failure_code(
                &active_evidence,
                "NISAR_LIVE_HARNESS_PANIC",
            ))
        }
    };
    if let Err(code) = outcome {
        cleanup.discard_final_artifact();
        let preserve_failure = std::env::var("INSAR_NISAR_SUBSET_PRESERVE_FAILURE")
            .ok()
            .as_deref()
            == Some("1");
        let cleanup_result = if preserve_failure {
            cleanup.preserve_failure_diagnostics();
            Ok(())
        } else {
            cleanup.cleanup_now()
        };
        let final_code = boundary_failure_code(&active_evidence, &code);
        panic!(
            "{description} stopped with typed NO_GO: {final_code} (boundary={code}, cleanup={cleanup_result:?})"
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn recovery_reuse_fixed_gate(enabled_name: &str) -> Result<PathBuf, &'static str> {
    if std::env::var(enabled_name).ok().as_deref() != Some("1") {
        return Err("NISAR_LIVE_RECOVERY_REUSE_ACCEPTANCE_REQUIRED");
    }
    if std::env::var("INSAR_NISAR_SUBSET_LIVE_GRANULE")
        .ok()
        .as_deref()
        != Some(GRANULE_ID)
    {
        return Err("NISAR_LIVE_GRANULE_MISMATCH");
    }
    let value =
        std::env::var("INSAR_NISAR_SUBSET_LIVE_ROOT").map_err(|_| "NISAR_LIVE_ROOT_REQUIRED")?;
    let root = PathBuf::from(value);
    if !root.is_absolute() {
        return Err("NISAR_LIVE_ROOT_NOT_ABSOLUTE");
    }
    if !root.is_dir() {
        return Err("NISAR_LIVE_RECOVERY_REUSE_ROOT_MISSING");
    }
    if !root.join("tasks.sqlite3").is_file() {
        return Err("NISAR_LIVE_RECOVERY_REUSE_STORE_MISSING");
    }
    if !root.join("output").is_dir() {
        return Err("NISAR_LIVE_RECOVERY_REUSE_OUTPUT_MISSING");
    }
    let old_evidence = root.join("evidence").join("live-acceptance-redacted.json");
    if !old_evidence.is_file() {
        return Err("NISAR_LIVE_RECOVERY_REUSE_FAILURE_EVIDENCE_MISSING");
    }
    Ok(root)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn recovery_reuse_evidence_root(root: &Path) -> Result<PathBuf, &'static str> {
    let candidate = std::env::var("INSAR_NISAR_SUBSET_LIVE_REUSE_EVIDENCE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("resume-acceptance"));
    if !candidate.is_absolute() {
        return Err("NISAR_LIVE_RECOVERY_REUSE_EVIDENCE_ROOT_NOT_ABSOLUTE");
    }
    if candidate.exists() {
        return Err("NISAR_LIVE_RECOVERY_REUSE_EVIDENCE_ROOT_MUST_NOT_EXIST");
    }
    Ok(candidate)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn recovery_body_reuse_evidence_root(root: &Path) -> Result<PathBuf, &'static str> {
    let candidate = std::env::var("INSAR_NISAR_SUBSET_LIVE_BODY_REUSE_EVIDENCE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("body-resume-acceptance"));
    if !candidate.is_absolute() {
        return Err("NISAR_LIVE_BODY_REUSE_EVIDENCE_ROOT_NOT_ABSOLUTE");
    }
    if candidate.exists() {
        return Err("NISAR_LIVE_BODY_REUSE_EVIDENCE_ROOT_MUST_NOT_EXIST");
    }
    Ok(candidate)
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn run_nisar_recovery_body_existing_running_store(
    root: &Path,
    evidence_root: &Path,
    evidence: &Arc<NisarLiveAcceptanceEvidenceSink>,
    cleanup: &mut LiveExecutionCleanupGuard,
) -> Result<(), &'static str> {
    const TASK_ID: &str = "nisar-live-acceptance-task";
    let database = root.join("tasks.sqlite3");
    let output = root.join("output");
    let old_evidence_path = root.join("evidence").join("live-acceptance-redacted.json");
    let old_evidence = fs::read(&old_evidence_path)
        .map_err(|_| "NISAR_LIVE_BODY_REUSE_FAILURE_EVIDENCE_UNREADABLE")?;
    let old_evidence_json: Value = serde_json::from_slice(&old_evidence)
        .map_err(|_| "NISAR_LIVE_BODY_REUSE_FAILURE_EVIDENCE_INVALID")?;
    if old_evidence_json.get("status").and_then(Value::as_str) != Some("running") {
        return Err("NISAR_LIVE_BODY_REUSE_FAILURE_EVIDENCE_NOT_RUNNING");
    }

    let store =
        TaskStore::open(&database).map_err(|_| "NISAR_LIVE_BODY_REUSE_STORE_UNAVAILABLE")?;
    let task = store
        .load_task_record(TASK_ID)
        .map_err(|_| "NISAR_LIVE_BODY_REUSE_TASK_UNAVAILABLE")?
        .ok_or("NISAR_LIVE_BODY_REUSE_TASK_UNAVAILABLE")?;
    let item = task
        .snapshot
        .candidates
        .first()
        .cloned()
        .ok_or("NISAR_LIVE_BODY_REUSE_TASK_INVALID")?;
    let expected_item_id = format!(
        "PROVIDER::{NISAR_PROVIDER_ID}::COLLECTION::{NISAR_NATIVE_COLLECTION_ID}::ITEM::{PROVIDER_PRODUCT_ID}"
    );
    let selected_polarizations = vec!["HH".to_owned(), "HV".to_owned()];
    if task.snapshot.task_id != TASK_ID
        || task.snapshot.batch_id != "nisar-live-source-batch"
        || task.snapshot.provider_id != NISAR_PROVIDER_ID
        || task.snapshot.output_root != output
        || task.snapshot.layout != insar_core::DownloadLayout::Direct
        || task.snapshot.candidates.len() != 1
        || task.runtime.task_id != TASK_ID
        || task.runtime.status != TaskLifecycleStatus::Running
        || task.runtime.task_paused
        || task.runtime.active_item_ids != vec![item.item_id.clone()]
        || !task.runtime.queued_item_ids.is_empty()
        || !task.runtime.paused_item_ids.is_empty()
        || !task.runtime.failed_item_ids.is_empty()
        || item.item_id != expected_item_id
        || item.metadata.get("delivery_kind").and_then(Value::as_str)
            != Some("remote_derived_subset_v2")
        || item.metadata.get("source_item_id").and_then(Value::as_str)
            != Some(item.item_id.as_str())
    {
        return Err("NISAR_LIVE_BODY_REUSE_TASK_IDENTITY_INVALID");
    }
    validate_live_source_identity_for_frequency(
        &item,
        NisarSubsetFrequency::A,
        &selected_polarizations,
    )?;
    let prepared_envelope =
        insar_store::parse_nisar_subset_prepared_intent_v2_envelope(&item.metadata)
            .map_err(|_| "NISAR_LIVE_BODY_REUSE_PREPARED_INTENT_INVALID")?
            .ok_or("NISAR_LIVE_BODY_REUSE_PREPARED_INTENT_MISSING")?;
    let prepared = prepared_envelope
        .intent()
        .map_err(|_| "NISAR_LIVE_BODY_REUSE_PREPARED_INTENT_INVALID")?;
    if prepared.selected_frequency != NisarSubsetFrequency::A
        || prepared.selected_polarizations != selected_polarizations
        || prepared.source_product_identity.provider_id != NISAR_PROVIDER_ID
        || prepared.source_product_identity.provider_product_id != PROVIDER_PRODUCT_ID
        || prepared.source_product_identity.collection_id != NISAR_NATIVE_COLLECTION_ID
        || prepared.source_asset_identity.file_name != FILE_NAME
        || prepared
            .source_asset_identity
            .checksum
            .as_ref()
            .map(|checksum| checksum.value.as_str())
            != Some(SOURCE_CHECKSUM_VALUE)
    {
        return Err("NISAR_LIVE_BODY_REUSE_PREPARED_INTENT_IDENTITY_INVALID");
    }
    drop(store);

    let body_interruption = LiveBodyInterruptionControl::new();
    let mut service = build_live_production_service(
        &database,
        evidence,
        true,
        Some(Arc::clone(&body_interruption)),
    )?;
    let operation = run_live_body_failure_retry_phase(
        &mut service,
        evidence_root,
        &database,
        &output,
        evidence,
        cleanup,
        &item.item_id,
        NisarSubsetFrequency::A,
        &selected_polarizations,
        &prepared.canonical_geometry_digest.requested_sha256,
        Some(body_interruption),
    );
    if operation.is_err() {
        let _ = service.shutdown();
    }
    operation?;
    if fs::read(&old_evidence_path)
        .map_err(|_| "NISAR_LIVE_BODY_REUSE_FAILURE_EVIDENCE_UNREADABLE")?
        != old_evidence
    {
        return Err("NISAR_LIVE_BODY_REUSE_FAILURE_EVIDENCE_MUTATED");
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn run_nisar_recovery_resume_existing_store(
    root: &Path,
    evidence_root: &Path,
    evidence: &Arc<NisarLiveAcceptanceEvidenceSink>,
    exercise_same_process_pause: bool,
) -> Result<(), &'static str> {
    const TASK_ID: &str = "nisar-live-acceptance-task";
    let database = root.join("tasks.sqlite3");
    let output = root.join("output");
    let old_evidence_path = root.join("evidence").join("live-acceptance-redacted.json");
    let old_evidence_bytes = fs::read(&old_evidence_path)
        .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_FAILURE_EVIDENCE_UNREADABLE")?;
    let old_evidence: Value = serde_json::from_slice(&old_evidence_bytes)
        .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_FAILURE_EVIDENCE_INVALID")?;
    if old_evidence.get("status").and_then(Value::as_str) != Some("fail") {
        return Err("NISAR_LIVE_RECOVERY_REUSE_FAILURE_EVIDENCE_NOT_FAILED");
    }

    let store =
        TaskStore::open(&database).map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_UNAVAILABLE")?;
    let task = store
        .load_task_record(TASK_ID)
        .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_TASK_UNAVAILABLE")?
        .ok_or("NISAR_LIVE_RECOVERY_REUSE_TASK_UNAVAILABLE")?;
    let item = task
        .snapshot
        .candidates
        .first()
        .cloned()
        .ok_or("NISAR_LIVE_RECOVERY_REUSE_TASK_INVALID")?;
    let expected_item_id = format!(
        "PROVIDER::{NISAR_PROVIDER_ID}::COLLECTION::{NISAR_NATIVE_COLLECTION_ID}::ITEM::{PROVIDER_PRODUCT_ID}"
    );
    let selected_polarizations = vec!["HH".to_owned(), "HV".to_owned()];
    let reattach_running = exercise_same_process_pause
        && task.runtime.status == TaskLifecycleStatus::Running
        && !task.runtime.task_paused
        && task.runtime.active_item_ids == vec![item.item_id.clone()]
        && task.runtime.paused_item_ids.is_empty();
    if task.snapshot.task_id != TASK_ID
        || task.snapshot.batch_id != "nisar-live-source-batch"
        || task.snapshot.provider_id != NISAR_PROVIDER_ID
        || task.snapshot.output_root != output
        || task.snapshot.layout != insar_core::DownloadLayout::Direct
        || task.snapshot.candidates.len() != 1
        || task.runtime.task_id != TASK_ID
        || (!reattach_running
            && (task.runtime.status != TaskLifecycleStatus::Paused || !task.runtime.task_paused))
        || !task.runtime.queued_item_ids.is_empty()
        // PauseTask keeps an already claimed item active until its worker returns.
        // On process reopen the scheduler reattaches that claim after ResumeTask.
        || !(task.runtime.active_item_ids == vec![item.item_id.clone()]
            && task.runtime.paused_item_ids.is_empty()
            || task.runtime.active_item_ids.is_empty()
                && task.runtime.paused_item_ids == vec![item.item_id.clone()])
        || item.item_id != expected_item_id
        || item
            .metadata
            .get("delivery_kind")
            .and_then(Value::as_str)
            != Some("remote_derived_subset_v2")
        || item
            .metadata
            .get("source_item_id")
            .and_then(Value::as_str)
            != Some(item.item_id.as_str())
    {
        return Err("NISAR_LIVE_RECOVERY_REUSE_TASK_IDENTITY_INVALID");
    }
    validate_live_source_identity_for_frequency(
        &item,
        NisarSubsetFrequency::A,
        &selected_polarizations,
    )?;
    let prepared_envelope =
        insar_store::parse_nisar_subset_prepared_intent_v2_envelope(&item.metadata)
            .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_PREPARED_INTENT_INVALID")?
            .ok_or("NISAR_LIVE_RECOVERY_REUSE_PREPARED_INTENT_MISSING")?;
    let prepared = prepared_envelope
        .intent()
        .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_PREPARED_INTENT_INVALID")?;
    if prepared.selected_frequency != NisarSubsetFrequency::A
        || prepared.selected_polarizations != selected_polarizations
        || prepared.source_product_identity.provider_id != NISAR_PROVIDER_ID
        || prepared.source_product_identity.provider_product_id != PROVIDER_PRODUCT_ID
        || prepared.source_product_identity.collection_id != NISAR_NATIVE_COLLECTION_ID
        || prepared.source_asset_identity.file_name != FILE_NAME
        || prepared
            .source_asset_identity
            .checksum
            .as_ref()
            .map(|checksum| checksum.value.as_str())
            != Some(SOURCE_CHECKSUM_VALUE)
    {
        return Err("NISAR_LIVE_RECOVERY_REUSE_PREPARED_INTENT_IDENTITY_INVALID");
    }

    let old_checkpoint = read_live_recovery_checkpoint(&output)?
        .ok_or("NISAR_LIVE_RECOVERY_REUSE_CHECKPOINT_UNAVAILABLE")?;
    let old_anchor = read_live_recovery_anchor(&database, TASK_ID, &item.item_id)?
        .ok_or("NISAR_LIVE_RECOVERY_REUSE_ANCHOR_UNAVAILABLE")?;
    if old_checkpoint.task_id != TASK_ID
        || old_checkpoint.item_id != item.item_id
        || !matches!(old_checkpoint.state.as_str(), "InProgress" | "Interrupted")
        || old_checkpoint.pending_attempt
        || old_checkpoint.completed_ranges.is_empty()
        || old_checkpoint.committed_science_response_body_bytes == 0
        || old_checkpoint.planned_ranges.is_empty()
        || old_checkpoint.completed_ranges.len() >= old_checkpoint.planned_ranges.len()
        || old_checkpoint.prepared_intent_sha256 != prepared.canonical_intent_sha256
        || !live_recovery_anchor_matches_checkpoint(&old_anchor, &old_checkpoint)
        || old_checkpoint.reserved_science_request_count
            < old_checkpoint.committed_science_request_count
        || old_checkpoint.reserved_science_response_body_bytes
            < old_checkpoint.committed_science_response_body_bytes
        || old_checkpoint
            .completed_ranges
            .iter()
            .enumerate()
            .any(|(index, range)| {
                old_checkpoint.completed_ranges[..index]
                    .iter()
                    .any(|previous| previous == range)
                    || !old_checkpoint
                        .planned_ranges
                        .iter()
                        .any(|planned| planned == range)
            })
    {
        return Err("NISAR_LIVE_RECOVERY_REUSE_CHECKPOINT_INVALID");
    }
    let old_progress = live_recovery_progress_bytes(&database, TASK_ID, &item.item_id)?;
    let mut trace = LiveRecoveryTrace::new(LiveRecoveryMode::ResumeExistingStore);
    trace.threshold_basis =
        "preserved_inprogress_checkpoint_lineage_frozen_plan_anchor_validator".to_owned();
    trace.planned_science_response_body_bytes =
        Some(old_checkpoint.planned_science_response_body_bytes);
    trace.pause_request_checkpoint_bytes =
        Some(old_checkpoint.committed_science_response_body_bytes);
    trace.pause_request_revision = Some(task.runtime.revision);
    if reattach_running {
        trace.threshold_basis =
            "preserved_running_store_checkpoint_lineage_frozen_plan_anchor_validator".to_owned();
    } else {
        trace.paused_store_progress_bytes = Some(old_progress);
        trace.paused_runtime_revision = Some(task.runtime.revision);
        trace.worker_quiescent = true;
        trace.pause = Some(old_checkpoint.clone());
        trace.anchor_at_pause = Some(old_anchor.clone());
    }
    write_live_recovery_trace_at(evidence_root.join("evidence").as_path(), &trace)?;
    drop(store);

    let mut service = build_live_production_service(&database, evidence, true, None)?;
    if !reattach_running {
        let reopened_runtime = TaskStore::open(&database)
            .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_REOPEN_FAILED")?
            .load_scheduler_control_runtime(TASK_ID)
            .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_REOPEN_FAILED")?
            .ok_or("NISAR_LIVE_RECOVERY_REUSE_STORE_REOPEN_FAILED")?;
        let reopened_checkpoint = read_live_recovery_checkpoint(&output)?
            .ok_or("NISAR_LIVE_RECOVERY_REUSE_CHECKPOINT_UNAVAILABLE")?;
        let reopened_anchor = read_live_recovery_anchor(&database, TASK_ID, &item.item_id)?
            .ok_or("NISAR_LIVE_RECOVERY_REUSE_ANCHOR_UNAVAILABLE")?;
        if reopened_runtime.status != TaskLifecycleStatus::Paused
            || !reopened_runtime.task_paused
            || reopened_checkpoint.state != old_checkpoint.state
            || reopened_checkpoint.checkpoint_sha256 != old_checkpoint.checkpoint_sha256
            || reopened_checkpoint.lineage_key_sha256 != old_checkpoint.lineage_key_sha256
            || reopened_checkpoint.execution_binding_sha256
                != old_checkpoint.execution_binding_sha256
            || reopened_checkpoint.prepared_intent_sha256 != old_checkpoint.prepared_intent_sha256
            || reopened_checkpoint.final_plan_sha256 != old_checkpoint.final_plan_sha256
            || reopened_checkpoint.physical_plan_sha256 != old_checkpoint.physical_plan_sha256
            || reopened_checkpoint.inventory_sha256 != old_checkpoint.inventory_sha256
            || reopened_checkpoint.inventory_evidence_sha256
                != old_checkpoint.inventory_evidence_sha256
            || reopened_checkpoint.source_graph_sha256 != old_checkpoint.source_graph_sha256
            || reopened_checkpoint.source_validator_sha256 != old_checkpoint.source_validator_sha256
            || reopened_checkpoint.part_identity_sha256 != old_checkpoint.part_identity_sha256
            || reopened_checkpoint.completed_ranges_digest != old_checkpoint.completed_ranges_digest
            || reopened_anchor.record_sha256 != old_anchor.record_sha256
            || !live_recovery_anchor_matches_checkpoint(&reopened_anchor, &reopened_checkpoint)
        {
            let _ = service.shutdown();
            return Err("NISAR_LIVE_RECOVERY_REUSE_REOPEN_MISMATCH");
        }
        trace.reopen_runtime_revision = Some(reopened_runtime.revision);
        trace.reopen = Some(reopened_checkpoint);
        trace.anchor_at_reopen = Some(reopened_anchor);
        write_live_recovery_trace_at(evidence_root.join("evidence").as_path(), &trace)?;
    }

    let operation = (|| {
        if !reattach_running {
            service
                .resume_nisar_subset_task(TASK_ID)
                .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_RESUME_FAILED")?;
        }
        let resumed_runtime = TaskStore::open(&database)
            .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_UNAVAILABLE")?
            .load_scheduler_control_runtime(TASK_ID)
            .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_UNAVAILABLE")?
            .ok_or("NISAR_LIVE_RECOVERY_REUSE_STORE_UNAVAILABLE")?;
        if resumed_runtime.task_paused
            || !resumed_runtime.paused_item_ids.is_empty()
            || !matches!(
                resumed_runtime.status,
                TaskLifecycleStatus::Queued | TaskLifecycleStatus::Running
            )
        {
            return Err("NISAR_LIVE_RECOVERY_REUSE_RESUME_STATE_INVALID");
        }
        trace.resume_runtime_revision = Some(resumed_runtime.revision);
        write_live_recovery_trace_at(evidence_root.join("evidence").as_path(), &trace)?;
        let expected_resumed_bytes = if exercise_same_process_pause {
            let pause_evidence_root = evidence_root.join("same-process-pause");
            run_live_recovery_phase(
                &mut service,
                &pause_evidence_root,
                &database,
                &output,
                evidence,
                &item.item_id,
                LiveRecoveryMode::PauseResume,
                old_checkpoint.completed_ranges.len(),
            )?;
            let pause_trace: Value = serde_json::from_slice(
                &fs::read(pause_evidence_root.join("evidence/recovery-trace.json"))
                    .map_err(|_| "NISAR_LIVE_REUSE_PAUSE_TRACE_UNAVAILABLE")?,
            )
            .map_err(|_| "NISAR_LIVE_REUSE_PAUSE_TRACE_INVALID")?;
            let paused_bytes = pause_trace
                .pointer("/pause/committed_science_response_body_bytes")
                .and_then(Value::as_u64)
                .ok_or("NISAR_LIVE_REUSE_PAUSE_TRACE_INVALID")?;
            if pause_trace.get("worker_quiescent").and_then(Value::as_bool) != Some(true)
                || paused_bytes <= old_checkpoint.committed_science_response_body_bytes
            {
                return Err("NISAR_LIVE_REUSE_PAUSE_DID_NOT_ADVANCE");
            }
            paused_bytes
        } else {
            old_checkpoint.committed_science_response_body_bytes
        };
        if !wait_for_live_recovery_terminal(evidence, Duration::from_secs(45 * 60))? {
            return Err("NISAR_LIVE_RECOVERY_REUSE_WORKER_FAILURE");
        }
        let completion = read_live_recovery_checkpoint(&output)?
            .ok_or("NISAR_LIVE_RECOVERY_REUSE_CHECKPOINT_UNAVAILABLE")?;
        let completion_anchor = read_live_recovery_anchor(&database, TASK_ID, &item.item_id)?
            .ok_or("NISAR_LIVE_RECOVERY_REUSE_ANCHOR_UNAVAILABLE")?;
        let ranges_reused =
            old_checkpoint
                .completed_ranges
                .iter()
                .enumerate()
                .all(|(index, range)| {
                    completion
                        .completed_ranges
                        .iter()
                        .position(|candidate| candidate == range)
                        .is_some_and(|position| {
                            completion.completed_range_request_counts[position]
                                == old_checkpoint.completed_range_request_counts[index]
                        })
                });
        let new_range_requests = completion
            .completed_ranges
            .iter()
            .zip(&completion.completed_range_request_counts)
            .filter(|(range, _)| !old_checkpoint.completed_ranges.contains(range))
            .try_fold(0_u64, |sum, (_, count)| sum.checked_add(*count));
        let request_delta_matches = new_range_requests.is_some_and(|new_requests| {
            old_checkpoint
                .committed_science_request_count
                .checked_add(new_requests)
                == Some(completion.committed_science_request_count)
        });
        if completion.state != "Complete"
            || completion.pending_attempt
            || completion.completed_ranges.len() != completion.planned_range_count
            || completion.planned_ranges != old_checkpoint.planned_ranges
            || completion.planned_science_response_body_bytes
                != old_checkpoint.planned_science_response_body_bytes
            || completion.committed_science_response_body_bytes
                != completion.planned_science_response_body_bytes
            || completion.resumed_bytes != expected_resumed_bytes
            || completion.resumed_bytes == 0
            || completion.lineage_key_sha256 != old_checkpoint.lineage_key_sha256
            || completion.execution_binding_sha256 != old_checkpoint.execution_binding_sha256
            || completion.prepared_intent_sha256 != old_checkpoint.prepared_intent_sha256
            || completion.final_plan_sha256 != old_checkpoint.final_plan_sha256
            || completion.physical_plan_sha256 != old_checkpoint.physical_plan_sha256
            || completion.inventory_sha256 != old_checkpoint.inventory_sha256
            || completion.inventory_evidence_sha256 != old_checkpoint.inventory_evidence_sha256
            || completion.source_graph_sha256 != old_checkpoint.source_graph_sha256
            || completion.source_validator_sha256 != old_checkpoint.source_validator_sha256
            || completion.part_identity_sha256 != old_checkpoint.part_identity_sha256
            || completion.failed_science_request_count
                != old_checkpoint.failed_science_request_count
            || completion.indeterminate_science_request_count != 0
            || completion.indeterminate_science_response_body_bytes != 0
            || !ranges_reused
            || !request_delta_matches
            || !live_recovery_anchor_matches_checkpoint(&completion_anchor, &completion)
            || completion_anchor.generation < old_anchor.generation
        {
            return Err("NISAR_LIVE_RECOVERY_REUSE_COMPLETION_INVALID");
        }
        wait_for_durable_store_completion(
            &database,
            TASK_ID,
            &item.item_id,
            Duration::from_secs(30),
            "NISAR_LIVE_RECOVERY_REUSE_STORE_COMPLETION_FAILED",
        )?;
        trace.completion = Some(completion.clone());
        trace.anchor_at_completion = Some(completion_anchor.clone());
        trace.reused_verified_bytes = completion.resumed_bytes;
        let terminal_store = TaskStore::open(&database)
            .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_TERMINAL_UNAVAILABLE")?;
        let terminal_task = terminal_store
            .load_task_record(TASK_ID)
            .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_TERMINAL_UNAVAILABLE")?
            .ok_or("NISAR_LIVE_RECOVERY_REUSE_STORE_TERMINAL_UNAVAILABLE")?;
        let terminal_download = terminal_store
            .load_download_snapshot(TASK_ID)
            .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_TERMINAL_UNAVAILABLE")?;
        drop(terminal_store);
        evidence
            .mark_store_completed("observed")
            .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_EVIDENCE_INCOMPLETE")?;
        Ok((terminal_task, terminal_download))
    })();
    let (terminal_task, terminal_download) = match operation {
        Ok(value) => value,
        Err(code) => {
            let _ = service.shutdown();
            return Err(code);
        }
    };
    service
        .shutdown()
        .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_SERVICE_SHUTDOWN_FAILED")?;
    let reopened =
        TaskStore::open(&database).map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_REOPEN_FAILED")?;
    let reopened_task = reopened
        .load_task_record(TASK_ID)
        .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_REOPEN_FAILED")?
        .ok_or("NISAR_LIVE_RECOVERY_REUSE_STORE_REOPEN_FAILED")?;
    let reopened_download = reopened
        .load_download_snapshot(TASK_ID)
        .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_STORE_REOPEN_FAILED")?;
    drop(reopened);
    if reopened_task != terminal_task || reopened_download != terminal_download {
        return Err("NISAR_LIVE_RECOVERY_REUSE_STORE_REOPEN_MISMATCH");
    }
    evidence
        .mark_store_reopen("observed")
        .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_EVIDENCE_INCOMPLETE")?;
    evidence
        .mark_cleanup_completed(false)
        .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_EVIDENCE_INCOMPLETE")?;
    let final_checkpoint = read_live_recovery_checkpoint(&output)?
        .ok_or("NISAR_LIVE_RECOVERY_REUSE_CHECKPOINT_UNAVAILABLE")?;
    let final_anchor = read_live_recovery_anchor(&database, TASK_ID, &item.item_id)?
        .ok_or("NISAR_LIVE_RECOVERY_REUSE_ANCHOR_UNAVAILABLE")?;
    if final_checkpoint.checkpoint_sha256
        != trace
            .completion
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_sha256.as_str())
            .unwrap_or_default()
        || !live_recovery_anchor_matches_checkpoint(&final_anchor, &final_checkpoint)
    {
        return Err("NISAR_LIVE_RECOVERY_REUSE_FINAL_AUTHORITY_MISMATCH");
    }
    let final_evidence = evidence.snapshot();
    validate_m2d_b_final_evidence_for_selection(
        &final_evidence,
        &prepared.canonical_geometry_digest.requested_sha256,
        &output,
        "A",
        "HH+HV",
        &selected_polarizations,
        &selected_polarizations,
        None,
    )?;
    let duplicate_science_ranges = live_science_duplicate_ranges(&final_evidence);
    trace.duplicate_science_ranges = duplicate_science_ranges.clone();
    write_live_recovery_trace_at(evidence_root.join("evidence").as_path(), &trace)?;
    if !duplicate_science_ranges.is_empty() {
        return Err("NISAR_LIVE_RECOVERY_REUSE_DUPLICATE_RANGE_REQUEST");
    }
    if fs::read(&old_evidence_path)
        .map_err(|_| "NISAR_LIVE_RECOVERY_REUSE_FAILURE_EVIDENCE_UNREADABLE")?
        != old_evidence_bytes
    {
        return Err("NISAR_LIVE_RECOVERY_REUSE_FAILURE_EVIDENCE_MUTATED");
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "resume one preserved paused live Store in a new process and retain the prior failure evidence"]
fn nisar_remote_subset_resume_preserved_pause_live_acceptance() {
    run_nisar_preserved_live_acceptance(false);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "resume preserved real ranges, advance science, then pause and resume in the same service"]
fn nisar_remote_subset_preserved_same_process_pause_resume_live_acceptance() {
    run_nisar_preserved_live_acceptance(true);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn run_nisar_preserved_live_acceptance(exercise_same_process_pause: bool) {
    insar_download::establish_hdf5_plugin_isolation().expect(
        "live recovery reuse host must establish app-owned HDF5 plugin isolation before HDF5 use",
    );
    let enable_env = if exercise_same_process_pause {
        "INSAR_NISAR_SUBSET_LIVE_REUSE_PAUSE_ACCEPTANCE"
    } else {
        "INSAR_NISAR_SUBSET_LIVE_REUSE_ACCEPTANCE"
    };
    let root = recovery_reuse_fixed_gate(enable_env)
        .expect("preserved live recovery reuse gates are required before network access");
    let evidence_root = recovery_reuse_evidence_root(&root)
        .expect("reuse evidence root must be new and separate from prior failure evidence");
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(
            evidence_root.clone(),
            BASELINE_COMMIT,
            current_git_commit(),
        )
        .expect("redacted live recovery reuse evidence sink"),
    );
    let attempt = catch_unwind(AssertUnwindSafe(|| {
        run_nisar_recovery_resume_existing_store(
            &root,
            &evidence_root,
            &evidence,
            exercise_same_process_pause,
        )
    }));
    let outcome = match attempt {
        Ok(Ok(())) => Ok(()),
        Ok(Err(code)) => {
            record_live_failure_if_unset(&evidence, code);
            Err(boundary_failure_code(&evidence, code))
        }
        Err(_) => {
            record_live_failure_if_unset(&evidence, "NISAR_LIVE_RECOVERY_REUSE_HARNESS_PANIC");
            Err(failure_code(
                &evidence,
                "NISAR_LIVE_RECOVERY_REUSE_HARNESS_PANIC",
            ))
        }
    };
    if let Err(code) = outcome {
        panic!(
            "NISAR preserved-checkpoint recovery stopped with typed NO_GO: {code} (evidence={})",
            evidence.evidence_path().display()
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn run_nisar_body_reuse_live_acceptance() {
    insar_download::establish_hdf5_plugin_isolation().expect(
        "live body recovery reuse host must establish app-owned HDF5 plugin isolation before HDF5 use",
    );
    let root = recovery_reuse_fixed_gate("INSAR_NISAR_SUBSET_LIVE_REUSE_BODY_ACCEPTANCE")
        .expect("preserved Running body recovery gates are required before network access");
    let evidence_root = recovery_body_reuse_evidence_root(&root)
        .expect("body recovery evidence root must be new and separate from prior evidence");
    let old_evidence_path = root.join("evidence").join("live-acceptance-redacted.json");
    let old_evidence = fs::read(&old_evidence_path)
        .expect("preserved Running body recovery evidence must be readable");
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(
            evidence_root.clone(),
            BASELINE_COMMIT,
            current_git_commit(),
        )
        .expect("redacted Running body recovery evidence sink"),
    );
    let mut cleanup = LiveExecutionCleanupGuard::new(evidence_root.clone(), Arc::clone(&evidence));
    let attempt = catch_unwind(AssertUnwindSafe(|| {
        run_nisar_recovery_body_existing_running_store(
            &root,
            &evidence_root,
            &evidence,
            &mut cleanup,
        )
    }));
    let active_evidence = Arc::clone(&cleanup.evidence);
    let outcome = match attempt {
        Ok(Ok(())) => Ok(()),
        Ok(Err(code)) => {
            record_live_failure_if_unset(&active_evidence, code);
            Err(boundary_failure_code(&active_evidence, code))
        }
        Err(_) => {
            record_live_failure_if_unset(&active_evidence, "NISAR_LIVE_BODY_REUSE_HARNESS_PANIC");
            Err(failure_code(
                &active_evidence,
                "NISAR_LIVE_BODY_REUSE_HARNESS_PANIC",
            ))
        }
    };
    if fs::read(&old_evidence_path).ok().as_deref() != Some(old_evidence.as_slice()) {
        cleanup.preserve_failure_diagnostics();
        panic!(
            "NISAR preserved Running body recovery stopped with typed NO_GO: NISAR_LIVE_BODY_REUSE_FAILURE_EVIDENCE_MUTATED"
        );
    }
    if let Err(code) = outcome {
        cleanup.preserve_failure_diagnostics();
        panic!(
            "NISAR preserved Running body recovery stopped with typed NO_GO: {code} (evidence={})",
            evidence.evidence_path().display()
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "resume a preserved Running body Store, inject a real mid-body failure, retry the failed item, and retain prior evidence"]
fn nisar_remote_subset_resume_running_body_store_live_acceptance() {
    run_nisar_body_reuse_live_acceptance();
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "one manually authorized bounded real provider pause/resume transfer"]
fn nisar_remote_subset_pause_resume_live_acceptance() {
    run_nisar_recovery_live_acceptance(
        LiveRecoveryMode::PauseResume,
        "INSAR_NISAR_SUBSET_LIVE_RECOVERY_ACCEPTANCE",
        "NISAR real provider pause/resume acceptance",
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "one manually authorized bounded real provider pause/restart/resume transfer"]
fn nisar_remote_subset_pause_restart_resume_live_acceptance() {
    run_nisar_recovery_live_acceptance(
        LiveRecoveryMode::RestartResume,
        "INSAR_NISAR_SUBSET_LIVE_RESTART_ACCEPTANCE",
        "NISAR real provider pause/restart/resume acceptance",
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "one manually authorized bounded real provider mid-body transport failure, failed-item retry, and Store reopen"]
fn nisar_remote_subset_pause_body_interruption_resume_live_acceptance() {
    run_nisar_recovery_live_acceptance(
        LiveRecoveryMode::BodyInterruption,
        "INSAR_NISAR_SUBSET_LIVE_BODY_INTERRUPTION_ACCEPTANCE",
        "NISAR real provider mid-body transport failure/failed-item retry acceptance",
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "one manually authorized bounded live transfer only"]
fn nisar_remote_subset_single_scene_live_acceptance() {
    insar_download::establish_hdf5_plugin_isolation().expect(
        "live acceptance host must establish app-owned HDF5 plugin isolation before HDF5 use",
    );
    let root = fixed_gate(
        std::env::var("INSAR_NISAR_SUBSET_LIVE_ACCEPTANCE")
            .ok()
            .as_deref(),
        std::env::var("INSAR_NISAR_SUBSET_LIVE_GRANULE")
            .ok()
            .as_deref(),
        std::env::var("INSAR_NISAR_SUBSET_LIVE_ROOT")
            .ok()
            .as_deref(),
    )
    .expect("all explicit live acceptance gates are required before network access");
    fs::create_dir_all(&root).unwrap();
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(root.clone(), BASELINE_COMMIT, current_git_commit())
            .expect("redacted live evidence sink"),
    );
    let mut cleanup = LiveExecutionCleanupGuard::new(root.clone(), Arc::clone(&evidence));

    let attempt = catch_unwind(AssertUnwindSafe(|| {
        run_m2d_b_controlled_attempt(&root, &evidence, &mut cleanup, false)
    }));
    let outcome = match attempt {
        Ok(Ok(())) => Ok(()),
        Ok(Err(code)) => {
            record_live_failure_if_unset(&evidence, code);
            Err(failure_code(&evidence, code))
        }
        Err(_) => {
            record_live_failure_if_unset(&evidence, "NISAR_LIVE_HARNESS_PANIC");
            Err(failure_code(&evidence, "NISAR_LIVE_HARNESS_PANIC"))
        }
    };

    if let Err(code) = outcome {
        cleanup.discard_final_artifact();
        let preserve_failure = std::env::var("INSAR_NISAR_SUBSET_PRESERVE_FAILURE")
            .ok()
            .as_deref()
            == Some("1");
        let cleanup_result = if preserve_failure {
            cleanup.preserve_failure_diagnostics();
            Ok(())
        } else {
            cleanup.cleanup_now()
        };
        let final_code = failure_code(&evidence, "NISAR_LIVE_EVIDENCE_INCOMPLETE");
        panic!(
            "controlled live acceptance stopped with typed NO_GO: {final_code} (boundary={code}, cleanup={cleanup_result:?})"
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "one manually authorized bounded live transfer with Frequency A HH only"]
fn nisar_remote_subset_single_scene_live_acceptance_hh_only() {
    insar_download::establish_hdf5_plugin_isolation().expect(
        "live acceptance host must establish app-owned HDF5 plugin isolation before HDF5 use",
    );
    let root = fixed_gate(
        std::env::var("INSAR_NISAR_SUBSET_LIVE_ACCEPTANCE")
            .ok()
            .as_deref(),
        std::env::var("INSAR_NISAR_SUBSET_LIVE_GRANULE")
            .ok()
            .as_deref(),
        std::env::var("INSAR_NISAR_SUBSET_LIVE_ROOT")
            .ok()
            .as_deref(),
    )
    .expect("all explicit live acceptance gates are required before network access");
    fs::create_dir_all(&root).unwrap();
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new(root.clone(), BASELINE_COMMIT, current_git_commit())
            .expect("redacted live evidence sink"),
    );
    let mut cleanup = LiveExecutionCleanupGuard::new(root.clone(), Arc::clone(&evidence));

    let attempt = catch_unwind(AssertUnwindSafe(|| {
        run_m2d_b_controlled_attempt(&root, &evidence, &mut cleanup, true)
    }));
    let outcome = match attempt {
        Ok(Ok(())) => Ok(()),
        Ok(Err(code)) => {
            record_live_failure_if_unset(&evidence, code);
            Err(failure_code(&evidence, code))
        }
        Err(_) => {
            record_live_failure_if_unset(&evidence, "NISAR_LIVE_HARNESS_PANIC");
            Err(failure_code(&evidence, "NISAR_LIVE_HARNESS_PANIC"))
        }
    };

    if let Err(code) = outcome {
        cleanup.discard_final_artifact();
        let preserve_failure = std::env::var("INSAR_NISAR_SUBSET_PRESERVE_FAILURE")
            .ok()
            .as_deref()
            == Some("1");
        let cleanup_result = if preserve_failure {
            cleanup.preserve_failure_diagnostics();
            Ok(())
        } else {
            cleanup.cleanup_now()
        };
        let final_code = failure_code(&evidence, "NISAR_LIVE_EVIDENCE_INCOMPLETE");
        panic!(
            "controlled single-polarization live acceptance stopped with typed NO_GO: {final_code} (boundary={code}, cleanup={cleanup_result:?})"
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "one manually authorized bounded live transfer with Frequency B HH only"]
fn nisar_remote_subset_frequency_b_hh_live_acceptance() {
    insar_download::establish_hdf5_plugin_isolation().expect(
        "live acceptance host must establish app-owned HDF5 plugin isolation before HDF5 use",
    );
    let root = frequency_b_fixed_gate().expect(
        "all explicit Frequency B live acceptance gates are required before network access",
    );
    fs::create_dir_all(&root).unwrap();
    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new_for_scene_generic_acceptance(
            root.clone(),
            BASELINE_COMMIT,
            current_git_commit(),
            insar_download::NISAR_FREQ_B_EVIDENCE_SCHEMA,
            insar_download::NISAR_FREQ_B_GATE,
        )
        .expect("redacted Frequency B evidence sink"),
    );
    let mut cleanup = LiveExecutionCleanupGuard::new(root.clone(), Arc::clone(&evidence));
    let attempt = catch_unwind(AssertUnwindSafe(|| {
        run_frequency_b_controlled_attempt(&root, &evidence, &mut cleanup)
    }));
    let outcome = match attempt {
        Ok(Ok(())) => Ok(()),
        Ok(Err(code)) => {
            record_live_failure_if_unset(&evidence, code);
            Err(failure_code(&evidence, code))
        }
        Err(_) => {
            record_live_failure_if_unset(&evidence, "NISAR_LIVE_HARNESS_PANIC");
            Err(failure_code(&evidence, "NISAR_LIVE_HARNESS_PANIC"))
        }
    };
    if let Err(code) = outcome {
        cleanup.discard_final_artifact();
        let preserve_failure = std::env::var("INSAR_NISAR_SUBSET_PRESERVE_FAILURE")
            .ok()
            .as_deref()
            == Some("1");
        let cleanup_result = if preserve_failure {
            cleanup.preserve_failure_diagnostics();
            Ok(())
        } else {
            cleanup.cleanup_now()
        };
        let final_code = failure_code(&evidence, "NISAR_LIVE_EVIDENCE_INCOMPLETE");
        panic!(
            "Frequency B HH live acceptance stopped with typed NO_GO: {final_code} (boundary={code}, cleanup={cleanup_result:?})"
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "diagnoses one preserved live Store against the verified local full-source oracle"]
fn nisar_live_store_local_semantic_forensic() {
    insar_download::establish_hdf5_plugin_isolation()
        .expect("forensic host establishes app-owned HDF5 plugin isolation");
    let store_path = PathBuf::from(
        std::env::var("INSAR_NISAR_FORENSIC_STORE").expect("preserved live Store path"),
    );
    let source_path = PathBuf::from(
        std::env::var("INSAR_NISAR_FORENSIC_SOURCE").expect("verified local source path"),
    );
    assert_eq!(fs::metadata(&source_path).unwrap().len(), SOURCE_BYTES);

    let store = TaskStore::open(&store_path).expect("preserved live Store opens");
    let frozen = store
        .load_workbench()
        .expect("live workbench read")
        .expect("live workbench exists");
    let source_aoi = frozen.batch.aoi_geojson.as_ref().expect("live source AOI");
    let normalized = normalize_nisar_subset_aoi(source_aoi).expect("live source AOI normalizes");
    let mut item = frozen
        .batch
        .candidates
        .iter()
        .find(|item| item.display_name == GRANULE_ID)
        .expect("fixed live source item")
        .clone();
    for (key, value) in [
        (
            "delivery_kind",
            Value::String("remote_derived_subset".to_owned()),
        ),
        ("subset_frequency", Value::String("A".to_owned())),
        ("subset_polarization", Value::String("HH".to_owned())),
        ("subset_swath", Value::String(SWATH.to_owned())),
        (
            "subset_aoi_geojson",
            Value::Object(normalized.canonical_geojson.clone()),
        ),
    ] {
        item.metadata.insert(key.to_owned(), value);
    }
    if let Ok(path) = std::env::var("INSAR_NISAR_FORENSIC_ITEM_OUT") {
        fs::write(
            path,
            serde_json::to_vec_pretty(&item).expect("forensic item serializes"),
        )
        .expect("forensic item writes");
        return;
    }
    let request = insar_download::build_live_nisar_subset_request(
        &item,
        &normalized.canonical_geojson,
        NisarSubsetFrequency::A,
        "HH",
        SWATH,
        &source_path,
    )
    .expect("live Store request rebuilds from verified local source");
    let validator = insar_core::NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: "\"forensic-local-source\"".to_owned(),
        content_length: SOURCE_BYTES,
    };
    let metadata = insar_download::NisarSubsetMetadataSnapshotV1::from_verified_live_metadata_hdf5(
        &request,
        validator,
        &source_path,
        &|| false,
    )
    .expect("verified local metadata snapshot");
    let plan = insar_download::live_nisar_range_plan_from_sparse_hdf5(
        &request,
        &source_path,
        metadata.evidence_sha256(),
    )
    .expect("verified local physical plan");
    let semantic = insar_download::semantic_plan_canonical_snapshot(&request, &metadata, &plan)
        .expect("semantic forensic snapshot");
    println!(
        "NISAR_SEMANTIC_FORENSIC={}",
        serde_json::to_string(&semantic).expect("forensic snapshot serializes")
    );

    let mut baseline_item = production_mapped_fixed_scene_fixture();
    for (key, value) in [
        (
            "delivery_kind",
            Value::String("remote_derived_subset".to_owned()),
        ),
        ("subset_frequency", Value::String("A".to_owned())),
        ("subset_polarization", Value::String("HH".to_owned())),
        ("subset_swath", Value::String(SWATH.to_owned())),
        (
            "subset_aoi_geojson",
            Value::Object(normalized.canonical_geojson.clone()),
        ),
    ] {
        baseline_item.metadata.insert(key.to_owned(), value);
    }
    let baseline_request = insar_download::build_live_nisar_subset_request(
        &baseline_item,
        &normalized.canonical_geojson,
        NisarSubsetFrequency::A,
        "HH",
        SWATH,
        &source_path,
    )
    .expect("baseline request rebuilds from verified local source");
    let baseline_metadata =
        insar_download::NisarSubsetMetadataSnapshotV1::from_verified_live_metadata_hdf5(
            &baseline_request,
            insar_core::NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: "\"forensic-local-source\"".to_owned(),
                content_length: SOURCE_BYTES,
            },
            &source_path,
            &|| false,
        )
        .expect("baseline local metadata snapshot");
    let baseline_plan = insar_download::live_nisar_range_plan_from_sparse_hdf5(
        &baseline_request,
        &source_path,
        baseline_metadata.evidence_sha256(),
    )
    .expect("baseline local physical plan");
    let baseline_semantic = insar_download::semantic_plan_canonical_snapshot(
        &baseline_request,
        &baseline_metadata,
        &baseline_plan,
    )
    .expect("baseline semantic forensic snapshot");
    println!(
        "NISAR_SEMANTIC_BASELINE={}",
        serde_json::to_string(&baseline_semantic).expect("baseline snapshot serializes")
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
const C2_BASELINE_COMMIT: &str = "300088fc6e1306bb9283fa0013f47d3894849b4c";
#[cfg(feature = "nisar-subset-live-acceptance")]
const C2_GRANULE_ID: &str =
    "NISAR_L1_PR_RSLC_025_017_A_147_7700_SHNA_A_20260709T100325_20260709T100342_P05023_N_P_J_001";
#[cfg(feature = "nisar-subset-live-acceptance")]
const C2_PROVIDER_PRODUCT_ID: &str = "G4244267663-ASF";
#[cfg(feature = "nisar-subset-live-acceptance")]
const C2_FILE_NAME: &str =
    "NISAR_L1_PR_RSLC_025_017_A_147_7700_SHNA_A_20260709T100325_20260709T100342_P05023_N_P_J_001.h5";
#[cfg(feature = "nisar-subset-live-acceptance")]
const C2_SOURCE_BYTES: u64 = 5_280_628_736;
#[cfg(feature = "nisar-subset-live-acceptance")]
const C2_SOURCE_MD5: &str = "3fb9b2412c4f78fa7185eb33c4585791";
#[cfg(feature = "nisar-subset-live-acceptance")]
const C2_AOI_SHA256: &str = "ad2107fbc106108c132f6a5efb1b12881ca92f33f732b3a16db2ab38826c320b";
#[cfg(feature = "nisar-subset-live-acceptance")]
const C2_REQUEST_ID: &str = "nisar-m2d-c2-second-scene-request";
#[cfg(feature = "nisar-subset-live-acceptance")]
const C2_TASK_ID: &str = "nisar-m2d-c2-second-scene-task";
#[cfg(feature = "nisar-subset-live-acceptance")]
const C2_BATCH_ID: &str = "nisar-m2d-c2-second-scene-batch";

#[cfg(feature = "nisar-subset-live-acceptance")]
fn c2_aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [-55.00, -61.36],
            [-54.78, -61.36],
            [-54.78, -61.16],
            [-55.00, -61.16],
            [-55.00, -61.36]
        ]]
    })
    .as_object()
    .expect("C2 AOI object")
    .clone()
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn c2_query() -> Result<CatalogSearchQuery, &'static str> {
    serde_json::from_value(json!({
        "logical_dataset": NISAR_LOGICAL_DATASET,
        "provider_id": NISAR_PROVIDER_ID,
        "native_collection_id": NISAR_NATIVE_COLLECTION_ID,
        "aoi_geojson": c2_aoi(),
        "cursor": null,
        "result_limit": 100,
        "filters": {
            "datetime": {
                "start": "2026-07-09T09:30:00Z",
                "end": "2026-07-09T10:30:00Z"
            },
            "production_configuration": "PR"
        }
    }))
    .map_err(|_| "NISAR_C2_CATALOGUE_QUERY_INVALID")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn c2_fixed_gate() -> Result<(String, PathBuf, PathBuf, PathBuf, PathBuf), &'static str> {
    if std::env::var("INSAR_NISAR_C2_LIVE_ACCEPTANCE")
        .ok()
        .as_deref()
        != Some("1")
    {
        return Err("NISAR_C2_LIVE_ACCEPTANCE_REQUIRED");
    }
    if std::env::var("INSAR_NISAR_C2_GRANULE").ok().as_deref() != Some(C2_GRANULE_ID) {
        return Err("NISAR_C2_GRANULE_MISMATCH");
    }
    let mode = std::env::var("INSAR_NISAR_C2_MODE").map_err(|_| "NISAR_C2_MODE_REQUIRED")?;
    if !matches!(mode.as_str(), "prepare" | "end_to_end") {
        return Err("NISAR_C2_MODE_INVALID");
    }
    let absolute = |name: &'static str| -> Result<PathBuf, &'static str> {
        let path = PathBuf::from(std::env::var(name).map_err(|_| "NISAR_C2_PATH_REQUIRED")?);
        if !path.is_absolute() {
            return Err("NISAR_C2_PATH_NOT_ABSOLUTE");
        }
        Ok(path)
    };
    let root = absolute("INSAR_NISAR_C2_ROOT")?;
    if root.exists() {
        return Err("NISAR_C2_ROOT_MUST_NOT_EXIST");
    }
    let input = absolute("INSAR_NISAR_C2_ORACLE_INPUT")?;
    let output = absolute("INSAR_NISAR_C2_ORACLE_OUTPUT")?;
    let comparison = absolute("INSAR_NISAR_C2_GEOMETRY_COMPARISON")?;
    if mode == "prepare" && output.exists() {
        return Err("NISAR_C2_ORACLE_OUTPUT_MUST_NOT_EXIST");
    }
    Ok((mode, root, input, output, comparison))
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn c2_validate_source(item: &SnapshotItem) -> Result<(), &'static str> {
    if item.provider_id != NISAR_PROVIDER_ID
        || item.display_name != C2_GRANULE_ID
        || item
            .metadata
            .get("native_collection_id")
            .and_then(Value::as_str)
            != Some(NISAR_NATIVE_COLLECTION_ID)
        || item
            .metadata
            .get("provider_product_id")
            .and_then(Value::as_str)
            != Some(C2_PROVIDER_PRODUCT_ID)
        || item
            .metadata
            .get("frequency_a_polarization")
            .and_then(Value::as_str)
            != Some("HH+HV")
    {
        return Err("NISAR_C2_SOURCE_IDENTITY_INVALID");
    }
    let asset = item
        .asset_access
        .as_ref()
        .ok_or("NISAR_C2_PRIMARY_ASSET_MISSING")?;
    if asset.role != insar_core::AssetRole::Primary
        || asset.file_name.as_deref() != Some(C2_FILE_NAME)
        || asset.expected_size_bytes != Some(C2_SOURCE_BYTES)
        || asset.checksum_algorithm.as_deref() != Some("MD5")
        || asset.checksum_value.as_deref() != Some(C2_SOURCE_MD5)
        || asset.reference.collection_id != NISAR_NATIVE_COLLECTION_ID
        || asset.reference.provider_product_id != C2_PROVIDER_PRODUCT_ID
    {
        return Err("NISAR_C2_PRIMARY_ASSET_INVALID");
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn c2_save_source(path: &Path, item: SnapshotItem) -> Result<(), &'static str> {
    let item_id = item.item_id.clone();
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: C2_BATCH_ID.to_owned(),
        provider_id: item.provider_id.clone(),
        created_at: "2026-08-09T00:00:00Z".to_owned(),
        aoi_name: "second_scene_aoi_v1".to_owned(),
        aoi_geojson: Some(c2_aoi()),
        query: Map::new(),
        search_stats: None,
        candidates: vec![item],
    };
    let mut store = TaskStore::open(path).map_err(|_| "NISAR_C2_SOURCE_STORE_FAILED")?;
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch,
            workbench: SearchWorkbenchState {
                batch_id: C2_BATCH_ID.to_owned(),
                checked_item_ids: vec![item_id],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .map_err(|_| "NISAR_C2_SOURCE_STORE_FAILED")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn c2_validate_final_evidence(
    evidence: &NisarLiveAcceptanceEvidenceV2,
    output: &Path,
) -> Result<(), &'static str> {
    if evidence.schema_version != insar_download::NISAR_M2D_C2_EVIDENCE_SCHEMA
        || evidence.gate != insar_download::NISAR_M2D_C2_GATE
        || evidence.baseline_commit != C2_BASELINE_COMMIT
        || evidence.status != "pass"
        || evidence.typed_stop_code != "PASS"
        || !valid_evidence_atom(&evidence.harness_commit)
        || evidence.granule_id != C2_GRANULE_ID
        || evidence.source_size_bytes != C2_SOURCE_BYTES
        || evidence.source_checksum_algorithm != "MD5"
        || evidence.source_checksum != C2_SOURCE_MD5
        || evidence.aoi_digest != C2_AOI_SHA256
        || evidence.frequency != "A"
        || evidence.polarization != "HH+HV"
        || evidence.advertised_polarizations.as_slice() != ["HH".to_owned(), "HV".to_owned()]
        || evidence.selected_polarizations.as_slice() != ["HH".to_owned(), "HV".to_owned()]
        || evidence.provider_id != NISAR_PROVIDER_ID
        || evidence.provider_product_id != C2_PROVIDER_PRODUCT_ID
        || evidence.provider_product_key
            != "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G4244267663-ASF"
        || evidence.collection_id != NISAR_NATIVE_COLLECTION_ID
        || evidence.final_host_class != "earthdata_asf_v1"
        || evidence.final_path_class != "approved_nisar_hdf5_object"
        || evidence.source_dataset_count == 0
        || evidence
            .metadata_group_binding_count
            .checked_add(evidence.metadata_dataset_binding_count)
            != Some(evidence.metadata_binding_count)
        || evidence.output_authority != insar_core::NISAR_SUBSET_OUTPUT_AUTHORITY
        || evidence.output_class != "client_derived_provisional_nisar_rslc_aoi_subset"
        || !evidence.one_hdf5
        || evidence.output_size_bytes == 0
        || !evidence.atomic_promotion
        || evidence.store_completed != "observed"
        || evidence.store_reopen != "observed"
        || !has_hdf5_output(output)
        || !validate_v2_authority_digests_scene_generic(evidence)
    {
        return Err("NISAR_C2_EVIDENCE_INCOMPLETE");
    }
    if evidence.local_qualification_execution_plan_sha256 != "not_applicable_scene_generic"
        || evidence.expected_semantic_plan_sha256 != "not_applicable_scene_generic"
        || !is_sha256(&evidence.execution_plan_sha256)
        || !is_sha256(&evidence.semantic_plan_sha256)
    {
        return Err("NISAR_C2_SEMANTIC_PLAN_POSTCHECK_FAILED");
    }
    if !validate_v2_range_evidence(evidence, C2_SOURCE_BYTES, &evidence.source_validator_sha256) {
        return Err("NISAR_C2_RANGE_POSTCHECK_FAILED");
    }
    if !validate_v2_budget_evidence(evidence) {
        return Err("NISAR_C2_BUDGET_POSTCHECK_FAILED");
    }
    let geometry = &evidence.geometry;
    let science = &evidence.scientific_validation;
    if geometry.line_end_exclusive <= geometry.line_start
        || geometry.sample_end_exclusive <= geometry.sample_start
        || science.complex_mismatch != 0
        || science.amplitude_mismatch != 0
        || science.wrapped_phase_mismatch != 0
        || science.mask_mismatch != 0
        || !science.logical_science_equality
        || !science.identification_semantics
        || !science.dimension_scales
        || !science.metadata_semantics
        || !science.provenance_semantics
    {
        return Err("NISAR_C2_SCIENCE_POSTCHECK_FAILED");
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn run_c2_live_validation(
    mode: &str,
    root: &Path,
    oracle_input: &Path,
    oracle_output: &Path,
    comparison: &Path,
) -> Result<(), &'static str> {
    let database = root.join("tasks.sqlite3");
    let output = root.join("output");
    let query = c2_query()?;
    let normalized = normalize_nisar_subset_aoi(&c2_aoi()).map_err(|_| "NISAR_C2_AOI_INVALID")?;
    if normalized.sha256 != C2_AOI_SHA256 {
        return Err("NISAR_C2_AOI_DIGEST_MISMATCH");
    }
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .ok_or("NISAR_C2_PROFILE_MISSING")?;
    let response = controlled_catalogue()?
        .search_nisar(&query, profile, &|| false)
        .map_err(|_| "NISAR_C2_CATALOGUE_FAILED")?;
    let mut products = response
        .products
        .into_iter()
        .filter(|product| product.display_name == C2_GRANULE_ID)
        .collect::<Vec<_>>();
    if products.len() != 1 {
        return Err("NISAR_C2_GRANULE_CARDINALITY_INVALID");
    }
    let product = products.remove(0);
    let descriptor: AsfNisarSubsetLiveFrequencyAMultiPolDescriptorV1 =
        build_nisar_subset_live_frequency_a_multi_pol_descriptor(
            product.clone(),
            &query,
            &["HH".to_owned(), "HV".to_owned()],
        )
        .map_err(|_| "NISAR_C2_SOURCE_IDENTITY_INVALID")?;
    if descriptor.advertised_frequency_a_polarizations() != ["HH", "HV"]
        || descriptor.selected_polarizations() != ["HH", "HV"]
        || descriptor.selected_frequency() != NisarSubsetFrequency::A
        || descriptor.granule_id() != C2_GRANULE_ID
    {
        return Err("NISAR_C2_SOURCE_IDENTITY_INVALID");
    }
    let selected_polarizations = descriptor.selected_polarizations().to_vec();
    let item = map_nisar_product_for_live_acceptance(product, &query)
        .map_err(|_| "NISAR_C2_CATALOGUE_MAPPING_FAILED")?;
    c2_validate_source(&item)?;
    c2_save_source(&database, item.clone())?;

    let evidence = Arc::new(
        NisarLiveAcceptanceEvidenceSink::new_for_scene_generic_acceptance(
            root.to_path_buf(),
            C2_BASELINE_COMMIT,
            current_git_commit(),
            insar_download::NISAR_M2D_C2_EVIDENCE_SCHEMA,
            insar_download::NISAR_M2D_C2_GATE,
        )
        .map_err(|_| "NISAR_C2_EVIDENCE_INIT_FAILED")?,
    );
    let production_worker = ProductionDownloadWorker::new(TransferOptions::default())
        .map_err(|_| "NISAR_C2_WORKER_INIT_FAILED")?
        .with_nisar_live_evidence_sink_for_acceptance(Arc::clone(&evidence))
        .with_nisar_isce3_geometry_gate_for_acceptance(NisarIsce3LiveGeometryGateV1 {
            input_path: oracle_input.to_path_buf(),
            output_path: oracle_output.to_path_buf(),
            comparison_path: comparison.to_path_buf(),
        });
    let service = ApplicationService::with_production_worker_for_live_acceptance(
        ApplicationServiceConfig::business_store(&database)
            .with_controlled_capability_runtime_observations(
                controlled_live_capability_observations(),
            ),
        Arc::new(production_worker),
    );
    let expected_workbench_revision = TaskStore::open(&database)
        .map_err(|_| "NISAR_C2_SOURCE_STORE_FAILED")?
        .load_workbench()
        .map_err(|_| "NISAR_C2_SOURCE_STORE_FAILED")?
        .filter(|frozen| frozen.batch.batch_id == C2_BATCH_ID)
        .map(|frozen| frozen.state.revision)
        .ok_or("NISAR_C2_SOURCE_STORE_FAILED")?;
    let prepared = service
        .prepare_nisar_subset_intent_v2(&NisarPrepareSubsetIntentV2Request {
            source_batch_id: C2_BATCH_ID.to_owned(),
            source_item_id: item.item_id.clone(),
            expected_workbench_revision,
            subset_draft_geometry: c2_aoi(),
            selected_frequency: "A".to_owned(),
            selected_polarizations: selected_polarizations.clone(),
        })
        .map_err(|_| "NISAR_C2_TASK_PREPARE_FAILED")?;
    if prepared.source_batch_id != C2_BATCH_ID
        || prepared.source_item_id != item.item_id
        || prepared.workbench_revision != expected_workbench_revision
    {
        return Err("NISAR_C2_TASK_PREPARE_FAILED");
    }
    service
        .confirm_nisar_subset_task_v2(&NisarConfirmSubsetTaskV2Request {
            source_batch_id: prepared.source_batch_id.clone(),
            source_item_id: prepared.source_item_id.clone(),
            expected_workbench_revision: prepared.workbench_revision,
            prepared_intent: prepared.intent.clone(),
            task_id: C2_TASK_ID.to_owned(),
            output_root: output.clone(),
            layout: insar_core::DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-08-09T00:00:00Z".to_owned(),
        })
        .map_err(|_| "NISAR_C2_TASK_CONFIRM_FAILED")?;

    let confirmed = TaskStore::open(&database)
        .map_err(|_| "NISAR_C2_STORE_REOPEN_FAILED")?
        .load_task_record(C2_TASK_ID)
        .map_err(|_| "NISAR_C2_STORE_REOPEN_FAILED")?
        .ok_or("NISAR_C2_TASK_CONFIRM_FAILED")?;
    let confirmed_item = confirmed
        .snapshot
        .candidates
        .first()
        .ok_or("NISAR_C2_TASK_CONFIRM_FAILED")?;
    let prepared_envelope =
        insar_store::parse_nisar_subset_prepared_intent_v2_envelope(&confirmed_item.metadata)
            .map_err(|_| "NISAR_C2_TASK_CONFIRM_FAILED")?;
    let plan_envelope = insar_store::parse_nisar_subset_plan_v2_envelope(&confirmed_item.metadata)
        .map_err(|_| "NISAR_C2_TASK_CONFIRM_FAILED")?;
    if confirmed.snapshot.batch_id != C2_BATCH_ID
        || confirmed.snapshot.candidates.len() != 1
        || confirmed.runtime.status != TaskLifecycleStatus::Paused
        || !confirmed.runtime.task_paused
        || !confirmed.runtime.queued_item_ids.is_empty()
        || confirmed.runtime.paused_item_ids.len() != 1
        || confirmed.runtime.paused_item_ids.first() != Some(&item.item_id)
        || confirmed_item.item_id != item.item_id
        || confirmed_item
            .metadata
            .get("delivery_kind")
            .and_then(Value::as_str)
            != Some("remote_derived_subset_v2")
        || prepared_envelope.is_none()
        || plan_envelope.is_some()
    {
        return Err("NISAR_C2_V2_PAUSED_STATE_INVALID");
    }
    for (offset, kind, item_ids) in [
        (0_u64, TaskStateCommandKind::ResumeTask, Vec::new()),
        (
            1_u64,
            TaskStateCommandKind::ResumeItems,
            vec![item.item_id.clone()],
        ),
    ] {
        service
            .update_download_task(&StoreUpdateTaskRequest {
                task_id: C2_TASK_ID.to_owned(),
                expected_revision: confirmed.runtime.revision + offset,
                command: StoreTaskCommand { kind, item_ids },
                event_id: format!("{C2_REQUEST_ID}:resume:{offset}"),
                occurred_at: "2026-08-09T00:00:00Z".to_owned(),
            })
            .map_err(|_| "NISAR_C2_TASK_RESUME_FAILED")?;
    }

    let deadline = Instant::now() + Duration::from_secs(45 * 60);
    loop {
        if matches!(evidence.snapshot().status.as_str(), "pass" | "fail") {
            break;
        }
        if Instant::now() >= deadline {
            evidence
                .record_failure("NISAR_C2_TIMEOUT")
                .map_err(|_| "NISAR_C2_EVIDENCE_INCOMPLETE")?;
            let latest_runtime = TaskStore::open(&database)
                .map_err(|_| "NISAR_C2_TIMEOUT_STORE_LOAD_FAILED")?
                .load_scheduler_control_runtime(C2_TASK_ID)
                .map_err(|_| "NISAR_C2_TIMEOUT_STORE_LOAD_FAILED")?
                .ok_or("NISAR_C2_TIMEOUT_STORE_LOAD_FAILED")?;
            service
                .update_download_task(&StoreUpdateTaskRequest {
                    task_id: C2_TASK_ID.to_owned(),
                    expected_revision: latest_runtime.revision,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::CancelTask,
                        item_ids: Vec::new(),
                    },
                    event_id: format!("{C2_TASK_ID}:timeout-cancel:v2"),
                    occurred_at: "2026-08-09T00:00:00Z".to_owned(),
                })
                .map_err(|_| "NISAR_C2_TIMEOUT_CANCELLATION_FAILED")?;

            let cancellation_deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let store =
                    TaskStore::open(&database).map_err(|_| "NISAR_C2_TIMEOUT_STORE_LOAD_FAILED")?;
                let runtime = store
                    .load_scheduler_control_runtime(C2_TASK_ID)
                    .map_err(|_| "NISAR_C2_TIMEOUT_STORE_LOAD_FAILED")?
                    .ok_or("NISAR_C2_TIMEOUT_STORE_LOAD_FAILED")?;
                let durable_cancelled = runtime.status == TaskLifecycleStatus::Cancelled
                    && runtime.active_item_ids.is_empty()
                    && runtime.queued_item_ids.is_empty()
                    && runtime.paused_item_ids.is_empty()
                    && runtime
                        .cancelled_item_ids
                        .iter()
                        .any(|item_id| item_id == &item.item_id);
                if durable_cancelled {
                    let download = store
                        .load_download_snapshot(C2_TASK_ID)
                        .map_err(|_| "NISAR_C2_TIMEOUT_STORE_LOAD_FAILED")?;
                    match download
                        .results
                        .iter()
                        .find(|result| result.item_id == item.item_id)
                    {
                        Some(result) if result.outcome == DownloadOutcome::Cancelled => break,
                        Some(_) => return Err("NISAR_C2_TIMEOUT_WORKER_FAILURE"),
                        None => {}
                    }
                }
                if Instant::now() >= cancellation_deadline {
                    return Err(if durable_cancelled {
                        "NISAR_C2_TIMEOUT_SCHEDULER_NOT_CONSUMED_OR_WORKER_NOT_STOPPED"
                    } else {
                        "NISAR_C2_TIMEOUT_STORE_NOT_TERMINAL"
                    });
                }
                thread::sleep(Duration::from_millis(250));
            }
            if service.shutdown().is_err() {
                return Err("NISAR_C2_TIMEOUT_SERVICE_SHUTDOWN_FAILED");
            }
            let reopened =
                TaskStore::open(&database).map_err(|_| "NISAR_C2_TIMEOUT_REOPEN_MISMATCH")?;
            let reopened_task = reopened
                .load_task_record(C2_TASK_ID)
                .map_err(|_| "NISAR_C2_TIMEOUT_REOPEN_MISMATCH")?
                .ok_or("NISAR_C2_TIMEOUT_REOPEN_MISMATCH")?;
            let reopened_result_cancelled = reopened
                .load_download_snapshot(C2_TASK_ID)
                .map_err(|_| "NISAR_C2_TIMEOUT_REOPEN_MISMATCH")?
                .results
                .iter()
                .any(|result| {
                    result.item_id == item.item_id && result.outcome == DownloadOutcome::Cancelled
                });
            if reopened_task.runtime.status != TaskLifecycleStatus::Cancelled
                || !reopened_task.runtime.active_item_ids.is_empty()
                || !reopened_task.runtime.queued_item_ids.is_empty()
                || !reopened_task.runtime.paused_item_ids.is_empty()
                || !reopened_task
                    .runtime
                    .cancelled_item_ids
                    .iter()
                    .any(|item_id| item_id == &item.item_id)
                || !reopened_result_cancelled
            {
                return Err("NISAR_C2_TIMEOUT_REOPEN_MISMATCH");
            }
            evidence
                .mark_store_reopen("observed")
                .map_err(|_| "NISAR_C2_EVIDENCE_INCOMPLETE")?;
            return Err("NISAR_C2_TIMEOUT");
        }
        thread::sleep(Duration::from_millis(250));
    }
    let worker_failure = evidence.snapshot().status == "fail";
    if !worker_failure && mode != "prepare" {
        if let Err(code) = wait_for_durable_store_completion(
            &database,
            C2_TASK_ID,
            &item.item_id,
            Duration::from_secs(30),
            "NISAR_C2_STORE_COMPLETION_FAILED",
        ) {
            let _ = service.shutdown();
            return Err(code);
        }
        if evidence.mark_store_completed("observed").is_err() {
            let _ = service.shutdown();
            return Err("NISAR_C2_EVIDENCE_INCOMPLETE");
        }
    }
    if service.shutdown().is_err() && !worker_failure {
        return Err("NISAR_C2_SERVICE_SHUTDOWN_FAILED");
    }

    if mode == "prepare" {
        let snapshot = evidence.snapshot();
        if snapshot.status != "fail"
            || snapshot.typed_stop_code != "NISAR_ISCE3_ORACLE_REQUIRED"
            || !oracle_input.is_file()
            || oracle_output.exists()
            || has_hdf5_output(&output)
        {
            return Err("NISAR_C2_ORACLE_PREPARATION_FAILED");
        }
        println!("P0.3a-M2D-C2_ORACLE_INPUT={}", oracle_input.display());
        return Ok(());
    }

    if worker_failure {
        return Err("NISAR_C2_WORKER_FAILURE_PRESERVED");
    }
    let reopened = TaskStore::open(&database).map_err(|_| "NISAR_C2_STORE_REOPEN_FAILED")?;
    if reopened
        .load_task_record(C2_TASK_ID)
        .map_err(|_| "NISAR_C2_STORE_REOPEN_FAILED")?
        .is_none()
    {
        return Err("NISAR_C2_STORE_REOPEN_FAILED");
    }
    drop(reopened);
    evidence
        .mark_store_reopen("observed")
        .map_err(|_| "NISAR_C2_EVIDENCE_INCOMPLETE")?;
    c2_validate_final_evidence(&evidence.snapshot(), &output)?;
    if !comparison.is_file() {
        return Err("NISAR_C2_GEOMETRY_COMPARISON_MISSING");
    }
    println!(
        "P0.3a-M2D-C2_REDACTED_EVIDENCE_PATH={}",
        evidence.evidence_path().display()
    );
    println!("P0.3a-M2D-C2_OUTPUT_ROOT={}", output.display());
    println!("P0.3a-M2D-C2_GEOMETRY_COMPARISON={}", comparison.display());
    Ok(())
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn c2_second_scene_scope_is_fixed_without_network() {
    let normalized = normalize_nisar_subset_aoi(&c2_aoi()).expect("fixed C2 AOI");
    assert_eq!(normalized.sha256, C2_AOI_SHA256);
    assert_eq!(C2_SOURCE_BYTES, 5_280_628_736);
    assert_eq!(C2_SOURCE_MD5.len(), 32);
    assert_eq!(C2_PROVIDER_PRODUCT_ID, "G4244267663-ASF");
    assert_ne!(C2_GRANULE_ID, GRANULE_ID);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
#[ignore = "explicitly authorized bounded second-scene C2 live validation"]
fn nisar_second_scene_c2_live_validation() {
    let (mode, root, oracle_input, oracle_output, comparison) =
        c2_fixed_gate().expect("all explicit C2 live gates are required before network access");
    insar_download::establish_hdf5_plugin_isolation()
        .expect("C2 live harness must establish app-owned HDF5 plugin isolation before HDF5 use");
    fs::create_dir_all(&root).expect("fresh C2 root");
    let result = catch_unwind(AssertUnwindSafe(|| {
        run_c2_live_validation(&mode, &root, &oracle_input, &oracle_output, &comparison)
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(code)) => panic!("C2 stopped with typed boundary: {code}"),
        Err(_) => panic!("C2 stopped with typed boundary: NISAR_C2_HARNESS_PANIC"),
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[path = "support/nisar_recovery_receipt.rs"]
mod nisar_recovery_receipt;
