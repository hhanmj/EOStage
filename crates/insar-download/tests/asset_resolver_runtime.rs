use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use insar_download::{
    AuthRefreshBudget, Clock, CredentialMaterial, CredentialProvider, CredentialProviderError,
    ProxyPolicy, RemoteValidator, ResolveContext, ResolveError, ResolvedAsset, ResolvedChecksum,
    ResolvedMethod, TimeoutPolicy,
};

struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

struct FixtureCredentials;

impl CredentialProvider for FixtureCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        match profile {
            "fixture" => CredentialMaterial::bearer("TOP_SECRET_TOKEN"),
            _ => Err(CredentialProviderError::Missing),
        }
    }
}

#[test]
fn resolved_asset_debug_redacts_query_headers_and_cookies() {
    let expires_at = SystemTime::UNIX_EPOCH + Duration::from_secs(200);
    let mut asset = ResolvedAsset::new(
        "https://example.org/file.zip?Signature=TOP_SECRET_QUERY",
        ResolvedMethod::Get,
    )
    .unwrap()
    .with_expiry(expires_at)
    .with_expected_size(Some(4096))
    .with_checksum(Some(ResolvedChecksum::new("sha256", "abc123").unwrap()))
    .with_suggested_filename(Some("file.zip".to_string()))
    .with_remote_validator(RemoteValidator {
        etag: Some("fixture-etag".to_string()),
        last_modified: Some("Wed, 01 Jan 2025 00:00:00 GMT".to_string()),
        content_length: Some(4096),
    });
    asset
        .insert_header("Authorization", "Bearer TOP_SECRET_HEADER")
        .unwrap();
    asset
        .insert_cookie("urs_user_already_logged", "TOP_SECRET_COOKIE")
        .unwrap();

    let debug = format!("{asset:?}");
    assert!(debug.contains("https://example.org/file.zip"));
    assert!(debug.contains("Authorization"));
    assert!(debug.contains("urs_user_already_logged"));
    for secret in [
        "TOP_SECRET_QUERY",
        "TOP_SECRET_HEADER",
        "TOP_SECRET_COOKIE",
        "Signature=",
    ] {
        assert!(!debug.contains(secret), "debug leaked {secret}");
    }
    assert_eq!(asset.expected_size(), Some(4096));
    assert_eq!(asset.suggested_filename(), Some("file.zip"));
}

#[test]
fn injected_clock_controls_expiry_without_wall_clock_reads() {
    let expiry = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let asset = ResolvedAsset::new("https://example.org/file.zip", ResolvedMethod::Get)
        .unwrap()
        .with_expiry(expiry);

    asset
        .ensure_fresh(&FixedClock(
            SystemTime::UNIX_EPOCH + Duration::from_secs(99),
        ))
        .unwrap();
    let error = asset
        .ensure_fresh(&FixedClock(
            SystemTime::UNIX_EPOCH + Duration::from_secs(100),
        ))
        .unwrap_err();
    assert_eq!(error.code(), "ASSET_RESOLUTION_EXPIRED");
}

#[test]
fn resolve_context_is_cancellable_bounded_and_debug_safe() {
    let credentials = FixtureCredentials;
    let clock = FixedClock(SystemTime::UNIX_EPOCH);
    let cancelled = || true;
    let context = ResolveContext::new(&credentials, &cancelled, &clock)
        .with_proxy_policy(ProxyPolicy::System)
        .with_timeout_policy(
            TimeoutPolicy::new(Duration::from_secs(10), Duration::from_secs(60)).unwrap(),
        )
        .with_auth_refresh_budget(AuthRefreshBudget::new(1));

    assert_eq!(
        context.check_cancelled().unwrap_err().code(),
        "ASSET_RESOLUTION_CANCELLED"
    );
    let material = context.credential_provider().resolve("fixture").unwrap();
    assert!(!format!("{context:?}").contains("TOP_SECRET_TOKEN"));
    assert!(!format!("{material:?}").contains("TOP_SECRET_TOKEN"));

    let mut budget = AuthRefreshBudget::new(1);
    budget.try_consume().unwrap();
    assert_eq!(
        budget.try_consume().unwrap_err().code(),
        "ASSET_REFRESH_BUDGET_EXHAUSTED"
    );
    assert_eq!(budget.remaining(), 0);
}

#[test]
fn invalid_timeout_and_secret_material_return_typed_errors() {
    assert_eq!(
        TimeoutPolicy::new(Duration::ZERO, Duration::from_secs(1))
            .unwrap_err()
            .code(),
        "ASSET_ACCESS_DESCRIPTOR_INVALID"
    );
    assert!(CredentialMaterial::bearer("  ").is_err());
    assert!(ResolvedChecksum::new("", "abc").is_err());
    let error = ResolveError::not_configured();
    assert_eq!(error.code(), "ASSET_RESOLVER_NOT_CONFIGURED");
}

#[test]
fn resolved_asset_runtime_type_is_absent_from_persistence_contracts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "contracts/v3/provider-contract.schema.json",
        "ui/src/lib/generated/v3-contracts.ts",
        "crates/insar-store/src/repository.rs",
    ] {
        let text = fs::read_to_string(root.join(relative)).unwrap();
        assert!(
            !text.contains("ResolvedAsset"),
            "{relative} persists runtime access"
        );
    }
    let source =
        fs::read_to_string(root.join("crates/insar-download/src/asset_resolver/types.rs")).unwrap();
    assert!(!source.contains("Serialize"));
    assert!(!source.contains("Deserialize"));
}
