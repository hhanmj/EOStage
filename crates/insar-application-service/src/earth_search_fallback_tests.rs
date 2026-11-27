//! Production planning and publication with the catalogue I/O port replaced.
//! No public request or download is exercised by these tests.
use super::*;
use insar_core::ProviderResult;
use insar_provider::catalog::{
    EARTH_SEARCH_LANDSAT_PROVIDER_ID as EARTH, EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID,
    MPC_LANDSAT_PROVIDER_ID as MPC,
};
use serde_json::json;
use std::sync::Weak;

const NATIVE: &str = "LC09_L2SP_123045_20260101_02_T1";
const DISPLAY: &str = "LC09_L2SP_123045_20260101_20260103_02_T1";
const REQUEST_ID: &str = "earth-plan-query";

struct TestRoot(PathBuf);
impl TestRoot {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("sceneport-earth-fallback-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    Fallback,
    PrimaryInvalid,
    BackupInvalid,
    CancelPrimary,
}

struct Catalogue {
    scenario: Scenario,
    calls: Mutex<Vec<CatalogSearchQuery>>,
    dispatcher: Mutex<Weak<Dispatcher>>,
}
impl LiveCatalogSearch for Catalogue {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        assert!(!cancelled());
        self.calls.lock().unwrap().push(query.clone());
        if query.provider_id == MPC {
            if matches!(self.scenario, Scenario::CancelPrimary) {
                assert!(self
                    .dispatcher
                    .lock()
                    .unwrap()
                    .upgrade()
                    .unwrap()
                    .cancel_request(REQUEST_ID));
            }
            let code = if matches!(self.scenario, Scenario::PrimaryInvalid) {
                "PLANETARY_COMPUTER_RESPONSE_INVALID"
            } else {
                "PLANETARY_COMPUTER_TRANSPORT_FAILED"
            };
            return Err(
                ContractError::new(code, "synthetic catalogue failure", Map::new()).unwrap(),
            );
        }
        assert_eq!(
            query.provider_id, EARTH,
            "only the admitted backup can be called"
        );
        let mut response = earth_response(query);
        if matches!(self.scenario, Scenario::BackupInvalid) {
            response.products[0]
                .properties
                .insert("browse_available".to_owned(), json!(true));
        }
        Ok(response)
    }
}

fn geometry() -> Map<String, Value> {
    json!({"type":"Polygon","coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]})
        .as_object()
        .unwrap()
        .clone()
}
fn request() -> ApplicationCatalogSearchAndFreezeRequest {
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: REQUEST_ID.to_owned(),
        batch_id: "earth-planned-batch".to_owned(),
        created_at: "2026-07-28T00:00:00Z".to_owned(),
        aoi_name: "Synthetic fallback AOI".to_owned(),
        aoi_geojson: Some(geometry()),
        workbench_page_size: 24,
        // An obsolete source hint must not control the product's current plan.
        search: serde_json::from_value(json!({
            "logical_dataset":LANDSAT_C2_L2_LOGICAL_DATASET,
            "provider_id":"usgs.eros.m2m","native_collection_id":"landsat_ot_c2_l2",
            "aoi_geojson":geometry(), "result_limit":2,
            "filters":{"cloud_cover_lte":40.0,"spacecraft":["landsat_8","landsat_9"],
                "processing_levels":["l2sp","l2sr"],
                "datetime":{"start":"2026-01-01T00:00:00Z","end":"2026-01-02T00:00:00Z"}}
        }))
        .unwrap(),
    }
}
fn earth_response(query: &CatalogSearchQuery) -> CatalogSearchResponse {
    let profile = CollectionProfileRegistry
        .get(EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID)
        .unwrap();
    serde_json::from_value(json!({
        "collection":{"logical_dataset":LANDSAT_C2_L2_LOGICAL_DATASET,"provider_id":EARTH,
            "native_collection_id":profile.native_collection_id,"collection_profile_id":profile.profile_id,
            "identity_rule_id":profile.identity_rule_id,"filename_rule_id":profile.filename_rule_id,
            "delivery_model":"archive_first","production_enabled":true},
        "products":[{"provider_id":EARTH,"product_id":NATIVE,"display_name":DISPLAY,
            "source_kind":"landsat","product_kind":"optical_scene",
            "acquisition_datetime":"2026-01-01T00:00:00Z","footprint_geojson":geometry(),
            "assets":[],"payload":{},"properties":{
                "entity_id":NATIVE,"landsat_product_id":DISPLAY,"landsat_scene_id":"LC91230452026001LGN00",
                "dataset_alias":profile.native_collection_id,"spacecraft":"landsat_9",
                "processing_level":"l2sp","data_type_l2":"OLI_TIRS_L2SP","collection_category":"T1",
                "cloud_cover":12.5,"wrs_path":"123","wrs_row":"045","acquisition_date":"2026/01/01",
                "acquisition_precision":"day","browse_available":false}}],
        "stats":{"requested_limit":query.result_limit,"matched_count":null,"request_count":1,
            "transport_part_count":1,"fetched_record_count":1,"mapped_record_count":1,
            "profile_filtered_record_count":0,"exact_duplicate_record_count":0,
            "unique_provider_product_count":1,"exact_matched_count":1,"returned_count":1,
            "remote_exhausted":true,"working_set_satisfied":true,"elapsed_ms":1,"truncated":false},
        "next_cursor":null,"item_errors":[]
    })).unwrap()
}

fn service(root: &TestRoot, scenario: Scenario) -> (ApplicationService, Arc<Catalogue>) {
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(
        root.0.join("tasks.sqlite3"),
    ));
    service.startup().unwrap();
    let catalog = Arc::new(Catalogue {
        scenario,
        calls: Mutex::new(Vec::new()),
        dispatcher: Mutex::new(Weak::new()),
    });
    let dispatcher = Arc::new(Dispatcher::with_production_catalogue_for_application(
        Arc::new(insar_provider::asf::search::AsfProvider::production().unwrap()),
        catalog.clone(),
    ));
    *catalog.dispatcher.lock().unwrap() = Arc::downgrade(&dispatcher);
    // Retain real constructor observations and factory. Substitution is private
    // to this test, not an exported production admission override.
    service.runtime.lock().unwrap().as_mut().unwrap().dispatcher = Some(dispatcher);
    let workspace = service
        .workspace_open(&WorkspaceOpenRequest {
            request_id: "earth-workspace".to_owned(),
            workspace_id: "earth-workspace".to_owned(),
            workspace_root: root.0.clone(),
            workspace_name: "Synthetic".to_owned(),
            expected_revision: None,
        })
        .unwrap();
    let project = service
        .project_create(&WorkspaceNamedNodeRequest {
            request_id: "earth-project".to_owned(),
            parent_id: "earth-workspace".to_owned(),
            expected_revision: workspace.revision,
            name: "Synthetic project".to_owned(),
        })
        .unwrap();
    let region = service
        .region_create(&WorkspaceNamedNodeRequest {
            request_id: "earth-region".to_owned(),
            parent_id: project.project_id.unwrap(),
            expected_revision: project.revision,
            name: "Synthetic region".to_owned(),
        })
        .unwrap();
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "earth-aoi".to_owned(),
            region_id: region.region_id.unwrap(),
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry: geometry(),
            bbox: vec![110.0, 30.0, 111.0, 31.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "Synthetic fallback AOI".to_owned(),
            bound_at: "2026-07-28T00:00:00Z".to_owned(),
        })
        .unwrap();
    (service, catalog)
}

#[test]
fn automatic_earth_fallback_freezes_actual_source_and_survives_store_reopen() {
    let root = TestRoot::new();
    let (service, catalog) = service(&root, Scenario::Fallback);
    let input = request();
    let snapshot = service.catalog_search_and_freeze_legacy(&input).unwrap();
    let batch = snapshot.restore.workbench_batch.unwrap();
    assert_eq!(batch.provider_id, EARTH);
    assert_eq!(batch.query["provider_id"], EARTH);
    let stats = batch.search_stats.as_ref().unwrap();
    assert_eq!(stats.matched_count, None);
    assert_eq!(stats.source_attempts.len(), 2);
    assert_eq!(stats.source_attempts[0].provider_id, MPC);
    assert_eq!(
        stats.source_attempts[0].outcome,
        FrozenSearchSourceAttemptOutcome::Fallback
    );
    assert_eq!(
        stats.source_attempts[0].fallback_reason,
        Some(FrozenSearchSourceFallbackReason::TransientNetwork)
    );
    assert_eq!(stats.source_attempts[1].provider_id, EARTH);
    assert_eq!(
        stats.source_attempts[1].outcome,
        FrozenSearchSourceAttemptOutcome::Succeeded
    );
    assert_eq!(stats.source_attempts[1].fallback_reason, None);
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(batch.candidates[0].metadata["provider_product_id"], NATIVE);
    assert_eq!(
        batch.candidates[0].item_id,
        provider_product_key(EARTH, "landsat-c2-l2", NATIVE).unwrap()
    );
    assert!(
        batch.candidates[0].asset_access.is_none()
            && batch.candidates[0].execution_policy.is_none()
    );
    assert!(snapshot.restore.tasks.is_empty());
    let calls = catalog.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .map(|q| q.provider_id.as_str())
            .collect::<Vec<_>>(),
        [MPC, EARTH]
    );
    assert!(calls
        .iter()
        .all(|q| q.filters == input.search.filters && q.aoi_geojson == input.search.aoi_geojson));
    drop(calls);
    service.shutdown().unwrap();
    let reopened = ApplicationService::new(ApplicationServiceConfig::business_store(
        root.0.join("tasks.sqlite3"),
    ));
    assert_eq!(
        reopened
            .snapshot(true)
            .unwrap()
            .restore
            .workbench_batch
            .unwrap(),
        batch
    );
    reopened.shutdown().unwrap();
}

#[test]
fn automatic_earth_fallback_does_not_hide_cancellation_or_bad_source_evidence() {
    for scenario in [
        Scenario::PrimaryInvalid,
        Scenario::BackupInvalid,
        Scenario::CancelPrimary,
    ] {
        let root = TestRoot::new();
        let (service, catalog) = service(&root, scenario);
        assert!(service
            .catalog_search_and_freeze_legacy(&request())
            .is_err());
        let snapshot = service.snapshot(true).unwrap();
        assert!(snapshot.restore.workbench_batch.is_none() && snapshot.restore.tasks.is_empty());
        let expected = if matches!(scenario, Scenario::BackupInvalid) {
            vec![MPC, EARTH]
        } else {
            vec![MPC]
        };
        assert_eq!(
            catalog
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|q| q.provider_id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        service.shutdown().unwrap();
    }
}

#[test]
fn unknown_stac_total_never_waives_completeness_or_observed_count_checks() {
    let mut query = request().search;
    query.provider_id = EARTH.to_owned();
    query.native_collection_id = "landsat-c2-l2".to_owned();
    let response = earth_response(&query);
    assert!(
        validate_complete_catalogue_response(&response, &query, RouteUse::Production, false)
            .is_ok()
    );
    for mutation in 0..6 {
        let mut invalid = response.clone();
        match mutation {
            0 => invalid.stats.remote_exhausted = false,
            1 => invalid.stats.matched_count = Some(0),
            2 => invalid.stats.matched_count = Some(2),
            3 => invalid.stats.fetched_record_count = 2,
            4 => invalid.stats.returned_count = 2,
            _ => invalid.stats.exact_duplicate_record_count = 1,
        }
        assert!(
            validate_complete_catalogue_response(&invalid, &query, RouteUse::Production, false)
                .is_err(),
            "mutation {mutation}"
        );
    }
    let mut known = response.clone();
    known.stats.matched_count = Some(1);
    assert!(
        validate_complete_catalogue_response(&known, &query, RouteUse::Production, false).is_ok()
    );
    query.result_limit = Some(1);
    let mut finite = earth_response(&query);
    finite.stats.remote_exhausted = false;
    assert!(
        validate_complete_catalogue_response(&finite, &query, RouteUse::Production, false).is_ok()
    );
    assert_eq!(finite.stats.matched_count, None);
}
