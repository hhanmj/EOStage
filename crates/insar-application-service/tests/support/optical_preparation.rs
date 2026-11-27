use super::*;
use std::sync::atomic::AtomicUsize;

struct GatedSafeCatalog {
    response: CatalogSearchResponse,
    entered: AtomicUsize,
    release: AtomicBool,
}
impl LiveCatalogSearch for GatedSafeCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let ordinal = self.entered.fetch_add(1, Ordering::SeqCst);
        if ordinal == 0 {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !self.release.load(Ordering::Acquire) && !cancelled() && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(2));
            }
        }
        if cancelled() {
            return Err(ContractError::new(
                "CATALOGUE_REQUEST_CANCELLED",
                "fixture cancelled",
                Map::new(),
            )
            .unwrap());
        }
        Ok(self.response.clone())
    }
}

fn hundred_sources(database: &TempStore) -> insar_core::FrozenSearchBatch {
    let (mut batch, first) = seed_microsoft_sentinel2_archive_source(database);
    batch.batch_id = "hundred-safe-source".to_owned();
    batch.search_stats = None;
    batch.query.insert("result_limit".to_owned(), json!(100));
    batch.candidates = (0..100)
        .map(|index| {
            let mut parts = first
                .display_name
                .strip_suffix(".SAFE")
                .unwrap()
                .split('_')
                .map(str::to_owned)
                .collect::<Vec<_>>();
            parts[6] = format!("20260918T14{:02}{:02}", index / 60, index % 60);
            let native = [
                parts[0].as_str(),
                &parts[1],
                &parts[2],
                &parts[4],
                &parts[5],
                &parts[6],
            ]
            .join("_");
            let mut item = first.clone();
            item.item_id =
                provider_product_key("microsoft.planetary-computer", "sentinel-2-l2a", &native)
                    .unwrap();
            item.display_name = format!("{}.SAFE", parts.join("_"));
            item.metadata
                .insert("provider_product_id".to_owned(), json!(native));
            item.metadata
                .insert("provider_product_key".to_owned(), json!(item.item_id));
            item.metadata
                .insert("stac_item_id".to_owned(), json!(native));
            item.validate().unwrap();
            item
        })
        .collect();
    TaskStore::open(&database.0)
        .unwrap()
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch.clone(),
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 100,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    batch
}

#[test]
fn hundred_safe_preparation_counts_fast_scenes_while_first_waits_and_clears_after_result() {
    for cancel in [false, true] {
        let database = TempStore::new(if cancel {
            "safe-progress-cancel"
        } else {
            "safe-progress-complete"
        });
        let source = hundred_sources(&database);
        let products = source
            .candidates
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let mut product =
                    exact_archive_product(&format!("00000000-0000-4000-8000-{:012}", index + 1));
                product.display_name = item.display_name.clone();
                product.acquisition_datetime = item.acquisition_datetime.clone();
                product.footprint_geojson = item.footprint_geojson.clone();
                product
                    .properties
                    .insert("product_type".to_owned(), json!("S2MSI2A"));
                product.properties.insert("online".to_owned(), json!(true));
                product.properties.insert(
                    "checksums".to_owned(),
                    json!([{"algorithm":"MD5","value":"00000000000000000000000000000000"}]),
                );
                product
            })
            .collect();
        let mut response = complete_response(products);
        response.collection.production_enabled = true;
        response.stats.requested_limit = Some(100);
        response.stats.exact_matched_count = 100;
        response.stats.remote_exhausted = true;
        response.stats.working_set_satisfied = true;
        let catalog = Arc::new(GatedSafeCatalog {
            response,
            entered: AtomicUsize::new(0),
            release: AtomicBool::new(false),
        });
        let service = Arc::new(
            ApplicationService::with_catalogue_production_business_runtime(
                ApplicationServiceConfig::business_store(&database.0),
                Arc::new(FixtureAsf),
                catalog.clone(),
                Arc::new(RecordingWorker::default()),
            ),
        );
        bind_aoi(&service, "safe-progress");
        let request = Sentinel2CreateArchiveBatchTaskRequest {
            request_id: "hundred-safe-preparation".to_owned(),
            source_batch_id: source.batch_id.clone(),
            expected_workbench_revision: 0,
            item_ids: source
                .candidates
                .iter()
                .map(|item| item.item_id.clone())
                .collect(),
            task_id: "hundred-safe-task".to_owned(),
            output_root: database.0.with_extension("output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-30T00:00:00Z".to_owned(),
        };
        let worker = {
            let service = service.clone();
            let request = request.clone();
            thread::spawn(move || service.create_sentinel2_archive_batch_task(&request))
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let observed = loop {
            let value = service
                .optical_preparation_progress(&request.request_id)
                .unwrap();
            if value.as_ref().is_some_and(|p| p.prepared_count == 3) || Instant::now() >= deadline {
                break value;
            }
            thread::sleep(Duration::from_millis(2));
        };
        let before = service.snapshot(true).unwrap();
        if cancel {
            assert!(service.cancel_request(&request.request_id).unwrap());
        }
        catalog.release.store(true, Ordering::Release);
        let result = worker.join().unwrap();
        let observed = observed.expect("observable progress while provider is pending");
        assert_eq!(
            (
                observed.requested_count,
                observed.prepared_count,
                observed.admitted_count,
                observed.rejected_count
            ),
            (100, 3, 3, 0)
        );
        assert_eq!(
            observed.phase,
            insar_application_service::OpticalPreparationPhase::Preparing
        );
        assert!(before.restore.tasks.is_empty());
        assert!(service
            .optical_preparation_progress(&request.request_id)
            .unwrap()
            .is_none());
        if cancel {
            assert!(result.unwrap_err().code().contains("CANCEL"));
            assert!(service.snapshot(true).unwrap().restore.tasks.is_empty());
        } else {
            let result = result.unwrap();
            assert_eq!(result.admission.admitted_count, 100);
            assert_eq!(result.snapshot.unwrap().core.tasks[0].selected_count, 100);
            assert_eq!(catalog.entered.load(Ordering::SeqCst), 100);
        }
        assert_eq!(
            service.snapshot(true).unwrap().restore.workbench_batch,
            Some(source)
        );
        service.shutdown().unwrap();
    }
}
