//! Explicit user-initiated release update checker.
//!
//! Privacy contract (`PRIV-REG-001`): NEVER invoked automatically at startup or in background timers;
//! only executed when the user explicitly clicks "检查更新" in the About modal.

use insar_application_service::AgentApplication;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::oauth_login::build_http_client;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/hhanmj/EOStage/releases/latest";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppUpdateCheckResult {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub has_update: bool,
    pub release_url: Option<String>,
    pub release_notes: Option<String>,
    pub checked_at: String,
    pub status: String, // "up_to_date" | "update_available" | "no_release" | "offline_error"
    pub message_zh: String,
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedVersion {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<String>,
}

fn parse_version(raw: &str) -> Option<ParsedVersion> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    if trimmed.is_empty() {
        return None;
    }
    let (core, pre) = match trimmed.split_once('-') {
        Some((c, p)) => (c, Some(p.to_owned())),
        None => (trimmed, None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    Some(ParsedVersion {
        major,
        minor,
        patch,
        prerelease: pre,
    })
}

pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    let (Some(cur), Some(cand)) = (parse_version(current), parse_version(candidate)) else {
        return false;
    };
    match (cand.major, cand.minor, cand.patch).cmp(&(cur.major, cur.minor, cur.patch)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (&cur.prerelease, &cand.prerelease) {
            // Stable 2.2.0 > 2.2.0-beta.0
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(a), Some(b)) => b > a,
            (None, None) => false,
        },
    }
}

fn update_check_url() -> String {
    std::env::var("EOSTAGE_UPDATE_CHECK_URL")
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| GITHUB_LATEST_RELEASE_URL.to_owned())
}

pub fn check_latest_release(
    application: &AgentApplication,
    current_version: &str,
) -> AppUpdateCheckResult {
    let checked_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let client = match build_http_client(application, Duration::from_secs(8)) {
        Ok(c) => c,
        Err(err) => {
            return AppUpdateCheckResult {
                current_version: current_version.to_owned(),
                latest_version: None,
                has_update: false,
                release_url: None,
                release_notes: None,
                checked_at,
                status: "offline_error".to_owned(),
                message_zh: format!("无法初始化网络请求：{err}"),
            };
        }
    };

    let resp = match client
        .get(update_check_url())
        .header("Accept", "application/vnd.github+json")
        .send()
    {
        Ok(r) => r,
        Err(_) => {
            return AppUpdateCheckResult {
                current_version: current_version.to_owned(),
                latest_version: None,
                has_update: false,
                release_url: None,
                release_notes: None,
                checked_at,
                status: "offline_error".to_owned(),
                message_zh: "检查更新失败：暂时无法连接发布服务器，请检查网络或代理设置。"
                    .to_owned(),
            };
        }
    };

    if resp.status().as_u16() == 404 {
        return AppUpdateCheckResult {
            current_version: current_version.to_owned(),
            latest_version: None,
            has_update: false,
            release_url: Some("https://github.com/hhanmj/EOStage/releases".to_owned()),
            release_notes: None,
            checked_at,
            status: "no_release".to_owned(),
            message_zh: "当前已是最新测试版本，暂无更高正式版本发布。".to_owned(),
        };
    }

    if !resp.status().is_success() {
        return AppUpdateCheckResult {
            current_version: current_version.to_owned(),
            latest_version: None,
            has_update: false,
            release_url: None,
            release_notes: None,
            checked_at,
            status: "offline_error".to_owned(),
            message_zh: format!(
                "检查更新失败（HTTP {}），请稍后再试。",
                resp.status().as_u16()
            ),
        };
    }

    let payload: serde_json::Value = match resp.json() {
        Ok(v) => v,
        Err(_) => {
            return AppUpdateCheckResult {
                current_version: current_version.to_owned(),
                latest_version: None,
                has_update: false,
                release_url: None,
                release_notes: None,
                checked_at,
                status: "offline_error".to_owned(),
                message_zh: "检查更新失败：返回格式异常。".to_owned(),
            };
        }
    };

    let tag = payload
        .get("tag_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(latest_tag) = tag else {
        return AppUpdateCheckResult {
            current_version: current_version.to_owned(),
            latest_version: None,
            has_update: false,
            release_url: Some("https://github.com/hhanmj/EOStage/releases".to_owned()),
            release_notes: None,
            checked_at,
            status: "no_release".to_owned(),
            message_zh: "暂未检测到有效版本标签。".to_owned(),
        };
    };

    let release_url = payload
        .get("html_url")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| Some("https://github.com/hhanmj/EOStage/releases".to_owned()));
    let release_notes = payload
        .get("body")
        .and_then(|v| v.as_str())
        .map(|s| s.chars().take(600).collect::<String>());
    let has_update = is_newer_version(current_version, latest_tag);
    AppUpdateCheckResult {
        current_version: current_version.to_owned(),
        latest_version: Some(latest_tag.to_owned()),
        has_update,
        release_url,
        release_notes,
        checked_at,
        status: if has_update {
            "update_available".to_owned()
        } else {
            "up_to_date".to_owned()
        },
        message_zh: if has_update {
            format!(
                "发现新版本 {}（当前 {}），可前往下载安装。",
                latest_tag, current_version
            )
        } else {
            format!("当前已是最新版本（{}）。", current_version)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison_handles_v_prefix_and_prerelease() {
        assert!(is_newer_version("2.2.0-beta.0", "v2.2.0"));
        assert!(is_newer_version("2.2.0-beta.0", "2.3.0-beta.0"));
        assert!(is_newer_version("2.2.0-beta.0", "2.2.0-beta.1"));
        assert!(!is_newer_version("2.2.0-beta.0", "2.2.0-beta.0"));
        assert!(!is_newer_version("2.2.0", "v2.2.0-beta.9"));
        assert!(!is_newer_version("2.2.0", "2.1.9"));
        assert!(!is_newer_version("2.2.0", "invalid"));
    }
}
