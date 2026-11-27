use std::cmp::Ordering;

use geo::{Area, BooleanOps, BoundingRect, Coord, LineString, MultiPolygon, Polygon};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    polygonal_topology::{
        geometry_bbox, projected_components_have_valid_topology,
        projected_copies_have_non_overlapping_interiors, projected_polygon_has_valid_topology,
        ProjectedTopologyBudget,
    },
    BBox,
};

/// Native render copies can extend one component width beyond the continuous
/// AOI reference domain. The source is geographic [-180, 180], every unwrapped
/// component is at most one world wide, and the producer only admits adjacent
/// component/reference copies. Keeping this shared bound at ±900 makes the
/// DTO contract explicit while leaving room for the exact endpoint aliases at
/// ±540 and one additional component-width margin.
pub const MAX_PROJECTED_RENDER_LONGITUDE: f64 = 900.0;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WrappedAoiEnvelope {
    pub south: f64,
    pub north: f64,
    pub crosses_antimeridian: bool,
    pub display_west: f64,
    pub display_east: f64,
    pub request_windows: Vec<BBox>,
}

/// Independently valid periodic polygon copies whose interiors are proven not
/// to overlap. Keeping the copies separate permits shared seam boundaries
/// without constructing an invalid `MultiPolygon` or double-counting area.
#[derive(Clone, Debug)]
pub struct AlignedPolygonalCopies {
    polygons: Vec<Polygon<f64>>,
}

impl AlignedPolygonalCopies {
    pub fn as_slice(&self) -> &[Polygon<f64>] {
        &self.polygons
    }

    /// Converts independently aligned copies into a render-only MultiPolygon.
    /// The source geometry remains untouched.
    pub fn geojson(&self) -> Option<Map<String, Value>> {
        crate::polygonal_topology::polygonal_geojson_map(&MultiPolygon(self.polygons.clone()), true)
    }

    /// Returns the display extent without applying canonical geographic bounds.
    pub fn display_bbox(&self) -> Option<[f64; 4]> {
        let rect = MultiPolygon(self.polygons.clone()).bounding_rect()?;
        let bbox = [rect.min().x, rect.min().y, rect.max().x, rect.max().y];
        (bbox.iter().all(|value| value.is_finite())
            && bbox[0] < bbox[2]
            && bbox[1] < bbox[3]
            && bbox[1] >= -90.0
            && bbox[3] <= 90.0)
            .then_some(bbox)
    }

    pub fn longitudes_within_render_contract(&self) -> bool {
        polygons_longitudes_within_render_contract(&self.polygons)
    }

    /// Sums each independent copy's intersection with the reference geometry.
    pub fn overlap_area(&self, reference: &MultiPolygon<f64>) -> Option<f64> {
        self.polygons.iter().try_fold(0.0_f64, |total, polygon| {
            let intersection = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                polygon.intersection(reference)
            }))
            .ok()?;
            let area = intersection.unsigned_area();
            if !area.is_finite() || area < 0.0 {
                return None;
            }
            let total = total + area;
            total.is_finite().then_some(total)
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct LongitudeInterval {
    west: f64,
    east: f64,
}

#[derive(Clone, Copy, Debug)]
struct IndexedLongitudeInterval {
    west: WorldIndexedLongitude,
    east: WorldIndexedLongitude,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IndexedSpan {
    Zero,
    LessThanWorld,
    FullWorld,
    MoreThanWorld,
}

#[derive(Clone, Debug)]
struct ExactExpansion {
    components: Vec<f64>,
}

impl ExactExpansion {
    fn from_terms(terms: impl IntoIterator<Item = f64>) -> Option<Self> {
        let mut expansion = Self {
            components: Vec::new(),
        };
        for term in terms {
            if !term.is_finite() {
                return None;
            }
            expansion.add_term(term)?;
        }
        Some(expansion)
    }

    fn add_term(&mut self, term: f64) -> Option<()> {
        let mut sum = term;
        let mut components = Vec::with_capacity(self.components.len() + 1);
        for &component in &self.components {
            let (next_sum, error) = two_sum(sum, component)?;
            if error != 0.0 {
                components.push(error);
            }
            sum = next_sum;
        }
        if sum != 0.0 || components.is_empty() {
            components.push(sum);
        }
        self.components = components;
        Some(())
    }

    fn sub(&self, other: &Self) -> Option<Self> {
        let mut result = self.clone();
        for &component in &other.components {
            result.add_term(-component)?;
        }
        Some(result)
    }

    fn exact_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.sub(other)?.sign().cmp(&0))
    }

    fn sign(&self) -> i8 {
        self.components
            .iter()
            .rev()
            .find(|component| **component != 0.0)
            .map_or(0, |component| if *component < 0.0 { -1 } else { 1 })
    }
}

fn two_sum(left: f64, right: f64) -> Option<(f64, f64)> {
    let sum = left + right;
    if !sum.is_finite() {
        return None;
    }
    let right_virtual = sum - left;
    let left_virtual = sum - right_virtual;
    let right_roundoff = right - right_virtual;
    let left_roundoff = left - left_virtual;
    Some((sum, left_roundoff + right_roundoff))
}

#[derive(Clone, Copy, Debug)]
struct CircularSegment {
    start: f64,
    end: f64,
}

#[derive(Clone, Copy, Debug)]
enum CircularCover {
    Full,
    Arc(IndexedLongitudeInterval),
}

#[derive(Clone, Copy, Debug)]
struct WorldIndexedLongitude {
    source: f64,
    worlds: i32,
}

impl WorldIndexedLongitude {
    fn canonical(source: f64, worlds: i32) -> Option<Self> {
        let source = if source == 0.0 { 0.0 } else { source };
        if source == 180.0 {
            Some(Self {
                source: -180.0,
                worlds: worlds.checked_add(1)?,
            })
        } else {
            Some(Self { source, worlds })
        }
    }

    fn shifted(self, additional_worlds: i32) -> Option<Self> {
        Some(Self {
            source: self.source,
            worlds: self.worlds.checked_add(additional_worlds)?,
        })
    }

    fn indexed_cmp(self, other: Self) -> Ordering {
        self.worlds
            .cmp(&other.worlds)
            .then(self.source.total_cmp(&other.source))
    }
}

fn exact_difference_cmp(minuend: f64, subtrahend: f64, value: f64) -> Option<Ordering> {
    if !minuend.is_finite() || !subtrahend.is_finite() || !value.is_finite() {
        return None;
    }
    let approximate = minuend - subtrahend;
    if approximate.is_infinite() {
        return approximate.partial_cmp(&value);
    }
    Some(
        ExactExpansion::from_terms([minuend, -subtrahend, -value])?
            .sign()
            .cmp(&0),
    )
}

fn exact_indexed_value(longitude: WorldIndexedLongitude) -> Option<ExactExpansion> {
    ExactExpansion::from_terms([longitude.source, f64::from(longitude.worlds) * 360.0])
}

#[derive(Clone, Copy, Debug)]
struct WorldIndexedCoord {
    source: Coord<f64>,
    worlds: i32,
}

type WorldIndexedRing = Vec<WorldIndexedCoord>;

#[derive(Clone, Debug)]
struct UnwrappedPolygon {
    exterior: WorldIndexedRing,
    interiors: Vec<WorldIndexedRing>,
    west: WorldIndexedLongitude,
    east: WorldIndexedLongitude,
}

pub fn wrapped_aoi_envelope(geometry: &MultiPolygon<f64>) -> Option<WrappedAoiEnvelope> {
    let (south, north) = latitude_bounds(geometry)?;
    let indexed_interval = minimal_continuous_longitude_interval(geometry)?;
    match indexed_span(&indexed_interval)? {
        IndexedSpan::Zero | IndexedSpan::MoreThanWorld => return None,
        IndexedSpan::FullWorld => return Some(full_world_envelope(south, north)),
        IndexedSpan::LessThanWorld => {}
    }
    let interval = project_interval(&indexed_interval)?;
    if !interval.west.is_finite() || !interval.east.is_finite() || interval.east <= interval.west {
        return None;
    }
    let request_windows = split_at_antimeridian(geometry, &interval, south, north)?;
    if request_windows.is_empty() || request_windows.len() > 2 {
        return None;
    }
    Some(WrappedAoiEnvelope {
        south,
        north,
        crosses_antimeridian: request_windows.len() == 2,
        display_west: interval.west,
        display_east: interval.east,
        request_windows,
    })
}

/// Projects each already-validated component into one continuous display
/// domain. Returns `None` rather than mutating or unioning components when the
/// periodic projection makes their topology ambiguous.
pub fn align_polygonal_to_longitude_domain(
    geometry: &MultiPolygon<f64>,
    display_west: f64,
    display_east: f64,
) -> Option<MultiPolygon<f64>> {
    if !display_west.is_finite()
        || !display_east.is_finite()
        || exact_difference_cmp(display_east, display_west, 0.0)? != Ordering::Greater
        || exact_difference_cmp(display_east, display_west, 360.0)? == Ordering::Greater
    {
        return None;
    }
    latitude_bounds(geometry)?;
    let polygons = unwrap_polygons(geometry)?;
    let mut topology_budget = ProjectedTopologyBudget::new();
    let mut aligned = Vec::with_capacity(polygons.len());

    for component in polygons {
        let shift = component_shift_to_projected_domain(&component, display_west, display_east)?;
        let validate_component = !component_has_one_world_index(&component);
        let polygon = project_polygon(&component, shift)?;
        if validate_component
            && !projected_polygon_has_valid_topology(&polygon, &mut topology_budget)
        {
            return None;
        }
        aligned.push(polygon);
    }

    let aligned = MultiPolygon(aligned);
    projected_components_have_valid_topology(&aligned, &mut topology_budget).then_some(aligned)
}

/// Aligns one validated polygonal geometry into the requested display domain
/// and returns its render-only GeoJSON plus exact display bbox. Unlike the
/// adjacent-copy helper below, this produces one continuous AOI geometry and
/// never adds a periodic alias outside the domain.
pub fn align_polygonal_to_longitude_domain_render(
    geometry: &MultiPolygon<f64>,
    display_west: f64,
    display_east: f64,
) -> Option<(Map<String, Value>, [f64; 4])> {
    let aligned = align_polygonal_to_longitude_domain(geometry, display_west, display_east)?;
    if !polygons_longitudes_within_render_contract(&aligned.0) {
        return None;
    }
    let geojson = crate::polygonal_topology::polygonal_geojson_map(&aligned, true)?;
    let envelope = aligned.bounding_rect()?;
    let geometry_bbox = [
        envelope.min().x,
        envelope.min().y,
        envelope.max().x,
        envelope.max().y,
    ];
    if !geometry_bbox
        .iter()
        .all(|coordinate| coordinate.is_finite())
        || geometry_bbox[0] >= geometry_bbox[2]
        || geometry_bbox[1] >= geometry_bbox[3]
        || geometry_bbox[1] < -90.0
        || geometry_bbox[3] > 90.0
    {
        return None;
    }
    // The authority's bbox is the complete native display domain, even when
    // a valid global geometry does not touch one of the domain edges.
    Some((
        geojson,
        [
            display_west,
            geometry_bbox[1],
            display_east,
            geometry_bbox[3],
        ],
    ))
}

/// Projects raw coordinates once into every adjacent periodic copy with a
/// positive-width overlap against the reference domain. The returned
/// result contains at most two copies of each source component;
/// boundary-only copies are excluded so area-based consumers cannot count a
/// seam twice. Copies remain independent so shared boundaries are valid, while
/// robust pair checks reject interior overlap before coverage can be summed.
pub fn align_polygonal_to_longitude_reference(
    geometry: &MultiPolygon<f64>,
    reference_west: f64,
    reference_east: f64,
) -> Option<AlignedPolygonalCopies> {
    if !reference_west.is_finite()
        || !reference_east.is_finite()
        || exact_difference_cmp(reference_east, reference_west, 0.0)? != Ordering::Greater
        || exact_difference_cmp(reference_east, reference_west, 360.0)? == Ordering::Greater
        || !(-180.0..180.0).contains(&reference_west)
    {
        return None;
    }
    let polygons = unwrap_polygons(geometry)?;
    // Retain aggregate circular-union validation, but choose copies per
    // component: a valid full-world union can contain a component crossing the
    // canonical seam that cannot fit wholly inside either aggregate copy.
    minimal_continuous_longitude_interval(geometry)?;
    let mut topology_budget = ProjectedTopologyBudget::new();
    let mut aligned = Vec::with_capacity(polygons.len().saturating_mul(2));
    for component in &polygons {
        let validate_component = !component_has_one_world_index(component);
        let interval = indexed_component_interval_at(component, 0)?;
        let component_shift = interval.west.worlds.checked_neg()?;
        let interval = shift_indexed_interval(&interval, component_shift)?;
        if interval.west.worlds != 0 {
            return None;
        }
        for reference_shift in
            overlapping_reference_world_shifts(&interval, reference_west, reference_east)?
        {
            let polygon =
                project_polygon(component, component_shift.checked_add(reference_shift)?)?;
            if validate_component
                && !projected_polygon_has_valid_topology(&polygon, &mut topology_budget)
            {
                return None;
            }
            aligned.push(polygon);
        }
    }
    projected_copies_have_non_overlapping_interiors(&aligned, &mut topology_budget)
        .then_some(AlignedPolygonalCopies { polygons: aligned })
}

/// Projects each periodic component copy whose closed longitude interval
/// touches the reference domain. Copies stay separate so boundary-only aliases
/// are usable for intersection filtering without forming an invalid
/// `MultiPolygon` or contributing duplicate area.
pub(crate) fn align_polygonal_to_longitude_reference_for_intersection(
    geometry: &MultiPolygon<f64>,
    reference_west: f64,
    reference_east: f64,
) -> Option<Vec<Polygon<f64>>> {
    if !reference_west.is_finite()
        || !reference_east.is_finite()
        || exact_difference_cmp(reference_east, reference_west, 0.0)? != Ordering::Greater
        || exact_difference_cmp(reference_east, reference_west, 360.0)? == Ordering::Greater
        || !(-180.0..180.0).contains(&reference_west)
    {
        return None;
    }
    let polygons = unwrap_polygons(geometry)?;
    minimal_continuous_longitude_interval(geometry)?;
    let mut topology_budget = ProjectedTopologyBudget::new();
    let mut aligned = Vec::with_capacity(polygons.len().saturating_mul(3));
    for component in &polygons {
        let validate_component = !component_has_one_world_index(component);
        let interval = indexed_component_interval_at(component, 0)?;
        let component_shift = interval.west.worlds.checked_neg()?;
        let interval = shift_indexed_interval(&interval, component_shift)?;
        if interval.west.worlds != 0 {
            return None;
        }
        for reference_shift in
            touching_reference_world_shifts(&interval, reference_west, reference_east)?
        {
            let polygon =
                project_polygon(component, component_shift.checked_add(reference_shift)?)?;
            if validate_component
                && !projected_polygon_has_valid_topology(&polygon, &mut topology_budget)
            {
                return None;
            }
            aligned.push(polygon);
        }
    }
    Some(aligned)
}

fn component_has_one_world_index(component: &UnwrappedPolygon) -> bool {
    let Some(first) = component
        .exterior
        .first()
        .map(|coordinate| coordinate.worlds)
    else {
        return false;
    };
    component
        .exterior
        .iter()
        .chain(component.interiors.iter().flatten())
        .all(|coordinate| coordinate.worlds == first)
}

fn polygons_longitudes_within_render_contract(polygons: &[Polygon<f64>]) -> bool {
    polygons.iter().all(|polygon| {
        std::iter::once(polygon.exterior())
            .chain(polygon.interiors().iter())
            .flat_map(|ring| ring.0.iter())
            .all(|coordinate| {
                coordinate.x.is_finite()
                    && coordinate.x >= -MAX_PROJECTED_RENDER_LONGITUDE
                    && coordinate.x <= MAX_PROJECTED_RENDER_LONGITUDE
            })
    })
}

fn overlapping_reference_world_shifts(
    interval: &IndexedLongitudeInterval,
    reference_west: f64,
    reference_east: f64,
) -> Option<Vec<i32>> {
    let reference_west = ExactExpansion::from_terms([reference_west])?;
    let reference_east = ExactExpansion::from_terms([reference_east])?;
    let mut shifts = Vec::with_capacity(2);
    // Both west endpoints are canonical world-zero values and both intervals
    // are at most one world wide, so only these adjacent copies can overlap.
    for worlds in [-1, 0, 1] {
        let west = exact_indexed_value(interval.west.shifted(worlds)?)?;
        let east = exact_indexed_value(interval.east.shifted(worlds)?)?;
        if east.exact_cmp(&reference_west)? == Ordering::Greater
            && west.exact_cmp(&reference_east)? == Ordering::Less
        {
            shifts.push(worlds);
        }
    }
    (shifts.len() <= 2).then_some(shifts)
}

fn touching_reference_world_shifts(
    interval: &IndexedLongitudeInterval,
    reference_west: f64,
    reference_east: f64,
) -> Option<Vec<i32>> {
    let mut shifts = Vec::with_capacity(3);
    for worlds in [-1, 0, 1] {
        // Filtering is defined by `geo::Intersects` on the final projected
        // f64 geometry. Select closed-interval touches using those same rounded
        // endpoints so a shared decimal seam is not rejected before `geo`
        // observes the identical coordinate.
        let projected = project_interval(&shift_indexed_interval(interval, worlds)?)?;
        if projected.east >= reference_west && projected.west <= reference_east {
            shifts.push(worlds);
        }
    }
    (shifts.len() <= 3).then_some(shifts)
}

fn latitude_bounds(geometry: &MultiPolygon<f64>) -> Option<(f64, f64)> {
    let [_, south, _, north] = geometry_bbox(geometry)?;
    Some((south, north))
}

fn minimal_continuous_longitude_interval(
    geometry: &MultiPolygon<f64>,
) -> Option<IndexedLongitudeInterval> {
    let polygons = unwrap_polygons(geometry)?;
    if geometry_has_full_world_edge(geometry) {
        return full_world_indexed_interval();
    }
    minimum_actual_component_union(&polygons)
}

fn minimum_actual_component_union(
    polygons: &[UnwrappedPolygon],
) -> Option<IndexedLongitudeInterval> {
    let cover = minimum_circular_cover(polygons)?;
    let CircularCover::Arc(cover_interval) = cover else {
        return full_world_indexed_interval();
    };

    // A largest uncovered gap fixes the relative world copy of every actual
    // component interval. Circular values select copies only; the final extrema
    // and validity below remain indexed source endpoints, so projected hints
    // can never clip a source coordinate inward.
    let relative_shifts = polygons
        .iter()
        .map(|component| component_shift_to_fit_cover(component, &cover_interval))
        .collect::<Option<Vec<_>>>()?;
    canonicalized_actual_interval(polygons, &relative_shifts)
}

fn minimum_circular_cover(polygons: &[UnwrappedPolygon]) -> Option<CircularCover> {
    if polygons.is_empty() {
        return None;
    }

    let mut segments = Vec::with_capacity(polygons.len().saturating_mul(2));
    for component in polygons {
        let interval = indexed_component_interval_at(component, 0)?;
        match indexed_span(&interval)? {
            IndexedSpan::Zero | IndexedSpan::MoreThanWorld => return None,
            IndexedSpan::FullWorld => return Some(CircularCover::Full),
            IndexedSpan::LessThanWorld => {}
        }

        if interval.east.worlds == interval.west.worlds {
            segments.push(CircularSegment {
                start: interval.west.source,
                end: interval.east.source,
            });
        } else if interval.east.worlds == interval.west.worlds.checked_add(1)? {
            segments.push(CircularSegment {
                start: interval.west.source,
                end: 180.0,
            });
            segments.push(CircularSegment {
                start: -180.0,
                end: interval.east.source,
            });
        } else {
            return None;
        }
    }

    segments.sort_by(|left, right| {
        left.start
            .total_cmp(&right.start)
            .then(left.end.total_cmp(&right.end))
    });
    let mut merged: Vec<CircularSegment> = Vec::with_capacity(segments.len());
    for segment in segments {
        if let Some(last) = merged.last_mut() {
            if segment.start <= last.end {
                last.end = last.end.max(segment.end);
                continue;
            }
        }
        merged.push(segment);
    }
    let first = *merged.first()?;
    let last = *merged.last()?;

    // Stable sorted order is the deterministic tie-break: the first exact
    // maximum gap wins. Its indexed complement is the minimum circular arc.
    let mut best: Option<(ExactExpansion, IndexedLongitudeInterval)> = None;
    for pair in merged.windows(2) {
        let gap_west = WorldIndexedLongitude::canonical(pair[0].end, 0)?;
        let gap_east = WorldIndexedLongitude::canonical(pair[1].start, 0)?;
        let gap = exact_indexed_value(gap_east)?.sub(&exact_indexed_value(gap_west)?)?;
        let cover = IndexedLongitudeInterval {
            west: gap_east,
            east: gap_west.shifted(1)?,
        };
        if best
            .as_ref()
            .is_none_or(|(current, _)| gap.exact_cmp(current) == Some(Ordering::Greater))
        {
            best = Some((gap, cover));
        }
    }
    let wrap_gap_west = WorldIndexedLongitude::canonical(last.end, 0)?;
    let wrap_gap_east = WorldIndexedLongitude::canonical(first.start, 1)?;
    let wrap_gap = exact_indexed_value(wrap_gap_east)?.sub(&exact_indexed_value(wrap_gap_west)?)?;
    let wrap_cover = IndexedLongitudeInterval {
        west: WorldIndexedLongitude::canonical(first.start, 0)?,
        east: WorldIndexedLongitude::canonical(last.end, 0)?,
    };
    if best
        .as_ref()
        .is_none_or(|(current, _)| wrap_gap.exact_cmp(current) == Some(Ordering::Greater))
    {
        best = Some((wrap_gap, wrap_cover));
    }

    let (largest_gap, cover) = best?;
    if largest_gap.sign() != 1 {
        Some(CircularCover::Full)
    } else {
        Some(CircularCover::Arc(cover))
    }
}

fn component_shift_to_fit_cover(
    component: &UnwrappedPolygon,
    cover: &IndexedLongitudeInterval,
) -> Option<i32> {
    interval_shift_to_fit(&indexed_component_interval_at(component, 0)?, cover)
}

fn interval_shift_to_fit(
    interval: &IndexedLongitudeInterval,
    container: &IndexedLongitudeInterval,
) -> Option<i32> {
    let preferred = container.west.worlds.checked_sub(interval.west.worlds)?;
    [
        Some(preferred),
        preferred.checked_sub(1),
        preferred.checked_add(1),
    ]
    .into_iter()
    .flatten()
    .find(|&worlds| {
        let Some(candidate) = shift_indexed_interval(interval, worlds) else {
            return false;
        };
        candidate.west.indexed_cmp(container.west) != Ordering::Less
            && candidate.east.indexed_cmp(container.east) != Ordering::Greater
    })
}

fn component_shift_to_projected_domain(
    component: &UnwrappedPolygon,
    display_west: f64,
    display_east: f64,
) -> Option<i32> {
    let component_west = projected_longitude(component.west, 0)?;
    let preferred = ceiling_world_shift(display_west - component_west)?;
    [
        Some(preferred),
        preferred.checked_sub(1),
        preferred.checked_add(1),
    ]
    .into_iter()
    .flatten()
    .find(|&worlds| {
        let Some(candidate) = indexed_component_interval_at(component, worlds) else {
            return false;
        };
        project_interval(&candidate).is_some_and(|projected| {
            projected.west >= display_west && projected.east <= display_east
        })
    })
}

fn ceiling_world_shift(delta: f64) -> Option<i32> {
    let worlds = (delta / 360.0).ceil();
    if !worlds.is_finite() || worlds < f64::from(i32::MIN) || worlds > f64::from(i32::MAX) {
        return None;
    }
    Some(worlds as i32)
}

fn canonicalized_actual_interval(
    polygons: &[UnwrappedPolygon],
    relative_shifts: &[i32],
) -> Option<IndexedLongitudeInterval> {
    // The circular sweep already uses the canonical [-180, 180) seam. Check
    // that placement first, then its two adjacent integer-world copies.
    for global_shift in [0_i32, -1, 1] {
        let shifts = relative_shifts
            .iter()
            .map(|relative| relative.checked_add(global_shift))
            .collect::<Option<Vec<_>>>()?;
        let candidate = interval_from_component_shifts(polygons, &shifts)?;
        if candidate.west.worlds == 0 && valid_actual_interval(&candidate)? {
            return Some(candidate);
        }
    }
    None
}

fn interval_from_component_shifts(
    polygons: &[UnwrappedPolygon],
    shifts: &[i32],
) -> Option<IndexedLongitudeInterval> {
    if polygons.is_empty() || polygons.len() != shifts.len() {
        return None;
    }
    let mut west: Option<WorldIndexedLongitude> = None;
    let mut east: Option<WorldIndexedLongitude> = None;
    for (component, worlds) in polygons.iter().zip(shifts) {
        let interval = indexed_component_interval_at(component, *worlds)?;
        if west.is_none_or(|current| interval.west.indexed_cmp(current) == Ordering::Less) {
            west = Some(interval.west);
        }
        if east.is_none_or(|current| interval.east.indexed_cmp(current) == Ordering::Greater) {
            east = Some(interval.east);
        }
    }
    Some(IndexedLongitudeInterval {
        west: west?,
        east: east?,
    })
}

fn valid_actual_interval(interval: &IndexedLongitudeInterval) -> Option<bool> {
    Some(matches!(
        indexed_span(interval)?,
        IndexedSpan::LessThanWorld | IndexedSpan::FullWorld
    ))
}

fn projected_longitude(longitude: WorldIndexedLongitude, additional_worlds: i32) -> Option<f64> {
    let longitude = longitude.shifted(additional_worlds)?;
    let projected = longitude.source + f64::from(longitude.worlds) * 360.0;
    projected.is_finite().then_some(projected)
}

fn projected_source_longitude(source: f64, worlds: i32, additional_worlds: i32) -> Option<f64> {
    let total_worlds = worlds.checked_add(additional_worlds)?;
    let projected = source + f64::from(total_worlds) * 360.0;
    projected.is_finite().then_some(projected)
}

fn indexed_component_interval_at(
    component: &UnwrappedPolygon,
    additional_worlds: i32,
) -> Option<IndexedLongitudeInterval> {
    shift_indexed_interval(
        &IndexedLongitudeInterval {
            west: component.west,
            east: component.east,
        },
        additional_worlds,
    )
}

fn shift_indexed_interval(
    interval: &IndexedLongitudeInterval,
    additional_worlds: i32,
) -> Option<IndexedLongitudeInterval> {
    Some(IndexedLongitudeInterval {
        west: interval.west.shifted(additional_worlds)?,
        east: interval.east.shifted(additional_worlds)?,
    })
}

fn project_interval(interval: &IndexedLongitudeInterval) -> Option<LongitudeInterval> {
    Some(LongitudeInterval {
        west: projected_longitude(interval.west, 0)?,
        east: projected_longitude(interval.east, 0)?,
    })
}

fn indexed_span(interval: &IndexedLongitudeInterval) -> Option<IndexedSpan> {
    match interval.east.indexed_cmp(interval.west) {
        Ordering::Less => None,
        Ordering::Equal => Some(IndexedSpan::Zero),
        Ordering::Greater => {
            let one_world_east = interval.west.shifted(1)?;
            Some(match interval.east.indexed_cmp(one_world_east) {
                Ordering::Less => IndexedSpan::LessThanWorld,
                Ordering::Equal => IndexedSpan::FullWorld,
                Ordering::Greater => IndexedSpan::MoreThanWorld,
            })
        }
    }
}

fn full_world_indexed_interval() -> Option<IndexedLongitudeInterval> {
    let west = WorldIndexedLongitude::canonical(-180.0, 0)?;
    Some(IndexedLongitudeInterval {
        west,
        east: west.shifted(1)?,
    })
}

fn unwrap_polygons(geometry: &MultiPolygon<f64>) -> Option<Vec<UnwrappedPolygon>> {
    if geometry.0.is_empty() {
        return None;
    }
    geometry.0.iter().map(unwrap_polygon).collect()
}

fn unwrap_polygon(polygon: &Polygon<f64>) -> Option<UnwrappedPolygon> {
    let exterior = unwrap_ring(polygon.exterior())?;
    let (exterior_west, exterior_east) = ring_longitude_bounds(&exterior)?;
    let exterior_interval = IndexedLongitudeInterval {
        west: exterior_west,
        east: exterior_east,
    };
    let mut west = exterior_west;
    let mut east = exterior_east;
    let mut interiors = Vec::with_capacity(polygon.interiors().len());

    for ring in polygon.interiors() {
        let ring = unwrap_ring(ring)?;
        let (ring_west, ring_east) = ring_longitude_bounds(&ring)?;
        let ring_interval = IndexedLongitudeInterval {
            west: ring_west,
            east: ring_east,
        };
        let shift = interval_shift_to_fit(&ring_interval, &exterior_interval)?;
        let ring = offset_ring_worlds(ring, shift)?;
        let (ring_west, ring_east) = ring_longitude_bounds(&ring)?;
        if ring_west.indexed_cmp(west) == Ordering::Less {
            west = ring_west;
        }
        if ring_east.indexed_cmp(east) == Ordering::Greater {
            east = ring_east;
        }
        interiors.push(ring);
    }

    let interval = IndexedLongitudeInterval { west, east };
    if !matches!(
        indexed_span(&interval)?,
        IndexedSpan::LessThanWorld | IndexedSpan::FullWorld
    ) {
        return None;
    }
    Some(UnwrappedPolygon {
        exterior,
        interiors,
        west,
        east,
    })
}

fn unwrap_ring(ring: &LineString<f64>) -> Option<WorldIndexedRing> {
    if ring.0.len() < 4 || ring.0.first() != ring.0.last() {
        return None;
    }
    let first = ring.0[0];
    if !valid_geographic_coord(first) {
        return None;
    }
    let mut unwrapped = Vec::with_capacity(ring.0.len());
    let mut worlds = 0_i32;
    unwrapped.push(WorldIndexedCoord {
        source: first,
        worlds,
    });

    for pair in ring.0.windows(2) {
        let raw_previous = pair[0];
        let raw_current = pair[1];
        if !valid_geographic_coord(raw_current) {
            return None;
        }
        let compared_to_positive_180 = exact_difference_cmp(raw_current.x, raw_previous.x, 180.0)?;
        let compared_to_negative_180 = exact_difference_cmp(raw_current.x, raw_previous.x, -180.0)?;
        let rounded_delta = raw_current.x - raw_previous.x;
        let world_delta = if is_explicit_full_world_edge(raw_previous.x, raw_current.x)
            || rounded_delta == 180.0
            || rounded_delta == -180.0
            || compared_to_positive_180 == Ordering::Equal
            || compared_to_negative_180 == Ordering::Equal
        {
            0
        } else if compared_to_positive_180 == Ordering::Greater {
            -1
        } else if compared_to_negative_180 == Ordering::Less {
            1
        } else {
            0
        };
        worlds = worlds.checked_add(world_delta)?;
        unwrapped.push(WorldIndexedCoord {
            source: raw_current,
            worlds,
        });
    }

    let last = *unwrapped.last()?;
    if last.source != first || last.worlds != 0 {
        return None;
    }
    let (west, east) = ring_longitude_bounds(&unwrapped)?;
    (!matches!(
        indexed_span(&IndexedLongitudeInterval { west, east })?,
        IndexedSpan::MoreThanWorld
    ))
    .then_some(unwrapped)
}

fn valid_geographic_coord(coord: Coord<f64>) -> bool {
    coord.x.is_finite()
        && coord.y.is_finite()
        && (-180.0..=180.0).contains(&coord.x)
        && (-90.0..=90.0).contains(&coord.y)
}

fn ring_longitude_bounds(
    ring: &[WorldIndexedCoord],
) -> Option<(WorldIndexedLongitude, WorldIndexedLongitude)> {
    let first = *ring.first()?;
    let mut west = WorldIndexedLongitude::canonical(first.source.x, first.worlds)?;
    let mut east = west;
    for coord in &ring[1..] {
        let longitude = WorldIndexedLongitude::canonical(coord.source.x, coord.worlds)?;
        if longitude.indexed_cmp(west) == Ordering::Less {
            west = longitude;
        }
        if longitude.indexed_cmp(east) == Ordering::Greater {
            east = longitude;
        }
    }
    Some((west, east))
}

fn offset_ring_worlds(
    mut ring: WorldIndexedRing,
    additional_worlds: i32,
) -> Option<WorldIndexedRing> {
    for coord in &mut ring {
        coord.worlds = coord.worlds.checked_add(additional_worlds)?;
    }
    Some(ring)
}

fn geometry_has_full_world_edge(geometry: &MultiPolygon<f64>) -> bool {
    geometry.0.iter().any(|polygon| {
        std::iter::once(polygon.exterior())
            .chain(polygon.interiors().iter())
            .any(|ring| {
                ring.0
                    .windows(2)
                    .any(|edge| is_explicit_full_world_edge(edge[0].x, edge[1].x))
            })
    })
}

fn is_explicit_full_world_edge(first: f64, second: f64) -> bool {
    (first == -180.0 && second == 180.0) || (first == 180.0 && second == -180.0)
}

fn full_world_envelope(south: f64, north: f64) -> WrappedAoiEnvelope {
    WrappedAoiEnvelope {
        south,
        north,
        crosses_antimeridian: false,
        display_west: -180.0,
        display_east: 180.0,
        request_windows: vec![bbox(-180.0, south, 180.0, north)],
    }
}

fn split_at_antimeridian(
    geometry: &MultiPolygon<f64>,
    interval: &LongitudeInterval,
    south: f64,
    north: f64,
) -> Option<Vec<BBox>> {
    let west = interval.west;
    let east = interval.east;
    if !west.is_finite()
        || !east.is_finite()
        || !south.is_finite()
        || !north.is_finite()
        || !(-180.0..180.0).contains(&west)
        || east <= west
    {
        return None;
    }

    let aligned = align_polygonal_to_longitude_domain(geometry, west, east)?;
    let windows = if east <= 180.0 {
        let (source_west, source_east) = non_crossing_source_extrema(geometry, &aligned)?;
        vec![bbox(
            west.min(source_west),
            south,
            east.max(source_east),
            north,
        )]
    } else {
        // Classify by the actual aligned copy, then take request boundaries
        // directly from the paired source coordinate.
        let (first_west, second_east) = crossing_source_extrema(geometry, &aligned)?;
        vec![
            bbox(west.min(first_west), south, 180.0, north),
            bbox(-180.0, south, second_east, north),
        ]
    };
    (windows.len() <= 2 && windows.iter().all(|window| window.validate().is_ok()))
        .then_some(windows)
}

fn non_crossing_source_extrema(
    source: &MultiPolygon<f64>,
    aligned: &MultiPolygon<f64>,
) -> Option<(f64, f64)> {
    let mut west = f64::INFINITY;
    let mut east = f64::NEG_INFINITY;
    visit_paired_coordinates(source, aligned, |source_coord, aligned_coord| {
        let request_longitude = request_longitude_for_aligned_copy(source_coord, aligned_coord)?;
        west = west.min(request_longitude);
        east = east.max(request_longitude);
        Some(())
    })?;
    (west.is_finite() && east.is_finite()).then_some((west, east))
}

fn crossing_source_extrema(
    source: &MultiPolygon<f64>,
    aligned: &MultiPolygon<f64>,
) -> Option<(f64, f64)> {
    let mut first_west: Option<f64> = None;
    let mut second_east: Option<f64> = None;
    visit_paired_coordinates(source, aligned, |source_coord, aligned_coord| {
        let request_longitude = request_longitude_for_aligned_copy(source_coord, aligned_coord)?;
        if aligned_coord.x > 180.0 {
            second_east = Some(
                second_east.map_or(request_longitude, |current| current.max(request_longitude)),
            );
        } else {
            first_west = Some(
                first_west.map_or(request_longitude, |current| current.min(request_longitude)),
            );
        }
        Some(())
    })?;
    Some((first_west?, second_east?))
}

fn visit_paired_coordinates(
    source: &MultiPolygon<f64>,
    aligned: &MultiPolygon<f64>,
    mut visitor: impl FnMut(Coord<f64>, Coord<f64>) -> Option<()>,
) -> Option<()> {
    if source.0.len() != aligned.0.len() {
        return None;
    }
    for (source_polygon, aligned_polygon) in source.0.iter().zip(&aligned.0) {
        visit_paired_ring(
            source_polygon.exterior(),
            aligned_polygon.exterior(),
            &mut visitor,
        )?;
        if source_polygon.interiors().len() != aligned_polygon.interiors().len() {
            return None;
        }
        for (source_ring, aligned_ring) in source_polygon
            .interiors()
            .iter()
            .zip(aligned_polygon.interiors())
        {
            visit_paired_ring(source_ring, aligned_ring, &mut visitor)?;
        }
    }
    Some(())
}

fn visit_paired_ring(
    source: &LineString<f64>,
    aligned: &LineString<f64>,
    visitor: &mut impl FnMut(Coord<f64>, Coord<f64>) -> Option<()>,
) -> Option<()> {
    if source.0.len() != aligned.0.len() {
        return None;
    }
    for (&source_coord, &aligned_coord) in source.0.iter().zip(&aligned.0) {
        visitor(source_coord, aligned_coord)?;
    }
    Some(())
}

fn request_longitude_for_aligned_copy(source: Coord<f64>, aligned: Coord<f64>) -> Option<f64> {
    if !source.x.is_finite() || !aligned.x.is_finite() {
        return None;
    }
    if source.x == -180.0 && aligned.x == 180.0 {
        Some(180.0)
    } else if source.x == 180.0 && aligned.x == -180.0 {
        Some(-180.0)
    } else {
        Some(source.x)
    }
}

fn bbox(west: f64, south: f64, east: f64, north: f64) -> BBox {
    BBox {
        west,
        east,
        south,
        north,
        crs: "EPSG:4326".into(),
    }
}

fn project_polygon(polygon: &UnwrappedPolygon, additional_worlds: i32) -> Option<Polygon<f64>> {
    Some(Polygon::new(
        project_ring(&polygon.exterior, additional_worlds)?,
        polygon
            .interiors
            .iter()
            .map(|ring| project_ring(ring, additional_worlds))
            .collect::<Option<Vec<_>>>()?,
    ))
}

fn project_ring(ring: &[WorldIndexedCoord], additional_worlds: i32) -> Option<LineString<f64>> {
    let coordinates = ring
        .iter()
        .map(|coord| {
            Some(Coord {
                x: projected_source_longitude(coord.source.x, coord.worlds, additional_worlds)?,
                y: coord.source.y,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(LineString::new(coordinates))
}
