use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetResolverParameters, StableAssetReference,
};
use insar_download::{
    AssetResolver, Clock, CredentialMaterial, CredentialProvider, CredentialProviderError,
    EarthdataAssetResolver, LocalFileAssetResolver, PublicHttpsAssetResolver, ResolveContext,
};
use reqwest::Url;

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

struct NoCredentials;

impl CredentialProvider for NoCredentials {
    fn resolve(&self, _profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        Err(CredentialProviderError::Missing)
    }
}

fn reference() -> StableAssetReference {
    StableAssetReference {
        provider_id: "fixture".to_string(),
        collection_id: "fixture-collection".to_string(),
        provider_product_id: "fixture-product".to_string(),
        provider_product_key: "fixture::product".to_string(),
        canonical_product_key: None,
        asset_key: "primary".to_string(),
        stable_asset_key: "fixture::product::primary".to_string(),
    }
}

fn descriptor(
    resolver_id: &str,
    scheme: AssetAccessScheme,
    stable_href: &str,
    credential_profile: Option<&str>,
) -> AssetAccessDescriptor {
    AssetAccessDescriptor {
        resolver_id: resolver_id.to_string(),
        scheme,
        stable_href: Some(stable_href.to_string()),
        requires_credentials: credential_profile.is_some(),
        credential_profile: credential_profile.map(str::to_string),
        resolver_parameters: AssetResolverParameters {},
    }
}

fn resolve_with_no_credentials(
    resolver: &dyn AssetResolver,
    descriptor: &AssetAccessDescriptor,
) -> Result<insar_download::ResolvedAsset, insar_download::ResolveError> {
    let credentials = NoCredentials;
    let cancelled = || false;
    let clock = FixedClock;
    resolver.resolve(
        &reference(),
        descriptor,
        &ResolveContext::new(&credentials, &cancelled, &clock),
    )
}

#[test]
fn public_https_rejects_insecure_signed_and_non_public_locations() {
    let resolver = PublicHttpsAssetResolver::new();
    for href in [
        "http://example.org/product.bin",
        "https://user:secret@example.org/product.bin",
        "https://example.org/product.bin?Signature=SECRET",
        "https://example.org/product.bin#SECRET",
        "https://localhost/product.bin",
        "https://127.0.0.1/product.bin",
        "https://10.0.0.1/product.bin",
        "https://169.254.1.1/product.bin",
        "https://[::1]/product.bin",
        "https://service.local/product.bin",
    ] {
        let error = resolve_with_no_credentials(
            &resolver,
            &descriptor("public_https", AssetAccessScheme::PublicHttps, href, None),
        )
        .unwrap_err();
        assert_eq!(
            error.code(),
            "ASSET_ACCESS_NOT_DURABLE",
            "unexpected result for {href}"
        );
    }
}

#[test]
fn public_https_accepts_only_unauthenticated_public_https() {
    let resolver = PublicHttpsAssetResolver::new();
    let resolved = resolve_with_no_credentials(
        &resolver,
        &descriptor(
            "public_https",
            AssetAccessScheme::PublicHttps,
            "https://example.org/products/native.tif",
            None,
        ),
    )
    .unwrap();
    assert_eq!(
        resolved.effective_url().as_str(),
        "https://example.org/products/native.tif"
    );

    let authenticated = descriptor(
        "public_https",
        AssetAccessScheme::PublicHttps,
        "https://example.org/products/native.tif",
        Some("forbidden"),
    );
    assert_eq!(
        resolve_with_no_credentials(&resolver, &authenticated)
            .unwrap_err()
            .code(),
        "ASSET_ACCESS_DESCRIPTOR_INVALID"
    );
}

#[test]
fn local_file_requires_an_absolute_existing_regular_file_without_writing() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("insar-resolver-{suffix}"));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("fixture.bin");
    fs::write(&path, b"fixture-bytes").unwrap();
    let href = Url::from_file_path(&path).unwrap().to_string();

    let resolver = LocalFileAssetResolver::new();
    let resolved = resolve_with_no_credentials(
        &resolver,
        &descriptor("local_file", AssetAccessScheme::LocalFile, &href, None),
    )
    .unwrap();
    assert_eq!(resolved.expected_size(), Some(13));
    assert_eq!(resolved.suggested_filename(), Some("fixture.bin"));
    assert_eq!(fs::read(&path).unwrap(), b"fixture-bytes");

    let missing_href = Url::from_file_path(root.join("missing.bin"))
        .unwrap()
        .to_string();
    assert_eq!(
        resolve_with_no_credentials(
            &resolver,
            &descriptor(
                "local_file",
                AssetAccessScheme::LocalFile,
                &missing_href,
                None,
            ),
        )
        .unwrap_err()
        .code(),
        "ASSET_REFERENCE_INVALID"
    );

    fs::remove_dir_all(&root).unwrap();
}

#[derive(Default)]
struct RecordingCredentials {
    calls: Mutex<Vec<String>>,
}

impl CredentialProvider for RecordingCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        self.calls.lock().unwrap().push(profile.to_string());
        match profile {
            "task-a" => CredentialMaterial::bearer("TOKEN_FOR_TASK_A"),
            "task-b" => CredentialMaterial::bearer("TOKEN_FOR_TASK_B"),
            "rejected" => Err(CredentialProviderError::Rejected),
            "timeout" => Err(CredentialProviderError::Timeout),
            _ => Err(CredentialProviderError::Missing),
        }
    }
}

fn resolve_earthdata(
    resolver: &EarthdataAssetResolver,
    credentials: &dyn CredentialProvider,
    profile: &str,
) -> Result<insar_download::ResolvedAsset, insar_download::ResolveError> {
    let cancelled = || false;
    let clock = FixedClock;
    resolver.resolve(
        &reference(),
        &descriptor(
            "earthdata_https",
            AssetAccessScheme::EarthdataHttps,
            "https://datapool.asf.alaska.edu/SLC/fixture.zip",
            Some(profile),
        ),
        &ResolveContext::new(credentials, &cancelled, &clock),
    )
}

#[test]
fn earthdata_requires_approved_host_and_maps_credential_failures() {
    let resolver = EarthdataAssetResolver::official();
    let credentials = RecordingCredentials::default();

    assert_eq!(
        resolve_earthdata(&resolver, &credentials, "missing")
            .unwrap_err()
            .code(),
        "ASSET_CREDENTIALS_MISSING"
    );
    let rejected = resolve_earthdata(&resolver, &credentials, "rejected").unwrap_err();
    assert_eq!(rejected.code(), "ASSET_AUTH_FAILED");
    assert!(!format!("{rejected:?}").contains("rejected"));
    assert_eq!(
        resolve_earthdata(&resolver, &credentials, "timeout")
            .unwrap_err()
            .code(),
        "ASSET_RESOLUTION_TIMEOUT"
    );

    let unapproved = descriptor(
        "earthdata_https",
        AssetAccessScheme::EarthdataHttps,
        "https://example.org/not-earthdata.bin",
        Some("task-a"),
    );
    let cancelled = || false;
    let clock = FixedClock;
    assert_eq!(
        resolver
            .resolve(
                &reference(),
                &unapproved,
                &ResolveContext::new(&credentials, &cancelled, &clock),
            )
            .unwrap_err()
            .code(),
        "ASSET_ACCESS_NOT_DURABLE"
    );
}

#[test]
fn earthdata_host_policy_requires_https_443_and_complete_dns_labels() {
    let resolver = EarthdataAssetResolver::official();
    let credentials = RecordingCredentials::default();
    let cancelled = || false;
    let clock = FixedClock;

    for href in [
        "https://earthdatacloud.nasa.gov/product.bin",
        "https://bucket.earthdatacloud.nasa.gov.:443/product.bin",
    ] {
        let resolved = resolver
            .resolve(
                &reference(),
                &descriptor(
                    "earthdata_https",
                    AssetAccessScheme::EarthdataHttps,
                    href,
                    Some("task-a"),
                ),
                &ResolveContext::new(&credentials, &cancelled, &clock),
            )
            .unwrap_or_else(|error| panic!("expected accepted URL {href}: {error:?}"));
        assert_eq!(resolved.effective_url().scheme(), "https");
    }

    for href in [
        "https://fakeearthdatacloud.nasa.gov/product.bin",
        "https://earthdatacloud.nasa.gov.evil.example/product.bin",
        "https://user@earthdatacloud.nasa.gov/product.bin",
        "https://earthdatacloud.nasa.gov/product.bin?Signature=SECRET",
        "https://earthdatacloud.nasa.gov/product.bin#fragment",
        "https://earthdatacloud.nasa.gov:444/product.bin",
    ] {
        let error = resolver
            .resolve(
                &reference(),
                &descriptor(
                    "earthdata_https",
                    AssetAccessScheme::EarthdataHttps,
                    href,
                    Some("task-a"),
                ),
                &ResolveContext::new(&credentials, &cancelled, &clock),
            )
            .unwrap_err();
        assert_eq!(error.code(), "ASSET_ACCESS_NOT_DURABLE", "{href}");
    }
}

#[test]
fn concurrent_earthdata_resolution_keeps_profiles_and_secrets_isolated() {
    let resolver = Arc::new(EarthdataAssetResolver::official());
    let credentials = Arc::new(RecordingCredentials::default());
    let mut workers = Vec::new();
    for profile in ["task-a", "task-b"] {
        let resolver = Arc::clone(&resolver);
        let credentials = Arc::clone(&credentials);
        workers.push(thread::spawn(move || {
            let asset = resolve_earthdata(&resolver, credentials.as_ref(), profile).unwrap();
            let debug = format!("{asset:?}");
            assert!(debug.contains("Authorization"));
            assert!(!debug.contains("TOKEN_FOR_TASK_A"));
            assert!(!debug.contains("TOKEN_FOR_TASK_B"));
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }

    let mut calls = credentials.calls.lock().unwrap().clone();
    calls.sort();
    assert_eq!(calls, vec!["task-a", "task-b"]);
}

#[test]
fn basic_credentials_remain_typed_in_the_resolver_boundary() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let context_source =
        fs::read_to_string(root.join("crates/insar-download/src/asset_resolver/context.rs"))
            .unwrap();
    assert!(!context_source.contains("format!(\"Basic {}:{}\""));

    let credential = CredentialMaterial::basic("FIXTURE_USER", "FIXTURE_PASSWORD").unwrap();
    assert!(!format!("{credential:?}").contains("FIXTURE_PASSWORD"));
}
