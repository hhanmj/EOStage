//! Exact catalogue mapping + real Store, with synthetic USGS and transfer ports.
use super::*;
use insar_application_service::LandsatCreateBundleBatchTaskRequest;

#[derive(Default)]
struct OfficialBatch {
    rejected: Vec<String>,
    mismatched: bool,
    block: AtomicBool,
    entered: AtomicU64,
    options: AtomicU64,
    active: AtomicU64,
    maximum: AtomicU64,
}

fn two_scenes(mpc: bool) -> CatalogSearchResponse {
    let mut response = if mpc {
        mpc_landsat_response()
    } else {
        landsat_response()
    };
    response.collection.production_enabled = true;
    if mpc {
        response.products[0].properties["data_type_l2"] = json!("LANDSAT_9_L2SP");
        response.products[0].properties["acquisition_precision"] = json!("timestamp");
        response.products[0].acquisition_datetime = Some("2026-01-01T02:33:20Z".into());
    }
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

fn earth_two_scenes() -> CatalogSearchResponse {
    let mut response = two_scenes(true);
    let profile = insar_provider::catalog::CollectionProfileRegistry
        .get(insar_provider::catalog::EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID)
        .unwrap();
    response.collection.provider_id = profile.provider_id.to_owned();
    response.collection.native_collection_id = profile.native_collection_id.to_owned();
    response.collection.collection_profile_id = profile.profile_id.to_owned();
    response.collection.identity_rule_id = profile.identity_rule_id.to_owned();
    response.collection.filename_rule_id = profile.filename_rule_id.to_owned();
    for product in &mut response.products {
        product.provider_id = profile.provider_id.to_owned();
        product.properties["dataset_alias"] = json!(profile.native_collection_id);
        product.properties["browse_available"] = json!(false);
    }
    response
}

fn earth_request() -> ApplicationCatalogSearchAndFreezeRequest {
    let mut request = mpc_landsat_request();
    request.search.provider_id =
        insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID.to_owned();
    request.search.native_collection_id =
        insar_provider::catalog::EARTH_SEARCH_LANDSAT_COLLECTION_ID.to_owned();
    request
}

impl LiveLandsatBundleOptions for OfficialBatch {
    fn resolve_official_product(
        &self,
        display: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<ProviderProduct, insar_provider::usgs_m2m::UsgsM2mError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.block.load(Ordering::Acquire) && display.contains("20260101") && !cancelled() {
            assert!(
                Instant::now() < deadline,
                "isolated lookup must finish or be cancelled"
            );
            thread::sleep(Duration::from_millis(2));
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        let mut product = two_scenes(false)
            .products
            .into_iter()
            .find(|p| p.display_name == display)
            .unwrap();
        if self.mismatched {
            product.properties["wrs_row"] = json!("046");
        }
        Ok(product)
    }
    fn full_bundle_option(
        &self,
        dataset: &str,
        entity: &str,
        display: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mFullBundleOption, insar_provider::usgs_m2m::UsgsM2mError> {
        assert!(!cancelled());
        assert_eq!(dataset, "landsat_ot_c2_l2");
        let product = two_scenes(false)
            .products
            .into_iter()
            .find(|p| p.display_name == display)
            .unwrap();
        assert_eq!(entity, product.product_id);
        self.options.fetch_add(1, Ordering::SeqCst);
        let mut option = landsat_bundle_option();
        option.entity_id = entity.to_owned();
        option.display_id = display.to_owned();
        option.expected_file_name = format!("{display}.tar");
        if self.rejected.iter().any(|id| id == display) {
            option.entity_id = "another-product".to_owned();
        }
        Ok(option)
    }
}

fn setup(
    database: &TempStore,
    port: Arc<OfficialBatch>,
    mpc: bool,
) -> (
    Arc<ApplicationService>,
    LandsatCreateBundleBatchTaskRequest,
    insar_core::FrozenSearchBatch,
) {
    let service = Arc::new(
        ApplicationService::with_catalogue_production_business_runtime_and_landsat_delivery(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(StaticCatalog {
                response: two_scenes(mpc),
            }),
            port,
            Arc::new(RecordingWorker::default()),
        ),
    );
    bind_aoi(&service, "official-batch");
    let request = if mpc {
        mpc_landsat_request()
    } else {
        landsat_request()
    };
    let batch = service
        .catalog_search_and_freeze_legacy(&request)
        .unwrap()
        .restore
        .workbench_batch
        .unwrap();
    let revision = service
        .core_restore(true)
        .unwrap()
        .core
        .workbench
        .unwrap()
        .revision;
    let request = LandsatCreateBundleBatchTaskRequest {
        request_id: "official-batch-request".into(),
        source_batch_id: batch.batch_id.clone(),
        expected_workbench_revision: revision,
        item_ids: batch.candidates.iter().map(|i| i.item_id.clone()).collect(),
        asset_keys: Vec::new(),
        task_id: "official-batch-task".into(),
        output_root: database.0.with_extension("output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-30T00:00:00Z".into(),
    };
    (service, request, batch)
}

fn setup_earth(
    database: &TempStore,
    port: Arc<OfficialBatch>,
) -> (
    Arc<ApplicationService>,
    LandsatCreateBundleBatchTaskRequest,
    insar_core::FrozenSearchBatch,
) {
    let service = Arc::new(
        ApplicationService::with_catalogue_production_business_runtime_and_landsat_delivery(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(StaticCatalog {
                response: earth_two_scenes(),
            }),
            port,
            Arc::new(RecordingWorker::default()),
        ),
    );
    bind_aoi(&service, "official-batch-earth");
    let batch = service
        .catalog_search_and_freeze_legacy(&earth_request())
        .unwrap()
        .restore
        .workbench_batch
        .unwrap();
    let revision = service
        .core_restore(true)
        .unwrap()
        .core
        .workbench
        .unwrap()
        .revision;
    let request = LandsatCreateBundleBatchTaskRequest {
        request_id: "official-batch-earth-request".into(),
        source_batch_id: batch.batch_id.clone(),
        expected_workbench_revision: revision,
        item_ids: batch
            .candidates
            .iter()
            .map(|item| item.item_id.clone())
            .collect(),
        asset_keys: Vec::new(),
        task_id: "official-batch-earth-task".into(),
        output_root: database.0.with_extension("earth-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-10-01T00:00:00Z".into(),
    };
    (service, request, batch)
}

#[test]
fn mpc_and_usgs_two_archives_share_one_task_without_changing_source_or_delivery() {
    for mpc in [true, false] {
        let database = TempStore::new("official-batch-two");
        let port = Arc::new(OfficialBatch::default());
        let (service, request, source) = setup(&database, port.clone(), mpc);
        let result = service.create_landsat_bundle_batch_task(&request).unwrap();
        assert_eq!(result.admission.admitted_count, 2);
        let summary = &result.snapshot.as_ref().unwrap().core.tasks[0];
        assert_eq!(summary.selected_count, 2);
        assert_eq!(summary.delivery_requested_count, Some(2));
        assert_eq!(summary.delivery_rejected_count, Some(0));
        let state = service.snapshot(true).unwrap();
        assert_eq!(state.restore.workbench_batch.as_ref(), Some(&source));
        assert_eq!(state.restore.tasks.len(), 1);
        let task = &state.restore.tasks[0];
        assert_eq!(task.provider_id, "usgs.eros.m2m");
        assert_eq!(task.output_root, request.output_root);
        assert_eq!(task.aoi_geojson, source.aoi_geojson);
        assert_eq!(task.candidates.len(), 2);
        for (item, id) in task.candidates.iter().zip(&request.item_ids) {
            assert_eq!(item.metadata["source_item_id"], *id);
            let access = item.asset_access.as_ref().unwrap();
            assert!(access.file_name.as_ref().unwrap().ends_with(".tar"));
            assert!(access.expected_size_bytes.unwrap() > 0);
            assert!(matches!(
                insar_download::classify_frozen_execution(
                    item,
                    &insar_download::AsfCollectionProfileRegistry,
                    &insar_download::AssetExecutionPolicyRegistry
                ),
                insar_download::FrozenExecution::ResolverAuthoritative(_)
            ));
        }
        assert_eq!(port.entered.load(Ordering::SeqCst), if mpc { 2 } else { 0 });
        assert_eq!(port.options.load(Ordering::SeqCst), 2);
        assert!(port.maximum.load(Ordering::SeqCst) <= 2);
        // Store accepts an identical committed request idempotently; it must
        // return the same task, not turn a safe replay into a new download.
        let replay = service.create_landsat_bundle_batch_task(&request).unwrap();
        assert_eq!(replay.admission, result.admission);
        let replayed = service.snapshot(true).unwrap();
        assert_eq!(replayed.restore.tasks.len(), 1);
        assert_eq!(&replayed.restore.tasks[0], task);
        service.shutdown().unwrap();
        let reopened =
            ApplicationService::new(ApplicationServiceConfig::business_store(&database.0));
        let page = reopened
            .task_source_batch_page(&StoreTaskSourceBatchPageRequest {
                task_id: request.task_id.clone(),
                page_index: 0,
                page_size: 24,
            })
            .unwrap();
        assert_eq!(page.source_batch_id, source.batch_id);
        assert_eq!(page.total_count, 2);
        assert!(page.items.iter().all(|i| i.belongs_to_task));
        assert_eq!(
            reopened.task_delivery_admission(&request.task_id).unwrap(),
            Some(result.admission)
        );
        reopened.shutdown().unwrap();
    }
}

#[test]
fn earth_search_catalogue_can_handoff_two_scenes_to_one_usgs_archive_task() {
    let database = TempStore::new("official-batch-earth");
    let port = Arc::new(OfficialBatch::default());
    let (service, request, source) = setup_earth(&database, port.clone());

    let result = service.create_landsat_bundle_batch_task(&request).unwrap();
    assert_eq!(result.admission.requested_count, 2);
    assert_eq!(result.admission.admitted_count, 2);
    assert_eq!(result.admission.rejected_count, 0);
    assert_eq!(
        source.provider_id,
        insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID
    );
    let task = &result.snapshot.as_ref().unwrap().core.tasks[0];
    assert_eq!(
        task.provider_id,
        insar_provider::catalog::LANDSAT_C2_L2_PROVIDER_ID
    );
    assert_eq!(task.selected_count, 2);
    assert_eq!(port.entered.load(Ordering::SeqCst), 2);
    assert_eq!(port.options.load(Ordering::SeqCst), 2);
    assert!(port.maximum.load(Ordering::SeqCst) <= 2);
    assert_eq!(
        service
            .snapshot(true)
            .unwrap()
            .restore
            .workbench_batch
            .as_ref(),
        Some(&source)
    );
    service.shutdown().unwrap();
}

#[test]
fn archive_partial_and_total_rejection_preserve_exact_source_ids_and_reopen_report() {
    for rejected_count in [1, 2] {
        let database = TempStore::new("official-batch-rejection");
        let rejected = two_scenes(false)
            .products
            .into_iter()
            .rev()
            .take(rejected_count)
            .map(|p| p.display_name)
            .collect();
        let (service, request, source) = setup(
            &database,
            Arc::new(OfficialBatch {
                rejected,
                ..Default::default()
            }),
            true,
        );
        let result = service.create_landsat_bundle_batch_task(&request).unwrap();
        assert_eq!(result.admission.rejected_count, rejected_count as u64);
        assert_eq!(
            result.admission.admitted_count + result.admission.rejected_count,
            2
        );
        for rejection in &result.admission.rejections {
            assert!(request.item_ids.contains(&rejection.source_item_id));
            assert_eq!(
                rejection.reason_code,
                "USGS_LANDSAT_BUNDLE_FROZEN_EVIDENCE_INVALID"
            );
        }
        assert_eq!(result.snapshot.is_none(), rejected_count == 2);
        assert_eq!(
            service.snapshot(true).unwrap().restore.workbench_batch,
            Some(source.clone())
        );
        assert_eq!(
            service.snapshot(true).unwrap().restore.tasks.len(),
            usize::from(rejected_count != 2)
        );
        service.shutdown().unwrap();
        if rejected_count == 1 {
            let reopened =
                ApplicationService::new(ApplicationServiceConfig::business_store(&database.0));
            assert_eq!(
                reopened.task_delivery_admission(&request.task_id).unwrap(),
                Some(result.admission)
            );
            let page = reopened
                .task_source_batch_page(&StoreTaskSourceBatchPageRequest {
                    task_id: request.task_id,
                    page_index: 0,
                    page_size: 24,
                })
                .unwrap();
            assert_eq!(page.items.iter().filter(|i| i.belongs_to_task).count(), 1);
            assert_eq!(page.items[0].item.item_id, request.item_ids[0]);
            reopened.shutdown().unwrap();
        }
    }
}

fn wait_other_scene(service: &ApplicationService, request_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if service
            .optical_preparation_progress(request_id)
            .unwrap()
            .is_some_and(|p| p.prepared_count == 1)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "completed scene progress must not wait for blocked first scene"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn archive_cancel_and_duplicate_are_bounded_without_partial_task_commit() {
    let database = TempStore::new("official-batch-cancel");
    let port = Arc::new(OfficialBatch {
        block: AtomicBool::new(true),
        ..Default::default()
    });
    let (service, request, _) = setup(&database, port.clone(), true);
    let owner = service.clone();
    let copied = request.clone();
    let pending = thread::spawn(move || owner.create_landsat_bundle_batch_task(&copied));
    wait_other_scene(&service, &request.request_id);
    assert!(service.create_landsat_bundle_batch_task(&request).is_err());
    assert!(service.cancel_request(&request.request_id).unwrap());
    assert!(pending.join().unwrap().is_err());
    assert!(service.snapshot(true).unwrap().restore.tasks.is_empty());
    assert_eq!(port.options.load(Ordering::SeqCst), 1);
    assert!(service
        .optical_preparation_progress(&request.request_id)
        .unwrap()
        .is_none());
    service.shutdown().unwrap();
}

#[test]
fn archive_source_change_during_prepare_commits_from_the_named_batch() {
    let database = TempStore::new("official-batch-change");
    let port = Arc::new(OfficialBatch {
        block: AtomicBool::new(true),
        ..Default::default()
    });
    let (service, request, _) = setup(&database, port.clone(), true);
    let owner = service.clone();
    let copied = request.clone();
    let pending = thread::spawn(move || owner.create_landsat_bundle_batch_task(&copied));
    wait_other_scene(&service, &request.request_id);
    // A new search replaces the Workbench slot with a different batch. The
    // pending task was admitted from the batch its request named and that
    // batch is unchanged, so the task must still commit: unified search keeps
    // one frozen batch per source and the slot has no authority over a named
    // batch's admission.
    let mut next = mpc_landsat_request();
    next.request_id = "new-source-request".into();
    next.batch_id = "new-source-batch".into();
    let changed = service.catalog_search_and_freeze_legacy(&next);
    port.block.store(false, Ordering::Release);
    let committed = pending.join().unwrap().expect("named batch is unchanged");
    changed.unwrap();
    assert_eq!(committed.admission.admitted_count, 2);
    assert!(committed.snapshot.is_some());
    assert_eq!(
        service
            .snapshot(true)
            .unwrap()
            .restore
            .workbench_batch
            .unwrap()
            .batch_id,
        next.batch_id
    );
    service.shutdown().unwrap();
}

#[test]
fn archive_duplicate_ids_stale_revision_and_mismatched_products_cannot_authorize_download() {
    let database = TempStore::new("official-batch-invalid");
    let port = Arc::new(OfficialBatch {
        mismatched: true,
        ..Default::default()
    });
    let (service, request, _) = setup(&database, port.clone(), true);
    let mut invalid = request.clone();
    invalid.item_ids[1] = invalid.item_ids[0].clone();
    assert!(service.create_landsat_bundle_batch_task(&invalid).is_err());
    invalid = request.clone();
    invalid.expected_workbench_revision += 1;
    assert!(service.create_landsat_bundle_batch_task(&invalid).is_err());
    assert_eq!(port.entered.load(Ordering::SeqCst), 0);
    let rejected = service.create_landsat_bundle_batch_task(&request).unwrap();
    assert_eq!(rejected.admission.rejected_count, 2);
    assert!(rejected.snapshot.is_none());
    assert_eq!(port.options.load(Ordering::SeqCst), 0);
    assert!(service.snapshot(true).unwrap().restore.tasks.is_empty());
    service.shutdown().unwrap();
}
