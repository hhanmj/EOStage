//! Issue #3 R2 B4: the route-neutral intent path freezes the same batch as the
//! reviewed seed query it replaces. Offline only: synthetic products, a
//! recording catalogue port, no Provider transport and no download.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationCatalogIntentSearchAndFreezeRequest,
    ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    CatalogSearchIntent,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogSearchQuery, CatalogSearchResponse,
    CreateTaskFromWorkbenchRequest, DownloadItemProgress, DownloadItemResult, DownloadLayout,
    ProviderResult,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use serde_json::{json, Map, Value};

mod common;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempStore(PathBuf);
impl TempStore {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "sceneport-catalogue-intent-{label}-{}-{id}.sqlite3",
            std::process::id()
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
        panic!("optical intent must never call ASF");
    }
}

struct NoDownload;
impl DownloadWorker for NoDownload {
    fn execute(
        &self,
        _: DownloadWork,
        _: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        panic!("search must never schedule a download");
    }
}

/// Records every query the catalogue port receives and answers with one
/// fixed synthetic response.
struct RecordingCatalog {
    queries: Mutex<Vec<CatalogSearchQuery>>,
    response: CatalogSearchResponse,
}
impl Default for RecordingCatalog {
    fn default() -> Self {
        Self::answering(sentinel2_response())
    }
}
impl RecordingCatalog {
    fn answering(response: CatalogSearchResponse) -> Self {
        Self {
            queries: Mutex::new(Vec::new()),
            response,
        }
    }
}
impl LiveCatalogSearch for RecordingCatalog {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.queries.lock().unwrap().push(query.clone());
        Ok(self.response.clone())
    }
}

fn aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn filters() -> Map<String, Value> {
    json!({
        "product_type": "S2MSI2A",
        "cloud_cover_lte": 4.2,
        "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z"}
    })
    .as_object()
    .unwrap()
    .clone()
}

fn sentinel2_response() -> CatalogSearchResponse {
    serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": "copernicus.dataspace.odata",
            "native_collection_id": "SENTINEL-2",
            "collection_profile_id": "cdse.sentinel-2.l2a.collection.v1",
            "identity_rule_id": "cdse.sentinel-2.l2a.identity.v1",
            "filename_rule_id": "cdse.sentinel-2.l2a.archive-filename.v1",
            "delivery_model": "archive_first",
            "production_enabled": false
        },
        "products": [{
            "provider_id": "copernicus.dataspace.odata",
            "product_id": "00000000-0000-4000-8000-000000000011",
            "display_name": "S2_FIXTURE_VALID.SAFE",
            "source_kind": "sentinel2",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2026-01-01T10:00:00Z",
            "footprint_geojson": {"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},
            "assets": [{
                "asset_key": "archive",
                "stable_asset_key": "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::00000000-0000-4000-8000-000000000011::ASSET::archive",
                "role": "primary",
                "native_roles": ["archive"],
                "size_bytes": 2048
            }],
            "properties": {"product_type":"S2MSI2A", "online":true, "cloud_cover":4.2},
            "payload": {}
        }],
        "stats": {
            "requested_limit": 2, "matched_count": 1, "request_count": 1, "transport_part_count": 1,
            "fetched_record_count": 1, "mapped_record_count": 1, "profile_filtered_record_count": 0,
            "exact_duplicate_record_count": 0, "unique_provider_product_count": 1,
            "exact_matched_count": 1, "returned_count": 1, "remote_exhausted": true,
            "working_set_satisfied": true, "elapsed_ms": 1, "truncated": false
        },
        "item_errors": []
    }))
    .unwrap()
}

fn service(database: &TempStore, catalog: Arc<RecordingCatalog>) -> ApplicationService {
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(NoAsf),
        catalog,
        Arc::new(NoDownload),
    );
    let region_id = common::ensure_fixture_region(&service, "intent");
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "intent-bind-aoi".to_owned(),
            region_id,
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry: aoi(),
            bbox: vec![110.0, 30.0, 111.0, 31.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "Catalogue fixture AOI".to_owned(),
            bound_at: "2026-07-29T00:00:00Z".to_owned(),
        })
        .unwrap();
    service
}

fn seed_request() -> ApplicationCatalogSearchAndFreezeRequest {
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: "seed-request".to_owned(),
        search: CatalogSearchQuery {
            logical_dataset: "optical.sentinel-2.l2a".to_owned(),
            provider_id: "copernicus.dataspace.odata".to_owned(),
            native_collection_id: "SENTINEL-2".to_owned(),
            aoi_geojson: Some(aoi()),
            cursor: None,
            result_limit: Some(2),
            filters: filters(),
        },
        batch_id: "seed-batch".to_owned(),
        created_at: "2026-07-26T00:00:00Z".to_owned(),
        aoi_name: "Fixture AOI".to_owned(),
        aoi_geojson: Some(aoi()),
        workbench_page_size: 24,
    }
}

fn intent_request() -> ApplicationCatalogIntentSearchAndFreezeRequest {
    ApplicationCatalogIntentSearchAndFreezeRequest {
        request_id: "intent-request".to_owned(),
        intent: CatalogSearchIntent {
            resource_id: "science.sentinel-2.l2a.v1".to_owned(),
            search_schema_id: "search.optical.sentinel-2.l2a.v1".to_owned(),
            schema_revision: 1,
            aoi_geojson: Some(aoi()),
            result_limit: Some(2),
            filters: filters(),
        },
        batch_id: "intent-batch".to_owned(),
        created_at: "2026-07-26T00:00:00Z".to_owned(),
        aoi_name: "Fixture AOI".to_owned(),
        aoi_geojson: Some(aoi()),
        workbench_page_size: 24,
    }
}

#[test]
fn intent_freezes_the_same_batch_as_the_reviewed_seed_query() {
    let seed_store = TempStore::new("seed");
    let seed_catalog = Arc::new(RecordingCatalog::default());
    let seed_service = service(&seed_store, Arc::clone(&seed_catalog));
    let seeded = seed_service
        .catalog_search_and_freeze(&seed_request())
        .unwrap();

    let intent_store = TempStore::new("intent");
    let intent_catalog = Arc::new(RecordingCatalog::default());
    let intent_service = service(&intent_store, Arc::clone(&intent_catalog));
    let planned = intent_service
        .catalog_intent_search_and_freeze_with_progress(&intent_request(), |_| {})
        .unwrap();

    // The Provider port receives the identical provider-bound query.
    let seed_queries = seed_catalog.queries.lock().unwrap().clone();
    let intent_queries = intent_catalog.queries.lock().unwrap().clone();
    assert_eq!(seed_queries.len(), 1);
    assert_eq!(intent_queries, seed_queries);

    // Both freeze one candidate with the same identity and source.
    let seeded = seeded.core.workbench.expect("seed batch");
    let planned = planned.core.workbench.expect("intent batch");
    assert_eq!(planned.candidate_count, 1);
    assert_eq!(planned.candidate_count, seeded.candidate_count);
    assert_eq!(planned.batch_id, "intent-batch");
    let seeded_batch = seed_service.restore(true).unwrap().workbench_batch.unwrap();
    let planned_batch = intent_service
        .restore(true)
        .unwrap()
        .workbench_batch
        .unwrap();
    assert_eq!(planned_batch.provider_id, "copernicus.dataspace.odata");
    assert_eq!(planned_batch.provider_id, seeded_batch.provider_id);
    assert_eq!(planned_batch.query, seeded_batch.query);
    assert_eq!(
        planned_batch
            .candidates
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>(),
        seeded_batch
            .candidates
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>()
    );
    seed_service.shutdown().unwrap();
    intent_service.shutdown().unwrap();
}

#[test]
fn invalid_intent_is_rejected_before_any_provider_call() {
    let store = TempStore::new("invalid");
    let catalog = Arc::new(RecordingCatalog::default());
    let service = service(&store, Arc::clone(&catalog));
    let mut request = intent_request();
    request
        .intent
        .filters
        .insert("eo:cloud_cover".to_owned(), json!(10));
    assert!(service
        .catalog_intent_search_and_freeze_with_progress(&request, |_| {})
        .is_err());
    assert!(catalog.queries.lock().unwrap().is_empty());
    assert!(service.restore(true).unwrap().workbench_batch.is_none());
    service.shutdown().unwrap();
}

/// Shape of a reviewed HLS L30 v2.0 product after the Microsoft adapter.
fn hls_l30_response() -> CatalogSearchResponse {
    serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.hls.l30.v2",
            "provider_id": "microsoft.planetary-computer",
            "native_collection_id": "hls2-l30",
            "collection_profile_id": "planetary-computer.hls2-l30.scene.v1",
            "identity_rule_id": "planetary-computer.hls2.identity.v1",
            "filename_rule_id": "planetary-computer.hls2.search-only.v1",
            "delivery_model": "archive_first",
            "production_enabled": true
        },
        "products": [{
            "provider_id": "microsoft.planetary-computer",
            "product_id": "HLS.L30.T49RDP.2024177T030756.v2.0",
            "display_name": "HLS.L30.T49RDP.2024177T030756.v2.0",
            "source_kind": "landsat",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2024-06-25T03:07:56.017810Z",
            "footprint_geojson": {"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},
            "assets": [],
            "properties": {"platform": "landsat_8", "cloud_cover": 79.0},
            "payload": {}
        }],
        "stats": {
            "requested_limit": 100, "matched_count": 1, "request_count": 1, "transport_part_count": 1,
            "fetched_record_count": 1, "mapped_record_count": 1, "profile_filtered_record_count": 0,
            "exact_duplicate_record_count": 0, "unique_provider_product_count": 1,
            "exact_matched_count": 1, "returned_count": 1, "remote_exhausted": true,
            "working_set_satisfied": true, "elapsed_ms": 1, "truncated": false
        },
        "item_errors": []
    }))
    .unwrap()
}

fn hls_intent_request() -> ApplicationCatalogIntentSearchAndFreezeRequest {
    ApplicationCatalogIntentSearchAndFreezeRequest {
        request_id: "hls-intent-request".to_owned(),
        intent: CatalogSearchIntent {
            resource_id: "science.hls.l30.v2".to_owned(),
            search_schema_id: "search.optical.hls.l30.v2".to_owned(),
            schema_revision: 1,
            aoi_geojson: Some(aoi()),
            result_limit: Some(100),
            filters: json!({
                "spacecraft": ["landsat_8"],
                "cloud_cover_lte": 80.0,
                "datetime": {"start": "2024-06-01T00:00:00Z", "end": "2024-06-30T23:59:59Z"}
            })
            .as_object()
            .unwrap()
            .clone(),
        },
        batch_id: "hls-intent-batch".to_owned(),
        created_at: "2026-10-19T00:00:00Z".to_owned(),
        aoi_name: "Fixture AOI".to_owned(),
        aoi_geojson: Some(aoi()),
        workbench_page_size: 24,
    }
}

#[test]
fn hls_intent_freezes_search_only_scenes_without_transfer_authority() {
    let store = TempStore::new("hls");
    let catalog = Arc::new(RecordingCatalog::answering(hls_l30_response()));
    let service = service(&store, Arc::clone(&catalog));
    let snapshot = service
        .catalog_intent_search_and_freeze_with_progress(&hls_intent_request(), |_| {})
        .unwrap();

    // The application planned the Registry route for the HLS product.
    let queries = catalog.queries.lock().unwrap().clone();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].logical_dataset, "optical.hls.l30.v2");
    assert_eq!(queries[0].provider_id, "microsoft.planetary-computer");
    assert_eq!(queries[0].native_collection_id, "hls2-l30");
    assert_eq!(
        queries[0].filters.get("spacecraft"),
        Some(&json!(["landsat_8"]))
    );

    assert_eq!(
        snapshot.core.workbench.expect("HLS batch").candidate_count,
        1
    );
    let batch = service.restore(true).unwrap().workbench_batch.unwrap();
    assert_eq!(batch.provider_id, "microsoft.planetary-computer");
    let [candidate] = batch.candidates.as_slice() else {
        panic!("exactly one frozen HLS candidate");
    };
    assert_eq!(candidate.display_name, "HLS.L30.T49RDP.2024177T030756.v2.0");
    assert_eq!(
        candidate.metadata.get("logical_dataset"),
        Some(&json!("optical.hls.l30.v2"))
    );
    assert_eq!(
        candidate.metadata.get("resolver_policy_id"),
        Some(&json!("planetary-computer.hls2.search-only.v1"))
    );
    assert_eq!(
        candidate.metadata.get("properties"),
        Some(&json!({"platform": "landsat_8", "cloud_cover": 79.0}))
    );
    assert_eq!(candidate.metadata.get("assets"), Some(&json!([])));
    assert!(candidate.payload.is_empty());
    assert!(candidate.asset_access.is_none());
    assert!(candidate.execution_policy.is_none());
    let serialized = serde_json::to_string(candidate).unwrap();
    assert!(!serialized.contains("blob.core.windows.net"));
    service.shutdown().unwrap();
}

/// HLS v2.0 datasets now support full physical download tasks.
#[test]
fn hls_batches_admit_a_download_task() {
    let store = TempStore::new("hls-task-admit");
    let catalog = Arc::new(RecordingCatalog::answering(hls_l30_response()));
    let service = service(&store, Arc::clone(&catalog));
    let snapshot = service
        .catalog_intent_search_and_freeze_with_progress(&hls_intent_request(), |_| {})
        .unwrap();
    let workbench = snapshot.core.workbench.expect("HLS batch");
    let item_id = service
        .restore(true)
        .unwrap()
        .workbench_batch
        .unwrap()
        .candidates[0]
        .item_id
        .clone();
    let output_root = std::env::temp_dir().join("sceneport-hls-task-output");

    let snapshot = service
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "hls-task-exact".to_owned(),
            batch_id: workbench.batch_id.clone(),
            expected_workbench_revision: workbench.revision,
            exact_item_id: Some(item_id.clone()),
            output_root: output_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-20T00:00:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(snapshot.core.tasks.len(), 1);
    assert_eq!(snapshot.core.tasks[0].task_id, "hls-task-exact");
    assert_eq!(service.restore(true).unwrap().tasks.len(), 1);
    service.shutdown().unwrap();
}

#[test]
fn hls_freeze_rejects_a_product_outside_the_reviewed_identity() {
    let mut response = hls_l30_response();
    response.products[0].product_id = "HLS.S30.T49RDP.2024177T030756.v2.0".to_owned();
    response.products[0].display_name = response.products[0].product_id.clone();
    let store = TempStore::new("hls-invalid");
    let catalog = Arc::new(RecordingCatalog::answering(response));
    let service = service(&store, Arc::clone(&catalog));
    assert!(service
        .catalog_intent_search_and_freeze_with_progress(&hls_intent_request(), |_| {})
        .is_err());
    assert!(service.restore(true).unwrap().workbench_batch.is_none());
    service.shutdown().unwrap();
}

// ---------------------------------------------------------------------------
// Unified-search (intent) Landsat freeze to reviewed asset task (2026-10-31).
// The new workbench freezes Landsat C2 L2 via the intent path; the reviewed
// asset task must accept that frozen item exactly like the legacy seed batch.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct LandsatAssetManifest {
    calls: Mutex<Vec<(String, String, String)>>,
}

impl insar_application_service::LiveLandsatAssetManifest for LandsatAssetManifest {
    fn landsat_asset_manifest(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<
        insar_provider::planetary_computer::PlanetaryComputerLandsatManifest,
        insar_provider::planetary_computer::PlanetaryComputerError,
    > {
        assert!(!cancelled());
        self.calls.lock().unwrap().push((
            dataset_alias.to_owned(),
            entity_id.to_owned(),
            display_id.to_owned(),
        ));
        let display = "LC09_L2SP_123045_20260101_20260103_02_T1";
        // The reviewed manifest validator requires the exact l2sp asset set (22).
        let definitions = [
            ("coastal", "_SR_B1.TIF", insar_core::AssetRole::Primary),
            ("blue", "_SR_B2.TIF", insar_core::AssetRole::Primary),
            ("green", "_SR_B3.TIF", insar_core::AssetRole::Primary),
            ("red", "_SR_B4.TIF", insar_core::AssetRole::Primary),
            ("nir08", "_SR_B5.TIF", insar_core::AssetRole::Primary),
            ("swir16", "_SR_B6.TIF", insar_core::AssetRole::Primary),
            ("swir22", "_SR_B7.TIF", insar_core::AssetRole::Primary),
            ("lwir11", "_ST_B10.TIF", insar_core::AssetRole::Primary),
            ("drad", "_ST_DRAD.TIF", insar_core::AssetRole::Primary),
            ("emis", "_ST_EMIS.TIF", insar_core::AssetRole::Primary),
            ("emsd", "_ST_EMSD.TIF", insar_core::AssetRole::Primary),
            ("trad", "_ST_TRAD.TIF", insar_core::AssetRole::Primary),
            ("urad", "_ST_URAD.TIF", insar_core::AssetRole::Primary),
            ("atran", "_ST_ATRAN.TIF", insar_core::AssetRole::Primary),
            ("cdist", "_ST_CDIST.TIF", insar_core::AssetRole::Primary),
            ("qa", "_ST_QA.TIF", insar_core::AssetRole::Auxiliary),
            (
                "qa_pixel",
                "_QA_PIXEL.TIF",
                insar_core::AssetRole::Auxiliary,
            ),
            (
                "qa_radsat",
                "_QA_RADSAT.TIF",
                insar_core::AssetRole::Auxiliary,
            ),
            (
                "qa_aerosol",
                "_SR_QA_AEROSOL.TIF",
                insar_core::AssetRole::Auxiliary,
            ),
            ("ang", "_ANG.txt", insar_core::AssetRole::Metadata),
            ("mtl.txt", "_MTL.txt", insar_core::AssetRole::Metadata),
            ("mtl.xml", "_MTL.xml", insar_core::AssetRole::Metadata),
        ];
        let assets = definitions
            .into_iter()
            .enumerate()
            .map(|(index, (asset_key, suffix, role))| {
                let file_name = format!("{display}{suffix}");
                insar_provider::planetary_computer::PlanetaryComputerLandsatAsset {
                    asset_key: asset_key.to_owned(),
                    stable_href: format!(
                        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2026/123/045/{display}/{file_name}"
                    ),
                    file_name,
                    role,
                    expected_size_bytes: 10_000 + index as u64,
                }
            })
            .collect::<Vec<_>>();
        Ok(
            insar_provider::planetary_computer::PlanetaryComputerLandsatManifest {
                entity_id: entity_id.to_owned(),
                display_id: display.to_owned(),
                stac_item_id: entity_id.to_owned(),
                processing_level: "l2sp".to_owned(),
                total_size_bytes: assets.iter().map(|asset| asset.expected_size_bytes).sum(),
                assets,
            },
        )
    }
}

fn mpc_landsat_catalog_response() -> CatalogSearchResponse {
    serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.landsat.c2.l2",
            "provider_id": "microsoft.planetary-computer",
            "native_collection_id": "landsat-c2-l2",
            "collection_profile_id": "planetary-computer.landsat-c2-l2.scene.v1",
            "identity_rule_id": "planetary-computer.landsat-c2-l2.identity.v1",
            "filename_rule_id": "planetary-computer.landsat-c2-l2.asset-filename.v1",
            "delivery_model": "archive_first",
            "production_enabled": true
        },
        "products": [{
            "provider_id": "microsoft.planetary-computer",
            "product_id": "LC09_L2SP_123045_20260101_02_T1",
            "display_name": "LC09_L2SP_123045_20260101_20260103_02_T1",
            "source_kind": "landsat",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2026-01-01T00:00:00Z",
            "footprint_geojson": {"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},
            "assets": [],
            "properties": {
                "entity_id": "LC09_L2SP_123045_20260101_02_T1",
                "landsat_product_id": "LC09_L2SP_123045_20260101_20260103_02_T1",
                "dataset_alias": "landsat-c2-l2",
                "spacecraft": "landsat_9",
                "processing_level": "l2sp",
                "data_type_l2": "LANDSAT_9_L2SP",
                "collection_category": "T1",
                "cloud_cover": 12.5,
                "wrs_path": "123",
                "wrs_row": "045",
                "acquisition_date": "2026/01/01",
                "acquisition_precision": "day",
                "browse_available": true
            },
            "payload": {}
        }],
        "stats": {
            "requested_limit": 2000, "matched_count": 1, "request_count": 1, "transport_part_count": 1,
            "fetched_record_count": 1, "mapped_record_count": 1, "profile_filtered_record_count": 0,
            "exact_duplicate_record_count": 0, "unique_provider_product_count": 1,
            "exact_matched_count": 1, "returned_count": 1, "remote_exhausted": true,
            "working_set_satisfied": true, "elapsed_ms": 1, "truncated": false
        },
        "item_errors": []
    }))
    .unwrap()
}

#[test]
fn intent_frozen_landsat_item_creates_a_reviewed_asset_selection_task() {
    let store = TempStore::new("landsat-intent-asset");
    let manifest = Arc::new(LandsatAssetManifest::default());
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_assets(
        ApplicationServiceConfig::business_store(&store.0),
        Arc::new(NoAsf),
        Arc::new(RecordingCatalog::answering(mpc_landsat_catalog_response())),
        manifest.clone(),
        Arc::new(NoDownload),
    );
    let region_id = common::ensure_fixture_region(&service, "landsat-intent");
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "landsat-intent-bind".to_owned(),
            region_id,
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry: aoi(),
            bbox: vec![110.0, 30.0, 111.0, 31.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "Catalogue fixture AOI".to_owned(),
            bound_at: "2026-10-31T00:00:00Z".to_owned(),
        })
        .unwrap();

    // The real unified-search entry (same worker thread the desktop uses).
    let shared = std::sync::Arc::new(service);
    shared
        .search_start(insar_core::UnifiedSearchRequest {
            resource_id: "science.landsat.c2-l2.v1".to_owned(),
            aoi: aoi(),
            aoi_name: "Fixture AOI".to_owned(),
            time_range: None,
            filters: json!({
                "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-03T00:00:00Z"},
                "cloud_cover_lte": 40.0,
                "spacecraft": ["landsat_8", "landsat_9"],
                "processing_levels": ["l2sp", "l2sr"]
            })
            .as_object()
            .unwrap()
            .clone(),
            request_id: "unified-landsat-search-1".to_owned(),
        })
        .expect("unified search accepted");
    // Wait for the worker thread to finish freezing the batch.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let workbench = loop {
        let Some(done) = shared.search_progress("unified-landsat-search-1") else {
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(
                std::time::Instant::now() < deadline,
                "unified search never finished"
            );
            continue;
        };
        if done.terminal {
            assert!(done.error.is_none(), "unified search failed: {done:?}");
            break shared
                .restore(true)
                .unwrap()
                .workbench_batch
                .expect("unified batch activated the workbench");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "unified search never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let item = &workbench.candidates[0];

    // The new workbench reads the file list first (same as the live UI).
    let details = shared
        .landsat_asset_details(insar_application_service::LandsatAssetDetailsRequest {
            request_id: "landsat-intent-details-1".to_owned(),
            batch_id: workbench.batch_id.clone(),
            item_id: item.item_id.clone(),
        })
        .expect("asset details for the intent-frozen item");
    assert_eq!(details.assets.len(), 22);

    let result = shared.create_landsat_asset_selection_task(
        &insar_application_service::LandsatCreateAssetTaskRequest {
            request_id: "landsat-intent-asset-1".to_owned(),
            source_batch_id: workbench.batch_id.clone(),
            item_id: item.item_id.clone(),
            asset_keys: vec![
                "mtl.txt".to_owned(),
                "mtl.xml".to_owned(),
                "qa_radsat".to_owned(),
            ],
            task_id: "landsat-intent-asset-task-1".to_owned(),
            output_root: store.0.parent().unwrap().join("landsat-intent-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-10-31T01:00:00Z".to_owned(),
        },
    );
    result.expect("intent-frozen Landsat item must create a reviewed asset task");
    let task = shared
        .restore(true)
        .unwrap()
        .tasks
        .iter()
        .find(|task| task.task_id == "landsat-intent-asset-task-1")
        .expect("asset task")
        .clone();
    assert_eq!(task.candidates.len(), 3);
    // Details read and task preparation each resolve the manifest once.
    assert_eq!(manifest.calls.lock().unwrap().len(), 2);
    shared.shutdown().unwrap();
}

#[test]
fn landsat_asset_task_admits_from_the_named_batch_after_another_source_takes_the_slot() {
    let store = TempStore::new("landsat-slot-moved");
    let manifest = Arc::new(LandsatAssetManifest::default());
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_assets(
        ApplicationServiceConfig::business_store(&store.0),
        Arc::new(NoAsf),
        Arc::new(RecordingCatalog::answering(mpc_landsat_catalog_response())),
        manifest.clone(),
        Arc::new(NoDownload),
    );
    let region_id = common::ensure_fixture_region(&service, "landsat-slot-moved");
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "landsat-slot-moved-bind".to_owned(),
            region_id,
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry: aoi(),
            bbox: vec![110.0, 30.0, 111.0, 31.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "Catalogue fixture AOI".to_owned(),
            bound_at: "2026-10-31T00:00:00Z".to_owned(),
        })
        .unwrap();
    let shared = std::sync::Arc::new(service);

    // 1. Freeze Landsat through the real unified-search worker thread.
    shared
        .search_start(insar_core::UnifiedSearchRequest {
            resource_id: "science.landsat.c2-l2.v1".to_owned(),
            aoi: aoi(),
            aoi_name: "Fixture AOI".to_owned(),
            time_range: None,
            filters: json!({
                "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-03T00:00:00Z"},
                "cloud_cover_lte": 40.0,
                "spacecraft": ["landsat_8", "landsat_9"],
                "processing_levels": ["l2sp", "l2sr"]
            })
            .as_object()
            .unwrap()
            .clone(),
            request_id: "unified-landsat-search-a".to_owned(),
        })
        .expect("unified Landsat search accepted");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let landsat_batch = loop {
        let Some(done) = shared.search_progress("unified-landsat-search-a") else {
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(
                std::time::Instant::now() < deadline,
                "Landsat search never finished"
            );
            continue;
        };
        if done.terminal {
            assert!(
                done.error.is_none(),
                "unified Landsat search failed: {done:?}"
            );
            break shared
                .restore(true)
                .unwrap()
                .workbench_batch
                .expect("Landsat batch frozen and active");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let item = &landsat_batch.candidates[0];

    // 2. Search again (the same or another source): the new batch replaces
    //    the Workbench slot, so the slot no longer holds the named batch.
    shared
        .search_start(insar_core::UnifiedSearchRequest {
            resource_id: "science.landsat.c2-l2.v1".to_owned(),
            aoi: aoi(),
            aoi_name: "Fixture AOI".to_owned(),
            time_range: None,
            filters: json!({
                "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-03T00:00:00Z"},
                "cloud_cover_lte": 40.0,
                "spacecraft": ["landsat_8", "landsat_9"],
                "processing_levels": ["l2sp", "l2sr"]
            })
            .as_object()
            .unwrap()
            .clone(),
            request_id: "unified-landsat-search-b".to_owned(),
        })
        .expect("second unified search accepted");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let Some(done) = shared.search_progress("unified-landsat-search-b") else {
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(
                std::time::Instant::now() < deadline,
                "second search never finished"
            );
            continue;
        };
        if done.terminal {
            assert!(done.error.is_none(), "unified search failed: {done:?}");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let slot = shared
        .restore(true)
        .unwrap()
        .workbench_batch
        .expect("slot still holds a batch");
    assert_ne!(
        slot.batch_id, landsat_batch.batch_id,
        "the slot must have moved"
    );

    // 3. The Landsat asset task must admit from the named frozen batch even
    //    though the Workbench slot now holds the other source's results.
    shared
        .create_landsat_asset_selection_task(
            &insar_application_service::LandsatCreateAssetTaskRequest {
                request_id: "landsat-slot-moved-asset-1".to_owned(),
                source_batch_id: landsat_batch.batch_id.clone(),
                item_id: item.item_id.clone(),
                asset_keys: vec![
                    "mtl.txt".to_owned(),
                    "mtl.xml".to_owned(),
                    "qa_radsat".to_owned(),
                ],
                task_id: "landsat-slot-moved-asset-task-1".to_owned(),
                output_root: store.0.parent().unwrap().join("landsat-slot-moved-output"),
                layout: DownloadLayout::Direct,
                concurrency: 2,
                created_at: "2026-10-31T02:00:00Z".to_owned(),
            },
        )
        .expect("Landsat asset task admits from the named batch after the slot moved");
    let task = shared
        .restore(true)
        .unwrap()
        .tasks
        .iter()
        .find(|task| task.task_id == "landsat-slot-moved-asset-task-1")
        .expect("asset task")
        .clone();
    assert_eq!(task.candidates.len(), 3);
    assert_eq!(manifest.calls.lock().unwrap().len(), 1);

    // 4. A request naming a batch that does not exist is still rejected.
    let error = shared
        .create_landsat_asset_selection_task(
            &insar_application_service::LandsatCreateAssetTaskRequest {
                request_id: "landsat-slot-moved-asset-2".to_owned(),
                source_batch_id: "batch-unified-nonexistent-0000000000000000".to_owned(),
                item_id: item.item_id.clone(),
                asset_keys: vec!["red".to_owned()],
                task_id: "landsat-slot-moved-asset-task-2".to_owned(),
                output_root: store.0.parent().unwrap().join("landsat-slot-moved-output"),
                layout: DownloadLayout::Direct,
                concurrency: 2,
                created_at: "2026-10-31T02:00:01Z".to_owned(),
            },
        )
        .unwrap_err();
    assert_eq!(error.code(), "LANDSAT_PUBLIC_ASSET_FROZEN_EVIDENCE_INVALID");

    // 5. A request whose item is not in the named batch is still rejected;
    //    naming a valid batch never bypasses the per-item membership check.
    let error = shared
        .create_landsat_asset_selection_task(
            &insar_application_service::LandsatCreateAssetTaskRequest {
                request_id: "landsat-slot-moved-asset-3".to_owned(),
                source_batch_id: landsat_batch.batch_id.clone(),
                item_id: "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_069015_20261012_02_T1".to_owned(),
                asset_keys: vec!["red".to_owned()],
                task_id: "landsat-slot-moved-asset-task-3".to_owned(),
                output_root: store.0.parent().unwrap().join("landsat-slot-moved-output"),
                layout: DownloadLayout::Direct,
                concurrency: 2,
                created_at: "2026-10-31T02:00:02Z".to_owned(),
            },
        )
        .unwrap_err();
    assert_eq!(error.code(), "LANDSAT_PUBLIC_ASSET_FROZEN_EVIDENCE_INVALID");
    shared.shutdown().unwrap();
}

#[test]
fn unified_create_task_expands_mpc_landsat_scene_into_reviewed_asset_items() {
    let store = TempStore::new("landsat-unified-create-task");
    let manifest = Arc::new(LandsatAssetManifest::default());
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_assets(
        ApplicationServiceConfig::business_store(&store.0),
        Arc::new(NoAsf),
        Arc::new(RecordingCatalog::answering(mpc_landsat_catalog_response())),
        manifest.clone(),
        Arc::new(NoDownload),
    );
    let region_id = common::ensure_fixture_region(&service, "landsat-unified-create-task");
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "landsat-unified-create-task-bind".to_owned(),
            region_id,
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry: aoi(),
            bbox: vec![110.0, 30.0, 111.0, 31.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "Catalogue fixture AOI".to_owned(),
            bound_at: "2026-10-31T00:00:00Z".to_owned(),
        })
        .unwrap();

    let shared = Arc::new(service);
    shared
        .search_start(insar_core::UnifiedSearchRequest {
            resource_id: "science.landsat.c2-l2.v1".to_owned(),
            aoi: aoi(),
            aoi_name: "Fixture AOI".to_owned(),
            time_range: Some(insar_core::TimeRange {
                start: Some("2026-01-01T00:00:00Z".to_owned()),
                end: Some("2026-01-03T00:00:00Z".to_owned()),
            }),
            filters: json!({
                "cloud_cover_lte": 40.0
            })
            .as_object()
            .unwrap()
            .clone(),
            request_id: "unified-landsat-create-task-search-1".to_owned(),
        })
        .expect("unified search with omitted RestoreAll defaults must succeed");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let workbench = loop {
        let Some(done) = shared.search_progress("unified-landsat-create-task-search-1") else {
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(std::time::Instant::now() < deadline, "search timed out");
            continue;
        };
        if done.terminal {
            assert!(done.error.is_none(), "unified search failed: {done:?}");
            break shared
                .restore(true)
                .unwrap()
                .workbench_batch
                .expect("unified batch activated the workbench");
        }
        assert!(std::time::Instant::now() < deadline, "search timed out");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let scene = &workbench.candidates[0];
    assert!(
        scene.execution_policy.is_none(),
        "frozen parent scene item has no direct execution_policy before asset expansion"
    );

    let output_root = store.0.parent().unwrap().join("landsat-unified-task-out");
    let created = shared
        .create_task(
            workbench.batch_id.clone(),
            0,
            vec![scene.item_id.clone()],
            insar_application_service::UnifiedCreateTaskOptions {
                output_root: output_root.clone(),
                layout: DownloadLayout::SarDatasetProcessing,
                concurrency: 2,
                include_precise_orbit: None,
            },
        )
        .expect("unified create_task must expand MPC Landsat scene into reviewed asset items");
    assert!(created.created);
    assert!(
        created.warning.is_none(),
        "MPC Landsat must not trigger precise-orbit warning: {:?}",
        created.warning
    );

    // Re-issuing against the live task reuses the expanded explicit-asset task cleanly.
    let reused = shared
        .create_task(
            workbench.batch_id.clone(),
            0,
            vec![scene.item_id.clone()],
            insar_application_service::UnifiedCreateTaskOptions {
                output_root,
                layout: DownloadLayout::Direct,
                concurrency: 2,
                include_precise_orbit: None,
            },
        )
        .expect("re-issuing create_task against live MPC Landsat task must reuse cleanly");
    assert!(!reused.created);
    assert_eq!(reused.task_id, created.task_id);

    let restored = shared
        .core_restore(true)
        .expect("core_restore must succeed");
    assert_eq!(restored.core.tasks.len(), 1);
    assert_eq!(restored.core.tasks[0].selected_count, 22);
    assert_eq!(restored.core.tasks[0].layout, DownloadLayout::Direct);

    let full = shared.restore(true).unwrap();
    let task = full
        .tasks
        .iter()
        .find(|t| t.task_id == created.task_id)
        .expect("stored task snapshot");
    assert_eq!(task.candidates.len(), 22);
    assert!(task.candidates.iter().all(|asset| {
        asset.asset_access.is_some()
            && asset.execution_policy.as_ref().is_some_and(|p| {
                p.policy_id == insar_provider::catalog::MPC_LANDSAT_ASSET_POLICY_ID
            })
            && asset
                .metadata
                .get("source_item_id")
                .and_then(|v| v.as_str())
                == Some(scene.item_id.as_str())
    }));

    shared.shutdown().unwrap();
}
