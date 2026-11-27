use insar_provider::asf::nisar_subset::nearest_radar_axis_indices;

fn full_axis() -> Vec<f64> {
    (0..100).map(|index| index as f64).collect()
}

#[test]
fn coarse_grid_maps_to_unique_nearest_radar_indices() {
    let grid = (0..10).map(|index| index as f64 * 10.0).collect::<Vec<_>>();
    let (mapped, diagnostics) = nearest_radar_axis_indices(&grid, &full_axis()).unwrap();
    let expected = (0..10).map(|index| index * 10).collect::<Vec<_>>();
    assert_eq!(mapped, expected);
    assert!(mapped.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(diagnostics.grid_length, 10);
    assert_eq!(diagnostics.full_length, 100);
    assert_eq!(diagnostics.max_nearest_residual, 0.0);
    assert_eq!(diagnostics.mapped_first_index, 0);
    assert_eq!(diagnostics.mapped_last_index, 90);
    assert!(diagnostics.strictly_increasing);
    assert_eq!(diagnostics.median_full_spacing, Some(1.0));
}

#[test]
fn offset_grid_maps_to_actual_nearest_values_with_tie_to_lower_index() {
    let grid = vec![4.3, 14.7, 25.5, 40.2, 71.1];
    let (mapped, diagnostics) = nearest_radar_axis_indices(&grid, &full_axis()).unwrap();
    // 4.3 -> 4 (0.3 vs 0.7); 14.7 -> 15 (0.3 vs 0.7); 25.5 ties -> 25
    // (lower index); 40.2 -> 40; 71.1 -> 71.
    assert_eq!(mapped, vec![4, 15, 25, 40, 71]);
    assert!(diagnostics.max_nearest_residual <= 0.5);
    assert!(diagnostics.strictly_increasing);
}

#[test]
fn non_monotonic_or_non_finite_axes_are_rejected() {
    assert!(nearest_radar_axis_indices(&[1.0, 3.0, 2.0], &full_axis()).is_none());
    assert!(nearest_radar_axis_indices(&[1.0, 2.0], &[5.0, 4.0, 6.0]).is_none());
    assert!(nearest_radar_axis_indices(&[1.0, f64::NAN], &full_axis()).is_none());
    assert!(nearest_radar_axis_indices(&[1.0, 2.0], &[1.0, f64::INFINITY]).is_none());
    assert!(nearest_radar_axis_indices(&[1.0], &full_axis()).is_none());
}

#[test]
fn both_decreasing_axes_map_consistently() {
    let full = (0..100).rev().map(|index| index as f64).collect::<Vec<_>>();
    let grid = (0..10)
        .rev()
        .map(|index| index as f64 * 10.0)
        .collect::<Vec<_>>();
    let (mapped, diagnostics) = nearest_radar_axis_indices(&grid, &full).unwrap();
    // full[9]=90, full[19]=80, ..., full[99]=0 for a decreasing axis.
    assert_eq!(
        mapped,
        (0..10).map(|index| index * 10 + 9).collect::<Vec<_>>()
    );
    assert!(mapped.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(!diagnostics.grid_increasing);
    assert!(!diagnostics.full_increasing);
}

#[test]
fn grid_denser_than_radar_axis_collapses_and_fails_closed() {
    // Full axis spacing is 10; grid nodes 4,5,6 all collapse to index 0.
    let full = vec![0.0, 10.0, 20.0];
    let grid = vec![4.0, 5.0, 6.0];
    assert!(nearest_radar_axis_indices(&grid, &full).is_none());
}

#[test]
fn margin_nodes_outside_the_radar_axis_span_are_trimmed() {
    // Grid axis extends beyond the radar axis at both ends; the in-span nodes
    // map by actual value and the margin nodes are dropped with diagnostics.
    let full = vec![0.0, 10.0, 20.0];
    let grid = vec![-5.0, 4.0, 14.0, 19.0, 34.0];
    let (mapped, diagnostics) = nearest_radar_axis_indices(&grid, &full).unwrap();
    // Radar indices (values 0/10/20) for the in-span nodes 4/14/19.
    assert_eq!(mapped, vec![0, 1, 2]);
    assert_eq!(diagnostics.kept_first_grid_index, 1);
    assert_eq!(diagnostics.kept_last_grid_index, 3);
    assert_eq!(diagnostics.margin_nodes_dropped, 2);
    assert!(diagnostics.strictly_increasing);
    assert!(diagnostics.max_nearest_residual <= 5.0);
}

#[test]
fn single_in_span_node_is_insufficient_for_a_grid_axis() {
    let full = vec![0.0, 10.0, 20.0];
    let grid = vec![-5.0, 5.0, 30.0];
    assert!(nearest_radar_axis_indices(&grid, &full).is_none());
}

#[test]
fn grid_exactly_spanning_the_radar_axis_has_zero_margin_drop() {
    let full = vec![0.0, 10.0, 20.0];
    let grid = vec![0.0, 10.0, 20.0];
    let (mapped, diagnostics) = nearest_radar_axis_indices(&grid, &full).unwrap();
    assert_eq!(mapped, vec![0, 1, 2]);
    assert_eq!(diagnostics.margin_nodes_dropped, 0);
    assert_eq!(diagnostics.kept_first_grid_index, 0);
    assert_eq!(diagnostics.kept_last_grid_index, 2);
}
