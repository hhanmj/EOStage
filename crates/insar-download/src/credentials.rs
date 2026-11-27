use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard,
};

#[cfg(test)]
#[path = "credentials_legacy_search_token_tests.rs"]
mod credentials_legacy_search_token_tests;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::asset_resolver::cdse::CDSE_CREDENTIAL_PROFILE;
use crate::asset_resolver::{CredentialMaterial, CredentialProvider, CredentialProviderError};
use crate::bhoonidhi_oauth::BhoonidhiLoginMaterial;
use crate::execution_policy::EARTHDATA_BURST_CREDENTIAL_PROFILE;
use crate::{CdseOAuthSession, TransferAuth};

const EDL_HOST: &str = "urs.earthdata.nasa.gov";
pub const EARTHDATA_CREDENTIAL_PROFILE: &str = "earthdata-default";
/// These service and username values are part of the v2.1.9 desktop keyring
/// compatibility boundary. Keep them stable so Rust/Tauri reads credentials
/// saved by the Python desktop and CLI.
pub const EARTHDATA_KEYRING_SERVICE: &str = "eostage:earthdata";
pub const CDSE_KEYRING_SERVICE: &str = "eostage:cdse";
pub const USGS_KEYRING_SERVICE: &str = "eostage:usgs-m2m";
/// Stable Windows Credential Manager service for the Bhoonidhi login.
/// Keep this value fixed so the desktop UI, Rust runtime, and future builds
/// address the same current-user credential target.
pub const BHOONIDHI_KEYRING_SERVICE: &str = "eostage:bhoonidhi";
pub const OPENTOPOGRAPHY_KEYRING_SERVICE: &str = "eostage:opentopography";
pub const GACOS_KEYRING_SERVICE: &str = "eostage:gacos";
/// Agent model API keys: each channel's key lives on its own target
/// `eostage:agent-model:<channel id>`; the bare value is the legacy shared
/// target that `credentials::agent_model` migrates away from.
pub const AGENT_MODEL_KEYRING_SERVICE: &str = "eostage:agent-model";
/// Stable OS Keyring service for Google Earth Engine (GEE) credentials.
#[allow(dead_code)]
pub const GEE_KEYRING_SERVICE: &str = "eostage:earthengine";
#[allow(dead_code)]
const GEE_PROJECT_ID_KEY: &str = "project_id";
#[allow(dead_code)]
const GEE_SECRET_KEY: &str = "secret";
#[allow(dead_code)]
const GEE_AUTH_MODE_KEY: &str = "auth_mode";
#[allow(dead_code)]
const GEE_CLIENT_EMAIL_KEY: &str = "client_email";
const EARTHDATA_TOKEN_KEY: &str = "token";
const EARTHDATA_USERNAME_KEY: &str = "username";
const EARTHDATA_PASSWORD_KEY: &str = "password";
const CDSE_USERNAME_KEY: &str = "username";
const CDSE_PASSWORD_KEY: &str = "password";
const USGS_USERNAME_KEY: &str = "username";
const USGS_APPLICATION_TOKEN_KEY: &str = "application_token";
const BHOONIDHI_USERNAME_KEY: &str = "username";
const BHOONIDHI_PASSWORD_KEY: &str = "password";
const OPENTOPOGRAPHY_KEY: &str = "api_key";
const GACOS_EMAIL_KEY: &str = "email";
const TOKEN_ENV: &str = "EARTHDATA_TOKEN";
const USERNAME_ENV: &str = "INSAR_EARTHDATA_USERNAME";
const PASSWORD_ENV: &str = "INSAR_EARTHDATA_PASSWORD";
const MAX_NETRC_BYTES: u64 = 1024 * 1024;
const ISOLATE_EMPTY_KEYCHAIN_ENV: &str = "EOSTAGE_ISOLATE_EMPTY_KEYCHAIN";

#[cfg(test)]
static TEST_ISOLATE_EMPTY_KEYCHAIN_OVERRIDE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn parse_isolated_empty_keychain_flag(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed == "1" || trimmed.eq_ignore_ascii_case("true")
}

pub(crate) fn is_isolated_empty_keychain_mode() -> bool {
    #[cfg(test)]
    if TEST_ISOLATE_EMPTY_KEYCHAIN_OVERRIDE.load(Ordering::SeqCst) {
        return true;
    }
    std::env::var(ISOLATE_EMPTY_KEYCHAIN_ENV)
        .map(|value| parse_isolated_empty_keychain_flag(&value))
        .unwrap_or(false)
}

struct BhoonidhiCredentialMutationState {
    lock: RwLock<()>,
    revision: AtomicU64,
}

static BHOONIDHI_CREDENTIAL_MUTATION_STATE: OnceLock<BhoonidhiCredentialMutationState> =
    OnceLock::new();

static DESKTOP_LOGIN_PAIR_LOCK: OnceLock<RwLock<()>> = OnceLock::new();

fn desktop_login_pair_lock() -> &'static RwLock<()> {
    DESKTOP_LOGIN_PAIR_LOCK.get_or_init(|| RwLock::new(()))
}

fn desktop_login_pair_write_guard() -> Result<RwLockWriteGuard<'static, ()>, DesktopCredentialError>
{
    desktop_login_pair_lock()
        .write()
        .map_err(|_| DesktopCredentialError::StoreUnavailable)
}

fn desktop_login_pair_read_guard() -> Result<RwLockReadGuard<'static, ()>, DesktopCredentialError> {
    desktop_login_pair_lock()
        .read()
        .map_err(|_| DesktopCredentialError::StoreUnavailable)
}

fn bhoonidhi_credential_mutation_state() -> &'static BhoonidhiCredentialMutationState {
    BHOONIDHI_CREDENTIAL_MUTATION_STATE.get_or_init(|| BhoonidhiCredentialMutationState {
        lock: RwLock::new(()),
        revision: AtomicU64::new(0),
    })
}

fn bhoonidhi_credential_write_guard(
) -> Result<RwLockWriteGuard<'static, ()>, DesktopCredentialError> {
    bhoonidhi_credential_mutation_state()
        .lock
        .write()
        .map_err(|_| DesktopCredentialError::StoreUnavailable)
}

pub(crate) fn bhoonidhi_credential_read_guard() -> Result<RwLockReadGuard<'static, ()>, ()> {
    bhoonidhi_credential_mutation_state()
        .lock
        .read()
        .map_err(|_| ())
}

pub(crate) fn bhoonidhi_credential_revision() -> u64 {
    bhoonidhi_credential_mutation_state()
        .revision
        .load(Ordering::Acquire)
}

fn mark_bhoonidhi_credential_revision() -> Result<(), DesktopCredentialError> {
    bhoonidhi_credential_mutation_state()
        .revision
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |revision| {
            revision.checked_add(1)
        })
        .map(|_| ())
        .map_err(|_| DesktopCredentialError::StoreUnavailable)
}

#[cfg(test)]
pub(crate) fn mark_bhoonidhi_credentials_changed_for_test() {
    let _guard = bhoonidhi_credential_write_guard().expect("credential mutation lock");
    mark_bhoonidhi_credential_revision().expect("credential revision");
}

/// Closed field selectors for an explicit, current-user desktop reveal action.
/// Status polling must never call this operation or return its payload.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopCredentialField {
    CdseUsername,
    CdsePassword,
    UsgsUsername,
    UsgsApplicationToken,
    BhoonidhiUsername,
    BhoonidhiPassword,
    EarthdataUsername,
    EarthdataPassword,
    EarthdataToken,
    OpentopographyKey,
    GacosEmail,
}

impl DesktopCredentialField {
    fn target(self) -> (&'static str, &'static str) {
        match self {
            DesktopCredentialField::CdseUsername => (CDSE_KEYRING_SERVICE, CDSE_USERNAME_KEY),
            DesktopCredentialField::CdsePassword => (CDSE_KEYRING_SERVICE, CDSE_PASSWORD_KEY),
            DesktopCredentialField::UsgsUsername => (USGS_KEYRING_SERVICE, USGS_USERNAME_KEY),
            DesktopCredentialField::UsgsApplicationToken => {
                (USGS_KEYRING_SERVICE, USGS_APPLICATION_TOKEN_KEY)
            }
            DesktopCredentialField::BhoonidhiUsername => {
                (BHOONIDHI_KEYRING_SERVICE, BHOONIDHI_USERNAME_KEY)
            }
            DesktopCredentialField::BhoonidhiPassword => {
                (BHOONIDHI_KEYRING_SERVICE, BHOONIDHI_PASSWORD_KEY)
            }
            DesktopCredentialField::EarthdataUsername => {
                (EARTHDATA_KEYRING_SERVICE, EARTHDATA_USERNAME_KEY)
            }
            DesktopCredentialField::EarthdataPassword => {
                (EARTHDATA_KEYRING_SERVICE, EARTHDATA_PASSWORD_KEY)
            }
            DesktopCredentialField::EarthdataToken => {
                (EARTHDATA_KEYRING_SERVICE, EARTHDATA_TOKEN_KEY)
            }
            DesktopCredentialField::OpentopographyKey => {
                (OPENTOPOGRAPHY_KEYRING_SERVICE, OPENTOPOGRAPHY_KEY)
            }
            DesktopCredentialField::GacosEmail => (GACOS_KEYRING_SERVICE, GACOS_EMAIL_KEY),
        }
    }
}

#[path = "credential_editor.rs"]
mod editor;
pub use editor::{read_desktop_credential_for_edit, update_desktop_credential_field};

pub fn reveal_desktop_credential_field(
    field: DesktopCredentialField,
) -> Result<Option<zeroize::Zeroizing<String>>, DesktopCredentialError> {
    let (service, key) = field.target();
    // Do not read environment variables, arbitrary Credential Manager targets or .netrc.
    // Pair credentials share the same read/write boundary as their native save/login path.
    let _guard = match field {
        DesktopCredentialField::CdseUsername
        | DesktopCredentialField::CdsePassword
        | DesktopCredentialField::UsgsUsername
        | DesktopCredentialField::UsgsApplicationToken => Some(desktop_login_pair_read_guard()?),
        DesktopCredentialField::BhoonidhiUsername | DesktopCredentialField::BhoonidhiPassword => {
            Some(
                bhoonidhi_credential_read_guard()
                    .map_err(|_| DesktopCredentialError::StoreUnavailable)?,
            )
        }
        _ => None,
    };
    keyring_get(service, key).map(|value| value.map(zeroize::Zeroizing::new))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarthdataCredentialSource {
    Auto,
    EnvironmentToken,
    EnvironmentLogin,
    Netrc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EarthdataCredentialReadiness {
    Ready,
    Missing,
    Invalid,
    Unavailable,
}

#[derive(Debug, Error)]
pub enum EarthdataCredentialError {
    #[error("未配置可用的 Earthdata 凭据")]
    Missing,
    #[error("Earthdata 凭据内容无效")]
    Invalid,
    #[error("无法安全读取 .netrc")]
    NetrcRead,
    #[error(".netrc 格式无效")]
    NetrcParse,
    #[error("Sentinel-1 Burst requires an Earthdata account session cookie")]
    BurstCookieRequired,
}

#[derive(Debug, Error)]
pub enum DesktopCredentialError {
    #[error("credential store unavailable")]
    StoreUnavailable,
    #[error("credential input is invalid")]
    InvalidInput,
}

impl DesktopCredentialError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::StoreUnavailable => "CREDENTIAL_STORE_UNAVAILABLE",
            Self::InvalidInput => "DL004",
        }
    }

    pub const fn reason_zh(&self) -> &'static str {
        match self {
            Self::StoreUnavailable => "无法访问系统凭证存储，请检查当前 Windows 用户的凭证管理器。",
            Self::InvalidInput => "凭证内容为空或格式不正确，请检查后重新保存。",
        }
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod chunked_secret {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};
    use zeroize::Zeroize;

    use super::DesktopCredentialError;

    pub const MANIFEST_PREFIX: &str = "__EOSTAGE_CRED_CHUNKS_V1__:";
    /// Leave comfortable headroom below Windows `CRED_MAX_CREDENTIAL_BLOB_SIZE` (2560 B).
    pub const SINGLE_BLOB_MAX_UTF16_BYTES: usize = 2400;
    /// Raw UTF-8 chunk size stored per Windows credential target (well below 2560 B).
    pub const CHUNK_MAX_UTF8_BYTES: usize = 2048;
    pub const MAX_CHUNKS: usize = 64;
    pub const MAX_TOTAL_BYTES: usize = 131_072;

    static GEN_COUNTER: AtomicU64 = AtomicU64::new(1);

    pub trait RawTargetStore {
        fn with_raw_target<T>(
            &self,
            target: &str,
            inspect: impl FnOnce(&str, &[u8]) -> Result<T, DesktopCredentialError>,
        ) -> Result<Option<T>, DesktopCredentialError>;

        fn write_raw_target(
            &self,
            target: &str,
            username: &str,
            blob: &mut [u8],
        ) -> Result<(), DesktopCredentialError>;

        fn delete_raw_target(&self, target: &str) -> Result<bool, DesktopCredentialError>;
    }

    struct ZeroizeBytes(Vec<u8>);

    impl ZeroizeBytes {
        fn with_capacity(cap: usize) -> Self {
            Self(Vec::with_capacity(cap))
        }

        fn into_string(mut self) -> Result<String, DesktopCredentialError> {
            let bytes = std::mem::take(&mut self.0);
            String::from_utf8(bytes).map_err(|err| {
                let mut invalid = err.into_bytes();
                invalid.zeroize();
                DesktopCredentialError::StoreUnavailable
            })
        }
    }

    impl Drop for ZeroizeBytes {
        fn drop(&mut self) {
            self.0.zeroize();
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
    pub struct Manifest {
        pub v: u8,
        #[serde(rename = "gen")]
        pub generation: String,
        pub n: usize,
        pub len: usize,
        pub sha256: String,
    }

    impl Manifest {
        fn validate(&self) -> Result<(), DesktopCredentialError> {
            let valid_gen = !self.generation.is_empty()
                && self.generation.len() <= 32
                && self
                    .generation
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
            let valid_sha = self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
            if self.v != 1
                || !valid_gen
                || self.n == 0
                || self.n > MAX_CHUNKS
                || self.len == 0
                || self.len > MAX_TOTAL_BYTES
                || !valid_sha
            {
                return Err(DesktopCredentialError::StoreUnavailable);
            }
            Ok(())
        }
    }

    pub fn encode_utf16le(value: &str) -> Vec<u8> {
        value.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    pub fn decode_password_bytes(bytes: &[u8]) -> Result<String, DesktopCredentialError> {
        if bytes.len().is_multiple_of(2) {
            let units = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>();
            if let Ok(value) = String::from_utf16(&units) {
                return Ok(value
                    .trim_start_matches('\u{feff}')
                    .trim_end_matches('\0')
                    .to_owned());
            }
        }
        String::from_utf8(bytes.to_vec())
            .map(|value| value.trim_end_matches('\0').to_owned())
            .map_err(|_| DesktopCredentialError::StoreUnavailable)
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(64);
        for b in digest {
            hex.push_str(&format!("{b:02x}"));
        }
        hex
    }

    fn next_generation_id() -> String {
        let seq = GEN_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let mut hasher = Sha256::new();
        hasher.update(pid.to_le_bytes());
        hasher.update(seq.to_le_bytes());
        hasher.update(nanos.to_le_bytes());
        let digest = hasher.finalize();
        let mut out = String::with_capacity(12);
        for b in &digest[..6] {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }

    pub fn chunk_target_name(target: &str, generation: &str, index: usize) -> String {
        format!("{target}#c{generation}:{index}")
    }

    fn parse_and_validate_manifest(raw_json: &str) -> Result<Manifest, DesktopCredentialError> {
        let manifest: Manifest =
            serde_json::from_str(raw_json).map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn read_target_username<S: RawTargetStore>(
        store: &S,
        target: &str,
    ) -> Result<Option<String>, DesktopCredentialError> {
        store.with_raw_target(target, |username, _| Ok(username.to_owned()))
    }

    fn inspect_existing_manifest<S: RawTargetStore>(
        store: &S,
        target: &str,
        expected_owner: Option<&str>,
    ) -> Option<(String, Manifest)> {
        store
            .with_raw_target(target, |owner, blob| {
                if let Some(exp) = expected_owner {
                    if owner != exp {
                        return Ok(None);
                    }
                }
                let Ok(decoded) = decode_password_bytes(blob) else {
                    return Ok(None);
                };
                let Some(rest) = decoded.strip_prefix(MANIFEST_PREFIX) else {
                    return Ok(None);
                };
                let Ok(manifest) = parse_and_validate_manifest(rest) else {
                    return Ok(None);
                };
                Ok(Some((owner.to_owned(), manifest)))
            })
            .ok()
            .flatten()
            .flatten()
    }

    fn delete_manifest_chunks<S: RawTargetStore>(
        store: &S,
        target: &str,
        username: &str,
        manifest: &Manifest,
    ) {
        for idx in 1..=manifest.n {
            let chunk_target = chunk_target_name(target, &manifest.generation, idx);
            if matches!(
                read_target_username(store, &chunk_target),
                Ok(Some(ref owner)) if owner == username
            ) {
                let _ = store.delete_raw_target(&chunk_target);
            }
        }
    }

    pub fn write_secret<S: RawTargetStore>(
        store: &S,
        target: &str,
        username: &str,
        password: &str,
    ) -> Result<(), DesktopCredentialError> {
        let prior_same_owner = inspect_existing_manifest(store, target, Some(username));
        let utf16_bytes_len = password.encode_utf16().count().saturating_mul(2);
        if utf16_bytes_len <= SINGLE_BLOB_MAX_UTF16_BYTES && !password.starts_with(MANIFEST_PREFIX)
        {
            let mut blob = encode_utf16le(password);
            let res = store.write_raw_target(target, username, &mut blob);
            blob.zeroize();
            res?;
            if let Some((_, old_manifest)) = prior_same_owner {
                delete_manifest_chunks(store, target, username, &old_manifest);
            }
            return Ok(());
        }

        let raw_bytes = password.as_bytes();
        if raw_bytes.is_empty() || raw_bytes.len() > MAX_TOTAL_BYTES {
            return Err(DesktopCredentialError::InvalidInput);
        }
        let chunks: Vec<&[u8]> = raw_bytes.chunks(CHUNK_MAX_UTF8_BYTES).collect();
        if chunks.is_empty() || chunks.len() > MAX_CHUNKS {
            return Err(DesktopCredentialError::InvalidInput);
        }

        let generation = next_generation_id();
        let manifest = Manifest {
            v: 1,
            generation: generation.clone(),
            n: chunks.len(),
            len: raw_bytes.len(),
            sha256: sha256_hex(raw_bytes),
        };
        let manifest_json = serde_json::to_string(&manifest)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let manifest_str = format!("{MANIFEST_PREFIX}{manifest_json}");
        let mut manifest_blob = encode_utf16le(&manifest_str);
        if manifest_blob.len() > SINGLE_BLOB_MAX_UTF16_BYTES {
            manifest_blob.zeroize();
            return Err(DesktopCredentialError::StoreUnavailable);
        }

        for (idx, chunk) in chunks.iter().enumerate() {
            let chunk_target = chunk_target_name(target, &generation, idx + 1);
            let mut chunk_buf = chunk.to_vec();
            let write_res = store.write_raw_target(&chunk_target, username, &mut chunk_buf);
            chunk_buf.zeroize();
            if let Err(err) = write_res {
                for rollback_idx in 1..=(idx + 1) {
                    let _ = store.delete_raw_target(&chunk_target_name(
                        target,
                        &generation,
                        rollback_idx,
                    ));
                }
                manifest_blob.zeroize();
                return Err(err);
            }
        }

        let manifest_write_res = store.write_raw_target(target, username, &mut manifest_blob);
        manifest_blob.zeroize();
        if let Err(err) = manifest_write_res {
            for rollback_idx in 1..=chunks.len() {
                let _ =
                    store.delete_raw_target(&chunk_target_name(target, &generation, rollback_idx));
            }
            return Err(err);
        }

        if let Some((_, old_manifest)) = prior_same_owner {
            if old_manifest.generation != generation {
                delete_manifest_chunks(store, target, username, &old_manifest);
            }
        }
        Ok(())
    }

    pub fn read_secret_owned<S: RawTargetStore>(
        store: &S,
        target: &str,
        expected_owner: Option<&str>,
    ) -> Result<Option<(String, String)>, DesktopCredentialError> {
        enum EntryKind {
            Plain { owner: String, value: String },
            Chunked { owner: String, manifest: Manifest },
        }

        let found = store.with_raw_target(target, |owner, blob| {
            if let Some(exp) = expected_owner {
                if owner != exp {
                    return Ok(None);
                }
            }
            let decoded = decode_password_bytes(blob)?;
            if let Some(rest) = decoded.strip_prefix(MANIFEST_PREFIX) {
                let manifest = parse_and_validate_manifest(rest)?;
                Ok(Some(EntryKind::Chunked {
                    owner: owner.to_owned(),
                    manifest,
                }))
            } else {
                Ok(Some(EntryKind::Plain {
                    owner: owner.to_owned(),
                    value: decoded,
                }))
            }
        })?;

        match found.flatten() {
            None => Ok(None),
            Some(EntryKind::Plain { owner, value }) => Ok(Some((owner, value))),
            Some(EntryKind::Chunked { owner, manifest }) => {
                let mut reassembled = ZeroizeBytes::with_capacity(manifest.len);
                for idx in 1..=manifest.n {
                    let chunk_target = chunk_target_name(target, &manifest.generation, idx);
                    let present = store.with_raw_target(&chunk_target, |chunk_owner, blob| {
                        if chunk_owner != owner {
                            return Err(DesktopCredentialError::StoreUnavailable);
                        }
                        if reassembled.0.len().saturating_add(blob.len()) > manifest.len {
                            return Err(DesktopCredentialError::StoreUnavailable);
                        }
                        reassembled.0.extend_from_slice(blob);
                        Ok(())
                    })?;
                    if present.is_none() {
                        return Err(DesktopCredentialError::StoreUnavailable);
                    }
                }
                if reassembled.0.len() != manifest.len
                    || sha256_hex(&reassembled.0) != manifest.sha256
                {
                    return Err(DesktopCredentialError::StoreUnavailable);
                }
                let secret = reassembled.into_string()?;
                Ok(Some((owner, secret)))
            }
        }
    }

    pub fn delete_secret_owned<S: RawTargetStore>(
        store: &S,
        target: &str,
        username: &str,
    ) -> Result<bool, DesktopCredentialError> {
        let inspected = store.with_raw_target(target, |owner, blob| {
            if owner != username {
                return Ok(None);
            }
            let manifest = decode_password_bytes(blob)
                .ok()
                .and_then(|decoded| decoded.strip_prefix(MANIFEST_PREFIX).map(str::to_owned))
                .and_then(|rest| parse_and_validate_manifest(&rest).ok());
            Ok(Some(manifest))
        })?;

        let Some(Some(maybe_manifest)) = inspected else {
            return Ok(false);
        };

        // Delete the manifest target first so any partial chunk cleanup failure
        // cannot leave a dangling manifest that errors on future reads.
        let removed = store.delete_raw_target(target)?;
        if let Some(manifest) = maybe_manifest {
            delete_manifest_chunks(store, target, username, &manifest);
        }
        Ok(removed)
    }

    pub fn set_with_collision_move<S: RawTargetStore>(
        store: &S,
        service: &str,
        username: &str,
        password: &str,
    ) -> Result<(), DesktopCredentialError> {
        if let Some(other) = read_target_username(store, service)? {
            if !other.is_empty() && other != username {
                let old_manifest = inspect_existing_manifest(store, service, Some(&other));
                if let Some((_, mut existing_password)) =
                    read_secret_owned(store, service, Some(&other))?
                {
                    let compound = format!("{other}@{service}");
                    let move_res = write_secret(store, &compound, &other, &existing_password);
                    existing_password.zeroize();
                    move_res?;
                }
                write_secret(store, service, username, password)?;
                if let Some((_, old_m)) = old_manifest {
                    delete_manifest_chunks(store, service, &other, &old_m);
                }
                return Ok(());
            }
        }
        write_secret(store, service, username, password)
    }
}

#[cfg(target_os = "windows")]
mod platform_keyring {
    use std::ptr;

    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND, FILETIME};
    use windows_sys::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_ENTERPRISE,
        CRED_TYPE_GENERIC,
    };

    use super::chunked_secret::{self, RawTargetStore};
    use super::DesktopCredentialError;

    struct WinCredStore;

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn from_wide(value: *const u16) -> String {
        if value.is_null() {
            return String::new();
        }
        let mut length = 0;
        while *value.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(value, length))
    }

    /// One `CredReadW` of `target`. Windows has no read call that omits the
    /// blob: it is always copied into a buffer the API allocates. `inspect`
    /// sees the stored user name and that buffer, which is then zeroed in
    /// place and freed whatever `inspect` returned.
    fn with_target<T>(
        target: &str,
        inspect: impl FnOnce(&str, &[u8]) -> Result<T, DesktopCredentialError>,
    ) -> Result<Option<T>, DesktopCredentialError> {
        let target = to_wide(target);
        let mut raw = ptr::null_mut();
        let result = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) };
        if result == 0 {
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(DesktopCredentialError::StoreUnavailable)
            };
        }
        let outcome = unsafe {
            let credential = &*raw;
            let username = from_wide(credential.UserName);
            let size = credential.CredentialBlobSize as usize;
            let outcome = if credential.CredentialBlob.is_null() {
                inspect(&username, &[])
            } else {
                inspect(
                    &username,
                    std::slice::from_raw_parts(credential.CredentialBlob, size),
                )
            };
            if !credential.CredentialBlob.is_null() {
                ptr::write_bytes(credential.CredentialBlob, 0, size);
            }
            outcome
        };
        unsafe { CredFree(raw.cast()) };
        outcome.map(Some)
    }

    fn write_target_blob(
        target_name: &str,
        username: &str,
        blob: &mut [u8],
    ) -> Result<(), DesktopCredentialError> {
        let mut target = to_wide(target_name);
        let mut username = to_wide(username);
        let credential = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            Comment: ptr::null_mut(),
            LastWritten: FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            },
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_ENTERPRISE,
            AttributeCount: 0,
            Attributes: ptr::null_mut(),
            TargetAlias: ptr::null_mut(),
            UserName: username.as_mut_ptr(),
        };
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            let err = unsafe { GetLastError() };
            eprintln!(
                "WIN_CRED_WRITE_FAILED target={} blob_bytes={} win32_err={}{}",
                target_name,
                blob.len(),
                err,
                if err == 1783 {
                    " (RPC_X_BAD_STUB_DATA/blob_too_large)"
                } else {
                    ""
                }
            );
            Err(DesktopCredentialError::StoreUnavailable)
        } else {
            Ok(())
        }
    }

    impl RawTargetStore for WinCredStore {
        fn with_raw_target<T>(
            &self,
            target: &str,
            inspect: impl FnOnce(&str, &[u8]) -> Result<T, DesktopCredentialError>,
        ) -> Result<Option<T>, DesktopCredentialError> {
            with_target(target, inspect)
        }

        fn write_raw_target(
            &self,
            target: &str,
            username: &str,
            blob: &mut [u8],
        ) -> Result<(), DesktopCredentialError> {
            write_target_blob(target, username, blob)
        }

        fn delete_raw_target(&self, target: &str) -> Result<bool, DesktopCredentialError> {
            delete_target(target)
        }
    }

    /// Writes raw blob bytes, so tests can store a value no decoder accepts.
    #[cfg(test)]
    pub(super) fn write_raw_for_test(
        target: &str,
        username: &str,
        blob: &[u8],
    ) -> Result<(), DesktopCredentialError> {
        write_target_blob(target, username, &mut blob.to_vec())
    }

    fn delete_target(target: &str) -> Result<bool, DesktopCredentialError> {
        let target = to_wide(target);
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
            return Ok(true);
        }
        if unsafe { GetLastError() } == ERROR_NOT_FOUND {
            Ok(false)
        } else {
            Err(DesktopCredentialError::StoreUnavailable)
        }
    }

    fn matching_target(
        target: &str,
        username: &str,
    ) -> Result<Option<String>, DesktopCredentialError> {
        Ok(
            chunked_secret::read_secret_owned(&WinCredStore, target, Some(username))?
                .map(|(_, secret)| secret),
        )
    }

    pub(super) fn get(
        service: &str,
        username: &str,
    ) -> Result<Option<String>, DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Ok(None);
        }
        // Python keyring's WinVaultKeyring first uses the bare service as the
        // target, then falls back to `{username}@{service}` after a collision.
        if let Some(password) = matching_target(service, username)? {
            return Ok(Some(password));
        }
        let compound = format!("{username}@{service}");
        if let Some(password) = matching_target(&compound, username)? {
            return Ok(Some(password));
        }

        // Also read entries written by Rust keyring's default Windows target
        // convention (`{username}.{service}`) during the migration window.
        let rust_target = format!("{username}.{service}");
        matching_target(&rust_target, username)
    }

    pub(super) fn set(
        service: &str,
        username: &str,
        password: &str,
    ) -> Result<(), DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        // Match Python keyring's stable Windows target layout so older builds
        // and the current Tauri runtime share exactly the same credentials.
        chunked_secret::set_with_collision_move(&WinCredStore, service, username, password)
    }

    /// Deletes every target layout that belongs to `username`, matching on the
    /// target name and stored user name only; the secret is never decoded.
    pub(super) fn delete(service: &str, username: &str) -> Result<bool, DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        let mut removed = false;
        for target in [
            service.to_owned(),
            format!("{username}@{service}"),
            format!("{username}.{service}"),
        ] {
            removed |= chunked_secret::delete_secret_owned(&WinCredStore, &target, username)?;
        }
        Ok(removed)
    }

    /// Exact-target access, without the Python layout fallbacks. One read: the
    /// stored user name is compared first, and another owner's secret is never
    /// decoded.
    pub(super) fn read_owned(
        target: &str,
        username: &str,
    ) -> Result<Option<String>, DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Ok(None);
        }
        Ok(
            chunked_secret::read_secret_owned(&WinCredStore, target, Some(username))?
                .map(|(_, secret)| secret),
        )
    }

    pub(super) fn write_owned(
        target: &str,
        username: &str,
        password: &str,
    ) -> Result<(), DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        chunked_secret::write_secret(&WinCredStore, target, username, password)
    }

    /// Deletes `target` only when its stored user name is `username`; the
    /// secret is never decoded.
    pub(super) fn delete_owned(
        target: &str,
        username: &str,
    ) -> Result<bool, DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        chunked_secret::delete_secret_owned(&WinCredStore, target, username)
    }
}

#[cfg(target_os = "macos")]
mod platform_keyring {
    use std::collections::BTreeMap;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    use base64::Engine as _;
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
    use ring::rand::{SecureRandom, SystemRandom};
    use zeroize::Zeroizing;

    use super::DesktopCredentialError;

    type VaultMap = BTreeMap<String, BTreeMap<String, String>>;

    const KEYCHAIN_MASTER_SERVICE: &str = "org.eostage.desktop.vault";
    const KEYCHAIN_MASTER_ACCOUNT_PREFIX: &str = "master-key-v1";
    const VAULT_ENVELOPE_VERSION: u8 = 2;
    const VAULT_ENVELOPE_ALG: &str = "aes-256-gcm";
    const VAULT_AAD: &[u8] = b"eostage-mac-vault-v2";

    #[derive(serde::Serialize, serde::Deserialize)]
    struct EncryptedVaultEnvelope {
        v: u8,
        alg: String,
        nonce: String,
        ciphertext: String,
    }

    pub(super) trait MasterKeyStore {
        fn read_key(&self) -> Result<Option<Zeroizing<[u8; 32]>>, DesktopCredentialError>;
        fn write_key(&self, key: &[u8; 32]) -> Result<(), DesktopCredentialError>;
    }

    pub(super) struct MacKeychainMasterKeyStore {
        pub(super) account: String,
    }

    impl MacKeychainMasterKeyStore {
        pub(super) fn canonicalize_with_missing_suffix(path: &Path) -> PathBuf {
            let mut missing_suffix: Vec<std::ffi::OsString> = Vec::new();
            let mut cursor = path;
            while !cursor.exists() {
                if let Some(name) = cursor.file_name() {
                    missing_suffix.push(name.to_os_string());
                } else {
                    break;
                }
                match cursor.parent() {
                    Some(parent) if !parent.as_os_str().is_empty() => cursor = parent,
                    _ => break,
                }
            }
            let mut base = fs::canonicalize(cursor).unwrap_or_else(|_| cursor.to_path_buf());
            for seg in missing_suffix.into_iter().rev() {
                base.push(seg);
            }
            base
        }

        pub(super) fn for_vault(vault_path: &Path) -> Self {
            use sha2::{Digest, Sha256};
            let normalized = Self::canonicalize_with_missing_suffix(vault_path);
            let digest = Sha256::digest(normalized.to_string_lossy().as_bytes());
            let mut suffix = String::with_capacity(24);
            for byte in &digest[..12] {
                use std::fmt::Write as _;
                let _ = write!(&mut suffix, "{byte:02x}");
            }
            Self {
                account: format!("{KEYCHAIN_MASTER_ACCOUNT_PREFIX}-{suffix}"),
            }
        }
    }

    impl MasterKeyStore for MacKeychainMasterKeyStore {
        fn read_key(&self) -> Result<Option<Zeroizing<[u8; 32]>>, DesktopCredentialError> {
            let entry = keyring::Entry::new(KEYCHAIN_MASTER_SERVICE, &self.account)
                .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
            match entry.get_password() {
                Ok(raw_b64) => {
                    let raw_b64 = Zeroizing::new(raw_b64);
                    decode_master_key_b64(raw_b64.trim()).map(Some)
                }
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(DesktopCredentialError::StoreUnavailable),
            }
        }

        fn write_key(&self, key: &[u8; 32]) -> Result<(), DesktopCredentialError> {
            let entry = keyring::Entry::new(KEYCHAIN_MASTER_SERVICE, &self.account)
                .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
            let encoded = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(key));
            entry
                .set_password(&encoded)
                .map_err(|_| DesktopCredentialError::StoreUnavailable)
        }
    }

    /// Sidecar key store used exclusively for unit tests (`#[cfg(test)]`) and
    /// explicitly isolated vault paths (`EOSTAGE_MAC_CREDENTIAL_VAULT_PATH`),
    /// exercising real AES-256-GCM encryption and restart persistence without
    /// touching or prompting the user's real macOS login Keychain.
    pub(super) struct SidecarFileMasterKeyStore {
        key_path: PathBuf,
    }

    impl SidecarFileMasterKeyStore {
        pub(super) fn for_vault(vault_path: &Path) -> Self {
            let mut os = vault_path.as_os_str().to_os_string();
            os.push(".key");
            Self {
                key_path: PathBuf::from(os),
            }
        }
    }

    impl MasterKeyStore for SidecarFileMasterKeyStore {
        fn read_key(&self) -> Result<Option<Zeroizing<[u8; 32]>>, DesktopCredentialError> {
            match fs::read_to_string(&self.key_path) {
                Ok(raw) => {
                    let raw = Zeroizing::new(raw);
                    if raw.trim().is_empty() {
                        return Ok(None);
                    }
                    decode_master_key_b64(raw.trim()).map(Some)
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(_) => Err(DesktopCredentialError::StoreUnavailable),
            }
        }

        fn write_key(&self, key: &[u8; 32]) -> Result<(), DesktopCredentialError> {
            let encoded = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(key));
            atomic_write_0600(&self.key_path, encoded.as_bytes())
        }
    }

    enum ActiveKeyStore {
        Keychain(MacKeychainMasterKeyStore),
        Sidecar(SidecarFileMasterKeyStore),
    }

    impl MasterKeyStore for ActiveKeyStore {
        fn read_key(&self) -> Result<Option<Zeroizing<[u8; 32]>>, DesktopCredentialError> {
            match self {
                Self::Keychain(s) => s.read_key(),
                Self::Sidecar(s) => s.read_key(),
            }
        }

        fn write_key(&self, key: &[u8; 32]) -> Result<(), DesktopCredentialError> {
            match self {
                Self::Keychain(s) => s.write_key(key),
                Self::Sidecar(s) => s.write_key(key),
            }
        }
    }

    fn decode_master_key_b64(raw_b64: &str) -> Result<Zeroizing<[u8; 32]>, DesktopCredentialError> {
        let decoded = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(raw_b64)
                .map_err(|_| DesktopCredentialError::StoreUnavailable)?,
        );
        if decoded.len() != 32 {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        let mut out = Zeroizing::new([0u8; 32]);
        out.copy_from_slice(&decoded);
        Ok(out)
    }

    fn atomic_write_0600(path: &Path, bytes: &[u8]) -> Result<(), DesktopCredentialError> {
        let parent = path
            .parent()
            .ok_or(DesktopCredentialError::StoreUnavailable)?;
        if !parent.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        }
        static MAC_VAULT_TMP_SEQ: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("vault");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for _attempt in 0..8_u8 {
            let seq = MAC_VAULT_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tmp_path = parent.join(format!(
                ".{file_name}.tmp-{}-{}-{}",
                std::process::id(),
                nanos,
                seq
            ));
            let mut file = match OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&tmp_path)
            {
                Ok(f) => f,
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(DesktopCredentialError::StoreUnavailable),
            };
            let write_res = (|| -> Result<(), std::io::Error> {
                file.write_all(bytes)?;
                file.sync_all()?;
                drop(file);
                fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
                fs::rename(&tmp_path, path)?;
                Ok(())
            })();
            if write_res.is_err() {
                let _ = fs::remove_file(&tmp_path);
                return Err(DesktopCredentialError::StoreUnavailable);
            }
            return Ok(());
        }
        Err(DesktopCredentialError::StoreUnavailable)
    }

    fn active_key_store(vault_path: &Path) -> ActiveKeyStore {
        let has_override = std::env::var_os("EOSTAGE_MAC_CREDENTIAL_VAULT_PATH")
            .is_some_and(|v| !v.is_empty());
        if cfg!(test) || has_override {
            ActiveKeyStore::Sidecar(SidecarFileMasterKeyStore::for_vault(vault_path))
        } else {
            ActiveKeyStore::Keychain(MacKeychainMasterKeyStore::for_vault(vault_path))
        }
    }

    pub(super) type VaultCache = Mutex<Option<(PathBuf, VaultMap)>>;
    type MasterKeyCache = Mutex<Option<(PathBuf, Zeroizing<[u8; 32]>)>>;

    fn store_lock() -> &'static VaultCache {
        static STORE: OnceLock<VaultCache> = OnceLock::new();
        STORE.get_or_init(|| Mutex::new(None))
    }

    fn master_key_cache() -> &'static MasterKeyCache {
        static KEY_CACHE: OnceLock<MasterKeyCache> = OnceLock::new();
        KEY_CACHE.get_or_init(|| Mutex::new(None))
    }

    fn load_or_create_master_key(
        vault_path: &Path,
        store: &dyn MasterKeyStore,
        allow_create: bool,
        use_process_cache: bool,
    ) -> Result<Zeroizing<[u8; 32]>, DesktopCredentialError> {
        if use_process_cache {
            let guard = master_key_cache()
                .lock()
                .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
            if let Some((cached_path, cached_key)) = guard.as_ref() {
                if cached_path == vault_path {
                    return Ok(cached_key.clone());
                }
            }
        }
        let key = match store.read_key()? {
            Some(existing) => existing,
            None => {
                if !allow_create {
                    return Err(DesktopCredentialError::StoreUnavailable);
                }
                let mut generated = Zeroizing::new([0u8; 32]);
                SystemRandom::new()
                    .fill(&mut *generated)
                    .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
                store.write_key(&generated)?;
                generated
            }
        };
        if use_process_cache {
            let mut guard = master_key_cache()
                .lock()
                .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
            *guard = Some((vault_path.to_path_buf(), key.clone()));
        }
        Ok(key)
    }

    fn encrypt_vault_envelope(
        vault: &VaultMap,
        key_bytes: &[u8; 32],
    ) -> Result<Vec<u8>, DesktopCredentialError> {
        let mut plaintext = Zeroizing::new(
            serde_json::to_vec(vault).map_err(|_| DesktopCredentialError::StoreUnavailable)?,
        );
        let mut nonce_bytes = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let unbound = UnboundKey::new(&AES_256_GCM, key_bytes)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let sealing_key = LessSafeKey::new(unbound);
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        sealing_key
            .seal_in_place_append_tag(nonce, Aad::from(VAULT_AAD), &mut *plaintext)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let envelope = EncryptedVaultEnvelope {
            v: VAULT_ENVELOPE_VERSION,
            alg: VAULT_ENVELOPE_ALG.to_owned(),
            nonce: base64::engine::general_purpose::STANDARD.encode(nonce_bytes),
            ciphertext: base64::engine::general_purpose::STANDARD.encode(&*plaintext),
        };
        serde_json::to_vec_pretty(&envelope).map_err(|_| DesktopCredentialError::StoreUnavailable)
    }

    fn decrypt_vault_envelope(
        envelope: &EncryptedVaultEnvelope,
        key_bytes: &[u8; 32],
    ) -> Result<VaultMap, DesktopCredentialError> {
        if envelope.v != VAULT_ENVELOPE_VERSION || envelope.alg != VAULT_ENVELOPE_ALG {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        let nonce_vec = base64::engine::general_purpose::STANDARD
            .decode(envelope.nonce.trim())
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        if nonce_vec.len() != NONCE_LEN {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        let mut nonce_arr = [0u8; NONCE_LEN];
        nonce_arr.copy_from_slice(&nonce_vec);
        let mut in_out = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(envelope.ciphertext.trim())
                .map_err(|_| DesktopCredentialError::StoreUnavailable)?,
        );
        let unbound = UnboundKey::new(&AES_256_GCM, key_bytes)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let opening_key = LessSafeKey::new(unbound);
        let nonce = Nonce::assume_unique_for_key(nonce_arr);
        let plaintext = opening_key
            .open_in_place(nonce, Aad::from(VAULT_AAD), &mut in_out)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        serde_json::from_slice::<VaultMap>(plaintext)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)
    }

    fn vault_path() -> Result<PathBuf, DesktopCredentialError> {
        if let Some(override_path) = std::env::var_os("EOSTAGE_MAC_CREDENTIAL_VAULT_PATH") {
            let path = PathBuf::from(override_path);
            if !path.as_os_str().is_empty() {
                return Ok(path);
            }
        }
        #[cfg(test)]
        {
            Ok(std::env::temp_dir().join(format!(
                "eostage-unit-test-credential-vault-{}.json",
                std::process::id()
            )))
        }
        #[cfg(not(test))]
        {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or(DesktopCredentialError::StoreUnavailable)?;
            Ok(home
                .join("Library")
                .join("Application Support")
                .join("EOStage")
                .join("v3")
                .join("credentials-vault.v1.json"))
        }
    }

    pub(super) fn load_vault_from_disk_with_store(
        path: &Path,
        store: &dyn MasterKeyStore,
        use_process_cache: bool,
    ) -> Result<(VaultMap, bool), DesktopCredentialError> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok((VaultMap::new(), false));
            }
            Err(_) => return Err(DesktopCredentialError::StoreUnavailable),
        };
        if raw.trim().is_empty() {
            return Ok((VaultMap::new(), true));
        }
        let parsed_val: serde_json::Value =
            serde_json::from_str(&raw).map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let is_encrypted_envelope = parsed_val
            .as_object()
            .and_then(|obj| obj.get("v"))
            .is_some_and(|v| v.is_u64());
        if is_encrypted_envelope {
            let envelope: EncryptedVaultEnvelope = serde_json::from_value(parsed_val)
                .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
            let key = load_or_create_master_key(path, store, false, use_process_cache)?;
            let vault = decrypt_vault_envelope(&envelope, &key)?;
            return Ok((vault, true));
        }
        // Legacy plaintext VaultMap: transparently migrate in place to AES-256-GCM
        // v2 envelope. If Keychain/master key store is unavailable, fail closed
        // without touching the legacy plaintext file on disk.
        let legacy_vault: VaultMap = serde_json::from_value(parsed_val)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let key = load_or_create_master_key(path, store, true, use_process_cache)?;
        let encrypted_bytes = encrypt_vault_envelope(&legacy_vault, &key)?;
        atomic_write_0600(path, &encrypted_bytes)?;
        Ok((legacy_vault, true))
    }

    pub(super) fn save_vault_to_disk_with_store(
        path: &Path,
        vault: &VaultMap,
        store: &dyn MasterKeyStore,
        use_process_cache: bool,
    ) -> Result<(), DesktopCredentialError> {
        let key = load_or_create_master_key(path, store, true, use_process_cache)?;
        let encrypted_bytes = encrypt_vault_envelope(vault, &key)?;
        atomic_write_0600(path, &encrypted_bytes)
    }

    /// Cross-process advisory exclusive lock (`flock(LOCK_EX)`) serializing
    /// first-time Keychain master-key creation, legacy plaintext migration,
    /// and encrypted read-modify-write cycles across concurrent app processes.
    struct VaultFileLock {
        _file: File,
    }

    impl VaultFileLock {
        fn acquire(vault_path: &Path) -> Result<Self, DesktopCredentialError> {
            let parent = vault_path
                .parent()
                .ok_or(DesktopCredentialError::StoreUnavailable)?;
            if !parent.exists() {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(parent)
                    .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
            }
            let mut lock_os = vault_path.as_os_str().to_os_string();
            lock_os.push(".lock");
            let lock_path = PathBuf::from(lock_os);
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .mode(0o600)
                .open(&lock_path)
                .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if rc != 0 {
                return Err(DesktopCredentialError::StoreUnavailable);
            }
            Ok(Self { _file: file })
        }
    }

    pub(super) fn with_vault_using_store<T>(
        cache: &VaultCache,
        current_path: &Path,
        store: &dyn MasterKeyStore,
        use_process_cache: bool,
        mutate: bool,
        f: impl FnOnce(&mut VaultMap) -> Result<(T, bool), DesktopCredentialError>,
    ) -> Result<T, DesktopCredentialError> {
        let mut guard = cache
            .lock()
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        if mutate {
            let _cross_proc_lock = VaultFileLock::acquire(current_path)?;
            let (mut vault, existed) =
                load_vault_from_disk_with_store(current_path, store, use_process_cache)?;
            let (result, changed) = f(&mut vault)?;
            if changed {
                save_vault_to_disk_with_store(current_path, &vault, store, use_process_cache)?;
                *guard = Some((current_path.to_path_buf(), vault));
            } else if existed {
                *guard = Some((current_path.to_path_buf(), vault));
            }
            return Ok(result);
        }
        let path_changed =
            guard.as_ref().map(|(cached_path, _)| cached_path.as_path()) != Some(current_path);
        if path_changed {
            if !current_path.exists() {
                // Fresh install / no vault on disk: do not create lock files,
                // do not cache a stale empty snapshot forever if another
                // process later creates the file, and never touch Keychain.
                let mut empty = VaultMap::new();
                let (result, _) = f(&mut empty)?;
                return Ok(result);
            }
            let _cross_proc_lock = VaultFileLock::acquire(current_path)?;
            let (vault, existed) =
                load_vault_from_disk_with_store(current_path, store, use_process_cache)?;
            if !existed {
                let mut empty = VaultMap::new();
                let (result, _) = f(&mut empty)?;
                return Ok(result);
            }
            *guard = Some((current_path.to_path_buf(), vault));
        }
        let (_, vault) = guard
            .as_mut()
            .ok_or(DesktopCredentialError::StoreUnavailable)?;
        let (result, _) = f(vault)?;
        Ok(result)
    }

    fn with_vault<T>(
        mutate: bool,
        f: impl FnOnce(&mut VaultMap) -> Result<(T, bool), DesktopCredentialError>,
    ) -> Result<T, DesktopCredentialError> {
        let current_path = vault_path()?;
        let store = active_key_store(&current_path);
        with_vault_using_store(store_lock(), &current_path, &store, true, mutate, f)
    }

    pub(super) fn get_raw(
        service: &str,
        username: &str,
    ) -> Result<Option<String>, DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Ok(None);
        }
        with_vault(false, |vault| {
            let value = vault
                .get(service)
                .and_then(|accounts| accounts.get(username))
                .cloned();
            Ok((value, false))
        })
    }

    pub(super) fn get(
        service: &str,
        username: &str,
    ) -> Result<Option<String>, DesktopCredentialError> {
        Ok(get_raw(service, username)?.filter(|value| !value.trim().is_empty()))
    }

    pub(super) fn set(
        service: &str,
        username: &str,
        password: &str,
    ) -> Result<(), DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        with_vault(true, |vault| {
            vault
                .entry(service.to_owned())
                .or_default()
                .insert(username.to_owned(), password.to_owned());
            Ok(((), true))
        })
    }

    pub(super) fn delete(service: &str, username: &str) -> Result<bool, DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        with_vault(true, |vault| {
            let mut removed = false;
            if let Some(accounts) = vault.get_mut(service) {
                removed = accounts.remove(username).is_some();
                if accounts.is_empty() {
                    vault.remove(service);
                }
            }
            Ok((removed, removed))
        })
    }

    // The platform keyring keys entries by service and account already.
    pub(super) use self::{delete as delete_owned, get as read_owned, set as write_owned};

    pub(super) fn read_vault_file_readonly(
        path: &Path,
    ) -> Result<Option<serde_json::Value>, DesktopCredentialError> {
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(DesktopCredentialError::StoreUnavailable),
        };
        if !meta.file_type().is_file() {
            return Ok(None);
        }
        let raw = fs::read_to_string(path).map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        if raw.trim().is_empty() {
            return Ok(None);
        }
        let parsed_val: serde_json::Value =
            serde_json::from_str(&raw).map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let is_encrypted_envelope = parsed_val
            .as_object()
            .and_then(|obj| obj.get("v"))
            .is_some_and(|v| v.is_u64());
        if !is_encrypted_envelope {
            return Ok(Some(parsed_val));
        }
        let _cross_proc_lock = VaultFileLock::acquire(path)?;
        let envelope: EncryptedVaultEnvelope = serde_json::from_value(parsed_val)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let store = active_key_store(path);
        let key = load_or_create_master_key(path, &store, false, true)?;
        let vault = decrypt_vault_envelope(&envelope, &key)?;
        serde_json::to_value(vault)
            .map(Some)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)
    }

    pub(super) fn scrub_vault_file_services(
        path: &Path,
        should_remove: &dyn Fn(&str) -> bool,
    ) -> Result<bool, DesktopCredentialError> {
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err(DesktopCredentialError::StoreUnavailable),
        };
        if !meta.file_type().is_file() {
            return Ok(false);
        }
        // Always acquire `store_lock()` BEFORE `VaultFileLock::acquire` to match
        // `with_vault_using_store` lock ordering and prevent AB-BA deadlocks.
        let mut guard = store_lock()
            .lock()
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let _cross_proc_lock = VaultFileLock::acquire(path)?;
        let raw = fs::read_to_string(path).map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        if raw.trim().is_empty() {
            return Ok(false);
        }
        let mut parsed_val: serde_json::Value =
            serde_json::from_str(&raw).map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let is_encrypted_envelope = parsed_val
            .as_object()
            .and_then(|obj| obj.get("v"))
            .is_some_and(|v| v.is_u64());
        if is_encrypted_envelope {
            let envelope: EncryptedVaultEnvelope = serde_json::from_value(parsed_val)
                .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
            let store = active_key_store(path);
            let key = load_or_create_master_key(path, &store, false, true)?;
            let mut vault = decrypt_vault_envelope(&envelope, &key)?;
            let before_len = vault.len();
            vault.retain(|k, _| !should_remove(k));
            if vault.len() == before_len {
                return Ok(false);
            }
            let encrypted_bytes = encrypt_vault_envelope(&vault, &key)?;
            atomic_write_0600(path, &encrypted_bytes)?;
            if vault_path().ok().as_deref() == Some(path) {
                *guard = Some((path.to_path_buf(), vault));
            }
            return Ok(true);
        }
        let Some(obj) = parsed_val.as_object_mut() else {
            return Ok(false);
        };
        let before_len = obj.len();
        obj.retain(|k, _| !should_remove(k));
        if obj.len() == before_len {
            return Ok(false);
        }
        let serialized = serde_json::to_vec_pretty(&parsed_val)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        atomic_write_0600(path, &serialized)?;
        if vault_path().ok().as_deref() == Some(path) {
            *guard = None;
        }
        Ok(true)
    }
}

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
mod platform_keyring {
    use keyring::Entry;

    use super::DesktopCredentialError;

    pub(super) fn get(
        service: &str,
        username: &str,
    ) -> Result<Option<String>, DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Ok(None);
        }
        let entry =
            Entry::new(service, username).map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        Ok(entry
            .get_password()
            .ok()
            .filter(|value| !value.trim().is_empty()))
    }

    pub(super) fn set(
        service: &str,
        username: &str,
        password: &str,
    ) -> Result<(), DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        Entry::new(service, username)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?
            .set_password(password)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)
    }

    pub(super) fn delete(service: &str, username: &str) -> Result<bool, DesktopCredentialError> {
        if super::is_isolated_empty_keychain_mode() {
            return Err(DesktopCredentialError::StoreUnavailable);
        }
        let entry =
            Entry::new(service, username).map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        Ok(entry.delete_credential().is_ok())
    }

    // The platform keyring keys entries by service and account already.
    pub(super) use self::{delete as delete_owned, get as read_owned, set as write_owned};
}

fn keyring_get(service: &str, username: &str) -> Result<Option<String>, DesktopCredentialError> {
    Ok(platform_keyring::get(service, username)?.filter(|value| !value.trim().is_empty()))
}

fn keyring_set(service: &str, username: &str, value: &str) -> Result<(), DesktopCredentialError> {
    platform_keyring::set(service, username, value)
}

fn keyring_delete(service: &str, username: &str) -> Result<bool, DesktopCredentialError> {
    platform_keyring::delete(service, username)
}

fn keyring_earthdata_material() -> Result<CredentialMaterial, EarthdataCredentialError> {
    let _guard = desktop_login_pair_read_guard().map_err(|_| EarthdataCredentialError::Invalid)?;
    let token = keyring_get(EARTHDATA_KEYRING_SERVICE, EARTHDATA_TOKEN_KEY)
        .map_err(|_| EarthdataCredentialError::Missing)?;
    if let Some(token) = token {
        return CredentialMaterial::bearer(token).map_err(|_| EarthdataCredentialError::Invalid);
    }

    keyring_earthdata_login_material_unlocked()
}

fn keyring_earthdata_login_material() -> Result<CredentialMaterial, EarthdataCredentialError> {
    let _guard = desktop_login_pair_read_guard().map_err(|_| EarthdataCredentialError::Invalid)?;
    keyring_earthdata_login_material_unlocked()
}

fn keyring_earthdata_login_material_unlocked(
) -> Result<CredentialMaterial, EarthdataCredentialError> {
    let username = keyring_get(EARTHDATA_KEYRING_SERVICE, EARTHDATA_USERNAME_KEY)
        .map_err(|_| EarthdataCredentialError::Missing)?;
    let password = keyring_get(EARTHDATA_KEYRING_SERVICE, EARTHDATA_PASSWORD_KEY)
        .map_err(|_| EarthdataCredentialError::Missing)?;
    match (username, password) {
        (Some(username), Some(password)) => CredentialMaterial::basic(username, password)
            .map_err(|_| EarthdataCredentialError::Invalid),
        _ => Err(EarthdataCredentialError::Missing),
    }
}

fn resolve_system_earthdata_material(
    source: EarthdataCredentialSource,
    environment: impl Fn(&str) -> Option<String>,
    netrc_paths: &[PathBuf],
) -> Result<CredentialMaterial, EarthdataCredentialError> {
    if source == EarthdataCredentialSource::Auto {
        if let Ok(material) = keyring_earthdata_material() {
            return Ok(material);
        }
    }
    resolve_earthdata_material_with(source, environment, netrc_paths)
}

fn mask_username(username: &str) -> String {
    let trimmed = username.trim();
    if trimmed.chars().count() <= 1 {
        return "***".to_owned();
    }
    let first = trimmed.chars().next().unwrap_or('*');
    format!("{first}***")
}

fn mask_email(email: &str) -> String {
    let trimmed = email.trim();
    let Some((local, domain)) = trimmed.split_once('@') else {
        return "***".to_owned();
    };
    let first = local.chars().next().unwrap_or('*');
    format!("{first}***@{domain}")
}

fn keyring_earthdata_status() -> Result<String, DesktopCredentialError> {
    let _guard = desktop_login_pair_read_guard()?;
    if keyring_get(EARTHDATA_KEYRING_SERVICE, EARTHDATA_TOKEN_KEY)?.is_some() {
        return Ok("token".to_owned());
    }
    let username = keyring_get(EARTHDATA_KEYRING_SERVICE, EARTHDATA_USERNAME_KEY)?;
    let password = keyring_get(EARTHDATA_KEYRING_SERVICE, EARTHDATA_PASSWORD_KEY)?;
    if let (Some(username), Some(_)) = (username, password) {
        return Ok(format!("login:{}", mask_username(&username)));
    }
    Ok("none".to_owned())
}

/// Return the same privacy-safe Earthdata status as the Python desktop service.
/// The returned value never contains a token or password.
pub fn desktop_earthdata_status() -> String {
    if is_isolated_empty_keychain_mode() {
        return "none".to_owned();
    }
    let keyring_status = keyring_earthdata_status();
    if let Ok(status) = &keyring_status {
        if status != "none" {
            return status.clone();
        }
    }
    if std::env::var(TOKEN_ENV)
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return "env-token".to_owned();
    }
    if let (Some(username), Some(password)) = (
        std::env::var(USERNAME_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty()),
        std::env::var(PASSWORD_ENV)
            .ok()
            .filter(|value| !value.is_empty()),
    ) {
        let _ = password;
        return format!("login:{}", mask_username(&username));
    }
    let netrc_paths = default_netrc_paths();
    if netrc_material(&netrc_paths).is_ok() {
        return "netrc".to_owned();
    }
    if keyring_status.is_err() {
        "unavailable".to_owned()
    } else {
        "none".to_owned()
    }
}

/// Return a secret-free, local-only readiness result from the same
/// keyring/environment/.netrc precedence used by the production resolver.
/// This function never serializes or logs the resolved material and never
/// performs a remote authentication request.
pub fn desktop_earthdata_readiness() -> EarthdataCredentialReadiness {
    if is_isolated_empty_keychain_mode() {
        return EarthdataCredentialReadiness::Missing;
    }
    let keyring_result = match keyring_earthdata_status() {
        Ok(status) if status != "none" => keyring_earthdata_material(),
        Err(_) => Err(EarthdataCredentialError::NetrcRead),
        _ => Err(EarthdataCredentialError::Missing),
    };
    earthdata_readiness_from_materials(
        keyring_result,
        |key| std::env::var(key).ok(),
        &default_netrc_paths(),
    )
}

fn earthdata_readiness_from_materials(
    keyring_result: Result<CredentialMaterial, EarthdataCredentialError>,
    environment: impl Fn(&str) -> Option<String>,
    netrc_paths: &[PathBuf],
) -> EarthdataCredentialReadiness {
    let mut fallback = None;
    for result in [
        keyring_result,
        token_material(&environment),
        login_material(&environment),
        netrc_material(netrc_paths),
    ] {
        match result {
            Ok(material) => {
                drop(material);
                return EarthdataCredentialReadiness::Ready;
            }
            Err(error) => remember_readiness(&mut fallback, &error),
        }
    }
    fallback.unwrap_or(EarthdataCredentialReadiness::Missing)
}

fn remember_readiness(
    current: &mut Option<EarthdataCredentialReadiness>,
    error: &EarthdataCredentialError,
) {
    let candidate = match error {
        EarthdataCredentialError::Missing => EarthdataCredentialReadiness::Missing,
        EarthdataCredentialError::Invalid | EarthdataCredentialError::NetrcParse => {
            EarthdataCredentialReadiness::Invalid
        }
        EarthdataCredentialError::NetrcRead | EarthdataCredentialError::BurstCookieRequired => {
            EarthdataCredentialReadiness::Unavailable
        }
    };
    let should_replace = match current {
        None => true,
        Some(EarthdataCredentialReadiness::Missing) => true,
        Some(EarthdataCredentialReadiness::Unavailable)
            if candidate == EarthdataCredentialReadiness::Invalid =>
        {
            true
        }
        _ => false,
    };
    if should_replace {
        *current = Some(candidate);
    }
}

pub fn desktop_opentopography_status() -> String {
    match keyring_get(OPENTOPOGRAPHY_KEYRING_SERVICE, OPENTOPOGRAPHY_KEY) {
        Ok(Some(_)) => "set".to_owned(),
        Ok(None) => "none".to_owned(),
        Err(_) => "unavailable".to_owned(),
    }
}

/// Resolve the OpenTopography key only at the native provider boundary.  The
/// value is never projected to the UI, Store, task payload, or diagnostics.
pub fn resolve_desktop_opentopography_key() -> Result<Option<String>, DesktopCredentialError> {
    keyring_get(OPENTOPOGRAPHY_KEYRING_SERVICE, OPENTOPOGRAPHY_KEY)
}

mod agent_model;
pub use agent_model::{
    delete_desktop_agent_model_key, desktop_agent_model_key_ready, store_desktop_agent_model_key,
    with_desktop_agent_model_key,
};

/// Read the legacy `eostage:earthengine` entry (`(optional project_id, secret)`)
/// through the active credential store (decrypting macOS AES-256-GCM vaults
/// transparently after migration).
pub fn read_active_legacy_earthengine_entry(
) -> Result<Option<(Option<String>, String)>, DesktopCredentialError> {
    if is_isolated_empty_keychain_mode() {
        return Ok(None);
    }
    let Some(secret) = keyring_get(GEE_KEYRING_SERVICE, "secret")? else {
        return Ok(None);
    };
    let project_id = keyring_get(GEE_KEYRING_SERVICE, "project_id")?
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    Ok(Some((project_id, secret)))
}

/// Delete any legacy `eostage:earthengine` entries from the active credential
/// store (including encrypted macOS vaults).
pub fn clear_active_legacy_earthengine_entry() -> Result<bool, DesktopCredentialError> {
    if is_isolated_empty_keychain_mode() {
        return Err(DesktopCredentialError::StoreUnavailable);
    }
    let mut removed = false;
    for key in ["secret", "project_id", "token"] {
        removed |= keyring_delete(GEE_KEYRING_SERVICE, key)?;
    }
    Ok(removed)
}

/// Read an arbitrary `credentials-vault.v1.json` path as a JSON object without
/// migrating plaintext files or prompting Keychain for unencrypted fixtures.
/// Encrypted macOS v2 envelopes are transparently decrypted.
pub fn read_local_vault_map_readonly(
    vault_path: &std::path::Path,
) -> Result<Option<serde_json::Value>, DesktopCredentialError> {
    if is_isolated_empty_keychain_mode() {
        return Ok(None);
    }
    #[cfg(target_os = "macos")]
    {
        platform_keyring::read_vault_file_readonly(vault_path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let Ok(meta) = std::fs::symlink_metadata(vault_path) else {
            return Ok(None);
        };
        if !meta.file_type().is_file() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(vault_path)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        if raw.trim().is_empty() {
            return Ok(None);
        }
        serde_json::from_str::<serde_json::Value>(&raw)
            .map(Some)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)
    }
}

/// Atomically remove matching top-level service keys from a local
/// `credentials-vault.v1.json` file (supporting both legacy plaintext vaults
/// and AES-256-GCM encrypted macOS vaults, while refusing symlinks).
pub fn scrub_local_vault_services(
    vault_path: &std::path::Path,
    should_remove: impl Fn(&str) -> bool,
) -> Result<bool, DesktopCredentialError> {
    if is_isolated_empty_keychain_mode() {
        return Err(DesktopCredentialError::StoreUnavailable);
    }
    #[cfg(target_os = "macos")]
    {
        platform_keyring::scrub_vault_file_services(vault_path, &should_remove)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let Ok(meta) = std::fs::symlink_metadata(vault_path) else {
            return Ok(false);
        };
        if !meta.file_type().is_file() {
            return Ok(false);
        }
        let raw = std::fs::read_to_string(vault_path)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        if raw.trim().is_empty() {
            return Ok(false);
        }
        let mut parsed: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        let Some(obj) = parsed.as_object_mut() else {
            return Ok(false);
        };
        let before_len = obj.len();
        obj.retain(|k, _| !should_remove(k));
        if obj.len() == before_len {
            return Ok(false);
        }
        let serialized = serde_json::to_vec_pretty(&parsed)
            .map_err(|_| DesktopCredentialError::StoreUnavailable)?;
        static NON_MAC_SCRUB_TMP_SEQ: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let parent = vault_path
            .parent()
            .ok_or(DesktopCredentialError::StoreUnavailable)?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for _attempt in 0..8_u8 {
            let seq = NON_MAC_SCRUB_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tmp_path = parent.join(format!(
                ".credentials-vault.v1.json.tmp-{}-{}-{}",
                std::process::id(),
                nanos,
                seq
            ));
            let mut opts = std::fs::OpenOptions::new();
            opts.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut file = match opts.open(&tmp_path) {
                Ok(f) => f,
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(DesktopCredentialError::StoreUnavailable),
            };
            let write_res = (|| -> Result<(), std::io::Error> {
                use std::io::Write;
                file.write_all(&serialized)?;
                file.sync_all()?;
                drop(file);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
                }
                std::fs::rename(&tmp_path, vault_path)?;
                Ok(())
            })();
            if write_res.is_err() {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(DesktopCredentialError::StoreUnavailable);
            }
            return Ok(true);
        }
        Err(DesktopCredentialError::StoreUnavailable)
    }
}

/// Resolve the Settings email only inside a native execution closure. The
/// credential is zeroized on return and must never be projected to the renderer,
/// a frozen task, a log, or a persisted provider response.
pub fn with_desktop_gacos_email<T>(
    action: impl FnOnce(&str) -> T,
) -> Result<Option<T>, DesktopCredentialError> {
    let Some(email) = keyring_get(GACOS_KEYRING_SERVICE, GACOS_EMAIL_KEY)? else {
        return Ok(None);
    };
    let email = zeroize::Zeroizing::new(email);
    Ok(Some(action(email.as_str())))
}

fn gacos_result_link_key(fingerprint: &str) -> Result<String, DesktopCredentialError> {
    if fingerprint.len() != 64
        || !fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DesktopCredentialError::InvalidInput);
    }
    Ok(format!("result-link-{fingerprint}"))
}

/// A manually supplied result URL is a credential, not a task field. Keep it in
/// the existing current-user credential store so a restart can resume safely.
pub fn store_desktop_gacos_result_link(
    fingerprint: &str,
    raw: &str,
) -> Result<(), DesktopCredentialError> {
    use sha2::{Digest, Sha256};
    let key = gacos_result_link_key(fingerprint)?;
    insar_provider::gacos_delivery::validate_manual_result_url(raw)
        .map_err(|_| DesktopCredentialError::InvalidInput)?;
    if format!("{:x}", Sha256::digest(raw.as_bytes())) != fingerprint {
        return Err(DesktopCredentialError::InvalidInput);
    }
    keyring_set(GACOS_KEYRING_SERVICE, &key, raw)
}

/// Expose a delivery URL only to a scoped native transport callback. Never
/// return this value through IPC, reports, diagnostics or frozen task JSON.
pub fn with_desktop_gacos_result_link<T>(
    fingerprint: &str,
    action: impl FnOnce(&str) -> T,
) -> Result<Option<T>, DesktopCredentialError> {
    use sha2::{Digest, Sha256};
    let key = gacos_result_link_key(fingerprint)?;
    let Some(raw) = keyring_get(GACOS_KEYRING_SERVICE, &key)? else {
        return Ok(None);
    };
    let raw = zeroize::Zeroizing::new(raw);
    insar_provider::gacos_delivery::validate_manual_result_url(&raw)
        .map_err(|_| DesktopCredentialError::InvalidInput)?;
    if format!("{:x}", Sha256::digest(raw.as_bytes())) != fingerprint {
        return Err(DesktopCredentialError::InvalidInput);
    }
    Ok(Some(action(&raw)))
}

pub fn desktop_gacos_status() -> String {
    match keyring_get(GACOS_KEYRING_SERVICE, GACOS_EMAIL_KEY) {
        Ok(Some(email)) => mask_email(&email),
        Ok(None) => "none".to_owned(),
        Err(_) => "unavailable".to_owned(),
    }
}

pub fn store_desktop_earthdata_token(token: &str) -> Result<(), DesktopCredentialError> {
    let _guard = desktop_login_pair_write_guard()?;
    let token = token.trim();
    if token.is_empty() {
        return Err(DesktopCredentialError::InvalidInput);
    }
    keyring_set(EARTHDATA_KEYRING_SERVICE, EARTHDATA_TOKEN_KEY, token)?;
    let _ = keyring_delete(EARTHDATA_KEYRING_SERVICE, EARTHDATA_USERNAME_KEY);
    let _ = keyring_delete(EARTHDATA_KEYRING_SERVICE, EARTHDATA_PASSWORD_KEY);
    Ok(())
}

pub fn store_desktop_earthdata_login(
    username: &str,
    password: &str,
) -> Result<(), DesktopCredentialError> {
    let _guard = desktop_login_pair_write_guard()?;
    let username = username.trim();
    if username.is_empty() || password.is_empty() {
        return Err(DesktopCredentialError::InvalidInput);
    }
    keyring_set(EARTHDATA_KEYRING_SERVICE, EARTHDATA_USERNAME_KEY, username)?;
    keyring_set(EARTHDATA_KEYRING_SERVICE, EARTHDATA_PASSWORD_KEY, password)?;
    let _ = keyring_delete(EARTHDATA_KEYRING_SERVICE, EARTHDATA_TOKEN_KEY);
    Ok(())
}

pub fn clear_desktop_earthdata() -> Result<bool, DesktopCredentialError> {
    let _guard = desktop_login_pair_write_guard()?;
    let mut removed = false;
    for username in [
        EARTHDATA_TOKEN_KEY,
        EARTHDATA_USERNAME_KEY,
        EARTHDATA_PASSWORD_KEY,
    ] {
        removed |= keyring_delete(EARTHDATA_KEYRING_SERVICE, username)?;
    }
    Ok(removed)
}

fn desktop_login_pair_status(service: &str, username_key: &str, secret_key: &str) -> String {
    let Ok(_guard) = desktop_login_pair_read_guard() else {
        return "unavailable".to_owned();
    };
    let username = keyring_get(service, username_key);
    let secret = keyring_get(service, secret_key);
    match (username, secret) {
        (Ok(Some(username)), Ok(Some(_))) => format!("login:{}", mask_username(&username)),
        (Ok(None), Ok(None)) => "none".to_owned(),
        (Ok(Some(_)), Ok(None)) | (Ok(None), Ok(Some(_))) => "incomplete".to_owned(),
        _ => "unavailable".to_owned(),
    }
}

pub fn desktop_cdse_status() -> String {
    desktop_login_pair_status(CDSE_KEYRING_SERVICE, CDSE_USERNAME_KEY, CDSE_PASSWORD_KEY)
}

pub fn desktop_usgs_status() -> String {
    desktop_login_pair_status(
        USGS_KEYRING_SERVICE,
        USGS_USERNAME_KEY,
        USGS_APPLICATION_TOKEN_KEY,
    )
}

fn store_desktop_login_pair(
    service: &str,
    username_key: &str,
    secret_key: &str,
    username: &str,
    secret: &str,
) -> Result<(), DesktopCredentialError> {
    let username = username.trim();
    if username.is_empty() || secret.is_empty() {
        return Err(DesktopCredentialError::InvalidInput);
    }
    let _mutation = desktop_login_pair_write_guard()?;
    let previous_username = keyring_get(service, username_key)?;
    let previous_secret = keyring_get(service, secret_key)?;
    keyring_set(service, username_key, username)?;
    if let Err(error) = keyring_set(service, secret_key, secret) {
        match previous_username {
            Some(value) => {
                let _ = keyring_set(service, username_key, &value);
            }
            None => {
                let _ = keyring_delete(service, username_key);
            }
        }
        match previous_secret {
            Some(value) => {
                let _ = keyring_set(service, secret_key, &value);
            }
            None => {
                let _ = keyring_delete(service, secret_key);
            }
        }
        return Err(error);
    }
    Ok(())
}

fn clear_desktop_login_pair(
    service: &str,
    username_key: &str,
    secret_key: &str,
) -> Result<bool, DesktopCredentialError> {
    let _mutation = desktop_login_pair_write_guard()?;
    let mut removed = false;
    removed |= keyring_delete(service, username_key)?;
    removed |= keyring_delete(service, secret_key)?;
    Ok(removed)
}

fn resolve_desktop_login_pair(
    service: &str,
    username_key: &str,
    secret_key: &str,
) -> Result<(zeroize::Zeroizing<String>, zeroize::Zeroizing<String>), DesktopCredentialError> {
    let _guard = desktop_login_pair_read_guard()?;
    let username = keyring_get(service, username_key)?
        .filter(|value| !value.trim().is_empty())
        .ok_or(DesktopCredentialError::InvalidInput)?;
    let secret = keyring_get(service, secret_key)?
        .filter(|value| !value.is_empty())
        .ok_or(DesktopCredentialError::InvalidInput)?;
    Ok((
        zeroize::Zeroizing::new(username),
        zeroize::Zeroizing::new(secret),
    ))
}

pub fn store_desktop_cdse_login(
    username: &str,
    password: &str,
) -> Result<(), DesktopCredentialError> {
    store_desktop_login_pair(
        CDSE_KEYRING_SERVICE,
        CDSE_USERNAME_KEY,
        CDSE_PASSWORD_KEY,
        username,
        password,
    )
}

pub fn clear_desktop_cdse() -> Result<bool, DesktopCredentialError> {
    clear_desktop_login_pair(CDSE_KEYRING_SERVICE, CDSE_USERNAME_KEY, CDSE_PASSWORD_KEY)
}

pub fn resolve_desktop_cdse_login(
) -> Result<(zeroize::Zeroizing<String>, zeroize::Zeroizing<String>), DesktopCredentialError> {
    resolve_desktop_login_pair(CDSE_KEYRING_SERVICE, CDSE_USERNAME_KEY, CDSE_PASSWORD_KEY)
}

pub fn store_desktop_usgs_login(
    username: &str,
    application_token: &str,
) -> Result<(), DesktopCredentialError> {
    store_desktop_login_pair(
        USGS_KEYRING_SERVICE,
        USGS_USERNAME_KEY,
        USGS_APPLICATION_TOKEN_KEY,
        username,
        application_token,
    )
}

pub fn clear_desktop_usgs() -> Result<bool, DesktopCredentialError> {
    clear_desktop_login_pair(
        USGS_KEYRING_SERVICE,
        USGS_USERNAME_KEY,
        USGS_APPLICATION_TOKEN_KEY,
    )
}

pub fn resolve_desktop_usgs_login(
) -> Result<(zeroize::Zeroizing<String>, zeroize::Zeroizing<String>), DesktopCredentialError> {
    resolve_desktop_login_pair(
        USGS_KEYRING_SERVICE,
        USGS_USERNAME_KEY,
        USGS_APPLICATION_TOKEN_KEY,
    )
}

/// Return a secret-free Bhoonidhi login status from Windows Credential Manager
/// (or the platform keyring on non-Windows development hosts).
pub fn desktop_bhoonidhi_status() -> String {
    let username = keyring_get(BHOONIDHI_KEYRING_SERVICE, BHOONIDHI_USERNAME_KEY);
    let password = keyring_get(BHOONIDHI_KEYRING_SERVICE, BHOONIDHI_PASSWORD_KEY);
    match (username, password) {
        (Ok(Some(username)), Ok(Some(_))) => format!("login:{}", mask_username(&username)),
        (Ok(None), Ok(None)) => "none".to_owned(),
        _ => "unavailable".to_owned(),
    }
}

/// Read Bhoonidhi credentials only from the native desktop credential store.
/// The returned login material is short-lived and owns zeroizing secrets.
pub fn resolve_desktop_bhoonidhi_login() -> Result<BhoonidhiLoginMaterial, DesktopCredentialError> {
    let username = keyring_get(BHOONIDHI_KEYRING_SERVICE, BHOONIDHI_USERNAME_KEY)?
        .ok_or(DesktopCredentialError::StoreUnavailable)?;
    let password = keyring_get(BHOONIDHI_KEYRING_SERVICE, BHOONIDHI_PASSWORD_KEY)?
        .ok_or(DesktopCredentialError::StoreUnavailable)?;
    BhoonidhiLoginMaterial::new(username, password)
        .map_err(|_| DesktopCredentialError::InvalidInput)
}

pub fn store_desktop_bhoonidhi_login(
    username: &str,
    password: &str,
) -> Result<(), DesktopCredentialError> {
    let username = username.trim();
    if username.is_empty() || password.is_empty() {
        return Err(DesktopCredentialError::InvalidInput);
    }
    let _mutation = bhoonidhi_credential_write_guard()?;
    keyring_set(BHOONIDHI_KEYRING_SERVICE, BHOONIDHI_USERNAME_KEY, username)?;
    if let Err(error) = keyring_set(BHOONIDHI_KEYRING_SERVICE, BHOONIDHI_PASSWORD_KEY, password) {
        let _ = keyring_delete(BHOONIDHI_KEYRING_SERVICE, BHOONIDHI_USERNAME_KEY);
        return Err(error);
    }
    mark_bhoonidhi_credential_revision()?;
    Ok(())
}

pub fn clear_desktop_bhoonidhi() -> Result<bool, DesktopCredentialError> {
    let _mutation = bhoonidhi_credential_write_guard()?;
    let mut removed = false;
    for username in [BHOONIDHI_USERNAME_KEY, BHOONIDHI_PASSWORD_KEY] {
        removed |= keyring_delete(BHOONIDHI_KEYRING_SERVICE, username)?;
    }
    mark_bhoonidhi_credential_revision()?;
    Ok(removed)
}

pub fn store_desktop_opentopography_key(api_key: &str) -> Result<(), DesktopCredentialError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(DesktopCredentialError::InvalidInput);
    }
    keyring_set(OPENTOPOGRAPHY_KEYRING_SERVICE, OPENTOPOGRAPHY_KEY, api_key)
}

pub fn clear_desktop_opentopography_key() -> Result<bool, DesktopCredentialError> {
    keyring_delete(OPENTOPOGRAPHY_KEYRING_SERVICE, OPENTOPOGRAPHY_KEY)
}

pub fn store_desktop_gacos_email(email: &str) -> Result<(), DesktopCredentialError> {
    let email = email.trim();
    let valid = email.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
    });
    if !valid {
        return Err(DesktopCredentialError::InvalidInput);
    }
    keyring_set(GACOS_KEYRING_SERVICE, GACOS_EMAIL_KEY, email)
}

pub fn clear_desktop_gacos_email() -> Result<bool, DesktopCredentialError> {
    keyring_delete(GACOS_KEYRING_SERVICE, GACOS_EMAIL_KEY)
}

/// Status for Google Earth Engine (GEE) credentials in the native desktop store.
/// Returns "login:<project_id>", "login:<project_id>:<masked_email>",
/// "none", "incomplete", or "unavailable".
#[allow(dead_code)]
pub fn desktop_gee_status() -> String {
    let project_id = keyring_get(GEE_KEYRING_SERVICE, GEE_PROJECT_ID_KEY);
    let secret = keyring_get(GEE_KEYRING_SERVICE, GEE_SECRET_KEY);
    match (project_id, secret) {
        (Ok(Some(project_id)), Ok(Some(_))) => {
            if let Ok(Some(email)) = keyring_get(GEE_KEYRING_SERVICE, GEE_CLIENT_EMAIL_KEY) {
                format!("login:{}:{}", project_id, mask_email(&email))
            } else {
                format!("login:{}", project_id)
            }
        }
        (Ok(None), Ok(None)) => "none".to_owned(),
        (Ok(Some(_)), Ok(None)) | (Ok(None), Ok(Some(_))) => "incomplete".to_owned(),
        _ => "unavailable".to_owned(),
    }
}

#[allow(dead_code)]
pub fn store_desktop_gee_credentials(
    project_id: &str,
    auth_mode: &str,
    secret: &str,
    client_email: Option<&str>,
) -> Result<(), DesktopCredentialError> {
    let project_id = project_id.trim();
    let auth_mode = auth_mode.trim();
    if project_id.is_empty() || secret.is_empty() {
        return Err(DesktopCredentialError::InvalidInput);
    }
    keyring_set(GEE_KEYRING_SERVICE, GEE_PROJECT_ID_KEY, project_id)?;
    keyring_set(GEE_KEYRING_SERVICE, GEE_AUTH_MODE_KEY, auth_mode)?;
    keyring_set(GEE_KEYRING_SERVICE, GEE_SECRET_KEY, secret)?;
    if let Some(email) = client_email.map(str::trim).filter(|s| !s.is_empty()) {
        keyring_set(GEE_KEYRING_SERVICE, GEE_CLIENT_EMAIL_KEY, email)?;
    } else {
        let _ = keyring_delete(GEE_KEYRING_SERVICE, GEE_CLIENT_EMAIL_KEY);
    }
    Ok(())
}

#[allow(dead_code)]
pub fn resolve_desktop_gee_credentials(
) -> Result<(String, String, zeroize::Zeroizing<String>, Option<String>), DesktopCredentialError> {
    let project_id = keyring_get(GEE_KEYRING_SERVICE, GEE_PROJECT_ID_KEY)?
        .filter(|v| !v.trim().is_empty())
        .ok_or(DesktopCredentialError::InvalidInput)?;
    let auth_mode = keyring_get(GEE_KEYRING_SERVICE, GEE_AUTH_MODE_KEY)?
        .unwrap_or_else(|| "service_account".to_owned());
    let secret = keyring_get(GEE_KEYRING_SERVICE, GEE_SECRET_KEY)?
        .filter(|v| !v.is_empty())
        .ok_or(DesktopCredentialError::InvalidInput)?;
    let client_email =
        keyring_get(GEE_KEYRING_SERVICE, GEE_CLIENT_EMAIL_KEY)?.filter(|v| !v.trim().is_empty());
    Ok((
        project_id,
        auth_mode,
        zeroize::Zeroizing::new(secret),
        client_email,
    ))
}

#[allow(dead_code)]
pub fn clear_desktop_gee_credentials() -> Result<bool, DesktopCredentialError> {
    let mut removed = false;
    removed |= keyring_delete(GEE_KEYRING_SERVICE, GEE_PROJECT_ID_KEY)?;
    removed |= keyring_delete(GEE_KEYRING_SERVICE, GEE_AUTH_MODE_KEY)?;
    removed |= keyring_delete(GEE_KEYRING_SERVICE, GEE_SECRET_KEY)?;
    removed |= keyring_delete(GEE_KEYRING_SERVICE, GEE_CLIENT_EMAIL_KEY)?;
    Ok(removed)
}

impl EarthdataCredentialError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BurstCookieRequired => "ASF_BURST_COOKIE_REQUIRED",
            _ => "DL004",
        }
    }

    pub fn reason_zh(&self) -> &'static str {
        match self {
            Self::Missing => "未找到 Earthdata Token、账号密码或 .netrc 配置。",
            Self::Invalid => "Earthdata 凭据为空或不完整，请重新保存。",
            Self::NetrcRead => "无法读取 .netrc，请检查文件位置、大小和权限。",
            Self::NetrcParse => "无法解析 .netrc 中的 Earthdata 登录条目。",
            Self::BurstCookieRequired => {
                "Sentinel-1 Burst 下载需要账号密码或 .netrc 建立 asf-urs Cookie；仅 Token 不可用。"
            }
        }
    }
}

pub fn resolve_earthdata_burst_credential(
    netrc_path: Option<&Path>,
) -> Result<TransferAuth, EarthdataCredentialError> {
    let paths = netrc_path
        .map(|path| vec![path.to_path_buf()])
        .unwrap_or_else(default_netrc_paths);
    if let Ok(material) = keyring_earthdata_login_material() {
        return material_to_transfer_auth(material);
    }
    match resolve_earthdata_burst_credential_with(|key| std::env::var(key).ok(), &paths) {
        Err(EarthdataCredentialError::Missing)
            if keyring_get(EARTHDATA_KEYRING_SERVICE, EARTHDATA_TOKEN_KEY)
                .ok()
                .flatten()
                .is_some() =>
        {
            Err(EarthdataCredentialError::BurstCookieRequired)
        }
        result => result,
    }
}

pub fn resolve_earthdata_burst_credential_with(
    environment: impl Fn(&str) -> Option<String>,
    netrc_paths: &[PathBuf],
) -> Result<TransferAuth, EarthdataCredentialError> {
    resolve_earthdata_burst_material_with(environment, netrc_paths)
        .and_then(material_to_transfer_auth)
}

pub fn resolve_earthdata_credential_profile_with(
    profile: &str,
    environment: impl Fn(&str) -> Option<String>,
    netrc_paths: &[PathBuf],
) -> Result<CredentialMaterial, EarthdataCredentialError> {
    match profile {
        EARTHDATA_CREDENTIAL_PROFILE => resolve_earthdata_material_with(
            EarthdataCredentialSource::Auto,
            environment,
            netrc_paths,
        ),
        EARTHDATA_BURST_CREDENTIAL_PROFILE => {
            resolve_earthdata_burst_material_with(environment, netrc_paths)
        }
        _ => Err(EarthdataCredentialError::Missing),
    }
}

/// Test-only seam for the loopback legacy-search-token fixtures: exposes the
/// private credential-material projection to the sibling test module.
#[cfg(test)]
pub(crate) fn material_to_transfer_auth_for_test(
    material: CredentialMaterial,
) -> Result<TransferAuth, EarthdataCredentialError> {
    material_to_transfer_auth(material)
}

/// Resolve a bearer token usable for authenticated metadata search of the
/// restricted legacy SAR collections (ERS-1/2, RADARSAT-1, JERS-1). A stored
/// Earthdata token is returned directly; otherwise the saved login is
/// exchanged for a token through EDL's `find_or_create_token` endpoint using
/// the same verified session machinery as downloads. Returns the opaque token
/// string only; callers must never persist or log it.
pub fn resolve_earthdata_search_token() -> Result<String, EarthdataCredentialError> {
    match resolve_earthdata_credential(EarthdataCredentialSource::Auto, None) {
        Ok(TransferAuth::Bearer(token)) => {
            let token = crate::http_transfer::expose_transfer_auth_bearer(&token);
            if token.is_empty() {
                return Err(EarthdataCredentialError::Invalid);
            }
            Ok(token)
        }
        Ok(TransferAuth::Basic { username, password }) => {
            exchange_basic_login_for_search_token(TransferAuth::Basic { username, password })
        }
        _ => Err(EarthdataCredentialError::Missing),
    }
}

/// Exchange a resolved Basic Earthdata login for the bearer token that
/// authenticated metadata search of the restricted legacy collections needs,
/// through EDL's `find_or_create_token` session machinery. Split from
/// `resolve_earthdata_search_token` so tests can drive a fixture netrc login
/// through the real session code without touching live EDL endpoints or the
/// developer's own `.netrc`.
fn exchange_basic_login_for_search_token(
    credential: TransferAuth,
) -> Result<String, EarthdataCredentialError> {
    let transfer = crate::http_transfer::HttpTransfer::new(crate::TransferOptions::default())
        .map_err(|_| EarthdataCredentialError::Invalid)?;
    transfer
        .ensure_earthdata_session_with_credential(&credential)
        .map_err(|_| EarthdataCredentialError::Invalid)?;
    transfer
        .earthdata_session_token()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
        .ok_or(EarthdataCredentialError::Invalid)
}

pub fn resolve_earthdata_credential(
    source: EarthdataCredentialSource,
    netrc_path: Option<&Path>,
) -> Result<TransferAuth, EarthdataCredentialError> {
    if is_isolated_empty_keychain_mode() && netrc_path.is_none() {
        return Err(EarthdataCredentialError::Missing);
    }
    let paths = netrc_path
        .map(|path| vec![path.to_path_buf()])
        .unwrap_or_else(default_netrc_paths);
    if source == EarthdataCredentialSource::Auto {
        return resolve_system_earthdata_material(source, |key| std::env::var(key).ok(), &paths)
            .and_then(material_to_transfer_auth);
    }
    resolve_earthdata_credential_with(source, |key| std::env::var(key).ok(), &paths)
}

pub fn resolve_earthdata_credential_with(
    source: EarthdataCredentialSource,
    environment: impl Fn(&str) -> Option<String>,
    netrc_paths: &[PathBuf],
) -> Result<TransferAuth, EarthdataCredentialError> {
    resolve_earthdata_material_with(source, environment, netrc_paths)
        .and_then(material_to_transfer_auth)
}

fn resolve_earthdata_material_with(
    source: EarthdataCredentialSource,
    environment: impl Fn(&str) -> Option<String>,
    netrc_paths: &[PathBuf],
) -> Result<CredentialMaterial, EarthdataCredentialError> {
    match source {
        EarthdataCredentialSource::EnvironmentToken => token_material(&environment),
        EarthdataCredentialSource::EnvironmentLogin => login_material(&environment),
        EarthdataCredentialSource::Netrc => netrc_material(netrc_paths),
        EarthdataCredentialSource::Auto => token_material(&environment)
            .or_else(|_| login_material(&environment))
            .or_else(|_| netrc_material(netrc_paths))
            .map_err(|_| EarthdataCredentialError::Missing),
    }
}

fn resolve_earthdata_burst_material_with(
    environment: impl Fn(&str) -> Option<String>,
    netrc_paths: &[PathBuf],
) -> Result<CredentialMaterial, EarthdataCredentialError> {
    if let Ok(material) = login_material(&environment) {
        return Ok(material);
    }
    if let Ok(material) = netrc_material(netrc_paths) {
        return Ok(material);
    }
    if environment(TOKEN_ENV).is_some() {
        return Err(EarthdataCredentialError::BurstCookieRequired);
    }
    Err(EarthdataCredentialError::Missing)
}

fn material_to_transfer_auth(
    material: CredentialMaterial,
) -> Result<TransferAuth, EarthdataCredentialError> {
    material
        .into_transfer_auth()
        .map_err(|_| EarthdataCredentialError::Invalid)
}

fn token_material(
    environment: &impl Fn(&str) -> Option<String>,
) -> Result<CredentialMaterial, EarthdataCredentialError> {
    let token = environment(TOKEN_ENV).ok_or(EarthdataCredentialError::Missing)?;
    CredentialMaterial::bearer(token).map_err(|_| EarthdataCredentialError::Invalid)
}

fn login_material(
    environment: &impl Fn(&str) -> Option<String>,
) -> Result<CredentialMaterial, EarthdataCredentialError> {
    let username = environment(USERNAME_ENV).ok_or(EarthdataCredentialError::Missing)?;
    let password = environment(PASSWORD_ENV).ok_or(EarthdataCredentialError::Missing)?;
    CredentialMaterial::basic(username, password).map_err(|_| EarthdataCredentialError::Invalid)
}

fn is_opportunistic_windows_root_netrc(path: &Path) -> bool {
    let Some(raw) = path.to_str() else {
        return false;
    };
    raw.eq_ignore_ascii_case(r"C:\.netrc") || raw.eq_ignore_ascii_case(r"C:\_netrc")
}

fn netrc_material(paths: &[PathBuf]) -> Result<CredentialMaterial, EarthdataCredentialError> {
    let mut found_primary_file = false;
    let mut primary_read_err = false;
    let mut primary_parse_err = false;

    for path in paths {
        let is_opportunistic = is_opportunistic_windows_root_netrc(path);
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                if !is_opportunistic {
                    primary_read_err = true;
                }
                continue;
            }
        };
        if !metadata.is_file() || metadata.len() > MAX_NETRC_BYTES {
            if !is_opportunistic {
                primary_read_err = true;
            }
            continue;
        }
        let Ok(contents) = fs::read_to_string(path) else {
            if !is_opportunistic {
                primary_read_err = true;
            }
            continue;
        };
        if !is_opportunistic {
            found_primary_file = true;
        }
        match parse_netrc_entry(&contents, EDL_HOST) {
            Ok(Some((username, password))) => {
                if let Ok(material) = CredentialMaterial::basic(username, password) {
                    return Ok(material);
                }
                if !is_opportunistic {
                    primary_parse_err = true;
                }
            }
            Ok(None) => {}
            Err(_) => {
                if !is_opportunistic {
                    primary_parse_err = true;
                }
            }
        }
    }

    if primary_parse_err || found_primary_file {
        Err(EarthdataCredentialError::NetrcParse)
    } else if primary_read_err {
        Err(EarthdataCredentialError::NetrcRead)
    } else {
        Err(EarthdataCredentialError::Missing)
    }
}

pub struct EnvironmentEarthdataCredentialProvider;

impl EnvironmentEarthdataCredentialProvider {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for EnvironmentEarthdataCredentialProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialProvider for EnvironmentEarthdataCredentialProvider {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        if is_isolated_empty_keychain_mode() {
            return Err(CredentialProviderError::Missing);
        }
        let paths = default_netrc_paths();
        let resolved = match profile {
            EARTHDATA_CREDENTIAL_PROFILE => resolve_system_earthdata_material(
                EarthdataCredentialSource::Auto,
                |key| std::env::var(key).ok(),
                &paths,
            ),
            EARTHDATA_BURST_CREDENTIAL_PROFILE => {
                if let Ok(material) = keyring_earthdata_login_material() {
                    Ok(material)
                } else {
                    match resolve_earthdata_burst_material_with(
                        |key| std::env::var(key).ok(),
                        &paths,
                    ) {
                        Err(EarthdataCredentialError::Missing)
                            if keyring_get(EARTHDATA_KEYRING_SERVICE, EARTHDATA_TOKEN_KEY)
                                .ok()
                                .flatten()
                                .is_some() =>
                        {
                            Err(EarthdataCredentialError::BurstCookieRequired)
                        }
                        result => result,
                    }
                }
            }
            _ => return Err(CredentialProviderError::Missing),
        };
        resolved.map_err(|error| match error {
            EarthdataCredentialError::Missing => CredentialProviderError::Missing,
            EarthdataCredentialError::BurstCookieRequired => {
                CredentialProviderError::BurstCookieRequired
            }
            EarthdataCredentialError::Invalid
            | EarthdataCredentialError::NetrcRead
            | EarthdataCredentialError::NetrcParse => CredentialProviderError::Failed,
        })
    }
}

/// Routes only the two reviewed production credential profiles. Earthdata
/// material remains owned by its existing provider; CDSE material remains in
/// the caller-owned runtime OAuth session. Unknown profiles fail closed.
pub struct ProductionCredentialProvider {
    earthdata: Arc<dyn CredentialProvider>,
    cdse_session: Arc<CdseOAuthSession>,
}

impl ProductionCredentialProvider {
    pub fn new(
        earthdata: Arc<dyn CredentialProvider>,
        cdse_session: Arc<CdseOAuthSession>,
    ) -> Self {
        Self {
            earthdata,
            cdse_session,
        }
    }
}

impl CredentialProvider for ProductionCredentialProvider {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        match profile {
            EARTHDATA_CREDENTIAL_PROFILE | EARTHDATA_BURST_CREDENTIAL_PROFILE => {
                self.earthdata.resolve(profile)
            }
            CDSE_CREDENTIAL_PROFILE => self.cdse_session.resolve(profile),
            _ => Err(CredentialProviderError::Missing),
        }
    }
}

impl fmt::Debug for ProductionCredentialProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProductionCredentialProvider")
            .field("earthdata", &"[CONTROLLED]")
            .field("cdse_session", &"[REDACTED]")
            .finish()
    }
}

fn normalize_netrc_machine_host(raw: &str) -> &str {
    let trimmed = raw.trim();
    let without_scheme = if trimmed.len() >= 8 && trimmed[..8].eq_ignore_ascii_case("https://") {
        &trimmed[8..]
    } else if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("http://") {
        &trimmed[7..]
    } else {
        trimmed
    };
    without_scheme.trim_end_matches('/')
}

fn parse_netrc_entry(
    contents: &str,
    host: &str,
) -> Result<Option<(String, String)>, EarthdataCredentialError> {
    let tokens = tokenize(contents)?;
    let mut index = 0;
    while index < tokens.len() {
        if !tokens[index].eq_ignore_ascii_case("machine") {
            index += 1;
            continue;
        }
        let machine = tokens
            .get(index + 1)
            .ok_or(EarthdataCredentialError::NetrcParse)?;
        index += 2;
        let mut login = None;
        let mut password = None;
        while index < tokens.len()
            && !tokens[index].eq_ignore_ascii_case("machine")
            && !tokens[index].eq_ignore_ascii_case("default")
        {
            if tokens[index].eq_ignore_ascii_case("login") {
                login = tokens.get(index + 1).cloned();
                index += 2;
            } else if tokens[index].eq_ignore_ascii_case("password") {
                password = tokens.get(index + 1).cloned();
                index += 2;
            } else if tokens[index].eq_ignore_ascii_case("account") {
                index += 2;
            } else if tokens[index].eq_ignore_ascii_case("macdef") {
                return Err(EarthdataCredentialError::NetrcParse);
            } else {
                index += 1;
            }
        }
        if normalize_netrc_machine_host(machine).eq_ignore_ascii_case(host) {
            return match (login, password) {
                (Some(login), Some(password)) if !login.is_empty() && !password.is_empty() => {
                    Ok(Some((login, password)))
                }
                _ => Err(EarthdataCredentialError::NetrcParse),
            };
        }
    }
    Ok(None)
}

fn tokenize(contents: &str) -> Result<Vec<String>, EarthdataCredentialError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut characters = contents.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '#' if current.is_empty() => {
                for value in characters.by_ref() {
                    if value == '\n' {
                        break;
                    }
                }
            }
            '"' | '\'' if current.is_empty() => {
                let quote = character;
                let mut closed = false;
                while let Some(value) = characters.next() {
                    if value == quote {
                        closed = true;
                        break;
                    }
                    if value == '\\' {
                        current.push(
                            characters
                                .next()
                                .ok_or(EarthdataCredentialError::NetrcParse)?,
                        );
                    } else {
                        current.push(value);
                    }
                }
                if !closed {
                    return Err(EarthdataCredentialError::NetrcParse);
                }
            }
            value if value.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            value => current.push(value),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn push_unique_netrc_path(out: &mut Vec<PathBuf>, candidate: PathBuf, is_windows: bool) {
    let Some(candidate_str) = candidate.to_str() else {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
        return;
    };
    let already = out.iter().any(|existing| match existing.to_str() {
        Some(existing_str) if is_windows => existing_str.eq_ignore_ascii_case(candidate_str),
        Some(existing_str) => existing_str == candidate_str,
        None => existing == &candidate,
    });
    if !already {
        out.push(candidate);
    }
}

fn netrc_paths_from_env(
    is_windows: bool,
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(explicit) = env("NETRC") {
        let trimmed = explicit.to_string_lossy().trim().to_owned();
        if !trimmed.is_empty() {
            push_unique_netrc_path(&mut out, PathBuf::from(trimmed), is_windows);
        }
    }

    let mut home_dirs = Vec::new();
    for var in ["USERPROFILE", "HOME"] {
        if let Some(dir) = env(var) {
            if !dir.to_string_lossy().trim().is_empty() {
                home_dirs.push(PathBuf::from(dir));
            }
        }
    }
    if let (Some(drive), Some(path)) = (env("HOMEDRIVE"), env("HOMEPATH")) {
        let combined = format!(
            "{}{}",
            drive.to_string_lossy().trim(),
            path.to_string_lossy().trim()
        );
        if !combined.is_empty() {
            home_dirs.push(PathBuf::from(combined));
        }
    }

    for home in home_dirs {
        push_unique_netrc_path(&mut out, home.join(".netrc"), is_windows);
        push_unique_netrc_path(&mut out, home.join("_netrc"), is_windows);
    }

    if is_windows {
        push_unique_netrc_path(&mut out, PathBuf::from(r"C:\.netrc"), true);
        push_unique_netrc_path(&mut out, PathBuf::from(r"C:\_netrc"), true);
    }
    out
}

fn default_netrc_paths() -> Vec<PathBuf> {
    if is_isolated_empty_keychain_mode() {
        return Vec::new();
    }
    netrc_paths_from_env(cfg!(target_os = "windows"), |key| std::env::var_os(key))
}

#[cfg(test)]
mod tests {
    use super::{
        earthdata_readiness_from_materials, EarthdataCredentialError, EarthdataCredentialReadiness,
    };
    use std::path::PathBuf;

    #[test]
    fn valid_local_token_projects_ready_without_returning_material() {
        let readiness = earthdata_readiness_from_materials(
            Err(EarthdataCredentialError::Missing),
            |key| (key == "EARTHDATA_TOKEN").then(|| "test-token".to_owned()),
            &[],
        );
        assert_eq!(readiness, EarthdataCredentialReadiness::Ready);
    }

    #[test]
    fn missing_local_material_projects_missing() {
        let readiness = earthdata_readiness_from_materials(
            Err(EarthdataCredentialError::Missing),
            |_| None,
            &[],
        );
        assert_eq!(readiness, EarthdataCredentialReadiness::Missing);
    }

    #[test]
    fn malformed_local_material_projects_invalid() {
        let readiness = earthdata_readiness_from_materials(
            Err(EarthdataCredentialError::Missing),
            |key| (key == "EARTHDATA_TOKEN").then(|| "   ".to_owned()),
            &[PathBuf::from("missing-netrc-for-readiness-test")],
        );
        assert_eq!(readiness, EarthdataCredentialReadiness::Invalid);
    }

    #[test]
    fn netrc_candidate_discovery_deduplicates_and_includes_windows_roots() {
        let paths = super::netrc_paths_from_env(true, |key| match key {
            "NETRC" => Some(std::ffi::OsString::from(r"D:\creds\_netrc")),
            "USERPROFILE" => Some(std::ffi::OsString::from(r"C:\Users\Alice")),
            "HOME" => Some(std::ffi::OsString::from(r"c:\users\alice")),
            "HOMEDRIVE" => Some(std::ffi::OsString::from("C:")),
            "HOMEPATH" => Some(std::ffi::OsString::from(r"\Users\Alice")),
            _ => None,
        });
        assert_eq!(
            paths,
            vec![
                PathBuf::from(r"D:\creds\_netrc"),
                PathBuf::from(r"C:\Users\Alice").join(".netrc"),
                PathBuf::from(r"C:\Users\Alice").join("_netrc"),
                PathBuf::from(r"C:\.netrc"),
                PathBuf::from(r"C:\_netrc"),
            ]
        );
    }
}

#[cfg(test)]
mod gacos_result_credential_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn result_identity_and_host_fail_before_any_credential_store_access() {
        let raw = "https://www.gacos.net/results/example.zip";
        assert!(store_desktop_gacos_result_link("bad", raw).is_err());
        assert!(store_desktop_gacos_result_link(&"0".repeat(64), raw).is_err());
        let unrelated = "https://example.org/results/example.zip";
        let digest = format!("{:x}", Sha256::digest(unrelated.as_bytes()));
        assert!(store_desktop_gacos_result_link(&digest, unrelated).is_err());
        assert!(with_desktop_gacos_result_link("email", |_| panic!(
            "must not read another credential"
        ))
        .is_err());
        assert_eq!(
            gacos_result_link_key(&"a".repeat(64)).unwrap(),
            format!("result-link-{}", "a".repeat(64))
        );
    }
}

#[cfg(test)]
mod gee_credential_tests {
    use super::*;

    #[test]
    fn test_gee_keyring_lifecycle() {
        // Clear initially
        let _ = clear_desktop_gee_credentials();

        // 1. Initially status is none
        assert_eq!(desktop_gee_status(), "none");

        // 2. Reject empty inputs
        assert!(store_desktop_gee_credentials("", "oauth_token", "secret", None).is_err());
        assert!(store_desktop_gee_credentials("my-proj", "oauth_token", "", None).is_err());

        // 3. Store OAuth token
        store_desktop_gee_credentials("ee-test-proj", "oauth_token", "ya29.sample_token_123", None)
            .expect("store gee oauth credentials");
        assert_eq!(desktop_gee_status(), "login:ee-test-proj");

        let (proj, mode, sec, email) =
            resolve_desktop_gee_credentials().expect("resolve gee credentials");
        assert_eq!(proj, "ee-test-proj");
        assert_eq!(mode, "oauth_token");
        assert_eq!(&*sec, "ya29.sample_token_123");
        assert_eq!(email, None);

        // 4. Update with Service Account and email
        store_desktop_gee_credentials(
            "ee-test-proj-sa",
            "service_account",
            "{\"client_email\":\"my-sa@ee-test.iam.gserviceaccount.com\"}",
            Some("my-sa@ee-test.iam.gserviceaccount.com"),
        )
        .expect("store gee sa credentials");

        assert_eq!(
            desktop_gee_status(),
            "login:ee-test-proj-sa:m***@ee-test.iam.gserviceaccount.com"
        );

        let (proj2, mode2, sec2, email2) =
            resolve_desktop_gee_credentials().expect("resolve gee sa credentials");
        assert_eq!(proj2, "ee-test-proj-sa");
        assert_eq!(mode2, "service_account");
        assert!(sec2.contains("my-sa@ee-test.iam.gserviceaccount.com"));
        assert_eq!(
            email2.as_deref(),
            Some("my-sa@ee-test.iam.gserviceaccount.com")
        );

        // 4b. Verify EOSTAGE_ISOLATE_EMPTY_KEYCHAIN=1 masks all saved credentials AND blocks writes/deletes
        store_desktop_opentopography_key("ot-live-key-preserve").expect("seed opentopo key");
        store_desktop_agent_model_key("ch-preserve", "sk-live-preserve")
            .expect("seed agent model key");
        assert_eq!(desktop_opentopography_status(), "set");
        assert!(desktop_agent_model_key_ready("ch-preserve").expect("agent key ready"));

        assert!(parse_isolated_empty_keychain_flag("1"));
        assert!(parse_isolated_empty_keychain_flag(" true "));
        assert!(!parse_isolated_empty_keychain_flag("0"));
        assert!(!parse_isolated_empty_keychain_flag(""));

        struct IsolateEnvReset;
        impl Drop for IsolateEnvReset {
            fn drop(&mut self) {
                TEST_ISOLATE_EMPTY_KEYCHAIN_OVERRIDE.store(false, Ordering::SeqCst);
            }
        }
        {
            let _reset = IsolateEnvReset;
            TEST_ISOLATE_EMPTY_KEYCHAIN_OVERRIDE.store(true, Ordering::SeqCst);

            assert_eq!(desktop_gee_status(), "none");
            assert_eq!(desktop_cdse_status(), "none");
            assert_eq!(desktop_usgs_status(), "none");
            assert_eq!(desktop_bhoonidhi_status(), "none");
            assert_eq!(desktop_opentopography_status(), "none");
            assert_eq!(desktop_gacos_status(), "none");
            assert_eq!(desktop_earthdata_status(), "none");
            assert_eq!(
                desktop_earthdata_readiness(),
                EarthdataCredentialReadiness::Missing
            );
            assert!(!desktop_agent_model_key_ready("ch-preserve").expect("isolated agent key"));
            assert!(
                read_desktop_credential_for_edit(DesktopCredentialField::OpentopographyKey)
                    .expect("isolated read_for_edit")
                    .is_none()
            );

            // Any write or delete attempt in isolated mode MUST fail closed without touching real entries
            assert!(matches!(
                store_desktop_opentopography_key("ot-overwrite-attempt"),
                Err(DesktopCredentialError::StoreUnavailable)
            ));
            assert!(matches!(
                clear_desktop_opentopography_key(),
                Err(DesktopCredentialError::StoreUnavailable)
            ));
            assert!(matches!(
                clear_desktop_gee_credentials(),
                Err(DesktopCredentialError::StoreUnavailable)
            ));
            assert!(matches!(
                delete_desktop_agent_model_key("ch-preserve"),
                Err(DesktopCredentialError::StoreUnavailable)
            ));
        }

        // After leaving isolated mode, every pre-existing credential remains 100% intact
        assert_eq!(
            desktop_gee_status(),
            "login:ee-test-proj-sa:m***@ee-test.iam.gserviceaccount.com"
        );
        assert_eq!(desktop_opentopography_status(), "set");
        assert_eq!(
            resolve_desktop_opentopography_key()
                .expect("preserved opentopo key")
                .as_deref(),
            Some("ot-live-key-preserve")
        );
        assert_eq!(
            with_desktop_agent_model_key("ch-preserve", str::to_owned)
                .expect("preserved agent key"),
            Some("sk-live-preserve".to_owned())
        );
        let _ = clear_desktop_opentopography_key();
        let _ = delete_desktop_agent_model_key("ch-preserve");

        // 5. Clear credentials
        let cleared = clear_desktop_gee_credentials().expect("clear gee credentials");
        assert!(cleared);
        assert_eq!(desktop_gee_status(), "none");
        assert!(resolve_desktop_gee_credentials().is_err());
    }
}

#[cfg(test)]
mod chunked_secret_tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::chunked_secret::{
        chunk_target_name, delete_secret_owned, read_secret_owned, set_with_collision_move,
        write_secret, Manifest, RawTargetStore, MANIFEST_PREFIX,
    };
    use super::DesktopCredentialError;

    const WIN32_CRED_MAX_BLOB_BYTES: usize = 2560;

    struct MockWinCredStore {
        entries: Mutex<BTreeMap<String, (String, Vec<u8>)>>,
        fail_write_after: Mutex<Option<usize>>,
        writes: Mutex<usize>,
    }

    impl MockWinCredStore {
        fn new() -> Self {
            Self {
                entries: Mutex::new(BTreeMap::new()),
                fail_write_after: Mutex::new(None),
                writes: Mutex::new(0),
            }
        }

        fn target_count(&self) -> usize {
            self.entries.lock().unwrap().len()
        }

        fn keys(&self) -> Vec<String> {
            self.entries.lock().unwrap().keys().cloned().collect()
        }
    }

    impl RawTargetStore for MockWinCredStore {
        fn with_raw_target<T>(
            &self,
            target: &str,
            inspect: impl FnOnce(&str, &[u8]) -> Result<T, DesktopCredentialError>,
        ) -> Result<Option<T>, DesktopCredentialError> {
            let guard = self.entries.lock().unwrap();
            let Some((owner, blob)) = guard.get(target) else {
                return Ok(None);
            };
            inspect(owner, blob).map(Some)
        }

        fn write_raw_target(
            &self,
            target: &str,
            username: &str,
            blob: &mut [u8],
        ) -> Result<(), DesktopCredentialError> {
            {
                let mut count = self.writes.lock().unwrap();
                *count += 1;
                if let Some(limit) = *self.fail_write_after.lock().unwrap() {
                    if *count > limit {
                        return Err(DesktopCredentialError::StoreUnavailable);
                    }
                }
            }
            if blob.len() > WIN32_CRED_MAX_BLOB_BYTES {
                return Err(DesktopCredentialError::StoreUnavailable);
            }
            self.entries
                .lock()
                .unwrap()
                .insert(target.to_owned(), (username.to_owned(), blob.to_vec()));
            Ok(())
        }

        fn delete_raw_target(&self, target: &str) -> Result<bool, DesktopCredentialError> {
            Ok(self.entries.lock().unwrap().remove(target).is_some())
        }
    }

    #[test]
    fn short_secret_uses_single_entry_and_cleans_up_prior_chunks_on_overwrite() {
        let store = MockWinCredStore::new();
        let long_secret = "x".repeat(4200);
        write_secret(
            &store,
            "eostage:agent-model:gee-credentials",
            "gee-credentials",
            &long_secret,
        )
        .expect("write chunked");
        assert!(store.target_count() > 2);

        write_secret(
            &store,
            "eostage:agent-model:gee-credentials",
            "gee-credentials",
            "short-token",
        )
        .expect("overwrite with short secret");
        assert_eq!(store.keys(), vec!["eostage:agent-model:gee-credentials"]);
        let loaded = read_secret_owned(
            &store,
            "eostage:agent-model:gee-credentials",
            Some("gee-credentials"),
        )
        .expect("read short");
        assert_eq!(
            loaded,
            Some(("gee-credentials".to_owned(), "short-token".to_owned()))
        );
    }

    #[test]
    fn literal_manifest_prefix_string_roundtrips_without_confusion() {
        let store = MockWinCredStore::new();
        let tricky = format!("{MANIFEST_PREFIX}not-actually-a-manifest");
        write_secret(&store, "t", "u", &tricky).expect("write tricky");
        let loaded = read_secret_owned(&store, "t", Some("u")).expect("read tricky");
        assert_eq!(loaded, Some(("u".to_owned(), tricky)));
        assert!(delete_secret_owned(&store, "t", "u").expect("delete tricky"));
        assert_eq!(store.target_count(), 0);
    }

    #[test]
    fn gee_service_account_and_oauth_envelopes_exceeding_2560_utf16_bytes_roundtrip_and_delete() {
        let store = MockWinCredStore::new();
        let private_key_body = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSj".repeat(65);
        let sa_json = format!(
            "{{\"type\":\"service_account\",\"project_id\":\"ee-demo-proj\",\"client_email\":\"sa@ee-demo.iam.gserviceaccount.com\",\"private_key\":\"-----BEGIN PRIVATE KEY-----\\n{private_key_body}\\n-----END PRIVATE KEY-----\\n\"}}"
        );
        let envelope = serde_json::json!({
            "project_id": "ee-demo-proj",
            "auth_mode": "service_account",
            "client_email": "sa@ee-demo.iam.gserviceaccount.com",
            "secret": sa_json,
        })
        .to_string();
        assert!(envelope.len() * 2 > WIN32_CRED_MAX_BLOB_BYTES);

        write_secret(
            &store,
            "eostage:agent-model:gee-credentials",
            "gee-credentials",
            &envelope,
        )
        .expect("write large gee envelope under 2560B cap");

        let read_back = read_secret_owned(
            &store,
            "eostage:agent-model:gee-credentials",
            Some("gee-credentials"),
        )
        .expect("read large gee envelope");
        assert_eq!(
            read_back,
            Some(("gee-credentials".to_owned(), envelope.clone()))
        );

        let updated_envelope = format!("{envelope} ");
        write_secret(
            &store,
            "eostage:agent-model:gee-credentials",
            "gee-credentials",
            &updated_envelope,
        )
        .expect("update large gee envelope");
        assert_eq!(
            store.target_count(),
            1 + updated_envelope.len().div_ceil(2048),
            "old generation chunks must be removed"
        );

        assert!(delete_secret_owned(
            &store,
            "eostage:agent-model:gee-credentials",
            "gee-credentials"
        )
        .expect("delete large gee envelope"));
        assert_eq!(store.target_count(), 0, "zero orphan chunks after delete");
    }

    #[test]
    fn multibyte_utf8_crossing_chunk_boundary_roundtrips_intact() {
        let store = MockWinCredStore::new();
        let mut s = "a".repeat(2047);
        s.push_str("中文遥感数据测试🎉");
        s.push_str(&"b".repeat(2100));
        write_secret(&store, "utf8-target", "owner", &s).expect("write utf8 chunked");
        let loaded = read_secret_owned(&store, "utf8-target", Some("owner")).expect("read utf8");
        assert_eq!(loaded, Some(("owner".to_owned(), s)));
    }

    #[test]
    fn failed_chunked_write_rolls_back_and_preserves_previous_credential() {
        let store = MockWinCredStore::new();
        write_secret(&store, "svc", "u", "previous-secret").expect("seed prior");

        *store.writes.lock().unwrap() = 0;
        *store.fail_write_after.lock().unwrap() = Some(2);
        let big = "z".repeat(6000);
        let err = write_secret(&store, "svc", "u", &big).expect_err("must fail on 3rd chunk write");
        assert!(matches!(err, DesktopCredentialError::StoreUnavailable));
        *store.fail_write_after.lock().unwrap() = None;

        assert_eq!(store.keys(), vec!["svc"]);
        let loaded = read_secret_owned(&store, "svc", Some("u")).expect("prior still intact");
        assert_eq!(loaded, Some(("u".to_owned(), "previous-secret".to_owned())));
    }

    #[test]
    fn corrupt_missing_or_wrong_owner_chunk_fails_closed() {
        let store = MockWinCredStore::new();
        let big = "k".repeat(4500);
        write_secret(&store, "svc", "alice", &big).expect("seed");

        let manifest_raw = store.entries.lock().unwrap().get("svc").unwrap().1.clone();
        let decoded = super::chunked_secret::decode_password_bytes(&manifest_raw).unwrap();
        let manifest: Manifest =
            serde_json::from_str(decoded.strip_prefix(MANIFEST_PREFIX).unwrap()).unwrap();

        let chunk2 = chunk_target_name("svc", &manifest.generation, 2);
        // Flip owner on chunk 2 -> fail closed
        store.entries.lock().unwrap().get_mut(&chunk2).unwrap().0 = "bob".to_owned();
        assert!(matches!(
            read_secret_owned(&store, "svc", Some("alice")),
            Err(DesktopCredentialError::StoreUnavailable)
        ));

        // Restore owner, corrupt byte -> SHA-256 mismatch -> fail closed
        {
            let mut map = store.entries.lock().unwrap();
            let entry = map.get_mut(&chunk2).unwrap();
            entry.0 = "alice".to_owned();
            entry.1[0] ^= 0xff;
        }
        assert!(matches!(
            read_secret_owned(&store, "svc", Some("alice")),
            Err(DesktopCredentialError::StoreUnavailable)
        ));

        // Remove chunk 2 -> missing chunk -> fail closed
        store.entries.lock().unwrap().remove(&chunk2);
        assert!(matches!(
            read_secret_owned(&store, "svc", Some("alice")),
            Err(DesktopCredentialError::StoreUnavailable)
        ));
    }

    #[test]
    fn delete_undecodable_blob_and_skip_other_owner() {
        let store = MockWinCredStore::new();
        store.entries.lock().unwrap().insert(
            "svc".to_owned(),
            ("owner-x".to_owned(), 0xD800u16.to_le_bytes().to_vec()),
        );

        assert!(!delete_secret_owned(&store, "svc", "owner-y").expect("skip other"));
        assert_eq!(
            read_secret_owned(&store, "svc", Some("owner-y")).expect("ignore other"),
            None
        );
        assert!(delete_secret_owned(&store, "svc", "owner-x").expect("delete own"));
        assert_eq!(store.target_count(), 0);
    }

    #[test]
    fn collision_move_of_chunked_entry_preserves_both_and_cleans_orphans() {
        let store = MockWinCredStore::new();
        let alice_secret = "A".repeat(4500);
        let bob_secret = "B".repeat(4500);

        set_with_collision_move(&store, "shared-svc", "alice", &alice_secret).expect("seed alice");
        set_with_collision_move(&store, "shared-svc", "bob", &bob_secret)
            .expect("collide with bob");

        let read_alice =
            read_secret_owned(&store, "alice@shared-svc", Some("alice")).expect("read alice");
        let read_bob = read_secret_owned(&store, "shared-svc", Some("bob")).expect("read bob");
        assert_eq!(read_alice, Some(("alice".to_owned(), alice_secret)));
        assert_eq!(read_bob, Some(("bob".to_owned(), bob_secret.clone())));

        let alice_keys: Vec<_> = store
            .keys()
            .into_iter()
            .filter(|k| k.starts_with("shared-svc#"))
            .collect();
        assert_eq!(
            alice_keys.len(),
            bob_secret.len().div_ceil(2048),
            "only bob's chunks may remain under shared-svc#"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_encrypted_vault_lifecycle_migration_and_fail_closed_guards() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Mutex;
        use zeroize::Zeroizing;

        struct MockMasterKeyStore {
            key: Mutex<Option<Zeroizing<[u8; 32]>>>,
            locked: Mutex<bool>,
            reads: AtomicUsize,
            writes: AtomicUsize,
        }

        impl MockMasterKeyStore {
            fn new() -> Self {
                Self {
                    key: Mutex::new(None),
                    locked: Mutex::new(false),
                    reads: AtomicUsize::new(0),
                    writes: AtomicUsize::new(0),
                }
            }
        }

        impl super::platform_keyring::MasterKeyStore for MockMasterKeyStore {
            fn read_key(&self) -> Result<Option<Zeroizing<[u8; 32]>>, DesktopCredentialError> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                if *self.locked.lock().unwrap() {
                    return Err(DesktopCredentialError::StoreUnavailable);
                }
                Ok(self.key.lock().unwrap().clone())
            }

            fn write_key(&self, key: &[u8; 32]) -> Result<(), DesktopCredentialError> {
                self.writes.fetch_add(1, Ordering::SeqCst);
                if *self.locked.lock().unwrap() {
                    return Err(DesktopCredentialError::StoreUnavailable);
                }
                *self.key.lock().unwrap() = Some(Zeroizing::new(*key));
                Ok(())
            }
        }

        let dir = std::env::temp_dir().join(format!(
            "eostage-mac-vault-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let vault_path = dir.join("credentials-vault.v1.json");
        let store = MockMasterKeyStore::new();

        // 1. Fresh install: non-existent file never touches Keychain.
        let (empty_map, existed) =
            super::platform_keyring::load_vault_from_disk_with_store(&vault_path, &store, false)
                .expect("missing vault loads empty");
        assert!(!existed);
        assert!(empty_map.is_empty());
        assert_eq!(store.reads.load(Ordering::SeqCst), 0);
        assert_eq!(store.writes.load(Ordering::SeqCst), 0);

        // 2. Save secret -> creates 1 master key, writes AES-256-GCM v2 envelope, zero plaintext on disk.
        let mut vault = std::collections::BTreeMap::new();
        vault
            .entry("eostage:model-key".to_owned())
            .or_insert_with(std::collections::BTreeMap::new)
            .insert("openai".to_owned(), "sk-super-secret-model-key".to_owned());
        super::platform_keyring::save_vault_to_disk_with_store(&vault_path, &vault, &store, false)
            .expect("save encrypted vault");
        assert_eq!(store.writes.load(Ordering::SeqCst), 1);

        let disk_raw = std::fs::read_to_string(&vault_path).expect("read encrypted file");
        assert!(!disk_raw.contains("sk-super-secret-model-key"));
        assert!(!disk_raw.contains("eostage:model-key"));
        let disk_json: serde_json::Value = serde_json::from_str(&disk_raw).unwrap();
        assert_eq!(disk_json["v"], 2);
        assert_eq!(disk_json["alg"], "aes-256-gcm");

        // 3. Simulated process restart (use_process_cache = false) -> decrypts cleanly.
        let (reloaded, existed) =
            super::platform_keyring::load_vault_from_disk_with_store(&vault_path, &store, false)
                .expect("reload encrypted vault");
        assert!(existed);
        assert_eq!(
            reloaded["eostage:model-key"]["openai"],
            "sk-super-secret-model-key"
        );

        // 4. Missing master key with existing encrypted file -> fails closed, does NOT regenerate or overwrite.
        let saved_key = store.key.lock().unwrap().take();
        let before_bytes = std::fs::read(&vault_path).unwrap();
        let missing_err =
            super::platform_keyring::load_vault_from_disk_with_store(&vault_path, &store, false);
        assert!(matches!(
            missing_err,
            Err(DesktopCredentialError::StoreUnavailable)
        ));
        assert_eq!(store.writes.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(&vault_path).unwrap(), before_bytes);

        // 5. Locked Keychain -> fails closed.
        *store.key.lock().unwrap() = saved_key;
        *store.locked.lock().unwrap() = true;
        assert!(matches!(
            super::platform_keyring::load_vault_from_disk_with_store(&vault_path, &store, false),
            Err(DesktopCredentialError::StoreUnavailable)
        ));
        *store.locked.lock().unwrap() = false;

        // 6. Tampered ciphertext -> fails closed.
        let mut tampered = disk_json.clone();
        tampered["ciphertext"] = serde_json::Value::String("AAAA".to_owned());
        std::fs::write(&vault_path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        assert!(matches!(
            super::platform_keyring::load_vault_from_disk_with_store(&vault_path, &store, false),
            Err(DesktopCredentialError::StoreUnavailable)
        ));

        // 7. Legacy plaintext migration: locked Keychain fails closed WITHOUT touching legacy file.
        let legacy_path = dir.join("legacy-vault.json");
        let legacy_raw = r#"{"eostage:asf":{"user":"legacy-pass"}}"#;
        std::fs::write(&legacy_path, legacy_raw).unwrap();
        *store.locked.lock().unwrap() = true;
        assert!(matches!(
            super::platform_keyring::load_vault_from_disk_with_store(&legacy_path, &store, false),
            Err(DesktopCredentialError::StoreUnavailable)
        ));
        assert_eq!(std::fs::read_to_string(&legacy_path).unwrap(), legacy_raw);

        // Unlock -> transparently migrates legacy plaintext to encrypted v2 envelope.
        *store.locked.lock().unwrap() = false;
        let (migrated, existed) =
            super::platform_keyring::load_vault_from_disk_with_store(&legacy_path, &store, false)
                .expect("migrate legacy vault");
        assert!(existed);
        assert_eq!(migrated["eostage:asf"]["user"], "legacy-pass");
        let migrated_disk = std::fs::read_to_string(&legacy_path).unwrap();
        assert!(!migrated_disk.contains("legacy-pass"));
        assert!(migrated_disk.contains("\"alg\": \"aes-256-gcm\""));

        // 8. Cache rollback / no cache poisoning on failed set() or delete() and fresh delete().
        let cache_vault_path = dir.join("cache-guard.v1.json");
        let cache_store = MockMasterKeyStore::new();
        let cache: super::platform_keyring::VaultCache = Mutex::new(None);

        // 8a. Fresh delete() when file does not exist neither touches Keychain nor populates cache.
        let fresh_del = super::platform_keyring::with_vault_using_store(
            &cache,
            &cache_vault_path,
            &cache_store,
            false,
            true,
            |v| Ok((v.remove("svc").is_some(), false)),
        )
        .expect("fresh delete ok");
        assert!(!fresh_del);
        assert_eq!(cache_store.reads.load(Ordering::SeqCst), 0);
        assert!(cache.lock().unwrap().is_none());

        // 8b. Failed set() when Keychain is locked must NOT poison in-memory cache.
        *cache_store.locked.lock().unwrap() = true;
        let failed_set = super::platform_keyring::with_vault_using_store(
            &cache,
            &cache_vault_path,
            &cache_store,
            false,
            true,
            |v| {
                v.entry("svc".to_owned())
                    .or_default()
                    .insert("user".to_owned(), "unsaved-secret".to_owned());
                Ok(((), true))
            },
        );
        assert!(matches!(
            failed_set,
            Err(DesktopCredentialError::StoreUnavailable)
        ));
        assert!(cache.lock().unwrap().is_none());
        let read_after_failed_set = super::platform_keyring::with_vault_using_store(
            &cache,
            &cache_vault_path,
            &cache_store,
            false,
            false,
            |v| Ok((v.get("svc").and_then(|m| m.get("user")).cloned(), false)),
        )
        .expect("read missing file after failed set");
        assert_eq!(read_after_failed_set, None);

        // 8c. Unlock -> successful set(), then lock -> failed delete() must keep cached & disk value intact.
        *cache_store.locked.lock().unwrap() = false;
        super::platform_keyring::with_vault_using_store(
            &cache,
            &cache_vault_path,
            &cache_store,
            false,
            true,
            |v| {
                v.entry("svc".to_owned())
                    .or_default()
                    .insert("user".to_owned(), "persisted-secret".to_owned());
                Ok(((), true))
            },
        )
        .expect("persisted set ok");

        *cache_store.locked.lock().unwrap() = true;
        let failed_del = super::platform_keyring::with_vault_using_store(
            &cache,
            &cache_vault_path,
            &cache_store,
            false,
            true,
            |v| {
                let removed = v
                    .get_mut("svc")
                    .and_then(|m| m.remove("user"))
                    .is_some();
                Ok((removed, removed))
            },
        );
        assert!(matches!(
            failed_del,
            Err(DesktopCredentialError::StoreUnavailable)
        ));
        let cached_val = super::platform_keyring::with_vault_using_store(
            &cache,
            &cache_vault_path,
            &cache_store,
            false,
            false,
            |v| Ok((v.get("svc").and_then(|m| m.get("user")).cloned(), false)),
        )
        .expect("cached value remains intact after failed delete");
        assert_eq!(cached_val.as_deref(), Some("persisted-secret"));

        // 9. Concurrent cross-process initialization & legacy migration serialization.
        struct SlowMasterKeyStore {
            key: Mutex<Option<Zeroizing<[u8; 32]>>>,
            writes: AtomicUsize,
        }
        impl super::platform_keyring::MasterKeyStore for SlowMasterKeyStore {
            fn read_key(&self) -> Result<Option<Zeroizing<[u8; 32]>>, DesktopCredentialError> {
                Ok(self.key.lock().unwrap().clone())
            }
            fn write_key(&self, key: &[u8; 32]) -> Result<(), DesktopCredentialError> {
                std::thread::sleep(std::time::Duration::from_millis(25));
                self.writes.fetch_add(1, Ordering::SeqCst);
                *self.key.lock().unwrap() = Some(Zeroizing::new(*key));
                Ok(())
            }
        }

        // 9a. Two independent processes racing first-time save on fresh vault.
        let race_vault_path = dir.join("race-init.v1.json");
        let race_store = std::sync::Arc::new(SlowMasterKeyStore {
            key: Mutex::new(None),
            writes: AtomicUsize::new(0),
        });
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for (user, secret) in [("user_a", "secret-a"), ("user_b", "secret-b")] {
            let p = race_vault_path.clone();
            let st = std::sync::Arc::clone(&race_store);
            let b = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let proc_cache: super::platform_keyring::VaultCache = Mutex::new(None);
                b.wait();
                super::platform_keyring::with_vault_using_store(
                    &proc_cache,
                    &p,
                    st.as_ref(),
                    false,
                    true,
                    |v| {
                        v.entry("svc".to_owned())
                            .or_default()
                            .insert(user.to_owned(), secret.to_owned());
                        Ok(((), true))
                    },
                )
            }));
        }
        for h in handles {
            h.join().unwrap().expect("concurrent first save succeeds");
        }
        assert_eq!(race_store.writes.load(Ordering::SeqCst), 1);
        let (restarted_map, _) = super::platform_keyring::load_vault_from_disk_with_store(
            &race_vault_path,
            race_store.as_ref(),
            false,
        )
        .expect("restarted read succeeds after concurrent init");
        assert_eq!(restarted_map["svc"]["user_a"], "secret-a");
        assert_eq!(restarted_map["svc"]["user_b"], "secret-b");

        // 9b. Two independent processes racing concurrent legacy plaintext migration.
        let race_legacy_path = dir.join("race-legacy.v1.json");
        std::fs::write(&race_legacy_path, r#"{"svc":{"legacy":"old-val"}}"#).unwrap();
        let legacy_store = std::sync::Arc::new(SlowMasterKeyStore {
            key: Mutex::new(None),
            writes: AtomicUsize::new(0),
        });
        let legacy_barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut legacy_handles = Vec::new();
        for _ in 0..2 {
            let p = race_legacy_path.clone();
            let st = std::sync::Arc::clone(&legacy_store);
            let b = std::sync::Arc::clone(&legacy_barrier);
            legacy_handles.push(std::thread::spawn(move || {
                let proc_cache: super::platform_keyring::VaultCache = Mutex::new(None);
                b.wait();
                super::platform_keyring::with_vault_using_store(
                    &proc_cache,
                    &p,
                    st.as_ref(),
                    false,
                    false,
                    |v| Ok((v.get("svc").and_then(|m| m.get("legacy")).cloned(), false)),
                )
            }));
        }
        for h in legacy_handles {
            assert_eq!(
                h.join().unwrap().expect("concurrent migration read").as_deref(),
                Some("old-val")
            );
        }
        assert_eq!(legacy_store.writes.load(Ordering::SeqCst), 1);
        let (migrated_restarted, _) = super::platform_keyring::load_vault_from_disk_with_store(
            &race_legacy_path,
            legacy_store.as_ref(),
            false,
        )
        .expect("restarted read succeeds after concurrent legacy migration");
        assert_eq!(migrated_restarted["svc"]["legacy"], "old-val");

        // 10. Migrated encrypted vault with legacy `eostage:earthengine` remains readable & scrubbable via local vault helpers.
        let ee_vault_path = dir.join("ee-legacy.v1.json");
        let ee_raw = serde_json::json!({
            "eostage:earthengine": {
                "project_id": "ee-legacy-proj",
                "secret": "ya29.legacy-ee-oauth"
            },
            "eostage:cdse": {
                "username": "keep-cdse@example.com"
            }
        });
        std::fs::write(&ee_vault_path, serde_json::to_vec_pretty(&ee_raw).unwrap()).unwrap();
        let ee_sidecar = super::platform_keyring::SidecarFileMasterKeyStore::for_vault(&ee_vault_path);
        let (migrated_ee, _) = super::platform_keyring::load_vault_from_disk_with_store(
            &ee_vault_path,
            &ee_sidecar,
            false,
        )
        .expect("legacy ee vault migrates to encrypted v2");
        assert_eq!(
            migrated_ee["eostage:earthengine"]["secret"],
            "ya29.legacy-ee-oauth"
        );
        let on_disk_ee = std::fs::read_to_string(&ee_vault_path).unwrap();
        assert!(!on_disk_ee.contains("ya29.legacy-ee-oauth"));
        assert!(on_disk_ee.contains("\"alg\": \"aes-256-gcm\""));

        let decrypted_doc = super::read_local_vault_map_readonly(&ee_vault_path)
            .expect("read_local_vault_map_readonly decrypts v2 vault")
            .expect("vault present");
        assert_eq!(
            decrypted_doc["eostage:earthengine"]["secret"],
            "ya29.legacy-ee-oauth"
        );
        assert_eq!(
            decrypted_doc["eostage:earthengine"]["project_id"],
            "ee-legacy-proj"
        );

        let scrubbed = super::scrub_local_vault_services(&ee_vault_path, |k| {
            k == "eostage:earthengine"
                || k == "eostage:agent-model:gee-credentials"
                || k.starts_with("eostage:agent-model:gee-proj-")
        })
        .expect("scrub_local_vault_services succeeds on encrypted v2 vault");
        assert!(scrubbed);
        let after_scrub = super::read_local_vault_map_readonly(&ee_vault_path)
            .expect("re-read encrypted vault after scrub")
            .expect("vault still present");
        assert!(after_scrub.get("eostage:earthengine").is_none());
        assert_eq!(
            after_scrub["eostage:cdse"]["username"],
            "keep-cdse@example.com"
        );

        // 11. Distinct vault paths derive distinct Keychain master-key identities.
        let kc_a = super::platform_keyring::MacKeychainMasterKeyStore::for_vault(
            &dir.join("home_a/credentials-vault.v1.json"),
        );
        let kc_b = super::platform_keyring::MacKeychainMasterKeyStore::for_vault(
            &dir.join("home_b/credentials-vault.v1.json"),
        );
        let kc_a2 = super::platform_keyring::MacKeychainMasterKeyStore::for_vault(
            &dir.join("home_a/credentials-vault.v1.json"),
        );
        assert_ne!(kc_a.account, kc_b.account);
        assert_eq!(kc_a.account, kc_a2.account);

        // 12. Symlinked home ancestor with nonexistent parent derives identical Keychain account before & after parent creation.
        let real_home = dir.join("real_home");
        let link_home = dir.join("link_home");
        std::fs::create_dir_all(&real_home).unwrap();
        std::os::unix::fs::symlink(&real_home, &link_home).unwrap();
        let sym_vault_path = link_home.join("Library/Application Support/EOStage/v3/credentials-vault.v1.json");
        let kc_before_mkdir = super::platform_keyring::MacKeychainMasterKeyStore::for_vault(&sym_vault_path);
        std::fs::create_dir_all(sym_vault_path.parent().unwrap()).unwrap();
        let kc_after_mkdir = super::platform_keyring::MacKeychainMasterKeyStore::for_vault(&sym_vault_path);
        let kc_real_path = super::platform_keyring::MacKeychainMasterKeyStore::for_vault(
            &real_home.join("Library/Application Support/EOStage/v3/credentials-vault.v1.json"),
        );
        assert_eq!(kc_before_mkdir.account, kc_after_mkdir.account);
        assert_eq!(kc_before_mkdir.account, kc_real_path.account);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
