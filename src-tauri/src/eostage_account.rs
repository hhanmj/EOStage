//! Native Rust client for EOStage official account authentication, revocable
//! session lifecycle, remote billing configuration, orders, credit redemption,
//! usage billing logs, and official model catalog synchronization.
//!
//! Security boundary: revocable session bearer tokens (`eos_sess_...`) are stored
//! exclusively inside the OS credential store via `DesktopKeys` under
//! `OFFICIAL_CHANNEL_ID` and never serialized back to WebView IPC responses.

use base64::Engine as _;
use insar_agent_model::channels::{
    ChannelConfig, ChannelKeys, ChannelModel, ChannelStore, ReasoningLevel,
};
use insar_agent_model::ReasoningMode;
use insar_application_service::AgentApplication;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agent::{AgentChannelView, AgentHostState, DesktopKeys};

pub const OFFICIAL_CHANNEL_ID: &str = "eostage-official";
const LEGACY_OFFICIAL_CHANNEL_ID: &str = "eostage-plan";
const DEFAULT_EOSTAGE_API_ORIGIN: &str = "https://api.eostage.org";
const ACCOUNT_CACHE_FILENAME: &str = "eostage-account.json";
const DEVICE_ID_FILENAME: &str = "device-id";

pub fn is_official_channel_id(channel_id: &str) -> bool {
    matches!(
        channel_id.trim(),
        OFFICIAL_CHANNEL_ID | LEGACY_OFFICIAL_CHANNEL_ID
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct EOStageAccountStatus {
    pub logged_in: bool,
    pub session_expired: bool,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub status: String,
    pub trial_granted: bool,
    pub total_credits: u64,
    pub remaining_credits: u64,
    pub trial_credits: u64,
    pub account_credits: u64,
    pub used_credits: u64,
    pub initial_trial_credits: u64,
    pub total_recharged_credits: u64,
    pub trial_expires_at: Option<u64>,
    pub active_sessions: u32,
    pub proxy_base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EOStageLoginStartResult {
    pub session_id: String,
    pub authorize_url: String,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EOStageLoginPollResult {
    pub status: String,
    pub account: Option<EOStageAccountStatus>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EOStagePlan {
    pub id: String,
    pub title: String,
    pub price_cny: f64,
    pub credits: u64,
    #[serde(default)]
    pub validity_days: Option<u64>,
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub checkout_url: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EOStageBillingConfig {
    pub config_version: u32,
    pub plans_enabled: bool,
    pub orders_enabled: bool,
    pub redeem_enabled: bool,
    pub credits_per_cny: u64,
    pub trial_credits: u64,
    pub trial_expire_days: u64,
    pub currency: String,
    pub currency_symbol: String,
    pub checkout_provider: String,
    #[serde(default)]
    pub checkout_notice: String,
    #[serde(default)]
    pub announcement: String,
    #[serde(default)]
    pub plans: Vec<EOStagePlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EOStageOrder {
    pub order_no: String,
    pub plan_id: String,
    pub title: String,
    pub amount_fen: u64,
    pub credits: u64,
    pub provider: String,
    pub status: String,
    #[serde(default)]
    pub pay_url: String,
    pub created_at: u64,
    #[serde(default)]
    pub paid_at: Option<u64>,
    #[serde(default)]
    pub cancelled_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EOStageRedeemResult {
    pub added_credits: u64,
    pub account: EOStageAccountStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EOStageUsageItem {
    pub id: String,
    pub request_id: String,
    pub model: String,
    pub created_at: u64,
    pub credits: f64,
    pub quota_units: u64,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub use_time_ms: u64,
    #[serde(default)]
    pub uncached_input_credits_per_1m: Option<u64>,
    #[serde(default)]
    pub cached_input_credits_per_1m: Option<u64>,
    #[serde(default)]
    pub output_credits_per_1m: Option<u64>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EOStageUsageListResult {
    pub items: Vec<EOStageUsageItem>,
}

#[derive(Debug, Clone)]
struct LoginBrowserSession {
    device_hash: String,
    captured_ticket: Option<String>,
    captured_error: Option<String>,
    expires_at_ms: u64,
}

static LOGIN_SESSIONS: OnceLock<Mutex<HashMap<String, LoginBrowserSession>>> = OnceLock::new();

fn login_sessions() -> &'static Mutex<HashMap<String, LoginBrowserSession>> {
    LOGIN_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn api_origin() -> String {
    std::env::var("EOSTAGE_ACCOUNT_API_BASE")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_EOSTAGE_API_ORIGIN.to_owned())
}

pub(crate) fn proxy_base_url() -> String {
    format!("{}/eostage/v1/proxy/v1", api_origin())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn random_base64url(num_bytes: usize) -> String {
    let mut bytes = vec![0u8; num_bytes];
    let _ = SystemRandom::new().fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
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

pub fn device_hash_for_root(data_root: &Path) -> Result<String, String> {
    let agent_dir = data_root.join("agent");
    std::fs::create_dir_all(&agent_dir).map_err(|e| format!("无法创建应用目录: {e}"))?;
    let id_path = agent_dir.join(DEVICE_ID_FILENAME);
    let raw_secret = match std::fs::read_to_string(&id_path) {
        Ok(existing) if existing.trim().len() >= 32 => existing.trim().to_owned(),
        _ => {
            let mut buf = [0u8; 32];
            SystemRandom::new()
                .fill(&mut buf)
                .map_err(|_| "生成设备标识失败".to_owned())?;
            let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
            let tmp = agent_dir.join("device-id.tmp");
            std::fs::write(&tmp, &hex).map_err(|e| format!("保存设备标识失败: {e}"))?;
            std::fs::rename(&tmp, &id_path).map_err(|e| format!("保存设备标识失败: {e}"))?;
            hex
        }
    };
    let digest = Sha256::digest(format!("eostage-device-v1:{raw_secret}").as_bytes());
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn cache_path(data_root: &Path) -> std::path::PathBuf {
    data_root.join("agent").join(ACCOUNT_CACHE_FILENAME)
}

fn write_cached_account(data_root: &Path, status: &EOStageAccountStatus) {
    let dir = data_root.join("agent");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(bytes) = serde_json::to_vec_pretty(status) {
        let tmp = dir.join("eostage-account.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, cache_path(data_root));
        }
    }
}

fn read_cached_account(data_root: &Path) -> Option<EOStageAccountStatus> {
    let bytes = std::fs::read(cache_path(data_root)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn clear_local_official_account_state_with(data_root: &Path, keys: &impl ChannelKeys) {
    let store = ChannelStore::new(data_root.join("agent"));
    let _ = store.delete(OFFICIAL_CHANNEL_ID, keys);
    let _ = store.delete(LEGACY_OFFICIAL_CHANNEL_ID, keys);
    let _ = std::fs::remove_file(cache_path(data_root));
}

fn clear_local_official_account_state(data_root: &Path) {
    clear_local_official_account_state_with(data_root, &DesktopKeys);
}

fn read_session_token() -> Result<Option<String>, String> {
    insar_download::with_desktop_agent_model_key(OFFICIAL_CHANNEL_ID, |key| key.to_owned())
        .map_err(|_| "系统凭据库不可用".to_owned())
}

fn parse_account_json(value: &Value, logged_in: bool) -> EOStageAccountStatus {
    EOStageAccountStatus {
        logged_in,
        session_expired: false,
        email: value
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_owned),
        display_name: value
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        status: value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or(if logged_in { "active" } else { "logged_out" })
            .to_owned(),
        trial_granted: value
            .get("trial_granted")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        total_credits: value
            .get("total_credits")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        remaining_credits: value
            .get("remaining_credits")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        trial_credits: value
            .get("trial_credits")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        account_credits: value
            .get("account_credits")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        used_credits: value
            .get("used_credits")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        initial_trial_credits: value
            .get("initial_trial_credits")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        total_recharged_credits: value
            .get("total_recharged_credits")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        trial_expires_at: value.get("trial_expires_at").and_then(Value::as_u64),
        active_sessions: value
            .get("active_sessions")
            .and_then(Value::as_u64)
            .unwrap_or(1) as u32,
        proxy_base_url: Some(proxy_base_url()),
    }
}

fn channel_model_from_remote(val: &Value) -> Option<ChannelModel> {
    let id = val.get("id")?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }
    let name = val
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let protocol = val
        .get("protocol")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let context_window = val.get("context_window").and_then(Value::as_u64);
    let max_output_tokens = val.get("max_output_tokens").and_then(Value::as_u64);
    let mut input_types: Vec<String> = val
        .get("input_types")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| vec!["text".to_owned()]);
    if input_types.is_empty() {
        input_types.push("text".to_owned());
    }
    let reasoning_levels: Vec<ReasoningLevel> = val
        .get("reasoning_levels")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let level = item.get("level")?.as_str()?.trim().to_owned();
                    if level.is_empty() {
                        return None;
                    }
                    let value = item.get("value").and_then(Value::as_str).map(str::to_owned);
                    Some(ReasoningLevel { level, value })
                })
                .collect()
        })
        .unwrap_or_default();

    let is_free = val.get("is_free").and_then(Value::as_bool).or_else(|| {
        let rate_in = val.get("rate_input_credits_per_1m").and_then(Value::as_u64);
        let rate_out = val
            .get("rate_output_credits_per_1m")
            .and_then(Value::as_u64);
        match (rate_in, rate_out) {
            (Some(0), Some(0)) => Some(true),
            (Some(_), _) | (_, Some(_)) => Some(false),
            (None, None) => None,
        }
    });
    Some(ChannelModel {
        id: id.to_owned(),
        name,
        protocol,
        context_window,
        max_output_tokens,
        input_types,
        reasoning_levels,
        is_free,
        supports_tools: val.get("supports_tools").and_then(Value::as_bool),
        supports_structured_output: val
            .get("supports_structured_output")
            .and_then(Value::as_bool),
        supports_web_search: val.get("supports_web_search").and_then(Value::as_bool),
        supports_mid_turn_system: val.get("supports_mid_turn_system").and_then(Value::as_bool),
        reasoning_param_mapping: val
            .get("reasoning_param_mapping")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// Merge server-advertised official models with any user-customized local
/// `ChannelModel` overrides so custom context limits / thinking levels survive sync.
pub fn merge_official_models(existing: &[ChannelModel], remote: &[Value]) -> Vec<ChannelModel> {
    let existing_by_id: HashMap<&str, &ChannelModel> =
        existing.iter().map(|m| (m.id.as_str(), m)).collect();
    let mut merged: Vec<ChannelModel> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for raw in remote {
        let Some(remote_model) = channel_model_from_remote(raw) else {
            continue;
        };
        if !seen.insert(remote_model.id.clone()) {
            continue;
        }
        if let Some(prev) = existing_by_id.get(remote_model.id.as_str()) {
            merged.push(ChannelModel {
                id: remote_model.id,
                name: prev.name.clone().or(remote_model.name),
                protocol: prev.protocol.clone().or(remote_model.protocol),
                context_window: prev.context_window.or(remote_model.context_window),
                max_output_tokens: prev.max_output_tokens.or(remote_model.max_output_tokens),
                input_types: if prev.input_types.is_empty() {
                    remote_model.input_types
                } else {
                    prev.input_types.clone()
                },
                reasoning_levels: if prev.reasoning_levels.is_empty() {
                    remote_model.reasoning_levels
                } else {
                    prev.reasoning_levels.clone()
                },
                supports_tools: prev.supports_tools.or(remote_model.supports_tools),
                supports_structured_output: prev
                    .supports_structured_output
                    .or(remote_model.supports_structured_output),
                supports_web_search: prev
                    .supports_web_search
                    .or(remote_model.supports_web_search),
                supports_mid_turn_system: prev
                    .supports_mid_turn_system
                    .or(remote_model.supports_mid_turn_system),
                is_free: remote_model.is_free,
                reasoning_param_mapping: prev
                    .reasoning_param_mapping
                    .clone()
                    .or(remote_model.reasoning_param_mapping),
            });
        } else {
            merged.push(remote_model);
        }
    }

    for prev in existing {
        if seen.insert(prev.id.clone()) {
            let mut retained = (*prev).clone();
            if !remote.is_empty() {
                retained.is_free = None;
            }
            merged.push(retained);
        }
    }
    merged
}

pub fn extract_ticket_from_input(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("eos_tkt_") && !trimmed.contains(char::is_whitespace) {
        return Some(trimmed.to_owned());
    }
    let query_part = trimmed.split_once('?').map(|(_, q)| q).unwrap_or(trimmed);
    for pair in query_part.replace(['?', '#'], "&").split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        if k == "ticket" {
            let decoded = percent_decode(v.trim());
            if decoded.starts_with("eos_tkt_") {
                return Some(decoded);
            }
        }
    }
    None
}

fn upsert_official_channel_with_keys(
    data_root: &Path,
    remote_models: &[Value],
    new_session_token: Option<&str>,
    keys: &impl ChannelKeys,
) -> Result<(), String> {
    let store = ChannelStore::new(data_root.join("agent"));
    let _ = store.delete(LEGACY_OFFICIAL_CHANNEL_ID, keys);
    let existing = store
        .saved(OFFICIAL_CHANNEL_ID)
        .map_err(|_| "读取官方渠道配置失败".to_owned())?;
    let existing_default = store.default_id().ok().flatten();
    let existing_channels = store.list(keys).unwrap_or_default();

    let merged_models = existing
        .as_ref()
        .map(|c| merge_official_models(&c.models, remote_models))
        .unwrap_or_else(|| merge_official_models(&[], remote_models));

    let default_model = existing
        .as_ref()
        .map(|c| c.model.trim().to_owned())
        .filter(|m| !m.is_empty() && merged_models.iter().any(|x| x.id == *m))
        .or_else(|| merged_models.first().map(|m| m.id.clone()))
        .unwrap_or_else(|| "glm-5.3-flash".to_owned());

    let channel = ChannelConfig {
        id: OFFICIAL_CHANNEL_ID.to_owned(),
        protocol: existing
            .as_ref()
            .map(|c| c.protocol.clone())
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| "chat_completions".to_owned()),
        base_url: proxy_base_url(),
        model: default_model,
        reasoning_mode: existing
            .as_ref()
            .map(|c| c.reasoning_mode)
            .unwrap_or(ReasoningMode::ProviderDefault),
        reasoning_switch_unsupported: existing
            .as_ref()
            .is_some_and(|c| c.reasoning_switch_unsupported),
        models: merged_models,
    };

    store
        .save(channel, new_session_token, keys)
        .map_err(|_| "保存 EOStage 官方渠道凭据失败".to_owned())?;

    if existing_default.is_none() || existing_channels.is_empty() {
        let _ = store.set_default(OFFICIAL_CHANNEL_ID);
    }
    Ok(())
}

fn upsert_official_channel(
    data_root: &Path,
    remote_models: &[Value],
    new_session_token: Option<&str>,
) -> Result<(), String> {
    upsert_official_channel_with_keys(data_root, remote_models, new_session_token, &DesktopKeys)
}

fn spawn_login_loopback_listener(session_id: String, expected_state: String) -> Option<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let _ = listener.set_nonblocking(true);

    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(300);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                    let mut buf = [0u8; 4096];
                    if let Ok(n) = stream.read(&mut buf) {
                        if n == 0 {
                            continue;
                        }
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let target = req
                            .lines()
                            .next()
                            .and_then(|l| l.split_whitespace().nth(1))
                            .unwrap_or("");
                        if target.starts_with("/favicon.ico") {
                            let _ = stream.write_all(
                                b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            );
                            continue;
                        }

                        let mut ticket_val: Option<String> = None;
                        let mut state_val: Option<String> = None;
                        let mut err_val: Option<String> = None;
                        if let Some((_, q)) = target.split_once('?') {
                            for pair in q.split('&') {
                                let Some((k, v)) = pair.split_once('=') else {
                                    continue;
                                };
                                let decoded = percent_decode(v);
                                match k {
                                    "ticket" => ticket_val = Some(decoded),
                                    "state" => state_val = Some(decoded),
                                    "error" => err_val = Some(decoded),
                                    _ => {}
                                }
                            }
                        }

                        let mut map = login_sessions().lock().unwrap_or_else(|p| p.into_inner());
                        let html = if let Some(sess) = map.get_mut(&session_id) {
                            if let Some(err) = err_val {
                                sess.captured_error = Some(err);
                                "<!doctype html><html><head><meta charset='utf-8'><title>登录未完成</title></head><body style='font-family:system-ui,-apple-system,sans-serif;padding:40px;color:#18181b;'><h2>登录未完成</h2><p>请关闭此页面并返回 EOStage 桌面端重试。</p></body></html>"
                            } else if state_val.as_deref() == Some(expected_state.as_str()) {
                                if let Some(tkt) = ticket_val.filter(|t| t.starts_with("eos_tkt_"))
                                {
                                    sess.captured_ticket = Some(tkt);
                                    "<!doctype html><html><head><meta charset='utf-8'><title>登录成功 - EOStage</title></head><body style='font-family:system-ui,-apple-system,sans-serif;padding:40px;color:#18181b;'><h2>EOStage 账号登录成功</h2><p>桌面端已自动接收授权，您可以关闭此标签页返回工作台。</p><script>setTimeout(function(){try{window.close();}catch(e){}},1200);</script></body></html>"
                                } else {
                                    sess.captured_error = Some("未检测到有效的登录票据".into());
                                    "<!doctype html><html><head><meta charset='utf-8'><title>登录失败</title></head><body style='font-family:system-ui;padding:40px;'><h2>未检测到登录票据</h2></body></html>"
                                }
                            } else {
                                sess.captured_error = Some("登录状态校验失败，请重试".into());
                                "<!doctype html><html><head><meta charset='utf-8'><title>校验失败</title></head><body style='font-family:system-ui;padding:40px;'><h2>State 校验失败</h2></body></html>"
                            }
                        } else {
                            "<!doctype html><html><head><meta charset='utf-8'></head><body style='font-family:system-ui;padding:40px;'><h3>登录会话已结束</h3></body></html>"
                        };
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
                            html.len(),
                            html
                        );
                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.flush();
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(120));
                    let active = login_sessions()
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .contains_key(&session_id);
                    if !active {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    Some(port)
}

fn exchange_ticket_and_activate(
    application: &AgentApplication,
    data_root: &Path,
    ticket: &str,
    device_hash: &str,
) -> Result<EOStageAccountStatus, String> {
    let client = crate::oauth_login::build_http_client(application, Duration::from_secs(20))?;
    let url = format!("{}/eostage/v1/auth/exchange", api_origin());
    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "ticket": ticket,
            "device_hash": device_hash,
            "client_info": format!("eostage-desktop-{}", std::env::consts::OS),
        }))
        .send()
        .map_err(|e| format!("连接 EOStage 账号服务失败: {e}"))?;
    let status_code = resp.status().as_u16();
    let payload: Value = resp
        .json()
        .map_err(|e| format!("解析 EOStage 登录响应失败: {e}"))?;
    if status_code >= 300 || payload.get("ok").and_then(Value::as_bool) != Some(true) {
        let msg = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("登录凭据兑换失败");
        return Err(msg.to_owned());
    }

    let session_token = payload
        .get("session_token")
        .and_then(Value::as_str)
        .filter(|s| s.starts_with("eos_sess_"))
        .ok_or_else(|| "服务端未返回有效会话令牌".to_owned())?;
    let account_val = payload
        .get("account")
        .ok_or_else(|| "服务端未返回账号信息".to_owned())?;
    let remote_models = payload
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    upsert_official_channel(data_root, &remote_models, Some(session_token))?;
    let account = parse_account_json(account_val, true);
    write_cached_account(data_root, &account);
    Ok(account)
}

pub fn account_status(
    application: &AgentApplication,
    refresh: bool,
) -> Result<EOStageAccountStatus, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let token = match read_session_token()? {
        Some(t) if !t.trim().is_empty() => t,
        _ => {
            return Ok(EOStageAccountStatus {
                logged_in: false,
                session_expired: false,
                status: "logged_out".to_owned(),
                ..EOStageAccountStatus::default()
            });
        }
    };

    if !refresh {
        if let Some(mut cached) = read_cached_account(&data_root) {
            cached.logged_in = true;
            cached.session_expired = false;
            return Ok(cached);
        }
    }

    let client = crate::oauth_login::build_http_client(application, Duration::from_secs(15))?;
    let url = format!("{}/eostage/v1/account/me", api_origin());
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .map_err(|e| format!("获取账号余额失败: {e}"))?;

    let status_code = resp.status().as_u16();
    if status_code == 401 {
        clear_local_official_account_state(&data_root);
        return Ok(EOStageAccountStatus {
            logged_in: false,
            session_expired: true,
            status: "logged_out".to_owned(),
            ..EOStageAccountStatus::default()
        });
    }
    let payload: Value = resp.json().map_err(|e| format!("解析账号信息失败: {e}"))?;
    if status_code >= 300 || payload.get("ok").and_then(Value::as_bool) != Some(true) {
        let msg = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("获取账号信息失败");
        return Err(msg.to_owned());
    }

    let account = parse_account_json(&payload, true);
    write_cached_account(&data_root, &account);
    Ok(account)
}

pub fn login_start(application: &AgentApplication) -> Result<EOStageLoginStartResult, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let device_hash = device_hash_for_root(&data_root)?;
    let session_id = format!("eos_login_{}", random_base64url(12));
    let state = random_base64url(16);
    let expires_in_secs = 300;

    let port = spawn_login_loopback_listener(session_id.clone(), state.clone())
        .ok_or_else(|| "无法启动本地安全回调监听".to_owned())?;
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let authorize_url = format!(
        "{}/auth/login?redirect_uri={}&state={}&device_hash={}",
        api_origin(),
        percent_encode(&redirect_uri),
        percent_encode(&state),
        percent_encode(&device_hash),
    );

    let mut map = login_sessions().lock().unwrap_or_else(|p| p.into_inner());
    map.retain(|_, s| s.expires_at_ms > now_ms());
    map.insert(
        session_id.clone(),
        LoginBrowserSession {
            device_hash,
            captured_ticket: None,
            captured_error: None,
            expires_at_ms: now_ms().saturating_add(expires_in_secs * 1000),
        },
    );

    Ok(EOStageLoginStartResult {
        session_id,
        authorize_url,
        expires_in_secs,
    })
}

pub fn login_poll(
    application: &AgentApplication,
    session_id: &str,
) -> Result<EOStageLoginPollResult, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let (captured_ticket, captured_error, device_hash, expired) = {
        let mut map = login_sessions().lock().unwrap_or_else(|p| p.into_inner());
        let Some(sess) = map.get_mut(session_id) else {
            return Ok(EOStageLoginPollResult {
                status: "expired".to_owned(),
                account: None,
                message: Some("登录会话已过期，请重新发起登录".to_owned()),
            });
        };
        if now_ms() >= sess.expires_at_ms {
            map.remove(session_id);
            return Ok(EOStageLoginPollResult {
                status: "expired".to_owned(),
                account: None,
                message: Some("登录会话已超时，请重新获取登录链接".to_owned()),
            });
        }
        let err = sess.captured_error.take();
        let tkt = sess.captured_ticket.take();
        let dh = sess.device_hash.clone();
        if err.is_some() || tkt.is_some() {
            map.remove(session_id);
        }
        (tkt, err, dh, false)
    };
    let _ = expired;

    if let Some(err) = captured_error {
        return Ok(EOStageLoginPollResult {
            status: "error".to_owned(),
            account: None,
            message: Some(err),
        });
    }
    let Some(ticket) = captured_ticket else {
        return Ok(EOStageLoginPollResult {
            status: "pending".to_owned(),
            account: None,
            message: None,
        });
    };

    let account = exchange_ticket_and_activate(application, &data_root, &ticket, &device_hash)?;
    Ok(EOStageLoginPollResult {
        status: "authorized".to_owned(),
        account: Some(account),
        message: Some("登录成功".to_owned()),
    })
}

pub fn login_complete_code(
    application: &AgentApplication,
    code_or_url: &str,
) -> Result<EOStageAccountStatus, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let ticket = extract_ticket_from_input(code_or_url)
        .ok_or_else(|| "未识别到有效的登录码，请完整复制网页端显示的登录码".to_owned())?;
    let device_hash = device_hash_for_root(&data_root)?;
    exchange_ticket_and_activate(application, &data_root, &ticket, &device_hash)
}

pub fn account_logout(application: &AgentApplication) -> Result<EOStageAccountStatus, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    if let Ok(Some(token)) = read_session_token() {
        if let Ok(client) =
            crate::oauth_login::build_http_client(application, Duration::from_secs(6))
        {
            let url = format!("{}/eostage/v1/auth/logout", api_origin());
            let _ = client
                .post(&url)
                .bearer_auth(token)
                .json(&serde_json::json!({}))
                .send();
        }
    }
    clear_local_official_account_state(&data_root);
    Ok(EOStageAccountStatus {
        logged_in: false,
        session_expired: false,
        status: "logged_out".to_owned(),
        ..EOStageAccountStatus::default()
    })
}

fn sync_official_models_with<K, F>(
    data_root: &Path,
    session_token: Option<&str>,
    origin: &str,
    client: &reqwest::blocking::Client,
    keys: &K,
    refresh_views: F,
) -> Result<Vec<AgentChannelView>, String>
where
    K: ChannelKeys,
    F: FnOnce() -> Result<Vec<AgentChannelView>, String>,
{
    let token = session_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "请先登录 EOStage 账号".to_owned())?;
    let url = format!("{}/eostage/v1/models", origin.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .map_err(|e| format!("拉取官方模型目录失败: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 {
        clear_local_official_account_state_with(data_root, keys);
        return Err("登录状态已失效，请重新登录".to_owned());
    }
    let payload: Value = resp
        .json()
        .map_err(|e| format!("解析官方模型目录失败: {e}"))?;
    if status >= 300 || payload.get("ok").and_then(Value::as_bool) == Some(false) {
        let msg = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("拉取官方模型目录失败");
        return Err(msg.to_owned());
    }
    let remote_models = payload
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "官方模型目录格式异常".to_owned())?;
    upsert_official_channel_with_keys(data_root, &remote_models, None, keys)?;
    refresh_views()
}

pub fn sync_official_models(
    application: &AgentApplication,
    host: &AgentHostState,
) -> Result<Vec<AgentChannelView>, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let token = require_bearer_token()?;
    let client = crate::oauth_login::build_http_client(application, Duration::from_secs(15))?;
    sync_official_models_with(
        &data_root,
        Some(&token),
        &api_origin(),
        &client,
        &DesktopKeys,
        || {
            host.channels_get()
                .map_err(|e| format!("刷新本地模型列表失败: {e:?}"))
        },
    )
}

pub fn billing_config(application: &AgentApplication) -> Result<EOStageBillingConfig, String> {
    let client = crate::oauth_login::build_http_client(application, Duration::from_secs(15))?;
    let url = format!("{}/eostage/v1/billing/config", api_origin());
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("获取计费配置失败: {e}"))?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().map_err(|e| format!("解析计费配置失败: {e}"))?;
    if status >= 300 || payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("获取计费配置失败".to_owned());
    }
    serde_json::from_value(payload).map_err(|e| format!("解析计费结构失败: {e}"))
}

fn require_bearer_token() -> Result<String, String> {
    read_session_token()?
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "请先登录 EOStage 账号".to_owned())
}

pub fn billing_redeem(
    application: &AgentApplication,
    code: &str,
) -> Result<EOStageRedeemResult, String> {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Err("请输入 Credits 兑换码".to_owned());
    }
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let token = require_bearer_token()?;
    let client = crate::oauth_login::build_http_client(application, Duration::from_secs(15))?;
    let url = format!("{}/eostage/v1/billing/redeem", api_origin());
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({ "code": trimmed }))
        .send()
        .map_err(|e| format!("兑换请求失败: {e}"))?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().map_err(|e| format!("解析兑换结果失败: {e}"))?;
    if status == 401 {
        clear_local_official_account_state(&data_root);
        return Err("登录状态已失效，请重新登录".to_owned());
    }
    if status >= 300 || payload.get("ok").and_then(Value::as_bool) != Some(true) {
        let msg = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("兑换失败");
        return Err(msg.to_owned());
    }
    let added_credits = payload
        .get("added_credits")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let account = payload
        .get("account")
        .map(|a| parse_account_json(a, true))
        .ok_or_else(|| "服务端未返回更新后的余额".to_owned())?;
    write_cached_account(&data_root, &account);
    Ok(EOStageRedeemResult {
        added_credits,
        account,
    })
}

pub fn billing_orders_list(application: &AgentApplication) -> Result<Vec<EOStageOrder>, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let token = require_bearer_token()?;
    let client = crate::oauth_login::build_http_client(application, Duration::from_secs(15))?;
    let url = format!("{}/eostage/v1/billing/orders", api_origin());
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .map_err(|e| format!("获取订单记录失败: {e}"))?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().map_err(|e| format!("解析订单列表失败: {e}"))?;
    if status == 401 {
        clear_local_official_account_state(&data_root);
        return Err("登录状态已失效，请重新登录".to_owned());
    }
    if status >= 300 || payload.get("ok").and_then(Value::as_bool) != Some(true) {
        let msg = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("获取订单记录失败");
        return Err(msg.to_owned());
    }
    serde_json::from_value(
        payload
            .get("orders")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])),
    )
    .map_err(|e| format!("解析订单数据失败: {e}"))
}

pub fn billing_order_create(
    application: &AgentApplication,
    plan_id: &str,
) -> Result<EOStageOrder, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let token = require_bearer_token()?;
    let client = crate::oauth_login::build_http_client(application, Duration::from_secs(15))?;
    let url = format!("{}/eostage/v1/billing/orders", api_origin());
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({ "plan_id": plan_id.trim() }))
        .send()
        .map_err(|e| format!("创建订单失败: {e}"))?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().map_err(|e| format!("解析订单失败: {e}"))?;
    if status == 401 {
        clear_local_official_account_state(&data_root);
        return Err("登录状态已失效，请重新登录".to_owned());
    }
    if status >= 300 || payload.get("ok").and_then(Value::as_bool) != Some(true) {
        let msg = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("创建订单失败");
        return Err(msg.to_owned());
    }
    serde_json::from_value(payload.get("order").cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("解析订单信息失败: {e}"))
}

pub fn billing_order_action(
    application: &AgentApplication,
    order_no: &str,
    cancel: bool,
) -> Result<EOStageOrder, String> {
    let clean_no = order_no.trim();
    if clean_no.is_empty() || clean_no.contains('/') || clean_no.contains("..") {
        return Err("无效的订单号".to_owned());
    }
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let token = require_bearer_token()?;
    let client = crate::oauth_login::build_http_client(application, Duration::from_secs(15))?;
    let url = if cancel {
        format!(
            "{}/eostage/v1/billing/orders/{clean_no}/cancel",
            api_origin()
        )
    } else {
        format!("{}/eostage/v1/billing/orders/{clean_no}", api_origin())
    };
    let req = if cancel {
        client
            .post(&url)
            .bearer_auth(token)
            .json(&serde_json::json!({}))
    } else {
        client.get(&url).bearer_auth(token)
    };
    let resp = req.send().map_err(|e| format!("订单操作失败: {e}"))?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().map_err(|e| format!("解析订单结果失败: {e}"))?;
    if status == 401 {
        clear_local_official_account_state(&data_root);
        return Err("登录状态已失效，请重新登录".to_owned());
    }
    if status >= 300 || payload.get("ok").and_then(Value::as_bool) != Some(true) {
        let msg = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("订单操作失败");
        return Err(msg.to_owned());
    }
    serde_json::from_value(payload.get("order").cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("解析订单信息失败: {e}"))
}

pub fn usage_list(
    application: &AgentApplication,
    limit: Option<u32>,
) -> Result<EOStageUsageListResult, String> {
    let data_root = application
        .data_root()
        .ok_or_else(|| "应用数据目录不可用".to_owned())?;
    let token = require_bearer_token()?;
    let client = crate::oauth_login::build_http_client(application, Duration::from_secs(15))?;
    let lim = limit.unwrap_or(30).clamp(1, 100);
    let url = format!("{}/eostage/v1/usage?limit={lim}", api_origin());
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .map_err(|e| format!("获取用量明细失败: {e}"))?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().map_err(|e| format!("解析用量明细失败: {e}"))?;
    if status == 401 {
        clear_local_official_account_state(&data_root);
        return Err("登录状态已失效，请重新登录".to_owned());
    }
    if status >= 300 || payload.get("ok").and_then(Value::as_bool) != Some(true) {
        let msg = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("获取用量明细失败");
        return Err(msg.to_owned());
    }
    let items: Vec<EOStageUsageItem> = serde_json::from_value(
        payload
            .get("items")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])),
    )
    .map_err(|e| format!("解析用量记录失败: {e}"))?;
    Ok(EOStageUsageListResult { items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_hash_is_anonymous_and_stable_across_calls() {
        let tmp = std::env::temp_dir().join(format!("eostage-devhash-{}", now_ms()));
        let first = device_hash_for_root(&tmp).expect("first hash");
        let second = device_hash_for_root(&tmp).expect("second hash");
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(first, second);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn merge_official_models_preserves_user_customized_params() {
        let existing = vec![ChannelModel {
            id: "glm-5.3-flash".into(),
            name: Some("My GLM".into()),
            protocol: None,
            context_window: Some(65536),
            max_output_tokens: None,
            input_types: vec!["text".into()],
            reasoning_levels: vec![
                ReasoningLevel {
                    level: "off".into(),
                    value: None,
                },
                ReasoningLevel {
                    level: "high".into(),
                    value: Some("high".into()),
                },
            ],
            supports_tools: Some(false),
            ..ChannelModel::default()
        }];
        let remote = vec![
            serde_json::json!({
                "id": "glm-5.3-flash",
                "name": "GLM-5.3 Flash",
                "protocol": "chat_completions",
                "context_window": 131072,
                "max_output_tokens": 16384,
                "supports_tools": true,
                "reasoning_levels": [{"level": "low", "value": "low"}]
            }),
            serde_json::json!({
                "id": "glm-6-preview",
                "name": "GLM-6 Preview",
                "context_window": 200000
            }),
        ];
        let merged = merge_official_models(&existing, &remote);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name.as_deref(), Some("My GLM"));
        assert_eq!(merged[0].context_window, Some(65536));
        assert_eq!(merged[0].max_output_tokens, Some(16384));
        assert_eq!(merged[0].supports_tools, Some(false));
        assert_eq!(merged[0].reasoning_levels.len(), 2);
        assert_eq!(merged[1].id, "glm-6-preview");
        assert_eq!(merged[1].context_window, Some(200000));
    }

    #[test]
    fn merge_official_models_syncs_server_authoritative_is_free() {
        let existing = vec![
            ChannelModel {
                id: "neohorse-1.5-flash-preview".into(),
                name: Some("My NeoHorse".into()),
                is_free: Some(false),
                ..ChannelModel::default()
            },
            ChannelModel {
                id: "glm-5.3-flash".into(),
                name: Some("GLM-5.3 Flash".into()),
                is_free: Some(true),
                ..ChannelModel::default()
            },
        ];
        let remote = vec![
            serde_json::json!({
                "id": "neohorse-1.5-flash-preview",
                "is_free": true,
                "rate_input_credits_per_1m": 0,
                "rate_output_credits_per_1m": 0
            }),
            serde_json::json!({
                "id": "glm-5.3-flash",
                "is_free": false,
                "rate_input_credits_per_1m": 803,
                "rate_output_credits_per_1m": 2811
            }),
            serde_json::json!({
                "id": "egg-alpha-003",
                "rate_input_credits_per_1m": 0,
                "rate_output_credits_per_1m": 0
            }),
        ];
        let merged = merge_official_models(&existing, &remote);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].is_free, Some(true));
        assert_eq!(merged[1].is_free, Some(false));
        assert_eq!(merged[2].is_free, Some(true));
    }

    #[test]
    fn extract_ticket_accepts_raw_ticket_and_callback_url() {
        assert_eq!(
            extract_ticket_from_input("  eos_tkt_abc123  ").as_deref(),
            Some("eos_tkt_abc123")
        );
        assert_eq!(
            extract_ticket_from_input(
                "http://127.0.0.1:41234/callback?ticket=eos_tkt_xyz&state=s1"
            )
            .as_deref(),
            Some("eos_tkt_xyz")
        );
        assert_eq!(extract_ticket_from_input("sk-wrong-key"), None);
    }

    #[derive(Default)]
    struct MockKeys(Mutex<HashMap<String, String>>);

    impl ChannelKeys for MockKeys {
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
    fn sync_official_models_requires_session_and_checks_http_status() {
        let tmp = std::env::temp_dir().join(format!(
            "eostage-sync-models-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let keys = MockKeys::default();
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();

        // 1. Logged-out call fails before touching network or ChannelStore
        let err =
            sync_official_models_with(&tmp, None, "http://127.0.0.1:9", &client, &keys, || {
                Ok(vec![])
            })
            .unwrap_err();
        assert!(err.contains("登录"), "unexpected error: {err}");
        assert!(ChannelStore::new(tmp.join("agent"))
            .saved(OFFICIAL_CHANNEL_ID)
            .unwrap()
            .is_none());

        // Seed cached account + official channel for HTTP 500 & 401 checks
        upsert_official_channel_with_keys(
            &tmp,
            &[serde_json::json!({"id": "glm-5.3-flash"})],
            Some("eos_sess_test"),
            &keys,
        )
        .unwrap();
        write_cached_account(
            &tmp,
            &EOStageAccountStatus {
                logged_in: true,
                ..Default::default()
            },
        );

        let spawn_once = |status_line: &'static str, body: &'static str| {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let handle = std::thread::spawn(move || {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf);
                    let resp = format!(
                        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
            });
            (format!("http://127.0.0.1:{port}"), handle)
        };

        // 2. HTTP 500 fails cleanly without deleting existing channel
        let (origin_500, h500) = spawn_once(
            "500 Internal Server Error",
            r#"{"ok":false,"message":"server busy"}"#,
        );
        let err_500 = sync_official_models_with(
            &tmp,
            Some("eos_sess_test"),
            &origin_500,
            &client,
            &keys,
            || Ok(vec![]),
        )
        .unwrap_err();
        let _ = h500.join();
        assert!(err_500.contains("server busy"));
        assert!(ChannelStore::new(tmp.join("agent"))
            .saved(OFFICIAL_CHANNEL_ID)
            .unwrap()
            .is_some());

        // 3. HTTP 401 clears local official channel & cache
        let (origin_401, h401) =
            spawn_once("401 Unauthorized", r#"{"ok":false,"message":"expired"}"#);
        let err_401 = sync_official_models_with(
            &tmp,
            Some("eos_sess_test"),
            &origin_401,
            &client,
            &keys,
            || Ok(vec![]),
        )
        .unwrap_err();
        let _ = h401.join();
        assert!(err_401.contains("失效"));
        assert!(ChannelStore::new(tmp.join("agent"))
            .saved(OFFICIAL_CHANNEL_ID)
            .unwrap()
            .is_none());
        assert!(!cache_path(&tmp).exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn merge_official_models_clears_stale_free_flag_on_delisted_models() {
        let initial_remote = vec![
            serde_json::json!({
                "id": "old-free-model",
                "name": "Old Free",
                "is_free": true
            }),
            serde_json::json!({
                "id": "active-model",
                "name": "Active",
                "is_free": false
            }),
        ];
        let existing = merge_official_models(&[], &initial_remote);
        assert_eq!(existing[0].is_free, Some(true));

        let next_remote = vec![serde_json::json!({
            "id": "active-model",
            "name": "Active",
            "is_free": true
        })];
        let merged = merge_official_models(&existing, &next_remote);
        let active = merged.iter().find(|m| m.id == "active-model").unwrap();
        let delisted = merged.iter().find(|m| m.id == "old-free-model").unwrap();
        assert_eq!(active.is_free, Some(true));
        assert_eq!(delisted.is_free, None);
    }
}
