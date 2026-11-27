//! Shared resumable file and transfer engine for provider downloads.

pub const NISAR_SUBSET_V2_DELIVERY_KIND: &str = "remote_derived_subset_v2";

#[cfg(test)]
extern crate self as insar_download;

mod artifact_validation;
mod asset_resolver;
mod bhoonidhi_credentials;
#[cfg(test)]
mod bhoonidhi_known_product_transfer_tests;
mod bhoonidhi_oauth;
#[cfg(test)]
mod bhoonidhi_probe_retry_tests;
mod bhoonidhi_runtime;
#[cfg(test)]
mod cdse_live_acceptance;
#[cfg(test)]
mod cdse_loopback_transfer;
mod cdse_network;
mod cdse_oauth;
mod cdse_quicklook;
mod checksum;
mod credentials;
mod destination_lease;
pub mod disk_guard;
mod execution_policy;
mod execution_policy_registry;
mod file_core;
mod file_identity;
mod hdf5_chunk_visit;
mod hdf5_file;
mod hdf5_plugin_isolation;
mod hdf5_range_transfer;
mod http_transfer;
// Downloader D3: local-first dedup before credentials, resolution and network.
mod local_reuse;
// Downloader D3-2: the coordinator's batch pre-scan of queued items.
mod local_scan;
// B15: task-scoped, user-confirmed cleanup of renamed-aside conflict files.
pub mod conflict_cleanup;
mod long_path;
mod nisar_hdf5_dimension_scales;
mod nisar_hdf5_validation;
#[cfg(feature = "nisar-subset-live-acceptance")]
mod nisar_isce3_oracle;
#[cfg(feature = "nisar-subset-production")]
mod nisar_live_admission;
#[cfg(feature = "nisar-subset-live-acceptance")]
mod nisar_live_evidence;
pub mod nisar_multilevel_subset;
#[cfg(feature = "nisar-subset-production")]
mod nisar_output_diagnostics_v2;
mod nisar_plan_authority;
#[cfg(feature = "nisar-subset-production")]
mod nisar_planning_v2;
mod nisar_science_root;
mod nisar_ssar_hdf5_validation;
mod redirect_policy;
mod resolved_part_checkpoint;
// Task 2 intentionally stops before the production execution route consumes this layer.
#[cfg(feature = "nisar-subset-production")]
#[allow(dead_code)]
mod nisar_science_transfer_v2;
// Issue #8 slice 6: bounded streaming and output admission of V2 windows.
#[cfg(feature = "nisar-subset-production")]
mod nisar_subset_stream_v2;
// Task 3 exposes the reviewed V2 Writer contract internally; Task 4 wires the route.
#[cfg_attr(feature = "nisar-subset-production", allow(dead_code))]
mod nisar_subset_writer;
#[cfg(feature = "nisar-subset-production")]
mod nisar_workspace_rebinding_v2;
// Downloader enhancement D1: per-artifact admission and output slots of the V2 writer.
#[cfg(feature = "nisar-subset-production")]
mod nisar_writer_admission_v2;
// Downloader enhancement D2: stage timings of one V2 writer run.
#[cfg(feature = "nisar-subset-production")]
mod nisar_writer_timings_v2;
#[cfg(test)]
mod perf_evidence_tests;
mod pinned_directory;
#[cfg(test)]
mod process_isolation_tests;
mod production_worker;
mod resolver_execution;
mod safe_promotion;
#[cfg(test)]
mod safe_promotion_qualification_tests;
mod scheduler;
mod segment_planner;
#[cfg(test)]
mod test_suites;
#[cfg(test)]
mod tls_certificate_tests;
mod transfer_activity;
mod transfer_admission;
mod transfer_backend;
pub mod transfer_engine_prefs;
pub mod transfer_tuning;
// Downloader enhancement D1d: one bounded retry policy for transient requests.
mod transient_retry;

pub(crate) use artifact_validation::ArtifactValidatorRegistry;
#[cfg(test)]
pub(crate) use artifact_validation::CompletedArtifactValidator;
pub use artifact_validation::{
    ArtifactValidationContext, ArtifactValidationError, ArtifactValidationErrorKind,
    ArtifactValidationIssue, ArtifactValidationPlan, ArtifactValidatorBinding, ArtifactValidatorId,
    NisarFrequency, NisarPolarization, NisarRslcValidationContext,
    NISAR_RSLC_STRUCTURE_VALIDATOR_BINDING, NISAR_RSLC_STRUCTURE_VALIDATOR_ID,
    NISAR_RSLC_STRUCTURE_VALIDATOR_V2_BINDING, NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
    NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION, NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_BINDING,
    NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_ID, NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
};
pub use asset_resolver::{
    classify_asset_access, evaluate_refresh, run_shadow_isolated, run_shadow_then_legacy,
    AssetAccessClassification, AssetResolver, AssetResolverRegistry, AuthRefreshBudget,
    AuthRefreshPermit, BhoonidhiAssetResolver, BhoonidhiDownloadPermit, BhoonidhiDownloadProbe,
    BhoonidhiDownloadProbeError, BhoonidhiDownloadProbeEvidence, CdseArchiveAssetResolver, Clock,
    CredentialMaterial, CredentialProvider, CredentialProviderError,
    EarthSearchSentinel2BandAssetResolver, EarthdataAssetResolver, LandsatM2mAssetResolver,
    LegacyAdapter, LegacyTransferIntent, LocalFileAssetResolver,
    PlanetaryComputerLandsatAssetResolver, PlanetaryComputerReviewedAssetResolver, ProxyPolicy,
    PublicHttpsAssetResolver, RefreshDecision, RefreshEvaluation, RefreshReason, RemoteValidator,
    ReqwestBhoonidhiDownloadProbe, ResolveContext, ResolveError, ResolveErrorKind, ResolvedAsset,
    ResolvedChecksum, ResolvedMethod, ResolverShadowDiagnostics, ResolverShadowInput,
    ResolverShadowObservation, ResolverShadowObserver, ResolverShadowRuntime, ShadowAccessIntent,
    ShadowAssetClass, ShadowAuthKind, ShadowCredentialShapeProvider, ShadowDestinationPolicy,
    ShadowHostClass, ShadowMismatchCode, ShadowNetworkAccess, ShadowOutcome, ShadowPathClass,
    ShadowProviderClass, ShadowResolveContext, SystemClock, TimeoutPolicy,
    TransferIntentFingerprint, BHOONIDHI_DOWNLOAD_ASSET_KEY, BHOONIDHI_DOWNLOAD_ORIGIN,
    BHOONIDHI_DOWNLOAD_PATH, BHOONIDHI_DOWNLOAD_PROBE_CONNECT_TIMEOUT,
    BHOONIDHI_DOWNLOAD_PROBE_MAX_RESPONSE_BYTES, BHOONIDHI_DOWNLOAD_PROBE_RANGE,
    BHOONIDHI_DOWNLOAD_PROBE_TOTAL_TIMEOUT, BHOONIDHI_DOWNLOAD_RESOLVER_ID, BHOONIDHI_DOWNLOAD_URL,
    BHOONIDHI_MAX_CONCURRENT_DOWNLOADS, EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID,
    PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID, PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID,
    PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID,
    PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID, SHADOW_DESCRIPTOR_UNAVAILABLE,
    SHADOW_ORBIT_DESCRIPTOR_UNAVAILABLE,
};
pub use bhoonidhi_credentials::{
    clear_desktop_bhoonidhi_credentials, desktop_bhoonidhi_readiness,
    read_desktop_bhoonidhi_login_material, resolve_desktop_bhoonidhi_credential,
    store_desktop_bhoonidhi_credentials, BhoonidhiCredentialError, BhoonidhiCredentialReadiness,
    BHOONIDHI_CREDENTIAL_MAX_SOURCE_BYTES, BHOONIDHI_CREDENTIAL_PROFILE,
    BHOONIDHI_CREDENTIAL_TARGET,
};
pub use bhoonidhi_oauth::{
    BhoonidhiAccessToken, BhoonidhiAuthError, BhoonidhiLoginMaterial, BhoonidhiOAuthSession,
    BhoonidhiRefreshOutcome, BhoonidhiSession, BhoonidhiTokenEndpoint, BhoonidhiTokenGrant,
    BhoonidhiTokenSnapshot, ReqwestBhoonidhiAuthEndpoint, ReqwestBhoonidhiTokenEndpoint,
    BHOONIDHI_API_ORIGIN, BHOONIDHI_AUTH_MAX_RESPONSE_BYTES, BHOONIDHI_LOGOUT_ENDPOINT,
    BHOONIDHI_TOKEN_ENDPOINT,
};
pub use bhoonidhi_runtime::{BhoonidhiRuntimeCredentialProvider, BhoonidhiSessionTokenProvider};
pub use cdse_network::{
    CdseArchiveNetworkRuntime, CdseNetworkDiagnostic, CdseNetworkDiagnosticStage,
    CdseNetworkDiagnostics, CdseNetworkFoundation, CdseNetworkProbe, ReqwestCdseNetworkProbe,
};
pub use cdse_oauth::{
    CdseAuthError, CdseLoginMaterial, CdseOAuthSession, CdseRefreshOutcome, CdseTokenEndpoint,
    CdseTokenGrant, CdseTokenSnapshot, ReqwestCdseTokenEndpoint, CDSE_IDENTITY_TOKEN_ENDPOINT,
    CDSE_OAUTH_CLIENT_ID,
};
pub use cdse_quicklook::{
    CdseQuicklookClient, CdseQuicklookError, CdseQuicklookImage, CDSE_QUICKLOOK_MAX_BYTES,
    CDSE_QUICKLOOK_ORIGIN,
};
pub use checksum::{
    verify_file_checksum, verify_file_checksum_with_report, ChecksumAlgorithm, ChecksumError,
    ChecksumVerificationReport, ExpectedChecksum,
};
pub use credentials::{
    clear_desktop_bhoonidhi, clear_desktop_cdse, clear_desktop_earthdata,
    clear_desktop_gacos_email, clear_desktop_opentopography_key, clear_desktop_usgs,
    desktop_bhoonidhi_status, desktop_cdse_status, desktop_earthdata_readiness,
    desktop_earthdata_status, desktop_gacos_status, desktop_opentopography_status,
    desktop_usgs_status, read_desktop_credential_for_edit, resolve_desktop_bhoonidhi_login,
    resolve_desktop_cdse_login, resolve_desktop_opentopography_key, resolve_desktop_usgs_login,
    resolve_earthdata_burst_credential, resolve_earthdata_burst_credential_with,
    resolve_earthdata_credential, resolve_earthdata_credential_profile_with,
    resolve_earthdata_credential_with, resolve_earthdata_search_token,
    reveal_desktop_credential_field, store_desktop_bhoonidhi_login, store_desktop_cdse_login,
    store_desktop_earthdata_login, store_desktop_earthdata_token, store_desktop_gacos_email,
    store_desktop_gacos_result_link, store_desktop_opentopography_key, store_desktop_usgs_login,
    update_desktop_credential_field, with_desktop_gacos_email, with_desktop_gacos_result_link,
    DesktopCredentialError, DesktopCredentialField, EarthdataCredentialError,
    EarthdataCredentialReadiness, EarthdataCredentialSource,
    EnvironmentEarthdataCredentialProvider, ProductionCredentialProvider,
    BHOONIDHI_KEYRING_SERVICE, CDSE_KEYRING_SERVICE, EARTHDATA_CREDENTIAL_PROFILE,
    EARTHDATA_KEYRING_SERVICE, GACOS_KEYRING_SERVICE, OPENTOPOGRAPHY_KEYRING_SERVICE,
    USGS_KEYRING_SERVICE,
};
pub use credentials::{
    clear_active_legacy_earthengine_entry, delete_desktop_agent_model_key,
    desktop_agent_model_key_ready, read_active_legacy_earthengine_entry,
    read_local_vault_map_readonly, scrub_local_vault_services, store_desktop_agent_model_key,
    with_desktop_agent_model_key, AGENT_MODEL_KEYRING_SERVICE,
};
#[doc(hidden)]
pub use destination_lease::{
    destination_lease_key_for_qualification_test, DestinationLeaseController,
    DestinationLeaseError, DestinationLeaseGuard, DestinationLeaseSnapshot, OpaqueDestinationKey,
};
pub(crate) use execution_policy::evaluate_resolver_cutover_eligibility_with_artifact_validators;
pub use execution_policy::{
    build_burst_frozen_asset_access, evaluate_resolver_cutover_eligibility,
    validate_execution_policy, AsfCollectionProfileRegistry, AssetExecutionPolicyDefinition,
    AssetExecutionPolicyRegistry, AuthRefreshPolicyId, EligibilityReason, EligibleResolverAsset,
    ExecutionDeliveryModel, ExecutionPolicyError, ResolverCutoverEligibility,
    ValidatedExecutionPolicy, APPROVED_CREDENTIAL_PROFILES, APPROVED_PROCESSING_LEVELS,
    APPROVED_PRODUCT_FAMILIES, APPROVED_RESOLVER_IDS, ASF_BURST_ASSET_EXECUTION_POLICY_ID,
    ASF_BURST_COLLECTION_PROFILE_ID, ASF_SENTINEL1_STANDARD_POLICY_ID,
    ASF_SENTINEL1_STANDARD_PROFILE_ID, CDSE_APPROVED_CREDENTIAL_PROFILES,
    CDSE_APPROVED_RESOLVER_IDS, CDSE_SENTINEL2_L2A_PROFILE_ID, EARTHDATA_BURST_CREDENTIAL_PROFILE,
};
pub use execution_policy::{
    PLANETARY_COMPUTER_HLS2_L30_POLICY_ID, PLANETARY_COMPUTER_HLS2_S30_POLICY_ID,
};
pub use execution_policy_registry::{
    build_cdse_archive_access, build_earth_search_sentinel2_band_cog_item,
    build_landsat_c2_l2_bundle_access, build_offline_cdse_archive_access,
    build_planetary_computer_landsat_asset_access, build_planetary_computer_reviewed_cog_item,
    CatalogExecutionPolicyDefinition, CatalogExecutionPolicyError, CatalogExecutionPolicyInput,
    CatalogExecutionPolicyRegistry, CatalogPolicyContext, CdseArchiveEvidence,
    CDSE_SENTINEL2_L2A_POLICY_ID, EARTH_SEARCH_SENTINEL2_L2A_BAND_COG_POLICY_ID,
    PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID, PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID,
    PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID,
    PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY_ID, USGS_LANDSAT_C2_L2_POLICY_ID,
    USGS_LANDSAT_C2_L2_POLICY_ID_V1, USGS_LANDSAT_C2_L2_POLICY_ID_V2,
    USGS_LANDSAT_C2_L2_RESOLVER_ID, USGS_LANDSAT_C2_L2_RESOLVER_ID_V1,
    USGS_LANDSAT_C2_L2_RESOLVER_ID_V2,
};
pub use file_core::{
    finalize_part, inspect_destination, inspect_resolved_destination, open_part_for_transfer,
    part_completed_bytes, part_path, plan_download_target, plan_sar_catalog_target, read_exact_at,
    DownloadFileError, FilePreparation, ResolvedPartContext,
};
pub use hdf5_plugin_isolation::{
    establish_hdf5_plugin_isolation, establish_hdf5_process_isolation,
    hdf5_plugin_isolation_established, hdf5_plugin_loading_disabled, hdf5_process_isolation_status,
    Hdf5ProcessIsolationStatus,
};
#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
#[doc(hidden)]
pub use hdf5_plugin_isolation::{Hdf5TestLock, Hdf5TestLockGuard, HDF5_TEST_LOCK};
// Used by the live-acceptance local qualification tests in
// nisar_subset_writer_tests.rs, which offline-nisar-subset-test-support also
// compiles outside cfg(test); other builds with the feature leave it unused.
#[cfg(feature = "nisar-subset-live-acceptance")]
#[cfg_attr(
    not(any(test, feature = "offline-nisar-subset-test-support")),
    allow(unused_imports)
)]
pub(crate) use hdf5_range_transfer::NisarLiveTransportAdmissionV1;
#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub use hdf5_range_transfer::{
    build_live_nisar_subset_request, discover_live_nisar_metadata_chunks,
    live_nisar_range_plan_from_sparse_hdf5, prefetch_live_nisar_metadata,
    preflight_live_nisar_source, seed_live_metadata_range, validate_nisar_live_acceptance_budget,
    NisarLiveMetadataDiscoveryV1, NisarLiveMetadataPrefetchV1, NisarLiveNetworkBudgetV1,
    NisarLivePreflightV1, NISAR_LIVE_MAX_REQUESTS, NISAR_LIVE_MAX_RESPONSE_BODY_BYTES,
    NISAR_LIVE_METADATA_PREFIX_BYTES,
};
pub use hdf5_range_transfer::{
    Hdf5PhysicalRangeV1, Hdf5RangePlanV1, Hdf5RangeTransferError, Hdf5RangeTransferMetricsV1,
    Hdf5RangeTransferResultV1, Hdf5RangeWorkspaceV1, OfflineHdf5RangeTransfer,
};
pub use http_transfer::{
    AcceptedPollBudget, AcceptedPollBudgetError, AuthRejection, BurstAuthEndpoints,
    BurstSessionError, EarthdataAuthProbeOutcome, HttpTransfer, RedirectBudget,
    ResolvedTransferResult, ResolvedTransferSession, ResponseHostClass, TransferAttemptLedger,
    TransferAuth, TransferOptions, TransferProgress, TransferRequest, TransferRequestError,
    TransferResult, TransferUpdate, MAX_REDIRECT_HOPS,
};
#[cfg(feature = "nisar-subset-live-acceptance")]
#[doc(hidden)]
pub use nisar_isce3_oracle::NisarIsce3LiveGeometryGateV1;
#[cfg(feature = "nisar-subset-production")]
pub(crate) use nisar_live_admission::validate_nisar_live_admission;
// Used by the same local qualification tests (see NisarLiveTransportAdmissionV1).
#[cfg(feature = "nisar-subset-live-acceptance")]
#[cfg_attr(
    not(any(test, feature = "offline-nisar-subset-test-support")),
    allow(unused_imports)
)]
pub(crate) use nisar_live_admission::NisarLiveAdmissionError;
#[cfg(feature = "nisar-subset-live-acceptance")]
#[doc(hidden)]
pub use nisar_live_evidence::{
    NisarLiveAcceptanceEvidenceSink, NisarLiveAcceptanceEvidenceV2, NisarLiveBudgetEvidenceV2,
    NisarLiveGeometryEvidenceV2, NisarLiveRangeFailureObservationV2, NisarLiveRangeObservationV2,
    NisarLiveScientificValidationEvidenceV2, NISAR_FREQ_B_EVIDENCE_SCHEMA, NISAR_FREQ_B_GATE,
    NISAR_M2D_A_CURRENT_PLAN_SHA256, NISAR_M2D_A_LOCAL_EXECUTION_PLAN_SHA256,
    NISAR_M2D_B2_SEMANTIC_PLAN_AUTHORITY_SHA256, NISAR_M2D_B_EVIDENCE_SCHEMA, NISAR_M2D_B_GATE,
    NISAR_M2D_C2_EVIDENCE_SCHEMA, NISAR_M2D_C2_GATE,
};
pub use nisar_plan_authority::{
    derive_nisar_live_semantic_plan_sha256, semantic_plan_canonical_snapshot,
    NisarLivePlanSemanticAuthorityV1, NisarSemanticPlanAuthorityError,
    NisarSemanticPlanCanonicalSnapshotV1, NISAR_LIVE_PLAN_SEMANTIC_AUTHORITY_CONTRACT_V1,
};
#[cfg(feature = "nisar-subset-production")]
pub use nisar_planning_v2::{
    plan_and_freeze_nisar_subset_v2, plan_and_freeze_nisar_subset_v2_with_progress,
    NisarSubsetBootstrapBudgetV2, NisarSubsetBootstrapCategoryStatisticsV2,
    NisarSubsetBootstrapRangeV2, NisarSubsetBootstrapRevalidationV2,
    NisarSubsetBootstrapStatisticsV2, NisarSubsetMetadataBootstrapOutputV2,
    NisarSubsetMetadataBootstrapV2, NisarSubsetPlanningCompleteV2, NisarSubsetPlanningErrorV2,
    NisarSubsetVerifiedInventoryIdentityV2,
};
pub use nisar_ssar_hdf5_validation::NisarSsarRslcValidationContext;
#[cfg(feature = "offline-nisar-subset-test-support")]
#[doc(hidden)]
pub use nisar_subset_writer::tests::{
    ready_case_with_selection as prepare_offline_nisar_subset_acceptance_case,
    rebind_ready_case_request as rebind_offline_nisar_subset_acceptance_request,
    snapshot as offline_nisar_subset_acceptance_snapshot,
    writer as offline_nisar_subset_acceptance_writer,
    ReadyCase as OfflineNisarSubsetAcceptanceCase,
};
pub use nisar_subset_writer::{
    build_nisar_subset_source_dependency_graph_v2, NisarSubsetMetadataSelectionV2,
    NisarSubsetMetadataSnapshotV1, NisarSubsetSourceDependencyGraphBuildErrorV2,
    NisarSubsetVerifiedMetadataAttributeV2, NisarSubsetVerifiedMetadataDatasetKindV2,
    NisarSubsetVerifiedMetadataDatasetV2, NisarSubsetVerifiedMetadataEvidenceV2,
    NisarSubsetVerifiedMetadataGroupV2, NisarSubsetVerifiedMetadataInventoryV2,
    NisarSubsetVerifiedSelectedScienceV2, NisarSubsetWriteError, NisarSubsetWriteResultV1,
    OfflineNisarSubsetWriter,
};
#[cfg(feature = "nisar-subset-live-acceptance")]
#[doc(hidden)]
pub use production_worker::NisarSubsetV2BodyInterruptionHook;
pub use production_worker::{
    EarthdataSessionOwner, ProductionDownloadWorker, ProductionWorkerError,
};
pub use resolver_execution::{
    classify_frozen_execution, route_frozen_execution, FrozenExecution, ResolverExecutionError,
    ResolverExecutionRuntime,
};
pub use scheduler::{
    DemTaskLimits, DownloadScheduler, DownloadWork, DownloadWorker, LocalScanPhase,
    LocalScanVerdict, NisarExecutionPlanFreezeAuthority, NisarExecutionPlanFreezeError,
    NisarExecutionPlanReadyV2, NisarSourceSizeFreezeError, NisarSourceSizeReadyV2,
    OrbitRecheckClock, SharedTaskStore, SystemOrbitRecheckClock, WorkerControl,
};
pub use segment_planner::{
    accept_ranges_supports_bytes, parse_content_range, parse_entity_tag,
    partition_half_open_ranges, plan_segments, validate_segment_response, EntityTag,
    FrozenChecksumAlgorithm, FrozenChecksumEvidence, IdentityConflictReason, LastModified,
    MalformedEvidenceReason, MaximumSegmentPolicy, ParsedContentRange, RangeCapability,
    RemoteLength, RestartReason, SegmentLevel, SegmentPartitionError, SegmentPlan,
    SegmentPlanContext, SegmentPlanningDecision, SegmentPlanningInput, SegmentRange,
    SegmentResponseDecision, SegmentResponseEvidence, SegmentResponseStatus, SingleStreamDecision,
    SingleStreamReason, StableObjectIdentity, TerminalResponseReason,
};
pub use transfer_activity::{
    TaskTransferActivityRegistry, TaskTransferSample, TransferActivityError,
};
pub use transfer_admission::{
    AdmissionWaitReasonCounts, AdmittedTransferBackend, OpaqueHostLabel,
    TransferAdmissionController, TransferAdmissionPolicy, TransferAdmissionSnapshot,
};
pub use transfer_backend::{
    TransferBackend, TransferBackendKind, TransferBackendSelector, ValidatedResolvedTransferResult,
};
pub use transfer_engine_prefs::{
    active_default_ssd_staging_dir, active_transfer_engine_prefs,
    active_transfer_engine_prefs_override, clear_active_transfer_engine_prefs_override,
    default_ssd_staging_dir_for_app_data, load_and_activate_transfer_engine_prefs_in_dir,
    load_transfer_engine_prefs_in_dir, normalize_max_download_speed_kib_s,
    normalize_per_file_segments, save_and_activate_transfer_engine_prefs_in_dir,
    save_transfer_engine_prefs_in_dir, set_active_default_ssd_staging_dir,
    set_active_transfer_engine_prefs, transfer_engine_prefs_path, SmartConcurrencyInputs,
    SmartConcurrencyTuner, TransferEnginePrefs, DEFAULT_SSD_STAGING_SUBDIR,
    MAX_DOWNLOAD_SPEED_KIB_S_LIMIT, TRANSFER_ENGINE_PREFS_FILENAME,
};
pub use transfer_tuning::{
    global_bandwidth_limiter, global_host_concurrency_tracker, AcquireOutcome, BandwidthLimiter,
    HostConcurrencyTracker, RelativeSpeedConfig, DEFAULT_MIN_SPLIT_CHUNK_BYTES,
    DEFAULT_RESUME_OVERLAP_BYTES,
};
