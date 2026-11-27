use chrono::{Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    LazyLock, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const QUOTA_CACHE_TTL_SECS: u64 = 120;
const UNLIMITED_QUOTA_THRESHOLD: f64 = 1_000_000_000_000_000.0;
const LEDGER_FILE_NAME: &str = "gee_usage_ledger.v1.json";
const MAX_STORED_MONTHS: usize = 24;
const MAX_PENDING_PROFILES: usize = 64;
const MAX_PENDING_PROFILE_AGE_MS: u64 = 60_000;

static TMP_WRITE_SEQ: AtomicU64 = AtomicU64::new(1);
static LEDGER_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Clone, Debug)]
struct CachedRemoteQuotaSnapshot {
    registration_state: Option<String>,
    account_tier: String,
    account_tier_label: String,
    monthly_limit_eecu_seconds: Option<f64>,
    monthly_limit_eecu_hours: Option<f64>,
    daily_limit_eecu_seconds: Option<f64>,
    read_requests_per_minute: Option<u64>,
    cloud_monitoring: GeeCloudMonitoringQuota,
    fetched_at_ms: u64,
    cached_at: Instant,
}

static REMOTE_QUOTA_CACHE: LazyLock<Mutex<HashMap<String, CachedRemoteQuotaSnapshot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GeeCloudMonitoringQuota {
    pub unlocked: bool,
    pub reason: Option<String>,
    pub monthly_used_eecu_seconds: Option<f64>,
    pub monthly_used_eecu_hours: Option<f64>,
    pub monthly_remaining_eecu_hours: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GeeLocalUsageLedger {
    pub month_utc: String,
    pub total_requests: u64,
    pub analysis_runs: u64,
    pub chunk_downloads: u64,
    pub script_runs: u64,
    pub profiled_eecu_seconds: f64,
    pub has_eecu_profile: bool,
    pub total_wall_seconds: f64,
    pub last_call_eecu_seconds: Option<f64>,
    pub last_call_wall_seconds: Option<f64>,
    pub last_updated_at_ms: Option<u64>,
}

impl GeeLocalUsageLedger {
    pub fn zero_for_month(month_utc: &str) -> Self {
        Self {
            month_utc: month_utc.to_owned(),
            total_requests: 0,
            analysis_runs: 0,
            chunk_downloads: 0,
            script_runs: 0,
            profiled_eecu_seconds: 0.0,
            has_eecu_profile: false,
            total_wall_seconds: 0.0,
            last_call_eecu_seconds: None,
            last_call_wall_seconds: None,
            last_updated_at_ms: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GeeQuotaStatusResponse {
    pub ok: bool,
    pub configured: bool,
    pub project_id: Option<String>,
    pub registration_state: Option<String>,
    pub account_tier: String,
    pub account_tier_label: String,
    pub monthly_limit_eecu_seconds: Option<f64>,
    pub monthly_limit_eecu_hours: Option<f64>,
    pub daily_limit_eecu_seconds: Option<f64>,
    pub read_requests_per_minute: Option<u64>,
    pub parallelism_restricted: bool,
    pub last_quota_status_header: Option<String>,
    pub cloud_monitoring: GeeCloudMonitoringQuota,
    pub local_ledger: GeeLocalUsageLedger,
    pub fetched_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingProfileEntry {
    id: String,
    month_utc: String,
    recorded_at_ms: u64,
    from_last_call: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedUsageLedgerFile {
    version: u32,
    #[serde(default)]
    parallelism_restricted: bool,
    #[serde(default)]
    last_quota_status_header: Option<String>,
    #[serde(default)]
    pending_profiles: Vec<PendingProfileEntry>,
    #[serde(default)]
    months: BTreeMap<String, GeeLocalUsageLedger>,
}

impl Default for PersistedUsageLedgerFile {
    fn default() -> Self {
        Self {
            version: 1,
            parallelism_restricted: false,
            last_quota_status_header: None,
            pending_profiles: Vec::new(),
            months: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeeLedgerOpKind {
    GeneralRequest,
    AnalysisRun,
    ChunkDownload,
    ScriptRun,
}

#[derive(Clone, Debug)]
struct ObservedProfileId {
    id: String,
    finished_at: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct GeeCallTracker {
    pub http_requests: u64,
    pub total_wall_seconds: f64,
    pub saw_rate_limit_or_restriction: Option<bool>,
    pub last_quota_header: Option<String>,
    observed_profiles: Vec<ObservedProfileId>,
}

impl GeeCallTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe_http_response(
        &mut self,
        http_status: u16,
        headers: &reqwest::header::HeaderMap,
        wall_seconds: f64,
    ) {
        self.http_requests += 1;
        if wall_seconds.is_finite() && wall_seconds >= 0.0 {
            self.total_wall_seconds += wall_seconds;
        }

        if http_status == 429 {
            self.saw_rate_limit_or_restriction = Some(true);
            if self.last_quota_header.is_none() {
                self.last_quota_header = Some("http_429_rate_limited".to_owned());
            }
        }

        for (name, val) in headers.iter() {
            let key_lower = name.as_str().to_ascii_lowercase();
            if key_lower == "x-earth-engine-quota-status" {
                if let Ok(raw_s) = val.to_str() {
                    if let Some(clean_hdr) = sanitize_quota_status_header(raw_s) {
                        let lower = clean_hdr.to_ascii_lowercase();
                        if lower.contains("parallelism_restricted=true")
                            || (lower.contains("restricted") && !lower.contains("restricted=false"))
                        {
                            self.saw_rate_limit_or_restriction = Some(true);
                        } else if (lower.contains("parallelism_restricted=false")
                            || lower == "ok"
                            || lower == "normal")
                            && self.saw_rate_limit_or_restriction != Some(true)
                        {
                            self.saw_rate_limit_or_restriction = Some(false);
                        }
                        self.last_quota_header = Some(clean_hdr);
                    }
                }
            } else if key_lower == "x-earth-engine-computation-profile" {
                if let Ok(raw_p) = val.to_str() {
                    if let Some(clean_pid) = sanitize_profile_id(raw_p) {
                        if !self.observed_profiles.iter().any(|p| p.id == clean_pid) {
                            self.observed_profiles.push(ObservedProfileId {
                                id: clean_pid,
                                finished_at: Instant::now(),
                            });
                        }
                    }
                }
            }
        }
    }

    pub fn merge_from(&mut self, other: GeeCallTracker) {
        self.http_requests = self.http_requests.saturating_add(other.http_requests);
        if other.total_wall_seconds.is_finite() && other.total_wall_seconds >= 0.0 {
            self.total_wall_seconds += other.total_wall_seconds;
        }
        let other_rate_limited = other.saw_rate_limit_or_restriction == Some(true);
        let self_rate_limited = self.saw_rate_limit_or_restriction == Some(true);
        if other_rate_limited {
            self.saw_rate_limit_or_restriction = Some(true);
        } else if self.saw_rate_limit_or_restriction.is_none() {
            self.saw_rate_limit_or_restriction = other.saw_rate_limit_or_restriction;
        }
        if let Some(hdr) = other.last_quota_header {
            if other_rate_limited || !self_rate_limited || self.last_quota_header.is_none() {
                self.last_quota_header = Some(hdr);
            }
        }
        for profile in other.observed_profiles {
            if !self.observed_profiles.iter().any(|p| p.id == profile.id) {
                self.observed_profiles.push(profile);
            }
        }
    }
}

pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn current_utc_month_key() -> String {
    Utc::now().format("%Y-%m").to_string()
}

fn round_eecu_secs(v: f64) -> f64 {
    if !v.is_finite() || v <= 0.0 {
        0.0
    } else {
        (v * 1_000_000.0).round() / 1_000_000.0
    }
}

fn round_wall_secs(v: f64) -> f64 {
    if !v.is_finite() || v <= 0.0 {
        0.0
    } else {
        (v * 1_000.0).round() / 1_000.0
    }
}

fn round_hours(v: f64) -> f64 {
    if !v.is_finite() || v <= 0.0 {
        0.0
    } else {
        (v * 10_000.0).round() / 10_000.0
    }
}

pub fn sanitize_quota_status_header(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if [
        "authorization:",
        "bearer ",
        "password=",
        "token=",
        "cookie=",
        "signature=",
        "x-amz-",
        "x-goog-",
        ".netrc",
        "file://",
    ]
    .iter()
    .any(|bad| lower.contains(bad))
    {
        return None;
    }
    let cleaned: String = trimmed
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '/' | '\\' => '·',
            other => other,
        })
        .take(160)
        .collect();
    let final_str = cleaned.trim().to_owned();
    if final_str.is_empty() {
        None
    } else {
        Some(final_str)
    }
}

fn sanitize_profile_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 96 {
        return None;
    }
    if trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

pub fn invalidate_quota_cache() {
    if let Ok(mut guard) = REMOTE_QUOTA_CACHE.lock() {
        guard.clear();
    }
}

// ============================================================================
// Pure Parsing & Classification Helpers (100% Real Google API Payloads)
// ============================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedConsumerQuotas {
    pub monthly_limit_eecu_seconds: Option<f64>,
    pub monthly_limit_eecu_hours: Option<f64>,
    pub daily_limit_eecu_seconds: Option<f64>,
    pub read_requests_per_minute: Option<u64>,
}

fn parse_quota_limit_number(val: &Value) -> Option<f64> {
    let n = val
        .as_f64()
        .or_else(|| val.as_i64().map(|i| i as f64))
        .or_else(|| val.as_str().and_then(|s| s.trim().parse::<f64>().ok()))?;
    if !n.is_finite() || !(0.0..UNLIMITED_QUOTA_THRESHOLD).contains(&n) {
        None
    } else {
        Some(n)
    }
}

fn extract_metric_effective_limit(metrics: &[Value], target_metric: &str) -> Option<f64> {
    let mut best_limit: Option<f64> = None;
    for m in metrics {
        if m.get("metric").and_then(Value::as_str) != Some(target_metric) {
            continue;
        }
        let Some(limits) = m.get("consumerQuotaLimits").and_then(Value::as_array) else {
            continue;
        };
        for lim in limits {
            let Some(buckets) = lim.get("quotaBuckets").and_then(Value::as_array) else {
                continue;
            };
            for bucket in buckets {
                if let Some(eff) = bucket
                    .get("effectiveLimit")
                    .and_then(parse_quota_limit_number)
                {
                    best_limit = Some(match best_limit {
                        Some(cur) => cur.min(eff),
                        None => eff,
                    });
                }
            }
        }
    }
    best_limit
}

pub fn parse_consumer_quota_metrics(doc: &Value) -> ParsedConsumerQuotas {
    let metrics = doc
        .get("metrics")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let monthly_sec = extract_metric_effective_limit(
        metrics,
        "earthengine.googleapis.com/monthly_eecu_usage_time",
    );
    let monthly_hours = monthly_sec.map(|sec| ((sec / 3600.0) * 100.0).round() / 100.0);
    let daily_sec =
        extract_metric_effective_limit(metrics, "earthengine.googleapis.com/daily_eecu_usage_time");
    let read_rpm =
        extract_metric_effective_limit(metrics, "earthengine.googleapis.com/read_requests")
            .map(|v| v.round() as u64);

    ParsedConsumerQuotas {
        monthly_limit_eecu_seconds: monthly_sec,
        monthly_limit_eecu_hours: monthly_hours,
        daily_limit_eecu_seconds: daily_sec,
        read_requests_per_minute: read_rpm,
    }
}

pub fn classify_gee_account_tier(
    registration_state: Option<&str>,
    monthly_limit_eecu_hours: Option<f64>,
) -> (String, String) {
    let Some(raw_state) = registration_state.map(str::trim).filter(|s| !s.is_empty()) else {
        return ("unknown".to_owned(), "已配置 GEE 项目".to_owned());
    };
    let upper = raw_state.to_ascii_uppercase();
    if upper == "REGISTERED_COMMERCIALLY" {
        return ("commercial".to_owned(), "商业计费账号".to_owned());
    }
    if upper == "REGISTERED_NOT_COMMERCIALLY"
        || upper.contains("NOT_COMMERCIAL")
        || upper.contains("NONCOMMERCIAL")
    {
        return match monthly_limit_eecu_hours {
            Some(hours) if hours <= 200.0 => (
                "noncommercial_community".to_owned(),
                "非商业科研档 (Community)".to_owned(),
            ),
            Some(hours) if hours <= 5000.0 => (
                "noncommercial_contributor".to_owned(),
                "进阶科研档 (Contributor)".to_owned(),
            ),
            Some(_) => (
                "noncommercial_partner".to_owned(),
                "合作科研档 (Partner)".to_owned(),
            ),
            None => (
                "noncommercial_unlimited".to_owned(),
                "非商业科研账号".to_owned(),
            ),
        };
    }
    ("unknown".to_owned(), "已配置 GEE 项目".to_owned())
}

/// Honest tier classification that distinguishes `serviceusage` fetch failure (`quotas_opt == None`)
/// from a verified HTTP 200 unlimited response (`quotas_opt == Some(q)` with `q.monthly_limit_eecu_hours == None`).
pub fn classify_gee_account_tier_checked(
    registration_state: Option<&str>,
    quotas_opt: Option<&ParsedConsumerQuotas>,
) -> (String, String) {
    let Some(raw_state) = registration_state.map(str::trim).filter(|s| !s.is_empty()) else {
        return ("unknown".to_owned(), "已配置 GEE 项目".to_owned());
    };
    let upper = raw_state.to_ascii_uppercase();
    if upper == "REGISTERED_COMMERCIALLY" {
        return ("commercial".to_owned(), "商业计费账号".to_owned());
    }
    if upper == "REGISTERED_NOT_COMMERCIALLY"
        || upper.contains("NOT_COMMERCIAL")
        || upper.contains("NONCOMMERCIAL")
    {
        let Some(quotas) = quotas_opt else {
            return (
                "noncommercial_unknown".to_owned(),
                "非商业科研账号".to_owned(),
            );
        };
        return classify_gee_account_tier(Some(raw_state), quotas.monthly_limit_eecu_hours);
    }
    ("unknown".to_owned(), "已配置 GEE 项目".to_owned())
}

pub fn sum_monitoring_timeseries_seconds(doc: &Value) -> f64 {
    let Some(series_arr) = doc.get("timeSeries").and_then(Value::as_array) else {
        return 0.0;
    };
    let mut total_secs = 0.0_f64;
    for ts in series_arr {
        let Some(points) = ts.get("points").and_then(Value::as_array) else {
            continue;
        };
        for pt in points {
            let Some(val_obj) = pt.get("value") else {
                continue;
            };
            let pt_val = val_obj
                .get("doubleValue")
                .and_then(Value::as_f64)
                .or_else(|| val_obj.get("int64Value").and_then(Value::as_f64))
                .or_else(|| {
                    val_obj
                        .get("int64Value")
                        .and_then(Value::as_i64)
                        .map(|i| i as f64)
                })
                .or_else(|| {
                    val_obj
                        .get("int64Value")
                        .and_then(Value::as_str)
                        .and_then(|s| s.trim().parse::<f64>().ok())
                });
            if let Some(v) = pt_val {
                if v.is_finite() && v > 0.0 {
                    total_secs += v;
                }
            }
        }
    }
    total_secs
}

pub fn parse_cloud_monitoring_response(
    http_status: u16,
    body_text: &str,
    monthly_limit_eecu_hours: Option<f64>,
) -> GeeCloudMonitoringQuota {
    if http_status == 200 {
        let Ok(doc) = serde_json::from_str::<Value>(body_text) else {
            return GeeCloudMonitoringQuota {
                unlocked: false,
                reason: Some("monitoring_unavailable".to_owned()),
                monthly_used_eecu_seconds: None,
                monthly_used_eecu_hours: None,
                monthly_remaining_eecu_hours: None,
            };
        };
        let used_secs = round_eecu_secs(sum_monitoring_timeseries_seconds(&doc));
        let used_hours = round_hours(used_secs / 3600.0);
        let remaining_hours =
            monthly_limit_eecu_hours.map(|lim| round_hours((lim - used_hours).max(0.0)));
        return GeeCloudMonitoringQuota {
            unlocked: true,
            reason: None,
            monthly_used_eecu_seconds: Some(used_secs),
            monthly_used_eecu_hours: Some(used_hours),
            monthly_remaining_eecu_hours: remaining_hours,
        };
    }

    if http_status == 403 {
        let lower = body_text.to_ascii_lowercase();
        if lower.contains("billing") {
            return GeeCloudMonitoringQuota {
                unlocked: false,
                reason: Some("billing_not_enabled".to_owned()),
                monthly_used_eecu_seconds: None,
                monthly_used_eecu_hours: None,
                monthly_remaining_eecu_hours: None,
            };
        }
    }

    GeeCloudMonitoringQuota {
        unlocked: false,
        reason: Some("monitoring_unavailable".to_owned()),
        monthly_used_eecu_seconds: None,
        monthly_used_eecu_hours: None,
        monthly_remaining_eecu_hours: None,
    }
}

pub fn parse_profile_eecu_seconds(profile_doc: &Value) -> Option<f64> {
    let table = profile_doc.get("result").unwrap_or(profile_doc);
    let cols = table.get("cols")?.as_array()?;
    let rows = table.get("rows")?.as_array()?;

    let eecu_col_idx = cols.iter().position(|c| {
        c.get("label")
            .and_then(Value::as_str)
            .is_some_and(|lbl| lbl.to_ascii_uppercase().contains("EECU"))
    })?;
    let desc_col_idx = cols.iter().position(|c| {
        c.get("label")
            .and_then(Value::as_str)
            .is_some_and(|lbl| lbl.eq_ignore_ascii_case("Description"))
    });

    let mut sum = 0.0_f64;
    for row in rows {
        let Some(cells) = row.as_array() else {
            continue;
        };
        if let Some(di) = desc_col_idx {
            if cells
                .get(di)
                .and_then(|c| c.get("v"))
                .and_then(Value::as_str)
                .is_some_and(|d| d.trim().eq_ignore_ascii_case("total"))
            {
                continue;
            }
        }
        if let Some(v) = cells
            .get(eecu_col_idx)
            .and_then(|c| c.get("v"))
            .and_then(Value::as_f64)
        {
            if v.is_finite() && v >= 0.0 {
                sum += v;
            }
        }
    }
    Some(round_eecu_secs(sum))
}

// ============================================================================
// Persistent Secret-Free Local Monthly Compute Ledger (`gee_usage_ledger.v1.json`)
// ============================================================================

fn ledger_file_path(app_data_dir: &Path) -> std::path::PathBuf {
    app_data_dir.join(LEDGER_FILE_NAME)
}

fn read_ledger_file_unlocked(app_data_dir: &Path) -> PersistedUsageLedgerFile {
    let path = ledger_file_path(app_data_dir);
    let Ok(raw) = fs::read_to_string(&path) else {
        return PersistedUsageLedgerFile::default();
    };
    serde_json::from_str::<PersistedUsageLedgerFile>(&raw).unwrap_or_default()
}

fn write_ledger_file_atomic_unlocked(
    app_data_dir: &Path,
    doc: &PersistedUsageLedgerFile,
) -> Result<(), String> {
    fs::create_dir_all(app_data_dir).map_err(|e| format!("GEE_LEDGER_DIR_ERR: {e}"))?;
    let target = ledger_file_path(app_data_dir);
    let seq = TMP_WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = app_data_dir.join(format!(
        "{LEDGER_FILE_NAME}.tmp-{}-{seq}",
        std::process::id()
    ));
    let bytes =
        serde_json::to_vec_pretty(doc).map_err(|e| format!("GEE_LEDGER_SERIALIZE_ERR: {e}"))?;
    fs::write(&tmp_path, &bytes).map_err(|e| format!("GEE_LEDGER_WRITE_TMP_ERR: {e}"))?;
    if let Err(first_err) = fs::rename(&tmp_path, &target) {
        #[cfg(target_os = "windows")]
        {
            let _ = fs::remove_file(&target);
            if fs::rename(&tmp_path, &target).is_ok() {
                return Ok(());
            }
        }
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("GEE_LEDGER_RENAME_ERR: {first_err}"));
    }
    Ok(())
}

fn trim_old_months(months: &mut BTreeMap<String, GeeLocalUsageLedger>) {
    while months.len() > MAX_STORED_MONTHS {
        let Some(oldest_key) = months.keys().next().cloned() else {
            break;
        };
        months.remove(&oldest_key);
    }
}

pub fn load_local_ledger_snapshot_for_month(
    app_data_dir: &Path,
    month_utc: &str,
) -> (GeeLocalUsageLedger, bool, Option<String>) {
    let _guard = LEDGER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let doc = read_ledger_file_unlocked(app_data_dir);
    let ledger = doc
        .months
        .get(month_utc)
        .cloned()
        .unwrap_or_else(|| GeeLocalUsageLedger::zero_for_month(month_utc));
    (
        ledger,
        doc.parallelism_restricted,
        doc.last_quota_status_header,
    )
}

#[derive(Clone, Debug)]
pub struct LedgerMutationInput {
    pub op_kind: GeeLedgerOpKind,
    pub op_succeeded: bool,
    pub http_requests: u64,
    pub wall_seconds: f64,
    pub profiled_eecu_seconds: Option<f64>,
    pub saw_rate_limit_or_restriction: Option<bool>,
    pub last_quota_header: Option<String>,
    pub pending_profile_ids: Vec<String>,
    pub updated_at_ms: u64,
}

pub fn apply_ledger_mutation_for_month(
    app_data_dir: &Path,
    month_utc: &str,
    input: LedgerMutationInput,
) -> Result<GeeLocalUsageLedger, String> {
    let _guard = LEDGER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut doc = read_ledger_file_unlocked(app_data_dir);
    doc.version = 1;

    if let Some(restricted) = input.saw_rate_limit_or_restriction {
        doc.parallelism_restricted = restricted;
    }
    if let Some(hdr) = input
        .last_quota_header
        .as_deref()
        .and_then(sanitize_quota_status_header)
    {
        doc.last_quota_status_header = Some(hdr);
    }

    let entry = doc
        .months
        .entry(month_utc.to_owned())
        .or_insert_with(|| GeeLocalUsageLedger::zero_for_month(month_utc));

    if input.http_requests > 0 || input.op_succeeded {
        entry.total_requests = entry.total_requests.saturating_add(input.http_requests);
        if input.op_succeeded {
            match input.op_kind {
                GeeLedgerOpKind::GeneralRequest => {}
                GeeLedgerOpKind::AnalysisRun => {
                    entry.analysis_runs = entry.analysis_runs.saturating_add(1);
                }
                GeeLedgerOpKind::ChunkDownload => {
                    entry.chunk_downloads = entry.chunk_downloads.saturating_add(1);
                }
                GeeLedgerOpKind::ScriptRun => {
                    entry.script_runs = entry.script_runs.saturating_add(1);
                }
            }
        }
        let wall_clean = round_wall_secs(input.wall_seconds);
        entry.total_wall_seconds = round_wall_secs(entry.total_wall_seconds + wall_clean);
        entry.last_call_wall_seconds = Some(wall_clean);
        entry.last_updated_at_ms = Some(input.updated_at_ms);

        // Clear `from_last_call` on older pending profiles before recording this call's profiles
        for p in &mut doc.pending_profiles {
            p.from_last_call = false;
        }

        if let Some(eecu_s) = input.profiled_eecu_seconds {
            let clean_eecu = round_eecu_secs(eecu_s);
            entry.profiled_eecu_seconds = round_eecu_secs(entry.profiled_eecu_seconds + clean_eecu);
            entry.has_eecu_profile = true;
            entry.last_call_eecu_seconds = Some(clean_eecu);
        } else {
            entry.last_call_eecu_seconds = None;
        }

        for raw_pid in input.pending_profile_ids {
            if let Some(pid) = sanitize_profile_id(&raw_pid) {
                if !doc.pending_profiles.iter().any(|p| p.id == pid) {
                    doc.pending_profiles.push(PendingProfileEntry {
                        id: pid,
                        month_utc: month_utc.to_owned(),
                        recorded_at_ms: input.updated_at_ms,
                        from_last_call: true,
                    });
                }
            }
        }
        while doc.pending_profiles.len() > MAX_PENDING_PROFILES {
            doc.pending_profiles.remove(0);
        }
    }

    let snapshot = entry.clone();
    trim_old_months(&mut doc.months);
    write_ledger_file_atomic_unlocked(app_data_dir, &doc)?;
    Ok(snapshot)
}

// ============================================================================
// Live GEE Profile Flushing (`Profile.getProfiles`) & Tracker Finalization
// ============================================================================

fn request_profile_eecu_seconds(
    client: &reqwest::blocking::Client,
    base_gee_v1_url: &str,
    project_id: &str,
    access_token: &str,
    profile_ids: &[String],
) -> Result<f64, String> {
    if profile_ids.is_empty() {
        return Ok(0.0);
    }
    let url = format!(
        "{}/projects/{project_id}/value:compute",
        base_gee_v1_url.trim_end_matches('/')
    );
    let expr = json!({
        "expression": {
            "result": "2",
            "values": {
                "0": { "constantValue": profile_ids },
                "1": { "constantValue": "json" },
                "2": {
                    "functionInvocationValue": {
                        "functionName": "Profile.getProfiles",
                        "arguments": {
                            "ids": { "valueReference": "0" },
                            "format": { "valueReference": "1" }
                        }
                    }
                }
            }
        }
    });

    let retry_delays_ms: [Option<u64>; 3] = [Some(250), Some(600), None];
    let mut last_err = String::new();
    for retry_delay in retry_delays_ms {
        let resp = match client
            .post(&url)
            .bearer_auth(access_token)
            .header("X-Goog-User-Project", project_id)
            .json(&expr)
            .send()
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("PROFILE_NET_ERR: {e}");
                if let Some(delay_ms) = retry_delay {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    continue;
                }
                return Err(last_err);
            }
        };
        let status = resp.status();
        let status_u16 = status.as_u16();
        let bytes = match resp.bytes() {
            Ok(b) => b,
            Err(e) => {
                last_err = format!("PROFILE_READ_ERR ({status_u16}): {e}");
                if let Some(delay_ms) = retry_delay {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    continue;
                }
                return Err(last_err);
            }
        };
        if let (502..=504, Some(delay_ms)) = (status_u16, retry_delay) {
            std::thread::sleep(Duration::from_millis(delay_ms));
            continue;
        }
        if !status.is_success() {
            let msg = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|d| d["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "profile error".to_owned());
            return Err(format!("HTTP {status_u16}: {msg}"));
        }
        let doc: Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("PROFILE_PARSE_ERR ({status_u16}): {e}"))?;
        return parse_profile_eecu_seconds(&doc)
            .ok_or_else(|| "PROFILE_SCHEMA_MISSING_EECU_COL".to_owned());
    }
    Err(last_err)
}

pub fn finalize_gee_tracker(
    app_data_dir: &Path,
    gee_ctx: Option<(&reqwest::blocking::Client, &str, &str)>,
    op_kind: GeeLedgerOpKind,
    op_succeeded: bool,
    tracker: GeeCallTracker,
) {
    if tracker.http_requests == 0
        && !op_succeeded
        && tracker.saw_rate_limit_or_restriction.is_none()
        && tracker.last_quota_header.is_none()
    {
        return;
    }

    let mut resolved_eecu: Option<f64> = None;
    let mut deferred_ids: Vec<String> = Vec::new();

    if let Some((client, project_id, access_token)) = gee_ctx {
        let mut mature_ids = Vec::new();
        for item in &tracker.observed_profiles {
            // Profiles older than 850ms are typically already flushed on Google's backend;
            // younger profiles are deferred to `pending_profiles` so user calls never block!
            if item.finished_at.elapsed() >= Duration::from_millis(850) {
                mature_ids.push(item.id.clone());
            } else {
                deferred_ids.push(item.id.clone());
            }
        }
        if !mature_ids.is_empty() {
            match request_profile_eecu_seconds(
                client,
                "https://earthengine.googleapis.com/v1",
                project_id,
                access_token,
                &mature_ids,
            ) {
                Ok(secs) => {
                    resolved_eecu = Some(secs);
                }
                Err(_) => {
                    // Defer to next flush so no profile is ever lost
                    let mut combined = mature_ids;
                    combined.extend(deferred_ids);
                    deferred_ids = combined;
                }
            }
        }
    } else {
        deferred_ids = tracker
            .observed_profiles
            .into_iter()
            .map(|p| p.id)
            .collect();
    }

    let month_utc = current_utc_month_key();
    let _ = apply_ledger_mutation_for_month(
        app_data_dir,
        &month_utc,
        LedgerMutationInput {
            op_kind,
            op_succeeded,
            http_requests: tracker.http_requests,
            wall_seconds: tracker.total_wall_seconds,
            profiled_eecu_seconds: resolved_eecu,
            saw_rate_limit_or_restriction: tracker.saw_rate_limit_or_restriction,
            last_quota_header: tracker.last_quota_header,
            pending_profile_ids: deferred_ids,
            updated_at_ms: now_epoch_ms(),
        },
    );
}

pub fn record_script_execution(app_data_dir: &Path, wall_seconds: f64, profile_ids: &[String]) {
    let month_utc = current_utc_month_key();
    let req_count = (profile_ids.len() as u64).max(1);
    let _ = apply_ledger_mutation_for_month(
        app_data_dir,
        &month_utc,
        LedgerMutationInput {
            op_kind: GeeLedgerOpKind::ScriptRun,
            op_succeeded: true,
            http_requests: req_count,
            wall_seconds,
            profiled_eecu_seconds: None,
            saw_rate_limit_or_restriction: None,
            last_quota_header: None,
            pending_profile_ids: profile_ids.to_vec(),
            updated_at_ms: now_epoch_ms(),
        },
    );
}

pub fn flush_pending_profiles(
    app_data_dir: &Path,
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
) {
    flush_pending_profiles_with_base(
        app_data_dir,
        client,
        "https://earthengine.googleapis.com/v1",
        project_id,
        access_token,
    );
}

fn flush_pending_profiles_with_base(
    app_data_dir: &Path,
    client: &reqwest::blocking::Client,
    base_gee_v1_url: &str,
    project_id: &str,
    access_token: &str,
) {
    let pending_snapshot: Vec<PendingProfileEntry> = {
        let _guard = LEDGER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let doc = read_ledger_file_unlocked(app_data_dir);
        doc.pending_profiles
    };
    if pending_snapshot.is_empty() {
        return;
    }

    let now_ms = now_epoch_ms();
    let active: Vec<PendingProfileEntry> = pending_snapshot
        .into_iter()
        .filter(|p| now_ms.saturating_sub(p.recorded_at_ms) <= MAX_PENDING_PROFILE_AGE_MS)
        .collect();

    if active.is_empty() {
        let _guard = LEDGER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = read_ledger_file_unlocked(app_data_dir);
        doc.pending_profiles.clear();
        let _ = write_ledger_file_atomic_unlocked(app_data_dir, &doc);
        return;
    }

    // Fast path: if all pending profiles belong to the same month & same `from_last_call` flag, try single batch first
    let newest_age_ms = active
        .iter()
        .map(|p| now_ms.saturating_sub(p.recorded_at_ms))
        .min()
        .unwrap_or(999_999);

    let all_ids: Vec<String> = active.iter().map(|p| p.id.clone()).collect();
    let mut resolved_per_id: HashMap<String, f64> = HashMap::new();
    let mut still_pending_ids: Vec<String> = Vec::new();

    let single_group = active
        .windows(2)
        .all(|w| w[0].month_utc == w[1].month_utc && w[0].from_last_call == w[1].from_last_call);

    let mut batch_ok = false;
    if single_group {
        if let Ok(total_secs) = request_profile_eecu_seconds(
            client,
            base_gee_v1_url,
            project_id,
            access_token,
            &all_ids,
        ) {
            if let Some(first) = active.first() {
                resolved_per_id.insert(first.id.clone(), total_secs);
                for rest in active.iter().skip(1) {
                    resolved_per_id.insert(rest.id.clone(), 0.0);
                }
            }
            batch_ok = true;
        } else if newest_age_ms < 2_200 {
            let wait_ms = (1_400_u64.saturating_sub(newest_age_ms)).clamp(350, 1_100);
            std::thread::sleep(Duration::from_millis(wait_ms));
            if let Ok(total_secs) = request_profile_eecu_seconds(
                client,
                base_gee_v1_url,
                project_id,
                access_token,
                &all_ids,
            ) {
                if let Some(first) = active.first() {
                    resolved_per_id.insert(first.id.clone(), total_secs);
                    for rest in active.iter().skip(1) {
                        resolved_per_id.insert(rest.id.clone(), 0.0);
                    }
                }
                batch_ok = true;
            }
        }
    }

    if !batch_ok {
        for item in &active {
            let single_id = std::slice::from_ref(&item.id);
            match request_profile_eecu_seconds(
                client,
                base_gee_v1_url,
                project_id,
                access_token,
                single_id,
            ) {
                Ok(secs) => {
                    resolved_per_id.insert(item.id.clone(), secs);
                }
                Err(_) => {
                    if now_epoch_ms().saturating_sub(item.recorded_at_ms) < 15_000 {
                        still_pending_ids.push(item.id.clone());
                    }
                }
            }
        }
    }

    let _guard = LEDGER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut doc = read_ledger_file_unlocked(app_data_dir);
    for item in &active {
        if let Some(&secs) = resolved_per_id.get(&item.id) {
            let entry = doc
                .months
                .entry(item.month_utc.clone())
                .or_insert_with(|| GeeLocalUsageLedger::zero_for_month(&item.month_utc));
            entry.profiled_eecu_seconds = round_eecu_secs(entry.profiled_eecu_seconds + secs);
            entry.has_eecu_profile = true;
            if item.from_last_call {
                let prev = entry.last_call_eecu_seconds.unwrap_or(0.0);
                entry.last_call_eecu_seconds = Some(round_eecu_secs(prev + secs));
            }
            entry.last_updated_at_ms = Some(now_epoch_ms());
        }
    }
    doc.pending_profiles
        .retain(|p| still_pending_ids.iter().any(|sid| sid == &p.id));
    let _ = write_ledger_file_atomic_unlocked(app_data_dir, &doc);
}

// ============================================================================
// Live Remote Quota & Cloud Monitoring Inspection (`gee_quota_status`)
// ============================================================================

#[derive(Clone, Debug)]
pub struct QuotaApiEndpoints {
    pub earthengine_v1_base: String,
    pub serviceusage_v1beta1_base: String,
    pub monitoring_v3_base: String,
}

impl Default for QuotaApiEndpoints {
    fn default() -> Self {
        Self {
            earthengine_v1_base: "https://earthengine.googleapis.com/v1".to_owned(),
            serviceusage_v1beta1_base: "https://serviceusage.googleapis.com/v1beta1".to_owned(),
            monitoring_v3_base: "https://monitoring.googleapis.com/v3".to_owned(),
        }
    }
}

fn send_get_bytes_with_retry(
    build_req: impl Fn() -> reqwest::blocking::RequestBuilder,
) -> Option<(u16, Vec<u8>)> {
    let retry_delays_ms: [Option<u64>; 3] = [Some(250), Some(600), None];
    for retry_delay in retry_delays_ms {
        let Ok(resp) = build_req().send() else {
            if let Some(delay_ms) = retry_delay {
                std::thread::sleep(Duration::from_millis(delay_ms));
                continue;
            }
            return None;
        };
        let status = resp.status().as_u16();
        let Ok(bytes) = resp.bytes() else {
            if let Some(delay_ms) = retry_delay {
                std::thread::sleep(Duration::from_millis(delay_ms));
                continue;
            }
            return None;
        };
        if let (502..=504, Some(delay_ms)) = (status, retry_delay) {
            std::thread::sleep(Duration::from_millis(delay_ms));
            continue;
        }
        return Some((status, bytes.to_vec()));
    }
    None
}

fn fetch_project_registration_state(
    client: &reqwest::blocking::Client,
    ee_base: &str,
    project_id: &str,
    access_token: &str,
) -> Option<String> {
    let url = format!(
        "{}/projects/{project_id}/config",
        ee_base.trim_end_matches('/')
    );
    let (status, bytes) = send_get_bytes_with_retry(|| {
        client
            .get(&url)
            .bearer_auth(access_token)
            .header("X-Goog-User-Project", project_id)
    })?;
    if !(200..300).contains(&status) {
        return None;
    }
    let doc: Value = serde_json::from_slice(&bytes).ok()?;
    doc.get("registrationState")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn fetch_consumer_quota_metrics(
    client: &reqwest::blocking::Client,
    su_base: &str,
    project_id: &str,
    access_token: &str,
) -> Option<ParsedConsumerQuotas> {
    let url = format!(
        "{}/projects/{project_id}/services/earthengine.googleapis.com/consumerQuotaMetrics",
        su_base.trim_end_matches('/')
    );
    let mut all_metrics: Vec<Value> = Vec::new();
    let mut page_token: Option<String> = None;

    for _ in 0..5 {
        let pt_clone = page_token.clone();
        let (status, bytes) = send_get_bytes_with_retry(|| {
            let mut req = client
                .get(&url)
                .bearer_auth(access_token)
                .header("X-Goog-User-Project", project_id)
                .query(&[("view", "FULL")]);
            if let Some(ref pt) = pt_clone {
                req = req.query(&[("pageToken", pt.as_str())]);
            }
            req
        })?;
        if !(200..300).contains(&status) {
            return None;
        }
        let doc: Value = serde_json::from_slice(&bytes).ok()?;
        if let Some(arr) = doc.get("metrics").and_then(Value::as_array) {
            all_metrics.extend(arr.iter().cloned());
        }
        match doc
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(nxt) => page_token = Some(nxt.to_owned()),
            None => break,
        }
    }

    Some(parse_consumer_quota_metrics(&json!({
        "metrics": all_metrics
    })))
}

fn fetch_cloud_monitoring_monthly_usage(
    client: &reqwest::blocking::Client,
    mon_base: &str,
    project_id: &str,
    access_token: &str,
    monthly_limit_eecu_hours: Option<f64>,
) -> GeeCloudMonitoringQuota {
    let now_utc = Utc::now();
    let start_iso = format!("{:04}-{:02}-01T00:00:00Z", now_utc.year(), now_utc.month());
    let end_utc = if now_utc.day() == 1
        && now_utc.hour() == 0
        && now_utc.minute() == 0
        && now_utc.second() == 0
    {
        now_utc + chrono::Duration::seconds(1)
    } else {
        now_utc
    };
    let end_iso = end_utc.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let url = format!(
        "{}/projects/{project_id}/timeSeries",
        mon_base.trim_end_matches('/')
    );
    let filter_expr = r#"metric.type = "earthengine.googleapis.com/project/cpu/usage_time""#;

    let mut total_used_secs = 0.0_f64;
    let mut page_token: Option<String> = None;

    for page_idx in 0..10 {
        let pt_clone = page_token.clone();
        let Some((status, bytes)) = send_get_bytes_with_retry(|| {
            let mut req = client
                .get(&url)
                .bearer_auth(access_token)
                .header("X-Goog-User-Project", project_id)
                .query(&[
                    ("filter", filter_expr),
                    ("interval.startTime", start_iso.as_str()),
                    ("interval.endTime", end_iso.as_str()),
                ]);
            if let Some(ref pt) = pt_clone {
                req = req.query(&[("pageToken", pt.as_str())]);
            }
            req
        }) else {
            return GeeCloudMonitoringQuota {
                unlocked: false,
                reason: Some("monitoring_unavailable".to_owned()),
                monthly_used_eecu_seconds: None,
                monthly_used_eecu_hours: None,
                monthly_remaining_eecu_hours: None,
            };
        };
        let body_text = String::from_utf8_lossy(&bytes).into_owned();

        if status != 200 {
            if page_idx == 0 {
                return parse_cloud_monitoring_response(
                    status,
                    &body_text,
                    monthly_limit_eecu_hours,
                );
            }
            break;
        }

        let Ok(doc) = serde_json::from_str::<Value>(&body_text) else {
            return GeeCloudMonitoringQuota {
                unlocked: false,
                reason: Some("monitoring_unavailable".to_owned()),
                monthly_used_eecu_seconds: None,
                monthly_used_eecu_hours: None,
                monthly_remaining_eecu_hours: None,
            };
        };
        total_used_secs += sum_monitoring_timeseries_seconds(&doc);
        match doc
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(nxt) => page_token = Some(nxt.to_owned()),
            None => break,
        }
    }

    let used_secs = round_eecu_secs(total_used_secs);
    let used_hours = round_hours(used_secs / 3600.0);
    let remaining_hours =
        monthly_limit_eecu_hours.map(|lim| round_hours((lim - used_hours).max(0.0)));
    GeeCloudMonitoringQuota {
        unlocked: true,
        reason: None,
        monthly_used_eecu_seconds: Some(used_secs),
        monthly_used_eecu_hours: Some(used_hours),
        monthly_remaining_eecu_hours: remaining_hours,
    }
}

pub fn build_unconfigured_quota_status(app_data_dir: &Path) -> GeeQuotaStatusResponse {
    let month_utc = current_utc_month_key();
    let (local_ledger, parallelism_restricted, last_quota_status_header) =
        load_local_ledger_snapshot_for_month(app_data_dir, &month_utc);
    GeeQuotaStatusResponse {
        ok: true,
        configured: false,
        project_id: None,
        registration_state: None,
        account_tier: "unconfigured".to_owned(),
        account_tier_label: "未配置".to_owned(),
        monthly_limit_eecu_seconds: None,
        monthly_limit_eecu_hours: None,
        daily_limit_eecu_seconds: None,
        read_requests_per_minute: None,
        parallelism_restricted,
        last_quota_status_header,
        cloud_monitoring: GeeCloudMonitoringQuota {
            unlocked: false,
            reason: Some("unconfigured".to_owned()),
            monthly_used_eecu_seconds: None,
            monthly_used_eecu_hours: None,
            monthly_remaining_eecu_hours: None,
        },
        local_ledger,
        fetched_at_ms: now_epoch_ms(),
    }
}

pub fn fetch_configured_quota_status_with_endpoints(
    app_data_dir: &Path,
    client: &reqwest::blocking::Client,
    project_id: &str,
    access_token: &str,
    refresh: bool,
    endpoints: &QuotaApiEndpoints,
) -> GeeQuotaStatusResponse {
    // 1. Always flush any deferred computation profiles into the persistent local ledger first
    flush_pending_profiles_with_base(
        app_data_dir,
        client,
        &endpoints.earthengine_v1_base,
        project_id,
        access_token,
    );

    let month_utc = current_utc_month_key();
    let (local_ledger, parallelism_restricted, last_quota_status_header) =
        load_local_ledger_snapshot_for_month(app_data_dir, &month_utc);

    let cache_key = format!(
        "{}|{}|{}",
        endpoints.earthengine_v1_base, endpoints.serviceusage_v1beta1_base, project_id
    );

    // 2. Check 120-second remote snapshot cache unless `refresh == true`
    if !refresh {
        if let Ok(guard) = REMOTE_QUOTA_CACHE.lock() {
            if let Some(cached) = guard.get(&cache_key) {
                if cached.cached_at.elapsed() < Duration::from_secs(QUOTA_CACHE_TTL_SECS) {
                    return GeeQuotaStatusResponse {
                        ok: true,
                        configured: true,
                        project_id: Some(project_id.to_owned()),
                        registration_state: cached.registration_state.clone(),
                        account_tier: cached.account_tier.clone(),
                        account_tier_label: cached.account_tier_label.clone(),
                        monthly_limit_eecu_seconds: cached.monthly_limit_eecu_seconds,
                        monthly_limit_eecu_hours: cached.monthly_limit_eecu_hours,
                        daily_limit_eecu_seconds: cached.daily_limit_eecu_seconds,
                        read_requests_per_minute: cached.read_requests_per_minute,
                        parallelism_restricted,
                        last_quota_status_header,
                        cloud_monitoring: cached.cloud_monitoring.clone(),
                        local_ledger,
                        fetched_at_ms: cached.fetched_at_ms,
                    };
                }
            }
        }
    }

    // 3. Perform live Google Cloud / Earth Engine inspection
    let registration_state = fetch_project_registration_state(
        client,
        &endpoints.earthengine_v1_base,
        project_id,
        access_token,
    );
    let quotas_opt = fetch_consumer_quota_metrics(
        client,
        &endpoints.serviceusage_v1beta1_base,
        project_id,
        access_token,
    );

    let (account_tier, account_tier_label) =
        classify_gee_account_tier_checked(registration_state.as_deref(), quotas_opt.as_ref());

    let monthly_limit_eecu_seconds = quotas_opt
        .as_ref()
        .and_then(|q| q.monthly_limit_eecu_seconds);
    let monthly_limit_eecu_hours = quotas_opt.as_ref().and_then(|q| q.monthly_limit_eecu_hours);
    let daily_limit_eecu_seconds = quotas_opt.as_ref().and_then(|q| q.daily_limit_eecu_seconds);
    let read_requests_per_minute = quotas_opt.as_ref().and_then(|q| q.read_requests_per_minute);

    let cloud_monitoring = fetch_cloud_monitoring_monthly_usage(
        client,
        &endpoints.monitoring_v3_base,
        project_id,
        access_token,
        monthly_limit_eecu_hours,
    );

    let fetched_at_ms = now_epoch_ms();

    // Only cache when BOTH `getConfig` AND `consumerQuotaMetrics` succeeded so partial/transient failures are never cached!
    if registration_state.is_some() && quotas_opt.is_some() {
        if let Ok(mut guard) = REMOTE_QUOTA_CACHE.lock() {
            guard.insert(
                cache_key,
                CachedRemoteQuotaSnapshot {
                    registration_state: registration_state.clone(),
                    account_tier: account_tier.clone(),
                    account_tier_label: account_tier_label.clone(),
                    monthly_limit_eecu_seconds,
                    monthly_limit_eecu_hours,
                    daily_limit_eecu_seconds,
                    read_requests_per_minute,
                    cloud_monitoring: cloud_monitoring.clone(),
                    fetched_at_ms,
                    cached_at: Instant::now(),
                },
            );
        }
    }

    GeeQuotaStatusResponse {
        ok: true,
        configured: true,
        project_id: Some(project_id.to_owned()),
        registration_state,
        account_tier,
        account_tier_label,
        monthly_limit_eecu_seconds,
        monthly_limit_eecu_hours,
        daily_limit_eecu_seconds,
        read_requests_per_minute,
        parallelism_restricted,
        last_quota_status_header,
        cloud_monitoring,
        local_ledger,
        fetched_at_ms,
    }
}

pub fn get_gee_quota_status(app_data_dir: &Path, refresh: bool) -> Result<Value, String> {
    let creds = match super::load_saved_gee_credentials(app_data_dir) {
        Ok(Some(mat)) => mat,
        Ok(None) => {
            let resp = build_unconfigured_quota_status(app_data_dir);
            return serde_json::to_value(resp).map_err(|e| format!("GEE_QUOTA_JSON_ERR: {e}"));
        }
        Err(_) => {
            let resp = build_unconfigured_quota_status(app_data_dir);
            return serde_json::to_value(resp).map_err(|e| format!("GEE_QUOTA_JSON_ERR: {e}"));
        }
    };

    let clean_project_id: String = creds
        .project_id
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();

    let fallback_unknown = || -> Result<Value, String> {
        let month_utc = current_utc_month_key();
        let (local_ledger, parallelism_restricted, last_quota_status_header) =
            load_local_ledger_snapshot_for_month(app_data_dir, &month_utc);
        let resp = GeeQuotaStatusResponse {
            ok: true,
            configured: true,
            project_id: Some(clean_project_id.clone()),
            registration_state: None,
            account_tier: "unknown".to_owned(),
            account_tier_label: "已配置 GEE 项目".to_owned(),
            monthly_limit_eecu_seconds: None,
            monthly_limit_eecu_hours: None,
            daily_limit_eecu_seconds: None,
            read_requests_per_minute: None,
            parallelism_restricted,
            last_quota_status_header,
            cloud_monitoring: GeeCloudMonitoringQuota {
                unlocked: false,
                reason: Some("monitoring_unavailable".to_owned()),
                monthly_used_eecu_seconds: None,
                monthly_used_eecu_hours: None,
                monthly_remaining_eecu_hours: None,
            },
            local_ledger,
            fetched_at_ms: now_epoch_ms(),
        };
        serde_json::to_value(resp).map_err(|e| format!("GEE_QUOTA_JSON_ERR: {e}"))
    };

    let Ok(client) = super::build_gee_http_client(app_data_dir, 20) else {
        return fallback_unknown();
    };
    let Ok((_, access_token)) =
        super::resolve_gee_access_token(app_data_dir, &client, Some(&creds))
    else {
        return fallback_unknown();
    };

    let resp = fetch_configured_quota_status_with_endpoints(
        app_data_dir,
        &client,
        &clean_project_id,
        &access_token,
        refresh,
        &QuotaApiEndpoints::default(),
    );
    serde_json::to_value(resp).map_err(|e| format!("GEE_QUOTA_JSON_ERR: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    #[test]
    fn unit_tier_classification_and_consumer_quota_metrics_parsing() {
        // 1. Exact tier mapping across Community (150h), Contributor (1000h), Partner (100000h), Unlimited, Commercial, Unknown
        assert_eq!(
            classify_gee_account_tier(Some("REGISTERED_NOT_COMMERCIALLY"), Some(150.0)),
            (
                "noncommercial_community".into(),
                "非商业科研档 (Community)".into()
            )
        );
        assert_eq!(
            classify_gee_account_tier(Some("REGISTERED_NOT_COMMERCIALLY"), Some(1000.0)),
            (
                "noncommercial_contributor".into(),
                "进阶科研档 (Contributor)".into()
            )
        );
        assert_eq!(
            classify_gee_account_tier(Some("REGISTERED_NOT_COMMERCIALLY"), Some(100_000.0)),
            (
                "noncommercial_partner".into(),
                "合作科研档 (Partner)".into()
            )
        );
        assert_eq!(
            classify_gee_account_tier(Some("REGISTERED_NOT_COMMERCIALLY"), None),
            ("noncommercial_unlimited".into(), "非商业科研账号".into())
        );
        assert_eq!(
            classify_gee_account_tier(Some("REGISTERED_COMMERCIALLY"), Some(150.0)),
            ("commercial".into(), "商业计费账号".into())
        );
        assert_eq!(
            classify_gee_account_tier(None, None),
            ("unknown".into(), "已配置 GEE 项目".into())
        );

        // 2. Real Service Usage v1beta1 consumerQuotaMetrics payload shape
        let su_doc = json!({
            "metrics": [
                {
                    "metric": "earthengine.googleapis.com/daily_eecu_usage_time",
                    "consumerQuotaLimits": [{
                        "quotaBuckets": [{
                            "effectiveLimit": "9223372036854775807",
                            "defaultLimit": "9223372036854775807"
                        }]
                    }]
                },
                {
                    "metric": "earthengine.googleapis.com/monthly_eecu_usage_time",
                    "consumerQuotaLimits": [{
                        "quotaBuckets": [{
                            "effectiveLimit": "540000",
                            "defaultLimit": "9223372036854775807"
                        }]
                    }]
                },
                {
                    "metric": "earthengine.googleapis.com/read_requests",
                    "consumerQuotaLimits": [
                        { "quotaBuckets": [{ "effectiveLimit": "6000" }] },
                        { "quotaBuckets": [{ "effectiveLimit": "6000" }] }
                    ]
                }
            ]
        });
        let parsed = parse_consumer_quota_metrics(&su_doc);
        assert_eq!(parsed.monthly_limit_eecu_seconds, Some(540_000.0));
        assert_eq!(parsed.monthly_limit_eecu_hours, Some(150.0));
        assert_eq!(parsed.daily_limit_eecu_seconds, None);
        assert_eq!(parsed.read_requests_per_minute, Some(6000));

        // 3. Real Profile.getProfiles JSON table parsing (`EECU·s` column + null cells)
        let profile_doc = json!({
            "result": {
                "cols": [
                    { "label": "EECU·s", "type": "number" },
                    { "label": "Peak Mem", "type": "number" },
                    { "label": "Count", "type": "number" },
                    { "label": "Description", "type": "string" }
                ],
                "rows": [
                    [{ "f": "  0.028", "v": 0.028 }, { "v": 44000 }, { "v": 39 }, { "v": "(plumbing)" }],
                    [{ "f": "  0.002", "v": 0.002 }, { "v": 21000 }, { "v": 14 }, { "v": "Algorithm Image.reduceRegion" }],
                    [{ "f": "   -   ", "v": null }, { "v": 8000 }, { "v": 1 }, { "v": "Algorithm Image.load" }]
                ]
            }
        });
        assert_eq!(parse_profile_eecu_seconds(&profile_doc), Some(0.03));
    }

    #[test]
    fn unit_cloud_monitoring_403_billing_fallback_and_200_timeseries_summation() {
        // 1. 403 requires billing (cardless noncommercial project)
        let billing_403_body = r#"{
            "error": {
                "code": 403,
                "message": "This API method requires billing to be enabled. Please enable billing on project #123456.",
                "status": "PERMISSION_DENIED"
            }
        }"#;
        let locked = parse_cloud_monitoring_response(403, billing_403_body, Some(150.0));
        assert!(!locked.unlocked);
        assert_eq!(locked.reason.as_deref(), Some("billing_not_enabled"));
        assert_eq!(locked.monthly_used_eecu_seconds, None);
        assert_eq!(locked.monthly_used_eecu_hours, None);
        assert_eq!(locked.monthly_remaining_eecu_hours, None);

        // 2. Non-billing 503 error -> monitoring_unavailable
        let unavail = parse_cloud_monitoring_response(
            503,
            r#"{"error":{"message":"unavailable"}}"#,
            Some(150.0),
        );
        assert!(!unavail.unlocked);
        assert_eq!(unavail.reason.as_deref(), Some("monitoring_unavailable"));

        // 3. 200 OK with empty `{}` (billing enabled, zero usage so far this month)
        let empty_ok = parse_cloud_monitoring_response(200, "{}", Some(150.0));
        assert!(empty_ok.unlocked);
        assert_eq!(empty_ok.reason, None);
        assert_eq!(empty_ok.monthly_used_eecu_seconds, Some(0.0));
        assert_eq!(empty_ok.monthly_used_eecu_hours, Some(0.0));
        assert_eq!(empty_ok.monthly_remaining_eecu_hours, Some(150.0));

        // 4. 200 OK with multiple timeSeries points (doubleValue + int64Value string = 451,800s = 125.5h out of 1000h -> 874.5h remaining)
        let ts_body = json!({
            "timeSeries": [
                {
                    "points": [
                        { "value": { "doubleValue": 360000.0 } },
                        { "value": { "doubleValue": 55800.0 } }
                    ]
                },
                {
                    "points": [
                        { "value": { "int64Value": "36000" } }
                    ]
                }
            ]
        })
        .to_string();
        let unlocked = parse_cloud_monitoring_response(200, &ts_body, Some(1000.0));
        assert!(unlocked.unlocked);
        assert_eq!(unlocked.reason, None);
        assert_eq!(unlocked.monthly_used_eecu_seconds, Some(451_800.0));
        assert_eq!(unlocked.monthly_used_eecu_hours, Some(125.5));
        assert_eq!(unlocked.monthly_remaining_eecu_hours, Some(874.5));
    }

    #[test]
    fn unit_local_monthly_ledger_atomic_persistence_rollover_and_parallelism_flag() {
        let tmp = std::env::temp_dir().join(format!(
            "eostage-gee-ledger-unit-{}-{}",
            std::process::id(),
            now_epoch_ms()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Record October activity (1 analysis with real EECU profile + 1 chunk download without profile)
        let oct_1 = apply_ledger_mutation_for_month(
            &tmp,
            "2026-10",
            LedgerMutationInput {
                op_kind: GeeLedgerOpKind::AnalysisRun,
                op_succeeded: true,
                http_requests: 4,
                wall_seconds: 2.345,
                profiled_eecu_seconds: Some(1.25),
                saw_rate_limit_or_restriction: Some(false),
                last_quota_header: None,
                pending_profile_ids: Vec::new(),
                updated_at_ms: 1790000000000,
            },
        )
        .expect("write oct analysis");
        assert_eq!(oct_1.month_utc, "2026-10");
        assert_eq!(oct_1.total_requests, 4);
        assert_eq!(oct_1.analysis_runs, 1);
        assert!(oct_1.has_eecu_profile);
        assert!((oct_1.profiled_eecu_seconds - 1.25).abs() < 1e-6);

        // Unprofiled call MUST NOT conflate wall_seconds with profiled_eecu_seconds!
        let oct_2 = apply_ledger_mutation_for_month(
            &tmp,
            "2026-10",
            LedgerMutationInput {
                op_kind: GeeLedgerOpKind::ChunkDownload,
                op_succeeded: true,
                http_requests: 3,
                wall_seconds: 5.5,
                profiled_eecu_seconds: None,
                saw_rate_limit_or_restriction: Some(true),
                last_quota_header: Some("parallelism_restricted=true".into()),
                pending_profile_ids: Vec::new(),
                updated_at_ms: 1790000005000,
            },
        )
        .expect("write oct chunk download");
        assert_eq!(oct_2.total_requests, 7);
        assert_eq!(oct_2.analysis_runs, 1);
        assert_eq!(oct_2.chunk_downloads, 1);
        assert!((oct_2.profiled_eecu_seconds - 1.25).abs() < 1e-6);
        assert_eq!(oct_2.last_call_eecu_seconds, None);
        assert!((oct_2.total_wall_seconds - 7.845).abs() < 1e-3);

        // Verify persisted JSON on disk contains zero leftover .tmp files and zero secrets/project IDs
        let ledger_path = tmp.join(LEDGER_FILE_NAME);
        assert!(ledger_path.is_file());
        let raw_disk = fs::read_to_string(&ledger_path).unwrap();
        assert!(!raw_disk.contains("project_id"));
        assert!(!raw_disk.contains("private_key"));

        // Month rollover to November -> clean zero counters for 2026-11 while preserving parallelism_restricted
        let (nov_zero, restricted, last_hdr) =
            load_local_ledger_snapshot_for_month(&tmp, "2026-11");
        assert_eq!(nov_zero.month_utc, "2026-11");
        assert_eq!(nov_zero.total_requests, 0);
        assert_eq!(nov_zero.analysis_runs, 0);
        assert_eq!(nov_zero.chunk_downloads, 0);
        assert_eq!(nov_zero.script_runs, 0);
        assert_eq!(nov_zero.profiled_eecu_seconds, 0.0);
        assert!(!nov_zero.has_eecu_profile);
        assert!(restricted);
        assert_eq!(last_hdr.as_deref(), Some("parallelism_restricted=true"));

        // Observe HTTP response headers via GeeCallTracker (`parallelism_restricted=false` clears flag)
        let mut tracker = GeeCallTracker::new();
        let mut hdrs = reqwest::header::HeaderMap::new();
        hdrs.insert(
            "X-Earth-Engine-Quota-Status",
            reqwest::header::HeaderValue::from_static("parallelism_restricted=false"),
        );
        hdrs.insert(
            "X-Earth-Engine-Computation-Profile",
            reqwest::header::HeaderValue::from_static("PROF123ABC"),
        );
        tracker.observe_http_response(200, &hdrs, 0.42);
        assert_eq!(tracker.http_requests, 1);
        assert_eq!(tracker.saw_rate_limit_or_restriction, Some(false));
        assert_eq!(tracker.observed_profiles.len(), 1);
        assert_eq!(tracker.observed_profiles[0].id, "PROF123ABC");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unit_mock_http_quota_status_caching_refresh_and_safe_value() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_srv = Arc::clone(&hits);

        let srv = std::thread::spawn(move || {
            for _ in 0..6 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req_line = String::from_utf8_lossy(&buf[..n]);
                let first_line = req_line.lines().next().unwrap_or("");
                hits_srv.fetch_add(1, Ordering::SeqCst);

                let (status_line, body) = if first_line.contains("/config") {
                    (
                        "HTTP/1.1 200 OK",
                        r#"{"name":"projects/demo-gee-proj/config","registrationState":"REGISTERED_NOT_COMMERCIALLY"}"#.to_owned(),
                    )
                } else if first_line.contains("/consumerQuotaMetrics") {
                    (
                        "HTTP/1.1 200 OK",
                        json!({
                            "metrics": [
                                {
                                    "metric": "earthengine.googleapis.com/monthly_eecu_usage_time",
                                    "consumerQuotaLimits": [{ "quotaBuckets": [{ "effectiveLimit": "540000" }] }]
                                },
                                {
                                    "metric": "earthengine.googleapis.com/read_requests",
                                    "consumerQuotaLimits": [{ "quotaBuckets": [{ "effectiveLimit": "6000" }] }]
                                }
                            ]
                        })
                        .to_string(),
                    )
                } else {
                    (
                        "HTTP/1.1 403 Forbidden",
                        r#"{"error":{"code":403,"message":"This API method requires billing to be enabled."}}"#.to_owned(),
                    )
                };

                let resp = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let tmp = std::env::temp_dir().join(format!(
            "eostage-gee-quota-http-unit-{}-{}",
            std::process::id(),
            now_epoch_ms()
        ));
        let _ = fs::create_dir_all(&tmp);
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let base = format!("http://{addr}");
        let endpoints = QuotaApiEndpoints {
            earthengine_v1_base: base.clone(),
            serviceusage_v1beta1_base: base.clone(),
            monitoring_v3_base: base,
        };

        // 1st call: hits mock server (3 requests: config + consumerQuotaMetrics + timeSeries)
        let r1 = fetch_configured_quota_status_with_endpoints(
            &tmp,
            &client,
            "demo-gee-proj",
            "tok-test",
            false,
            &endpoints,
        );
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        assert!(r1.configured);
        assert_eq!(r1.account_tier, "noncommercial_community");
        assert_eq!(r1.account_tier_label, "非商业科研档 (Community)");
        assert_eq!(r1.monthly_limit_eecu_hours, Some(150.0));
        assert_eq!(r1.read_requests_per_minute, Some(6000));
        assert!(!r1.cloud_monitoring.unlocked);
        assert_eq!(
            r1.cloud_monitoring.reason.as_deref(),
            Some("billing_not_enabled")
        );

        // 2nd call with `refresh = false`: served from 120s cache (still 3 hits), yet picks up fresh local ledger!
        let cur_month = current_utc_month_key();
        apply_ledger_mutation_for_month(
            &tmp,
            &cur_month,
            LedgerMutationInput {
                op_kind: GeeLedgerOpKind::AnalysisRun,
                op_succeeded: true,
                http_requests: 2,
                wall_seconds: 1.2,
                profiled_eecu_seconds: Some(0.45),
                saw_rate_limit_or_restriction: None,
                last_quota_header: None,
                pending_profile_ids: Vec::new(),
                updated_at_ms: now_epoch_ms(),
            },
        )
        .unwrap();

        let r2 = fetch_configured_quota_status_with_endpoints(
            &tmp,
            &client,
            "demo-gee-proj",
            "tok-test",
            false,
            &endpoints,
        );
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        assert_eq!(r2.local_ledger.total_requests, 2);
        assert_eq!(r2.local_ledger.analysis_runs, 1);
        assert!((r2.local_ledger.profiled_eecu_seconds - 0.45).abs() < 1e-6);

        // 3rd call with `refresh = true`: bypasses cache (hits becomes 6)
        let r3 = fetch_configured_quota_status_with_endpoints(
            &tmp,
            &client,
            "demo-gee-proj",
            "tok-test",
            true,
            &endpoints,
        );
        assert_eq!(hits.load(Ordering::SeqCst), 6);

        // Verify `insar_agent::run::safe_value` accepts `gee_quota_status` JSON verbatim without redacting ANY field!
        let r3_val = serde_json::to_value(&r3).unwrap();
        let safe = insar_agent::run::safe_value(&r3_val)
            .expect("gee_quota_status output must pass safe_value directly");
        assert_eq!(safe, r3_val);
        let sanitized = super::super::sanitize_for_agent_trace(&r3_val);
        assert_eq!(sanitized, r3_val);

        let _ = srv.join();
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unit_serviceusage_failure_not_misclassified_or_cached_vs_true_unlimited() {
        // Direct unit check on `classify_gee_account_tier_checked`:
        // 1. `serviceusage` failure (`None`) -> `"noncommercial_unknown"` (NEVER `"noncommercial_unlimited"`!)
        let (tier_fail, label_fail) =
            classify_gee_account_tier_checked(Some("REGISTERED_NOT_COMMERCIALLY"), None);
        assert_eq!(tier_fail, "noncommercial_unknown");
        assert_eq!(label_fail, "非商业科研账号");

        // 2. `serviceusage` HTTP 200 with `effectiveLimit = 9223372036854775807` (`Some` with `monthly_limit_eecu_hours == None`) -> `"noncommercial_unlimited"`
        let parsed_unlim = parse_consumer_quota_metrics(&json!({
            "metrics": [{
                "metric": "earthengine.googleapis.com/monthly_eecu_usage_time",
                "consumerQuotaLimits": [{ "quotaBuckets": [{ "effectiveLimit": "9223372036854775807" }] }]
            }]
        }));
        assert_eq!(parsed_unlim.monthly_limit_eecu_hours, None);
        let (tier_unlim, label_unlim) = classify_gee_account_tier_checked(
            Some("REGISTERED_NOT_COMMERCIALLY"),
            Some(&parsed_unlim),
        );
        assert_eq!(tier_unlim, "noncommercial_unlimited");
        assert_eq!(label_unlim, "非商业科研账号");

        // 3. End-to-end mock HTTP server:
        //    - Round 1: `getConfig` returns 200, `consumerQuotaMetrics` returns 403 -> must return `noncommercial_unknown` AND MUST NOT populate `REMOTE_QUOTA_CACHE`!
        //    - Round 2 (`refresh = false`): immediately hits server again (`consumerQuotaMetrics` now returns 200 `"540000"`) -> recovers to `noncommercial_community` (150h) and caches!
        //    - Round 3 (`refresh = false`): served from cache without new HTTP hits!
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let su_calls = Arc::new(AtomicUsize::new(0));
        let su_calls_srv = Arc::clone(&su_calls);
        let total_hits = Arc::new(AtomicUsize::new(0));
        let total_hits_srv = Arc::clone(&total_hits);

        let srv = std::thread::spawn(move || {
            for _ in 0..6 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req_line = String::from_utf8_lossy(&buf[..n]);
                let first_line = req_line.lines().next().unwrap_or("");
                total_hits_srv.fetch_add(1, Ordering::SeqCst);

                let (status_line, body) = if first_line.contains("/config") {
                    (
                        "HTTP/1.1 200 OK",
                        r#"{"name":"projects/partial-proj/config","registrationState":"REGISTERED_NOT_COMMERCIALLY"}"#.to_owned(),
                    )
                } else if first_line.contains("/consumerQuotaMetrics") {
                    let call_idx = su_calls_srv.fetch_add(1, Ordering::SeqCst);
                    if call_idx == 0 {
                        (
                            "HTTP/1.1 403 Forbidden",
                            r#"{"error":{"code":403,"message":"Service Usage API disabled"}}"#
                                .to_owned(),
                        )
                    } else {
                        (
                            "HTTP/1.1 200 OK",
                            json!({
                                "metrics": [{
                                    "metric": "earthengine.googleapis.com/monthly_eecu_usage_time",
                                    "consumerQuotaLimits": [{ "quotaBuckets": [{ "effectiveLimit": "540000" }] }]
                                }]
                            })
                            .to_string(),
                        )
                    }
                } else {
                    (
                        "HTTP/1.1 403 Forbidden",
                        r#"{"error":{"code":403,"message":"requires billing to be enabled"}}"#
                            .to_owned(),
                    )
                };

                let resp = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let tmp = std::env::temp_dir().join(format!(
            "eostage-gee-quota-partial-unit-{}-{}",
            std::process::id(),
            now_epoch_ms()
        ));
        let _ = fs::create_dir_all(&tmp);
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let base = format!("http://{addr}");
        let endpoints = QuotaApiEndpoints {
            earthengine_v1_base: base.clone(),
            serviceusage_v1beta1_base: base.clone(),
            monitoring_v3_base: base,
        };

        // Round 1: `serviceusage` fails -> `noncommercial_unknown`, `monthly_limit_eecu_hours == None`, NOT cached!
        let r1 = fetch_configured_quota_status_with_endpoints(
            &tmp,
            &client,
            "partial-proj",
            "tok-test",
            false,
            &endpoints,
        );
        assert_eq!(r1.account_tier, "noncommercial_unknown");
        assert_eq!(r1.monthly_limit_eecu_hours, None);
        assert_eq!(total_hits.load(Ordering::SeqCst), 3);

        // Round 2 (`refresh = false`): because Round 1 was NOT cached, it re-queries and recovers to `noncommercial_community` (150h)!
        let r2 = fetch_configured_quota_status_with_endpoints(
            &tmp,
            &client,
            "partial-proj",
            "tok-test",
            false,
            &endpoints,
        );
        assert_eq!(r2.account_tier, "noncommercial_community");
        assert_eq!(r2.monthly_limit_eecu_hours, Some(150.0));
        assert_eq!(total_hits.load(Ordering::SeqCst), 6);

        // Round 3 (`refresh = false`): served from cache (`total_hits` stays 6)!
        let r3 = fetch_configured_quota_status_with_endpoints(
            &tmp,
            &client,
            "partial-proj",
            "tok-test",
            false,
            &endpoints,
        );
        assert_eq!(r3.account_tier, "noncommercial_community");
        assert_eq!(r3.monthly_limit_eecu_hours, Some(150.0));
        assert_eq!(total_hits.load(Ordering::SeqCst), 6);

        let _ = srv.join();
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unit_gee_call_tracker_merge_preserves_429_header_against_200_ok() {
        let mut parent = GeeCallTracker::new();
        parent.merge_from(GeeCallTracker {
            http_requests: 1,
            total_wall_seconds: 0.4,
            observed_profiles: Vec::new(),
            saw_rate_limit_or_restriction: Some(true),
            last_quota_header: Some("http_429_rate_limited".into()),
        });
        parent.merge_from(GeeCallTracker {
            http_requests: 1,
            total_wall_seconds: 0.3,
            observed_profiles: Vec::new(),
            saw_rate_limit_or_restriction: Some(false),
            last_quota_header: Some("parallelism_restricted=false".into()),
        });

        assert_eq!(parent.http_requests, 2);
        assert_eq!(parent.saw_rate_limit_or_restriction, Some(true));
        assert_eq!(
            parent.last_quota_header.as_deref(),
            Some("http_429_rate_limited")
        );
    }
}
