use insar_core::{
    catalog_search_cache_key, decide_sar_search_cache, sar_search_cache_key, CatalogSearchQuery,
    FrozenSearchBatch, SarSearchCacheDecision, SarSearchQuery, SnapshotItem,
};
use serde_json::{json, Map};

type QueryMutation = Box<dyn Fn(&mut SarSearchQuery)>;

fn query() -> SarSearchQuery {
    serde_json::from_value(json!({
        "provider": "ASF",
        "dataset": "SENTINEL-1",
        "platforms": ["Sentinel-1A"],
        "instrument": "C-SAR",
        "processing_levels": ["SLC"],
        "product_families": ["SLC"],
        "beam_modes": ["IW"],
        "polarizations": ["VV+VH"],
        "flight_direction": "ASCENDING",
        "look_direction": "RIGHT",
        "relative_orbit": 118,
        "frame": 1000,
        "asf_frame": "01000",
        "path": 118,
        "start": "2024-01-01T00:00:00Z",
        "end": "2024-02-01T00:00:00Z",
        "track_range": {"start": "118", "end": "118"},
        "frame_range": {"start": "01000", "end": "01000"},
        "max_results": 20,
        "burst": {
            "burst_id": "B001",
            "full_burst_id": "FULL-B001",
            "relative_burst_id": 1,
            "absolute_burst_id": 2,
            "subswath": "IW1",
            "burst_index": 3
        }
    }))
    .expect("valid SAR query")
}

fn item(id: &str, acquisition_datetime: Option<&str>) -> SnapshotItem {
    SnapshotItem {
        item_id: id.to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: id.to_owned(),
        acquisition_datetime: acquisition_datetime.map(ToOwned::to_owned),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn batch(query: &SarSearchQuery, acquisitions: &[Option<&str>]) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: "cached-batch".to_owned(),
        provider_id: "asf.catalog".to_owned(),
        created_at: "2026-08-18T00:00:00Z".to_owned(),
        aoi_name: "无 AOI".to_owned(),
        aoi_geojson: query.aoi_geojson.clone(),
        query: serde_json::to_value(query)
            .expect("query serialization")
            .as_object()
            .expect("query object")
            .clone(),
        search_stats: None,
        candidates: acquisitions
            .iter()
            .enumerate()
            .map(|(index, acquisition)| item(&format!("ITEM-{index}"), *acquisition))
            .collect(),
    }
}

fn catalog_query() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": "sar.nisar.l.rslc.provisional.v1",
        "provider_id": "asf.nasa.earthdata",
        "native_collection_id": "C2853145197-ASF",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[37.720070447767895, 10.85690570018474], [37.721296026021705, 10.85690570018474], [37.721296026021705, 10.857787104635449], [37.720070447767895, 10.85690570018474]]]
        },
        "cursor": null,
        "result_limit": 250,
        "filters": {
            "datetime": {"start":"2026-07-01T00:00:00Z", "end":"2026-08-01T00:00:00Z"},
            "production_configuration": "PR",
            "band": "L",
            "frequency_a_polarization": "HH"
        }
    }))
    .expect("valid typed catalogue query")
}

#[test]
fn typed_catalogue_cache_identity_covers_aoi_frequency_band_and_result_limit() {
    let original = catalog_query();
    let original_key = catalog_search_cache_key(&original).unwrap();

    let mut equivalent = original.clone();
    equivalent.aoi_geojson = Some(
        json!({
            "coordinates": [[[37.720070447767895, 10.85690570018474], [37.721296026021705, 10.85690570018474], [37.721296026021705, 10.857787104635449], [37.720070447767895, 10.85690570018474]]],
            "type": "Polygon"
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    assert_eq!(catalog_search_cache_key(&equivalent).unwrap(), original_key);

    type QueryMutation = Box<dyn Fn(&mut CatalogSearchQuery)>;
    let mutations: Vec<QueryMutation> = vec![
        Box::new(|query| query.provider_id = "other.provider".to_owned()),
        Box::new(|query| query.native_collection_id = "C_OTHER".to_owned()),
        Box::new(|query| query.logical_dataset = "sar.nisar.s.rslc.provisional.v1".to_owned()),
        Box::new(|query| query.result_limit = Some(251)),
        Box::new(|query| {
            query
                .filters
                .insert("frequency_a_polarization".to_owned(), json!("HH+HV"));
        }),
        Box::new(|query| {
            query.filters.insert("band".to_owned(), json!("S"));
        }),
        Box::new(|query| {
            query.filters.insert(
                "datetime".to_owned(),
                json!({"start":"2026-07-02T00:00:00Z", "end":"2026-08-01T00:00:00Z"}),
            );
        }),
        Box::new(|query| {
            query.aoi_geojson = Some(
                json!({
                    "type": "Polygon",
                    "coordinates": [[[38.0, 11.0], [38.1, 11.0], [38.1, 11.1], [38.0, 11.0]]]
                })
                .as_object()
                .unwrap()
                .clone(),
            );
        }),
    ];
    for mutation in mutations {
        let mut changed = original.clone();
        mutation(&mut changed);
        assert_ne!(catalog_search_cache_key(&changed).unwrap(), original_key);
    }
}

#[test]
fn cache_identity_covers_every_scientific_filter_and_normalizes_sets() {
    let original = query();
    let original_key = sar_search_cache_key(&original).expect("cache key");

    let mut reordered = original.clone();
    reordered.platforms = vec!["Sentinel-1B".to_owned(), "Sentinel-1A".to_owned()];
    let mut same_set = reordered.clone();
    same_set.platforms.reverse();
    assert_eq!(
        sar_search_cache_key(&reordered).unwrap(),
        sar_search_cache_key(&same_set).unwrap()
    );

    let mutations: Vec<QueryMutation> = vec![
        Box::new(|query| query.provider = "OTHER".to_owned()),
        Box::new(|query| query.dataset = "SLC-BURST".to_owned()),
        Box::new(|query| query.platforms = vec!["Sentinel-1B".to_owned()]),
        Box::new(|query| query.instrument = Some("OTHER".to_owned())),
        Box::new(|query| query.processing_levels = vec!["GRD_HD".to_owned()]),
        Box::new(|query| query.product_families = vec!["GRD".to_owned()]),
        Box::new(|query| query.beam_modes = vec!["EW".to_owned()]),
        Box::new(|query| query.polarizations = vec!["HH".to_owned()]),
        Box::new(|query| query.flight_direction = Some("DESCENDING".to_owned())),
        Box::new(|query| query.look_direction = Some("LEFT".to_owned())),
        Box::new(|query| query.relative_orbit = Some(119)),
        Box::new(|query| query.frame = Some(1001)),
        Box::new(|query| query.asf_frame = Some("01001".to_owned())),
        Box::new(|query| query.path = Some(119)),
        Box::new(|query| query.start = Some("2024-01-02T00:00:00Z".to_owned())),
        Box::new(|query| query.end = Some("2024-02-02T00:00:00Z".to_owned())),
        Box::new(|query| query.track_range.as_mut().unwrap().end = Some("119".to_owned())),
        Box::new(|query| query.frame_range.as_mut().unwrap().end = Some("01001".to_owned())),
        Box::new(|query| query.burst.as_mut().unwrap().burst_id = Some("B002".to_owned())),
    ];
    for mutation in mutations {
        let mut changed = original.clone();
        mutation(&mut changed);
        assert_ne!(sar_search_cache_key(&changed).unwrap(), original_key);
    }
}

#[test]
fn aoi_identity_is_canonical_and_target_specific() {
    let mut first = query();
    first.aoi_geojson = Some(
        json!({
            "type": "Polygon",
            "coordinates": [[[100, 30], [101, 30], [101, 31], [100, 30]]]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let mut equivalent = first.clone();
    equivalent.aoi_geojson = Some(
        json!({
            "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 30.0]]],
            "type": "Polygon"
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    assert_eq!(
        sar_search_cache_key(&first).unwrap(),
        sar_search_cache_key(&equivalent).unwrap()
    );
    equivalent.max_results = Some(21);
    assert_ne!(
        sar_search_cache_key(&first).unwrap(),
        sar_search_cache_key(&equivalent).unwrap()
    );
}

#[test]
fn decision_reuses_exact_results_and_only_increments_safe_no_aoi_order() {
    let cached_query = query();
    let cached = batch(
        &cached_query,
        &[
            Some("2024-02-03T00:00:00Z"),
            Some("2024-02-02T00:00:00Z"),
            Some("2024-02-01T00:00:00Z"),
        ],
    );
    let mut smaller = cached_query.clone();
    smaller.max_results = Some(2);
    assert_eq!(
        decide_sar_search_cache(&smaller, &cached, false).unwrap(),
        SarSearchCacheDecision::Reuse
    );
    let mut larger = cached_query.clone();
    larger.max_results = Some(5);
    assert_eq!(
        decide_sar_search_cache(&larger, &cached, false).unwrap(),
        SarSearchCacheDecision::Incremental {
            older_than: "2024-02-01T00:00:00Z".to_owned(),
            missing_results: 2,
        }
    );

    let mut aoi_query = cached_query.clone();
    aoi_query.aoi_geojson = Some(
        json!({
            "type": "Polygon",
            "coordinates": [[[100, 30], [101, 30], [101, 31], [100, 30]]]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let aoi_cached = batch(&aoi_query, &[Some("2024-02-03T00:00:00Z")]);
    let mut larger_aoi = aoi_query.clone();
    larger_aoi.max_results = Some(5);
    assert_eq!(
        decide_sar_search_cache(&larger_aoi, &aoi_cached, false).unwrap(),
        SarSearchCacheDecision::Miss
    );

    for unsafe_order in [
        vec![Some("2024-02-03T00:00:00Z"), None],
        vec![Some("2024-02-03T00:00:00Z"), Some("2024-02-03T00:00:00Z")],
        vec![Some("2024-02-02T00:00:00Z"), Some("2024-02-03T00:00:00Z")],
        vec![Some("not-a-date"), Some("2024-02-01T00:00:00Z")],
    ] {
        assert_eq!(
            decide_sar_search_cache(&larger, &batch(&cached_query, &unsafe_order), false).unwrap(),
            SarSearchCacheDecision::Miss
        );
    }
}

#[test]
fn zero_candidate_batch_with_unmapped_records_is_not_reused() {
    // Issue #14: a build that could not map Sentinel-1D bursts froze 44 fetched
    // records as 0 candidates; the cache then served that empty result for 24 h.
    let cached_query = query();
    let mut cached = batch(&cached_query, &[]);
    cached.search_stats = Some(
        serde_json::from_value(json!({
            "matched_count": 44,
            "fetched_record_count": 44,
            "mapped_record_count": 0,
            "provider_returned_count": 0,
            "frozen_candidate_count": 0,
            "truncated": false,
            "source": "ASF_CMR"
        }))
        .expect("valid stats"),
    );
    assert_eq!(
        decide_sar_search_cache(&cached_query, &cached, true).unwrap(),
        SarSearchCacheDecision::Miss
    );
    let mut unlimited = cached_query.clone();
    unlimited.max_results = None;
    assert_eq!(
        decide_sar_search_cache(&unlimited, &cached, true).unwrap(),
        SarSearchCacheDecision::Miss
    );
}

#[test]
fn zero_candidate_batch_with_every_record_mapped_is_still_reused() {
    let cached_query = query();
    let mut cached = batch(&cached_query, &[]);
    cached.search_stats = Some(
        serde_json::from_value(json!({
            "matched_count": 3,
            "fetched_record_count": 3,
            "mapped_record_count": 3,
            "provider_returned_count": 0,
            "frozen_candidate_count": 0,
            "truncated": false,
            "source": "ASF_CMR"
        }))
        .expect("valid stats"),
    );
    assert_eq!(
        decide_sar_search_cache(&cached_query, &cached, true).unwrap(),
        SarSearchCacheDecision::Reuse
    );
}

/// Identity version 2 (2026-10-26): reviewed STAC sources now ask for
/// newest-first on dated searches and cap in that order, so a working set
/// cached under version 1 was selected with different query semantics and
/// must miss instead of being reused.
#[test]
fn typed_catalogue_cache_key_changed_with_newest_first_query_semantics() {
    const VERSION_1_KEY: &str = "278cfc017c67e3e2d7ea624f47d100a99550ff62f0a667b34101fc8824fa6ea1";
    let key = catalog_search_cache_key(&catalog_query()).unwrap();
    assert_ne!(key, VERSION_1_KEY);
    assert_eq!(key.len(), 64);
    assert_eq!(catalog_search_cache_key(&catalog_query()).unwrap(), key);
}
