use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use chrono::{DateTime, SecondsFormat};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    ActionRule, DatasetCapabilities, FilterDescriptor, FilterPlatformLimit, FilterValueType,
    SarSourceCapability, SarSourceSearchKind,
};

pub const RESOURCE_REGISTRY_SCHEMA_VERSION: u32 = 1;
pub const RESOURCE_REGISTRY_ID: &str = "eostage.resource-registry.v1";
pub const EMBEDDED_RESOURCE_REGISTRY_JSON: &str =
    include_str!("../../../contracts/v3/resource-registry.v1.json");

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StaticCapabilityLifecycle {
    Reserved,
    Implemented,
    AdmissionControlled,
    Retained,
    Retired,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CredentialRequirement {
    None,
    Required,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Search,
    Preview,
    FullArchive,
    AssetSet,
    RemoteSubset,
    Resolve,
    Download,
    Plan,
    Import,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RegistryAssetRole {
    Primary,
    Metadata,
    Browse,
    Thumbnail,
    Auxiliary,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryModel {
    SingleArchive,
    MultiAsset,
    RangeSubset,
    ResolverFile,
    TiledCog,
    BboxApi,
    UserMediatedRequest,
    UserMediatedImport,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ValidationRule {
    ExpectedSize,
    Md5,
    Sha256,
    Sha512,
    MediaType,
    GeoTiffDecode,
    Hdf5Semantic,
    OrbitFilename,
    OrbitValidity,
    GacosProductPair,
    ProviderIdentity,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRequirements {
    pub build_capability: bool,
    pub runtime_readiness: bool,
    pub credential_readiness: bool,
    pub admission_evidence: bool,
    pub policy_permission: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderRouteDescriptor {
    pub route_id: String,
    pub provider_id: String,
    pub native_collection_id: Option<String>,
    pub implementation_binding_id: String,
    pub lifecycle: StaticCapabilityLifecycle,
    pub credential_requirement: CredentialRequirement,
    pub endpoint_class: String,
    pub priority: u32,
    #[serde(default)]
    pub coverage: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssetRoleRule {
    pub role: RegistryAssetRole,
    pub min_count: u32,
    pub max_count: Option<u32>,
    pub media_types: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssetSchemaDescriptor {
    pub asset_schema_id: String,
    pub roles: Vec<AssetRoleRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeliveryCapabilityDescriptor {
    pub delivery_profile_id: String,
    pub model: DeliveryModel,
    pub supports_range: bool,
    pub supports_resume: bool,
    pub atomic_promotion: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValidationProfileDescriptor {
    pub validation_profile_id: String,
    pub rules: Vec<ValidationRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LicenseProvenanceDescriptor {
    pub license_id: Option<String>,
    pub source_url: Option<String>,
    pub citation: Option<String>,
    pub provenance_binding_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDeclaration {
    pub capability_id: String,
    pub kind: CapabilityKind,
    pub lifecycle: StaticCapabilityLifecycle,
    pub implementation_binding_id: Option<String>,
    pub provider_route_ids: Vec<String>,
    pub asset_schema_id: Option<String>,
    pub delivery_profile_id: Option<String>,
    pub validation_profile_id: Option<String>,
    pub credential_requirement: CredentialRequirement,
    pub admission_policy_id: Option<String>,
    pub runtime_requirements: RuntimeRequirements,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceDescriptorCore {
    pub resource_id: String,
    pub display_name: String,
    pub descriptor_version: u32,
    pub lifecycle: StaticCapabilityLifecycle,
    pub implementation_binding_id: Option<String>,
    pub provider_routes: Vec<ProviderRouteDescriptor>,
    pub capabilities: Vec<CapabilityDeclaration>,
    pub asset_schemas: Vec<AssetSchemaDescriptor>,
    pub delivery_profiles: Vec<DeliveryCapabilityDescriptor>,
    pub validation_profiles: Vec<ValidationProfileDescriptor>,
    pub license_provenance: LicenseProvenanceDescriptor,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProductSpecificationVariant {
    pub variant_id: String,
    pub processing_level: String,
    pub product_type: String,
    pub specification_version: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcquisitionTimeRange {
    pub start: String,
    pub end: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegistryFilterValueType {
    Integer,
    PaddedString,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegistryFilterPlatformLimit {
    pub platform: String,
    pub minimum: u64,
    pub maximum: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegistryFilterDescriptor {
    pub supported: bool,
    pub parameter: Option<String>,
    pub label: String,
    pub value_type: RegistryFilterValueType,
    pub pattern: Option<String>,
    pub minimum: Option<u64>,
    pub maximum: Option<u64>,
    pub exact_width: Option<u64>,
    pub platform_limits: Vec<RegistryFilterPlatformLimit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegistrySearchAction {
    pub action: String,
    pub platforms: Vec<String>,
    pub product_families: Vec<String>,
    pub required_fields: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SarSearchProjectionKind {
    LegacyAsfSearch,
    TypedCatalogProfile,
}

impl From<&SarSearchProjectionKind> for SarSourceSearchKind {
    fn from(value: &SarSearchProjectionKind) -> Self {
        match value {
            SarSearchProjectionKind::LegacyAsfSearch => Self::LegacyAsfSearch,
            SarSearchProjectionKind::TypedCatalogProfile => Self::TypedCatalogProfile,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum RegistryScalarValue {
    String(String),
    Integer(i64),
    Boolean(bool),
}

impl RegistryScalarValue {
    fn as_json(&self) -> Value {
        match self {
            Self::String(value) => Value::String(value.clone()),
            Self::Integer(value) => Value::Number((*value).into()),
            Self::Boolean(value) => Value::Bool(*value),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SarSourceFilterSemantics {
    pub supported_keys: Vec<String>,
    pub fixed: BTreeMap<String, RegistryScalarValue>,
    pub options: BTreeMap<String, Vec<String>>,
}

impl SarSourceFilterSemantics {
    fn as_json_map(&self) -> Map<String, Value> {
        Map::from_iter([
            (
                "supported_keys".to_owned(),
                Value::Array(
                    self.supported_keys
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            ),
            (
                "fixed".to_owned(),
                Value::Object(Map::from_iter(
                    self.fixed
                        .iter()
                        .map(|(key, value)| (key.clone(), value.as_json())),
                )),
            ),
            (
                "options".to_owned(),
                Value::Object(Map::from_iter(self.options.iter().map(|(key, values)| {
                    (
                        key.clone(),
                        Value::Array(values.iter().cloned().map(Value::String).collect()),
                    )
                }))),
            ),
        ])
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SarSearchSemantics {
    pub provider_route_id: String,
    pub selector: String,
    pub projection_kind: SarSearchProjectionKind,
    pub projection_order: u32,
    pub logical_dataset: Option<String>,
    pub source_filters: Option<SarSourceFilterSemantics>,
    pub platforms: Vec<String>,
    pub instruments: Vec<String>,
    pub processing_levels: Vec<String>,
    pub product_families: Vec<String>,
    pub beam_modes: Vec<String>,
    pub polarizations: Vec<String>,
    pub supports_flight_direction: bool,
    pub supports_look_direction: bool,
    pub supports_relative_orbit: bool,
    pub supports_frame: bool,
    pub supports_path: bool,
    pub supports_burst: bool,
    pub frame_filter: Option<RegistryFilterDescriptor>,
    pub track_filter: Option<RegistryFilterDescriptor>,
    pub supported_actions: Vec<RegistrySearchAction>,
}

impl SarSearchSemantics {
    fn dataset_capabilities(&self, display_name: &str) -> DatasetCapabilities {
        DatasetCapabilities {
            dataset: self.selector.clone(),
            display_name: display_name.to_owned(),
            platforms: self.platforms.clone(),
            instruments: self.instruments.clone(),
            processing_levels: self.processing_levels.clone(),
            product_families: self.product_families.clone(),
            beam_modes: self.beam_modes.clone(),
            polarizations: self.polarizations.clone(),
            supports_flight_direction: self.supports_flight_direction,
            supports_look_direction: self.supports_look_direction,
            supports_relative_orbit: self.supports_relative_orbit,
            supports_frame: self.supports_frame,
            supports_path: self.supports_path,
            supports_burst: self.supports_burst,
            frame_filter: self.frame_filter.as_ref().map(Into::into),
            track_filter: self.track_filter.as_ref().map(Into::into),
            supported_actions: self.supported_actions.iter().map(Into::into).collect(),
        }
    }

    fn source_capability(
        &self,
        display_name: &str,
        route: &ProviderRouteDescriptor,
    ) -> SarSourceCapability {
        let typed = matches!(
            self.projection_kind,
            SarSearchProjectionKind::TypedCatalogProfile
        );
        let filters = if typed {
            self.source_filters
                .as_ref()
                .expect("validated typed SAR search has source filters")
                .as_json_map()
        } else {
            Map::from_iter([
                (
                    "legacy_dataset".to_owned(),
                    Value::String(self.selector.clone()),
                ),
                (
                    "dataset_capabilities".to_owned(),
                    serde_json::to_value(self.dataset_capabilities(display_name))
                        .expect("DatasetCapabilities is JSON serializable"),
                ),
            ])
        };
        SarSourceCapability {
            selector: self.selector.clone(),
            display_name: display_name.to_owned(),
            search_kind: (&self.projection_kind).into(),
            logical_dataset: self.logical_dataset.clone(),
            provider_id: Some(route.provider_id.clone()),
            native_collection_id: typed.then(|| route.native_collection_id.clone()).flatten(),
            collection_profile_id: typed.then(|| route.implementation_binding_id.clone()),
            available: true,
            disabled_reason: None,
            filters,
            supported_actions: self
                .supported_actions
                .iter()
                .map(|action| action.action.clone())
                .collect(),
        }
    }
}

impl From<&RegistryFilterDescriptor> for FilterDescriptor {
    fn from(value: &RegistryFilterDescriptor) -> Self {
        Self {
            supported: value.supported,
            parameter: value.parameter.clone(),
            label: value.label.clone(),
            value_type: match value.value_type {
                RegistryFilterValueType::Integer => FilterValueType::Integer,
                RegistryFilterValueType::PaddedString => FilterValueType::PaddedString,
            },
            pattern: value.pattern.clone(),
            minimum: value.minimum,
            maximum: value.maximum,
            exact_width: value.exact_width,
            platform_limits: value.platform_limits.iter().map(Into::into).collect(),
        }
    }
}

impl From<&RegistryFilterPlatformLimit> for FilterPlatformLimit {
    fn from(value: &RegistryFilterPlatformLimit) -> Self {
        Self {
            platform: value.platform.clone(),
            minimum: value.minimum,
            maximum: value.maximum,
        }
    }
}

impl From<&RegistrySearchAction> for ActionRule {
    fn from(value: &RegistrySearchAction) -> Self {
        Self {
            action: value.action.clone(),
            platforms: value.platforms.clone(),
            product_families: value.product_families.clone(),
            required_fields: value.required_fields.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScienceDatasetSemantics {
    pub search_schema_id: Option<String>,
    pub catalogue_logical_dataset: Option<String>,
    pub selection_schema_id: String,
    pub mission: String,
    pub platforms: Vec<String>,
    pub instrument: Option<String>,
    pub product_family: String,
    pub product_variants: Vec<ProductSpecificationVariant>,
    pub product_property_ids: Vec<String>,
    pub acquisition_time_range: Option<AcquisitionTimeRange>,
    pub sar_search: Option<SarSearchSemantics>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrbitValiditySemantics {
    ContainsAcquisitionInterval,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrbitCreationPreference {
    NewestCreation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OrbitResourceSemantics {
    pub supported_platforms: Vec<String>,
    pub orbit_product_types: Vec<String>,
    pub validity_semantics: OrbitValiditySemantics,
    pub creation_preference: OrbitCreationPreference,
    pub publication_preference: Vec<String>,
    pub matching_policy_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DemResourceSemantics {
    pub runtime_dataset_id: String,
    pub coverage: String,
    #[serde(deserialize_with = "deserialize_json_f64")]
    pub nominal_resolution: f64,
    pub resolution_unit: String,
    pub horizontal_crs: String,
    pub vertical_datum: String,
    pub tile_scheme: String,
    pub nodata_semantics: String,
}

fn deserialize_json_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let number = serde_json::Number::deserialize(deserializer)?;
    number
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| serde::de::Error::custom("finite JSON number required"))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AtmosphericResourceSemantics {
    pub date_binding: String,
    pub aoi_binding: String,
    pub max_dates_per_batch: u32,
    pub output_asset_family: String,
    pub request_semantics: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "resource_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceDescriptor {
    ScienceDataset {
        common: ResourceDescriptorCore,
        science: Box<ScienceDatasetSemantics>,
    },
    Orbit {
        common: ResourceDescriptorCore,
        orbit: OrbitResourceSemantics,
    },
    Dem {
        common: ResourceDescriptorCore,
        dem: DemResourceSemantics,
    },
    Atmospheric {
        common: ResourceDescriptorCore,
        atmospheric: AtmosphericResourceSemantics,
    },
}

impl ResourceDescriptor {
    pub fn common(&self) -> &ResourceDescriptorCore {
        match self {
            Self::ScienceDataset { common, .. }
            | Self::Orbit { common, .. }
            | Self::Dem { common, .. }
            | Self::Atmospheric { common, .. } => common,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceDependencyConsumer {
    pub resource_id: Option<String>,
    pub capability_id: Option<String>,
    pub planning_context_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "requirement_kind",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CompanionRequirement {
    Orbit {
        target_resource_id: String,
        required: bool,
        platform_equality: bool,
        validity_contains_acquisition: bool,
        preferred_product_types: Vec<String>,
        matching_policy_id: String,
    },
    DemCoverage {
        target_resource_ids: Vec<String>,
        required: bool,
        coverage_contains_aoi: bool,
        accepted_horizontal_crs: Vec<String>,
        accepted_vertical_datums: Vec<String>,
        selection_policy_id: String,
    },
    AtmosphericDateAoi {
        target_resource_id: String,
        required: bool,
        match_acquisition_dates: bool,
        bind_processing_aoi: bool,
        selection_policy_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceDependency {
    pub dependency_id: String,
    pub consumer: ResourceDependencyConsumer,
    pub requirement: CompanionRequirement,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceRegistry {
    pub schema_version: u32,
    pub registry_id: String,
    pub review_version: String,
    pub resources: Vec<ResourceDescriptor>,
    pub dependencies: Vec<ResourceDependency>,
}

#[derive(Clone, Copy, Debug)]
pub struct SarProviderSelectorBinding<'a> {
    pub resource: &'a ResourceDescriptor,
    pub science: &'a ScienceDatasetSemantics,
    pub search: &'a SarSearchSemantics,
    pub route: &'a ProviderRouteDescriptor,
    pub capability: &'a CapabilityDeclaration,
}

#[derive(Clone, Copy, Debug)]
pub struct ProviderRouteCandidate<'a> {
    pub resource: &'a ResourceDescriptor,
    pub capability: &'a CapabilityDeclaration,
    pub route: &'a ProviderRouteDescriptor,
}

impl ResourceRegistry {
    pub fn embedded() -> Result<Self, String> {
        Self::embedded_ref().cloned()
    }

    pub fn embedded_ref() -> Result<&'static Self, String> {
        static REGISTRY: OnceLock<Result<ResourceRegistry, String>> = OnceLock::new();
        match REGISTRY.get_or_init(|| {
            let registry: Self = serde_json::from_str(EMBEDDED_RESOURCE_REGISTRY_JSON)
                .map_err(|error| format!("resource registry JSON is invalid: {error}"))?;
            registry.validate()?;
            Ok(registry)
        }) {
            Ok(registry) => Ok(registry),
            Err(error) => Err(error.clone()),
        }
    }

    pub fn resource(&self, resource_id: &str) -> Option<&ResourceDescriptor> {
        self.resources
            .iter()
            .find(|resource| resource.common().resource_id == resource_id)
    }

    pub fn capability(&self, capability_id: &str) -> Option<&CapabilityDeclaration> {
        self.resources
            .iter()
            .flat_map(|resource| &resource.common().capabilities)
            .find(|capability| capability.capability_id == capability_id)
    }

    /// Resolve one catalogue science product from an exact reviewed source key.
    ///
    /// The logical dataset is explicit registry data; it is never inferred from
    /// resource ids, schema ids, provider names, or collection-name patterns.
    /// Only a route that is actually referenced by a non-retired Search
    /// capability can establish the binding.
    pub fn catalogue_search_resource_for_source(
        &self,
        logical_dataset: &str,
        provider_id: &str,
        native_collection_id: &str,
    ) -> Result<Option<&ResourceDescriptor>, String> {
        let mut matches = self.resources.iter().filter(|resource| {
            let ResourceDescriptor::ScienceDataset { common, science } = resource else {
                return false;
            };
            if science.catalogue_logical_dataset.as_deref() != Some(logical_dataset) {
                return false;
            }
            common.provider_routes.iter().any(|route| {
                route.provider_id == provider_id
                    && route.native_collection_id.as_deref() == Some(native_collection_id)
                    && !matches!(
                        route.lifecycle,
                        StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
                    )
                    && common.capabilities.iter().any(|capability| {
                        capability.kind == CapabilityKind::Search
                            && !matches!(
                                capability.lifecycle,
                                StaticCapabilityLifecycle::Reserved
                                    | StaticCapabilityLifecycle::Retired
                            )
                            && capability
                                .provider_route_ids
                                .iter()
                                .any(|route_id| route_id == &route.route_id)
                    })
            })
        });
        let resource = matches.next();
        if matches.next().is_some() {
            return Err(format!(
                "ambiguous catalogue source binding: {logical_dataset} / {provider_id} / {native_collection_id}"
            ));
        }
        Ok(resource)
    }

    /// Build one deterministic, read-only provider candidate list for a product
    /// operation. Product identity stays at the Resource layer; provider routes
    /// are ordered only after the capability binding has proved that the route
    /// is allowed to serve that operation.
    pub fn provider_route_candidates(
        &self,
        resource_id: &str,
        capability_kind: CapabilityKind,
    ) -> Result<Vec<ProviderRouteCandidate<'_>>, String> {
        let resource = self
            .resource(resource_id)
            .ok_or_else(|| format!("unknown resource_id: {resource_id}"))?;
        let common = resource.common();
        let mut candidates = Vec::new();

        for capability in &common.capabilities {
            if capability.kind != capability_kind
                || matches!(
                    capability.lifecycle,
                    StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
                )
            {
                continue;
            }
            for route_id in &capability.provider_route_ids {
                let route = common
                    .provider_routes
                    .iter()
                    .find(|route| route.route_id == *route_id)
                    .ok_or_else(|| {
                        format!(
                            "capability {} references unknown provider route {}",
                            capability.capability_id, route_id
                        )
                    })?;
                if matches!(
                    route.lifecycle,
                    StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
                ) {
                    continue;
                }
                candidates.push(ProviderRouteCandidate {
                    resource,
                    capability,
                    route,
                });
            }
        }

        candidates.sort_by(|left, right| {
            left.route
                .priority
                .cmp(&right.route.priority)
                .then_with(|| left.route.route_id.cmp(&right.route.route_id))
                .then_with(|| {
                    left.capability
                        .capability_id
                        .cmp(&right.capability.capability_id)
                })
        });
        if let Some(window) = candidates.windows(2).find(|window| {
            window[0].route.priority == window[1].route.priority
                && window[0].route.route_id != window[1].route.route_id
        }) {
            return Err(format!(
                "ambiguous provider route priority {} for resource {} capability kind {:?}: {} and {}",
                window[0].route.priority,
                resource_id,
                capability_kind,
                window[0].route.route_id,
                window[1].route.route_id
            ));
        }
        Ok(candidates)
    }

    /// Select the first executable route from the reviewed candidate order.
    /// Current runtime observations are provided by the caller, while the
    /// capability declaration remains the authority for every readiness gate.
    pub fn select_provider_route_candidate<F>(
        &self,
        resource_id: &str,
        capability_kind: CapabilityKind,
        mut effective_inputs: F,
    ) -> Result<Option<ProviderRouteCandidate<'_>>, String>
    where
        F: FnMut(&CapabilityDeclaration, &ProviderRouteDescriptor) -> EffectiveCapabilityInputs,
    {
        Ok(self
            .provider_route_candidates(resource_id, capability_kind)?
            .into_iter()
            .find(|candidate| {
                project_effective_capability(
                    candidate.capability,
                    effective_inputs(candidate.capability, candidate.route),
                )
                .state
                    == EffectiveCapabilityState::Executable
            }))
    }

    pub fn resource_for_provider_selector(
        &self,
        provider_id: &str,
        selector: &str,
    ) -> Option<&ResourceDescriptor> {
        self.provider_selector_binding(provider_id, selector)
            .map(|binding| binding.resource)
    }

    pub fn provider_selector_binding(
        &self,
        provider_id: &str,
        selector: &str,
    ) -> Option<SarProviderSelectorBinding<'_>> {
        self.sar_provider_selector_bindings().find(|binding| {
            binding.route.provider_id == provider_id
                && binding
                    .search
                    .selector
                    .eq_ignore_ascii_case(selector.trim())
        })
    }

    pub fn sar_dataset_capabilities(&self, provider_id: &str) -> Vec<DatasetCapabilities> {
        let mut bindings = self
            .sar_provider_selector_bindings()
            .filter(|binding| {
                binding.route.provider_id == provider_id
                    && matches!(
                        binding.search.projection_kind,
                        SarSearchProjectionKind::LegacyAsfSearch
                    )
            })
            .collect::<Vec<_>>();
        bindings.sort_by_key(|binding| binding.search.projection_order);
        bindings
            .into_iter()
            .map(|binding| {
                binding
                    .search
                    .dataset_capabilities(&binding.resource.common().display_name)
            })
            .collect()
    }

    pub fn sar_source_capabilities(&self) -> Vec<SarSourceCapability> {
        self.sar_source_capabilities_for_providers(&[])
    }

    /// Project every implemented typed and legacy SAR source in registry order.
    ///
    /// The unfiltered projection is the authority used by consumers that expose
    /// the complete SAR source catalogue, including provider-specific typed
    /// routes such as Bhoonidhi S-RSLC.
    pub fn sar_source_capabilities_for_providers(
        &self,
        provider_ids: &[&str],
    ) -> Vec<SarSourceCapability> {
        let mut bindings = self
            .sar_provider_selector_bindings()
            .filter(|binding| {
                provider_ids.is_empty()
                    || provider_ids
                        .iter()
                        .any(|provider_id| *provider_id == binding.route.provider_id)
            })
            .collect::<Vec<_>>();
        bindings.sort_by_key(|binding| binding.search.projection_order);
        bindings
            .into_iter()
            .map(|binding| {
                binding
                    .search
                    .source_capability(&binding.resource.common().display_name, binding.route)
            })
            .collect()
    }

    fn sar_provider_selector_bindings(
        &self,
    ) -> impl Iterator<Item = SarProviderSelectorBinding<'_>> {
        self.resources.iter().filter_map(|resource| {
            let ResourceDescriptor::ScienceDataset { common, science } = resource else {
                return None;
            };
            if common.lifecycle != StaticCapabilityLifecycle::Implemented {
                return None;
            }
            let search = science.sar_search.as_ref()?;
            let route = common.provider_routes.iter().find(|route| {
                route.route_id == search.provider_route_id
                    && route.lifecycle == StaticCapabilityLifecycle::Implemented
            })?;
            let capability = common.capabilities.iter().find(|capability| {
                capability.kind == CapabilityKind::Search
                    && capability.lifecycle == StaticCapabilityLifecycle::Implemented
                    && capability
                        .provider_route_ids
                        .iter()
                        .any(|route_id| route_id == &route.route_id)
            })?;
            Some(SarProviderSelectorBinding {
                resource,
                science,
                search,
                route,
                capability,
            })
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != RESOURCE_REGISTRY_SCHEMA_VERSION {
            return Err("resource registry schema_version must equal 1".to_string());
        }
        if self.registry_id != RESOURCE_REGISTRY_ID {
            return Err("resource registry identity mismatch".to_string());
        }
        validate_id(&self.review_version, "review_version")?;
        if self.resources.is_empty() {
            return Err("resource registry must contain resources".to_string());
        }

        let mut resource_ids = HashSet::new();
        let mut capability_ids = HashSet::new();
        let mut route_ids = HashSet::new();
        let mut asset_schema_ids = HashSet::new();
        let mut delivery_profile_ids = HashSet::new();
        let mut validation_profile_ids = HashSet::new();
        let mut dem_runtime_dataset_ids = HashSet::new();
        let mut provider_selectors = HashSet::new();
        let mut sar_projection_orders = HashSet::new();
        let mut catalogue_logical_datasets = HashSet::new();

        for resource in &self.resources {
            let common = resource.common();
            validate_id(&common.resource_id, "resource_id")?;
            if let Some(binding_id) = &common.implementation_binding_id {
                validate_id(binding_id, "implementation_binding_id")?;
            }
            if !resource_ids.insert(common.resource_id.as_str()) {
                return Err(format!("duplicate resource_id: {}", common.resource_id));
            }
            if common.display_name.trim().is_empty() || common.descriptor_version == 0 {
                return Err(format!(
                    "resource {} has invalid display name or descriptor version",
                    common.resource_id
                ));
            }

            let mut local_routes = HashSet::new();
            for route in &common.provider_routes {
                validate_id(&route.route_id, "route_id")?;
                validate_id(&route.provider_id, "provider_id")?;
                validate_id(
                    &route.implementation_binding_id,
                    "implementation_binding_id",
                )?;
                if route.endpoint_class.trim().is_empty() || route.priority == 0 {
                    return Err(format!("route {} is incomplete", route.route_id));
                }
                if route
                    .native_collection_id
                    .as_deref()
                    .is_some_and(str::is_empty)
                    || route.coverage.as_deref().is_some_and(str::is_empty)
                {
                    return Err(format!(
                        "route {} has an empty optional field",
                        route.route_id
                    ));
                }
                if !route_ids.insert(route.route_id.as_str()) {
                    return Err(format!("duplicate route_id: {}", route.route_id));
                }
                local_routes.insert(route.route_id.as_str());
            }

            let mut local_asset_schemas = HashSet::new();
            for schema in &common.asset_schemas {
                validate_id(&schema.asset_schema_id, "asset_schema_id")?;
                if !asset_schema_ids.insert(schema.asset_schema_id.as_str()) {
                    return Err(format!(
                        "duplicate asset_schema_id: {}",
                        schema.asset_schema_id
                    ));
                }
                local_asset_schemas.insert(schema.asset_schema_id.as_str());
                if schema.roles.is_empty() {
                    return Err(format!("asset schema {} is empty", schema.asset_schema_id));
                }
                let mut roles = HashSet::new();
                for role in &schema.roles {
                    if role
                        .max_count
                        .is_some_and(|maximum| maximum < role.min_count)
                        || !roles.insert(&role.role)
                    {
                        return Err(format!(
                            "asset schema {} has invalid cardinality or duplicate role",
                            schema.asset_schema_id
                        ));
                    }
                }
            }

            let mut local_delivery_profiles = HashSet::new();
            for delivery in &common.delivery_profiles {
                validate_id(&delivery.delivery_profile_id, "delivery_profile_id")?;
                if delivery.supports_resume && !delivery.supports_range {
                    return Err(format!(
                        "delivery {} cannot resume without Range support",
                        delivery.delivery_profile_id
                    ));
                }
                if !delivery_profile_ids.insert(delivery.delivery_profile_id.as_str()) {
                    return Err(format!(
                        "duplicate delivery_profile_id: {}",
                        delivery.delivery_profile_id
                    ));
                }
                local_delivery_profiles.insert(delivery.delivery_profile_id.as_str());
            }

            let mut local_validation_profiles = HashSet::new();
            for validation in &common.validation_profiles {
                validate_id(&validation.validation_profile_id, "validation_profile_id")?;
                if validation.rules.is_empty() {
                    return Err(format!(
                        "validation profile {} is empty",
                        validation.validation_profile_id
                    ));
                }
                if !validation_profile_ids.insert(validation.validation_profile_id.as_str()) {
                    return Err(format!(
                        "duplicate validation_profile_id: {}",
                        validation.validation_profile_id
                    ));
                }
                local_validation_profiles.insert(validation.validation_profile_id.as_str());
            }

            for capability in &common.capabilities {
                validate_id(&capability.capability_id, "capability_id")?;
                if let Some(binding_id) = &capability.implementation_binding_id {
                    validate_id(binding_id, "implementation_binding_id")?;
                }
                if let Some(policy_id) = &capability.admission_policy_id {
                    validate_id(policy_id, "admission_policy_id")?;
                }
                if !capability_ids.insert(capability.capability_id.as_str()) {
                    return Err(format!(
                        "duplicate capability_id: {}",
                        capability.capability_id
                    ));
                }
                if capability
                    .provider_route_ids
                    .iter()
                    .any(|route_id| !local_routes.contains(route_id.as_str()))
                {
                    return Err(format!(
                        "capability {} references a foreign route",
                        capability.capability_id
                    ));
                }
                if !matches!(
                    capability.lifecycle,
                    StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
                ) && capability.provider_route_ids.is_empty()
                {
                    return Err(format!(
                        "capability {} has no provider route",
                        capability.capability_id
                    ));
                }
                if capability.provider_route_ids.iter().any(|route_id| {
                    common
                        .provider_routes
                        .iter()
                        .find(|route| route.route_id == *route_id)
                        .is_some_and(|route| {
                            route.credential_requirement != capability.credential_requirement
                        })
                }) {
                    return Err(format!(
                        "capability {} credential requirement disagrees with its route",
                        capability.capability_id
                    ));
                }
                if capability
                    .asset_schema_id
                    .as_deref()
                    .is_some_and(|identity| !local_asset_schemas.contains(identity))
                    || capability
                        .delivery_profile_id
                        .as_deref()
                        .is_some_and(|identity| !local_delivery_profiles.contains(identity))
                    || capability
                        .validation_profile_id
                        .as_deref()
                        .is_some_and(|identity| !local_validation_profiles.contains(identity))
                {
                    return Err(format!(
                        "capability {} references a foreign profile",
                        capability.capability_id
                    ));
                }
                if capability.runtime_requirements.admission_evidence
                    != capability.admission_policy_id.is_some()
                {
                    return Err(format!(
                        "capability {} admission requirement and policy disagree",
                        capability.capability_id
                    ));
                }
                if capability.runtime_requirements.credential_readiness
                    != matches!(
                        capability.credential_requirement,
                        CredentialRequirement::Required
                    )
                {
                    return Err(format!(
                        "capability {} credential requirement disagrees",
                        capability.capability_id
                    ));
                }
            }

            if let ResourceDescriptor::Dem { dem, .. } = resource {
                if !dem_runtime_dataset_ids.insert(dem.runtime_dataset_id.as_str()) {
                    return Err(format!(
                        "duplicate DEM runtime_dataset_id: {}",
                        dem.runtime_dataset_id
                    ));
                }
            }
            if let ResourceDescriptor::ScienceDataset { science, .. } = resource {
                if let Some(logical_dataset) = science.catalogue_logical_dataset.as_deref() {
                    validate_id(logical_dataset, "catalogue_logical_dataset")?;
                    if science.search_schema_id.is_none()
                        || !catalogue_logical_datasets.insert(logical_dataset)
                    {
                        return Err(format!(
                            "science resource {} has an invalid or duplicate catalogue logical dataset",
                            common.resource_id
                        ));
                    }
                    let has_search_route = common.capabilities.iter().any(|capability| {
                        capability.kind == CapabilityKind::Search
                            && !matches!(
                                capability.lifecycle,
                                StaticCapabilityLifecycle::Reserved
                                    | StaticCapabilityLifecycle::Retired
                            )
                            && capability.provider_route_ids.iter().any(|route_id| {
                                common.provider_routes.iter().any(|route| {
                                    route.route_id == *route_id
                                        && route.native_collection_id.is_some()
                                        && !matches!(
                                            route.lifecycle,
                                            StaticCapabilityLifecycle::Reserved
                                                | StaticCapabilityLifecycle::Retired
                                        )
                                })
                            })
                    });
                    if !has_search_route {
                        return Err(format!(
                            "science resource {} has a catalogue logical dataset without a Search route",
                            common.resource_id
                        ));
                    }
                }
                if let Some(search) = &science.sar_search {
                    let route = common
                        .provider_routes
                        .iter()
                        .find(|route| route.route_id == search.provider_route_id)
                        .ok_or_else(|| {
                            format!(
                                "SAR selector {} references an unknown provider route",
                                search.selector
                            )
                        })?;
                    let selector_key = format!(
                        "{}::{}",
                        route.provider_id,
                        search.selector.trim().to_ascii_uppercase()
                    );
                    if !provider_selectors.insert(selector_key) {
                        return Err(format!(
                            "duplicate provider SAR selector: {}",
                            search.selector
                        ));
                    }
                    if search.projection_order == 0
                        || !sar_projection_orders.insert(search.projection_order)
                    {
                        return Err(format!(
                            "duplicate or zero SAR projection order: {}",
                            search.projection_order
                        ));
                    }
                }
            }
            validate_typed_semantics(resource)?;
        }

        let mut dependency_ids = HashSet::new();
        for dependency in &self.dependencies {
            validate_id(&dependency.dependency_id, "dependency_id")?;
            if !dependency_ids.insert(dependency.dependency_id.as_str()) {
                return Err(format!(
                    "duplicate dependency_id: {}",
                    dependency.dependency_id
                ));
            }
            if dependency.consumer.resource_id.is_none()
                && dependency.consumer.capability_id.is_none()
                && dependency.consumer.planning_context_id.is_none()
            {
                return Err(format!(
                    "dependency {} has no consumer",
                    dependency.dependency_id
                ));
            }
            if dependency
                .consumer
                .resource_id
                .as_deref()
                .is_some_and(|identity| !resource_ids.contains(identity))
                || dependency
                    .consumer
                    .capability_id
                    .as_deref()
                    .is_some_and(|identity| !capability_ids.contains(identity))
            {
                return Err(format!(
                    "dependency {} references an unknown consumer",
                    dependency.dependency_id
                ));
            }
            if let (Some(resource_id), Some(capability_id)) = (
                dependency.consumer.resource_id.as_deref(),
                dependency.consumer.capability_id.as_deref(),
            ) {
                let capability_belongs_to_resource =
                    self.resource(resource_id).is_some_and(|resource| {
                        resource
                            .common()
                            .capabilities
                            .iter()
                            .any(|capability| capability.capability_id == capability_id)
                    });
                if !capability_belongs_to_resource {
                    return Err(format!(
                        "dependency {} consumer capability belongs to another resource",
                        dependency.dependency_id
                    ));
                }
            }
            if let Some(planning_context_id) = &dependency.consumer.planning_context_id {
                validate_id(planning_context_id, "planning_context_id")?;
            }
            validate_dependency_target(self, dependency)?;
        }
        Ok(())
    }
}

fn validate_typed_semantics(resource: &ResourceDescriptor) -> Result<(), String> {
    match resource {
        ResourceDescriptor::ScienceDataset { common, science } => {
            if science.selection_schema_id.trim().is_empty()
                || science.mission.trim().is_empty()
                || science.platforms.is_empty()
                || science.instrument.as_deref().is_some_and(str::is_empty)
                || science.product_family.trim().is_empty()
                || science.product_variants.is_empty()
            {
                return Err(format!(
                    "science resource {} has incomplete science semantics",
                    common.resource_id
                ));
            }
            if let Some(range) = &science.acquisition_time_range {
                let start = parse_canonical_registry_timestamp(&range.start);
                let end = range
                    .end
                    .as_deref()
                    .map(parse_canonical_registry_timestamp)
                    .transpose();
                let descending = matches!(
                    (&start, &end),
                    (Ok(start), Ok(Some(end))) if start > end
                );
                if start.is_err() || end.is_err() || descending {
                    return Err(format!(
                        "science resource {} has an invalid acquisition time range",
                        common.resource_id
                    ));
                }
            }
            if let Some(search) = &science.sar_search {
                validate_sar_search(common, science, search)?;
            }
        }
        ResourceDescriptor::Orbit { common, orbit } => {
            if orbit.supported_platforms.is_empty()
                || orbit.orbit_product_types.is_empty()
                || orbit.publication_preference.is_empty()
                || orbit.matching_policy_id.trim().is_empty()
            {
                return Err(format!(
                    "orbit resource {} has incomplete orbit semantics",
                    common.resource_id
                ));
            }
        }
        ResourceDescriptor::Dem { common, dem } => {
            if dem.runtime_dataset_id.trim().is_empty()
                || !dem.nominal_resolution.is_finite()
                || dem.nominal_resolution <= 0.0
                || dem.coverage.trim().is_empty()
                || dem.horizontal_crs.trim().is_empty()
                || dem.vertical_datum.trim().is_empty()
                || dem.tile_scheme.trim().is_empty()
                || dem.nodata_semantics.trim().is_empty()
            {
                return Err(format!(
                    "DEM resource {} has incomplete DEM semantics",
                    common.resource_id
                ));
            }
        }
        ResourceDescriptor::Atmospheric {
            common,
            atmospheric,
        } => {
            if atmospheric.date_binding.trim().is_empty()
                || atmospheric.aoi_binding.trim().is_empty()
                || atmospheric.max_dates_per_batch == 0
                || atmospheric.output_asset_family.trim().is_empty()
                || atmospheric.request_semantics.trim().is_empty()
            {
                return Err(format!(
                    "atmospheric resource {} has incomplete semantics",
                    common.resource_id
                ));
            }
        }
    }
    Ok(())
}

fn validate_sar_search(
    common: &ResourceDescriptorCore,
    science: &ScienceDatasetSemantics,
    search: &SarSearchSemantics,
) -> Result<(), String> {
    if science.search_schema_id.is_none()
        || search.selector.trim().is_empty()
        || search.projection_order == 0
        || search.product_families.is_empty()
        || search.polarizations.is_empty()
        || search.supported_actions.is_empty()
    {
        return Err(format!(
            "science resource {} has incomplete SAR search semantics",
            common.resource_id
        ));
    }
    let route = common
        .provider_routes
        .iter()
        .find(|route| route.route_id == search.provider_route_id)
        .ok_or_else(|| {
            format!(
                "science resource {} has an unknown SAR provider route",
                common.resource_id
            )
        })?;
    // A retired resource (e.g. the ASF-removed legacy missions, Owner
    // evidence 2026-10-31) keeps its declared SAR semantics and route
    // identity for a future restore, but neither the route nor the search
    // capability may stay executable; otherwise the retirement would be
    // unenforceable at runtime.
    if matches!(common.lifecycle, StaticCapabilityLifecycle::Retired) {
        if !matches!(
            route.lifecycle,
            StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
        ) || !common.capabilities.iter().all(|capability| {
            matches!(
                capability.lifecycle,
                StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
            )
        }) {
            return Err(format!(
                "science resource {} is retired but keeps an executable SAR selector",
                common.resource_id
            ));
        }
    } else if common.lifecycle != StaticCapabilityLifecycle::Implemented
        || route.lifecycle != StaticCapabilityLifecycle::Implemented
        || !common.capabilities.iter().any(|capability| {
            capability.kind == CapabilityKind::Search
                && capability.lifecycle == StaticCapabilityLifecycle::Implemented
                && capability
                    .provider_route_ids
                    .iter()
                    .any(|route_id| route_id == &route.route_id)
        })
    {
        return Err(format!(
            "science resource {} cannot project an executable SAR selector",
            common.resource_id
        ));
    }
    for values in [
        &search.platforms,
        &search.instruments,
        &search.processing_levels,
        &search.product_families,
        &search.beam_modes,
        &search.polarizations,
    ] {
        if values.iter().any(|value| value.trim().is_empty())
            || values.iter().collect::<HashSet<_>>().len() != values.len()
        {
            return Err(format!(
                "science resource {} has empty or duplicate SAR search values",
                common.resource_id
            ));
        }
    }
    if matches!(
        search.projection_kind,
        SarSearchProjectionKind::LegacyAsfSearch
    ) && (search.logical_dataset.is_some()
        || search.source_filters.is_some()
        || search.supports_frame != search.frame_filter.is_some()
        || (search.supports_relative_orbit || search.supports_path)
            != search.track_filter.is_some())
    {
        return Err(format!(
            "science resource {} has inconsistent legacy SAR projection semantics",
            common.resource_id
        ));
    }
    if matches!(
        search.projection_kind,
        SarSearchProjectionKind::TypedCatalogProfile
    ) {
        let Some(logical_dataset) = search.logical_dataset.as_deref() else {
            return Err(format!(
                "science resource {} has no typed logical dataset",
                common.resource_id
            ));
        };
        validate_id(logical_dataset, "logical_dataset")?;
        let Some(filters) = &search.source_filters else {
            return Err(format!(
                "science resource {} has no typed SAR source filters",
                common.resource_id
            ));
        };
        if filters.supported_keys.is_empty()
            || filters.fixed.is_empty()
            || filters
                .supported_keys
                .iter()
                .any(|value| value.trim().is_empty())
            || filters.supported_keys.iter().collect::<HashSet<_>>().len()
                != filters.supported_keys.len()
            || filters.fixed.iter().any(|(key, value)| {
                key.trim().is_empty()
                    || matches!(value, RegistryScalarValue::String(text) if text.trim().is_empty())
            })
            || filters.options.iter().any(|(key, values)| {
                key.trim().is_empty()
                    || values.is_empty()
                    || values.iter().any(|value| value.trim().is_empty())
                    || values.iter().collect::<HashSet<_>>().len() != values.len()
            })
        {
            return Err(format!(
                "science resource {} has invalid typed SAR source filters",
                common.resource_id
            ));
        }
    }
    for filter in [&search.frame_filter, &search.track_filter]
        .into_iter()
        .flatten()
    {
        if !filter.supported
            || filter.parameter.as_deref().is_none_or(str::is_empty)
            || filter.label.trim().is_empty()
            || filter.pattern.as_deref().is_none_or(str::is_empty)
            || filter
                .minimum
                .zip(filter.maximum)
                .is_some_and(|(minimum, maximum)| minimum > maximum)
            || filter.exact_width == Some(0)
            || filter
                .platform_limits
                .iter()
                .any(|limit| limit.platform.trim().is_empty() || limit.minimum > limit.maximum)
        {
            return Err(format!(
                "science resource {} has an invalid SAR filter descriptor",
                common.resource_id
            ));
        }
    }
    if search.supported_actions.iter().any(|action| {
        action.action.trim().is_empty()
            || action.platforms.iter().any(|value| value.trim().is_empty())
            || action
                .product_families
                .iter()
                .any(|value| value.trim().is_empty())
            || action
                .required_fields
                .iter()
                .any(|value| value.trim().is_empty())
    }) {
        return Err(format!(
            "science resource {} has an invalid SAR action",
            common.resource_id
        ));
    }
    Ok(())
}

fn parse_canonical_registry_timestamp(
    value: &str,
) -> Result<DateTime<chrono::FixedOffset>, String> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| format!("invalid RFC 3339 timestamp: {error}"))?;
    if parsed.offset().local_minus_utc() != 0
        || parsed.to_rfc3339_opts(SecondsFormat::Secs, true) != value
    {
        return Err("timestamp must use canonical UTC whole-second RFC 3339 form".to_owned());
    }
    Ok(parsed)
}

fn validate_dependency_target(
    registry: &ResourceRegistry,
    dependency: &ResourceDependency,
) -> Result<(), String> {
    let correct_kind = match &dependency.requirement {
        CompanionRequirement::Orbit {
            target_resource_id,
            preferred_product_types,
            matching_policy_id,
            ..
        } => {
            if preferred_product_types.is_empty() || matching_policy_id.trim().is_empty() {
                return Err(format!(
                    "dependency {} has incomplete orbit matching semantics",
                    dependency.dependency_id
                ));
            }
            matches!(
                registry.resource(target_resource_id),
                Some(ResourceDescriptor::Orbit { .. })
            )
        }
        CompanionRequirement::DemCoverage {
            target_resource_ids,
            accepted_horizontal_crs,
            accepted_vertical_datums,
            selection_policy_id,
            ..
        } => {
            if target_resource_ids.is_empty()
                || accepted_horizontal_crs.is_empty()
                || accepted_vertical_datums.is_empty()
                || selection_policy_id.trim().is_empty()
            {
                return Err(format!(
                    "dependency {} has incomplete DEM matching semantics",
                    dependency.dependency_id
                ));
            }
            target_resource_ids.iter().all(|resource_id| {
                matches!(
                    registry.resource(resource_id),
                    Some(ResourceDescriptor::Dem { .. })
                )
            })
        }
        CompanionRequirement::AtmosphericDateAoi {
            target_resource_id,
            selection_policy_id,
            ..
        } => {
            if selection_policy_id.trim().is_empty() {
                return Err(format!(
                    "dependency {} has no atmospheric selection policy",
                    dependency.dependency_id
                ));
            }
            matches!(
                registry.resource(target_resource_id),
                Some(ResourceDescriptor::Atmospheric { .. })
            )
        }
    };
    if correct_kind {
        Ok(())
    } else {
        Err(format!(
            "dependency {} targets a missing or incorrectly typed resource",
            dependency.dependency_id
        ))
    }
}

fn validate_id(value: &str, field: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.len() <= 160
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        });
    valid
        .then_some(())
        .ok_or_else(|| format!("{field} is not a stable lowercase identity: {value}"))
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildCapabilityState {
    Available,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeReadinessState {
    Ready,
    NotReady,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialReadinessState {
    Ready,
    Missing,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceAdmissionState {
    Accepted,
    Rejected,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPolicyState {
    Allowed,
    Denied,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EffectiveCapabilityInputs {
    pub build: Option<BuildCapabilityState>,
    pub runtime: Option<RuntimeReadinessState>,
    pub credential: Option<CredentialReadinessState>,
    pub admission: Option<EvidenceAdmissionState>,
    pub policy: Option<CapabilityPolicyState>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveCapabilityState {
    Executable,
    BlockedBuild,
    BlockedRuntime,
    BlockedCredential,
    BlockedAdmission,
    BlockedPolicy,
    Reserved,
    Retired,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveCapabilityReason {
    Ready,
    StaticReserved,
    StaticRetired,
    BuildUnavailableOrUnknown,
    RuntimeNotReadyOrUnknown,
    CredentialMissingOrUnknown,
    AdmissionMissingRejectedOrUnknown,
    PolicyDeniedOrUnknown,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EffectiveCapabilityProjection {
    pub state: EffectiveCapabilityState,
    pub reason: EffectiveCapabilityReason,
}

pub fn project_effective_capability(
    declaration: &CapabilityDeclaration,
    inputs: EffectiveCapabilityInputs,
) -> EffectiveCapabilityProjection {
    let projected = |state, reason| EffectiveCapabilityProjection { state, reason };
    match declaration.lifecycle {
        StaticCapabilityLifecycle::Reserved => {
            return projected(
                EffectiveCapabilityState::Reserved,
                EffectiveCapabilityReason::StaticReserved,
            )
        }
        StaticCapabilityLifecycle::Retired => {
            return projected(
                EffectiveCapabilityState::Retired,
                EffectiveCapabilityReason::StaticRetired,
            )
        }
        StaticCapabilityLifecycle::Implemented
        | StaticCapabilityLifecycle::AdmissionControlled
        | StaticCapabilityLifecycle::Retained => {}
    }
    if declaration.runtime_requirements.build_capability
        && inputs.build != Some(BuildCapabilityState::Available)
    {
        return projected(
            EffectiveCapabilityState::BlockedBuild,
            EffectiveCapabilityReason::BuildUnavailableOrUnknown,
        );
    }
    if declaration.runtime_requirements.runtime_readiness
        && inputs.runtime != Some(RuntimeReadinessState::Ready)
    {
        return projected(
            EffectiveCapabilityState::BlockedRuntime,
            EffectiveCapabilityReason::RuntimeNotReadyOrUnknown,
        );
    }
    if declaration.runtime_requirements.credential_readiness
        && inputs.credential != Some(CredentialReadinessState::Ready)
    {
        return projected(
            EffectiveCapabilityState::BlockedCredential,
            EffectiveCapabilityReason::CredentialMissingOrUnknown,
        );
    }
    if declaration.runtime_requirements.admission_evidence
        && inputs.admission != Some(EvidenceAdmissionState::Accepted)
    {
        return projected(
            EffectiveCapabilityState::BlockedAdmission,
            EffectiveCapabilityReason::AdmissionMissingRejectedOrUnknown,
        );
    }
    if declaration.runtime_requirements.policy_permission
        && inputs.policy != Some(CapabilityPolicyState::Allowed)
    {
        return projected(
            EffectiveCapabilityState::BlockedPolicy,
            EffectiveCapabilityReason::PolicyDeniedOrUnknown,
        );
    }
    projected(
        EffectiveCapabilityState::Executable,
        EffectiveCapabilityReason::Ready,
    )
}
