use insar_provider::boundary::{GlobalBoundaryProvider, NominatimBoundaryProvider};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

fn pace_public_request() {
    static LAST_REQUEST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    let mut last_request = LAST_REQUEST
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("public request pace lock");
    if let Some(last) = *last_request {
        let minimum_interval = Duration::from_millis(1_050);
        if let Some(remaining) = minimum_interval.checked_sub(last.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
    *last_request = Some(Instant::now());
}

#[test]
#[ignore = "explicit public Nominatim administrative-boundary acceptance"]
fn non_china_administrative_boundary_remains_bindable() {
    let provider = NominatimBoundaryProvider::production().expect("Nominatim provider initializes");
    pace_public_request();
    let candidates = provider
        .discover("California, United States", 1)
        .expect("global administrative boundary discovery");
    let candidate = candidates.first().expect("one global boundary candidate");
    assert_eq!(candidate.country_code.as_deref(), Some("US"));
    assert!(candidate.bbox[0] < candidate.bbox[2] && candidate.bbox[1] < candidate.bbox[3]);

    pace_public_request();
    let result = provider
        .resolve(&candidate.provider_object_id)
        .expect("selected global boundary resolution");
    assert_eq!(result.provider_object_id, candidate.provider_object_id);
    assert!(matches!(
        result.geometry_kind.as_str(),
        "polygon" | "multipolygon"
    ));
    assert_eq!(result.country_code.as_deref(), Some("US"));
    assert!(result.bbox[0] < result.bbox[2] && result.bbox[1] < result.bbox[3]);
}

#[test]
#[ignore = "explicit public Nominatim Taiwan administrative-boundary acceptance"]
fn taiwan_city_or_county_uses_osm_polygon_authority() {
    let provider = NominatimBoundaryProvider::production().expect("Nominatim provider initializes");
    pace_public_request();
    let results = provider
        .search_taiwan("花莲县", 3)
        .expect("Taiwan administrative boundary search");
    let result = results
        .iter()
        .find(|result| result.country_code.as_deref() == Some("TW"))
        .expect("one Taiwan boundary result");
    assert!(matches!(
        result.geometry_kind.as_str(),
        "polygon" | "multipolygon"
    ));
    assert!(result.bbox[0] < result.bbox[2] && result.bbox[1] < result.bbox[3]);
}

#[test]
#[ignore = "explicit public Nominatim Taiwan administrative-boundary acceptance"]
fn taiwan_nantou_selector_name_resolves_one_osm_polygon() {
    let provider = NominatimBoundaryProvider::production().expect("Nominatim provider initializes");
    pace_public_request();
    let results = provider
        .search_taiwan("南投县", 1)
        .expect("Nantou selector name resolves");
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(result.country_code.as_deref(), Some("TW"));
    assert!(matches!(
        result.geometry_kind.as_str(),
        "polygon" | "multipolygon"
    ));
    assert!(result.bbox[0] < result.bbox[2] && result.bbox[1] < result.bbox[3]);
}
