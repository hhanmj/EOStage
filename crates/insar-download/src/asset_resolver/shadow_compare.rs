use std::fmt;
use std::path::PathBuf;

use insar_core::{AssetRole, DownloadLayout};
use reqwest::Url;

use super::shadow_plan::{classify_host, classify_path};
use super::{ResolveError, ShadowAccessIntent, ShadowAuthKind, ShadowHostClass, ShadowPathClass};
use crate::{plan_download_target, plan_sar_catalog_target};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShadowProviderClass {
    Asf,
    Orbit,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShadowMismatchCode {
    Provider,
    HostClass,
    PathClass,
    Filename,
    AuthKind,
    ExpectedSize,
    Checksum,
    Destination,
    AssetRole,
}

pub struct LegacyTransferIntent {
    pub provider_id: String,
    pub url: String,
    pub file_name: String,
    pub auth_kind: ShadowAuthKind,
    pub expected_size: Option<u64>,
    pub checksum_algorithm: Option<String>,
    pub checksum_value: Option<String>,
    pub destination: PathBuf,
    pub destination_layout: DownloadLayout,
    pub asset_role: AssetRole,
}

impl fmt::Debug for LegacyTransferIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyTransferIntent")
            .field("provider_id", &"[CLASSIFIED]")
            .field("url", &"[REDACTED]")
            .field("file_name", &"[REDACTED]")
            .field("auth_kind", &self.auth_kind)
            .field("expected_size", &self.expected_size.is_some())
            .field(
                "checksum",
                &self.checksum_value.as_ref().map(|_| "[REDACTED]"),
            )
            .field("destination", &"[REDACTED]")
            .field("destination_layout", &self.destination_layout)
            .field("asset_role", &self.asset_role)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TransferIntentFingerprint {
    provider_class: ShadowProviderClass,
    host_class: ShadowHostClass,
    path_class: ShadowPathClass,
    file_name: Option<String>,
    auth_kind: ShadowAuthKind,
    expected_size: Option<u64>,
    checksum_algorithm: Option<String>,
    checksum_value: Option<String>,
    destination: PathBuf,
    destination_layout: DownloadLayout,
    asset_role: AssetRole,
}

impl TransferIntentFingerprint {
    pub fn from_legacy(value: &LegacyTransferIntent) -> Result<Self, ResolveError> {
        let url = Url::parse(&value.url).map_err(|_| ResolveError::access_not_durable())?;
        let host = url
            .host_str()
            .ok_or_else(ResolveError::access_not_durable)?;
        let host_class = classify_host(host, url.path());
        let path_class = classify_path(host_class, Some(&value.file_name), url.path());
        Self::from_parts(
            provider_class(&value.provider_id),
            host_class,
            path_class,
            Some(value.file_name.clone()),
            value.auth_kind,
            value.expected_size,
            value.checksum_algorithm.clone(),
            value.checksum_value.clone(),
            value.destination.clone(),
            value.destination_layout.clone(),
            value.asset_role.clone(),
        )
    }

    pub fn from_shadow(
        value: &ShadowAccessIntent,
        destination: PathBuf,
        destination_layout: DownloadLayout,
    ) -> Result<Self, ResolveError> {
        Self::from_parts(
            provider_class(value.provider_id()),
            value.host_class(),
            value.path_class(),
            value.file_name().map(str::to_string),
            value.auth_kind(),
            value.expected_size(),
            value.checksum_algorithm().map(str::to_string),
            value.checksum_value().map(str::to_string),
            destination,
            destination_layout,
            value.asset_role(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        provider_class: ShadowProviderClass,
        host_class: ShadowHostClass,
        path_class: ShadowPathClass,
        file_name: Option<String>,
        auth_kind: ShadowAuthKind,
        expected_size: Option<u64>,
        checksum_algorithm: Option<String>,
        checksum_value: Option<String>,
        destination: PathBuf,
        destination_layout: DownloadLayout,
        asset_role: AssetRole,
    ) -> Result<Self, ResolveError> {
        if (checksum_algorithm.is_some() != checksum_value.is_some())
            || file_name
                .as_deref()
                .is_none_or(|name| name.trim().is_empty())
            || destination.as_os_str().is_empty()
        {
            return Err(ResolveError::descriptor_invalid());
        }
        Ok(Self {
            provider_class,
            host_class,
            path_class,
            file_name,
            auth_kind,
            expected_size,
            checksum_algorithm,
            checksum_value,
            destination,
            destination_layout,
            asset_role,
        })
    }

    pub fn compare(&self, other: &Self) -> Vec<ShadowMismatchCode> {
        let mut mismatches = Vec::new();
        if self.provider_class != other.provider_class {
            mismatches.push(ShadowMismatchCode::Provider);
        }
        if self.host_class != other.host_class {
            mismatches.push(ShadowMismatchCode::HostClass);
        }
        if self.path_class != other.path_class {
            mismatches.push(ShadowMismatchCode::PathClass);
        }
        if self.file_name != other.file_name {
            mismatches.push(ShadowMismatchCode::Filename);
        }
        if self.auth_kind != other.auth_kind {
            mismatches.push(ShadowMismatchCode::AuthKind);
        }
        if self.expected_size != other.expected_size {
            mismatches.push(ShadowMismatchCode::ExpectedSize);
        }
        if self.checksum_algorithm != other.checksum_algorithm
            || self.checksum_value != other.checksum_value
        {
            mismatches.push(ShadowMismatchCode::Checksum);
        }
        if self.destination != other.destination
            || self.destination_layout != other.destination_layout
        {
            mismatches.push(ShadowMismatchCode::Destination);
        }
        if self.asset_role != other.asset_role {
            mismatches.push(ShadowMismatchCode::AssetRole);
        }
        mismatches
    }

    pub fn checksum_algorithm(&self) -> Option<&str> {
        self.checksum_algorithm.as_deref()
    }

    pub fn checksum_value(&self) -> Option<&str> {
        self.checksum_value.as_deref()
    }

    pub fn with_provider_class(mut self, value: ShadowProviderClass) -> Self {
        self.provider_class = value;
        self
    }

    pub fn with_host_class(mut self, value: ShadowHostClass) -> Self {
        self.host_class = value;
        self
    }

    pub fn with_path_class(mut self, value: ShadowPathClass) -> Self {
        self.path_class = value;
        self
    }

    pub fn with_file_name(mut self, value: Option<String>) -> Self {
        self.file_name = value;
        self
    }

    pub fn with_auth_kind(mut self, value: ShadowAuthKind) -> Self {
        self.auth_kind = value;
        self
    }

    pub fn with_expected_size(mut self, value: Option<u64>) -> Self {
        self.expected_size = value;
        self
    }

    pub fn with_checksum(mut self, algorithm: Option<String>, value: Option<String>) -> Self {
        self.checksum_algorithm = algorithm;
        self.checksum_value = value;
        self
    }

    pub fn with_destination(mut self, value: PathBuf) -> Self {
        self.destination = value;
        self
    }

    pub fn with_asset_role(mut self, value: AssetRole) -> Self {
        self.asset_role = value;
        self
    }
}

impl fmt::Debug for TransferIntentFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransferIntentFingerprint")
            .field("provider_class", &self.provider_class)
            .field("host_class", &self.host_class)
            .field("path_class", &self.path_class)
            .field("file_name", &self.file_name.as_ref().map(|_| "[REDACTED]"))
            .field("auth_kind", &self.auth_kind)
            .field("expected_size", &self.expected_size.is_some())
            .field(
                "checksum",
                &self.checksum_value.as_ref().map(|_| "[REDACTED]"),
            )
            .field("destination", &"[REDACTED]")
            .field("destination_layout", &self.destination_layout)
            .field("asset_role", &self.asset_role)
            .finish()
    }
}

#[derive(Clone)]
pub struct ShadowDestinationPolicy {
    output_root: PathBuf,
    layout: DownloadLayout,
    dataset: String,
    processing_level: String,
}

impl ShadowDestinationPolicy {
    pub fn new(
        output_root: PathBuf,
        layout: DownloadLayout,
        dataset: impl Into<String>,
        processing_level: impl Into<String>,
    ) -> Self {
        Self {
            output_root,
            layout,
            dataset: dataset.into(),
            processing_level: processing_level.into(),
        }
    }

    pub fn layout(&self) -> &DownloadLayout {
        &self.layout
    }

    pub fn destination_for(&self, file_name: &str) -> Result<PathBuf, ResolveError> {
        if self.layout == DownloadLayout::SarDatasetProcessing {
            plan_sar_catalog_target(
                &self.output_root,
                &self.dataset,
                &self.processing_level,
                file_name,
            )
            .map_err(|_| ResolveError::failed())
        } else {
            plan_download_target(&self.output_root, &self.layout, file_name)
                .map_err(|_| ResolveError::failed())
        }
    }
}

fn provider_class(provider_id: &str) -> ShadowProviderClass {
    let normalized = provider_id.trim().to_ascii_lowercase();
    if normalized.starts_with("asf.") {
        ShadowProviderClass::Asf
    } else if normalized.contains("orbit") {
        ShadowProviderClass::Orbit
    } else {
        ShadowProviderClass::Other
    }
}
