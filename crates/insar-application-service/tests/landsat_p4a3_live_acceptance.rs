//! Explicit, one-shot production acceptance for one Landsat C2 L2 full bundle.
//!
//! Ordinary suites compile but ignore the live test. The live path accepts login
//! material only through one fixed current-user named pipe, shares one in-memory
//! M2M session across search, option discovery, resolution, and transfer, and
//! emits only bounded non-secret evidence.

use std::io::{Cursor, Error, ErrorKind, Read};

use insar_provider::usgs_m2m::{UsgsM2mApplicationToken, UsgsM2mLoginMaterial};
use zeroize::Zeroizing;

const MAX_USERNAME_BYTES: usize = 256;
const MAX_APPLICATION_TOKEN_BYTES: usize = 4 * 1024;

#[test]
#[ignore = "requires explicit P4A-3 authorization through the one-time current-user named pipe"]
fn landsat_p4a3_full_bundle_store_resolver_transfer_live_acceptance() {
    #[cfg(not(all(windows, feature = "usgs-m2m-live-acceptance")))]
    panic!("LANDSAT_P4A3_LIVE_HARNESS_WINDOWS_AND_FEATURE_REQUIRED");

    #[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
    report_live_result(live::run());
}

#[test]
#[ignore = "requires explicit Owner authorization and native saved USGS credentials"]
fn landsat_p4a3_saved_desktop_full_bundle_live_acceptance() {
    #[cfg(not(all(windows, feature = "usgs-m2m-live-acceptance")))]
    panic!("LANDSAT_P4A3_LIVE_HARNESS_WINDOWS_AND_FEATURE_REQUIRED");

    #[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
    {
        eprintln!("credential_source=native_saved");
        report_live_result(live::run_with_saved_desktop_credentials());
    }
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
fn report_live_result(result: Result<live::Report, &'static str>) {
    match result {
        Ok(report) => {
            eprintln!("LANDSAT_P4A3_LIVE outcome=completed");
            eprintln!("candidate_count={}", report.candidate_count);
            eprintln!("bundle_count=1");
            eprintln!("bundle_role=full_bundle");
            eprintln!("host_class=usgs_landsat_download_v1");
            eprintln!("expected_bytes={}", report.expected_bytes);
            eprintln!("actual_bytes={}", report.actual_bytes);
            eprintln!("checksum_algorithm={}", report.checksum_algorithm);
            eprintln!("checksum_verified={}", report.checksum_verified);
            eprintln!("size_verified=true");
            eprintln!("part_observed={}", report.part_observed);
            eprintln!("atomic_final=true");
            eprintln!("lifecycle_active={}", report.lifecycle_active);
            eprintln!("lifecycle_completed={}", report.lifecycle_completed);
            eprintln!("store_restore_completed=true");
            eprintln!("logout_completed=true");
        }
        Err(code) => {
            eprintln!("LANDSAT_P4A3_LIVE outcome=stopped");
            eprintln!("code={code}");
            panic!("LANDSAT_P4A3_LIVE_STOPPED");
        }
    }
}

fn read_login_material(pipe: &mut impl Read) -> Result<UsgsM2mLoginMaterial, &'static str> {
    let mut username = read_bounded_utf8(pipe, MAX_USERNAME_BYTES)?;
    let mut token = read_bounded_utf8(pipe, MAX_APPLICATION_TOKEN_BYTES)?;
    let mut trailing = Zeroizing::new([0_u8; 1]);
    match pipe.read(&mut *trailing) {
        Ok(0) => {}
        Err(error) if error.kind() == ErrorKind::BrokenPipe => {}
        Ok(_) | Err(_) => return Err("LANDSAT_P4A3_CREDENTIAL_PIPE_INVALID"),
    }
    let token = UsgsM2mApplicationToken::new(std::mem::take(&mut *token))
        .map_err(|_| "LANDSAT_P4A3_CREDENTIAL_MATERIAL_INVALID")?;
    UsgsM2mLoginMaterial::new(std::mem::take(&mut *username), token)
        .map_err(|_| "LANDSAT_P4A3_CREDENTIAL_MATERIAL_INVALID")
}

fn read_bounded_utf8(
    pipe: &mut impl Read,
    maximum: usize,
) -> Result<Zeroizing<String>, &'static str> {
    let mut length = Zeroizing::new([0_u8; 4]);
    pipe.read_exact(&mut *length)
        .map_err(|_| "LANDSAT_P4A3_CREDENTIAL_PIPE_INVALID")?;
    let length = u32::from_le_bytes(*length) as usize;
    if length == 0 || length > maximum {
        return Err("LANDSAT_P4A3_CREDENTIAL_PIPE_INVALID");
    }
    let mut bytes = Zeroizing::new(vec![0_u8; length]);
    pipe.read_exact(&mut bytes)
        .map_err(|_| "LANDSAT_P4A3_CREDENTIAL_PIPE_INVALID")?;
    std::str::from_utf8(&bytes)
        .map(str::to_owned)
        .map(Zeroizing::new)
        .map_err(|_| "LANDSAT_P4A3_CREDENTIAL_PIPE_INVALID")
}

#[test]
fn p4a3_pipe_framing_is_bounded_and_secret_debug_is_redacted() {
    let username = b"synthetic-p4a3-user";
    let token = b"synthetic-p4a3-application-token";
    let mut frame = Vec::new();
    frame.extend_from_slice(&(username.len() as u32).to_le_bytes());
    frame.extend_from_slice(username);
    frame.extend_from_slice(&(token.len() as u32).to_le_bytes());
    frame.extend_from_slice(token);
    let material = read_login_material(&mut frame.as_slice()).unwrap();
    let debug = format!("{material:?}");
    assert!(!debug.contains("synthetic-p4a3-user"));
    assert!(!debug.contains("synthetic-p4a3-application-token"));

    let mut oversized = Vec::new();
    oversized.extend_from_slice(&((MAX_USERNAME_BYTES + 1) as u32).to_le_bytes());
    assert!(read_login_material(&mut oversized.as_slice()).is_err());
}

struct BrokenPipeAtEof(Cursor<Vec<u8>>);

impl Read for BrokenPipeAtEof {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.0.position() == self.0.get_ref().len() as u64 {
            Err(Error::from(ErrorKind::BrokenPipe))
        } else {
            self.0.read(buffer)
        }
    }
}

#[test]
fn p4a3_pipe_accepts_windows_broken_pipe_only_after_both_complete_frames() {
    let username = b"synthetic-p4a3-user";
    let token = b"synthetic-p4a3-application-token";
    let mut frame = Vec::new();
    frame.extend_from_slice(&(username.len() as u32).to_le_bytes());
    frame.extend_from_slice(username);
    frame.extend_from_slice(&(token.len() as u32).to_le_bytes());
    frame.extend_from_slice(token);
    let material = read_login_material(&mut BrokenPipeAtEof(Cursor::new(frame))).unwrap();
    let debug = format!("{material:?}");
    assert!(!debug.contains("synthetic-p4a3-user"));
    assert!(!debug.contains("synthetic-p4a3-application-token"));

    let truncated = vec![1_u8, 0, 0, 0];
    assert!(read_login_material(&mut BrokenPipeAtEof(Cursor::new(truncated))).is_err());
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
mod live {
    use std::ffi::OsStr;
    use std::fs::{self, OpenOptions};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use insar_application_service::{
        ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
        ApplicationServiceError, LandsatCreateBundleTaskRequest, LiveLandsatBundleOptions,
    };
    use insar_core::{
        AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogSearchQuery, ContractError,
        DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadItemStatus,
        DownloadOutcome, ProviderNetworkPolicy, ProviderResult,
    };
    use insar_download::{
        part_path, resolve_desktop_usgs_login, verify_file_checksum, DownloadWork, DownloadWorker,
        ExpectedChecksum, ProductionDownloadWorker, TransferOptions, WorkerControl,
    };
    use insar_provider::usgs_m2m::{
        UsgsM2mDownloadClient, UsgsM2mError, UsgsM2mFullBundleOption, UsgsM2mSceneSearchClient,
        UsgsM2mSceneSearchRouter, UsgsM2mSceneSearchState, UsgsM2mSession, UsgsM2mSessionStatus,
    };
    use insar_service::LiveAsfSearch;
    use serde_json::{json, Map};
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    use super::{read_login_material, UsgsM2mApplicationToken, UsgsM2mLoginMaterial};

    const CREDENTIAL_PIPE: &str = r"\\.\pipe\insar-studio-usgs-m2m-p4a3-v1";
    const TASK_ID: &str = "landsat-p4a3-live-task";
    const MAX_BUNDLE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
    const FREE_SPACE_FLOOR_MARGIN: u64 = 512 * 1024 * 1024;
    const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
    static NEXT_WORKSPACE_ID: AtomicU64 = AtomicU64::new(0);

    pub(super) struct Report {
        pub candidate_count: usize,
        pub expected_bytes: u64,
        pub actual_bytes: u64,
        pub checksum_algorithm: String,
        pub checksum_verified: bool,
        pub part_observed: bool,
        pub lifecycle_active: bool,
        pub lifecycle_completed: bool,
    }

    struct TempWorkspace {
        root: PathBuf,
        store: PathBuf,
        output: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Result<Self, &'static str> {
            let id = NEXT_WORKSPACE_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "insar-landsat-p4a3-live-{}-{id}",
                std::process::id()
            ));
            let output = root.join("output");
            fs::create_dir_all(&output).map_err(|_| "LANDSAT_P4A3_WORKSPACE_UNAVAILABLE")?;
            Ok(Self {
                store: root.join("task-store.sqlite3"),
                root,
                output,
            })
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let temp = std::env::temp_dir();
            if self.root.starts_with(&temp)
                && self
                    .root
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.starts_with("insar-landsat-p4a3-live-"))
            {
                let _ = fs::remove_dir_all(&self.root);
            }
        }
    }

    #[derive(Clone, Copy)]
    enum GateDecision {
        Release,
        Reject,
    }

    struct TransferGate {
        decision: Mutex<Option<GateDecision>>,
        changed: Condvar,
    }

    impl TransferGate {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                decision: Mutex::new(None),
                changed: Condvar::new(),
            })
        }

        fn decide(&self, decision: GateDecision) {
            if let Ok(mut state) = self.decision.lock() {
                *state = Some(decision);
                self.changed.notify_all();
            }
        }

        fn wait(&self, control: &WorkerControl) -> GateDecision {
            let Ok(mut state) = self.decision.lock() else {
                return GateDecision::Reject;
            };
            loop {
                if control.is_cancelled() {
                    return GateDecision::Reject;
                }
                if let Some(decision) = *state {
                    return decision;
                }
                let Ok((next, _)) = self.changed.wait_timeout(state, Duration::from_millis(100))
                else {
                    return GateDecision::Reject;
                };
                state = next;
            }
        }
    }

    struct GatedWorker {
        inner: Arc<ProductionDownloadWorker>,
        gate: Arc<TransferGate>,
    }

    impl DownloadWorker for GatedWorker {
        fn execute(
            &self,
            work: DownloadWork,
            control: WorkerControl,
            progress: &mut dyn FnMut(DownloadItemProgress),
        ) -> DownloadItemResult {
            match self.gate.wait(&control) {
                GateDecision::Release => self.inner.execute(work, control, progress),
                GateDecision::Reject => DownloadItemResult {
                    task_id: work.task.task_id,
                    item_id: work.item.item_id,
                    outcome: if control.is_cancelled() {
                        DownloadOutcome::Cancelled
                    } else {
                        DownloadOutcome::Failed
                    },
                    destination: work.task.output_root,
                    bytes_written: 0,
                    attempt_count: 0,
                    failure: Some(DownloadFailure {
                        code: "LANDSAT_P4A3_PREFLIGHT_REJECTED".to_owned(),
                        reason_zh: "Landsat live 验收预检未通过，未开始传输。".to_owned(),
                        retryable: false,
                    }),
                    finished_at: "2026-07-29T00:00:00Z".to_owned(),
                },
            }
        }
    }

    struct RecordingOptions {
        client: UsgsM2mDownloadClient,
        calls: AtomicU64,
        option: Mutex<Option<UsgsM2mFullBundleOption>>,
    }

    impl RecordingOptions {
        fn new(session: Arc<UsgsM2mSession>) -> Arc<Self> {
            Arc::new(Self {
                client: UsgsM2mDownloadClient::new(session),
                calls: AtomicU64::new(0),
                option: Mutex::new(None),
            })
        }

        fn recorded_once(&self) -> Option<UsgsM2mFullBundleOption> {
            if self.calls.load(Ordering::Acquire) != 1 {
                return None;
            }
            self.option.lock().ok()?.clone()
        }
    }

    impl LiveLandsatBundleOptions for RecordingOptions {
        fn full_bundle_option(
            &self,
            dataset_alias: &str,
            entity_id: &str,
            display_id: &str,
            cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<UsgsM2mFullBundleOption, UsgsM2mError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            let option =
                self.client
                    .full_bundle_option(dataset_alias, entity_id, display_id, cancelled)?;
            if let Ok(mut recorded) = self.option.lock() {
                *recorded = Some(option.clone());
            }
            Ok(option)
        }
    }

    struct NoAsf;

    impl LiveAsfSearch for NoAsf {
        fn search(
            &self,
            _request: &AsfRemoteSearchRequest,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<AsfRemoteSearchResponse> {
            Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
        }
    }

    struct Preflight {
        expected_file_name: String,
        expected_bytes: u64,
        checksum_algorithm: String,
        checksum: Option<ExpectedChecksum>,
    }

    struct TransferEvidence {
        result: DownloadItemResult,
        final_path: PathBuf,
        part_observed: bool,
        lifecycle_active: bool,
        lifecycle_completed: bool,
    }

    pub(super) fn run() -> Result<Report, &'static str> {
        let material = match read_named_pipe_material() {
            Ok(material) => material,
            Err(code) => {
                eprintln!("endpoint_class=credential_pipe");
                report_zero_request_counts();
                return Err(code);
            }
        };
        run_with_material(material, ProviderNetworkPolicy::system())
    }

    pub(super) fn run_with_saved_desktop_credentials() -> Result<Report, &'static str> {
        let (mut username, mut token) = resolve_desktop_usgs_login()
            .map_err(|_| "LANDSAT_P4A3_SAVED_CREDENTIAL_UNAVAILABLE")?;
        let token = UsgsM2mApplicationToken::new(std::mem::take(&mut *token))
            .map_err(|_| "LANDSAT_P4A3_CREDENTIAL_MATERIAL_INVALID")?;
        let material = UsgsM2mLoginMaterial::new(std::mem::take(&mut *username), token)
            .map_err(|_| "LANDSAT_P4A3_CREDENTIAL_MATERIAL_INVALID")?;
        run_with_material(material, ProviderNetworkPolicy::direct())
    }

    fn run_with_material(
        material: UsgsM2mLoginMaterial,
        network_policy: ProviderNetworkPolicy,
    ) -> Result<Report, &'static str> {
        eprintln!("network_route={:?}", network_policy.route());
        let session = Arc::new(
            UsgsM2mSession::new(network_policy, Duration::from_secs(60))
                .map_err(|_| "LANDSAT_P4A3_SESSION_CONFIG_INVALID")?,
        );
        if let Err(error) = session.login(material, &|| false) {
            eprintln!("provider_code={}", error.code_str());
            eprintln!("endpoint_class={}", error.endpoint().as_str());
            report_request_counts(&session);
            return Err("LANDSAT_P4A3_LOGIN_FAILED");
        }

        let outcome = match session.permissions(&|| false) {
            Ok(permissions) => {
                eprintln!("permission_user={}", permissions.has_user());
                eprintln!("permission_download={}", permissions.has_download());
                eprintln!("permission_order={}", permissions.has_order());
                if permissions.has_download() {
                    run_authenticated(Arc::clone(&session))
                } else {
                    Err("LANDSAT_P4A3_DOWNLOAD_PERMISSION_MISSING")
                }
            }
            Err(error) => {
                report_provider_error(&error);
                Err("LANDSAT_P4A3_PERMISSIONS_FAILED")
            }
        };
        let logout = session.logout(&|| false);
        let logged_out = matches!(session.status(), Ok(UsgsM2mSessionStatus::LoggedOut));
        if let Err(error) = &logout {
            eprintln!("provider_code={}", error.code_str());
            eprintln!("endpoint_class={}", error.endpoint().as_str());
        }
        report_request_counts(&session);
        let counts = session.request_counts();
        if logout.is_err() || !logged_out {
            return Err("LANDSAT_P4A3_LOGOUT_FAILED");
        }
        if counts.login_token > 1
            || counts.permissions > 1
            || counts.dataset_search != 0
            || counts.dataset_filters > 2
            || counts.scene_search > 3
            || counts.download_options > 1
            || counts.download_request > 2
            || counts.download_retrieve > 180
            || counts.logout > 1
        {
            return Err("LANDSAT_P4A3_REQUEST_BUDGET_EXCEEDED");
        }
        if outcome.is_ok()
            && (counts.login_token != 1
                || counts.permissions != 1
                || !(1..=2).contains(&counts.dataset_filters)
                || !(1..=3).contains(&counts.scene_search)
                || counts.download_options != 1
                || !(1..=2).contains(&counts.download_request)
                || counts.logout != 1)
        {
            return Err("LANDSAT_P4A3_SUCCESS_EVIDENCE_INCOMPLETE");
        }
        outcome
    }

    fn run_authenticated(session: Arc<UsgsM2mSession>) -> Result<Report, &'static str> {
        let workspace = TempWorkspace::new()?;
        let catalogue_client = Arc::new(UsgsM2mSceneSearchClient::new(Arc::clone(&session)));
        let catalogue = Arc::new(UsgsM2mSceneSearchRouter::new(catalogue_client));
        if catalogue.state() != UsgsM2mSceneSearchState::LiveAccepted
            || !catalogue.production_enabled()
        {
            return Err("LANDSAT_P4A3_PRODUCTION_ROUTE_INVALID");
        }
        let options = RecordingOptions::new(Arc::clone(&session));
        let production_worker = Arc::new(
            ProductionDownloadWorker::new_with_usgs_session(
                TransferOptions::default(),
                Arc::clone(&session),
            )
            .map_err(|_| "LANDSAT_P4A3_WORKER_UNAVAILABLE")?,
        );
        if !production_worker.shares_usgs_session(&session) {
            return Err("LANDSAT_P4A3_SESSION_NOT_SHARED");
        }
        let gate = TransferGate::new();
        let worker: Arc<dyn DownloadWorker> = Arc::new(GatedWorker {
            inner: production_worker,
            gate: Arc::clone(&gate),
        });
        let service =
            ApplicationService::with_catalogue_production_business_runtime_and_landsat_delivery(
                ApplicationServiceConfig::business_store(&workspace.store),
                Arc::new(NoAsf),
                catalogue,
                options.clone(),
                worker,
            );
        let outcome = (|| {
            let request = live_search_request()?;
            let frozen = service
                .catalog_search_and_freeze_legacy(&request)
                .map_err(|error| {
                    eprintln!("application_code={}", error.code());
                    "LANDSAT_P4A3_SEARCH_FAILED"
                })?;
            let source_batch = frozen
                .restore
                .workbench_batch
                .ok_or("LANDSAT_P4A3_SOURCE_BATCH_ABSENT")?;
            let selected = source_batch
                .candidates
                .first()
                .cloned()
                .ok_or("LANDSAT_P4A3_SCENE_UNAVAILABLE")?;
            if selected.asset_access.is_some() || selected.execution_policy.is_some() {
                return Err("LANDSAT_P4A3_SOURCE_BATCH_INVALID");
            }
            let candidate_count = source_batch.candidates.len();
            service
                .create_landsat_bundle_task_legacy(&LandsatCreateBundleTaskRequest {
                    request_id: "landsat-p4a3-live-bundle".to_owned(),
                    source_batch_id: source_batch.batch_id,
                    item_id: selected.item_id.clone(),
                    task_id: TASK_ID.to_owned(),
                    output_root: workspace.output.clone(),
                    layout: insar_core::DownloadLayout::Direct,
                    concurrency: 1,
                    created_at: "2026-07-29T00:00:01Z".to_owned(),
                })
                .map_err(|error| {
                    eprintln!("application_code={}", error.code());
                    if let ApplicationServiceError::UsgsM2m(provider_error) = &error {
                        report_provider_error(provider_error);
                    }
                    "LANDSAT_P4A3_TASK_CREATE_FAILED"
                })?;

            let preflight = options
                .recorded_once()
                .ok_or("LANDSAT_P4A3_OPTION_DISCOVERY_COUNT_INVALID")
                .and_then(|option| preflight(&workspace.output, option));
            let preflight = match preflight {
                Ok(preflight) => preflight,
                Err(code) => {
                    gate.decide(GateDecision::Reject);
                    return Err(code);
                }
            };
            gate.decide(GateDecision::Release);

            let transfer = wait_for_transfer(&service, &selected.item_id, &workspace.output)?;
            verify_final(&preflight, &transfer)?;
            service
                .shutdown()
                .map_err(|_| "LANDSAT_P4A3_STORE_CLOSE_FAILED")?;

            let reopened =
                ApplicationService::new(ApplicationServiceConfig::business_store(&workspace.store));
            let restored = reopened
                .download_snapshot(TASK_ID)
                .map_err(|_| "LANDSAT_P4A3_STORE_REOPEN_FAILED")?;
            let restored_completed = restored.results.iter().any(|result| {
                result.item_id == selected.item_id
                    && result.outcome == DownloadOutcome::Completed
                    && result.bytes_written == transfer.result.bytes_written
            });
            let close_reopened = reopened.shutdown();
            if !restored_completed
                || close_reopened.is_err()
                || part_path(&transfer.final_path).exists()
                || !transfer.final_path.is_file()
            {
                return Err("LANDSAT_P4A3_STORE_RESTORE_FAILED");
            }

            Ok(Report {
                candidate_count,
                expected_bytes: preflight.expected_bytes,
                actual_bytes: transfer.result.bytes_written,
                checksum_algorithm: preflight.checksum_algorithm,
                checksum_verified: preflight.checksum.is_some(),
                part_observed: transfer.part_observed,
                lifecycle_active: transfer.lifecycle_active,
                lifecycle_completed: transfer.lifecycle_completed,
            })
        })();
        if !service.is_shutting_down() {
            gate.decide(GateDecision::Reject);
            let _ = service.shutdown();
        }
        outcome
    }

    fn preflight(
        output: &Path,
        option: UsgsM2mFullBundleOption,
    ) -> Result<Preflight, &'static str> {
        let expected_bytes = option
            .expected_size_bytes
            .filter(|size| (1..=MAX_BUNDLE_BYTES).contains(size))
            .ok_or("LANDSAT_P4A3_EXPECTED_SIZE_REQUIRED")?;
        let expected_file_name = option.expected_file_name.clone();
        let direct_path = output.join(&expected_file_name);
        if direct_path.exists() || part_path(&direct_path).exists() {
            return Err("LANDSAT_P4A3_DESTINATION_OCCUPIED");
        }
        let margin = (expected_bytes / 5).max(FREE_SPACE_FLOOR_MARGIN);
        let required = expected_bytes
            .checked_add(margin)
            .ok_or("LANDSAT_P4A3_DISK_SPACE_INSUFFICIENT")?;
        if available_disk_space(output)? < required {
            return Err("LANDSAT_P4A3_DISK_SPACE_INSUFFICIENT");
        }
        let checksum = match (
            option.checksum_algorithm.as_deref(),
            option.checksum_value.as_deref(),
        ) {
            (Some(algorithm), Some(value)) => Some(
                ExpectedChecksum::new(algorithm, value)
                    .map_err(|_| "LANDSAT_P4A3_CHECKSUM_INVALID")?,
            ),
            (None, None) => None,
            _ => return Err("LANDSAT_P4A3_CHECKSUM_INVALID"),
        };
        let checksum_algorithm = option
            .checksum_algorithm
            .unwrap_or_else(|| "none_size_authority".to_owned());
        Ok(Preflight {
            expected_file_name,
            expected_bytes,
            checksum_algorithm,
            checksum,
        })
    }

    fn available_disk_space(path: &Path) -> Result<u64, &'static str> {
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let mut available = 0_u64;
        let mut total = 0_u64;
        let mut free = 0_u64;
        let result =
            unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut free) };
        (result != 0)
            .then_some(available)
            .ok_or("LANDSAT_P4A3_DISK_SPACE_UNAVAILABLE")
    }

    fn wait_for_transfer(
        service: &ApplicationService,
        item_id: &str,
        output_root: &Path,
    ) -> Result<TransferEvidence, &'static str> {
        let deadline = Instant::now() + TRANSFER_TIMEOUT;
        let mut part_observed = false;
        let mut lifecycle_active = false;
        let mut lifecycle_completed = false;
        loop {
            part_observed |= contains_part_file(output_root);
            let snapshot = service
                .download_snapshot(TASK_ID)
                .map_err(|_| "LANDSAT_P4A3_TASK_SNAPSHOT_FAILED")?;
            for progress in snapshot
                .progress
                .iter()
                .filter(|progress| progress.item_id == item_id)
            {
                lifecycle_active |= progress.status == DownloadItemStatus::Active;
                lifecycle_completed |= progress.status == DownloadItemStatus::Completed;
            }
            if let Some(result) = snapshot
                .results
                .iter()
                .find(|result| result.item_id == item_id)
            {
                lifecycle_completed |= result.outcome == DownloadOutcome::Completed;
                if result.outcome != DownloadOutcome::Completed {
                    eprintln!("transfer_outcome={:?}", result.outcome);
                    eprintln!("transfer_bytes_written={}", result.bytes_written);
                    if let Some(failure) = &result.failure {
                        eprintln!("transfer_failure_code={}", failure.code);
                        eprintln!("transfer_failure_retryable={}", failure.retryable);
                        eprintln!("transfer_failure_reason={}", failure.reason_zh);
                    }
                    return Err(
                        match result.failure.as_ref().map(|failure| failure.code.as_str()) {
                            Some("DL001") => "LANDSAT_P4A3_TRANSFER_DL001",
                            Some("DL002") => "LANDSAT_P4A3_TRANSFER_DL002",
                            Some("DL003") => "LANDSAT_P4A3_TRANSFER_DL003",
                            Some("DL004") => "LANDSAT_P4A3_TRANSFER_DL004",
                            Some("DL005") => "LANDSAT_P4A3_TRANSFER_DL005",
                            Some(_) => "LANDSAT_P4A3_TRANSFER_OTHER",
                            None => "LANDSAT_P4A3_TRANSFER_NO_FAILURE_DETAIL",
                        },
                    );
                }
                let final_path = result.destination.clone();
                if !part_observed || !final_path.is_file() {
                    return Err("LANDSAT_P4A3_PART_OR_FINAL_NOT_OBSERVED");
                }
                return Ok(TransferEvidence {
                    result: result.clone(),
                    final_path,
                    part_observed,
                    lifecycle_active,
                    lifecycle_completed,
                });
            }
            if Instant::now() >= deadline {
                return Err("LANDSAT_P4A3_TRANSFER_TIMEOUT");
            }
            thread::sleep(Duration::from_millis(250));
        }
    }

    fn contains_part_file(root: &Path) -> bool {
        let Ok(entries) = fs::read_dir(root) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            if path.is_dir() {
                contains_part_file(&path)
            } else {
                path.file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.ends_with(".part"))
            }
        })
    }

    fn verify_final(
        preflight: &Preflight,
        transfer: &TransferEvidence,
    ) -> Result<(), &'static str> {
        let actual = fs::metadata(&transfer.final_path)
            .map_err(|_| "LANDSAT_P4A3_FINAL_FILE_MISSING")?
            .len();
        if actual == 0
            || actual != transfer.result.bytes_written
            || transfer.final_path.file_name().and_then(OsStr::to_str)
                != Some(preflight.expected_file_name.as_str())
            || part_path(&transfer.final_path).exists()
        {
            return Err("LANDSAT_P4A3_SIZE_OR_PROMOTION_INVALID");
        }
        if let Some(checksum) = preflight.checksum.as_ref() {
            verify_file_checksum(&transfer.final_path, checksum)
                .map_err(|_| "LANDSAT_P4A3_CHECKSUM_MISMATCH")?;
        }
        Ok(())
    }

    fn live_search_request() -> Result<ApplicationCatalogSearchAndFreezeRequest, &'static str> {
        let search: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": "optical.landsat.c2.l2",
            "provider_id": "usgs.eros.m2m",
            "native_collection_id": "landsat_ot_c2_l2",
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[
                    [-75.446, 36.3793],
                    [-76.9196, 36.3793],
                    [-76.9196, 35.5713],
                    [-75.446, 35.5713],
                    [-75.446, 36.3793]
                ]]
            },
            "result_limit": 10,
            "filters": {
                "datetime": {
                    "start": "2023-03-24T00:00:00Z",
                    "end": "2023-03-27T00:00:00Z"
                },
                "cloud_cover_lte": 100.0,
                "spacecraft": ["landsat_9"],
                "processing_levels": ["l2sp", "l2sr"]
            }
        }))
        .map_err(|_| "LANDSAT_P4A3_QUERY_INVALID")?;
        search
            .validate()
            .map_err(|_| "LANDSAT_P4A3_QUERY_INVALID")?;
        Ok(ApplicationCatalogSearchAndFreezeRequest {
            request_id: "landsat-p4a3-live-search".to_owned(),
            batch_id: "landsat-p4a3-live-source-batch".to_owned(),
            created_at: "2026-07-29T00:00:00Z".to_owned(),
            aoi_name: "P4A-3 bounded live acceptance AOI".to_owned(),
            aoi_geojson: search.aoi_geojson.clone(),
            search,
            workbench_page_size: 10,
        })
    }

    fn read_named_pipe_material() -> Result<UsgsM2mLoginMaterial, &'static str> {
        let mut pipe = OpenOptions::new()
            .read(true)
            .open(CREDENTIAL_PIPE)
            .map_err(|_| "LANDSAT_P4A3_CREDENTIAL_PIPE_UNAVAILABLE")?;
        read_login_material(&mut pipe)
    }

    fn report_request_counts(session: &UsgsM2mSession) {
        let counts = session.request_counts();
        eprintln!("login_token_requests={}", counts.login_token);
        eprintln!("permissions_requests={}", counts.permissions);
        eprintln!("dataset_search_requests={}", counts.dataset_search);
        eprintln!("dataset_filters_requests={}", counts.dataset_filters);
        eprintln!("scene_search_requests={}", counts.scene_search);
        eprintln!("download_options_requests={}", counts.download_options);
        eprintln!("download_request_requests={}", counts.download_request);
        eprintln!("download_retrieve_requests={}", counts.download_retrieve);
        eprintln!("logout_requests={}", counts.logout);
    }

    fn report_zero_request_counts() {
        eprintln!("login_token_requests=0");
        eprintln!("permissions_requests=0");
        eprintln!("dataset_search_requests=0");
        eprintln!("dataset_filters_requests=0");
        eprintln!("scene_search_requests=0");
        eprintln!("download_options_requests=0");
        eprintln!("download_request_requests=0");
        eprintln!("download_retrieve_requests=0");
        eprintln!("logout_requests=0");
    }

    fn report_provider_error(error: &UsgsM2mError) {
        eprintln!("provider_code={}", error.code_str());
        eprintln!("endpoint_class={}", error.endpoint().as_str());
        match error.http_status() {
            Some(status) => eprintln!("http_status={status}"),
            None => eprintln!("http_status=none"),
        }
    }
}
