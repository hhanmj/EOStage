//! Output admission before the Store freeze (Issue #8, slice 6): a plan whose
//! derived output exceeds the shared bound, or that the output volume cannot
//! hold with every planned Range, is refused before the freeze, so no science
//! request is spent. The refusal survives the prepared-plan recovery path and
//! the worker states it as a local, Chinese diagnostic.
use super::*;
use crate::nisar_subset_stream_v2::{V2AvailableDiskForTest, V2OutputLimitForTest};

fn plan_with_freeze_probe(
    work: &DownloadWork,
    bootstrap: &NisarSubsetLocalHdf5MetadataBootstrapV2,
    froze: &AtomicBool,
) -> Result<NisarSubsetPlanningCompleteV2, NisarSubsetPlanningErrorV2> {
    plan_and_freeze_nisar_subset_v2_with(work, &WorkerControl::default(), bootstrap, |ready, _| {
        froze.store(true, Ordering::SeqCst);
        Ok(ready.proposed_plan)
    })
}

#[test]
fn planning_refuses_an_output_above_the_bound_before_the_freeze_and_on_recovery() {
    let (case, intent, bootstrap, _source_path) =
        concrete_fixture("planning-admission-limit", NisarSubsetFrequency::A, &["HH"]);
    let work = work(&case, &intent);
    let froze = AtomicBool::new(false);
    {
        // The 4 x 4 fixture window derives 208 bytes (science, mask, scales).
        let _limit = V2OutputLimitForTest::set(64);
        let refused = plan_with_freeze_probe(&work, bootstrap.as_ref(), &froze).unwrap_err();
        let NisarSubsetPlanningErrorV2::OutputLimitExceeded { required, limit } = refused else {
            panic!("expected the output bound to refuse the plan, got {refused:?}");
        };
        assert_eq!((required, limit), (4 * 4 * 9 + (4 + 4) * 8, 64));
        assert!(
            !froze.load(Ordering::SeqCst),
            "a refused plan is never frozen"
        );
        // The prepared checkpoint is recovered on retry; it is refused again
        // before the freeze, not offered to the Store.
        assert_eq!(
            plan_with_freeze_probe(&work, bootstrap.as_ref(), &froze).unwrap_err(),
            refused
        );
        assert!(!froze.load(Ordering::SeqCst));
    }
    // Within the bound the recovered plan is admitted and frozen.
    plan_with_freeze_probe(&work, bootstrap.as_ref(), &froze).unwrap();
    assert!(froze.load(Ordering::SeqCst));
}

#[test]
fn planning_refuses_missing_free_space_for_output_and_pending_ranges_before_the_freeze() {
    let (case, intent, bootstrap, _source_path) =
        concrete_fixture("planning-admission-disk", NisarSubsetFrequency::A, &["HH"]);
    let work = work(&case, &intent);
    let froze = AtomicBool::new(false);
    let refused = {
        let _available = V2AvailableDiskForTest::set(4096);
        plan_with_freeze_probe(&work, bootstrap.as_ref(), &froze).unwrap_err()
    };
    let NisarSubsetPlanningErrorV2::OutputDiskSpaceInsufficient {
        required,
        available,
    } = refused
    else {
        panic!("expected missing free space to refuse the plan, got {refused:?}");
    };
    assert_eq!(available, 4096);
    assert!(
        !froze.load(Ordering::SeqCst),
        "a refused plan is never frozen"
    );
    let completed = plan_with_freeze_probe(&work, bootstrap.as_ref(), &froze).unwrap();
    let physical = completed.physical_execution_plan();
    assert_eq!(
        required,
        crate::nisar_subset_stream_v2::v2_derived_output_logical_bytes(physical).unwrap()
            + physical
                .estimated_execution_transfer_response_bytes()
                .unwrap()
            + crate::nisar_subset_stream_v2::V2_DISK_HEADROOM_BYTES,
        "the planning requirement is the derived output plus every planned Range"
    );
}

/// On the production route an output refused at planning freezes nothing in
/// the Store and sends no science request: every request the server saw is
/// a planning request of the durable planning checkpoint. The item fails with
/// the local Chinese diagnostic and its sizes.
#[test]
fn production_route_v2_output_refusal_freezes_nothing_and_sends_no_science_request() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let label = "route-output-refusal";
    let case = route_fixture(label, NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let work = route_ready_work(&case, &intent);
    let database = case._root.0.join(format!("{label}.sqlite"));
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    resume_scheduler_task(&mut store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let _limit = V2OutputLimitForTest::set(64);
    let worker = Arc::new(route_worker(server.url.clone(), &case, &work.item));
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(25));
    scheduler.wake();
    wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Failed);
    scheduler.shutdown();
    let requests = server.finish();
    let failure = store
        .lock()
        .unwrap()
        .load_download_snapshot(&work.task.task_id)
        .unwrap()
        .results
        .into_iter()
        .find_map(|result| result.failure)
        .unwrap();
    assert_eq!(failure.code, "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT");
    assert!(!failure.retryable);
    for expected in [
        "裁剪结果超出本机输出上限",
        "计划未冻结，没有发起科学数据请求；已下载数据与断点已保留",
        "[stage=planning_admission source=local code=NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT",
        "limit=64]",
    ] {
        assert!(
            failure.reason_zh.contains(expected),
            "{}",
            failure.reason_zh
        );
    }
    let stored = store
        .lock()
        .unwrap()
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    assert!(
        parse_nisar_subset_plan_v2_envelope(&stored.snapshot.candidates[0].metadata)
            .unwrap()
            .is_none(),
        "no plan was frozen in the Store"
    );
    let recovery_workspace = Hdf5RangeWorkspaceV1::new(
        crate::production_worker::nisar_subset_v2_recovery_workspace_root(&work),
    )
    .unwrap();
    let planning_checkpoint: NisarSubsetPlanningWorkspaceCheckpointV2 = serde_json::from_str(
        &recovery_workspace
            .load_planning_checkpoint_v2()
            .unwrap()
            .expect("durable planning checkpoint"),
    )
    .unwrap();
    // Review of slice 7 (P2-5): no request reached a science Range.
    for request in &requests {
        let (start, end) = request_range_bounds(request);
        for range in planning_checkpoint.physical_plan.science_ranges() {
            assert!(
                end <= range.start() || start >= range.end_exclusive(),
                "a science Range was requested: {request}"
            );
        }
    }
    assert_eq!(
        requests.len() as u64,
        planning_checkpoint
            .bootstrap_statistics
            .total_planning_request_count(),
        "only planning requests were sent: {requests:?}"
    );
    assert!(recovery_workspace
        .load_science_checkpoint_v2()
        .unwrap()
        .is_none());
}

/// A frozen plan is admitted again before any science request (the free
/// space may be gone since the freeze, or the plan predates admission): the
/// check before science refuses with its own stage, after the freeze and
/// before any science request.
#[test]
fn production_route_v2_check_before_science_refuses_a_frozen_plan_without_science() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let label = "route-pre-science-refusal";
    let case = route_fixture(label, NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let work = route_ready_work(&case, &intent);
    let database = case._root.0.join(format!("{label}.sqlite"));
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    resume_scheduler_task(&mut store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    // Free space disappears between the freeze and the first science request.
    let full_disk: Arc<Mutex<Option<V2AvailableDiskForTest>>> = Arc::new(Mutex::new(None));
    let hook_disk = Arc::clone(&full_disk);
    let worker = Arc::new(
        route_worker(server.url.clone(), &case, &work.item)
            .with_nisar_subset_v2_after_planning_hook_for_test(Arc::new(move |_| {
                *hook_disk.lock().unwrap() = Some(V2AvailableDiskForTest::set(4096));
            })),
    );
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(25));
    scheduler.wake();
    wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Failed);
    scheduler.shutdown();
    drop(full_disk.lock().unwrap().take());
    let requests = server.finish();
    let failure = store
        .lock()
        .unwrap()
        .load_download_snapshot(&work.task.task_id)
        .unwrap()
        .results
        .into_iter()
        .find_map(|result| result.failure)
        .unwrap();
    assert_eq!(
        failure.code,
        "NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT"
    );
    assert!(
        failure.retryable,
        "freeing space and retrying is the next step"
    );
    for expected in [
        "本次没有发起科学数据请求；已下载数据与断点已保留",
        "[stage=pre_science_admission source=local \
         code=NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT",
        "limit=4096]",
    ] {
        assert!(
            failure.reason_zh.contains(expected),
            "{}",
            failure.reason_zh
        );
    }
    let stored = store
        .lock()
        .unwrap()
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    assert!(
        parse_nisar_subset_plan_v2_envelope(&stored.snapshot.candidates[0].metadata)
            .unwrap()
            .is_some(),
        "the plan was frozen before the check"
    );
    let recovery_workspace = Hdf5RangeWorkspaceV1::new(
        crate::production_worker::nisar_subset_v2_recovery_workspace_root(&work),
    )
    .unwrap();
    let planning_checkpoint: NisarSubsetPlanningWorkspaceCheckpointV2 = serde_json::from_str(
        &recovery_workspace
            .load_planning_checkpoint_v2()
            .unwrap()
            .expect("durable planning checkpoint"),
    )
    .unwrap();
    // Review of slice 7 (P2-5): no request reached a science Range.
    for request in &requests {
        let (start, end) = request_range_bounds(request);
        for range in planning_checkpoint.physical_plan.science_ranges() {
            assert!(
                end <= range.start() || start >= range.end_exclusive(),
                "a science Range was requested: {request}"
            );
        }
    }
    assert_eq!(
        requests.len() as u64,
        planning_checkpoint
            .bootstrap_statistics
            .total_planning_request_count(),
        "no science request was sent: {requests:?}"
    );
    assert!(recovery_workspace
        .load_science_checkpoint_v2()
        .unwrap()
        .is_none());
}
