use super::*;

#[test]
fn durable_planning_checkpoint_precedes_store_freeze() {
    let (case, intent, bootstrap, _source) = concrete_fixture(
        "checkpoint-before-store-freeze",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    plan_and_freeze_nisar_subset_v2_with(
        &work(&case, &intent),
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| {
            let text = fs::read_to_string(bootstrap.planning_checkpoint_path_for_test())
                .expect("Store freeze must not precede its durable recovery checkpoint");
            let checkpoint = parse_planning_workspace_checkpoint_v2(&text).unwrap();
            assert_eq!(
                checkpoint.final_plan_sha256,
                ready.proposed_plan.canonical_semantic_plan_digest
            );
            Ok(ready.proposed_plan)
        },
    )
    .unwrap();
}

#[test]
fn pause_after_store_commit_keeps_the_frozen_plan_recoverable() {
    let (case, intent, bootstrap, _source) =
        concrete_fixture("pause-after-store-commit", NisarSubsetFrequency::A, &["HH"]);
    let mut work = work(&case, &intent);
    let control = WorkerControl::default();
    let mut committed = None;
    let result = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &control,
        bootstrap.as_ref(),
        |ready, control| {
            committed = Some(ready.proposed_plan);
            control.set_paused_for_test(true);
            Err(NisarSubsetPlanningErrorV2::ExecutionStopped)
        },
    );
    assert_eq!(
        result.unwrap_err(),
        NisarSubsetPlanningErrorV2::ExecutionStopped
    );
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, &committed.unwrap()).unwrap();
    let resumed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |_, _| panic!("already committed plan must not freeze again"),
    )
    .expect("a lost Store acknowledgement must retain a usable checkpoint");
    assert!(resumed.planning_workspace_checkpoint_reused());
}

#[test]
fn pause_before_store_commit_reuses_prepared_checkpoint_without_bootstrap() {
    let (case, intent, bootstrap, _source) = concrete_fixture(
        "pause-before-store-commit",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let revalidations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = Arc::clone(&revalidations);
    let bootstrap = Arc::try_unwrap(bootstrap)
        .ok()
        .unwrap()
        .with_revalidation_hook_for_test(Arc::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        }));
    let work = work(&case, &intent);
    let control = WorkerControl::default();
    let result = plan_and_freeze_nisar_subset_v2_with(&work, &control, &bootstrap, |_, control| {
        control.set_paused_for_test(true);
        Err(NisarSubsetPlanningErrorV2::ExecutionStopped)
    });
    assert_eq!(
        result.unwrap_err(),
        NisarSubsetPlanningErrorV2::ExecutionStopped
    );
    let path = bootstrap.planning_checkpoint_path_for_test();
    let before = fs::read(&path).expect("prepared checkpoint survives rejected commit");
    let resumed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        &bootstrap,
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    assert!(resumed.planning_workspace_checkpoint_reused());
    assert_eq!(
        revalidations.load(Ordering::SeqCst),
        1,
        "completed preparation must not issue unaccounted network probes on acknowledgement retry"
    );
    assert_eq!(
        before,
        fs::read(path).unwrap(),
        "must not reinitialize the prepared workspace"
    );
}

#[test]
#[ignore = "explicit read-only inspection of a supplied task snapshot and its metadata part"]
fn inspect_owner_frozen_plan_geolocation_read_only() {
    let snapshot_path = std::env::var("INSAR_DIAGNOSTIC_TASK_SNAPSHOT").unwrap();
    let mut persisted: Value = serde_json::from_slice(&fs::read(snapshot_path).unwrap()).unwrap();
    let object = persisted.as_object_mut().unwrap();
    object.remove("persistence_version");
    object.remove("source_item_ids");
    let candidates = object.remove("selected_products").unwrap();
    object.insert("candidates".to_owned(), candidates);
    let snapshot: FrozenTaskSnapshot = serde_json::from_value(persisted).unwrap();
    for item in &snapshot.candidates {
        let work = DownloadWork {
            task: snapshot.clone(),
            item: item.clone(),
        };
        let plan = parse_nisar_subset_plan_v2_envelope(&item.metadata)
            .unwrap()
            .unwrap()
            .plan()
            .unwrap();
        let root = science_root_for_identity(&plan.source_product_identity).unwrap();
        let file = hdf5::File::open(route_recovery_root(&work).join("ranges.part")).unwrap();
        let geo =
            read_physical_geolocation_inputs_v2(&file, root, &plan.selected_frequency).unwrap();
        for polarization in &plan.selected_polarizations {
            let path = root.selected_science_path(
                match plan.selected_frequency {
                    NisarSubsetFrequency::A => "A",
                    NisarSubsetFrequency::B => "B",
                },
                polarization,
            );
            let dataset = file.dataset(&path).unwrap();
            let result = insar_provider::asf::nisar_subset::build_live_geolocation_evidence(
                &plan.effective_geometry.canonical_geojson,
                plan.selected_frequency.clone(),
                polarization,
                root.swath(),
                &geo.grid_zero_doppler_time,
                &geo.grid_slant_range,
                &geo.swath_zero_doppler_time,
                &geo.swath_slant_range,
                geo.height_layers_m.clone(),
                geo.longitudes.clone(),
                geo.latitudes.clone(),
                dataset.shape().try_into().unwrap(),
                dataset
                    .dcpl()
                    .unwrap()
                    .get_chunk()
                    .unwrap()
                    .unwrap()
                    .try_into()
                    .unwrap(),
                geo.minimum_hae_m,
                geo.maximum_hae_m,
            );
            println!(
                "item={} polarization={} geolocation={:?}",
                item.item_id,
                polarization,
                result.as_ref().map(|e| &e.windows)
            );
            result.unwrap();
        }
    }
}

#[test]
fn legacy_frozen_plan_repairs_missing_checkpoint_from_its_own_prefix() {
    let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let case = route_fixture("legacy-frozen-prefix-repair", NisarSubsetFrequency::A);
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(
        fs::read(&case.source_path).unwrap(),
        ROUTE_ETAG,
        RouteServerMode::Exact,
    );
    let work = route_ready_work(&case, &intent);
    let database = case._root.0.join("legacy-repair.sqlite");
    pause_route_after_planning_once(&case, &server.url, &work, &database);
    let root = route_recovery_root(&work);
    let checkpoint_path = root.join("planning-workspace-v2.json");
    let previous: NisarSubsetPlanningWorkspaceCheckpointV2 =
        serde_json::from_slice(&fs::read(&checkpoint_path).unwrap()).unwrap();
    let workspace = Hdf5RangeWorkspaceV1::new(root.clone()).unwrap();
    let prefix_before = workspace
        .load_planning_prefix_checkpoint_v2()
        .unwrap()
        .unwrap();
    // Only this test-owned fixture is changed, modeling the former crash window.
    fs::remove_file(&checkpoint_path).unwrap();
    let mut store = TaskStore::open(&database).unwrap();
    let frozen_before = store.load_task_record(&work.task.task_id).unwrap().unwrap();
    assert!(
        store
            .load_nisar_subset_execution_anchor_v2(
                &work.task.task_id,
                &work.item.item_id,
                insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            )
            .unwrap()
            .is_none(),
        "this repair models the old pre-science crash window"
    );
    update_route_item_state(
        &mut store,
        &work,
        TaskStateCommandKind::ResumeItems,
        "legacy-repair-resume",
    );
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&store),
        Arc::new(route_worker(server.url.clone(), &case, &work.item)),
        Duration::from_millis(25),
    );
    scheduler.wake();
    let deadline = Instant::now() + Duration::from_secs(180);
    let terminal = loop {
        let task = store
            .lock()
            .unwrap()
            .load_task_record(&work.task.task_id)
            .unwrap()
            .unwrap();
        if matches!(
            task.runtime.status,
            TaskLifecycleStatus::Completed | TaskLifecycleStatus::Failed
        ) {
            break task;
        }
        assert!(
            Instant::now() < deadline,
            "legacy recovery did not terminate"
        );
        wait_for_route_progress();
    };
    scheduler.shutdown();
    assert_eq!(
        terminal.runtime.status,
        TaskLifecycleStatus::Completed,
        "{:?}",
        store
            .lock()
            .unwrap()
            .load_download_snapshot(&work.task.task_id)
    );
    let repaired: NisarSubsetPlanningWorkspaceCheckpointV2 =
        serde_json::from_slice(&fs::read(checkpoint_path).unwrap()).unwrap();
    assert_eq!(previous.final_plan_sha256, repaired.final_plan_sha256);
    assert_eq!(previous.physical_plan, repaired.physical_plan);
    assert_eq!(
        frozen_before.snapshot.candidates[0].metadata, terminal.snapshot.candidates[0].metadata,
        "reconstruction must not replace the frozen Store plan"
    );
    assert_eq!(
        previous.sparse_part_identity_sha256,
        repaired.sparse_part_identity_sha256
    );
    assert_eq!(
        prefix_before,
        workspace
            .load_planning_prefix_checkpoint_v2()
            .unwrap()
            .unwrap()
    );
    let requests = server.finish();
    for range in previous
        .bootstrap_statistics
        .metadata_prefix()
        .requested_ranges()
    {
        let expected = format!("{}-{}", range.start(), range.end_exclusive() - 1);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request_range_value(request) == expected)
                .count(),
            1,
            "legacy repair redownloaded a verified prefix"
        );
    }
}

// A missing planning record is recoverable only if reconstruction retains the
// exact physical identity already committed by science. In particular, the new
// production budget cannot silently replace a legacy budget under an old anchor.
fn assert_missing_planning_checkpoint_with_science_anchor(legacy_budget: bool) {
    let label = if legacy_budget {
        "missing-planning-legacy-science"
    } else {
        "missing-planning-current-science"
    };
    let case = route_fixture(label, NisarSubsetFrequency::A);
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(
        fs::read(&case.source_path).unwrap(),
        ROUTE_ETAG,
        RouteServerMode::Exact,
    );
    let work = route_ready_work(&case, &intent);
    let database = case._root.0.join("science-repair.sqlite");
    pause_route_after_planning_once(&case, &server.url, &work, &database);
    let root = route_recovery_root(&work);
    let planning_path = root.join("planning-workspace-v2.json");
    let mut original: NisarSubsetPlanningWorkspaceCheckpointV2 =
        serde_json::from_slice(&fs::read(&planning_path).unwrap()).unwrap();
    if legacy_budget {
        let budget = &original.physical_plan.combined_budget;
        original.physical_plan.combined_budget = project_combined_budget_with_policy_v2(
            budget.committed_planning_request_count,
            budget.committed_planning_response_body_bytes,
            budget.projected_science_request_count,
            budget.projected_science_response_body_bytes,
            original.source_size,
            total_request_limit_v2(NisarScienceRoot::Lsar).unwrap(),
            None,
        )
        .unwrap();
        original.physical_plan.physical_plan_sha256 = original
            .physical_plan
            .recompute_physical_plan_sha256()
            .unwrap();
        original.checkpoint_sha256 = original.recompute_sha256().unwrap();
        // Test-owned fixture models a correctly persisted older producer, before
        // any science authority is created. No production anchor is forged.
        fs::write(&planning_path, serde_json::to_vec(&original).unwrap()).unwrap();
    }
    let mut store = TaskStore::open(&database).unwrap();
    update_route_item_state(
        &mut store,
        &work,
        TaskStateCommandKind::ResumeItems,
        "first-science-resume",
    );
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let (tx, rx) = mpsc::channel();
    let worker = route_worker(server.url.clone(), &case, &work.item)
        .with_nisar_subset_v2_after_science_commit_hook_for_test(Arc::new(
            move |index, control| {
                assert_eq!(index, 0);
                tx.send(()).unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while !control.is_paused() {
                    assert!(Instant::now() < deadline, "science pause was not observed");
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
    rx.recv_timeout(Duration::from_secs(30)).unwrap();
    update_route_item_state(
        &mut store.lock().unwrap(),
        &work,
        TaskStateCommandKind::PauseItems,
        "first-science-pause",
    );
    scheduler.wake();
    wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Interrupted);
    scheduler.shutdown();
    let mut store = Arc::try_unwrap(store).ok().unwrap().into_inner().unwrap();
    let anchor_before = store
        .load_nisar_subset_execution_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )
        .unwrap()
        .unwrap();
    let science_path = root.join("science-checkpoint-v2.json");
    let science_before = fs::read(&science_path).unwrap();
    let science: Value = serde_json::from_slice(&science_before).unwrap();
    assert_eq!(science["completed_ranges"].as_array().unwrap().len(), 1);
    assert_eq!(
        science["physical_plan_sha256"],
        json!(original.physical_plan.physical_plan_sha256())
    );
    let first_range = &science["completed_ranges"][0];
    let first_request = format!(
        "{}-{}",
        first_range["start"].as_u64().unwrap(),
        first_range["end_exclusive"].as_u64().unwrap() - 1
    );
    let before_requests = server.requests.lock().unwrap().len();
    let prefix_before = fs::read(root.join("planning-prefix-v2.json")).unwrap();
    let frozen_before = store
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap()
        .snapshot
        .candidates[0]
        .metadata
        .clone();
    fs::remove_file(&planning_path).unwrap();
    update_route_item_state(
        &mut store,
        &work,
        TaskStateCommandKind::ResumeItems,
        "missing-planning-resume",
    );
    drop(store);
    let store: SharedTaskStore = Arc::new(Mutex::new(TaskStore::open(&database).unwrap()));
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&store),
        Arc::new(route_worker(server.url.clone(), &case, &work.item)),
        Duration::from_millis(25),
    );
    scheduler.wake();
    if legacy_budget {
        wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Failed);
    } else {
        wait_for_route_completed(&store, &work.task.task_id);
    }
    scheduler.shutdown();
    let requests = server.finish();
    let store = store.lock().unwrap();
    let repaired: NisarSubsetPlanningWorkspaceCheckpointV2 =
        serde_json::from_slice(&fs::read(planning_path).unwrap()).unwrap();
    let final_task = store.load_task_record(&work.task.task_id).unwrap().unwrap();
    assert_eq!(frozen_before, final_task.snapshot.candidates[0].metadata);
    assert_eq!(
        prefix_before,
        fs::read(root.join("planning-prefix-v2.json")).unwrap()
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request_range_value(request) == first_request)
            .count(),
        1,
        "committed science Range must not be fetched again"
    );
    let anchor_after = store
        .load_nisar_subset_execution_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )
        .unwrap()
        .unwrap();
    if legacy_budget {
        assert_eq!(final_task.runtime.status, TaskLifecycleStatus::Failed);
        assert_ne!(
            original.physical_plan.physical_plan_sha256(),
            repaired.physical_plan.physical_plan_sha256()
        );
        assert_eq!(anchor_before.canonical_json, anchor_after.canonical_json);
        assert_eq!(anchor_before.record_sha256, anchor_after.record_sha256);
        assert_eq!(science_before, fs::read(science_path).unwrap());
        let snapshot = store.load_download_snapshot(&work.task.task_id).unwrap();
        assert_eq!(
            snapshot
                .results
                .iter()
                .find_map(|result| result.failure.as_ref())
                .unwrap()
                .code,
            "NISAR_SUBSET_V2_RECOVERY_INVALID"
        );
        let science_ranges = original.physical_plan.execution_transfer_ranges().unwrap();
        assert!(
            requests[before_requests..].iter().all(|request| {
                let bounds = request_range_bounds(request);
                !science_ranges
                    .iter()
                    .any(|range| bounds == (range.start(), range.end_exclusive()))
            }),
            "changed budget identity must fail before another science request"
        );
    } else {
        assert_eq!(final_task.runtime.status, TaskLifecycleStatus::Completed);
        assert_eq!(original.physical_plan, repaired.physical_plan);
        assert_eq!(
            anchor_before.lineage_key_sha256,
            anchor_after.lineage_key_sha256
        );
        let complete: Value = serde_json::from_slice(&fs::read(science_path).unwrap()).unwrap();
        assert_eq!(complete["state"], json!("Complete"));
        assert!(complete["resumed_bytes"].as_u64().unwrap() > 0);
    }
}

#[test]
fn missing_planning_checkpoint_resumes_only_the_same_science_authority() {
    let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    assert_missing_planning_checkpoint_with_science_anchor(false);
}

#[test]
fn missing_planning_checkpoint_cannot_widen_a_legacy_science_anchor_budget() {
    let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    assert_missing_planning_checkpoint_with_science_anchor(true);
}

#[test]
fn missing_legacy_prefix_cannot_initialize_or_truncate_an_existing_part() {
    let (case, _, bootstrap, _) =
        concrete_fixture("missing-legacy-prefix", NisarSubsetFrequency::A, &["HH"]);
    let workspace = bootstrap.science_workspace_v2();
    let size = 1024;
    let mut part = workspace.reset_planning_part_v2(size).unwrap();
    part.write_all(b"preserved-owner-bytes").unwrap();
    drop(part);
    let before = fs::read(workspace.part_path()).unwrap();
    let error = NisarPrefixMaterializationSessionV2::open_or_initialize_validated(
        workspace,
        size,
        &"a".repeat(64),
        vec![NisarSubsetBootstrapRangeV2 {
            start: 0,
            end_exclusive: 1,
        }],
        &"b".repeat(64),
        true,
        false,
    )
    .err()
    .unwrap();
    assert_eq!(error, NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing);
    assert_eq!(before, fs::read(workspace.part_path()).unwrap());
    drop(case);
}

#[test]
fn continuing_second_task_does_not_interrupt_first_task_in_shared_scheduler() {
    let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let case = route_fixture("concurrent-task-continue", NisarSubsetFrequency::A);
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(
        fs::read(&case.source_path).unwrap(),
        ROUTE_ETAG,
        RouteServerMode::Exact,
    );
    let work_a = route_ready_work(&case, &intent);
    let mut work_b = work_a.clone();
    work_b.task.task_id.push_str("-second");
    work_b.task.batch_id.push_str("-second");
    let mut initial = TaskStore::open(case._root.0.join("concurrent.sqlite")).unwrap();
    seed_scheduler_task(&mut initial, &work_a);
    seed_scheduler_task(&mut initial, &work_b);
    resume_scheduler_task(&mut initial, &work_a);
    let store: SharedTaskStore = Arc::new(Mutex::new(initial));
    let (boundary_tx, boundary_rx) = mpsc::channel();
    let worker = route_worker(server.url.clone(), &case, &work_a.item)
        .with_nisar_subset_v2_after_planning_hook_for_test(Arc::new(move |control| {
            boundary_tx.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(120);
            while !control.is_paused() && !control.is_cancelled() {
                assert!(Instant::now() < deadline, "task control was not observed");
                wait_for_route_progress();
            }
        }));
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&store),
        Arc::new(worker),
        Duration::from_millis(25),
    );
    scheduler.wake();
    boundary_rx.recv_timeout(Duration::from_secs(60)).unwrap();
    let checkpoint_a_path = route_recovery_root(&work_a).join("planning-workspace-v2.json");
    let checkpoint_a = fs::read(&checkpoint_a_path).unwrap();
    resume_scheduler_task(&mut store.lock().unwrap(), &work_b);
    scheduler.wake();
    boundary_rx.recv_timeout(Duration::from_secs(60)).unwrap();
    assert_eq!(checkpoint_a, fs::read(&checkpoint_a_path).unwrap());
    assert_ne!(route_recovery_root(&work_a), route_recovery_root(&work_b));
    update_route_item_state(
        &mut store.lock().unwrap(),
        &work_b,
        TaskStateCommandKind::PauseItems,
        "pause-second-only",
    );
    scheduler.wake();
    wait_for_route_outcome(&store, &work_b.task.task_id, DownloadOutcome::Interrupted);
    let first = store
        .lock()
        .unwrap()
        .load_task_record(&work_a.task.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(first.runtime.status, TaskLifecycleStatus::Running);
    assert_eq!(
        first.runtime.active_item_ids,
        vec![work_a.item.item_id.clone()]
    );
    assert_eq!(checkpoint_a, fs::read(checkpoint_a_path).unwrap());
    update_route_item_state(
        &mut store.lock().unwrap(),
        &work_a,
        TaskStateCommandKind::PauseItems,
        "pause-first-at-end",
    );
    scheduler.wake();
    wait_for_route_outcome(&store, &work_a.task.task_id, DownloadOutcome::Interrupted);
    scheduler.shutdown();
    let _requests = server.finish();
    for work in [&work_a, &work_b] {
        let snapshot = store
            .lock()
            .unwrap()
            .load_download_snapshot(&work.task.task_id)
            .unwrap();
        assert!(snapshot
            .results
            .iter()
            .all(|result| result.failure.is_none()));
    }
}
