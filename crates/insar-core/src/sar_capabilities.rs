use crate::{ProviderCapabilitiesResponse, ResourceDescriptor, ResourceRegistry};

const ASF_PROVIDER_ID: &str = "asf.catalog";
const ASF_SOURCE_VERSION: &str = "asf-search 12.2.2";
const ASF_SOURCE_PROVIDER_IDS: &[&str] = &["asf.catalog", "asf.nasa.earthdata"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AsfCatalogArchiveWindow {
    pub start: String,
    pub end: Option<String>,
}

/// Project the reviewed ASF compatibility contract from ResourceRegistry.
///
/// The compatibility response includes the legacy ASF adapter and the typed
/// Earthdata routes owned by that adapter. Provider-specific routes such as
/// Bhoonidhi remain in the registry's complete SAR source projection and are
/// exposed by their owning application route instead of being mislabeled as
/// ASF sources.
pub fn asf_catalog_capabilities() -> ProviderCapabilitiesResponse {
    let registry =
        ResourceRegistry::embedded_ref().expect("embedded Resource Registry must be valid");
    ProviderCapabilitiesResponse {
        provider_id: ASF_PROVIDER_ID.to_owned(),
        available: true,
        reason: None,
        datasets: registry.sar_dataset_capabilities(ASF_PROVIDER_ID),
        // Registry owns static source filters/actions; the service only overlays runtime
        // route and profile readiness on this ASF-provider compatibility projection.
        sources: registry.sar_source_capabilities_for_providers(ASF_SOURCE_PROVIDER_IDS),
        source_version: ASF_SOURCE_VERSION.to_owned(),
    }
}

/// Return the conservative acquisition-time envelope used when a catalogue
/// query leaves its dates blank. Historical bounds use complete calendar years
/// from ASF's published mission coverage so reprocessed edge acquisitions are
/// not silently excluded.
pub fn asf_catalog_archive_window(dataset: &str) -> Option<AsfCatalogArchiveWindow> {
    let registry = ResourceRegistry::embedded_ref().ok()?;
    let resource = registry.resource_for_provider_selector(ASF_PROVIDER_ID, dataset)?;
    let ResourceDescriptor::ScienceDataset { science, .. } = resource else {
        return None;
    };
    let range = science.acquisition_time_range.as_ref()?;
    Some(AsfCatalogArchiveWindow {
        start: range.start.clone(),
        end: range.end.clone(),
    })
}
