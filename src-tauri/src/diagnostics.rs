//! Opt-in sanitized error diagnostics store and exporter.
//!
//! Privacy contract:
//! - Disabled by default (`diagnostics_upload_enabled = false`); no startup ping or idle traffic.
//! - Stores at most 50 bounded, scrubbed JSONL records under `<app_data_dir>/diagnostics/error_reports.jsonl`.
//! - Never collects machine/account/AOI/prompt identifiers, credentials, or user home paths.

use insar_application_service::AgentApplication;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use crate::desktop_ui_prefs::{get_diagnostics_upload_enabled, set_diagnostics_upload_enabled};
use crate::oauth_login::build_http_client;

const DIAGNOSTICS_DIR: &str = "diagnostics";
const DIAGNOSTICS_FILE: &str = "error_reports.jsonl";
const DEFAULT_DIAGNOSTICS_ENDPOINT: &str = "https://api.eostage.org/eostage/v1/diagnostics/report";
const MAX_LOCAL_REPORTS: usize = 50;
const MAX_MESSAGE_LEN: usize = 400;
const MAX_STACK_LEN: usize = 1500;
const MAX_SOURCE_LEN: usize = 120;
const MAX_UPLOAD_BATCH_COUNT: usize = 10;
const MAX_UPLOAD_BATCH_BYTES: usize = 7000;
const DEDUP_WINDOW: Duration = Duration::from_secs(600);
const HOUR_WINDOW: Duration = Duration::from_secs(3600);
const MAX_AUTO_UPLOADS_PER_HOUR: usize = 10;

static FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static RATE_LOCK: OnceLock<Mutex<AutoUploadLimiter>> = OnceLock::new();
static PANIC_HOOK_ONCE: Once = Once::new();

#[derive(Default)]
struct AutoUploadLimiter {
    last_by_signature: HashMap<String, Instant>,
    recent_uploads: VecDeque<Instant>,
}

impl AutoUploadLimiter {
    fn allow_and_record(&mut self, signature: &str, now: Instant) -> bool {
        while let Some(&front) = self.recent_uploads.front() {
            if now.duration_since(front) > HOUR_WINDOW {
                self.recent_uploads.pop_front();
            } else {
                break;
            }
        }
        if self.recent_uploads.len() >= MAX_AUTO_UPLOADS_PER_HOUR {
            return false;
        }
        if let Some(&last) = self.last_by_signature.get(signature) {
            if now.duration_since(last) < DEDUP_WINDOW {
                return false;
            }
        }
        self.last_by_signature.insert(signature.to_owned(), now);
        self.recent_uploads.push_back(now);
        true
    }
}

fn file_guard() -> std::sync::MutexGuard<'static, ()> {
    FILE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn rate_limiter() -> std::sync::MutexGuard<'static, AutoUploadLimiter> {
    RATE_LOCK
        .get_or_init(|| Mutex::new(AutoUploadLimiter::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SanitizedErrorReport {
    pub report_id: String,
    pub created_at: String,
    pub app_version: String,
    pub os: String,
    pub arch: String,
    pub category: String,
    pub error_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    pub source: String,
    pub sanitized_message: String,
    pub sanitized_stack: String,
    #[serde(default)]
    pub uploaded: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiagnosticsRecordInput {
    pub category: String,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub http_status: Option<u16>,
    #[serde(default)]
    pub source: Option<String>,
    pub message: String,
    #[serde(default)]
    pub stack: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DiagnosticsStateView {
    pub enabled: bool,
    pub total_count: usize,
    pub pending_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DiagnosticsUploadOutcome {
    pub attempted: usize,
    pub uploaded: usize,
    pub failed: usize,
    pub message_zh: String,
}

fn diagnostics_file_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(DIAGNOSTICS_DIR).join(DIAGNOSTICS_FILE)
}

fn random_report_id() -> String {
    let mut bytes = [0u8; 8];
    let _ = SystemRandom::new().fill(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("diag-{}", hex)
}

fn truncate_chars(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_owned();
    }
    text.chars().take(max_len).collect()
}

fn normalize_category(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "frontend_error" => "frontend_error",
        "unhandled_rejection" => "unhandled_rejection",
        "react_boundary" => "react_boundary",
        "backend_error" => "backend_error",
        "rust_panic" => "rust_panic",
        _ => "backend_error",
    }
    .to_owned()
}

fn normalize_error_code(raw: Option<&str>) -> String {
    let cleaned: String = raw
        .unwrap_or("")
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "UNSPECIFIED".to_owned()
    } else {
        cleaned.to_ascii_uppercase()
    }
}

fn scrub_user_home_paths(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let rem: String = chars[i..].iter().collect();
        let lower = rem.to_ascii_lowercase();
        let prefix_len = if lower.starts_with("/users/") || lower.starts_with("/home/") {
            if lower.starts_with("/users/") {
                Some("/users/".len())
            } else {
                Some("/home/".len())
            }
        } else if rem.len() >= 10
            && chars[i].is_ascii_alphabetic()
            && chars[i + 1] == ':'
            && (chars[i + 2] == '\\' || chars[i + 2] == '/')
            && lower[3..].starts_with("users")
            && (chars.get(i + 8) == Some(&'\\') || chars.get(i + 8) == Some(&'/'))
        {
            Some(9)
        } else {
            None
        };
        if let Some(plen) = prefix_len {
            out.push_str("<HOME>");
            i += plen;
            while i < chars.len()
                && !matches!(
                    chars[i],
                    '/' | '\\'
                        | ' '
                        | '\t'
                        | '\n'
                        | '\r'
                        | ':'
                        | '"'
                        | '\''
                        | ')'
                        | ']'
                        | ','
                        | ';'
                )
            {
                i += 1;
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn scrub_query_strings(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '?'
            && i > 0
            && !chars[i - 1].is_whitespace()
            && chars.get(i + 1).is_some_and(|c| c.is_ascii_alphanumeric())
        {
            out.push_str("?<redacted>");
            i += 1;
            while i < chars.len()
                && !matches!(
                    chars[i],
                    ' ' | '\t' | '\n' | '\r' | '"' | '\'' | '>' | ')' | ']' | ','
                )
            {
                i += 1;
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/' | '=')
}

fn scrub_secrets_and_emails(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let rem: String = chars[i..].iter().collect();
        let lower = rem.to_ascii_lowercase();

        // Redact Authorization headers: Bearer / Basic <token>
        if lower.starts_with("bearer ") || lower.starts_with("basic ") {
            let head_len = if lower.starts_with("bearer ") { 7 } else { 6 };
            out.push_str(&rem[..head_len]);
            out.push_str("<redacted>");
            i += head_len;
            while i < chars.len() && is_token_char(chars[i]) {
                i += 1;
            }
            continue;
        }

        // Redact key=value / secret=value style assignments
        let mut matched_kv = None;
        for kw in [
            "api_key=",
            "apikey=",
            "access_token=",
            "refresh_token=",
            "session_token=",
            "token=",
            "password=",
            "secret=",
            "authorization=",
            "authorization:",
        ] {
            if lower.starts_with(kw) {
                matched_kv = Some(kw.len());
                break;
            }
        }
        if let Some(klen) = matched_kv {
            out.push_str(&rem[..klen]);
            i += klen;
            while i < chars.len() && chars[i] == ' ' {
                out.push(' ');
                i += 1;
            }
            out.push_str("<redacted>");
            if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let q = chars[i];
                i += 1;
                while i < chars.len() && chars[i] != q {
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
            } else {
                while i < chars.len() && is_token_char(chars[i]) {
                    i += 1;
                }
            }
            continue;
        }

        // Redact known API key / token prefixes when embedded standalone
        let at_boundary = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
        if at_boundary {
            let key_prefix = [
                "sk-ant-",
                "sk-",
                "eos_sess_",
                "eos_",
                "ghp_",
                "gho_",
                "aiza",
            ]
            .into_iter()
            .find(|p| lower.starts_with(p));
            if let Some(pref) = key_prefix {
                let mut end = i + pref.len();
                while end < chars.len()
                    && (chars[end].is_ascii_alphanumeric()
                        || chars[end] == '_'
                        || chars[end] == '-')
                {
                    end += 1;
                }
                if end - i >= 12 {
                    out.push_str("<redacted_key>");
                    i = end;
                    continue;
                }
            }
        }

        // Redact email addresses local@domain.tld
        if chars[i] == '@' && !out.is_empty() {
            let mut local_start = out.len();
            while local_start > 0 {
                let b = out.as_bytes()[local_start - 1] as char;
                if b.is_ascii_alphanumeric() || matches!(b, '.' | '_' | '-' | '+') {
                    local_start -= 1;
                } else {
                    break;
                }
            }
            if local_start < out.len() {
                let mut end = i + 1;
                let mut saw_dot = false;
                while end < chars.len()
                    && (chars[end].is_ascii_alphanumeric()
                        || chars[end] == '.'
                        || chars[end] == '-')
                {
                    if chars[end] == '.' {
                        saw_dot = true;
                    }
                    end += 1;
                }
                if saw_dot && end > i + 3 {
                    out.truncate(local_start);
                    out.push_str("<redacted_email>");
                    i = end;
                    continue;
                }
            }
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn sanitize_diagnostic_text(raw: &str, max_len: usize) -> String {
    let cleaned_ctrl: String = raw
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let no_paths = scrub_user_home_paths(&cleaned_ctrl);
    let no_query = scrub_query_strings(&no_paths);
    let no_secrets = scrub_secrets_and_emails(&no_query);
    truncate_chars(no_secrets.trim(), max_len)
}

fn read_reports_unlocked(app_data_dir: &Path) -> Vec<SanitizedErrorReport> {
    let path = diagnostics_file_path(app_data_dir);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut reports: Vec<SanitizedErrorReport> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<SanitizedErrorReport>(line).ok())
        .collect();
    if reports.len() > MAX_LOCAL_REPORTS {
        let keep_from = reports.len() - MAX_LOCAL_REPORTS;
        reports = reports.split_off(keep_from);
    }
    reports
}

fn write_reports_unlocked(
    app_data_dir: &Path,
    reports: &[SanitizedErrorReport],
) -> Result<(), String> {
    let dir = app_data_dir.join(DIAGNOSTICS_DIR);
    std::fs::create_dir_all(&dir).map_err(|err| format!("创建诊断目录失败: {err}"))?;
    let path = diagnostics_file_path(app_data_dir);
    let bounded = if reports.len() > MAX_LOCAL_REPORTS {
        &reports[reports.len() - MAX_LOCAL_REPORTS..]
    } else {
        reports
    };
    let mut buf = Vec::new();
    for report in bounded {
        let line =
            serde_json::to_string(report).map_err(|err| format!("序列化诊断记录失败: {err}"))?;
        buf.extend_from_slice(line.as_bytes());
        buf.push(b'\n');
    }
    let tmp = dir.join(format!(
        ".error_reports.{}.{}.tmp",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    {
        let mut f =
            std::fs::File::create(&tmp).map_err(|err| format!("写入诊断缓存失败: {err}"))?;
        f.write_all(&buf)
            .map_err(|err| format!("写入诊断数据失败: {err}"))?;
        let _ = f.sync_all();
    }
    if let Err(err) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("保存诊断记录失败: {err}"));
    }
    Ok(())
}

pub fn diagnostics_state(app_data_dir: &Path) -> DiagnosticsStateView {
    let enabled = get_diagnostics_upload_enabled(app_data_dir);
    let reports = {
        let _guard = file_guard();
        read_reports_unlocked(app_data_dir)
    };
    let pending_count = reports.iter().filter(|r| !r.uploaded).count();
    DiagnosticsStateView {
        enabled,
        total_count: reports.len(),
        pending_count,
    }
}

pub fn set_diagnostics_enabled_state(
    app_data_dir: &Path,
    enabled: bool,
) -> Result<DiagnosticsStateView, String> {
    set_diagnostics_upload_enabled(app_data_dir, enabled)?;
    Ok(diagnostics_state(app_data_dir))
}

pub fn list_diagnostics(app_data_dir: &Path) -> Vec<SanitizedErrorReport> {
    let mut reports = {
        let _guard = file_guard();
        read_reports_unlocked(app_data_dir)
    };
    reports.reverse();
    reports
}

pub fn clear_diagnostics(app_data_dir: &Path) -> Result<DiagnosticsStateView, String> {
    {
        let _guard = file_guard();
        let path = diagnostics_file_path(app_data_dir);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|err| format!("清空本地诊断记录失败: {err}"))?;
        }
    }
    Ok(diagnostics_state(app_data_dir))
}

pub fn build_sanitized_report(
    input: DiagnosticsRecordInput,
    app_version: &str,
) -> SanitizedErrorReport {
    let category = normalize_category(&input.category);
    let error_code = normalize_error_code(input.error_code.as_deref());
    let source =
        sanitize_diagnostic_text(input.source.as_deref().unwrap_or("desktop"), MAX_SOURCE_LEN);
    let sanitized_message = sanitize_diagnostic_text(&input.message, MAX_MESSAGE_LEN);
    let sanitized_stack =
        sanitize_diagnostic_text(input.stack.as_deref().unwrap_or(""), MAX_STACK_LEN);
    SanitizedErrorReport {
        report_id: random_report_id(),
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        app_version: truncate_chars(app_version.trim(), 48),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        category,
        error_code,
        http_status: input.http_status,
        source: if source.is_empty() {
            "desktop".to_owned()
        } else {
            source
        },
        sanitized_message,
        sanitized_stack,
        uploaded: false,
    }
}

pub fn append_local_diagnostic_report(
    app_data_dir: &Path,
    report: SanitizedErrorReport,
) -> Result<SanitizedErrorReport, String> {
    let _guard = file_guard();
    let mut reports = read_reports_unlocked(app_data_dir);
    reports.push(report.clone());
    if reports.len() > MAX_LOCAL_REPORTS {
        let keep_from = reports.len() - MAX_LOCAL_REPORTS;
        reports = reports.split_off(keep_from);
    }
    write_reports_unlocked(app_data_dir, &reports)?;
    Ok(report)
}

fn diagnostics_endpoint() -> String {
    std::env::var("EOSTAGE_DIAGNOSTICS_ENDPOINT")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_DIAGNOSTICS_ENDPOINT.to_owned())
}

fn post_reports_batch_to_endpoint(
    application: &AgentApplication,
    endpoint: &str,
    reports: &[SanitizedErrorReport],
) -> Result<(), String> {
    if reports.is_empty() {
        return Ok(());
    }
    let client = build_http_client(application, Duration::from_secs(8))?;
    let resp = client
        .post(endpoint)
        .json(&serde_json::json!({ "reports": reports }))
        .send()
        .map_err(|err| format!("发送异常诊断失败: {err}"))?;
    if !resp.status().is_success() {
        return Err(format!("异常诊断服务返回 HTTP {}", resp.status().as_u16()));
    }
    Ok(())
}

fn post_reports_batch(
    application: &AgentApplication,
    reports: &[SanitizedErrorReport],
) -> Result<(), String> {
    post_reports_batch_to_endpoint(application, &diagnostics_endpoint(), reports)
}

fn mark_reports_uploaded(app_data_dir: &Path, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    let _guard = file_guard();
    let mut reports = read_reports_unlocked(app_data_dir);
    let mut changed = false;
    for r in &mut reports {
        if !r.uploaded && ids.iter().any(|id| id == &r.report_id) {
            r.uploaded = true;
            changed = true;
        }
    }
    if changed {
        let _ = write_reports_unlocked(app_data_dir, &reports);
    }
}

pub fn record_diagnostic_and_maybe_upload(
    app_data_dir: &Path,
    app_version: &str,
    input: DiagnosticsRecordInput,
    application: Option<AgentApplication>,
) -> Result<SanitizedErrorReport, String> {
    let report = build_sanitized_report(input, app_version);
    let saved = append_local_diagnostic_report(app_data_dir, report)?;
    if get_diagnostics_upload_enabled(app_data_dir) {
        if let Some(app) = application {
            let signature = format!("{}|{}|{}", saved.category, saved.error_code, saved.source);
            let allowed = {
                let mut limiter = rate_limiter();
                limiter.allow_and_record(&signature, Instant::now())
            };
            if allowed {
                let data_dir = app_data_dir.to_path_buf();
                let item = saved.clone();
                std::thread::spawn(move || {
                    if post_reports_batch(&app, std::slice::from_ref(&item)).is_ok() {
                        mark_reports_uploaded(&data_dir, &[item.report_id]);
                    }
                });
            }
        }
    }
    Ok(saved)
}

fn serialized_batch_len(reports: &[SanitizedErrorReport]) -> usize {
    serde_json::to_vec(&serde_json::json!({ "reports": reports }))
        .map(|buf| buf.len())
        .unwrap_or(usize::MAX)
}

fn build_upload_batches(pending: &[SanitizedErrorReport]) -> Vec<&[SanitizedErrorReport]> {
    let mut batches = Vec::new();
    let mut start = 0usize;
    while start < pending.len() {
        let mut end = start + 1;
        while end < pending.len() && (end - start) < MAX_UPLOAD_BATCH_COUNT {
            if serialized_batch_len(&pending[start..=end]) > MAX_UPLOAD_BATCH_BYTES {
                break;
            }
            end += 1;
        }
        batches.push(&pending[start..end]);
        start = end;
    }
    batches
}

fn post_reports_batch_with_split(
    application: &AgentApplication,
    endpoint: &str,
    batch: &[SanitizedErrorReport],
    uploaded: &mut usize,
    failed: &mut usize,
    uploaded_ids: &mut Vec<String>,
) {
    match post_reports_batch_to_endpoint(application, endpoint, batch) {
        Ok(()) => {
            *uploaded += batch.len();
            for r in batch {
                uploaded_ids.push(r.report_id.clone());
            }
        }
        Err(err) if batch.len() > 1 && err.contains("HTTP 413") => {
            let (left, right) = batch.split_at(batch.len() / 2);
            post_reports_batch_with_split(
                application,
                endpoint,
                left,
                uploaded,
                failed,
                uploaded_ids,
            );
            post_reports_batch_with_split(
                application,
                endpoint,
                right,
                uploaded,
                failed,
                uploaded_ids,
            );
        }
        Err(_) => {
            *failed += batch.len();
        }
    }
}

pub fn upload_pending_diagnostics(
    app_data_dir: &Path,
    application: &AgentApplication,
) -> Result<DiagnosticsUploadOutcome, String> {
    upload_pending_diagnostics_to_endpoint(app_data_dir, application, &diagnostics_endpoint())
}

fn upload_pending_diagnostics_to_endpoint(
    app_data_dir: &Path,
    application: &AgentApplication,
    endpoint: &str,
) -> Result<DiagnosticsUploadOutcome, String> {
    let pending: Vec<SanitizedErrorReport> = {
        let _guard = file_guard();
        read_reports_unlocked(app_data_dir)
            .into_iter()
            .filter(|r| !r.uploaded)
            .collect()
    };
    if pending.is_empty() {
        return Ok(DiagnosticsUploadOutcome {
            attempted: 0,
            uploaded: 0,
            failed: 0,
            message_zh: "没有待上报的本地诊断记录。".to_owned(),
        });
    }

    let mut uploaded = 0usize;
    let mut failed = 0usize;
    let mut uploaded_ids = Vec::new();
    for chunk in build_upload_batches(&pending) {
        post_reports_batch_with_split(
            application,
            endpoint,
            chunk,
            &mut uploaded,
            &mut failed,
            &mut uploaded_ids,
        );
    }
    mark_reports_uploaded(app_data_dir, &uploaded_ids);
    let message_zh = if failed == 0 {
        format!("已成功上报 {} 条脱敏诊断记录。", uploaded)
    } else if uploaded > 0 {
        format!("已上报 {} 条记录，另有 {} 条发送失败。", uploaded, failed)
    } else {
        "暂时无法连接诊断服务，记录已保留在本机。".to_owned()
    };
    Ok(DiagnosticsUploadOutcome {
        attempted: pending.len(),
        uploaded,
        failed,
        message_zh,
    })
}

/// Install a chained panic hook that records scrubbed crash summaries locally
/// without performing any network I/O inside the panic handler.
pub fn install_panic_hook(app_data_dir: PathBuf, app_version: String) {
    PANIC_HOOK_ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let payload_msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
                (*s).to_owned()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "Rust panic".to_owned()
            };
            let location = info
                .location()
                .map(|loc| format!("{}:{}", loc.file(), loc.line()))
                .unwrap_or_else(|| "unknown".to_owned());
            let backtrace = std::backtrace::Backtrace::capture();
            let report = build_sanitized_report(
                DiagnosticsRecordInput {
                    category: "rust_panic".to_owned(),
                    error_code: Some("RUST_PANIC".to_owned()),
                    http_status: None,
                    source: Some(location),
                    message: payload_msg,
                    stack: Some(backtrace.to_string()),
                },
                &app_version,
            );
            let _ = append_local_diagnostic_report(&app_data_dir, report);
            prev(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    static TEST_SEQ: AtomicUsize = AtomicUsize::new(1);

    fn temp_dir(tag: &str) -> PathBuf {
        let n = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eostage-diag-test-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sanitize_scrubs_home_paths_secrets_emails_and_queries() {
        let raw = "Error at /Users/alice/EOStage/src/app.rs and C:\\Users\\Bob\\work\\file.txt with Bearer secretToken12345 and sk-abc1234567890xyz for alice@example.com on https://api.example.com/v1?api_key=xyz987&code=111";
        let scrubbed = sanitize_diagnostic_text(raw, 1000);
        assert!(!scrubbed.contains("alice"));
        assert!(!scrubbed.contains("Bob"));
        assert!(!scrubbed.contains("secretToken12345"));
        assert!(!scrubbed.contains("sk-abc1234567890xyz"));
        assert!(!scrubbed.contains("alice@example.com"));
        assert!(!scrubbed.contains("xyz987"));
        assert!(scrubbed.contains("<HOME>"));
        assert!(scrubbed.contains("<redacted>"));
        assert!(scrubbed.contains("<redacted_key>"));
        assert!(scrubbed.contains("<redacted_email>"));
    }

    #[test]
    fn rolling_store_caps_at_fifty_and_defaults_upload_disabled() {
        let dir = temp_dir("cap");
        let st0 = diagnostics_state(&dir);
        assert!(!st0.enabled);
        assert_eq!(st0.total_count, 0);

        for i in 0..60 {
            let report = build_sanitized_report(
                DiagnosticsRecordInput {
                    category: "frontend_error".to_owned(),
                    error_code: Some(format!("E_{i}")),
                    http_status: None,
                    source: Some("ui".to_owned()),
                    message: format!("msg {i}"),
                    stack: None,
                },
                "2.2.0-beta.0",
            );
            append_local_diagnostic_report(&dir, report).unwrap();
        }
        let items = list_diagnostics(&dir);
        assert_eq!(items.len(), 50);
        assert_eq!(items[0].error_code, "E_59");
        assert_eq!(items[49].error_code, "E_10");

        let cleared = clear_diagnostics(&dir).unwrap();
        assert_eq!(cleared.total_count, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_mode_never_auto_uploads_while_explicit_upload_marks_uploaded() {
        let dir = temp_dir("upload");
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_srv = Arc::clone(&hits);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                hits_srv.fetch_add(1, Ordering::SeqCst);
                let mut s = stream;
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\n\r\n{\"ok\":true}",
                );
            }
        });
        let endpoint = format!("http://{addr}/eostage/v1/diagnostics/report");

        let svc =
            crate::application::ApplicationServiceState::with_store_path(dir.join("tasks.sqlite3"));
        let app = svc.agent_application();
        record_diagnostic_and_maybe_upload(
            &dir,
            "2.2.0-beta.0",
            DiagnosticsRecordInput {
                category: "backend_error".to_owned(),
                error_code: Some("OFFLINE_CHECK".to_owned()),
                http_status: Some(502),
                source: Some("test".to_owned()),
                message: "gateway error for user@test.com".to_owned(),
                stack: None,
            },
            Some(app.clone()),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(hits.load(Ordering::SeqCst), 0);

        let outcome = upload_pending_diagnostics_to_endpoint(&dir, &app, &endpoint).unwrap();
        assert_eq!(outcome.uploaded, 1);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(diagnostics_state(&dir).pending_count, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upload_pending_diagnostics_splits_batches_under_byte_budget() {
        let dir = temp_dir("byte-budget");
        set_diagnostics_upload_enabled(&dir, false).unwrap();

        let svc =
            crate::application::ApplicationServiceState::with_store_path(dir.join("tasks.sqlite3"));
        let app = svc.agent_application();
        let long_stack = "x".repeat(MAX_STACK_LEN);
        let long_msg = "m".repeat(MAX_MESSAGE_LEN);
        for i in 0..6 {
            record_diagnostic_and_maybe_upload(
                &dir,
                "2.2.0-beta.0",
                DiagnosticsRecordInput {
                    category: "frontend_error".to_owned(),
                    error_code: Some(format!("ERR_{i}")),
                    http_status: Some(500),
                    source: Some("App.tsx".to_owned()),
                    message: long_msg.clone(),
                    stack: Some(long_stack.clone()),
                },
                Some(app.clone()),
            )
            .unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let batch_lengths = Arc::new(Mutex::new(Vec::<usize>::new()));
        let batch_lengths_srv = Arc::clone(&batch_lengths);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let _ = s.set_read_timeout(Some(Duration::from_millis(300)));
                let mut raw = Vec::new();
                let mut tmp = [0u8; 4096];
                while let Ok(n) = s.read(&mut tmp) {
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&raw[..pos]).to_ascii_lowercase();
                        let cl = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if raw.len() >= pos + 4 + cl {
                            batch_lengths_srv.lock().unwrap().push(cl);
                            break;
                        }
                    }
                }
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\n\r\n{\"ok\":true}",
                );
            }
        });
        let endpoint = format!("http://{addr}/eostage/v1/diagnostics/report");

        let outcome = upload_pending_diagnostics_to_endpoint(&dir, &app, &endpoint).unwrap();
        assert_eq!(outcome.attempted, 6);
        assert_eq!(outcome.uploaded, 6);
        assert_eq!(outcome.failed, 0);
        assert_eq!(diagnostics_state(&dir).pending_count, 0);

        let lengths = batch_lengths.lock().unwrap().clone();
        assert!(
            lengths.len() >= 2,
            "6 large stack-bearing reports must split across >= 2 batches, got {lengths:?}"
        );
        for len in &lengths {
            assert!(
                *len <= MAX_UPLOAD_BATCH_BYTES,
                "each batch body ({len} B) must stay <= {MAX_UPLOAD_BATCH_BYTES} B"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
