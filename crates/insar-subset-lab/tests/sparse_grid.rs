use insar_subset_lab::{
    admit_sparse_grid_selection, sparse_grid_polygon_cells_v1, validate_reference_window,
    SparseCellIntersection, SparseGeoGrid, SparseGridError, SparseGridReferenceWindow, Window2D,
    SPARSE_GRID_CELL_MASK_SEMANTICS, SPARSE_GRID_MAX_AOI_GEOJSON_BYTES,
    SPARSE_GRID_MAX_CANDIDATE_REFERENCE_CHUNK_AREA_RATIO, SPARSE_GRID_POLYGON_CELLS_V1,
};

#[test]
fn a_subgrid_aoi_selects_its_cell_even_when_no_sparse_node_is_inside() {
    let grid = regular_grid(3, &[0.0], &[0.0]);
    let aoi = polygon(0.4, 0.4, 0.6, 0.6);

    let selection = sparse_grid_polygon_cells_v1(&aoi, &grid, 0.0, 0.0).unwrap();

    assert_eq!(selection.method, SPARSE_GRID_POLYGON_CELLS_V1);
    assert_eq!(selection.mask_semantics, SPARSE_GRID_CELL_MASK_SEMANTICS);
    assert_eq!(selection.selected_cells.len(), 1);
    assert_eq!(selection.selected_cells[0].cell, [0, 0]);
    assert_eq!(
        selection.selected_cells[0].intersecting_height_indices,
        vec![0]
    );
    assert_eq!(selection.components.len(), 1);
    assert_eq!(
        selection.components[0].logical_window,
        Window2D::new(0, 10, 0, 10, [20, 20]).unwrap()
    );
    assert_eq!(selection.components[0].selected_cell_mask, vec![true]);
    assert!(grid
        .layer_nodes(0)
        .unwrap()
        .iter()
        .all(|[longitude, latitude]| {
            !(*longitude > 0.4 && *longitude < 0.6 && *latitude > 0.4 && *latitude < 0.6)
        }));
}

#[test]
fn all_bracketing_hae_layers_are_checked_instead_of_one_representative_slice() {
    let grid = regular_grid(3, &[0.0, 100.0], &[0.0, 10.0]);
    let upper_layer_aoi = polygon(10.4, 0.4, 10.6, 0.6);

    let selection = sparse_grid_polygon_cells_v1(&upper_layer_aoi, &grid, 25.0, 75.0).unwrap();

    assert_eq!(selection.height_layer_indices, vec![0, 1]);
    assert_eq!(selection.height_layers_m, vec![0.0, 100.0]);
    assert_eq!(selection.selected_cells.len(), 1);
    assert_eq!(selection.selected_cells[0].cell, [0, 0]);
    assert_eq!(
        selection.selected_cells[0].intersecting_height_indices,
        vec![1]
    );
    assert_eq!(
        selection.selected_cells[0].intersections,
        vec![SparseCellIntersection {
            height_layer_index: 1,
            geometry_part_index: 0,
        }]
    );
}

#[test]
fn multipolygon_cells_remain_two_components_and_never_claim_the_gap() {
    let grid = regular_grid(5, &[0.0], &[0.0]);
    let aoi = format!(
        r#"{{"type":"MultiPolygon","coordinates":[{},{}]}}"#,
        polygon_coordinates(0.2, 0.2, 0.8, 0.8),
        polygon_coordinates(2.2, 0.2, 2.8, 0.8)
    );

    let selection = sparse_grid_polygon_cells_v1(&aoi, &grid, 0.0, 0.0).unwrap();

    assert_eq!(
        selection
            .selected_cells
            .iter()
            .map(|cell| cell.cell)
            .collect::<Vec<_>>(),
        vec![[0, 0], [0, 2]]
    );
    assert_eq!(selection.components.len(), 2);
    assert_eq!(
        selection.components[0].logical_window,
        Window2D::new(0, 10, 0, 10, [40, 40]).unwrap()
    );
    assert_eq!(
        selection.components[1].logical_window,
        Window2D::new(0, 10, 20, 30, [40, 40]).unwrap()
    );
    assert_eq!(selection.components[0].geometry_part_index, 0);
    assert_eq!(selection.components[1].geometry_part_index, 1);
    assert_eq!(
        selection
            .components
            .iter()
            .map(|component| component.selected_cell_mask.clone())
            .collect::<Vec<_>>(),
        vec![vec![true], vec![true]]
    );
}

#[test]
fn polygon_holes_remain_false_in_the_component_cell_mask() {
    let grid = regular_grid(4, &[0.0], &[0.0]);
    let aoi = r#"{
        "type":"Polygon",
        "coordinates":[
          [[-0.2,-0.2],[3.2,-0.2],[3.2,3.2],[-0.2,3.2],[-0.2,-0.2]],
          [[0.9,0.9],[2.1,0.9],[2.1,2.1],[0.9,2.1],[0.9,0.9]]
        ]
      }"#;

    let selection = sparse_grid_polygon_cells_v1(aoi, &grid, 0.0, 0.0).unwrap();

    assert!(!selection
        .selected_cells
        .iter()
        .any(|cell| cell.cell == [1, 1]));
    assert_eq!(selection.components.len(), 1);
    let component = &selection.components[0];
    assert_eq!(component.mask_origin_cell, [0, 0]);
    assert_eq!(component.mask_shape_cells, [3, 3]);
    assert_eq!(component.selected_cell_mask.len(), 9);
    assert!(!component.selected_cell_mask[4]);
    assert_eq!(
        component
            .selected_cell_mask
            .iter()
            .filter(|selected| **selected)
            .count(),
        8
    );
}

#[test]
fn reference_gate_requires_containment_and_the_predeclared_area_ceiling() {
    let shape = [100, 100];
    let candidate = Window2D::new(0, 20, 0, 20, shape).unwrap();
    let reference = Window2D::new(0, 15, 0, 20, shape).unwrap();
    let ratio = validate_reference_window(candidate, reference).unwrap();
    assert!((ratio - 4.0 / 3.0).abs() < 1.0e-12);
    assert_eq!(SPARSE_GRID_MAX_CANDIDATE_REFERENCE_CHUNK_AREA_RATIO, 1.5);

    assert_eq!(
        validate_reference_window(
            Window2D::new(0, 30, 0, 30, shape).unwrap(),
            Window2D::new(0, 10, 0, 10, shape).unwrap(),
        ),
        Err(SparseGridError::CandidateAreaExceeded)
    );
    assert_eq!(
        validate_reference_window(
            Window2D::new(0, 10, 0, 10, shape).unwrap(),
            Window2D::new(5, 15, 5, 15, shape).unwrap(),
        ),
        Err(SparseGridError::ReferenceUndercovered)
    );

    let grid = regular_grid(3, &[0.0], &[0.0]);
    let selection =
        sparse_grid_polygon_cells_v1(&polygon(0.2, 0.2, 0.8, 0.8), &grid, 0.0, 0.0).unwrap();
    let admission = admit_sparse_grid_selection(
        &selection,
        &[SparseGridReferenceWindow {
            component_index: 0,
            geometry_part_index: 0,
            reference_chunk_window: Window2D::new(0, 10, 0, 10, [20, 20]).unwrap(),
        }],
    )
    .unwrap();
    assert_eq!(admission.component_area_ratios, vec![1.0]);
    assert_eq!(admission.maximum_area_ratio, 1.0);
    assert_eq!(
        admit_sparse_grid_selection(&selection, &[]),
        Err(SparseGridError::ReferenceSetIncomplete)
    );
}

#[test]
fn unsupported_geometry_height_and_true_no_overlap_are_typed() {
    let grid = regular_grid(3, &[0.0, 100.0], &[0.0, 0.0]);
    assert_eq!(
        sparse_grid_polygon_cells_v1(
            r#"{"type":"LineString","coordinates":[[0,0],[1,1]]}"#,
            &grid,
            0.0,
            100.0,
        ),
        Err(SparseGridError::InvalidGeometry)
    );
    assert_eq!(
        sparse_grid_polygon_cells_v1(
            r#"{"type":"Polygon","coordinates":[[[0,0],[3,2],[0,3],[2,0],[0,0]]]}"#,
            &grid,
            0.0,
            100.0,
        ),
        Err(SparseGridError::InvalidGeometry)
    );
    assert_eq!(
        sparse_grid_polygon_cells_v1(
            r#"{"type":"Polygon","coordinates":[[[0,0],[2,0],[2,2],[0,2],[0,0]],[[3,3],[4,3],[4,4],[3,4],[3,3]]]}"#,
            &grid,
            0.0,
            100.0,
        ),
        Err(SparseGridError::InvalidGeometry)
    );
    assert_eq!(
        sparse_grid_polygon_cells_v1(
            r#"{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,4],[0,0]],[[1,1],[2,1],[3,1],[1,1]]]}"#,
            &grid,
            0.0,
            100.0,
        ),
        Err(SparseGridError::InvalidGeometry)
    );
    assert_eq!(
        sparse_grid_polygon_cells_v1(
            &" ".repeat(SPARSE_GRID_MAX_AOI_GEOJSON_BYTES + 1),
            &grid,
            0.0,
            100.0,
        ),
        Err(SparseGridError::InvalidGeometry)
    );
    assert_eq!(
        sparse_grid_polygon_cells_v1(&polygon(0.2, 0.2, 0.8, 0.8), &grid, -1.0, 50.0),
        Err(SparseGridError::HeightEnvelopeUnsupported)
    );
    assert_eq!(
        sparse_grid_polygon_cells_v1(&polygon(20.0, 20.0, 21.0, 21.0), &grid, 0.0, 100.0),
        Err(SparseGridError::NoOverlap)
    );
}

#[test]
fn cell_layer_part_work_is_bounded_before_the_intersection_scan() {
    let grid = regular_grid(90, &[0.0, 100.0], &[0.0, 0.0]);
    let polygons = (0..600)
        .map(|index| {
            let column = (index % 80) as f64;
            let row = (index / 80) as f64;
            polygon_coordinates(column + 0.1, row + 0.1, column + 0.2, row + 0.2)
        })
        .collect::<Vec<_>>()
        .join(",");
    let aoi = format!(r#"{{"type":"MultiPolygon","coordinates":[{polygons}]}}"#);

    assert_eq!(
        sparse_grid_polygon_cells_v1(&aoi, &grid, 0.0, 100.0),
        Err(SparseGridError::CandidateBudgetExceeded)
    );
}

fn regular_grid(size: usize, heights: &[f64], longitude_shifts: &[f64]) -> SparseGeoGrid {
    assert_eq!(heights.len(), longitude_shifts.len());
    let node_rows = (0..size).map(|index| index * 10).collect::<Vec<_>>();
    let node_columns = node_rows.clone();
    let longitudes = longitude_shifts
        .iter()
        .map(|shift| {
            (0..size)
                .flat_map(|_| (0..size).map(|column| column as f64 + shift))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let latitudes = heights
        .iter()
        .map(|_| {
            (0..size)
                .flat_map(|row| (0..size).map(move |_| row as f64))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    SparseGeoGrid::new(
        node_rows,
        node_columns,
        heights.to_vec(),
        longitudes,
        latitudes,
        [(size - 1) * 10, (size - 1) * 10],
        [10, 10],
    )
    .unwrap()
}

fn polygon(west: f64, south: f64, east: f64, north: f64) -> String {
    format!(
        r#"{{"type":"Polygon","coordinates":{}}}"#,
        polygon_coordinates(west, south, east, north)
    )
}

fn polygon_coordinates(west: f64, south: f64, east: f64, north: f64) -> String {
    format!(
        "[[[{west},{south}],[{east},{south}],[{east},{north}],[{west},{north}],[{west},{south}]]]"
    )
}
