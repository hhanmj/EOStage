use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::BhoonidhiSession;
use insar_core::{ProviderNetworkPolicy, SnapshotItem};
use insar_provider::usgs_m2m::UsgsM2mSession;

use crate::transfer_backend::ValidatedTransferDispatch;
use crate::{
    evaluate_refresh, evaluate_resolver_cutover_eligibility,
    evaluate_resolver_cutover_eligibility_with_artifact_validators, validate_execution_policy,
    ArtifactValidatorRegistry, AsfCollectionProfileRegistry, AssetExecutionPolicyRegistry,
    AssetResolverRegistry, AuthRefreshBudget, BhoonidhiAssetResolver,
    BhoonidhiSessionTokenProvider, CdseAuthError, CdseOAuthSession, Clock, CredentialProvider,
    EligibleResolverAsset, EnvironmentEarthdataCredentialProvider, ExecutionPolicyError,
    LandsatM2mAssetResolver, ProductionCredentialProvider, RefreshEvaluation, RefreshReason,
    ReqwestBhoonidhiDownloadProbe, ResolveContext, ResolveError, ResolveErrorKind, ResolvedAsset,
    ResolvedTransferResult, ResolverCutoverEligibility, ResponseHostClass, SystemClock,
    TransferBackend, TransferRequest, TransferRequestError, TransferResult, TransferUpdate,
    ValidatedExecutionPolicy,
};

#[derive(Debug)]
pub enum ResolverExecutionError {
    Policy(ExecutionPolicyError),
    Ineligible(&'static str),
    Runtime(ResolveError),
    CdseLogin(ResolveError),
    UsgsSession(ResolveError),
    BhoonidhiLogin(ResolveError),
    Permission,
    Integrity(ResolveError),
    Cancelled,
    Request(TransferRequestError),
}

impl ResolverExecutionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Integrity(_) => "DL002",
            Self::Permission => "DL004",
            Self::Cancelled => "DL001",
            Self::Runtime(error) if error.kind() == ResolveErrorKind::BurstCookieRequired => {
                "ASF_BURST_COOKIE_REQUIRED"
            }
            Self::Runtime(error)
                if matches!(
                    error.kind(),
                    ResolveErrorKind::CredentialsMissing | ResolveErrorKind::AuthFailed
                ) =>
            {
                "DL004"
            }
            Self::CdseLogin(_) => "DL004",
            Self::UsgsSession(_) | Self::BhoonidhiLogin(_) => "DL004",
            Self::Policy(ExecutionPolicyError::ArtifactValidationUnavailable) => {
                "NISAR_HDF5_VALIDATION_UNAVAILABLE"
            }
            Self::Policy(_) | Self::Ineligible(_) | Self::Runtime(_) | Self::Request(_) => "DL005",
        }
    }

    pub fn reason_zh(&self) -> &'static str {
        match self {
            Self::Permission => "远程服务拒绝了下载权限，未回退到旧下载地址。",
            Self::Integrity(_) => "认证刷新后的资产身份或校验信息发生冲突，未继续传输。",
            Self::Cancelled => "下载已取消，未继续解析或传输资产。",
            Self::Runtime(error) if error.kind() == ResolveErrorKind::BurstCookieRequired => {
                "Sentinel-1 Burst 下载需要账号密码或 .netrc 建立会话 Cookie。"
            }
            Self::Runtime(error)
                if matches!(
                    error.kind(),
                    ResolveErrorKind::CredentialsMissing | ResolveErrorKind::AuthFailed
                ) =>
            {
                "未找到有效的 Earthdata 凭据，或凭据被远程服务拒绝。"
            }
            Self::CdseLogin(_) => "未登录 CDSE 或 CDSE 会话已失效，未回退到旧下载地址。",
            Self::UsgsSession(_) => "未登录 USGS M2M 或当前会话已失效，未回退到旧下载地址。",
            Self::BhoonidhiLogin(_) => {
                "未登录 Bhoonidhi 或当前会话已失效，任务与断点已保留，未回退到其他下载源。"
            }
            Self::Policy(ExecutionPolicyError::ArtifactValidationUnavailable) => {
                "NISAR HDF5 结构校验器不可用，未解析凭据、访问网络或生成最终文件。"
            }
            Self::Policy(_) | Self::Ineligible(_) => {
                "冻结的 Resolver 执行策略无效，未回退到 Legacy 下载。"
            }
            Self::Runtime(_) | Self::Request(_) => "无法解析或传输冻结的下载资产，请稍后重试。",
        }
    }

    pub fn diagnostic_reason(&self) -> String {
        match self {
            Self::Runtime(error)
            | Self::CdseLogin(error)
            | Self::UsgsSession(error)
            | Self::BhoonidhiLogin(error)
            | Self::Integrity(error) => error
                .diagnostic()
                .map(str::to_owned)
                .unwrap_or_else(|| self.reason_zh().to_owned()),
            Self::Request(error) => error.to_string(),
            _ => self.reason_zh().to_owned(),
        }
    }

    /// Only exact, typed auth codes may cross into persisted science diagnostics.
    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn bhoonidhi_auth_cause(&self) -> Option<crate::BhoonidhiAuthError> {
        use crate::BhoonidhiAuthError as Auth;
        let Self::BhoonidhiLogin(error) = self else {
            return None;
        };
        let diagnostic = error.diagnostic()?;
        [
            Auth::Configuration,
            Auth::Missing,
            Auth::CredentialInvalid,
            Auth::Rejected,
            Auth::MaxSessions,
            Auth::ActivationRequired,
            Auth::RateLimited,
            Auth::Transport,
            Auth::Timeout,
            Auth::ResponseInvalid,
            Auth::RefreshUnavailable,
            Auth::RefreshExhausted,
            Auth::LocalRefreshRateLimited,
            Auth::SessionChanged,
            Auth::SessionUnavailable,
            Auth::Synchronization,
        ]
        .into_iter()
        .find(|cause| cause.code() == diagnostic)
    }

    /// Persist only allowlisted static diagnostics, never provider response text.
    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn bhoonidhi_auth_diagnostic(&self) -> Option<&'static str> {
        if let Some(cause) = self.bhoonidhi_auth_cause() {
            return Some(cause.code());
        }
        let (Self::BhoonidhiLogin(error) | Self::Runtime(error)) = self else {
            return None;
        };
        let diagnostic = error.diagnostic()?;
        crate::asset_resolver::bhoonidhi::BHOONIDHI_POST_REFRESH_PROBE_TRANSIENT_DIAGNOSTICS
            .into_iter()
            .find(|allowed| *allowed == diagnostic)
    }

    /// A science-path auth refresh failure that may be retried: a transient
    /// refresh grant (timeout, transport) or a transient post-refresh probe.
    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn bhoonidhi_auth_refresh_is_transient(&self) -> bool {
        use crate::BhoonidhiAuthError as Auth;
        matches!(
            self.bhoonidhi_auth_cause(),
            Some(Auth::Timeout | Auth::Transport)
        ) || matches!(
            self,
            Self::Runtime(error)
                if matches!(error.kind(), ResolveErrorKind::Timeout | ResolveErrorKind::Unavailable)
        )
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Runtime(error)
                if matches!(
                    error.kind(),
                    ResolveErrorKind::Timeout | ResolveErrorKind::Expired | ResolveErrorKind::Unavailable
                )
        )
    }
}

#[derive(Debug)]
pub enum FrozenExecution {
    MissingPolicyCompatibility,
    ExplicitLegacy,
    ResolverAuthoritative(Box<EligibleResolverAsset>),
    Invalid(ResolverExecutionError),
}

pub fn classify_frozen_execution(
    item: &SnapshotItem,
    profiles: &AsfCollectionProfileRegistry,
    policies: &AssetExecutionPolicyRegistry,
) -> FrozenExecution {
    match validate_execution_policy(item.execution_policy.as_ref(), profiles, policies) {
        Ok(ValidatedExecutionPolicy::MissingPolicyCompatibility) => {
            FrozenExecution::MissingPolicyCompatibility
        }
        Ok(ValidatedExecutionPolicy::ExplicitLegacy(_)) => FrozenExecution::ExplicitLegacy,
        Ok(ValidatedExecutionPolicy::ResolverAuthoritative(_)) => {
            match evaluate_resolver_cutover_eligibility(item, profiles, policies) {
                ResolverCutoverEligibility::Eligible(eligible) => {
                    FrozenExecution::ResolverAuthoritative(eligible)
                }
                ResolverCutoverEligibility::Ineligible(reason) => {
                    FrozenExecution::Invalid(ResolverExecutionError::Ineligible(reason.code()))
                }
                ResolverCutoverEligibility::Invalid(error) => {
                    FrozenExecution::Invalid(ResolverExecutionError::Policy(error))
                }
            }
        }
        Err(error) => FrozenExecution::Invalid(ResolverExecutionError::Policy(error)),
    }
}

#[doc(hidden)]
pub fn route_frozen_execution<C, T>(
    execution: FrozenExecution,
    context: C,
    legacy: impl FnOnce(C) -> T,
    resolver: impl FnOnce(C, Box<EligibleResolverAsset>) -> T,
    invalid: impl FnOnce(C, ResolverExecutionError) -> T,
) -> T {
    match execution {
        FrozenExecution::MissingPolicyCompatibility | FrozenExecution::ExplicitLegacy => {
            legacy(context)
        }
        FrozenExecution::ResolverAuthoritative(eligible) => resolver(context, eligible),
        FrozenExecution::Invalid(error) => invalid(context, error),
    }
}

pub struct ResolverExecutionRuntime {
    resolver_registry: AssetResolverRegistry,
    credential_provider: Arc<dyn CredentialProvider>,
    clock: Arc<dyn Clock>,
    network_policy: ProviderNetworkPolicy,
    cdse_session: Option<Arc<CdseOAuthSession>>,
    usgs_session: Option<Arc<UsgsM2mSession>>,
    usgs_login_loader: Option<Arc<dyn UsgsDesktopLoginLoader>>,
    bhoonidhi_session: Option<Arc<BhoonidhiSession>>,
    bhoonidhi_network_policy: Option<ProviderNetworkPolicy>,
    profiles: AsfCollectionProfileRegistry,
    policies: AssetExecutionPolicyRegistry,
    artifact_validators: ArtifactValidatorRegistry,
}

/// Loads the saved USGS M2M login material for one bounded transfer-side
/// re-login after the runtime apiKey was invalidated. Production reads the
/// native desktop credential store; tests inject a fixture so no unit test
/// ever reads the real keyring or performs a network login.
pub trait UsgsDesktopLoginLoader: Send + Sync {
    fn load(
        &self,
    ) -> Result<
        insar_provider::usgs_m2m::UsgsM2mLoginMaterial,
        crate::credentials::DesktopCredentialError,
    >;
}

struct NativeUsgsDesktopLoginLoader;

impl UsgsDesktopLoginLoader for NativeUsgsDesktopLoginLoader {
    fn load(
        &self,
    ) -> Result<
        insar_provider::usgs_m2m::UsgsM2mLoginMaterial,
        crate::credentials::DesktopCredentialError,
    > {
        let (username, application_token) = crate::credentials::resolve_desktop_usgs_login()?;
        insar_provider::usgs_m2m::UsgsM2mLoginMaterial::new(
            username.to_string(),
            insar_provider::usgs_m2m::UsgsM2mApplicationToken::new(application_token.to_string())
                .map_err(|_| crate::credentials::DesktopCredentialError::InvalidInput)?,
        )
        .map_err(|_| crate::credentials::DesktopCredentialError::InvalidInput)
    }
}

/// Durable-evidence binding (restart resume) for one transfer execution.
///
/// Only policies whose resolved access URL changes between executions install
/// a binding. MPC binds to the whole frozen access (its SAS URL expires but
/// the frozen descriptor is URL-free). The USGS official archive (B13) binds
/// to the frozen stable reference — the entity/display id — because every M2M
/// resolve returns a newly signed dds.cr.usgs.gov object URL; binding to the
/// URL (or omitting the binding) would discard every restart prefix. The
/// checkpoint itself still requires a matching strong ETag and an exact 206
/// Content-Range before any append, so bytes of a different object are never
/// spliced onto a recovered prefix.
///
/// The USGS binding depends only on frozen stable identity (product key,
/// asset key, stable asset key, display file name): the task id, item id,
/// policy id, per-request resolver state and the D806 advisory filesize are
/// deliberately excluded, because the restart path recomputes them and a
/// per-attempt or advisory value must not invalidate a saved prefix (live
/// B13 evidence 2026-10-31: a restart-time binding mismatch rejected the
/// checkpoint and failed the task; the advisory 957,283,399 disagreed with
/// the real 957,306,880 object). The binding is stored in the checkpoint
/// record; the destination lease, directory marker and file name are
/// verified separately by `PartCheckpoint::bound`, and the total against the
/// real response's 206 Content-Range.
fn durable_recovery_identity(
    task_id: &str,
    item_id: &str,
    policy_id: &'static str,
    frozen_access: &insar_core::FrozenAssetAccess,
    auth_refresh_policy: crate::AuthRefreshPolicyId,
) -> Result<Option<String>, ResolverExecutionError> {
    use sha2::{Digest, Sha256};
    let identity = match auth_refresh_policy {
        crate::AuthRefreshPolicyId::PlanetaryComputerSasV1
        | crate::AuthRefreshPolicyId::PublicAnonymousHttpsV1 => {
            serde_json::to_vec(&(task_id, item_id, policy_id, frozen_access))
        }
        crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1 => serde_json::to_vec(&(
            &frozen_access.reference.provider_product_key,
            &frozen_access.reference.asset_key,
            &frozen_access.reference.stable_asset_key,
            frozen_access.file_name.as_deref(),
        )),
        _ => return Ok(None),
    }
    .map_err(|_| ResolverExecutionError::Runtime(ResolveError::descriptor_invalid()))?;
    Ok(Some(format!("{:x}", Sha256::digest(identity))))
}

impl ResolverExecutionRuntime {
    /// Add official Landsat delivery without replacing the composed CDSE or
    /// Bhoonidhi authorities. Session installation itself performs no requests.
    pub fn with_usgs_session(mut self, session: Arc<UsgsM2mSession>) -> Result<Self, ResolveError> {
        if self.usgs_session.is_some() {
            return Err(ResolveError::descriptor_invalid());
        }
        self.resolver_registry
            .register(Arc::new(LandsatM2mAssetResolver::legacy(Arc::clone(
                &session,
            ))))?;
        self.resolver_registry
            .register(Arc::new(LandsatM2mAssetResolver::new(Arc::clone(&session))))?;
        self.usgs_session = Some(session);
        Ok(self)
    }

    /// Test seam: replace the native USGS credential loader so a unit test can
    /// drive the transfer-side re-login without reading the real keyring or
    /// performing a network login. Production keeps the native loader.
    #[cfg(test)]
    pub(crate) fn with_usgs_login_loader(
        mut self,
        loader: Arc<dyn UsgsDesktopLoginLoader>,
    ) -> Self {
        self.usgs_login_loader = Some(loader);
        self
    }

    pub fn production() -> Result<Self, ResolveError> {
        Ok(Self::with_dependencies_and_sessions(
            AssetResolverRegistry::with_standard_resolvers()?,
            Arc::new(EnvironmentEarthdataCredentialProvider::new()),
            Arc::new(SystemClock),
            ProviderNetworkPolicy::system(),
            None,
            None,
            None,
        ))
    }

    /// Constructs the standard production registry with the caller-owned CDSE
    /// session. The session is shared by reference; this constructor never
    /// creates a per-request replacement or reads durable credentials.
    pub fn production_with_cdse_session(
        cdse_session: Arc<CdseOAuthSession>,
        network_policy: ProviderNetworkPolicy,
    ) -> Result<Self, ResolveError> {
        if cdse_session.network_policy() != network_policy {
            return Err(ResolveError::descriptor_invalid());
        }
        let credentials: Arc<dyn CredentialProvider> = Arc::new(ProductionCredentialProvider::new(
            Arc::new(EnvironmentEarthdataCredentialProvider::new()),
            Arc::clone(&cdse_session),
        ));
        Ok(Self::with_dependencies_and_sessions(
            AssetResolverRegistry::with_standard_resolvers_for_network(network_policy)?,
            credentials,
            Arc::new(SystemClock),
            network_policy,
            Some(cdse_session),
            None,
            None,
        ))
    }

    /// Construct one production runtime around the caller-owned CDSE and USGS
    /// sessions. The USGS resolver receives the same `Arc<UsgsM2mSession>` used
    /// by catalogue search and bundle-option discovery.
    pub fn production_with_cdse_and_usgs_sessions(
        cdse_session: Arc<CdseOAuthSession>,
        network_policy: ProviderNetworkPolicy,
        usgs_session: Arc<UsgsM2mSession>,
    ) -> Result<Self, ResolveError> {
        if cdse_session.network_policy() != network_policy {
            return Err(ResolveError::descriptor_invalid());
        }
        let credentials: Arc<dyn CredentialProvider> = Arc::new(ProductionCredentialProvider::new(
            Arc::new(EnvironmentEarthdataCredentialProvider::new()),
            Arc::clone(&cdse_session),
        ));
        Self::with_usgs_resolver(
            credentials,
            network_policy,
            Some(cdse_session),
            usgs_session,
        )
    }

    /// Construct one production runtime around caller-owned CDSE and
    /// Bhoonidhi sessions. The Bhoonidhi session is shared with catalogue
    /// search; the resolver owns no second token cache or credential store.
    pub fn production_with_cdse_and_bhoonidhi_sessions(
        cdse_session: Arc<CdseOAuthSession>,
        network_policy: ProviderNetworkPolicy,
        bhoonidhi_session: Arc<BhoonidhiSession>,
        probe_timeout: Duration,
    ) -> Result<Self, ResolveError> {
        let provider = Arc::new(
            BhoonidhiSessionTokenProvider::new(Arc::clone(&bhoonidhi_session)).with_desktop_login(),
        );
        Self::production_with_cdse_and_bhoonidhi_provider(
            cdse_session,
            network_policy,
            bhoonidhi_session,
            provider,
            probe_timeout,
        )
    }

    /// Production composition seam that accepts the one provider instance
    /// shared with catalogue search. Cloning this `Arc` preserves the single
    /// lazy-login gate and generation-aware session boundary.
    pub fn production_with_cdse_and_bhoonidhi_provider(
        cdse_session: Arc<CdseOAuthSession>,
        network_policy: ProviderNetworkPolicy,
        bhoonidhi_session: Arc<BhoonidhiSession>,
        bhoonidhi_provider: Arc<BhoonidhiSessionTokenProvider>,
        probe_timeout: Duration,
    ) -> Result<Self, ResolveError> {
        if cdse_session.network_policy() != network_policy
            || !Arc::ptr_eq(bhoonidhi_provider.session(), &bhoonidhi_session)
        {
            return Err(ResolveError::descriptor_invalid());
        }
        let credentials: Arc<dyn CredentialProvider> = Arc::new(ProductionCredentialProvider::new(
            Arc::new(EnvironmentEarthdataCredentialProvider::new()),
            Arc::clone(&cdse_session),
        ));
        let mut resolver_registry =
            AssetResolverRegistry::with_standard_resolvers_for_network(network_policy)?;
        let resolver_provider: Arc<dyn insar_provider::bhoonidhi::BhoonidhiAccessTokenProvider> =
            bhoonidhi_provider.clone();
        resolver_registry.register(Arc::new(BhoonidhiAssetResolver::new(
            resolver_provider,
            Arc::new(ReqwestBhoonidhiDownloadProbe::new()),
        )))?;
        let _ = probe_timeout;
        let bhoonidhi_network_policy = bhoonidhi_session.network_policy();
        let mut runtime = Self::with_dependencies_and_sessions(
            resolver_registry,
            credentials,
            Arc::new(SystemClock),
            network_policy,
            Some(cdse_session),
            None,
            Some(bhoonidhi_session),
        );
        runtime.bhoonidhi_network_policy = Some(bhoonidhi_network_policy);
        Ok(runtime)
    }

    /// Construct the exact Bhoonidhi S-RSLC runtime around the same
    /// process-owned session used by catalogue search.
    pub fn production_with_bhoonidhi_session(
        bhoonidhi_session: Arc<BhoonidhiSession>,
        probe_timeout: Duration,
    ) -> Result<Self, ResolveError> {
        let provider = Arc::new(
            BhoonidhiSessionTokenProvider::new(Arc::clone(&bhoonidhi_session)).with_desktop_login(),
        );
        Self::production_with_bhoonidhi_provider(bhoonidhi_session, provider, probe_timeout)
    }

    pub fn production_with_bhoonidhi_provider(
        bhoonidhi_session: Arc<BhoonidhiSession>,
        bhoonidhi_provider: Arc<BhoonidhiSessionTokenProvider>,
        probe_timeout: Duration,
    ) -> Result<Self, ResolveError> {
        let network_policy = bhoonidhi_session.network_policy();
        if !Arc::ptr_eq(bhoonidhi_provider.session(), &bhoonidhi_session) {
            return Err(ResolveError::descriptor_invalid());
        }
        let mut resolver_registry =
            AssetResolverRegistry::with_standard_resolvers_for_network(network_policy)?;
        let resolver_provider: Arc<dyn insar_provider::bhoonidhi::BhoonidhiAccessTokenProvider> =
            bhoonidhi_provider.clone();
        resolver_registry.register(Arc::new(BhoonidhiAssetResolver::new(
            resolver_provider,
            Arc::new(ReqwestBhoonidhiDownloadProbe::new()),
        )))?;
        let _ = probe_timeout;
        Ok(Self::with_dependencies_and_sessions(
            resolver_registry,
            Arc::new(EnvironmentEarthdataCredentialProvider::new()),
            Arc::new(SystemClock),
            network_policy,
            None,
            None,
            Some(bhoonidhi_session),
        ))
    }

    /// Construct the existing System-route runtime with one caller-owned USGS
    /// session. This is also the live-acceptance worker boundary.
    pub fn production_with_usgs_session(
        usgs_session: Arc<UsgsM2mSession>,
    ) -> Result<Self, ResolveError> {
        Self::with_usgs_resolver(
            Arc::new(EnvironmentEarthdataCredentialProvider::new()),
            ProviderNetworkPolicy::system(),
            None,
            usgs_session,
        )
    }

    fn with_usgs_resolver(
        credential_provider: Arc<dyn CredentialProvider>,
        network_policy: ProviderNetworkPolicy,
        cdse_session: Option<Arc<CdseOAuthSession>>,
        usgs_session: Arc<UsgsM2mSession>,
    ) -> Result<Self, ResolveError> {
        let mut resolver_registry =
            AssetResolverRegistry::with_standard_resolvers_for_network(network_policy)?;
        resolver_registry.register(Arc::new(LandsatM2mAssetResolver::legacy(Arc::clone(
            &usgs_session,
        ))))?;
        resolver_registry.register(Arc::new(LandsatM2mAssetResolver::new(Arc::clone(
            &usgs_session,
        ))))?;
        Ok(Self::with_dependencies_and_sessions(
            resolver_registry,
            credential_provider,
            Arc::new(SystemClock),
            network_policy,
            cdse_session,
            Some(usgs_session),
            None,
        ))
    }

    #[doc(hidden)]
    pub fn with_dependencies(
        resolver_registry: AssetResolverRegistry,
        credential_provider: Arc<dyn CredentialProvider>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::with_dependencies_and_network_policy(
            resolver_registry,
            credential_provider,
            clock,
            ProviderNetworkPolicy::system(),
        )
    }

    #[doc(hidden)]
    pub fn with_dependencies_and_network_policy(
        resolver_registry: AssetResolverRegistry,
        credential_provider: Arc<dyn CredentialProvider>,
        clock: Arc<dyn Clock>,
        network_policy: ProviderNetworkPolicy,
    ) -> Self {
        Self::with_dependencies_and_sessions(
            resolver_registry,
            credential_provider,
            clock,
            network_policy,
            None,
            None,
            None,
        )
    }

    /// Controlled-endpoint composition seam. It retains the caller-owned
    /// Bhoonidhi session so pointer identity remains observable while tests
    /// replace only the resolver endpoint.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn with_bhoonidhi_dependencies_for_test(
        resolver_registry: AssetResolverRegistry,
        credential_provider: Arc<dyn CredentialProvider>,
        clock: Arc<dyn Clock>,
        bhoonidhi_session: Arc<BhoonidhiSession>,
    ) -> Self {
        let network_policy = bhoonidhi_session.network_policy();
        Self::with_dependencies_and_sessions(
            resolver_registry,
            credential_provider,
            clock,
            network_policy,
            None,
            None,
            Some(Arc::clone(&bhoonidhi_session)),
        )
    }

    fn with_dependencies_and_sessions(
        resolver_registry: AssetResolverRegistry,
        credential_provider: Arc<dyn CredentialProvider>,
        clock: Arc<dyn Clock>,
        network_policy: ProviderNetworkPolicy,
        cdse_session: Option<Arc<CdseOAuthSession>>,
        usgs_session: Option<Arc<UsgsM2mSession>>,
        bhoonidhi_session: Option<Arc<BhoonidhiSession>>,
    ) -> Self {
        Self {
            resolver_registry,
            credential_provider,
            clock,
            network_policy,
            cdse_session,
            usgs_session,
            usgs_login_loader: None,
            bhoonidhi_network_policy: bhoonidhi_session
                .as_ref()
                .map(|session| session.network_policy()),
            bhoonidhi_session,
            profiles: AsfCollectionProfileRegistry,
            policies: AssetExecutionPolicyRegistry,
            artifact_validators: ArtifactValidatorRegistry::standard(),
        }
    }

    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) fn with_dependencies_and_artifact_validators_for_test(
        resolver_registry: AssetResolverRegistry,
        credential_provider: Arc<dyn CredentialProvider>,
        clock: Arc<dyn Clock>,
        artifact_validators: ArtifactValidatorRegistry,
    ) -> Self {
        Self {
            resolver_registry,
            credential_provider,
            clock,
            network_policy: ProviderNetworkPolicy::system(),
            cdse_session: None,
            usgs_session: None,
            usgs_login_loader: None,
            bhoonidhi_session: None,
            bhoonidhi_network_policy: None,
            profiles: AsfCollectionProfileRegistry,
            policies: AssetExecutionPolicyRegistry,
            artifact_validators,
        }
    }

    pub fn classify(&self, item: &SnapshotItem) -> FrozenExecution {
        classify_frozen_execution_with_artifact_validators(
            item,
            &self.profiles,
            &self.policies,
            &self.artifact_validators,
        )
    }

    pub const fn is_ready(&self) -> bool {
        true
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    #[doc(hidden)]
    pub const fn bhoonidhi_network_policy(&self) -> Option<ProviderNetworkPolicy> {
        self.bhoonidhi_network_policy
    }

    #[doc(hidden)]
    pub fn shares_usgs_session(&self, session: &Arc<UsgsM2mSession>) -> bool {
        self.usgs_session
            .as_ref()
            .is_some_and(|owned| Arc::ptr_eq(owned, session))
    }

    #[doc(hidden)]
    pub fn shares_bhoonidhi_session(&self, session: &Arc<BhoonidhiSession>) -> bool {
        self.bhoonidhi_session
            .as_ref()
            .is_some_and(|owned| Arc::ptr_eq(owned, session))
    }

    fn refresh_expired_cdse_session(
        &self,
        eligible: &EligibleResolverAsset,
    ) -> Result<(), ResolverExecutionError> {
        if !matches!(
            eligible.policy.auth_refresh_policy_id,
            crate::AuthRefreshPolicyId::CdseOAuthBearerV1
        ) {
            return Ok(());
        }
        let session = self.cdse_session.as_ref().ok_or_else(|| {
            ResolverExecutionError::CdseLogin(ResolveError::credentials_missing())
        })?;
        session
            .refresh_if_expired()
            .map(|_| ())
            .map_err(map_cdse_session_error)
    }

    fn refresh_rejected_cdse_session(
        &self,
        eligible: &EligibleResolverAsset,
    ) -> Result<(), ResolverExecutionError> {
        if !matches!(
            eligible.policy.auth_refresh_policy_id,
            crate::AuthRefreshPolicyId::CdseOAuthBearerV1
        ) {
            return Ok(());
        }
        let session = self.cdse_session.as_ref().ok_or_else(|| {
            ResolverExecutionError::CdseLogin(ResolveError::credentials_missing())
        })?;
        let snapshot = session.snapshot().map_err(map_cdse_session_error)?;
        let generation = snapshot.generation.ok_or_else(|| {
            ResolverExecutionError::CdseLogin(ResolveError::credentials_missing())
        })?;
        session
            .refresh_after_rejection(generation)
            .map(|_| ())
            .map_err(map_cdse_session_error)
    }

    fn refresh_rejected_bhoonidhi_session(
        &self,
        eligible: &EligibleResolverAsset,
        rejected_asset: &ResolvedAsset,
    ) -> Result<(), ResolverExecutionError> {
        if !matches!(
            eligible.policy.auth_refresh_policy_id,
            crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1
        ) {
            return Ok(());
        }
        let session = self.bhoonidhi_session.as_ref().ok_or_else(|| {
            ResolverExecutionError::BhoonidhiLogin(ResolveError::credentials_missing())
        })?;
        let generation = rejected_asset
            .auth_generation()
            .ok_or_else(|| ResolverExecutionError::BhoonidhiLogin(ResolveError::auth_failed()))?;
        session
            .refresh_after_rejection(generation)
            .map(|_| ())
            .map_err(|error| {
                ResolverExecutionError::BhoonidhiLogin(
                    ResolveError::auth_failed().with_diagnostic(error.code()),
                )
            })
    }

    fn refresh_rejected_usgs_session(
        &self,
        eligible: &EligibleResolverAsset,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<(), ResolverExecutionError> {
        // Only a USGS M2M item may touch the USGS session: the production
        // worker installs the shared session for every provider, and an MPC or
        // CDSE item must never fail because this machine has no USGS
        // credentials.
        if !matches!(
            eligible.policy.auth_refresh_policy_id,
            crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1
        ) {
            return Ok(());
        }
        let Some(session) = self.usgs_session.as_ref() else {
            return Err(ResolverExecutionError::UsgsSession(
                ResolveError::credentials_missing(),
            ));
        };
        // apiKey expiry surfaces as a non-authenticated session: relogin from
        // the saved desktop credentials, bounded to one attempt per item.
        let status = session
            .status()
            .map_err(|_| ResolverExecutionError::UsgsSession(ResolveError::auth_failed()))?;
        if status == insar_provider::usgs_m2m::UsgsM2mSessionStatus::Authenticated {
            return Ok(());
        }
        if cancelled() {
            return Err(ResolverExecutionError::Cancelled);
        }
        let loader: Arc<dyn UsgsDesktopLoginLoader> = self
            .usgs_login_loader
            .clone()
            .unwrap_or_else(|| Arc::new(NativeUsgsDesktopLoginLoader));
        let material = loader.load().map_err(|_| {
            ResolverExecutionError::UsgsSession(ResolveError::credentials_missing())
        })?;
        let outcome = session
            .relogin_after_invalidation(material, cancelled)
            .map_err(|error| {
                // A transport/timeout failure is not a credential rejection:
                // keep it retryable and distinct from the auth pause.
                match error.code() {
                    insar_provider::usgs_m2m::UsgsM2mErrorCode::Timeout => {
                        ResolverExecutionError::Runtime(ResolveError::timeout())
                    }
                    insar_provider::usgs_m2m::UsgsM2mErrorCode::TransportFailed => {
                        ResolverExecutionError::Runtime(
                            ResolveError::unavailable().with_diagnostic(error.code_str()),
                        )
                    }
                    insar_provider::usgs_m2m::UsgsM2mErrorCode::Cancelled => {
                        ResolverExecutionError::Cancelled
                    }
                    _ => ResolverExecutionError::UsgsSession(ResolveError::auth_failed()),
                }
            })?;
        let _ = outcome;
        Ok(())
    }

    fn refresh_rejected_resolver_access(
        &self,
        eligible: &EligibleResolverAsset,
    ) -> Result<(), ResolverExecutionError> {
        if !matches!(
            eligible.policy.auth_refresh_policy_id,
            crate::AuthRefreshPolicyId::PlanetaryComputerSasV1
        ) {
            return Ok(());
        }
        self.resolver_registry
            .prepare_auth_refresh(&eligible.frozen_access)
            .map_err(ResolverExecutionError::Runtime)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn execute_resolver_authoritative(
        &self,
        transfer: &dyn TransferBackend,
        eligible: &EligibleResolverAsset,
        task_id: &str,
        item_id: &str,
        destination: PathBuf,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> Result<TransferResult, ResolverExecutionError> {
        if cancelled() {
            return Err(ResolverExecutionError::Cancelled);
        }
        self.refresh_expired_cdse_session(eligible)?;
        // A USGS apiKey expires server-side (~2h); an unauthenticated session
        // is re-established from the saved desktop credentials before the item
        // resolves, so an idle gap between items cannot fail the task.
        self.refresh_rejected_usgs_session(eligible, cancelled)?;
        // A pause also ends a Bhoonidhi resolution and its probe retry waits
        // (downloader enhancement D1d), as in the NISAR subset route; the item
        // is then interrupted, not failed.
        let stop = || cancelled() || paused();
        let resolve_stop: &(dyn Fn() -> bool + Sync) = if eligible.policy.auth_refresh_policy_id
            == crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1
        {
            &stop
        } else {
            cancelled
        };
        let previous = match self.resolve(eligible, resolve_stop) {
            Ok(asset) => asset,
            Err(ResolverExecutionError::Runtime(error))
                if eligible.policy.auth_refresh_policy_id
                    == crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1
                    && matches!(
                        error.kind(),
                        ResolveErrorKind::CredentialsMissing | ResolveErrorKind::AuthFailed
                    ) =>
            {
                // M4: the local state was still Authenticated at the pre-check,
                // but the M2M request itself discovered the apiKey had expired
                // server-side. Re-login once from the saved credentials and
                // retry the resolution of this item a single time; the retry
                // uses the normal error path so a second failure stays failed.
                self.refresh_rejected_usgs_session(eligible, cancelled)?;
                self.resolve(eligible, resolve_stop)
                    .map_err(|error| contextualize_provider_session_error(eligible, error))?
            }
            Err(error) => return Err(contextualize_provider_session_error(eligible, error)),
        };
        previous
            .ensure_fresh(self.clock.as_ref())
            .map_err(ResolverExecutionError::Runtime)?;
        let request = transfer_request(task_id, item_id, destination.clone(), &previous)?;
        let mut session = transfer.resolved_session();
        if let Some(binding) = durable_recovery_identity(
            task_id,
            item_id,
            eligible.policy.policy_id,
            &eligible.frozen_access,
            eligible.policy.auth_refresh_policy_id,
        )? {
            session.set_recovery_identity(binding);
        }
        let first = download_resolved_for_eligible(
            transfer,
            eligible,
            &request,
            &previous,
            &mut session,
            cancelled,
            paused,
            progress,
        );
        match first {
            ResolvedTransferResult::Finished(result) => return Ok(result),
            ResolvedTransferResult::AuthRejected(rejection)
                if rejection.refresh_candidate
                    && rejection.response_host_class == ResponseHostClass::Approved
                    && matches!(rejection.status_code, 401 | 403) => {}
            ResolvedTransferResult::AuthRejected(_) => {
                return Err(ResolverExecutionError::Permission);
            }
        }
        let mut refresh_budget = AuthRefreshBudget::default();
        let permit = refresh_budget
            .try_consume()
            .map_err(|_| ResolverExecutionError::Permission)?;
        if cancelled() {
            return Err(ResolverExecutionError::Cancelled);
        }
        self.refresh_rejected_cdse_session(eligible)?;
        self.refresh_rejected_bhoonidhi_session(eligible, &previous)?;
        self.refresh_rejected_usgs_session(eligible, cancelled)?;
        self.refresh_rejected_resolver_access(eligible)?;
        let refreshed = self
            .resolve(eligible, cancelled)
            .map_err(|error| contextualize_provider_session_error(eligible, error))
            .map_err(map_refresh_resolve_error)?;
        if cancelled() {
            return Err(ResolverExecutionError::Cancelled);
        }
        let completed_bytes = crate::file_core::part_completed_bytes(&destination);
        let decision = evaluate_refresh(RefreshEvaluation {
            reason: RefreshReason::Unauthorized,
            permit,
            previous_reference: &eligible.frozen_access.reference,
            refreshed_reference: &eligible.frozen_access.reference,
            previous: &previous,
            refreshed: &refreshed,
            completed_bytes,
            prior_response_validator: session.prior_response_validator(),
            clock: self.clock.as_ref(),
        })
        .map_err(ResolverExecutionError::Integrity)?;
        session.apply_refresh_decision(decision);
        let refreshed_request = transfer_request(task_id, item_id, destination, &refreshed)?;
        match download_resolved_for_eligible(
            transfer,
            eligible,
            &refreshed_request,
            &refreshed,
            &mut session,
            cancelled,
            paused,
            progress,
        ) {
            ResolvedTransferResult::Finished(result) => Ok(result),
            ResolvedTransferResult::AuthRejected(_) => Err(ResolverExecutionError::Permission),
        }
    }

    fn resolve(
        &self,
        eligible: &EligibleResolverAsset,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<ResolvedAsset, ResolverExecutionError> {
        let context = ResolveContext::new(
            self.credential_provider.as_ref(),
            cancelled,
            self.clock.as_ref(),
        )
        .with_proxy_policy(
            if eligible.policy.auth_refresh_policy_id
                == crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1
            {
                self.bhoonidhi_network_policy
                    .unwrap_or(self.network_policy)
                    .route()
            } else {
                self.network_policy.route()
            },
        );
        self.resolver_registry
            .resolve_frozen(&eligible.frozen_access, &context)
            .map_err(|error| {
                if error.kind() == ResolveErrorKind::Cancelled {
                    ResolverExecutionError::Cancelled
                } else {
                    ResolverExecutionError::Runtime(error)
                }
            })
    }

    /// Resolve the frozen NISAR source asset for the production Remote
    /// Subset adapter. This deliberately keeps the existing resolver,
    /// credential, and redirect policy in one place; it does not perform a
    /// complete-file transfer.
    #[cfg(feature = "nisar-subset-production")]
    pub fn resolve_for_nisar_remote_subset(
        &self,
        item: &SnapshotItem,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<ResolvedAsset, ResolverExecutionError> {
        let execution = self.classify(item);
        let FrozenExecution::ResolverAuthoritative(eligible) = execution else {
            return Err(ResolverExecutionError::Ineligible(
                "NISAR_LIVE_RESOLVER_NOT_ELIGIBLE",
            ));
        };
        self.refresh_expired_cdse_session(&eligible)?;
        let asset = self
            .resolve(&eligible, cancelled)
            .map_err(|error| contextualize_provider_session_error(&eligible, error))?;
        asset
            .ensure_fresh(self.clock.as_ref())
            .map_err(ResolverExecutionError::Runtime)?;
        Ok(asset)
    }

    /// Refresh one rejected Bhoonidhi bearer used by a NISAR exact-Range
    /// request, then re-resolve the same frozen source authority. Callers must
    /// revalidate URL, strong validator, size, checksum, and product identity
    /// before reissuing the Range request.
    #[cfg(feature = "nisar-subset-production")]
    pub fn refresh_bhoonidhi_nisar_remote_subset(
        &self,
        item: &SnapshotItem,
        rejected_asset: &ResolvedAsset,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<ResolvedAsset, ResolverExecutionError> {
        if cancelled() {
            return Err(ResolverExecutionError::Cancelled);
        }
        let execution = self.classify(item);
        let FrozenExecution::ResolverAuthoritative(eligible) = execution else {
            return Err(ResolverExecutionError::Ineligible(
                "NISAR_LIVE_RESOLVER_NOT_ELIGIBLE",
            ));
        };
        if !matches!(
            eligible.policy.auth_refresh_policy_id,
            crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1
        ) {
            return Err(ResolverExecutionError::Permission);
        }
        self.refresh_rejected_bhoonidhi_session(&eligible, rejected_asset)?;
        if cancelled() {
            return Err(ResolverExecutionError::Cancelled);
        }
        let refreshed = self
            .resolve(&eligible, cancelled)
            .map_err(|error| contextualize_provider_session_error(&eligible, error))
            .map_err(map_refresh_resolve_error)?;
        refreshed
            .ensure_fresh(self.clock.as_ref())
            .map_err(ResolverExecutionError::Runtime)?;
        Ok(refreshed)
    }

    /// Compatibility name for the controlled acceptance harness. The normal
    /// application route uses `resolve_for_nisar_remote_subset` directly.
    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn resolve_for_nisar_live_acceptance(
        &self,
        item: &SnapshotItem,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<ResolvedAsset, ResolverExecutionError> {
        self.resolve_for_nisar_remote_subset(item, cancelled)
    }
}

fn classify_frozen_execution_with_artifact_validators(
    item: &SnapshotItem,
    profiles: &AsfCollectionProfileRegistry,
    policies: &AssetExecutionPolicyRegistry,
    artifact_validators: &ArtifactValidatorRegistry,
) -> FrozenExecution {
    match validate_execution_policy(item.execution_policy.as_ref(), profiles, policies) {
        Ok(ValidatedExecutionPolicy::MissingPolicyCompatibility) => {
            FrozenExecution::MissingPolicyCompatibility
        }
        Ok(ValidatedExecutionPolicy::ExplicitLegacy(_)) => FrozenExecution::ExplicitLegacy,
        Ok(ValidatedExecutionPolicy::ResolverAuthoritative(_)) => {
            match evaluate_resolver_cutover_eligibility_with_artifact_validators(
                item,
                profiles,
                policies,
                artifact_validators,
            ) {
                ResolverCutoverEligibility::Eligible(eligible) => {
                    FrozenExecution::ResolverAuthoritative(eligible)
                }
                ResolverCutoverEligibility::Ineligible(reason) => {
                    FrozenExecution::Invalid(ResolverExecutionError::Ineligible(reason.code()))
                }
                ResolverCutoverEligibility::Invalid(error) => {
                    FrozenExecution::Invalid(ResolverExecutionError::Policy(error))
                }
            }
        }
        Err(error) => FrozenExecution::Invalid(ResolverExecutionError::Policy(error)),
    }
}

#[allow(clippy::too_many_arguments)]
fn download_resolved_for_eligible(
    transfer: &dyn TransferBackend,
    eligible: &EligibleResolverAsset,
    request: &TransferRequest,
    asset: &ResolvedAsset,
    session: &mut crate::ResolvedTransferSession,
    cancelled: &(dyn Fn() -> bool + Sync),
    paused: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(TransferUpdate),
) -> ResolvedTransferResult {
    match eligible.artifact_validation_plan() {
        Some(plan) => {
            let dispatch = ValidatedTransferDispatch::new();
            let sealed = transfer.download_resolved_validated(
                &dispatch,
                request,
                asset,
                eligible.policy.auth_refresh_policy_id,
                plan,
                session,
                cancelled,
                paused,
                progress,
            );
            sealed
                .into_verified_result(request, plan, session, &dispatch)
                .unwrap_or_else(|| {
                    ResolvedTransferResult::Finished(
                        plan.unavailable_transfer_result(
                            &request.destination,
                            session.attempts_used(),
                        ),
                    )
                })
        }
        None => transfer.download_resolved(
            request,
            asset,
            eligible.policy.auth_refresh_policy_id,
            session,
            cancelled,
            paused,
            progress,
        ),
    }
}

fn map_refresh_resolve_error(error: ResolverExecutionError) -> ResolverExecutionError {
    match error {
        ResolverExecutionError::Runtime(error)
            if matches!(
                error.kind(),
                ResolveErrorKind::ReferenceInvalid
                    | ResolveErrorKind::DescriptorInvalid
                    | ResolveErrorKind::Failed
            ) =>
        {
            ResolverExecutionError::Integrity(error)
        }
        other => other,
    }
}

fn contextualize_provider_session_error(
    eligible: &EligibleResolverAsset,
    error: ResolverExecutionError,
) -> ResolverExecutionError {
    contextualize_provider_session_error_for_policy(eligible.policy.auth_refresh_policy_id, error)
}

fn contextualize_provider_session_error_for_policy(
    policy: crate::AuthRefreshPolicyId,
    error: ResolverExecutionError,
) -> ResolverExecutionError {
    if matches!(policy, crate::AuthRefreshPolicyId::CdseOAuthBearerV1)
        && matches!(
            &error,
            ResolverExecutionError::Runtime(resolve_error)
                if matches!(
                    resolve_error.kind(),
                    ResolveErrorKind::CredentialsMissing | ResolveErrorKind::AuthFailed
                )
        )
    {
        let ResolverExecutionError::Runtime(resolve_error) = error else {
            unreachable!("the preceding match establishes the runtime error variant");
        };
        return ResolverExecutionError::CdseLogin(resolve_error);
    }
    if matches!(policy, crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1)
        && matches!(
            &error,
            ResolverExecutionError::Runtime(resolve_error)
                if matches!(
                    resolve_error.kind(),
                    ResolveErrorKind::CredentialsMissing | ResolveErrorKind::AuthFailed
                )
        )
    {
        let ResolverExecutionError::Runtime(resolve_error) = error else {
            unreachable!("the preceding match establishes the runtime error variant");
        };
        return ResolverExecutionError::BhoonidhiLogin(resolve_error);
    }
    if matches!(policy, crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1)
        && matches!(
            &error,
            ResolverExecutionError::Runtime(resolve_error)
                if matches!(
                    resolve_error.kind(),
                    ResolveErrorKind::CredentialsMissing | ResolveErrorKind::AuthFailed
                )
        )
    {
        let ResolverExecutionError::Runtime(resolve_error) = error else {
            unreachable!("the preceding match establishes the runtime error variant");
        };
        return ResolverExecutionError::UsgsSession(resolve_error);
    }
    error
}

fn map_cdse_session_error(error: CdseAuthError) -> ResolverExecutionError {
    match error {
        CdseAuthError::SessionUnavailable
        | CdseAuthError::RefreshUnavailable
        | CdseAuthError::RefreshExhausted
        | CdseAuthError::AccessExpired
        | CdseAuthError::SessionChanged => {
            ResolverExecutionError::CdseLogin(ResolveError::credentials_missing())
        }
        CdseAuthError::Rejected => ResolverExecutionError::CdseLogin(ResolveError::auth_failed()),
        CdseAuthError::Timeout => ResolverExecutionError::Runtime(ResolveError::timeout()),
        CdseAuthError::Configuration | CdseAuthError::LoginMaterialInvalid => {
            ResolverExecutionError::Runtime(ResolveError::descriptor_invalid())
        }
        CdseAuthError::Transport
        | CdseAuthError::ResponseInvalid
        | CdseAuthError::Synchronization => ResolverExecutionError::Runtime(ResolveError::failed()),
    }
}

fn transfer_request(
    task_id: &str,
    item_id: &str,
    destination: PathBuf,
    asset: &ResolvedAsset,
) -> Result<TransferRequest, ResolverExecutionError> {
    TransferRequest::new(
        task_id,
        item_id,
        asset.effective_url().as_str(),
        destination,
        asset.expected_size(),
    )
    .map_err(ResolverExecutionError::Request)
}

#[cfg(test)]
mod tests {
    use super::{
        contextualize_provider_session_error_for_policy, map_cdse_session_error,
        ResolverExecutionError,
    };
    use crate::CdseAuthError;

    /// Design 2026-10-17 §4: a post-refresh probe that is temporarily
    /// unavailable is not an asset identity conflict (Integrity) and keeps its
    /// static staged cause; only transient refresh/probe failures are retryable.
    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn bhoonidhi_transient_post_refresh_probe_is_not_integrity_and_stays_retryable() {
        use crate::BhoonidhiAuthError as Auth;
        let probe = ResolverExecutionError::Runtime(
            crate::ResolveError::unavailable()
                .with_diagnostic("BHOONIDHI_DOWNLOAD_PROBE_STATUS_503 stage=post_refresh_probe"),
        );
        let mapped = super::map_refresh_resolve_error(probe);
        assert!(
            matches!(mapped, ResolverExecutionError::Runtime(_)),
            "{mapped:?}"
        );
        assert_ne!(mapped.code(), "DL002");
        assert!(mapped.retryable());
        assert!(mapped.bhoonidhi_auth_refresh_is_transient());
        assert_eq!(
            mapped.bhoonidhi_auth_diagnostic(),
            Some("BHOONIDHI_DOWNLOAD_PROBE_STATUS_503 stage=post_refresh_probe")
        );
        for cause in [Auth::Timeout, Auth::Transport] {
            let error = ResolverExecutionError::BhoonidhiLogin(
                crate::ResolveError::auth_failed().with_diagnostic(cause.code()),
            );
            assert!(error.bhoonidhi_auth_refresh_is_transient(), "{cause:?}");
        }
        for cause in [
            Auth::Rejected,
            Auth::RefreshExhausted,
            Auth::LocalRefreshRateLimited,
            Auth::RateLimited,
        ] {
            let error = ResolverExecutionError::BhoonidhiLogin(
                crate::ResolveError::auth_failed().with_diagnostic(cause.code()),
            );
            assert!(!error.bhoonidhi_auth_refresh_is_transient(), "{cause:?}");
        }
        let integrity = super::map_refresh_resolve_error(ResolverExecutionError::Runtime(
            crate::ResolveError::failed(),
        ));
        assert_eq!(integrity.code(), "DL002");
        assert!(!integrity.bhoonidhi_auth_refresh_is_transient());
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn bhoonidhi_science_auth_diagnostics_reject_raw_or_unrelated_text() {
        use crate::BhoonidhiAuthError as Auth;
        for cause in [
            Auth::Configuration,
            Auth::Missing,
            Auth::CredentialInvalid,
            Auth::Rejected,
            Auth::MaxSessions,
            Auth::ActivationRequired,
            Auth::RateLimited,
            Auth::Transport,
            Auth::Timeout,
            Auth::ResponseInvalid,
            Auth::RefreshUnavailable,
            Auth::RefreshExhausted,
            Auth::LocalRefreshRateLimited,
            Auth::SessionChanged,
            Auth::SessionUnavailable,
            Auth::Synchronization,
        ] {
            let error = ResolverExecutionError::BhoonidhiLogin(
                crate::ResolveError::auth_failed().with_diagnostic(cause.code()),
            );
            assert_eq!(error.bhoonidhi_auth_cause(), Some(cause));
            assert_eq!(error.bhoonidhi_auth_diagnostic(), Some(cause.code()));
        }
        for diagnostic in [
            "BHOONIDHI_AUTH_TIMEOUT https://example.invalid/private",
            "Bearer fixture-secret",
            "BHOONIDHI_AUTH_TIMEOUT\n",
            "BHOONIDHI_DOWNLOAD_PROBE_STATUS_401 stage=post_refresh_probe https://example.invalid/private",
            "BHOONIDHI_DOWNLOAD_PROBE_STATUS_401 stage=post_refresh_probe\n",
            "unknown",
        ] {
            let error = ResolverExecutionError::BhoonidhiLogin(
                crate::ResolveError::auth_failed().with_diagnostic(diagnostic),
            );
            assert_eq!(error.bhoonidhi_auth_cause(), None);
            assert_eq!(error.bhoonidhi_auth_diagnostic(), None);
        }
        let unrelated = ResolverExecutionError::Runtime(
            crate::ResolveError::auth_failed().with_diagnostic(Auth::Timeout.code()),
        );
        assert_eq!(unrelated.bhoonidhi_auth_cause(), None);
        assert_eq!(unrelated.bhoonidhi_auth_diagnostic(), None);
    }

    #[test]
    fn bhoonidhi_science_auth_refresh_preserves_typed_endpoint_failure() {
        use super::*;
        use crate::{
            BhoonidhiAuthError, BhoonidhiLoginMaterial, BhoonidhiTokenEndpoint, BhoonidhiTokenGrant,
        };
        use insar_core::{
            provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
            AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters, AssetRole,
            FrozenAssetAccess, StableAssetReference,
        };
        use insar_provider::bhoonidhi::*;
        use serde_json::{json, Map};

        struct Endpoint(BhoonidhiAuthError);
        impl BhoonidhiTokenEndpoint for Endpoint {
            fn network_policy(&self) -> ProviderNetworkPolicy {
                ProviderNetworkPolicy::system()
            }
            fn password_grant(
                &self,
                _: &BhoonidhiLoginMaterial,
            ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
                BhoonidhiTokenGrant::new(
                    "fixture-access",
                    Some("fixture-refresh".to_owned()),
                    Some(1200),
                )
            }
            fn refresh_grant(
                &self,
                _: &str,
                _: &str,
            ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
                Err(self.0)
            }
            fn logout(&self, _: &str) -> Result<(), BhoonidhiAuthError> {
                Ok(())
            }
        }
        const PRODUCT: &str = "NISAR_S1_PR_RSLC_030_026_A_020_3700_DHNA_A_20260907T234951_20260907T235005_P00500_M_P_I_001";
        let key =
            provider_product_key(BHOONIDHI_PROVIDER_ID, BHOONIDHI_COLLECTION_ID, PRODUCT).unwrap();
        let item = SnapshotItem {
            item_id: key.clone(), provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
            display_name: PRODUCT.to_owned(), acquisition_datetime: None, footprint_geojson: None,
            metadata: json!({
                "dataset": BHOONIDHI_LOGICAL_DATASET, "logical_dataset": BHOONIDHI_LOGICAL_DATASET,
                "native_collection_id": BHOONIDHI_COLLECTION_ID, "collection_profile_id": BHOONIDHI_PROFILE_ID,
                "identity_rule_id": BHOONIDHI_IDENTITY_RULE_ID, "filename_rule_id": BHOONIDHI_FILENAME_RULE_ID,
                "resolver_policy_id": BHOONIDHI_RESOLVER_POLICY_ID, "delivery_model": "archive_first",
                "product_family": BHOONIDHI_COLLECTION_ID, "processing_level": "RSLC",
                "provider_product_id": PRODUCT, "provider_product_key": key,
                "properties": {"ID": PRODUCT, "Online": "Y", "Platform": "NISAR", "Instrument": "SSAR",
                    "Producer": "ISRO", "ProductCode": "ST0000KED", "Mode": "W", "Polarization": "DH",
                    "SoftwareVersion": "1.4.00", "Coverage": "P", "mode": "W", "polarization": "DH", "software_version": "1.4.00"}
            }).as_object().unwrap().clone(),
            payload: Map::new(),
            asset_access: Some(FrozenAssetAccess {
                reference: StableAssetReference { provider_id: BHOONIDHI_PROVIDER_ID.to_owned(), collection_id: BHOONIDHI_COLLECTION_ID.to_owned(), provider_product_id: PRODUCT.to_owned(), provider_product_key: key.clone(), canonical_product_key: None, asset_key: "data".to_owned(), stable_asset_key: stable_asset_key(&key,"data").unwrap() },
                descriptor: AssetAccessDescriptor { resolver_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(), scheme: AssetAccessScheme::OauthBearerHttps, stable_href: Some("https://bhoonidhi-api.nrsc.gov.in/download".to_owned()), requires_credentials: true, credential_profile: Some("bhoonidhi-default".to_owned()), resolver_parameters: AssetResolverParameters {} },
                role: AssetRole::Primary, file_name: Some(format!("{PRODUCT}.h5")), expected_size_bytes: None, checksum_algorithm: None, checksum_value: None,
            }),
            execution_policy: Some(AssetExecutionPolicy { authority: AssetExecutionAuthority::ResolverAuthoritative, policy_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(), collection_profile_id: Some(BHOONIDHI_PROFILE_ID.to_owned()) }),
        };
        for cause in [
            BhoonidhiAuthError::Timeout,
            BhoonidhiAuthError::Rejected,
            BhoonidhiAuthError::RateLimited,
            BhoonidhiAuthError::Transport,
        ] {
            let session = Arc::new(
                BhoonidhiSession::new(
                    ProviderNetworkPolicy::system(),
                    Arc::new(Endpoint(cause)),
                    Arc::new(SystemClock),
                )
                .unwrap(),
            );
            session
                .login(BhoonidhiLoginMaterial::new("fixture-user", "fixture-password").unwrap())
                .unwrap();
            let generation = session.access_token().unwrap().generation();
            let provider = Arc::new(BhoonidhiSessionTokenProvider::new(Arc::clone(&session)));
            let runtime = ResolverExecutionRuntime::production_with_bhoonidhi_provider(
                session,
                provider,
                Duration::from_secs(1),
            )
            .unwrap();
            let FrozenExecution::ResolverAuthoritative(eligible) = runtime.classify(&item) else {
                panic!("fixture must satisfy production policy")
            };
            let asset = ResolvedAsset::new(
                "https://bhoonidhi-api.nrsc.gov.in/download",
                crate::ResolvedMethod::Get,
            )
            .unwrap()
            .with_auth_generation(generation);
            let error = runtime
                .refresh_rejected_bhoonidhi_session(&eligible, &asset)
                .unwrap_err();
            assert_eq!(error.code(), "DL004");
            assert!(!error.retryable());
            assert_eq!(error.diagnostic_reason(), cause.code());
        }
    }

    #[test]
    fn usgs_composition_preserves_existing_sessions_and_registers_v1_and_v2_resolvers() {
        use super::*;
        let policy = ProviderNetworkPolicy::system();
        let cdse = Arc::new(
            crate::CdseNetworkFoundation::new(policy)
                .oauth_session()
                .unwrap(),
        );
        let bhoonidhi = Arc::new(
            BhoonidhiSession::new(
                policy,
                Arc::new(crate::ReqwestBhoonidhiTokenEndpoint::new(policy).unwrap()),
                Arc::new(SystemClock),
            )
            .unwrap(),
        );
        let provider = Arc::new(BhoonidhiSessionTokenProvider::new(Arc::clone(&bhoonidhi)));
        let runtime = ResolverExecutionRuntime::production_with_cdse_and_bhoonidhi_provider(
            Arc::clone(&cdse),
            policy,
            Arc::clone(&bhoonidhi),
            provider,
            Duration::from_secs(1),
        )
        .unwrap();
        let before = runtime
            .resolver_registry
            .resolver_ids()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let usgs =
            Arc::new(UsgsM2mSession::new(policy, std::time::Duration::from_secs(1)).unwrap());
        let runtime = runtime.with_usgs_session(Arc::clone(&usgs)).unwrap();
        assert!(runtime.shares_usgs_session(&usgs));
        assert!(runtime.shares_bhoonidhi_session(&bhoonidhi));
        assert!(Arc::ptr_eq(runtime.cdse_session.as_ref().unwrap(), &cdse));
        let after = runtime.resolver_registry.resolver_ids();
        assert_eq!(after.len(), before.len() + 2);
        assert!(before.iter().all(|id| after.contains(&id.as_str())));
        assert!(after.contains(&crate::USGS_LANDSAT_C2_L2_RESOLVER_ID_V1));
        assert!(after.contains(&crate::USGS_LANDSAT_C2_L2_RESOLVER_ID_V2));
        assert_eq!(usgs.request_counts().scene_search, 0);
        assert_eq!(usgs.request_counts().download_request, 0);
        assert!(
            runtime.with_usgs_session(usgs).is_err(),
            "an existing session must never be replaced"
        );
    }

    #[test]
    fn logged_out_or_replaced_cdse_sessions_have_a_distinct_typed_download_error() {
        for error in [
            CdseAuthError::SessionUnavailable,
            CdseAuthError::SessionChanged,
            CdseAuthError::RefreshUnavailable,
            CdseAuthError::RefreshExhausted,
        ] {
            let mapped = map_cdse_session_error(error);
            assert_eq!(mapped.code(), "DL004");
            assert!(mapped.reason_zh().contains("CDSE"));
            assert!(matches!(mapped, ResolverExecutionError::CdseLogin(_)));
        }
    }

    #[test]
    fn nisar_bhoonidhi_auth_failure_keeps_provider_session_context() {
        let mapped = contextualize_provider_session_error_for_policy(
            crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1,
            ResolverExecutionError::Runtime(crate::ResolveError::auth_failed()),
        );

        assert_eq!(mapped.code(), "DL004");
        assert!(mapped.reason_zh().contains("Bhoonidhi"));
        assert!(!mapped.retryable());
        assert!(matches!(mapped, ResolverExecutionError::BhoonidhiLogin(_)));
    }

    #[test]
    fn nisar_bhoonidhi_stale_product_403_stays_non_auth_dl005() {
        let diag = crate::asset_resolver::bhoonidhi::BHOONIDHI_PROBE_FORBIDDEN_DIAGNOSTIC_ZH;
        let mapped = contextualize_provider_session_error_for_policy(
            crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1,
            ResolverExecutionError::Runtime(crate::ResolveError::failed().with_diagnostic(diag)),
        );

        assert_eq!(mapped.code(), "DL005");
        assert!(!mapped.retryable());
        assert_eq!(mapped.diagnostic_reason(), diag);
        assert!(matches!(mapped, ResolverExecutionError::Runtime(_)));
    }

    #[test]
    fn resolver_execution_prefers_preserved_external_diagnostic_text() {
        let diagnostic =
            "USGS M2M endpoint=download_request code=USGS_M2M_REMOTE_ERROR upstream_error_code=ORDER_NOT_ALLOWED";
        let error = ResolverExecutionError::Runtime(
            crate::ResolveError::failed().with_diagnostic(diagnostic),
        );

        assert_eq!(error.code(), "DL005");
        assert_eq!(error.diagnostic_reason(), diagnostic);
        assert_ne!(error.diagnostic_reason(), error.reason_zh());
    }

    #[test]
    fn usgs_session_relogin_keeps_the_typed_dl004_pause_shape() {
        // A USGS item with no session installed in the runtime is a typed
        // credentials-missing DL004 (the worker must be composed with the
        // shared session). A session without saved desktop credentials maps
        // to the same typed pause, never a hard DL005, and never performs a
        // network login. The full invalidation/relogin path is covered by the
        // provider session tests
        // (relogin_after_invalidation_restores_an_expired_session_once).
        use super::*;
        struct MissingCredentials;
        impl UsgsDesktopLoginLoader for MissingCredentials {
            fn load(
                &self,
            ) -> Result<
                insar_provider::usgs_m2m::UsgsM2mLoginMaterial,
                crate::credentials::DesktopCredentialError,
            > {
                Err(crate::credentials::DesktopCredentialError::InvalidInput)
            }
        }

        let usgs_item = EligibleResolverAsset::for_test_without_validation_plan(
            usgs_frozen_access_fixture(),
            static_policy_for_kind(PolicyKind::UsgsM2m),
        );
        let no_cancel = || false;

        let policy = ProviderNetworkPolicy::system();
        let cdse = Arc::new(
            crate::CdseNetworkFoundation::new(policy)
                .oauth_session()
                .unwrap(),
        );
        let runtime =
            ResolverExecutionRuntime::production_with_cdse_session(Arc::clone(&cdse), policy)
                .unwrap();
        let no_session = runtime
            .refresh_rejected_usgs_session(&usgs_item, &no_cancel)
            .unwrap_err();
        assert_eq!(no_session.code(), "DL004");
        assert!(matches!(no_session, ResolverExecutionError::UsgsSession(_)));

        let usgs =
            Arc::new(UsgsM2mSession::new(policy, std::time::Duration::from_secs(1)).unwrap());
        let runtime = runtime
            .with_usgs_session(Arc::clone(&usgs))
            .unwrap()
            .with_usgs_login_loader(Arc::new(MissingCredentials));
        let error = runtime
            .refresh_rejected_usgs_session(&usgs_item, &no_cancel)
            .unwrap_err();
        assert_eq!(error.code(), "DL004");
        assert!(!error.retryable());
        assert!(matches!(error, ResolverExecutionError::UsgsSession(_)));
        assert_eq!(usgs.request_counts().login_token, 0);
        assert_eq!(
            usgs.status().unwrap(),
            insar_provider::usgs_m2m::UsgsM2mSessionStatus::Ready
        );
    }

    /// H1: the shared USGS session is installed for every provider by the
    /// production worker; a non-USGS item (for example Planetary Computer)
    /// must never touch it, or a machine without USGS credentials could fail
    /// an unrelated download with a USGS credential error.
    #[test]
    fn non_usgs_items_never_touch_the_shared_usgs_session() {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingLoader(Arc<AtomicUsize>);
        impl UsgsDesktopLoginLoader for CountingLoader {
            fn load(
                &self,
            ) -> Result<
                insar_provider::usgs_m2m::UsgsM2mLoginMaterial,
                crate::credentials::DesktopCredentialError,
            > {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(crate::credentials::DesktopCredentialError::InvalidInput)
            }
        }

        let policy = ProviderNetworkPolicy::system();
        let cdse = Arc::new(
            crate::CdseNetworkFoundation::new(policy)
                .oauth_session()
                .unwrap(),
        );
        let usgs =
            Arc::new(UsgsM2mSession::new(policy, std::time::Duration::from_secs(1)).unwrap());
        let counter = Arc::new(AtomicUsize::new(0));
        let loader: Arc<dyn UsgsDesktopLoginLoader> =
            Arc::new(CountingLoader(Arc::clone(&counter)));
        let runtime =
            ResolverExecutionRuntime::production_with_cdse_session(Arc::clone(&cdse), policy)
                .unwrap()
                .with_usgs_session(Arc::clone(&usgs))
                .unwrap()
                .with_usgs_login_loader(loader);

        let no_cancel = || false;
        for kind in [PolicyKind::Cdse, PolicyKind::Mpc, PolicyKind::Bhoonidhi] {
            let item = EligibleResolverAsset::for_test_without_validation_plan(
                usgs_frozen_access_fixture(),
                static_policy_for_kind(kind),
            );
            // The recovery seam is a no-op for the non-USGS item even though
            // the never-logged-in USGS session is installed: no credential
            // read, no login attempt, no error.
            assert!(
                runtime
                    .refresh_rejected_usgs_session(&item, &no_cancel)
                    .is_ok(),
                "{kind:?} item must not touch the USGS session"
            );
        }
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert_eq!(usgs.request_counts().login_token, 0);
    }

    #[derive(Clone, Copy, Debug)]
    enum PolicyKind {
        UsgsM2m,
        Cdse,
        Mpc,
        Bhoonidhi,
    }

    fn static_policy_for_kind(kind: PolicyKind) -> &'static crate::AssetExecutionPolicyDefinition {
        let registry = crate::AssetExecutionPolicyRegistry;
        let policy_id = match kind {
            PolicyKind::UsgsM2m => crate::USGS_LANDSAT_C2_L2_POLICY_ID_V2,
            PolicyKind::Cdse => crate::CDSE_SENTINEL2_L2A_POLICY_ID,
            PolicyKind::Mpc => insar_provider::catalog::MPC_LANDSAT_ASSET_POLICY_ID,
            PolicyKind::Bhoonidhi => insar_provider::bhoonidhi::BHOONIDHI_RESOLVER_POLICY_ID,
        };
        registry
            .get(policy_id)
            .expect("a reviewed static policy exists for each test kind")
    }

    /// A minimal USGS-shaped frozen access: the recovery seam only reads the
    /// policy's refresh kind, so the fixture needs no valid locator.
    fn usgs_frozen_access_fixture() -> insar_core::FrozenAssetAccess {
        use insar_core::{
            stable_asset_key, AssetAccessDescriptor, AssetAccessScheme, AssetResolverParameters,
            AssetRole, FrozenAssetAccess, StableAssetReference,
        };
        let product_key = insar_core::provider_product_key(
            "usgs.eros.m2m",
            "landsat_ot_c2_l2",
            "LC91230452026001LGN00",
        )
        .unwrap();
        FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: "usgs.eros.m2m".to_owned(),
                collection_id: "landsat_ot_c2_l2".to_owned(),
                provider_product_id: "LC91230452026001LGN00".to_owned(),
                provider_product_key: product_key.clone(),
                canonical_product_key: None,
                asset_key: "5e83d14fec7cae84".to_owned(),
                stable_asset_key: stable_asset_key(&product_key, "5e83d14fec7cae84").unwrap(),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: crate::USGS_LANDSAT_C2_L2_RESOLVER_ID_V2.to_owned(),
                scheme: AssetAccessScheme::UsgsM2m,
                stable_href: None,
                requires_credentials: true,
                credential_profile: Some(
                    insar_provider::usgs_m2m::USGS_M2M_SESSION_PROFILE_ID.to_owned(),
                ),
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some("LC09_L2SP_123045_20260101_20260103_02_T1.tar".to_owned()),
            expected_size_bytes: Some(1024),
            checksum_algorithm: None,
            checksum_value: None,
        }
    }

    // B13: the USGS official archive resolves a newly signed dds.cr.usgs.gov
    // object URL on every execution, so the durable prefix evidence must bind
    // to the frozen locator (entity/display id), never the URL. These tests
    // drive the real execute_resolver_authoritative seam with an eligible
    // USGS item and a recording transfer backend.

    /// A resolver-authoritative USGS Landsat bundle item built from the same
    /// reviewed catalog authority the production freeze uses.
    fn usgs_eligible_fixture() -> Box<crate::EligibleResolverAsset> {
        use crate::{AsfCollectionProfileRegistry, AssetExecutionPolicyRegistry};
        let provider_product_key = insar_core::provider_product_key(
            "usgs.eros.m2m",
            "landsat_ot_c2_l2",
            "LC91230452026001LGN00",
        )
        .unwrap();
        let mut item: insar_core::SnapshotItem = serde_json::from_value(serde_json::json!({
            "item_id": provider_product_key,
            "provider_id": "usgs.eros.m2m",
            "display_name": "LC09_L2SP_123045_20260101_20260103_02_T1",
            "acquisition_datetime": "2026-01-01T00:00:00Z",
            "footprint_geojson": null,
            "metadata": {
                "logical_dataset": "optical.landsat.c2.l2",
                "native_collection_id": "landsat_ot_c2_l2",
                "collection_profile_id": insar_provider::catalog::LANDSAT_C2_L2_SCENE_PROFILE_ID,
                "identity_rule_id": "usgs.eros.m2m.landsat.c2.l2.identity.v1",
                "filename_rule_id": "usgs.eros.m2m.landsat.c2.l2.full-bundle-filename.v1",
                "delivery_model": "archive_first",
                "provider_product_id": "LC91230452026001LGN00",
                "provider_product_key": provider_product_key,
                "source_kind": "landsat",
                "product_kind": "optical_scene",
                "properties": {
                    "entity_id": "LC91230452026001LGN00",
                    "landsat_product_id": "LC09_L2SP_123045_20260101_20260103_02_T1",
                    "dataset_alias": "landsat_ot_c2_l2",
                    "processing_level": "l2sp"
                }
            },
            "payload": {},
            "asset_access": null,
            "execution_policy": null
        }))
        .unwrap();
        let option = insar_provider::usgs_m2m::UsgsM2mFullBundleOption {
            dataset_alias: "landsat_ot_c2_l2".to_owned(),
            dataset_id: "5e83d14f2fc39685".to_owned(),
            entity_id: "LC91230452026001LGN00".to_owned(),
            display_id: "LC09_L2SP_123045_20260101_20260103_02_T1".to_owned(),
            option_id: insar_provider::usgs_m2m::LANDSAT_C2_L2_BUNDLE_OPTION_ID.to_owned(),
            product_name: insar_provider::usgs_m2m::LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME.to_owned(),
            product_code: insar_provider::usgs_m2m::LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE.to_owned(),
            download_system: insar_provider::usgs_m2m::LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM
                .to_owned(),
            availability: insar_provider::usgs_m2m::UsgsM2mBundleAvailability::Available,
            expected_file_name: "LC09_L2SP_123045_20260101_20260103_02_T1.tar".to_owned(),
            expected_size_bytes: Some(6),
            checksum_algorithm: None,
            checksum_value: None,
        };
        item.asset_access = Some(
            crate::build_landsat_c2_l2_bundle_access(
                &item,
                &option,
                crate::CatalogPolicyContext::Production,
            )
            .expect("reviewed Landsat bundle authority"),
        );
        item.execution_policy = Some(
            serde_json::from_value(serde_json::json!({
                "authority": "resolver_authoritative",
                "policy_id": crate::USGS_LANDSAT_C2_L2_POLICY_ID_V2,
                "collection_profile_id": insar_provider::catalog::LANDSAT_C2_L2_SCENE_PROFILE_ID
            }))
            .unwrap(),
        );
        match crate::classify_frozen_execution(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ) {
            crate::FrozenExecution::ResolverAuthoritative(eligible) => eligible,
            other => panic!("the USGS bundle fixture must be resolver eligible: {other:?}"),
        }
    }

    #[test]
    fn usgs_recovery_identity_is_stable_across_restarts_and_separates_objects() {
        // The same frozen USGS item resolves a different signed object URL on
        // every execution, and the restart path recomputes task-scoped inputs,
        // so the durable-evidence binding (B13 restart resume) must depend
        // only on frozen stable identity: identical across restarts, task ids,
        // item ids and policy ids; different for a different frozen object
        // (product key, asset key, display file name or frozen total); absent
        // for policies without a re-resolved URL.
        let eligible = usgs_eligible_fixture();
        let access = &eligible.frozen_access;
        let policy_id = eligible.policy.policy_id;
        let a = super::durable_recovery_identity(
            "usgs-recovery-task",
            "usgs-recovery-item",
            policy_id,
            access,
            crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        )
        .unwrap()
        .expect("the USGS official archive must install a recovery identity");
        assert_eq!(a.len(), 64, "the binding is a SHA-256 hex digest");
        // Different task id, item id and policy id must NOT change the
        // binding: live B13 evidence 2026-10-31 showed a restart-time
        // recomputation rejecting the saved checkpoint and failing the task.
        for (task, item) in [
            ("usgs-recovery-task", "usgs-recovery-item"),
            ("usgs-recovery-task-b", "another-item"),
        ] {
            let binding = super::durable_recovery_identity(
                task,
                item,
                policy_id,
                access,
                crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
            )
            .unwrap()
            .expect("every USGS execution installs the binding");
            assert_eq!(a, binding, "per-restart inputs must not change the binding");
        }
        // A different frozen object must not share prefix evidence: a drifted
        // product key, asset key, display file name or frozen total each
        // change the binding, so a checkpoint of another object never loads.
        for mutation in 0..4 {
            let mut drifted = access.clone();
            match mutation {
                0 => {
                    drifted.reference.provider_product_key = insar_core::provider_product_key(
                        "usgs.eros.m2m",
                        "landsat_ot_c2_l2",
                        "LC81230322026182LGN00",
                    )
                    .unwrap();
                }
                1 => drifted.reference.asset_key = "another-option".to_owned(),
                2 => {
                    drifted.file_name =
                        Some("LC08_L2SP_125032_20261005_20261009_02_T1.tar".to_owned())
                }
                // The D806 advisory filesize must NOT change the binding: it
                // is advisory (live B13 2026-10-31: advisory 957,283,399 vs
                // real 957,306,880) — assert stability, not drift, below.
                _ => {
                    drifted.expected_size_bytes = Some(957_306_881);
                    continue;
                }
            }
            let drifted_binding = super::durable_recovery_identity(
                "usgs-recovery-task",
                "usgs-recovery-item",
                policy_id,
                &drifted,
                crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
            )
            .unwrap()
            .expect("the drifted item still binds");
            assert_ne!(
                a, drifted_binding,
                "a different frozen object must not share prefix evidence (mutation {mutation})"
            );
        }
        // The advisory size differs but the identity does not: same binding.
        let mut advisory_drift = access.clone();
        advisory_drift.expected_size_bytes = Some(957_283_399);
        let advisory_binding = super::durable_recovery_identity(
            "usgs-recovery-task",
            "usgs-recovery-item",
            policy_id,
            &advisory_drift,
            crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        )
        .unwrap()
        .expect("the advisory-drifted item still binds");
        assert_eq!(
            a, advisory_binding,
            "an advisory filesize must not change the binding"
        );
        // The existing MPC binding keeps its whole-frozen-access shape and
        // other policies stay closed.
        let mpc = super::durable_recovery_identity(
            "task",
            "item",
            policy_id,
            access,
            crate::AuthRefreshPolicyId::PlanetaryComputerSasV1,
        )
        .unwrap();
        assert_eq!(mpc.as_deref().map(str::len), Some(64));
        assert!(
            super::durable_recovery_identity(
                "task",
                "item",
                policy_id,
                access,
                crate::AuthRefreshPolicyId::EarthdataStandardV1,
            )
            .unwrap()
            .is_none(),
            "policies without a re-resolved URL keep the gate closed"
        );
    }

    /// B13 entry-point coverage: drive the same execution path the Landsat
    /// bundle task uses — worker classification plus
    /// `execute_resolver_authoritative` with the real `HttpTransfer` — against
    /// a local fixture server that answers like dds.cr.usgs.gov (no strong
    /// ETag, Last-Modified present, Range honoured). The first execution is
    /// interrupted mid-transfer, the second (after a simulated app restart)
    /// must resume from the durable checkpoint instead of restarting at zero.
    #[test]
    fn usgs_bundle_entry_point_resumes_after_restart_through_the_real_transfer() {
        use crate::asset_resolver::{
            AssetResolverRegistry, ResolveContext, ResolveError, ResolvedAsset, ResolvedMethod,
        };
        use crate::{HttpTransfer, TransferOptions, TransferUpdate};
        use insar_core::{AssetAccessDescriptor, AssetAccessScheme, FrozenAssetAccess};
        use std::io::{BufRead, BufReader};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};

        // The 6-byte fixture object is served with Last-Modified and no ETag,
        // the shape recorded for dds.cr.usgs.gov object responses.
        struct NoCredentials;
        impl crate::CredentialProvider for NoCredentials {
            fn resolve(
                &self,
                _profile: &str,
            ) -> Result<crate::CredentialMaterial, crate::CredentialProviderError> {
                Err(crate::CredentialProviderError::Missing)
            }
        }

        use std::io::Write;

        /// One-shot concurrent fixture server. The response thread is spawned
        /// BEFORE the synchronous transfer runs, or the transfer would block
        /// forever waiting for an answer (the first executed request is
        /// answered by the thread, not by the test body).
        struct FixtureServer {
            handle: Option<std::thread::JoinHandle<String>>,
            port: u16,
        }
        impl FixtureServer {
            fn spawn(status_line: &'static str, headers: &'static str, body: &'static str) -> Self {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let port = listener.local_addr().unwrap().port();
                let handle = std::thread::spawn(move || {
                    listener.set_nonblocking(true).unwrap();
                    let end = Instant::now() + Duration::from_secs(20);
                    let stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(Instant::now() < end, "expected transfer request missing");
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            Err(error) => panic!("{error}"),
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(20)))
                        .unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut request = String::new();
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        request.push_str(&line);
                    }
                    let response = format!(
                        "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
                        body.len()
                    );
                    reader.into_inner().write_all(response.as_bytes()).unwrap();
                    request.to_ascii_lowercase()
                });
                Self {
                    handle: Some(handle),
                    port,
                }
            }
            fn base_url(&self) -> String {
                format!("http://127.0.0.1:{}/object.tar", self.port)
            }
            fn join(mut self) -> String {
                self.handle.take().unwrap().join().unwrap()
            }
        }

        /// A resolver under the reviewed V2 bundle resolver id that returns
        /// per-call signed URLs pointing at the local fixture server.
        struct SignedUrlResolver {
            base_url: String,
            calls: std::sync::atomic::AtomicUsize,
        }
        impl crate::AssetResolver for SignedUrlResolver {
            fn resolver_id(&self) -> &'static str {
                crate::USGS_LANDSAT_C2_L2_RESOLVER_ID_V2
            }
            fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
                &[AssetAccessScheme::UsgsM2m]
            }
            fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
                descriptor.scheme == AssetAccessScheme::UsgsM2m
                    && descriptor.resolver_id == self.resolver_id()
            }
            fn resolve(
                &self,
                _reference: &insar_core::StableAssetReference,
                _descriptor: &AssetAccessDescriptor,
                _context: &ResolveContext<'_>,
            ) -> Result<ResolvedAsset, ResolveError> {
                let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                Ok(ResolvedAsset::new(
                    &format!("{}?sig=fixture-call-{call}", self.base_url),
                    ResolvedMethod::Get,
                )?
                .with_expected_size(Some(6)))
            }
            fn plan_shadow(
                &self,
                _frozen: &FrozenAssetAccess,
                _context: &crate::asset_resolver::ShadowResolveContext<'_>,
            ) -> Result<crate::asset_resolver::ShadowAccessIntent, ResolveError> {
                Err(ResolveError::failed())
            }
        }

        let root =
            std::env::temp_dir().join(format!("insar-usgs-entry-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let destination = root.join("LC09_L2SP_123045_20260101_20260103_02_T1.tar");

        // Execution 1: the full 6-byte object is offered; the transfer is
        // cancelled after 3 bytes (chunk_size 3), simulating the app stop.
        // The response thread is spawned first so the synchronous transfer
        // finds an answering peer.
        let server = FixtureServer::spawn(
            "200 OK",
            "Last-Modified: Wed, 21 Oct 2026 07:28:00 GMT\r\n",
            "abcdef",
        );
        let resolver = std::sync::Arc::new(SignedUrlResolver {
            base_url: server.base_url(),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut registry = AssetResolverRegistry::new();
        registry
            .register(std::sync::Arc::clone(&resolver) as std::sync::Arc<dyn crate::AssetResolver>)
            .unwrap();
        let network_policy = insar_core::ProviderNetworkPolicy::system();
        let usgs = std::sync::Arc::new(
            insar_provider::usgs_m2m::UsgsM2mSession::authenticated_for_tests(network_policy)
                .unwrap(),
        );
        let runtime = super::ResolverExecutionRuntime::with_dependencies_and_sessions(
            registry,
            std::sync::Arc::new(NoCredentials),
            std::sync::Arc::new(crate::SystemClock),
            network_policy,
            None,
            Some(usgs),
            None,
        );
        let transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            chunk_size: 3,
            max_attempts: 1,
            timeout: Duration::from_secs(8),
            backoff: Duration::ZERO,
            ..TransferOptions::default()
        })
        .unwrap();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let cancel_probe = std::sync::Arc::clone(&cancel);
        let first = runtime
            .execute_resolver_authoritative(
                &transfer,
                &usgs_eligible_fixture(),
                "usgs-entry-task",
                "usgs-entry-item",
                destination.clone(),
                &|| cancel_probe.load(Ordering::SeqCst),
                &|| false,
                &mut |update| {
                    if let TransferUpdate::Progress(progress) = update {
                        if progress.bytes_completed == 3 {
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                },
            )
            .unwrap();
        assert_eq!(first.outcome, insar_core::DownloadOutcome::Interrupted);
        let first_request = server.join();
        assert!(!first_request.contains("range:"));
        assert_eq!(
            std::fs::read(crate::part_path(&destination)).unwrap(),
            b"abc"
        );
        assert!(
            destination
                .parent()
                .unwrap()
                .join("LC09_L2SP_123045_20260101_20260103_02_T1.tar.part.resume.json")
                .exists(),
            "the interrupted prefix must leave durable checkpoint evidence"
        );
        drop(runtime);
        drop(transfer);

        // App restart: a brand-new session and transfer. The re-resolved URL
        // is different (new signature), the object is the same
        // (same Last-Modified, Range honoured) — the checkpoint must resume.
        let server = FixtureServer::spawn(
            "206 Partial Content",
            "Content-Range: bytes 3-5/6\r\nLast-Modified: Wed, 21 Oct 2026 07:28:00 GMT\r\n",
            "def",
        );
        let resolver = std::sync::Arc::new(SignedUrlResolver {
            base_url: server.base_url(),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut registry = AssetResolverRegistry::new();
        registry
            .register(std::sync::Arc::clone(&resolver) as std::sync::Arc<dyn crate::AssetResolver>)
            .unwrap();
        let usgs = std::sync::Arc::new(
            insar_provider::usgs_m2m::UsgsM2mSession::authenticated_for_tests(network_policy)
                .unwrap(),
        );
        let runtime = super::ResolverExecutionRuntime::with_dependencies_and_sessions(
            registry,
            std::sync::Arc::new(NoCredentials),
            std::sync::Arc::new(crate::SystemClock),
            network_policy,
            None,
            Some(usgs),
            None,
        );
        let transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            chunk_size: 3,
            max_attempts: 1,
            timeout: Duration::from_secs(8),
            backoff: Duration::ZERO,
            ..TransferOptions::default()
        })
        .unwrap();
        let second = runtime
            .execute_resolver_authoritative(
                &transfer,
                &usgs_eligible_fixture(),
                // Deliberately different per-restart inputs from the first
                // execution: the binding must depend only on frozen stable
                // identity, so the saved checkpoint still resumes (live B13
                // evidence 2026-10-31).
                "usgs-entry-task-restarted",
                "usgs-entry-item-restarted",
                destination.clone(),
                &|| false,
                &|| false,
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(
            second.outcome,
            insar_core::DownloadOutcome::Completed,
            "{second:?}"
        );
        let second_request = server.join();
        assert!(
            second_request.contains("range: bytes=3-\r\n"),
            "the restart must resume from the checkpoint, not restart at zero"
        );
        assert!(
            second_request.contains("if-range: wed, 21 oct 2026 07:28:00 gmt\r\n"),
            "the probe must carry the Last-Modified validator"
        );
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"abcdef",
            "the completed file is the exact fixture object"
        );
        assert!(!crate::part_path(&destination).exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
