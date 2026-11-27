use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    LazyLock, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const APPROVAL_TTL_SECS: u64 = 30 * 60;
const MAX_APPROVAL_REGISTRY_ENTRIES: usize = 256;
const SCRIPT_TIMEOUT_SECS: u64 = 90;
static APPROVAL_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
struct ScriptApprovalEntry {
    inspected_script: String,
    script_sha256: String,
    created_at_epoch: u64,
    consumed: bool,
}

static APPROVAL_REGISTRY: LazyLock<Mutex<HashMap<String, ScriptApprovalEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScriptReviewFinding {
    pub severity: String, // "ERROR" | "WARN" | "INFO"
    pub code: String,
    pub line: usize,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptInspectionReport {
    pub ok: bool,
    pub blocked: bool,
    pub blocked_reasons: Vec<String>,
    pub scientific_warnings: Vec<String>,
    #[serde(default)]
    pub findings: Vec<ScriptReviewFinding>,
    pub script_sha256: String,
    pub line_count: usize,
    pub approval_id: Option<String>,
    pub env_ready: bool,
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn venv_python_path(app_data_dir: &Path) -> PathBuf {
    let venv_root = app_data_dir.join("gee_python_env");
    if cfg!(target_os = "windows") {
        venv_root.join("Scripts").join("python.exe")
    } else {
        venv_root.join("bin").join("python")
    }
}

pub fn check_gee_python_env_ready(app_data_dir: &Path) -> bool {
    let py = venv_python_path(app_data_dir);
    if !py.is_file() {
        return false;
    }
    let marker = app_data_dir.join("gee_python_env").join(".ee_ready_v1");
    marker.is_file()
}

pub fn get_script_env_status(app_data_dir: &Path) -> Value {
    let py = venv_python_path(app_data_dir);
    let ready = check_gee_python_env_ready(app_data_dir);
    json!({
        "ok": true,
        "dataset": "gee_script_env_status",
        "env_ready": ready,
        "venv_exists": py.is_file(),
        "isolated": true,
        "note_zh": if ready {
            "独立 GEE Python 运行环境已就绪"
        } else {
            "首次执行自定义 GEE 脚本时将自动初始化轻量独立环境"
        }
    })
}

fn build_masked_views(normalized: &str) -> (String, String) {
    let chars: Vec<char> = normalized.chars().collect();
    let len = chars.len();
    let mut no_comments = String::with_capacity(normalized.len());
    let mut code_only = String::with_capacity(normalized.len());
    let mut i = 0usize;

    while i < len {
        let ch = chars[i];
        // Comment outside string
        if ch == '#' {
            while i < len && chars[i] != '\n' {
                no_comments.push(' ');
                code_only.push(' ');
                i += 1;
            }
            continue;
        }
        // String literal (single, double, or triple-quoted)
        if ch == '\'' || ch == '"' {
            let quote = ch;
            let is_triple = i + 2 < len && chars[i + 1] == quote && chars[i + 2] == quote;
            if is_triple {
                for _ in 0..3 {
                    no_comments.push(quote);
                    code_only.push(' ');
                }
                i += 3;
                while i < len {
                    if i + 2 < len
                        && chars[i] == quote
                        && chars[i + 1] == quote
                        && chars[i + 2] == quote
                    {
                        for _ in 0..3 {
                            no_comments.push(quote);
                            code_only.push(' ');
                        }
                        i += 3;
                        break;
                    }
                    let c = chars[i];
                    if c == '\\' && i + 1 < len {
                        no_comments.push(c);
                        no_comments.push(chars[i + 1]);
                        code_only.push(' ');
                        code_only.push(if chars[i + 1] == '\n' { '\n' } else { ' ' });
                        i += 2;
                        continue;
                    }
                    no_comments.push(c);
                    code_only.push(if c == '\n' { '\n' } else { ' ' });
                    i += 1;
                }
            } else {
                no_comments.push(quote);
                code_only.push(' ');
                i += 1;
                while i < len {
                    let c = chars[i];
                    if c == '\n' {
                        no_comments.push('\n');
                        code_only.push('\n');
                        i += 1;
                        break;
                    }
                    if c == '\\' && i + 1 < len {
                        no_comments.push(c);
                        no_comments.push(chars[i + 1]);
                        code_only.push(' ');
                        code_only.push(if chars[i + 1] == '\n' { '\n' } else { ' ' });
                        i += 2;
                        continue;
                    }
                    no_comments.push(c);
                    code_only.push(' ');
                    i += 1;
                    if c == quote {
                        break;
                    }
                }
            }
            continue;
        }
        no_comments.push(ch);
        code_only.push(ch);
        i += 1;
    }

    (no_comments, code_only)
}

fn line_of_substr(text: &str, needle: &str) -> usize {
    match text.find(needle) {
        Some(byte_idx) => text[..byte_idx].bytes().filter(|&b| b == b'\n').count() + 1,
        None => 1,
    }
}

fn expand_leading_indent(line: &str) -> usize {
    let mut indent = 0usize;
    for ch in line.chars() {
        match ch {
            ' ' => indent += 1,
            '\t' => indent += 4,
            _ => break,
        }
    }
    indent
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn collect_mapped_function_names(code_only: &str) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    let chars: Vec<char> = code_only.chars().collect();
    let needle: Vec<char> = ".map(".chars().collect();
    let mut i = 0usize;
    while i + needle.len() <= chars.len() {
        if chars[i..i + needle.len()] == needle[..] {
            let mut j = i + needle.len();
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let start = j;
            while j < chars.len() && is_ident_char(chars[j]) {
                j += 1;
            }
            if j > start {
                let ident: String = chars[start..j].iter().collect();
                if ident != "lambda" {
                    names.insert(ident);
                }
            }
        }
        i += 1;
    }
    names
}

fn parse_def_name(trimmed_line: &str) -> Option<String> {
    let rest = trimmed_line
        .strip_prefix("def ")
        .or_else(|| trimmed_line.strip_prefix("async def "))?;
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|&c| is_ident_char(c))
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn is_loop_header(trimmed_line: &str) -> bool {
    (trimmed_line.starts_with("for ") || trimmed_line.starts_with("while "))
        && trimmed_line.ends_with(':')
}

fn contains_standalone_call(line: &str, func_name: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let needle: Vec<char> = format!("{func_name}(").chars().collect();
    if chars.len() < needle.len() {
        return false;
    }
    for i in 0..=chars.len() - needle.len() {
        if chars[i..i + needle.len()] == needle[..] {
            let prev_ok = i == 0 || (!is_ident_char(chars[i - 1]) && chars[i - 1] != '.');
            if prev_ok {
                return true;
            }
        }
    }
    false
}

fn extract_call_arg_spans(code_only: &str, call_pattern: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = code_only.chars().collect();
    let pat: Vec<char> = call_pattern.chars().collect();
    let mut results = Vec::new();
    if chars.len() < pat.len() {
        return results;
    }
    let mut line_no = 1usize;
    let mut i = 0usize;
    while i + pat.len() <= chars.len() {
        if chars[i..i + pat.len()] == pat[..] {
            let start_line = line_no;
            let open_paren = i + pat.len() - 1;
            if chars.get(open_paren) == Some(&'(') {
                let mut depth = 1usize;
                let mut j = open_paren + 1;
                let mut args_buf = String::new();
                while j < chars.len() && depth > 0 {
                    let c = chars[j];
                    if c == '(' {
                        depth += 1;
                        args_buf.push(c);
                    } else if c == ')' {
                        depth -= 1;
                        if depth > 0 {
                            args_buf.push(c);
                        }
                    } else {
                        args_buf.push(c);
                    }
                    j += 1;
                }
                results.push((start_line, args_buf));
            }
        }
        if chars[i] == '\n' {
            line_no += 1;
        }
        i += 1;
    }
    results
}

fn count_top_level_positional_args(args: &str) -> usize {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let mut depth = 0usize;
    let mut positional = 0usize;
    let mut cur = String::new();
    for ch in trimmed.chars() {
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(ch);
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            ',' if depth == 0 => {
                let part = cur.trim();
                if !part.is_empty() && !part.contains('=') {
                    positional += 1;
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let tail = cur.trim();
    if !tail.is_empty() && !tail.contains('=') {
        positional += 1;
    }
    positional
}

fn has_kwarg(args: &str, name: &str) -> bool {
    let chars: Vec<char> = args.chars().collect();
    let pat: Vec<char> = name.chars().collect();
    if chars.len() < pat.len() {
        return false;
    }
    for i in 0..=chars.len() - pat.len() {
        if chars[i..i + pat.len()] == pat[..] {
            let prev_ok = i == 0 || !is_ident_char(chars[i - 1]);
            if !prev_ok {
                continue;
            }
            let mut j = i + pat.len();
            while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
                j += 1;
            }
            if j < chars.len() && chars[j] == '=' && chars.get(j + 1) != Some(&'=') {
                return true;
            }
        }
    }
    false
}

/// Pure static security gate + remote-sensing scientific sanity lint.
/// Does NOT mutate `APPROVAL_REGISTRY` or issue an `approval_id`.
pub fn validate_gee_python_script_static(
    app_data_dir: &Path,
    raw_script: &str,
) -> ScriptInspectionReport {
    let normalized = raw_script.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = raw_script.trim();
    let script_sha = sha256_hex(trimmed);
    let line_count = if trimmed.is_empty() {
        0
    } else {
        normalized.lines().count()
    };

    let mut blocked_reasons = Vec::new();
    if trimmed.is_empty() {
        blocked_reasons.push("脚本内容不能为空".to_owned());
    }
    if trimmed.len() > 64 * 1024 {
        blocked_reasons.push("脚本长度超出 64 KB 安全上限".to_owned());
    }

    let lower = trimmed.to_ascii_lowercase();
    let compact_lower: String = lower.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let forbidden_patterns: &[(&str, &str)] = &[
        ("subprocess", "禁止调用 subprocess 子进程模块"),
        ("os.system", "禁止调用 os.system 执行系统命令"),
        ("os.popen", "禁止调用 os.popen 执行系统命令"),
        ("os.spawn", "禁止调用 os.spawn 创建进程"),
        ("os.exec", "禁止调用 os.exec 替换进程"),
        ("import pty", "禁止导入 pty 伪终端模块"),
        ("from pty", "禁止导入 pty 伪终端模块"),
        ("import socket", "禁止直接使用底层 socket 网络套接字"),
        ("from socket", "禁止直接使用底层 socket 网络套接字"),
        ("shutil.rmtree", "禁止调用 shutil.rmtree 删除目录树"),
        ("os.remove(", "禁止在云端算子脚本中删除本地文件"),
        ("os.unlink(", "禁止在云端算子脚本中删除本地文件"),
        ("ctypes", "禁止调用 ctypes 原生动态库接口"),
        ("multiprocessing", "禁止在受控脚本中派生多进程"),
        ("importlib", "禁止导入 importlib 动态加载模块"),
        ("__builtins__", "禁止访问 __builtins__ 内置命名空间"),
        ("builtins", "禁止导入或访问 builtins 内置模块"),
        ("getattr(", "禁止调用 getattr() 动态属性反射"),
        ("__import__", "禁止动态 __import__ 绕过模块审计"),
        ("eval(", "禁止调用动态 eval()"),
        ("exec(", "禁止调用动态 exec()"),
        ("compile(", "禁止调用动态 compile()"),
        ("webbrowser", "禁止自动拉起外部浏览器"),
    ];

    for (pat, reason) in forbidden_patterns {
        let matched = if pat.ends_with('(') {
            lower.contains(pat) || compact_lower.contains(pat)
        } else {
            lower.contains(pat)
        };
        if matched && !blocked_reasons.iter().any(|r| r == reason) {
            blocked_reasons.push((*reason).to_owned());
        }
    }

    let (code_no_comments, code_only) = build_masked_views(&normalized);
    let mapped_fns = collect_mapped_function_names(&code_only);

    let mut findings: Vec<ScriptReviewFinding> = Vec::new();
    let mut push_finding = |severity: &str, code: &str, line: usize, message: String| {
        if !findings.iter().any(|f| f.code == code && f.line == line) {
            findings.push(ScriptReviewFinding {
                severity: severity.to_owned(),
                code: code.to_owned(),
                line: line.max(1),
                message,
            });
        }
    };

    #[derive(Clone, Debug)]
    enum BlockKind {
        Def(String),
        Loop,
    }
    let mut block_stack: Vec<(BlockKind, usize)> = Vec::new();

    for (idx, line) in code_only.lines().enumerate() {
        let line_no = idx + 1;
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let indent = expand_leading_indent(line);
        while let Some((_, blk_indent)) = block_stack.last() {
            if *blk_indent >= indent {
                block_stack.pop();
            } else {
                break;
            }
        }
        if let Some(fn_name) = parse_def_name(t) {
            block_stack.push((BlockKind::Def(fn_name), indent));
        } else if is_loop_header(t) {
            block_stack.push((BlockKind::Loop, indent));
        }

        let in_mapped_fn = block_stack.iter().any(|(k, _)| match k {
            BlockKind::Def(name) => mapped_fns.contains(name),
            BlockKind::Loop => false,
        });
        let in_loop = block_stack
            .iter()
            .any(|(k, _)| matches!(k, BlockKind::Loop))
            || (t.contains(" for ") && t.contains(" in ") && (t.contains('[') || t.contains('(')));
        let inline_map_lambda = t.contains(".map(") && t.contains("lambda");

        if t.contains(".getInfo(") {
            if in_mapped_fn || inline_map_lambda {
                push_finding(
                    "ERROR",
                    "mapped-getinfo",
                    line_no,
                    "禁止在 collection.map() 映射函数内部调用 getInfo()，应保持服务端计算图直到最终统计或导出。".to_owned(),
                );
            } else if in_loop {
                push_finding(
                    "ERROR",
                    "loop-getinfo",
                    line_no,
                    "避免在 Python 循环内逐项调用 getInfo()，易触发并发限流与超时；请改用服务端 reduceRegion 或批量导出。".to_owned(),
                );
            } else {
                push_finding(
                    "INFO",
                    "getinfo-diagnostic",
                    line_no,
                    "检测到顶层 getInfo() 调用：仅用于提取少量统计摘要，切勿用于拉取大尺寸像元数组。".to_owned(),
                );
            }
        }

        if in_mapped_fn && contains_standalone_call(t, "print") {
            push_finding(
                "WARN",
                "mapped-print",
                line_no,
                "避免在 map() 映射函数内部调用 print()，服务端映射执行不会按预期逐像元输出日志。"
                    .to_owned(),
            );
        }
        if t.contains(".toList(") {
            push_finding(
                "WARN",
                "tolist-usage",
                line_no,
                "慎用 collection.toList()（易将整套集合拉入内存导致超限），优先使用 filter、limit 或服务端 Reducer。".to_owned(),
            );
        }
        if t.contains("ee.Algorithms.If(") || t.contains(".Algorithms.If(") {
            push_finding(
                "WARN",
                "algorithms-if",
                line_no,
                "慎用 ee.Algorithms.If()，它会同时构图两个分支；优先使用 filter、掩膜条件或分集合处理。".to_owned(),
            );
        }
        if t.contains(".reproject(") {
            push_finding(
                "WARN",
                "reproject-caution",
                line_no,
                "慎用强制 .reproject()，它会打破金字塔多尺度优化并增加 EECU 耗时；通常应在 reduceRegion 或 Export 中指定 crs 与 scale。".to_owned(),
            );
        }
        if t.contains("ee.Authenticate(") {
            push_finding(
                "INFO",
                "authenticate-present",
                line_no,
                "宿主已自动注入短效授权会话，受控脚本中无需调用 ee.Authenticate()。".to_owned(),
            );
        }
    }

    for (line_no, args) in extract_call_arg_spans(&code_only, ".reduceRegion(") {
        let pos_count = count_top_level_positional_args(&args);
        if !has_kwarg(&args, "geometry") && pos_count < 2 {
            push_finding(
                "WARN",
                "reduceRegion-missing-geometry",
                line_no,
                "调用 reduceRegion 时缺少 geometry 参数，请显式指定研究区范围。".to_owned(),
            );
        }
        if !has_kwarg(&args, "scale") && pos_count < 3 {
            push_finding(
                "WARN",
                "reduceRegion-missing-scale",
                line_no,
                "调用 reduceRegion 时未显式指定 scale（米），默认金字塔尺度可能导致面积或统计值失真。".to_owned(),
            );
        }
        if !has_kwarg(&args, "maxPixels") && !has_kwarg(&args, "bestEffort") {
            push_finding(
                "WARN",
                "reduceRegion-missing-pixel-limit",
                line_no,
                "调用 reduceRegion 时未指定 maxPixels（如 1e9）或 bestEffort=True，较大研究区可能触发 Too many pixels 报错。".to_owned(),
            );
        }
    }

    for (line_no, args) in extract_call_arg_spans(&code_only, ".reduceRegions(") {
        let pos_count = count_top_level_positional_args(&args);
        if !has_kwarg(&args, "collection") && pos_count < 1 {
            push_finding(
                "WARN",
                "reduceRegions-missing-collection",
                line_no,
                "调用 reduceRegions 时缺少 collection 要素集合参数。".to_owned(),
            );
        }
        if !has_kwarg(&args, "scale") && pos_count < 3 {
            push_finding(
                "WARN",
                "reduceRegions-missing-scale",
                line_no,
                "调用 reduceRegions 时未显式指定 scale（米），可能影响分区统计空间一致性。"
                    .to_owned(),
            );
        }
    }

    for pat in [
        "Export.image.toDrive(",
        "Export.image.toCloudStorage(",
        "Export.image.toAsset(",
    ] {
        for (line_no, args) in extract_call_arg_spans(&code_only, pat) {
            if !has_kwarg(&args, "region") {
                push_finding(
                    "WARN",
                    "export-image-missing-region",
                    line_no,
                    "影像导出任务未显式指定 region，大区域或未裁剪影像可能触发意外超限。"
                        .to_owned(),
                );
            }
            if !has_kwarg(&args, "scale") {
                push_finding(
                    "WARN",
                    "export-image-missing-scale",
                    line_no,
                    "影像导出任务未显式指定 scale（米），请明确科研输出分辨率。".to_owned(),
                );
            }
            if !has_kwarg(&args, "maxPixels") {
                push_finding(
                    "WARN",
                    "export-image-missing-maxPixels",
                    line_no,
                    "影像导出任务建议显式设置 maxPixels（如 1e10）以防默认像元上限中断任务。"
                        .to_owned(),
                );
            }
        }
    }

    for pat in [
        "Export.table.toDrive(",
        "Export.table.toCloudStorage(",
        "Export.table.toAsset(",
    ] {
        for (line_no, args) in extract_call_arg_spans(&code_only, pat) {
            if !has_kwarg(&args, "collection") && count_top_level_positional_args(&args) < 1 {
                push_finding(
                    "WARN",
                    "export-table-missing-collection",
                    line_no,
                    "表格导出任务缺少 collection 参数。".to_owned(),
                );
            }
        }
    }

    if code_only.contains("Export.") && !code_only.contains(".start(") {
        push_finding(
            "INFO",
            "export-not-started",
            line_of_substr(&code_only, "Export."),
            "检测到 Export 任务配置但未调用 .start()；仅在人工确认导出参数后再启动任务时适用。"
                .to_owned(),
        );
    }

    // Dataset-specific scientific checks on comment-stripped source
    if code_no_comments.contains("COPERNICUS/S2_SR")
        || code_no_comments.contains("S2_SR_HARMONIZED")
    {
        let s2_line = line_of_substr(
            &code_no_comments,
            if code_no_comments.contains("S2_SR_HARMONIZED") {
                "S2_SR_HARMONIZED"
            } else {
                "COPERNICUS/S2_SR"
            },
        );
        if !code_no_comments.contains("0.0001") && !code_no_comments.contains("divide(10000") {
            push_finding(
                "WARN",
                "s2-reflectance-scale",
                s2_line,
                "检测到 Sentinel-2 地表反射率数据集，但未发现 ×0.0001 或 divide(10000) 定标缩放；若仅计算归一化指数可忽略，若做阈值或物理反演请确认量纲。".to_owned(),
            );
        }
        let has_pixel_mask = code_no_comments.contains("updateMask")
            || code_no_comments.contains("CLOUD_SCORE_PLUS")
            || code_no_comments.contains("S2_CLOUD_PROBABILITY")
            || code_no_comments.contains("SCL")
            || code_no_comments.contains("QA60")
            || code_no_comments.contains("csPlus");
        if !has_pixel_mask {
            push_finding(
                "WARN",
                "s2-no-pixel-mask",
                s2_line,
                "Sentinel-2 场景云量筛选不等价于像元级去云；建议关联 Cloud Score+、SCL 或云概率波段并调用 updateMask()。".to_owned(),
            );
        }
        if code_no_comments.contains("QA60") {
            push_finding(
                "WARN",
                "s2-qa60-gap",
                line_of_substr(&code_no_comments, "QA60"),
                "Sentinel-2 QA60 波段在 2022-01-25 至 2024-02-28 存在历史生成差异，跨多年合成建议改用 Cloud Score+ 或 SCL。".to_owned(),
            );
        }
    }

    if code_no_comments.contains("C02/T1_L2") {
        let l_line = line_of_substr(&code_no_comments, "C02/T1_L2");
        if code_no_comments.contains("SR_B")
            && !code_no_comments.contains("0.0000275")
            && !code_no_comments.contains("2.75e-05")
            && !code_no_comments.contains("2.75e-5")
        {
            push_finding(
                "WARN",
                "landsat-c2-missing-scale",
                l_line,
                "检测到 Landsat C2 L2 地表反射率波段 (SR_B*)，官方物理缩放公式为 DN × 0.0000275 - 0.2（注意负偏置对直接计算 NDVI 也有影响）。".to_owned(),
            );
        }
        if code_no_comments.contains("ST_B10") && !code_no_comments.contains("0.00341802") {
            push_finding(
                "WARN",
                "landsat-c2-missing-thermal-scale",
                line_of_substr(&code_no_comments, "ST_B10"),
                "检测到 Landsat C2 L2 热红外地表温度波段 (ST_B10)，官方开尔文定标公式为 DN × 0.00341802 + 149.0。".to_owned(),
            );
        }
        if !code_no_comments.contains("QA_PIXEL") {
            push_finding(
                "WARN",
                "landsat-c2-missing-qa",
                l_line,
                "检测到 Landsat Collection 2 L2 数据集但未读取 QA_PIXEL 位掩膜，建议剔除填充、膨胀云、卷云、云、云影、雪及 QA_RADSAT 饱和像元。".to_owned(),
            );
        }
    }

    if code_no_comments.contains("MODIS/061/MOD13Q1") {
        let m_line = line_of_substr(&code_no_comments, "MODIS/061/MOD13Q1");
        if !code_no_comments.contains("0.0001") {
            push_finding(
                "WARN",
                "modis-vi-missing-scale",
                m_line,
                "MOD13Q1 的 NDVI 与 EVI 原始数值按 0.0001 缩放，统计或导出前请乘以 0.0001。"
                    .to_owned(),
            );
        }
        if !code_no_comments.contains("DetailedQA") && !code_no_comments.contains("SummaryQA") {
            push_finding(
                "INFO",
                "modis-vi-no-qa",
                m_line,
                "MOD13Q1 时序分析建议结合 DetailedQA 或 SummaryQA 剔除云雪干扰像元。".to_owned(),
            );
        }
    }

    if code_no_comments.contains("GOOGLE/DYNAMICWORLD/V1")
        && (code_no_comments.contains("select('label')")
            || code_no_comments.contains("select(\"label\")"))
    {
        push_finding(
            "WARN",
            "dynamicworld-label-only",
            line_of_substr(&code_no_comments, "GOOGLE/DYNAMICWORLD/V1"),
            "Dynamic World 的 label 仅为最高概率类别编码，建议同时检查各土地覆盖概率波段或置信度阈值。".to_owned(),
        );
    }

    if (code_no_comments.contains("ESA/WorldCover")
        || code_no_comments.contains("GOOGLE/DYNAMICWORLD/V1"))
        && code_only.contains(".mean(")
    {
        push_finding(
            "ERROR",
            "categorical-mean",
            line_of_substr(&code_only, ".mean("),
            "地物分类产品（WorldCover 或 Dynamic World）的类别编码属于离散分类值，严禁调用 .mean() 求平均；请改用 mode()、分组统计或 pixelArea() 面积统计。".to_owned(),
        );
    }

    if code_only.contains("ImageCollection(")
        && !code_only.contains("filterDate")
        && !code_only.contains("filterBounds")
        && !code_only.contains("filter(")
    {
        push_finding(
            "WARN",
            "unfiltered-collection",
            line_of_substr(&code_only, "ImageCollection("),
            "检测到 ee.ImageCollection 未添加时空过滤（filterDate 与 filterBounds），可能消耗过多 GEE 计算额度 (EECU)。".to_owned(),
        );
    }

    findings.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.code.cmp(&b.code)));

    let mut scientific_warnings: Vec<String> = Vec::new();
    for f in &findings {
        if f.severity == "ERROR" || f.severity == "WARN" {
            let line_msg = format!("[{} L{}] {}", f.severity, f.line, f.message);
            if !scientific_warnings.contains(&line_msg) {
                scientific_warnings.push(line_msg);
            }
        }
    }

    let blocked = !blocked_reasons.is_empty();
    ScriptInspectionReport {
        ok: !blocked,
        blocked,
        blocked_reasons,
        scientific_warnings,
        findings,
        script_sha256: script_sha,
        line_count,
        approval_id: None,
        env_ready: check_gee_python_env_ready(app_data_dir),
    }
}

/// Static security gate + remote-sensing scientific sanity lint.
/// Generates a one-time `approval_id` bound to `sha256(script.trim())` ONLY if `blocked == false`.
pub fn inspect_gee_python_script(app_data_dir: &Path, raw_script: &str) -> ScriptInspectionReport {
    let mut report = validate_gee_python_script_static(app_data_dir, raw_script);
    if !report.blocked {
        let trimmed = raw_script.trim().to_owned();
        let now = now_epoch_secs();
        let seq = APPROVAL_COUNTER.fetch_add(1, Ordering::Relaxed);
        let raw_id = sha256_hex(&format!(
            "{}:{now}:{seq}:{}",
            report.script_sha256,
            std::process::id()
        ));
        let token_id = format!("gee-appr-{}", &raw_id[..16]);
        if let Ok(mut guard) = APPROVAL_REGISTRY.lock() {
            // Garbage-collect expired entries
            guard.retain(|_, v| now.saturating_sub(v.created_at_epoch) <= APPROVAL_TTL_SECS);
            if guard.len() >= MAX_APPROVAL_REGISTRY_ENTRIES {
                guard.retain(|_, v| !v.consumed);
            }
            while guard.len() >= MAX_APPROVAL_REGISTRY_ENTRIES {
                let Some(oldest_key) = guard
                    .iter()
                    .min_by_key(|(_, v)| v.created_at_epoch)
                    .map(|(k, _)| k.clone())
                else {
                    break;
                };
                guard.remove(&oldest_key);
            }
            guard.insert(
                token_id.clone(),
                ScriptApprovalEntry {
                    inspected_script: trimmed,
                    script_sha256: report.script_sha256.clone(),
                    created_at_epoch: now,
                    consumed: false,
                },
            );
        }
        report.approval_id = Some(token_id);
    }
    report
}

/// Read-only local desktop IPC query for displaying the exact uncorrupted inspected script
/// on the human confirmation card without consuming the one-time approval token.
/// Never exposed as an Agent LLM tool, preserving `AGENT-REG-002` outbound trace scrubbing.
pub fn preview_approved_gee_script(approval_id: &str) -> Result<Value, String> {
    let id_trimmed = approval_id.trim();
    if id_trimmed.is_empty() {
        return Err("GEE_SCRIPT_APPROVAL_REQUIRED: 缺少脚本审批编号".to_owned());
    }
    let now = now_epoch_secs();
    let mut guard = APPROVAL_REGISTRY
        .lock()
        .map_err(|_| "GEE_APPROVAL_LOCK_POISONED".to_owned())?;
    let Some(entry) = guard.get(id_trimmed) else {
        return Err("GEE_SCRIPT_APPROVAL_INVALID: 未找到有效的脚本审批记录或审批已过期".to_owned());
    };
    if now.saturating_sub(entry.created_at_epoch) > APPROVAL_TTL_SECS {
        guard.remove(id_trimmed);
        return Err(
            "GEE_SCRIPT_APPROVAL_EXPIRED: 脚本确认已超过 30 分钟有效期，请重新确认".to_owned(),
        );
    }
    Ok(json!({
        "ok": true,
        "dataset": "gee_script_preview",
        "approval_id": id_trimmed,
        "script": entry.inspected_script,
        "script_sha256": entry.script_sha256,
    }))
}

fn verify_and_consume_approval(approval_id: &str, raw_script: &str) -> Result<String, String> {
    let id_trimmed = approval_id.trim();
    if id_trimmed.is_empty() {
        return Err(
            "GEE_SCRIPT_APPROVAL_REQUIRED: 自定义 GEE 脚本必须先经安全检查并由用户点击确认后方可执行"
                .to_owned(),
        );
    }
    let candidate_trimmed = raw_script.trim();
    let candidate_sha = sha256_hex(candidate_trimmed);
    let now = now_epoch_secs();
    let mut guard = APPROVAL_REGISTRY
        .lock()
        .map_err(|_| "GEE_APPROVAL_LOCK_POISONED".to_owned())?;
    let Some(entry) = guard.get_mut(id_trimmed) else {
        return Err(
            "GEE_SCRIPT_APPROVAL_INVALID: 未找到有效的脚本审批记录或审批已过期，请重新审阅并确认脚本"
                .to_owned(),
        );
    };
    if entry.consumed {
        return Err(
            "GEE_SCRIPT_APPROVAL_CONSUMED: 该审批编号已使用过一次，重新运行请再次确认".to_owned(),
        );
    }
    if now.saturating_sub(entry.created_at_epoch) > APPROVAL_TTL_SECS {
        guard.remove(id_trimmed);
        return Err(
            "GEE_SCRIPT_APPROVAL_EXPIRED: 脚本确认已超过 30 分钟有效期，请重新确认".to_owned(),
        );
    }
    let exact_match = entry.script_sha256 == candidate_sha;
    let outbound_redacted_match = !exact_match && {
        let scrubbed = insar_agent::history::outbound_text(&entry.inspected_script);
        candidate_trimmed == scrubbed.trim()
    };
    if !exact_match && !outbound_redacted_match {
        return Err(
            "GEE_SCRIPT_HASH_MISMATCH: 脚本内容在确认后发生了变动，已拦截执行，请重新审阅确认"
                .to_owned(),
        );
    }
    entry.consumed = true;
    Ok(entry.inspected_script.clone())
}

fn find_host_python3() -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(custom) = std::env::var("INSAR_PYTHON_EXE") {
        if !custom.trim().is_empty() {
            candidates.push(PathBuf::from(custom));
        }
    }
    if cfg!(target_os = "windows") {
        candidates.push(PathBuf::from("python"));
        candidates.push(PathBuf::from("py"));
    } else {
        candidates.push(PathBuf::from("/opt/homebrew/bin/python3"));
        candidates.push(PathBuf::from("/usr/local/bin/python3"));
        candidates.push(PathBuf::from("/usr/bin/python3"));
        candidates.push(PathBuf::from("python3"));
    }

    for cand in candidates {
        let ok = Command::new(&cand)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Ok(cand);
        }
    }
    Err("GEE_PYTHON_NOT_FOUND: 未在系统中检测到可用的 Python 3，无法初始化自定义脚本环境（内置 GEE 云算子无需 Python 仍可正常使用）".to_owned())
}

pub fn ensure_gee_python_env(
    app_data_dir: &Path,
    proxy_url: Option<&str>,
) -> Result<PathBuf, String> {
    let venv_dir = app_data_dir.join("gee_python_env");
    let py = venv_python_path(app_data_dir);
    let marker = venv_dir.join(".ee_ready_v1");
    if py.is_file() && marker.is_file() {
        return Ok(py);
    }

    let base_py = find_host_python3()?;
    if !py.is_file() {
        fs::create_dir_all(app_data_dir).map_err(|e| format!("GEE_VENV_DIR_FAILED: {e}"))?;
        let status = Command::new(&base_py)
            .arg("-m")
            .arg("venv")
            .arg(&venv_dir)
            .status()
            .map_err(|e| format!("GEE_VENV_CREATE_FAILED: {e}"))?;
        if !status.success() {
            return Err("GEE_VENV_CREATE_NONZERO: 创建独立 Python 虚拟环境失败".to_owned());
        }
    }

    let mut pip_cmd = Command::new(&py);
    pip_cmd.args([
        "-m",
        "pip",
        "install",
        "--disable-pip-version-check",
        "--quiet",
        "earthengine-api",
        "requests",
    ]);
    if let Some(px) = proxy_url.filter(|s| !s.trim().is_empty()) {
        pip_cmd.env("HTTPS_PROXY", px);
        pip_cmd.env("HTTP_PROXY", px);
    }
    let out = pip_cmd
        .output()
        .map_err(|e| format!("GEE_PIP_INSTALL_FAILED: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "GEE_PIP_INSTALL_ERROR: 安装 earthengine-api 失败: {}",
            stderr.chars().take(400).collect::<String>()
        ));
    }

    let _ = fs::write(&marker, b"ok\n");
    Ok(py)
}

/// Execute an approved GEE Python script in an isolated subprocess with `env_clear()`,
/// passing ONLY the short-lived OAuth2 access token (never the RSA private key).
pub fn execute_approved_gee_script(
    app_data_dir: &Path,
    approval_id: &str,
    script: &str,
    project_id: &str,
    access_token: &str,
    aoi_bbox: [f64; 4],
    proxy_url: Option<&str>,
) -> Result<Value, String> {
    // 1. Re-run pure static inspection (without inserting an unconsumed token) so tampered payloads never slip past
    let candidate_report = validate_gee_python_script_static(app_data_dir, script);
    if candidate_report.blocked {
        return Err(format!(
            "GEE_SCRIPT_BLOCKED: {}",
            candidate_report.blocked_reasons.join("；")
        ));
    }
    let canonical_script = verify_and_consume_approval(approval_id, script)?;
    let report = validate_gee_python_script_static(app_data_dir, &canonical_script);
    if report.blocked {
        return Err(format!(
            "GEE_SCRIPT_BLOCKED: {}",
            report.blocked_reasons.join("；")
        ));
    }

    // 2. Ensure venv is bootstrapped
    let py = ensure_gee_python_env(app_data_dir, proxy_url)?;

    // 3. Prepare isolated working directory
    let run_nonce = format!("run-{}-{}", now_epoch_secs(), &sha256_hex(approval_id)[..8]);
    let work_dir = app_data_dir.join("gee_script_runs").join(&run_nonce);
    fs::create_dir_all(&work_dir).map_err(|e| format!("GEE_SCRIPT_WORKDIR_FAILED: {e}"))?;

    let user_script_path = work_dir.join("user_script.py");
    let runner_script_path = work_dir.join("_eostage_runner.py");
    let result_json_path = work_dir.join("_result.json");
    let preview_png_path = work_dir.join("preview.png");

    fs::write(&user_script_path, canonical_script.trim())
        .map_err(|e| format!("GEE_WRITE_USER_SCRIPT_FAILED: {e}"))?;

    let runner_code = r#"import json, os, pathlib, sys, traceback
import requests
import ee
from google.oauth2.credentials import Credentials

_RESULT_JSON_PATH = os.environ.pop("EOSTAGE_GEE_RESULT_JSON", "_result.json")

def main():
    token = os.environ.pop("EOSTAGE_GEE_ACCESS_TOKEN", None)
    project_id = os.environ.pop("EOSTAGE_GEE_PROJECT_ID", None)
    bbox = json.loads(os.environ.pop("EOSTAGE_GEE_AOI_BBOX", "[117.2,31.8,117.3,31.9]"))
    out_png = os.environ.pop("EOSTAGE_GEE_PREVIEW_PNG", "")
    user_py = os.environ.pop("EOSTAGE_GEE_USER_SCRIPT", "user_script.py")
    _out_dir = pathlib.Path(out_png).parent if out_png else pathlib.Path.cwd()
    preview_png_path_str = str(_out_dir / "preview.png")

    creds = Credentials(token=token)
    ee.Initialize(credentials=creds, project=project_id)
    os.environ.pop("EOSTAGE_GEE_ACCESS_TOKEN", None)
    os.environ.pop("EOSTAGE_GEE_PROJECT_ID", None)
    del token, creds

    aoi = ee.Geometry.Rectangle(bbox)
    ns = {
        "__name__": "__main__",
        "ee": ee,
        "AOI_BBOX": bbox,
        "BBOX": bbox,
        "AOI": aoi,
        "PROJECT_ID": project_id,
        "PREVIEW_PNG_PATH": preview_png_path_str,
        "RESULT": None,
        "PREVIEW_IMAGE": None,
        "VIS_PARAMS": None,
    }
    with open(user_py, "r", encoding="utf-8") as f:
        code = f.read()

    profile_ids = []
    def _prof_hook(pid):
        if isinstance(pid, str) and pid.strip():
            profile_ids.append(pid.strip())

    def _run_body():
        exec(compile(code, "user_gee_script.py", "exec"), ns)
        res_local = ns.get("RESULT")
        if hasattr(res_local, "getInfo"):
            res_local = res_local.getInfo()

        saved_local = False
        preview_img = ns.get("PREVIEW_IMAGE")
        if preview_img is not None and hasattr(preview_img, "getThumbURL"):
            vis = dict(ns.get("VIS_PARAMS") or {})
            vis.setdefault("region", aoi)
            vis.setdefault("dimensions", 512)
            vis.setdefault("format", "png")
            thumb_url = preview_img.getThumbURL(vis)
            r = requests.get(thumb_url, timeout=45)
            if r.ok and r.content[:8] == b"\x89PNG\r\n\x1a\n":
                with open(preview_png_path_str, "wb") as pf:
                    pf.write(r.content)
                saved_local = True
        elif os.path.isfile(preview_png_path_str) and os.path.getsize(preview_png_path_str) > 0:
            saved_local = True
        return res_local, saved_local

    ee_data = getattr(ee, "data", None)
    prof_fn = getattr(ee_data, "profiling", None) if ee_data is not None else None
    if callable(prof_fn):
        with prof_fn(_prof_hook):
            res, preview_saved = _run_body()
    else:
        res, preview_saved = _run_body()

    with open(_RESULT_JSON_PATH, "w", encoding="utf-8") as out_f:
        json.dump(
            {"ok": True, "result": res, "preview_saved": preview_saved, "profile_ids": profile_ids},
            out_f,
            ensure_ascii=False,
        )

if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        with open(_RESULT_JSON_PATH, "w", encoding="utf-8") as f:
            json.dump({"ok": False, "error": str(exc), "traceback": traceback.format_exc()[-1200:]}, f, ensure_ascii=False)
        sys.exit(1)
"#;
    fs::write(&runner_script_path, runner_code)
        .map_err(|e| format!("GEE_WRITE_RUNNER_FAILED: {e}"))?;

    // 4. Spawn isolated child process with env_clear()
    let mut cmd = Command::new(&py);
    cmd.env_clear();
    cmd.current_dir(&work_dir);
    cmd.arg(&runner_script_path);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let venv_bin = py.parent().unwrap_or_else(|| Path::new(""));
    let minimal_path = if cfg!(target_os = "windows") {
        format!(r"{};C:\Windows\System32;C:\Windows", venv_bin.display())
    } else {
        format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", venv_bin.display())
    };
    cmd.env("PATH", minimal_path);
    cmd.env("HOME", &work_dir);
    cmd.env("USERPROFILE", &work_dir);
    cmd.env("TMPDIR", &work_dir);
    cmd.env("TEMP", &work_dir);
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("EOSTAGE_GEE_ACCESS_TOKEN", access_token);
    cmd.env("EOSTAGE_GEE_PROJECT_ID", project_id);
    cmd.env(
        "EOSTAGE_GEE_AOI_BBOX",
        serde_json::to_string(&aoi_bbox).unwrap_or_else(|_| "[117.2,31.8,117.3,31.9]".into()),
    );
    cmd.env("EOSTAGE_GEE_RESULT_JSON", &result_json_path);
    cmd.env("EOSTAGE_GEE_PREVIEW_PNG", &preview_png_path);
    cmd.env("EOSTAGE_GEE_USER_SCRIPT", &user_script_path);
    if let Some(px) = proxy_url.filter(|s| !s.trim().is_empty()) {
        cmd.env("HTTPS_PROXY", px);
        cmd.env("HTTP_PROXY", px);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("GEE_SCRIPT_SPAWN_FAILED: {e}"))?;
    let start = Instant::now();
    let timeout = Duration::from_secs(SCRIPT_TIMEOUT_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "GEE_SCRIPT_TIMEOUT: 脚本运行超过 {} 秒安全时限已终止",
                        SCRIPT_TIMEOUT_SECS
                    ));
                }
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(e) => return Err(format!("GEE_SCRIPT_WAIT_ERROR: {e}")),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("GEE_SCRIPT_OUTPUT_ERROR: {e}"))?;
    let stdout_str = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr_str = String::from_utf8_lossy(&output.stderr).trim().to_owned();

    let parsed_res: Value = if result_json_path.is_file() {
        fs::read_to_string(&result_json_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({"ok": false, "error": "结果 JSON 解析失败"}))
    } else {
        json!({
            "ok": output.status.success(),
            "error": if stderr_str.is_empty() { "脚本未返回结构化结果" } else { &stderr_str }
        })
    };

    if parsed_res["ok"].as_bool() != Some(true) {
        let err_msg = parsed_res["error"]
            .as_str()
            .unwrap_or("自定义 GEE 脚本执行失败");
        return Err(format!("GEE_SCRIPT_RUNTIME_ERROR: {err_msg}"));
    }

    let script_wall_secs = start.elapsed().as_secs_f64();
    let script_profile_ids: Vec<String> = parsed_res
        .get("profile_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    super::quota::record_script_execution(app_data_dir, script_wall_secs, &script_profile_ids);

    let preview_bytes = if preview_png_path.is_file() {
        fs::read(&preview_png_path).ok().filter(|b| !b.is_empty())
    } else {
        None
    };

    // Clean up user script file if desired, keeping preview.png if generated
    let _ = fs::remove_file(&runner_script_path);

    Ok(json!({
        "ok": true,
        "dataset": "gee_script_execute",
        "result": parsed_res["result"],
        "stdout": stdout_str.chars().take(2000).collect::<String>(),
        "preview_png_path": preview_bytes.as_ref().map(|_| preview_png_path.to_string_lossy().to_string()),
        "preview_png_bytes_len": preview_bytes.as_ref().map(|b| b.len()).unwrap_or(0),
        "scientific_warnings": report.scientific_warnings,
        "findings": report.findings,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_validator_blocks_reflection_and_does_not_leak_approval_tokens() {
        let tmp = std::env::temp_dir().join(format!(
            "eostage-gee-script-unit-{}-{}",
            std::process::id(),
            now_epoch_secs()
        ));
        let _ = fs::create_dir_all(&tmp);

        for snippet in [
            "import importlib\nm = importlib.import_module('os')",
            "x = __builtins__['open']('foo')",
            "import builtins\nbuiltins.open('foo')",
            "fn = getattr(os, 'sys' + 'tem')",
            "fn = getattr (os, 'popen')",
        ] {
            let rep = validate_gee_python_script_static(&tmp, snippet);
            assert!(rep.blocked, "snippet must be blocked: {snippet}");
            assert!(rep.approval_id.is_none());
        }

        let clean_script = "col = ee.ImageCollection('COPERNICUS/S2_SR_HARMONIZED').filterBounds(AOI)\nRESULT = {'bbox': BBOX}";
        let before_unconsumed = APPROVAL_REGISTRY
            .lock()
            .unwrap()
            .values()
            .filter(|e| !e.consumed && e.script_sha256 == sha256_hex(clean_script))
            .count();

        // Pure static validation issues zero tokens
        let pure_rep = validate_gee_python_script_static(&tmp, clean_script);
        assert!(!pure_rep.blocked);
        assert!(pure_rep.approval_id.is_none());

        // Inspect issues exactly 1 token; re-validating + consuming leaves 0 unconsumed tokens for this script
        let issued = inspect_gee_python_script(&tmp, clean_script);
        let appr_id = issued.approval_id.expect("approval_id issued");
        let recheck = validate_gee_python_script_static(&tmp, clean_script);
        assert!(!recheck.blocked);
        verify_and_consume_approval(&appr_id, clean_script).expect("consume once");
        assert!(verify_and_consume_approval(&appr_id, clean_script).is_err());

        let after_unconsumed = APPROVAL_REGISTRY
            .lock()
            .unwrap()
            .values()
            .filter(|e| !e.consumed && e.script_sha256 == sha256_hex(clean_script))
            .count();
        assert_eq!(after_unconsumed, before_unconsumed);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn mock_venv_runner_pops_token_injects_bbox_and_picks_up_direct_preview_png() {
        let Ok(host_py) = find_host_python3() else {
            return;
        };
        let tmp = std::env::temp_dir().join(format!(
            "eostage-gee-runner-sandbox-{}-{}",
            std::process::id(),
            APPROVAL_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&tmp);
        let venv_dir = tmp.join("gee_python_env");
        let bin_dir = if cfg!(target_os = "windows") {
            venv_dir.join("Scripts")
        } else {
            venv_dir.join("bin")
        };
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(venv_dir.join(".ee_ready_v1"), b"ok\n").unwrap();

        // Create a tiny wrapper python + stub `ee`, `requests`, `google.oauth2.credentials` in PYTHONPATH-free site stub
        // Note: runner sets current_dir(&work_dir), wait—stub modules in venv site or wrapper script!
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let stub_pkg = venv_dir.join("stub_pkgs");
            fs::create_dir_all(stub_pkg.join("google").join("oauth2")).unwrap();
            fs::write(stub_pkg.join("google").join("__init__.py"), b"").unwrap();
            fs::write(
                stub_pkg.join("google").join("oauth2").join("__init__.py"),
                b"",
            )
            .unwrap();
            fs::write(
                stub_pkg.join("google").join("oauth2").join("credentials.py"),
                b"class Credentials:\n    def __init__(self, token=None):\n        self.token = token\n",
            )
            .unwrap();
            fs::write(stub_pkg.join("requests.py"), b"").unwrap();
            fs::write(
                stub_pkg.join("ee.py"),
                b"class _Geom:\n    @staticmethod\n    def Rectangle(b):\n        return {'rect': b}\nGeometry = _Geom\ndef Initialize(credentials=None, project=None):\n    assert credentials.token == 'secret-bearer-xyz'\n    assert project == 'proj-unit'\n",
            )
            .unwrap();

            let wrapper = bin_dir.join("python");
            let script_sh = format!(
                "#!/bin/sh\nPYTHONPATH=\"{}\" exec \"{}\" \"$@\"\n",
                stub_pkg.display(),
                host_py.display()
            );
            fs::write(&wrapper, script_sh.as_bytes()).unwrap();
            let mut perms = fs::metadata(&wrapper).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&wrapper, perms).unwrap();

            let user_code = "\
import os
assert 'EOSTAGE_GEE_ACCESS_TOKEN' not in os.environ, 'token leaked in env!'
assert 'EOSTAGE_GEE_PROJECT_ID' not in os.environ, 'project_id leaked in env!'
assert BBOX == AOI_BBOX
with open(PREVIEW_PNG_PATH, 'wb') as f:
    f.write(b'\\x89PNG\\r\\n\\x1a\\nfake-png-payload')
RESULT = {'env_clean': True, 'bbox': BBOX}
";
            let rep = inspect_gee_python_script(&tmp, user_code);
            assert!(!rep.blocked, "{:?}", rep.blocked_reasons);
            let appr = rep.approval_id.unwrap();
            let out = execute_approved_gee_script(
                &tmp,
                &appr,
                user_code,
                "proj-unit",
                "secret-bearer-xyz",
                [117.2, 31.8, 117.3, 31.9],
                None,
            )
            .expect("sandboxed script executes cleanly");
            assert_eq!(out["ok"], true);
            assert_eq!(out["result"]["env_clean"], true);
            assert_eq!(out["result"]["bbox"], json!([117.2, 31.8, 117.3, 31.9]));
            assert!(out["preview_png_bytes_len"].as_u64().unwrap() > 8);
            assert!(out["preview_png_path"]
                .as_str()
                .unwrap()
                .ends_with("preview.png"));

            // Regression for [P1-2]: division (`ndvi = (b8 - b4)/denom`) and CJK slash (`# 光学/雷达融合`)
            // get rewritten to `[local-reference]` by `insar_agent::history::outbound_text`.
            // 1) `preview_approved_gee_script` returns the exact uncorrupted script without consuming the token.
            // 2) Genuine tampering is rejected with `GEE_SCRIPT_HASH_MISMATCH` without consuming the token.
            // 3) Executing with EITHER the outbound-scrubbed preview OR the exact script runs the uncorrupted formula cleanly!
            let slash_script = "\
# 光学/雷达融合
b8 = 0.75
b4 = 0.25
denom = b8 + b4
ndvi = (b8 - b4)/denom
RESULT = {'ndvi': ndvi}
";
            let scrubbed_preview = insar_agent::history::outbound_text(slash_script);
            assert!(
                scrubbed_preview.contains("[local-reference]"),
                "precondition: outbound_text scrubs division/CJK slash: {scrubbed_preview}"
            );

            let rep_slash = inspect_gee_python_script(&tmp, slash_script);
            assert!(!rep_slash.blocked, "{:?}", rep_slash.blocked_reasons);
            let appr_slash = rep_slash.approval_id.expect("approval_id issued");

            let preview_val =
                preview_approved_gee_script(&appr_slash).expect("preview_approved_gee_script ok");
            assert_eq!(preview_val["ok"], true);
            assert_eq!(preview_val["approval_id"], appr_slash);
            assert_eq!(preview_val["script"], slash_script.trim());
            assert_eq!(preview_val["script_sha256"], rep_slash.script_sha256);

            let tampered_err = execute_approved_gee_script(
                &tmp,
                &appr_slash,
                "RESULT = {'ndvi': 999.0}",
                "proj-unit",
                "secret-bearer-xyz",
                [117.2, 31.8, 117.3, 31.9],
                None,
            )
            .expect_err("tampered script must be rejected");
            assert!(
                tampered_err.contains("GEE_SCRIPT_HASH_MISMATCH"),
                "expected GEE_SCRIPT_HASH_MISMATCH, got: {tampered_err}"
            );

            // Even when caller sends the outbound-scrubbed `script_preview`, execution uses `entry.inspected_script`!
            let out_from_scrubbed = execute_approved_gee_script(
                &tmp,
                &appr_slash,
                &scrubbed_preview,
                "proj-unit",
                "secret-bearer-xyz",
                [117.2, 31.8, 117.3, 31.9],
                None,
            )
            .expect("outbound-scrubbed preview executes canonical inspected_script");
            assert_eq!(out_from_scrubbed["ok"], true);
            assert!((out_from_scrubbed["result"]["ndvi"].as_f64().unwrap() - 0.5).abs() < 1e-9);
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scientific_review_catches_antipatterns_ignores_comments_handles_crlf_and_passes_safe_value()
    {
        let tmp =
            std::env::temp_dir().join(format!("eostage-gee-science-review-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);

        // 1. Comments and strings containing `.getInfo()` / `.mean()` must NOT trigger false findings,
        //    and CRLF + multi-byte UTF-8 Chinese text must compute exact 1-based line numbers.
        let clean_commented = "# 中文注释：这里提到了 img.getInfo() 与 WorldCover.mean() 但只是说明\r\nnote = \"img.getInfo() inside string\"\r\ns2 = ee.ImageCollection('COPERNICUS/S2_SR_HARMONIZED').filterBounds(AOI).filterDate('2025-01-01', '2025-06-01')\r\ndef mask_fn(img):\r\n\treturn img.updateMask(img.select('SCL').neq(9)).divide(10000)\r\nres = s2.map(mask_fn).median().reduceRegion(reducer=ee.Reducer.mean(), geometry=AOI, scale=10, maxPixels=1e9)\r\nRESULT = res.getInfo()\r\n";
        let clean_rep = validate_gee_python_script_static(&tmp, clean_commented);
        assert!(!clean_rep.blocked);
        assert!(
            !clean_rep.findings.iter().any(|f| f.code == "mapped-getinfo"
                || f.code == "loop-getinfo"
                || f.code == "categorical-mean"
                || f.code == "s2-no-pixel-mask"),
            "unexpected findings on clean script: {:?}",
            clean_rep.findings
        );
        let top_info = clean_rep
            .findings
            .iter()
            .find(|f| f.code == "getinfo-diagnostic")
            .expect("top-level getInfo yields INFO");
        assert_eq!(top_info.severity, "INFO");
        assert_eq!(top_info.line, 7);

        // 2. Anti-pattern & scientific pitfalls script
        let bad_science = r#"
s2 = ee.ImageCollection('COPERNICUS/S2_SR_HARMONIZED').filterBounds(AOI).filter(ee.Filter.lt('CLOUDY_PIXEL_PERCENTAGE', 20))
def bad_map(img):
    print(img.get('system:index'))
    v = img.getInfo()
    return img
mapped = s2.map(bad_map)
for i in range(3):
    x = mapped.first().getInfo()
wc = ee.ImageCollection('ESA/WorldCover/v200').filterBounds(AOI).first()
avg_cls = wc.reduceRegion(ee.Reducer.mean(), AOI)
Export.image.toDrive(image=avg_cls, description='out')
"#
        .replace('\n', "\r\n");
        let rep = validate_gee_python_script_static(&tmp, &bad_science);
        assert!(!rep.blocked, "scientific findings must not set blocked");
        let codes: Vec<&str> = rep.findings.iter().map(|f| f.code.as_str()).collect();
        for expected in [
            "s2-no-pixel-mask",
            "mapped-print",
            "mapped-getinfo",
            "loop-getinfo",
            "categorical-mean",
            "reduceRegion-missing-scale",
            "reduceRegion-missing-pixel-limit",
            "export-image-missing-region",
            "export-image-missing-scale",
            "export-image-missing-maxPixels",
            "export-not-started",
        ] {
            assert!(
                codes.contains(&expected),
                "missing expected finding {expected} in {codes:?}"
            );
        }
        assert!(
            rep.scientific_warnings.iter().any(|w| w.contains("L5]")),
            "mapped-getinfo line number should appear in scientific_warnings: {:?}",
            rep.scientific_warnings
        );

        let report_json = serde_json::to_value(&rep).unwrap();
        insar_agent::run::safe_value(&report_json)
            .expect("ScriptInspectionReport with findings must pass safe_value");

        let _ = fs::remove_dir_all(&tmp);
    }
}
