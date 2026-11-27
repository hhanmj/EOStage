//! The task card reads an active item's latest progress row (Issue #8, slice
//! 6): a zero-rate row means the science transfer is over and the output is
//! being produced. The worker therefore publishes no zero-rate row before the
//! science TransferComplete row; before it, an item without a row in this run
//! reads as preparing or verifying.
use super::*;

struct ProgressRecordingWorker {
    inner: ProductionDownloadWorker,
    rows: Arc<Mutex<Vec<insar_core::DownloadItemProgress>>>,
}

impl DownloadWorker for ProgressRecordingWorker {
    fn execute_with_authority(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        plan_freeze: &crate::NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
    ) -> insar_core::DownloadItemResult {
        let rows = Arc::clone(&self.rows);
        self.inner
            .execute_with_authority(work, control, plan_freeze, &mut |row| {
                rows.lock().unwrap().push(row.clone());
                progress(row);
            })
    }

    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
    ) -> insar_core::DownloadItemResult {
        self.inner.execute(work, control, progress)
    }
}

#[test]
fn production_route_v2_publishes_no_zero_rate_row_before_the_transfer_is_complete() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let label = "route-progress-phases";
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
    let rows = Arc::new(Mutex::new(Vec::new()));
    let worker = Arc::new(ProgressRecordingWorker {
        inner: route_worker(server.url.clone(), &case, &work.item),
        rows: Arc::clone(&rows),
    });
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(25));
    scheduler.wake();
    wait_for_route_completed(&store, &work.task.task_id);
    scheduler.shutdown();
    drop(server.finish());
    let rows = rows.lock().unwrap().clone();
    let active = rows
        .iter()
        .filter(|row| row.status == insar_core::DownloadItemStatus::Active)
        .collect::<Vec<_>>();
    let zero_rate = active
        .iter()
        .enumerate()
        .filter(|(_, row)| row.bytes_per_second <= 0.0)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert_eq!(
        zero_rate,
        vec![active.len() - 1],
        "the only zero-rate active row is the last one, published when the science \
         transfer completed: {rows:?}"
    );
    assert!(active.last().unwrap().bytes_completed > 0, "{rows:?}");
    assert!(
        active[..active.len() - 1]
            .iter()
            .all(|row| row.bytes_per_second > 0.0 && row.total_bytes.is_none()),
        "{rows:?}"
    );
    assert_eq!(
        rows.last().unwrap().status,
        insar_core::DownloadItemStatus::Completed
    );
}
