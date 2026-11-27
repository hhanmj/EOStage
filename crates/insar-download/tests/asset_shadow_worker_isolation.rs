use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use insar_core::{AssetAccessScheme, AssetRole, DownloadLayout, FrozenAssetAccess};
use insar_download::{
    run_shadow_then_legacy, AssetResolverRegistry, Clock, LegacyTransferIntent,
    ProductionDownloadWorker, ResolverShadowDiagnostics, ResolverShadowInput,
    ResolverShadowObserver, ResolverShadowRuntime, ShadowAssetClass, ShadowAuthKind,
    ShadowCredentialShapeProvider, ShadowDestinationPolicy, ShadowOutcome,
    TransferIntentFingerprint, TransferOptions, SHADOW_ORBIT_DESCRIPTOR_UNAVAILABLE,
};

struct PanicObserver;

impl ResolverShadowObserver for PanicObserver {
    fn observe(&self, _input: &ResolverShadowInput) {
        panic!("fixture observer panic");
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

struct NoShapes;

impl ShadowCredentialShapeProvider for NoShapes {
    fn auth_kind(&self, _profile: &str) -> Result<ShadowAuthKind, insar_download::ResolveError> {
        Err(insar_download::ResolveError::credentials_missing())
    }
}

struct FixtureShapes;

impl ShadowCredentialShapeProvider for FixtureShapes {
    fn auth_kind(&self, profile: &str) -> Result<ShadowAuthKind, insar_download::ResolveError> {
        match profile {
            "earthdata-default" => Ok(ShadowAuthKind::EarthdataBasic),
            "earthdata-burst-session" => Ok(ShadowAuthKind::EarthdataSessionCookie),
            _ => Err(insar_download::ResolveError::credentials_missing()),
        }
    }
}

struct MutableClock(Arc<AtomicU64>);

impl Clock for MutableClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

struct AdvancingShapes(Arc<AtomicU64>);

impl ShadowCredentialShapeProvider for AdvancingShapes {
    fn auth_kind(&self, _profile: &str) -> Result<ShadowAuthKind, insar_download::ResolveError> {
        self.0.store(25, Ordering::SeqCst);
        Ok(ShadowAuthKind::EarthdataBasic)
    }
}

fn input(asset_class: ShadowAssetClass) -> ResolverShadowInput {
    let legacy = TransferIntentFingerprint::from_legacy(&LegacyTransferIntent {
        provider_id: "asf.catalog".to_string(),
        url: "https://datapool.asf.alaska.edu/SLC/fixture.zip".to_string(),
        file_name: "fixture.zip".to_string(),
        auth_kind: ShadowAuthKind::EarthdataBasic,
        expected_size: Some(5),
        checksum_algorithm: None,
        checksum_value: None,
        destination: PathBuf::from("fixture-output/fixture.zip"),
        destination_layout: DownloadLayout::Direct,
        asset_role: AssetRole::Primary,
    })
    .unwrap();
    ResolverShadowInput::new(
        asset_class,
        None,
        legacy,
        ShadowDestinationPolicy::new(
            PathBuf::from("fixture-output"),
            DownloadLayout::Direct,
            "SENTINEL-1",
            "SLC",
        ),
    )
}

fn sentinel_fixture() -> (
    FrozenAssetAccess,
    LegacyTransferIntent,
    ShadowDestinationPolicy,
) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asset_resolver_shadow/sentinel1_standard.json");
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let legacy = &value["legacy"];
    let policy = &value["policy"];
    (
        serde_json::from_value(value["frozen"].clone()).unwrap(),
        LegacyTransferIntent {
            provider_id: legacy["provider_id"].as_str().unwrap().to_string(),
            url: legacy["url"].as_str().unwrap().to_string(),
            file_name: legacy["file_name"].as_str().unwrap().to_string(),
            auth_kind: ShadowAuthKind::EarthdataBasic,
            expected_size: legacy["expected_size"].as_u64(),
            checksum_algorithm: legacy["checksum_algorithm"].as_str().map(str::to_string),
            checksum_value: legacy["checksum_value"].as_str().map(str::to_string),
            destination: PathBuf::from(legacy["destination"].as_str().unwrap()),
            destination_layout: DownloadLayout::Sentinel1Slc,
            asset_role: AssetRole::Primary,
        },
        ShadowDestinationPolicy::new(
            PathBuf::from(policy["output_root"].as_str().unwrap()),
            DownloadLayout::Sentinel1Slc,
            "SENTINEL-1",
            "SLC",
        ),
    )
}

fn runtime(
    shapes: Arc<dyn ShadowCredentialShapeProvider>,
    clock: Arc<dyn Clock>,
    diagnostics: Arc<ResolverShadowDiagnostics>,
    enabled: impl IntoIterator<Item = ShadowAssetClass>,
) -> ResolverShadowRuntime {
    ResolverShadowRuntime::new(
        AssetResolverRegistry::with_standard_resolvers().unwrap(),
        shapes,
        clock,
        diagnostics,
        enabled,
        Duration::from_millis(25),
    )
    .unwrap()
}

fn execute_once(observer: &dyn ResolverShadowObserver, input: &ResolverShadowInput) {
    let calls = AtomicUsize::new(0);
    run_shadow_then_legacy(Some(observer), Some(input), || {
        calls.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn disabled_and_panicking_observers_leave_one_identical_legacy_transfer() {
    for observer in [None, Some(&PanicObserver as &dyn ResolverShadowObserver)] {
        let transfer_calls = AtomicUsize::new(0);
        let destination = PathBuf::from("fixture-output/fixture.zip");
        let result =
            run_shadow_then_legacy(observer, Some(&input(ShadowAssetClass::LegacyUrl)), || {
                transfer_calls.fetch_add(1, Ordering::SeqCst);
                ("legacy-result", destination.clone())
            });

        assert_eq!(transfer_calls.load(Ordering::SeqCst), 1);
        assert_eq!(result, ("legacy-result", destination));
    }
}

#[test]
fn missing_shadow_input_cannot_skip_or_duplicate_legacy_transfer() {
    let transfer_calls = AtomicUsize::new(0);
    let result = run_shadow_then_legacy(Some(&PanicObserver), None, || {
        transfer_calls.fetch_add(1, Ordering::SeqCst);
        42
    });
    assert_eq!(result, 42);
    assert_eq!(transfer_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn production_worker_shadow_hook_is_default_off_and_explicit_only() {
    let worker = ProductionDownloadWorker::new(TransferOptions::default()).unwrap();
    assert!(!worker.shadow_runtime_enabled());
    let runtime = ResolverShadowRuntime::new(
        AssetResolverRegistry::with_standard_resolvers().unwrap(),
        Arc::new(NoShapes),
        Arc::new(FixedClock),
        Arc::new(ResolverShadowDiagnostics::new(4)),
        [ShadowAssetClass::Sentinel1Standard],
        Duration::from_millis(25),
    )
    .unwrap();
    let worker = worker.with_shadow_runtime(Arc::new(runtime));
    assert!(worker.shadow_runtime_enabled());
}

#[test]
fn runtime_records_parity_mismatch_and_honest_ineligible_codes() {
    let diagnostics = Arc::new(ResolverShadowDiagnostics::new(8));
    let runtime = runtime(
        Arc::new(FixtureShapes),
        Arc::new(FixedClock),
        Arc::clone(&diagnostics),
        [
            ShadowAssetClass::Sentinel1Standard,
            ShadowAssetClass::LegacyUrl,
            ShadowAssetClass::OrbitSceneId,
        ],
    );
    let (frozen, legacy, policy) = sentinel_fixture();
    let parity = ResolverShadowInput::new(
        ShadowAssetClass::Sentinel1Standard,
        Some(frozen.clone()),
        TransferIntentFingerprint::from_legacy(&legacy).unwrap(),
        policy.clone(),
    );
    execute_once(&runtime, &parity);

    let mut mismatched_legacy = legacy;
    mismatched_legacy.expected_size = Some(1);
    let mismatch = ResolverShadowInput::new(
        ShadowAssetClass::Sentinel1Standard,
        Some(frozen),
        TransferIntentFingerprint::from_legacy(&mismatched_legacy).unwrap(),
        policy,
    );
    execute_once(&runtime, &mismatch);
    execute_once(&runtime, &input(ShadowAssetClass::LegacyUrl));
    execute_once(&runtime, &input(ShadowAssetClass::OrbitSceneId));

    let observations = diagnostics.observations();
    assert_eq!(observations[0].outcome, ShadowOutcome::Parity);
    assert_eq!(observations[1].outcome, ShadowOutcome::Mismatch);
    assert_eq!(observations[2].outcome, ShadowOutcome::Ineligible);
    assert_eq!(observations[3].outcome, ShadowOutcome::Ineligible);
    assert_eq!(
        observations[3].resolver_error_code,
        Some(SHADOW_ORBIT_DESCRIPTOR_UNAVAILABLE)
    );
}

#[test]
fn resolver_error_expired_budget_and_precancel_remain_observer_only() {
    let (mut frozen, legacy, policy) = sentinel_fixture();
    frozen.descriptor.scheme = AssetAccessScheme::PublicHttps;
    let diagnostics = Arc::new(ResolverShadowDiagnostics::new(4));
    let normal = runtime(
        Arc::new(FixtureShapes),
        Arc::new(FixedClock),
        Arc::clone(&diagnostics),
        [ShadowAssetClass::Sentinel1Standard],
    );
    let resolver_error = ResolverShadowInput::new(
        ShadowAssetClass::Sentinel1Standard,
        Some(frozen),
        TransferIntentFingerprint::from_legacy(&legacy).unwrap(),
        policy.clone(),
    );
    execute_once(&normal, &resolver_error);

    let time = Arc::new(AtomicU64::new(0));
    let expired_diagnostics = Arc::new(ResolverShadowDiagnostics::new(4));
    let expired = runtime(
        Arc::new(AdvancingShapes(Arc::clone(&time))),
        Arc::new(MutableClock(time)),
        Arc::clone(&expired_diagnostics),
        [ShadowAssetClass::Sentinel1Standard],
    );
    let (frozen, legacy, _) = sentinel_fixture();
    let expired_input = ResolverShadowInput::new(
        ShadowAssetClass::Sentinel1Standard,
        Some(frozen.clone()),
        TransferIntentFingerprint::from_legacy(&legacy).unwrap(),
        policy.clone(),
    );
    execute_once(&expired, &expired_input);

    let cancelled_diagnostics = Arc::new(ResolverShadowDiagnostics::new(4));
    let cancelled = runtime(
        Arc::new(FixtureShapes),
        Arc::new(FixedClock),
        Arc::clone(&cancelled_diagnostics),
        [ShadowAssetClass::Sentinel1Standard],
    );
    let cancelled_input = ResolverShadowInput::new(
        ShadowAssetClass::Sentinel1Standard,
        Some(frozen),
        TransferIntentFingerprint::from_legacy(&legacy).unwrap(),
        policy,
    )
    .with_cancelled(true);
    execute_once(&cancelled, &cancelled_input);

    assert_eq!(
        diagnostics.observations()[0].outcome,
        ShadowOutcome::ResolverError
    );
    assert_eq!(
        expired_diagnostics.observations()[0].resolver_error_code,
        Some("ASSET_RESOLUTION_TIMEOUT")
    );
    assert_eq!(
        cancelled_diagnostics.observations()[0].resolver_error_code,
        Some("ASSET_RESOLUTION_CANCELLED")
    );
}
