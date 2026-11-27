#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use std::fs;
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use std::fs::OpenOptions;
use std::io::{Cursor, Error, ErrorKind, Read};
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use std::path::PathBuf;
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use std::sync::Arc;
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use std::time::Duration;

use insar_application_service::ApplicationCatalogSearchAndFreezeRequest;
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, ApplicationServiceError,
};
use insar_core::CatalogSearchQuery;
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, ContractError, DownloadItemProgress,
    DownloadItemResult, ProviderNetworkPolicy, ProviderResult,
};
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::usgs_m2m::{UsgsM2mApplicationToken, UsgsM2mLoginMaterial};
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use insar_provider::usgs_m2m::{
    UsgsM2mSceneSearchClient, UsgsM2mSceneSearchRouter, UsgsM2mSceneSearchState, UsgsM2mSession,
    UsgsM2mSessionStatus,
};
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use insar_service::LiveAsfSearch;
use serde_json::json;
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
use serde_json::Map;
use zeroize::Zeroizing;

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
const P4A2A_LIVE_CREDENTIAL_PIPE: &str = r"\\.\pipe\insar-studio-usgs-m2m-p4a2a-v1";
const MAX_USERNAME_BYTES: usize = 256;
const MAX_APPLICATION_TOKEN_BYTES: usize = 4 * 1024;
#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
static NEXT_STORE_ID: AtomicU64 = AtomicU64::new(0);

#[test]
#[ignore = "requires fresh explicit P4A-2A USGS credential authorization through the one-time named pipe"]
fn landsat_p4a2a_production_search_freeze_store_restore_live_acceptance() {
    #[cfg(not(all(windows, feature = "usgs-m2m-live-acceptance")))]
    panic!("LANDSAT_P4A2A_LIVE_HARNESS_WINDOWS_AND_FEATURE_REQUIRED");

    #[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
    match run_live_acceptance() {
        Ok(candidate_count) => {
            eprintln!("LANDSAT_P4A2A_LIVE outcome=completed");
            eprintln!("candidate_count={candidate_count}");
            eprintln!("production_router=true");
            eprintln!("store_restore_match=true");
            eprintln!("download_task_count=0");
            eprintln!("logout_completed=true");
        }
        Err(code) => {
            eprintln!("LANDSAT_P4A2A_LIVE outcome=stopped");
            eprintln!("code={code}");
            panic!("LANDSAT_P4A2A_LIVE_STOPPED");
        }
    }
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
fn run_live_acceptance() -> Result<usize, &'static str> {
    with_live_session(|session| {
        let client = Arc::new(UsgsM2mSceneSearchClient::new(session));
        let router = Arc::new(UsgsM2mSceneSearchRouter::new(Arc::clone(&client)));
        if router.state() != UsgsM2mSceneSearchState::LiveAccepted
            || !router.production_enabled()
            || router.network_policy() != ProviderNetworkPolicy::system()
        {
            return Err("LANDSAT_P4A2A_ACCEPTANCE_ROUTE_INVALID");
        }

        let store = TempStore::new();
        let request = live_request()?;
        let service = live_service(&store, router.clone());
        let snapshot = service
            .catalog_search_and_freeze_legacy(&request)
            .map_err(|error| {
                match &error {
                    ApplicationServiceError::Provider(provider) => {
                        eprintln!("provider_code={}", provider.code);
                        eprintln!(
                            "endpoint_class={}",
                            provider.debug_context["endpoint_class"]
                                .as_str()
                                .unwrap_or("application_provider")
                        );
                        eprintln!(
                            "parser_stage={}",
                            provider.debug_context["parser_stage"]
                                .as_str()
                                .unwrap_or("not_applicable")
                        );
                    }
                    _ => {
                        eprintln!("application_code={}", error.code());
                        eprintln!("endpoint_class=application_service");
                    }
                }
                report_redacted_replays(&client);
                "LANDSAT_P4A2A_SEARCH_FREEZE_FAILED"
            })?;
        report_redacted_replays(&client);
        let batch = snapshot
            .restore
            .workbench_batch
            .ok_or("LANDSAT_P4A2A_BATCH_ABSENT")?;
        if batch.candidates.is_empty()
            || !snapshot.restore.tasks.is_empty()
            || batch.candidates.iter().any(|candidate| {
                !candidate.payload.is_empty()
                    || candidate.asset_access.is_some()
                    || candidate.execution_policy.is_some()
                    || candidate.metadata["assets"] != json!([])
            })
        {
            return Err("LANDSAT_P4A2A_FROZEN_BATCH_INVALID");
        }
        let frozen = serde_json::to_string(&batch)
            .map_err(|_| "LANDSAT_P4A2A_BATCH_SERIALIZATION_FAILED")?;
        let lowercase = frozen.to_ascii_lowercase();
        if ["https://", "http://", "download", "resolver_id"]
            .iter()
            .any(|forbidden| lowercase.contains(forbidden))
        {
            return Err("LANDSAT_P4A2A_LOCATOR_LEAK");
        }
        let candidate_count = batch.candidates.len();
        service
            .shutdown()
            .map_err(|_| "LANDSAT_P4A2A_STORE_CLOSE_FAILED")?;

        let reopened = live_service(&store, router);
        let restored = reopened
            .snapshot(true)
            .map_err(|_| "LANDSAT_P4A2A_STORE_REOPEN_FAILED")?;
        let restored_batch = restored
            .restore
            .workbench_batch
            .ok_or("LANDSAT_P4A2A_RESTORED_BATCH_ABSENT")?;
        if serde_json::to_string(&restored_batch)
            .map_err(|_| "LANDSAT_P4A2A_BATCH_SERIALIZATION_FAILED")?
            != frozen
            || !restored.restore.tasks.is_empty()
        {
            return Err("LANDSAT_P4A2A_STORE_RESTORE_MISMATCH");
        }
        reopened
            .shutdown()
            .map_err(|_| "LANDSAT_P4A2A_STORE_CLOSE_FAILED")?;
        Ok(candidate_count)
    })
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
fn report_redacted_replays(client: &UsgsM2mSceneSearchClient) {
    let replays = client.take_live_acceptance_redacted_replays();
    let serialized = serde_json::to_string(&replays).unwrap_or_else(|_| "[]".to_owned());
    eprintln!("redacted_scene_replays={serialized}");
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
fn live_service(store: &TempStore, router: Arc<UsgsM2mSceneSearchRouter>) -> ApplicationService {
    ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&store.0),
        Arc::new(NoAsf),
        router,
        Arc::new(NoDownloadWorker),
    )
}

fn live_request() -> Result<ApplicationCatalogSearchAndFreezeRequest, &'static str> {
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
    .map_err(|_| "LANDSAT_P4A2A_QUERY_INVALID")?;
    search
        .validate()
        .map_err(|_| "LANDSAT_P4A2A_QUERY_INVALID")?;
    Ok(ApplicationCatalogSearchAndFreezeRequest {
        request_id: "landsat-p4a2a-live-search".to_owned(),
        batch_id: "landsat-p4a2a-live-batch".to_owned(),
        created_at: "2026-07-28T00:00:00Z".to_owned(),
        aoi_name: "P4A-2A bounded live acceptance AOI".to_owned(),
        aoi_geojson: search.aoi_geojson.clone(),
        search,
        workbench_page_size: 10,
    })
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
fn with_live_session<T>(
    operation: impl FnOnce(Arc<UsgsM2mSession>) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let material = read_named_pipe_material()?;
    let session = Arc::new(
        UsgsM2mSession::new(ProviderNetworkPolicy::system(), Duration::from_secs(60))
            .map_err(|_| "LANDSAT_P4A2A_SESSION_CONFIG_INVALID")?,
    );
    if let Err(error) = session.login(material, &|| false) {
        eprintln!("provider_code={}", error.code_str());
        eprintln!("endpoint_class={}", error.endpoint().as_str());
        report_request_counts(&session);
        return Err("LANDSAT_P4A2A_LOGIN_FAILED");
    }

    let outcome = operation(Arc::clone(&session));
    let logout = session.logout(&|| false);
    let signed_out = matches!(session.status(), Ok(UsgsM2mSessionStatus::LoggedOut));
    if let Err(error) = logout {
        eprintln!("provider_code={}", error.code_str());
        eprintln!("endpoint_class={}", error.endpoint().as_str());
        report_request_counts(&session);
        return Err("LANDSAT_P4A2A_LOGOUT_FAILED");
    }
    report_request_counts(&session);
    let counts = session.request_counts();
    if counts.login_token != 1
        || counts.dataset_search != 0
        || counts.dataset_filters > 2
        || counts.scene_search > 3
        || counts.logout != 1
    {
        return Err("LANDSAT_P4A2A_REQUEST_BUDGET_EXCEEDED");
    }
    if !signed_out {
        return Err("LANDSAT_P4A2A_LOGOUT_FAILED");
    }
    outcome
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
fn report_request_counts(session: &UsgsM2mSession) {
    let counts = session.request_counts();
    eprintln!("login_token_requests={}", counts.login_token);
    eprintln!("dataset_search_requests={}", counts.dataset_search);
    eprintln!("dataset_filters_requests={}", counts.dataset_filters);
    eprintln!("scene_search_requests={}", counts.scene_search);
    eprintln!("logout_requests={}", counts.logout);
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
fn read_named_pipe_material() -> Result<UsgsM2mLoginMaterial, &'static str> {
    let mut pipe = OpenOptions::new()
        .read(true)
        .open(P4A2A_LIVE_CREDENTIAL_PIPE)
        .map_err(|_| "LANDSAT_P4A2A_CREDENTIAL_PIPE_UNAVAILABLE")?;
    read_login_material(&mut pipe)
}

fn read_login_material(pipe: &mut impl Read) -> Result<UsgsM2mLoginMaterial, &'static str> {
    let mut username = read_bounded_utf8(pipe, MAX_USERNAME_BYTES)?;
    let mut token = read_bounded_utf8(pipe, MAX_APPLICATION_TOKEN_BYTES)?;
    let mut trailing = Zeroizing::new([0_u8; 1]);
    match pipe.read(&mut *trailing) {
        Ok(0) => {}
        Err(error) if error.kind() == ErrorKind::BrokenPipe => {}
        Ok(_) | Err(_) => return Err("LANDSAT_P4A2A_CREDENTIAL_PIPE_INVALID"),
    }
    let token = UsgsM2mApplicationToken::new(std::mem::take(&mut *token))
        .map_err(|_| "LANDSAT_P4A2A_CREDENTIAL_MATERIAL_INVALID")?;
    UsgsM2mLoginMaterial::new(std::mem::take(&mut *username), token)
        .map_err(|_| "LANDSAT_P4A2A_CREDENTIAL_MATERIAL_INVALID")
}

fn read_bounded_utf8(
    pipe: &mut impl Read,
    maximum: usize,
) -> Result<Zeroizing<String>, &'static str> {
    let mut length = Zeroizing::new([0_u8; 4]);
    pipe.read_exact(&mut *length)
        .map_err(|_| "LANDSAT_P4A2A_CREDENTIAL_PIPE_INVALID")?;
    let length = u32::from_le_bytes(*length) as usize;
    if length == 0 || length > maximum {
        return Err("LANDSAT_P4A2A_CREDENTIAL_PIPE_INVALID");
    }
    let mut bytes = Zeroizing::new(vec![0_u8; length]);
    pipe.read_exact(&mut bytes)
        .map_err(|_| "LANDSAT_P4A2A_CREDENTIAL_PIPE_INVALID")?;
    std::str::from_utf8(&bytes)
        .map(str::to_owned)
        .map(Zeroizing::new)
        .map_err(|_| "LANDSAT_P4A2A_CREDENTIAL_PIPE_INVALID")
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
struct TempStore(PathBuf);

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
impl TempStore {
    fn new() -> Self {
        let id = NEXT_STORE_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "insar-landsat-p4a2a-live-{}-{id}.sqlite3",
            std::process::id()
        )))
    }
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
impl Drop for TempStore {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
struct NoAsf;

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
impl LiveAsfSearch for NoAsf {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
    }
}

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
struct NoDownloadWorker;

#[cfg(all(windows, feature = "usgs-m2m-live-acceptance"))]
impl DownloadWorker for NoDownloadWorker {
    fn execute(
        &self,
        _work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        panic!("P4A-2A scene-only freeze must not create download work")
    }
}

#[test]
fn p4a2a_live_query_and_pipe_material_remain_closed_offline() {
    live_request().expect("the live query is fixed and valid");
    let username = b"synthetic-p4a2a-user";
    let token = b"synthetic-p4a2a-application-token";
    let mut frame = Vec::new();
    frame.extend_from_slice(&(username.len() as u32).to_le_bytes());
    frame.extend_from_slice(username);
    frame.extend_from_slice(&(token.len() as u32).to_le_bytes());
    frame.extend_from_slice(token);
    let material = read_login_material(&mut frame.as_slice()).unwrap();
    let debug = format!("{material:?}");
    assert!(!debug.contains("synthetic-p4a2a-user"));
    assert!(!debug.contains("synthetic-p4a2a-application-token"));
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
fn p4a2a_pipe_accepts_windows_broken_pipe_only_after_complete_frames() {
    let username = b"synthetic-p4a2a-user";
    let token = b"synthetic-p4a2a-application-token";
    let mut frame = Vec::new();
    frame.extend_from_slice(&(username.len() as u32).to_le_bytes());
    frame.extend_from_slice(username);
    frame.extend_from_slice(&(token.len() as u32).to_le_bytes());
    frame.extend_from_slice(token);

    let material = read_login_material(&mut BrokenPipeAtEof(Cursor::new(frame))).unwrap();
    let debug = format!("{material:?}");
    assert!(!debug.contains("synthetic-p4a2a-user"));
    assert!(!debug.contains("synthetic-p4a2a-application-token"));

    let truncated = vec![1_u8, 0, 0, 0];
    assert!(read_login_material(&mut BrokenPipeAtEof(Cursor::new(truncated))).is_err());
}
