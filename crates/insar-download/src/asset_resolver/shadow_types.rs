use std::fmt;
use std::time::{Duration, SystemTime};

use insar_core::{AssetAccessScheme, AssetRole};

use super::{Clock, ResolveError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowNetworkAccess {
    Forbidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowAuthKind {
    None,
    EarthdataBearer,
    EarthdataBasic,
    EarthdataSessionCookie,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowHostClass {
    AsfDatapool,
    EarthdataCloud,
    AsfBurst,
    AsfOrbit,
    PublicHttpsOther,
    LocalFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowPathClass {
    Archive,
    BurstTiff,
    BurstXml,
    OrbitEof,
    NativeFile,
}

pub trait ShadowCredentialShapeProvider: Send + Sync {
    fn auth_kind(&self, profile: &str) -> Result<ShadowAuthKind, ResolveError>;
}

pub struct ShadowResolveContext<'a> {
    credential_shapes: &'a dyn ShadowCredentialShapeProvider,
    cancellation: &'a (dyn Fn() -> bool + Sync),
    clock: &'a dyn Clock,
    deadline: SystemTime,
    network_access: ShadowNetworkAccess,
}

impl<'a> ShadowResolveContext<'a> {
    pub fn new(
        credential_shapes: &'a dyn ShadowCredentialShapeProvider,
        cancellation: &'a (dyn Fn() -> bool + Sync),
        clock: &'a dyn Clock,
        budget: Duration,
    ) -> Result<Self, ResolveError> {
        if budget.is_zero() {
            return Err(ResolveError::descriptor_invalid());
        }
        let deadline = clock
            .now()
            .checked_add(budget)
            .ok_or_else(ResolveError::timeout)?;
        Ok(Self {
            credential_shapes,
            cancellation,
            clock,
            deadline,
            network_access: ShadowNetworkAccess::Forbidden,
        })
    }

    pub fn network_access(&self) -> ShadowNetworkAccess {
        self.network_access
    }

    pub fn checkpoint(&self) -> Result<(), ResolveError> {
        if (self.cancellation)() {
            return Err(ResolveError::cancelled());
        }
        if self.clock.now() >= self.deadline {
            return Err(ResolveError::timeout());
        }
        Ok(())
    }

    pub(crate) fn credential_shape(&self, profile: &str) -> Result<ShadowAuthKind, ResolveError> {
        self.checkpoint()?;
        let kind = self.credential_shapes.auth_kind(profile)?;
        self.checkpoint()?;
        Ok(kind)
    }
}

impl fmt::Debug for ShadowResolveContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShadowResolveContext")
            .field("credential_shapes", &"[SHAPE_ONLY]")
            .field("cancellation", &"[CALLBACK]")
            .field("clock", &"[INJECTED]")
            .field("deadline", &"[BOUNDED]")
            .field("network_access", &self.network_access)
            .finish()
    }
}

/// Runtime-only, non-executable Shadow planning output.
///
/// ```compile_fail
/// # use insar_download::ShadowAccessIntent;
/// fn persist(value: &ShadowAccessIntent) {
///     let _ = serde_json::to_value(value);
/// }
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct ShadowAccessIntent {
    provider_id: String,
    scheme: AssetAccessScheme,
    host_class: ShadowHostClass,
    path_class: ShadowPathClass,
    file_name: Option<String>,
    auth_kind: ShadowAuthKind,
    expected_size: Option<u64>,
    checksum_algorithm: Option<String>,
    checksum_value: Option<String>,
    asset_role: AssetRole,
}

impl ShadowAccessIntent {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        provider_id: String,
        scheme: AssetAccessScheme,
        host_class: ShadowHostClass,
        path_class: ShadowPathClass,
        file_name: Option<String>,
        auth_kind: ShadowAuthKind,
        expected_size: Option<u64>,
        checksum_algorithm: Option<String>,
        checksum_value: Option<String>,
        asset_role: AssetRole,
    ) -> Result<Self, ResolveError> {
        if provider_id.trim().is_empty()
            || (checksum_algorithm.is_some() != checksum_value.is_some())
            || file_name
                .as_deref()
                .is_some_and(|name| name.trim().is_empty())
        {
            return Err(ResolveError::descriptor_invalid());
        }
        Ok(Self {
            provider_id,
            scheme,
            host_class,
            path_class,
            file_name,
            auth_kind,
            expected_size,
            checksum_algorithm,
            checksum_value,
            asset_role,
        })
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn scheme(&self) -> AssetAccessScheme {
        self.scheme.clone()
    }

    pub fn host_class(&self) -> ShadowHostClass {
        self.host_class
    }

    pub fn path_class(&self) -> ShadowPathClass {
        self.path_class
    }

    pub fn file_name(&self) -> Option<&str> {
        self.file_name.as_deref()
    }

    pub fn auth_kind(&self) -> ShadowAuthKind {
        self.auth_kind
    }

    pub fn expected_size(&self) -> Option<u64> {
        self.expected_size
    }

    pub fn checksum_algorithm(&self) -> Option<&str> {
        self.checksum_algorithm.as_deref()
    }

    pub fn checksum_value(&self) -> Option<&str> {
        self.checksum_value.as_deref()
    }

    pub fn asset_role(&self) -> AssetRole {
        self.asset_role.clone()
    }
}

impl fmt::Debug for ShadowAccessIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShadowAccessIntent")
            .field("provider_id", &"[CLASSIFIED]")
            .field("scheme", &self.scheme)
            .field("host_class", &self.host_class)
            .field("path_class", &self.path_class)
            .field("file_name", &self.file_name.as_ref().map(|_| "[REDACTED]"))
            .field("auth_kind", &self.auth_kind)
            .field("expected_size", &self.expected_size.is_some())
            .field(
                "checksum",
                &self.checksum_value.as_ref().map(|_| "[REDACTED]"),
            )
            .field("asset_role", &self.asset_role)
            .finish()
    }
}
