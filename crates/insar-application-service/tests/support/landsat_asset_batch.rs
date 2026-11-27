use super::*;
use insar_application_service::LandsatCreateAssetBatchTaskRequest;

#[derive(Default)]
struct BatchManifest {
    rejected: Vec<String>,
    block: AtomicBool,
    entered: AtomicU64,
    active: AtomicU64,
    maximum: AtomicU64,
}

impl LiveLandsatAssetManifest for BatchManifest {
    fn landsat_asset_manifest(
        &self,
        _collection: &str,
        native: &str,
        display: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerLandsatManifest, PlanetaryComputerError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        while self.block.load(Ordering::SeqCst) && !cancelled() {
            thread::sleep(Duration::from_millis(2));
        }
        let mut manifest = planetary_computer_manifest();
        let old = manifest.display_id.clone();
        for asset in &mut manifest.assets {
            asset.file_name = asset.file_name.replace(&old, display);
            asset.stable_href = asset.stable_href.replace(&old, display);
        }
        manifest.entity_id = if self.rejected.iter().any(|id| id == native) {
            "wrong-scene".to_owned()
        } else {
            native.to_owned()
        };
        manifest.stac_item_id = native.to_owned();
        manifest.display_id = display.to_owned();
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(manifest)
    }
}

fn response() -> CatalogSearchResponse {
    let mut response = mpc_landsat_response();
    let first = serde_json::to_string(&response.products[0]).unwrap();
    response.products.push(
        serde_json::from_str(
            &first
                .replace("20260101", "20260102")
                .replace("20260103", "20260104")
                .replace("2026-01-01", "2026-01-02")
                .replace("2026/01/01", "2026/01/02")
                .replace("2026001LGN", "2026002LGN"),
        )
        .unwrap(),
    );
    response.stats.returned_count = 2;
    response.stats.unique_provider_product_count = 2;
    response.stats.exact_matched_count = 2;
    response.stats.mapped_record_count = 2;
    response
}

fn service(database: &TempStore, manifest: Arc<BatchManifest>) -> ApplicationService {
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_assets(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: response(),
        }),
        manifest,
        Arc::new(RecordingWorker::default()),
    );
    bind_aoi(&service, "batch-landsat");
    service
        .catalog_search_and_freeze_legacy(&mpc_landsat_request())
        .unwrap();
    service
}

fn request(
    service: &ApplicationService,
    database: &TempStore,
) -> LandsatCreateAssetBatchTaskRequest {
    let source = service.core_restore(true).unwrap().core.workbench.unwrap();
    LandsatCreateAssetBatchTaskRequest {
        request_id: "landsat-batch-request".to_owned(),
        source_batch_id: source.batch_id,
        expected_workbench_revision: source.revision,
        item_ids: vec![
            frozen_mpc_landsat_item_id().to_owned(),
            frozen_mpc_landsat_item_id().replace("20260101", "20260102"),
        ],
        asset_keys: Vec::new(),
        task_id: "landsat-batch-task".to_owned(),
        output_root: database.0.with_extension("output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-29T00:00:00Z".to_owned(),
    }
}

#[test]
fn two_scenes_create_one_task_with_44_files_and_preserve_search() {
    let database = TempStore::new("landsat-batch-two");
    let manifest = Arc::new(BatchManifest::default());
    let service = service(&database, manifest.clone());
    let mut request = request(&service, &database);
    let current = service.core_restore(true).unwrap().core.workbench.unwrap();
    let selected = service
        .update_workbench_selection(&insar_core::WorkbenchSelectionCommand {
            batch_id: current.batch_id.clone(),
            expected_revision: current.revision,
            kind: insar_core::WorkbenchSelectionCommandKind::SelectAll,
            item_ids: vec![],
            page_index: 0,
            page_size: 1,
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
    let visible = service
        .workbench_page(&StoreWorkbenchPageRequest {
            batch_id: current.batch_id,
            expected_revision: selected.revision,
            page_index: 0,
            page_size: 1,
        })
        .unwrap();
    assert_eq!(visible.page.items.len(), 1);
    assert_eq!(visible.page.checked_count, 2);
    request.expected_workbench_revision = selected.revision;
    request.item_ids.clear();
    let before = service.snapshot(true).unwrap().restore.workbench_batch;
    let result = service.create_landsat_asset_batch_task(&request).unwrap();
    assert_eq!(result.admission.admitted_count, 2);
    let summary = &result.snapshot.as_ref().unwrap().core.tasks[0];
    assert_eq!(summary.selected_count, 44);
    assert_eq!(summary.delivery_requested_count, Some(2));
    assert_eq!(summary.delivery_rejected_count, Some(0));
    let restored = service.snapshot(true).unwrap();
    assert_eq!(restored.restore.workbench_batch, before);
    assert_eq!(restored.restore.tasks.len(), 1);
    let task = &restored.restore.tasks[0];
    assert_eq!(task.output_root, request.output_root);
    assert_eq!(task.candidates.len(), 44);
    let by_scene =
        task.candidates
            .iter()
            .fold(BTreeMap::<String, usize>::new(), |mut counts, item| {
                *counts
                    .entry(item.metadata["source_item_id"].as_str().unwrap().to_owned())
                    .or_default() += 1;
                counts
            });
    assert_eq!(by_scene.len(), 2);
    assert!(by_scene.values().all(|count| *count == 22));
    assert!(task.candidates.iter().all(|item| item
        .asset_access
        .as_ref()
        .unwrap()
        .expected_size_bytes
        .is_some()));
    assert!(manifest.maximum.load(Ordering::SeqCst) <= 2);
    let scene_page = service
        .task_source_batch_page(&insar_core::StoreTaskSourceBatchPageRequest {
            task_id: request.task_id.clone(),
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(
        scene_page.total_count, 2,
        "source results are scenes, not 44 execution files"
    );
    assert_eq!(scene_page.source_batch_id, request.source_batch_id);
    assert!(scene_page.items.iter().all(|item| item.belongs_to_task));
    service.shutdown().unwrap();
}

#[test]
fn store_owned_empty_selection_and_stale_revision_fail_before_manifest() {
    let database = TempStore::new("landsat-batch-store-selection-guards");
    let manifest = Arc::new(BatchManifest::default());
    let service = service(&database, manifest.clone());
    let mut request = request(&service, &database);
    request.item_ids.clear();
    assert!(
        service.create_landsat_asset_batch_task(&request).is_err(),
        "empty item_ids must not mean select-all when Store has no checked rows"
    );
    assert_eq!(manifest.entered.load(Ordering::SeqCst), 0);

    let current = service.core_restore(true).unwrap().core.workbench.unwrap();
    let selected = service
        .update_workbench_selection(&insar_core::WorkbenchSelectionCommand {
            batch_id: current.batch_id,
            expected_revision: current.revision,
            kind: insar_core::WorkbenchSelectionCommandKind::SelectAll,
            item_ids: vec![],
            page_index: 0,
            page_size: 1,
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
    assert!(selected.revision > request.expected_workbench_revision);
    assert!(
        service.create_landsat_asset_batch_task(&request).is_err(),
        "Store-owned selection requires the exact post-selection revision"
    );
    assert_eq!(
        manifest.entered.load(Ordering::SeqCst),
        0,
        "selection guards must fail before any live manifest request"
    );
    service.shutdown().unwrap();
}

#[test]
fn partial_rejections_are_per_scene_and_survive_store_reopen() {
    let database = TempStore::new("landsat-batch-partial");
    let port = Arc::new(BatchManifest {
        rejected: vec!["LC09_L2SP_123045_20260102_02_T1".to_owned()],
        ..Default::default()
    });
    let service = service(&database, port);
    let request = request(&service, &database);
    let result = service.create_landsat_asset_batch_task(&request).unwrap();
    assert_eq!(result.admission.requested_count, 2);
    assert_eq!(result.admission.admitted_count, 1);
    assert_eq!(result.admission.rejected_count, 1);
    assert_eq!(
        result.admission.rejections[0].source_item_id,
        request.item_ids[1]
    );
    assert_eq!(
        result.snapshot.as_ref().unwrap().core.tasks[0].selected_count,
        22
    );
    assert_eq!(
        service.task_delivery_admission(&request.task_id).unwrap(),
        Some(result.admission.clone())
    );
    service.shutdown().unwrap();
    let reopened =
        ApplicationService::with_catalogue_production_business_runtime_and_landsat_assets(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(StaticCatalog {
                response: response(),
            }),
            Arc::new(BatchManifest::default()),
            Arc::new(RecordingWorker::default()),
        );
    assert_eq!(
        reopened.task_delivery_admission(&request.task_id).unwrap(),
        Some(result.admission)
    );
    assert_eq!(reopened.snapshot(true).unwrap().restore.tasks.len(), 1);
    let scene_page = reopened
        .task_source_batch_page(&insar_core::StoreTaskSourceBatchPageRequest {
            task_id: request.task_id.clone(),
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(scene_page.total_count, 2);
    assert_eq!(
        scene_page
            .items
            .iter()
            .filter(|item| item.belongs_to_task)
            .count(),
        1
    );
    reopened.shutdown().unwrap();
}

#[test]
fn all_rejected_and_invalid_inputs_never_create_a_task() {
    let database = TempStore::new("landsat-batch-rejected");
    let port = Arc::new(BatchManifest {
        rejected: vec![
            "LC09_L2SP_123045_20260101_02_T1".to_owned(),
            "LC09_L2SP_123045_20260102_02_T1".to_owned(),
        ],
        ..Default::default()
    });
    let service = service(&database, port.clone());
    let request = request(&service, &database);
    let mut invalid = request.clone();
    invalid.item_ids.push(invalid.item_ids[0].clone());
    assert!(service.create_landsat_asset_batch_task(&invalid).is_err());
    invalid = request.clone();
    invalid.expected_workbench_revision += 1;
    assert!(service.create_landsat_asset_batch_task(&invalid).is_err());
    assert_eq!(port.entered.load(Ordering::SeqCst), 0);
    let result = service.create_landsat_asset_batch_task(&request).unwrap();
    assert!(result.snapshot.is_none());
    assert_eq!(result.admission.rejected_count, 2);
    assert!(service.snapshot(true).unwrap().restore.tasks.is_empty());
    service.shutdown().unwrap();
}

#[test]
fn cancellation_and_duplicate_inflight_request_never_commit_partial_files() {
    let database = TempStore::new("landsat-batch-cancel");
    let port = Arc::new(BatchManifest {
        block: AtomicBool::new(true),
        ..Default::default()
    });
    let service = Arc::new(service(&database, port.clone()));
    let request = request(&service, &database);
    let operation = request.clone();
    let worker_service = service.clone();
    let worker = thread::spawn(move || worker_service.create_landsat_asset_batch_task(&operation));
    let deadline = Instant::now() + Duration::from_secs(10);
    while port.entered.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    let duplicate = service.create_landsat_asset_batch_task(&request);
    let progress = service
        .optical_preparation_progress(&request.request_id)
        .unwrap();
    assert!(service.cancel_request(&request.request_id).unwrap());
    port.block.store(false, Ordering::SeqCst);
    assert_eq!(
        duplicate.unwrap_err().code(),
        "LANDSAT_PUBLIC_ASSET_REQUEST_DUPLICATE"
    );
    assert!(worker.join().unwrap().is_err());
    let progress = progress.expect("Landsat observes the same preparation registry");
    assert_eq!((progress.requested_count, progress.prepared_count), (2, 0));
    assert!(service
        .optical_preparation_progress(&request.request_id)
        .unwrap()
        .is_none());
    assert!(service.snapshot(true).unwrap().restore.tasks.is_empty());
    service.shutdown().unwrap();
}
