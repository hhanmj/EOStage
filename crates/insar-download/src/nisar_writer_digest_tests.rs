//! D2 of the downloader enhancement plan (2026-10-18): the writer digests the
//! completed science material twice, at its entry and once more right before
//! it publishes; every other check compares the identities of its fixed open
//! handles and of the workspace child (volume serial, file id, size and
//! modification marker) without reading science bytes. The ledger counts the
//! digested bytes per path. Integrity is not relaxed: a byte changed with its
//! timestamps restored, a truncated or replaced part and bytes that change
//! under an unchanged identity are all refused. Digests stop at a pause.
use super::writer_admission::WriterInput;
use super::writer_baseline::{
    normalized_baseline, science_complete_fixture, BaselineRun, ScienceCompleteFixture,
    EDGE_CHUNK_BASELINE, EDGE_CHUNK_LAYOUT,
};
use super::*;
use crate::nisar_science_transfer_v2::{
    completed_material_hash_ledger, final_digest_view_for_test, material_digest_hook_for_test,
    NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as ScienceError,
};
use crate::nisar_subset_writer::tests::writer_with_anchor;
use crate::nisar_subset_writer::{
    NisarSubsetWriteError as Error, NisarSubsetWriteResultV2, WriterFailpointV2,
};
use crate::nisar_writer_timings_v2::{last_writer_timings_for_test, NisarWriterStageV2 as Stage};
use crate::DestinationLeaseController;

const ENTRY: &str = "writer_science_revalidation";
const IDENTITY: &str = "writer_part_material";
const FINAL: &str = "writer_final_digest";
/// The read buffer of the material digest (`SCIENCE_READ_BUFFER_BYTES`).
const DIGEST_CHUNK_BYTES: u64 = 64 * 1024;

fn workspace_key(fixture: &ScienceCompleteFixture) -> String {
    fixture
        .bootstrap
        .science_workspace_v2()
        .bound_workspace_identity_v2()
        .unwrap()
        .0
}

fn completed_bytes(fixture: &ScienceCompleteFixture) -> u64 {
    fixture
        .science
        .completed_ranges()
        .iter()
        .map(|(start, end)| end - start)
        .sum()
}

fn digest_chunks(fixture: &ScienceCompleteFixture) -> u64 {
    fixture
        .science
        .completed_ranges()
        .iter()
        .map(|(start, end)| (end - start).div_ceil(DIGEST_CHUNK_BYTES))
        .sum()
}

/// Bytes digested per site for this workspace: (entry, identity, final).
fn digested(workspace: &str) -> (u64, u64, u64) {
    (
        completed_material_hash_ledger::hashed_bytes(workspace, ENTRY),
        completed_material_hash_ledger::hashed_bytes(workspace, IDENTITY),
        completed_material_hash_ledger::hashed_bytes(workspace, FINAL),
    )
}

fn delta(after: (u64, u64, u64), before: (u64, u64, u64)) -> (u64, u64, u64) {
    (after.0 - before.0, after.1 - before.1, after.2 - before.2)
}

struct Case {
    fixture: ScienceCompleteFixture,
    anchor: ScienceCheckpointAnchorFixture,
    output: PathBuf,
    anchor_path: PathBuf,
}

fn case(label: &str) -> Case {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(label, &["HH"], Some(EDGE_CHUNK_LAYOUT), &anchor);
    let output = fixture.case.root.0.join(format!("{label}-output"));
    let anchor_path = fixture.case.root.0.join(format!("{label}-anchor.json"));
    Case {
        fixture,
        anchor,
        output,
        anchor_path,
    }
}

impl Case {
    fn write(
        &self,
        control: &WorkerControl,
        failpoint: WriterFailpointV2,
    ) -> Result<NisarSubsetWriteResultV2, Error> {
        let writer = writer_with_anchor(
            &self.output,
            &self.anchor_path,
            DestinationLeaseController::new_for_qualification_test(),
        );
        WriterInput::of(&self.fixture, &self.anchor).write_observed(
            &writer,
            control,
            failpoint,
            &mut |_| {},
        )
    }

    fn anchor_state(&self) -> Option<String> {
        let text = fs::read_to_string(&self.anchor_path).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        value["state"].as_str().map(str::to_owned)
    }

    fn part_path(&self) -> PathBuf {
        self.fixture.bootstrap.science_workspace_v2().part_path()
    }

    fn first_completed_offset(&self) -> u64 {
        self.fixture.science.completed_ranges()[0].0
    }

    fn published_h5_count(&self) -> usize {
        fs::read_dir(&self.output).map_or(0, |entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("h5")
                })
                .count()
        })
    }
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

/// Flips one byte of `path` at `offset` and puts its timestamps back, so
/// its size, times and identity look unchanged.
fn flip_byte_keeping_times(path: &Path, offset: u64) {
    let before = fs::metadata(path).unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let mut byte = [0_u8; 1];
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.read_exact(&mut byte).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&[byte[0] ^ 0x5a]).unwrap();
    file.sync_all().unwrap();
    let times = fs::FileTimes::new()
        .set_modified(before.modified().unwrap())
        .set_accessed(before.accessed().unwrap());
    #[cfg(windows)]
    let times = {
        use std::os::windows::fs::FileTimesExt;
        times.set_created(before.created().unwrap())
    };
    file.set_times(times).unwrap();
    drop(file);
    let after = fs::metadata(path).unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
}

/// The new-candidate path digests the completed material once at the writer
/// entry and once before promotion; the five checks in between (around the
/// metadata and window reads, before the candidate, after it is written and
/// after it is validated) digest nothing. Before D2 this path digested it
/// 1 + 7 x 3 = 22 times. The run reports its stage timings.
#[test]
fn writer_v2_digests_the_completed_material_twice_per_new_candidate() {
    let case = case("writer-v2-digest-twice");
    let workspace = workspace_key(&case.fixture);
    let total = completed_bytes(&case.fixture);
    assert!(total > 0);
    let before = digested(&workspace);
    let written = case
        .write(&WorkerControl::default(), WriterFailpointV2::None)
        .unwrap();
    assert_eq!(
        delta(digested(&workspace), before),
        (total, 0, total),
        "entry, identity checks, final digest"
    );
    assert_edge_baseline(&case.fixture, &written);
    let timings = last_writer_timings_for_test().expect("the run reported its timings");
    assert_eq!(
        timings.stages(),
        vec![
            Stage::Admission,
            Stage::EntryRevalidation,
            Stage::MetadataCapture,
            Stage::WindowCapture,
            Stage::CandidateWrite,
            Stage::CandidateValidation,
            Stage::FinalRevalidation,
            Stage::Publication,
        ]
    );
    let line = timings.diagnostic_line(
        &case.fixture.work.task.task_id,
        &case.fixture.work.item.item_id,
        "completed",
    );
    eprintln!("{line} completed_material_bytes={total}");
    assert!(line.contains(" final_revalidation_ms="), "{line}");
}

/// Recovering a published artifact also digests the material twice: at the
/// entry and before the recovered artifact is completed.
#[test]
fn writer_v2_artifact_recovery_digests_the_completed_material_twice() {
    let case = case("writer-v2-digest-recovery");
    let workspace = workspace_key(&case.fixture);
    let total = completed_bytes(&case.fixture);
    assert_eq!(
        case.write(
            &WorkerControl::default(),
            WriterFailpointV2::AfterArtifactPromotion
        )
        .unwrap_err(),
        Error::InjectedFailure
    );
    let before = digested(&workspace);
    let written = case
        .write(&WorkerControl::default(), WriterFailpointV2::None)
        .unwrap();
    assert_eq!(delta(digested(&workspace), before), (total, 0, total));
    assert_eq!(
        last_writer_timings_for_test().unwrap().stages(),
        vec![
            Stage::Admission,
            Stage::EntryRevalidation,
            Stage::MetadataCapture,
            Stage::WindowCapture,
            Stage::ArtifactRecovery,
            Stage::FinalRevalidation,
            Stage::Publication,
        ]
    );
    assert_edge_baseline(&case.fixture, &written);
}

/// A kept CandidateValidated candidate is revalidated and promoted with the
/// same two digests. Windows only: only Windows reuses a kept
/// CandidateValidated candidate (the writer's candidate reuse is
/// `cfg(windows)`; elsewhere the candidate is rebuilt from the verified
/// science, which the new-candidate test covers).
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_kept_candidate_recovery_digests_the_completed_material_twice() {
    let case = case("writer-v2-digest-kept");
    let workspace = workspace_key(&case.fixture);
    let total = completed_bytes(&case.fixture);
    assert_eq!(
        case.write(
            &WorkerControl::default(),
            WriterFailpointV2::AfterValidationBeforePromotion
        )
        .unwrap_err(),
        Error::InjectedFailure
    );
    assert_eq!(case.anchor_state().as_deref(), Some("CandidateValidated"));
    let before = digested(&workspace);
    let written = case
        .write(&WorkerControl::default(), WriterFailpointV2::None)
        .unwrap();
    assert_eq!(delta(digested(&workspace), before), (total, 0, total));
    assert_edge_baseline(&case.fixture, &written);
}

/// A byte changed while the writer was not running, with the file's size and
/// timestamps put back, leaves every identity fact unchanged: the entry
/// digest refuses it and nothing is written.
#[test]
fn writer_v2_entry_digest_refuses_a_byte_changed_with_its_timestamps_restored() {
    let case = case("writer-v2-digest-entry-tamper");
    let part = case.part_path();
    let offset = case.first_completed_offset();
    flip_byte_keeping_times(&part, offset);
    assert_eq!(
        case.write(&WorkerControl::default(), WriterFailpointV2::None)
            .unwrap_err(),
        Error::IncompleteScienceAuthority
    );
    assert_eq!(case.published_h5_count(), 0);
    assert!(writer_v2_candidate_paths(&case.output).is_empty());
    flip_byte_keeping_times(&part, offset);
    let written = case
        .write(&WorkerControl::default(), WriterFailpointV2::None)
        .unwrap();
    assert_edge_baseline(&case.fixture, &written);
}

/// Review of D1/D2 (downloader enhancement D1b, P2-6): a part truncated into
/// its first completed Range and extended back to its size, with its
/// timestamps restored, keeps its size and times, so no identity fact shows
/// the change; the entry digest refuses it and nothing is written.
#[test]
fn writer_v2_entry_digest_refuses_a_part_truncated_and_restored_with_its_timestamps() {
    let case = case("writer-v2-digest-truncate-restore");
    let part = case.part_path();
    let backup = case.fixture.case.root.0.join("part-backup.bin");
    fs::copy(&part, &backup).unwrap();
    let before = fs::metadata(&part).unwrap();
    {
        let file = OpenOptions::new().write(true).open(&part).unwrap();
        file.set_len(case.first_completed_offset()).unwrap();
        file.set_len(before.len()).unwrap();
        file.sync_all().unwrap();
        file.set_times(restored_times(&before)).unwrap();
    }
    let after = fs::metadata(&part).unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(
        case.write(&WorkerControl::default(), WriterFailpointV2::None)
            .unwrap_err(),
        Error::IncompleteScienceAuthority
    );
    assert_eq!(case.published_h5_count(), 0);
    assert!(writer_v2_candidate_paths(&case.output).is_empty());
    fs::copy(&backup, &part).unwrap();
    OpenOptions::new()
        .write(true)
        .open(&part)
        .unwrap()
        .set_times(restored_times(&before))
        .unwrap();
    let written = case
        .write(&WorkerControl::default(), WriterFailpointV2::None)
        .unwrap();
    assert_edge_baseline(&case.fixture, &written);
}

fn restored_times(metadata: &fs::Metadata) -> fs::FileTimes {
    let times = fs::FileTimes::new()
        .set_modified(metadata.modified().unwrap())
        .set_accessed(metadata.accessed().unwrap());
    #[cfg(windows)]
    let times = {
        use std::os::windows::fs::FileTimesExt;
        times.set_created(metadata.created().unwrap())
    };
    times
}

/// Bytes the writer reads that differ from the checkpoint while every
/// identity fact stays the same (here: the final digest is pointed at a copy
/// of the part with one byte changed) are refused right before promotion.
/// The validated candidate is kept; the next run publishes the baseline.
/// Windows only for the kept-candidate assertions (see the kept-candidate
/// test); the refusal itself does not depend on the platform.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_final_digest_refuses_bytes_changed_under_an_unchanged_identity() {
    let case = case("writer-v2-digest-final-tamper");
    let workspace = workspace_key(&case.fixture);
    let total = completed_bytes(&case.fixture);
    let altered = case.fixture.case.root.0.join("altered-part.bin");
    fs::copy(case.part_path(), &altered).unwrap();
    flip_byte_keeping_times(&altered, case.first_completed_offset());
    let before = digested(&workspace);
    {
        let _view = final_digest_view_for_test::install(altered.clone());
        assert_eq!(
            case.write(&WorkerControl::default(), WriterFailpointV2::None)
                .unwrap_err(),
            Error::IncompleteScienceAuthority
        );
    }
    let (entry, identity, final_digest) = delta(digested(&workspace), before);
    assert_eq!((entry, identity), (total, 0), "the entry digest passed");
    assert!(final_digest > 0, "the final digest read the changed bytes");
    assert_eq!(case.published_h5_count(), 0, "nothing was published");
    assert_eq!(case.anchor_state().as_deref(), Some("CandidateValidated"));
    assert_eq!(writer_v2_candidate_paths(&case.output).len(), 1);
    let written = case
        .write(&WorkerControl::default(), WriterFailpointV2::None)
        .unwrap();
    assert_edge_baseline(&case.fixture, &written);
}

/// A pause during the entry digest stops it between two reads and is a pause.
#[test]
fn writer_v2_pause_during_the_entry_digest_stops_it_and_is_a_pause() {
    let case = case("writer-v2-digest-entry-pause");
    assert!(
        digest_chunks(&case.fixture) >= 3,
        "the fixture has several reads"
    );
    let control = WorkerControl::default();
    let reads = Arc::new(AtomicU64::new(0));
    {
        let (hook_control, hook_reads) = (control.clone(), Arc::clone(&reads));
        let _hook = material_digest_hook_for_test::install(move || {
            if hook_reads.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
                hook_control.set_paused_for_test(true);
            }
        });
        assert_eq!(
            case.write(&control, WriterFailpointV2::None).unwrap_err(),
            Error::Cancelled
        );
    }
    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "the digest stopped at the read after the pause"
    );
    assert!(writer_v2_candidate_paths(&case.output).is_empty());
    let written = case
        .write(&WorkerControl::default(), WriterFailpointV2::None)
        .unwrap();
    assert_edge_baseline(&case.fixture, &written);
}

/// A pause during the final digest stops it and is a pause; the validated
/// candidate is kept and the next run publishes it. Windows only for the
/// kept-candidate assertions (see the kept-candidate test).
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_pause_during_the_final_digest_keeps_the_validated_candidate() {
    let case = case("writer-v2-digest-final-pause");
    let entry_reads = digest_chunks(&case.fixture);
    let control = WorkerControl::default();
    let reads = Arc::new(AtomicU64::new(0));
    {
        let (hook_control, hook_reads) = (control.clone(), Arc::clone(&reads));
        let _hook = material_digest_hook_for_test::install(move || {
            if hook_reads.fetch_add(1, Ordering::SeqCst) + 1 == entry_reads + 2 {
                hook_control.set_paused_for_test(true);
            }
        });
        assert_eq!(
            case.write(&control, WriterFailpointV2::None).unwrap_err(),
            Error::Cancelled
        );
    }
    assert_eq!(reads.load(Ordering::SeqCst), entry_reads + 2);
    assert_eq!(case.anchor_state().as_deref(), Some("CandidateValidated"));
    assert_eq!(writer_v2_candidate_paths(&case.output).len(), 1);
    let written = case
        .write(&WorkerControl::default(), WriterFailpointV2::None)
        .unwrap();
    assert_edge_baseline(&case.fixture, &written);
}

/// The science executor's continuation digest (a continue of a complete
/// item) stops at a pause too, instead of reading every completed Range
/// first; without a pause the same continuation succeeds.
#[test]
fn science_continuation_digest_stops_at_a_pause() {
    let case = case("science-digest-continuation-pause");
    let physical = case.fixture.completed.physical_execution_plan();
    let asset = science_asset(
        &case.fixture.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let authority = case.fixture.authority(&case.anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(
        super::writer_baseline::baseline_science_http(),
        asset,
    )
    .unwrap();
    let control = WorkerControl::default();
    let reads = Arc::new(AtomicU64::new(0));
    {
        let (hook_control, hook_reads) = (control.clone(), Arc::clone(&reads));
        let _hook = material_digest_hook_for_test::install(move || {
            if hook_reads.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
                hook_control.set_paused_for_test(true);
            }
        });
        assert_eq!(
            executor
                .execute(
                    &case.fixture.work,
                    &case.fixture.completed,
                    &authority,
                    &control
                )
                .unwrap_err(),
            ScienceError::Paused
        );
    }
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    let resumed = executor
        .execute(
            &case.fixture.work,
            &case.fixture.completed,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap();
    assert!(resumed.complete());
}

/// Review of D1/D2 (downloader enhancement D1b, P2-4): the scheduler clears
/// the pause of a still-running worker when the item is resumed at once. A
/// pause the entry digest observed stays a pause even though the flag is
/// clear again when the writer maps the stop; it used to become the
/// non-retryable IncompleteScienceAuthority.
#[test]
fn writer_v2_pause_then_immediate_resume_during_the_entry_digest_is_a_pause() {
    let case = case("writer-v2-digest-entry-race");
    let control = WorkerControl::default();
    let reads = Arc::new(AtomicU64::new(0));
    {
        let (pause, resume, hook_reads) = (control.clone(), control.clone(), Arc::clone(&reads));
        let _hook = material_digest_hook_for_test::install_with_after_stop(
            move || {
                if hook_reads.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
                    pause.set_paused_for_test(true);
                }
            },
            move || resume.set_paused_for_test(false),
        );
        assert_eq!(
            case.write(&control, WriterFailpointV2::None).unwrap_err(),
            Error::Cancelled
        );
    }
    assert!(
        !control.is_paused(),
        "the item was resumed while the writer stopped"
    );
    assert!(writer_v2_candidate_paths(&case.output).is_empty());
    let written = case
        .write(&WorkerControl::default(), WriterFailpointV2::None)
        .unwrap();
    assert_edge_baseline(&case.fixture, &written);
}

/// The same race in the final digest: a pause, not a failure, and the
/// validated candidate is kept. Windows only: only Windows reuses a kept
/// CandidateValidated candidate (Linux rebuilds it from the science).
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_pause_then_immediate_resume_during_the_final_digest_keeps_the_candidate() {
    let case = case("writer-v2-digest-final-race");
    let entry_reads = digest_chunks(&case.fixture);
    let control = WorkerControl::default();
    let reads = Arc::new(AtomicU64::new(0));
    {
        let (pause, resume, hook_reads) = (control.clone(), control.clone(), Arc::clone(&reads));
        let _hook = material_digest_hook_for_test::install_with_after_stop(
            move || {
                if hook_reads.fetch_add(1, Ordering::SeqCst) + 1 == entry_reads + 2 {
                    pause.set_paused_for_test(true);
                }
            },
            move || resume.set_paused_for_test(false),
        );
        assert_eq!(
            case.write(&control, WriterFailpointV2::None).unwrap_err(),
            Error::Cancelled
        );
    }
    assert_eq!(case.anchor_state().as_deref(), Some("CandidateValidated"));
    assert_eq!(writer_v2_candidate_paths(&case.output).len(), 1);
    let written = case
        .write(&WorkerControl::default(), WriterFailpointV2::None)
        .unwrap();
    assert_edge_baseline(&case.fixture, &written);
}

/// L4 2026-10-18 (downloader enhancement D1c): a pause while the writer
/// revalidated its entry took 28.6 s to take effect (WRITER_TIMINGS stopped,
/// entry_revalidation 41,373 ms). The planning revalidation digested the
/// planning material (195 MB per item) and looked up every chunk of the
/// selected datasets without asking for a stop. Here every step of those two
/// loops takes 100 ms, as on a real source; a pause must still take effect
/// within 2 s, and it is a pause.
#[test]
fn writer_v2_pause_during_entry_revalidation_takes_effect_within_two_seconds() {
    let case = case("writer-v2-entry-pause-latency");
    let control = WorkerControl::default();
    let steps = Arc::new(AtomicU64::new(0));
    let paused_at = Arc::new(Mutex::new(None::<Instant>));
    let pauser = {
        let (control, steps, paused_at) =
            (control.clone(), Arc::clone(&steps), Arc::clone(&paused_at));
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(120);
            while steps.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            thread::sleep(Duration::from_millis(300));
            *paused_at.lock().unwrap() = Some(Instant::now());
            control.set_paused_for_test(true);
        })
    };
    let slow_step = {
        let steps = Arc::clone(&steps);
        move || {
            steps.fetch_add(1, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(100));
        }
    };
    let result = {
        let _slow = crate::hdf5_range_transfer::long_step_hook_for_test::install(slow_step);
        case.write(&control, WriterFailpointV2::None)
    };
    let returned_at = Instant::now();
    pauser.join().unwrap();
    let paused_at = paused_at.lock().unwrap().expect("the pause was set");
    let latency = returned_at.saturating_duration_since(paused_at);
    eprintln!(
        "entry revalidation pause latency {latency:?} after {} slow steps",
        steps.load(Ordering::SeqCst)
    );
    assert_eq!(result.unwrap_err(), Error::Cancelled, "a pause is a pause");
    assert!(
        latency < Duration::from_secs(2),
        "the pause took effect within 2 s: {latency:?}"
    );
    assert!(writer_v2_candidate_paths(&case.output).is_empty());
}
