//! Synthetic administrative-boundary providers for the packaged offline smoke.
//!
//! This fixture is deliberately narrower than the production providers. It
//! exists only to exercise the real Application Service boundary path without
//! network access. Its polygon is synthetic and must never be presented as an
//! authoritative map or product acceptance evidence.

use insar_core::{ContractError, ProviderResult};
use insar_provider::boundary::{
    ChinaAdminOptions, ChinaBoundaryProvider, GlobalBoundaryProvider, ProviderBoundaryCandidate,
    ProviderBoundaryFeature,
};
use serde_json::{json, Map, Value};
use std::sync::Arc;

const NATIONAL_ADCODE: &str = "100000";
const DATAV_SOURCE: &str = "DataV areas_v3";

pub(crate) fn providers() -> (
    Arc<dyn GlobalBoundaryProvider>,
    Arc<dyn ChinaBoundaryProvider>,
) {
    (
        Arc::new(OfflineSmokeGlobalBoundaryProvider),
        Arc::new(OfflineSmokeChinaBoundaryProvider),
    )
}

#[derive(Clone, Copy, Debug)]
struct OfflineSmokeGlobalBoundaryProvider;

impl GlobalBoundaryProvider for OfflineSmokeGlobalBoundaryProvider {
    fn search(&self, _query: &str, _limit: usize) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        Err(unsupported("global.search"))
    }

    fn discover(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        Err(unsupported("global.discover"))
    }

    fn resolve(&self, _provider_object_id: &str) -> ProviderResult<ProviderBoundaryFeature> {
        Err(unsupported("global.resolve"))
    }

    fn discover_china(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        Err(unsupported("global.discover_china"))
    }
}

#[derive(Clone, Copy, Debug)]
struct OfflineSmokeChinaBoundaryProvider;

impl ChinaBoundaryProvider for OfflineSmokeChinaBoundaryProvider {
    fn options(&self, _province: &str, _city: &str) -> ProviderResult<ChinaAdminOptions> {
        Err(unsupported("china.options"))
    }

    fn resolve_exact_adcode(&self, adcode: &str) -> ProviderResult<ProviderBoundaryFeature> {
        if adcode == NATIONAL_ADCODE {
            Ok(national_feature())
        } else {
            Err(unsupported("china.resolve_exact_adcode"))
        }
    }

    fn search(
        &self,
        _province: &str,
        _city: &str,
        _district: &str,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        Err(unsupported("china.search"))
    }
}

fn national_feature() -> ProviderBoundaryFeature {
    ProviderBoundaryFeature {
        provider_object_id: NATIONAL_ADCODE.to_owned(),
        name: "中国".to_owned(),
        display_name: "中国（离线自检样例，非真实边界）".to_owned(),
        country_code: Some("CN".to_owned()),
        admin_level: Some("country".to_owned()),
        geometry: json!({
            "type": "Polygon",
            "coordinates": [[[73.5, 18.0], [135.1, 18.0], [135.1, 53.6], [73.5, 18.0]]]
        }),
        geometry_kind: "polygon".to_owned(),
        bbox: [73.5, 18.0, 135.1, 53.6],
        source: DATAV_SOURCE.to_owned(),
        attribution: None,
    }
}

fn unsupported(operation: &str) -> ContractError {
    let mut context = Map::new();
    context.insert("operation".to_owned(), Value::String(operation.to_owned()));
    ContractError::new(
        "BOUNDARY_PROVIDER_REQUEST_INVALID",
        "离线 smoke 行政区 fixture 仅支持全国 100000 的精确边界请求",
        context,
    )
    .expect("static offline smoke boundary error")
}

#[cfg(test)]
mod tests {
    use super::{providers, DATAV_SOURCE, NATIONAL_ADCODE};

    #[test]
    fn national_fixture_is_a_valid_synthetic_exact_boundary() {
        let (_, china) = providers();
        let result = china
            .resolve_exact_adcode(NATIONAL_ADCODE)
            .expect("offline national fixture");

        assert_eq!(result.provider_object_id, NATIONAL_ADCODE);
        assert_eq!(result.source, DATAV_SOURCE);
        assert_eq!(result.geometry_kind, "polygon");
        assert!(result.geometry.is_object());
        assert_eq!(result.admin_level.as_deref(), Some("country"));
        let normalized =
            insar_provider::boundary::normalize_polygonal_geometry(result.geometry.clone())
                .expect("synthetic polygon normalizes");
        assert_eq!(normalized.geometry_kind, "polygon");
        // Canonicalization expands the envelope slightly for numeric precision.
        for (actual, expected) in normalized.bbox.iter().zip(result.bbox) {
            assert!((actual - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn unsupported_fixture_inputs_and_global_paths_fail_closed() {
        let (global, china) = providers();
        assert_eq!(
            china.resolve_exact_adcode("420000").unwrap_err().code,
            "BOUNDARY_PROVIDER_REQUEST_INVALID"
        );
        assert_eq!(
            global.discover_china("中国", 8).unwrap_err().code,
            "BOUNDARY_PROVIDER_REQUEST_INVALID"
        );
    }
}
