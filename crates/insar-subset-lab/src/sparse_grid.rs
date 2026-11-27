use std::collections::{BTreeSet, VecDeque};

use geo::{
    line_intersection::{line_intersection, LineIntersection},
    Area, Contains, Coord, Intersects, Line, LineString, Point, Polygon,
};
use geojson::{Geometry as GeoJsonGeometry, Value as GeoJsonValue};
use serde::Serialize;
use thiserror::Error;

use crate::Window2D;

pub const SPARSE_GRID_POLYGON_CELLS_V1: &str = "sparse_grid_polygon_cells_v1";
pub const SPARSE_GRID_CELL_MASK_SEMANTICS: &str = "sparse_cell_intersects_aoi_any_bracketing_hae";
pub const SPARSE_GRID_MAX_CANDIDATE_REFERENCE_CHUNK_AREA_RATIO: f64 = 1.5;
pub const SPARSE_GRID_MAX_AOI_GEOJSON_BYTES: usize = 256 * 1024;

const MAX_GRID_NODES: usize = 1_000_000;
const MAX_HEIGHT_LAYERS: usize = 64;
const MAX_AOI_VERTICES: usize = 4_096;
const MAX_SELECTED_CELLS: usize = 16_384;
const MAX_CELL_LAYER_PART_INTERSECTIONS: usize = 32_768;
const MAX_CELL_LAYER_PART_TESTS: usize = 8_000_000;
const MAX_COMPONENT_MASK_CELLS: usize = 1_048_576;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SparseGridError {
    #[error("AOI must be a bounded EPSG:4326 Polygon or MultiPolygon")]
    InvalidGeometry,
    #[error("sparse geolocation grid is invalid or unsupported")]
    InvalidGrid,
    #[error("requested HAE envelope is outside the bracketing grid layers")]
    HeightEnvelopeUnsupported,
    #[error("AOI does not intersect any candidate sparse-grid cell")]
    NoOverlap,
    #[error("candidate sparse-cell count exceeds the lab budget")]
    CandidateBudgetExceeded,
    #[error("candidate window does not contain the independent reference")]
    ReferenceUndercovered,
    #[error("candidate/reference Chunk area ratio exceeds the admission ceiling")]
    CandidateAreaExceeded,
    #[error("independent reference set does not cover every sparse-grid component")]
    ReferenceSetIncomplete,
    #[error("independent reference set is inconsistent with the sparse-grid selection")]
    ReferenceSetInvalid,
}

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
    ) -> Result<Self, SparseGridError> {
        let rows = node_rows.len();
        let columns = node_columns.len();
        let node_count = rows
            .checked_mul(columns)
            .filter(|count| *count <= MAX_GRID_NODES)
            .ok_or(SparseGridError::InvalidGrid)?;
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
            return Err(SparseGridError::InvalidGrid);
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
                return Err(SparseGridError::InvalidGrid);
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

    pub fn layer_nodes(&self, layer_index: usize) -> Result<Vec<[f64; 2]>, SparseGridError> {
        let longitude = self
            .longitudes
            .get(layer_index)
            .ok_or(SparseGridError::InvalidGrid)?;
        let latitude = self
            .latitudes
            .get(layer_index)
            .ok_or(SparseGridError::InvalidGrid)?;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
) -> Result<SparseGridSelection, SparseGridError> {
    let aoi = parse_aoi(aoi_geojson)?;
    let height_layer_indices =
        bracket_height_layers(&grid.height_layers_m, minimum_hae_m, maximum_hae_m)?;
    let cell_layer_part_tests = (grid.rows() - 1)
        .checked_mul(grid.columns() - 1)
        .and_then(|count| count.checked_mul(height_layer_indices.len()))
        .and_then(|count| count.checked_mul(aoi.part_count()))
        .filter(|count| *count <= MAX_CELL_LAYER_PART_TESTS)
        .ok_or(SparseGridError::CandidateBudgetExceeded)?;
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
                    return Err(SparseGridError::CandidateBudgetExceeded);
                }
                cell_layer_part_intersections = cell_layer_part_intersections
                    .checked_add(intersections.len())
                    .filter(|count| *count <= MAX_CELL_LAYER_PART_INTERSECTIONS)
                    .ok_or(SparseGridError::CandidateBudgetExceeded)?;
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
        return Err(SparseGridError::NoOverlap);
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
) -> Result<f64, SparseGridError> {
    if !valid_window(candidate) || !valid_window(reference) {
        return Err(SparseGridError::InvalidGrid);
    }
    if candidate.row_start > reference.row_start
        || candidate.column_start > reference.column_start
        || candidate.row_end < reference.row_end
        || candidate.column_end < reference.column_end
    {
        return Err(SparseGridError::ReferenceUndercovered);
    }
    let candidate_area = window_area(candidate)?;
    let reference_area = window_area(reference)?;
    let ratio = candidate_area as f64 / reference_area as f64;
    if ratio > SPARSE_GRID_MAX_CANDIDATE_REFERENCE_CHUNK_AREA_RATIO {
        return Err(SparseGridError::CandidateAreaExceeded);
    }
    Ok(ratio)
}

pub fn admit_sparse_grid_selection(
    selection: &SparseGridSelection,
    references: &[SparseGridReferenceWindow],
) -> Result<SparseGridAdmission, SparseGridError> {
    if selection.method != SPARSE_GRID_POLYGON_CELLS_V1
        || selection.chunk_halo != 0
        || selection.mask_semantics != SPARSE_GRID_CELL_MASK_SEMANTICS
    {
        return Err(SparseGridError::ReferenceSetInvalid);
    }
    if selection.components.is_empty() || references.len() != selection.components.len() {
        return Err(SparseGridError::ReferenceSetIncomplete);
    }
    let mut seen_components = BTreeSet::new();
    let mut component_area_ratios = vec![0.0; selection.components.len()];
    for reference in references {
        let component = selection
            .components
            .get(reference.component_index)
            .ok_or(SparseGridError::ReferenceSetInvalid)?;
        if component.geometry_part_index != reference.geometry_part_index
            || !seen_components.insert(reference.component_index)
        {
            return Err(SparseGridError::ReferenceSetInvalid);
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

fn parse_aoi(value: &str) -> Result<AoiGeometry, SparseGridError> {
    if value.len() > SPARSE_GRID_MAX_AOI_GEOJSON_BYTES {
        return Err(SparseGridError::InvalidGeometry);
    }
    let geometry: GeoJsonGeometry =
        serde_json::from_str(value).map_err(|_| SparseGridError::InvalidGeometry)?;
    if geometry.bbox.is_some()
        || geometry
            .foreign_members
            .as_ref()
            .is_some_and(|members| !members.is_empty())
    {
        return Err(SparseGridError::InvalidGeometry);
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
                return Err(SparseGridError::InvalidGeometry);
            }
            Ok(AoiGeometry::new(polygons))
        }
        _ => Err(SparseGridError::InvalidGeometry),
    }
}

fn build_polygon(
    rings: Vec<Vec<Vec<f64>>>,
    vertex_count: &mut usize,
) -> Result<Polygon<f64>, SparseGridError> {
    if rings.is_empty() {
        return Err(SparseGridError::InvalidGeometry);
    }
    let mut rings = rings.into_iter();
    let exterior = build_ring(
        rings.next().ok_or(SparseGridError::InvalidGeometry)?,
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
            return Err(SparseGridError::InvalidGeometry);
        }
    }
    let polygon = Polygon::new(exterior, interiors);
    if !polygon.unsigned_area().is_finite() || polygon.unsigned_area() <= 0.0 {
        return Err(SparseGridError::InvalidGeometry);
    }
    Ok(polygon)
}

fn build_ring(
    positions: Vec<Vec<f64>>,
    vertex_count: &mut usize,
) -> Result<LineString<f64>, SparseGridError> {
    *vertex_count = vertex_count
        .checked_add(positions.len())
        .filter(|count| *count <= MAX_AOI_VERTICES)
        .ok_or(SparseGridError::InvalidGeometry)?;
    if positions.len() < 4 || positions.first() != positions.last() {
        return Err(SparseGridError::InvalidGeometry);
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
                return Err(SparseGridError::InvalidGeometry);
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
        return Err(SparseGridError::InvalidGeometry);
    }
    let ring = LineString::new(coordinates);
    if !ring_is_simple(&ring) {
        return Err(SparseGridError::InvalidGeometry);
    }
    Ok(ring)
}

fn bracket_height_layers(
    layers: &[f64],
    minimum: f64,
    maximum: f64,
) -> Result<Vec<usize>, SparseGridError> {
    if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum {
        return Err(SparseGridError::HeightEnvelopeUnsupported);
    }
    let lower = layers.iter().rposition(|height| *height <= minimum);
    let upper = layers.iter().position(|height| *height >= maximum);
    let (Some(lower), Some(upper)) = (lower, upper) else {
        return Err(SparseGridError::HeightEnvelopeUnsupported);
    };
    if lower > upper {
        return Err(SparseGridError::HeightEnvelopeUnsupported);
    }
    Ok((lower..=upper).collect())
}

fn cell_polygon(
    grid: &SparseGeoGrid,
    layer: usize,
    row: usize,
    column: usize,
) -> Result<Polygon<f64>, SparseGridError> {
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
        return Err(SparseGridError::InvalidGrid);
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
) -> Result<SparseGridComponent, SparseGridError> {
    let minimum_row = cells
        .iter()
        .map(|cell| cell[0])
        .min()
        .ok_or(SparseGridError::InvalidGrid)?;
    let maximum_row = cells
        .iter()
        .map(|cell| cell[0])
        .max()
        .ok_or(SparseGridError::InvalidGrid)?;
    let minimum_column = cells
        .iter()
        .map(|cell| cell[1])
        .min()
        .ok_or(SparseGridError::InvalidGrid)?;
    let maximum_column = cells
        .iter()
        .map(|cell| cell[1])
        .max()
        .ok_or(SparseGridError::InvalidGrid)?;
    let mask_shape = [
        maximum_row - minimum_row + 1,
        maximum_column - minimum_column + 1,
    ];
    let mask_cells = mask_shape[0]
        .checked_mul(mask_shape[1])
        .filter(|count| *count <= MAX_COMPONENT_MASK_CELLS)
        .ok_or(SparseGridError::CandidateBudgetExceeded)?;
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
    .map_err(|_| SparseGridError::InvalidGrid)?;
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
) -> Result<Window2D, SparseGridError> {
    let row_start = window.row_start / chunk[0] * chunk[0];
    let column_start = window.column_start / chunk[1] * chunk[1];
    let row_end = window
        .row_end
        .div_ceil(chunk[0])
        .checked_mul(chunk[0])
        .map(|value| value.min(shape[0]))
        .ok_or(SparseGridError::InvalidGrid)?;
    let column_end = window
        .column_end
        .div_ceil(chunk[1])
        .checked_mul(chunk[1])
        .map(|value| value.min(shape[1]))
        .ok_or(SparseGridError::InvalidGrid)?;
    Window2D::new(row_start, row_end, column_start, column_end, shape)
        .map_err(|_| SparseGridError::InvalidGrid)
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

fn window_area(window: Window2D) -> Result<usize, SparseGridError> {
    (window.row_end - window.row_start)
        .checked_mul(window.column_end - window.column_start)
        .filter(|area| *area > 0)
        .ok_or(SparseGridError::InvalidGrid)
}
