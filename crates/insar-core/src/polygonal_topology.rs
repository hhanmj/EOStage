use geo::{
    coordinate_position::{coord_pos_relative_to_ring, CoordPos, CoordinatePosition},
    line_intersection::{line_intersection, LineIntersection},
    Area, BooleanOps, BoundingRect, Coord, Geometry, Line, LineString, MultiPolygon, Polygon,
    Relate,
};
use geojson::Geometry as GeoJsonGeometry;
use serde_json::{Map, Value};

// These limits protect quadratic topology work for untrusted Provider product
// footprints and direct remote-query projection only. They are deliberately not
// the admission contract for an authoritative administrative/user AOI.
pub const MAX_POLYGONAL_COMPONENTS: usize = 64;
pub const MAX_POLYGONAL_RINGS_PER_POLYGON: usize = 32;
pub const MAX_POLYGONAL_COORDS_PER_RING: usize = 2_048;
pub const MAX_POLYGONAL_TOTAL_COORDS: usize = 4_096;

/// Matches the reviewed native administrative Provider response ceiling. This
/// is a transport/memory safety bound, not a polygon/ring product restriction.
pub const MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES: usize = 8 * 1024 * 1024;
// These ceilings bound only derived longitude-domain topology work. They never
// reject or rewrite admitted exact AOI geometry.
const MAX_PROJECTED_TOPOLOGY_WORK: u64 = 250_000;
const MAX_PROJECTED_SWEEP_CANDIDATES: u64 = 2_000_000;

#[derive(Debug)]
pub(crate) struct ProjectedTopologyBudget {
    remaining_edge_work: u64,
    remaining_sweep_candidates: u64,
}

impl ProjectedTopologyBudget {
    pub(crate) fn new() -> Self {
        Self {
            remaining_edge_work: MAX_PROJECTED_TOPOLOGY_WORK,
            remaining_sweep_candidates: MAX_PROJECTED_SWEEP_CANDIDATES,
        }
    }

    fn consume_edge_work(&mut self, work: Option<u64>) -> bool {
        let Some(work) = work else {
            return false;
        };
        let Some(remaining) = self.remaining_edge_work.checked_sub(work) else {
            return false;
        };
        self.remaining_edge_work = remaining;
        true
    }

    fn consume_sweep_candidate(&mut self) -> bool {
        let Some(remaining) = self.remaining_sweep_candidates.checked_sub(1) else {
            return false;
        };
        self.remaining_sweep_candidates = remaining;
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedAuthoritativePolygonalGeometry {
    pub geometry: MultiPolygon<f64>,
    pub geojson: Map<String, Value>,
    pub bbox: [f64; 4],
}

/// Validates a full, already-authoritative Polygon/MultiPolygon without applying
/// the smaller ASF projection limits. A scalable Boolean union canonicalizes
/// ring order/shared boundaries; a changed area fails closed rather than
/// silently repairing a user or persisted authority.
pub fn validated_authoritative_polygonal_geojson(
    value: &Map<String, Value>,
) -> Option<ValidatedAuthoritativePolygonalGeometry> {
    let serialized = serde_json::to_vec(&Value::Object(value.clone())).ok()?;
    if serialized.len() > MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES {
        return None;
    }
    if let Some(geometry) = validated_polygonal_geojson(value) {
        return Some(ValidatedAuthoritativePolygonalGeometry {
            bbox: geometry_bbox(&geometry)?,
            geometry,
            geojson: value.clone(),
        });
    }
    canonical_authoritative_polygonal_geojson(value, false)
}

/// Canonicalizes a reviewed Provider/import Polygon/MultiPolygon before it
/// becomes AOI authority. This is the native equivalent of Golden's unary-union
/// repair and is intentionally separate from remote ASF request projection.
pub fn canonicalized_authoritative_polygonal_geojson(
    value: &Map<String, Value>,
) -> Option<ValidatedAuthoritativePolygonalGeometry> {
    canonical_authoritative_polygonal_geojson(value, true)
}

/// Parses only raw GeoJSON Polygon/MultiPolygon and proves bounded simple-feature topology.
///
/// Feature wrappers and geometry collections deliberately have no implicit union authority.
pub fn validated_polygonal_geojson(value: &Map<String, Value>) -> Option<MultiPolygon<f64>> {
    let geometry = GeoJsonGeometry::from_json_value(Value::Object(value.clone())).ok()?;
    let geometry = Geometry::try_from(geometry.value).ok()?;
    let geometry = match geometry {
        Geometry::Polygon(polygon) => MultiPolygon(vec![polygon]),
        Geometry::MultiPolygon(polygons) => polygons,
        _ => return None,
    };
    valid_multipolygon(&geometry).then_some(geometry)
}

/// Returns the exact geographic envelope of a validated raw Polygon/MultiPolygon.
///
/// This is the shared fixed-scalar projection used by persistence boundaries; it
/// deliberately does not accept Feature wrappers or manufacture a bbox for an
/// invalid topology.
pub fn validated_polygonal_bbox(value: &Map<String, Value>) -> Option<[f64; 4]> {
    let geometry = validated_polygonal_geojson(value)?;
    let envelope = geometry.bounding_rect()?;
    let bbox = [
        envelope.min().x,
        envelope.min().y,
        envelope.max().x,
        envelope.max().y,
    ];
    (bbox.iter().all(|coordinate| coordinate.is_finite())
        && bbox[0] >= -180.0
        && bbox[2] <= 180.0
        && bbox[1] >= -90.0
        && bbox[3] <= 90.0
        && bbox[0] < bbox[2]
        && bbox[1] < bbox[3])
        .then_some(bbox)
}

pub fn validated_authoritative_polygonal_bbox(value: &Map<String, Value>) -> Option<[f64; 4]> {
    validated_authoritative_polygonal_geojson(value).map(|value| value.bbox)
}

fn canonical_authoritative_polygonal_geojson(
    value: &Map<String, Value>,
    repair: bool,
) -> Option<ValidatedAuthoritativePolygonalGeometry> {
    let serialized = serde_json::to_vec(&Value::Object(value.clone())).ok()?;
    if serialized.len() > MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES {
        return None;
    }
    let prefer_multi = value.get("type").and_then(Value::as_str) == Some("MultiPolygon");
    let geometry = parse_polygonal(value)?;
    if !structurally_valid_authoritative_geometry(&geometry) {
        return None;
    }
    let source_area = geometry.unsigned_area();
    if !source_area.is_finite() || source_area <= 0.0 {
        return None;
    }
    let canonical = catch_boolean_union(&geometry)?;
    if !structurally_valid_authoritative_geometry(&canonical) {
        return None;
    }
    let canonical_area = canonical.unsigned_area();
    if !canonical_area.is_finite() || canonical_area <= 0.0 {
        return None;
    }
    if !repair {
        let scale = source_area.abs().max(canonical_area.abs()).max(1.0);
        if (source_area - canonical_area).abs() > scale * 1.0e-9 {
            return None;
        }
    }
    let bbox = geometry_bbox(&canonical)?;
    let geojson = polygonal_geojson_map(&canonical, prefer_multi)?;
    let canonical_bytes = serde_json::to_vec(&Value::Object(geojson.clone())).ok()?;
    if canonical_bytes.len() > MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES {
        return None;
    }
    Some(ValidatedAuthoritativePolygonalGeometry {
        geometry: canonical,
        geojson,
        bbox,
    })
}

fn parse_polygonal(value: &Map<String, Value>) -> Option<MultiPolygon<f64>> {
    let geometry = GeoJsonGeometry::from_json_value(Value::Object(value.clone())).ok()?;
    let geometry = Geometry::try_from(geometry.value).ok()?;
    match geometry {
        Geometry::Polygon(polygon) => Some(MultiPolygon(vec![polygon])),
        Geometry::MultiPolygon(polygons) => Some(polygons),
        _ => None,
    }
}

fn structurally_valid_authoritative_geometry(geometry: &MultiPolygon<f64>) -> bool {
    !geometry.0.is_empty()
        && geometry.0.iter().all(|polygon| {
            std::iter::once(polygon.exterior())
                .chain(polygon.interiors().iter())
                .all(|ring| {
                    ring.0.len() >= 4
                        && ring.0.first() == ring.0.last()
                        && ring.0.windows(2).all(|edge| edge[0] != edge[1])
                        && ring.0.iter().all(|coord| {
                            coord.x.is_finite()
                                && coord.y.is_finite()
                                && (-180.0..=180.0).contains(&coord.x)
                                && (-90.0..=90.0).contains(&coord.y)
                        })
                })
        })
}

fn catch_boolean_union(geometry: &MultiPolygon<f64>) -> Option<MultiPolygon<f64>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MultiPolygon(Vec::new()).union(geometry)
    }))
    .ok()
}

/// Serializes a polygonal geometry without changing its coordinates.
///
/// Render authorities use this after native longitude projection has produced
/// copies outside the canonical [-180, 180] interval.
pub fn polygonal_geojson_map(
    geometry: &MultiPolygon<f64>,
    prefer_multi: bool,
) -> Option<Map<String, Value>> {
    let geometry = if geometry.0.len() == 1 && !prefer_multi {
        GeoJsonGeometry::new(geojson::Value::from(&geometry.0[0]))
    } else {
        GeoJsonGeometry::new(geojson::Value::from(geometry))
    };
    serde_json::to_value(geometry).ok()?.as_object().cloned()
}

pub(crate) fn geometry_bbox(geometry: &MultiPolygon<f64>) -> Option<[f64; 4]> {
    let envelope = geometry.bounding_rect()?;
    let bbox = [
        envelope.min().x,
        envelope.min().y,
        envelope.max().x,
        envelope.max().y,
    ];
    (bbox.iter().all(|coordinate| coordinate.is_finite())
        && bbox[0] >= -180.0
        && bbox[2] <= 180.0
        && bbox[1] >= -90.0
        && bbox[3] <= 90.0
        && bbox[0] < bbox[2]
        && bbox[1] < bbox[3])
        .then_some(bbox)
}

fn valid_ring(ring: &LineString<f64>) -> bool {
    ring.0.len() >= 4
        && ring.0.first() == ring.0.last()
        && ring
            .0
            .iter()
            .all(|coord| coord.x.is_finite() && coord.y.is_finite())
        && ring.0.windows(2).all(|edge| edge[0] != edge[1])
        && ring_is_simple(ring)
}

fn valid_polygon(polygon: &Polygon<f64>) -> bool {
    valid_ring(polygon.exterior())
        && polygon.interiors().iter().all(valid_ring)
        && polygon
            .interiors()
            .iter()
            .all(|hole| ring_is_strictly_inside(hole, polygon.exterior()))
        && polygon.interiors().iter().enumerate().all(|(index, hole)| {
            polygon.interiors()[index + 1..]
                .iter()
                .all(|other| rings_are_disjoint(hole, other))
        })
        && polygon.unsigned_area().is_finite()
        && polygon.unsigned_area() > 0.0
}

pub(crate) fn projected_polygon_has_valid_topology(
    polygon: &Polygon<f64>,
    budget: &mut ProjectedTopologyBudget,
) -> bool {
    consume_projected_polygon_validation_work(polygon, budget) && valid_polygon(polygon)
}

fn ring_edge_count(ring: &LineString<f64>) -> Option<u64> {
    u64::try_from(ring.0.len().checked_sub(1)?).ok()
}

fn ring_validation_work(ring: &LineString<f64>) -> Option<u64> {
    let edges = ring_edge_count(ring)?;
    let non_adjacent_pairs = if edges < 3 {
        0
    } else {
        edges.checked_mul(edges.checked_sub(3)?)?.checked_div(2)?
    };
    edges.checked_add(non_adjacent_pairs)
}

fn polygon_total_edges(polygon: &Polygon<f64>) -> Option<u64> {
    std::iter::once(polygon.exterior())
        .chain(polygon.interiors().iter())
        .try_fold(0u64, |total, ring| {
            total.checked_add(ring_edge_count(ring)?)
        })
}

fn consume_projected_polygon_validation_work(
    polygon: &Polygon<f64>,
    budget: &mut ProjectedTopologyBudget,
) -> bool {
    for ring in std::iter::once(polygon.exterior()).chain(polygon.interiors().iter()) {
        if !budget.consume_edge_work(ring_validation_work(ring)) {
            return false;
        }
    }

    let Some(exterior_edges) = ring_edge_count(polygon.exterior()) else {
        return false;
    };
    for hole in polygon.interiors() {
        let relation_work = ring_edge_count(hole)
            .and_then(|hole_edges| hole_edges.checked_mul(exterior_edges))
            .and_then(|work| work.checked_mul(2));
        if !budget.consume_edge_work(relation_work) {
            return false;
        }
    }
    for (index, hole) in polygon.interiors().iter().enumerate() {
        let Some(hole_edges) = ring_edge_count(hole) else {
            return false;
        };
        for other in &polygon.interiors()[index + 1..] {
            let relation_work = ring_edge_count(other).and_then(|other_edges| {
                hole_edges
                    .checked_mul(other_edges)?
                    .checked_add(hole_edges)?
                    .checked_add(other_edges)
            });
            if !budget.consume_edge_work(relation_work) {
                return false;
            }
        }
    }
    budget.consume_edge_work(polygon_total_edges(polygon).and_then(|work| work.checked_mul(2)))
}

fn valid_multipolygon(geometry: &MultiPolygon<f64>) -> bool {
    geometry_within_bounds(geometry)
        && !geometry.0.is_empty()
        && geometry.0.iter().all(valid_polygon)
        && geometry.0.iter().enumerate().all(|(index, polygon)| {
            geometry.0[index + 1..]
                .iter()
                .all(|other| polygons_are_disjoint(polygon, other))
        })
}

fn geometry_within_bounds(geometry: &MultiPolygon<f64>) -> bool {
    if geometry.0.len() > MAX_POLYGONAL_COMPONENTS {
        return false;
    }

    let mut total_coordinates = 0usize;
    for polygon in &geometry.0 {
        if polygon.interiors().len() + 1 > MAX_POLYGONAL_RINGS_PER_POLYGON {
            return false;
        }
        for ring in std::iter::once(polygon.exterior()).chain(polygon.interiors().iter()) {
            if ring.0.len() > MAX_POLYGONAL_COORDS_PER_RING {
                return false;
            }
            let Some(next_total) = total_coordinates.checked_add(ring.0.len()) else {
                return false;
            };
            total_coordinates = next_total;
            if total_coordinates > MAX_POLYGONAL_TOTAL_COORDS {
                return false;
            }
        }
    }
    true
}

fn ring_is_simple(ring: &LineString<f64>) -> bool {
    let edge_count = ring.0.len() - 1;
    (0..edge_count).all(|first| {
        ((first + 1)..edge_count).all(|second| {
            let adjacent = second == first + 1 || (first == 0 && second + 1 == edge_count);
            adjacent
                || !segments_have_disallowed_intersection(
                    ring.0[first],
                    ring.0[first + 1],
                    ring.0[second],
                    ring.0[second + 1],
                    false,
                )
        })
    })
}

fn ring_is_strictly_inside(inner: &LineString<f64>, outer: &LineString<f64>) -> bool {
    !rings_have_disallowed_intersection(inner, outer, false)
        && inner
            .0
            .iter()
            .take(inner.0.len() - 1)
            .all(|point| coord_pos_relative_to_ring(*point, outer) == CoordPos::Inside)
}

fn rings_are_disjoint(first: &LineString<f64>, second: &LineString<f64>) -> bool {
    !rings_have_disallowed_intersection(first, second, false)
        && coord_pos_relative_to_ring(first.0[0], second) == CoordPos::Outside
        && coord_pos_relative_to_ring(second.0[0], first) == CoordPos::Outside
}

fn polygons_are_disjoint(first: &Polygon<f64>, second: &Polygon<f64>) -> bool {
    !polygon_exterior_has_disallowed_intersection(first, second)
        && !polygon_exterior_has_disallowed_intersection(second, first)
        && first
            .exterior()
            .0
            .iter()
            .take(first.exterior().0.len() - 1)
            .all(|point| second.coordinate_position(point) != CoordPos::Inside)
        && second
            .exterior()
            .0
            .iter()
            .take(second.exterior().0.len() - 1)
            .all(|point| first.coordinate_position(point) != CoordPos::Inside)
}

#[derive(Clone, Copy, Debug)]
struct ProjectedComponentBounds {
    index: usize,
    west: f64,
    east: f64,
    south: f64,
    north: f64,
    exterior_edges: u64,
    total_edges: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectedSweepAxis {
    Longitude,
    Latitude,
}

impl ProjectedSweepAxis {
    fn other(self) -> Self {
        match self {
            Self::Longitude => Self::Latitude,
            Self::Latitude => Self::Longitude,
        }
    }
}

impl ProjectedComponentBounds {
    fn interval(self, axis: ProjectedSweepAxis) -> (f64, f64) {
        match axis {
            ProjectedSweepAxis::Longitude => (self.west, self.east),
            ProjectedSweepAxis::Latitude => (self.south, self.north),
        }
    }
}

fn projected_component_bounds(polygons: &[Polygon<f64>]) -> Option<Vec<ProjectedComponentBounds>> {
    let mut bounds = Vec::with_capacity(polygons.len());
    for (index, polygon) in polygons.iter().enumerate() {
        let envelope = polygon.bounding_rect()?;
        let values = [
            envelope.min().x,
            envelope.max().x,
            envelope.min().y,
            envelope.max().y,
        ];
        if !values.into_iter().all(f64::is_finite) {
            return None;
        }
        bounds.push(ProjectedComponentBounds {
            index,
            west: values[0],
            east: values[1],
            south: values[2],
            north: values[3],
            exterior_edges: ring_edge_count(polygon.exterior())?,
            total_edges: polygon_total_edges(polygon)?,
        });
    }
    Some(bounds)
}

fn projected_polygon_pair_work(
    first: ProjectedComponentBounds,
    second: ProjectedComponentBounds,
) -> Option<u64> {
    first
        .exterior_edges
        .checked_mul(second.total_edges)?
        .checked_add(second.exterior_edges.checked_mul(first.total_edges)?)?
        .checked_mul(2)
}

fn projected_relate_pair_work(
    first: ProjectedComponentBounds,
    second: ProjectedComponentBounds,
) -> Option<u64> {
    let total_edges = first.total_edges.checked_add(second.total_edges)?;
    // `Relate` nodes every ring in both inputs before computing their mutual
    // topology. The square expands to a² + b² + 2ab, covering both self-node
    // sets and both cross interactions. Add fixed per-edge setup and apply a
    // safety factor before any untrusted robust-topology work begins.
    total_edges
        .checked_mul(total_edges)?
        .checked_add(total_edges.checked_mul(4)?)?
        .checked_mul(2)
}

fn projected_sweep_candidate_count(
    bounds: &[ProjectedComponentBounds],
    axis: ProjectedSweepAxis,
) -> Option<u64> {
    let mut starts = bounds
        .iter()
        .map(|bound| bound.interval(axis).0)
        .collect::<Vec<_>>();
    let mut ends = bounds
        .iter()
        .map(|bound| bound.interval(axis).1)
        .collect::<Vec<_>>();
    starts.sort_by(f64::total_cmp);
    ends.sort_by(f64::total_cmp);

    let mut ended = 0usize;
    let mut candidates = 0u64;
    for (position, start) in starts.into_iter().enumerate() {
        while ended < ends.len() && ends[ended] < start {
            ended += 1;
        }
        let active = position.checked_sub(ended)?;
        candidates = candidates.checked_add(u64::try_from(active).ok()?)?;
    }
    Some(candidates)
}

fn preferred_projected_sweep_axis(
    bounds: &[ProjectedComponentBounds],
) -> Option<(ProjectedSweepAxis, u64)> {
    let longitude = projected_sweep_candidate_count(bounds, ProjectedSweepAxis::Longitude)?;
    let latitude = projected_sweep_candidate_count(bounds, ProjectedSweepAxis::Latitude)?;
    if latitude < longitude {
        Some((ProjectedSweepAxis::Latitude, latitude))
    } else {
        Some((ProjectedSweepAxis::Longitude, longitude))
    }
}

/// Checks the pairwise relationships that can change when already-validated
/// polygon components are translated into different periodic world copies.
pub(crate) fn projected_components_have_valid_topology(
    geometry: &MultiPolygon<f64>,
    budget: &mut ProjectedTopologyBudget,
) -> bool {
    projected_polygon_pairs_satisfy(
        &geometry.0,
        budget,
        projected_polygon_pair_work,
        polygons_are_disjoint,
    )
}

/// Periodic copies are scored independently, so coincident boundaries are
/// harmless but overlapping interiors would double-count coverage.
pub(crate) fn projected_copies_have_non_overlapping_interiors(
    polygons: &[Polygon<f64>],
    budget: &mut ProjectedTopologyBudget,
) -> bool {
    projected_polygon_pairs_satisfy(
        polygons,
        budget,
        projected_relate_pair_work,
        polygons_have_disjoint_interiors,
    )
}

fn projected_polygon_pairs_satisfy(
    polygons: &[Polygon<f64>],
    budget: &mut ProjectedTopologyBudget,
    pair_work: impl Fn(ProjectedComponentBounds, ProjectedComponentBounds) -> Option<u64>,
    relation_is_valid: impl Fn(&Polygon<f64>, &Polygon<f64>) -> bool,
) -> bool {
    let Some(mut bounds) = projected_component_bounds(polygons) else {
        return false;
    };
    let Some((axis, _)) = preferred_projected_sweep_axis(&bounds) else {
        return false;
    };
    let other_axis = axis.other();
    bounds.sort_by(|left, right| {
        let (left_start, left_end) = left.interval(axis);
        let (right_start, right_end) = right.interval(axis);
        let (left_other_start, left_other_end) = left.interval(other_axis);
        let (right_other_start, right_other_end) = right.interval(other_axis);
        left_start
            .total_cmp(&right_start)
            .then_with(|| left_end.total_cmp(&right_end))
            .then_with(|| left_other_start.total_cmp(&right_other_start))
            .then_with(|| left_other_end.total_cmp(&right_other_end))
            .then_with(|| left.index.cmp(&right.index))
    });
    for (position, current) in bounds.iter().copied().enumerate() {
        let (_, current_end) = current.interval(axis);
        let (current_other_start, current_other_end) = current.interval(other_axis);
        for other in bounds[position + 1..].iter().copied() {
            let (other_start, _) = other.interval(axis);
            if other_start > current_end {
                break;
            }
            if !budget.consume_sweep_candidate() {
                return false;
            }
            let (other_other_start, other_other_end) = other.interval(other_axis);
            if other_other_start > current_other_end || other_other_end < current_other_start {
                continue;
            }
            if !budget.consume_edge_work(pair_work(current, other))
                || !relation_is_valid(&polygons[current.index], &polygons[other.index])
            {
                return false;
            }
        }
    }
    true
}

fn polygons_have_disjoint_interiors(first: &Polygon<f64>, second: &Polygon<f64>) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let relation = first.relate(second);
        relation.is_disjoint() || relation.is_touches()
    }))
    .unwrap_or(false)
}

fn polygon_exterior_has_disallowed_intersection(
    source: &Polygon<f64>,
    target: &Polygon<f64>,
) -> bool {
    std::iter::once(target.exterior())
        .chain(target.interiors().iter())
        .any(|target_ring| rings_have_disallowed_intersection(source.exterior(), target_ring, true))
}

fn rings_have_disallowed_intersection(
    first: &LineString<f64>,
    second: &LineString<f64>,
    allow_non_proper_point_touch: bool,
) -> bool {
    first.0.windows(2).any(|first_edge| {
        second.0.windows(2).any(|second_edge| {
            segments_have_disallowed_intersection(
                first_edge[0],
                first_edge[1],
                second_edge[0],
                second_edge[1],
                allow_non_proper_point_touch,
            )
        })
    })
}

fn segments_have_disallowed_intersection(
    first_start: Coord<f64>,
    first_end: Coord<f64>,
    second_start: Coord<f64>,
    second_end: Coord<f64>,
    allow_non_proper_point_touch: bool,
) -> bool {
    match line_intersection(
        Line::new(first_start, first_end),
        Line::new(second_start, second_end),
    ) {
        None => false,
        Some(LineIntersection::Collinear { .. }) => true,
        Some(LineIntersection::SinglePoint { is_proper, .. }) => {
            is_proper || !allow_non_proper_point_touch
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_topology_sweep_chooses_sparse_axis_for_vertical_component_stacks() {
        let geometry = MultiPolygon(
            (0..4_096)
                .map(|index| {
                    let south = -80.0 + index as f64 * 0.03;
                    Polygon::new(
                        LineString::new(vec![
                            Coord { x: 0.0, y: south },
                            Coord { x: 1.0, y: south },
                            Coord {
                                x: 1.0,
                                y: south + 0.01,
                            },
                            Coord {
                                x: 0.0,
                                y: south + 0.01,
                            },
                            Coord { x: 0.0, y: south },
                        ]),
                        Vec::new(),
                    )
                })
                .collect(),
        );
        let bounds = projected_component_bounds(&geometry.0).unwrap();
        let (axis, _) = preferred_projected_sweep_axis(&bounds).unwrap();

        assert_eq!(axis, ProjectedSweepAxis::Latitude);
        assert_eq!(projected_sweep_candidate_count(&bounds, axis), Some(0));
    }
}
