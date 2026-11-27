//! Read-only R2 presentation projection. The Resource Registry remains authoritative.
//! This module never chooses a provider or grants delivery readiness.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    CapabilityKind, ProductSpecificationVariant, ResourceDescriptor, ResourceRegistry,
    StaticCapabilityLifecycle,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetPresentation {
    pub resource_id: String,
    pub descriptor_revision: u32,
    pub display_name: String,
    pub search_schema_id: String,
    pub mission: String,
    pub platforms: Vec<String>,
    pub instrument: Option<String>,
    pub product_family: String,
    pub product_variants: Vec<ProductSpecificationVariant>,
    /// The declared, non-retired search routes only. Not runtime readiness.
    pub declared_search_route_ids: Vec<String>,
}

/// A projection, not a parallel dataset catalogue or an admission decision.
/// Unknown resources, non-science resources and missing search schemas are rejected.
pub fn project_dataset_presentation(
    registry: &ResourceRegistry,
    resource_id: &str,
) -> Result<DatasetPresentation, String> {
    let resource = registry
        .resource(resource_id)
        .ok_or_else(|| format!("unknown resource: {resource_id}"))?;
    let ResourceDescriptor::ScienceDataset { common, science } = resource else {
        return Err(format!("resource is not a science dataset: {resource_id}"));
    };
    if matches!(
        common.lifecycle,
        StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
    ) {
        return Err(format!("resource is not active: {resource_id}"));
    }
    let schema_id = science
        .search_schema_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| format!("resource has no search schema: {resource_id}"))?;

    let declared_route_ids: BTreeSet<String> = common
        .capabilities
        .iter()
        .filter(|capability| {
            capability.kind == CapabilityKind::Search
                && !matches!(
                    capability.lifecycle,
                    StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
                )
        })
        .flat_map(|capability| &capability.provider_route_ids)
        .filter(|route_id| {
            common.provider_routes.iter().any(|route| {
                &route.route_id == *route_id
                    && !matches!(
                        route.lifecycle,
                        StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
                    )
            })
        })
        .cloned()
        .collect();

    if declared_route_ids.is_empty() {
        return Err(format!(
            "resource has no declared search route: {resource_id}"
        ));
    }

    Ok(DatasetPresentation {
        resource_id: common.resource_id.clone(),
        descriptor_revision: common.descriptor_version,
        display_name: common.display_name.clone(),
        search_schema_id: schema_id.to_owned(),
        mission: science.mission.clone(),
        platforms: science.platforms.clone(),
        instrument: science.instrument.clone(),
        product_family: science.product_family.clone(),
        product_variants: science.product_variants.clone(),
        declared_search_route_ids: declared_route_ids.into_iter().collect(),
    })
}
