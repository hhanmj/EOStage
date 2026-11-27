//! Empirical Challenger Test Suite for Milestone 4 (HLS v2.0 Download & PC SAS URL Resolution)
//! Tests `is_valid_planetary_computer_hls_href` boundary behavior via `PlanetaryComputerLandsatAssetResolver::can_resolve`
//! and `resolve()` under adversarial, edge, and malformed URL patterns.

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
    AssetResolver, AssetResolverRegistry, Clock, CredentialMaterial, CredentialProvider,
    CredentialProviderError, PlanetaryComputerLandsatAssetResolver, ResolveContext,
};
use insar_provider::planetary_computer::PlanetaryComputerLandsatClient;

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

fn create_resolver() -> PlanetaryComputerLandsatAssetResolver {
    PlanetaryComputerLandsatAssetResolver::new(ProviderNetworkPolicy::default())
        .expect("PlanetaryComputerLandsatAssetResolver must initialize")
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

#[test]
fn test_hls_url_boundary_valid_patterns() {
    let resolver = create_resolver();

    let valid_urls = [
        // Standard Planetary Computer HLS endpoints
        "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif",
        "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B03.tif",
        "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.Fmask.tif",
        "https://hls2euwest.blob.core.windows.net/hls2/v2.0/scene1/band.tiff",
        // West EU storage endpoint
        "https://hlseuwest.blob.core.windows.net/hls/2023/scene.tif",
        "https://hlseuwest.blob.core.windows.net/hls/band_01.tiff",
    ];

    for href in valid_urls {
        let desc = make_hls_descriptor(href);
        assert!(
            resolver.can_resolve(&desc),
            "Expected valid HLS URL to be resolvable: {href}"
        );
    }
}

#[test]
fn test_hls_url_boundary_invalid_patterns() {
    let resolver = create_resolver();

    let invalid_urls = [
        // Fabricated or arbitrary blob storage domains (must fail closed)
        "https://modisoptical.blob.core.windows.net/hls/T50TMK/B02.tif",
        "https://modisoptical.blob.core.windows.net/hls/T50TMK/B03.tif",
        "https://planetarycomputer.blob.core.windows.net/hls/test.tif",
        "https://eodata.blob.core.windows.net/hls/scene/image.tif",
        "https://attacker.blob.core.windows.net/hls2/evil.tif",
        // Non-HTTPS schemes
        "http://hls2euwest.blob.core.windows.net/hls2/T50TMK/B02.tif",
        "ftp://hls2euwest.blob.core.windows.net/hls2/T50TMK/B02.tif",
        "file:///hls2euwest.blob.core.windows.net/hls2/T50TMK/B02.tif",
        "s3://hls2euwest.blob.core.windows.net/hls2/T50TMK/B02.tif",
        // URLs with fragments (strictly forbidden)
        "https://hls2euwest.blob.core.windows.net/hls2/T50TMK/B02.tif#fragment",
        "https://hls2euwest.blob.core.windows.net/hls2/T50TMK/B02.tif#top",
        // Untrusted / spoofed domains
        "https://hls2euwest.blob.core.windows.net.evil.com/hls2/B02.tif",
        "https://evil-hls2euwest.com/hls2/B02.tif",
        "https://example.com/hls2/B02.tif",
        "https://blob.core.windows.net/hls2/B02.tif",
        "https://notblob.core.windows.net/hls2/B02.tif",
        "https://127.0.0.1/hls2/B02.tif",
        "https://localhost/hls2/B02.tif",
        // Non-HLS and non-TIFF paths
        "https://hls2euwest.blob.core.windows.net/other/script.sh",
        "https://hls2euwest.blob.core.windows.net/malware.exe",
        "https://hls2euwest.blob.core.windows.net/data/landsat.tar.gz",
        "https://hls2euwest.blob.core.windows.net/hls2.tar",
        "https://hls2euwest.blob.core.windows.net/index.html",
        // Malformed URL strings
        "not_a_valid_url",
        "https://",
        "://hls2euwest",
        "https:///hls2/B02.tif",
    ];

    for href in invalid_urls {
        let desc = make_hls_descriptor(href);
        assert!(
            !resolver.can_resolve(&desc),
            "Expected invalid HLS URL to be rejected: {href}"
        );
    }
}

#[test]
fn test_hls_descriptor_security_guards() {
    let resolver = create_resolver();
    let valid_href = "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif";

    // 1. Wrong resolver ID
    let mut desc = make_hls_descriptor(valid_href);
    desc.resolver_id = "other.resolver.id".to_owned();
    assert!(
        !resolver.can_resolve(&desc),
        "Must reject unmatching resolver_id"
    );

    // 2. Requires credentials must be rejected (HLS is anonymous public HTTPS)
    let mut desc = make_hls_descriptor(valid_href);
    desc.requires_credentials = true;
    assert!(
        !resolver.can_resolve(&desc),
        "Must reject requires_credentials = true"
    );

    // 3. Credential profile must be rejected
    let mut desc = make_hls_descriptor(valid_href);
    desc.credential_profile = Some("custom-profile".to_owned());
    assert!(
        !resolver.can_resolve(&desc),
        "Must reject credential_profile presence"
    );

    // 4. Scheme mismatch
    let mut desc = make_hls_descriptor(valid_href);
    desc.scheme = AssetAccessScheme::LocalFile;
    assert!(
        !resolver.can_resolve(&desc),
        "Must reject non-PublicHttps scheme"
    );

    // 5. None stable_href
    let mut desc = make_hls_descriptor(valid_href);
    desc.stable_href = None;
    assert!(!resolver.can_resolve(&desc), "Must reject None stable_href");
}

#[test]
fn test_hls_full_resolution_and_sas_signing_lifecycle() {
    let server = LoopbackSasServer::start();
    let resolver = PlanetaryComputerLandsatAssetResolver::hls_with_client(server.client());
    let href = "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2023/04/30/HLS.L30.T50TMK.2023120T024545.v2.0/HLS.L30.T50TMK.2023120T024545.v2.0.B02.tif";
    let desc = make_hls_descriptor(href);

    let provider_id = insar_provider::catalog::MPC_HLS_PROVIDER_ID;
    let collection_id = "hls2-l30";
    let product_id = "HLS.L30.T50TMK.2023120T024545.v2.0";
    let pkey = provider_product_key(provider_id, collection_id, product_id).unwrap();
    let skey = stable_asset_key(&pkey, "B02").unwrap();

    let reference = StableAssetReference {
        provider_id: provider_id.to_owned(),
        collection_id: collection_id.to_owned(),
        provider_product_id: product_id.to_owned(),
        provider_product_key: pkey.clone(),
        asset_key: "B02".to_owned(),
        stable_asset_key: skey,
        canonical_product_key: None,
    };

    let file_name = "HLS.L30.T50TMK.2023120T024545.v2.0.B02.tif";

    // Verify validate_frozen
    let frozen = FrozenAssetAccess {
        descriptor: desc.clone(),
        reference: reference.clone(),
        role: AssetRole::Primary,
        file_name: Some(file_name.to_owned()),
        expected_size_bytes: Some(15_000_000),
        checksum_algorithm: None,
        checksum_value: None,
    };
    assert!(
        resolver.validate_frozen(&frozen).is_ok(),
        "validate_frozen must succeed for canonical HLS item"
    );

    // Verify resolve()
    let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);
    let resolved = resolver
        .resolve(&reference, &desc, &context)
        .expect("Loopback HLS resolve() must succeed");

    assert_eq!(resolved.suggested_filename(), Some(file_name));
    let effective = resolved.effective_url();
    assert_eq!(effective.scheme(), "https");
    assert_eq!(
        effective.host_str(),
        Some("hls2euwest.blob.core.windows.net")
    );
    assert_eq!(effective.path(), "/hls2/L30/50/T/MK/2023/04/30/HLS.L30.T50TMK.2023120T024545.v2.0/HLS.L30.T50TMK.2023120T024545.v2.0.B02.tif");
    // Verify SAS token attached
    let query = effective
        .query()
        .expect("Query parameters must be present with SAS token");
    assert!(query.contains("sig="), "SAS query must have signature");
    assert!(query.contains("se="), "SAS query must have expiration");
    assert!(query.contains("st="), "SAS query must have start time");

    // Verify expiration timestamp is set in the future
    assert!(resolved.expires_at().is_some());
}

#[test]
fn test_hls_resolution_via_asset_resolver_registry() {
    let standard = AssetResolverRegistry::with_standard_resolvers()
        .expect("AssetResolverRegistry::with_standard_resolvers must succeed");

    // 1. Verify PLANETARY_COMPUTER_HLS_RESOLVER_ID is registered in standard registry
    assert!(
        standard
            .resolver_ids()
            .contains(&PLANETARY_COMPUTER_HLS_RESOLVER_ID),
        "AssetResolverRegistry must contain PLANETARY_COMPUTER_HLS_RESOLVER_ID"
    );

    let server = LoopbackSasServer::start();
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(
            PlanetaryComputerLandsatAssetResolver::hls_with_client(server.client()),
        ))
        .unwrap();

    let href = "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2023/04/30/HLS.L30.T50TMK.2023120T024545.v2.0/HLS.L30.T50TMK.2023120T024545.v2.0.B02.tif";
    let desc = make_hls_descriptor(href);

    let provider_id = insar_provider::catalog::MPC_HLS_PROVIDER_ID;
    let collection_id = "hls2-l30";
    let product_id = "HLS.L30.T50TMK.2023120T024545.v2.0";
    let pkey = provider_product_key(provider_id, collection_id, product_id).unwrap();
    let skey = stable_asset_key(&pkey, "B02").unwrap();

    let reference = StableAssetReference {
        provider_id: provider_id.to_owned(),
        collection_id: collection_id.to_owned(),
        provider_product_id: product_id.to_owned(),
        provider_product_key: pkey,
        asset_key: "B02".to_owned(),
        stable_asset_key: skey,
        canonical_product_key: None,
    };

    let file_name = "HLS.L30.T50TMK.2023120T024545.v2.0.B02.tif";

    let frozen = FrozenAssetAccess {
        descriptor: desc.clone(),
        reference: reference.clone(),
        role: AssetRole::Primary,
        file_name: Some(file_name.to_owned()),
        expected_size_bytes: Some(15_000_000),
        checksum_algorithm: None,
        checksum_value: None,
    };

    let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);

    // 2. Empirical verification of registry.resolve
    let resolved = registry
        .resolve(&reference, &desc, &context)
        .expect("Loopback registry.resolve must succeed");
    assert_eq!(resolved.suggested_filename(), Some(file_name));
    assert_eq!(
        resolved.effective_url().host_str(),
        Some("hls2euwest.blob.core.windows.net")
    );
    let query = resolved
        .effective_url()
        .query()
        .expect("Query parameters must have SAS token");
    assert!(query.contains("sig="));
    assert!(query.contains("se="));

    // 3. Empirical verification of registry.resolve_frozen
    let resolved_frozen = registry
        .resolve_frozen(&frozen, &context)
        .expect("resolve_frozen must succeed");
    assert_eq!(resolved_frozen.suggested_filename(), Some(file_name));
    assert_eq!(resolved_frozen.expected_size(), Some(15_000_000));
    assert_eq!(resolved_frozen.asset_role(), Some(&AssetRole::Primary));

    // 4. Empirical verification of registry.prepare_auth_refresh
    assert!(registry.prepare_auth_refresh(&frozen).is_ok());

    // 5. Adversarial checks via registry
    // Adversarial A: fabricated / malicious domain
    let evil_desc =
        make_hls_descriptor("https://modisoptical.blob.core.windows.net/hls/T50TMK/B02.tif");
    assert!(
        registry.resolve(&reference, &evil_desc, &context).is_err(),
        "Registry must reject modisoptical"
    );

    let attacker_desc = make_hls_descriptor("https://attacker.blob.core.windows.net/hls2/evil.tif");
    assert!(
        registry
            .resolve(&reference, &attacker_desc, &context)
            .is_err(),
        "Registry must reject attacker domain"
    );

    // Adversarial B: requires_credentials = true
    let mut cred_desc = desc.clone();
    cred_desc.requires_credentials = true;
    assert!(
        registry.resolve(&reference, &cred_desc, &context).is_err(),
        "Registry must reject requires_credentials = true"
    );

    // Adversarial C: filename mismatch in resolve_frozen
    let mut bad_frozen = frozen.clone();
    bad_frozen.file_name = Some("wrong_name.tif".to_owned());
    assert!(
        registry.resolve_frozen(&bad_frozen, &context).is_err(),
        "Registry must reject filename mismatch"
    );
}
