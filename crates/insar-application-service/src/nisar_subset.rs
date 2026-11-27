use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use chrono::SecondsFormat;
use insar_core::{
    canonicalize_nisar_subset_geojson_v2, derive_nisar_subset_geometry_authority_v2,
    embed_request_manifest, validated_authoritative_polygonal_geojson, AssetAccessScheme,
    AssetExecutionAuthority, AssetExecutionPolicy, AssetRole, CatalogSearchQuery, DownloadLayout,
    EffectiveCapabilityState, EvidenceAdmissionState, FrozenAssetAccess, FrozenSearchBatch,
    NisarSubsetChecksumAlgorithm, NisarSubsetChecksumV1, NisarSubsetFrequency,
    NisarSubsetFrequencyCapabilityV2, NisarSubsetGeometryRoleV2, NisarSubsetGeometryV2,
    NisarSubsetPlanV2Error, NisarSubsetPreparedIntentBuildInputV2, NisarSubsetPreparedIntentV2,
    NisarSubsetQualityStatusV2, NisarSubsetSourceAssetIdentityV2, NisarSubsetSourceCapabilityV2,
    NisarSubsetSourceDependencyGraphV2, NisarSubsetSourceProductIdentityV2, ProviderProduct,
    SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest, StoreSearchBatchRequest,
    StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus, TaskStateCommandKind,
    NISAR_SUBSET_IMPLEMENTATION_SCOPE, NISAR_SUBSET_OUTPUT_AUTHORITY,
};
use insar_download::NisarSsarRslcValidationContext;
use insar_provider::asf::nisar_subset::{
    canonical_nisar_polarizations_for_frequency_label, normalize_nisar_subset_aoi,
    plan_nisar_subset, prepare_nisar_subset_intent_v2, supported_dataset_polarizations,
    AsfNisarSubsetPreparedIntentDraftV2,
};
use insar_provider::catalog::{
    BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROFILE_ID,
    BHOONIDHI_PROVIDER_ID, BHOONIDHI_REQUIRED_PRODUCT_TYPE, BHOONIDHI_RESOLVER_POLICY_ID,
    NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID, NISAR_P05023_CRID,
    NISAR_P05023_PRODUCT_SPECIFICATION_VERSION, NISAR_PROFILE_ID, NISAR_PROVIDER_ID,
    NISAR_REQUIRED_PRODUCT_TYPE, NISAR_RESOLVER_POLICY_ID,
};
use insar_store::{
    embed_nisar_subset_prepared_intent_v2_envelope, parse_nisar_subset_prepared_intent_v2_envelope,
    NisarAppendReceiptStatus, NisarArchivedRecoveryReceiptStatus, NisarSuccessorReceiptStatus,
    StoreAppendExplicitTaskItemsRequest, StoreCreateHistoricalExplicitTaskRequest, StoreError,
    StoreNisarAppendIdentity, StoreNisarSuccessorIdentity, StoredTask, StoredWorkbench, TaskStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{
    parse_normalized_utc, valid_catalogue_opaque_id, ApplicationService, ApplicationServiceError,
    CoreServiceSnapshot, NISAR_REMOTE_SUBSET_CAPABILITY_ID,
};

pub const NISAR_REMOTE_SUBSET_FEATURE_ID: &str = "nisar_remote_subset_v1";

pub use insar_store::StoreCreateNisarArchivedRecoveryV2Request as NisarCreateArchivedRecoveryV2Request;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NisarArchivedRecoveryContextV2 {
    pub source_task_id: String,
    pub source_item_id: String,
    pub expected_source_revision: u64,
    pub new_task_id: String,
    pub output_root: PathBuf,
}
pub const NISAR_REMOTE_SUBSET_OFFLINE_POLICY_ID: &str = "nisar.remote-subset.offline-only.v1";

const BHOONIDHI_SSAR_CRID: &str = "P00500";
const MAX_NISAR_SUBSET_SOURCE_ITEM_ID_BYTES: usize = 1024;

/// Source-free failures for the acceptance-only Frequency A/HH projection.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum AsfNisarSubsetLiveDescriptorError {
    #[error("NISAR_LIVE_DESCRIPTOR_MAPPER_REJECTED")]
    AuthoritativeMapperRejected,
    #[error("NISAR_LIVE_DESCRIPTOR_PROJECTION_INVALID")]
    ProjectionInvalid,
    #[error("NISAR_LIVE_DESCRIPTOR_ASSET_AUTHORITY_INVALID")]
    AssetAuthorityInvalid,
    #[error("NISAR_LIVE_DESCRIPTOR_PRIMARY_ASSET_MISSING")]
    PrimaryAssetMissing,
    #[error("NISAR_LIVE_DESCRIPTOR_FREQUENCY_A_MISSING")]
    FrequencyAMissing,
}

impl AsfNisarSubsetLiveDescriptorError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::AuthoritativeMapperRejected => "NISAR_LIVE_DESCRIPTOR_MAPPER_REJECTED",
            Self::ProjectionInvalid => "NISAR_LIVE_DESCRIPTOR_PROJECTION_INVALID",
            Self::AssetAuthorityInvalid => "NISAR_LIVE_DESCRIPTOR_ASSET_AUTHORITY_INVALID",
            Self::PrimaryAssetMissing => "NISAR_LIVE_DESCRIPTOR_PRIMARY_ASSET_MISSING",
            Self::FrequencyAMissing => "NISAR_LIVE_DESCRIPTOR_FREQUENCY_A_MISSING",
        }
    }
}

/// Secret-free authority for one production-mapped Frequency A/HH live case.
/// The fields are private and this type is Serialize-only; construction is
/// possible only through the mapper-boundary seam below.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1 {
    provider_id: String,
    provider_product_id: String,
    provider_product_key: String,
    collection_id: String,
    collection_profile_id: String,
    product_specification_version: String,
    crid: String,
    granule_id: String,
    asset_role: AssetRole,
    asset_key: String,
    stable_asset_key: String,
    file_name: String,
    expected_size_bytes: u64,
    checksum: NisarSubsetChecksumV1,
    source_footprint_geojson: Map<String, Value>,
    acquisition_datetime: String,
    quality_status: NisarSubsetQualityStatusV2,
    advertised_frequency_a_polarizations: Vec<String>,
    selected_frequency: NisarSubsetFrequency,
    selected_polarizations: Vec<String>,
}

impl AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1 {
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }
    pub fn provider_product_id(&self) -> &str {
        &self.provider_product_id
    }
    pub fn provider_product_key(&self) -> &str {
        &self.provider_product_key
    }
    pub fn collection_id(&self) -> &str {
        &self.collection_id
    }
    pub fn collection_profile_id(&self) -> &str {
        &self.collection_profile_id
    }
    pub fn product_specification_version(&self) -> &str {
        &self.product_specification_version
    }
    pub fn crid(&self) -> &str {
        &self.crid
    }
    pub fn granule_id(&self) -> &str {
        &self.granule_id
    }
    pub fn asset_role(&self) -> &AssetRole {
        &self.asset_role
    }
    pub fn asset_key(&self) -> &str {
        &self.asset_key
    }
    pub fn stable_asset_key(&self) -> &str {
        &self.stable_asset_key
    }
    pub fn file_name(&self) -> &str {
        &self.file_name
    }
    pub const fn expected_size_bytes(&self) -> u64 {
        self.expected_size_bytes
    }
    pub fn checksum(&self) -> &NisarSubsetChecksumV1 {
        &self.checksum
    }
    pub fn source_footprint_geojson(&self) -> &Map<String, Value> {
        &self.source_footprint_geojson
    }
    pub fn acquisition_datetime(&self) -> &str {
        &self.acquisition_datetime
    }
    pub const fn quality_status(&self) -> NisarSubsetQualityStatusV2 {
        self.quality_status
    }
    pub fn advertised_frequency_a_polarizations(&self) -> &[String] {
        &self.advertised_frequency_a_polarizations
    }
    pub fn selected_frequency(&self) -> NisarSubsetFrequency {
        self.selected_frequency.clone()
    }
    pub fn selected_polarizations(&self) -> &[String] {
        &self.selected_polarizations
    }
}

/// Secret-free authority for one production-mapped Frequency A multi-polarization
/// live case. The fields remain private and this type is Serialize-only; the
/// selected dataset set can only be created after the authoritative mapper has
/// accepted the source and advertised Frequency A capability.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AsfNisarSubsetLiveFrequencyAMultiPolDescriptorV1 {
    provider_id: String,
    provider_product_id: String,
    provider_product_key: String,
    collection_id: String,
    collection_profile_id: String,
    product_specification_version: String,
    crid: String,
    granule_id: String,
    asset_role: AssetRole,
    asset_key: String,
    stable_asset_key: String,
    file_name: String,
    expected_size_bytes: u64,
    checksum: NisarSubsetChecksumV1,
    source_footprint_geojson: Map<String, Value>,
    acquisition_datetime: String,
    quality_status: NisarSubsetQualityStatusV2,
    advertised_frequency_a_polarizations: Vec<String>,
    selected_frequency: NisarSubsetFrequency,
    selected_polarizations: Vec<String>,
}

impl AsfNisarSubsetLiveFrequencyAMultiPolDescriptorV1 {
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }
    pub fn provider_product_id(&self) -> &str {
        &self.provider_product_id
    }
    pub fn provider_product_key(&self) -> &str {
        &self.provider_product_key
    }
    pub fn collection_id(&self) -> &str {
        &self.collection_id
    }
    pub fn collection_profile_id(&self) -> &str {
        &self.collection_profile_id
    }
    pub fn product_specification_version(&self) -> &str {
        &self.product_specification_version
    }
    pub fn crid(&self) -> &str {
        &self.crid
    }
    pub fn granule_id(&self) -> &str {
        &self.granule_id
    }
    pub fn asset_role(&self) -> &AssetRole {
        &self.asset_role
    }
    pub fn asset_key(&self) -> &str {
        &self.asset_key
    }
    pub fn stable_asset_key(&self) -> &str {
        &self.stable_asset_key
    }
    pub fn file_name(&self) -> &str {
        &self.file_name
    }
    pub const fn expected_size_bytes(&self) -> u64 {
        self.expected_size_bytes
    }
    pub fn checksum(&self) -> &NisarSubsetChecksumV1 {
        &self.checksum
    }
    pub fn source_footprint_geojson(&self) -> &Map<String, Value> {
        &self.source_footprint_geojson
    }
    pub fn acquisition_datetime(&self) -> &str {
        &self.acquisition_datetime
    }
    pub const fn quality_status(&self) -> NisarSubsetQualityStatusV2 {
        self.quality_status
    }
    pub fn advertised_frequency_a_polarizations(&self) -> &[String] {
        &self.advertised_frequency_a_polarizations
    }
    pub fn selected_frequency(&self) -> NisarSubsetFrequency {
        self.selected_frequency.clone()
    }
    pub fn selected_polarizations(&self) -> &[String] {
        &self.selected_polarizations
    }
}

impl From<AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1>
    for AsfNisarSubsetLiveFrequencyAMultiPolDescriptorV1
{
    fn from(value: AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1) -> Self {
        let AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1 {
            provider_id,
            provider_product_id,
            provider_product_key,
            collection_id,
            collection_profile_id,
            product_specification_version,
            crid,
            granule_id,
            asset_role,
            asset_key,
            stable_asset_key,
            file_name,
            expected_size_bytes,
            checksum,
            source_footprint_geojson,
            acquisition_datetime,
            quality_status,
            advertised_frequency_a_polarizations,
            selected_frequency,
            selected_polarizations,
        } = value;
        Self {
            provider_id,
            provider_product_id,
            provider_product_key,
            collection_id,
            collection_profile_id,
            product_specification_version,
            crid,
            granule_id,
            asset_role,
            asset_key,
            stable_asset_key,
            file_name,
            expected_size_bytes,
            checksum,
            source_footprint_geojson,
            acquisition_datetime,
            quality_status,
            advertised_frequency_a_polarizations,
            selected_frequency,
            selected_polarizations,
        }
    }
}

/// Run the authoritative ApplicationService mapper exactly once, then project
/// only the secret-free Frequency A/HH descriptor from its SnapshotItem.
pub fn build_nisar_subset_live_frequency_a_hh_descriptor(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
) -> Result<AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1, AsfNisarSubsetLiveDescriptorError> {
    let item = super::map_nisar_rslc_catalogue_product(product, query)
        .map_err(|_| AsfNisarSubsetLiveDescriptorError::AuthoritativeMapperRejected)?;
    project_nisar_subset_live_frequency_a_descriptor(&item, true)
}

/// Build a mapper-bound Frequency A multi-polarization descriptor. Requested
/// dataset names are canonicalized to sorted unique order, then every one is
/// required to be present in the mapped source's authoritative A capability.
/// No filename or locator text participates in polarization selection.
pub fn build_nisar_subset_live_frequency_a_multi_pol_descriptor(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
    requested_polarizations: &[String],
) -> Result<AsfNisarSubsetLiveFrequencyAMultiPolDescriptorV1, AsfNisarSubsetLiveDescriptorError> {
    let selected_polarizations =
        normalize_requested_frequency_a_polarizations(requested_polarizations)?;
    let item = super::map_nisar_rslc_catalogue_product(product, query)
        .map_err(|_| AsfNisarSubsetLiveDescriptorError::AuthoritativeMapperRejected)?;
    let single = project_nisar_subset_live_frequency_a_descriptor(&item, false)?;
    if selected_polarizations.iter().any(|polarization| {
        !single
            .advertised_frequency_a_polarizations()
            .contains(polarization)
    }) {
        return Err(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid);
    }
    let mut descriptor = AsfNisarSubsetLiveFrequencyAMultiPolDescriptorV1::from(single);
    descriptor.selected_polarizations = selected_polarizations;
    Ok(descriptor)
}

/// Convenience projection for the currently supported Frequency A HH+HV
/// live case.
pub fn build_nisar_subset_live_frequency_a_hh_hv_descriptor(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
) -> Result<AsfNisarSubsetLiveFrequencyAMultiPolDescriptorV1, AsfNisarSubsetLiveDescriptorError> {
    build_nisar_subset_live_frequency_a_multi_pol_descriptor(
        product,
        query,
        &["HH".to_owned(), "HV".to_owned()],
    )
}

fn normalize_requested_frequency_a_polarizations(
    requested_polarizations: &[String],
) -> Result<Vec<String>, AsfNisarSubsetLiveDescriptorError> {
    if requested_polarizations.len() < 2 || requested_polarizations.len() > 4 {
        return Err(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid);
    }
    let mut normalized = requested_polarizations.to_vec();
    if normalized.iter().any(|value| {
        value.is_empty()
            || value != value.trim()
            || !value.bytes().all(|byte| byte.is_ascii_uppercase())
            || !supported_dataset_polarizations(&NisarSubsetFrequency::A).contains(&value.as_str())
    }) {
        return Err(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid);
    }
    normalized.sort();
    if normalized.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid);
    }
    Ok(normalized)
}

fn project_nisar_subset_live_frequency_a_descriptor(
    item: &SnapshotItem,
    require_hh: bool,
) -> Result<AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1, AsfNisarSubsetLiveDescriptorError> {
    if item.validate().is_err() {
        return Err(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid);
    }
    let asset = item
        .asset_access
        .as_ref()
        .ok_or(AsfNisarSubsetLiveDescriptorError::PrimaryAssetMissing)?;
    if item.provider_id != NISAR_PROVIDER_ID || asset.role != AssetRole::Primary {
        return Err(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid);
    }
    let policy = item
        .execution_policy
        .as_ref()
        .ok_or(AsfNisarSubsetLiveDescriptorError::AssetAuthorityInvalid)?;
    if policy.authority != AssetExecutionAuthority::ResolverAuthoritative
        || policy.policy_id != NISAR_RESOLVER_POLICY_ID
        || policy.collection_profile_id.as_deref() != Some(NISAR_PROFILE_ID)
        || asset.reference.provider_id != NISAR_PROVIDER_ID
        || asset.reference.collection_id != NISAR_NATIVE_COLLECTION_ID
    {
        return Err(AsfNisarSubsetLiveDescriptorError::AssetAuthorityInvalid);
    }

    let provider_product_id = metadata_text(item, "provider_product_id")
        .filter(|value| safe_projection_atom(value, 512))
        .ok_or(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid)?;
    let provider_product_key = metadata_text(item, "provider_product_key")
        .filter(|value| safe_projection_atom(value, 1024))
        .ok_or(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid)?;
    if item.item_id != provider_product_key
        || asset.reference.provider_product_id != provider_product_id
        || asset.reference.provider_product_key != provider_product_key
    {
        return Err(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid);
    }
    let granule_id = item
        .display_name
        .as_str()
        .strip_prefix("NISAR_L1_PR_RSLC_")
        .filter(|_| safe_projection_atom(&item.display_name, 512))
        .map(|_| item.display_name.clone())
        .ok_or(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid)?;
    let asset_key = asset.reference.asset_key.as_str();
    let stable_asset_key = asset.reference.stable_asset_key.as_str();
    if !safe_projection_atom(asset_key, 512) || !safe_projection_atom(stable_asset_key, 1024) {
        return Err(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid);
    }

    let file_name = asset
        .file_name
        .as_deref()
        .filter(|value| safe_projection_atom(value, 512))
        .filter(|value| value.to_ascii_lowercase().ends_with(".h5"))
        .filter(|value| *value == format!("{granule_id}.h5"))
        .ok_or(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid)?
        .to_owned();
    let expected_size_bytes = asset
        .expected_size_bytes
        .filter(|value| *value > 0)
        .ok_or(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid)?;
    let checksum = projected_source_checksum(asset)
        .ok_or(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid)?;
    let source_footprint_geojson = item
        .footprint_geojson
        .as_ref()
        .and_then(canonical_live_footprint)
        .ok_or(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid)?;
    let acquisition_datetime = item
        .acquisition_datetime
        .as_deref()
        .and_then(super::parse_normalized_utc)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
        .ok_or(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid)?;
    let label = metadata_text(item, "frequency_a_polarization")
        .ok_or(AsfNisarSubsetLiveDescriptorError::FrequencyAMissing)?;
    let advertised_frequency_a_polarizations =
        canonical_nisar_polarizations_for_frequency_label(&NisarSubsetFrequency::A, label)
            .ok_or(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid)?;
    if require_hh
        && !advertised_frequency_a_polarizations
            .iter()
            .any(|value| value == "HH")
    {
        return Err(AsfNisarSubsetLiveDescriptorError::ProjectionInvalid);
    }
    let selected_polarizations = if require_hh {
        vec!["HH".to_owned()]
    } else {
        advertised_frequency_a_polarizations.clone()
    };

    Ok(AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1 {
        provider_id: item.provider_id.clone(),
        provider_product_id: provider_product_id.to_owned(),
        provider_product_key: provider_product_key.to_owned(),
        collection_id: asset.reference.collection_id.clone(),
        collection_profile_id: NISAR_PROFILE_ID.to_owned(),
        product_specification_version: NISAR_P05023_PRODUCT_SPECIFICATION_VERSION.to_owned(),
        crid: NISAR_P05023_CRID.to_owned(),
        granule_id,
        asset_role: AssetRole::Primary,
        asset_key: asset_key.to_owned(),
        stable_asset_key: stable_asset_key.to_owned(),
        file_name,
        expected_size_bytes,
        checksum,
        source_footprint_geojson,
        acquisition_datetime,
        quality_status: NisarSubsetQualityStatusV2::Provisional,
        advertised_frequency_a_polarizations,
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations,
    })
}

fn projected_source_checksum(asset: &FrozenAssetAccess) -> Option<NisarSubsetChecksumV1> {
    let algorithm = asset.checksum_algorithm.as_deref()?;
    let value = asset.checksum_value.as_deref()?;
    let (algorithm, length) = if algorithm.eq_ignore_ascii_case("md5") {
        (NisarSubsetChecksumAlgorithm::Md5, 32)
    } else if algorithm.eq_ignore_ascii_case("sha1") {
        (NisarSubsetChecksumAlgorithm::Sha1, 40)
    } else if algorithm.eq_ignore_ascii_case("sha256") || algorithm.eq_ignore_ascii_case("sha-256")
    {
        (NisarSubsetChecksumAlgorithm::Sha256, 64)
    } else {
        return None;
    };
    (value.len() == length
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && safe_projection_atom(value, length))
    .then(|| NisarSubsetChecksumV1 {
        algorithm,
        value: value.to_ascii_lowercase(),
    })
}

fn canonical_live_footprint(value: &Map<String, Value>) -> Option<Map<String, Value>> {
    let geometry = validated_authoritative_polygonal_geojson(value)?;
    let kind = geometry.geojson.get("type")?.as_str()?;
    let coordinates = geometry.geojson.get("coordinates")?.clone();
    matches!(kind, "Polygon" | "MultiPolygon").then_some(Map::from_iter([
        ("type".to_owned(), Value::String(kind.to_owned())),
        ("coordinates".to_owned(), coordinates),
    ]))
}

fn safe_projection_atom(value: &str, maximum_bytes: usize) -> bool {
    let lowercase = value.to_ascii_lowercase();
    !value.is_empty()
        && value.len() <= maximum_bytes
        && value == value.trim()
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
        && !value.contains(['/', '\\', '?', '#', '@'])
        && !lowercase.contains("://")
        && ![
            "authorization",
            "cookie",
            "credential",
            "password",
            "username",
            "bearer",
            "secret",
            "token",
            "x-amz-signature",
        ]
        .iter()
        .any(|fragment| lowercase.contains(fragment))
}

fn metadata_text<'a>(item: &'a SnapshotItem, key: &str) -> Option<&'a str> {
    item.metadata.get(key).and_then(Value::as_str)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarRemoteSubsetCapability {
    pub feature_id: String,
    pub visible: bool,
    pub enabled: bool,
    pub implementation_scope: String,
    pub offline_verified: bool,
    pub live_verified: bool,
    pub production_enabled: bool,
    pub default_enabled: bool,
    pub output_authority: String,
    pub supported_product_type: String,
    pub supported_specification_versions: Vec<String>,
    pub supported_frequencies: Vec<String>,
    pub supported_polarizations: BTreeMap<String, Vec<String>>,
    pub reason_code: Option<String>,
}

impl NisarRemoteSubsetCapability {
    pub(crate) fn new(
        fixture_enabled: bool,
        live_verified: bool,
        production_enabled: bool,
    ) -> Self {
        let supported_polarizations = [NisarSubsetFrequency::A, NisarSubsetFrequency::B]
            .into_iter()
            .map(|frequency| {
                let name = match frequency {
                    NisarSubsetFrequency::A => "A",
                    NisarSubsetFrequency::B => "B",
                };
                let values = supported_dataset_polarizations(&frequency)
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect();
                (name.to_owned(), values)
            })
            .collect();
        let enabled = fixture_enabled || production_enabled;
        Self {
            feature_id: NISAR_REMOTE_SUBSET_FEATURE_ID.to_owned(),
            // The normal host must advertise the surface while keeping the
            // live operation remains disabled until product policy admits it.
            visible: true,
            enabled,
            implementation_scope: if production_enabled {
                "live_range_writer_store_v1".to_owned()
            } else {
                NISAR_SUBSET_IMPLEMENTATION_SCOPE.to_owned()
            },
            offline_verified: true,
            live_verified,
            production_enabled,
            default_enabled: super::resource_capabilities::application_default_enabled(
                NISAR_REMOTE_SUBSET_CAPABILITY_ID,
            ),
            output_authority: NISAR_SUBSET_OUTPUT_AUTHORITY.to_owned(),
            supported_product_type: NISAR_REQUIRED_PRODUCT_TYPE.to_owned(),
            supported_specification_versions: vec![
                NISAR_P05023_PRODUCT_SPECIFICATION_VERSION.to_owned()
            ],
            supported_frequencies: vec!["A".to_owned(), "B".to_owned()],
            supported_polarizations,
            reason_code: (!enabled).then(|| "NISAR_SUBSET_NOT_ENABLED".to_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarCreateRemoteSubsetTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    pub item_id: String,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub frequency: NisarSubsetFrequency,
    pub polarization: String,
    pub created_at: String,
}

/// User-owned V2 intent preparation input. Every scientific authority omitted
/// from this shape is recovered from frozen Rust-owned source evidence.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarPrepareSubsetIntentV2Request {
    pub source_batch_id: String,
    pub source_item_id: String,
    pub expected_workbench_revision: u64,
    pub subset_draft_geometry: serde_json::Map<String, Value>,
    pub selected_frequency: String,
    pub selected_polarizations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NisarPreparedSubsetIntentV2 {
    pub source_batch_id: String,
    pub source_item_id: String,
    pub workbench_revision: u64,
    pub intent: NisarSubsetPreparedIntentV2,
}

/// Read-only authoritative overlap and capability check for one frozen batch.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarPreflightSubsetBatchV2Request {
    pub source_batch_id: String,
    pub source_item_ids: Vec<String>,
    pub expected_workbench_revision: u64,
    pub subset_draft_geometry: serde_json::Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetSelectionV2 {
    pub frequency: String,
    pub polarizations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NisarPreflightEligibleItemV2 {
    pub source_item_id: String,
    pub effective_geometry: NisarSubsetGeometryV2,
    pub available_selections: Vec<NisarSubsetSelectionV2>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NisarPreflightSubsetBatchV2 {
    pub source_batch_id: String,
    pub workbench_revision: u64,
    pub eligible_items: Vec<NisarPreflightEligibleItemV2>,
    pub excluded_items: Vec<NisarSubsetBatchExcludedItemV2>,
    pub common_selections: Vec<NisarSubsetSelectionV2>,
}

/// One copied workspace request applied to an ordered, bounded set of exact
/// products from the same frozen SearchBatch.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarPrepareSubsetBatchV2Request {
    pub source_batch_id: String,
    pub source_item_ids: Vec<String>,
    pub expected_workbench_revision: u64,
    pub subset_draft_geometry: serde_json::Map<String, Value>,
    pub selected_frequency: String,
    pub selected_polarizations: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NisarSubsetBatchExclusionReason {
    NoOverlap,
    FrequencyIncompatible,
    PolarizationIncompatible,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetBatchExcludedItemV2 {
    pub source_item_id: String,
    pub reason: NisarSubsetBatchExclusionReason,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NisarPreparedSubsetBatchV2 {
    pub source_batch_id: String,
    pub workbench_revision: u64,
    pub intents: Vec<NisarPreparedSubsetIntentV2>,
    pub excluded_items: Vec<NisarSubsetBatchExcludedItemV2>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarConfirmSubsetBatchItemV2 {
    pub source_item_id: String,
    pub prepared_intent: NisarSubsetPreparedIntentV2,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarConfirmSubsetBatchTaskV2Request {
    pub source_batch_id: String,
    pub expected_workbench_revision: u64,
    pub items: Vec<NisarConfirmSubsetBatchItemV2>,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

/// Append exact source scenes to an active AOI subset task. The service
/// reconstructs one PreparedIntent per source from the task-frozen AOI,
/// Frequency, and polarization selection; callers never carry science intent.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarAppendSubsetBatchTaskV2Request {
    pub task_id: String,
    pub expected_task_revision: u64,
    pub source_item_ids: Vec<String>,
    pub event_id: String,
    pub occurred_at: String,
}

impl NisarAppendSubsetBatchTaskV2Request {
    pub(crate) fn receipt_identity(&self) -> StoreNisarAppendIdentity {
        StoreNisarAppendIdentity {
            task_id: self.task_id.clone(),
            expected_task_revision: self.expected_task_revision,
            source_item_ids: self.source_item_ids.clone(),
            event_id: self.event_id.clone(),
            occurred_at: self.occurred_at.clone(),
        }
    }
}

/// Read the immutable science and destination authority frozen into an
/// existing NISAR subset task. Candidate products are intentionally omitted;
/// callers page the task's source batch through the bounded source-page API.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarAppendSubsetContextV2Request {
    pub task_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NisarAppendSubsetContextV2 {
    pub task_id: String,
    pub source_batch_id: String,
    pub task_revision: u64,
    pub task_status: TaskLifecycleStatus,
    pub frozen_aoi_geojson: Map<String, Value>,
    pub selected_frequency: String,
    pub selected_polarizations: Vec<String>,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    /// Original frozen selection, not the entire source batch.
    pub original_item_ids: Vec<String>,
}

/// Authoritatively preflight new source scenes against an existing task's
/// frozen intent. This request is independent of the active workbench.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarPreflightSubsetAppendV2Request {
    pub task_id: String,
    pub expected_task_revision: u64,
    pub source_item_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NisarPreflightSubsetAppendV2 {
    pub task_id: String,
    pub source_batch_id: String,
    pub task_revision: u64,
    pub eligible_items: Vec<NisarPreflightEligibleItemV2>,
    pub excluded_items: Vec<NisarSubsetBatchExcludedItemV2>,
    pub common_selections: Vec<NisarSubsetSelectionV2>,
}

/// Create a fresh successor for a terminal NISAR subset task. The successor
/// retains the source task's frozen batch and science intent; its destination
/// fields are supplied explicitly after user confirmation.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarCreateSubsetSuccessorTaskV2Request {
    pub source_task_id: String,
    pub expected_task_revision: u64,
    pub source_item_ids: Vec<String>,
    pub new_task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
    /// Fresh reconstruction only: never transfers historic Range/checkpoints.
    #[serde(default)]
    pub recreate_original_items: bool,
}

impl NisarCreateSubsetSuccessorTaskV2Request {
    pub(crate) fn receipt_identity(&self) -> StoreNisarSuccessorIdentity {
        StoreNisarSuccessorIdentity {
            source_task_id: self.source_task_id.clone(),
            expected_task_revision: self.expected_task_revision,
            source_item_ids: self.source_item_ids.clone(),
            new_task_id: self.new_task_id.clone(),
            output_root: self.output_root.clone(),
            layout: self.layout.clone(),
            concurrency: self.concurrency,
            created_at: self.created_at.clone(),
            recreate_original_items: self.recreate_original_items,
        }
    }
}

/// Destination-confirmed V2 task input. The exact prepared intent is accepted
/// only after being revalidated against the current frozen source authority.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NisarConfirmSubsetTaskV2Request {
    pub source_batch_id: String,
    pub source_item_id: String,
    pub expected_workbench_revision: u64,
    pub prepared_intent: NisarSubsetPreparedIntentV2,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

/// Process-local synthetic graph evidence for deterministic tests only.
/// It is intentionally not serializable and cannot cross a transport boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct NisarSubsetDependencyEvidenceV2Fixture {
    pub source_item_id: String,
    pub graph: NisarSubsetSourceDependencyGraphV2,
}

#[derive(Clone)]
struct PreparedSubsetContextV2 {
    source_item: insar_core::SnapshotItem,
    source_page_size: u64,
    prepared: NisarPreparedSubsetIntentV2,
}

struct NisarAppendAuthorityV2 {
    task: StoredTask,
    source_batch: FrozenSearchBatch,
    intent: NisarSubsetPreparedIntentV2,
}

struct NisarAppendPreflightEvaluationV2 {
    eligible_items: Vec<NisarPreflightEligibleItemV2>,
    excluded_items: Vec<NisarSubsetBatchExcludedItemV2>,
}

impl ApplicationService {
    pub fn nisar_remote_subset_capability(
        &self,
    ) -> Result<NisarRemoteSubsetCapability, ApplicationServiceError> {
        self.ensure_running()?;
        let status = self
            .effective_capability(NISAR_REMOTE_SUBSET_CAPABILITY_ID)?
            .expect("NISAR Remote Subset declaration is embedded in the registry");
        let live_verified = status.inputs.admission == Some(EvidenceAdmissionState::Accepted);
        let production_enabled = status.projection.state == EffectiveCapabilityState::Executable;
        Ok(NisarRemoteSubsetCapability::new(
            self.nisar_subset_fixture_evidence.is_some(),
            live_verified,
            production_enabled,
        ))
    }

    pub fn create_nisar_remote_subset_task(
        &self,
        request: &NisarCreateRemoteSubsetTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        let capability = self.nisar_remote_subset_capability()?;
        if !capability.visible || !capability.enabled {
            return Err(ApplicationServiceError::NisarSubsetNotEnabled);
        }
        if !valid_request(request) {
            return Err(ApplicationServiceError::NisarSubsetRequestInvalid);
        }
        if !supported_dataset_polarizations(&request.frequency)
            .contains(&request.polarization.as_str())
        {
            return Err(ApplicationServiceError::NisarSubsetSelectionUnsupported);
        }
        // The remote subset reads NISAR L ranges from ASF with Earthdata.
        self.ensure_earthdata_for_admission()?;

        let frozen = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store
                .load_workbench()
                .map_err(ApplicationServiceError::from)?
                .ok_or(ApplicationServiceError::NisarSubsetSourceInvalid)
        })?;
        if frozen.batch.batch_id != request.source_batch_id {
            return Err(ApplicationServiceError::NisarSubsetSourceInvalid);
        }
        if !frozen
            .state
            .checked_item_ids
            .iter()
            .any(|item_id| item_id == &request.item_id)
        {
            return Err(ApplicationServiceError::NisarSubsetSourceInvalid);
        }
        let source_item = frozen
            .batch
            .candidates
            .iter()
            .find(|item| item.item_id == request.item_id)
            .ok_or(ApplicationServiceError::NisarSubsetSourceInvalid)?;
        validate_source_projection(source_item)?;

        let batch_aoi = frozen
            .batch
            .aoi_geojson
            .as_ref()
            .ok_or(ApplicationServiceError::NisarSubsetSourceInvalid)?;
        let normalized_aoi = normalize_nisar_subset_aoi(batch_aoi)
            .map_err(|_| ApplicationServiceError::NisarSubsetSourceInvalid)?;
        let evidence = self
            .nisar_subset_fixture_evidence
            .as_ref()
            .and_then(|values| {
                values.iter().find(|evidence| {
                    evidence.aoi_sha256 == normalized_aoi.sha256
                        && evidence.frequency == request.frequency
                        && evidence.swath == "LSAR_RSLC"
                })
            });
        let live_request = evidence.is_none();
        if live_request && !capability.production_enabled {
            return Err(ApplicationServiceError::NisarSubsetSelectionUnsupported);
        }
        let manifest = evidence
            .map(|evidence| {
                plan_nisar_subset(
                    source_item,
                    batch_aoi,
                    request.frequency.clone(),
                    &request.polarization,
                    "LSAR_RSLC",
                    evidence,
                )
                .map_err(|_| ApplicationServiceError::NisarSubsetSelectionUnsupported)
            })
            .transpose()?;

        let mut derived_item = source_item.clone();
        if let Some(manifest) = &manifest {
            embed_request_manifest(&mut derived_item.metadata, manifest)
                .map_err(|_| ApplicationServiceError::NisarSubsetRequestInvalid)?;
        }
        for (key, value) in [
            ("delivery_kind", "remote_derived_subset"),
            ("output_authority", NISAR_SUBSET_OUTPUT_AUTHORITY),
            (
                "implementation_scope",
                if live_request {
                    "live_range_writer_store_v1"
                } else {
                    NISAR_SUBSET_IMPLEMENTATION_SCOPE
                },
            ),
            ("source_batch_id", request.source_batch_id.as_str()),
            ("source_item_id", request.item_id.as_str()),
            (
                "subset_frequency",
                match &request.frequency {
                    NisarSubsetFrequency::A => "A",
                    NisarSubsetFrequency::B => "B",
                },
            ),
            ("subset_polarization", request.polarization.as_str()),
            ("subset_swath", "LSAR_RSLC"),
        ] {
            derived_item
                .metadata
                .insert(key.to_owned(), Value::String(value.to_owned()));
        }
        if live_request {
            derived_item.metadata.insert(
                "subset_aoi_geojson".to_owned(),
                Value::Object(normalized_aoi.canonical_geojson.clone()),
            );
        }
        derived_item.execution_policy = Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: if live_request {
                NISAR_RESOLVER_POLICY_ID.to_owned()
            } else {
                NISAR_REMOTE_SUBSET_OFFLINE_POLICY_ID.to_owned()
            },
            collection_profile_id: Some(NISAR_PROFILE_ID.to_owned()),
        });

        let derived_batch_id = format!("nisar-subset-{}", request.task_id);
        if !valid_catalogue_opaque_id(&derived_batch_id) {
            return Err(ApplicationServiceError::NisarSubsetRequestInvalid);
        }
        let mut derived_batch = frozen.batch.clone();
        derived_batch.batch_id = derived_batch_id.clone();
        derived_batch.created_at = request.created_at.clone();
        derived_batch.candidates = vec![derived_item];
        derived_batch.query.insert(
            "derived_from_batch_id".to_owned(),
            Value::String(request.source_batch_id.clone()),
        );
        derived_batch.query.insert(
            "delivery_kind".to_owned(),
            Value::String("remote_derived_subset".to_owned()),
        );
        derived_batch
            .validate()
            .map_err(|_| ApplicationServiceError::NisarSubsetRequestInvalid)?;
        let batch_request = StoreSearchBatchRequest {
            batch: derived_batch,
            workbench: SearchWorkbenchState {
                batch_id: derived_batch_id.clone(),
                checked_item_ids: vec![request.item_id.clone()],
                page_index: 0,
                page_size: frozen.state.page_size,
                revision: 0,
            },
            activate_workbench: false,
        };
        let task_request = StoreCreateTaskRequest {
            task_id: request.task_id.clone(),
            batch_id: derived_batch_id,
            selected_item_ids: vec![request.item_id.clone()],
            checked_item_ids: vec![request.item_id.clone()],
            page_index: 0,
            page_size: frozen.state.page_size,
            output_root: request.output_root.clone(),
            layout: request.layout.clone(),
            concurrency: request.concurrency,
            created_at: request.created_at.clone(),
        };

        self.with_business_runtime(|runtime| {
            {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let current = store
                    .load_workbench()
                    .map_err(ApplicationServiceError::from)?
                    .ok_or(ApplicationServiceError::NisarSubsetSourceChanged)?;
                if current != frozen {
                    return Err(ApplicationServiceError::NisarSubsetSourceChanged);
                }
                store
                    .save_search_batch_and_create_task(&batch_request, &task_request)
                    .map_err(|_| ApplicationServiceError::NisarSubsetTaskConflict)?;
            }
            runtime
                .scheduler
                .as_ref()
                .ok_or(ApplicationServiceError::BusinessRuntimeUnavailable)?
                .wake();
            Self::snapshot_from_runtime(runtime)
        })
    }
}

impl ApplicationService {
    pub fn read_nisar_subset_append_context_v2(
        &self,
        request: &NisarAppendSubsetContextV2Request,
    ) -> Result<NisarAppendSubsetContextV2, ApplicationServiceError> {
        self.ensure_running()?;
        validate_nisar_append_task_id(&request.task_id)?;
        self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let authority = load_nisar_append_authority_v2(&store, &request.task_id)?;
            Ok(NisarAppendSubsetContextV2 {
                task_id: request.task_id.clone(),
                source_batch_id: authority.task.snapshot.batch_id,
                task_revision: authority.task.runtime.revision,
                task_status: authority.task.runtime.status,
                frozen_aoi_geojson: authority.intent.requested_geometry.canonical_geojson,
                selected_frequency: frequency_name_v2(&authority.intent.selected_frequency)
                    .to_owned(),
                selected_polarizations: authority.intent.selected_polarizations,
                output_root: authority.task.snapshot.output_root,
                layout: authority.task.snapshot.layout,
                concurrency: authority.task.runtime.concurrency,
                original_item_ids: authority
                    .task
                    .snapshot
                    .candidates
                    .iter()
                    .map(|item| item.item_id.clone())
                    .collect(),
            })
        })
    }

    pub fn preflight_nisar_subset_append_v2(
        &self,
        request: &NisarPreflightSubsetAppendV2Request,
    ) -> Result<NisarPreflightSubsetAppendV2, ApplicationServiceError> {
        self.ensure_running()?;
        validate_nisar_append_task_id(&request.task_id)?;
        validate_nisar_append_source_ids(&request.source_item_ids)?;
        self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let authority = load_nisar_append_authority_v2(&store, &request.task_id)?;
            if authority.task.runtime.revision != request.expected_task_revision {
                return Err(ApplicationServiceError::NisarSubsetV2SearchBatchStale);
            }
            let evaluation =
                evaluate_nisar_append_candidates_v2(&authority, &request.source_item_ids)?;
            let common_selections = common_nisar_subset_selections_v2(&evaluation.eligible_items);
            Ok(NisarPreflightSubsetAppendV2 {
                task_id: request.task_id.clone(),
                source_batch_id: authority.task.snapshot.batch_id,
                task_revision: authority.task.runtime.revision,
                eligible_items: evaluation.eligible_items,
                excluded_items: evaluation.excluded_items,
                common_selections,
            })
        })
    }

    pub fn create_nisar_subset_successor_task_v2(
        &self,
        request: &NisarCreateSubsetSuccessorTaskV2Request,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_running()?;
        validate_nisar_append_task_id(&request.source_task_id)?;
        validate_nisar_append_task_id(&request.new_task_id)?;
        if request.source_task_id == request.new_task_id
            || request.output_root.as_os_str().is_empty()
            || !request.output_root.is_absolute()
            || !(1..=8).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
        {
            return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
        }
        validate_nisar_append_source_ids(&request.source_item_ids)?;
        let identity = request.receipt_identity();
        let receipt_status = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let status = store
                .nisar_successor_receipt_status(&identity)
                .map_err(map_v2_store_error)?;
            if status == NisarSuccessorReceiptStatus::Committed
                && store
                    .load_task_record(&request.new_task_id)
                    .map_err(map_v2_store_error)?
                    .is_none()
            {
                return Err(ApplicationServiceError::NisarSubsetV2TaskConflict);
            }
            Ok(status)
        })?;
        match receipt_status {
            NisarSuccessorReceiptStatus::Committed => {
                return self.with_business_runtime(Self::snapshot_from_runtime);
            }
            NisarSuccessorReceiptStatus::Conflict => {
                return Err(ApplicationServiceError::NisarSubsetV2TaskConflict);
            }
            NisarSuccessorReceiptStatus::Unknown => {}
        }
        let mut store_attempted = false;
        let outcome = (|| {
            self.ensure_task_session_for_admission(&request.source_task_id)?;
            self.with_business_runtime(|runtime| {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                match store
                    .nisar_successor_receipt_status(&identity)
                    .map_err(map_v2_store_error)?
                {
                    NisarSuccessorReceiptStatus::Committed => {
                        if store
                            .load_task_record(&request.new_task_id)
                            .map_err(map_v2_store_error)?
                            .is_none()
                        {
                            return Err(ApplicationServiceError::NisarSubsetV2TaskConflict);
                        }
                        drop(store);
                        return Self::snapshot_from_runtime(runtime);
                    }
                    NisarSuccessorReceiptStatus::Conflict => {
                        return Err(ApplicationServiceError::NisarSubsetV2TaskConflict)
                    }
                    NisarSuccessorReceiptStatus::Unknown => {}
                }
                let authority = load_nisar_append_authority_v2(&store, &request.source_task_id)?;
                if authority.task.runtime.revision != request.expected_task_revision {
                    return Err(ApplicationServiceError::NisarSubsetV2SearchBatchStale);
                }
                if !matches!(
                    authority.task.runtime.status,
                    TaskLifecycleStatus::Completed
                        | TaskLifecycleStatus::Failed
                        | TaskLifecycleStatus::Cancelled
                ) {
                    return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
                }
                let existing_sources = task_source_item_ids_v2(&authority.task);
                if request.recreate_original_items {
                    // Explicit fresh reconstruction, never a checkpoint migration.
                    // Recreate exactly the original scenes with a separate output.
                    if existing_sources.len() != authority.task.snapshot.candidates.len()
                        || request.source_item_ids.len() != existing_sources.len()
                        || request
                            .source_item_ids
                            .iter()
                            .any(|id| !existing_sources.contains(id))
                        || request.output_root
                            != authority
                                .task
                                .snapshot
                                .output_root
                                .join(&request.new_task_id)
                    {
                        return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
                    }
                } else {
                    if request
                        .source_item_ids
                        .iter()
                        .any(|id| existing_sources.contains(id))
                    {
                        return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
                    }
                    let evaluation =
                        evaluate_nisar_append_candidates_v2(&authority, &request.source_item_ids)?;
                    if !evaluation.excluded_items.is_empty() {
                        return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
                    }
                }
                let (source_item_ids, task_items, source_page_size) =
                    build_nisar_append_task_items_v2(&authority, &request.source_item_ids)?;
                if task_items.is_empty() {
                    return Err(ApplicationServiceError::NisarSubsetV2EmptyEffectiveGeometry);
                }
                let selected_item_ids = task_items
                    .iter()
                    .map(|item| item.item_id.clone())
                    .collect::<Vec<_>>();
                let task_request = StoreCreateTaskRequest {
                    task_id: request.new_task_id.clone(),
                    batch_id: authority.task.snapshot.batch_id.clone(),
                    selected_item_ids: selected_item_ids.clone(),
                    checked_item_ids: selected_item_ids,
                    page_index: 0,
                    page_size: source_page_size,
                    output_root: request.output_root.clone(),
                    layout: request.layout.clone(),
                    concurrency: request.concurrency,
                    created_at: request.created_at.clone(),
                };
                store_attempted = true;
                let (stored, replayed) = store
                    .create_historical_paused_explicit_task_with_receipt(
                        &StoreCreateHistoricalExplicitTaskRequest {
                            source_task_id: request.source_task_id.clone(),
                            expected_source_task_revision: request.expected_task_revision,
                            task: task_request,
                            source_item_ids,
                            task_items,
                        },
                        &identity,
                    )
                    .map_err(map_v2_store_error)?;
                if !replayed
                    && (stored.runtime.status != TaskLifecycleStatus::Paused
                        || !stored.runtime.task_paused
                        || !stored.runtime.queued_item_ids.is_empty()
                        || stored.runtime.paused_item_ids.len() != stored.snapshot.candidates.len())
                {
                    return Err(ApplicationServiceError::NisarSubsetV2StorePersistenceFailed);
                }
                drop(store);
                Self::snapshot_from_runtime(runtime)
            })
        })();
        if outcome.is_err() && !store_attempted {
            // Another Store connection may have committed the identical
            // request while mutable admission checks were in progress.
            let committed = self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                Ok(store
                    .nisar_successor_receipt_status(&identity)
                    .map_err(map_v2_store_error)?
                    == NisarSuccessorReceiptStatus::Committed
                    && store
                        .load_task_record(&request.new_task_id)
                        .map_err(map_v2_store_error)?
                        .is_some())
            });
            if matches!(committed, Ok(true)) {
                return self.with_business_runtime(Self::snapshot_from_runtime);
            }
        }
        outcome
    }

    pub fn read_nisar_archived_recovery_context_v2(
        &self,
        source_task_id: &str,
        new_task_id: &str,
    ) -> Result<NisarArchivedRecoveryContextV2, ApplicationServiceError> {
        self.ensure_running()?;
        validate_nisar_append_task_id(source_task_id)?;
        validate_nisar_append_task_id(new_task_id)?;
        if source_task_id == new_task_id
            || !new_task_id
                .bytes()
                .all(|v| v.is_ascii_alphanumeric() || v == b'-' || v == b'_')
        {
            return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
        }
        self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let source = store
                .inspect_nisar_archived_recovery_source_v2(source_task_id)
                .map_err(map_v2_store_error)?;
            Ok(NisarArchivedRecoveryContextV2 {
                source_task_id: source_task_id.to_owned(),
                source_item_id: source
                    .snapshot
                    .candidates
                    .first()
                    .ok_or(ApplicationServiceError::NisarSubsetV2SourceItemUnavailable)?
                    .item_id
                    .clone(),
                expected_source_revision: source.runtime.revision,
                new_task_id: new_task_id.to_owned(),
                output_root: source.snapshot.output_root.join(new_task_id),
            })
        })
    }

    pub fn create_nisar_archived_recovery_v2(
        &self,
        request: &NisarCreateArchivedRecoveryV2Request,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_running()?;
        validate_nisar_append_task_id(&request.source_task_id)?;
        validate_nisar_append_task_id(&request.new_task_id)?;
        if parse_normalized_utc(&request.created_at).is_none() {
            return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
        }
        let read_receipt = || {
            self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let status = store
                    .nisar_archived_recovery_receipt_status(request)
                    .map_err(map_v2_store_error)?;
                Ok(status)
            })
        };
        match read_receipt()? {
            NisarArchivedRecoveryReceiptStatus::Committed => {
                return self.with_business_runtime(Self::snapshot_from_runtime)
            }
            NisarArchivedRecoveryReceiptStatus::Conflict => {
                return Err(map_v2_store_error(StoreError::AlreadyExists))
            }
            NisarArchivedRecoveryReceiptStatus::Unknown => {}
        }
        let mut store_attempted = false;
        let outcome = (|| {
            self.ensure_task_session_for_admission(&request.source_task_id)?;
            self.with_business_runtime(|runtime| {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                store_attempted = true;
                store
                    .create_nisar_archived_recovery_v2(request)
                    .map_err(map_v2_store_error)?;
                drop(store);
                Self::snapshot_from_runtime(runtime)
            })
        })();
        if outcome.is_err()
            && !store_attempted
            && matches!(
                read_receipt(),
                Ok(NisarArchivedRecoveryReceiptStatus::Committed)
            )
        {
            return self.with_business_runtime(Self::snapshot_from_runtime);
        }
        outcome
    }

    pub fn prepare_nisar_subset_intent_v2(
        &self,
        request: &NisarPrepareSubsetIntentV2Request,
    ) -> Result<NisarPreparedSubsetIntentV2, ApplicationServiceError> {
        Ok(self
            .prepare_nisar_subset_context_v2(request, true)?
            .prepared)
    }

    pub fn preflight_nisar_subset_batch_v2(
        &self,
        request: &NisarPreflightSubsetBatchV2Request,
    ) -> Result<NisarPreflightSubsetBatchV2, ApplicationServiceError> {
        self.ensure_running()?;
        validate_v2_batch_identity(&request.source_batch_id, &request.source_item_ids)?;
        validate_v2_draft_geometry(&request.subset_draft_geometry)?;
        let frozen = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let active = store
                .load_workbench()
                .map_err(ApplicationServiceError::from)?;
            if active
                .as_ref()
                .is_some_and(|slot| slot.batch.batch_id == request.source_batch_id)
            {
                Ok(active)
            } else {
                let Some(batch) = store
                    .load_search_batch(&request.source_batch_id)
                    .map_err(ApplicationServiceError::from)?
                else {
                    return Ok(None);
                };
                Ok(Some(StoredWorkbench {
                    state: SearchWorkbenchState {
                        batch_id: batch.batch_id.clone(),
                        checked_item_ids: Vec::new(),
                        page_index: 0,
                        page_size: 25,
                        revision: 0,
                    },
                    batch,
                }))
            }
        })?;
        let frozen = frozen.ok_or(ApplicationServiceError::NisarSubsetSourceInvalid)?;
        if frozen.batch.batch_id != request.source_batch_id {
            return Err(ApplicationServiceError::NisarSubsetSourceInvalid);
        }
        if frozen.state.revision != request.expected_workbench_revision {
            return Err(ApplicationServiceError::NisarSubsetV2SearchBatchStale);
        }

        let by_id = frozen
            .batch
            .candidates
            .iter()
            .map(|item| (item.item_id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        let mut eligible_items = Vec::with_capacity(request.source_item_ids.len());
        let mut excluded_items = Vec::new();
        for source_item_id in &request.source_item_ids {
            let source_item = by_id
                .get(source_item_id.as_str())
                .copied()
                .ok_or(ApplicationServiceError::NisarSubsetSourceInvalid)?;
            validate_source_projection_v2(source_item)?;
            let source_footprint = source_item
                .footprint_geojson
                .as_ref()
                .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
            let authority = match derive_nisar_subset_geometry_authority_v2(
                source_footprint,
                &request.subset_draft_geometry,
            ) {
                Ok(authority) => authority,
                Err(NisarSubsetPlanV2Error::EmptyEffectiveGeometry) => {
                    excluded_items.push(NisarSubsetBatchExcludedItemV2 {
                        source_item_id: source_item_id.clone(),
                        reason: NisarSubsetBatchExclusionReason::NoOverlap,
                    });
                    continue;
                }
                Err(error) => return Err(map_v2_core_error(error)),
            };
            eligible_items.push(NisarPreflightEligibleItemV2 {
                source_item_id: source_item_id.clone(),
                effective_geometry: authority.effective_geometry,
                available_selections: available_nisar_subset_selections_v2(source_item),
            });
        }
        if eligible_items.is_empty() {
            return Err(ApplicationServiceError::NisarSubsetV2EmptyEffectiveGeometry);
        }
        let common_selections = common_nisar_subset_selections_v2(&eligible_items);
        Ok(NisarPreflightSubsetBatchV2 {
            source_batch_id: request.source_batch_id.clone(),
            workbench_revision: request.expected_workbench_revision,
            eligible_items,
            excluded_items,
            common_selections,
        })
    }

    pub fn prepare_nisar_subset_batch_v2(
        &self,
        request: &NisarPrepareSubsetBatchV2Request,
    ) -> Result<NisarPreparedSubsetBatchV2, ApplicationServiceError> {
        let preflight =
            self.preflight_nisar_subset_batch_v2(&NisarPreflightSubsetBatchV2Request {
                source_batch_id: request.source_batch_id.clone(),
                source_item_ids: request.source_item_ids.clone(),
                expected_workbench_revision: request.expected_workbench_revision,
                subset_draft_geometry: request.subset_draft_geometry.clone(),
            })?;
        let mut intents = Vec::with_capacity(preflight.eligible_items.len());
        let excluded_items = preflight.excluded_items;
        for eligible in preflight.eligible_items {
            let source_item_id = eligible.source_item_id;
            let single = NisarPrepareSubsetIntentV2Request {
                source_batch_id: request.source_batch_id.clone(),
                source_item_id: source_item_id.clone(),
                expected_workbench_revision: request.expected_workbench_revision,
                subset_draft_geometry: request.subset_draft_geometry.clone(),
                selected_frequency: request.selected_frequency.clone(),
                selected_polarizations: request.selected_polarizations.clone(),
            };
            let context = self.prepare_nisar_subset_context_v2(&single, false)?;
            intents.push(context.prepared);
        }
        if intents.is_empty() {
            return Err(ApplicationServiceError::NisarSubsetV2EmptyEffectiveGeometry);
        }
        Ok(NisarPreparedSubsetBatchV2 {
            source_batch_id: request.source_batch_id.clone(),
            workbench_revision: request.expected_workbench_revision,
            intents,
            excluded_items,
        })
    }

    pub fn confirm_nisar_subset_task_v2(
        &self,
        request: &NisarConfirmSubsetTaskV2Request,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 96
            || request.output_root.as_os_str().is_empty()
            || !request.output_root.is_absolute()
            || !(1..=8).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
        {
            return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
        }
        request
            .prepared_intent
            .validate()
            .map_err(|_| ApplicationServiceError::NisarSubsetV2PreparedIntentInvalid)?;
        self.ensure_frozen_selection_for_admission(
            &request.source_batch_id,
            std::slice::from_ref(&request.source_item_id),
        )?;
        let prepare_request = NisarPrepareSubsetIntentV2Request {
            source_batch_id: request.source_batch_id.clone(),
            source_item_id: request.source_item_id.clone(),
            expected_workbench_revision: request.expected_workbench_revision,
            subset_draft_geometry: request
                .prepared_intent
                .requested_geometry
                .canonical_geojson
                .clone(),
            selected_frequency: frequency_name_v2(&request.prepared_intent.selected_frequency)
                .to_owned(),
            selected_polarizations: request.prepared_intent.selected_polarizations.clone(),
        };
        let prepared = self.prepare_nisar_subset_context_v2(&prepare_request, true)?;
        if prepared.prepared.intent.canonical_intent_sha256
            != request.prepared_intent.canonical_intent_sha256
            || prepared.prepared.intent != request.prepared_intent
        {
            return Err(ApplicationServiceError::NisarSubsetV2PreparedIntentIdentityMismatch);
        }
        let mut task_item = prepared.source_item;
        task_item.metadata.insert(
            "source_item_id".to_owned(),
            Value::String(request.source_item_id.clone()),
        );
        task_item.metadata.insert(
            "delivery_kind".to_owned(),
            Value::String("remote_derived_subset_v2".to_owned()),
        );
        embed_nisar_subset_prepared_intent_v2_envelope(
            &mut task_item.metadata,
            &request.prepared_intent,
        )
        .map_err(map_v2_store_error)?;
        let task_request = StoreCreateTaskRequest {
            task_id: request.task_id.clone(),
            batch_id: request.source_batch_id.clone(),
            selected_item_ids: vec![task_item.item_id.clone()],
            checked_item_ids: vec![task_item.item_id.clone()],
            page_index: 0,
            page_size: prepared.source_page_size,
            output_root: request.output_root.clone(),
            layout: request.layout.clone(),
            concurrency: request.concurrency,
            created_at: request.created_at.clone(),
        };
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let stored = store
                .create_paused_task_with_explicit_items(
                    &task_request,
                    request.expected_workbench_revision,
                    std::slice::from_ref(&request.source_item_id),
                    std::slice::from_ref(&task_item),
                )
                .map_err(map_v2_store_error)?;
            if stored.runtime.status != TaskLifecycleStatus::Paused
                || !stored.runtime.task_paused
                || !stored.runtime.queued_item_ids.is_empty()
            {
                return Err(ApplicationServiceError::NisarSubsetV2StorePersistenceFailed);
            }
            drop(store);
            Self::snapshot_from_runtime(runtime)
        })
    }

    pub fn confirm_nisar_subset_batch_task_v2(
        &self,
        request: &NisarConfirmSubsetBatchTaskV2Request,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_running()?;
        let unique_ids = request
            .items
            .iter()
            .map(|item| item.source_item_id.as_str())
            .collect::<BTreeSet<_>>();
        if !valid_catalogue_opaque_id(&request.source_batch_id)
            || !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 96
            || request.items.is_empty()
            || unique_ids.len() != request.items.len()
            || request
                .items
                .iter()
                .any(|item| !valid_nisar_subset_source_item_id(&item.source_item_id))
            || request.output_root.as_os_str().is_empty()
            || !request.output_root.is_absolute()
            || !(1..=8).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
        {
            return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
        }

        let mut source_item_ids = Vec::with_capacity(request.items.len());
        let mut task_items = Vec::with_capacity(request.items.len());
        let mut source_page_size = None;
        for item in &request.items {
            item.prepared_intent
                .validate()
                .map_err(|_| ApplicationServiceError::NisarSubsetV2PreparedIntentInvalid)?;
            let prepare_request = NisarPrepareSubsetIntentV2Request {
                source_batch_id: request.source_batch_id.clone(),
                source_item_id: item.source_item_id.clone(),
                expected_workbench_revision: request.expected_workbench_revision,
                subset_draft_geometry: item
                    .prepared_intent
                    .requested_geometry
                    .canonical_geojson
                    .clone(),
                selected_frequency: frequency_name_v2(&item.prepared_intent.selected_frequency)
                    .to_owned(),
                selected_polarizations: item.prepared_intent.selected_polarizations.clone(),
            };
            let prepared = self.prepare_nisar_subset_context_v2(&prepare_request, false)?;
            if prepared.prepared.intent.canonical_intent_sha256
                != item.prepared_intent.canonical_intent_sha256
                || prepared.prepared.intent != item.prepared_intent
            {
                return Err(ApplicationServiceError::NisarSubsetV2PreparedIntentIdentityMismatch);
            }
            if source_page_size
                .replace(prepared.source_page_size)
                .is_some_and(|page_size| page_size != prepared.source_page_size)
            {
                return Err(ApplicationServiceError::NisarSubsetV2SearchBatchStale);
            }
            let mut task_item = prepared.source_item;
            task_item.metadata.insert(
                "source_item_id".to_owned(),
                Value::String(item.source_item_id.clone()),
            );
            task_item.metadata.insert(
                "delivery_kind".to_owned(),
                Value::String("remote_derived_subset_v2".to_owned()),
            );
            embed_nisar_subset_prepared_intent_v2_envelope(
                &mut task_item.metadata,
                &item.prepared_intent,
            )
            .map_err(map_v2_store_error)?;
            source_item_ids.push(item.source_item_id.clone());
            task_items.push(task_item);
        }

        let selected_item_ids = task_items
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>();
        let task_request = StoreCreateTaskRequest {
            task_id: request.task_id.clone(),
            batch_id: request.source_batch_id.clone(),
            selected_item_ids: selected_item_ids.clone(),
            checked_item_ids: selected_item_ids,
            page_index: 0,
            page_size: source_page_size
                .ok_or(ApplicationServiceError::NisarSubsetV2RequestInvalid)?,
            output_root: request.output_root.clone(),
            layout: request.layout.clone(),
            concurrency: request.concurrency,
            created_at: request.created_at.clone(),
        };
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let stored = store
                .create_paused_task_with_explicit_items(
                    &task_request,
                    request.expected_workbench_revision,
                    &source_item_ids,
                    &task_items,
                )
                .map_err(map_v2_store_error)?;
            if stored.snapshot.candidates.len() != task_items.len()
                || stored.runtime.status != TaskLifecycleStatus::Paused
                || !stored.runtime.task_paused
                || !stored.runtime.queued_item_ids.is_empty()
                || stored.runtime.paused_item_ids.len() != task_items.len()
            {
                return Err(ApplicationServiceError::NisarSubsetV2StorePersistenceFailed);
            }
            drop(store);
            Self::snapshot_from_runtime(runtime)
        })
    }

    pub fn append_nisar_subset_batch_task_v2(
        &self,
        request: &NisarAppendSubsetBatchTaskV2Request,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_running()?;
        if !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 96
            || !valid_catalogue_opaque_id(&request.event_id)
            || parse_normalized_utc(&request.occurred_at).is_none()
        {
            return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
        }
        validate_nisar_append_source_ids(&request.source_item_ids)?;
        let identity = request.receipt_identity();
        let read_receipt = || {
            self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                store
                    .nisar_append_receipt_status(&identity)
                    .map_err(|error| match error {
                        StoreError::NotFound => {
                            ApplicationServiceError::NisarSubsetV2SourceBatchUnavailable
                        }
                        other => map_v2_store_error(other),
                    })
            })
        };
        match read_receipt()? {
            NisarAppendReceiptStatus::Committed => {
                return self.with_business_runtime(Self::snapshot_from_runtime)
            }
            NisarAppendReceiptStatus::Conflict => {
                return Err(map_v2_store_error(StoreError::AlreadyExists))
            }
            NisarAppendReceiptStatus::Unknown => {}
        }

        let mut store_succeeded = false;
        let outcome = self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let authority = load_nisar_append_authority_v2(&store, &request.task_id)?;
            if authority.task.runtime.revision != request.expected_task_revision
                || !matches!(
                    authority.task.runtime.status,
                    TaskLifecycleStatus::Queued
                        | TaskLifecycleStatus::Running
                        | TaskLifecycleStatus::Paused
                )
            {
                return Err(ApplicationServiceError::NisarSubsetV2SearchBatchStale);
            }
            let existing_sources = task_source_item_ids_v2(&authority.task);
            if request
                .source_item_ids
                .iter()
                .any(|source_item_id| existing_sources.contains(source_item_id))
            {
                return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
            }
            let evaluation =
                evaluate_nisar_append_candidates_v2(&authority, &request.source_item_ids)?;
            if !evaluation.excluded_items.is_empty() {
                return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
            }
            let (source_item_ids, task_items, _) =
                build_nisar_append_task_items_v2(&authority, &request.source_item_ids)?;
            if task_items.is_empty() {
                return Err(ApplicationServiceError::NisarSubsetV2EmptyEffectiveGeometry);
            }
            let authoritative_revision = authority.task.runtime.revision;
            let appended = store
                .append_nisar_items_with_authority(
                    &StoreAppendExplicitTaskItemsRequest {
                        task_id: request.task_id.clone(),
                        expected_revision: authoritative_revision,
                        event_id: request.event_id.clone(),
                        occurred_at: request.occurred_at.clone(),
                        source_item_ids,
                        task_items,
                    },
                    &authority.task,
                    &authority.source_batch,
                    &identity,
                )
                .map_err(map_v2_store_error)?;
            store_succeeded = true;
            if appended.runtime.revision <= authoritative_revision
                || appended.runtime.selected_item_ids.len() != appended.snapshot.candidates.len()
            {
                return Err(ApplicationServiceError::NisarSubsetV2StorePersistenceFailed);
            }
            drop(store);
            runtime
                .scheduler
                .as_ref()
                .ok_or(ApplicationServiceError::BusinessRuntimeUnavailable)?
                .wake();
            Self::snapshot_from_runtime(runtime)
        });
        if outcome.is_err()
            && !store_succeeded
            && matches!(read_receipt(), Ok(NisarAppendReceiptStatus::Committed))
        {
            return self.with_business_runtime(Self::snapshot_from_runtime);
        }
        outcome
    }

    pub fn resume_nisar_subset_task(
        &self,
        task_id: &str,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_running()?;
        if !valid_catalogue_opaque_id(task_id) || task_id.len() > 96 {
            return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
        }
        self.ensure_task_session_for_admission(task_id)?;
        let probe = insar_download::disk_guard::default_free_space_probe();
        self.refuse_resume_on_low_disk(task_id, probe.as_ref())?;

        self.with_business_runtime(|runtime| {
            {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let stored = store
                    .load_task_record(task_id)?
                    .ok_or(ApplicationServiceError::NisarSubsetV2SourceBatchUnavailable)?;
                if stored.snapshot.candidates.is_empty()
                    || stored.snapshot.candidates.iter().any(|item| {
                        item.metadata.get("delivery_kind").and_then(Value::as_str)
                            != Some("remote_derived_subset_v2")
                    })
                    || !matches!(stored.runtime.status, TaskLifecycleStatus::Paused)
                {
                    return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
                }

                let occurred_at = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true);
                // PauseTask reaches Store before an active worker necessarily publishes its
                // Interrupted result. Normalize those active items while task_paused is still
                // true so a fast Continue cannot clear the task pause and strand the late result.
                let normalized =
                    if stored.runtime.task_paused && !stored.runtime.active_item_ids.is_empty() {
                        let active_item_ids = stored.runtime.active_item_ids.clone();
                        store.update_task(&StoreUpdateTaskRequest {
                            task_id: task_id.to_owned(),
                            expected_revision: stored.runtime.revision,
                            command: StoreTaskCommand {
                                kind: TaskStateCommandKind::PauseItems,
                                item_ids: active_item_ids,
                            },
                            event_id: format!(
                                "nisar-subset-resume-normalize-items-{task_id}-{}",
                                stored.runtime.revision
                            ),
                            occurred_at: occurred_at.clone(),
                        })?
                    } else {
                        stored
                    };
                let task_resumed = if normalized.runtime.task_paused {
                    store.update_task(&StoreUpdateTaskRequest {
                        task_id: task_id.to_owned(),
                        expected_revision: normalized.runtime.revision,
                        command: StoreTaskCommand {
                            kind: TaskStateCommandKind::ResumeTask,
                            item_ids: Vec::new(),
                        },
                        event_id: format!(
                            "nisar-subset-resume-task-{task_id}-{}",
                            normalized.runtime.revision
                        ),
                        occurred_at: occurred_at.clone(),
                    })?
                } else {
                    normalized
                };
                let paused_item_ids = task_resumed.runtime.paused_item_ids.clone();
                let resumed = if paused_item_ids.is_empty() {
                    task_resumed
                } else {
                    store.update_task(&StoreUpdateTaskRequest {
                        task_id: task_id.to_owned(),
                        expected_revision: task_resumed.runtime.revision,
                        command: StoreTaskCommand {
                            kind: TaskStateCommandKind::ResumeItems,
                            item_ids: paused_item_ids,
                        },
                        event_id: format!(
                            "nisar-subset-resume-items-{task_id}-{}",
                            task_resumed.runtime.revision
                        ),
                        occurred_at,
                    })?
                };
                let accounted = resumed.runtime.queued_item_ids.len()
                    + resumed.runtime.active_item_ids.len()
                    + resumed.runtime.completed_item_ids.len()
                    + resumed.runtime.failed_item_ids.len()
                    + resumed.runtime.skipped_item_ids.len()
                    + resumed.runtime.cancelled_item_ids.len();
                if resumed.runtime.task_paused
                    || !resumed.runtime.paused_item_ids.is_empty()
                    || accounted != resumed.snapshot.candidates.len()
                    || !matches!(
                        resumed.runtime.status,
                        TaskLifecycleStatus::Queued | TaskLifecycleStatus::Running
                    )
                {
                    return Err(ApplicationServiceError::NisarSubsetV2StorePersistenceFailed);
                }
            }
            runtime
                .scheduler
                .as_ref()
                .ok_or(ApplicationServiceError::BusinessRuntimeUnavailable)?
                .wake();
            Self::snapshot_from_runtime(runtime)
        })
    }

    pub fn start_newly_confirmed_task(
        &self,
        task_id: &str,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_running()?;
        if !valid_catalogue_opaque_id(task_id) || task_id.len() > 96 {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "newly confirmed task identity is invalid".to_owned(),
            });
        }
        self.ensure_task_session_for_admission(task_id)?;
        let probe = insar_download::disk_guard::default_free_space_probe();
        self.refuse_resume_on_low_disk(task_id, probe.as_ref())?;

        self.with_business_runtime(|runtime| {
            {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let stored = store.load_task_record(task_id)?.ok_or_else(|| {
                    ApplicationServiceError::InvalidRequest {
                        reason: "newly confirmed task does not exist".to_owned(),
                    }
                })?;
                let exact_subset_identity = stored.snapshot.task_id == task_id
                    && stored.runtime.task_id == task_id
                    && stored.runtime.status == TaskLifecycleStatus::Paused
                    && stored.runtime.task_paused
                    && stored.runtime.queued_item_ids.is_empty()
                    && stored.runtime.active_item_ids.is_empty()
                    && !stored.snapshot.candidates.is_empty()
                    && stored.runtime.paused_item_ids.len() == stored.snapshot.candidates.len()
                    && stored.snapshot.candidates.iter().all(|item| {
                        item.metadata.get("delivery_kind").and_then(Value::as_str)
                            == Some("remote_derived_subset_v2")
                            && item
                                .metadata
                                .get("source_item_id")
                                .and_then(Value::as_str)
                                .is_some_and(|source_item_id| source_item_id == item.item_id)
                            && stored.runtime.paused_item_ids.contains(&item.item_id)
                    });
                if !exact_subset_identity {
                    return Err(ApplicationServiceError::InvalidRequest {
                        reason: "task is not the exact newly confirmed NISAR subset task"
                            .to_owned(),
                    });
                }

                let occurred_at = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true);
                let task_resumed = store.update_task(&StoreUpdateTaskRequest {
                    task_id: task_id.to_owned(),
                    expected_revision: stored.runtime.revision,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::ResumeTask,
                        item_ids: Vec::new(),
                    },
                    event_id: format!(
                        "nisar-subset-start-task-{task_id}-{}",
                        stored.runtime.revision
                    ),
                    occurred_at: occurred_at.clone(),
                })?;
                let paused_item_ids = task_resumed.runtime.paused_item_ids.clone();
                let started = store.update_task(&StoreUpdateTaskRequest {
                    task_id: task_id.to_owned(),
                    expected_revision: task_resumed.runtime.revision,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::ResumeItems,
                        item_ids: paused_item_ids,
                    },
                    event_id: format!(
                        "nisar-subset-start-items-{task_id}-{}",
                        task_resumed.runtime.revision
                    ),
                    occurred_at,
                })?;
                let non_paused_item_count = started.runtime.queued_item_ids.len()
                    + started.runtime.active_item_ids.len()
                    + started.runtime.completed_item_ids.len()
                    + started.runtime.failed_item_ids.len()
                    + started.runtime.skipped_item_ids.len()
                    + started.runtime.cancelled_item_ids.len();
                if started.snapshot.task_id != task_id
                    || started.runtime.task_id != task_id
                    || started.runtime.task_paused
                    || !started.runtime.paused_item_ids.is_empty()
                    || non_paused_item_count != started.snapshot.candidates.len()
                    || !matches!(
                        started.runtime.status,
                        TaskLifecycleStatus::Queued | TaskLifecycleStatus::Running
                    )
                {
                    return Err(ApplicationServiceError::NisarSubsetV2StorePersistenceFailed);
                }
            }
            runtime
                .scheduler
                .as_ref()
                .ok_or(ApplicationServiceError::BusinessRuntimeUnavailable)?
                .wake();
            Self::snapshot_from_runtime(runtime)
        })
    }

    fn prepare_nisar_subset_context_v2(
        &self,
        request: &NisarPrepareSubsetIntentV2Request,
        require_selected: bool,
    ) -> Result<PreparedSubsetContextV2, ApplicationServiceError> {
        self.ensure_running()?;
        if !valid_catalogue_opaque_id(&request.source_batch_id)
            || !valid_nisar_subset_source_item_id(&request.source_item_id)
            || request.selected_polarizations.is_empty()
            || request.selected_polarizations.len() > 4
            || request.selected_polarizations.iter().any(|value| {
                value.is_empty()
                    || value != value.trim()
                    || value.len() > 8
                    || !value.bytes().all(|byte| byte.is_ascii_uppercase())
            })
        {
            return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
        }
        let selected_frequency = parse_v2_frequency(&request.selected_frequency)?;
        validate_v2_draft_geometry(&request.subset_draft_geometry)?;

        let frozen = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let active = store
                .load_workbench()
                .map_err(ApplicationServiceError::from)?;
            if active
                .as_ref()
                .is_some_and(|slot| slot.batch.batch_id == request.source_batch_id)
                || require_selected
            {
                Ok(active)
            } else {
                let Some(batch) = store
                    .load_search_batch(&request.source_batch_id)
                    .map_err(ApplicationServiceError::from)?
                else {
                    return Ok(None);
                };
                Ok(Some(StoredWorkbench {
                    state: SearchWorkbenchState {
                        batch_id: batch.batch_id.clone(),
                        checked_item_ids: Vec::new(),
                        page_index: 0,
                        page_size: 25,
                        revision: 0,
                    },
                    batch,
                }))
            }
        })?;
        let frozen = frozen.ok_or(ApplicationServiceError::NisarSubsetV2SourceBatchUnavailable)?;
        if frozen.batch.batch_id != request.source_batch_id {
            return Err(ApplicationServiceError::NisarSubsetV2SourceBatchUnavailable);
        }
        if frozen.state.revision != request.expected_workbench_revision {
            return Err(ApplicationServiceError::NisarSubsetV2SearchBatchStale);
        }
        let source_item = frozen
            .batch
            .candidates
            .iter()
            .find(|item| item.item_id == request.source_item_id)
            .cloned()
            .ok_or(ApplicationServiceError::NisarSubsetV2SourceItemUnavailable)?;
        if require_selected
            && !frozen
                .state
                .checked_item_ids
                .iter()
                .any(|item_id| item_id == &request.source_item_id)
        {
            return Err(ApplicationServiceError::NisarSubsetV2ItemNotSelected);
        }
        validate_source_projection_v2(&source_item)?;
        validate_v2_frequency_presence(&source_item, &selected_frequency)?;
        let intent = if source_item.provider_id == BHOONIDHI_PROVIDER_ID {
            prepare_bhoonidhi_ssar_subset_intent_v2(
                &source_item,
                request.subset_draft_geometry.clone(),
                selected_frequency,
                request.selected_polarizations.clone(),
            )?
        } else {
            prepare_nisar_subset_intent_v2(
                &source_item,
                AsfNisarSubsetPreparedIntentDraftV2 {
                    requested_geometry: request.subset_draft_geometry.clone(),
                    selected_frequency,
                    selected_polarizations: request.selected_polarizations.clone(),
                },
            )
            .map_err(map_v2_provider_error)?
        };
        intent
            .validate()
            .map_err(|_| ApplicationServiceError::NisarSubsetV2PreparedIntentInvalid)?;
        Ok(PreparedSubsetContextV2 {
            source_item,
            source_page_size: frozen.state.page_size,
            prepared: NisarPreparedSubsetIntentV2 {
                source_batch_id: request.source_batch_id.clone(),
                source_item_id: request.source_item_id.clone(),
                workbench_revision: frozen.state.revision,
                intent,
            },
        })
    }
}

fn validate_nisar_append_task_id(task_id: &str) -> Result<(), ApplicationServiceError> {
    if !valid_catalogue_opaque_id(task_id) || task_id.len() > 96 {
        return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
    }
    Ok(())
}

fn validate_nisar_append_source_ids(
    source_item_ids: &[String],
) -> Result<(), ApplicationServiceError> {
    let unique_ids = source_item_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if source_item_ids.is_empty()
        || unique_ids.len() != source_item_ids.len()
        || source_item_ids
            .iter()
            .any(|item_id| !valid_nisar_subset_source_item_id(item_id))
    {
        return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
    }
    Ok(())
}

fn task_source_item_ids_v2(task: &StoredTask) -> BTreeSet<String> {
    task.snapshot
        .candidates
        .iter()
        .filter_map(|item| {
            item.metadata
                .get("source_item_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

fn load_nisar_append_authority_v2(
    store: &TaskStore,
    task_id: &str,
) -> Result<NisarAppendAuthorityV2, ApplicationServiceError> {
    let (task, source_batch) = store
        .load_task_with_source_batch_snapshot(task_id)
        .map_err(map_v2_store_error)?
        .ok_or(ApplicationServiceError::NisarSubsetV2SourceBatchUnavailable)?;
    if !matches!(
        task.snapshot.provider_id.as_str(),
        NISAR_PROVIDER_ID | BHOONIDHI_PROVIDER_ID
    ) || task.snapshot.candidates.is_empty()
        || task.snapshot.candidates.iter().any(|item| {
            item.provider_id != task.snapshot.provider_id
                || item.metadata.get("delivery_kind").and_then(Value::as_str)
                    != Some("remote_derived_subset_v2")
        })
    {
        return Err(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch);
    }
    if source_batch.provider_id != task.snapshot.provider_id {
        return Err(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch);
    }
    let source_ids = task_source_item_ids_v2(&task);
    if source_ids.len() != task.snapshot.candidates.len()
        || task.snapshot.candidates.iter().any(|item| {
            let source_item_id = item.metadata.get("source_item_id").and_then(Value::as_str);
            source_item_id != Some(item.item_id.as_str())
                || source_item_id.is_none_or(|id| !source_ids.contains(id))
                || !source_batch
                    .candidates
                    .iter()
                    .any(|source| source.item_id == item.item_id)
        })
    {
        return Err(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch);
    }

    let mut shared_intent: Option<NisarSubsetPreparedIntentV2> = None;
    for item in &task.snapshot.candidates {
        validate_source_projection_v2(item)?;
        let intent = parse_nisar_subset_prepared_intent_v2_envelope(&item.metadata)
            .map_err(map_v2_store_error)?
            .ok_or(ApplicationServiceError::NisarSubsetV2PreparedIntentInvalid)?
            .intent()
            .map_err(map_v2_store_error)?;
        if intent.source_product_identity.provider_id != item.provider_id
            || intent.source_product_identity.provider_product_key != item.item_id
        {
            return Err(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch);
        }
        if let Some(shared) = &shared_intent {
            if intent.requested_geometry != shared.requested_geometry
                || intent.selected_frequency != shared.selected_frequency
                || intent.selected_polarizations != shared.selected_polarizations
            {
                return Err(ApplicationServiceError::NisarSubsetV2PreparedIntentIdentityMismatch);
            }
        } else {
            shared_intent = Some(intent);
        }
    }
    Ok(NisarAppendAuthorityV2 {
        task,
        source_batch,
        intent: shared_intent.ok_or(ApplicationServiceError::NisarSubsetV2PreparedIntentInvalid)?,
    })
}

fn evaluate_nisar_append_candidates_v2(
    authority: &NisarAppendAuthorityV2,
    source_item_ids: &[String],
) -> Result<NisarAppendPreflightEvaluationV2, ApplicationServiceError> {
    let by_id = authority
        .source_batch
        .candidates
        .iter()
        .map(|item| (item.item_id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let selected_frequency = frequency_name_v2(&authority.intent.selected_frequency);
    let selected_polarizations = authority
        .intent
        .selected_polarizations
        .iter()
        .collect::<BTreeSet<_>>();
    let requested_geometry = &authority.intent.requested_geometry.canonical_geojson;
    let mut eligible_items = Vec::with_capacity(source_item_ids.len());
    let mut excluded_items = Vec::new();
    for source_item_id in source_item_ids {
        let source_item = by_id
            .get(source_item_id.as_str())
            .copied()
            .ok_or(ApplicationServiceError::NisarSubsetV2SourceItemUnavailable)?;
        validate_source_projection_v2(source_item)?;
        let source_footprint = source_item
            .footprint_geojson
            .as_ref()
            .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
        let geometry_authority =
            match derive_nisar_subset_geometry_authority_v2(source_footprint, requested_geometry) {
                Ok(authority) => authority,
                Err(NisarSubsetPlanV2Error::EmptyEffectiveGeometry) => {
                    excluded_items.push(NisarSubsetBatchExcludedItemV2 {
                        source_item_id: source_item_id.clone(),
                        reason: NisarSubsetBatchExclusionReason::NoOverlap,
                    });
                    continue;
                }
                Err(error) => return Err(map_v2_core_error(error)),
            };
        let available_selections = available_nisar_subset_selections_v2(source_item);
        let Some(selected) = available_selections
            .iter()
            .find(|selection| selection.frequency == selected_frequency)
        else {
            excluded_items.push(NisarSubsetBatchExcludedItemV2 {
                source_item_id: source_item_id.clone(),
                reason: NisarSubsetBatchExclusionReason::FrequencyIncompatible,
            });
            continue;
        };
        let available_polarizations = selected
            .polarizations
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if !selected_polarizations
            .iter()
            .all(|polarization| available_polarizations.contains(polarization.as_str()))
        {
            excluded_items.push(NisarSubsetBatchExcludedItemV2 {
                source_item_id: source_item_id.clone(),
                reason: NisarSubsetBatchExclusionReason::PolarizationIncompatible,
            });
            continue;
        }
        eligible_items.push(NisarPreflightEligibleItemV2 {
            source_item_id: source_item_id.clone(),
            effective_geometry: geometry_authority.effective_geometry,
            available_selections,
        });
    }
    Ok(NisarAppendPreflightEvaluationV2 {
        eligible_items,
        excluded_items,
    })
}

fn build_nisar_append_task_items_v2(
    authority: &NisarAppendAuthorityV2,
    source_item_ids: &[String],
) -> Result<(Vec<String>, Vec<SnapshotItem>, u64), ApplicationServiceError> {
    let mut task_items = Vec::with_capacity(source_item_ids.len());
    for source_item_id in source_item_ids {
        let source_item = authority
            .source_batch
            .candidates
            .iter()
            .find(|item| item.item_id == *source_item_id)
            .cloned()
            .ok_or(ApplicationServiceError::NisarSubsetV2SourceItemUnavailable)?;
        let intent = if source_item.provider_id == BHOONIDHI_PROVIDER_ID {
            prepare_bhoonidhi_ssar_subset_intent_v2(
                &source_item,
                authority
                    .intent
                    .requested_geometry
                    .canonical_geojson
                    .clone(),
                authority.intent.selected_frequency.clone(),
                authority.intent.selected_polarizations.clone(),
            )?
        } else {
            prepare_nisar_subset_intent_v2(
                &source_item,
                AsfNisarSubsetPreparedIntentDraftV2 {
                    requested_geometry: authority
                        .intent
                        .requested_geometry
                        .canonical_geojson
                        .clone(),
                    selected_frequency: authority.intent.selected_frequency.clone(),
                    selected_polarizations: authority.intent.selected_polarizations.clone(),
                },
            )
            .map_err(map_v2_provider_error)?
        };
        if intent.requested_geometry != authority.intent.requested_geometry
            || intent.selected_frequency != authority.intent.selected_frequency
            || intent.selected_polarizations != authority.intent.selected_polarizations
        {
            return Err(ApplicationServiceError::NisarSubsetV2PreparedIntentIdentityMismatch);
        }
        let mut task_item = source_item;
        task_item.metadata.insert(
            "source_item_id".to_owned(),
            Value::String(source_item_id.clone()),
        );
        task_item.metadata.insert(
            "delivery_kind".to_owned(),
            Value::String("remote_derived_subset_v2".to_owned()),
        );
        embed_nisar_subset_prepared_intent_v2_envelope(&mut task_item.metadata, &intent)
            .map_err(map_v2_store_error)?;
        task_items.push(task_item);
    }
    Ok((
        source_item_ids.to_vec(),
        task_items,
        authority.task.runtime.page_size,
    ))
}

pub(crate) fn valid_nisar_subset_source_item_id(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= MAX_NISAR_SUBSET_SOURCE_ITEM_ID_BYTES
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn bhoonidhi_ssar_property_text<'a>(item: &'a SnapshotItem, key: &str) -> Option<&'a str> {
    item.metadata
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && *value == value.trim())
}

fn bhoonidhi_ssar_validation_context_v2(
    item: &SnapshotItem,
) -> Result<NisarSsarRslcValidationContext, ApplicationServiceError> {
    let asset = item
        .asset_access
        .as_ref()
        .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
    let execution = item
        .execution_policy
        .as_ref()
        .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
    let provider_product_id = asset.reference.provider_product_id.as_str();
    let file_name = asset
        .file_name
        .as_deref()
        .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
    let coverage = bhoonidhi_ssar_property_text(item, "Coverage")
        .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
    let provider_partial = match coverage {
        "P" => true,
        "F" => false,
        _ => return Err(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch),
    };

    if item.provider_id != BHOONIDHI_PROVIDER_ID
        || metadata_text(item, "logical_dataset") != Some(BHOONIDHI_LOGICAL_DATASET)
        || metadata_text(item, "native_collection_id") != Some(BHOONIDHI_NATIVE_COLLECTION_ID)
        || metadata_text(item, "collection_profile_id") != Some(BHOONIDHI_PROFILE_ID)
        || metadata_text(item, "processing_level") != Some(BHOONIDHI_REQUIRED_PRODUCT_TYPE)
        || metadata_text(item, "band") != Some("S")
        || metadata_text(item, "primary_asset_key") != Some("data")
        || metadata_text(item, "provider_product_id") != Some(provider_product_id)
        || metadata_text(item, "provider_product_key") != Some(item.item_id.as_str())
        || metadata_text(item, "primary_file_name") != Some(file_name)
        || bhoonidhi_ssar_property_text(item, "ID") != Some(provider_product_id)
        || bhoonidhi_ssar_property_text(item, "Online") != Some("Y")
        || asset.reference.provider_id != BHOONIDHI_PROVIDER_ID
        || asset.reference.collection_id != BHOONIDHI_NATIVE_COLLECTION_ID
        || asset.reference.provider_product_key != item.item_id
        || asset.reference.asset_key != "data"
        || asset.role != AssetRole::Primary
        || asset.descriptor.resolver_id != BHOONIDHI_RESOLVER_POLICY_ID
        || asset.descriptor.scheme != AssetAccessScheme::OauthBearerHttps
        || !asset.descriptor.requires_credentials
        || execution.authority != AssetExecutionAuthority::ResolverAuthoritative
        || execution.policy_id != BHOONIDHI_RESOLVER_POLICY_ID
        || execution.collection_profile_id.as_deref() != Some(BHOONIDHI_PROFILE_ID)
    {
        return Err(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch);
    }

    NisarSsarRslcValidationContext::from_frozen_bhoonidhi_deferred_size(
        provider_product_id.to_owned(),
        file_name.to_owned(),
        bhoonidhi_ssar_property_text(item, "SoftwareVersion")
            .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?,
        bhoonidhi_ssar_property_text(item, "Mode")
            .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?,
        bhoonidhi_ssar_property_text(item, "Polarization")
            .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?,
        provider_partial,
    )
    .map_err(|_| ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)
}

pub(crate) fn validate_bhoonidhi_ssar_frozen_item(
    item: &SnapshotItem,
) -> Result<(), ApplicationServiceError> {
    bhoonidhi_ssar_validation_context_v2(item).map(|_| ())
}

fn bhoonidhi_ssar_source_capability_v2(
    item: &SnapshotItem,
    context: &NisarSsarRslcValidationContext,
) -> Result<NisarSubsetSourceCapabilityV2, ApplicationServiceError> {
    let asset = item
        .asset_access
        .as_ref()
        .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
    let polarizations = context.subset_polarization_labels();
    if polarizations.is_empty() {
        return Err(ApplicationServiceError::NisarSubsetV2UnsupportedPolarization);
    }
    let frequencies = vec![NisarSubsetFrequencyCapabilityV2 {
        frequency: NisarSubsetFrequency::A,
        available_polarizations: polarizations,
    }];
    #[derive(Serialize)]
    struct Evidence<'a> {
        provider_product_key: &'a str,
        stable_asset_key: &'a str,
        product_version: &'a str,
        frequency: &'static str,
        frequencies: &'a [NisarSubsetFrequencyCapabilityV2],
    }
    let bytes = serde_json::to_vec(&Evidence {
        provider_product_key: &asset.reference.provider_product_key,
        stable_asset_key: &asset.reference.stable_asset_key,
        product_version: context.product_version(),
        frequency: "A",
        frequencies: &frequencies,
    })
    .map_err(|_| ApplicationServiceError::NisarSubsetV2PreparedIntentInvalid)?;
    Ok(NisarSubsetSourceCapabilityV2 {
        evidence_sha256: format!("{:x}", Sha256::digest(bytes)),
        frequencies,
    })
}

fn prepare_bhoonidhi_ssar_subset_intent_v2(
    item: &SnapshotItem,
    requested_geometry: Map<String, Value>,
    selected_frequency: NisarSubsetFrequency,
    selected_polarizations: Vec<String>,
) -> Result<NisarSubsetPreparedIntentV2, ApplicationServiceError> {
    if selected_frequency != NisarSubsetFrequency::A {
        return Err(ApplicationServiceError::NisarSubsetV2UnsupportedFrequency);
    }
    let context = bhoonidhi_ssar_validation_context_v2(item)?;
    let asset = item
        .asset_access
        .as_ref()
        .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
    let source_footprint = item
        .footprint_geojson
        .as_ref()
        .cloned()
        .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
    NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: NisarSubsetSourceProductIdentityV2 {
            provider_id: item.provider_id.clone(),
            collection_id: asset.reference.collection_id.clone(),
            collection_profile_id: BHOONIDHI_PROFILE_ID.to_owned(),
            provider_product_id: asset.reference.provider_product_id.clone(),
            provider_product_key: asset.reference.provider_product_key.clone(),
            product_specification_version: context.product_version().to_owned(),
            crid: BHOONIDHI_SSAR_CRID.to_owned(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        },
        source_asset_identity: NisarSubsetSourceAssetIdentityV2 {
            asset_key: asset.reference.asset_key.clone(),
            stable_asset_key: asset.reference.stable_asset_key.clone(),
            file_name: asset
                .file_name
                .clone()
                .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?,
            checksum: projected_source_checksum(asset),
        },
        source_capability: bhoonidhi_ssar_source_capability_v2(item, &context)?,
        source_footprint,
        requested_geometry,
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations,
    })
    .map_err(map_v2_core_error)
}

fn validate_v2_batch_identity(
    source_batch_id: &str,
    source_item_ids: &[String],
) -> Result<(), ApplicationServiceError> {
    let unique_ids = source_item_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if !valid_catalogue_opaque_id(source_batch_id)
        || source_item_ids.is_empty()
        || unique_ids.len() != source_item_ids.len()
        || source_item_ids
            .iter()
            .any(|item_id| !valid_nisar_subset_source_item_id(item_id))
    {
        return Err(ApplicationServiceError::NisarSubsetV2RequestInvalid);
    }
    Ok(())
}

fn available_nisar_subset_selections_v2(item: &SnapshotItem) -> Vec<NisarSubsetSelectionV2> {
    if item.provider_id == BHOONIDHI_PROVIDER_ID {
        // Preflight intentionally maps incompatible Bhoonidhi polarization items
        // to an empty selection list so batch UI can exclude them while explicit
        // prepare rejects with a typed error.
        return bhoonidhi_ssar_validation_context_v2(item)
            .map(|context| {
                vec![NisarSubsetSelectionV2 {
                    frequency: "A".to_owned(),
                    polarizations: context.subset_polarization_labels(),
                }]
            })
            .unwrap_or_default();
    }
    [NisarSubsetFrequency::A, NisarSubsetFrequency::B]
        .into_iter()
        .filter_map(|frequency| {
            let key = match frequency {
                NisarSubsetFrequency::A => "frequency_a_polarization",
                NisarSubsetFrequency::B => "frequency_b_polarization",
            };
            let label = item.metadata.get(key)?.as_str()?;
            let polarizations =
                canonical_nisar_polarizations_for_frequency_label(&frequency, label)?;
            Some(NisarSubsetSelectionV2 {
                frequency: frequency_name_v2(&frequency).to_owned(),
                polarizations,
            })
        })
        .collect()
}

fn common_nisar_subset_selections_v2(
    items: &[NisarPreflightEligibleItemV2],
) -> Vec<NisarSubsetSelectionV2> {
    let Some(first) = items.first() else {
        return Vec::new();
    };
    ["A", "B"]
        .into_iter()
        .filter_map(|frequency| {
            let first_selection = first
                .available_selections
                .iter()
                .find(|selection| selection.frequency == frequency)?;
            let mut common = first_selection
                .polarizations
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            for item in &items[1..] {
                let available = item
                    .available_selections
                    .iter()
                    .find(|selection| selection.frequency == frequency)
                    .map(|selection| {
                        selection
                            .polarizations
                            .iter()
                            .cloned()
                            .collect::<BTreeSet<_>>()
                    })
                    .unwrap_or_default();
                common.retain(|polarization| available.contains(polarization));
            }
            (!common.is_empty()).then(|| NisarSubsetSelectionV2 {
                frequency: frequency.to_owned(),
                polarizations: common.into_iter().collect(),
            })
        })
        .collect()
}

fn parse_v2_frequency(value: &str) -> Result<NisarSubsetFrequency, ApplicationServiceError> {
    match value {
        "A" => Ok(NisarSubsetFrequency::A),
        "B" => Ok(NisarSubsetFrequency::B),
        _ => Err(ApplicationServiceError::NisarSubsetV2UnsupportedFrequency),
    }
}

fn frequency_name_v2(value: &NisarSubsetFrequency) -> &'static str {
    match value {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    }
}

fn validate_v2_draft_geometry(
    geometry: &serde_json::Map<String, Value>,
) -> Result<(), ApplicationServiceError> {
    canonicalize_nisar_subset_geojson_v2(geometry, NisarSubsetGeometryRoleV2::Requested)
        .map(|_| ())
        .map_err(map_v2_core_error)
}

fn validate_source_projection_v2(
    item: &insar_core::SnapshotItem,
) -> Result<(), ApplicationServiceError> {
    if item.provider_id == BHOONIDHI_PROVIDER_ID {
        bhoonidhi_ssar_validation_context_v2(item)?;
        return Ok(());
    }
    validate_source_projection(item)
        .map_err(|_| ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
    let asset = item
        .asset_access
        .as_ref()
        .ok_or(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch)?;
    if item.metadata.get("quality_status").and_then(Value::as_str) != Some("Provisional")
        || asset.reference.provider_id != item.provider_id
        || asset.reference.provider_product_key != item.item_id
        || asset
            .file_name
            .as_deref()
            .is_none_or(|value| !value.to_ascii_lowercase().ends_with(".h5"))
    {
        return Err(ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch);
    }
    Ok(())
}

fn validate_v2_frequency_presence(
    item: &insar_core::SnapshotItem,
    frequency: &NisarSubsetFrequency,
) -> Result<(), ApplicationServiceError> {
    if item.provider_id == BHOONIDHI_PROVIDER_ID {
        let context = bhoonidhi_ssar_validation_context_v2(item)?;
        return if *frequency == NisarSubsetFrequency::A
            && !context.subset_polarization_labels().is_empty()
        {
            Ok(())
        } else {
            Err(ApplicationServiceError::NisarSubsetV2UnsupportedFrequency)
        };
    }
    let key = match frequency {
        NisarSubsetFrequency::A => "frequency_a_polarization",
        NisarSubsetFrequency::B => "frequency_b_polarization",
    };
    item.metadata
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(|_| ())
        .ok_or(ApplicationServiceError::NisarSubsetV2UnsupportedFrequency)
}

fn map_v2_core_error(error: NisarSubsetPlanV2Error) -> ApplicationServiceError {
    match error {
        NisarSubsetPlanV2Error::GeometryInvalid {
            geometry: NisarSubsetGeometryRoleV2::Requested,
            ..
        } => ApplicationServiceError::NisarSubsetV2DraftInvalid,
        NisarSubsetPlanV2Error::AntimeridianUnsupported { .. } => {
            ApplicationServiceError::NisarSubsetV2AntimeridianUnsupported
        }
        NisarSubsetPlanV2Error::EmptyEffectiveGeometry => {
            ApplicationServiceError::NisarSubsetV2EmptyEffectiveGeometry
        }
        NisarSubsetPlanV2Error::PolarizationInvalid(_) => {
            ApplicationServiceError::NisarSubsetV2UnsupportedPolarization
        }
        NisarSubsetPlanV2Error::DependencyGraphInvalid(_) => {
            ApplicationServiceError::NisarSubsetV2DependencyGraphInvalid
        }
        NisarSubsetPlanV2Error::DependencyClosureInvalid(_) => {
            ApplicationServiceError::NisarSubsetV2DependencyClosureInvalid
        }
        _ => ApplicationServiceError::NisarSubsetV2PlanValidationFailed,
    }
}

fn map_v2_provider_error(error: insar_core::ContractError) -> ApplicationServiceError {
    match error.code.as_str() {
        "NISAR_SUBSET_ANTIMERIDIAN_UNSUPPORTED" => {
            ApplicationServiceError::NisarSubsetV2AntimeridianUnsupported
        }
        "NISAR_SUBSET_EFFECTIVE_GEOMETRY_EMPTY" => {
            ApplicationServiceError::NisarSubsetV2EmptyEffectiveGeometry
        }
        "NISAR_SUBSET_DEPENDENCY_CLOSURE_INVALID" => {
            ApplicationServiceError::NisarSubsetV2DependencyClosureInvalid
        }
        "NISAR_SUBSET_POLARIZATION_UNSUPPORTED" => {
            ApplicationServiceError::NisarSubsetV2UnsupportedPolarization
        }
        "NISAR_SUBSET_NOT_SUPPORTED" => {
            ApplicationServiceError::NisarSubsetV2SourceIdentityMismatch
        }
        _ => ApplicationServiceError::NisarSubsetV2PlanValidationFailed,
    }
}

fn map_v2_store_error(error: StoreError) -> ApplicationServiceError {
    match error {
        StoreError::RevisionConflict => ApplicationServiceError::NisarSubsetV2SearchBatchStale,
        StoreError::AlreadyExists => ApplicationServiceError::NisarSubsetV2TaskConflict,
        StoreError::NisarSubsetPreparedIntentInvalid => {
            ApplicationServiceError::NisarSubsetV2PreparedIntentInvalid
        }
        StoreError::NisarSubsetPreparedIntentIdentityMismatch
        | StoreError::NisarSubsetPreparedIntentAlreadyFrozen => {
            ApplicationServiceError::NisarSubsetV2PreparedIntentIdentityMismatch
        }
        _ => ApplicationServiceError::NisarSubsetV2StorePersistenceFailed,
    }
}

fn valid_request(request: &NisarCreateRemoteSubsetTaskRequest) -> bool {
    valid_catalogue_opaque_id(&request.request_id)
        && valid_catalogue_opaque_id(&request.source_batch_id)
        && valid_catalogue_opaque_id(&request.item_id)
        && valid_catalogue_opaque_id(&request.task_id)
        && request.task_id.len() <= 96
        && request.output_root.is_absolute()
        && !request.output_root.as_os_str().is_empty()
        && (1..=8).contains(&request.concurrency)
        && parse_normalized_utc(&request.created_at).is_some()
        && request.polarization == request.polarization.trim()
}

fn validate_source_projection(
    item: &insar_core::SnapshotItem,
) -> Result<(), ApplicationServiceError> {
    let text = |key: &str| item.metadata.get(key).and_then(Value::as_str);
    if item.provider_id != NISAR_PROVIDER_ID
        || item.metadata.get("logical_dataset").and_then(Value::as_str)
            != Some(NISAR_LOGICAL_DATASET)
        || text("processing_level") != Some(NISAR_REQUIRED_PRODUCT_TYPE)
        || text("production_configuration") != Some("PR")
        || text("crid") != Some(NISAR_P05023_CRID)
        || text("product_specification_version") != Some(NISAR_P05023_PRODUCT_SPECIFICATION_VERSION)
    {
        return Err(ApplicationServiceError::NisarSubsetSourceInvalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        available_nisar_subset_selections_v2, prepare_bhoonidhi_ssar_subset_intent_v2,
        validate_v2_batch_identity, BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_NATIVE_COLLECTION_ID,
        BHOONIDHI_PROFILE_ID, BHOONIDHI_PROVIDER_ID, BHOONIDHI_RESOLVER_POLICY_ID,
    };
    use insar_core::{
        AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy,
        AssetResolverParameters, AssetRole, FrozenAssetAccess, NisarSubsetFrequency, SnapshotItem,
        StableAssetReference,
    };
    use serde_json::{json, Map, Value};

    const SBAND_PRODUCT_ID: &str =
        "NISAR_S1_PR_RSLC_029_053_A_140_2500_CRNA_A_20260831T053450_20260831T053520_P00500_N_P_I_001";

    fn rectangle(west: f64, south: f64, east: f64, north: f64) -> Map<String, Value> {
        json!({
            "type": "Polygon",
            "coordinates": [[[west, south], [east, south], [east, north], [west, north], [west, south]]]
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn sband_item_with_polarization_code(
        filename_polarization: &str,
        provider_polarization: &str,
    ) -> SnapshotItem {
        let product_id = SBAND_PRODUCT_ID.replace("_CRNA_", &format!("_{filename_polarization}_"));
        let item_id = format!(
            "PROVIDER::{BHOONIDHI_PROVIDER_ID}::COLLECTION::{BHOONIDHI_NATIVE_COLLECTION_ID}::ITEM::{product_id}"
        );
        let file_name = format!("{product_id}.h5");
        let stable_asset_key = format!("{item_id}::ASSET::data");
        SnapshotItem {
            item_id: item_id.clone(),
            provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
            display_name: product_id.clone(),
            acquisition_datetime: Some("2026-08-31T05:34:50Z".to_owned()),
            footprint_geojson: Some(rectangle(74.0, 5.0, 78.0, 7.0)),
            metadata: Map::from_iter([
                (
                    "logical_dataset".to_owned(),
                    Value::String(BHOONIDHI_LOGICAL_DATASET.to_owned()),
                ),
                (
                    "native_collection_id".to_owned(),
                    Value::String(BHOONIDHI_NATIVE_COLLECTION_ID.to_owned()),
                ),
                (
                    "collection_profile_id".to_owned(),
                    Value::String(BHOONIDHI_PROFILE_ID.to_owned()),
                ),
                (
                    "processing_level".to_owned(),
                    Value::String("RSLC".to_owned()),
                ),
                ("band".to_owned(), Value::String("S".to_owned())),
                (
                    "primary_asset_key".to_owned(),
                    Value::String("data".to_owned()),
                ),
                (
                    "provider_product_id".to_owned(),
                    Value::String(product_id.clone()),
                ),
                (
                    "provider_product_key".to_owned(),
                    Value::String(item_id.clone()),
                ),
                (
                    "primary_file_name".to_owned(),
                    Value::String(file_name.clone()),
                ),
                (
                    "properties".to_owned(),
                    json!({
                        "ID": product_id,
                        "Online": "Y",
                        "Coverage": "P",
                        "SoftwareVersion": "1.4.00",
                        "Mode": "W",
                        "Polarization": provider_polarization
                    }),
                ),
            ]),
            payload: Map::new(),
            asset_access: Some(FrozenAssetAccess {
                reference: StableAssetReference {
                    provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
                    collection_id: BHOONIDHI_NATIVE_COLLECTION_ID.to_owned(),
                    provider_product_id: product_id,
                    provider_product_key: item_id,
                    canonical_product_key: None,
                    asset_key: "data".to_owned(),
                    stable_asset_key,
                },
                descriptor: AssetAccessDescriptor {
                    resolver_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
                    scheme: AssetAccessScheme::OauthBearerHttps,
                    stable_href: Some("https://bhoonidhi-api.nrsc.gov.in/download".to_owned()),
                    requires_credentials: true,
                    credential_profile: Some("bhoonidhi-default".to_owned()),
                    resolver_parameters: AssetResolverParameters {},
                },
                role: AssetRole::Primary,
                file_name: Some(file_name),
                expected_size_bytes: None,
                checksum_algorithm: None,
                checksum_value: None,
            }),
            execution_policy: Some(AssetExecutionPolicy {
                authority: AssetExecutionAuthority::ResolverAuthoritative,
                policy_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
                collection_profile_id: Some(BHOONIDHI_PROFILE_ID.to_owned()),
            }),
        }
    }

    fn sband_item() -> SnapshotItem {
        sband_item_with_polarization_code("CRNA", "CP")
    }

    #[test]
    fn batch_identity_accepts_two_hundred_selected_scenes() {
        let item_ids = (0..200)
            .map(|index| format!("PROVIDER::asf.nasa.earthdata::ITEM::G{index:010}-ASF"))
            .collect::<Vec<_>>();

        assert!(validate_v2_batch_identity("search-batch-200", &item_ids).is_ok());
    }

    #[test]
    fn batch_identity_accepts_real_bhoonidhi_sband_item_length_without_widening_batch_ids() {
        let item = sband_item();
        assert!(item.item_id.len() > 128);
        assert!(validate_v2_batch_identity("sband-search-batch", &[item.item_id]).is_ok());
        assert!(validate_v2_batch_identity(&"x".repeat(129), &["safe-item".to_owned()]).is_err());
    }

    #[test]
    fn bhoonidhi_crna_preflight_exposes_frequency_a_rh_rv_only() {
        let item = sband_item();
        let selections = available_nisar_subset_selections_v2(&item);
        assert_eq!(selections.len(), 1);
        assert_eq!(selections[0].frequency, "A");
        assert_eq!(selections[0].polarizations, ["RH", "RV"]);
    }

    #[test]
    fn bhoonidhi_dhna_preflight_exposes_hh_hv_with_exact_provider_identity() {
        let item = sband_item_with_polarization_code("DHNA", "DH");
        let selections = available_nisar_subset_selections_v2(&item);
        assert_eq!(selections.len(), 1);
        assert_eq!(selections[0].frequency, "A");
        assert_eq!(selections[0].polarizations, ["HH", "HV"]);

        let intent = prepare_bhoonidhi_ssar_subset_intent_v2(
            &item,
            rectangle(75.0, 5.5, 76.0, 6.5),
            NisarSubsetFrequency::A,
            vec!["HH".to_owned(), "HV".to_owned()],
        )
        .expect("reviewed DHNA S-band item should prepare");
        assert_eq!(intent.selected_polarizations, ["HH", "HV"]);

        for mismatched in ["CP", "DP", "DV", "UNKNOWN"] {
            let item = sband_item_with_polarization_code("DHNA", mismatched);
            assert!(
                available_nisar_subset_selections_v2(&item).is_empty(),
                "{mismatched}"
            );
            assert!(
                prepare_bhoonidhi_ssar_subset_intent_v2(
                    &item,
                    rectangle(75.0, 5.5, 76.0, 6.5),
                    NisarSubsetFrequency::A,
                    vec!["HH".to_owned(), "HV".to_owned()],
                )
                .is_err(),
                "DHNA must reject Provider polarization {mismatched}"
            );
        }
    }

    #[test]
    fn bhoonidhi_crna_prepared_intent_keeps_ssar_identity_and_rejects_frequency_b() {
        let item = sband_item();
        let intent = prepare_bhoonidhi_ssar_subset_intent_v2(
            &item,
            rectangle(75.0, 5.5, 76.0, 6.5),
            NisarSubsetFrequency::A,
            vec!["RV".to_owned(), "RH".to_owned()],
        )
        .expect("reviewed CRNA S-band item should prepare");
        assert_eq!(
            intent.source_product_identity.provider_id,
            BHOONIDHI_PROVIDER_ID
        );
        assert_eq!(
            intent.source_product_identity.collection_id,
            BHOONIDHI_NATIVE_COLLECTION_ID
        );
        assert_eq!(
            intent.source_product_identity.collection_profile_id,
            BHOONIDHI_PROFILE_ID
        );
        assert_eq!(
            intent.source_product_identity.product_specification_version,
            "1.2.1"
        );
        assert_eq!(intent.source_product_identity.crid, "P00500");
        assert_eq!(intent.selected_frequency, NisarSubsetFrequency::A);
        assert_eq!(intent.selected_polarizations, ["RH", "RV"]);
        assert_eq!(
            intent.source_capability.frequencies[0].available_polarizations,
            ["RH", "RV"]
        );

        assert!(prepare_bhoonidhi_ssar_subset_intent_v2(
            &item,
            rectangle(75.0, 5.5, 76.0, 6.5),
            NisarSubsetFrequency::B,
            vec!["RH".to_owned()],
        )
        .is_err());
    }
}
