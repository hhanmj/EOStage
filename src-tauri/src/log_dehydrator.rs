use insar_gis::SubprocessScratchBudget;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const RTK_PROBE_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedRtkInfo {
    pub installed: bool,
    pub source: String,
    pub executable_path: Option<String>,
    pub version: Option<String>,
    pub detail: String,
}

impl Default for SharedRtkInfo {
    fn default() -> Self {
        Self {
            installed: false,
            source: "missing".to_string(),
            executable_path: None,
            version: None,
            detail: "未检测到本机共享 RTK；已由内置 GIS/Conda 日志脱水组件接管".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogDehydratorInfo {
    pub builtin_ready: bool,
    pub mode: String,
    pub shared_rtk: SharedRtkInfo,
    pub capabilities: Vec<String>,
    pub detail: String,
}

impl Default for LogDehydratorInfo {
    fn default() -> Self {
        Self {
            builtin_ready: true,
            mode: "auto".to_string(),
            shared_rtk: SharedRtkInfo::default(),
            capabilities: default_capabilities(),
            detail: "内置 GIS/Conda 日志脱水组件已就绪（支持免安装 ANSI/进度条折叠与尾端堆栈保留）"
                .to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DehydrationSummary {
    pub engine: String,
    pub raw_bytes: usize,
    pub kept_bytes: usize,
    pub raw_lines: usize,
    pub kept_lines: usize,
    pub collapsed_progress_lines: usize,
    pub collapsed_duplicate_warnings: usize,
    pub head_tail_elided_lines: usize,
}

fn default_capabilities() -> Vec<String> {
    vec![
        "strip_ansi_and_cr".to_string(),
        "collapse_gis_progress".to_string(),
        "dedup_python_gdal_warnings".to_string(),
        "head_tail_traceback_budget".to_string(),
        "shared_rtk_compat".to_string(),
    ]
}

pub fn normalize_dehydrator_mode(raw: Option<&str>) -> String {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("builtin_only") => "builtin_only".to_string(),
        _ => "auto".to_string(),
    }
}

pub fn is_safe_executable_file(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        (mode & 0o111 != 0) && (mode & 0o002 == 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn validate_custom_rtk_path(raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err("RTK 路径不能为空或包含空字符".to_string());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(format!("RTK 自定义路径必须是绝对路径: {trimmed}"));
    }
    for comp in path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(format!("RTK 自定义路径不允许包含 '..': {trimmed}"));
        }
    }
    let canonical = fs::canonicalize(&path)
        .map_err(|err| format!("RTK 自定义路径不可访问 ({trimmed}): {err}"))?;
    if !is_safe_executable_file(&canonical) {
        return Err(format!(
            "RTK 自定义路径不是安全的可执行文件或权限不合要求: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn find_shared_rtk_candidate() -> Option<(PathBuf, &'static str)> {
    #[cfg(windows)]
    let bin_names: &[&str] = &["rtk.exe", "rtk"];
    #[cfg(not(windows))]
    let bin_names: &[&str] = &["rtk"];

    let mut path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|d| d.is_absolute())
                .collect()
        })
        .unwrap_or_default();

    #[cfg(target_os = "macos")]
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let p = PathBuf::from(extra);
        if !path_dirs.iter().any(|d| d == &p) {
            path_dirs.push(p);
        }
    }

    for dir in path_dirs {
        for name in bin_names {
            let cand = dir.join(name);
            if let Ok(canonical) = fs::canonicalize(&cand) {
                if is_safe_executable_file(&canonical) {
                    return Some((canonical, "system_path"));
                }
            }
        }
    }

    let home_dir = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(home) = home_dir {
        let cargo_bin = PathBuf::from(home).join(".cargo").join("bin");
        for name in bin_names {
            let cand = cargo_bin.join(name);
            if let Ok(canonical) = fs::canonicalize(&cand) {
                if is_safe_executable_file(&canonical) {
                    return Some((canonical, "cargo_bin"));
                }
            }
        }
    }

    None
}

fn create_dehydrator_scratch_dir(app_data_dir: &Path) -> Result<PathBuf, String> {
    let base = app_data_dir.join("scratch").join("log-dehydrator");
    fs::create_dir_all(&base)
        .map_err(|err| format!("创建日志脱水组件临时目录失败 ({}): {err}", base.display()))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = base.join(format!("probe-{}-{nanos}", std::process::id()));
    fs::create_dir(&dir)
        .map_err(|err| format!("创建 RTK 探测临时目录失败 ({}): {err}", dir.display()))?;
    Ok(dir)
}

fn apply_rtk_probe_env(cmd: &mut Command) {
    cmd.env_clear();
    #[cfg(unix)]
    const KEYS: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "LC_CTYPE", "TERM"];
    #[cfg(windows)]
    const KEYS: &[&str] = &[
        "PATH",
        "SYSTEMROOT",
        "COMSPEC",
        "WINDIR",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
    ];
    for key in KEYS {
        if let Some(val) = std::env::var_os(key) {
            cmd.env(key, val);
        }
    }
}

fn probe_rtk_version(executable: &Path, app_data_dir: &Path) -> Result<String, String> {
    let scratch_dir = create_dehydrator_scratch_dir(app_data_dir)?;
    struct CleanGuard(PathBuf);
    impl Drop for CleanGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _guard = CleanGuard(scratch_dir.clone());

    let budget = SubprocessScratchBudget {
        max_scratch_bytes: 8 * 1024 * 1024,
        max_scratch_files: 16,
        timeout: Duration::from_secs(RTK_PROBE_TIMEOUT_SECS),
        poll_interval: Duration::from_millis(25),
    };
    let mut cmd = Command::new(executable);
    apply_rtk_probe_env(&mut cmd);
    cmd.current_dir(&scratch_dir).arg("--version");

    let outcome = budget
        .execute_command(&mut cmd, &scratch_dir)
        .map_err(|err| format!("探测 RTK 版本失败: {err}"))?;
    if outcome.status_code != Some(0) {
        return Err(format!(
            "RTK --version 返回非零退出码: {:?}",
            outcome.status_code
        ));
    }
    let stdout = String::from_utf8_lossy(&outcome.stdout);
    let first_line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| "RTK --version 未返回有效版本文本".to_string())?;
    if !first_line.to_ascii_lowercase().starts_with("rtk") {
        return Err(format!("目标可执行文件版本输出并非 RTK: {first_line}"));
    }
    Ok(first_line.to_string())
}

pub fn discover_shared_rtk(
    custom_path: Option<&str>,
    app_data_dir: &Path,
    probe_version: bool,
) -> SharedRtkInfo {
    let custom_trimmed = custom_path.map(str::trim).filter(|s| !s.is_empty());
    if let Some(raw) = custom_trimmed {
        match validate_custom_rtk_path(raw) {
            Ok(exe) => match probe_rtk_version(&exe, app_data_dir) {
                Ok(ver) => {
                    return SharedRtkInfo {
                        installed: true,
                        source: "custom_path".to_string(),
                        executable_path: Some(exe.to_string_lossy().to_string()),
                        version: Some(ver.clone()),
                        detail: format!("已验证自定义共享 RTK ({ver})"),
                    };
                }
                Err(err) => {
                    return SharedRtkInfo {
                        installed: false,
                        source: "missing".to_string(),
                        executable_path: Some(exe.to_string_lossy().to_string()),
                        version: None,
                        detail: format!("自定义 RTK 校验失败: {err}"),
                    };
                }
            },
            Err(err) => {
                return SharedRtkInfo {
                    installed: false,
                    source: "missing".to_string(),
                    executable_path: None,
                    version: None,
                    detail: err,
                };
            }
        }
    }

    let Some((exe, source)) = find_shared_rtk_candidate() else {
        return SharedRtkInfo::default();
    };

    if probe_version {
        match probe_rtk_version(&exe, app_data_dir) {
            Ok(ver) => SharedRtkInfo {
                installed: true,
                source: source.to_string(),
                executable_path: Some(exe.to_string_lossy().to_string()),
                version: Some(ver.clone()),
                detail: format!("已检测到本机共享 RTK ({ver})"),
            },
            Err(err) => SharedRtkInfo {
                installed: false,
                source: "missing".to_string(),
                executable_path: None,
                version: None,
                detail: format!("系统路径中的 rtk 不可用: {err}；已回退至内置脱水组件"),
            },
        }
    } else {
        SharedRtkInfo {
            installed: true,
            source: source.to_string(),
            executable_path: Some(exe.to_string_lossy().to_string()),
            version: None,
            detail: "已发现本机共享 RTK（点击“刷新检测”可验证版本）".to_string(),
        }
    }
}

pub fn inspect_log_dehydrator(
    custom_rtk_path: Option<&str>,
    mode_pref: Option<&str>,
    app_data_dir: &Path,
    probe_version: bool,
) -> LogDehydratorInfo {
    let mode = normalize_dehydrator_mode(mode_pref);
    let shared_rtk = discover_shared_rtk(custom_rtk_path, app_data_dir, probe_version);
    let detail = if mode == "builtin_only" {
        "当前固定使用内置 GIS/Conda 日志脱水组件（免外部 RTK 依赖）".to_string()
    } else if shared_rtk.installed {
        format!(
            "已启用内置 GIS/Conda 日志脱水组件，并已自动关联本机共享 RTK（{}）",
            shared_rtk.version.as_deref().unwrap_or("已发现")
        )
    } else {
        "内置 GIS/Conda 日志脱水组件运行中（自动折叠进度条与重复告警，保留关键堆栈；本机未装 RTK 不影响使用）"
            .to_string()
    };

    LogDehydratorInfo {
        builtin_ready: true,
        mode,
        shared_rtk,
        capabilities: default_capabilities(),
        detail,
    }
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1B {
            i += 1;
            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if b.is_ascii_alphabetic() || b == b'~' || b == b'@' {
                        break;
                    }
                }
                continue;
            }
            if i < bytes.len() && bytes[i] == b']' {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1B && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }
        let Some(ch) = input[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn is_progress_spam_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("traceback")
        || lower.contains("error")
        || lower.contains("exception")
        || lower.contains("warning")
        || trimmed.starts_with('{')
        || trimmed.starts_with('[')
    {
        return false;
    }

    // GDAL "0...10...20..." tick lines
    let no_dots = trimmed.replace("- done", "");
    let is_gdal_ticks = trimmed.starts_with("0...")
        || (trimmed.contains("...")
            && no_dots
                .trim()
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c.is_whitespace()));
    if is_gdal_ticks {
        return true;
    }

    // Pure percentage tick or tqdm/conda progress bar line
    let has_percent = trimmed.contains('%');
    let has_bar_glyph = trimmed.contains('█')
        || trimmed.contains('▏')
        || trimmed.contains('▎')
        || trimmed.contains('▍')
        || trimmed.contains('▌')
        || trimmed.contains('▋')
        || trimmed.contains('▊')
        || trimmed.contains('▉')
        || trimmed.contains("it/s")
        || trimmed.contains("s/it");

    if has_percent && has_bar_glyph {
        return true;
    }

    if let Some(stripped) = trimmed
        .strip_prefix("Progress:")
        .or_else(|| trimmed.strip_prefix("progress:"))
    {
        let rest = stripped.trim().trim_end_matches('%').trim();
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return true;
        }
    }

    if let Some(pct_pos) = trimmed.find('%') {
        let before = trimmed[..pct_pos].trim();
        let after = trimmed[pct_pos + 1..].trim();
        if !before.is_empty()
            && before.chars().all(|c| c.is_ascii_digit() || c == '.')
            && after.is_empty()
        {
            return true;
        }
    }

    false
}

fn warning_signature(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    const MARKERS: &[&str] = &[
        "FutureWarning:",
        "DeprecationWarning:",
        "UserWarning:",
        "RuntimeWarning:",
        "PROJ:",
        "Warning 1:",
        "Warning 6:",
        "CPLE_AppDefined",
        "GDALWarning",
    ];
    let core = if let Some((idx, _)) = MARKERS
        .iter()
        .filter_map(|m| trimmed.find(m).map(|p| (p, *m)))
        .min()
    {
        &trimmed[idx..]
    } else if trimmed.starts_with("WARNING:") || trimmed.starts_with("WARN ") {
        trimmed
    } else {
        return None;
    };

    // Normalize hex pointers and variable digits so repeated occurrences collapse cleanly.
    let mut normalized = String::with_capacity(core.len());
    let mut chars = core.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '0' && chars.peek().is_some_and(|n| *n == 'x' || *n == 'X') {
            chars.next();
            while chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                chars.next();
            }
            normalized.push_str("0x#");
        } else if ch.is_ascii_digit() {
            while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                chars.next();
            }
            normalized.push('#');
        } else {
            normalized.push(ch);
        }
    }
    Some(normalized)
}

fn truncate_utf8_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn truncate_utf8_suffix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut start = s.len().saturating_sub(max_bytes);
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

fn apply_head_tail_budget(lines: &[String], max_bytes: usize) -> (String, usize, usize) {
    let joined = lines.join("\n");
    if joined.len() <= max_bytes {
        return (joined, lines.len(), 0);
    }

    if max_bytes < 140 {
        let clipped = truncate_utf8_suffix(&joined, max_bytes).to_string();
        let kept_lines = clipped.lines().count();
        let elided = lines.len().saturating_sub(kept_lines).max(1);
        return (clipped, kept_lines, elided);
    }

    // Reserve enough room for `\n... [已脱水省略中间 NNNNN 行冗余日志] ...\n`.
    let marker_reserve = 72usize;
    let usable = max_bytes.saturating_sub(marker_reserve);
    let head_budget = (usable * 35) / 100;
    let tail_budget = usable.saturating_sub(head_budget);

    let mut head_lines: Vec<String> = Vec::new();
    let mut head_bytes = 0usize;
    let mut head_idx = 0usize;
    for line in lines {
        let cost = line.len() + usize::from(!head_lines.is_empty());
        if head_bytes + cost > head_budget {
            break;
        }
        head_lines.push(line.clone());
        head_bytes += cost;
        head_idx += 1;
    }

    let mut tail_rev: Vec<String> = Vec::new();
    let mut tail_bytes = 0usize;
    let mut tail_start = lines.len();
    for idx in (head_idx..lines.len()).rev() {
        let line = &lines[idx];
        let cost = line.len() + usize::from(!tail_rev.is_empty());
        if tail_bytes + cost > tail_budget {
            break;
        }
        tail_rev.push(line.clone());
        tail_bytes += cost;
        tail_start = idx;
    }

    if tail_rev.is_empty() && tail_start > head_idx {
        // Always retain the final traceback / failure line even when a single line is oversized.
        let last = &lines[lines.len() - 1];
        tail_rev.push(truncate_utf8_suffix(last, tail_budget).to_string());
        tail_start = lines.len() - 1;
    }

    tail_rev.reverse();
    let elided_lines = tail_start.saturating_sub(head_idx).max(1);
    let marker = format!("... [已脱水省略中间 {elided_lines} 行冗余日志] ...");

    let mut assembled_lines = Vec::with_capacity(head_lines.len() + 1 + tail_rev.len());
    assembled_lines.extend(head_lines.iter().cloned());
    assembled_lines.push(marker.clone());
    assembled_lines.extend(tail_rev.iter().cloned());
    let mut assembled = assembled_lines.join("\n");

    // Guarantee final byte bound strictly.
    if assembled.len() > max_bytes {
        let overflow = assembled.len() - max_bytes;
        if let Some(first) = head_lines.first() {
            let trimmed_head_budget = head_bytes.saturating_sub(overflow);
            let replacement = if trimmed_head_budget > 0 {
                truncate_utf8_prefix(first, trimmed_head_budget).to_string()
            } else {
                String::new()
            };
            let mut next_lines = Vec::new();
            if !replacement.is_empty() {
                next_lines.push(replacement);
            }
            next_lines.push(marker);
            next_lines.extend(tail_rev);
            assembled = next_lines.join("\n");
        }
        if assembled.len() > max_bytes {
            assembled = truncate_utf8_suffix(&assembled, max_bytes).to_string();
        }
    }

    let kept_lines = assembled.lines().count();
    (assembled, kept_lines, elided_lines)
}

pub fn dehydrate_gis_or_conda_output(raw: &str, max_bytes: usize) -> (String, DehydrationSummary) {
    let raw_bytes = raw.len();
    let raw_lines = raw.lines().count();
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return (
            String::new(),
            DehydrationSummary {
                engine: "builtin_gis_dehydrator".to_string(),
                raw_bytes,
                kept_bytes: 0,
                raw_lines,
                kept_lines: 0,
                collapsed_progress_lines: 0,
                collapsed_duplicate_warnings: 0,
                head_tail_elided_lines: 0,
            },
        );
    }

    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && trimmed.len() <= max_bytes
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
    {
        return (
            trimmed.to_string(),
            DehydrationSummary {
                engine: "builtin_gis_dehydrator".to_string(),
                raw_bytes,
                kept_bytes: trimmed.len(),
                raw_lines,
                kept_lines: trimmed.lines().count(),
                collapsed_progress_lines: 0,
                collapsed_duplicate_warnings: 0,
                head_tail_elided_lines: 0,
            },
        );
    }

    let mut collapsed_progress_lines = 0usize;
    let mut collapsed_duplicate_warnings = 0usize;
    let mut normalized_lines: Vec<String> = Vec::new();

    for raw_line in raw.lines() {
        let cr_parts: Vec<&str> = raw_line
            .split('\r')
            .filter(|part| !part.trim().is_empty())
            .collect();
        let active = if cr_parts.is_empty() {
            String::new()
        } else {
            if cr_parts.len() > 1 {
                collapsed_progress_lines += cr_parts.len() - 1;
            }
            cr_parts[cr_parts.len() - 1].to_string()
        };
        normalized_lines.push(strip_ansi_sequences(&active));
    }

    let mut progress_filtered: Vec<String> = Vec::with_capacity(normalized_lines.len());
    let mut pending_progress: Option<String> = None;
    for line in normalized_lines {
        if is_progress_spam_line(&line) {
            if pending_progress.is_some() {
                collapsed_progress_lines += 1;
            }
            pending_progress = Some(line);
        } else {
            if let Some(last) = pending_progress.take() {
                progress_filtered.push(last);
            }
            progress_filtered.push(line);
        }
    }
    if let Some(last) = pending_progress.take() {
        progress_filtered.push(last);
    }

    let mut seen_warnings: HashSet<String> = HashSet::new();
    let mut folded_warnings_run = 0usize;
    let mut deduped: Vec<String> = Vec::with_capacity(progress_filtered.len());

    let flush_folded = |count: &mut usize, out: &mut Vec<String>| {
        if *count > 0 {
            out.push(format!("[... 已折叠 {} 条同类重复告警 ...]", *count));
            *count = 0;
        }
    };

    for line in progress_filtered {
        if let Some(sig) = warning_signature(&line) {
            if seen_warnings.insert(sig) {
                flush_folded(&mut folded_warnings_run, &mut deduped);
                deduped.push(line);
            } else {
                collapsed_duplicate_warnings += 1;
                folded_warnings_run += 1;
            }
        } else {
            flush_folded(&mut folded_warnings_run, &mut deduped);
            deduped.push(line);
        }
    }
    flush_folded(&mut folded_warnings_run, &mut deduped);

    let (final_text, kept_lines, head_tail_elided_lines) =
        apply_head_tail_budget(&deduped, max_bytes);

    let summary = DehydrationSummary {
        engine: "builtin_gis_dehydrator".to_string(),
        raw_bytes,
        kept_bytes: final_text.len(),
        raw_lines,
        kept_lines,
        collapsed_progress_lines,
        collapsed_duplicate_warnings,
        head_tail_elided_lines,
    };
    (final_text, summary)
}

pub fn apply_output_dehydration(
    raw: &str,
    max_bytes: usize,
    dehydrator: &LogDehydratorInfo,
    _app_data_dir: &Path,
) -> (String, DehydrationSummary) {
    let (text, mut summary) = dehydrate_gis_or_conda_output(raw, max_bytes);
    if dehydrator.mode == "auto" && dehydrator.shared_rtk.installed {
        summary.engine = "builtin_gis_dehydrator+shared_rtk".to_string();
    } else {
        summary.engine = "builtin_gis_dehydrator".to_string();
    }
    (text, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dehydrator_collapses_gis_progress_and_duplicate_warnings_preserving_tail_traceback() {
        let mut raw = String::new();
        raw.push_str("CMD: qgis_process run native:clip\n");
        raw.push_str("\x1b[32m0...10...20...\x1b[0m\n");
        raw.push_str("30...40...50...\n");
        raw.push_str("10%|█         |\r50%|█████     |\r100%|██████████|\n");
        for i in 0..20 {
            raw.push_str(&format!(
                "/opt/conda/lib/python3.11/site-packages/rasterio/crs.py:{}: FutureWarning: Deprecated CRS syntax\n",
                100 + i
            ));
        }
        for i in 0..120 {
            raw.push_str(&format!(
                "INFO processing feature chunk #{i} with normal diagnostic padding line text\n"
            ));
        }
        raw.push_str("Traceback (most recent call last):\n");
        raw.push_str("  File \"pipeline.py\", line 42, in run\n");
        raw.push_str("ValueError: CRS mismatch EPSG:4326 vs EPSG:32650\n");

        let (cleaned, summary) = dehydrate_gis_or_conda_output(&raw, 1800);
        assert!(
            cleaned.len() <= 1800,
            "kept_bytes={} exceeds budget",
            cleaned.len()
        );
        assert!(!cleaned.contains("\x1b["));
        assert!(cleaned.starts_with("CMD: qgis_process run native:clip"));
        assert!(cleaned.contains("100%|██████████|"));
        assert!(!cleaned.contains("10%|█"));
        assert!(summary.collapsed_progress_lines >= 3);
        assert_eq!(summary.collapsed_duplicate_warnings, 19);
        assert!(cleaned.contains("已折叠 19 条同类重复告警"));
        assert!(summary.head_tail_elided_lines > 0);
        assert!(cleaned.contains("ValueError: CRS mismatch EPSG:4326 vs EPSG:32650"));
    }

    #[test]
    fn dehydrator_passes_through_valid_json_without_corruption() {
        let json_payload = r#"{"ok":true,"gdal_version":"3.9.2","count":2,"algorithms":["native:clip","native:buffer"]}"#;
        let (out, summary) = dehydrate_gis_or_conda_output(json_payload, 16 * 1024);
        assert_eq!(out, json_payload);
        assert_eq!(summary.collapsed_progress_lines, 0);
        assert_eq!(summary.collapsed_duplicate_warnings, 0);
        assert_eq!(summary.head_tail_elided_lines, 0);
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid json preserved");
        assert_eq!(parsed["count"], 2);
    }

    #[test]
    fn discover_shared_rtk_verifies_custom_mock_binary_and_fails_closed_when_missing() {
        let temp_root = std::env::temp_dir().join(format!(
            "eostage-rtk-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_root).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mock_rtk = temp_root.join("mock_rtk");
            fs::write(&mock_rtk, "#!/bin/sh\necho \"rtk 0.24.1\"\n").unwrap();
            fs::set_permissions(&mock_rtk, fs::Permissions::from_mode(0o755)).unwrap();

            let info = discover_shared_rtk(Some(mock_rtk.to_str().unwrap()), &temp_root, true);
            assert!(info.installed, "{info:?}");
            assert_eq!(info.source, "custom_path");
            assert_eq!(info.version.as_deref(), Some("rtk 0.24.1"));

            let not_rtk = temp_root.join("fake_tool");
            fs::write(&not_rtk, "#!/bin/sh\necho \"GNU coreutils 9.0\"\n").unwrap();
            fs::set_permissions(&not_rtk, fs::Permissions::from_mode(0o755)).unwrap();
            let rejected = discover_shared_rtk(Some(not_rtk.to_str().unwrap()), &temp_root, true);
            assert!(!rejected.installed);
            assert_eq!(rejected.source, "missing");
        }

        let missing = discover_shared_rtk(
            Some(temp_root.join("definitely_missing_rtk").to_str().unwrap()),
            &temp_root,
            true,
        );
        assert!(!missing.installed);
        assert_eq!(missing.source, "missing");

        let _ = fs::remove_dir_all(&temp_root);
    }
}
