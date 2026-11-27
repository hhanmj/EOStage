//! A writer failure after a complete science transfer, retried by the user
//! (RetryItems), reuses the frozen plan and every verified Range: the retry
//! sends no request and only produces the output (Issue #8, slice 6).
use super::*;
use crate::nisar_subset_stream_v2::V2AvailableDiskForTest;

#[test]
fn production_route_v2_retry_after_a_writer_failure_only_rewrites_the_output() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let label = "route-writer-retry";
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
    // Free space vanishes once science is under way: planning and the check
    // before science pass, the writer then refuses the output.
    let full_disk: Arc<Mutex<Option<V2AvailableDiskForTest>>> = Arc::new(Mutex::new(None));
    let hook_disk = Arc::clone(&full_disk);
    let worker = Arc::new(
        route_worker(server.url.clone(), &case, &work.item)
            .with_nisar_subset_v2_after_science_commit_hook_for_test(Arc::new(move |_, _| {
                let mut slot = hook_disk.lock().unwrap();
                if slot.is_none() {
                    *slot = Some(V2AvailableDiskForTest::set(4096));
                }
            })),
    );
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(25));
    scheduler.wake();
    wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Failed);
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
        "输出目录所在磁盘剩余空间不足",
        "已下载数据与断点已保留",
        "重试只重新生成文件、不重新下载",
        "[stage=writer source=local code=NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT",
        "limit=4096]",
    ] {
        assert!(
            failure.reason_zh.contains(expected),
            "{}",
            failure.reason_zh
        );
    }
    let requests_before_retry = server.requests.lock().unwrap().len();
    assert!(requests_before_retry > 0);

    // Space is free again; the user retries the failed item.
    drop(full_disk.lock().unwrap().take());
    update_route_item_state(
        &mut store.lock().unwrap(),
        &work,
        TaskStateCommandKind::RetryItems,
        "writer-retry",
    );
    scheduler.wake();
    wait_for_route_completed(&store, &work.task.task_id);
    scheduler.shutdown();
    let requests = server.finish();
    assert_eq!(
        requests.len(),
        requests_before_retry,
        "the retry reused the frozen plan and every verified Range: no request was sent"
    );
    let snapshot = store
        .lock()
        .unwrap()
        .load_download_snapshot(&work.task.task_id)
        .unwrap();
    let completed = snapshot
        .results
        .iter()
        .find(|result| result.outcome == DownloadOutcome::Completed)
        .expect("the retry produced the output");
    assert!(completed.destination.is_file());
}
