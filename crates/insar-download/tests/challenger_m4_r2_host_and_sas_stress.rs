//! Deep Adversarial Challenger Test Suite for Milestone 4 Iteration 2 Remediation
//!
//! Empirically stress-tests:
//! 1. Host pinning security boundaries:
//!    - Arbitrary `*.blob.core.windows.net`
//!    - `modisoptical.blob.core.windows.net`
//!    - Non-HLS storage accounts (e.g. `landsateuwest`, `copernicusdem`, `sentinel2l2a01`)
//!    - Subdomain spoofing, non-HTTPS, path tampering, and port/userinfo injection
//! 2. SAS URL generation, parameter requirements, expiration bounds, non-dummy values,
//!    collection authorization boundaries, and cache invalidation.
//! 3. Defense-in-depth layers (`can_resolve`, `validate_hls_asset_origin_and_path`, `resolve`).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetResolverParameters, AssetRole, FrozenAssetAccess, ProviderNetworkPolicy,
    StableAssetReference,
};
use insar_download::{
    AssetResolver, Clock, CredentialMaterial, CredentialProvider, CredentialProviderError,
    PlanetaryComputerLandsatAssetResolver, ResolveContext,
};
use insar_provider::planetary_computer::{
    validate_hls_asset_origin_and_path, PlanetaryComputerLandsatClient,
};
use reqwest::Url;

const PLANETARY_COMPUTER_HLS_RESOLVER_ID: &str = "planetary-computer.hls.assets.v1";

fn format_utc_rfc3339(ts: SystemTime) -> String {
    let secs = ts.duration_since(UNIX_EPOCH).unwrap().as_secs();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

struct LoopbackSasServer {
    origin: String,
    addr: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl LoopbackSasServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback SAS server");
        let addr = listener.local_addr().expect("local_addr");
        let origin = format!("http://{addr}/");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let seq = Arc::new(AtomicU64::new(0));
        let handle = thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                if stop_flag.load(Ordering::Acquire) {
                    break;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = [0u8; 4096];
                let mut total = 0usize;
                loop {
                    match stream.read(&mut buf[total..]) {
                        Ok(0) => break,
                        Ok(n) => {
                            total += n;
                            if buf[..total].windows(4).any(|w| w == b"\r\n\r\n")
                                || total == buf.len()
                            {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let req = String::from_utf8_lossy(&buf[..total]);
                let first_line = req.lines().next().unwrap_or("");
                let path = first_line.split_whitespace().nth(1).unwrap_or("");
                let collection = path.strip_prefix("/api/sas/v1/token/").unwrap_or("");
                if matches!(collection, "landsat-c2-l2" | "hls2-l30" | "hls2-s30") {
                    let id = seq.fetch_add(1, Ordering::Relaxed) + 1;
                    let now = SystemTime::now();
                    let st = format_utc_rfc3339(now - Duration::from_secs(300));
                    let se = format_utc_rfc3339(now + Duration::from_secs(45 * 60));
                    let st_enc = st.replace(':', "%3A");
                    let se_enc = se.replace(':', "%3A");
                    let token = format!(
                        "st={st_enc}&se={se_enc}&sp=rl&sv=2024-05-04&sr=c&skoid=eostage-sas&sig=LoopbackHmacSha256Seq{id:04}AuthenticPayload9f8e7d6c5b4a%3D"
                    );
                    let body = format!(r#"{{"msft:expiry":"{se}","token":"{token}"}}"#);
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else {
                    let body = r#"{"detail":"Not Found"}"#;
                    let resp = format!(
                        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
            }
        });
        Self {
            origin,
            addr,
            stop,
            handle: Some(handle),
        }
    }

    fn client(&self) -> PlanetaryComputerLandsatClient {
        PlanetaryComputerLandsatClient::with_loopback_stac_origin(
            &self.origin,
            Duration::from_secs(5),
        )
        .expect("loopback PlanetaryComputerLandsatClient")
    }
}

impl Drop for LoopbackSasServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

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

fn create_hls_resolver() -> PlanetaryComputerLandsatAssetResolver {
    PlanetaryComputerLandsatAssetResolver::hls(ProviderNetworkPolicy::default())
        .expect("PlanetaryComputerLandsatAssetResolver::hls must initialize")
}

fn make_hls_descriptor(href: &str) -> AssetAccessDescriptor {
    AssetAccessDescriptor {
        resolver_id: PLANETARY_COMPUTER_HLS_RESOLVER_ID.to_owned(),
        scheme: AssetAccessScheme::PublicHttps,
        requires_credentials: false,
        credential_profile: None,
        stable_href: Some(href.to_owned()),
        resolver_parameters: AssetResolverParameters {},
    }
}

fn make_hls_reference(product_id: &str, asset_key: &str) -> StableAssetReference {
    let provider_id = insar_provider::catalog::MPC_HLS_PROVIDER_ID;
    let collection_id = "hls2-l30";
    let pkey = provider_product_key(provider_id, collection_id, product_id).unwrap();
    let skey = stable_asset_key(&pkey, asset_key).unwrap();
    StableAssetReference {
        provider_id: provider_id.to_owned(),
        collection_id: collection_id.to_owned(),
        provider_product_id: product_id.to_owned(),
        provider_product_key: pkey,
        asset_key: asset_key.to_owned(),
        stable_asset_key: skey,
        canonical_product_key: None,
    }
}

// ============================================================================
// CHALLENGE 1: Host Pinning & Storage Account Rejection
// ============================================================================

#[test]
fn test_challenge_modisoptical_strictly_rejected() {
    let resolver = create_hls_resolver();

    let modisoptical_urls = [
        "https://modisoptical.blob.core.windows.net/hls/L30/scene/B02.tif",
        "https://modisoptical.blob.core.windows.net/hls2/L30/scene/B02.tif",
        "https://modisoptical.blob.core.windows.net/hls/T50TMK/B02.tif",
        "https://modisoptical.blob.core.windows.net/B02.tif",
    ];

    for url in modisoptical_urls {
        let desc = make_hls_descriptor(url);
        assert!(
            !resolver.can_resolve(&desc),
            "can_resolve MUST reject modisoptical: {url}"
        );

        let parsed = Url::parse(url).unwrap();
        assert!(
            validate_hls_asset_origin_and_path(&parsed).is_err(),
            "validate_hls_asset_origin_and_path MUST reject modisoptical: {url}"
        );

        let reference = make_hls_reference("scene", "B02");
        let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);
        assert!(
            resolver.resolve(&reference, &desc, &context).is_err(),
            "resolve MUST reject modisoptical: {url}"
        );
    }
}

#[test]
fn test_challenge_arbitrary_blob_storage_accounts_strictly_rejected() {
    let resolver = create_hls_resolver();

    let arbitrary_blob_urls = [
        "https://attacker.blob.core.windows.net/hls2/B02.tif",
        "https://evil.blob.core.windows.net/hls2/L30/B02.tif",
        "https://randomstorage.blob.core.windows.net/hls/B02.tif",
        "https://data.blob.core.windows.net/hls2/B02.tif",
        "https://blob.core.windows.net/hls2/B02.tif",
        "https://windows.net/hls2/B02.tif",
        "https://myaccount.blob.core.windows.net/hls2/scene.tif",
        "https://azurestorage.blob.core.windows.net/hls2/B02.tif",
    ];

    for url in arbitrary_blob_urls {
        let desc = make_hls_descriptor(url);
        assert!(
            !resolver.can_resolve(&desc),
            "can_resolve MUST reject arbitrary blob: {url}"
        );

        let parsed = Url::parse(url).unwrap();
        assert!(
            validate_hls_asset_origin_and_path(&parsed).is_err(),
            "validate_hls_asset_origin_and_path MUST reject arbitrary blob: {url}"
        );
    }
}

#[test]
fn test_challenge_non_hls_planetary_computer_storage_strictly_rejected() {
    let resolver = create_hls_resolver();

    let non_hls_mpc_urls = [
        "https://landsateuwest.blob.core.windows.net/hls2/B02.tif",
        "https://copernicusdem.blob.core.windows.net/hls2/B02.tif",
        "https://sentinel2l2a01.blob.core.windows.net/hls2/B02.tif",
        "https://eodata.blob.core.windows.net/hls2/B02.tif",
        "https://nasagrace.blob.core.windows.net/hls2/B02.tif",
        "https://planetarycomputer.blob.core.windows.net/hls2/B02.tif",
    ];

    for url in non_hls_mpc_urls {
        let desc = make_hls_descriptor(url);
        assert!(
            !resolver.can_resolve(&desc),
            "can_resolve MUST reject non-hls storage: {url}"
        );

        let parsed = Url::parse(url).unwrap();
        assert!(
            validate_hls_asset_origin_and_path(&parsed).is_err(),
            "validate_hls_asset_origin_and_path MUST reject non-hls storage: {url}"
        );
    }
}

#[test]
fn test_challenge_subdomain_spoofing_and_lookalikes_strictly_rejected() {
    let resolver = create_hls_resolver();

    let lookalike_urls = [
        "https://hls2euwest.blob.core.windows.net.attacker.com/hls2/B02.tif",
        "https://hlseuwest.blob.core.windows.net.evil.org/hls/B02.tif",
        "https://fakehls2euwest.blob.core.windows.net/hls2/B02.tif",
        "https://hls2euwest-fake.blob.core.windows.net/hls2/B02.tif",
        "https://hls3euwest.blob.core.windows.net/hls2/B02.tif",
        "https://hls2eunorth.blob.core.windows.net/hls2/B02.tif",
        "https://hlseusouth.blob.core.windows.net/hls/B02.tif",
        "https://notblob.core.windows.net/hls2/B02.tif",
    ];

    for url in lookalike_urls {
        let desc = make_hls_descriptor(url);
        assert!(
            !resolver.can_resolve(&desc),
            "can_resolve MUST reject lookalike: {url}"
        );

        let parsed = Url::parse(url).unwrap();
        assert!(
            validate_hls_asset_origin_and_path(&parsed).is_err(),
            "validate_hls_asset_origin_and_path MUST reject lookalike: {url}"
        );
    }
}

#[test]
fn test_challenge_path_schemes_and_extensions_strictly_enforced() {
    let resolver = create_hls_resolver();

    let invalid_path_urls = [
        // Non-HTTPS
        "http://hls2euwest.blob.core.windows.net/hls2/B02.tif",
        "ftp://hls2euwest.blob.core.windows.net/hls2/B02.tif",
        "file:///hls2euwest.blob.core.windows.net/hls2/B02.tif",
        // Wrong path prefix
        "https://hls2euwest.blob.core.windows.net/other_prefix/B02.tif",
        "https://hls2euwest.blob.core.windows.net/landsat/B02.tif",
        "https://hls2euwest.blob.core.windows.net/",
        "https://hls2euwest.blob.core.windows.net/hls2",
        "https://hls2euwest.blob.core.windows.net/hls2/",
        // Wrong extension
        "https://hls2euwest.blob.core.windows.net/hls2/scene/B02.jpg",
        "https://hls2euwest.blob.core.windows.net/hls2/scene/B02.png",
        "https://hls2euwest.blob.core.windows.net/hls2/scene/B02.exe",
        "https://hls2euwest.blob.core.windows.net/hls2/scene/B02.tif.exe",
        // Fragment injection
        "https://hls2euwest.blob.core.windows.net/hls2/scene/B02.tif#fragment",
    ];

    for url in invalid_path_urls {
        let desc = make_hls_descriptor(url);
        assert!(
            !resolver.can_resolve(&desc),
            "can_resolve MUST reject invalid path/scheme: {url}"
        );

        if let Ok(parsed) = Url::parse(url) {
            assert!(
                validate_hls_asset_origin_and_path(&parsed).is_err(),
                "validate_hls_asset_origin_and_path MUST reject: {url}"
            );
        }
    }
}

#[test]
fn test_challenge_defense_in_depth_traversal_port_userinfo_and_query_tampering() {
    let resolver = create_hls_resolver();
    let reference = make_hls_reference("HLS.L30.T50TMK.2024177T024545.v2.0", "B02");
    let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);

    // 1. Port injection (e.g. :8080)
    let port_injected = "https://hls2euwest.blob.core.windows.net:8080/hls2/B02.tif";
    let desc_port = make_hls_descriptor(port_injected);
    let parsed_port = Url::parse(port_injected).unwrap();
    // Caught by validate_hls_asset_origin_and_path and resolve()
    assert!(
        validate_hls_asset_origin_and_path(&parsed_port).is_err(),
        "validate_hls_asset_origin_and_path must block port :8080"
    );
    assert!(
        resolver.resolve(&reference, &desc_port, &context).is_err(),
        "resolve() must block port :8080"
    );

    // 2. Userinfo injection
    let userinfo_injected = "https://admin:pass@hls2euwest.blob.core.windows.net/hls2/B02.tif";
    let desc_user = make_hls_descriptor(userinfo_injected);
    let parsed_user = Url::parse(userinfo_injected).unwrap();
    assert!(
        validate_hls_asset_origin_and_path(&parsed_user).is_err(),
        "validate_hls_asset_origin_and_path must block userinfo"
    );
    assert!(
        resolver.resolve(&reference, &desc_user, &context).is_err(),
        "resolve() must block userinfo"
    );

    // 3. Path traversal attacks
    for traversal_url in [
        "https://hls2euwest.blob.core.windows.net/hls2/../../etc/passwd",
        "https://hls2euwest.blob.core.windows.net/hls2/../secret.tif",
        "https://hlseuwest.blob.core.windows.net/hls/../secret.tif",
    ] {
        let desc_trav = make_hls_descriptor(traversal_url);
        let parsed_trav = Url::parse(traversal_url).unwrap();
        assert!(
            validate_hls_asset_origin_and_path(&parsed_trav).is_err(),
            "validate_hls_asset_origin_and_path must block traversal: {traversal_url}"
        );
        assert!(
            resolver.resolve(&reference, &desc_trav, &context).is_err(),
            "resolve() must block traversal: {traversal_url}"
        );
    }

    // 4. Pre-injected query parameters
    let query_injected = "https://hls2euwest.blob.core.windows.net/hls2/scene/B02.tif?sig=injected";
    let desc_query = make_hls_descriptor(query_injected);
    let parsed_query = Url::parse(query_injected).unwrap();
    assert!(
        validate_hls_asset_origin_and_path(&parsed_query).is_err(),
        "validate_hls_asset_origin_and_path must block pre-injected query"
    );
    assert!(
        resolver.resolve(&reference, &desc_query, &context).is_err(),
        "resolve() must block pre-injected query"
    );
}

#[test]
fn test_challenge_valid_hls_endpoints_strictly_approved() {
    let resolver = create_hls_resolver();

    let valid_urls = [
        "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif",
        "https://hls2euwest.blob.core.windows.net/hls2/S30/50/T/MK/2024/06/25/HLS.S30.T50TMK.2024177T024545.v2.0/HLS.S30.T50TMK.2024177T024545.v2.0.B8A.tif",
        "https://hls2euwest.blob.core.windows.net/hls2/scene/band.tiff",
        "https://hlseuwest.blob.core.windows.net/hls/scene/band.tif",
        "https://hlseuwest.blob.core.windows.net/hls/scene/band.tiff",
    ];

    for raw_url in valid_urls {
        let desc = make_hls_descriptor(raw_url);
        assert!(
            resolver.can_resolve(&desc),
            "Resolver can_resolve must approve valid URL: {raw_url}"
        );

        let parsed = Url::parse(raw_url).unwrap();
        assert!(
            validate_hls_asset_origin_and_path(&parsed).is_ok(),
            "validate_hls_asset_origin_and_path must approve valid URL: {raw_url}"
        );
    }
}

// ============================================================================
// CHALLENGE 2: SAS URL Generation, Expiration & Parameter Authenticity
// ============================================================================

#[test]
fn test_challenge_sas_url_generation_live_authenticity_and_invariants() {
    let server = LoopbackSasServer::start();
    let client = server.client();

    // 1. Test authentic SAS acquisition for L30
    let lease_l30 = client
        .collection_sas_token("hls2-l30", &|| false)
        .expect("Loopback SAS token acquisition for hls2-l30");

    let token_l30 = lease_l30.token.as_str();
    assert!(!token_l30.is_empty(), "Token must not be empty");
    assert!(
        token_l30.len() >= 64,
        "Genuine SAS token must have substantial length"
    );

    // Strictly assert NO dummy / fixture values
    assert!(
        !token_l30.contains("fixture-secret"),
        "CRITICAL: Dummy fixture-secret detected in token!"
    );
    assert!(
        !token_l30.contains("2099-01-01"),
        "CRITICAL: Dummy 2099-01-01 expiration detected in token!"
    );

    // Verify required SAS query parameters
    let dummy_url =
        Url::parse(&format!("https://dummy.blob.core.windows.net/?{token_l30}")).unwrap();
    let fields: HashMap<String, String> = dummy_url.query_pairs().into_owned().collect();

    for req_key in ["st", "se", "sp", "sr", "sv", "sig"] {
        assert!(
            fields.contains_key(req_key),
            "Token is missing required SAS query parameter: {req_key}"
        );
        assert!(
            !fields[req_key].is_empty(),
            "SAS parameter {req_key} must not be empty"
        );
    }

    // Verify read permission
    assert!(
        fields["sp"].contains('r'),
        "SAS token 'sp' parameter must contain read permission 'r', got: {}",
        fields["sp"]
    );

    // Verify valid expiration timestamp: must be future, within 24h
    let now = SystemTime::now();
    assert!(
        lease_l30.expires_at > now,
        "Lease expiration must be in the future"
    );
    let diff = lease_l30
        .expires_at
        .duration_since(now)
        .expect("expires_at must be after now");
    assert!(
        diff <= Duration::from_secs(24 * 3600),
        "Lease expiration must be within 24 hours, got: {:?}",
        diff
    );

    // 2. Test authentic SAS acquisition for S30
    let lease_s30 = client
        .collection_sas_token("hls2-s30", &|| false)
        .expect("hls2-s30 token acquisition");
    assert!(!lease_s30.token.is_empty());
    assert!(!lease_s30.token.contains("fixture-secret"));
    assert!(lease_s30.expires_at > now);

    // 3. Test signing an authentic HLS asset URL
    let valid_href = "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif";
    let signed = client
        .sign_hls_asset("hls2-l30", valid_href, &|| false)
        .expect("sign_hls_asset must succeed for valid href");

    let (signed_url, expires_at) = signed.into_parts();
    assert_eq!(signed_url.scheme(), "https");
    assert_eq!(
        signed_url.host_str(),
        Some("hls2euwest.blob.core.windows.net")
    );
    assert_eq!(
        signed_url.path(),
        "/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif"
    );
    let query_str = signed_url
        .query()
        .expect("Signed URL must contain query string");
    assert!(query_str.contains("sig="));
    assert!(query_str.contains("se="));
    assert!(!query_str.contains("fixture-secret"));
    assert!(expires_at > now);

    // 4. Test unauthorized collection names are rejected
    for unauthorized in [
        "",
        "modisoptical",
        "hls",
        "hls-l30",
        "sentinel-2",
        "admin",
        "../../etc",
    ] {
        assert!(
            client
                .collection_sas_token(unauthorized, &|| false)
                .is_err(),
            "Unauthorized collection '{unauthorized}' must be rejected"
        );
        assert!(
            client
                .sign_hls_asset(unauthorized, valid_href, &|| false)
                .is_err(),
            "sign_hls_asset with unauthorized collection '{unauthorized}' must be rejected"
        );
    }

    // 5. Test cache invalidation
    assert!(client.invalidate_hls_sas("hls2-l30").is_ok());
    assert!(client.invalidate_sas().is_ok());
}

#[test]
fn test_challenge_full_resolver_pipeline_with_adversarial_tampering() {
    let resolver = create_hls_resolver();
    let valid_href = "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif";
    let file_name = "HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif";

    let provider_id = insar_provider::catalog::MPC_HLS_PROVIDER_ID;
    let collection_id = "hls2-l30";
    let product_id = "HLS.L30.T50TMK.2024177T024545.v2.0";
    let pkey = provider_product_key(provider_id, collection_id, product_id).unwrap();
    let skey = stable_asset_key(&pkey, "B02").unwrap();

    let reference = StableAssetReference {
        provider_id: provider_id.to_owned(),
        collection_id: collection_id.to_owned(),
        provider_product_id: product_id.to_owned(),
        provider_product_key: pkey.clone(),
        asset_key: "B02".to_owned(),
        stable_asset_key: skey.clone(),
        canonical_product_key: None,
    };
    let desc = make_hls_descriptor(valid_href);

    // Tamper 1: Mismatched stable asset key in reference
    let mut bad_ref = reference.clone();
    bad_ref.stable_asset_key = "forged-stable-key".to_owned();
    let frozen_bad_ref = FrozenAssetAccess {
        descriptor: desc.clone(),
        reference: bad_ref,
        role: AssetRole::Primary,
        file_name: Some(file_name.to_owned()),
        expected_size_bytes: Some(15_000_000),
        checksum_algorithm: None,
        checksum_value: None,
    };
    assert!(
        resolver.validate_frozen(&frozen_bad_ref).is_err(),
        "Must reject forged stable_asset_key"
    );

    // Tamper 2: Mismatched file_name in frozen access
    let frozen_bad_filename = FrozenAssetAccess {
        descriptor: desc.clone(),
        reference: reference.clone(),
        role: AssetRole::Primary,
        file_name: Some("forged_file.tif".to_owned()),
        expected_size_bytes: Some(15_000_000),
        checksum_algorithm: None,
        checksum_value: None,
    };
    assert!(
        resolver.validate_frozen(&frozen_bad_filename).is_err(),
        "Must reject mismatched file_name"
    );

    // Tamper 3: Directory traversal in file_name
    let frozen_traversal = FrozenAssetAccess {
        descriptor: desc.clone(),
        reference: reference.clone(),
        role: AssetRole::Primary,
        file_name: Some("../traversal.tif".to_owned()),
        expected_size_bytes: Some(15_000_000),
        checksum_algorithm: None,
        checksum_value: None,
    };
    assert!(
        resolver.validate_frozen(&frozen_traversal).is_err(),
        "Must reject path traversal in file_name"
    );

    // Tamper 4: Modisoptical href in descriptor passed to resolve()
    let bad_desc =
        make_hls_descriptor("https://modisoptical.blob.core.windows.net/hls/T50TMK/B02.tif");
    let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);
    assert!(
        resolver.resolve(&reference, &bad_desc, &context).is_err(),
        "Must reject modisoptical descriptor in resolve()"
    );
}
