//! Characterization baseline of the NISAR V2 subset writer (Issue #8, slice 6).
//!
//! Four loopback fixtures pin the bytes that the writer produces: many small
//! chunks that HDF5 writes through its default chunk cache, chunks larger
//! than the default 1 MiB chunk cache (written directly), partial edge chunks
//! in both dimensions, and two polarizations that share one validity mask. The
//! SHA-256 values were produced by the whole-window writer of 656cb1ac before
//! any writer change; a writer refactor must keep them equal.
//!
//! The fixture source is created without HDF5 object timestamps, so the
//! source bytes and every provenance digest derived from them (prepared
//! intent, final and physical plan, inventory, dependency graph, source
//! validator, completed-Range evidence, derived artifact id) are constant and
//! stay pinned. Two kinds of bytes still differ between runs and are masked
//! before hashing, nothing else:
//! - the writer stores wall-clock times in the version-2 object headers of
//!   its groups; the four time fields of each such header and the chunk-0
//!   checksum covering them are zeroed, and only headers whose stored
//!   checksum verifies are touched (a coincidental "OHDR" in data is not);
//! - `scienceCheckpointSha256` and `writerSelectionSha256` (which hashes the
//!   former) bind the recovery workspace's canonical path and file-system
//!   identity by design, so their values are replaced by '0' characters;
//!   the sidecar additionally names the raw artifact SHA-256.
//!
//! Their correctness is covered elsewhere: `validate_v2_candidate` compares
//! every provenance dataset with the revalidated selection, the
//! `CorruptProvenanceBeforeValidation` case of
//! `writer_v2_validator_rejects_corruption_and_recovery_closes_promotion_gaps`
//! rejects a tampered `writerSelectionSha256`, and the sidecar must equal the
//! rebuilt manifest (`validate_v2_manifest_sidecar`, recovery comparison).
use super::*;

const FREQUENCY_A: &str = "/science/LSAR/RSLC/swaths/frequencyA";
const SWATHS: &str = "/science/LSAR/RSLC/swaths";

#[derive(Clone, Copy, Debug)]
pub(super) struct ScienceLayout {
    pub(super) rows: usize,
    pub(super) cols: usize,
    pub(super) science_chunk: (usize, usize),
    pub(super) mask_chunk: (usize, usize),
}

pub(super) fn baseline_science_value(polarization: usize, index: usize) -> TestCFloat32 {
    let scaled = (index % 4093) as f32;
    TestCFloat32 {
        r: scaled * 0.25 + polarization as f32 * 1_000.0 + 1.0,
        i: -(((index * 7 + polarization * 13) % 3001) as f32) * 0.5 - 0.125,
    }
}

fn replace_dataset<T: hdf5::H5Type>(
    file: &hdf5::File,
    group: &str,
    name: &str,
    shape: (usize, usize),
    chunk: Option<(usize, usize)>,
    values: &[T],
) {
    let parent = file.group(group).unwrap();
    if parent.link_exists(name) {
        parent.unlink(name).unwrap();
    }
    let builder = parent.new_dataset::<T>().shape(shape);
    let builder = match chunk {
        Some(chunk) => builder.chunk(chunk),
        None => builder,
    };
    builder
        .create(name)
        .and_then(|dataset| dataset.write_raw(values))
        .unwrap();
}

fn replace_vector(file: &hdf5::File, group: &str, name: &str, values: &[f64]) {
    let parent = file.group(group).unwrap();
    if parent.link_exists(name) {
        parent.unlink(name).unwrap();
    }
    parent
        .new_dataset::<f64>()
        .shape((values.len(),))
        .create(name)
        .and_then(|dataset| dataset.write_raw(values))
        .unwrap();
}

/// Replaces frequency A science, mask and axes with one consistent layout.
/// Every polarization keeps the same shape and chunking; values differ.
pub(super) fn rebuild_frequency_a(file: &hdf5::File, layout: ScienceLayout) {
    let cells = layout.rows * layout.cols;
    for (index, polarization) in ["HH", "HV"].into_iter().enumerate() {
        let values = (0..cells)
            .map(|cell| baseline_science_value(index, cell))
            .collect::<Vec<_>>();
        replace_dataset(
            file,
            FREQUENCY_A,
            polarization,
            (layout.rows, layout.cols),
            Some(layout.science_chunk),
            &values,
        );
    }
    let mask = (0..cells)
        .map(|cell| (cell % 251) as u8)
        .collect::<Vec<_>>();
    replace_dataset(
        file,
        FREQUENCY_A,
        "inputDataExceptionMask",
        (layout.rows, layout.cols),
        Some(layout.mask_chunk),
        &mask,
    );
    let last_sample = (layout.cols - 1) as f64;
    let slant_range = (0..layout.cols)
        .map(|sample| 800_000.0 + 30.0 * sample as f64 / last_sample)
        .collect::<Vec<_>>();
    replace_vector(file, FREQUENCY_A, "slantRange", &slant_range);
    let valid_samples = (0..layout.rows)
        .flat_map(|_| [0_u32, (layout.cols - 1) as u32])
        .collect::<Vec<_>>();
    replace_dataset(
        file,
        FREQUENCY_A,
        "validSamplesSubSwath1",
        (layout.rows, 2),
        None,
        &valid_samples,
    );
    let last_line = (layout.rows - 1) as f64;
    let zero_doppler_time = (0..layout.rows)
        .map(|line| 1_000.0 + 1.5 * line as f64 / last_line)
        .collect::<Vec<_>>();
    replace_vector(file, SWATHS, "zeroDopplerTime", &zero_doppler_time);
}

pub(super) fn baseline_science_http() -> HttpTransfer {
    HttpTransfer::new_with_proxy_policy(
        TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(30),
            chunk_size: 64 * 1024,
            ..TransferOptions::default()
        },
        ProxyPolicy::Direct,
    )
    .unwrap()
}

/// A planned, science-complete V2 item whose writer has not run yet.
pub(super) struct ScienceCompleteFixture {
    pub(super) case: crate::nisar_subset_writer::tests::V2SourceCase,
    pub(super) work: DownloadWork,
    pub(super) completed: NisarSubsetPlanningCompleteV2,
    pub(super) bootstrap: Arc<NisarSubsetLocalHdf5MetadataBootstrapV2>,
    pub(super) url_root: String,
    pub(super) science: crate::nisar_science_transfer_v2::NisarSubsetScienceTransferResultV2,
    pub(super) science_request_count: usize,
}

pub(super) fn science_complete_fixture(
    label: &str,
    polarizations: &[&str],
    layout: Option<ScienceLayout>,
    anchor: &ScienceCheckpointAnchorFixture,
) -> ScienceCompleteFixture {
    science_complete_fixture_with(label, polarizations, layout, |_| {}, anchor)
}

/// `mutate` runs on the source after the layout is rebuilt.
pub(super) fn science_complete_fixture_with(
    label: &str,
    polarizations: &[&str],
    layout: Option<ScienceLayout>,
    mutate: impl FnOnce(&hdf5::File),
    anchor: &ScienceCheckpointAnchorFixture,
) -> ScienceCompleteFixture {
    let case = crate::nisar_subset_writer::tests::v2_source_case(label, move |file| {
        if let Some(layout) = layout {
            rebuild_frequency_a(file, layout);
        }
        mutate(file);
        // The same uncompressed unselected ballast as `concrete_fixture`, so the
        // metadata-to-source ratio exercises the production bootstrap budget.
        let science_ballast = vec![0_u8; 4 * 1024 * 1024];
        file.group(FREQUENCY_A)
            .unwrap()
            .new_dataset::<u8>()
            .shape((science_ballast.len(),))
            .chunk((64 * 1024,))
            .create("RV")
            .and_then(|dataset| dataset.write_raw(&science_ballast))
            .unwrap();
    });
    let intent = prepared_intent_for_source_request(
        &case.request,
        NisarSubsetFrequency::A,
        polarizations,
        json!({
            "type": "Polygon",
            "coordinates": [[[36.1, 9.1], [36.9, 9.1], [36.9, 9.9], [36.1, 9.9], [36.1, 9.1]]]
        }),
    );
    let source_path = case.source_path.clone();
    let hdf5_extent = fs::metadata(&source_path).unwrap().len();
    let workspace =
        Hdf5RangeWorkspaceV1::new(case.root.0.join(format!("{label}-bootstrap"))).unwrap();
    let bootstrap = Arc::new(
        NisarSubsetLocalHdf5MetadataBootstrapV2::new_for_test(
            source_path.clone(),
            hdf5_extent,
            workspace,
        )
        .unwrap(),
    );
    let mut work = work_for_source_request(&case.request, &case.root.0, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, anchor);
    let science = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        baseline_science_http(),
        asset,
    )
    .unwrap()
    .execute(&work, &completed, &authority, &WorkerControl::default())
    .unwrap();
    let url_root = server.url_root.clone();
    let science_request_count = server.finish().len();
    assert_eq!(
        science_request_count,
        physical.execution_transfer_ranges().unwrap().len()
    );
    ScienceCompleteFixture {
        case,
        work,
        completed,
        bootstrap,
        url_root,
        science,
        science_request_count,
    }
}

impl ScienceCompleteFixture {
    /// The science authority over the same checkpoint anchor; the URL only
    /// binds the frozen remote source identity, no request is sent.
    pub(super) fn authority<'a>(
        &'a self,
        anchor: &'a ScienceCheckpointAnchorFixture,
    ) -> crate::nisar_science_transfer_v2::NisarSubsetScienceAuthorityBindingV2<'a> {
        let physical = self.completed.physical_execution_plan();
        let asset = science_asset(
            &self.url_root,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        );
        science_authority(
            self.bootstrap.as_ref(),
            &asset,
            &self.work,
            &self.completed,
            anchor,
        )
    }
}

pub(super) struct BaselineRun {
    pub(super) artifact_sha256: String,
    pub(super) artifact_bytes: u64,
    pub(super) windows: Value,
    pub(super) manifest: Value,
    pub(super) artifact: Vec<u8>,
    pub(super) sidecar: Vec<u8>,
    pub(super) source_sha256: String,
}

pub(super) fn run_baseline_writer(
    label: &str,
    polarizations: &[&str],
    layout: Option<ScienceLayout>,
) -> BaselineRun {
    let anchor = ScienceCheckpointAnchorFixture::default();
    let fixture = science_complete_fixture(label, polarizations, layout, &anchor);
    let authority = fixture.authority(&anchor);
    let written = crate::nisar_subset_writer::tests::writer(
        &fixture.case.root.0.join(format!("{label}-output")),
    )
    .write_and_validate_v2(
        &fixture.work,
        &fixture.completed,
        &fixture.science,
        &authority,
        &WorkerControl::default(),
    )
    .unwrap();
    assert!(fixture.science_request_count > 0);
    let artifact_sha256 = test_file_sha256(written.artifact_path());
    assert_eq!(artifact_sha256, written.manifest().artifact_sha256());
    let manifest = serde_json::to_value(written.manifest()).unwrap();
    BaselineRun {
        artifact_sha256,
        artifact_bytes: fs::metadata(written.artifact_path()).unwrap().len(),
        windows: manifest["dataset_windows"].clone(),
        manifest,
        artifact: fs::read(written.artifact_path()).unwrap(),
        sidecar: fs::read(written.manifest_path()).unwrap(),
        source_sha256: test_file_sha256(&fixture.case.source_path),
    }
}

/// Provenance values bound to the recovery workspace (see the module note).
const WORKSPACE_BOUND_MANIFEST_FIELDS: &[&str] =
    &["science_checkpoint_sha256", "writer_selection_sha256"];

fn find_from(bytes: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

fn mask_every(bytes: &mut [u8], needle: &[u8]) -> usize {
    let mut hits = 0;
    let mut from = 0;
    while let Some(found) = find_from(bytes, needle, from) {
        bytes[found..found + needle.len()].fill(b'0');
        hits += 1;
        from = found + needle.len();
    }
    hits
}

/// HDF5 metadata checksum (Jenkins lookup3 `hashlittle`, `H5_checksum_lookup3`).
fn hdf5_metadata_checksum(key: &[u8]) -> u32 {
    fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
        *a = a.wrapping_sub(*c);
        *a ^= c.rotate_left(4);
        *c = c.wrapping_add(*b);
        *b = b.wrapping_sub(*a);
        *b ^= a.rotate_left(6);
        *a = a.wrapping_add(*c);
        *c = c.wrapping_sub(*b);
        *c ^= b.rotate_left(8);
        *b = b.wrapping_add(*a);
        *a = a.wrapping_sub(*c);
        *a ^= c.rotate_left(16);
        *c = c.wrapping_add(*b);
        *b = b.wrapping_sub(*a);
        *b ^= a.rotate_left(19);
        *a = a.wrapping_add(*c);
        *c = c.wrapping_sub(*b);
        *c ^= b.rotate_left(4);
        *b = b.wrapping_add(*a);
    }
    fn finish(a: &mut u32, b: &mut u32, c: &mut u32) {
        *c ^= *b;
        *c = c.wrapping_sub(b.rotate_left(14));
        *a ^= *c;
        *a = a.wrapping_sub(c.rotate_left(11));
        *b ^= *a;
        *b = b.wrapping_sub(a.rotate_left(25));
        *c ^= *b;
        *c = c.wrapping_sub(b.rotate_left(16));
        *a ^= *c;
        *a = a.wrapping_sub(c.rotate_left(4));
        *b ^= *a;
        *b = b.wrapping_sub(a.rotate_left(14));
        *c ^= *b;
        *c = c.wrapping_sub(b.rotate_left(24));
    }
    let word = |bytes: &[u8]| {
        bytes
            .iter()
            .enumerate()
            .fold(0_u32, |value, (index, byte)| {
                value | (u32::from(*byte) << (8 * index))
            })
    };
    let initial = 0xdead_beef_u32.wrapping_add(key.len() as u32);
    let (mut a, mut b, mut c) = (initial, initial, initial);
    let mut rest = key;
    while rest.len() > 12 {
        a = a.wrapping_add(word(&rest[0..4]));
        b = b.wrapping_add(word(&rest[4..8]));
        c = c.wrapping_add(word(&rest[8..12]));
        mix(&mut a, &mut b, &mut c);
        rest = &rest[12..];
    }
    if rest.is_empty() {
        return c;
    }
    a = a.wrapping_add(word(&rest[..rest.len().min(4)]));
    if rest.len() > 4 {
        b = b.wrapping_add(word(&rest[4..rest.len().min(8)]));
    }
    if rest.len() > 8 {
        c = c.wrapping_add(word(&rest[8..]));
    }
    finish(&mut a, &mut b, &mut c);
    c
}

/// Zeroes the access, modification, change and birth times of every
/// version-2 object header that stores times, and the chunk-0 checksum that
/// covers them. A header is touched only when its stored checksum verifies.
/// Returns the number of masked headers.
fn mask_object_header_times(bytes: &mut [u8]) -> usize {
    let mut masked = 0;
    let mut from = 0;
    while let Some(found) = find_from(bytes, b"OHDR", from) {
        from = found + 4;
        let (Some(&version), Some(&flags)) = (bytes.get(found + 4), bytes.get(found + 5)) else {
            break;
        };
        if version != 2 || flags & 0x20 == 0 {
            continue;
        }
        let mut size_at = found + 22;
        if flags & 0x10 != 0 {
            size_at += 4;
        }
        let width = 1_usize << (flags & 0x03);
        let Some(size_bytes) = bytes.get(size_at..size_at + width) else {
            continue;
        };
        let size = size_bytes
            .iter()
            .rev()
            .fold(0_usize, |value, byte| (value << 8) | usize::from(*byte));
        let Some(checksum_at) = size_at
            .checked_add(width)
            .and_then(|value| value.checked_add(size))
            .filter(|value| value.saturating_add(4) <= bytes.len())
        else {
            continue;
        };
        let stored = u32::from_le_bytes(bytes[checksum_at..checksum_at + 4].try_into().unwrap());
        if hdf5_metadata_checksum(&bytes[found..checksum_at]) != stored {
            continue;
        }
        bytes[found + 6..found + 22].fill(0);
        bytes[checksum_at..checksum_at + 4].fill(0);
        masked += 1;
    }
    masked
}

pub(super) struct NormalizedBaseline {
    pub(super) artifact_sha256: String,
    pub(super) sidecar_sha256: String,
    pub(super) timed_headers: usize,
}

pub(super) fn normalized_baseline(run: &BaselineRun) -> NormalizedBaseline {
    let mut artifact = run.artifact.clone();
    let mut sidecar = run.sidecar.clone();
    let timed_headers = mask_object_header_times(&mut artifact);
    for field in WORKSPACE_BOUND_MANIFEST_FIELDS {
        let value = run.manifest[*field].as_str().unwrap();
        assert_eq!(mask_every(&mut artifact, value.as_bytes()), 1, "{field}");
        assert_eq!(mask_every(&mut sidecar, value.as_bytes()), 1, "{field}");
    }
    let raw_artifact_sha256 = run.manifest["artifact_sha256"].as_str().unwrap();
    assert_eq!(mask_every(&mut sidecar, raw_artifact_sha256.as_bytes()), 1);
    NormalizedBaseline {
        artifact_sha256: format!("{:x}", Sha256::digest(&artifact)),
        sidecar_sha256: format!("{:x}", Sha256::digest(&sidecar)),
        timed_headers,
    }
}

pub(super) const SMALL_CHUNK_LAYOUT: ScienceLayout = ScienceLayout {
    rows: 24,
    cols: 22,
    science_chunk: (2, 2),
    mask_chunk: (8, 11),
};
pub(super) const LARGE_CHUNK_LAYOUT: ScienceLayout = ScienceLayout {
    rows: 640,
    cols: 640,
    science_chunk: (384, 384),
    mask_chunk: (384, 384),
};
pub(super) const EDGE_CHUNK_LAYOUT: ScienceLayout = ScienceLayout {
    rows: 40,
    cols: 36,
    science_chunk: (6, 8),
    mask_chunk: (5, 4),
};
pub(super) const DUAL_POL_LAYOUT: ScienceLayout = ScienceLayout {
    rows: 30,
    cols: 26,
    science_chunk: (4, 6),
    mask_chunk: (3, 5),
};

/// (normalized artifact SHA-256, normalized sidecar SHA-256) measured with the
/// whole-window writer of 656cb1ac (logs 008 and 009 of the slice-6 evidence).
pub(super) const SMALL_CHUNK_BASELINE: (&str, &str) = (
    "b88f66142fd4b97b8bb874eec30f2a16643854a347e176ffebe30503b99177f5",
    "8d07f99c2dc1ee20da7a32ad0412902f42b8451ffd61c1440461fa96e7bc1d2f",
);
pub(super) const LARGE_CHUNK_BASELINE: (&str, &str) = (
    "62ea8d93efb08954547032688e3a03589fa2cd572adfade7c5a1127f3e2bf3bf",
    "b0d20b998a5cbf2ac2ad8a6a556d4e72baaf834b276e1a297afe9e87f33e02f9",
);
pub(super) const EDGE_CHUNK_BASELINE: (&str, &str) = (
    "4d0b420133ac1b338773fcd0026a649293a2976291670c31d2692350b8193268",
    "3e6926bd4adb802be8399f2664a364d82b651b1a003fefe2cea33cc0c6915286",
);
pub(super) const DUAL_POL_BASELINE: (&str, &str) = (
    "1bb12cf8e99b201f5a90dbc786e2b2eb57c0bcccc8e39c675a7327391aa64262",
    "9aff4681ebda3508a4e6e8ec14a066f4dbcda51de9e0806296296acba0236e47",
);

pub(super) fn assert_baseline(
    label: &str,
    polarizations: &[&str],
    layout: ScienceLayout,
    expected: (&str, &str),
) -> BaselineRun {
    let run = run_baseline_writer(label, polarizations, Some(layout));
    let normalized = normalized_baseline(&run);
    eprintln!(
        "WRITER_BASELINE label={label} normalized_artifact_sha256={} normalized_sidecar_sha256={} raw_artifact_sha256={} artifact_bytes={} timed_headers={} windows={}",
        normalized.artifact_sha256,
        normalized.sidecar_sha256,
        run.artifact_sha256,
        run.artifact_bytes,
        normalized.timed_headers,
        run.windows
    );
    assert_eq!(normalized.artifact_sha256, expected.0, "{label} artifact");
    assert_eq!(normalized.sidecar_sha256, expected.1, "{label} sidecar");
    run
}

#[test]
fn writer_v2_baseline_small_cached_chunks_are_byte_stable() {
    assert_baseline(
        "writer-v2-baseline-small-chunk",
        &["HH"],
        SMALL_CHUNK_LAYOUT,
        SMALL_CHUNK_BASELINE,
    );
}

#[test]
fn writer_v2_baseline_chunks_above_one_mebibyte_are_byte_stable() {
    assert_baseline(
        "writer-v2-baseline-large-chunk",
        &["HH"],
        LARGE_CHUNK_LAYOUT,
        LARGE_CHUNK_BASELINE,
    );
}

#[test]
fn writer_v2_baseline_partial_edge_chunks_are_byte_stable() {
    assert_baseline(
        "writer-v2-baseline-edge-chunk",
        &["HH"],
        EDGE_CHUNK_LAYOUT,
        EDGE_CHUNK_BASELINE,
    );
}

#[test]
fn writer_v2_baseline_dual_polarization_shared_mask_is_byte_stable() {
    assert_baseline(
        "writer-v2-baseline-dual-pol",
        &["HH", "HV"],
        DUAL_POL_LAYOUT,
        DUAL_POL_BASELINE,
    );
}

/// The normalization masks only run-bound bytes: two runs in different
/// workspaces differ exactly in the workspace-bound provenance values and the
/// raw artifact digest, yet agree byte for byte after normalization.
#[test]
fn writer_v2_baseline_normalization_masks_only_run_bound_bytes() {
    let first = run_baseline_writer("writer-v2-baseline-run-a", &["HH"], Some(EDGE_CHUNK_LAYOUT));
    let second = run_baseline_writer(
        "writer-v2-baseline-run-b-longer-label",
        &["HH"],
        Some(EDGE_CHUNK_LAYOUT),
    );
    assert_eq!(
        first.source_sha256, second.source_sha256,
        "the fixture source is reproducible"
    );
    let differing = first
        .manifest
        .as_object()
        .unwrap()
        .iter()
        .filter(|(key, value)| second.manifest[key.as_str()] != **value)
        .map(|(key, _)| key.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        differing,
        BTreeSet::from([
            "artifact_sha256",
            "science_checkpoint_sha256",
            "writer_selection_sha256"
        ])
    );
    assert_eq!(first.artifact.len(), second.artifact.len());
    let left = normalized_baseline(&first);
    let right = normalized_baseline(&second);
    assert!(left.timed_headers > 0);
    assert_eq!(left.timed_headers, right.timed_headers);
    assert_eq!(left.artifact_sha256, right.artifact_sha256);
    assert_eq!(left.sidecar_sha256, right.sidecar_sha256);
}

#[test]
fn hdf5_metadata_checksum_matches_the_lookup3_reference_vectors() {
    // Bob Jenkins' published hashlittle vectors: "" -> 0xdeadbeef and
    // "Four score and seven years ago" -> 0x17770551 (initval 0).
    assert_eq!(hdf5_metadata_checksum(b""), 0xdead_beef);
    assert_eq!(
        hdf5_metadata_checksum(b"Four score and seven years ago"),
        0x1777_0551
    );
}
