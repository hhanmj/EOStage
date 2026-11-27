use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

const PREFS_FILENAME: &str = "desktop_ui_prefs.v1.json";
const ACCOUNT_CACHE_FILENAME: &str = "eostage-account.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopOnboardingState {
    pub completed: bool,
    pub source: String,
    pub returning_user: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopUiPrefsFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    onboarding_completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    onboarding_completed_at: Option<String>,
    #[serde(default)]
    diagnostics_upload_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_resource_id: Option<String>,
}

fn default_version() -> u32 {
    1
}

impl Default for DesktopUiPrefsFile {
    fn default() -> Self {
        Self {
            version: 1,
            onboarding_completed: false,
            onboarding_completed_at: None,
            diagnostics_upload_enabled: false,
            active_resource_id: None,
        }
    }
}

fn prefs_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(PREFS_FILENAME)
}

fn read_prefs_file(path: &Path) -> Option<DesktopUiPrefsFile> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<DesktopUiPrefsFile>(&raw).ok()
}

fn write_prefs_file_atomic(app_data_dir: &Path, prefs: &DesktopUiPrefsFile) -> Result<(), String> {
    std::fs::create_dir_all(app_data_dir).map_err(|err| format!("创建桌面配置目录失败: {err}"))?;
    let path = prefs_path(app_data_dir);
    let tmp = app_data_dir.join(format!(
        ".{PREFS_FILENAME}.{}.{}.tmp",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    let payload =
        serde_json::to_vec_pretty(prefs).map_err(|err| format!("序列化桌面偏好配置失败: {err}"))?;
    {
        let mut file =
            std::fs::File::create(&tmp).map_err(|err| format!("写入临时偏好文件失败: {err}"))?;
        file.write_all(&payload)
            .map_err(|err| format!("写入偏好数据失败: {err}"))?;
        let _ = file.sync_all();
    }
    if let Err(err) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("保存桌面偏好配置失败: {err}"));
    }
    Ok(())
}

/// Detect returning-user state strictly within `app_data_dir` (never OS Keychain
/// or global directories outside the profile root).
fn has_local_returning_user_artifacts(app_data_dir: &Path, has_existing_tasks: bool) -> bool {
    if has_existing_tasks {
        return true;
    }
    if app_data_dir
        .join("agent")
        .join(ACCOUNT_CACHE_FILENAME)
        .is_file()
    {
        return true;
    }
    let sessions_dir = app_data_dir.join("agent").join("sessions");
    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && p.join("session.json").is_file() {
                return true;
            }
        }
    }
    false
}

pub fn get_desktop_onboarding_state(
    app_data_dir: &Path,
    has_existing_tasks: bool,
) -> DesktopOnboardingState {
    let path = prefs_path(app_data_dir);
    if let Some(prefs) = read_prefs_file(&path) {
        return DesktopOnboardingState {
            completed: prefs.onboarding_completed,
            source: "prefs_file".to_owned(),
            returning_user: false,
        };
    }

    if has_local_returning_user_artifacts(app_data_dir, has_existing_tasks) {
        let prefs = DesktopUiPrefsFile {
            version: 1,
            onboarding_completed: true,
            onboarding_completed_at: Some(chrono::Utc::now().to_rfc3339()),
            diagnostics_upload_enabled: false,
            active_resource_id: None,
        };
        let _ = write_prefs_file_atomic(app_data_dir, &prefs);
        return DesktopOnboardingState {
            completed: true,
            source: "returning_user".to_owned(),
            returning_user: true,
        };
    }

    DesktopOnboardingState {
        completed: false,
        source: "default".to_owned(),
        returning_user: false,
    }
}

pub fn set_desktop_onboarding_completed(
    app_data_dir: &Path,
    completed: bool,
) -> Result<DesktopOnboardingState, String> {
    let mut prefs = read_prefs_file(&prefs_path(app_data_dir)).unwrap_or_default();
    prefs.version = 1;
    prefs.onboarding_completed = completed;
    prefs.onboarding_completed_at = completed.then(|| chrono::Utc::now().to_rfc3339());
    write_prefs_file_atomic(app_data_dir, &prefs)?;
    Ok(DesktopOnboardingState {
        completed,
        source: "prefs_file".to_owned(),
        returning_user: false,
    })
}

pub fn get_diagnostics_upload_enabled(app_data_dir: &Path) -> bool {
    read_prefs_file(&prefs_path(app_data_dir))
        .map(|prefs| prefs.diagnostics_upload_enabled)
        .unwrap_or(false)
}

pub fn set_diagnostics_upload_enabled(app_data_dir: &Path, enabled: bool) -> Result<bool, String> {
    let mut prefs = read_prefs_file(&prefs_path(app_data_dir)).unwrap_or_default();
    prefs.version = 1;
    prefs.diagnostics_upload_enabled = enabled;
    write_prefs_file_atomic(app_data_dir, &prefs)?;
    Ok(enabled)
}

fn validate_active_resource_id(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 128
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
    {
        return Err("无效的数据源标识".to_owned());
    }
    Ok(Some(trimmed.to_owned()))
}

pub fn get_active_resource_id(app_data_dir: &Path) -> Option<String> {
    let raw = read_prefs_file(&prefs_path(app_data_dir))?.active_resource_id?;
    validate_active_resource_id(&raw).ok().flatten()
}

pub fn set_active_resource_id(
    app_data_dir: &Path,
    resource_id: &str,
    has_existing_tasks: bool,
) -> Result<Option<String>, String> {
    let normalized = validate_active_resource_id(resource_id)?;
    let path = prefs_path(app_data_dir);
    let mut prefs = match read_prefs_file(&path) {
        Some(existing) => existing,
        None => {
            let returning = has_local_returning_user_artifacts(app_data_dir, has_existing_tasks);
            DesktopUiPrefsFile {
                version: 1,
                onboarding_completed: returning,
                onboarding_completed_at: returning.then(|| chrono::Utc::now().to_rfc3339()),
                diagnostics_upload_enabled: false,
                active_resource_id: None,
            }
        }
    };
    prefs.version = 1;
    prefs.active_resource_id = normalized.clone();
    write_prefs_file_atomic(app_data_dir, &prefs)?;
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(1);

    fn temp_app_dir(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "insar-desktop-ui-prefs-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp app_data_dir");
        dir
    }

    #[test]
    fn fresh_app_dir_starts_uncompleted_and_persists_completion_across_reload() {
        let dir = temp_app_dir("fresh");
        let initial = get_desktop_onboarding_state(&dir, false);
        assert!(!initial.completed);
        assert_eq!(initial.source, "default");
        assert!(!initial.returning_user);

        let saved = set_desktop_onboarding_completed(&dir, true).expect("save onboarding state");
        assert!(saved.completed);
        assert_eq!(saved.source, "prefs_file");

        let reloaded = get_desktop_onboarding_state(&dir, false);
        assert!(reloaded.completed);
        assert_eq!(reloaded.source, "prefs_file");
        assert!(!reloaded.returning_user);

        let reset = set_desktop_onboarding_completed(&dir, false).expect("reset onboarding state");
        assert!(!reset.completed);
        let reloaded_reset = get_desktop_onboarding_state(&dir, true);
        assert!(!reloaded_reset.completed);
        assert_eq!(reloaded_reset.source, "prefs_file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn returning_user_session_or_account_cache_or_tasks_auto_completes_within_app_dir() {
        let dir_sessions = temp_app_dir("sessions");
        let sdir = dir_sessions.join("agent").join("sessions").join("s1");
        std::fs::create_dir_all(&sdir).unwrap();
        std::fs::write(sdir.join("session.json"), "{}").unwrap();
        let st_sessions = get_desktop_onboarding_state(&dir_sessions, false);
        assert!(st_sessions.completed);
        assert!(st_sessions.returning_user);
        assert_eq!(st_sessions.source, "returning_user");
        // Auto-persisted to prefs file so later reads hit prefs_file
        assert_eq!(
            get_desktop_onboarding_state(&dir_sessions, false).source,
            "prefs_file"
        );
        let _ = std::fs::remove_dir_all(&dir_sessions);

        let dir_acct = temp_app_dir("account");
        std::fs::create_dir_all(dir_acct.join("agent")).unwrap();
        std::fs::write(dir_acct.join("agent").join(ACCOUNT_CACHE_FILENAME), "{}").unwrap();
        let st_acct = get_desktop_onboarding_state(&dir_acct, false);
        assert!(st_acct.completed);
        assert!(st_acct.returning_user);
        let _ = std::fs::remove_dir_all(&dir_acct);

        let dir_tasks = temp_app_dir("tasks");
        let st_tasks = get_desktop_onboarding_state(&dir_tasks, true);
        assert!(st_tasks.completed);
        assert!(st_tasks.returning_user);
        let _ = std::fs::remove_dir_all(&dir_tasks);
    }

    #[test]
    fn diagnostics_upload_defaults_false_and_persists_independently() {
        let dir = temp_app_dir("diag-pref");
        assert!(!get_diagnostics_upload_enabled(&dir));
        set_desktop_onboarding_completed(&dir, true).unwrap();
        assert!(!get_diagnostics_upload_enabled(&dir));

        assert!(set_diagnostics_upload_enabled(&dir, true).unwrap());
        assert!(get_diagnostics_upload_enabled(&dir));
        assert!(get_desktop_onboarding_state(&dir, false).completed);

        assert!(!set_diagnostics_upload_enabled(&dir, false).unwrap());
        assert!(!get_diagnostics_upload_enabled(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn active_resource_id_persists_and_preserves_returning_user_onboarding() {
        let dir = temp_app_dir("active-resource");
        assert_eq!(get_active_resource_id(&dir), None);

        // Simulate returning user artifact existing before prefs file is created
        std::fs::create_dir_all(dir.join("agent")).unwrap();
        std::fs::write(dir.join("agent").join(ACCOUNT_CACHE_FILENAME), "{}").unwrap();

        let saved = set_active_resource_id(&dir, "science.landsat.c2-l2.v1", false)
            .expect("save active resource id");
        assert_eq!(saved.as_deref(), Some("science.landsat.c2-l2.v1"));
        assert_eq!(
            get_active_resource_id(&dir).as_deref(),
            Some("science.landsat.c2-l2.v1")
        );

        let onboarding = get_desktop_onboarding_state(&dir, false);
        assert!(onboarding.completed);

        assert!(set_active_resource_id(&dir, "bad/path", false).is_err());
        assert_eq!(
            get_active_resource_id(&dir).as_deref(),
            Some("science.landsat.c2-l2.v1")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
