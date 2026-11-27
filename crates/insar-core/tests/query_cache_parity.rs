use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use insar_core::{
    cache_satisfies_limit, canonical_geojson_hash, canonical_query_key, normalize_bbox,
    normalize_date, normalize_filter, normalize_ranges, query_covers, ranges_cover,
    CanonicalAsfQuery,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    filters: Vec<StringCase>,
    dates: Vec<StringCase>,
    ranges: Vec<RangeCase>,
    bboxes: Vec<BboxCase>,
    geojson: Vec<GeojsonCase>,
    queries: BTreeMap<String, QueryCase>,
    coverage: Vec<CoverageCase>,
    range_coverage: Vec<RangeCoverageCase>,
    limit_reuse: Vec<LimitCase>,
}

#[derive(Deserialize)]
struct StringCase {
    input: String,
    expected: String,
}

#[derive(Deserialize)]
struct RangeCase {
    input: String,
    expected: Vec<[u64; 2]>,
}

#[derive(Deserialize)]
struct BboxCase {
    input: BboxInput,
    expected: Value,
}

#[derive(Deserialize)]
struct BboxInput {
    west: f64,
    east: f64,
    south: f64,
    north: f64,
    crs: String,
}

#[derive(Deserialize)]
struct GeojsonCase {
    input: Value,
    expected_hash: String,
}

#[derive(Deserialize)]
struct QueryCase {
    value: CanonicalAsfQuery,
    key: String,
}

#[derive(Deserialize)]
struct CoverageCase {
    cached: String,
    requested: String,
    expected: bool,
}

#[derive(Deserialize)]
struct RangeCoverageCase {
    cached: String,
    requested: String,
    expected: bool,
}

#[derive(Deserialize)]
struct LimitCase {
    available: usize,
    complete: bool,
    requested: Option<usize>,
    expected: bool,
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/v3_pure_logic/query_cache.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn query_normalization_and_hashes_match_python_fixture() {
    let fixture = fixture();
    for case in fixture.filters {
        assert_eq!(normalize_filter(&case.input), case.expected);
    }
    for case in fixture.dates {
        assert_eq!(normalize_date(&case.input), case.expected);
    }
    for case in fixture.ranges {
        let actual = normalize_ranges(&case.input)
            .into_iter()
            .map(|range| [range.start, range.end])
            .collect::<Vec<_>>();
        assert_eq!(actual, case.expected);
    }
    for case in fixture.bboxes {
        let actual = normalize_bbox(
            case.input.west,
            case.input.east,
            case.input.south,
            case.input.north,
            &case.input.crs,
        );
        assert_eq!(serde_json::to_value(actual).unwrap(), case.expected);
    }
    for case in fixture.geojson {
        assert_eq!(canonical_geojson_hash(&case.input), case.expected_hash);
    }
    for case in fixture.queries.values() {
        assert_eq!(canonical_query_key(&case.value), case.key);
    }
}

#[test]
fn cache_coverage_and_limit_reuse_match_python_fixture() {
    let fixture = fixture();
    for case in fixture.coverage {
        assert_eq!(
            query_covers(
                &fixture.queries[&case.cached].value,
                &fixture.queries[&case.requested].value,
            ),
            case.expected,
        );
    }
    for case in fixture.range_coverage {
        assert_eq!(ranges_cover(&case.cached, &case.requested), case.expected,);
    }
    for case in fixture.limit_reuse {
        assert_eq!(
            cache_satisfies_limit(case.available, case.complete, case.requested),
            case.expected,
        );
    }
}
