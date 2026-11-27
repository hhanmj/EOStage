use std::sync::Mutex;

use insar_core::{
    AccountId, AssetAccessScheme, CredentialRequirement, FrozenSearchBatch, ResourceRegistry,
    SnapshotItem, TypedError,
};
use insar_download::{
    desktop_bhoonidhi_readiness, desktop_cdse_status, desktop_earthdata_readiness,
    desktop_gacos_status, desktop_opentopography_status, desktop_usgs_status,
    resolve_desktop_cdse_login, resolve_desktop_usgs_login, BhoonidhiCredentialReadiness,
    DesktopCredentialError, EarthdataCredentialReadiness, BHOONIDHI_CREDENTIAL_PROFILE,
    EARTHDATA_BURST_CREDENTIAL_PROFILE,
};
use serde::{Deserialize, Serialize};

use crate::{
    ApplicationService, ApplicationServiceError, BusinessRuntimeFactory, CdseLoginRequest,
    UsgsM2mLoginRequest,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    LoggedIn,
    LoginRequired,
    CredentialsMissing,
    ServiceUnavailable,
    NotRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountSessionStatus {
    pub provider: String,
    pub state: AccountState,
}

/// Serializes readiness checks and session establishment for one provider. The
/// owner holds neither credential material nor copies of the provider session.
#[derive(Default)]
pub(crate) struct AccountSessionGate {
    lane: Mutex<()>,
}

impl AccountSessionGate {
    pub(crate) fn ensure<E>(
        &self,
        status: impl FnOnce() -> Result<bool, E>,
        establish: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), E> {
        let _guard = self
            .lane
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if status()? {
            Ok(())
        } else {
            establish()
        }
    }
}

fn saved_login_state(status: &str) -> AccountState {
    if status.starts_with("login:") {
        AccountState::LoginRequired
    } else if status == "unavailable" {
        AccountState::ServiceUnavailable
    } else {
        AccountState::CredentialsMissing
    }
}

fn saved_login_for_admission(
    status: &str,
    provider: &'static str,
) -> Result<(), ApplicationServiceError> {
    match saved_login_state(status) {
        AccountState::LoginRequired => Ok(()),
        AccountState::ServiceUnavailable => {
            Err(ApplicationServiceError::AccountServiceUnavailable { provider })
        }
        _ => Err(ApplicationServiceError::AccountLoginRequired { provider }),
    }
}

fn saved_login_resolve_error(
    error: DesktopCredentialError,
    provider: &'static str,
) -> ApplicationServiceError {
    match error {
        DesktopCredentialError::StoreUnavailable => {
            ApplicationServiceError::AccountServiceUnavailable { provider }
        }
        DesktopCredentialError::InvalidInput => {
            ApplicationServiceError::AccountLoginRequired { provider }
        }
    }
}

/// Establishes or refreshes the session of one account before a task is
/// admitted. Production uses the saved-credential services; tests inject fakes.
/// `Ok(Some(warning))` admits the task although the session could not be
/// established yet; the worker establishes it itself.
pub(crate) trait AccountSessionPort {
    fn ensure_account_session(
        &self,
        account: &AccountId,
    ) -> Result<Option<String>, ApplicationServiceError>;
}

/// The unified `create_task` admission check installed in W2's seam.
pub(crate) struct AccountSessionPreflight<'a, P: ?Sized> {
    port: &'a P,
    warning: Mutex<Option<String>>,
}

impl<'a, P: ?Sized> AccountSessionPreflight<'a, P> {
    pub(crate) fn new(port: &'a P) -> Self {
        Self {
            port,
            warning: Mutex::new(None),
        }
    }
}

impl<P: AccountSessionPort + Sync + ?Sized> crate::TaskAdmissionPreflight
    for AccountSessionPreflight<'_, P>
{
    fn check(
        &self,
        batch: &FrozenSearchBatch,
        selected_items: &[SnapshotItem],
        item_ids: &[String],
    ) -> Result<(), TypedError> {
        let warnings = preflight(self.port, batch, selected_items, item_ids)?;
        *self
            .warning
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            (!warnings.is_empty()).then(|| warnings.join("；"));
        Ok(())
    }

    fn admission_warning(&self) -> Option<String> {
        self.warning
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

pub(crate) fn account_display_name(account: &AccountId) -> &'static str {
    match account {
        AccountId::Cdse => "CDSE",
        AccountId::UsgsM2m => "USGS",
        AccountId::Earthdata => "Earthdata",
        AccountId::Bhoonidhi => "Bhoonidhi",
        AccountId::Mpc => "MPC",
        AccountId::Opentopography => "OpenTopography",
        AccountId::Gacos => "GACOS",
    }
}

fn provider_account(provider_id: &str) -> Option<AccountId> {
    match provider_id {
        "asf.nasa.earthdata" | "asf.catalog" => Some(AccountId::Earthdata),
        "copernicus.dataspace.odata" => Some(AccountId::Cdse),
        "usgs.eros.m2m" => Some(AccountId::UsgsM2m),
        "isro.nrsc.bhoonidhi" => Some(AccountId::Bhoonidhi),
        "opentopography" => Some(AccountId::Opentopography),
        "gacos.webform" => Some(AccountId::Gacos),
        _ => None,
    }
}

fn preflight_invalid(message: &str) -> TypedError {
    TypedError {
        code: "V3_APPLICATION_INVALID_REQUEST".to_owned(),
        message_zh: message.to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    }
}

/// The account required by the batch's provider route in the registry. Any
/// credentialed route of the provider counts: search may be anonymous while
/// transfer is not, and the transfer route is chosen later by the resolver.
fn batch_route_account(batch: &FrozenSearchBatch) -> Result<Option<AccountId>, TypedError> {
    let resource_id = crate::unified_sources::source_for_batch(batch)
        .ok_or_else(|| preflight_invalid("无法验证该批次的数据源身份"))?;
    let registry =
        ResourceRegistry::embedded_ref().map_err(|_| preflight_invalid("数据源注册表不可用"))?;
    let resource = registry
        .resource(&resource_id)
        .ok_or_else(|| preflight_invalid("数据源不存在"))?;
    let credentialed = resource.common().provider_routes.iter().any(|route| {
        route.provider_id == batch.provider_id
            && route.credential_requirement != CredentialRequirement::None
    });
    if !credentialed {
        return Ok(None);
    }
    provider_account(&batch.provider_id)
        .map(Some)
        .ok_or_else(|| preflight_invalid("无法确认该数据源所需的账号"))
}

/// Accounts implied by frozen asset access, in a fixed order.
pub(crate) fn item_accounts<'a>(
    items: impl IntoIterator<Item = &'a SnapshotItem>,
) -> Vec<AccountId> {
    let mut accounts = Vec::new();
    for item in items {
        let Some(access) = &item.asset_access else {
            continue;
        };
        let account = match access.descriptor.scheme {
            AssetAccessScheme::OauthBearerHttps
                if item.provider_id == "copernicus.dataspace.odata" =>
            {
                Some(AccountId::Cdse)
            }
            AssetAccessScheme::UsgsM2m => Some(AccountId::UsgsM2m),
            _ if access.descriptor.credential_profile.as_deref()
                == Some(BHOONIDHI_CREDENTIAL_PROFILE) =>
            {
                Some(AccountId::Bhoonidhi)
            }
            AssetAccessScheme::EarthdataHttps => Some(AccountId::Earthdata),
            _ if access.descriptor.credential_profile.as_deref()
                == Some(EARTHDATA_BURST_CREDENTIAL_PROFILE) =>
            {
                Some(AccountId::Earthdata)
            }
            _ => None,
        };
        if let Some(account) = account {
            if !accounts.contains(&account) {
                accounts.push(account);
            }
        }
    }
    let order = [
        AccountId::Cdse,
        AccountId::UsgsM2m,
        AccountId::Earthdata,
        AccountId::Bhoonidhi,
    ];
    accounts.sort_by_key(|account| order.iter().position(|known| known == account));
    accounts
}

/// Codes that mean the network or the remote service, not the credential,
/// failed. They are checked first: CDSE's `CDSE_OAUTH_TRANSPORT_FAILED`
/// contains "AUTH" but must never ask the user to log in again.
fn network_class(code: &str) -> bool {
    [
        "NETWORK",
        "TRANSPORT",
        "TIMEOUT",
        "UNREACHABLE",
        "RESPONSE_INVALID",
        "SYNCHRONIZATION",
        "RATE_LIMITED",
    ]
    .iter()
    .any(|fragment| code.contains(fragment))
}

/// Codes that mean the remote service explicitly refused the credential.
fn credential_rejected(code: &str) -> bool {
    (code.ends_with("_REJECTED") && !code.contains("REDIRECT"))
        || code.ends_with("_LOGIN_INVALID")
        || code.starts_with("USGS_M2M_AUTH_")
        || code.ends_with("_UNAUTHORIZED")
        || code.contains("CREDENTIALS_MISSING")
        || code.contains("INVALID_GRANT")
}

/// Map a session failure to the typed admission error. Missing or rejected
/// credentials name the account so the new UI can offer login in place.
fn admission_error(account: &AccountId, error: ApplicationServiceError) -> TypedError {
    let name = account_display_name(account);
    let code = error.code().to_owned();
    let (message_zh, retryable, needs_account) = match &error {
        ApplicationServiceError::AccountLoginRequired { .. } => {
            (error.to_string(), false, Some(account.clone()))
        }
        ApplicationServiceError::AccountServiceUnavailable { .. } => {
            (error.to_string(), true, None)
        }
        _ if network_class(&code) => (format!("暂时无法连接 {name}，请稍后重试建任务"), true, None),
        _ if credential_rejected(&code) => (
            format!("需要登录 {name} 后才能继续"),
            false,
            Some(account.clone()),
        ),
        _ => (
            format!("暂不能确认 {name} 账号会话，请稍后重试"),
            true,
            None,
        ),
    };
    TypedError {
        code,
        message_zh,
        retryable,
        needs_account,
        needs_user_confirmation: false,
    }
}

/// Admission outcome of establishing the shared Earthdata session. `Some` is a
/// warning: the task is admitted and the worker establishes the session.
fn earthdata_session_admission(
    result: Result<bool, insar_core::DownloadFailure>,
) -> Result<Option<String>, ApplicationServiceError> {
    match result {
        Ok(_) => Ok(None),
        Err(failure) if failure.code == "DL004" => {
            Err(ApplicationServiceError::AccountLoginRequired {
                provider: "Earthdata",
            })
        }
        // AUTH-REG-001: only an explicit refusal of the credential blocks
        // admission. Transport failures and rate limits do not; the worker
        // establishes the session and refreshes once on 401.
        Err(_) => Ok(Some(
            "暂未能建立 Earthdata 会话（网络或服务限流），任务已创建，下载开始时将重新建立会话"
                .to_owned(),
        )),
    }
}

/// Earthdata state for the account status query. `established` is whether the
/// worker currently holds the shared session; a held session is usable even
/// while the saved credential cannot be read.
fn earthdata_account_state(
    readiness: EarthdataCredentialReadiness,
    established: bool,
) -> AccountState {
    if established {
        return AccountState::LoggedIn;
    }
    match readiness {
        EarthdataCredentialReadiness::Ready => AccountState::LoginRequired,
        EarthdataCredentialReadiness::Missing | EarthdataCredentialReadiness::Invalid => {
            AccountState::CredentialsMissing
        }
        EarthdataCredentialReadiness::Unavailable => AccountState::ServiceUnavailable,
    }
}

/// Earthdata task admission against the worker's shared session owner.
/// `running` is the owner of an already running runtime (no startup);
/// `start` starts the runtime and returns its owner. A session held by the
/// running worker admits at once: no credential read, no request.
fn earthdata_account_admission(
    running: Option<&dyn insar_download::EarthdataSessionOwner>,
    readiness: impl FnOnce() -> EarthdataCredentialReadiness,
    start: impl FnOnce() -> Result<
        Option<std::sync::Arc<dyn insar_download::EarthdataSessionOwner>>,
        ApplicationServiceError,
    >,
) -> Result<Option<String>, ApplicationServiceError> {
    if running.is_some_and(|owner| owner.earthdata_session_established()) {
        return Ok(None);
    }
    match readiness() {
        EarthdataCredentialReadiness::Ready => {}
        EarthdataCredentialReadiness::Unavailable => {
            return Err(ApplicationServiceError::AccountServiceUnavailable {
                provider: "Earthdata",
            })
        }
        _ => {
            return Err(ApplicationServiceError::AccountLoginRequired {
                provider: "Earthdata",
            })
        }
    }
    match start()? {
        Some(owner) => earthdata_session_admission(owner.ensure_earthdata_session()),
        None => Ok(None),
    }
}

/// `account_sessions::preflight`: establish every account session the selected
/// items need, before any task row exists. No Store or runtime lock is held.
/// Returns the warnings of sessions left to the worker.
pub(crate) fn preflight<P: AccountSessionPort + ?Sized>(
    port: &P,
    batch: &FrozenSearchBatch,
    selected_items: &[SnapshotItem],
    item_ids: &[String],
) -> Result<Vec<String>, TypedError> {
    let mut warnings = Vec::new();
    for account in &selection_accounts(batch, selected_items, item_ids)? {
        if let Some(warning) = port
            .ensure_account_session(account)
            .map_err(|error| admission_error(account, error))?
        {
            warnings.push(warning);
        }
    }
    Ok(warnings)
}

/// The accounts a frozen selection needs: frozen asset access of the selected
/// items plus the credentialed route of the batch provider. #22 B slice 2:
/// the selected items come from the bounded selection read, so the batch
/// header's (empty) candidate array is never scanned here.
pub(crate) fn selection_accounts(
    batch: &FrozenSearchBatch,
    selected_items: &[SnapshotItem],
    item_ids: &[String],
) -> Result<Vec<AccountId>, TypedError> {
    let selected = item_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    debug_assert!(
        selected_items
            .iter()
            .all(|item| selected.contains(item.item_id.as_str())),
        "the selection must name exactly the requested items"
    );
    let mut accounts = item_accounts(selected_items);
    if let Some(account) = batch_route_account(batch)? {
        if !accounts.contains(&account) {
            accounts.push(account);
        }
    }
    Ok(accounts)
}

/// DEM downloads need an account only on the OpenTopography route.
pub(crate) fn dem_provider_account(provider: &str) -> Option<AccountId> {
    match provider.trim().to_ascii_uppercase().as_str() {
        "OPENTOPOGRAPHY" => Some(AccountId::Opentopography),
        // NISAR DEM and ASTER GDEM v3 tiles live behind Earthdata Login: the
        // existing shared Earthdata account session is reused, not a new one.
        "NISAR_DEM_ASF" | "EARTHDATA" | "LPDAAC" | "ASF" => Some(AccountId::Earthdata),
        "COPERNICUS_AWS" | "FABDEM_PUBLIC" | "GEM15_PUBLIC" => None,
        _ => None,
    }
}

/// `account_session_statuses` names each account by its `AccountId` wire name.
fn account_status_name(account: &AccountId) -> &'static str {
    match account {
        AccountId::Cdse => "cdse",
        AccountId::UsgsM2m => "usgs_m2m",
        AccountId::Earthdata => "earthdata",
        AccountId::Bhoonidhi => "bhoonidhi",
        AccountId::Mpc => "mpc",
        AccountId::Opentopography => "opentopography",
        AccountId::Gacos => "gacos",
    }
}

fn accounts_ready(accounts: &[AccountId], statuses: &[AccountSessionStatus]) -> bool {
    accounts.iter().all(|account| {
        statuses.iter().any(|status| {
            status.provider == account_status_name(account)
                && matches!(
                    status.state,
                    AccountState::LoggedIn
                        | AccountState::LoginRequired
                        | AccountState::NotRequired
                )
        })
    })
}

fn project_account_statuses(
    cdse_state: AccountState,
    usgs_state: AccountState,
    earthdata_state: AccountState,
    bhoonidhi_state: AccountState,
    opentopography_status: &str,
    gacos_status: &str,
) -> Vec<AccountSessionStatus> {
    let opentopography_state = match opentopography_status {
        "set" => AccountState::LoggedIn,
        "unavailable" => AccountState::ServiceUnavailable,
        _ => AccountState::CredentialsMissing,
    };
    let gacos_state = match gacos_status {
        "none" => AccountState::CredentialsMissing,
        "unavailable" => AccountState::ServiceUnavailable,
        // Owner requirement (2026-11-12 #4): GACOS requires a notification email;
        // once saved it must project as Configured (LoggedIn), never NotRequired ("无需认证").
        _ => AccountState::LoggedIn,
    };
    vec![
        AccountSessionStatus {
            provider: "cdse".to_owned(),
            state: cdse_state,
        },
        AccountSessionStatus {
            provider: "usgs_m2m".to_owned(),
            state: usgs_state,
        },
        AccountSessionStatus {
            provider: "earthdata".to_owned(),
            state: earthdata_state,
        },
        AccountSessionStatus {
            provider: "bhoonidhi".to_owned(),
            state: bhoonidhi_state,
        },
        AccountSessionStatus {
            provider: "mpc".to_owned(),
            state: AccountState::NotRequired,
        },
        AccountSessionStatus {
            provider: "opentopography".to_owned(),
            state: opentopography_state,
        },
        AccountSessionStatus {
            provider: "gacos".to_owned(),
            state: gacos_state,
        },
    ]
}

impl ApplicationService {
    pub fn account_session_statuses(
        &self,
    ) -> Result<Vec<AccountSessionStatus>, ApplicationServiceError> {
        self.ensure_running()?;
        self.ensure_cdse_controls_available()?;
        let cdse = self.cdse_session_runtime()?;
        let usgs = self.usgs_m2m_session_runtime()?;
        let cdse_state = if cdse
            .status()
            .map_err(ApplicationServiceError::Cdse)?
            .authenticated
        {
            AccountState::LoggedIn
        } else {
            saved_login_state(&desktop_cdse_status())
        };
        let usgs_state = if usgs
            .status()
            .map_err(ApplicationServiceError::UsgsM2m)?
            .authenticated
        {
            AccountState::LoggedIn
        } else {
            saved_login_state(&desktop_usgs_status())
        };
        let earthdata_state = earthdata_account_state(
            desktop_earthdata_readiness(),
            self.earthdata_session_owner()
                .is_some_and(|owner| owner.earthdata_session_established()),
        );
        let bhoonidhi_state = match desktop_bhoonidhi_readiness() {
            BhoonidhiCredentialReadiness::Ready => {
                if self.bhoonidhi_session_status()?.authenticated {
                    AccountState::LoggedIn
                } else {
                    AccountState::LoginRequired
                }
            }
            BhoonidhiCredentialReadiness::Missing | BhoonidhiCredentialReadiness::Invalid => {
                AccountState::CredentialsMissing
            }
            BhoonidhiCredentialReadiness::Unavailable => AccountState::ServiceUnavailable,
        };
        Ok(project_account_statuses(
            cdse_state,
            usgs_state,
            earthdata_state,
            bhoonidhi_state,
            &desktop_opentopography_status(),
            &desktop_gacos_status(),
        ))
    }

    pub(crate) fn ensure_batch_items_for_admission(
        &self,
        batch_id: &str,
        selected_ids: &[String],
    ) -> Result<(), ApplicationServiceError> {
        if !matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            return Ok(());
        }
        let store = self.store_handle()?;
        let items = {
            let store = store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            // #22 B slice 2: the selected items only, never the whole frozen
            // candidate array. A missing batch is left to the entry's own
            // validation, as before.
            store
                .load_batch_selection(batch_id, selected_ids)?
                .map(|selection| selection.items)
                .unwrap_or_default()
        };
        self.ensure_accounts_for_items(&items)
    }

    pub(crate) fn ensure_dem_provider_for_admission(
        &self,
        provider: &str,
    ) -> Result<(), ApplicationServiceError> {
        match dem_provider_account(provider) {
            Some(account) => self.ensure_account_session(&account).map(|_| ()),
            None => Ok(()),
        }
    }

    /// Frozen-batch entries outside the unified seam (NISAR subset V2): the
    /// same accounts as `preflight`. A missing batch is left to the entry's
    /// own validation.
    pub(crate) fn ensure_frozen_selection_for_admission(
        &self,
        batch_id: &str,
        item_ids: &[String],
    ) -> Result<(), ApplicationServiceError> {
        if !matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            return Ok(());
        }
        let store = self.store_handle()?;
        let selection = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            // #22 B slice 2: bounded selection read — the header plus the
            // named products, not the whole candidate array.
            .load_batch_selection(batch_id, item_ids)?;
        let Some(selection) = selection else {
            return Ok(());
        };
        // An unverifiable source or registry failure must not be admitted as
        // "needs no account"; the error reaches the caller instead.
        let accounts = selection_accounts(&selection.batch, &selection.items, item_ids)
            .map_err(|error| crate::invalid_request(error.message_zh))?;
        for account in &accounts {
            self.ensure_account_session(account)?;
        }
        Ok(())
    }

    /// Whether every account a frozen selection needs (`selection_accounts`,
    /// the create_task preflight's set) is usable without user action: logged
    /// in, a saved credential the preflight logs in with, or not required.
    /// A missing batch or an unverifiable source is not ready. Read only: no
    /// session is established here.
    pub fn selection_accounts_ready(
        &self,
        batch_id: &str,
        item_ids: &[String],
    ) -> Result<bool, ApplicationServiceError> {
        let store = self.store_handle()?;
        let selection = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            // #22 B slice 2: bounded selection read, as above.
            .load_batch_selection(batch_id, item_ids)?;
        let Some(selection) = selection else {
            return Ok(false);
        };
        let Ok(accounts) = selection_accounts(&selection.batch, &selection.items, item_ids) else {
            return Ok(false);
        };
        if accounts.is_empty() {
            return Ok(true);
        }
        let statuses = self.account_session_statuses()?;
        Ok(accounts_ready(&accounts, &statuses))
    }

    /// The saved Earthdata login changed or was removed: retire the shared
    /// session so no transfer keeps using the previous account.
    pub fn earthdata_credentials_changed(&self) {
        if let Some(owner) = self.earthdata_session_owner() {
            owner.invalidate_earthdata_sessions();
        }
    }

    /// The running worker's session owner, without starting the runtime.
    fn earthdata_session_owner(
        &self,
    ) -> Option<std::sync::Arc<dyn insar_download::EarthdataSessionOwner>> {
        self.runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(|runtime| runtime.earthdata_sessions.clone())
    }

    pub(crate) fn ensure_burst_for_admission(&self) -> Result<(), ApplicationServiceError> {
        self.ensure_earthdata_for_admission()
    }

    /// Legacy entries have no warning field; a session left to the worker is
    /// admitted like the unified entry.
    pub(crate) fn ensure_earthdata_for_admission(&self) -> Result<(), ApplicationServiceError> {
        self.ensure_account_session(&AccountId::Earthdata)
            .map(|_| ())
    }

    /// Establish the Earthdata session the production worker's transfers share
    /// (Burst extractor and Earthdata archives), so the first transfer reuses
    /// it. Without a saved username/password there is nothing to establish.
    fn ensure_earthdata_session_for_admission(
        &self,
    ) -> Result<Option<String>, ApplicationServiceError> {
        let running = self.earthdata_session_owner();
        earthdata_account_admission(running.as_deref(), desktop_earthdata_readiness, || {
            self.with_business_runtime(|runtime| Ok(runtime.earthdata_sessions.clone()))
        })
    }

    pub(crate) fn ensure_task_session_for_admission(
        &self,
        task_id: &str,
    ) -> Result<(), ApplicationServiceError> {
        if !matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            return Ok(());
        }
        let store = self.store_handle()?;
        let items = {
            let store = store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store
                .load_task(task_id)?
                .map(|task| task.snapshot.candidates)
                .unwrap_or_default()
        };
        self.ensure_accounts_for_items(&items)
    }

    pub(crate) fn ensure_task_selection_for_admission(
        &self,
        task_id: &str,
        selected_ids: &[String],
    ) -> Result<(), ApplicationServiceError> {
        if !matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            return Ok(());
        }
        let store = self.store_handle()?;
        let (frozen, progressive_source) = {
            let store = store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let frozen = store.load_task(task_id)?.map(|task| task.snapshot);
            let source = if frozen.as_ref().is_some_and(|snapshot| {
                snapshot
                    .query
                    .get("delivery_kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("progressive_admission")
            }) {
                store.progressive_task_source_batch_id(task_id)?
            } else {
                None
            };
            (frozen, source)
        };
        if let Some(frozen) = frozen {
            let selected = selected_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>();
            self.ensure_accounts_for_items(
                frozen
                    .candidates
                    .iter()
                    .filter(|item| selected.contains(&item.item_id)),
            )?;
            self.ensure_batch_items_for_admission(
                progressive_source.as_deref().unwrap_or(&frozen.batch_id),
                selected_ids,
            )?;
        }
        Ok(())
    }

    pub(crate) fn ensure_workbench_selection_for_admission(
        &self,
        batch_id: &str,
    ) -> Result<(), ApplicationServiceError> {
        if !matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            return Ok(());
        }
        let store = self.store_handle()?;
        let (batch_id, selected) = {
            let store = store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let Some(workbench) = store.load_workbench()? else {
                return Ok(());
            };
            if workbench.batch.batch_id != batch_id {
                return Ok(());
            }
            (workbench.batch.batch_id, workbench.state.checked_item_ids)
        };
        self.ensure_batch_items_for_admission(&batch_id, &selected)
    }

    pub(crate) fn ensure_accounts_for_items<'a>(
        &self,
        items: impl IntoIterator<Item = &'a SnapshotItem>,
    ) -> Result<(), ApplicationServiceError> {
        if !matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            return Ok(());
        }
        for account in item_accounts(items) {
            self.ensure_account_session(&account)?;
        }
        Ok(())
    }

    fn ensure_saved_readiness(
        &self,
        provider: &'static str,
        ready: bool,
        unavailable: bool,
    ) -> Result<(), ApplicationServiceError> {
        if ready {
            Ok(())
        } else if unavailable {
            Err(ApplicationServiceError::AccountServiceUnavailable { provider })
        } else {
            Err(ApplicationServiceError::AccountLoginRequired { provider })
        }
    }

    pub(crate) fn ensure_cdse_for_admission(&self) -> Result<(), ApplicationServiceError> {
        self.ensure_cdse_controls_available()?;
        self.cdse_admission_gate.ensure(
            || {
                let runtime = self.cdse_session_runtime()?;
                let snapshot = runtime
                    .session
                    .snapshot()
                    .map_err(ApplicationServiceError::Cdse)?;
                if snapshot.authenticated {
                    runtime
                        .session
                        .refresh_if_expired()
                        .map_err(ApplicationServiceError::Cdse)?;
                }
                Ok(snapshot.authenticated)
            },
            || {
                saved_login_for_admission(&desktop_cdse_status(), "CDSE")?;
                let (username, password) = resolve_desktop_cdse_login()
                    .map_err(|error| saved_login_resolve_error(error, "CDSE"))?;
                self.cdse_login(CdseLoginRequest {
                    username: username.to_string(),
                    password: password.to_string(),
                    totp: None,
                })?;
                Ok(())
            },
        )
    }

    pub(crate) fn ensure_usgs_for_admission(&self) -> Result<(), ApplicationServiceError> {
        self.ensure_usgs_controls_available()?;
        self.usgs_admission_gate.ensure(
            || {
                Ok(self
                    .usgs_m2m_session_runtime()?
                    .status()
                    .map_err(ApplicationServiceError::UsgsM2m)?
                    .authenticated)
            },
            || {
                saved_login_for_admission(&desktop_usgs_status(), "USGS")?;
                let (username, application_token) = resolve_desktop_usgs_login()
                    .map_err(|error| saved_login_resolve_error(error, "USGS"))?;
                self.usgs_m2m_login(UsgsM2mLoginRequest {
                    username: username.to_string(),
                    application_token: application_token.to_string(),
                })?;
                Ok(())
            },
        )
    }
}

impl AccountSessionPort for ApplicationService {
    /// Fixture runtimes own no saved-credential services and are skipped like
    /// the legacy admission guards. CDSE and USGS sessions are established in
    /// the process-wide session the worker shares. Bhoonidhi only checks the
    /// saved credential: its worker logs in on its own system-proxy route, and
    /// AUTH-REG-001 forbids gating a configured credential on a separate probe.
    fn ensure_account_session(
        &self,
        account: &AccountId,
    ) -> Result<Option<String>, ApplicationServiceError> {
        if !matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            return Ok(None);
        }
        // A route change applied after this check would retire the session.
        self.apply_pending_network_policy();
        match account {
            AccountId::Cdse => self.ensure_cdse_for_admission().map(|()| None),
            AccountId::UsgsM2m => self.ensure_usgs_for_admission().map(|()| None),
            AccountId::Earthdata => self.ensure_earthdata_session_for_admission(),
            AccountId::Bhoonidhi => {
                let readiness = desktop_bhoonidhi_readiness();
                self.ensure_saved_readiness(
                    "Bhoonidhi",
                    readiness == BhoonidhiCredentialReadiness::Ready,
                    readiness == BhoonidhiCredentialReadiness::Unavailable,
                )
                .map(|()| None)
            }
            AccountId::Opentopography => {
                let status = desktop_opentopography_status();
                self.ensure_saved_readiness(
                    "OpenTopography",
                    status == "set",
                    status == "unavailable",
                )
                .map(|()| None)
            }
            AccountId::Gacos => {
                let status = desktop_gacos_status();
                self.ensure_saved_readiness(
                    "GACOS",
                    status != "none" && status != "unavailable",
                    status == "unavailable",
                )
                .map(|()| None)
            }
            AccountId::Mpc => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    /// W3f: the Agent gate's account readiness. A saved credential the
    /// preflight logs in with is ready; a missing credential, an unavailable
    /// service or an account the status list does not name is not.
    #[test]
    fn selection_accounts_are_ready_unless_a_credential_or_service_is_missing() {
        use super::{accounts_ready, AccountSessionStatus, AccountState};
        use insar_core::AccountId;
        let status = |provider: &str, state| AccountSessionStatus {
            provider: provider.to_owned(),
            state,
        };
        let statuses = vec![
            status("cdse", AccountState::LoggedIn),
            status("earthdata", AccountState::LoginRequired),
            status("usgs_m2m", AccountState::CredentialsMissing),
            status("bhoonidhi", AccountState::ServiceUnavailable),
            status("mpc", AccountState::NotRequired),
        ];
        assert!(accounts_ready(&[], &statuses));
        assert!(accounts_ready(
            &[AccountId::Cdse, AccountId::Earthdata, AccountId::Mpc],
            &statuses
        ));
        assert!(!accounts_ready(
            &[AccountId::Cdse, AccountId::UsgsM2m],
            &statuses
        ));
        assert!(!accounts_ready(&[AccountId::Bhoonidhi], &statuses));
        assert!(!accounts_ready(&[AccountId::Gacos], &statuses));
    }

    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn account_status_distinguishes_saved_missing_and_unavailable_credentials() {
        assert_eq!(saved_login_state("login:u***"), AccountState::LoginRequired);
        assert_eq!(saved_login_state("none"), AccountState::CredentialsMissing);
        assert_eq!(
            saved_login_state("unavailable"),
            AccountState::ServiceUnavailable
        );
    }

    #[test]
    fn nisar_dem_admission_requires_the_shared_earthdata_session() {
        // NISAR DEM tiles live behind Earthdata Login on the ASF cloud, so the
        // existing Earthdata account session must be reused; no new credential
        // store or provider account is introduced.
        assert_eq!(
            dem_provider_account("NISAR_DEM_ASF"),
            Some(AccountId::Earthdata)
        );
        assert_eq!(
            dem_provider_account("nisar_dem_asf"),
            Some(AccountId::Earthdata)
        );
        assert_eq!(dem_provider_account("COPERNICUS_AWS"), None);
        assert_eq!(dem_provider_account("FABDEM_PUBLIC"), None);
        assert_eq!(dem_provider_account("GEM15_PUBLIC"), None);
        assert_eq!(
            dem_provider_account("OPENTOPOGRAPHY"),
            Some(AccountId::Opentopography)
        );
    }

    #[test]
    fn status_projection_uses_account_ids_and_returns_no_credential_values() {
        let statuses = project_account_statuses(
            AccountState::LoginRequired,
            AccountState::CredentialsMissing,
            AccountState::ServiceUnavailable,
            AccountState::LoggedIn,
            "set",
            "none",
        );
        let providers = statuses
            .iter()
            .map(|status| status.provider.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            providers,
            [
                "cdse",
                "usgs_m2m",
                "earthdata",
                "bhoonidhi",
                "mpc",
                "opentopography",
                "gacos",
            ]
        );
        assert_eq!(statuses[5].state, AccountState::LoggedIn);
        assert_eq!(statuses[6].state, AccountState::CredentialsMissing);
        // Configured GACOS notification email projects as LoggedIn ("已配置"), not NotRequired ("无需认证").
        assert_eq!(
            project_account_statuses(
                AccountState::CredentialsMissing,
                AccountState::CredentialsMissing,
                AccountState::CredentialsMissing,
                AccountState::CredentialsMissing,
                "set",
                "m***@example.org",
            )[6]
            .state,
            AccountState::LoggedIn
        );
        assert_eq!(
            serde_json::to_value(&statuses[5]).expect("status serialization"),
            serde_json::json!({"provider": "opentopography", "state": "logged_in"})
        );
        assert_eq!(
            project_account_statuses(
                AccountState::CredentialsMissing,
                AccountState::CredentialsMissing,
                AccountState::CredentialsMissing,
                AccountState::CredentialsMissing,
                "none",
                "unavailable",
            )[5..]
                .iter()
                .map(|status| status.state)
                .collect::<Vec<_>>(),
            [
                AccountState::CredentialsMissing,
                AccountState::ServiceUnavailable
            ]
        );
    }

    #[test]
    fn unavailable_fake_credential_store_rejects_admission_as_service_unavailable() {
        let fake_vault_status = || "unavailable";
        for (provider, expected_code) in [
            ("CDSE", "CDSE_ACCOUNT_SERVICE_UNAVAILABLE"),
            ("USGS", "USGS_ACCOUNT_SERVICE_UNAVAILABLE"),
        ] {
            let gate = AccountSessionGate::default();
            let result = gate.ensure(
                || Ok::<_, ApplicationServiceError>(false),
                || saved_login_for_admission(fake_vault_status(), provider),
            );
            let error = result.expect_err("unavailable credential store must block admission");
            assert_eq!(
                error.code(),
                expected_code,
                "{provider} must not request login"
            );
            assert!(error.to_string().contains("凭据") || error.to_string().contains("凭证"));
            assert_eq!(
                saved_login_state(fake_vault_status()),
                AccountState::ServiceUnavailable
            );
            let resolve_error =
                saved_login_resolve_error(DesktopCredentialError::StoreUnavailable, provider);
            assert_eq!(resolve_error.code(), expected_code);
            assert_eq!(
                saved_login_for_admission("none", provider)
                    .expect_err("missing credential requires login")
                    .code(),
                format!("{provider}_ACCOUNT_LOGIN_REQUIRED")
            );
            saved_login_for_admission("login:u***", provider)
                .expect("saved credentials should still proceed to session establishment");
        }
    }

    #[test]
    fn saved_login_is_established_once_for_concurrent_task_admission() {
        let gate = Arc::new(AccountSessionGate::default());
        let logged_in = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(3));
        let mut clients = Vec::new();
        for _ in 0..2 {
            let (gate, logged_in, attempts, start) = (
                Arc::clone(&gate),
                Arc::clone(&logged_in),
                Arc::clone(&attempts),
                Arc::clone(&start),
            );
            clients.push(thread::spawn(move || {
                start.wait();
                gate.ensure(
                    || Ok::<_, &'static str>(logged_in.load(Ordering::Acquire)),
                    || {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        logged_in.store(true, Ordering::Release);
                        Ok(())
                    },
                )
            }));
        }
        start.wait();
        for client in clients {
            client.join().expect("admission thread").expect("ready");
        }
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_login_must_reject_admission_without_marking_session_ready() {
        let gate = AccountSessionGate::default();
        let result = gate.ensure(
            || Ok::<_, &'static str>(false),
            || Err("fixture-login-rejected"),
        );
        assert_eq!(result, Err("fixture-login-rejected"));
    }

    #[test]
    fn network_failures_during_admission_are_retryable_not_a_login_request() {
        use insar_download::CdseAuthError;
        for error in [
            CdseAuthError::Transport,
            CdseAuthError::Timeout,
            CdseAuthError::ResponseInvalid,
            CdseAuthError::Synchronization,
        ] {
            let typed = admission_error(&AccountId::Cdse, ApplicationServiceError::Cdse(error));
            assert_eq!(typed.needs_account, None, "{}", error.code());
            assert!(typed.retryable, "{}", error.code());
            assert!(!typed.message_zh.contains("需要登录"), "{}", error.code());
        }
        for error in [CdseAuthError::Rejected, CdseAuthError::LoginMaterialInvalid] {
            let typed = admission_error(&AccountId::Cdse, ApplicationServiceError::Cdse(error));
            assert_eq!(
                typed.needs_account,
                Some(AccountId::Cdse),
                "{}",
                error.code()
            );
            assert!(!typed.retryable, "{}", error.code());
        }
    }

    #[test]
    fn only_a_rejected_earthdata_credential_blocks_admission() {
        let failure = |code: &str, retryable: bool| insar_core::DownloadFailure {
            code: code.to_owned(),
            reason_zh: "fixture".to_owned(),
            retryable,
        };
        // Transport failures and rate limits (both DL005) admit the task with a
        // warning; the worker establishes the session and refreshes on 401.
        for transient in [failure("DL005", true), failure("DL005", false)] {
            let warning = earthdata_session_admission(Err(transient))
                .expect("transport failure must not block admission");
            assert!(warning.is_some_and(|text| !text.is_empty()));
        }
        assert_eq!(earthdata_session_admission(Ok(true)).unwrap(), None);
        assert_eq!(
            earthdata_session_admission(Err(failure("DL004", false)))
                .expect_err("rejected credential")
                .code(),
            "EARTHDATA_ACCOUNT_LOGIN_REQUIRED"
        );
    }

    #[test]
    fn existing_session_never_reads_saved_credentials_again() {
        let gate = AccountSessionGate::default();
        gate.ensure(
            || Ok::<_, &'static str>(true),
            || panic!("unnecessary login"),
        )
        .expect("ready session");
    }

    /// Worker session owner whose `ensure` stands for one token request.
    #[derive(Default)]
    struct FakeEarthdataOwner {
        established: AtomicBool,
        token_requests: AtomicUsize,
    }

    impl insar_download::EarthdataSessionOwner for FakeEarthdataOwner {
        fn ensure_earthdata_session(&self) -> Result<bool, insar_core::DownloadFailure> {
            if !self.established.load(Ordering::Acquire) {
                self.token_requests.fetch_add(1, Ordering::SeqCst);
                self.established.store(true, Ordering::Release);
            }
            Ok(true)
        }

        fn invalidate_earthdata_sessions(&self) {
            self.established.store(false, Ordering::Release);
        }

        fn earthdata_session_established(&self) -> bool {
            self.established.load(Ordering::Acquire)
        }
    }

    #[test]
    fn a_second_task_admission_reuses_the_established_earthdata_session() {
        use insar_download::EarthdataSessionOwner;
        // Held concretely so the token-request count can be asserted.
        let owner = Arc::new(FakeEarthdataOwner::default());
        let fake: Arc<dyn insar_download::EarthdataSessionOwner> = owner.clone();
        let readiness_reads = AtomicUsize::new(0);
        let starts = AtomicUsize::new(0);
        let admit = |running: Option<&dyn insar_download::EarthdataSessionOwner>| {
            earthdata_account_admission(
                running,
                || {
                    readiness_reads.fetch_add(1, Ordering::SeqCst);
                    EarthdataCredentialReadiness::Ready
                },
                || {
                    starts.fetch_add(1, Ordering::SeqCst);
                    Ok(Some(Arc::clone(&fake)))
                },
            )
        };
        // First task: no runtime yet, the session is established once.
        assert_eq!(admit(None).expect("first admission"), None);
        // Later tasks in the same session: no credential read, no startup, no
        // token request.
        for _ in 0..3 {
            assert_eq!(admit(Some(owner.as_ref())).expect("reuse"), None);
        }
        assert_eq!(readiness_reads.load(Ordering::SeqCst), 1);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(owner.token_requests.load(Ordering::SeqCst), 1);
        assert!(owner.earthdata_session_established());

        // A retired session (credential changed) is established again.
        owner.invalidate_earthdata_sessions();
        assert_eq!(admit(Some(owner.as_ref())).expect("re-establish"), None);
        assert_eq!(readiness_reads.load(Ordering::SeqCst), 2);
        assert_eq!(owner.token_requests.load(Ordering::SeqCst), 2);

        // Without a saved credential the request is refused before startup.
        let error = earthdata_account_admission(
            None,
            || EarthdataCredentialReadiness::Missing,
            || panic!("no runtime startup without a credential"),
        )
        .expect_err("missing credential");
        assert_eq!(error.code(), "EARTHDATA_ACCOUNT_LOGIN_REQUIRED");
    }

    #[test]
    fn earthdata_status_follows_the_shared_session() {
        use EarthdataCredentialReadiness as R;
        assert_eq!(
            earthdata_account_state(R::Ready, true),
            AccountState::LoggedIn
        );
        assert_eq!(
            earthdata_account_state(R::Ready, false),
            AccountState::LoginRequired
        );
        // A held session stays usable even if the vault is briefly unreadable.
        assert_eq!(
            earthdata_account_state(R::Unavailable, true),
            AccountState::LoggedIn
        );
        assert_eq!(
            earthdata_account_state(R::Unavailable, false),
            AccountState::ServiceUnavailable
        );
        for readiness in [R::Missing, R::Invalid] {
            assert_eq!(
                earthdata_account_state(readiness, false),
                AccountState::CredentialsMissing
            );
        }
    }
}
