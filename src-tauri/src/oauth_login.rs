//! Native Rust key-free OAuth login (Device Code, Loopback PKCE S256),
//! OAuth token auto-refresh,
//! and public model registry sync (`models.dev` + OpenRouter).
//!
//! Secrets (API keys, OAuth access/refresh tokens) remain strictly inside Rust
//! and the OS credential store (`insar-download`); WebView IPC only receives
//! non-secret status and channel metadata.

use base64::Engine as _;
use insar_agent_model::channels::{ChannelConfig, ChannelStore};
use insar_agent_model::transport::ModelRoute;
use insar_agent_model::ReasoningMode;
use insar_agent_model::Zeroizing;
use insar_application_service::{AgentApplication, NetworkProxyMode};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agent::DesktopKeys;

const OPENAI_CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const PKCE_CODEX_PRIMARY_PORT: u16 = 1455;
const CLAUDE_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_PKCE_PORT: u16 = 54545;
const CLAUDE_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const CLAUDE_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const GITHUB_COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_COPILOT_USER_URL: &str = "https://api.github.com/copilot_internal/user";
const KIMI_CODE_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const EOSTAGE_OAUTH_ENVELOPE_KIND: &str = "eostage_oauth_v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthKeyEnvelope {
    pub kind: String,
    pub provider: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at_ms: Option<u64>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub copilot_api_token: Option<String>,
    #[serde(default)]
    pub copilot_expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct DeviceAuthSession {
    provider_id: String,
    device_code: String,
    user_code: String,
    expires_at_ms: u64,
}

#[derive(Debug, Clone)]
struct PkceAuthSession {
    provider_id: String,
    code_verifier: String,
    state: String,
    redirect_uri: String,
    captured_code: Option<String>,
    captured_error: Option<String>,
    exchange_in_progress: bool,
    completed_result: Option<OAuthPkceCompleteResult>,
    expires_at_ms: u64,
}

static DEVICE_SESSIONS: OnceLock<Mutex<HashMap<String, DeviceAuthSession>>> = OnceLock::new();
static PKCE_SESSIONS: OnceLock<Mutex<HashMap<String, PkceAuthSession>>> = OnceLock::new();
static PKCE_LISTENER_GENERATION: AtomicU64 = AtomicU64::new(0);
static PKCE_LISTENER_ACTIVE: AtomicBool = AtomicBool::new(false);
static PKCE_LISTENER_PORT: AtomicU16 = AtomicU16::new(0);

pub fn has_active_pkce_listener() -> bool {
    PKCE_LISTENER_ACTIVE.load(Ordering::SeqCst)
}

pub fn has_recent_or_active_native_pkce_session() -> bool {
    if has_active_pkce_listener() {
        return true;
    }
    let now = now_ms();
    pkce_sessions()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .any(|s| s.expires_at_ms > now && s.completed_result.is_none())
}

fn device_sessions() -> &'static Mutex<HashMap<String, DeviceAuthSession>> {
    DEVICE_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pkce_sessions() -> &'static Mutex<HashMap<String, PkceAuthSession>> {
    PKCE_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn random_base64url(num_bytes: usize) -> String {
    let mut bytes = vec![0u8; num_bytes];
    let rng = SystemRandom::new();
    let _ = rng.fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn build_http_client(
    application: &AgentApplication,
    timeout: Duration,
) -> Result<reqwest::blocking::Client, String> {
    let settings = application
        .0
        .get_network_settings()
        .map_err(|_| "网络配置不可用".to_owned())?;
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent("EOStage/0.1.0");
    builder = match settings.proxy_mode() {
        NetworkProxyMode::Auto => builder,
        NetworkProxyMode::Direct => builder.no_proxy(),
        NetworkProxyMode::Manual => {
            let proxy_url = settings.proxy_url.trim();
            if proxy_url.is_empty() {
                builder
            } else {
                let proxy =
                    reqwest::Proxy::all(proxy_url).map_err(|e| format!("代理地址无效: {e}"))?;
                builder.no_proxy().proxy(proxy)
            }
        }
    };
    builder
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))
}

#[derive(Debug, Clone, Serialize)]
pub struct OAuthDeviceStartResult {
    pub session_id: String,
    pub provider_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OAuthDevicePollResult {
    pub status: String, // "pending" | "authorized" | "expired" | "error"
    pub channel_id: Option<String>,
    pub account_label: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OAuthPkceStartResult {
    pub session_id: String,
    pub provider_id: String,
    pub authorize_url: String,
    pub redirect_uri: String,
    pub state: String,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OAuthPkceCompleteResult {
    pub status: String, // "pending" | "authorized" | "expired" | "error"
    pub channel_id: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedPublicModelSpec {
    pub id: String,
    pub provider: Option<String>,
    pub name: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    pub supports_tools: bool,
    pub supports_image: bool,
    pub supports_video: bool,
    pub supports_pdf: bool,
    pub reasoning: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicModelRegistrySyncResult {
    pub items: Vec<NormalizedPublicModelSpec>,
    pub source_summary: String,
}

fn extract_chatgpt_account_id_from_jwt(jwt: &str) -> Option<String> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(parts[1]))
        .ok()?;
    let payload: Value = serde_json::from_slice(&decoded).ok()?;
    payload
        .get("https://api.openai.com/auth")
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("chatgpt_account_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn extract_jwt_exp_ms(jwt: &str) -> Option<u64> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(parts[1]))
        .ok()?;
    let payload: Value = serde_json::from_slice(&decoded).ok()?;
    payload
        .get("exp")
        .and_then(Value::as_u64)
        .map(|sec| sec.saturating_mul(1000))
}

fn upsert_channel_config(
    data_root: &Path,
    channel: ChannelConfig,
    secret: &str,
) -> Result<(), String> {
    upsert_channel_config_with(data_root, channel, secret, &DesktopKeys)
}

fn upsert_channel_config_with(
    data_root: &Path,
    channel: ChannelConfig,
    secret: &str,
    keys: &impl insar_agent_model::channels::ChannelKeys,
) -> Result<(), String> {
    let store = ChannelStore::new(data_root.join("agent"));
    let existing_default = store.default_id().ok().flatten();
    let existing_channels = store.list(keys).unwrap_or_default();
    store
        .save(channel.clone(), Some(secret), keys)
        .map_err(|_| "保存模型渠道及凭据失败".to_owned())?;
    let has_configured_model = !channel.model.trim().is_empty() || !channel.models.is_empty();
    if existing_default.is_none() && existing_channels.is_empty() && has_configured_model {
        let _ = store.set_default(&channel.id);
    }
    Ok(())
}

fn openai_codex_channel_config() -> ChannelConfig {
    ChannelConfig {
        id: "openai-codex-oauth".into(),
        protocol: "responses".into(),
        base_url: "https://chatgpt.com/backend-api/codex".into(),
        model: String::new(),
        reasoning_mode: ReasoningMode::ProviderDefault,
        reasoning_switch_unsupported: false,
        models: Vec::new(),
    }
}

fn github_copilot_channel_config() -> ChannelConfig {
    ChannelConfig {
        id: "github-copilot-oauth".into(),
        protocol: "chat_completions".into(),
        base_url: "https://api.githubcopilot.com".into(),
        model: String::new(),
        reasoning_mode: ReasoningMode::ProviderDefault,
        reasoning_switch_unsupported: false,
        models: Vec::new(),
    }
}

fn claude_oauth_channel_config() -> ChannelConfig {
    ChannelConfig {
        id: "claude-oauth".into(),
        protocol: "anthropic_messages".into(),
        base_url: "https://api.anthropic.com".into(),
        model: String::new(),
        reasoning_mode: ReasoningMode::ProviderDefault,
        reasoning_switch_unsupported: false,
        models: Vec::new(),
    }
}

fn openrouter_channel_config() -> ChannelConfig {
    ChannelConfig {
        id: "openrouter-oauth".into(),
        protocol: "chat_completions".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        model: String::new(),
        reasoning_mode: ReasoningMode::ProviderDefault,
        reasoning_switch_unsupported: false,
        models: Vec::new(),
    }
}

pub fn oauth_device_start(
    application: &AgentApplication,
    provider_id: &str,
) -> Result<OAuthDeviceStartResult, String> {
    let canonical = match provider_id.trim() {
        "openai_codex" | "openai_codex_pkce" => "openai_codex",
        "github_copilot" | "github_copilot_device" => "github_copilot",
        other => {
            return Err(format!(
                "不支持的设备授权平台: {other}（当前支持 OpenAI Codex、Claude、GitHub Copilot、OpenRouter 官方免密授权）"
            ));
        }
    };
    let client = build_http_client(application, Duration::from_secs(20))?;
    let session_id = format!("dev_{}", random_base64url(12));

    let (device_code, user_code, verification_uri, interval_secs, expires_in_secs) = match canonical
    {
        "openai_codex" => {
            let resp = client
                .post("https://auth.openai.com/api/accounts/deviceauth/usercode")
                .json(&serde_json::json!({ "client_id": OPENAI_CODEX_CLIENT_ID }))
                .send()
                .map_err(|e| format!("请求 OpenAI 设备授权码失败: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let text = resp.text().unwrap_or_default();
                return Err(format!("OpenAI 设备授权返回 HTTP {status}: {text}"));
            }
            let json: Value = resp
                .json()
                .map_err(|e| format!("解析 OpenAI 授权响应失败: {e}"))?;
            let device_auth_id = json["device_auth_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "OpenAI 未返回 device_auth_id".to_owned())?
                .to_owned();
            let user_code = json["user_code"]
                .as_str()
                .or_else(|| json["userCode"].as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "OpenAI 未返回一次性验证码 user_code".to_owned())?
                .to_owned();
            let interval = json["interval"]
                .as_u64()
                .or_else(|| {
                    json["interval"]
                        .as_str()
                        .and_then(|s| s.parse::<u64>().ok())
                })
                .unwrap_or(5)
                .clamp(3, 15);
            let expires = json["expires_in"].as_u64().unwrap_or(900);
            (
                device_auth_id,
                user_code,
                "https://auth.openai.com/codex/device".to_owned(),
                interval,
                expires,
            )
        }
        "github_copilot" => {
            let resp = client
                .post("https://github.com/login/device/code")
                .header("Accept", "application/json")
                .form(&[
                    ("client_id", GITHUB_COPILOT_CLIENT_ID),
                    ("scope", "read:user"),
                ])
                .send()
                .map_err(|e| format!("请求 GitHub Copilot 设备授权码失败: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let text = resp.text().unwrap_or_default();
                return Err(format!("GitHub 设备授权返回 HTTP {status}: {text}"));
            }
            let json: Value = resp
                .json()
                .map_err(|e| format!("解析 GitHub 设备授权响应失败: {e}"))?;
            let device_code = json["device_code"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "GitHub 未返回 device_code".to_owned())?
                .to_owned();
            let user_code = json["user_code"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "GitHub 未返回设备授权码 user_code".to_owned())?
                .to_owned();
            let verification_uri = json["verification_uri"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("https://github.com/login/device")
                .to_owned();
            let interval = json["interval"].as_u64().unwrap_or(5).clamp(5, 30);
            let expires = json["expires_in"].as_u64().unwrap_or(900);
            (device_code, user_code, verification_uri, interval, expires)
        }
        _ => unreachable!(),
    };

    let session = DeviceAuthSession {
        provider_id: canonical.to_owned(),
        device_code,
        user_code: user_code.clone(),
        expires_at_ms: now_ms().saturating_add(expires_in_secs * 1000),
    };

    let mut map = device_sessions().lock().unwrap_or_else(|p| p.into_inner());
    map.retain(|_, s| s.expires_at_ms > now_ms());
    map.insert(session_id.clone(), session);

    Ok(OAuthDeviceStartResult {
        session_id,
        provider_id: canonical.to_owned(),
        user_code,
        verification_uri,
        interval_secs,
        expires_in_secs,
    })
}

pub fn oauth_device_poll(
    application: &AgentApplication,
    session_id: &str,
) -> Result<OAuthDevicePollResult, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let session = {
        let map = device_sessions().lock().unwrap_or_else(|p| p.into_inner());
        map.get(session_id).cloned()
    }
    .ok_or_else(|| "授权会话不存在或已清理".to_owned())?;

    if now_ms() >= session.expires_at_ms {
        let mut map = device_sessions().lock().unwrap_or_else(|p| p.into_inner());
        map.remove(session_id);
        return Ok(OAuthDevicePollResult {
            status: "expired".into(),
            channel_id: None,
            account_label: None,
            message: Some("设备授权码已过期，请重新生成".into()),
        });
    }

    let client = build_http_client(application, Duration::from_secs(20))?;

    match session.provider_id.as_str() {
        "openai_codex" => {
            let poll_resp = client
                .post("https://auth.openai.com/api/accounts/deviceauth/token")
                .json(&serde_json::json!({
                    "device_auth_id": session.device_code,
                    "user_code": session.user_code,
                }))
                .send()
                .map_err(|e| format!("轮询 OpenAI 授权状态失败: {e}"))?;
            let status = poll_resp.status().as_u16();
            if status == 403 || status == 404 || status == 428 {
                return Ok(OAuthDevicePollResult {
                    status: "pending".into(),
                    channel_id: None,
                    account_label: None,
                    message: None,
                });
            }
            if !poll_resp.status().is_success() {
                return Ok(OAuthDevicePollResult {
                    status: "error".into(),
                    channel_id: None,
                    account_label: None,
                    message: Some(format!("OpenAI 设备轮询返回 HTTP {status}")),
                });
            }
            let code_payload: Value = poll_resp
                .json()
                .map_err(|e| format!("解析 OpenAI 授权凭证失败: {e}"))?;
            let authorization_code = code_payload["authorization_code"]
                .as_str()
                .ok_or_else(|| "OpenAI 响应缺少 authorization_code".to_owned())?;
            let code_verifier = code_payload["code_verifier"]
                .as_str()
                .ok_or_else(|| "OpenAI 响应缺少 code_verifier".to_owned())?;

            let token_resp = client
                .post("https://auth.openai.com/oauth/token")
                .form(&[
                    ("grant_type", "authorization_code"),
                    ("code", authorization_code),
                    (
                        "redirect_uri",
                        "https://auth.openai.com/deviceauth/callback",
                    ),
                    ("client_id", OPENAI_CODEX_CLIENT_ID),
                    ("code_verifier", code_verifier),
                ])
                .send()
                .map_err(|e| format!("兑换 OpenAI OAuth 令牌失败: {e}"))?;
            if !token_resp.status().is_success() {
                return Err(format!(
                    "OpenAI OAuth 令牌兑换失败 (HTTP {})",
                    token_resp.status().as_u16()
                ));
            }
            let tokens: Value = token_resp
                .json()
                .map_err(|e| format!("解析 OpenAI OAuth 令牌 JSON 失败: {e}"))?;
            let access_token = tokens["access_token"]
                .as_str()
                .ok_or_else(|| "响应缺少 access_token".to_owned())?
                .to_owned();
            let refresh_token = tokens["refresh_token"].as_str().map(str::to_owned);
            let expires_in = tokens["expires_in"].as_u64().unwrap_or(3600);
            let account_id = extract_chatgpt_account_id_from_jwt(&access_token).or_else(|| {
                tokens["id_token"]
                    .as_str()
                    .and_then(extract_chatgpt_account_id_from_jwt)
            });

            let envelope = OAuthKeyEnvelope {
                kind: EOSTAGE_OAUTH_ENVELOPE_KIND.into(),
                provider: "openai_codex".into(),
                access_token,
                refresh_token,
                expires_at_ms: Some(now_ms().saturating_add(expires_in * 1000)),
                account_id: account_id.clone(),
                copilot_api_token: None,
                copilot_expires_at_ms: None,
            };
            let secret_json = serde_json::to_string(&envelope).map_err(|e| e.to_string())?;
            let channel = openai_codex_channel_config();
            let channel_id = channel.id.clone();
            upsert_channel_config(&data_root, channel, &secret_json)?;

            let mut map = device_sessions().lock().unwrap_or_else(|p| p.into_inner());
            map.remove(session_id);

            Ok(OAuthDevicePollResult {
                status: "authorized".into(),
                channel_id: Some(channel_id),
                account_label: account_id,
                message: Some("已完成 ChatGPT / OpenAI 免 Key 登录并安全写入钥匙串".into()),
            })
        }
        "github_copilot" => {
            let token_resp = client
                .post("https://github.com/login/oauth/access_token")
                .header("Accept", "application/json")
                .form(&[
                    ("client_id", GITHUB_COPILOT_CLIENT_ID),
                    ("device_code", session.device_code.as_str()),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ])
                .send()
                .map_err(|e| format!("轮询 GitHub Copilot 授权状态失败: {e}"))?;
            let status = token_resp.status().as_u16();
            if !token_resp.status().is_success() {
                return Ok(OAuthDevicePollResult {
                    status: "error".into(),
                    channel_id: None,
                    account_label: None,
                    message: Some(format!("GitHub 设备轮询返回 HTTP {status}")),
                });
            }
            let payload: Value = token_resp
                .json()
                .map_err(|e| format!("解析 GitHub 设备授权响应失败: {e}"))?;
            if let Some(err_code) = payload["error"].as_str().filter(|s| !s.is_empty()) {
                match err_code {
                    "authorization_pending" | "slow_down" => {
                        return Ok(OAuthDevicePollResult {
                            status: "pending".into(),
                            channel_id: None,
                            account_label: None,
                            message: None,
                        });
                    }
                    "expired_token" => {
                        let mut map = device_sessions().lock().unwrap_or_else(|p| p.into_inner());
                        map.remove(session_id);
                        return Ok(OAuthDevicePollResult {
                            status: "expired".into(),
                            channel_id: None,
                            account_label: None,
                            message: Some("GitHub 设备授权码已过期，请重新生成".into()),
                        });
                    }
                    "access_denied" => {
                        let mut map = device_sessions().lock().unwrap_or_else(|p| p.into_inner());
                        map.remove(session_id);
                        return Ok(OAuthDevicePollResult {
                            status: "error".into(),
                            channel_id: None,
                            account_label: None,
                            message: Some("您已取消 GitHub Copilot 授权".into()),
                        });
                    }
                    _ => {
                        let detail = payload["error_description"].as_str().unwrap_or(err_code);
                        let mut map = device_sessions().lock().unwrap_or_else(|p| p.into_inner());
                        map.remove(session_id);
                        return Ok(OAuthDevicePollResult {
                            status: "error".into(),
                            channel_id: None,
                            account_label: None,
                            message: Some(format!("GitHub Copilot 授权失败: {detail}")),
                        });
                    }
                }
            }
            let access_token = payload["access_token"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| "GitHub 响应缺少 access_token".to_owned())?
                .to_owned();
            let envelope = OAuthKeyEnvelope {
                kind: EOSTAGE_OAUTH_ENVELOPE_KIND.into(),
                provider: "github_copilot".into(),
                access_token,
                refresh_token: None,
                expires_at_ms: None,
                account_id: None,
                copilot_api_token: None,
                copilot_expires_at_ms: None,
            };
            let secret_json = serde_json::to_string(&envelope).map_err(|e| e.to_string())?;
            let channel = github_copilot_channel_config();
            let channel_id = channel.id.clone();
            upsert_channel_config(&data_root, channel, &secret_json)?;

            let mut map = device_sessions().lock().unwrap_or_else(|p| p.into_inner());
            map.remove(session_id);

            Ok(OAuthDevicePollResult {
                status: "authorized".into(),
                channel_id: Some(channel_id),
                account_label: None,
                message: Some("已完成 GitHub Copilot 设备码授权并安全写入钥匙串".into()),
            })
        }
        other => Err(format!("不支持的设备授权平台: {other}")),
    }
}

struct PkceListenerGuard {
    generation: u64,
}

impl Drop for PkceListenerGuard {
    fn drop(&mut self) {
        if PKCE_LISTENER_GENERATION.load(Ordering::SeqCst) == self.generation {
            PKCE_LISTENER_ACTIVE.store(false, Ordering::SeqCst);
            PKCE_LISTENER_PORT.store(0, Ordering::SeqCst);
        }
    }
}

struct PreparedPkceListener {
    port: u16,
    generation: u64,
    listener_v4: Option<TcpListener>,
    listener_v6: Option<TcpListener>,
}

fn wake_loopback_port(port: u16) {
    if port == 0 {
        return;
    }
    let addr_v4 = SocketAddr::from(([127, 0, 0, 1], port));
    let _ = TcpStream::connect_timeout(&addr_v4, Duration::from_millis(60));
}

fn bind_loopback_v4_with_retry(port: u16, attempts: usize) -> Result<TcpListener, std::io::Error> {
    let mut last_err = std::io::Error::new(ErrorKind::AddrInUse, "port busy");
    for idx in 0..attempts {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => return Ok(listener),
            Err(err) => {
                let retryable = err.kind() == ErrorKind::AddrInUse;
                last_err = err;
                if !retryable || idx + 1 >= attempts {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        }
    }
    Err(last_err)
}

fn prepare_pkce_loopback_listener(preferred_ports: &[u16]) -> Result<PreparedPkceListener, String> {
    let prev_port = PKCE_LISTENER_PORT.load(Ordering::SeqCst);
    let generation = PKCE_LISTENER_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    PKCE_LISTENER_ACTIVE.store(false, Ordering::SeqCst);
    if prev_port != 0 {
        wake_loopback_port(prev_port);
    }

    let listener_v4 = if preferred_ports.is_empty() {
        TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("无法绑定本地回调端口: {e}"))?
    } else {
        let mut bound = None;
        let mut last_err = None;
        for &port in preferred_ports {
            match bind_loopback_v4_with_retry(port, 12) {
                Ok(l) => {
                    bound = Some(l);
                    break;
                }
                Err(err) => {
                    last_err = Some((port, err));
                }
            }
        }
        match bound {
            Some(l) => l,
            None => {
                let detail = last_err
                    .map(|(p, e)| format!("端口 {p}: {e}"))
                    .unwrap_or_else(|| "端口被占用".to_owned());
                let ports_hint = preferred_ports
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join("/");
                return Err(format!(
                    "无法启动本地 OAuth 回调监听（{detail}），请关闭占用 {ports_hint} 端口的程序后重试"
                ));
            }
        }
    };

    let port = listener_v4
        .local_addr()
        .map_err(|e| format!("读取本地回调端口失败: {e}"))?
        .port();
    let _ = listener_v4.set_nonblocking(true);
    let listener_v6 = TcpListener::bind(("::1", port)).ok();
    if let Some(ref l) = listener_v6 {
        let _ = l.set_nonblocking(true);
    }

    PKCE_LISTENER_PORT.store(port, Ordering::SeqCst);
    PKCE_LISTENER_ACTIVE.store(true, Ordering::SeqCst);

    Ok(PreparedPkceListener {
        port,
        generation,
        listener_v4: Some(listener_v4),
        listener_v6,
    })
}

fn spawn_prepared_pkce_listener(
    prepared: PreparedPkceListener,
    session_id: String,
    expected_state: String,
) {
    let PreparedPkceListener {
        generation,
        listener_v4,
        listener_v6,
        ..
    } = prepared;

    std::thread::spawn(move || {
        let _guard = PkceListenerGuard { generation };
        let deadline = std::time::Instant::now() + Duration::from_secs(600);
        while std::time::Instant::now() < deadline {
            if PKCE_LISTENER_GENERATION.load(Ordering::SeqCst) != generation {
                break;
            }
            {
                let map = pkce_sessions().lock().unwrap_or_else(|p| p.into_inner());
                let keep_listening = map.get(&session_id).is_some_and(|sess| {
                    sess.expires_at_ms > now_ms()
                        && sess.completed_result.is_none()
                        && sess.captured_code.is_none()
                        && sess.captured_error.is_none()
                });
                if !keep_listening {
                    break;
                }
            }

            let accepted = listener_v4
                .as_ref()
                .and_then(|l| l.accept().ok())
                .or_else(|| listener_v6.as_ref().and_then(|l| l.accept().ok()));

            match accepted {
                Some((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                    let mut buf = [0u8; 4096];
                    if let Ok(n) = stream.read(&mut buf) {
                        if n == 0 {
                            continue;
                        }
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let first_line = req.lines().next().unwrap_or("");
                        let Some(target) = first_line.split_whitespace().nth(1) else {
                            continue;
                        };
                        if target.starts_with("/favicon.ico") {
                            let _ = stream.write_all(
                                b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            );
                            continue;
                        }
                        let path_only = target.split(['?', '#']).next().unwrap_or(target);
                        if !path_only.starts_with("/auth/callback")
                            && !path_only.starts_with("/callback")
                        {
                            let _ = stream.write_all(
                                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            );
                            continue;
                        }

                        let parsed = extract_code_and_state(target);
                        let mut should_break = false;
                        let mut map = pkce_sessions().lock().unwrap_or_else(|p| p.into_inner());
                        let target_key = if map.contains_key(&session_id) {
                            Some(session_id.clone())
                        } else {
                            parsed.state.as_deref().and_then(|st| {
                                map.iter()
                                    .find(|(_, s)| s.state == st)
                                    .map(|(k, _)| k.clone())
                            })
                        };
                        let (status_line, html) = if let Some(sess) =
                            target_key.and_then(|k| map.get_mut(&k))
                        {
                            if let Some(err) = parsed.error {
                                sess.captured_error = Some(err);
                                should_break = true;
                                (
                                    "200 OK",
                                    "<!doctype html><html><head><meta charset=\"utf-8\"><title>EOStage 授权未完成</title></head><body style=\"font-family:system-ui,-apple-system,sans-serif;padding:32px\"><h3>EOStage 授权未完成</h3><p>您可以关闭此页面并返回桌面端重试。</p></body></html>",
                                )
                            } else if parsed.state.as_deref() == Some(sess.state.as_str())
                                || parsed.state.as_deref() == Some(expected_state.as_str())
                                || parsed.state.is_none()
                            {
                                if let Some(code) = parsed.code {
                                    sess.captured_code = Some(code);
                                    should_break = true;
                                    (
                                        "200 OK",
                                        "<!doctype html><html><head><meta charset=\"utf-8\"><title>EOStage 登录成功</title></head><body style=\"font-family:system-ui,-apple-system,sans-serif;padding:32px\"><h2>EOStage 登录授权成功</h2><p>凭据已在后台安全接收并自动完成兑换，您可以关闭此浏览器标签页返回桌面端。</p><script>setTimeout(function(){try{window.close();}catch(e){}},1200);</script></body></html>",
                                    )
                                } else {
                                    // Missing code (e.g., preflight probe): answer cleanly and keep listening.
                                    (
                                        "400 Bad Request",
                                        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body style=\"font-family:system-ui;padding:32px\"><h3>未检测到授权码</h3></body></html>",
                                    )
                                }
                            } else {
                                (
                                    "400 Bad Request",
                                    "<!doctype html><html><head><meta charset=\"utf-8\"></head><body style=\"font-family:system-ui;padding:32px\"><h3>State mismatch</h3></body></html>",
                                )
                            }
                        } else {
                            should_break = true;
                            (
                                "200 OK",
                                "<!doctype html><html><head><meta charset=\"utf-8\"></head><body style=\"font-family:system-ui;padding:32px\"><h3>授权会话已结束</h3></body></html>",
                            )
                        };
                        drop(map);
                        let resp = format!(
                            "HTTP/1.1 {status_line}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
                            html.len(),
                            html
                        );
                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.flush();
                        if should_break {
                            break;
                        }
                    }
                }
                None => {
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
    });
}

#[derive(Default, Debug, PartialEq, Eq)]
struct CallbackQueryParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                    if let Ok(val) = u8::from_str_radix(hex, 16) {
                        out.push(val);
                        i += 3;
                        continue;
                    }
                }
                out.push(bytes[i]);
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn extract_code_and_state(input: &str) -> CallbackQueryParams {
    let trimmed = input.trim();
    let mut params = CallbackQueryParams::default();
    if trimmed.is_empty() {
        return params;
    }

    let raw_query = if let Some((_, q)) = trimmed.split_once('?') {
        q
    } else if let Some((_, f)) = trimmed.split_once('#') {
        f
    } else {
        trimmed
    };
    let normalized_query = raw_query.replace(['?', '#'], "&");

    if !normalized_query.contains('=') && !trimmed.contains("://") && !trimmed.starts_with('/') {
        if let Some((raw_code, raw_state)) = trimmed.split_once('#') {
            let c = raw_code.trim();
            let s = raw_state.trim();
            if !c.is_empty() {
                params.code = Some(c.to_owned());
            }
            if !s.is_empty() {
                params.state = Some(s.to_owned());
            }
            return params;
        }
        params.code = Some(trimmed.to_owned());
        return params;
    }

    for pair in normalized_query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let key = percent_decode(k);
        let val = percent_decode(v);
        if val.is_empty() {
            continue;
        }
        match key.as_str() {
            "code" => params.code = Some(val),
            "state" => params.state = Some(val),
            "error" | "error_description" if params.error.is_none() => {
                params.error = Some(val);
            }
            _ => {}
        }
    }
    params
}

fn normalize_pkce_provider_id(provider_id: &str) -> Result<&'static str, String> {
    match provider_id.trim() {
        "openai_codex" | "openai_codex_pkce" => Ok("openai_codex"),
        "claude_code" | "claude_oauth" | "claude_oauth_pkce" => Ok("claude_code"),
        "openrouter" | "openrouter_pkce" => Ok("openrouter"),
        other => Err(format!("不支持的 PKCE 平台: {other}")),
    }
}

pub fn oauth_pkce_start(provider_id: &str) -> Result<OAuthPkceStartResult, String> {
    let canonical = normalize_pkce_provider_id(provider_id)?;
    let session_id = format!("pkce_{}", random_base64url(12));
    let code_verifier = random_base64url(48);
    let challenge_digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge_digest);
    let state = random_base64url(16);

    let (prepared, authorize_url, redirect_uri) = match canonical {
        "openai_codex" => {
            let prepared = prepare_pkce_loopback_listener(&[PKCE_CODEX_PRIMARY_PORT])?;
            let redirect_uri = format!("http://localhost:{}/auth/callback", prepared.port);
            let authorize_url = format!(
                "https://auth.openai.com/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&id_token_add_organizations=true&codex_cli_simplified_flow=true&state={}&originator=codex_cli_rs",
                percent_encode(OPENAI_CODEX_CLIENT_ID),
                percent_encode(&redirect_uri),
                percent_encode("openid profile email offline_access"),
                percent_encode(&code_challenge),
                percent_encode(&state),
            );
            (prepared, authorize_url, redirect_uri)
        }
        "claude_code" => {
            let prepared = prepare_pkce_loopback_listener(&[CLAUDE_PKCE_PORT])?;
            let redirect_uri = format!("http://localhost:{}/callback", prepared.port);
            let authorize_url = format!(
                "{CLAUDE_AUTHORIZE_URL}?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
                percent_encode(CLAUDE_OAUTH_CLIENT_ID),
                percent_encode(&redirect_uri),
                percent_encode("user:profile user:inference user:sessions:claude_code"),
                percent_encode(&code_challenge),
                percent_encode(&state),
            );
            (prepared, authorize_url, redirect_uri)
        }
        "openrouter" => {
            let prepared = prepare_pkce_loopback_listener(&[])?;
            let redirect_uri = format!("http://localhost:{}/auth/callback", prepared.port);
            let full_callback_url = format!("{redirect_uri}?state={}", percent_encode(&state));
            let authorize_url = format!(
                "https://openrouter.ai/auth?callback_url={}&code_challenge={}&code_challenge_method=S256",
                percent_encode(&full_callback_url),
                percent_encode(&code_challenge),
            );
            (prepared, authorize_url, redirect_uri)
        }
        _ => unreachable!(),
    };

    let expires_in_secs = 600;
    let session = PkceAuthSession {
        provider_id: canonical.to_owned(),
        code_verifier,
        state: state.clone(),
        redirect_uri: redirect_uri.clone(),
        captured_code: None,
        captured_error: None,
        exchange_in_progress: false,
        completed_result: None,
        expires_at_ms: now_ms().saturating_add(expires_in_secs * 1000),
    };
    {
        let mut map = pkce_sessions().lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, s| s.expires_at_ms > now_ms());
        map.insert(session_id.clone(), session);
    }
    spawn_prepared_pkce_listener(prepared, session_id.clone(), state.clone());

    Ok(OAuthPkceStartResult {
        session_id,
        provider_id: canonical.to_owned(),
        authorize_url,
        redirect_uri,
        state,
        expires_in_secs,
    })
}

fn finish_pkce_session_result(
    session_id: &str,
    result: OAuthPkceCompleteResult,
) -> OAuthPkceCompleteResult {
    let mut map = pkce_sessions().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(sess) = map.get_mut(session_id) {
        sess.exchange_in_progress = false;
        sess.completed_result = Some(result.clone());
    }
    result
}

pub fn oauth_pkce_poll_or_exchange(
    application: &AgentApplication,
    session_id: &str,
    callback_input: Option<&str>,
) -> Result<OAuthPkceCompleteResult, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;

    let manual_trimmed = callback_input.map(str::trim).filter(|s| !s.is_empty());

    let (session, taken_loopback_code) = {
        let mut map = pkce_sessions().lock().unwrap_or_else(|p| p.into_inner());
        let Some(sess) = map.get_mut(session_id) else {
            return Err("PKCE 授权会话不存在或已过期".to_owned());
        };
        if let Some(done) = sess.completed_result.clone() {
            return Ok(done);
        }
        if sess.exchange_in_progress {
            return Ok(OAuthPkceCompleteResult {
                status: "pending".into(),
                channel_id: None,
                message: None,
            });
        }
        if now_ms() >= sess.expires_at_ms {
            map.remove(session_id);
            return Ok(OAuthPkceCompleteResult {
                status: "expired".into(),
                channel_id: None,
                message: Some("PKCE 授权链接已过期，请重新生成".into()),
            });
        }
        if let Some(err) = sess.captured_error.take() {
            let result = OAuthPkceCompleteResult {
                status: "error".into(),
                channel_id: None,
                message: Some(err),
            };
            sess.completed_result = Some(result.clone());
            return Ok(result);
        }
        let code = if manual_trimmed.is_none() {
            let taken = sess.captured_code.take();
            if taken.is_some() {
                sess.exchange_in_progress = true;
            }
            taken
        } else {
            None
        };
        (sess.clone(), code)
    };

    let code = if let Some(manual) = manual_trimmed {
        // Direct JWT or JSON token fallback
        if session.provider_id == "openai_codex"
            && (manual.starts_with('{')
                || manual
                    .trim_start_matches("Bearer ")
                    .trim()
                    .split('.')
                    .count()
                    == 3)
        {
            let raw_tok = if manual.starts_with('{') {
                serde_json::from_str::<Value>(manual)
                    .ok()
                    .and_then(|v| v["access_token"].as_str().map(str::to_owned))
                    .ok_or_else(|| "JSON 中未找到有效 access_token".to_owned())?
            } else {
                manual.trim_start_matches("Bearer ").trim().to_owned()
            };
            let account_id = extract_chatgpt_account_id_from_jwt(&raw_tok);
            let envelope = OAuthKeyEnvelope {
                kind: EOSTAGE_OAUTH_ENVELOPE_KIND.into(),
                provider: "openai_codex".into(),
                access_token: raw_tok,
                refresh_token: None,
                expires_at_ms: None,
                account_id: account_id.clone(),
                copilot_api_token: None,
                copilot_expires_at_ms: None,
            };
            let secret_json = serde_json::to_string(&envelope).map_err(|e| e.to_string())?;
            let channel = openai_codex_channel_config();
            let channel_id = channel.id.clone();
            upsert_channel_config(&data_root, channel, &secret_json)?;
            return Ok(finish_pkce_session_result(
                session_id,
                OAuthPkceCompleteResult {
                    status: "authorized".into(),
                    channel_id: Some(channel_id),
                    message: Some(format!(
                        "已导入 OpenAI Codex 授权令牌并安全写入钥匙串{}",
                        account_id
                            .map(|a| format!(" (Account {a})"))
                            .unwrap_or_default()
                    )),
                },
            ));
        }

        let parsed = extract_code_and_state(manual);
        if let Some(err) = parsed.error {
            return Ok(finish_pkce_session_result(
                session_id,
                OAuthPkceCompleteResult {
                    status: "error".into(),
                    channel_id: None,
                    message: Some(err),
                },
            ));
        }
        if let Some(st) = parsed.state.as_deref() {
            if st != session.state {
                return Ok(OAuthPkceCompleteResult {
                    status: "error".into(),
                    channel_id: None,
                    message: Some(
                        "回调链接中的 state 参数不匹配，请确认粘贴的是本次授权的回调地址".into(),
                    ),
                });
            }
        }
        let Some(extracted) = parsed.code else {
            return Ok(OAuthPkceCompleteResult {
                status: "error".into(),
                channel_id: None,
                message: Some("未能从回调内容中提取 code 授权码".into()),
            });
        };
        {
            let mut map = pkce_sessions().lock().unwrap_or_else(|p| p.into_inner());
            if let Some(sess) = map.get_mut(session_id) {
                if let Some(done) = sess.completed_result.clone() {
                    return Ok(done);
                }
                if sess.exchange_in_progress {
                    return Ok(OAuthPkceCompleteResult {
                        status: "pending".into(),
                        channel_id: None,
                        message: None,
                    });
                }
                sess.exchange_in_progress = true;
            }
        }
        extracted
    } else if let Some(captured) = taken_loopback_code {
        captured
    } else {
        return Ok(OAuthPkceCompleteResult {
            status: "pending".into(),
            channel_id: None,
            message: None,
        });
    };

    let client = match build_http_client(application, Duration::from_secs(20)) {
        Ok(c) => c,
        Err(err) => {
            return Ok(finish_pkce_session_result(
                session_id,
                OAuthPkceCompleteResult {
                    status: "error".into(),
                    channel_id: None,
                    message: Some(err),
                },
            ));
        }
    };
    match session.provider_id.as_str() {
        "openai_codex" => {
            let token_resp = match client
                .post("https://auth.openai.com/oauth/token")
                .header("Accept", "application/json")
                .form(&[
                    ("grant_type", "authorization_code"),
                    ("client_id", OPENAI_CODEX_CLIENT_ID),
                    ("code", code.as_str()),
                    ("redirect_uri", session.redirect_uri.as_str()),
                    ("code_verifier", session.code_verifier.as_str()),
                ])
                .send()
            {
                Ok(r) => r,
                Err(e) => {
                    return Ok(finish_pkce_session_result(
                        session_id,
                        OAuthPkceCompleteResult {
                            status: "error".into(),
                            channel_id: None,
                            message: Some(format!("兑换 OpenAI Codex OAuth 令牌失败: {e}")),
                        },
                    ));
                }
            };
            if !token_resp.status().is_success() {
                let status = token_resp.status().as_u16();
                let body = token_resp.text().unwrap_or_default();
                let detail = serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v["error_description"]
                            .as_str()
                            .or_else(|| v["error"].as_str())
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| format!("HTTP {status}"));
                return Ok(finish_pkce_session_result(
                    session_id,
                    OAuthPkceCompleteResult {
                        status: "error".into(),
                        channel_id: None,
                        message: Some(format!(
                            "OpenAI OAuth 令牌兑换失败 (HTTP {status}: {detail})，请重新点击生成授权链接重试"
                        )),
                    },
                ));
            }
            let tokens: Value = match token_resp.json() {
                Ok(v) => v,
                Err(e) => {
                    return Ok(finish_pkce_session_result(
                        session_id,
                        OAuthPkceCompleteResult {
                            status: "error".into(),
                            channel_id: None,
                            message: Some(format!("解析 OpenAI OAuth 令牌 JSON 失败: {e}")),
                        },
                    ));
                }
            };
            let Some(access_token) = tokens["access_token"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
            else {
                return Ok(finish_pkce_session_result(
                    session_id,
                    OAuthPkceCompleteResult {
                        status: "error".into(),
                        channel_id: None,
                        message: Some("OpenAI OAuth 响应缺少 access_token".into()),
                    },
                ));
            };
            let refresh_token = tokens["refresh_token"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned);
            let expires_in = tokens["expires_in"].as_u64().unwrap_or(3600);
            let account_id = extract_chatgpt_account_id_from_jwt(&access_token).or_else(|| {
                tokens["id_token"]
                    .as_str()
                    .and_then(extract_chatgpt_account_id_from_jwt)
            });

            let envelope = OAuthKeyEnvelope {
                kind: EOSTAGE_OAUTH_ENVELOPE_KIND.into(),
                provider: "openai_codex".into(),
                access_token,
                refresh_token,
                expires_at_ms: extract_jwt_exp_ms(tokens["access_token"].as_str().unwrap_or(""))
                    .or_else(|| Some(now_ms().saturating_add(expires_in * 1000))),
                account_id: account_id.clone(),
                copilot_api_token: None,
                copilot_expires_at_ms: None,
            };
            let secret_json = match serde_json::to_string(&envelope) {
                Ok(s) => s,
                Err(e) => {
                    return Ok(finish_pkce_session_result(
                        session_id,
                        OAuthPkceCompleteResult {
                            status: "error".into(),
                            channel_id: None,
                            message: Some(e.to_string()),
                        },
                    ));
                }
            };
            let channel = openai_codex_channel_config();
            let channel_id = channel.id.clone();
            if let Err(err) = upsert_channel_config(&data_root, channel, &secret_json) {
                return Ok(finish_pkce_session_result(
                    session_id,
                    OAuthPkceCompleteResult {
                        status: "error".into(),
                        channel_id: None,
                        message: Some(err),
                    },
                ));
            }

            Ok(finish_pkce_session_result(
                session_id,
                OAuthPkceCompleteResult {
                    status: "authorized".into(),
                    channel_id: Some(channel_id),
                    message: Some(format!(
                        "已完成 ChatGPT / OpenAI Codex 浏览器 PKCE 授权并安全写入钥匙串{}",
                        account_id
                            .map(|a| format!(" (Account {a})"))
                            .unwrap_or_default()
                    )),
                },
            ))
        }
        "claude_code" => {
            let token_resp = match client
                .post(CLAUDE_TOKEN_URL)
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({
                    "grant_type": "authorization_code",
                    "code": code,
                    "state": session.state,
                    "client_id": CLAUDE_OAUTH_CLIENT_ID,
                    "redirect_uri": session.redirect_uri,
                    "code_verifier": session.code_verifier,
                }))
                .send()
            {
                Ok(r) => r,
                Err(e) => {
                    return Ok(finish_pkce_session_result(
                        session_id,
                        OAuthPkceCompleteResult {
                            status: "error".into(),
                            channel_id: None,
                            message: Some(format!("兑换 Claude OAuth 令牌失败: {e}")),
                        },
                    ));
                }
            };
            if !token_resp.status().is_success() {
                let status = token_resp.status().as_u16();
                let body = token_resp.text().unwrap_or_default();
                let detail = serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v["error_description"]
                            .as_str()
                            .or_else(|| v.pointer("/error/message").and_then(Value::as_str))
                            .or_else(|| v["error"].as_str())
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| format!("HTTP {status}"));
                return Ok(finish_pkce_session_result(
                    session_id,
                    OAuthPkceCompleteResult {
                        status: "error".into(),
                        channel_id: None,
                        message: Some(format!(
                            "Claude OAuth 令牌兑换失败 (HTTP {status}: {detail})，请重新点击生成授权链接重试"
                        )),
                    },
                ));
            }
            let tokens: Value = match token_resp.json() {
                Ok(v) => v,
                Err(e) => {
                    return Ok(finish_pkce_session_result(
                        session_id,
                        OAuthPkceCompleteResult {
                            status: "error".into(),
                            channel_id: None,
                            message: Some(format!("解析 Claude OAuth 令牌 JSON 失败: {e}")),
                        },
                    ));
                }
            };
            let Some(access_token) = tokens["access_token"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
            else {
                return Ok(finish_pkce_session_result(
                    session_id,
                    OAuthPkceCompleteResult {
                        status: "error".into(),
                        channel_id: None,
                        message: Some("Claude OAuth 响应缺少 access_token".into()),
                    },
                ));
            };
            let refresh_token = tokens["refresh_token"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned);
            let expires_in = tokens["expires_in"].as_u64().unwrap_or(28_800);
            let account_id = tokens
                .pointer("/account/email_address")
                .or_else(|| tokens.pointer("/account/uuid"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);

            let envelope = OAuthKeyEnvelope {
                kind: EOSTAGE_OAUTH_ENVELOPE_KIND.into(),
                provider: "claude_code".into(),
                access_token,
                refresh_token,
                expires_at_ms: Some(now_ms().saturating_add(expires_in.saturating_mul(1000))),
                account_id: account_id.clone(),
                copilot_api_token: None,
                copilot_expires_at_ms: None,
            };
            let secret_json = match serde_json::to_string(&envelope) {
                Ok(s) => s,
                Err(e) => {
                    return Ok(finish_pkce_session_result(
                        session_id,
                        OAuthPkceCompleteResult {
                            status: "error".into(),
                            channel_id: None,
                            message: Some(e.to_string()),
                        },
                    ));
                }
            };
            let channel = claude_oauth_channel_config();
            let channel_id = channel.id.clone();
            if let Err(err) = upsert_channel_config(&data_root, channel, &secret_json) {
                return Ok(finish_pkce_session_result(
                    session_id,
                    OAuthPkceCompleteResult {
                        status: "error".into(),
                        channel_id: None,
                        message: Some(err),
                    },
                ));
            }
            Ok(finish_pkce_session_result(
                session_id,
                OAuthPkceCompleteResult {
                    status: "authorized".into(),
                    channel_id: Some(channel_id),
                    message: Some(format!(
                        "已完成 Claude 浏览器 PKCE 授权并安全写入钥匙串{}",
                        account_id.map(|a| format!(" ({a})")).unwrap_or_default()
                    )),
                },
            ))
        }
        "openrouter" => {
            let resp = match client
                .post("https://openrouter.ai/api/v1/auth/keys")
                .json(&serde_json::json!({
                    "code": code,
                    "code_verifier": session.code_verifier,
                    "code_challenge_method": "S256",
                }))
                .send()
            {
                Ok(r) => r,
                Err(e) => {
                    return Ok(finish_pkce_session_result(
                        session_id,
                        OAuthPkceCompleteResult {
                            status: "error".into(),
                            channel_id: None,
                            message: Some(format!("兑换 OpenRouter API Key 失败: {e}")),
                        },
                    ));
                }
            };
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                return Ok(finish_pkce_session_result(
                    session_id,
                    OAuthPkceCompleteResult {
                        status: "error".into(),
                        channel_id: None,
                        message: Some(format!("OpenRouter 兑换接口返回 HTTP {status}")),
                    },
                ));
            }
            let payload: Value = match resp.json() {
                Ok(v) => v,
                Err(e) => {
                    return Ok(finish_pkce_session_result(
                        session_id,
                        OAuthPkceCompleteResult {
                            status: "error".into(),
                            channel_id: None,
                            message: Some(format!("解析 OpenRouter 兑换响应失败: {e}")),
                        },
                    ));
                }
            };
            let Some(api_key) = payload["key"].as_str().filter(|s| !s.trim().is_empty()) else {
                return Ok(finish_pkce_session_result(
                    session_id,
                    OAuthPkceCompleteResult {
                        status: "error".into(),
                        channel_id: None,
                        message: Some(
                            "OpenRouter 未返回有效 key（注意：授权码仅能兑换一次）".into(),
                        ),
                    },
                ));
            };

            let channel = openrouter_channel_config();
            let channel_id = channel.id.clone();
            if let Err(err) = upsert_channel_config(&data_root, channel, api_key) {
                return Ok(finish_pkce_session_result(
                    session_id,
                    OAuthPkceCompleteResult {
                        status: "error".into(),
                        channel_id: None,
                        message: Some(err),
                    },
                ));
            }

            Ok(finish_pkce_session_result(
                session_id,
                OAuthPkceCompleteResult {
                    status: "authorized".into(),
                    channel_id: Some(channel_id),
                    message: Some("已通过 OpenRouter PKCE 换取专属密钥并安全写入钥匙串".into()),
                },
            ))
        }
        other => Ok(finish_pkce_session_result(
            session_id,
            OAuthPkceCompleteResult {
                status: "error".into(),
                channel_id: None,
                message: Some(format!("不支持的 PKCE 平台: {other}")),
            },
        )),
    }
}

fn attach_codex_headers(env: &OAuthKeyEnvelope, route: ModelRoute) -> ModelRoute {
    let mut headers = Vec::new();
    if let Some(acct) = env.account_id.as_deref().filter(|s| !s.is_empty()) {
        headers.push(("ChatGPT-Account-Id".into(), acct.to_owned()));
    }
    headers.push(("originator".into(), "codex_cli_rs".into()));
    route.with_extra_headers(headers)
}

fn refresh_codex_oauth_envelope(
    application: &AgentApplication,
    channel_id: &str,
    env: &mut OAuthKeyEnvelope,
) -> bool {
    let Some(refresh_tok) = env.refresh_token.clone().filter(|s| !s.trim().is_empty()) else {
        return false;
    };
    let Ok(client) = build_http_client(application, Duration::from_secs(15)) else {
        return false;
    };
    let Ok(resp) = client
        .post("https://auth.openai.com/oauth/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_tok.as_str()),
            ("client_id", OPENAI_CODEX_CLIENT_ID),
        ])
        .send()
    else {
        return false;
    };
    if !resp.status().is_success() {
        return false;
    }
    let Ok(json) = resp.json::<Value>() else {
        return false;
    };
    let Some(new_access) = json["access_token"].as_str().filter(|s| !s.is_empty()) else {
        return false;
    };
    env.access_token = new_access.to_owned();
    if let Some(new_ref) = json["refresh_token"].as_str().filter(|s| !s.is_empty()) {
        env.refresh_token = Some(new_ref.to_owned());
    }
    let expires_in = json["expires_in"].as_u64().unwrap_or(3600);
    env.expires_at_ms = extract_jwt_exp_ms(&env.access_token)
        .or_else(|| Some(now_ms().saturating_add(expires_in * 1000)));
    if let Some(acct) = extract_chatgpt_account_id_from_jwt(&env.access_token).or_else(|| {
        json["id_token"]
            .as_str()
            .and_then(extract_chatgpt_account_id_from_jwt)
    }) {
        env.account_id = Some(acct);
    }
    if let Ok(updated) = serde_json::to_string(env) {
        let _ = insar_download::store_desktop_agent_model_key(channel_id, &updated);
    }
    true
}

fn refresh_claude_oauth_envelope(
    application: &AgentApplication,
    channel_id: &str,
    env: &mut OAuthKeyEnvelope,
) -> bool {
    let Some(refresh_tok) = env.refresh_token.clone().filter(|s| !s.trim().is_empty()) else {
        return false;
    };
    let Ok(client) = build_http_client(application, Duration::from_secs(15)) else {
        return false;
    };
    let Ok(resp) = client
        .post(CLAUDE_TOKEN_URL)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_tok,
            "client_id": CLAUDE_OAUTH_CLIENT_ID,
        }))
        .send()
    else {
        return false;
    };
    if !resp.status().is_success() {
        return false;
    }
    let Ok(json) = resp.json::<Value>() else {
        return false;
    };
    let Some(new_access) = json["access_token"].as_str().filter(|s| !s.is_empty()) else {
        return false;
    };
    env.access_token = new_access.to_owned();
    if let Some(new_ref) = json["refresh_token"].as_str().filter(|s| !s.is_empty()) {
        env.refresh_token = Some(new_ref.to_owned());
    }
    let expires_in = json["expires_in"].as_u64().unwrap_or(28_800);
    env.expires_at_ms = Some(now_ms().saturating_add(expires_in.saturating_mul(1000)));
    if let Ok(updated) = serde_json::to_string(env) {
        let _ = insar_download::store_desktop_agent_model_key(channel_id, &updated);
    }
    true
}

/// On an HTTP 401 from an OpenAI Codex or Claude OAuth channel, force-refresh the OAuth token
/// and return the updated `(secret, route)`.
pub fn force_refresh_or_reload_codex_oauth(
    application: &AgentApplication,
    channel_id: &str,
    mut route: ModelRoute,
) -> Option<(Zeroizing<String>, ModelRoute)> {
    let raw_secret = insar_download::with_desktop_agent_model_key(channel_id, |k| k.to_owned())
        .ok()
        .flatten()?;
    let trimmed = raw_secret.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let mut env = serde_json::from_str::<OAuthKeyEnvelope>(trimmed).ok()?;
    if env.kind != EOSTAGE_OAUTH_ENVELOPE_KIND {
        return None;
    }
    if env.provider == "claude_code" {
        if !refresh_claude_oauth_envelope(application, channel_id, &mut env) {
            return None;
        }
        let mut headers = route.extra_headers.clone();
        headers.retain(|(k, _)| {
            !k.eq_ignore_ascii_case("x-eostage-auth-mode")
                && !k.eq_ignore_ascii_case("anthropic-beta")
        });
        headers.push(("x-eostage-auth-mode".into(), "bearer".into()));
        headers.push(("anthropic-beta".into(), "oauth-2025-04-20".into()));
        route = route.with_extra_headers(headers);
        return Some((Zeroizing::new(env.access_token), route));
    }
    if env.provider != "openai_codex" {
        return None;
    }

    let refreshed = refresh_codex_oauth_envelope(application, channel_id, &mut env);

    if !refreshed {
        return None;
    }
    let updated_route = attach_codex_headers(&env, route);
    Some((Zeroizing::new(env.access_token), updated_route))
}

/// Unwraps plain API keys unchanged, or handles `eostage_oauth_v1` OAuth envelopes
/// (auto-refreshing expired tokens or exchanging GitHub Copilot session tokens)
/// and injects required provider headers onto `ModelRoute`.
pub fn resolve_runtime_secret_and_route(
    application: &AgentApplication,
    channel_id: &str,
    raw_secret: &str,
    mut route: ModelRoute,
) -> (Zeroizing<String>, ModelRoute) {
    let trimmed = raw_secret.trim();
    if !trimmed.starts_with('{') {
        return (Zeroizing::new(raw_secret.to_owned()), route);
    }
    let Ok(mut env) = serde_json::from_str::<OAuthKeyEnvelope>(trimmed) else {
        return (Zeroizing::new(raw_secret.to_owned()), route);
    };
    if env.kind != EOSTAGE_OAUTH_ENVELOPE_KIND {
        return (Zeroizing::new(raw_secret.to_owned()), route);
    }

    let now = now_ms();
    match env.provider.as_str() {
        "openai_codex" => {
            let effective_exp = env
                .expires_at_ms
                .or_else(|| extract_jwt_exp_ms(&env.access_token));
            if effective_exp.is_some_and(|exp| now.saturating_add(60_000) >= exp) {
                let _ = refresh_codex_oauth_envelope(application, channel_id, &mut env);
            }
            if env
                .account_id
                .as_deref()
                .is_none_or(|s| s.trim().is_empty())
            {
                env.account_id = extract_chatgpt_account_id_from_jwt(&env.access_token);
            }
            let route = attach_codex_headers(&env, route);
            (Zeroizing::new(env.access_token), route)
        }
        "github_copilot" => {
            let needs_exchange = env.copilot_api_token.is_none()
                || env
                    .copilot_expires_at_ms
                    .is_none_or(|exp| now.saturating_add(60_000) >= exp);
            if needs_exchange {
                env.copilot_api_token = None;
                env.copilot_expires_at_ms = None;
                if let Ok(client) = build_http_client(application, Duration::from_secs(15)) {
                    if let Ok(resp) = client
                        .get("https://api.github.com/copilot_internal/v2/token")
                        .header("Authorization", format!("token {}", env.access_token))
                        .header("Accept", "application/json")
                        .header("User-Agent", "GithubCopilot/1.155.0")
                        .header("Editor-Version", "vscode/1.95.0")
                        .header("Editor-Plugin-Version", "copilot-chat/0.22.4")
                        .send()
                    {
                        if resp.status().is_success() {
                            if let Ok(json) = resp.json::<Value>() {
                                if let Some(tok) = json["token"].as_str().filter(|s| !s.is_empty())
                                {
                                    env.copilot_api_token = Some(tok.to_owned());
                                    if let Some(exp_sec) = json["expires_at"].as_u64() {
                                        env.copilot_expires_at_ms =
                                            Some(exp_sec.saturating_mul(1000));
                                    }
                                    if let Ok(updated) = serde_json::to_string(&env) {
                                        let _ = insar_download::store_desktop_agent_model_key(
                                            channel_id, &updated,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let active_token = env.copilot_api_token.unwrap_or(env.access_token);
            let headers = vec![
                ("Copilot-Integration-Id".into(), "vscode-chat".into()),
                ("Editor-Version".into(), "vscode/1.95.0".into()),
                ("Editor-Plugin-Version".into(), "copilot-chat/0.22.4".into()),
            ];
            route = route.with_extra_headers(headers);
            (Zeroizing::new(active_token), route)
        }
        "kimi_code" => {
            if env
                .expires_at_ms
                .is_some_and(|exp| now.saturating_add(60_000) >= exp)
            {
                if let Some(refresh_tok) = env.refresh_token.clone() {
                    if let Ok(client) = build_http_client(application, Duration::from_secs(15)) {
                        if let Ok(resp) = client
                            .post("https://auth.kimi.com/api/oauth/token")
                            .form(&[
                                ("grant_type", "refresh_token"),
                                ("refresh_token", refresh_tok.as_str()),
                                ("client_id", KIMI_CODE_CLIENT_ID),
                            ])
                            .send()
                        {
                            if resp.status().is_success() {
                                if let Ok(json) = resp.json::<Value>() {
                                    if let Some(new_access) =
                                        json["access_token"].as_str().filter(|s| !s.is_empty())
                                    {
                                        env.access_token = new_access.to_owned();
                                        if let Some(new_ref) =
                                            json["refresh_token"].as_str().filter(|s| !s.is_empty())
                                        {
                                            env.refresh_token = Some(new_ref.to_owned());
                                        }
                                        let expires_in =
                                            json["expires_in"].as_u64().unwrap_or(3600);
                                        env.expires_at_ms =
                                            Some(now_ms().saturating_add(expires_in * 1000));
                                        if let Ok(updated) = serde_json::to_string(&env) {
                                            let _ = insar_download::store_desktop_agent_model_key(
                                                channel_id, &updated,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            route = route.with_extra_headers(vec![("User-Agent".into(), "KimiCLI/1.3.0".into())]);
            (Zeroizing::new(env.access_token), route)
        }
        "claude_code" => {
            if env
                .expires_at_ms
                .is_some_and(|exp| now.saturating_add(60_000) >= exp)
            {
                let _ = refresh_claude_oauth_envelope(application, channel_id, &mut env);
            }
            let mut headers = route.extra_headers.clone();
            headers.push(("x-eostage-auth-mode".into(), "bearer".into()));
            headers.push(("anthropic-beta".into(), "oauth-2025-04-20".into()));
            route = route.with_extra_headers(headers);
            (Zeroizing::new(env.access_token), route)
        }
        "gemini_cli" => {
            let mut headers = route.extra_headers.clone();
            headers.push(("x-eostage-auth-mode".into(), "bearer".into()));
            route = route.with_extra_headers(headers);
            (Zeroizing::new(env.access_token), route)
        }
        _ => (Zeroizing::new(env.access_token), route),
    }
}

pub fn sync_public_model_registry(
    application: &AgentApplication,
) -> Result<PublicModelRegistrySyncResult, String> {
    let client = build_http_client(application, Duration::from_secs(18))?;
    let mut specs: Vec<NormalizedPublicModelSpec> = Vec::new();
    let mut sources: Vec<String> = Vec::new();

    if let Ok(resp) = client
        .get("https://models.dev/api.json")
        .header("Accept", "application/json")
        .send()
    {
        if resp.status().is_success() {
            if let Ok(payload) = resp.json::<Value>() {
                if let Some(providers) = payload.as_object() {
                    let mut parsed_count = 0usize;
                    for (provider_id, pval) in providers {
                        let provider_name = pval["name"]
                            .as_str()
                            .unwrap_or(provider_id.as_str())
                            .to_owned();
                        let Some(models_obj) = pval["models"].as_object() else {
                            continue;
                        };
                        for (model_key, mval) in models_obj {
                            let context_window = mval
                                .pointer("/limit/context")
                                .and_then(Value::as_u64)
                                .unwrap_or(0);
                            if context_window < 1024 {
                                continue;
                            }
                            let raw_out = mval
                                .pointer("/limit/output")
                                .and_then(Value::as_u64)
                                .unwrap_or(0);
                            let max_output = if raw_out >= 256 {
                                raw_out
                            } else {
                                (context_window / 8).clamp(8192, 65536)
                            };
                            let inputs = mval
                                .pointer("/modalities/input")
                                .and_then(Value::as_array)
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(Value::as_str)
                                        .map(|s| s.to_ascii_lowercase())
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_else(|| vec!["text".into()]);
                            let raw_id = mval["id"]
                                .as_str()
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .unwrap_or(model_key.trim());
                            if raw_id.is_empty() {
                                continue;
                            }
                            let id = if raw_id.contains('/') {
                                raw_id.to_owned()
                            } else {
                                format!("{provider_id}/{raw_id}")
                            };
                            let name = mval["name"]
                                .as_str()
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .unwrap_or(&id)
                                .to_owned();
                            specs.push(NormalizedPublicModelSpec {
                                id,
                                provider: Some(provider_name.clone()),
                                name,
                                context_window,
                                max_output_tokens: max_output,
                                supports_tools: mval["tool_call"].as_bool().unwrap_or(true),
                                supports_image: inputs.iter().any(|x| x == "image"),
                                supports_video: inputs.iter().any(|x| x == "video"),
                                supports_pdf: inputs.iter().any(|x| x == "pdf"),
                                reasoning: mval["reasoning"].as_bool().unwrap_or(false),
                            });
                            parsed_count += 1;
                        }
                    }
                    if parsed_count > 0 {
                        sources.push("models.dev".into());
                    }
                }
            }
        }
    }

    if let Ok(resp) = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Accept", "application/json")
        .send()
    {
        if resp.status().is_success() {
            if let Ok(payload) = resp.json::<Value>() {
                if let Some(arr) = payload["data"].as_array() {
                    let mut parsed_count = 0usize;
                    for entry in arr {
                        let Some(id) = entry["id"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        else {
                            continue;
                        };
                        let ctx = entry["context_length"]
                            .as_u64()
                            .or_else(|| {
                                entry
                                    .pointer("/top_provider/context_length")
                                    .and_then(Value::as_u64)
                            })
                            .unwrap_or(0);
                        if ctx < 1024 {
                            continue;
                        }
                        let raw_out = entry
                            .pointer("/top_provider/max_completion_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        let max_out = if raw_out >= 256 {
                            raw_out
                        } else {
                            (ctx / 8).clamp(8192, 65536)
                        };
                        let input_mods = entry
                            .pointer("/architecture/input_modalities")
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(|s| s.to_ascii_lowercase())
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_else(|| vec!["text".into()]);
                        let supported_params = entry["supported_parameters"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(|s| s.to_ascii_lowercase())
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        let raw_name = entry["name"].as_str().unwrap_or(id);
                        let clean_name = raw_name
                            .split_once(": ")
                            .map(|(_, r)| r.trim())
                            .unwrap_or(raw_name.trim())
                            .to_owned();
                        specs.push(NormalizedPublicModelSpec {
                            id: id.to_owned(),
                            provider: id.split('/').next().map(str::to_owned),
                            name: if clean_name.is_empty() {
                                id.to_owned()
                            } else {
                                clean_name
                            },
                            context_window: ctx,
                            max_output_tokens: max_out,
                            supports_tools: supported_params.is_empty()
                                || supported_params.iter().any(|p| p == "tools"),
                            supports_image: input_mods.iter().any(|m| m == "image"),
                            supports_video: input_mods.iter().any(|m| m == "video"),
                            supports_pdf: input_mods.iter().any(|m| m == "file" || m == "pdf"),
                            reasoning: supported_params
                                .iter()
                                .any(|p| p == "reasoning" || p == "include_reasoning"),
                        });
                        parsed_count += 1;
                    }
                    if parsed_count > 0 {
                        sources.push("OpenRouter".into());
                    }
                }
            }
        }
    }

    if specs.is_empty() {
        return Err("公开模型元数据服务暂不可达，当前使用内置模型库".into());
    }

    let mut dedup: HashMap<String, NormalizedPublicModelSpec> = HashMap::new();
    for item in specs {
        let key = item.id.to_ascii_lowercase();
        let replace = match dedup.get(&key) {
            None => true,
            Some(prev) => {
                item.context_window > prev.context_window
                    || (item.context_window == prev.context_window
                        && item.max_output_tokens > prev.max_output_tokens)
            }
        };
        if replace {
            dedup.insert(key, item);
        }
    }
    let mut items: Vec<NormalizedPublicModelSpec> = dedup.into_values().collect();
    items.sort_by(|a, b| {
        b.context_window
            .cmp(&a.context_window)
            .then_with(|| a.id.cmp(&b.id))
    });
    if items.len() > 2400 {
        items.truncate(2400);
    }

    Ok(PublicModelRegistrySyncResult {
        items,
        source_summary: sources.join(" + "),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelAuthMeta {
    pub channel_id: String,
    pub auth_kind: String, // "official" | "subscription" | "api"
    pub oauth_provider: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionQuotaWindow {
    pub used_percent: i64,
    pub remaining_percent: i64,
    pub window_minutes: Option<i64>,
    pub reset_at_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionChannelQuota {
    pub channel_id: String,
    pub oauth_provider: String,
    pub plan_type: Option<String>,
    pub account_id: Option<String>,
    pub primary_window: Option<SubscriptionQuotaWindow>,
    pub secondary_window: Option<SubscriptionQuotaWindow>,
    pub reset_credits_available: Option<i64>,
    pub credits_has_credits: Option<bool>,
    pub credits_unlimited: Option<bool>,
    pub credits_balance: Option<String>,
    pub fetched_at_ms: u64,
}

pub fn classify_channel_secret_auth_meta(raw_secret: Option<&str>) -> ChannelAuthMeta {
    if let Some(trimmed) = raw_secret.map(str::trim).filter(|s| s.starts_with('{')) {
        if let Ok(env) = serde_json::from_str::<OAuthKeyEnvelope>(trimmed) {
            if env.kind == EOSTAGE_OAUTH_ENVELOPE_KIND && !env.provider.trim().is_empty() {
                let account_id = env
                    .account_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or_else(|| extract_chatgpt_account_id_from_jwt(&env.access_token));
                return ChannelAuthMeta {
                    channel_id: String::new(),
                    auth_kind: "subscription".to_owned(),
                    oauth_provider: Some(env.provider),
                    account_id,
                };
            }
        }
    }
    ChannelAuthMeta {
        channel_id: String::new(),
        auth_kind: "api".to_owned(),
        oauth_provider: None,
        account_id: None,
    }
}

pub fn inspect_channels_auth_meta(
    application: &AgentApplication,
) -> Result<Vec<ChannelAuthMeta>, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let store = ChannelStore::new(data_root.join("agent"));
    let channels = store
        .list(&DesktopKeys)
        .map_err(|e| format!("读取模型渠道失败: {e:?}"))?;
    let mut out = Vec::with_capacity(channels.len());
    for (channel, _key_ready) in channels {
        if crate::eostage_account::is_official_channel_id(&channel.id) {
            out.push(ChannelAuthMeta {
                channel_id: channel.id,
                auth_kind: "official".to_owned(),
                oauth_provider: None,
                account_id: None,
            });
            continue;
        }
        let raw_secret =
            insar_download::with_desktop_agent_model_key(&channel.id, |k| k.to_owned())
                .ok()
                .flatten();
        let mut meta = classify_channel_secret_auth_meta(raw_secret.as_deref());
        meta.channel_id = channel.id;
        out.push(meta);
    }
    Ok(out)
}

fn parse_quota_window(window: &Value, now_secs: i64) -> Option<SubscriptionQuotaWindow> {
    let obj = window.as_object()?;
    let used_raw = obj.get("used_percent").and_then(Value::as_f64)?;
    let used_clamped = used_raw.clamp(0.0, 100.0);
    let used_percent = used_clamped.round() as i64;
    let remaining_percent = (100.0 - used_clamped).round() as i64;
    let window_minutes = obj
        .get("limit_window_seconds")
        .and_then(Value::as_i64)
        .filter(|secs| *secs > 0)
        .map(|secs| (secs / 60).max(1));
    let reset_at_secs = obj
        .get("reset_at")
        .and_then(Value::as_i64)
        .filter(|ts| *ts > 0)
        .or_else(|| {
            obj.get("reset_after_seconds")
                .and_then(Value::as_i64)
                .filter(|secs| *secs >= 0)
                .map(|secs| now_secs.saturating_add(secs))
        });
    Some(SubscriptionQuotaWindow {
        used_percent,
        remaining_percent,
        window_minutes,
        reset_at_secs,
    })
}

pub fn parse_codex_usage_response(
    channel_id: &str,
    fallback_account_id: Option<String>,
    usage: &Value,
    now_secs: i64,
    fetched_at_ms: u64,
) -> SubscriptionChannelQuota {
    let plan_type = usage
        .get("plan_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let primary_window = usage
        .pointer("/rate_limit/primary_window")
        .and_then(|v| parse_quota_window(v, now_secs));
    let secondary_window = usage
        .pointer("/rate_limit/secondary_window")
        .and_then(|v| parse_quota_window(v, now_secs));
    let reset_credits_available = usage
        .pointer("/rate_limit_reset_credits/available_count")
        .and_then(Value::as_i64);
    let credits_has_credits = usage
        .pointer("/credits/has_credits")
        .and_then(Value::as_bool);
    let credits_unlimited = usage.pointer("/credits/unlimited").and_then(Value::as_bool);
    let credits_balance = usage.pointer("/credits/balance").and_then(|v| {
        if let Some(s) = v.as_str() {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        } else {
            v.as_f64().map(|n| format!("{n:.2}"))
        }
    });

    SubscriptionChannelQuota {
        channel_id: channel_id.to_owned(),
        oauth_provider: "openai_codex".to_owned(),
        plan_type,
        account_id: fallback_account_id,
        primary_window,
        secondary_window,
        reset_credits_available,
        credits_has_credits,
        credits_unlimited,
        credits_balance,
        fetched_at_ms,
    }
}

fn sanitize_provider_quota_http_error(provider_label: &str, status: u16, body: &str) -> String {
    if status == 401 || status == 403 {
        return format!("{provider_label} 订阅授权已失效（HTTP {status}），请重新登录授权");
    }
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("detail"))
                .or_else(|| v.get("message"))
                .and_then(Value::as_str)
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_default();
    if detail.is_empty() {
        format!("查询 {provider_label} 订阅额度失败（HTTP {status}）")
    } else {
        format!("查询 {provider_label} 订阅额度失败（HTTP {status}）: {detail}")
    }
}

fn parse_rfc3339_timestamp_secs(raw: Option<&Value>) -> Option<i64> {
    raw.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp())
}

fn parse_claude_usage_window(
    obj: &Value,
    default_window_minutes: i64,
) -> Option<SubscriptionQuotaWindow> {
    let used_raw = obj.get("utilization").and_then(Value::as_f64)?;
    let used_clamped = used_raw.clamp(0.0, 100.0);
    let used_percent = used_clamped.round() as i64;
    let remaining_percent = (100.0 - used_clamped).round() as i64;
    let reset_at_secs = parse_rfc3339_timestamp_secs(obj.get("resets_at"));
    Some(SubscriptionQuotaWindow {
        used_percent,
        remaining_percent,
        window_minutes: Some(default_window_minutes),
        reset_at_secs,
    })
}

pub fn parse_claude_usage_response(
    channel_id: &str,
    account_id: Option<String>,
    usage: &Value,
    fetched_at_ms: u64,
) -> SubscriptionChannelQuota {
    let primary_window = usage
        .get("five_hour")
        .filter(|v| !v.is_null())
        .and_then(|v| parse_claude_usage_window(v, 300));
    let secondary_window = usage
        .get("seven_day")
        .filter(|v| !v.is_null())
        .and_then(|v| parse_claude_usage_window(v, 10080));
    let extra_enabled = usage
        .pointer("/extra_usage/is_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let credits_balance = if extra_enabled {
        let monthly = usage
            .pointer("/extra_usage/monthly_limit")
            .and_then(Value::as_f64);
        let used = usage
            .pointer("/extra_usage/used_credits")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        monthly.map(|limit| format!("{:.2}", (limit - used).max(0.0)))
    } else {
        None
    };

    SubscriptionChannelQuota {
        channel_id: channel_id.to_owned(),
        oauth_provider: "claude_code".to_owned(),
        plan_type: Some("Claude".to_owned()),
        account_id,
        primary_window,
        secondary_window,
        reset_credits_available: None,
        credits_has_credits: Some(extra_enabled),
        credits_unlimited: None,
        credits_balance,
        fetched_at_ms,
    }
}

fn parse_copilot_snapshot_window(
    snapshot: &Value,
    reset_at_secs: Option<i64>,
) -> Option<SubscriptionQuotaWindow> {
    let obj = snapshot.as_object()?;
    let unlimited = obj
        .get("unlimited")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let rem_raw = if unlimited {
        100.0
    } else if let Some(pct) = obj.get("percent_remaining").and_then(Value::as_f64) {
        pct
    } else {
        let ent = obj.get("entitlement").and_then(Value::as_f64)?;
        let rem = obj
            .get("quota_remaining")
            .or_else(|| obj.get("remaining"))
            .and_then(Value::as_f64)?;
        if ent <= 0.0 {
            0.0
        } else {
            (rem / ent) * 100.0
        }
    };
    let rem_clamped = rem_raw.clamp(0.0, 100.0);
    let remaining_percent = rem_clamped.round() as i64;
    let used_percent = (100.0 - rem_clamped).round() as i64;
    Some(SubscriptionQuotaWindow {
        used_percent,
        remaining_percent,
        window_minutes: None,
        reset_at_secs,
    })
}

pub fn parse_copilot_usage_response(
    channel_id: &str,
    usage: &Value,
    fetched_at_ms: u64,
) -> SubscriptionChannelQuota {
    let plan_type = usage
        .get("copilot_plan")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let reset_at_secs = parse_rfc3339_timestamp_secs(usage.get("quota_reset_date_utc"));
    let snapshots = usage.get("quota_snapshots");
    let premium = snapshots.and_then(|s| s.get("premium_interactions"));
    let chat = snapshots.and_then(|s| s.get("chat"));
    let primary_window = premium.and_then(|v| parse_copilot_snapshot_window(v, reset_at_secs));
    let secondary_window = chat.and_then(|v| parse_copilot_snapshot_window(v, reset_at_secs));
    let unlimited_flag = premium
        .and_then(|v| v.get("unlimited"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && chat
            .and_then(|v| v.get("unlimited"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

    SubscriptionChannelQuota {
        channel_id: channel_id.to_owned(),
        oauth_provider: "github_copilot".to_owned(),
        plan_type,
        account_id: None,
        primary_window,
        secondary_window,
        reset_credits_available: None,
        credits_has_credits: None,
        credits_unlimited: unlimited_flag.then_some(true),
        credits_balance: None,
        fetched_at_ms,
    }
}

pub fn fetch_subscription_channel_quota(
    application: &AgentApplication,
    channel_id: &str,
) -> Result<SubscriptionChannelQuota, String> {
    let raw_secret = insar_download::with_desktop_agent_model_key(channel_id, |k| k.to_owned())
        .map_err(|e| format!("读取渠道凭据失败: {e}"))?
        .ok_or_else(|| "当前渠道未保存订阅授权凭据".to_owned())?;
    let trimmed = raw_secret.trim();
    if !trimmed.starts_with('{') {
        return Err("当前渠道为 API 密钥模式，暂无订阅额度接口".to_owned());
    }
    let mut env = serde_json::from_str::<OAuthKeyEnvelope>(trimmed)
        .map_err(|_| "当前渠道凭据不是订阅授权信封".to_owned())?;
    if env.kind != EOSTAGE_OAUTH_ENVELOPE_KIND {
        return Err("当前渠道凭据不是订阅授权信封".to_owned());
    }

    let client = build_http_client(application, Duration::from_secs(20))?;
    let now = now_ms();

    match env.provider.as_str() {
        "openai_codex" => {
            let effective_exp = env
                .expires_at_ms
                .or_else(|| extract_jwt_exp_ms(&env.access_token));
            if effective_exp.is_some_and(|exp| now.saturating_add(60_000) >= exp) {
                let _ = refresh_codex_oauth_envelope(application, channel_id, &mut env);
            }
            if env
                .account_id
                .as_deref()
                .is_none_or(|s| s.trim().is_empty())
            {
                env.account_id = extract_chatgpt_account_id_from_jwt(&env.access_token);
            }
            let send_usage_request = |env_ref: &OAuthKeyEnvelope| {
                let mut req = client
                    .get(CODEX_USAGE_URL)
                    .bearer_auth(&env_ref.access_token)
                    .header("Accept", "application/json")
                    .header("originator", "codex_cli_rs")
                    .header("User-Agent", "codex_cli_rs");
                if let Some(acct) = env_ref
                    .account_id
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                {
                    req = req.header("ChatGPT-Account-Id", acct);
                }
                req.send()
            };
            let mut resp = send_usage_request(&env)
                .map_err(|e| format!("请求 ChatGPT 订阅额度接口失败: {e}"))?;
            if resp.status().as_u16() == 401
                && refresh_codex_oauth_envelope(application, channel_id, &mut env)
            {
                resp = send_usage_request(&env)
                    .map_err(|e| format!("刷新后请求 ChatGPT 订阅额度接口失败: {e}"))?;
            }
            let status = resp.status().as_u16();
            let body = resp.text().unwrap_or_default();
            if !(200..300).contains(&status) {
                return Err(sanitize_provider_quota_http_error("ChatGPT", status, &body));
            }
            let usage: Value = serde_json::from_str(&body)
                .map_err(|e| format!("解析 ChatGPT 订阅额度响应失败: {e}"))?;
            let fetched_at_ms = now_ms();
            let now_secs = (fetched_at_ms / 1000) as i64;
            Ok(parse_codex_usage_response(
                channel_id,
                env.account_id.clone(),
                &usage,
                now_secs,
                fetched_at_ms,
            ))
        }
        "claude_code" => {
            if env
                .expires_at_ms
                .is_some_and(|exp| now.saturating_add(60_000) >= exp)
            {
                let _ = refresh_claude_oauth_envelope(application, channel_id, &mut env);
            }
            let send_usage_request = |env_ref: &OAuthKeyEnvelope| {
                client
                    .get(CLAUDE_USAGE_URL)
                    .bearer_auth(&env_ref.access_token)
                    .header("Accept", "application/json")
                    .header("anthropic-beta", "oauth-2025-04-20")
                    .header("User-Agent", "claude-code/2.0.0")
                    .send()
            };
            let mut resp = send_usage_request(&env)
                .map_err(|e| format!("请求 Claude 订阅额度接口失败: {e}"))?;
            if resp.status().as_u16() == 401
                && refresh_claude_oauth_envelope(application, channel_id, &mut env)
            {
                resp = send_usage_request(&env)
                    .map_err(|e| format!("刷新后请求 Claude 订阅额度接口失败: {e}"))?;
            }
            let status = resp.status().as_u16();
            let body = resp.text().unwrap_or_default();
            if !(200..300).contains(&status) {
                return Err(sanitize_provider_quota_http_error("Claude", status, &body));
            }
            let usage: Value = serde_json::from_str(&body)
                .map_err(|e| format!("解析 Claude 订阅额度响应失败: {e}"))?;
            let fetched_at_ms = now_ms();
            Ok(parse_claude_usage_response(
                channel_id,
                env.account_id.clone(),
                &usage,
                fetched_at_ms,
            ))
        }
        "github_copilot" => {
            let resp = client
                .get(GITHUB_COPILOT_USER_URL)
                .header("Authorization", format!("token {}", env.access_token))
                .header("Accept", "application/json")
                .header("User-Agent", "GithubCopilot/1.155.0")
                .header("Editor-Version", "vscode/1.95.0")
                .header("Editor-Plugin-Version", "copilot-chat/0.22.4")
                .send()
                .map_err(|e| format!("请求 GitHub Copilot 订阅额度接口失败: {e}"))?;
            let status = resp.status().as_u16();
            let body = resp.text().unwrap_or_default();
            if !(200..300).contains(&status) {
                return Err(sanitize_provider_quota_http_error(
                    "GitHub Copilot",
                    status,
                    &body,
                ));
            }
            let usage: Value = serde_json::from_str(&body)
                .map_err(|e| format!("解析 GitHub Copilot 订阅额度响应失败: {e}"))?;
            let fetched_at_ms = now_ms();
            Ok(parse_copilot_usage_response(
                channel_id,
                &usage,
                fetched_at_ms,
            ))
        }
        other => Err(format!("暂不支持查询 {other} 的订阅额度")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_code_and_state_handles_codex_and_openrouter_callbacks() {
        let codex = extract_code_and_state(
            "http://localhost:1455/auth/callback?code=ac_test_12345&state=st_abc",
        );
        assert_eq!(codex.code.as_deref(), Some("ac_test_12345"));
        assert_eq!(codex.state.as_deref(), Some("st_abc"));
        assert_eq!(codex.error, None);

        let openrouter_double_q = extract_code_and_state(
            "http://localhost:54321/auth/callback?state=st_xyz?code=or_code_999",
        );
        assert_eq!(openrouter_double_q.code.as_deref(), Some("or_code_999"));
        assert_eq!(openrouter_double_q.state.as_deref(), Some("st_xyz"));

        let raw = extract_code_and_state("direct_auth_code_token");
        assert_eq!(raw.code.as_deref(), Some("direct_auth_code_token"));
        assert_eq!(raw.state, None);

        let claude_manual = extract_code_and_state("claude_code_99#claude_state_88");
        assert_eq!(claude_manual.code.as_deref(), Some("claude_code_99"));
        assert_eq!(claude_manual.state.as_deref(), Some("claude_state_88"));
    }

    #[test]
    fn extract_chatgpt_account_id_parses_jwt_claim() {
        let payload = serde_json::json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_sci_42"
            }
        });
        let encoded_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{encoded_payload}.sig");
        assert_eq!(
            extract_chatgpt_account_id_from_jwt(&fake_jwt).as_deref(),
            Some("acct_sci_42")
        );
    }

    #[test]
    fn oauth_pkce_start_supports_openai_codex_and_openrouter() {
        let codex = oauth_pkce_start("openai_codex").expect("openai_codex pkce start");
        assert_eq!(codex.provider_id, "openai_codex");
        assert_eq!(codex.redirect_uri, "http://localhost:1455/auth/callback");
        assert!(codex
            .authorize_url
            .starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(codex.authorize_url.contains("code_challenge_method=S256"));
        assert!(codex
            .authorize_url
            .contains("codex_cli_simplified_flow=true"));
        assert!(has_recent_or_active_native_pkce_session());

        let openrouter = oauth_pkce_start("openrouter").expect("openrouter pkce start");
        assert_eq!(openrouter.provider_id, "openrouter");
        assert!(openrouter
            .authorize_url
            .starts_with("https://openrouter.ai/auth?callback_url="));
        assert!(openrouter
            .authorize_url
            .contains("code_challenge_method=S256"));

        let claude = oauth_pkce_start("claude_code").expect("claude pkce start");
        assert_eq!(claude.provider_id, "claude_code");
        assert_eq!(claude.redirect_uri, "http://localhost:54545/callback");
        assert!(claude
            .authorize_url
            .starts_with("https://claude.ai/oauth/authorize?"));
        assert!(claude.authorize_url.contains("code_challenge_method=S256"));

        // Release the test listener cleanly so ports 1455/54545 are never left occupied.
        let prev_port = PKCE_LISTENER_PORT.load(Ordering::SeqCst);
        PKCE_LISTENER_GENERATION.fetch_add(1, Ordering::SeqCst);
        PKCE_LISTENER_ACTIVE.store(false, Ordering::SeqCst);
        wake_loopback_port(prev_port);
        let mut map = pkce_sessions().lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&codex.session_id);
        map.remove(&openrouter.session_id);
        map.remove(&claude.session_id);
    }

    #[test]
    fn parse_codex_usage_response_and_auth_meta_work() {
        let usage = serde_json::json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 22.4,
                    "limit_window_seconds": 18000,
                    "reset_after_seconds": 3600
                },
                "secondary_window": {
                    "used_percent": 65.0,
                    "limit_window_seconds": 604800,
                    "reset_at": 1_900_000_000
                }
            },
            "rate_limit_reset_credits": { "available_count": 2 },
            "credits": { "balance": "12.50", "unlimited": false }
        });
        let quota = parse_codex_usage_response(
            "openai-codex-oauth",
            Some("acct_42".into()),
            &usage,
            1_800_000_000,
            1_800_000_000_000,
        );
        assert_eq!(quota.plan_type.as_deref(), Some("pro"));
        assert_eq!(quota.primary_window.as_ref().unwrap().remaining_percent, 78);
        assert_eq!(
            quota.primary_window.as_ref().unwrap().window_minutes,
            Some(300)
        );
        assert_eq!(
            quota.primary_window.as_ref().unwrap().reset_at_secs,
            Some(1_800_003_600)
        );
        assert_eq!(
            quota.secondary_window.as_ref().unwrap().remaining_percent,
            35
        );
        assert_eq!(
            quota.secondary_window.as_ref().unwrap().window_minutes,
            Some(10080)
        );
        assert_eq!(quota.reset_credits_available, Some(2));
        assert_eq!(quota.credits_balance.as_deref(), Some("12.50"));

        let sub_secret = serde_json::to_string(&OAuthKeyEnvelope {
            kind: EOSTAGE_OAUTH_ENVELOPE_KIND.into(),
            provider: "openai_codex".into(),
            access_token: "tok".into(),
            refresh_token: None,
            expires_at_ms: None,
            account_id: Some("acct_sub".into()),
            copilot_api_token: None,
            copilot_expires_at_ms: None,
        })
        .unwrap();
        let sub_meta = classify_channel_secret_auth_meta(Some(&sub_secret));
        assert_eq!(sub_meta.auth_kind, "subscription");
        assert_eq!(sub_meta.oauth_provider.as_deref(), Some("openai_codex"));

        let api_meta = classify_channel_secret_auth_meta(Some("sk-or-v1-abc"));
        assert_eq!(api_meta.auth_kind, "api");
        assert_eq!(api_meta.oauth_provider, None);

        let claude_usage = serde_json::json!({
            "five_hour": { "utilization": 37.0, "resets_at": "2026-11-24T12:00:00Z" },
            "seven_day": { "utilization": 26.0, "resets_at": "2026-11-30T00:00:00Z" },
            "extra_usage": { "is_enabled": true, "monthly_limit": 20.0, "used_credits": 4.5 }
        });
        let cq = parse_claude_usage_response(
            "claude-oauth",
            Some("user@example.edu".into()),
            &claude_usage,
            1_800_000_000_000,
        );
        assert_eq!(cq.oauth_provider, "claude_code");
        assert_eq!(cq.primary_window.as_ref().unwrap().remaining_percent, 63);
        assert_eq!(cq.secondary_window.as_ref().unwrap().remaining_percent, 74);
        assert_eq!(cq.credits_balance.as_deref(), Some("15.50"));

        let copilot_usage = serde_json::json!({
            "copilot_plan": "individual",
            "quota_reset_date_utc": "2026-12-01T00:00:00Z",
            "quota_snapshots": {
                "premium_interactions": { "entitlement": 150, "remaining": 142, "percent_remaining": 94.6, "unlimited": false },
                "chat": { "unlimited": true }
            }
        });
        let pq =
            parse_copilot_usage_response("github-copilot-oauth", &copilot_usage, 1_800_000_000_000);
        assert_eq!(pq.oauth_provider, "github_copilot");
        assert_eq!(pq.plan_type.as_deref(), Some("individual"));
        assert_eq!(pq.primary_window.as_ref().unwrap().remaining_percent, 95);
        assert_eq!(pq.secondary_window.as_ref().unwrap().remaining_percent, 100);
        assert_eq!(pq.credits_balance, None);
    }

    #[test]
    fn oauth_rejects_pruned_unsupported_providers() {
        for unsupported in [
            "google_gemini",
            "google_gemini_pkce",
            "kimi_code",
            "kimi_code_device",
        ] {
            assert!(
                oauth_pkce_start(unsupported).is_err(),
                "PKCE must reject pruned provider {unsupported}"
            );
        }

        let root = std::env::temp_dir().join(format!(
            "eostage-test-oauth-pruned-device-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&root);
        let app = AgentApplication(std::sync::Arc::new(
            insar_application_service::ApplicationService::new(
                insar_application_service::ApplicationServiceConfig::store(root.join("s.sqlite")),
            ),
        ));
        for unsupported in ["kimi_code", "google_gemini"] {
            let err = oauth_device_start(&app, unsupported)
                .expect_err("device start must reject pruned provider");
            assert!(err.contains("不支持的设备授权平台"));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn oauth_channel_configs_have_zero_prefilled_models_and_emit_bearer_headers() {
        for cfg in [
            openai_codex_channel_config(),
            github_copilot_channel_config(),
            openrouter_channel_config(),
        ] {
            assert!(cfg.model.is_empty(), "channel {}", cfg.id);
            assert!(cfg.models.is_empty(), "channel {}", cfg.id);
        }

        let root = std::env::temp_dir().join(format!("eostage-oauth-route-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let app = AgentApplication(std::sync::Arc::new(
            insar_application_service::ApplicationService::new(
                insar_application_service::ApplicationServiceConfig::store(root.join("s.sqlite")),
            ),
        ));

        let claude_env = serde_json::to_string(&OAuthKeyEnvelope {
            kind: EOSTAGE_OAUTH_ENVELOPE_KIND.into(),
            provider: "claude_code".into(),
            access_token: "claude-tok".into(),
            refresh_token: Some("claude-ref".into()),
            expires_at_ms: Some(u64::MAX / 2),
            account_id: None,
            copilot_api_token: None,
            copilot_expires_at_ms: None,
        })
        .unwrap();
        let base_claude = ModelRoute::new(
            insar_agent_model::Protocol::AnthropicMessages,
            "https://api.anthropic.com",
            "claude-sonnet-4-5",
        )
        .unwrap();
        let (k, r) =
            resolve_runtime_secret_and_route(&app, "claude-code-cli", &claude_env, base_claude);
        assert_eq!(k.as_str(), "claude-tok");
        assert!(r
            .extra_headers
            .iter()
            .any(|(h, v)| h == "x-eostage-auth-mode" && v == "bearer"));
        assert!(r
            .extra_headers
            .iter()
            .any(|(h, v)| h == "anthropic-beta" && v == "oauth-2025-04-20"));

        let gem_env = serde_json::to_string(&OAuthKeyEnvelope {
            kind: EOSTAGE_OAUTH_ENVELOPE_KIND.into(),
            provider: "gemini_cli".into(),
            access_token: "gem-tok".into(),
            refresh_token: Some("gem-ref".into()),
            expires_at_ms: Some(4_102_444_800_000),
            account_id: None,
            copilot_api_token: None,
            copilot_expires_at_ms: None,
        })
        .unwrap();
        let base_gem = ModelRoute::new(
            insar_agent_model::Protocol::GoogleGemini,
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-pro",
        )
        .unwrap();
        let (gk, gr) = resolve_runtime_secret_and_route(&app, "gemini-cli", &gem_env, base_gem);
        assert_eq!(gk.as_str(), "gem-tok");
        assert!(gr
            .extra_headers
            .iter()
            .any(|(h, v)| h == "x-eostage-auth-mode" && v == "bearer"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn extract_jwt_exp_ms_works() {
        let payload = serde_json::json!({
            "exp": 1_800_000_000u64,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_id_fallback_9"
            }
        });
        let encoded_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let jwt = format!("eyJhbGciOiJSUzI1NiJ9.{encoded_payload}.sig");
        assert_eq!(extract_jwt_exp_ms(&jwt), Some(1_800_000_000_000u64));
        assert_eq!(
            extract_chatgpt_account_id_from_jwt(&jwt).as_deref(),
            Some("acct_id_fallback_9")
        );
    }

    #[derive(Default)]
    struct MockKeys(Mutex<HashMap<String, String>>);

    impl insar_agent_model::channels::ChannelKeys for MockKeys {
        fn save_key(
            &self,
            channel_id: &str,
            key: &str,
        ) -> Result<(), insar_agent_model::channels::ChannelError> {
            self.0
                .lock()
                .unwrap()
                .insert(channel_id.to_owned(), key.to_owned());
            Ok(())
        }
        fn has_key(
            &self,
            channel_id: &str,
        ) -> Result<bool, insar_agent_model::channels::ChannelError> {
            Ok(self.0.lock().unwrap().contains_key(channel_id))
        }
        fn delete_key(
            &self,
            channel_id: &str,
        ) -> Result<bool, insar_agent_model::channels::ChannelError> {
            Ok(self.0.lock().unwrap().remove(channel_id).is_some())
        }
    }

    #[test]
    fn upsert_channel_config_never_sets_empty_model_oauth_as_default() {
        let root = std::env::temp_dir().join(format!(
            "eostage-test-oauth-no-default-hijack-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::create_dir_all(&root);

        let codex = openai_codex_channel_config();
        assert!(codex.model.is_empty());
        assert!(codex.models.is_empty());

        let mock_keys = MockKeys::default();
        upsert_channel_config_with(&root, codex, "dummy-secret", &mock_keys).unwrap();
        let store = ChannelStore::new(root.join("agent"));
        assert_eq!(
            store.default_id().unwrap(),
            None,
            "OAuth channel without configured models must not silently become default"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
