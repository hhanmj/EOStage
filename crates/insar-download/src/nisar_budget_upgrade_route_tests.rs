//! Production-route evidence for the one-time NISAR budget upgrade: real Store,
//! real scheduler and worker, a loopback Range server and the retained local
//! workspace. "Crash" runs stop the worker at an exact durable boundary and shut
//! the scheduler down without applying the worker's result, as a killed process
//! would; the next scheduler is the old-Active startup on a reopened SQLite Store.
use super::*;
use crate::nisar_science_transfer_v2::{
    advance_legacy_science_state_v2_for_test, LegacyScienceChangeV2ForTest,
    NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2,
};
use crate::scheduler::{NisarBudgetMigrationHookForTest, NisarBudgetMigrationStageForTest};
use insar_core::{DownloadFailure, DownloadItemProgress, DownloadItemResult, TaskRuntimeSnapshot};
use insar_store::{
    StoreAttachNisarSubsetExecutionPlanV2Request,
    StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request,
    StoreNisarBudgetUpgradeDownloadProgressV2, StoreNisarBudgetUpgradeDownloadResultV2,
    StoredNisarBudgetUpgradeProofV2, StoredNisarBudgetUpgradeRefusalV2,
    StoredNisarSubsetExecutionAnchorV2, NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
};
use rusqlite::OptionalExtension;
use std::sync::atomic::AtomicUsize;

const REFUSED: &str = "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED";

/// Exact five-column download_results row: outcome, finished_at, result_json,
/// last_known_total_bytes and bytes_written.
type RawResultRow = (String, String, String, Option<i64>, i64);

fn sha256_path(path: &std::path::Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn raw_result_row(database: &Path, work: &DownloadWork) -> Option<RawResultRow> {
    rusqlite::Connection::open(database)
        .unwrap()
        .query_row(
            "SELECT outcome,finished_at,result_json,last_known_total_bytes,bytes_written \
             FROM download_results WHERE task_id=?1 AND item_id=?2",
            rusqlite::params![work.task.task_id, work.item.item_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .unwrap()
}

fn raw_row_as_evidence(row: RawResultRow) -> StoreNisarBudgetUpgradeDownloadResultV2 {
    StoreNisarBudgetUpgradeDownloadResultV2::Present {
        outcome: row.0,
        finished_at: row.1,
        result_json: row.2,
        last_known_total_bytes: row.3.map(|value| value as u64),
        bytes_written: row.4 as u64,
    }
}

struct UpgradeRouteWorker {
    inner: ProductionDownloadWorker,
    hook: Option<NisarBudgetMigrationHookForTest>,
    /// Set when a hook panics: the worker then behaves as a killed process.
    crashed: Option<Arc<AtomicBool>>,
    finished: Arc<AtomicUsize>,
}

impl DownloadWorker for UpgradeRouteWorker {
    fn execute_with_authority(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        authority: &crate::NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        if let Some(hook) = &self.hook {
            control.set_budget_migration_hook_for_test(Arc::clone(hook));
        }
        let Some(crashed) = &self.crashed else {
            let result = self
                .inner
                .execute_with_authority(work, control, authority, progress);
            self.finished.fetch_add(1, Ordering::SeqCst);
            return result;
        };
        let interrupted = DownloadItemResult {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            outcome: DownloadOutcome::Interrupted,
            destination: work.task.output_root.clone(),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-10-16T00:00:09Z".into(),
        };
        let worker_control = control.clone();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.inner
                .execute_with_authority(work, worker_control, authority, progress)
        }));
        match outcome {
            Ok(result) => {
                self.finished.fetch_add(1, Ordering::SeqCst);
                result
            }
            Err(_) => {
                // A killed process applies nothing more: hold this worker until
                // scheduler shutdown cancels it, so its result is never persisted.
                crashed.store(true, Ordering::SeqCst);
                let deadline = Instant::now() + Duration::from_secs(60);
                while !control.is_cancelled() {
                    assert!(
                        Instant::now() < deadline,
                        "crashed worker was never shut down"
                    );
                    wait_for_route_progress();
                }
                interrupted
            }
        }
    }

    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.inner.execute(work, control, progress)
    }
}

/// The frozen legacy V1 ceilings of the seeded plan.
#[derive(Clone, Copy)]
struct LegacyCeilings {
    science_requests: u64,
    response_body_bytes: u64,
}

/// Builds legacy changes from the frozen V1 ceilings and the quiescent local
/// checkpoint JSON.
type LegacyChanges = Box<dyn Fn(LegacyCeilings, &Value) -> Vec<LegacyScienceChangeV2ForTest>>;

/// The legacy state an older build left before this upgrade existed.
enum LegacySeed {
    /// Paused after the first verified Range: a quiescent committed predecessor.
    Quiescent,
    /// Paused after every verified Range except the last one (in plan order).
    QuiescentAllButLastRange,
    /// The quiescent seed followed by one more coherent legacy transition,
    /// optionally left prepared but uncommitted in the anchor.
    Advanced(LegacyChanges, bool),
    /// Crash after the genesis anchor was prepared, before any local checkpoint.
    PendingGenesis,
    /// Planning finished but science never started: no science anchor or checkpoint.
    PlanningOnly,
}

struct RetainedRoute {
    case: ReadyCase,
    work: DownloadWork,
    database: PathBuf,
    server: RouteRangeServer,
    checkpoint: Option<String>,
    part_sha: String,
    old_result: DownloadItemResult,
    ranges: Vec<NisarSubsetPhysicalRangeV2>,
    v1_science_limit: u64,
}

fn seed_retained_route(label: &str) -> RetainedRoute {
    seed_route(label, LegacySeed::Quiescent)
}

fn seed_route(label: &str, seed: LegacySeed) -> RetainedRoute {
    crate::establish_hdf5_plugin_isolation().unwrap();
    let case = route_fixture(label, NisarSubsetFrequency::A);
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let mut work = route_ready_work(&case, &intent);
    let database = case._root.0.join("upgrade.sqlite3");
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    resume_scheduler_task(&mut store, &work);
    store
        .update_scheduler_runtime(&StoreUpdateTaskRequest {
            task_id: work.task.task_id.clone(),
            expected_revision: 2,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec![work.item.item_id.clone()],
            },
            event_id: "seed-active".into(),
            occurred_at: "2026-10-16T00:00:00Z".into(),
        })
        .unwrap();
    let workspace = Hdf5RangeWorkspaceV1::new(route_recovery_root(&work)).unwrap();
    let bootstrap = NisarSubsetLocalHdf5MetadataBootstrapV2::new_for_test(
        case.source_path.clone(),
        fs::metadata(&case.source_path).unwrap().len(),
        workspace.clone(),
    )
    .unwrap();
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        &bootstrap,
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    assert!(completed
        .physical_execution_plan()
        .budget_projection()
        .uses_physical_execution_union_v1());
    store
        .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
            expected_revision: 3,
            occurred_at: "2026-10-16T00:00:01Z".into(),
            plan: completed.frozen_plan().clone(),
        })
        .unwrap();
    let saved = store.load_task_record(&work.task.task_id).unwrap().unwrap();
    work.task = saved.snapshot;
    work.item = work.task.candidates[0].clone();
    let physical = completed.physical_execution_plan();
    let server = RouteRangeServer::start(
        fs::read(&case.source_path).unwrap(),
        &physical.source_validator().value,
        RouteServerMode::Exact,
    );
    let asset = science_asset(
        server.url.trim_end_matches("/source.h5"),
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    if !matches!(seed, LegacySeed::PlanningOnly) {
        let anchor = ScienceCheckpointAnchorFixture::default();
        let authority = science_authority(&bootstrap, &asset, &work, &completed, &anchor);
        let control = WorkerControl::default();
        if matches!(seed, LegacySeed::PendingGenesis) {
            anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Genesis);
            let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();
            assert_eq!(
                executor.execute(&work, &completed, &authority, &control),
                Err(NisarSubsetScienceTransferErrorV2::Persistence)
            );
            assert!(workspace.load_science_checkpoint_v2().unwrap().is_none());
        } else {
            let paused = control.clone();
            let pause_after = match seed {
                LegacySeed::QuiescentAllButLastRange => physical
                    .execution_transfer_ranges()
                    .unwrap()
                    .len()
                    .checked_sub(2)
                    .expect("fixture needs at least two science Ranges"),
                _ => 0,
            };
            let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset)
                .unwrap()
                .with_after_commit_hook_for_test(Arc::new(move |index| {
                    if index == pause_after {
                        paused.set_paused_for_test(true);
                    }
                }));
            assert_eq!(
                executor.execute(&work, &completed, &authority, &control),
                Err(NisarSubsetScienceTransferErrorV2::Paused)
            );
        }
        let (lineage, mut record) = anchor.state.lock().unwrap().clone().unwrap();
        match &seed {
            LegacySeed::Quiescent | LegacySeed::QuiescentAllButLastRange => {
                assert!(!record.has_pending_checkpoint())
            }
            LegacySeed::PendingGenesis => assert!(record.has_pending_checkpoint()),
            LegacySeed::PlanningOnly => unreachable!(),
            LegacySeed::Advanced(changes, leave_pending) => {
                let local: Value =
                    serde_json::from_str(&workspace.load_science_checkpoint_v2().unwrap().unwrap())
                        .unwrap();
                let changes = changes(
                    LegacyCeilings {
                        science_requests: physical.budget_projection().max_science_request_count(),
                        response_body_bytes: physical.budget_projection().max_response_body_bytes(),
                    },
                    &local,
                );
                record = advance_legacy_science_state_v2_for_test(
                    &workspace,
                    &record,
                    &changes,
                    *leave_pending,
                );
            }
        }
        assert!(store
            .compare_and_swap_nisar_subset_execution_anchor_v2(
                &StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
                    task_id: work.task.task_id.clone(),
                    item_id: work.item.item_id.clone(),
                    anchor_kind: NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND.into(),
                    lineage_key_sha256: lineage,
                    expected_record_sha256: None,
                    record_sha256: record.record_sha256().into(),
                    canonical_json: record.canonical_json().unwrap(),
                }
            )
            .unwrap());
    }
    let old_result = DownloadItemResult {
        task_id: work.task.task_id.clone(),
        item_id: work.item.item_id.clone(),
        outcome: DownloadOutcome::Failed,
        destination: work.task.output_root.clone(),
        bytes_written: 0,
        attempt_count: 1,
        failure: Some(DownloadFailure {
            code: "ORIGINAL_FAILURE".into(),
            reason_zh: "Original retained failure text".into(),
            retryable: false,
        }),
        finished_at: "2026-10-16T00:00:02Z".into(),
    };
    store.save_scheduler_download_result(&old_result).unwrap();
    RetainedRoute {
        checkpoint: workspace.load_science_checkpoint_v2().unwrap(),
        part_sha: sha256_path(&workspace.part_path()),
        ranges: physical.execution_transfer_ranges().unwrap(),
        v1_science_limit: physical.budget_projection().max_science_request_count(),
        case,
        work,
        database,
        server,
        old_result,
    }
}

fn route_worker_for(fixture: &RetainedRoute, count: &Arc<AtomicUsize>) -> ProductionDownloadWorker {
    route_worker_with_resolve_counter(
        fixture.server.url.clone(),
        &fixture.case,
        &fixture.work.item,
        Some(Arc::clone(count)),
    )
}

fn start_route_scheduler(
    store: &SharedTaskStore,
    inner: ProductionDownloadWorker,
    hook: Option<NisarBudgetMigrationHookForTest>,
    crashed: Option<Arc<AtomicBool>>,
    finished: Arc<AtomicUsize>,
) -> DownloadScheduler {
    DownloadScheduler::start_with_store(
        Arc::clone(store),
        Arc::new(UpgradeRouteWorker {
            inner,
            hook,
            crashed,
            finished,
        }),
        Duration::from_millis(25),
    )
}

fn open_shared(fixture: &RetainedRoute) -> SharedTaskStore {
    Arc::new(Mutex::new(
        TaskStore::open_existing(&fixture.database).unwrap(),
    ))
}

fn wait_for_runtime(
    store: &SharedTaskStore,
    fixture: &RetainedRoute,
    what: &str,
    done: impl Fn(&TaskRuntimeSnapshot) -> bool,
) -> TaskRuntimeSnapshot {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let runtime = store
            .lock()
            .unwrap()
            .load_task_record(&fixture.work.task.task_id)
            .unwrap()
            .unwrap()
            .runtime;
        if done(&runtime) {
            return runtime;
        }
        assert!(Instant::now() < deadline, "route did not reach: {what}");
        wait_for_route_progress();
    }
}

fn terminal(runtime: &TaskRuntimeSnapshot) -> bool {
    matches!(
        runtime.status,
        TaskLifecycleStatus::Completed | TaskLifecycleStatus::Failed
    )
}

fn run_retained_route(
    fixture: &RetainedRoute,
    hook: Option<NisarBudgetMigrationHookForTest>,
    count: Arc<AtomicUsize>,
) -> TaskLifecycleStatus {
    let store = open_shared(fixture);
    let scheduler = start_route_scheduler(
        &store,
        route_worker_for(fixture, &count),
        hook,
        None,
        Arc::new(AtomicUsize::new(0)),
    );
    let runtime = wait_for_runtime(&store, fixture, "a terminal task", terminal);
    scheduler.shutdown();
    runtime.status
}

/// Stop the worker at an exact durable boundary as a killed process would.
fn crash_retained_route(
    fixture: &RetainedRoute,
    stage: NisarBudgetMigrationStageForTest,
    count: &Arc<AtomicUsize>,
) {
    let store = open_shared(fixture);
    let crashed = Arc::new(AtomicBool::new(false));
    let scheduler = start_route_scheduler(
        &store,
        route_worker_for(fixture, count),
        Some(Arc::new(move |current, _| {
            if current == stage {
                panic!("simulated process crash at {current:?}");
            }
        })),
        Some(Arc::clone(&crashed)),
        Arc::new(AtomicUsize::new(0)),
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    while !crashed.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "crash point {stage:?} not reached"
        );
        wait_for_route_progress();
    }
    scheduler.shutdown();
}

fn item_command(
    store: &mut TaskStore,
    task_id: &str,
    item_id: &str,
    kind: TaskStateCommandKind,
    label: &str,
) {
    let revision = store
        .load_task_record(task_id)
        .unwrap()
        .unwrap()
        .runtime
        .revision;
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: revision,
            command: StoreTaskCommand {
                kind,
                item_ids: vec![item_id.to_owned()],
            },
            event_id: format!("test-{label}-{revision}"),
            occurred_at: "2026-10-16T00:00:05Z".into(),
        })
        .unwrap();
}

fn retry_failed_item(fixture: &RetainedRoute) {
    let mut store = TaskStore::open_existing(&fixture.database).unwrap();
    item_command(
        &mut store,
        &fixture.work.task.task_id,
        &fixture.work.item.item_id,
        TaskStateCommandKind::RetryItems,
        "retry",
    );
}

/// A one-shot hook that issues a real Store PauseItems at `stage` and makes
/// the worker observe it, as the scheduler's live-control propagation would.
fn pause_at(
    stage: NisarBudgetMigrationStageForTest,
    store: &SharedTaskStore,
    fixture: &RetainedRoute,
) -> NisarBudgetMigrationHookForTest {
    let store = Arc::clone(store);
    let task_id = fixture.work.task.task_id.clone();
    let item_id = fixture.work.item.item_id.clone();
    let fired = Arc::new(AtomicBool::new(false));
    Arc::new(move |current, control| {
        if current == stage && !fired.swap(true, Ordering::SeqCst) {
            item_command(
                &mut store.lock().unwrap(),
                &task_id,
                &item_id,
                TaskStateCommandKind::PauseItems,
                "pause",
            );
            control.set_paused_for_test(true);
        }
    })
}

fn load_proof(fixture: &RetainedRoute) -> Option<StoredNisarBudgetUpgradeProofV2> {
    TaskStore::open_existing(&fixture.database)
        .unwrap()
        .load_nisar_budget_upgrade_proof_v2(&fixture.work.task.task_id, &fixture.work.item.item_id)
        .unwrap()
}

fn load_refusals(fixture: &RetainedRoute) -> Vec<StoredNisarBudgetUpgradeRefusalV2> {
    TaskStore::open_existing(&fixture.database)
        .unwrap()
        .list_nisar_budget_upgrade_refusals_v2(
            &fixture.work.task.task_id,
            &fixture.work.item.item_id,
        )
        .unwrap()
}

fn load_store_anchor(fixture: &RetainedRoute) -> Option<StoredNisarSubsetExecutionAnchorV2> {
    TaskStore::open_existing(&fixture.database)
        .unwrap()
        .load_nisar_subset_execution_anchor_v2(
            &fixture.work.task.task_id,
            &fixture.work.item.item_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )
        .unwrap()
}

fn anchor_record(anchor: &StoredNisarSubsetExecutionAnchorV2) -> Value {
    serde_json::from_str(&anchor.canonical_json).unwrap()
}

fn local_checkpoint(fixture: &RetainedRoute) -> Option<Value> {
    Hdf5RangeWorkspaceV1::open_existing(route_recovery_root(&fixture.work))
        .unwrap()
        .load_science_checkpoint_v2()
        .unwrap()
        .map(|text| serde_json::from_str(&text).unwrap())
}

fn visible_failure(fixture: &RetainedRoute) -> Option<DownloadFailure> {
    let results = TaskStore::open_existing(&fixture.database)
        .unwrap()
        .load_download_snapshot(&fixture.work.task.task_id)
        .unwrap()
        .results;
    assert!(results.len() <= 1);
    results.into_iter().next().and_then(|result| result.failure)
}

fn science_range_request_count(requests: &[String], range: &NisarSubsetPhysicalRangeV2) -> usize {
    let header = format!(
        "range: bytes={}-{}\r\n",
        range.start(),
        range.end_exclusive() - 1
    );
    requests
        .iter()
        .filter(|request| request.to_ascii_lowercase().contains(&header))
        .count()
}

fn assert_each_range_requested_once(fixture: &RetainedRoute) {
    let requests = fixture.server.requests.lock().unwrap().clone();
    for range in &fixture.ranges {
        assert_eq!(
            science_range_request_count(&requests, range),
            1,
            "{}-{}",
            range.start(),
            range.end_exclusive()
        );
    }
}

fn assert_completed_with_single_proof(
    fixture: &RetainedRoute,
    status: TaskLifecycleStatus,
) -> StoredNisarBudgetUpgradeProofV2 {
    let store = TaskStore::open_existing(&fixture.database).unwrap();
    assert_eq!(
        status,
        TaskLifecycleStatus::Completed,
        "results={:?}; events={:?}",
        store.load_download_snapshot(&fixture.work.task.task_id),
        store.list_task_events(&fixture.work.task.task_id)
    );
    drop(store);
    assert_each_range_requested_once(fixture);
    load_proof(fixture).expect("the immutable proof survives success cleanup")
}

fn assert_visible_refusal(fixture: &RetainedRoute, phase: &str) -> DownloadFailure {
    let failure = visible_failure(fixture).expect("a visible local failure");
    assert_eq!(failure.code, REFUSED, "{}", failure.reason_zh);
    assert!(!failure.retryable);
    assert!(
        failure.reason_zh.contains("本地校验未通过")
            && failure.reason_zh.contains("没有发起任何网络请求")
            && failure.reason_zh.contains(phase),
        "{}",
        failure.reason_zh
    );
    failure
}

fn checkpoint_u64(checkpoint: &Value, key: &str) -> u64 {
    checkpoint[key].as_u64().unwrap()
}

fn proof_old_science(proof: &StoredNisarBudgetUpgradeProofV2) -> Value {
    serde_json::from_str(&proof.evidence().science_checkpoint_canonical_json).unwrap()
}

#[test]
fn production_budget_upgrade_reuses_retained_range_through_real_store_and_writer() {
    let fixture = seed_retained_route("budget-upgrade-route");
    let raw_old = raw_result_row(&fixture.database, &fixture.work).unwrap();
    assert_eq!(
        serde_json::from_str::<DownloadItemResult>(&raw_old.2).unwrap(),
        fixture.old_result
    );
    let count = Arc::new(AtomicUsize::new(0));
    let status = run_retained_route(&fixture, None, count.clone());
    let proof = assert_completed_with_single_proof(&fixture, status);
    assert_eq!(
        Some(proof.evidence().science_checkpoint_canonical_json.clone()),
        fixture.checkpoint
    );
    assert_eq!(
        proof.context().previous_result,
        raw_row_as_evidence(raw_old)
    );
    assert_eq!(
        proof.context().previous_progress,
        StoreNisarBudgetUpgradeDownloadProgressV2::Absent
    );
    let old = proof_old_science(&proof);
    assert_eq!(old["completed_ranges"].as_array().unwrap().len(), 1);
    assert!(count.load(Ordering::SeqCst) >= 2);
    assert!(load_refusals(&fixture).is_empty());
    assert_single_upgrade_event(&fixture, &proof);
    // A quiescent predecessor needed no reconciliation: nothing to retain.
    assert_eq!(proof.evidence().pre_reconciliation, None);
}

/// Review P2: a committed upgrade is visible in task history exactly once.
fn assert_single_upgrade_event(fixture: &RetainedRoute, proof: &StoredNisarBudgetUpgradeProofV2) {
    let events = TaskStore::open_existing(&fixture.database)
        .unwrap()
        .list_task_events(&fixture.work.task.task_id)
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "nisar_budget_upgrade")
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(
        events[0].item_id.as_deref(),
        Some(fixture.work.item.item_id.as_str())
    );
    assert_eq!(
        events[0].payload["proof_sha256"],
        serde_json::json!(proof.proof_sha256())
    );
}

/// Decision 2026-10-17 (a): a progress row left by an earlier process is
/// retained exactly in the proof instead of refusing the upgrade.
#[test]
fn production_budget_upgrade_retains_a_stale_progress_row_and_completes() {
    let fixture = seed_retained_route("budget-upgrade-stale-progress");
    let mut store = TaskStore::open_existing(&fixture.database).unwrap();
    store
        .save_scheduler_download_progress(&DownloadItemProgress {
            task_id: fixture.work.task.task_id.clone(),
            item_id: fixture.work.item.item_id.clone(),
            status: insar_core::DownloadItemStatus::Active,
            bytes_completed: 1,
            total_bytes: None,
            bytes_per_second: 0.5,
            attempt: 1,
            updated_at: "2026-10-16T00:00:03Z".into(),
        })
        .unwrap();
    let progress_before = store
        .load_download_snapshot(&fixture.work.task.task_id)
        .unwrap()
        .progress;
    assert_eq!(progress_before.len(), 1);
    drop(store);
    let count = Arc::new(AtomicUsize::new(0));
    let status = run_retained_route(&fixture, None, count);
    let proof = assert_completed_with_single_proof(&fixture, status);
    let StoreNisarBudgetUpgradeDownloadProgressV2::Present {
        progress_json,
        bytes_completed,
        ..
    } = &proof.context().previous_progress
    else {
        panic!("the stale progress row must be retained in the proof");
    };
    assert_eq!(
        serde_json::from_str::<DownloadItemProgress>(progress_json).unwrap(),
        progress_before[0]
    );
    assert_eq!(*bytes_completed, 1);
    assert_eq!(
        proof.context().previous_result,
        StoreNisarBudgetUpgradeDownloadResultV2::Absent
    );
    assert!(load_refusals(&fixture).is_empty());
}

#[test]
fn production_budget_upgrade_material_corruption_is_a_visible_terminal_refusal() {
    let fixture = seed_retained_route("budget-upgrade-corrupt");
    let workspace =
        Hdf5RangeWorkspaceV1::open_existing(route_recovery_root(&fixture.work)).unwrap();
    {
        // Same counters and checkpoint; one byte of a verified Range is corrupt.
        let offset = fixture.ranges[0].start();
        let mut part = OpenOptions::new()
            .read(true)
            .write(true)
            .open(workspace.part_path())
            .unwrap();
        let mut byte = [0_u8; 1];
        part.seek(SeekFrom::Start(offset)).unwrap();
        part.read_exact(&mut byte).unwrap();
        part.seek(SeekFrom::Start(offset)).unwrap();
        part.write_all(&[byte[0] ^ 0x5a]).unwrap();
        part.sync_all().unwrap();
    }
    let corrupt_part_sha = sha256_path(&workspace.part_path());
    assert_ne!(corrupt_part_sha, fixture.part_sha);
    let raw_old = raw_result_row(&fixture.database, &fixture.work).unwrap();
    let anchor_before = load_store_anchor(&fixture);
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let count = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        run_retained_route(&fixture, None, count.clone()),
        TaskLifecycleStatus::Failed
    );
    assert_eq!(count.load(Ordering::SeqCst), 1, "no execution resolver");
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    // This small fixture's first science Range lies inside the retained planning
    // metadata material, so the planning-material digest catches it first.
    let first = assert_visible_refusal(&fixture, "planning_validation");
    let refusals = load_refusals(&fixture);
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].failure(), &first);
    assert_eq!(
        refusals[0].previous_result(),
        &raw_row_as_evidence(raw_old.clone())
    );
    assert!(load_proof(&fixture).is_none());
    assert_eq!(load_store_anchor(&fixture), anchor_before);
    assert_eq!(
        workspace.load_science_checkpoint_v2().unwrap(),
        fixture.checkpoint
    );
    assert_eq!(sha256_path(&workspace.part_path()), corrupt_part_sha);
    let events = TaskStore::open_existing(&fixture.database)
        .unwrap()
        .list_task_events(&fixture.work.task.task_id)
        .unwrap();
    let failed = events
        .iter()
        .filter(|event| event.event_type == "mark_failed")
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].error.as_ref().unwrap()["code"], REFUSED);

    // Continuing again is not a silent loop: it shows the same local refusal
    // again, retains the first refusal unchanged and still sends nothing.
    retry_failed_item(&fixture);
    let count = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        run_retained_route(&fixture, None, count.clone()),
        TaskLifecycleStatus::Failed
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    assert_visible_refusal(&fixture, "planning_validation");
    let refusals_again = load_refusals(&fixture);
    assert_eq!(refusals_again.len(), 2);
    assert_eq!(refusals_again[0], refusals[0]);
    let StoreNisarBudgetUpgradeDownloadResultV2::Present { result_json, .. } =
        refusals_again[1].previous_result()
    else {
        panic!("the second refusal retains the first visible refusal row");
    };
    assert_eq!(
        serde_json::from_str::<DownloadItemResult>(result_json)
            .unwrap()
            .failure
            .unwrap()
            .code,
        REFUSED
    );
    assert_eq!(
        raw_result_row(&fixture.database, &fixture.work).unwrap().0,
        "failed"
    );
}

fn crash_then_old_active_startup(label: &str, stage: NisarBudgetMigrationStageForTest) {
    let fixture = seed_retained_route(label);
    let raw_old = raw_result_row(&fixture.database, &fixture.work).unwrap();
    let old_checkpoint: Value = serde_json::from_str(fixture.checkpoint.as_ref().unwrap()).unwrap();
    let old_generation = checkpoint_u64(&old_checkpoint, "anchor_generation");
    let anchor_before = load_store_anchor(&fixture).unwrap();
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let crash_count = Arc::new(AtomicUsize::new(0));
    crash_retained_route(&fixture, stage, &crash_count);
    // Nothing before the crash point resolved an asset or sent a request.
    assert_eq!(crash_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    let runtime = TaskStore::open_existing(&fixture.database)
        .unwrap()
        .load_task_record(&fixture.work.task.task_id)
        .unwrap()
        .unwrap()
        .runtime;
    assert!(runtime.active_item_ids.contains(&fixture.work.item.item_id));
    assert_eq!(
        raw_result_row(&fixture.database, &fixture.work),
        Some(raw_old.clone())
    );
    assert!(load_refusals(&fixture).is_empty());
    let anchor = load_store_anchor(&fixture).unwrap();
    let record = anchor_record(&anchor);
    let local = local_checkpoint(&fixture).unwrap();
    let proof_after_crash = load_proof(&fixture);
    match stage {
        NisarBudgetMigrationStageForTest::BeforeStorePrepare => {
            assert!(proof_after_crash.is_none());
            assert_eq!(anchor, anchor_before);
            assert_eq!(local, old_checkpoint);
        }
        NisarBudgetMigrationStageForTest::AfterStorePrepare
        | NisarBudgetMigrationStageForTest::AfterLocalTarget => {
            let proof = proof_after_crash.as_ref().unwrap();
            assert!(!record["pending_transition"].is_null());
            assert_eq!(
                record["committed_checkpoint_sha256"],
                old_checkpoint["checkpoint_sha256"]
            );
            if stage == NisarBudgetMigrationStageForTest::AfterStorePrepare {
                assert_eq!(local, old_checkpoint);
            } else {
                assert_eq!(
                    local["budget_upgrade_receipt"]["proof_sha256"],
                    proof.proof_sha256()
                );
                assert_eq!(
                    checkpoint_u64(&local, "anchor_generation"),
                    old_generation + 1
                );
            }
        }
        NisarBudgetMigrationStageForTest::AfterCommit => {
            let proof = proof_after_crash.as_ref().unwrap();
            assert!(record["pending_transition"].is_null());
            assert_eq!(
                record["committed_checkpoint_sha256"],
                local["checkpoint_sha256"]
            );
            assert_eq!(
                local["budget_upgrade_receipt"]["proof_sha256"],
                proof.proof_sha256()
            );
            assert_eq!(
                checkpoint_u64(&local, "anchor_generation"),
                old_generation + 1
            );
        }
        NisarBudgetMigrationStageForTest::RefusalRevisionRead => {
            unreachable!("a coordinator refusal boundary is not a worker crash point")
        }
    }
    // Restart on a reopened SQLite Store with the item still Active.
    let count = Arc::new(AtomicUsize::new(0));
    let status = run_retained_route(&fixture, None, count.clone());
    let proof = assert_completed_with_single_proof(&fixture, status);
    assert!(count.load(Ordering::SeqCst) >= 2);
    if let Some(proof_after_crash) = proof_after_crash {
        assert_eq!(
            proof, proof_after_crash,
            "recovery never re-expands the budget"
        );
    }
    assert_eq!(
        proof.context().previous_result,
        raw_row_as_evidence(raw_old)
    );
    assert_eq!(proof_old_science(&proof), old_checkpoint);
    // The event commits atomically with the proof: never lost, never repeated.
    assert_single_upgrade_event(&fixture, &proof);
}

#[test]
fn production_budget_upgrade_crash_before_store_prepare_restarts_cleanly() {
    crash_then_old_active_startup(
        "budget-crash-before-prepare",
        NisarBudgetMigrationStageForTest::BeforeStorePrepare,
    );
}

#[test]
fn production_budget_upgrade_crash_after_pending_anchor_reconciles_once() {
    crash_then_old_active_startup(
        "budget-crash-after-pending",
        NisarBudgetMigrationStageForTest::AfterStorePrepare,
    );
}

#[test]
fn production_budget_upgrade_crash_after_local_target_reconciles_once() {
    crash_then_old_active_startup(
        "budget-crash-after-local",
        NisarBudgetMigrationStageForTest::AfterLocalTarget,
    );
}

#[test]
fn production_budget_upgrade_crash_after_final_cas_reconciles_once() {
    crash_then_old_active_startup(
        "budget-crash-after-commit",
        NisarBudgetMigrationStageForTest::AfterCommit,
    );
}

fn pause_then_resume(label: &str, stage: NisarBudgetMigrationStageForTest) {
    let fixture = seed_retained_route(label);
    let raw_old = raw_result_row(&fixture.database, &fixture.work).unwrap();
    let anchor_before = load_store_anchor(&fixture);
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let count = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let store = open_shared(&fixture);
    let scheduler = start_route_scheduler(
        &store,
        route_worker_for(&fixture, &count),
        Some(pause_at(stage, &store, &fixture)),
        None,
        Arc::clone(&finished),
    );
    let item = fixture.work.item.item_id.clone();
    wait_for_runtime(&store, &fixture, "paused item", |runtime| {
        runtime.paused_item_ids.contains(&item) && finished.load(Ordering::SeqCst) >= 1
    });
    // Let the coordinator apply the finished worker before inspecting.
    thread::sleep(Duration::from_millis(200));
    {
        let store = store.lock().unwrap();
        let runtime = store
            .load_task_record(&fixture.work.task.task_id)
            .unwrap()
            .unwrap()
            .runtime;
        assert!(runtime.paused_item_ids.contains(&item));
        assert!(!runtime.failed_item_ids.contains(&item));
        assert!(store
            .list_nisar_budget_upgrade_refusals_v2(&fixture.work.task.task_id, &item)
            .unwrap()
            .is_empty());
    }
    assert_eq!(count.load(Ordering::SeqCst), 1, "no resolver before resume");
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    assert_eq!(
        raw_result_row(&fixture.database, &fixture.work),
        Some(raw_old.clone()),
        "a lifecycle stop during migration replaces nothing"
    );
    let proof_after_pause = load_proof(&fixture);
    match stage {
        NisarBudgetMigrationStageForTest::BeforeStorePrepare => {
            assert!(
                proof_after_pause.is_none(),
                "pause before the first CAS refuses"
            );
            assert_eq!(load_store_anchor(&fixture), anchor_before);
            assert_eq!(
                local_checkpoint(&fixture),
                fixture
                    .checkpoint
                    .as_ref()
                    .map(|text| serde_json::from_str::<Value>(text).unwrap())
            );
        }
        _ => {
            let proof = proof_after_pause.as_ref().expect("authorized receipt");
            let local = local_checkpoint(&fixture).unwrap();
            assert_eq!(
                local["budget_upgrade_receipt"]["proof_sha256"],
                proof.proof_sha256()
            );
        }
    }
    item_command(
        &mut store.lock().unwrap(),
        &fixture.work.task.task_id,
        &item,
        TaskStateCommandKind::ResumeItems,
        "resume",
    );
    let runtime = wait_for_runtime(&store, &fixture, "terminal after resume", terminal);
    scheduler.shutdown();
    drop(store);
    let proof = assert_completed_with_single_proof(&fixture, runtime.status);
    if let Some(proof_after_pause) = proof_after_pause {
        assert_eq!(
            proof, proof_after_pause,
            "the second Resume reuses the receipt"
        );
    }
    assert_eq!(
        proof.context().previous_result,
        raw_row_as_evidence(raw_old)
    );
}

#[test]
fn production_budget_upgrade_pause_before_first_cas_keeps_old_state_then_resumes() {
    pause_then_resume(
        "budget-pause-before-cas",
        NisarBudgetMigrationStageForTest::BeforeStorePrepare,
    );
}

#[test]
fn production_budget_upgrade_pause_after_commit_reuses_receipt_on_second_resume() {
    pause_then_resume(
        "budget-pause-after-commit",
        NisarBudgetMigrationStageForTest::AfterCommit,
    );
}

#[test]
fn production_budget_upgrade_second_resume_after_partial_upgraded_progress_keeps_one_proof() {
    let fixture = seed_retained_route("budget-second-resume");
    assert!(
        fixture.ranges.len() >= 3,
        "fixture needs a Range after the first upgraded commit, has {}",
        fixture.ranges.len()
    );
    let count = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let store = open_shared(&fixture);
    let task_id = fixture.work.task.task_id.clone();
    let item = fixture.work.item.item_id.clone();
    let hook_store = Arc::clone(&store);
    let (hook_task, hook_item) = (task_id.clone(), item.clone());
    let fired = Arc::new(AtomicBool::new(false));
    let worker = route_worker_for(&fixture, &count)
        .with_nisar_subset_v2_after_science_commit_hook_for_test(Arc::new(
            move |index, control| {
                if index == 1 && !fired.swap(true, Ordering::SeqCst) {
                    item_command(
                        &mut hook_store.lock().unwrap(),
                        &hook_task,
                        &hook_item,
                        TaskStateCommandKind::PauseItems,
                        "pause-science",
                    );
                    control.set_paused_for_test(true);
                }
            },
        ));
    let scheduler = start_route_scheduler(&store, worker, None, None, Arc::clone(&finished));
    wait_for_runtime(
        &store,
        &fixture,
        "paused after an upgraded Range",
        |runtime| runtime.paused_item_ids.contains(&item) && finished.load(Ordering::SeqCst) >= 1,
    );
    thread::sleep(Duration::from_millis(200));
    let first_proof = load_proof(&fixture).expect("upgrade committed before science");
    let local = local_checkpoint(&fixture).unwrap();
    assert_eq!(
        local["budget_upgrade_receipt"]["proof_sha256"],
        first_proof.proof_sha256()
    );
    assert_eq!(local["completed_ranges"].as_array().unwrap().len(), 2);
    item_command(
        &mut store.lock().unwrap(),
        &task_id,
        &item,
        TaskStateCommandKind::ResumeItems,
        "resume",
    );
    let runtime = wait_for_runtime(&store, &fixture, "terminal after resume", terminal);
    scheduler.shutdown();
    drop(store);
    let proof = assert_completed_with_single_proof(&fixture, runtime.status);
    assert_eq!(proof, first_proof, "a second Resume never expands again");
}

/// A concurrent Store write between context capture and the transaction is a
/// refused admission: terminal and visible, and a later continue succeeds.
#[test]
fn production_budget_upgrade_store_race_is_a_visible_refusal_and_retry_completes() {
    let fixture = seed_retained_route("budget-store-race");
    let anchor_before = load_store_anchor(&fixture);
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let count = Arc::new(AtomicUsize::new(0));
    let store = open_shared(&fixture);
    let hook_store = Arc::clone(&store);
    let (task_id, item_id) = (
        fixture.work.task.task_id.clone(),
        fixture.work.item.item_id.clone(),
    );
    let fired = Arc::new(AtomicBool::new(false));
    let race: NisarBudgetMigrationHookForTest = Arc::new(move |stage, _| {
        if stage == NisarBudgetMigrationStageForTest::BeforeStorePrepare
            && !fired.swap(true, Ordering::SeqCst)
        {
            hook_store
                .lock()
                .unwrap()
                .save_scheduler_download_progress(&DownloadItemProgress {
                    task_id: task_id.clone(),
                    item_id: item_id.clone(),
                    status: insar_core::DownloadItemStatus::Active,
                    bytes_completed: 2,
                    total_bytes: None,
                    bytes_per_second: 0.0,
                    attempt: 1,
                    updated_at: "2026-10-16T00:00:04Z".into(),
                })
                .unwrap();
        }
    });
    let scheduler = start_route_scheduler(
        &store,
        route_worker_for(&fixture, &count),
        Some(race),
        None,
        Arc::new(AtomicUsize::new(0)),
    );
    let runtime = wait_for_runtime(&store, &fixture, "refused admission", terminal);
    scheduler.shutdown();
    drop(store);
    assert_eq!(runtime.status, TaskLifecycleStatus::Failed);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    assert_visible_refusal(&fixture, "store_admission");
    let refusals = load_refusals(&fixture);
    assert_eq!(refusals.len(), 1);
    assert!(matches!(
        refusals[0].previous_progress(),
        StoreNisarBudgetUpgradeDownloadProgressV2::Present {
            bytes_completed: 2,
            ..
        }
    ));
    assert!(load_proof(&fixture).is_none());
    assert_eq!(load_store_anchor(&fixture), anchor_before);
    retry_failed_item(&fixture);
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let proof = assert_completed_with_single_proof(&fixture, status);
    // The upgrade proof retains the visible refusal row it supersedes.
    let StoreNisarBudgetUpgradeDownloadResultV2::Present { result_json, .. } =
        &proof.context().previous_result
    else {
        panic!("the superseded refusal row must be retained");
    };
    assert_eq!(
        serde_json::from_str::<DownloadItemResult>(result_json)
            .unwrap()
            .failure
            .unwrap()
            .code,
        REFUSED
    );
    assert_eq!(
        proof.context().previous_progress,
        StoreNisarBudgetUpgradeDownloadProgressV2::Absent
    );
}

fn reserved_requests(local: &Value) -> u64 {
    checkpoint_u64(local, "reserved_science_request_count")
}

fn advanced(
    changes: impl Fn(LegacyCeilings, &Value) -> Vec<LegacyScienceChangeV2ForTest> + 'static,
    leave_pending: bool,
) -> LegacySeed {
    LegacySeed::Advanced(Box::new(changes), leave_pending)
}

/// Decision 2026-10-17 (b): an attempt whose complete response was recorded
/// before a crash is charged as failed by the executor's own reconciliation;
/// its Range is then fetched once more under the upgraded budget.
#[test]
fn production_budget_upgrade_reconciles_a_response_recorded_legacy_attempt() {
    let fixture = seed_route(
        "budget-response-recorded",
        advanced(
            |_, _| vec![LegacyScienceChangeV2ForTest::ResponseRecordedAttempt(2)],
            false,
        ),
    );
    let local: Value = serde_json::from_str(fixture.checkpoint.as_ref().unwrap()).unwrap();
    assert_eq!(local["pending_attempt"]["state"], "ResponseRecorded");
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let proof = assert_completed_with_single_proof(&fixture, status);
    let old = proof_old_science(&proof);
    assert!(old["pending_attempt"].is_null());
    assert_eq!(
        checkpoint_u64(&old, "failed_science_request_count"),
        checkpoint_u64(&local, "failed_science_request_count") + 2
    );
    assert_eq!(
        checkpoint_u64(&old, "indeterminate_science_request_count"),
        0
    );
    assert_eq!(
        reserved_requests(&old),
        reserved_requests(&local),
        "the recorded requests stay charged"
    );
    assert_eq!(
        checkpoint_u64(&old, "anchor_generation"),
        checkpoint_u64(&local, "anchor_generation") + 1
    );
}

/// Owner decision 2026-10-17 (option B, replacing decision A): a Reserved
/// attempt a killed process left is charged as indeterminate by the executor's
/// own reconciliation (rule (b)) and the one-time upgrade commits. The resume
/// then fetches only the remaining Ranges (each Range once overall), and the
/// writer accepts the charge as an accounting fact because every completed
/// Range passes its response digest and material revalidation; the artifact
/// manifest records the charge bounds. Under decision A this item was refused
/// before any request (stage=science_recovery_precheck) and could never be
/// written; this test replaces that characterization.
#[test]
fn production_budget_upgrade_writes_after_a_hard_crash_reserved_attempt_with_charge_bounds() {
    let fixture = seed_route(
        "budget-pending-attempt",
        advanced(
            |_, _| vec![LegacyScienceChangeV2ForTest::ReservedAttempt(None)],
            false,
        ),
    );
    let local: Value = serde_json::from_str(fixture.checkpoint.as_ref().unwrap()).unwrap();
    assert_eq!(local["pending_attempt"]["state"], "Reserved");
    let pending_bytes = checkpoint_u64(&local["pending_attempt"], "reserved_response_body_bytes");
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let proof = assert_completed_with_single_proof(&fixture, status);
    assert!(
        load_refusals(&fixture).is_empty(),
        "the upgrade itself was not refused"
    );
    let old = proof_old_science(&proof);
    assert!(old["pending_attempt"].is_null());
    assert_eq!(
        checkpoint_u64(&old, "indeterminate_science_request_count"),
        1
    );
    assert_eq!(
        checkpoint_u64(&old, "indeterminate_science_response_body_bytes"),
        pending_bytes
    );
    assert_eq!(
        reserved_requests(&old),
        reserved_requests(&local),
        "the pending reservation is reclassified, never refunded"
    );
    let manifests = fs::read_dir(&fixture.work.task.output_root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".artifact-manifest-v2.json"))
        })
        .collect::<Vec<_>>();
    assert_eq!(manifests.len(), 1, "{manifests:?}");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(&manifests[0]).unwrap()).unwrap();
    let range_bytes = fixture
        .ranges
        .iter()
        .map(|range| range.end_exclusive() - range.start())
        .sum::<u64>();
    let range_count = fixture.ranges.len() as u64;
    assert_eq!(
        manifest["indeterminate_science_charge"],
        serde_json::json!({
            "indeterminate_science_request_count": 1,
            "indeterminate_science_response_body_bytes": pending_bytes,
            "science_request_count_lower_bound": range_count,
            "science_request_count_upper_bound": range_count + 1,
            "science_response_body_bytes_lower_bound": range_bytes,
            "science_response_body_bytes_upper_bound": range_bytes + pending_bytes,
        })
    );
}

/// Decision 2026-10-17 (c): a legacy transition prepared but never committed
/// is committed by the existing recovery rule before the upgrade.
#[test]
fn production_budget_upgrade_commits_a_legacy_pending_transition_then_upgrades() {
    let fixture = seed_route(
        "budget-pending-transition",
        advanced(
            |_, _| vec![LegacyScienceChangeV2ForTest::ChargeFailedRequests(1)],
            true,
        ),
    );
    let predecessor: Value = serde_json::from_str(fixture.checkpoint.as_ref().unwrap()).unwrap();
    let pending_anchor_before = load_store_anchor(&fixture).unwrap().canonical_json;
    let record = anchor_record(&load_store_anchor(&fixture).unwrap());
    assert!(!record["pending_transition"].is_null());
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let proof = assert_completed_with_single_proof(&fixture, status);
    let old = proof_old_science(&proof);
    assert_eq!(
        checkpoint_u64(&old, "failed_science_request_count"),
        checkpoint_u64(&predecessor, "failed_science_request_count") + 1
    );
    // Review P2: the exact raw pending state reconciliation settled is retained.
    let pre = proof
        .evidence()
        .pre_reconciliation
        .as_ref()
        .expect("pending legacy state must be retained");
    assert_eq!(pre.science_anchor_canonical_json, pending_anchor_before);
    assert_eq!(
        pre.local_checkpoint_canonical_json.as_deref(),
        fixture.checkpoint.as_deref()
    );
    assert_eq!(
        checkpoint_u64(&old, "anchor_generation"),
        checkpoint_u64(&predecessor, "anchor_generation") + 1
    );
}

/// Decision 2026-10-17 (c): a pending genesis without any local checkpoint is
/// committed by the existing genesis recovery and then upgraded.
#[test]
fn production_budget_upgrade_recovers_a_legacy_pending_genesis_without_local_checkpoint() {
    let fixture = seed_route("budget-pending-genesis", LegacySeed::PendingGenesis);
    assert!(fixture.checkpoint.is_none());
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let proof = assert_completed_with_single_proof(&fixture, status);
    let old = proof_old_science(&proof);
    assert_eq!(checkpoint_u64(&old, "anchor_generation"), 0);
    assert!(old["completed_ranges"].as_array().unwrap().is_empty());
    assert_eq!(reserved_requests(&old), 0);
}

/// A known legacy overrun within one legacy fetch is budget exhaustion, not
/// corruption: every charge is retained and only missing Ranges are budgeted.
#[test]
fn production_budget_upgrade_admits_bounded_legacy_overrun_as_budget_exhaustion() {
    let fixture = seed_route(
        "budget-overrun",
        advanced(
            |ceilings, local| {
                vec![LegacyScienceChangeV2ForTest::ChargeFailedRequests(
                    ceilings.science_requests + 1 - reserved_requests(local),
                )]
            },
            false,
        ),
    );
    let local: Value = serde_json::from_str(fixture.checkpoint.as_ref().unwrap()).unwrap();
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let proof = assert_completed_with_single_proof(&fixture, status);
    let evidence = proof.evidence();
    assert_eq!(
        reserved_requests(&local),
        fixture.v1_science_limit + 1,
        "the predecessor exceeds its frozen V1 ceiling"
    );
    assert_eq!(
        evidence.old_limits.max_science_request_count,
        fixture.v1_science_limit
    );
    let remaining =
        (fixture.ranges.len() - local["completed_ranges"].as_array().unwrap().len()) as u64;
    assert_eq!(
        evidence.new_limits.max_science_request_count,
        fixture.v1_science_limit + 1 + 44 * remaining
    );
    assert_eq!(proof_old_science(&proof), local);
}

/// "已超出旧上限 + pending": a legacy overrun inside one legacy fetch, with the
/// overrunning transition still pending and a recorded response attempt, is
/// reconciled under the narrow predecessor budget (charges kept), then upgraded.
#[test]
fn production_budget_upgrade_reconciles_overrun_plus_pending_within_one_legacy_fetch() {
    let fixture = seed_route(
        "budget-overrun-pending",
        advanced(
            |ceilings, local| {
                vec![
                    LegacyScienceChangeV2ForTest::ChargeFailedRequests(
                        ceilings.science_requests + 1 - reserved_requests(local) - 2,
                    ),
                    LegacyScienceChangeV2ForTest::ResponseRecordedAttempt(2),
                ]
            },
            true,
        ),
    );
    let predecessor: Value = serde_json::from_str(fixture.checkpoint.as_ref().unwrap()).unwrap();
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let proof = assert_completed_with_single_proof(&fixture, status);
    let old = proof_old_science(&proof);
    assert!(old["pending_attempt"].is_null());
    assert_eq!(reserved_requests(&old), fixture.v1_science_limit + 1);
    assert!(reserved_requests(&old) > proof.evidence().old_limits.max_science_request_count);
    assert_eq!(
        checkpoint_u64(&old, "indeterminate_science_request_count"),
        0
    );
    assert_eq!(
        checkpoint_u64(&old, "failed_science_request_count"),
        checkpoint_u64(&predecessor, "failed_science_request_count")
            + (fixture.v1_science_limit + 1 - reserved_requests(&predecessor))
    );
}

/// Same counters, corrupted evidence: a committed checkpoint whose completed
/// Range digest disagrees with the retained part bytes is corruption.
#[test]
fn production_budget_upgrade_refuses_a_corrupted_completed_range_digest_before_io() {
    let fixture = seed_route(
        "budget-corrupt-digest",
        advanced(
            |_, _| vec![LegacyScienceChangeV2ForTest::CorruptFirstCompletedDigest],
            false,
        ),
    );
    assert_refused_without_io(&fixture, "predecessor_validation");
}

fn assert_refused_without_io(fixture: &RetainedRoute, phase: &str) {
    let raw_old = raw_result_row(&fixture.database, &fixture.work).unwrap();
    let anchor_before = load_store_anchor(fixture);
    let local_before = local_checkpoint(fixture);
    let part_before = sha256_path(
        &Hdf5RangeWorkspaceV1::open_existing(route_recovery_root(&fixture.work))
            .unwrap()
            .part_path(),
    );
    let requests_before = fixture.server.requests.lock().unwrap().len();
    for attempt in 1..=2 {
        if attempt == 2 {
            retry_failed_item(fixture);
        }
        let count = Arc::new(AtomicUsize::new(0));
        assert_eq!(
            run_retained_route(fixture, None, count.clone()),
            TaskLifecycleStatus::Failed
        );
        assert_eq!(count.load(Ordering::SeqCst), 1, "no execution resolver");
        assert_eq!(
            fixture.server.requests.lock().unwrap().len(),
            requests_before
        );
        assert_visible_refusal(fixture, phase);
        assert_eq!(load_refusals(fixture).len(), attempt);
        assert!(load_proof(fixture).is_none());
        assert_eq!(load_store_anchor(fixture), anchor_before);
        assert_eq!(local_checkpoint(fixture), local_before);
        assert_eq!(
            sha256_path(
                &Hdf5RangeWorkspaceV1::open_existing(route_recovery_root(&fixture.work))
                    .unwrap()
                    .part_path()
            ),
            part_before
        );
    }
    assert_eq!(
        load_refusals(fixture)[0].previous_result(),
        &raw_row_as_evidence(raw_old)
    );
}

/// Reconciliation failure is terminal and visible, never a silent loop.
#[test]
fn production_budget_upgrade_refuses_excessive_overrun_with_pending_at_reconciliation() {
    let fixture = seed_route(
        "budget-overrun-pending-excess",
        advanced(
            |ceilings, local| {
                vec![
                    LegacyScienceChangeV2ForTest::ChargeFailedRequests(
                        ceilings.science_requests + 21 - reserved_requests(local),
                    ),
                    LegacyScienceChangeV2ForTest::ReservedAttempt(None),
                ]
            },
            true,
        ),
    );
    assert_refused_without_io(&fixture, "legacy_reconciliation");
}

#[test]
fn production_budget_upgrade_refuses_excessive_quiescent_overrun_before_io() {
    let fixture = seed_route(
        "budget-overrun-excess",
        advanced(
            |ceilings, local| {
                vec![LegacyScienceChangeV2ForTest::ChargeFailedRequests(
                    ceilings.science_requests + 22 - reserved_requests(local),
                )]
            },
            false,
        ),
    );
    assert_refused_without_io(&fixture, "predecessor_validation");
}

#[test]
fn production_budget_upgrade_refuses_legacy_body_overrun_before_io() {
    let fixture = seed_route(
        "budget-body-overrun",
        advanced(
            |ceilings, local| {
                let charged = checkpoint_u64(local, "committed_planning_response_body_bytes")
                    + checkpoint_u64(local, "reserved_science_response_body_bytes");
                vec![LegacyScienceChangeV2ForTest::ChargeFailedBodyBytes(
                    ceilings.response_body_bytes + 1 - charged,
                )]
            },
            false,
        ),
    );
    assert_refused_without_io(&fixture, "predecessor_validation");
}

/// A retained route whose legacy overrun is refused at predecessor validation
/// before any request (see the excessive quiescent overrun test).
fn seed_refused_overrun_route(label: &str) -> RetainedRoute {
    seed_route(
        label,
        advanced(
            |ceilings, local| {
                vec![LegacyScienceChangeV2ForTest::ChargeFailedRequests(
                    ceilings.science_requests + 22 - reserved_requests(local),
                )]
            },
            false,
        ),
    )
}

/// Review 2026-10-17 round 2, P2-1: a task-level pause that lands after the
/// scheduler read the runtime revision for a refused migration, but before
/// its Store refusal transaction, made that transaction conflict; the fallback
/// then overwrote the retained result row with the refusal and marked the item
/// failed despite the pause. The transaction is now retried once at the new
/// revision: the pause wins, no row is replaced and no evidence is claimed.
#[test]
fn production_budget_upgrade_refusal_racing_a_task_pause_keeps_the_pause_and_the_result_row() {
    let fixture = seed_refused_overrun_route("budget-refusal-pause-race");
    let raw_old = raw_result_row(&fixture.database, &fixture.work).unwrap();
    let item = fixture.work.item.item_id.clone();
    let database = fixture.database.clone();
    let task_id = fixture.work.task.task_id.clone();
    let fired = Arc::new(AtomicBool::new(false));
    let pause: NisarBudgetMigrationHookForTest = Arc::new(move |stage, _| {
        if stage == NisarBudgetMigrationStageForTest::RefusalRevisionRead
            && !fired.swap(true, Ordering::SeqCst)
        {
            // The application's pause command uses its own Store connection.
            let mut other = TaskStore::open_existing(&database).unwrap();
            let revision = other
                .load_task_record(&task_id)
                .unwrap()
                .unwrap()
                .runtime
                .revision;
            other
                .update_task(&StoreUpdateTaskRequest {
                    task_id: task_id.clone(),
                    expected_revision: revision,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::PauseTask,
                        item_ids: vec![],
                    },
                    event_id: "concurrent-task-pause".into(),
                    occurred_at: "2026-10-16T00:00:06Z".into(),
                })
                .unwrap();
        }
    });
    let store = open_shared(&fixture);
    let scheduler = start_route_scheduler(
        &store,
        route_worker_for(&fixture, &Arc::new(AtomicUsize::new(0))),
        Some(pause),
        None,
        Arc::new(AtomicUsize::new(0)),
    );
    let runtime = wait_for_runtime(&store, &fixture, "a settled refusal", |runtime| {
        runtime.paused_item_ids.contains(&item) || runtime.failed_item_ids.contains(&item)
    });
    scheduler.shutdown();
    drop(store);
    assert!(runtime.task_paused, "{runtime:?}");
    assert!(runtime.paused_item_ids.contains(&item), "{runtime:?}");
    assert!(!runtime.failed_item_ids.contains(&item), "{runtime:?}");
    assert_eq!(
        raw_result_row(&fixture.database, &fixture.work),
        Some(raw_old),
        "the retained result row is not overwritten"
    );
    assert!(
        load_refusals(&fixture).is_empty(),
        "a pause replaces nothing"
    );
    assert!(load_proof(&fixture).is_none());
}

/// Review 2026-10-17 round 2, P2-1: when the refusal evidence cannot be
/// written (here an injected abort on the evidence insert), the fallback no
/// longer overwrites the retained result row it could not copy. The item still
/// ends failed, so it is not dispatched again in a loop, and the local
/// diagnostic is recorded on its task event.
#[test]
fn production_budget_upgrade_unretained_refusal_keeps_the_result_row_and_records_the_diagnostic() {
    let fixture = seed_refused_overrun_route("budget-refusal-unretained");
    let raw_old = raw_result_row(&fixture.database, &fixture.work).unwrap();
    rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER inject_refusal_evidence_failure \
             BEFORE INSERT ON nisar_budget_upgrade_refusal_v2 \
             BEGIN SELECT RAISE(ABORT, 'injected refusal evidence failure'); END;",
        )
        .unwrap();
    let requests_before = fixture.server.requests.lock().unwrap().len();
    assert_eq!(
        run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0))),
        TaskLifecycleStatus::Failed
    );
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    assert_eq!(
        raw_result_row(&fixture.database, &fixture.work),
        Some(raw_old),
        "the row the evidence should have retained is not overwritten"
    );
    assert!(load_refusals(&fixture).is_empty());
    let events = TaskStore::open_existing(&fixture.database)
        .unwrap()
        .list_task_events(&fixture.work.task.task_id)
        .unwrap();
    let event = events.last().unwrap();
    assert_eq!(event.event_type, "mark_failed");
    let failure: DownloadFailure = serde_json::from_value(event.error.clone().unwrap()).unwrap();
    assert_eq!(failure.code, REFUSED);
    assert!(!failure.retryable);
    for expected in [
        "predecessor_validation",
        "原下载记录未被覆盖",
        "store_error=V3_STORE_DATABASE_ERROR",
    ] {
        assert!(
            failure.reason_zh.contains(expected),
            "{}",
            failure.reason_zh
        );
    }
    // Review 2026-10-17 round 3, P2-3: the card codes and the task page read
    // that event as the item's current failure, not the retained old row, so
    // the card asks for no login check and the detail shows the local cause.
    let store = TaskStore::open_existing(&fixture.database).unwrap();
    let summary = store
        .core_restore(false)
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| task.task_id == fixture.work.task.task_id)
        .unwrap();
    assert_eq!(summary.task_type.as_deref(), Some("nisar_subset"));
    assert_eq!(
        summary
            .failure_codes
            .iter()
            .map(|code| code.code.as_str())
            .collect::<Vec<_>>(),
        vec![REFUSED]
    );
    let page = store
        .load_task_page(&insar_core::StoreTaskPageRequest {
            task_id: fixture.work.task.task_id.clone(),
            expected_revision: 0,
            page_index: 0,
            page_size: 10,
        })
        .unwrap();
    assert_eq!(page.items[0].failure.as_ref(), Some(&failure));
}

/// Review 2026-10-17 round 4 (Issue #8 slice 6): when the refusal evidence
/// and its unretained fallback both fail, nothing records the refusal, the
/// item stays Active, and the scheduler used to dispatch the same migration
/// again at once, in a loop. It now waits 1 s, doubling, before the next
/// dispatch; once the Store accepts writes again the refusal is recorded.
#[test]
fn production_budget_upgrade_double_store_failure_redispatches_with_backoff() {
    let fixture = seed_refused_overrun_route("budget-refusal-double-fault");
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER inject_refusal_evidence_failure \
             BEFORE INSERT ON nisar_budget_upgrade_refusal_v2 \
             BEGIN SELECT RAISE(ABORT, 'injected refusal evidence failure'); END; \
             CREATE TRIGGER inject_mark_failed_failure \
             BEFORE INSERT ON task_events WHEN NEW.event_type = 'mark_failed' \
             BEGIN SELECT RAISE(ABORT, 'injected mark-failed failure'); END;",
        )
        .unwrap();
    let store = open_shared(&fixture);
    let finished = Arc::new(AtomicUsize::new(0));
    let scheduler = start_route_scheduler(
        &store,
        route_worker_for(&fixture, &Arc::new(AtomicUsize::new(0))),
        None,
        None,
        Arc::clone(&finished),
    );
    let window = Instant::now();
    while finished.load(Ordering::SeqCst) == 0 {
        assert!(window.elapsed() < Duration::from_secs(60), "first dispatch");
        wait_for_route_progress();
    }
    let first_finish = Instant::now();
    thread::sleep(Duration::from_millis(2_500));
    let dispatches = finished.load(Ordering::SeqCst);
    let elapsed = first_finish.elapsed();
    // 1 s, then 2 s: at most two more dispatches fit the observed window.
    assert!(
        dispatches <= 3,
        "{dispatches} dispatches in {elapsed:?}: the unrecorded refusal must not loop"
    );
    let runtime = store
        .lock()
        .unwrap()
        .load_task_record(&fixture.work.task.task_id)
        .unwrap()
        .unwrap()
        .runtime;
    assert!(
        runtime.active_item_ids.contains(&fixture.work.item.item_id),
        "nothing was recorded: {runtime:?}"
    );
    connection
        .execute_batch(
            "DROP TRIGGER inject_refusal_evidence_failure; \
             DROP TRIGGER inject_mark_failed_failure;",
        )
        .unwrap();
    let item = fixture.work.item.item_id.clone();
    let runtime = wait_for_runtime(&store, &fixture, "the recorded refusal", |runtime| {
        runtime.failed_item_ids.contains(&item)
    });
    scheduler.shutdown();
    assert!(runtime.failed_item_ids.contains(&item));
    assert_eq!(
        load_refusals(&fixture).len(),
        1,
        "the evidence is retained once"
    );
}

fn tamper_proof_row(fixture: &RetainedRoute, statement: &str, values: &[&str]) {
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER nisar_budget_upgrade_is_immutable; \
             DROP TRIGGER nisar_budget_upgrade_cannot_be_deleted;",
        )
        .unwrap();
    connection
        .execute(statement, rusqlite::params_from_iter(values.iter()))
        .unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER nisar_budget_upgrade_is_immutable \
             BEFORE UPDATE ON nisar_budget_upgrade_v2 \
             BEGIN SELECT RAISE(ABORT, 'NISAR budget upgrade proof is immutable'); END; \
             CREATE TRIGGER nisar_budget_upgrade_cannot_be_deleted \
             BEFORE DELETE ON nisar_budget_upgrade_v2 \
             BEGIN SELECT RAISE(ABORT, 'NISAR budget upgrade proof must be retained'); END;",
        )
        .unwrap();
}

fn crash_then_tamper_then_refuse(
    label: &str,
    stage: NisarBudgetMigrationStageForTest,
    tamper: impl FnOnce(&RetainedRoute, &StoredNisarBudgetUpgradeProofV2),
) {
    let fixture = seed_retained_route(label);
    crash_retained_route(&fixture, stage, &Arc::new(AtomicUsize::new(0)));
    let proof = load_proof(&fixture).unwrap();
    tamper(&fixture, &proof);
    let anchor_before = load_store_anchor(&fixture);
    let local_before = local_checkpoint(&fixture);
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let count = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        run_retained_route(&fixture, None, count.clone()),
        TaskLifecycleStatus::Failed
    );
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "no I/O before reconciliation"
    );
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    assert_visible_refusal(&fixture, "proof_reconciliation");
    assert_eq!(load_store_anchor(&fixture), anchor_before);
    assert_eq!(local_checkpoint(&fixture), local_before);
}

#[test]
fn production_budget_upgrade_refuses_a_proof_digest_mismatch() {
    crash_then_tamper_then_refuse(
        "budget-proof-digest",
        NisarBudgetMigrationStageForTest::AfterStorePrepare,
        |fixture, _| {
            tamper_proof_row(
                fixture,
                "UPDATE nisar_budget_upgrade_v2 SET proof_sha256=?1",
                &[&"0".repeat(64)],
            );
        },
    );
}

#[test]
fn production_budget_upgrade_refuses_resealed_proof_limit_tampering() {
    crash_then_tamper_then_refuse(
        "budget-proof-content",
        NisarBudgetMigrationStageForTest::AfterStorePrepare,
        |fixture, proof| {
            let limits = &proof.evidence().new_limits;
            let original = format!(
                "\"new_limits\":{{\"max_request_count\":{},\"max_science_request_count\":{},\"max_response_body_bytes\":{}}}",
                limits.max_request_count,
                limits.max_science_request_count,
                limits.max_response_body_bytes
            );
            let widened = format!(
                "\"new_limits\":{{\"max_request_count\":{},\"max_science_request_count\":{},\"max_response_body_bytes\":{}}}",
                limits.max_request_count + 1000,
                limits.max_science_request_count + 1000,
                limits.max_response_body_bytes
            );
            let canonical = proof.canonical_json().unwrap();
            assert_eq!(canonical.matches(&original).count(), 1);
            let tampered = canonical.replace(&original, &widened);
            let digest = format!("{:x}", Sha256::digest(tampered.as_bytes()));
            tamper_proof_row(
                fixture,
                "UPDATE nisar_budget_upgrade_v2 SET proof_sha256=?1, canonical_json=?2",
                &[&digest, &tampered],
            );
            // The Store accepts a coherently resealed document; only the
            // download-side derivation from the retained evidence rejects it.
            assert!(load_proof(fixture).is_some());
        },
    );
}

#[test]
fn production_budget_upgrade_refuses_a_receipt_without_its_proof() {
    crash_then_tamper_then_refuse(
        "budget-receipt-without-proof",
        NisarBudgetMigrationStageForTest::AfterLocalTarget,
        |fixture, _| {
            tamper_proof_row(fixture, "DELETE FROM nisar_budget_upgrade_v2", &[]);
            assert!(load_proof(fixture).is_none());
        },
    );
}

/// Review boundary: a legacy V1 item whose science never started has no state
/// to inherit. It runs under its original V1 limits and nothing is upgraded.
#[test]
fn production_budget_upgrade_leaves_v1_without_science_state_on_original_limits() {
    let fixture = seed_route("budget-v1-no-science", LegacySeed::PlanningOnly);
    assert!(fixture.checkpoint.is_none());
    assert!(load_store_anchor(&fixture).is_none());
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let store = TaskStore::open_existing(&fixture.database).unwrap();
    assert_eq!(
        status,
        TaskLifecycleStatus::Completed,
        "results={:?}",
        store.load_download_snapshot(&fixture.work.task.task_id)
    );
    drop(store);
    assert_each_range_requested_once(&fixture);
    assert!(
        load_proof(&fixture).is_none(),
        "no upgrade without a predecessor checkpoint"
    );
    assert!(load_refusals(&fixture).is_empty());
}

/// Review boundary: archived recovery never transfers an upgrade receipt. A
/// receipt-free source is admitted; the same source carrying a receipt is
/// refused before any destination checkpoint or part exists.
#[cfg(windows)]
#[test]
fn archived_recovery_refuses_a_receipt_bearing_source_before_destination_writes() {
    for (label, seed, admitted) in [
        ("budget-archive-plain", LegacySeed::Quiescent, true),
        (
            "budget-archive-receipt",
            advanced(
                |_, _| vec![LegacyScienceChangeV2ForTest::AttachUpgradeReceipt],
                false,
            ),
            false,
        ),
    ] {
        let fixture = seed_route(label, seed);
        let source =
            Hdf5RangeWorkspaceV1::open_existing(route_recovery_root(&fixture.work)).unwrap();
        let anchor = load_store_anchor(&fixture).unwrap();
        let mut new_work = fixture.work.clone();
        new_work.task.task_id = "explicit-archived-recovery".to_owned();
        new_work.task.output_root = fixture.case._root.0.join("recovered-output");
        let destination =
            Hdf5RangeWorkspaceV1::new(fixture.case._root.0.join("recovery-workspace")).unwrap();
        let source_part = sha256_path(&source.part_path());
        let prepared = crate::nisar_science_transfer_v2::prepare_archived_nisar_recovery_v2(
            &fixture.work,
            &new_work,
            &anchor.canonical_json,
            &anchor.record_sha256,
            &source,
            &destination,
            &WorkerControl::default(),
        );
        assert_eq!(
            prepared.is_ok(),
            admitted,
            "{label}: {:?}",
            prepared.as_ref().err()
        );
        drop(prepared);
        if !admitted {
            assert!(matches!(
                crate::nisar_science_transfer_v2::prepare_archived_nisar_recovery_v2(
                    &fixture.work,
                    &new_work,
                    &anchor.canonical_json,
                    &anchor.record_sha256,
                    &source,
                    &destination,
                    &WorkerControl::default(),
                ),
                Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
            ));
            assert!(destination.load_science_checkpoint_v2().unwrap().is_none());
            assert!(!destination.part_path().exists());
        }
        assert_eq!(sha256_path(&source.part_path()), source_part);
        assert_eq!(
            source.load_science_checkpoint_v2().unwrap(),
            fixture.checkpoint
        );
    }
}

/// Review P1 (2026-10-17): while this item migrates, another item of the same
/// task writes progress or a result, which advances the task-level download
/// revision (simulated here with the same single counter write). Admission now
/// compares only this item's own rows, so the upgrade is admitted and completes.
#[test]
fn production_budget_upgrade_admits_a_sibling_task_revision_advance_during_migration() {
    let fixture = seed_retained_route("budget-sibling-revision");
    let store = open_shared(&fixture);
    let hook_store = Arc::clone(&store);
    let database = fixture.database.clone();
    let task_id = fixture.work.task.task_id.clone();
    let fired = Arc::new(AtomicBool::new(false));
    let sibling: NisarBudgetMigrationHookForTest = Arc::new(move |stage, _| {
        if stage == NisarBudgetMigrationStageForTest::BeforeStorePrepare
            && !fired.swap(true, Ordering::SeqCst)
        {
            // Hold the coordinator's Store so this write is not interleaved.
            let _coordinator = hook_store.lock().unwrap();
            let connection = rusqlite::Connection::open(&database).unwrap();
            connection.busy_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(
                connection
                    .execute(
                        "UPDATE download_engine_state SET revision=revision+1 WHERE task_id=?1",
                        [&task_id],
                    )
                    .unwrap(),
                1
            );
        }
    });
    let scheduler = start_route_scheduler(
        &store,
        route_worker_for(&fixture, &Arc::new(AtomicUsize::new(0))),
        Some(sibling),
        None,
        Arc::new(AtomicUsize::new(0)),
    );
    let runtime = wait_for_runtime(&store, &fixture, "terminal", terminal);
    scheduler.shutdown();
    drop(store);
    let proof = assert_completed_with_single_proof(&fixture, runtime.status);
    assert!(load_refusals(&fixture).is_empty());
    assert_single_upgrade_event(&fixture, &proof);
}

/// Review P2: a transient local Store failure at admission is not a validation
/// refusal. It keeps the ordinary, retryable failure; a later continue
/// upgrades. Review round 2 (P1-2) limits "transient" to a revision conflict, a
/// busy database and an unavailable coordinator, so the injected failure is now
/// the admission transaction losing its anchor revision race (the trigger skips
/// the anchor row update); an injected SQLite abort is a terminal refusal (see
/// the database-refusal test below).
#[test]
fn production_budget_upgrade_transient_store_failure_is_an_ordinary_retryable_failure() {
    let fixture = seed_retained_route("budget-transient-store");
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER inject_transient_store_failure \
             BEFORE UPDATE ON nisar_subset_execution_anchors_v2 \
             BEGIN SELECT RAISE(IGNORE); END;",
        )
        .unwrap();
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let count = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        run_retained_route(&fixture, None, count.clone()),
        TaskLifecycleStatus::Failed
    );
    assert_eq!(count.load(Ordering::SeqCst), 1, "no execution resolver");
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    let failure = visible_failure(&fixture).unwrap();
    assert_eq!(failure.code, "NISAR_SUBSET_V2_CHECKPOINT_PERSISTENCE");
    assert!(failure.retryable, "{}", failure.reason_zh);
    assert!(
        failure.reason_zh.contains("没有发起任何网络请求")
            && failure.reason_zh.contains("stage=budget_migration_entry"),
        "{}",
        failure.reason_zh
    );
    assert!(load_proof(&fixture).is_none());
    // Still inside the migration guard: the replaced row was retained first.
    assert_eq!(load_refusals(&fixture).len(), 1);
    connection
        .execute_batch("DROP TRIGGER inject_transient_store_failure;")
        .unwrap();
    drop(connection);
    retry_failed_item(&fixture);
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let proof = assert_completed_with_single_proof(&fixture, status);
    assert_single_upgrade_event(&fixture, &proof);
}

/// Review 2026-10-17 round 2, P1-2: a database refusal at admission (here an
/// injected SQLite abort inside the IMMEDIATE transaction) is not transient. It
/// is the terminal local refusal with the Store's own code, never the ordinary
/// "temporarily unavailable, continue later" failure; once the cause is gone a
/// later continue still upgrades.
#[test]
fn production_budget_upgrade_database_refusal_at_admission_is_terminal_with_its_store_code() {
    let fixture = seed_retained_route("budget-store-database-refusal");
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER inject_store_database_refusal BEFORE INSERT ON nisar_budget_upgrade_v2 \
             BEGIN SELECT RAISE(ABORT, 'injected Store database refusal'); END;",
        )
        .unwrap();
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let count = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        run_retained_route(&fixture, None, count.clone()),
        TaskLifecycleStatus::Failed
    );
    assert_eq!(count.load(Ordering::SeqCst), 1, "no execution resolver");
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    let failure = assert_visible_refusal(&fixture, "store_admission");
    assert!(
        failure.reason_zh.contains("V3_STORE_DATABASE_ERROR")
            && !failure.reason_zh.contains("暂时无法访问"),
        "{}",
        failure.reason_zh
    );
    assert!(load_proof(&fixture).is_none());
    assert_eq!(load_refusals(&fixture).len(), 1);
    connection
        .execute_batch("DROP TRIGGER inject_store_database_refusal;")
        .unwrap();
    drop(connection);
    retry_failed_item(&fixture);
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let proof = assert_completed_with_single_proof(&fixture, status);
    assert_single_upgrade_event(&fixture, &proof);
}

/// Review 2026-10-17 round 2, P1-2: a digest-intact proof row of a schema
/// version this program does not support is a terminal local refusal that
/// keeps the Store's own code. Before, it was folded into the transient
/// "store temporarily unavailable, continue later" failure (retryable) and the
/// code was lost. Nothing is sent and the old evidence stays in place.
#[test]
fn production_budget_upgrade_unsupported_proof_version_is_a_terminal_refusal_with_its_store_code() {
    let fixture = seed_retained_route("budget-proof-version-unsupported");
    let canonical = r#"{"contract":"nisar_budget_upgrade_proof_v1","schema_version":3}"#;
    rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "INSERT INTO nisar_budget_upgrade_v2(task_id,item_id,proof_sha256,canonical_json) VALUES (?1,?2,?3,?4)",
            rusqlite::params![
                fixture.work.task.task_id,
                fixture.work.item.item_id,
                format!("{:x}", Sha256::digest(canonical.as_bytes())),
                canonical
            ],
        )
        .unwrap();
    let anchor_before = load_store_anchor(&fixture);
    let local_before = local_checkpoint(&fixture);
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let count = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        run_retained_route(&fixture, None, count.clone()),
        TaskLifecycleStatus::Failed
    );
    assert_eq!(count.load(Ordering::SeqCst), 1, "no execution resolver");
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    let failure = assert_visible_refusal(&fixture, "proof_reconciliation");
    assert!(
        failure
            .reason_zh
            .contains("V3_STORE_NISAR_BUDGET_UPGRADE_PROOF_VERSION_UNSUPPORTED")
            && failure.reason_zh.contains("不是暂时性故障")
            && !failure.reason_zh.contains("暂时无法访问"),
        "{}",
        failure.reason_zh
    );
    assert_eq!(load_refusals(&fixture).len(), 1);
    assert_eq!(load_store_anchor(&fixture), anchor_before);
    assert_eq!(local_checkpoint(&fixture), local_before);
}

/// Rule 5 of 2026-10-17 and review P2: an item that does not declare the legacy
/// V1 policy (here: planning has not started) only checks that no proof exists.
/// A proof row that cannot be read keeps the ordinary recovery code instead of
/// the terminal upgrade refusal, still before any I/O.
#[test]
fn production_budget_upgrade_unreadable_proof_for_a_non_v1_item_keeps_the_ordinary_code() {
    let case = route_fixture("budget-non-v1-proof", NisarSubsetFrequency::A);
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let work = route_ready_work(&case, &intent);
    let database = case._root.0.join("upgrade.sqlite3");
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    resume_scheduler_task(&mut store, &work);
    drop(store);
    let canonical = r#"{"contract":"not_a_budget_upgrade_proof"}"#;
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "INSERT INTO nisar_budget_upgrade_v2(task_id,item_id,proof_sha256,canonical_json) VALUES (?1,?2,?3,?4)",
            rusqlite::params![
                work.task.task_id,
                work.item.item_id,
                format!("{:x}", Sha256::digest(canonical.as_bytes())),
                canonical
            ],
        )
        .unwrap();
    let server = RouteRangeServer::start(
        fs::read(&case.source_path).unwrap(),
        ROUTE_ETAG,
        RouteServerMode::Exact,
    );
    let count = Arc::new(AtomicUsize::new(0));
    let store: SharedTaskStore = Arc::new(Mutex::new(TaskStore::open_existing(&database).unwrap()));
    let scheduler = start_route_scheduler(
        &store,
        route_worker_with_resolve_counter(
            server.url.clone(),
            &case,
            &work.item,
            Some(Arc::clone(&count)),
        ),
        None,
        None,
        Arc::new(AtomicUsize::new(0)),
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let runtime = store
            .lock()
            .unwrap()
            .load_task_record(&work.task.task_id)
            .unwrap()
            .unwrap()
            .runtime;
        if terminal(&runtime) {
            assert_eq!(runtime.status, TaskLifecycleStatus::Failed);
            break;
        }
        assert!(Instant::now() < deadline, "non-V1 item did not finish");
        wait_for_route_progress();
    }
    scheduler.shutdown();
    let results = store
        .lock()
        .unwrap()
        .load_download_snapshot(&work.task.task_id)
        .unwrap()
        .results;
    drop(store);
    let failure = results[0].failure.clone().unwrap();
    assert_eq!(
        failure.code, "NISAR_SUBSET_V2_RECOVERY_INVALID",
        "{}",
        failure.reason_zh
    );
    assert_ne!(failure.code, REFUSED);
    assert_eq!(count.load(Ordering::SeqCst), 1, "no execution resolver");
    assert!(server.finish().is_empty(), "no request before the decision");
}

/// First byte of [start, end) not covered by any sorted, non-overlapping range.
fn first_uncovered_byte(start: u64, end: u64, covered: &[(u64, u64)]) -> Option<u64> {
    let mut cursor = start;
    for (from, to) in covered {
        if *to <= cursor {
            continue;
        }
        if *from > cursor {
            break;
        }
        cursor = *to;
        if cursor >= end {
            return None;
        }
    }
    (cursor < end).then_some(cursor)
}

/// Review gap: corruption of a verified science Range byte that the retained
/// planning material does not cover is caught by the predecessor's own Range
/// digest (predecessor_validation), not by planning validation: a visible
/// refusal before any I/O that preserves everything.
#[test]
fn production_budget_upgrade_science_byte_corruption_outside_planning_is_refused_before_io() {
    // Most of this small fixture's science Ranges lie inside the retained
    // planning material; completing all but the last includes some that do not.
    let fixture = seed_route(
        "budget-science-byte-corrupt",
        LegacySeed::QuiescentAllButLastRange,
    );
    let workspace =
        Hdf5RangeWorkspaceV1::open_existing(route_recovery_root(&fixture.work)).unwrap();
    let planning: Value =
        serde_json::from_str(&workspace.load_planning_checkpoint_v2().unwrap().unwrap()).unwrap();
    let mut material = [
        "metadata_prefix",
        "metadata_discovery",
        "additional_geolocation",
    ]
    .iter()
    .flat_map(|stage| {
        planning["bootstrap_statistics"][stage]["requested_ranges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|range| {
                (
                    range["start"].as_u64().unwrap(),
                    range["end_exclusive"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    })
    .collect::<Vec<_>>();
    material.sort_unstable();
    let local = local_checkpoint(&fixture).unwrap();
    let offset = local["completed_ranges"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|range| {
            first_uncovered_byte(
                range["start"].as_u64().unwrap(),
                range["end_exclusive"].as_u64().unwrap(),
                &material,
            )
        })
        .unwrap_or_else(|| {
            panic!(
                "fixture needs a verified science byte outside the planning material: \
                 material={material:?} completed={:?} ranges={:?}",
                local["completed_ranges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|range| (range["start"].as_u64(), range["end_exclusive"].as_u64()))
                    .collect::<Vec<_>>(),
                fixture
                    .ranges
                    .iter()
                    .map(|range| (range.start(), range.end_exclusive()))
                    .collect::<Vec<_>>()
            )
        });
    let mut part = OpenOptions::new()
        .read(true)
        .write(true)
        .open(workspace.part_path())
        .unwrap();
    let mut byte = [0_u8; 1];
    part.seek(SeekFrom::Start(offset)).unwrap();
    part.read_exact(&mut byte).unwrap();
    part.seek(SeekFrom::Start(offset)).unwrap();
    part.write_all(&[byte[0] ^ 0x5a]).unwrap();
    part.sync_all().unwrap();
    drop(part);
    assert_refused_without_io(&fixture, "predecessor_validation");
}

fn completed_bytes(checkpoint: &Value) -> u64 {
    checkpoint["completed_ranges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|range| range["end_exclusive"].as_u64().unwrap() - range["start"].as_u64().unwrap())
        .sum()
}

fn hashed_bytes(workspace: &str, site: &str) -> u64 {
    crate::nisar_science_transfer_v2::completed_material_hash_ledger::hashed_bytes(workspace, site)
}

/// Review P2 (performance): the first upgraded continuation hashes each
/// completed Range once in the migration entry (deriving the proof) and adds no
/// proof pass to the executor or the writer. Exact pass counts per path:
/// entry 1 x initial; executor recovery 1 + start check 1 (initial) + final
/// check 1 (all); writer science revalidation 1 (all).
#[test]
fn production_budget_upgrade_first_continuation_hashes_completed_material_once_per_path() {
    let fixture = seed_retained_route("budget-hash-first");
    let old: Value = serde_json::from_str(fixture.checkpoint.as_ref().unwrap()).unwrap();
    let workspace = old["workspace_canonical_path"].as_str().unwrap().to_owned();
    let initial = completed_bytes(&old);
    let total: u64 = fixture
        .ranges
        .iter()
        .map(|range| range.len().unwrap())
        .sum();
    assert!(initial > 0 && total > initial);
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    assert_completed_with_single_proof(&fixture, status);
    assert_eq!(hashed_bytes(&workspace, "migration_entry"), initial);
    assert_eq!(hashed_bytes(&workspace, "executor"), 2 * initial + total);
    assert_eq!(
        hashed_bytes(&workspace, "writer_science_revalidation"),
        total
    );
}

/// Review P2 (performance): a continuation of an already-upgraded item hashes
/// its completed Ranges once at the migration entry (before the guard clears)
/// and the proof check adds no pass to the executor or the writer.
#[test]
fn production_budget_upgrade_existing_proof_continuation_hashes_completed_material_once_per_path() {
    let fixture = seed_retained_route("budget-hash-second");
    let workspace = local_checkpoint(&fixture).unwrap()["workspace_canonical_path"]
        .as_str()
        .unwrap()
        .to_owned();
    let total: u64 = fixture
        .ranges
        .iter()
        .map(|range| range.len().unwrap())
        .sum();
    let finished = Arc::new(AtomicUsize::new(0));
    let store = open_shared(&fixture);
    let task_id = fixture.work.task.task_id.clone();
    let item = fixture.work.item.item_id.clone();
    let hook_store = Arc::clone(&store);
    let (hook_task, hook_item) = (task_id.clone(), item.clone());
    let fired = Arc::new(AtomicBool::new(false));
    let worker = route_worker_for(&fixture, &Arc::new(AtomicUsize::new(0)))
        .with_nisar_subset_v2_after_science_commit_hook_for_test(Arc::new(
            move |index, control| {
                if index == 1 && !fired.swap(true, Ordering::SeqCst) {
                    item_command(
                        &mut hook_store.lock().unwrap(),
                        &hook_task,
                        &hook_item,
                        TaskStateCommandKind::PauseItems,
                        "pause-science",
                    );
                    control.set_paused_for_test(true);
                }
            },
        ));
    let scheduler = start_route_scheduler(&store, worker, None, None, Arc::clone(&finished));
    wait_for_runtime(
        &store,
        &fixture,
        "paused after an upgraded Range",
        |runtime| runtime.paused_item_ids.contains(&item) && finished.load(Ordering::SeqCst) >= 1,
    );
    thread::sleep(Duration::from_millis(200));
    let paused = local_checkpoint(&fixture).unwrap();
    assert!(paused["budget_upgrade_receipt"].is_object());
    let completed = completed_bytes(&paused);
    let entry_before = hashed_bytes(&workspace, "migration_entry");
    let executor_before = hashed_bytes(&workspace, "executor");
    assert_eq!(hashed_bytes(&workspace, "writer_science_revalidation"), 0);
    item_command(
        &mut store.lock().unwrap(),
        &task_id,
        &item,
        TaskStateCommandKind::ResumeItems,
        "resume",
    );
    let runtime = wait_for_runtime(&store, &fixture, "terminal after resume", terminal);
    scheduler.shutdown();
    drop(store);
    assert_completed_with_single_proof(&fixture, runtime.status);
    assert_eq!(
        hashed_bytes(&workspace, "migration_entry") - entry_before,
        completed
    );
    assert_eq!(
        hashed_bytes(&workspace, "executor") - executor_before,
        2 * completed + total
    );
    assert_eq!(
        hashed_bytes(&workspace, "writer_science_revalidation"),
        total
    );
}

/// Design 2026-10-17 §5 (workspace rebinding): a retained V1 task whose output
/// root was renamed and is reached through a junction at the old path keeps
/// its recorded path through the budget-upgrade entry and the anchor CAS; the
/// first acceptance writes the non-authoritative evidence once. After a pause
/// with the proof committed, the resume reuses that proof, fetches only the
/// remaining Ranges and the writer verifies and publishes the artifact.
#[cfg(windows)]
#[test]
fn production_budget_upgrade_resumes_a_renamed_output_root_behind_a_junction() {
    let fixture = seed_retained_route("budget-upgrade-renamed-root");
    let stored_before = load_store_anchor(&fixture).unwrap();
    let anchor_before = anchor_record(&stored_before);
    let recorded = local_checkpoint(&fixture).unwrap()["workspace_canonical_path"]
        .as_str()
        .unwrap()
        .to_owned();
    let output = fixture.work.task.output_root.clone();
    let _renamed = super::workspace_rebinding::RenamedBehindJunction::new(&output, "-renamed");
    let count = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let store = open_shared(&fixture);
    let scheduler = start_route_scheduler(
        &store,
        route_worker_for(&fixture, &count),
        Some(pause_at(
            NisarBudgetMigrationStageForTest::AfterCommit,
            &store,
            &fixture,
        )),
        None,
        Arc::clone(&finished),
    );
    let item = fixture.work.item.item_id.clone();
    wait_for_runtime(
        &store,
        &fixture,
        "paused after the committed upgrade",
        |runtime| runtime.paused_item_ids.contains(&item) && finished.load(Ordering::SeqCst) >= 1,
    );
    thread::sleep(Duration::from_millis(200));
    scheduler.shutdown();
    drop(store);
    assert!(load_refusals(&fixture).is_empty());
    let proof = load_proof(&fixture).expect("the upgrade committed behind the junction");
    let local = local_checkpoint(&fixture).unwrap();
    assert_eq!(
        local["workspace_canonical_path"],
        serde_json::json!(recorded)
    );
    assert_eq!(
        local["budget_upgrade_receipt"]["proof_sha256"],
        proof.proof_sha256()
    );
    let anchor_after = anchor_record(&load_store_anchor(&fixture).unwrap());
    for key in ["lineage_key_sha256", "execution_binding_sha256"] {
        assert_eq!(anchor_after[key], anchor_before[key], "{key}");
    }
    assert!(anchor_after["generation"].as_u64() > anchor_before["generation"].as_u64());
    let evidence: Value = serde_json::from_str(
        &fs::read_to_string(route_recovery_root(&fixture.work).join("workspace-rebinding-v2.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        evidence["contract"],
        serde_json::json!("nisar_workspace_rebinding_evidence_v2")
    );
    assert_eq!(
        evidence["authority"],
        serde_json::json!("non_authoritative")
    );
    assert_eq!(
        evidence["task_id"],
        serde_json::json!(fixture.work.task.task_id)
    );
    assert_eq!(
        evidence["item_id"],
        serde_json::json!(fixture.work.item.item_id)
    );
    assert_eq!(evidence["recorded_path"], serde_json::json!(recorded));
    let current = evidence["current_path"].as_str().unwrap();
    assert!(
        current != recorded && current.contains("output-renamed"),
        "{current}"
    );
    assert_eq!(evidence["identity"], local["workspace_identity"]);
    assert_eq!(evidence["volume_file_system"], serde_json::json!("NTFS"));
    assert_eq!(
        evidence["recorded_path_resolution"],
        serde_json::json!("same_identity")
    );
    assert_eq!(
        evidence["anchor_record_sha256"],
        serde_json::json!(stored_before.record_sha256)
    );
    assert_eq!(evidence["anchor_generation"], anchor_before["generation"]);
    assert_eq!(
        evidence["science_checkpoint_sha256"],
        serde_json::from_str::<Value>(fixture.checkpoint.as_ref().unwrap()).unwrap()
            ["checkpoint_sha256"]
    );
    assert!(evidence["planning_checkpoint_sha256"].is_string());
    item_command(
        &mut TaskStore::open_existing(&fixture.database).unwrap(),
        &fixture.work.task.task_id,
        &item,
        TaskStateCommandKind::ResumeItems,
        "resume",
    );
    let status = run_retained_route(&fixture, None, count);
    let completed = assert_completed_with_single_proof(&fixture, status);
    assert_eq!(completed, proof, "the resume reuses the committed proof");
    // The local evidence left with the workspace; one Store event remains.
    let events = TaskStore::open_existing(&fixture.database)
        .unwrap()
        .list_task_events(&fixture.work.task.task_id)
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "nisar_workspace_rebinding")
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(
        events[0].payload["recorded_path"],
        evidence["recorded_path"]
    );
    assert_eq!(events[0].payload["current_path"], evidence["current_path"]);
    assert_eq!(events[0].payload["identity"], evidence["identity"]);
}

/// Design 2026-10-17 §6.1: with a Store anchor the worker only opens the
/// existing recovery workspace. When the old output root no longer resolves
/// (moved without a junction), the item fails locally and nothing is
/// recreated at the old path.
#[test]
fn production_route_with_an_anchor_never_recreates_a_missing_recovery_workspace() {
    let fixture = seed_retained_route("anchored-workspace-missing");
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let output = fixture.work.task.output_root.clone();
    let moved = output.with_file_name("output-moved");
    fs::rename(&output, &moved).unwrap();
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let output_recreated = output.exists();
    let failure = visible_failure(&fixture);
    if output_recreated {
        let _ = fs::remove_dir_all(&output);
    }
    fs::rename(&moved, &output).unwrap();
    assert_eq!(status, TaskLifecycleStatus::Failed);
    let failure = failure.expect("a visible local failure");
    assert_eq!(
        failure.code, "NISAR_SUBSET_V2_RECOVERY_INVALID",
        "{}",
        failure.reason_zh
    );
    assert!(
        !output_recreated,
        "the old output root must not be recreated"
    );
    assert!(load_proof(&fixture).is_none());
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before,
        "no request after the seeded transfer"
    );
}

/// Review 2026-10-17 round 3, P2-1: a failure before the migration decision
/// keeps the prior result row of a retained V1 item (for example the retained
/// 465/467 failure) byte for byte as the first immutable refusal evidence, and
/// the visible failure replacing it is exactly that recorded local failure.
fn assert_prior_row_retained(fixture: &RetainedRoute, raw_old: RawResultRow) -> DownloadFailure {
    let refusals = load_refusals(fixture);
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert_eq!(
        refusals[0].previous_result(),
        &raw_row_as_evidence(raw_old),
        "the prior result row is retained before it is replaced"
    );
    let failure = visible_failure(fixture).expect("a visible local failure");
    assert_eq!(refusals[0].failure(), &failure);
    assert!(
        failure.reason_zh.contains("没有发起任何网络请求")
            && failure.reason_zh.contains("source=local"),
        "{}",
        failure.reason_zh
    );
    failure
}

#[cfg(windows)]
fn rebinding_events(fixture: &RetainedRoute) -> Vec<insar_store::StoredTaskEvent> {
    TaskStore::open_existing(&fixture.database)
        .unwrap()
        .list_task_events(&fixture.work.task.task_id)
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "nisar_workspace_rebinding")
        .collect()
}

/// Review 2026-10-17 round 3, P2-1: with a Store anchor, a recovery workspace
/// that no longer resolves fails before the migration decision. That failure
/// is shown only after the prior result row was retained, and it names the
/// local cause instead of replacing the row with an English diagnostic.
#[test]
fn production_route_missing_anchored_workspace_retains_the_prior_result_row() {
    let fixture = seed_retained_route("anchored-workspace-missing-evidence");
    let raw_old = raw_result_row(&fixture.database, &fixture.work).unwrap();
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let output = fixture.work.task.output_root.clone();
    let moved = output.with_file_name("output-moved");
    fs::rename(&output, &moved).unwrap();
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let output_recreated = output.exists();
    if output_recreated {
        let _ = fs::remove_dir_all(&output);
    }
    fs::rename(&moved, &output).unwrap();
    assert_eq!(status, TaskLifecycleStatus::Failed);
    assert!(
        !output_recreated,
        "the old output root must not be recreated"
    );
    let failure = assert_prior_row_retained(&fixture, raw_old);
    assert_eq!(
        failure.code, "NISAR_SUBSET_V2_RECOVERY_INVALID",
        "{}",
        failure.reason_zh
    );
    assert!(!failure.retryable, "{}", failure.reason_zh);
    assert!(
        failure.reason_zh.contains("恢复工作区")
            && failure.reason_zh.contains("stage=recovery_workspace_open"),
        "{}",
        failure.reason_zh
    );
    assert!(load_proof(&fixture).is_none());
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before,
        "no request after the seeded transfer"
    );
}

/// Review 2026-10-17 round 3, P2-1: writing the rebinding evidence is local file
/// I/O before the migration decision. When it fails, nothing is sent, the prior
/// result row is retained before the visible failure replaces it, and the
/// failure is a retryable local cause: once the cause is gone, a continue
/// rebinds, upgrades and completes.
#[cfg(windows)]
#[test]
fn production_route_rebinding_evidence_write_failure_is_transient_and_retains_the_prior_row() {
    let fixture = seed_retained_route("rebinding-evidence-write-failure");
    let raw_old = raw_result_row(&fixture.database, &fixture.work).unwrap();
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let output = fixture.work.task.output_root.clone();
    let _renamed = super::workspace_rebinding::RenamedBehindJunction::new(&output, "-renamed");
    // A directory where the atomic writer stages its file: an ordinary I/O
    // failure of that write.
    let staged = route_recovery_root(&fixture.work).join("workspace-rebinding-v2.json.next");
    fs::create_dir(&staged).unwrap();
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    assert_eq!(status, TaskLifecycleStatus::Failed);
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    let failure = assert_prior_row_retained(&fixture, raw_old);
    assert_eq!(
        failure.code, "NISAR_SUBSET_V2_CHECKPOINT_PERSISTENCE",
        "{}",
        failure.reason_zh
    );
    assert!(failure.retryable, "{}", failure.reason_zh);
    assert!(
        failure
            .reason_zh
            .contains("stage=workspace_rebinding_evidence"),
        "{}",
        failure.reason_zh
    );
    assert!(load_proof(&fixture).is_none());
    assert!(rebinding_events(&fixture).is_empty());
    assert!(!route_recovery_root(&fixture.work)
        .join("workspace-rebinding-v2.json")
        .exists());
    fs::remove_dir(&staged).unwrap();
    retry_failed_item(&fixture);
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    let proof = assert_completed_with_single_proof(&fixture, status);
    assert_single_upgrade_event(&fixture, &proof);
    assert_eq!(rebinding_events(&fixture).len(), 1);
}

/// Review 2026-10-17 round 3, P2-1: the Store refuses the rebinding task event
/// (an injected SQLite abort). A database refusal is not transient: the item
/// fails terminally with the Store's own code before any request, after the
/// prior result row was retained.
#[cfg(windows)]
#[test]
fn production_route_rebinding_event_store_refusal_is_terminal_and_retains_the_prior_row() {
    let fixture = seed_retained_route("rebinding-event-store-refusal");
    let raw_old = raw_result_row(&fixture.database, &fixture.work).unwrap();
    rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER inject_rebinding_event_refusal BEFORE INSERT ON task_events \
             WHEN NEW.event_type = 'nisar_workspace_rebinding' \
             BEGIN SELECT RAISE(ABORT, 'injected rebinding event refusal'); END;",
        )
        .unwrap();
    let requests_before = fixture.server.requests.lock().unwrap().len();
    let output = fixture.work.task.output_root.clone();
    let _renamed = super::workspace_rebinding::RenamedBehindJunction::new(&output, "-renamed");
    let status = run_retained_route(&fixture, None, Arc::new(AtomicUsize::new(0)));
    assert_eq!(status, TaskLifecycleStatus::Failed);
    assert_eq!(
        fixture.server.requests.lock().unwrap().len(),
        requests_before
    );
    let failure = assert_prior_row_retained(&fixture, raw_old);
    assert_eq!(
        failure.code, "V3_STORE_DATABASE_ERROR",
        "{}",
        failure.reason_zh
    );
    assert!(!failure.retryable, "{}", failure.reason_zh);
    assert!(
        failure
            .reason_zh
            .contains("stage=workspace_rebinding_event")
            && failure.reason_zh.contains("不是暂时性故障"),
        "{}",
        failure.reason_zh
    );
    assert!(load_proof(&fixture).is_none());
    assert!(rebinding_events(&fixture).is_empty());
}
