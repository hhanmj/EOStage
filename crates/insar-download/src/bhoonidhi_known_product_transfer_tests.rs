//! Opt-in transport diagnosis for a previously recorded CP product, without search.
//! This proves real bytes/Range only, not Store resume or a completed science crop.
use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetResolverParameters, ProviderNetworkPolicy, StableAssetReference,
};
use insar_download::{
    AssetResolver, BhoonidhiAssetResolver, BhoonidhiDownloadProbe, BhoonidhiDownloadProbeError,
    BhoonidhiDownloadProbeEvidence, BhoonidhiSession, BhoonidhiSessionTokenProvider,
    ReqwestBhoonidhiDownloadProbe, ReqwestBhoonidhiTokenEndpoint, ResolveContext, ResolvedAsset,
    SystemClock, BHOONIDHI_CREDENTIAL_PROFILE, BHOONIDHI_DOWNLOAD_RESOLVER_ID,
    BHOONIDHI_DOWNLOAD_URL,
};
use insar_provider::bhoonidhi::{BHOONIDHI_COLLECTION_ID, BHOONIDHI_PROVIDER_ID};
use reqwest::header::{ACCEPT_ENCODING, CONTENT_RANGE, ETAG, IF_RANGE, RANGE};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

// Exact public identity in commit 34252492's live SSAR Range transaction evidence.
// The M processing token is significant; do not substitute a unit fixture's N token.
const PRODUCT: &str =
    "NISAR_S1_PR_RSLC_029_063_D_079_2500_CRNA_A_20260829T135915_20260829T135945_P00500_M_P_I_001";
const CHUNK: u64 = 1024 * 1024;

fn record(root: &Path, value: Value) {
    let text = value.to_string();
    for forbidden in [
        "bearer ",
        "password",
        "access_token",
        "refresh_token",
        "authorization:",
    ] {
        assert!(!text.to_lowercase().contains(forbidden));
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("range-events.jsonl"))
        .unwrap();
    writeln!(file, "{text}").unwrap();
    println!("{text}");
}

struct ObservedProbe(PathBuf);
impl BhoonidhiDownloadProbe for ObservedProbe {
    fn probe(
        &self,
        asset: &ResolvedAsset,
        context: &ResolveContext<'_>,
    ) -> Result<BhoonidhiDownloadProbeEvidence, BhoonidhiDownloadProbeError> {
        let started = Instant::now();
        let result = ReqwestBhoonidhiDownloadProbe::new().probe(asset, context);
        let status = match &result {
            Ok(_) => Some(206),
            Err(BhoonidhiDownloadProbeError::Status(value)) => Some(*value),
            _ => None,
        };
        record(
            &self.0,
            json!({"phase":"production_resolver_probe", "ok":result.is_ok(),
            "http_status":status, "error_category":result.as_ref().err().map(|e| format!("{e:?}")),
            "elapsed_ms":started.elapsed().as_millis(), "requested_range":"bytes=0-0",
            "total_bytes":result.as_ref().ok().map(|r| r.expected_size_bytes())}),
        );
        result
    }
}

struct SessionCleanup(Arc<BhoonidhiSessionTokenProvider>);
impl Drop for SessionCleanup {
    fn drop(&mut self) {
        let _ = self.0.logout();
    }
}

fn range_bytes(
    asset: &ResolvedAsset,
    root: &Path,
    start: u64,
    length: u64,
) -> Result<Vec<u8>, &'static str> {
    let end = start
        .checked_add(length)
        .and_then(|v| v.checked_sub(1))
        .ok_or("range_overflow")?;
    if length == 0 || length > CHUNK {
        return Err("range_budget");
    }
    let etag = asset
        .remote_validator()
        .etag
        .as_deref()
        .ok_or("validator_missing")?;
    let total = asset.expected_size().ok_or("size_missing")?;
    if end >= total {
        return Err("range_outside_source");
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| "client_build")?;
    let started = Instant::now();
    let response = asset
        .apply_request_access(client.get(asset.effective_url().clone()))
        .header(RANGE, format!("bytes={start}-{end}"))
        .header(IF_RANGE, etag)
        .header(ACCEPT_ENCODING, "identity")
        .send();
    let status = response.as_ref().ok().map(|r| r.status().as_u16());
    record(
        root,
        json!({"phase":"range_response", "range_start":start,"range_length":length,
        "http_status":status,"elapsed_ms":started.elapsed().as_millis(),
        "timeout":response.as_ref().err().is_some_and(|e| e.is_timeout())}),
    );
    let response = response.map_err(|_| "range_transport")?;
    if status != Some(206) {
        return Err("range_http_status");
    }
    if response.url() != asset.effective_url()
        || response.content_length() != Some(length)
        || response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            != Some(format!("bytes {start}-{end}/{total}").as_str())
        || response.headers().get(ETAG).and_then(|v| v.to_str().ok()) != Some(etag)
    {
        return Err("range_identity_or_headers");
    }
    let mut bytes = Vec::new();
    response
        .take(length + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "range_body")?;
    if bytes.len() as u64 != length {
        return Err("range_body_length");
    }
    record(
        root,
        json!({"phase":"range_bytes", "range_start":start,"bytes":bytes.len(),
        "sha256":format!("{:x}",Sha256::digest(&bytes)),"elapsed_ms":started.elapsed().as_millis()}),
    );
    Ok(bytes)
}

#[test]
#[ignore = "explicit Owner authorization and fresh EOSTAGE_BHOONIDHI_RANGE_ROOT; real saved login and at most 2 MiB plus 4097 bytes, no catalogue"]
fn known_cp_product_download_and_range_continuation() {
    assert_eq!(
        std::env::var("EOSTAGE_BHOONIDHI_RANGE_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(std::env::var_os("EOSTAGE_BHOONIDHI_RANGE_ROOT").unwrap());
    assert!(root.is_absolute() && !root.exists());
    fs::create_dir_all(&root).unwrap();
    let policy = ProviderNetworkPolicy::system();
    let session = Arc::new(
        BhoonidhiSession::new(
            policy,
            Arc::new(ReqwestBhoonidhiTokenEndpoint::new(policy).unwrap()),
            Arc::new(SystemClock),
        )
        .unwrap(),
    );
    let provider = Arc::new(BhoonidhiSessionTokenProvider::new(session).with_desktop_login());
    let _cleanup = SessionCleanup(Arc::clone(&provider));
    let started = Instant::now();
    let auth = provider.check_authentication();
    record(
        &root,
        json!({"phase":"authentication","ok":auth.is_ok(),
        "code":auth.as_ref().err().map(|e|e.code()),"elapsed_ms":started.elapsed().as_millis(),
        "catalogue_requests":0}),
    );
    if auth.is_err() {
        let shutdown = provider.logout();
        record(
            &root,
            json!({"phase":"finished", "ok":false, "download_requests":0,
            "code":auth.as_ref().err().map(|e|e.code()), "logout_ok":shutdown.is_ok(),
            "logout_code":shutdown.as_ref().err().map(|e|e.code())}),
        );
        panic!("saved authentication failed; see safe event record");
    }
    let key =
        provider_product_key(BHOONIDHI_PROVIDER_ID, BHOONIDHI_COLLECTION_ID, PRODUCT).unwrap();
    let reference = StableAssetReference {
        provider_id: BHOONIDHI_PROVIDER_ID.into(),
        collection_id: BHOONIDHI_COLLECTION_ID.into(),
        provider_product_id: PRODUCT.into(),
        provider_product_key: key.clone(),
        canonical_product_key: None,
        asset_key: "data".into(),
        stable_asset_key: stable_asset_key(&key, "data").unwrap(),
    };
    let descriptor = AssetAccessDescriptor {
        resolver_id: BHOONIDHI_DOWNLOAD_RESOLVER_ID.into(),
        scheme: AssetAccessScheme::OauthBearerHttps,
        stable_href: Some(BHOONIDHI_DOWNLOAD_URL.into()),
        requires_credentials: true,
        credential_profile: Some(BHOONIDHI_CREDENTIAL_PROFILE.into()),
        resolver_parameters: AssetResolverParameters {},
    };
    let resolver =
        BhoonidhiAssetResolver::new(provider.clone(), Arc::new(ObservedProbe(root.clone())));
    let clock = SystemClock;
    let cancelled = || false;
    let context = ResolveContext::new(provider.as_ref(), &cancelled, &clock);
    let result = (|| -> Result<(), String> {
        let asset = resolver
            .resolve(&reference, &descriptor, &context)
            .map_err(|e| format!("{:?}", e.kind()))?;
        let first = range_bytes(&asset, &root, 0, CHUNK)?;
        if !first.starts_with(b"\x89HDF\r\n\x1a\n") {
            return Err("hdf5_signature".into());
        }
        let path = root.join("prefix.part");
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|_| "part_create")?;
        file.write_all(&first).map_err(|_| "part_write")?;
        file.sync_all().map_err(|_| "part_sync")?;
        drop(file);
        let second = range_bytes(&asset, &root, CHUNK, CHUNK)?;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|_| "part_reopen")?;
        file.write_all(&second).map_err(|_| "part_append")?;
        file.sync_all().map_err(|_| "part_sync")?;
        drop(file);
        let prefix = fs::read(&path).map_err(|_| "part_verify")?;
        if prefix[..CHUNK as usize] != first {
            return Err("part_prefix_changed".into());
        }
        let tail = range_bytes(
            &asset,
            &root,
            asset.expected_size().ok_or("size_missing")? - 4096,
            4096,
        )?;
        record(
            &root,
            json!({"phase":"range_diagnostic_complete","ok":true,"product_id":PRODUCT,
            "prefix_bytes":prefix.len(),"tail_bytes":tail.len(),"prefix_unchanged":true,
            "hdf5_signature":true,"full_download_verified":false,"store_resume_verified":false,
            "science_crop_verified":false}),
        );
        Ok(())
    })();
    let shutdown = provider.logout();
    record(
        &root,
        json!({"phase":"finished","ok":result.is_ok(),"code":result.as_ref().err(),
        "logout_ok":shutdown.is_ok(),"logout_code":shutdown.as_ref().err().map(|e|e.code())}),
    );
    assert!(shutdown.is_ok(), "test session cleanup failed");
    assert!(
        result.is_ok(),
        "real download/Range diagnostic failed: {:?}",
        result.err()
    );
}
