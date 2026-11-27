pub mod catalog;
pub mod quota;
pub mod raster_mosaic;
pub mod script_runner;

use base64::engine::general_purpose::{STANDARD as B64_STD, URL_SAFE_NO_PAD as B64_URL};
use base64::Engine;
use catalog::{
    compare_gee_datasets, curated_science_guidance, dataset_deprecation_info,
    fetch_dataset_stac_bands, known_asset_spatial_bounds, lookup_dataset_profile,
    recommend_gee_dataset_bundle, search_gee_catalog_with_options, verify_gee_dataset,
    CatalogSearchOptions, GeeDatasetProfile,
};
use quota::{
    finalize_gee_tracker, flush_pending_profiles, get_gee_quota_status, invalidate_quota_cache,
    GeeCallTracker, GeeLedgerOpKind,
};
use raster_mosaic::{
    decode_npy_tile, plan_chunk_grid, summarize_raster_stats, write_geotiff_f32_epsg4326,
    ChunkGridPlan, DecodedNpyTile, AUTO_CHUNK_CONFIRM_THRESHOLD, CHUNK_TILE_MAX_PX,
    HARD_MAX_CHUNKS,
};
use ring::rand::SystemRandom;
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
use script_runner::{
    execute_approved_gee_script, get_script_env_status, inspect_gee_python_script,
    preview_approved_gee_script, sha256_hex,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const GEE_CREDENTIAL_CHANNEL: &str = "gee-credentials";
const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const GEE_SCOPE: &str =
    "https://www.googleapis.com/auth/earthengine https://www.googleapis.com/auth/cloud-platform";

#[derive(Clone, Debug)]
struct CachedBearerToken {
    access_token: String,
    expires_at_epoch: u64,
}

static TOKEN_CACHE: LazyLock<Mutex<HashMap<String, CachedBearerToken>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedGeeCredentialMaterial {
    pub project_id: String,
    pub auth_mode: String, // "service_account" | "oauth_token"
    pub client_email: Option<String>,
    pub private_key_pem: Option<String>,
    pub token_uri: String,
    pub oauth_token: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeeArtifactRecord {
    pub artifact_id: String,
    pub title: String,
    pub dataset_id: String,
    pub operation: String,
    pub bbox: [f64; 4],
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub preview_data_url: Option<String>,
    pub before_preview_data_url: Option<String>,
    pub after_preview_data_url: Option<String>,
    pub tile_url_template: Option<String>,
    pub before_tile_url_template: Option<String>,
    pub after_tile_url_template: Option<String>,
    pub output_path: Option<String>,
    pub size_bytes: Option<u64>,
    pub vis_min: f64,
    pub vis_max: f64,
    pub palette: Vec<String>,
    pub unit: String,
    pub stats: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_layer_record: Option<Value>,
    pub created_at: u64,
}

static ARTIFACT_CACHE: LazyLock<Mutex<HashMap<String, GeeArtifactRecord>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(super) fn gee_rest_api_base(_app_data_dir: &Path) -> String {
    #[cfg(test)]
    {
        if let Ok(raw) = fs::read_to_string(_app_data_dir.join("gee_credentials.json")) {
            if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                if let Some(base) = parsed
                    .get("api_base_url")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|u| u.starts_with("http://127.0.0.1:"))
                {
                    return base.trim_end_matches('/').to_owned();
                }
            }
        }
    }
    "https://earthengine.googleapis.com".to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EffectiveGeeProxy {
    Direct,
    ProxyUrl(String),
    SystemDefault,
}

fn is_supported_http_proxy_url(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn resolve_effective_gee_proxy(app_data_dir: &Path) -> EffectiveGeeProxy {
    // 1. Check EOStage `network-settings.json` first (supports `ApplicationNetworkSettings`
    //    `proxy_enabled` / `proxy_url` as well as `mode` / `proxy_mode` / `test_loopback`).
    let net_path = app_data_dir.join("network-settings.json");
    if let Ok(raw) = fs::read_to_string(&net_path) {
        if let Ok(doc) = serde_json::from_str::<Value>(&raw) {
            if cfg!(debug_assertions)
                && doc.get("test_loopback").and_then(Value::as_bool) == Some(true)
            {
                return EffectiveGeeProxy::Direct;
            }
            let proxy_enabled_opt = doc.get("proxy_enabled").and_then(Value::as_bool);
            let mode = doc["mode"]
                .as_str()
                .or_else(|| doc["proxy_mode"].as_str())
                .map(str::trim)
                .unwrap_or("");
            if mode.eq_ignore_ascii_case("direct")
                || mode.eq_ignore_ascii_case("none")
                || proxy_enabled_opt == Some(false)
            {
                return EffectiveGeeProxy::Direct;
            }
            let configured_proxy = doc["proxy_url"]
                .as_str()
                .or_else(|| doc["https_proxy"].as_str())
                .or_else(|| doc["http_proxy"].as_str())
                .or_else(|| doc["all_proxy"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if let Some(p) = configured_proxy {
                if proxy_enabled_opt == Some(true)
                    || (proxy_enabled_opt.is_none()
                        && (mode.eq_ignore_ascii_case("manual")
                            || mode.eq_ignore_ascii_case("custom")
                            || mode.is_empty()))
                {
                    let normalized = if p.contains("://") {
                        p.to_owned()
                    } else {
                        format!("http://{p}")
                    };
                    if is_supported_http_proxy_url(&normalized) {
                        return EffectiveGeeProxy::ProxyUrl(normalized);
                    }
                }
            }
        }
    }

    // 2. Check process environment variables (only HTTP/HTTPS URLs; reqwest has no `socks` feature)
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() && is_supported_http_proxy_url(trimmed) {
                return EffectiveGeeProxy::ProxyUrl(trimmed.to_owned());
            }
        }
    }

    // 3. On macOS, inspect `scutil --proxy` and pick HTTP/HTTPS proxy explicitly so reqwest's
    //    `system-proxy` never collides with a dual HTTP+SOCKS system configuration.
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("/usr/sbin/scutil")
            .arg("--proxy")
            .output()
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                let mut https_enable = false;
                let mut https_host = String::new();
                let mut https_port = String::new();
                let mut http_enable = false;
                let mut http_host = String::new();
                let mut http_port = String::new();
                for line in text.lines() {
                    let parts: Vec<&str> = line.split(':').map(str::trim).collect();
                    if parts.len() == 2 {
                        match parts[0] {
                            "HTTPSEnable" => https_enable = parts[1] == "1",
                            "HTTPSProxy" => https_host = parts[1].to_owned(),
                            "HTTPSPort" => https_port = parts[1].to_owned(),
                            "HTTPEnable" => http_enable = parts[1] == "1",
                            "HTTPProxy" => http_host = parts[1].to_owned(),
                            "HTTPPort" => http_port = parts[1].to_owned(),
                            _ => {}
                        }
                    }
                }
                if https_enable && !https_host.is_empty() && !https_port.is_empty() {
                    return EffectiveGeeProxy::ProxyUrl(format!(
                        "http://{https_host}:{https_port}"
                    ));
                }
                if http_enable && !http_host.is_empty() && !http_port.is_empty() {
                    return EffectiveGeeProxy::ProxyUrl(format!("http://{http_host}:{http_port}"));
                }
            }
        }
    }

    EffectiveGeeProxy::SystemDefault
}

/// Read configured proxy URL from EOStage `network-settings.json`, environment variables,
/// or macOS active system proxy (`scutil --proxy`).
pub fn detect_effective_proxy_url(app_data_dir: &Path) -> Option<String> {
    match resolve_effective_gee_proxy(app_data_dir) {
        EffectiveGeeProxy::ProxyUrl(url) => Some(url),
        EffectiveGeeProxy::Direct | EffectiveGeeProxy::SystemDefault => None,
    }
}

pub fn build_gee_http_client(
    app_data_dir: &Path,
    timeout_secs: u64,
) -> Result<reqwest::blocking::Client, String> {
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(timeout_secs.clamp(10, 180)))
        .user_agent("EOStage-GeeCloudSuite/1.0");

    match resolve_effective_gee_proxy(app_data_dir) {
        EffectiveGeeProxy::Direct => {
            builder = builder.no_proxy();
        }
        EffectiveGeeProxy::ProxyUrl(proxy_url) => {
            builder = builder.no_proxy();
            if let Ok(px) = reqwest::Proxy::all(&proxy_url) {
                let px = px.no_proxy(reqwest::NoProxy::from_string("127.0.0.1,localhost,::1"));
                builder = builder.proxy(px);
            }
        }
        EffectiveGeeProxy::SystemDefault => {}
    }

    builder
        .build()
        .map_err(|e| format!("GEE_HTTP_CLIENT_INIT_FAILED: {e}"))
}

// ============================================================================
// Credential Vault Resolution + Pure-Rust RS256 OAuth2 Token Minting
// ============================================================================

pub fn invalidate_gee_runtime_caches() {
    if let Ok(mut cache) = TOKEN_CACHE.lock() {
        cache.clear();
    }
    invalidate_quota_cache();
}

fn parse_credential_payload(
    raw_secret: &str,
    fallback_project_id: Option<&str>,
) -> Result<SavedGeeCredentialMaterial, String> {
    let trimmed = raw_secret.trim();
    if trimmed.is_empty() {
        return Err("GEE_CREDENTIAL_EMPTY: 凭据内容为空".to_owned());
    }

    if let Ok(doc) = serde_json::from_str::<Value>(trimmed) {
        // Case A: Envelope JSON {"project_id", "auth_mode", "secret", "client_email"}
        if let Some(inner_secret) = doc.get("secret").and_then(Value::as_str) {
            let env_proj = doc
                .get("project_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or(fallback_project_id);
            let auth_mode = doc
                .get("auth_mode")
                .and_then(Value::as_str)
                .unwrap_or("service_account");
            if auth_mode == "oauth_token" && !inner_secret.trim_start().starts_with('{') {
                let pid = env_proj.ok_or_else(|| {
                    "GEE_PROJECT_ID_MISSING: 使用 OAuth 访问令牌时必须填写 Google Cloud Project ID"
                        .to_owned()
                })?;
                return Ok(SavedGeeCredentialMaterial {
                    project_id: pid.to_owned(),
                    auth_mode: "oauth_token".to_owned(),
                    client_email: None,
                    private_key_pem: None,
                    token_uri: DEFAULT_TOKEN_URI.to_owned(),
                    oauth_token: Some(inner_secret.trim().to_owned()),
                });
            }
            return parse_credential_payload(inner_secret, env_proj);
        }

        let resolved_json_pid = fallback_project_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                doc.get("project_id")
                    .or_else(|| doc.get("quota_project_id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            });
        let token_uri = doc
            .get("token_uri")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| s.starts_with("https://"))
            .unwrap_or(DEFAULT_TOKEN_URI);

        // Case B1: Direct access_token JSON
        if let Some(tok) = doc
            .get("access_token")
            .or_else(|| doc.get("token"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let pid = resolved_json_pid.ok_or_else(|| {
                "GEE_PROJECT_ID_MISSING: 未找到 Google Cloud Project ID".to_owned()
            })?;
            return Ok(SavedGeeCredentialMaterial {
                project_id: pid.to_owned(),
                auth_mode: "oauth_token".to_owned(),
                client_email: None,
                private_key_pem: None,
                token_uri: token_uri.to_owned(),
                oauth_token: Some(tok.to_owned()),
            });
        }

        // Case B2: Google ADC / Earth Engine CLI `authorized_user` OAuth JSON (`refresh_token`)
        if doc
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
        {
            let pid = resolved_json_pid.ok_or_else(|| {
                "GEE_PROJECT_ID_MISSING: 未找到 Google Cloud Project ID".to_owned()
            })?;
            return Ok(SavedGeeCredentialMaterial {
                project_id: pid.to_owned(),
                auth_mode: "authorized_user".to_owned(),
                client_email: None,
                private_key_pem: None,
                token_uri: token_uri.to_owned(),
                oauth_token: Some(trimmed.to_owned()),
            });
        }

        // Case B3: Direct Google Cloud Service Account JSON
        let private_key = doc
            .get("private_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| s.contains("PRIVATE KEY"))
            .ok_or_else(|| {
                "GEE_SA_PRIVATE_KEY_MISSING: Service Account JSON 缺少有效的 private_key 字段"
                    .to_owned()
            })?;
        let client_email = doc
            .get("client_email")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| s.contains('@'))
            .ok_or_else(|| {
                "GEE_SA_CLIENT_EMAIL_MISSING: Service Account JSON 缺少有效的 client_email 字段"
                    .to_owned()
            })?;
        let project_id = resolved_json_pid
            .ok_or_else(|| "GEE_PROJECT_ID_MISSING: 未找到 Google Cloud Project ID".to_owned())?;

        return Ok(SavedGeeCredentialMaterial {
            project_id: project_id.to_owned(),
            auth_mode: "service_account".to_owned(),
            client_email: Some(client_email.to_owned()),
            private_key_pem: Some(private_key.to_owned()),
            token_uri: token_uri.to_owned(),
            oauth_token: None,
        });
    }

    // Case C: Bare OAuth2 access token (e.g. ya29....)
    let pid = fallback_project_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "GEE_PROJECT_ID_MISSING: 使用 OAuth 访问令牌时必须填写 Google Cloud Project ID"
                .to_owned()
        })?;
    Ok(SavedGeeCredentialMaterial {
        project_id: pid.to_owned(),
        auth_mode: "oauth_token".to_owned(),
        client_email: None,
        private_key_pem: None,
        token_uri: DEFAULT_TOKEN_URI.to_owned(),
        oauth_token: Some(trimmed.trim_start_matches("Bearer ").trim().to_owned()),
    })
}

pub fn load_saved_gee_credentials(
    app_data_dir: &Path,
) -> Result<Option<SavedGeeCredentialMaterial>, String> {
    if crate::geodata_tools::is_isolated_empty_keychain_mode() {
        return Ok(None);
    }

    let meta_path = app_data_dir.join("gee_credentials.json");
    let meta_doc: Option<Value> = fs::read_to_string(&meta_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let fallback_pid = meta_doc
        .as_ref()
        .and_then(|d| d.get("project_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // 1. Scoped keyring lookup (`gee-proj-{project_id}`) when metadata specifies `project_id`
    if let Some(pid) = fallback_pid {
        if let Some(sec) = crate::geodata_tools::resolve_gee_keyring_secret_for_project(pid) {
            if !sec.trim().is_empty() {
                return parse_credential_payload(&sec, Some(pid)).map(Some);
            }
        }
    }

    // 2. Primary bundle in system credential store (`gee-credentials`).
    // Under `#[cfg(test)]`, never read ambient host Keychain/vault unless explicitly copied into `app_data_dir`.
    #[cfg(not(test))]
    {
        if let Some((k_proj, _k_mode, k_sec, _k_email)) =
            crate::geodata_tools::resolve_gee_keyring_bundle()
        {
            if !k_sec.trim().is_empty() {
                let eff_pid = fallback_pid.or(Some(k_proj.as_str()));
                return parse_credential_payload(&k_sec, eff_pid).map(Some);
            }
        }
        if let Ok(Some(raw_secret)) =
            insar_download::with_desktop_agent_model_key(GEE_CREDENTIAL_CHANNEL, |s| s.to_owned())
        {
            if !raw_secret.trim().is_empty() {
                return parse_credential_payload(&raw_secret, fallback_pid).map(Some);
            }
        }
        if let Ok(Some((ee_pid, raw_secret))) =
            insar_download::read_active_legacy_earthengine_entry()
        {
            if !raw_secret.trim().is_empty() {
                let eff_pid = fallback_pid.or(ee_pid.as_deref());
                return parse_credential_payload(&raw_secret, eff_pid).map(Some);
            }
        }
    }

    // 3. Fallback when vault file sits directly inside `app_data_dir`
    //    (transparently reads both legacy plaintext and encrypted macOS vaults).
    let local_vault = app_data_dir.join("credentials-vault.v1.json");
    if local_vault.is_file() {
        if let Ok(Some(vdoc)) = insar_download::read_local_vault_map_readonly(&local_vault) {
            if let Some(raw_secret) = fallback_pid
                .and_then(|pid| {
                    vdoc.get(format!("eostage:agent-model:gee-proj-{pid}"))
                        .and_then(|m| m.get(format!("gee-proj-{pid}")))
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    vdoc.get("eostage:agent-model:gee-credentials")
                        .and_then(|m| m.get("gee-credentials"))
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    vdoc.get("eostage:earthengine")
                        .and_then(|m| m.get("secret"))
                        .and_then(Value::as_str)
                })
            {
                let v_pid = fallback_pid.or_else(|| {
                    vdoc.get("eostage:earthengine")
                        .and_then(|m| m.get("project_id"))
                        .and_then(Value::as_str)
                });
                return parse_credential_payload(raw_secret, v_pid).map(Some);
            }
        }
    }

    Ok(None)
}

pub fn save_gee_credentials(
    app_data_dir: &Path,
    project_id_input: Option<&str>,
    raw_secret_input: &str,
) -> Result<Value, String> {
    if crate::geodata_tools::is_isolated_empty_keychain_mode() {
        return Err(
            "GEE_ISOLATED_EMPTY_KEYCHAIN_READONLY: 当前处于空钥匙串隔离测试模式，禁止修改或覆盖本机凭据"
                .to_owned(),
        );
    }
    let parsed = parse_credential_payload(raw_secret_input, project_id_input)?;
    let envelope = json!({
        "project_id": parsed.project_id,
        "auth_mode": parsed.auth_mode,
        "client_email": parsed.client_email,
        "secret": raw_secret_input.trim(),
    });
    let envelope_str =
        serde_json::to_string(&envelope).map_err(|e| format!("GEE_SERIALIZE_ERR: {e}"))?;

    insar_download::store_desktop_agent_model_key(GEE_CREDENTIAL_CHANNEL, &envelope_str)
        .map_err(|e| format!("GEE_CREDENTIAL_STORE_ERR: {}", e.code()))?;
    let _ = insar_download::store_desktop_agent_model_key(
        &format!("gee-proj-{}", parsed.project_id),
        raw_secret_input.trim(),
    );

    // Save secret-free metadata file in app_data_dir
    let _ = fs::create_dir_all(app_data_dir);
    let meta = json!({
        "configured": true,
        "project_id": parsed.project_id,
        "auth_mode": parsed.auth_mode,
        "client_email": parsed.client_email,
        "updated_at": now_epoch_secs(),
    });
    let meta_path = app_data_dir.join("gee_credentials.json");
    let _ = fs::write(
        &meta_path,
        serde_json::to_vec_pretty(&meta).unwrap_or_default(),
    );

    // Invalidate token and quota caches so new key takes effect immediately
    invalidate_gee_runtime_caches();

    Ok(json!({
        "ok": true,
        "configured": true,
        "gee_ready": true,
        "project_id": parsed.project_id,
        "auth_mode": parsed.auth_mode,
        "client_email": parsed.client_email,
    }))
}

pub fn clear_gee_credentials(app_data_dir: &Path) -> Result<Value, String> {
    if crate::geodata_tools::is_isolated_empty_keychain_mode() {
        return Err(
            "GEE_ISOLATED_EMPTY_KEYCHAIN_READONLY: 当前处于空钥匙串隔离测试模式，禁止清除本机凭据"
                .to_owned(),
        );
    }
    if let Ok(Some(existing)) = load_saved_gee_credentials(app_data_dir) {
        let _ = insar_download::delete_desktop_agent_model_key(&format!(
            "gee-proj-{}",
            existing.project_id
        ));
    }
    let _ = insar_download::delete_desktop_agent_model_key(GEE_CREDENTIAL_CHANNEL);
    let _ = insar_download::clear_active_legacy_earthengine_entry();
    let _ = fs::remove_file(app_data_dir.join("gee_credentials.json"));
    crate::geodata_tools::scrub_gee_entries_from_local_vault(app_data_dir);
    invalidate_gee_runtime_caches();
    Ok(json!({
        "ok": true,
        "configured": false,
        "gee_ready": false,
    }))
}

pub fn gee_account_status(app_data_dir: &Path) -> Value {
    match load_saved_gee_credentials(app_data_dir) {
        Ok(Some(mat)) => json!({
            "ok": true,
            "configured": true,
            "gee_ready": true,
            "project_id": mat.project_id,
            "auth_mode": mat.auth_mode,
            "client_email": mat.client_email,
        }),
        Ok(None) => json!({
            "ok": true,
            "configured": false,
            "gee_ready": false,
            "project_id": null,
            "auth_mode": null,
            "client_email": null,
        }),
        Err(err) => json!({
            "ok": false,
            "configured": false,
            "gee_ready": false,
            "error": err,
        }),
    }
}

fn parse_pkcs8_pem_der(pem: &str) -> Result<Vec<u8>, String> {
    let mut b64 = String::with_capacity(pem.len());
    for line in pem.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("-----BEGIN") || t.starts_with("-----END") {
            continue;
        }
        b64.push_str(t);
    }
    B64_STD
        .decode(b64.as_bytes())
        .map_err(|e| format!("GEE_PEM_BASE64_INVALID: Service Account 私钥解码失败: {e}"))
}

fn sign_service_account_jwt(
    client_email: &str,
    private_key_pem: &str,
    token_uri: &str,
) -> Result<String, String> {
    let pkcs8_der = parse_pkcs8_pem_der(private_key_pem)?;
    let key_pair = RsaKeyPair::from_pkcs8(&pkcs8_der)
        .map_err(|_| "GEE_RSA_PKCS8_INVALID: 无法解析 PKCS#8 RSA 私钥".to_owned())?;

    let now = now_epoch_secs();
    let header_b64 = B64_URL.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = json!({
        "iss": client_email,
        "scope": GEE_SCOPE,
        "aud": token_uri,
        "iat": now,
        "exp": now + 3600,
    });
    let claims_bytes =
        serde_json::to_vec(&claims).map_err(|e| format!("GEE_JWT_CLAIMS_ERR: {e}"))?;
    let claims_b64 = B64_URL.encode(&claims_bytes);
    let signing_input = format!("{header_b64}.{claims_b64}");

    let rng = SystemRandom::new();
    let mut sig = vec![0u8; key_pair.public().modulus_len()];
    key_pair
        .sign(&RSA_PKCS1_SHA256, &rng, signing_input.as_bytes(), &mut sig)
        .map_err(|_| "GEE_RSA_SIGN_FAILED: Service Account JWT 签名失败".to_owned())?;
    let sig_b64 = B64_URL.encode(&sig);

    Ok(format!("{signing_input}.{sig_b64}"))
}

pub fn resolve_gee_access_token(
    app_data_dir: &Path,
    client: &reqwest::blocking::Client,
    override_mat: Option<&SavedGeeCredentialMaterial>,
) -> Result<(String, String), String> {
    let owned_mat;
    let mat = if let Some(m) = override_mat {
        m
    } else {
        owned_mat = load_saved_gee_credentials(app_data_dir)?.ok_or_else(|| {
            "GEE_ACCOUNT_NOT_CONFIGURED: 尚未配置 Google Earth Engine 账号，请在「设置 -> 数据服务账号 -> Google Earth Engine」中填入 Service Account JSON 密钥".to_owned()
        })?;
        &owned_mat
    };

    if mat.auth_mode == "oauth_token" {
        let tok = mat
            .oauth_token
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "GEE_OAUTH_TOKEN_EMPTY: OAuth 访问令牌为空".to_owned())?;
        return Ok((mat.project_id.clone(), tok.to_owned()));
    }

    if mat.auth_mode == "authorized_user" {
        let raw_json = mat
            .oauth_token
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "GEE_OAUTH_TOKEN_EMPTY: authorized_user JSON 为空".to_owned())?;
        let cache_key = format!("authorized_user:{}", mat.project_id);
        let now = now_epoch_secs();
        if let Ok(guard) = TOKEN_CACHE.lock() {
            if let Some(cached) = guard.get(&cache_key) {
                if cached.expires_at_epoch > now + 120 {
                    return Ok((mat.project_id.clone(), cached.access_token.clone()));
                }
            }
        }
        let tok = crate::geodata_tools::resolve_gee_bearer_token(app_data_dir, client, raw_json)?;
        if let Ok(mut guard) = TOKEN_CACHE.lock() {
            guard.insert(
                cache_key,
                CachedBearerToken {
                    access_token: tok.clone(),
                    expires_at_epoch: now + 3000,
                },
            );
        }
        return Ok((mat.project_id.clone(), tok));
    }

    let email = mat
        .client_email
        .as_deref()
        .ok_or_else(|| "GEE_CLIENT_EMAIL_MISSING".to_owned())?;
    let pem = mat
        .private_key_pem
        .as_deref()
        .ok_or_else(|| "GEE_PRIVATE_KEY_MISSING".to_owned())?;
    let cache_key = format!("{}:{email}", mat.project_id);
    let now = now_epoch_secs();

    if let Ok(guard) = TOKEN_CACHE.lock() {
        if let Some(cached) = guard.get(&cache_key) {
            if cached.expires_at_epoch > now + 120 {
                return Ok((mat.project_id.clone(), cached.access_token.clone()));
            }
        }
    }

    let jwt = sign_service_account_jwt(email, pem, &mat.token_uri)?;
    let retry_delays_ms: [Option<u64>; 3] = [Some(250), Some(600), None];
    let mut status_code = 0_u16;
    let mut raw_bytes = Vec::new();
    let mut last_net_err: Option<String> = None;

    for retry_delay in retry_delays_ms {
        match client
            .post(&mat.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", jwt.as_str()),
            ])
            .send()
        {
            Ok(resp) => {
                let st = resp.status().as_u16();
                match resp.bytes() {
                    Ok(bytes) => {
                        status_code = st;
                        raw_bytes = bytes.to_vec();
                        last_net_err = None;
                        if let (502..=504, Some(delay_ms)) = (st, retry_delay) {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                            continue;
                        }
                        break;
                    }
                    Err(e) => {
                        last_net_err = Some(format!("GEE_OAUTH_RESPONSE_READ_ERROR: {e}"));
                        if let Some(delay_ms) = retry_delay {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                            continue;
                        }
                    }
                }
            }
            Err(e) => {
                last_net_err = Some(format!(
                    "GEE_OAUTH_TOKEN_EXCHANGE_NETWORK_ERROR: 无法连接 Google OAuth2 服务（请检查网络或代理设置）: {e}"
                ));
                if let Some(delay_ms) = retry_delay {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    continue;
                }
            }
        }
    }

    if let Some(err_msg) = last_net_err {
        return Err(err_msg);
    }

    if !(200..300).contains(&status_code) {
        let desc = serde_json::from_slice::<Value>(&raw_bytes)
            .ok()
            .and_then(|body| {
                body["error_description"]
                    .as_str()
                    .or_else(|| body["error"].as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| {
                let txt = String::from_utf8_lossy(&raw_bytes);
                let trimmed = txt.trim();
                if trimmed.is_empty() {
                    "未知鉴权错误".to_owned()
                } else {
                    trimmed.chars().take(240).collect()
                }
            });
        return Err(format!(
            "GEE_OAUTH_EXCHANGE_REJECTED (HTTP {status_code}): {desc}"
        ));
    }

    let body: Value = serde_json::from_slice(&raw_bytes)
        .map_err(|e| format!("GEE_OAUTH_RESPONSE_PARSE_ERROR: {e}"))?;

    let access_token = body["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "GEE_OAUTH_NO_ACCESS_TOKEN: Google 未返回 access_token".to_owned())?
        .to_owned();
    let expires_in = body["expires_in"].as_u64().unwrap_or(3600);

    if let Ok(mut guard) = TOKEN_CACHE.lock() {
        guard.insert(
            cache_key,
            CachedBearerToken {
                access_token: access_token.clone(),
                expires_at_epoch: now + expires_in,
            },
        );
    }

    Ok((mat.project_id.clone(), access_token))
}

// ============================================================================
// Google Earth Engine REST API v1 Expression DAG Builder & Callers
// ============================================================================

#[derive(Default, Clone, Debug)]
struct ExprBuilder {
    next_id: usize,
    values: serde_json::Map<String, Value>,
}

impl ExprBuilder {
    fn new() -> Self {
        Self::default()
    }

    fn alloc(&mut self, node_val: Value) -> String {
        let id = self.next_id.to_string();
        self.next_id += 1;
        self.values.insert(id.clone(), node_val);
        id
    }

    fn constant(&mut self, v: Value) -> String {
        self.alloc(json!({ "constantValue": v }))
    }

    fn call(&mut self, func_name: &str, args: &[(&str, &str)]) -> String {
        let mut arg_map = serde_json::Map::new();
        for (k, ref_id) in args {
            arg_map.insert((*k).to_owned(), json!({ "valueReference": *ref_id }));
        }
        self.alloc(json!({
            "functionInvocationValue": {
                "functionName": func_name,
                "arguments": arg_map,
            }
        }))
    }

    fn build(self, root_id: &str) -> Value {
        json!({
            "result": root_id,
            "values": self.values,
        })
    }

    fn rectangle(&mut self, bbox: [f64; 4]) -> String {
        let min_lon = bbox[0].min(bbox[2]);
        let min_lat = bbox[1].min(bbox[3]);
        let max_lon = bbox[0].max(bbox[2]);
        let max_lat = bbox[1].max(bbox[3]);
        let coords = self.constant(json!([min_lon, min_lat, max_lon, max_lat]));
        // Explicit planar CRS + `geodesic: false` avoids northward bowing on wide E-W extents (e.g. Tibet)
        let epsg = self.constant(json!("EPSG:4326"));
        let crs = self.call("Projection", &[("crs", &epsg)]);
        let geodesic = self.constant(json!(false));
        let even_odd = self.constant(json!(true));
        self.call(
            "GeometryConstructors.Rectangle",
            &[
                ("coordinates", &coords),
                ("crs", &crs),
                ("geodesic", &geodesic),
                ("evenOdd", &even_odd),
            ],
        )
    }

    fn filtered_collection(
        &mut self,
        asset_id: &str,
        geom_node: &str,
        start_date: Option<&str>,
        end_date: Option<&str>,
        cloud_prop: Option<&str>,
        max_cloud: Option<f64>,
    ) -> String {
        let cid = self.constant(json!(asset_id));
        let mut col = self.call("ImageCollection.load", &[("id", &cid)]);

        // Spatial filter
        let all_field = self.constant(json!(".all"));
        let feat = self.call("Feature", &[("geometry", geom_node)]);
        let spatial_filter = self.call(
            "Filter.intersects",
            &[("leftField", &all_field), ("rightValue", &feat)],
        );
        col = self.call(
            "Collection.filter",
            &[("collection", &col), ("filter", &spatial_filter)],
        );

        // Temporal filter (skip for static spatial-tile DEM / landcover collections)
        let is_static_spatial_collection = matches!(
            asset_id.trim(),
            "COPERNICUS/DEM/GLO30"
                | "COPERNICUS/DEM/GLO90"
                | "ESA/WorldCover/v100"
                | "ESA/WorldCover/v200"
                | "JRC/GSW1_4/GlobalSurfaceWater"
                | "JAXA/ALOS/AW3D30/V3_2"
                | "projects/climate-and-sustainability/assets/gem-15_v1"
                | "projects/sat-io/open-datasets/FABDEM"
                | "projects/sat-io/open-datasets/GHSL/GHSL_BUILT_H_AGBH_2018_100M"
                | "UMD/hansen/global_forest_change_2023_v1_11"
        );
        if !is_static_spatial_collection {
            if let (Some(s), Some(e)) = (
                start_date.map(str::trim).filter(|x| !x.is_empty()),
                end_date.map(str::trim).filter(|x| !x.is_empty()),
            ) {
                // Note: GEE DateRange end is exclusive when given YYYY-MM-DD; if s == e, append T23:59:59Z
                let end_adj = if s == e && !e.contains('T') {
                    format!("{e}T23:59:59Z")
                } else {
                    e.to_owned()
                };
                let s_node = self.constant(json!(s));
                let e_node = self.constant(json!(end_adj));
                let dr = self.call("DateRange", &[("start", &s_node), ("end", &e_node)]);
                let ts_field = self.constant(json!("system:time_start"));
                let date_filter = self.call(
                    "Filter.dateRangeContains",
                    &[("leftValue", &dr), ("rightField", &ts_field)],
                );
                col = self.call(
                    "Collection.filter",
                    &[("collection", &col), ("filter", &date_filter)],
                );
            }
        }

        // Cloud cover filter
        if let (Some(prop), Some(cloud_lim)) = (
            cloud_prop.map(str::trim).filter(|x| !x.is_empty()),
            max_cloud.filter(|c| c.is_finite() && *c >= 0.0 && *c < 100.0),
        ) {
            let prop_node = self.constant(json!(prop));
            let lim_node = self.constant(json!(cloud_lim));
            let cloud_filter = self.call(
                "Filter.lessThan",
                &[("leftField", &prop_node), ("rightValue", &lim_node)],
            );
            col = self.call(
                "Collection.filter",
                &[("collection", &col), ("filter", &cloud_filter)],
            );
        }

        col
    }

    fn apply_sar_filters(
        &mut self,
        mut col: String,
        asset_id: &str,
        sar_polarization: Option<&str>,
        orbit_direction: Option<&str>,
    ) -> String {
        if asset_id.trim().eq_ignore_ascii_case("COPERNICUS/S1_GRD") {
            if let Some(pol) = sar_polarization
                .map(str::trim)
                .filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case("ALL"))
            {
                let upper_pol = pol.to_ascii_uppercase();
                let pol_field = self.constant(json!("transmitterReceiverPolarisation"));
                let pol_val = self.constant(json!(upper_pol));
                let pol_filter = self.call(
                    "Filter.listContains",
                    &[("leftField", &pol_field), ("rightValue", &pol_val)],
                );
                col = self.call(
                    "Collection.filter",
                    &[("collection", &col), ("filter", &pol_filter)],
                );
            }
            if let Some(orb) = orbit_direction
                .map(str::trim)
                .filter(|o| !o.is_empty() && !o.eq_ignore_ascii_case("ALL"))
            {
                let norm_orb = if orb.eq_ignore_ascii_case("ascending")
                    || orb.eq_ignore_ascii_case("asc")
                {
                    Some("ASCENDING")
                } else if orb.eq_ignore_ascii_case("descending") || orb.eq_ignore_ascii_case("desc")
                {
                    Some("DESCENDING")
                } else {
                    None
                };
                if let Some(pass) = norm_orb {
                    let orb_field = self.constant(json!("orbitProperties_pass"));
                    let orb_val = self.constant(json!(pass));
                    let orb_filter = self.call(
                        "Filter.equals",
                        &[("leftField", &orb_field), ("rightValue", &orb_val)],
                    );
                    col = self.call(
                        "Collection.filter",
                        &[("collection", &col), ("filter", &orb_filter)],
                    );
                }
            }
        }
        col
    }

    /// Composite a filtered collection using `median` (default), `mean`, `min`, `max`, or `mosaic`,
    /// and select `raw_bands` while stripping the reducer suffix (`_median` etc.) back to `raw_bands`!
    fn composite_and_select_bands(
        &mut self,
        col_node: &str,
        reducer_mode: &str,
        raw_bands: &[String],
    ) -> String {
        let mode = reducer_mode.trim().to_ascii_lowercase();
        if mode == "mosaic" {
            let mos = self.call("ImageCollection.mosaic", &[("collection", col_node)]);
            if raw_bands.is_empty() {
                return mos;
            }
            let sel = self.constant(json!(raw_bands));
            return self.call("Image.select", &[("input", &mos), ("bandSelectors", &sel)]);
        }

        let (reducer_fn, suffix) = match mode.as_str() {
            "mean" => ("Reducer.mean", "_mean"),
            "min" => ("Reducer.min", "_min"),
            "max" => ("Reducer.max", "_max"),
            _ => ("Reducer.median", "_median"),
        };
        let red = self.call(reducer_fn, &[]);
        let reduced = self.call(
            "ImageCollection.reduce",
            &[("collection", col_node), ("reducer", &red)],
        );
        if raw_bands.is_empty() {
            return reduced;
        }
        let suffixed: Vec<String> = raw_bands.iter().map(|b| format!("{b}{suffix}")).collect();
        let sel = self.constant(json!(suffixed));
        let clean_names = self.constant(json!(raw_bands));
        self.call(
            "Image.select",
            &[
                ("input", &reduced),
                ("bandSelectors", &sel),
                ("newNames", &clean_names),
            ],
        )
    }

    fn select_single_band(&mut self, img_node: &str, band: &str) -> String {
        let sel = self.constant(json!([band]));
        self.call(
            "Image.select",
            &[("input", img_node), ("bandSelectors", &sel)],
        )
    }

    fn calibrate_optical_band(
        &mut self,
        asset_id: &str,
        band_img_node: &str,
        band_name: &str,
    ) -> String {
        let upper_id = asset_id.to_ascii_uppercase();
        if upper_id.contains("S2_SR") {
            let factor = self.constant(json!(0.0001_f64));
            let c_img = self.call("Image.constant", &[("value", &factor)]);
            return self.call(
                "Image.multiply",
                &[("image1", band_img_node), ("image2", &c_img)],
            );
        }
        if upper_id.contains("C02/T1_L2") && band_name.starts_with("SR_B") {
            let mult_v = self.constant(json!(0.0000275_f64));
            let off_v = self.constant(json!(-0.2_f64));
            let mult_img = self.call("Image.constant", &[("value", &mult_v)]);
            let off_img = self.call("Image.constant", &[("value", &off_v)]);
            let scaled = self.call(
                "Image.multiply",
                &[("image1", band_img_node), ("image2", &mult_img)],
            );
            return self.call("Image.add", &[("image1", &scaled), ("image2", &off_img)]);
        }
        band_img_node.to_owned()
    }

    fn normalized_diff_calibrated(
        &mut self,
        asset_id: &str,
        composite_node: &str,
        band_a: &str,
        band_b: &str,
        out_name: &str,
    ) -> String {
        let a_raw = self.select_single_band(composite_node, band_a);
        let b_raw = self.select_single_band(composite_node, band_b);
        let a = self.calibrate_optical_band(asset_id, &a_raw, band_a);
        let b = self.calibrate_optical_band(asset_id, &b_raw, band_b);
        let num = self.call("Image.subtract", &[("image1", &a), ("image2", &b)]);
        let den = self.call("Image.add", &[("image1", &a), ("image2", &b)]);
        let ratio = self.call("Image.divide", &[("image1", &num), ("image2", &den)]);
        let names = self.constant(json!([out_name]));
        self.call("Image.rename", &[("input", &ratio), ("names", &names)])
    }
}

fn extract_gee_http_error_message(bytes: &[u8], default_msg: &str) -> String {
    if let Ok(doc) = serde_json::from_slice::<Value>(bytes) {
        if let Some(msg) = doc["error"]["message"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return msg.to_owned();
        }
    }
    let raw_txt = String::from_utf8_lossy(bytes);
    let trimmed = raw_txt.trim();
    if trimmed.is_empty() {
        default_msg.to_owned()
    } else {
        trimmed.chars().take(300).collect()
    }
}

fn gee_post_json(
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    url: &str,
    body: &Value,
    tracker: &mut GeeCallTracker,
) -> Result<Value, String> {
    let retry_delays_ms: [Option<u64>; 3] = [Some(250), Some(600), None];
    let mut last_err = String::new();

    for retry_delay in retry_delays_ms {
        let call_t0 = std::time::Instant::now();
        let resp = match client
            .post(url)
            .bearer_auth(access_token)
            .header("X-Goog-User-Project", project_id)
            .header("X-Earth-Engine-Computation-Profiling", "1")
            .json(body)
            .send()
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("GEE_API_NETWORK_ERROR: 请求 Google Earth Engine 失败: {e}");
                if let Some(delay_ms) = retry_delay {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    continue;
                }
                return Err(last_err);
            }
        };

        let status = resp.status();
        let status_u16 = status.as_u16();
        let resp_headers = resp.headers().clone();
        let bytes = match resp.bytes() {
            Ok(b) => b,
            Err(e) => {
                let wall_secs = call_t0.elapsed().as_secs_f64();
                tracker.observe_http_response(status_u16, &resp_headers, wall_secs);
                last_err = format!("GEE_API_READ_ERROR (HTTP {status_u16}): {e}");
                if let Some(delay_ms) = retry_delay {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    continue;
                }
                return Err(last_err);
            }
        };
        let wall_secs = call_t0.elapsed().as_secs_f64();
        // Always record HTTP status & headers BEFORE checking success or decoding JSON so non-JSON 429/502 are never lost!
        tracker.observe_http_response(status_u16, &resp_headers, wall_secs);

        if let (502..=504, Some(delay_ms)) = (status_u16, retry_delay) {
            std::thread::sleep(Duration::from_millis(delay_ms));
            continue;
        }

        if !status.is_success() {
            let msg = extract_gee_http_error_message(&bytes, "未知 Earth Engine API 错误");
            return Err(format!("GEE_API_ERROR (HTTP {status_u16}): {msg}"));
        }

        return serde_json::from_slice::<Value>(&bytes)
            .map_err(|e| format!("GEE_API_PARSE_ERROR (HTTP {status_u16}): {e}"));
    }

    Err(last_err)
}

fn gee_post_bytes(
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    url: &str,
    body: &Value,
    tracker: &mut GeeCallTracker,
) -> Result<Vec<u8>, String> {
    let retry_delays_ms: [Option<u64>; 3] = [Some(250), Some(600), None];
    let mut last_err = String::new();

    for retry_delay in retry_delays_ms {
        let call_t0 = std::time::Instant::now();
        let resp = match client
            .post(url)
            .bearer_auth(access_token)
            .header("X-Goog-User-Project", project_id)
            .header("X-Earth-Engine-Computation-Profiling", "1")
            .json(body)
            .send()
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("GEE_PIXELS_NETWORK_ERROR: 请求栅格计算失败: {e}");
                if let Some(delay_ms) = retry_delay {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    continue;
                }
                return Err(last_err);
            }
        };

        let status = resp.status();
        let status_u16 = status.as_u16();
        let resp_headers = resp.headers().clone();
        let bytes = match resp.bytes() {
            Ok(b) => b,
            Err(e) => {
                let wall_secs = call_t0.elapsed().as_secs_f64();
                tracker.observe_http_response(status_u16, &resp_headers, wall_secs);
                last_err = format!("GEE_PIXELS_READ_ERROR: {e}");
                if let Some(delay_ms) = retry_delay {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    continue;
                }
                return Err(last_err);
            }
        };
        let wall_secs = call_t0.elapsed().as_secs_f64();
        tracker.observe_http_response(status_u16, &resp_headers, wall_secs);

        if let (502..=504, Some(delay_ms)) = (status_u16, retry_delay) {
            std::thread::sleep(Duration::from_millis(delay_ms));
            continue;
        }

        if !status.is_success() {
            let msg = extract_gee_http_error_message(&bytes, "未知栅格计算错误");
            return Err(format!(
                "GEE_COMPUTE_PIXELS_ERROR (HTTP {status_u16}): {msg}"
            ));
        }

        return Ok(bytes.to_vec());
    }

    Err(last_err)
}

fn compute_value(
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    expression: Value,
    tracker: &mut GeeCallTracker,
) -> Result<Value, String> {
    let url = format!("https://earthengine.googleapis.com/v1/projects/{project_id}/value:compute");
    let res = gee_post_json(
        client,
        project_id,
        access_token,
        &url,
        &json!({ "expression": expression }),
        tracker,
    )?;
    Ok(res.get("result").cloned().unwrap_or(Value::Null))
}

#[allow(clippy::too_many_arguments)]
fn try_create_map_tile_url(
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    expression: &Value,
    vis_min: f64,
    vis_max: f64,
    palette: &[String],
    tracker: &mut GeeCallTracker,
) -> Result<String, String> {
    let url = format!("https://earthengine.googleapis.com/v1/projects/{project_id}/maps");
    let mut vis = json!({
        "ranges": [{ "min": vis_min, "max": vis_max }],
    });
    if !palette.is_empty() {
        vis["paletteColors"] = json!(palette);
    }
    let body = json!({
        "expression": expression,
        "fileFormat": "PNG",
        "visualizationOptions": vis,
    });
    let res = gee_post_json(client, project_id, access_token, &url, &body, tracker)?;
    let map_name = res["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "GEE_MAP_TILE_NAME_MISSING: Earth Engine 未返回地图图层名称".to_owned())?;
    Ok(format!(
        "https://earthengine.googleapis.com/v1/{map_name}/tiles/{{z}}/{{x}}/{{y}}"
    ))
}

#[allow(clippy::too_many_arguments)]
fn create_map_tile_url(
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    expression: &Value,
    vis_min: f64,
    vis_max: f64,
    palette: &[String],
    tracker: &mut GeeCallTracker,
) -> Option<String> {
    try_create_map_tile_url(
        client,
        project_id,
        access_token,
        expression,
        vis_min,
        vis_max,
        palette,
        tracker,
    )
    .ok()
}

const WEB_MERCATOR_RADIUS_M: f64 = 6_378_137.0;
const WEB_MERCATOR_MAX_LAT: f64 = 85.051_128_779_806_6;

pub(crate) fn lon_lat_to_epsg3857(lon: f64, lat: f64) -> (f64, f64) {
    let clamped_lat = lat.clamp(-WEB_MERCATOR_MAX_LAT, WEB_MERCATOR_MAX_LAT);
    let x = WEB_MERCATOR_RADIUS_M * lon.to_radians();
    let y = WEB_MERCATOR_RADIUS_M
        * (std::f64::consts::FRAC_PI_4 + clamped_lat.to_radians() * 0.5)
            .tan()
            .ln();
    (x, y)
}

pub(crate) fn compute_epsg3857_preview_grid(bbox: [f64; 4]) -> (u32, u32, f64, f64, f64, f64) {
    let min_lon = bbox[0].min(bbox[2]);
    let max_lon = bbox[0].max(bbox[2]);
    let min_lat = bbox[1].min(bbox[3]);
    let max_lat = bbox[1].max(bbox[3]);
    let (xmin, ymin) = lon_lat_to_epsg3857(min_lon, min_lat);
    let (xmax, ymax) = lon_lat_to_epsg3857(max_lon, max_lat);
    let dx = (xmax - xmin).abs().max(1.0);
    let dy = (ymax - ymin).abs().max(1.0);
    let aspect = (dx / dy).clamp(0.35, 2.8);
    let (w, h) = if aspect >= 1.0 {
        (480u32, ((480.0 / aspect).round() as u32).clamp(120, 480))
    } else {
        (((480.0 * aspect).round() as u32).clamp(120, 480), 480u32)
    };
    let sx = dx / (w as f64);
    let sy = -(dy / (h as f64));
    (w, h, sx, sy, xmin, ymax)
}

pub(crate) fn build_compute_pixels_png_body(
    expression: &Value,
    bbox: [f64; 4],
    vis_min: f64,
    vis_max: f64,
    palette: &[String],
) -> Value {
    let (w, h, sx, sy, translate_x, translate_y) = compute_epsg3857_preview_grid(bbox);
    let mut vis = json!({
        "ranges": [{ "min": vis_min, "max": vis_max }],
    });
    if !palette.is_empty() {
        vis["paletteColors"] = json!(palette);
    }
    // Render PNG in EPSG:3857 so pixel Y scales match Leaflet's Web Mercator imageOverlay viewport
    json!({
        "expression": expression,
        "fileFormat": "PNG",
        "visualizationOptions": vis,
        "grid": {
            "dimensions": { "width": w, "height": h },
            "affineTransform": {
                "scaleX": sx,
                "shearX": 0.0,
                "translateX": translate_x,
                "shearY": 0.0,
                "scaleY": sy,
                "translateY": translate_y,
            },
            "crsCode": "EPSG:3857",
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn try_render_png_preview_data_url(
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    expression: &Value,
    bbox: [f64; 4],
    vis_min: f64,
    vis_max: f64,
    palette: &[String],
    tracker: &mut GeeCallTracker,
) -> Result<String, String> {
    let url =
        format!("https://earthengine.googleapis.com/v1/projects/{project_id}/image:computePixels");
    let body = build_compute_pixels_png_body(expression, bbox, vis_min, vis_max, palette);
    let png_bytes = gee_post_bytes(client, project_id, access_token, &url, &body, tracker)?;
    if png_bytes.len() < 8 || &png_bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("GEE_PREVIEW_PNG_INVALID: Earth Engine 返回的预览图不是有效 PNG".to_owned());
    }
    Ok(format!(
        "data:image/png;base64,{}",
        B64_STD.encode(&png_bytes)
    ))
}

#[allow(clippy::too_many_arguments)]
fn render_png_preview_data_url(
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    expression: &Value,
    bbox: [f64; 4],
    vis_min: f64,
    vis_max: f64,
    palette: &[String],
    tracker: &mut GeeCallTracker,
) -> Option<String> {
    try_render_png_preview_data_url(
        client,
        project_id,
        access_token,
        expression,
        bbox,
        vis_min,
        vis_max,
        palette,
        tracker,
    )
    .ok()
}

// ============================================================================
// Spectral & Radar Index Specification
// ============================================================================

struct IndexSpec {
    index_name: String,
    required_bands: Vec<String>,
    vis_min: f64,
    vis_max: f64,
    palette: Vec<String>,
    unit: String,
}

fn resolve_index_spec(asset_id: &str, index_raw: &str) -> Result<IndexSpec, String> {
    let idx = index_raw.trim().to_ascii_uppercase();
    let upper_asset = asset_id.to_ascii_uppercase();
    let is_landsat_l2 = upper_asset.contains("LC08") || upper_asset.contains("LC09");
    let is_landsat5 = upper_asset.contains("LT05") || upper_asset.contains("LE07");

    let (blue, green, red, nir, swir1, swir2) = if is_landsat_l2 {
        ("SR_B2", "SR_B3", "SR_B4", "SR_B5", "SR_B6", "SR_B7")
    } else if is_landsat5 {
        ("SR_B1", "SR_B2", "SR_B3", "SR_B4", "SR_B5", "SR_B7")
    } else {
        // Default Sentinel-2 MSI band naming
        ("B2", "B3", "B4", "B8", "B11", "B12")
    };

    let veg_pal: Vec<String> = [
        "#d73027", "#fdae61", "#ffffbf", "#a6d96a", "#1a9850", "#006837",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let water_pal: Vec<String> = [
        "#f7fbff", "#deebf7", "#9ecae1", "#4292c6", "#08519c", "#08306b",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let urban_pal: Vec<String> = ["#2b83ba", "#abdda4", "#ffffbf", "#fdae61", "#d7191c"]
        .into_iter()
        .map(str::to_owned)
        .collect();

    match idx.as_str() {
        "NDVI" => Ok(IndexSpec {
            index_name: "NDVI".into(),
            required_bands: vec![nir.into(), red.into()],
            vis_min: -0.2,
            vis_max: 0.85,
            palette: veg_pal,
            unit: "NDVI".into(),
        }),
        "EVI" => Ok(IndexSpec {
            index_name: "EVI".into(),
            required_bands: vec![nir.into(), red.into(), blue.into()],
            vis_min: -0.1,
            vis_max: 0.8,
            palette: veg_pal,
            unit: "EVI".into(),
        }),
        "NDWI" => Ok(IndexSpec {
            index_name: "NDWI".into(),
            required_bands: vec![green.into(), nir.into()],
            vis_min: -0.5,
            vis_max: 0.6,
            palette: water_pal,
            unit: "NDWI".into(),
        }),
        "MNDWI" => Ok(IndexSpec {
            index_name: "MNDWI".into(),
            required_bands: vec![green.into(), swir1.into()],
            vis_min: -0.5,
            vis_max: 0.65,
            palette: water_pal,
            unit: "MNDWI".into(),
        }),
        "NDBI" => Ok(IndexSpec {
            index_name: "NDBI".into(),
            required_bands: vec![swir1.into(), nir.into()],
            vis_min: -0.4,
            vis_max: 0.5,
            palette: urban_pal,
            unit: "NDBI".into(),
        }),
        "NBR" => Ok(IndexSpec {
            index_name: "NBR".into(),
            required_bands: vec![nir.into(), swir2.into()],
            vis_min: -0.5,
            vis_max: 0.8,
            palette: veg_pal,
            unit: "NBR".into(),
        }),
        "BSI" => Ok(IndexSpec {
            index_name: "BSI".into(),
            required_bands: vec![swir1.into(), red.into(), nir.into(), blue.into()],
            vis_min: -0.4,
            vis_max: 0.5,
            palette: urban_pal,
            unit: "BSI".into(),
        }),
        "SAR_DIFF" | "VV_VH_DIFF" => Ok(IndexSpec {
            index_name: "SAR_DIFF".into(),
            required_bands: vec!["VV".into(), "VH".into()],
            vis_min: 0.0,
            vis_max: 15.0,
            palette: ["#08306b", "#4292c6", "#ffffbf", "#fdae61", "#d73027"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            unit: "dB".into(),
        }),
        _ => Err(format!(
            "UNSUPPORTED_GEE_INDEX: 不支持的内置指数 `{index_raw}`（支持 NDVI, EVI, NDWI, MNDWI, NDBI, NBR, BSI, SAR_DIFF）"
        )),
    }
}

fn build_index_node(
    eb: &mut ExprBuilder,
    asset_id: &str,
    comp_node: &str,
    spec: &IndexSpec,
) -> String {
    let b = &spec.required_bands;
    match spec.index_name.as_str() {
        "NDVI" | "NDWI" | "MNDWI" | "NDBI" | "NBR" => {
            eb.normalized_diff_calibrated(asset_id, comp_node, &b[0], &b[1], &spec.index_name)
        }
        "EVI" => {
            // 2.5 * (NIR - RED) / (NIR + 6*RED - 7.5*BLUE + 1.0)
            let nir_r = eb.select_single_band(comp_node, &b[0]);
            let red_r = eb.select_single_band(comp_node, &b[1]);
            let blu_r = eb.select_single_band(comp_node, &b[2]);
            let nir = eb.calibrate_optical_band(asset_id, &nir_r, &b[0]);
            let red = eb.calibrate_optical_band(asset_id, &red_r, &b[1]);
            let blu = eb.calibrate_optical_band(asset_id, &blu_r, &b[2]);

            let c25_v = eb.constant(json!(2.5_f64));
            let c6_v = eb.constant(json!(6.0_f64));
            let c75_v = eb.constant(json!(7.5_f64));
            let c1_v = eb.constant(json!(1.0_f64));
            let c25 = eb.call("Image.constant", &[("value", &c25_v)]);
            let c6 = eb.call("Image.constant", &[("value", &c6_v)]);
            let c75 = eb.call("Image.constant", &[("value", &c75_v)]);
            let c1 = eb.call("Image.constant", &[("value", &c1_v)]);

            let diff = eb.call("Image.subtract", &[("image1", &nir), ("image2", &red)]);
            let num = eb.call("Image.multiply", &[("image1", &c25), ("image2", &diff)]);

            let r6 = eb.call("Image.multiply", &[("image1", &c6), ("image2", &red)]);
            let b75 = eb.call("Image.multiply", &[("image1", &c75), ("image2", &blu)]);
            let d1 = eb.call("Image.add", &[("image1", &nir), ("image2", &r6)]);
            let d2 = eb.call("Image.subtract", &[("image1", &d1), ("image2", &b75)]);
            let den = eb.call("Image.add", &[("image1", &d2), ("image2", &c1)]);
            let evi = eb.call("Image.divide", &[("image1", &num), ("image2", &den)]);
            let names = eb.constant(json!(["EVI"]));
            eb.call("Image.rename", &[("input", &evi), ("names", &names)])
        }
        "BSI" => {
            // ((SWIR1 + RED) - (NIR + BLUE)) / ((SWIR1 + RED) + (NIR + BLUE))
            let sw_r = eb.select_single_band(comp_node, &b[0]);
            let rd_r = eb.select_single_band(comp_node, &b[1]);
            let nr_r = eb.select_single_band(comp_node, &b[2]);
            let bl_r = eb.select_single_band(comp_node, &b[3]);
            let sw = eb.calibrate_optical_band(asset_id, &sw_r, &b[0]);
            let rd = eb.calibrate_optical_band(asset_id, &rd_r, &b[1]);
            let nr = eb.calibrate_optical_band(asset_id, &nr_r, &b[2]);
            let bl = eb.calibrate_optical_band(asset_id, &bl_r, &b[3]);

            let left = eb.call("Image.add", &[("image1", &sw), ("image2", &rd)]);
            let right = eb.call("Image.add", &[("image1", &nr), ("image2", &bl)]);
            let num = eb.call("Image.subtract", &[("image1", &left), ("image2", &right)]);
            let den = eb.call("Image.add", &[("image1", &left), ("image2", &right)]);
            let bsi = eb.call("Image.divide", &[("image1", &num), ("image2", &den)]);
            let names = eb.constant(json!(["BSI"]));
            eb.call("Image.rename", &[("input", &bsi), ("names", &names)])
        }
        _ => {
            // SAR_DIFF: VV - VH (dB polarization difference)
            let vv = eb.select_single_band(comp_node, "VV");
            let vh = eb.select_single_band(comp_node, "VH");
            let diff = eb.call("Image.subtract", &[("image1", &vv), ("image2", &vh)]);
            let names = eb.constant(json!(["SAR_DIFF"]));
            eb.call("Image.rename", &[("input", &diff), ("names", &names)])
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_source_image_node(
    eb: &mut ExprBuilder,
    profile: Option<&GeeDatasetProfile>,
    asset_id: &str,
    geom_node: &str,
    start_date: Option<&str>,
    end_date: Option<&str>,
    max_cloud: Option<f64>,
    reducer_mode: &str,
    bands_to_keep: &[String],
) -> String {
    let asset_type = profile
        .map(|p| p.asset_type.as_str())
        .unwrap_or("image_collection");
    let cloud_prop = profile.and_then(|p| p.cloud_property.as_deref());

    if asset_type == "image" {
        let id_node = eb.constant(json!(asset_id));
        let img = eb.call("Image.load", &[("id", &id_node)]);
        if bands_to_keep.is_empty() {
            img
        } else {
            let sel = eb.constant(json!(bands_to_keep));
            eb.call("Image.select", &[("input", &img), ("bandSelectors", &sel)])
        }
    } else {
        let col = eb.filtered_collection(
            asset_id, geom_node, start_date, end_date, cloud_prop, max_cloud,
        );
        eb.composite_and_select_bands(&col, reducer_mode, bands_to_keep)
    }
}

// ============================================================================
// Artifact Persistence Registry (bridges Safe Agent Traces <-> Rich UI Overlays)
// ============================================================================

pub fn store_gee_artifact(app_data_dir: &Path, record: GeeArtifactRecord) -> String {
    let id = record.artifact_id.clone();
    if let Ok(mut guard) = ARTIFACT_CACHE.lock() {
        guard.insert(id.clone(), record.clone());
    }
    let dir = app_data_dir.join("gee_artifacts");
    if fs::create_dir_all(&dir).is_ok() {
        let path = dir.join(format!("{id}.json"));
        if let Ok(bytes) = serde_json::to_vec_pretty(&record) {
            let _ = fs::write(path, bytes);
        }
    }
    id
}

pub fn load_gee_artifact(
    app_data_dir: &Path,
    artifact_id: &str,
) -> Result<GeeArtifactRecord, String> {
    let clean_id: String = artifact_id
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if clean_id.is_empty() {
        return Err("GEE_ARTIFACT_ID_EMPTY".to_owned());
    }
    if let Ok(guard) = ARTIFACT_CACHE.lock() {
        if let Some(rec) = guard.get(&clean_id) {
            return Ok(rec.clone());
        }
    }
    let path = app_data_dir
        .join("gee_artifacts")
        .join(format!("{clean_id}.json"));
    let raw = fs::read_to_string(&path)
        .map_err(|_| format!("GEE_ARTIFACT_NOT_FOUND: 未找到编号为 {clean_id} 的分析图层"))?;
    serde_json::from_str(&raw).map_err(|e| format!("GEE_ARTIFACT_PARSE_ERR: {e}"))
}

fn new_artifact_id(seed: &str) -> String {
    let h = sha256_hex(&format!(
        "{seed}:{}:{}",
        now_epoch_secs(),
        std::process::id()
    ));
    format!("gee-art-{}", &h[..10])
}

// ============================================================================
// High-Level Operations: Verify, Pre-Flight Check, Cloud Analyze, Chunked GeoTIFF
// ============================================================================

pub fn verify_gee_account(
    app_data_dir: &Path,
    override_project_id: Option<&str>,
    override_secret: Option<&str>,
) -> Result<Value, String> {
    let client = build_gee_http_client(app_data_dir, 25)?;
    let override_mat = if let Some(sec) = override_secret.map(str::trim).filter(|s| !s.is_empty()) {
        Some(parse_credential_payload(sec, override_project_id)?)
    } else {
        None
    };

    let t0 = std::time::Instant::now();
    let (project_id, access_token) =
        resolve_gee_access_token(app_data_dir, &client, override_mat.as_ref())?;

    let mut tracker = GeeCallTracker::new();
    let mut eb = ExprBuilder::new();
    let one = eb.constant(json!(1));
    let res = compute_value(
        &client,
        &project_id,
        &access_token,
        eb.build(&one),
        &mut tracker,
    );
    let latency_ms = t0.elapsed().as_millis() as u64;
    finalize_gee_tracker(
        app_data_dir,
        Some((&client, &project_id, &access_token)),
        GeeLedgerOpKind::GeneralRequest,
        res.as_ref().is_ok_and(|v| v.as_i64() == Some(1)),
        tracker,
    );
    let res = res?;

    if res.as_i64() != Some(1) {
        return Err(format!("GEE_VERIFY_UNEXPECTED_RESULT: {res}"));
    }

    Ok(json!({
        "ok": true,
        "verified": true,
        "project_id": project_id,
        "latency_ms": latency_ms,
        "message_zh": format!("Google Earth Engine 云端计算与下载接口连通正常（项目 {}，耗时 {} ms）", project_id, latency_ms),
    }))
}

pub(crate) fn is_static_dem_or_timeless_asset(
    asset_id: &str,
    profile: Option<&GeeDatasetProfile>,
) -> bool {
    if let Some(p) = profile {
        if p.asset_type == "image"
            || p.tags
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|tok| tok.eq_ignore_ascii_case("dem") || tok.eq_ignore_ascii_case("elevation"))
        {
            return true;
        }
    }
    let upper = asset_id.trim().to_ascii_uppercase();
    upper.contains("/DEM/")
        || upper.contains("COPERNICUS/DEM")
        || upper.contains("FABDEM")
        || upper.contains("AW3D30")
        || upper.contains("NASADEM")
        || upper.contains("SRTM")
        || upper.contains("MERIT/DEM")
        || upper.contains("ESA/WORLDCOVER")
        || upper.contains("JRC/GSW1_4/GLOBALSURFACEWATER")
        || upper.contains("JRC/GHSL")
        || upper.contains("GHSL_BUILT")
        || upper.contains("GLOBAL_FOREST_CHANGE")
        || upper.contains("GEM-15")
}

/// Checks whether two WGS-84 bounding boxes `[west, south, east, north]` intersect.
pub fn bbox_intersects(a: [f64; 4], b: [f64; 4]) -> bool {
    let a_min_x = a[0].min(a[2]);
    let a_max_x = a[0].max(a[2]);
    let a_min_y = a[1].min(a[3]);
    let a_max_y = a[1].max(a[3]);
    let b_min_x = b[0].min(b[2]);
    let b_max_x = b[0].max(b[2]);
    let b_min_y = b[1].min(b[3]);
    let b_max_y = b[1].max(b[3]);
    a_min_x <= b_max_x && a_max_x >= b_min_x && a_min_y <= b_max_y && a_max_y >= b_min_y
}

fn format_gee_epoch_ms_iso(ms: i64) -> String {
    let total_secs = ms.div_euclid(1000);
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y_final = if m <= 2 { y + 1 } else { y };
    format!("{y_final:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

#[allow(clippy::too_many_arguments)]
fn build_collection_scenes_expr(
    asset_id: &str,
    bbox: [f64; 4],
    start_date: Option<&str>,
    end_date: Option<&str>,
    cloud_prop: Option<&str>,
    max_cloud: Option<f64>,
    sar_polarization: Option<&str>,
    orbit_direction: Option<&str>,
) -> Value {
    let mut eb_s = ExprBuilder::new();
    let geom_s = eb_s.rectangle(bbox);
    let col_raw = eb_s.filtered_collection(
        asset_id, &geom_s, start_date, end_date, cloud_prop, max_cloud,
    );
    let col_s = eb_s.apply_sar_filters(col_raw, asset_id, sar_polarization, orbit_direction);
    let lim_n = eb_s.constant(json!(50));
    let sort_k = eb_s.constant(json!("system:time_start"));
    let asc_n = eb_s.constant(json!(false));
    let limited = eb_s.call(
        "Collection.limit",
        &[
            ("collection", &col_s),
            ("limit", &lim_n),
            ("key", &sort_k),
            ("ascending", &asc_n),
        ],
    );
    let p_idx = eb_s.constant(json!("system:index"));
    let ids_node = eb_s.call(
        "AggregateFeatureCollection.array",
        &[("collection", &limited), ("property", &p_idx)],
    );
    let p_time = eb_s.constant(json!("system:time_start"));
    let times_node = eb_s.call(
        "AggregateFeatureCollection.array",
        &[("collection", &limited), ("property", &p_time)],
    );
    let clouds_node = cloud_prop.map(|cp| {
        let p_cp = eb_s.constant(json!(cp));
        eb_s.call(
            "AggregateFeatureCollection.array",
            &[("collection", &limited), ("property", &p_cp)],
        )
    });
    let mut dict_vals = serde_json::Map::new();
    dict_vals.insert("ids".into(), json!({ "valueReference": ids_node }));
    dict_vals.insert("times".into(), json!({ "valueReference": times_node }));
    if let Some(cn) = clouds_node {
        dict_vals.insert("clouds".into(), json!({ "valueReference": cn }));
    }
    let dict_root = eb_s.alloc(json!({ "dictionaryValue": { "values": dict_vals } }));
    eb_s.build(&dict_root)
}

#[allow(clippy::too_many_arguments)]
fn compute_collection_scenes(
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    asset_id: &str,
    bbox: [f64; 4],
    start_date: Option<&str>,
    end_date: Option<&str>,
    cloud_prop: Option<&str>,
    max_cloud: Option<f64>,
    sar_polarization: Option<&str>,
    orbit_direction: Option<&str>,
    tracker: &mut GeeCallTracker,
) -> Vec<Value> {
    let expr = build_collection_scenes_expr(
        asset_id,
        bbox,
        start_date,
        end_date,
        cloud_prop,
        max_cloud,
        sar_polarization,
        orbit_direction,
    );
    let Ok(res) = compute_value(client, project_id, access_token, expr, tracker) else {
        return Vec::new();
    };
    let Some(ids) = res.get("ids").and_then(Value::as_array) else {
        return Vec::new();
    };
    let times = res.get("times").and_then(Value::as_array);
    let clouds = res.get("clouds").and_then(Value::as_array);
    let trimmed_base = asset_id.trim_end_matches('/');

    let mut out = Vec::with_capacity(ids.len().min(50));
    for (idx, id_val) in ids.iter().take(50).enumerate() {
        let Some(sys_idx) = id_val.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let image_id = if sys_idx.contains('/') {
            sys_idx.to_owned()
        } else {
            format!("{trimmed_base}/{sys_idx}")
        };
        let acq_date = times
            .and_then(|arr| arr.get(idx))
            .and_then(Value::as_f64)
            .filter(|t| t.is_finite())
            .map(|ms| format_gee_epoch_ms_iso(ms.round() as i64));
        let cloud_cover = clouds.and_then(|arr| arr.get(idx)).and_then(Value::as_f64);
        out.push(json!({
            "id": sys_idx,
            "image_id": image_id,
            "acquisition_date": acq_date,
            "cloud_cover": cloud_cover,
        }));
    }
    out
}

pub fn check_gee_availability(
    app_data_dir: &Path,
    asset_id: &str,
    bbox: [f64; 4],
    start_date: Option<&str>,
    end_date: Option<&str>,
    max_cloud: Option<f64>,
    requested_scale_m: Option<f64>,
) -> Result<Value, String> {
    check_gee_availability_with_filters(
        app_data_dir,
        asset_id,
        bbox,
        start_date,
        end_date,
        max_cloud,
        requested_scale_m,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn check_gee_availability_with_filters(
    app_data_dir: &Path,
    asset_id: &str,
    bbox: [f64; 4],
    start_date: Option<&str>,
    end_date: Option<&str>,
    max_cloud: Option<f64>,
    requested_scale_m: Option<f64>,
    sar_polarization: Option<&str>,
    orbit_direction: Option<&str>,
) -> Result<Value, String> {
    let trimmed_id = asset_id.trim();
    if trimmed_id.is_empty() {
        return Err("GEE_ASSET_ID_EMPTY: 数据集 ID 不能为空".to_owned());
    }
    let profile = lookup_dataset_profile(trimmed_id);
    let native_scale = requested_scale_m
        .or_else(|| profile.as_ref().map(|p| p.native_scale_m))
        .unwrap_or(30.0);
    let bands_est = profile
        .as_ref()
        .map(|p| p.default_bands.len().max(1))
        .unwrap_or(1);

    let grid_plan: ChunkGridPlan = plan_chunk_grid(bbox, native_scale, bands_est, false, false)?;
    let quota_note_zh = if grid_plan.total_chunks <= AUTO_CHUNK_CONFIRM_THRESHOLD {
        format!(
            "当前研究区按原生 {} 米分辨率共 {} 个分块（<= {} 块），可直接按原生全分辨率下载。",
            grid_plan.requested_scale_m, grid_plan.total_chunks, AUTO_CHUNK_CONFIRM_THRESHOLD
        )
    } else if grid_plan.total_chunks <= HARD_MAX_CHUNKS {
        format!(
            "当前研究区按原生 {} 米分辨率共需 {} 个分块（处于 10–{} 块区间），下载时需确认按原生全分辨率拼接或改用推荐 {} 米轻量分辨率。",
            grid_plan.requested_scale_m,
            grid_plan.total_chunks,
            HARD_MAX_CHUNKS,
            grid_plan.suggested_preview_scale_m
        )
    } else {
        format!(
            "当前研究区按原生 {} 米分辨率共需 {} 个分块（已超过单次在线拼接硬上限 {} 块），不支持静默降采样，请缩小研究区范围（AOI）或改用推荐 {} 米轻量预览分辨率（use_preview_scale: true）。",
            grid_plan.requested_scale_m,
            grid_plan.total_chunks,
            HARD_MAX_CHUNKS,
            grid_plan.suggested_preview_scale_m
        )
    };

    let asset_type = profile
        .as_ref()
        .map(|p| p.asset_type.as_str())
        .unwrap_or("image_collection");

    let default_bands = profile
        .as_ref()
        .map(|p| p.default_bands.clone())
        .unwrap_or_default();

    if let Some(bounds) = known_asset_spatial_bounds(trimmed_id) {
        if !bbox_intersects(bbox, bounds) {
            return Ok(json!({
                "ok": true,
                "dataset": "gee_check_availability",
                "asset_id": trimmed_id,
                "title_zh": profile.as_ref().map(|p| p.title_zh.as_str()).unwrap_or(trimmed_id),
                "asset_type": asset_type,
                "start_date": start_date,
                "end_date": end_date,
                "max_cloud": max_cloud,
                "bands": default_bands,
                "total_scenes": 0,
                "cloud_filtered_scenes": 0,
                "scenes": [],
                "native_scale_m": native_scale,
                "grid_plan": grid_plan,
                "quota_note_zh": quota_note_zh,
                "empty_reason_zh": "当前研究区不在该区域性数据集的空间覆盖范围内（0 景）",
            }));
        }
    }

    let client = build_gee_http_client(app_data_dir, 30)?;
    let (project_id, access_token) = resolve_gee_access_token(app_data_dir, &client, None)?;

    if asset_type == "image" {
        return Ok(json!({
            "ok": true,
            "dataset": "gee_check_availability",
            "asset_id": trimmed_id,
            "title_zh": profile.as_ref().map(|p| p.title_zh.as_str()).unwrap_or(trimmed_id),
            "asset_type": "image",
            "start_date": profile.as_ref().map(|p| p.start_date.as_str()).unwrap_or(""),
            "end_date": profile.as_ref().map(|p| p.end_date.as_str()).unwrap_or(""),
            "bands": default_bands,
            "total_scenes": 1,
            "cloud_filtered_scenes": 1,
            "scenes": [{
                "id": trimmed_id,
                "image_id": trimmed_id,
                "acquisition_date": profile.as_ref().map(|p| p.start_date.as_str()),
                "cloud_cover": Value::Null,
            }],
            "native_scale_m": native_scale,
            "grid_plan": grid_plan,
            "quota_note_zh": quota_note_zh,
        }));
    }

    let cloud_prop = profile.as_ref().and_then(|p| p.cloud_property.as_deref());
    let is_timeless = is_static_dem_or_timeless_asset(trimmed_id, profile.as_ref());
    let eff_start = if is_timeless { None } else { start_date };
    let eff_end = if is_timeless { None } else { end_date };
    let eff_cloud = if is_timeless { None } else { max_cloud };
    let mut tracker = GeeCallTracker::new();
    let outcome: Result<(u64, u64, Vec<Value>), String> = (|| {
        // Compute total scenes (no cloud filter, but with AOI & SAR filters if applicable) and cloud-filtered scenes
        let mut eb_total = ExprBuilder::new();
        let geom_t = eb_total.rectangle(bbox);
        let col_t_raw =
            eb_total.filtered_collection(trimmed_id, &geom_t, eff_start, eff_end, None, None);
        let col_t =
            eb_total.apply_sar_filters(col_t_raw, trimmed_id, sar_polarization, orbit_direction);
        let sz_t = eb_total.call("Collection.size", &[("collection", &col_t)]);
        let total_scenes = compute_value(
            &client,
            &project_id,
            &access_token,
            eb_total.build(&sz_t),
            &mut tracker,
        )?
        .as_u64()
        .unwrap_or(0);

        let cloud_filtered_scenes = if !is_timeless && cloud_prop.is_some() && eff_cloud.is_some() {
            let mut eb_c = ExprBuilder::new();
            let geom_c = eb_c.rectangle(bbox);
            let col_c_raw = eb_c.filtered_collection(
                trimmed_id, &geom_c, eff_start, eff_end, cloud_prop, eff_cloud,
            );
            let col_c =
                eb_c.apply_sar_filters(col_c_raw, trimmed_id, sar_polarization, orbit_direction);
            let sz_c = eb_c.call("Collection.size", &[("collection", &col_c)]);
            compute_value(
                &client,
                &project_id,
                &access_token,
                eb_c.build(&sz_c),
                &mut tracker,
            )?
            .as_u64()
            .unwrap_or(0)
        } else {
            total_scenes
        };
        let scenes = if cloud_filtered_scenes > 0 {
            compute_collection_scenes(
                &client,
                &project_id,
                &access_token,
                trimmed_id,
                bbox,
                eff_start,
                eff_end,
                cloud_prop,
                eff_cloud,
                sar_polarization,
                orbit_direction,
                &mut tracker,
            )
        } else {
            Vec::new()
        };
        Ok((total_scenes, cloud_filtered_scenes, scenes))
    })();
    finalize_gee_tracker(
        app_data_dir,
        Some((&client, &project_id, &access_token)),
        GeeLedgerOpKind::GeneralRequest,
        outcome.is_ok(),
        tracker,
    );
    let (total_scenes, cloud_filtered_scenes, scenes) = outcome?;

    let empty_reason_zh = if cloud_filtered_scenes == 0 {
        if trimmed_id.to_ascii_lowercase().contains("gem-15") {
            Some("当前研究区不在 Google GEM-15 官方覆盖范围内（v1 不含中国大陆、俄罗斯、蒙古、朝鲜及南北极，0 景）".to_owned())
        } else if total_scenes > 0 {
            Some(format!(
                "当前时间窗内共有 {total_scenes} 景影像，但满足当前云量上限（<= {}%）的影像为 0 景，可放宽云量上限或扩大时间范围",
                max_cloud.unwrap_or(100.0)
            ))
        } else {
            Some("当前研究区与筛选条件下暂无可用 GEE 影像（0 景）".to_owned())
        }
    } else {
        None
    };

    Ok(json!({
        "ok": true,
        "dataset": "gee_check_availability",
        "asset_id": trimmed_id,
        "title_zh": profile.as_ref().map(|p| p.title_zh.as_str()).unwrap_or(trimmed_id),
        "asset_type": "image_collection",
        "start_date": start_date,
        "end_date": end_date,
        "max_cloud": max_cloud,
        "bands": default_bands,
        "total_scenes": total_scenes,
        "cloud_filtered_scenes": cloud_filtered_scenes,
        "scenes": scenes,
        "native_scale_m": native_scale,
        "grid_plan": grid_plan,
        "quota_note_zh": quota_note_zh,
        "empty_reason_zh": empty_reason_zh,
    }))
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct GeeCloudAnalyzeParams {
    pub asset_id: Option<String>,
    /// `"composite"` | `"index"` | `"zonal_stats"` | `"change_detect"` | `"time_series"`
    pub operation: Option<String>,
    pub index_name: Option<String>,
    pub reducer: Option<String>,
    pub bands: Option<Vec<String>>,
    pub bbox: Option<[f64; 4]>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub before_start_date: Option<String>,
    pub before_end_date: Option<String>,
    pub after_start_date: Option<String>,
    pub after_end_date: Option<String>,
    pub max_cloud: Option<f64>,
    pub scale_m: Option<f64>,
    /// Optional threshold for area calculation in `km²` (e.g. `NDVI > 0.3`, `MNDWI > 0.0`)
    pub threshold: Option<f64>,
    /// `"gt"` (default) or `"lt"`
    pub threshold_op: Option<String>,
    /// Number of stepped intervals for `time_series` (2..=24)
    pub step_count: Option<usize>,
    /// Step size in days for `time_series` (aligned with Agent tool schema)
    pub step_days: Option<u32>,
}

fn non_empty_trimmed(opt: Option<&str>) -> Option<&str> {
    opt.map(str::trim).filter(|s| !s.is_empty())
}

pub(crate) fn resolve_change_detect_dates(
    params: &GeeCloudAnalyzeParams,
) -> (String, String, String, String) {
    let before_s = non_empty_trimmed(params.before_start_date.as_deref());
    let before_e = non_empty_trimmed(params.before_end_date.as_deref());
    let after_s = non_empty_trimmed(params.after_start_date.as_deref());
    let after_e = non_empty_trimmed(params.after_end_date.as_deref());
    let main_s = non_empty_trimmed(params.start_date.as_deref());
    let main_e = non_empty_trimmed(params.end_date.as_deref());

    if before_s.is_none() && (after_s.is_some() || after_e.is_some()) {
        let b_start = main_s.unwrap_or("2024-06-01").to_owned();
        let b_end = before_e.or(main_e).unwrap_or("2024-09-01").to_owned();
        let a_start = after_s.unwrap_or("2025-06-01").to_owned();
        let a_end = after_e.unwrap_or("2025-09-01").to_owned();
        (b_start, b_end, a_start, a_end)
    } else {
        let b_start = before_s.unwrap_or("2024-06-01").to_owned();
        let b_end = before_e.unwrap_or("2024-09-01").to_owned();
        let a_start = after_s.or(main_s).unwrap_or("2025-06-01").to_owned();
        let a_end = after_e.or(main_e).unwrap_or("2025-09-01").to_owned();
        (b_start, b_end, a_start, a_end)
    }
}

pub(crate) fn resolve_time_series_steps(
    s_dt: chrono::NaiveDate,
    e_dt: chrono::NaiveDate,
    step_days: Option<u32>,
    step_count: Option<usize>,
) -> usize {
    if let Some(sd) = step_days.filter(|&d| d > 0) {
        let span_days = (e_dt - s_dt).num_days().max(1) as f64;
        ((span_days / (sd as f64)).ceil() as usize).clamp(2, 24)
    } else {
        step_count.unwrap_or(6).clamp(2, 24)
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_non_empty_collection(
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    asset_id: &str,
    bbox: [f64; 4],
    start_date: Option<&str>,
    end_date: Option<&str>,
    cloud_prop: Option<&str>,
    max_cloud: Option<f64>,
    tracker: &mut GeeCallTracker,
) -> Result<u64, String> {
    let mut eb = ExprBuilder::new();
    let geom = eb.rectangle(bbox);
    let col = eb.filtered_collection(asset_id, &geom, start_date, end_date, cloud_prop, max_cloud);
    let sz = eb.call("Collection.size", &[("collection", &col)]);
    let val = compute_value(client, project_id, access_token, eb.build(&sz), tracker)?;
    let count = val.as_u64().ok_or_else(|| {
        format!("GEE_INVALID_COLLECTION_SIZE: Earth Engine 未返回有效景数 ({val})")
    })?;
    if count == 0 {
        return Err(
            "GEE_EMPTY_COLLECTION: 当前研究区与时间窗内未检索到可用影像（0 景），请放宽日期或云量阈值。"
                .to_owned(),
        );
    }
    Ok(count)
}

#[allow(clippy::too_many_arguments)]
fn compute_single_band_stats_and_area(
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    eb_base: &ExprBuilder,
    img_node: &str,
    band_name: &str,
    geom_node: &str,
    scale_m: f64,
    threshold: Option<(f64, &str)>,
    tracker: &mut GeeCallTracker,
) -> Result<Value, String> {
    let mut eb = eb_base.clone();
    let r_mean = eb.call("Reducer.mean", &[]);
    let r_mm = eb.call("Reducer.minMax", &[]);
    let shared_true = eb.constant(json!(true));
    let r_comb = eb.call(
        "Reducer.combine",
        &[
            ("reducer1", &r_mean),
            ("reducer2", &r_mm),
            ("sharedInputs", &shared_true),
        ],
    );
    let sc_node = eb.constant(json!(scale_m));
    let best_effort = eb.constant(json!(true));
    let max_px = eb.constant(json!(50_000_000_i64));
    let stats_node = eb.call(
        "Image.reduceRegion",
        &[
            ("image", img_node),
            ("reducer", &r_comb),
            ("geometry", geom_node),
            ("scale", &sc_node),
            ("bestEffort", &best_effort),
            ("maxPixels", &max_px),
        ],
    );
    let raw_stats = compute_value(
        client,
        project_id,
        access_token,
        eb.build(&stats_node),
        tracker,
    )?;

    let mean_key = format!("{band_name}_mean");
    let min_key = format!("{band_name}_min");
    let max_key = format!("{band_name}_max");
    let mean_v = raw_stats.get(&mean_key).and_then(Value::as_f64);
    let min_v = raw_stats.get(&min_key).and_then(Value::as_f64);
    let max_v = raw_stats.get(&max_key).and_then(Value::as_f64);

    let mut area_km2_val: Option<f64> = None;
    if let Some((thresh, op)) = threshold {
        let mut eb_a = eb_base.clone();
        let th_const = eb_a.constant(json!(thresh));
        let th_img = eb_a.call("Image.constant", &[("value", &th_const)]);
        let cmp_fn = if op.eq_ignore_ascii_case("lt") {
            "Image.lt"
        } else {
            "Image.gt"
        };
        let mask = eb_a.call(cmp_fn, &[("image1", img_node), ("image2", &th_img)]);
        let px_area = eb_a.call("Image.pixelArea", &[]);
        let masked_area = eb_a.call("Image.updateMask", &[("image", &px_area), ("mask", &mask)]);
        let r_sum = eb_a.call("Reducer.sum", &[]);
        let sc_a = eb_a.constant(json!(scale_m));
        let be_a = eb_a.constant(json!(true));
        let mp_a = eb_a.constant(json!(50_000_000_i64));
        let sum_node = eb_a.call(
            "Image.reduceRegion",
            &[
                ("image", &masked_area),
                ("reducer", &r_sum),
                ("geometry", geom_node),
                ("scale", &sc_a),
                ("bestEffort", &be_a),
                ("maxPixels", &mp_a),
            ],
        );
        let area_res = compute_value(
            client,
            project_id,
            access_token,
            eb_a.build(&sum_node),
            tracker,
        )?;
        if let Some(area_m2) = area_res.get("area").and_then(Value::as_f64) {
            area_km2_val = Some((area_m2 / 1_000_000.0 * 1000.0).round() / 1000.0);
        }
    }

    Ok(json!({
        "band": band_name,
        "mean": mean_v.map(|v| (v * 10000.0).round() / 10000.0),
        "min": min_v.map(|v| (v * 10000.0).round() / 10000.0),
        "max": max_v.map(|v| (v * 10000.0).round() / 10000.0),
        "threshold_area_km2": area_km2_val,
    }))
}

fn default_single_band_palette() -> Vec<String> {
    vec![
        "#0c2c84".into(),
        "#41b6c4".into(),
        "#ffffcc".into(),
        "#fd8d3c".into(),
        "#800026".into(),
    ]
}

/// Ensure composite outputs always yield either 1 band (with palette) or 3 bands (RGB)
/// so EE `/maps` and `image:computePixels` PNG never reject 2-band (e.g. S1 VV/VH) or >3-band assets.
fn prepare_composite_vis_node(
    eb: &mut ExprBuilder,
    comp_node: &str,
    selected_bands: &[String],
) -> (String, Option<String>, Vec<String>) {
    match selected_bands.len() {
        0 => {
            let zero = eb.constant(json!([0]));
            let renamed = eb.constant(json!(["BAND0"]));
            let single = eb.call(
                "Image.select",
                &[
                    ("input", comp_node),
                    ("bandSelectors", &zero),
                    ("newNames", &renamed),
                ],
            );
            (
                single,
                Some("BAND0".to_owned()),
                default_single_band_palette(),
            )
        }
        1 => (
            comp_node.to_owned(),
            Some(selected_bands[0].clone()),
            default_single_band_palette(),
        ),
        2 => {
            let sel = eb.constant(json!([
                selected_bands[0],
                selected_bands[1],
                selected_bands[0]
            ]));
            let rgb_names = eb.constant(json!(["vis_r", "vis_g", "vis_b"]));
            let rgb_node = eb.call(
                "Image.select",
                &[
                    ("input", comp_node),
                    ("bandSelectors", &sel),
                    ("newNames", &rgb_names),
                ],
            );
            (rgb_node, Some("vis_r".to_owned()), Vec::new())
        }
        3 => (
            comp_node.to_owned(),
            Some(selected_bands[0].clone()),
            Vec::new(),
        ),
        _ => {
            let top3 = eb.constant(json!(&selected_bands[..3]));
            let rgb_node = eb.call(
                "Image.select",
                &[("input", comp_node), ("bandSelectors", &top3)],
            );
            (rgb_node, Some(selected_bands[0].clone()), Vec::new())
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct GeeExportPreflightParams {
    pub asset_id: Option<String>,
    pub bbox: Option<[f64; 4]>,
    pub scale_m: Option<f64>,
    pub bands: Option<Vec<String>>,
    pub index_name: Option<String>,
    pub algorithm_kind: Option<String>,
    pub destination: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub max_cloud: Option<f64>,
    pub reducer: Option<String>,
    pub qa_mask: Option<String>,
    pub target_variable: Option<String>,
    pub crs: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn build_data_layer_record(
    asset_id: &str,
    profile: Option<&GeeDatasetProfile>,
    target_variable: Option<&str>,
    start_date: Option<&str>,
    end_date: Option<&str>,
    temporal_reducer: Option<&str>,
    bbox: Option<[f64; 4]>,
    spatial_extent_source: &str,
    analysis_scale_m: Option<f64>,
    crs: Option<&str>,
    bands: &[String],
    qa_mask_override: Option<&str>,
    output_target: &str,
    unit_override: Option<&str>,
) -> Value {
    let clean_id = asset_id.trim();
    let dataset_source = profile.map(|p| p.source.as_str()).unwrap_or_else(|| {
        if clean_id.is_empty() {
            "unknown"
        } else if clean_id.starts_with("projects/") {
            "community"
        } else {
            "official"
        }
    });
    let asset_type = profile
        .map(|p| p.asset_type.as_str())
        .unwrap_or("image_collection");
    let is_timeless = is_static_dem_or_timeless_asset(clean_id, profile);
    let (deprecated, replacement_id) = dataset_deprecation_info(clean_id);
    let (scale_note, qa_default_note) = curated_science_guidance(clean_id);
    let qa_note = qa_mask_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(qa_default_note);
    let native_scale_m = profile.map(|p| p.native_scale_m);
    let eff_bands: Vec<String> = if !bands.is_empty() {
        bands.to_vec()
    } else {
        profile.map(|p| p.default_bands.clone()).unwrap_or_default()
    };
    let verification_status = if deprecated {
        "deprecated"
    } else if profile.is_some() {
        if dataset_source == "community" {
            "community_unverified"
        } else {
            "catalog_curated"
        }
    } else if clean_id.is_empty() {
        "unknown"
    } else {
        "catalog_unverified"
    };
    let target_var = target_variable
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(if clean_id.is_empty() {
            "未指定"
        } else {
            clean_id
        });
    let unit_val = unit_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("参见物理定标说明");

    json!({
        "target_variable": target_var,
        "dataset_source": dataset_source,
        "dataset_id": clean_id,
        "asset_type": asset_type,
        "deprecated": deprecated,
        "replacement_id": replacement_id,
        "time_range": {
            "start_date": start_date.map(str::trim).filter(|s| !s.is_empty()),
            "end_date": end_date.map(str::trim).filter(|s| !s.is_empty()),
            "is_timeless": is_timeless,
        },
        "temporal_reducer": temporal_reducer.map(str::trim).filter(|s| !s.is_empty()),
        "spatial_extent_source": spatial_extent_source,
        "bbox": bbox,
        "native_scale_m": native_scale_m,
        "analysis_scale_m": analysis_scale_m,
        "crs": crs.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("EPSG:4326"),
        "bands": eff_bands,
        "scale_offset_note": scale_note,
        "unit": unit_val,
        "qa_mask_method": qa_note,
        "output_target": output_target,
        "verification_status": verification_status,
    })
}

fn classify_algorithm_tile_safety(algorithm_kind: &str) -> (&'static str, &'static str) {
    let norm = algorithm_kind.trim().to_ascii_lowercase();
    match norm.as_str() {
        "pca_rsei" | "pca" | "rsei" => (
            "tile_unsafe",
            "PCA 或 RSEI 等需全局主成分归一化与方差统计的算法不可独立分块拼接，否则瓦片边界会产生数值跳变。",
        ),
        "global_zonal_stats" | "whole_aoi_stats" => (
            "tile_unsafe",
            "需整区全局统计量的指标不可拆分独立分块导出后直接拼接，应改用服务端全区归约或批量任务导出。",
        ),
        "segmentation" | "connected_components" | "watershed" | "region_growing" => (
            "tile_unsafe",
            "连通域标记、图像分割或分水岭类算法跨块边界易被截断，不适合无重叠独立分块。",
        ),
        "ml_training" | "classification_training" => (
            "tile_unsafe",
            "分类样本采样与训练过程依赖全区样本分布一致性，不宜按网格拆块分别训练。",
        ),
        _ => (
            "tile_safe",
            "逐像元光谱指数、时间合成或固定模型推理支持独立网格分块。",
        ),
    }
}

pub fn plan_gee_export_preflight(_app_data_dir: &Path, params: &GeeExportPreflightParams) -> Value {
    let clean_asset_id = params
        .asset_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    let profile = if clean_asset_id.is_empty() {
        None
    } else {
        lookup_dataset_profile(clean_asset_id)
    };
    let is_timeless = !clean_asset_id.is_empty()
        && is_static_dem_or_timeless_asset(clean_asset_id, profile.as_ref());

    let mut missing_parameters: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if clean_asset_id.is_empty() {
        missing_parameters.push("asset_id".to_owned());
        warnings.push("缺少 GEE 数据集资产 ID（asset_id）。".to_owned());
    } else {
        let (dep, repl) = dataset_deprecation_info(clean_asset_id);
        if dep {
            let suffix = repl.map(|r| format!("，建议改用 {r}")).unwrap_or_default();
            warnings.push(format!("所选数据集 {clean_asset_id} 已标记退役{suffix}。"));
        }
    }

    let valid_bbox = params.bbox.and_then(|b| {
        let min_lon = b[0].min(b[2]);
        let max_lon = b[0].max(b[2]);
        let min_lat = b[1].min(b[3]);
        let max_lat = b[1].max(b[3]);
        if min_lon.is_finite()
            && max_lon.is_finite()
            && min_lat.is_finite()
            && max_lat.is_finite()
            && min_lon >= -180.0
            && max_lon <= 180.0
            && min_lat >= -90.0
            && max_lat <= 90.0
            && (max_lon - min_lon) >= 1e-6
            && (max_lat - min_lat) >= 1e-6
        {
            Some([min_lon, min_lat, max_lon, max_lat])
        } else {
            None
        }
    });
    if valid_bbox.is_none() {
        missing_parameters.push("bbox".to_owned());
        if params.bbox.is_some() {
            warnings.push("传入的研究区 bbox 经纬度范围无效，无法估算像元与分块规模。".to_owned());
        } else {
            warnings.push("未提供研究区范围（bbox）且当前工作区未绑定 AOI。".to_owned());
        }
    }

    let start_clean = params
        .start_date
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let end_clean = params
        .end_date
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if !clean_asset_id.is_empty() && !is_timeless {
        if start_clean.is_none() {
            missing_parameters.push("start_date".to_owned());
        }
        if end_clean.is_none() {
            missing_parameters.push("end_date".to_owned());
        }
        if start_clean.is_none() || end_clean.is_none() {
            warnings.push("时序影像集合建议明确 start_date 与 end_date 时间窗口。".to_owned());
        }
    }

    let invalid_scale = params.scale_m.is_some_and(|s| !s.is_finite() || s <= 0.0);
    if invalid_scale {
        missing_parameters.push("scale_m".to_owned());
        warnings.push("请求分辨率 scale_m 必须为大于 0 的有限数值（单位：米）。".to_owned());
    }
    let eff_scale = params
        .scale_m
        .filter(|s| s.is_finite() && *s > 0.0)
        .or_else(|| profile.as_ref().map(|p| p.native_scale_m))
        .unwrap_or(30.0)
        .clamp(1.0, 50_000.0);

    let band_list: Vec<String> = if let Some(idx) = params
        .index_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        vec![idx.to_owned()]
    } else if let Some(b) = params.bands.as_ref().filter(|v| !v.is_empty()) {
        b.clone()
    } else {
        profile
            .as_ref()
            .map(|p| p.default_bands.clone())
            .unwrap_or_default()
    };
    let bands_est = band_list.len().clamp(1, 16);

    let alg_kind = params
        .algorithm_kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if params
                .index_name
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            {
                "per_pixel_index"
            } else {
                "composite"
            }
        });
    let (tile_safety, tile_safety_reason) = classify_algorithm_tile_safety(alg_kind);
    if tile_safety == "tile_unsafe" {
        warnings.push(tile_safety_reason.to_owned());
    }

    let dest_norm = params
        .destination
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("local")
        .to_ascii_lowercase();

    let (
        raw_width_px,
        raw_height_px,
        raw_total_pixels,
        chunks_x,
        chunks_y,
        raw_total_chunks,
        estimated_mb,
        grid_plan_val,
    ) = if let Some(b) = valid_bbox {
        let mid_lat_rad = ((b[1] + b[3]) * 0.5).to_radians();
        let meters_per_deg_lat = 111_320.0_f64;
        let meters_per_deg_lon = (111_320.0_f64 * mid_lat_rad.cos().abs().max(0.08)).max(1000.0);
        let ground_width_m = (b[2] - b[0]) * meters_per_deg_lon;
        let ground_height_m = (b[3] - b[1]) * meters_per_deg_lat;
        let rw = ((ground_width_m / eff_scale).ceil() as u64).max(4);
        let rh = ((ground_height_m / eff_scale).ceil() as u64).max(4);
        let cx = rw.div_ceil(CHUNK_TILE_MAX_PX as u64).max(1);
        let cy = rh.div_ceil(CHUNK_TILE_MAX_PX as u64).max(1);
        let rt = cx.saturating_mul(cy);
        let rpix = rw.saturating_mul(rh);
        let est_mb = ((rpix as f64) * (bands_est as f64) * 4.0) / (1024.0 * 1024.0);
        let gp = plan_chunk_grid(b, eff_scale, bands_est, false, false).ok();
        (
            rw,
            rh,
            rpix,
            cx,
            cy,
            rt,
            (est_mb * 100.0).round() / 100.0,
            gp.and_then(|g| serde_json::to_value(g).ok()),
        )
    } else {
        (0, 0, 0, 0, 0, 0, 0.0, None)
    };

    let explicit_external_dest = matches!(
        dest_norm.as_str(),
        "drive" | "google_drive" | "cloud_storage" | "gcs" | "ee_asset" | "asset"
    );

    let ready = missing_parameters.is_empty();
    let recommended_route = if !ready {
        "needs_parameters"
    } else if explicit_external_dest
        || raw_total_chunks > HARD_MAX_CHUNKS as u64
        || (tile_safety == "tile_unsafe" && raw_total_chunks > 1)
    {
        "external_batch_export"
    } else if raw_total_chunks > AUTO_CHUNK_CONFIRM_THRESHOLD as u64 {
        "local_chunked_geotiff_confirm"
    } else {
        "local_chunked_geotiff_auto"
    };

    let quota_confirm_required = recommended_route == "local_chunked_geotiff_confirm";
    if recommended_route == "external_batch_export" && raw_total_chunks > HARD_MAX_CHUNKS as u64 {
        warnings.push(format!(
            "当前研究区按 {eff_scale} 米分辨率共需 {raw_total_chunks} 个分块（超过本机单次直拉拼接上限 {HARD_MAX_CHUNKS} 块），系统不会静默降采样；建议改用外部批量导出（Drive 或云存储）或缩小研究区。"
        ));
    }

    let message_zh = match recommended_route {
        "needs_parameters" => format!(
            "导出预演尚未就绪，缺少参数：{}。",
            missing_parameters.join("、")
        ),
        "external_batch_export" => format!(
            "预演完成（{}×{} 像元，{} 分块，约 {:.1} MB）：建议走外部批量导出通道（{}）。",
            raw_width_px, raw_height_px, raw_total_chunks, estimated_mb, tile_safety
        ),
        "local_chunked_geotiff_confirm" => format!(
            "预演完成（{}×{} 像元，{} 分块，约 {:.1} MB）：可走本机分块拼接下载（需确认原生全分辨率或预览分辨率）。",
            raw_width_px, raw_height_px, raw_total_chunks, estimated_mb
        ),
        _ => format!(
            "预演完成（{}×{} 像元，{} 分块，约 {:.1} MB）：满足自动本机全分辨率分块下载条件。",
            raw_width_px, raw_height_px, raw_total_chunks, estimated_mb
        ),
    };

    let data_layer_record = build_data_layer_record(
        clean_asset_id,
        profile.as_ref(),
        params
            .target_variable
            .as_deref()
            .or(params.index_name.as_deref()),
        start_clean,
        end_clean,
        params.reducer.as_deref(),
        valid_bbox,
        if valid_bbox.is_some() {
            "aoi_bbox"
        } else {
            "missing"
        },
        Some(eff_scale),
        params.crs.as_deref(),
        &band_list,
        params.qa_mask.as_deref(),
        recommended_route,
        params.index_name.as_deref(),
    );

    json!({
        "ok": true,
        "dataset": "gee_export_preflight",
        "ready": ready,
        "asset_id": clean_asset_id,
        "scale_m": eff_scale,
        "algorithm_kind": alg_kind,
        "tile_safety": tile_safety,
        "tile_safety_reason": tile_safety_reason,
        "destination": dest_norm,
        "recommended_route": recommended_route,
        "quota_confirm_required": quota_confirm_required,
        "raw_width_px": raw_width_px,
        "raw_height_px": raw_height_px,
        "raw_total_pixels": raw_total_pixels,
        "chunks_x": chunks_x,
        "chunks_y": chunks_y,
        "raw_total_chunks": raw_total_chunks,
        "estimated_mb": estimated_mb,
        "grid_plan": grid_plan_val,
        "missing_parameters": missing_parameters,
        "warnings": warnings,
        "message_zh": message_zh,
        "data_layer_record": data_layer_record,
    })
}

pub fn execute_gee_cloud_analyze(
    app_data_dir: &Path,
    params: &GeeCloudAnalyzeParams,
) -> Result<Value, String> {
    let asset_id = params
        .asset_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("COPERNICUS/S2_SR_HARMONIZED");
    let profile = lookup_dataset_profile(asset_id);
    let bbox = params.bbox.unwrap_or([117.20, 31.78, 117.34, 31.89]);
    let op = params
        .operation
        .as_deref()
        .unwrap_or("index")
        .trim()
        .to_ascii_lowercase();
    let reducer_mode = params.reducer.as_deref().unwrap_or("median");
    let max_cloud = Some(params.max_cloud.unwrap_or(30.0));
    let scale_m = params
        .scale_m
        .or_else(|| profile.as_ref().map(|p| p.native_scale_m))
        .unwrap_or(30.0)
        .clamp(10.0, 10_000.0);

    let client = build_gee_http_client(app_data_dir, 60)?;
    let (project_id, access_token) = resolve_gee_access_token(app_data_dir, &client, None)?;
    let is_collection = profile
        .as_ref()
        .map(|p| p.asset_type.as_str())
        .unwrap_or("image_collection")
        != "image";
    let cloud_prop = profile.as_ref().and_then(|p| p.cloud_property.as_deref());

    let mut tracker = GeeCallTracker::new();
    let outcome: Result<Value, String> = (|| {
        // 1. Two-Period Change Detection (with Before + After + Diff Swipe Layers!)
        if op == "change_detect" || op == "change" {
            let idx_name = params.index_name.as_deref().unwrap_or("NDVI");
            let spec = resolve_index_spec(asset_id, idx_name)?;
            let (b_start_owned, b_end_owned, a_start_owned, a_end_owned) =
                resolve_change_detect_dates(params);
            let b_start = b_start_owned.as_str();
            let b_end = b_end_owned.as_str();
            let a_start = a_start_owned.as_str();
            let a_end = a_end_owned.as_str();

            if is_collection {
                verify_non_empty_collection(
                    &client,
                    &project_id,
                    &access_token,
                    asset_id,
                    bbox,
                    Some(b_start),
                    Some(b_end),
                    cloud_prop,
                    max_cloud,
                    &mut tracker,
                )?;
                verify_non_empty_collection(
                    &client,
                    &project_id,
                    &access_token,
                    asset_id,
                    bbox,
                    Some(a_start),
                    Some(a_end),
                    cloud_prop,
                    max_cloud,
                    &mut tracker,
                )?;
            }

            let mut eb = ExprBuilder::new();
            let geom = eb.rectangle(bbox);
            let comp_before = build_source_image_node(
                &mut eb,
                profile.as_ref(),
                asset_id,
                &geom,
                Some(b_start),
                Some(b_end),
                max_cloud,
                reducer_mode,
                &spec.required_bands,
            );
            let comp_after = build_source_image_node(
                &mut eb,
                profile.as_ref(),
                asset_id,
                &geom,
                Some(a_start),
                Some(a_end),
                max_cloud,
                reducer_mode,
                &spec.required_bands,
            );
            let idx_before = build_index_node(&mut eb, asset_id, &comp_before, &spec);
            let idx_after = build_index_node(&mut eb, asset_id, &comp_after, &spec);
            let diff_raw = eb.call(
                "Image.subtract",
                &[("image1", &idx_after), ("image2", &idx_before)],
            );
            let diff_names = eb.constant(json!(["DIFF"]));
            let idx_diff = eb.call(
                "Image.rename",
                &[("input", &diff_raw), ("names", &diff_names)],
            );

            let clipped_before =
                eb.call("Image.clip", &[("input", &idx_before), ("geometry", &geom)]);
            let clipped_after =
                eb.call("Image.clip", &[("input", &idx_after), ("geometry", &geom)]);
            let clipped_diff = eb.call("Image.clip", &[("input", &idx_diff), ("geometry", &geom)]);

            let thresh = params
                .threshold
                .map(|t| (t, params.threshold_op.as_deref().unwrap_or("gt")));
            let stats_before = compute_single_band_stats_and_area(
                &client,
                &project_id,
                &access_token,
                &eb,
                &idx_before,
                &spec.index_name,
                &geom,
                scale_m,
                None,
                &mut tracker,
            )?;
            let stats_after = compute_single_band_stats_and_area(
                &client,
                &project_id,
                &access_token,
                &eb,
                &idx_after,
                &spec.index_name,
                &geom,
                scale_m,
                None,
                &mut tracker,
            )?;
            let stats_diff = compute_single_band_stats_and_area(
                &client,
                &project_id,
                &access_token,
                &eb,
                &idx_diff,
                "DIFF",
                &geom,
                scale_m,
                thresh.or(Some((0.1, "gt"))),
                &mut tracker,
            )?;

            let expr_before = eb.clone().build(&clipped_before);
            let expr_after = eb.clone().build(&clipped_after);
            let expr_diff = eb.build(&clipped_diff);

            let diff_palette: Vec<String> = ["#b2182b", "#ef8a62", "#f7f7f7", "#67a9cf", "#2166ac"]
                .into_iter()
                .map(str::to_owned)
                .collect();
            let diff_range = if spec.index_name == "SAR_DIFF" {
                6.0
            } else {
                0.35
            };

            let before_png = try_render_png_preview_data_url(
                &client,
                &project_id,
                &access_token,
                &expr_before,
                bbox,
                spec.vis_min,
                spec.vis_max,
                &spec.palette,
                &mut tracker,
            )
            .ok();
            let after_png = try_render_png_preview_data_url(
                &client,
                &project_id,
                &access_token,
                &expr_after,
                bbox,
                spec.vis_min,
                spec.vis_max,
                &spec.palette,
                &mut tracker,
            )
            .ok();
            let diff_png_res = try_render_png_preview_data_url(
                &client,
                &project_id,
                &access_token,
                &expr_diff,
                bbox,
                -diff_range,
                diff_range,
                &diff_palette,
                &mut tracker,
            );
            let before_tiles = try_create_map_tile_url(
                &client,
                &project_id,
                &access_token,
                &expr_before,
                spec.vis_min,
                spec.vis_max,
                &spec.palette,
                &mut tracker,
            )
            .ok();
            let after_tiles = try_create_map_tile_url(
                &client,
                &project_id,
                &access_token,
                &expr_after,
                spec.vis_min,
                spec.vis_max,
                &spec.palette,
                &mut tracker,
            )
            .ok();
            let diff_tiles_res = try_create_map_tile_url(
                &client,
                &project_id,
                &access_token,
                &expr_diff,
                -diff_range,
                diff_range,
                &diff_palette,
                &mut tracker,
            );
            if let (Err(preview_err), Err(tile_err)) = (&diff_png_res, &diff_tiles_res) {
                return Err(format!("{preview_err}; {tile_err}"));
            }
            let diff_png = diff_png_res.ok();
            let diff_tiles = diff_tiles_res.ok();

            let combined_stats = json!({
                "before_period": [b_start, b_end],
                "after_period": [a_start, a_end],
                "before": stats_before,
                "after": stats_after,
                "diff": stats_diff,
            });

            let art_id = new_artifact_id(&format!("chg-{asset_id}-{}", spec.index_name));
            let title = format!(
                "{} 前后两期变化 ({} -> {})",
                spec.index_name, b_start, a_end
            );
            let data_layer_record = build_data_layer_record(
                asset_id,
                profile.as_ref(),
                Some(&spec.index_name),
                Some(b_start),
                Some(a_end),
                Some(reducer_mode),
                Some(bbox),
                "working_or_request_bbox",
                Some(scale_m),
                Some("EPSG:4326"),
                &spec.required_bands,
                None,
                "cloud_map_layer",
                Some(&format!("Δ{}", spec.unit)),
            );
            store_gee_artifact(
                app_data_dir,
                GeeArtifactRecord {
                    artifact_id: art_id.clone(),
                    title: title.clone(),
                    dataset_id: asset_id.to_owned(),
                    operation: "change_detect".to_owned(),
                    bbox,
                    start_date: Some(b_start.to_owned()),
                    end_date: Some(a_end.to_owned()),
                    preview_data_url: diff_png.clone(),
                    before_preview_data_url: before_png.clone(),
                    after_preview_data_url: after_png.clone(),
                    tile_url_template: diff_tiles.clone(),
                    before_tile_url_template: before_tiles.clone(),
                    after_tile_url_template: after_tiles.clone(),
                    output_path: None,
                    size_bytes: None,
                    vis_min: -diff_range,
                    vis_max: diff_range,
                    palette: diff_palette.clone(),
                    unit: format!("Δ{}", spec.unit),
                    stats: combined_stats.clone(),
                    data_layer_record: Some(data_layer_record.clone()),
                    created_at: now_epoch_secs(),
                },
            );

            return Ok(json!({
                "ok": true,
                "dataset": "gee_cloud_analyze",
                "operation": "change_detect",
                "artifact_id": art_id,
                "title": title,
                "asset_id": asset_id,
                "index_name": spec.index_name,
                "bbox": bbox,
                "scale_m": scale_m,
                "stats": combined_stats,
                "preview_data_url": diff_png,
                "before_preview_data_url": before_png,
                "after_preview_data_url": after_png,
                "tile_url_template": diff_tiles,
                "before_tile_url_template": before_tiles,
                "after_tile_url_template": after_tiles,
                "vis_min": -diff_range,
                "vis_max": diff_range,
                "palette": diff_palette,
                "data_layer_record": data_layer_record,
            }));
        }

        // 2. Stepped Time Series Curve Extraction
        if op == "time_series" {
            let s_raw = params.start_date.as_deref().unwrap_or("2025-01-01");
            let e_raw = params.end_date.as_deref().unwrap_or("2025-10-01");
            let idx_name = params.index_name.as_deref().unwrap_or("NDVI");
            let spec = resolve_index_spec(asset_id, idx_name)?;

            let s_dt = chrono::NaiveDate::parse_from_str(&s_raw[..10.min(s_raw.len())], "%Y-%m-%d")
                .map_err(|e| format!("INVALID_START_DATE: {e}"))?;
            let e_dt = chrono::NaiveDate::parse_from_str(&e_raw[..10.min(e_raw.len())], "%Y-%m-%d")
                .map_err(|e| format!("INVALID_END_DATE: {e}"))?;
            let steps = resolve_time_series_steps(s_dt, e_dt, params.step_days, params.step_count);
            let raw_span_days = (e_dt - s_dt).num_days();
            let use_exact_step_days = params
                .step_days
                .filter(|&d| d > 0)
                .map(|d| (d as i64) * ((steps as i64) - 1) < raw_span_days)
                .unwrap_or(false);
            let total_days = raw_span_days.max(steps as i64);
            let stride_days = if use_exact_step_days {
                params.step_days.unwrap_or(30) as i64
            } else {
                (total_days / (steps as i64)).max(1)
            };

            let mut points = Vec::with_capacity(steps);
            for i in 0..steps {
                let ws = s_dt + chrono::Duration::days((i as i64) * stride_days);
                let we = if i + 1 == steps {
                    if e_dt > ws {
                        e_dt
                    } else {
                        ws + chrono::Duration::days(1)
                    }
                } else {
                    s_dt + chrono::Duration::days(((i + 1) as i64) * stride_days)
                };
                let ws_s = ws.format("%Y-%m-%d").to_string();
                let we_s = we.format("%Y-%m-%d").to_string();

                let mut eb = ExprBuilder::new();
                let geom = eb.rectangle(bbox);
                let col = eb.filtered_collection(
                    asset_id,
                    &geom,
                    Some(&ws_s),
                    Some(&we_s),
                    cloud_prop,
                    max_cloud,
                );
                let sz_node = eb.call("Collection.size", &[("collection", &col)]);
                let count_val = compute_value(
                    &client,
                    &project_id,
                    &access_token,
                    eb.clone().build(&sz_node),
                    &mut tracker,
                )?;
                let count = count_val.as_u64().ok_or_else(|| {
                    format!(
                        "GEE_INVALID_COLLECTION_SIZE: 时间窗口 {ws_s}..{we_s} 景数返回无效 ({count_val})"
                    )
                })?;
                if count == 0 {
                    points.push(json!({
                        "window_start": ws_s,
                        "window_end": we_s,
                        "scene_count": 0,
                        "mean": null,
                        "min": null,
                        "max": null,
                        "stats": {
                            "mean": null,
                            "min": null,
                            "max": null,
                        },
                    }));
                    continue;
                }
                let comp = eb.composite_and_select_bands(&col, reducer_mode, &spec.required_bands);
                let idx_node = build_index_node(&mut eb, asset_id, &comp, &spec);
                let st = compute_single_band_stats_and_area(
                    &client,
                    &project_id,
                    &access_token,
                    &eb,
                    &idx_node,
                    &spec.index_name,
                    &geom,
                    scale_m.max(30.0),
                    None,
                    &mut tracker,
                )?;
                points.push(json!({
                    "window_start": ws_s,
                    "window_end": we_s,
                    "scene_count": count,
                    "mean": st["mean"],
                    "min": st["min"],
                    "max": st["max"],
                    "stats": {
                        "mean": st["mean"],
                        "min": st["min"],
                        "max": st["max"],
                    },
                }));
            }

            let ts_stats = json!({
                "series": points,
            });
            let art_id = new_artifact_id(&format!("ts-{asset_id}-{}", spec.index_name));
            let title = format!("{} 时间序列 ({} -> {})", spec.index_name, s_raw, e_raw);
            let data_layer_record = build_data_layer_record(
                asset_id,
                profile.as_ref(),
                Some(&spec.index_name),
                Some(s_raw),
                Some(e_raw),
                Some(reducer_mode),
                Some(bbox),
                "working_or_request_bbox",
                Some(scale_m.max(30.0)),
                Some("EPSG:4326"),
                &spec.required_bands,
                None,
                "time_series_table",
                Some(&spec.unit),
            );
            store_gee_artifact(
                app_data_dir,
                GeeArtifactRecord {
                    artifact_id: art_id.clone(),
                    title: title.clone(),
                    dataset_id: asset_id.to_owned(),
                    operation: "time_series".to_owned(),
                    bbox,
                    start_date: Some(s_raw.to_owned()),
                    end_date: Some(e_raw.to_owned()),
                    preview_data_url: None,
                    before_preview_data_url: None,
                    after_preview_data_url: None,
                    tile_url_template: None,
                    before_tile_url_template: None,
                    after_tile_url_template: None,
                    output_path: None,
                    size_bytes: None,
                    vis_min: spec.vis_min,
                    vis_max: spec.vis_max,
                    palette: spec.palette.clone(),
                    unit: spec.unit.clone(),
                    stats: ts_stats.clone(),
                    data_layer_record: Some(data_layer_record.clone()),
                    created_at: now_epoch_secs(),
                },
            );

            return Ok(json!({
                "ok": true,
                "dataset": "gee_cloud_analyze",
                "operation": "time_series",
                "artifact_id": art_id,
                "title": title,
                "asset_id": asset_id,
                "index_name": spec.index_name,
                "bbox": bbox,
                "start_date": s_raw,
                "end_date": e_raw,
                "step_count": steps,
                "stats": ts_stats,
                "series": points,
                "data_layer_record": data_layer_record,
            }));
        }

        // 3. Single-Period Composite / Spectral-or-Radar Index / Zonal Stats + Area
        let start_d = params.start_date.as_deref().unwrap_or("2025-05-01");
        let end_d = params.end_date.as_deref().unwrap_or("2025-09-30");
        let is_timeless = is_static_dem_or_timeless_asset(asset_id, profile.as_ref());
        let eff_start = if is_timeless { None } else { Some(start_d) };
        let eff_end = if is_timeless { None } else { Some(end_d) };
        let eff_cloud = if is_timeless { None } else { max_cloud };
        if is_collection {
            verify_non_empty_collection(
                &client,
                &project_id,
                &access_token,
                asset_id,
                bbox,
                eff_start,
                eff_end,
                if is_timeless { None } else { cloud_prop },
                eff_cloud,
                &mut tracker,
            )?;
        }
        let mut eb = ExprBuilder::new();
        let geom = eb.rectangle(bbox);

        let (target_node, raw_vis_min, raw_vis_max, palette, label_title, stat_band, unit) = if op
            == "index"
            || params
                .index_name
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
        {
            let idx_name = params.index_name.as_deref().unwrap_or("NDVI");
            let spec = resolve_index_spec(asset_id, idx_name)?;
            let comp = build_source_image_node(
                &mut eb,
                profile.as_ref(),
                asset_id,
                &geom,
                eff_start,
                eff_end,
                eff_cloud,
                reducer_mode,
                &spec.required_bands,
            );
            let idx_node = build_index_node(&mut eb, asset_id, &comp, &spec);
            (
                idx_node,
                spec.vis_min,
                spec.vis_max,
                spec.palette,
                format!("{} ({})", spec.index_name, reducer_mode),
                Some(spec.index_name),
                spec.unit,
            )
        } else {
            let mut default_bands = params
                .bands
                .clone()
                .filter(|b| !b.is_empty())
                .or_else(|| profile.as_ref().map(|p| p.default_bands.clone()))
                .unwrap_or_default();
            if default_bands.is_empty()
                && gee_rest_api_base(app_data_dir) == "https://earthengine.googleapis.com"
            {
                if let Ok(stac) = fetch_dataset_stac_bands(&client, asset_id) {
                    if let Some(arr) = stac["bands"].as_array() {
                        let stac_names: Vec<String> = arr
                            .iter()
                            .filter_map(|b| {
                                b["name"]
                                    .as_str()
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .map(str::to_owned)
                            })
                            .collect();
                        if stac_names.len() >= 3 {
                            default_bands = stac_names.into_iter().take(3).collect();
                        } else if let Some(first) = stac_names.into_iter().next() {
                            default_bands = vec![first];
                        }
                    }
                }
            }
            let comp = build_source_image_node(
                &mut eb,
                profile.as_ref(),
                asset_id,
                &geom,
                eff_start,
                eff_end,
                eff_cloud,
                reducer_mode,
                &default_bands,
            );
            let (vis_node, vis_stat_band, pal) =
                prepare_composite_vis_node(&mut eb, &comp, &default_bands);
            let vmin = profile.as_ref().map(|p| p.vis_min).unwrap_or(0.0);
            let vmax = profile.as_ref().map(|p| p.vis_max).unwrap_or(3000.0);
            let title_zh = profile
                .as_ref()
                .map(|p| p.title_zh.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(asset_id);
            (
                vis_node,
                vmin,
                vmax,
                pal,
                format!("{title_zh} ({reducer_mode})"),
                vis_stat_band,
                "DN".to_owned(),
            )
        };

        let stats = if let Some(ref sb) = stat_band {
            let single = eb.select_single_band(&target_node, sb);
            let thresh = params
                .threshold
                .map(|t| (t, params.threshold_op.as_deref().unwrap_or("gt")));
            compute_single_band_stats_and_area(
                &client,
                &project_id,
                &access_token,
                &eb,
                &single,
                sb,
                &geom,
                scale_m,
                thresh,
                &mut tracker,
            )?
        } else {
            json!({})
        };

        // For unknown datasets in composite/layer mode, stretch vis range to observed [min, max]
        let (vis_min, vis_max) = if profile.is_none() && op != "index" {
            match (stats["min"].as_f64(), stats["max"].as_f64()) {
                (Some(mn), Some(mx)) if mn.is_finite() && mx.is_finite() && mx > mn => (mn, mx),
                _ => (raw_vis_min, raw_vis_max),
            }
        } else {
            (raw_vis_min, raw_vis_max)
        };

        let clipped = eb.call(
            "Image.clip",
            &[("input", &target_node), ("geometry", &geom)],
        );
        let expr = eb.build(&clipped);

        let preview_res = try_render_png_preview_data_url(
            &client,
            &project_id,
            &access_token,
            &expr,
            bbox,
            vis_min,
            vis_max,
            &palette,
            &mut tracker,
        );
        let tile_res = try_create_map_tile_url(
            &client,
            &project_id,
            &access_token,
            &expr,
            vis_min,
            vis_max,
            &palette,
            &mut tracker,
        );
        if let (Err(preview_err), Err(tile_err)) = (&preview_res, &tile_res) {
            return Err(format!("{preview_err}; {tile_err}"));
        }
        let preview_png = preview_res.ok();
        let tile_url = tile_res.ok();

        let art_id = new_artifact_id(&format!("ana-{asset_id}-{op}"));
        let data_layer_record = build_data_layer_record(
            asset_id,
            profile.as_ref(),
            params.index_name.as_deref().or(stat_band.as_deref()),
            eff_start,
            eff_end,
            Some(reducer_mode),
            Some(bbox),
            "working_or_request_bbox",
            Some(scale_m),
            Some("EPSG:4326"),
            params.bands.as_deref().unwrap_or(&[]),
            None,
            "cloud_map_layer",
            Some(&unit),
        );
        store_gee_artifact(
            app_data_dir,
            GeeArtifactRecord {
                artifact_id: art_id.clone(),
                title: label_title.clone(),
                dataset_id: asset_id.to_owned(),
                operation: op.clone(),
                bbox,
                start_date: Some(start_d.to_owned()),
                end_date: Some(end_d.to_owned()),
                preview_data_url: preview_png.clone(),
                before_preview_data_url: None,
                after_preview_data_url: None,
                tile_url_template: tile_url.clone(),
                before_tile_url_template: None,
                after_tile_url_template: None,
                output_path: None,
                size_bytes: None,
                vis_min,
                vis_max,
                palette: palette.clone(),
                unit: unit.clone(),
                stats: stats.clone(),
                data_layer_record: Some(data_layer_record.clone()),
                created_at: now_epoch_secs(),
            },
        );

        Ok(json!({
            "ok": true,
            "dataset": "gee_cloud_analyze",
            "operation": op,
            "artifact_id": art_id,
            "title": label_title,
            "asset_id": asset_id,
            "bbox": bbox,
            "start_date": start_d,
            "end_date": end_d,
            "scale_m": scale_m,
            "stats": stats,
            "preview_data_url": preview_png,
            "tile_url_template": tile_url,
            "vis_min": vis_min,
            "vis_max": vis_max,
            "palette": palette,
            "unit": unit,
            "data_layer_record": data_layer_record,
        }))
    })();

    finalize_gee_tracker(
        app_data_dir,
        Some((&client, &project_id, &access_token)),
        GeeLedgerOpKind::AnalysisRun,
        outcome.is_ok(),
        tracker,
    );
    outcome
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct GeeChunkedDownloadParams {
    pub asset_id: Option<String>,
    pub index_name: Option<String>,
    pub reducer: Option<String>,
    pub bands: Option<Vec<String>>,
    pub bbox: Option<[f64; 4]>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub max_cloud: Option<f64>,
    pub scale_m: Option<f64>,
    pub output_path: Option<String>,
    #[serde(default)]
    pub confirm_full_resolution: bool,
    #[serde(default)]
    pub use_preview_scale: bool,
}

/// Google-Drive-Free Chunked Grid Download + Seamless Local Float32 GeoTIFF Stitching.
pub fn execute_gee_chunked_download(
    app_data_dir: &Path,
    params: &GeeChunkedDownloadParams,
) -> Result<Value, String> {
    if let Some(raw_out) = params
        .output_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let p = Path::new(raw_out);
        if p.file_name().is_none()
            || p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(
                "UNSAFE_GEOTIFF_OUTPUT_PATH: 拒绝包含路径穿越(..)或缺少文件名的目标路径".to_owned(),
            );
        }
    }
    let asset_id = params
        .asset_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("COPERNICUS/S2_SR_HARMONIZED");
    let profile = lookup_dataset_profile(asset_id);
    let bbox = params.bbox.unwrap_or([117.24, 31.81, 117.30, 31.86]);
    let req_scale = params
        .scale_m
        .or_else(|| profile.as_ref().map(|p| p.native_scale_m))
        .unwrap_or(30.0);
    let reducer_mode = params.reducer.as_deref().unwrap_or("median");
    let max_cloud = Some(params.max_cloud.unwrap_or(30.0));
    let start_d = params.start_date.as_deref().unwrap_or("2025-05-01");
    let end_d = params.end_date.as_deref().unwrap_or("2025-09-30");

    let index_opt = params
        .index_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("none"));
    let bands_est = if index_opt.is_some() {
        1
    } else {
        params
            .bands
            .as_ref()
            .map(Vec::len)
            .filter(|n| *n > 0)
            .or_else(|| profile.as_ref().map(|p| p.default_bands.len().max(1)))
            .unwrap_or(1)
    };

    let plan = plan_chunk_grid(
        bbox,
        req_scale,
        bands_est,
        params.confirm_full_resolution,
        params.use_preview_scale,
    )?;

    // Above HARD_MAX_CHUNKS (64) without explicit preview scale: NEVER silently coarsen or promise full-res single download!
    if plan.total_chunks > HARD_MAX_CHUNKS && !params.use_preview_scale {
        return Err(format!(
            "GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD: 当前研究区按原生 {} 米分辨率共需请求 {} 个分块（{}×{} 像元，预估 {:.1} MB，超出单次在线无损拼接硬上限 {} 块），系统拒绝静默降采样篡改科研分辨率。请缩小研究区范围（AOI），或传 use_preview_scale: true 改用推荐的 {} 米轻量预览分辨率下载。",
            plan.requested_scale_m,
            plan.total_chunks,
            plan.full_width,
            plan.full_height,
            plan.estimated_mb,
            HARD_MAX_CHUNKS,
            plan.suggested_preview_scale_m
        ));
    }

    // Quota & Scientific Resolution Guardrail (9 < total_chunks <= 64): NEVER silently downsample!
    if plan.quota_confirm_required {
        let confirm_record = build_data_layer_record(
            asset_id,
            profile.as_ref(),
            index_opt,
            Some(start_d),
            Some(end_d),
            Some(reducer_mode),
            Some(plan.bbox),
            "working_or_request_bbox",
            Some(plan.effective_scale_m),
            Some("EPSG:4326"),
            params.bands.as_deref().unwrap_or(&[]),
            None,
            "local_chunked_geotiff_confirm",
            index_opt,
        );
        return Ok(json!({
            "ok": true,
            "quota_confirm_required": true,
            "asset_id": asset_id,
            "grid_plan": plan,
            "data_layer_record": confirm_record,
            "message_zh": format!(
                "当前研究区按原生 {} 米分辨率共需请求 {} 个分块（{}×{} 像元，预估 {:.1} MB，处于 10–{} 块需确认区间，未超单次 {} 块硬上限）。请选择：① 确认按原生全分辨率自动分块拼接下载；或 ② 改用推荐的 {} 米轻量预览分辨率下载。",
                plan.requested_scale_m,
                plan.total_chunks,
                plan.full_width,
                plan.full_height,
                plan.estimated_mb,
                HARD_MAX_CHUNKS,
                HARD_MAX_CHUNKS,
                plan.suggested_preview_scale_m
            ),
        }));
    }

    let client = build_gee_http_client(app_data_dir, 90)?;
    let (project_id, access_token) = resolve_gee_access_token(app_data_dir, &client, None)?;
    let mut tracker = GeeCallTracker::new();

    let outcome: Result<Value, String> = (|| {
        let mut eb = ExprBuilder::new();
        let geom = eb.rectangle(plan.bbox);
        let is_timeless = is_static_dem_or_timeless_asset(asset_id, profile.as_ref());
        let eff_start = if is_timeless { None } else { Some(start_d) };
        let eff_end = if is_timeless { None } else { Some(end_d) };
        let eff_cloud = if is_timeless { None } else { max_cloud };

        let (target_node, vis_min, vis_max, palette, title_label) = if let Some(idx_raw) = index_opt
        {
            let spec = resolve_index_spec(asset_id, idx_raw)?;
            let comp = build_source_image_node(
                &mut eb,
                profile.as_ref(),
                asset_id,
                &geom,
                eff_start,
                eff_end,
                eff_cloud,
                reducer_mode,
                &spec.required_bands,
            );
            let idx_node = build_index_node(&mut eb, asset_id, &comp, &spec);
            let float_node = eb.call("Image.toFloat", &[("value", &idx_node)]);
            (
                float_node,
                spec.vis_min,
                spec.vis_max,
                spec.palette,
                format!("{}_{}", asset_id.replace('/', "_"), spec.index_name),
            )
        } else {
            let sel_bands = params
                .bands
                .clone()
                .filter(|b| !b.is_empty())
                .or_else(|| profile.as_ref().map(|p| p.default_bands.clone()))
                .unwrap_or_default();
            let comp = build_source_image_node(
                &mut eb,
                profile.as_ref(),
                asset_id,
                &geom,
                eff_start,
                eff_end,
                eff_cloud,
                reducer_mode,
                &sel_bands,
            );
            let float_node = eb.call("Image.toFloat", &[("value", &comp)]);
            let vmin = profile.as_ref().map(|p| p.vis_min).unwrap_or(0.0);
            let vmax = profile.as_ref().map(|p| p.vis_max).unwrap_or(3000.0);
            let pal = if sel_bands.len() == 1 {
                vec![
                    "#0c2c84".into(),
                    "#41b6c4".into(),
                    "#ffffcc".into(),
                    "#fd8d3c".into(),
                    "#800026".into(),
                ]
            } else {
                Vec::new()
            };
            (
                float_node,
                vmin,
                vmax,
                pal,
                format!("{}_{}", asset_id.replace('/', "_"), reducer_mode),
            )
        };

        let expr = eb.clone().build(&target_node);
        let api_base = gee_rest_api_base(app_data_dir);
        let compute_url = format!("{api_base}/v1/projects/{project_id}/image:computePixels");

        let (actual_bands, band_names, stitched, chunks_completed) = fetch_and_stitch_gee_chunks(
            app_data_dir,
            &client,
            &project_id,
            &access_token,
            &compute_url,
            &expr,
            &plan,
            &mut tracker,
        )?;

        let file_name = format!(
            "gee_{}_{:.0}m_{}.tif",
            title_label
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                })
                .collect::<String>(),
            plan.effective_scale_m,
            now_epoch_secs()
        );
        let requested_out_path = params
            .output_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| app_data_dir.join("gee_downloads").join(&file_name));

        let req_parent = requested_out_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| app_data_dir.join("gee_downloads"));
        let req_stem = requested_out_path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("gee_mosaic")
            .to_owned();
        let req_ext = requested_out_path
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("tif")
            .to_owned();

        let mut final_out_path = None;
        let mut bytes_written = 0u64;
        for collision_idx in 0..=1024u32 {
            let cand_path = if collision_idx == 0 {
                requested_out_path.clone()
            } else {
                req_parent.join(format!("{req_stem}_{collision_idx}.{req_ext}"))
            };
            if fs::symlink_metadata(&cand_path).is_ok() {
                continue;
            }
            match write_geotiff_f32_epsg4326(
                &cand_path,
                plan.full_width,
                plan.full_height,
                actual_bands as u16,
                plan.bbox,
                &stitched,
            ) {
                Ok(written) => {
                    final_out_path = Some(cand_path);
                    bytes_written = written;
                    break;
                }
                Err(err) if err.starts_with("GEOTIFF_TARGET_ALREADY_EXISTS:") => continue,
                Err(err) => return Err(err),
            }
        }
        let out_path = final_out_path.ok_or_else(|| {
            "GEOTIFF_COLLISION_EXHAUSTED: 无法分配不覆盖已有文件的目标文件名".to_owned()
        })?;

        let raster_stats = summarize_raster_stats(&stitched, &band_names);

        // Also generate lightweight PNG preview & XYZ tile template for immediate map overlay
        let clipped = eb.call(
            "Image.clip",
            &[("input", &target_node), ("geometry", &geom)],
        );
        let clip_expr = eb.build(&clipped);
        let eff_min = if palette.is_empty() {
            vis_min
        } else {
            vis_min.min(raster_stats.min)
        };
        let eff_max = if palette.is_empty() {
            vis_max
        } else {
            vis_max.max(raster_stats.max)
        };
        let (preview_png, tile_url) = if api_base == "https://earthengine.googleapis.com" {
            let p = render_png_preview_data_url(
                &client,
                &project_id,
                &access_token,
                &clip_expr,
                plan.bbox,
                eff_min,
                eff_max,
                &palette,
                &mut tracker,
            );
            let t = create_map_tile_url(
                &client,
                &project_id,
                &access_token,
                &clip_expr,
                eff_min,
                eff_max,
                &palette,
                &mut tracker,
            );
            (p, t)
        } else {
            (None, None)
        };

        let art_id = new_artifact_id(&format!("dl-{title_label}"));
        let stats_json = serde_json::to_value(&raster_stats).unwrap_or_else(|_| json!({}));
        let data_layer_record = build_data_layer_record(
            asset_id,
            profile.as_ref(),
            index_opt,
            eff_start,
            eff_end,
            Some(reducer_mode),
            Some(plan.bbox),
            "working_or_request_bbox",
            Some(plan.effective_scale_m),
            Some("EPSG:4326"),
            &band_names,
            None,
            "local_geotiff",
            Some(index_opt.unwrap_or("Float32")),
        );
        store_gee_artifact(
            app_data_dir,
            GeeArtifactRecord {
                artifact_id: art_id.clone(),
                title: format!("{} ({:.0}m GeoTIFF)", title_label, plan.effective_scale_m),
                dataset_id: asset_id.to_owned(),
                operation: "chunked_download".to_owned(),
                bbox: plan.bbox,
                start_date: Some(start_d.to_owned()),
                end_date: Some(end_d.to_owned()),
                preview_data_url: preview_png.clone(),
                before_preview_data_url: None,
                after_preview_data_url: None,
                tile_url_template: tile_url.clone(),
                before_tile_url_template: None,
                after_tile_url_template: None,
                output_path: Some(out_path.to_string_lossy().to_string()),
                size_bytes: Some(bytes_written),
                vis_min: eff_min,
                vis_max: eff_max,
                palette: palette.clone(),
                unit: index_opt.unwrap_or("Float32").to_owned(),
                stats: stats_json.clone(),
                data_layer_record: Some(data_layer_record.clone()),
                created_at: now_epoch_secs(),
            },
        );

        Ok(json!({
            "ok": true,
            "quota_confirm_required": false,
            "dataset": "gee_chunked_download",
            "artifact_id": art_id,
            "asset_id": asset_id,
            "file_name": out_path.file_name().and_then(|s| s.to_str()).unwrap_or(&file_name),
            "output_path": out_path.to_string_lossy(),
            "bytes_written": bytes_written,
            "width": plan.full_width,
            "height": plan.full_height,
            "band_count": actual_bands,
            "band_names": band_names,
            "effective_scale_m": plan.effective_scale_m,
            "chunks_stitched": chunks_completed,
            "grid_plan": plan,
            "stats": stats_json,
            "preview_data_url": preview_png,
            "tile_url_template": tile_url,
            "vis_min": eff_min,
            "vis_max": eff_max,
            "palette": palette,
            "data_layer_record": data_layer_record,
        }))
    })();

    finalize_gee_tracker(
        app_data_dir,
        Some((&client, &project_id, &access_token)),
        GeeLedgerOpKind::ChunkDownload,
        outcome.is_ok(),
        tracker,
    );
    outcome
}

#[allow(clippy::too_many_arguments)]
fn fetch_and_stitch_gee_chunks(
    app_data_dir: &Path,
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    compute_url: &str,
    expr: &Value,
    plan: &ChunkGridPlan,
    tracker: &mut GeeCallTracker,
) -> Result<(usize, Vec<String>, Vec<f32>, u32), String> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Clone, Copy)]
    struct ChunkJob {
        cx: usize,
        cy: usize,
        x0: usize,
        y0: usize,
        cw: usize,
        ch: usize,
        tile_translate_x: f64,
        tile_translate_y: f64,
    }

    let full_w = plan.full_width as usize;
    let full_h = plan.full_height as usize;
    let tile_px = plan.chunk_size_px as usize;
    let min_lon = plan.bbox[0];
    let max_lat = plan.bbox[3];

    let mut jobs = Vec::with_capacity((plan.chunks_x * plan.chunks_y) as usize);
    for cy in 0..(plan.chunks_y as usize) {
        let y0 = cy * tile_px;
        let ch = (full_h - y0).min(tile_px);
        let tile_translate_y = max_lat + (y0 as f64) * plan.scale_y_deg;
        for cx in 0..(plan.chunks_x as usize) {
            let x0 = cx * tile_px;
            let cw = (full_w - x0).min(tile_px);
            let tile_translate_x = min_lon + (x0 as f64) * plan.scale_x_deg;
            jobs.push(ChunkJob {
                cx,
                cy,
                x0,
                y0,
                cw,
                ch,
                tile_translate_x,
                tile_translate_y,
            });
        }
    }

    let configured_workers = insar_download::load_transfer_engine_prefs_in_dir(app_data_dir)
        .map(|p| usize::from(p.tile_chunk_concurrency))
        .unwrap_or(16)
        .clamp(1, 64);
    let worker_count = configured_workers.min(jobs.len().max(1));

    let next_idx = AtomicUsize::new(0);
    let abort = AtomicBool::new(false);
    let first_error: Mutex<Option<String>> = Mutex::new(None);
    let decoded_tiles: Mutex<Vec<Option<DecodedNpyTile>>> =
        Mutex::new((0..jobs.len()).map(|_| None).collect());
    let shared_tracker = Mutex::new(tracker);

    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| {
                while !abort.load(Ordering::Acquire) {
                    let idx = next_idx.fetch_add(1, Ordering::Relaxed);
                    let Some(&job) = jobs.get(idx) else {
                        break;
                    };
                    let req_body = json!({
                        "expression": expr,
                        "fileFormat": "NPY",
                        "grid": {
                            "dimensions": { "width": job.cw, "height": job.ch },
                            "affineTransform": {
                                "scaleX": plan.scale_x_deg,
                                "shearX": 0.0,
                                "translateX": job.tile_translate_x,
                                "shearY": 0.0,
                                "scaleY": plan.scale_y_deg,
                                "translateY": job.tile_translate_y,
                            },
                            "crsCode": "EPSG:4326",
                        }
                    });
                    let mut local_tracker = GeeCallTracker::new();
                    let bytes_res = gee_post_bytes(
                        client,
                        project_id,
                        access_token,
                        compute_url,
                        &req_body,
                        &mut local_tracker,
                    );
                    if let Ok(mut guard) = shared_tracker.lock() {
                        guard.merge_from(local_tracker);
                    }
                    let bytes = match bytes_res {
                        Ok(b) => b,
                        Err(err) => {
                            abort.store(true, Ordering::Release);
                            if let Ok(mut slot) = first_error.lock() {
                                if slot.is_none() {
                                    *slot = Some(err);
                                }
                            }
                            break;
                        }
                    };
                    let tile = match decode_npy_tile(&bytes) {
                        Ok(t) => t,
                        Err(err) => {
                            abort.store(true, Ordering::Release);
                            if let Ok(mut slot) = first_error.lock() {
                                if slot.is_none() {
                                    *slot = Some(err);
                                }
                            }
                            break;
                        }
                    };
                    if tile.width != job.cw || tile.height != job.ch {
                        abort.store(true, Ordering::Release);
                        if let Ok(mut slot) = first_error.lock() {
                            if slot.is_none() {
                                *slot = Some(format!(
                                    "GEE_TILE_DIM_MISMATCH: 分块 ({},{}) 期望 {}×{}，实际 {}×{}",
                                    job.cx, job.cy, job.cw, job.ch, tile.width, tile.height
                                ));
                            }
                        }
                        break;
                    }
                    if let Ok(mut tiles_guard) = decoded_tiles.lock() {
                        tiles_guard[idx] = Some(tile);
                    }
                }
            });
        }
    });

    if let Some(err) = first_error.into_inner().unwrap_or(None) {
        return Err(err);
    }

    let tiles = decoded_tiles
        .into_inner()
        .map_err(|_| "GEE_CHUNK_LOCK_POISONED".to_owned())?;
    let mut actual_bands = 0usize;
    let mut band_names: Vec<String> = Vec::new();
    let mut stitched: Vec<f32> = Vec::new();
    let mut chunks_completed = 0u32;

    for (job, maybe_tile) in jobs.iter().zip(tiles) {
        let tile = maybe_tile
            .ok_or_else(|| format!("GEE_TILE_MISSING: 分块 ({},{}) 缺失", job.cx, job.cy))?;
        if actual_bands == 0 {
            actual_bands = tile.band_names.len().max(1);
            band_names = tile.band_names.clone();
            stitched = vec![f32::NAN; full_w * full_h * actual_bands];
        } else if tile.band_names.len() != actual_bands {
            return Err("GEE_TILE_BAND_COUNT_MISMATCH: 分块波段数不一致".to_owned());
        }
        for row in 0..job.ch {
            let src_start = row * job.cw * actual_bands;
            let src_end = src_start + job.cw * actual_bands;
            let dst_start = ((job.y0 + row) * full_w + job.x0) * actual_bands;
            let dst_end = dst_start + job.cw * actual_bands;
            stitched[dst_start..dst_end].copy_from_slice(&tile.pixels[src_start..src_end]);
        }
        chunks_completed += 1;
    }

    Ok((actual_bands, band_names, stitched, chunks_completed))
}

// ============================================================================
// Unified Tauri Geodata Dispatcher (`gee_*` commands) + Agent Safe Projection
// ============================================================================

#[allow(clippy::too_many_arguments)]
pub fn dispatch_gee_geodata_query(
    app_data_dir: &Path,
    dataset: &str,
    keyword: Option<&str>,
    bbox: Option<[f64; 4]>,
    start_date: Option<&str>,
    end_date: Option<&str>,
    output_path: Option<&str>,
    gee_params: Option<&Value>,
) -> Result<Value, String> {
    let p = gee_params.cloned().unwrap_or_else(|| json!({}));

    match dataset {
        "gee_account_status" => Ok(gee_account_status(app_data_dir)),
        "gee_quota_status" => {
            let refresh = p["refresh"].as_bool().unwrap_or(false);
            get_gee_quota_status(app_data_dir, refresh)
        }
        "gee_account_save" => {
            let pid = p["project_id"]
                .as_str()
                .or(keyword)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let sec = p["secret"]
                .as_str()
                .or_else(|| p["service_account_json"].as_str())
                .ok_or_else(|| {
                    "GEE_SECRET_REQUIRED: 请提供 Service Account JSON 密钥".to_owned()
                })?;
            save_gee_credentials(app_data_dir, pid, sec)
        }
        "gee_account_verify" => {
            let pid = p["project_id"].as_str();
            let sec = p["secret"].as_str();
            verify_gee_account(app_data_dir, pid, sec)
        }
        "gee_account_clear" => clear_gee_credentials(app_data_dir),
        "gee_catalog" | "gee_catalog_search" => {
            let q = p["query"].as_str().or(keyword).unwrap_or("");
            let opts = CatalogSearchOptions {
                include_community: p["include_community"].as_bool().unwrap_or(true),
                limit: p["limit"].as_u64().unwrap_or(24) as usize,
                force_refresh: p["force_refresh"].as_bool().unwrap_or(false),
                source_filter: p["source"].as_str().map(str::to_owned),
                provider_filter: p["provider"].as_str().map(str::to_owned),
                kind_filter: p["kind"].as_str().map(str::to_owned),
                max_resolution_m: p["max_resolution_m"].as_f64(),
                include_deprecated: p["include_deprecated"].as_bool().unwrap_or(false),
            };
            let client = build_gee_http_client(app_data_dir, 20).ok();
            Ok(search_gee_catalog_with_options(
                app_data_dir,
                client.as_ref(),
                q,
                &opts,
            ))
        }
        "gee_recommend_datasets" => {
            let task = p["task"].as_str().or(keyword).unwrap_or("");
            let limit_per_role = p["limit_per_role"].as_u64().unwrap_or(2) as usize;
            let include_comm = p["include_community"].as_bool().unwrap_or(true);
            let client = build_gee_http_client(app_data_dir, 20).ok();
            Ok(recommend_gee_dataset_bundle(
                app_data_dir,
                client.as_ref(),
                task,
                limit_per_role,
                include_comm,
            ))
        }
        "gee_compare_datasets" => {
            let ids: Vec<String> = p["ids"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let include_comm = p["include_community"].as_bool().unwrap_or(true);
            let client = build_gee_http_client(app_data_dir, 20).ok();
            Ok(compare_gee_datasets(
                app_data_dir,
                client.as_ref(),
                &ids,
                include_comm,
            ))
        }
        "gee_verify_dataset" => {
            let asset_id = p["asset_id"]
                .as_str()
                .or_else(|| p["id"].as_str())
                .or(keyword)
                .unwrap_or("");
            let live = p["live"].as_bool().unwrap_or(false);
            let client = if live {
                build_gee_http_client(app_data_dir, 20).ok()
            } else {
                None
            };
            verify_gee_dataset(app_data_dir, client.as_ref(), asset_id, live)
        }
        "gee_dataset_advisor" => {
            let mode = p["mode"]
                .as_str()
                .map(str::trim)
                .unwrap_or("recommend")
                .to_ascii_lowercase();
            let include_comm = p["include_community"].as_bool().unwrap_or(true);
            match mode.as_str() {
                "recommend" => {
                    let task = p["task"].as_str().or(keyword).unwrap_or("");
                    let limit_per_role = p["limit_per_role"].as_u64().unwrap_or(2) as usize;
                    let client = build_gee_http_client(app_data_dir, 20).ok();
                    Ok(recommend_gee_dataset_bundle(
                        app_data_dir,
                        client.as_ref(),
                        task,
                        limit_per_role,
                        include_comm,
                    ))
                }
                "compare" => {
                    let ids: Vec<String> = p["ids"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default();
                    let client = build_gee_http_client(app_data_dir, 20).ok();
                    Ok(compare_gee_datasets(
                        app_data_dir,
                        client.as_ref(),
                        &ids,
                        include_comm,
                    ))
                }
                "verify" => {
                    let asset_id = p["asset_id"]
                        .as_str()
                        .or_else(|| p["id"].as_str())
                        .or(keyword)
                        .unwrap_or("");
                    let live = p["live"].as_bool().unwrap_or(false);
                    let client = if live {
                        build_gee_http_client(app_data_dir, 20).ok()
                    } else {
                        None
                    };
                    verify_gee_dataset(app_data_dir, client.as_ref(), asset_id, live)
                }
                _ => Err(
                    "GEE_ADVISOR_INVALID_MODE: mode 必须为 recommend、compare 或 verify".to_owned(),
                ),
            }
        }
        "gee_export_preflight" => {
            let mut parsed: GeeExportPreflightParams =
                serde_json::from_value(p.clone()).unwrap_or_default();
            if parsed.asset_id.is_none() {
                parsed.asset_id = keyword.map(str::to_owned);
            }
            if parsed.bbox.is_none() {
                parsed.bbox = bbox;
            }
            if parsed.start_date.is_none() {
                parsed.start_date = start_date.map(str::to_owned);
            }
            if parsed.end_date.is_none() {
                parsed.end_date = end_date.map(str::to_owned);
            }
            Ok(plan_gee_export_preflight(app_data_dir, &parsed))
        }
        "gee_dataset_bands" => {
            let asset_id = p["asset_id"]
                .as_str()
                .or(keyword)
                .ok_or_else(|| "GEE_ASSET_ID_REQUIRED".to_owned())?;
            let client = build_gee_http_client(app_data_dir, 20)?;
            fetch_dataset_stac_bands(&client, asset_id)
        }
        "gee_check_availability" => {
            let asset_id = p["asset_id"]
                .as_str()
                .or(keyword)
                .unwrap_or("COPERNICUS/S2_SR_HARMONIZED");
            let eff_bbox = p
                .get("bbox")
                .and_then(|v| serde_json::from_value::<[f64; 4]>(v.clone()).ok())
                .or(bbox)
                .unwrap_or([117.20, 31.78, 117.34, 31.89]);
            let sd = p["start_date"].as_str().or(start_date);
            let ed = p["end_date"].as_str().or(end_date);
            let mc = p["max_cloud"].as_f64();
            let sc = p["scale_m"].as_f64();
            let sar_pol = p["sar_polarization"].as_str();
            let orb_dir = p["orbit_direction"].as_str();
            check_gee_availability_with_filters(
                app_data_dir,
                asset_id,
                eff_bbox,
                sd,
                ed,
                mc,
                sc,
                sar_pol,
                orb_dir,
            )
        }
        "gee_cloud_analyze" | "gee_map_tiles" => {
            let mut parsed: GeeCloudAnalyzeParams =
                serde_json::from_value(p.clone()).unwrap_or_default();
            if parsed.asset_id.is_none() {
                parsed.asset_id = keyword.map(str::to_owned);
            }
            if parsed.bbox.is_none() {
                parsed.bbox = bbox;
            }
            if parsed.start_date.is_none() {
                parsed.start_date = start_date.map(str::to_owned);
            }
            if parsed.end_date.is_none() {
                parsed.end_date = end_date.map(str::to_owned);
            }
            if dataset == "gee_map_tiles" && parsed.operation.is_none() {
                parsed.operation = Some("composite".to_owned());
            }
            execute_gee_cloud_analyze(app_data_dir, &parsed)
        }
        "gee_chunked_download" | "gee_download" => {
            let mut parsed: GeeChunkedDownloadParams =
                serde_json::from_value(p.clone()).unwrap_or_default();
            if parsed.asset_id.is_none() {
                parsed.asset_id = keyword.map(str::to_owned);
            }
            if parsed.bbox.is_none() {
                parsed.bbox = bbox;
            }
            if parsed.start_date.is_none() {
                parsed.start_date = start_date.map(str::to_owned);
            }
            if parsed.end_date.is_none() {
                parsed.end_date = end_date.map(str::to_owned);
            }
            if parsed.output_path.is_none() {
                parsed.output_path = output_path.map(str::to_owned);
            }
            execute_gee_chunked_download(app_data_dir, &parsed)
        }
        "gee_artifact_get" => {
            let art_id = p["artifact_id"]
                .as_str()
                .or(keyword)
                .ok_or_else(|| "GEE_ARTIFACT_ID_REQUIRED".to_owned())?;
            let rec = load_gee_artifact(app_data_dir, art_id)?;
            serde_json::to_value(rec).map_err(|e| format!("GEE_ARTIFACT_JSON_ERR: {e}"))
        }
        "gee_script_env_status" => Ok(get_script_env_status(app_data_dir)),
        "gee_script_inspect" => {
            let script = p["script"]
                .as_str()
                .or(keyword)
                .ok_or_else(|| "GEE_SCRIPT_REQUIRED".to_owned())?;
            let rep = inspect_gee_python_script(app_data_dir, script);
            serde_json::to_value(rep).map_err(|e| format!("GEE_INSPECT_JSON_ERR: {e}"))
        }
        "gee_script_preview" => {
            let approval_id = p["approval_id"]
                .as_str()
                .or_else(|| p["approval_token"].as_str())
                .or(keyword)
                .unwrap_or("");
            preview_approved_gee_script(approval_id)
        }
        "gee_script_execute" => {
            let script = p["script"]
                .as_str()
                .ok_or_else(|| "GEE_SCRIPT_REQUIRED".to_owned())?;
            let approval_id = p["approval_id"]
                .as_str()
                .or_else(|| p["approval_token"].as_str())
                .unwrap_or("");
            let eff_bbox = p
                .get("bbox")
                .and_then(|v| serde_json::from_value::<[f64; 4]>(v.clone()).ok())
                .or(bbox)
                .unwrap_or([117.20, 31.78, 117.34, 31.89]);
            let proxy = detect_effective_proxy_url(app_data_dir);
            let client = build_gee_http_client(app_data_dir, 30)?;
            let (project_id, access_token) = resolve_gee_access_token(app_data_dir, &client, None)?;
            let raw_res = execute_approved_gee_script(
                app_data_dir,
                approval_id,
                script,
                &project_id,
                &access_token,
                eff_bbox,
                proxy.as_deref(),
            )?;
            flush_pending_profiles(app_data_dir, &client, &project_id, &access_token);

            // If preview.png was produced, encode data URL & register artifact
            let mut out = raw_res;
            if let Some(png_path_str) = out["preview_png_path"].as_str() {
                if let Ok(bytes) = fs::read(png_path_str) {
                    let data_url = format!("data:image/png;base64,{}", B64_STD.encode(&bytes));
                    let art_id = new_artifact_id("script");
                    store_gee_artifact(
                        app_data_dir,
                        GeeArtifactRecord {
                            artifact_id: art_id.clone(),
                            title: "GEE 自定义脚本计算结果".to_owned(),
                            dataset_id: "CUSTOM_EE_SCRIPT".to_owned(),
                            operation: "script_execute".to_owned(),
                            bbox: eff_bbox,
                            start_date: None,
                            end_date: None,
                            preview_data_url: Some(data_url.clone()),
                            before_preview_data_url: None,
                            after_preview_data_url: None,
                            tile_url_template: None,
                            before_tile_url_template: None,
                            after_tile_url_template: None,
                            output_path: None,
                            size_bytes: None,
                            vis_min: 0.0,
                            vis_max: 1.0,
                            palette: Vec::new(),
                            unit: "".to_owned(),
                            stats: out["result"].clone(),
                            data_layer_record: None,
                            created_at: now_epoch_secs(),
                        },
                    );
                    out["artifact_id"] = json!(art_id);
                    out["preview_data_url"] = json!(data_url);
                }
            }
            Ok(out)
        }
        _ => Err(format!(
            "UNKNOWN_GEE_OPERATION: 未知的 GEE 操作 `{dataset}`"
        )),
    }
}

/// Sanitize a GEE tool result so it is 100% guaranteed to pass `insar_agent::run::safe_value`:
/// - Strips large base64 PNG strings (`preview_data_url`, `*_preview_data_url`), `{z}/{x}/{y}` templates,
///   and local filesystem paths (`output_path`, `preview_png_path`), preserving `artifact_id` + `file_name` + numeric `stats`!
/// - Replaces any standalone ` / ` or path-like slash in prose while keeping GEE dataset IDs (`COPERNICUS/S2_SR_HARMONIZED`).
pub fn sanitize_for_agent_trace(raw: &Value) -> Value {
    match raw {
        Value::Object(map) => {
            let mut clean = serde_json::Map::new();
            for (k, v) in map {
                let kl = k.to_ascii_lowercase();
                if matches!(
                    kl.as_str(),
                    "preview_data_url"
                        | "before_preview_data_url"
                        | "after_preview_data_url"
                        | "tile_url_template"
                        | "before_tile_url_template"
                        | "after_tile_url_template"
                        | "output_path"
                        | "output_root"
                        | "preview_png_path"
                        | "asset_url"
                        | "stac_url"
                        | "client_email"
                        | "docs"
                        | "license_url"
                        | "url"
                ) || kl.contains("token")
                    || kl.contains("secret")
                    || kl.contains("password")
                    || kl.contains("credential")
                    || kl.contains("api_key")
                    || kl.contains("cookie")
                    || kl.contains("authorization")
                    || kl.contains("access_key")
                    || kl.contains("netrc")
                {
                    continue;
                }
                clean.insert(k.clone(), sanitize_for_agent_trace(v));
            }
            Value::Object(clean)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sanitize_for_agent_trace).collect()),
        Value::String(s) => Value::String(sanitize_string_for_agent_trace(s)),
        _ => raw.clone(),
    }
}

fn sanitize_string_for_agent_trace(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    if (s.starts_with('/') && !s.starts_with("//") && s.len() > 1)
        || s.contains(":\\")
        || s.contains("\\\\")
        || lower.contains("file://")
        || lower.contains("gs://")
        || lower.contains("ee://")
        || lower.contains("ftp://")
        || lower.contains("%2f")
        || s.contains("/Users/")
        || s.contains("/home/")
        || s.contains("/var/")
        || lower.contains(".netrc")
    {
        return "[local]".to_owned();
    }
    if (lower.contains("http://") || lower.contains("https://"))
        && (s.contains('?') || s.contains('#') || s.contains('@'))
    {
        return "[redacted]".to_owned();
    }

    let normalized = s.replace(" / ", " · ");
    let chars: Vec<char> = normalized.chars().collect();
    let mut out = String::with_capacity(normalized.len() + 8);
    for (idx, &ch) in chars.iter().enumerate() {
        if ch != '/' {
            out.push(ch);
            continue;
        }
        let prev = idx.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(idx + 1).copied();
        let keep_slash = match (prev, next) {
            (Some(p), Some(n))
                if p.is_ascii() && n.is_ascii() && !p.is_whitespace() && !n.is_whitespace() =>
            {
                // Preserve pure ASCII GEE asset IDs (`COPERNICUS/S2_SR_HARMONIZED`,
                // `MODIS/061/MOD11A1`, `UCSB-CHG/CHIRPS/DAILY`) while replacing
                // punctuation-adjacent `/` before letters (`)/Terra`) that `unix_path_in` flags.
                p.is_ascii_alphanumeric() || matches!(p, '.' | ':' | '/') || !n.is_alphabetic()
            }
            _ => false,
        };
        if keep_slash {
            out.push('/');
        } else {
            out.push('·');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gee_cloud_suite::catalog::search_gee_catalog;

    fn build_synthetic_npy_v1(
        width: usize,
        height: usize,
        fields: &[(&str, &str)],
        vals: &[f32],
    ) -> Vec<u8> {
        let descr_items: Vec<String> = fields
            .iter()
            .map(|(n, dt)| format!("('{n}', '{dt}')"))
            .collect();
        let dict_str = format!(
            "{{'descr': [{}], 'fortran_order': False, 'shape': ({height}, {width}), }}",
            descr_items.join(", ")
        );
        let mut header_bytes = dict_str.into_bytes();
        // Pad so (10 + header_len) % 64 == 0, ending with '\n'
        while (10 + header_bytes.len() + 1) % 64 != 0 {
            header_bytes.push(b' ');
        }
        header_bytes.push(b'\n');
        let hlen = header_bytes.len() as u16;

        let mut buf = Vec::new();
        buf.extend_from_slice(b"\x93NUMPY\x01\x00");
        buf.extend_from_slice(&hlen.to_le_bytes());
        buf.extend_from_slice(&header_bytes);
        for &v in vals {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf
    }

    #[test]
    fn unit_npy_decoder_and_geotiff_writer_preserve_exact_pixels_and_tags() {
        let pixels = vec![
            0.12_f32, 0.34, 0.56, 0.78, // row 0 (2 px * 2 bands)
            -0.25, 0.0, 0.91, -0.05, // row 1
        ];
        let npy = build_synthetic_npy_v1(2, 2, &[("NDVI", "<f4"), ("NDWI", "<f4")], &pixels);
        let decoded = decode_npy_tile(&npy).expect("decode synthetic 2x2x2 npy");
        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.band_names, vec!["NDVI", "NDWI"]);
        assert_eq!(decoded.pixels.len(), 8);
        assert!((decoded.pixels[0] - 0.12).abs() < 1e-6);
        assert!((decoded.pixels[6] - 0.91).abs() < 1e-6);

        let tmp =
            std::env::temp_dir().join(format!("eostage-gee-geotiff-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let tif_path = tmp.join("mosaic_test.tif");
        let bbox = [117.20, 31.80, 117.30, 31.90];
        let written = write_geotiff_f32_epsg4326(&tif_path, 2, 2, 2, bbox, &decoded.pixels)
            .expect("write float32 geotiff");
        assert!(written > 200);

        let raw = fs::read(&tif_path).unwrap();
        assert_eq!(&raw[0..4], b"II*\0");
        let entry_count = u16::from_le_bytes([raw[8], raw[9]]) as usize;
        assert_eq!(entry_count, 14);
        // Verify strictly ascending TIFF tag IDs (required by TIFF 6.0 / GDAL)
        let mut prev_tag = 0u16;
        for i in 0..entry_count {
            let off = 10 + i * 12;
            let tag = u16::from_le_bytes([raw[off], raw[off + 1]]);
            assert!(
                tag > prev_tag,
                "tags must be strictly ascending: {tag} <= {prev_tag}"
            );
            prev_tag = tag;
        }
        assert_eq!(prev_tag, 34735); // GeoKeyDirectoryTag

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unit_quota_guardrail_and_bilingual_catalog_and_script_security_gate() {
        // 1. Small AOI at 10m -> <= 9 chunks -> auto-approved
        let small = plan_chunk_grid([117.25, 31.82, 117.28, 31.85], 10.0, 1, false, false).unwrap();
        assert!(!small.quota_confirm_required);
        assert!(small.total_chunks <= 9);

        // 2. Large AOI at 10m -> > 9 chunks -> requires user resolution/quota choice (never silent downsample!)
        let large = plan_chunk_grid([117.0, 31.5, 117.8, 32.2], 10.0, 3, false, false).unwrap();
        assert!(large.quota_confirm_required);
        assert_eq!(large.effective_scale_m, 10.0);
        assert!(large.suggested_preview_scale_m > 10.0);

        // 3. Bilingual catalog search
        let tmp = std::env::temp_dir().join(format!("eostage-gee-cat-unit-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        let s2_hits = search_gee_catalog(&tmp, None, "哨兵2 地表反射率", true, 10, false);
        assert!(s2_hits["count"].as_u64().unwrap() >= 1);
        assert_eq!(s2_hits["items"][0]["id"], "COPERNICUS/S2_SR_HARMONIZED");

        let night_hits = search_gee_catalog(&tmp, None, "夜间灯光", true, 10, false);
        assert!(night_hits["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|it| it["id"] == "NOAA/VIIRS/DNB/MONTHLY_V1/VCMSLCFG"));

        // 4. Script security gate blocks subprocess/socket & flags unscaled Landsat C2 L2
        let bad = inspect_gee_python_script(&tmp, "import subprocess\nsubprocess.run(['ls'])");
        assert!(bad.blocked);
        assert!(bad.approval_id.is_none());

        let warn_rep = inspect_gee_python_script(
            &tmp,
            "col = ee.ImageCollection('LANDSAT/LC08/C02/T1_L2')\nimg = col.first().select('SR_B4')\nRESULT = img.reduceRegion(ee.Reducer.mean(), AOI)",
        );
        assert!(!warn_rep.blocked);
        assert!(warn_rep.approval_id.is_some());
        assert!(warn_rep.scientific_warnings.len() >= 2);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unit_change_detect_dates_and_time_series_step_days_resolution() {
        // 1. Agent schema pattern: `start_date`/`end_date` + `after_start_date`/`after_end_date` (without `before_start_date`)
        let p_agent = GeeCloudAnalyzeParams {
            start_date: Some("2020-06-01".into()),
            end_date: Some("2020-09-01".into()),
            after_start_date: Some("2025-06-01".into()),
            after_end_date: Some("2025-09-01".into()),
            ..Default::default()
        };
        let (bs, be, as_, ae) = resolve_change_detect_dates(&p_agent);
        assert_eq!(bs, "2020-06-01");
        assert_eq!(be, "2020-09-01");
        assert_eq!(as_, "2025-06-01");
        assert_eq!(ae, "2025-09-01");

        // 2. Explicit `before_start_date`/`before_end_date` + `after_start_date`/`after_end_date`
        let p_explicit = GeeCloudAnalyzeParams {
            before_start_date: Some("2019-05-01".into()),
            before_end_date: Some("2019-08-31".into()),
            after_start_date: Some("2024-05-01".into()),
            after_end_date: Some("2024-08-31".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_change_detect_dates(&p_explicit),
            (
                "2019-05-01".into(),
                "2019-08-31".into(),
                "2024-05-01".into(),
                "2024-08-31".into()
            )
        );

        // 3. `time_series` `step_days` calculation & 2..=24 clamping
        let d0 = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let d90 = chrono::NaiveDate::from_ymd_opt(2025, 4, 1).unwrap(); // 90 days
        assert_eq!(resolve_time_series_steps(d0, d90, Some(30), None), 3);
        assert_eq!(resolve_time_series_steps(d0, d90, Some(120), None), 2); // clamped min 2
        assert_eq!(resolve_time_series_steps(d0, d90, Some(2), None), 24); // 45 -> clamped max 24
        assert_eq!(resolve_time_series_steps(d0, d90, None, Some(8)), 8);
        assert_eq!(resolve_time_series_steps(d0, d90, None, None), 6);
    }

    #[test]
    fn unit_sanitize_for_agent_trace_handles_chinese_slashes_and_passes_safe_value() {
        let raw = json!({
            "ok": true,
            "asset_id": "COPERNICUS/S2_SR_HARMONIZED",
            "community_id": "projects/sat-io/open-datasets/FABDEM",
            "modis_title": "MODIS 全球逐日 1公里地表温度 (LST 白天/夜间)",
            "era5_title": "ECMWF ERA5-Land 全球逐日陆面气象再分析 (气温/降水/土壤湿度)",
            "mixed_prose": "对比 白天/Night 与 Day/夜间 以及 (SAR)/InSAR 指标",
            "local_leak": "/tmp/should_never_leak.tif",
        });
        let sanitized = sanitize_for_agent_trace(&raw);
        insar_agent::run::safe_value(&sanitized)
            .expect("sanitized JSON with Chinese-adjacent slashes must pass safe_value");
        assert_eq!(sanitized["asset_id"], "COPERNICUS/S2_SR_HARMONIZED");
        assert_eq!(
            sanitized["community_id"],
            "projects/sat-io/open-datasets/FABDEM"
        );
        assert_eq!(
            sanitized["modis_title"],
            "MODIS 全球逐日 1公里地表温度 (LST 白天·夜间)"
        );
        assert_eq!(
            sanitized["era5_title"],
            "ECMWF ERA5-Land 全球逐日陆面气象再分析 (气温·降水·土壤湿度)"
        );
        assert_eq!(sanitized["local_leak"], "[local]");

        // Verify every single built-in curated catalog item passes `safe_value` after projection
        let tmp = std::env::temp_dir().join(format!("eostage-gee-safe-cat-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        let all_cat = search_gee_catalog(&tmp, None, "", true, 100, false);
        let safe_cat = sanitize_for_agent_trace(&all_cat);
        insar_agent::run::safe_value(&safe_cat).expect("all curated catalog items pass safe_value");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unit_chunked_download_above_64_chunks_returns_explicit_error_and_mid_range_prompts() {
        let tmp = std::env::temp_dir().join(format!("eostage-gee-dl-guard-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);

        // 9 < raw_total <= 64 -> honest quota confirmation prompt
        let mid_res = execute_gee_chunked_download(
            &tmp,
            &GeeChunkedDownloadParams {
                asset_id: Some("COPERNICUS/S2_SR_HARMONIZED".into()),
                bbox: Some([117.10, 31.70, 117.31, 31.90]),
                scale_m: Some(10.0),
                confirm_full_resolution: false,
                use_preview_scale: false,
                ..Default::default()
            },
        )
        .expect("10..=64 chunks returns quota_confirm_required");
        assert_eq!(mid_res["quota_confirm_required"], true);
        assert!(mid_res["message_zh"].as_str().unwrap().contains("64 块"));

        // raw_total > 64 without use_preview_scale -> explicit error (both confirm=false and confirm=true)
        for confirm in [false, true] {
            let err = execute_gee_chunked_download(
                &tmp,
                &GeeChunkedDownloadParams {
                    asset_id: Some("COPERNICUS/S2_SR_HARMONIZED".into()),
                    bbox: Some([117.0, 31.5, 117.8, 32.2]),
                    scale_m: Some(10.0),
                    confirm_full_resolution: confirm,
                    use_preview_scale: false,
                    ..Default::default()
                },
            )
            .expect_err(">64 chunks without use_preview_scale must error explicitly");
            assert!(
                err.contains("GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD"),
                "{err}"
            );
            assert!(err.contains("use_preview_scale: true"), "{err}");
        }

        // Unconfigured `gee_quota_status` route returns honest unconfigured payload and passes `safe_value` verbatim
        let unconf_quota = dispatch_gee_geodata_query(
            &tmp,
            "gee_quota_status",
            None,
            None,
            None,
            None,
            None,
            Some(&json!({"refresh": true})),
        )
        .expect("unconfigured gee_quota_status");
        assert_eq!(unconf_quota["ok"], true);
        assert_eq!(unconf_quota["configured"], false);
        assert_eq!(unconf_quota["account_tier"], "unconfigured");
        assert_eq!(unconf_quota["cloud_monitoring"]["unlocked"], false);
        assert_eq!(unconf_quota["cloud_monitoring"]["reason"], "unconfigured");
        let sanitized_unconf = sanitize_for_agent_trace(&unconf_quota);
        assert_eq!(sanitized_unconf, unconf_quota);
        assert_eq!(
            insar_agent::run::safe_value(&unconf_quota).expect("unconfigured quota safe_value"),
            unconf_quota
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unit_non_json_429_and_transient_502_retry_and_direct_proxy_policy() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let tmp = std::env::temp_dir().join(format!(
            "eostage-gee-http-resilience-{}-{}",
            std::process::id(),
            now_epoch_secs()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // 1. Proxy policy resolution (`direct` vs `manual`)
        fs::write(
            tmp.join("network-settings.json"),
            r#"{"mode":"direct","https_proxy":"http://127.0.0.1:9999"}"#,
        )
        .unwrap();
        assert_eq!(resolve_effective_gee_proxy(&tmp), EffectiveGeeProxy::Direct);
        assert_eq!(detect_effective_proxy_url(&tmp), None);

        fs::write(
            tmp.join("network-settings.json"),
            r#"{"mode":"manual","https_proxy":"127.0.0.1:7890"}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_effective_gee_proxy(&tmp),
            EffectiveGeeProxy::ProxyUrl("http://127.0.0.1:7890".to_owned())
        );

        // Verify `ApplicationNetworkSettings` schema (`proxy_enabled` + `proxy_url`, no `mode` field)
        fs::write(
            tmp.join("network-settings.json"),
            r#"{"proxy_enabled":false,"proxy_url":"http://127.0.0.1:7890"}"#,
        )
        .unwrap();
        assert_eq!(resolve_effective_gee_proxy(&tmp), EffectiveGeeProxy::Direct);
        assert_eq!(detect_effective_proxy_url(&tmp), None);

        fs::write(
            tmp.join("network-settings.json"),
            r#"{"proxy_enabled":true,"proxy_url":"http://127.0.0.1:17890"}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_effective_gee_proxy(&tmp),
            EffectiveGeeProxy::ProxyUrl("http://127.0.0.1:17890".to_owned())
        );
        assert_eq!(
            detect_effective_proxy_url(&tmp).as_deref(),
            Some("http://127.0.0.1:17890")
        );

        // Reset to `direct` so loopback HTTP server tests bypass any host proxy deterministically
        fs::write(tmp.join("network-settings.json"), r#"{"mode":"direct"}"#).unwrap();
        let client = build_gee_http_client(&tmp, 10).expect("direct client builds");

        // 2. Non-JSON HTML `HTTP 429` MUST record `parallelism_restricted = true` & `http_requests = 1`
        let listener_429 = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr_429 = listener_429.local_addr().unwrap();
        let srv_429 = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener_429.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "<html><body>429 Too Many Requests</body></html>";
                let resp = format!(
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let mut tracker_429 = GeeCallTracker::new();
        let err_429 = gee_post_json(
            &client,
            "proj-demo",
            "tok-demo",
            &format!("http://{addr_429}/v1/projects/proj-demo/value:compute"),
            &json!({"expression": {}}),
            &mut tracker_429,
        )
        .expect_err("non-JSON 429 must fail with GEE_API_ERROR");
        assert!(err_429.contains("GEE_API_ERROR (HTTP 429)"), "{err_429}");
        assert_eq!(tracker_429.http_requests, 1);
        assert_eq!(tracker_429.saw_rate_limit_or_restriction, Some(true));
        finalize_gee_tracker(&tmp, None, GeeLedgerOpKind::AnalysisRun, false, tracker_429);
        let unconf_after_429 = dispatch_gee_geodata_query(
            &tmp,
            "gee_quota_status",
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(unconf_after_429["parallelism_restricted"], true);
        assert_eq!(unconf_after_429["local_ledger"]["total_requests"], 1);
        let _ = srv_429.join();

        // 3. Transient non-JSON `HTTP 502` followed by `HTTP 200` automatically retries & succeeds
        let listener_502 = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr_502 = listener_502.local_addr().unwrap();
        let srv_502 = std::thread::spawn(move || {
            for idx in 0..2 {
                if let Ok((mut stream, _)) = listener_502.accept() {
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    if idx == 0 {
                        let body = "Bad Gateway";
                        let resp = format!(
                            "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(resp.as_bytes());
                    } else {
                        let body = r#"{"result":42}"#;
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(resp.as_bytes());
                    }
                }
            }
        });

        let mut tracker_502 = GeeCallTracker::new();
        let ok_after_502 = gee_post_json(
            &client,
            "proj-demo",
            "tok-demo",
            &format!("http://{addr_502}/v1/projects/proj-demo/value:compute"),
            &json!({"expression": {}}),
            &mut tracker_502,
        )
        .expect("transient 502 recovers on retry");
        assert_eq!(ok_after_502["result"], 42);
        assert_eq!(tracker_502.http_requests, 2);
        let _ = srv_502.join();

        let _ = fs::remove_dir_all(&tmp);
    }

    struct TempDirGuard(PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    #[ignore = "requires EOSTAGE_GEE_LIVE_APP_DATA_DIR"]
    fn live_gee_cloud_suite_end_to_end_if_vault_present() {
        let raw_vault_dir = std::env::var("EOSTAGE_GEE_LIVE_APP_DATA_DIR")
            .expect("EOSTAGE_GEE_LIVE_APP_DATA_DIR must be set to run ignored live GEE E2E test");
        let src_app_dir = PathBuf::from(raw_vault_dir.trim());
        let src_vault = src_app_dir.join("credentials-vault.v1.json");
        assert!(
            src_vault.is_file(),
            "credentials-vault.v1.json not found in EOSTAGE_GEE_LIVE_APP_DATA_DIR"
        );

        let tmp_out = std::env::temp_dir().join(format!(
            "eostage-gee-live-e2e-{}-{}",
            std::process::id(),
            now_epoch_secs()
        ));
        let _ = fs::remove_dir_all(&tmp_out);
        fs::create_dir_all(&tmp_out).unwrap();
        let _cleanup_guard = TempDirGuard(tmp_out.clone());

        // Decrypt source vault (if AES-256-GCM encrypted) and materialize into
        // the isolated RAII temp dir so it decrypts/reads independently of the source path.
        let decrypted_vault = insar_download::read_local_vault_map_readonly(&src_vault)
            .expect("read source vault")
            .expect("source vault non-empty");
        let dst_vault = tmp_out.join("credentials-vault.v1.json");
        let serialized_vault = serde_json::to_vec_pretty(&decrypted_vault).unwrap();
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&dst_vault)
                .unwrap();
            file.write_all(&serialized_vault).unwrap();
        }
        #[cfg(not(unix))]
        {
            fs::write(&dst_vault, &serialized_vault).unwrap();
        }
        let mat = load_saved_gee_credentials(&tmp_out)
            .expect("parse live GEE credentials")
            .expect("live GEE credentials present");
        assert!(!mat.project_id.trim().is_empty());

        // 1. Live Verify
        let ver = verify_gee_account(&tmp_out, None, None).expect("live GEE verify");
        assert_eq!(ver["verified"], true);

        // 2. Live Pre-Flight Availability Check (Hefei Chaohu Lake north shore)
        let hefei_bbox = [117.26, 31.81, 117.31, 31.85];
        let avail = check_gee_availability(
            &tmp_out,
            "COPERNICUS/S2_SR_HARMONIZED",
            hefei_bbox,
            Some("2025-05-01"),
            Some("2025-09-30"),
            Some(30.0),
            Some(10.0),
        )
        .expect("live GEE check_availability");
        assert!(avail["total_scenes"].as_u64().unwrap() > 0);
        assert!(avail["cloud_filtered_scenes"].as_u64().unwrap() > 0);

        // 3. Live Cloud NDVI + Threshold Area (NDVI > 0.3 in km²) + PNG Preview + XYZ Map Tiles
        let ndvi_res = execute_gee_cloud_analyze(
            &tmp_out,
            &GeeCloudAnalyzeParams {
                asset_id: Some("COPERNICUS/S2_SR_HARMONIZED".into()),
                operation: Some("index".into()),
                index_name: Some("NDVI".into()),
                reducer: Some("median".into()),
                bbox: Some(hefei_bbox),
                start_date: Some("2025-05-01".into()),
                end_date: Some("2025-09-30".into()),
                max_cloud: Some(30.0),
                scale_m: Some(20.0),
                threshold: Some(0.3),
                threshold_op: Some("gt".into()),
                ..Default::default()
            },
        )
        .expect("live GEE cloud NDVI + area");
        assert_eq!(ndvi_res["ok"], true);
        assert!(ndvi_res["stats"]["mean"].as_f64().is_some());
        assert!(ndvi_res["stats"]["threshold_area_km2"].as_f64().unwrap() >= 0.0);
        assert!(ndvi_res["preview_data_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(ndvi_res["tile_url_template"]
            .as_str()
            .unwrap()
            .contains("/tiles/{z}/{x}/{y}"));

        // Verify Agent-safe projection passes `insar_agent::run::safe_value`!
        let agent_safe = sanitize_for_agent_trace(&ndvi_res);
        assert!(insar_agent::run::safe_value(&agent_safe).is_ok());

        // 3b. Live Time Series with `step_days` + stored artifact!
        let ts_res = execute_gee_cloud_analyze(
            &tmp_out,
            &GeeCloudAnalyzeParams {
                asset_id: Some("COPERNICUS/S2_SR_HARMONIZED".into()),
                operation: Some("time_series".into()),
                index_name: Some("NDVI".into()),
                reducer: Some("median".into()),
                bbox: Some(hefei_bbox),
                start_date: Some("2025-05-01".into()),
                end_date: Some("2025-09-01".into()),
                max_cloud: Some(30.0),
                scale_m: Some(30.0),
                step_days: Some(65),
                ..Default::default()
            },
        )
        .expect("live GEE time_series");
        assert_eq!(ts_res["ok"], true);
        assert!(ts_res["artifact_id"]
            .as_str()
            .unwrap()
            .starts_with("gee-art-"));
        assert_eq!(ts_res["series"].as_array().unwrap().len(), 2);
        let ts_safe = sanitize_for_agent_trace(&ts_res);
        assert!(insar_agent::run::safe_value(&ts_safe).is_ok());

        // 4. Live Multi-Chunk (2x1 or 2x2) Grid Download + Local Seamless GeoTIFF Stitching!
        // At 10m over 0.06° x 0.055° (~5.7km x 6.1km -> ~570x612 px -> 2x2 = 4 chunks!)
        let multi_bbox = [117.25, 31.80, 117.31, 31.855];
        let dl_tif = tmp_out.join("hefei_ndvi_10m_stitched.tif");
        let dl_res = execute_gee_chunked_download(
            &tmp_out,
            &GeeChunkedDownloadParams {
                asset_id: Some("COPERNICUS/S2_SR_HARMONIZED".into()),
                index_name: Some("NDVI".into()),
                reducer: Some("median".into()),
                bbox: Some(multi_bbox),
                start_date: Some("2025-05-01".into()),
                end_date: Some("2025-09-30".into()),
                max_cloud: Some(30.0),
                scale_m: Some(10.0),
                output_path: Some(dl_tif.to_string_lossy().to_string()),
                confirm_full_resolution: false,
                use_preview_scale: false,
                bands: None,
            },
        )
        .expect("live 2x2 chunked GEE GeoTIFF download");
        assert_eq!(dl_res["ok"], true);
        assert_eq!(dl_res["quota_confirm_required"], false);
        assert!(dl_res["chunks_stitched"].as_u64().unwrap() >= 2);
        assert!(dl_tif.is_file());
        assert!(dl_res["bytes_written"].as_u64().unwrap() > 500_000);

        // 5. Live Account Tier + Consumer Quota Cap + Per-Request EECU Profiling Ledger!
        let quota_res = dispatch_gee_geodata_query(
            &tmp_out,
            "gee_quota_status",
            None,
            None,
            None,
            None,
            None,
            Some(&json!({"refresh": true})),
        )
        .expect("live GEE quota status");
        assert_eq!(quota_res["ok"], true);
        assert_eq!(quota_res["configured"], true);
        assert_eq!(quota_res["project_id"], mat.project_id);
        assert_eq!(
            quota_res["registration_state"],
            "REGISTERED_NOT_COMMERCIALLY"
        );
        assert_eq!(quota_res["account_tier"], "noncommercial_community");
        assert_eq!(quota_res["account_tier_label"], "非商业科研档 (Community)");
        assert_eq!(quota_res["monthly_limit_eecu_seconds"], 540_000.0);
        assert_eq!(quota_res["monthly_limit_eecu_hours"], 150.0);
        assert!(quota_res["daily_limit_eecu_seconds"].is_null());
        assert_eq!(quota_res["read_requests_per_minute"], 6000);

        // Verify local monthly compute ledger recorded real requests + real Google EECU-seconds!
        let ledger = &quota_res["local_ledger"];
        assert!(ledger["total_requests"].as_u64().unwrap() >= 8);
        assert!(ledger["analysis_runs"].as_u64().unwrap() >= 2);
        assert!(ledger["chunk_downloads"].as_u64().unwrap() >= 1);
        assert_eq!(ledger["has_eecu_profile"], true);
        assert!(
            ledger["profiled_eecu_seconds"].as_f64().unwrap() > 0.0,
            "expected positive profiled_eecu_seconds from Profile.getProfiles, got {ledger}"
        );
        assert!(ledger["total_wall_seconds"].as_f64().unwrap() > 0.0);
        assert!(tmp_out.join("gee_usage_ledger.v1.json").is_file());

        // Verify ledger file contains zero secrets and zero project IDs
        let ledger_disk_str = fs::read_to_string(tmp_out.join("gee_usage_ledger.v1.json")).unwrap();
        assert!(!ledger_disk_str.contains(&mat.project_id));
        assert!(!ledger_disk_str.contains("private_key"));

        // Verify Agent trace projection preserves `gee_quota_status` verbatim and passes `safe_value`!
        let quota_safe = sanitize_for_agent_trace(&quota_res);
        assert_eq!(quota_safe, quota_res);
        assert_eq!(
            insar_agent::run::safe_value(&quota_safe).expect("live quota safe_value"),
            quota_res
        );
    }

    #[test]
    fn unit_planar_non_geodesic_rectangle_and_epsg3857_compute_pixels_grid() {
        // Wide Tibetan longitude span where geodesic rectangles noticeably bow convexly/concavely
        let tibet_bbox = [80.0, 28.0, 98.0, 36.0];
        let mut eb = ExprBuilder::new();
        let rect_node = eb.rectangle(tibet_bbox);
        let expr = eb.build(&rect_node);

        let root_id = expr["result"].as_str().unwrap();
        let rect_fn = &expr["values"][root_id]["functionInvocationValue"];
        assert_eq!(rect_fn["functionName"], "GeometryConstructors.Rectangle");

        let geo_ref = rect_fn["arguments"]["geodesic"]["valueReference"]
            .as_str()
            .unwrap();
        assert_eq!(
            expr["values"][geo_ref]["constantValue"],
            json!(false),
            "Rectangle geometry must explicitly set geodesic=false to prevent convex/concave E-W edge distortion"
        );

        let eo_ref = rect_fn["arguments"]["evenOdd"]["valueReference"]
            .as_str()
            .unwrap();
        assert_eq!(expr["values"][eo_ref]["constantValue"], json!(true));

        let crs_ref = rect_fn["arguments"]["crs"]["valueReference"]
            .as_str()
            .unwrap();
        assert_eq!(
            expr["values"][crs_ref]["functionInvocationValue"]["functionName"],
            "Projection"
        );
        let epsg_ref = expr["values"][crs_ref]["functionInvocationValue"]["arguments"]["crs"]
            ["valueReference"]
            .as_str()
            .unwrap();
        assert_eq!(
            expr["values"][epsg_ref]["constantValue"],
            json!("EPSG:4326")
        );

        // Verify computePixels PNG grid uses EPSG:3857 Web Mercator meters & affineTransform
        let palette = vec!["#000000".to_owned(), "#ffffff".to_owned()];
        let body = build_compute_pixels_png_body(&expr, tibet_bbox, -0.2, 0.85, &palette);
        assert_eq!(body["grid"]["crsCode"], json!("EPSG:3857"));
        let at = &body["grid"]["affineTransform"];
        let (xmin, _ymin) = lon_lat_to_epsg3857(80.0, 28.0);
        let (xmax, ymax) = lon_lat_to_epsg3857(98.0, 36.0);
        assert!((at["translateX"].as_f64().unwrap() - xmin).abs() < 1e-3);
        assert!((at["translateY"].as_f64().unwrap() - ymax).abs() < 1e-3);
        let w = body["grid"]["dimensions"]["width"].as_f64().unwrap();
        let h = body["grid"]["dimensions"]["height"].as_f64().unwrap();
        assert!(at["scaleX"].as_f64().unwrap() > 0.0);
        assert!(at["scaleY"].as_f64().unwrap() < 0.0);
        assert!(
            (at["translateX"].as_f64().unwrap() + w * at["scaleX"].as_f64().unwrap() - xmax).abs()
                < 1e-3
        );
        let (_, ymin) = lon_lat_to_epsg3857(80.0, 28.0);
        assert!(
            (at["translateY"].as_f64().unwrap() + h * at["scaleY"].as_f64().unwrap() - ymin).abs()
                < 1e-3
        );
    }

    #[test]
    fn unit_prepare_composite_vis_node_handles_2_band_sar_and_unknown_bands() {
        // Case A: Sentinel-1 GRD 2-band -> expands to 3-band false-color RGB (no palette)
        let mut eb2 = ExprBuilder::new();
        let src2 = eb2.constant(json!(1));
        let (node2, stat2, pal2) =
            prepare_composite_vis_node(&mut eb2, &src2, &["VV".into(), "VH".into()]);
        assert_eq!(stat2.as_deref(), Some("vis_r"));
        assert!(pal2.is_empty());
        let expr2 = eb2.build(&node2);
        let fn2 = &expr2["values"][node2]["functionInvocationValue"];
        assert_eq!(fn2["functionName"], "Image.select");
        let sel_ref = fn2["arguments"]["bandSelectors"]["valueReference"]
            .as_str()
            .unwrap();
        assert_eq!(
            expr2["values"][sel_ref]["constantValue"],
            json!(["VV", "VH", "VV"])
        );

        // Case B: Unknown dataset (0 known bands) -> selects index 0 renamed to BAND0 with palette
        let mut eb0 = ExprBuilder::new();
        let src0 = eb0.constant(json!(1));
        let (node0, stat0, pal0) = prepare_composite_vis_node(&mut eb0, &src0, &[]);
        assert_eq!(stat0.as_deref(), Some("BAND0"));
        assert!(!pal0.is_empty());
        let expr0 = eb0.build(&node0);
        let fn0 = &expr0["values"][node0]["functionInvocationValue"];
        assert_eq!(fn0["functionName"], "Image.select");
        let sel0_ref = fn0["arguments"]["bandSelectors"]["valueReference"]
            .as_str()
            .unwrap();
        assert_eq!(expr0["values"][sel0_ref]["constantValue"], json!([0]));
    }

    #[test]
    fn unit_static_dem_and_timeless_assets_skip_date_and_cloud_filters() {
        let cop_dem = lookup_dataset_profile("COPERNICUS/DEM/GLO30");
        let fabdem = lookup_dataset_profile("projects/sat-io/open-datasets/FABDEM");
        let s2 = lookup_dataset_profile("COPERNICUS/S2_SR_HARMONIZED");

        assert!(is_static_dem_or_timeless_asset(
            "COPERNICUS/DEM/GLO30",
            cop_dem.as_ref()
        ));
        assert!(is_static_dem_or_timeless_asset(
            "projects/sat-io/open-datasets/FABDEM",
            fabdem.as_ref()
        ));
        assert!(is_static_dem_or_timeless_asset("USGS/SRTMGL1_003", None));
        assert!(is_static_dem_or_timeless_asset("ESA/WorldCover/v200", None));
        assert!(!is_static_dem_or_timeless_asset(
            "COPERNICUS/S2_SR_HARMONIZED",
            s2.as_ref()
        ));

        // When timeless helper disables date/cloud args, filtered_collection emits NO Filter.dateRangeContains
        let mut eb_dem = ExprBuilder::new();
        let geom_dem = eb_dem.rectangle([117.2, 31.8, 117.3, 31.9]);
        let col_dem =
            eb_dem.filtered_collection("COPERNICUS/DEM/GLO30", &geom_dem, None, None, None, None);
        let built_dem = serde_json::to_string(&eb_dem.build(&col_dem)).unwrap();
        assert!(
            !built_dem.contains("Filter.dateRangeContains"),
            "Static DEM should not apply Filter.dateRangeContains: {built_dem}"
        );
        assert!(built_dem.contains("Filter.intersects"));

        // Non-timeless optical collection keeps Filter.dateRangeContains and Filter.lessThan
        let mut eb_s2 = ExprBuilder::new();
        let geom_s2 = eb_s2.rectangle([117.2, 31.8, 117.3, 31.9]);
        let col_s2 = eb_s2.filtered_collection(
            "COPERNICUS/S2_SR_HARMONIZED",
            &geom_s2,
            Some("2025-04-01"),
            Some("2025-06-30"),
            Some("CLOUDY_PIXEL_PERCENTAGE"),
            Some(30.0),
        );
        let built_s2 = serde_json::to_string(&eb_s2.build(&col_s2)).unwrap();
        assert!(built_s2.contains("Filter.dateRangeContains"));
        assert!(built_s2.contains("Filter.lessThan"));
    }

    #[test]
    fn test_build_collection_scenes_expr_uses_limit_and_aggregate_array() {
        let expr = build_collection_scenes_expr(
            "COPERNICUS/S2_SR_HARMONIZED",
            [117.2, 31.8, 117.3, 31.9],
            Some("2025-05-01"),
            Some("2025-09-30"),
            Some("CLOUDY_PIXEL_PERCENTAGE"),
            Some(25.0),
            None,
            None,
        );
        let serialized = serde_json::to_string(&expr).unwrap();
        assert!(serialized.contains("\"Collection.limit\""));
        assert!(serialized.contains("\"AggregateFeatureCollection.array\""));
        assert!(serialized.contains("\"dictionaryValue\""));
        assert!(serialized.contains("\"system:index\""));
        assert!(serialized.contains("\"system:time_start\""));
        assert!(serialized.contains("\"CLOUDY_PIXEL_PERCENTAGE\""));

        let expr_s1 = build_collection_scenes_expr(
            "COPERNICUS/S1_GRD",
            [116.68, 30.95, 117.97, 32.54],
            Some("2026-06-01"),
            Some("2026-11-16"),
            None,
            None,
            None,
            None,
        );
        let values = expr_s1["values"].as_object().unwrap();
        let limit_node = values
            .values()
            .find(|node| node["functionInvocationValue"]["functionName"] == "Collection.limit")
            .expect("Collection.limit node present");
        let limit_args = limit_node["functionInvocationValue"]["arguments"]
            .as_object()
            .unwrap();
        let mut arg_names: Vec<&str> = limit_args.keys().map(String::as_str).collect();
        arg_names.sort_unstable();
        assert_eq!(arg_names, vec!["ascending", "collection", "key", "limit"]);
        assert!(!limit_args.contains_key("max"));
        assert!(!limit_args.contains_key("property"));
    }

    #[test]
    fn unit_gem15_static_collection_catalog_and_dag_skips_date_filter() {
        let profile =
            lookup_dataset_profile("projects/climate-and-sustainability/assets/gem-15_v1")
                .expect("GEM-15 curated profile must exist in GEE Cloud Suite catalog");
        assert_eq!(profile.native_scale_m, 15.0);
        assert_eq!(profile.asset_type, "image_collection");
        assert_eq!(profile.default_bands, vec!["dtm", "dsm"]);

        let mut eb = ExprBuilder::new();
        let geom = eb.rectangle([-122.44, 37.75, -122.42, 37.77]);
        let col = eb.filtered_collection(
            &profile.id,
            &geom,
            Some("2026-01-01"),
            Some("2026-11-15"),
            profile.cloud_property.as_deref(),
            None,
        );
        let root = eb.composite_and_select_bands(&col, "mosaic", &["dtm".into()]);
        let dag = eb.build(&root);
        let dag_str = dag.to_string();
        assert!(
            dag_str.contains("Filter.intersects"),
            "spatial filter must remain present"
        );
        assert!(
            !dag_str.contains("Filter.dateRangeContains"),
            "static GEM-15 collection must skip temporal date filter: {dag_str}"
        );
    }

    #[test]
    fn unit_bbox_intersects_and_known_spatial_bounds_clcd() {
        assert!(bbox_intersects(
            [117.2, 31.8, 117.3, 31.9],
            [73.0, 18.0, 136.0, 54.0]
        ));
        assert!(!bbox_intersects(
            [-122.5, 37.7, -122.3, 37.8],
            [73.0, 18.0, 136.0, 54.0]
        ));
        let clcd_bounds = known_asset_spatial_bounds("projects/sat-io/open-datasets/CLCD")
            .expect("CLCD must have mainland China bounds");
        assert!(!bbox_intersects([-122.5, 37.7, -122.3, 37.8], clcd_bounds));
        assert!(bbox_intersects([117.2, 31.8, 117.3, 31.9], clcd_bounds));
    }

    #[test]
    fn unit_sar_collection_filter_dag_emits_polarization_and_orbit() {
        let mut eb = ExprBuilder::new();
        let geom = eb.rectangle([117.2, 31.8, 117.3, 31.9]);
        let raw = eb.filtered_collection(
            "COPERNICUS/S1_GRD",
            &geom,
            Some("2026-06-01"),
            Some("2026-11-16"),
            None,
            None,
        );
        let col = eb.apply_sar_filters(raw, "COPERNICUS/S1_GRD", Some("VH"), Some("DESCENDING"));
        let dag_str = eb.build(&col).to_string();
        assert!(dag_str.contains("Filter.listContains"));
        assert!(dag_str.contains("transmitterReceiverPolarisation"));
        assert!(dag_str.contains("Filter.equals"));
        assert!(dag_str.contains("orbitProperties_pass"));
        assert!(dag_str.contains("DESCENDING"));
    }

    #[test]
    fn unit_gee_chunked_download_executes_chunks_concurrently_and_merges_tracker() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let tmp = std::env::temp_dir().join(format!(
            "eostage-gee-parallel-chunks-{}-{}",
            std::process::id(),
            now_epoch_secs()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let _guard = TempDirGuard(tmp.clone());

        fs::write(tmp.join("network-settings.json"), r#"{"mode":"direct"}"#).unwrap();
        insar_download::save_transfer_engine_prefs_in_dir(
            &tmp,
            insar_download::TransferEnginePrefs {
                tile_chunk_concurrency: 4,
                ..insar_download::TransferEnginePrefs::default()
            },
        )
        .unwrap();

        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak_in_flight = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let in_flight_srv = Arc::clone(&in_flight);
        let peak_srv = Arc::clone(&peak_in_flight);
        let srv = std::thread::spawn(move || {
            let mut handles = Vec::new();
            for _ in 0..4 {
                if let Ok((mut stream, _)) = listener.accept() {
                    let cur_cnt = Arc::clone(&in_flight_srv);
                    let peak_cnt = Arc::clone(&peak_srv);
                    handles.push(std::thread::spawn(move || {
                        let cur = cur_cnt.fetch_add(1, Ordering::SeqCst) + 1;
                        peak_cnt.fetch_max(cur, Ordering::SeqCst);
                        let mut buf = [0u8; 4096];
                        let _ = stream.read(&mut buf);
                        std::thread::sleep(Duration::from_millis(80));
                        let req_txt = String::from_utf8_lossy(&buf);
                        let fill_val = if req_txt.contains("100.0") {
                            1.0_f32
                        } else {
                            2.0_f32
                        };
                        let npy = build_synthetic_npy_v1(
                            2,
                            2,
                            &[("B1", "<f4")],
                            &[fill_val, fill_val, fill_val, fill_val],
                        );
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nX-Earth-Engine-Quota-Status: parallelism_restricted=false\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            npy.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&npy);
                        let _ = stream.flush();
                        cur_cnt.fetch_sub(1, Ordering::SeqCst);
                    }));
                }
            }
            for h in handles {
                let _ = h.join();
            }
        });

        let client = build_gee_http_client(&tmp, 10).unwrap();
        let plan = ChunkGridPlan {
            bbox: [100.0, 30.0, 100.04, 30.04],
            requested_scale_m: 100.0,
            effective_scale_m: 100.0,
            full_width: 4,
            full_height: 4,
            chunk_size_px: 2,
            chunks_x: 2,
            chunks_y: 2,
            total_chunks: 4,
            scale_x_deg: 0.01,
            scale_y_deg: -0.01,
            estimated_mb: 0.01,
            quota_confirm_required: false,
            suggested_preview_scale_m: 100.0,
        };
        let mut tracker = GeeCallTracker::new();
        let (bands, names, stitched, count) = fetch_and_stitch_gee_chunks(
            &tmp,
            &client,
            "proj-test",
            "tok-test",
            &format!("http://{addr}/v1/projects/proj-test/image:computePixels"),
            &json!({"expression": {}}),
            &plan,
            &mut tracker,
        )
        .expect("parallel chunked download succeeds");
        let _ = srv.join();

        assert_eq!(bands, 1);
        assert_eq!(names, vec!["B1"]);
        assert_eq!(count, 4);
        assert_eq!(stitched.len(), 16);
        assert_eq!(tracker.http_requests, 4);
        assert_eq!(tracker.saw_rate_limit_or_restriction, Some(false));
        assert!(
            peak_in_flight.load(Ordering::SeqCst) > 1,
            "chunks must run concurrently (>1 in-flight), peak={}",
            peak_in_flight.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn unit_gee_chunked_download_propagates_429_rate_limit_tracker() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let tmp = std::env::temp_dir().join(format!(
            "eostage-gee-parallel-429-{}-{}",
            std::process::id(),
            now_epoch_secs()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let _guard = TempDirGuard(tmp.clone());

        fs::write(tmp.join("network-settings.json"), r#"{"mode":"direct"}"#).unwrap();
        insar_download::save_transfer_engine_prefs_in_dir(
            &tmp,
            insar_download::TransferEnginePrefs {
                tile_chunk_concurrency: 4,
                ..insar_download::TransferEnginePrefs::default()
            },
        )
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = std::thread::spawn(move || {
            for idx in 0..2 {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    if idx == 0 {
                        let body = "Too Many Requests";
                        let hdr = format!(
                            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: text/plain\r\nX-Earth-Engine-Quota-Status: parallelism_restricted=true\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                    } else {
                        let npy = build_synthetic_npy_v1(2, 2, &[("B1", "<f4")], &[1.0; 4]);
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            npy.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&npy);
                    }
                    let _ = stream.flush();
                }
            }
        });

        let client = build_gee_http_client(&tmp, 10).unwrap();
        let plan = ChunkGridPlan {
            bbox: [100.0, 30.0, 100.04, 30.02],
            requested_scale_m: 100.0,
            effective_scale_m: 100.0,
            full_width: 4,
            full_height: 2,
            chunk_size_px: 2,
            chunks_x: 2,
            chunks_y: 1,
            total_chunks: 2,
            scale_x_deg: 0.01,
            scale_y_deg: -0.01,
            estimated_mb: 0.01,
            quota_confirm_required: false,
            suggested_preview_scale_m: 100.0,
        };
        let mut tracker = GeeCallTracker::new();
        let err = fetch_and_stitch_gee_chunks(
            &tmp,
            &client,
            "proj-test",
            "tok-test",
            &format!("http://{addr}/v1/projects/proj-test/image:computePixels"),
            &json!({"expression": {}}),
            &plan,
            &mut tracker,
        )
        .expect_err("429 chunk must fail chunked download");
        let _ = srv.join();

        assert!(err.contains("HTTP 429"), "unexpected error: {err}");
        assert_eq!(tracker.saw_rate_limit_or_restriction, Some(true));
        assert!(tracker.http_requests >= 1);
    }

    #[test]
    fn unit_export_preflight_tile_safety_legacy_artifact_and_trace_hardening() {
        let tmp =
            std::env::temp_dir().join(format!("eostage-gee-preflight-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("gee_artifacts")).unwrap();

        // 1. Missing bbox does NOT silently fall back to Hefei
        let missing_bbox = plan_gee_export_preflight(
            &tmp,
            &GeeExportPreflightParams {
                asset_id: Some("COPERNICUS/S2_SR_HARMONIZED".into()),
                start_date: Some("2025-05-01".into()),
                end_date: Some("2025-09-01".into()),
                ..Default::default()
            },
        );
        assert_eq!(missing_bbox["ok"], true);
        assert_eq!(missing_bbox["ready"], false);
        assert_eq!(missing_bbox["recommended_route"], "needs_parameters");
        assert!(missing_bbox["missing_parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "bbox"));

        // 2. Small AOI -> local_chunked_geotiff_auto
        let auto_pre = plan_gee_export_preflight(
            &tmp,
            &GeeExportPreflightParams {
                asset_id: Some("COPERNICUS/S2_SR_HARMONIZED".into()),
                bbox: Some([117.25, 31.82, 117.28, 31.85]),
                start_date: Some("2025-05-01".into()),
                end_date: Some("2025-09-01".into()),
                scale_m: Some(10.0),
                ..Default::default()
            },
        );
        assert_eq!(auto_pre["ready"], true);
        assert_eq!(auto_pre["recommended_route"], "local_chunked_geotiff_auto");
        assert_eq!(auto_pre["quota_confirm_required"], false);
        assert_eq!(auto_pre["data_layer_record"]["dataset_source"], "official");

        // 3. Medium AOI -> local_chunked_geotiff_confirm
        let mid_pre = plan_gee_export_preflight(
            &tmp,
            &GeeExportPreflightParams {
                asset_id: Some("COPERNICUS/S2_SR_HARMONIZED".into()),
                bbox: Some([117.10, 31.70, 117.31, 31.90]),
                start_date: Some("2025-05-01".into()),
                end_date: Some("2025-09-01".into()),
                scale_m: Some(10.0),
                ..Default::default()
            },
        );
        assert_eq!(
            mid_pre["recommended_route"],
            "local_chunked_geotiff_confirm"
        );
        assert_eq!(mid_pre["quota_confirm_required"], true);

        // 4. Continental AOI reports unclamped dimensions (>32768) & routes to external_batch_export
        let huge_pre = plan_gee_export_preflight(
            &tmp,
            &GeeExportPreflightParams {
                asset_id: Some("COPERNICUS/S2_SR_HARMONIZED".into()),
                bbox: Some([100.0, 20.0, 125.0, 45.0]),
                start_date: Some("2025-05-01".into()),
                end_date: Some("2025-09-01".into()),
                scale_m: Some(10.0),
                ..Default::default()
            },
        );
        assert!(huge_pre["raw_width_px"].as_u64().unwrap() > 32_768);
        assert!(huge_pre["raw_total_chunks"].as_u64().unwrap() > 64);
        assert_eq!(huge_pre["recommended_route"], "external_batch_export");

        // 5. tile_unsafe algorithm over >1 chunk routes to external_batch_export
        let unsafe_pre = plan_gee_export_preflight(
            &tmp,
            &GeeExportPreflightParams {
                asset_id: Some("COPERNICUS/S2_SR_HARMONIZED".into()),
                bbox: Some([117.10, 31.70, 117.31, 31.90]),
                start_date: Some("2025-05-01".into()),
                end_date: Some("2025-09-01".into()),
                scale_m: Some(10.0),
                algorithm_kind: Some("pca_rsei".into()),
                ..Default::default()
            },
        );
        assert_eq!(unsafe_pre["tile_safety"], "tile_unsafe");
        assert_eq!(unsafe_pre["recommended_route"], "external_batch_export");

        // 6. Legacy GeeArtifactRecord JSON on disk without `data_layer_record` still deserializes
        let legacy_json = json!({
            "artifact_id": "gee-art-legacy001",
            "title": "Legacy Layer",
            "dataset_id": "COPERNICUS/S2_SR_HARMONIZED",
            "operation": "index",
            "bbox": [117.2, 31.8, 117.3, 31.9],
            "start_date": "2025-05-01",
            "end_date": "2025-09-01",
            "preview_data_url": null,
            "before_preview_data_url": null,
            "after_preview_data_url": null,
            "tile_url_template": null,
            "before_tile_url_template": null,
            "after_tile_url_template": null,
            "output_path": null,
            "size_bytes": null,
            "vis_min": -1.0,
            "vis_max": 1.0,
            "palette": ["#ffffff"],
            "unit": "NDVI",
            "stats": {},
            "created_at": 1700000000
        });
        fs::write(
            tmp.join("gee_artifacts").join("gee-art-legacy001.json"),
            serde_json::to_vec(&legacy_json).unwrap(),
        )
        .unwrap();
        let loaded_legacy = load_gee_artifact(&tmp, "gee-art-legacy001")
            .expect("legacy artifact without data_layer_record must deserialize");
        assert!(loaded_legacy.data_layer_record.is_none());

        // 7. Sanitization blocks gs://, cookie, docs, query strings & passes safe_value
        let dirty = json!({
            "ok": true,
            "bucket": "gs://private-bucket/path",
            "cookie": "session=abc",
            "docs": "https://example.org/doc?a=1",
            "prose_url": "https://example.org/item?token=secret#frag",
            "preflight": huge_pre,
        });
        let cleaned = sanitize_for_agent_trace(&dirty);
        assert!(cleaned.get("cookie").is_none());
        assert!(cleaned.get("docs").is_none());
        assert_eq!(cleaned["bucket"], "[local]");
        assert_eq!(cleaned["prose_url"], "[redacted]");
        insar_agent::run::safe_value(&cleaned)
            .expect("hardened sanitized payload must pass safe_value");

        // 8. Dispatch routes `gee_dataset_advisor` (recommend/compare/verify) and `gee_export_preflight`
        let adv_rec = dispatch_gee_geodata_query(
            &tmp,
            "gee_dataset_advisor",
            None,
            None,
            None,
            None,
            None,
            Some(&json!({"mode": "recommend", "task": "flood risk"})),
        )
        .expect("advisor recommend");
        assert_eq!(adv_rec["recipe"], "flood-risk");

        let adv_cmp = dispatch_gee_geodata_query(
            &tmp,
            "gee_dataset_advisor",
            None,
            None,
            None,
            None,
            None,
            Some(&json!({"mode": "compare", "ids": ["COPERNICUS/DEM/GLO30", "USGS/SRTMGL1_003"]})),
        )
        .expect("advisor compare");
        assert_eq!(adv_cmp["datasets"].as_array().unwrap().len(), 2);

        let adv_ver = dispatch_gee_geodata_query(
            &tmp,
            "gee_dataset_advisor",
            None,
            None,
            None,
            None,
            None,
            Some(&json!({"mode": "verify", "asset_id": "COPERNICUS/S2_SR_HARMONIZED", "live": false})),
        )
        .expect("advisor verify");
        assert_eq!(adv_ver["catalog_found"], true);

        let _ = fs::remove_dir_all(&tmp);
    }
}
