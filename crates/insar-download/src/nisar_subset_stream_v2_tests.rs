use super::*;

fn covered_once(tiles: &[V2StreamTile], rows: usize, cols: usize) -> bool {
    let mut seen = vec![0_u8; rows * cols];
    for tile in tiles {
        for row in tile.row_start..tile.row_end {
            for col in tile.col_start..tile.col_end {
                seen[row * cols + col] += 1;
            }
        }
    }
    seen.iter().all(|count| *count == 1)
}

/// The output chunks a tile touches, in the order a whole-window write visits
/// them (row-major over the chunk grid).
fn chunk_visit_order(tiles: &[V2StreamTile], chunk: (usize, usize)) -> Vec<(usize, usize)> {
    let mut order = Vec::new();
    for tile in tiles {
        for chunk_row in tile.row_start / chunk.0..tile.row_end.div_ceil(chunk.0) {
            for chunk_col in tile.col_start / chunk.1..tile.col_end.div_ceil(chunk.1) {
                order.push((chunk_row, chunk_col));
            }
        }
    }
    order
}

#[test]
fn bands_are_whole_chunk_rows_within_the_budget_and_cover_the_window_once() {
    // 14848 x 8192 complex samples with 512 x 512 chunks: one chunk row is
    // 32 MiB, exactly the default budget.
    let tiles =
        v2_stream_tiles([14_848, 8_192], [512, 512], 8, V2_STREAM_BAND_BUDGET_BYTES).unwrap();
    assert_eq!(tiles.len(), 29);
    assert!(tiles
        .iter()
        .all(|tile| tile.col_start == 0 && tile.col_end == 8_192));
    assert!(tiles.iter().all(|tile| tile.row_start % 512 == 0));
    assert_eq!(v2_largest_tile_bytes(&tiles, 8), 32 * 1024 * 1024);

    let tiles = v2_stream_tiles([40, 36], [6, 8], 8, 6 * 36 * 8 * 2 + 7).unwrap();
    assert!(covered_once(&tiles, 40, 36));
    assert_eq!(
        tiles
            .iter()
            .map(|tile| (tile.row_start, tile.row_end))
            .collect::<Vec<_>>(),
        vec![(0, 12), (12, 24), (24, 36), (36, 40)]
    );
}

#[test]
fn a_chunk_row_wider_than_the_budget_streams_runs_of_whole_chunks_in_row_major_order() {
    let tiles = v2_stream_tiles([20, 30], [4, 7], 8, 4 * 7 * 8 * 2).unwrap();
    assert!(covered_once(&tiles, 20, 30));
    assert!(tiles
        .iter()
        .all(|tile| tile.row_end - tile.row_start <= 4 && tile.col_start % 7 == 0));
    assert!(v2_largest_tile_bytes(&tiles, 8) <= 4 * 7 * 8 * 2);
    let order = chunk_visit_order(&tiles, (4, 7));
    let mut row_major = order.clone();
    row_major.sort();
    assert_eq!(order, row_major);
    assert_eq!(order.len(), 5 * 5);
}

#[test]
fn a_budget_below_one_chunk_keeps_the_chunk_whole_and_the_tile_limit_is_absolute() {
    let tiles = v2_stream_tiles([10, 10], [4, 4], 8, 1).unwrap();
    assert_eq!(v2_largest_tile_bytes(&tiles, 8), 4 * 4 * 8);
    assert!(covered_once(&tiles, 10, 10));
    // One 4096 x 4096 complex chunk is 128 MiB, above the absolute tile limit.
    assert_eq!(
        v2_stream_tiles([8_192, 8_192], [4_096, 4_096], 8, usize::MAX),
        None
    );
    assert_eq!(v2_stream_tiles([0, 10], [4, 4], 8, 1024), None);
    assert_eq!(v2_stream_tiles([10, 10], [0, 4], 8, 1024), None);
}

#[test]
fn output_chunks_are_clipped_to_the_window_like_the_writer_creates_them() {
    let tiles = v2_stream_tiles([3, 5], [512, 512], 1, 1024).unwrap();
    assert_eq!(
        tiles,
        vec![V2StreamTile {
            row_start: 0,
            row_end: 3,
            col_start: 0,
            col_end: 5,
        }]
    );
}

fn extent(component_id: u16, ordinal: usize, rows: u64, cols: u64) -> V2DerivedWindowExtent {
    V2DerivedWindowExtent {
        component_id,
        ordinal,
        line_start: 10,
        line_end_exclusive: 10 + rows,
        sample_start: 20,
        sample_end_exclusive: 20 + cols,
    }
}

#[test]
fn output_volume_counts_one_mask_and_one_scale_pair_per_shared_window() {
    // One polarization: 8 bytes science + 1 byte mask per sample, plus scales.
    assert_eq!(
        v2_derived_output_logical_bytes_of([extent(0, 0, 3, 5)]),
        Some(3 * 5 * 9 + (3 + 5) * 8)
    );
    // HH and HV share the window: science twice, mask and scales once.
    assert_eq!(
        v2_derived_output_logical_bytes_of([extent(0, 0, 3, 5), extent(0, 0, 3, 5)]),
        Some(3 * 5 * 17 + (3 + 5) * 8)
    );
    // A second component has its own mask and scales.
    assert_eq!(
        v2_derived_output_logical_bytes_of([extent(0, 0, 3, 5), extent(1, 1, 3, 5)]),
        Some(2 * (3 * 5 * 9 + (3 + 5) * 8))
    );
    let mut inverted = extent(0, 0, 3, 5);
    inverted.line_end_exclusive = 0;
    assert_eq!(v2_derived_output_logical_bytes_of([inverted]), None);
}

#[test]
fn the_output_bound_admits_the_june29_window_and_refuses_beyond_two_gib() {
    let june29 = v2_derived_output_logical_bytes_of([extent(0, 0, 14_848, 8_192)]).unwrap();
    assert_eq!(v2_check_output_bytes(Some(june29)), Ok(june29));
    assert_eq!(
        v2_check_output_bytes(Some(V2_OUTPUT_LOGICAL_LIMIT_BYTES)),
        Ok(V2_OUTPUT_LOGICAL_LIMIT_BYTES)
    );
    assert_eq!(
        v2_check_output_bytes(Some(V2_OUTPUT_LOGICAL_LIMIT_BYTES + 1)),
        Err(V2OutputAdmissionRefusal::OutputLimit {
            required: V2_OUTPUT_LOGICAL_LIMIT_BYTES + 1,
            limit: V2_OUTPUT_LOGICAL_LIMIT_BYTES,
        })
    );
    assert_eq!(
        v2_check_output_bytes(None),
        Err(V2OutputAdmissionRefusal::OutputLimit {
            required: u64::MAX,
            limit: V2_OUTPUT_LOGICAL_LIMIT_BYTES,
        })
    );
}

#[test]
fn disk_admission_reports_required_and_available_bytes() {
    let directory = std::env::temp_dir();
    {
        let _available = V2AvailableDiskForTest::set(10);
        assert_eq!(
            v2_check_disk_space(&directory, 5),
            Err(V2OutputAdmissionRefusal::DiskSpace {
                required: 5 + V2_DISK_HEADROOM_BYTES,
                available: 10,
            })
        );
    }
    let _available = V2AvailableDiskForTest::set(5 + V2_DISK_HEADROOM_BYTES);
    assert_eq!(v2_check_disk_space(&directory, 5), Ok(()));
}

#[test]
fn the_real_volume_reports_free_space() {
    assert!(available_disk_bytes(&std::env::temp_dir()).is_some_and(|bytes| bytes > 0));
}
