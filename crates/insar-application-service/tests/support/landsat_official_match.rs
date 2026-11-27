use super::*;

struct MatchedOfficialPort {
    product: ProviderProduct,
    matches: AtomicU64,
    options: AtomicU64,
    block: bool,
    release: AtomicBool,
}

impl LiveLandsatBundleOptions for MatchedOfficialPort {
    fn resolve_official_product(
        &self,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<ProviderProduct, insar_provider::usgs_m2m::UsgsM2mError> {
        assert_eq!(display_id, "LC09_L2SP_123045_20260101_20260103_02_T1");
        self.matches.fetch_add(1, Ordering::SeqCst);
        while self.block && !cancelled() && !self.release.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
        Ok(self.product.clone())
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
        assert_eq!(entity, "LC91230452026001LGN00");
        assert_eq!(display, self.product.display_name);
        self.options.fetch_add(1, Ordering::SeqCst);
        Ok(landsat_bundle_option())
    }
}

fn setup(
    database: &TempStore,
    product: ProviderProduct,
    block: bool,
) -> (
    Arc<ApplicationService>,
    Arc<MatchedOfficialPort>,
    insar_core::FrozenSearchBatch,
) {
    let port = Arc::new(MatchedOfficialPort {
        product,
        matches: AtomicU64::new(0),
        options: AtomicU64::new(0),
        block,
        release: AtomicBool::new(false),
    });
    // Match the actual PlanetaryComputer mapper, not USGS metadata with only
    // its provider label changed. The channels have distinct type vocabularies.
    let mut response = mpc_landsat_response();
    response.products[0].properties["data_type_l2"] = json!("LANDSAT_9_L2SP");
    response.products[0].properties["acquisition_precision"] = json!("timestamp");
    response.products[0].acquisition_datetime = Some("2026-01-01T02:33:20Z".into());
    let service = Arc::new(
        ApplicationService::with_catalogue_production_business_runtime_and_landsat_delivery(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(StaticCatalog { response }),
            port.clone(),
            Arc::new(RecordingWorker::default()),
        ),
    );
    bind_aoi(&service, "official-match");
    let batch = service
        .catalog_search_and_freeze_legacy(&mpc_landsat_request())
        .unwrap()
        .restore
        .workbench_batch
        .unwrap();
    (service, port, batch)
}

fn request(batch: &insar_core::FrozenSearchBatch) -> LandsatCreateBundleTaskRequest {
    LandsatCreateBundleTaskRequest {
        request_id: "match-official-request".into(),
        source_batch_id: batch.batch_id.clone(),
        item_id: batch.candidates[0].item_id.clone(),
        task_id: "match-official-task".into(),
        output_root: PathBuf::from("fixture-official-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-29T00:00:00Z".into(),
    }
}

#[test]
fn mpc_selection_creates_exact_usgs_archive_without_relabelling_search_or_assets() {
    let database = TempStore::new("mpc-official-match");
    let official = landsat_response().products.remove(0);
    let (service, port, batch) = setup(&database, official, false);
    let req = request(&batch);
    let created = service
        .create_landsat_bundle_task(&req)
        .expect("MPC selection resolves official identity");
    assert_eq!(created.core.tasks.len(), 1);
    assert_eq!(created.core.tasks[0].provider_id, "usgs.eros.m2m");
    assert_eq!(created.core.tasks[0].selected_count, 1);
    let snapshot = service.snapshot(true).unwrap();
    assert_eq!(snapshot.restore.workbench_batch.as_ref(), Some(&batch));
    assert_eq!(
        batch.candidates[0].metadata["properties"]["data_type_l2"],
        "LANDSAT_9_L2SP"
    );
    let task = &snapshot.restore.tasks[0];
    assert_eq!(task.candidates.len(), 1);
    let item = &task.candidates[0];
    assert_eq!(item.item_id, frozen_landsat_item_id());
    assert_eq!(item.metadata["properties"]["data_type_l2"], "OLI_TIRS_L2SP");
    assert!(
        matches!(
            insar_download::classify_frozen_execution(
                item,
                &insar_download::AsfCollectionProfileRegistry,
                &insar_download::AssetExecutionPolicyRegistry
            ),
            insar_download::FrozenExecution::ResolverAuthoritative(_)
        ),
        "the matched archive must be executable by the production Resolver policy"
    );
    assert_eq!(item.metadata["source_item_id"], batch.candidates[0].item_id);
    assert_eq!(
        item.asset_access.as_ref().unwrap().reference.provider_id,
        "usgs.eros.m2m"
    );
    assert_eq!(task.query["derived_from_batch_id"], batch.batch_id);
    assert_eq!(port.matches.load(Ordering::SeqCst), 1);
    assert_eq!(port.options.load(Ordering::SeqCst), 1);
    service.shutdown().unwrap();
    let reopened = ApplicationService::new(ApplicationServiceConfig::business_store(&database.0));
    let source = reopened
        .task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: req.task_id,
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(source.source_batch_id, batch.batch_id);
    assert_eq!(source.items[0].item.item_id, batch.candidates[0].item_id);
    assert!(source.items[0].belongs_to_task);
    reopened.shutdown().unwrap();
}

#[test]
fn official_match_rejects_reprocessing_and_scientific_mismatch_before_option_or_store() {
    for mutation in 0..7 {
        let database = TempStore::new(&format!("official-conflict-{mutation}"));
        let mut product = landsat_response().products.remove(0);
        match mutation {
            0 => {
                product.display_name = product.display_name.replace("20260103", "20260104");
                product.properties["landsat_product_id"] = json!(product.display_name);
            }
            1 => product.properties["wrs_row"] = json!("046"),
            2 => product.properties["spacecraft"] = json!("landsat_8"),
            3 => product.properties["processing_level"] = json!("l2sr"),
            4 => product.provider_id = "microsoft.planetary-computer".into(),
            // ProviderProduct is already normalized by both source mappers to
            // three-digit WRS coordinates; do not weaken that shared contract.
            5 => product.properties["wrs_row"] = json!("45"),
            _ => product.properties["data_type_l2"] = json!("OLI_TIRS_L2SR"),
        }
        let (service, port, batch) = setup(&database, product, false);
        assert!(service
            .create_landsat_bundle_task(&request(&batch))
            .is_err());
        assert_eq!(port.options.load(Ordering::SeqCst), 0);
        assert!(service.snapshot(true).unwrap().restore.tasks.is_empty());
        assert_eq!(
            service.snapshot(true).unwrap().restore.workbench_batch,
            Some(batch)
        );
        service.shutdown().unwrap();
    }
}

#[test]
fn cancelling_official_lookup_and_duplicate_request_create_no_task() {
    let database = TempStore::new("official-cancel");
    let (service, port, batch) = setup(&database, landsat_response().products.remove(0), true);
    let req = request(&batch);
    let copy = request(&batch);
    let running = service.clone();
    let handle = thread::spawn(move || running.create_landsat_bundle_task(&copy));
    let deadline = Instant::now() + Duration::from_secs(3);
    while port.matches.load(Ordering::SeqCst) == 0
        && !handle.is_finished()
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(5));
    }
    let entered = port.matches.load(Ordering::SeqCst);
    if entered > 0 {
        assert!(service.create_landsat_bundle_task(&req).is_err());
    }
    let _ = service.cancel_request(&req.request_id);
    assert!(handle.join().unwrap().is_err());
    assert_eq!(
        entered, 1,
        "lookup must be reached, not the obsolete source prohibition"
    );
    assert_eq!(port.options.load(Ordering::SeqCst), 0);
    assert!(service.snapshot(true).unwrap().restore.tasks.is_empty());
    service.shutdown().unwrap();
}

#[test]
fn changing_search_during_official_lookup_commits_from_the_named_batch() {
    let database = TempStore::new("official-source-changed");
    let (service, port, batch) = setup(&database, landsat_response().products.remove(0), true);
    let req = request(&batch);
    let running = service.clone();
    let handle = thread::spawn(move || running.create_landsat_bundle_task(&req));
    let deadline = Instant::now() + Duration::from_secs(3);
    while port.matches.load(Ordering::Acquire) == 0
        && !handle.is_finished()
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(5));
    }
    // A new search replaces the Workbench slot with a different batch. The
    // pending bundle task was admitted from the batch its request named and
    // that batch is unchanged, so it must still publish: the slot has no
    // authority over a named batch's admission (unified search keeps one
    // frozen batch per source).
    let mut next = mpc_landsat_request();
    next.request_id = "newer-official-source-request".into();
    next.batch_id = "newer-official-source-batch".into();
    let changed = service.catalog_search_and_freeze_legacy(&next);
    port.release.store(true, Ordering::Release);
    let outcome = handle.join().unwrap();
    changed.expect("independent search remains responsive during official lookup");
    assert!(outcome.is_ok(), "named batch is unchanged: {outcome:?}");
    let snapshot = service.snapshot(true).unwrap();
    assert_eq!(snapshot.restore.tasks.len(), 1);
    assert_eq!(
        snapshot.restore.workbench_batch.unwrap().batch_id,
        next.batch_id
    );
    service.shutdown().unwrap();
}
