use std::panic::{catch_unwind, AssertUnwindSafe};

use geo::{
    coordinate_position::{CoordPos, CoordinatePosition},
    line_intersection::{line_intersection, LineIntersection},
    Area, BooleanOps, BoundingRect, Coord, Line, MultiPolygon, Polygon, Rect,
};
use insar_core::{
    align_polygonal_to_longitude_domain, align_polygonal_to_longitude_reference,
    validated_authoritative_polygonal_geojson, validated_polygonal_geojson, wrapped_aoi_envelope,
    BBox,
};
use serde_json::{json, Map, Value};

#[derive(Clone, Debug)]
struct PreparedCatalogueAoiPolygon {
    polygon: Polygon<f64>,
    single: MultiPolygon<f64>,
    rect: Option<Rect<f64>>,
}

#[derive(Clone, Debug)]
pub struct PreparedCatalogueAoi {
    polygons: Vec<PreparedCatalogueAoiPolygon>,
    display_west: f64,
    display_east: f64,
}

impl PreparedCatalogueAoi {
    pub fn from_geojson(aoi: &Map<String, Value>) -> Option<Self> {
        let canonical = authoritative_catalogue_geometry(aoi)?;
        Self::new(&canonical)
    }

    pub fn new(aoi: &MultiPolygon<f64>) -> Option<Self> {
        let (aligned_aoi, display_west, display_east) =
            if let Some(envelope) = wrapped_aoi_envelope(aoi) {
                let aligned = align_polygonal_to_longitude_domain(
                    aoi,
                    envelope.display_west,
                    envelope.display_east,
                )
                .or_else(|| {
                    let mut polygons = Vec::with_capacity(aoi.0.len());
                    for polygon in &aoi.0 {
                        polygons.extend(
                            align_polygonal_to_longitude_domain(
                                &MultiPolygon(vec![polygon.clone()]),
                                envelope.display_west,
                                envelope.display_east,
                            )?
                            .0,
                        );
                    }
                    Some(MultiPolygon(polygons))
                })?;
                (aligned, envelope.display_west, envelope.display_east)
            } else {
                let rect = aoi.bounding_rect()?;
                if !rect.min().x.is_finite()
                    || !rect.max().x.is_finite()
                    || rect.min().x < -180.0
                    || rect.max().x > 180.0
                    || rect.max().x <= rect.min().x
                    || rect.max().x - rect.min().x > 180.0
                {
                    return None;
                }
                (aoi.clone(), rect.min().x, rect.max().x)
            };

        let polygons = aligned_aoi
            .0
            .into_iter()
            .map(|polygon| {
                let rect = polygon.bounding_rect();
                let single = MultiPolygon(vec![polygon.clone()]);
                PreparedCatalogueAoiPolygon {
                    polygon,
                    single,
                    rect,
                }
            })
            .collect();

        Some(Self {
            polygons,
            display_west,
            display_east,
        })
    }

    pub fn intersects_footprint(&self, footprint_geojson: &Map<String, Value>) -> bool {
        let Some(footprint) = validated_polygonal_geojson(footprint_geojson) else {
            return false;
        };
        let Some(aligned_footprint) = align_polygonal_to_longitude_reference(
            &footprint,
            self.display_west,
            self.display_east,
        ) else {
            return false;
        };
        for aoi_poly in &self.polygons {
            for fp_poly in aligned_footprint.as_slice() {
                if polygon_pair_has_positive_overlap(aoi_poly, fp_poly) == Some(true) {
                    return true;
                }
            }
        }
        false
    }

    /// Positive-area overlap (in squared degrees, for ordering only) between
    /// a request rectangle and the exact AOI. `None` when it cannot be
    /// decided; the caller then keeps the rectangle rather than guess.
    pub(crate) fn request_tile_overlap_area(
        &self,
        tile_geojson: &Map<String, Value>,
    ) -> Option<f64> {
        let tile = validated_polygonal_geojson(tile_geojson)?;
        let aligned =
            align_polygonal_to_longitude_reference(&tile, self.display_west, self.display_east)?;
        let mut total = 0.0;
        for aoi_poly in &self.polygons {
            let Some(aoi_rect) = aoi_poly.rect else {
                continue;
            };
            for tile_poly in aligned.as_slice() {
                let Some(tile_rect) = tile_poly.bounding_rect() else {
                    continue;
                };
                if tile_rect.max().x <= aoi_rect.min().x
                    || tile_rect.min().x >= aoi_rect.max().x
                    || tile_rect.max().y <= aoi_rect.min().y
                    || tile_rect.min().y >= aoi_rect.max().y
                {
                    continue;
                }
                match polygon_pair_has_positive_overlap(aoi_poly, tile_poly) {
                    Some(false) => continue,
                    Some(true) => {}
                    None => return None,
                }
                let intersection = catch_unwind(AssertUnwindSafe(|| {
                    tile_poly.intersection(&aoi_poly.single)
                }))
                .ok()?;
                let area = intersection.unsigned_area();
                if !area.is_finite() || area < 0.0 {
                    return None;
                }
                total += area;
            }
        }
        Some(total)
    }
}

/// Request rectangles that can contain exact AOI hits, most-overlapping
/// first. A rectangle with zero positive-area overlap (for example the sea or
/// a neighbouring country inside a large administrative envelope) cannot
/// contribute an admitted product and is not requested. Ties keep the
/// deterministic planner order. Undecidable overlap keeps the rectangle.
pub(crate) fn order_request_tiles_by_exact_overlap(
    prepared: Option<&PreparedCatalogueAoi>,
    tiles: Vec<Map<String, Value>>,
) -> Vec<Map<String, Value>> {
    let Some(prepared) = prepared else {
        return tiles;
    };
    let mut scored = tiles
        .into_iter()
        .enumerate()
        .filter_map(
            |(index, tile)| match prepared.request_tile_overlap_area(&tile) {
                Some(area) if area > 0.0 => Some((area, index, tile)),
                Some(_) => None,
                None => Some((-1.0, index, tile)),
            },
        )
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.0.total_cmp(&left.0).then(left.1.cmp(&right.1)));
    scored.into_iter().map(|(_, _, tile)| tile).collect()
}

fn polygon_pair_has_positive_overlap(
    aoi_poly: &PreparedCatalogueAoiPolygon,
    fp_poly: &Polygon<f64>,
) -> Option<bool> {
    let Some(aoi_rect) = aoi_poly.rect else {
        return Some(false);
    };
    let Some(fp_rect) = fp_poly.bounding_rect() else {
        return Some(false);
    };

    // 1. Disjoint or boundary-only touching bounding boxes have zero 2D overlap area.
    if fp_rect.max().x <= aoi_rect.min().x
        || fp_rect.min().x >= aoi_rect.max().x
        || fp_rect.max().y <= aoi_rect.min().y
        || fp_rect.min().y >= aoi_rect.max().y
    {
        return Some(false);
    }

    // 2. Both polygons are validated OGC simple polygons (positive area, no spikes), so every
    //    boundary vertex lies in the closure of its polygon's open interior. If any AOI vertex
    //    lies strictly inside `fp_poly` (`CoordPos::Inside`), open-interior overlap is > 0.
    for ring in std::iter::once(aoi_poly.polygon.exterior()).chain(aoi_poly.polygon.interiors()) {
        for &coord in &ring.0 {
            if coord.x > fp_rect.min().x
                && coord.x < fp_rect.max().x
                && coord.y > fp_rect.min().y
                && coord.y < fp_rect.max().y
                && fp_poly.coordinate_position(&coord) == CoordPos::Inside
            {
                return Some(true);
            }
        }
    }

    // 3. Check one representative vertex from each ring of `fp_poly` against `aoi_poly`.
    for ring in std::iter::once(fp_poly.exterior()).chain(fp_poly.interiors()) {
        if let Some(&coord) = ring.0.first() {
            if coord.x > aoi_rect.min().x
                && coord.x < aoi_rect.max().x
                && coord.y > aoi_rect.min().y
                && coord.y < aoi_rect.max().y
                && aoi_poly.polygon.coordinate_position(&coord) == CoordPos::Inside
            {
                return Some(true);
            }
        }
    }

    // 4. Collect AOI boundary segments overlapping `fp_rect` and test against `fp_poly` segments.
    //    If boundaries do not intersect at all and neither polygon has a ring strictly inside
    //    the other (checked in steps 2 & 3), `aoi_poly ∩ fp_poly` is empty (`area == 0`).
    let fp_lines: Vec<Line<f64>> = fp_poly
        .exterior()
        .lines()
        .chain(fp_poly.interiors().iter().flat_map(|ring| ring.lines()))
        .collect();
    let mut candidate_aoi_lines: Vec<Line<f64>> = Vec::new();
    let mut boundaries_intersect = false;
    for line in aoi_poly.polygon.exterior().lines().chain(
        aoi_poly
            .polygon
            .interiors()
            .iter()
            .flat_map(|ring| ring.lines()),
    ) {
        let lx_min = line.start.x.min(line.end.x);
        let lx_max = line.start.x.max(line.end.x);
        let ly_min = line.start.y.min(line.end.y);
        let ly_max = line.start.y.max(line.end.y);
        if lx_max < fp_rect.min().x
            || lx_min > fp_rect.max().x
            || ly_max < fp_rect.min().y
            || ly_min > fp_rect.max().y
        {
            continue;
        }
        candidate_aoi_lines.push(line);
        if !boundaries_intersect {
            for fp_line in &fp_lines {
                if line_intersection(line, *fp_line).is_some() {
                    boundaries_intersect = true;
                    break;
                }
            }
        }
    }
    if !boundaries_intersect {
        return Some(false);
    }

    // 5. Boundaries cross or touch: check remaining footprint vertices for strict containment.
    for ring in std::iter::once(fp_poly.exterior()).chain(fp_poly.interiors()) {
        for &coord in ring.0.iter().skip(1) {
            if coord.x > aoi_rect.min().x
                && coord.x < aoi_rect.max().x
                && coord.y > aoi_rect.min().y
                && coord.y < aoi_rect.max().y
                && aoi_poly.polygon.coordinate_position(&coord) == CoordPos::Inside
            {
                return Some(true);
            }
        }
    }

    // 6. Edge-straddling crossing check without full sweep-line BooleanOps over 150k+ vertices:
    //    split each footprint edge (and intersecting AOI edges) at boundary intersection points;
    //    if any open sub-segment midpoint lies strictly inside the other polygon, positive 2D
    //    area overlap is guaranteed (preserving every edge-straddling satellite scene).
    if any_split_subsegment_midpoint_inside(&fp_lines, &candidate_aoi_lines, &aoi_poly.polygon)
        || any_split_subsegment_midpoint_inside(&candidate_aoi_lines, &fp_lines, fp_poly)
    {
        return Some(true);
    }

    // 7. Remaining degenerate case: boundaries share collinear segments without entering each
    //    other's open interior (e.g. identical polygons or shared-edge touch).
    if let Ok(intersection) =
        catch_unwind(AssertUnwindSafe(|| fp_poly.intersection(&aoi_poly.single)))
    {
        let area = intersection.unsigned_area();
        if area.is_finite() && area >= 0.0 {
            return Some(area > 0.0);
        }
    }
    Some(collinear_shared_boundary_has_same_side_interior(
        &fp_lines,
        &candidate_aoi_lines,
        fp_poly,
        &aoi_poly.polygon,
    ))
}

fn line_param_t(line: &Line<f64>, pt: Coord<f64>) -> f64 {
    let dx = line.end.x - line.start.x;
    let dy = line.end.y - line.start.y;
    let denom = dx * dx + dy * dy;
    if denom <= 0.0 {
        return 0.0;
    }
    (((pt.x - line.start.x) * dx + (pt.y - line.start.y) * dy) / denom).clamp(0.0, 1.0)
}

fn any_split_subsegment_midpoint_inside(
    primary_lines: &[Line<f64>],
    clip_lines: &[Line<f64>],
    target_polygon: &Polygon<f64>,
) -> bool {
    let Some(target_rect) = target_polygon.bounding_rect() else {
        return false;
    };
    let mut params = Vec::with_capacity(16);
    for &seg in primary_lines {
        let sx_min = seg.start.x.min(seg.end.x);
        let sx_max = seg.start.x.max(seg.end.x);
        let sy_min = seg.start.y.min(seg.end.y);
        let sy_max = seg.start.y.max(seg.end.y);
        if sx_max <= target_rect.min().x
            || sx_min >= target_rect.max().x
            || sy_max <= target_rect.min().y
            || sy_min >= target_rect.max().y
        {
            continue;
        }
        params.clear();
        params.push(0.0);
        params.push(1.0);
        for &clip in clip_lines {
            let cx_min = clip.start.x.min(clip.end.x);
            let cx_max = clip.start.x.max(clip.end.x);
            let cy_min = clip.start.y.min(clip.end.y);
            let cy_max = clip.start.y.max(clip.end.y);
            if cx_max < sx_min || cx_min > sx_max || cy_max < sy_min || cy_min > sy_max {
                continue;
            }
            match line_intersection(seg, clip) {
                Some(LineIntersection::SinglePoint { intersection, .. }) => {
                    params.push(line_param_t(&seg, intersection));
                }
                Some(LineIntersection::Collinear { intersection }) => {
                    params.push(line_param_t(&seg, intersection.start));
                    params.push(line_param_t(&seg, intersection.end));
                }
                None => {}
            }
        }
        if params.len() <= 2 {
            continue;
        }
        params.sort_by(|a, b| a.total_cmp(b));
        for pair in params.windows(2) {
            let (t0, t1) = (pair[0], pair[1]);
            if t1 - t0 <= 1e-9 {
                continue;
            }
            let tm = 0.5 * (t0 + t1);
            let mid = Coord {
                x: seg.start.x + (seg.end.x - seg.start.x) * tm,
                y: seg.start.y + (seg.end.y - seg.start.y) * tm,
            };
            if mid.x > target_rect.min().x
                && mid.x < target_rect.max().x
                && mid.y > target_rect.min().y
                && mid.y < target_rect.max().y
                && target_polygon.coordinate_position(&mid) == CoordPos::Inside
            {
                return true;
            }
        }
    }
    false
}

fn collinear_shared_boundary_has_same_side_interior(
    fp_lines: &[Line<f64>],
    aoi_lines: &[Line<f64>],
    fp_poly: &Polygon<f64>,
    aoi_poly: &Polygon<f64>,
) -> bool {
    for &fp_line in fp_lines {
        for &aoi_line in aoi_lines {
            if let Some(LineIntersection::Collinear { intersection }) =
                line_intersection(fp_line, aoi_line)
            {
                let dx = intersection.end.x - intersection.start.x;
                let dy = intersection.end.y - intersection.start.y;
                let len = dx.hypot(dy);
                if len <= 1e-12 {
                    continue;
                }
                let mid = Coord {
                    x: 0.5 * (intersection.start.x + intersection.end.x),
                    y: 0.5 * (intersection.start.y + intersection.end.y),
                };
                let eps = (len * 1e-4).min(1e-5);
                let nx = -dy / len * eps;
                let ny = dx / len * eps;
                for probe in [
                    Coord {
                        x: mid.x + nx,
                        y: mid.y + ny,
                    },
                    Coord {
                        x: mid.x - nx,
                        y: mid.y - ny,
                    },
                ] {
                    if fp_poly.coordinate_position(&probe) == CoordPos::Inside
                        && aoi_poly.coordinate_position(&probe) == CoordPos::Inside
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

pub fn authoritative_catalogue_geometry(aoi: &Map<String, Value>) -> Option<MultiPolygon<f64>> {
    match aoi.get("type").and_then(Value::as_str)? {
        "Polygon" | "MultiPolygon" => {
            validated_authoritative_polygonal_geojson(aoi).map(|validated| validated.geometry)
        }
        "Feature" => aoi
            .get("geometry")
            .and_then(Value::as_object)
            .and_then(authoritative_catalogue_geometry),
        "FeatureCollection" => {
            let mut polygons = Vec::new();
            for feature in aoi.get("features")?.as_array()? {
                let feature = feature.as_object()?;
                polygons.extend(authoritative_catalogue_geometry(feature)?.0);
            }
            (!polygons.is_empty()).then_some(MultiPolygon(polygons))
        }
        _ => None,
    }
}

pub(crate) fn footprint_intersects_exact_catalogue_aoi(
    footprint_geojson: &Map<String, Value>,
    aoi: &MultiPolygon<f64>,
) -> bool {
    PreparedCatalogueAoi::new(aoi)
        .is_some_and(|prepared| prepared.intersects_footprint(footprint_geojson))
}

/// Serialized AOI size above which a single-request STAC route (HLS, Earth
/// Search Landsat, Microsoft Landsat) sends a compact envelope instead.
pub(crate) const COMPACT_REQUEST_AOI_BYTES: usize = 24 * 1024;

/// A compact request envelope plus the exact admission filter it requires.
/// The envelope is a transport detail; only `exact` decides which returned
/// products belong to the user's AOI (SAR-REG-010).
pub(crate) struct CompactRequestAoi {
    pub(crate) request_geojson: Map<String, Value>,
    pub(crate) exact: PreparedCatalogueAoi,
}

/// For an oversized AOI, one envelope request rectangle and the prepared
/// original canonical AOI. `None` keeps the original AOI as the request
/// geometry: it is small enough, or no envelope could be paired with an exact
/// filter (an envelope without exact admission would leak foreign scenes).
pub(crate) fn compact_oversized_request_aoi(aoi: &Map<String, Value>) -> Option<CompactRequestAoi> {
    let size = serde_json::to_vec(aoi).map(|bytes| bytes.len()).ok()?;
    if size <= COMPACT_REQUEST_AOI_BYTES {
        return None;
    }
    let canonical = authoritative_catalogue_geometry(aoi)?;
    let exact = PreparedCatalogueAoi::new(&canonical)?;
    let request_geojson = bounded_catalogue_request_tiles(&canonical, 180.0, 90.0, 1)?
        .into_iter()
        .next()?;
    Some(CompactRequestAoi {
        request_geojson,
        exact,
    })
}

pub(crate) fn bounded_catalogue_request_tiles(
    aoi: &MultiPolygon<f64>,
    target_longitude_span: f64,
    target_latitude_span: f64,
    max_parts: usize,
) -> Option<Vec<Map<String, Value>>> {
    if !target_longitude_span.is_finite()
        || !target_latitude_span.is_finite()
        || target_longitude_span <= 0.0
        || target_latitude_span <= 0.0
        || max_parts == 0
    {
        return None;
    }
    let mut tiles = wrapped_aoi_envelope(aoi)
        .map(|envelope| envelope.request_windows)
        .or_else(|| native_catalogue_bbox(aoi).map(|bbox| vec![bbox]))?;
    if tiles.is_empty() || tiles.len() > max_parts {
        return None;
    }
    while tiles.len() < max_parts {
        let Some((index, _)) = tiles
            .iter()
            .enumerate()
            .filter_map(|(index, tile)| {
                let longitude_ratio = (tile.east - tile.west) / target_longitude_span;
                let latitude_ratio = (tile.north - tile.south) / target_latitude_span;
                let score = longitude_ratio.max(latitude_ratio);
                (score > 1.0).then_some((index, score))
            })
            .max_by(|left, right| left.1.total_cmp(&right.1))
        else {
            break;
        };
        let tile = tiles.remove(index);
        let width = tile.east - tile.west;
        let height = tile.north - tile.south;
        let (first, second) = if width / target_longitude_span >= height / target_latitude_span {
            let middle = tile.west + width / 2.0;
            (
                BBox {
                    east: middle,
                    ..tile.clone()
                },
                BBox {
                    west: middle,
                    ..tile
                },
            )
        } else {
            let middle = tile.south + height / 2.0;
            (
                BBox {
                    north: middle,
                    ..tile.clone()
                },
                BBox {
                    south: middle,
                    ..tile
                },
            )
        };
        tiles.extend([first, second]);
    }
    tiles.sort_by(|left, right| {
        left.west
            .total_cmp(&right.west)
            .then_with(|| left.south.total_cmp(&right.south))
            .then_with(|| left.east.total_cmp(&right.east))
            .then_with(|| left.north.total_cmp(&right.north))
    });
    Some(
        tiles
            .into_iter()
            .map(|tile| {
                json!({
                    "type": "Polygon",
                    "coordinates": [[
                        [tile.west, tile.south],
                        [tile.east, tile.south],
                        [tile.east, tile.north],
                        [tile.west, tile.north],
                        [tile.west, tile.south]
                    ]]
                })
                .as_object()
                .expect("catalogue tile is an object")
                .clone()
            })
            .collect(),
    )
}

fn native_catalogue_bbox(geometry: &MultiPolygon<f64>) -> Option<BBox> {
    let rect = geometry.bounding_rect()?;
    let bbox = BBox {
        west: rect.min().x,
        east: rect.max().x,
        south: rect.min().y,
        north: rect.max().y,
        crs: "EPSG:4326".to_owned(),
    };
    (bbox.east - bbox.west <= 180.0 && bbox.validate().is_ok()).then_some(bbox)
}

/// Test fixture: a China-like administrative AOI with two disjoint, densely
/// digitised components (west 97–100°E 37–40°N, north-east 120–123°E
/// 48–51°N). Serialized it exceeds [`COMPACT_REQUEST_AOI_BYTES`], so the
/// single-request STAC routes must send its envelope; the gap between the
/// components (e.g. 108–110°E 43–45°N) is inside the envelope but not the AOI.
#[cfg(test)]
pub(crate) fn dense_disjoint_china_like_aoi_for_test() -> Map<String, Value> {
    fn dense_ring(west: f64, south: f64, east: f64, north: f64, per_side: u32) -> Vec<Value> {
        let mut ring = Vec::with_capacity(per_side as usize * 4 + 1);
        for index in 0..per_side {
            let t = f64::from(index) / f64::from(per_side);
            ring.push(json!([west + (east - west) * t, south]));
        }
        for index in 0..per_side {
            let t = f64::from(index) / f64::from(per_side);
            ring.push(json!([east, south + (north - south) * t]));
        }
        for index in 0..per_side {
            let t = f64::from(index) / f64::from(per_side);
            ring.push(json!([east - (east - west) * t, north]));
        }
        for index in 0..per_side {
            let t = f64::from(index) / f64::from(per_side);
            ring.push(json!([west, north - (north - south) * t]));
        }
        ring.push(ring[0].clone());
        ring
    }
    let aoi = json!({
        "type": "MultiPolygon",
        "coordinates": [
            [dense_ring(97.0, 37.0, 100.0, 40.0, 400)],
            [dense_ring(120.0, 48.0, 123.0, 51.0, 400)]
        ]
    });
    let aoi = aoi.as_object().expect("fixture is an object").clone();
    assert!(serde_json::to_vec(&aoi).unwrap().len() > COMPACT_REQUEST_AOI_BYTES);
    aoi
}

#[cfg(test)]
mod tests {
    use super::*;
    use insar_core::validated_authoritative_polygonal_geojson;

    #[test]
    fn inner_mongolia_scale_geometry_is_split_into_small_bounded_tiles() {
        let geometry = json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[97.0,37.0],[108.0,37.0],[108.0,45.0],[97.0,45.0],[97.0,37.0]]],
                [[[108.5,42.0],[126.0,42.0],[126.0,53.0],[108.5,53.0],[108.5,42.0]]]
            ]
        });
        let validated =
            validated_authoritative_polygonal_geojson(geometry.as_object().unwrap()).unwrap();
        let tiles = bounded_catalogue_request_tiles(&validated.geometry, 15.0, 8.0, 8).unwrap();
        assert!((2..=8).contains(&tiles.len()));
        assert!(tiles
            .iter()
            .all(|tile| serde_json::to_vec(tile).unwrap().len() < 1024));
    }

    #[test]
    fn oversized_aoi_gets_one_envelope_only_with_its_exact_filter() {
        let aoi = dense_disjoint_china_like_aoi_for_test();
        let compact = compact_oversized_request_aoi(&aoi).expect("oversized AOI is compacted");
        let ring = compact.request_geojson["coordinates"][0]
            .as_array()
            .unwrap();
        assert_eq!(ring.len(), 5, "the request geometry is one rectangle");
        assert_eq!(ring[0], json!([97.0, 37.0]));
        assert_eq!(ring[2], json!([123.0, 51.0]));
        let gap = json!({"type":"Polygon","coordinates":[[[108.0,43.0],[110.0,43.0],[110.0,45.0],[108.0,45.0],[108.0,43.0]]]});
        let west = json!({"type":"Polygon","coordinates":[[[98.0,38.0],[99.0,38.0],[99.0,39.0],[98.0,39.0],[98.0,38.0]]]});
        assert!(!compact.exact.intersects_footprint(gap.as_object().unwrap()));
        assert!(compact
            .exact
            .intersects_footprint(west.as_object().unwrap()));

        let small = json!({"type":"Polygon","coordinates":[[[98.0,38.0],[99.0,38.0],[99.0,39.0],[98.0,39.0],[98.0,38.0]]]});
        assert!(compact_oversized_request_aoi(small.as_object().unwrap()).is_none());
    }

    #[test]
    fn request_tiles_without_exact_overlap_are_skipped_and_the_rest_ordered_by_overlap() {
        let aoi = json!({
            "type":"MultiPolygon",
            "coordinates":[
                // Small western component, large north-eastern component.
                [[[97.0,39.0],[98.0,39.0],[98.0,40.0],[97.0,40.0],[97.0,39.0]]],
                [[[121.0,46.0],[125.0,46.0],[125.0,51.0],[121.0,51.0],[121.0,46.0]]]
            ]
        });
        let canonical = authoritative_catalogue_geometry(aoi.as_object().unwrap()).unwrap();
        let tiles = bounded_catalogue_request_tiles(&canonical, 15.0, 8.0, 8).unwrap();
        assert_eq!(
            tiles.len(),
            4,
            "planner rectangles cover the whole envelope"
        );
        let prepared = PreparedCatalogueAoi::new(&canonical).unwrap();
        let ordered = order_request_tiles_by_exact_overlap(Some(&prepared), tiles.clone());
        assert_eq!(ordered.len(), 2, "the two gap rectangles are not requested");
        let west_of = |tile: &Map<String, Value>| tile["coordinates"][0][0][0].as_f64().unwrap();
        let south_of = |tile: &Map<String, Value>| tile["coordinates"][0][0][1].as_f64().unwrap();
        // The planner order starts in the west; the large overlap goes first.
        assert_eq!(west_of(&tiles[0]), 97.0);
        assert_eq!((west_of(&ordered[0]), south_of(&ordered[0])), (111.0, 45.0));
        assert_eq!((west_of(&ordered[1]), south_of(&ordered[1])), (97.0, 39.0));
        // Without an exact filter the plan is kept unchanged.
        assert_eq!(
            order_request_tiles_by_exact_overlap(None, tiles.clone()),
            tiles
        );
    }

    #[test]
    fn exact_filter_rejects_the_gap_between_disjoint_aoi_components() {
        let aoi = json!({
            "type":"MultiPolygon",
            "coordinates":[
                [[[97.0,37.0],[100.0,37.0],[100.0,40.0],[97.0,40.0],[97.0,37.0]]],
                [[[120.0,48.0],[123.0,48.0],[123.0,51.0],[120.0,51.0],[120.0,48.0]]]
            ]
        });
        let canonical = authoritative_catalogue_geometry(aoi.as_object().unwrap()).unwrap();
        let gap = json!({
            "type":"Polygon",
            "coordinates":[[[108.0,43.0],[110.0,43.0],[110.0,45.0],[108.0,45.0],[108.0,43.0]]]
        });
        let inside = json!({
            "type":"Polygon",
            "coordinates":[[[98.0,38.0],[99.0,38.0],[99.0,39.0],[98.0,39.0],[98.0,38.0]]]
        });
        assert!(!footprint_intersects_exact_catalogue_aoi(
            gap.as_object().unwrap(),
            &canonical
        ));
        assert!(footprint_intersects_exact_catalogue_aoi(
            inside.as_object().unwrap(),
            &canonical
        ));
    }

    #[test]
    fn prepared_exact_filter_preserves_positive_area_and_handles_dense_administrative_aoi_fast() {
        // Build a 6,000-vertex circular administrative-scale polygon centered at Hefei (117.3, 31.8)
        // with an interior hole [117.25..117.35, 31.75..31.85].
        let n = 6_000_usize;
        let mut ext = Vec::with_capacity(n + 1);
        for i in 0..n {
            let theta = (i as f64) * std::f64::consts::TAU / (n as f64);
            ext.push(json!([117.3 + 0.6 * theta.cos(), 31.8 + 0.6 * theta.sin()]));
        }
        ext.push(ext[0].clone());
        let hole = json!([
            [117.25, 31.75],
            [117.25, 31.85],
            [117.35, 31.85],
            [117.35, 31.75],
            [117.25, 31.75]
        ]);
        let aoi_json = json!({
            "type": "Polygon",
            "coordinates": [ext, hole]
        });
        let canonical = authoritative_catalogue_geometry(aoi_json.as_object().unwrap()).unwrap();
        let prepared = PreparedCatalogueAoi::new(&canonical).unwrap();

        // 1. Inside hole -> zero positive overlap (`false`)
        let in_hole = json!({
            "type": "Polygon",
            "coordinates": [[[117.27, 31.77],[117.33, 31.77],[117.33, 31.83],[117.27, 31.83],[117.27, 31.77]]]
        });
        assert!(!prepared.intersects_footprint(in_hole.as_object().unwrap()));

        // 2. Corner of bounding box [117.80..117.89, 32.30..32.39] outside circle (dx>=0.5, dy>=0.5 -> r>=0.707 > 0.6) -> `false`
        let bbox_corner_outside = json!({
            "type": "Polygon",
            "coordinates": [[[117.80, 32.30],[117.89, 32.30],[117.89, 32.39],[117.80, 32.39],[117.80, 32.30]]]
        });
        assert!(!prepared.intersects_footprint(bbox_corner_outside.as_object().unwrap()));

        // 3. Touching exact rightmost boundary vertex [117.9, 31.8] with zero interior overlap -> `false`
        let boundary_touch = json!({
            "type": "Polygon",
            "coordinates": [[[117.9, 31.7],[118.2, 31.7],[118.2, 31.9],[117.9, 31.9],[117.9, 31.7]]]
        });
        assert!(!prepared.intersects_footprint(boundary_touch.as_object().unwrap()));

        // 4. `+`-shaped strip crossing entire AOI without any footprint vertex inside AOI -> `true`
        let crossing_strip = json!({
            "type": "Polygon",
            "coordinates": [[[116.5, 31.90],[118.1, 31.90],[118.1, 31.95],[116.5, 31.95],[116.5, 31.90]]]
        });
        assert!(prepared.intersects_footprint(crossing_strip.as_object().unwrap()));

        // 5. Swath covering entire AOI -> `true`
        let covering_swath = json!({
            "type": "Polygon",
            "coordinates": [[[115.0, 30.0],[120.0, 30.0],[120.0, 34.0],[115.0, 34.0],[115.0, 30.0]]]
        });
        assert!(prepared.intersects_footprint(covering_swath.as_object().unwrap()));

        // 6. Latency guard: 80 mixed footprints against 6,000-vertex AOI must finish well under 200ms even in debug builds.
        let started = std::time::Instant::now();
        for i in 0..80 {
            let candidate = match i % 4 {
                0 => &in_hole,
                1 => &bbox_corner_outside,
                2 => &covering_swath,
                _ => &crossing_strip,
            };
            let _ = prepared.intersects_footprint(candidate.as_object().unwrap());
        }
        assert!(
            started.elapsed().as_millis() < 200,
            "80 exact checks against 6,000-vertex AOI took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn edge_straddling_scenes_with_partial_aoi_overlap_are_never_dropped() {
        let aoi = dense_disjoint_china_like_aoi_for_test();
        let prepared = PreparedCatalogueAoi::from_geojson(&aoi).expect("prepared AOI");

        // 1. Scene straddling the eastern border of western component (99.98..101.20°E):
        //    only ~1.6% of scene width overlaps AOI [97..100°E, 37..40°N], rest sticks outside.
        let tiny_border_sliver = json!({
            "type": "Polygon",
            "coordinates": [[[99.98, 38.20], [101.20, 38.20], [101.20, 39.20], [99.98, 39.20], [99.98, 38.20]]]
        });
        assert!(
            prepared.intersects_footprint(tiny_border_sliver.as_object().unwrap()),
            "edge-straddling scene with partial AOI overlap must be retained"
        );

        // 2. Scene crossing a straight border segment with ZERO footprint vertices inside AOI
        //    and ZERO AOI vertices inside footprint (dense step = 3/400 = 0.0075°, so use a
        //    narrow 0.003° coastal/border strip between two consecutive AOI vertices).
        let between_vertices_strip = json!({
            "type": "Polygon",
            "coordinates": [[[96.50, 37.002], [100.50, 37.002], [100.50, 37.005], [96.50, 37.005], [96.50, 37.002]]]
        });
        assert!(
            prepared.intersects_footprint(between_vertices_strip.as_object().unwrap()),
            "scene straddling straight AOI segment with zero vertices inside must be retained"
        );

        // 3. MultiPolygon footprint where sub-polygon 0 is disjoint outside AOI, and sub-polygon 1
        //    straddles the AOI corner [99.95..100.80, 39.95..40.80].
        let multipoly_edge_scene = json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[105.0, 42.0], [106.0, 42.0], [106.0, 43.0], [105.0, 43.0], [105.0, 42.0]]],
                [[[99.95, 39.95], [100.80, 39.95], [100.80, 40.80], [99.95, 40.80], [99.95, 39.95]]]
            ]
        });
        assert!(
            prepared.intersects_footprint(multipoly_edge_scene.as_object().unwrap()),
            "MultiPolygon footprint with any edge-straddling part must be retained"
        );

        // 4. Completely disjoint scene just outside the border [100.05..101.05] is still excluded.
        let strictly_outside = json!({
            "type": "Polygon",
            "coordinates": [[[100.05, 38.20], [101.05, 38.20], [101.05, 39.20], [100.05, 39.20], [100.05, 38.20]]]
        });
        assert!(
            !prepared.intersects_footprint(strictly_outside.as_object().unwrap()),
            "completely disjoint scene outside AOI must be excluded"
        );
    }
}
