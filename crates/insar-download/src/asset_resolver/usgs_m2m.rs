use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use insar_core::{
    stable_asset_key, AssetAccessDescriptor, AssetAccessScheme, AssetRole, FrozenAssetAccess,
    StableAssetReference,
};
use insar_provider::usgs_m2m::{
    is_supported_landsat_c2_l2_bundle_option, UsgsM2mDownloadClient, UsgsM2mDownloadRequestOutcome,
    UsgsM2mError, UsgsM2mErrorCode, UsgsM2mPreparingDownload, UsgsM2mResolvedDownload,
    UsgsM2mSession, LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM, LANDSAT_C2_L2_BUNDLE_OPTION_ID,
    LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE, LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
    USGS_M2M_SESSION_PROFILE_ID,
};
use reqwest::Url;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::types::ResolvedHostPolicy;
use super::{
    AssetResolver, ResolveContext, ResolveError, ResolvedAsset, ResolvedChecksum, ResolvedMethod,
    ShadowAccessIntent, ShadowResolveContext,
};

pub const USGS_LANDSAT_C2_L2_RESOLVER_ID_V1: &str = "usgs.m2m.landsat.c2.l2.full-bundle.v1";
pub const USGS_LANDSAT_C2_L2_RESOLVER_ID_V2: &str = "usgs.m2m.landsat.c2.l2.full-bundle.v2";
pub const USGS_LANDSAT_C2_L2_RESOLVER_ID: &str = USGS_LANDSAT_C2_L2_RESOLVER_ID_V2;
pub const USGS_LANDSAT_DOWNLOAD_HOST: &str = "landsatlook.usgs.gov";
/// Current M2M D806/dds_ms delivery host, recorded live 2026-10-31 for
/// LC09_L2SP_125033_20261013_20261022_02_T1: object URLs of the exact shape
/// https://dds.cr.usgs.gov/<segment>/<segment> with no query.
pub const USGS_LANDSAT_DDS_DOWNLOAD_HOST: &str = "dds.cr.usgs.gov";

const MAX_PREPARATION_ATTEMPTS: u32 = 90;
const MAX_PREPARATION_DURATION: Duration = Duration::from_secs(15 * 60);
const PREPARATION_POLL_INTERVAL: Duration = Duration::from_secs(10);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrozenBundleAvailability {
    Available,
    Preparable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrozenLandsatBundleLocator<'a> {
    pub dataset_alias: &'a str,
    pub entity_id: &'a str,
    pub option_id: &'a str,
    pub availability: FrozenBundleAvailability,
    pub display_id: &'a str,
    pub product_code: &'a str,
    pub download_system: &'a str,
}

pub(crate) fn parse_frozen_landsat_bundle_locator_v1(
    value: &str,
) -> Option<FrozenLandsatBundleLocator<'_>> {
    let parts = value.split(':').collect::<Vec<_>>();
    let ["urn", "usgs", "m2m", "landsat", dataset_alias, entity_id, "full_bundle", option_id, availability, display_id] =
        parts.as_slice()
    else {
        return None;
    };
    if *option_id != LANDSAT_C2_L2_BUNDLE_OPTION_ID {
        return None;
    }
    finish_frozen_landsat_bundle_locator(
        dataset_alias,
        entity_id,
        option_id,
        availability,
        display_id,
        LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE,
        LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM,
    )
}

pub(crate) fn parse_frozen_landsat_bundle_locator_v2(
    value: &str,
) -> Option<FrozenLandsatBundleLocator<'_>> {
    let parts = value.split(':').collect::<Vec<_>>();
    let ["urn", "usgs", "m2m", "landsat", dataset_alias, entity_id, "full_bundle", option_id, availability, display_id, product_code, download_system] =
        parts.as_slice()
    else {
        return None;
    };
    if !is_supported_landsat_c2_l2_bundle_option(
        option_id,
        LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
        product_code,
        download_system,
    ) {
        return None;
    }
    finish_frozen_landsat_bundle_locator(
        dataset_alias,
        entity_id,
        option_id,
        availability,
        display_id,
        product_code,
        download_system,
    )
}

fn finish_frozen_landsat_bundle_locator<'a>(
    dataset_alias: &'a str,
    entity_id: &'a str,
    option_id: &'a str,
    availability: &'a str,
    display_id: &'a str,
    product_code: &'a str,
    download_system: &'a str,
) -> Option<FrozenLandsatBundleLocator<'a>> {
    if !matches!(
        dataset_alias,
        "landsat_ot_c2_l2" | "landsat_tm_c2_l2" | "landsat_etm_c2_l2"
    ) || !valid_identifier(entity_id)
        || !valid_identifier(option_id)
        || !valid_identifier(display_id)
    {
        return None;
    }
    let availability = match availability {
        "available" => FrozenBundleAvailability::Available,
        "preparable" => FrozenBundleAvailability::Preparable,
        _ => return None,
    };
    Some(FrozenLandsatBundleLocator {
        dataset_alias,
        entity_id,
        option_id,
        availability,
        display_id,
        product_code,
        download_system,
    })
}

pub(crate) fn parse_frozen_landsat_bundle_locator_for_resolver<'a>(
    value: &'a str,
    resolver_id: &str,
) -> Option<FrozenLandsatBundleLocator<'a>> {
    match resolver_id {
        USGS_LANDSAT_C2_L2_RESOLVER_ID_V1 => parse_frozen_landsat_bundle_locator_v1(value),
        USGS_LANDSAT_C2_L2_RESOLVER_ID_V2 => parse_frozen_landsat_bundle_locator_v2(value),
        _ => None,
    }
}

struct BundleResolved {
    entity_id: String,
    display_id: Option<String>,
    runtime_url: Zeroizing<String>,
    expected_size_bytes: Option<u64>,
    checksum: Option<(String, String)>,
}

impl BundleResolved {
    fn from_provider(value: UsgsM2mResolvedDownload) -> Self {
        Self {
            entity_id: value.entity_id().to_owned(),
            display_id: value.display_id().map(str::to_owned),
            runtime_url: Zeroizing::new(value.runtime_url().to_owned()),
            expected_size_bytes: value.expected_size_bytes(),
            checksum: value
                .checksum()
                .map(|(algorithm, checksum)| (algorithm.to_owned(), checksum.to_owned())),
        }
    }
}

impl fmt::Debug for BundleResolved {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BundleResolved")
            .field("entity_id", &self.entity_id)
            .field("display_id", &self.display_id)
            .field("runtime_url", &"[REDACTED]")
            .field("expected_size_bytes", &self.expected_size_bytes)
            .field("checksum", &self.checksum.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

enum PreparingToken {
    Provider(UsgsM2mPreparingDownload),
    #[cfg(test)]
    Fixture,
}

enum BundleRequestOutcome {
    Available(BundleResolved),
    Preparing(PreparingToken),
}

trait BundleResolutionPort: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn request(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        option_id: &str,
        label: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<BundleRequestOutcome, ResolveError>;

    fn retrieve(
        &self,
        label: &str,
        preparing: &PreparingToken,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<BundleResolved>, ResolveError>;
}

struct ProviderBundleResolutionPort {
    client: UsgsM2mDownloadClient,
}

impl BundleResolutionPort for ProviderBundleResolutionPort {
    fn request(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        option_id: &str,
        label: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<BundleRequestOutcome, ResolveError> {
        self.client
            .request_full_bundle(
                dataset_alias,
                entity_id,
                display_id,
                option_id,
                label,
                cancelled,
            )
            .map(|outcome| match outcome {
                UsgsM2mDownloadRequestOutcome::Available(value) => {
                    BundleRequestOutcome::Available(BundleResolved::from_provider(value))
                }
                UsgsM2mDownloadRequestOutcome::Preparing(value) => {
                    BundleRequestOutcome::Preparing(PreparingToken::Provider(value))
                }
            })
            .map_err(map_provider_error)
    }

    fn retrieve(
        &self,
        label: &str,
        preparing: &PreparingToken,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<BundleResolved>, ResolveError> {
        match preparing {
            PreparingToken::Provider(preparing) => self
                .client
                .retrieve_full_bundle(label, preparing, cancelled)
                .map(|value| value.map(BundleResolved::from_provider))
                .map_err(map_provider_error),
            #[cfg(test)]
            PreparingToken::Fixture => Err(ResolveError::descriptor_invalid()),
        }
    }
}

#[derive(Clone, Copy)]
struct PreparationPolicy {
    max_attempts: u32,
    max_duration: Duration,
    poll_interval: Duration,
}

impl PreparationPolicy {
    const PRODUCTION: Self = Self {
        max_attempts: MAX_PREPARATION_ATTEMPTS,
        max_duration: MAX_PREPARATION_DURATION,
        poll_interval: PREPARATION_POLL_INTERVAL,
    };

    fn valid(self) -> bool {
        self.max_attempts > 0 && !self.max_duration.is_zero()
    }
}

pub struct LandsatM2mAssetResolver {
    port: Arc<dyn BundleResolutionPort>,
    preparation: PreparationPolicy,
    resolver_id: &'static str,
}

impl LandsatM2mAssetResolver {
    pub fn new(session: Arc<UsgsM2mSession>) -> Self {
        Self::with_resolver_id(session, USGS_LANDSAT_C2_L2_RESOLVER_ID_V2)
    }

    pub fn legacy(session: Arc<UsgsM2mSession>) -> Self {
        Self::with_resolver_id(session, USGS_LANDSAT_C2_L2_RESOLVER_ID_V1)
    }

    fn with_resolver_id(session: Arc<UsgsM2mSession>, resolver_id: &'static str) -> Self {
        Self {
            port: Arc::new(ProviderBundleResolutionPort {
                client: UsgsM2mDownloadClient::new(session),
            }),
            preparation: PreparationPolicy::PRODUCTION,
            resolver_id,
        }
    }

    fn parse_locator<'a>(&self, value: &'a str) -> Option<FrozenLandsatBundleLocator<'a>> {
        parse_frozen_landsat_bundle_locator_for_resolver(value, self.resolver_id)
    }

    fn resolve_preparing(
        &self,
        label: &str,
        preparing: &PreparingToken,
        context: &ResolveContext<'_>,
    ) -> Result<BundleResolved, ResolveError> {
        if !self.preparation.valid() {
            return Err(ResolveError::descriptor_invalid());
        }
        let started = Instant::now();
        for _ in 0..self.preparation.max_attempts {
            context.check_cancelled()?;
            let remaining = self
                .preparation
                .max_duration
                .saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(ResolveError::timeout());
            }
            wait_for_poll(context, self.preparation.poll_interval.min(remaining))?;
            context.check_cancelled()?;
            if started.elapsed() >= self.preparation.max_duration {
                return Err(ResolveError::timeout());
            }
            let resolved = self
                .port
                .retrieve(label, preparing, &|| context.check_cancelled().is_err())?;
            if started.elapsed() >= self.preparation.max_duration {
                return Err(ResolveError::timeout());
            }
            if let Some(resolved) = resolved {
                return Ok(resolved);
            }
        }
        Err(ResolveError::timeout())
    }
}

impl AssetResolver for LandsatM2mAssetResolver {
    fn resolver_id(&self) -> &'static str {
        self.resolver_id
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::UsgsM2m]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.resolver_id == self.resolver_id
            && descriptor.scheme == AssetAccessScheme::UsgsM2m
            && descriptor.requires_credentials
            && descriptor.credential_profile.as_deref() == Some(USGS_M2M_SESSION_PROFILE_ID)
            && descriptor
                .stable_href
                .as_deref()
                .and_then(|value| self.parse_locator(value))
                .is_some()
    }

    fn validate_frozen(&self, frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        let locator = frozen
            .descriptor
            .stable_href
            .as_deref()
            .and_then(|value| self.parse_locator(value))
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let reference = &frozen.reference;
        let expected_key = stable_asset_key(&reference.provider_product_key, locator.option_id)
            .map_err(|_| ResolveError::reference_invalid())?;
        let file_name = frozen
            .file_name
            .as_deref()
            .filter(|value| safe_bundle_filename(value))
            .ok_or_else(ResolveError::descriptor_invalid)?;
        if reference.provider_id != "usgs.eros.m2m"
            || reference.collection_id != locator.dataset_alias
            || reference.provider_product_id != locator.entity_id
            || reference.asset_key != locator.option_id
            || reference.stable_asset_key != expected_key
            || frozen.role != AssetRole::Primary
            || display_id_from_filename(file_name) != Some(locator.display_id)
            || !valid_integrity(
                frozen.expected_size_bytes,
                frozen.checksum_algorithm.as_deref(),
                frozen.checksum_value.as_deref(),
            )
        {
            return Err(ResolveError::descriptor_invalid());
        }
        Ok(())
    }

    fn resolve(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        context.check_cancelled()?;
        if !self.can_resolve(descriptor) {
            return Err(ResolveError::descriptor_invalid());
        }
        let locator = descriptor
            .stable_href
            .as_deref()
            .and_then(|value| self.parse_locator(value))
            .ok_or_else(ResolveError::descriptor_invalid)?;
        if reference.provider_id != "usgs.eros.m2m"
            || reference.collection_id != locator.dataset_alias
            || reference.provider_product_id != locator.entity_id
            || reference.asset_key != locator.option_id
        {
            return Err(ResolveError::reference_invalid());
        }
        let display_id = locator.display_id;
        let label = request_label(&reference.stable_asset_key);
        let resolved = match self.port.request(
            locator.dataset_alias,
            locator.entity_id,
            display_id,
            locator.option_id,
            &label,
            &|| context.check_cancelled().is_err(),
        )? {
            BundleRequestOutcome::Available(value) => value,
            BundleRequestOutcome::Preparing(preparing) => {
                self.resolve_preparing(&label, &preparing, context)?
            }
        };
        context.check_cancelled()?;
        if resolved.entity_id != locator.entity_id
            || resolved
                .display_id
                .as_deref()
                .is_some_and(|value| value != display_id)
            || !approved_runtime_url(&resolved.runtime_url)
        {
            // E2: the sanitized stage/host shape goes only to the opt-in
            // stderr diagnostic — never into the persisted reason_zh, whose
            // user-facing message must stay plain Chinese. It names which
            // sub-check failed and the URL host shape; never the entity,
            // label or the full signed URL.
            write_resolve_rejection_diagnostic(&format!(
                "stage=runtime_identity_or_url_rejected entity_match={} display_match={} url_host={}",
                resolved.entity_id == locator.entity_id,
                resolved
                    .display_id
                    .as_deref()
                    .is_none_or(|value| value == display_id),
                url_host_shape(&resolved.runtime_url),
            ));
            return Err(ResolveError::failed());
        }
        let checksum = resolved
            .checksum
            .map(|(algorithm, value)| ResolvedChecksum::new(algorithm, value))
            .transpose()?;
        let mut asset = ResolvedAsset::new(resolved.runtime_url.as_str(), ResolvedMethod::Get)?
            .with_host_policy(ResolvedHostPolicy::UsgsLandsatDownloadV1)
            .with_expected_size(resolved.expected_size_bytes)
            .with_checksum(checksum)
            .with_suggested_filename(Some(format!("{display_id}.tar")))
            .with_asset_role(AssetRole::Primary);
        if self.resolver_id == USGS_LANDSAT_C2_L2_RESOLVER_ID_V2 {
            // D806 download-options filesize is advisory. The reviewed runtime
            // response's Content-Length/Content-Range is the payload authority.
            asset = asset.with_remote_response_size_authority();
        }
        Ok(asset)
    }

    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::not_configured())
    }
}

fn display_id_from_filename(value: &str) -> Option<&str> {
    let value = value.strip_suffix(".tar")?;
    valid_identifier(value).then_some(value)
}

fn safe_bundle_filename(value: &str) -> bool {
    value.len() <= 128
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains('\0')
        && display_id_from_filename(value).is_some()
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value == value.trim()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn valid_integrity(size: Option<u64>, algorithm: Option<&str>, value: Option<&str>) -> bool {
    if size.is_some_and(|value| value == 0) {
        return false;
    }
    match (algorithm, value) {
        (Some(algorithm), Some(value)) => crate::ExpectedChecksum::new(algorithm, value).is_ok(),
        (None, None) => size.is_some(),
        _ => false,
    }
}

fn approved_runtime_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .is_some_and(|url| approved_runtime_url_parsed(&url))
}

/// Host (and path-segment count only) of a runtime URL, fit for a persisted
/// diagnostic. The path, query and any signed material are never included.
fn url_host_shape(value: &str) -> String {
    Url::parse(value)
        .ok()
        .map(|url| {
            let segments = url
                .path_segments()
                .map(|segments| segments.filter(|segment| !segment.is_empty()).count())
                .unwrap_or(0);
            format!(
                "{}://{} segments={} query={}",
                url.scheme(),
                url.host_str().unwrap_or("?"),
                segments,
                url.query().is_some()
            )
        })
        .unwrap_or_else(|| "unparsable".to_owned())
}

/// Opt-in resolve-rejection diagnosis under the shared
/// `INSAR_DIAGNOSTIC_USGS_M2M` gate. Emitted to stderr only: the persisted
/// user-facing reason stays plain Chinese, while the sanitized stage/host
/// shape remains available for live diagnosis.
fn write_resolve_rejection_diagnostic(detail: &str) {
    if std::env::var("INSAR_DIAGNOSTIC_USGS_M2M")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
    {
        eprintln!("INSAR_USGS_M2M_RESOLVE_DIAGNOSTIC {{\"event\":\"usgs_m2m_resolve_rejected\",\"{detail}\"}}");
    }
}

pub(crate) fn approved_runtime_url_parsed(url: &Url) -> bool {
    if !approved_download_response_url_parsed(url) {
        return false;
    }
    let path = url.path().to_ascii_lowercase();
    let segment_count = url
        .path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()).count())
        .unwrap_or(0);
    let legacy_data_url = path.starts_with("/data/") && segment_count >= 2;
    let current_broker_url = segment_count == 1 && url.query().is_some();
    legacy_data_url || current_broker_url || approved_dds_runtime_url_parsed(url, segment_count)
}

/// The reviewed dds.cr.usgs.gov object shape: exactly two non-empty path
/// segments, https, default port, no userinfo, no fragment, no query.
fn approved_dds_runtime_url_parsed(url: &Url, segment_count: usize) -> bool {
    url.host_str() == Some(USGS_LANDSAT_DDS_DOWNLOAD_HOST)
        && segment_count == 2
        && url.query().is_none()
}

pub(crate) fn approved_download_response_url_parsed(url: &Url) -> bool {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(
            url.host_str(),
            Some(USGS_LANDSAT_DOWNLOAD_HOST) | Some(USGS_LANDSAT_DDS_DOWNLOAD_HOST)
        )
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
        || url.path().len() > 2048
        || url.as_str().len() > 16 * 1024
    {
        return false;
    }
    let path = url.path().to_ascii_lowercase();
    if path.contains("/../")
        || path.ends_with("/..")
        || path.contains("/%2e%2e/")
        || path.ends_with("/%2e%2e")
    {
        return false;
    }
    url.path_segments()
        .is_some_and(|mut segments| segments.find(|segment| !segment.is_empty()).is_some())
}

fn request_label(stable_asset_key: &str) -> String {
    let digest = Sha256::digest(stable_asset_key.as_bytes());
    let mut label = String::from("insar_landsat_");
    for byte in &digest[..16] {
        use std::fmt::Write as _;
        let _ = write!(label, "{byte:02x}");
    }
    label
}

fn wait_for_poll(context: &ResolveContext<'_>, duration: Duration) -> Result<(), ResolveError> {
    let mut remaining = duration;
    while !remaining.is_zero() {
        context.check_cancelled()?;
        let step = remaining.min(CANCELLATION_POLL_INTERVAL);
        std::thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
    Ok(())
}

fn map_provider_error(error: UsgsM2mError) -> ResolveError {
    let diagnostic = error.diagnostic_message();
    let mapped = match error.code() {
        UsgsM2mErrorCode::AuthRequired => ResolveError::credentials_missing(),
        UsgsM2mErrorCode::AuthInvalid
        | UsgsM2mErrorCode::AuthUnauthorized
        | UsgsM2mErrorCode::AuthExpired => ResolveError::auth_failed(),
        UsgsM2mErrorCode::Timeout => ResolveError::timeout(),
        UsgsM2mErrorCode::Cancelled => ResolveError::cancelled(),
        UsgsM2mErrorCode::ConfigInvalid
        | UsgsM2mErrorCode::DownloadOptionInvalid
        | UsgsM2mErrorCode::DownloadIdentityConflict
        | UsgsM2mErrorCode::DownloadIntegrityMissing
        | UsgsM2mErrorCode::DownloadPreparationInvalid => ResolveError::descriptor_invalid(),
        _ => ResolveError::failed(),
    };
    mapped.with_diagnostic(diagnostic)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Mutex;
    use std::time::SystemTime;

    use insar_core::{AssetResolverParameters, ProxyPolicy};

    use super::*;
    use crate::{
        AssetResolverRegistry, Clock, CredentialMaterial, CredentialProvider,
        CredentialProviderError,
    };

    const ENTITY_ID: &str = "LC91230452026001LGN00";
    const DISPLAY_ID: &str = "LC09_L2SP_123045_20260101_20260103_02_T1";

    struct NoCredentials;

    impl CredentialProvider for NoCredentials {
        fn resolve(&self, _profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
            Err(CredentialProviderError::Missing)
        }
    }

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH + Duration::from_secs(1)
        }
    }

    struct FixturePort {
        request: Mutex<Option<BundleRequestOutcome>>,
        retrieves: Mutex<VecDeque<Option<BundleResolved>>>,
        retrieve_calls: AtomicU32,
    }

    impl FixturePort {
        fn immediate(resolved: BundleResolved) -> Self {
            Self {
                request: Mutex::new(Some(BundleRequestOutcome::Available(resolved))),
                retrieves: Mutex::new(VecDeque::new()),
                retrieve_calls: AtomicU32::new(0),
            }
        }

        fn preparing(retrieves: Vec<Option<BundleResolved>>) -> Self {
            Self {
                request: Mutex::new(Some(BundleRequestOutcome::Preparing(
                    PreparingToken::Fixture,
                ))),
                retrieves: Mutex::new(retrieves.into()),
                retrieve_calls: AtomicU32::new(0),
            }
        }
    }

    impl BundleResolutionPort for FixturePort {
        fn request(
            &self,
            _dataset_alias: &str,
            _entity_id: &str,
            _display_id: &str,
            _option_id: &str,
            _label: &str,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<BundleRequestOutcome, ResolveError> {
            self.request
                .lock()
                .map_err(|_| ResolveError::failed())?
                .take()
                .ok_or_else(ResolveError::failed)
        }

        fn retrieve(
            &self,
            _label: &str,
            preparing: &PreparingToken,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<Option<BundleResolved>, ResolveError> {
            if !matches!(preparing, PreparingToken::Fixture) {
                return Err(ResolveError::descriptor_invalid());
            }
            self.retrieve_calls.fetch_add(1, Ordering::Relaxed);
            self.retrieves
                .lock()
                .map_err(|_| ResolveError::failed())?
                .pop_front()
                .ok_or_else(ResolveError::timeout)
        }
    }

    fn resolved(url: &str) -> BundleResolved {
        BundleResolved {
            entity_id: ENTITY_ID.to_owned(),
            display_id: Some(DISPLAY_ID.to_owned()),
            runtime_url: Zeroizing::new(url.to_owned()),
            expected_size_bytes: Some(1024),
            checksum: Some(("sha256".to_owned(), "a".repeat(64))),
        }
    }

    fn frozen() -> FrozenAssetAccess {
        let product_key =
            insar_core::provider_product_key("usgs.eros.m2m", "landsat_ot_c2_l2", ENTITY_ID)
                .unwrap();
        let asset_key = LANDSAT_C2_L2_BUNDLE_OPTION_ID.to_owned();
        FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: "usgs.eros.m2m".to_owned(),
                collection_id: "landsat_ot_c2_l2".to_owned(),
                provider_product_id: ENTITY_ID.to_owned(),
                provider_product_key: product_key.clone(),
                canonical_product_key: None,
                asset_key: asset_key.clone(),
                stable_asset_key: stable_asset_key(&product_key, &asset_key).unwrap(),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: USGS_LANDSAT_C2_L2_RESOLVER_ID_V1.to_owned(),
                scheme: AssetAccessScheme::UsgsM2m,
                stable_href: Some(format!(
                    "urn:usgs:m2m:landsat:landsat_ot_c2_l2:{ENTITY_ID}:full_bundle:{LANDSAT_C2_L2_BUNDLE_OPTION_ID}:available:{DISPLAY_ID}"
                )),
                requires_credentials: true,
                credential_profile: Some(USGS_M2M_SESSION_PROFILE_ID.to_owned()),
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some(format!("{DISPLAY_ID}.tar")),
            expected_size_bytes: Some(1024),
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: Some("a".repeat(64)),
        }
    }

    fn frozen_v2() -> FrozenAssetAccess {
        let mut frozen = frozen();
        let option_id = "632210d4770592cf".to_owned();
        frozen.reference.asset_key = option_id.clone();
        frozen.reference.stable_asset_key =
            stable_asset_key(&frozen.reference.provider_product_key, &option_id).unwrap();
        frozen.descriptor.resolver_id = USGS_LANDSAT_C2_L2_RESOLVER_ID_V2.to_owned();
        frozen.descriptor.stable_href = Some(format!(
            "urn:usgs:m2m:landsat:landsat_ot_c2_l2:{ENTITY_ID}:full_bundle:{option_id}:available:{DISPLAY_ID}:D806:dds_ms"
        ));
        frozen
    }

    fn fixture_resolver(port: Arc<dyn BundleResolutionPort>) -> LandsatM2mAssetResolver {
        LandsatM2mAssetResolver {
            port,
            preparation: PreparationPolicy {
                max_attempts: 3,
                max_duration: Duration::from_secs(1),
                poll_interval: Duration::ZERO,
            },
            resolver_id: USGS_LANDSAT_C2_L2_RESOLVER_ID_V1,
        }
    }

    fn fixture_resolver_v2(port: Arc<dyn BundleResolutionPort>) -> LandsatM2mAssetResolver {
        LandsatM2mAssetResolver {
            port,
            preparation: PreparationPolicy {
                max_attempts: 3,
                max_duration: Duration::from_secs(1),
                poll_interval: Duration::ZERO,
            },
            resolver_id: USGS_LANDSAT_C2_L2_RESOLVER_ID_V2,
        }
    }

    #[test]
    fn frozen_locator_accepts_legacy_and_current_bundle_semantics_only() {
        let legacy = format!(
            "urn:usgs:m2m:landsat:landsat_ot_c2_l2:{ENTITY_ID}:full_bundle:{LANDSAT_C2_L2_BUNDLE_OPTION_ID}:available:{DISPLAY_ID}"
        );
        let parsed = parse_frozen_landsat_bundle_locator_v1(&legacy).expect("legacy locator");
        assert_eq!(parsed.product_code, LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE);
        assert_eq!(parsed.download_system, LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM);

        let current = format!(
            "urn:usgs:m2m:landsat:landsat_ot_c2_l2:{ENTITY_ID}:full_bundle:632210d4770592cf:available:{DISPLAY_ID}:D806:dds_ms"
        );
        let parsed = parse_frozen_landsat_bundle_locator_v2(&current).expect("current locator");
        assert_eq!(parsed.option_id, "632210d4770592cf");
        assert_eq!(parsed.product_code, "D806");
        assert_eq!(parsed.download_system, "dds_ms");

        assert!(parse_frozen_landsat_bundle_locator_v2(&format!(
            "urn:usgs:m2m:landsat:landsat_ot_c2_l2:{ENTITY_ID}:full_bundle:wrong-option:available:{DISPLAY_ID}:D806:dds_ms"
        ))
        .is_none());

        assert!(parse_frozen_landsat_bundle_locator_v2(&format!(
            "urn:usgs:m2m:landsat:landsat_ot_c2_l2:{ENTITY_ID}:full_bundle:opaque:available:{DISPLAY_ID}:D806:folder"
        ))
        .is_none());
        assert!(parse_frozen_landsat_bundle_locator_v2(&format!(
            "urn:usgs:m2m:landsat:landsat_ot_c2_l2:{ENTITY_ID}:full_bundle:632210d4770592cf:available:{DISPLAY_ID}"
        ))
        .is_none());
    }

    #[test]
    fn immediate_resolution_accepts_only_the_exact_download_host_and_redacts_the_url() {
        let frozen = frozen();
        let resolver = fixture_resolver(Arc::new(FixturePort::immediate(resolved(
            "https://landsatlook.usgs.gov/data/LC09_bundle.tar?signature=runtime-only",
        ))));
        let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock)
            .with_proxy_policy(ProxyPolicy::System);
        resolver.validate_frozen(&frozen).unwrap();
        let asset = resolver
            .resolve(&frozen.reference, &frozen.descriptor, &context)
            .unwrap();
        assert_eq!(asset.expected_size(), Some(1024));
        assert_eq!(
            asset.host_policy(),
            Some(ResolvedHostPolicy::UsgsLandsatDownloadV1)
        );
        let debug = format!("{asset:?}");
        assert!(!debug.contains("signature=runtime-only"));
        assert!(!debug.contains("LC09_bundle.tar"));
    }

    #[test]
    fn v2_uses_remote_payload_size_authority_while_v1_remains_strict() {
        let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);

        let legacy_frozen = frozen();
        let legacy = fixture_resolver(Arc::new(FixturePort::immediate(resolved(
            "https://landsatlook.usgs.gov/data/LC09_bundle.tar",
        ))))
        .resolve(
            &legacy_frozen.reference,
            &legacy_frozen.descriptor,
            &context,
        )
        .unwrap();
        assert!(!legacy.remote_response_size_is_authoritative());

        let current_frozen = frozen_v2();
        let current = fixture_resolver_v2(Arc::new(FixturePort::immediate(resolved(
            "https://landsatlook.usgs.gov/data/LC09_bundle.tar",
        ))))
        .resolve(
            &current_frozen.reference,
            &current_frozen.descriptor,
            &context,
        )
        .unwrap();
        assert!(current.remote_response_size_is_authoritative());
    }

    #[test]
    fn current_landsatlook_broker_shape_is_accepted_without_relaxing_origin() {
        let frozen = frozen();
        let resolver = fixture_resolver(Arc::new(FixturePort::immediate(resolved(
            "https://landsatlook.usgs.gov/broker?download=opaque-runtime-value",
        ))));
        let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);
        let asset = resolver
            .resolve(&frozen.reference, &frozen.descriptor, &context)
            .unwrap();
        assert_eq!(
            asset.host_policy(),
            Some(ResolvedHostPolicy::UsgsLandsatDownloadV1)
        );
        let debug = format!("{asset:?}");
        assert!(!debug.contains("opaque-runtime-value"));
        assert!(!debug.contains("download="));
    }

    #[test]
    fn same_landsatlook_origin_redirect_shape_is_allowed_but_never_cross_origin() {
        for allowed in [
            "https://landsatlook.usgs.gov/download/current-object",
            "https://landsatlook.usgs.gov/one-segment",
            "https://landsatlook.usgs.gov/data/path/object.tar",
        ] {
            assert!(approved_download_response_url_parsed(
                &Url::parse(allowed).unwrap()
            ));
        }
        for rejected in [
            "https://example.com/download/current-object",
            "http://landsatlook.usgs.gov/download/current-object",
            "https://user@landsatlook.usgs.gov/download/current-object",
            "https://landsatlook.usgs.gov/",
        ] {
            assert!(!approved_download_response_url_parsed(
                &Url::parse(rejected).unwrap()
            ));
        }
    }

    /// Recorded live 2026-10-31 for LC09_L2SP_125033_20261013_20261022_02_T1:
    /// the current D806/dds_ms delivery hands out object URLs of the exact
    /// shape https://dds.cr.usgs.gov/<segment>/<segment> with no query. The
    /// runtime allowlist accepts exactly that shape; every deviation (query,
    /// wrong segment count, other dds-like host, traversal) fails closed.
    #[test]
    fn recorded_dds_object_shape_is_the_only_accepted_dds_url() {
        // The exact recorded shape resolves.
        let frozen = frozen();
        let resolver = fixture_resolver(Arc::new(FixturePort::immediate(resolved(
            "https://dds.cr.usgs.gov/e0821/LC09_L2SP_125033_20261013_20261022_02_T1.tar",
        ))));
        let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);
        let asset = resolver
            .resolve(&frozen.reference, &frozen.descriptor, &context)
            .expect("the recorded dds.cr.usgs.gov object shape is approved");
        assert_eq!(
            asset.host_policy(),
            Some(ResolvedHostPolicy::UsgsLandsatDownloadV1)
        );

        for allowed in [
            "https://dds.cr.usgs.gov/e0821/bundle.tar",
            "https://dds.cr.usgs.gov/segment-1/segment-2",
        ] {
            assert!(
                approved_runtime_url_parsed(&Url::parse(allowed).unwrap()),
                "{allowed} must be approved"
            );
        }
        for rejected in [
            // Query was absent in the recorded answer; a signed query is not
            // part of the reviewed dds shape.
            "https://dds.cr.usgs.gov/e0821/bundle.tar?sig=opaque",
            // Exactly two non-empty segments, not one and not three.
            "https://dds.cr.usgs.gov/only-one",
            "https://dds.cr.usgs.gov/one/two/three",
            "https://dds.cr.usgs.gov/",
            // The host must be exact; look-alikes fail closed.
            "https://dds.cr.usgs.gov.evil.example/e0821/bundle.tar",
            "http://dds.cr.usgs.gov/e0821/bundle.tar",
            "https://user@dds.cr.usgs.gov/e0821/bundle.tar",
            "https://dds.cr.usgs.gov/e0821/bundle.tar#fragment",
            "https://dds.cr.usgs.gov/e0821/%2e%2e/secret.tar",
        ] {
            assert!(
                !approved_runtime_url_parsed(&Url::parse(rejected).unwrap()),
                "{rejected} must be rejected"
            );
        }
    }

    #[test]
    fn wrong_scheme_host_port_userinfo_fragment_and_path_fail_before_transfer() {
        for url in [
            "http://landsatlook.usgs.gov/data/bundle.tar",
            "https://landsatlook.usgs.gov.evil.example/data/bundle.tar",
            "https://landsatlook.usgs.gov:444/data/bundle.tar",
            "https://user@landsatlook.usgs.gov/data/bundle.tar",
            "https://landsatlook.usgs.gov/data/bundle.tar#fragment",
            "https://landsatlook.usgs.gov/other/bundle.tar",
            "https://landsatlook.usgs.gov/broker",
            "https://landsatlook.usgs.gov/data/%2e%2e/secret.tar",
        ] {
            let frozen = frozen();
            let resolver = fixture_resolver(Arc::new(FixturePort::immediate(resolved(url))));
            let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);
            assert!(resolver
                .resolve(&frozen.reference, &frozen.descriptor, &context)
                .is_err());
        }
    }

    /// E2: the runtime-identity/URL rejection keeps its sanitized stage/host
    /// detail on the opt-in stderr diagnostic only — the persisted
    /// user-facing reason stays plain Chinese (the generic DL005 message),
    /// with no URL shape, identifiers or signed material in the error itself.
    #[test]
    fn url_rejection_keeps_the_persisted_reason_plain_chinese() {
        let frozen = frozen();
        // A dds.cr.usgs.gov URL with a signed query is not the reviewed shape;
        // the resolve fails without leaking the URL into the error.
        let rejected = resolved("https://dds.cr.usgs.gov/e0821/bundle.tar?sig=opaque");
        let resolver = fixture_resolver(Arc::new(FixturePort::immediate(rejected)));
        let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);
        let error = resolver
            .resolve(&frozen.reference, &frozen.descriptor, &context)
            .unwrap_err();
        assert_eq!(error.kind(), crate::ResolveErrorKind::Failed);
        // No diagnostic on the error: the DL005 reason_zh shown to the user is
        // the plain-Chinese generic message, never the stage detail.
        assert_eq!(error.diagnostic(), None);
    }

    #[test]
    fn preparation_polling_is_bounded_detects_non_progress_and_honors_cancellation() {
        let frozen = frozen();
        let completed_port = Arc::new(FixturePort::preparing(vec![
            None,
            Some(resolved(
                "https://landsatlook.usgs.gov/data/LC09_bundle.tar",
            )),
        ]));
        let resolver = fixture_resolver(completed_port.clone());
        let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);
        resolver
            .resolve(&frozen.reference, &frozen.descriptor, &context)
            .unwrap();
        assert_eq!(completed_port.retrieve_calls.load(Ordering::Relaxed), 2);

        let stalled_port = Arc::new(FixturePort::preparing(vec![None, None, None]));
        let resolver = fixture_resolver(stalled_port.clone());
        let error = resolver
            .resolve(&frozen.reference, &frozen.descriptor, &context)
            .unwrap_err();
        assert_eq!(error.kind(), crate::ResolveErrorKind::Timeout);
        assert_eq!(stalled_port.retrieve_calls.load(Ordering::Relaxed), 3);

        let cancelled = || true;
        let resolver = fixture_resolver(Arc::new(FixturePort::preparing(vec![None])));
        let error = resolver
            .resolve(
                &frozen.reference,
                &frozen.descriptor,
                &ResolveContext::new(&NoCredentials, &cancelled, &FixedClock),
            )
            .unwrap_err();
        assert_eq!(error.kind(), crate::ResolveErrorKind::Cancelled);
    }

    #[test]
    fn polling_wait_checks_cancellation_without_waiting_for_the_remote_interval() {
        let frozen = frozen();
        let resolver = LandsatM2mAssetResolver {
            port: Arc::new(FixturePort::preparing(vec![None])),
            preparation: PreparationPolicy {
                max_attempts: 1,
                max_duration: Duration::from_secs(5),
                poll_interval: Duration::from_secs(5),
            },
            resolver_id: USGS_LANDSAT_C2_L2_RESOLVER_ID_V1,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancelled);
        let trigger = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            signal.store(true, Ordering::Release);
        });
        let cancelled_probe = || cancelled.load(Ordering::Acquire);
        let started = Instant::now();
        let error = resolver
            .resolve(
                &frozen.reference,
                &frozen.descriptor,
                &ResolveContext::new(&NoCredentials, &cancelled_probe, &FixedClock),
            )
            .unwrap_err();
        trigger.join().unwrap();
        assert_eq!(error.kind(), crate::ResolveErrorKind::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn preparation_deadline_stops_before_a_poll_that_would_cross_the_wall_clock_budget() {
        let frozen = frozen();
        let port = Arc::new(FixturePort::preparing(vec![Some(resolved(
            "https://landsatlook.usgs.gov/data/LC09_bundle.tar",
        ))]));
        let resolver = LandsatM2mAssetResolver {
            port: port.clone(),
            preparation: PreparationPolicy {
                max_attempts: 3,
                max_duration: Duration::from_millis(20),
                poll_interval: Duration::from_secs(1),
            },
            resolver_id: USGS_LANDSAT_C2_L2_RESOLVER_ID_V1,
        };
        let started = Instant::now();
        let error = resolver
            .resolve(
                &frozen.reference,
                &frozen.descriptor,
                &ResolveContext::new(&NoCredentials, &|| false, &FixedClock),
            )
            .unwrap_err();
        assert_eq!(error.kind(), crate::ResolveErrorKind::Timeout);
        assert_eq!(port.retrieve_calls.load(Ordering::Relaxed), 0);
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn frozen_and_runtime_identity_or_integrity_conflicts_fail_closed() {
        let resolver = fixture_resolver(Arc::new(FixturePort::immediate(resolved(
            "https://landsatlook.usgs.gov/data/LC09_bundle.tar",
        ))));
        let mut wrong_frozen = frozen();
        wrong_frozen.reference.provider_id = "other.provider".to_owned();
        assert_eq!(
            resolver.validate_frozen(&wrong_frozen).unwrap_err().kind(),
            crate::ResolveErrorKind::DescriptorInvalid
        );

        let mut wrong_frozen = frozen();
        wrong_frozen.file_name = Some("../unsafe.tar".to_owned());
        assert_eq!(
            resolver.validate_frozen(&wrong_frozen).unwrap_err().kind(),
            crate::ResolveErrorKind::DescriptorInvalid
        );

        let mut runtime = resolved("https://landsatlook.usgs.gov/data/LC09_bundle.tar");
        runtime.expected_size_bytes = Some(2048);
        let mut registry = AssetResolverRegistry::new();
        registry
            .register(Arc::new(fixture_resolver(Arc::new(
                FixturePort::immediate(runtime),
            ))))
            .unwrap();
        let error = registry
            .resolve_frozen(
                &frozen(),
                &ResolveContext::new(&NoCredentials, &|| false, &FixedClock),
            )
            .unwrap_err();
        assert_eq!(error.kind(), crate::ResolveErrorKind::Failed);

        let mut runtime = resolved("https://landsatlook.usgs.gov/data/LC09_bundle.tar");
        runtime.entity_id = "LC80000000000000000000".to_owned();
        let resolver = fixture_resolver(Arc::new(FixturePort::immediate(runtime)));
        let identity_frozen = frozen();
        let error = resolver
            .resolve(
                &identity_frozen.reference,
                &identity_frozen.descriptor,
                &ResolveContext::new(&NoCredentials, &|| false, &FixedClock),
            )
            .unwrap_err();
        assert_eq!(error.kind(), crate::ResolveErrorKind::Failed);

        let mut runtime = resolved("https://landsatlook.usgs.gov/data/LC09_bundle.tar");
        runtime.checksum = Some(("crc32".to_owned(), "a".repeat(8)));
        let mut registry = AssetResolverRegistry::new();
        registry
            .register(Arc::new(fixture_resolver(Arc::new(
                FixturePort::immediate(runtime),
            ))))
            .unwrap();
        assert!(registry
            .resolve_frozen(
                &frozen(),
                &ResolveContext::new(&NoCredentials, &|| false, &FixedClock),
            )
            .is_err());
    }
}
