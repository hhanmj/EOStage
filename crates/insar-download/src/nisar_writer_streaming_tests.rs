//! Streaming V2 writer (Issue #8, slice 6): bounded tiles, bitwise
//! validation, cancellation between tiles, free-space admission and recovery
//! of a candidate or artifact written with a different tile budget. Every
//! fixture is loopback-only and checked against the characterization baseline.
use super::writer_baseline::{
    baseline_science_value, normalized_baseline, run_baseline_writer, science_complete_fixture,
    science_complete_fixture_with, BaselineRun, ScienceCompleteFixture, DUAL_POL_BASELINE,
    DUAL_POL_LAYOUT, EDGE_CHUNK_BASELINE, EDGE_CHUNK_LAYOUT, LARGE_CHUNK_BASELINE,
    LARGE_CHUNK_LAYOUT,
};
use super::*;
use crate::nisar_subset_stream_v2::{
    reset_v2_stream_observation_for_test, v2_stream_observation_for_test, V2AfterTileHookForTest,
    V2AvailableDiskForTest, V2StreamBudgetForTest, V2StreamPass,
};
use crate::nisar_subset_writer::{NisarSubsetWriteError as Error, WriterFailpointV2};

fn assert_matches_baseline(run: &BaselineRun, expected: (&str, &str), label: &str) {
    let normalized = normalized_baseline(run);
    assert_eq!(normalized.artifact_sha256, expected.0, "{label} artifact");
    assert_eq!(normalized.sidecar_sha256, expected.1, "{label} sidecar");
}

fn write_fixture(
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

fn baseline_run_from(
    written: &crate::nisar_subset_writer::NisarSubsetWriteResultV2,
    fixture: &ScienceCompleteFixture,
) -> BaselineRun {
    let manifest = serde_json::to_value(written.manifest()).unwrap();
    BaselineRun {
        artifact_sha256: test_file_sha256(written.artifact_path()),
        artifact_bytes: fs::metadata(written.artifact_path()).unwrap().len(),
        windows: manifest["dataset_windows"].clone(),
        manifest,
        artifact: fs::read(written.artifact_path()).unwrap(),
        sidecar: fs::read(written.manifest_path()).unwrap(),
        source_sha256: test_file_sha256(&fixture.case.source_path),
    }
}

fn h5_files(root: &Path) -> usize {
    fs::read_dir(root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("h5")
                })
                .count()
        })
        .unwrap_or(0)
}

/// Before slice 6 the writer captured whole windows and refused anything above
/// its working-set limit (NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT). With a band
/// budget below the window size the streamed write succeeds, never reads or
/// writes more than the budget at once, and produces the baseline bytes.
#[test]
fn writer_v2_streams_a_window_larger_than_the_band_budget_within_the_budget() {
    let budget = 2 * 1024 * 1024;
    let _budget = V2StreamBudgetForTest::set(budget);
    reset_v2_stream_observation_for_test();
    let run = run_baseline_writer("writer-v2-stream-large", &["HH"], Some(LARGE_CHUNK_LAYOUT));
    let (largest, tiles) = v2_stream_observation_for_test();
    assert_matches_baseline(
        &run,
        LARGE_CHUNK_BASELINE,
        "large chunks under a 2 MiB budget",
    );
    // 640 x 640 complex samples (3.3 MB) are larger than the budget: the
    // science window streams as a 384-row and a 256-row band, the mask as one
    // tile, in the write pass and again in the validation pass.
    assert!(
        largest <= budget,
        "largest tile {largest} exceeds the budget {budget}"
    );
    assert_eq!(tiles, 2 * (2 + 1), "tiles streamed");
}

#[test]
fn writer_v2_bytes_do_not_depend_on_the_tile_budget() {
    // A one-byte budget streams one chunk per tile: runs of single chunks in
    // row-major order, the narrowest split the writer ever uses.
    let _budget = V2StreamBudgetForTest::set(1);
    reset_v2_stream_observation_for_test();
    let edge = run_baseline_writer("writer-v2-stream-edge-1b", &["HH"], Some(EDGE_CHUNK_LAYOUT));
    assert_matches_baseline(
        &edge,
        EDGE_CHUNK_BASELINE,
        "edge chunks, one chunk per tile",
    );
    let (largest, tiles) = v2_stream_observation_for_test();
    assert_eq!(largest, 6 * 8 * 8, "one 6 x 8 complex chunk");
    assert!(
        tiles > 2 * (7 * 5),
        "every chunk was streamed alone: {tiles}"
    );
    let dual = run_baseline_writer(
        "writer-v2-stream-dual-1b",
        &["HH", "HV"],
        Some(DUAL_POL_LAYOUT),
    );
    assert_matches_baseline(
        &dual,
        DUAL_POL_BASELINE,
        "dual polarization, one chunk per tile",
    );
}

const NAN_QUIET_PAYLOAD: u32 = 0x7fc0_0001;
const NAN_NEGATIVE_PAYLOAD: u32 = 0xffc0_1234;

/// A NaN source sample used to fail validation (derived PartialEq says NaN !=
/// NaN), so such a window could never be written. Validation now compares bit
/// patterns, and the artifact keeps each NaN payload and the sign of zero.
#[test]
fn writer_v2_writes_and_validates_nan_and_signed_zero_samples_bit_for_bit() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let special = [
        (
            0_usize,
            0_usize,
            f32::from_bits(NAN_QUIET_PAYLOAD),
            -0.0_f32,
        ),
        (17, 9, -0.0, f32::from_bits(NAN_NEGATIVE_PAYLOAD)),
        (
            39,
            35,
            f32::from_bits(NAN_NEGATIVE_PAYLOAD),
            f32::from_bits(NAN_QUIET_PAYLOAD),
        ),
    ];
    let fixture = science_complete_fixture_with(
        "writer-v2-stream-nan",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        move |file| {
            let dataset = file
                .dataset("/science/LSAR/RSLC/swaths/frequencyA/HH")
                .unwrap();
            for (row, col, r, i) in special {
                dataset
                    .write_slice(
                        &ndarray::arr2(&[[TestCFloat32 { r, i }]]),
                        (row..row + 1, col..col + 1),
                    )
                    .unwrap();
            }
        },
        &anchor,
    );
    let output = fixture.case.root.0.join("writer-v2-stream-nan-output");
    let written = write_fixture(
        &fixture,
        &anchor,
        &output,
        &WorkerControl::default(),
        WriterFailpointV2::None,
    )
    .unwrap();
    let artifact = hdf5::File::open(written.artifact_path()).unwrap();
    let derived = artifact
        .dataset("/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA/HH")
        .unwrap();
    for (row, col, r, i) in special {
        let value = derived
            .read_slice_2d::<TestCFloat32, _>((row..row + 1, col..col + 1))
            .unwrap()[[0, 0]];
        assert_eq!(
            (value.r.to_bits(), value.i.to_bits()),
            (r.to_bits(), i.to_bits())
        );
    }
    let ordinary = derived
        .read_slice_2d::<TestCFloat32, _>((1..2, 1..2))
        .unwrap()[[0, 0]];
    let expected = baseline_science_value(0, EDGE_CHUNK_LAYOUT.cols + 1);
    assert_eq!(
        (ordinary.r.to_bits(), ordinary.i.to_bits()),
        (expected.r.to_bits(), expected.i.to_bits())
    );
}

/// A single flipped bit in the last sample of the last tile is found, in the
/// science window and in the mask, with every tile size.
#[test]
fn writer_v2_detects_one_flipped_bit_in_the_last_tile() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-stream-tamper",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    for budget in [1, 6 * 36 * 8 * 2, usize::MAX] {
        let _budget = V2StreamBudgetForTest::set(budget);
        for (label, failpoint) in [
            (
                "science",
                WriterFailpointV2::CorruptLastScienceSampleBeforeValidation,
            ),
            (
                "mask",
                WriterFailpointV2::CorruptLastMaskSampleBeforeValidation,
            ),
        ] {
            let output = fixture
                .case
                .root
                .0
                .join(format!("writer-v2-stream-tamper-{label}-{budget}"));
            let error = write_fixture(
                &fixture,
                &anchor,
                &output,
                &WorkerControl::default(),
                failpoint,
            )
            .unwrap_err();
            assert_eq!(
                error,
                Error::PostWriteScienceValidation,
                "{label} budget={budget}"
            );
            assert_eq!(h5_files(&output), 0, "{label} budget={budget}");
            assert!(
                writer_v2_candidate_paths(&output).is_empty(),
                "{label} budget={budget}"
            );
        }
    }
}

/// A pause between two tiles stops the write before the next read; no output
/// is published and the next run writes the baseline bytes.
#[test]
fn writer_v2_pause_between_tiles_stops_without_output_and_resumes_to_the_baseline() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-stream-pause",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    let output = fixture.case.root.0.join("writer-v2-stream-pause-output");
    let _budget = V2StreamBudgetForTest::set(6 * 36 * 8);
    let control = WorkerControl::default();
    let tiles_seen = Arc::new(AtomicUsize::new(0));
    let hook_control = control.clone();
    let hook_tiles = Arc::clone(&tiles_seen);
    let hook = V2AfterTileHookForTest::set(move |pass, index| {
        hook_tiles.fetch_add(1, Ordering::SeqCst);
        if pass == V2StreamPass::Write && index == 1 {
            hook_control.set_paused_for_test(true);
        }
    });
    let error = write_fixture(
        &fixture,
        &anchor,
        &output,
        &control,
        WriterFailpointV2::None,
    )
    .unwrap_err();
    drop(hook);
    assert_eq!(error, Error::Cancelled);
    assert_eq!(
        tiles_seen.load(Ordering::SeqCst),
        2,
        "the pause stops before the third tile"
    );
    assert_eq!(h5_files(&output), 0);
    assert!(writer_v2_candidate_paths(&output).is_empty());
    let resumed = write_fixture(
        &fixture,
        &anchor,
        &output,
        &WorkerControl::default(),
        WriterFailpointV2::None,
    )
    .unwrap();
    assert_matches_baseline(
        &baseline_run_from(&resumed, &fixture),
        EDGE_CHUNK_BASELINE,
        "resumed",
    );
}

/// A candidate or artifact written in one tile per dataset (the former
/// whole-window write) and anchored before a crash is recovered by the
/// streamed validator with another tile size, without rewriting it. A pause
/// during that validation is an interruption, not a recovery failure.
#[test]
fn writer_v2_recovers_artifacts_written_with_another_tile_budget() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-stream-recover",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );

    #[cfg(windows)]
    {
        let output = fixture
            .case
            .root
            .0
            .join("writer-v2-stream-recover-candidate");
        assert_eq!(
            write_fixture(
                &fixture,
                &anchor,
                &output,
                &WorkerControl::default(),
                WriterFailpointV2::AfterValidationBeforePromotion,
            )
            .unwrap_err(),
            Error::InjectedFailure
        );
        assert_eq!(
            crate::nisar_subset_writer::tests::recovery_anchor_state_for_test(&output),
            Some("CandidateValidated")
        );
        let candidate_sha256 = test_file_sha256(&writer_v2_candidate_paths(&output)[0]);
        let _budget = V2StreamBudgetForTest::set(1);
        let recovered = write_fixture(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::None,
        )
        .unwrap();
        assert_eq!(
            test_file_sha256(recovered.artifact_path()),
            candidate_sha256
        );
        assert_matches_baseline(
            &baseline_run_from(&recovered, &fixture),
            EDGE_CHUNK_BASELINE,
            "recovered candidate",
        );
    }

    let output = fixture
        .case
        .root
        .0
        .join("writer-v2-stream-recover-artifact");
    assert_eq!(
        write_fixture(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::AfterArtifactPromotion,
        )
        .unwrap_err(),
        Error::InjectedFailure
    );
    let published = fs::read_dir(&output)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("h5"))
        .unwrap();
    let published_sha256 = test_file_sha256(&published);
    let _budget = V2StreamBudgetForTest::set(1);
    let control = WorkerControl::default();
    let hook_control = control.clone();
    let hook = V2AfterTileHookForTest::set(move |pass, index| {
        if pass == V2StreamPass::Verify && index == 3 {
            hook_control.set_paused_for_test(true);
        }
    });
    assert_eq!(
        write_fixture(
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
    assert_eq!(
        crate::nisar_subset_writer::tests::recovery_anchor_state_for_test(&output),
        Some("ArtifactPromoted")
    );
    let recovered = write_fixture(
        &fixture,
        &anchor,
        &output,
        &WorkerControl::default(),
        WriterFailpointV2::None,
    )
    .unwrap();
    assert_eq!(
        test_file_sha256(recovered.artifact_path()),
        published_sha256
    );
    assert_matches_baseline(
        &baseline_run_from(&recovered, &fixture),
        EDGE_CHUNK_BASELINE,
        "recovered artifact",
    );
}

/// Missing free space is refused before a candidate or anchor exists, with the
/// required and available bytes; the same write succeeds once space exists.
#[test]
fn writer_v2_refuses_insufficient_free_space_before_any_candidate() {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(
        "writer-v2-stream-disk",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
        &anchor,
    );
    let output = fixture.case.root.0.join("writer-v2-stream-disk-output");
    let refused = {
        let _available = V2AvailableDiskForTest::set(4096);
        write_fixture(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::None,
        )
        .unwrap_err()
    };
    let Error::InsufficientDiskSpace {
        required,
        available,
    } = refused
    else {
        panic!("expected a free-space refusal, got {refused:?}");
    };
    assert_eq!(available, 4096);
    assert!(required > crate::nisar_subset_stream_v2::V2_DISK_HEADROOM_BYTES);
    assert_eq!(
        refused.code(),
        "NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT"
    );
    assert_eq!(h5_files(&output), 0);
    assert!(writer_v2_candidate_paths(&output).is_empty());
    assert_eq!(
        crate::nisar_subset_writer::tests::recovery_anchor_state_for_test(&output),
        None
    );
    let written = write_fixture(
        &fixture,
        &anchor,
        &output,
        &WorkerControl::default(),
        WriterFailpointV2::None,
    )
    .unwrap();
    assert_matches_baseline(
        &baseline_run_from(&written, &fixture),
        EDGE_CHUNK_BASELINE,
        "after space",
    );
}

#[cfg(windows)]
fn current_working_set_bytes() -> usize {
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(
            process: isize,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
    }
    let mut counters = ProcessMemoryCounters {
        cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        ..ProcessMemoryCounters::default()
    };
    // SAFETY: the pseudo handle of the current process needs no closing and
    // `counters` is a writable record of exactly the size passed.
    let status =
        unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    assert_ne!(status, 0, "process memory counters");
    counters.working_set_size
}

/// Peak-memory probe for the slice report (run explicitly with --ignored):
/// a 2048 x 2048 complex window (32 MiB science, 4 MiB mask) is written and
/// validated while a sampler records the process working set. The growth
/// over the writer call stays well below one window because only tiles of
/// the band budget are held.
#[cfg(windows)]
#[test]
#[ignore = "peak-memory probe for the Issue #8 slice 6 report; run with --ignored"]
fn writer_v2_streaming_peak_memory_probe() {
    const LAYOUT: super::writer_baseline::ScienceLayout = super::writer_baseline::ScienceLayout {
        rows: 2048,
        cols: 2048,
        science_chunk: (256, 256),
        mask_chunk: (256, 256),
    };
    const SCIENCE_BYTES: usize = 2048 * 2048 * 8;
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture("writer-v2-peak-probe", &["HH"], Some(LAYOUT), &anchor);
    for (label, budget) in [
        ("4mib", 4 * 1024 * 1024),
        (
            "default",
            crate::nisar_subset_stream_v2::V2_STREAM_BAND_BUDGET_BYTES,
        ),
    ] {
        let _budget = V2StreamBudgetForTest::set(budget);
        reset_v2_stream_observation_for_test();
        let output = fixture
            .case
            .root
            .0
            .join(format!("writer-v2-peak-probe-{label}"));
        let baseline = current_working_set_bytes();
        let peak = Arc::new(AtomicUsize::new(baseline));
        let done = Arc::new(AtomicBool::new(false));
        let sampler = {
            let (peak, done) = (Arc::clone(&peak), Arc::clone(&done));
            thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    peak.fetch_max(current_working_set_bytes(), Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(1));
                }
            })
        };
        let written = write_fixture(
            &fixture,
            &anchor,
            &output,
            &WorkerControl::default(),
            WriterFailpointV2::None,
        )
        .unwrap();
        done.store(true, Ordering::SeqCst);
        sampler.join().unwrap();
        let peak = peak.load(Ordering::SeqCst);
        let (largest_tile, tiles) = v2_stream_observation_for_test();
        eprintln!(
            "WRITER_PEAK budget={label} band_budget_bytes={budget} window_science_bytes={SCIENCE_BYTES} \
             largest_tile_bytes={largest_tile} tiles={tiles} working_set_before={baseline} \
             working_set_peak={peak} growth={} artifact_bytes={}",
            peak.saturating_sub(baseline),
            fs::metadata(written.artifact_path()).unwrap().len()
        );
        assert!(largest_tile <= budget.max(256 * 256 * 8));
    }
}
