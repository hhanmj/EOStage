use std::panic::{catch_unwind, AssertUnwindSafe};

use geo::{Area, BooleanOps, LineString, MultiPolygon, Polygon};
use insar_core::{
    align_polygonal_to_longitude_domain, align_polygonal_to_longitude_reference,
    validated_authoritative_polygonal_geojson, validated_polygonal_geojson, wrapped_aoi_envelope,
    BBox, SarProduct, SarSearchQuery,
};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Default)]
struct CoverageScore {
    ratio: f64,
    overlap: f64,
}

type EffectiveAoi = (MultiPolygon<f64>, Option<(f64, f64)>);

fn geometry_from_map(value: &Map<String, Value>) -> Option<MultiPolygon<f64>> {
    validated_polygonal_geojson(value)
}

fn authoritative_geometry_from_map(value: &Map<String, Value>) -> Option<MultiPolygon<f64>> {
    validated_authoritative_polygonal_geojson(value).map(|value| value.geometry)
}

fn bbox_geometry(bbox: &BBox) -> Option<MultiPolygon<f64>> {
    if ![bbox.west, bbox.south, bbox.east, bbox.north]
        .into_iter()
        .all(f64::is_finite)
        || bbox.west >= bbox.east
        || bbox.south >= bbox.north
    {
        return None;
    }
    Some(MultiPolygon(vec![Polygon::new(
        LineString::from(vec![
            (bbox.west, bbox.south),
            (bbox.east, bbox.south),
            (bbox.east, bbox.north),
            (bbox.west, bbox.north),
            (bbox.west, bbox.south),
        ]),
        Vec::new(),
    )]))
}

fn effective_aoi(query: &SarSearchQuery) -> Option<EffectiveAoi> {
    match query.aoi_geojson.as_ref() {
        Some(aoi_geojson) => {
            let aoi = authoritative_geometry_from_map(aoi_geojson)?;
            let envelope = wrapped_aoi_envelope(&aoi)?;
            let aligned = align_polygonal_to_longitude_domain(
                &aoi,
                envelope.display_west,
                envelope.display_east,
            )?;
            Some((
                aligned,
                Some((envelope.display_west, envelope.display_east)),
            ))
        }
        None => query
            .bbox
            .as_ref()
            .and_then(bbox_geometry)
            .map(|aoi| (aoi, None)),
    }
}

fn score(
    product: &SarProduct,
    aoi: &MultiPolygon<f64>,
    aoi_area: f64,
    longitude_domain: Option<(f64, f64)>,
) -> CoverageScore {
    let Some(footprint) = product
        .footprint_geojson
        .as_ref()
        .and_then(geometry_from_map)
    else {
        return CoverageScore::default();
    };
    let overlap = if let Some((display_west, display_east)) = longitude_domain {
        let Some(aligned) =
            align_polygonal_to_longitude_reference(&footprint, display_west, display_east)
        else {
            return CoverageScore::default();
        };
        let Some(overlap) = aligned.overlap_area(aoi) else {
            return CoverageScore::default();
        };
        overlap
    } else {
        let Ok(intersection) = catch_unwind(AssertUnwindSafe(|| footprint.intersection(aoi)))
        else {
            return CoverageScore::default();
        };
        intersection.unsigned_area()
    };
    let ratio = overlap / aoi_area;
    if !overlap.is_finite() || !ratio.is_finite() || overlap <= 0.0 || ratio <= 0.0 {
        return CoverageScore::default();
    }
    CoverageScore { ratio, overlap }
}

pub(super) fn prioritize_sar_products(
    products: &mut Vec<SarProduct>,
    query: &SarSearchQuery,
    requested_limit: u64,
) {
    if products.len() <= requested_limit as usize {
        return;
    }
    let Some((aoi, longitude_domain)) = effective_aoi(query) else {
        return;
    };
    let aoi_area = aoi.unsigned_area();
    if !aoi_area.is_finite() || aoi_area <= 0.0 {
        return;
    }

    let mut ranked = products
        .drain(..)
        .enumerate()
        .map(|(index, product)| {
            let score = score(&product, &aoi, aoi_area, longitude_domain);
            (index, product, score)
        })
        .collect::<Vec<_>>();
    if ranked.iter().all(|(_, _, score)| score.overlap == 0.0) {
        ranked.sort_by_key(|(index, _, _)| *index);
    } else {
        ranked.sort_by(|left, right| {
            right
                .2
                .ratio
                .total_cmp(&left.2.ratio)
                .then_with(|| right.2.overlap.total_cmp(&left.2.overlap))
                .then_with(|| left.0.cmp(&right.0))
                .then_with(|| left.1.product_key.cmp(&right.1.product_key))
        });
    }
    products.extend(ranked.into_iter().map(|(_, product, _)| product));
}
