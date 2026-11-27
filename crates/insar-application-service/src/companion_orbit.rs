use super::*;

/// Orbit scheduled-fetch S4: the parent SLC scene id of one frozen burst
/// asset, from its official asset URL. The URL was already frozen and
/// validated when the burst task was created (`validate_frozen_burst_asset`:
/// https on the ASF burst host, no query, no fragment); the parent scene id
/// is its first path segment. Returns `None` when the segment is not a
/// Sentinel-1 SLC scene id.
fn burst_parent_scene_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("sentinel1-burst.asf.alaska.edu")
        || parsed.port_or_known_default() != Some(443)
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    let scene = parsed.path_segments()?.next()?.to_owned();
    let scene = insar_core::parse_sentinel1_identifier(&scene)
        .ok()
        .filter(|scene| scene.product_type == "SLC")?;
    Some(scene.scene_id)
}

/// Orbit scheduled-fetch S4 (review finding 6): the parent scene id from the
/// frozen identity — the parent product's scene id carried in the asset
/// metadata (`scene_id`, cloned from the parent when the burst task froze),
/// with the parent's `ASF::SLC-BURST::{scene}` item id in
/// `payload.source_item_id` as the fallback. Identity is authoritative; the
/// URL is only a cross-check.
fn burst_parent_scene_from_identity(item: &SnapshotItem) -> Option<String> {
    let candidates = [
        item.metadata.get("scene_id").and_then(Value::as_str),
        item.payload
            .get("source_item_id")
            .and_then(Value::as_str)
            .and_then(|source| source.strip_prefix("ASF::SLC-BURST::")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|name| insar_core::parse_sentinel1_identifier(name).ok())
        .filter(|scene| scene.product_type == "SLC")
        .map(|scene| scene.scene_id)
}

/// Orbit scheduled-fetch S4 (review finding 3): what the companion builder
/// did — the unified create_task path builds its TaskRef warning from
/// `skipped_unsupported`, so the builder is the single source of truth for
/// which scenes it covered.
#[derive(Debug)]
pub(crate) struct CompanionOrbitOutcome {
    /// The store snapshot the companion creation produced.
    pub(crate) snapshot: CoreServiceSnapshot,
    /// How many selected items the builder skipped as unsupported (no
    /// derivable parent scene, or a frozen identity that disagrees with its
    /// official asset URL). 0 when every scene is covered.
    pub(crate) skipped_unsupported: usize,
}

impl ApplicationService {
    /// Freeze real scene IDs; the production worker resolves EOF metadata.
    pub fn create_companion_orbit_task(
        &self,
        source_task_id: &str,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.create_companion_orbit_task_with_policy(source_task_id, false)
            .map(|outcome| outcome.snapshot)
    }

    /// Orbit scheduled-fetch S4: the unified `create_task` path allows a
    /// partial companion — unsupported scenes are skipped and counted in
    /// [`CompanionOrbitOutcome::skipped_unsupported`] (the TaskRef warning
    /// is built from it), while the strict direct command keeps failing on
    /// the first unsupported scene.
    pub(crate) fn create_companion_orbit_task_with_policy(
        &self,
        source_task_id: &str,
        skip_unsupported: bool,
    ) -> Result<CompanionOrbitOutcome, ApplicationServiceError> {
        if source_task_id.trim().is_empty() {
            return Err(invalid_request("companion Orbit requires a source task"));
        }
        self.ensure_formal_orbit_capability()?;
        let skipped_unsupported = std::cell::Cell::new(0_usize);
        let snapshot = self.mutate_business_store(|store| {
            let source = store
                .load_task(source_task_id)?
                .ok_or_else(|| invalid_request("companion Orbit source task was not found"))?;
            let task_id = format!("{source_task_id}-orbit");
            if let Some(existing) = store.load_task(&task_id)? {
                if existing.snapshot.operation.as_ref().and_then(|operation| {
                    operation
                        .payload
                        .get("source_task_id")
                        .and_then(Value::as_str)
                }) != Some(source_task_id)
                {
                    return Err(invalid_request("companion Orbit task identity conflicts"));
                }
                return Ok(());
            }
            // Like standalone DEM, this task is scoped by its frozen source,
            // not by whichever Region happens to be open in the UI.
            let region_id = "companion-orbit".to_owned();
            let snapshot = &source.snapshot;
            if !matches!(
                snapshot.provider_id.as_str(),
                "asf.sentinel1" | "asf.catalog"
            ) {
                return Err(invalid_request(
                    "companion Orbit source provider is unsupported",
                ));
            }
            let mut candidates = Vec::new();
            let mut seen_scenes = std::collections::BTreeSet::new();
            // The warning counts scenes, not files: a burst parent with
            // several TIFF/XML assets is one scene, and it is skipped only
            // when none of its assets yields an orbit scene.
            let mut covered_burst_parents = std::collections::BTreeSet::new();
            let mut skipped_burst_parents = std::collections::BTreeSet::new();
            for item_id in &snapshot.initial_selected_item_ids {
                let item = snapshot
                    .candidates
                    .iter()
                    .find(|item| &item.item_id == item_id)
                    .ok_or_else(|| {
                        invalid_request("companion Orbit source selection is incomplete")
                    })?;
                // A burst asset item carries no SLC scene id of its own. Its
                // parent scene comes from the frozen identity (the asset
                // metadata keeps the parent's scene_id; the parent's item id
                // is `ASF::SLC-BURST::{scene}` in payload.source_item_id);
                // the official asset URL's first path segment names the same
                // parent and is a cross-check — on mismatch the asset counts
                // as unsupported (review finding 6: never silently pick one
                // of two disagreeing identities). Bursts of the same parent
                // scene dedupe to one orbit item.
                // A burst asset's parent scene is decided ONLY by the frozen
                // identity cross-checked against the official asset URL —
                // the generic SLC-evidence fallback never rescues a burst
                // asset whose identity is underivable or disagrees with its
                // URL (review finding 6: never silently pick one).
                let burst_parent = (item.payload.get("burst_asset") == Some(&Value::Bool(true)))
                    .then(|| {
                        item.payload
                            .get("source_item_id")
                            .and_then(Value::as_str)
                            .unwrap_or(item.item_id.as_str())
                            .to_owned()
                    });
                let scene = if burst_parent.is_some() {
                    burst_parent_scene_from_identity(item).and_then(|from_identity| {
                        let from_url = item
                            .payload
                            .get("download_url")
                            .and_then(Value::as_str)
                            .and_then(burst_parent_scene_from_url);
                        (from_url == Some(from_identity.clone())).then_some(from_identity)
                    })
                } else {
                    [
                        item.payload.get("scene_id").and_then(Value::as_str),
                        item.metadata.get("scene_id").and_then(Value::as_str),
                        Some(item.display_name.as_str()),
                        Some(item.item_id.as_str()),
                    ]
                    .into_iter()
                    .flatten()
                    .find_map(|name| {
                        let mut normalized = name.trim();
                        if let Some((_, tail)) = normalized.rsplit_once("::ITEM::") {
                            normalized = tail.trim();
                        }
                        for suffix in [
                            "-SLC", "-GRD_HD", "-GRD_HS", "-GRD_MD", "-GRD_MS", "-GRD_FD", "-RAW",
                            "-OCN",
                        ] {
                            if let Some(stem) = normalized.strip_suffix(suffix) {
                                normalized = stem;
                                break;
                            }
                        }
                        insar_core::parse_sentinel1_identifier(normalized).ok()
                    })
                    .filter(|scene| scene.product_type == "SLC")
                    .map(|scene| scene.scene_id)
                };
                if let Some(parent) = &burst_parent {
                    if scene.is_some() {
                        covered_burst_parents.insert(parent.clone());
                    } else if skip_unsupported {
                        skipped_burst_parents.insert(parent.clone());
                        continue;
                    }
                }
                let scene = match scene {
                    // Dedupe: later bursts of an already-covered parent
                    // scene add nothing.
                    Some(scene) if !seen_scenes.insert(scene.clone()) => continue,
                    Some(scene) => scene,
                    // S4 partial coverage: an unsupported scene (or a burst
                    // asset whose identity disagrees with its URL) is
                    // skipped and counted here; the caller builds the
                    // TaskRef warning from the count.
                    None if skip_unsupported => {
                        skipped_unsupported.set(skipped_unsupported.get() + 1);
                        continue;
                    }
                    None => {
                        return Err(invalid_request(
                            "companion Orbit only supports Sentinel-1 SLC scenes",
                        ))
                    }
                };
                candidates.push(SnapshotItem {
                    item_id: scene.clone(),
                    provider_id: "sentinel1.orbit".to_owned(),
                    display_name: scene,
                    acquisition_datetime: item.acquisition_datetime.clone(),
                    footprint_geojson: item.footprint_geojson.clone(),
                    metadata: Map::from_iter([
                        (
                            "source_item_id".to_owned(),
                            Value::String(item.item_id.clone()),
                        ),
                        ("region_id".to_owned(), Value::String(region_id.clone())),
                    ]),
                    payload: Map::new(),
                    asset_access: None,
                    execution_policy: None,
                });
            }
            if candidates.is_empty() {
                return Err(invalid_request("companion Orbit source selection is empty"));
            }
            let ids = candidates
                .iter()
                .map(|item| item.item_id.clone())
                .collect::<Vec<_>>();
            let batch_id = format!("{task_id}-batch");
            let operation = TaskOperationDescriptor {
                operation_id: format!("{task_id}:poeorb"),
                kind: "orbit".to_owned(),
                phase: "download".to_owned(),
                region_id,
                approved_worker_operation: "orbit_download".to_owned(),
                payload: Map::from_iter([
                    (
                        "source_task_id".to_owned(),
                        Value::String(source_task_id.to_owned()),
                    ),
                    (
                        "resource_id".to_owned(),
                        Value::String("orbit.sentinel-1.poeorb.v1".to_owned()),
                    ),
                    (
                        "capability_id".to_owned(),
                        Value::String("capability.sentinel-1.poeorb.download.v1".to_owned()),
                    ),
                ]),
            };
            store.save_search_batch_and_create_task_with_projection(
                &StoreSearchBatchRequest {
                    batch: FrozenSearchBatch {
                        schema_version: 1,
                        batch_id: batch_id.clone(),
                        provider_id: "sentinel1.orbit".to_owned(),
                        created_at: snapshot.created_at.clone(),
                        aoi_name: snapshot.aoi_name.clone(),
                        aoi_geojson: snapshot.aoi_geojson.clone(),
                        query: Map::from_iter([(
                            "source_task_id".to_owned(),
                            Value::String(source_task_id.to_owned()),
                        )]),
                        search_stats: None,
                        candidates,
                    },
                    workbench: SearchWorkbenchState {
                        batch_id: batch_id.clone(),
                        checked_item_ids: ids.clone(),
                        page_index: 0,
                        page_size: 250,
                        revision: 0,
                    },
                    activate_workbench: false,
                },
                &StoreCreateTaskRequest {
                    task_id,
                    batch_id,
                    selected_item_ids: ids.clone(),
                    checked_item_ids: ids,
                    page_index: 0,
                    page_size: 250,
                    output_root: snapshot.output_root.clone(),
                    layout: DownloadLayout::SentinelOrbitAuxPoeorb,
                    // Adjacent SLC scenes can resolve to the same EOF path.
                    // Serialize this batch so the next scene reuses the completed file.
                    concurrency: 1,
                    created_at: snapshot.created_at.clone(),
                },
                Some(&operation),
                None,
            )?;
            skipped_unsupported.set(
                skipped_unsupported.get()
                    + skipped_burst_parents
                        .difference(&covered_burst_parents)
                        .count(),
            );
            Ok(())
        })?;
        Ok(CompanionOrbitOutcome {
            snapshot,
            skipped_unsupported: skipped_unsupported.get(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insar_core::{
        AsfRemoteSearchRequest, AsfRemoteSearchResponse, DownloadItemProgress, DownloadItemResult,
        DownloadOutcome, ProviderResult, RuntimeReadinessState,
    };
    use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
    use insar_service::LiveAsfSearch;

    struct EmptySearch;
    impl LiveAsfSearch for EmptySearch {
        fn search(
            &self,
            _: &AsfRemoteSearchRequest,
            _: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<AsfRemoteSearchResponse> {
            unreachable!()
        }
    }
    struct CompletedWorker;
    /// A worker that keeps every item active until the service tears down,
    /// so waiting items stay queued and their recheck rows survive the S3
    /// scheduler reconciliation (which clears rows of unqueued items).
    struct HoldingWorker;
    impl DownloadWorker for HoldingWorker {
        fn execute(
            &self,
            work: DownloadWork,
            control: WorkerControl,
            _: &mut dyn FnMut(DownloadItemProgress),
        ) -> DownloadItemResult {
            while !control.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            DownloadItemResult {
                task_id: work.task.task_id,
                item_id: work.item.item_id,
                outcome: DownloadOutcome::Cancelled,
                destination: work.task.output_root,
                bytes_written: 0,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-11-01T00:00:00Z".to_owned(),
            }
        }
    }

    impl DownloadWorker for CompletedWorker {
        fn execute(
            &self,
            work: DownloadWork,
            _: WorkerControl,
            _: &mut dyn FnMut(DownloadItemProgress),
        ) -> DownloadItemResult {
            DownloadItemResult {
                task_id: work.task.task_id,
                item_id: work.item.item_id,
                outcome: DownloadOutcome::Completed,
                destination: work.task.output_root,
                bytes_written: 0,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-09-05T00:00:01Z".to_owned(),
            }
        }
    }
    fn service(path: &std::path::Path) -> ApplicationService {
        ApplicationService::with_fixture_business_runtime(
            ApplicationServiceConfig::business_store(path).with_capability_runtime_observations(
                CapabilityRuntimeObservations::default().with_runtime(
                    "orbit-download.asf-sentinel-1.v1",
                    RuntimeReadinessState::Ready,
                ),
            ),
            Arc::new(EmptySearch),
            Arc::new(CompletedWorker),
        )
    }
    fn seed(service: &ApplicationService, root: &std::path::Path, slc: bool) {
        service.mutate_business_store(|store| {
            let candidates = (0..251).map(|index| {
                let scene_id = format!("S1A_IW_{}__1SDV_20240312T000000_20240312T000027_052931_066A01_{index:04X}", if slc { "SLC" } else { "RAW" });
                SnapshotItem {
                    item_id: format!("product-{index}"), provider_id: "asf.sentinel1".to_owned(),
                    display_name: scene_id.clone(), acquisition_datetime: Some("2024-03-12T00:00:00Z".to_owned()),
                    footprint_geojson: None, metadata: Map::new(),
                    payload: Map::from_iter([("scene_id".to_owned(), Value::String(scene_id))]),
                    asset_access: None, execution_policy: None,
                }
            }).collect();
            store.save_search_batch(&StoreSearchBatchRequest {
                batch: FrozenSearchBatch {
                    schema_version: 1, batch_id: "source-batch".to_owned(), provider_id: "asf.sentinel1".to_owned(),
                    created_at: "2026-09-05T00:00:00Z".to_owned(), aoi_name: "Frozen source AOI".to_owned(),
                    aoi_geojson: None, query: Map::new(), search_stats: None, candidates,
                },
                workbench: SearchWorkbenchState { batch_id: "source-batch".to_owned(),
                    checked_item_ids: vec!["product-0".to_owned(), "product-250".to_owned()],
                    page_index: 0, page_size: 250, revision: 0 }, activate_workbench: true,
            })?;
            store.create_task(&StoreCreateTaskRequest {
                task_id: "slc-task".to_owned(), batch_id: "source-batch".to_owned(),
                selected_item_ids: vec!["product-0".to_owned(), "product-250".to_owned()],
                checked_item_ids: vec!["product-0".to_owned(), "product-250".to_owned()],
                page_index: 0, page_size: 250, output_root: root.join("chosen-slc"),
                layout: DownloadLayout::Sentinel1Slc, concurrency: 3,
                created_at: "2026-09-05T00:00:00Z".to_owned(),
            })?;
            Ok(())
        }).unwrap();
    }
    #[test]
    fn companion_orbit_without_region_freezes_cross_page_selection_and_reopens_idempotently() {
        let root = std::env::temp_dir().join(format!(
            "companion-orbit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("tasks.sqlite3");
        let app = service(&path);
        seed(&app, &root, true);
        app.create_companion_orbit_task("slc-task").unwrap();
        let frozen = app
            .with_business_runtime(|runtime| {
                let store = runtime.store.lock().unwrap();
                let source = store.load_task("slc-task")?.unwrap();
                let orbit = store.load_task("slc-task-orbit")?.unwrap();
                assert_eq!(orbit.snapshot.candidates.len(), 2);
                assert_eq!(orbit.runtime.concurrency, 1);
                assert_eq!(orbit.snapshot.output_root, root.join("chosen-slc"));
                assert_eq!(
                    orbit.snapshot.layout,
                    DownloadLayout::SentinelOrbitAuxPoeorb
                );
                assert_eq!(orbit.snapshot.aoi_name, source.snapshot.aoi_name);
                assert_eq!(orbit.snapshot.aoi_geojson, source.snapshot.aoi_geojson);
                assert!(orbit.snapshot.orbit_association.is_none());
                assert!(orbit.snapshot.candidates[1].item_id.ends_with("00FA"));
                assert!(orbit
                    .snapshot
                    .candidates
                    .iter()
                    .all(|item| item.payload.is_empty() && item.asset_access.is_none()));
                assert_eq!(
                    source.snapshot.initial_selected_item_ids,
                    ["product-0", "product-250"]
                );
                Ok(orbit.snapshot)
            })
            .unwrap();
        app.create_companion_orbit_task("slc-task").unwrap();
        app.shutdown().unwrap();
        let reopened = service(&path);
        reopened.create_companion_orbit_task("slc-task").unwrap();
        reopened
            .with_business_runtime(|runtime| {
                assert_eq!(
                    runtime
                        .store
                        .lock()
                        .unwrap()
                        .load_task("slc-task-orbit")?
                        .unwrap()
                        .snapshot,
                    frozen
                );
                Ok(())
            })
            .unwrap();
        reopened.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn companion_orbit_rejects_non_slc_without_creating_a_partial_batch() {
        let root =
            std::env::temp_dir().join(format!("companion-orbit-invalid-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let app = service(&root.join("tasks.sqlite3"));
        seed(&app, &root, false);
        assert!(app.create_companion_orbit_task("slc-task").is_err());
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            assert!(store.load_task("slc-task-orbit")?.is_none());
            assert!(store.load_search_batch("slc-task-orbit-batch")?.is_none());
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    // ---- Orbit scheduled-fetch S4 ------------------------------------------

    fn s4_service(path: &std::path::Path, orbit_ready: bool, hold: bool) -> ApplicationService {
        let observations = if orbit_ready {
            CapabilityRuntimeObservations::default().with_runtime(
                "orbit-download.asf-sentinel-1.v1",
                RuntimeReadinessState::Ready,
            )
        } else {
            CapabilityRuntimeObservations::default()
        };
        ApplicationService::with_fixture_business_runtime(
            ApplicationServiceConfig::business_store(path)
                .with_capability_runtime_observations(observations),
            Arc::new(EmptySearch),
            if hold {
                Arc::new(HoldingWorker)
            } else {
                Arc::new(CompletedWorker)
            },
        )
    }

    /// A unified-create_task-compatible frozen batch: the registry route is
    /// resolvable (`asf.catalog` + `dataset` selector), so the account
    /// preflight admits it the way a real Sentinel-1 batch is admitted.
    fn s4_seed(service: &ApplicationService, slc: bool) {
        service.mutate_business_store(|store| {
            let scene_id = format!(
                "S1A_IW_{}__1SDV_20240312T000000_20240312T000027_052931_066A01_{:04X}",
                if slc { "SLC" } else { "GRDH" },
                0
            );
            let candidates = vec![
                SnapshotItem {
                    item_id: "product-0".to_owned(),
                    provider_id: "asf.catalog".to_owned(),
                    display_name: scene_id.clone(),
                    acquisition_datetime: Some("2024-03-12T00:00:00Z".to_owned()),
                    footprint_geojson: None,
                    metadata: Map::new(),
                    payload: Map::from_iter([(
                        "scene_id".to_owned(),
                        Value::String(scene_id),
                    )]),
                    asset_access: None,
                    execution_policy: None,
                },
                SnapshotItem {
                    item_id: "product-250".to_owned(),
                    provider_id: "asf.catalog".to_owned(),
                    display_name: "S1A_IW_SLC__1SDV_20240312T000100_20240312T000127_052931_066A01_00FA".to_owned(),
                    acquisition_datetime: Some("2024-03-12T00:01:00Z".to_owned()),
                    footprint_geojson: None,
                    metadata: Map::new(),
                    payload: Map::from_iter([(
                        "scene_id".to_owned(),
                        Value::String(
                            "S1A_IW_SLC__1SDV_20240312T000100_20240312T000127_052931_066A01_00FA"
                                .to_owned(),
                        ),
                    )]),
                    asset_access: None,
                    execution_policy: None,
                },
            ];
            store.save_search_batch(&StoreSearchBatchRequest {
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "s4-source-batch".to_owned(),
                    provider_id: "asf.catalog".to_owned(),
                    created_at: "2026-11-01T00:00:00Z".to_owned(),
                    aoi_name: "Orbit S4 AOI".to_owned(),
                    aoi_geojson: None,
                    query: Map::from_iter([(
                        "dataset".to_owned(),
                        Value::String("SENTINEL-1".to_owned()),
                    )]),
                    search_stats: None,
                    candidates,
                },
                workbench: SearchWorkbenchState {
                    batch_id: "s4-source-batch".to_owned(),
                    checked_item_ids: vec!["product-0".to_owned(), "product-250".to_owned()],
                    page_index: 0,
                    page_size: 250,
                    revision: 0,
                },
                activate_workbench: false,
            })?;
            Ok(())
        })
        .unwrap();
    }

    /// A mixed selection: one SLC scene (companion-capable) and one GRD
    /// scene (not), for the Owner 11-02 partial-coverage rule.
    fn s4_seed_mixed(service: &ApplicationService) {
        service.mutate_business_store(|store| {
            let candidates = vec![
                SnapshotItem {
                    item_id: "slc-1".to_owned(),
                    provider_id: "asf.catalog".to_owned(),
                    display_name: "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000".to_owned(),
                    acquisition_datetime: Some("2024-03-12T00:00:00Z".to_owned()),
                    footprint_geojson: None,
                    metadata: Map::new(),
                    payload: Map::from_iter([(
                        "scene_id".to_owned(),
                        Value::String(
                            "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000"
                                .to_owned(),
                        ),
                    )]),
                    asset_access: None,
                    execution_policy: None,
                },
                SnapshotItem {
                    item_id: "grd-1".to_owned(),
                    provider_id: "asf.catalog".to_owned(),
                    display_name: "S1A_IW_GRDH_1SDV_20240312T000100_20240312T000127_052931_066A01_00FA".to_owned(),
                    acquisition_datetime: Some("2024-03-12T00:01:00Z".to_owned()),
                    footprint_geojson: None,
                    metadata: Map::new(),
                    payload: Map::from_iter([(
                        "scene_id".to_owned(),
                        Value::String(
                            "S1A_IW_GRDH_1SDV_20240312T000100_20240312T000127_052931_066A01_00FA"
                                .to_owned(),
                        ),
                    )]),
                    asset_access: None,
                    execution_policy: None,
                },
            ];
            store.save_search_batch(&StoreSearchBatchRequest {
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "s4-mixed-batch".to_owned(),
                    provider_id: "asf.catalog".to_owned(),
                    created_at: "2026-11-01T00:00:00Z".to_owned(),
                    aoi_name: "Orbit S4 mixed AOI".to_owned(),
                    aoi_geojson: None,
                    query: Map::from_iter([(
                        "dataset".to_owned(),
                        Value::String("SENTINEL-1".to_owned()),
                    )]),
                    search_stats: None,
                    candidates,
                },
                workbench: SearchWorkbenchState {
                    batch_id: "s4-mixed-batch".to_owned(),
                    checked_item_ids: vec!["slc-1".to_owned(), "grd-1".to_owned()],
                    page_index: 0,
                    page_size: 250,
                    revision: 0,
                },
                activate_workbench: false,
            })?;
            Ok(())
        })
        .unwrap();
    }

    fn s4_options() -> crate::UnifiedCreateTaskOptions {
        crate::UnifiedCreateTaskOptions {
            output_root: std::env::temp_dir().join("orbit-s4-output"),
            layout: DownloadLayout::Sentinel1Slc,
            concurrency: 1,
            include_precise_orbit: None,
        }
    }

    /// SLC defaults to including the precise orbit: the companion task
    /// exists next to the image task without the caller naming the option.
    #[test]
    fn include_precise_orbit_defaults_on_for_slc_and_creates_the_companion_task() {
        let root = s4_root("slc-default");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        s4_seed(&app, true);
        let task = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                s4_options(),
            )
            .expect("slc task with default orbit admitted");
        assert!(task.created);
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            let companion = store
                .load_task(&format!("{}-orbit", task.task_id))?
                .expect("companion orbit task was created by the default");
            assert_eq!(
                companion.snapshot.operation.as_ref().unwrap().payload["source_task_id"],
                Value::String(task.task_id.clone())
            );
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Idempotence: re-creating a task for the same products returns the
    /// existing task (`created: false`) and never creates (or re-creates) a
    /// companion — the first call already created it or declined it.
    #[test]
    fn reusing_an_existing_task_never_creates_a_companion() {
        let root = s4_root("idempotent");
        let app = s4_service(&root.join("tasks.sqlite3"), true, true);
        s4_seed(&app, true);
        // Create the task once with the companion declined...
        let mut declined = s4_options();
        declined.include_precise_orbit = Some(false);
        let first = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                declined,
            )
            .unwrap();
        assert!(first.created);
        // ...then ask for it again: the task is reused (created: false) and
        // never gains the companion that was declined the first time.
        let second = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                s4_options(),
            )
            .expect("the existing task is reused");
        assert!(!second.created);
        assert_eq!(first.task_id, second.task_id);
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            assert!(
                store
                    .load_task(&format!("{}-orbit", second.task_id))?
                    .is_none(),
                "a reused task never gains a companion"
            );
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Review finding 2: re-issuing `Some(true)` against a live reused task
    /// never errors — the reuse check runs before the explicit-request
    /// refusals — and the TaskRef carries the orbit warning: the reused
    /// task's companion state (created then or not) rides along as
    /// 「已复用现有任务，精密轨道未附带」 when no companion exists.
    #[test]
    fn reusing_with_explicit_orbit_never_errors_and_warns() {
        let root = s4_root("reuse-explicit");
        let app = s4_service(&root.join("tasks.sqlite3"), true, true);
        s4_seed(&app, true);
        // Create the task once with the companion declined (no companion).
        let mut declined = s4_options();
        declined.include_precise_orbit = Some(false);
        let first = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                declined,
            )
            .unwrap();
        assert!(first.created);
        // Re-issue with Some(true): the task is reused, never an error, and
        // the ref carries the typed reuse warning.
        let mut explicit = s4_options();
        explicit.include_precise_orbit = Some(true);
        let second = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                explicit,
            )
            .expect("re-issuing Some(true) against a live task never errors");
        assert!(!second.created);
        assert_eq!(first.task_id, second.task_id);
        assert_eq!(
            second.warning.as_deref(),
            Some("已复用现有任务，精密轨道未附带"),
            "the reuse names why no companion rides along"
        );
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Review finding 2: the reuse check runs before the Some(true)
    /// refusals — re-issuing `Some(true)` against a live task never errors
    /// even when the orbit capability is unavailable.
    #[test]
    fn reusing_with_explicit_orbit_and_unavailable_capability_still_reuses() {
        let root = s4_root("reuse-explicit-no-capability");
        // Create the task while the capability is available.
        {
            let app = s4_service(&root.join("tasks.sqlite3"), true, true);
            s4_seed(&app, true);
            let mut declined = s4_options();
            declined.include_precise_orbit = Some(false);
            let first = app
                .create_task(
                    "s4-source-batch".to_owned(),
                    0,
                    vec!["product-0".to_owned()],
                    declined,
                )
                .unwrap();
            assert!(first.created);
            app.shutdown().unwrap();
        }
        // Reopen with the orbit capability unavailable: re-issuing Some(true)
        // against the live task must reuse it, not refuse.
        let app = s4_service(&root.join("tasks.sqlite3"), false, true);
        let mut explicit = s4_options();
        explicit.include_precise_orbit = Some(true);
        let second = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                explicit,
            )
            .expect("reuse precedes the explicit-request refusals");
        assert!(!second.created);
        assert_eq!(
            second.warning.as_deref(),
            Some("已复用现有任务，精密轨道未附带"),
        );
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Review finding 2: a reused task whose companion exists carries no
    /// orbit warning (the companion is already riding along).
    #[test]
    fn reusing_with_explicit_orbit_and_existing_companion_warns_nothing() {
        let root = s4_root("reuse-explicit-companion");
        let app = s4_service(&root.join("tasks.sqlite3"), true, true);
        s4_seed(&app, true);
        // Create the task once with the companion included.
        let first = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                s4_options(),
            )
            .unwrap();
        assert!(first.created);
        let mut explicit = s4_options();
        explicit.include_precise_orbit = Some(true);
        let second = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                explicit,
            )
            .expect("the task is reused");
        assert!(!second.created);
        assert!(
            second.warning.is_none(),
            "an existing companion needs no warning: {second:?}"
        );
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Burst asset identity helper: one burst parent product with the real
    /// CMR shape (native-id scene id, assets on
    /// sentinel1-burst.asf.alaska.edu under the parent scene path).
    fn burst_asset(
        parent_scene: &str,
        file: &str,
        media_type: &str,
        size_bytes: Option<i64>,
    ) -> insar_core::ProviderAsset {
        let parent_key = format!("ASF::SLC-BURST::{parent_scene}");
        let asset_key = format!("{parent_scene}:{file}");
        insar_core::ProviderAsset {
            stable_asset_key: Some(insar_core::stable_asset_key(&parent_key, &asset_key).unwrap()),
            asset_key: Some(asset_key),
            access_descriptor: None,
            role: if file.ends_with(".tiff") {
                insar_core::AssetRole::Primary
            } else {
                insar_core::AssetRole::Metadata
            },
            native_roles: vec!["USE SERVICE API".to_owned()],
            url: Some(format!(
                "https://sentinel1-burst.asf.alaska.edu/{parent_scene}/{file}"
            )),
            file_name: Some(file.to_owned()),
            media_type: Some(media_type.to_owned()),
            size_bytes,
        }
    }

    fn burst_parent(scene: &str, acquisition: &str) -> SnapshotItem {
        let assets = vec![
            burst_asset(scene, "measurement.tiff", "image/tiff", Some(1_024)),
            burst_asset(scene, "metadata.xml", "application/xml", None),
        ];
        SnapshotItem {
            item_id: format!("ASF::SLC-BURST::{scene}"),
            provider_id: "asf.catalog".to_owned(),
            display_name: scene.to_owned(),
            acquisition_datetime: Some(acquisition.to_owned()),
            footprint_geojson: None,
            metadata: Map::from_iter([
                (
                    "provider".to_owned(),
                    Value::String("asf.catalog".to_owned()),
                ),
                ("dataset".to_owned(), Value::String("SLC-BURST".to_owned())),
                (
                    "product_family".to_owned(),
                    Value::String("BURST".to_owned()),
                ),
                ("scene_id".to_owned(), Value::String(scene.to_owned())),
                ("assets".to_owned(), serde_json::to_value(&assets).unwrap()),
            ]),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        }
    }

    fn seed_burst_batch(service: &ApplicationService, parents: Vec<SnapshotItem>) {
        let ids = parents
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>();
        service
            .mutate_business_store(|store| {
                store.save_search_batch(&StoreSearchBatchRequest {
                    batch: FrozenSearchBatch {
                        schema_version: 1,
                        batch_id: "s4-burst-batch".to_owned(),
                        provider_id: "asf.catalog".to_owned(),
                        created_at: "2026-11-01T00:00:00Z".to_owned(),
                        aoi_name: "Orbit S4 burst AOI".to_owned(),
                        aoi_geojson: None,
                        query: Map::from_iter([(
                            "dataset".to_owned(),
                            Value::String("SLC-BURST".to_owned()),
                        )]),
                        search_stats: None,
                        candidates: parents,
                    },
                    workbench: SearchWorkbenchState {
                        batch_id: "s4-burst-batch".to_owned(),
                        checked_item_ids: ids,
                        page_index: 0,
                        page_size: 250,
                        revision: 0,
                    },
                    activate_workbench: false,
                })?;
                Ok(())
            })
            .unwrap();
    }

    /// A Burst task with bursts from two parent scenes creates one companion
    /// carrying two orbit items — one per distinct parent scene.
    #[test]
    fn burst_task_with_two_parent_scenes_creates_one_companion_with_two_orbit_items() {
        let root = s4_root("burst-two-parents");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        seed_burst_batch(
            &app,
            vec![
                burst_parent(
                    "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000",
                    "2024-03-12T00:00:00Z",
                ),
                burst_parent(
                    "S1A_IW_SLC__1SDV_20240312T000100_20240312T000127_052931_066A01_00FA",
                    "2024-03-12T00:01:00Z",
                ),
            ],
        );
        let mut options = s4_options();
        options.include_precise_orbit = Some(true);
        options.layout = DownloadLayout::Direct;
        let task = app
            .create_task(
                "s4-burst-batch".to_owned(),
                0,
                vec![
                    "ASF::SLC-BURST::S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000".to_owned(),
                    "ASF::SLC-BURST::S1A_IW_SLC__1SDV_20240312T000100_20240312T000127_052931_066A01_00FA".to_owned(),
                ],
                options,
            )
            .expect("the burst task and its companion are created");
        assert!(task.created);
        assert!(task.warning.is_none(), "no partial warning: {task:?}");
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            let companion = store
                .load_task(&format!("{}-orbit", task.task_id))?
                .expect("the companion covers both parent scenes");
            let mut orbit_ids = companion
                .snapshot
                .candidates
                .iter()
                .map(|item| item.item_id.clone())
                .collect::<Vec<_>>();
            orbit_ids.sort();
            assert_eq!(
                orbit_ids,
                vec![
                    "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000"
                        .to_owned(),
                    "S1A_IW_SLC__1SDV_20240312T000100_20240312T000127_052931_066A01_00FA"
                        .to_owned(),
                ],
                "one orbit item per distinct parent scene"
            );
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Bursts from the same parent scene dedupe to a single orbit item.
    #[test]
    fn bursts_from_the_same_parent_scene_dedupe_to_one_orbit_item() {
        let root = s4_root("burst-same-parent");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        // One parent product expands into one primary and one metadata asset,
        // both under the parent scene path: they dedupe to one orbit item.
        seed_burst_batch(
            &app,
            vec![burst_parent(
                "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000",
                "2024-03-12T00:00:00Z",
            )],
        );
        let mut options = s4_options();
        options.include_precise_orbit = Some(true);
        options.layout = DownloadLayout::Direct;
        let task = app
            .create_task(
                "s4-burst-batch".to_owned(),
                0,
                vec!["ASF::SLC-BURST::S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000".to_owned()],
                options,
            )
            .expect("the burst task and its companion are created");
        assert!(task.created);
        assert!(task.warning.is_none(), "no partial warning: {task:?}");
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            let companion = store
                .load_task(&format!("{}-orbit", task.task_id))?
                .expect("the companion exists");
            assert_eq!(
                companion.snapshot.candidates.len(),
                1,
                "the parent's burst assets dedupe to one orbit item"
            );
            assert!(companion.snapshot.candidates[0]
                .item_id
                .contains("S1A_IW_SLC__1SDV_20240312T000000"));
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Review finding 6: the burst parent scene comes from the frozen
    /// identity (asset metadata scene_id / payload source_item_id), with the
    /// official asset URL as a cross-check. An asset whose URL names a
    /// DIFFERENT parent scene than its frozen identity disagrees with itself:
    /// it counts as unsupported (skipped, counted in the warning) — never
    /// silently pick one of the two. Once the image row exists this is a
    /// warning, never an error.
    #[test]
    fn mismatched_burst_identity_counts_as_unsupported_and_warns() {
        let root = s4_root("burst-identity-mismatch");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        // Identity scene A; the primary asset's URL names scene B.
        let scene_a = "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000";
        let scene_b = "S1A_IW_SLC__1SDV_20240312T000100_20240312T000127_052931_066A01_00FA";
        let mut parent = burst_parent(scene_a, "2024-03-12T00:00:00Z");
        let mut mismatched = burst_asset(scene_a, "measurement.tiff", "image/tiff", Some(1_024));
        mismatched.url = Some(format!(
            "https://sentinel1-burst.asf.alaska.edu/{scene_b}/measurement.tiff"
        ));
        let assets = vec![
            mismatched,
            burst_asset(scene_a, "metadata.xml", "application/xml", None),
        ];
        parent
            .metadata
            .insert("assets".to_owned(), serde_json::to_value(&assets).unwrap());
        seed_burst_batch(&app, vec![parent]);
        let mut options = s4_options();
        options.include_precise_orbit = Some(true);
        options.layout = DownloadLayout::Direct;
        let task = app
            .create_task(
                "s4-burst-batch".to_owned(),
                0,
                vec![format!("ASF::SLC-BURST::{scene_a}")],
                options,
            )
            .expect("the image task is created; the mismatch is a warning");
        assert!(task.created);
        // The warning counts scenes, not files: the parent scene is still
        // covered through its agreeing metadata asset, so nothing is skipped.
        assert!(
            task.warning.is_none(),
            "a covered parent scene is not reported as skipped: {task:?}"
        );
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            assert!(
                store.load_task(&task.task_id)?.is_some(),
                "the image task exists"
            );
            // The metadata asset of the same parent still agrees with its
            // identity, so the companion covers that one scene; only the
            // mismatched primary asset was skipped.
            let companion = store
                .load_task(&format!("{}-orbit", task.task_id))?
                .expect("the agreeing asset still gets its orbit item");
            assert_eq!(companion.snapshot.candidates.len(), 1);
            assert!(companion.snapshot.candidates[0]
                .item_id
                .contains("20240312T000000"));
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The skipped count is in scenes: a burst parent whose every asset
    /// disagrees with its frozen identity is one skipped scene, however many
    /// files it has, and the other parent still gets its orbit item.
    #[test]
    fn a_fully_mismatched_burst_parent_counts_as_one_skipped_scene() {
        let root = s4_root("burst-parent-skipped");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        let scene_a = "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000";
        let scene_b = "S1A_IW_SLC__1SDV_20240312T000100_20240312T000127_052931_066A01_00FA";
        let scene_c = "S1A_IW_SLC__1SDV_20240312T000200_20240312T000227_052931_066A01_01F4";
        let mut parent_b = burst_parent(scene_b, "2024-03-12T00:01:00Z");
        let assets = [
            ("measurement.tiff", "image/tiff", Some(1_024)),
            ("metadata.xml", "application/xml", None),
        ]
        .into_iter()
        .map(|(file, mime, size)| {
            let mut asset = burst_asset(scene_b, file, mime, size);
            asset.url = Some(format!(
                "https://sentinel1-burst.asf.alaska.edu/{scene_c}/{file}"
            ));
            asset
        })
        .collect::<Vec<_>>();
        parent_b
            .metadata
            .insert("assets".to_owned(), serde_json::to_value(&assets).unwrap());
        seed_burst_batch(
            &app,
            vec![burst_parent(scene_a, "2024-03-12T00:00:00Z"), parent_b],
        );
        let mut options = s4_options();
        options.include_precise_orbit = Some(true);
        options.layout = DownloadLayout::Direct;
        let task = app
            .create_task(
                "s4-burst-batch".to_owned(),
                0,
                vec![
                    format!("ASF::SLC-BURST::{scene_a}"),
                    format!("ASF::SLC-BURST::{scene_b}"),
                ],
                options,
            )
            .expect("the image task is created; the skipped parent is a warning");
        assert!(task.created);
        assert_eq!(task.warning.as_deref(), Some("1 景暂无法附带精密轨道"));
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            let companion = store
                .load_task(&format!("{}-orbit", task.task_id))?
                .expect("the other parent still gets its orbit item");
            assert_eq!(companion.snapshot.candidates.len(), 1);
            assert!(companion.snapshot.candidates[0]
                .item_id
                .contains("20240312T000000"));
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The preflight warning and the orbit warning combine (joined with
    /// 「；」, preflight first) instead of the orbit warning being dropped
    /// when the account preflight already produced one.
    struct WarningPreflight;
    impl crate::TaskAdmissionPreflight for WarningPreflight {
        fn check(
            &self,
            _batch: &FrozenSearchBatch,
            _selected_items: &[SnapshotItem],
            _item_ids: &[String],
        ) -> Result<(), insar_core::TypedError> {
            Ok(())
        }
        fn admission_warning(&self) -> Option<String> {
            Some("账号会话由传输任务自行建立".to_owned())
        }
    }

    #[test]
    fn orbit_warning_joins_the_preflight_warning_instead_of_dropping() {
        let root = s4_root("warning-join");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        s4_seed_mixed(&app);
        let mut options = s4_options();
        options.include_precise_orbit = Some(true);
        let task = app
            .create_task_with_preflight(
                "s4-mixed-batch".to_owned(),
                0,
                vec!["slc-1".to_owned(), "grd-1".to_owned()],
                options,
                &WarningPreflight,
            )
            .expect("a partial companion with both warnings is not an error");
        assert!(task.created);
        assert_eq!(
            task.warning.as_deref(),
            Some("账号会话由传输任务自行建立；1 景暂无法附带精密轨道")
        );
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// GRD (non-SLC product type) defaults to excluding the precise orbit:
    /// no companion task, and the create call still succeeds.
    #[test]
    fn include_precise_orbit_defaults_off_for_non_slc() {
        let root = s4_root("grd-default");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        // The GRDH-seeded product has no SLC identity, which is the
        // GRD-class default-off case.
        s4_seed(&app, false);
        let task = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                s4_options(),
            )
            .expect("non-slc task admitted without an orbit companion");
        assert!(task.created);
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            assert!(store
                .load_task(&format!("{}-orbit", task.task_id))?
                .is_none());
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// An explicit false turns the companion off even for SLC.
    #[test]
    fn include_precise_orbit_false_skips_the_companion_for_slc() {
        let root = s4_root("slc-off");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        s4_seed(&app, true);
        let mut options = s4_options();
        options.include_precise_orbit = Some(false);
        let task = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                options,
            )
            .expect("slc task without orbit admitted");
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            assert!(store
                .load_task(&format!("{}-orbit", task.task_id))?
                .is_none());
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// With the capability unavailable and the orbit explicitly requested
    /// (`Some(true)`), the create call fails with a typed, understandable
    /// reason — never a silent skip of the companion task.
    #[test]
    fn include_precise_orbit_explicit_with_unavailable_capability_fails_with_a_typed_reason() {
        let root = s4_root("no-capability-explicit");
        let app = s4_service(&root.join("tasks.sqlite3"), false, false);
        s4_seed(&app, true);
        let mut options = s4_options();
        options.include_precise_orbit = Some(true);
        let error = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                options,
            )
            .expect_err("explicit capability-unavailable create fails");
        assert_eq!(error.code, "V3_STORE_READINESS_UNSUPPORTED");
        assert!(!error.message_zh.is_empty());
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// With the capability unavailable and the orbit only defaulted on
    /// (`None` + SLC policy), the image task is still created, the companion
    /// is skipped, and a typed warning rides the TaskRef — never a silent
    /// skip and never a blocked image download.
    #[test]
    fn include_precise_orbit_defaulted_with_unavailable_capability_warns_and_creates_the_image() {
        let root = s4_root("no-capability-defaulted");
        let app = s4_service(&root.join("tasks.sqlite3"), false, false);
        s4_seed(&app, true);
        let task = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                s4_options(),
            )
            .expect("the image task is created despite the unavailable orbit capability");
        assert!(task.created);
        assert_eq!(
            task.warning.as_deref(),
            Some("精密轨道暂不可用，本次未附带")
        );
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            assert!(
                store
                    .load_task(&format!("{}-orbit", task.task_id))?
                    .is_none(),
                "no companion task exists for the defaulted orbit"
            );
            assert!(
                store.load_task(&task.task_id)?.is_some(),
                "the image task exists"
            );
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Owner 11-02 rule: an explicitly requested companion (`Some(true)`)
    /// over a mixed selection creates the image task, the companion for the
    /// supported scenes, and a typed TaskRef warning naming how many scenes
    /// were skipped. A partial unavailability is never an error.
    #[test]
    fn include_precise_orbit_explicit_partial_creates_companion_for_supported_scenes() {
        let root = s4_root("partial-explicit");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        s4_seed_mixed(&app);
        let mut options = s4_options();
        options.include_precise_orbit = Some(true);
        let task = app
            .create_task(
                "s4-mixed-batch".to_owned(),
                0,
                vec!["slc-1".to_owned(), "grd-1".to_owned()],
                options,
            )
            .expect("a partial companion is not an error");
        assert!(task.created);
        assert_eq!(task.warning.as_deref(), Some("1 景暂无法附带精密轨道"));
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            let companion = store
                .load_task(&format!("{}-orbit", task.task_id))?
                .expect("the companion covers the supported scene");
            assert_eq!(companion.snapshot.candidates.len(), 1);
            assert!(companion.snapshot.candidates[0].item_id.contains("SLC"));
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Owner 11-02 rule: an explicitly requested companion (`Some(true)`)
    /// with NO supported scene refuses with the typed, understandable
    /// reason instead of creating an image task without any orbit.
    #[test]
    fn include_precise_orbit_explicit_with_no_supported_scene_refuses() {
        let root = s4_root("unsupported-explicit");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        s4_seed_mixed(&app);
        let mut options = s4_options();
        options.include_precise_orbit = Some(true);
        let error = app
            .create_task(
                "s4-mixed-batch".to_owned(),
                0,
                vec!["grd-1".to_owned()],
                options,
            )
            .expect_err("an explicitly requested companion with no supported scene refuses");
        assert_eq!(error.code, "V3_APPLICATION_INVALID_REQUEST");
        assert!(error.message_zh.contains("精密轨道"));
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The overview projects `source_task_id` and `auxiliary_kind` from the
    /// companion-orbit operation payload; the image task projects neither.
    #[test]
    fn overview_projects_source_task_id_and_auxiliary_kind_for_companion_tasks() {
        let root = s4_root("overview");
        let app = s4_service(&root.join("tasks.sqlite3"), true, true);
        s4_seed(&app, true);
        let task = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                s4_options(),
            )
            .unwrap();
        let overview = app.task_overview().unwrap();
        let companion = overview
            .queue
            .iter()
            .find(|queued| queued.source_task_id.is_some())
            .expect("companion task carries source_task_id");
        assert_eq!(
            companion.source_task_id.as_deref(),
            Some(task.task_id.as_str())
        );
        assert_eq!(companion.auxiliary_kind.as_deref(), Some("AUX_POEORB"));
        let image = overview
            .queue
            .iter()
            .find(|queued| queued.task_id == task.task_id)
            .expect("image task is in the queue");
        assert_eq!(image.source_task_id, None);
        assert_eq!(image.auxiliary_kind, None);
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A seeded schedule row (the ORB003 exhausted state, durable for its
    /// audit trail) projects into the task page item's `orbit_recheck`; the
    /// image task's page stays without one.
    #[test]
    fn task_page_projects_the_persisted_orbit_recheck_row() {
        let root = s4_root("page");
        let app = s4_service(&root.join("tasks.sqlite3"), true, false);
        s4_seed(&app, true);
        let task = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned()],
                s4_options(),
            )
            .unwrap();
        let companion_id = format!("{}-orbit", task.task_id);
        app.mutate_business_store(|store| {
            store.upsert_orbit_recheck(&insar_store::StoreOrbitRecheckRequest {
                task_id: companion_id.clone(),
                item_id: "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000"
                    .to_owned(),
                scene_id: "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000"
                    .to_owned(),
                orbit_type: "POEORB".to_owned(),
                acquisition_at: "2024-03-12T00:00:00Z".to_owned(),
                attempt: 1,
                next_check_at: "2026-11-02T00:00:00Z".to_owned(),
                deadline_at: "2026-12-31T00:00:00Z".to_owned(),
                state: insar_store::StoreOrbitRecheckState::Exhausted,
                last_error_code: Some("ORB003".to_owned()),
                updated_at: "2026-11-01T00:00:00Z".to_owned(),
            })?;
            Ok(())
        })
        .unwrap();
        let page = app
            .task_page(&insar_core::StoreTaskPageRequest {
                task_id: companion_id,
                expected_revision: 0,
                page_index: 0,
                page_size: 100,
            })
            .unwrap();
        let with_row = page
            .items
            .iter()
            .find(|item| item.orbit_recheck.is_some())
            .expect("the seeded row projects into the page");
        let recheck = with_row.orbit_recheck.as_ref().unwrap();
        assert_eq!(recheck.orbit_type, "POEORB");
        assert_eq!(recheck.state, "exhausted");
        assert_eq!(recheck.attempt, 1);
        assert_eq!(recheck.next_check_at, "2026-11-02T00:00:00Z");
        assert_eq!(recheck.deadline_at, "2026-12-31T00:00:00Z");
        assert_eq!(recheck.expected_file_name, None);
        let image_page = app
            .task_page(&insar_core::StoreTaskPageRequest {
                task_id: task.task_id.clone(),
                expected_revision: 0,
                page_index: 0,
                page_size: 100,
            })
            .unwrap();
        assert!(image_page
            .items
            .iter()
            .all(|item| item.orbit_recheck.is_none()));
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 「立即检查」 reuses the row-level retry: the service clears the
    /// persisted schedule row for the retried item (and every row of the
    /// task for a whole-task retry). No new command kind. The seeded rows
    /// use the ORB003 exhausted state: waiting rows are only durable while
    /// their item sits queued (the S3 gate), while exhausted rows persist
    /// for their audit trail until a retry clears them.
    #[test]
    fn row_retry_clears_the_persisted_orbit_recheck_row() {
        let root = s4_root("retry");
        let app = s4_service(&root.join("tasks.sqlite3"), true, true);
        s4_seed(&app, true);
        let task = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned(), "product-250".to_owned()],
                s4_options(),
            )
            .unwrap();
        let companion_id = format!("{}-orbit", task.task_id);
        let item_ids = [
            "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000",
            "S1A_IW_SLC__1SDV_20240312T000100_20240312T000127_052931_066A01_00FA",
        ]
        .map(|id: &str| id.to_owned());
        app.mutate_business_store(|store| {
            for id in &item_ids {
                store.upsert_orbit_recheck(&insar_store::StoreOrbitRecheckRequest {
                    task_id: companion_id.clone(),
                    item_id: id.clone(),
                    scene_id: id.clone(),
                    orbit_type: "POEORB".to_owned(),
                    acquisition_at: "2024-03-12T00:00:00Z".to_owned(),
                    attempt: 1,
                    next_check_at: "2036-11-02T00:00:00Z".to_owned(),
                    deadline_at: "2044-05-11T00:00:00Z".to_owned(),
                    state: insar_store::StoreOrbitRecheckState::Exhausted,
                    last_error_code: Some("ORB003".to_owned()),
                    updated_at: "2026-11-01T00:00:00Z".to_owned(),
                })?;
            }
            Ok(())
        })
        .unwrap();
        // The ORB003 (exhausted) finish leaves its item failed; drive the
        // same terminal state so the user-facing row retry is a valid
        // revival, the way「立即检查」revives an exhausted row.
        // The scheduler may bump the task revision between a read and a
        // command (the HoldingWorker keeps items active); retry the small
        // conflict window instead of racing it.
        fn command_with_retry(
            app: &ApplicationService,
            companion_id: &str,
            build: impl Fn(u64) -> insar_core::StoreUpdateTaskRequest,
        ) -> CoreServiceSnapshot {
            for _ in 0..20 {
                let revision = app
                    .with_business_runtime(|runtime| {
                        let store = runtime.store.lock().unwrap();
                        Ok(store
                            .load_task(companion_id)?
                            .map(|task| task.runtime.revision))
                    })
                    .unwrap()
                    .unwrap();
                match app.update_download_task(&build(revision)) {
                    Ok(snapshot) => return snapshot,
                    Err(ApplicationServiceError::Store(
                        insar_store::StoreError::RevisionConflict,
                    )) => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(error) => panic!("unexpected command error: {error:?}"),
                }
            }
            panic!("command kept conflicting with the scheduler");
        }
        let _ = command_with_retry(&app, &companion_id, |revision| {
            insar_core::StoreUpdateTaskRequest {
                task_id: companion_id.to_owned(),
                expected_revision: revision,
                command: insar_core::StoreTaskCommand {
                    kind: insar_core::TaskStateCommandKind::MarkFailed,
                    item_ids: vec![item_ids[0].clone()],
                },
                event_id: "orbit-s4-mark-failed".to_owned(),
                occurred_at: "2026-11-01T00:00:00Z".to_owned(),
            }
        });
        let _ = command_with_retry(&app, &companion_id, |revision| {
            insar_core::StoreUpdateTaskRequest {
                task_id: companion_id.to_owned(),
                expected_revision: revision,
                command: insar_core::StoreTaskCommand {
                    kind: insar_core::TaskStateCommandKind::RetryItems,
                    item_ids: vec![item_ids[0].clone()],
                },
                event_id: "orbit-s4-retry-row".to_owned(),
                occurred_at: "2026-11-01T00:00:00Z".to_owned(),
            }
        });
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            assert!(
                store
                    .load_orbit_recheck(&companion_id, &item_ids[0])?
                    .is_none(),
                "the retried row's schedule row is cleared"
            );
            assert!(
                store
                    .load_orbit_recheck(&companion_id, &item_ids[1])?
                    .is_some(),
                "the untouched row keeps its audit trail"
            );
            Ok(())
        })
        .unwrap();
        // A whole-task retry expands to the task's failed/cancelled items,
        // so drive the remaining item to failed first, then retry the whole
        // task: every schedule row of the task is cleared.
        let _ = command_with_retry(&app, &companion_id, |revision| {
            insar_core::StoreUpdateTaskRequest {
                task_id: companion_id.to_owned(),
                expected_revision: revision,
                command: insar_core::StoreTaskCommand {
                    kind: insar_core::TaskStateCommandKind::MarkFailed,
                    item_ids: vec![item_ids[1].clone()],
                },
                event_id: "orbit-s4-mark-failed-2".to_owned(),
                occurred_at: "2026-11-01T00:00:00Z".to_owned(),
            }
        });
        let _ = command_with_retry(&app, &companion_id, |revision| {
            insar_core::StoreUpdateTaskRequest {
                task_id: companion_id.to_owned(),
                expected_revision: revision,
                command: insar_core::StoreTaskCommand {
                    kind: insar_core::TaskStateCommandKind::RetryTask,
                    item_ids: vec![],
                },
                event_id: "orbit-s4-retry-task".to_owned(),
                occurred_at: "2026-11-01T00:00:00Z".to_owned(),
            }
        });
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            assert!(
                store
                    .load_orbit_recheck(&companion_id, &item_ids[1])?
                    .is_none(),
                "the task retry cleared the remaining row"
            );
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    fn s4_root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "companion-orbit-s4-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// Review finding 1+5: a whole-task retry must clear only the schedule
    /// rows of the items it actually revives (failed ∪ cancelled as the
    /// store expands RetryTask), never the Waiting rows of still-queued
    /// items — otherwise those scenes dispatch immediately with their
    /// attempt/backoff reset. The clear is atomic with the retry (one store
    /// transaction), so a crash cannot leave a queued item gated by a stale
    /// row.
    #[test]
    fn task_retry_keeps_waiting_rows_of_still_queued_items() {
        let root = s4_root("retry-waiting-kept");
        let app = s4_service(&root.join("tasks.sqlite3"), true, true);
        s4_seed(&app, true);
        let task = app
            .create_task(
                "s4-source-batch".to_owned(),
                0,
                vec!["product-0".to_owned(), "product-250".to_owned()],
                s4_options(),
            )
            .unwrap();
        let companion_id = format!("{}-orbit", task.task_id);
        let item_ids = [
            "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000",
            "S1A_IW_SLC__1SDV_20240312T000100_20240312T000127_052931_066A01_00FA",
        ]
        .map(|id: &str| id.to_owned());
        // One Exhausted row on a failed item (the retry revives it) and one
        // Waiting row on a queued, gated item (the retry must not touch it).
        app.mutate_business_store(|store| {
            store.upsert_orbit_recheck(&insar_store::StoreOrbitRecheckRequest {
                task_id: companion_id.clone(),
                item_id: item_ids[0].clone(),
                scene_id: item_ids[0].clone(),
                orbit_type: "POEORB".to_owned(),
                acquisition_at: "2024-03-12T00:00:00Z".to_owned(),
                attempt: 5,
                next_check_at: "2036-11-02T00:00:00Z".to_owned(),
                deadline_at: "2044-05-11T00:00:00Z".to_owned(),
                state: insar_store::StoreOrbitRecheckState::Exhausted,
                last_error_code: Some("ORB003".to_owned()),
                updated_at: "2026-11-01T00:00:00Z".to_owned(),
            })?;
            store.upsert_orbit_recheck(&insar_store::StoreOrbitRecheckRequest {
                task_id: companion_id.clone(),
                item_id: item_ids[1].clone(),
                scene_id: item_ids[1].clone(),
                orbit_type: "POEORB".to_owned(),
                acquisition_at: "2024-03-12T00:01:00Z".to_owned(),
                attempt: 2,
                next_check_at: "2036-11-02T00:00:00Z".to_owned(),
                deadline_at: "2044-05-11T00:00:00Z".to_owned(),
                state: insar_store::StoreOrbitRecheckState::Waiting,
                last_error_code: Some("ORB002".to_owned()),
                updated_at: "2026-11-01T00:00:00Z".to_owned(),
            })?;
            Ok(())
        })
        .unwrap();
        // Drive item 0 to failed (the ORB003 finish state); item 1 stays
        // queued behind its Waiting gate.
        let mut store_revision = app
            .with_business_runtime(|runtime| {
                let store = runtime.store.lock().unwrap();
                Ok(store
                    .load_task(&companion_id)?
                    .map(|task| task.runtime.revision))
            })
            .unwrap()
            .unwrap();
        for attempt in 0..20 {
            match app.update_download_task(&insar_core::StoreUpdateTaskRequest {
                task_id: companion_id.clone(),
                expected_revision: store_revision,
                command: insar_core::StoreTaskCommand {
                    kind: insar_core::TaskStateCommandKind::MarkFailed,
                    item_ids: vec![item_ids[0].clone()],
                },
                event_id: format!("orbit-s4-mark-failed-{attempt}"),
                occurred_at: "2026-11-01T00:00:00Z".to_owned(),
            }) {
                Ok(_) => break,
                Err(ApplicationServiceError::Store(insar_store::StoreError::RevisionConflict)) => {
                    store_revision = app
                        .with_business_runtime(|runtime| {
                            let store = runtime.store.lock().unwrap();
                            Ok(store
                                .load_task(&companion_id)?
                                .map(|task| task.runtime.revision))
                        })
                        .unwrap()
                        .unwrap();
                }
                Err(error) => panic!("unexpected mark-failed error: {error:?}"),
            }
        }
        // The whole-task retry revives only the failed item.
        let mut store_revision = app
            .with_business_runtime(|runtime| {
                let store = runtime.store.lock().unwrap();
                Ok(store
                    .load_task(&companion_id)?
                    .map(|task| task.runtime.revision))
            })
            .unwrap()
            .unwrap();
        for attempt in 0..20 {
            match app.update_download_task(&insar_core::StoreUpdateTaskRequest {
                task_id: companion_id.clone(),
                expected_revision: store_revision,
                command: insar_core::StoreTaskCommand {
                    kind: insar_core::TaskStateCommandKind::RetryTask,
                    item_ids: vec![],
                },
                event_id: format!("orbit-s4-retry-task-{attempt}"),
                occurred_at: "2026-11-01T00:00:00Z".to_owned(),
            }) {
                Ok(_) => break,
                Err(ApplicationServiceError::Store(insar_store::StoreError::RevisionConflict)) => {
                    store_revision = app
                        .with_business_runtime(|runtime| {
                            let store = runtime.store.lock().unwrap();
                            Ok(store
                                .load_task(&companion_id)?
                                .map(|task| task.runtime.revision))
                        })
                        .unwrap()
                        .unwrap();
                }
                Err(error) => panic!("unexpected retry error: {error:?}"),
            }
        }
        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            assert!(
                store
                    .load_orbit_recheck(&companion_id, &item_ids[0])?
                    .is_none(),
                "the revived (exhausted) item's row is cleared"
            );
            let waiting = store
                .load_orbit_recheck(&companion_id, &item_ids[1])?
                .expect("the still-queued waiting item keeps its schedule row");
            assert_eq!(waiting.attempt, 2, "the waiting row keeps its attempt");
            assert_eq!(waiting.state, insar_store::StoreOrbitRecheckState::Waiting);
            Ok(())
        })
        .unwrap();
        app.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
