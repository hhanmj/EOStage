//! Issue #3 R2 slice B4: route-neutral catalogue search intent.
//!
//! The caller names a product (a Registry resource), the reviewed search
//! filter schema revision it rendered, and predicates in that schema's
//! route-neutral query-key vocabulary. It never names a provider or a native
//! collection. Planning turns the intent into the provider-bound seed query
//! that the existing catalogue path already accepts: the product's first
//! reviewed Search route by Registry priority. The existing fallback planner
//! then derives the executable route list from that seed, so route choice,
//! eligibility and fallback stay Registry- and capability-owned.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use insar_core::{
    CapabilityKind, CatalogSearchQuery, ResourceDescriptor, ResourceRegistry,
    SearchFilterSchemaCatalog, StaticCapabilityLifecycle,
};

use crate::{
    catalogue_provider_error, ApplicationCatalogSearchAndFreezeRequest, ApplicationServiceError,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSearchIntent {
    pub resource_id: String,
    pub search_schema_id: String,
    pub schema_revision: u32,
    #[serde(default)]
    pub aoi_geojson: Option<Map<String, Value>>,
    pub result_limit: Option<u64>,
    #[serde(default)]
    pub filters: Map<String, Value>,
}

/// The intent form of [`ApplicationCatalogSearchAndFreezeRequest`]. Every
/// field other than `intent` keeps the same meaning and validation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationCatalogIntentSearchAndFreezeRequest {
    pub request_id: String,
    pub intent: CatalogSearchIntent,
    pub batch_id: String,
    pub created_at: String,
    pub aoi_name: String,
    #[serde(default)]
    pub aoi_geojson: Option<Map<String, Value>>,
    pub workbench_page_size: u64,
}

/// Plan an intent request into the provider-bound request the catalogue
/// path executes. Pure: reads only the embedded Registry and schema catalog.
pub fn plan_catalogue_intent_request(
    request: &ApplicationCatalogIntentSearchAndFreezeRequest,
) -> Result<ApplicationCatalogSearchAndFreezeRequest, ApplicationServiceError> {
    let registry = ResourceRegistry::embedded_ref()
        .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable)?;
    let schemas = SearchFilterSchemaCatalog::embedded_ref()
        .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable)?;
    let search = plan_catalogue_intent(registry, schemas, &request.intent)?;
    Ok(ApplicationCatalogSearchAndFreezeRequest {
        request_id: request.request_id.clone(),
        search,
        batch_id: request.batch_id.clone(),
        created_at: request.created_at.clone(),
        aoi_name: request.aoi_name.clone(),
        aoi_geojson: request.aoi_geojson.clone(),
        workbench_page_size: request.workbench_page_size,
    })
}

pub fn plan_catalogue_intent(
    registry: &ResourceRegistry,
    schemas: &SearchFilterSchemaCatalog,
    intent: &CatalogSearchIntent,
) -> Result<CatalogSearchQuery, ApplicationServiceError> {
    let invalid = |reason: String| ApplicationServiceError::InvalidRequest { reason };
    let resource = registry
        .resource(&intent.resource_id)
        .ok_or_else(|| invalid(format!("unknown catalogue resource {}", intent.resource_id)))?;
    let ResourceDescriptor::ScienceDataset { common, science } = resource else {
        return Err(invalid(format!(
            "resource {} is not a catalogue science dataset",
            intent.resource_id
        )));
    };
    if matches!(
        common.lifecycle,
        StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
    ) {
        return Err(invalid(format!(
            "resource {} is not active",
            intent.resource_id
        )));
    }
    if science.search_schema_id.as_deref() != Some(intent.search_schema_id.as_str()) {
        return Err(invalid(format!(
            "resource {} does not use search schema {}",
            intent.resource_id, intent.search_schema_id
        )));
    }
    let logical_dataset = science
        .catalogue_logical_dataset
        .as_deref()
        .ok_or_else(|| {
            invalid(format!(
                "resource {} has no catalogue logical dataset",
                intent.resource_id
            ))
        })?;
    let schema = schemas
        .schema(&intent.search_schema_id)
        .filter(|schema| schema.resource_id == intent.resource_id)
        .ok_or_else(|| {
            invalid(format!(
                "search schema {} is not reviewed for {}",
                intent.search_schema_id, intent.resource_id
            ))
        })?;
    if schema.schema_revision != intent.schema_revision {
        return Err(invalid(format!(
            "search schema {} revision {} is not current (current {})",
            schema.schema_id, intent.schema_revision, schema.schema_revision
        )));
    }
    // Review M1: values, not only keys, must match the reviewed schema; this
    // command is public and must not rely on each adapter to reject them.
    schema
        .validate_query_filters(&intent.filters)
        .map_err(invalid)?;

    // The seed only fixes product identity for the existing planner; the
    // executable route list and its order are derived from it downstream.
    let seed = registry
        .provider_route_candidates(&intent.resource_id, CapabilityKind::Search)
        .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_ROUTE_UNAVAILABLE",
                "当前没有可执行的数据来源",
                "no_reviewed_search_route",
            )
        })?;
    let native_collection_id = seed.route.native_collection_id.as_deref().ok_or_else(|| {
        catalogue_provider_error(
            "CATALOGUE_ROUTE_UNAVAILABLE",
            "已审核的数据来源缺少原生集合标识",
            "provider_native_collection",
        )
    })?;
    Ok(CatalogSearchQuery {
        logical_dataset: logical_dataset.to_owned(),
        provider_id: seed.route.provider_id.clone(),
        native_collection_id: native_collection_id.to_owned(),
        aoi_geojson: intent.aoi_geojson.clone(),
        cursor: None,
        result_limit: insar_core::clamp_search_target(intent.result_limit),
        filters: intent.filters.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn aoi() -> Map<String, Value> {
        json!({"type": "Polygon", "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]})
            .as_object()
            .unwrap()
            .clone()
    }

    fn intent(resource: &str, schema: &str, filters: Value) -> CatalogSearchIntent {
        CatalogSearchIntent {
            resource_id: resource.to_owned(),
            search_schema_id: schema.to_owned(),
            schema_revision: 1,
            aoi_geojson: Some(aoi()),
            result_limit: Some(100),
            filters: filters.as_object().unwrap().clone(),
        }
    }

    fn plan(intent: &CatalogSearchIntent) -> Result<CatalogSearchQuery, ApplicationServiceError> {
        plan_catalogue_intent(
            ResourceRegistry::embedded_ref().unwrap(),
            SearchFilterSchemaCatalog::embedded_ref().unwrap(),
            intent,
        )
    }

    fn reason(error: ApplicationServiceError) -> String {
        match error {
            ApplicationServiceError::InvalidRequest { reason } => reason,
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn sentinel2_intent_seeds_the_registry_first_search_route() {
        let query = plan(&intent(
            "science.sentinel-2.l2a.v1",
            "search.optical.sentinel-2.l2a.v1",
            json!({"product_type": "S2MSI2A", "spacecraft": ["sentinel_2a"], "cloud_cover_lte": 20.0}),
        ))
        .unwrap();
        // Same identity the Workbench previously hard-coded; the fallback
        // planner downstream still derives CDSE -> Microsoft from it.
        assert_eq!(query.logical_dataset, "optical.sentinel-2.l2a");
        assert_eq!(query.provider_id, "copernicus.dataspace.odata");
        assert_eq!(query.native_collection_id, "SENTINEL-2");
        assert_eq!(query.cursor, None);
        assert_eq!(query.result_limit, Some(100));
        assert_eq!(query.aoi_geojson, Some(aoi()));
        assert_eq!(query.filters.get("cloud_cover_lte"), Some(&json!(20.0)));
    }

    #[test]
    fn landsat_intent_seeds_microsoft_as_before() {
        let query = plan(&intent(
            "science.landsat.c2-l2.v1",
            "search.optical.landsat.c2-l2.v1",
            json!({"spacecraft": ["landsat_8", "landsat_9"], "processing_levels": ["l2sp", "l2sr"]}),
        ))
        .unwrap();
        assert_eq!(query.logical_dataset, "optical.landsat.c2.l2");
        assert_eq!(query.provider_id, "microsoft.planetary-computer");
        assert_eq!(query.native_collection_id, "landsat-c2-l2");
    }

    #[test]
    fn intents_are_checked_against_the_reviewed_schema() {
        let mut stale = intent(
            "science.landsat.c2-l2.v1",
            "search.optical.landsat.c2-l2.v1",
            json!({}),
        );
        stale.schema_revision = 2;
        assert!(reason(plan(&stale).unwrap_err()).contains("not current"));

        let unknown_key = intent(
            "science.landsat.c2-l2.v1",
            "search.optical.landsat.c2-l2.v1",
            json!({"eo:cloud_cover": 10}),
        );
        assert!(reason(plan(&unknown_key).unwrap_err()).contains("not declared"));

        let wrong_schema = intent(
            "science.landsat.c2-l2.v1",
            "search.optical.sentinel-2.l2a.v1",
            json!({}),
        );
        assert!(reason(plan(&wrong_schema).unwrap_err()).contains("does not use search schema"));

        let missing_constant = intent(
            "science.sentinel-2.l2a.v1",
            "search.optical.sentinel-2.l2a.v1",
            json!({}),
        );
        assert!(
            reason(plan(&missing_constant).unwrap_err()).contains("constant filter product_type")
        );

        let wrong_constant = intent(
            "science.sentinel-2.l2a.v1",
            "search.optical.sentinel-2.l2a.v1",
            json!({"product_type": "S2MSI1C"}),
        );
        assert!(reason(plan(&wrong_constant).unwrap_err()).contains("constant filter product_type"));

        // Review M1: values outside the reviewed schema never reach an adapter.
        for (filters, needle) in [
            (json!({"spacecraft": ["landsat_7"]}), "outside"),
            (json!({"cloud_cover_lte": 500}), "number from 0 to 100"),
            (json!({"cloud_cover_lte": "x"}), "number from 0 to 100"),
            (
                json!({"datetime": {"start": "2024-06-01T00:00:00Z"}}),
                "start and end",
            ),
        ] {
            let bad = intent("science.hls.l30.v2", "search.optical.hls.l30.v2", filters);
            assert!(reason(plan(&bad).unwrap_err()).contains(needle), "{needle}");
        }

        let not_science = intent(
            "dem.cop30.v1",
            "search.optical.sentinel-2.l2a.v1",
            json!({}),
        );
        assert!(reason(plan(&not_science).unwrap_err()).contains("not a catalogue science dataset"));

        let unknown = intent(
            "science.not-registered.v1",
            "search.optical.sentinel-2.l2a.v1",
            json!({}),
        );
        assert!(reason(plan(&unknown).unwrap_err()).contains("unknown catalogue resource"));

        // SAR products are not on the generic catalogue path (no catalogue
        // logical dataset), so they cannot be searched by intent.
        let sar = intent(
            "science.alos.palsar.v1",
            "search.asf.alos-palsar.v1",
            json!({}),
        );
        assert!(reason(plan(&sar).unwrap_err()).contains("no catalogue logical dataset"));

        // A catalogue product whose schema is not in the reviewed catalog.
        let mut catalog = SearchFilterSchemaCatalog::embedded_ref().unwrap().clone();
        catalog
            .schemas
            .retain(|schema| schema.resource_id != "science.landsat.c2-l2.v1");
        let unreviewed = intent(
            "science.landsat.c2-l2.v1",
            "search.optical.landsat.c2-l2.v1",
            json!({}),
        );
        let error = plan_catalogue_intent(
            ResourceRegistry::embedded_ref().unwrap(),
            &catalog,
            &unreviewed,
        )
        .unwrap_err();
        assert!(reason(error).contains("not reviewed"));
    }

    #[test]
    fn intent_request_keeps_every_non_search_field() {
        let request = ApplicationCatalogIntentSearchAndFreezeRequest {
            request_id: "catalogue-landsat-1".to_owned(),
            intent: intent(
                "science.landsat.c2-l2.v1",
                "search.optical.landsat.c2-l2.v1",
                json!({"spacecraft": ["landsat_9"], "processing_levels": ["l2sr"]}),
            ),
            batch_id: "catalogue-landsat-batch-1".to_owned(),
            created_at: "2026-10-19T12:00:00Z".to_owned(),
            aoi_name: "Fixture AOI".to_owned(),
            aoi_geojson: Some(aoi()),
            workbench_page_size: 50,
        };
        let planned = plan_catalogue_intent_request(&request).unwrap();
        assert_eq!(planned.request_id, request.request_id);
        assert_eq!(planned.batch_id, request.batch_id);
        assert_eq!(planned.created_at, request.created_at);
        assert_eq!(planned.aoi_name, request.aoi_name);
        assert_eq!(planned.aoi_geojson, request.aoi_geojson);
        assert_eq!(planned.workbench_page_size, 50);
        assert_eq!(planned.search.aoi_geojson, request.aoi_geojson);
        planned.validate().unwrap();
    }

    #[test]
    fn intent_wire_shape_rejects_provider_identity() {
        let with_provider = json!({
            "resource_id": "science.landsat.c2-l2.v1",
            "search_schema_id": "search.optical.landsat.c2-l2.v1",
            "schema_revision": 1,
            "aoi_geojson": null,
            "result_limit": 100,
            "filters": {},
            "provider_id": "microsoft.planetary-computer"
        });
        assert!(serde_json::from_value::<CatalogSearchIntent>(with_provider).is_err());
    }
}
