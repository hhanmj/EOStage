//! Challenger Deep Dive Adversarial Test Suite for Milestone 4 Iteration 3
//!
//! Validates:
//! 1. Live and offline SAS client behavior, URL generation, and error conditions.
//! 2. Host validation against host pinning (rejecting spoofed hosts while allowing valid Azure blob storage hosts).
//! 3. Resolver routing and defense-in-depth across the entire resolution pipeline.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetResolverParameters, ProviderNetworkPolicy, StableAssetReference,
};
use insar_download::{
    AssetResolver, AssetResolverRegistry, Clock, CredentialMaterial, CredentialProvider,
    CredentialProviderError, PlanetaryComputerLandsatAssetResolver, ResolveContext,
};
use insar_provider::planetary_computer::{
    validate_hls_asset_origin_and_path, PlanetaryComputerErrorCode, PlanetaryComputerLandsatClient,
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
// SECTION 1: SAS CLIENT BEHAVIOR, LIVE/OFFLINE, URL GENERATION & ERRORS
// ============================================================================

#[test]
fn test_sas_client_offline_and_cancellation_paths() {
    let client = PlanetaryComputerLandsatClient::new(ProviderNetworkPolicy::default())
        .expect("Client creation");

    let valid_href = "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif";

    // 1. Cancellation before network call in collection_sas_token
    let cancelled = || true;
    let initial_requests = client.request_counts().sas_token;
    let cancel_res = client.collection_sas_token("hls2-l30", &cancelled);
    assert!(cancel_res.is_err(), "Must error when cancelled");
    assert_eq!(
        client.request_counts().sas_token,
        initial_requests,
        "No network request should be dispatched when cancelled"
    );

    // 2. Cancellation before network call in sign_hls_asset
    let cancel_sign_res = client.sign_hls_asset("hls2-l30", valid_href, &cancelled);
    assert!(
        cancel_sign_res.is_err(),
        "sign_hls_asset must error when cancelled"
    );
    assert_eq!(
        client.request_counts().sas_token,
        initial_requests,
        "No network request should be dispatched when cancelled in sign_hls_asset"
    );

    // 3. Unauthorized collections must fail closed immediately without dispatching network requests
    let bad_collections = [
        "",
        "hls",
        "hls2",
        "hls-l30",
        "hls-s30",
        "landsat",
        "landsat-8",
        "modisoptical",
        "admin",
        "../../etc/passwd",
        "hls2-l30\0evil",
        "hls2-l30;rm -rf /",
    ];

    for bad_col in bad_collections {
        let err = match client.collection_sas_token(bad_col, &|| false) {
            Err(e) => e,
            Ok(_) => panic!("Expected collection_sas_token to fail for '{bad_col}'"),
        };
        assert_eq!(
            err.code(),
            PlanetaryComputerErrorCode::IdentityMismatch,
            "Bad collection '{bad_col}' must be rejected with IdentityMismatch"
        );
        let sign_err = match client.sign_hls_asset(bad_col, valid_href, &|| false) {
            Err(e) => e,
            Ok(_) => panic!("Expected sign_hls_asset to fail for '{bad_col}'"),
        };
        assert_eq!(
            sign_err.code(),
            PlanetaryComputerErrorCode::IdentityMismatch
        );
    }
    assert_eq!(
        client.request_counts().sas_token,
        initial_requests,
        "Zero network requests for unauthorized collections"
    );
}

#[test]
fn test_sas_client_live_token_caching_and_refresh_lifecycle() {
    let server = LoopbackSasServer::start();
    let client = server.client();

    // First call: loopback SAS endpoint fetch
    let lease_1 = client
        .collection_sas_token("hls2-l30", &|| false)
        .expect("Initial loopback SAS token acquisition");

    let count_after_first = client.request_counts().sas_token;
    assert_eq!(count_after_first, 1, "Exactly one token request recorded");

    // Second call: MUST hit cache without increasing request count
    let lease_2 = client
        .collection_sas_token("hls2-l30", &|| false)
        .expect("Cached token acquisition");
    assert_eq!(
        client.request_counts().sas_token,
        count_after_first,
        "Second call must hit cache without dispatching network request"
    );
    assert_eq!(
        lease_1.token.as_str(),
        lease_2.token.as_str(),
        "Tokens must match from cache"
    );
    assert_eq!(
        lease_1.expires_at, lease_2.expires_at,
        "Expiration must match from cache"
    );

    // Invalidate L30 cache specifically
    client
        .invalidate_hls_sas("hls2-l30")
        .expect("Invalidate L30 cache");

    // Third call: must fetch fresh token and increment request count
    let lease_3 = client
        .collection_sas_token("hls2-l30", &|| false)
        .expect("Post-invalidation token acquisition");
    assert_eq!(
        client.request_counts().sas_token,
        count_after_first + 1,
        "Third call after cache invalidation must fetch fresh token from server"
    );
    assert!(!lease_3.token.is_empty());
    assert_ne!(
        lease_1.token.as_str(),
        lease_3.token.as_str(),
        "Refreshed token after invalidation must carry fresh sequence signature"
    );
}

#[test]
fn test_sas_signed_url_structure_and_no_parameter_leakage() {
    let server = LoopbackSasServer::start();
    let client = server.client();

    let valid_href = "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif";
    let signed = client
        .sign_hls_asset("hls2-l30", valid_href, &|| false)
        .expect("Loopback SAS signing must succeed");

    let (url, expires_at) = signed.into_parts();
    // Verify no double-question marks or mangled query
    let raw_url = url.as_str();
    assert_eq!(
        raw_url.matches('?').count(),
        1,
        "URL must have exactly one '?' separator: {raw_url}"
    );
    assert!(!raw_url.contains("??"), "URL must not contain '??'");
    assert!(!raw_url.contains("?&"), "URL must not contain '?&'");
    assert!(!raw_url.contains("&&"), "URL must not contain '&&'");

    // Verify expiration timestamp
    let now = SystemTime::now();
    assert!(expires_at > now, "Expiration must be in future");
    assert!(
        expires_at.duration_since(now).unwrap() <= Duration::from_secs(86400),
        "Expiration must be within 24h"
    );

    // Verify debug formatting redacts signed URL
    let signed_debug = client
        .sign_hls_asset("hls2-l30", valid_href, &|| false)
        .unwrap();
    let debug_output = format!("{signed_debug:?}");
    assert!(
        debug_output.contains("[REDACTED_SIGNED_URL]"),
        "PlanetaryComputerSignedAsset Debug formatting must redact signed URL!"
    );
    assert!(
        !debug_output.contains("sig="),
        "PlanetaryComputerSignedAsset Debug formatting must not leak signature!"
    );
}

// ============================================================================
// SECTION 2: HOST PINNING & SPOOFING ADVERSARIAL STRESS
// ============================================================================

#[test]
fn test_host_pinning_rejects_subdomain_and_authority_spoofing() {
    let resolver = create_hls_resolver();

    let adversarial_hosts = [
        // Prefix/suffix domain spoofing
        "https://hls2euwest.blob.core.windows.net.attacker.com/hls2/B02.tif",
        "https://attacker-hls2euwest.blob.core.windows.net/hls2/B02.tif",
        "https://hls2euwest.blob.core.windows.net-attacker.com/hls2/B02.tif",
        "https://hls2euwest.blob.core.windows.net.evil.org/hls2/B02.tif",
        "https://hlseuwest.blob.core.windows.net.evil.org/hls/B02.tif",
        // Subdomain manipulation
        "https://evil.hls2euwest.blob.core.windows.net/hls2/B02.tif",
        "https://sub.hlseuwest.blob.core.windows.net/hls/B02.tif",
        // Authority / Userinfo trickery
        "https://hls2euwest.blob.core.windows.net@attacker.com/hls2/B02.tif",
        "https://user:hls2euwest.blob.core.windows.net@attacker.com/hls2/B02.tif",
        "https://attacker.com#hls2euwest.blob.core.windows.net/hls2/B02.tif",
        // Sibling Azure storage accounts
        "https://copernicusdem.blob.core.windows.net/hls2/B02.tif",
        "https://elevationeuwest.blob.core.windows.net/hls2/B02.tif",
        "https://sentinel1euwest.blob.core.windows.net/hls2/B02.tif",
        "https://sentinel2l2a01.blob.core.windows.net/hls2/B02.tif",
        "https://landsateuwest.blob.core.windows.net/hls2/B02.tif",
        "https://modisoptical.blob.core.windows.net/hls2/B02.tif",
        "https://modisoptical.blob.core.windows.net/hls/B02.tif",
        // Generic cloud storage
        "https://blob.core.windows.net/hls2/B02.tif",
        "https://core.windows.net/hls2/B02.tif",
        "https://windows.net/hls2/B02.tif",
        "https://s3.amazonaws.com/hls2euwest.blob.core.windows.net/hls2/B02.tif",
    ];

    for href in adversarial_hosts {
        let desc = make_hls_descriptor(href);
        assert!(
            !resolver.can_resolve(&desc),
            "Resolver can_resolve MUST reject spoofed host: {href}"
        );

        if let Ok(url) = Url::parse(href) {
            assert!(
                validate_hls_asset_origin_and_path(&url).is_err(),
                "validate_hls_asset_origin_and_path MUST reject spoofed host: {href}"
            );
        }

        let reference = make_hls_reference("scene", "B02");
        let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);
        assert!(
            resolver.resolve(&reference, &desc, &context).is_err(),
            "Resolver resolve() MUST reject spoofed host: {href}"
        );
    }
}

#[test]
fn test_host_pinning_rejects_tampered_schemes_ports_paths_and_traversals() {
    let resolver = create_hls_resolver();
    let reference = make_hls_reference("HLS.L30.T50TMK.2024177T024545.v2.0", "B02");
    let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);

    // 1. URLs that can_resolve rejects upfront (invalid schemes, containers, extensions, fragments)
    let can_resolve_rejections = [
        // Non-HTTPS schemes
        "http://hls2euwest.blob.core.windows.net/hls2/B02.tif",
        "ftp://hls2euwest.blob.core.windows.net/hls2/B02.tif",
        "file:///hls2euwest.blob.core.windows.net/hls2/B02.tif",
        "s3://hls2euwest.blob.core.windows.net/hls2/B02.tif",
        "gopher://hls2euwest.blob.core.windows.net/hls2/B02.tif",
        // Path container mismatches
        "https://hls2euwest.blob.core.windows.net/other_container/B02.tif",
        "https://hls2euwest.blob.core.windows.net/hls3/B02.tif",
        "https://hls2euwest.blob.core.windows.net/hls2_fake/B02.tif",
        "https://hlseuwest.blob.core.windows.net/hls_fake/B02.tif",
        "https://hls2euwest.blob.core.windows.net/",
        "https://hls2euwest.blob.core.windows.net/hls2",
        "https://hls2euwest.blob.core.windows.net/hls2/",
        // Extension tampering
        "https://hls2euwest.blob.core.windows.net/hls2/B02.png",
        "https://hls2euwest.blob.core.windows.net/hls2/B02.jpg",
        "https://hls2euwest.blob.core.windows.net/hls2/B02.exe",
        "https://hls2euwest.blob.core.windows.net/hls2/B02.tar",
        "https://hls2euwest.blob.core.windows.net/hls2/B02.tar.gz",
        "https://hls2euwest.blob.core.windows.net/hls2/B02.tif.exe",
        "https://hls2euwest.blob.core.windows.net/hls2/B02.tiff.sh",
        // Fragments
        "https://hls2euwest.blob.core.windows.net/hls2/B02.tif#fragment",
    ];

    for href in can_resolve_rejections {
        let desc = make_hls_descriptor(href);
        assert!(
            !resolver.can_resolve(&desc),
            "Resolver can_resolve MUST reject: {href}"
        );

        if let Ok(url) = Url::parse(href) {
            assert!(
                validate_hls_asset_origin_and_path(&url).is_err(),
                "validate_hls_asset_origin_and_path MUST reject: {href}"
            );
        }
    }

    // 2. Defense-in-depth: Port injection, userinfo, path traversal, and pre-injected queries
    // These may match basic scheme/host/extension syntactic patterns, but MUST be strictly
    // rejected by validate_hls_asset_origin_and_path and resolve()
    let defense_in_depth_attacks = [
        "https://hls2euwest.blob.core.windows.net:80/hls2/B02.tif",
        "https://hls2euwest.blob.core.windows.net:8080/hls2/B02.tif",
        "https://hls2euwest.blob.core.windows.net:8443/hls2/B02.tif",
        "https://hlseuwest.blob.core.windows.net:444/hls/B02.tif",
        "https://admin:pass@hls2euwest.blob.core.windows.net/hls2/B02.tif",
        "https://user@hls2euwest.blob.core.windows.net/hls2/B02.tif",
        "https://hls2euwest.blob.core.windows.net/hls2/../../etc/passwd",
        "https://hls2euwest.blob.core.windows.net/hls2/../secret.tif",
        "https://hlseuwest.blob.core.windows.net/hls/../secret.tif",
        "https://hls2euwest.blob.core.windows.net/hls2/B02.tif?pre_injected=1",
        "https://hls2euwest.blob.core.windows.net/hls2/B02.tif?sig=forged_token",
    ];

    for href in defense_in_depth_attacks {
        let desc = make_hls_descriptor(href);
        let url = Url::parse(href).expect("valid URL parse");
        assert!(
            validate_hls_asset_origin_and_path(&url).is_err(),
            "validate_hls_asset_origin_and_path MUST block attack: {href}"
        );
        assert!(
            resolver.resolve(&reference, &desc, &context).is_err(),
            "Resolver resolve() MUST block attack: {href}"
        );
    }
}

// ============================================================================
// SECTION 3: PIPELINE RESOLUTION & RESOLVED HOST POLICY VERIFICATION
// ============================================================================

#[test]
fn test_resolved_asset_host_policy_enforcement() {
    let standard = AssetResolverRegistry::with_standard_resolvers().unwrap();
    assert!(standard
        .resolver_ids()
        .contains(&PLANETARY_COMPUTER_HLS_RESOLVER_ID));

    let server = LoopbackSasServer::start();
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(
            PlanetaryComputerLandsatAssetResolver::hls_with_client(server.client()),
        ))
        .unwrap();

    let valid_href = "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif";
    let desc = make_hls_descriptor(valid_href);
    let reference = make_hls_reference("HLS.L30.T50TMK.2024177T024545.v2.0", "B02");
    let context = ResolveContext::new(&NoCredentials, &|| false, &FixedClock);

    let resolved = registry
        .resolve(&reference, &desc, &context)
        .expect("Loopback registry resolve must succeed");

    let effective_url = resolved.effective_url();
    assert_eq!(effective_url.scheme(), "https");
    assert_eq!(
        effective_url.host_str(),
        Some("hls2euwest.blob.core.windows.net")
    );
    assert!(effective_url.query().is_some(), "Must have SAS query");

    // Check suggested filename
    assert_eq!(
        resolved.suggested_filename(),
        Some("HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif")
    );
}

#[test]
fn test_mpc_landsat_c2_l2_snapshot_item_does_not_misroute_to_hls() {
    use insar_core::SnapshotItem;
    use insar_download::ResolverExecutionRuntime;
    let raw = r#"{
  "item_id": "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_120039_20240926_02_T1::ASSET::ang",
  "provider_id": "microsoft.planetary-computer",
  "display_name": "LC08_L2SP_120039_20240926_20241001_02_T1_ANG.txt",
  "acquisition_datetime": "2024-09-26T02:37:37.856601Z",
  "footprint_geojson": null,
  "metadata": {
    "assets": [],
    "collection_profile_id": "planetary-computer.landsat-c2-l2.scene.v1",
    "delivery_asset_key": "ang",
    "delivery_asset_role": "metadata",
    "delivery_model": "archive_first",
    "delivery_source": "planetary_computer",
    "filename_rule_id": "planetary-computer.landsat-c2-l2.asset-filename.v1",
    "identity_rule_id": "planetary-computer.landsat-c2-l2.identity.v1",
    "logical_dataset": "optical.landsat.c2.l2",
    "native_collection_id": "landsat-c2-l2",
    "product_kind": "optical_scene",
    "properties": {
      "acquisition_date": "2024/09/26",
      "acquisition_precision": "timestamp",
      "browse_available": true,
      "cloud_cover": 95.21,
      "collection_category": "T1",
      "data_type_l2": "LANDSAT_8_L2SP",
      "dataset_alias": "landsat-c2-l2",
      "entity_id": "LC08_L2SP_120039_20240926_02_T1",
      "landsat_product_id": "LC08_L2SP_120039_20240926_20241001_02_T1",
      "landsat_scene_id": "LC81200392024270LGN00",
      "processing_level": "l2sp",
      "spacecraft": "landsat_8",
      "wrs_path": "120",
      "wrs_row": "039"
    },
    "provider_product_id": "LC08_L2SP_120039_20240926_02_T1",
    "provider_product_key": "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_120039_20240926_02_T1",
    "source_item_id": "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_120039_20240926_02_T1",
    "source_kind": "landsat"
  },
  "payload": {},
  "asset_access": {
    "reference": {
      "provider_id": "microsoft.planetary-computer",
      "collection_id": "landsat-c2-l2",
      "provider_product_id": "LC08_L2SP_120039_20240926_02_T1",
      "provider_product_key": "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_120039_20240926_02_T1",
      "canonical_product_key": null,
      "asset_key": "ang",
      "stable_asset_key": "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_120039_20240926_02_T1::ASSET::ang"
    },
    "descriptor": {
      "resolver_id": "planetary-computer.landsat-c2-l2.assets.v1",
      "scheme": "public_https",
      "stable_href": "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2024/120/039/LC08_L2SP_120039_20240926_20241001_02_T1/LC08_L2SP_120039_20240926_20241001_02_T1_ANG.txt",
      "requires_credentials": false,
      "credential_profile": null,
      "resolver_parameters": {}
    },
    "role": "metadata",
    "file_name": "LC08_L2SP_120039_20240926_20241001_02_T1_ANG.txt",
    "expected_size_bytes": 117122,
    "checksum_algorithm": null,
    "checksum_value": null
  },
  "execution_policy": {
    "authority": "resolver_authoritative",
    "policy_id": "planetary-computer.landsat-c2-l2.asset-set.v2",
    "collection_profile_id": "planetary-computer.landsat-c2-l2.scene.v1"
  }
}"#;
    let item: SnapshotItem = serde_json::from_str(raw).unwrap();
    let runtime = ResolverExecutionRuntime::production().unwrap();
    assert!(format!("{:?}", runtime.classify(&item)).starts_with("ResolverAuthoritative("));
    let frozen = item.asset_access.as_ref().unwrap();
    let server = LoopbackSasServer::start();
    let resolver = PlanetaryComputerLandsatAssetResolver::with_client(server.client());
    assert!(resolver.can_resolve(&frozen.descriptor));
    resolver
        .validate_frozen(frozen)
        .expect("MPC Landsat C2 L2 frozen asset must validate via Landsat branch, not HLS");
    let cancelled_ctx = ResolveContext::new(&NoCredentials, &|| true, &FixedClock);
    let err = resolver
        .resolve(&frozen.reference, &frozen.descriptor, &cancelled_ctx)
        .expect_err("cancelled context must cancel cleanly");
    assert_eq!(err.code(), "ASSET_RESOLUTION_CANCELLED");

    let live_ctx = ResolveContext::new(&NoCredentials, &|| false, &insar_download::SystemClock)
        .with_proxy_policy(insar_core::ProxyPolicy::System);
    let resolved = resolver
        .resolve(&frozen.reference, &frozen.descriptor, &live_ctx)
        .expect("MPC Landsat C2 L2 asset must resolve signed URL without HLS misrouting");
    assert_eq!(
        resolved.suggested_filename(),
        Some("LC08_L2SP_120039_20240926_20241001_02_T1_ANG.txt")
    );
    assert!(resolved
        .effective_url()
        .as_str()
        .contains("landsateuwest.blob.core.windows.net"));
}
