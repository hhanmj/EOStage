mod support;

use insar_subset_lab::{
    discover_chunk_layout, discover_chunk_layout_with_prefix, read_raw_chunk_payloads, ByteRange,
    MergePolicy, RangePlan, SubsetLabError, Window2D, DEFAULT_METADATA_PREFIX_BYTES,
};

use support::{create_rslc_fixture, TempRoot};

#[test]
fn range_merge_is_deterministic_and_respects_the_sixteen_mib_gap_boundary() {
    let mib = 1024_u64 * 1024;
    let source_len = 128 * mib;
    let policy = MergePolicy {
        maximum_gap_bytes: 16 * mib,
        maximum_extra_bytes: 32 * mib,
        maximum_ranges: 8,
        maximum_transfer_bytes: source_len,
    };
    let ranges = [
        ByteRange::new(40 * mib, 41 * mib, source_len).unwrap(),
        ByteRange::new(0, mib, source_len).unwrap(),
        ByteRange::new(17 * mib, 18 * mib, source_len).unwrap(),
        ByteRange::new(18 * mib, 19 * mib, source_len).unwrap(),
    ];
    let plan = RangePlan::build(source_len, ranges, policy).unwrap();
    assert_eq!(
        plan.ranges(),
        &[
            ByteRange::new(0, 19 * mib, source_len).unwrap(),
            ByteRange::new(40 * mib, 41 * mib, source_len).unwrap(),
        ]
    );
    assert_eq!(plan.selected_bytes(), 4 * mib);
    assert_eq!(plan.extra_bytes(), 16 * mib);
    assert_eq!(plan.transfer_bytes(), 20 * mib);

    let outside = RangePlan::build(
        source_len,
        [
            ByteRange::new(0, mib, source_len).unwrap(),
            ByteRange::new(17 * mib + 1, 18 * mib + 1, source_len).unwrap(),
        ],
        policy,
    )
    .unwrap();
    assert_eq!(outside.ranges().len(), 2);
    assert_eq!(outside.extra_bytes(), 0);
}

#[test]
fn overlapping_ranges_collapse_without_double_counting_and_budgets_fail_closed() {
    let source_len = 10_000;
    let ranges = [
        ByteRange::new(100, 300, source_len).unwrap(),
        ByteRange::new(200, 400, source_len).unwrap(),
        ByteRange::new(400, 500, source_len).unwrap(),
    ];
    let plan = RangePlan::build(
        source_len,
        ranges,
        MergePolicy {
            maximum_gap_bytes: 0,
            maximum_extra_bytes: 0,
            maximum_ranges: 1,
            maximum_transfer_bytes: 400,
        },
    )
    .unwrap();
    assert_eq!(
        plan.ranges(),
        &[ByteRange::new(100, 500, source_len).unwrap()]
    );
    assert_eq!(plan.selected_bytes(), 400);
    assert_eq!(plan.transfer_bytes(), 400);
    assert!(RangePlan::build(
        source_len,
        plan.ranges().iter().copied(),
        MergePolicy {
            maximum_transfer_bytes: 399,
            ..MergePolicy::default()
        },
    )
    .is_err());
}

#[test]
fn optional_gap_merge_never_turns_a_valid_transfer_budget_into_failure() {
    let source_len = 100;
    let plan = RangePlan::build(
        source_len,
        [
            ByteRange::new(0, 10, source_len).unwrap(),
            ByteRange::new(20, 30, source_len).unwrap(),
        ],
        MergePolicy {
            maximum_gap_bytes: 10,
            maximum_extra_bytes: 10,
            maximum_ranges: 2,
            maximum_transfer_bytes: 20,
        },
    )
    .unwrap();
    assert_eq!(plan.ranges().len(), 2);
    assert_eq!(plan.selected_bytes(), 20);
    assert_eq!(plan.transfer_bytes(), 20);
    assert_eq!(plan.extra_bytes(), 0);
}

#[test]
fn serialized_ranges_and_plan_totals_reject_invariant_bypasses() {
    assert!(serde_json::from_str::<ByteRange>(r#"{"start":5,"end_exclusive":5}"#).is_err());
    let plan = RangePlan::build(
        1_000,
        [ByteRange::new(100, 200, 1_000).unwrap()],
        MergePolicy {
            maximum_gap_bytes: 0,
            maximum_extra_bytes: 0,
            maximum_ranges: 1,
            maximum_transfer_bytes: 100,
        },
    )
    .unwrap();
    let encoded = serde_json::to_value(&plan).unwrap();
    assert_eq!(
        serde_json::from_value::<RangePlan>(encoded.clone()).unwrap(),
        plan
    );

    let mut tampered = encoded;
    tampered["transfer_bytes"] = serde_json::json!(99);
    assert!(serde_json::from_value::<RangePlan>(tampered).is_err());
}

#[test]
fn sparse_prefix_probe_grows_until_the_real_superblock_and_chunk_index_are_available() {
    let root = TempRoot::new("prefix-growth");
    let source = root.file("source-userblock.h5");
    let scratch = root.file("prefix-scratch.h5");
    create_rslc_fixture(&source, 4_096, [128, 128], [16, 16]);
    let discovery = discover_chunk_layout_with_prefix(
        &source,
        &scratch,
        "/science/LSAR/RSLC/swaths/frequencyA/HH",
        Window2D::new(32, 64, 32, 64, [128, 128]).unwrap(),
        512,
        64 * 1024,
    )
    .unwrap();
    assert!(discovery.attempts > 1);
    assert!(discovery.prefix_bytes >= 4_096);
    assert!(discovery.prefix_bytes < discovery.source_len);
    assert_eq!(discovery.layout.chunks.len(), 4);
    assert_eq!(discovery.layout.source_path, source);

    let direct = discover_chunk_layout(
        &source,
        "/science/LSAR/RSLC/swaths/frequencyA/HH",
        Window2D::new(32, 64, 32, 64, [128, 128]).unwrap(),
    )
    .unwrap();
    assert_eq!(
        read_raw_chunk_payloads(&discovery.layout).unwrap(),
        read_raw_chunk_payloads(&direct).unwrap()
    );
}

#[test]
fn configured_128_mib_prefix_is_bounded_to_a_small_source_file() {
    let root = TempRoot::new("prefix-default");
    let source = root.file("source.h5");
    let scratch = root.file("scratch.h5");
    create_rslc_fixture(&source, 0, [8, 8], [2, 2]);
    let source_len = std::fs::metadata(&source).unwrap().len();
    let discovery = discover_chunk_layout_with_prefix(
        &source,
        &scratch,
        "/science/LSAR/RSLC/swaths/frequencyA/HH",
        Window2D::new(0, 2, 0, 2, [8, 8]).unwrap(),
        DEFAULT_METADATA_PREFIX_BYTES,
        DEFAULT_METADATA_PREFIX_BYTES,
    )
    .unwrap();

    assert_eq!(discovery.attempts, 1);
    assert_eq!(discovery.prefix_bytes, source_len);
    assert!(source_len < DEFAULT_METADATA_PREFIX_BYTES);
}

#[test]
fn sparse_prefix_probe_rejects_a_hard_link_alias_without_truncating_source() {
    let root = TempRoot::new("prefix-hard-link");
    let source = root.file("source.h5");
    let alias = root.file("source-alias.h5");
    create_rslc_fixture(&source, 0, [8, 8], [2, 2]);
    std::fs::hard_link(&source, &alias).unwrap();
    let before = std::fs::read(&source).unwrap();

    assert_eq!(
        discover_chunk_layout_with_prefix(
            &source,
            &alias,
            "/science/LSAR/RSLC/swaths/frequencyA/HH",
            Window2D::new(0, 2, 0, 2, [8, 8]).unwrap(),
            512,
            16 * 1024,
        ),
        Err(SubsetLabError::ScratchPathUnsafe)
    );
    assert_eq!(std::fs::read(&source).unwrap(), before);
}

#[test]
fn zero_gap_policy_coalesces_adjacent_inner_chunks_and_preserves_strided_shard_gaps() {
    use insar_subset_lab::ZARR_V3_SHARD_128_TAIL_INDEX_BYTES;

    // Simulate a 42 MiB Zarr v3 shard (`1 x 128 x 2048 x 2048`, inner chunks `1 x 64 x 256 x 256` -> `[2, 8, 8]` = 128 inner chunks)
    // where each compressed inner chunk is ~320 KiB and the shard ends with a 2052-byte tail index.
    let chunk_bytes = 320 * 1024_u64;
    let total_chunks = 128_u64;
    let source_len = total_chunks * chunk_bytes + ZARR_V3_SHARD_128_TAIL_INDEX_BYTES;

    // Suppose an AOI window selects a 2x2 spatial sub-grid in the first Matryoshka 64D slice (`c = 0..64`):
    // - Row 2, Cols 3..=4 -> inner chunk indices 19 and 20 (contiguous in byte layout)
    // - Row 3, Cols 3..=4 -> inner chunk indices 27 and 28 (contiguous in byte layout, separated from Row 2 by 6 unselected chunks = 1.92 MiB gap)
    // Plus the 2052-byte tail index at the end of the shard.
    let selected_ranges = [
        ByteRange::new(19 * chunk_bytes, 20 * chunk_bytes, source_len).unwrap(),
        ByteRange::new(20 * chunk_bytes, 21 * chunk_bytes, source_len).unwrap(),
        ByteRange::new(27 * chunk_bytes, 28 * chunk_bytes, source_len).unwrap(),
        ByteRange::new(28 * chunk_bytes, 29 * chunk_bytes, source_len).unwrap(),
        ByteRange::tail(ZARR_V3_SHARD_128_TAIL_INDEX_BYTES, source_len).unwrap(),
    ];

    // 1. With `MergePolicy::zero_gap()`, adjacent inner chunks (19+20 and 27+28) coalesce cleanly,
    //    while the 1.92 MiB gap and the gap before the 2052-byte tail index are never bridged (`extra_bytes == 0`).
    let zero_gap_plan =
        RangePlan::build(source_len, selected_ranges, MergePolicy::zero_gap()).unwrap();
    assert_eq!(
        zero_gap_plan.ranges(),
        &[
            ByteRange::new(19 * chunk_bytes, 21 * chunk_bytes, source_len).unwrap(),
            ByteRange::new(27 * chunk_bytes, 29 * chunk_bytes, source_len).unwrap(),
            ByteRange::new(
                source_len - ZARR_V3_SHARD_128_TAIL_INDEX_BYTES,
                source_len,
                source_len
            )
            .unwrap(),
        ]
    );
    assert_eq!(
        zero_gap_plan.selected_bytes(),
        4 * chunk_bytes + ZARR_V3_SHARD_128_TAIL_INDEX_BYTES
    );
    assert_eq!(zero_gap_plan.extra_bytes(), 0);
    assert_eq!(
        zero_gap_plan.transfer_bytes(),
        zero_gap_plan.selected_bytes()
    );

    // 2. Contrast with `MergePolicy::default()` (16 MiB gap), which bridges the 1.92 MiB gap between rows
    //    and pulls in 6 unselected inner chunks (`6 * 320 KiB` extra bytes).
    let default_plan =
        RangePlan::build(source_len, selected_ranges, MergePolicy::default()).unwrap();
    assert!(default_plan.extra_bytes() >= 6 * chunk_bytes);
    assert!(default_plan.transfer_bytes() > zero_gap_plan.transfer_bytes());

    // 3. `with_zero_gap()` preserves custom range/transfer caps while enforcing zero extra bytes,
    //    and `ByteRange::tail` rejects zero or oversized tail lengths.
    let custom_zero = MergePolicy {
        maximum_gap_bytes: 1024,
        maximum_extra_bytes: 4096,
        maximum_ranges: 16,
        maximum_transfer_bytes: 8 * 1024 * 1024,
    }
    .with_zero_gap();
    assert_eq!(custom_zero.maximum_gap_bytes, 0);
    assert_eq!(custom_zero.maximum_extra_bytes, 0);
    assert_eq!(custom_zero.maximum_ranges, 16);
    assert_eq!(
        ByteRange::tail(0, source_len),
        Err(SubsetLabError::InvalidRange)
    );
    assert_eq!(
        ByteRange::tail(source_len + 1, source_len),
        Err(SubsetLabError::InvalidRange)
    );

    // 4. `ByteRange::from_zarr_v3_shard_entry` handles the `(u64::MAX, u64::MAX)` empty-chunk
    //    sentinel cleanly and fails closed on partial sentinels, overflow, or tail-index intrusion.
    assert_eq!(
        ByteRange::from_zarr_v3_shard_entry(
            u64::MAX,
            u64::MAX,
            source_len,
            ZARR_V3_SHARD_128_TAIL_INDEX_BYTES,
        ),
        Ok(None)
    );
    assert_eq!(
        ByteRange::from_zarr_v3_shard_entry(
            127 * chunk_bytes,
            chunk_bytes,
            source_len,
            ZARR_V3_SHARD_128_TAIL_INDEX_BYTES,
        ),
        Ok(Some(
            ByteRange::new(127 * chunk_bytes, 128 * chunk_bytes, source_len).unwrap()
        ))
    );
    assert_eq!(
        ByteRange::from_zarr_v3_shard_entry(
            127 * chunk_bytes,
            chunk_bytes + 1,
            source_len,
            ZARR_V3_SHARD_128_TAIL_INDEX_BYTES,
        ),
        Err(SubsetLabError::InvalidRange)
    );
    assert_eq!(
        ByteRange::from_zarr_v3_shard_entry(
            u64::MAX,
            chunk_bytes,
            source_len,
            ZARR_V3_SHARD_128_TAIL_INDEX_BYTES,
        ),
        Err(SubsetLabError::InvalidRange)
    );
    assert_eq!(
        ByteRange::from_zarr_v3_shard_entry(
            u64::MAX - 8,
            16,
            source_len,
            ZARR_V3_SHARD_128_TAIL_INDEX_BYTES,
        ),
        Err(SubsetLabError::InvalidRange)
    );
    assert_eq!(
        ByteRange::from_zarr_v3_shard_entry(0, 0, source_len, ZARR_V3_SHARD_128_TAIL_INDEX_BYTES,),
        Err(SubsetLabError::InvalidRange)
    );
}
