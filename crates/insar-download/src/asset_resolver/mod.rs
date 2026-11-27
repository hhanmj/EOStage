pub(crate) mod bhoonidhi;
mod bhoonidhi_probe_rejection;
pub(crate) mod cdse;
mod compat;
mod context;
mod earth_search_sentinel2;
mod earthdata;
mod error;
mod local_file;
mod planetary_computer;
mod planetary_computer_reviewed;
mod public_https;
mod refresh;
mod registry;
mod shadow_compare;
mod shadow_plan;
mod shadow_runtime;
mod shadow_types;
mod types;
pub(crate) mod usgs_m2m;

pub use compat::{classify_asset_access, AssetAccessClassification, LegacyAdapter};
use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, FrozenAssetAccess, StableAssetReference,
};

pub use bhoonidhi::{
    try_acquire_bhoonidhi_download_slot, BhoonidhiAssetResolver, BhoonidhiDownloadPermit,
    BhoonidhiDownloadProbe, BhoonidhiDownloadProbeError, BhoonidhiDownloadProbeEvidence,
    ReqwestBhoonidhiDownloadProbe, BHOONIDHI_DOWNLOAD_ASSET_KEY, BHOONIDHI_DOWNLOAD_ORIGIN,
    BHOONIDHI_DOWNLOAD_PATH, BHOONIDHI_DOWNLOAD_PROBE_CONNECT_TIMEOUT,
    BHOONIDHI_DOWNLOAD_PROBE_MAX_RESPONSE_BYTES, BHOONIDHI_DOWNLOAD_PROBE_RANGE,
    BHOONIDHI_DOWNLOAD_PROBE_TOTAL_TIMEOUT, BHOONIDHI_DOWNLOAD_RESOLVER_ID, BHOONIDHI_DOWNLOAD_URL,
    BHOONIDHI_MAX_CONCURRENT_DOWNLOADS,
};
pub use cdse::CdseArchiveAssetResolver;
pub use context::{
    AuthRefreshBudget, AuthRefreshPermit, Clock, CredentialMaterial, CredentialProvider,
    CredentialProviderError, ResolveContext, SystemClock, TimeoutPolicy,
};
pub use earth_search_sentinel2::{
    EarthSearchSentinel2BandAssetResolver, EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID,
};
pub use earthdata::EarthdataAssetResolver;
pub use error::{ResolveError, ResolveErrorKind};
pub use insar_core::ProxyPolicy;
pub use local_file::LocalFileAssetResolver;
pub use planetary_computer::{
    is_valid_planetary_computer_hls_href, PlanetaryComputerLandsatAssetResolver,
    PLANETARY_COMPUTER_HLS_RESOLVER_ID, PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID,
};
pub use planetary_computer_reviewed::{
    PlanetaryComputerReviewedAssetResolver, PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID,
    PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID,
    PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID,
};
pub use public_https::PublicHttpsAssetResolver;
pub use refresh::{evaluate_refresh, RefreshDecision, RefreshEvaluation, RefreshReason};
pub use registry::AssetResolverRegistry;
pub use shadow_compare::{
    LegacyTransferIntent, ShadowDestinationPolicy, ShadowMismatchCode, ShadowProviderClass,
    TransferIntentFingerprint,
};
pub use shadow_runtime::{
    run_shadow_isolated, run_shadow_then_legacy, ResolverShadowDiagnostics, ResolverShadowInput,
    ResolverShadowObservation, ResolverShadowObserver, ResolverShadowRuntime, ShadowAssetClass,
    ShadowOutcome, SHADOW_DESCRIPTOR_UNAVAILABLE, SHADOW_ORBIT_DESCRIPTOR_UNAVAILABLE,
};
pub use shadow_types::{
    ShadowAccessIntent, ShadowAuthKind, ShadowCredentialShapeProvider, ShadowHostClass,
    ShadowNetworkAccess, ShadowPathClass, ShadowResolveContext,
};
pub(crate) use types::ResolvedHostPolicy;
pub(crate) use types::{asf_extractor_signed_object_diagnostic, ResolvedAuthKind};
pub use types::{RemoteValidator, ResolvedAsset, ResolvedChecksum, ResolvedMethod};
pub use usgs_m2m::LandsatM2mAssetResolver;

pub trait AssetResolver: Send + Sync {
    fn resolver_id(&self) -> &'static str;

    fn supported_schemes(&self) -> &'static [AssetAccessScheme];

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool;

    /// Resolver-specific validation of immutable access evidence before runtime
    /// credential lookup. Generic resolvers retain their existing behavior;
    /// providers with a closed archive identity can fail closed here.
    fn validate_frozen(&self, _frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        Ok(())
    }

    fn resolve(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError>;

    fn prepare_auth_refresh(&self, _frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        Ok(())
    }

    fn plan_shadow(
        &self,
        frozen: &FrozenAssetAccess,
        context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError>;
}
