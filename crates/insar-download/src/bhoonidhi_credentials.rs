//! Bhoonidhi desktop credential boundary.
//!
//! Durable Bhoonidhi login material is stored in the native desktop credential
//! store. On Windows this is the current-user Windows Credential Manager via
//! `CredReadW`/`CredWriteW`/`CredDeleteW` in `credentials.rs`; the JWT and
//! refresh token remain process-memory-only in `BhoonidhiSession`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bhoonidhi_oauth::BhoonidhiLoginMaterial;
use crate::credentials::{
    clear_desktop_bhoonidhi, desktop_bhoonidhi_status, resolve_desktop_bhoonidhi_login,
    store_desktop_bhoonidhi_login, DesktopCredentialError, BHOONIDHI_KEYRING_SERVICE,
};

pub const BHOONIDHI_CREDENTIAL_PROFILE: &str = "bhoonidhi-default";
/// Maximum accepted login-field size, matching the OAuth boundary.
pub const BHOONIDHI_CREDENTIAL_MAX_SOURCE_BYTES: u64 = 4 * 1024;
/// The stable Credential Manager service/target namespace. This is not a file
/// path and must not be replaced with an application config location.
pub const BHOONIDHI_CREDENTIAL_TARGET: &str = BHOONIDHI_KEYRING_SERVICE;

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum BhoonidhiCredentialError {
    #[error("未找到 Bhoonidhi 凭据")]
    Missing,
    #[error("Bhoonidhi 凭据存储不可用")]
    Unavailable,
    #[error("Bhoonidhi 凭据内容无效")]
    Invalid,
}

impl BhoonidhiCredentialError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Missing => "BHOONIDHI_CREDENTIALS_MISSING",
            Self::Unavailable => "BHOONIDHI_CREDENTIAL_STORE_UNAVAILABLE",
            Self::Invalid => "BHOONIDHI_CREDENTIALS_INVALID",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BhoonidhiCredentialReadiness {
    Ready,
    Missing,
    Invalid,
    Unavailable,
}

fn map_store_error(error: DesktopCredentialError) -> BhoonidhiCredentialError {
    match error {
        DesktopCredentialError::InvalidInput => BhoonidhiCredentialError::Invalid,
        DesktopCredentialError::StoreUnavailable => BhoonidhiCredentialError::Unavailable,
    }
}

/// Read the login only from the native current-user credential store. No file,
/// environment variable, or shell credential source is consulted.
pub fn resolve_desktop_bhoonidhi_credential(
) -> Result<BhoonidhiLoginMaterial, BhoonidhiCredentialError> {
    match desktop_bhoonidhi_status().as_str() {
        "none" => Err(BhoonidhiCredentialError::Missing),
        "unavailable" => Err(BhoonidhiCredentialError::Unavailable),
        _ => resolve_desktop_bhoonidhi_login().map_err(map_store_error),
    }
}

pub fn read_desktop_bhoonidhi_login_material(
) -> Result<BhoonidhiLoginMaterial, BhoonidhiCredentialError> {
    resolve_desktop_bhoonidhi_credential()
}

/// Save username/password through the native Credential Manager boundary.
/// Secrets are passed directly to the keyring helper and are not serialized.
pub fn store_desktop_bhoonidhi_credentials(
    username: &str,
    password: &str,
) -> Result<(), BhoonidhiCredentialError> {
    if username.trim().is_empty() || password.is_empty() {
        return Err(BhoonidhiCredentialError::Invalid);
    }
    store_desktop_bhoonidhi_login(username, password).map_err(map_store_error)
}

pub fn clear_desktop_bhoonidhi_credentials() -> Result<bool, BhoonidhiCredentialError> {
    clear_desktop_bhoonidhi().map_err(map_store_error)
}

/// Project only a secret-free readiness state. No auth request is made.
pub fn desktop_bhoonidhi_readiness() -> BhoonidhiCredentialReadiness {
    match desktop_bhoonidhi_status().as_str() {
        "none" => BhoonidhiCredentialReadiness::Missing,
        "unavailable" => BhoonidhiCredentialReadiness::Unavailable,
        _ => match resolve_desktop_bhoonidhi_login() {
            Ok(material) => {
                drop(material);
                BhoonidhiCredentialReadiness::Ready
            }
            Err(DesktopCredentialError::InvalidInput) => BhoonidhiCredentialReadiness::Invalid,
            Err(DesktopCredentialError::StoreUnavailable) => {
                BhoonidhiCredentialReadiness::Unavailable
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_boundary_is_a_manager_target_not_a_file_path() {
        assert_eq!(BHOONIDHI_CREDENTIAL_TARGET, "eostage:bhoonidhi");
        assert!(!BHOONIDHI_CREDENTIAL_TARGET
            .bytes()
            .any(|byte| byte == b'/' || byte == 92));
    }

    #[test]
    fn invalid_login_is_rejected_before_store_access() {
        assert_eq!(
            store_desktop_bhoonidhi_credentials("", "secret").unwrap_err(),
            BhoonidhiCredentialError::Invalid
        );
        assert_eq!(
            store_desktop_bhoonidhi_credentials("user", "").unwrap_err(),
            BhoonidhiCredentialError::Invalid
        );
    }
}
