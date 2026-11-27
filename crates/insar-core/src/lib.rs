mod asf;
mod burst_assets;
mod cache;
mod catalog_identity;
mod dataset_presentation;
mod error;
mod frozen_task_graph;
#[rustfmt::skip]
pub mod generated;
mod longitude_envelope;
mod network_policy;
mod nisar_subset;
mod nisar_subset_plan_v2;
mod operation_contract;
mod orbit;
mod path_plan;
pub mod polygonal_topology;
mod product_identity;
mod provider;
mod query;
mod registry;
mod resource_registry;
mod sar_cache;
mod sar_capabilities;
mod search_filter_schema;
mod sentinel1;
mod stac_scientific_metadata;
pub mod storage_guard;
mod task_state;

pub use asf::{decide_asf_cache, normalize_asf_features};
pub use burst_assets::{
    freeze_burst_task_items, frozen_burst_task_policy, validate_burst_task_items,
    validate_burst_task_items_for_policy, BurstAssetEvidenceInvalid, BurstAssetPolicy,
    ASF_BURST_ASSET_EXECUTION_POLICY_ID, ASF_BURST_COLLECTION_PROFILE_ID,
};
pub use cache::{asf_search_cache_key, cache_satisfies_limit, query_covers, ranges_cover};
pub use catalog_identity::{provider_product_key, stable_asset_key};
pub use dataset_presentation::{project_dataset_presentation, DatasetPresentation};
pub use error::{sanitize_external_error_message, validate_debug_context, SENSITIVE_CONTEXT_KEYS};
pub use frozen_task_graph::{
    validate_frozen_task_snapshot_v3, validate_frozen_task_snapshot_v3_graph,
    FrozenTaskSnapshotV3CompanionCoverageSubject, FrozenTaskSnapshotV3GraphError,
    FrozenTaskSnapshotV3GraphErrorKind, FrozenTaskSnapshotV3GraphIdentity,
    FrozenTaskSnapshotV3GraphLocation, FrozenTaskSnapshotV3GraphReference,
    FrozenTaskSnapshotV3OrbitIssue, FrozenTaskSnapshotV3TransferGraphIssue,
    FrozenTaskSnapshotV3ValidationError,
};
pub use generated::*;
pub use longitude_envelope::{
    align_polygonal_to_longitude_domain, align_polygonal_to_longitude_domain_render,
    align_polygonal_to_longitude_reference, wrapped_aoi_envelope, AlignedPolygonalCopies,
    WrappedAoiEnvelope, MAX_PROJECTED_RENDER_LONGITUDE,
};
pub use network_policy::{
    dns_name_matches, normalize_dns_name, ManualProxy, ProviderNetworkPolicy, ProxyPolicy,
};
pub use nisar_subset::*;
pub use nisar_subset_plan_v2::*;
pub use operation_contract::{
    validate_gacos_import_operation, validate_gacos_manual_request_operation,
    validate_gacos_result_operation,
};
pub use orbit::{match_orbits, parse_orbit_filenames};
pub use path_plan::{
    plan_desktop_orbit_target, plan_desktop_target, plan_sar_catalog_target,
    plan_sar_product_target, DesktopProductFamily, DesktopProductLayout,
};
pub use polygonal_topology::{
    canonicalized_authoritative_polygonal_geojson, polygonal_geojson_map,
    validated_authoritative_polygonal_bbox, validated_authoritative_polygonal_geojson,
    validated_polygonal_bbox, validated_polygonal_geojson, ValidatedAuthoritativePolygonalGeometry,
};
pub use product_identity::canonical_product_key;
pub use provider::{CatalogProvider, DataProvider, ProviderResult};
pub use query::{
    canonical_geojson_hash, canonical_query_key, normalize_bbox, normalize_date, normalize_filter,
    normalize_ranges, CanonicalAsfQuery, CanonicalBbox, InclusiveRange,
};
pub use registry::ProviderRegistry;
pub use resource_registry::*;
pub use sar_cache::{
    catalog_search_cache_key, decide_sar_search_cache, sar_search_cache_key, SarSearchCacheDecision,
};
pub use sar_capabilities::{
    asf_catalog_archive_window, asf_catalog_capabilities, AsfCatalogArchiveWindow,
};
pub use search_filter_schema::{
    EmptySelection, NumberOperator, SearchFilter, SearchFilterGroup, SearchFilterOption,
    SearchFilterSchema, SearchFilterSchemaCatalog, SearchFilterWorkbench,
    SearchFilterWorkbenchPanel, EMBEDDED_SEARCH_FILTER_SCHEMAS_JSON,
    SEARCH_FILTER_SCHEMA_CATALOG_ID,
};
pub use sentinel1::{parse_sentinel1_identifier, sentinel1_relative_orbit, ParsedSentinel1Name};
pub use stac_scientific_metadata::{
    project_stac_scientific_metadata, InsarPairScientificMetadata, SarStacScientificMetadata,
    StacScientificMetadata, INSAR_STAC_PROPOSAL, SAR_STAC_EXTENSION,
};
pub use task_state::{
    reduce_task_state, validate_task_state, TaskSelectionState, TaskStateCommand,
};

/// #22 B design item 6: one safety cap for every catalogue search path —
/// application layer defaults, source descriptors, provider loops and staging
/// checks. The IPC page size (250) is a transport bound, not this search cap.
pub const SEARCH_SAFETY_CAP: u64 = 10_000;

/// Clamp an explicit or configured finite search target to
/// [`SEARCH_SAFETY_CAP`]. `None` (the route default / exhaustive ALL) is kept;
/// it is never replaced by the ceiling here.
pub fn clamp_search_target(limit: Option<u64>) -> Option<u64> {
    limit.map(|limit| limit.min(SEARCH_SAFETY_CAP))
}

/// The one normalization every typed catalogue entry point applies before
/// validation, cursor fingerprinting, execution and statistics, so a request
/// for 10,001 is executed, fingerprinted and reported as 10,000 everywhere.
pub fn clamp_catalogue_search_query(query: &CatalogSearchQuery) -> CatalogSearchQuery {
    let mut clamped = query.clone();
    clamped.result_limit = clamp_search_target(query.result_limit);
    clamped
}

#[cfg(test)]
mod search_safety_cap_tests {
    use super::*;

    #[test]
    fn explicit_targets_clamp_to_the_safety_cap_and_defaults_stay_unset() {
        assert_eq!(clamp_search_target(None), None);
        assert_eq!(clamp_search_target(Some(250)), Some(250));
        assert_eq!(clamp_search_target(Some(10_000)), Some(10_000));
        assert_eq!(clamp_search_target(Some(10_001)), Some(10_000));
        assert_eq!(clamp_search_target(Some(u64::MAX)), Some(10_000));
        let query: CatalogSearchQuery = serde_json::from_value(serde_json::json!({
            "logical_dataset": "optical.fixture", "provider_id": "fixture",
            "native_collection_id": "fixture", "result_limit": 10_001
        }))
        .unwrap();
        assert!(query.validate().is_err());
        let clamped = clamp_catalogue_search_query(&query);
        assert_eq!(clamped.result_limit, Some(10_000));
        assert!(clamped.validate().is_ok());
    }
}
