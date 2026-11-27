//! Smoke-only loopback catalogue runtime for the packaged UI smoke test.
//!
//! The packaged two-phase AOI smoke needs a real search that crosses the full
//! ApplicationService boundary (Store AOI precondition -> Dispatcher ->
//! provider port -> response validation -> Store freeze) without public
//! network access. This module provides a loopback catalogue provider that
//! records the exact `CatalogSearchQuery` it receives, so the smoke can prove
//! the provider actually saw the geometry restored from the persisted AOI.
//!
//! Production construction never uses this runtime; it is only reachable from
//! the Tauri host's UI-smoke mode.

use std::sync::{Arc, Mutex};

use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogCollectionDescriptor,
    CatalogDeliveryModel, CatalogSearchQuery, CatalogSearchResponse, CatalogSearchStats,
    ContractError, DownloadItemProgress, DownloadItemResult, DownloadOutcome, ProviderResult,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::asf::{map_alos2_umm_granule, CmrGranuleEnvelope};
use insar_provider::catalog::{
    CollectionProfileRegistry, ALOS2_FILENAME_RULE_ID, ALOS2_IDENTITY_RULE_ID,
    ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use serde_json::{Map, Value};

use crate::{ApplicationService, ApplicationServiceConfig};

const ALOS2_GRANULE_FIXTURE: &str =
    include_str!("../../../docs/fixtures/asf-alos2-p1/granule-wbd-cmr-umm.json");

/// Dummy ASF search port: the UI smoke only exercises the typed catalogue
/// route, so the legacy ASF search boundary always fails closed.
struct UiSmokeAsfSearch;

impl LiveAsfSearch for UiSmokeAsfSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "ui smoke fixture", Map::new()).unwrap())
    }
}

/// Offline loopback download worker used by the packaged smoke. It completes
/// only formal DEM/Orbit tasks and never performs a provider request.
struct UiSmokeWorker;

impl DownloadWorker for UiSmokeWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let operation_kind = work
            .task
            .operation
            .as_ref()
            .map(|operation| operation.kind.as_str())
            .unwrap_or_default();
        assert!(
            matches!(operation_kind, "dem" | "orbit"),
            "ui smoke only executes formal DEM/Orbit loopback tasks"
        );
        progress(DownloadItemProgress {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            status: insar_core::DownloadItemStatus::Active,
            bytes_completed: 0,
            total_bytes: Some(0),
            bytes_per_second: 0.0,
            attempt: 1,
            updated_at: "2026-08-12T00:00:01Z".to_owned(),
        });
        std::thread::sleep(std::time::Duration::from_millis(80));
        if control.is_cancelled() {
            return DownloadItemResult {
                task_id: work.task.task_id,
                item_id: work.item.item_id,
                outcome: DownloadOutcome::Cancelled,
                destination: work.task.output_root,
                bytes_written: 0,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-08-12T00:00:02Z".to_owned(),
            };
        }
        while control.is_paused() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-08-12T00:00:03Z".to_owned(),
        }
    }
}

/// Loopback ALOS-2 catalogue provider that records every query it receives.
pub struct UiSmokeLoopbackCatalog {
    response: CatalogSearchResponse,
    recorded: Mutex<Option<CatalogSearchQuery>>,
}

impl UiSmokeLoopbackCatalog {
    /// Build the loopback provider from the frozen ALOS-2 CMR granule fixture.
    pub fn new() -> Result<Arc<Self>, String> {
        let value: Value = serde_json::from_str(ALOS2_GRANULE_FIXTURE)
            .map_err(|error| format!("alos2 fixture parse: {error}"))?;
        let envelope = CmrGranuleEnvelope::try_from(&value)
            .map_err(|error| format!("alos2 envelope: {error}"))?;
        let profile = CollectionProfileRegistry
            .get(ALOS2_PROFILE_ID)
            .expect("ALOS-2 profile exists in the registry");
        let product = map_alos2_umm_granule(&envelope, profile)
            .map_err(|error| format!("alos2 mapping: {error}"))?;
        let response = CatalogSearchResponse {
            collection: CatalogCollectionDescriptor {
                logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
                provider_id: ALOS2_PROVIDER_ID.to_owned(),
                native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
                collection_profile_id: ALOS2_PROFILE_ID.to_owned(),
                identity_rule_id: ALOS2_IDENTITY_RULE_ID.to_owned(),
                filename_rule_id: ALOS2_FILENAME_RULE_ID.to_owned(),
                delivery_model: CatalogDeliveryModel::ArchiveFirst,
                production_enabled: false,
            },
            products: vec![product],
            stats: CatalogSearchStats {
                requested_limit: Some(25),
                matched_count: Some(1),
                request_count: 1,
                transport_part_count: 1,
                fetched_record_count: 1,
                mapped_record_count: 1,
                profile_filtered_record_count: 0,
                exact_duplicate_record_count: 0,
                unique_provider_product_count: 1,
                exact_matched_count: 1,
                returned_count: 1,
                remote_exhausted: true,
                working_set_satisfied: true,
                elapsed_ms: 0,
                truncated: false,
            },
            next_cursor: None,
            item_errors: Vec::new(),
        };
        Ok(Arc::new(Self {
            response,
            recorded: Mutex::new(None),
        }))
    }

    /// The exact query the provider received, if any.
    pub fn recorded_query(&self) -> Option<CatalogSearchQuery> {
        self.recorded.lock().ok().and_then(|guard| guard.clone())
    }
}

impl LiveCatalogSearch for UiSmokeLoopbackCatalog {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let mut recorded = self.recorded.lock().map_err(|_| {
            ContractError::new("UI_SMOKE_STATE_POISONED", "ui smoke state", Map::new()).unwrap()
        })?;
        *recorded = Some(query.clone());
        drop(recorded);
        Ok(self.response.clone())
    }
}

/// Build an ApplicationService whose typed catalogue route is the loopback
/// provider. Only reachable from the Tauri UI-smoke mode.
pub fn with_ui_smoke_loopback_runtime(
    config: ApplicationServiceConfig,
    catalog: Arc<UiSmokeLoopbackCatalog>,
) -> ApplicationService {
    ApplicationService::with_catalogue_fixture_business_runtime(
        config,
        Arc::new(UiSmokeAsfSearch),
        catalog,
        Arc::new(UiSmokeWorker),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ApplicationBindRegionAoiRequest, ApplicationCatalogSearchAndFreezeRequest,
        WorkspaceNamedNodeRequest, WorkspaceOpenRequest,
    };
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn sample_geometry() -> Map<String, Value> {
        json!({
            "type": "Polygon",
            "coordinates": [[[116.1, 39.7], [116.5, 39.7], [116.5, 40.0], [116.1, 40.0], [116.1, 39.7]]]
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn alos2_query(aoi_geojson: Map<String, Value>) -> CatalogSearchQuery {
        CatalogSearchQuery {
            logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
            provider_id: ALOS2_PROVIDER_ID.to_owned(),
            native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(aoi_geojson),
            cursor: None,
            result_limit: Some(25),
            filters: json!({
                "datetime": {"start": "2023-03-31T00:00:00Z", "end": "2023-04-01T00:00:00Z"},
                "beam_mode": "WBD",
                "polarizations": ["HH", "HV"],
                "flight_direction": "DESCENDING",
                "look_direction": "RIGHT",
                "path": 161,
                "frame": 3150
            })
            .as_object()
            .unwrap()
            .clone(),
        }
    }

    #[test]
    fn two_phase_bind_restart_and_search_records_the_persisted_aoi() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let store = std::env::temp_dir().join(format!(
            "insar-ui-smoke-two-phase-{id}-{}.sqlite3",
            std::process::id()
        ));
        let catalog = UiSmokeLoopbackCatalog::new().expect("loopback catalog");
        let geometry = sample_geometry();

        // Phase A: isolated Store, bind AOI through the service, confirm the
        // write, then shut down normally.
        let service_a = with_ui_smoke_loopback_runtime(
            ApplicationServiceConfig::business_store(store.clone()),
            Arc::clone(&catalog),
        );
        let workspace = service_a
            .workspace_open(&WorkspaceOpenRequest {
                request_id: "ui-smoke-workspace-open".to_owned(),
                workspace_id: "ui-smoke-workspace".to_owned(),
                workspace_root: std::env::temp_dir(),
                workspace_name: "UI smoke workspace".to_owned(),
                expected_revision: None,
            })
            .expect("smoke workspace opens");
        let project = service_a
            .project_create(&WorkspaceNamedNodeRequest {
                request_id: "ui-smoke-project-create".to_owned(),
                parent_id: workspace.workspace_id.clone(),
                expected_revision: workspace.revision,
                name: "UI smoke project".to_owned(),
            })
            .expect("smoke project creates");
        let project_id = project.project_id.expect("smoke project ID");
        let region = service_a
            .region_create(&WorkspaceNamedNodeRequest {
                request_id: "ui-smoke-region-create".to_owned(),
                parent_id: project_id,
                expected_revision: project.revision,
                name: "UI smoke region".to_owned(),
            })
            .expect("smoke region creates");
        let region_id = region.region_id.expect("smoke region ID");
        service_a
            .bind_region_aoi(&ApplicationBindRegionAoiRequest {
                request_id: "ui-smoke-phase-a".to_owned(),
                region_id,
                expected_revision: None,
                source_kind: "manual_bbox".to_owned(),
                geometry: geometry.clone(),
                bbox: vec![116.1, 39.7, 116.5, 40.0],
                crs: "EPSG:4326".to_owned(),
                display_name: "Smoke AOI".to_owned(),
                bound_at: "2026-08-06T00:00:00Z".to_owned(),
            })
            .expect("phase A bind persists");
        assert!(
            service_a
                .core_restore(true)
                .unwrap()
                .core
                .region_aoi
                .is_some(),
            "phase A must write the AOI to the Store"
        );
        service_a.shutdown().expect("phase A shutdown");
        drop(service_a);

        // Phase B: restart on the same Store without rebinding, then run a
        // real search; the provider must receive the restored geometry.
        let service_b = with_ui_smoke_loopback_runtime(
            ApplicationServiceConfig::business_store(store.clone()),
            Arc::clone(&catalog),
        );
        let restored = service_b
            .core_restore(true)
            .unwrap()
            .core
            .region_aoi
            .expect("AOI must restore after restart without rebinding");
        assert_eq!(restored.geometry, geometry);

        let request = ApplicationCatalogSearchAndFreezeRequest {
            request_id: "ui-smoke-phase-b".to_owned(),
            search: alos2_query(restored.geometry.clone()),
            batch_id: "ui-smoke-phase-b-batch".to_owned(),
            created_at: "2026-08-06T00:00:01Z".to_owned(),
            aoi_name: "Smoke AOI".to_owned(),
            aoi_geojson: Some(restored.geometry.clone()),
            workbench_page_size: 24,
        };
        let snapshot = service_b
            .catalog_search_and_freeze(&request)
            .expect("loopback search freezes through the full service boundary");
        assert_eq!(snapshot.core.workbench.unwrap().candidate_count, 1,);
        let recorded = catalog
            .recorded_query()
            .expect("provider received the search request");
        assert_eq!(recorded.aoi_geojson, Some(restored.geometry));
        assert_eq!(recorded.logical_dataset, ALOS2_LOGICAL_DATASET);
        service_b.shutdown().expect("phase B shutdown");

        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", store.display(), suffix));
        }
    }
}
