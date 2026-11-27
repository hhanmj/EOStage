//! ASF removed legacy mission admission contract: ERS, JERS-1 and
//! RADARSAT-1 were removed from ASF's searchable interfaces (Owner
//! evidence 2026-10-31), so their Registry resources are retired and a
//! legacy search for them is rejected with the typed Chinese removal
//! reason before any batch or task row exists. The download-side
//! `ASF_DATASET_ACCESS_RESTRICTED` reclassification remains defense in
//! depth for batches frozen before the retirement.

use std::sync::atomic::{AtomicU64, Ordering};

use insar_application_service::{
    ApplicationSarSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, DownloadItemProgress, DownloadItemResult,
    DownloadOutcome, ProviderResult,
};
use insar_service::LiveAsfSearch;
use serde_json::json;

static NEXT_STORE: AtomicU64 = AtomicU64::new(0);

struct NeverSearch;

impl LiveAsfSearch for NeverSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        panic!("retired legacy missions must never reach the provider transport")
    }

    fn search_sar(
        &self,
        request: &insar_core::SarRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<insar_core::SarRemoteSearchResponse> {
        panic!(
            "retired legacy missions must never reach the provider transport: {}",
            request.query.dataset
        );
    }
}

fn service(label: &str) -> ApplicationService {
    let sequence = NEXT_STORE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "insar-asf-retired-admission-{label}-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("temp store root");
    let store = root.join("tasks.sqlite3");
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&store),
        std::sync::Arc::new(NeverSearch),
        std::sync::Arc::new(NoopWorker),
    );
    std::fs::remove_dir_all(&root).ok();
    service
}

struct NoopWorker;

impl insar_download::DownloadWorker for NoopWorker {
    fn execute(
        &self,
        work: insar_download::DownloadWork,
        _control: insar_download::WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-10-31T00:00:01Z".to_owned(),
        }
    }
}

fn legacy_freeze_request(dataset: &str, request_id: &str) -> ApplicationSarSearchAndFreezeRequest {
    ApplicationSarSearchAndFreezeRequest {
        request_id: request_id.to_owned(),
        search: serde_json::from_value(json!({
            "query": {
                "provider": "asf",
                "dataset": dataset,
                "start": "1991-01-01T00:00:00Z",
                "end": "2013-12-31T23:59:59Z",
                "max_results": 1
            }
        }))
        .unwrap(),
        batch_id: format!("{request_id}-batch"),
        created_at: "2026-10-31T00:00:00Z".to_owned(),
        aoi_name: "Fairbanks".to_owned(),
        aoi_geojson: None,
        page_size: 1,
    }
}

#[test]
fn retired_legacy_mission_searches_are_rejected_with_the_typed_reason() {
    let service = service("retired-search");
    for dataset in ["ERS", "JERS-1", "RADARSAT-1"] {
        let error = service
            .sar_search_and_freeze(&legacy_freeze_request(dataset, "retired-search"))
            .expect_err("retired legacy mission search must be rejected");
        let reason = match &error {
            insar_application_service::ApplicationServiceError::Provider(provider) => {
                assert_eq!(provider.code, "ASF_DATASET_REMOVED", "{dataset}");
                provider.user_message.clone()
            }
            other => {
                panic!("expected the typed ASF removal rejection for {dataset}, got {other:?}")
            }
        };
        assert_eq!(reason, "ASF 已从检索接口下线该数据集", "{dataset}");
    }

    let restored = service.restore(true).unwrap();
    assert!(
        restored.workbench_batch.is_none(),
        "no retired legacy mission batch was frozen"
    );

    service.shutdown().unwrap();
}

#[test]
fn unified_search_rejects_retired_resources_with_the_same_reason() {
    let service = std::sync::Arc::new(service("retired-unified"));
    for (resource_id, request_id) in [
        ("science.ers-1-2.sar.v1", "unified-ers"),
        ("science.jers-1.sar.v1", "unified-jers"),
        ("science.radarsat-1.sar.v1", "unified-radarsat"),
    ] {
        let request = insar_application_service::UnifiedSearchRequest {
            request_id: request_id.to_owned(),
            resource_id: resource_id.to_owned(),
            aoi: serde_json::json!({
                "type": "Polygon",
                "coordinates": [[
                    [-147.82, 64.78],
                    [-147.52, 64.78],
                    [-147.52, 64.88],
                    [-147.82, 64.88],
                    [-147.82, 64.78]
                ]]
            })
            .as_object()
            .expect("AOI is an object")
            .clone(),
            aoi_name: "Fairbanks".to_owned(),
            time_range: None,
            filters: serde_json::Map::new(),
        };
        let error = service
            .search_start(request)
            .expect_err("unified search must reject the retired resource");
        match error {
            insar_application_service::ApplicationServiceError::InvalidRequest { reason } => {
                assert_eq!(reason, "ASF 已从检索接口下线该数据集", "{resource_id}");
            }
            other => {
                panic!("expected the typed removal rejection for {resource_id}, got {other:?}")
            }
        }
    }
    service.shutdown().unwrap();
}
