//! Deterministic post-reply numerical & active-AOI fact verifier for `insar-agent`.
//!
//! Clean-room design: extracts operational facts strictly from the current run's
//! sealed tool trace (`tool_result` and `execution` events) — never from static
//! SOP/Skill prose — tracks AOI switches via `working_aoi_set` so stale counts
//! from an earlier area cannot pass as current-AOI statistics, and verifies
//! concrete quantitative claims (scene counts, created task counts, inline
//! ratio-to-percentage arithmetic) in the assistant's Markdown reply.

use crate::run::TraceEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Exact integer rounding check for `mantissa / 10^decimals == round(100 * n / t)`:
/// `t > 0` and `|2 * mantissa * t - 200 * n * 10^decimals| <= t`.
pub fn pct_matches_rounding(
    numerator: u64,
    denominator: u64,
    mantissa: u64,
    decimals: u32,
) -> bool {
    if denominator == 0 || decimals > 6 {
        return false;
    }
    let scale = 10_i128.pow(decimals);
    let lhs = 2_i128
        .saturating_mul(i128::from(mantissa))
        .saturating_mul(i128::from(denominator));
    let rhs = 200_i128
        .saturating_mul(i128::from(numerator))
        .saturating_mul(scale);
    lhs.abs_diff(rhs) <= u128::from(denominator)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObservedSearchBatch {
    pub batch_key: String,
    pub total_count: Option<u64>,
    pub returned_count: Option<u64>,
    pub aoi_label: Option<String>,
    /// True if `working_aoi_set` subsequently switched the workspace to a different AOI.
    pub stale_aoi: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunFactLedger {
    pub active_aoi_name: Option<String>,
    pub active_aoi_bbox: Option<[f64; 4]>,
    pub search_batches: Vec<ObservedSearchBatch>,
    pub created_task_ids: BTreeSet<String>,
    pub observed_any_tool: bool,
}

impl RunFactLedger {
    /// Builds an authoritative fact ledger exclusively from the current run's
    /// sealed `TraceEvent` log.
    pub fn from_trace_events(events: &[TraceEvent]) -> Self {
        let mut ledger = Self::default();
        let mut batch_index: BTreeMap<String, usize> = BTreeMap::new();
        let mut anonymous_batch_seq = 0usize;

        for event in events {
            match event.kind.as_str() {
                "tool_result" => {
                    ledger.observed_any_tool = true;
                    let tool_id = event
                        .payload
                        .get("tool_id")
                        .or_else(|| event.payload.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let summary = event.payload.get("summary").unwrap_or(&Value::Null);

                    if tool_id == "working_aoi_set" || tool_id == "working_aoi_get" {
                        let next_name = summary
                            .get("display_name")
                            .or_else(|| summary.get("name"))
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned);
                        let next_bbox = parse_bbox4(summary.get("bbox"));

                        if tool_id == "working_aoi_set" {
                            let switched = (next_name.is_some()
                                && ledger.active_aoi_name.is_some()
                                && next_name != ledger.active_aoi_name)
                                || (next_bbox.is_some()
                                    && ledger.active_aoi_bbox.is_some()
                                    && next_bbox != ledger.active_aoi_bbox);
                            if switched {
                                for batch in &mut ledger.search_batches {
                                    batch.stale_aoi = true;
                                }
                            }
                        }
                        if next_name.is_some() {
                            ledger.active_aoi_name = next_name;
                        }
                        if next_bbox.is_some() {
                            ledger.active_aoi_bbox = next_bbox;
                        }
                    }

                    // Search results observed from `search_progress` or `batch_page`
                    if matches!(tool_id, "search_progress" | "batch_page")
                        || summary.get("search_batch_id").is_some()
                    {
                        let total_count = first_u64(
                            summary,
                            &["total_count", "total_hits", "total", "candidate_count"],
                        )
                        .or_else(|| {
                            summary
                                .get("result_count")
                                .and_then(Value::as_u64)
                                .or_else(|| summary.get("count").and_then(Value::as_u64))
                        });
                        let returned_count = summary
                            .get("returned_count")
                            .and_then(Value::as_u64)
                            .or_else(|| {
                                summary
                                    .get("items")
                                    .and_then(Value::as_array)
                                    .map(|a| a.len() as u64)
                            });
                        if total_count.is_some() || returned_count.is_some() {
                            let key = summary
                                .get("search_batch_id")
                                .or_else(|| summary.get("batch_id"))
                                .or_else(|| summary.get("request_id"))
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                                .unwrap_or_else(|| {
                                    anonymous_batch_seq += 1;
                                    format!("anon-batch-{anonymous_batch_seq}")
                                });
                            if let Some(&idx) = batch_index.get(&key) {
                                let slot = &mut ledger.search_batches[idx];
                                if total_count.is_some() {
                                    slot.total_count = total_count;
                                }
                                if returned_count.is_some() {
                                    slot.returned_count = returned_count;
                                }
                                slot.stale_aoi = false;
                            } else {
                                let idx = ledger.search_batches.len();
                                batch_index.insert(key.clone(), idx);
                                ledger.search_batches.push(ObservedSearchBatch {
                                    batch_key: key,
                                    total_count,
                                    returned_count,
                                    aoi_label: ledger.active_aoi_name.clone(),
                                    stale_aoi: false,
                                });
                            }
                        }
                    }

                    // Created download task IDs observed from extension/direct task tools
                    if matches!(
                        tool_id,
                        "create_task" | "nisar_create_subset_task" | "gee_chunked_download"
                    ) {
                        if let Some(tid) = summary.get("task_id").and_then(Value::as_str) {
                            if !tid.trim().is_empty() {
                                ledger.created_task_ids.insert(tid.trim().to_owned());
                            }
                        }
                    }
                }
                "execution" => {
                    ledger.observed_any_tool = true;
                    let tool_id = event
                        .payload
                        .get("tool_id")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if tool_id == "working_aoi_set" {
                        let summary = event.payload.get("summary").unwrap_or(&Value::Null);
                        let next_name = summary
                            .get("display_name")
                            .or_else(|| summary.get("name"))
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned);
                        let next_bbox = parse_bbox4(summary.get("bbox"));
                        let switched = (next_name.is_some()
                            && ledger.active_aoi_name.is_some()
                            && next_name != ledger.active_aoi_name)
                            || (next_bbox.is_some()
                                && ledger.active_aoi_bbox.is_some()
                                && next_bbox != ledger.active_aoi_bbox);
                        if switched {
                            for batch in &mut ledger.search_batches {
                                batch.stale_aoi = true;
                            }
                        }
                        if next_name.is_some() {
                            ledger.active_aoi_name = next_name;
                        }
                        if next_bbox.is_some() {
                            ledger.active_aoi_bbox = next_bbox;
                        }
                    }
                    if tool_id == "create_task" {
                        if let Some(tid) = event.payload.get("task_id").and_then(Value::as_str) {
                            if !tid.trim().is_empty() {
                                ledger.created_task_ids.insert(tid.trim().to_owned());
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        ledger
    }

    pub fn active_search_batches(&self) -> Vec<&ObservedSearchBatch> {
        self.search_batches
            .iter()
            .filter(|b| !b.stale_aoi)
            .collect()
    }

    pub fn stale_search_batches(&self) -> Vec<&ObservedSearchBatch> {
        self.search_batches.iter().filter(|b| b.stale_aoi).collect()
    }

    pub fn created_task_count(&self) -> u64 {
        self.created_task_ids.len() as u64
    }

    fn allowed_active_scene_counts(&self) -> BTreeSet<u64> {
        let mut allowed = BTreeSet::new();
        let active = self.active_search_batches();
        let mut total_sum = 0u64;
        let mut has_total = false;
        let mut ret_sum = 0u64;
        let mut has_ret = false;
        for batch in active {
            if let Some(t) = batch.total_count {
                allowed.insert(t);
                total_sum = total_sum.saturating_add(t);
                has_total = true;
            }
            if let Some(r) = batch.returned_count {
                allowed.insert(r);
                ret_sum = ret_sum.saturating_add(r);
                has_ret = true;
            }
        }
        if has_total {
            allowed.insert(total_sum);
        }
        if has_ret {
            allowed.insert(ret_sum);
        }
        allowed
    }

    fn stale_only_scene_counts(&self) -> BTreeSet<u64> {
        let mut stale = BTreeSet::new();
        let mut sum = 0u64;
        for batch in self.stale_search_batches() {
            if let Some(t) = batch.total_count {
                stale.insert(t);
                sum = sum.saturating_add(t);
            }
            if let Some(r) = batch.returned_count {
                stale.insert(r);
            }
        }
        if !self.stale_search_batches().is_empty() {
            stale.insert(sum);
        }
        stale
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FactMismatchKind {
    SceneCountMismatch,
    StaleAoiCount,
    TaskCountMismatch,
    PercentageRoundingMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactMismatch {
    pub kind: FactMismatchKind,
    pub claimed_text: String,
    pub explanation_zh: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactVerificationReport {
    pub checked_claims: usize,
    pub mismatches: Vec<FactMismatch>,
}

impl FactVerificationReport {
    pub fn is_clean(&self) -> bool {
        self.mismatches.is_empty()
    }

    /// Formats a concise, honest Chinese correction block when deterministic
    /// verification detects numerical drift or stale-AOI counts in the reply.
    /// Returns `None` when all checked claims match the observed run facts.
    pub fn audit_suffix_zh(&self) -> Option<String> {
        if self.mismatches.is_empty() {
            return None;
        }
        let mut lines = vec!["**本轮数据核对提示（以工具实测结果为准）**".to_owned()];
        for mismatch in &self.mismatches {
            lines.push(format!(
                "- ⚠️ 原文表述「{}」与本轮实测记录不一致：{}",
                mismatch.claimed_text, mismatch.explanation_zh
            ));
        }
        Some(lines.join("\n"))
    }
}

/// Convenience entry point for `session.rs::conclude`: builds the ledger from
/// sealed trace events and verifies the reply text deterministically.
pub fn verify_trace_reply(events: &[TraceEvent], reply_text: &str) -> FactVerificationReport {
    let ledger = RunFactLedger::from_trace_events(events);
    verify_reply(&ledger, reply_text)
}

/// Verifies concrete quantitative claims in `reply_text` against `ledger`.
pub fn verify_reply(ledger: &RunFactLedger, reply_text: &str) -> FactVerificationReport {
    let prose = strip_fenced_code_blocks(reply_text);
    let mut report = FactVerificationReport::default();

    // 1. Verify explicit scene/product search hit counts when search batches were observed
    let active_counts = ledger.allowed_active_scene_counts();
    let stale_counts = ledger.stale_only_scene_counts();
    if !active_counts.is_empty() || !stale_counts.is_empty() {
        for (claimed_n, snippet) in extract_scene_count_claims(&prose) {
            report.checked_claims += 1;
            if active_counts.contains(&claimed_n) {
                continue;
            }
            if stale_counts.contains(&claimed_n) {
                let active_desc = format_count_set(&active_counts);
                report.mismatches.push(FactMismatch {
                    kind: FactMismatchKind::StaleAoiCount,
                    claimed_text: snippet,
                    explanation_zh: format!(
                        "该数量属于切换研究区前的历史检索结果，当前研究区本轮实测景数为 {active_desc}"
                    ),
                });
            } else {
                let active_desc = format_count_set(&active_counts);
                report.mismatches.push(FactMismatch {
                    kind: FactMismatchKind::SceneCountMismatch,
                    claimed_text: snippet,
                    explanation_zh: format!("当前研究区本轮工具实测返回景数为 {active_desc}"),
                });
            }
        }
    }

    // 2. Verify explicit created download task count claims when any tool ran
    if ledger.observed_any_tool {
        let actual_tasks = ledger.created_task_count();
        for (claimed_n, snippet) in extract_created_task_count_claims(&prose) {
            report.checked_claims += 1;
            if claimed_n != actual_tasks {
                report.mismatches.push(FactMismatch {
                    kind: FactMismatchKind::TaskCountMismatch,
                    claimed_text: snippet,
                    explanation_zh: if actual_tasks == 0 {
                        "本轮实际尚未创建任何下载任务（0 个）".to_owned()
                    } else {
                        format!("本轮实际创建的下载任务数为 {actual_tasks} 个")
                    },
                });
            }
        }
    }

    // 3. Verify explicit inline `n/total (p%)` ratio-percentage arithmetic
    for (num, den, mantissa, decimals, snippet) in extract_ratio_pct_claims(&prose) {
        report.checked_claims += 1;
        if !pct_matches_rounding(num, den, mantissa, decimals) {
            let expected = (num as f64) * 100.0 / (den as f64);
            report.mismatches.push(FactMismatch {
                kind: FactMismatchKind::PercentageRoundingMismatch,
                claimed_text: snippet,
                explanation_zh: format!(
                    "按实测比例 {num}/{den} 精确换算应为 {:.prec$}%",
                    expected,
                    prec = decimals as usize
                ),
            });
        }
    }

    report
}

fn format_count_set(counts: &BTreeSet<u64>) -> String {
    if counts.is_empty() {
        return "0 景（尚未对新研究区执行检索）".to_owned();
    }
    let parts: Vec<String> = counts.iter().map(|n| format!("{n} 景")).collect();
    parts.join(" / ")
}

fn first_u64(val: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|k| val.get(*k).and_then(Value::as_u64))
}

fn parse_bbox4(val: Option<&Value>) -> Option<[f64; 4]> {
    let arr = val?.as_array()?;
    if arr.len() != 4 {
        return None;
    }
    let b = [
        arr[0].as_f64()?,
        arr[1].as_f64()?,
        arr[2].as_f64()?,
        arr[3].as_f64()?,
    ];
    b.iter().all(|v| v.is_finite()).then_some(b)
}

fn strip_fenced_code_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Extracts `(count, matched_snippet)` from explicit search-hit phrasings such as
/// `共找到 12 景`, `检索到 8 条影像`, `共返回 5 景`, `found 12 scenes`.
fn extract_scene_count_claims(text: &str) -> Vec<(u64, String)> {
    const PREFIXES: &[&str] = &[
        "共找到",
        "共检索到",
        "检索到",
        "查找到",
        "共查到",
        "查到",
        "共返回",
        "返回了",
        "命中了",
        "共命中",
        "found ",
        "returned ",
    ];
    const SUFFIXES: &[&str] = &[
        "景",
        "条影像",
        "条产品",
        "条记录",
        "条结果",
        "幅影像",
        "幅",
        " scenes",
        " scene",
        " products",
        " items",
    ];
    extract_prefixed_count_claims(text, PREFIXES, SUFFIXES)
}

/// Extracts `(count, matched_snippet)` from explicit created-task phrasings such as
/// `已创建 2 个下载任务`, `新建了 1 个任务`, `created 2 download tasks`.
fn extract_created_task_count_claims(text: &str) -> Vec<(u64, String)> {
    const PREFIXES: &[&str] = &[
        "已创建",
        "已为您创建",
        "成功创建",
        "新建了",
        "共创建",
        "创建了",
        "created ",
    ];
    const SUFFIXES: &[&str] = &[
        "个下载任务",
        "个任务",
        "项下载任务",
        "条下载任务",
        " download tasks",
        " download task",
        " tasks",
        " task",
    ];
    extract_prefixed_count_claims(text, PREFIXES, SUFFIXES)
}

fn extract_prefixed_count_claims(
    text: &str,
    prefixes: &[&str],
    suffixes: &[&str],
) -> Vec<(u64, String)> {
    let mut results = Vec::new();
    let mut seen_positions = BTreeSet::new();

    for prefix in prefixes {
        let mut search_from = 0usize;
        while let Some(rel) = text[search_from..].find(prefix) {
            let start = search_from + rel;
            let after_prefix = start + prefix.len();
            search_from = after_prefix;
            if !seen_positions.insert(start) {
                continue;
            }
            // Skip optional spaces / markdown bold markers (`**`) / `约`
            let tail = &text[after_prefix..];
            let mut offset = 0usize;
            for ch in tail.chars() {
                if ch.is_whitespace() || ch == '*' || ch == '`' {
                    offset += ch.len_utf8();
                } else {
                    break;
                }
            }
            let num_slice = &tail[offset..];
            let digit_bytes: usize = num_slice.bytes().take_while(|b| b.is_ascii_digit()).count();
            if digit_bytes == 0 || digit_bytes > 9 {
                continue;
            }
            let Ok(num) = num_slice[..digit_bytes].parse::<u64>() else {
                continue;
            };
            let after_num = &num_slice[digit_bytes..];
            let mut suffix_skip = 0usize;
            for ch in after_num.chars() {
                if ch == ' ' || ch == '*' || ch == '`' {
                    suffix_skip += ch.len_utf8();
                } else {
                    break;
                }
            }
            let after_num_clean = &after_num[suffix_skip..];
            if let Some(matched_suffix) = suffixes
                .iter()
                .find(|suf| after_num_clean.starts_with(**suf))
            {
                let snippet_end =
                    after_prefix + offset + digit_bytes + suffix_skip + matched_suffix.len();
                let snippet = text[start..snippet_end].replace("**", "");
                results.push((num, snippet));
            }
        }
    }
    results
}

/// Extracts explicit `n/total (p%)` or `n/total（p%）` arithmetic expressions.
fn extract_ratio_pct_claims(text: &str) -> Vec<(u64, u64, u64, u32, String)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'/' && i > 0 && i + 1 < bytes.len() {
            // Walk left for numerator digits
            let mut n_start = i;
            while n_start > 0 && bytes[n_start - 1].is_ascii_digit() {
                n_start -= 1;
            }
            // Walk right for denominator digits
            let mut d_end = i + 1;
            while d_end < bytes.len() && bytes[d_end].is_ascii_digit() {
                d_end += 1;
            }
            // Skip dates like 2026/11/20 by ensuring neither side is adjacent to another '/'
            let prev_is_slash = n_start > 0 && bytes[n_start - 1] == b'/';
            let next_is_slash = d_end < bytes.len() && bytes[d_end] == b'/';
            if n_start < i && d_end > i + 1 && !prev_is_slash && !next_is_slash {
                if let (Ok(num), Ok(den)) = (
                    text[n_start..i].parse::<u64>(),
                    text[i + 1..d_end].parse::<u64>(),
                ) {
                    if den > 0 && num <= den {
                        // Look immediately after denominator for `(` or `（` followed by `<pct>%`
                        let rest = &text[d_end..];
                        let trimmed_ws = rest.trim_start_matches([' ', '\t']);
                        let open_len = if trimmed_ws.starts_with('(') {
                            1
                        } else if trimmed_ws.starts_with('（') {
                            '（'.len_utf8()
                        } else {
                            0
                        };
                        if open_len > 0 {
                            let inner = trimmed_ws[open_len..].trim_start();
                            if let Some((mantissa, decimals, consumed)) = parse_percentage(inner) {
                                let after_pct = inner[consumed..].trim_start();
                                let close_len = if after_pct.starts_with(')') {
                                    1
                                } else if after_pct.starts_with('）') {
                                    '）'.len_utf8()
                                } else {
                                    0
                                };
                                if close_len > 0 {
                                    let snippet = format!("{num}/{den} ({})", &inner[..consumed]);
                                    out.push((num, den, mantissa, decimals, snippet));
                                }
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    out
}

/// Parses `37.5%` or `50％` -> `(mantissa, decimals, byte_len)`.
fn parse_percentage(s: &str) -> Option<(u64, u32, usize)> {
    let bytes = s.as_bytes();
    let mut idx = 0usize;
    let mut saw_digit = false;
    let mut saw_dot = false;
    let mut decimals = 0u32;
    let mut mantissa_digits = String::new();

    while idx < bytes.len() {
        let b = bytes[idx];
        if b.is_ascii_digit() {
            saw_digit = true;
            mantissa_digits.push(b as char);
            if saw_dot {
                decimals += 1;
            }
            idx += 1;
        } else if b == b'.' && !saw_dot && saw_digit {
            saw_dot = true;
            idx += 1;
        } else {
            break;
        }
    }
    if !saw_digit || decimals > 6 || mantissa_digits.len() > 12 {
        return None;
    }
    let after = &s[idx..];
    let pct_len = if after.starts_with('%') {
        1
    } else if after.starts_with('％') {
        '％'.len_utf8()
    } else {
        return None;
    };
    let mantissa = mantissa_digits.parse::<u64>().ok()?;
    Some((mantissa, decimals, idx + pct_len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn trace(seq: u64, kind: &str, payload: Value) -> TraceEvent {
        TraceEvent {
            sequence: seq,
            kind: kind.into(),
            payload,
            previous_sha256: String::new(),
            sha256: String::new(),
        }
    }

    #[test]
    fn pct_rounding_checker_is_exact() {
        // 3/8 = 37.5%
        assert!(pct_matches_rounding(3, 8, 375, 1));
        assert!(pct_matches_rounding(3, 8, 38, 0));
        assert!(!pct_matches_rounding(3, 8, 425, 1));
        // 1/3 = 33.33%
        assert!(pct_matches_rounding(1, 3, 333, 1));
        assert!(!pct_matches_rounding(1, 3, 340, 1));
        assert!(!pct_matches_rounding(1, 0, 0, 0));
    }

    #[test]
    fn clean_reply_matching_real_batch_and_task_counts_passes_without_suffix() {
        let events = vec![
            trace(
                0,
                "tool_result",
                json!({
                    "tool_id": "search_progress",
                    "summary": { "search_batch_id": "batch-1", "total_count": 12, "returned_count": 10 }
                }),
            ),
            trace(
                1,
                "execution",
                json!({ "tool_id": "create_task", "task_id": "task-1" }),
            ),
        ];
        let report = verify_trace_reply(
            &events,
            "已为您检索完成，共找到 12 景影像，并已创建 1 个下载任务，其中高质量占比 3/8（37.5%）。",
        );
        assert!(report.is_clean(), "{report:?}");
        assert_eq!(report.checked_claims, 3);
        assert!(report.audit_suffix_zh().is_none());
    }

    #[test]
    fn catches_hallucinated_scene_count_unbacked_task_and_bad_percentage() {
        let events = vec![trace(
            0,
            "tool_result",
            json!({
                "tool_id": "search_progress",
                "summary": { "search_batch_id": "batch-1", "total_count": 0, "returned_count": 0 }
            }),
        )];
        let report = verify_trace_reply(
            &events,
            "本次检索共找到 9 景可用影像，已创建 2 个下载任务，低云比例 3/8（42.5%）。",
        );
        assert_eq!(report.mismatches.len(), 3, "{report:?}");
        assert_eq!(
            report.mismatches[0].kind,
            FactMismatchKind::SceneCountMismatch
        );
        assert_eq!(
            report.mismatches[1].kind,
            FactMismatchKind::TaskCountMismatch
        );
        assert_eq!(
            report.mismatches[2].kind,
            FactMismatchKind::PercentageRoundingMismatch
        );
        let suffix = report.audit_suffix_zh().unwrap();
        assert!(suffix.contains("0 景"));
        assert!(suffix.contains("37.5%"));
        assert!(!suffix.contains("batch-1"));
    }

    #[test]
    fn catches_stale_aoi_count_after_working_aoi_switch() {
        let events = vec![
            trace(
                0,
                "execution",
                json!({
                    "tool_id": "working_aoi_set",
                    "summary": { "display_name": "合肥市", "bbox": [117.1, 31.7, 117.5, 32.0] }
                }),
            ),
            trace(
                1,
                "tool_result",
                json!({
                    "tool_id": "search_progress",
                    "summary": { "search_batch_id": "batch-hefei", "total_count": 18 }
                }),
            ),
            trace(
                2,
                "execution",
                json!({
                    "tool_id": "working_aoi_set",
                    "summary": { "display_name": "成都市", "bbox": [103.8, 30.4, 104.3, 30.9] }
                }),
            ),
            trace(
                3,
                "tool_result",
                json!({
                    "tool_id": "search_progress",
                    "summary": { "search_batch_id": "batch-chengdu", "total_count": 4 }
                }),
            ),
        ];
        let report = verify_trace_reply(&events, "成都市当前共找到 18 景哨兵影像。");
        assert_eq!(report.mismatches.len(), 1, "{report:?}");
        assert_eq!(report.mismatches[0].kind, FactMismatchKind::StaleAoiCount);
        let suffix = report.audit_suffix_zh().unwrap();
        assert!(suffix.contains("切换研究区前"));
        assert!(suffix.contains("4 景"));
    }
}
