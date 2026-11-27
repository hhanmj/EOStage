//! Independent review of slice 6 (P1, fixed in Issue #8 slice 7): once the
//! writer anchored a candidate as CandidateValidated, that candidate is the
//! recovery state. A pause, a failed promotion or a transient read error
//! after that point leaves it in place, so the next run reuses it instead of
//! finding the anchor without its candidate
//! (NISAR_SUBSET_V2_WRITER_RECOVERY_INVALID, not retryable, item locked). A
//! pause anywhere in the writer is reported as a pause.
use super::writer_baseline::{
    normalized_baseline, science_complete_fixture, BaselineRun, ScienceCompleteFixture,
    EDGE_CHUNK_BASELINE, EDGE_CHUNK_LAYOUT,
};
use super::*;
use crate::nisar_subset_stream_v2::{
    V2AfterTileHookForTest, V2StreamBudgetForTest, V2StreamPass, V2VerifyReadFaultForTest,
};
use crate::nisar_subset_writer::{
    NisarSubsetWriteError as Error, V2CandidateFaultForTest, WriterFailpointV2,
};

fn write(
    fixture: &ScienceCompleteFixture,
    anchor: &ScienceCheckpointAnchorFixture,
    output: &Path,
    control: &WorkerControl,
    failpoint: WriterFailpointV2,
) -> Result<crate::nisar_subset_writer::NisarSubsetWriteResultV2, Error> {
    let authority = fixture.authority(anchor);
    crate::nisar_subset_writer::tests::writer(output).write_and_validate_v2_with_failpoint_for_test(
        &fixture.work,
        &fixture.completed,
        &fixture.science,
        &authority,
        control,
        failpoint,
    )
}

fn assert_resumes_to_the_baseline(
    fixture: &ScienceCompleteFixture,
    anchor: &ScienceCheckpointAnchorFixture,
    output: &Path,
) {
    let written = write(
        fixture,
        anchor,
        output,
        &WorkerControl::default(),
        WriterFailpointV2::None,
    )
    .unwrap();
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

fn assert_candidate_kept_as_validated(output: &Path) {
    assert_eq!(
        crate::nisar_subset_writer::tests::recovery_anchor_state_for_test(output),
        Some("CandidateValidated")
    );
    assert_eq!(
        writer_v2_candidate_paths(output).len(),
        1,
        "the validated candidate is the recovery state and is kept"
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_pause_after_the_validated_anchor_resumes_from_the_kept_candidate() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-resume-pause",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    let output = fixture.case.root.0.join("writer-v2-resume-pause-output");
    let error = write(
        &fixture,
        &anchor,
        &output,
        &WorkerControl::default(),
        WriterFailpointV2::PauseAfterValidatedAnchor,
    )
    .unwrap_err();
    assert_eq!(error, Error::Cancelled, "a pause is reported as a pause");
    assert_candidate_kept_as_validated(&output);
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_failed_promotion_is_retried_from_the_kept_candidate() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-resume-promotion",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    let output = fixture
        .case
        .root
        .0
        .join("writer-v2-resume-promotion-output");
    assert_eq!(
        write(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::AtomicPromotionFailureBeforeRename,
        )
        .unwrap_err(),
        Error::AtomicPromotion
    );
    assert_candidate_kept_as_validated(&output);
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_pause_or_read_error_while_revalidating_a_kept_candidate_is_retryable() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-resume-reuse",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    let output = fixture.case.root.0.join("writer-v2-resume-reuse-output");
    // A candidate kept at CandidateValidated, as after a crash before promotion.
    assert_eq!(
        write(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::AfterValidationBeforePromotion,
        )
        .unwrap_err(),
        Error::InjectedFailure
    );
    assert_candidate_kept_as_validated(&output);
    let _budget = V2StreamBudgetForTest::set(1);
    // A pause while the kept candidate is revalidated tile by tile.
    let control = WorkerControl::default();
    let hook_control = control.clone();
    let hook = V2AfterTileHookForTest::set(move |pass, index| {
        if pass == V2StreamPass::Verify && index == 2 {
            hook_control.set_paused_for_test(true);
        }
    });
    assert_eq!(
        write(
            &fixture,
            &anchor,
            &output,
            &control,
            WriterFailpointV2::None
        )
        .unwrap_err(),
        Error::Cancelled
    );
    drop(hook);
    assert_candidate_kept_as_validated(&output);
    // A transient read error while it is revalidated: a local, retryable
    // read failure, not a bit mismatch and not a recovery refusal.
    {
        let _fault = V2VerifyReadFaultForTest::arm(1);
        let error = write(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::None,
        )
        .unwrap_err();
        assert_eq!(error, Error::Persistence);
        assert!(
            crate::nisar_output_diagnostics_v2::nisar_v2_writer_failure_zh(error, None).retryable
        );
    }
    assert_candidate_kept_as_validated(&output);
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

#[test]
fn writer_v2_a_paused_worker_entering_the_writer_is_reported_as_paused() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-entry-pause",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    let output = fixture.case.root.0.join("writer-v2-entry-pause-output");
    let control = WorkerControl::default();
    control.set_paused_for_test(true);
    assert_eq!(
        write(
            &fixture,
            &anchor,
            &output,
            &control,
            WriterFailpointV2::None
        )
        .unwrap_err(),
        Error::Cancelled
    );
    assert!(writer_v2_candidate_paths(&output).is_empty());
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

// Review of slice 7 (Issue #8 slice 7b, P1-1, P1-2, P2-1 to P2-3): after
// CandidateValidated the candidate is kept by default, including when the
// anchor CAS reports a stop or a persistence failure (unknown outcome); only
// a candidate whose size or SHA-256 contradicts the anchor is removed, and a
// candidate that stays unreadable is rebuilt from the verified science. A
// pause during artifact recovery is a pause. A held output lock was a
// retryable local failure until downloader enhancement D1 (2026-10-18); the
// writer now waits for it (see the admission tests).

fn fresh_fixture(
    label: &str,
    anchor: &ScienceCheckpointAnchorFixture,
) -> (ScienceCompleteFixture, PathBuf) {
    let fixture = science_complete_fixture(label, &["HH"], Some(EDGE_CHUNK_LAYOUT), anchor);
    let output = fixture.case.root.0.join(format!("{label}-output"));
    (fixture, output)
}

#[cfg(any(windows, target_os = "macos"))]
fn keep_a_validated_candidate(
    fixture: &ScienceCompleteFixture,
    anchor: &ScienceCheckpointAnchorFixture,
    output: &Path,
) {
    assert_eq!(
        write(
            fixture,
            anchor,
            output,
            &WorkerControl::default(),
            WriterFailpointV2::AfterValidationBeforePromotion,
        )
        .unwrap_err(),
        Error::InjectedFailure
    );
    assert_candidate_kept_as_validated(output);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_stop_reported_after_the_validated_anchor_commit_keeps_the_candidate() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let (fixture, output) = fresh_fixture("writer-v2-keep-cas-after", &anchor);
    assert_eq!(
        write(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::ValidatedAnchorStoppedAfterCommit,
        )
        .unwrap_err(),
        Error::Cancelled
    );
    assert_candidate_kept_as_validated(&output);
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_stop_reported_before_the_validated_anchor_commit_keeps_the_candidate() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let (fixture, output) = fresh_fixture("writer-v2-keep-cas-before", &anchor);
    assert_eq!(
        write(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::ValidatedAnchorStoppedBeforeCommit,
        )
        .unwrap_err(),
        Error::Cancelled
    );
    assert_eq!(
        crate::nisar_subset_writer::tests::recovery_anchor_state_for_test(&output),
        Some("CandidateBuilding")
    );
    assert_eq!(writer_v2_candidate_paths(&output).len(), 1);
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_unreadable_hash_of_a_kept_candidate_is_retryable_and_keeps_it() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let (fixture, output) = fresh_fixture("writer-v2-keep-hash-read", &anchor);
    keep_a_validated_candidate(&fixture, &anchor, &output);
    let error = {
        let _fault = V2CandidateFaultForTest::hash_read();
        write(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::None,
        )
        .unwrap_err()
    };
    assert_eq!(error, Error::Persistence);
    assert_candidate_kept_as_validated(&output);
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_failed_hdf5_open_of_a_kept_candidate_is_retryable_and_keeps_it() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let (fixture, output) = fresh_fixture("writer-v2-keep-hdf5-open", &anchor);
    keep_a_validated_candidate(&fixture, &anchor, &output);
    let error = {
        let _fault = V2CandidateFaultForTest::hdf5_open();
        write(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::None,
        )
        .unwrap_err()
    };
    assert_eq!(error, Error::Persistence);
    assert_candidate_kept_as_validated(&output);
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_part_rehash_failure_after_revalidating_a_kept_candidate_keeps_it() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let (fixture, output) = fresh_fixture("writer-v2-keep-part-rehash", &anchor);
    keep_a_validated_candidate(&fixture, &anchor, &output);
    assert_eq!(
        write(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::WritePartAfterCandidateValidation,
        )
        .unwrap_err(),
        Error::IncompleteScienceAuthority
    );
    assert_candidate_kept_as_validated(&output);
    #[cfg(not(windows))]
    {
        use std::io::{Read, Seek, SeekFrom, Write};
        let part_path = fixture.bootstrap.science_workspace_v2().part_path();
        let offset = fixture
            .completed
            .physical_execution_plan()
            .science_ranges()
            .first()
            .unwrap()
            .start();
        let mut part = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&part_path)
            .unwrap();
        part.seek(SeekFrom::Start(offset)).unwrap();
        let mut byte = [0_u8; 1];
        part.read_exact(&mut byte).unwrap();
        byte[0] ^= 0x5a;
        part.seek(SeekFrom::Start(offset)).unwrap();
        part.write_all(&byte).unwrap();
        part.sync_all().unwrap();
    }
    // On Windows the OS refuses the write while the part is pinned; on Unix
    // verify_part_identity catches the mutation and once the part is intact
    // again the next run resumes from the kept candidate.
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn writer_v2_rebuilds_a_kept_candidate_that_stays_unreadable() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let (fixture, output) = fresh_fixture("writer-v2-keep-unreadable", &anchor);
    keep_a_validated_candidate(&fixture, &anchor, &output);
    let _budget = V2StreamBudgetForTest::set(1);
    {
        let _fault = V2VerifyReadFaultForTest::arm(1);
        assert_eq!(
            write(
                &fixture,
                &anchor,
                &output,
                &WorkerControl::default(),
                WriterFailpointV2::None,
            )
            .unwrap_err(),
            Error::Persistence
        );
    }
    assert_candidate_kept_as_validated(&output);
    {
        let _fault = V2VerifyReadFaultForTest::arm(1);
        assert_eq!(
            write(
                &fixture,
                &anchor,
                &output,
                &WorkerControl::default(),
                WriterFailpointV2::None,
            )
            .unwrap_err(),
            Error::Persistence
        );
    }
    // Unreadable twice in a row: the candidate is removed and the anchor is
    // back at CandidateBuilding, so the next run rewrites it from the
    // verified science without any download.
    assert_eq!(
        crate::nisar_subset_writer::tests::recovery_anchor_state_for_test(&output),
        Some("CandidateBuilding")
    );
    assert!(writer_v2_candidate_paths(&output).is_empty());
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

#[test]
fn writer_v2_pause_during_artifact_recovery_revalidation_is_a_pause() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let (fixture, output) = fresh_fixture("writer-v2-recovery-pause", &anchor);
    assert_eq!(
        write(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::AfterArtifactPromotion,
        )
        .unwrap_err(),
        Error::InjectedFailure
    );
    assert_eq!(
        write(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::PauseBeforeArtifactRecoveryRevalidation,
        )
        .unwrap_err(),
        Error::Cancelled
    );
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}

/// Downloader enhancement D1 (2026-10-18), requirement change: a writer that
/// finds the directory lock of an earlier build held (that build takes it
/// exclusively while it writes) waits for it instead of failing with
/// NISAR_SUBSET_V2_OUTPUT_BUSY; a pause stops the wait within one second and
/// is reported as a pause, never as an unsafe directory or a failure.
#[test]
fn writer_v2_waits_for_an_earlier_build_holding_the_directory_lock() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let (fixture, output) = fresh_fixture("writer-v2-output-busy", &anchor);
    fs::create_dir_all(&output).unwrap();
    let held = crate::hdf5_range_transfer::acquire_hdf5_workspace_lock(
        &output.join(".nisar-subset-writer-v2.lock"),
    )
    .unwrap();
    let control = WorkerControl::default();
    let input = super::writer_admission::WriterInput::of(&fixture, &anchor);
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let worker_control = control.clone();
        let output = &output;
        scope.spawn(move || {
            let writer = crate::nisar_subset_writer::tests::writer(output);
            result_tx
                .send(input.write(&writer, &worker_control))
                .unwrap();
        });
        assert!(
            result_rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .is_err(),
            "the writer waits while the earlier build holds the directory lock"
        );
        let paused_at = std::time::Instant::now();
        control.set_paused_for_test(true);
        let error = result_rx
            .recv_timeout(std::time::Duration::from_secs(120))
            .expect("the waiting writer returns")
            .unwrap_err();
        assert!(
            paused_at.elapsed() < std::time::Duration::from_secs(1),
            "a pause stops the wait within one second: {:?}",
            paused_at.elapsed()
        );
        assert_eq!(error, Error::Cancelled, "a pause is a pause");
    });
    assert!(writer_v2_candidate_paths(&output).is_empty());
    drop(held);
    assert_resumes_to_the_baseline(&fixture, &anchor, &output);
}
