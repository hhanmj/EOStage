//! Local HTTP + real SSAR HDF5 + production Worker/Store recovery.
//! No saved credentials, official requests, alternate task queue or LSAR alias.
use super::*;
use insar_core::{AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters};
use insar_provider::bhoonidhi::{
    BHOONIDHI_COLLECTION_ID, BHOONIDHI_FILENAME_RULE_ID, BHOONIDHI_IDENTITY_RULE_ID,
    BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_PROFILE_ID, BHOONIDHI_PROVIDER_ID,
    BHOONIDHI_RESOLVER_POLICY_ID,
};

fn sband_work(case: &ReadyCase, source_path: &Path) -> (DownloadWork, NisarSubsetPreparedIntentV2) {
    let seed = prepared_sband_intent(source_path);
    let key = seed.source_product_identity.provider_product_key.clone();
    // The production Bhoonidhi catalogue has no checksum or size. Those must
    // not be invented from our local source just to pass resume validation.
    let intent = NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: seed.source_product_identity,
        source_asset_identity: NisarSubsetSourceAssetIdentityV2 {
            asset_key: "data".to_owned(),
            stable_asset_key: stable_asset_key(&key, "data").unwrap(),
            file_name: format!("{SBAND_FIXTURE_GRANULE}.h5"),
            checksum: None,
        },
        source_capability: seed.source_capability,
        source_footprint: seed.source_footprint_geometry.canonical_geojson,
        requested_geometry: seed.requested_geometry.canonical_geojson,
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations: vec!["RH".to_owned(), "RV".to_owned()],
    })
    .unwrap();
    let file_name = intent.source_asset_identity.file_name.clone();
    let mut metadata = json!({
        "dataset": BHOONIDHI_LOGICAL_DATASET,
        "logical_dataset": BHOONIDHI_LOGICAL_DATASET,
        "native_collection_id": BHOONIDHI_COLLECTION_ID,
        "collection_profile_id": BHOONIDHI_PROFILE_ID,
        "identity_rule_id": BHOONIDHI_IDENTITY_RULE_ID,
        "filename_rule_id": BHOONIDHI_FILENAME_RULE_ID,
        "resolver_policy_id": BHOONIDHI_RESOLVER_POLICY_ID,
        "delivery_model": "archive_first",
        "product_family": BHOONIDHI_COLLECTION_ID,
        "processing_level": "RSLC", "band": "S",
        "provider_product_id": SBAND_FIXTURE_GRANULE,
        "provider_product_key": key,
        "primary_asset_key": "data", "primary_file_name": file_name,
        "source_item_id": key,
        "delivery_kind": NISAR_SUBSET_V2_DELIVERY_KIND,
        "properties": {
            "ID": SBAND_FIXTURE_GRANULE, "Online": "Y", "Coverage": "P",
            "Platform": "NISAR", "Instrument": "SSAR", "Producer": "ISRO",
            "ProductCode": "ST0000KED", "Mode": "W", "Polarization": "CP",
            "SoftwareVersion": "1.4.00", "mode": "W", "polarization": "CP",
            "software_version": "1.4.00"
        }
    })
    .as_object()
    .unwrap()
    .clone();
    embed_nisar_subset_prepared_intent_v2_envelope(&mut metadata, &intent).unwrap();
    let item = SnapshotItem {
        item_id: key.clone(),
        provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
        display_name: SBAND_FIXTURE_GRANULE.to_owned(),
        acquisition_datetime: Some("2026-08-29T13:59:15Z".to_owned()),
        footprint_geojson: Some(intent.source_footprint_geometry.canonical_geojson.clone()),
        metadata,
        payload: Map::new(),
        asset_access: Some(FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
                collection_id: BHOONIDHI_COLLECTION_ID.to_owned(),
                provider_product_id: SBAND_FIXTURE_GRANULE.to_owned(),
                provider_product_key: key.clone(),
                canonical_product_key: None,
                asset_key: "data".to_owned(),
                stable_asset_key: stable_asset_key(&key, "data").unwrap(),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
                scheme: AssetAccessScheme::OauthBearerHttps,
                stable_href: Some("https://bhoonidhi-api.nrsc.gov.in/download".to_owned()),
                requires_credentials: true,
                credential_profile: Some("bhoonidhi-default".to_owned()),
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some(file_name),
            expected_size_bytes: None,
            checksum_algorithm: None,
            checksum_value: None,
        }),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
            collection_profile_id: Some(BHOONIDHI_PROFILE_ID.to_owned()),
        }),
    };
    let mut work = work(case, &intent);
    work.item = item.clone();
    work.task.candidates = vec![item];
    work.task.initial_selected_item_ids = vec![key];
    work.task.aoi_geojson = Some(intent.requested_geometry.canonical_geojson.clone());
    (work, intent)
}

struct SbandCredentials(&'static str);
impl CredentialProvider for SbandCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "bhoonidhi-default");
        CredentialMaterial::bearer(self.0)
    }
}

struct SbandLocalResolver {
    url: String,
    source_size: u64,
}
impl AssetResolver for SbandLocalResolver {
    fn resolver_id(&self) -> &'static str {
        BHOONIDHI_RESOLVER_POLICY_ID
    }
    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::OauthBearerHttps]
    }
    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.scheme == AssetAccessScheme::OauthBearerHttps
            && descriptor.resolver_id == BHOONIDHI_RESOLVER_POLICY_ID
    }
    fn resolve(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        context.check_cancelled()?;
        assert_eq!(reference.provider_id, BHOONIDHI_PROVIDER_ID);
        assert_eq!(reference.provider_product_id, SBAND_FIXTURE_GRANULE);
        assert_eq!(reference.asset_key, "data");
        assert!(self.can_resolve(descriptor));
        let credential = context
            .credential_provider()
            .resolve("bhoonidhi-default")
            .map_err(|_| ResolveError::credentials_missing())?;
        let mut asset = ResolvedAsset::new(&self.url, ResolvedMethod::Get)?
            .with_expected_size(Some(self.source_size))
            .with_suggested_filename(Some(format!("{SBAND_FIXTURE_GRANULE}.h5")))
            .with_asset_role(AssetRole::Primary);
        credential.apply_to(&mut asset)?;
        Ok(asset)
    }
    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::failed())
    }
}

fn sband_worker(
    server_url: &str,
    source_size: u64,
    item: &SnapshotItem,
    credential: &'static str,
) -> ProductionDownloadWorker {
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(SbandLocalResolver {
            url: format!(
                "{}{SBAND_FIXTURE_GRANULE}.h5",
                server_url.trim_end_matches("source.h5")
            ),
            source_size,
        }))
        .unwrap();
    let runtime = ResolverExecutionRuntime::with_dependencies(
        registry,
        Arc::new(SbandCredentials(credential)),
        Arc::new(RouteClock),
    );
    // Exercise the unchanged production CP identity/validator gate, not a
    // private 'approved' shortcut or the Earthdata resolver.
    runtime
        .resolve_for_nisar_remote_subset(item, &|| false)
        .unwrap_or_else(|error| panic!("SSAR frozen authority rejected: {error:?}"));
    let options = TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 64 * 1024,
        ..TransferOptions::default()
    };
    let http = Arc::new(HttpTransfer::new_with_proxy_policy(options, ProxyPolicy::Direct).unwrap());
    ProductionDownloadWorker::new(options)
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, http.clone())
        .with_nisar_subset_v2_http_for_test(http)
}

#[test]
fn sband_production_route_metadata_discovery_503_reaches_normal_failure_reason() {
    let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let case = route_fixture("ssar-metadata-discovery-503", NisarSubsetFrequency::A);
    let source_path = case._root.0.join("ssar-deferred.h5");
    let _source_pin = create_ssar_source_fixture_with_science_layout(&source_path, true, true);
    let source = fs::read(&source_path).unwrap();
    let source_size = source.len() as u64;
    let (work, _) = sband_work(&case, &source_path);
    let database = case._root.0.join("ssar-metadata-discovery-503.sqlite");
    let server = RouteRangeServer::start(
        source,
        ROUTE_ETAG,
        RouteServerMode::Retryable503OnMetadataDiscovery,
    );
    let url = server.url.clone();
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    resume_scheduler_task(&mut store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    // Downloader D1d: a transient metadata-discovery failure is retried after
    // the bounded backoff; here it is recorded and not slept.
    let worker = Arc::new(
        sband_worker(&url, source_size, &work.item, "SBAND_FIXTURE_DIAGNOSTIC")
            .with_nisar_subset_v2_science_retry_backoff_for_test(Arc::new(|_, _| Duration::ZERO)),
    );
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(25));
    scheduler.wake();

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let task = store
            .lock()
            .unwrap()
            .load_task_record(&work.task.task_id)
            .unwrap()
            .unwrap();
        if task.runtime.status == TaskLifecycleStatus::Failed {
            break;
        }
        assert_ne!(task.runtime.status, TaskLifecycleStatus::Completed);
        assert!(
            Instant::now() < deadline,
            "SSAR metadata-discovery failure did not terminate"
        );
        wait_for_route_progress();
    }
    scheduler.shutdown();
    let requests = server.finish();
    assert_eq!(
        requests
            .iter()
            .map(|request| request_range_bounds(request))
            .collect::<Vec<_>>(),
        vec![
            (0, 1),
            (0, 4_015),
            (4_031, 34_915),
            (35_627, 96_625),
            (4_291_223, 4_295_981),
            // Downloader D1d: the retryable 503 of the metadata-discovery
            // Range is retried up to three attempts.
            (4_015, 4_031),
            (4_015, 4_031),
            (4_015, 4_031),
        ],
        "source-size persistence must not change the existing request sequence"
    );
    {
        let locked = store.lock().unwrap();
        let task = locked
            .load_task_record(&work.task.task_id)
            .unwrap()
            .unwrap();
        assert_eq!(task.runtime.status, TaskLifecycleStatus::Failed);
        assert!(
            parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[0].metadata)
                .unwrap()
                .is_none(),
            "metadata discovery failed before a plan envelope could be frozen"
        );
        assert_eq!(
            task.snapshot.candidates[0]
                .asset_access
                .as_ref()
                .and_then(|access| access.expected_size_bytes),
            Some(source_size),
            "resolver-verified SSAR source size must be frozen before metadata discovery fails"
        );
        let restored = locked
            .core_restore(false)
            .unwrap()
            .tasks
            .into_iter()
            .find(|task| task.task_id == work.task.task_id)
            .unwrap();
        assert_eq!(restored.status, TaskLifecycleStatus::Failed);
        assert_eq!(restored.network_bytes_received, Some(100_656));
        assert_eq!(restored.transferred_bytes, 0);
        assert_eq!(restored.source_total_bytes, Some(source_size));
    }
    drop(store);
    let reopened = TaskStore::open(&database).unwrap();
    let reopened_task = reopened
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        reopened_task.snapshot.candidates[0]
            .asset_access
            .as_ref()
            .and_then(|access| access.expected_size_bytes),
        Some(source_size)
    );
    assert_eq!(
        reopened
            .core_restore(false)
            .unwrap()
            .tasks
            .into_iter()
            .find(|task| task.task_id == work.task.task_id)
            .unwrap()
            .source_total_bytes,
        Some(source_size)
    );
    let snapshot = reopened.load_download_snapshot(&work.task.task_id).unwrap();
    assert_eq!(snapshot.results[0].bytes_written, 0);
    let failure = snapshot.results[0]
        .failure
        .as_ref()
        .expect("failed SSAR route must retain a typed failure");
    assert_eq!(failure.code, "NISAR_SUBSET_V2_TRANSPORT_FAILED");
    assert!(failure.retryable);
    assert!(
        failure.reason_zh.contains("stage=metadata_discovery"),
        "reason={} requests={requests:?}",
        failure.reason_zh
    );
    assert!(
        failure
            .reason_zh
            .contains("transport_cause=retryable_http_status"),
        "reason={}",
        failure.reason_zh
    );
    assert!(
        failure.reason_zh.contains("http_status=503"),
        "reason={}",
        failure.reason_zh
    );
    assert!(
        failure.reason_zh.contains("attempts=3"),
        "reason={}",
        failure.reason_zh
    );
    assert_eq!(
        fs::read_dir(&work.task.output_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|value| value == "h5"))
            .count(),
        0
    );
}

/// Downloader enhancement D1d (L5 recheck 2026-10-19): in the S-band run one
/// metadata-discovery Range failed with transport_cause=request_other and
/// request_count=1, failing the task after 195,041,809 bytes. A transient
/// metadata-discovery failure is now retried like a preflight or science Range:
/// at most three attempts, waiting the bounded backoff (5 s, then 20 s; here
/// recorded and not slept) before each retry.
#[test]
fn sband_production_route_retries_a_transient_metadata_discovery_failure_and_completes() {
    let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let label = "ssar-metadata-discovery-reset-once";
    let case = route_fixture(label, NisarSubsetFrequency::A);
    let source_path = case._root.0.join("ssar-deferred.h5");
    let _source_pin = create_ssar_source_fixture_with_science_layout(&source_path, true, true);
    let source = fs::read(&source_path).unwrap();
    let source_size = source.len() as u64;
    let (work, intent) = sband_work(&case, &source_path);
    let database = case._root.0.join(format!("{label}.sqlite"));
    let server = RouteRangeServer::start(
        source,
        ROUTE_ETAG,
        RouteServerMode::MetadataDiscoveryResetOnce,
    );
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    resume_scheduler_task(&mut store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let backoffs = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&backoffs);
    let worker = Arc::new(
        sband_worker(
            &server.url,
            source_size,
            &work.item,
            "SBAND_FIXTURE_DISCOVERY_RETRY",
        )
        .with_nisar_subset_v2_science_retry_backoff_for_test(Arc::new(
            move |retry, http_status| {
                recorded.lock().unwrap().push((retry, http_status));
                Duration::ZERO
            },
        )),
    );
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(25));
    scheduler.wake();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let task = store
            .lock()
            .unwrap()
            .load_task_record(&work.task.task_id)
            .unwrap()
            .unwrap();
        if task.runtime.status == TaskLifecycleStatus::Completed {
            break;
        }
        if task.runtime.status == TaskLifecycleStatus::Failed {
            let snapshot = store
                .lock()
                .unwrap()
                .load_download_snapshot(&work.task.task_id)
                .unwrap();
            panic!(
                "a transient metadata-discovery failure is retried: {:?}",
                snapshot
                    .results
                    .first()
                    .and_then(|result| result.failure.clone())
            );
        }
        assert!(Instant::now() < deadline, "route did not complete");
        wait_for_route_progress();
    }
    scheduler.shutdown();
    let requests = server.finish();
    // The preflight, the four metadata prefix Ranges, then the reset
    // metadata-discovery Range and its one retry (later stages may read the
    // same bytes again).
    assert_eq!(
        requests
            .iter()
            .take(7)
            .map(|request| request_range_bounds(request))
            .collect::<Vec<_>>(),
        vec![
            (0, 1),
            (0, 4_015),
            (4_031, 34_915),
            (35_627, 96_625),
            (4_291_223, 4_295_981),
            (4_015, 4_031),
            (4_015, 4_031),
        ],
        "the reset metadata-discovery Range is retried once"
    );
    assert_eq!(
        backoffs.lock().unwrap().first().copied(),
        Some((1, None)),
        "the retry waited the first backoff"
    );
    drop(store);
    assert_sband_artifact(&work, &intent, &database);
}

/// Design 2026-10-17 §3/§6 at worker level: a transient failure of the first
/// science Range (HTTP 503, or a connection closed before any response) is
/// settled durably and retried once; the task then completes with every Range
/// committed once and the failed request charged, never refunded.
#[test]
fn sband_production_route_retries_a_transient_science_failure_and_completes() {
    let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    for (label, mode) in [
        ("ssar-science-503-once", RouteServerMode::Science503Once),
        ("ssar-science-reset-once", RouteServerMode::ScienceResetOnce),
    ] {
        let case = route_fixture(label, NisarSubsetFrequency::A);
        let source_path = case._root.0.join("ssar-deferred.h5");
        let _source_pin = create_ssar_source_fixture_with_science_layout(&source_path, true, true);
        let source = fs::read(&source_path).unwrap();
        let source_size = source.len() as u64;
        let (work, intent) = sband_work(&case, &source_path);
        let database = case._root.0.join(format!("{label}.sqlite"));
        let server = RouteRangeServer::start(source, ROUTE_ETAG, mode);
        let science_phase = Arc::clone(&server.science_phase);
        let mut store = TaskStore::open(&database).unwrap();
        seed_scheduler_task(&mut store, &work);
        resume_scheduler_task(&mut store, &work);
        let store: SharedTaskStore = Arc::new(Mutex::new(store));
        let worker = Arc::new(
            sband_worker(&server.url, source_size, &work.item, "SBAND_FIXTURE_RETRY")
                .with_nisar_subset_v2_after_planning_hook_for_test(Arc::new(move |_| {
                    science_phase.store(true, Ordering::SeqCst);
                }))
                .with_nisar_subset_v2_science_retry_backoff_for_test(Arc::new(|_, _| {
                    Duration::ZERO
                })),
        );
        let scheduler = DownloadScheduler::start_with_store(
            Arc::clone(&store),
            worker,
            Duration::from_millis(25),
        );
        scheduler.wake();
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let status = store
                .lock()
                .unwrap()
                .load_task_record(&work.task.task_id)
                .unwrap()
                .unwrap()
                .runtime
                .status;
            if status == TaskLifecycleStatus::Completed {
                break;
            }
            assert_ne!(
                status,
                TaskLifecycleStatus::Failed,
                "{label}: a transient failure is retried"
            );
            assert!(Instant::now() < deadline, "{label}: route did not complete");
            wait_for_route_progress();
        }
        scheduler.shutdown();
        let requests = server.finish();
        let checkpoint: serde_json::Value = serde_json::from_str(
            &Hdf5RangeWorkspaceV1::open_existing(route_recovery_root(&work))
                .unwrap()
                .load_science_checkpoint_v2()
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(checkpoint["failed_science_request_count"], 1, "{label}");
        let first = &checkpoint["completed_ranges"][0];
        let first_bounds = (
            first["start"].as_u64().unwrap(),
            first["end_exclusive"].as_u64().unwrap(),
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request_range_bounds(request) == first_bounds)
                .count(),
            2,
            "{label}: the failed Range is requested exactly twice"
        );
        for range in checkpoint["completed_ranges"].as_array().unwrap() {
            assert_eq!(range["request_count"], 1, "{label}");
        }
        drop(store);
        assert_sband_artifact(&work, &intent, &database);
    }
}

fn assert_sband_restart(boundary: RoutePauseBoundary) {
    let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let case = route_fixture("ssar-task-restart", NisarSubsetFrequency::A);
    let source_path = case._root.0.join("ssar-deferred.h5");
    let _source_pin = create_ssar_source_fixture_with_science_layout(&source_path, true, true);
    let source = fs::read(&source_path).unwrap();
    let source_size = source.len() as u64;
    let source_hash = test_file_sha256(&source_path);
    let (work, intent) = sband_work(&case, &source_path);
    let database = case._root.0.join("ssar-task.sqlite");
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let url = server.url.clone();
    let worker_count = AtomicUsize::new(0);
    let requests = assert_route_v2_pause_reopen_resume(
        boundary,
        true,
        work.clone(),
        database.clone(),
        server,
        || {
            let credential = if worker_count.fetch_add(1, Ordering::SeqCst) == 0 {
                "SBAND_FIXTURE_BEFORE_RESTART"
            } else {
                "SBAND_FIXTURE_AFTER_RESTART"
            };
            sband_worker(&url, source_size, &work.item, credential)
        },
    );
    assert_eq!(worker_count.load(Ordering::SeqCst), 2);
    assert!(requests
        .iter()
        .any(|r| r.contains("SBAND_FIXTURE_BEFORE_RESTART")));
    assert!(requests
        .last()
        .unwrap()
        .contains("SBAND_FIXTURE_AFTER_RESTART"));
    assert!(requests
        .iter()
        .all(|r| request_range_bounds(r) != (0, source_size)));
    assert_eq!(test_file_sha256(&source_path), source_hash);
    assert_sband_artifact(&work, &intent, &database);
}

fn assert_sband_artifact(
    work: &DownloadWork,
    intent: &NisarSubsetPreparedIntentV2,
    database: &Path,
) {
    let stored = TaskStore::open(database).unwrap();
    let completed = stored
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(completed.runtime.status, TaskLifecycleStatus::Completed);
    assert_eq!(completed.snapshot.provider_id, BHOONIDHI_PROVIDER_ID);
    assert_eq!(completed.snapshot.candidates.len(), 1);
    assert_eq!(completed.snapshot.candidates[0].item_id, work.item.item_id);
    let plan = parse_nisar_subset_plan_v2_envelope(&completed.snapshot.candidates[0].metadata)
        .unwrap()
        .unwrap()
        .plan()
        .unwrap();
    assert_eq!(plan.selected_polarizations, ["RH", "RV"]);
    let restored_intent =
        parse_nisar_subset_prepared_intent_v2_envelope(&completed.snapshot.candidates[0].metadata)
            .unwrap()
            .unwrap()
            .intent()
            .unwrap();
    assert_eq!(&restored_intent, intent);
    let artifacts: Vec<_> = fs::read_dir(&work.task.output_root)
        .unwrap()
        .map(Result::unwrap)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "h5"))
        .collect();
    assert_eq!(
        artifacts.len(),
        1,
        "one product, one multi-polarization artifact"
    );
    let artifact = hdf5::File::open(&artifacts[0]).unwrap();
    assert!(!artifact.link_exists("/science/LSAR"));
    for pol in ["RH", "RV"] {
        let dataset = artifact
            .dataset(&format!(
                "/science/SSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA/{pol}"
            ))
            .unwrap();
        assert_eq!(dataset.ndim(), 2);
        let values = dataset.read_raw::<TestCFloat32>().unwrap();
        assert!(!values.is_empty());
        assert!(values
            .iter()
            .all(|v| v.r.is_finite() && v.i.is_finite() && v.r != 0.0));
    }
    let sidecar: Value = serde_json::from_slice(
        &fs::read(work.task.output_root.join(format!(
            "{}.artifact-manifest-v2.json",
            artifacts[0].file_name().unwrap().to_string_lossy()
        )))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(sidecar["selected_polarizations"], json!(["RH", "RV"]));
    assert_eq!(sidecar["output_authority"], json!("client_derived"));
    assert_eq!(sidecar["quality_status"], json!("PROVISIONAL"));
    assert_eq!(sidecar["complete"], json!(true));
    assert_eq!(
        sidecar["artifact_sha256"],
        json!(test_file_sha256(&artifacts[0]))
    );
    assert_eq!(
        sidecar["prepared_intent_sha256"],
        json!(intent.canonical_intent_sha256)
    );
    assert_eq!(
        sidecar["requested_geometry_sha256"],
        json!(intent.canonical_geometry_digest.requested_sha256)
    );
    let source = hdf5::File::open(database.parent().unwrap().join("ssar-deferred.h5")).unwrap();
    let windows = sidecar["dataset_windows"].as_array().unwrap();
    assert_eq!(windows.len(), 2);
    let mut compared_samples = 0;
    for window in windows {
        let pol = window["polarization"].as_str().unwrap();
        assert!(["RH", "RV"].contains(&pol));
        let original = source
            .dataset(window["source_dataset_path"].as_str().unwrap())
            .unwrap();
        let sample = |key: &str| usize::try_from(window[key].as_u64().unwrap()).unwrap();
        let expected = original
            .read_slice_2d::<TestCFloat32, _>((
                sample("line_start")..sample("line_end_exclusive"),
                sample("sample_start")..sample("sample_end_exclusive"),
            ))
            .unwrap();
        let actual = artifact
            .dataset(&format!(
                "/science/SSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA/{pol}"
            ))
            .unwrap()
            .read_2d::<TestCFloat32>()
            .unwrap();
        assert_eq!(actual.shape(), expected.shape());
        for (actual, expected) in actual.iter().zip(expected.iter()) {
            assert_eq!(
                (actual.r.to_bits(), actual.i.to_bits()),
                (expected.r.to_bits(), expected.i.to_bits())
            );
            compared_samples += 1;
        }
    }
    assert!(compared_samples > 0);
    println!("SSAR artifact verified: RH/RV samples={compared_samples}, bitwise_equal=true, one_HDF5=true");
}

#[test]
fn sband_task_pause_after_planning_reopens_same_plan_and_completes_rh_rv() {
    assert_sband_restart(RoutePauseBoundary::AfterPlanning);
}

#[test]
fn sband_task_restart_after_science_reuses_verified_range_and_completes_rh_rv() {
    assert_sband_restart(RoutePauseBoundary::AfterFirstScienceRange);
}

/// Stop the actual Worker after its first durable science checkpoint. The caller
/// owns the temporary root and the loopback server, so even process restart tests
/// cannot consult a saved account or an external data service.
fn pause_sband_for_recovery(
    work: &DownloadWork,
    database: &Path,
    server: &RouteRangeServer,
    source_size: u64,
) -> String {
    let mut store = TaskStore::open(database).unwrap();
    seed_scheduler_task(&mut store, work);
    resume_scheduler_task(&mut store, work);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let (tx, rx) = mpsc::channel();
    let paths = ["RH", "RV"].map(|pol| format!("/science/SSAR/RSLC/swaths/frequencyA/{pol}"));
    let physical = crate::hdf5_range_transfer::inspect_nisar_planning_physical_ranges_v2(
        &database.parent().unwrap().join("ssar-deferred.h5"),
        &paths,
        crate::nisar_science_root::NisarScienceRoot::Ssar,
    )
    .unwrap();
    assert!(!physical.selected_science_ranges.is_empty());
    let diagnostic_science_ranges = physical.selected_science_ranges.clone();
    let requests = Arc::clone(&server.requests);
    let worker = sband_worker(&server.url, source_size, &work.item, "SBAND_FIXTURE_PARENT")
        .with_nisar_subset_v2_after_science_commit_hook_for_test(Arc::new(
            move |index, control| {
                let (start, end) = request_range_bounds(requests.lock().unwrap().last().unwrap());
                if !physical
                    .selected_science_ranges
                    .iter()
                    // The inventory unions adjacent chunks; one committed
                    // Range need not contain that entire merged interval.
                    .any(|&(a, b)| start < b && a < end)
                {
                    return;
                }
                tx.send(index).unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while !control.is_paused() {
                    assert!(
                        Instant::now() < deadline,
                        "SSAR task pause did not reach Worker"
                    );
                    wait_for_route_progress();
                }
            },
        ));
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&store),
        Arc::new(worker),
        Duration::from_millis(25),
    );
    scheduler.wake();
    // This later gate waits through the dependency ranges to an actual image
    // chunk. Use the existing 120s complete-route budget, not the 20s early
    // planning gate. Keep the measured preparation time in the test evidence.
    let boundary_started = Instant::now();
    let saved_index = rx.recv_timeout(Duration::from_secs(120)).unwrap_or_else(|error| {
        let ranges = server.requests.lock().unwrap().iter().map(|request| request_range_bounds(request)).collect::<Vec<_>>();
        let snapshot = store.lock().unwrap().load_download_snapshot(&work.task.task_id).unwrap();
        panic!("SSAR data pause boundary missing: {error:?}; source_science={diagnostic_science_ranges:?}; requests={ranges:?}; outcomes={:?}",
            snapshot.results.iter().map(|r| (&r.outcome, &r.failure)).collect::<Vec<_>>());
    });
    println!(
        "SSAR image-chunk boundary elapsed_ms={}",
        boundary_started.elapsed().as_millis()
    );
    let completed_range = request_range_value(server.requests.lock().unwrap().last().unwrap());
    update_route_item_state(
        &mut store.lock().unwrap(),
        work,
        TaskStateCommandKind::PauseTask,
        "pause",
    );
    scheduler.wake();
    wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Interrupted);
    scheduler.shutdown();
    let store = Arc::try_unwrap(store).ok().unwrap().into_inner().unwrap();
    let record = store.load_task_record(&work.task.task_id).unwrap().unwrap();
    assert!(record.runtime.task_paused);
    assert!(record.runtime.paused_item_ids.contains(&work.item.item_id));
    let checkpoint: Value = serde_json::from_slice(
        &fs::read(route_recovery_root(work).join("science-checkpoint-v2.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        checkpoint["completed_ranges"].as_array().unwrap().len(),
        saved_index + 1
    );
    println!(
        "SSAR paused after image-data chunk: completed_ranges={}, image_range={completed_range}",
        saved_index + 1
    );
    assert!(checkpoint["pending_attempt"].is_null());
    assert_eq!(checkpoint["state"], json!("Interrupted"));
    completed_range
}

fn reopen_sband_task(database: &Path, expected_root: &Path) -> (TaskStore, DownloadWork) {
    let mut store = TaskStore::open(database).unwrap();
    let task = store.load_task_record("nisar-v2-task").unwrap().unwrap();
    assert!(task.runtime.task_paused);
    assert_eq!(task.snapshot.provider_id, BHOONIDHI_PROVIDER_ID);
    assert_eq!(
        fs::canonicalize(&task.snapshot.output_root).unwrap(),
        fs::canonicalize(expected_root.join("output")).unwrap()
    );
    assert_eq!(task.snapshot.candidates.len(), 1);
    let work = DownloadWork {
        item: task.snapshot.candidates[0].clone(),
        task: task.snapshot,
    };
    // Match the current ApplicationService task-level resume orchestration.
    update_route_item_state(
        &mut store,
        &work,
        TaskStateCommandKind::ResumeTask,
        "reopen-task",
    );
    update_route_item_state(
        &mut store,
        &work,
        TaskStateCommandKind::ResumeItems,
        "reopen-items",
    );
    (store, work)
}

#[test]
#[ignore = "internal child process of the isolated SSAR restart test; no external requests"]
fn sband_resume_child_process() {
    assert_eq!(std::env::var("EOSTAGE_SSAR_TEST_CHILD").as_deref(), Ok("1"));
    let root = fs::canonicalize(std::env::var_os("EOSTAGE_SSAR_TEST_ROOT").unwrap()).unwrap();
    assert!(root.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("insar-nisar-subset-writer-"));
    let authority: Value =
        serde_json::from_slice(&fs::read(root.join("child-authority.json")).unwrap()).unwrap();
    assert_ne!(
        authority["parent_pid"].as_u64(),
        Some(u64::from(std::process::id()))
    );
    let url = reqwest::Url::parse(authority["server_url"].as_str().unwrap()).unwrap();
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert_eq!(url.path(), "/source.h5");
    assert!(url.query().is_none() && url.username().is_empty() && url.password().is_none());
    crate::establish_hdf5_plugin_isolation().unwrap();
    let source_path = root.join("ssar-deferred.h5");
    assert_eq!(
        test_file_sha256(&source_path),
        authority["source_sha256"].as_str().unwrap()
    );
    let source_file = hdf5::File::open(&source_path).unwrap();
    let _pin =
        crate::nisar_subset_writer::TestSourcePin::acquire_while_hdf5_open(&source_path).unwrap();
    source_file.close().unwrap();
    let (store, work) = reopen_sband_task(&root.join("ssar-task.sqlite"), &root);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let worker = sband_worker(
        url.as_str(),
        fs::metadata(&source_path).unwrap().len(),
        &work.item,
        "SBAND_FIXTURE_FRESH_PROCESS",
    );
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&store),
        Arc::new(worker),
        Duration::from_millis(25),
    );
    scheduler.wake();
    wait_for_route_completed(&store, &work.task.task_id);
    scheduler.shutdown();
    fs::write(
        root.join("child-result.json"),
        serde_json::to_vec(&json!({
            "pid": std::process::id(), "task_id": work.task.task_id,
            "source_sha256": test_file_sha256(&source_path), "public_requests": 0
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn sband_new_process_resumes_checkpoint_without_refetching_verified_range() {
    use std::process::{Command, Stdio};
    let case = route_fixture("ssar-new-process", NisarSubsetFrequency::A);
    let source_path = case._root.0.join("ssar-deferred.h5");
    let _pin = create_ssar_source_fixture_with_science_layout(&source_path, true, true);
    let source = fs::read(&source_path).unwrap();
    let source_size = source.len() as u64;
    let source_hash = test_file_sha256(&source_path);
    let (work, intent) = sband_work(&case, &source_path);
    let database = case._root.0.join("ssar-task.sqlite");
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let completed_range = pause_sband_for_recovery(&work, &database, &server, source_size);
    let plan_before = TaskStore::open(&database)
        .unwrap()
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap()
        .snapshot
        .candidates[0]
        .metadata
        .clone();
    fs::write(
        case._root.0.join("child-authority.json"),
        serde_json::to_vec(&json!({
            "parent_pid": std::process::id(), "server_url": server.url, "source_sha256": source_hash
        }))
        .unwrap(),
    )
    .unwrap();
    let log_path = case._root.0.join("child-process.log");
    let log = fs::File::create(&log_path).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "nisar_planning_v2::tests::sband_resume::sband_resume_child_process",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("EOSTAGE_SSAR_TEST_CHILD", "1")
        .env("EOSTAGE_SSAR_TEST_ROOT", &case._root.0)
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    let child_pid = child.id();
    assert_ne!(child_pid, std::process::id());
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "SSAR child timed out: {}",
                fs::read_to_string(&log_path).unwrap()
            );
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "SSAR child failed: {}",
        fs::read_to_string(&log_path).unwrap()
    );
    let report: Value =
        serde_json::from_slice(&fs::read(case._root.0.join("child-result.json")).unwrap()).unwrap();
    assert_eq!(report["pid"], json!(child_pid));
    assert_eq!(report["source_sha256"], json!(source_hash));
    let requests = server.finish();
    assert_eq!(
        requests
            .iter()
            .filter(|r| request_range_value(r) == completed_range)
            .count(),
        1
    );
    assert!(requests
        .last()
        .unwrap()
        .contains("SBAND_FIXTURE_FRESH_PROCESS"));
    assert!(requests
        .iter()
        .all(|r| request_range_bounds(r) != (0, source_size)));
    let plan_after = TaskStore::open(&database)
        .unwrap()
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap()
        .snapshot
        .candidates[0]
        .metadata
        .clone();
    assert_eq!(plan_before, plan_after);
    assert_sband_artifact(&work, &intent, &database);
    println!("SSAR new-process resume verified: parent={}, child={}, range_reused=true, public_requests=0",
        std::process::id(), child_pid);
}

#[derive(Clone, Copy, Debug)]
enum RecoveryChange {
    SourceValidator,
    CorruptCheckpoint,
    CancelTask,
}

fn assert_sband_failed_recovery(change: RecoveryChange) {
    let case = route_fixture("ssar-failed-recovery", NisarSubsetFrequency::A);
    let source_path = case._root.0.join("ssar-deferred.h5");
    let _pin = create_ssar_source_fixture_with_science_layout(&source_path, true, true);
    let source = fs::read(&source_path).unwrap();
    let source_size = source.len() as u64;
    let (work, _) = sband_work(&case, &source_path);
    let database = case._root.0.join("ssar-task.sqlite");
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let completed_range = pause_sband_for_recovery(&work, &database, &server, source_size);
    let checkpoint_path = route_recovery_root(&work).join("science-checkpoint-v2.json");
    if matches!(change, RecoveryChange::CorruptCheckpoint) {
        fs::write(&checkpoint_path, b"invalid isolated checkpoint").unwrap();
    }
    let preserved_checkpoint = fs::read(&checkpoint_path).unwrap();
    let sparse = Hdf5RangeWorkspaceV1::new(route_recovery_root(&work)).unwrap();
    let preserved_sparse_hash = test_file_sha256(&sparse.part_path());
    if matches!(change, RecoveryChange::SourceValidator) {
        server.enable_validator_drift();
    }
    let mut store = TaskStore::open(&database).unwrap();
    if matches!(change, RecoveryChange::CancelTask) {
        update_route_item_state(
            &mut store,
            &work,
            TaskStateCommandKind::CancelTask,
            "cancel-paused",
        );
    } else {
        drop(store);
        store = reopen_sband_task(&database, &case._root.0).0;
    }
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let before_requests = server.requests.lock().unwrap().len();
    let worker = sband_worker(
        &server.url,
        source_size,
        &work.item,
        "SBAND_FIXTURE_RECOVERY",
    );
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&store),
        Arc::new(worker),
        Duration::from_millis(25),
    );
    scheduler.wake();
    if matches!(change, RecoveryChange::CancelTask) {
        thread::sleep(Duration::from_millis(100));
    } else {
        wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Failed);
    }
    scheduler.shutdown();
    let stored = store.lock().unwrap();
    let task = stored
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    if matches!(change, RecoveryChange::CancelTask) {
        assert_eq!(task.runtime.status, TaskLifecycleStatus::Cancelled);
        assert_eq!(server.requests.lock().unwrap().len(), before_requests);
    } else {
        assert_eq!(task.runtime.status, TaskLifecycleStatus::Failed);
        let snapshot = stored.load_download_snapshot(&work.task.task_id).unwrap();
        let failure = snapshot
            .results
            .iter()
            .find(|r| r.outcome == DownloadOutcome::Failed)
            .unwrap()
            .failure
            .as_ref()
            .unwrap();
        let expected = match change {
            RecoveryChange::SourceValidator => "NISAR_SUBSET_V2_VALIDATOR_CHANGED",
            RecoveryChange::CorruptCheckpoint => "NISAR_SUBSET_V2_RECOVERY_INVALID",
            RecoveryChange::CancelTask => unreachable!(),
        };
        assert_eq!(failure.code, expected);
    }
    let after_bytes = fs::read(&checkpoint_path).unwrap();
    if matches!(change, RecoveryChange::SourceValidator) {
        // A rejected response must be durably charged; byte-identical checkpoint
        // files would incorrectly erase the failed attempt from the budget.
        let mut before: Value = serde_json::from_slice(&preserved_checkpoint).unwrap();
        let mut after: Value = serde_json::from_slice(&after_bytes).unwrap();
        for key in [
            "failed_science_request_count",
            "issued_science_request_count",
            "reserved_science_request_count",
        ] {
            assert_eq!(
                after[key].as_u64().unwrap(),
                before[key].as_u64().unwrap() + 1
            );
        }
        assert_eq!(
            after["resumed_bytes"],
            before["committed_science_response_body_bytes"]
        );
        assert!(
            after["reserved_science_response_body_bytes"]
                .as_u64()
                .unwrap()
                > before["reserved_science_response_body_bytes"]
                    .as_u64()
                    .unwrap()
        );
        assert!(
            after["anchor_generation"].as_u64().unwrap()
                > before["anchor_generation"].as_u64().unwrap()
        );
        for key in [
            "failed_science_request_count",
            "issued_science_request_count",
            "reserved_science_request_count",
            "resumed_bytes",
            "reserved_science_response_body_bytes",
            "anchor_generation",
            "checkpoint_sha256",
            "previous_checkpoint_sha256",
        ] {
            before.as_object_mut().unwrap().remove(key);
            after.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(
            after, before,
            "source, plan, completed ranges and written bytes must remain unchanged"
        );
    } else {
        assert_eq!(after_bytes, preserved_checkpoint);
    }
    assert_eq!(test_file_sha256(&sparse.part_path()), preserved_sparse_hash);
    assert!(!fs::read_dir(&work.task.output_root)
        .unwrap()
        .map(Result::unwrap)
        .any(|e| e.path().extension().is_some_and(|s| s == "h5")));
    let requests = server.finish();
    assert_eq!(
        requests
            .iter()
            .filter(|r| request_range_value(r) == completed_range)
            .count(),
        1
    );
    assert!(requests
        .iter()
        .all(|r| request_range_bounds(r) != (0, source_size)));
}

#[test]
fn sband_recovery_rejects_source_validator_change_without_full_download() {
    assert_sband_failed_recovery(RecoveryChange::SourceValidator);
}

#[test]
fn sband_recovery_rejects_corrupt_checkpoint_without_erasing_it() {
    assert_sband_failed_recovery(RecoveryChange::CorruptCheckpoint);
}

#[test]
fn sband_cancelled_paused_task_never_restarts_or_discards_checkpoint() {
    assert_sband_failed_recovery(RecoveryChange::CancelTask);
}
