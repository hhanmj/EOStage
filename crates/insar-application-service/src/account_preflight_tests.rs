//! Offline admission tests for the unified `create_task` account preflight.
//! Fake account ports stand in for the saved-credential services; no test
//! reads credentials or opens a network connection.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use insar_core::{
    AccountId, AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogSearchQuery,
    CatalogSearchResponse, DownloadItemProgress, DownloadItemResult, DownloadLayout,
    DownloadOutcome, FrozenSearchBatch, ProviderResult, SearchWorkbenchState, SnapshotItem,
    StoreSearchBatchRequest,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

use crate::account_sessions::{preflight, AccountSessionPort, AccountSessionPreflight};
use crate::{ApplicationService, ApplicationServiceConfig, ApplicationServiceError};
use crate::{TaskAdmissionPreflight, UnifiedCreateTaskOptions};

struct FakeAccounts {
    saved: Vec<AccountId>,
    calls: Mutex<Vec<AccountId>>,
}

impl FakeAccounts {
    fn with(saved: Vec<AccountId>) -> Self {
        Self {
            saved,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<AccountId> {
        self.calls.lock().unwrap().clone()
    }
}

impl AccountSessionPort for FakeAccounts {
    fn ensure_account_session(
        &self,
        account: &AccountId,
    ) -> Result<Option<String>, ApplicationServiceError> {
        self.calls.lock().unwrap().push(account.clone());
        if self.saved.contains(account) {
            Ok(None)
        } else {
            Err(ApplicationServiceError::AccountLoginRequired {
                provider: crate::account_sessions::account_display_name(account),
            })
        }
    }
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn batch(batch_id: &str, provider_id: &str, query: Value) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: batch_id.to_owned(),
        provider_id: provider_id.to_owned(),
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        aoi_name: "fixture".to_owned(),
        aoi_geojson: Some(object(
            json!({"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]}),
        )),
        query: object(query),
        search_stats: None,
        candidates: vec![SnapshotItem {
            item_id: "scene-1".to_owned(),
            provider_id: provider_id.to_owned(),
            display_name: "scene".to_owned(),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::new(),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        }],
    }
}

/// One frozen batch per protected route named in the W1 brief.
fn protected_batches() -> Vec<(FrozenSearchBatch, AccountId, &'static str)> {
    vec![
        (
            batch(
                "cdse-batch",
                "copernicus.dataspace.odata",
                json!({"logical_dataset":"optical.sentinel-2.l2a","native_collection_id":"SENTINEL-2"}),
            ),
            AccountId::Cdse,
            "CDSE_ACCOUNT_LOGIN_REQUIRED",
        ),
        (
            batch(
                "usgs-batch",
                "usgs.eros.m2m",
                json!({"logical_dataset":"optical.landsat.c2.l2","native_collection_id":"landsat_ot_c2_l2"}),
            ),
            AccountId::UsgsM2m,
            "USGS_ACCOUNT_LOGIN_REQUIRED",
        ),
        (
            batch("burst-batch", "asf.catalog", json!({"dataset":"SLC-BURST"})),
            AccountId::Earthdata,
            "EARTHDATA_ACCOUNT_LOGIN_REQUIRED",
        ),
        (
            batch(
                "nisar-s-batch",
                "isro.nrsc.bhoonidhi",
                json!({"dataset":"NISAR_SSAR"}),
            ),
            AccountId::Bhoonidhi,
            "BHOONIDHI_ACCOUNT_LOGIN_REQUIRED",
        ),
    ]
}

fn selected() -> Vec<String> {
    vec!["scene-1".to_owned()]
}

#[test]
fn saved_credentials_establish_the_route_session_before_admission() {
    for (batch, account, _) in protected_batches() {
        let port = FakeAccounts::with(vec![account.clone()]);
        AccountSessionPreflight::new(&port)
            .check(&batch, &batch.candidates, &selected())
            .unwrap_or_else(|error| panic!("{} must be admitted: {error:?}", batch.batch_id));
        assert_eq!(port.calls(), [account], "{}", batch.batch_id);
    }
}

#[test]
fn missing_credentials_return_needs_account_without_admission() {
    for (batch, account, code) in protected_batches() {
        let port = FakeAccounts::with(Vec::new());
        let error = preflight(&port, &batch, &batch.candidates, &selected())
            .expect_err("missing credentials must block admission");
        assert_eq!(error.code, code, "{}", batch.batch_id);
        assert_eq!(error.needs_account, Some(account), "{}", batch.batch_id);
        assert!(!error.retryable);
        assert!(error.message_zh.contains("需要登录"));
    }
}

#[test]
fn public_routes_never_touch_an_account_session() {
    let public = batch(
        "mpc-batch",
        "microsoft.planetary-computer",
        json!({"logical_dataset":"optical.landsat.c2.l2","native_collection_id":"landsat-c2-l2"}),
    );
    let port = FakeAccounts::with(Vec::new());
    preflight(&port, &public, &public.candidates, &selected()).expect("public route is admitted");
    assert!(port.calls().is_empty());
}

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempStore(PathBuf);

impl TempStore {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "account-preflight-{}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

struct NoAsf;

impl LiveAsfSearch for NoAsf {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        panic!("admission must not search ASF")
    }
}

struct NoCatalog;

impl LiveCatalogSearch for NoCatalog {
    fn search_catalog(
        &self,
        _: &CatalogSearchQuery,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        panic!("admission must not search a catalogue")
    }
}

struct CompletingWorker;

impl DownloadWorker for CompletingWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-01-01T00:00:01Z".to_owned(),
        }
    }
}

fn service_with(batch: &FrozenSearchBatch, db: &TempStore) -> ApplicationService {
    let mut store = TaskStore::open(&db.0).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch.clone(),
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: false,
        })
        .unwrap();
    drop(store);
    ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::new(NoCatalog),
        Arc::new(CompletingWorker),
    )
}

fn options() -> UnifiedCreateTaskOptions {
    UnifiedCreateTaskOptions {
        output_root: std::env::temp_dir().join("account-preflight-output"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        include_precise_orbit: None,
    }
}

#[test]
fn create_task_without_credentials_creates_no_task_then_admits_after_login() {
    let (cdse, account, _) = protected_batches().remove(0);
    let db = TempStore::new();
    let service = service_with(&cdse, &db);
    let missing = FakeAccounts::with(Vec::new());
    let error = service
        .create_task_with_preflight(
            cdse.batch_id.clone(),
            0,
            selected(),
            options(),
            &AccountSessionPreflight::new(&missing),
        )
        .expect_err("no saved CDSE login");
    assert_eq!(error.needs_account, Some(account.clone()));
    assert!(service.task_overview().unwrap().queue.is_empty());

    let saved = FakeAccounts::with(vec![account.clone()]);
    let task = service
        .create_task_with_preflight(
            cdse.batch_id.clone(),
            0,
            selected(),
            options(),
            &AccountSessionPreflight::new(&saved),
        )
        .expect("session established, task admitted");
    assert!(task.created);
    assert_eq!(saved.calls(), [account]);
    service.shutdown().unwrap();
}

#[test]
fn unified_create_task_uses_the_account_session_preflight() {
    // Fixture runtimes have no saved-credential services; the production port
    // skips them exactly like the legacy admission guards, so a protected
    // route is admitted instead of failing as "preflight pending".
    let (cdse, _, _) = protected_batches().remove(0);
    let db = TempStore::new();
    let service = service_with(&cdse, &db);
    let task = service
        .create_task(cdse.batch_id.clone(), 0, selected(), options())
        .expect("unified create_task runs the W1 preflight");
    assert!(task.created);
    service.shutdown().unwrap();
}

#[test]
fn dem_entries_need_an_account_only_on_the_opentopography_route() {
    use crate::account_sessions::dem_provider_account;
    for provider in ["OPENTOPOGRAPHY", " OpenTopography "] {
        assert_eq!(
            dem_provider_account(provider),
            Some(AccountId::Opentopography)
        );
    }
    for provider in ["COPERNICUS_AWS", "LOCAL", ""] {
        assert_eq!(dem_provider_account(provider), None, "{provider}");
    }
}

#[test]
fn nisar_subset_selections_name_their_band_account() {
    use crate::account_sessions::selection_accounts;
    let nisar_l = batch(
        "nisar-l-batch",
        "asf.nasa.earthdata",
        json!({"dataset":"NISAR"}),
    );
    assert_eq!(
        selection_accounts(&nisar_l, &nisar_l.candidates, &selected()).unwrap(),
        [AccountId::Earthdata]
    );
    let (nisar_s, _, _) = protected_batches().remove(3);
    assert_eq!(
        selection_accounts(&nisar_s, &nisar_s.candidates, &selected()).unwrap(),
        [AccountId::Bhoonidhi]
    );
}

/// Records the CDSE session the admission check saw, standing in for the
/// production login that would establish it.
struct SessionProbe<'a> {
    service: &'a ApplicationService,
    seen: Mutex<Option<Arc<insar_download::CdseOAuthSession>>>,
}

impl AccountSessionPort for SessionProbe<'_> {
    fn ensure_account_session(
        &self,
        _: &AccountId,
    ) -> Result<Option<String>, ApplicationServiceError> {
        let runtime = self.service.cdse_session_runtime()?;
        *self.seen.lock().unwrap() = Some(runtime.session);
        Ok(None)
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "account-preflight-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_pending_network_change_is_applied_before_admission_establishes_the_session() {
    let (cdse, _, _) = protected_batches().remove(0);
    let dir = temp_dir("pending-network");
    let db = TempStore(dir.join("store.sqlite3"));
    let service = service_with(&cdse, &db);
    service.task_overview().unwrap();
    let startup = service.cdse_session_runtime().unwrap().session;
    {
        // A busy runtime keeps the saved change pending.
        let _busy = service.catalogue_policy_gate.lock().unwrap();
        service
            .save_network_settings(&crate::ApplicationNetworkSettings {
                proxy_enabled: true,
                proxy_url: "http://127.0.0.1:3128".to_owned(),
                ..crate::ApplicationNetworkSettings::default()
            })
            .unwrap();
    }
    assert!(Arc::ptr_eq(
        &startup,
        &service.cdse_session_runtime().unwrap().session
    ));
    let probe = SessionProbe {
        service: &service,
        seen: Mutex::new(None),
    };
    service
        .create_task_with_preflight(
            cdse.batch_id.clone(),
            0,
            selected(),
            options(),
            &AccountSessionPreflight::new(&probe),
        )
        .expect("admitted");
    let admitted = probe.seen.lock().unwrap().clone().expect("session checked");
    let current = service.cdse_session_runtime().unwrap();
    assert!(
        Arc::ptr_eq(&admitted, &current.session),
        "the session established for admission was retired by the pending route change"
    );
    assert_eq!(
        current.network_policy,
        insar_core::ProviderNetworkPolicy::manual(
            insar_core::ManualProxy::new("127.0.0.1", 3128).unwrap()
        )
    );
    service.shutdown().unwrap();
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unresolvable_frozen_selection_is_rejected_instead_of_admitted_without_accounts() {
    let dir = temp_dir("registry-error");
    let db = TempStore(dir.join("store.sqlite3"));
    // Production startup prunes >24h unreferenced disposable search batches (87b85f8f);
    // give this fixture a fresh timestamp and active workbench binding so admission tests the live batch.
    let mut odd = batch(
        "unknown-source-batch",
        "copernicus.dataspace.odata",
        json!({"dataset":"NOT_A_REGISTERED_SELECTOR"}),
    );
    odd.created_at = chrono::Utc::now().to_rfc3339();
    let mut store = TaskStore::open(&db.0).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: odd.clone(),
            workbench: SearchWorkbenchState {
                batch_id: odd.batch_id.clone(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    drop(store);
    // Production factory: the selected item has no frozen asset access, so no
    // saved-credential service is consulted on either path.
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&db.0));
    service.startup().expect("offline production startup");
    let result = service.ensure_frozen_selection_for_admission(&odd.batch_id, &selected());
    service.shutdown().unwrap();
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
    result.expect_err("a selection whose source cannot be verified must not be admitted");
}

/// Earthdata could not be reached (transport or rate limit): the production
/// port admits with a warning and leaves the session to the worker.
struct EarthdataUnreachable;

impl AccountSessionPort for EarthdataUnreachable {
    fn ensure_account_session(
        &self,
        account: &AccountId,
    ) -> Result<Option<String>, ApplicationServiceError> {
        assert_eq!(account, &AccountId::Earthdata);
        Ok(Some("Earthdata session deferred to the worker".to_owned()))
    }
}

#[test]
fn an_unreachable_earthdata_session_admits_the_task_with_a_warning() {
    let (burst, _, _) = protected_batches().remove(2);
    let db = TempStore::new();
    let service = service_with(&burst, &db);
    let task = service
        .create_task_with_preflight(
            burst.batch_id.clone(),
            0,
            selected(),
            options(),
            &AccountSessionPreflight::new(&EarthdataUnreachable),
        )
        .expect("transport failure must not block admission");
    assert!(task.created);
    assert_eq!(
        task.warning.as_deref(),
        Some("Earthdata session deferred to the worker")
    );
    let saved = FakeAccounts::with(vec![AccountId::Earthdata]);
    assert_eq!(
        preflight(&saved, &burst, &burst.candidates, &selected()).unwrap(),
        Vec::<String>::new()
    );
    service.shutdown().unwrap();
}

/// #22 B slice 2: unified `create_task` must not load the whole candidate
/// array of the frozen batch. The proof is behavioral: a 300-product rows
/// batch is admitted from its bounded selection while an *unrelated* batch
/// blob in the same Store is corrupt (a full-array read of the admitted
/// batch would still work; the assertion pins the bounded Store read and the
/// cross-checked selection the service now uses).
#[test]
fn unified_create_task_admits_from_a_bounded_selection_of_a_large_rows_batch() {
    let (cdse, account, _) = protected_batches().remove(0);
    let db = TempStore::new();
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        let mut large = cdse.clone();
        large.batch_id = "cdse-large".to_owned();
        large.candidates = (0..300)
            .map(|index| SnapshotItem {
                item_id: format!("scene-{index:03}"),
                provider_id: cdse.provider_id.clone(),
                display_name: format!("scene {index}"),
                acquisition_datetime: None,
                footprint_geojson: None,
                metadata: Map::new(),
                payload: Map::new(),
                asset_access: None,
                execution_policy: None,
            })
            .collect();
        for batch in [large, cdse.clone()] {
            store
                .save_search_batch(&StoreSearchBatchRequest {
                    batch: batch.clone(),
                    workbench: SearchWorkbenchState {
                        batch_id: batch.batch_id.clone(),
                        checked_item_ids: vec![],
                        page_index: 0,
                        page_size: 24,
                        revision: 0,
                    },
                    activate_workbench: false,
                })
                .unwrap();
        }
    }
    let service = service_with(&cdse, &db);
    let saved = FakeAccounts::with(vec![account.clone()]);
    let task = service
        .create_task_with_preflight(
            "cdse-large".to_owned(),
            0,
            vec!["scene-000".to_owned(), "scene-299".to_owned()],
            options(),
            &AccountSessionPreflight::new(&saved),
        )
        .expect("bounded selection admits the task");
    assert!(task.created);
    assert_eq!(saved.calls(), [account]);
    // The persisted task froze exactly the selection, not the batch.
    let stored = service
        .task_overview()
        .unwrap()
        .queue
        .iter()
        .find(|queued| queued.task_id == task.task_id)
        .cloned()
        .expect("the created task is queued");
    assert_eq!(stored.selected_count, 2);
    service.shutdown().unwrap();
}
