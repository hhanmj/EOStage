use std::fmt;
use std::time::{Duration, SystemTime};

use insar_core::ProxyPolicy;

use super::types::{ResolvedAuthorization, SecretValue};
use super::ResolveError;

pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeoutPolicy {
    pub connect: Duration,
    pub operation: Duration,
}

impl TimeoutPolicy {
    pub fn new(connect: Duration, operation: Duration) -> Result<Self, ResolveError> {
        if connect.is_zero() || operation.is_zero() || connect > operation {
            return Err(ResolveError::descriptor_invalid());
        }
        Ok(Self { connect, operation })
    }
}

impl Default for TimeoutPolicy {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(30),
            operation: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct AuthRefreshBudget {
    remaining: u8,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AuthRefreshPermit {
    _reserved: (),
}

impl AuthRefreshBudget {
    pub const fn new(remaining: u8) -> Self {
        Self { remaining }
    }

    pub fn try_consume(&mut self) -> Result<AuthRefreshPermit, ResolveError> {
        if self.remaining == 0 {
            return Err(ResolveError::refresh_budget_exhausted());
        }
        self.remaining -= 1;
        Ok(AuthRefreshPermit { _reserved: () })
    }

    pub const fn remaining(&self) -> u8 {
        self.remaining
    }
}

impl Default for AuthRefreshBudget {
    fn default() -> Self {
        Self::new(1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialProviderError {
    Missing,
    Rejected,
    Timeout,
    Failed,
    BurstCookieRequired,
}

pub struct CredentialMaterial(CredentialMaterialKind);

enum CredentialMaterialKind {
    Bearer(SecretValue),
    Basic {
        username: SecretValue,
        password: SecretValue,
    },
    Cookie {
        name: String,
        value: SecretValue,
    },
}

impl CredentialMaterial {
    pub fn bearer(value: impl Into<String>) -> Result<Self, CredentialProviderError> {
        SecretValue::new(value)
            .map(|value| Self(CredentialMaterialKind::Bearer(value)))
            .map_err(|_| CredentialProviderError::Failed)
    }

    pub fn basic(
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, CredentialProviderError> {
        Ok(Self(CredentialMaterialKind::Basic {
            username: SecretValue::new(username).map_err(|_| CredentialProviderError::Failed)?,
            password: SecretValue::new(password).map_err(|_| CredentialProviderError::Failed)?,
        }))
    }

    pub fn cookie(
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, CredentialProviderError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(CredentialProviderError::Failed);
        }
        Ok(Self(CredentialMaterialKind::Cookie {
            name,
            value: SecretValue::new(value).map_err(|_| CredentialProviderError::Failed)?,
        }))
    }

    pub fn apply_to(self, asset: &mut super::ResolvedAsset) -> Result<(), ResolveError> {
        match self.0 {
            CredentialMaterialKind::Bearer(value) => {
                asset.set_authorization(ResolvedAuthorization::Bearer(value));
                Ok(())
            }
            CredentialMaterialKind::Basic { username, password } => {
                asset.set_authorization(ResolvedAuthorization::Basic { username, password });
                Ok(())
            }
            CredentialMaterialKind::Cookie { name, value } => {
                asset.insert_cookie(name, value.expose())
            }
        }
    }

    pub(crate) fn apply_bearer_to(
        self,
        asset: &mut super::ResolvedAsset,
    ) -> Result<(), ResolveError> {
        match self.0 {
            CredentialMaterialKind::Bearer(value) => {
                asset.set_authorization(ResolvedAuthorization::Bearer(value));
                Ok(())
            }
            CredentialMaterialKind::Basic { .. } | CredentialMaterialKind::Cookie { .. } => {
                Err(ResolveError::auth_failed())
            }
        }
    }

    pub(crate) fn into_transfer_auth(
        self,
    ) -> Result<crate::TransferAuth, crate::TransferRequestError> {
        match self.0 {
            CredentialMaterialKind::Bearer(value) => {
                crate::TransferAuth::bearer(value.expose().to_string())
            }
            CredentialMaterialKind::Basic { username, password } => crate::TransferAuth::basic(
                username.expose().to_string(),
                password.expose().to_string(),
            ),
            CredentialMaterialKind::Cookie { .. } => {
                Err(crate::TransferRequestError::InvalidIdentity)
            }
        }
    }
}

impl fmt::Debug for CredentialMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.0 {
            CredentialMaterialKind::Bearer(_) => "bearer",
            CredentialMaterialKind::Basic { .. } => "basic",
            CredentialMaterialKind::Cookie { .. } => "cookie",
        };
        formatter
            .debug_struct("CredentialMaterial")
            .field("kind", &kind)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

pub trait CredentialProvider: Send + Sync {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError>;
}

pub struct ResolveContext<'a> {
    credential_provider: &'a dyn CredentialProvider,
    cancellation: &'a (dyn Fn() -> bool + Sync),
    clock: &'a dyn Clock,
    proxy_policy: ProxyPolicy,
    timeout_policy: TimeoutPolicy,
    auth_refresh_budget: AuthRefreshBudget,
}

impl<'a> ResolveContext<'a> {
    pub fn new(
        credential_provider: &'a dyn CredentialProvider,
        cancellation: &'a (dyn Fn() -> bool + Sync),
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            credential_provider,
            cancellation,
            clock,
            proxy_policy: ProxyPolicy::System,
            timeout_policy: TimeoutPolicy::default(),
            auth_refresh_budget: AuthRefreshBudget::default(),
        }
    }

    pub fn with_proxy_policy(mut self, policy: ProxyPolicy) -> Self {
        self.proxy_policy = policy;
        self
    }

    pub fn with_timeout_policy(mut self, policy: TimeoutPolicy) -> Self {
        self.timeout_policy = policy;
        self
    }

    pub fn with_auth_refresh_budget(mut self, budget: AuthRefreshBudget) -> Self {
        self.auth_refresh_budget = budget;
        self
    }

    pub fn check_cancelled(&self) -> Result<(), ResolveError> {
        if (self.cancellation)() {
            return Err(ResolveError::cancelled());
        }
        Ok(())
    }

    pub fn credential_provider(&self) -> &dyn CredentialProvider {
        self.credential_provider
    }

    pub fn clock(&self) -> &dyn Clock {
        self.clock
    }

    pub fn proxy_policy(&self) -> ProxyPolicy {
        self.proxy_policy
    }

    pub fn timeout_policy(&self) -> TimeoutPolicy {
        self.timeout_policy
    }

    pub fn auth_refresh_budget(&self) -> &AuthRefreshBudget {
        &self.auth_refresh_budget
    }
}

impl fmt::Debug for ResolveContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolveContext")
            .field("credential_provider", &"[CONTROLLED]")
            .field("cancellation", &"[CALLBACK]")
            .field("clock", &"[INJECTED]")
            .field("proxy_policy", &self.proxy_policy)
            .field("timeout_policy", &self.timeout_policy)
            .field("auth_refresh_budget", &self.auth_refresh_budget)
            .finish()
    }
}
