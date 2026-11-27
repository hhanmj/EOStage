use super::*;
use insar_provider::cdse::loopback_client::LoopbackODataClient;

// Exercises the production application/Store through the real OData HTTP and
// geometry code, with only the remote endpoint replaced by loopback.
struct HttpCatalogue(Mutex<LoopbackODataClient>);
impl LiveCatalogSearch for HttpCatalogue {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.0.lock().unwrap().search_all(query, cancelled)
    }
}

struct OneResponse {
    endpoint: String,
    request: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl OneResponse {
    fn new(body: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/odata/v1/", listener.local_addr().unwrap());
        let request = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, flag) = (request.clone(), stop.clone());
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !flag.load(Ordering::Acquire) && Instant::now() < deadline {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let wire = read_request(&mut socket);
                assert!(wire.len() < 8192);
                *captured.lock().unwrap() = Some(wire);
                let _ = write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
                break;
            }
        });
        Self {
            endpoint,
            request,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for OneResponse {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn dense_aoi() -> Map<String, Value> {
    let mut ring = (0..1024)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::TAU / 1024.0;
            json!([110.0 + angle.cos(), 30.0 + angle.sin()])
        })
        .collect::<Vec<_>>();
    ring.push(ring[0].clone());
    json!({"type":"Polygon","coordinates":[ring]})
        .as_object()
        .unwrap()
        .clone()
}

#[test]
fn complex_cdse_http_search_freezes_exact_aoi_and_preserves_archive_task_after_restart() {
    let geometry = dense_aoi();
    let mut operation = request("complex-cdse-http", "complex-cdse-batch");
    operation.search.aoi_geojson = Some(geometry.clone());
    operation.aoi_geojson = Some(geometry.clone());
    assert_eq!(
        build_odata_request_for_use(&operation.search, RouteUse::Production)
            .unwrap_err()
            .code,
        "CDSE_ODATA_AOI_TOO_COMPLEX"
    );
    let record = |index: u64, x: f64, y: f64| {
        json!({
            "Id":format!("00000000-0000-4000-8000-{index:012}"),"Name":format!("S2_FIXTURE_{index}.SAFE"),
            "Collection":{"Name":"SENTINEL-2"},"ContentLength":2048,
            "ContentDate":{"Start":"2026-01-01T10:00:00Z"},"Online":true,
            "Checksum":[{"Algorithm":"MD5","Value":"00000000000000000000000000000000"}],
            "Attributes":[{"Name":"productType","Value":"S2MSI2A"},{"Name":"cloudCover","Value":1.0}],
            "GeoFootprint":{"type":"Polygon","coordinates":[[[x,y],[x+0.05,y],[x+0.05,y+0.05],[x,y+0.05],[x,y]]]}
        })
    };
    let server = OneResponse::new(
        json!({"@odata.count":4,"value":[
            record(1,110.9,30.9),record(2,110.5,30.0),record(3,109.5,30.0),record(3,109.5,30.0)
        ]})
        .to_string(),
    );
    let database = TempStore::new("cdse-spatial-freeze");
    let service = ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(HttpCatalogue(Mutex::new(
            LoopbackODataClient::new(&server.endpoint, Duration::from_secs(5)).unwrap(),
        ))),
        Arc::new(RecordingWorker::default()),
    );
    let region_id = common::ensure_fixture_region(&service, "cdse-spatial-freeze");
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "cdse-spatial-bind".into(),
            region_id,
            expected_revision: None,
            source_kind: "administrative_boundary".into(),
            geometry: geometry.clone(),
            bbox: vec![109.0, 29.0, 111.0, 31.0],
            crs: "EPSG:4326".into(),
            display_name: "Synthetic complex boundary".into(),
            bound_at: "2026-07-29T00:00:00Z".into(),
        })
        .unwrap();
    let frozen = service
        .catalog_search_and_freeze_legacy(&operation)
        .unwrap();
    let batch = frozen.restore.workbench_batch.unwrap();
    assert_eq!(batch.aoi_geojson, Some(geometry.clone()));
    assert_eq!(batch.query["aoi_geojson"], json!(geometry));
    assert_eq!(batch.candidates.len(), 2);
    assert!(batch
        .candidates
        .iter()
        .all(|item| !item.item_id.ends_with("000000000001")));
    let ids = batch
        .candidates
        .iter()
        .map(|item| item.item_id.clone())
        .collect::<Vec<_>>();
    let created = service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: "complex-cdse-task".into(),
            batch_id: batch.batch_id.clone(),
            selected_item_ids: ids.clone(),
            checked_item_ids: ids.clone(),
            page_index: 0,
            page_size: 24,
            output_root: database.0.with_extension("output"),
            layout: DownloadLayout::SarDatasetProcessing,
            concurrency: 1,
            created_at: "2026-07-29T00:00:00Z".into(),
        })
        .unwrap();
    let task = created
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "complex-cdse-task")
        .unwrap();
    assert_eq!(task.aoi_geojson, Some(geometry.clone()));
    assert_eq!(task.initial_selected_item_ids, ids);
    assert!(task.candidates.iter().all(|item| item
        .asset_access
        .as_ref()
        .unwrap()
        .descriptor
        .resolver_id
        == "cdse.archive.resolver.v1"));
    assert_eq!(created.restore.workbench_batch, Some(batch.clone()));
    service.shutdown().unwrap();
    let reopened = ApplicationService::new(ApplicationServiceConfig::business_store(&database.0));
    let restored = reopened.restore(true).unwrap();
    assert_eq!(restored.workbench_batch, Some(batch));
    assert_eq!(restored.tasks.len(), 1);
    assert_eq!(restored.tasks[0].aoi_geojson, Some(geometry));
    assert_eq!(restored.tasks[0].initial_selected_item_ids, ids);
    reopened.shutdown().unwrap();
    let wire = server.request.lock().unwrap();
    let wire = wire
        .as_ref()
        .expect("real compact request reached loopback");
    assert!(wire.len() < 2048);
    assert!(!wire.to_ascii_lowercase().contains("authorization:"));
}

#[test]
fn compact_cdse_statistics_do_not_waive_simple_query_or_count_integrity() {
    for case in 0..6 {
        let geometry = if case == 0 { aoi() } else { dense_aoi() };
        let database = TempStore::new("cdse-spatial-negative");
        let mut response = production_response();
        response.stats.requested_limit = Some(2);
        response.stats.matched_count = None;
        response.stats.request_count = 1;
        response.stats.transport_part_count = 1;
        response.stats.fetched_record_count = 1;
        response.stats.mapped_record_count = 1;
        response.stats.unique_provider_product_count = 1;
        response.stats.exact_duplicate_record_count = 0;
        response.stats.exact_matched_count = 1;
        response.stats.returned_count = 1;
        response.stats.remote_exhausted = true;
        response.stats.working_set_satisfied = true;
        response.stats.truncated = false;
        match case {
            0 => {} // An ordinary CDSE request must still supply its total.
            1 => response.stats.matched_count = Some(1), // Rectangle total cannot become AOI total.
            2 => response.stats.mapped_record_count = 2,
            3 => {
                response.stats.transport_part_count = 9;
                response.stats.request_count = 9;
            }
            4 => response.stats.request_count = 129,
            _ => response.stats.remote_exhausted = false,
        }
        let service = ApplicationService::with_catalogue_production_business_runtime(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(StaticCatalog { response }),
            Arc::new(RecordingWorker::default()),
        );
        let region_id = common::ensure_fixture_region(&service, "cdse-negative");
        service
            .bind_region_aoi(&ApplicationBindRegionAoiRequest {
                request_id: "cdse-negative-bind".into(),
                region_id,
                expected_revision: None,
                source_kind: "manual_polygon".into(),
                geometry: geometry.clone(),
                bbox: if case == 0 {
                    vec![110.0, 30.0, 111.0, 31.0]
                } else {
                    vec![109.0, 29.0, 111.0, 31.0]
                },
                crs: "EPSG:4326".into(),
                display_name: "Synthetic negative scope".into(),
                bound_at: "2026-07-29T00:00:00Z".into(),
            })
            .unwrap();
        let mut operation = request("cdse-negative", "cdse-negative-batch");
        operation.aoi_geojson = Some(geometry.clone());
        operation.search.aoi_geojson = Some(geometry);
        assert!(
            service
                .catalog_search_and_freeze_legacy(&operation)
                .is_err(),
            "case {case}"
        );
        assert_empty_store(&service);
        service.shutdown().unwrap();
    }
}
