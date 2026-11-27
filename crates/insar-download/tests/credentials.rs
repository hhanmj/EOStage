use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_download::{
    resolve_earthdata_burst_credential_with, resolve_earthdata_credential_profile_with,
    resolve_earthdata_credential_with, CredentialProvider, CredentialProviderError,
    EarthdataCredentialSource, EnvironmentEarthdataCredentialProvider, TransferAuth,
    EARTHDATA_BURST_CREDENTIAL_PROFILE, EARTHDATA_CREDENTIAL_PROFILE,
};

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-download-credentials-{}-{nonce}-{seq}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn resolve(
    source: EarthdataCredentialSource,
    environment: BTreeMap<&str, &str>,
    paths: &[PathBuf],
) -> Result<TransferAuth, insar_download::EarthdataCredentialError> {
    resolve_earthdata_credential_with(
        source,
        |key| environment.get(key).map(|value| (*value).to_string()),
        paths,
    )
}

#[test]
fn auto_prefers_token_then_injected_keyring_login_without_debug_leaks() {
    let auth = resolve(
        EarthdataCredentialSource::Auto,
        BTreeMap::from([
            ("EARTHDATA_TOKEN", "TOKEN_SECRET"),
            ("INSAR_EARTHDATA_USERNAME", "user-secret"),
            ("INSAR_EARTHDATA_PASSWORD", "password-secret"),
        ]),
        &[],
    )
    .unwrap();
    let debug = format!("{auth:?}");
    assert!(debug.contains("bearer"));
    assert!(!debug.contains("TOKEN_SECRET"));
    assert!(!debug.contains("password-secret"));

    let login = resolve(
        EarthdataCredentialSource::Auto,
        BTreeMap::from([
            ("INSAR_EARTHDATA_USERNAME", "user-secret"),
            ("INSAR_EARTHDATA_PASSWORD", "password-secret"),
        ]),
        &[],
    )
    .unwrap();
    let debug = format!("{login:?}");
    assert!(debug.contains("basic"));
    assert!(!debug.contains("user-secret"));
}

#[test]
fn netrc_supports_comments_quotes_and_windows_alternate_file_name() {
    let temp = TempDirectory::new();
    let path = temp.path().join("_netrc");
    fs::write(
        &path,
        "# Earthdata\nmachine urs.earthdata.nasa.gov login \"earth user\" password 'secret pass'\n",
    )
    .unwrap();

    let auth = resolve(EarthdataCredentialSource::Netrc, BTreeMap::new(), &[path]).unwrap();
    let debug = format!("{auth:?}");
    assert!(debug.contains("basic"));
    assert!(!debug.contains("earth user"));
    assert!(!debug.contains("secret pass"));
}

#[test]
fn netrc_supports_utf8_bom_crlf_case_insensitive_keywords_and_fallback_paths() {
    let temp = TempDirectory::new();
    let unrelated = temp.path().join(".netrc");
    fs::write(
        &unrelated,
        "machine urs.earthdata.nasa.gov.evil.com login spoof password SPOOF_SECRET\r\n",
    )
    .unwrap();
    let valid_fallback = temp.path().join("_netrc");
    fs::write(
        &valid_fallback,
        "\u{feff}MACHINE https://URS.EARTHDATA.NASA.GOV/\r\nLOGIN CaseSensitiveUser\r\nPASSWORD \"Case!Pass#1\"\r\n",
    )
    .unwrap();

    let auth = resolve(
        EarthdataCredentialSource::Netrc,
        BTreeMap::new(),
        &[unrelated.clone(), valid_fallback],
    )
    .unwrap();
    let debug = format!("{auth:?}");
    assert!(debug.contains("basic"));
    assert!(!debug.contains("CaseSensitiveUser"));
    assert!(!debug.contains("Case!Pass#1"));

    // Lookalike host alone must be rejected.
    let err = resolve(
        EarthdataCredentialSource::Netrc,
        BTreeMap::new(),
        &[unrelated],
    )
    .unwrap_err();
    assert_eq!(err.code(), "DL004");
}

#[test]
fn missing_and_malformed_credentials_return_only_bounded_dl004_errors() {
    let missing = resolve(EarthdataCredentialSource::Auto, BTreeMap::new(), &[]).unwrap_err();
    assert_eq!(missing.code(), "DL004");
    assert!(missing.reason_zh().contains("Earthdata"));

    let temp = TempDirectory::new();
    let path = temp.path().join(".netrc");
    fs::write(
        &path,
        "machine urs.earthdata.nasa.gov login user password 'SECRET",
    )
    .unwrap();
    let malformed =
        resolve(EarthdataCredentialSource::Netrc, BTreeMap::new(), &[path]).unwrap_err();
    assert_eq!(malformed.code(), "DL004");
    assert!(!format!("{malformed}").contains("SECRET"));
}

#[test]
fn burst_requires_account_or_netrc_and_rejects_token_only_with_typed_code() {
    let token_only = resolve_earthdata_burst_credential_with(
        |key| (key == "EARTHDATA_TOKEN").then(|| "TOKEN_SECRET".to_string()),
        &[],
    )
    .unwrap_err();
    assert_eq!(token_only.code(), "ASF_BURST_COOKIE_REQUIRED");
    assert!(!format!("{token_only}").contains("TOKEN_SECRET"));

    let login = resolve_earthdata_burst_credential_with(
        |key| match key {
            "INSAR_EARTHDATA_USERNAME" => Some("earth-user".to_string()),
            "INSAR_EARTHDATA_PASSWORD" => Some("SECRET_PASSWORD".to_string()),
            _ => None,
        },
        &[],
    )
    .unwrap();
    let debug = format!("{login:?}");
    assert!(debug.contains("basic"));
    assert!(!debug.contains("SECRET_PASSWORD"));
}

#[test]
fn production_resolver_credential_provider_rejects_unknown_profiles_without_discovery() {
    let provider = EnvironmentEarthdataCredentialProvider::new();
    assert_eq!(
        provider.resolve("unknown-profile").unwrap_err(),
        CredentialProviderError::Missing
    );
}

#[test]
fn dedicated_burst_profile_is_basic_only_and_token_only_stays_typed() {
    let login = resolve_earthdata_credential_profile_with(
        EARTHDATA_BURST_CREDENTIAL_PROFILE,
        |key| match key {
            "INSAR_EARTHDATA_USERNAME" => Some("burst-user".to_owned()),
            "INSAR_EARTHDATA_PASSWORD" => Some("BURST_PASSWORD".to_owned()),
            _ => None,
        },
        &[],
    )
    .unwrap();
    let debug = format!("{login:?}");
    assert!(debug.contains("basic"));
    assert!(!debug.contains("BURST_PASSWORD"));

    let token_only = resolve_earthdata_credential_profile_with(
        EARTHDATA_BURST_CREDENTIAL_PROFILE,
        |key| (key == "EARTHDATA_TOKEN").then(|| "TOKEN_ONLY_SECRET".to_owned()),
        &[],
    )
    .unwrap_err();
    assert_eq!(token_only.code(), "ASF_BURST_COOKIE_REQUIRED");
    assert!(!format!("{token_only:?}").contains("TOKEN_ONLY_SECRET"));

    let standard = resolve_earthdata_credential_profile_with(
        EARTHDATA_CREDENTIAL_PROFILE,
        |key| (key == "EARTHDATA_TOKEN").then(|| "STANDARD_TOKEN".to_owned()),
        &[],
    )
    .unwrap();
    assert!(format!("{standard:?}").contains("bearer"));
}
