use super::*;
use insar_application_service::Sentinel2CreateBandBatchTaskRequest;

const ACQUIRED: &str = "2026-07-06T03:25:11Z";
const SAFE: [&str; 2] = [
    "S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE",
    "S2C_MSIL2A_20260706T032511_N0513_R018_T48SYC_20260707T065916.SAFE",
];
fn native(safe: &str) -> String {
    let p = safe
        .strip_suffix(".SAFE")
        .unwrap()
        .split('_')
        .collect::<Vec<_>>();
    [p[0], p[1], p[2], p[4], p[5], p[6]].join("_")
}
fn manifest(safe: &str, keys: &[String]) -> MpcSentinel2BandManifest {
    MpcSentinel2BandManifest {
        item_id: native(safe), safe_name: safe.to_owned(),
        assets: keys.iter().map(|key| {
            let resolution = match key.as_str() { "B02" | "B03" | "B04" | "B08" => 10, "B01" | "B09" => 60, _ => 20 };
            let file_name = format!("T48SYC_20260706T032511_{key}_{resolution}m.tif");
            MpcSentinel2BandAsset {
                asset_key: key.clone(), resolution_m: resolution, file_name: file_name.clone(),
                stable_href: format!("https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/{safe}/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R{resolution}m/{file_name}"),
            }
        }).collect(),
    }
}

#[derive(Default)]
struct Evidence {
    reject: Vec<String>,
    fault: Option<&'static str>,
    sign_rate_limited: bool,
    block: AtomicBool,
    entered: AtomicU64,
    active: AtomicU64,
    maximum: AtomicU64,
}
impl LiveSentinel2BandEvidence for Evidence {
    fn resolve_band_manifest(
        &self,
        safe: &str,
        acquired: Option<&str>,
        keys: &[String],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<MpcSentinel2BandManifest> {
        assert_eq!(acquired, Some(ACQUIRED));
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        self.entered.fetch_add(1, Ordering::SeqCst);
        while self.block.load(Ordering::SeqCst) && !cancelled() {
            thread::sleep(Duration::from_millis(2));
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        if cancelled() {
            return Err(ContractError::new("STAC_CANCELLED", "cancelled", Map::new()).unwrap());
        }
        let mut result = manifest(safe, keys);
        if safe == SAFE[1] {
            match self.fault {
                Some("identity") => result.item_id = native(SAFE[0]),
                Some("resolution") => result.assets[0].resolution_m = 60,
                Some("signed") => result.assets[0].stable_href.push_str("?sig=fixture"),
                Some("baseline") => {
                    result.assets[0].stable_href =
                        result.assets[0].stable_href.replace("N0513", "N0999")
                }
                Some("missing") => {
                    result.assets.pop();
                }
                _ => {}
            }
        }
        Ok(result)
    }
    fn probe_band_asset(
        &self,
        product: &str,
        key: &str,
        href: &str,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<PlanetaryComputerReviewedAssetEvidence> {
        // Fail on the second band: the first file must not leak into the task.
        if key == "B11" && self.reject.iter().any(|safe| native(safe) == product) {
            if self.sign_rate_limited {
                let context = [
                    ("reason", "http_status_transient"),
                    ("http_status", "429"),
                    ("retry_after_seconds", "3"),
                    ("attempts", "4"),
                ]
                .map(|(key, value)| (key.to_owned(), Value::String(value.to_owned())));
                return Err(ContractError::new(
                    "MPC_ASSET_SIGN_RATE_LIMITED",
                    "fixture rate limit",
                    Map::from_iter(context),
                )
                .unwrap());
            }
            return Err(ContractError::new("STAC_TIMEOUT", "fixture timeout", Map::new()).unwrap());
        }
        Ok(PlanetaryComputerReviewedAssetEvidence {
            file_name: href
                .split('?')
                .next()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_owned(),
            expected_size_bytes: if self.fault == Some("size") && product == native(SAFE[1]) {
                0
            } else {
                1024
            },
        })
    }
}

fn response(mpc: bool) -> CatalogSearchResponse {
    let products = SAFE
        .iter()
        .enumerate()
        .map(|(i, safe)| {
            let mut product =
                exact_archive_product(&format!("00000000-0000-4000-8000-{:012}", i + 1));
            product.display_name = (*safe).to_owned();
            product.acquisition_datetime = Some(ACQUIRED.to_owned());
            product.footprint_geojson = Some(aoi());
            product.properties.insert(
                "checksums".to_owned(),
                json!([{"algorithm":"MD5","value":"00000000000000000000000000000000"}]),
            );
            if mpc {
                product.provider_id = "microsoft.planetary-computer".to_owned();
                product.product_id = native(safe);
                product.assets.clear();
                product.properties = json!({"product_type":"S2MSI2A","cloud_cover":4.2})
                    .as_object()
                    .unwrap()
                    .clone();
            }
            product
        })
        .collect();
    let mut response = complete_response(products);
    response.collection.production_enabled = true;
    response.stats.exact_matched_count = 2;
    response.stats.requested_limit = Some(2);
    response.stats.matched_count = Some(2);
    response.stats.remote_exhausted = true;
    response.stats.working_set_satisfied = true;
    if mpc {
        response.collection = serde_json::from_value(json!({
        "logical_dataset":"optical.sentinel-2.l2a","provider_id":"microsoft.planetary-computer",
        "native_collection_id":"sentinel-2-l2a","collection_profile_id":"planetary-computer.sentinel-2-l2a.scene.v1",
        "identity_rule_id":"planetary-computer.sentinel-2-l2a.identity.v1","filename_rule_id":"planetary-computer.sentinel-2-l2a.product-uri.v1",
        "delivery_model":"archive_first","production_enabled":true})).unwrap();
        response.stats.matched_count = None;
    }
    response
        .validate()
        .expect("synthetic catalogue follows the production response contract");
    response
}
fn service(db: &TempStore, mpc: bool) -> ApplicationService {
    ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: response(mpc),
        }),
        Arc::new(RecordingWorker::default()),
    )
}
fn setup(db: &TempStore, mpc: bool) -> (ApplicationService, Sentinel2CreateBandBatchTaskRequest) {
    let service = service(db, mpc);
    bind_aoi(&service, "band-batch");
    let mut search = super::request("band-batch-search", "band-batch-source");
    if mpc {
        search.search.provider_id = "microsoft.planetary-computer".into();
        search.search.native_collection_id = "sentinel-2-l2a".into();
    }
    let frozen = service
        .catalog_search_and_freeze_legacy(&search)
        .unwrap()
        .restore
        .workbench_batch
        .unwrap();
    let summary = service.core_restore(true).unwrap().core.workbench.unwrap();
    let request = Sentinel2CreateBandBatchTaskRequest {
        request_id: "band-batch-request".into(),
        source_batch_id: frozen.batch_id,
        expected_workbench_revision: summary.revision,
        item_ids: frozen
            .candidates
            .iter()
            .map(|i| i.item_id.clone())
            .collect(),
        asset_keys: vec!["B02".into(), "B11".into()],
        task_id: "band-batch-task".into(),
        output_root: db.0.with_extension("output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-30T00:00:00Z".into(),
    };
    (service, request)
}

#[test]
fn mpc_and_cdse_batches_keep_two_scenes_four_files_one_task_and_restart_source() {
    for mpc in [true, false] {
        let db = TempStore::new("band-batch-restart");
        let (app, mut request) = setup(&db, mpc);
        let current = app.core_restore(true).unwrap().core.workbench.unwrap();
        let selected = app
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
        let visible = app
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
        let before = app.snapshot(true).unwrap().restore.workbench_batch;
        let evidence = Evidence::default();
        let created = app
            .create_sentinel2_band_batch_task_with_evidence_for_test(&request, &evidence)
            .unwrap();
        assert_eq!(created.admission.admitted_count, 2);
        let summary = &created.snapshot.as_ref().unwrap().core.tasks[0];
        assert_eq!(summary.selected_count, 4);
        assert_eq!(summary.delivery_requested_count, Some(2));
        let snapshot = app.snapshot(true).unwrap();
        assert_eq!(snapshot.restore.workbench_batch, before);
        assert_eq!(snapshot.restore.tasks.len(), 1);
        let task = &snapshot.restore.tasks[0];
        assert_eq!(task.candidates.len(), 4);
        assert_eq!(task.aoi_geojson, before.as_ref().unwrap().aoi_geojson);
        assert_eq!(task.output_root, request.output_root);
        let directories = task
            .candidates
            .iter()
            .map(|i| i.metadata["delivery_product_directory"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            directories.len(),
            2,
            "two processing versions with identical basenames need distinct directories"
        );
        assert_eq!(
            task.candidates[0].display_name,
            task.candidates[2].display_name
        );
        for item in &task.candidates {
            let access = item.asset_access.as_ref().unwrap();
            assert!(!access.descriptor.requires_credentials);
            assert!(!access
                .descriptor
                .stable_href
                .as_deref()
                .unwrap()
                .contains('?'));
            assert_eq!(access.expected_size_bytes, Some(1024));
            assert_eq!(
                item.execution_policy.as_ref().unwrap().policy_id,
                "planetary-computer.sentinel-2-l2a.band-cog.asset.v1"
            );
        }
        assert!(evidence.maximum.load(Ordering::SeqCst) <= 2);
        app.shutdown().unwrap();
        let reopened = service(&db, mpc);
        let restored = reopened.snapshot(true).unwrap();
        assert_eq!(restored.restore.workbench_batch, before);
        assert_eq!(restored.restore.tasks.len(), 1);
        assert_eq!(
            reopened.task_delivery_admission(&request.task_id).unwrap(),
            Some(created.admission)
        );
        let page = reopened
            .task_source_batch_page(&insar_core::StoreTaskSourceBatchPageRequest {
                task_id: request.task_id,
                page_index: 0,
                page_size: 24,
            })
            .unwrap();
        assert_eq!(page.total_count, 2);
        assert_eq!(page.source_batch_id, request.source_batch_id);
        assert!(page.items.iter().all(|i| i.belongs_to_task));
        reopened.shutdown().unwrap();
    }
}

#[test]
fn store_owned_empty_selection_and_stale_revision_fail_before_band_evidence() {
    let db = TempStore::new("band-batch-store-selection-guards");
    let (app, mut request) = setup(&db, true);
    let evidence = Evidence::default();
    request.item_ids.clear();
    assert!(
        app.create_sentinel2_band_batch_task_with_evidence_for_test(&request, &evidence)
            .is_err(),
        "empty item_ids must not mean select-all when Store has no checked rows"
    );
    assert_eq!(evidence.entered.load(Ordering::SeqCst), 0);

    let current = app.core_restore(true).unwrap().core.workbench.unwrap();
    let selected = app
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
        app.create_sentinel2_band_batch_task_with_evidence_for_test(&request, &evidence)
            .is_err(),
        "Store-owned selection requires the exact post-selection revision"
    );
    assert_eq!(
        evidence.entered.load(Ordering::SeqCst),
        0,
        "selection guards must fail before live STAC evidence"
    );
    app.shutdown().unwrap();
}

#[test]
fn sign_failure_class_and_detail_survive_admission_and_restart() {
    let db = TempStore::new("band-batch-sign-detail");
    let (app, request) = setup(&db, true);
    let evidence = Evidence {
        reject: vec![SAFE[1].to_owned()],
        sign_rate_limited: true,
        ..Default::default()
    };
    let result = app
        .create_sentinel2_band_batch_task_with_evidence_for_test(&request, &evidence)
        .unwrap();
    assert_eq!(result.admission.rejected_count, 1);
    let rejection = &result.admission.rejections[0];
    assert_eq!(rejection.reason_code, "MPC_ASSET_SIGN_RATE_LIMITED");
    assert_eq!(
        rejection.reason_detail.as_deref(),
        Some("reason=http_status_transient;http_status=429;retry_after_seconds=3;attempts=4")
    );
    app.shutdown().unwrap();
    let reopened = service(&db, true);
    assert_eq!(
        reopened.task_delivery_admission(&request.task_id).unwrap(),
        Some(result.admission)
    );
    reopened.shutdown().unwrap();
}

#[test]
fn partial_and_all_rejections_never_publish_a_scenes_incomplete_band_set() {
    for count in [1, 2] {
        let db = TempStore::new("band-batch-partial");
        let (app, request) = setup(&db, true);
        let evidence = Evidence {
            reject: SAFE[2 - count..].iter().map(|s| (*s).to_owned()).collect(),
            ..Default::default()
        };
        let result = app
            .create_sentinel2_band_batch_task_with_evidence_for_test(&request, &evidence)
            .unwrap();
        assert_eq!(result.admission.rejected_count, count as u64);
        assert!(result
            .admission
            .rejections
            .iter()
            .all(|r| r.reason_code == "STAC_TIMEOUT"));
        if count == 2 {
            assert!(result.snapshot.is_none());
            assert!(app.snapshot(true).unwrap().restore.tasks.is_empty());
        } else {
            assert_eq!(
                result.snapshot.as_ref().unwrap().core.tasks[0].selected_count,
                2
            );
        }
        app.shutdown().unwrap();
        if count == 1 {
            let reopened = service(&db, true);
            assert_eq!(
                reopened.task_delivery_admission(&request.task_id).unwrap(),
                Some(result.admission)
            );
            let page = reopened
                .task_source_batch_page(&insar_core::StoreTaskSourceBatchPageRequest {
                    task_id: request.task_id,
                    page_index: 0,
                    page_size: 24,
                })
                .unwrap();
            assert_eq!(page.items.iter().filter(|i| i.belongs_to_task).count(), 1);
            reopened.shutdown().unwrap();
        }
    }
}

#[test]
fn invalid_selection_is_rejected_before_evidence_and_asset_conflicts_reject_the_whole_scene() {
    let db = TempStore::new("band-batch-validation");
    let (app, request) = setup(&db, true);
    let evidence = Evidence::default();
    for case in 0..5 {
        let mut invalid = request.clone();
        match case {
            0 => invalid.asset_keys = vec!["B10".into()],
            1 => invalid.asset_keys.push("B02".into()),
            2 => invalid.item_ids.push(invalid.item_ids[0].clone()),
            3 => invalid.expected_workbench_revision += 1,
            _ => invalid.item_ids = (0..251).map(|n| format!("item-{n}")).collect(),
        }
        assert!(app
            .create_sentinel2_band_batch_task_with_evidence_for_test(&invalid, &evidence)
            .is_err());
    }
    assert_eq!(evidence.entered.load(Ordering::SeqCst), 0);
    app.shutdown().unwrap();
    for fault in [
        "identity",
        "resolution",
        "signed",
        "baseline",
        "missing",
        "size",
    ] {
        let db = TempStore::new("band-batch-conflict");
        let (app, request) = setup(&db, true);
        let result = app
            .create_sentinel2_band_batch_task_with_evidence_for_test(
                &request,
                &Evidence {
                    fault: Some(fault),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(result.admission.admitted_count, 1, "{fault}");
        assert_eq!(
            result.admission.rejections[0].source_item_id,
            request.item_ids[1]
        );
        assert_eq!(result.snapshot.unwrap().core.tasks[0].selected_count, 2);
        app.shutdown().unwrap();
    }
}

#[test]
fn cancellation_duplicate_and_new_search_are_bounded_by_the_named_batch() {
    for cancel in [true, false] {
        let db = TempStore::new("band-batch-cancel");
        let (app, request) = setup(&db, true);
        let app = Arc::new(app);
        let evidence = Arc::new(Evidence {
            block: AtomicBool::new(true),
            ..Default::default()
        });
        let running = app.clone();
        let copy = request.clone();
        let port = evidence.clone();
        let worker = thread::spawn(move || {
            running.create_sentinel2_band_batch_task_with_evidence_for_test(&copy, port.as_ref())
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while evidence.entered.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        let entered = evidence.entered.load(Ordering::SeqCst);
        let progress = app
            .optical_preparation_progress(&request.request_id)
            .unwrap();
        let duplicate = app
            .create_sentinel2_band_batch_task_with_evidence_for_test(&request, evidence.as_ref());
        if cancel {
            let _ = app.cancel_request(&request.request_id);
        } else {
            // A replacement search moves the Workbench slot to a different
            // batch. The pending task was admitted from the batch its request
            // named and that batch is unchanged, so it must still publish
            // (the slot has no authority over a named batch's admission).
            let mut next = super::request("replacement-search", "replacement-batch");
            next.search.provider_id = "microsoft.planetary-computer".into();
            next.search.native_collection_id = "sentinel-2-l2a".into();
            app.catalog_search_and_freeze_legacy(&next).unwrap();
        }
        evidence.block.store(false, Ordering::SeqCst);
        let result = worker.join().unwrap();
        assert_eq!(entered, 2);
        let progress = progress.expect("S2 bands observe the same preparation registry");
        assert_eq!((progress.requested_count, progress.prepared_count), (2, 0));
        assert!(app
            .optical_preparation_progress(&request.request_id)
            .unwrap()
            .is_none());
        assert!(duplicate.is_err());
        if cancel {
            assert!(result.is_err());
            assert!(app.snapshot(true).unwrap().restore.tasks.is_empty());
        } else {
            let committed = result.expect("named batch is unchanged");
            assert!(committed.snapshot.is_some());
            assert_eq!(app.snapshot(true).unwrap().restore.tasks.len(), 1);
        }
        app.shutdown().unwrap();
    }
}
