//! Route tests of the bounded transient retry of the Bhoonidhi download probe
//! (downloader enhancement D1d). L4 2026-10-18: one "operation timed out
//! (while sending Bhoonidhi download probe request)" failed the item at once
//! as DL005.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters, AssetRole,
    DownloadOutcome, FrozenAssetAccess, FrozenTaskSnapshot, SnapshotItem, StableAssetReference,
};
use insar_provider::bhoonidhi::{
    BhoonidhiAccessTokenProvider, BhoonidhiAuthError as CatalogAuthError, BHOONIDHI_COLLECTION_ID,
    BHOONIDHI_FILENAME_RULE_ID, BHOONIDHI_IDENTITY_RULE_ID, BHOONIDHI_LOGICAL_DATASET,
    BHOONIDHI_PROFILE_ID, BHOONIDHI_PROVIDER_ID, BHOONIDHI_RESOLVER_POLICY_ID,
};
use serde_json::{json, Map};
use zeroize::Zeroizing;

use crate::asset_resolver::bhoonidhi::parse_download_probe_response;
use crate::transfer_backend::{
    TransferBackend, TransferBackendKind, ValidatedResolvedTransferResult,
    ValidatedTransferDispatch,
};
use crate::{
    ArtifactValidationPlan, AssetResolverRegistry, AuthRefreshPolicyId, BhoonidhiAssetResolver,
    BhoonidhiDownloadProbe, BhoonidhiDownloadProbeError, BhoonidhiDownloadProbeEvidence,
    CredentialMaterial, CredentialProvider, CredentialProviderError, DownloadWork, DownloadWorker,
    ProductionDownloadWorker, ResolveContext, ResolvedAsset, ResolvedTransferResult,
    ResolvedTransferSession, ResolverExecutionRuntime, SystemClock, TransferAuth, TransferOptions,
    TransferRequest, TransferResult, TransferUpdate, WorkerControl,
};

const PRODUCT: &str =
    "NISAR_S1_PR_RSLC_030_026_A_020_3700_DHNA_A_20260907T234951_20260907T235005_P00500_M_P_I_001";
const SOURCE_SIZE: u64 = 21_054_588_114;

struct TokenProvider;

impl BhoonidhiAccessTokenProvider for TokenProvider {
    fn access_token(&self) -> Result<Zeroizing<String>, CatalogAuthError> {
        Ok(Zeroizing::new("fixture-bhoonidhi-access".to_owned()))
    }
}

struct NoCredentials;

impl CredentialProvider for NoCredentials {
    fn resolve(&self, _profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        Err(CredentialProviderError::Missing)
    }
}

/// Fails the first `failures` probes with a send timeout, then answers.
struct FlakyProbe {
    failures: usize,
    calls: AtomicUsize,
}

impl BhoonidhiDownloadProbe for FlakyProbe {
    fn probe(
        &self,
        asset: &ResolvedAsset,
        _context: &ResolveContext<'_>,
    ) -> Result<BhoonidhiDownloadProbeEvidence, BhoonidhiDownloadProbeError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) < self.failures {
            return Err(BhoonidhiDownloadProbeError::Timeout {
                operation: "sending Bhoonidhi download probe request",
                message:
                    "error sending request for url (https://bhoonidhi-api.nrsc.gov.in/download)"
                        .to_owned(),
                source_chain: vec!["operation timed out".to_owned()],
                secret_material_redacted: false,
            });
        }
        parse_download_probe_response(
            asset.effective_url(),
            asset.effective_url(),
            206,
            Some(&format!("bytes 0-0/{SOURCE_SIZE}")),
            Some(1),
            Some("\"308dbc892d0ce417013aa66f28a61cb0-402\""),
            None,
            1,
        )
    }
}

/// Stands in for the validated transfer: records the call and reports a
/// completed transfer, so the route test exercises resolution only.
#[derive(Default)]
struct CompletingBackend {
    calls: AtomicUsize,
}

impl TransferBackend for CompletingBackend {
    fn kind(&self) -> TransferBackendKind {
        TransferBackendKind::NativeSingleStream
    }

    fn is_available(&self) -> bool {
        true
    }

    fn download_legacy(
        &self,
        _request: &TransferRequest,
        _auth: &TransferAuth,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        panic!("a frozen Bhoonidhi item must never use the Legacy transfer")
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(3).unwrap()
    }

    fn download_resolved(
        &self,
        _request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        _session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        panic!("the S-band product is transferred through its validator")
    }

    fn download_resolved_validated(
        &self,
        dispatch: &ValidatedTransferDispatch,
        request: &TransferRequest,
        asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        plan: &ArtifactValidationPlan,
        session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> ValidatedResolvedTransferResult {
        assert_eq!(asset.expected_size(), Some(SOURCE_SIZE));
        self.calls.fetch_add(1, Ordering::SeqCst);
        ValidatedResolvedTransferResult::seal(
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                bytes_written: SOURCE_SIZE,
                attempt_count: 1,
                failure: None,
            }),
            request,
            plan,
            session,
            dispatch,
        )
    }
}

fn bhoonidhi_item() -> SnapshotItem {
    let key =
        provider_product_key(BHOONIDHI_PROVIDER_ID, BHOONIDHI_COLLECTION_ID, PRODUCT).unwrap();
    SnapshotItem {
        item_id: key.clone(),
        provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
        display_name: PRODUCT.to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: json!({
            "dataset": BHOONIDHI_LOGICAL_DATASET, "logical_dataset": BHOONIDHI_LOGICAL_DATASET,
            "native_collection_id": BHOONIDHI_COLLECTION_ID, "collection_profile_id": BHOONIDHI_PROFILE_ID,
            "identity_rule_id": BHOONIDHI_IDENTITY_RULE_ID, "filename_rule_id": BHOONIDHI_FILENAME_RULE_ID,
            "resolver_policy_id": BHOONIDHI_RESOLVER_POLICY_ID, "delivery_model": "archive_first",
            "product_family": BHOONIDHI_COLLECTION_ID, "processing_level": "RSLC",
            "provider_product_id": PRODUCT, "provider_product_key": key,
            "properties": {"ID": PRODUCT, "Online": "Y", "Platform": "NISAR", "Instrument": "SSAR",
                "Producer": "ISRO", "ProductCode": "ST0000KED", "Mode": "W", "Polarization": "DH",
                "SoftwareVersion": "1.4.00", "Coverage": "P", "mode": "W", "polarization": "DH",
                "software_version": "1.4.00"}
        })
        .as_object()
        .unwrap()
        .clone(),
        payload: Map::new(),
        asset_access: Some(FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
                collection_id: BHOONIDHI_COLLECTION_ID.to_owned(),
                provider_product_id: PRODUCT.to_owned(),
                provider_product_key: key.clone(),
                canonical_product_key: None,
                asset_key: "data".to_owned(),
                stable_asset_key: stable_asset_key(&key, "data").unwrap(),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
                scheme: AssetAccessScheme::OauthBearerHttps,
                stable_href: Some("https://bhoonidhi-api.nrsc.gov.in/download".to_owned()),
                requires_credentials: true,
                credential_profile: Some("bhoonidhi-default".to_owned()),
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some(format!("{PRODUCT}.h5")),
            expected_size_bytes: None,
            checksum_algorithm: None,
            checksum_value: None,
        }),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
            collection_profile_id: Some(BHOONIDHI_PROFILE_ID.to_owned()),
        }),
    }
}

fn work(output_root: &Path, item: SnapshotItem) -> DownloadWork {
    let task: FrozenTaskSnapshot = serde_json::from_value(json!({
        "schema_version": 1,
        "task_id": "bhoonidhi-probe-retry",
        "batch_id": "bhoonidhi-probe-retry-batch",
        "provider_id": BHOONIDHI_PROVIDER_ID,
        "created_at": "2026-10-19T00:00:00Z",
        "aoi_name": "Bhoonidhi fixture AOI",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[77.0,12.0],[78.0,12.0],[78.0,13.0],[77.0,13.0],[77.0,12.0]]]
        },
        "query": {"logical_dataset": BHOONIDHI_LOGICAL_DATASET},
        "candidates": [item.clone()],
        "initial_selected_item_ids": [item.item_id.clone()],
        "output_root": output_root.to_string_lossy(),
        "layout": "direct"
    }))
    .expect("frozen Bhoonidhi task");
    DownloadWork { task, item }
}

fn worker(
    resolver: BhoonidhiAssetResolver,
    backend: Arc<CompletingBackend>,
) -> ProductionDownloadWorker {
    let mut registry = AssetResolverRegistry::new();
    registry.register(Arc::new(resolver)).unwrap();
    let runtime = ResolverExecutionRuntime::with_dependencies(
        registry,
        Arc::new(NoCredentials),
        Arc::new(SystemClock),
    );
    ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, backend)
}

fn output_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "insar-bhoonidhi-probe-retry-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// RED before D1d: the first send timeout failed the item as DL005.
#[test]
fn a_bhoonidhi_route_retries_a_timed_out_download_probe_and_transfers() {
    let probe = Arc::new(FlakyProbe {
        failures: 1,
        calls: AtomicUsize::new(0),
    });
    let backend = Arc::new(CompletingBackend::default());
    let worker = worker(
        BhoonidhiAssetResolver::new(Arc::new(TokenProvider), probe.clone()),
        Arc::clone(&backend),
    );
    let root = output_root("timeout-once");
    let started = Instant::now();
    let result = worker.execute(
        work(&root, bhoonidhi_item()),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(
        result.outcome,
        DownloadOutcome::Completed,
        "{:?}",
        result.failure
    );
    assert_eq!(
        probe.calls.load(Ordering::SeqCst),
        2,
        "one retry after the timeout"
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    // The first retry waits the shared bounded backoff (5 s).
    assert!(
        started.elapsed() >= Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_bhoonidhi_route_fails_after_three_timed_out_probes_naming_the_attempts() {
    let probe = Arc::new(FlakyProbe {
        failures: usize::MAX,
        calls: AtomicUsize::new(0),
    });
    let backend = Arc::new(CompletingBackend::default());
    let worker = worker(
        BhoonidhiAssetResolver::new(Arc::new(TokenProvider), probe.clone())
            .with_probe_retry_backoff_for_test(Arc::new(|_, _| Duration::ZERO)),
        Arc::clone(&backend),
    );
    let root = output_root("timeout-always");
    let result = worker.execute(
        work(&root, bhoonidhi_item()),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Failed);
    let failure = result.failure.expect("a failed probe names its failure");
    assert_eq!(failure.code, "DL005");
    assert!(failure.retryable);
    // Downloader D1d: the timeout declares its network cause for the task card.
    assert!(
        failure.reason_zh.contains("operation timed out")
            && failure
                .reason_zh
                .ends_with(" [stage=download_probe attempts=3 source=network]"),
        "{}",
        failure.reason_zh
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 3);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_pause_during_the_probe_retry_wait_interrupts_the_item_instead_of_failing_it() {
    let probe = Arc::new(FlakyProbe {
        failures: usize::MAX,
        calls: AtomicUsize::new(0),
    });
    let backend = Arc::new(CompletingBackend::default());
    let worker = worker(
        BhoonidhiAssetResolver::new(Arc::new(TokenProvider), probe.clone())
            .with_probe_retry_backoff_for_test(Arc::new(|_, _| Duration::from_secs(30))),
        Arc::clone(&backend),
    );
    let root = output_root("pause-in-backoff");
    let control = WorkerControl::default();
    let pauser = control.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        pauser.set_paused_for_test(true);
    });
    let started = Instant::now();
    let result = worker.execute(work(&root, bhoonidhi_item()), control, &mut |_| {});
    thread.join().unwrap();

    assert_eq!(
        result.outcome,
        DownloadOutcome::Interrupted,
        "{:?}",
        result.failure
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    let _ = std::fs::remove_dir_all(&root);
}
