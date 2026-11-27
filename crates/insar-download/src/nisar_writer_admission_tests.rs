//! D1 of the downloader enhancement plan (2026-10-18): output admission of the
//! NISAR V2 writer. L4 round 3 ran one task with concurrency 2 whose items
//! share one output directory; the second item failed with
//! NISAR_SUBSET_V2_OUTPUT_BUSY because one non-blocking lock covered the whole
//! directory and was taken only after about ten full material hashes.
//! Writers of different artifacts in one directory must both complete, writers
//! of one artifact serialize, and a held lock is waited for; a pause stops the
//! wait within one second and is reported as a pause. While a writer waits,
//! its item shows 等待写出.
use super::writer_baseline::{
    normalized_baseline, science_complete_fixture, BaselineRun, ScienceCompleteFixture,
    EDGE_CHUNK_BASELINE, EDGE_CHUNK_LAYOUT,
};
use super::*;
use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferResultV2;
use crate::nisar_subset_stream_v2::{V2AfterTileHookForTest, V2StreamPass};
use crate::nisar_subset_writer::tests::writer_with_anchor;
use crate::nisar_subset_writer::{
    NisarSubsetWriteError as Error, NisarSubsetWriteResultV2, WriterFailpointV2,
};
use crate::nisar_writer_admission_v2::{
    NisarWriterAdmissionEventV2 as Event, NisarWriterOutputSlotsV2,
    NisarWriterWaitReasonV2 as Reason,
};
use crate::{DestinationLeaseController, OfflineNisarSubsetWriter};

/// Bound of every cross-thread step; a hang fails the test instead of the run.
const STEP_TIMEOUT: Duration = Duration::from_secs(120);

/// What a writer thread needs from a science-complete fixture. The fixture
/// itself stays on the test thread, which owns the HDF5 test lock.
#[derive(Clone, Copy)]
pub(super) struct WriterInput<'a> {
    work: &'a DownloadWork,
    completed: &'a NisarSubsetPlanningCompleteV2,
    science: &'a NisarSubsetScienceTransferResultV2,
    bootstrap: &'a NisarSubsetLocalHdf5MetadataBootstrapV2,
    url_root: &'a str,
    anchor: &'a ScienceCheckpointAnchorFixture,
}

impl<'a> WriterInput<'a> {
    pub(super) fn of(
        fixture: &'a ScienceCompleteFixture,
        anchor: &'a ScienceCheckpointAnchorFixture,
    ) -> Self {
        Self {
            work: &fixture.work,
            completed: &fixture.completed,
            science: &fixture.science,
            bootstrap: fixture.bootstrap.as_ref(),
            url_root: &fixture.url_root,
            anchor,
        }
    }

    pub(super) fn write(
        &self,
        writer: &OfflineNisarSubsetWriter,
        control: &WorkerControl,
    ) -> Result<NisarSubsetWriteResultV2, Error> {
        self.write_observed(writer, control, WriterFailpointV2::None, &mut |_| {})
    }

    pub(super) fn write_observed(
        &self,
        writer: &OfflineNisarSubsetWriter,
        control: &WorkerControl,
        failpoint: WriterFailpointV2,
        admission: &mut dyn FnMut(Event),
    ) -> Result<NisarSubsetWriteResultV2, Error> {
        let physical = self.completed.physical_execution_plan();
        let asset = science_asset(
            self.url_root,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        );
        let authority = science_authority(
            self.bootstrap,
            &asset,
            self.work,
            self.completed,
            self.anchor,
        );
        writer.write_and_validate_v2_observed_with_failpoint_for_test(
            self.work,
            self.completed,
            self.science,
            &authority,
            control,
            failpoint,
            admission,
        )
    }
}

fn artifact_id(fixture: &ScienceCompleteFixture) -> String {
    fixture
        .completed
        .frozen_plan()
        .provenance
        .derived_artifact_id
        .clone()
}

fn assert_edge_baseline(fixture: &ScienceCompleteFixture, written: &NisarSubsetWriteResultV2) {
    let manifest = serde_json::to_value(written.manifest()).unwrap();
    let run = BaselineRun {
        artifact_sha256: test_file_sha256(written.artifact_path()),
        artifact_bytes: fs::metadata(written.artifact_path()).unwrap().len(),
        windows: manifest["dataset_windows"].clone(),
        manifest,
        artifact: fs::read(written.artifact_path()).unwrap(),
        sidecar: fs::read(written.manifest_path()).unwrap(),
        source_sha256: test_file_sha256(&fixture.case.source_path),
    };
    let normalized = normalized_baseline(&run);
    assert_eq!(
        (
            normalized.artifact_sha256.as_str(),
            normalized.sidecar_sha256.as_str()
        ),
        EDGE_CHUNK_BASELINE
    );
}

fn assert_published(written: &NisarSubsetWriteResultV2) {
    assert_eq!(
        test_file_sha256(written.artifact_path()),
        written.manifest().artifact_sha256()
    );
    assert!(written.manifest_path().is_file());
}

struct Finished {
    result: Result<NisarSubsetWriteResultV2, Error>,
    at: Instant,
    events: Vec<Event>,
}

/// Runs writer A until its first streamed tile, then writer B, and holds A
/// inside its output until B announced that it waits (or B finished). Both
/// writers share one destination lease and the given output slots, as the
/// items of one worker do.
fn run_overlapping(
    first: (WriterInput<'_>, &Path),
    second: (WriterInput<'_>, &Path),
    output: &Path,
    lease: &Arc<DestinationLeaseController>,
    slots: &Arc<NisarWriterOutputSlotsV2>,
) -> (Finished, Finished) {
    let (inside_tx, inside_rx) = mpsc::channel::<()>();
    let second_waiting_or_done = Arc::new(AtomicBool::new(false));
    thread::scope(|scope| {
        let hook_release = Arc::clone(&second_waiting_or_done);
        let first_handle = scope.spawn(move || {
            let writer = writer_with_anchor(output, first.1, Arc::clone(lease))
                .with_output_slots_v2(Arc::clone(slots));
            let fired = AtomicBool::new(false);
            let _hook = V2AfterTileHookForTest::set(move |pass, _| {
                if pass == V2StreamPass::Write && !fired.swap(true, Ordering::SeqCst) {
                    inside_tx.send(()).unwrap();
                    let deadline = Instant::now() + STEP_TIMEOUT;
                    while !hook_release.load(Ordering::SeqCst) && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
            });
            let mut events = Vec::new();
            let result = first.0.write_observed(
                &writer,
                &WorkerControl::default(),
                WriterFailpointV2::None,
                &mut |event| events.push(event),
            );
            Finished {
                result,
                at: Instant::now(),
                events,
            }
        });
        inside_rx
            .recv_timeout(STEP_TIMEOUT)
            .expect("the first writer reached its output");
        let release = &second_waiting_or_done;
        let second_handle = scope.spawn(move || {
            let writer = writer_with_anchor(output, second.1, Arc::clone(lease))
                .with_output_slots_v2(Arc::clone(slots));
            let mut events = Vec::new();
            let result = second.0.write_observed(
                &writer,
                &WorkerControl::default(),
                WriterFailpointV2::None,
                &mut |event| {
                    if matches!(event, Event::Waiting { .. }) {
                        release.store(true, Ordering::SeqCst);
                    }
                    events.push(event);
                },
            );
            release.store(true, Ordering::SeqCst);
            Finished {
                result,
                at: Instant::now(),
                events,
            }
        });
        (first_handle.join().unwrap(), second_handle.join().unwrap())
    })
}

/// L4 round 3 (Issue #8 task A, July1): two items of one task, concurrency 2,
/// one output directory, the default single output slot. The second writer
/// waits (等待写出) instead of failing with NISAR_SUBSET_V2_OUTPUT_BUSY, is
/// admitted when the first returns, and both artifacts are published.
#[test]
fn writer_v2_two_items_writing_into_one_output_directory_both_complete() {
    let anchor_a = ScienceCheckpointAnchorFixture::default();
    let anchor_b = ScienceCheckpointAnchorFixture::default();
    let a = science_complete_fixture(
        "writer-v2-admission-two-a",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor_a,
    );
    let b = science_complete_fixture(
        "writer-v2-admission-two-b",
        &["HH", "HV"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor_b,
    );
    assert_ne!(artifact_id(&a), artifact_id(&b), "two different artifacts");
    let output = a.case.root.0.join("writer-v2-admission-two-output");
    let anchor_path_a = a.case.root.0.join("anchor-a.json");
    let anchor_path_b = b.case.root.0.join("anchor-b.json");
    let lease = DestinationLeaseController::new_for_qualification_test();
    let slots = NisarWriterOutputSlotsV2::new(
        crate::nisar_writer_admission_v2::NISAR_WRITER_V2_DEFAULT_OUTPUT_SLOTS,
    );
    let (first, second) = run_overlapping(
        (WriterInput::of(&a, &anchor_a), &anchor_path_a),
        (WriterInput::of(&b, &anchor_b), &anchor_path_b),
        &output,
        &lease,
        &slots,
    );
    let first_written = first.result.expect("the first item is published");
    let second_written = second
        .result
        .expect("the second item waits for the output and is published");
    assert!(first.events.is_empty(), "{:?}", first.events);
    assert_eq!(
        second.events,
        vec![
            Event::Waiting {
                reason: Reason::OutputSlot
            },
            Event::Admitted
        ]
    );
    assert!(second.at >= first.at);
    assert_edge_baseline(&a, &first_written);
    assert_published(&second_written);
    assert!(writer_v2_candidate_paths(&output).is_empty());
    assert_eq!(slots.in_use_for_test(), 0);
}

/// Two tasks producing one artifact into one directory never write it at the
/// same time: the second writer waits until the first returned. It then finds
/// an artifact anchored by another task and refuses it without touching it
/// (unchanged recovery semantics); it does not fail as busy.
#[test]
fn writer_v2_two_writers_of_one_artifact_serialize() {
    let anchor_a = ScienceCheckpointAnchorFixture::default();
    let anchor_b = ScienceCheckpointAnchorFixture::default();
    let a = science_complete_fixture(
        "writer-v2-admission-same-a",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor_a,
    );
    let b = science_complete_fixture(
        "writer-v2-admission-same-b",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor_b,
    );
    assert_eq!(artifact_id(&a), artifact_id(&b), "one artifact");
    let output = a.case.root.0.join("writer-v2-admission-same-output");
    let anchor_path_a = a.case.root.0.join("anchor-a.json");
    let anchor_path_b = b.case.root.0.join("anchor-b.json");
    let lease = DestinationLeaseController::new_for_qualification_test();
    // More slots than writers: only the artifact serializes them.
    let slots = NisarWriterOutputSlotsV2::new(2);
    let (first, second) = run_overlapping(
        (WriterInput::of(&a, &anchor_a), &anchor_path_a),
        (WriterInput::of(&b, &anchor_b), &anchor_path_b),
        &output,
        &lease,
        &slots,
    );
    let first_written = first.result.expect("the first task publishes the artifact");
    let published_sha256 = test_file_sha256(first_written.artifact_path());
    assert_eq!(
        second.result.unwrap_err(),
        Error::RecoveryInvalid,
        "the second task waits, then refuses an artifact anchored by another task"
    );
    assert_eq!(
        second.events,
        vec![
            Event::Waiting {
                reason: Reason::SameArtifactInProcess
            },
            Event::Admitted
        ]
    );
    assert!(
        second.at >= first.at,
        "the second writer finished only after the first one returned"
    );
    assert_eq!(
        test_file_sha256(first_written.artifact_path()),
        published_sha256
    );
    assert_edge_baseline(&a, &first_written);
    assert!(writer_v2_candidate_paths(&output).is_empty());
}

/// Writers of different artifacts in one directory are not serialized by the
/// artifact lock: with two output slots both are inside their output at the
/// same time and neither waits.
#[test]
fn writer_v2_different_artifacts_in_one_directory_write_in_parallel() {
    let anchor_a = ScienceCheckpointAnchorFixture::default();
    let anchor_b = ScienceCheckpointAnchorFixture::default();
    let a = science_complete_fixture(
        "writer-v2-admission-parallel-a",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor_a,
    );
    let b = science_complete_fixture(
        "writer-v2-admission-parallel-b",
        &["HH", "HV"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor_b,
    );
    let output = a.case.root.0.join("writer-v2-admission-parallel-output");
    let anchor_path_a = a.case.root.0.join("anchor-a.json");
    let anchor_path_b = b.case.root.0.join("anchor-b.json");
    let lease = DestinationLeaseController::new_for_qualification_test();
    // EOSTAGE_NISAR_WRITER_SLOTS=2 (review of D1/D2, downloader enhancement
    // D1b: the number of output slots is configurable).
    let slots = NisarWriterOutputSlotsV2::new(
        crate::nisar_writer_admission_v2::parse_nisar_writer_output_slots(Some("2")).unwrap(),
    );
    let inside = Arc::new(AtomicUsize::new(0));
    let both_inside = Arc::new(AtomicBool::new(false));
    let inputs = [
        (WriterInput::of(&a, &anchor_a), anchor_path_a.as_path()),
        (WriterInput::of(&b, &anchor_b), anchor_path_b.as_path()),
    ];
    let results = thread::scope(|scope| {
        let handles = inputs
            .into_iter()
            .map(|(input, anchor_path)| {
                let inside = Arc::clone(&inside);
                let both_inside = Arc::clone(&both_inside);
                let (output, lease, slots) = (&output, &lease, &slots);
                scope.spawn(move || {
                    let writer = writer_with_anchor(output, anchor_path, Arc::clone(lease))
                        .with_output_slots_v2(Arc::clone(slots));
                    let fired = AtomicBool::new(false);
                    let _hook = V2AfterTileHookForTest::set(move |pass, _| {
                        if pass == V2StreamPass::Write && !fired.swap(true, Ordering::SeqCst) {
                            inside.fetch_add(1, Ordering::SeqCst);
                            let deadline = Instant::now() + Duration::from_secs(10);
                            while inside.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
                                thread::sleep(Duration::from_millis(10));
                            }
                            if inside.load(Ordering::SeqCst) >= 2 {
                                both_inside.store(true, Ordering::SeqCst);
                            }
                        }
                    });
                    let mut events = Vec::new();
                    let result = input.write_observed(
                        &writer,
                        &WorkerControl::default(),
                        WriterFailpointV2::None,
                        &mut |event| events.push(event),
                    );
                    (result, events)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    for (result, events) in &results {
        assert_published(result.as_ref().expect("both artifacts are published"));
        assert!(events.is_empty(), "no writer waited: {events:?}");
    }
    assert!(
        both_inside.load(Ordering::SeqCst),
        "two different artifacts were written at the same time"
    );
}

/// A writer that finds its artifact lock held by another writer (another
/// process) waits instead of failing; a pause stops the wait within one second
/// and is a pause; the next run publishes the baseline artifact.
#[test]
fn writer_v2_waits_for_a_held_artifact_lock_and_pauses_within_one_second() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-admission-held",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    let output = fixture.case.root.0.join("writer-v2-admission-held-output");
    let anchor_path = fixture.case.root.0.join("anchor.json");
    let lock_directory = output.join(".eostage-locks");
    fs::create_dir_all(&lock_directory).unwrap();
    let held = crate::hdf5_range_transfer::acquire_hdf5_workspace_lock(&lock_directory.join(
        format!("nisar-subset-writer-v2-{}.lock", artifact_id(&fixture)),
    ))
    .unwrap();
    let lease = DestinationLeaseController::new_for_qualification_test();
    let control = WorkerControl::default();
    let input = WriterInput::of(&fixture, &anchor);
    let (event_tx, event_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    thread::scope(|scope| {
        let worker_control = control.clone();
        let (output, anchor_path, lease) = (&output, &anchor_path, &lease);
        scope.spawn(move || {
            let writer = writer_with_anchor(output, anchor_path, Arc::clone(lease));
            let result = input.write_observed(
                &writer,
                &worker_control,
                WriterFailpointV2::None,
                &mut |event| {
                    let _ = event_tx.send(event);
                },
            );
            result_tx.send(result).unwrap();
        });
        assert_eq!(
            event_rx.recv_timeout(STEP_TIMEOUT).unwrap(),
            Event::Waiting {
                reason: Reason::SameArtifactOtherProcess
            },
            "the writer announces that it waits for its artifact lock"
        );
        assert!(
            result_rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "the writer keeps waiting while the lock is held"
        );
        let paused_at = Instant::now();
        control.set_paused_for_test(true);
        let result = result_rx
            .recv_timeout(STEP_TIMEOUT)
            .expect("the waiting writer returns");
        assert!(
            paused_at.elapsed() < Duration::from_secs(1),
            "a pause stops the wait within one second: {:?}",
            paused_at.elapsed()
        );
        assert_eq!(result.unwrap_err(), Error::Cancelled, "a pause is a pause");
        assert!(
            event_rx.try_recv().is_err(),
            "a paused writer is never admitted"
        );
    });
    assert!(writer_v2_candidate_paths(&output).is_empty());
    drop(held);
    let writer = writer_with_anchor(&output, &anchor_path, lease);
    let written = input.write(&writer, &WorkerControl::default()).unwrap();
    assert_edge_baseline(&fixture, &written);
}

/// Found by the tests above: a pause that arrives while the admitted writer
/// reads the metadata closure was reported as a non-retryable binding failure
/// (NISAR_SUBSET_V2_WRITER_BINDING). It is a pause.
#[test]
fn writer_v2_pause_while_reading_the_metadata_closure_is_a_pause() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-admission-metadata-pause",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    let output = fixture
        .case
        .root
        .0
        .join("writer-v2-admission-metadata-pause-output");
    let anchor_path = fixture.case.root.0.join("anchor.json");
    let lease = DestinationLeaseController::new_for_qualification_test();
    let writer = writer_with_anchor(&output, &anchor_path, Arc::clone(&lease));
    let input = WriterInput::of(&fixture, &anchor);
    assert_eq!(
        input
            .write_observed(
                &writer,
                &WorkerControl::default(),
                WriterFailpointV2::PauseBeforeMetadataCapture,
                &mut |_| {},
            )
            .unwrap_err(),
        Error::Cancelled
    );
    assert!(writer_v2_candidate_paths(&output).is_empty());
    let written = input.write(&writer, &WorkerControl::default()).unwrap();
    assert_edge_baseline(&fixture, &written);
}

/// Review of D1/D2 (downloader enhancement D1b, P2-4): the scheduler resumes
/// the item right after the metadata read stopped, so the pause flag is clear
/// again when the writer maps the stop. It is still a pause, not the
/// non-retryable NISAR_SUBSET_V2_WRITER_BINDING.
#[test]
fn writer_v2_pause_then_immediate_resume_while_reading_the_metadata_closure_is_a_pause() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-admission-metadata-race",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    let output = fixture
        .case
        .root
        .0
        .join("writer-v2-admission-metadata-race-output");
    let anchor_path = fixture.case.root.0.join("anchor.json");
    let lease = DestinationLeaseController::new_for_qualification_test();
    let writer = writer_with_anchor(&output, &anchor_path, Arc::clone(&lease));
    let input = WriterInput::of(&fixture, &anchor);
    let control = WorkerControl::default();
    assert_eq!(
        input
            .write_observed(
                &writer,
                &control,
                WriterFailpointV2::PauseThenResumeDuringMetadataCapture,
                &mut |_| {},
            )
            .unwrap_err(),
        Error::Cancelled
    );
    assert!(
        !control.is_paused(),
        "the item was resumed while the writer stopped"
    );
    assert!(writer_v2_candidate_paths(&output).is_empty());
    let written = input.write(&writer, &WorkerControl::default()).unwrap();
    assert_edge_baseline(&fixture, &written);
}

/// Review of D1/D2 (downloader enhancement D1b, P2-7): the per-artifact lock
/// files live in the hidden `.eostage-locks` directory of the output root,
/// not beside the user's artifacts, and are never removed (removing a held
/// lock file would let a later writer lock a new file on Unix). Only the
/// earlier builds' directory lock stays in the output root while earlier
/// builds may still write there.
#[test]
fn writer_v2_keeps_its_artifact_locks_in_a_hidden_lock_directory() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-admission-lock-directory",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    let output = fixture
        .case
        .root
        .0
        .join("writer-v2-admission-lock-directory-output");
    let anchor_path = fixture.case.root.0.join("anchor.json");
    let lease = DestinationLeaseController::new_for_qualification_test();
    let writer = writer_with_anchor(&output, &anchor_path, lease);
    let written = WriterInput::of(&fixture, &anchor)
        .write(&writer, &WorkerControl::default())
        .unwrap();
    assert_edge_baseline(&fixture, &written);
    let id = artifact_id(&fixture);
    let root_locks = fs::read_dir(&output)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".lock"))
        .collect::<Vec<_>>();
    assert_eq!(
        root_locks,
        vec![".nisar-subset-writer-v2.lock".to_owned()],
        "only the earlier builds' directory lock stays in the output root"
    );
    let lock_directory = output.join(".eostage-locks");
    assert!(lock_directory.is_dir());
    assert!(lock_directory
        .join(format!("nisar-subset-writer-v2-{id}.lock"))
        .is_file());
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        assert_ne!(
            fs::metadata(&lock_directory).unwrap().file_attributes() & FILE_ATTRIBUTE_HIDDEN,
            0,
            "the lock directory is hidden"
        );
    }
}

/// The production route when an earlier build holds the output directory
/// (D1c): after the threshold (here 300 ms) the task log records one warning
/// naming the earlier build, the item keeps waiting without failing, and once
/// the earlier build releases the directory the output is published.
#[test]
fn production_route_v2_warns_when_an_earlier_build_holds_the_output_directory() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let label = "route-writer-earlier-build-wait";
    let case = route_fixture(label, NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let work = route_ready_work(&case, &intent);
    fs::create_dir_all(&work.task.output_root).unwrap();
    let earlier_build = crate::hdf5_range_transfer::acquire_hdf5_workspace_lock(
        &work.task.output_root.join(".nisar-subset-writer-v2.lock"),
    )
    .unwrap();
    let database = case._root.0.join(format!("{label}.sqlite"));
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    resume_scheduler_task(&mut store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let worker = Arc::new(
        route_worker(server.url.clone(), &case, &work.item)
            .with_nisar_writer_long_wait_after_for_test(Duration::from_millis(300)),
    );
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(5));
    scheduler.wake();
    let deadline = Instant::now() + STEP_TIMEOUT;
    let long_wait = loop {
        let events = store
            .lock()
            .unwrap()
            .list_task_events(&work.task.task_id)
            .unwrap();
        if let Some(event) = events
            .into_iter()
            .find(|event| event.event_type == "nisar_output_wait_earlier_build")
        {
            break event;
        }
        assert!(
            Instant::now() < deadline,
            "the earlier build's hold was never recorded"
        );
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        long_wait.item_id.as_deref(),
        Some(work.item.item_id.as_str())
    );
    assert_eq!(long_wait.payload["reason"], json!("earlier_build"));
    if cfg!(windows) {
        // The earlier build's process id is in its locked lock file, which
        // another handle cannot read on Windows.
        assert_eq!(long_wait.payload["holder_pid"], json!(null));
    }
    {
        let store = store.lock().unwrap();
        let revision = store
            .load_task_record(&work.task.task_id)
            .unwrap()
            .unwrap()
            .runtime
            .revision;
        let page = store
            .load_task_event_page(&insar_store::StoreTaskEventPageRequest {
                task_id: work.task.task_id.clone(),
                expected_revision: revision,
                before_sequence: None,
                page_size: 20,
            })
            .unwrap();
        assert!(
            page.items.iter().any(|item| item.level == "warning"
                && item.message_zh == "写出已等待超过 5 分钟：旧版本程序正在向同一输出目录写出"),
            "{page:?}"
        );
    }
    assert_eq!(waiting_items(&store, &work.task.task_id), 1);
    let runtime = store
        .lock()
        .unwrap()
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap()
        .runtime;
    assert!(runtime.failed_item_ids.is_empty(), "{runtime:?}");
    drop(earlier_build);
    wait_for_route_completed(&store, &work.task.task_id);
    scheduler.shutdown();
    drop(server.finish());
}

struct ProgressRows {
    inner: ProductionDownloadWorker,
    rows: Arc<Mutex<Vec<insar_core::DownloadItemProgress>>>,
}

impl DownloadWorker for ProgressRows {
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

fn item_command(store: &SharedTaskStore, work: &DownloadWork, kind: TaskStateCommandKind) {
    let mut store = store.lock().unwrap();
    let revision = store
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap()
        .runtime
        .revision;
    let event_id = format!("writer-admission-{kind:?}-{revision}");
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: work.task.task_id.clone(),
            expected_revision: revision,
            command: StoreTaskCommand {
                kind,
                item_ids: vec![work.item.item_id.clone()],
            },
            event_id,
            occurred_at: "2026-10-18T00:00:05Z".to_owned(),
        })
        .unwrap();
}

fn waiting_items(store: &SharedTaskStore, task_id: &str) -> u64 {
    store
        .lock()
        .unwrap()
        .core_restore(true)
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| task.task_id == task_id)
        .map_or(0, |task| task.active_progress.waiting)
}

/// The production route: the worker's only output slot is held by another
/// writer when the science transfer ends. The item publishes a queued row, so
/// the Store counts it as waiting (the task card shows 等待写出). A wait beyond
/// the threshold (here 300 ms instead of 5 minutes) records no task event:
/// waiting for this program's own output slot is ordinary queueing (D1c, L4
/// 2026-10-18: one write took 306 s, so the next item always waited longer
/// than 5 minutes). A Store pause reaches the waiting writer within one second
/// and leaves the item paused, not failed. After the slot is free, a resume
/// publishes the output.
#[test]
fn production_route_v2_waits_for_the_output_slot_visibly_and_pauses() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let label = "route-writer-slot-wait";
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
    let inner = route_worker(server.url.clone(), &case, &work.item)
        .with_nisar_writer_long_wait_after_for_test(Duration::from_millis(300));
    let held_slot = inner.nisar_writer_output_slots_for_test().hold_for_test();
    let worker = Arc::new(ProgressRows {
        inner,
        rows: Arc::clone(&rows),
    });
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(5));
    scheduler.wake();
    let deadline = Instant::now() + STEP_TIMEOUT;
    while waiting_items(&store, &work.task.task_id) == 0 {
        assert!(
            Instant::now() < deadline,
            "the item never waited: {:?}",
            rows.lock().unwrap()
        );
        thread::sleep(Duration::from_millis(5));
    }
    {
        let rows = rows.lock().unwrap();
        let last = rows.last().unwrap();
        assert_eq!(
            last.status,
            insar_core::DownloadItemStatus::Queued,
            "{rows:?}"
        );
        assert_eq!(last.bytes_per_second, 0.0);
        assert!(
            last.bytes_completed > 0 && last.total_bytes.is_none(),
            "{rows:?}"
        );
    }
    // Well beyond the 300 ms threshold: the wait is reported only on the
    // local diagnostic stream.
    thread::sleep(Duration::from_millis(900));
    assert_eq!(waiting_items(&store, &work.task.task_id), 1);
    let output_wait_events = store
        .lock()
        .unwrap()
        .list_task_events(&work.task.task_id)
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type.starts_with("nisar_output_wait_"))
        .count();
    assert_eq!(output_wait_events, 0, "ordinary queueing records no event");
    let paused_at = Instant::now();
    item_command(&store, &work, TaskStateCommandKind::PauseItems);
    loop {
        let snapshot = store
            .lock()
            .unwrap()
            .load_download_snapshot(&work.task.task_id)
            .unwrap();
        if let Some(result) = snapshot.results.first() {
            assert_eq!(result.outcome, DownloadOutcome::Interrupted, "{snapshot:?}");
            assert!(
                result.failure.is_none(),
                "a pause is not a failure: {snapshot:?}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the paused writer never returned"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        paused_at.elapsed() < Duration::from_secs(1),
        "the waiting writer stopped within one second of the pause: {:?}",
        paused_at.elapsed()
    );
    let runtime = store
        .lock()
        .unwrap()
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap()
        .runtime;
    assert!(
        runtime.paused_item_ids.contains(&work.item.item_id),
        "{runtime:?}"
    );
    assert!(runtime.failed_item_ids.is_empty(), "{runtime:?}");
    drop(held_slot);
    item_command(&store, &work, TaskStateCommandKind::ResumeItems);
    wait_for_route_completed(&store, &work.task.task_id);
    scheduler.shutdown();
    drop(server.finish());
}
