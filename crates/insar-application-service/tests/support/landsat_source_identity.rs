//! Reuse the existing Landsat fixtures, varying channel identity, never science.
use super::*;
use insar_provider::catalog::{
    CollectionProfileRegistry, MPC_LANDSAT_ASSET_POLICY_ID, MPC_LANDSAT_COLLECTION_ID,
    MPC_LANDSAT_PROVIDER_ID, MPC_LANDSAT_SCENE_PROFILE_ID,
};

const NATIVE_ID: &str = "LC09_L2SP_123045_20260101_02_T1";

fn native_response() -> CatalogSearchResponse {
    let mut response = landsat_response();
    let profile = CollectionProfileRegistry
        .get(MPC_LANDSAT_SCENE_PROFILE_ID)
        .unwrap();
    let collection = &mut response.collection;
    collection.provider_id = profile.provider_id.to_owned();
    collection.native_collection_id = profile.native_collection_id.to_owned();
    collection.collection_profile_id = profile.profile_id.to_owned();
    collection.identity_rule_id = profile.identity_rule_id.to_owned();
    collection.filename_rule_id = profile.filename_rule_id.to_owned();
    collection.production_enabled = true;
    let product = &mut response.products[0];
    product.provider_id = profile.provider_id.to_owned();
    product.product_id = NATIVE_ID.to_owned();
    product.properties["entity_id"] = json!(NATIVE_ID);
    product.properties["dataset_alias"] = json!(profile.native_collection_id);
    response
}

fn native_request() -> ApplicationCatalogSearchAndFreezeRequest {
    let mut request = landsat_request();
    request.search.provider_id = MPC_LANDSAT_PROVIDER_ID.to_owned();
    request.search.native_collection_id = MPC_LANDSAT_COLLECTION_ID.to_owned();
    request
}

#[test]
fn microsoft_scene_freezes_and_reopens_without_relabelling_or_changing_science() {
    let database = TempStore::new("native-microsoft-source-freeze");
    let mut legacy = landsat_response();
    let science = legacy.products.remove(0);
    let service = production_fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: native_response(),
        }),
    );
    let snapshot = service
        .catalog_search_and_freeze_legacy(&native_request())
        .unwrap();
    let batch = snapshot.restore.workbench_batch.unwrap();
    let item = &batch.candidates[0];
    assert_eq!(batch.provider_id, MPC_LANDSAT_PROVIDER_ID);
    assert_eq!(
        batch.query["native_collection_id"],
        MPC_LANDSAT_COLLECTION_ID
    );
    assert_eq!(
        item.item_id,
        provider_product_key(
            MPC_LANDSAT_PROVIDER_ID,
            MPC_LANDSAT_COLLECTION_ID,
            NATIVE_ID
        )
        .unwrap()
    );
    assert_eq!(item.acquisition_datetime, science.acquisition_datetime);
    assert_eq!(item.footprint_geojson, science.footprint_geojson);
    assert_eq!(item.display_name, science.display_name);
    for key in [
        "landsat_scene_id",
        "landsat_product_id",
        "wrs_path",
        "wrs_row",
        "cloud_cover",
        "processing_level",
        "acquisition_precision",
    ] {
        assert_eq!(
            item.metadata["properties"][key], science.properties[key],
            "{key}"
        );
    }
    assert!(item.asset_access.is_none() && item.execution_policy.is_none());
    assert!(snapshot.restore.tasks.is_empty());
    let frozen = serde_json::to_vec(&batch).unwrap();
    service.shutdown().unwrap();
    let reopened = ApplicationService::new(ApplicationServiceConfig::business_store(&database.0));
    assert_eq!(
        serde_json::to_vec(
            &reopened
                .snapshot(true)
                .unwrap()
                .restore
                .workbench_batch
                .unwrap()
        )
        .unwrap(),
        frozen
    );
    reopened.shutdown().unwrap();
}

#[test]
fn mixed_source_and_native_identity_forgery_never_publishes_a_batch() {
    for mutation in 0..4 {
        let database = TempStore::new(&format!("native-source-invalid-{mutation}"));
        let mut response = native_response();
        match mutation {
            0 => response.collection.provider_id = "usgs.eros.m2m".to_owned(),
            1 => response.products[0].provider_id = "usgs.eros.m2m".to_owned(),
            2 => response.products[0].properties["dataset_alias"] = json!("landsat_ot_c2_l2"),
            _ => {
                response.products[0].product_id = "LC91230452026001LGN00".to_owned();
                response.products[0].properties["entity_id"] = json!("LC91230452026001LGN00");
            }
        }
        let service = production_fixture_service(&database, Arc::new(StaticCatalog { response }));
        assert!(service
            .catalog_search_and_freeze_legacy(&native_request())
            .is_err());
        assert_empty_store(&service);
        service.shutdown().unwrap();
    }
}

#[test]
fn microsoft_browse_receives_native_item_id_not_the_legacy_scene_id() {
    let database = TempStore::new("native-microsoft-browse");
    let browse = Arc::new(RecordingLandsatBrowse::default());
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_browse(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: native_response(),
        }),
        browse.clone(),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "native-microsoft-browse");
    let batch = service
        .catalog_search_and_freeze_legacy(&native_request())
        .unwrap()
        .restore
        .workbench_batch
        .unwrap();
    service
        .landsat_browse(LandsatBrowseRequest {
            request_id: "native-browse-request".to_owned(),
            batch_id: batch.batch_id.clone(),
            item_id: batch.candidates[0].item_id.clone(),
        })
        .unwrap();
    let calls = browse.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.provider_id, MPC_LANDSAT_PROVIDER_ID);
    assert_eq!(calls[0].0.native_collection_id, MPC_LANDSAT_COLLECTION_ID);
    assert_eq!(calls[0].1, NATIVE_ID);
    drop(calls);
    assert_eq!(
        service
            .snapshot(true)
            .unwrap()
            .restore
            .workbench_batch
            .unwrap(),
        batch
    );
    service.shutdown().unwrap();
}

struct NativeManifest;
impl LiveLandsatAssetManifest for NativeManifest {
    fn landsat_asset_manifest(
        &self,
        collection: &str,
        id: &str,
        display: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerLandsatManifest, PlanetaryComputerError> {
        assert!(!cancelled());
        assert_eq!(collection, MPC_LANDSAT_COLLECTION_ID);
        assert_eq!(id, NATIVE_ID);
        let mut manifest = planetary_computer_manifest();
        assert_eq!(display, manifest.display_id);
        manifest.entity_id = NATIVE_ID.to_owned();
        Ok(manifest)
    }
}

#[test]
fn native_microsoft_task_uses_v2_authority_and_reopens_without_changing_source() {
    let database = TempStore::new("native-microsoft-assets");
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_assets(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: native_response(),
        }),
        Arc::new(NativeManifest),
        Arc::new(RecordingWorker::default()),
    );
    bind_aoi(&service, "native-microsoft-assets");
    let batch = service
        .catalog_search_and_freeze_legacy(&native_request())
        .unwrap()
        .restore
        .workbench_batch
        .unwrap();
    let result = service
        .create_landsat_asset_task_legacy(&LandsatCreateBundleTaskRequest {
            request_id: "native-assets-create".to_owned(),
            source_batch_id: batch.batch_id.clone(),
            item_id: batch.candidates[0].item_id.clone(),
            task_id: "native-assets-task".to_owned(),
            output_root: database.0.parent().unwrap().join("native-assets-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-07-29T08:00:00Z".to_owned(),
        })
        .unwrap();
    let task = result
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "native-assets-task")
        .unwrap();
    assert_eq!(task.candidates.len(), 22);
    for item in &task.candidates {
        assert_eq!(item.provider_id, MPC_LANDSAT_PROVIDER_ID);
        assert_eq!(
            item.asset_access
                .as_ref()
                .unwrap()
                .reference
                .provider_product_id,
            NATIVE_ID
        );
        assert_eq!(
            item.execution_policy.as_ref().unwrap().policy_id,
            MPC_LANDSAT_ASSET_POLICY_ID
        );
        assert!(matches!(
            insar_download::classify_frozen_execution(
                item,
                &insar_download::AsfCollectionProfileRegistry,
                &insar_download::AssetExecutionPolicyRegistry
            ),
            insar_download::FrozenExecution::ResolverAuthoritative(_)
        ));
    }
    assert_eq!(result.restore.workbench_batch.as_ref().unwrap(), &batch);
    let immutable = serde_json::to_value(task).unwrap();
    service.shutdown().unwrap();
    let reopened = ApplicationService::new(ApplicationServiceConfig::business_store(&database.0));
    let restored = reopened.snapshot(true).unwrap();
    let restored_task = restored
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "native-assets-task")
        .unwrap();
    assert_eq!(serde_json::to_value(restored_task).unwrap(), immutable);
    assert_eq!(restored.restore.workbench_batch.unwrap(), batch);
    reopened.shutdown().unwrap();
}

#[test]
fn microsoft_scene_does_not_silently_request_a_usgs_bundle() {
    let database = TempStore::new("native-source-no-usgs-bundle");
    let options = Arc::new(RecordingLandsatBundleOptions::default());
    let service =
        ApplicationService::with_catalogue_production_business_runtime_and_landsat_delivery(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(StaticCatalog {
                response: native_response(),
            }),
            options.clone(),
            Arc::new(FixtureWorker),
        );
    bind_aoi(&service, "native-source-no-usgs-bundle");
    let batch = service
        .catalog_search_and_freeze_legacy(&native_request())
        .unwrap()
        .restore
        .workbench_batch
        .unwrap();
    let request = LandsatCreateBundleTaskRequest {
        request_id: "reject-usgs-bundle".to_owned(),
        source_batch_id: batch.batch_id.clone(),
        item_id: batch.candidates[0].item_id.clone(),
        task_id: "must-not-exist".to_owned(),
        output_root: database.0.parent().unwrap().join("no-output"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-07-29T08:00:00Z".to_owned(),
    };
    assert!(service.create_landsat_bundle_task_legacy(&request).is_err());
    assert!(options.calls.lock().unwrap().is_empty());
    let snapshot = service.snapshot(true).unwrap();
    assert!(snapshot.restore.tasks.is_empty());
    assert_eq!(snapshot.restore.workbench_batch.unwrap(), batch);
    service.shutdown().unwrap();
}

fn earth_response() -> CatalogSearchResponse {
    use insar_provider::catalog::EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID;
    let mut response = native_response();
    let profile = CollectionProfileRegistry
        .get(EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID)
        .unwrap();
    response.collection.provider_id = profile.provider_id.to_owned();
    response.collection.native_collection_id = profile.native_collection_id.to_owned();
    response.collection.collection_profile_id = profile.profile_id.to_owned();
    response.collection.identity_rule_id = profile.identity_rule_id.to_owned();
    response.collection.filename_rule_id = profile.filename_rule_id.to_owned();
    let product = &mut response.products[0];
    product.provider_id = profile.provider_id.to_owned();
    product.properties["dataset_alias"] = json!(profile.native_collection_id);
    product.properties["browse_available"] = json!(false);
    response
}

fn earth_request() -> ApplicationCatalogSearchAndFreezeRequest {
    let mut request = native_request();
    request.search.provider_id =
        insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID.to_owned();
    request
}

#[test]
fn earth_search_freezes_and_reopens_without_acquiring_delivery_authority() {
    let database = TempStore::new("earth-source-freeze");
    let browse = Arc::new(RecordingLandsatBrowse::default());
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_browse(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: earth_response(),
        }),
        browse.clone(),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "earth-source-freeze");
    let snapshot = service
        .catalog_search_and_freeze_legacy(&earth_request())
        .unwrap();
    let batch = snapshot.restore.workbench_batch.unwrap();
    assert_eq!(
        batch.provider_id,
        insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID
    );
    let item = &batch.candidates[0];
    assert_eq!(item.metadata["provider_product_id"], NATIVE_ID);
    assert_eq!(
        item.item_id,
        provider_product_key(&batch.provider_id, MPC_LANDSAT_COLLECTION_ID, NATIVE_ID).unwrap()
    );
    assert_eq!(item.metadata["properties"]["browse_available"], false);
    assert!(item.asset_access.is_none() && item.execution_policy.is_none());
    assert!(service
        .landsat_browse(LandsatBrowseRequest {
            request_id: "earth-browse-rejected".to_owned(),
            batch_id: batch.batch_id.clone(),
            item_id: item.item_id.clone(),
        })
        .is_err());
    assert!(browse.calls.lock().unwrap().is_empty());
    assert!(service.snapshot(true).unwrap().restore.tasks.is_empty());
    service.shutdown().unwrap();
    let reopened = ApplicationService::new(ApplicationServiceConfig::business_store(&database.0));
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

#[derive(Default)]
struct CountingManifest(AtomicU64);

impl LiveLandsatAssetManifest for CountingManifest {
    fn landsat_asset_manifest(
        &self,
        collection: &str,
        id: &str,
        display: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerLandsatManifest, PlanetaryComputerError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        NativeManifest.landsat_asset_manifest(collection, id, display, cancelled)
    }
}

#[test]
fn earth_search_rejects_download_before_calling_another_sources_manifest_port() {
    let database = TempStore::new("earth-no-manifest");
    let manifest = Arc::new(CountingManifest::default());
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_assets(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: earth_response(),
        }),
        manifest.clone(),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "earth-no-manifest");
    let batch = service
        .catalog_search_and_freeze_legacy(&earth_request())
        .unwrap()
        .restore
        .workbench_batch
        .unwrap();
    let output_root = database.0.with_extension("earth-output");
    let request = LandsatCreateBundleTaskRequest {
        request_id: "earth-download-rejected".to_owned(),
        source_batch_id: batch.batch_id.clone(),
        item_id: batch.candidates[0].item_id.clone(),
        task_id: "earth-must-not-create".to_owned(),
        output_root: output_root.clone(),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-07-29T08:00:00Z".to_owned(),
    };
    assert!(service.create_landsat_asset_task_legacy(&request).is_err());
    assert_eq!(
        manifest.0.load(Ordering::Relaxed),
        0,
        "search-only must be refused before any manifest discovery"
    );
    assert!(service.create_landsat_bundle_task_legacy(&request).is_err());
    let restored = service.snapshot(true).unwrap();
    assert!(restored.restore.tasks.is_empty());
    assert_eq!(restored.restore.workbench_batch.unwrap(), batch);
    assert!(!output_root.exists());
    service.shutdown().unwrap();
}

#[test]
fn earth_search_rejects_consistent_native_id_forgery_and_preview_grant() {
    for mutation in 0..2 {
        let database = TempStore::new(&format!("earth-invalid-authority-{mutation}"));
        let mut response = earth_response();
        if mutation == 0 {
            response.products[0].product_id = "LC91230452026001LGN00".to_owned();
            response.products[0].properties["entity_id"] = json!("LC91230452026001LGN00");
        } else {
            response.products[0].properties["browse_available"] = json!(true);
        }
        let service = production_fixture_service(&database, Arc::new(StaticCatalog { response }));
        assert!(
            service
                .catalog_search_and_freeze_legacy(&earth_request())
                .is_err(),
            "mutation {mutation}"
        );
        assert_empty_store(&service);
        service.shutdown().unwrap();
    }
}
