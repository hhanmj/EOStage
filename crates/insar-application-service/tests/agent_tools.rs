//! Each Agent catalog tool reaches the one desktop ApplicationService through
//! `HostPort<AgentApplication>` against a temporary Store and offline fixtures.
use insar_agent::{executor::ToolPort, host::HostPort};
use insar_application_service::{AgentApplication, ApplicationService, ApplicationServiceConfig};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogSearchQuery, CatalogSearchResponse,
    DownloadItemProgress, DownloadItemResult, DownloadOutcome, FrozenSearchBatch, ProviderResult,
    SearchWorkbenchState, SnapshotItem, StoreSearchBatchRequest,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
    time::Duration,
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct TempStore(PathBuf);
impl TempStore {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "agent-tools-{}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for TempStore {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
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
        panic!("optical route must not call ASF")
    }
}
/// Holds each item until the task is paused or cancelled; never touches disk.
struct HeldWorker;
impl DownloadWorker for HeldWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        while !control.is_cancelled() && !control.is_paused() {
            std::thread::sleep(Duration::from_millis(5));
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: if control.is_cancelled() {
                DownloadOutcome::Cancelled
            } else {
                DownloadOutcome::Interrupted
            },
            destination: work.task.output_root.join(work.item.display_name),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-10-24T00:00:00Z".to_owned(),
        }
    }
}
struct Catalog(CatalogSearchResponse);
impl LiveCatalogSearch for Catalog {
    fn search_catalog(
        &self,
        _: &CatalogSearchQuery,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        Ok(self.0.clone())
    }
}
fn aoi() -> Map<String, Value> {
    json!({"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]})
        .as_object()
        .unwrap()
        .clone()
}
fn fixture() -> CatalogSearchResponse {
    serde_json::from_value(json!({
        "collection":{"logical_dataset":"optical.hls.l30.v2","provider_id":"microsoft.planetary-computer","native_collection_id":"hls2-l30","collection_profile_id":"planetary-computer.hls2-l30.scene.v1","identity_rule_id":"planetary-computer.hls2.identity.v1","filename_rule_id":"planetary-computer.hls2.search-only.v1","delivery_model":"archive_first","production_enabled":true},
        "products":[{"provider_id":"microsoft.planetary-computer","product_id":"HLS.L30.T49RDP.2024177T030756.v2.0","display_name":"HLS.L30.T49RDP.2024177T030756.v2.0","source_kind":"landsat","product_kind":"optical_scene","acquisition_datetime":"2024-06-25T03:07:56.017810Z","footprint_geojson":{"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},"assets":[],"properties":{"platform":"landsat_8","cloud_cover":79.0},"payload":{}}],
        // Owner decision 2026-11-21: streaming optical sources (including HLS L30) continue up to 2,000 after first 250.
        "stats":{"requested_limit":2000,"matched_count":1,"request_count":1,"transport_part_count":1,"fetched_record_count":1,"mapped_record_count":1,"profile_filtered_record_count":0,"exact_duplicate_record_count":0,"unique_provider_product_count":1,"exact_matched_count":1,"returned_count":1,"remote_exhausted":true,"working_set_satisfied":true,"elapsed_ms":1,"truncated":false},"item_errors":[]
    })).unwrap()
}
fn seeded_service(db: &TempStore) -> Arc<ApplicationService> {
    let mut store = TaskStore::open(&db.0).unwrap();
    store.save_search_batch(&StoreSearchBatchRequest {
        batch: FrozenSearchBatch { schema_version:1,batch_id:"public-batch".to_owned(),
            provider_id:"microsoft.planetary-computer".to_owned(),created_at:"2026-01-01T00:00:00Z".to_owned(),
            aoi_name:"fixture".to_owned(),aoi_geojson:Some(aoi()),
            query:json!({"logical_dataset":"optical.landsat.c2.l2","native_collection_id":"landsat-c2-l2"}).as_object().unwrap().clone(),
            search_stats:None,candidates:vec![SnapshotItem {item_id:"public-scene".to_owned(),provider_id:"microsoft.planetary-computer".to_owned(),display_name:"scene".to_owned(),acquisition_datetime:None,footprint_geojson:None,metadata:Map::new(),payload:Map::new(),asset_access:None,execution_policy:None}]},
        workbench:SearchWorkbenchState{batch_id:"public-batch".to_owned(),checked_item_ids:vec![],page_index:0,page_size:24,revision:0},
        activate_workbench:true,
    }).unwrap();
    drop(store);
    Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::new(Catalog(fixture())),
        Arc::new(HeldWorker),
    ))
}
fn read(port: &mut HostPort<AgentApplication>, command: &str, input: Value) -> Value {
    port.read(command, &input)
        .unwrap_or_else(|error| panic!("{command}: {error:?}"))
        .summary
}

#[test]
fn agent_read_tools_reach_the_desktop_service_offline() {
    let db = TempStore::new();
    let service = seeded_service(&db);
    let mut port = HostPort::new(AgentApplication(Arc::clone(&service)));
    assert_eq!(
        read(&mut port, "application_service_readiness", json!({})),
        json!({"category":"ready"})
    );
    assert!(read(&mut port, "application_provider_capabilities", json!({})).is_object());
    assert_eq!(
        read(&mut port, "application_working_aoi", json!({}))["state"],
        "none"
    );
    let sources = read(&mut port, "application_list_sources", json!({}));
    assert!(sources
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["resource_id"] == "science.hls.l30.v2"));
    assert_eq!(
        read(&mut port, "application_task_overview", json!({})),
        json!({"queue":[],"history":[]})
    );
    let batch = read(
        &mut port,
        "application_batch_page",
        json!({"batch_id":"public-batch","revision":0,"page":0,"page_size":10}),
    );
    assert_eq!(batch["total_count"], 1);
    assert_eq!(batch["items"][0]["item_id"], "public-scene");
    assert!(batch["items"][0].get("payload").is_none());
    let workbench = read(
        &mut port,
        "application_workbench_page",
        json!({"page_index":0,"page_size":10}),
    );
    assert_eq!(workbench["batch_id"], "public-batch");
    assert_eq!(
        read(
            &mut port,
            "application_search_progress",
            json!({"request_id":"never-started"})
        ),
        Value::Null
    );
    service.shutdown().unwrap();
}

#[test]
fn disclosed_search_start_freezes_through_the_unified_search() {
    let db = TempStore::new();
    let service = seeded_service(&db);
    let mut port = HostPort::new(AgentApplication(Arc::clone(&service)));
    let request = json!({
        "request_id":"agent-search-1","resource_id":"science.hls.l30.v2","aoi":aoi(),
        "aoi_name":"测试区","time_range":null,
        "filters":{"spacecraft":["landsat_8"],"cloud_cover_lte":80.0}
    });
    assert!(
        port.read("application_search_start", &request).is_err(),
        "a provider request is never a silent read"
    );
    let started = port
        .submit("application_search_start", &request)
        .unwrap()
        .summary;
    assert_eq!(started["request_id"], "agent-search-1");
    let terminal = (0..150)
        .find_map(|_| {
            let progress = read(
                &mut port,
                "application_search_progress",
                json!({"request_id":"agent-search-1"}),
            );
            if progress["terminal"] == true {
                Some(progress)
            } else {
                std::thread::sleep(Duration::from_millis(20));
                None
            }
        })
        .expect("terminal progress");
    assert_eq!(terminal["stage"], "done");
    assert_eq!(terminal["items_kept"], 1);
    service.shutdown().unwrap();
}

#[test]
fn agent_task_tools_create_control_and_read_back_by_frozen_event_id() {
    let db = TempStore::new();
    let output = std::env::temp_dir().join(format!(
        "agent-tools-out-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let service = seeded_service(&db);
    let mut port = HostPort::new(AgentApplication(Arc::clone(&service)));
    let created = port
        .submit(
            "application_create_task",
            &json!({"batch_id":"public-batch","revision":0,"item_ids":["public-scene"],
                "options":{"output_root":output,"layout":"direct","concurrency":1},"event_id":"agent-create-1"}),
        )
        .unwrap()
        .summary;
    let task_id = created["task_id"].as_str().unwrap().to_owned();
    assert_eq!(created["created"], true);
    let page = read(
        &mut port,
        "application_download_task_page",
        json!({"task_id":task_id,"page_index":0,"page_size":10}),
    );
    assert_eq!(page["task_id"], task_id.as_str());
    let events = read(
        &mut port,
        "application_download_task_event_page",
        json!({"task_id":task_id,"page_size":10}),
    );
    assert_eq!(events["task_id"], task_id.as_str());
    let overview = read(&mut port, "application_task_overview", json!({}));
    assert!(overview.to_string().contains(&task_id));

    // The scheduler may advance the task between reads; retry on the next revision.
    let mut controlled = None;
    for attempt in 0..40 {
        let current = read(
            &mut port,
            "application_download_task_page",
            json!({"task_id":task_id,"page_index":0,"page_size":1}),
        );
        let revision = current["revision"].as_u64().unwrap();
        let event_id = format!("agent-pause-{attempt}");
        if let Ok(result) = port.submit(
            "application_task_action",
            &json!({"task_id":task_id,"revision":revision,"action":"pause","event_id":event_id}),
        ) {
            controlled = Some((result.summary, event_id));
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (paused, event_id) = controlled.expect("pause reached the Store");
    assert_eq!(paused["task_id"], task_id.as_str());
    let readback = port.task_state(&task_id, &event_id).unwrap().unwrap();
    assert_eq!(readback["last_event_id"], event_id.as_str());
    assert!(port.task_state(&task_id, "never-sent").unwrap().is_none());

    let mut cancelled = false;
    for attempt in 0..40 {
        let current = read(
            &mut port,
            "application_download_task_page",
            json!({"task_id":task_id,"page_index":0,"page_size":1}),
        );
        let request = json!({"task_id":task_id,"expected_revision":current["revision"],
            "command":{"kind":"cancel_task","item_ids":[]},
            "event_id":format!("agent-cancel-{attempt}"),"occurred_at":"2026-10-24T00:00:00Z"});
        if port
            .submit("application_update_download_task", &request)
            .is_ok()
        {
            cancelled = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(cancelled, "approved task control reached the Store");
    service.shutdown().unwrap();
    let _ = fs::remove_dir_all(output);
}
