//! Transport-neutral provider-ready application runtime for EOStage.
//!
//! The selected desktop runtime owns one Store lease, one Provider/Dispatcher
//! and one DownloadScheduler. JSON Lines and Tauri are bounded adapters around
//! this service; neither adapter owns Store state or business operations.

mod account_sessions;
mod admin_boundaries;
mod administrative_boundary_contracts;
mod agent_tools;
mod boundary_service;
mod capability_services;
mod catalogue_intent;
mod china_admin_authority;
mod companion_orbit;
mod datav_geometry_verification;
pub mod dem_limits;
pub mod experiment_projects;
mod external_download_task;
mod gacos_import_service;
mod gacos_import_worker;
mod gacos_result_archive;
mod gacos_result_service;
mod gacos_result_worker;
mod gacos_service;
mod geo_boundaries;
mod hls_catalogue;
mod network_routes;
mod nisar_subset;
mod optical_delivery;
pub mod optical_derived;
mod optical_preparation;
mod resource_capabilities;

pub use experiment_projects::{
    ExperimentProjectExportResult, ExperimentProjectItemView, ExperimentProjectSnapshot,
};
pub use optical_derived::{
    provider_network_policy_from_settings, read_network_policy_for_store_path,
    sign_mpc_optical_request_in_memory, OpticalDerivedCreateTaskRequest,
    OpticalDerivedPrecheckBatchRequest, OpticalDerivedTaskCreatedResult,
};
mod retry_fallback;
mod sentinel2_band_delivery;
mod stac_fixture;
mod ui_smoke_support;
mod unified_search;
mod unified_sources;
mod unified_tasks;
pub use insar_core::UnifiedSearchRequest;
pub use retry_fallback::TaskRetryFallbackOutcome;
pub use unified_tasks::{
    compare_retry_fallback, BatchAllMatchCreateTaskRequest, BatchAllMatchPreview,
    BatchAllMatchPreviewRequest, BatchAllMatchTaskOutcome, BatchSelectionPredicate,
    ExistingDownloadCheckItem, ExistingDownloadMatch, RetryFallbackComparison,
    TaskAdmissionPreflight, TaskOverview, UnifiedCreateTaskOptions, UnifiedTaskAction,
    UnifiedTaskItemAction, ALL_MATCH_PREDICATE_VERSION_V1, MAX_ALL_MATCH_CANDIDATES,
    MAX_ALL_MATCH_EXCLUSIONS,
};

pub use account_sessions::{AccountSessionStatus, AccountState};
pub use admin_boundaries::{
    AdminBoundaryBbox, AdminBoundaryOption, AdminBoundaryOptionsRequest,
    AdminBoundaryOptionsResult, AdminBoundaryResolveRequest, AdminBoundaryResult,
    AdminBoundarySearchRequest, AdminBoundarySearchResult,
};
pub use administrative_boundary_contracts::{
    AdministrativeBoundaryContractError, AdministrativeBoundaryContractErrorCode,
    ChinaAdminChildrenRequest, ChinaAdminGeometryResolveRequest, ChinaAdminGeometryResolver,
    ChinaAdminGeometryResult, ChinaAdminRootsRequest, ChinaAdminUnitListResult,
    ChinaAdminUnitRequest, ChinaAdminUnitResult, GlobalBoundaryCandidate,
    GlobalBoundaryDiscoveryRequest, GlobalBoundaryDiscoveryResolver, GlobalBoundaryResolveRequest,
    GlobalBoundaryResolved,
};
pub use agent_tools::AgentApplication;
pub use capability_services::{
    ApplicationAoiBundleFile, ApplicationAoiImportRequest, ApplicationAoiNormalizeRequest,
};
pub use catalogue_intent::{
    plan_catalogue_intent, plan_catalogue_intent_request,
    ApplicationCatalogIntentSearchAndFreezeRequest, CatalogSearchIntent,
};
pub use china_admin_authority::{
    ChinaAdminAuthority, ChinaAdminAuthorityError, ChinaAdminAuthorityErrorCode,
    ChinaAdminAuthorityGateStatus, ChinaAdminAuthorityStatus,
    ChinaAdminAuthorityVerificationReport, ChinaAdminLevel, ChinaAdminLicenseStatus,
    ChinaAdminSourceKind, ChinaAdminSourceManifest, ChinaAdminSpecialTag, ChinaAdminStructuralGate,
    ChinaAdminUnit, ChinaAdminUnitType, ChinaAdminUnitVerificationEvidence,
    ChinaAdminVerificationCount, CHINA_ADMIN_AUTHORITY_SCHEMA_VERSION,
    CHINA_ADMIN_CONTENT_HASH_SCOPE, CHINA_ADMIN_COUNTRY_ADCODE,
};
pub use datav_geometry_verification::{
    apply_datav_geometry_batch, bounded_exact_fallback_units,
    build_datav_geometry_verification_report, build_datav_geometry_verification_report_with_stats,
    enumerate_datav_geometry_batch_adcodes, enumerate_datav_geometry_expected_units,
    mark_reusable_pass_evidence, verify_datav_geometry_unit, DatavBatchApplyResult,
    DatavBatchResumeState, DatavGeometryBreakdown, DatavGeometryExpectedUnit,
    DatavGeometryGateStatus, DatavGeometryOutcome, DatavGeometryRequestStats,
    DatavGeometryTransportKind, DatavGeometryUnitEvidence, DatavGeometryVerificationError,
    DatavGeometryVerificationReport, DatavTransportRequestEvidence, DatavTransportRequestOutcome,
    DATAV_BATCH_GEOMETRY_RESOLVER_ID, DATAV_BATCH_RESUME_SCHEMA_VERSION,
    DATAV_EXACT_GEOMETRY_RESOLVER_ID, DATAV_GEOMETRY_VERIFICATION_SCHEMA_VERSION,
};
pub use external_download_task::{RecordExternalDownloadTaskRequest, RecordedExternalDownloadTask};
pub use gacos_import_service::GacosCreateImportTaskRequest;
pub use gacos_result_service::{GacosCreateResultTaskRequest, GacosSupplyResultLinkRequest};
pub use gacos_service::{GacosConfirmRequest, GacosRequestError};
pub use geo_boundaries::{
    GeoBoundaryCountriesResult, GeoBoundaryCountryOption, GeoBoundaryLevelOption,
    GeoBoundaryLevelsRequest, GeoBoundaryLevelsResult, GeoBoundaryNotice, GeoBoundaryReleaseInfo,
    GeoBoundaryResolveRequest, GeoBoundaryUnitOption, GeoBoundaryUnitsRequest,
    GeoBoundaryUnitsResult,
};
pub use optical_delivery::{
    HlsCreateBandBatchTaskRequest, HlsCreateBandBatchTaskResult,
    LandsatCreateAssetBatchTaskRequest, LandsatCreateAssetBatchTaskResult,
    LandsatCreateBundleBatchTaskRequest, LandsatCreateBundleBatchTaskResult,
    OpticalDeliveryAdmissionRejection, OpticalDeliveryAdmissionSummary,
    Sentinel2CreateBandBatchTaskRequest, Sentinel2CreateBandBatchTaskResult,
    Sentinel2L1cCreateBandBatchTaskRequest, Sentinel2L1cCreateBandBatchTaskResult,
};

pub use insar_store::{
    canonical_utc_identity_timestamp, SourceMutationStatus, StoreBatchPage, StoreBatchPageRequest,
    StoreBatchProgress, StoreDemConversionSource, StoreDemConversionSourceRequest,
    StoreDemSourceIdentity, StoreSarCacheDecision, StoreTaskEventPage, StoreTaskEventPageRequest,
    StoreTaskHistoryPage, StoreTaskHistoryPageRequest, StoreTaskOutputDirectory,
    StoreTaskOutputDirectoryRequest,
};
pub use nisar_subset::{
    build_nisar_subset_live_frequency_a_hh_descriptor,
    build_nisar_subset_live_frequency_a_hh_hv_descriptor,
    build_nisar_subset_live_frequency_a_multi_pol_descriptor, AsfNisarSubsetLiveDescriptorError,
    AsfNisarSubsetLiveFrequencyAMultiPolDescriptorV1,
    AsfNisarSubsetLiveFrequencyASinglePolDescriptorV1, NisarAppendSubsetBatchTaskV2Request,
    NisarAppendSubsetContextV2, NisarAppendSubsetContextV2Request, NisarArchivedRecoveryContextV2,
    NisarConfirmSubsetBatchItemV2, NisarConfirmSubsetBatchTaskV2Request,
    NisarConfirmSubsetTaskV2Request, NisarCreateArchivedRecoveryV2Request,
    NisarCreateRemoteSubsetTaskRequest, NisarCreateSubsetSuccessorTaskV2Request,
    NisarPreflightEligibleItemV2, NisarPreflightSubsetAppendV2,
    NisarPreflightSubsetAppendV2Request, NisarPreflightSubsetBatchV2,
    NisarPreflightSubsetBatchV2Request, NisarPrepareSubsetBatchV2Request,
    NisarPrepareSubsetIntentV2Request, NisarPreparedSubsetBatchV2, NisarPreparedSubsetIntentV2,
    NisarRemoteSubsetCapability, NisarSubsetBatchExcludedItemV2, NisarSubsetBatchExclusionReason,
    NisarSubsetDependencyEvidenceV2Fixture, NisarSubsetSelectionV2, NISAR_REMOTE_SUBSET_FEATURE_ID,
    NISAR_REMOTE_SUBSET_OFFLINE_POLICY_ID,
};
pub use resource_capabilities::{
    CapabilityRuntimeObservations, EffectiveCapabilityService, EffectiveCapabilityStatus,
    KnownResourceSummary, ResourceDeliveryCapability, ResourceDescriptorKind,
    NISAR_REMOTE_SUBSET_CAPABILITY_ID,
};

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::{DateTime, FixedOffset, SecondsFormat};
pub use insar_core::{
    catalog_search_cache_key, provider_product_key, sar_search_cache_key, stable_asset_key,
    validate_debug_context, validated_authoritative_polygonal_geojson, wrapped_aoi_envelope,
    AppendTaskFromSourceBatchRequest, AppendTaskFromWorkbenchRequest,
    ApplicationBindRegionAoiRequest, ApplicationClearRegionAoiRequest, ApplicationSearchProgress,
    ApplicationSearchProgressStage, AsfSearchStats, AssetAccessDescriptor, AssetAccessScheme,
    AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters, AssetRole, BBox,
    CatalogCollectionDescriptor, CatalogDeliveryModel, CatalogSearchQuery, CatalogSearchResponse,
    CatalogSearchStats, ContractError, CreateTaskFromHistoricalSourceBatchRequest,
    CreateTaskFromWorkbenchRequest, DataSourceKind, DownloadEngineSnapshot, DownloadFailure,
    DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadLayout, DownloadOutcome,
    FrozenAssetAccess, FrozenOrbitAssociationV1, FrozenSearchBatch, FrozenSearchSourceAttempt,
    FrozenSearchSourceAttemptOutcome, FrozenSearchSourceFallbackReason, FrozenSearchStats,
    OrbitFileMetadata, OrbitMatchRequest, OrbitParseRequest, OrbitPlatform, OrbitProductType,
    OrbitSceneInput, PreviewAssetRequest, PreviewAssetRole, PreviewCachePolicy,
    PreviewCredentialRequirement, PreviewDescriptor, PreviewKind, ProductKind, ProviderAsset,
    ProviderCapabilitiesResponse, ProviderNetworkPolicy, ProviderProduct, RegionAoiState,
    ResolvedPreviewAsset, SarDownloadPolicyMode, SarDownloadPolicySettings, SarProduct,
    SarRemoteSearchRequest, SarSearchCountRequest, SearchWorkbenchState, SnapshotItem,
    StableAssetReference, StoreAppendTaskRequest, StoreArchiveTaskRequest, StoreCreateTaskRequest,
    StoreDeleteHistoryRequest, StoreRestoreRequest, StoreRestoreResponse, StoreSearchBatchRequest,
    StoreTaskCommand, StoreTaskPage, StoreTaskPageRequest, StoreTaskSourceBatchPage,
    StoreTaskSourceBatchPageRequest, StoreUpdateTaskRequest, StoreUpdateTaskViewRequest,
    StoreUpdateWorkbenchRequest, TaskConflictFileDeleteResult, TaskConflictFileExpectation,
    TaskConflictFileList, TaskLifecycleStatus, TaskOperationDescriptor, TaskStateCommandKind,
    WorkbenchSelectionCommand, WorkbenchSelectionSummary, WorkspaceContextState,
    WorkspaceProjectState, WorkspaceRegionState,
};
pub use insar_core::{match_orbits, parse_orbit_filenames};
use insar_core::{SearchBatchProgressState, SearchCancelOutcome};
use insar_download::{
    build_cdse_archive_access, build_earth_search_sentinel2_band_cog_item,
    build_landsat_c2_l2_bundle_access, build_planetary_computer_landsat_asset_access,
    build_planetary_computer_reviewed_cog_item, desktop_bhoonidhi_readiness,
    desktop_earthdata_readiness, AssetResolver, BhoonidhiAssetResolver,
    BhoonidhiCredentialReadiness, BhoonidhiSession, BhoonidhiSessionTokenProvider,
    BhoonidhiTokenSnapshot, CatalogPolicyContext, CdseArchiveEvidence, CdseAuthError,
    CdseLoginMaterial, CdseNetworkDiagnosticStage, CdseNetworkDiagnostics, CdseNetworkFoundation,
    CdseOAuthSession, CdseQuicklookClient, CdseQuicklookError, CdseTokenSnapshot,
    DownloadScheduler, DownloadWork, DownloadWorker, EarthdataCredentialReadiness,
    NisarExecutionPlanFreezeAuthority, ProductionDownloadWorker, ReqwestBhoonidhiDownloadProbe,
    ReqwestBhoonidhiTokenEndpoint, ReqwestCdseNetworkProbe, ResolveContext, ResolveError,
    ResolveErrorKind, SharedTaskStore, SystemClock, TransferAdmissionController, TransferOptions,
    WorkerControl, PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID, USGS_LANDSAT_C2_L2_POLICY_ID,
};
use insar_provider::asf::catalog_client::MAX_CATALOGUE_CANDIDATE_RECORDS;
use insar_provider::asf::nisar::{
    NISAR_MAX_ASSET_GRAPH_JSON_BYTES, NISAR_MAX_DEDUPLICATED_ASSETS, NISAR_MAX_FILE_NAME_BYTES,
    NISAR_MAX_LOCATOR_BYTES, NISAR_MAX_MEDIA_TYPE_BYTES, NISAR_MAX_NATIVE_ROLES,
    NISAR_MAX_NATIVE_ROLE_BYTES, NISAR_MAX_RAW_RESOURCE_ENTRIES,
};
use insar_provider::bhoonidhi::{BhoonidhiAccessTokenProvider, BhoonidhiCatalogClient};
use insar_provider::catalog::StacOfflineFixtureAdmission;
use insar_provider::catalog::{
    alos2_expected_polarizations, CollectionProfileRegistry, DatasetRoutePolicy,
    DatasetRoutePolicyRegistry, EarthSearchSentinel2BandManifest,
    EarthSearchSentinel2CatalogClient, MpcSentinel2BandManifest, MpcSentinel2CatalogClient,
    NisarRslcProductSpecificationProfileRegistry, RouteUse, ALOS2_COLLECTION_SHORT_NAME,
    ALOS2_COLLECTION_VERSION, ALOS2_FILENAME_RULE_ID, ALOS2_IDENTITY_RULE_ID,
    ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID,
    ALOS2_RESOLVER_POLICY_ID, BHOONIDHI_FILENAME_RULE_ID, BHOONIDHI_IDENTITY_RULE_ID,
    BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROFILE_ID,
    BHOONIDHI_PROVIDER_ID, BHOONIDHI_REQUIRED_PRODUCT_TYPE, BHOONIDHI_RESOLVER_POLICY_ID,
    LANDSAT_C2_L2_LOGICAL_DATASET, LANDSAT_C2_L2_PROVIDER_ID, LANDSAT_C2_L2_SCENE_PROFILE_ID,
    NISAR_BAND, NISAR_COLLECTION_SHORT_NAME, NISAR_COLLECTION_VERSION, NISAR_FILENAME_RULE_ID,
    NISAR_FLIGHT_DIRECTIONS, NISAR_FRAME_COVERAGES, NISAR_FREQUENCY_A_POLARIZATIONS,
    NISAR_FREQUENCY_B_POLARIZATIONS, NISAR_IDENTITY_RULE_ID, NISAR_LOGICAL_DATASET,
    NISAR_LOOK_DIRECTIONS, NISAR_NATIVE_COLLECTION_ID, NISAR_PRODUCTION_CONFIGURATION,
    NISAR_PROFILE_ID, NISAR_PROVIDER_ID, NISAR_QUALITY_STATUS, NISAR_RANGE_BANDWIDTHS,
    NISAR_REQUIRED_PRODUCT_TYPE, NISAR_RESOLVER_POLICY_ID, NISAR_SCIENCE_LEVEL,
    USGS_LANDSAT_C2_L2_LEGACY_SCENE_FILENAME_RULE_ID,
};
use insar_provider::cdse::archive_filename::archive_filename_from_odata_name_for_rule;
use insar_provider::planetary_computer::{
    PlanetaryComputerError, PlanetaryComputerLandsatClient, PlanetaryComputerLandsatManifest,
};
use insar_provider::planetary_computer_asset::{
    sentinel2_l2a_reflectance_band_resolution, validate_planetary_computer_reviewed_asset_identity,
    PlanetaryComputerAssetSigner, PlanetaryComputerResolvedAssetEvidence,
    PlanetaryComputerReviewedAssetClass, PlanetaryComputerReviewedAssetEvidence,
    MPC_COPDEM_GLO30_COLLECTION_ID, MPC_COPDEM_GLO30_LOGICAL_DATASET, MPC_COPDEM_GLO30_PROFILE_ID,
    MPC_REVIEWED_STAC_IDENTITY_RULE_ID, MPC_SENTINEL1_GRD_COLLECTION_ID,
    MPC_SENTINEL1_GRD_LOGICAL_DATASET, MPC_SENTINEL1_GRD_PROFILE_ID,
    PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use insar_provider::preview::{
    resolve_preview_candidate, PreviewAssetError, PreviewHttpClient, PreviewProductClass,
};
use insar_provider::usgs_m2m::{
    UsgsM2mApplicationToken, UsgsM2mDownloadClient, UsgsM2mEndpointClass, UsgsM2mError,
    UsgsM2mErrorCode, UsgsM2mFullBundleOption, UsgsM2mLoginMaterial, UsgsM2mSceneSearchRouter,
    UsgsM2mSession, UsgsM2mSessionStatus as ProviderUsgsM2mSessionStatus,
};
use insar_service::{
    CancellationRegistry, CatalogSearchStreamOutcome, CatalogSearchStreamSink,
    CatalogSourceAttempt, CatalogSourceAttemptOutcome, CatalogSourceFallbackReason, Dispatcher,
    LiveAsfSearch, LiveCatalogSearch,
};
use insar_store::{
    BurstAppendReceiptStatus, BurstSuccessorReceiptStatus, HistoricalSuccessorReceiptStatus,
    ReacquireEventStatus, SourceAppendReceiptStatus, StoreAppendExplicitTaskItemsRequest,
    StoreBurstAppendIdentity, StoreBurstSuccessorIdentity,
    StoreCreateHistoricalExplicitTaskRequest, StoreError, StoreHealth,
    StoreSearchBatchStagingRequest, StoreStandaloneAoi, StoreStandaloneAoiInput,
    StoreWorkbenchPageRequest, TaskStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
pub use ui_smoke_support::{with_ui_smoke_loopback_runtime, UiSmokeLoopbackCatalog};
use url::Url;
use uuid::Uuid;

/// Capability advertised by a Sidecar that opens and verifies the Store before
/// reporting startup readiness.
pub const STORE_STARTUP_READINESS_CAPABILITY: &str = "store_startup_readiness_v1";

/// The read-only wire remains compatible while this transport-neutral service
/// grows bounded provider/task operations behind named adapters.
pub const APPLICATION_SERVICE_CONTRACT_VERSION: &str = "application_service_readonly_v1";

const SCHEDULER_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Session-only CDSE network route selection. It is intentionally separate
/// from persisted task and scientific data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CdseNetworkMode {
    System,
    Direct,
}

impl CdseNetworkMode {
    fn policy(self) -> Result<ProviderNetworkPolicy, ApplicationServiceError> {
        match self {
            Self::System => Ok(ProviderNetworkPolicy::system()),
            Self::Direct => Ok(ProviderNetworkPolicy::direct()),
        }
    }

    /// Legacy wire projection. The frozen legacy UI parses only "system" and
    /// "direct" and only needs to know the route goes through a proxy, so a
    /// manual proxy is reported as "system"; the new settings page reads all
    /// three modes from the network settings commands.
    const fn from_policy(policy: ProviderNetworkPolicy) -> Self {
        match policy.route() {
            insar_core::ProxyPolicy::System | insar_core::ProxyPolicy::Manual(_) => Self::System,
            insar_core::ProxyPolicy::Direct => Self::Direct,
        }
    }
}

/// Write-only desktop input. Do not derive `Debug`, `Clone`, or `Serialize`:
/// the fields are moved immediately into `CdseLoginMaterial`, which zeroizes
/// them on drop.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CdseLoginRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub totp: Option<String>,
}

/// Redacted session state safe to return through the desktop bridge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CdseSessionStatus {
    pub network_mode: CdseNetworkMode,
    pub authenticated: bool,
    pub refresh_available: bool,
    pub expires_in_seconds: Option<u64>,
}

/// Credential-free diagnostic output. It deliberately contains only the
/// requested route and a coarse transport stage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CdseNetworkDiagnosticResult {
    pub network_mode: CdseNetworkMode,
    pub stage: String,
}

/// A typed request for one quicklook frozen in a persisted SearchBatch.
/// The UI never supplies a remote URL or credential to this boundary.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CdseQuicklookRequest {
    pub request_id: String,
    pub batch_id: String,
    pub item_id: String,
}

/// Runtime-only image data safe to send over the desktop bridge. This value is
/// intentionally never persisted in the Store, task snapshot, or session.
#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub struct CdseQuicklookResponse {
    pub media_type: String,
    pub data_url: String,
    pub byte_length: usize,
}

/// Resolve one frozen CDSE Sentinel-2 product to the exact equivalent MPC STAC
/// Item used only for map rendering. No provider asset URL, credential or
/// download authority crosses the desktop boundary.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sentinel2MapPreviewRequest {
    pub request_id: String,
    pub batch_id: String,
    pub item_id: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Sentinel2MapPreviewResponse {
    pub product_key: String,
    pub map_item_id: String,
}

/// Write one desktop-controlled Preview data URL to a user-selected directory.
/// The renderer never receives a filesystem handle; validation and the atomic
/// write stay inside the application service boundary.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewSaveRequest {
    pub data_url: String,
    pub output_dir: PathBuf,
    pub file_name: String,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewSaveResponse {
    pub path: String,
    pub file_name: String,
    pub bytes_written: u64,
}

enum FrozenPreviewPlan {
    Direct {
        product_key: String,
        product_class: PreviewProductClass,
        assets: Vec<ProviderAsset>,
    },
    Cdse,
    Landsat,
}

/// Write-only USGS input. It is consumed into zeroizing Provider types and is
/// never serializable back to a desktop client.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsgsM2mLoginRequest {
    pub username: String,
    pub application_token: String,
}

/// Secret-free state for the one process-local USGS M2M session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsgsM2mDesktopSessionStatus {
    pub state: String,
    pub authenticated: bool,
}

/// Secret-free state for the process-local Bhoonidhi OAuth session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BhoonidhiDesktopSessionStatus {
    pub authenticated: bool,
    pub refresh_available: bool,
    pub expires_in_seconds: Option<u64>,
}

/// Frozen identifiers only. React cannot provide a browse locator or header.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandsatBrowseRequest {
    pub request_id: String,
    pub batch_id: String,
    pub item_id: String,
}

/// Runtime-only browse image returned by the bounded desktop command.
#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub struct LandsatBrowseResponse {
    pub media_type: String,
    pub data_url: String,
    pub byte_length: usize,
}

/// Frozen identifiers only. React cannot provide an asset locator or header.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandsatAssetDetailsRequest {
    pub request_id: String,
    pub batch_id: String,
    pub item_id: String,
}

/// Safe display metadata for one provider-owned Landsat asset. Runtime
/// locators and SAS material are intentionally absent from this boundary.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LandsatAssetDetail {
    pub asset_key: String,
    pub file_name: String,
    pub role: String,
    pub size_bytes: u64,
}

/// Disposable read-only projection of the exact frozen Microsoft scene.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LandsatAssetDetailsResponse {
    pub product_key: String,
    pub display_id: String,
    pub processing_level: String,
    pub total_size_bytes: u64,
    pub assets: Vec<LandsatAssetDetail>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BhoonidhiAssetSizeRequest {
    pub request_id: String,
    pub batch_id: String,
    pub item_id: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BhoonidhiAssetSizeResponse {
    pub product_key: String,
    pub size_bytes: u64,
}

#[doc(hidden)]
pub struct LandsatBrowseImage {
    media_type: String,
    bytes: Vec<u8>,
}

impl LandsatBrowseImage {
    #[doc(hidden)]
    pub fn from_parts(media_type: impl Into<String>, bytes: Vec<u8>) -> Option<Self> {
        let media_type = media_type.into();
        (!bytes.is_empty()
            && bytes.len() <= 16 * 1024 * 1024
            && matches!(
                media_type.as_str(),
                "image/jpeg" | "image/png" | "image/webp"
            ))
        .then_some(Self { media_type, bytes })
    }
}

#[doc(hidden)]
pub trait LiveLandsatBrowse: Send + Sync {
    fn fetch_landsat_browse(
        &self,
        query: &CatalogSearchQuery,
        entity_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<LandsatBrowseImage>, UsgsM2mError>;
}

/// Frozen scene and local task intent only. Option identity and executable
/// authority are derived inside the application runtime.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandsatCreateBundleTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    pub item_id: String,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LandsatCreateAssetTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    pub item_id: String,
    #[serde(default)]
    pub asset_keys: Vec<String>,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReviewedMpcCreateAssetTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    pub item_id: String,
    pub asset_keys: Vec<String>,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

#[doc(hidden)]
pub trait LiveReviewedMpcAssetEvidence: Send + Sync {
    fn resolve_reviewed_asset(
        &self,
        class: PlanetaryComputerReviewedAssetClass,
        provider_product_id: &str,
        asset_key: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerResolvedAssetEvidence, ContractError>;
}

impl LiveReviewedMpcAssetEvidence for PlanetaryComputerAssetSigner {
    fn resolve_reviewed_asset(
        &self,
        class: PlanetaryComputerReviewedAssetClass,
        provider_product_id: &str,
        asset_key: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerResolvedAssetEvidence, ContractError> {
        PlanetaryComputerAssetSigner::resolve_reviewed_asset(
            self,
            class,
            provider_product_id,
            asset_key,
            cancelled,
        )
    }
}

/// Re-resolve one frozen Microsoft Sentinel-2 search-only result against the
/// authoritative CDSE catalogue before creating an executable SAFE archive task.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sentinel2CreateArchiveTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    pub item_id: String,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

const MAX_SENTINEL2_ARCHIVE_BATCH_ITEMS: usize = 250;
const SENTINEL2_ARCHIVE_BATCH_LOOKUP_CONCURRENCY: usize = 4;

enum Sentinel2ArchiveAdmissionOutcome {
    Admitted(Box<SnapshotItem>),
    Rejected(&'static str),
}

struct Sentinel2ArchiveBatchLookupContext {
    dispatcher: Arc<Dispatcher>,
    route_use: RouteUse,
    live_acceptance_candidate: bool,
    source_query: CatalogSearchQuery,
    lookup_request_id: String,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sentinel2CreateArchiveBatchTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    /// Empty item_ids means: consume the Store-owned checked set at this
    /// exact revision. Non-empty item_ids remains an explicit selection.
    pub expected_workbench_revision: u64,
    pub item_ids: Vec<String>,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

pub type Sentinel2ArchiveAdmissionRejection = OpticalDeliveryAdmissionRejection;
pub type Sentinel2ArchiveAdmissionSummary = OpticalDeliveryAdmissionSummary;
pub type Sentinel2CreateArchiveBatchTaskResult = LandsatCreateAssetBatchTaskResult;
pub use optical_preparation::{OpticalPreparationPhase, OpticalPreparationProgress};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sentinel2CreateBandTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    pub item_id: String,
    pub asset_keys: Vec<String>,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

#[doc(hidden)]
pub trait LiveSentinel2BandEvidence: Send + Sync {
    fn resolve_band_manifest(
        &self,
        safe_name: &str,
        acquisition_datetime: Option<&str>,
        asset_keys: &[String],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<MpcSentinel2BandManifest, ContractError>;

    fn probe_band_asset(
        &self,
        provider_product_id: &str,
        asset_key: &str,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerReviewedAssetEvidence, ContractError>;

    fn resolve_earth_search_band_manifest(
        &self,
        _item_id: &str,
        _safe_name: &str,
        _acquisition_datetime: Option<&str>,
        _asset_keys: &[String],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<EarthSearchSentinel2BandManifest, ContractError> {
        Err(ContractError::new(
            "EARTH_SEARCH_BAND_MANIFEST_UNCONFIGURED",
            "Element 84 Earth Search Sentinel-2 波段解析未配置",
            Map::new(),
        )
        .expect("static contract error"))
    }

    fn probe_earth_search_band_asset(
        &self,
        _provider_product_id: &str,
        _asset_key: &str,
        _stable_href: &str,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerReviewedAssetEvidence, ContractError> {
        Err(ContractError::new(
            "EARTH_SEARCH_BAND_PROBE_UNCONFIGURED",
            "Element 84 Earth Search Sentinel-2 波段探测未配置",
            Map::new(),
        )
        .expect("static contract error"))
    }
}

pub(crate) struct Sentinel2BandEvidenceRuntime {
    catalog: MpcSentinel2CatalogClient,
    signer: PlanetaryComputerAssetSigner,
    earth_search: EarthSearchSentinel2CatalogClient,
}

impl Sentinel2BandEvidenceRuntime {
    pub(crate) fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, ContractError> {
        Ok(Self {
            catalog: MpcSentinel2CatalogClient::new(network_policy)?,
            signer: PlanetaryComputerAssetSigner::new(network_policy)?,
            earth_search: EarthSearchSentinel2CatalogClient::new(network_policy)?,
        })
    }
}

impl LiveSentinel2BandEvidence for Sentinel2BandEvidenceRuntime {
    fn resolve_band_manifest(
        &self,
        safe_name: &str,
        acquisition_datetime: Option<&str>,
        asset_keys: &[String],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<MpcSentinel2BandManifest, ContractError> {
        self.catalog.resolve_analysis_band_manifest(
            safe_name,
            acquisition_datetime,
            asset_keys,
            cancelled,
        )
    }

    fn probe_band_asset(
        &self,
        provider_product_id: &str,
        asset_key: &str,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerReviewedAssetEvidence, ContractError> {
        self.signer.probe_reviewed_asset(
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog,
            provider_product_id,
            asset_key,
            stable_href,
            cancelled,
        )
    }

    fn resolve_earth_search_band_manifest(
        &self,
        item_id: &str,
        safe_name: &str,
        acquisition_datetime: Option<&str>,
        asset_keys: &[String],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<EarthSearchSentinel2BandManifest, ContractError> {
        self.earth_search.resolve_analysis_band_manifest_by_item_id(
            item_id,
            safe_name,
            acquisition_datetime,
            asset_keys,
            cancelled,
        )
    }

    fn probe_earth_search_band_asset(
        &self,
        provider_product_id: &str,
        asset_key: &str,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerReviewedAssetEvidence, ContractError> {
        self.earth_search.probe_public_band_asset(
            provider_product_id,
            asset_key,
            stable_href,
            cancelled,
        )
    }
}

pub use insar_core::BurstAssetPolicy;

/// Creates executable task-scoped Burst assets while retaining the immutable
/// source SearchBatch as the task's historical batch identity.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BurstCreateAssetTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    pub selected_product_ids: Vec<String>,
    pub asset_policy: BurstAssetPolicy,
    pub expected_workbench_revision: u64,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BurstAppendAssetTaskRequest {
    pub request_id: String,
    pub task_id: String,
    pub selected_product_ids: Vec<String>,
    pub expected_task_revision: u64,
    pub event_id: String,
    pub occurred_at: String,
}

impl BurstAppendAssetTaskRequest {
    fn receipt_identity(&self) -> StoreBurstAppendIdentity {
        StoreBurstAppendIdentity {
            request_id: self.request_id.clone(),
            task_id: self.task_id.clone(),
            selected_product_ids: self.selected_product_ids.clone(),
            expected_task_revision: self.expected_task_revision,
            event_id: self.event_id.clone(),
            occurred_at: self.occurred_at.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BurstCreateHistoricalAssetTaskRequest {
    pub request_id: String,
    pub source_task_id: String,
    pub expected_source_task_revision: u64,
    pub selected_product_ids: Vec<String>,
    pub new_task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

impl BurstCreateHistoricalAssetTaskRequest {
    fn receipt_identity(&self) -> StoreBurstSuccessorIdentity {
        StoreBurstSuccessorIdentity {
            request_id: self.request_id.clone(),
            source_task_id: self.source_task_id.clone(),
            expected_source_task_revision: self.expected_source_task_revision,
            selected_product_ids: self.selected_product_ids.clone(),
            new_task_id: self.new_task_id.clone(),
            output_root: self.output_root.clone(),
            layout: self.layout.clone(),
            concurrency: self.concurrency,
            created_at: self.created_at.clone(),
        }
    }
}

/// Desktop-only query input. This borrows the same request types used for the
/// writes and is not part of the generated sidecar protocol.
pub enum SourceMutationStatusQuery<'a> {
    SourceAppend(&'a AppendTaskFromSourceBatchRequest),
    HistoricalSuccessor(&'a CreateTaskFromHistoricalSourceBatchRequest),
    BurstAppend(&'a BurstAppendAssetTaskRequest),
    BurstSuccessor(&'a BurstCreateHistoricalAssetTaskRequest),
    NisarAppend(&'a NisarAppendSubsetBatchTaskV2Request),
    NisarSuccessor(&'a NisarCreateSubsetSuccessorTaskV2Request),
    NisarArchivedRecovery(&'a NisarCreateArchivedRecoveryV2Request),
    Reacquire(&'a StoreUpdateTaskRequest),
}

pub use insar_core::{ASF_BURST_ASSET_EXECUTION_POLICY_ID, ASF_BURST_COLLECTION_PROFILE_ID};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceOpenRequest {
    pub request_id: String,
    pub workspace_id: String,
    pub workspace_root: PathBuf,
    pub workspace_name: String,
    #[serde(default)]
    pub expected_revision: Option<u64>,
}

pub type WorkspaceCreateRequest = WorkspaceOpenRequest;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePrepareLocationRequest {
    pub request_id: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePreparedLocation {
    pub path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceNamedNodeRequest {
    pub request_id: String,
    pub parent_id: String,
    pub expected_revision: u64,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSelectNodeRequest {
    pub request_id: String,
    pub parent_id: String,
    pub expected_revision: u64,
    pub node_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DemTaskOperation {
    Inspect,
    PrepareSarscape,
    Convert,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemTaskPlanRequest {
    pub request_id: String,
    pub task_id: String,
    pub region_id: String,
    pub resource_id: String,
    pub capability_id: String,
    pub dataset: String,
    pub provider: String,
    pub operation: DemTaskOperation,
    pub input_path: PathBuf,
    pub output_root: PathBuf,
    #[serde(default)]
    pub source_vertical_datum: Option<String>,
    #[serde(default)]
    pub target_vertical_datum: Option<String>,
    #[serde(default)]
    pub output_mode: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemTaskPlan {
    pub request_id: String,
    pub task_id: String,
    pub region_id: String,
    pub resource_id: String,
    pub capability_id: String,
    pub dataset: String,
    pub provider: String,
    pub operation: DemTaskOperation,
    pub input_path: PathBuf,
    pub output_root: PathBuf,
    #[serde(default)]
    pub source_vertical_datum: Option<String>,
    #[serde(default)]
    pub target_vertical_datum: Option<String>,
    #[serde(default)]
    pub output_mode: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemTaskRequest {
    pub plan: DemTaskPlan,
    pub concurrency: u64,
}

/// Product-owned DEM scheduler defaults.  This is deliberately persisted next
/// to the Store rather than in the v6 user-data schema: changing the default
/// must not trigger a Store migration or become mutable task execution state.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DemTaskSettings {
    pub max_concurrent_tasks: u64,
    pub max_concurrent_conversions: u64,
}

impl Default for DemTaskSettings {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 2,
            max_concurrent_conversions: 1,
        }
    }
}

/// A context-free DEM operation.  Unlike a formal project-bound task this
/// route is allowed to inspect or convert a user-selected local raster before
/// a Workspace/Project/Region exists.  It never receives credentials and it
/// never owns Store task state.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandaloneDemRequest {
    pub request_id: String,
    pub dataset: String,
    pub provider: String,
    pub operation: DemTaskOperation,
    pub input_path: PathBuf,
    pub output_root: PathBuf,
    #[serde(default)]
    pub bbox: Option<Vec<f64>>,
    #[serde(default)]
    pub source_vertical_datum: Option<String>,
    #[serde(default)]
    pub target_vertical_datum: Option<String>,
    #[serde(default)]
    pub output_mode: Option<String>,
}

/// A context-free online DEM task.  The bbox is the task's frozen request
/// scope; it is intentionally not read from Workspace/Project/Region state.
/// The resulting task is still owned by Store/Scheduler and therefore uses the
/// same bounded task-page and task-level mutation surface as project-bound
/// work.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandaloneDemTaskRequest {
    pub request_id: String,
    pub task_id: String,
    pub dataset: String,
    pub provider: String,
    pub bbox: Vec<f64>,
    pub output_root: PathBuf,
    pub convert: bool,
    pub concurrency: u64,
    pub created_at: String,
}

/// A context-free DEM task whose spatial authority is the exact AOI currently
/// persisted by the ApplicationService Store. The transport sends the exact
/// geometry and display identity only as a stale-view guard; request windows
/// are always derived natively from the Store-owned AOI.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandaloneDemAoiTaskRequest {
    pub request_id: String,
    pub task_id: String,
    pub dataset: String,
    pub provider: String,
    pub aoi_geojson: Map<String, Value>,
    pub aoi_name: String,
    pub output_root: PathBuf,
    pub convert: bool,
    pub concurrency: u64,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneImportRequest {
    pub source: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrbitDirectoryMatchRequest {
    pub orbit_dir: PathBuf,
    pub scenes: Vec<OrbitSceneInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GacosResultImportRequest {
    pub ztd_path: PathBuf,
    pub rsc_path: PathBuf,
    #[serde(default)]
    pub output_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationNetworkSettings {
    pub proxy_enabled: bool,
    pub proxy_url: String,
    pub cache_enabled: bool,
    pub cache_dir: String,
    pub cache_limit_mb: u64,
    /// User-configured Tianditu token used by admitted map/annotation providers.
    /// This stays in the product-owned settings sidecar and never enters Store.
    pub tianditu_token: String,
    pub asf_ssl_verify: bool,
}

/// One provider entry endpoint's connectivity probe outcome, serialized to the
/// desktop settings card. `error_code` uses the fixed stage set of the
/// single-endpoint probe; `site` is the provider homepage the UI links to.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderConnectivityResult {
    pub id: String,
    pub site: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub http_status: Option<u16>,
    pub error_code: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProxyMode {
    Auto,
    Manual,
    Direct,
}

impl ApplicationNetworkSettings {
    pub fn proxy_mode(&self) -> NetworkProxyMode {
        match (self.proxy_enabled, self.proxy_url.trim().is_empty()) {
            (false, _) => NetworkProxyMode::Direct,
            (true, true) => NetworkProxyMode::Auto,
            (true, false) => NetworkProxyMode::Manual,
        }
    }
}

impl Default for ApplicationNetworkSettings {
    fn default() -> Self {
        Self {
            proxy_enabled: false,
            proxy_url: String::new(),
            cache_enabled: true,
            cache_dir: String::new(),
            cache_limit_mb: 2_048,
            tianditu_token: String::new(),
            asf_ssl_verify: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemConversionRequest {
    pub request_id: String,
    pub source_task_id: String,
    pub conversion_task_id: String,
    pub region_id: String,
    pub resource_id: String,
    pub capability_id: String,
    pub dataset: String,
    pub provider: String,
    pub input_path: PathBuf,
    pub output_root: PathBuf,
    #[serde(default)]
    pub source_vertical_datum: Option<String>,
    #[serde(default)]
    pub target_vertical_datum: Option<String>,
    #[serde(default)]
    pub output_mode: Option<String>,
    pub created_at: String,
    pub concurrency: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrbitTaskRequest {
    pub request_id: String,
    pub task_id: String,
    pub region_id: String,
    pub association: insar_core::FrozenOrbitAssociationV1,
    pub output_root: PathBuf,
    pub created_at: String,
    pub concurrency: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandaloneOrbitSceneInput {
    pub scene_id: String,
    #[serde(default)]
    pub acquisition_datetime: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandaloneOrbitTaskRequest {
    pub request_id: String,
    pub task_id: String,
    pub scenes: Vec<StandaloneOrbitSceneInput>,
    pub output_root: PathBuf,
    #[serde(default = "default_true")]
    pub use_orbit_subdir: bool,
    pub created_at: String,
}

fn default_true() -> bool {
    true
}

/// Application-owned seam for the remaining explicitly approved DEM execution
/// operations. AOI codecs are product-native and must not use this seam.
/// Normal production implementations execute the product-owned native GIS
/// boundary. Explicit history/smoke fixtures may inject a legacy adapter, but
/// the adapter never receives Store, credential, provider-network, or queue
/// authority.
pub trait DemOperationPort: Send + Sync + 'static {
    fn call(&self, operation: &str, payload: Value) -> Result<Value, String>;

    fn call_with_transfer_progress(
        &self,
        operation: &str,
        payload: Value,
        _interrupted: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(DemTransferEvent),
    ) -> Result<Value, String> {
        self.call(operation, payload)
    }
}

/// Native HTTP body activity only; never raster size or estimated output progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemTransferEvent {
    BodyStarted,
    BytesReceived(u64),
    BodyStopped,
}

#[doc(hidden)]
pub trait LiveLandsatBundleOptions: Send + Sync {
    fn resolve_official_product(
        &self,
        _display_id: &str,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<ProviderProduct, UsgsM2mError> {
        Err(UsgsM2mError::new(
            insar_provider::usgs_m2m::UsgsM2mErrorCode::DownloadUnavailable,
            insar_provider::usgs_m2m::UsgsM2mEndpointClass::SceneSearch,
        ))
    }

    fn full_bundle_option(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mFullBundleOption, UsgsM2mError>;
}

impl LiveLandsatBundleOptions for UsgsM2mDownloadClient {
    fn resolve_official_product(
        &self,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<ProviderProduct, UsgsM2mError> {
        insar_provider::usgs_m2m::UsgsM2mSceneSearchClient::new(self.session())
            .exact_product(display_id, cancelled)
    }

    fn full_bundle_option(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mFullBundleOption, UsgsM2mError> {
        self.full_bundle_option(dataset_alias, entity_id, display_id, cancelled)
    }
}

#[doc(hidden)]
pub trait LiveLandsatAssetManifest: Send + Sync {
    fn landsat_asset_manifest(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerLandsatManifest, PlanetaryComputerError>;
}

impl LiveLandsatAssetManifest for PlanetaryComputerLandsatClient {
    fn landsat_asset_manifest(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerLandsatManifest, PlanetaryComputerError> {
        self.manifest(dataset_alias, entity_id, display_id, cancelled)
    }
}

impl LiveLandsatBrowse for UsgsM2mSceneSearchRouter {
    fn fetch_landsat_browse(
        &self,
        query: &CatalogSearchQuery,
        entity_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<LandsatBrowseImage>, UsgsM2mError> {
        self.fetch_browse(query, entity_id, cancelled).map(|image| {
            image.and_then(|image| {
                LandsatBrowseImage::from_parts(image.media_type(), image.bytes().to_vec())
            })
        })
    }
}

impl LiveLandsatBrowse for PlanetaryComputerLandsatClient {
    fn fetch_landsat_browse(
        &self,
        query: &CatalogSearchQuery,
        entity_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<LandsatBrowseImage>, UsgsM2mError> {
        self.fetch_browse(query, entity_id, cancelled)
            .map_err(map_planetary_computer_browse_error)
            .map(|image| {
                image.and_then(|(media_type, bytes)| {
                    LandsatBrowseImage::from_parts(media_type, bytes)
                })
            })
    }
}

fn map_planetary_computer_browse_error(error: PlanetaryComputerError) -> UsgsM2mError {
    let code = match error.code() {
        insar_provider::planetary_computer::PlanetaryComputerErrorCode::Cancelled => {
            UsgsM2mErrorCode::Cancelled
        }
        insar_provider::planetary_computer::PlanetaryComputerErrorCode::ConfigInvalid
        | insar_provider::planetary_computer::PlanetaryComputerErrorCode::IdentityMismatch => {
            UsgsM2mErrorCode::ResponseInvalid
        }
        insar_provider::planetary_computer::PlanetaryComputerErrorCode::ResponseTooLarge => {
            UsgsM2mErrorCode::ResponseTooLarge
        }
        insar_provider::planetary_computer::PlanetaryComputerErrorCode::TransportFailed => {
            UsgsM2mErrorCode::TransportFailed
        }
        _ => UsgsM2mErrorCode::RemoteError,
    };
    UsgsM2mError::new(code, UsgsM2mEndpointClass::BrowseFetch)
}

#[derive(Clone)]
struct CdseSessionRuntime {
    network_policy: ProviderNetworkPolicy,
    session: Arc<CdseOAuthSession>,
}

impl CdseSessionRuntime {
    fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, CdseAuthError> {
        let foundation = CdseNetworkFoundation::new(network_policy);
        Ok(Self {
            network_policy,
            session: Arc::new(foundation.oauth_session()?),
        })
    }

    fn status(&self) -> Result<CdseSessionStatus, CdseAuthError> {
        Ok(cdse_status(self.network_policy, self.session.snapshot()?))
    }
}

#[derive(Clone)]
struct BhoonidhiSessionRuntime {
    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    acceptance_lease: Option<Arc<std::fs::File>>,
    session: Arc<BhoonidhiSession>,
    provider: Arc<BhoonidhiSessionTokenProvider>,
    admission: Arc<TransferAdmissionController>,
}

impl BhoonidhiSessionRuntime {
    fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, ApplicationServiceError> {
        let endpoint = ReqwestBhoonidhiTokenEndpoint::new(network_policy)
            .map_err(|_| ApplicationServiceError::RuntimeInitialization)?;
        let session = Arc::new(
            BhoonidhiSession::new(network_policy, Arc::new(endpoint), Arc::new(SystemClock))
                .map_err(|_| ApplicationServiceError::RuntimeInitialization)?,
        );
        let provider =
            Arc::new(BhoonidhiSessionTokenProvider::new(Arc::clone(&session)).with_desktop_login());
        Ok(Self {
            session,
            provider,
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            acceptance_lease: None,
            admission: TransferAdmissionController::compatibility(),
        })
    }
}

#[cfg(test)]
mod bhoonidhi_shutdown_tests;

#[cfg(test)]
mod account_preflight_tests;

#[cfg(test)]
mod bhoonidhi_catalogue_completion_tests;

#[derive(Clone)]
struct UsgsM2mSessionRuntime {
    session: Arc<UsgsM2mSession>,
    download_client: Arc<UsgsM2mDownloadClient>,
}

#[cfg(test)]
const fn usgs_m2m_default_network_policy() -> ProviderNetworkPolicy {
    // USGS M2M is reachable on the ordinary direct route in the supported
    // desktop environment. Keep provider proxy policy explicit; Bhoonidhi
    // retains its separate network handling.
    ProviderNetworkPolicy::direct()
}

impl UsgsM2mSessionRuntime {
    fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, UsgsM2mError> {
        let session = Arc::new(UsgsM2mSession::new(
            network_policy,
            Duration::from_secs(60),
        )?);
        let download_client = Arc::new(UsgsM2mDownloadClient::new(Arc::clone(&session)));
        Ok(Self {
            session,
            download_client,
        })
    }

    fn status(&self) -> Result<UsgsM2mDesktopSessionStatus, UsgsM2mError> {
        Ok(usgs_desktop_status(self.session.status()?))
    }
}

#[cfg(test)]
mod usgs_network_policy_tests {
    use super::*;

    #[test]
    fn usgs_m2m_runtime_bypasses_system_proxy_by_default() {
        assert_eq!(
            usgs_m2m_default_network_policy(),
            ProviderNetworkPolicy::direct()
        );
        let runtime = UsgsM2mSessionRuntime::new(usgs_m2m_default_network_policy())
            .expect("USGS runtime configuration");
        assert_eq!(
            runtime.session.network_policy(),
            ProviderNetworkPolicy::direct()
        );
    }
}

fn usgs_desktop_status(status: ProviderUsgsM2mSessionStatus) -> UsgsM2mDesktopSessionStatus {
    let state = match status {
        ProviderUsgsM2mSessionStatus::Ready => "ready",
        ProviderUsgsM2mSessionStatus::LoggingIn => "logging_in",
        ProviderUsgsM2mSessionStatus::Authenticated => "authenticated",
        ProviderUsgsM2mSessionStatus::LoginFailed => "login_failed",
        ProviderUsgsM2mSessionStatus::LoggingOut => "logging_out",
        ProviderUsgsM2mSessionStatus::LoggedOut => "logged_out",
    };
    UsgsM2mDesktopSessionStatus {
        state: state.to_owned(),
        authenticated: status == ProviderUsgsM2mSessionStatus::Authenticated,
    }
}

fn bhoonidhi_desktop_status(snapshot: BhoonidhiTokenSnapshot) -> BhoonidhiDesktopSessionStatus {
    BhoonidhiDesktopSessionStatus {
        authenticated: snapshot.authenticated,
        refresh_available: snapshot.refresh_available,
        expires_in_seconds: snapshot.expires_in_seconds,
    }
}

fn cdse_status(
    network_policy: ProviderNetworkPolicy,
    snapshot: CdseTokenSnapshot,
) -> CdseSessionStatus {
    CdseSessionStatus {
        network_mode: CdseNetworkMode::from_policy(network_policy),
        authenticated: snapshot.authenticated,
        refresh_available: snapshot.refresh_available,
        expires_in_seconds: snapshot.expires_in_seconds,
    }
}

fn diagnostic_stage_name(stage: CdseNetworkDiagnosticStage) -> &'static str {
    match stage {
        CdseNetworkDiagnosticStage::DnsUnavailable => "dns_unavailable",
        CdseNetworkDiagnosticStage::TcpUnavailable => "tcp_unavailable",
        CdseNetworkDiagnosticStage::TlsHandshakeFailed => "tls_handshake_failed",
        CdseNetworkDiagnosticStage::ProxyConfigurationRejected => "proxy_configuration_rejected",
        CdseNetworkDiagnosticStage::Timeout => "timeout",
        CdseNetworkDiagnosticStage::HttpReached => "http_reached",
        CdseNetworkDiagnosticStage::TransportUnavailable => "transport_unavailable",
    }
}

mod projection;

pub use projection::{
    canonical_core_snapshot_value, canonical_restore_value, canonical_snapshot_value,
    canonical_workbench_page_value, derive_workbench_page_render, ApplicationWorkbenchPage,
    ApplicationWorkbenchPageRequest, CoreAoiDisplayAuthority, CoreServiceSnapshot, DisplayBbox,
    ReadonlyServiceSnapshot, WorkbenchPageRenderAuthority, WorkbenchPageRenderItem,
    MAX_PAGE_RENDER_JSON_BYTES, MAX_PROJECTED_RENDER_LONGITUDE, MAX_RENDER_GEOMETRY_JSON_BYTES,
};

/// Fixed-size acknowledgement for the automatic archive mutation. Archive
/// writes must not trigger or return a terminal CoreRestore projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchiveDownloadTaskAck {
    pub task_id: String,
    pub revision: u64,
    pub archived_at: String,
}

/// Default persisted Store location used by a desktop host. This function only
/// resolves a local path; it does not open SQLite or inspect credentials.
pub fn default_store_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("APPDATA")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            #[cfg(target_os = "macos")]
            {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
                    .map(|home| home.join("Library").join("Application Support"))
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                std::env::var_os("XDG_DATA_HOME")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
                    .or_else(|| {
                        std::env::var_os("HOME")
                            .filter(|v| !v.is_empty())
                            .map(PathBuf::from)
                            .map(|home| home.join(".local").join("share"))
                    })
            }
            #[cfg(not(unix))]
            {
                None
            }
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("EOStage").join("v3").join("tasks.sqlite3")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplicationServiceMode {
    Readonly,
    Business,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationServiceConfig {
    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    auth_revoke_acceptance_profile: Option<PathBuf>,
    store_path: Option<PathBuf>,
    require_store: bool,
    require_existing_store: bool,
    mode: ApplicationServiceMode,
    capability_observations: CapabilityRuntimeObservations,
    suppress_embedded_nisar_admission: bool,
    verification_only: bool,
}

impl ApplicationServiceConfig {
    pub fn is_verification_only(&self) -> bool {
        self.verification_only
    }
}

#[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
mod bhoonidhi_auth_revoke_acceptance;

impl ApplicationServiceConfig {
    /// Configure a read-only service that creates/opens the explicit Store.
    pub fn store(path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: Some(path.into()),
            require_store: true,
            require_existing_store: false,
            mode: ApplicationServiceMode::Readonly,
            capability_observations: CapabilityRuntimeObservations::default(),
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            auth_revoke_acceptance_profile: None,
            suppress_embedded_nisar_admission: false,
            verification_only: false,
        }
    }

    /// Configure a read-only host that only opens an existing Store.
    pub fn existing_store(path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: Some(path.into()),
            require_store: true,
            require_existing_store: true,
            mode: ApplicationServiceMode::Readonly,
            capability_observations: CapabilityRuntimeObservations::default(),
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            auth_revoke_acceptance_profile: None,
            suppress_embedded_nisar_admission: false,
            verification_only: false,
        }
    }

    /// Configure the downloads-enabled provider business runtime. This is an
    /// explicit profile; it is never inferred from a shell or environment.
    pub fn business_store(path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: Some(path.into()),
            require_store: true,
            require_existing_store: false,
            mode: ApplicationServiceMode::Business,
            capability_observations: CapabilityRuntimeObservations::default(),
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            auth_revoke_acceptance_profile: None,
            suppress_embedded_nisar_admission: false,
            verification_only: false,
        }
    }

    /// Configure a business runtime that refuses to create a missing Store.
    pub fn existing_business_store(path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: Some(path.into()),
            require_store: true,
            require_existing_store: true,
            mode: ApplicationServiceMode::Business,
            capability_observations: CapabilityRuntimeObservations::default(),
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            auth_revoke_acceptance_profile: None,
            suppress_embedded_nisar_admission: false,
            verification_only: false,
        }
    }

    /// Configure a liveness-only service. It never opens a Store.
    pub fn liveness_only() -> Self {
        Self {
            store_path: None,
            require_store: false,
            require_existing_store: false,
            mode: ApplicationServiceMode::Readonly,
            capability_observations: CapabilityRuntimeObservations::default(),
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            auth_revoke_acceptance_profile: None,
            suppress_embedded_nisar_admission: false,
            verification_only: false,
        }
    }

    /// Configure a service that may open a Store when a path is supplied.
    pub fn optional_store(path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: Some(path.into()),
            require_store: false,
            require_existing_store: false,
            mode: ApplicationServiceMode::Readonly,
            capability_observations: CapabilityRuntimeObservations::default(),
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            auth_revoke_acceptance_profile: None,
            suppress_embedded_nisar_admission: false,
            verification_only: false,
        }
    }

    /// Configure the business runtime used by the packaged, provider-free
    /// smoke test. It keeps the normal runtime graph intact while removing the
    /// embedded NISAR acceptance input so this deterministic probe cannot claim
    /// live verification from static repository evidence.
    pub fn offline_smoke(path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: Some(path.into()),
            require_store: true,
            require_existing_store: false,
            mode: ApplicationServiceMode::Business,
            capability_observations: CapabilityRuntimeObservations::default(),
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            auth_revoke_acceptance_profile: None,
            suppress_embedded_nisar_admission: true,
            verification_only: false,
        }
    }

    /// Configure a verification-only service over an existing Store.
    ///
    /// The Store is opened SQLite read-only with the exact supported schema,
    /// no migration, no journal/synchronous pragma change, and no scheduler,
    /// Dispatcher, Worker, Provider, credential, or mutation capability. This
    /// profile is injectable only by the internal test/acceptance harness; no
    /// normal UI or Tauri command may select it.
    pub fn verification_only(path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: Some(path.into()),
            require_store: true,
            require_existing_store: true,
            mode: ApplicationServiceMode::Readonly,
            capability_observations: CapabilityRuntimeObservations::default(),
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            auth_revoke_acceptance_profile: None,
            suppress_embedded_nisar_admission: false,
            verification_only: true,
        }
    }

    pub fn requires_store(&self) -> bool {
        self.require_store
    }

    pub fn store_path(&self) -> Option<&Path> {
        self.store_path.as_deref()
    }

    pub fn requires_existing_store(&self) -> bool {
        self.require_existing_store
    }

    pub fn is_business(&self) -> bool {
        self.mode == ApplicationServiceMode::Business
    }

    /// Compose a secret-free observation snapshot supplied by build, provider,
    /// credential, evidence, or product-policy owners. Querying the resulting
    /// service still performs no provider operation.
    pub fn with_capability_runtime_observations(
        mut self,
        observations: CapabilityRuntimeObservations,
    ) -> Self {
        // Ordinary host configuration may provide local build/runtime/credential
        // observations for deterministic fixtures, but it cannot manufacture a
        // release admission or product policy decision.
        self.capability_observations = observations.without_admission_and_policy();
        self
    }

    /// Controlled acceptance-only seam. It is feature-gated so a normal
    /// desktop build has no API that can inject admission or product policy.
    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn with_controlled_capability_runtime_observations(
        mut self,
        observations: CapabilityRuntimeObservations,
    ) -> Self {
        self.capability_observations = observations;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Created,
    StoreReady,
    LivenessOnly,
    ShuttingDown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceReadiness {
    pub contract: String,
    pub status: ServiceStatus,
    #[serde(default)]
    pub store_schema_version: Option<i64>,
}

impl ServiceReadiness {
    fn store_ready(health: &StoreHealth) -> Self {
        Self {
            contract: APPLICATION_SERVICE_CONTRACT_VERSION.to_owned(),
            status: ServiceStatus::StoreReady,
            store_schema_version: Some(health.schema_version),
        }
    }

    fn liveness_only() -> Self {
        Self {
            contract: APPLICATION_SERVICE_CONTRACT_VERSION.to_owned(),
            status: ServiceStatus::LivenessOnly,
            store_schema_version: None,
        }
    }
}

/// The operation-facing AOI lifecycle is intentionally separate from the
/// saved Workspace/Project/Region hierarchy. `NoAoi` is a real state: it means
/// that a provider query must not contain a spatial filter or an inferred
/// world-wide bbox.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationAoiState {
    #[default]
    NoAoi,
    Preview,
    Working,
    SavedRegion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationWorkingAoi {
    pub state: ApplicationAoiState,
    pub source_kind: String,
    pub geometry: Map<String, Value>,
    pub bbox: Vec<f64>,
    pub crs: String,
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationSetWorkingAoiRequest {
    pub source_kind: String,
    pub geometry: Map<String, Value>,
    pub bbox: Vec<f64>,
    pub crs: String,
    pub display_name: String,
}

/// Input to the atomic Provider-search and SearchBatch-freeze operation.
/// `aoi_geojson` is carried separately so the service can reject a search whose
/// frozen geometry differs from the submitted scientific query.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationSarSearchAndFreezeRequest {
    pub request_id: String,
    pub search: SarRemoteSearchRequest,
    pub batch_id: String,
    pub created_at: String,
    pub aoi_name: String,
    #[serde(default)]
    pub aoi_geojson: Option<Map<String, Value>>,
    pub page_size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationSarBackgroundCountRequest {
    pub request_id: String,
    pub batch_id: String,
    pub expected_workbench_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationSarBackgroundCountResult {
    pub applied: bool,
    pub total_count: Option<u64>,
    pub snapshot: Option<CoreServiceSnapshot>,
}

impl ApplicationSarBackgroundCountRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_catalogue_opaque_id(&self.request_id) {
            return Err("request_id must be a bounded opaque identifier".to_owned());
        }
        if !valid_catalogue_opaque_id(&self.batch_id) {
            return Err("batch_id must be a bounded opaque identifier".to_owned());
        }
        Ok(())
    }
}

pub type SarSearchProgressStage = ApplicationSearchProgressStage;
pub type SarSearchProgressUpdate = ApplicationSearchProgress;
pub type ApplicationSearchProgressProjection = ApplicationSearchProgress;

trait ApplicationSearchProgressExt {
    fn live(request_id: &str, stage: SarSearchProgressStage, message: &str) -> Self;
    fn measured(
        request_id: &str,
        stage: SarSearchProgressStage,
        admitted_count: u64,
        requested_limit: u64,
        message: &str,
    ) -> Self;
    fn counted(
        request_id: &str,
        stage: SarSearchProgressStage,
        admitted_count: u64,
        message: &str,
    ) -> Self;
    fn terminal(request_id: &str, stage: SarSearchProgressStage, message: &str) -> Self;
    fn with_elapsed(self, started: Instant) -> Self;
    fn carry_forward(self, previous: Option<&Self>) -> Self;
}

fn max_observed(current: Option<u64>, previous: Option<u64>) -> Option<u64> {
    match (current, previous) {
        (Some(current), Some(previous)) => Some(current.max(previous)),
        (current, previous) => current.or(previous),
    }
}

impl ApplicationSearchProgressExt for ApplicationSearchProgress {
    fn live(request_id: &str, stage: SarSearchProgressStage, message: &str) -> Self {
        Self {
            request_id: request_id.to_owned(),
            sequence: 0,
            stage,
            provider_request_count: None,
            raw_fetched_count: None,
            mapped_count: None,
            exact_matched_count: None,
            admitted_count: None,
            requested_limit: None,
            remote_total: None,
            batch_id: None,
            loaded_count: None,
            elapsed_ms: 0,
            message: message.to_owned(),
            terminal: false,
        }
    }

    fn measured(
        request_id: &str,
        stage: SarSearchProgressStage,
        admitted_count: u64,
        requested_limit: u64,
        message: &str,
    ) -> Self {
        Self {
            exact_matched_count: Some(admitted_count),
            admitted_count: Some(admitted_count),
            requested_limit: Some(requested_limit),
            ..Self::live(request_id, stage, message)
        }
    }

    fn counted(
        request_id: &str,
        stage: SarSearchProgressStage,
        admitted_count: u64,
        message: &str,
    ) -> Self {
        Self {
            exact_matched_count: Some(admitted_count),
            admitted_count: Some(admitted_count),
            ..Self::live(request_id, stage, message)
        }
    }

    fn terminal(request_id: &str, stage: SarSearchProgressStage, message: &str) -> Self {
        Self {
            terminal: true,
            ..Self::live(request_id, stage, message)
        }
    }

    fn with_elapsed(mut self, started: Instant) -> Self {
        self.elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        self
    }

    fn carry_forward(mut self, previous: Option<&Self>) -> Self {
        let Some(previous) = previous.filter(|value| value.request_id == self.request_id) else {
            return self;
        };
        self.provider_request_count =
            max_observed(self.provider_request_count, previous.provider_request_count);
        self.raw_fetched_count = max_observed(self.raw_fetched_count, previous.raw_fetched_count);
        self.mapped_count = max_observed(self.mapped_count, previous.mapped_count);
        self.exact_matched_count =
            max_observed(self.exact_matched_count, previous.exact_matched_count);
        self.admitted_count = max_observed(self.admitted_count, previous.admitted_count);
        self.requested_limit = self.requested_limit.or(previous.requested_limit);
        self.remote_total = self.remote_total.or(previous.remote_total);
        // #22 B slice 3(c): a live update keeps naming the readable batch; a
        // terminal update states it explicitly (a batch may have been kept
        // or withdrawn), so it never inherits a stale one.
        if !self.terminal {
            self.batch_id = self.batch_id.take().or_else(|| previous.batch_id.clone());
            self.loaded_count = max_observed(self.loaded_count, previous.loaded_count);
        }
        self.elapsed_ms = self.elapsed_ms.max(previous.elapsed_ms);
        self
    }
}

impl ApplicationSarSearchAndFreezeRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_catalogue_opaque_id(&self.request_id) {
            return Err("request_id must be a bounded opaque identifier".to_owned());
        }
        if !valid_catalogue_opaque_id(&self.batch_id) {
            return Err("batch_id must be a bounded opaque identifier".to_owned());
        }
        if self.created_at.trim().is_empty() {
            return Err("created_at is required".to_owned());
        }
        if self.aoi_name.trim().is_empty() {
            return Err("aoi_name is required".to_owned());
        }
        if self.page_size == 0 {
            return Err("page_size must be >= 1".to_owned());
        }
        self.search.validate()?;
        if let Some(aoi_geojson) = &self.aoi_geojson {
            if validated_authoritative_polygonal_geojson(aoi_geojson).is_none() {
                return Err(
                    "aoi_geojson must be a valid authoritative Polygon or MultiPolygon".to_owned(),
                );
            }
        }
        if self.search.query.aoi_geojson != self.aoi_geojson {
            return Err("search query AOI must equal the frozen AOI geometry".to_owned());
        }
        Ok(())
    }
}

const MAX_CATALOGUE_WORKBENCH_PAGE_SIZE: u64 = 250;
const MAX_CATALOGUE_CANDIDATE_JSON_BYTES: usize = 64 * 1024;
const MAX_CATALOGUE_PRODUCTS: usize = MAX_CATALOGUE_CANDIDATE_RECORDS as usize;
const SAR_SEARCH_CACHE_MAX_AGE_SECONDS: u64 = 24 * 60 * 60;
const VOLATILE_PROVIDER_SEARCH_CACHE_MAX_AGE_SECONDS: u64 = 2 * 60 * 60;

fn catalog_search_cache_max_age_seconds(provider_id: &str) -> u64 {
    if provider_id
        .trim()
        .to_ascii_lowercase()
        .contains("bhoonidhi")
    {
        VOLATILE_PROVIDER_SEARCH_CACHE_MAX_AGE_SECONDS
    } else {
        SAR_SEARCH_CACHE_MAX_AGE_SECONDS
    }
}

type CataloguePublicationHook = Arc<dyn Fn() + Send + Sync>;

/// Internal input to the catalogue-to-SearchBatch vertical. It is carried only
/// through the reviewed ApplicationService/DesktopShell boundary, not Sidecar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationCatalogSearchAndFreezeRequest {
    pub request_id: String,
    pub search: CatalogSearchQuery,
    pub batch_id: String,
    pub created_at: String,
    pub aoi_name: String,
    #[serde(default)]
    pub aoi_geojson: Option<Map<String, Value>>,
    pub workbench_page_size: u64,
}

impl ApplicationCatalogSearchAndFreezeRequest {
    pub fn validate(&self) -> Result<(), String> {
        self.validate_for(RouteUse::OfflineFixture)
    }

    pub fn validate_for(&self, route_use: RouteUse) -> Result<(), String> {
        self.validate_for_mode(route_use, false)
    }

    fn validate_for_mode(
        &self,
        route_use: RouteUse,
        live_acceptance_candidate: bool,
    ) -> Result<(), String> {
        self.validate_for_mode_with_fixture(route_use, live_acceptance_candidate, None)
    }

    fn validate_for_mode_with_fixture(
        &self,
        route_use: RouteUse,
        live_acceptance_candidate: bool,
        fixture: Option<&StacOfflineFixtureAdmission>,
    ) -> Result<(), String> {
        if !valid_opaque_id(&self.request_id) {
            return Err("request_id must be a bounded opaque identifier".to_owned());
        }
        if !valid_opaque_id(&self.batch_id) {
            return Err("batch_id must be a bounded opaque identifier".to_owned());
        }
        if !valid_exact_text(&self.created_at) {
            return Err(
                "created_at must be non-empty, exactly trimmed, and control-free".to_owned(),
            );
        }
        if !valid_exact_text(&self.aoi_name) {
            return Err("aoi_name must be non-empty, exactly trimmed, and control-free".to_owned());
        }
        if !(1..=MAX_CATALOGUE_WORKBENCH_PAGE_SIZE).contains(&self.workbench_page_size) {
            return Err(
                "workbench_page_size is outside the catalogue application bound".to_owned(),
            );
        }
        if self.search.cursor.is_some() {
            return Err("catalogue batch freeze does not accept a continuation cursor".to_owned());
        }
        self.search.validate()?;
        if self.aoi_geojson != self.search.aoi_geojson {
            return Err("search query AOI must equal the frozen AOI geometry".to_owned());
        }
        if self
            .aoi_geojson
            .as_ref()
            .is_some_and(|geometry| validated_authoritative_polygonal_geojson(geometry).is_none())
        {
            return Err(
                "aoi_geojson must be a valid authoritative Polygon or MultiPolygon".to_owned(),
            );
        }
        if let Some(admission) = fixture.filter(|a| a.targets(&self.search)) {
            if live_acceptance_candidate {
                return Err("offline fixture cannot use live acceptance".to_owned());
            }
            return admission
                .validate_query(&self.search, route_use)
                .map_err(|_| "offline fixture request is not admitted".to_owned());
        }
        let route = resolve_catalogue_route(
            &self.search.logical_dataset,
            &self.search.provider_id,
            &self.search.native_collection_id,
            route_use,
            live_acceptance_candidate,
        )
        .map_err(|_| "catalogue route is unavailable for this runtime".to_owned())?;
        let profile = CollectionProfileRegistry
            .get(route.collection_profile_id)
            .ok_or_else(|| "catalogue collection profile is unavailable".to_owned())?;
        if route.catalogue_aoi_crs != "EPSG:4326"
            || profile.logical_dataset != self.search.logical_dataset
            || profile.provider_id != self.search.provider_id
            || profile.native_collection_id != self.search.native_collection_id
        {
            return Err("catalogue route/profile does not match the request".to_owned());
        }
        if matches!(route_use, RouteUse::Production)
            && !live_acceptance_candidate
            && (!route.production_enabled || !profile.production_enabled)
        {
            return Err("catalogue route/profile is not enabled for production".to_owned());
        }
        DatasetRoutePolicyRegistry
            .validate_filter_keys(
                &self.search.logical_dataset,
                &self
                    .search
                    .filters
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            )
            .map_err(|_| "catalogue query uses an unsupported filter".to_owned())?;
        Ok(())
    }
}

fn validate_working_aoi(request: &ApplicationSetWorkingAoiRequest) -> Result<(), String> {
    if request.source_kind.trim().is_empty() {
        return Err("working AOI source_kind is required".to_owned());
    }
    if request.display_name.trim().is_empty() {
        return Err("working AOI display_name is required".to_owned());
    }
    if request.crs != "EPSG:4326" {
        return Err("working AOI crs must equal EPSG:4326".to_owned());
    }
    if request.bbox.len() < 4 || request.bbox.iter().any(|value| !value.is_finite()) {
        return Err("working AOI bbox must contain four finite EPSG:4326 values".to_owned());
    }
    if validated_authoritative_polygonal_geojson(&request.geometry).is_none() {
        return Err(
            "working AOI geometry must be a valid authoritative Polygon or MultiPolygon".to_owned(),
        );
    }
    Ok(())
}

fn application_working_aoi(
    value: StoreStandaloneAoi,
    state: ApplicationAoiState,
) -> ApplicationWorkingAoi {
    ApplicationWorkingAoi {
        state,
        source_kind: value.source_kind,
        geometry: value.geometry,
        bbox: value.bbox,
        crs: value.crs,
        display_name: value.display_name,
    }
}

fn resolve_catalogue_route(
    logical_dataset: &str,
    provider_id: &str,
    native_collection_id: &str,
    route_use: RouteUse,
    live_acceptance_candidate: bool,
) -> Result<&'static DatasetRoutePolicy, ()> {
    #[cfg(feature = "usgs-m2m-live-acceptance")]
    if live_acceptance_candidate {
        if route_use != RouteUse::Production {
            return Err(());
        }
        return DatasetRoutePolicyRegistry
            .resolve_live_acceptance_candidate(logical_dataset, provider_id, native_collection_id)
            .map_err(|_| ());
    }
    #[cfg(not(feature = "usgs-m2m-live-acceptance"))]
    let _ = live_acceptance_candidate;

    DatasetRoutePolicyRegistry
        .resolve(
            logical_dataset,
            provider_id,
            native_collection_id,
            route_use,
        )
        .map_err(|_| ())
}

/// B15: which task's renamed-aside conflict files to enumerate.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoreTaskConflictFilesRequest {
    pub task_id: String,
}

impl StoreTaskConflictFilesRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.task_id.chars().count() < 1 {
            return Err("task_id is too short".to_string());
        }
        if self.task_id.chars().count() > 128 {
            return Err("task_id is too long".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ApplicationServiceError {
    #[error("the GACOS request operation failed")]
    Gacos(#[from] GacosRequestError),
    #[error("Store readiness capability is unavailable")]
    CapabilityUnavailable { capability: &'static str },
    #[error("a Store path is required for this service")]
    StorePathRequired,
    #[error("the Store is not ready")]
    StoreNotReady,
    #[error("the provider-ready business runtime is unavailable")]
    BusinessRuntimeUnavailable,
    #[error("the provider-ready business runtime failed to initialize")]
    RuntimeInitialization,
    #[error("the Resource Registry is unavailable")]
    ResourceRegistryUnavailable,
    #[error("the application request is invalid")]
    InvalidRequest { reason: String },
    #[error("the OpenTopography DEM request exceeds the provider area limit")]
    OpenTopographyAreaLimitExceeded { requested_km2: u64, limit_km2: u64 },
    #[error("the DEM range exceeds the native raster limit")]
    DemRangeLimitExceeded {
        width: u64,
        height: u64,
        pixels: u64,
        coarser_dataset_available: bool,
    },
    #[error("the optional processing engine is unavailable for {operation}")]
    ProcessingEngineUnavailable { operation: &'static str },
    #[error("the workspace revision conflicts with durable state")]
    WorkspaceRevisionConflict { expected: u64, actual: u64 },
    #[error("the formal task source is not completed")]
    FormalTaskSourceIncomplete,
    #[error("the formal task route is unavailable")]
    FormalTaskRouteUnavailable,
    #[error("the application service is shutting down")]
    ShuttingDown,
    #[error("the storage drive is below the download free-space floor")]
    DiskLow { reason: insar_core::TaskPauseReason },
    #[error("NISAR Remote Subset is not enabled")]
    NisarSubsetNotEnabled,
    #[error("the NISAR Remote Subset source is invalid")]
    NisarSubsetSourceInvalid,
    #[error("the NISAR Remote Subset selection is unsupported")]
    NisarSubsetSelectionUnsupported,
    #[error("the NISAR Remote Subset request is invalid")]
    NisarSubsetRequestInvalid,
    #[error("the NISAR Remote Subset task conflicts with durable state")]
    NisarSubsetTaskConflict,
    #[error("the frozen NISAR source changed before task creation")]
    NisarSubsetSourceChanged,
    #[error("the NISAR V2 request is invalid")]
    NisarSubsetV2RequestInvalid,
    #[error("the NISAR V2 source batch is unavailable")]
    NisarSubsetV2SourceBatchUnavailable,
    #[error("the NISAR V2 source item is unavailable")]
    NisarSubsetV2SourceItemUnavailable,
    #[error("the NISAR V2 source item is not selected")]
    NisarSubsetV2ItemNotSelected,
    #[error("the frozen NISAR V2 source identity is inconsistent")]
    NisarSubsetV2SourceIdentityMismatch,
    #[error("the NISAR V2 subset draft geometry is invalid")]
    NisarSubsetV2DraftInvalid,
    #[error("the NISAR V2 subset draft crosses the antimeridian")]
    NisarSubsetV2AntimeridianUnsupported,
    #[error("the NISAR V2 effective geometry is empty")]
    NisarSubsetV2EmptyEffectiveGeometry,
    #[error("the selected NISAR V2 frequency is unsupported")]
    NisarSubsetV2UnsupportedFrequency,
    #[error("the selected NISAR V2 polarization is unsupported")]
    NisarSubsetV2UnsupportedPolarization,
    #[error("the NISAR V2 prepared intent failed validation")]
    NisarSubsetV2PreparedIntentInvalid,
    #[error("the NISAR V2 prepared intent does not match the frozen source authority")]
    NisarSubsetV2PreparedIntentIdentityMismatch,
    #[error("authoritative NISAR V2 dependency evidence is unavailable")]
    NisarSubsetV2DependencyEvidenceUnavailable,
    #[error("the authoritative NISAR V2 dependency graph is invalid")]
    NisarSubsetV2DependencyGraphInvalid,
    #[error("the derived NISAR V2 dependency closure is invalid")]
    NisarSubsetV2DependencyClosureInvalid,
    #[error("the canonical NISAR V2 plan failed validation")]
    NisarSubsetV2PlanValidationFailed,
    #[error("the Store failed to persist the frozen NISAR V2 authority")]
    NisarSubsetV2StorePersistenceFailed,
    #[error("the NISAR V2 task identity already exists")]
    NisarSubsetV2TaskConflict,
    #[error("the NISAR V2 source SearchBatch changed concurrently")]
    NisarSubsetV2SearchBatchStale,
    #[error("the application service state lock is poisoned")]
    StatePoisoned,
    #[error("the Store failed to open or validate")]
    Store(#[source] StoreError),
    #[error("the Provider operation failed")]
    Provider(#[source] ContractError),
    #[error("the shared Preview operation failed")]
    Preview(#[source] PreviewAssetError),
    #[error("the CDSE session operation failed")]
    Cdse(#[source] CdseAuthError),
    #[error("需要登录 {provider} 后才能继续")]
    AccountLoginRequired { provider: &'static str },
    #[error(
        "无法访问系统凭据存储，暂不能确认 {provider} 账号状态，请检查 Windows 凭据管理器后重试"
    )]
    AccountServiceUnavailable { provider: &'static str },
    #[error("the USGS M2M session or browse operation failed")]
    UsgsM2m(#[source] UsgsM2mError),
    #[error("the frozen CDSE quicklook evidence is invalid")]
    CdseQuicklookEvidenceInvalid,
    #[error("the frozen CDSE product has no quicklook")]
    CdseQuicklookUnavailable,
    #[error("a CDSE quicklook request identifier is already active")]
    CdseQuicklookRequestDuplicate,
    #[error("the CDSE quicklook operation failed")]
    CdseQuicklook(#[source] CdseQuicklookError),
    #[error("the frozen Landsat browse evidence is invalid")]
    LandsatBrowseEvidenceInvalid,
    #[error("the frozen Landsat scene has no browse image")]
    LandsatBrowseUnavailable,
    #[error("a Landsat browse request identifier is already active")]
    LandsatBrowseRequestDuplicate,
    #[error("the Landsat browse request was cancelled")]
    LandsatBrowseCancelled,
    #[error("the frozen Landsat full-bundle evidence is invalid")]
    LandsatBundleEvidenceInvalid,
    #[error("a Landsat bundle option request identifier is already active")]
    LandsatBundleRequestDuplicate,
    #[error("the Landsat bundle task request was cancelled")]
    LandsatBundleCancelled,
    #[error("the public Landsat asset manifest operation failed")]
    PlanetaryComputer(#[source] PlanetaryComputerError),
    #[error("the frozen public Landsat asset evidence is invalid")]
    LandsatAssetEvidenceInvalid,
    #[error("a public Landsat asset request identifier is already active")]
    LandsatAssetRequestDuplicate,
    #[error("the public Landsat asset task request was cancelled")]
    LandsatAssetCancelled,
    #[error("the frozen Bhoonidhi S-band asset evidence is invalid")]
    BhoonidhiAssetEvidenceInvalid,
    #[error("a Bhoonidhi S-band size request identifier is already active")]
    BhoonidhiAssetRequestDuplicate,
    #[error("the Bhoonidhi S-band size request was cancelled")]
    BhoonidhiAssetCancelled,
    #[error("the Bhoonidhi S-band product size is unavailable")]
    BhoonidhiAssetSizeUnavailable,
    #[error("the frozen reviewed Planetary Computer asset evidence is invalid")]
    ReviewedMpcAssetEvidenceInvalid,
    #[error("the frozen Microsoft Sentinel-2 delivery evidence is invalid")]
    Sentinel2ArchiveEvidenceInvalid,
    #[error("the frozen Microsoft Sentinel-2 product could not be resolved to one CDSE archive")]
    Sentinel2ArchiveUnavailable,
    #[error("a Sentinel-2 archive delivery request identifier is already active")]
    Sentinel2ArchiveRequestDuplicate,
    #[error("the Sentinel-2 archive delivery request was cancelled")]
    Sentinel2ArchiveCancelled,
    #[error("the frozen Sentinel-2 analysis-band delivery evidence is invalid")]
    Sentinel2BandEvidenceInvalid,
    #[error("a Sentinel-2 analysis-band delivery request identifier is already active")]
    Sentinel2BandRequestDuplicate,
    #[error("the Sentinel-2 analysis-band delivery request was cancelled")]
    Sentinel2BandCancelled,
    #[error("the frozen ASF Burst asset evidence is invalid")]
    BurstAssetEvidenceInvalid,
    #[error("the Preview output directory is invalid")]
    PreviewDirectoryInvalid,
    #[error("the Preview image could not be written")]
    PreviewWriteFailed,
    #[error("a CDSE network policy change is blocked by active CDSE work")]
    CdseNetworkBusy,
}

impl ApplicationServiceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Gacos(error) => error.code(),
            Self::CapabilityUnavailable { .. } => "V3_STORE_READINESS_UNSUPPORTED",
            Self::StorePathRequired => "V3_STORE_PATH_REQUIRED",
            Self::StoreNotReady => "V3_STORE_NOT_READY",
            Self::BusinessRuntimeUnavailable => "V3_APPLICATION_BUSINESS_RUNTIME_UNAVAILABLE",
            Self::RuntimeInitialization => "V3_DOWNLOAD_INITIALIZATION_FAILED",
            Self::ResourceRegistryUnavailable => "RESOURCE_REGISTRY_UNAVAILABLE",
            Self::InvalidRequest { .. } => "V3_APPLICATION_INVALID_REQUEST",
            Self::OpenTopographyAreaLimitExceeded { .. } => "DEM_PROVIDER_AREA_LIMIT_EXCEEDED",
            Self::DemRangeLimitExceeded { .. } => "DEM_RANGE_LIMIT_EXCEEDED",
            Self::ProcessingEngineUnavailable { .. } => "PROCESSING_ENGINE_UNAVAILABLE",
            Self::WorkspaceRevisionConflict { .. } => "WORKSPACE_REVISION_CONFLICT",
            Self::FormalTaskSourceIncomplete => "FORMAL_TASK_SOURCE_INCOMPLETE",
            Self::FormalTaskRouteUnavailable => "FORMAL_TASK_ROUTE_UNAVAILABLE",
            Self::ShuttingDown => "V3_APPLICATION_SHUTTING_DOWN",
            Self::DiskLow { .. } => insar_core::storage_guard::DISK_LOW_ERROR_CODE,
            Self::NisarSubsetNotEnabled => "NISAR_SUBSET_NOT_ENABLED",
            Self::NisarSubsetSourceInvalid => "NISAR_SUBSET_SOURCE_INVALID",
            Self::NisarSubsetSelectionUnsupported => "NISAR_SUBSET_SELECTION_UNSUPPORTED",
            Self::NisarSubsetRequestInvalid => "NISAR_SUBSET_REQUEST_INVALID",
            Self::NisarSubsetTaskConflict => "NISAR_SUBSET_TASK_CONFLICT",
            Self::NisarSubsetSourceChanged => "NISAR_SUBSET_SOURCE_CHANGED",
            Self::NisarSubsetV2RequestInvalid => "NISAR_SUBSET_V2_REQUEST_INVALID",
            Self::NisarSubsetV2SourceBatchUnavailable => "NISAR_SUBSET_V2_SOURCE_BATCH_UNAVAILABLE",
            Self::NisarSubsetV2SourceItemUnavailable => "NISAR_SUBSET_V2_SOURCE_ITEM_UNAVAILABLE",
            Self::NisarSubsetV2ItemNotSelected => "NISAR_SUBSET_V2_ITEM_NOT_SELECTED",
            Self::NisarSubsetV2SourceIdentityMismatch => "NISAR_SUBSET_V2_SOURCE_IDENTITY_MISMATCH",
            Self::NisarSubsetV2DraftInvalid => "NISAR_SUBSET_V2_DRAFT_INVALID",
            Self::NisarSubsetV2AntimeridianUnsupported => {
                "NISAR_SUBSET_V2_ANTIMERIDIAN_UNSUPPORTED"
            }
            Self::NisarSubsetV2EmptyEffectiveGeometry => "NISAR_SUBSET_V2_EFFECTIVE_GEOMETRY_EMPTY",
            Self::NisarSubsetV2UnsupportedFrequency => "NISAR_SUBSET_V2_FREQUENCY_UNSUPPORTED",
            Self::NisarSubsetV2UnsupportedPolarization => {
                "NISAR_SUBSET_V2_POLARIZATION_UNSUPPORTED"
            }
            Self::NisarSubsetV2PreparedIntentInvalid => "NISAR_SUBSET_V2_PREPARED_INTENT_INVALID",
            Self::NisarSubsetV2PreparedIntentIdentityMismatch => {
                "NISAR_SUBSET_V2_PREPARED_INTENT_IDENTITY_MISMATCH"
            }
            Self::NisarSubsetV2DependencyEvidenceUnavailable => {
                "NISAR_SUBSET_V2_DEPENDENCY_EVIDENCE_UNAVAILABLE"
            }
            Self::NisarSubsetV2DependencyGraphInvalid => "NISAR_SUBSET_V2_DEPENDENCY_GRAPH_INVALID",
            Self::NisarSubsetV2DependencyClosureInvalid => {
                "NISAR_SUBSET_V2_DEPENDENCY_CLOSURE_INVALID"
            }
            Self::NisarSubsetV2PlanValidationFailed => "NISAR_SUBSET_V2_PLAN_VALIDATION_FAILED",
            Self::NisarSubsetV2StorePersistenceFailed => "NISAR_SUBSET_V2_STORE_PERSISTENCE_FAILED",
            Self::NisarSubsetV2TaskConflict => "NISAR_SUBSET_V2_TASK_CONFLICT",
            Self::NisarSubsetV2SearchBatchStale => "NISAR_SUBSET_V2_SEARCH_BATCH_STALE",
            Self::StatePoisoned => "V3_APPLICATION_STATE_POISONED",
            Self::Store(error) => error.code(),
            Self::Preview(error) => error.code(),
            Self::Cdse(error) => error.code(),
            Self::AccountLoginRequired { provider: "CDSE" } => "CDSE_ACCOUNT_LOGIN_REQUIRED",
            Self::AccountLoginRequired { provider: "USGS" } => "USGS_ACCOUNT_LOGIN_REQUIRED",
            Self::AccountLoginRequired {
                provider: "Earthdata",
            } => "EARTHDATA_ACCOUNT_LOGIN_REQUIRED",
            Self::AccountLoginRequired {
                provider: "Bhoonidhi",
            } => "BHOONIDHI_ACCOUNT_LOGIN_REQUIRED",
            Self::AccountLoginRequired {
                provider: "OpenTopography",
            } => "OPENTOPOGRAPHY_ACCOUNT_LOGIN_REQUIRED",
            Self::AccountLoginRequired { .. } => "ACCOUNT_LOGIN_REQUIRED",
            Self::AccountServiceUnavailable { provider: "CDSE" } => {
                "CDSE_ACCOUNT_SERVICE_UNAVAILABLE"
            }
            Self::AccountServiceUnavailable { provider: "USGS" } => {
                "USGS_ACCOUNT_SERVICE_UNAVAILABLE"
            }
            Self::AccountServiceUnavailable { .. } => "ACCOUNT_SERVICE_UNAVAILABLE",
            Self::UsgsM2m(error) => error.code_str(),
            Self::CdseQuicklookEvidenceInvalid => "CDSE_QUICKLOOK_FROZEN_EVIDENCE_INVALID",
            Self::CdseQuicklookUnavailable => "CDSE_QUICKLOOK_UNAVAILABLE",
            Self::CdseQuicklookRequestDuplicate => "CDSE_QUICKLOOK_REQUEST_DUPLICATE",
            Self::CdseQuicklook(error) => error.code(),
            Self::LandsatBrowseEvidenceInvalid => "USGS_LANDSAT_BROWSE_FROZEN_EVIDENCE_INVALID",
            Self::LandsatBrowseUnavailable => "USGS_LANDSAT_BROWSE_UNAVAILABLE",
            Self::LandsatBrowseRequestDuplicate => "USGS_LANDSAT_BROWSE_REQUEST_DUPLICATE",
            Self::LandsatBrowseCancelled => "USGS_M2M_CANCELLED",
            Self::LandsatBundleEvidenceInvalid => "USGS_LANDSAT_BUNDLE_FROZEN_EVIDENCE_INVALID",
            Self::LandsatBundleRequestDuplicate => "USGS_LANDSAT_BUNDLE_REQUEST_DUPLICATE",
            Self::LandsatBundleCancelled => "USGS_M2M_CANCELLED",
            Self::PlanetaryComputer(error) => error.code().as_str(),
            Self::LandsatAssetEvidenceInvalid => "LANDSAT_PUBLIC_ASSET_FROZEN_EVIDENCE_INVALID",
            Self::LandsatAssetRequestDuplicate => "LANDSAT_PUBLIC_ASSET_REQUEST_DUPLICATE",
            Self::LandsatAssetCancelled => "LANDSAT_PUBLIC_ASSET_CANCELLED",
            Self::BhoonidhiAssetEvidenceInvalid => "BHOONIDHI_ASSET_FROZEN_EVIDENCE_INVALID",
            Self::BhoonidhiAssetRequestDuplicate => "BHOONIDHI_ASSET_REQUEST_DUPLICATE",
            Self::BhoonidhiAssetCancelled => "BHOONIDHI_ASSET_CANCELLED",
            Self::BhoonidhiAssetSizeUnavailable => "BHOONIDHI_ASSET_SIZE_UNAVAILABLE",
            Self::ReviewedMpcAssetEvidenceInvalid => "MPC_REVIEWED_ASSET_FROZEN_EVIDENCE_INVALID",
            Self::Sentinel2ArchiveEvidenceInvalid => "SENTINEL2_ARCHIVE_FROZEN_EVIDENCE_INVALID",
            Self::Sentinel2ArchiveUnavailable => "SENTINEL2_ARCHIVE_DELIVERY_UNAVAILABLE",
            Self::Sentinel2ArchiveRequestDuplicate => "SENTINEL2_ARCHIVE_REQUEST_DUPLICATE",
            Self::Sentinel2ArchiveCancelled => "SENTINEL2_ARCHIVE_CANCELLED",
            Self::Sentinel2BandEvidenceInvalid => "SENTINEL2_BAND_FROZEN_EVIDENCE_INVALID",
            Self::Sentinel2BandRequestDuplicate => "SENTINEL2_BAND_REQUEST_DUPLICATE",
            Self::Sentinel2BandCancelled => "SENTINEL2_BAND_CANCELLED",
            Self::BurstAssetEvidenceInvalid => "ASF_BURST_FROZEN_ASSET_EVIDENCE_INVALID",
            Self::PreviewDirectoryInvalid => "PREVIEW_DIR_INVALID",
            Self::PreviewWriteFailed => "PREVIEW_WRITE_FAILED",
            Self::CdseNetworkBusy => "CDSE_NETWORK_POLICY_BUSY",
            Self::Provider(error) => match error.code.as_str() {
                "DL001" => "DL001",
                "ASF002" => "ASF002",
                "ASF_BURST_PYTHON_ADAPTER_REQUIRED" => "ASF_BURST_PYTHON_ADAPTER_REQUIRED",
                "ASF_QUERY_INVALID" => "ASF_QUERY_INVALID",
                "ASF_QUERY_UNSUPPORTED" => "ASF_QUERY_UNSUPPORTED",
                "ASF_PAGINATION_INVALID" => "ASF_PAGINATION_INVALID",
                "ASF_IDENTITY_CONFLICT" => "ASF_IDENTITY_CONFLICT",
                "CATALOGUE_CAPABILITY_UNAVAILABLE" => "CATALOGUE_CAPABILITY_UNAVAILABLE",
                "CATALOGUE_REQUEST_CANCELLED" => "CATALOGUE_REQUEST_CANCELLED",
                "CATALOGUE_REQUEST_DUPLICATE" => "CATALOGUE_REQUEST_DUPLICATE",
                "CATALOGUE_RESPONSE_INVALID" => "CATALOGUE_RESPONSE_INVALID",
                "CATALOGUE_RESPONSE_INCOMPLETE" => "CATALOGUE_RESPONSE_INCOMPLETE",
                "CATALOGUE_RESULT_LIMIT_EXCEEDED" => "CATALOGUE_RESULT_LIMIT_EXCEEDED",
                "CATALOGUE_PRODUCT_INVALID" => "CATALOGUE_PRODUCT_INVALID",
                "CATALOGUE_CONFIGURATION_INVALID" => "CATALOGUE_CONFIGURATION_INVALID",
                "CATALOGUE_CURSOR_UNSUPPORTED" => "CATALOGUE_CURSOR_UNSUPPORTED",
                "CATALOGUE_FILTER_INVALID" => "CATALOGUE_FILTER_INVALID",
                "CATALOGUE_FILTER_UNSUPPORTED" => "CATALOGUE_FILTER_UNSUPPORTED",
                "CATALOGUE_HTTP_CLIENT_FAILURE" => "CATALOGUE_HTTP_CLIENT_FAILURE",
                "CATALOGUE_HTTP_SERVER_FAILURE" => "CATALOGUE_HTTP_SERVER_FAILURE",
                "CATALOGUE_INVALID_POLYGON" => "CATALOGUE_INVALID_POLYGON",
                "CATALOGUE_PAGE_BUDGET_EXCEEDED" => "CATALOGUE_PAGE_BUDGET_EXCEEDED",
                "CATALOGUE_PAGINATION_INVALID" => "CATALOGUE_PAGINATION_INVALID",
                "CATALOGUE_PRODUCT_IDENTITY_CONFLICT" => "CATALOGUE_PRODUCT_IDENTITY_CONFLICT",
                "CATALOGUE_QUERY_INVALID" => "CATALOGUE_QUERY_INVALID",
                "CATALOGUE_REQUEST_BUDGET_EXCEEDED" => "CATALOGUE_REQUEST_BUDGET_EXCEEDED",
                "CATALOGUE_ROUTE_DISABLED" => "CATALOGUE_ROUTE_DISABLED",
                "CATALOGUE_PROVIDER_TIMEOUT" => "CATALOGUE_PROVIDER_TIMEOUT",
                "CATALOGUE_RESPONSE_INVALID_JSON" => "CATALOGUE_RESPONSE_INVALID_JSON",
                "CATALOGUE_RESPONSE_TOO_LARGE" => "CATALOGUE_RESPONSE_TOO_LARGE",
                "CATALOGUE_SPATIAL_PLAN_EXCEEDED" => "CATALOGUE_SPATIAL_PLAN_EXCEEDED",
                "CATALOGUE_TIME_BUDGET_EXCEEDED" => "CATALOGUE_TIME_BUDGET_EXCEEDED",
                "CATALOGUE_TRANSPORT_FAILURE" => "CATALOGUE_TRANSPORT_FAILURE",
                "STAC_BINDING_INVALID" => "STAC_BINDING_INVALID",
                "STAC_QUERY_INVALID" => "STAC_QUERY_INVALID",
                "STAC_CURSOR_INVALID" => "STAC_CURSOR_INVALID",
                "STAC_RESPONSE_INVALID" => "STAC_RESPONSE_INVALID",
                "STAC_IDENTITY_MISMATCH" => "STAC_IDENTITY_MISMATCH",
                "STAC_ASSET_INVALID" => "STAC_ASSET_INVALID",
                "STAC_SEARCH_CANCELLED" => "STAC_SEARCH_CANCELLED",
                "STAC_LIMIT_EXCEEDED" => "STAC_LIMIT_EXCEEDED",
                "STAC_TIMEOUT" => "STAC_TIMEOUT",
                "STAC_NETWORK_ERROR" => "STAC_NETWORK_ERROR",
                "STAC_REDIRECT_REJECTED" => "STAC_REDIRECT_REJECTED",
                "STAC_ACCESS_DENIED" => "STAC_ACCESS_DENIED",
                "STAC_RATE_LIMITED" => "STAC_RATE_LIMITED",
                "STAC_SERVER_ERROR" => "STAC_SERVER_ERROR",
                "STAC_HTTP_ERROR" => "STAC_HTTP_ERROR",
                "PLANETARY_COMPUTER_CANCELLED" => "PLANETARY_COMPUTER_CANCELLED",
                "PLANETARY_COMPUTER_CONFIG_INVALID" => "PLANETARY_COMPUTER_CONFIG_INVALID",
                "PLANETARY_COMPUTER_TRANSPORT_FAILED" => "PLANETARY_COMPUTER_TRANSPORT_FAILED",
                "PLANETARY_COMPUTER_HTTP_REJECTED" => "PLANETARY_COMPUTER_HTTP_REJECTED",
                "PLANETARY_COMPUTER_RESPONSE_TOO_LARGE" => "PLANETARY_COMPUTER_RESPONSE_TOO_LARGE",
                "PLANETARY_COMPUTER_RESPONSE_INVALID" => "PLANETARY_COMPUTER_RESPONSE_INVALID",
                "PLANETARY_COMPUTER_IDENTITY_MISMATCH" => "PLANETARY_COMPUTER_IDENTITY_MISMATCH",
                "MPC_ASSET_CANCELLED" => "MPC_ASSET_CANCELLED",
                "MPC_ASSET_CONFIG_INVALID" => "MPC_ASSET_CONFIG_INVALID",
                "MPC_ASSET_REJECTED" => "MPC_ASSET_REJECTED",
                "MPC_ASSET_SIGN_FAILED" => "MPC_ASSET_SIGN_FAILED",
                "MPC_ASSET_SIGN_RATE_LIMITED" => "MPC_ASSET_SIGN_RATE_LIMITED",
                "MPC_ASSET_SIGN_REJECTED" => "MPC_ASSET_SIGN_REJECTED",
                "MPC_ASSET_SIGN_UNAVAILABLE" => "MPC_ASSET_SIGN_UNAVAILABLE",
                "MPC_ASSET_SIGN_TRANSPORT" => "MPC_ASSET_SIGN_TRANSPORT",
                "MPC_ASSET_SIGN_RESPONSE_INVALID" => "MPC_ASSET_SIGN_RESPONSE_INVALID",
                "MPC_ASSET_PROBE_FAILED" => "MPC_ASSET_PROBE_FAILED",
                "MPC_ASSET_ITEM_FAILED" => "MPC_ASSET_ITEM_FAILED",
                "ASF_COLLECTION_IDENTITY_MISMATCH" => "ASF_COLLECTION_IDENTITY_MISMATCH",
                "ASF_PRODUCT_IDENTITY_MISSING" => "ASF_PRODUCT_IDENTITY_MISSING",
                "ASF_PRODUCT_SCOPE_MISMATCH" => "ASF_PRODUCT_SCOPE_MISMATCH",
                "ASF_PRIMARY_ASSET_MISSING" => "ASF_PRIMARY_ASSET_MISSING",
                "ASF_PRIMARY_ASSET_AMBIGUOUS" => "ASF_PRIMARY_ASSET_AMBIGUOUS",
                "ASF_ASSET_CLASSIFICATION_INVALID" => "ASF_ASSET_CLASSIFICATION_INVALID",
                "ASF_ASSET_GRAPH_LIMIT_EXCEEDED" => "ASF_ASSET_GRAPH_LIMIT_EXCEEDED",
                "BHOONIDHI_PRODUCT_INVALID" => "BHOONIDHI_PRODUCT_INVALID",
                "BHOONIDHI_PRODUCT_IDENTITY_MISMATCH" => "BHOONIDHI_PRODUCT_IDENTITY_MISMATCH",
                "BHOONIDHI_PRODUCT_SCOPE_MISMATCH" => "BHOONIDHI_PRODUCT_SCOPE_MISMATCH",
                "BHOONIDHI_ITEM_INVALID" => "BHOONIDHI_ITEM_INVALID",
                "BHOONIDHI_QUERY_INVALID" => "BHOONIDHI_QUERY_INVALID",
                "BHOONIDHI_CURSOR_INVALID" => "BHOONIDHI_CURSOR_INVALID",
                "BHOONIDHI_RESPONSE_INVALID" => "BHOONIDHI_RESPONSE_INVALID",
                "BHOONIDHI_RESPONSE_TOO_LARGE" => "BHOONIDHI_RESPONSE_TOO_LARGE",
                "BHOONIDHI_PRODUCT_IDENTITY_CONFLICT" => "BHOONIDHI_PRODUCT_IDENTITY_CONFLICT",
                "BHOONIDHI_SEARCH_BUDGET_EXCEEDED" => "BHOONIDHI_SEARCH_BUDGET_EXCEEDED",
                "BHOONIDHI_SEARCH_CANCELLED" => "BHOONIDHI_SEARCH_CANCELLED",
                "BHOONIDHI_AUTH_UNAVAILABLE" => "BHOONIDHI_AUTH_UNAVAILABLE",
                "BHOONIDHI_OAUTH_CONFIGURATION_INVALID" => "BHOONIDHI_OAUTH_CONFIGURATION_INVALID",
                "BHOONIDHI_CREDENTIALS_MISSING" => "BHOONIDHI_CREDENTIALS_MISSING",
                "BHOONIDHI_CREDENTIALS_INVALID" => "BHOONIDHI_CREDENTIALS_INVALID",
                "BHOONIDHI_AUTH_REJECTED" => "BHOONIDHI_AUTH_REJECTED",
                "BHOONIDHI_MAX_SESSIONS" => "BHOONIDHI_MAX_SESSIONS",
                "BHOONIDHI_ACTIVATION_REQUIRED" => "BHOONIDHI_ACTIVATION_REQUIRED",
                "BHOONIDHI_AUTH_TRANSPORT_FAILED" => "BHOONIDHI_AUTH_TRANSPORT_FAILED",
                "BHOONIDHI_AUTH_TIMEOUT" => "BHOONIDHI_AUTH_TIMEOUT",
                "BHOONIDHI_AUTH_RESPONSE_INVALID" => "BHOONIDHI_AUTH_RESPONSE_INVALID",
                "BHOONIDHI_REFRESH_UNAVAILABLE" => "BHOONIDHI_REFRESH_UNAVAILABLE",
                "BHOONIDHI_REFRESH_EXHAUSTED" => "BHOONIDHI_REFRESH_EXHAUSTED",
                "BHOONIDHI_SESSION_CHANGED" => "BHOONIDHI_SESSION_CHANGED",
                "BHOONIDHI_SESSION_UNAVAILABLE" => "BHOONIDHI_SESSION_UNAVAILABLE",
                "BHOONIDHI_SYNCHRONIZATION_FAILED" => "BHOONIDHI_SYNCHRONIZATION_FAILED",
                "BHOONIDHI_UNAUTHORIZED" => "BHOONIDHI_UNAUTHORIZED",
                "BHOONIDHI_FORBIDDEN" => "BHOONIDHI_FORBIDDEN",
                "BHOONIDHI_RATE_LIMITED" => "BHOONIDHI_RATE_LIMITED",
                "BHOONIDHI_SERVER_ERROR" => "BHOONIDHI_SERVER_ERROR",
                "BHOONIDHI_HTTP_ERROR" => "BHOONIDHI_HTTP_ERROR",
                "BHOONIDHI_NETWORK_ERROR" => "BHOONIDHI_NETWORK_ERROR",
                "BHOONIDHI_ENDPOINT_INVALID" => "BHOONIDHI_ENDPOINT_INVALID",
                "BHOONIDHI_CONFIG_INVALID" => "BHOONIDHI_CONFIG_INVALID",
                "NISAR_QUALITY_STATUS_MISMATCH" => "NISAR_QUALITY_STATUS_MISMATCH",
                "CDSE_ODATA_CANCELLED" => "CDSE_ODATA_CANCELLED",
                "CDSE_ODATA_TIMEOUT" => "CDSE_ODATA_TIMEOUT",
                "CDSE_ODATA_HTTP_FAILURE" => "CDSE_ODATA_HTTP_FAILURE",
                "CDSE_ODATA_HTTP_REDIRECT" => "CDSE_ODATA_HTTP_REDIRECT",
                "CDSE_ODATA_HTTP_UNAUTHORIZED" => "CDSE_ODATA_HTTP_UNAUTHORIZED",
                "CDSE_ODATA_HTTP_FORBIDDEN" => "CDSE_ODATA_HTTP_FORBIDDEN",
                "CDSE_ODATA_HTTP_NOT_FOUND" => "CDSE_ODATA_HTTP_NOT_FOUND",
                "CDSE_ODATA_HTTP_RATE_LIMITED" => "CDSE_ODATA_HTTP_RATE_LIMITED",
                "CDSE_ODATA_HTTP_SERVER" => "CDSE_ODATA_HTTP_SERVER",
                "CDSE_ODATA_HTTP_STATUS" => "CDSE_ODATA_HTTP_STATUS",
                "CDSE_ODATA_RESPONSE_TOO_LARGE" => "CDSE_ODATA_RESPONSE_TOO_LARGE",
                "CDSE_ODATA_JSON_INVALID" => "CDSE_ODATA_JSON_INVALID",
                "CDSE_ODATA_PAGE_INVALID" => "CDSE_ODATA_PAGE_INVALID",
                "CDSE_ODATA_UUID_CONFLICT" => "CDSE_ODATA_UUID_CONFLICT",
                "BOUNDARY_QUERY_REQUIRED" => "BOUNDARY_QUERY_REQUIRED",
                "BOUNDARY_PROVIDER_UNAVAILABLE" => "BOUNDARY_PROVIDER_UNAVAILABLE",
                "BOUNDARY_PROVIDER_HTTP_ERROR" => "BOUNDARY_PROVIDER_HTTP_ERROR",
                "BOUNDARY_PROVIDER_RESPONSE_INVALID" => "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                "BOUNDARY_PROVIDER_RESPONSE_TOO_LARGE" => "BOUNDARY_PROVIDER_RESPONSE_TOO_LARGE",
                "BOUNDARY_GEOMETRY_UNAVAILABLE" => "BOUNDARY_GEOMETRY_UNAVAILABLE",
                "BOUNDARY_CHINA_SELECTION_INVALID" => "BOUNDARY_CHINA_SELECTION_INVALID",
                "BOUNDARY_PROVIDER_RATE_LIMITED" => "BOUNDARY_PROVIDER_RATE_LIMITED",
                "BOUNDARY_PROVIDER_NON_DATA_RESPONSE" => "BOUNDARY_PROVIDER_NON_DATA_RESPONSE",
                "BOUNDARY_PROVIDER_REQUEST_INVALID" => "BOUNDARY_PROVIDER_REQUEST_INVALID",
                "BOUNDARY_PROVIDER_IDENTITY_INVALID" => "BOUNDARY_PROVIDER_IDENTITY_INVALID",
                "BOUNDARY_PROVIDER_IDENTITY_MISMATCH" => "BOUNDARY_PROVIDER_IDENTITY_MISMATCH",
                "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE" => "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE",
                "BOUNDARY_ROUTE_REFUSED_DISPUTED" => "BOUNDARY_ROUTE_REFUSED_DISPUTED",
                "BOUNDARY_TAIWAN_FIRST_LOAD_FAILED" => "BOUNDARY_TAIWAN_FIRST_LOAD_FAILED",
                "BOUNDARY_PROVIDER_CONFIGURATION_INVALID" => {
                    "BOUNDARY_PROVIDER_CONFIGURATION_INVALID"
                }
                _ => "V3_PROVIDER_OPERATION_FAILED",
            },
        }
    }
}

impl From<StoreError> for ApplicationServiceError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// Check negotiated Sidecar capabilities before accepting a required Store
/// startup request. Liveness-only probes deliberately remain compatible.
pub fn require_store_readiness_capability(
    capabilities: &[impl AsRef<str>],
    require_store: bool,
) -> Result<(), ApplicationServiceError> {
    if require_store
        && !capabilities
            .iter()
            .any(|capability| capability.as_ref() == STORE_STARTUP_READINESS_CAPABILITY)
    {
        return Err(ApplicationServiceError::CapabilityUnavailable {
            capability: STORE_STARTUP_READINESS_CAPABILITY,
        });
    }
    Ok(())
}

enum BusinessRuntimeFactory {
    Production {
        #[cfg(feature = "nisar-subset-live-acceptance")]
        worker_override: Option<Arc<dyn DownloadWorker>>,
    },
    Fixture {
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Option<Arc<dyn LiveCatalogSearch>>,
        catalogue_route_use: Option<RouteUse>,
        landsat_browse: Option<Arc<dyn LiveLandsatBrowse>>,
        landsat_bundle_options: Option<Arc<dyn LiveLandsatBundleOptions>>,
        landsat_asset_manifest: Option<Arc<dyn LiveLandsatAssetManifest>>,
        worker: Arc<dyn DownloadWorker>,
    },
}

struct ServiceRuntime {
    store: SharedTaskStore,
    dispatcher: Option<Arc<Dispatcher>>,
    scheduler: Option<DownloadScheduler>,
    capability_observations: CapabilityRuntimeObservations,
    /// The production worker's shared Earthdata session, for task admission.
    earthdata_sessions: Option<Arc<dyn insar_download::EarthdataSessionOwner>>,
}

struct NativeDemDownloadWorker {
    transfer_activity: insar_download::TaskTransferActivityRegistry,
    base: Arc<dyn DownloadWorker>,
    port: Arc<dyn DemOperationPort>,
    store: Option<SharedTaskStore>,
    completed_dem_windows: Mutex<BTreeMap<String, DemWindowCompletion>>,
    cdse_session: Option<(Arc<insar_download::CdseOAuthSession>, ProviderNetworkPolicy)>,
}

struct Sentinel2L1cBandWorker {
    transfer_activity: insar_download::TaskTransferActivityRegistry,
    base: Arc<dyn DownloadWorker>,
    cdse_session: Option<Arc<insar_download::CdseOAuthSession>>,
    network_policy: ProviderNetworkPolicy,
}

fn native_dem_download_failure(error: Option<&str>) -> DownloadFailure {
    // Only fixed native categories cross into Store/UI. Never forward arbitrary
    // context: provider errors can contain request URLs or credential material.
    let category = error
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    let reason = match category {
        "NATIVE_DEM_OUTPUT_LIMIT" => {
            "所选范围的原分辨率 DEM 超出当前原生栅格处理上限；未自动缩小范围或降低分辨率"
        }
        "NATIVE_DEM_TILE_LIMIT" => "所选范围需要的 DEM 源瓦片数量超出当前任务上限",
        "NATIVE_DEM_SIZE_LIMIT" => "DEM 源数据大小超出当前任务下载上限",
        "NATIVE_DEM_CREDENTIAL_REQUIRED" => {
            "OpenTopography API key 未配置或被服务拒绝，请检查凭据设置"
        }
        "NATIVE_DEM_CREDENTIAL_UNAVAILABLE" => "无法读取 OpenTopography 凭据，请检查凭据存储状态",
        "NATIVE_DEM_NETWORK_FAILED" => "DEM 网络请求或数据传输失败，请检查网络连接",
        "NATIVE_DEM_PROVIDER_FAILED" => "DEM 数据服务拒绝请求或返回失败响应",
        "NATIVE_DEM_PROVIDER_UNSUPPORTED" | "NATIVE_DEM_PROVIDER_INVALID" => {
            "当前 DEM 产品与数据源组合不受支持"
        }
        "NATIVE_DEM_PROXY_INVALID" => "DEM 代理配置无效，请检查产品网络设置",
        "NATIVE_DEM_HTTP_UNAVAILABLE" => "无法初始化 DEM 网络客户端",
        "NATIVE_DEM_OUTPUT_UNAVAILABLE" => {
            "无法创建、写入或完成 DEM 输出文件，请检查目录权限和磁盘空间"
        }
        "NATIVE_DEM_OUTPUT_ESCAPE" => "DEM 输出路径未通过任务目录安全检查",
        "NATIVE_DEM_INVALID_PAYLOAD" => "DEM 原生任务参数无效",
        "NATIVE_DEM_INVALID_RASTER" => "DEM 源数据未通过栅格有效性检查",
        _ => {
            return DownloadFailure {
                code: "NATIVE_DEM_OPERATION_FAILED".to_owned(),
                reason_zh: "产品 native DEM 操作执行失败".to_owned(),
                retryable: false,
            }
        }
    };
    DownloadFailure {
        code: category.to_owned(),
        reason_zh: reason.to_owned(),
        // Preserve the existing native worker policy; classification alone must
        // not start retries of provider requests or partially produced files.
        retryable: false,
    }
}

#[derive(Default)]
struct DemWindowCompletion {
    item_ids: BTreeSet<String>,
    finalizing: bool,
}

impl NativeDemDownloadWorker {
    fn finalize_dem_windows_if_ready(
        &self,
        work: &DownloadWork,
        operation: &TaskOperationDescriptor,
    ) -> Result<(), String> {
        let window_count = work
            .item
            .payload
            .get("window_count")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        if window_count <= 1 {
            return Ok(());
        }
        let expected_count = usize::try_from(window_count)
            .map_err(|_| "DEM window count exceeds the native task bound".to_owned())?;
        if expected_count != 2 {
            return Err("DEM antimeridian finalization requires exactly two windows".to_owned());
        }

        let expected_item_ids = work
            .task
            .candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .payload
                    .get("window_count")
                    .and_then(Value::as_u64)
                    == Some(window_count)
            })
            .map(|candidate| candidate.item_id.clone())
            .collect::<BTreeSet<_>>();
        if expected_item_ids.len() != expected_count
            || !expected_item_ids.contains(&work.item.item_id)
        {
            return Err("DEM window item identity does not match the frozen task".to_owned());
        }
        let persisted_completed = if let Some(store) = self.store.as_ref() {
            let store = store
                .lock()
                .map_err(|_| "DEM Store completion state is unavailable".to_owned())?;
            store
                .load_download_snapshot(&work.task.task_id)
                .map_err(|_| "DEM Store completion snapshot is unavailable".to_owned())?
                .results
                .into_iter()
                .filter(|result| {
                    result.outcome == DownloadOutcome::Completed
                        && expected_item_ids.contains(&result.item_id)
                })
                .map(|result| result.item_id)
                .collect::<BTreeSet<_>>()
        } else {
            BTreeSet::new()
        };
        let claimed = {
            let mut tasks = self
                .completed_dem_windows
                .lock()
                .map_err(|_| "DEM window completion state is unavailable".to_owned())?;
            let completion = tasks.entry(work.task.task_id.clone()).or_default();
            completion.item_ids.insert(work.item.item_id.clone());
            completion.item_ids.extend(persisted_completed);
            let ready = completion.item_ids == expected_item_ids;
            if ready && !completion.finalizing {
                completion.finalizing = true;
                true
            } else {
                false
            }
        };
        if !claimed {
            return Ok(());
        }

        let mut payload = operation.payload.clone();
        payload.insert(
            "task_id".to_owned(),
            Value::String(work.task.task_id.clone()),
        );
        payload.insert(
            "window_count".to_owned(),
            Value::Number(window_count.into()),
        );
        let result = self
            .port
            .call("dem_finalize_windows", Value::Object(payload));
        let accepted = matches!(
            &result,
            Ok(value) if value.get("ok").and_then(Value::as_bool) == Some(true)
        );
        let mut tasks = self
            .completed_dem_windows
            .lock()
            .map_err(|_| "DEM window completion state is unavailable".to_owned())?;
        if accepted {
            tasks.remove(&work.task.task_id);
            Ok(())
        } else {
            if let Some(completion) = tasks.get_mut(&work.task.task_id) {
                completion.finalizing = false;
            }
            Err(result
                .err()
                .unwrap_or_else(|| "native DEM window finalization failed".to_owned()))
        }
    }
}

impl DownloadWorker for NativeDemDownloadWorker {
    fn capability_runtime_observations(&self) -> Vec<(String, insar_core::RuntimeReadinessState)> {
        self.base.capability_runtime_observations()
    }

    fn task_transfer_sample(&self, task_id: &str) -> Option<insar_download::TaskTransferSample> {
        self.transfer_activity
            .sample_if_known(task_id, std::time::Instant::now())
            .ok()
            .flatten()
            .or_else(|| self.base.task_transfer_sample(task_id))
    }

    fn execute_with_authority(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        plan_freeze: &NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        if work.item.payload.contains_key("optical_derived_spec") {
            return optical_derived::execute_optical_derived_item(
                self.port.as_ref(),
                &self.transfer_activity,
                self.cdse_session.as_ref(),
                work,
                control,
                progress,
            );
        }
        let is_native_dem = work.task.operation.as_ref().is_some_and(|operation| {
            matches!(
                operation.approved_worker_operation.as_str(),
                "dem_download" | "dem_inspect" | "dem_prepare_sarscape" | "dem_convert"
            )
        });
        if is_native_dem {
            self.execute(work, control, progress)
        } else {
            self.base
                .execute_with_authority(work, control, plan_freeze, progress)
        }
    }

    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        if work.item.payload.contains_key("optical_derived_spec") {
            return optical_derived::execute_optical_derived_item(
                self.port.as_ref(),
                &self.transfer_activity,
                self.cdse_session.as_ref(),
                work,
                control,
                progress,
            );
        }
        let Some(operation) = work.task.operation.as_ref() else {
            return self.base.execute(work, control, progress);
        };
        if !matches!(
            operation.approved_worker_operation.as_str(),
            "dem_download" | "dem_inspect" | "dem_prepare_sarscape" | "dem_convert"
        ) {
            return self.base.execute(work, control, progress);
        }

        let task_id = work.task.task_id.clone();
        let item_id = work.item.item_id.clone();
        let mut payload = operation.payload.clone();
        for key in ["bbox", "window_index", "window_count"] {
            if let Some(value) = work.item.payload.get(key).cloned() {
                payload.insert(key.to_owned(), value);
            }
        }
        let destination = if operation.approved_worker_operation == "dem_download" {
            let path = work
                .item
                .payload
                .get("input_path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    work.task
                        .output_root
                        .join(work.item.display_name.replace(['/', '\\'], "_"))
                });
            if !path.is_absolute() || !path.starts_with(&work.task.output_root) {
                return DownloadItemResult {
                    task_id: work.task.task_id.clone(),
                    item_id: work.item.item_id.clone(),
                    outcome: DownloadOutcome::Failed,
                    destination: path,
                    bytes_written: 0,
                    attempt_count: 1,
                    failure: Some(DownloadFailure {
                        code: "NATIVE_DEM_DESTINATION_INVALID".to_owned(),
                        reason_zh: "DEM 输出路径不属于当前任务目录".to_owned(),
                        retryable: false,
                    }),
                    finished_at: chrono::Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true),
                };
            }
            path
        } else {
            work.task
                .output_root
                .join(work.item.display_name.replace(['/', '\\'], "_"))
        };
        if operation.approved_worker_operation == "dem_download" {
            let destination_value = Value::String(destination.to_string_lossy().to_string());
            payload.insert("input_path".to_owned(), destination_value.clone());
            payload.insert("path".to_owned(), destination_value);
        }
        payload.insert("task_id".to_owned(), Value::String(task_id.clone()));
        payload.insert("item_id".to_owned(), Value::String(item_id.clone()));
        payload.insert(
            "destination".to_owned(),
            Value::String(destination.to_string_lossy().to_string()),
        );
        let now = || chrono::Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true);
        progress(DownloadItemProgress {
            task_id: task_id.clone(),
            item_id: item_id.clone(),
            status: DownloadItemStatus::Active,
            bytes_completed: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            attempt: 1,
            updated_at: now(),
        });
        if control.is_cancelled() {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Cancelled,
                destination,
                bytes_written: 0,
                attempt_count: 0,
                failure: None,
                finished_at: now(),
            };
        }
        let operation_result = self.port.call_with_transfer_progress(
            operation.approved_worker_operation.as_str(),
            Value::Object(payload),
            &|| control.is_cancelled() || control.is_paused(),
            &mut |event| {
                let observed_at = std::time::Instant::now();
                let sample =
                    match event {
                        DemTransferEvent::BodyStarted => {
                            self.transfer_activity
                                .body_started(&task_id, &item_id, 1, observed_at)
                        }
                        DemTransferEvent::BytesReceived(bytes) => self
                            .transfer_activity
                            .observe_bytes(&task_id, &item_id, 1, bytes, observed_at),
                        DemTransferEvent::BodyStopped => {
                            self.transfer_activity
                                .body_stopped(&task_id, &item_id, 1, observed_at)
                        }
                    };
                if sample.is_ok() {
                    // Source tile bytes are not the size of the final cropped raster.
                    progress(DownloadItemProgress {
                        task_id: task_id.clone(),
                        item_id: item_id.clone(),
                        status: DownloadItemStatus::Active,
                        bytes_completed: 0,
                        total_bytes: None,
                        bytes_per_second: self
                            .transfer_activity
                            .member_bytes_per_second(&task_id, &item_id, 1)
                            .unwrap_or(0.0),
                        attempt: 1,
                        updated_at: now(),
                    });
                }
            },
        );
        if control.is_cancelled() {
            return DownloadItemResult {
                task_id: task_id.clone(),
                item_id: item_id.clone(),
                outcome: DownloadOutcome::Cancelled,
                destination: destination.clone(),
                bytes_written: 0,
                attempt_count: 0,
                failure: None,
                finished_at: now(),
            };
        }
        if control.is_paused() {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Interrupted,
                destination,
                bytes_written: 0,
                attempt_count: 1,
                failure: None,
                finished_at: now(),
            };
        }
        match operation_result {
            Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(true) => {
                if operation.approved_worker_operation == "dem_download"
                    && self
                        .finalize_dem_windows_if_ready(&work, operation)
                        .is_err()
                {
                    return DownloadItemResult {
                        task_id,
                        item_id,
                        outcome: DownloadOutcome::Failed,
                        destination,
                        bytes_written: 0,
                        attempt_count: 1,
                        failure: Some(DownloadFailure {
                            code: "NATIVE_DEM_FINALIZATION_FAILED".to_owned(),
                            reason_zh: "DEM 跨日期变更线窗口合并或转换失败".to_owned(),
                            retryable: false,
                        }),
                        finished_at: now(),
                    };
                }
                progress(DownloadItemProgress {
                    task_id: task_id.clone(),
                    item_id: item_id.clone(),
                    status: DownloadItemStatus::Completed,
                    bytes_completed: result
                        .get("bytes_written")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    total_bytes: result.get("bytes_written").and_then(Value::as_u64),
                    bytes_per_second: 0.0,
                    attempt: 1,
                    updated_at: now(),
                });
                DownloadItemResult {
                    task_id,
                    item_id,
                    outcome: DownloadOutcome::Completed,
                    destination,
                    bytes_written: result
                        .get("bytes_written")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    attempt_count: 1,
                    failure: None,
                    finished_at: now(),
                }
            }
            result => DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Failed,
                destination,
                bytes_written: 0,
                attempt_count: 1,
                failure: Some(native_dem_download_failure(
                    result.as_ref().err().map(String::as_str),
                )),
                finished_at: now(),
            },
        }
    }
}

impl Sentinel2L1cBandWorker {
    fn execute_band_item(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        use chrono::SecondsFormat;
        let now = || chrono::Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true);
        let task_id = work.task.task_id.clone();
        let item_id = work.item.item_id.clone();
        let fallback_dest = work.task.output_root.join(&work.item.display_name);

        let Some(spec) = work
            .item
            .payload
            .get("sentinel2_l1c_band_spec")
            .and_then(Value::as_object)
        else {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Failed,
                destination: fallback_dest,
                bytes_written: 0,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: "S2_L1C_SPEC_MISSING".to_owned(),
                    reason_zh: "缺少 Sentinel-2 L1C 波段下载规格".to_owned(),
                    retryable: false,
                }),
                finished_at: now(),
            };
        };

        let Some(source_url) = spec.get("source_url").and_then(Value::as_str) else {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Failed,
                destination: fallback_dest,
                bytes_written: 0,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: "S2_L1C_SPEC_INVALID".to_owned(),
                    reason_zh: "Sentinel-2 L1C 波段规格缺少源地址".to_owned(),
                    retryable: false,
                }),
                finished_at: now(),
            };
        };
        let directory = spec
            .get("directory")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && !s.contains(['/', '\\', '\0']) && !s.contains(".."))
            .unwrap_or("sentinel2_l1c");
        let file_name = spec
            .get("file_name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && !s.contains(['/', '\\', '\0']) && !s.contains(".."))
            .unwrap_or(&work.item.display_name);
        let requires_cdse_auth = spec
            .get("requires_cdse_auth")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let expected_size_bytes = spec.get("expected_size_bytes").and_then(Value::as_u64);

        let destination = work.task.output_root.join(directory).join(file_name);
        if !destination.is_absolute() || !destination.starts_with(&work.task.output_root) {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Failed,
                destination,
                bytes_written: 0,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: "S2_L1C_DESTINATION_INVALID".to_owned(),
                    reason_zh: "Sentinel-2 L1C 输出路径不属于当前任务目录".to_owned(),
                    retryable: false,
                }),
                finished_at: now(),
            };
        }

        if let Ok(meta) = std::fs::metadata(&destination) {
            if meta.is_file()
                && meta.len() > 0
                && expected_size_bytes.is_none_or(|exp| meta.len() == exp)
            {
                let len = meta.len();
                progress(DownloadItemProgress {
                    task_id: task_id.clone(),
                    item_id: item_id.clone(),
                    status: DownloadItemStatus::Completed,
                    bytes_completed: len,
                    total_bytes: Some(len),
                    bytes_per_second: 0.0,
                    attempt: 0,
                    updated_at: now(),
                });
                return DownloadItemResult {
                    task_id,
                    item_id,
                    outcome: DownloadOutcome::Skipped,
                    destination,
                    bytes_written: len,
                    attempt_count: 0,
                    failure: None,
                    finished_at: now(),
                };
            }
        }

        progress(DownloadItemProgress {
            task_id: task_id.clone(),
            item_id: item_id.clone(),
            status: DownloadItemStatus::Active,
            bytes_completed: 0,
            total_bytes: expected_size_bytes,
            bytes_per_second: 0.0,
            attempt: 1,
            updated_at: now(),
        });

        if control.is_cancelled() {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Cancelled,
                destination,
                bytes_written: 0,
                attempt_count: 0,
                failure: None,
                finished_at: now(),
            };
        }
        if control.is_paused() {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Interrupted,
                destination,
                bytes_written: 0,
                attempt_count: 1,
                failure: None,
                finished_at: now(),
            };
        }

        let client = match sentinel2_band_delivery::build_optical_http_client(
            self.network_policy,
            "EOStage/Sentinel2L1cTransfer",
            std::time::Duration::from_secs(300),
        ) {
            Ok(client) => client,
            Err(_) => {
                return DownloadItemResult {
                    task_id,
                    item_id,
                    outcome: DownloadOutcome::Failed,
                    destination,
                    bytes_written: 0,
                    attempt_count: 1,
                    failure: Some(DownloadFailure {
                        code: "S2_L1C_HTTP_UNAVAILABLE".to_owned(),
                        reason_zh: "无法初始化 Sentinel-2 L1C 下载客户端".to_owned(),
                        retryable: false,
                    }),
                    finished_at: now(),
                };
            }
        };

        let part_path = destination.with_file_name(format!("{file_name}.part"));
        let _ =
            self.transfer_activity
                .body_started(&task_id, &item_id, 1, std::time::Instant::now());
        let mut received = 0u64;
        let dl_res = sentinel2_band_delivery::download_http_asset_to_path_with_cdse_retry(
            &client,
            source_url,
            &part_path,
            self.cdse_session.as_ref(),
            requires_cdse_auth,
            &|| control.is_cancelled() || control.is_paused(),
            &mut |delta| {
                received = received.saturating_add(delta);
                let observed_at = std::time::Instant::now();
                let _ =
                    self.transfer_activity
                        .observe_bytes(&task_id, &item_id, 1, delta, observed_at);
                progress(DownloadItemProgress {
                    task_id: task_id.clone(),
                    item_id: item_id.clone(),
                    status: DownloadItemStatus::Active,
                    bytes_completed: received,
                    total_bytes: expected_size_bytes,
                    bytes_per_second: self
                        .transfer_activity
                        .member_bytes_per_second(&task_id, &item_id, 1)
                        .unwrap_or(0.0),
                    attempt: 1,
                    updated_at: now(),
                });
            },
        );
        let _ =
            self.transfer_activity
                .body_stopped(&task_id, &item_id, 1, std::time::Instant::now());

        if control.is_cancelled() {
            let _ = std::fs::remove_file(&part_path);
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Cancelled,
                destination,
                bytes_written: 0,
                attempt_count: 0,
                failure: None,
                finished_at: now(),
            };
        }
        if control.is_paused() {
            let _ = std::fs::remove_file(&part_path);
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Interrupted,
                destination,
                bytes_written: 0,
                attempt_count: 1,
                failure: None,
                finished_at: now(),
            };
        }

        match dl_res {
            Ok(bytes) => {
                if let Err(err) = std::fs::rename(&part_path, &destination) {
                    let _ = std::fs::remove_file(&part_path);
                    return DownloadItemResult {
                        task_id,
                        item_id,
                        outcome: DownloadOutcome::Failed,
                        destination,
                        bytes_written: 0,
                        attempt_count: 1,
                        failure: Some(DownloadFailure {
                            code: "S2_L1C_FINALIZE_FAILED".to_owned(),
                            reason_zh: format!("无法保存波段文件到目标目录: {err}"),
                            retryable: false,
                        }),
                        finished_at: now(),
                    };
                }
                progress(DownloadItemProgress {
                    task_id: task_id.clone(),
                    item_id: item_id.clone(),
                    status: DownloadItemStatus::Completed,
                    bytes_completed: bytes,
                    total_bytes: Some(bytes),
                    bytes_per_second: 0.0,
                    attempt: 1,
                    updated_at: now(),
                });
                DownloadItemResult {
                    task_id,
                    item_id,
                    outcome: DownloadOutcome::Completed,
                    destination,
                    bytes_written: bytes,
                    attempt_count: 1,
                    failure: None,
                    finished_at: now(),
                }
            }
            Err((code, reason_zh, retryable)) => {
                let _ = std::fs::remove_file(&part_path);
                DownloadItemResult {
                    task_id,
                    item_id,
                    outcome: DownloadOutcome::Failed,
                    destination,
                    bytes_written: 0,
                    attempt_count: 1,
                    failure: Some(DownloadFailure {
                        code,
                        reason_zh,
                        retryable,
                    }),
                    finished_at: now(),
                }
            }
        }
    }
}

impl DownloadWorker for Sentinel2L1cBandWorker {
    fn capability_runtime_observations(&self) -> Vec<(String, insar_core::RuntimeReadinessState)> {
        self.base.capability_runtime_observations()
    }

    fn task_transfer_sample(&self, task_id: &str) -> Option<insar_download::TaskTransferSample> {
        self.transfer_activity
            .sample_if_known(task_id, std::time::Instant::now())
            .ok()
            .flatten()
            .or_else(|| self.base.task_transfer_sample(task_id))
    }

    fn execute_with_authority(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        plan_freeze: &NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        if work.item.payload.contains_key("sentinel2_l1c_band_spec") {
            return self.execute_band_item(work, control, progress);
        }
        self.base
            .execute_with_authority(work, control, plan_freeze, progress)
    }

    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        if work.item.payload.contains_key("sentinel2_l1c_band_spec") {
            return self.execute_band_item(work, control, progress);
        }
        self.base.execute(work, control, progress)
    }
}

/// Owns exactly one Store lease and, in explicit business mode, the Dispatcher
/// and DownloadScheduler for the selected desktop runtime.
pub struct ApplicationService {
    config: ApplicationServiceConfig,
    boundary_service: boundary_service::BoundaryService,
    business_factory: Option<BusinessRuntimeFactory>,
    catalogue_before_publication: Option<CataloguePublicationHook>,
    catalogue_after_publication: Option<CataloguePublicationHook>,
    runtime: Mutex<Option<ServiceRuntime>>,
    cdse_session: Mutex<Option<CdseSessionRuntime>>,
    bhoonidhi_session: Mutex<Option<BhoonidhiSessionRuntime>>,
    usgs_session: Mutex<Option<UsgsM2mSessionRuntime>>,
    cdse_admission_gate: account_sessions::AccountSessionGate,
    usgs_admission_gate: account_sessions::AccountSessionGate,
    /// Saved network settings changed while transfers were active; applied
    /// to new requests once the runtime is idle.
    network_apply: network_routes::PendingNetworkApply,
    // Installed only by the hidden offline constructor; never restored from Store.
    offline_stac_fixture: Option<Arc<StacOfflineFixtureAdmission>>,
    catalogue_live_acceptance_candidate: bool,
    nisar_subset_fixture_evidence:
        Option<Arc<[insar_provider::asf::nisar_subset::NisarSubsetGeolocationEvidenceV1]>>,
    #[allow(
        dead_code,
        reason = "reserved for Phase-2C final-plan dependency-evidence integration"
    )]
    nisar_subset_v2_fixture_evidence:
        Option<Arc<[nisar_subset::NisarSubsetDependencyEvidenceV2Fixture]>>,
    dem_operation_port: Option<Arc<dyn DemOperationPort>>,
    // Serializes a catalogue request's registration with a CDSE network-policy
    // replacement, preserving the Dispatcher that owns its cancellation lease.
    catalogue_policy_gate: Mutex<()>,
    // Couples authoritative AOI reads used for capability planning/task
    // creation with every mutation that can change the active AOI identity.
    aoi_policy_gate: Mutex<()>,
    gacos_submission_gate: Mutex<Option<Instant>>,
    gacos_result_links: Arc<gacos_result_worker::GacosResultLinkCache>,
    quicklook_cancellations: CancellationRegistry,
    optical_preparations: optical_preparation::OpticalPreparationRegistry,
    /// #22 B slice 3(b): largest single staging chunk the streaming sink has
    /// flushed. Test-only evidence that the in-memory candidate window stays
    /// bounded; never read by production paths.
    catalogue_streaming_max_chunk: std::sync::Mutex<usize>,
    unified_searches: Arc<unified_search::UnifiedSearchRegistry>,
    shutting_down: AtomicBool,
}

pub(crate) type SarContinuationPage = (Vec<SnapshotItem>, Option<String>, Option<u64>, bool);

impl ApplicationService {
    /// Read the exact request's recorded commit fact and target row state.
    /// `Unknown` is not proof that an earlier writer has stopped.
    pub fn query_source_mutation_status(
        &self,
        query: SourceMutationStatusQuery<'_>,
    ) -> Result<SourceMutationStatus, ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            use insar_store::SourceMutationQuery;
            let result = match query {
                SourceMutationStatusQuery::SourceAppend(request) => {
                    store.query_source_mutation_status(SourceMutationQuery::SourceAppend(request))
                }
                SourceMutationStatusQuery::HistoricalSuccessor(request) => store
                    .query_source_mutation_status(SourceMutationQuery::HistoricalSuccessor(
                        request,
                    )),
                SourceMutationStatusQuery::BurstAppend(request) => store
                    .query_source_mutation_status(SourceMutationQuery::BurstAppend(
                        &request.receipt_identity(),
                    )),
                SourceMutationStatusQuery::BurstSuccessor(request) => store
                    .query_source_mutation_status(SourceMutationQuery::BurstSuccessor(
                        &request.receipt_identity(),
                    )),
                SourceMutationStatusQuery::NisarAppend(request) => store
                    .query_source_mutation_status(SourceMutationQuery::NisarAppend(
                        &request.receipt_identity(),
                    )),
                SourceMutationStatusQuery::NisarSuccessor(request) => store
                    .query_source_mutation_status(SourceMutationQuery::NisarSuccessor(
                        &request.receipt_identity(),
                    )),
                SourceMutationStatusQuery::NisarArchivedRecovery(request) => store
                    .query_source_mutation_status(SourceMutationQuery::NisarArchivedRecovery(
                        request,
                    )),
                SourceMutationStatusQuery::Reacquire(request) => {
                    store.query_source_mutation_status(SourceMutationQuery::Reacquire(request))
                }
            };
            result.map_err(ApplicationServiceError::from)
        })
    }
    pub fn new(config: ApplicationServiceConfig) -> Self {
        let business_factory = config
            .is_business()
            .then_some(BusinessRuntimeFactory::Production {
                #[cfg(feature = "nisar-subset-live-acceptance")]
                worker_override: None,
            });
        Self {
            config,
            boundary_service: boundary_service::BoundaryService::production(),
            business_factory,
            catalogue_before_publication: None,
            catalogue_after_publication: None,
            runtime: Mutex::new(None),
            cdse_session: Mutex::new(None),
            bhoonidhi_session: Mutex::new(None),
            usgs_session: Mutex::new(None),
            cdse_admission_gate: account_sessions::AccountSessionGate::default(),
            usgs_admission_gate: account_sessions::AccountSessionGate::default(),
            network_apply: network_routes::PendingNetworkApply::default(),
            catalogue_live_acceptance_candidate: false,
            offline_stac_fixture: None,
            nisar_subset_fixture_evidence: None,
            nisar_subset_v2_fixture_evidence: None,
            dem_operation_port: None,
            catalogue_policy_gate: Mutex::new(()),
            aoi_policy_gate: Mutex::new(()),
            gacos_submission_gate: Mutex::new(None),
            gacos_result_links: Arc::new(
                gacos_result_worker::GacosResultLinkCache::with_desktop_credentials(),
            ),
            quicklook_cancellations: CancellationRegistry::default(),
            optical_preparations: optical_preparation::OpticalPreparationRegistry::default(),
            catalogue_streaming_max_chunk: std::sync::Mutex::new(0),
            unified_searches: Arc::default(),
            shutting_down: AtomicBool::new(false),
        }
    }

    /// Explicit provider injection for offline boundary contract tests. Normal
    /// production construction always uses the product-selected providers.
    #[doc(hidden)]
    pub fn with_boundary_providers(
        config: ApplicationServiceConfig,
        global_provider: Arc<dyn insar_provider::boundary::GlobalBoundaryProvider>,
        china_provider: Arc<dyn insar_provider::boundary::ChinaBoundaryProvider>,
    ) -> Self {
        let mut service = Self::new(config);
        service.boundary_service =
            boundary_service::BoundaryService::with_providers(global_provider, china_provider);
        service
    }

    /// Persist an operation AOI without creating or mutating a saved Region.
    /// The standalone AOI remains dormant while a Region is selected and
    /// becomes active again only after the user leaves the Region context.
    pub fn set_working_aoi(
        &self,
        request: &ApplicationSetWorkingAoiRequest,
    ) -> Result<ApplicationWorkingAoi, ApplicationServiceError> {
        validate_working_aoi(request)
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let store = self.store_handle()?;
        let mut store = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let expected_revision = store.load_standalone_aoi()?.map(|value| value.revision);
        let updated_at = canonical_utc_identity_timestamp(
            &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )
        .ok_or_else(|| ApplicationServiceError::InvalidRequest {
            reason: "working AOI timestamp could not be normalized".to_owned(),
        })?;
        let stored = store.save_standalone_aoi(&StoreStandaloneAoiInput {
            expected_revision,
            source_kind: request.source_kind.trim().to_owned(),
            geometry: request.geometry.clone(),
            bbox: request.bbox.clone(),
            crs: request.crs.clone(),
            display_name: request.display_name.trim().to_owned(),
            updated_at,
        })?;
        Ok(application_working_aoi(
            stored,
            ApplicationAoiState::Working,
        ))
    }

    pub fn working_aoi(&self) -> Result<Option<ApplicationWorkingAoi>, ApplicationServiceError> {
        let store = self.store_handle()?;
        let value = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .load_standalone_aoi()?;
        Ok(value.map(|value| application_working_aoi(value, ApplicationAoiState::Working)))
    }

    pub fn working_aoi_state(&self) -> ApplicationAoiState {
        match self.working_aoi() {
            Ok(Some(value)) => value.state,
            Ok(None) => ApplicationAoiState::NoAoi,
            Err(err) => {
                eprintln!(
                    "[application_service] working_aoi_state fallback to NoAoi: code={}",
                    err.code()
                );
                ApplicationAoiState::NoAoi
            }
        }
    }

    pub fn clear_working_aoi(&self) -> Result<(), ApplicationServiceError> {
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let store = self.store_handle()?;
        let mut store = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let expected_revision = store.load_standalone_aoi()?.map(|value| value.revision);
        store.clear_standalone_aoi(expected_revision)?;
        Ok(())
    }

    fn resolve_sar_request(
        &self,
        request: &ApplicationSarSearchAndFreezeRequest,
    ) -> Result<ApplicationSarSearchAndFreezeRequest, ApplicationServiceError> {
        let mut resolved = request.clone();
        // Slice 3(a): the same clamp the ASF planners apply, before the cache
        // key, the frozen query and progress targets are derived.
        resolved.search.query.max_results =
            insar_core::clamp_search_target(resolved.search.query.max_results);
        if resolved.aoi_geojson.is_none() && resolved.search.query.aoi_geojson.is_none() {
            if let Some(working) = self.active_operation_aoi()? {
                resolved.aoi_geojson = Some(working.geometry.clone());
                resolved.search.query.aoi_geojson = resolved.aoi_geojson.clone();
                resolved.aoi_name = working.display_name;
            }
        }
        // The desktop form carries a bbox for map presentation alongside the
        // authoritative AOI geometry. The formal Burst CMR route accepts only
        // that exact Polygon/MultiPolygon authority; retaining the redundant
        // envelope would make every rectangle and administrative AOI fail as
        // an unsupported second spatial filter.
        if resolved
            .search
            .query
            .dataset
            .eq_ignore_ascii_case("SLC-BURST")
            && resolved.search.query.aoi_geojson.is_some()
        {
            resolved.search.query.bbox = None;
        }
        Ok(resolved)
    }

    fn resolve_catalogue_request(
        &self,
        request: &ApplicationCatalogSearchAndFreezeRequest,
    ) -> Result<ApplicationCatalogSearchAndFreezeRequest, ApplicationServiceError> {
        let mut resolved = request.clone();
        // Slice 3(a): one clamp before validation, cache keys, fallback
        // candidates, execution and statistics.
        resolved.search = insar_core::clamp_catalogue_search_query(&resolved.search);
        if resolved.aoi_geojson.is_none() && resolved.search.aoi_geojson.is_none() {
            if let Some(working) = self.active_operation_aoi()? {
                resolved.aoi_geojson = Some(working.geometry.clone());
                resolved.search.aoi_geojson = resolved.aoi_geojson.clone();
                resolved.aoi_name = working.display_name;
            }
        }
        Ok(resolved)
    }

    fn catalogue_candidate_queries(
        &self,
        seed: &CatalogSearchQuery,
        route_use: RouteUse,
        fixture: bool,
    ) -> Result<Vec<CatalogSearchQuery>, ApplicationServiceError> {
        if route_use != RouteUse::Production
            || !matches!(
                self.business_factory.as_ref(),
                Some(BusinessRuntimeFactory::Production { .. })
            )
            || fixture
            || self.catalogue_live_acceptance_candidate
            || seed.cursor.is_some()
        {
            return Ok(vec![seed.clone()]);
        }
        let Some(candidates) = self
            .capability_query_service()?
            .executable_catalogue_fallback_routes_for_source(
                &seed.logical_dataset,
                &seed.provider_id,
                &seed.native_collection_id,
            )
            .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable)?
        else {
            return Ok(vec![seed.clone()]);
        };
        let mut queries = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let native_collection_id =
                candidate
                    .route
                    .native_collection_id
                    .as_deref()
                    .ok_or_else(|| {
                        catalogue_provider_error(
                            "CATALOGUE_ROUTE_UNAVAILABLE",
                            "已审核的数据来源缺少原生集合标识",
                            "provider_native_collection",
                        )
                    })?;
            DatasetRoutePolicyRegistry
                .resolve(
                    &seed.logical_dataset,
                    &candidate.route.provider_id,
                    native_collection_id,
                    RouteUse::Production,
                )
                .map_err(|_| {
                    catalogue_provider_error(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "已审核的数据来源与目录路由不一致",
                        "provider_route_binding",
                    )
                })?;
            CollectionProfileRegistry
                .for_source(
                    &seed.logical_dataset,
                    &candidate.route.provider_id,
                    native_collection_id,
                )
                .ok_or_else(|| {
                    catalogue_provider_error(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "已审核的数据来源缺少产品绑定",
                        "provider_collection_profile",
                    )
                })?;
            let mut query = seed.clone();
            query.provider_id = candidate.route.provider_id.clone();
            query.native_collection_id = native_collection_id.to_owned();
            queries.push(query);
        }
        if queries.is_empty() {
            return Err(catalogue_provider_error(
                "CATALOGUE_ROUTE_UNAVAILABLE",
                "当前没有可执行的数据来源",
                "eligible_provider_candidates",
            ));
        }
        Ok(queries)
    }

    fn active_operation_aoi(
        &self,
    ) -> Result<Option<ApplicationWorkingAoi>, ApplicationServiceError> {
        let store = self.store_handle()?;
        let store = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        if let Some(context) = store.load_workspace_context()? {
            if let Some(region_id) = context.region_id.as_deref() {
                return store
                    .load_region_aoi_for_workspace(&context.workspace_id, region_id)?
                    .map(|value| {
                        Ok(ApplicationWorkingAoi {
                            state: ApplicationAoiState::SavedRegion,
                            source_kind: value.source_kind,
                            geometry: value.geometry,
                            bbox: value.bbox,
                            crs: value.crs,
                            display_name: value.display_name,
                        })
                    })
                    .transpose();
            }
        }
        Ok(store
            .load_standalone_aoi()?
            .map(|value| application_working_aoi(value, ApplicationAoiState::Working)))
    }

    /// Construct the normal production runtime with the product-owned DEM
    /// operation port. The port is wrapped around the existing Rust scheduler
    /// worker; Store, Scheduler, credentials, and provider transfers remain
    /// owned by this service and the existing Rust download stack.
    pub fn with_dem_operation_port(
        config: ApplicationServiceConfig,
        port: Arc<dyn DemOperationPort>,
    ) -> Self {
        let mut service = Self::new(config);
        service.dem_operation_port = Some(port);
        service
    }

    #[doc(hidden)]
    pub fn attach_dem_operation_port(mut self, port: Arc<dyn DemOperationPort>) -> Self {
        self.dem_operation_port = Some(port);
        self
    }

    /// Construct the production catalogue/Store/Scheduler runtime with the
    /// already-qualified ProductionDownloadWorker supplied by the controlled
    /// NISAR acceptance harness. This is an acceptance-only seam; normal
    /// desktop construction continues to use `ApplicationService::new`.
    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn with_production_worker_for_live_acceptance(
        config: ApplicationServiceConfig,
        worker: Arc<dyn DownloadWorker>,
    ) -> Self {
        assert!(
            config.is_business(),
            "live acceptance runtime requires business configuration"
        );
        Self {
            config,
            boundary_service: boundary_service::BoundaryService::production(),
            business_factory: Some(BusinessRuntimeFactory::Production {
                worker_override: Some(worker),
            }),
            catalogue_before_publication: None,
            catalogue_after_publication: None,
            runtime: Mutex::new(None),
            cdse_session: Mutex::new(None),
            bhoonidhi_session: Mutex::new(None),
            usgs_session: Mutex::new(None),
            cdse_admission_gate: account_sessions::AccountSessionGate::default(),
            usgs_admission_gate: account_sessions::AccountSessionGate::default(),
            network_apply: network_routes::PendingNetworkApply::default(),
            catalogue_live_acceptance_candidate: false,
            offline_stac_fixture: None,
            nisar_subset_fixture_evidence: None,
            nisar_subset_v2_fixture_evidence: None,
            dem_operation_port: None,
            catalogue_policy_gate: Mutex::new(()),
            aoi_policy_gate: Mutex::new(()),
            gacos_submission_gate: Mutex::new(None),
            gacos_result_links: Arc::new(gacos_result_worker::GacosResultLinkCache::default()),
            quicklook_cancellations: CancellationRegistry::default(),
            optical_preparations: optical_preparation::OpticalPreparationRegistry::default(),
            catalogue_streaming_max_chunk: std::sync::Mutex::new(0),
            unified_searches: Arc::default(),
            shutting_down: AtomicBool::new(false),
        }
    }

    /// Construct an explicitly selected fixture business runtime. The injected
    /// ports make offline Provider/transfer evidence possible without credentials
    /// or external network access.
    #[doc(hidden)]
    pub fn with_fixture_business_runtime(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        worker: Arc<dyn DownloadWorker>,
    ) -> Self {
        assert!(
            config.is_business(),
            "fixture runtime requires business configuration"
        );
        Self {
            config,
            boundary_service: boundary_service::BoundaryService::production(),
            business_factory: Some(BusinessRuntimeFactory::Fixture {
                search,
                catalog_search: None,
                catalogue_route_use: None,
                landsat_browse: None,
                landsat_bundle_options: None,
                landsat_asset_manifest: None,
                worker,
            }),
            catalogue_before_publication: None,
            catalogue_after_publication: None,
            runtime: Mutex::new(None),
            cdse_session: Mutex::new(None),
            bhoonidhi_session: Mutex::new(None),
            usgs_session: Mutex::new(None),
            cdse_admission_gate: account_sessions::AccountSessionGate::default(),
            usgs_admission_gate: account_sessions::AccountSessionGate::default(),
            network_apply: network_routes::PendingNetworkApply::default(),
            catalogue_live_acceptance_candidate: false,
            offline_stac_fixture: None,
            nisar_subset_fixture_evidence: None,
            nisar_subset_v2_fixture_evidence: None,
            dem_operation_port: None,
            catalogue_policy_gate: Mutex::new(()),
            aoi_policy_gate: Mutex::new(()),
            gacos_submission_gate: Mutex::new(None),
            gacos_result_links: Arc::new(gacos_result_worker::GacosResultLinkCache::default()),
            quicklook_cancellations: CancellationRegistry::default(),
            optical_preparations: optical_preparation::OpticalPreparationRegistry::default(),
            catalogue_streaming_max_chunk: std::sync::Mutex::new(0),
            unified_searches: Arc::default(),
            shutting_down: AtomicBool::new(false),
        }
    }

    /// Construct a fixture-only business runtime with the disabled catalogue
    /// port. Production construction never injects this port.
    #[doc(hidden)]
    pub fn with_catalogue_fixture_business_runtime(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        worker: Arc<dyn DownloadWorker>,
    ) -> Self {
        Self::with_catalogue_fixture_business_runtime_with_publication_hook(
            config,
            search,
            catalog_search,
            worker,
            None,
        )
    }

    /// Test-only composition of the normal Store/publication pipeline with one
    /// immutable process-local admission. No production route is added.
    #[doc(hidden)]
    pub fn with_offline_stac_fixture_business_runtime(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        worker: Arc<dyn DownloadWorker>,
        admission: Arc<StacOfflineFixtureAdmission>,
    ) -> Self {
        let mut service =
            Self::with_catalogue_fixture_business_runtime(config, search, catalog_search, worker);
        service.offline_stac_fixture = Some(admission);
        service
    }

    /// Explicit offline-only NISAR Remote Subset fixture runtime. The evidence
    /// is typed, process-local and contains no URL, credential or source path.
    /// Production construction cannot enable this capability.
    #[doc(hidden)]
    pub fn with_offline_nisar_subset_fixture(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        worker: Arc<dyn DownloadWorker>,
        evidence: Vec<insar_provider::asf::nisar_subset::NisarSubsetGeolocationEvidenceV1>,
    ) -> Self {
        assert!(
            !evidence.is_empty(),
            "offline NISAR subset fixture requires typed geolocation evidence"
        );
        let mut service =
            Self::with_catalogue_fixture_business_runtime(config, search, catalog_search, worker);
        service.nisar_subset_fixture_evidence = Some(evidence.into());
        service
    }

    /// Explicit fixture-only V2 dependency authority reserved for later final-plan
    /// construction. The graph is process-local, cannot be deserialized from a
    /// request, and is deliberately irrelevant to prepared-intent creation.
    #[doc(hidden)]
    pub fn with_offline_nisar_subset_v2_fixture(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        worker: Arc<dyn DownloadWorker>,
        evidence: Vec<nisar_subset::NisarSubsetDependencyEvidenceV2Fixture>,
    ) -> Self {
        assert!(
            !evidence.is_empty(),
            "offline NISAR V2 fixture requires typed dependency evidence"
        );
        let mut service =
            Self::with_catalogue_fixture_business_runtime(config, search, catalog_search, worker);
        service.nisar_subset_v2_fixture_evidence = Some(evidence.into());
        service
    }

    /// Construct an offline-injected business runtime that exercises the closed
    /// production catalogue route without constructing a public-network client.
    /// It exists only for production-freeze integration tests; the real desktop
    /// construction remains `BusinessRuntimeFactory::Production`.
    #[doc(hidden)]
    pub fn with_catalogue_production_business_runtime(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        worker: Arc<dyn DownloadWorker>,
    ) -> Self {
        assert!(
            config.is_business(),
            "fixture runtime requires business configuration"
        );
        Self {
            config,
            boundary_service: boundary_service::BoundaryService::production(),
            business_factory: Some(BusinessRuntimeFactory::Fixture {
                search,
                catalog_search: Some(catalog_search),
                catalogue_route_use: Some(RouteUse::Production),
                landsat_browse: None,
                landsat_bundle_options: None,
                landsat_asset_manifest: None,
                worker,
            }),
            catalogue_before_publication: None,
            catalogue_after_publication: None,
            runtime: Mutex::new(None),
            cdse_session: Mutex::new(None),
            bhoonidhi_session: Mutex::new(None),
            usgs_session: Mutex::new(None),
            cdse_admission_gate: account_sessions::AccountSessionGate::default(),
            usgs_admission_gate: account_sessions::AccountSessionGate::default(),
            network_apply: network_routes::PendingNetworkApply::default(),
            catalogue_live_acceptance_candidate: false,
            offline_stac_fixture: None,
            nisar_subset_fixture_evidence: None,
            nisar_subset_v2_fixture_evidence: None,
            dem_operation_port: None,
            catalogue_policy_gate: Mutex::new(()),
            aoi_policy_gate: Mutex::new(()),
            gacos_submission_gate: Mutex::new(None),
            gacos_result_links: Arc::new(gacos_result_worker::GacosResultLinkCache::default()),
            quicklook_cancellations: CancellationRegistry::default(),
            optical_preparations: optical_preparation::OpticalPreparationRegistry::default(),
            catalogue_streaming_max_chunk: std::sync::Mutex::new(0),
            unified_searches: Arc::default(),
            shutting_down: AtomicBool::new(false),
        }
    }

    #[doc(hidden)]
    pub fn with_catalogue_production_business_runtime_and_landsat_browse(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        landsat_browse: Arc<dyn LiveLandsatBrowse>,
        worker: Arc<dyn DownloadWorker>,
    ) -> Self {
        assert!(
            config.is_business(),
            "fixture runtime requires business configuration"
        );
        Self {
            config,
            boundary_service: boundary_service::BoundaryService::production(),
            business_factory: Some(BusinessRuntimeFactory::Fixture {
                search,
                catalog_search: Some(catalog_search),
                catalogue_route_use: Some(RouteUse::Production),
                landsat_browse: Some(landsat_browse),
                landsat_bundle_options: None,
                landsat_asset_manifest: None,
                worker,
            }),
            catalogue_before_publication: None,
            catalogue_after_publication: None,
            runtime: Mutex::new(None),
            cdse_session: Mutex::new(None),
            bhoonidhi_session: Mutex::new(None),
            usgs_session: Mutex::new(None),
            cdse_admission_gate: account_sessions::AccountSessionGate::default(),
            usgs_admission_gate: account_sessions::AccountSessionGate::default(),
            network_apply: network_routes::PendingNetworkApply::default(),
            catalogue_live_acceptance_candidate: false,
            offline_stac_fixture: None,
            nisar_subset_fixture_evidence: None,
            nisar_subset_v2_fixture_evidence: None,
            dem_operation_port: None,
            catalogue_policy_gate: Mutex::new(()),
            aoi_policy_gate: Mutex::new(()),
            gacos_submission_gate: Mutex::new(None),
            gacos_result_links: Arc::new(gacos_result_worker::GacosResultLinkCache::default()),
            quicklook_cancellations: CancellationRegistry::default(),
            optical_preparations: optical_preparation::OpticalPreparationRegistry::default(),
            catalogue_streaming_max_chunk: std::sync::Mutex::new(0),
            unified_searches: Arc::default(),
            shutting_down: AtomicBool::new(false),
        }
    }

    #[doc(hidden)]
    pub fn with_catalogue_production_business_runtime_and_landsat_delivery(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        landsat_bundle_options: Arc<dyn LiveLandsatBundleOptions>,
        worker: Arc<dyn DownloadWorker>,
    ) -> Self {
        assert!(
            config.is_business(),
            "fixture runtime requires business configuration"
        );
        Self {
            config,
            boundary_service: boundary_service::BoundaryService::production(),
            business_factory: Some(BusinessRuntimeFactory::Fixture {
                search,
                catalog_search: Some(catalog_search),
                catalogue_route_use: Some(RouteUse::Production),
                landsat_browse: None,
                landsat_bundle_options: Some(landsat_bundle_options),
                landsat_asset_manifest: None,
                worker,
            }),
            catalogue_before_publication: None,
            catalogue_after_publication: None,
            runtime: Mutex::new(None),
            cdse_session: Mutex::new(None),
            bhoonidhi_session: Mutex::new(None),
            usgs_session: Mutex::new(None),
            cdse_admission_gate: account_sessions::AccountSessionGate::default(),
            usgs_admission_gate: account_sessions::AccountSessionGate::default(),
            network_apply: network_routes::PendingNetworkApply::default(),
            catalogue_live_acceptance_candidate: false,
            offline_stac_fixture: None,
            nisar_subset_fixture_evidence: None,
            nisar_subset_v2_fixture_evidence: None,
            dem_operation_port: None,
            catalogue_policy_gate: Mutex::new(()),
            aoi_policy_gate: Mutex::new(()),
            gacos_submission_gate: Mutex::new(None),
            gacos_result_links: Arc::new(gacos_result_worker::GacosResultLinkCache::default()),
            quicklook_cancellations: CancellationRegistry::default(),
            optical_preparations: optical_preparation::OpticalPreparationRegistry::default(),
            catalogue_streaming_max_chunk: std::sync::Mutex::new(0),
            unified_searches: Arc::default(),
            shutting_down: AtomicBool::new(false),
        }
    }

    #[doc(hidden)]
    pub fn with_catalogue_production_business_runtime_and_landsat_assets(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        landsat_asset_manifest: Arc<dyn LiveLandsatAssetManifest>,
        worker: Arc<dyn DownloadWorker>,
    ) -> Self {
        assert!(
            config.is_business(),
            "fixture runtime requires business configuration"
        );
        Self {
            config,
            boundary_service: boundary_service::BoundaryService::production(),
            business_factory: Some(BusinessRuntimeFactory::Fixture {
                search,
                catalog_search: Some(catalog_search),
                catalogue_route_use: Some(RouteUse::Production),
                landsat_browse: None,
                landsat_bundle_options: None,
                landsat_asset_manifest: Some(landsat_asset_manifest),
                worker,
            }),
            catalogue_before_publication: None,
            catalogue_after_publication: None,
            runtime: Mutex::new(None),
            cdse_session: Mutex::new(None),
            bhoonidhi_session: Mutex::new(None),
            usgs_session: Mutex::new(None),
            cdse_admission_gate: account_sessions::AccountSessionGate::default(),
            usgs_admission_gate: account_sessions::AccountSessionGate::default(),
            network_apply: network_routes::PendingNetworkApply::default(),
            catalogue_live_acceptance_candidate: false,
            offline_stac_fixture: None,
            nisar_subset_fixture_evidence: None,
            nisar_subset_v2_fixture_evidence: None,
            dem_operation_port: None,
            catalogue_policy_gate: Mutex::new(()),
            aoi_policy_gate: Mutex::new(()),
            gacos_submission_gate: Mutex::new(None),
            gacos_result_links: Arc::new(gacos_result_worker::GacosResultLinkCache::default()),
            quicklook_cancellations: CancellationRegistry::default(),
            optical_preparations: optical_preparation::OpticalPreparationRegistry::default(),
            catalogue_streaming_max_chunk: std::sync::Mutex::new(0),
            unified_searches: Arc::default(),
            shutting_down: AtomicBool::new(false),
        }
    }

    #[cfg(feature = "usgs-m2m-live-acceptance")]
    #[doc(hidden)]
    pub fn with_catalogue_live_acceptance_business_runtime(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        worker: Arc<dyn DownloadWorker>,
    ) -> Self {
        assert!(
            config.is_business(),
            "live acceptance runtime requires business configuration"
        );
        Self {
            config,
            boundary_service: boundary_service::BoundaryService::production(),
            business_factory: Some(BusinessRuntimeFactory::Fixture {
                search,
                catalog_search: Some(catalog_search),
                catalogue_route_use: Some(RouteUse::Production),
                landsat_browse: None,
                landsat_bundle_options: None,
                landsat_asset_manifest: None,
                worker,
            }),
            catalogue_before_publication: None,
            catalogue_after_publication: None,
            runtime: Mutex::new(None),
            cdse_session: Mutex::new(None),
            bhoonidhi_session: Mutex::new(None),
            usgs_session: Mutex::new(None),
            cdse_admission_gate: account_sessions::AccountSessionGate::default(),
            usgs_admission_gate: account_sessions::AccountSessionGate::default(),
            network_apply: network_routes::PendingNetworkApply::default(),
            catalogue_live_acceptance_candidate: true,
            offline_stac_fixture: None,
            nisar_subset_fixture_evidence: None,
            nisar_subset_v2_fixture_evidence: None,
            dem_operation_port: None,
            catalogue_policy_gate: Mutex::new(()),
            aoi_policy_gate: Mutex::new(()),
            gacos_submission_gate: Mutex::new(None),
            gacos_result_links: Arc::new(gacos_result_worker::GacosResultLinkCache::default()),
            quicklook_cancellations: CancellationRegistry::default(),
            optical_preparations: optical_preparation::OpticalPreparationRegistry::default(),
            catalogue_streaming_max_chunk: std::sync::Mutex::new(0),
            unified_searches: Arc::default(),
            shutting_down: AtomicBool::new(false),
        }
    }

    /// Test-only synchronization point immediately before the catalogue lease
    /// arbitrates publication. It is unavailable from production construction.
    #[doc(hidden)]
    pub fn with_catalogue_fixture_business_runtime_with_publication_hook(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        worker: Arc<dyn DownloadWorker>,
        catalogue_before_publication: Option<CataloguePublicationHook>,
    ) -> Self {
        Self::with_catalogue_fixture_business_runtime_with_publication_hooks(
            config,
            search,
            catalog_search,
            worker,
            catalogue_before_publication,
            None,
        )
    }

    /// Test-only publication synchronization. The before and after hooks are
    /// unavailable from production construction and exist solely to prove the
    /// cancellation/publication arbitration around the Store transaction.
    #[doc(hidden)]
    pub fn with_catalogue_fixture_business_runtime_with_publication_hooks(
        config: ApplicationServiceConfig,
        search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        worker: Arc<dyn DownloadWorker>,
        catalogue_before_publication: Option<CataloguePublicationHook>,
        catalogue_after_publication: Option<CataloguePublicationHook>,
    ) -> Self {
        assert!(
            config.is_business(),
            "fixture runtime requires business configuration"
        );
        Self {
            config,
            boundary_service: boundary_service::BoundaryService::production(),
            business_factory: Some(BusinessRuntimeFactory::Fixture {
                search,
                catalog_search: Some(catalog_search),
                catalogue_route_use: Some(RouteUse::OfflineFixture),
                landsat_browse: None,
                landsat_bundle_options: None,
                landsat_asset_manifest: None,
                worker,
            }),
            catalogue_before_publication,
            catalogue_after_publication,
            runtime: Mutex::new(None),
            cdse_session: Mutex::new(None),
            bhoonidhi_session: Mutex::new(None),
            usgs_session: Mutex::new(None),
            cdse_admission_gate: account_sessions::AccountSessionGate::default(),
            usgs_admission_gate: account_sessions::AccountSessionGate::default(),
            network_apply: network_routes::PendingNetworkApply::default(),
            catalogue_live_acceptance_candidate: false,
            offline_stac_fixture: None,
            nisar_subset_fixture_evidence: None,
            nisar_subset_v2_fixture_evidence: None,
            dem_operation_port: None,
            catalogue_policy_gate: Mutex::new(()),
            aoi_policy_gate: Mutex::new(()),
            gacos_submission_gate: Mutex::new(None),
            gacos_result_links: Arc::new(gacos_result_worker::GacosResultLinkCache::default()),
            quicklook_cancellations: CancellationRegistry::default(),
            optical_preparations: optical_preparation::OpticalPreparationRegistry::default(),
            catalogue_streaming_max_chunk: std::sync::Mutex::new(0),
            unified_searches: Arc::default(),
            shutting_down: AtomicBool::new(false),
        }
    }

    pub fn status(&self) -> Result<ServiceStatus, ApplicationServiceError> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Ok(ServiceStatus::ShuttingDown);
        }
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        if runtime.is_some() {
            Ok(ServiceStatus::StoreReady)
        } else if self.config.requires_store() || self.config.store_path().is_some() {
            Ok(ServiceStatus::Created)
        } else {
            Ok(ServiceStatus::LivenessOnly)
        }
    }

    pub fn store_path(&self) -> Option<&Path> {
        self.config.store_path()
    }

    /// Open and health-check the configured Store exactly once. Business mode
    /// creates its Dispatcher and scheduler against that same shared Store lease.
    pub fn startup(&self) -> Result<ServiceReadiness, ApplicationServiceError> {
        self.ensure_running()?;
        if !self.config.requires_store() && self.config.store_path().is_none() {
            return Ok(ServiceReadiness::liveness_only());
        }

        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        self.ensure_running()?;
        if runtime.is_none() {
            let path = self
                .config
                .store_path()
                .ok_or(ApplicationServiceError::StorePathRequired)?;
            let mut store = if self.config.is_verification_only() {
                TaskStore::open_verification_read_only(path)?
            } else if self.config.requires_existing_store() {
                TaskStore::open_existing(path)?
            } else {
                TaskStore::open(path)?
            };
            if !self.config.is_verification_only() {
                // Another desktop instance may still own a live POST. Only
                // recover attempts after acquiring the same native send lease.
                if let Ok(_gacos_guard) = gacos_service::lock_gacos_submissions(path) {
                    store.recover_gacos_submitting_attempts(
                        None,
                        &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    )?;
                }
                // #22 B slice 3(c): a search still loading when its process
                // ended is sealed as interrupted (possibly incomplete) with
                // its arrived rows; it never silently reads as complete.
                if self.config.is_business() {
                    store.interrupt_loading_search_batches()?;
                    if matches!(
                        self.business_factory,
                        Some(BusinessRuntimeFactory::Production { .. })
                    ) {
                        if insar_download::active_default_ssd_staging_dir().is_none() {
                            if let Some(app_data_dir) = path.parent() {
                                insar_download::set_active_default_ssd_staging_dir(Some(
                                    insar_download::default_ssd_staging_dir_for_app_data(
                                        app_data_dir,
                                    ),
                                ));
                            }
                        }
                        let now_epoch = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let admin_cutoff_iso = (chrono::Utc::now()
                            - chrono::Duration::seconds(
                                TaskStore::ADMIN_BOUNDARY_CACHE_DEFAULT_TTL_SECONDS as i64,
                            ))
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                        let outcome =
                            store.prune_all_disposable_caches(now_epoch, &admin_cutoff_iso)?;
                        if outcome.reclaimed_bytes >= 16 * 1024 * 1024 {
                            let _ = store.reclaim_sqlite_space(true)?;
                        } else if outcome.removed_entries > 0 || outcome.reclaimed_bytes > 0 {
                            let _ = store.reclaim_sqlite_space(false)?;
                        }
                    } else {
                        let _ = store.prune_disposable_search_cache_bounds(
                            0,
                            TaskStore::SEARCH_CACHE_DEFAULT_TTL_SECONDS,
                            TaskStore::SEARCH_CACHE_MAX_ENTRIES,
                            TaskStore::SEARCH_CACHE_MAX_BYTES,
                        )?;
                    }
                }
            }
            let shared_store = Arc::new(Mutex::new(store));
            *runtime = Some(self.build_runtime(shared_store)?);
        }
        let health = runtime
            .as_ref()
            .expect("runtime initialized before health")
            .store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .health()?;
        Ok(ServiceReadiness::store_ready(&health))
    }

    pub fn health(&self) -> Result<StoreHealth, ApplicationServiceError> {
        let store = self.store_handle()?;
        let health = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .health()
            .map_err(ApplicationServiceError::from)?;
        Ok(health)
    }

    pub fn restore(
        &self,
        include_terminal: bool,
    ) -> Result<StoreRestoreResponse, ApplicationServiceError> {
        self.restore_request(&StoreRestoreRequest { include_terminal })
    }

    pub fn restore_request(
        &self,
        request: &StoreRestoreRequest,
    ) -> Result<StoreRestoreResponse, ApplicationServiceError> {
        self.ensure_running()?;
        let store = self.store_handle()?;
        let restore = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .restore(request)
            .map_err(ApplicationServiceError::from)?;
        Ok(restore)
    }

    pub fn snapshot(
        &self,
        include_terminal: bool,
    ) -> Result<ReadonlyServiceSnapshot, ApplicationServiceError> {
        let readiness = self.startup()?;
        let restore = self.restore(include_terminal)?;
        Ok(ReadonlyServiceSnapshot { readiness, restore })
    }

    /// Bounded startup/status restore.
    ///
    /// This is the only normal startup and status-poll restore path. It never
    /// deserializes or transports workbench candidate arrays or task candidate
    /// payloads; the full `snapshot`/`restore` route remains only for
    /// compatibility and focused tests.
    pub fn core_restore(
        &self,
        include_terminal: bool,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        let readiness = self.startup()?;
        let store = self.store_handle()?;
        let core = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .core_restore(include_terminal)
            .map_err(ApplicationServiceError::from)?;
        Ok(CoreServiceSnapshot::from_parts(readiness, core))
    }

    /// Bounded workbench page read through the active Store selection revision.
    pub fn workbench_page(
        &self,
        request: &StoreWorkbenchPageRequest,
    ) -> Result<ApplicationWorkbenchPage, ApplicationServiceError> {
        let readiness = self.startup()?;
        let store = self.store_handle()?;
        let guard = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let core = guard
            .core_restore(false)
            .map_err(ApplicationServiceError::from)?;
        let core_snapshot = CoreServiceSnapshot::from_parts(readiness.clone(), core);
        let page = guard
            .load_workbench_page(request)
            .map_err(ApplicationServiceError::from)?;
        let render = derive_workbench_page_render(&page, core_snapshot.aoi_display.as_ref());
        Ok(ApplicationWorkbenchPage {
            readiness,
            page,
            render,
        })
    }

    /// Read one immutable SearchBatch page with Store-owned task row states.
    pub fn batch_page(
        &self,
        request: &StoreBatchPageRequest,
    ) -> Result<StoreBatchPage, ApplicationServiceError> {
        self.startup()?;
        let store = self.store_handle()?;
        let result = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .load_batch_page(request)
            .map_err(ApplicationServiceError::from);
        result
    }

    /// Bounded page of the selected items in one persisted task. The Store
    /// expands the compact task snapshot internally and only this page crosses
    /// the ApplicationService boundary.
    pub fn task_page(
        &self,
        request: &StoreTaskPageRequest,
    ) -> Result<StoreTaskPage, ApplicationServiceError> {
        self.startup()?;
        let store = self.store_handle()?;
        let result = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .load_task_page(request)
            .map_err(ApplicationServiceError::from);
        result
    }

    /// Bounded read-only projection of the immutable SearchBatch referenced by
    /// a task. Runtime revision/status are observed and returned, never used as
    /// a stale-read gate.
    pub fn task_source_batch_page(
        &self,
        request: &StoreTaskSourceBatchPageRequest,
    ) -> Result<StoreTaskSourceBatchPage, ApplicationServiceError> {
        self.startup()?;
        let store = self.store_handle()?;
        let result = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .load_task_source_batch_page(request)
            .map_err(ApplicationServiceError::from);
        result
    }

    /// Bounded archive read kept separate from the mutable status restore.
    pub fn task_history_page(
        &self,
        request: &StoreTaskHistoryPageRequest,
    ) -> Result<StoreTaskHistoryPage, ApplicationServiceError> {
        self.startup()?;
        let store = self.store_handle()?;
        let result = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .load_task_history_page(request)
            .map_err(ApplicationServiceError::from);
        result
    }

    /// Revisioned, bounded task event page. Raw task event payload/error JSON
    /// remains inside Store and never crosses this application boundary.
    pub fn task_event_page(
        &self,
        request: &StoreTaskEventPageRequest,
    ) -> Result<StoreTaskEventPage, ApplicationServiceError> {
        self.startup()?;
        let store = self.store_handle()?;
        let result = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .load_task_event_page(request)
            .map_err(ApplicationServiceError::from);
        result
    }

    /// Resolve the persisted task output authority. The transport may use this
    /// exact result for a native shell action; callers never supply a path.
    pub fn resolve_task_output_directory(
        &self,
        request: &StoreTaskOutputDirectoryRequest,
    ) -> Result<StoreTaskOutputDirectory, ApplicationServiceError> {
        self.startup()?;
        let store = self.store_handle()?;
        let result = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .resolve_task_output_directory(request)
            .map_err(ApplicationServiceError::from);
        result
    }

    /// B15: enumerate the renamed-aside conflict files of one task's own item
    /// target directories. Read-only; the user decides what to delete.
    pub fn list_task_conflict_files(
        &self,
        task_id: &str,
    ) -> Result<TaskConflictFileList, ApplicationServiceError> {
        let (root, targets, recorded, active) = self.task_conflict_context(task_id)?;
        if active {
            return Err(ApplicationServiceError::InvalidRequest {
                reason:
                    "task is active; conflict files can be cleaned up once it pauses or finishes"
                        .to_owned(),
            });
        }
        if !root.exists() || (targets.is_empty() && recorded.is_empty()) {
            return Ok(TaskConflictFileList {
                task_id: task_id.to_owned(),
                files: Vec::new(),
                total_count: 0,
                total_size_bytes: 0,
            });
        }
        let mut listed =
            insar_download::conflict_cleanup::list_conflict_files(&root, &targets, &recorded)
                .map_err(|error| ApplicationServiceError::InvalidRequest {
                    reason: error.to_string(),
                })?;
        listed.task_id = task_id.to_owned();
        listed
            .validate()
            .map_err(|error| ApplicationServiceError::InvalidRequest { reason: error })?;
        Ok(listed)
    }

    /// B15: delete the conflict files the user confirmed, each by its exact
    /// (path, size, mtime) from the listing. Per-file outcomes are returned;
    /// missing or changed files are skipped, never deleted.
    pub fn delete_task_conflict_files(
        &self,
        task_id: &str,
        expected: &[(String, u64, u64)],
    ) -> Result<TaskConflictFileDeleteResult, ApplicationServiceError> {
        if expected.len() > 10_000 {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "too many conflict files in one deletion request".to_owned(),
            });
        }
        let (root, targets, _recorded, active) = self.task_conflict_context(task_id)?;
        if active {
            return Err(ApplicationServiceError::InvalidRequest {
                reason:
                    "task is active; conflict files can be cleaned up once it pauses or finishes"
                        .to_owned(),
            });
        }
        let mut result =
            insar_download::conflict_cleanup::delete_conflict_files(&root, &targets, expected)
                .map_err(|error| ApplicationServiceError::InvalidRequest {
                    reason: error.to_string(),
                })?;
        result.task_id = task_id.to_owned();
        result
            .validate()
            .map_err(|error| ApplicationServiceError::InvalidRequest { reason: error })?;
        Ok(result)
    }

    /// The persisted authority a conflict cleanup reads: the task's output
    /// root, its own item target directories and base file names, the conflict
    /// names the local reuse ledger recorded, and whether the task is active.
    /// The Store stays the data owner; only bounded projections cross here.
    fn task_conflict_context(
        &self,
        task_id: &str,
    ) -> Result<
        (
            PathBuf,
            Vec<insar_download::conflict_cleanup::ConflictTarget>,
            Vec<String>,
            bool,
        ),
        ApplicationServiceError,
    > {
        self.startup()?;
        if task_id.is_empty() || task_id.chars().count() > 128 {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "task id is invalid".to_owned(),
            });
        }
        let store = self.store_handle()?;
        let store = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let task = store
            .load_task(task_id)?
            .ok_or(ApplicationServiceError::Store(StoreError::NotFound))?;
        let active = matches!(
            task.runtime.status,
            TaskLifecycleStatus::Running | TaskLifecycleStatus::Queued
        ) || (!task.runtime.task_paused && !task.runtime.active_item_ids.is_empty());
        let snapshot = task.snapshot;
        let recorded = store
            .task_local_reuse_decisions(task_id)?
            .into_iter()
            .filter_map(|decision| match decision.disposition {
                insar_store::StoreLocalReuseDisposition::RenamedConflict {
                    conflict_file_name,
                    ..
                } => Some(conflict_file_name),
                _ => None,
            })
            .collect::<Vec<_>>();
        let download_results = store
            .load_download_snapshot(task_id)
            .map(|download| download.results)
            .unwrap_or_default();
        // The item's own recorded destination is the strongest target
        // authority: the transfer wrote it, so its directory owns the item's
        // conflict copies.
        let mut targets: Vec<insar_download::conflict_cleanup::ConflictTarget> = Vec::new();
        for result in &download_results {
            let Some(file_name) = result.destination.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(directory) = result.destination.parent() else {
                continue;
            };
            match targets
                .iter_mut()
                .find(|target| target.directory == directory)
            {
                Some(target) => {
                    if !target.file_names.iter().any(|name| name == file_name) {
                        target.file_names.push(file_name.to_owned());
                    }
                }
                None => targets.push(insar_download::conflict_cleanup::ConflictTarget {
                    directory: directory.to_path_buf(),
                    file_names: vec![file_name.to_owned()],
                }),
            }
        }
        // Items that never finished still have planned destinations derived
        // from the frozen snapshot the same way the worker plans them.
        for item in &snapshot.candidates {
            if !snapshot
                .initial_selected_item_ids
                .iter()
                .any(|id| id == &item.item_id)
                && !task
                    .runtime
                    .selected_item_ids
                    .iter()
                    .any(|id| id == &item.item_id)
            {
                continue;
            }
            let work = DownloadWork {
                task: snapshot.clone(),
                item: item.clone(),
            };
            let Some(file_name) = planned_item_file_name(&work) else {
                continue;
            };
            let Ok(destination) = insar_download::plan_download_target(
                &snapshot.output_root,
                &snapshot.layout,
                &file_name,
            ) else {
                continue;
            };
            let (directory, name) = (
                destination.parent().unwrap_or(Path::new("")).to_path_buf(),
                file_name,
            );
            match targets
                .iter_mut()
                .find(|target| target.directory == directory)
            {
                Some(target) => {
                    if !target.file_names.contains(&name) {
                        target.file_names.push(name);
                    }
                }
                None => targets.push(insar_download::conflict_cleanup::ConflictTarget {
                    directory,
                    file_names: vec![name],
                }),
            }
        }
        Ok((snapshot.output_root, targets, recorded, active))
    }

    /// Fixed scalar resolver for the latest authoritative formal DEM source.
    /// This deliberately stays separate from CoreRestore and history paging.
    pub fn resolve_dem_conversion_source(
        &self,
        request: &StoreDemConversionSourceRequest,
    ) -> Result<Option<StoreDemConversionSource>, ApplicationServiceError> {
        self.startup()?;
        let store = self.store_handle()?;
        let result = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .resolve_dem_conversion_source(request)
            .map_err(ApplicationServiceError::from);
        result
    }

    /// Persist the exact catalogue identity selected for DEM work. The Store
    /// remains the sole authority; the transport receives only the confirmed
    /// bounded scalars after the mutation commits.
    pub fn set_dem_source_identity(
        &self,
        identity: &StoreDemSourceIdentity,
    ) -> Result<StoreDemSourceIdentity, ApplicationServiceError> {
        let identity = canonical_selected_dem_source_identity(identity)?;
        self.with_business_runtime(|runtime| {
            runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?
                .save_dem_source_identity(&identity)
                .map_err(ApplicationServiceError::from)?;
            Ok(identity)
        })
    }

    /// Read the product-owned DEM scheduler defaults without opening the
    /// user Store.  The sidecar is bounded, local-only configuration; a
    /// missing file is the documented default rather than a failure.
    pub fn get_dem_task_settings(&self) -> Result<DemTaskSettings, ApplicationServiceError> {
        let path = self.dem_task_settings_path()?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DemTaskSettings::default())
            }
            Err(_) => {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "DEM task settings could not be read".to_owned(),
                })
            }
        };
        if bytes.len() > 64 * 1024 {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "DEM task settings file is too large".to_owned(),
            });
        }
        let settings: DemTaskSettings = serde_json::from_slice(&bytes).map_err(|_| {
            ApplicationServiceError::InvalidRequest {
                reason: "DEM task settings file is invalid".to_owned(),
            }
        })?;
        validate_dem_task_settings(&settings)?;
        Ok(settings)
    }

    /// Persist the DEM task admission limit and update the same live scheduler.
    /// Existing transfers drain naturally when the limit is lowered.
    pub fn save_dem_task_settings(
        &self,
        settings: &DemTaskSettings,
    ) -> Result<DemTaskSettings, ApplicationServiceError> {
        validate_dem_task_settings(settings)?;
        // Serialize publication with startup/shutdown. Startup holds this same
        // lifecycle lock while loading the sidecar, so it cannot install stale limits.
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let path = self.dem_task_settings_path()?;
        let parent = path
            .parent()
            .ok_or(ApplicationServiceError::StorePathRequired)?;
        std::fs::create_dir_all(parent).map_err(|_| ApplicationServiceError::InvalidRequest {
            reason: "DEM task settings directory could not be prepared".to_owned(),
        })?;
        let bytes = serde_json::to_vec_pretty(settings).map_err(|_| {
            ApplicationServiceError::InvalidRequest {
                reason: "DEM task settings could not be serialized".to_owned(),
            }
        })?;
        let temporary = parent.join(format!(".dem-task-settings-{}.tmp", uuid::Uuid::new_v4()));
        let written = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
            return Err(invalid_request("DEM task settings could not be committed"));
        }
        if let Some(runtime) = runtime.as_ref() {
            if let Some(scheduler) = runtime.scheduler.as_ref() {
                // Synchronize with the current admission scan. Once save returns,
                // no scan using the old cap may still start additional tasks.
                // Disk I/O above does not hold this Store lock.
                let _admission = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                scheduler.update_dem_task_limits(
                    insar_download::DemTaskLimits::new(
                        settings.max_concurrent_tasks,
                        settings.max_concurrent_conversions,
                    )
                    .expect("validated DEM settings"),
                );
            }
        }
        Ok(settings.clone())
    }

    /// Read the application-wide manual SAR download policy. Existing tasks do
    /// not consult this sidecar; callers freeze the returned values only when a
    /// new task is created.
    pub fn get_sar_download_policy_settings(
        &self,
    ) -> Result<SarDownloadPolicySettings, ApplicationServiceError> {
        let path = self.sar_download_policy_settings_path()?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(default_sar_download_policy_settings())
            }
            Err(_) => return Err(invalid_request("SAR download policy could not be read")),
        };
        if bytes.len() > 64 * 1024 {
            return Err(invalid_request("SAR download policy file is too large"));
        }
        let settings: SarDownloadPolicySettings = serde_json::from_slice(&bytes)
            .map_err(|_| invalid_request("SAR download policy file is invalid"))?;
        validate_sar_download_policy_settings(&settings)?;
        Ok(settings)
    }

    /// Atomically persist the only executable SAR download policy: a bounded
    /// manual concurrency and an output organization preference.
    pub fn save_sar_download_policy_settings(
        &self,
        settings: &SarDownloadPolicySettings,
    ) -> Result<SarDownloadPolicySettings, ApplicationServiceError> {
        validate_sar_download_policy_settings(settings)?;
        let path = self.sar_download_policy_settings_path()?;
        let parent = path
            .parent()
            .ok_or(ApplicationServiceError::StorePathRequired)?;
        std::fs::create_dir_all(parent)
            .map_err(|_| invalid_request("SAR download policy directory could not be prepared"))?;
        let bytes = serde_json::to_vec_pretty(settings)
            .map_err(|_| invalid_request("SAR download policy could not be serialized"))?;
        let temporary = parent.join(format!(
            ".sar-download-policy-settings-{}.tmp",
            std::process::id()
        ));
        std::fs::write(&temporary, bytes)
            .map_err(|_| invalid_request("SAR download policy could not be written"))?;
        if let Err(rename_error) = std::fs::rename(&temporary, &path) {
            let _ = std::fs::remove_file(&path);
            std::fs::rename(&temporary, &path).map_err(|_| {
                let _ = std::fs::remove_file(&temporary);
                invalid_request(format!(
                    "SAR download policy could not be committed: {rename_error}"
                ))
            })?;
        }
        Ok(settings.clone())
    }

    pub fn get_network_settings(
        &self,
    ) -> Result<ApplicationNetworkSettings, ApplicationServiceError> {
        let path = self.network_settings_path()?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ApplicationNetworkSettings::default())
            }
            Err(_) => return Err(invalid_request("network settings could not be read")),
        };
        if bytes.len() > 64 * 1024 {
            return Err(invalid_request("network settings file is too large"));
        }
        let settings: ApplicationNetworkSettings = serde_json::from_slice(&bytes)
            .map_err(|_| invalid_request("network settings file is invalid"))?;
        validate_network_settings(&settings)?;
        Ok(settings)
    }

    pub fn ordinary_proxy_policy(&self) -> insar_core::ProxyPolicy {
        self.network_policy(network_routes::ServiceRoute::Ordinary)
            .route()
    }

    pub fn save_network_settings(
        &self,
        settings: &ApplicationNetworkSettings,
    ) -> Result<ApplicationNetworkSettings, ApplicationServiceError> {
        let settings = settings.clone();
        validate_network_settings(&settings)?;
        let path = self.network_settings_path()?;
        let parent = path
            .parent()
            .ok_or(ApplicationServiceError::StorePathRequired)?;
        std::fs::create_dir_all(parent)
            .map_err(|_| invalid_request("network settings directory could not be prepared"))?;
        let bytes = serde_json::to_vec_pretty(&settings)
            .map_err(|_| invalid_request("network settings could not be serialized"))?;
        let temporary = parent.join(format!(".network-settings-{}.tmp", std::process::id()));
        std::fs::write(&temporary, bytes)
            .map_err(|_| invalid_request("network settings could not be written"))?;
        if let Err(rename_error) = std::fs::rename(&temporary, &path) {
            let _ = std::fs::remove_file(&path);
            std::fs::rename(&temporary, &path).map_err(|_| {
                let _ = std::fs::remove_file(&temporary);
                invalid_request(format!(
                    "network settings could not be committed: {rename_error}"
                ))
            })?;
        }
        self.request_network_policy_apply();
        Ok(settings)
    }

    /// Explicit credential-free reachability check of a fixed CDSE endpoint.
    /// This never updates saved settings or an active transfer's network route.
    pub fn test_network_connection(&self) -> Result<String, ApplicationServiceError> {
        self.ensure_running()?;
        let settings = self.get_network_settings()?;
        Ok(network_routes::test_connection_for_settings(&settings).to_owned())
    }

    /// Concurrent credential-free reachability probe of every provider entry
    /// endpoint under the saved proxy policy. Read-only, like the single CDSE
    /// probe: it never updates saved settings or an active transfer's route.
    pub fn probe_provider_connectivity(
        &self,
    ) -> Result<Vec<ProviderConnectivityResult>, ApplicationServiceError> {
        self.ensure_running()?;
        let settings = self.get_network_settings()?;
        Ok(network_routes::probe_providers_for_settings(&settings)
            .into_iter()
            .map(|outcome| ProviderConnectivityResult {
                id: outcome.id.to_owned(),
                site: network_routes::provider_probe_site(outcome.id)
                    .unwrap_or_default()
                    .to_owned(),
                ok: outcome.ok,
                latency_ms: outcome.latency_ms,
                http_status: outcome.http_status,
                error_code: outcome.error_code.to_owned(),
            })
            .collect())
    }

    /// Create a bounded online DEM task from an explicit bbox without
    /// requiring a Workspace/Project/Region.  The bbox is frozen into the
    /// task payload and the task is persisted through the normal Store and
    /// Scheduler path; no singleton context is synthesized or mutated.
    pub fn create_standalone_dem_task(
        &self,
        request: &StandaloneDemTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_formal_task_route()?;
        validate_standalone_dem_task_request(request)?;
        self.ensure_dem_provider_for_admission(&request.provider)?;
        let (resource_id, capability_id) = dem_identity(&request.dataset, &request.provider)
            .ok_or(ApplicationServiceError::ResourceRegistryUnavailable)?;
        self.ensure_formal_dem_capability(resource_id, capability_id)?;

        let region_id = "standalone-dem".to_owned();
        let bbox = request.bbox.clone();
        let geometry = standalone_bbox_geometry(&bbox)?;
        validate_dem_provider_request_windows(
            &request.dataset,
            &request.provider,
            &[BBox {
                west: bbox[0],
                south: bbox[1],
                east: bbox[2],
                north: bbox[3],
                crs: "EPSG:4326".to_owned(),
            }],
        )?;
        let request_tag = dem_request_file_tag(&request.created_at, &request.task_id)?;
        let source_vertical_datum = standalone_dem_source_datum(&request.dataset);
        let task_stem = dem_task_source_stem(&request.dataset, source_vertical_datum);
        let raw_name = format!("{task_stem}_{request_tag}.tif");
        let input_path = request.output_root.join(&raw_name);
        let mut payload = Map::new();
        payload.insert(
            "input_path".to_owned(),
            Value::String(input_path.to_string_lossy().to_string()),
        );
        payload.insert(
            "output_root".to_owned(),
            Value::String(request.output_root.to_string_lossy().to_string()),
        );
        payload.insert(
            "output_dir".to_owned(),
            Value::String(request.output_root.to_string_lossy().to_string()),
        );
        payload.insert(
            "path".to_owned(),
            Value::String(input_path.to_string_lossy().to_string()),
        );
        payload.insert("dataset".to_owned(), Value::String(request.dataset.clone()));
        payload.insert(
            "provider".to_owned(),
            Value::String(request.provider.clone()),
        );
        payload.insert(
            "resource_id".to_owned(),
            Value::String(resource_id.to_owned()),
        );
        payload.insert(
            "capability_id".to_owned(),
            Value::String(capability_id.to_owned()),
        );
        payload.insert(
            "bbox".to_owned(),
            serde_json::json!({
                "west": bbox[0],
                "south": bbox[1],
                "east": bbox[2],
                "north": bbox[3],
                "crs": "EPSG:4326",
            }),
        );
        payload.insert(
            "source_vertical_datum".to_owned(),
            Value::String(source_vertical_datum.to_owned()),
        );
        payload.insert(
            "target_vertical_datum".to_owned(),
            Value::String("WGS84_ELLIPSOID".to_owned()),
        );
        payload.insert(
            "output_mode".to_owned(),
            Value::String("sarscape".to_owned()),
        );
        payload.insert("convert".to_owned(), Value::Bool(request.convert));
        payload.insert(
            "source_policy".to_owned(),
            Value::String("standalone_bbox".to_owned()),
        );
        payload.insert(
            "proxy_policy".to_owned(),
            Value::String("product_owned".to_owned()),
        );
        match self.get_network_settings() {
            Ok(network) => {
                payload.insert(
                    "proxy_enabled".to_owned(),
                    Value::Bool(network.proxy_enabled),
                );
                payload.insert("proxy_url".to_owned(), Value::String(network.proxy_url));
            }
            Err(ApplicationServiceError::StorePathRequired) => {}
            Err(err) => return Err(err),
        }
        let operation = TaskOperationDescriptor {
            operation_id: format!("{}:dem_download", request.task_id),
            kind: "dem".to_owned(),
            phase: "download".to_owned(),
            region_id: region_id.clone(),
            approved_worker_operation: "dem_download".to_owned(),
            payload,
        };
        self.create_formal_task(
            &request.task_id,
            &format!("dem-standalone-{}", request.task_id),
            "compat.dem",
            &raw_name,
            RegionAoiState {
                region_id,
                revision: 0,
                source_kind: "standalone_bbox".to_owned(),
                geometry,
                bbox,
                crs: "EPSG:4326".to_owned(),
                display_name: format!("Standalone DEM {}", request.dataset),
                bound_at: request.created_at.clone(),
                updated_at: request.created_at.clone(),
            },
            request.created_at.clone(),
            request.output_root.clone(),
            request.concurrency,
            operation,
            None,
        )
    }

    /// Create one logical DEM task from the exact current Store-owned AOI.
    /// Antimeridian-crossing geometry is represented by two ordinary request
    /// windows under the same task identity; an ordinary AOI remains one item.
    pub fn create_standalone_dem_aoi_task(
        &self,
        request: &StandaloneDemAoiTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_formal_task_route()?;
        validate_standalone_dem_aoi_task_request(request)?;
        self.ensure_dem_provider_for_admission(&request.provider)?;
        let (resource_id, capability_id) = dem_identity(&request.dataset, &request.provider)
            .ok_or(ApplicationServiceError::ResourceRegistryUnavailable)?;
        self.ensure_formal_dem_capability(resource_id, capability_id)?;
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;

        let active = self
            .active_operation_aoi()?
            .ok_or_else(|| invalid_request("a current persisted AOI is required for a DEM task"))?;
        if active.geometry != request.aoi_geojson || active.display_name != request.aoi_name {
            return Err(invalid_request(
                "DEM AOI geometry or display identity is stale relative to Store authority",
            ));
        }
        let authoritative = validated_authoritative_polygonal_geojson(&active.geometry)
            .ok_or_else(|| invalid_request("persisted DEM AOI geometry is invalid"))?;
        // Authority has already passed full AOI topology and geographic bounds
        // validation. A span <= 180 degrees cannot wrap: every source edge fits
        // in the same canonical longitude domain. Derive only the acquisition
        // rectangle without imposing the render projection's quadratic topology
        // budget on an unchanged, admitted administrative boundary.
        let [west, south, east, north] = authoritative.bbox;
        let envelope = if east - west <= 180.0 {
            insar_core::WrappedAoiEnvelope {
                south,
                north,
                crosses_antimeridian: false,
                display_west: west,
                display_east: east,
                request_windows: vec![insar_core::BBox {
                    west,
                    south,
                    east,
                    north,
                    crs: "EPSG:4326".to_owned(),
                }],
            }
        } else {
            wrapped_aoi_envelope(&authoritative.geometry)
                .ok_or_else(|| invalid_request("persisted DEM AOI cannot derive request windows"))?
        };
        validate_capability_windows(&envelope.request_windows)?;
        validate_dem_provider_request_windows(
            &request.dataset,
            &request.provider,
            &envelope.request_windows,
        )?;

        let source_vertical_datum = standalone_dem_source_datum(&request.dataset);
        let task_stem = dem_task_source_stem(&request.dataset, source_vertical_datum);
        let request_tag = dem_request_file_tag(&request.created_at, &request.task_id)?;
        let raw_name = format!("{task_stem}_{request_tag}.tif");
        let input_path = request.output_root.join(&raw_name);
        let staging_root = request
            .output_root
            .join(".eostage")
            .join("dem-tasks")
            .join(&request.task_id);
        let windows_value = envelope
            .request_windows
            .iter()
            .map(capability_window_value)
            .collect::<Vec<_>>();
        let mut operation_payload = Map::new();
        operation_payload.insert(
            "input_path".to_owned(),
            Value::String(input_path.to_string_lossy().to_string()),
        );
        operation_payload.insert(
            "output_root".to_owned(),
            Value::String(request.output_root.to_string_lossy().to_string()),
        );
        operation_payload.insert(
            "output_dir".to_owned(),
            Value::String(request.output_root.to_string_lossy().to_string()),
        );
        operation_payload.insert(
            "path".to_owned(),
            Value::String(input_path.to_string_lossy().to_string()),
        );
        operation_payload.insert("dataset".to_owned(), Value::String(request.dataset.clone()));
        operation_payload.insert(
            "provider".to_owned(),
            Value::String(request.provider.clone()),
        );
        operation_payload.insert(
            "resource_id".to_owned(),
            Value::String(resource_id.to_owned()),
        );
        operation_payload.insert(
            "capability_id".to_owned(),
            Value::String(capability_id.to_owned()),
        );
        operation_payload.insert(
            "processing_windows".to_owned(),
            Value::Array(windows_value.clone()),
        );
        let window_count = envelope.request_windows.len();
        let window_inputs = (0..window_count)
            .map(|index| match (window_count, index) {
                (1, _) => input_path.clone(),
                (2, 0) => staging_root.join(format!("{task_stem}_{request_tag}_west.tif")),
                (2, _) => staging_root.join(format!("{task_stem}_{request_tag}_east.tif")),
                _ => unreachable!("validated capability windows are bounded to two"),
            })
            .collect::<Vec<_>>();
        operation_payload.insert(
            "window_inputs".to_owned(),
            Value::Array(
                window_inputs
                    .iter()
                    .map(|path| Value::String(path.to_string_lossy().to_string()))
                    .collect(),
            ),
        );
        operation_payload.insert(
            "merged_input_path".to_owned(),
            Value::String(input_path.to_string_lossy().to_string()),
        );
        operation_payload.insert(
            "source_vertical_datum".to_owned(),
            Value::String(source_vertical_datum.to_owned()),
        );
        operation_payload.insert(
            "target_vertical_datum".to_owned(),
            Value::String("WGS84_ELLIPSOID".to_owned()),
        );
        operation_payload.insert(
            "output_mode".to_owned(),
            Value::String("sarscape".to_owned()),
        );
        operation_payload.insert("convert".to_owned(), Value::Bool(request.convert));
        operation_payload.insert(
            "source_policy".to_owned(),
            Value::String("store_owned_aoi".to_owned()),
        );
        operation_payload.insert(
            "proxy_policy".to_owned(),
            Value::String("product_owned".to_owned()),
        );
        match self.get_network_settings() {
            Ok(network) => {
                operation_payload.insert(
                    "proxy_enabled".to_owned(),
                    Value::Bool(network.proxy_enabled),
                );
                operation_payload.insert("proxy_url".to_owned(), Value::String(network.proxy_url));
            }
            Err(ApplicationServiceError::StorePathRequired) => {}
            Err(err) => return Err(err),
        }

        let region_id = "standalone-dem-aoi".to_owned();
        operation_payload.insert("region_id".to_owned(), Value::String(region_id.clone()));
        let operation = TaskOperationDescriptor {
            operation_id: format!("{}:dem_download", request.task_id),
            kind: "dem".to_owned(),
            phase: "download".to_owned(),
            region_id: region_id.clone(),
            approved_worker_operation: "dem_download".to_owned(),
            payload: operation_payload,
        };
        let items = envelope
            .request_windows
            .iter()
            .enumerate()
            .map(|(index, window)| {
                let display_name = match window_count {
                    1 => raw_name.clone(),
                    2 if index == 0 => format!("{task_stem}_{request_tag}_west.tif"),
                    2 => format!("{task_stem}_{request_tag}_east.tif"),
                    _ => unreachable!("validated capability windows are bounded to two"),
                };
                let mut item_payload = operation.payload.clone();
                item_payload.insert(
                    "operation_id".to_owned(),
                    Value::String(operation.operation_id.clone()),
                );
                item_payload.insert(
                    "approved_worker_operation".to_owned(),
                    Value::String(operation.approved_worker_operation.clone()),
                );
                item_payload.insert("bbox".to_owned(), capability_window_value(window));
                let window_input = window_inputs[index].to_string_lossy().to_string();
                item_payload.insert("input_path".to_owned(), Value::String(window_input.clone()));
                item_payload.insert("path".to_owned(), Value::String(window_input));
                item_payload.insert(
                    "window_index".to_owned(),
                    Value::Number((index as u64).into()),
                );
                item_payload.insert(
                    "window_count".to_owned(),
                    Value::Number((window_count as u64).into()),
                );
                SnapshotItem {
                    item_id: format!("{}:window:{index}", request.task_id),
                    provider_id: "compat.dem".to_owned(),
                    display_name,
                    acquisition_datetime: None,
                    footprint_geojson: Some(active.geometry.clone()),
                    metadata: {
                        let mut metadata = Map::new();
                        metadata.insert("region_id".to_owned(), Value::String(region_id.clone()));
                        metadata
                            .insert("task_operation".to_owned(), Value::String("dem".to_owned()));
                        metadata.insert(
                            "window_index".to_owned(),
                            Value::Number((index as u64).into()),
                        );
                        metadata.insert(
                            "window_count".to_owned(),
                            Value::Number((window_count as u64).into()),
                        );
                        metadata
                    },
                    payload: item_payload,
                    asset_access: None,
                    execution_policy: None,
                }
            })
            .collect::<Vec<_>>();

        self.create_formal_task_with_items(
            &request.task_id,
            &format!("dem-aoi-{}", request.task_id),
            "compat.dem",
            RegionAoiState {
                region_id,
                revision: 0,
                source_kind: active.source_kind,
                geometry: active.geometry,
                bbox: active.bbox,
                crs: active.crs,
                display_name: active.display_name,
                bound_at: request.created_at.clone(),
                updated_at: request.created_at.clone(),
            },
            request.created_at.clone(),
            request.output_root.clone(),
            request.concurrency,
            operation,
            None,
            items,
        )
    }

    /// Execute a bounded local DEM operation through the product-owned native
    /// GIS seam.  No workspace context, scheduler, provider, or credential is
    /// consulted here; formal project-bound task creation remains a separate
    /// route.
    pub fn standalone_dem(
        &self,
        request: &StandaloneDemRequest,
    ) -> Result<Value, ApplicationServiceError> {
        self.ensure_standalone_dem_route()?;
        validate_standalone_dem_request(request)?;
        validate_dem_source_identity(
            &dem_resource_id(&request.dataset, &request.provider),
            &dem_capability_id(&request.dataset, &request.provider),
            &request.dataset,
            &request.provider,
        )?;
        let resource_id = dem_resource_id(&request.dataset, &request.provider);
        let capability_id = dem_capability_id(&request.dataset, &request.provider);
        self.ensure_formal_dem_capability(&resource_id, &capability_id)?;

        let mut payload = Map::new();
        payload.insert(
            "input_path".to_owned(),
            Value::String(request.input_path.to_string_lossy().to_string()),
        );
        payload.insert(
            "path".to_owned(),
            Value::String(request.input_path.to_string_lossy().to_string()),
        );
        payload.insert(
            "output_root".to_owned(),
            Value::String(request.output_root.to_string_lossy().to_string()),
        );
        payload.insert(
            "output_dir".to_owned(),
            Value::String(request.output_root.to_string_lossy().to_string()),
        );
        payload.insert("dataset".to_owned(), Value::String(request.dataset.clone()));
        payload.insert(
            "provider".to_owned(),
            Value::String(request.provider.clone()),
        );
        payload.insert("resource_id".to_owned(), Value::String(resource_id));
        payload.insert("capability_id".to_owned(), Value::String(capability_id));
        if let Some(bbox) = request.bbox.as_ref() {
            payload.insert(
                "bbox".to_owned(),
                serde_json::json!({
                    "west": bbox[0],
                    "south": bbox[1],
                    "east": bbox[2],
                    "north": bbox[3],
                    "crs": "EPSG:4326",
                }),
            );
        }
        if let Some(source) = request.source_vertical_datum.as_ref() {
            let source = resolve_standalone_dem_source_datum(&request.dataset, source);
            payload.insert("source_vertical_datum".to_owned(), Value::String(source));
        }
        if let Some(target) = request.target_vertical_datum.as_ref() {
            payload.insert(
                "target_vertical_datum".to_owned(),
                Value::String(target.clone()),
            );
        }
        if let Some(mode) = request.output_mode.as_ref() {
            payload.insert("output_mode".to_owned(), Value::String(mode.clone()));
        }
        let operation = dem_worker_operation(&request.operation);
        let result = self
            .dem_operation_port
            .as_ref()
            .ok_or(ApplicationServiceError::ProcessingEngineUnavailable {
                operation: "dem_processing",
            })?
            .call(operation, Value::Object(payload))
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        Ok(serde_json::json!({
            "ok": true,
            "request_id": request.request_id,
            "operation": operation,
            "dataset": request.dataset,
            "provider": request.provider,
            "native": true,
            "offline": true,
            "result": result,
        }))
    }

    /// Bounded manual Sentinel-1 import.  This is a preview/import projection
    /// only: it does not mutate Store, perform a provider search, or create a
    /// task.  The normal UI can subsequently freeze the selected rows through
    /// its existing SearchBatch/ApplicationService route.
    pub fn import_scenes(
        &self,
        request: &SceneImportRequest,
    ) -> Result<Value, ApplicationServiceError> {
        const MAX_IMPORT_BYTES: usize = 2 * 1024 * 1024;
        const MAX_SCENES: usize = 200;
        let source = request.source.trim().to_ascii_lowercase();
        let lines = match source.as_str() {
            "text" => request
                .text
                .as_deref()
                .ok_or_else(|| invalid_request("scene text is required"))?
                .to_owned(),
            "file" | "path" => {
                let path = request
                    .path
                    .as_ref()
                    .ok_or_else(|| invalid_request("scene file path is required"))?;
                let metadata = std::fs::metadata(path)
                    .map_err(|_| invalid_request("scene file is not readable"))?;
                if !metadata.is_file() || metadata.len() > MAX_IMPORT_BYTES as u64 {
                    return Err(invalid_request(
                        "scene file is missing or exceeds the 2 MiB limit",
                    ));
                }
                std::fs::read_to_string(path)
                    .map_err(|_| invalid_request("scene file is not UTF-8 text"))?
            }
            "directory" => {
                let root = request
                    .path
                    .as_ref()
                    .ok_or_else(|| invalid_request("scene directory is required"))?;
                let names = bounded_directory_file_names(root, 5_000)?;
                names.join("\n")
            }
            _ => return Err(invalid_request("scene import source is invalid")),
        };
        if lines.len() > MAX_IMPORT_BYTES {
            return Err(invalid_request("scene import exceeds the 2 MiB limit"));
        }
        let mut scenes = Vec::new();
        let mut duplicates = Vec::new();
        let mut errors = Vec::new();
        let mut seen = BTreeSet::new();
        for line in lines.lines().map(str::trim).filter(|line| !line.is_empty()) {
            if scenes.len() >= MAX_SCENES {
                errors.push(serde_json::json!({
                    "line": line,
                    "error": "manual scene import is limited to 200 scenes"
                }));
                break;
            }
            match insar_core::parse_sentinel1_identifier(line) {
                Ok(parsed) => {
                    if !seen.insert(parsed.scene_id.clone()) {
                        duplicates.push(parsed.scene_id);
                        continue;
                    }
                    scenes.push(scene_row_from_parsed(&parsed, line));
                }
                Err(error) => errors.push(serde_json::json!({
                    "line": line,
                    "error": error.to_string()
                })),
            }
        }
        if scenes.is_empty() {
            return Err(invalid_request("未能解析出任何有效的 Sentinel-1 场景"));
        }
        Ok(serde_json::json!({
            "ok": true,
            "scenes": scenes,
            "duplicates": duplicates,
            "errors": errors,
            "queried": seen.len(),
            "search": {
                "requested_limit": MAX_SCENES,
                "query_limit": MAX_SCENES,
                "total_count": seen.len(),
                "returned_count": seen.len(),
                "source": "manual_native"
            }
        }))
    }

    /// Match a bounded local orbit directory against the supplied bounded
    /// scene page.  Filename parsing and selection stay in insar-core; this
    /// method only owns the local directory boundary and JSON projection.
    pub fn match_orbits_directory(
        &self,
        request: &OrbitDirectoryMatchRequest,
    ) -> Result<Value, ApplicationServiceError> {
        if request.orbit_dir.as_os_str().is_empty() || !request.orbit_dir.is_dir() {
            return Err(invalid_request("orbit directory is not readable"));
        }
        if request.scenes.is_empty() || request.scenes.len() > 100 {
            return Err(invalid_request("orbit matching requires 1..=100 scenes"));
        }
        for scene in &request.scenes {
            scene
                .validate()
                .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        }
        let file_names = bounded_directory_file_names(&request.orbit_dir, 5_000)?;
        let parsed = parse_orbit_filenames(&OrbitParseRequest { file_names });
        let report = match_orbits(&OrbitMatchRequest {
            scenes: request.scenes.clone(),
            orbit_files: parsed.orbit_files.clone(),
        });
        Ok(serde_json::json!({
            "ok": true,
            "orbit_dir": request.orbit_dir,
            "orbit_files": parsed.orbit_files.len(),
            "parse_errors": parsed.errors,
            "report": report
        }))
    }

    /// Return the bounded, credential-free GACOS ledger sidecar.  Live submit
    /// is intentionally not part of this route; an absent ledger is a valid
    /// empty state for a fresh native Store.
    pub fn gacos_ledger(&self) -> Result<Value, ApplicationServiceError> {
        let path = self.gacos_ledger_path()?;
        let records = match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 2 * 1024 * 1024 {
                    return Err(invalid_request("GACOS ledger exceeds the 2 MiB limit"));
                }
                let value: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| invalid_request("GACOS ledger is invalid JSON"))?;
                match value {
                    Value::Array(records) => records,
                    Value::Object(mut object) => object
                        .remove("records")
                        .and_then(|value| value.as_array().cloned())
                        .ok_or_else(|| invalid_request("GACOS ledger records are missing"))?,
                    _ => return Err(invalid_request("GACOS ledger must contain records")),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Err(invalid_request("GACOS ledger could not be read")),
        };
        Ok(serde_json::json!({
            "ok": true,
            "records": records,
            "live_submit_enabled": false,
            "source": "product-owned-gacos-ledger"
        }))
    }

    /// The legacy DTO has no frozen request identity and cannot authorize an import.
    /// Normal desktop imports use `gacos_create_import_task` and the existing Scheduler.
    pub fn gacos_import_result(
        &self,
        _request: &GacosResultImportRequest,
    ) -> Result<Value, ApplicationServiceError> {
        Err(GacosRequestError::ImportContextRequired.into())
    }

    /// Apply one bounded, revision-guarded Workbench selection command in the
    /// Store. The command carries only an explicit delta or a page operation;
    /// the authoritative cross-page checked-ID set remains inside Store.
    pub fn update_workbench_selection(
        &self,
        command: &WorkbenchSelectionCommand,
    ) -> Result<WorkbenchSelectionSummary, ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store
                .apply_workbench_selection(command)
                .map_err(ApplicationServiceError::from)
        })
    }

    pub fn plan_dem_task(
        &self,
        request: &DemTaskPlanRequest,
    ) -> Result<DemTaskPlan, ApplicationServiceError> {
        self.ensure_formal_task_route()?;
        validate_formal_dem_task_identity(
            &request.request_id,
            &request.task_id,
            &request.region_id,
            &request.output_root,
            &request.created_at,
        )?;
        validate_dem_source_identity(
            &request.resource_id,
            &request.capability_id,
            &request.dataset,
            &request.provider,
        )?;
        self.ensure_formal_dem_capability(&request.resource_id, &request.capability_id)?;
        if request.input_path.as_os_str().is_empty() {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "DEM input_path is required".to_owned(),
            });
        }
        self.region_aoi_for_task(&request.region_id)?;
        Ok(DemTaskPlan {
            request_id: request.request_id.clone(),
            task_id: request.task_id.clone(),
            region_id: request.region_id.clone(),
            resource_id: request.resource_id.clone(),
            capability_id: request.capability_id.clone(),
            dataset: request.dataset.clone(),
            provider: request.provider.clone(),
            operation: request.operation.clone(),
            input_path: request.input_path.clone(),
            output_root: request.output_root.clone(),
            source_vertical_datum: request.source_vertical_datum.clone(),
            target_vertical_datum: request.target_vertical_datum.clone(),
            output_mode: request.output_mode.clone(),
            created_at: request.created_at.clone(),
        })
    }

    pub fn create_dem_task(
        &self,
        request: &DemTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_formal_task_route()?;
        if !(1..=10).contains(&request.concurrency) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "DEM concurrency must be between 1 and 10".to_owned(),
            });
        }
        self.ensure_dem_provider_for_admission(&request.plan.provider)?;
        let plan = self.plan_dem_task(&DemTaskPlanRequest {
            request_id: request.plan.request_id.clone(),
            task_id: request.plan.task_id.clone(),
            region_id: request.plan.region_id.clone(),
            resource_id: request.plan.resource_id.clone(),
            capability_id: request.plan.capability_id.clone(),
            dataset: request.plan.dataset.clone(),
            provider: request.plan.provider.clone(),
            operation: request.plan.operation.clone(),
            input_path: request.plan.input_path.clone(),
            output_root: request.plan.output_root.clone(),
            source_vertical_datum: request.plan.source_vertical_datum.clone(),
            target_vertical_datum: request.plan.target_vertical_datum.clone(),
            output_mode: request.plan.output_mode.clone(),
            created_at: request.plan.created_at.clone(),
        })?;
        let aoi = self.region_aoi_for_task(&plan.region_id)?;
        let operation_name = dem_worker_operation(&plan.operation);
        let phase = dem_operation_phase(&plan.operation);
        let mut payload = Map::new();
        payload.insert(
            "input_path".to_owned(),
            Value::String(plan.input_path.to_string_lossy().to_string()),
        );
        payload.insert(
            "output_root".to_owned(),
            Value::String(plan.output_root.to_string_lossy().to_string()),
        );
        payload.insert(
            "output_dir".to_owned(),
            Value::String(plan.output_root.to_string_lossy().to_string()),
        );
        payload.insert(
            "path".to_owned(),
            Value::String(plan.input_path.to_string_lossy().to_string()),
        );
        payload.insert(
            "region_id".to_owned(),
            Value::String(plan.region_id.clone()),
        );
        payload.insert(
            "resource_id".to_owned(),
            Value::String(plan.resource_id.clone()),
        );
        payload.insert(
            "capability_id".to_owned(),
            Value::String(plan.capability_id.clone()),
        );
        payload.insert("dataset".to_owned(), Value::String(plan.dataset.clone()));
        payload.insert("provider".to_owned(), Value::String(plan.provider.clone()));
        if let Some(source_vertical_datum) = plan.source_vertical_datum.as_ref() {
            payload.insert(
                "source_vertical_datum".to_owned(),
                Value::String(source_vertical_datum.clone()),
            );
        }
        if let Some(target_vertical_datum) = plan.target_vertical_datum.as_ref() {
            payload.insert(
                "target_vertical_datum".to_owned(),
                Value::String(target_vertical_datum.clone()),
            );
        }
        if let Some(output_mode) = plan.output_mode.as_ref() {
            payload.insert("output_mode".to_owned(), Value::String(output_mode.clone()));
        }
        if plan.operation == DemTaskOperation::Convert {
            payload.insert("execute".to_owned(), Value::Bool(true));
        }
        if plan.operation != DemTaskOperation::Inspect {
            if aoi.bbox.len() < 4 {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "formal DEM task requires a four-coordinate AOI bbox".to_owned(),
                });
            }
            payload.insert(
                "bbox".to_owned(),
                serde_json::json!({
                    "west": aoi.bbox[0],
                    "south": aoi.bbox[1],
                    "east": aoi.bbox[2],
                    "north": aoi.bbox[3],
                    "crs": aoi.crs.clone(),
                }),
            );
        }
        self.create_formal_task(
            &plan.task_id,
            &format!("dem-{}", plan.task_id),
            "compat.dem",
            &format!("DEM {}", plan.task_id),
            aoi,
            plan.created_at,
            plan.output_root,
            request.concurrency,
            TaskOperationDescriptor {
                operation_id: format!("{}:{operation_name}", plan.task_id),
                kind: "dem".to_owned(),
                phase: phase.to_owned(),
                region_id: plan.region_id,
                approved_worker_operation: operation_name.to_owned(),
                payload,
            },
            None,
        )
    }

    pub fn handoff_dem_conversion(
        &self,
        request: &DemConversionRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_formal_task_route()?;
        validate_formal_task_identity(
            &request.request_id,
            &request.conversion_task_id,
            &request.region_id,
            &request.output_root,
            &request.created_at,
        )?;
        validate_dem_source_identity(
            &request.resource_id,
            &request.capability_id,
            &request.dataset,
            &request.provider,
        )?;
        self.ensure_formal_dem_capability(&request.resource_id, &request.capability_id)?;
        if request.source_task_id.trim().is_empty()
            || request.input_path.as_os_str().is_empty()
            || !(1..=8).contains(&request.concurrency)
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "DEM conversion handoff input is invalid".to_owned(),
            });
        }
        self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let source = store
                .load_task(&request.source_task_id)?
                .ok_or(ApplicationServiceError::FormalTaskSourceIncomplete)?;
            validate_dem_conversion_source_task(&source, request)
        })?;
        let aoi = self.region_aoi_for_task(&request.region_id)?;
        let mut payload = Map::new();
        payload.insert(
            "input_path".to_owned(),
            Value::String(request.input_path.to_string_lossy().to_string()),
        );
        payload.insert(
            "output_root".to_owned(),
            Value::String(request.output_root.to_string_lossy().to_string()),
        );
        payload.insert(
            "output_dir".to_owned(),
            Value::String(request.output_root.to_string_lossy().to_string()),
        );
        payload.insert(
            "path".to_owned(),
            Value::String(request.input_path.to_string_lossy().to_string()),
        );
        payload.insert(
            "source_task_id".to_owned(),
            Value::String(request.source_task_id.clone()),
        );
        payload.insert(
            "region_id".to_owned(),
            Value::String(request.region_id.clone()),
        );
        payload.insert(
            "resource_id".to_owned(),
            Value::String(request.resource_id.clone()),
        );
        payload.insert(
            "capability_id".to_owned(),
            Value::String(request.capability_id.clone()),
        );
        payload.insert("dataset".to_owned(), Value::String(request.dataset.clone()));
        payload.insert(
            "provider".to_owned(),
            Value::String(request.provider.clone()),
        );
        if let Some(source_vertical_datum) = request.source_vertical_datum.as_ref() {
            payload.insert(
                "source_vertical_datum".to_owned(),
                Value::String(source_vertical_datum.clone()),
            );
        }
        if let Some(target_vertical_datum) = request.target_vertical_datum.as_ref() {
            payload.insert(
                "target_vertical_datum".to_owned(),
                Value::String(target_vertical_datum.clone()),
            );
        }
        if let Some(output_mode) = request.output_mode.as_ref() {
            payload.insert("output_mode".to_owned(), Value::String(output_mode.clone()));
        }
        if aoi.bbox.len() < 4 {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "formal DEM conversion requires a four-coordinate AOI bbox".to_owned(),
            });
        }
        payload.insert(
            "bbox".to_owned(),
            serde_json::json!({
                "west": aoi.bbox[0],
                "south": aoi.bbox[1],
                "east": aoi.bbox[2],
                "north": aoi.bbox[3],
                "crs": aoi.crs.clone(),
            }),
        );
        payload.insert("execute".to_owned(), Value::Bool(true));
        self.create_formal_task(
            &request.conversion_task_id,
            &format!("dem-conversion-{}", request.conversion_task_id),
            "compat.dem",
            &format!("DEM conversion {}", request.source_task_id),
            aoi,
            request.created_at.clone(),
            request.output_root.clone(),
            request.concurrency,
            TaskOperationDescriptor {
                operation_id: format!("{}:dem_convert", request.source_task_id),
                kind: "dem".to_owned(),
                phase: "conversion".to_owned(),
                region_id: request.region_id.clone(),
                approved_worker_operation: "dem_convert".to_owned(),
                payload,
            },
            None,
        )
    }

    pub fn create_orbit_task(
        &self,
        request: &OrbitTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if !(1..=10).contains(&request.concurrency) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Orbit concurrency must be between 1 and 10".to_owned(),
            });
        }
        validate_formal_task_identity(
            &request.request_id,
            &request.task_id,
            &request.region_id,
            &request.output_root,
            &request.created_at,
        )?;
        request
            .association
            .validate()
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        self.ensure_formal_orbit_capability()?;
        if request.association.source_item_id.trim().is_empty() {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Orbit source item ID is required".to_owned(),
            });
        }
        let aoi = self.region_aoi_for_task(&request.region_id)?;
        let association = request.association.clone();
        let mut payload = Map::new();
        payload.insert(
            "expected_filename".to_owned(),
            Value::String(association.matched_orbit_metadata.file_name.clone()),
        );
        payload.insert(
            "source_item_id".to_owned(),
            Value::String(association.source_item_id.clone()),
        );
        payload.insert(
            "orbit_transfer_item_id".to_owned(),
            Value::String(association.orbit_transfer_item_id.clone()),
        );
        payload.insert(
            "resource_id".to_owned(),
            Value::String("orbit.sentinel-1.poeorb.v1".to_owned()),
        );
        payload.insert(
            "capability_id".to_owned(),
            Value::String("capability.sentinel-1.poeorb.download.v1".to_owned()),
        );
        let display_name = association.matched_orbit_metadata.file_name.clone();
        self.create_formal_task(
            &request.task_id,
            &format!("orbit-{}", request.task_id),
            "sentinel1.orbit",
            &display_name,
            aoi,
            request.created_at.clone(),
            request.output_root.clone(),
            request.concurrency,
            TaskOperationDescriptor {
                operation_id: format!("{}:orbit_poeorb", request.task_id),
                kind: "orbit".to_owned(),
                phase: "association".to_owned(),
                region_id: request.region_id.clone(),
                approved_worker_operation: "orbit_download".to_owned(),
                payload,
            },
            Some(association),
        )
    }

    /// Create one standalone Sentinel-1 POEORB download task without requiring
    /// a legacy Workspace/Project/Region/AOI context. The worker resolves EOF
    /// metadata directly from each scene ID.
    pub fn create_standalone_orbit_task(
        &self,
        request: &StandaloneOrbitTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if !valid_formal_task_shape(
            &request.request_id,
            &request.task_id,
            "standalone-orbit",
            &request.output_root,
        ) || parse_normalized_utc(&request.created_at).is_none()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "standalone orbit task identity, output_root, or timestamp is invalid"
                    .to_owned(),
            });
        }
        if request.scenes.is_empty() || request.scenes.len() > 500 {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "standalone orbit task requires between 1 and 500 Sentinel-1 scenes"
                    .to_owned(),
            });
        }
        self.ensure_formal_orbit_capability()?;

        let active_aoi = self.active_operation_aoi().ok().flatten();
        let aoi_name = active_aoi
            .as_ref()
            .map(|aoi| aoi.display_name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Sentinel-1 精密轨道".to_owned());
        let aoi_geojson = active_aoi.map(|aoi| aoi.geometry);

        let mut seen_scenes = std::collections::BTreeSet::new();
        let mut candidates = Vec::with_capacity(request.scenes.len());
        for scene in &request.scenes {
            let mut normalized = scene.scene_id.trim();
            if let Some((_, tail)) = normalized.rsplit_once("::ITEM::") {
                normalized = tail.trim();
            }
            for suffix in [
                "-SLC", "-GRD_HD", "-GRD_HS", "-GRD_MD", "-GRD_MS", "-GRD_FD", "-RAW", "-OCN",
            ] {
                if let Some(stem) = normalized.strip_suffix(suffix) {
                    normalized = stem;
                    break;
                }
            }
            let Ok(parsed) = insar_core::parse_sentinel1_identifier(normalized) else {
                continue;
            };
            if !seen_scenes.insert(parsed.scene_id.clone()) {
                continue;
            }
            let acq_dt = scene
                .acquisition_datetime
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            candidates.push(SnapshotItem {
                item_id: parsed.scene_id.clone(),
                provider_id: "sentinel1.orbit".to_owned(),
                display_name: parsed.scene_id.clone(),
                acquisition_datetime: acq_dt,
                footprint_geojson: None,
                metadata: Map::from_iter([
                    ("source_item_id".to_owned(), Value::String(parsed.scene_id)),
                    (
                        "region_id".to_owned(),
                        Value::String("standalone-orbit".to_owned()),
                    ),
                ]),
                payload: Map::new(),
                asset_access: None,
                execution_policy: None,
            });
        }
        if candidates.is_empty() {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "standalone orbit task requires at least one valid Sentinel-1 scene"
                    .to_owned(),
            });
        }
        let ids = candidates
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>();
        let batch_id = format!("orbit-standalone-{}", request.task_id);
        let operation = TaskOperationDescriptor {
            operation_id: format!("{}:poeorb", request.task_id),
            kind: "orbit".to_owned(),
            phase: "download".to_owned(),
            region_id: "standalone-orbit".to_owned(),
            approved_worker_operation: "orbit_download".to_owned(),
            payload: Map::from_iter([
                (
                    "resource_id".to_owned(),
                    Value::String("orbit.sentinel-1.poeorb.v1".to_owned()),
                ),
                (
                    "capability_id".to_owned(),
                    Value::String("capability.sentinel-1.poeorb.download.v1".to_owned()),
                ),
            ]),
        };
        let layout = if request.use_orbit_subdir {
            DownloadLayout::SentinelOrbitAuxPoeorb
        } else {
            DownloadLayout::Direct
        };
        let page_size = u64::try_from(ids.len())
            .map_err(|_| invalid_request("standalone orbit item count is invalid"))?;
        self.mutate_business_store(|store| {
            store
                .save_search_batch_and_create_task_with_projection(
                    &StoreSearchBatchRequest {
                        batch: FrozenSearchBatch {
                            schema_version: 1,
                            batch_id: batch_id.clone(),
                            provider_id: "sentinel1.orbit".to_owned(),
                            created_at: request.created_at.clone(),
                            aoi_name,
                            aoi_geojson,
                            query: Map::from_iter([(
                                "source_policy".to_owned(),
                                Value::String("standalone_orbit".to_owned()),
                            )]),
                            search_stats: None,
                            candidates,
                        },
                        workbench: SearchWorkbenchState {
                            batch_id: batch_id.clone(),
                            checked_item_ids: ids.clone(),
                            page_index: 0,
                            page_size,
                            revision: 0,
                        },
                        activate_workbench: false,
                    },
                    &StoreCreateTaskRequest {
                        task_id: request.task_id.clone(),
                        batch_id,
                        selected_item_ids: ids.clone(),
                        checked_item_ids: ids,
                        page_index: 0,
                        page_size,
                        output_root: request.output_root.clone(),
                        layout,
                        // Adjacent SLC scenes can resolve to the same EOF path.
                        // Serialize this batch so the next scene reuses the completed file.
                        concurrency: 1,
                        created_at: request.created_at.clone(),
                    },
                    Some(&operation),
                    None,
                )
                .map_err(ApplicationServiceError::from)?;
            Ok(())
        })
    }

    pub fn workspace_prepare_location(
        &self,
        request: &WorkspacePrepareLocationRequest,
    ) -> Result<WorkspacePreparedLocation, ApplicationServiceError> {
        if request.request_id.trim().is_empty()
            || request.path.as_os_str().is_empty()
            || !request.path.is_absolute()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "workspace_prepare_location requires an absolute path".to_owned(),
            });
        }
        std::fs::create_dir_all(&request.path).map_err(|_| {
            ApplicationServiceError::InvalidRequest {
                reason: "workspace location could not be prepared".to_owned(),
            }
        })?;
        Ok(WorkspacePreparedLocation {
            path: request.path.clone(),
        })
    }

    /// Explicit compatibility-only full snapshot for the historical Orbit
    /// task acceptance surface.
    pub fn create_orbit_task_legacy(
        &self,
        request: &OrbitTaskRequest,
    ) -> Result<ReadonlyServiceSnapshot, ApplicationServiceError> {
        self.create_orbit_task(request)?;
        self.snapshot(true)
    }

    /// Open the one authoritative workspace context owned by the Store. The
    /// context carries stable project/region IDs; UI adapters never synthesize
    /// a replacement singleton context.
    pub fn workspace_open(
        &self,
        request: &WorkspaceOpenRequest,
    ) -> Result<WorkspaceContextState, ApplicationServiceError> {
        if request.request_id.trim().is_empty()
            || request.workspace_id.trim().is_empty()
            || request.workspace_name.trim().is_empty()
            || request.workspace_root.as_os_str().is_empty()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "workspace_open requires stable ID, root, and name".to_owned(),
            });
        }
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
            let existing = store.load_workspace_context()?;
            if let Some(current) = existing
                .as_ref()
                .filter(|value| value.workspace_id == request.workspace_id)
            {
                if let Some(expected) = request.expected_revision {
                    ensure_workspace_revision(expected, current.revision)?;
                }
            }
            let mut context = existing
                .filter(|value| value.workspace_id == request.workspace_id)
                .unwrap_or_else(|| WorkspaceContextState {
                    schema_version: 1,
                    workspace_id: request.workspace_id.clone(),
                    workspace_root: request.workspace_root.clone(),
                    workspace_name: request.workspace_name.clone(),
                    project_id: None,
                    project_name: None,
                    region_id: None,
                    region_name: None,
                    revision: 0,
                    updated_at: now.clone(),
                    projects: Vec::new(),
                    regions: Vec::new(),
                });
            context.workspace_root = request.workspace_root.clone();
            context.workspace_name = request.workspace_name.clone();
            context.revision = context.revision.checked_add(1).ok_or_else(|| {
                ApplicationServiceError::InvalidRequest {
                    reason: "workspace revision overflow".to_owned(),
                }
            })?;
            context.updated_at = now;
            sync_workspace_regions(&mut context);
            store.save_workspace_context(&context)?;
            Ok(context)
        })
    }

    pub fn workspace_create(
        &self,
        request: &WorkspaceCreateRequest,
    ) -> Result<WorkspaceContextState, ApplicationServiceError> {
        self.workspace_open(request)
    }

    pub fn project_create(
        &self,
        request: &WorkspaceNamedNodeRequest,
    ) -> Result<WorkspaceContextState, ApplicationServiceError> {
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        self.mutate_workspace_context(
            request.request_id.as_str(),
            request.expected_revision,
            |context| {
                if request.parent_id != context.workspace_id {
                    return Err(ApplicationServiceError::InvalidRequest {
                        reason: "project parent_id must equal workspace_id".to_owned(),
                    });
                }
                let safe_name = safe_workspace_name(&request.name)?;
                let project_id =
                    stable_workspace_node_id("project", &context.workspace_id, &request.name)?;
                if !context
                    .projects
                    .iter()
                    .any(|item| item.project_id == project_id)
                {
                    context.projects.push(WorkspaceProjectState {
                        workspace_id: context.workspace_id.clone(),
                        project_id: project_id.clone(),
                        project_name: request.name.clone(),
                        safe_name,
                        regions: Vec::new(),
                    });
                }
                context.project_id = Some(project_id);
                context.project_name = Some(request.name.clone());
                context.region_id = None;
                context.region_name = None;
                Ok(())
            },
        )
    }

    pub fn project_select(
        &self,
        request: &WorkspaceSelectNodeRequest,
    ) -> Result<WorkspaceContextState, ApplicationServiceError> {
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        self.mutate_workspace_context(
            request.request_id.as_str(),
            request.expected_revision,
            |context| {
                if request.parent_id != context.workspace_id {
                    return Err(ApplicationServiceError::InvalidRequest {
                        reason: "project parent_id must equal workspace_id".to_owned(),
                    });
                }
                let project = context
                    .projects
                    .iter()
                    .find(|item| item.project_id == request.node_id)
                    .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                        reason: "project_id is not present in workspace context".to_owned(),
                    })?;
                context.project_id = Some(project.project_id.clone());
                context.project_name = Some(project.project_name.clone());
                context.region_id = None;
                context.region_name = None;
                Ok(())
            },
        )
    }

    pub fn region_create(
        &self,
        request: &WorkspaceNamedNodeRequest,
    ) -> Result<WorkspaceContextState, ApplicationServiceError> {
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        self.mutate_workspace_context(
            request.request_id.as_str(),
            request.expected_revision,
            |context| {
                let project_id = context.project_id.clone().ok_or_else(|| {
                    ApplicationServiceError::InvalidRequest {
                        reason: "a project must be selected before creating a region".to_owned(),
                    }
                })?;
                if request.parent_id != project_id {
                    return Err(ApplicationServiceError::InvalidRequest {
                        reason: "region parent_id must equal the selected project_id".to_owned(),
                    });
                }
                let safe_name = safe_workspace_name(&request.name)?;
                let region_id = stable_workspace_node_id("region", &project_id, &request.name)?;
                let project = context
                    .projects
                    .iter_mut()
                    .find(|item| item.project_id == project_id)
                    .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                        reason: "selected project is not present in workspace context".to_owned(),
                    })?;
                if !project
                    .regions
                    .iter()
                    .any(|item| item.region_id == region_id)
                {
                    project.regions.push(WorkspaceRegionState {
                        project_id: project_id.clone(),
                        region_id: region_id.clone(),
                        region_name: request.name.clone(),
                        safe_name,
                    });
                }
                context.region_id = Some(region_id);
                context.region_name = Some(request.name.clone());
                Ok(())
            },
        )
    }

    pub fn region_select(
        &self,
        request: &WorkspaceSelectNodeRequest,
    ) -> Result<WorkspaceContextState, ApplicationServiceError> {
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        self.mutate_workspace_context(
            request.request_id.as_str(),
            request.expected_revision,
            |context| {
                let project_id = context.project_id.clone().ok_or_else(|| {
                    ApplicationServiceError::InvalidRequest {
                        reason: "a project must be selected before selecting a region".to_owned(),
                    }
                })?;
                if request.parent_id != project_id {
                    return Err(ApplicationServiceError::InvalidRequest {
                        reason: "region parent_id must equal the selected project_id".to_owned(),
                    });
                }
                let region = context
                    .projects
                    .iter()
                    .find(|item| item.project_id == project_id)
                    .and_then(|project| {
                        project
                            .regions
                            .iter()
                            .find(|item| item.region_id == request.node_id)
                    })
                    .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                        reason: "region_id is not present under the selected project".to_owned(),
                    })?;
                context.region_id = Some(region.region_id.clone());
                context.region_name = Some(region.region_name.clone());
                Ok(())
            },
        )
    }

    pub fn provider_capabilities(
        &self,
    ) -> Result<ProviderCapabilitiesResponse, ApplicationServiceError> {
        let dispatcher = self.dispatcher_handle()?;
        Ok(dispatcher.provider_capabilities())
    }

    /// Return all versioned static resources without opening a Store or
    /// initializing a provider runtime.
    pub fn list_known_resources(
        &self,
    ) -> Result<Vec<KnownResourceSummary>, ApplicationServiceError> {
        Ok(self.capability_query_service()?.list_known_resources())
    }

    pub fn resource_descriptor(
        &self,
        resource_id: &str,
    ) -> Result<Option<insar_core::ResourceDescriptor>, ApplicationServiceError> {
        Ok(self
            .capability_query_service()?
            .resource_descriptor(resource_id))
    }

    pub fn supported_search_capabilities(
        &self,
        resource_id: &str,
    ) -> Result<Vec<insar_core::CapabilityDeclaration>, ApplicationServiceError> {
        Ok(self
            .capability_query_service()?
            .supported_search_capabilities(resource_id))
    }

    pub fn delivery_capabilities(
        &self,
        resource_id: &str,
    ) -> Result<Vec<ResourceDeliveryCapability>, ApplicationServiceError> {
        Ok(self
            .capability_query_service()?
            .delivery_capabilities(resource_id))
    }

    pub fn effective_capability(
        &self,
        capability_id: &str,
    ) -> Result<Option<EffectiveCapabilityStatus>, ApplicationServiceError> {
        Ok(self
            .capability_query_service()?
            .effective_capability(capability_id))
    }

    pub fn resource_dependencies(
        &self,
        resource_id: Option<&str>,
        capability_id: Option<&str>,
        planning_context_id: Option<&str>,
    ) -> Result<Vec<insar_core::ResourceDependency>, ApplicationServiceError> {
        Ok(self.capability_query_service()?.resource_dependencies(
            resource_id,
            capability_id,
            planning_context_id,
        ))
    }

    /// #22 B slice 3(c): cancel a catalogue search. The background fetch is
    /// signalled to stop; a search that is already readable (loading) is
    /// sealed at once as `cancelled` with its arrived rows kept and marked
    /// possibly incomplete. A search cancelled before its first full page
    /// leaves no batch, exactly as before. When the request had already
    /// finished, the answer names its batch and earlier terminal state.
    pub fn cancel_catalog_search(
        &self,
        request_id: &str,
    ) -> Result<SearchCancelOutcome, ApplicationServiceError> {
        let cancelled = self.cancel_request(request_id)?;
        let kept = if cancelled {
            self.finalize_loading_catalogue(request_id, SearchBatchProgressState::Cancelled)?
        } else {
            None
        };
        let (batch_id, state) = match kept {
            Some(batch_id) => (Some(batch_id), Some(SearchBatchProgressState::Cancelled)),
            None => self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                Ok(store
                    .latest_search_batch_progress_for_request(request_id)?
                    .map_or((None, None), |progress| {
                        (Some(progress.batch_id), Some(progress.state))
                    }))
            })?,
        };
        Ok(SearchCancelOutcome {
            request_id: request_id.to_owned(),
            cancelled,
            batch_id,
            state,
        })
    }

    /// #22 B slice 3(c): the delivery row of a progressively published
    /// batch (state, loaded count, provider total); `None` for a batch that
    /// was never progressively published.
    pub fn search_batch_progress(
        &self,
        batch_id: &str,
    ) -> Result<Option<StoreBatchProgress>, ApplicationServiceError> {
        let store = self.store_handle()?;
        let progress = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .search_batch_progress(batch_id)
            .map_err(ApplicationServiceError::from);
        progress
    }

    pub fn cancel_request(&self, request_id: &str) -> Result<bool, ApplicationServiceError> {
        if !valid_opaque_id(request_id) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "request_id must be a bounded opaque identifier".to_owned(),
            });
        }
        let quicklook_cancelled = self.quicklook_cancellations.cancel(request_id);
        match self.dispatcher_handle() {
            Ok(dispatcher) => Ok(dispatcher.cancel_request(request_id) || quicklook_cancelled),
            Err(_) if quicklook_cancelled => Ok(true),
            Err(error) => Err(error),
        }
    }

    /// Return only redacted, session-scoped CDSE state. This operation never
    /// opens a Store or contacts the identity service.
    pub fn cdse_session_status(&self) -> Result<CdseSessionStatus, ApplicationServiceError> {
        self.ensure_cdse_controls_available()?;
        self.cdse_session_runtime()?
            .status()
            .map_err(ApplicationServiceError::Cdse)
    }

    /// Exchange write-only login input for a runtime-only CDSE session. The
    /// request is consumed immediately; neither this Service nor its Store keeps
    /// username, password, TOTP, access token, or refresh token.
    pub fn cdse_login(
        &self,
        request: CdseLoginRequest,
    ) -> Result<CdseSessionStatus, ApplicationServiceError> {
        self.ensure_cdse_controls_available()?;
        let CdseLoginRequest {
            username,
            password,
            totp,
        } = request;
        let material = CdseLoginMaterial::new(username, password, totp)
            .map_err(ApplicationServiceError::Cdse)?;
        let runtime = self.cdse_session_runtime()?;
        let snapshot = runtime
            .session
            .login(material)
            .map_err(ApplicationServiceError::Cdse)?;
        Ok(cdse_status(runtime.network_policy, snapshot))
    }

    /// Immediately zeroize the runtime-only CDSE access and refresh material.
    /// Existing transfer requests already holding a resolved Authorization value
    /// are not rewritten; future resolves observe the signed-out session.
    pub fn cdse_logout(&self) -> Result<CdseSessionStatus, ApplicationServiceError> {
        self.ensure_cdse_controls_available()?;
        let runtime = self.cdse_session_runtime()?;
        runtime
            .session
            .logout()
            .map_err(ApplicationServiceError::Cdse)?;
        runtime.status().map_err(ApplicationServiceError::Cdse)
    }

    /// Return only secret-free state for the process-local USGS M2M session.
    pub fn usgs_m2m_session_status(
        &self,
    ) -> Result<UsgsM2mDesktopSessionStatus, ApplicationServiceError> {
        self.ensure_usgs_controls_available()?;
        self.usgs_m2m_session_runtime()?
            .status()
            .map_err(ApplicationServiceError::UsgsM2m)
    }

    /// Consume one explicit username/Application Token request. Manual login may
    /// start a fresh attempt after logout or a failed attempt; no operation
    /// performs an automatic authentication retry.
    pub fn usgs_m2m_login(
        &self,
        request: UsgsM2mLoginRequest,
    ) -> Result<UsgsM2mDesktopSessionStatus, ApplicationServiceError> {
        self.ensure_usgs_controls_available()?;
        let UsgsM2mLoginRequest {
            username,
            application_token,
        } = request;
        let token = UsgsM2mApplicationToken::new(application_token)
            .map_err(ApplicationServiceError::UsgsM2m)?;
        let material =
            UsgsM2mLoginMaterial::new(username, token).map_err(ApplicationServiceError::UsgsM2m)?;
        let runtime = self.usgs_m2m_session_runtime()?;
        // A login that is already in flight (accounts page restoring the
        // session at startup) is awaited, not rejected: a download admission
        // that races it must fail only on a real authentication error.
        runtime
            .session
            .login_validating_material(material, &|| false)
            .map_err(ApplicationServiceError::UsgsM2m)?;
        runtime.status().map_err(ApplicationServiceError::UsgsM2m)
    }

    /// Clear the runtime API key even when the bounded remote logout fails.
    pub fn usgs_m2m_logout(&self) -> Result<UsgsM2mDesktopSessionStatus, ApplicationServiceError> {
        self.ensure_usgs_controls_available()?;
        let runtime = self.usgs_m2m_session_runtime()?;
        runtime
            .session
            .logout(&|| false)
            .map_err(ApplicationServiceError::UsgsM2m)?;
        runtime.status().map_err(ApplicationServiceError::UsgsM2m)
    }

    pub fn cdse_network_policy(&self) -> Result<CdseNetworkMode, ApplicationServiceError> {
        self.ensure_cdse_controls_available()?;
        Ok(CdseNetworkMode::from_policy(
            self.cdse_session_runtime()?.network_policy,
        ))
    }

    /// Change the route used by future CDSE catalogue/auth/archive operations.
    /// A worker with an active item is never replaced, so callers must pause or
    /// finish active downloads before changing modes. Switching routes clears
    /// the old session rather than moving a token across transport policies.
    pub fn set_cdse_network_policy(
        &self,
        mode: CdseNetworkMode,
    ) -> Result<CdseSessionStatus, ApplicationServiceError> {
        self.ensure_cdse_controls_available()?;
        let requested = mode.policy()?;
        let _catalogue_policy_gate = match self.catalogue_policy_gate.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return Err(ApplicationServiceError::CdseNetworkBusy),
            Err(TryLockError::Poisoned(poisoned)) => {
                catalogue_diagnostic(
                    "cdse-network-policy",
                    "catalogue_policy_gate_poison_recovered",
                );
                self.catalogue_policy_gate.clear_poison();
                poisoned.into_inner()
            }
        };
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let mut session = self
            .cdse_session
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let current = self.cdse_session_runtime_locked(&mut session)?;
        // The legacy "system" choice is how a saved manual proxy is reported.
        if CdseNetworkMode::from_policy(current.network_policy) == mode {
            return current.status().map_err(ApplicationServiceError::Cdse);
        }
        if let Some(existing) = runtime.as_ref() {
            if runtime_has_active_cdse_work(existing)? {
                return Err(ApplicationServiceError::CdseNetworkBusy);
            }
        }

        let replacement =
            CdseSessionRuntime::new(requested).map_err(ApplicationServiceError::Cdse)?;
        let previous_session = session.replace(replacement.clone());
        let previous_runtime = runtime.take();
        drop(session);
        drop(runtime);
        drop(_catalogue_policy_gate);
        if let Some(previous_session) = previous_session {
            previous_session
                .session
                .logout()
                .map_err(ApplicationServiceError::Cdse)?;
        }
        drop(previous_runtime);
        replacement.status().map_err(ApplicationServiceError::Cdse)
    }

    /// Perform one explicit credential-free diagnostic through the requested
    /// route. It never reads or changes the session, Store, task, or proxy
    /// configuration.
    pub fn cdse_network_diagnostic(
        &self,
        mode: CdseNetworkMode,
    ) -> Result<CdseNetworkDiagnosticResult, ApplicationServiceError> {
        self.ensure_cdse_controls_available()?;
        let policy = mode.policy()?;
        let stage = match ReqwestCdseNetworkProbe::new(policy) {
            Ok(probe) => {
                CdseNetworkDiagnostics::new(policy, Arc::new(probe))
                    .diagnose()
                    .stage
            }
            Err(stage) => stage,
        };
        Ok(CdseNetworkDiagnosticResult {
            network_mode: mode,
            stage: diagnostic_stage_name(stage).to_owned(),
        })
    }

    /// Resolve and materialize one Preview from its persisted frozen
    /// SearchBatch. The UI supplies only frozen identifiers; Provider locators,
    /// redirects, credentials and response bytes stay inside the desktop
    /// boundary. Historical products therefore remain previewable while the
    /// mutable Workbench points at another batch.
    pub fn preview_asset(
        &self,
        request: PreviewAssetRequest,
    ) -> Result<ResolvedPreviewAsset, ApplicationServiceError> {
        if request.validate().is_err()
            || !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.batch_id)
            || !valid_catalogue_opaque_id(&request.item_id)
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Preview request identifiers must be bounded opaque values".to_owned(),
            });
        }
        let plan = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let source = load_frozen_preview_source(&store, &request.batch_id, &request.item_id)
                .map_err(ApplicationServiceError::from)?
                .ok_or(ApplicationServiceError::Preview(
                    PreviewAssetError::ProviderNoPreview,
                ))?;
            frozen_preview_plan(&source.item)
        })?;

        match plan {
            FrozenPreviewPlan::Direct {
                product_key,
                product_class,
                assets,
            } => {
                let cancelled = self
                    .quicklook_cancellations
                    .try_register(&request.request_id)
                    .ok_or(ApplicationServiceError::Preview(PreviewAssetError::Timeout))?;
                let result = (|| {
                    let candidate = resolve_preview_candidate(&product_key, product_class, &assets)
                        .map_err(ApplicationServiceError::Preview)?;
                    let service = if matches!(product_class, PreviewProductClass::BhoonidhiSsarRslc)
                    {
                        network_routes::ServiceRoute::Bhoonidhi
                    } else {
                        network_routes::ServiceRoute::Ordinary
                    };
                    let client =
                        PreviewHttpClient::with_route(self.network_policy(service).route())
                            .map_err(ApplicationServiceError::Preview)?;
                    let image = client
                        .fetch(&candidate, &|| cancelled.load(Ordering::Acquire))
                        .map_err(ApplicationServiceError::Preview)?;
                    let byte_length = u64::try_from(image.bytes.len()).map_err(|_| {
                        ApplicationServiceError::Preview(PreviewAssetError::DecodeFailed)
                    })?;
                    let data_url = format!(
                        "data:{};base64,{}",
                        image.media_type,
                        BASE64_STANDARD.encode(&image.bytes)
                    );
                    let resolved = ResolvedPreviewAsset {
                        product_key: candidate.product_key,
                        descriptor: candidate.descriptor,
                        source_asset_role: candidate.source_asset_role,
                        source_asset_key: candidate.source_asset_key,
                        media_type: image.media_type,
                        data_url,
                        byte_length,
                    };
                    resolved.validate().map_err(|_| {
                        ApplicationServiceError::Preview(PreviewAssetError::DecodeFailed)
                    })?;
                    Ok(resolved)
                })();
                self.quicklook_cancellations.finish(&request.request_id);
                result
            }
            FrozenPreviewPlan::Cdse => {
                let response = self.cdse_quicklook(CdseQuicklookRequest {
                    request_id: request.request_id,
                    batch_id: request.batch_id,
                    item_id: request.item_id.clone(),
                })?;
                resolved_cdse_static_quicklook(request.item_id, response)
            }
            FrozenPreviewPlan::Landsat => {
                let response = self.landsat_browse(LandsatBrowseRequest {
                    request_id: request.request_id,
                    batch_id: request.batch_id,
                    item_id: request.item_id.clone(),
                })?;
                resolved_landsat_scene_browse(request.item_id, response)
            }
        }
    }

    /// Persist one already-resolved desktop Preview image. Only bounded image
    /// data URLs are accepted so a renderer cannot turn this into a generic
    /// network fetch or arbitrary file write primitive.
    pub fn save_preview_asset(
        &self,
        request: PreviewSaveRequest,
    ) -> Result<PreviewSaveResponse, ApplicationServiceError> {
        const MAX_PREVIEW_SAVE_BYTES: usize = 16 * 1024 * 1024;
        let (media_type, encoded) = request.data_url.split_once(";base64,").ok_or_else(|| {
            ApplicationServiceError::InvalidRequest {
                reason: "Preview data URL must use base64 encoding".to_owned(),
            }
        })?;
        let media_type = media_type.strip_prefix("data:").ok_or_else(|| {
            ApplicationServiceError::InvalidRequest {
                reason: "Preview data URL media type is invalid".to_owned(),
            }
        })?;
        let extension = match media_type {
            "image/jpeg" => ".jpg",
            "image/png" => ".png",
            "image/webp" => ".webp",
            _ => {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "Preview data URL media type is not supported".to_owned(),
                });
            }
        };
        if encoded.is_empty() || encoded.len() % 4 != 0 || encoded.len() > 24 * 1024 * 1024 {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Preview data URL payload is invalid or too large".to_owned(),
            });
        }
        let bytes = BASE64_STANDARD.decode(encoded).map_err(|_| {
            ApplicationServiceError::InvalidRequest {
                reason: "Preview data URL payload is invalid".to_owned(),
            }
        })?;
        if bytes.is_empty() || bytes.len() > MAX_PREVIEW_SAVE_BYTES {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Preview image is empty or exceeds the 16 MiB safety limit".to_owned(),
            });
        }

        let output_dir = request.output_dir;
        if !output_dir.is_absolute() || !output_dir.is_dir() {
            return Err(ApplicationServiceError::PreviewDirectoryInvalid);
        }
        let output_dir = output_dir
            .canonicalize()
            .map_err(|_| ApplicationServiceError::PreviewDirectoryInvalid)?;
        let stem = safe_preview_file_stem(&request.file_name);
        let file_name = format!("{stem}__preview{extension}");
        let destination = output_dir.join(&file_name);
        let temporary = output_dir.join(format!(".{file_name}.{}.part", Uuid::new_v4()));

        let write_result = (|| -> std::io::Result<()> {
            std::fs::write(&temporary, &bytes)?;
            match std::fs::rename(&temporary, &destination) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::fs::remove_file(&destination)?;
                    std::fs::rename(&temporary, &destination)
                }
                Err(error) => Err(error),
            }
        })();
        if write_result.is_err() {
            let _ = std::fs::remove_file(&temporary);
            return Err(ApplicationServiceError::PreviewWriteFailed);
        }

        Ok(PreviewSaveResponse {
            path: destination.to_string_lossy().into_owned(),
            file_name,
            bytes_written: bytes.len() as u64,
        })
    }

    /// Materialize one reviewed quicklook from its persisted frozen SearchBatch.
    /// The request carries no URL, asset identity, or secret. The exact frozen
    /// product UUID drives one bounded anonymous discovery before its reviewed
    /// image is returned as a runtime-only data URL.
    pub fn cdse_quicklook(
        &self,
        request: CdseQuicklookRequest,
    ) -> Result<CdseQuicklookResponse, ApplicationServiceError> {
        self.ensure_cdse_controls_available()?;
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.batch_id)
            || !valid_catalogue_opaque_id(&request.item_id)
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "quicklook request identifiers must be bounded opaque values".to_owned(),
            });
        }
        let dispatcher = self.dispatcher_handle()?;
        if dispatcher
            .catalogue_route_use()
            .map_err(ApplicationServiceError::Provider)?
            != RouteUse::Production
        {
            return Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid);
        }

        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::CdseQuicklookRequestDuplicate)?;
        let result = (|| {
            let evidence = self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let source =
                    load_frozen_preview_source(&store, &request.batch_id, &request.item_id)
                        .map_err(ApplicationServiceError::from)?
                        .ok_or(ApplicationServiceError::CdseQuicklookEvidenceInvalid)?;
                frozen_cdse_quicklook_product(&source.batch, &source.item)
            })?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::CdseQuicklook(
                    CdseQuicklookError::Cancelled,
                ));
            }
            // Freeze one System/Direct policy while registered with the policy
            // gate, then release the gate before lazy discovery and image I/O.
            self.apply_pending_network_policy();
            let runtime = {
                let _catalogue_policy_gate = match self.catalogue_policy_gate.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        catalogue_diagnostic(
                            &request.request_id,
                            "catalogue_policy_gate_poison_recovered",
                        );
                        self.catalogue_policy_gate.clear_poison();
                        poisoned.into_inner()
                    }
                };
                self.cdse_session_runtime()?
            };
            let client = CdseQuicklookClient::new(runtime.network_policy)
                .map_err(ApplicationServiceError::CdseQuicklook)?;
            let descriptor = client
                .discover(&evidence.product_id, &|| cancelled.load(Ordering::Acquire))
                .map_err(ApplicationServiceError::CdseQuicklook)?
                .ok_or(ApplicationServiceError::CdseQuicklookUnavailable)?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::CdseQuicklook(
                    CdseQuicklookError::Cancelled,
                ));
            }
            let image = client
                .fetch(
                    &descriptor.asset_id,
                    &descriptor.media_type,
                    descriptor.expected_length,
                    &|| cancelled.load(Ordering::Acquire),
                )
                .map_err(ApplicationServiceError::CdseQuicklook)?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::CdseQuicklook(
                    CdseQuicklookError::Cancelled,
                ));
            }
            let byte_length = image.len();
            let data_url = format!(
                "data:{};base64,{}",
                image.media_type(),
                BASE64_STANDARD.encode(image.bytes())
            );
            Ok(CdseQuicklookResponse {
                media_type: image.media_type().to_owned(),
                data_url,
                byte_length,
            })
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    /// Resolve a frozen CDSE scene to the exact matching MPC map-only Item.
    /// Search/download identity remains CDSE-owned; MPC is used only after the
    /// remote SAFE identity has been proven equal.
    pub fn sentinel2_map_preview(
        &self,
        request: Sentinel2MapPreviewRequest,
    ) -> Result<Sentinel2MapPreviewResponse, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.batch_id)
            || !valid_catalogue_opaque_id(&request.item_id)
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Sentinel-2 map preview identifiers must be bounded opaque values"
                    .to_owned(),
            });
        }
        let source = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            load_frozen_preview_source(&store, &request.batch_id, &request.item_id)
                .map_err(ApplicationServiceError::from)?
                .ok_or(ApplicationServiceError::CdseQuicklookEvidenceInvalid)
        })?;
        let evidence = frozen_cdse_quicklook_product(&source.batch, &source.item)?;
        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::Preview(PreviewAssetError::Timeout))?;
        let result = (|| {
            let runtime = self.cdse_session_runtime()?;
            let client = MpcSentinel2CatalogClient::new(runtime.network_policy)
                .map_err(ApplicationServiceError::Provider)?;
            let map_item_id = client
                .resolve_map_item_id(
                    &evidence.safe_name,
                    evidence.acquisition_datetime.as_deref(),
                    &|| cancelled.load(Ordering::Acquire),
                )
                .map_err(ApplicationServiceError::Provider)?;
            Ok(Sentinel2MapPreviewResponse {
                product_key: request.item_id.clone(),
                map_item_id,
            })
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    /// Resolve and fetch one exact Landsat browse from its persisted frozen
    /// batch.
    /// The request contains no locator, credential, or mutable Provider input.
    pub fn landsat_browse(
        &self,
        request: LandsatBrowseRequest,
    ) -> Result<LandsatBrowseResponse, ApplicationServiceError> {
        self.ensure_usgs_controls_available()?;
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.batch_id)
            || !valid_catalogue_opaque_id(&request.item_id)
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Landsat browse identifiers must be bounded opaque values".to_owned(),
            });
        }
        let dispatcher = self.dispatcher_handle()?;
        if dispatcher
            .catalogue_route_use()
            .map_err(ApplicationServiceError::Provider)?
            != RouteUse::Production
        {
            return Err(ApplicationServiceError::LandsatBrowseEvidenceInvalid);
        }

        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::LandsatBrowseRequestDuplicate)?;
        let result = (|| {
            let evidence = self
                .with_business_runtime(|runtime| {
                    let store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    let source =
                        load_frozen_preview_source(&store, &request.batch_id, &request.item_id)
                            .map_err(ApplicationServiceError::from)?
                            .ok_or(ApplicationServiceError::LandsatBrowseEvidenceInvalid)?;
                    if source.batch.provider_id != source.item.provider_id {
                        return Err(ApplicationServiceError::LandsatBrowseEvidenceInvalid);
                    }
                    frozen_landsat_browse_evidence(&source.batch, &source.item)
                })?
                .ok_or(ApplicationServiceError::LandsatBrowseUnavailable)?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::LandsatBrowseCancelled);
            }
            let port = self.landsat_browse_port()?;
            let image = port
                .fetch_landsat_browse(&evidence.query, &evidence.entity_id, &|| {
                    cancelled.load(Ordering::Acquire)
                })
                .map_err(ApplicationServiceError::UsgsM2m)?
                .ok_or(ApplicationServiceError::LandsatBrowseUnavailable)?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::LandsatBrowseCancelled);
            }
            let byte_length = image.bytes.len();
            let data_url = format!(
                "data:{};base64,{}",
                image.media_type,
                BASE64_STANDARD.encode(&image.bytes)
            );
            Ok(LandsatBrowseResponse {
                media_type: image.media_type,
                data_url,
                byte_length,
            })
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    /// Resolve display-only asset metadata for one frozen Microsoft Landsat
    /// scene. This operation never creates a derived batch/task and never
    /// returns provider locators, SAS tokens, headers, or credentials.
    pub fn landsat_asset_details(
        &self,
        request: LandsatAssetDetailsRequest,
    ) -> Result<LandsatAssetDetailsResponse, ApplicationServiceError> {
        self.ensure_usgs_controls_available()?;
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.batch_id)
            || !valid_catalogue_opaque_id(&request.item_id)
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Landsat asset detail identifiers must be bounded opaque values".to_owned(),
            });
        }
        let dispatcher = self.dispatcher_handle()?;
        if dispatcher
            .catalogue_route_use()
            .map_err(ApplicationServiceError::Provider)?
            != RouteUse::Production
        {
            return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
        }
        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::LandsatAssetRequestDuplicate)?;
        let result = (|| {
            let source = self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                load_frozen_preview_source(&store, &request.batch_id, &request.item_id)
                    .map_err(ApplicationServiceError::from)?
                    .ok_or(ApplicationServiceError::LandsatAssetEvidenceInvalid)
            })?;
            if source.batch.provider_id != insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
                || source.item.provider_id != insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
            {
                return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
            }
            // The shared delivery validator owns the source/profile identity
            // checks. Page size has no authority in this read-only projection;
            // one is only the non-zero sentinel required by that wrapper.
            let evidence = frozen_landsat_scene_evidence(&source.batch, 1, &source.item)
                .map_err(|_| ApplicationServiceError::LandsatAssetEvidenceInvalid)?;
            if evidence.dataset_alias != insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID {
                return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
            }
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::LandsatAssetCancelled);
            }
            let manifest = self
                .landsat_asset_manifest_port()?
                .landsat_asset_manifest(
                    &evidence.dataset_alias,
                    &evidence.entity_id,
                    &evidence.display_id,
                    &|| cancelled.load(Ordering::Acquire),
                )
                .map_err(ApplicationServiceError::PlanetaryComputer)?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::LandsatAssetCancelled);
            }
            project_landsat_asset_details(&source.item, &evidence, manifest)
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    pub fn bhoonidhi_asset_size(
        &self,
        request: BhoonidhiAssetSizeRequest,
    ) -> Result<BhoonidhiAssetSizeResponse, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.batch_id)
            || !nisar_subset::valid_nisar_subset_source_item_id(&request.item_id)
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason:
                    "Bhoonidhi asset size identifiers must match the bounded frozen S-band identity"
                        .to_owned(),
            });
        }
        let dispatcher = self.dispatcher_handle()?;
        if dispatcher
            .catalogue_route_use()
            .map_err(ApplicationServiceError::Provider)?
            != RouteUse::Production
        {
            return Err(ApplicationServiceError::BhoonidhiAssetEvidenceInvalid);
        }
        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::BhoonidhiAssetRequestDuplicate)?;
        let result = (|| {
            let source = self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                load_frozen_preview_source(&store, &request.batch_id, &request.item_id)
                    .map_err(ApplicationServiceError::from)?
                    .ok_or(ApplicationServiceError::BhoonidhiAssetEvidenceInvalid)
            })?;
            if source.batch.provider_id != BHOONIDHI_PROVIDER_ID
                || source.item.provider_id != BHOONIDHI_PROVIDER_ID
            {
                return Err(ApplicationServiceError::BhoonidhiAssetEvidenceInvalid);
            }
            nisar_subset::validate_bhoonidhi_ssar_frozen_item(&source.item)
                .map_err(|_| ApplicationServiceError::BhoonidhiAssetEvidenceInvalid)?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::BhoonidhiAssetCancelled);
            }
            let asset = source
                .item
                .asset_access
                .as_ref()
                .ok_or(ApplicationServiceError::BhoonidhiAssetEvidenceInvalid)?;
            let network_policy = self.network_policy(network_routes::ServiceRoute::Bhoonidhi);
            let runtime = self.bhoonidhi_session_runtime(network_policy)?;
            let token_provider: Arc<dyn BhoonidhiAccessTokenProvider> = runtime.provider.clone();
            let resolver = BhoonidhiAssetResolver::new(
                token_provider,
                Arc::new(ReqwestBhoonidhiDownloadProbe::new()),
            );
            let clock = SystemClock;
            let cancellation = || cancelled.load(Ordering::Acquire);
            let context = ResolveContext::new(runtime.provider.as_ref(), &cancellation, &clock)
                .with_proxy_policy(network_policy.route());
            let resolved = resolver
                .resolve(&asset.reference, &asset.descriptor, &context)
                .map_err(bhoonidhi_asset_size_resolve_error)?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::BhoonidhiAssetCancelled);
            }
            let size_bytes = resolved
                .expected_size()
                .filter(|size| *size > 0)
                .ok_or(ApplicationServiceError::BhoonidhiAssetSizeUnavailable)?;
            Ok(BhoonidhiAssetSizeResponse {
                product_key: source.item.item_id,
                size_bytes,
            })
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    pub fn create_landsat_bundle_task(
        &self,
        request: &LandsatCreateBundleTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_usgs_controls_available()?;
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.source_batch_id)
            || !valid_catalogue_opaque_id(&request.item_id)
            || !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 96
            || request.output_root.as_os_str().is_empty()
            || request.layout != DownloadLayout::Direct
            || !(1..=10).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Landsat bundle task input is invalid".to_owned(),
            });
        }
        let dispatcher = self.dispatcher_handle()?;
        if dispatcher
            .catalogue_route_use()
            .map_err(ApplicationServiceError::Provider)?
            != RouteUse::Production
        {
            return Err(ApplicationServiceError::LandsatBundleEvidenceInvalid);
        }
        if matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            self.ensure_usgs_for_admission()?;
        }
        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::LandsatBundleRequestDuplicate)?;
        let result = (|| {
            let evidence = self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                // Admit from the exact frozen batch named by the request; the
                // mutable Workbench slot may hold another source's results
                // (unified search keeps one frozen batch per source).
                let batch = store
                    .load_search_batch(&request.source_batch_id)
                    .map_err(ApplicationServiceError::from)?
                    .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
                let item = batch
                    .candidates
                    .iter()
                    .find(|candidate| candidate.item_id == request.item_id)
                    .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
                frozen_landsat_archive_source_evidence(&batch, 1, item)
            })?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::LandsatBundleCancelled);
            }
            let (mut derived_batch, selected_id) =
                self.prepare_landsat_archive_source(&evidence, &request.item_id, &cancelled)?;
            let derived_batch_id = format!("landsat-delivery-{}", request.task_id);
            if !valid_catalogue_opaque_id(&derived_batch_id) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "derived Landsat batch identifier is too long".to_owned(),
                });
            }
            derived_batch.batch_id = derived_batch_id.clone();
            derived_batch.created_at = request.created_at.clone();
            derived_batch.query.insert(
                "derived_from_batch_id".to_owned(),
                Value::String(request.source_batch_id.clone()),
            );
            derived_batch.query.insert(
                "delivery_kind".to_owned(),
                Value::String(
                    if evidence.source_batch.provider_id == LANDSAT_C2_L2_PROVIDER_ID {
                        "landsat_c2_l2_full_bundle"
                    } else {
                        "landsat_c2_l2_matched_full_bundle"
                    }
                    .to_owned(),
                ),
            );
            derived_batch
                .validate()
                .map_err(|_| ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
            validate_debug_context(&derived_batch.query)
                .map_err(|_| ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
            let batch_request = StoreSearchBatchRequest {
                batch: derived_batch,
                workbench: SearchWorkbenchState {
                    batch_id: derived_batch_id.clone(),
                    checked_item_ids: vec![selected_id.clone()],
                    page_index: 0,
                    page_size: evidence.page_size,
                    revision: 0,
                },
                activate_workbench: false,
            };
            let task_request = StoreCreateTaskRequest {
                task_id: request.task_id.clone(),
                batch_id: derived_batch_id,
                selected_item_ids: vec![selected_id.clone()],
                checked_item_ids: vec![selected_id],
                page_index: 0,
                page_size: evidence.page_size,
                output_root: landsat_product_output_root(
                    &request.output_root,
                    &evidence.display_id,
                ),
                layout: DownloadLayout::Direct,
                concurrency: request.concurrency,
                created_at: request.created_at.clone(),
            };
            self.with_business_runtime(|runtime| {
                {
                    let mut store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    // Re-read the named source batch inside the write
                    // transaction; the mutable Workbench slot is irrelevant.
                    let current = store
                        .load_search_batch(&request.source_batch_id)
                        .map_err(ApplicationServiceError::from)?
                        .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
                    if current != evidence.source_batch {
                        return Err(ApplicationServiceError::LandsatBundleEvidenceInvalid);
                    }
                    if cancelled.load(Ordering::Acquire) {
                        return Err(ApplicationServiceError::LandsatBundleCancelled);
                    }
                    store
                        .save_search_batch_and_create_task(&batch_request, &task_request)
                        .map_err(ApplicationServiceError::from)?;
                }
                runtime
                    .scheduler
                    .as_ref()
                    .expect("business runtime guard established scheduler availability")
                    .wake();
                Self::snapshot_from_runtime(runtime)
            })
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    /// Explicit compatibility-only full snapshot for the historical Landsat
    /// bundle acceptance surface.
    pub fn create_landsat_bundle_task_legacy(
        &self,
        request: &LandsatCreateBundleTaskRequest,
    ) -> Result<ReadonlyServiceSnapshot, ApplicationServiceError> {
        self.create_landsat_bundle_task(request)?;
        self.snapshot(true)
    }

    pub fn create_landsat_asset_task(
        &self,
        request: &LandsatCreateBundleTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        let selection = LandsatCreateAssetTaskRequest {
            request_id: request.request_id.clone(),
            source_batch_id: request.source_batch_id.clone(),
            item_id: request.item_id.clone(),
            asset_keys: Vec::new(),
            task_id: request.task_id.clone(),
            output_root: request.output_root.clone(),
            layout: request.layout.clone(),
            concurrency: request.concurrency,
            created_at: request.created_at.clone(),
        };
        self.create_landsat_asset_selection_task(&selection)
    }

    pub fn create_landsat_asset_selection_task(
        &self,
        request: &LandsatCreateAssetTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.source_batch_id)
            || !valid_catalogue_opaque_id(&request.item_id)
            || !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 96
            || request.output_root.as_os_str().is_empty()
            || request.layout != DownloadLayout::Direct
            || !(1..=8).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
            || request.asset_keys.len() > 22
            || request.asset_keys.iter().any(|key| {
                key.is_empty()
                    || key.len() > 32
                    || !key.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'_' | b'.')
                    })
            })
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Landsat asset task input is invalid".to_owned(),
            });
        }
        let dispatcher = self.dispatcher_handle()?;
        if dispatcher
            .catalogue_route_use()
            .map_err(ApplicationServiceError::Provider)?
            != RouteUse::Production
        {
            return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
        }
        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::LandsatAssetRequestDuplicate)?;
        let result = (|| {
            let evidence = self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                // The request names its frozen source batch. Unified search
                // keeps one frozen batch per source and the mutable Workbench
                // slot may hold another source's results by the time the user
                // downloads, so the slot is never read here (burst-fix
                // pattern: admit from the exact frozen batch).
                let batch = store
                    .load_search_batch(&request.source_batch_id)
                    .map_err(ApplicationServiceError::from)?
                    .ok_or_else(|| {
                        landsat_asset_diagnostic(&request.request_id, "source_batch_missing");
                        ApplicationServiceError::LandsatAssetEvidenceInvalid
                    })?;
                let item = batch
                    .candidates
                    .iter()
                    .find(|candidate| candidate.item_id == request.item_id)
                    .ok_or_else(|| {
                        landsat_asset_diagnostic(&request.request_id, "item_not_in_batch");
                        ApplicationServiceError::LandsatAssetEvidenceInvalid
                    })?;
                // A stable non-zero page size is required by the frozen
                // evidence wrapper; the mutable slot page size has no
                // authority over the named batch, so use the same sentinel
                // the read-only asset-details projection uses.
                frozen_landsat_scene_evidence(&batch, 1, item).map_err(|error| {
                    landsat_asset_diagnostic(
                        &request.request_id,
                        match &error {
                            ApplicationServiceError::LandsatBundleEvidenceInvalid => {
                                "scene_evidence_invalid"
                            }
                            _ => "scene_evidence_error",
                        },
                    );
                    ApplicationServiceError::LandsatAssetEvidenceInvalid
                })
            })?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::LandsatAssetCancelled);
            }
            let source_item = evidence
                .source_batch
                .candidates
                .iter()
                .find(|candidate| candidate.item_id == request.item_id)
                .ok_or(ApplicationServiceError::LandsatAssetEvidenceInvalid)?;
            let prepared = self.prepare_landsat_asset_items(
                &evidence,
                source_item,
                &request.asset_keys,
                &cancelled,
            )?;
            let derived_items = prepared.items;
            let selected_item_ids = derived_items
                .iter()
                .map(|item| item.item_id.clone())
                .collect::<Vec<_>>();
            if selected_item_ids.is_empty() {
                return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
            }
            let derived_batch_id = format!("landsat-assets-{}", request.task_id);
            if !valid_catalogue_opaque_id(&derived_batch_id) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "derived Landsat asset batch identifier is too long".to_owned(),
                });
            }
            let mut derived_batch = evidence.source_batch.clone();
            derived_batch.batch_id = derived_batch_id.clone();
            derived_batch.created_at = request.created_at.clone();
            derived_batch.candidates = derived_items;
            derived_batch.query.insert(
                "derived_from_batch_id".to_owned(),
                Value::String(request.source_batch_id.clone()),
            );
            derived_batch.query.insert(
                "delivery_kind".to_owned(),
                Value::String(
                    if prepared.complete {
                        "landsat_c2_l2_complete_asset_set"
                    } else {
                        "landsat_c2_l2_asset_selection"
                    }
                    .to_owned(),
                ),
            );
            derived_batch.query.insert(
                "delivery_asset_count".to_owned(),
                Value::Number((selected_item_ids.len() as u64).into()),
            );
            derived_batch
                .validate()
                .map_err(|_| ApplicationServiceError::LandsatAssetEvidenceInvalid)?;
            validate_debug_context(&derived_batch.query)
                .map_err(|_| ApplicationServiceError::LandsatAssetEvidenceInvalid)?;
            let batch_request = StoreSearchBatchRequest {
                batch: derived_batch,
                workbench: SearchWorkbenchState {
                    batch_id: derived_batch_id.clone(),
                    checked_item_ids: selected_item_ids.clone(),
                    page_index: 0,
                    page_size: evidence.page_size,
                    revision: 0,
                },
                activate_workbench: false,
            };
            let task_request = StoreCreateTaskRequest {
                task_id: request.task_id.clone(),
                batch_id: derived_batch_id,
                selected_item_ids: selected_item_ids.clone(),
                checked_item_ids: selected_item_ids,
                page_index: 0,
                page_size: evidence.page_size,
                output_root: landsat_product_output_root(
                    &request.output_root,
                    &prepared.display_id,
                ),
                layout: DownloadLayout::Direct,
                concurrency: request.concurrency,
                created_at: request.created_at.clone(),
            };
            self.with_business_runtime(|runtime| {
                {
                    let mut store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    // Re-read the named source batch inside the write
                    // transaction and require it to be unchanged. The mutable
                    // Workbench slot is irrelevant: another source's search may
                    // have replaced it while this task was being prepared.
                    let current = store
                        .load_search_batch(&request.source_batch_id)
                        .map_err(ApplicationServiceError::from)?
                        .ok_or(ApplicationServiceError::LandsatAssetEvidenceInvalid)?;
                    if current != evidence.source_batch {
                        landsat_asset_diagnostic(
                            &request.request_id,
                            "source_batch_changed_during_preparation",
                        );
                        return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
                    }
                    store
                        .save_search_batch_and_create_task(&batch_request, &task_request)
                        .map_err(ApplicationServiceError::from)?;
                }
                runtime
                    .scheduler
                    .as_ref()
                    .expect("business runtime guard established scheduler availability")
                    .wake();
                Self::snapshot_from_runtime(runtime)
            })
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    /// Create one executable reviewed Planetary Computer COG task from an
    /// already-frozen production source item. The caller selects only asset keys;
    /// stable hrefs, remote sizes and runtime signing authority are re-resolved
    /// inside the application boundary.
    pub fn create_reviewed_mpc_asset_task(
        &self,
        request: &ReviewedMpcCreateAssetTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_running()?;
        let network_policy = self.cdse_session_runtime()?.network_policy;
        let evidence_port = PlanetaryComputerAssetSigner::new(network_policy)
            .map_err(ApplicationServiceError::Provider)?;
        self.create_reviewed_mpc_asset_task_with_port(request, &evidence_port)
    }

    #[doc(hidden)]
    pub fn create_reviewed_mpc_asset_task_with_evidence_for_test(
        &self,
        request: &ReviewedMpcCreateAssetTaskRequest,
        evidence_port: &dyn LiveReviewedMpcAssetEvidence,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.create_reviewed_mpc_asset_task_with_port(request, evidence_port)
    }

    fn create_reviewed_mpc_asset_task_with_port(
        &self,
        request: &ReviewedMpcCreateAssetTaskRequest,
        evidence_port: &dyn LiveReviewedMpcAssetEvidence,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.request_id)
            || request.request_id.len() > 80
            || !valid_catalogue_opaque_id(&request.source_batch_id)
            || !valid_exact_text(&request.item_id)
            || !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 72
            || request.output_root.as_os_str().is_empty()
            || !matches!(&request.layout, DownloadLayout::Direct)
            || !(1..=4).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
            || request.asset_keys.is_empty()
            || request.asset_keys.len() > 4
            || request.asset_keys.iter().any(|key| {
                !valid_exact_text(key)
                    || key.len() > 16
                    || !key.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "reviewed Planetary Computer asset task input is invalid".to_owned(),
            });
        }
        let mut unique_keys = BTreeSet::new();
        if request
            .asset_keys
            .iter()
            .any(|key| !unique_keys.insert(key.as_str()))
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "reviewed Planetary Computer asset keys must be unique".to_owned(),
            });
        }

        let source = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let workbench = store
                .load_workbench()
                .map_err(ApplicationServiceError::from)?
                .ok_or(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
            if workbench.batch.batch_id != request.source_batch_id {
                return Err(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid);
            }
            let source_item = workbench
                .batch
                .candidates
                .iter()
                .find(|candidate| candidate.item_id == request.item_id)
                .ok_or(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
            frozen_reviewed_mpc_source_evidence(
                &workbench.batch,
                workbench.state.page_size,
                source_item,
            )
        })?;
        if !reviewed_mpc_asset_keys_allowed(&source, &request.asset_keys) {
            return Err(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid);
        }

        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
        let result = (|| {
            let mut derived_items = Vec::with_capacity(request.asset_keys.len());
            for asset_key in &request.asset_keys {
                if cancelled.load(Ordering::Acquire) {
                    return Err(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid);
                }
                let resolved = evidence_port
                    .resolve_reviewed_asset(
                        source.class,
                        &source.provider_product_id,
                        asset_key,
                        &|| cancelled.load(Ordering::Acquire),
                    )
                    .map_err(ApplicationServiceError::Provider)?;
                let frozen_evidence = PlanetaryComputerReviewedAssetEvidence {
                    file_name: resolved.file_name,
                    expected_size_bytes: resolved.expected_size_bytes,
                };
                derived_items.push(
                    build_planetary_computer_reviewed_cog_item(
                        &source.source_item,
                        source.class,
                        asset_key,
                        &resolved.stable_href,
                        &frozen_evidence,
                    )
                    .map_err(|_| ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?,
                );
            }
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid);
            }

            let selected_item_ids = derived_items
                .iter()
                .map(|item| item.item_id.clone())
                .collect::<Vec<_>>();
            let derived_batch_id = format!("mpc-reviewed-assets-{}", request.task_id);
            if !valid_catalogue_opaque_id(&derived_batch_id) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "derived reviewed MPC batch identifier is too long".to_owned(),
                });
            }
            let mut derived_batch = source.source_batch.clone();
            derived_batch.batch_id = derived_batch_id.clone();
            derived_batch.created_at = request.created_at.clone();
            derived_batch.candidates = derived_items;
            derived_batch.query.insert(
                "derived_from_batch_id".to_owned(),
                Value::String(request.source_batch_id.clone()),
            );
            derived_batch.query.insert(
                "delivery_kind".to_owned(),
                Value::String("planetary_computer_reviewed_cog_assets".to_owned()),
            );
            derived_batch.query.insert(
                "delivery_source_item_id".to_owned(),
                Value::String(source.source_item.item_id.clone()),
            );
            derived_batch.query.insert(
                "delivery_asset_count".to_owned(),
                Value::Number((selected_item_ids.len() as u64).into()),
            );
            derived_batch
                .validate()
                .map_err(|_| ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
            validate_debug_context(&derived_batch.query)
                .map_err(|_| ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;

            let batch_request = StoreSearchBatchRequest {
                batch: derived_batch,
                workbench: SearchWorkbenchState {
                    batch_id: derived_batch_id.clone(),
                    checked_item_ids: selected_item_ids.clone(),
                    page_index: 0,
                    page_size: source.page_size,
                    revision: 0,
                },
                activate_workbench: false,
            };
            let task_request = StoreCreateTaskRequest {
                task_id: request.task_id.clone(),
                batch_id: derived_batch_id,
                selected_item_ids: selected_item_ids.clone(),
                checked_item_ids: selected_item_ids,
                page_index: 0,
                page_size: source.page_size,
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
                        .ok_or(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
                    if current.batch != source.source_batch {
                        return Err(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid);
                    }
                    store
                        .save_search_batch_and_create_task(&batch_request, &task_request)
                        .map_err(ApplicationServiceError::from)?;
                }
                runtime
                    .scheduler
                    .as_ref()
                    .expect("business runtime guard established scheduler availability")
                    .wake();
                Self::snapshot_from_runtime(runtime)
            })
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    /// Create one executable Sentinel-2 analysis-band task without changing
    /// the active search Workbench. Full SAFE delivery remains CDSE-owned; this
    /// route freezes only reviewed MPC reflectance COG assets.
    pub fn create_sentinel2_band_task(
        &self,
        request: &Sentinel2CreateBandTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_running()?;
        let network_policy = self.cdse_session_runtime()?.network_policy;
        let evidence = Sentinel2BandEvidenceRuntime::new(network_policy)
            .map_err(ApplicationServiceError::Provider)?;
        self.create_sentinel2_band_task_with_port(request, &evidence)
    }

    #[doc(hidden)]
    pub fn create_sentinel2_band_task_with_evidence_for_test(
        &self,
        request: &Sentinel2CreateBandTaskRequest,
        evidence: &dyn LiveSentinel2BandEvidence,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.create_sentinel2_band_task_with_port(request, evidence)
    }

    fn create_sentinel2_band_task_with_port(
        &self,
        request: &Sentinel2CreateBandTaskRequest,
        evidence_port: &dyn LiveSentinel2BandEvidence,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.request_id)
            || request.request_id.len() > 80
            || !valid_catalogue_opaque_id(&request.source_batch_id)
            || !valid_exact_text(&request.item_id)
            || !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 72
            || request.output_root.as_os_str().is_empty()
            || request.layout != DownloadLayout::Direct
            || !(1..=8).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
            || request.asset_keys.is_empty()
            || request.asset_keys.len() > 12
            || request.asset_keys.iter().any(|key| {
                key.len() > 3 || sentinel2_l2a_reflectance_band_resolution(key).is_none()
            })
            || request.asset_keys.iter().collect::<BTreeSet<_>>().len() != request.asset_keys.len()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Sentinel-2 analysis-band task input is invalid".to_owned(),
            });
        }

        let (source_batch, page_size, source_item, source_identity) =
            self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let workbench = store
                    .load_workbench()
                    .map_err(ApplicationServiceError::from)?
                    .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
                if workbench.batch.batch_id != request.source_batch_id {
                    return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
                }
                let source_item = workbench
                    .batch
                    .candidates
                    .iter()
                    .find(|candidate| candidate.item_id == request.item_id)
                    .cloned()
                    .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
                let source_identity =
                    frozen_sentinel2_analysis_source(&workbench.batch, &source_item)?;
                Ok((
                    workbench.batch,
                    workbench.state.page_size,
                    source_item,
                    source_identity,
                ))
            })?;

        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::Sentinel2BandRequestDuplicate)?;
        let result = (|| {
            let (mpc_item_id, derived_items) =
                sentinel2_band_delivery::prepare_sentinel2_band_items(
                    &source_item,
                    &source_identity,
                    &request.asset_keys,
                    evidence_port,
                    &cancelled,
                )?;
            let class = PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::Sentinel2BandCancelled);
            }

            let selected_item_ids = derived_items
                .iter()
                .map(|item| item.item_id.clone())
                .collect::<Vec<_>>();
            let derived_batch_id = format!("sentinel2-bands-{}", request.task_id);
            if !valid_catalogue_opaque_id(&derived_batch_id) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "derived Sentinel-2 band batch identifier is too long".to_owned(),
                });
            }
            let source_query = serde_json::from_value::<CatalogSearchQuery>(Value::Object(
                source_batch.query.clone(),
            ))
            .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
            let mut delivery_query = source_query;
            if source_identity.source_provider_id
                == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
            {
                delivery_query.provider_id =
                    insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID.to_owned();
                delivery_query.native_collection_id =
                    insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID.to_owned();
            } else {
                delivery_query.provider_id = PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned();
                delivery_query.native_collection_id = class.collection_id().to_owned();
            }
            delivery_query.cursor = None;
            delivery_query.result_limit = Some(100);
            delivery_query.filters = Map::new();
            delivery_query
                .validate()
                .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
            let mut derived_query = serde_json::to_value(&delivery_query)
                .ok()
                .and_then(|value| value.as_object().cloned())
                .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
            for (key, value) in [
                (
                    "derived_from_batch_id",
                    Value::String(request.source_batch_id.clone()),
                ),
                (
                    "delivery_kind",
                    Value::String("sentinel2_l2a_analysis_band_set".to_owned()),
                ),
                (
                    "delivery_source_item_id",
                    Value::String(request.item_id.clone()),
                ),
                (
                    "delivery_safe_name",
                    Value::String(source_identity.safe_name.clone()),
                ),
                (
                    "delivery_asset_count",
                    Value::Number((selected_item_ids.len() as u64).into()),
                ),
            ] {
                derived_query.insert(key.to_owned(), value);
            }
            validate_debug_context(&derived_query)
                .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;

            let mut derived_batch = source_batch.clone();
            derived_batch.batch_id = derived_batch_id.clone();
            derived_batch.provider_id = delivery_query.provider_id.clone();
            derived_batch.created_at = request.created_at.clone();
            derived_batch.query = derived_query;
            derived_batch.search_stats = None;
            derived_batch.candidates = derived_items;
            derived_batch
                .validate()
                .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;

            let batch_request = StoreSearchBatchRequest {
                batch: derived_batch,
                workbench: SearchWorkbenchState {
                    batch_id: derived_batch_id.clone(),
                    checked_item_ids: selected_item_ids.clone(),
                    page_index: 0,
                    page_size,
                    revision: 0,
                },
                activate_workbench: false,
            };
            let task_request = StoreCreateTaskRequest {
                task_id: request.task_id.clone(),
                batch_id: derived_batch_id,
                selected_item_ids: selected_item_ids.clone(),
                checked_item_ids: selected_item_ids,
                page_index: 0,
                page_size,
                output_root: sentinel2_band_output_root(&request.output_root, &mpc_item_id),
                layout: DownloadLayout::Direct,
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
                        .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
                    if current.batch != source_batch {
                        return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
                    }
                    store
                        .save_search_batch_and_create_task(&batch_request, &task_request)
                        .map_err(ApplicationServiceError::from)?;
                }
                runtime
                    .scheduler
                    .as_ref()
                    .expect("business runtime guard established scheduler availability")
                    .wake();
                Self::snapshot_from_runtime(runtime)
            })
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    fn resolve_sentinel2_archive_batch_item(
        context: Sentinel2ArchiveBatchLookupContext,
        source_item_id: String,
        source_item: Option<SnapshotItem>,
    ) -> Result<(String, Sentinel2ArchiveAdmissionOutcome), ApplicationServiceError> {
        let Sentinel2ArchiveBatchLookupContext {
            dispatcher,
            route_use,
            live_acceptance_candidate,
            source_query,
            lookup_request_id,
            cancelled,
        } = context;
        let Some(source_item) = source_item else {
            return Ok((
                source_item_id,
                Sentinel2ArchiveAdmissionOutcome::Rejected(
                    "SENTINEL2_ARCHIVE_FROZEN_EVIDENCE_INVALID",
                ),
            ));
        };
        let source_profile = CollectionProfileRegistry
            .for_source(
                &source_query.logical_dataset,
                &source_query.provider_id,
                &source_query.native_collection_id,
            )
            .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
        let delivery_profile = CollectionProfileRegistry
            .for_source(
                "optical.sentinel-2.l2a",
                "copernicus.dataspace.odata",
                "SENTINEL-2",
            )
            .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;

        let provider_product_id = source_item
            .metadata
            .get("provider_product_id")
            .and_then(Value::as_str)
            .filter(|value| valid_exact_text(value));
        let source_evidence_valid = source_item.provider_id == source_query.provider_id
            && source_item.asset_access.is_none()
            && source_item.execution_policy.is_none()
            && source_item
                .metadata
                .get("collection_profile_id")
                .and_then(Value::as_str)
                == Some(source_profile.profile_id)
            && source_item
                .metadata
                .get("resolver_policy_id")
                .and_then(Value::as_str)
                == Some(source_profile.resolver_policy_id)
            && source_item.footprint_geojson.is_some()
            && provider_product_id.is_some_and(|provider_product_id| {
                if source_query.provider_id
                    == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
                {
                    insar_provider::catalog::validate_earth_search_sentinel2_source_id(
                        provider_product_id,
                        &source_item.display_name,
                        source_item.acquisition_datetime.as_deref(),
                    )
                    .is_ok()
                } else {
                    insar_provider::catalog::validate_mpc_sentinel2_source_id(
                        provider_product_id,
                        &source_item.display_name,
                        source_item.acquisition_datetime.as_deref(),
                    )
                    .is_ok()
                }
            });
        if !source_evidence_valid {
            return Ok((
                source_item_id,
                Sentinel2ArchiveAdmissionOutcome::Rejected(
                    "SENTINEL2_ARCHIVE_FROZEN_EVIDENCE_INVALID",
                ),
            ));
        }

        let Some(acquisition) = source_item
            .acquisition_datetime
            .as_deref()
            .and_then(parse_normalized_utc)
        else {
            return Ok((
                source_item_id,
                Sentinel2ArchiveAdmissionOutcome::Rejected(
                    "SENTINEL2_ARCHIVE_FROZEN_EVIDENCE_INVALID",
                ),
            ));
        };
        let window_delta = if source_query.provider_id
            == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
        {
            chrono::TimeDelta::minutes(30)
        } else {
            chrono::TimeDelta::seconds(2)
        };
        let (Some(start), Some(end)) = (
            acquisition.checked_sub_signed(window_delta),
            acquisition.checked_add_signed(window_delta),
        ) else {
            return Ok((
                source_item_id,
                Sentinel2ArchiveAdmissionOutcome::Rejected(
                    "SENTINEL2_ARCHIVE_FROZEN_EVIDENCE_INVALID",
                ),
            ));
        };
        let spacecraft = match source_item.display_name.get(..3) {
            Some("S2A") => "sentinel_2a",
            Some("S2B") => "sentinel_2b",
            Some("S2C") => "sentinel_2c",
            _ => {
                return Ok((
                    source_item_id,
                    Sentinel2ArchiveAdmissionOutcome::Rejected(
                        "SENTINEL2_ARCHIVE_FROZEN_EVIDENCE_INVALID",
                    ),
                ))
            }
        };

        let mut delivery_query = source_query;
        delivery_query.provider_id = delivery_profile.provider_id.to_owned();
        delivery_query.native_collection_id = delivery_profile.native_collection_id.to_owned();
        delivery_query.aoi_geojson = source_item.footprint_geojson.clone();
        delivery_query.cursor = None;
        delivery_query.result_limit = Some(100);
        delivery_query.filters = Map::from_iter([
            (
                "product_type".to_owned(),
                Value::String(delivery_profile.required_product_type.to_owned()),
            ),
            (
                "spacecraft".to_owned(),
                Value::Array(vec![Value::String(spacecraft.to_owned())]),
            ),
            (
                "datetime".to_owned(),
                serde_json::json!({
                    "start": start.to_rfc3339_opts(SecondsFormat::AutoSi, true),
                    "end": end.to_rfc3339_opts(SecondsFormat::AutoSi, true),
                }),
            ),
        ]);
        delivery_query
            .validate()
            .map_err(|_| ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
        if !valid_catalogue_opaque_id(&lookup_request_id) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Sentinel-2 archive batch lookup identifier is too long".to_owned(),
            });
        }

        let operation = match dispatcher
            .register_catalog_search_for_application(&lookup_request_id, &delivery_query)
        {
            Ok(operation) => match operation
                .execute_with_parent_cancellation(&|| cancelled.load(Ordering::Acquire))
            {
                Ok(operation) => operation,
                Err(error)
                    if error.code.contains("CANCEL") || cancelled.load(Ordering::Acquire) =>
                {
                    return Err(ApplicationServiceError::Sentinel2ArchiveCancelled)
                }
                Err(_) => {
                    return Ok((
                        source_item_id,
                        Sentinel2ArchiveAdmissionOutcome::Rejected(
                            "SENTINEL2_ARCHIVE_LOOKUP_FAILED",
                        ),
                    ))
                }
            },
            Err(error) if error.code.contains("CANCEL") || cancelled.load(Ordering::Acquire) => {
                return Err(ApplicationServiceError::Sentinel2ArchiveCancelled)
            }
            Err(_) => {
                return Ok((
                    source_item_id,
                    Sentinel2ArchiveAdmissionOutcome::Rejected("SENTINEL2_ARCHIVE_LOOKUP_FAILED"),
                ))
            }
        };
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::Sentinel2ArchiveCancelled);
        }
        if let Err(error) = validate_complete_catalogue_response(
            &operation.response,
            &delivery_query,
            route_use,
            live_acceptance_candidate,
        ) {
            if sar_search_error_is_cancelled(&error) || cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::Sentinel2ArchiveCancelled);
            }
            return Ok((
                source_item_id,
                Sentinel2ArchiveAdmissionOutcome::Rejected("SENTINEL2_ARCHIVE_LOOKUP_FAILED"),
            ));
        }
        let mut matches = operation
            .response
            .products
            .into_iter()
            .filter(|product| product.display_name == source_item.display_name)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Ok((
                source_item_id,
                Sentinel2ArchiveAdmissionOutcome::Rejected(
                    "SENTINEL2_ARCHIVE_DELIVERY_UNAVAILABLE",
                ),
            ));
        }
        let Ok(delivery_item) = map_sentinel2_catalogue_product(
            matches.pop().expect("one exact CDSE match is checked"),
            &delivery_query,
            route_use,
        ) else {
            return Ok((
                source_item_id,
                Sentinel2ArchiveAdmissionOutcome::Rejected(
                    "SENTINEL2_ARCHIVE_FROZEN_EVIDENCE_INVALID",
                ),
            ));
        };
        if delivery_item.display_name != source_item.display_name
            || delivery_item.provider_id != delivery_profile.provider_id
            || delivery_item.asset_access.is_none()
            || delivery_item.execution_policy.is_none()
        {
            return Ok((
                source_item_id,
                Sentinel2ArchiveAdmissionOutcome::Rejected(
                    "SENTINEL2_ARCHIVE_FROZEN_EVIDENCE_INVALID",
                ),
            ));
        }
        Ok((
            source_item_id,
            Sentinel2ArchiveAdmissionOutcome::Admitted(Box::new(delivery_item)),
        ))
    }

    /// Create one Store-owned SAFE archive task from a bounded frozen
    /// Sentinel-2 selection. Scene admission is resolved before any Store write;
    /// partial admission stays explicit in the derived SearchBatch query instead
    /// of becoming invisible missing rows or transfer failures.
    pub fn create_sentinel2_archive_batch_task(
        &self,
        request: &Sentinel2CreateArchiveBatchTaskRequest,
    ) -> Result<Sentinel2CreateArchiveBatchTaskResult, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.request_id)
            || request.request_id.len() > 60
            || !valid_catalogue_opaque_id(&request.source_batch_id)
            || request.item_ids.len() > MAX_SENTINEL2_ARCHIVE_BATCH_ITEMS
            || request
                .item_ids
                .iter()
                .any(|item_id| !valid_exact_text(item_id))
            || request.item_ids.iter().collect::<BTreeSet<_>>().len() != request.item_ids.len()
            || !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 72
            || request.output_root.as_os_str().is_empty()
            || request.layout != DownloadLayout::Direct
            || !(1..=8).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Sentinel-2 archive batch task input is invalid".to_owned(),
            });
        }

        let dispatcher = self.dispatcher_handle()?;
        let route_use = dispatcher
            .catalogue_route_use()
            .map_err(ApplicationServiceError::Provider)?;
        if route_use != RouteUse::Production {
            return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid);
        }
        if matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            self.ensure_cdse_for_admission()?;
        }
        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::Sentinel2ArchiveRequestDuplicate)?;

        let result = (|| {
            let (source_batch, page_size, source_query, source_items, selected_item_ids) = self
                .with_business_runtime(|runtime| {
                    let store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    let workbench = store
                        .load_workbench()
                        .map_err(ApplicationServiceError::from)?
                        .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    if workbench.batch.batch_id != request.source_batch_id
                        || workbench.state.revision != request.expected_workbench_revision
                    {
                        return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid);
                    }
                    let source_query = serde_json::from_value::<CatalogSearchQuery>(Value::Object(
                        workbench.batch.query.clone(),
                    ))
                    .map_err(|_| ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    let valid_secondary_source = (source_query.provider_id
                        == insar_provider::catalog::MPC_SENTINEL2_L2A_PROVIDER_ID
                        && source_query.native_collection_id
                            == insar_provider::catalog::MPC_SENTINEL2_L2A_COLLECTION_ID)
                        || (source_query.provider_id
                            == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
                            && source_query.native_collection_id
                                == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID);
                    if source_query.logical_dataset != "optical.sentinel-2.l2a"
                        || !valid_secondary_source
                        || workbench.batch.provider_id != source_query.provider_id
                    {
                        return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid);
                    }
                    let selected_item_ids = if request.item_ids.is_empty() {
                        workbench.state.checked_item_ids.clone()
                    } else {
                        request.item_ids.clone()
                    };
                    if selected_item_ids.is_empty()
                        || selected_item_ids.len() > MAX_SENTINEL2_ARCHIVE_BATCH_ITEMS
                        || selected_item_ids
                            .iter()
                            .any(|item_id| !valid_exact_text(item_id))
                        || selected_item_ids.iter().collect::<BTreeSet<_>>().len()
                            != selected_item_ids.len()
                    {
                        return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid);
                    }
                    let requested = selected_item_ids
                        .iter()
                        .map(String::as_str)
                        .collect::<BTreeSet<_>>();
                    let source_items = workbench
                        .batch
                        .candidates
                        .iter()
                        .filter(|candidate| requested.contains(candidate.item_id.as_str()))
                        .cloned()
                        .collect::<Vec<_>>();
                    Ok((
                        workbench.batch,
                        workbench.state.page_size,
                        source_query,
                        source_items,
                        selected_item_ids,
                    ))
                })?;
            let progress = self
                .optical_preparations
                .begin(&request.request_id, selected_item_ids.len())?;

            let delivery_profile = CollectionProfileRegistry
                .for_source(
                    "optical.sentinel-2.l2a",
                    "copernicus.dataspace.odata",
                    "SENTINEL-2",
                )
                .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;

            let by_id = source_items
                .into_iter()
                .map(|item| (item.item_id.clone(), item))
                .collect::<BTreeMap<_, _>>();
            let indexed = selected_item_ids
                .iter()
                .enumerate()
                .map(|(index, source_item_id)| {
                    (
                        index,
                        source_item_id.clone(),
                        by_id.get(source_item_id).cloned(),
                    )
                })
                .collect::<Vec<_>>();
            let mut admitted = Vec::with_capacity(selected_item_ids.len());
            let mut rejections = Vec::new();

            for chunk in indexed.chunks(SENTINEL2_ARCHIVE_BATCH_LOOKUP_CONCURRENCY) {
                if cancelled.load(Ordering::Acquire) {
                    return Err(ApplicationServiceError::Sentinel2ArchiveCancelled);
                }
                let outcomes = std::thread::scope(|scope| {
                    let mut handles = Vec::with_capacity(chunk.len());
                    for (index, source_item_id, source_item) in chunk {
                        let context = Sentinel2ArchiveBatchLookupContext {
                            dispatcher: Arc::clone(&dispatcher),
                            route_use,
                            live_acceptance_candidate: self.catalogue_live_acceptance_candidate,
                            source_query: source_query.clone(),
                            lookup_request_id: format!("{}-cdse-{index}", request.request_id),
                            cancelled: Arc::clone(&cancelled),
                        };
                        let source_item_id = source_item_id.clone();
                        let source_item = source_item.clone();
                        let progress = &progress;
                        handles.push(scope.spawn(move || {
                            let result = Self::resolve_sentinel2_archive_batch_item(
                                context,
                                source_item_id,
                                source_item,
                            )?;
                            progress.record(matches!(
                                &result.1,
                                Sentinel2ArchiveAdmissionOutcome::Admitted(_)
                            ))?;
                            Ok::<_, ApplicationServiceError>(result)
                        }));
                    }
                    let mut outcomes = Vec::with_capacity(handles.len());
                    for handle in handles {
                        outcomes.push(
                            handle
                                .join()
                                .map_err(|_| ApplicationServiceError::RuntimeInitialization)??,
                        );
                    }
                    Ok::<_, ApplicationServiceError>(outcomes)
                })?;

                for (source_item_id, outcome) in outcomes {
                    match outcome {
                        Sentinel2ArchiveAdmissionOutcome::Admitted(item) => admitted.push(*item),
                        Sentinel2ArchiveAdmissionOutcome::Rejected(reason_code) => {
                            rejections.push(Sentinel2ArchiveAdmissionRejection {
                                source_item_id,
                                reason_code: reason_code.to_owned(),
                                reason_detail: None,
                            });
                        }
                    }
                }
            }

            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::Sentinel2ArchiveCancelled);
            }
            let admission = Sentinel2ArchiveAdmissionSummary {
                requested_count: selected_item_ids.len() as u64,
                admitted_count: admitted.len() as u64,
                rejected_count: rejections.len() as u64,
                rejections,
            };
            admission.validate()?;
            if admitted.is_empty() {
                return Ok(Sentinel2CreateArchiveBatchTaskResult {
                    snapshot: None,
                    admission,
                });
            }
            progress.committing()?;

            let derived_batch_id = format!("sentinel2-archive-batch-{}", request.task_id);
            if !valid_catalogue_opaque_id(&derived_batch_id) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "derived Sentinel-2 archive batch identifier is too long".to_owned(),
                });
            }
            let selected_item_ids = admitted
                .iter()
                .map(|item| item.item_id.clone())
                .collect::<Vec<_>>();
            let rejection_values = admission
                .rejections
                .iter()
                .map(|rejection| {
                    serde_json::json!({
                        "source_item_id": rejection.source_item_id,
                        "reason_code": rejection.reason_code,
                    })
                })
                .collect::<Vec<_>>();
            let derived_query = Map::from_iter([
                (
                    "logical_dataset".to_owned(),
                    Value::String("optical.sentinel-2.l2a".to_owned()),
                ),
                (
                    "provider_id".to_owned(),
                    Value::String(delivery_profile.provider_id.to_owned()),
                ),
                (
                    "native_collection_id".to_owned(),
                    Value::String(delivery_profile.native_collection_id.to_owned()),
                ),
                (
                    "derived_from_batch_id".to_owned(),
                    Value::String(request.source_batch_id.clone()),
                ),
                (
                    "delivery_kind".to_owned(),
                    Value::String("sentinel2_cdse_safe_archive_batch".to_owned()),
                ),
                (
                    "search_source_provider_id".to_owned(),
                    Value::String(source_batch.provider_id.clone()),
                ),
                (
                    "delivery_requested_count".to_owned(),
                    Value::Number(admission.requested_count.into()),
                ),
                (
                    "delivery_admitted_count".to_owned(),
                    Value::Number(admission.admitted_count.into()),
                ),
                (
                    "delivery_rejected_count".to_owned(),
                    Value::Number(admission.rejected_count.into()),
                ),
                (
                    "delivery_admission_rejections".to_owned(),
                    Value::Array(rejection_values),
                ),
            ]);
            validate_debug_context(&derived_query)
                .map_err(|_| ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;

            let mut derived_batch = source_batch.clone();
            derived_batch.batch_id = derived_batch_id.clone();
            derived_batch.provider_id = delivery_profile.provider_id.to_owned();
            derived_batch.created_at = request.created_at.clone();
            derived_batch.query = derived_query;
            derived_batch.search_stats = None;
            derived_batch.candidates = admitted;
            derived_batch
                .validate()
                .map_err(|_| ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;

            let batch_request = StoreSearchBatchRequest {
                batch: derived_batch,
                workbench: SearchWorkbenchState {
                    batch_id: derived_batch_id.clone(),
                    checked_item_ids: selected_item_ids.clone(),
                    page_index: 0,
                    page_size,
                    revision: 0,
                },
                activate_workbench: false,
            };
            let task_request = StoreCreateTaskRequest {
                task_id: request.task_id.clone(),
                batch_id: derived_batch_id,
                selected_item_ids: selected_item_ids.clone(),
                checked_item_ids: selected_item_ids,
                page_index: 0,
                page_size,
                output_root: request.output_root.clone(),
                layout: request.layout.clone(),
                concurrency: request.concurrency,
                created_at: request.created_at.clone(),
            };
            let snapshot = self.with_business_runtime(|runtime| {
                {
                    let mut store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    let current = store
                        .load_workbench()
                        .map_err(ApplicationServiceError::from)?
                        .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    if current.batch != source_batch
                        || current.state.revision != request.expected_workbench_revision
                    {
                        return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid);
                    }
                    if cancelled.load(Ordering::Acquire) {
                        return Err(ApplicationServiceError::Sentinel2ArchiveCancelled);
                    }
                    store
                        .save_search_batch_and_create_task(&batch_request, &task_request)
                        .map_err(ApplicationServiceError::from)?;
                }
                runtime
                    .scheduler
                    .as_ref()
                    .expect("business runtime guard established scheduler availability")
                    .wake();
                Self::snapshot_from_runtime(runtime)
            })?;
            Ok(Sentinel2CreateArchiveBatchTaskResult {
                snapshot: Some(snapshot),
                admission,
            })
        })();

        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    /// Create one executable CDSE SAFE archive task from a frozen Microsoft
    /// Sentinel-2 search-only scene without changing the active search Workbench.
    ///
    /// Microsoft remains search-only. The scene is re-resolved through the
    /// already-admitted CDSE catalogue using its frozen footprint, platform,
    /// acquisition time and exact SAFE name; only the resulting CDSE item gains
    /// the existing CDSE archive execution policy.
    pub fn create_sentinel2_archive_task(
        &self,
        request: &Sentinel2CreateArchiveTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.request_id)
            || request.request_id.len() > 80
            || !valid_catalogue_opaque_id(&request.source_batch_id)
            || !valid_exact_text(&request.item_id)
            || !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 76
            || request.output_root.as_os_str().is_empty()
            || !(1..=8).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Sentinel-2 archive task input is invalid".to_owned(),
            });
        }
        let dispatcher = self.dispatcher_handle()?;
        let route_use = dispatcher
            .catalogue_route_use()
            .map_err(ApplicationServiceError::Provider)?;
        if route_use != RouteUse::Production {
            return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid);
        }
        if matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            self.ensure_cdse_for_admission()?;
        }
        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or(ApplicationServiceError::Sentinel2ArchiveRequestDuplicate)?;
        let result = (|| {
            let (source_batch, page_size, source_item, source_query) =
                self.with_business_runtime(|runtime| {
                    let store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    let workbench = store
                        .load_workbench()
                        .map_err(ApplicationServiceError::from)?
                        .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    if workbench.batch.batch_id != request.source_batch_id {
                        return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid);
                    }
                    let source_item = workbench
                        .batch
                        .candidates
                        .iter()
                        .find(|candidate| candidate.item_id == request.item_id)
                        .cloned()
                        .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    let source_query = serde_json::from_value::<CatalogSearchQuery>(Value::Object(
                        workbench.batch.query.clone(),
                    ))
                    .map_err(|_| ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    let source_profile = CollectionProfileRegistry
                        .for_source(
                            &source_query.logical_dataset,
                            &source_query.provider_id,
                            &source_query.native_collection_id,
                        )
                        .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    let provider_product_id = source_item
                        .metadata
                        .get("provider_product_id")
                        .and_then(Value::as_str)
                        .filter(|value| valid_exact_text(value))
                        .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    let valid_secondary_source = (source_query.provider_id
                        == insar_provider::catalog::MPC_SENTINEL2_L2A_PROVIDER_ID
                        && source_query.native_collection_id
                            == insar_provider::catalog::MPC_SENTINEL2_L2A_COLLECTION_ID)
                        || (source_query.provider_id
                            == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
                            && source_query.native_collection_id
                                == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID);
                    if source_query.logical_dataset != "optical.sentinel-2.l2a"
                        || !valid_secondary_source
                        || workbench.batch.provider_id != source_query.provider_id
                        || source_item.provider_id != source_query.provider_id
                        || source_item.asset_access.is_some()
                        || source_item.execution_policy.is_some()
                        || source_item
                            .metadata
                            .get("collection_profile_id")
                            .and_then(Value::as_str)
                            != Some(source_profile.profile_id)
                        || source_item
                            .metadata
                            .get("resolver_policy_id")
                            .and_then(Value::as_str)
                            != Some(source_profile.resolver_policy_id)
                        || source_item.footprint_geojson.is_none()
                    {
                        return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid);
                    }
                    if source_query.provider_id
                        == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
                    {
                        insar_provider::catalog::validate_earth_search_sentinel2_source_id(
                            provider_product_id,
                            &source_item.display_name,
                            source_item.acquisition_datetime.as_deref(),
                        )
                        .map_err(|_| ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    } else {
                        insar_provider::catalog::validate_mpc_sentinel2_source_id(
                            provider_product_id,
                            &source_item.display_name,
                            source_item.acquisition_datetime.as_deref(),
                        )
                        .map_err(|_| ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    }
                    Ok((
                        workbench.batch,
                        workbench.state.page_size,
                        source_item,
                        source_query,
                    ))
                })?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::Sentinel2ArchiveCancelled);
            }

            let acquisition = source_item
                .acquisition_datetime
                .as_deref()
                .and_then(parse_normalized_utc)
                .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
            let window_delta = if source_query.provider_id
                == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
            {
                chrono::TimeDelta::minutes(30)
            } else {
                chrono::TimeDelta::seconds(2)
            };
            let start = acquisition
                .checked_sub_signed(window_delta)
                .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
            let end = acquisition
                .checked_add_signed(window_delta)
                .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
            let spacecraft = match source_item.display_name.get(..3) {
                Some("S2A") => "sentinel_2a",
                Some("S2B") => "sentinel_2b",
                Some("S2C") => "sentinel_2c",
                _ => return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid),
            };

            let delivery_profile = CollectionProfileRegistry
                .for_source(
                    "optical.sentinel-2.l2a",
                    "copernicus.dataspace.odata",
                    "SENTINEL-2",
                )
                .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
            let mut delivery_query = source_query;
            delivery_query.provider_id = delivery_profile.provider_id.to_owned();
            delivery_query.native_collection_id = delivery_profile.native_collection_id.to_owned();
            delivery_query.aoi_geojson = source_item.footprint_geojson.clone();
            delivery_query.cursor = None;
            delivery_query.result_limit = Some(100);
            delivery_query.filters = Map::from_iter([
                (
                    "product_type".to_owned(),
                    Value::String(delivery_profile.required_product_type.to_owned()),
                ),
                (
                    "spacecraft".to_owned(),
                    Value::Array(vec![Value::String(spacecraft.to_owned())]),
                ),
                (
                    "datetime".to_owned(),
                    serde_json::json!({
                        "start": start.to_rfc3339_opts(SecondsFormat::AutoSi, true),
                        "end": end.to_rfc3339_opts(SecondsFormat::AutoSi, true),
                    }),
                ),
            ]);
            delivery_query
                .validate()
                .map_err(|_| ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;

            let lookup_request_id = format!("{}-cdse", request.request_id);
            if !valid_catalogue_opaque_id(&lookup_request_id) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "Sentinel-2 archive lookup request identifier is too long".to_owned(),
                });
            }
            let registered = dispatcher
                .register_catalog_search_for_application(&lookup_request_id, &delivery_query)
                .map_err(ApplicationServiceError::Provider)?;
            let operation = registered
                .execute_with_parent_cancellation(&|| cancelled.load(Ordering::Acquire))
                .map_err(ApplicationServiceError::Provider)?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::Sentinel2ArchiveCancelled);
            }
            validate_complete_catalogue_response(
                &operation.response,
                &delivery_query,
                route_use,
                self.catalogue_live_acceptance_candidate,
            )?;
            let mut matches = operation
                .response
                .products
                .into_iter()
                .filter(|product| product.display_name == source_item.display_name)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(ApplicationServiceError::Sentinel2ArchiveUnavailable);
            }
            let delivery_item = map_sentinel2_catalogue_product(
                matches.pop().expect("one exact CDSE match is checked"),
                &delivery_query,
                route_use,
            )?;
            if delivery_item.display_name != source_item.display_name
                || delivery_item.provider_id != delivery_profile.provider_id
                || delivery_item.asset_access.is_none()
                || delivery_item.execution_policy.is_none()
            {
                return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid);
            }

            let derived_batch_id = format!("sentinel2-archive-{}", request.task_id);
            if !valid_catalogue_opaque_id(&derived_batch_id) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "derived Sentinel-2 archive batch identifier is too long".to_owned(),
                });
            }
            let selected_item_id = delivery_item.item_id.clone();
            let mut derived_query = serde_json::to_value(&delivery_query)
                .ok()
                .and_then(|value| value.as_object().cloned())
                .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
            derived_query.insert(
                "derived_from_batch_id".to_owned(),
                Value::String(request.source_batch_id.clone()),
            );
            derived_query.insert(
                "delivery_kind".to_owned(),
                Value::String("sentinel2_cdse_safe_archive".to_owned()),
            );
            derived_query.insert(
                "search_source_provider_id".to_owned(),
                Value::String(source_batch.provider_id.clone()),
            );
            validate_debug_context(&derived_query)
                .map_err(|_| ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
            let mut derived_batch = source_batch.clone();
            derived_batch.batch_id = derived_batch_id.clone();
            derived_batch.provider_id = delivery_profile.provider_id.to_owned();
            derived_batch.created_at = request.created_at.clone();
            derived_batch.query = derived_query;
            derived_batch.search_stats = None;
            derived_batch.candidates = vec![delivery_item];
            derived_batch
                .validate()
                .map_err(|_| ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
            let batch_request = StoreSearchBatchRequest {
                batch: derived_batch,
                workbench: SearchWorkbenchState {
                    batch_id: derived_batch_id.clone(),
                    checked_item_ids: vec![selected_item_id.clone()],
                    page_index: 0,
                    page_size,
                    revision: 0,
                },
                activate_workbench: false,
            };
            let task_request = StoreCreateTaskRequest {
                task_id: request.task_id.clone(),
                batch_id: derived_batch_id,
                selected_item_ids: vec![selected_item_id.clone()],
                checked_item_ids: vec![selected_item_id],
                page_index: 0,
                page_size,
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
                        .ok_or(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid)?;
                    if current.batch != source_batch {
                        return Err(ApplicationServiceError::Sentinel2ArchiveEvidenceInvalid);
                    }
                    store
                        .save_search_batch_and_create_task(&batch_request, &task_request)
                        .map_err(ApplicationServiceError::from)?;
                }
                runtime
                    .scheduler
                    .as_ref()
                    .expect("business runtime guard established scheduler availability")
                    .wake();
                Self::snapshot_from_runtime(runtime)
            })
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    /// Explicit compatibility-only full snapshot for the historical Landsat
    /// asset-set acceptance surface.
    pub fn create_landsat_asset_task_legacy(
        &self,
        request: &LandsatCreateBundleTaskRequest,
    ) -> Result<ReadonlyServiceSnapshot, ApplicationServiceError> {
        self.create_landsat_asset_task(request)?;
        self.snapshot(true)
    }

    /// Freeze exact TIFF/XML children for selected SLC-BURST products without
    /// creating a derived SearchBatch or replacing the active Workbench.
    ///
    /// The desktop bridge supplies explicit source product IDs; the Store owns
    /// their expansion into resolver-authoritative frozen asset items.
    pub fn create_burst_asset_task(
        &self,
        request: &BurstCreateAssetTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_burst_for_admission()?;
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.source_batch_id)
            || !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 96
            || request.selected_product_ids.is_empty()
            || request
                .selected_product_ids
                .iter()
                .any(|item_id| !valid_catalogue_opaque_id(item_id))
            || request
                .selected_product_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != request.selected_product_ids.len()
            || request.output_root.as_os_str().is_empty()
            || request.layout != DownloadLayout::Direct
            || !(1..=8).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Burst asset task input is invalid".to_owned(),
            });
        }

        self.with_business_runtime(|runtime| {
            {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let workbench = store
                    .load_workbench()
                    .map_err(ApplicationServiceError::from)?
                    .ok_or(ApplicationServiceError::BurstAssetEvidenceInvalid)?;
                if workbench.batch.batch_id != request.source_batch_id
                    || workbench.state.revision != request.expected_workbench_revision
                    || workbench.batch.provider_id != "asf.catalog"
                    || workbench
                        .batch
                        .query
                        .get("dataset")
                        .and_then(Value::as_str)
                        .is_none_or(|dataset| !dataset.eq_ignore_ascii_case("SLC-BURST"))
                {
                    return Err(ApplicationServiceError::BurstAssetEvidenceInvalid);
                }

                let requested = request
                    .selected_product_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>();
                let selected_products = workbench
                    .batch
                    .candidates
                    .iter()
                    .filter(|item| requested.contains(item.item_id.as_str()))
                    .collect::<Vec<_>>();
                if selected_products.len() != requested.len() {
                    return Err(ApplicationServiceError::BurstAssetEvidenceInvalid);
                }
                let source_item_ids = selected_products
                    .iter()
                    .map(|item| item.item_id.clone())
                    .collect::<Vec<_>>();
                let task_items = freeze_burst_task_items(&selected_products, request.asset_policy)?;
                let task_item_ids = task_items
                    .iter()
                    .map(|item| item.item_id.clone())
                    .collect::<Vec<_>>();
                let task_request = StoreCreateTaskRequest {
                    task_id: request.task_id.clone(),
                    batch_id: request.source_batch_id.clone(),
                    selected_item_ids: task_item_ids.clone(),
                    checked_item_ids: task_item_ids,
                    page_index: workbench.state.page_index,
                    page_size: workbench.state.page_size,
                    output_root: request.output_root.clone(),
                    layout: request.layout.clone(),
                    concurrency: request.concurrency,
                    created_at: request.created_at.clone(),
                };
                store
                    .create_task_with_explicit_items(
                        &task_request,
                        request.expected_workbench_revision,
                        &source_item_ids,
                        &task_items,
                    )
                    .map_err(ApplicationServiceError::from)?;
            }

            runtime
                .scheduler
                .as_ref()
                .expect("business runtime guard established scheduler availability")
                .wake();
            Self::snapshot_from_runtime(runtime)
        })
    }

    /// Expand new parent Burst products using the policy already frozen into an
    /// active explicit-item task. Callers cannot change policy during append.
    pub fn append_burst_asset_task(
        &self,
        request: &BurstAppendAssetTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.task_id)
            || !valid_catalogue_opaque_id(&request.event_id)
            || request.selected_product_ids.is_empty()
            || request
                .selected_product_ids
                .iter()
                .any(|item_id| !valid_catalogue_opaque_id(item_id))
            || request
                .selected_product_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != request.selected_product_ids.len()
            || parse_normalized_utc(&request.occurred_at).is_none()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Burst asset append input is invalid".to_owned(),
            });
        }
        let identity = request.receipt_identity();
        let read_receipt = || {
            self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let status = store
                    .burst_append_receipt_status(&identity)
                    .map_err(ApplicationServiceError::from)?;
                if status == BurstAppendReceiptStatus::Committed
                    && store
                        .load_task_record(&request.task_id)
                        .map_err(ApplicationServiceError::from)?
                        .is_none()
                {
                    return Err(ApplicationServiceError::from(StoreError::NotFound));
                }
                Ok(status)
            })
        };
        match read_receipt()? {
            BurstAppendReceiptStatus::Committed => {
                return self.with_business_runtime(Self::snapshot_from_runtime)
            }
            BurstAppendReceiptStatus::Conflict => {
                return Err(ApplicationServiceError::from(StoreError::AlreadyExists))
            }
            BurstAppendReceiptStatus::Unknown => {}
        }
        let mut store_attempted = false;
        let outcome = (|| {
            self.ensure_burst_for_admission()?;
            self.with_business_runtime(|runtime| {
                {
                    let mut store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    let task = store
                        .load_scheduler_task(&request.task_id)
                        .map_err(ApplicationServiceError::from)?
                        .ok_or(ApplicationServiceError::BurstAssetEvidenceInvalid)?;
                    if task.runtime.revision != request.expected_task_revision {
                        return Err(ApplicationServiceError::BurstAssetEvidenceInvalid);
                    }
                    let progressive_source = store
                        .progressive_task_source_batch_id(&request.task_id)
                        .map_err(ApplicationServiceError::from)?;
                    if progressive_source.is_some()
                        && request.selected_product_ids.len()
                            > insar_store::MAX_WORKBENCH_PAGE_SIZE as usize
                    {
                        return Err(ApplicationServiceError::BurstAssetEvidenceInvalid);
                    }
                    let source_products = if let Some(source_id) = progressive_source.as_deref() {
                        store
                            .load_batch_selection(source_id, &request.selected_product_ids)
                            .map_err(ApplicationServiceError::from)?
                            .ok_or(ApplicationServiceError::BurstAssetEvidenceInvalid)?
                            .items
                    } else {
                        store
                            .load_search_batch(&task.snapshot.batch_id)
                            .map_err(ApplicationServiceError::from)?
                            .ok_or(ApplicationServiceError::BurstAssetEvidenceInvalid)?
                            .candidates
                    };
                    let policy = frozen_burst_task_policy(&task.snapshot.candidates)?;
                    let existing_sources = task
                        .snapshot
                        .candidates
                        .iter()
                        .filter_map(|item| item.metadata.get("source_item_id"))
                        .filter_map(Value::as_str)
                        .collect::<BTreeSet<_>>();
                    let requested = request
                        .selected_product_ids
                        .iter()
                        .map(String::as_str)
                        .collect::<BTreeSet<_>>();
                    if !requested.is_disjoint(&existing_sources) {
                        return Err(ApplicationServiceError::BurstAssetEvidenceInvalid);
                    }
                    let selected_products = source_products
                        .iter()
                        .filter(|item| requested.contains(item.item_id.as_str()))
                        .collect::<Vec<_>>();
                    if selected_products.len() != requested.len() {
                        return Err(ApplicationServiceError::BurstAssetEvidenceInvalid);
                    }
                    let source_item_ids = selected_products
                        .iter()
                        .map(|item| item.item_id.clone())
                        .collect::<Vec<_>>();
                    let task_items = freeze_burst_task_items(&selected_products, policy)?;
                    let append = StoreAppendExplicitTaskItemsRequest {
                        task_id: request.task_id.clone(),
                        expected_revision: request.expected_task_revision,
                        event_id: request.event_id.clone(),
                        occurred_at: request.occurred_at.clone(),
                        source_item_ids,
                        task_items,
                    };
                    let parents = selected_products.into_iter().cloned().collect::<Vec<_>>();
                    store_attempted = true;
                    store
                        .append_burst_items_with_receipt(
                            &append,
                            progressive_source.as_ref().map(|_| parents.as_slice()),
                            &identity,
                        )
                        .map_err(ApplicationServiceError::from)?;
                }
                runtime
                    .scheduler
                    .as_ref()
                    .expect("business runtime guard established scheduler availability")
                    .wake();
                Self::snapshot_from_runtime(runtime)
            })
        })();
        if outcome.is_err()
            && !store_attempted
            && matches!(read_receipt(), Ok(BurstAppendReceiptStatus::Committed))
        {
            return self.with_business_runtime(Self::snapshot_from_runtime);
        }
        outcome
    }

    /// Create a new explicit-item task from a completed/cancelled/failed Burst
    /// task, inheriting its frozen asset policy and immutable source batch. The
    /// current Main Workbench is neither read nor mutated.
    pub fn create_historical_burst_asset_task(
        &self,
        request: &BurstCreateHistoricalAssetTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.source_task_id)
            || !valid_catalogue_opaque_id(&request.new_task_id)
            || request.source_task_id == request.new_task_id
            || request.selected_product_ids.is_empty()
            || request
                .selected_product_ids
                .iter()
                .any(|item_id| !valid_catalogue_opaque_id(item_id))
            || request
                .selected_product_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != request.selected_product_ids.len()
            || request.output_root.as_os_str().is_empty()
            || request.layout != DownloadLayout::Direct
            || !(1..=8).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "historical Burst asset task input is invalid".to_owned(),
            });
        }
        let identity = request.receipt_identity();
        let read_receipt = || {
            self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let status = store
                    .burst_successor_receipt_status(&identity)
                    .map_err(ApplicationServiceError::from)?;
                if status == BurstSuccessorReceiptStatus::Committed
                    && store
                        .load_task_record(&request.new_task_id)
                        .map_err(ApplicationServiceError::from)?
                        .is_none()
                {
                    return Err(ApplicationServiceError::from(StoreError::NotFound));
                }
                Ok(status)
            })
        };
        match read_receipt()? {
            BurstSuccessorReceiptStatus::Committed => {
                return self.with_business_runtime(Self::snapshot_from_runtime)
            }
            BurstSuccessorReceiptStatus::Conflict => {
                return Err(ApplicationServiceError::from(StoreError::AlreadyExists))
            }
            BurstSuccessorReceiptStatus::Unknown => {}
        }
        let mut store_attempted = false;
        let outcome = (|| {
            self.ensure_burst_for_admission()?;
            self.with_business_runtime(|runtime| {
                {
                    let mut store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    let source_task = store
                        .load_task_record(&request.source_task_id)
                        .map_err(ApplicationServiceError::from)?
                        .ok_or(ApplicationServiceError::BurstAssetEvidenceInvalid)?;
                    if source_task.runtime.revision != request.expected_source_task_revision
                        || !matches!(
                            source_task.runtime.status,
                            insar_core::TaskLifecycleStatus::Completed
                                | insar_core::TaskLifecycleStatus::Cancelled
                                | insar_core::TaskLifecycleStatus::Failed
                        )
                    {
                        return Err(ApplicationServiceError::BurstAssetEvidenceInvalid);
                    }
                    let policy = frozen_burst_task_policy(&source_task.snapshot.candidates)?;
                    let progressive_source = store
                        .progressive_task_source_batch_id(&request.source_task_id)
                        .map_err(ApplicationServiceError::from)?;
                    if progressive_source.is_some()
                        && request.selected_product_ids.len()
                            > insar_store::MAX_WORKBENCH_PAGE_SIZE as usize
                    {
                        return Err(ApplicationServiceError::BurstAssetEvidenceInvalid);
                    }
                    let source_products = if let Some(source_id) = progressive_source.as_deref() {
                        store
                            .load_batch_selection(source_id, &request.selected_product_ids)
                            .map_err(ApplicationServiceError::from)?
                            .ok_or(ApplicationServiceError::BurstAssetEvidenceInvalid)?
                            .items
                    } else {
                        store
                            .load_search_batch(&source_task.snapshot.batch_id)
                            .map_err(ApplicationServiceError::from)?
                            .ok_or(ApplicationServiceError::BurstAssetEvidenceInvalid)?
                            .candidates
                    };
                    let requested = request
                        .selected_product_ids
                        .iter()
                        .map(String::as_str)
                        .collect::<BTreeSet<_>>();
                    let selected_products = source_products
                        .iter()
                        .filter(|item| requested.contains(item.item_id.as_str()))
                        .collect::<Vec<_>>();
                    if selected_products.len() != requested.len() {
                        return Err(ApplicationServiceError::BurstAssetEvidenceInvalid);
                    }
                    let source_item_ids = selected_products
                        .iter()
                        .map(|item| item.item_id.clone())
                        .collect::<Vec<_>>();
                    let task_items = freeze_burst_task_items(&selected_products, policy)?;
                    let item_ids = task_items
                        .iter()
                        .map(|item| item.item_id.clone())
                        .collect::<Vec<_>>();
                    let task = StoreCreateTaskRequest {
                        task_id: request.new_task_id.clone(),
                        batch_id: source_task.snapshot.batch_id.clone(),
                        selected_item_ids: item_ids.clone(),
                        checked_item_ids: item_ids,
                        page_index: 0,
                        page_size: source_task.runtime.page_size,
                        output_root: request.output_root.clone(),
                        layout: request.layout.clone(),
                        concurrency: request.concurrency,
                        created_at: request.created_at.clone(),
                    };
                    let successor = StoreCreateHistoricalExplicitTaskRequest {
                        source_task_id: request.source_task_id.clone(),
                        expected_source_task_revision: request.expected_source_task_revision,
                        task,
                        source_item_ids,
                        task_items,
                    };
                    let parents = selected_products.into_iter().cloned().collect::<Vec<_>>();
                    store_attempted = true;
                    store
                        .create_historical_burst_task_with_receipt(
                            &successor,
                            progressive_source.as_ref().map(|_| parents.as_slice()),
                            &identity,
                        )
                        .map_err(ApplicationServiceError::from)?;
                }
                runtime
                    .scheduler
                    .as_ref()
                    .expect("business runtime guard established scheduler availability")
                    .wake();
                Self::snapshot_from_runtime(runtime)
            })
        })();
        if outcome.is_err()
            && !store_attempted
            && matches!(read_receipt(), Ok(BurstSuccessorReceiptStatus::Committed))
        {
            return self.with_business_runtime(Self::snapshot_from_runtime);
        }
        outcome
    }

    /// Search through the existing typed Dispatcher port, then atomically persist
    /// the mapped immutable SearchBatch and initial Workbench state. A failed or
    /// cancelled search never reaches the Store mutation.
    pub fn bind_region_aoi(
        &self,
        request: &ApplicationBindRegionAoiRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        request
            .validate()
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        self.ensure_running()?;
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
        self.mutate_business_store(|store| {
            let mut context = store.load_workspace_context()?.ok_or_else(|| {
                ApplicationServiceError::InvalidRequest {
                    reason: "workspace_open and region_select must precede AOI binding".to_owned(),
                }
            })?;
            if context.region_id.as_deref() != Some(request.region_id.as_str()) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "AOI region_id is not the active workspace region".to_owned(),
                });
            }
            let workspace_id = context.workspace_id.clone();
            store.save_region_aoi_for_workspace(
                &workspace_id,
                &request.region_id,
                request.expected_revision,
                &request.source_kind,
                &request.geometry,
                &request.bbox,
                &request.crs,
                &request.display_name,
                &request.bound_at,
                &now,
            )?;
            context.revision =
                context
                    .revision
                    .checked_add(1)
                    .ok_or(ApplicationServiceError::InvalidRequest {
                        reason: "workspace revision overflow".to_owned(),
                    })?;
            context.updated_at = now.clone();
            store.save_workspace_context(&context)?;
            Ok(())
        })
    }

    pub fn clear_region_aoi(
        &self,
        request: &ApplicationClearRegionAoiRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        request
            .validate()
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        let _aoi_guard = self
            .aoi_policy_gate
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        self.mutate_business_store(|store| {
            let context = store.load_workspace_context()?.ok_or_else(|| {
                ApplicationServiceError::InvalidRequest {
                    reason: "workspace_open and region_select must precede AOI clearing".to_owned(),
                }
            })?;
            if context.region_id.as_deref() != Some(request.region_id.as_str()) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "AOI region_id is not the active workspace region".to_owned(),
                });
            }
            store
                .clear_region_aoi_for_workspace(
                    &context.workspace_id,
                    &request.region_id,
                    request.expected_revision,
                )
                .map_err(ApplicationServiceError::from)
        })
    }

    pub fn sar_search_and_freeze(
        &self,
        request: &ApplicationSarSearchAndFreezeRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.sar_search_and_freeze_with_progress(request, |_| {})
    }

    pub fn sar_background_count_and_update(
        &self,
        request: &ApplicationSarBackgroundCountRequest,
    ) -> Result<ApplicationSarBackgroundCountResult, ApplicationServiceError> {
        request
            .validate()
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        let frozen = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let Some(workbench) = store
                .load_workbench()
                .map_err(ApplicationServiceError::from)?
            else {
                return Ok(None);
            };
            if workbench.batch.batch_id != request.batch_id
                || workbench.state.revision != request.expected_workbench_revision
            {
                return Ok(None);
            }
            let query: insar_core::SarSearchQuery = serde_json::from_value(Value::Object(
                workbench.batch.query.clone(),
            ))
            .map_err(|_| ApplicationServiceError::InvalidRequest {
                reason: "frozen SAR query is invalid".to_owned(),
            })?;
            if query.max_results.is_none() {
                return Ok(None);
            }
            let fingerprint =
                sar_search_cache_key(&query).map_err(ApplicationServiceError::Provider)?;
            Ok(Some((query, fingerprint)))
        })?;
        let Some((query, fingerprint)) = frozen else {
            return Ok(ApplicationSarBackgroundCountResult {
                applied: false,
                total_count: None,
                snapshot: None,
            });
        };
        let dispatcher = self.dispatcher_handle()?;
        let response = dispatcher
            .count_sar_for_application(&request.request_id, &SarSearchCountRequest { query })
            .map_err(ApplicationServiceError::Provider)?;
        let Some(total_count) = response.total_count else {
            return Ok(ApplicationSarBackgroundCountResult {
                applied: false,
                total_count: None,
                snapshot: None,
            });
        };
        self.ensure_running()?;
        let snapshot = self.with_business_runtime(|runtime| {
            let applied = {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                store
                    .update_active_sar_matched_count(
                        &request.batch_id,
                        request.expected_workbench_revision,
                        &fingerprint,
                        total_count,
                    )
                    .map_err(ApplicationServiceError::from)?
            };
            if applied {
                Self::snapshot_from_runtime(runtime).map(Some)
            } else {
                Ok(None)
            }
        })?;
        Ok(ApplicationSarBackgroundCountResult {
            applied: snapshot.is_some(),
            total_count: Some(total_count),
            snapshot,
        })
    }

    pub fn sar_search_and_freeze_with_progress<F>(
        &self,
        request: &ApplicationSarSearchAndFreezeRequest,
        progress: F,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError>
    where
        F: Fn(SarSearchProgressUpdate),
    {
        let started = Instant::now();
        let latest_progress = Mutex::new(None::<SarSearchProgressUpdate>);
        let publish = |update: SarSearchProgressUpdate| {
            let update = {
                let mut latest = latest_progress
                    .lock()
                    .expect("search progress projection lock is not re-entrant");
                let mut update = update.carry_forward(latest.as_ref());
                update.sequence = latest
                    .as_ref()
                    .filter(|previous| previous.request_id == update.request_id)
                    .map_or(1, |previous| {
                        previous
                            .sequence
                            .checked_add(1)
                            .expect("application search progress sequence overflow")
                    });
                update
                    .validate()
                    .expect("application search progress projection must validate");
                *latest = Some(update.clone());
                update
            };
            progress(update);
        };
        let result = self.sar_search_and_freeze_inner(request, &publish);
        match &result {
            Ok(_) => publish(
                SarSearchProgressUpdate::terminal(
                    &request.request_id,
                    SarSearchProgressStage::Completed,
                    "ASF 检索结果已冻结到工作台",
                )
                .with_elapsed(started),
            ),
            Err(error) if sar_search_error_is_cancelled(error) => {
                publish(
                    SarSearchProgressUpdate::live(
                        &request.request_id,
                        SarSearchProgressStage::Cancelling,
                        "正在取消 ASF 检索",
                    )
                    .with_elapsed(started),
                );
                publish(
                    SarSearchProgressUpdate::terminal(
                        &request.request_id,
                        SarSearchProgressStage::Cancelled,
                        "ASF 检索已取消",
                    )
                    .with_elapsed(started),
                );
            }
            Err(_) => publish(
                SarSearchProgressUpdate::terminal(
                    &request.request_id,
                    SarSearchProgressStage::Failed,
                    "ASF 检索失败",
                )
                .with_elapsed(started),
            ),
        }
        result
    }

    fn sar_search_and_freeze_inner<F>(
        &self,
        request: &ApplicationSarSearchAndFreezeRequest,
        progress: &F,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError>
    where
        F: Fn(SarSearchProgressUpdate),
    {
        progress(SarSearchProgressUpdate::live(
            &request.request_id,
            SarSearchProgressStage::Validating,
            "正在校验 AOI 与 ASF 筛选条件",
        ));
        let request = self.resolve_sar_request(request)?;
        request
            .validate()
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        hls_catalogue::reject_asf_removed_legacy_dataset(&request.search.query.dataset)?;

        progress(SarSearchProgressUpdate::live(
            &request.request_id,
            SarSearchProgressStage::Planning,
            "正在检查持久化 ASF 检索缓存",
        ));

        let cache_key = sar_search_cache_key(&request.search.query)
            .map_err(ApplicationServiceError::Provider)?;
        let cache_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ApplicationServiceError::InvalidRequest {
                reason: "system clock is before the Unix epoch".to_owned(),
            })?
            .as_secs();
        let query = serde_json::to_value(&request.search.query)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                reason: "SAR query could not be frozen".to_owned(),
            })?;
        let cache_decision = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store
                .load_sar_cache_decision(
                    &request.search.query,
                    cache_now,
                    SAR_SEARCH_CACHE_MAX_AGE_SECONDS,
                )
                .map_err(ApplicationServiceError::from)
        })?;
        let incremental = match cache_decision {
            StoreSarCacheDecision::Reuse { batch: cached } => {
                let mut candidates = cached.candidates;
                if let Some(target) = request.search.query.max_results {
                    let target = usize::try_from(target).unwrap_or(usize::MAX);
                    candidates.truncate(target);
                }
                let candidate_count = u64::try_from(candidates.len()).unwrap_or(u64::MAX);
                let mut search_stats = cached.search_stats;
                if let Some(stats) = &mut search_stats {
                    stats.frozen_candidate_count = candidate_count;
                    // A cached batch that was itself possibly incomplete stays
                    // so, but only a ceiling target can make it so (3(a)).
                    stats.truncated = stopped_at_safety_ceiling(request.search.query.max_results)
                        && (stats.truncated
                            || stats
                                .matched_count
                                .is_some_and(|matched| matched > candidate_count)
                            || stats.provider_returned_count > candidate_count);
                }
                let batch = FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: request.batch_id.clone(),
                    provider_id: cached.provider_id,
                    created_at: request.created_at.clone(),
                    aoi_name: request.aoi_name.clone(),
                    aoi_geojson: request.aoi_geojson.clone(),
                    query,
                    search_stats,
                    candidates,
                };
                let workbench = SearchWorkbenchState {
                    batch_id: batch.batch_id.clone(),
                    checked_item_ids: Vec::new(),
                    page_index: 0,
                    page_size: request.page_size,
                    revision: 0,
                };
                progress(SarSearchProgressUpdate::live(
                    &request.request_id,
                    SarSearchProgressStage::Freezing,
                    "正在将缓存结果冻结到工作台",
                ));
                return self.mutate_business_store(|store| {
                    store
                        .save_search_batch(&StoreSearchBatchRequest {
                            batch,
                            workbench,
                            activate_workbench: true,
                        })
                        .map_err(ApplicationServiceError::from)
                });
            }
            StoreSarCacheDecision::Incremental {
                source_batch,
                older_than,
                missing_results,
            } if source_batch.search_stats.is_some() => {
                Some((source_batch, older_than, missing_results))
            }
            StoreSarCacheDecision::Incremental { .. } | StoreSarCacheDecision::Miss => None,
        };

        let dispatcher = self.dispatcher_handle()?;
        let (provider_search, cached_extension) = match incremental {
            Some((source_batch, older_than, missing_results)) => {
                let mut provider_search = request.search.clone();
                provider_search.query.max_results = Some(missing_results);
                provider_search.query.end = Some(strictly_before_rfc3339(&older_than)?);
                (
                    provider_search,
                    Some((source_batch, older_than, missing_results)),
                )
            }
            None => (request.search.clone(), None),
        };
        progress(SarSearchProgressUpdate::live(
            &request.request_id,
            SarSearchProgressStage::WaitingProvider,
            "ASF 请求已提交，正在等待服务端响应",
        ));
        let finite_target = provider_search.query.max_results;
        let provider_progress =
            |provider: insar_provider::asf::search::SarSearchProviderProgress| {
                let mut update = match finite_target {
                    Some(target) => SarSearchProgressUpdate::measured(
                        &request.request_id,
                        SarSearchProgressStage::Fetching,
                        provider.unique_admitted_count.min(target),
                        target,
                        &format!(
                            "已载入 {} / {}",
                            provider.unique_admitted_count.min(target),
                            target
                        ),
                    ),
                    None => SarSearchProgressUpdate::counted(
                        &request.request_id,
                        SarSearchProgressStage::Fetching,
                        provider.unique_admitted_count,
                        &provider.remote_total.map_or_else(
                            || {
                                format!(
                                    "已完成 {} 次请求，获取原始记录 {} 条",
                                    provider.provider_request_count, provider.raw_fetched_count
                                )
                            },
                            |total_count| {
                                format!(
                                    "已纳入 {} / {}",
                                    provider.unique_admitted_count, total_count
                                )
                            },
                        ),
                    ),
                };
                update.stage = match provider.stage {
                    insar_provider::asf::search::SarSearchProviderProgressStage::AdmittingResults => {
                        SarSearchProgressStage::Filtering
                    }
                    _ => SarSearchProgressStage::Fetching,
                };
                update.provider_request_count = Some(provider.provider_request_count);
                update.raw_fetched_count = Some(provider.raw_fetched_count);
                update.mapped_count = Some(provider.mapped_count);
                update.exact_matched_count = Some(provider.unique_admitted_count);
                update.admitted_count = Some(
                    provider
                        .requested_limit
                        .map_or(provider.unique_admitted_count, |limit| {
                            provider.unique_admitted_count.min(limit)
                        }),
                );
                update.requested_limit = provider.requested_limit;
                update.remote_total = provider.remote_total;
                progress(update);
            };
        let response = dispatcher
            .search_sar_for_application_with_progress(
                &request.request_id,
                &provider_search,
                &provider_progress,
            )
            .map_err(ApplicationServiceError::Provider)?;
        self.ensure_running()?;
        let fetched_count = u64::try_from(response.products.len()).unwrap_or(u64::MAX);
        let mut normalization_progress = if let Some(target) = request.search.query.max_results {
            let loaded = fetched_count.min(target);
            let message = match response.stats.total_count {
                Some(remote_total) if loaded >= target => {
                    format!("候选总量 {remote_total} · 已载入 {loaded}")
                }
                Some(remote_total) => {
                    format!("候选总量 {remote_total} · 已载入 {loaded} / {target}")
                }
                None => format!("已载入 {loaded} / {target}"),
            };
            SarSearchProgressUpdate::measured(
                &request.request_id,
                SarSearchProgressStage::Filtering,
                loaded,
                target,
                &message,
            )
        } else if let Some(total_count) = response.stats.total_count {
            SarSearchProgressUpdate::counted(
                &request.request_id,
                SarSearchProgressStage::Filtering,
                fetched_count,
                &format!("ASF 已返回元数据：已获取 {fetched_count} / {total_count}"),
            )
        } else {
            SarSearchProgressUpdate::counted(
                &request.request_id,
                SarSearchProgressStage::Filtering,
                fetched_count,
                &format!("ASF 已返回元数据：已获取 {fetched_count}，候选总量未知"),
            )
        };
        normalization_progress.remote_total = response.stats.total_count;
        progress(normalization_progress);
        let provider_search_stats =
            normalize_legacy_asf_search_stats(&response.stats, response.products.len());
        let provider_warned_incomplete = response
            .stats
            .warnings
            .iter()
            .any(|warning| warning.code == ASF_SEARCH_POSSIBLY_INCOMPLETE);
        let provider_mapping_failed = !response.errors.is_empty()
            || response
                .stats
                .mapping_error_count
                .is_some_and(|count| count > 0);

        let provider_candidates = response
            .products
            .into_iter()
            .map(map_sar_product)
            .collect::<Result<Vec<_>, _>>()?;
        let (candidates, mut search_stats, provider_id) = match cached_extension {
            Some((source_batch, older_than, _)) => merge_incremental_sar_results(
                source_batch,
                provider_candidates,
                provider_search_stats,
                &older_than,
                request.search.query.max_results,
            )?,
            None => {
                let provider_id = sar_candidate_provider_id(&provider_candidates)?;
                (provider_candidates, provider_search_stats, provider_id)
            }
        };
        // Cache completeness keeps the provider's own truncation facts; the
        // frozen possibly-incomplete flag then follows the 3(a) rule: a
        // typed provider warning, or the safety ceiling stopped the search.
        let complete = !provider_mapping_failed
            && sar_batch_is_complete(
                Some(&search_stats),
                u64::try_from(candidates.len()).unwrap_or(u64::MAX),
            );
        search_stats.truncated = sar_frozen_possibly_incomplete(
            search_stats.truncated,
            provider_warned_incomplete,
            request.search.query.max_results,
        );
        let batch = FrozenSearchBatch {
            schema_version: 1,
            batch_id: request.batch_id.clone(),
            provider_id,
            created_at: request.created_at.clone(),
            aoi_name: request.aoi_name.clone(),
            aoi_geojson: request.aoi_geojson.clone(),
            query,
            search_stats: Some(search_stats),
            candidates,
        };
        let workbench = SearchWorkbenchState {
            batch_id: batch.batch_id.clone(),
            checked_item_ids: Vec::new(),
            page_index: 0,
            page_size: request.page_size,
            revision: 0,
        };
        progress(SarSearchProgressUpdate::live(
            &request.request_id,
            SarSearchProgressStage::Freezing,
            "正在冻结检索批次与工作台",
        ));
        self.mutate_business_store(|store| {
            store
                .save_search_batch_with_sar_cache(
                    &StoreSearchBatchRequest {
                        batch,
                        workbench,
                        activate_workbench: true,
                    },
                    &cache_key,
                    cache_now,
                    complete,
                )
                .map_err(ApplicationServiceError::from)
        })
    }

    pub(crate) fn sar_search_first_page_or_stage_for_continuation_with_options<F>(
        &self,
        request: &ApplicationSarSearchAndFreezeRequest,
        continuation_cap: u64,
        bypass_cache: bool,
        progress: &F,
    ) -> Result<unified_search::SarFirstPageOutcome, ApplicationServiceError>
    where
        F: Fn(SarSearchProgressUpdate),
    {
        progress(SarSearchProgressUpdate::live(
            &request.request_id,
            SarSearchProgressStage::Validating,
            "正在校验 AOI 与 ASF 筛选条件",
        ));
        let request = self.resolve_sar_request(request)?;
        request
            .validate()
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        hls_catalogue::reject_asf_removed_legacy_dataset(&request.search.query.dataset)?;

        progress(SarSearchProgressUpdate::live(
            &request.request_id,
            SarSearchProgressStage::Planning,
            "正在检查持久化 ASF 检索缓存",
        ));

        let cache_key = sar_search_cache_key(&request.search.query)
            .map_err(ApplicationServiceError::Provider)?;
        let cache_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ApplicationServiceError::InvalidRequest {
                reason: "system clock is before the Unix epoch".to_owned(),
            })?
            .as_secs();
        let query = serde_json::to_value(&request.search.query)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                reason: "SAR query could not be frozen".to_owned(),
            })?;

        let cache_decision = if bypass_cache {
            StoreSarCacheDecision::Miss
        } else {
            self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                store
                    .load_sar_cache_decision(
                        &request.search.query,
                        cache_now,
                        SAR_SEARCH_CACHE_MAX_AGE_SECONDS,
                    )
                    .map_err(ApplicationServiceError::from)
            })?
        };
        let incremental = match cache_decision {
            StoreSarCacheDecision::Reuse { batch: cached } => {
                let mut candidates = cached.candidates;
                if let Some(target) = request.search.query.max_results {
                    let target = usize::try_from(target).unwrap_or(usize::MAX);
                    candidates.truncate(target);
                }
                let candidate_count = u64::try_from(candidates.len()).unwrap_or(u64::MAX);
                let mut search_stats = cached.search_stats;
                if let Some(stats) = &mut search_stats {
                    stats.frozen_candidate_count = candidate_count;
                    stats.truncated = stopped_at_safety_ceiling(request.search.query.max_results)
                        && (stats.truncated
                            || stats
                                .matched_count
                                .is_some_and(|matched| matched > candidate_count)
                            || stats.provider_returned_count > candidate_count);
                }
                let batch = FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: request.batch_id.clone(),
                    provider_id: cached.provider_id,
                    created_at: request.created_at.clone(),
                    aoi_name: request.aoi_name.clone(),
                    aoi_geojson: request.aoi_geojson.clone(),
                    query,
                    search_stats,
                    candidates,
                };
                let workbench = SearchWorkbenchState {
                    batch_id: batch.batch_id.clone(),
                    checked_item_ids: Vec::new(),
                    page_index: 0,
                    page_size: request.page_size,
                    revision: 0,
                };
                progress(SarSearchProgressUpdate::live(
                    &request.request_id,
                    SarSearchProgressStage::Freezing,
                    "正在将缓存结果冻结到工作台",
                ));
                let snapshot = self.mutate_business_store(|store| {
                    store
                        .save_search_batch(&StoreSearchBatchRequest {
                            batch,
                            workbench,
                            activate_workbench: true,
                        })
                        .map_err(ApplicationServiceError::from)
                })?;
                return Ok(unified_search::SarFirstPageOutcome::Sealed(snapshot));
            }
            StoreSarCacheDecision::Incremental {
                source_batch,
                older_than,
                missing_results,
            } if source_batch.search_stats.is_some() => {
                Some((source_batch, older_than, missing_results))
            }
            StoreSarCacheDecision::Incremental { .. } | StoreSarCacheDecision::Miss => None,
        };

        let dispatcher = self.dispatcher_handle()?;
        let (provider_search, cached_extension) = match incremental {
            Some((source_batch, older_than, missing_results)) => {
                let mut provider_search = request.search.clone();
                provider_search.query.max_results = Some(missing_results);
                provider_search.query.end = Some(strictly_before_rfc3339(&older_than)?);
                (
                    provider_search,
                    Some((source_batch, older_than, missing_results)),
                )
            }
            None => (request.search.clone(), None),
        };
        progress(SarSearchProgressUpdate::live(
            &request.request_id,
            SarSearchProgressStage::WaitingProvider,
            "ASF 请求已提交，正在等待服务端响应",
        ));
        let finite_target = provider_search.query.max_results;
        let provider_progress =
            |provider: insar_provider::asf::search::SarSearchProviderProgress| {
                let mut update = match finite_target {
                    Some(target) => SarSearchProgressUpdate::measured(
                        &request.request_id,
                        SarSearchProgressStage::Fetching,
                        provider.unique_admitted_count.min(target),
                        target,
                        &format!(
                            "已载入 {} / {}",
                            provider.unique_admitted_count.min(target),
                            target
                        ),
                    ),
                    None => SarSearchProgressUpdate::counted(
                        &request.request_id,
                        SarSearchProgressStage::Fetching,
                        provider.unique_admitted_count,
                        &format!(
                            "已完成 {} 次请求，获取原始记录 {} 条",
                            provider.provider_request_count, provider.raw_fetched_count
                        ),
                    ),
                };
                update.stage = match provider.stage {
                    insar_provider::asf::search::SarSearchProviderProgressStage::AdmittingResults => {
                        SarSearchProgressStage::Filtering
                    }
                    _ => SarSearchProgressStage::Fetching,
                };
                update.provider_request_count = Some(provider.provider_request_count);
                update.raw_fetched_count = Some(provider.raw_fetched_count);
                update.mapped_count = Some(provider.mapped_count);
                update.exact_matched_count = Some(provider.unique_admitted_count);
                update.admitted_count = Some(
                    provider
                        .requested_limit
                        .map_or(provider.unique_admitted_count, |limit| {
                            provider.unique_admitted_count.min(limit)
                        }),
                );
                update.requested_limit = provider.requested_limit;
                update.remote_total = provider.remote_total;
                progress(update);
            };
        let response = dispatcher
            .search_sar_for_application_with_progress(
                &request.request_id,
                &provider_search,
                &provider_progress,
            )
            .map_err(ApplicationServiceError::Provider)?;
        self.ensure_running()?;
        let provider_search_stats =
            normalize_legacy_asf_search_stats(&response.stats, response.products.len());
        let provider_warned_incomplete = response
            .stats
            .warnings
            .iter()
            .any(|warning| warning.code == ASF_SEARCH_POSSIBLY_INCOMPLETE);
        let provider_mapping_failed = !response.errors.is_empty()
            || response
                .stats
                .mapping_error_count
                .is_some_and(|count| count > 0);

        let provider_candidates = response
            .products
            .into_iter()
            .map(map_sar_product)
            .collect::<Result<Vec<_>, _>>()?;
        let (candidates, mut search_stats, provider_id) = match cached_extension {
            Some((source_batch, older_than, _)) => merge_incremental_sar_results(
                source_batch,
                provider_candidates,
                provider_search_stats,
                &older_than,
                request.search.query.max_results,
            )?,
            None => {
                let provider_id = sar_candidate_provider_id(&provider_candidates)?;
                (provider_candidates, provider_search_stats, provider_id)
            }
        };

        let first_page_len = u64::try_from(candidates.len()).unwrap_or(u64::MAX);
        let remote_total = search_stats.matched_count;
        let oldest_dt = candidates
            .iter()
            .filter_map(|item| item.acquisition_datetime.as_deref())
            .min()
            .map(str::to_owned);
        let can_continue = continuation_cap > first_page_len
            && first_page_len >= insar_store::PROGRESSIVE_PUBLICATION_ROWS
            && (search_stats.truncated
                || provider_warned_incomplete
                || remote_total.is_some_and(|total| total > first_page_len))
            && oldest_dt
                .as_deref()
                .is_some_and(|dt| strictly_before_rfc3339(dt).is_ok());

        if !can_continue {
            let complete = !provider_mapping_failed
                && sar_batch_is_complete(Some(&search_stats), first_page_len);
            search_stats.truncated = sar_frozen_possibly_incomplete(
                search_stats.truncated,
                provider_warned_incomplete,
                request.search.query.max_results,
            );
            let batch = FrozenSearchBatch {
                schema_version: 1,
                batch_id: request.batch_id.clone(),
                provider_id,
                created_at: request.created_at.clone(),
                aoi_name: request.aoi_name.clone(),
                aoi_geojson: request.aoi_geojson.clone(),
                query,
                search_stats: Some(search_stats),
                candidates,
            };
            let workbench = SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: request.page_size,
                revision: 0,
            };
            let snapshot = self.mutate_business_store(|store| {
                store
                    .save_search_batch_with_sar_cache(
                        &StoreSearchBatchRequest {
                            batch,
                            workbench,
                            activate_workbench: true,
                        },
                        &cache_key,
                        cache_now,
                        complete,
                    )
                    .map_err(ApplicationServiceError::from)
            })?;
            return Ok(unified_search::SarFirstPageOutcome::Sealed(snapshot));
        }

        let batch_header = FrozenSearchBatch {
            schema_version: 1,
            batch_id: request.batch_id.clone(),
            provider_id: provider_id.clone(),
            created_at: request.created_at.clone(),
            aoi_name: request.aoi_name.clone(),
            aoi_geojson: request.aoi_geojson.clone(),
            query,
            search_stats: None,
            candidates: Vec::new(),
        };
        let workbench = SearchWorkbenchState {
            batch_id: request.batch_id.clone(),
            checked_item_ids: Vec::new(),
            page_index: 0,
            page_size: request.page_size,
            revision: 0,
        };
        let seen_ids: std::collections::HashSet<String> =
            candidates.iter().map(|c| c.item_id.clone()).collect();
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store.begin_search_batch_staging(&StoreSearchBatchStagingRequest {
                request_id: request.request_id.clone(),
                batch_header,
                workbench,
                max_candidates: continuation_cap,
            })?;
            store.append_search_batch_staging_candidates(&request.request_id, &candidates)?;
            store.publish_search_batch_staging_progressive(&request.request_id, remote_total)?;
            Ok(())
        })?;
        Ok(unified_search::SarFirstPageOutcome::StagedForContinuation(
            unified_search::SarContinuationPlan {
                request_id: request.request_id,
                batch_id: request.batch_id,
                created_at: request.created_at,
                base_query: request.search.query,
                loaded_count: first_page_len,
                remote_total,
                oldest_dt: oldest_dt.unwrap_or_default(),
                seen_ids,
                stats: search_stats,
                continuation_cap,
            },
        ))
    }

    pub(crate) fn fetch_next_sar_continuation_page(
        &self,
        request_id: &str,
        base_query: &insar_core::SarSearchQuery,
        older_than: &str,
        page_limit: u64,
        seen_ids: &mut std::collections::HashSet<String>,
    ) -> Result<SarContinuationPage, ApplicationServiceError> {
        let mut next_query = base_query.clone();
        next_query.max_results = Some(page_limit);
        next_query.end = Some(strictly_before_rfc3339(older_than)?);
        let dispatcher = self.dispatcher_handle()?;
        let response = dispatcher
            .search_sar_for_application_with_progress(
                request_id,
                &SarRemoteSearchRequest { query: next_query },
                &|_| {},
            )
            .map_err(ApplicationServiceError::Provider)?;
        self.ensure_running()?;
        let raw_len = response.products.len();
        let truncated = response.stats.truncated;
        let remote_total = response.stats.total_count;
        let mut mapped_new = Vec::with_capacity(raw_len);
        let mut next_oldest: Option<String> = None;
        for prod in response.products {
            let item = map_sar_product(prod)?;
            if let Some(dt) = &item.acquisition_datetime {
                if next_oldest.as_deref().is_none_or(|cur| dt.as_str() < cur) {
                    next_oldest = Some(dt.clone());
                }
            }
            if seen_ids.insert(item.item_id.clone()) {
                mapped_new.push(item);
            }
        }
        Ok((
            mapped_new,
            next_oldest,
            remote_total,
            truncated.unwrap_or(false) || raw_len as u64 >= page_limit,
        ))
    }

    pub(crate) fn append_sar_continuation_candidates(
        &self,
        request_id: &str,
        candidates: &[SnapshotItem],
    ) -> Result<(), ApplicationServiceError> {
        if candidates.is_empty() {
            return Ok(());
        }
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store
                .append_search_batch_staging_candidates(request_id, candidates)
                .map_err(ApplicationServiceError::from)
        })
    }

    pub(crate) fn seal_sar_continuation_staging(
        &self,
        request_id: &str,
        batch_created_at: &str,
        stats: FrozenSearchStats,
        terminal: insar_core::SearchBatchProgressState,
    ) -> Result<(), ApplicationServiceError> {
        self.mutate_business_store(|store| {
            let activate_slot = store
                .load_workbench()?
                .is_none_or(|wb| wb.batch.created_at.as_str() <= batch_created_at);
            store
                .seal_search_batch_staging_publication(
                    request_id,
                    Some(stats),
                    activate_slot,
                    terminal,
                )
                .map_err(ApplicationServiceError::from)
        })
        .map(|_| ())
    }

    /// Explicit compatibility-only full snapshot after the bounded SAR
    /// publication path. Normal Tauri startup and mutation commands must use
    /// `sar_search_and_freeze` and a bounded page instead.
    pub fn sar_search_and_freeze_legacy(
        &self,
        request: &ApplicationSarSearchAndFreezeRequest,
    ) -> Result<ReadonlyServiceSnapshot, ApplicationServiceError> {
        self.sar_search_and_freeze(request)?;
        self.snapshot(true)
    }

    /// Search through the closed catalogue route, then atomically freeze one
    /// complete candidate set. Fixture routes remain non-executable while the
    /// reviewed production route freezes resolver-authoritative archives.
    pub fn catalog_search_and_freeze(
        &self,
        request: &ApplicationCatalogSearchAndFreezeRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.catalog_search_and_freeze_with_progress(request, |_| {})
    }

    /// Issue #3 R2 B4: plan a route-neutral intent into the reviewed seed
    /// query, then run the unchanged catalogue path and its fallback plan.
    pub fn catalog_intent_search_and_freeze_with_progress<F>(
        &self,
        request: &ApplicationCatalogIntentSearchAndFreezeRequest,
        progress: F,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError>
    where
        F: Fn(SarSearchProgressUpdate),
    {
        self.catalog_intent_search_and_freeze_with_progress_and_options(request, false, progress)
    }

    pub fn catalog_intent_search_and_freeze_with_progress_and_options<F>(
        &self,
        request: &ApplicationCatalogIntentSearchAndFreezeRequest,
        bypass_cache: bool,
        progress: F,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError>
    where
        F: Fn(SarSearchProgressUpdate),
    {
        let planned = plan_catalogue_intent_request(request)?;
        self.catalog_search_and_freeze_with_progress_and_options(&planned, bypass_cache, progress)
    }

    pub fn catalog_search_and_freeze_with_progress<F>(
        &self,
        request: &ApplicationCatalogSearchAndFreezeRequest,
        progress: F,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError>
    where
        F: Fn(SarSearchProgressUpdate),
    {
        self.catalog_search_and_freeze_with_progress_and_options(request, false, progress)
    }

    pub fn catalog_search_and_freeze_with_progress_and_options<F>(
        &self,
        request: &ApplicationCatalogSearchAndFreezeRequest,
        bypass_cache: bool,
        progress: F,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError>
    where
        F: Fn(SarSearchProgressUpdate),
    {
        let started = Instant::now();
        let latest_progress = Mutex::new(None::<SarSearchProgressUpdate>);
        let publish = |update: SarSearchProgressUpdate| {
            let update = {
                let mut latest = latest_progress
                    .lock()
                    .expect("catalogue progress projection lock is not re-entrant");
                let mut update = update.carry_forward(latest.as_ref());
                update.sequence = latest
                    .as_ref()
                    .filter(|previous| previous.request_id == update.request_id)
                    .map_or(1, |previous| {
                        previous
                            .sequence
                            .checked_add(1)
                            .expect("application search progress sequence overflow")
                    });
                update
                    .validate()
                    .expect("application search progress projection must validate");
                *latest = Some(update.clone());
                update
            };
            progress(update);
        };
        publish(SarSearchProgressUpdate::live(
            &request.request_id,
            SarSearchProgressStage::Validating,
            "正在校验目录检索条件",
        ));
        let result = self.catalog_search_and_freeze_inner(request, bypass_cache, &publish, started);
        // #22 B slice 3(c): the terminal update of a progressively delivered
        // search names its batch and loaded count, including a cancelled or
        // failed one whose arrived rows were kept.
        let kept = self
            .search_batch_progress(&request.batch_id)
            .ok()
            .flatten()
            .filter(|progress| progress.request_id == request.request_id);
        let terminal = |stage, message: &str| {
            let mut update = SarSearchProgressUpdate::terminal(&request.request_id, stage, message)
                .with_elapsed(started);
            if let Some(kept) = &kept {
                update.batch_id = Some(kept.batch_id.clone());
                update.loaded_count = Some(kept.loaded_count);
                update.remote_total = update.remote_total.or(kept.remote_total);
            }
            update
        };
        match &result {
            Ok(_) => publish(terminal(
                SarSearchProgressStage::Completed,
                "目录检索结果已冻结到工作台",
            )),
            Err(error) if sar_search_error_is_cancelled(error) => {
                publish(
                    SarSearchProgressUpdate::live(
                        &request.request_id,
                        SarSearchProgressStage::Cancelling,
                        "正在取消目录检索",
                    )
                    .with_elapsed(started),
                );
                publish(terminal(
                    SarSearchProgressStage::Cancelled,
                    if kept.is_some() {
                        "目录检索已取消，已到结果已保留，可能不完整"
                    } else {
                        "目录检索已取消"
                    },
                ));
            }
            Err(_) => publish(terminal(
                SarSearchProgressStage::Failed,
                if kept.is_some() {
                    "目录检索中途失败，已到结果已保留，可能不完整"
                } else {
                    "目录检索失败"
                },
            )),
        }
        result
    }

    fn catalog_search_and_freeze_inner<F>(
        &self,
        request: &ApplicationCatalogSearchAndFreezeRequest,
        bypass_cache: bool,
        progress: &F,
        started: Instant,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError>
    where
        F: Fn(SarSearchProgressUpdate),
    {
        let mut request = self.resolve_catalogue_request(request)?;
        let fixture = self
            .offline_stac_fixture
            .as_deref()
            .filter(|a| a.targets(&request.search));
        catalogue_diagnostic(&request.request_id, "application_request_resolved");
        // Hold this gate until Dispatcher registration has returned an owned
        // catalogue lease. A concurrent policy replacement then either happens
        // first (and this uses the new Dispatcher) or sees the live lease and
        // fails closed without losing cancellation ownership.
        self.apply_pending_network_policy();
        let (registered, route_use) = {
            catalogue_diagnostic(&request.request_id, "catalogue_policy_gate_lock");
            let _catalogue_policy_gate = match self.catalogue_policy_gate.lock() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    catalogue_diagnostic(
                        &request.request_id,
                        "catalogue_policy_gate_poison_recovered",
                    );
                    self.catalogue_policy_gate.clear_poison();
                    poisoned.into_inner()
                }
            };
            catalogue_diagnostic(&request.request_id, "catalogue_policy_gate_acquired");
            let dispatcher = self.dispatcher_handle()?;
            catalogue_diagnostic(&request.request_id, "dispatcher_acquired");
            let route_use = dispatcher
                .catalogue_route_use()
                .map_err(ApplicationServiceError::Provider)?;
            let candidate_queries =
                self.catalogue_candidate_queries(&request.search, route_use, fixture.is_some())?;
            request.search = candidate_queries.first().cloned().ok_or_else(|| {
                catalogue_provider_error(
                    "CATALOGUE_ROUTE_UNAVAILABLE",
                    "没有可执行的数据来源",
                    "eligible_provider_candidates",
                )
            })?;
            request
                .validate_for_mode_with_fixture(
                    route_use,
                    self.catalogue_live_acceptance_candidate,
                    fixture,
                )
                .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
            let registered = dispatcher
                .register_catalog_search_candidates_for_application(
                    &request.request_id,
                    &candidate_queries,
                )
                .map_err(ApplicationServiceError::Provider)?;
            catalogue_diagnostic(&request.request_id, "catalogue_registration_returned");
            (registered, route_use)
        };
        progress(
            SarSearchProgressUpdate::live(
                &request.request_id,
                SarSearchProgressStage::Planning,
                "正在检查持久化目录检索缓存",
            )
            .with_elapsed(started),
        );
        let mut cache_key =
            catalog_search_cache_key(&request.search).map_err(ApplicationServiceError::Provider)?;
        let cache_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ApplicationServiceError::InvalidRequest {
                reason: "system clock is before the Unix epoch".to_owned(),
            })?
            .as_secs();
        let query = serde_json::to_value(&request.search)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                reason: "catalogue query could not be frozen".to_owned(),
            })?;
        // A persisted batch is not a persisted fixture grant. Always revalidate
        // test Provider output; explicit bypass_cache also skips cache lookup
        // (and writes fresh results back to SQLite on completion).
        let max_cache_age_seconds =
            catalog_search_cache_max_age_seconds(&request.search.provider_id);
        let cached = if fixture.is_some() || bypass_cache {
            None
        } else {
            self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                store
                    .load_catalog_cache_batch(&request.search, cache_now, max_cache_age_seconds)
                    .map_err(ApplicationServiceError::from)
            })?
        };
        if let Some(cached) = cached {
            let candidate_count = u64::try_from(cached.candidates.len()).unwrap_or(u64::MAX);
            let search_stats = reused_catalogue_search_stats(cached.search_stats);
            let batch = FrozenSearchBatch {
                schema_version: 1,
                batch_id: request.batch_id.clone(),
                provider_id: request.search.provider_id.clone(),
                created_at: request.created_at.clone(),
                aoi_name: request.aoi_name.clone(),
                aoi_geojson: request.aoi_geojson.clone(),
                query,
                search_stats,
                candidates: cached.candidates,
            };
            let workbench = SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: request.workbench_page_size,
                revision: 0,
            };
            let mut cache_progress = SarSearchProgressUpdate::live(
                &request.request_id,
                SarSearchProgressStage::Freezing,
                &format!("命中持久化目录缓存，正在冻结 {candidate_count} 条结果"),
            )
            .with_elapsed(started);
            cache_progress.provider_request_count = Some(0);
            progress(cache_progress);
            return self.mutate_business_store(|store| {
                store
                    .save_search_batch(&StoreSearchBatchRequest {
                        batch,
                        workbench,
                        activate_workbench: true,
                    })
                    .map_err(ApplicationServiceError::from)
            });
        }

        progress(
            SarSearchProgressUpdate::live(
                &request.request_id,
                SarSearchProgressStage::WaitingProvider,
                "目录请求已提交，正在等待服务端响应",
            )
            .with_elapsed(started),
        );
        // #22 B slice 3(b) review fix 4: the streaming entry keeps the exact
        // pre-streaming Fetching progress closure. Migrated clients report
        // their own progress while streaming; unmigrated ones forward to
        // search_catalog_with_progress through the trait default, so ASF and
        // Bhoonidhi progress overrides are no longer bypassed.
        // #22 B slice 3(c): progressive delivery state shared by the sink
        // (which stages and publishes) and the provider progress (which
        // learns the provider total and reports the loaded count).
        let progressive = ProgressiveDelivery::new(&request.batch_id);
        let provider_progress =
            |provider: insar_provider::asf::catalog_client::CatalogSearchProviderProgress| {
                progressive.observe_remote_total(provider.remote_total);
                let admitted = provider
                    .requested_limit
                    .map_or(provider.unique_admitted_count, |limit| {
                        provider.unique_admitted_count.min(limit)
                    });
                let message = provider.requested_limit.map_or_else(
                    || {
                        format!(
                            "已完成 {} 次请求，ALL 已纳入 {}",
                            provider.provider_request_count, admitted
                        )
                    },
                    |limit| {
                        format!(
                            "已完成 {} 次请求，纳入 {} / {}",
                            provider.provider_request_count, admitted, limit
                        )
                    },
                );
                let mut update = match provider.requested_limit {
                    Some(limit) => SarSearchProgressUpdate::measured(
                        &request.request_id,
                        SarSearchProgressStage::Fetching,
                        admitted,
                        limit,
                        &message,
                    ),
                    None => SarSearchProgressUpdate::live(
                        &request.request_id,
                        SarSearchProgressStage::Fetching,
                        &message,
                    ),
                };
                update.provider_request_count = Some(provider.provider_request_count);
                update.raw_fetched_count = Some(provider.raw_fetched_count);
                update.mapped_count = Some(provider.mapped_count);
                update.exact_matched_count = Some(provider.exact_matched_count);
                update.admitted_count = Some(admitted);
                update.requested_limit = provider.requested_limit;
                update.remote_total = provider.remote_total;
                update.elapsed_ms = provider.elapsed_ms;
                progressive.annotate(&mut update);
                progress(update);
            };
        // #22 B slice 3(b): stream the winning source into staging in
        // bounded chunks instead of accumulating the whole provider response.
        // The reviewed fallback/cancellation loop stays in the registered
        // search; the sink maps and stages each chunk as it arrives and
        // resets its staging when a fallback source starts.
        struct StagingStreamSink<'a> {
            service: &'a ApplicationService,
            request: &'a ApplicationCatalogSearchAndFreezeRequest,
            route_use: RouteUse,
            fixture: Option<&'a StacOfflineFixtureAdmission>,
            // The exact query of the source currently streaming; products
            // map with their own source's query, not the planned seed.
            attempt_query: CatalogSearchQuery,
            chunk: Vec<SnapshotItem>,
            mapped_count: u64,
            progressive: &'a ProgressiveDelivery,
        }

        impl StagingStreamSink<'_> {
            /// Stage one bounded chunk, then (#22 B slice 3(c)) make the
            /// search readable once its first full page has arrived and keep
            /// the provider total current while it is loading.
            fn flush(&mut self) -> insar_core::ProviderResult<()> {
                let chunk = std::mem::take(&mut self.chunk);
                self.record_flush(chunk.len())?;
                self.service
                    .append_catalogue_staging_candidates(&self.request.request_id, &chunk)
                    .map_err(application_to_provider_error)?;
                let staged = self.progressive.add_staged(chunk.len() as u64);
                if !self.progressive.is_published() {
                    if staged >= insar_store::PROGRESSIVE_PUBLICATION_ROWS {
                        let remote_total = self.progressive.remote_total();
                        self.service
                            .publish_catalogue_progressive(&self.request.request_id, remote_total)
                            .map_err(application_to_provider_error)?;
                        self.progressive.mark_published(remote_total);
                    }
                } else if let Some(remote_total) = self.progressive.unrecorded_remote_total() {
                    self.service
                        .record_catalogue_remote_total(&self.request.request_id, remote_total)
                        .map_err(application_to_provider_error)?;
                }
                Ok(())
            }

            fn record_flush(&mut self, size: usize) -> insar_core::ProviderResult<()> {
                // The service-level seam keeps the max across ALL fallback
                // attempts (attempt() does not reset it).
                let mut max = self
                    .service
                    .catalogue_streaming_max_chunk
                    .lock()
                    .map_err(|_| limit_overflow_error("staging_chunk_tracking"))?;
                if size > *max {
                    *max = size;
                }
                Ok(())
            }
        }

        impl CatalogSearchStreamSink for StagingStreamSink<'_> {
            fn accept(&mut self, products: &[ProviderProduct]) -> insar_core::ProviderResult<()> {
                for product in products {
                    let mapped = match self.fixture {
                        Some(admission) => {
                            stac_fixture::map_product(admission, product.clone(), self.route_use)
                        }
                        None => map_catalogue_product(
                            product.clone(),
                            &self.attempt_query,
                            self.route_use,
                        ),
                    }
                    .map_err(application_to_provider_error)?;
                    self.chunk.push(mapped);
                    self.mapped_count = self
                        .mapped_count
                        .checked_add(1)
                        .ok_or_else(|| limit_overflow_error("staging_count_overflow"))?;
                    if self.chunk.len() >= CATALOGUE_STREAMING_CHUNK {
                        self.flush()?;
                    }
                }
                // Flush the final partial chunk with the same bounded write.
                if !self.chunk.is_empty() {
                    self.flush()?;
                }
                Ok(())
            }

            fn attempt(&mut self, query: &CatalogSearchQuery) -> insar_core::ProviderResult<()> {
                // #22 B slice 3(c): rows the user can already page through are
                // never silently replaced by another attempt (a fallback
                // source or a whole-response retry); the search ends with
                // the arrived rows kept instead.
                if self.progressive.is_published() {
                    return Err(application_to_provider_error(catalogue_provider_error(
                        "CATALOGUE_PROGRESSIVE_RESTART_REFUSED",
                        "已显示的检索结果不能被另一次来源尝试替换",
                        "progressive_restart",
                    )));
                }
                // A prior failed source's staged rows must not survive into
                // the fallback source's batch. max_chunk is NOT reset: it is
                // the test seam for the largest chunk across ALL fallback
                // attempts of this search, not per attempt.
                self.chunk.clear();
                self.mapped_count = 0;
                self.progressive.reset_staged();
                self.attempt_query = query.clone();
                self.service
                    .abort_catalogue_staging(&self.request.request_id)
                    .map_err(application_to_provider_error)?;
                // The batch header freezes this attempt's exact source query.
                let query_json = serde_json::to_value(query)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                    .ok_or_else(|| invalid_query_error("query_freeze"))?;
                let batch_header = FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: self.request.batch_id.clone(),
                    provider_id: query.provider_id.clone(),
                    created_at: self.request.created_at.clone(),
                    aoi_name: self.request.aoi_name.clone(),
                    aoi_geojson: self.request.aoi_geojson.clone(),
                    query: query_json,
                    search_stats: None,
                    candidates: Vec::new(),
                };
                let workbench = SearchWorkbenchState {
                    batch_id: batch_header.batch_id.clone(),
                    checked_item_ids: Vec::new(),
                    page_index: 0,
                    page_size: self.request.workbench_page_size,
                    revision: 0,
                };
                self.service
                    .with_business_runtime(|runtime| {
                        let mut store = runtime
                            .store
                            .lock()
                            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                        store
                            .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
                                request_id: self.request.request_id.clone(),
                                batch_header,
                                workbench,
                                max_candidates: MAX_CATALOGUE_CANDIDATE_RECORDS,
                            })
                            .map_err(ApplicationServiceError::from)
                    })
                    .map_err(application_to_provider_error)?;
                Ok(())
            }
        }

        let mut sink = StagingStreamSink {
            service: self,
            request: &request,
            route_use,
            fixture,
            attempt_query: request.search.clone(),
            chunk: Vec::with_capacity(CATALOGUE_STREAMING_CHUNK),
            mapped_count: 0,
            progressive: &progressive,
        };
        // Once rows are readable, a failed source ends the search with them
        // kept instead of falling back to another source.
        let fallback_reason = |error: &insar_core::ContractError| {
            if progressive.is_published() {
                None
            } else {
                catalogue_provider_fallback_reason(error)
            }
        };
        let use_direct_cdse_public_odata = fixture.is_none()
            && matches!(
                self.business_factory.as_ref(),
                Some(BusinessRuntimeFactory::Production { .. })
            )
            && matches!(
                request.search.logical_dataset.as_str(),
                "optical.sentinel-2.l1c" | "optical.sentinel-3.standard"
            )
            && request.search.provider_id == "copernicus.dataspace.odata";
        struct DirectCdseCancellationGuard<'a> {
            registry: &'a CancellationRegistry,
            request_id: String,
        }
        impl Drop for DirectCdseCancellationGuard<'_> {
            fn drop(&mut self) {
                self.registry.finish(&self.request_id);
            }
        }
        let mut direct_cancel_guard = None;
        let mut direct_cancelled_flag = None;
        let (operation_outcome, operation_query, operation_source_attempts, operation_lease) =
            if use_direct_cdse_public_odata {
                drop(registered);
                let cancelled = self
                    .quicklook_cancellations
                    .try_register(&request.request_id)
                    .ok_or_else(|| {
                        catalogue_provider_error(
                            "CATALOGUE_REQUEST_DUPLICATE",
                            "同一目录请求不能重复提交",
                            "duplicate_request_id",
                        )
                    })?;
                direct_cancel_guard = Some(DirectCdseCancellationGuard {
                    registry: &self.quicklook_cancellations,
                    request_id: request.request_id.clone(),
                });
                let cancelled_for_pub = Arc::clone(&cancelled);
                direct_cancelled_flag = Some(cancelled_for_pub);
                let mut run_direct =
                    || -> insar_core::ProviderResult<CatalogSearchStreamOutcome> {
                        if cancelled.load(Ordering::Acquire) {
                            return Err(insar_core::ContractError::new(
                                "CATALOGUE_REQUEST_CANCELLED",
                                "目录检索已取消",
                                Map::from_iter([(
                                    "stage".to_owned(),
                                    Value::String("before_provider".to_owned()),
                                )]),
                            )
                            .expect("bounded cancellation context"));
                        }
                        let network_policy = self
                            .cdse_session_runtime()
                            .map_err(application_to_provider_error)?
                            .network_policy;
                        let client =
                        insar_provider::cdse::catalog_client::CdseODataCatalogClient::with_network_policy(
                            Duration::from_secs(45),
                            network_policy,
                        )?;
                        sink.attempt(&request.search)?;
                        let response = client
                            .search_all(&request.search, &|| cancelled.load(Ordering::Acquire))?;
                        if cancelled.load(Ordering::Acquire) {
                            return Err(insar_core::ContractError::new(
                                "CATALOGUE_REQUEST_CANCELLED",
                                "目录检索已取消",
                                Map::from_iter([(
                                    "stage".to_owned(),
                                    Value::String("after_provider".to_owned()),
                                )]),
                            )
                            .expect("bounded cancellation context"));
                        }
                        provider_progress(
                            insar_provider::asf::catalog_client::CatalogSearchProviderProgress {
                                provider_request_count: response.stats.request_count,
                                transport_part_count: response.stats.transport_part_count,
                                raw_fetched_count: response.stats.fetched_record_count,
                                mapped_count: response.stats.mapped_record_count,
                                exact_matched_count: response.stats.exact_matched_count,
                                unique_admitted_count: response.stats.returned_count,
                                requested_limit: response.stats.requested_limit,
                                remote_total: response.stats.matched_count,
                                elapsed_ms: response.stats.elapsed_ms,
                            },
                        );
                        sink.accept(&response.products)?;
                        Ok(CatalogSearchStreamOutcome::WholeResponse { response })
                    };
                match run_direct() {
                    Ok(outcome) => (
                        outcome,
                        request.search.clone(),
                        vec![CatalogSourceAttempt {
                            provider_id: request.search.provider_id.clone(),
                            native_collection_id: request.search.native_collection_id.clone(),
                            outcome: CatalogSourceAttemptOutcome::Succeeded,
                        }],
                        None,
                    ),
                    Err(error) => {
                        let error = ApplicationServiceError::Provider(error);
                        self.stop_catalogue_staging(&request.request_id, &progressive, &error);
                        return Err(error);
                    }
                }
            } else {
                match registered.execute_candidates_streaming(
                    &fallback_reason,
                    &mut sink,
                    &provider_progress,
                ) {
                    Ok(operation) => (
                        operation.outcome,
                        operation.query,
                        operation.source_attempts,
                        Some(operation.lease),
                    ),
                    Err(error) => {
                        let error = ApplicationServiceError::Provider(error);
                        // Cancel or provider failure before publication leaves no
                        // batch, rows or staging; after it the arrived rows are kept.
                        self.stop_catalogue_staging(&request.request_id, &progressive, &error);
                        return Err(error);
                    }
                }
            };
        let _keep_direct_cancel_guard = direct_cancel_guard;
        let streamed_mapped_count = sink.mapped_count;
        drop(sink);
        self.ensure_running()?;
        if operation_query != request.search {
            request.search = operation_query.clone();
            cache_key = catalog_search_cache_key(&request.search)
                .map_err(ApplicationServiceError::Provider)?;
        }

        // Both outcome kinds deliver their products through the sink in
        // bounded chunks (a whole-response provider streams its one chunk
        // through the trait's default method), so every product has already
        // been mapped into staging before this point. What remains here is
        // response-level validation, progress and the seal; any failure of
        // them aborts the staging so no batch, rows or residue survive.
        let stage_result = (|| {
            let (provider_stats, streamed_collection) = match operation_outcome {
                CatalogSearchStreamOutcome::Streamed { collection, stats } => {
                    (stats, Some(collection))
                }
                CatalogSearchStreamOutcome::WholeResponse { response } => {
                    if let Some(admission) = fixture {
                        admission
                            .validate_response(&request.search, &response, route_use)
                            .map_err(ApplicationServiceError::Provider)?;
                    } else {
                        validate_complete_catalogue_response(
                            &response,
                            &request.search,
                            route_use,
                            self.catalogue_live_acceptance_candidate,
                        )?;
                    }
                    (response.stats.clone(), None)
                }
            };
            if let Some(collection) = streamed_collection {
                validate_catalogue_streamed_stats(
                    &collection,
                    &provider_stats,
                    &request.search,
                    route_use,
                    self.catalogue_live_acceptance_candidate,
                    streamed_mapped_count,
                    fixture,
                )?;
            }
            if provider_stats.returned_count != streamed_mapped_count {
                return Err(catalogue_provider_error(
                    "CATALOGUE_RESPONSE_INCOMPLETE",
                    "目录映射计数与 provider 响应不一致",
                    "staging_mapped_count",
                ));
            }
            // The guard above proved streamed_mapped_count == returned_count.
            let candidate_count = streamed_mapped_count;
            let mut search_stats =
                normalize_catalogue_search_stats(&provider_stats, candidate_count as usize);
            if fixture.is_none() {
                search_stats.source_attempts =
                    freeze_catalogue_source_attempts(&operation_source_attempts);
            }
            self.unified_searches
                .record_catalogue_outcome(&request.request_id, &provider_stats);

            let mapped_count = candidate_count;
            let normalized_message = format!("目录元数据已映射 {mapped_count} 条");
            let mut normalized = match request.search.result_limit {
                Some(limit) => SarSearchProgressUpdate::measured(
                    &request.request_id,
                    SarSearchProgressStage::Filtering,
                    mapped_count,
                    limit,
                    &normalized_message,
                ),
                None => SarSearchProgressUpdate::live(
                    &request.request_id,
                    SarSearchProgressStage::Filtering,
                    &normalized_message,
                ),
            }
            .with_elapsed(started);
            normalized.mapped_count = Some(provider_stats.mapped_record_count);
            normalized.exact_matched_count = Some(provider_stats.exact_matched_count);
            normalized.admitted_count = Some(mapped_count);
            normalized.requested_limit = request.search.result_limit;
            normalized.remote_total = provider_stats.matched_count;
            normalized.provider_request_count = Some(provider_stats.request_count);
            normalized.raw_fetched_count = Some(provider_stats.fetched_record_count);
            progress(normalized);

            if let Some(hook) = &self.catalogue_before_publication {
                hook();
            }
            self.ensure_running()?;
            if let Some(lease) = &operation_lease {
                lease
                    .begin_publication()
                    .map_err(ApplicationServiceError::Provider)?;
            } else if direct_cancelled_flag
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Acquire))
            {
                return Err(catalogue_provider_error(
                    "CATALOGUE_REQUEST_CANCELLED",
                    "目录检索已取消",
                    "before_publication",
                ));
            }
            progress(
                SarSearchProgressUpdate::live(
                    &request.request_id,
                    SarSearchProgressStage::Freezing,
                    "正在冻结目录检索批次与工作台",
                )
                .with_elapsed(started),
            );
            if let Some(hook) = &self.catalogue_after_publication {
                hook();
            }
            self.mutate_business_store(|store| {
                store
                    .seal_catalog_search_batch_staging_with_cache_streaming(
                        &request.request_id,
                        Some(search_stats),
                        true,
                        &request.search,
                        &cache_key,
                        cache_now,
                    )
                    .map_err(ApplicationServiceError::from)
            })
        })();
        if let Err(error) = &stage_result {
            // #22 B slice 3(c): the provider already returned, so this is a
            // response-level integrity failure (or a cancel before the seal).
            // An untrustworthy response keeps the pre-3(c) fail-closed
            // answer — no batch, rows or staging — even after rows were
            // readable; only a cancel keeps the arrived rows.
            if sar_search_error_is_cancelled(error) {
                self.stop_catalogue_staging(&request.request_id, &progressive, error);
            } else {
                let _ = self.abort_catalogue_staging(&request.request_id);
            }
        }
        stage_result
    }

    /// Explicit compatibility-only full snapshot after the bounded catalogue
    /// publication path. This method is retained for rollback/reference tests;
    /// it is not registered as a normal Tauri mutation command.
    pub fn catalog_search_and_freeze_legacy(
        &self,
        request: &ApplicationCatalogSearchAndFreezeRequest,
    ) -> Result<ReadonlyServiceSnapshot, ApplicationServiceError> {
        self.catalog_search_and_freeze(request)?;
        self.snapshot(true)
    }

    /// Test-only: the largest single staging chunk the streaming catalogue
    /// sink has flushed; proves the in-memory candidate window stays bounded
    /// by the streaming chunk size.
    #[doc(hidden)]
    pub fn take_max_catalogue_streaming_chunk(&self) -> usize {
        self.catalogue_streaming_max_chunk
            .lock()
            .map(|mut max| std::mem::take(&mut *max))
            .unwrap_or(0)
    }

    /// Test-only: the largest single read batch the runtime store's streaming
    /// seal has used; proves the application seals streamed searches with the
    /// streaming seal (bounded read batches), not the classic whole-batch one.
    #[doc(hidden)]
    pub fn take_max_catalogue_seal_read_batch(&self) -> usize {
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            Ok(store.take_max_seal_read_batch())
        })
        .unwrap_or(0)
    }

    pub fn update_search_workbench(
        &self,
        request: &StoreUpdateWorkbenchRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.mutate_business_store(|store| {
            store
                .update_workbench_request(request)
                .map_err(ApplicationServiceError::from)
        })
    }

    pub fn create_download_task(
        &self,
        request: &StoreCreateTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_batch_items_for_admission(&request.batch_id, &request.selected_item_ids)?;
        self.mutate_business_store(|store| {
            hls_catalogue::reject_search_only_batch(store, &request.batch_id)?;
            store
                .create_task(request)
                .map(|_| ())
                .map_err(ApplicationServiceError::from)
        })
    }

    /// Explicit compatibility-only full snapshot for legacy task callers.
    pub fn create_download_task_legacy(
        &self,
        request: &StoreCreateTaskRequest,
    ) -> Result<ReadonlyServiceSnapshot, ApplicationServiceError> {
        self.create_download_task(request)?;
        self.snapshot(true)
    }

    pub fn append_download_task(
        &self,
        request: &StoreAppendTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_task_selection_for_admission(&request.task_id, &request.item_ids)?;
        self.mutate_business_store(|store| {
            hls_catalogue::reject_search_only_task(store, &request.task_id)?;
            store
                .append_task_items(request)
                .map(|_| ())
                .map_err(ApplicationServiceError::from)
        })
    }

    /// Create a task from the Store-owned Workbench selection. No candidate or
    /// checked-ID array crosses this boundary.
    pub fn create_task_from_workbench(
        &self,
        request: &CreateTaskFromWorkbenchRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if let Some(item_id) = request.exact_item_id.as_ref() {
            self.ensure_batch_items_for_admission(
                &request.batch_id,
                std::slice::from_ref(item_id),
            )?;
        } else {
            self.ensure_workbench_selection_for_admission(&request.batch_id)?;
        }
        self.mutate_business_store(|store| {
            hls_catalogue::reject_search_only_batch(store, &request.batch_id)?;
            store
                .create_task_from_workbench(request)
                .map(|_| ())
                .map_err(ApplicationServiceError::from)
        })
    }

    /// Append the Store-owned Workbench selection to an existing task. The
    /// expected revision makes stale UI state fail closed.
    pub fn append_task_from_workbench(
        &self,
        request: &AppendTaskFromWorkbenchRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.ensure_workbench_selection_for_admission(&request.batch_id)?;
        self.mutate_business_store(|store| {
            hls_catalogue::reject_search_only_batch(store, &request.batch_id)?;
            store
                .append_task_from_workbench(request)
                .map(|_| ())
                .map_err(ApplicationServiceError::from)
        })
    }

    pub fn append_task_from_source_batch(
        &self,
        request: &AppendTaskFromSourceBatchRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        let read_receipt = || {
            self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let status = store
                    .source_append_receipt_status(request)
                    .map_err(ApplicationServiceError::from)?;
                if matches!(
                    status,
                    SourceAppendReceiptStatus::Committed
                        | SourceAppendReceiptStatus::LegacyCommitted
                ) && store
                    .load_task_record(&request.task_id)
                    .map_err(ApplicationServiceError::from)?
                    .is_none()
                {
                    return Err(ApplicationServiceError::from(StoreError::NotFound));
                }
                Ok(status)
            })
        };
        match read_receipt()? {
            SourceAppendReceiptStatus::Committed | SourceAppendReceiptStatus::LegacyCommitted => {
                return self.with_business_runtime(Self::snapshot_from_runtime)
            }
            SourceAppendReceiptStatus::Conflict => {
                return Err(ApplicationServiceError::from(StoreError::AlreadyExists))
            }
            SourceAppendReceiptStatus::Unknown => {}
        }
        let mut store_attempted = false;
        let outcome = (|| {
            self.ensure_task_selection_for_admission(&request.task_id, &request.item_ids)?;
            self.mutate_business_store(|store| {
                hls_catalogue::reject_search_only_task(store, &request.task_id)?;
                store_attempted = true;
                store
                    .append_task_from_source_batch(request)
                    .map(|_| ())
                    .map_err(ApplicationServiceError::from)
            })
        })();
        if outcome.is_err()
            && !store_attempted
            && matches!(
                read_receipt(),
                Ok(SourceAppendReceiptStatus::Committed
                    | SourceAppendReceiptStatus::LegacyCommitted)
            )
        {
            return self.with_business_runtime(Self::snapshot_from_runtime);
        }
        outcome
    }

    pub fn create_task_from_historical_source_batch(
        &self,
        request: &CreateTaskFromHistoricalSourceBatchRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        let read_receipt = || {
            self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let status = store
                    .historical_successor_receipt_status(request)
                    .map_err(ApplicationServiceError::from)?;
                if status == HistoricalSuccessorReceiptStatus::Committed
                    && store
                        .load_task_record(&request.new_task_id)
                        .map_err(ApplicationServiceError::from)?
                        .is_none()
                {
                    return Err(ApplicationServiceError::from(StoreError::NotFound));
                }
                Ok(status)
            })
        };
        match read_receipt()? {
            HistoricalSuccessorReceiptStatus::Committed => {
                return self.with_business_runtime(Self::snapshot_from_runtime)
            }
            HistoricalSuccessorReceiptStatus::Conflict => {
                return Err(ApplicationServiceError::from(StoreError::AlreadyExists))
            }
            HistoricalSuccessorReceiptStatus::Unknown => {}
        }
        let mut store_attempted = false;
        let outcome = (|| {
            self.ensure_task_selection_for_admission(&request.source_task_id, &request.item_ids)?;
            self.mutate_business_store(|store| {
                hls_catalogue::reject_search_only_task(store, &request.source_task_id)?;
                store_attempted = true;
                store
                    .create_task_from_historical_source_batch(request)
                    .map(|_| ())
                    .map_err(ApplicationServiceError::from)
            })
        })();
        if outcome.is_err()
            && !store_attempted
            && matches!(
                read_receipt(),
                Ok(HistoricalSuccessorReceiptStatus::Committed)
            )
        {
            return self.with_business_runtime(Self::snapshot_from_runtime);
        }
        outcome
    }

    pub fn update_download_task(
        &self,
        request: &StoreUpdateTaskRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        let probe = insar_download::disk_guard::default_free_space_probe();
        self.update_download_task_with_free_space_probe(request, probe.as_ref())
    }

    pub(crate) fn update_download_task_with_free_space_probe(
        &self,
        request: &StoreUpdateTaskRequest,
        probe: &dyn insar_download::disk_guard::FreeSpaceProbe,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        let read_reacquire = || {
            self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                store
                    .reacquire_event_status(request)
                    .map_err(ApplicationServiceError::from)
            })
        };
        let reacquire = request.command.kind == TaskStateCommandKind::ReacquireItems;
        if reacquire {
            match read_reacquire()? {
                ReacquireEventStatus::Committed => {
                    return self.with_business_runtime(Self::snapshot_from_runtime)
                }
                ReacquireEventStatus::Conflict => {
                    return Err(ApplicationServiceError::from(StoreError::AlreadyExists))
                }
                ReacquireEventStatus::Unknown => {}
            }
        }
        if matches!(
            request.command.kind,
            TaskStateCommandKind::ResumeTask
                | TaskStateCommandKind::ResumeItems
                | TaskStateCommandKind::RetryTask
                | TaskStateCommandKind::RetryItems
                | TaskStateCommandKind::ReacquireItems
        ) {
            if matches!(
                request.command.kind,
                TaskStateCommandKind::ResumeTask | TaskStateCommandKind::ResumeItems
            ) {
                self.refuse_resume_on_low_disk(&request.task_id, probe)?;
            }
            if let Err(error) = self.ensure_task_session_for_admission(&request.task_id) {
                if reacquire && matches!(read_reacquire(), Ok(ReacquireEventStatus::Committed)) {
                    return self.with_business_runtime(Self::snapshot_from_runtime);
                }
                return Err(error);
            }
        }
        // Orbit scheduled-fetch S4 (review findings 1 and 5): retry and
        // reacquire commands go through the Store's atomic entry point,
        // which applies the command and clears the recheck rows of exactly
        // the revived items in one transaction — never the Waiting rows of
        // still-queued items, and never a retry committed with stale rows.
        let retry_command = matches!(
            request.command.kind,
            TaskStateCommandKind::RetryTask
                | TaskStateCommandKind::RetryItems
                | TaskStateCommandKind::ReacquireItems
        );
        self.mutate_business_store(|store| {
            let updated = if retry_command {
                store.update_task_and_clear_orbit_rechecks(request)?
            } else {
                store.update_task(request)?
            };
            if matches!(&request.command.kind, TaskStateCommandKind::ResumeTask)
                && !updated.runtime.paused_item_ids.is_empty()
            {
                let digest = Sha256::digest(request.event_id.as_bytes());
                let event_hash = format!("{digest:x}");
                store.update_task(&StoreUpdateTaskRequest {
                    task_id: request.task_id.clone(),
                    expected_revision: updated.runtime.revision,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::ResumeItems,
                        item_ids: updated.runtime.paused_item_ids.clone(),
                    },
                    event_id: format!(
                        "resume-items-{}-{}",
                        updated.runtime.revision,
                        &event_hash[..16]
                    ),
                    occurred_at: request.occurred_at.clone(),
                })?;
            }
            Ok(())
        })
    }

    pub fn update_download_task_view(
        &self,
        request: &StoreUpdateTaskViewRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.mutate_business_store(|store| {
            store
                .update_task_view(request)
                .map(|_| ())
                .map_err(ApplicationServiceError::from)
        })
    }

    pub fn archive_download_task(
        &self,
        request: &StoreArchiveTaskRequest,
    ) -> Result<ArchiveDownloadTaskAck, ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            let history = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?
                .archive_task(request)?;
            Ok(ArchiveDownloadTaskAck {
                task_id: history.task.snapshot.task_id,
                revision: history.task.runtime.revision,
                archived_at: history.archived_at,
            })
        })
    }

    pub fn delete_download_history(
        &self,
        request: &StoreDeleteHistoryRequest,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.mutate_business_store(|store| {
            store
                .delete_history(request)
                .map_err(ApplicationServiceError::from)
        })
    }

    pub fn download_snapshot(
        &self,
        task_id: &str,
    ) -> Result<DownloadEngineSnapshot, ApplicationServiceError> {
        if !valid_opaque_id(task_id) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "task_id must be a bounded opaque identifier".to_owned(),
            });
        }
        self.with_business_runtime(|runtime| {
            runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?
                .load_download_snapshot(task_id)
                .map_err(ApplicationServiceError::from)
        })
    }

    /// Stop accepting new operations, stop/join the one scheduler, and drop the
    /// one Store lease. It is intentionally idempotent.
    pub fn shutdown(&self) -> Result<(), ApplicationServiceError> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        self.shutting_down.store(true, Ordering::Release);
        runtime.take();
        let mut session = self
            .cdse_session
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let cdse_logout = if let Some(session) = session.take() {
            session
                .session
                .logout()
                .map_err(ApplicationServiceError::Cdse)
        } else {
            Ok(())
        };
        drop(session);
        let mut usgs = self
            .usgs_session
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let usgs_logout = if let Some(runtime) = usgs.take() {
            runtime
                .session
                .logout(&|| false)
                .map_err(ApplicationServiceError::UsgsM2m)
        } else {
            Ok(())
        };
        drop(usgs);
        // Revoke only this process-owned Bhoonidhi session. Taking the owner
        // first makes shutdown idempotent even if the bounded logout fails.
        let bhoonidhi = self
            .bhoonidhi_session
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .take();
        let bhoonidhi_logout = if let Some(runtime) = bhoonidhi {
            runtime.provider.logout().map_err(|error| {
                ApplicationServiceError::Provider(
                    ContractError::new(error.code(), error.to_string(), Map::new())
                        .expect("fixed Bhoonidhi authentication error"),
                )
            })
        } else {
            Ok(())
        };
        cdse_logout?;
        usgs_logout?;
        bhoonidhi_logout
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
    }

    fn capability_query_service(
        &self,
    ) -> Result<EffectiveCapabilityService<'static>, ApplicationServiceError> {
        self.ensure_running()?;
        let runtime_observations = self
            .runtime
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .as_ref()
            .map(|runtime| runtime.capability_observations.clone())
            .unwrap_or_default();
        let cdse_credential_ready = self
            .cdse_session
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .as_ref()
            .map(|runtime| runtime.status().map(|status| status.authenticated))
            .transpose()
            .map_err(ApplicationServiceError::Cdse)?;
        let mut observations = resource_capabilities::application_default_capability_observations();
        observations.merge(runtime_observations);
        if let Some(ready) = cdse_credential_ready {
            observations = observations.with_credential(
                "route.cdse.sentinel-2.l2a.v1",
                if ready {
                    insar_core::CredentialReadinessState::Ready
                } else {
                    insar_core::CredentialReadinessState::Missing
                },
            );
        }
        let earthdata_readiness = desktop_earthdata_readiness();
        let earthdata_state = if earthdata_readiness == EarthdataCredentialReadiness::Ready {
            insar_core::CredentialReadinessState::Ready
        } else {
            insar_core::CredentialReadinessState::Missing
        };
        for route in resource_capabilities::application_earthdata_route_ids() {
            observations = observations.with_credential(*route, earthdata_state);
        }
        let bhoonidhi_state =
            if desktop_bhoonidhi_readiness() == BhoonidhiCredentialReadiness::Ready {
                insar_core::CredentialReadinessState::Ready
            } else {
                insar_core::CredentialReadinessState::Missing
            };
        observations = observations.with_credential(
            "route.bhoonidhi.nisar-ssar-rslc.catalog.v1",
            bhoonidhi_state,
        );
        observations = resource_capabilities::with_opentopography_readiness(
            observations,
            &insar_download::desktop_opentopography_status(),
        );
        observations.merge(self.config.capability_observations.clone());
        if self.config.suppress_embedded_nisar_admission {
            observations = observations
                .without_admission(resource_capabilities::NISAR_SUBSET_ADMISSION_POLICY);
        }
        EffectiveCapabilityService::embedded(observations)
            .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable)
    }

    fn ensure_cdse_controls_available(&self) -> Result<(), ApplicationServiceError> {
        self.ensure_running()?;
        if !self.config.is_business() || self.business_factory.is_none() {
            return Err(ApplicationServiceError::BusinessRuntimeUnavailable);
        }
        Ok(())
    }

    fn ensure_usgs_controls_available(&self) -> Result<(), ApplicationServiceError> {
        self.ensure_running()?;
        if !self.config.is_business() || self.business_factory.is_none() {
            return Err(ApplicationServiceError::BusinessRuntimeUnavailable);
        }
        Ok(())
    }

    fn usgs_m2m_session_runtime(&self) -> Result<UsgsM2mSessionRuntime, ApplicationServiceError> {
        let mut runtime = self
            .usgs_session
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        if runtime.is_none() {
            *runtime = Some(
                UsgsM2mSessionRuntime::new(
                    self.network_policy(network_routes::ServiceRoute::Ordinary),
                )
                .map_err(ApplicationServiceError::UsgsM2m)?,
            );
        }
        Ok(runtime
            .as_ref()
            .expect("a USGS M2M runtime is initialized before it is borrowed")
            .clone())
    }

    fn landsat_browse_port(&self) -> Result<Arc<dyn LiveLandsatBrowse>, ApplicationServiceError> {
        match self.business_factory.as_ref() {
            Some(BusinessRuntimeFactory::Production { .. }) => {
                self.apply_pending_network_policy();
                let network_policy = self.cdse_session_runtime()?.network_policy;
                let port: Arc<dyn LiveLandsatBrowse> = Arc::new(
                    PlanetaryComputerLandsatClient::new(network_policy)
                        .map_err(ApplicationServiceError::PlanetaryComputer)?,
                );
                Ok(port)
            }
            Some(BusinessRuntimeFactory::Fixture {
                landsat_browse: Some(port),
                ..
            }) => Ok(Arc::clone(port)),
            _ => Err(ApplicationServiceError::BusinessRuntimeUnavailable),
        }
    }

    fn landsat_bundle_options_port(
        &self,
    ) -> Result<Arc<dyn LiveLandsatBundleOptions>, ApplicationServiceError> {
        match self.business_factory.as_ref() {
            Some(BusinessRuntimeFactory::Production { .. }) => {
                let runtime = self.usgs_m2m_session_runtime()?;
                let port: Arc<dyn LiveLandsatBundleOptions> = runtime.download_client;
                Ok(port)
            }
            Some(BusinessRuntimeFactory::Fixture {
                landsat_bundle_options: Some(port),
                ..
            }) => Ok(Arc::clone(port)),
            _ => Err(ApplicationServiceError::BusinessRuntimeUnavailable),
        }
    }

    fn landsat_asset_manifest_port(
        &self,
    ) -> Result<Arc<dyn LiveLandsatAssetManifest>, ApplicationServiceError> {
        match self.business_factory.as_ref() {
            Some(BusinessRuntimeFactory::Production { .. }) => {
                self.apply_pending_network_policy();
                let network_policy = self.cdse_session_runtime()?.network_policy;
                let client = PlanetaryComputerLandsatClient::new(network_policy)
                    .map_err(ApplicationServiceError::PlanetaryComputer)?;
                Ok(Arc::new(client))
            }
            Some(BusinessRuntimeFactory::Fixture {
                landsat_asset_manifest: Some(port),
                ..
            }) => Ok(Arc::clone(port)),
            _ => Err(ApplicationServiceError::BusinessRuntimeUnavailable),
        }
    }

    fn cdse_session_runtime(&self) -> Result<CdseSessionRuntime, ApplicationServiceError> {
        let mut session = self
            .cdse_session
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        Ok(self.cdse_session_runtime_locked(&mut session)?.clone())
    }

    fn cdse_session_runtime_locked<'a>(
        &self,
        session: &'a mut Option<CdseSessionRuntime>,
    ) -> Result<&'a CdseSessionRuntime, ApplicationServiceError> {
        if session.is_none() {
            *session = Some(
                CdseSessionRuntime::new(
                    self.network_policy(network_routes::ServiceRoute::Ordinary),
                )
                .map_err(ApplicationServiceError::Cdse)?,
            );
        }
        Ok(session
            .as_ref()
            .expect("a CDSE session is initialized before it is borrowed"))
    }

    pub fn bhoonidhi_authentication_check(&self) -> Result<Value, ApplicationServiceError> {
        self.ensure_running()?;
        let policy = self.network_policy(network_routes::ServiceRoute::Bhoonidhi);
        self.bhoonidhi_session_runtime(policy)?
            .provider
            .check_authentication()
            .map_err(|error| {
                ApplicationServiceError::Provider(
                    ContractError::new(error.code(), error.to_string(), Map::new())
                        .expect("fixed Bhoonidhi authentication error"),
                )
            })?;
        Ok(serde_json::json!({"ok": true, "session_available": true, "catalogue_verified": false}))
    }

    /// Read the process-local OAuth lifetime without loading saved secrets or
    /// contacting the Bhoonidhi service.
    pub fn bhoonidhi_session_status(
        &self,
    ) -> Result<BhoonidhiDesktopSessionStatus, ApplicationServiceError> {
        self.ensure_running()?;
        let session = self
            .bhoonidhi_session
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let Some(runtime) = session.as_ref() else {
            return Ok(BhoonidhiDesktopSessionStatus {
                authenticated: false,
                refresh_available: false,
                expires_in_seconds: None,
            });
        };
        runtime
            .session
            .snapshot()
            .map(bhoonidhi_desktop_status)
            .map_err(|error| {
                ApplicationServiceError::Provider(
                    ContractError::new(error.code(), error.to_string(), Map::new())
                        .expect("fixed Bhoonidhi session status error"),
                )
            })
    }

    fn bhoonidhi_session_runtime(
        &self,
        network_policy: ProviderNetworkPolicy,
    ) -> Result<BhoonidhiSessionRuntime, ApplicationServiceError> {
        let mut session = self
            .bhoonidhi_session
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        if session.is_none() {
            let runtime = BhoonidhiSessionRuntime::new(network_policy)?;
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            let runtime = if let Some(profile) = &self.config.auth_revoke_acceptance_profile {
                runtime.bind_acceptance_profile(profile)?
            } else {
                runtime
            };
            *session = Some(runtime);
        }
        Ok(session
            .as_ref()
            .expect("a Bhoonidhi session is initialized before it is borrowed")
            .clone())
    }

    fn ensure_formal_task_route(&self) -> Result<(), ApplicationServiceError> {
        self.ensure_running()?;
        if !self.config.is_business() || self.business_factory.is_none() {
            return Err(ApplicationServiceError::FormalTaskRouteUnavailable);
        }
        if self.dem_operation_port.is_none() {
            return Err(ApplicationServiceError::ProcessingEngineUnavailable {
                operation: "dem_processing",
            });
        }
        Ok(())
    }

    fn ensure_standalone_dem_route(&self) -> Result<(), ApplicationServiceError> {
        self.ensure_running()?;
        if !self.config.is_business() || self.business_factory.is_none() {
            return Err(ApplicationServiceError::FormalTaskRouteUnavailable);
        }
        if self.dem_operation_port.is_none() {
            return Err(ApplicationServiceError::ProcessingEngineUnavailable {
                operation: "dem_processing",
            });
        }
        Ok(())
    }

    fn dem_task_settings_path(&self) -> Result<PathBuf, ApplicationServiceError> {
        let store_path = self
            .config
            .store_path()
            .ok_or(ApplicationServiceError::StorePathRequired)?;
        Ok(store_path.with_file_name("dem-task-settings.json"))
    }

    fn gacos_ledger_path(&self) -> Result<PathBuf, ApplicationServiceError> {
        let store_path = self
            .config
            .store_path()
            .ok_or(ApplicationServiceError::StorePathRequired)?;
        Ok(store_path.with_file_name("gacos-ledger.json"))
    }

    fn network_settings_path(&self) -> Result<PathBuf, ApplicationServiceError> {
        let store_path = self
            .config
            .store_path()
            .ok_or(ApplicationServiceError::StorePathRequired)?;
        Ok(store_path.with_file_name("network-settings.json"))
    }

    fn sar_download_policy_settings_path(&self) -> Result<PathBuf, ApplicationServiceError> {
        let store_path = self
            .config
            .store_path()
            .ok_or(ApplicationServiceError::StorePathRequired)?;
        Ok(store_path.with_file_name("sar-download-policy-settings.json"))
    }

    fn ensure_formal_dem_capability(
        &self,
        resource_id: &str,
        capability_id: &str,
    ) -> Result<(), ApplicationServiceError> {
        let status = self
            .effective_capability(capability_id)?
            .ok_or(ApplicationServiceError::ResourceRegistryUnavailable)?;
        let accepted_kind = status.capability.kind == insar_core::CapabilityKind::Download
            || (resource_id == "dem.user-local.v1"
                && status.capability.kind == insar_core::CapabilityKind::Import);
        if status.resource_id != resource_id || !accepted_kind {
            return Err(ApplicationServiceError::CapabilityUnavailable {
                capability: "formal_dem_download",
            });
        }
        if status.projection.state == insar_core::EffectiveCapabilityState::BlockedCredential {
            return Err(ApplicationServiceError::CapabilityUnavailable {
                capability: "formal_dem_credential",
            });
        }
        if status.projection.state != insar_core::EffectiveCapabilityState::Executable {
            return Err(ApplicationServiceError::CapabilityUnavailable {
                capability: "formal_dem_download",
            });
        }
        Ok(())
    }

    pub(crate) fn ensure_formal_orbit_capability(&self) -> Result<(), ApplicationServiceError> {
        let status = self
            .effective_capability("capability.sentinel-1.poeorb.download.v1")?
            .ok_or(ApplicationServiceError::ResourceRegistryUnavailable)?;
        if status.resource_id != "orbit.sentinel-1.poeorb.v1"
            || status.capability.kind != insar_core::CapabilityKind::Download
            || status.projection.state != insar_core::EffectiveCapabilityState::Executable
        {
            return Err(ApplicationServiceError::CapabilityUnavailable {
                capability: "formal_orbit_download",
            });
        }
        Ok(())
    }

    fn region_aoi_for_task(
        &self,
        region_id: &str,
    ) -> Result<RegionAoiState, ApplicationServiceError> {
        if region_id.trim().is_empty() {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "formal task region_id is required".to_owned(),
            });
        }
        self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let context = store.load_workspace_context()?.ok_or_else(|| {
                ApplicationServiceError::InvalidRequest {
                    reason: "workspace_open must precede formal task creation".to_owned(),
                }
            })?;
            if context.region_id.as_deref() != Some(region_id) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: "formal task region_id is not the active workspace region".to_owned(),
                });
            }
            let aoi = store
                .load_region_aoi_for_workspace(&context.workspace_id, region_id)?
                .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                    reason: "formal task requires a persisted region AOI".to_owned(),
                })?;
            Ok(aoi)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn create_formal_task(
        &self,
        task_id: &str,
        batch_id: &str,
        provider_id: &str,
        display_name: &str,
        aoi: RegionAoiState,
        created_at: String,
        output_root: PathBuf,
        concurrency: u64,
        operation: TaskOperationDescriptor,
        orbit_association: Option<FrozenOrbitAssociationV1>,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        let item_id = if let Some(association) = orbit_association.as_ref() {
            association.orbit_transfer_item_id.clone()
        } else {
            format!("{task_id}:operation")
        };
        let mut item_payload = operation.payload.clone();
        item_payload.insert(
            "operation_id".to_owned(),
            Value::String(operation.operation_id.clone()),
        );
        item_payload.insert(
            "approved_worker_operation".to_owned(),
            Value::String(operation.approved_worker_operation.clone()),
        );
        let item = SnapshotItem {
            item_id: item_id.clone(),
            provider_id: provider_id.to_owned(),
            display_name: display_name.to_owned(),
            acquisition_datetime: None,
            footprint_geojson: Some(aoi.geometry.clone()),
            metadata: {
                let mut metadata = Map::new();
                metadata.insert("region_id".to_owned(), Value::String(aoi.region_id.clone()));
                metadata.insert(
                    "task_operation".to_owned(),
                    Value::String(operation.kind.clone()),
                );
                metadata
            },
            payload: item_payload,
            asset_access: None,
            execution_policy: None,
        };
        self.create_formal_task_with_items(
            task_id,
            batch_id,
            provider_id,
            aoi,
            created_at,
            output_root,
            concurrency,
            operation,
            orbit_association,
            vec![item],
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_formal_task_with_items(
        &self,
        task_id: &str,
        batch_id: &str,
        provider_id: &str,
        aoi: RegionAoiState,
        created_at: String,
        output_root: PathBuf,
        concurrency: u64,
        operation: TaskOperationDescriptor,
        orbit_association: Option<FrozenOrbitAssociationV1>,
        items: Vec<SnapshotItem>,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        if items.is_empty() || items.len() > 2 {
            return Err(invalid_request(
                "formal bounded capability task must contain one or two items",
            ));
        }
        let selected_item_ids = items
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>();
        let mut unique_item_ids = selected_item_ids.clone();
        unique_item_ids.sort();
        unique_item_ids.dedup();
        if unique_item_ids.len() != selected_item_ids.len()
            || items.iter().any(|item| {
                item.provider_id != provider_id
                    || item.footprint_geojson.as_ref() != Some(&aoi.geometry)
                    || item.payload.get("operation_id").and_then(Value::as_str)
                        != Some(operation.operation_id.as_str())
                    || item
                        .payload
                        .get("approved_worker_operation")
                        .and_then(Value::as_str)
                        != Some(operation.approved_worker_operation.as_str())
            })
        {
            return Err(invalid_request(
                "formal bounded capability task items do not match task authority",
            ));
        }
        let page_size = u64::try_from(items.len())
            .map_err(|_| invalid_request("formal task item count is invalid"))?;
        let dem_source_identity = dem_source_identity_for_persisted_task(&operation, &created_at)?;
        let batch = FrozenSearchBatch {
            schema_version: 1,
            batch_id: batch_id.to_owned(),
            provider_id: provider_id.to_owned(),
            created_at: created_at.clone(),
            aoi_name: aoi.display_name.clone(),
            aoi_geojson: Some(aoi.geometry.clone()),
            query: {
                let mut query = Map::new();
                query.insert(
                    "formal_task_operation".to_owned(),
                    serde_json::to_value(&operation).map_err(|_| {
                        ApplicationServiceError::InvalidRequest {
                            reason: "formal task operation cannot be serialized".to_owned(),
                        }
                    })?,
                );
                query
            },
            search_stats: None,
            candidates: items,
        };
        batch
            .validate()
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        let workbench = SearchWorkbenchState {
            batch_id: batch_id.to_owned(),
            checked_item_ids: selected_item_ids.clone(),
            page_index: 0,
            page_size,
            revision: 0,
        };
        let batch_request = StoreSearchBatchRequest {
            batch,
            workbench,
            activate_workbench: false,
        };
        let task_request = StoreCreateTaskRequest {
            task_id: task_id.to_owned(),
            batch_id: batch_id.to_owned(),
            selected_item_ids: selected_item_ids.clone(),
            checked_item_ids: selected_item_ids,
            page_index: 0,
            page_size,
            output_root,
            layout: if operation.kind == "orbit" {
                insar_core::DownloadLayout::SentinelOrbitAuxPoeorb
            } else {
                insar_core::DownloadLayout::SarDatasetProcessing
            },
            concurrency,
            created_at,
        };
        self.with_business_runtime(|runtime| {
            {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                store
                    .save_search_batch_and_create_task_with_projection_and_dem_identity(
                        &batch_request,
                        &task_request,
                        Some(&operation),
                        orbit_association.as_ref(),
                        dem_source_identity.as_ref(),
                    )
                    .map_err(ApplicationServiceError::from)?;
            }
            runtime
                .scheduler
                .as_ref()
                .ok_or(ApplicationServiceError::BusinessRuntimeUnavailable)?
                .wake();
            Self::snapshot_from_runtime(runtime)
        })
    }

    fn build_runtime(
        &self,
        store: SharedTaskStore,
    ) -> Result<ServiceRuntime, ApplicationServiceError> {
        let Some(factory) = self.business_factory.as_ref() else {
            return Ok(ServiceRuntime {
                store,
                dispatcher: None,
                scheduler: None,
                capability_observations: CapabilityRuntimeObservations::default(),
                earthdata_sessions: None,
            });
        };
        let mut earthdata_sessions: Option<Arc<dyn insar_download::EarthdataSessionOwner>> = None;
        let (dispatcher, worker): (Dispatcher, Arc<dyn DownloadWorker>) = match factory {
            BusinessRuntimeFactory::Production {
                #[cfg(feature = "nisar-subset-live-acceptance")]
                worker_override,
            } => {
                let cdse = self.cdse_session_runtime()?;
                let bhoonidhi = self.bhoonidhi_session_runtime(
                    self.network_policy(network_routes::ServiceRoute::Bhoonidhi),
                )?;
                let usgs = self.usgs_m2m_session_runtime()?;
                // A route change rebuilds this runtime, so GACOS result fetches
                // follow the saved ordinary route from here on.
                self.gacos_result_links
                    .set_network_route(cdse.network_policy.route());
                let landsat_catalogue: Arc<dyn LiveCatalogSearch> = Arc::new(
                    PlanetaryComputerLandsatClient::new(cdse.network_policy)
                        .map_err(ApplicationServiceError::PlanetaryComputer)?,
                );
                let bhoonidhi_catalogue: Arc<dyn LiveCatalogSearch> = Arc::new(
                    // Search and transfer share the Bhoonidhi session's route.
                    BhoonidhiCatalogClient::production_with_route(
                        Arc::clone(&bhoonidhi.provider),
                        bhoonidhi.session.network_policy().route(),
                    )
                    .map_err(|_| ApplicationServiceError::RuntimeInitialization)?,
                );
                let dispatcher =
                    Dispatcher::try_new_for_application_with_planetary_and_bhoonidhi_catalogue(
                        cdse.network_policy,
                        landsat_catalogue,
                        bhoonidhi_catalogue,
                    )
                    .map_err(|_| ApplicationServiceError::RuntimeInitialization)?;
                let build_worker = || {
                    ProductionDownloadWorker::new_with_cdse_and_bhoonidhi_provider(
                        TransferOptions::default(),
                        Arc::clone(&cdse.session),
                        cdse.network_policy,
                        Arc::clone(&bhoonidhi.session),
                        Arc::clone(&bhoonidhi.provider),
                        Duration::from_secs(45),
                        Arc::clone(&bhoonidhi.admission),
                    )
                    .and_then(|worker| worker.with_usgs_session(Arc::clone(&usgs.session)))
                    .map_err(|_| ApplicationServiceError::RuntimeInitialization)
                };
                let worker: Arc<dyn DownloadWorker> = {
                    #[cfg(feature = "nisar-subset-live-acceptance")]
                    if let Some(worker) = worker_override {
                        Arc::clone(worker)
                    } else {
                        let production = Arc::new(build_worker()?);
                        earthdata_sessions = Some(production.clone());
                        production
                    }
                    #[cfg(not(feature = "nisar-subset-live-acceptance"))]
                    {
                        let production = Arc::new(build_worker()?);
                        earthdata_sessions = Some(production.clone());
                        production
                    }
                };
                (dispatcher, worker)
            }
            BusinessRuntimeFactory::Fixture {
                search,
                catalog_search,
                catalogue_route_use,
                landsat_browse: _,
                landsat_bundle_options: _,
                landsat_asset_manifest: _,
                worker,
            } => {
                let dispatcher = match catalog_search {
                    #[cfg(feature = "usgs-m2m-live-acceptance")]
                    Some(catalog_search) if self.catalogue_live_acceptance_candidate => {
                        Dispatcher::with_live_acceptance_catalogue_for_application(
                            Arc::clone(search),
                            Arc::clone(catalog_search),
                        )
                    }
                    Some(catalog_search)
                        if *catalogue_route_use == Some(RouteUse::OfflineFixture)
                            && self.offline_stac_fixture.is_some() =>
                    {
                        Dispatcher::with_offline_stac_fixture_for_application(
                            Arc::clone(search),
                            Arc::clone(catalog_search),
                            Arc::clone(
                                self.offline_stac_fixture
                                    .as_ref()
                                    .expect("fixture admission checked"),
                            ),
                        )
                    }
                    Some(catalog_search) if *catalogue_route_use == Some(RouteUse::Production) => {
                        Dispatcher::with_production_catalogue_for_application(
                            Arc::clone(search),
                            Arc::clone(catalog_search),
                        )
                    }
                    Some(catalog_search) => Dispatcher::with_search_ports_for_application(
                        Arc::clone(search),
                        Arc::clone(catalog_search),
                    ),
                    None => Dispatcher::with_asf_search_for_application(Arc::clone(search)),
                };
                (dispatcher, Arc::clone(worker))
            }
        };
        let cdse_runtime = self.cdse_session_runtime().ok();
        let cdse_session_pair = cdse_runtime
            .as_ref()
            .map(|rt| (Arc::clone(&rt.session), rt.network_policy));
        let worker: Arc<dyn DownloadWorker> = match self.dem_operation_port.as_ref() {
            Some(port) => Arc::new(NativeDemDownloadWorker {
                base: worker,
                port: Arc::clone(port),
                store: Some(Arc::clone(&store)),
                completed_dem_windows: Mutex::new(BTreeMap::new()),
                transfer_activity: insar_download::TaskTransferActivityRegistry::default(),
                cdse_session: cdse_session_pair.clone(),
            }),
            None => worker,
        };
        let worker: Arc<dyn DownloadWorker> = Arc::new(Sentinel2L1cBandWorker {
            base: worker,
            cdse_session: cdse_runtime.as_ref().map(|rt| Arc::clone(&rt.session)),
            network_policy: cdse_runtime
                .as_ref()
                .map(|rt| rt.network_policy)
                .unwrap_or_else(|| self.network_policy(network_routes::ServiceRoute::Ordinary)),
            transfer_activity: insar_download::TaskTransferActivityRegistry::default(),
        });
        let worker: Arc<dyn DownloadWorker> =
            Arc::new(gacos_import_worker::NativeGacosImportWorker {
                base: worker,
                port: self.dem_operation_port.clone(),
            });
        let worker: Arc<dyn DownloadWorker> =
            Arc::new(gacos_result_worker::NativeGacosResultWorker {
                base: worker,
                port: self.dem_operation_port.clone(),
                links: Arc::clone(&self.gacos_result_links),
            });
        let mut capability_observations = CapabilityRuntimeObservations::default();
        for (binding, state) in dispatcher.capability_runtime_observations() {
            capability_observations = capability_observations.with_runtime(binding, state);
        }
        for (binding, state) in worker.capability_runtime_observations() {
            capability_observations = capability_observations.with_runtime(binding, state);
        }
        let dem_settings = self.get_dem_task_settings()?;
        let scheduler = DownloadScheduler::start_with_store_and_dem_task_limits(
            Arc::clone(&store),
            worker,
            SCHEDULER_POLL_INTERVAL,
            insar_download::DemTaskLimits::new(
                dem_settings.max_concurrent_tasks,
                dem_settings.max_concurrent_conversions,
            )
            .expect("validated persisted DEM settings"),
        );
        Ok(ServiceRuntime {
            store,
            dispatcher: Some(Arc::new(dispatcher)),
            scheduler: Some(scheduler),
            capability_observations,
            earthdata_sessions,
        })
    }

    fn store_handle(&self) -> Result<SharedTaskStore, ApplicationServiceError> {
        self.startup()?;
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        runtime
            .as_ref()
            .map(|runtime| Arc::clone(&runtime.store))
            .ok_or(ApplicationServiceError::StoreNotReady)
    }

    /// Run a bounded business operation while holding the runtime lifecycle
    /// lock. Shutdown acquires the same lock before removing the scheduler and
    /// Store lease, so a mutation either completes with its authoritative Store
    /// projection or is rejected before it can obtain the Store writer.
    fn with_business_runtime<T>(
        &self,
        operation: impl FnOnce(&ServiceRuntime) -> Result<T, ApplicationServiceError>,
    ) -> Result<T, ApplicationServiceError> {
        self.ensure_running()?;
        if !self.config.is_business() || self.business_factory.is_none() {
            return Err(ApplicationServiceError::BusinessRuntimeUnavailable);
        }
        self.apply_pending_network_policy();
        self.startup()?;
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        self.ensure_running()?;
        let runtime = runtime
            .as_ref()
            .ok_or(ApplicationServiceError::BusinessRuntimeUnavailable)?;
        if runtime.dispatcher.is_none() || runtime.scheduler.is_none() {
            return Err(ApplicationServiceError::BusinessRuntimeUnavailable);
        }
        operation(runtime)
    }

    fn dispatcher_handle(&self) -> Result<Arc<Dispatcher>, ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            runtime
                .dispatcher
                .as_ref()
                .cloned()
                .ok_or(ApplicationServiceError::BusinessRuntimeUnavailable)
        })
    }

    fn mutate_workspace_context(
        &self,
        request_id: &str,
        expected_revision: u64,
        operation: impl FnOnce(&mut WorkspaceContextState) -> Result<(), ApplicationServiceError>,
    ) -> Result<WorkspaceContextState, ApplicationServiceError> {
        if request_id.trim().is_empty() {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "workspace operation request_id is required".to_owned(),
            });
        }
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let mut context = store.load_workspace_context()?.ok_or_else(|| {
                ApplicationServiceError::InvalidRequest {
                    reason: "workspace_open must precede project or region operations".to_owned(),
                }
            })?;
            ensure_workspace_revision(expected_revision, context.revision)?;
            operation(&mut context)?;
            context.revision = context.revision.checked_add(1).ok_or_else(|| {
                ApplicationServiceError::InvalidRequest {
                    reason: "workspace revision overflow".to_owned(),
                }
            })?;
            context.updated_at =
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
            sync_workspace_regions(&mut context);
            store.save_workspace_context(&context)?;
            Ok(context)
        })
    }

    fn append_catalogue_staging_candidates(
        &self,
        request_id: &str,
        candidates: &[SnapshotItem],
    ) -> Result<(), ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            match store.append_search_batch_staging_candidates(request_id, candidates) {
                Ok(()) => Ok(()),
                Err(StoreError::AlreadyExists) => Err(catalogue_provider_error(
                    "CATALOGUE_PRODUCT_INVALID",
                    "目录产品身份重复",
                    "duplicate_candidate_identity",
                )),
                Err(error) => Err(ApplicationServiceError::from(error)),
            }
        })
    }

    /// #22 B slice 3(c): make a staging request readable as a loading batch.
    fn publish_catalogue_progressive(
        &self,
        request_id: &str,
        remote_total: Option<u64>,
    ) -> Result<(), ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store
                .publish_search_batch_staging_progressive(request_id, remote_total)
                .map(|_| ())
                .map_err(ApplicationServiceError::from)
        })
    }

    fn record_catalogue_remote_total(
        &self,
        request_id: &str,
        remote_total: u64,
    ) -> Result<(), ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store
                .record_search_batch_remote_total(request_id, Some(remote_total))
                .map(|_| ())
                .map_err(ApplicationServiceError::from)
        })
    }

    /// Seal a loading request early, keeping its arrived rows under truthful
    /// possibly-incomplete statistics. Returns the kept batch id, or `None`
    /// when the request was not loading (already sealed or never published).
    fn finalize_loading_catalogue(
        &self,
        request_id: &str,
        state: SearchBatchProgressState,
    ) -> Result<Option<String>, ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let Some(loading) = store.loading_search_batch_for_request(request_id)? else {
                return Ok(None);
            };
            let stats = insar_store::partial_search_stats(
                loading.loaded_count,
                loading.remote_total,
                "TYPED_CATALOG",
            );
            Ok(store
                .finalize_search_batch_staging(request_id, stats, state)?
                .then_some(loading.batch_id))
        })
    }

    /// The end of a search whose fetch was cancelled or failed. Before
    /// progressive publication this is the pre-3(c) abort (no batch, rows or
    /// staging); after it the arrived rows are kept as a cancelled or failed
    /// batch. If
    /// that seal itself fails the request is aborted, so nothing half-written
    /// stays readable; if the runtime is already gone the loading state stays
    /// and the next start marks it interrupted.
    fn stop_catalogue_staging(
        &self,
        request_id: &str,
        progressive: &ProgressiveDelivery,
        error: &ApplicationServiceError,
    ) {
        if !progressive.is_published() {
            let _ = self.abort_catalogue_staging(request_id);
            return;
        }
        let state = if sar_search_error_is_cancelled(error) {
            SearchBatchProgressState::Cancelled
        } else {
            SearchBatchProgressState::Failed
        };
        if let Err(error) = self.finalize_loading_catalogue(request_id, state) {
            if !matches!(error, ApplicationServiceError::BusinessRuntimeUnavailable) {
                let _ = self.abort_catalogue_staging(request_id);
            }
        }
    }

    pub(crate) fn abort_catalogue_staging(
        &self,
        request_id: &str,
    ) -> Result<(), ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store
                .abort_search_batch_staging(request_id)
                .map_err(ApplicationServiceError::from)
        })
    }

    fn mutate_business_store(
        &self,
        operation: impl FnOnce(&mut TaskStore) -> Result<(), ApplicationServiceError>,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                operation(&mut store)?;
            }
            runtime
                .scheduler
                .as_ref()
                .expect("business runtime guard established scheduler availability")
                .wake();
            Self::snapshot_from_runtime(runtime)
        })
    }

    fn snapshot_from_runtime(
        runtime: &ServiceRuntime,
    ) -> Result<CoreServiceSnapshot, ApplicationServiceError> {
        let store = runtime
            .store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let readiness = ServiceReadiness::store_ready(&store.health()?);
        let core = store.core_restore(true)?;
        Ok(CoreServiceSnapshot::from_parts(readiness, core))
    }

    fn ensure_running(&self) -> Result<(), ApplicationServiceError> {
        if self.is_shutting_down() {
            Err(ApplicationServiceError::ShuttingDown)
        } else {
            Ok(())
        }
    }
}

fn sar_search_error_is_cancelled(error: &ApplicationServiceError) -> bool {
    error.code().contains("CANCEL")
        || matches!(
            error,
            ApplicationServiceError::Provider(source)
                if matches!(source.code.as_str(), "DL001" | "CATALOGUE_REQUEST_CANCELLED")
        )
}

/// B15: the file name the worker would plan for one frozen item, used only to
/// locate the item's target directory for conflict enumeration. Mirrors the
/// production worker's frozen-file-name precedence: the frozen asset access
/// name, then the legacy payload keys, then the display name.
fn planned_item_file_name(work: &DownloadWork) -> Option<String> {
    if let Some(access) = work.item.asset_access.as_ref() {
        if let Some(file_name) = access.file_name.as_deref() {
            if !file_name.trim().is_empty() {
                return Some(file_name.to_owned());
            }
        }
    }
    for key in ["expected_filename", "file_name"] {
        if let Some(value) = work.item.payload.get(key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    let lower_display = work.item.display_name.to_ascii_lowercase();
    if lower_display.ends_with(".zip")
        || lower_display.ends_with(".h5")
        || lower_display.ends_with(".hdf5")
        || lower_display.ends_with(".tar")
        || lower_display.ends_with(".safe")
    {
        return Some(work.item.display_name.clone());
    }
    let sanitized_id: String = work
        .item
        .item_id
        .chars()
        .map(|ch| {
            if matches!(ch, ':' | '/' | '\\' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                ch
            }
        })
        .collect();
    Some(format!("{sanitized_id}.zip"))
}

fn ensure_workspace_revision(expected: u64, actual: u64) -> Result<(), ApplicationServiceError> {
    if expected == actual {
        Ok(())
    } else {
        Err(ApplicationServiceError::WorkspaceRevisionConflict { expected, actual })
    }
}

fn sync_workspace_regions(context: &mut WorkspaceContextState) {
    context.regions = context
        .projects
        .iter()
        .flat_map(|project| project.regions.iter().cloned())
        .collect();
}

fn safe_workspace_name(name: &str) -> Result<String, ApplicationServiceError> {
    let mut suffix = String::new();
    let mut separator = false;
    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            suffix.push(character.to_ascii_lowercase());
            separator = false;
        } else if character.is_alphanumeric() {
            suffix.push_str(&format!("u{:x}", character as u32));
            separator = false;
        } else if !suffix.is_empty() && !separator {
            suffix.push('-');
            separator = true;
        }
        if suffix.len() >= 48 {
            break;
        }
    }
    while suffix.ends_with('-') {
        suffix.pop();
    }
    if suffix.is_empty() {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "workspace node name must contain a letter or digit".to_owned(),
        });
    }
    Ok(suffix)
}

fn stable_workspace_node_id(
    prefix: &str,
    parent_id: &str,
    name: &str,
) -> Result<String, ApplicationServiceError> {
    let parent = safe_workspace_name(parent_id)?;
    let suffix = safe_workspace_name(name)?;
    Ok(format!("{prefix}-{parent}-{suffix}"))
}

fn validate_formal_task_identity(
    request_id: &str,
    task_id: &str,
    region_id: &str,
    output_root: &Path,
    created_at: &str,
) -> Result<(), ApplicationServiceError> {
    if !valid_formal_task_shape(request_id, task_id, region_id, output_root)
        || parse_normalized_utc(created_at).is_none()
    {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "formal task identity or timestamp is invalid".to_owned(),
        });
    }
    Ok(())
}

fn validate_formal_dem_task_identity(
    request_id: &str,
    task_id: &str,
    region_id: &str,
    output_root: &Path,
    created_at: &str,
) -> Result<(), ApplicationServiceError> {
    if !valid_formal_task_shape(request_id, task_id, region_id, output_root)
        || canonical_utc_identity_timestamp(created_at).is_none()
    {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "formal task identity or timestamp is invalid".to_owned(),
        });
    }
    Ok(())
}

fn valid_formal_task_shape(
    request_id: &str,
    task_id: &str,
    region_id: &str,
    output_root: &Path,
) -> bool {
    valid_catalogue_opaque_id(request_id)
        && valid_catalogue_opaque_id(task_id)
        && valid_catalogue_opaque_id(region_id)
        && task_id.len() <= 96
        && !output_root.as_os_str().is_empty()
}

fn validate_dem_source_identity(
    resource_id: &str,
    capability_id: &str,
    dataset: &str,
    provider: &str,
) -> Result<(), ApplicationServiceError> {
    let expected = dem_identity(dataset, provider);
    if expected != Some((resource_id, capability_id)) {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "DEM dataset/provider/resource/capability identity is invalid".to_owned(),
        });
    }
    Ok(())
}

fn canonical_selected_dem_source_identity(
    identity: &StoreDemSourceIdentity,
) -> Result<StoreDemSourceIdentity, ApplicationServiceError> {
    let dataset = identity.dataset.trim().to_ascii_uppercase();
    let provider = identity.provider.trim().to_ascii_uppercase();
    if identity.dataset != dataset
        || identity.provider != provider
        || dem_identity(&identity.dataset, &identity.provider).is_none()
    {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "DEM dataset/provider selection identity is invalid".to_owned(),
        });
    }
    let updated_at = canonical_utc_identity_timestamp(&identity.updated_at).ok_or_else(|| {
        ApplicationServiceError::InvalidRequest {
            reason: "DEM dataset/provider selection identity is invalid".to_owned(),
        }
    })?;
    Ok(StoreDemSourceIdentity {
        dataset: identity.dataset.clone(),
        provider: identity.provider.clone(),
        updated_at,
    })
}

fn dem_source_identity_for_persisted_task(
    operation: &TaskOperationDescriptor,
    updated_at: &str,
) -> Result<Option<StoreDemSourceIdentity>, ApplicationServiceError> {
    if operation.kind != "dem"
        || !matches!(
            operation.approved_worker_operation.as_str(),
            "dem_download" | "dem_prepare_sarscape"
        )
    {
        return Ok(None);
    }
    let dataset = operation
        .payload
        .get("dataset")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_request("formal DEM task dataset is missing"))?;
    let provider = operation
        .payload
        .get("provider")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_request("formal DEM task provider is missing"))?;
    let identity = StoreDemSourceIdentity {
        dataset: dataset.to_owned(),
        provider: provider.to_owned(),
        updated_at: updated_at.to_owned(),
    };
    Ok(Some(canonical_selected_dem_source_identity(&identity)?))
}

fn validate_dem_conversion_source_task(
    source: &insar_store::StoredTask,
    request: &DemConversionRequest,
) -> Result<(), ApplicationServiceError> {
    let operation = source
        .snapshot
        .operation
        .as_ref()
        .ok_or(ApplicationServiceError::FormalTaskSourceIncomplete)?;
    let payload = &operation.payload;
    let payload_equals =
        |key: &str, expected: &str| payload.get(key).and_then(Value::as_str) == Some(expected);
    let input_path = request.input_path.to_string_lossy();
    let output_root = request.output_root.to_string_lossy();
    let operation_authorized = (operation.phase == "execute"
        && operation.approved_worker_operation == "dem_prepare_sarscape")
        || (operation.phase == "download" && operation.approved_worker_operation == "dem_download");
    if source.runtime.status != insar_core::TaskLifecycleStatus::Completed
        || operation.kind != "dem"
        || !operation_authorized
        || operation.region_id != request.region_id
        || source.snapshot.output_root != request.output_root
        || !(payload_equals("region_id", &request.region_id)
            || (operation.phase == "download"
                && operation.approved_worker_operation == "dem_download"
                && !payload.contains_key("region_id")))
        || !payload_equals("dataset", &request.dataset)
        || !payload_equals("provider", &request.provider)
        || !payload_equals("resource_id", &request.resource_id)
        || !payload_equals("capability_id", &request.capability_id)
        || !payload_equals("input_path", input_path.as_ref())
        || !payload_equals("path", input_path.as_ref())
        || !payload_equals("output_root", output_root.as_ref())
        || !payload_equals("output_dir", output_root.as_ref())
    {
        return Err(ApplicationServiceError::FormalTaskSourceIncomplete);
    }
    Ok(())
}

fn dem_identity(dataset: &str, provider: &str) -> Option<(&'static str, &'static str)> {
    let dataset = dataset.trim().to_ascii_uppercase();
    let provider = provider.trim().to_ascii_uppercase();
    match (dataset.as_str(), provider.as_str()) {
        ("COP30", "COPERNICUS_AWS") => {
            Some(("dem.cop30.v1", "capability.dem.cop30.aws-download.v1"))
        }
        ("COP30", "OPENTOPOGRAPHY") => Some((
            "dem.cop30.v1",
            "capability.dem.cop30.opentopography-download.v1",
        )),
        ("COP90", "COPERNICUS_AWS") => {
            Some(("dem.cop90.v1", "capability.dem.cop90.aws-download.v1"))
        }
        ("COP90", "OPENTOPOGRAPHY") => Some((
            "dem.cop90.v1",
            "capability.dem.cop90.opentopography-download.v1",
        )),
        ("NASADEM", "OPENTOPOGRAPHY") => {
            Some(("dem.nasadem.v1", "capability.dem.nasadem.download.v1"))
        }
        ("SRTM_GL3", "OPENTOPOGRAPHY") => {
            Some(("dem.srtm-gl3.v1", "capability.dem.srtm-gl3.download.v1"))
        }
        ("SRTM_GL1", "OPENTOPOGRAPHY") => {
            Some(("dem.srtm-gl1.v1", "capability.dem.srtm-gl1.download.v1"))
        }
        ("AW3D30", "OPENTOPOGRAPHY") => {
            Some(("dem.aw3d30.v1", "capability.dem.aw3d30.download.v1"))
        }
        ("SRTM_GL1_ELLIPSOIDAL", "OPENTOPOGRAPHY") => Some((
            "dem.srtm-gl1-ellipsoidal.v1",
            "capability.dem.srtm-gl1-ellipsoidal.download.v1",
        )),
        ("AW3D30_ELLIPSOIDAL", "OPENTOPOGRAPHY") => Some((
            "dem.aw3d30-ellipsoidal.v1",
            "capability.dem.aw3d30-ellipsoidal.download.v1",
        )),
        ("NISAR_DEM", "NISAR_DEM_ASF") => {
            Some(("dem.nisar-dem.v1", "capability.dem.nisar-dem.download.v1"))
        }
        ("FABDEM", "FABDEM_PUBLIC") => Some(("dem.fabdem.v1", "capability.dem.fabdem.download.v1")),
        ("GEM15_DTM", "GEM15_PUBLIC") => {
            Some(("dem.gem15-dtm.v1", "capability.dem.gem15-dtm.download.v1"))
        }
        ("GEM15_DSM", "GEM15_PUBLIC") => {
            Some(("dem.gem15-dsm.v1", "capability.dem.gem15-dsm.download.v1"))
        }
        ("ASTER_GDEM_V3", "EARTHDATA")
        | ("ASTGTM", "EARTHDATA")
        | ("ASTER_GDEM_V3", "LPDAAC")
        | ("ASTGTM", "LPDAAC")
        | ("ASTER_GDEM_V3", "ASF")
        | ("ASTGTM", "ASF") => Some((
            "dem.aster-gdem.v3",
            "capability.dem.aster-gdem-v3.download.v1",
        )),
        ("USER_LOCAL", "LOCAL_FILE") => {
            Some(("dem.user-local.v1", "capability.dem.user-local.convert.v1"))
        }
        _ => None,
    }
}

fn dem_resource_id(dataset: &str, provider: &str) -> String {
    dem_identity(dataset, provider)
        .map(|(resource_id, _)| resource_id.to_owned())
        .unwrap_or_default()
}

fn dem_capability_id(dataset: &str, provider: &str) -> String {
    dem_identity(dataset, provider)
        .map(|(_, capability_id)| capability_id.to_owned())
        .unwrap_or_default()
}

fn validate_dem_task_settings(settings: &DemTaskSettings) -> Result<(), ApplicationServiceError> {
    if !(1..=10).contains(&settings.max_concurrent_tasks)
        || settings.max_concurrent_conversions != 1
    {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "DEM task settings must use 1..=10 tasks and exactly 1 conversion".to_owned(),
        });
    }
    Ok(())
}

fn default_sar_download_policy_settings() -> SarDownloadPolicySettings {
    SarDownloadPolicySettings {
        mode: SarDownloadPolicyMode::Manual,
        max_concurrent_items: 2,
        organize_by_product: false,
        default_output_root: String::new(),
    }
}

fn validate_sar_download_policy_settings(
    settings: &SarDownloadPolicySettings,
) -> Result<(), ApplicationServiceError> {
    settings
        .validate()
        .map_err(|reason| invalid_request(format!("SAR download policy is invalid: {reason}")))?;
    if !matches!(settings.mode, SarDownloadPolicyMode::Manual) {
        return Err(invalid_request(
            "SAR download policy supports only manual mode",
        ));
    }
    if !settings.default_output_root.trim().is_empty()
        && !Path::new(settings.default_output_root.trim()).is_absolute()
    {
        return Err(invalid_request(
            "SAR default output root must be an absolute local path",
        ));
    }
    Ok(())
}

pub(crate) fn validate_network_settings(
    settings: &ApplicationNetworkSettings,
) -> Result<(), ApplicationServiceError> {
    // An empty proxy_url is the saved representation of the automatic
    // system-proxy mode and must stay acceptable; the old `Url::parse("")`
    // failure rejected every such save, which the settings UI then silently
    // dropped as a generic backend error.
    if !settings.proxy_url.trim().is_empty() {
        let url = Url::parse(settings.proxy_url.trim())
            .map_err(|_| invalid_request("proxy_url must contain an HTTP proxy host and port"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.port().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid_request(
                "proxy_url must contain only an HTTP proxy host and port",
            ));
        }
    }
    if settings.cache_limit_mb > 1024 * 1024 {
        return Err(invalid_request("cache_limit_mb exceeds the product limit"));
    }
    if settings.tianditu_token.len() > 4_096 {
        return Err(invalid_request("tianditu_token exceeds the product limit"));
    }
    if !settings.cache_dir.trim().is_empty() && !Path::new(settings.cache_dir.trim()).is_absolute()
    {
        return Err(invalid_request("cache_dir must be an absolute local path"));
    }
    Ok(())
}

fn validate_standalone_dem_request(
    request: &StandaloneDemRequest,
) -> Result<(), ApplicationServiceError> {
    if !valid_catalogue_opaque_id(&request.request_id)
        || request.input_path.as_os_str().is_empty()
        || request.output_root.as_os_str().is_empty()
        || !request.output_root.is_absolute()
        || request.dataset.trim().is_empty()
        || request.provider.trim().is_empty()
    {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "standalone DEM request is invalid".to_owned(),
        });
    }
    if let Some(bbox) = request.bbox.as_ref() {
        if bbox.len() != 4
            || bbox.iter().any(|value| !value.is_finite())
            || !(bbox[0] < bbox[2] && bbox[1] < bbox[3])
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "standalone DEM bbox is invalid".to_owned(),
            });
        }
    }
    if let Some(mode) = request.output_mode.as_deref() {
        if !matches!(mode.to_ascii_lowercase().as_str(), "ellipsoid" | "sarscape") {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "standalone DEM output_mode is invalid".to_owned(),
            });
        }
    }
    if let Some(source) = request.source_vertical_datum.as_deref() {
        if !matches!(
            source.to_ascii_uppercase().as_str(),
            "AUTO" | "EGM96" | "EGM2008" | "WGS84_ELLIPSOID"
        ) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "standalone DEM source vertical datum is invalid".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_standalone_dem_task_request(
    request: &StandaloneDemTaskRequest,
) -> Result<(), ApplicationServiceError> {
    if !valid_catalogue_opaque_id(&request.request_id)
        || !valid_catalogue_opaque_id(&request.task_id)
        || request.output_root.as_os_str().is_empty()
        || !request.output_root.is_absolute()
        || canonical_utc_identity_timestamp(&request.created_at).is_none()
        || !(1..=10).contains(&request.concurrency)
        || request.dataset.trim().is_empty()
        || request.provider.trim().is_empty()
    {
        return Err(invalid_request("standalone DEM task identity is invalid"));
    }
    if request.bbox.len() != 4
        || request.bbox.iter().any(|value| !value.is_finite())
        || request.bbox[0] < -180.0
        || request.bbox[2] > 180.0
        || request.bbox[1] < -90.0
        || request.bbox[3] > 90.0
        || !(request.bbox[0] < request.bbox[2] && request.bbox[1] < request.bbox[3])
        || request.bbox[2] - request.bbox[0] > 10.0
        || request.bbox[3] - request.bbox[1] > 10.0
    {
        return Err(invalid_request(
            "standalone DEM bbox must be valid and no larger than 10 degrees per axis",
        ));
    }
    if dem_identity(&request.dataset, &request.provider).is_none() {
        return Err(invalid_request(
            "standalone DEM provider route is not admitted",
        ));
    }
    Ok(())
}

fn validate_standalone_dem_aoi_task_request(
    request: &StandaloneDemAoiTaskRequest,
) -> Result<(), ApplicationServiceError> {
    if !valid_catalogue_opaque_id(&request.request_id)
        || !valid_catalogue_opaque_id(&request.task_id)
        || request.output_root.as_os_str().is_empty()
        || !request.output_root.is_absolute()
        || canonical_utc_identity_timestamp(&request.created_at).is_none()
        || !(1..=10).contains(&request.concurrency)
        || request.dataset.trim().is_empty()
        || request.provider.trim().is_empty()
        || request.aoi_name.trim().is_empty()
        || request.aoi_name != request.aoi_name.trim()
        || !valid_task_path_segment(&request.task_id)
    {
        return Err(invalid_request(
            "standalone DEM AOI task identity is invalid",
        ));
    }
    if validated_authoritative_polygonal_geojson(&request.aoi_geojson).is_none() {
        return Err(invalid_request(
            "standalone DEM AOI geometry must be an authoritative Polygon or MultiPolygon",
        ));
    }
    if dem_identity(&request.dataset, &request.provider).is_none() {
        return Err(invalid_request(
            "standalone DEM AOI provider route is not admitted",
        ));
    }
    Ok(())
}

fn valid_task_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn validate_capability_windows(windows: &[BBox]) -> Result<(), ApplicationServiceError> {
    if windows.is_empty()
        || windows.len() > 2
        || windows.iter().any(|window| {
            ![window.west, window.south, window.east, window.north]
                .into_iter()
                .all(f64::is_finite)
                || window.crs != "EPSG:4326"
                || window.west < -180.0
                || window.east > 180.0
                || window.south < -90.0
                || window.north > 90.0
                || window.west >= window.east
                || window.south >= window.north
        })
    {
        return Err(invalid_request(
            "AOI must derive one or two finite ordered EPSG:4326 request windows",
        ));
    }
    Ok(())
}

fn buffered_capability_windows(
    display_west: f64,
    display_east: f64,
    south: f64,
    north: f64,
    buffer_degrees: f64,
) -> Result<Vec<BBox>, ApplicationServiceError> {
    if ![display_west, display_east, south, north, buffer_degrees]
        .into_iter()
        .all(f64::is_finite)
        || display_west >= display_east
        || south >= north
        || !(0.0..=10.0).contains(&buffer_degrees)
    {
        return Err(invalid_request(
            "GACOS buffer and AOI display interval must be finite and bounded",
        ));
    }

    let buffered_south = (south - buffer_degrees).max(-90.0);
    let buffered_north = (north + buffer_degrees).min(90.0);
    let buffered_west = display_west - buffer_degrees;
    let buffered_east = display_east + buffer_degrees;
    let width = buffered_east - buffered_west;
    let crs = || "EPSG:4326".to_owned();
    if width >= 360.0 {
        return Err(invalid_request(
            "GACOS buffer would create a full-world request window",
        ));
    }
    let windows = {
        let normalized_west = (buffered_west + 180.0).rem_euclid(360.0) - 180.0;
        let world_shift = buffered_west - normalized_west;
        let normalized_east = buffered_east - world_shift;
        if normalized_east <= 180.0 {
            vec![BBox {
                west: normalized_west,
                south: buffered_south,
                east: normalized_east,
                north: buffered_north,
                crs: crs(),
            }]
        } else {
            vec![
                BBox {
                    west: normalized_west,
                    south: buffered_south,
                    east: 180.0,
                    north: buffered_north,
                    crs: crs(),
                },
                BBox {
                    west: -180.0,
                    south: buffered_south,
                    east: normalized_east - 360.0,
                    north: buffered_north,
                    crs: crs(),
                },
            ]
        }
    };
    validate_capability_windows(&windows)?;
    Ok(windows)
}

fn capability_window_value(window: &BBox) -> Value {
    serde_json::json!({
        "west": window.west,
        "south": window.south,
        "east": window.east,
        "north": window.north,
        "crs": window.crs.as_str(),
    })
}

fn standalone_bbox_geometry(bbox: &[f64]) -> Result<Map<String, Value>, ApplicationServiceError> {
    if bbox.len() != 4 {
        return Err(invalid_request(
            "standalone DEM bbox must contain four coordinates",
        ));
    }
    serde_json::json!({
        "type": "Polygon",
        "coordinates": [[
            [bbox[0], bbox[1]],
            [bbox[2], bbox[1]],
            [bbox[2], bbox[3]],
            [bbox[0], bbox[3]],
            [bbox[0], bbox[1]]
        ]]
    })
    .as_object()
    .cloned()
    .ok_or_else(|| invalid_request("standalone DEM bbox geometry could not be built"))
}

fn dem_task_stem(dataset: &str) -> String {
    let mut stem = dataset
        .trim()
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || value == '_' {
                value
            } else {
                '_'
            }
        })
        .collect::<String>();
    if stem.is_empty() {
        stem = "DEM".to_owned();
    }
    stem
}

fn dem_task_source_stem(dataset: &str, source_vertical_datum: &str) -> String {
    format!(
        "{}_{}",
        dem_task_stem(dataset),
        dem_task_stem(source_vertical_datum)
    )
}

fn dem_request_file_tag(
    created_at: &str,
    task_id: &str,
) -> Result<String, ApplicationServiceError> {
    let created_at = canonical_utc_identity_timestamp(created_at)
        .ok_or_else(|| invalid_request("standalone DEM created_at is invalid"))?;
    let date_tag = created_at
        .get(..10)
        .ok_or_else(|| invalid_request("standalone DEM created_at is invalid"))?
        .replace('-', "");
    let digest = Sha256::digest(task_id.as_bytes());
    let digest = format!("{digest:x}");
    Ok(format!("{date_tag}_{}", &digest[..8]))
}

fn validate_dem_provider_request_windows(
    dataset: &str,
    provider: &str,
    windows: &[BBox],
) -> Result<(), ApplicationServiceError> {
    if dem_limits::limits_apply(provider) {
        if let Some(estimate) = windows
            .iter()
            .map(|window| {
                dem_limits::estimate(
                    dataset,
                    [window.west, window.south, window.east, window.north],
                )
            })
            .find(|estimate| estimate.exceeds_mosaic() || estimate.exceeds_tiles())
        {
            return Err(ApplicationServiceError::DemRangeLimitExceeded {
                width: estimate.width,
                height: estimate.height,
                pixels: estimate.pixels,
                coarser_dataset_available: dem_limits::coarser_dataset_available(dataset),
            });
        }
        return Ok(());
    }
    if !provider.trim().eq_ignore_ascii_case("OPENTOPOGRAPHY") {
        return Ok(());
    }
    let dataset = dataset.trim().to_ascii_uppercase();
    let limit_km2 = match dataset.as_str() {
        "COP30"
        | "NASADEM"
        | "SRTM_GL1"
        | "AW3D30"
        | "SRTM_GL1_ELLIPSOIDAL"
        | "AW3D30_ELLIPSOIDAL" => 450_000.0,
        "COP90" | "SRTM_GL3" => 4_050_000.0,
        _ => return Ok(()),
    };
    if let Some(requested_km2) = windows
        .iter()
        .map(dem_request_window_area_km2)
        .find(|area| *area > limit_km2)
    {
        return Err(ApplicationServiceError::OpenTopographyAreaLimitExceeded {
            requested_km2: requested_km2.ceil() as u64,
            limit_km2: limit_km2 as u64,
        });
    }
    Ok(())
}

fn dem_request_window_area_km2(window: &BBox) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6_371.008_8;
    let west = window.west.to_radians();
    let east = window.east.to_radians();
    let south = window.south.to_radians();
    let north = window.north.to_radians();
    EARTH_RADIUS_KM.powi(2) * (east - west).abs() * (north.sin() - south.sin()).abs()
}

fn standalone_dem_source_datum(dataset: &str) -> &'static str {
    match dataset.trim().to_ascii_uppercase().as_str() {
        "COP30" | "COP90" | "FABDEM" => "EGM2008",
        // NISAR DEM heights are already re-referenced to the WGS84 ellipsoid
        // at the source, so no geoid conversion may be planned for it.
        "SRTM_GL1_ELLIPSOIDAL" | "AW3D30_ELLIPSOIDAL" | "NISAR_DEM" => "WGS84_ELLIPSOID",
        "GEM15_DTM" | "GEM15_DSM" => "EGM96",
        _ => "EGM96",
    }
}

fn resolve_standalone_dem_source_datum(dataset: &str, requested: &str) -> String {
    if requested.eq_ignore_ascii_case("auto") {
        standalone_dem_source_datum(dataset).to_owned()
    } else {
        requested.to_owned()
    }
}

#[cfg(test)]
mod dem_source_datum_tests {
    use super::{
        dem_identity, dem_request_file_tag, dem_task_source_stem,
        resolve_standalone_dem_source_datum, validate_dem_provider_request_windows,
        ApplicationServiceError, BBox,
    };

    #[test]
    fn auto_vertical_datum_uses_dataset_science_truth() {
        assert_eq!(
            resolve_standalone_dem_source_datum("COP30", "auto"),
            "EGM2008"
        );
        assert_eq!(
            resolve_standalone_dem_source_datum("COP90", "AUTO"),
            "EGM2008"
        );
        assert_eq!(
            resolve_standalone_dem_source_datum("NASADEM", "auto"),
            "EGM96"
        );
        assert_eq!(
            resolve_standalone_dem_source_datum("SRTM_GL1_ELLIPSOIDAL", "auto"),
            "WGS84_ELLIPSOID"
        );
        // NISAR DEM heights are already re-referenced to the WGS84 ellipsoid,
        // so no geoid conversion may ever be planned for this source.
        assert_eq!(
            resolve_standalone_dem_source_datum("NISAR_DEM", "auto"),
            "WGS84_ELLIPSOID"
        );
    }

    #[test]
    fn standalone_dem_file_tag_changes_with_request_time_or_task_identity() {
        let first = dem_request_file_tag("2026-09-17T00:00:00Z", "dem-task-a").unwrap();
        let later = dem_request_file_tag("2026-09-17T00:01:00Z", "dem-task-a").unwrap();
        let other = dem_request_file_tag("2026-09-17T00:00:00Z", "dem-task-b").unwrap();
        let next_day = dem_request_file_tag("2026-09-18T00:00:00Z", "dem-task-a").unwrap();

        assert!(first.starts_with("20260917_"));
        assert_eq!(first, later);
        assert_ne!(first, other);
        assert_ne!(first, next_day);
        assert_eq!(first.len(), "20260917_".len() + 8);
        assert!(first
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'));
    }

    #[test]
    fn standalone_dem_output_stem_preserves_source_product_and_vertical_datum() {
        assert_eq!(dem_task_source_stem("COP30", "EGM2008"), "COP30_EGM2008");
        assert_eq!(
            dem_task_source_stem("SRTM_GL1_ELLIPSOIDAL", "WGS84_ELLIPSOID"),
            "SRTM_GL1_ELLIPSOIDAL_WGS84_ELLIPSOID"
        );
        assert_eq!(
            dem_task_source_stem("NISAR_DEM", "WGS84_ELLIPSOID"),
            "NISAR_DEM_WGS84_ELLIPSOID"
        );
    }

    #[test]
    fn nisar_dem_identity_maps_to_the_earthdata_authenticated_asf_route() {
        let (resource_id, capability_id) =
            dem_identity("NISAR_DEM", "NISAR_DEM_ASF").expect("registered NISAR DEM route");
        assert_eq!(resource_id, "dem.nisar-dem.v1");
        assert_eq!(capability_id, "capability.dem.nisar-dem.download.v1");
        // The dataset stays on its own provider; auto cannot reroute it.
        assert_eq!(dem_identity("NISAR_DEM", "COPERNICUS_AWS"), None);
        assert_eq!(dem_identity("NISAR_DEM", "OPENTOPOGRAPHY"), None);
        assert_eq!(dem_identity("COP30", "NISAR_DEM_ASF"), None);
    }

    #[test]
    fn fabdem_identity_and_datum_resolve_to_public_egm2008_route() {
        let (resource_id, capability_id) =
            dem_identity("FABDEM", "FABDEM_PUBLIC").expect("registered FABDEM public route");
        assert_eq!(resource_id, "dem.fabdem.v1");
        assert_eq!(capability_id, "capability.dem.fabdem.download.v1");
        assert_eq!(dem_identity("FABDEM", "COPERNICUS_AWS"), None);
        assert_eq!(dem_identity("COP30", "FABDEM_PUBLIC"), None);
        assert_eq!(
            resolve_standalone_dem_source_datum("FABDEM", "auto"),
            "EGM2008"
        );
        assert_eq!(dem_task_source_stem("FABDEM", "EGM2008"), "FABDEM_EGM2008");
    }

    #[test]
    fn gem15_dtm_and_dsm_identity_and_datum_resolve_to_public_egm96_route() {
        let (dtm_res, dtm_cap) =
            dem_identity("GEM15_DTM", "GEM15_PUBLIC").expect("registered GEM-15 DTM public route");
        assert_eq!(dtm_res, "dem.gem15-dtm.v1");
        assert_eq!(dtm_cap, "capability.dem.gem15-dtm.download.v1");

        let (dsm_res, dsm_cap) =
            dem_identity("GEM15_DSM", "GEM15_PUBLIC").expect("registered GEM-15 DSM public route");
        assert_eq!(dsm_res, "dem.gem15-dsm.v1");
        assert_eq!(dsm_cap, "capability.dem.gem15-dsm.download.v1");

        assert_eq!(dem_identity("GEM15_DTM", "COPERNICUS_AWS"), None);
        assert_eq!(dem_identity("GEM15_DSM", "COPERNICUS_AWS"), None);
        assert_eq!(dem_identity("COP30", "GEM15_PUBLIC"), None);

        assert_eq!(
            resolve_standalone_dem_source_datum("GEM15_DTM", "auto"),
            "EGM96"
        );
        assert_eq!(
            resolve_standalone_dem_source_datum("GEM15_DSM", "auto"),
            "EGM96"
        );
        assert_eq!(
            dem_task_source_stem("GEM15_DTM", "EGM96"),
            "GEM15_DTM_EGM96"
        );
        assert_eq!(
            dem_task_source_stem("GEM15_DSM", "EGM96"),
            "GEM15_DSM_EGM96"
        );
    }

    #[test]
    fn aster_gdem_v3_identity_maps_to_the_earthdata_route() {
        let (resource_id, capability_id) =
            dem_identity("ASTER_GDEM_V3", "EARTHDATA").expect("registered ASTER GDEM route");
        assert_eq!(resource_id, "dem.aster-gdem.v3");
        assert_eq!(capability_id, "capability.dem.aster-gdem-v3.download.v1");
        let (resource_id, capability_id) =
            dem_identity("ASTGTM", "LPDAAC").expect("registered ASTER GDEM short alias");
        assert_eq!(resource_id, "dem.aster-gdem.v3");
        assert_eq!(capability_id, "capability.dem.aster-gdem-v3.download.v1");
        assert_eq!(dem_identity("ASTER_GDEM_V3", "COPERNICUS_AWS"), None);
    }

    #[test]
    fn nisar_dem_ranges_beyond_the_native_mosaic_are_refused_like_copernicus() {
        let error = validate_dem_provider_request_windows(
            "NISAR_DEM",
            "NISAR_DEM_ASF",
            &[window(100.0, 20.0, 111.111_1, 30.555_5)],
        )
        .expect_err("40000 x 38000 samples exceed the native mosaic limit");
        assert!(matches!(
            error,
            ApplicationServiceError::DemRangeLimitExceeded {
                width: 40_000,
                height: 38_000,
                ..
            }
        ));
        // A bounded window is admitted: NISAR DEM is 30 m (3600 samples/deg).
        assert!(validate_dem_provider_request_windows(
            "NISAR_DEM",
            "NISAR_DEM_ASF",
            &[window(108.0, 29.0, 116.0, 34.0)],
        )
        .is_ok());
    }

    #[test]
    fn opentopography_rejects_the_owner_acceptance_aoi_for_30m_but_not_90m() {
        let owner_aoi = BBox {
            west: 81.166590471095,
            south: 27.53829743089898,
            east: 92.108615864543,
            north: 33.18200262354754,
            crs: "EPSG:4326".to_owned(),
        };

        let error = validate_dem_provider_request_windows(
            "COP30",
            "OpenTopography",
            std::slice::from_ref(&owner_aoi),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ApplicationServiceError::OpenTopographyAreaLimitExceeded {
                requested_km2,
                limit_km2: 450_000,
            } if requested_km2 > 450_000
        ));
        assert!(
            validate_dem_provider_request_windows("COP90", "OPENTOPOGRAPHY", &[owner_aoi]).is_ok()
        );
    }

    fn window(west: f64, south: f64, east: f64, north: f64) -> BBox {
        BBox {
            west,
            south,
            east,
            north,
            crs: "EPSG:4326".to_owned(),
        }
    }

    #[test]
    fn copernicus_aws_ranges_beyond_the_native_mosaic_are_refused_before_a_task_exists() {
        let error = validate_dem_provider_request_windows(
            "COP30",
            "COPERNICUS_AWS",
            &[window(100.0, 20.0, 111.111_1, 30.555_5)],
        )
        .expect_err("40000 x 38000 samples exceed the native mosaic limit");
        assert_eq!(error.code(), "DEM_RANGE_LIMIT_EXCEEDED");
        assert!(matches!(
            error,
            ApplicationServiceError::DemRangeLimitExceeded {
                width: 40_000,
                height: 38_000,
                pixels: 1_520_000_000,
                coarser_dataset_available: true,
            }
        ));
        assert!(validate_dem_provider_request_windows(
            "COP30",
            "OPENTOPOGRAPHY",
            &[window(100.0, 20.0, 101.0, 21.0)],
        )
        .is_ok());
        let at_limit = window(0.0, 0.0, 9.102_2, 4.551_1);
        assert!(
            validate_dem_provider_request_windows("COP30", "COPERNICUS_AWS", &[at_limit]).is_ok()
        );
        assert!(validate_dem_provider_request_windows(
            "COP30",
            "COPERNICUS_AWS",
            &[window(0.0, 0.0, 9.102_2, 4.551_3)],
        )
        .is_err());
    }
}

fn dem_worker_operation(operation: &DemTaskOperation) -> &'static str {
    match operation {
        DemTaskOperation::Inspect => "dem_inspect",
        DemTaskOperation::PrepareSarscape => "dem_prepare_sarscape",
        DemTaskOperation::Convert => "dem_convert",
    }
}

fn dem_operation_phase(operation: &DemTaskOperation) -> &'static str {
    match operation {
        DemTaskOperation::Inspect => "plan",
        DemTaskOperation::PrepareSarscape => "execute",
        DemTaskOperation::Convert => "conversion",
    }
}

fn runtime_has_active_cdse_work(runtime: &ServiceRuntime) -> Result<bool, ApplicationServiceError> {
    if runtime
        .dispatcher
        .as_ref()
        .is_some_and(|dispatcher| dispatcher.has_active_catalogue_requests())
    {
        return Ok(true);
    }
    let store = runtime
        .store
        .lock()
        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
    let restore = store
        .restore(&StoreRestoreRequest {
            include_terminal: false,
        })
        .map_err(ApplicationServiceError::from)?;
    Ok(network_routes::runtimes_may_transfer(&restore.runtimes))
}

fn bhoonidhi_asset_size_resolve_error(error: ResolveError) -> ApplicationServiceError {
    match error.kind() {
        ResolveErrorKind::Cancelled => ApplicationServiceError::BhoonidhiAssetCancelled,
        ResolveErrorKind::ReferenceInvalid
        | ResolveErrorKind::DescriptorInvalid
        | ResolveErrorKind::AccessNotDurable
        | ResolveErrorKind::BurstCookieRequired => {
            ApplicationServiceError::BhoonidhiAssetEvidenceInvalid
        }
        ResolveErrorKind::CredentialsMissing => catalogue_provider_error(
            "BHOONIDHI_CREDENTIALS_MISSING",
            "Bhoonidhi 产品大小读取需要已保存的登录凭据",
            "asset_size_probe",
        ),
        ResolveErrorKind::AuthFailed | ResolveErrorKind::RefreshBudgetExhausted => {
            catalogue_provider_error(
                "BHOONIDHI_AUTH_REJECTED",
                "Bhoonidhi 登录会话无法用于读取产品大小",
                "asset_size_probe",
            )
        }
        ResolveErrorKind::Timeout => catalogue_provider_error(
            "BHOONIDHI_NETWORK_ERROR",
            "Bhoonidhi 产品大小读取超时",
            "asset_size_probe",
        ),
        ResolveErrorKind::ResolverNotConfigured
        | ResolveErrorKind::Expired
        | ResolveErrorKind::Failed
        | ResolveErrorKind::Unavailable => catalogue_provider_error(
            "BHOONIDHI_NETWORK_ERROR",
            "Bhoonidhi 产品大小读取失败",
            "asset_size_probe",
        ),
    }
}

/// #22 B slice 3(b): how many mapped candidates the streaming sink
/// accumulates in memory before flushing them into Store staging. Bounded so
/// the peak in-memory candidate set never grows with the result size.
const CATALOGUE_STREAMING_CHUNK: usize = 128;

/// #22 B slice 3(c): per-search progressive delivery state. The sink stages
/// and publishes; the provider progress closure learns the provider total and
/// reports the loaded count once the batch is readable.
struct ProgressiveDelivery {
    batch_id: String,
    staged: std::sync::atomic::AtomicU64,
    published: std::sync::atomic::AtomicBool,
    remote_total: Mutex<Option<u64>>,
    recorded_total: Mutex<Option<u64>>,
}

impl ProgressiveDelivery {
    fn new(batch_id: &str) -> Self {
        Self {
            batch_id: batch_id.to_owned(),
            staged: std::sync::atomic::AtomicU64::new(0),
            published: std::sync::atomic::AtomicBool::new(false),
            remote_total: Mutex::new(None),
            recorded_total: Mutex::new(None),
        }
    }

    fn is_published(&self) -> bool {
        self.published.load(std::sync::atomic::Ordering::Acquire)
    }

    fn mark_published(&self, recorded_total: Option<u64>) {
        if let Ok(mut recorded) = self.recorded_total.lock() {
            *recorded = recorded_total;
        }
        self.published
            .store(true, std::sync::atomic::Ordering::Release);
    }

    fn add_staged(&self, count: u64) -> u64 {
        self.staged
            .fetch_add(count, std::sync::atomic::Ordering::AcqRel)
            .saturating_add(count)
    }

    fn reset_staged(&self) {
        self.staged.store(0, std::sync::atomic::Ordering::Release);
    }

    fn observe_remote_total(&self, remote_total: Option<u64>) {
        if let (Some(total), Ok(mut current)) = (remote_total, self.remote_total.lock()) {
            *current = Some(total);
        }
    }

    fn remote_total(&self) -> Option<u64> {
        self.remote_total.lock().ok().and_then(|total| *total)
    }

    /// The provider total once it differs from what the Store recorded.
    fn unrecorded_remote_total(&self) -> Option<u64> {
        let total = self.remote_total()?;
        let mut recorded = self.recorded_total.lock().ok()?;
        if *recorded == Some(total) {
            return None;
        }
        *recorded = Some(total);
        Some(total)
    }

    /// While the batch is readable, a live update names it and its loaded
    /// count; before publication the update is unchanged.
    fn annotate(&self, update: &mut SarSearchProgressUpdate) {
        if self.is_published() {
            update.batch_id = Some(self.batch_id.clone());
            update.loaded_count = Some(self.staged.load(std::sync::atomic::Ordering::Acquire));
        }
    }
}

/// #22 B slice 3(b): adapt an `ApplicationServiceError` raised inside the
/// streaming sink back to the `ContractError` the provider boundary expects.
/// Application-only variants are collapsed to a fixed safe context.
fn application_to_provider_error(error: ApplicationServiceError) -> ContractError {
    match error {
        ApplicationServiceError::Provider(contract) => contract,
        _ => ContractError::new("CATALOGUE_PRODUCT_INVALID", "目录产品映射失败", Map::new())
            .expect("fixed catalogue mapping error context is safe"),
    }
}

fn limit_overflow_error(reason: &'static str) -> ContractError {
    ContractError::new(
        "CATALOGUE_RESULT_LIMIT_EXCEEDED",
        "目录 staging 候选计数溢出",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded catalogue staging context is safe")
}

fn invalid_query_error(reason: &'static str) -> ContractError {
    ContractError::new(
        "CATALOGUE_RESPONSE_INVALID",
        "目录查询无法冻结",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded catalogue query context is safe")
}

fn catalogue_provider_error(code: &str, message: &str, reason: &str) -> ApplicationServiceError {
    ApplicationServiceError::Provider(
        ContractError::new(
            code,
            message,
            Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
        )
        .expect("bounded catalogue provider context is safe"),
    )
}

fn catalogue_provider_fallback_reason(
    error: &ContractError,
) -> Option<CatalogSourceFallbackReason> {
    let status = error
        .debug_context
        .get("http_status")
        .and_then(Value::as_u64);
    match error.code.as_str() {
        "PLANETARY_COMPUTER_TRANSPORT_FAILED"
        | "CDSE_ODATA_TIMEOUT"
        | "CDSE_ODATA_HTTP_FAILURE"
        | "USGS_M2M_TRANSPORT_FAILED"
        | "USGS_M2M_TIMEOUT"
        | "STAC_TIMEOUT"
        | "STAC_NETWORK_ERROR" => Some(CatalogSourceFallbackReason::TransientNetwork),
        "USGS_M2M_RATE_LIMITED" | "STAC_RATE_LIMITED" | "CDSE_ODATA_HTTP_RATE_LIMITED" => {
            Some(CatalogSourceFallbackReason::RateLimited)
        }
        "PLANETARY_COMPUTER_HTTP_REJECTED" if status == Some(429) => {
            Some(CatalogSourceFallbackReason::RateLimited)
        }
        "PLANETARY_COMPUTER_HTTP_REJECTED" | "USGS_M2M_REMOTE_ERROR"
            if status.is_some_and(|status| (500..=599).contains(&status)) =>
        {
            Some(CatalogSourceFallbackReason::ServerError)
        }
        "STAC_SERVER_ERROR" | "CDSE_ODATA_HTTP_SERVER" => {
            Some(CatalogSourceFallbackReason::ServerError)
        }
        // Cancellation, auth, redirects, malformed/oversize responses, identity,
        // pagination, scientific mapping and configuration errors are terminal.
        _ => None,
    }
}

#[cfg(test)]
fn catalogue_provider_error_allows_fallback(error: &ContractError) -> bool {
    catalogue_provider_fallback_reason(error).is_some()
}

fn reused_catalogue_search_stats(
    mut stats: Option<FrozenSearchStats>,
) -> Option<FrozenSearchStats> {
    if let Some(stats) = &mut stats {
        // Cache reuse proves the cached result, not a new provider attempt. A
        // new frozen batch must never replay the previous request's fallback history.
        stats.source_attempts.clear();
    }
    stats
}

#[cfg(test)]
mod catalogue_fallback_error_tests {
    use super::*;

    fn provider_error(code: &str, status: Option<u64>) -> ContractError {
        let mut context = Map::new();
        if let Some(status) = status {
            context.insert("http_status".to_owned(), Value::Number(status.into()));
        }
        ContractError::new(code, "fixture provider error", context).unwrap()
    }

    #[test]
    fn production_landsat_plan_ignores_source_hint_and_keeps_unadmitted_usgs_out() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-auto-provider-plan-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let service = ApplicationService::new(ApplicationServiceConfig::business_store(
            root.join("tasks.sqlite3"),
        ));
        let seed: CatalogSearchQuery = serde_json::from_value(serde_json::json!({
            "logical_dataset": LANDSAT_C2_L2_LOGICAL_DATASET,
            "provider_id": "usgs.eros.m2m",
            "native_collection_id": "landsat_ot_c2_l2",
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]
            },
            "result_limit": 2,
            "filters": {
                "cloud_cover_lte": 40.0,
                "datetime": {"start":"2026-01-01T00:00:00Z","end":"2026-01-02T00:00:00Z"},
                "spacecraft": ["landsat_8", "landsat_9"],
                "processing_levels": ["l2sp", "l2sr"]
            }
        }))
        .unwrap();
        // Production capability observations are attached while the runtime is
        // constructed. The real catalogue path calls dispatcher_handle() before
        // building the candidate plan, so the unit test must observe the same order.
        service.startup().unwrap();
        let plan = service
            .catalogue_candidate_queries(&seed, RouteUse::Production, false)
            .unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(
            plan[1].provider_id,
            insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID
        );
        assert_eq!(
            plan[1].native_collection_id,
            insar_provider::catalog::EARTH_SEARCH_LANDSAT_COLLECTION_ID
        );
        assert!(plan
            .iter()
            .all(|candidate| candidate.filters == seed.filters
                && candidate.aoi_geojson == seed.aoi_geojson
                && candidate.result_limit == seed.result_limit));
        assert_eq!(
            plan[0].provider_id,
            insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
        );
        assert_eq!(
            plan[0].native_collection_id,
            insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID
        );
        assert!(plan
            .iter()
            .all(|query| query.provider_id != "usgs.eros.m2m"));
        service.shutdown().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn catalogue_binding_preserves_single_source_and_anonymous_sentinel2_query_scope() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-single-route-catalogue-plan-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let service = ApplicationService::new(ApplicationServiceConfig::business_store(
            root.join("tasks.sqlite3"),
        ));
        let seed: CatalogSearchQuery = serde_json::from_value(serde_json::json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": "copernicus.dataspace.odata",
            "native_collection_id": "SENTINEL-2",
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]
            },
            "result_limit": 2,
            "filters": {"product_type": "S2MSI2A"}
        }))
        .unwrap();
        service.startup().unwrap();
        let plan = service
            .catalogue_candidate_queries(&seed, RouteUse::Production, false)
            .unwrap();
        // S2 now has three admitted public search routes; its primary must not
        // disappear just because a download login is missing.
        let mut secondary = seed.clone();
        secondary.provider_id = insar_provider::catalog::MPC_SENTINEL2_L2A_PROVIDER_ID.to_owned();
        secondary.native_collection_id =
            insar_provider::catalog::MPC_SENTINEL2_L2A_COLLECTION_ID.to_owned();
        let mut tertiary = seed.clone();
        tertiary.provider_id =
            insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID.to_owned();
        tertiary.native_collection_id =
            insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID.to_owned();
        assert_eq!(plan, vec![seed.clone(), secondary, tertiary]);
        // Retain the original single-source invariant on an actual single-source product.
        let mut single = seed;
        single.logical_dataset = ALOS2_LOGICAL_DATASET.to_owned();
        single.provider_id = ALOS2_PROVIDER_ID.to_owned();
        single.native_collection_id = ALOS2_NATIVE_COLLECTION_ID.to_owned();
        single.filters.clear();
        assert_eq!(
            service
                .catalogue_candidate_queries(&single, RouteUse::Production, false)
                .unwrap(),
            vec![single]
        );
        service.shutdown().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fallback_reasons_are_coarse_and_secret_free() {
        assert_eq!(
            catalogue_provider_fallback_reason(&provider_error(
                "PLANETARY_COMPUTER_TRANSPORT_FAILED",
                None,
            )),
            Some(CatalogSourceFallbackReason::TransientNetwork)
        );
        assert_eq!(
            catalogue_provider_fallback_reason(&provider_error(
                "PLANETARY_COMPUTER_HTTP_REJECTED",
                Some(429),
            )),
            Some(CatalogSourceFallbackReason::RateLimited)
        );
        assert_eq!(
            catalogue_provider_fallback_reason(&provider_error(
                "PLANETARY_COMPUTER_HTTP_REJECTED",
                Some(503),
            )),
            Some(CatalogSourceFallbackReason::ServerError)
        );
    }

    #[test]
    fn catalogue_cache_reuse_never_replays_provider_attempt_history() {
        let stats: FrozenSearchStats = serde_json::from_value(serde_json::json!({
            "matched_count": 1,
            "fetched_record_count": 1,
            "mapped_record_count": 1,
            "provider_returned_count": 1,
            "frozen_candidate_count": 1,
            "truncated": false,
            "source": "TYPED_CATALOG",
            "source_attempts": [{
                "provider_id": "microsoft.planetary-computer",
                "native_collection_id": "landsat-c2-l2",
                "outcome": "succeeded",
                "fallback_reason": null
            }]
        }))
        .unwrap();
        assert!(reused_catalogue_search_stats(Some(stats))
            .unwrap()
            .source_attempts
            .is_empty());
    }

    #[test]
    fn only_bounded_transient_network_failures_allow_source_fallback() {
        for error in [
            provider_error("CDSE_ODATA_TIMEOUT", None),
            provider_error("CDSE_ODATA_HTTP_FAILURE", None),
            provider_error("CDSE_ODATA_HTTP_RATE_LIMITED", None),
            provider_error("CDSE_ODATA_HTTP_SERVER", None),
            provider_error("PLANETARY_COMPUTER_TRANSPORT_FAILED", None),
            provider_error("PLANETARY_COMPUTER_HTTP_REJECTED", Some(429)),
            provider_error("PLANETARY_COMPUTER_HTTP_REJECTED", Some(503)),
            provider_error("USGS_M2M_RATE_LIMITED", None),
            provider_error("USGS_M2M_TRANSPORT_FAILED", None),
            provider_error("USGS_M2M_TIMEOUT", None),
            provider_error("USGS_M2M_REMOTE_ERROR", Some(502)),
            provider_error("STAC_TIMEOUT", None),
            provider_error("STAC_NETWORK_ERROR", None),
            provider_error("STAC_RATE_LIMITED", None),
            provider_error("STAC_SERVER_ERROR", None),
        ] {
            assert!(
                catalogue_provider_error_allows_fallback(&error),
                "{}",
                error.code
            );
        }
    }

    #[test]
    fn cancel_auth_identity_and_bad_data_never_allow_source_fallback() {
        for error in [
            provider_error("CDSE_ODATA_CANCELLED", None),
            provider_error("CDSE_ODATA_HTTP_UNAUTHORIZED", None),
            provider_error("CDSE_ODATA_HTTP_FORBIDDEN", None),
            provider_error("CDSE_ODATA_HTTP_REDIRECT", None),
            provider_error("CDSE_ODATA_UUID_CONFLICT", None),
            provider_error("CDSE_ODATA_PAGE_INVALID", None),
            provider_error("CATALOGUE_REQUEST_CANCELLED", None),
            provider_error("PLANETARY_COMPUTER_CANCELLED", None),
            provider_error("PLANETARY_COMPUTER_HTTP_REJECTED", Some(403)),
            provider_error("PLANETARY_COMPUTER_RESPONSE_INVALID", None),
            provider_error("PLANETARY_COMPUTER_IDENTITY_MISMATCH", None),
            provider_error("USGS_M2M_AUTH_REQUIRED", None),
            provider_error("USGS_M2M_AUTH_INVALID", None),
            provider_error("USGS_M2M_RESPONSE_INVALID", None),
            provider_error("USGS_M2M_DATASET_IDENTITY_MISMATCH", None),
            provider_error("USGS_M2M_REMOTE_ERROR", Some(400)),
            provider_error("STAC_SEARCH_CANCELLED", None),
            provider_error("STAC_REDIRECT_REJECTED", None),
            provider_error("STAC_ACCESS_DENIED", None),
            provider_error("STAC_RESPONSE_INVALID", None),
            provider_error("STAC_IDENTITY_MISMATCH", None),
            provider_error("STAC_QUERY_INVALID", None),
            provider_error("STAC_LIMIT_EXCEEDED", None),
            provider_error("EARTH_SEARCH_LANDSAT_IDENTITY_MISMATCH", None),
            provider_error("EARTH_SEARCH_LANDSAT_RESPONSE_INVALID", None),
        ] {
            assert!(
                !catalogue_provider_error_allows_fallback(&error),
                "{}",
                error.code
            );
        }
    }
}

#[cfg(test)]
#[path = "earth_search_fallback_tests.rs"]
mod earth_search_fallback_tests;

fn reviewed_mpc_class_for_query(
    query: &CatalogSearchQuery,
) -> Option<PlanetaryComputerReviewedAssetClass> {
    match (
        query.logical_dataset.as_str(),
        query.provider_id.as_str(),
        query.native_collection_id.as_str(),
    ) {
        (
            MPC_SENTINEL1_GRD_LOGICAL_DATASET,
            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
            MPC_SENTINEL1_GRD_COLLECTION_ID,
        ) => Some(PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog),
        (
            MPC_COPDEM_GLO30_LOGICAL_DATASET,
            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
            MPC_COPDEM_GLO30_COLLECTION_ID,
        ) => Some(PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog),
        _ => None,
    }
}

fn validate_complete_catalogue_response(
    response: &CatalogSearchResponse,
    query: &CatalogSearchQuery,
    route_use: RouteUse,
    live_acceptance_candidate: bool,
) -> Result<(), ApplicationServiceError> {
    response.validate().map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_RESPONSE_INVALID",
            "目录响应格式无效",
            "response_validate",
        )
    })?;
    validate_catalogue_collection(
        &response.collection,
        query,
        route_use,
        live_acceptance_candidate,
    )?;
    if query.logical_dataset == ALOS2_LOGICAL_DATASET {
        return validate_complete_alos2_catalogue_response(response, query);
    }
    if query.logical_dataset == NISAR_LOGICAL_DATASET {
        return validate_complete_nisar_catalogue_response(response, query);
    }
    if query.logical_dataset == BHOONIDHI_LOGICAL_DATASET {
        return validate_complete_bhoonidhi_catalogue_response(response, query);
    }
    let product_count = response.products.len() as u64;
    validate_catalogue_generic_stats(
        &response.collection,
        &response.stats,
        query,
        route_use,
        live_acceptance_candidate,
        product_count,
    )?;
    // A streamed search validates each product's identity at mapping time;
    // the whole-response entry keeps its own product loop unchanged.
    let (expected_source_kind, expected_product_kind) =
        expected_catalogue_product_kinds(query).ok_or_else(catalogue_route_disabled)?;
    if response.products.iter().any(|product| {
        product.provider_id != query.provider_id
            || product.source_kind != expected_source_kind
            || product.product_kind != expected_product_kind
    }) {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INVALID",
            "目录产品身份或类型无效",
            "product_identity",
        ));
    }
    Ok(())
}

/// #22 B slice 3(b): validate a streamed search's stats-only result. The
/// products were validated one-by-one at mapping time and are no longer held;
/// the collection and statistics checks are exactly the whole-response ones,
/// with the mapped candidate count taking the place of the products length.
fn validate_catalogue_streamed_stats(
    collection: &CatalogCollectionDescriptor,
    stats: &CatalogSearchStats,
    query: &CatalogSearchQuery,
    route_use: RouteUse,
    live_acceptance_candidate: bool,
    mapped_candidate_count: u64,
    fixture: Option<&StacOfflineFixtureAdmission>,
) -> Result<(), ApplicationServiceError> {
    stats.validate().map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_RESPONSE_INVALID",
            "目录统计格式无效",
            "stats_validate",
        )
    })?;
    if let Some(admission) = fixture.filter(|a| a.targets(query)) {
        // #22 B slice 3(b) review fix 7: run the admission's stats-only
        // validation — the same checks validate_response applies to a whole
        // response, with the streamed candidate count taking the products
        // length's place. The per-product identity and duplicate checks ran
        // on each streamed product at mapping time.
        admission
            .validate_response_stats(
                query,
                collection,
                stats,
                mapped_candidate_count,
                RouteUse::OfflineFixture,
            )
            .map_err(|_| {
                catalogue_provider_error(
                    "CATALOGUE_RESPONSE_INCOMPLETE",
                    "目录响应必须完整且无错误",
                    "streamed_fixture_completeness",
                )
            })?;
        return Ok(());
    }
    // ASF and Bhoonidhi routes keep the whole-response entry; a streamed
    // outcome for them is a routing error, not a silently weaker check.
    if query.logical_dataset == ALOS2_LOGICAL_DATASET
        || query.logical_dataset == NISAR_LOGICAL_DATASET
        || query.logical_dataset == BHOONIDHI_LOGICAL_DATASET
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INVALID",
            "该目录路线必须整体校验后冻结",
            "streamed_unsupported_route",
        ));
    }
    validate_catalogue_generic_stats(
        collection,
        stats,
        query,
        route_use,
        live_acceptance_candidate,
        mapped_candidate_count,
    )?;
    // The per-product identity check ran inside the streaming mapper.
    expected_catalogue_product_kinds(query).ok_or_else(catalogue_route_disabled)?;
    Ok(())
}

fn catalogue_route_disabled() -> ApplicationServiceError {
    catalogue_provider_error(
        "CATALOGUE_ROUTE_DISABLED",
        "Sentinel-2 分析波段不允许通过通用 reviewed MPC 检索入口获取执行权",
        "sentinel2_band_search_forbidden",
    )
}

/// The response collection must match the reviewed route/profile exactly,
/// for whole-response and streamed outcomes alike.
fn validate_catalogue_collection(
    collection: &CatalogCollectionDescriptor,
    query: &CatalogSearchQuery,
    route_use: RouteUse,
    live_acceptance_candidate: bool,
) -> Result<(), ApplicationServiceError> {
    let route = resolve_catalogue_route(
        &query.logical_dataset,
        &query.provider_id,
        &query.native_collection_id,
        route_use,
        live_acceptance_candidate,
    )
    .map_err(|_| {
        catalogue_provider_error("CATALOGUE_RESPONSE_INVALID", "目录响应路由无效", "route")
    })?;
    let profile = CollectionProfileRegistry
        .get(route.collection_profile_id)
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_RESPONSE_INVALID",
                "目录响应集合档案无效",
                "profile",
            )
        })?;
    if collection.logical_dataset != query.logical_dataset
        || collection.provider_id != query.provider_id
        || collection.native_collection_id != query.native_collection_id
        || collection.collection_profile_id != profile.profile_id
        || collection.identity_rule_id != profile.identity_rule_id
        || collection.filename_rule_id != profile.filename_rule_id
        || collection.delivery_model != CatalogDeliveryModel::ArchiveFirst
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INVALID",
            "目录响应集合与请求不匹配",
            "collection",
        ));
    }
    let production_invalid = matches!(route_use, RouteUse::Production)
        && if live_acceptance_candidate {
            !collection.production_enabled
        } else {
            !route.production_enabled
                || !profile.production_enabled
                || !collection.production_enabled
        };
    if production_invalid {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INVALID",
            "目录响应集合未启用生产能力",
            "production_enabled",
        ));
    }
    Ok(())
}

/// The shared collection/statistics contract of a complete catalogue result.
/// `product_count` is the number of mapped products the caller holds (the
/// whole response's products, or a streamed search's staged candidates).
fn validate_catalogue_generic_stats(
    collection: &CatalogCollectionDescriptor,
    stats: &CatalogSearchStats,
    query: &CatalogSearchQuery,
    route_use: RouteUse,
    live_acceptance_candidate: bool,
    product_count: u64,
) -> Result<(), ApplicationServiceError> {
    validate_catalogue_collection(collection, query, route_use, live_acceptance_candidate)?;
    let is_landsat = query.logical_dataset == LANDSAT_C2_L2_LOGICAL_DATASET;
    let is_mpc_sentinel2 = query.logical_dataset == "optical.sentinel-2.l2a"
        && query.provider_id == insar_provider::catalog::MPC_SENTINEL2_L2A_PROVIDER_ID
        && query.native_collection_id == insar_provider::catalog::MPC_SENTINEL2_L2A_COLLECTION_ID;
    let is_earth_search_sentinel2 = query.logical_dataset == "optical.sentinel-2.l2a"
        && query.provider_id == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
        && query.native_collection_id
            == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID;
    let is_compact_cdse = query.provider_id == "copernicus.dataspace.odata"
        && matches!(
            query.native_collection_id.as_str(),
            "SENTINEL-2" | "SENTINEL-3"
        )
        && insar_provider::cdse::query::requires_compact_aoi_transport(query, RouteUse::Production);
    let reviewed_mpc_class = reviewed_mpc_class_for_query(query);
    let hls_product = hls_catalogue::hls_product_for_query(query);
    let completion_matches_intent = query
        .result_limit
        .map(|limit| stats.returned_count == limit || stats.remote_exhausted)
        .unwrap_or(stats.remote_exhausted);
    if product_count > MAX_CATALOGUE_PRODUCTS as u64
        || stats.fetched_record_count > MAX_CATALOGUE_CANDIDATE_RECORDS
        || stats.truncated
        || stats.requested_limit != query.result_limit
        || stats.returned_count != product_count
        || !stats.working_set_satisfied
        || !completion_matches_intent
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INCOMPLETE",
            "目录响应必须完整且无错误",
            "completeness",
        ));
    }
    // Reviewed STAC channels may omit a remote total. A finite working set or
    // an exhausted continuation still proves completion; neither invents a total.
    // A compact CDSE AOI has overlapping transport windows: their counts are
    // not an exact AOI total. Ordinary CDSE queries still require a remote total.
    let optional_remote_total = (is_landsat
        && matches!(
            query.provider_id.as_str(),
            insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
                | insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID
        ))
        || is_mpc_sentinel2
        || is_earth_search_sentinel2
        || reviewed_mpc_class.is_some()
        || hls_product.is_some()
        || is_compact_cdse;
    let remote_counts_valid = match stats.matched_count {
        Some(matched) => {
            matched >= stats.fetched_record_count
                && (!stats.remote_exhausted || matched == stats.fetched_record_count)
        }
        None => optional_remote_total,
    };
    let valid_counts = remote_counts_valid
        && (!is_compact_cdse
            || (stats.matched_count.is_none()
                && (1..=8).contains(&stats.transport_part_count)
                && stats.request_count >= stats.transport_part_count
                && stats.request_count <= 128))
        && stats.mapped_record_count == stats.fetched_record_count
        && stats
            .unique_provider_product_count
            .checked_add(stats.exact_duplicate_record_count)
            == Some(stats.mapped_record_count)
        && stats.exact_matched_count <= stats.unique_provider_product_count
        && stats.returned_count <= stats.exact_matched_count
        && (is_landsat
            || is_mpc_sentinel2
            || is_earth_search_sentinel2
            || is_compact_cdse
            || stats.exact_duplicate_record_count == 0);
    if !valid_counts {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INCOMPLETE",
            "目录响应必须完整且无错误",
            "counts",
        ));
    }
    Ok(())
}

/// The expected (source kind, product kind) pair of a route's products, or
/// `None` when the route is not searchable (reviewed Sentinel-2 bands).
fn expected_catalogue_product_kinds(
    query: &CatalogSearchQuery,
) -> Option<(DataSourceKind, ProductKind)> {
    let is_landsat = query.logical_dataset == LANDSAT_C2_L2_LOGICAL_DATASET;
    let reviewed_mpc_class = reviewed_mpc_class_for_query(query);
    let hls_product = hls_catalogue::hls_product_for_query(query);
    match reviewed_mpc_class {
        Some(PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog) => {
            Some((DataSourceKind::Sentinel1, ProductKind::SarGrd))
        }
        Some(PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog) => {
            Some((DataSourceKind::Dem, ProductKind::DemRaster))
        }
        Some(PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog) => None,
        None if is_landsat => Some((DataSourceKind::Landsat, ProductKind::OpticalScene)),
        None => match hls_product {
            Some(hls) => Some((hls.source_kind(), ProductKind::OpticalScene)),
            None => Some((DataSourceKind::Sentinel2, ProductKind::OpticalScene)),
        },
    }
}

fn validate_complete_alos2_catalogue_response(
    response: &CatalogSearchResponse,
    query: &CatalogSearchQuery,
) -> Result<(), ApplicationServiceError> {
    let stats = &response.stats;
    let product_count = response.products.len() as u64;
    if response.products.len() > MAX_CATALOGUE_PRODUCTS
        || stats.fetched_record_count > MAX_CATALOGUE_CANDIDATE_RECORDS
        || stats.unique_provider_product_count > MAX_CATALOGUE_CANDIDATE_RECORDS
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESULT_LIMIT_EXCEEDED",
            "ALOS-2 目录响应超过冻结上限",
            "result_limit",
        ));
    }
    let counts_are_complete = stats.requested_limit == query.result_limit
        && stats.request_count > 0
        && stats.transport_part_count > 0
        && stats.mapped_record_count == stats.fetched_record_count
        && stats
            .unique_provider_product_count
            .checked_add(stats.exact_duplicate_record_count)
            == Some(stats.mapped_record_count)
        && stats.exact_matched_count <= stats.unique_provider_product_count
        && stats.returned_count == product_count
        && product_count <= stats.exact_matched_count
        && stats.working_set_satisfied
        && query
            .result_limit
            .map(|limit| product_count == limit || stats.remote_exhausted)
            .unwrap_or(stats.remote_exhausted);
    if response.next_cursor.is_some()
        || stats.truncated
        || !response.item_errors.is_empty()
        || !counts_are_complete
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INCOMPLETE",
            "ALOS-2 目录响应必须完整且无错误",
            "completeness",
        ));
    }
    if response.products.iter().any(|product| {
        product.provider_id != query.provider_id
            || product.source_kind != DataSourceKind::Sar
            || product.product_kind != ProductKind::SarProduct
    }) {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INVALID",
            "ALOS-2 目录产品身份或类型无效",
            "product_identity",
        ));
    }
    Ok(())
}

fn validate_complete_nisar_catalogue_response(
    response: &CatalogSearchResponse,
    query: &CatalogSearchQuery,
) -> Result<(), ApplicationServiceError> {
    let stats = &response.stats;
    let product_count = response.products.len() as u64;
    if response.products.len() > MAX_CATALOGUE_PRODUCTS
        || stats.fetched_record_count > MAX_CATALOGUE_CANDIDATE_RECORDS
        || stats.unique_provider_product_count > MAX_CATALOGUE_CANDIDATE_RECORDS
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESULT_LIMIT_EXCEEDED",
            "NISAR 目录响应超过冻结上限",
            "result_limit",
        ));
    }
    let item_errors_are_acceptable = response
        .item_errors
        .iter()
        .all(|error| error.code == "ASF_PRODUCT_SCOPE_MISMATCH");
    let counts_are_complete = stats.requested_limit == query.result_limit
        && stats.request_count > 0
        && stats.transport_part_count > 0
        && stats
            .mapped_record_count
            .checked_add(stats.profile_filtered_record_count)
            .and_then(|count| count.checked_add(response.item_errors.len() as u64))
            == Some(stats.fetched_record_count)
        && stats
            .unique_provider_product_count
            .checked_add(stats.exact_duplicate_record_count)
            == Some(stats.mapped_record_count)
        && stats.exact_matched_count <= stats.unique_provider_product_count
        && stats.returned_count == product_count
        && product_count <= stats.exact_matched_count
        && stats.working_set_satisfied
        && query
            .result_limit
            .map(|limit| product_count == limit || stats.remote_exhausted)
            .unwrap_or(stats.remote_exhausted);
    if response.next_cursor.is_some()
        || stats.truncated
        || !item_errors_are_acceptable
        || !counts_are_complete
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INCOMPLETE",
            "NISAR 目录响应必须完整且仅允许范围过滤错误",
            "completeness",
        ));
    }
    if response.products.iter().any(|product| {
        product.provider_id != query.provider_id
            || product.source_kind != DataSourceKind::Sar
            || product.product_kind != ProductKind::SarProduct
    }) {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INVALID",
            "NISAR 目录产品身份或类型无效",
            "product_identity",
        ));
    }
    Ok(())
}

fn validate_complete_bhoonidhi_catalogue_response(
    response: &CatalogSearchResponse,
    query: &CatalogSearchQuery,
) -> Result<(), ApplicationServiceError> {
    let stats = &response.stats;
    let product_count = response.products.len() as u64;
    if response.products.len() > MAX_CATALOGUE_PRODUCTS
        || stats.fetched_record_count > MAX_CATALOGUE_CANDIDATE_RECORDS
        || stats.unique_provider_product_count > MAX_CATALOGUE_CANDIDATE_RECORDS
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESULT_LIMIT_EXCEEDED",
            "Bhoonidhi S-RSLC 目录响应超过冻结上限",
            "result_limit",
        ));
    }
    let counts_are_complete = stats.requested_limit == query.result_limit
        && stats.request_count > 0
        && stats.transport_part_count > 0
        && stats.mapped_record_count == stats.fetched_record_count
        && stats
            .unique_provider_product_count
            .checked_add(stats.exact_duplicate_record_count)
            == Some(stats.mapped_record_count)
        && stats.exact_matched_count <= stats.unique_provider_product_count
        && stats.returned_count == product_count
        && product_count <= stats.exact_matched_count
        && stats.working_set_satisfied
        && query
            .result_limit
            .map(|limit| product_count == limit || stats.remote_exhausted)
            .unwrap_or(stats.remote_exhausted);
    if response.next_cursor.is_some()
        || stats.truncated
        || !response.item_errors.is_empty()
        || !counts_are_complete
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INCOMPLETE",
            "Bhoonidhi S-RSLC 目录响应必须完整且无错误",
            "completeness",
        ));
    }
    if response.products.iter().any(|product| {
        product.provider_id != BHOONIDHI_PROVIDER_ID
            || product.source_kind != DataSourceKind::Sar
            || product.product_kind != ProductKind::SarSlc
    }) {
        return Err(catalogue_provider_error(
            "CATALOGUE_RESPONSE_INVALID",
            "Bhoonidhi S-RSLC 目录产品身份或类型无效",
            "product_identity",
        ));
    }
    Ok(())
}

fn valid_exact_text(value: &str) -> bool {
    !value.is_empty() && value == value.trim() && !value.chars().any(char::is_control)
}

fn parse_normalized_utc(value: &str) -> Option<DateTime<FixedOffset>> {
    if !valid_exact_text(value) {
        return None;
    }
    let parsed = DateTime::parse_from_rfc3339(value).ok()?;
    (parsed.offset().local_minus_utc() == 0
        && parsed.to_rfc3339_opts(SecondsFormat::AutoSi, true) == value)
        .then_some(parsed)
}

fn parse_catalogue_utc(value: &str) -> Option<DateTime<FixedOffset>> {
    if !valid_exact_text(value) {
        return None;
    }
    let parsed = DateTime::parse_from_rfc3339(value).ok()?;
    (parsed.offset().local_minus_utc() == 0).then_some(parsed)
}

fn canonical_odata_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .map(|parsed| parsed.hyphenated().to_string().eq_ignore_ascii_case(value))
        .unwrap_or(false)
}

fn valid_catalogue_display_name(value: &str) -> bool {
    const DISALLOWED_FRAGMENTS: &[&str] = &[
        "http",
        "token",
        "account",
        "credential",
        "password",
        "authorization",
        "cookie",
        "bearer",
        "secret",
        "path",
    ];
    let lowercase = value.to_ascii_lowercase();
    valid_exact_text(value)
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        && !DISALLOWED_FRAGMENTS
            .iter()
            .any(|fragment| lowercase.contains(fragment))
}

fn freeze_catalogue_checksums(value: &Value) -> Result<Value, ApplicationServiceError> {
    let checksums = value
        .as_array()
        .filter(|values| !values.is_empty() && values.len() <= 2)
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "目录产品校验和无效",
                "checksums",
            )
        })?;
    let mut ordered = BTreeMap::new();
    for checksum in checksums {
        let Some(checksum) = checksum.as_object() else {
            return Err(catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "目录产品校验和无效",
                "checksums",
            ));
        };
        let (Some(algorithm), Some(value)) = (
            checksum.get("algorithm").and_then(Value::as_str),
            checksum.get("value").and_then(Value::as_str),
        ) else {
            return Err(catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "目录产品校验和无效",
                "checksums",
            ));
        };
        let expected_hex_length = match algorithm {
            "MD5" => 32,
            "SHA256" => 64,
            _ => {
                return Err(catalogue_provider_error(
                    "CATALOGUE_PRODUCT_INVALID",
                    "目录产品校验和算法无效",
                    "checksums",
                ))
            }
        };
        if checksum.len() != 2
            || value.len() != expected_hex_length
            || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            || ordered.insert(algorithm, value.to_owned()).is_some()
        {
            return Err(catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "目录产品校验和无效",
                "checksums",
            ));
        }
    }
    Ok(Value::Array(
        ordered
            .into_iter()
            .map(|(algorithm, value)| {
                Value::Object(Map::from_iter([
                    ("algorithm".to_owned(), Value::String(algorithm.to_owned())),
                    ("value".to_owned(), Value::String(value)),
                ]))
            })
            .collect(),
    ))
}

fn freeze_catalogue_properties(
    properties: &Map<String, Value>,
    acquisition_datetime: &DateTime<FixedOffset>,
) -> Result<Map<String, Value>, ApplicationServiceError> {
    validate_debug_context(properties).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录产品属性包含敏感字段",
            "sensitive_properties",
        )
    })?;
    let mut frozen = Map::new();
    for (key, value) in properties {
        match key.as_str() {
            "product_type" => match value.as_str() {
                Some(
                    "S2MSI2A" | "S2MSI1C" | "OL_1_EFR___" | "OL_1_ERR___" | "OL_2_LFR___"
                    | "OL_2_LRR___" | "SL_1_RBT___" | "SL_2_LST___" | "SR_1_SRA___" | "SR_2_LAN___"
                    | "OL_1_EFR",
                ) => {
                    frozen.insert(key.clone(), value.clone());
                }
                _ => {
                    return Err(catalogue_provider_error(
                        "CATALOGUE_PRODUCT_INVALID",
                        "目录产品类型无效",
                        "product_type",
                    ))
                }
            },
            "online" if value.is_boolean() => {
                frozen.insert(key.clone(), value.clone());
            }
            "content_date_end" => match value.as_str().and_then(parse_normalized_utc) {
                Some(content_date_end) if content_date_end >= *acquisition_datetime => {
                    frozen.insert(key.clone(), value.clone());
                }
                _ => {
                    return Err(catalogue_provider_error(
                        "CATALOGUE_PRODUCT_INVALID",
                        "目录产品结束时间无效",
                        "content_date_end",
                    ))
                }
            },
            "cloud_cover"
                if value
                    .as_f64()
                    .is_some_and(|value| value.is_finite() && (0.0..=100.0).contains(&value)) =>
            {
                frozen.insert(key.clone(), value.clone());
            }
            "checksums" => {
                frozen.insert(key.clone(), freeze_catalogue_checksums(value)?);
            }
            // CDSE may expose S3Path in an OData record. It is provider-only
            // locator metadata, never identity or frozen task evidence, so it
            // must not prevent a valid archive from freezing or reach Store.
            "s3_path" => {}
            _ => {
                return Err(catalogue_provider_error(
                    "CATALOGUE_PRODUCT_INVALID",
                    "目录产品包含未批准的属性",
                    "properties",
                ))
            }
        }
    }
    if !frozen.contains_key("product_type") || !frozen.contains_key("online") {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录产品缺少必需属性",
            "properties",
        ));
    }
    Ok(frozen)
}

struct FrozenReviewedMpcSourceEvidence {
    source_batch: FrozenSearchBatch,
    page_size: u64,
    source_item: SnapshotItem,
    class: PlanetaryComputerReviewedAssetClass,
    provider_product_id: String,
}

fn frozen_reviewed_mpc_source_evidence(
    batch: &FrozenSearchBatch,
    page_size: u64,
    item: &SnapshotItem,
) -> Result<FrozenReviewedMpcSourceEvidence, ApplicationServiceError> {
    let profile_id = item
        .metadata
        .get("collection_profile_id")
        .and_then(Value::as_str)
        .ok_or(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
    let class = match profile_id {
        MPC_SENTINEL1_GRD_PROFILE_ID => PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
        MPC_COPDEM_GLO30_PROFILE_ID => PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
        _ => return Err(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid),
    };
    let provider_product_id = item
        .metadata
        .get("provider_product_id")
        .and_then(Value::as_str)
        .filter(|value| valid_exact_text(value))
        .ok_or(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
    let expected_product_key = provider_product_key(
        PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
        class.collection_id(),
        provider_product_id,
    )
    .map_err(|_| ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
    let source_kind = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => "sentinel1",
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => "dem",
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
            return Err(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid);
        }
    };
    let product_kind = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => "sar_grd",
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => "dem_raster",
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
            return Err(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid);
        }
    };
    let assets = item
        .metadata
        .get("assets")
        .and_then(Value::as_array)
        .filter(|assets| !assets.is_empty() && assets.len() <= 16)
        .ok_or(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;

    if page_size == 0
        || batch.validate().is_err()
        || item.validate().is_err()
        || batch.provider_id != PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID
        || item.provider_id != PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID
        || item.item_id != expected_product_key
        || item.display_name != provider_product_id
        || item.asset_access.is_some()
        || item.execution_policy.is_some()
        || item.metadata.contains_key("offline_fixture_only")
        || item.metadata.get("logical_dataset").and_then(Value::as_str)
            != Some(class.logical_dataset())
        || item
            .metadata
            .get("native_collection_id")
            .and_then(Value::as_str)
            != Some(class.collection_id())
        || item
            .metadata
            .get("identity_rule_id")
            .and_then(Value::as_str)
            != Some(MPC_REVIEWED_STAC_IDENTITY_RULE_ID)
        || item.metadata.get("source_kind").and_then(Value::as_str) != Some(source_kind)
        || item.metadata.get("product_kind").and_then(Value::as_str) != Some(product_kind)
        || item
            .metadata
            .get("provider_product_key")
            .and_then(Value::as_str)
            != Some(expected_product_key.as_str())
        || item.metadata.get("stac_item_id").and_then(Value::as_str) != Some(provider_product_id)
    {
        return Err(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid);
    }

    let mut seen = BTreeSet::new();
    for asset in assets {
        let asset = asset
            .as_object()
            .ok_or(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
        let asset_key = asset
            .get("asset_key")
            .and_then(Value::as_str)
            .filter(|value| valid_exact_text(value))
            .ok_or(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
        let expected_stable_key = stable_asset_key(&expected_product_key, asset_key)
            .map_err(|_| ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid)?;
        if !seen.insert(asset_key)
            || asset.get("stable_asset_key").and_then(Value::as_str)
                != Some(expected_stable_key.as_str())
            || asset.contains_key("href")
            || asset.contains_key("url")
            || asset.contains_key("access_descriptor")
        {
            return Err(ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid);
        }
    }

    Ok(FrozenReviewedMpcSourceEvidence {
        source_batch: batch.clone(),
        page_size,
        source_item: item.clone(),
        class,
        provider_product_id: provider_product_id.to_owned(),
    })
}

fn reviewed_mpc_asset_keys_allowed(
    source: &FrozenReviewedMpcSourceEvidence,
    asset_keys: &[String],
) -> bool {
    let class_allowed = match source.class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => asset_keys
            .iter()
            .all(|key| matches!(key.as_str(), "vv" | "vh" | "hh" | "hv")),
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => {
            asset_keys.len() == 1 && asset_keys[0] == "data"
        }
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => false,
    };
    if !class_allowed {
        return false;
    }
    let Some(assets) = source
        .source_item
        .metadata
        .get("assets")
        .and_then(Value::as_array)
    else {
        return false;
    };
    asset_keys.iter().all(|selected| {
        assets.iter().any(|asset| {
            asset.as_object().is_some_and(|asset| {
                asset.get("asset_key").and_then(Value::as_str) == Some(selected.as_str())
                    && asset.get("role").and_then(Value::as_str) == Some("primary")
            })
        })
    })
}

struct FrozenLandsatBrowseEvidence {
    query: CatalogSearchQuery,
    entity_id: String,
}

struct FrozenLandsatBundleEvidence {
    source_batch: FrozenSearchBatch,
    page_size: u64,
    dataset_alias: String,
    entity_id: String,
    display_id: String,
}

fn landsat_product_output_root(selected_root: &Path, display_id: &str) -> PathBuf {
    if selected_root
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == display_id)
    {
        selected_root.to_path_buf()
    } else {
        selected_root.join(display_id)
    }
}

fn frozen_landsat_bundle_evidence(
    batch: &FrozenSearchBatch,
    page_size: u64,
    item: &SnapshotItem,
) -> Result<FrozenLandsatBundleEvidence, ApplicationServiceError> {
    if item.provider_id != LANDSAT_C2_L2_PROVIDER_ID {
        return Err(ApplicationServiceError::LandsatBundleEvidenceInvalid);
    }
    frozen_landsat_archive_source_evidence(batch, page_size, item)
}

pub(crate) fn frozen_landsat_scene_evidence(
    batch: &FrozenSearchBatch,
    page_size: u64,
    item: &SnapshotItem,
) -> Result<FrozenLandsatBundleEvidence, ApplicationServiceError> {
    // Public anonymous asset delivery belongs only to Planetary Computer.
    // A search-only catalogue route must never gain this provider's manifest
    // authority merely because its Landsat identity happens to match.
    if item.provider_id != insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID {
        return Err(ApplicationServiceError::LandsatBundleEvidenceInvalid);
    }
    frozen_landsat_archive_source_evidence(batch, page_size, item)
}

fn frozen_landsat_archive_source_evidence(
    batch: &FrozenSearchBatch,
    page_size: u64,
    item: &SnapshotItem,
) -> Result<FrozenLandsatBundleEvidence, ApplicationServiceError> {
    // Official-archive intent may re-resolve a scientifically identical
    // Landsat scene from MPC or Earth Search to the reviewed USGS route.
    if !matches!(
        item.provider_id.as_str(),
        LANDSAT_C2_L2_PROVIDER_ID
            | insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
            | insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID
    ) {
        return Err(ApplicationServiceError::LandsatBundleEvidenceInvalid);
    }
    let properties = item
        .metadata
        .get("properties")
        .and_then(Value::as_object)
        .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
    let entity_id = properties
        .get("entity_id")
        .and_then(Value::as_str)
        .filter(|value| valid_landsat_identifier(value))
        .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
    let display_id = properties
        .get("landsat_product_id")
        .and_then(Value::as_str)
        .filter(|value| valid_landsat_identifier(value))
        .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
    let dataset_alias = properties
        .get("dataset_alias")
        .and_then(Value::as_str)
        .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
    let query: CatalogSearchQuery = serde_json::from_value(Value::Object(batch.query.clone()))
        .map_err(|_| ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
    let profile = CollectionProfileRegistry
        .for_source(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            &item.provider_id,
            dataset_alias,
        )
        .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
    let expected_item_id =
        provider_product_key(profile.provider_id, profile.native_collection_id, entity_id)
            .map_err(|_| ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
    if page_size == 0
        || batch.validate().is_err()
        || item.validate().is_err()
        || item.asset_access.is_some()
        || item.execution_policy.is_some()
        || query.validate().is_err()
        || query.logical_dataset != LANDSAT_C2_L2_LOGICAL_DATASET
        || query.provider_id != profile.provider_id
        || query.native_collection_id != profile.native_collection_id
        || query.aoi_geojson != batch.aoi_geojson
        || batch.provider_id != profile.provider_id
        || item.provider_id != profile.provider_id
        || item.item_id != expected_item_id
        || item.display_name != display_id
        || dataset_alias != profile.native_collection_id
        || item.metadata.get("logical_dataset").and_then(Value::as_str)
            != Some(profile.logical_dataset)
        || item
            .metadata
            .get("native_collection_id")
            .and_then(Value::as_str)
            != Some(profile.native_collection_id)
        || item
            .metadata
            .get("collection_profile_id")
            .and_then(Value::as_str)
            != Some(profile.profile_id)
        || item
            .metadata
            .get("identity_rule_id")
            .and_then(Value::as_str)
            != Some(profile.identity_rule_id)
        || item
            .metadata
            .get("filename_rule_id")
            .and_then(Value::as_str)
            .is_none_or(|value| {
                value != profile.filename_rule_id
                    && !(profile.provider_id == LANDSAT_C2_L2_PROVIDER_ID
                        && value == USGS_LANDSAT_C2_L2_LEGACY_SCENE_FILENAME_RULE_ID)
            })
        || item
            .metadata
            .get("provider_product_id")
            .and_then(Value::as_str)
            != Some(entity_id)
        || item
            .metadata
            .get("provider_product_key")
            .and_then(Value::as_str)
            != Some(item.item_id.as_str())
    {
        return Err(ApplicationServiceError::LandsatBundleEvidenceInvalid);
    }
    if matches!(
        profile.provider_id,
        insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
            | insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID
    ) {
        insar_provider::planetary_computer::validate_planetary_computer_landsat_source_id(
            entity_id, display_id,
        )
        .map_err(|_| ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
    }
    Ok(FrozenLandsatBundleEvidence {
        source_batch: batch.clone(),
        page_size,
        dataset_alias: dataset_alias.to_owned(),
        entity_id: entity_id.to_owned(),
        display_id: display_id.to_owned(),
    })
}

fn project_landsat_asset_details(
    item: &SnapshotItem,
    evidence: &FrozenLandsatBundleEvidence,
    manifest: PlanetaryComputerLandsatManifest,
) -> Result<LandsatAssetDetailsResponse, ApplicationServiceError> {
    let expected_processing_level = item
        .metadata
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get("processing_level"))
        .and_then(Value::as_str)
        .ok_or(ApplicationServiceError::LandsatAssetEvidenceInvalid)?;
    if manifest.entity_id != evidence.entity_id
        || manifest.display_id != evidence.display_id
        || manifest.stac_item_id != evidence.entity_id
        || !manifest
            .processing_level
            .eq_ignore_ascii_case(expected_processing_level)
        || manifest.assets.is_empty()
        || manifest.assets.len() > 64
    {
        return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
    }

    let mut seen_keys = BTreeSet::new();
    let mut projected = Vec::with_capacity(manifest.assets.len());
    let mut total_size_bytes = 0_u64;
    for asset in manifest.assets {
        if !valid_exact_text(&asset.asset_key)
            || asset.asset_key.len() > 96
            || !valid_exact_text(&asset.file_name)
            || asset.file_name.len() > 240
            || asset.expected_size_bytes == 0
            || !seen_keys.insert(asset.asset_key.clone())
        {
            return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
        }
        let role = match asset.role {
            AssetRole::Primary => "primary",
            AssetRole::Auxiliary => "auxiliary",
            AssetRole::Metadata => "metadata",
            _ => return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid),
        };
        total_size_bytes = total_size_bytes
            .checked_add(asset.expected_size_bytes)
            .ok_or(ApplicationServiceError::LandsatAssetEvidenceInvalid)?;
        projected.push(LandsatAssetDetail {
            asset_key: asset.asset_key,
            file_name: asset.file_name,
            role: role.to_owned(),
            size_bytes: asset.expected_size_bytes,
        });
    }
    if total_size_bytes != manifest.total_size_bytes {
        return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
    }
    projected.sort_by(|left, right| left.asset_key.cmp(&right.asset_key));
    Ok(LandsatAssetDetailsResponse {
        product_key: item.item_id.clone(),
        display_id: evidence.display_id.clone(),
        processing_level: manifest.processing_level,
        total_size_bytes,
        assets: projected,
    })
}

fn resolved_cdse_static_quicklook(
    product_key: String,
    response: CdseQuicklookResponse,
) -> Result<ResolvedPreviewAsset, ApplicationServiceError> {
    resolved_runtime_discovered_preview(
        product_key,
        PreviewAssetRole::Preview,
        response.media_type,
        response.data_url,
        response.byte_length,
        "cdse.sentinel-2.static-quicklook.v1",
        "CDSE static quicklook; desktop-controlled Preview only",
    )
}

fn resolved_landsat_scene_browse(
    product_key: String,
    response: LandsatBrowseResponse,
) -> Result<ResolvedPreviewAsset, ApplicationServiceError> {
    resolved_runtime_discovered_preview(
        product_key,
        PreviewAssetRole::Browse,
        response.media_type,
        response.data_url,
        response.byte_length,
        "planetary-computer.landsat-c2-l2.exact-scene-browse.v1",
        "Microsoft Planetary Computer Landsat scene Browse; desktop-controlled Preview only",
    )
}

fn resolved_runtime_discovered_preview(
    product_key: String,
    source_asset_role: PreviewAssetRole,
    media_type: String,
    data_url: String,
    byte_length: usize,
    render_profile_id: &'static str,
    provenance: &'static str,
) -> Result<ResolvedPreviewAsset, ApplicationServiceError> {
    let source_asset_key = format!("{product_key}::PREVIEW_DISCOVERY::{render_profile_id}");
    let descriptor = PreviewDescriptor {
        preview_kind: PreviewKind::Quicklook,
        source_asset_keys: vec![source_asset_key.clone()],
        render_profile_id: render_profile_id.to_owned(),
        media_type: media_type.clone(),
        credential_requirement: PreviewCredentialRequirement::None,
        cache_policy: PreviewCachePolicy::NoStore,
        provenance: provenance.to_owned(),
    };
    let resolved = ResolvedPreviewAsset {
        product_key,
        descriptor,
        source_asset_role,
        source_asset_key,
        media_type,
        data_url,
        byte_length: u64::try_from(byte_length)
            .map_err(|_| ApplicationServiceError::Preview(PreviewAssetError::DecodeFailed))?,
    };
    resolved
        .validate()
        .map_err(|_| ApplicationServiceError::Preview(PreviewAssetError::DecodeFailed))?;
    Ok(resolved)
}

/// Immutable preview authority loaded by batch and item identity. This source
/// is deliberately independent of the process-global mutable Workbench so a
/// historical task can open its product after a later search has been activated.
struct FrozenPreviewSource {
    batch: FrozenSearchBatch,
    item: SnapshotItem,
}

fn load_frozen_preview_source(
    store: &TaskStore,
    batch_id: &str,
    item_id: &str,
) -> Result<Option<FrozenPreviewSource>, StoreError> {
    // #22 B slice 2: preview and asset-detail projections need the batch
    // header and exactly one product, so the bounded selection read replaces
    // the full candidate array. The delivery flows that must compare or clone
    // the whole source batch re-read it in their write transaction.
    let selection = store.load_batch_selection(batch_id, &[item_id.to_owned()])?;
    let Some(selection) = selection else {
        return Ok(None);
    };
    let Some(item) = selection.items.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(FrozenPreviewSource {
        batch: selection.batch,
        item,
    }))
}

/// Project the reviewed HTTPS route from a frozen NISAR catalogue asset into
/// the provider-owned preview shape. NISAR's frozen asset graph intentionally
/// retains route and native-evidence objects instead of a transient `url`, so
/// preview discovery must not deserialize it as a live `ProviderAsset`.
fn frozen_nisar_preview_assets(
    item: &SnapshotItem,
) -> Result<Vec<ProviderAsset>, ApplicationServiceError> {
    let assets = item
        .metadata
        .get("assets")
        .and_then(Value::as_array)
        .ok_or(ApplicationServiceError::Preview(
            PreviewAssetError::ProviderNoPreview,
        ))?;
    let mut projected = Vec::new();
    for value in assets {
        let Some(asset) = value.as_object() else {
            return Err(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ));
        };
        if asset.get("role").and_then(Value::as_str) != Some("preview") {
            continue;
        }

        let asset_key = asset
            .get("asset_key")
            .and_then(Value::as_str)
            .filter(|value| safe_nisar_graph_text(value, NISAR_MAX_FILE_NAME_BYTES))
            .ok_or(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ))?;
        let stable_key = asset
            .get("stable_asset_key")
            .and_then(Value::as_str)
            .filter(|value| valid_exact_text(value))
            .ok_or(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ))?;
        let expected_stable_key = stable_asset_key(&item.item_id, asset_key)
            .map_err(|_| ApplicationServiceError::Preview(PreviewAssetError::ProviderNoPreview))?;
        if stable_key != expected_stable_key {
            return Err(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ));
        }

        let file_name = asset
            .get("file_name")
            .and_then(Value::as_str)
            .filter(|value| safe_nisar_basename(value))
            .ok_or(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ))?;
        let native_roles = asset
            .get("native_roles")
            .and_then(Value::as_array)
            .filter(|values| !values.is_empty() && values.len() <= NISAR_MAX_NATIVE_ROLES)
            .and_then(|values| {
                values
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .filter(|value| {
                                safe_nisar_graph_text(value, NISAR_MAX_NATIVE_ROLE_BYTES)
                            })
                            .map(str::to_owned)
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ))?;
        if !native_roles
            .iter()
            .any(|role| role == "GET RELATED VISUALIZATION")
        {
            return Err(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ));
        }

        let native_evidence =
            asset
                .get("native_evidence")
                .ok_or(ApplicationServiceError::Preview(
                    PreviewAssetError::ProviderNoPreview,
                ))?;
        if validate_nisar_native_evidence(native_evidence, file_name, &AssetRole::Preview)
            .map_err(|_| ApplicationServiceError::Preview(PreviewAssetError::ProviderNoPreview))?
        {
            return Err(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ));
        }
        let evidence = native_evidence
            .as_object()
            .ok_or(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ))?;
        let url_types = evidence.get("url_types").and_then(Value::as_array).ok_or(
            ApplicationServiceError::Preview(PreviewAssetError::ProviderNoPreview),
        )?;
        if url_types.len() != 1 || url_types[0].as_str() != Some("GET RELATED VISUALIZATION") {
            return Err(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ));
        }

        let routes = asset
            .get("routes")
            .and_then(Value::as_array)
            .filter(|values| !values.is_empty() && values.len() <= NISAR_MAX_RAW_RESOURCE_ENTRIES)
            .ok_or(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ))?;
        let mut route_keys = BTreeSet::new();
        let mut https_locator = None;
        for value in routes {
            let route = value
                .as_object()
                .filter(|route| exact_keys(route, &["scheme", "locator"]))
                .ok_or(ApplicationServiceError::Preview(
                    PreviewAssetError::ProviderNoPreview,
                ))?;
            let (Some(scheme), Some(locator)) = (
                route.get("scheme").and_then(Value::as_str),
                route.get("locator").and_then(Value::as_str),
            ) else {
                return Err(ApplicationServiceError::Preview(
                    PreviewAssetError::ProviderNoPreview,
                ));
            };
            if !nisar_route_matches_file(scheme, locator, file_name)
                || !route_keys.insert((scheme.to_owned(), locator.to_owned()))
            {
                return Err(ApplicationServiceError::Preview(
                    PreviewAssetError::ProviderNoPreview,
                ));
            }
            if scheme == "https" && https_locator.replace(locator.to_owned()).is_some() {
                return Err(ApplicationServiceError::Preview(
                    PreviewAssetError::ProviderNoPreview,
                ));
            }
        }
        let url = https_locator.ok_or(ApplicationServiceError::Preview(
            PreviewAssetError::ProviderNoPreview,
        ))?;
        let size_bytes = match asset.get("size_bytes") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_i64().filter(|value| *value >= 0).ok_or(
                ApplicationServiceError::Preview(PreviewAssetError::ProviderNoPreview),
            )?),
        };
        let projected_asset = ProviderAsset {
            asset_key: Some(asset_key.to_owned()),
            stable_asset_key: Some(stable_key.to_owned()),
            access_descriptor: None,
            role: AssetRole::Preview,
            native_roles,
            url: Some(url),
            file_name: Some(file_name.to_owned()),
            media_type: asset
                .get("media_type")
                .and_then(Value::as_str)
                .map(str::to_owned),
            size_bytes,
        };
        if projected_asset.validate().is_err() {
            return Err(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ));
        }
        projected.push(projected_asset);
    }
    Ok(projected)
}

fn frozen_preview_plan(item: &SnapshotItem) -> Result<FrozenPreviewPlan, ApplicationServiceError> {
    if item.validate().is_err() {
        return Err(ApplicationServiceError::Preview(
            PreviewAssetError::ProviderNoPreview,
        ));
    }
    let logical_dataset = item.metadata.get("logical_dataset").and_then(Value::as_str);
    if logical_dataset == Some(NISAR_LOGICAL_DATASET) {
        if item.provider_id != NISAR_PROVIDER_ID
            || item
                .metadata
                .get("provider_product_key")
                .and_then(Value::as_str)
                != Some(item.item_id.as_str())
        {
            return Err(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ));
        }
        let assets = frozen_nisar_preview_assets(item)?;
        return Ok(FrozenPreviewPlan::Direct {
            product_key: item.item_id.clone(),
            product_class: PreviewProductClass::NisarCompleteRslc,
            assets,
        });
    }
    if logical_dataset == Some(BHOONIDHI_LOGICAL_DATASET) {
        if item.provider_id != BHOONIDHI_PROVIDER_ID
            || item
                .metadata
                .get("provider_product_key")
                .and_then(Value::as_str)
                != Some(item.item_id.as_str())
        {
            return Err(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ));
        }
        let assets = serde_json::from_value::<Vec<ProviderAsset>>(
            item.metadata
                .get("assets")
                .cloned()
                .ok_or(ApplicationServiceError::Preview(
                    PreviewAssetError::ProviderNoPreview,
                ))?,
        )
        .map_err(|_| ApplicationServiceError::Preview(PreviewAssetError::ProviderNoPreview))?;
        return Ok(FrozenPreviewPlan::Direct {
            product_key: item.item_id.clone(),
            product_class: PreviewProductClass::BhoonidhiSsarRslc,
            assets,
        });
    }
    if item.metadata.get("dataset").and_then(Value::as_str) == Some("SENTINEL-1")
        && item
            .metadata
            .get("product_family")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("GRD"))
        && item.provider_id == "asf.catalog"
        && item.metadata.get("product_key").and_then(Value::as_str) == Some(item.item_id.as_str())
    {
        let assets = serde_json::from_value::<Vec<ProviderAsset>>(
            item.metadata
                .get("assets")
                .cloned()
                .ok_or(ApplicationServiceError::Preview(
                    PreviewAssetError::ProviderNoPreview,
                ))?,
        )
        .map_err(|_| ApplicationServiceError::Preview(PreviewAssetError::ProviderNoPreview))?;
        return Ok(FrozenPreviewPlan::Direct {
            product_key: item.item_id.clone(),
            product_class: PreviewProductClass::Sentinel1Grd,
            assets,
        });
    }
    if logical_dataset == Some("optical.sentinel-2.l2a")
        && item.provider_id == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
    {
        if item
            .metadata
            .get("provider_product_key")
            .and_then(Value::as_str)
            != Some(item.item_id.as_str())
        {
            return Err(ApplicationServiceError::Preview(
                PreviewAssetError::ProviderNoPreview,
            ));
        }
        let assets = serde_json::from_value::<Vec<ProviderAsset>>(
            item.metadata
                .get("assets")
                .cloned()
                .ok_or(ApplicationServiceError::Preview(
                    PreviewAssetError::ProviderNoPreview,
                ))?,
        )
        .map_err(|_| ApplicationServiceError::Preview(PreviewAssetError::ProviderNoPreview))?;
        return Ok(FrozenPreviewPlan::Direct {
            product_key: item.item_id.clone(),
            product_class: PreviewProductClass::EarthSearchSentinel2Quicklook,
            assets,
        });
    }
    if logical_dataset == Some("optical.sentinel-2.l2a") {
        return Ok(FrozenPreviewPlan::Cdse);
    }
    if logical_dataset == Some(LANDSAT_C2_L2_LOGICAL_DATASET) {
        return Ok(FrozenPreviewPlan::Landsat);
    }
    Err(ApplicationServiceError::Preview(
        PreviewAssetError::ProviderNoPreview,
    ))
}

fn frozen_landsat_browse_evidence(
    batch: &FrozenSearchBatch,
    item: &SnapshotItem,
) -> Result<Option<FrozenLandsatBrowseEvidence>, ApplicationServiceError> {
    if !matches!(
        item.provider_id.as_str(),
        LANDSAT_C2_L2_PROVIDER_ID | insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
    ) {
        return Err(ApplicationServiceError::LandsatBrowseEvidenceInvalid);
    }
    let metadata = &item.metadata;
    let properties = metadata
        .get("properties")
        .and_then(Value::as_object)
        .ok_or(ApplicationServiceError::LandsatBrowseEvidenceInvalid)?;
    let entity_id = properties
        .get("entity_id")
        .and_then(Value::as_str)
        .filter(|value| valid_landsat_identifier(value))
        .ok_or(ApplicationServiceError::LandsatBrowseEvidenceInvalid)?;
    let product_id = properties
        .get("landsat_product_id")
        .and_then(Value::as_str)
        .filter(|value| valid_landsat_identifier(value))
        .ok_or(ApplicationServiceError::LandsatBrowseEvidenceInvalid)?;
    let browse_available = properties
        .get("browse_available")
        .and_then(Value::as_bool)
        .ok_or(ApplicationServiceError::LandsatBrowseEvidenceInvalid)?;
    let query: CatalogSearchQuery = serde_json::from_value(Value::Object(batch.query.clone()))
        .map_err(|_| ApplicationServiceError::LandsatBrowseEvidenceInvalid)?;
    let profile = CollectionProfileRegistry
        .for_source(
            &query.logical_dataset,
            &query.provider_id,
            &query.native_collection_id,
        )
        .filter(|profile| profile.logical_dataset == LANDSAT_C2_L2_LOGICAL_DATASET)
        .ok_or(ApplicationServiceError::LandsatBrowseEvidenceInvalid)?;
    let expected_item_id =
        provider_product_key(profile.provider_id, profile.native_collection_id, entity_id)
            .map_err(|_| ApplicationServiceError::LandsatBrowseEvidenceInvalid)?;
    if item.validate().is_err()
        || query.validate().is_err()
        || query.logical_dataset != LANDSAT_C2_L2_LOGICAL_DATASET
        || query.provider_id != profile.provider_id
        || query.native_collection_id != profile.native_collection_id
        || query.aoi_geojson != batch.aoi_geojson
        || item.provider_id != profile.provider_id
        || batch.provider_id != profile.provider_id
        || item.item_id != expected_item_id
        || item.display_name != product_id
        || properties.get("dataset_alias").and_then(Value::as_str)
            != Some(profile.native_collection_id)
        || metadata.get("logical_dataset").and_then(Value::as_str)
            != Some(LANDSAT_C2_L2_LOGICAL_DATASET)
        || metadata.get("native_collection_id").and_then(Value::as_str)
            != Some(profile.native_collection_id)
        || metadata
            .get("collection_profile_id")
            .and_then(Value::as_str)
            != Some(profile.profile_id)
        || metadata.get("identity_rule_id").and_then(Value::as_str)
            != Some(profile.identity_rule_id)
        || metadata.get("source_kind").and_then(Value::as_str) != Some("landsat")
        || metadata.get("product_kind").and_then(Value::as_str) != Some("optical_scene")
        || metadata.get("provider_product_id").and_then(Value::as_str) != Some(entity_id)
        || metadata.get("provider_product_key").and_then(Value::as_str)
            != Some(item.item_id.as_str())
    {
        return Err(ApplicationServiceError::LandsatBrowseEvidenceInvalid);
    }
    if profile.provider_id == insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID {
        insar_provider::planetary_computer::validate_planetary_computer_landsat_source_id(
            entity_id, product_id,
        )
        .map_err(|_| ApplicationServiceError::LandsatBrowseEvidenceInvalid)?;
    }
    Ok(browse_available.then(|| FrozenLandsatBrowseEvidence {
        query,
        entity_id: entity_id.to_owned(),
    }))
}

struct FrozenCdseQuicklookProduct {
    product_id: String,
    safe_name: String,
    acquisition_datetime: Option<String>,
}

struct FrozenSentinel2AnalysisSource {
    source_provider_id: String,
    earth_search_item_id: Option<String>,
    safe_name: String,
    acquisition_datetime: String,
}

fn frozen_sentinel2_analysis_source(
    batch: &FrozenSearchBatch,
    item: &SnapshotItem,
) -> Result<FrozenSentinel2AnalysisSource, ApplicationServiceError> {
    let query: CatalogSearchQuery = serde_json::from_value(Value::Object(batch.query.clone()))
        .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
    if query.logical_dataset != "optical.sentinel-2.l2a" {
        return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
    }
    if query.provider_id == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID {
        let profile = CollectionProfileRegistry
            .for_source(
                &query.logical_dataset,
                &query.provider_id,
                &query.native_collection_id,
            )
            .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let provider_product_id = item
            .metadata
            .get("provider_product_id")
            .and_then(Value::as_str)
            .filter(|value| valid_exact_text(value))
            .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let expected_product_key = provider_product_key(
            profile.provider_id,
            profile.native_collection_id,
            provider_product_id,
        )
        .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        if batch.validate().is_err()
            || query.validate().is_err()
            || query.native_collection_id
                != insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID
            || batch.provider_id != query.provider_id
            || query.aoi_geojson != batch.aoi_geojson
            || item.validate().is_err()
            || item.provider_id != query.provider_id
            || !item.payload.is_empty()
            || item.asset_access.is_some()
            || item.execution_policy.is_some()
            || item.item_id != expected_product_key
            || item.metadata.get("logical_dataset").and_then(Value::as_str)
                != Some("optical.sentinel-2.l2a")
            || item
                .metadata
                .get("native_collection_id")
                .and_then(Value::as_str)
                != Some(insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID)
            || item
                .metadata
                .get("collection_profile_id")
                .and_then(Value::as_str)
                != Some(insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID)
            || item
                .metadata
                .get("identity_rule_id")
                .and_then(Value::as_str)
                != Some(insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID)
            || item
                .metadata
                .get("resolver_policy_id")
                .and_then(Value::as_str)
                != Some(insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID)
            || item
                .metadata
                .get("provider_product_key")
                .and_then(Value::as_str)
                != Some(item.item_id.as_str())
            || item.metadata.get("source_kind").and_then(Value::as_str) != Some("sentinel2")
            || item.metadata.get("product_kind").and_then(Value::as_str) != Some("optical_scene")
        {
            return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
        }
        insar_provider::catalog::validate_earth_search_sentinel2_source_id(
            provider_product_id,
            &item.display_name,
            item.acquisition_datetime.as_deref(),
        )
        .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let acquisition_datetime = item
            .acquisition_datetime
            .clone()
            .filter(|value| parse_normalized_utc(value).is_some())
            .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        return Ok(FrozenSentinel2AnalysisSource {
            source_provider_id: query.provider_id,
            earth_search_item_id: Some(provider_product_id.to_owned()),
            safe_name: item.display_name.clone(),
            acquisition_datetime,
        });
    }
    if query.provider_id == insar_provider::catalog::MPC_SENTINEL2_L2A_PROVIDER_ID {
        let profile = CollectionProfileRegistry
            .for_source(
                &query.logical_dataset,
                &query.provider_id,
                &query.native_collection_id,
            )
            .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let provider_product_id = item
            .metadata
            .get("provider_product_id")
            .and_then(Value::as_str)
            .filter(|value| valid_exact_text(value))
            .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let expected_product_key = provider_product_key(
            profile.provider_id,
            profile.native_collection_id,
            provider_product_id,
        )
        .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        if batch.validate().is_err()
            || query.validate().is_err()
            || query.native_collection_id
                != insar_provider::catalog::MPC_SENTINEL2_L2A_COLLECTION_ID
            || batch.provider_id != query.provider_id
            || query.aoi_geojson != batch.aoi_geojson
            || item.validate().is_err()
            || item.provider_id != query.provider_id
            || !item.payload.is_empty()
            || item.asset_access.is_some()
            || item.execution_policy.is_some()
            || item.item_id != expected_product_key
            || item.metadata.get("logical_dataset").and_then(Value::as_str)
                != Some("optical.sentinel-2.l2a")
            || item
                .metadata
                .get("native_collection_id")
                .and_then(Value::as_str)
                != Some(insar_provider::catalog::MPC_SENTINEL2_L2A_COLLECTION_ID)
            || item
                .metadata
                .get("collection_profile_id")
                .and_then(Value::as_str)
                != Some(insar_provider::catalog::MPC_SENTINEL2_L2A_PROFILE_ID)
            || item
                .metadata
                .get("identity_rule_id")
                .and_then(Value::as_str)
                != Some(insar_provider::catalog::MPC_SENTINEL2_L2A_IDENTITY_RULE_ID)
            || item
                .metadata
                .get("resolver_policy_id")
                .and_then(Value::as_str)
                != Some(insar_provider::catalog::MPC_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID)
            || item
                .metadata
                .get("provider_product_key")
                .and_then(Value::as_str)
                != Some(item.item_id.as_str())
            || item.metadata.get("source_kind").and_then(Value::as_str) != Some("sentinel2")
            || item.metadata.get("product_kind").and_then(Value::as_str) != Some("optical_scene")
        {
            return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
        }
        insar_provider::catalog::validate_mpc_sentinel2_source_id(
            provider_product_id,
            &item.display_name,
            item.acquisition_datetime.as_deref(),
        )
        .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let acquisition_datetime = item
            .acquisition_datetime
            .clone()
            .filter(|value| parse_normalized_utc(value).is_some())
            .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        return Ok(FrozenSentinel2AnalysisSource {
            source_provider_id: query.provider_id,
            earth_search_item_id: None,
            safe_name: item.display_name.clone(),
            acquisition_datetime,
        });
    }

    let cdse = frozen_cdse_quicklook_product(batch, item)
        .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
    let acquisition_datetime = cdse
        .acquisition_datetime
        .filter(|value| parse_normalized_utc(value).is_some())
        .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
    Ok(FrozenSentinel2AnalysisSource {
        source_provider_id: query.provider_id,
        earth_search_item_id: None,
        safe_name: cdse.safe_name,
        acquisition_datetime,
    })
}

/// Preview discovery is anchored to the same frozen product identity used by
/// archive resolution. It deliberately accepts no frozen quicklook URL or
/// asset ID: those are transient evidence returned by the fixed discovery call.
fn frozen_cdse_quicklook_product(
    batch: &FrozenSearchBatch,
    item: &SnapshotItem,
) -> Result<FrozenCdseQuicklookProduct, ApplicationServiceError> {
    let query: CatalogSearchQuery = serde_json::from_value(Value::Object(batch.query.clone()))
        .map_err(|_| ApplicationServiceError::CdseQuicklookEvidenceInvalid)?;
    let profile = CollectionProfileRegistry
        .for_source(
            &query.logical_dataset,
            &query.provider_id,
            &query.native_collection_id,
        )
        .filter(|profile| {
            matches!(
                profile.logical_dataset,
                "optical.sentinel-2.l2a" | "optical.sentinel-2.l1c" | "optical.sentinel-3.standard"
            )
        })
        .ok_or(ApplicationServiceError::CdseQuicklookEvidenceInvalid)?;
    if batch.validate().is_err()
        || query.validate().is_err()
        || query.logical_dataset != profile.logical_dataset
        || query.provider_id != profile.provider_id
        || query.native_collection_id != profile.native_collection_id
        || query.aoi_geojson != batch.aoi_geojson
        || batch.provider_id != profile.provider_id
        || item.validate().is_err()
        || item.provider_id != profile.provider_id
        || !item.payload.is_empty()
        || !valid_catalogue_display_name(&item.display_name)
        || item.metadata.get("logical_dataset").and_then(Value::as_str)
            != Some(profile.logical_dataset)
        || item
            .metadata
            .get("native_collection_id")
            .and_then(Value::as_str)
            != Some(profile.native_collection_id)
        || item
            .metadata
            .get("collection_profile_id")
            .and_then(Value::as_str)
            != Some(profile.profile_id)
        || item
            .metadata
            .get("identity_rule_id")
            .and_then(Value::as_str)
            != Some(profile.identity_rule_id)
        || item.metadata.get("source_kind").and_then(Value::as_str) != Some("sentinel2")
        || item.metadata.get("product_kind").and_then(Value::as_str) != Some("optical_scene")
    {
        return Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid);
    }
    let product_id = item
        .metadata
        .get("provider_product_id")
        .and_then(Value::as_str)
        .filter(|value| valid_exact_text(value) && canonical_odata_uuid(value))
        .ok_or(ApplicationServiceError::CdseQuicklookEvidenceInvalid)?;
    let expected_item_id = provider_product_key(
        profile.provider_id,
        profile.native_collection_id,
        product_id,
    )
    .map_err(|_| ApplicationServiceError::CdseQuicklookEvidenceInvalid)?;
    if item.item_id != expected_item_id
        || item
            .metadata
            .get("provider_product_key")
            .and_then(Value::as_str)
            != Some(item.item_id.as_str())
    {
        return Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid);
    }
    let assets = item
        .metadata
        .get("assets")
        .and_then(Value::as_array)
        .ok_or(ApplicationServiceError::CdseQuicklookEvidenceInvalid)?;
    let archive_indices = assets
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            (candidate.get("role").and_then(Value::as_str) == Some("primary")).then_some(index)
        })
        .collect::<Vec<_>>();
    let [archive_index] = archive_indices.as_slice() else {
        return Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid);
    };
    if assets.len() > 2
        || assets.iter().enumerate().any(|(index, candidate)| {
            index != *archive_index
                && candidate.get("role").and_then(Value::as_str) != Some("thumbnail")
        })
    {
        return Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid);
    }
    let archive = assets[*archive_index]
        .as_object()
        .ok_or(ApplicationServiceError::CdseQuicklookEvidenceInvalid)?;
    let (Some(asset_key), Some(stable_key), Some(role), Some(native_roles)) = (
        archive.get("asset_key").and_then(Value::as_str),
        archive.get("stable_asset_key").and_then(Value::as_str),
        archive.get("role").and_then(Value::as_str),
        archive.get("native_roles").and_then(Value::as_array),
    ) else {
        return Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid);
    };
    let expected_stable_key = stable_asset_key(&item.item_id, "archive")
        .map_err(|_| ApplicationServiceError::CdseQuicklookEvidenceInvalid)?;
    let archive_size_is_valid = match archive.get("size_bytes") {
        None => true,
        Some(value) => value.as_i64().is_some_and(|length| length >= 0),
    };
    if archive.len()
        != if archive.contains_key("size_bytes") {
            5
        } else {
            4
        }
        || asset_key != "archive"
        || stable_key != expected_stable_key
        || role != "primary"
        || native_roles.len() != 1
        || native_roles.first().and_then(Value::as_str) != Some("archive")
        || !archive_size_is_valid
    {
        return Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid);
    }
    Ok(FrozenCdseQuicklookProduct {
        product_id: product_id.to_owned(),
        safe_name: item.display_name.clone(),
        acquisition_datetime: item.acquisition_datetime.clone(),
    })
}

fn preferred_cdse_checksum(
    properties: &Map<String, Value>,
) -> Result<(String, String), ApplicationServiceError> {
    let checksums = properties
        .get("checksums")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "目录产品缺少归档校验和",
                "checksums",
            )
        })?;
    for algorithm in ["SHA256", "MD5"] {
        if let Some(checksum) = checksums
            .iter()
            .find(|value| value.get("algorithm").and_then(Value::as_str) == Some(algorithm))
        {
            let value = checksum
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    catalogue_provider_error(
                        "CATALOGUE_PRODUCT_INVALID",
                        "目录产品缺少有效的归档校验和",
                        "checksums",
                    )
                })?;
            return Ok((algorithm.to_owned(), value.to_owned()));
        }
    }
    Err(catalogue_provider_error(
        "CATALOGUE_PRODUCT_INVALID",
        "目录产品缺少受支持的归档校验和",
        "checksums",
    ))
}

fn sentinel2_band_output_root(selected_root: &Path, item_id: &str) -> PathBuf {
    if selected_root
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == item_id)
    {
        selected_root.to_path_buf()
    } else {
        selected_root.join(item_id)
    }
}

fn valid_landsat_identifier(value: &str) -> bool {
    valid_exact_text(value)
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn valid_wrs_coordinate(value: &str, maximum: u16) -> bool {
    value.len() == 3
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value
            .parse::<u16>()
            .is_ok_and(|value| (1..=maximum).contains(&value))
}

fn landsat_property_text<'a>(
    properties: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, ApplicationServiceError> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| valid_exact_text(value))
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Landsat 场景权威属性无效",
                "landsat_properties",
            )
        })
}

fn freeze_landsat_scene_properties(
    product: &ProviderProduct,
    acquisition_datetime: &DateTime<FixedOffset>,
) -> Result<Map<String, Value>, ApplicationServiceError> {
    const REQUIRED: &[&str] = &[
        "entity_id",
        "landsat_product_id",
        "dataset_alias",
        "spacecraft",
        "processing_level",
        "data_type_l2",
        "collection_category",
        "cloud_cover",
        "wrs_path",
        "wrs_row",
        "acquisition_date",
        "acquisition_precision",
        "browse_available",
    ];
    const OPTIONAL: &[&str] = &["landsat_scene_id"];
    validate_debug_context(&product.properties).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Landsat 场景属性包含敏感字段",
            "sensitive_properties",
        )
    })?;
    if REQUIRED
        .iter()
        .any(|key| !product.properties.contains_key(*key))
        || product
            .properties
            .keys()
            .any(|key| !REQUIRED.contains(&key.as_str()) && !OPTIONAL.contains(&key.as_str()))
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Landsat 场景包含未批准或缺失的属性",
            "landsat_properties",
        ));
    }
    let entity_id = landsat_property_text(&product.properties, "entity_id")?;
    let landsat_product_id = landsat_property_text(&product.properties, "landsat_product_id")?;
    let dataset_alias = landsat_property_text(&product.properties, "dataset_alias")?;
    let spacecraft = landsat_property_text(&product.properties, "spacecraft")?;
    let processing_level = landsat_property_text(&product.properties, "processing_level")?;
    let data_type_l2 = landsat_property_text(&product.properties, "data_type_l2")?;
    let collection_category = landsat_property_text(&product.properties, "collection_category")?;
    let wrs_path = landsat_property_text(&product.properties, "wrs_path")?;
    let wrs_row = landsat_property_text(&product.properties, "wrs_row")?;
    let acquisition_date = landsat_property_text(&product.properties, "acquisition_date")?;
    let acquisition_precision =
        landsat_property_text(&product.properties, "acquisition_precision")?;
    let cloud_cover = product
        .properties
        .get("cloud_cover")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value));
    let browse_available = product
        .properties
        .get("browse_available")
        .and_then(Value::as_bool);
    let scene_id_is_valid = product
        .properties
        .get("landsat_scene_id")
        .is_none_or(|value| value.as_str().is_some_and(valid_landsat_identifier));
    let expected_acquisition_date = acquisition_datetime.format("%Y/%m/%d").to_string();
    if entity_id != product.product_id
        || landsat_product_id != product.display_name
        || CollectionProfileRegistry
            .for_source(
                LANDSAT_C2_L2_LOGICAL_DATASET,
                &product.provider_id,
                dataset_alias,
            )
            .is_none()
        || !valid_landsat_identifier(entity_id)
        || !valid_landsat_identifier(landsat_product_id)
        || !matches!(
            spacecraft,
            "landsat_4" | "landsat_5" | "landsat_7" | "landsat_8" | "landsat_9"
        )
        || !matches!(processing_level, "l2sp" | "l2sr")
        || !data_type_l2.ends_with(match processing_level {
            "l2sp" => "_L2SP",
            "l2sr" => "_L2SR",
            _ => unreachable!("processing level is checked above"),
        })
        || !matches!(collection_category, "T1" | "T2" | "RT")
        || cloud_cover.is_none()
        || !valid_wrs_coordinate(wrs_path, 233)
        || !valid_wrs_coordinate(wrs_row, 248)
        || acquisition_date != expected_acquisition_date
        || !match acquisition_precision {
            "day" => acquisition_datetime.time() == chrono::NaiveTime::MIN,
            "timestamp" => true,
            _ => false,
        }
        || browse_available.is_none()
        || (product.provider_id == insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID
            && browse_available != Some(false))
        || !scene_id_is_valid
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Landsat 场景权威属性冲突",
            "landsat_authority",
        ));
    }
    Ok(product.properties.clone())
}

fn map_landsat_scene_product(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
) -> Result<SnapshotItem, ApplicationServiceError> {
    let profile = CollectionProfileRegistry
        .for_source(
            &query.logical_dataset,
            &query.provider_id,
            &query.native_collection_id,
        )
        .filter(|profile| profile.logical_dataset == LANDSAT_C2_L2_LOGICAL_DATASET)
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Landsat 来源身份无效",
                "landsat_source",
            )
        })?;
    if query.native_collection_id != profile.native_collection_id
        || product.source_kind != DataSourceKind::Landsat
        || product.product_kind != ProductKind::OpticalScene
        || product.provider_id != profile.provider_id
        || !valid_landsat_identifier(&product.product_id)
        || !valid_landsat_identifier(&product.display_name)
        || product.footprint_geojson.is_none()
        || product.footprint_bbox.is_some()
        || !product.assets.is_empty()
        || !product.payload.is_empty()
        || product.validate().is_err()
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Landsat 场景包含不受支持的字段",
            "landsat_product",
        ));
    }
    let acquisition_datetime = product
        .acquisition_datetime
        .as_deref()
        .and_then(parse_catalogue_utc)
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Landsat 场景获取时间无效",
                "acquisition_datetime",
            )
        })?;
    let frozen_properties = freeze_landsat_scene_properties(&product, &acquisition_datetime)?;
    if matches!(
        profile.provider_id,
        insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
            | insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID
    ) {
        insar_provider::planetary_computer::validate_planetary_computer_landsat_source_id(
            &product.product_id,
            &product.display_name,
        )
        .map_err(|_| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Landsat STAC 产品身份冲突",
                "stac_item_id",
            )
        })?;
    }
    let item_id = provider_product_key(
        &product.provider_id,
        profile.native_collection_id,
        &product.product_id,
    )
    .map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Landsat 场景身份无效",
            "provider_product_key",
        )
    })?;
    let metadata = Map::from_iter([
        (
            "logical_dataset".to_owned(),
            Value::String(profile.logical_dataset.to_owned()),
        ),
        (
            "native_collection_id".to_owned(),
            Value::String(profile.native_collection_id.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(profile.profile_id.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(profile.identity_rule_id.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(profile.filename_rule_id.to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            serde_json::to_value(profile.delivery_model.clone())
                .expect("closed delivery model serializes"),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product.product_id.clone()),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(item_id.clone()),
        ),
        (
            "source_kind".to_owned(),
            Value::String("landsat".to_owned()),
        ),
        (
            "product_kind".to_owned(),
            Value::String("optical_scene".to_owned()),
        ),
        ("properties".to_owned(), Value::Object(frozen_properties)),
        ("assets".to_owned(), Value::Array(Vec::new())),
    ]);
    validate_debug_context(&metadata).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Landsat 场景快照元数据包含敏感字段",
            "sensitive_metadata",
        )
    })?;
    let item = SnapshotItem {
        item_id,
        provider_id: product.provider_id,
        display_name: product.display_name,
        acquisition_datetime: Some(
            acquisition_datetime.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        ),
        footprint_geojson: product.footprint_geojson,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    item.validate().map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Landsat 场景快照无效",
            "snapshot_item",
        )
    })?;
    Ok(item)
}

fn map_catalogue_product(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
    route_use: RouteUse,
) -> Result<SnapshotItem, ApplicationServiceError> {
    match query.logical_dataset.as_str() {
        LANDSAT_C2_L2_LOGICAL_DATASET => map_landsat_scene_product(product, query),
        MPC_SENTINEL1_GRD_LOGICAL_DATASET | MPC_COPDEM_GLO30_LOGICAL_DATASET => {
            map_reviewed_mpc_catalogue_product(product, query)
        }
        ALOS2_LOGICAL_DATASET => map_alos2_catalogue_product(product, query),
        NISAR_LOGICAL_DATASET => map_nisar_rslc_catalogue_product(product, query),
        BHOONIDHI_LOGICAL_DATASET => map_bhoonidhi_catalogue_product(product, query),
        "optical.sentinel-2.l2a" | "optical.sentinel-2.l1c" | "optical.sentinel-3.standard" => {
            map_sentinel2_catalogue_product(product, query, route_use)
        }
        _ if hls_catalogue::hls_product_for_query(query).is_some() => {
            hls_catalogue::map_hls_scene_product(product, query)
        }
        _ => Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录产品数据集不受支持",
            "logical_dataset",
        )),
    }
}

fn map_reviewed_mpc_catalogue_product(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
) -> Result<SnapshotItem, ApplicationServiceError> {
    let class = reviewed_mpc_class_for_query(query).ok_or_else(|| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Planetary Computer 审核 COG 来源绑定无效",
            "source",
        )
    })?;
    let profile = CollectionProfileRegistry
        .for_source(
            &query.logical_dataset,
            &query.provider_id,
            &query.native_collection_id,
        )
        .filter(|profile| profile.profile_id == class.collection_profile_id())
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Planetary Computer 审核 COG 集合档案无效",
                "collection_profile",
            )
        })?;
    let (expected_source_kind, expected_product_kind, source_kind, product_kind) = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => (
            DataSourceKind::Sentinel1,
            ProductKind::SarGrd,
            "sentinel1",
            "sar_grd",
        ),
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => (
            DataSourceKind::Dem,
            ProductKind::DemRaster,
            "dem",
            "dem_raster",
        ),
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
            return Err(catalogue_provider_error(
                "CATALOGUE_ROUTE_DISABLED",
                "Sentinel-2 分析波段不允许进入通用 reviewed MPC 搜索映射",
                "sentinel2_band_search_forbidden",
            ));
        }
    };
    if query.validate().is_err()
        || product.validate().is_err()
        || product.provider_id != PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID
        || product.source_kind != expected_source_kind
        || product.product_kind != expected_product_kind
        || !valid_exact_text(&product.product_id)
        || product.product_id != product.display_name
        || product.footprint_geojson.is_none()
        || product.footprint_bbox.is_some()
        || !product.properties.is_empty()
        || !product.payload.is_empty()
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Planetary Computer 审核 COG 产品包含不受支持的字段",
            "product",
        ));
    }
    if product
        .footprint_geojson
        .as_ref()
        .and_then(validated_authoritative_polygonal_geojson)
        .is_none()
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Planetary Computer 审核 COG 产品几何无效",
            "footprint",
        ));
    }
    let acquisition_datetime = match product.acquisition_datetime.as_deref() {
        Some(value) => Some(parse_normalized_utc(value).ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Planetary Computer 审核 COG 产品采集时间无效",
                "acquisition_datetime",
            )
        })?),
        None => None,
    };
    if matches!(class, PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog)
        && acquisition_datetime.is_none()
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Sentinel-1 GRD 产品缺少规范采集时间",
            "acquisition_datetime",
        ));
    }
    let item_id = provider_product_key(
        PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
        class.collection_id(),
        &product.product_id,
    )
    .map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Planetary Computer 审核 COG 产品标识无效",
            "provider_product_key",
        )
    })?;

    let expected_count = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => 1..=4,
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => 1..=1,
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
            return Err(catalogue_provider_error(
                "CATALOGUE_ROUTE_DISABLED",
                "Sentinel-2 分析波段不允许进入通用 reviewed MPC 搜索映射",
                "sentinel2_band_search_forbidden",
            ));
        }
    };
    if !expected_count.contains(&product.assets.len()) {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Planetary Computer 审核 COG 资产数量无效",
            "assets",
        ));
    }
    let mut frozen_assets = Vec::with_capacity(product.assets.len());
    let mut seen = BTreeSet::new();
    for asset in &product.assets {
        let asset_key = asset
            .asset_key
            .as_deref()
            .filter(|value| valid_exact_text(value))
            .ok_or_else(|| {
                catalogue_provider_error(
                    "CATALOGUE_PRODUCT_INVALID",
                    "Planetary Computer 审核 COG 资产键无效",
                    "asset_key",
                )
            })?;
        let class_key_allowed = match class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => {
                matches!(asset_key, "vv" | "vh" | "hh" | "hv")
            }
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => asset_key == "data",
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => false,
        };
        let expected_stable_key = stable_asset_key(&item_id, asset_key).map_err(|_| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Planetary Computer 审核 COG 稳定资产标识无效",
                "stable_asset_key",
            )
        })?;
        let href = asset.url.as_deref().ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Planetary Computer 审核 COG 缺少稳定资产定位",
                "asset_href",
            )
        })?;
        let expected_name = validate_planetary_computer_reviewed_asset_identity(
            class,
            &product.product_id,
            asset_key,
            href,
        )
        .map_err(|_| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Planetary Computer 审核 COG 资产身份无效",
                "asset_identity",
            )
        })?;
        if !class_key_allowed
            || !seen.insert(asset_key)
            || asset.stable_asset_key.as_deref() != Some(expected_stable_key.as_str())
            || asset.access_descriptor.is_some()
            || asset.role != AssetRole::Primary
            || !asset.native_roles.iter().any(|role| role == "data")
            || asset.file_name.as_deref() != Some(expected_name.as_str())
            || !asset
                .media_type
                .as_deref()
                .is_some_and(|value| value.starts_with("image/tiff"))
            || asset.size_bytes.is_some_and(|value| value < 0)
        {
            return Err(catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Planetary Computer 审核 COG 资产证据无效",
                "asset",
            ));
        }
        let mut frozen = Map::from_iter([
            ("asset_key".to_owned(), Value::String(asset_key.to_owned())),
            (
                "stable_asset_key".to_owned(),
                Value::String(expected_stable_key),
            ),
            (
                "role".to_owned(),
                serde_json::to_value(AssetRole::Primary)
                    .expect("closed primary asset role serializes"),
            ),
            (
                "native_roles".to_owned(),
                Value::Array(
                    asset
                        .native_roles
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            ),
            ("file_name".to_owned(), Value::String(expected_name)),
            (
                "media_type".to_owned(),
                Value::String(
                    asset
                        .media_type
                        .as_deref()
                        .expect("validated reviewed COG media type")
                        .to_owned(),
                ),
            ),
        ]);
        if let Some(size_bytes) = asset.size_bytes {
            frozen.insert("size_bytes".to_owned(), Value::Number(size_bytes.into()));
        }
        // Search freezes descriptive asset identity only. Runtime delivery must
        // re-resolve exact evidence before Resolver authority can be minted.
        frozen_assets.push(Value::Object(frozen));
    }
    frozen_assets.sort_by(|left, right| {
        left.get("asset_key")
            .and_then(Value::as_str)
            .cmp(&right.get("asset_key").and_then(Value::as_str))
    });

    let metadata = Map::from_iter([
        (
            "logical_dataset".to_owned(),
            Value::String(profile.logical_dataset.to_owned()),
        ),
        (
            "native_collection_id".to_owned(),
            Value::String(profile.native_collection_id.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(profile.profile_id.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(profile.identity_rule_id.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(profile.filename_rule_id.to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            serde_json::to_value(profile.delivery_model.clone())
                .expect("closed delivery model serializes"),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(profile.resolver_policy_id.to_owned()),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product.product_id.clone()),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(item_id.clone()),
        ),
        (
            "stac_item_id".to_owned(),
            Value::String(product.product_id.clone()),
        ),
        (
            "source_kind".to_owned(),
            Value::String(source_kind.to_owned()),
        ),
        (
            "product_kind".to_owned(),
            Value::String(product_kind.to_owned()),
        ),
        ("assets".to_owned(), Value::Array(frozen_assets)),
    ]);
    validate_debug_context(&metadata).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Planetary Computer 审核 COG 快照元数据包含敏感字段",
            "sensitive_metadata",
        )
    })?;

    let item = SnapshotItem {
        item_id,
        provider_id: product.provider_id,
        display_name: product.display_name,
        acquisition_datetime: acquisition_datetime
            .map(|value| value.to_rfc3339_opts(SecondsFormat::AutoSi, true)),
        footprint_geojson: product.footprint_geojson,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    item.validate().map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Planetary Computer 审核 COG 快照项目无效",
            "snapshot_item",
        )
    })?;
    if serde_json::to_vec(&item)
        .map(|bytes| bytes.len() > MAX_CATALOGUE_CANDIDATE_JSON_BYTES)
        .unwrap_or(true)
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Planetary Computer 审核 COG 快照项目超出离线限制",
            "candidate_size",
        ));
    }
    Ok(item)
}

fn bhoonidhi_product_text<'a>(properties: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| valid_exact_text(value) && value.len() <= 512)
}

fn bhoonidhi_product_id_is_safe(value: &str) -> bool {
    valid_exact_text(value)
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        && !matches!(value, "." | "..")
}

fn map_bhoonidhi_catalogue_product(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
) -> Result<SnapshotItem, ApplicationServiceError> {
    let profile = CollectionProfileRegistry
        .for_logical_dataset(BHOONIDHI_LOGICAL_DATASET)
        .expect("Bhoonidhi profile is statically registered");
    if query.validate().is_err()
        || query.logical_dataset != BHOONIDHI_LOGICAL_DATASET
        || query.provider_id != BHOONIDHI_PROVIDER_ID
        || query.native_collection_id != BHOONIDHI_NATIVE_COLLECTION_ID
        || product.provider_id != BHOONIDHI_PROVIDER_ID
        || product.source_kind != DataSourceKind::Sar
        || product.product_kind != ProductKind::SarSlc
        || !bhoonidhi_product_id_is_safe(&product.product_id)
        || !valid_catalogue_display_name(&product.display_name)
        || product.footprint_geojson.is_none()
        || product.footprint_bbox.is_none()
        || !product.payload.is_empty()
        || product.validate().is_err()
    {
        return Err(catalogue_provider_error(
            "BHOONIDHI_PRODUCT_INVALID",
            "Bhoonidhi S-RSLC 目录产品包含不受支持的字段",
            "product_identity",
        ));
    }

    let geometry = product
        .footprint_geojson
        .as_ref()
        .and_then(validated_authoritative_polygonal_geojson);
    let canonical_bbox = geometry.as_ref().map(|value| value.bbox);
    let Some(footprint_bbox) = product.footprint_bbox.as_ref() else {
        unreachable!("validated above");
    };
    let bbox_matches_geometry = canonical_bbox.is_some_and(|bbox| {
        footprint_bbox.west == bbox[0]
            && footprint_bbox.south == bbox[1]
            && footprint_bbox.east == bbox[2]
            && footprint_bbox.north == bbox[3]
            && footprint_bbox.crs == "EPSG:4326"
    });
    if geometry.is_none() || !bbox_matches_geometry {
        return Err(catalogue_provider_error(
            "BHOONIDHI_PRODUCT_INVALID",
            "Bhoonidhi S-RSLC 产品足迹或规范化包络无效",
            "footprint",
        ));
    }

    let properties = &product.properties;
    validate_debug_context(properties).map_err(|_| {
        catalogue_provider_error(
            "BHOONIDHI_PRODUCT_INVALID",
            "Bhoonidhi S-RSLC 产品属性包含敏感字段",
            "sensitive_properties",
        )
    })?;
    if bhoonidhi_product_text(properties, "ID") != Some(product.product_id.as_str())
        || bhoonidhi_product_text(properties, "Online") != Some("Y")
        || bhoonidhi_product_text(properties, "provider_collection")
            != Some(BHOONIDHI_NATIVE_COLLECTION_ID)
    {
        return Err(catalogue_provider_error(
            "BHOONIDHI_PRODUCT_IDENTITY_MISMATCH",
            "Bhoonidhi S-RSLC 产品身份或在线状态无效",
            "provider_identity",
        ));
    }

    for (raw_key, normalized_key) in [
        ("ProductCode", "product_code"),
        ("SoftwareVersion", "software_version"),
        ("Producer", "producer"),
        ("Platform", "platform"),
        ("Instrument", "instrument"),
        ("Mode", "mode"),
        ("Polarization", "polarization"),
    ] {
        let Some(raw_value) = bhoonidhi_product_text(properties, raw_key) else {
            return Err(catalogue_provider_error(
                "BHOONIDHI_PRODUCT_SCOPE_MISMATCH",
                "Bhoonidhi S-RSLC 产品缺少原始科学属性",
                raw_key,
            ));
        };
        if bhoonidhi_product_text(properties, normalized_key) != Some(raw_value) {
            return Err(catalogue_provider_error(
                "BHOONIDHI_PRODUCT_SCOPE_MISMATCH",
                "Bhoonidhi S-RSLC 原始属性与规范化别名冲突",
                normalized_key,
            ));
        }
    }
    if bhoonidhi_product_text(properties, "Coverage").is_none() {
        return Err(catalogue_provider_error(
            "BHOONIDHI_PRODUCT_SCOPE_MISMATCH",
            "Bhoonidhi S-RSLC 产品缺少 Coverage 属性",
            "Coverage",
        ));
    }
    for alias in ["AcquistionMode", "AcquisitionMode"] {
        if let Some(raw_value) = bhoonidhi_product_text(properties, alias) {
            if bhoonidhi_product_text(properties, "acquisition_mode") != Some(raw_value) {
                return Err(catalogue_provider_error(
                    "BHOONIDHI_PRODUCT_SCOPE_MISMATCH",
                    "Bhoonidhi S-RSLC acquisition mode 别名冲突",
                    "acquisition_mode",
                ));
            }
        }
    }
    let flight_direction = bhoonidhi_product_text(properties, "flight_direction");
    if flight_direction.is_some_and(|value| !matches!(value, "ASCENDING" | "DESCENDING")) {
        return Err(catalogue_provider_error(
            "BHOONIDHI_PRODUCT_SCOPE_MISMATCH",
            "Bhoonidhi S-RSLC 升降轨信息无效",
            "flight_direction",
        ));
    }
    let look_direction = bhoonidhi_product_text(properties, "look_direction");
    if look_direction.is_some_and(|value| !matches!(value, "LEFT" | "RIGHT")) {
        return Err(catalogue_provider_error(
            "BHOONIDHI_PRODUCT_SCOPE_MISMATCH",
            "Bhoonidhi S-RSLC 侧视方向无效",
            "look_direction",
        ));
    }
    if let Some(scene_identifier) = bhoonidhi_product_text(properties, "Scene_Identifier") {
        if product.display_name != scene_identifier
            || bhoonidhi_product_text(properties, "scene_identifier") != Some(scene_identifier)
        {
            return Err(catalogue_provider_error(
                "BHOONIDHI_PRODUCT_IDENTITY_MISMATCH",
                "Bhoonidhi S-RSLC 场景身份冲突",
                "scene_identifier",
            ));
        }
    }
    let acquisition_datetime = product
        .acquisition_datetime
        .as_deref()
        .and_then(parse_normalized_utc)
        .ok_or_else(|| {
            catalogue_provider_error(
                "BHOONIDHI_PRODUCT_INVALID",
                "Bhoonidhi S-RSLC 产品获取时间无效",
                "acquisition_datetime",
            )
        })?;

    let item_id = provider_product_key(
        BHOONIDHI_PROVIDER_ID,
        BHOONIDHI_NATIVE_COLLECTION_ID,
        &product.product_id,
    )
    .map_err(|_| {
        catalogue_provider_error(
            "BHOONIDHI_PRODUCT_IDENTITY_MISMATCH",
            "Bhoonidhi S-RSLC 产品稳定身份无效",
            "provider_product_key",
        )
    })?;
    let stable_asset_key = stable_asset_key(&item_id, "data").map_err(|_| {
        catalogue_provider_error(
            "BHOONIDHI_PRODUCT_IDENTITY_MISMATCH",
            "Bhoonidhi S-RSLC 科学资产稳定身份无效",
            "stable_asset_key",
        )
    })?;
    let file_name = format!("{}.h5", product.product_id);
    let descriptor = AssetAccessDescriptor {
        resolver_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
        scheme: AssetAccessScheme::OauthBearerHttps,
        stable_href: Some("https://bhoonidhi-api.nrsc.gov.in/download".to_owned()),
        requires_credentials: true,
        credential_profile: Some("bhoonidhi-default".to_owned()),
        resolver_parameters: AssetResolverParameters {},
    };
    let access = FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
            collection_id: BHOONIDHI_NATIVE_COLLECTION_ID.to_owned(),
            provider_product_id: product.product_id.clone(),
            provider_product_key: item_id.clone(),
            canonical_product_key: None,
            asset_key: "data".to_owned(),
            stable_asset_key,
        },
        descriptor,
        role: AssetRole::Primary,
        file_name: Some(file_name.clone()),
        expected_size_bytes: None,
        checksum_algorithm: None,
        checksum_value: None,
    };

    let frozen_assets = serde_json::to_value(&product.assets).map_err(|_| {
        catalogue_provider_error(
            "BHOONIDHI_PRODUCT_INVALID",
            "Bhoonidhi S-RSLC 元数据资产无法冻结",
            "assets",
        )
    })?;
    let canonical_bbox = serde_json::to_value(footprint_bbox).map_err(|_| {
        catalogue_provider_error(
            "BHOONIDHI_PRODUCT_INVALID",
            "Bhoonidhi S-RSLC 规范化包络无法冻结",
            "footprint_bbox",
        )
    })?;
    let mut metadata = Map::from_iter([
        (
            "dataset".to_owned(),
            Value::String(BHOONIDHI_LOGICAL_DATASET.to_owned()),
        ),
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
            "identity_rule_id".to_owned(),
            Value::String(BHOONIDHI_IDENTITY_RULE_ID.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(BHOONIDHI_FILENAME_RULE_ID.to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            serde_json::to_value(profile.delivery_model.clone())
                .expect("closed delivery model serializes"),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(BHOONIDHI_RESOLVER_POLICY_ID.to_owned()),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product.product_id.clone()),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(item_id.clone()),
        ),
        (
            "product_family".to_owned(),
            Value::String(BHOONIDHI_NATIVE_COLLECTION_ID.to_owned()),
        ),
        (
            "processing_level".to_owned(),
            Value::String(BHOONIDHI_REQUIRED_PRODUCT_TYPE.to_owned()),
        ),
        ("band".to_owned(), Value::String("S".to_owned())),
        ("source_kind".to_owned(), Value::String("sar".to_owned())),
        (
            "product_kind".to_owned(),
            Value::String("sar_slc".to_owned()),
        ),
        (
            "primary_asset_key".to_owned(),
            Value::String("data".to_owned()),
        ),
        ("primary_file_name".to_owned(), Value::String(file_name)),
        ("footprint_bbox".to_owned(), canonical_bbox),
        (
            "properties".to_owned(),
            Value::Object(product.properties.clone()),
        ),
        ("assets".to_owned(), frozen_assets),
    ]);
    if let Some(value) = flight_direction {
        metadata.insert(
            "flight_direction".to_owned(),
            Value::String(value.to_owned()),
        );
    }
    if let Some(value) = look_direction {
        metadata.insert("look_direction".to_owned(), Value::String(value.to_owned()));
    }
    validate_debug_context(&metadata).map_err(|_| {
        catalogue_provider_error(
            "BHOONIDHI_PRODUCT_INVALID",
            "Bhoonidhi S-RSLC 快照元数据包含敏感字段",
            "sensitive_metadata",
        )
    })?;
    let item = SnapshotItem {
        item_id,
        provider_id: product.provider_id,
        display_name: product.display_name,
        acquisition_datetime: Some(
            acquisition_datetime.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        ),
        footprint_geojson: product.footprint_geojson,
        metadata,
        payload: Map::new(),
        asset_access: Some(access),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
            collection_profile_id: Some(BHOONIDHI_PROFILE_ID.to_owned()),
        }),
    };
    item.validate().map_err(|_| {
        catalogue_provider_error(
            "BHOONIDHI_PRODUCT_INVALID",
            "Bhoonidhi S-RSLC 快照项目无效",
            "snapshot_item",
        )
    })?;
    if serde_json::to_vec(&item)
        .map(|bytes| bytes.len() > MAX_CATALOGUE_CANDIDATE_JSON_BYTES)
        .unwrap_or(true)
    {
        return Err(catalogue_provider_error(
            "BHOONIDHI_PRODUCT_INVALID",
            "Bhoonidhi S-RSLC 快照项目超出离线限制",
            "candidate_size",
        ));
    }
    Ok(item)
}

struct FrozenNisarGraph {
    assets: Value,
    scientific_properties: Map<String, Value>,
    primary_asset_key: String,
    primary_stable_asset_key: String,
    primary_descriptor: AssetAccessDescriptor,
    primary_file_name: String,
    primary_size_bytes: u64,
    primary_checksum_algorithm: String,
    primary_checksum_value: String,
    browse_available: bool,
}

fn nisar_asset_error(reason: &'static str) -> ApplicationServiceError {
    catalogue_provider_error(
        "ASF_ASSET_CLASSIFICATION_INVALID",
        "NISAR 资产图无法安全冻结",
        reason,
    )
}

fn nisar_graph_limit_error(reason: &'static str) -> ApplicationServiceError {
    catalogue_provider_error(
        "ASF_ASSET_GRAPH_LIMIT_EXCEEDED",
        "NISAR 资产图超过冻结边界",
        reason,
    )
}

fn nisar_role_name(role: &AssetRole) -> Option<&'static str> {
    match role {
        AssetRole::Primary => Some("primary"),
        AssetRole::Preview => Some("preview"),
        AssetRole::Browse => Some("browse"),
        AssetRole::Thumbnail => Some("thumbnail"),
        AssetRole::Metadata => Some("metadata"),
        AssetRole::Auxiliary => Some("auxiliary"),
        AssetRole::Derived => None,
    }
}

fn exact_keys(value: &Map<String, Value>, expected: &[&str]) -> bool {
    value.len() == expected.len() && expected.iter().all(|key| value.contains_key(*key))
}

fn safe_nisar_graph_text(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_bytes
        && value == value.trim()
        && !value.chars().any(char::is_control)
        && !value.contains('\0')
}

fn safe_nisar_basename(value: &str) -> bool {
    safe_nisar_graph_text(value, NISAR_MAX_FILE_NAME_BYTES)
        && !value.contains(['/', '\\'])
        && !matches!(value, "." | "..")
}

fn nisar_graph_scope_is_safe(value: &Value) -> bool {
    match value {
        Value::String(value) => {
            let value = value.to_ascii_lowercase();
            !value.contains("s-band")
                && !value.contains("s_band")
                && !value.contains("bhoonidhi")
                && !value.contains("/private/")
        }
        Value::Array(values) => values.iter().all(nisar_graph_scope_is_safe),
        Value::Object(values) => values.values().all(nisar_graph_scope_is_safe),
        _ => true,
    }
}

fn nisar_route_matches_file(scheme: &str, locator: &str, file_name: &str) -> bool {
    if !safe_nisar_graph_text(locator, NISAR_MAX_LOCATOR_BYTES)
        || locator.contains(['?', '#', '\\'])
        || locator
            .rsplit('/')
            .next()
            .is_none_or(|value| value != file_name)
    {
        return false;
    }
    let prefixes = match scheme {
        "https" => [
            "https://nisar.asf.earthdatacloud.nasa.gov/",
            "https://example.invalid/",
        ]
        .as_slice(),
        "s3" => [
            "s3://sds-n-cumulus-prod-nisar-products/",
            "s3://sds-n-cumulus-prod-nisar-browse/",
            "s3://example-invalid/",
        ]
        .as_slice(),
        _ => return false,
    };
    prefixes.iter().any(|prefix| locator.starts_with(prefix))
}

fn nisar_checksum(value: &Value) -> Result<Option<(String, String)>, ApplicationServiceError> {
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_object()
        .filter(|value| exact_keys(value, &["algorithm", "value"]))
        .ok_or_else(|| nisar_asset_error("checksum"))?;
    let algorithm = value
        .get("algorithm")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "MD5" | "SHA-256" | "SHA256"))
        .ok_or_else(|| nisar_asset_error("checksum"))?;
    let checksum = value
        .get("value")
        .and_then(Value::as_str)
        .filter(|value| {
            let expected = if algorithm == "MD5" { 32 } else { 64 };
            value.len() == expected && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .ok_or_else(|| nisar_asset_error("checksum"))?;
    Ok(Some((algorithm.to_owned(), checksum.to_ascii_lowercase())))
}

fn validate_nisar_native_evidence(
    value: &Value,
    file_name: &str,
    role: &AssetRole,
) -> Result<bool, ApplicationServiceError> {
    let evidence = value
        .as_object()
        .filter(|value| {
            exact_keys(
                value,
                &[
                    "archive_name",
                    "product_match",
                    "url_types",
                    "url_subtypes",
                    "formats",
                ],
            )
        })
        .ok_or_else(|| nisar_asset_error("native_evidence"))?;
    let archive_name = match evidence.get("archive_name") {
        Some(Value::Null) => None,
        Some(Value::String(value)) if value == file_name => Some(value.as_str()),
        _ => return Err(nisar_asset_error("native_evidence")),
    };
    let product_match = evidence
        .get("product_match")
        .and_then(Value::as_bool)
        .ok_or_else(|| nisar_asset_error("native_evidence"))?;
    for key in ["url_types", "url_subtypes", "formats"] {
        let values = evidence
            .get(key)
            .and_then(Value::as_array)
            .filter(|values| values.len() <= NISAR_MAX_NATIVE_ROLES)
            .ok_or_else(|| nisar_asset_error("native_evidence"))?;
        if values.iter().any(|value| {
            value
                .as_str()
                .is_none_or(|value| !safe_nisar_graph_text(value, NISAR_MAX_NATIVE_ROLE_BYTES))
        }) {
            return Err(nisar_asset_error("native_evidence"));
        }
    }
    let valid_identity = match role {
        AssetRole::Primary => archive_name.is_some() && product_match,
        AssetRole::Preview | AssetRole::Browse | AssetRole::Thumbnail => !product_match,
        AssetRole::Metadata | AssetRole::Auxiliary => archive_name.is_some() && !product_match,
        AssetRole::Derived => false,
    };
    valid_identity
        .then_some(product_match)
        .ok_or_else(|| nisar_asset_error("native_evidence"))
}

fn validate_nisar_asset_graph(
    product: &ProviderProduct,
    item_id: &str,
) -> Result<FrozenNisarGraph, ApplicationServiceError> {
    let primary_assets = product
        .assets
        .iter()
        .filter(|asset| asset.role == AssetRole::Primary)
        .collect::<Vec<_>>();
    let primary_asset = match primary_assets.as_slice() {
        [] => {
            return Err(catalogue_provider_error(
                "ASF_PRIMARY_ASSET_MISSING",
                "NISAR Granule 缺少主科学 HDF5",
                "primary_asset",
            ));
        }
        [asset] => *asset,
        _ => {
            return Err(catalogue_provider_error(
                "ASF_PRIMARY_ASSET_AMBIGUOUS",
                "NISAR Granule 包含多个主科学 HDF5",
                "primary_asset",
            ));
        }
    };

    let graph = product
        .properties
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| nisar_asset_error("assets"))?;
    if graph.len() > NISAR_MAX_DEDUPLICATED_ASSETS
        || product.assets.len() > NISAR_MAX_DEDUPLICATED_ASSETS
    {
        return Err(nisar_graph_limit_error("deduplicated_asset_count"));
    }
    if serde_json::to_vec(graph)
        .map(|bytes| bytes.len() > NISAR_MAX_ASSET_GRAPH_JSON_BYTES)
        .unwrap_or(true)
    {
        return Err(nisar_graph_limit_error("metadata_assets_json_bytes"));
    }
    if graph.len() != product.assets.len()
        || !nisar_graph_scope_is_safe(&Value::Array(graph.clone()))
    {
        return Err(nisar_asset_error("assets"));
    }
    validate_debug_context(&Map::from_iter([(
        "assets".to_owned(),
        Value::Array(graph.clone()),
    )]))
    .map_err(|_| nisar_asset_error("assets"))?;

    let integrity = product
        .properties
        .get("asset_integrity")
        .and_then(Value::as_object)
        .ok_or_else(|| nisar_asset_error("asset_integrity"))?;
    let route_graph = product
        .properties
        .get("asset_routes")
        .and_then(Value::as_object)
        .ok_or_else(|| nisar_asset_error("asset_routes"))?;
    if route_graph.len() != graph.len() {
        return Err(nisar_asset_error("asset_routes"));
    }

    let mut providers = BTreeMap::<String, &ProviderAsset>::new();
    for asset in &product.assets {
        let asset_key = asset
            .asset_key
            .as_deref()
            .filter(|value| safe_nisar_graph_text(value, NISAR_MAX_FILE_NAME_BYTES))
            .ok_or_else(|| nisar_asset_error("asset_key"))?;
        if providers.insert(asset_key.to_owned(), asset).is_some() || asset.validate().is_err() {
            return Err(nisar_asset_error("asset_key"));
        }
    }

    let mut graph_keys = BTreeSet::new();
    let mut graph_primary_count = 0usize;
    let mut checksummed_count = 0usize;
    let mut represented_raw_resources = 0usize;
    let mut browse_available = false;
    let mut primary_freeze = None;
    for value in graph {
        let value = value
            .as_object()
            .filter(|value| {
                exact_keys(
                    value,
                    &[
                        "asset_key",
                        "stable_asset_key",
                        "role",
                        "native_roles",
                        "file_name",
                        "media_type",
                        "size_bytes",
                        "checksum",
                        "routes",
                        "native_evidence",
                    ],
                )
            })
            .ok_or_else(|| nisar_asset_error("asset"))?;
        let asset_key = value
            .get("asset_key")
            .and_then(Value::as_str)
            .filter(|value| safe_nisar_graph_text(value, NISAR_MAX_FILE_NAME_BYTES))
            .ok_or_else(|| nisar_asset_error("asset_key"))?;
        if !graph_keys.insert(asset_key.to_owned()) {
            return Err(nisar_asset_error("asset_key"));
        }
        let provider = providers
            .get(asset_key)
            .copied()
            .ok_or_else(|| nisar_asset_error("asset_key"))?;
        let stable_key = value
            .get("stable_asset_key")
            .and_then(Value::as_str)
            .filter(|value| valid_exact_text(value))
            .ok_or_else(|| nisar_asset_error("stable_asset_key"))?;
        let expected_stable_key = stable_asset_key(item_id, asset_key)
            .map_err(|_| nisar_asset_error("stable_asset_key"))?;
        let role = value
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| nisar_asset_error("role"))?;
        if nisar_role_name(&provider.role) != Some(role)
            || provider.stable_asset_key.as_deref() != Some(stable_key)
            || stable_key != expected_stable_key
        {
            return Err(nisar_asset_error("asset_identity"));
        }
        let native_roles = value
            .get("native_roles")
            .and_then(Value::as_array)
            .filter(|values| values.len() <= NISAR_MAX_NATIVE_ROLES)
            .ok_or_else(|| nisar_asset_error("native_roles"))?;
        let native_roles = native_roles
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| safe_nisar_graph_text(value, NISAR_MAX_NATIVE_ROLE_BYTES))
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| nisar_asset_error("native_roles"))?;
        if native_roles
            != provider
                .native_roles
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        {
            return Err(nisar_asset_error("native_roles"));
        }
        let file_name = value
            .get("file_name")
            .and_then(Value::as_str)
            .filter(|value| safe_nisar_basename(value))
            .ok_or_else(|| nisar_asset_error("file_name"))?;
        let media_type = value
            .get("media_type")
            .and_then(Value::as_str)
            .filter(|value| {
                safe_nisar_graph_text(value, NISAR_MAX_MEDIA_TYPE_BYTES)
                    && value.split_once('/').is_some()
            })
            .ok_or_else(|| nisar_asset_error("media_type"))?;
        let size_bytes = match value.get("size_bytes") {
            Some(Value::Null) => None,
            Some(value) => value.as_u64().filter(|value| *value > 0),
            None => None,
        };
        if provider.file_name.as_deref() != Some(file_name)
            || provider.media_type.as_deref() != Some(media_type)
            || provider
                .size_bytes
                .and_then(|value| u64::try_from(value).ok())
                != size_bytes
        {
            return Err(nisar_asset_error("asset_fields"));
        }

        let checksum = nisar_checksum(
            value
                .get("checksum")
                .ok_or_else(|| nisar_asset_error("checksum"))?,
        )?;
        if size_bytes.is_some() != checksum.is_some() {
            return Err(nisar_asset_error("asset_integrity"));
        }
        match &checksum {
            Some((algorithm, checksum_value)) => {
                checksummed_count += 1;
                let evidence = integrity
                    .get(asset_key)
                    .and_then(Value::as_object)
                    .filter(|value| {
                        exact_keys(value, &["file_name", "size_bytes", "algorithm", "value"])
                    })
                    .ok_or_else(|| nisar_asset_error("asset_integrity"))?;
                if evidence.get("file_name").and_then(Value::as_str) != Some(file_name)
                    || evidence.get("size_bytes").and_then(Value::as_u64) != size_bytes
                    || evidence.get("algorithm").and_then(Value::as_str) != Some(algorithm.as_str())
                    || evidence
                        .get("value")
                        .and_then(Value::as_str)
                        .is_none_or(|value| !value.eq_ignore_ascii_case(checksum_value))
                {
                    return Err(nisar_asset_error("asset_integrity"));
                }
            }
            None if integrity.contains_key(asset_key) => {
                return Err(nisar_asset_error("asset_integrity"));
            }
            None => {}
        }

        let routes = value
            .get("routes")
            .and_then(Value::as_array)
            .filter(|values| values.len() <= NISAR_MAX_RAW_RESOURCE_ENTRIES)
            .ok_or_else(|| nisar_asset_error("asset_routes"))?;
        if route_graph.get(asset_key) != Some(&Value::Array(routes.clone())) {
            return Err(nisar_asset_error("asset_routes"));
        }
        let mut route_set = BTreeSet::new();
        let mut https_routes = Vec::new();
        for route in routes {
            let route = route
                .as_object()
                .filter(|value| exact_keys(value, &["scheme", "locator"]))
                .ok_or_else(|| nisar_asset_error("asset_routes"))?;
            let (Some(scheme), Some(locator)) = (
                route.get("scheme").and_then(Value::as_str),
                route.get("locator").and_then(Value::as_str),
            ) else {
                return Err(nisar_asset_error("asset_routes"));
            };
            if !nisar_route_matches_file(scheme, locator, file_name)
                || !route_set.insert((scheme.to_owned(), locator.to_owned()))
            {
                return Err(nisar_asset_error("asset_routes"));
            }
            if scheme == "https" {
                https_routes.push(locator);
            }
        }
        let product_match = validate_nisar_native_evidence(
            value
                .get("native_evidence")
                .ok_or_else(|| nisar_asset_error("native_evidence"))?,
            file_name,
            &provider.role,
        )?;
        let archived = value["native_evidence"]["archive_name"].is_string();
        represented_raw_resources = represented_raw_resources
            .checked_add(routes.len() + usize::from(archived))
            .ok_or_else(|| nisar_graph_limit_error("represented_raw_resources"))?;
        if represented_raw_resources > NISAR_MAX_RAW_RESOURCE_ENTRIES {
            return Err(nisar_graph_limit_error("represented_raw_resources"));
        }

        match provider.role {
            AssetRole::Primary => {
                graph_primary_count += 1;
                let descriptor = provider
                    .access_descriptor
                    .as_ref()
                    .filter(|descriptor| {
                        descriptor.resolver_id == "earthdata_https"
                            && descriptor.scheme == AssetAccessScheme::EarthdataHttps
                            && descriptor.requires_credentials
                            && descriptor.credential_profile.as_deref() == Some("earthdata-default")
                            && descriptor.validate().is_ok()
                    })
                    .ok_or_else(|| nisar_asset_error("primary_descriptor"))?;
                let expected_file_name = format!("{}.h5", product.display_name);
                let (Some(size_bytes), Some((algorithm, checksum_value))) =
                    (size_bytes, checksum.clone())
                else {
                    return Err(nisar_asset_error("primary_integrity"));
                };
                if asset_key
                    != product.properties["primary_asset_key"]
                        .as_str()
                        .unwrap_or_default()
                    || file_name != expected_file_name
                    || media_type != "application/x-hdf5"
                    || !product_match
                    || provider.url.is_some()
                    || https_routes.as_slice()
                        != [descriptor.stable_href.as_deref().unwrap_or_default()]
                {
                    return Err(nisar_asset_error("primary_asset"));
                }
                primary_freeze = Some((
                    asset_key.to_owned(),
                    stable_key.to_owned(),
                    descriptor.clone(),
                    file_name.to_owned(),
                    size_bytes,
                    if algorithm == "SHA-256" {
                        "SHA256".to_owned()
                    } else {
                        algorithm
                    },
                    checksum_value,
                ));
            }
            AssetRole::Preview | AssetRole::Browse | AssetRole::Thumbnail => {
                if provider.access_descriptor.is_some()
                    || https_routes.len() != 1
                    || provider.url.as_deref() != https_routes.first().copied()
                {
                    return Err(nisar_asset_error("preview_asset"));
                }
                browse_available |= matches!(provider.role, AssetRole::Preview | AssetRole::Browse);
            }
            AssetRole::Metadata | AssetRole::Auxiliary => {
                if provider.access_descriptor.is_some() || provider.url.is_some() {
                    return Err(nisar_asset_error("non_executable_asset"));
                }
            }
            AssetRole::Derived => return Err(nisar_asset_error("derived_asset")),
        }
    }
    if graph_primary_count == 0 {
        return Err(catalogue_provider_error(
            "ASF_PRIMARY_ASSET_MISSING",
            "NISAR Granule 缺少主科学 HDF5",
            "primary_asset",
        ));
    }
    if graph_primary_count > 1 {
        return Err(catalogue_provider_error(
            "ASF_PRIMARY_ASSET_AMBIGUOUS",
            "NISAR Granule 包含多个主科学 HDF5",
            "primary_asset",
        ));
    }
    if graph_keys.len() != providers.len() || integrity.len() != checksummed_count {
        return Err(nisar_asset_error("asset_graph_closure"));
    }
    let Some((
        primary_asset_key,
        primary_stable_asset_key,
        primary_descriptor,
        primary_file_name,
        primary_size_bytes,
        primary_checksum_algorithm,
        primary_checksum_value,
    )) = primary_freeze
    else {
        return Err(nisar_asset_error("primary_asset"));
    };
    if primary_asset.asset_key.as_deref() != Some(primary_asset_key.as_str()) {
        return Err(nisar_asset_error("primary_asset"));
    }

    let mut scientific_properties = product.properties.clone();
    scientific_properties.remove("assets");
    scientific_properties.remove("asset_integrity");
    scientific_properties.remove("asset_routes");
    Ok(FrozenNisarGraph {
        assets: Value::Array(graph.clone()),
        scientific_properties,
        primary_asset_key,
        primary_stable_asset_key,
        primary_descriptor,
        primary_file_name,
        primary_size_bytes,
        primary_checksum_algorithm,
        primary_checksum_value,
        browse_available,
    })
}

fn nisar_property_text<'a>(
    properties: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, ApplicationServiceError> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| safe_nisar_graph_text(value, NISAR_MAX_FILE_NAME_BYTES))
        .ok_or_else(|| {
            catalogue_provider_error("CATALOGUE_PRODUCT_INVALID", "NISAR 产品权威属性无效", key)
        })
}

fn nisar_product_matches_query(
    properties: &Map<String, Value>,
    acquisition_start: DateTime<FixedOffset>,
    query: &CatalogSearchQuery,
) -> bool {
    for key in [
        "production_configuration",
        "frame_coverage",
        "frequency_a_polarization",
        "frequency_b_polarization",
        "range_bandwidth",
        "flight_direction",
        "look_direction",
    ] {
        if let Some(expected) = query.filters.get(key).and_then(Value::as_str) {
            if properties.get(key).and_then(Value::as_str) != Some(expected) {
                return false;
            }
        }
    }
    if let Some(expected) = query
        .filters
        .get("joint_observation")
        .and_then(Value::as_bool)
    {
        if properties.get("joint_observation").and_then(Value::as_bool) != Some(expected) {
            return false;
        }
    }
    for key in ["track", "relative_orbit", "frame"] {
        if let Some(expected) = query.filters.get(key).and_then(Value::as_u64) {
            let property = if key == "relative_orbit" {
                "relative_orbit"
            } else {
                key
            };
            if properties.get(property).and_then(Value::as_u64) != Some(expected) {
                return false;
            }
        }
    }
    if let Some(window) = query.filters.get("datetime").and_then(Value::as_object) {
        let start = window
            .get("start")
            .and_then(Value::as_str)
            .and_then(parse_normalized_utc);
        let end = window
            .get("end")
            .and_then(Value::as_str)
            .and_then(parse_normalized_utc);
        if !start
            .zip(end)
            .is_some_and(|(start, end)| start <= acquisition_start && acquisition_start < end)
        {
            return false;
        }
    }
    true
}

fn map_nisar_rslc_catalogue_product(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
) -> Result<SnapshotItem, ApplicationServiceError> {
    const REQUIRED_PROPERTIES: &[&str] = &[
        "provider_product_key",
        "provider_product_id",
        "cmr_granule_concept_id",
        "producer_granule_id",
        "logical_dataset",
        "collection_concept_id",
        "collection_short_name",
        "collection_version",
        "collection_profile_id",
        "identity_rule_id",
        "filename_rule_id",
        "resolver_policy_id",
        "processing_level",
        "science_level",
        "quality_status",
        "production_configuration",
        "band",
        "frame_coverage",
        "frame",
        "track",
        "relative_orbit",
        "flight_direction",
        "look_direction",
        "range_bandwidth",
        "joint_observation",
        "acquisition_end_utc",
        "primary_asset_key",
        "asset_integrity",
        "asset_routes",
        "assets",
        "preview_available",
        "full_product",
        "crid",
        "product_specification_version",
        "product_specification_profile_registry_version",
    ];
    const OPTIONAL_PROPERTIES: &[&str] = &[
        "asf_product_id",
        "pge_version",
        "frequency_a_polarization",
        "frequency_b_polarization",
    ];
    let profile = CollectionProfileRegistry
        .for_logical_dataset(NISAR_LOGICAL_DATASET)
        .expect("NISAR profile is statically registered");
    if query.validate().is_err()
        || query.logical_dataset != NISAR_LOGICAL_DATASET
        || query.provider_id != NISAR_PROVIDER_ID
        || query.native_collection_id != NISAR_NATIVE_COLLECTION_ID
        || product.provider_id != NISAR_PROVIDER_ID
        || product.source_kind != DataSourceKind::Sar
        || product.product_kind != ProductKind::SarProduct
        || !valid_alos2_concept_id(&product.product_id)
        || !valid_catalogue_display_name(&product.display_name)
        || product.footprint_geojson.is_none()
        || product.footprint_bbox.is_some()
        || !product.payload.is_empty()
        || product.validate().is_err()
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "NISAR 目录产品包含不受支持的字段",
            "nisar_product",
        ));
    }
    let properties = &product.properties;
    if properties
        .get("crid")
        .and_then(Value::as_str)
        .is_none_or(|value| !safe_nisar_graph_text(value, NISAR_MAX_FILE_NAME_BYTES))
        || properties
            .get("product_specification_version")
            .and_then(Value::as_str)
            .is_none_or(|value| !safe_nisar_graph_text(value, NISAR_MAX_FILE_NAME_BYTES))
        || properties
            .get("product_specification_profile_registry_version")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .is_none()
    {
        return Err(catalogue_provider_error(
            "ASF_PRODUCT_SCOPE_MISMATCH",
            "NISAR 缺少受支持的 CRID/产品规范版本",
            "product_specification_profile",
        ));
    }
    if REQUIRED_PROPERTIES
        .iter()
        .any(|key| !properties.contains_key(*key))
        || properties.keys().any(|key| {
            !REQUIRED_PROPERTIES.contains(&key.as_str())
                && !OPTIONAL_PROPERTIES.contains(&key.as_str())
        })
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "NISAR 产品属性集合不闭合",
            "properties",
        ));
    }
    if properties.get("quality_status").and_then(Value::as_str) != Some(NISAR_QUALITY_STATUS) {
        return Err(catalogue_provider_error(
            "NISAR_QUALITY_STATUS_MISMATCH",
            "NISAR 产品不是 Provisional 质量状态",
            "quality_status",
        ));
    }
    let crid = nisar_property_text(properties, "crid")?;
    let product_specification_version =
        nisar_property_text(properties, "product_specification_version")?;
    let product_specification_profile_registry_version = properties
        .get("product_specification_profile_registry_version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            catalogue_provider_error(
                "ASF_PRODUCT_SCOPE_MISMATCH",
                "NISAR 缺少受支持的产品规范 profile registry 版本",
                "product_specification_profile_registry_version",
            )
        })?;
    let product_specification_profile = NisarRslcProductSpecificationProfileRegistry
        .for_crid(crid)
        .filter(|profile| {
            profile.product_specification_version == product_specification_version
                && profile.registry_version == product_specification_profile_registry_version
                && profile.resolver_policy_id == NISAR_RESOLVER_POLICY_ID
        })
        .ok_or_else(|| {
            catalogue_provider_error(
                "ASF_PRODUCT_SCOPE_MISMATCH",
                "NISAR CRID 与产品规范版本不匹配",
                "product_specification_profile",
            )
        })?;
    let frequency_a = properties
        .get("frequency_a_polarization")
        .and_then(Value::as_str);
    let frequency_b = properties
        .get("frequency_b_polarization")
        .and_then(Value::as_str);
    let scope_matches = properties.get("processing_level").and_then(Value::as_str)
        == Some(NISAR_REQUIRED_PRODUCT_TYPE)
        && properties.get("science_level").and_then(Value::as_str) == Some(NISAR_SCIENCE_LEVEL)
        && properties
            .get("production_configuration")
            .and_then(Value::as_str)
            == Some(NISAR_PRODUCTION_CONFIGURATION)
        && properties.get("band").and_then(Value::as_str) == Some(NISAR_BAND)
        && properties
            .get("frame_coverage")
            .and_then(Value::as_str)
            .is_some_and(|value| NISAR_FRAME_COVERAGES.contains(&value))
        && properties
            .get("flight_direction")
            .and_then(Value::as_str)
            .is_some_and(|value| NISAR_FLIGHT_DIRECTIONS.contains(&value))
        && properties
            .get("look_direction")
            .and_then(Value::as_str)
            .is_some_and(|value| NISAR_LOOK_DIRECTIONS.contains(&value))
        && properties
            .get("range_bandwidth")
            .and_then(Value::as_str)
            .is_some_and(|value| NISAR_RANGE_BANDWIDTHS.contains(&value))
        && properties
            .get("joint_observation")
            .and_then(Value::as_bool)
            .is_some()
        && frequency_a
            .map(|value| NISAR_FREQUENCY_A_POLARIZATIONS.contains(&value))
            .unwrap_or(true)
        && frequency_b
            .map(|value| NISAR_FREQUENCY_B_POLARIZATIONS.contains(&value))
            .unwrap_or(true)
        && (frequency_a.is_some() || frequency_b.is_some());
    if !scope_matches {
        return Err(catalogue_provider_error(
            "ASF_PRODUCT_SCOPE_MISMATCH",
            "NISAR 产品超出 Provisional L-band RSLC 范围",
            "nisar_scope",
        ));
    }

    let item_id = provider_product_key(
        NISAR_PROVIDER_ID,
        NISAR_NATIVE_COLLECTION_ID,
        &product.product_id,
    )
    .map_err(|_| {
        catalogue_provider_error(
            "ASF_PRODUCT_IDENTITY_MISSING",
            "NISAR Granule 身份无效",
            "provider_product_key",
        )
    })?;
    let collection_matches = nisar_property_text(properties, "logical_dataset")?
        == NISAR_LOGICAL_DATASET
        && nisar_property_text(properties, "collection_concept_id")? == NISAR_NATIVE_COLLECTION_ID
        && nisar_property_text(properties, "collection_short_name")? == NISAR_COLLECTION_SHORT_NAME
        && nisar_property_text(properties, "collection_version")? == NISAR_COLLECTION_VERSION
        && nisar_property_text(properties, "collection_profile_id")? == NISAR_PROFILE_ID
        && nisar_property_text(properties, "identity_rule_id")? == NISAR_IDENTITY_RULE_ID
        && nisar_property_text(properties, "filename_rule_id")? == NISAR_FILENAME_RULE_ID
        && nisar_property_text(properties, "resolver_policy_id")?
            == product_specification_profile.resolver_policy_id;
    if !collection_matches {
        return Err(catalogue_provider_error(
            "ASF_COLLECTION_IDENTITY_MISMATCH",
            "NISAR Collection 身份冲突",
            "collection_identity",
        ));
    }
    let product_identity_matches = nisar_property_text(properties, "provider_product_key")?
        == item_id
        && nisar_property_text(properties, "provider_product_id")? == product.product_id
        && nisar_property_text(properties, "cmr_granule_concept_id")? == product.product_id
        && nisar_property_text(properties, "producer_granule_id")? == product.display_name;
    if !product_identity_matches {
        return Err(catalogue_provider_error(
            "ASF_PRODUCT_IDENTITY_MISSING",
            "NISAR Granule 身份证据冲突",
            "product_identity",
        ));
    }
    for key in ["asf_product_id", "pge_version"] {
        if properties.get(key).is_some_and(|value| {
            value
                .as_str()
                .is_none_or(|value| !safe_nisar_graph_text(value, NISAR_MAX_FILE_NAME_BYTES))
        }) {
            return Err(catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "NISAR 可选身份属性无效",
                "optional_identity",
            ));
        }
    }

    let acquisition_start = product
        .acquisition_datetime
        .as_deref()
        .and_then(parse_normalized_utc)
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "NISAR 产品获取时间无效",
                "acquisition_start_utc",
            )
        })?;
    let _acquisition_end =
        parse_normalized_utc(nisar_property_text(properties, "acquisition_end_utc")?)
            .filter(|value| *value >= acquisition_start)
            .ok_or_else(|| {
                catalogue_provider_error(
                    "CATALOGUE_PRODUCT_INVALID",
                    "NISAR 产品获取区间无效",
                    "acquisition_end_utc",
                )
            })?;
    let frame = properties.get("frame").and_then(Value::as_u64);
    let track = properties.get("track").and_then(Value::as_u64);
    let relative_orbit = properties.get("relative_orbit").and_then(Value::as_u64);
    if frame.is_none_or(|value| value == 0 || value > i32::MAX as u64)
        || track.is_none_or(|value| value == 0 || value > i32::MAX as u64)
        || relative_orbit != track
        || !nisar_product_matches_query(properties, acquisition_start, query)
    {
        return Err(catalogue_provider_error(
            "ASF_PRODUCT_SCOPE_MISMATCH",
            "NISAR 产品与冻结检索范围不一致",
            "query_scope",
        ));
    }

    let graph = validate_nisar_asset_graph(&product, &item_id)?;
    if properties.get("preview_available").and_then(Value::as_bool) != Some(graph.browse_available)
        || properties.get("full_product").and_then(Value::as_bool) != Some(true)
    {
        return Err(nisar_asset_error("product_projection"));
    }
    validate_debug_context(&graph.scientific_properties).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "NISAR 科学属性包含敏感字段",
            "scientific_properties",
        )
    })?;

    let mut metadata = Map::from_iter([
        (
            "dataset".to_owned(),
            Value::String(NISAR_LOGICAL_DATASET.to_owned()),
        ),
        (
            "logical_dataset".to_owned(),
            Value::String(NISAR_LOGICAL_DATASET.to_owned()),
        ),
        (
            "native_collection_id".to_owned(),
            Value::String(NISAR_NATIVE_COLLECTION_ID.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(NISAR_PROFILE_ID.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(NISAR_IDENTITY_RULE_ID.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(NISAR_FILENAME_RULE_ID.to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            serde_json::to_value(profile.delivery_model.clone())
                .expect("closed delivery model serializes"),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(NISAR_RESOLVER_POLICY_ID.to_owned()),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product.product_id.clone()),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(item_id.clone()),
        ),
        (
            "product_family".to_owned(),
            Value::String(NISAR_COLLECTION_SHORT_NAME.to_owned()),
        ),
        (
            "processing_level".to_owned(),
            Value::String(NISAR_REQUIRED_PRODUCT_TYPE.to_owned()),
        ),
        (
            "quality_status".to_owned(),
            Value::String(NISAR_QUALITY_STATUS.to_owned()),
        ),
        ("source_kind".to_owned(), Value::String("sar".to_owned())),
        (
            "product_kind".to_owned(),
            Value::String("sar_product".to_owned()),
        ),
        (
            "properties".to_owned(),
            Value::Object(graph.scientific_properties),
        ),
        ("assets".to_owned(), graph.assets),
    ]);
    for key in [
        "band",
        "science_level",
        "production_configuration",
        "crid",
        "product_specification_version",
        "frequency_a_polarization",
        "frequency_b_polarization",
    ] {
        if let Some(value) = properties.get(key) {
            metadata.insert(key.to_owned(), value.clone());
        }
    }
    validate_debug_context(&metadata).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "NISAR 快照元数据包含敏感字段",
            "sensitive_metadata",
        )
    })?;
    let access = FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: NISAR_PROVIDER_ID.to_owned(),
            collection_id: NISAR_NATIVE_COLLECTION_ID.to_owned(),
            provider_product_id: product.product_id.clone(),
            provider_product_key: item_id.clone(),
            canonical_product_key: None,
            asset_key: graph.primary_asset_key,
            stable_asset_key: graph.primary_stable_asset_key,
        },
        descriptor: graph.primary_descriptor,
        role: AssetRole::Primary,
        file_name: Some(graph.primary_file_name),
        expected_size_bytes: Some(graph.primary_size_bytes),
        checksum_algorithm: Some(graph.primary_checksum_algorithm),
        checksum_value: Some(graph.primary_checksum_value),
    };
    let item = SnapshotItem {
        item_id,
        provider_id: product.provider_id,
        display_name: product.display_name,
        acquisition_datetime: Some(acquisition_start.to_rfc3339_opts(SecondsFormat::AutoSi, true)),
        footprint_geojson: product.footprint_geojson,
        metadata,
        payload: Map::new(),
        asset_access: Some(access),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: NISAR_RESOLVER_POLICY_ID.to_owned(),
            collection_profile_id: Some(NISAR_PROFILE_ID.to_owned()),
        }),
    };
    item.validate().map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "NISAR 快照项目无效",
            "snapshot_item",
        )
    })?;
    if serde_json::to_vec(&item)
        .map(|bytes| bytes.len() > MAX_CATALOGUE_CANDIDATE_JSON_BYTES)
        .unwrap_or(true)
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "NISAR 快照项目超出离线限制",
            "candidate_size",
        ));
    }
    Ok(item)
}

/// Reuse the production-shaped NISAR mapper from the bounded acceptance test.
/// This is a doc-hidden seam only: normal ApplicationService catalogue routing
/// and production enablement remain unchanged.
#[doc(hidden)]
pub fn map_nisar_product_for_live_acceptance(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
) -> Result<SnapshotItem, ApplicationServiceError> {
    map_nisar_rslc_catalogue_product(product, query)
}

fn valid_alos2_concept_id(value: &str) -> bool {
    value
        .strip_prefix('G')
        .and_then(|value| value.strip_suffix("-ASF"))
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn valid_alos2_zip_basename(value: &str) -> bool {
    valid_exact_text(value)
        && value.len() <= 512
        && !value.contains(['/', '\\', '\0'])
        && value.to_ascii_lowercase().ends_with(".zip")
        && !matches!(value, "." | "..")
}

fn valid_alos2_route_locator(scheme: &str, locator: &str) -> bool {
    let prefix = match scheme {
        "https" => "https://",
        "s3" => "s3://",
        _ => return false,
    };
    let Some(authority_and_path) = locator.strip_prefix(prefix) else {
        return false;
    };
    let authority = authority_and_path.split('/').next().unwrap_or_default();
    let host = authority.strip_suffix(":443").unwrap_or(authority);
    let authority_is_valid = !host.is_empty()
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'));
    let port_is_valid = match scheme {
        "https" => !authority.contains(':') || authority.ends_with(":443"),
        "s3" => !authority.contains(':'),
        _ => unreachable!("closed ALOS-2 route scheme is checked above"),
    };
    valid_exact_text(locator)
        && !authority.is_empty()
        && authority_is_valid
        && port_is_valid
        && !authority.contains('@')
        && !locator.contains(['?', '#'])
}

fn alos2_route_matches_archive(scheme: &str, locator: &str, file_name: &str) -> bool {
    valid_alos2_route_locator(scheme, locator)
        && locator
            .rsplit('/')
            .next()
            .is_some_and(|value| value == file_name)
}

fn alos2_property_text<'a>(
    properties: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, ApplicationServiceError> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| valid_exact_text(value))
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "ALOS-2 产品权威属性无效",
                "alos2_properties",
            )
        })
}

fn map_alos2_catalogue_product(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
) -> Result<SnapshotItem, ApplicationServiceError> {
    let profile = CollectionProfileRegistry
        .for_logical_dataset(ALOS2_LOGICAL_DATASET)
        .expect("ALOS-2 profile is statically registered");
    if query.provider_id != ALOS2_PROVIDER_ID
        || query.native_collection_id != ALOS2_NATIVE_COLLECTION_ID
        || product.provider_id != ALOS2_PROVIDER_ID
        || product.source_kind != DataSourceKind::Sar
        || product.product_kind != ProductKind::SarProduct
        || !valid_alos2_concept_id(&product.product_id)
        || !valid_catalogue_display_name(&product.display_name)
        || product.footprint_geojson.is_none()
        || product.footprint_bbox.is_some()
        || !product.payload.is_empty()
        || product.validate().is_err()
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "ALOS-2 目录产品包含不受支持的字段",
            "alos2_product",
        ));
    }
    validate_debug_context(&product.properties).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "ALOS-2 产品属性包含敏感字段",
            "sensitive_properties",
        )
    })?;
    let acquisition_datetime = product
        .acquisition_datetime
        .as_deref()
        .and_then(parse_normalized_utc)
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "ALOS-2 产品获取时间无效",
                "acquisition_datetime",
            )
        })?;
    let properties = &product.properties;
    let item_id = provider_product_key(
        ALOS2_PROVIDER_ID,
        ALOS2_NATIVE_COLLECTION_ID,
        &product.product_id,
    )
    .map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "ALOS-2 产品身份无效",
            "provider_product_key",
        )
    })?;
    let acquisition_end =
        parse_normalized_utc(alos2_property_text(properties, "acquisition_end_utc")?).ok_or_else(
            || {
                catalogue_provider_error(
                    "CATALOGUE_PRODUCT_INVALID",
                    "ALOS-2 产品获取区间无效",
                    "acquisition_end_utc",
                )
            },
        )?;
    if acquisition_end < acquisition_datetime {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "ALOS-2 产品获取区间无效",
            "acquisition_end_utc",
        ));
    }
    let beam_mode = alos2_property_text(properties, "beam_mode")?;
    let polarizations = properties
        .get("polarizations")
        .and_then(Value::as_array)
        .and_then(|values| values.iter().map(Value::as_str).collect::<Option<Vec<_>>>());
    let identity_matches = alos2_property_text(properties, "provider_product_key")? == item_id
        && alos2_property_text(properties, "provider_product_id")? == product.product_id
        && alos2_property_text(properties, "cmr_granule_concept_id")? == product.product_id
        && alos2_property_text(properties, "cmr_native_id")? == product.display_name
        && valid_exact_text(alos2_property_text(properties, "producer_granule_id")?)
        && alos2_property_text(properties, "logical_dataset")? == ALOS2_LOGICAL_DATASET
        && alos2_property_text(properties, "collection_concept_id")? == ALOS2_NATIVE_COLLECTION_ID
        && alos2_property_text(properties, "collection_short_name")? == ALOS2_COLLECTION_SHORT_NAME
        && alos2_property_text(properties, "collection_version")? == ALOS2_COLLECTION_VERSION
        && alos2_property_text(properties, "collection_profile_id")? == ALOS2_PROFILE_ID
        && alos2_property_text(properties, "identity_rule_id")? == ALOS2_IDENTITY_RULE_ID
        && alos2_property_text(properties, "filename_rule_id")? == ALOS2_FILENAME_RULE_ID
        && alos2_property_text(properties, "resolver_policy_id")? == ALOS2_RESOLVER_POLICY_ID
        && alos2_property_text(properties, "platform")? == "ALOS-2"
        && alos2_property_text(properties, "instrument")? == "PALSAR-2"
        && alos2_property_text(properties, "processing_level")? == "1.1"
        && polarizations
            .as_deref()
            .is_some_and(|values| alos2_expected_polarizations(beam_mode) == Some(values));
    if !identity_matches
        || properties.get("preview_available").and_then(Value::as_bool) != Some(false)
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "ALOS-2 产品权威身份或范围冲突",
            "alos2_authority",
        ));
    }
    let primary_count = product
        .assets
        .iter()
        .filter(|asset| asset.role == AssetRole::Primary)
        .count();
    if primary_count == 0 {
        return Err(catalogue_provider_error(
            "ASF_PRIMARY_ASSET_MISSING",
            "ALOS-2 Granule 缺少主 ZIP",
            "primary_asset",
        ));
    }
    if primary_count > 1 {
        return Err(catalogue_provider_error(
            "ASF_PRIMARY_ASSET_AMBIGUOUS",
            "ALOS-2 Granule 包含多个主 ZIP 身份",
            "primary_asset",
        ));
    }
    if product.assets.len() != 1 {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "ALOS-2 产品资产图包含非主 ZIP 资产",
            "assets",
        ));
    }
    let asset = &product.assets[0];
    let (Some(asset_key), Some(stable_key), Some(descriptor), Some(file_name), Some(size_bytes)) = (
        asset.asset_key.as_deref(),
        asset.stable_asset_key.as_deref(),
        asset.access_descriptor.as_ref(),
        asset.file_name.as_deref(),
        asset.size_bytes.and_then(|value| u64::try_from(value).ok()),
    ) else {
        return Err(catalogue_provider_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "ALOS-2 ZIP 资产证据无效",
            "asset_fields",
        ));
    };
    let expected_stable_key = stable_asset_key(&item_id, asset_key).map_err(|_| {
        catalogue_provider_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "ALOS-2 ZIP 资产身份无效",
            "stable_asset_key",
        )
    })?;
    let stable_href = descriptor.stable_href.as_deref().unwrap_or_default();
    if asset_key != file_name
        || !valid_alos2_zip_basename(file_name)
        || stable_key != expected_stable_key
        || asset.role != AssetRole::Primary
        || asset.native_roles != ["archive"]
        || asset.url.is_some()
        || asset.media_type.as_deref() != Some("application/zip")
        || size_bytes == 0
        || descriptor.resolver_id != "earthdata_https"
        || descriptor.scheme != AssetAccessScheme::EarthdataHttps
        || !descriptor.requires_credentials
        || descriptor.credential_profile.as_deref() != Some("earthdata-default")
        || descriptor.validate().is_err()
        || !alos2_route_matches_archive("https", stable_href, file_name)
    {
        return Err(catalogue_provider_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "ALOS-2 ZIP 资产证据无效",
            "primary_asset",
        ));
    }

    let integrity = properties
        .get("asset_integrity")
        .and_then(Value::as_object)
        .filter(|values| values.len() == 1)
        .and_then(|values| values.get(asset_key))
        .and_then(Value::as_object)
        .filter(|value| value.len() == 4)
        .ok_or_else(|| {
            catalogue_provider_error(
                "ASF_ASSET_CLASSIFICATION_INVALID",
                "ALOS-2 ZIP 完整性证据无效",
                "asset_integrity",
            )
        })?;
    let checksum_algorithm = integrity
        .get("algorithm")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let checksum_value = integrity
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let checksum_length = match checksum_algorithm {
        "MD5" => 32,
        "SHA256" => 64,
        _ => 0,
    };
    if integrity.get("file_name").and_then(Value::as_str) != Some(file_name)
        || integrity.get("size_bytes").and_then(Value::as_u64) != Some(size_bytes)
        || checksum_value.len() != checksum_length
        || !checksum_value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(catalogue_provider_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "ALOS-2 ZIP 完整性证据冲突",
            "asset_integrity",
        ));
    }

    let routes = properties
        .get("asset_routes")
        .and_then(Value::as_object)
        .filter(|values| values.len() == 1)
        .and_then(|values| values.get(asset_key))
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty() && values.len() <= 8)
        .ok_or_else(|| {
            catalogue_provider_error(
                "ASF_ASSET_CLASSIFICATION_INVALID",
                "ALOS-2 ZIP 路由证据无效",
                "asset_routes",
            )
        })?;
    let mut unique_routes = BTreeMap::new();
    let mut https_routes = 0usize;
    for route in routes {
        let Some(route) = route.as_object().filter(|value| value.len() == 2) else {
            return Err(catalogue_provider_error(
                "ASF_ASSET_CLASSIFICATION_INVALID",
                "ALOS-2 ZIP 路由证据无效",
                "asset_routes",
            ));
        };
        let (Some(scheme), Some(locator)) = (
            route.get("scheme").and_then(Value::as_str),
            route.get("locator").and_then(Value::as_str),
        ) else {
            return Err(catalogue_provider_error(
                "ASF_ASSET_CLASSIFICATION_INVALID",
                "ALOS-2 ZIP 路由证据无效",
                "asset_routes",
            ));
        };
        if !alos2_route_matches_archive(scheme, locator, file_name)
            || unique_routes
                .insert((scheme.to_owned(), locator.to_owned()), ())
                .is_some()
        {
            return Err(catalogue_provider_error(
                "ASF_ASSET_CLASSIFICATION_INVALID",
                "ALOS-2 ZIP 路由证据无效",
                "asset_routes",
            ));
        }
        if scheme == "https" {
            https_routes += 1;
            if locator != stable_href {
                return Err(catalogue_provider_error(
                    "ASF_ASSET_CLASSIFICATION_INVALID",
                    "ALOS-2 ZIP HTTPS 路由与执行描述符冲突",
                    "https_route",
                ));
            }
        }
    }
    if https_routes != 1 {
        return Err(catalogue_provider_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "ALOS-2 ZIP 缺少唯一 HTTPS 执行路由",
            "https_route",
        ));
    }

    let frozen_assets = serde_json::to_value(&product.assets).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "ALOS-2 资产图无法冻结",
            "assets",
        )
    })?;
    let metadata = Map::from_iter([
        (
            "dataset".to_owned(),
            Value::String(ALOS2_LOGICAL_DATASET.to_owned()),
        ),
        (
            "logical_dataset".to_owned(),
            Value::String(ALOS2_LOGICAL_DATASET.to_owned()),
        ),
        (
            "native_collection_id".to_owned(),
            Value::String(ALOS2_NATIVE_COLLECTION_ID.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(ALOS2_PROFILE_ID.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(ALOS2_IDENTITY_RULE_ID.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(ALOS2_FILENAME_RULE_ID.to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            serde_json::to_value(profile.delivery_model.clone())
                .expect("closed delivery model serializes"),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(ALOS2_RESOLVER_POLICY_ID.to_owned()),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product.product_id.clone()),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(item_id.clone()),
        ),
        (
            "product_family".to_owned(),
            Value::String(ALOS2_COLLECTION_SHORT_NAME.to_owned()),
        ),
        (
            "processing_level".to_owned(),
            Value::String("1.1".to_owned()),
        ),
        ("source_kind".to_owned(), Value::String("sar".to_owned())),
        (
            "product_kind".to_owned(),
            Value::String("sar_product".to_owned()),
        ),
        (
            "properties".to_owned(),
            Value::Object(product.properties.clone()),
        ),
        ("assets".to_owned(), frozen_assets),
    ]);
    validate_debug_context(&metadata).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "ALOS-2 快照元数据包含敏感字段",
            "sensitive_metadata",
        )
    })?;
    let access = FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: ALOS2_PROVIDER_ID.to_owned(),
            collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
            provider_product_id: product.product_id.clone(),
            provider_product_key: item_id.clone(),
            canonical_product_key: None,
            asset_key: asset_key.to_owned(),
            stable_asset_key: stable_key.to_owned(),
        },
        descriptor: descriptor.clone(),
        role: AssetRole::Primary,
        file_name: Some(file_name.to_owned()),
        expected_size_bytes: Some(size_bytes),
        checksum_algorithm: Some(checksum_algorithm.to_owned()),
        checksum_value: Some(checksum_value.to_owned()),
    };
    let item = SnapshotItem {
        item_id,
        provider_id: product.provider_id,
        display_name: product.display_name,
        acquisition_datetime: Some(
            acquisition_datetime.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        ),
        footprint_geojson: product.footprint_geojson,
        metadata,
        payload: Map::new(),
        asset_access: Some(access),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: ALOS2_RESOLVER_POLICY_ID.to_owned(),
            collection_profile_id: Some(ALOS2_PROFILE_ID.to_owned()),
        }),
    };
    item.validate().map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "ALOS-2 快照项目无效",
            "snapshot_item",
        )
    })?;
    if serde_json::to_vec(&item)
        .map(|bytes| bytes.len() > MAX_CATALOGUE_CANDIDATE_JSON_BYTES)
        .unwrap_or(true)
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "ALOS-2 快照项目超出离线限制",
            "candidate_size",
        ));
    }
    Ok(item)
}

fn map_mpc_sentinel2_search_product(
    product: ProviderProduct,
    profile: &insar_provider::catalog::CollectionProfile,
) -> Result<SnapshotItem, ApplicationServiceError> {
    if product.source_kind != DataSourceKind::Sentinel2
        || product.product_kind != ProductKind::OpticalScene
        || product.provider_id != profile.provider_id
        || product.footprint_bbox.is_some()
        || !product.assets.is_empty()
        || !product.payload.is_empty()
        || product.properties.len() != 2
        || product
            .properties
            .get("product_type")
            .and_then(Value::as_str)
            != Some("S2MSI2A")
        || !product
            .properties
            .get("cloud_cover")
            .and_then(Value::as_f64)
            .is_some_and(|value| value.is_finite() && (0.0..=100.0).contains(&value))
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Microsoft Sentinel-2 检索结果包含未批准的字段",
            "search_only_product",
        ));
    }
    let acquisition_datetime = product
        .acquisition_datetime
        .as_deref()
        .and_then(parse_normalized_utc)
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Microsoft Sentinel-2 获取时间无效",
                "acquisition_datetime",
            )
        })?;
    insar_provider::catalog::validate_mpc_sentinel2_source_id(
        &product.product_id,
        &product.display_name,
        product.acquisition_datetime.as_deref(),
    )
    .map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Microsoft Sentinel-2 原生身份不一致",
            "source_identity",
        )
    })?;
    validate_debug_context(&product.properties).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Microsoft Sentinel-2 属性包含敏感字段",
            "sensitive_properties",
        )
    })?;
    let item_id = provider_product_key(
        &product.provider_id,
        profile.native_collection_id,
        &product.product_id,
    )
    .map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Microsoft Sentinel-2 产品身份无效",
            "provider_product_key",
        )
    })?;
    let metadata = Map::from_iter([
        (
            "logical_dataset".to_owned(),
            Value::String(profile.logical_dataset.to_owned()),
        ),
        (
            "native_collection_id".to_owned(),
            Value::String(profile.native_collection_id.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(profile.profile_id.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(profile.identity_rule_id.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(profile.filename_rule_id.to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            serde_json::to_value(profile.delivery_model.clone())
                .expect("closed delivery model serializes"),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(profile.resolver_policy_id.to_owned()),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product.product_id.clone()),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(item_id.clone()),
        ),
        (
            "source_kind".to_owned(),
            Value::String("sentinel2".to_owned()),
        ),
        (
            "product_kind".to_owned(),
            Value::String("optical_scene".to_owned()),
        ),
        ("properties".to_owned(), Value::Object(product.properties)),
        ("assets".to_owned(), Value::Array(Vec::new())),
    ]);
    validate_debug_context(&metadata).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Microsoft Sentinel-2 快照元数据包含敏感字段",
            "sensitive_metadata",
        )
    })?;
    let item = SnapshotItem {
        item_id,
        provider_id: product.provider_id,
        display_name: product.display_name,
        acquisition_datetime: Some(
            acquisition_datetime.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        ),
        footprint_geojson: product.footprint_geojson,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    item.validate().map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Microsoft Sentinel-2 快照项目无效",
            "snapshot_item",
        )
    })?;
    if serde_json::to_vec(&item)
        .map(|bytes| bytes.len() > MAX_CATALOGUE_CANDIDATE_JSON_BYTES)
        .unwrap_or(true)
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Microsoft Sentinel-2 快照项目超出离线限制",
            "candidate_size",
        ));
    }
    Ok(item)
}

fn map_earth_search_sentinel2_search_product(
    product: ProviderProduct,
    profile: &insar_provider::catalog::CollectionProfile,
) -> Result<SnapshotItem, ApplicationServiceError> {
    if product.source_kind != DataSourceKind::Sentinel2
        || product.product_kind != ProductKind::OpticalScene
        || product.provider_id != profile.provider_id
        || product.footprint_bbox.is_some()
        || product.assets.len() > 1
        || !product.payload.is_empty()
        || product.properties.len() != 2
        || product
            .properties
            .get("product_type")
            .and_then(Value::as_str)
            != Some("S2MSI2A")
        || !product
            .properties
            .get("cloud_cover")
            .and_then(Value::as_f64)
            .is_some_and(|value| value.is_finite() && (0.0..=100.0).contains(&value))
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Element 84 Earth Search Sentinel-2 检索结果包含未批准的字段",
            "search_only_product",
        ));
    }
    let acquisition_datetime = product
        .acquisition_datetime
        .as_deref()
        .and_then(parse_normalized_utc)
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Element 84 Earth Search Sentinel-2 获取时间无效",
                "acquisition_datetime",
            )
        })?;
    insar_provider::catalog::validate_earth_search_sentinel2_source_id(
        &product.product_id,
        &product.display_name,
        product.acquisition_datetime.as_deref(),
    )
    .map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Element 84 Earth Search Sentinel-2 原生身份不一致",
            "source_identity",
        )
    })?;
    validate_debug_context(&product.properties).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Element 84 Earth Search Sentinel-2 属性包含敏感字段",
            "sensitive_properties",
        )
    })?;
    let item_id = provider_product_key(
        &product.provider_id,
        profile.native_collection_id,
        &product.product_id,
    )
    .map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Element 84 Earth Search Sentinel-2 产品身份无效",
            "provider_product_key",
        )
    })?;
    for asset in &product.assets {
        let valid_thumb = asset.role == AssetRole::Thumbnail
            && asset.access_descriptor.is_none()
            && asset
                .url
                .as_deref()
                .and_then(|href| url::Url::parse(href).ok())
                .is_some_and(|url| {
                    insar_provider::catalog::is_valid_earth_search_sentinel2_thumbnail_url(
                        &product.product_id,
                        &url,
                    )
                });
        if !valid_thumb {
            return Err(catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Element 84 Earth Search Sentinel-2 快视图资产无效",
                "thumbnail_asset",
            ));
        }
    }
    let frozen_assets = serde_json::to_value(&product.assets)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Element 84 Earth Search Sentinel-2 资产序列化失败",
                "assets_serialize",
            )
        })?;
    let metadata = Map::from_iter([
        (
            "logical_dataset".to_owned(),
            Value::String(profile.logical_dataset.to_owned()),
        ),
        (
            "native_collection_id".to_owned(),
            Value::String(profile.native_collection_id.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(profile.profile_id.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(profile.identity_rule_id.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(profile.filename_rule_id.to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            serde_json::to_value(profile.delivery_model.clone())
                .expect("closed delivery model serializes"),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(profile.resolver_policy_id.to_owned()),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product.product_id.clone()),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(item_id.clone()),
        ),
        (
            "source_kind".to_owned(),
            Value::String("sentinel2".to_owned()),
        ),
        (
            "product_kind".to_owned(),
            Value::String("optical_scene".to_owned()),
        ),
        ("properties".to_owned(), Value::Object(product.properties)),
        ("assets".to_owned(), Value::Array(frozen_assets)),
    ]);
    validate_debug_context(&metadata).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Element 84 Earth Search Sentinel-2 快照元数据包含敏感字段",
            "sensitive_metadata",
        )
    })?;
    let item = SnapshotItem {
        item_id,
        provider_id: product.provider_id,
        display_name: product.display_name,
        acquisition_datetime: Some(
            acquisition_datetime.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        ),
        footprint_geojson: product.footprint_geojson,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    item.validate().map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Element 84 Earth Search Sentinel-2 快照项目无效",
            "snapshot_item",
        )
    })?;
    if serde_json::to_vec(&item)
        .map(|bytes| bytes.len() > MAX_CATALOGUE_CANDIDATE_JSON_BYTES)
        .unwrap_or(true)
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "Element 84 Earth Search Sentinel-2 快照项目超出离线限制",
            "candidate_size",
        ));
    }
    Ok(item)
}

fn map_sentinel2_catalogue_product(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
    route_use: RouteUse,
) -> Result<SnapshotItem, ApplicationServiceError> {
    let profile = CollectionProfileRegistry
        .for_source(
            &query.logical_dataset,
            &query.provider_id,
            &query.native_collection_id,
        )
        .filter(|profile| {
            matches!(
                profile.logical_dataset,
                "optical.sentinel-2.l2a" | "optical.sentinel-2.l1c" | "optical.sentinel-3.standard"
            )
        })
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "目录产品来源绑定无效",
                "collection_profile",
            )
        })?;
    if profile.provider_id == insar_provider::catalog::MPC_SENTINEL2_L2A_PROVIDER_ID {
        if query.validate().is_err() {
            return Err(catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Microsoft Sentinel-2 检索请求无效",
                "query",
            ));
        }
        return map_mpc_sentinel2_search_product(product, profile);
    }
    if profile.provider_id == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID {
        if query.validate().is_err() {
            return Err(catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "Element 84 Earth Search Sentinel-2 检索请求无效",
                "query",
            ));
        }
        return map_earth_search_sentinel2_search_product(product, profile);
    }
    if query.validate().is_err()
        || product.source_kind != DataSourceKind::Sentinel2
        || product.product_kind != ProductKind::OpticalScene
        || product.provider_id != profile.provider_id
        || !valid_exact_text(&product.product_id)
        || !canonical_odata_uuid(&product.product_id)
        || !valid_catalogue_display_name(&product.display_name)
        || product.footprint_bbox.is_some()
        || !product.payload.is_empty()
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录产品包含不受支持的字段",
            "product",
        ));
    }
    let acquisition_datetime = product
        .acquisition_datetime
        .as_deref()
        .and_then(parse_normalized_utc)
        .ok_or_else(|| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "目录产品获取时间无效",
                "acquisition_datetime",
            )
        })?;
    let frozen_properties =
        freeze_catalogue_properties(&product.properties, &acquisition_datetime)?;
    if product.assets.len() != 1 {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录产品必须只包含唯一归档主资产",
            "assets",
        ));
    }
    let item_id = provider_product_key(
        &product.provider_id,
        profile.native_collection_id,
        &product.product_id,
    )
    .map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录产品身份无效",
            "provider_product_key",
        )
    })?;
    let mut primary_assets = product
        .assets
        .iter()
        .filter(|asset| asset.role == AssetRole::Primary)
        .collect::<Vec<_>>();
    if primary_assets.len() != 1 {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录产品必须包含唯一的归档主资产",
            "archive_asset",
        ));
    }
    let asset = primary_assets
        .pop()
        .expect("one primary asset is checked above");
    let expected_stable_asset_key = stable_asset_key(&item_id, "archive").map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录资产身份无效",
            "stable_asset_key",
        )
    })?;
    if asset.asset_key.as_deref() != Some("archive")
        || asset.stable_asset_key.as_deref() != Some(expected_stable_asset_key.as_str())
        || asset.role != AssetRole::Primary
        || asset.native_roles != ["archive"]
        || asset.media_type.is_some()
        || asset.access_descriptor.is_some()
        || asset.url.is_some()
        || asset.file_name.is_some()
        || asset.size_bytes.is_some_and(|value| value < 0)
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录产品必须包含唯一的归档主资产",
            "archive_asset",
        ));
    }
    let mut frozen_asset = Map::from_iter([
        ("asset_key".to_owned(), Value::String("archive".to_owned())),
        (
            "stable_asset_key".to_owned(),
            Value::String(expected_stable_asset_key),
        ),
        (
            "role".to_owned(),
            serde_json::to_value(AssetRole::Primary)
                .expect("closed normalized asset role serializes"),
        ),
        (
            "native_roles".to_owned(),
            Value::Array(vec![Value::String("archive".to_owned())]),
        ),
    ]);
    if let Some(size_bytes) = asset.size_bytes {
        frozen_asset.insert("size_bytes".to_owned(), Value::Number(size_bytes.into()));
    }
    let frozen_assets = vec![Value::Object(frozen_asset)];
    let mut metadata = Map::from_iter([
        (
            "logical_dataset".to_owned(),
            Value::String(profile.logical_dataset.to_owned()),
        ),
        (
            "native_collection_id".to_owned(),
            Value::String(profile.native_collection_id.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(profile.profile_id.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(profile.identity_rule_id.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(profile.filename_rule_id.to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            serde_json::to_value(profile.delivery_model.clone())
                .expect("closed delivery model serializes"),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(profile.resolver_policy_id.to_owned()),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product.product_id),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(item_id.clone()),
        ),
        (
            "source_kind".to_owned(),
            Value::String("sentinel2".to_owned()),
        ),
        (
            "product_kind".to_owned(),
            Value::String("optical_scene".to_owned()),
        ),
        ("properties".to_owned(), Value::Object(frozen_properties)),
        ("assets".to_owned(), Value::Array(frozen_assets)),
    ]);
    validate_debug_context(&metadata).map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录快照元数据包含敏感字段",
            "sensitive_metadata",
        )
    })?;
    let mut item = SnapshotItem {
        item_id,
        provider_id: product.provider_id,
        display_name: product.display_name,
        acquisition_datetime: Some(
            acquisition_datetime.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        ),
        footprint_geojson: product.footprint_geojson,
        metadata: std::mem::take(&mut metadata),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    if matches!(route_use, RouteUse::Production) {
        let expected_size_bytes = asset.size_bytes.and_then(|value| u64::try_from(value).ok());
        let properties = item
            .metadata
            .get("properties")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                catalogue_provider_error(
                    "CATALOGUE_PRODUCT_INVALID",
                    "目录产品属性无效",
                    "properties",
                )
            })?;
        let (checksum_algorithm, checksum_value) = preferred_cdse_checksum(properties)?;
        if properties.get("online").and_then(Value::as_bool) != Some(true)
            || expected_size_bytes.is_none_or(|value| value == 0)
        {
            return Err(catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "目录产品缺少可下载归档证据",
                "archive_downloadability",
            ));
        }
        let filename_evidence =
            archive_filename_from_odata_name_for_rule(&item.display_name, profile.filename_rule_id)
                .map_err(|_| {
                    catalogue_provider_error(
                        "CATALOGUE_PRODUCT_INVALID",
                        "目录产品归档文件名无效",
                        "archive_filename",
                    )
                })?;
        let access = build_cdse_archive_access(
            &item,
            CdseArchiveEvidence {
                filename_evidence: Some(filename_evidence),
                expected_size_bytes,
                checksum_algorithm: Some(checksum_algorithm),
                checksum_value: Some(checksum_value),
            },
            CatalogPolicyContext::Production,
        )
        .map_err(|_| {
            catalogue_provider_error(
                "CATALOGUE_PRODUCT_INVALID",
                "目录产品归档执行策略无效",
                "archive_access",
            )
        })?;
        item.asset_access = Some(access);
        item.execution_policy = Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: profile.resolver_policy_id.to_owned(),
            collection_profile_id: Some(profile.profile_id.to_owned()),
        });
    }
    item.validate().map_err(|_| {
        catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录快照项目无效",
            "snapshot_item",
        )
    })?;
    if serde_json::to_vec(&item)
        .map(|bytes| bytes.len() > MAX_CATALOGUE_CANDIDATE_JSON_BYTES)
        .unwrap_or(true)
    {
        return Err(catalogue_provider_error(
            "CATALOGUE_PRODUCT_INVALID",
            "目录快照项目超出离线限制",
            "candidate_size",
        ));
    }
    Ok(item)
}

/// The typed provider warning a SAR route attaches when it cannot prove the
/// remote side was exhausted (see `insar_provider::asf::burst`).
const ASF_SEARCH_POSSIBLY_INCOMPLETE: &str = "ASF_SEARCH_POSSIBLY_INCOMPLETE";

fn normalize_legacy_asf_search_stats(
    stats: &AsfSearchStats,
    frozen_candidate_count: usize,
) -> FrozenSearchStats {
    let matched_count = stats.matched_count.or(stats.total_count);
    FrozenSearchStats {
        matched_count,
        fetched_record_count: stats.fetched_record_count,
        mapped_record_count: stats.mapped_record_count,
        provider_returned_count: stats.returned_count,
        frozen_candidate_count: frozen_candidate_count as u64,
        // The typed provider warning (e.g. Burst's served-fewer-than-hits)
        // is frozen as the batch's possibly-incomplete flag, never dropped.
        truncated: stats
            .truncated
            .unwrap_or_else(|| matched_count.is_some_and(|count| count > stats.returned_count))
            || stats
                .warnings
                .iter()
                .any(|warning| warning.code == ASF_SEARCH_POSSIBLY_INCOMPLETE),
        source_attempts: vec![],
        source: canonical_legacy_asf_search_source(&stats.source).to_owned(),
    }
}

fn strictly_before_rfc3339(value: &str) -> Result<String, ApplicationServiceError> {
    let parsed = DateTime::parse_from_rfc3339(value).map_err(|_| {
        ApplicationServiceError::InvalidRequest {
            reason: "cached SAR continuation boundary is not RFC3339".to_owned(),
        }
    })?;
    let previous = parsed
        .checked_sub_signed(chrono::Duration::nanoseconds(1))
        .ok_or_else(|| ApplicationServiceError::InvalidRequest {
            reason: "cached SAR continuation boundary underflow".to_owned(),
        })?;
    Ok(previous.to_rfc3339_opts(SecondsFormat::Nanos, true))
}

fn sar_candidate_provider_id(
    candidates: &[SnapshotItem],
) -> Result<String, ApplicationServiceError> {
    let provider_id = candidates
        .first()
        .map(|candidate| candidate.provider_id.clone())
        .unwrap_or_else(|| "asf.catalog".to_owned());
    if candidates
        .iter()
        .any(|candidate| candidate.provider_id != provider_id)
    {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "SAR search mapped multiple provider identities".to_owned(),
        });
    }
    Ok(provider_id)
}

fn merge_incremental_sar_results(
    source_batch: FrozenSearchBatch,
    provider_candidates: Vec<SnapshotItem>,
    provider_stats: FrozenSearchStats,
    older_than: &str,
    requested_target: Option<u64>,
) -> Result<(Vec<SnapshotItem>, FrozenSearchStats, String), ApplicationServiceError> {
    let boundary = DateTime::parse_from_rfc3339(older_than).map_err(|_| {
        ApplicationServiceError::InvalidRequest {
            reason: "cached SAR continuation boundary is not RFC3339".to_owned(),
        }
    })?;
    let mut previous = boundary;
    for candidate in &provider_candidates {
        let value = candidate.acquisition_datetime.as_deref().ok_or_else(|| {
            ApplicationServiceError::InvalidRequest {
                reason: "incremental SAR result has no acquisition time".to_owned(),
            }
        })?;
        let acquired = DateTime::parse_from_rfc3339(value).map_err(|_| {
            ApplicationServiceError::InvalidRequest {
                reason: "incremental SAR acquisition time is not RFC3339".to_owned(),
            }
        })?;
        let canonical = acquired.to_rfc3339_opts(SecondsFormat::AutoSi, true);
        if canonical != value || acquired >= previous {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "incremental SAR results are not strictly older and descending".to_owned(),
            });
        }
        previous = acquired;
    }

    let provider_id = source_batch.provider_id.clone();
    if provider_candidates
        .iter()
        .any(|candidate| candidate.provider_id != provider_id)
    {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "incremental SAR provider identity changed".to_owned(),
        });
    }
    let cached_stats =
        source_batch
            .search_stats
            .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                reason: "incremental SAR cache has no authoritative statistics".to_owned(),
            })?;
    let mut seen = source_batch
        .candidates
        .iter()
        .map(|candidate| candidate.item_id.clone())
        .collect::<BTreeSet<_>>();
    let mut candidates = source_batch.candidates;
    candidates.extend(
        provider_candidates
            .into_iter()
            .filter(|candidate| seen.insert(candidate.item_id.clone())),
    );
    let merged_count = candidates.len();
    if let Some(target) = requested_target {
        candidates.truncate(usize::try_from(target).unwrap_or(usize::MAX));
    }
    let cut_by_target = candidates.len() < merged_count;
    let candidate_count = u64::try_from(candidates.len()).unwrap_or(u64::MAX);
    let provider_returned_count = cached_stats
        .provider_returned_count
        .checked_add(provider_stats.provider_returned_count)
        .ok_or_else(|| ApplicationServiceError::InvalidRequest {
            reason: "incremental SAR provider count overflow".to_owned(),
        })?;
    let add_optional = |left: Option<u64>, right: Option<u64>| -> Option<u64> {
        left.zip(right)
            .and_then(|(left, right)| left.checked_add(right))
    };
    let matched_count = cached_stats.matched_count.or(provider_stats.matched_count);
    let stats = FrozenSearchStats {
        matched_count,
        fetched_record_count: add_optional(
            cached_stats.fetched_record_count,
            provider_stats.fetched_record_count,
        ),
        mapped_record_count: add_optional(
            cached_stats.mapped_record_count,
            provider_stats.mapped_record_count,
        ),
        provider_returned_count,
        frozen_candidate_count: candidate_count,
        truncated: provider_stats.truncated
            || cut_by_target
            || matched_count.is_some_and(|matched| matched > candidate_count),
        source_attempts: vec![],
        source: cached_stats.source,
    };
    Ok((candidates, stats, provider_id))
}

fn sar_batch_is_complete(stats: Option<&FrozenSearchStats>, candidate_count: u64) -> bool {
    stats.is_some_and(|stats| {
        stats
            .matched_count
            .is_some_and(|matched| matched <= candidate_count)
            || (!stats.truncated && stats.provider_returned_count <= candidate_count)
    })
}

fn canonical_legacy_asf_search_source(source: &str) -> &'static str {
    match source {
        "ASF_BATCH" => "ASF_BATCH",
        "CMR" => "ASF_CMR",
        _ => "ASF",
    }
}

#[cfg(test)]
mod catalogue_exhaustion_projection_tests {
    use super::*;

    #[test]
    fn capped_unknown_total_is_marked_incomplete_but_exhaustion_is_not() {
        let mut stats = CatalogSearchStats {
            requested_limit: Some(10_000),
            matched_count: None,
            request_count: 100,
            transport_part_count: 1,
            fetched_record_count: 10_000,
            mapped_record_count: 10_000,
            profile_filtered_record_count: 0,
            exact_duplicate_record_count: 0,
            unique_provider_product_count: 10_000,
            exact_matched_count: 10_000,
            returned_count: 10_000,
            remote_exhausted: false,
            working_set_satisfied: true,
            elapsed_ms: 0,
            truncated: false,
        };
        assert!(normalize_catalogue_search_stats(&stats, 10_000).truncated);
        stats.remote_exhausted = true;
        assert!(!normalize_catalogue_search_stats(&stats, 10_000).truncated);
    }

    #[test]
    fn meeting_a_target_below_the_ceiling_is_not_possibly_incomplete() {
        for target in [100_u64, 250, 500] {
            let stats: CatalogSearchStats = serde_json::from_value(serde_json::json!({
                "requested_limit": target, "matched_count": 10_001, "request_count": 1,
                "transport_part_count": 1, "fetched_record_count": target,
                "mapped_record_count": target, "profile_filtered_record_count": 0,
                "exact_duplicate_record_count": 0, "unique_provider_product_count": target,
                "exact_matched_count": target, "returned_count": target,
                "remote_exhausted": false, "working_set_satisfied": true,
                "elapsed_ms": 0, "truncated": false
            }))
            .unwrap();
            assert!(!normalize_catalogue_search_stats(&stats, target as usize).truncated);
        }
        // SAR: Burst/SearchAPI report target stops as truncated; only the
        // ceiling, ALL, or a typed warning freezes as possibly incomplete.
        for target in [Some(250), Some(500)] {
            assert!(!sar_frozen_possibly_incomplete(true, false, target));
            assert!(sar_frozen_possibly_incomplete(true, true, target));
        }
        assert!(sar_frozen_possibly_incomplete(true, false, Some(10_000)));
        assert!(sar_frozen_possibly_incomplete(true, false, None));
        assert!(!sar_frozen_possibly_incomplete(false, false, Some(10_000)));
        assert!(stopped_at_safety_ceiling(None));
        assert!(stopped_at_safety_ceiling(Some(10_000)));
        assert!(!stopped_at_safety_ceiling(Some(9_999)));
    }

    #[test]
    fn sar_possibly_incomplete_warning_is_frozen_not_dropped() {
        let mut stats: AsfSearchStats = serde_json::from_value(serde_json::json!({
            "requested_limit": 250, "query_limit": 250, "total_count": null,
            "candidate_count": 3, "returned_count": 3, "source": "CMR_BURST",
            "cmr_enriched": false, "truncated": false
        }))
        .unwrap();
        assert!(!normalize_legacy_asf_search_stats(&stats, 3).truncated);
        stats.warnings.push(
            insar_core::ContractError::new(
                ASF_SEARCH_POSSIBLY_INCOMPLETE,
                "ASF 报告的匹配总数多于实际返回的记录",
                serde_json::Map::new(),
            )
            .unwrap(),
        );
        assert!(normalize_legacy_asf_search_stats(&stats, 3).truncated);
    }
}

/// #22 B slice 3(a): the possibly-incomplete flag fires only when the
/// 10,000 SEARCH_SAFETY_CAP (or ALL, which it bounds) was the target. Meeting
/// an explicit or default target below it is a normal, complete-enough result.
fn stopped_at_safety_ceiling(target: Option<u64>) -> bool {
    target.is_none_or(|target| target >= insar_core::SEARCH_SAFETY_CAP)
}

/// The frozen SAR flag: a typed provider warning, or the provider's own
/// truncation when the safety ceiling (or ALL) was the target.
fn sar_frozen_possibly_incomplete(
    provider_truncated: bool,
    provider_warned_incomplete: bool,
    target: Option<u64>,
) -> bool {
    provider_warned_incomplete || (provider_truncated && stopped_at_safety_ceiling(target))
}

fn normalize_catalogue_search_stats(
    stats: &CatalogSearchStats,
    frozen_candidate_count: usize,
) -> FrozenSearchStats {
    FrozenSearchStats {
        matched_count: stats.matched_count,
        fetched_record_count: Some(stats.fetched_record_count),
        mapped_record_count: Some(stats.mapped_record_count),
        provider_returned_count: stats.returned_count,
        frozen_candidate_count: frozen_candidate_count as u64,
        // Meeting a requested or default target below the ceiling is a normal
        // result; only the safety ceiling stopping an unexhausted search is.
        truncated: !stats.remote_exhausted && stopped_at_safety_ceiling(stats.requested_limit),
        source_attempts: vec![],
        source: "TYPED_CATALOG".to_owned(),
    }
}

fn freeze_catalogue_source_attempts(
    attempts: &[CatalogSourceAttempt],
) -> Vec<FrozenSearchSourceAttempt> {
    attempts
        .iter()
        .map(|attempt| {
            let (outcome, fallback_reason) = match attempt.outcome {
                CatalogSourceAttemptOutcome::Succeeded => {
                    (FrozenSearchSourceAttemptOutcome::Succeeded, None)
                }
                CatalogSourceAttemptOutcome::Fallback(reason) => (
                    FrozenSearchSourceAttemptOutcome::Fallback,
                    Some(match reason {
                        CatalogSourceFallbackReason::TransientNetwork => {
                            FrozenSearchSourceFallbackReason::TransientNetwork
                        }
                        CatalogSourceFallbackReason::RateLimited => {
                            FrozenSearchSourceFallbackReason::RateLimited
                        }
                        CatalogSourceFallbackReason::ServerError => {
                            FrozenSearchSourceFallbackReason::ServerError
                        }
                    }),
                ),
            };
            FrozenSearchSourceAttempt {
                provider_id: attempt.provider_id.clone(),
                native_collection_id: attempt.native_collection_id.clone(),
                outcome,
                fallback_reason,
            }
        })
        .collect()
}

fn freeze_burst_task_items(
    source_products: &[&SnapshotItem],
    policy: BurstAssetPolicy,
) -> Result<Vec<SnapshotItem>, ApplicationServiceError> {
    insar_core::freeze_burst_task_items(source_products, policy)
        .map_err(|_| ApplicationServiceError::BurstAssetEvidenceInvalid)
}

/// Expand the SLC-BURST products of a unified-search batch's exact selection
/// into the per-asset task items the verified burst worker downloads. The
/// burst catalogue mapper freezes the parent product (one item per burst, no
/// download_url); a task created from that parent shape carries no asset
/// locator and fails DL004 in the worker. This is the same freeze
/// `create_burst_asset_task` uses, so identity, host allowlist, roles and
/// sizes keep their verified checks. Non-burst selections are returned
/// unchanged (`Ok(None)`); invalid burst evidence fails closed.
pub(crate) fn expand_unified_burst_selection(
    selected_products: &[&SnapshotItem],
) -> Result<Option<Vec<SnapshotItem>>, ApplicationServiceError> {
    let contains_burst = selected_products.iter().any(|item| {
        item.provider_id == "asf.catalog"
            && item
                .metadata
                .get("dataset")
                .and_then(Value::as_str)
                .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SLC-BURST"))
    });
    if !contains_burst {
        return Ok(None);
    }
    freeze_burst_task_items(selected_products, BurstAssetPolicy::All).map(Some)
}

fn frozen_burst_task_policy(
    task_items: &[SnapshotItem],
) -> Result<BurstAssetPolicy, ApplicationServiceError> {
    insar_core::frozen_burst_task_policy(task_items)
        .map_err(|_| ApplicationServiceError::BurstAssetEvidenceInvalid)
}

fn map_sar_product(product: SarProduct) -> Result<SnapshotItem, ApplicationServiceError> {
    if product.provider != "asf.catalog" {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "M1 accepts only mapped ASF catalogue products".to_owned(),
        });
    }
    let is_seasat = product.dataset.eq_ignore_ascii_case("SEASAT");
    let is_burst = product.dataset.eq_ignore_ascii_case("SLC-BURST");
    let frozen_assets = if is_seasat || is_burst {
        Some(serde_json::to_value(&product.assets).map_err(|_| {
            ApplicationServiceError::InvalidRequest {
                reason: "Provider product assets could not be frozen".to_owned(),
            }
        })?)
    } else {
        let preview_assets = product
            .assets
            .iter()
            .filter(|asset| {
                matches!(
                    asset.role,
                    AssetRole::Preview | AssetRole::Browse | AssetRole::Thumbnail
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        (!preview_assets.is_empty())
            .then(|| serde_json::to_value(preview_assets))
            .transpose()
            .map_err(|_| ApplicationServiceError::InvalidRequest {
                reason: "Provider Preview assets could not be frozen".to_owned(),
            })?
    };
    let frozen_burst_metadata = if is_burst {
        let burst =
            product
                .burst
                .as_ref()
                .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                    reason: "Provider product metadata could not be frozen".to_owned(),
                })?;
        Some((
            serde_json::to_value(burst).map_err(|_| ApplicationServiceError::InvalidRequest {
                reason: "Provider product metadata could not be frozen".to_owned(),
            })?,
            Value::Object(product.native_metadata.clone()),
        ))
    } else {
        None
    };
    let mut metadata = Map::new();
    metadata.insert(
        "product_key".to_owned(),
        Value::String(product.product_key.clone()),
    );
    metadata.insert(
        "provider".to_owned(),
        Value::String(product.provider.clone()),
    );
    metadata.insert("dataset".to_owned(), Value::String(product.dataset));
    metadata.insert(
        "product_id".to_owned(),
        Value::String(product.product_id.clone()),
    );
    metadata.insert(
        "product_family".to_owned(),
        Value::String(product.product_family),
    );
    insert_optional_string(
        &mut metadata,
        "provider_product_key",
        product.provider_product_key,
    );
    insert_optional_string(
        &mut metadata,
        "provider_product_id",
        product.provider_product_id,
    );
    insert_optional_string(
        &mut metadata,
        "canonical_product_key",
        product.canonical_product_key,
    );
    insert_optional_string(&mut metadata, "identity_rule_id", product.identity_rule_id);
    insert_optional_string(&mut metadata, "scene_id", product.scene_id);
    insert_optional_string(&mut metadata, "platform", product.platform);
    insert_optional_string(&mut metadata, "instrument", product.instrument);
    insert_optional_string(&mut metadata, "processing_level", product.processing_level);
    insert_optional_string(&mut metadata, "beam_mode", product.beam_mode);
    insert_optional_string(&mut metadata, "beam_swath", product.beam_swath);
    insert_optional_string(&mut metadata, "polarization", product.polarization);
    insert_optional_string(&mut metadata, "flight_direction", product.flight_direction);
    insert_optional_string(&mut metadata, "look_direction", product.look_direction);
    insert_optional_string(&mut metadata, "asf_frame", product.asf_frame);
    insert_optional_number(&mut metadata, "relative_orbit", product.relative_orbit);
    insert_optional_number(&mut metadata, "absolute_orbit", product.absolute_orbit);
    insert_optional_number(&mut metadata, "frame", product.frame);
    insert_optional_number(&mut metadata, "path", product.path);
    insert_optional_string(&mut metadata, "acquisition_stop", product.acquisition_stop);

    let mut payload = Map::new();
    if let Some(url) = product.download_url {
        if !is_stable_download_url(&url) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Provider download URL is not a stable frozen locator".to_owned(),
            });
        }
        payload.insert("download_url".to_owned(), Value::String(url));
    }
    if let Some(file_name) = &product.file_name {
        payload.insert("file_name".to_owned(), Value::String(file_name.clone()));
        metadata.insert("file_name".to_owned(), Value::String(file_name.clone()));
    }
    if let Some(file_size_remote) = product.file_size_remote {
        payload.insert(
            "file_size_remote".to_owned(),
            Value::Number(file_size_remote.into()),
        );
    }
    if let Some(checksum) = product.checksum {
        metadata.insert("checksum".to_owned(), Value::String(checksum));
    }
    let asset_keys = product
        .assets
        .iter()
        .filter_map(|asset| asset.stable_asset_key.clone().map(Value::String))
        .collect::<Vec<_>>();
    if !asset_keys.is_empty() {
        metadata.insert("stable_asset_keys".to_owned(), Value::Array(asset_keys));
    }
    if let Some(assets) = frozen_assets {
        metadata.insert("assets".to_owned(), assets);
    }
    if let Some((burst, native_metadata)) = frozen_burst_metadata {
        metadata.insert("burst".to_owned(), burst);
        metadata.insert("native_metadata".to_owned(), native_metadata);
    }
    let display_name = product.file_name.unwrap_or(product.product_id);
    Ok(SnapshotItem {
        item_id: product.product_key,
        provider_id: product.provider,
        display_name,
        acquisition_datetime: product.acquisition_start,
        footprint_geojson: product.footprint_geojson,
        metadata,
        payload,
        asset_access: None,
        execution_policy: None,
    })
}

fn insert_optional_string(target: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        target.insert(key.to_owned(), Value::String(value));
    }
}

fn insert_optional_number(target: &mut Map<String, Value>, key: &str, value: Option<i64>) {
    if let Some(value) = value {
        target.insert(key.to_owned(), Value::Number(value.into()));
    }
}

fn is_stable_download_url(value: &str) -> bool {
    (value.starts_with("https://") || value.starts_with("http://"))
        && !value.contains('?')
        && !value.contains('#')
}

fn safe_preview_file_stem(value: &str) -> String {
    let mut base = value.trim().to_owned();
    for extension in [
        ".zip", ".tar", ".tif", ".tiff", ".jpg", ".jpeg", ".png", ".webp",
    ] {
        if base.to_ascii_lowercase().ends_with(extension) {
            base.truncate(base.len() - extension.len());
            break;
        }
    }
    let mut safe = String::with_capacity(base.len().min(180));
    let mut previous_was_separator = false;
    for character in base.chars() {
        let allowed = character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.');
        let character = if allowed { character } else { '_' };
        if character == '_' {
            if previous_was_separator {
                continue;
            }
            previous_was_separator = true;
        } else {
            previous_was_separator = false;
        }
        safe.push(character);
        if safe.len() == 180 {
            break;
        }
    }
    let safe = safe.trim_matches([' ', '.', '_']).to_owned();
    if safe.is_empty() {
        "product".to_owned()
    } else {
        safe
    }
}

fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= 128 && !value.chars().any(char::is_control)
}

fn valid_catalogue_opaque_id(value: &str) -> bool {
    valid_opaque_id(value) && value == value.trim()
}

fn catalogue_diagnostic(request_id: &str, stage: &str) {
    if std::env::var_os("INSAR_DIAGNOSTIC_CATALOGUE").is_some() {
        eprintln!("INSAR_CATALOGUE_DIAGNOSTIC request_id={request_id} stage={stage}");
    }
}

/// Same opt-in diagnostic channel as the catalogue freeze path: names the
/// exact sub-check that rejected a reviewed Landsat asset task request so a
/// live failure can be attributed without weakening the checks themselves.
pub(crate) fn landsat_asset_diagnostic(request_id: &str, stage: &str) {
    if std::env::var_os("INSAR_DIAGNOSTIC_CATALOGUE").is_some() {
        eprintln!("INSAR_LANDSAT_ASSET_DIAGNOSTIC request_id={request_id} stage={stage}");
    }
}

fn invalid_request(reason: impl Into<String>) -> ApplicationServiceError {
    ApplicationServiceError::InvalidRequest {
        reason: reason.into(),
    }
}

fn bounded_directory_file_names(
    root: &Path,
    max_files: usize,
) -> Result<Vec<String>, ApplicationServiceError> {
    if !root.is_dir() {
        return Err(invalid_request("directory is not readable"));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut names = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory)
            .map_err(|_| invalid_request("directory entry list is not readable"))?;
        for entry in entries {
            let entry = entry.map_err(|_| invalid_request("directory entry is not readable"))?;
            let file_type = entry
                .file_type()
                .map_err(|_| invalid_request("directory entry type is not readable"))?;
            if file_type.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if names.len() >= max_files {
                return Err(invalid_request(format!(
                    "directory contains more than {max_files} files"
                )));
            }
            names.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    names.sort_unstable();
    Ok(names)
}

fn scene_row_from_parsed(parsed: &insar_core::ParsedSentinel1Name, source: &str) -> Value {
    let acquisition_datetime = if parsed.acquisition_start.len() >= 15 {
        format!(
            "{}-{}-{}T{}:{}:{}Z",
            &parsed.acquisition_start[0..4],
            &parsed.acquisition_start[4..6],
            &parsed.acquisition_start[6..8],
            &parsed.acquisition_start[9..11],
            &parsed.acquisition_start[11..13],
            &parsed.acquisition_start[13..15],
        )
    } else {
        parsed.acquisition_start.clone()
    };
    let file_name = source
        .split('?')
        .next()
        .unwrap_or(source)
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(source)
        .to_owned();
    let mut row = serde_json::json!({
        "scene_id": parsed.scene_id,
        "platform": parsed.platform,
        "product_type": parsed.product_type,
        "beam_mode": parsed.beam_mode,
        "polarization": parsed.polarization_channels.join(""),
        "acquisition_datetime": acquisition_datetime,
        "absolute_orbit": parsed.absolute_orbit,
        "relative_orbit": insar_core::sentinel1_relative_orbit(
            &parsed.platform,
            Some(parsed.absolute_orbit),
        ),
        "path": null,
        "frame": null,
        "orbit_direction": "UNKNOWN",
        "file_size_remote": null,
        "footprint_bbox": null,
        "footprint_geojson": null,
        "has_url": parsed.source_url.is_some(),
        "provider": "ASF",
        "dataset": "SENTINEL-1",
        "file_name": file_name,
        "native_metadata": {
            "resolution": parsed.resolution,
            "processing_level": parsed.processing_level,
            "processing_class": parsed.processing_class,
            "acquisition_stop": parsed.acquisition_stop,
            "datatake_id": parsed.datatake_id,
            "unique_id": parsed.unique_id,
        }
    });
    if let Some(url) = parsed.source_url.as_ref() {
        if let Some(object) = row.as_object_mut() {
            object.insert("download_url".to_owned(), Value::String(url.clone()));
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::{
        frozen_cdse_quicklook_product, frozen_preview_plan, load_frozen_preview_source,
        map_bhoonidhi_catalogue_product, map_nisar_rslc_catalogue_product, map_sar_product,
        map_sentinel2_catalogue_product, require_store_readiness_capability,
        resolve_preview_candidate, resolved_cdse_static_quicklook, resolved_landsat_scene_browse,
        safe_preview_file_stem, ApplicationService, ApplicationServiceConfig,
        ApplicationServiceError, CdseQuicklookResponse, DemOperationPort, DemTaskOperation,
        DemTaskPlanRequest, FrozenPreviewPlan, LandsatBrowseResponse, NativeDemDownloadWorker,
        PreviewProductClass, PreviewSaveRequest, ServiceStatus, BHOONIDHI_LOGICAL_DATASET,
        BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROVIDER_ID, BHOONIDHI_REQUIRED_PRODUCT_TYPE,
        BHOONIDHI_RESOLVER_POLICY_ID, LANDSAT_C2_L2_LOGICAL_DATASET, LANDSAT_C2_L2_PROVIDER_ID,
        NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID, NISAR_PROFILE_ID, NISAR_PROVIDER_ID,
        STORE_STARTUP_READINESS_CAPABILITY,
    };
    use insar_core::{
        provider_product_key, stable_asset_key, AsfRawFeature, AssetRole, CatalogSearchQuery,
        ContractError, FrozenSearchBatch, PreviewAssetRole, ProviderProduct, SearchWorkbenchState,
        SnapshotItem, StoreSearchBatchRequest,
    };
    use insar_provider::asf::{
        burst::map_burst_umm_granule, map_nisar_rslc_umm_granule, metadata_mapper::map_sar_feature,
        CmrGranuleEnvelope,
    };
    use insar_provider::bhoonidhi::map_bhoonidhi_feature;
    use insar_provider::catalog::CollectionProfileRegistry;
    use insar_store::TaskStore;
    use serde_json::{json, Map, Value};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    struct TelemetryOnlyWorker;

    impl insar_download::DownloadWorker for TelemetryOnlyWorker {
        fn task_transfer_sample(
            &self,
            task_id: &str,
        ) -> Option<insar_download::TaskTransferSample> {
            (task_id == "task-telemetry").then_some(insar_download::TaskTransferSample {
                network_bytes_received: 8_192,
                active_millis: 12_345,
                active_count: 1,
                bytes_per_second: 4_096.0,
            })
        }

        fn execute(
            &self,
            _work: insar_download::DownloadWork,
            _control: insar_download::WorkerControl,
            _progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
        ) -> insar_core::DownloadItemResult {
            panic!("telemetry delegation test must not execute a download")
        }
    }

    struct UnusedDemPort;

    impl DemOperationPort for UnusedDemPort {
        fn call(&self, _operation: &str, _payload: Value) -> Result<Value, String> {
            panic!("telemetry delegation test must not execute DEM")
        }
    }

    #[test]
    fn native_dem_wrapper_preserves_base_download_transfer_telemetry() {
        let worker = NativeDemDownloadWorker {
            base: Arc::new(TelemetryOnlyWorker),
            port: Arc::new(UnusedDemPort),
            store: None,
            completed_dem_windows: Mutex::new(BTreeMap::new()),
            transfer_activity: insar_download::TaskTransferActivityRegistry::default(),
            cdse_session: None,
        };
        assert_eq!(
            insar_download::DownloadWorker::task_transfer_sample(&worker, "task-telemetry"),
            Some(insar_download::TaskTransferSample {
                network_bytes_received: 8_192,
                active_millis: 12_345,
                active_count: 1,
                bytes_per_second: 4_096.0,
            })
        );
        let start = std::time::Instant::now() - std::time::Duration::from_secs(2);
        assert_eq!(
            worker
                .transfer_activity
                .sample_if_known("native-dem", start)
                .unwrap(),
            None
        );
        worker
            .transfer_activity
            .body_started("native-dem", "tile", 1, start)
            .unwrap();
        let observed = start + std::time::Duration::from_secs(1);
        worker
            .transfer_activity
            .observe_bytes("native-dem", "tile", 1, 4096, observed)
            .unwrap();
        let active =
            insar_download::DownloadWorker::task_transfer_sample(&worker, "native-dem").unwrap();
        assert_eq!(active.bytes_per_second, 4096.0);
        assert_eq!(active.active_count, 1);
        assert!(active.active_millis >= 1000);
        worker
            .transfer_activity
            .body_stopped("native-dem", "tile", 1, std::time::Instant::now())
            .unwrap();
        let stopped =
            insar_download::DownloadWorker::task_transfer_sample(&worker, "native-dem").unwrap();
        assert_eq!(stopped.bytes_per_second, 0.0);
        assert_eq!(stopped.active_count, 0);
        assert!(stopped.active_millis >= active.active_millis);
    }

    #[test]
    fn liveness_probe_does_not_require_store_capability() {
        assert!(require_store_readiness_capability(&["legacy_liveness"], false).is_ok());
    }

    #[test]
    fn preview_file_stem_is_bounded_and_path_safe() {
        assert_eq!(safe_preview_file_stem("S1A/IW:GRD?P1.zip"), "S1A_IW_GRD_P1");
        assert_eq!(safe_preview_file_stem(".."), "product");
        assert!(safe_preview_file_stem(&"x".repeat(300)).len() <= 180);
    }

    #[test]
    fn desktop_preview_save_writes_a_bounded_data_url_atomically() {
        let directory = std::env::temp_dir().join(format!("insar-preview-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).expect("temporary preview directory");
        let service = ApplicationService::new(ApplicationServiceConfig::liveness_only());
        let saved = service
            .save_preview_asset(PreviewSaveRequest {
                data_url: "data:image/png;base64,AQID".to_owned(),
                output_dir: directory.clone(),
                file_name: "S1A/IW:P1.zip".to_owned(),
            })
            .expect("preview save succeeds");
        assert_eq!(saved.file_name, "S1A_IW_P1__preview.png");
        assert_eq!(saved.bytes_written, 3);
        assert_eq!(
            std::fs::read(&saved.path).expect("saved preview bytes"),
            [1, 2, 3]
        );
        assert_eq!(
            service
                .save_preview_asset(PreviewSaveRequest {
                    data_url: "data:image/png;base64,AQID".to_owned(),
                    output_dir: PathBuf::from("relative-preview-dir"),
                    file_name: "P1".to_owned(),
                })
                .expect_err("relative output directory is rejected")
                .code(),
            "PREVIEW_DIR_INVALID"
        );
        std::fs::remove_dir_all(directory).expect("temporary preview directory cleanup");
    }

    #[test]
    fn required_store_rejects_older_capability_set() {
        let error = require_store_readiness_capability(&["legacy_liveness"], true).unwrap_err();
        assert_eq!(error.code(), "V3_STORE_READINESS_UNSUPPORTED");
        assert!(error.to_string().contains("Store readiness"));
    }

    #[test]
    fn explicit_capability_is_accepted() {
        assert!(
            require_store_readiness_capability(&[STORE_STARTUP_READINESS_CAPABILITY], true).is_ok()
        );
        assert!(ApplicationServiceConfig::liveness_only()
            .store_path()
            .is_none());
        assert_eq!(
            ApplicationService::new(ApplicationServiceConfig::liveness_only())
                .status()
                .unwrap(),
            ServiceStatus::LivenessOnly
        );
    }

    #[test]
    fn mapped_legacy_asf_product_is_admitted_to_the_search_batch() {
        let feature: AsfRawFeature = serde_json::from_value(json!({
            "type": "Feature",
            "properties": {
                "granuleName": "S1A_IW_SLC__1SDV_20260801T000000_20260801T000027_060000_000001_0001",
                "fileID": "S1A_IW_SLC__1SDV_20260801T000000_20260801T000027_060000_000001_0001-SLC",
                "platform": "Sentinel-1A",
                "processingLevel": "SLC",
                "fileName": "fixture.zip",
                "url": "https://datapool.asf.alaska.edu/SLC/fixture.zip"
            },
            "geometry": null
        }))
        .expect("valid ASF feature fixture");
        let mapped = map_sar_feature("SENTINEL-1", &feature).expect("mapped ASF product");

        let frozen = map_sar_product(mapped)
            .expect("a provider-mapped ASF product must be admitted by ApplicationService");
        assert_eq!(frozen.provider_id, "asf.catalog");
        assert_eq!(frozen.metadata["provider"], "asf.catalog");
    }

    #[test]
    fn sentinel_grd_families_freeze_only_official_provider_preview_evidence() {
        for processing_level in ["GRD_HD", "GRD_HS", "GRD_MD", "GRD_MS"] {
            let feature: AsfRawFeature = serde_json::from_value(json!({
                "type": "Feature",
                "properties": {
                    "granuleName": format!(
                        "S1A_IW_GRDH_1SDV_20260801T000000_20260801T000027_060000_000001_0001-{processing_level}"
                    ),
                    "fileID": format!("fixture-{processing_level}"),
                    "platform": "Sentinel-1A",
                    "processingLevel": processing_level,
                    "fileName": "fixture.zip",
                    "url": format!("https://datapool.asf.alaska.edu/{processing_level}/fixture.zip"),
                    "browse": format!("https://datapool.asf.alaska.edu/{processing_level}/fixture.jpg")
                },
                "geometry": null
            }))
            .expect("valid ASF GRD fixture");
            let mapped = map_sar_feature("SENTINEL-1", &feature).expect("mapped ASF GRD product");
            let frozen = map_sar_product(mapped).expect("Sentinel-1 GRD product must freeze");
            let assets = frozen.metadata["assets"]
                .as_array()
                .expect("Preview evidence is frozen");
            assert_eq!(assets.len(), 1, "{processing_level}");
            assert_eq!(assets[0]["role"], "browse", "{processing_level}");
            assert_eq!(
                assets[0]["native_roles"],
                json!(["browse"]),
                "{processing_level}"
            );
            assert!(assets[0]["url"].as_str().is_some(), "{processing_level}");
            assert!(matches!(
                frozen_preview_plan(&frozen),
                Ok(FrozenPreviewPlan::Direct {
                    product_class: insar_provider::preview::PreviewProductClass::Sentinel1Grd,
                    ..
                })
            ));
        }
    }

    #[test]
    fn nisar_frozen_catalogue_preview_uses_reviewed_https_route() {
        let captured: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/asf/nisar/provisional-rslc/granule-partial.json"
        ))
        .expect("captured NISAR fixture");
        let raw = captured["cmr_response"]["items"][0].clone();
        let envelope = CmrGranuleEnvelope::try_from(&raw).expect("NISAR UMM envelope");
        let profile = CollectionProfileRegistry
            .get(NISAR_PROFILE_ID)
            .expect("NISAR profile exists");
        let product =
            map_nisar_rslc_umm_granule(&envelope, profile).expect("captured NISAR product maps");
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": NISAR_LOGICAL_DATASET,
            "provider_id": NISAR_PROVIDER_ID,
            "native_collection_id": NISAR_NATIVE_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[-179.0, 63.0], [-173.0, 63.0], [-173.0, 66.0], [-179.0, 66.0], [-179.0, 63.0]]]
            },
            "cursor": null,
            "result_limit": 25,
            "filters": {
                "datetime": {"start": "2026-07-23T07:48:00Z", "end": "2026-07-23T07:49:00Z"},
                "production_configuration": "PR",
                "frame_coverage": "PARTIAL",
                "frequency_a_polarization": "HH",
                "range_bandwidth": "5",
                "joint_observation": false,
                "flight_direction": "DESCENDING",
                "look_direction": "LEFT",
                "track": 45,
                "frame": 56
            }
        }))
        .expect("captured NISAR query");
        let frozen = map_nisar_rslc_catalogue_product(product, &query)
            .expect("captured NISAR product freezes");
        let expected_item_id = frozen.item_id.clone();
        let (product_class, assets) =
            match frozen_preview_plan(&frozen).expect("frozen NISAR preview plan") {
                FrozenPreviewPlan::Direct {
                    product_key,
                    product_class,
                    assets,
                } => {
                    assert_eq!(product_key, expected_item_id);
                    (product_class, assets)
                }
                _ => panic!("captured NISAR product must have a direct preview plan"),
            };
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].role, super::AssetRole::Preview);
        assert!(assets[0].access_descriptor.is_none());
        assert_eq!(assets[0].media_type.as_deref(), Some("image/png"));
        let resolved = resolve_preview_candidate(&expected_item_id, product_class, &assets)
            .expect("frozen reviewed NISAR route resolves");
        assert_eq!(
            resolved.url.host_str(),
            Some("nisar.asf.earthdatacloud.nasa.gov")
        );
        assert!(resolved.url.path().ends_with("_LATLON.png"));
        assert!(resolved.url.query().is_none());
        assert!(resolved.url.fragment().is_none());
    }

    #[test]
    fn persisted_preview_source_survives_active_workbench_switch_for_all_supported_plans() {
        let path = std::env::temp_dir().join(format!(
            "insar-preview-source-authority-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after UNIX epoch")
                .as_nanos()
        ));
        let mut store = TaskStore::open(&path).expect("preview source store opens");
        let fixtures = [
            (
                "s1",
                "asf.catalog",
                json!({
                    "dataset": "SENTINEL-1",
                    "product_family": "GRD",
                    "product_key": "s1-source",
                    "assets": []
                })
                .as_object()
                .expect("fixture metadata object")
                .clone(),
            ),
            (
                "nisar",
                NISAR_PROVIDER_ID,
                json!({
                    "logical_dataset": NISAR_LOGICAL_DATASET,
                    "provider_product_key": "nisar-source",
                    "assets": []
                })
                .as_object()
                .expect("fixture metadata object")
                .clone(),
            ),
            (
                "cdse",
                "copernicus.dataspace.odata",
                json!({"logical_dataset": "optical.sentinel-2.l2a"})
                    .as_object()
                    .expect("fixture metadata object")
                    .clone(),
            ),
            (
                "landsat",
                LANDSAT_C2_L2_PROVIDER_ID,
                json!({"logical_dataset": LANDSAT_C2_L2_LOGICAL_DATASET})
                    .as_object()
                    .expect("fixture metadata object")
                    .clone(),
            ),
        ];

        for (label, provider_id, metadata) in fixtures {
            let source_item_id = format!("{label}-source");
            let mut source_metadata = metadata;
            if label == "s1" {
                source_metadata["product_key"] = Value::String(source_item_id.clone());
            }
            if label == "nisar" {
                source_metadata["provider_product_key"] = Value::String(source_item_id.clone());
            }
            let source_item = SnapshotItem {
                item_id: source_item_id.clone(),
                provider_id: provider_id.to_owned(),
                display_name: format!("{label} source"),
                acquisition_datetime: None,
                footprint_geojson: None,
                metadata: source_metadata,
                payload: Map::new(),
                asset_access: None,
                execution_policy: None,
            };
            let active_item = SnapshotItem {
                item_id: format!("{label}-active"),
                provider_id: provider_id.to_owned(),
                display_name: format!("{label} active"),
                acquisition_datetime: None,
                footprint_geojson: None,
                metadata: Map::new(),
                payload: Map::new(),
                asset_access: None,
                execution_policy: None,
            };
            for (batch_id, item, activate_workbench) in [
                (format!("{label}-source-batch"), source_item, false),
                (format!("{label}-active-batch"), active_item, true),
            ] {
                let batch = FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: batch_id.clone(),
                    provider_id: provider_id.to_owned(),
                    created_at: "2026-09-05T00:00:00Z".to_owned(),
                    aoi_name: format!("{label} AOI"),
                    aoi_geojson: None,
                    query: Map::new(),
                    search_stats: None,
                    candidates: vec![item],
                };
                store
                    .save_search_batch(&StoreSearchBatchRequest {
                        batch,
                        workbench: SearchWorkbenchState {
                            batch_id,
                            checked_item_ids: Vec::new(),
                            page_index: 0,
                            page_size: 24,
                            revision: 0,
                        },
                        activate_workbench,
                    })
                    .expect("preview source batch persists");
            }

            let source = load_frozen_preview_source(
                &store,
                &format!("{label}-source-batch"),
                &source_item_id,
            )
            .expect("source batch reads")
            .expect("source item remains available after active switch");
            assert_eq!(source.batch.batch_id, format!("{label}-source-batch"));
            assert_eq!(source.item.item_id, source_item_id);
            match label {
                "s1" | "nisar" => {
                    assert!(matches!(
                        frozen_preview_plan(&source.item),
                        Ok(FrozenPreviewPlan::Direct { .. })
                    ));
                }
                "cdse" => {
                    assert!(matches!(
                        frozen_preview_plan(&source.item),
                        Ok(FrozenPreviewPlan::Cdse)
                    ));
                }
                "landsat" => {
                    assert!(matches!(
                        frozen_preview_plan(&source.item),
                        Ok(FrozenPreviewPlan::Landsat)
                    ));
                }
                _ => unreachable!("fixture plan is exhaustive"),
            }
            assert!(load_frozen_preview_source(
                &store,
                &format!("{label}-source-batch"),
                "other-provider-item"
            )
            .expect("unknown source item read")
            .is_none());
        }
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[test]
    fn controlled_optical_preview_wrappers_emit_truthful_renderable_roles() {
        let sentinel = resolved_cdse_static_quicklook(
            "cdse-product-id".to_owned(),
            CdseQuicklookResponse {
                media_type: "image/jpeg".to_owned(),
                data_url: "data:image/jpeg;base64,AQID".to_owned(),
                byte_length: 3,
            },
        )
        .expect("CDSE static quicklook resolves as Preview");
        assert_eq!(sentinel.source_asset_role, PreviewAssetRole::Preview);
        assert_eq!(
            sentinel.source_asset_key,
            "cdse-product-id::PREVIEW_DISCOVERY::cdse.sentinel-2.static-quicklook.v1"
        );
        assert_eq!(
            sentinel.descriptor.source_asset_keys,
            vec![sentinel.source_asset_key.clone()]
        );

        let landsat = resolved_landsat_scene_browse(
            "landsat-product-id".to_owned(),
            LandsatBrowseResponse {
                media_type: "image/png".to_owned(),
                data_url: "data:image/png;base64,AQID".to_owned(),
                byte_length: 3,
            },
        )
        .expect("Microsoft scene Browse resolves as Browse without relabelling the parent product");
        assert_eq!(landsat.source_asset_role, PreviewAssetRole::Browse);
        assert_eq!(
            landsat.source_asset_key,
            "landsat-product-id::PREVIEW_DISCOVERY::planetary-computer.landsat-c2-l2.exact-scene-browse.v1"
        );
        assert_eq!(
            landsat.descriptor.source_asset_keys,
            vec![landsat.source_asset_key.clone()]
        );
    }

    #[test]
    fn seasat_search_batch_freezes_the_hdf5_primary_and_companion_assets() {
        let feature: AsfRawFeature = serde_json::from_str(include_str!(
            "../../insar-provider/tests/fixtures/asf-seasat/SS_01502_STD_F2536.geojson"
        ))
        .expect("real ASF SEASAT fixture");
        let mapped = map_sar_feature("SEASAT", &feature).expect("mapped SEASAT product");

        let frozen = map_sar_product(mapped).expect("SEASAT product must freeze");

        assert_eq!(frozen.payload["file_name"], "SS_01502_STD_F2536.h5");
        assert_eq!(frozen.payload["file_size_remote"], 425_004_910_u64);
        assert_eq!(
            frozen.metadata["checksum"],
            "fd99f89b380163d20c50b4a26fa0db12"
        );
        let assets = frozen.metadata["assets"]
            .as_array()
            .expect("SEASAT companion assets must survive SearchBatch freeze");
        assert_eq!(
            assets
                .iter()
                .filter(|asset| asset["role"] == "primary")
                .count(),
            1
        );
        assert!(assets.iter().any(|asset| {
            asset["role"] == "primary"
                && asset["file_name"] == "SS_01502_STD_F2536.h5"
                && asset["media_type"] == "application/x-hdf5"
                && asset["size_bytes"] == 425_004_910_i64
        }));
        assert!(assets.iter().any(|asset| {
            asset["role"] == "derived" && asset["file_name"] == "SS_01502_STD_F2536.tif"
        }));
        assert!(assets.iter().any(|asset| asset["role"] == "browse"));
        assert!(assets.iter().any(|asset| asset["role"] == "metadata"));
    }

    #[test]
    fn burst_search_batch_freezes_assets_and_authoritative_metadata() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../insar-provider/tests/fixtures/asf/burst/sentinel-1-bursts-first-page.json"
        ))
        .expect("official ASF Burst UMM fixture");
        let granule = fixture["items"][0].clone();
        let mapped = map_burst_umm_granule(&granule).expect("official Burst fixture must map");
        let expected_assets =
            serde_json::to_value(&mapped.assets).expect("Burst assets must serialize");
        let expected_burst =
            serde_json::to_value(&mapped.burst).expect("Burst identity must serialize");
        let expected_native_metadata = serde_json::to_value(&mapped.native_metadata)
            .expect("sanitized Burst native metadata must serialize");

        let frozen = map_sar_product(mapped).expect("Burst product must freeze");

        assert_eq!(frozen.metadata["assets"], expected_assets);
        let assets = frozen.metadata["assets"]
            .as_array()
            .expect("Burst assets must remain an array");
        assert_eq!(assets.len(), 2);
        for asset in assets {
            for field in [
                "asset_key",
                "stable_asset_key",
                "role",
                "native_roles",
                "url",
                "file_name",
                "media_type",
                "size_bytes",
            ] {
                assert!(
                    asset.get(field).is_some(),
                    "missing frozen asset field {field}"
                );
            }
        }

        assert_eq!(frozen.metadata["burst"], expected_burst);
        assert_eq!(frozen.metadata["burst"]["full_burst_id"], "141_301495_IW3");
        assert_eq!(frozen.metadata["burst"]["relative_burst_id"], 301_495);
        assert_eq!(frozen.metadata["burst"]["absolute_burst_id"], 5_714_273);
        assert_eq!(frozen.metadata["burst"]["subswath"], "IW3");
        assert_eq!(frozen.metadata["burst"]["burst_index"], 0);

        assert_eq!(frozen.metadata["native_metadata"], expected_native_metadata);
        assert_eq!(
            frozen.metadata["native_metadata"]["cmr_granule_concept_id"],
            "G3128067030-ASF"
        );
        assert_eq!(
            frozen.metadata["native_metadata"]["cmr_native_id"],
            "S1_301495_IW3_20141003T054235_VV_D5C8-BURST"
        );
        assert!(frozen.metadata["native_metadata"]["preview_available"].is_boolean());
        assert!(frozen.metadata["native_metadata"]["raw_authority"].is_object());
    }

    #[test]
    fn missing_dem_processing_engine_is_typed_and_does_not_open_the_store() {
        let service = ApplicationService::new(ApplicationServiceConfig::business_store(
            std::env::temp_dir().join(format!(
                "insar-studio-ac1-missing-worker-{}.sqlite3",
                std::process::id()
            )),
        ));
        let error = service
            .plan_dem_task(&DemTaskPlanRequest {
                request_id: "ac1-dem-plan".to_owned(),
                task_id: "ac1-dem-task".to_owned(),
                region_id: "ac1-region".to_owned(),
                resource_id: "dem.cop30.v1".to_owned(),
                capability_id: "capability.dem.cop30.aws-download.v1".to_owned(),
                dataset: "COP30".to_owned(),
                provider: "COPERNICUS_AWS".to_owned(),
                operation: DemTaskOperation::Inspect,
                input_path: std::path::PathBuf::from("input.tif"),
                output_root: std::path::PathBuf::from("output"),
                source_vertical_datum: None,
                target_vertical_datum: None,
                output_mode: None,
                created_at: "2026-08-14T00:00:00Z".to_owned(),
            })
            .expect_err("missing Worker must fail closed");
        assert_eq!(error.code(), "PROCESSING_ENGINE_UNAVAILABLE");
        assert!(service.status().is_ok());
    }

    fn frozen_cdse_product_item() -> SnapshotItem {
        let product_id = "00000000-0000-4000-8000-000000000011";
        let item_id = provider_product_key("copernicus.dataspace.odata", "SENTINEL-2", product_id)
            .expect("fixture product key");
        let stable_key = stable_asset_key(&item_id, "archive").expect("fixture asset key");
        SnapshotItem {
            item_id,
            provider_id: "copernicus.dataspace.odata".to_owned(),
            display_name: "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE"
                .to_owned(),
            acquisition_datetime: Some("2026-07-26T12:53:21Z".to_owned()),
            footprint_geojson: None,
            metadata: json!({
                "logical_dataset": "optical.sentinel-2.l2a",
                "native_collection_id": "SENTINEL-2",
                "collection_profile_id": "cdse.sentinel-2.l2a.collection.v1",
                "identity_rule_id": "cdse.sentinel-2.l2a.identity.v1",
                "provider_product_id": product_id,
                "provider_product_key": "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::00000000-0000-4000-8000-000000000011",
                "source_kind": "sentinel2",
                "product_kind": "optical_scene",
                "assets": [{
                    "asset_key": "archive",
                    "stable_asset_key": stable_key,
                    "role": "primary",
                    "native_roles": ["archive"]
                }]
            })
            .as_object()
            .expect("fixture metadata object")
            .clone(),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        }
    }

    fn frozen_cdse_product_batch(item: SnapshotItem) -> FrozenSearchBatch {
        let aoi = json!({
            "type":"Polygon",
            "coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]
        })
        .as_object()
        .expect("fixture AOI")
        .clone();
        let query = serde_json::to_value(CatalogSearchQuery {
            logical_dataset: "optical.sentinel-2.l2a".to_owned(),
            provider_id: "copernicus.dataspace.odata".to_owned(),
            native_collection_id: "SENTINEL-2".to_owned(),
            aoi_geojson: Some(aoi.clone()),
            cursor: None,
            result_limit: Some(1),
            filters: json!({"product_type":"S2MSI2A"})
                .as_object()
                .expect("fixture filters")
                .clone(),
        })
        .expect("fixture query serializes")
        .as_object()
        .expect("fixture query object")
        .clone();
        FrozenSearchBatch {
            schema_version: 1,
            batch_id: "cdse-quicklook-batch".to_owned(),
            provider_id: "copernicus.dataspace.odata".to_owned(),
            created_at: "2026-07-27T00:00:00Z".to_owned(),
            aoi_name: "CDSE quicklook AOI".to_owned(),
            aoi_geojson: Some(aoi),
            query,
            search_stats: None,
            candidates: vec![item],
        }
    }

    #[test]
    fn sentinel2_mapper_requires_the_exact_query_source_profile() {
        let product_id = "00000000-0000-4000-8000-000000000011";
        let item_id = provider_product_key("copernicus.dataspace.odata", "SENTINEL-2", product_id)
            .expect("fixture product key");
        let stable_key = stable_asset_key(&item_id, "archive").expect("fixture asset key");
        let product: ProviderProduct = serde_json::from_value(json!({
            "provider_id": "copernicus.dataspace.odata",
            "product_id": product_id,
            "display_name": "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE",
            "source_kind": "sentinel2",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2026-07-26T12:53:21Z",
            "footprint_geojson": {
                "type":"Polygon",
                "coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]
            },
            "assets": [{
                "asset_key": "archive",
                "stable_asset_key": stable_key,
                "role": "primary",
                "native_roles": ["archive"],
                "size_bytes": 2048
            }],
            "properties": {"product_type":"S2MSI2A", "online":true, "cloud_cover":4.2},
            "payload": {}
        }))
        .expect("valid Sentinel-2 provider product");
        let forged_query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": "microsoft.planetary-computer",
            "native_collection_id": "landsat-c2-l2",
            "aoi_geojson": {
                "type":"Polygon",
                "coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]
            },
            "result_limit": 1,
            "filters": {"product_type":"S2MSI2A"}
        }))
        .expect("syntactically valid forged source query");

        let error = map_sentinel2_catalogue_product(
            product,
            &forged_query,
            insar_provider::catalog::RouteUse::OfflineFixture,
        )
        .expect_err("query source cannot borrow the default Sentinel-2 profile");
        assert_eq!(error.code(), "CATALOGUE_PRODUCT_INVALID");
    }

    #[test]
    fn earth_search_sentinel2_mapper_freezes_public_thumbnail_and_identity() {
        let product_id = "S2A_27VXL_20260726_0_L2A";
        let item_id = provider_product_key("element84.earth-search", "sentinel-2-l2a", product_id)
            .expect("fixture product key");
        let stable_thumb = stable_asset_key(&item_id, "thumbnail").expect("fixture thumbnail key");
        let product: ProviderProduct = serde_json::from_value(json!({
            "provider_id": "element84.earth-search",
            "product_id": product_id,
            "display_name": "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE",
            "source_kind": "sentinel2",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2026-07-26T12:53:21Z",
            "footprint_geojson": {
                "type":"Polygon",
                "coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]
            },
            "assets": [{
                "asset_key": "thumbnail",
                "stable_asset_key": stable_thumb,
                "role": "thumbnail",
                "native_roles": ["thumbnail"],
                "url": "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/27/V/XL/2026/7/S2A_27VXL_20260726_0_L2A/thumbnail.jpg",
                "media_type": "image/jpeg"
            }],
            "properties": {"product_type":"S2MSI2A", "cloud_cover":4.2},
            "payload": {}
        }))
        .expect("valid Earth Search Sentinel-2 provider product");
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": "element84.earth-search",
            "native_collection_id": "sentinel-2-l2a",
            "aoi_geojson": {
                "type":"Polygon",
                "coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]
            },
            "result_limit": 1,
            "filters": {"product_type":"S2MSI2A"}
        }))
        .expect("valid Earth Search query");

        let frozen = map_sentinel2_catalogue_product(
            product,
            &query,
            insar_provider::catalog::RouteUse::OfflineFixture,
        )
        .expect("Earth Search Sentinel-2 product with public thumbnail should freeze");
        assert_eq!(frozen.item_id, item_id);
        assert_eq!(frozen.provider_id, "element84.earth-search");
        assert_eq!(
            frozen
                .metadata
                .get("assets")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn frozen_cdse_product_identity_excludes_quicklook_metadata() {
        let item = frozen_cdse_product_item();
        let batch = frozen_cdse_product_batch(item.clone());
        let evidence =
            frozen_cdse_quicklook_product(&batch, &item).expect("trusted fixture evidence");
        assert_eq!(evidence.product_id, "00000000-0000-4000-8000-000000000011");
        assert_eq!(
            evidence.safe_name,
            "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE"
        );
        assert_eq!(
            evidence.acquisition_datetime.as_deref(),
            Some("2026-07-26T12:53:21Z")
        );
        let assets = item.metadata["assets"].as_array().expect("archive list");
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0]["asset_key"], "archive");
        assert!(assets[0].get("href").is_none());
        assert!(assets[0].get("media_type").is_none());

        let mut legacy_thumbnail = item.clone();
        legacy_thumbnail
            .metadata
            .get_mut("assets")
            .and_then(Value::as_array_mut)
            .expect("fixture archive list")
            .push(json!({"role":"thumbnail","legacy":"ignored"}));
        assert!(
            frozen_cdse_quicklook_product(&batch, &legacy_thumbnail).is_ok(),
            "legacy thumbnail metadata is ignored before lazy discovery"
        );

        let mut missing = item.clone();
        missing.metadata.remove("provider_product_id");
        assert!(matches!(
            frozen_cdse_quicklook_product(&batch, &missing),
            Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid)
        ));

        let mut conflicting = item.clone();
        let duplicate = conflicting
            .metadata
            .get_mut("assets")
            .and_then(|assets| assets.as_array_mut())
            .expect("fixture archive list")[0]
            .clone();
        conflicting
            .metadata
            .get_mut("assets")
            .and_then(|assets| assets.as_array_mut())
            .expect("fixture archive list")
            .push(duplicate);
        assert!(matches!(
            frozen_cdse_quicklook_product(&batch, &conflicting),
            Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid)
        ));

        let mut untrusted_archive = item.clone();
        untrusted_archive
            .metadata
            .get_mut("assets")
            .and_then(|assets| assets.as_array_mut())
            .expect("fixture archive list")[0]["href"] = json!("https://example.invalid/archive");
        assert!(matches!(
            frozen_cdse_quicklook_product(&batch, &untrusted_archive),
            Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid)
        ));

        let mut forged_source = batch.clone();
        forged_source.query = json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": "microsoft.planetary-computer",
            "native_collection_id": "landsat-c2-l2",
            "aoi_geojson": forged_source.aoi_geojson.clone(),
            "cursor": null,
            "result_limit": 1,
            "filters": {"product_type":"S2MSI2A"}
        })
        .as_object()
        .expect("forged query object")
        .clone();
        assert!(matches!(
            frozen_cdse_quicklook_product(&forged_source, &item),
            Err(ApplicationServiceError::CdseQuicklookEvidenceInvalid)
        ));
    }

    #[test]
    fn reviewed_cdse_odata_failures_keep_a_typed_application_error_code() {
        for code in [
            "CDSE_ODATA_TIMEOUT",
            "CDSE_ODATA_HTTP_RATE_LIMITED",
            "CDSE_ODATA_HTTP_REDIRECT",
            "CDSE_ODATA_PAGE_INVALID",
        ] {
            let error = ApplicationServiceError::Provider(
                ContractError::new(code, "fixture", Map::new()).expect("safe fixture error"),
            );
            assert_eq!(error.code(), code);
        }
        let unknown = ApplicationServiceError::Provider(
            ContractError::new("PROVIDER_FIXTURE_UNKNOWN", "fixture", Map::new())
                .expect("safe fixture error"),
        );
        assert_eq!(unknown.code(), "V3_PROVIDER_OPERATION_FAILED");
    }

    #[test]
    fn reviewed_stac_and_planetary_computer_failures_keep_typed_application_codes() {
        for code in [
            "STAC_IDENTITY_MISMATCH",
            "STAC_RESPONSE_INVALID",
            "STAC_TIMEOUT",
            "STAC_NETWORK_ERROR",
            "STAC_RATE_LIMITED",
            "STAC_SERVER_ERROR",
            "PLANETARY_COMPUTER_TRANSPORT_FAILED",
            "PLANETARY_COMPUTER_RESPONSE_TOO_LARGE",
            "PLANETARY_COMPUTER_RESPONSE_INVALID",
            "PLANETARY_COMPUTER_IDENTITY_MISMATCH",
        ] {
            let error = ApplicationServiceError::Provider(
                ContractError::new(code, "fixture", Map::new()).expect("safe fixture error"),
            );
            assert_eq!(error.code(), code);
        }
    }

    #[test]
    fn mpc_sign_failure_classes_keep_typed_application_codes() {
        for code in [
            "MPC_ASSET_SIGN_RATE_LIMITED",
            "MPC_ASSET_SIGN_REJECTED",
            "MPC_ASSET_SIGN_UNAVAILABLE",
            "MPC_ASSET_SIGN_TRANSPORT",
            "MPC_ASSET_SIGN_RESPONSE_INVALID",
        ] {
            let error = ApplicationServiceError::Provider(
                ContractError::new(code, "fixture", Map::new()).expect("safe fixture error"),
            );
            assert_eq!(error.code(), code);
        }
    }

    #[test]
    fn reviewed_burst_failures_keep_a_typed_application_error_code() {
        for code in [
            "ASF002",
            "ASF_QUERY_INVALID",
            "ASF_QUERY_UNSUPPORTED",
            "ASF_PAGINATION_INVALID",
            "ASF_IDENTITY_CONFLICT",
        ] {
            let error = ApplicationServiceError::Provider(
                ContractError::new(code, "fixture", Map::new()).expect("safe fixture error"),
            );
            assert_eq!(error.code(), code);
        }
    }
    #[test]
    fn reviewed_bhoonidhi_failures_keep_typed_application_error_codes() {
        for code in [
            "BHOONIDHI_ITEM_INVALID",
            "BHOONIDHI_QUERY_INVALID",
            "BHOONIDHI_CURSOR_INVALID",
            "BHOONIDHI_RESPONSE_INVALID",
            "BHOONIDHI_RESPONSE_TOO_LARGE",
            "BHOONIDHI_PRODUCT_IDENTITY_CONFLICT",
            "BHOONIDHI_SEARCH_BUDGET_EXCEEDED",
            "BHOONIDHI_SEARCH_CANCELLED",
            "BHOONIDHI_AUTH_UNAVAILABLE",
            "BHOONIDHI_OAUTH_CONFIGURATION_INVALID",
            "BHOONIDHI_CREDENTIALS_MISSING",
            "BHOONIDHI_CREDENTIALS_INVALID",
            "BHOONIDHI_AUTH_REJECTED",
            "BHOONIDHI_MAX_SESSIONS",
            "BHOONIDHI_ACTIVATION_REQUIRED",
            "BHOONIDHI_AUTH_TRANSPORT_FAILED",
            "BHOONIDHI_AUTH_TIMEOUT",
            "BHOONIDHI_AUTH_RESPONSE_INVALID",
            "BHOONIDHI_REFRESH_UNAVAILABLE",
            "BHOONIDHI_REFRESH_EXHAUSTED",
            "BHOONIDHI_SESSION_CHANGED",
            "BHOONIDHI_SESSION_UNAVAILABLE",
            "BHOONIDHI_SYNCHRONIZATION_FAILED",
            "BHOONIDHI_UNAUTHORIZED",
            "BHOONIDHI_FORBIDDEN",
            "BHOONIDHI_RATE_LIMITED",
            "BHOONIDHI_SERVER_ERROR",
            "BHOONIDHI_HTTP_ERROR",
            "BHOONIDHI_NETWORK_ERROR",
            "BHOONIDHI_ENDPOINT_INVALID",
            "BHOONIDHI_CONFIG_INVALID",
        ] {
            let error = ApplicationServiceError::Provider(
                ContractError::new(code, "fixture", Map::new()).expect("safe fixture error"),
            );
            assert_eq!(error.code(), code);
        }
    }

    fn bhoonidhi_fixture_feature() -> Value {
        let item_id = "NISAR_S1_PR_RSLC_029_077_D_086_2500_CRNA_A_20260830T132132_20260830T132141_P00500_N_P_I_001";
        json!({
            "type": "Feature",
            "id": item_id,
            "collection": BHOONIDHI_NATIVE_COLLECTION_ID,
            "bbox": [111.0, 31.0, 110.0, 30.0],
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 31.0], [110.0, 30.0]]]
            },
            "properties": {
                "ID": item_id,
                "Online": "Y",
                "Coverage": "P",
                "ProductCode": "ST0000KED",
                "SoftwareVersion": "1.4.00",
                "Producer": "ISRO",
                "Platform": "NISAR",
                "Instrument": "SSAR",
                "Mode": "W",
                "Polarization": "CP",
                "AcquistionMode": "J",
                "Node": "Descending",
                "Look_dir": "Left",
                "Scene_Identifier": "2026242132100_77_86",
                "datetime": "2026-08-30T13:21:32Z"
            },
            "assets": {
                "metadata": {"href": "https://bhoonidhi.nrsc.gov.in/stac/NISAR/item.met", "type": "application/json"},
                "thumbnail": {"href": "https://bhoonidhi.nrsc.gov.in/stac/NISAR/item.jpg", "type": "image/jpeg"}
            }
        })
    }

    fn bhoonidhi_query() -> CatalogSearchQuery {
        serde_json::from_value(json!({
            "logical_dataset": BHOONIDHI_LOGICAL_DATASET,
            "provider_id": BHOONIDHI_PROVIDER_ID,
            "native_collection_id": BHOONIDHI_NATIVE_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[109.0, 29.0], [112.0, 29.0], [112.0, 32.0], [109.0, 29.0]]]
            },
            "cursor": null,
            "result_limit": 1,
            "filters": {
                "datetime": {"start": "2026-08-30T00:00:00Z", "end": "2026-08-31T00:00:00Z"},
                "online": "Y",
                "mode": "W",
                "polarization": "CP"
            }
        }))
        .expect("Bhoonidhi S-RSLC query fixture")
    }

    #[test]
    fn bhoonidhi_product_freezes_exact_identity_and_runtime_download_descriptor() {
        let product = map_bhoonidhi_feature(&bhoonidhi_fixture_feature())
            .expect("provider mapper accepts the reviewed S-RSLC feature");
        let item = map_bhoonidhi_catalogue_product(product, &bhoonidhi_query())
            .expect("application mapper freezes the reviewed S-RSLC product");
        assert_eq!(item.provider_id, BHOONIDHI_PROVIDER_ID);
        assert_eq!(item.display_name, "2026242132100_77_86");
        assert_eq!(
            item.metadata["native_collection_id"],
            BHOONIDHI_NATIVE_COLLECTION_ID
        );
        assert_eq!(
            item.metadata["product_family"],
            BHOONIDHI_NATIVE_COLLECTION_ID
        );
        assert_eq!(
            item.metadata["processing_level"],
            BHOONIDHI_REQUIRED_PRODUCT_TYPE
        );
        assert_eq!(item.metadata["product_kind"], "sar_slc");
        assert_eq!(item.metadata["properties"]["Mode"], "W");
        assert_eq!(item.metadata["properties"]["mode"], "W");
        assert_eq!(item.metadata["properties"]["Polarization"], "CP");
        assert_eq!(item.metadata["properties"]["polarization"], "CP");
        assert_eq!(item.metadata["properties"]["Coverage"], "P");
        assert_eq!(
            item.metadata["properties"]["flight_direction"],
            "DESCENDING"
        );
        assert_eq!(item.metadata["properties"]["look_direction"], "LEFT");
        assert_eq!(item.metadata["flight_direction"], "DESCENDING");
        assert_eq!(item.metadata["look_direction"], "LEFT");
        assert_eq!(
            item.metadata["properties"]["provider_bbox"],
            json!([111.0, 31.0, 110.0, 30.0])
        );
        assert_eq!(item.metadata["assets"].as_array().unwrap().len(), 2);
        let access = item
            .asset_access
            .as_ref()
            .expect("primary runtime identity");
        let expected_file_name = format!("{}.h5", access.reference.provider_product_id);
        assert_eq!(access.reference.provider_id, BHOONIDHI_PROVIDER_ID);
        assert_eq!(
            access.reference.collection_id,
            BHOONIDHI_NATIVE_COLLECTION_ID
        );
        assert_eq!(access.reference.asset_key, "data");
        assert_eq!(
            access.file_name.as_deref(),
            Some(expected_file_name.as_str())
        );
        assert_eq!(access.expected_size_bytes, None);
        assert_eq!(access.checksum_value, None);
        assert_eq!(access.descriptor.resolver_id, BHOONIDHI_RESOLVER_POLICY_ID);
        assert_eq!(
            access.descriptor.stable_href.as_deref(),
            Some("https://bhoonidhi-api.nrsc.gov.in/download")
        );
        assert_eq!(
            access.descriptor.credential_profile.as_deref(),
            Some("bhoonidhi-default")
        );
        assert_eq!(
            item.execution_policy.as_ref().unwrap().policy_id,
            BHOONIDHI_RESOLVER_POLICY_ID
        );
        match frozen_preview_plan(&item).expect("Bhoonidhi public thumbnail is previewable") {
            FrozenPreviewPlan::Direct {
                product_class: PreviewProductClass::BhoonidhiSsarRslc,
                assets,
                ..
            } => {
                assert_eq!(assets.len(), 2);
                assert!(assets
                    .iter()
                    .any(|asset| asset.role == AssetRole::Thumbnail));
            }
            _ => panic!("Bhoonidhi S-band must use the direct thumbnail preview path"),
        }
    }

    #[test]
    fn bhoonidhi_product_rejects_raw_normalized_science_property_conflicts() {
        let product = map_bhoonidhi_feature(&bhoonidhi_fixture_feature())
            .expect("provider mapper accepts the reviewed S-RSLC feature");
        let mut product = product;
        product
            .properties
            .insert("mode".to_owned(), Value::String("HH".to_owned()));
        let error = map_bhoonidhi_catalogue_product(product, &bhoonidhi_query())
            .expect_err("conflicting raw and normalized mode must fail closed");
        assert_eq!(error.code(), "BHOONIDHI_PRODUCT_SCOPE_MISMATCH");
    }

    #[test]
    fn preferred_cdse_checksum_returns_error_without_panicking_when_value_missing() {
        let mut props = serde_json::Map::new();
        props.insert(
            "checksums".to_owned(),
            serde_json::json!([{"algorithm": "SHA256", "value": 12345}]),
        );
        let err = super::preferred_cdse_checksum(&props)
            .expect_err("non-string checksum value must return typed error rather than panic");
        assert_eq!(err.code(), "CATALOGUE_PRODUCT_INVALID");
    }

    #[test]
    fn catalog_search_cache_max_age_uses_2h_for_bhoonidhi_and_24h_for_others() {
        assert_eq!(
            super::catalog_search_cache_max_age_seconds("isro.nrsc.bhoonidhi"),
            2 * 60 * 60
        );
        assert_eq!(
            super::catalog_search_cache_max_age_seconds("ISRO_BHOONIDHI"),
            2 * 60 * 60
        );
        for provider_id in [
            "asf",
            "asf.catalog",
            "cdse",
            "microsoft.planetary-computer",
            "usgs.m2m",
        ] {
            assert_eq!(
                super::catalog_search_cache_max_age_seconds(provider_id),
                24 * 60 * 60,
                "{provider_id}"
            );
        }
    }
}
