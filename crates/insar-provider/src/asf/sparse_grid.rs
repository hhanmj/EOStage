//! Production-owned hardened NISAR subset geometry.
//!
//! This is the single canonical implementation of
//! `sparse_grid_polygon_cells_v1`. It was promoted from the historical
//! `insar-subset-lab` experiment so the live Range production path, the
//! provider planner, qualification tests, and subset-lab tests share one
//! algorithm and never diverge under the same algorithm ID.
//!
//! It deliberately contains no network, credential, resolver, HDF5, or writer
//! capability. It consumes a bounded AOI and a reviewed sparse geolocation
//! grid and returns conservative radar-window cells.

use std::collections::{BTreeSet, VecDeque};

use geo::{
    line_intersection::{line_intersection, LineIntersection},
    Area, Contains, Coord, Intersects, Line, LineString, Point, Polygon,
};
use geojson::{Geometry as GeoJsonGeometry, Value as GeoJsonValue};
use insar_core::ContractError;
use serde::Serialize;
use serde_json::{Map, Value};

pub const SPARSE_GRID_POLYGON_CELLS_V1: &str = "sparse_grid_polygon_cells_v1";
pub const SPARSE_GRID_CELL_MASK_SEMANTICS: &str = "sparse_cell_intersects_aoi_any_bracketing_hae";
pub const SPARSE_GRID_MAX_CANDIDATE_REFERENCE_CHUNK_AREA_RATIO: f64 = 1.5;
pub const SPARSE_GRID_MAX_AOI_GEOJSON_BYTES: usize = 256 * 1024;

const MAX_GRID_NODES: usize = 1_000_000;
const MAX_HEIGHT_LAYERS: usize = 64;
const MAX_AOI_VERTICES: usize = 4_096;
// Real NASA ASF NISAR L1 RSLC geolocation grids measure ~556 x 349 (193,140 cells),
// so practical study AOIs easily select >16,384 cells; 262,144 allows full-scene
// L1 RSLC grids while keeping allocations well below MAX_COMPONENT_MASK_CELLS.
const MAX_SELECTED_CELLS: usize = 262_144;
const MAX_CELL_LAYER_PART_INTERSECTIONS: usize = MAX_SELECTED_CELLS * MAX_HEIGHT_LAYERS;
const MAX_CELL_LAYER_PART_TESTS: usize = 8_000_000;
const MAX_COMPONENT_MASK_CELLS: usize = 1_048_576;

/// Two-dimensional half-open radar window `[row_start,row_end) x
/// [column_start,column_end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Window2D {
    pub row_start: usize,
    pub row_end: usize,
    pub column_start: usize,
    pub column_end: usize,
}

impl Window2D {
    pub fn new(
        row_start: usize,
        row_end: usize,
        column_start: usize,
        column_end: usize,
        shape: [usize; 2],
    ) -> Result<Self, ContractError> {
        if row_start >= row_end
            || column_start >= column_end
            || row_end > shape[0]
            || column_end > shape[1]
        {
            return Err(geometry_error("invalid_window"));
        }
        Ok(Self {
            row_start,
            row_end,
            column_start,
            column_end,
        })
    }

    pub fn shape(self) -> [usize; 2] {
        [
            self.row_end - self.row_start,
            self.column_end - self.column_start,
        ]
    }
}

/// Sparse geolocation grid: per-height-layer node longitudes/latitudes over
/// the same radar row/column node axes.
#[derive(Clone, Debug, PartialEq)]
pub struct SparseGeoGrid {
    node_rows: Vec<usize>,
    node_columns: Vec<usize>,
    height_layers_m: Vec<f64>,
    longitudes: Vec<Vec<f64>>,
    latitudes: Vec<Vec<f64>>,
    radar_shape: [usize; 2],
    chunk_shape: [usize; 2],
}

impl SparseGeoGrid {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node_rows: Vec<usize>,
        node_columns: Vec<usize>,
        height_layers_m: Vec<f64>,
        longitudes: Vec<Vec<f64>>,
        latitudes: Vec<Vec<f64>>,
        radar_shape: [usize; 2],
        chunk_shape: [usize; 2],
    ) -> Result<Self, ContractError> {
        let rows = node_rows.len();
        let columns = node_columns.len();
        let node_count = rows
            .checked_mul(columns)
            .filter(|count| *count <= MAX_GRID_NODES)
            .ok_or_else(|| geometry_error("invalid_grid"))?;
        if rows < 2
            || columns < 2
            || height_layers_m.is_empty()
            || height_layers_m.len() > MAX_HEIGHT_LAYERS
            || longitudes.len() != height_layers_m.len()
            || latitudes.len() != height_layers_m.len()
            || radar_shape.contains(&0)
            || chunk_shape.contains(&0)
            || !strictly_increasing_usize(&node_rows)
            || !strictly_increasing_usize(&node_columns)
            || node_rows.last().is_none_or(|value| *value > radar_shape[0])
            || node_columns
                .last()
                .is_none_or(|value| *value > radar_shape[1])
            || !strictly_increasing_f64(&height_layers_m)
        {
            return Err(geometry_error("invalid_grid"));
        }
        for (longitude, latitude) in longitudes.iter().zip(&latitudes) {
            if longitude.len() != node_count
                || latitude.len() != node_count
                || longitude
                    .iter()
                    .any(|value| !value.is_finite() || !(-180.0..=180.0).contains(value))
                || latitude
                    .iter()
                    .any(|value| !value.is_finite() || !(-90.0..=90.0).contains(value))
            {
                return Err(geometry_error("invalid_grid"));
            }
        }
        Ok(Self {
            node_rows,
            node_columns,
            height_layers_m,
            longitudes,
            latitudes,
            radar_shape,
            chunk_shape,
        })
    }

    pub fn layer_nodes(&self, layer_index: usize) -> Result<Vec<[f64; 2]>, ContractError> {
        let longitude = self
            .longitudes
            .get(layer_index)
            .ok_or_else(|| geometry_error("invalid_grid"))?;
        let latitude = self
            .latitudes
            .get(layer_index)
            .ok_or_else(|| geometry_error("invalid_grid"))?;
        Ok(longitude
            .iter()
            .zip(latitude)
            .map(|(longitude, latitude)| [*longitude, *latitude])
            .collect())
    }

    pub fn radar_shape(&self) -> [usize; 2] {
        self.radar_shape
    }

    pub fn chunk_shape(&self) -> [usize; 2] {
        self.chunk_shape
    }

    pub fn height_layers_m(&self) -> &[f64] {
        &self.height_layers_m
    }

    fn rows(&self) -> usize {
        self.node_rows.len()
    }

    fn columns(&self) -> usize {
        self.node_columns.len()
    }

    fn node(&self, layer: usize, row: usize, column: usize) -> Coord<f64> {
        let index = row * self.columns() + column;
        Coord {
            x: self.longitudes[layer][index],
            y: self.latitudes[layer][index],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SparseCellIntersection {
    pub height_layer_index: usize,
    pub geometry_part_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SelectedSparseCell {
    pub cell: [usize; 2],
    pub intersecting_height_indices: Vec<usize>,
    pub intersecting_geometry_parts: Vec<usize>,
    pub intersections: Vec<SparseCellIntersection>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SparseGridComponent {
    pub geometry_part_index: usize,
    pub selected_cells: Vec<[usize; 2]>,
    pub mask_origin_cell: [usize; 2],
    pub mask_shape_cells: [usize; 2],
    pub selected_cell_mask: Vec<bool>,
    pub logical_window: Window2D,
    pub chunk_window: Window2D,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SparseGridSelection {
    pub method: String,
    pub chunk_halo: usize,
    pub mask_semantics: String,
    pub height_layer_indices: Vec<usize>,
    pub height_layers_m: Vec<f64>,
    pub selected_cells: Vec<SelectedSparseCell>,
    pub components: Vec<SparseGridComponent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SparseGridReferenceWindow {
    pub component_index: usize,
    pub geometry_part_index: usize,
    pub reference_chunk_window: Window2D,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SparseGridAdmission {
    pub component_area_ratios: Vec<f64>,
    pub maximum_area_ratio: f64,
}

pub fn sparse_grid_polygon_cells_v1(
    aoi_geojson: &str,
    grid: &SparseGeoGrid,
    minimum_hae_m: f64,
    maximum_hae_m: f64,
) -> Result<SparseGridSelection, ContractError> {
    let aoi = parse_aoi(aoi_geojson)?;
    let height_layer_indices =
        bracket_height_layers(&grid.height_layers_m, minimum_hae_m, maximum_hae_m)?;
    let cell_layer_part_tests = (grid.rows() - 1)
        .checked_mul(grid.columns() - 1)
        .and_then(|count| count.checked_mul(height_layer_indices.len()))
        .and_then(|count| count.checked_mul(aoi.part_count()))
        .filter(|count| *count <= MAX_CELL_LAYER_PART_TESTS)
        .ok_or_else(|| geometry_error("candidate_budget"))?;
    debug_assert!(cell_layer_part_tests > 0);
    let mut selected_cells = Vec::new();
    let mut cell_layer_part_intersections = 0_usize;
    for row in 0..grid.rows() - 1 {
        for column in 0..grid.columns() - 1 {
            let mut intersections = Vec::new();
            for layer in &height_layer_indices {
                let cell = cell_polygon(grid, *layer, row, column)?;
                for geometry_part_index in aoi.intersecting_parts(&cell) {
                    intersections.push(SparseCellIntersection {
                        height_layer_index: *layer,
                        geometry_part_index,
                    });
                }
            }
            if !intersections.is_empty() {
                if selected_cells.len() >= MAX_SELECTED_CELLS {
                    return Err(geometry_error("candidate_budget"));
                }
                cell_layer_part_intersections = cell_layer_part_intersections
                    .checked_add(intersections.len())
                    .filter(|count| *count <= MAX_CELL_LAYER_PART_INTERSECTIONS)
                    .ok_or_else(|| geometry_error("candidate_budget"))?;
                let intersecting_height_indices = intersections
                    .iter()
                    .map(|intersection| intersection.height_layer_index)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let intersecting_geometry_parts = intersections
                    .iter()
                    .map(|intersection| intersection.geometry_part_index)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                selected_cells.push(SelectedSparseCell {
                    cell: [row, column],
                    intersecting_height_indices,
                    intersecting_geometry_parts,
                    intersections,
                });
            }
        }
    }
    if selected_cells.is_empty() {
        return Err(geometry_error("no_overlap"));
    }

    let mut components = Vec::new();
    for geometry_part_index in 0..aoi.part_count() {
        let selected_coordinates = selected_cells
            .iter()
            .filter(|selected| {
                selected
                    .intersecting_geometry_parts
                    .contains(&geometry_part_index)
            })
            .map(|selected| (selected.cell[0], selected.cell[1]))
            .collect::<BTreeSet<_>>();
        for cells in connected_components(&selected_coordinates) {
            components.push(component_from_cells(grid, geometry_part_index, cells)?);
        }
    }
    components.sort_unstable_by_key(|component| {
        (component.geometry_part_index, component.selected_cells[0])
    });
    Ok(SparseGridSelection {
        method: SPARSE_GRID_POLYGON_CELLS_V1.to_owned(),
        chunk_halo: 0,
        mask_semantics: SPARSE_GRID_CELL_MASK_SEMANTICS.to_owned(),
        height_layers_m: height_layer_indices
            .iter()
            .map(|index| grid.height_layers_m[*index])
            .collect(),
        height_layer_indices,
        selected_cells,
        components,
    })
}

pub fn validate_reference_window(
    candidate: Window2D,
    reference: Window2D,
) -> Result<f64, ContractError> {
    if !valid_window(candidate) || !valid_window(reference) {
        return Err(geometry_error("invalid_grid"));
    }
    if candidate.row_start > reference.row_start
        || candidate.column_start > reference.column_start
        || candidate.row_end < reference.row_end
        || candidate.column_end < reference.column_end
    {
        return Err(geometry_error("reference_undercovered"));
    }
    let candidate_area = window_area(candidate)?;
    let reference_area = window_area(reference)?;
    let ratio = candidate_area as f64 / reference_area as f64;
    if ratio > SPARSE_GRID_MAX_CANDIDATE_REFERENCE_CHUNK_AREA_RATIO {
        return Err(geometry_error("candidate_area_exceeded"));
    }
    Ok(ratio)
}

pub fn admit_sparse_grid_selection(
    selection: &SparseGridSelection,
    references: &[SparseGridReferenceWindow],
) -> Result<SparseGridAdmission, ContractError> {
    if selection.method != SPARSE_GRID_POLYGON_CELLS_V1
        || selection.chunk_halo != 0
        || selection.mask_semantics != SPARSE_GRID_CELL_MASK_SEMANTICS
    {
        return Err(geometry_error("reference_set_invalid"));
    }
    if selection.components.is_empty() || references.len() != selection.components.len() {
        return Err(geometry_error("reference_set_incomplete"));
    }
    let mut seen_components = BTreeSet::new();
    let mut component_area_ratios = vec![0.0; selection.components.len()];
    for reference in references {
        let component = selection
            .components
            .get(reference.component_index)
            .ok_or_else(|| geometry_error("reference_set_invalid"))?;
        if component.geometry_part_index != reference.geometry_part_index
            || !seen_components.insert(reference.component_index)
        {
            return Err(geometry_error("reference_set_invalid"));
        }
        component_area_ratios[reference.component_index] =
            validate_reference_window(component.chunk_window, reference.reference_chunk_window)?;
    }
    let maximum_area_ratio = component_area_ratios
        .iter()
        .copied()
        .fold(0.0_f64, f64::max);
    Ok(SparseGridAdmission {
        component_area_ratios,
        maximum_area_ratio,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GeoEnvelope {
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
}

impl GeoEnvelope {
    fn from_ring(ring: &LineString<f64>) -> Self {
        ring.0.iter().fold(
            Self {
                min_lon: f64::INFINITY,
                min_lat: f64::INFINITY,
                max_lon: f64::NEG_INFINITY,
                max_lat: f64::NEG_INFINITY,
            },
            |env, coord| Self {
                min_lon: env.min_lon.min(coord.x),
                min_lat: env.min_lat.min(coord.y),
                max_lon: env.max_lon.max(coord.x),
                max_lat: env.max_lat.max(coord.y),
            },
        )
    }

    fn intersects(self, other: Self) -> bool {
        self.max_lon >= other.min_lon
            && self.min_lon <= other.max_lon
            && self.max_lat >= other.min_lat
            && self.min_lat <= other.max_lat
    }
}

/// Checks whether a 4-corner radar geolocation cell forms a strictly convex
/// quadrilateral with consistent non-zero winding (odlil/SAR-Downloader fast-path).
fn quad_is_convex(corners: &[Coord<f64>; 4]) -> bool {
    let mut positive = false;
    let mut negative = false;
    for index in 0..4 {
        let a = corners[index];
        let b = corners[(index + 1) % 4];
        let c = corners[(index + 2) % 4];
        let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
        if !cross.is_finite() || cross == 0.0 {
            return false;
        }
        if cross > 0.0 {
            positive = true;
        } else {
            negative = true;
        }
        if positive && negative {
            return false;
        }
    }
    true
}

struct AoiGeometry {
    polygons: Vec<Polygon<f64>>,
    envelopes: Vec<GeoEnvelope>,
}

impl AoiGeometry {
    fn new(polygons: Vec<Polygon<f64>>) -> Self {
        let envelopes = polygons
            .iter()
            .map(|polygon| GeoEnvelope::from_ring(polygon.exterior()))
            .collect();
        Self {
            polygons,
            envelopes,
        }
    }

    fn intersecting_parts(&self, cell: &Polygon<f64>) -> Vec<usize> {
        let coords = &cell.exterior().0;
        let cell_env = GeoEnvelope::from_ring(cell.exterior());
        let convex_corners = if coords.len() == 5 {
            let corners = [coords[0], coords[1], coords[2], coords[3]];
            quad_is_convex(&corners).then_some(corners)
        } else {
            None
        };
        self.polygons
            .iter()
            .zip(&self.envelopes)
            .enumerate()
            .filter_map(|(index, (polygon, part_env))| {
                if !cell_env.intersects(*part_env) {
                    return None;
                }
                if let Some(corners) = &convex_corners {
                    if corners
                        .iter()
                        .any(|&corner| polygon.intersects(&Point::from(corner)))
                    {
                        return Some(index);
                    }
                }
                polygon.intersects(cell).then_some(index)
            })
            .collect()
    }

    fn part_count(&self) -> usize {
        self.polygons.len()
    }
}

fn parse_aoi(value: &str) -> Result<AoiGeometry, ContractError> {
    if value.len() > SPARSE_GRID_MAX_AOI_GEOJSON_BYTES {
        return Err(geometry_error("invalid_geometry"));
    }
    let geometry: GeoJsonGeometry =
        serde_json::from_str(value).map_err(|_| geometry_error("invalid_geometry"))?;
    if geometry.bbox.is_some()
        || geometry
            .foreign_members
            .as_ref()
            .is_some_and(|members| !members.is_empty())
    {
        return Err(geometry_error("invalid_geometry"));
    }
    let mut vertex_count = 0_usize;
    match geometry.value {
        GeoJsonValue::Polygon(coordinates) => Ok(AoiGeometry::new(vec![build_polygon(
            coordinates,
            &mut vertex_count,
        )?])),
        GeoJsonValue::MultiPolygon(polygons) if !polygons.is_empty() => {
            let polygons = polygons
                .into_iter()
                .map(|coordinates| build_polygon(coordinates, &mut vertex_count))
                .collect::<Result<Vec<_>, _>>()?;
            if polygons.iter().enumerate().any(|(index, polygon)| {
                polygons
                    .iter()
                    .skip(index + 1)
                    .any(|other| polygon.intersects(other))
            }) {
                return Err(geometry_error("invalid_geometry"));
            }
            Ok(AoiGeometry::new(polygons))
        }
        _ => Err(geometry_error("invalid_geometry")),
    }
}

fn build_polygon(
    rings: Vec<Vec<Vec<f64>>>,
    vertex_count: &mut usize,
) -> Result<Polygon<f64>, ContractError> {
    if rings.is_empty() {
        return Err(geometry_error("invalid_geometry"));
    }
    let mut rings = rings.into_iter();
    let exterior = build_ring(
        rings
            .next()
            .ok_or_else(|| geometry_error("invalid_geometry"))?,
        vertex_count,
    )?;
    let interiors = rings
        .map(|ring| build_ring(ring, vertex_count))
        .collect::<Result<Vec<_>, _>>()?;
    let exterior_only = Polygon::new(exterior.clone(), vec![]);
    for (index, interior) in interiors.iter().enumerate() {
        let first = Point::from(interior.0[0]);
        if !exterior_only.contains(&first)
            || rings_intersect(&exterior, interior)
            || interiors.iter().take(index).any(|previous| {
                rings_intersect(previous, interior)
                    || Polygon::new(previous.clone(), vec![]).contains(&first)
                    || Polygon::new(interior.clone(), vec![]).contains(&Point::from(previous.0[0]))
            })
        {
            return Err(geometry_error("invalid_geometry"));
        }
    }
    let polygon = Polygon::new(exterior, interiors);
    if !polygon.unsigned_area().is_finite() || polygon.unsigned_area() <= 0.0 {
        return Err(geometry_error("invalid_geometry"));
    }
    Ok(polygon)
}

fn build_ring(
    positions: Vec<Vec<f64>>,
    vertex_count: &mut usize,
) -> Result<LineString<f64>, ContractError> {
    *vertex_count = vertex_count
        .checked_add(positions.len())
        .filter(|count| *count <= MAX_AOI_VERTICES)
        .ok_or_else(|| geometry_error("invalid_geometry"))?;
    if positions.len() < 4 || positions.first() != positions.last() {
        return Err(geometry_error("invalid_geometry"));
    }
    let mut minimum_longitude = f64::INFINITY;
    let mut maximum_longitude = f64::NEG_INFINITY;
    let coordinates = positions
        .into_iter()
        .map(|position| {
            if position.len() != 2
                || !position[0].is_finite()
                || !position[1].is_finite()
                || !(-180.0..=180.0).contains(&position[0])
                || !(-90.0..=90.0).contains(&position[1])
            {
                return Err(geometry_error("invalid_geometry"));
            }
            minimum_longitude = minimum_longitude.min(position[0]);
            maximum_longitude = maximum_longitude.max(position[0]);
            Ok(Coord {
                x: position[0],
                y: position[1],
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if maximum_longitude - minimum_longitude > 180.0 {
        return Err(geometry_error("invalid_geometry"));
    }
    let ring = LineString::new(coordinates);
    if !ring_is_simple(&ring) {
        return Err(geometry_error("invalid_geometry"));
    }
    Ok(ring)
}

fn bracket_height_layers(
    layers: &[f64],
    minimum: f64,
    maximum: f64,
) -> Result<Vec<usize>, ContractError> {
    if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum {
        return Err(geometry_error("height_envelope"));
    }
    let lower = layers.iter().rposition(|height| *height <= minimum);
    let upper = layers.iter().position(|height| *height >= maximum);
    let (Some(lower), Some(upper)) = (lower, upper) else {
        return Err(geometry_error("height_envelope"));
    };
    if lower > upper {
        return Err(geometry_error("height_envelope"));
    }
    Ok((lower..=upper).collect())
}

fn cell_polygon(
    grid: &SparseGeoGrid,
    layer: usize,
    row: usize,
    column: usize,
) -> Result<Polygon<f64>, ContractError> {
    let corners = [
        grid.node(layer, row, column),
        grid.node(layer, row, column + 1),
        grid.node(layer, row + 1, column + 1),
        grid.node(layer, row + 1, column),
    ];
    let (minimum, maximum) = corners.iter().fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(minimum, maximum), coordinate| (minimum.min(coordinate.x), maximum.max(coordinate.x)),
    );
    let is_convex = quad_is_convex(&corners);
    let coordinates = vec![corners[0], corners[1], corners[2], corners[3], corners[0]];
    let polygon = Polygon::new(LineString::new(coordinates), vec![]);
    if maximum - minimum > 180.0
        || (!is_convex && !ring_is_simple(polygon.exterior()))
        || !polygon.unsigned_area().is_finite()
        || polygon.unsigned_area() <= 0.0
    {
        return Err(geometry_error("invalid_grid"));
    }
    Ok(polygon)
}

fn connected_components(selected: &BTreeSet<(usize, usize)>) -> Vec<Vec<[usize; 2]>> {
    let mut remaining = selected.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.pop_first() {
        let mut queue = VecDeque::from([start]);
        let mut component = Vec::new();
        while let Some((row, column)) = queue.pop_front() {
            component.push([row, column]);
            for neighbour in neighbours(row, column) {
                if remaining.remove(&neighbour) {
                    queue.push_back(neighbour);
                }
            }
        }
        component.sort_unstable();
        components.push(component);
    }
    components.sort_unstable_by_key(|component| component[0]);
    components
}

fn neighbours(row: usize, column: usize) -> Vec<(usize, usize)> {
    let mut values = Vec::with_capacity(4);
    if row > 0 {
        values.push((row - 1, column));
    }
    if column > 0 {
        values.push((row, column - 1));
    }
    values.push((row + 1, column));
    values.push((row, column + 1));
    values
}

fn component_from_cells(
    grid: &SparseGeoGrid,
    geometry_part_index: usize,
    cells: Vec<[usize; 2]>,
) -> Result<SparseGridComponent, ContractError> {
    let minimum_row = cells
        .iter()
        .map(|cell| cell[0])
        .min()
        .ok_or_else(|| geometry_error("invalid_grid"))?;
    let maximum_row = cells
        .iter()
        .map(|cell| cell[0])
        .max()
        .ok_or_else(|| geometry_error("invalid_grid"))?;
    let minimum_column = cells
        .iter()
        .map(|cell| cell[1])
        .min()
        .ok_or_else(|| geometry_error("invalid_grid"))?;
    let maximum_column = cells
        .iter()
        .map(|cell| cell[1])
        .max()
        .ok_or_else(|| geometry_error("invalid_grid"))?;
    let mask_shape = [
        maximum_row - minimum_row + 1,
        maximum_column - minimum_column + 1,
    ];
    let mask_cells = mask_shape[0]
        .checked_mul(mask_shape[1])
        .filter(|count| *count <= MAX_COMPONENT_MASK_CELLS)
        .ok_or_else(|| geometry_error("candidate_budget"))?;
    let mut mask = vec![false; mask_cells];
    for cell in &cells {
        let index = (cell[0] - minimum_row) * mask_shape[1] + cell[1] - minimum_column;
        mask[index] = true;
    }
    let logical_window = Window2D::new(
        grid.node_rows[minimum_row],
        grid.node_rows[maximum_row + 1],
        grid.node_columns[minimum_column],
        grid.node_columns[maximum_column + 1],
        grid.radar_shape,
    )
    .map_err(|_| geometry_error("invalid_grid"))?;
    let chunk_window = expand_to_chunks(logical_window, grid.chunk_shape, grid.radar_shape)?;
    Ok(SparseGridComponent {
        geometry_part_index,
        selected_cells: cells,
        mask_origin_cell: [minimum_row, minimum_column],
        mask_shape_cells: mask_shape,
        selected_cell_mask: mask,
        logical_window,
        chunk_window,
    })
}

fn expand_to_chunks(
    window: Window2D,
    chunk: [usize; 2],
    shape: [usize; 2],
) -> Result<Window2D, ContractError> {
    let row_start = window.row_start / chunk[0] * chunk[0];
    let column_start = window.column_start / chunk[1] * chunk[1];
    let row_end = window
        .row_end
        .div_ceil(chunk[0])
        .checked_mul(chunk[0])
        .map(|value| value.min(shape[0]))
        .ok_or_else(|| geometry_error("invalid_grid"))?;
    let column_end = window
        .column_end
        .div_ceil(chunk[1])
        .checked_mul(chunk[1])
        .map(|value| value.min(shape[1]))
        .ok_or_else(|| geometry_error("invalid_grid"))?;
    Window2D::new(row_start, row_end, column_start, column_end, shape)
        .map_err(|_| geometry_error("invalid_grid"))
}

fn strictly_increasing_usize(values: &[usize]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn ring_is_simple(ring: &LineString<f64>) -> bool {
    let coordinates = &ring.0;
    if coordinates.len() < 4
        || coordinates.first() != coordinates.last()
        || coordinates.windows(2).any(|pair| pair[0] == pair[1])
    {
        return false;
    }
    let area = Polygon::new(ring.clone(), vec![]).unsigned_area();
    if !area.is_finite() || area <= 0.0 {
        return false;
    }
    let segment_count = coordinates.len() - 1;
    for first in 0..segment_count {
        let first_line = Line::new(coordinates[first], coordinates[first + 1]);
        for second in first + 1..segment_count {
            let second_line = Line::new(coordinates[second], coordinates[second + 1]);
            let shared_endpoint = if second == first + 1 {
                Some(coordinates[second])
            } else if first == 0 && second + 1 == segment_count {
                Some(coordinates[0])
            } else {
                None
            };
            match (shared_endpoint, line_intersection(first_line, second_line)) {
                (
                    Some(shared),
                    Some(LineIntersection::SinglePoint {
                        intersection,
                        is_proper: false,
                    }),
                ) if intersection == shared => {}
                (None, None) => {}
                _ => return false,
            }
        }
    }
    true
}

fn rings_intersect(left: &LineString<f64>, right: &LineString<f64>) -> bool {
    left.lines().any(|left_line| {
        right
            .lines()
            .any(|right_line| left_line.intersects(&right_line))
    })
}

fn strictly_increasing_f64(values: &[f64]) -> bool {
    values.iter().all(|value| value.is_finite()) && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn valid_window(window: Window2D) -> bool {
    window.row_start < window.row_end && window.column_start < window.column_end
}

fn window_area(window: Window2D) -> Result<usize, ContractError> {
    (window.row_end - window.row_start)
        .checked_mul(window.column_end - window.column_start)
        .filter(|area| *area > 0)
        .ok_or_else(|| geometry_error("invalid_grid"))
}

fn geometry_error(reason: &'static str) -> ContractError {
    ContractError::new(
        "NISAR_SUBSET_GEOMETRY_INVALID",
        "NISAR subset hardened geometry rejected the AOI/grid evidence",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded NISAR subset geometry error is safe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn sample_grid() -> SparseGeoGrid {
        let rows = (0..11).collect::<Vec<_>>();
        let cols = (0..11).collect::<Vec<_>>();
        let heights = vec![0.0, 1000.0];
        let mut lons = Vec::new();
        let mut lats = Vec::new();
        for _ in 0..2 {
            let mut layer_lon = Vec::new();
            let mut layer_lat = Vec::new();
            for row in 0..11 {
                for col in 0..11 {
                    layer_lon.push(100.0 + (col as f64) * 0.1);
                    layer_lat.push(30.0 + (row as f64) * 0.1);
                }
            }
            lons.push(layer_lon);
            lats.push(layer_lat);
        }
        SparseGeoGrid::new(rows, cols, heights, lons, lats, [100, 100], [10, 10]).unwrap()
    }

    fn dense_layered_grid(node_count: usize, layer_count: usize) -> SparseGeoGrid {
        let rows = (0..node_count).collect::<Vec<_>>();
        let columns = (0..node_count).collect::<Vec<_>>();
        let heights = (0..layer_count)
            .map(|layer| (layer as f64) * 500.0)
            .collect::<Vec<_>>();
        let mut longitude_layer = Vec::with_capacity(node_count * node_count);
        let mut latitude_layer = Vec::with_capacity(node_count * node_count);
        for row in 0..node_count {
            for column in 0..node_count {
                longitude_layer.push(100.0 + (column as f64) * 0.01);
                latitude_layer.push(30.0 + (row as f64) * 0.01);
            }
        }
        let longitudes = vec![longitude_layer; layer_count];
        let latitudes = vec![latitude_layer; layer_count];
        SparseGeoGrid::new(
            rows,
            columns,
            heights,
            longitudes,
            latitudes,
            [node_count, node_count],
            [1, 1],
        )
        .unwrap()
    }

    #[test]
    fn real_scale_twenty_layer_grid_within_selected_cell_limit_is_admitted() {
        let grid = dense_layered_grid(102, 20);
        let aoi = r#"{"type":"Polygon","coordinates":[[[99.99,29.99],[101.02,29.99],[101.02,31.02],[99.99,31.02],[99.99,29.99]]]}"#;

        let selection = sparse_grid_polygon_cells_v1(aoi, &grid, 0.0, 9_500.0)
            .expect("20 HAE layers must not exhaust a cap below the selected-cell authority");

        assert_eq!(selection.selected_cells.len(), 10_201);
        assert_eq!(selection.height_layer_indices.len(), 20);
        assert_eq!(selection.components.len(), 1);
        assert_eq!(
            MAX_CELL_LAYER_PART_INTERSECTIONS,
            MAX_SELECTED_CELLS * MAX_HEIGHT_LAYERS
        );
    }

    #[test]
    fn selected_cell_limit_remains_fail_closed() {
        // Updated from 130x130 (>16,384) to 514x514 (>262,144) because real
        // NASA ASF NISAR L1 RSLC 556x349 geolocationGrids reach 193,140 cells.
        let grid = dense_layered_grid(514, 1);
        let aoi = r#"{"type":"Polygon","coordinates":[[[99.99,29.99],[105.14,29.99],[105.14,35.14],[99.99,35.14],[99.99,29.99]]]}"#;

        let error = sparse_grid_polygon_cells_v1(aoi, &grid, 0.0, 0.0)
            .expect_err("more than 262,144 selected cells must remain rejected");

        assert_eq!(
            error.debug_context.get("reason").and_then(Value::as_str),
            Some("candidate_budget")
        );
    }

    #[test]
    fn nisarsize_556x349_grid_exceeding_legacy_16384_cell_cap_is_admitted() {
        // Real L1 RSLC frames have ~556x349x20 nodes; a realistic regional AOI
        // intersects >16,384 cells and must not fail with candidate_budget.
        let rows = (0..180).collect::<Vec<_>>();
        let columns = (0..180).collect::<Vec<_>>();
        let heights = vec![-500.0, 4500.0, 9000.0];
        let mut lon = Vec::with_capacity(180 * 180);
        let mut lat = Vec::with_capacity(180 * 180);
        for row in 0..180 {
            for col in 0..180 {
                lon.push(84.0 + (col as f64) * 0.01);
                lat.push(28.0 + (row as f64) * 0.01);
            }
        }
        let grid = SparseGeoGrid::new(
            rows,
            columns,
            heights,
            vec![lon; 3],
            vec![lat; 3],
            [1800, 1800],
            [64, 64],
        )
        .unwrap();
        let aoi = r#"{"type":"Polygon","coordinates":[[[84.10,28.10],[85.60,28.10],[85.60,29.60],[84.10,29.60],[84.10,28.10]]]}"#;
        let selection = sparse_grid_polygon_cells_v1(aoi, &grid, -500.0, 9000.0)
            .expect("22k+ selected cells must succeed cleanly");
        assert!(selection.selected_cells.len() > 16_384);
        assert_eq!(selection.components.len(), 1);
    }

    #[test]
    fn candidate_test_limit_remains_fail_closed() {
        let grid = dense_layered_grid(102, 20);
        let polygons = (0..40)
            .map(|index| {
                let longitude = -170.0 + (index as f64) * 4.0;
                vec![vec![
                    vec![longitude, -80.0],
                    vec![longitude + 0.1, -80.0],
                    vec![longitude + 0.1, -79.9],
                    vec![longitude, -79.9],
                    vec![longitude, -80.0],
                ]]
            })
            .collect::<Vec<_>>();
        let aoi = serde_json::json!({
            "type": "MultiPolygon",
            "coordinates": polygons,
        })
        .to_string();

        let error = sparse_grid_polygon_cells_v1(&aoi, &grid, 0.0, 9_500.0)
            .expect_err("more than eight million cell-layer-part tests must remain rejected");

        assert_eq!(
            error.debug_context.get("reason").and_then(Value::as_str),
            Some("candidate_budget")
        );
    }

    #[test]
    fn subgrid_small_aoi_does_not_false_no_overlap() {
        // AOI is smaller than a single grid cell but must still hit the
        // containing sparse-grid cell(s): the accepted `subgrid_small`
        // contract.
        let aoi = r#"{"type":"Polygon","coordinates":[[[100.40,30.40],[100.41,30.40],[100.41,30.41],[100.40,30.41],[100.40,30.40]]]}"#;
        let selection = sparse_grid_polygon_cells_v1(aoi, &sample_grid(), 0.0, 1000.0)
            .expect("subgrid_small AOI must not false no-overlap");
        assert!(!selection.components.is_empty());
        assert!(!selection.selected_cells.is_empty());
    }

    #[test]
    fn chunk_window_is_conservative_expansion_of_logical_window() {
        let aoi = r#"{"type":"Polygon","coordinates":[[[100.40,30.40],[100.60,30.40],[100.60,30.60],[100.40,30.60],[100.40,30.40]]]}"#;
        let selection = sparse_grid_polygon_cells_v1(aoi, &sample_grid(), 0.0, 1000.0)
            .expect("AOI must intersect");
        for component in &selection.components {
            assert!(component.chunk_window.row_start <= component.logical_window.row_start);
            assert!(component.chunk_window.column_start <= component.logical_window.column_start);
            assert!(component.chunk_window.row_end >= component.logical_window.row_end);
            assert!(component.chunk_window.column_end >= component.logical_window.column_end);
            // chunk alignment
            assert_eq!(component.chunk_window.row_start % 10, 0);
            assert_eq!(component.chunk_window.column_start % 10, 0);
        }
    }

    #[test]
    fn no_overlap_returns_typed_error_not_silent_empty() {
        let aoi = r#"{"type":"Polygon","coordinates":[[[50.0,70.0],[50.1,70.0],[50.1,70.1],[50.0,70.1],[50.0,70.0]]]}"#;
        let result = sparse_grid_polygon_cells_v1(aoi, &sample_grid(), 0.0, 1000.0);
        assert!(result.is_err());
        let error = result.unwrap_err();
        let reason = error
            .debug_context
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("missing");
        assert_eq!(reason, "no_overlap", "{error:?}");
    }

    #[test]
    fn reference_window_admission_rejects_undercoverage_and_excess() {
        // Candidate must conservatively contain the independent reference and
        // stay within the area ceiling (the ISCE3 reference oracle contract).
        let candidate = Window2D::new(0, 3, 0, 4, [100, 100]).unwrap();
        let contained = Window2D::new(0, 3, 0, 3, [100, 100]).unwrap();
        let ratio = validate_reference_window(candidate, contained).expect("contained reference");
        assert!(ratio > 0.0);
        assert!(ratio <= SPARSE_GRID_MAX_CANDIDATE_REFERENCE_CHUNK_AREA_RATIO);
        // Undercovered reference must fail closed.
        let under = Window2D::new(7, 9, 0, 6, [100, 100]).unwrap();
        assert!(validate_reference_window(candidate, under).is_err());
        // Excessive candidate must fail closed.
        let excess = Window2D::new(0, 100, 0, 100, [100, 100]).unwrap();
        assert!(validate_reference_window(excess, contained).is_err());
    }

    #[test]
    fn convex_interior_fast_path_matches_full_polygon_intersection_and_rejects_bowtie() {
        let convex_quad = [
            Coord { x: 100.0, y: 30.0 },
            Coord { x: 100.1, y: 30.0 },
            Coord { x: 100.1, y: 30.1 },
            Coord { x: 100.0, y: 30.1 },
        ];
        assert!(quad_is_convex(&convex_quad));

        let bowtie_quad = [
            Coord { x: 100.0, y: 30.0 },
            Coord { x: 100.1, y: 30.1 },
            Coord { x: 100.1, y: 30.0 },
            Coord { x: 100.0, y: 30.1 },
        ];
        assert!(!quad_is_convex(&bowtie_quad));

        let grid = sample_grid();
        // Polygon with interior hole covering part of the grid
        let aoi_with_hole = r#"{"type":"Polygon","coordinates":[[[100.15,30.15],[100.85,30.15],[100.85,30.85],[100.15,30.85],[100.15,30.15]],[[100.42,30.42],[100.58,30.42],[100.58,30.58],[100.42,30.58],[100.42,30.42]]]}"#;
        let parsed = parse_aoi(aoi_with_hole).expect("valid hole-bearing AOI");
        for row in 0..grid.rows() - 1 {
            for column in 0..grid.columns() - 1 {
                let cell = cell_polygon(&grid, 0, row, column).expect("valid cell");
                let fast = parsed.intersecting_parts(&cell);
                let slow = parsed
                    .polygons
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, poly)| poly.intersects(&cell).then_some(idx))
                    .collect::<Vec<_>>();
                assert_eq!(fast, slow, "cell [{row}, {column}] mismatch");
            }
        }
    }
}
