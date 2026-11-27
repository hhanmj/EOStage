//! Earlier turns of a session, for the next run's model request (A1 design
//! §5, L4 r7 P1-1): each earlier run becomes one user message (what the user
//! wrote) and one assistant message (a digest of that run's record and its
//! answer). The digest is built by host code from the run record, which holds
//! only trace-safe values: tools called, searches and the batches they froze,
//! the products of those batches (from the frozen batch in the Store, so the
//! orbit direction and size are there even when the run read only ids),
//! plans with their outcome, refusals, tasks created, and the conclusion text. Tool results are never replayed
//! verbatim and nothing is sent as a tool call.
//!
//! Bound: at most [`HISTORY_BUDGET_CHARS`] characters of history. Runs are
//! taken newest first; a run whose full digest no longer fits is shortened to
//! its user text and answer, and runs older than the first one that does not
//! fit even then are left out, with a note on the oldest message kept.
use crate::{message_dir, run::TraceEvent};
use insar_agent_model::Message;
use serde_json::{json, Value};

/// Characters of earlier conversation sent with a new message (legacy digest fallback).
pub const HISTORY_BUDGET_CHARS: usize = 24_000;
/// Hysteresis watermark for append-only multi-turn transcripts: stay verbatim
/// (preserving the prefix KV cache) up to 96k chars, then compact down to 24k.
pub const APPEND_HISTORY_COMPACT_TRIGGER_CHARS: usize = 96_000;
/// Target recent-turn tail kept verbatim after hysteresis compaction triggers.
pub const APPEND_HISTORY_COMPACT_TARGET_CHARS: usize = 24_000;
/// Maximum character length for the frozen host compaction summary message.
pub const COMPACT_SUMMARY_MAX_CHARS: usize = 8_000;
/// Products listed per batch in a digest.
const ITEMS_PER_BATCH: usize = 25;
/// One assistant text or answer in a digest.
const MAX_TEXT_CHARS: usize = 1_500;
/// The user text and answer of a shortened run.
const SHORT_TEXT_CHARS: usize = 600;

/// What one earlier run of the session left: the user's message, the
/// assistant's streamed text, its conclusion or error code, and its run record.
#[derive(Debug, Default, Clone)]
pub struct Turn {
    /// The earlier run this turn was (its record is read on demand).
    pub run_id: String,
    pub user: String,
    pub assistant: Vec<String>,
    pub conclusion: Option<String>,
    pub error: Option<String>,
    pub events: Vec<TraceEvent>,
    /// The frozen candidates (as JSON) of each batch the record names.
    pub batches: Vec<(String, Vec<Value>)>,
}

/// Batches a run record names: frozen by one of its searches or read.
pub fn named_batches(events: &[TraceEvent]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for event in events {
        let named = match event.kind.as_str() {
            "search_batch" => text(&event.payload, "batch_id"),
            "tool_result" => event
                .payload
                .get("summary")
                .and_then(|summary| text(summary, "search_batch_id")),
            _ => None,
        };
        if let Some(batch) = named {
            if !found.iter().any(|known| known == batch) {
                found.push(batch.to_owned());
            }
        }
    }
    found
}

/// The product ids a run record's read results name, per batch — the bounded
/// set `load_turn_record` needs from the Store (#22 B slice 2). A batch the
/// record froze but never read has no entry, so its digest falls back to one
/// bounded page of newest products instead of the whole candidate array.
pub fn named_items(events: &[TraceEvent]) -> std::collections::HashMap<String, Vec<String>> {
    let mut found: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for event in events {
        if event.kind != "tool_result" {
            continue;
        }
        let Some(summary) = event.payload.get("summary") else {
            continue;
        };
        let Some(batch) = text(summary, "search_batch_id") else {
            continue;
        };
        let entry = found.entry(batch.to_owned()).or_default();
        for item in summary
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let read = item.get("item").unwrap_or(item);
            if let Some(id) = text(read, "item_id") {
                if !entry.iter().any(|known| known == id) {
                    entry.push(id.to_owned());
                }
            }
        }
    }
    found
}

/// Text as it may leave the machine: a directory the message named is not
/// this run's `message_dir`, so its placeholder becomes a plain reference;
/// each word the run record would refuse (a local path, a signed or
/// credential-bearing value) becomes `[local-reference]`; text still refused
/// as a whole (a marker spanning words) is left out.
pub fn outbound_text(text: &str) -> String {
    let text = match message_dir::extract(text) {
        Some((_, scrubbed)) => scrubbed,
        None => text.to_owned(),
    }
    .replace(message_dir::PLACEHOLDER, crate::outbound::TEXT_MARKER);
    crate::outbound::redact_words(&text)
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(max).collect();
    clipped.push('…');
    clipped
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn legacy_tool_summary_line(tool: &str, summary: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(ok) = summary.get("ok").and_then(Value::as_bool) {
        parts.push(format!("ok={ok}"));
    }
    for key in [
        "display_name",
        "admin_name",
        "code",
        "dataset",
        "operation",
        "status",
        "error",
        "message",
    ] {
        if let Some(val) = summary.get(key).and_then(Value::as_str) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                parts.push(format!("{key}={}", clip(trimmed, 80)));
            }
        }
    }
    for key in ["count", "items_kept", "total_count"] {
        if let Some(n) = summary.get(key).and_then(Value::as_u64) {
            parts.push(format!("{key}={n}"));
        }
    }
    let detail = if parts.is_empty() {
        clip(&summary.to_string(), 140)
    } else {
        parts.join(", ")
    };
    format!("已调用历史工具 {tool}（仅作只读历史参考；当前仅可调用工具列表中启用的工具）：{detail}")
}

/// One product: id, name, time, direction, level, mode, size. `entry` is a
/// read item (a workbench row wraps it in `item`) or a frozen candidate.
fn item_line(entry: &Value) -> Option<String> {
    let item = entry.get("item").unwrap_or(entry);
    let id = text(item, "item_id")?;
    let metadata = item.get("metadata").cloned().unwrap_or(Value::Null);
    let mut parts = vec![id.to_owned()];
    if let Some(name) = text(item, "display_name") {
        parts.push(name.to_owned());
    }
    if let Some(time) = text(item, "acquisition_datetime") {
        parts.push(time.to_owned());
    }
    for key in ["flight_direction", "processing_level", "beam_mode"] {
        if let Some(value) = text(&metadata, key) {
            parts.push(value.to_owned());
        }
    }
    if let Some(bytes) = item
        .pointer("/payload/file_size_remote")
        .and_then(Value::as_u64)
    {
        parts.push(format!("{bytes} 字节"));
    }
    Some(format!("  - {}", parts.join(" | ")))
}

/// The facts of one run record, one line each (product listings indented).
/// A product read is listed from its frozen candidate when the Store has it.
fn record_lines(events: &[TraceEvent], frozen: &[(String, Vec<Value>)]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut reads: Vec<String> = Vec::new();
    let candidate = |batch: &str, id: &str| {
        frozen
            .iter()
            .find(|(known, _)| known == batch)
            .and_then(|(_, items)| items.iter().find(|item| text(item, "item_id") == Some(id)))
            .cloned()
    };
    // Newest read of each batch: (batch, revision, total, items).
    let mut batches: Vec<(String, Value, Value, Vec<String>)> = Vec::new();
    for event in events {
        let payload = &event.payload;
        match event.kind.as_str() {
            "tool_call" => {
                if let Some(tool) = text(payload, "tool_id") {
                    if payload.get("version").is_some() && !reads.iter().any(|r| r == tool) {
                        reads.push(tool.to_owned());
                    }
                }
            }
            "tool_result" => {
                let summary = payload.get("summary").cloned().unwrap_or(Value::Null);
                let tool = text(payload, "tool_id").unwrap_or("");
                if let Some(batch) = text(&summary, "search_batch_id") {
                    let items: Vec<String> = summary
                        .get("items")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|entry| {
                            let read = entry.get("item").unwrap_or(entry);
                            text(read, "item_id")
                                .and_then(|id| candidate(batch, id))
                                .unwrap_or_else(|| entry.clone())
                        })
                        .filter_map(|entry| item_line(&entry))
                        .take(ITEMS_PER_BATCH)
                        .collect();
                    let entry = (
                        batch.to_owned(),
                        summary.get("revision").cloned().unwrap_or(Value::Null),
                        summary.get("total_count").cloned().unwrap_or(Value::Null),
                        items,
                    );
                    match batches.iter_mut().find(|(id, ..)| id == batch) {
                        Some(found) => *found = entry,
                        None => batches.push(entry),
                    }
                } else if matches!(tool, "search_progress" | "search_start")
                    && text(&summary, "stage") == Some("done")
                {
                    lines.push(format!(
                        "检索 {} 完成，保留 {} 条",
                        text(&summary, "request_id").unwrap_or("?"),
                        summary.get("items_kept").unwrap_or(&Value::Null)
                    ));
                } else if tool == "create_task" {
                    lines.push(format!(
                        "建任务结果：任务 {}，状态 {}，新建 {}",
                        text(&summary, "task_id").unwrap_or("?"),
                        text(&summary, "status").unwrap_or("?"),
                        summary.get("created").unwrap_or(&Value::Null)
                    ));
                } else if !tool.is_empty() {
                    // Legacy / extension / plugin tool compatibility:
                    // preserve historical outcomes as read-only textual context so resumed sessions do not
                    // lose prior findings even if that tool is now disabled or unavailable, without offering it.
                    let note = legacy_tool_summary_line(tool, &summary);
                    if !note.is_empty() {
                        lines.push(note);
                    }
                }
            }
            "search_batch" => lines.push(format!(
                "检索 {} 冻结了批次 {}",
                text(payload, "request_id").unwrap_or("?"),
                text(payload, "batch_id").unwrap_or("?")
            )),
            "plan" => lines.push(format!(
                "历史计划编号 {}（{} 步，最高档位 {}）",
                text(payload, "plan_id").unwrap_or("?"),
                payload.get("steps").unwrap_or(&Value::Null),
                payload.get("max_tier").unwrap_or(&Value::Null)
            )),
            "approval" => {
                let decision = match text(payload, "decision") {
                    Some("approved") => "已批准",
                    Some("rejected") => "用户在确认卡上拒绝",
                    Some("expired") => "已失效",
                    Some("refused_forbidden") => "禁止的操作，已拒绝",
                    _ => "未决",
                };
                let via = match text(payload, "via") {
                    Some("auto_by_mode") => "（按权限模式自动）",
                    Some("desktop_card") => "（桌面确认卡）",
                    _ => "",
                };
                lines.push(format!(
                    "计划 {}：{decision}{via}",
                    text(payload, "plan_id").unwrap_or("?")
                ));
            }
            "plan_refused" => lines.push(format!(
                "主机拒绝了计划：reason {}",
                text(payload, "reason").unwrap_or("?")
            )),
            "execution" => {
                if let Some(task) = text(payload, "task_id") {
                    lines.push(format!(
                        "执行 {}：任务 {task}{}",
                        text(payload, "tool_id").unwrap_or("步骤"),
                        if payload.get("created") == Some(&json!(true)) {
                            "（本次新建）"
                        } else {
                            ""
                        }
                    ));
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if !reads.is_empty() {
        out.push(format!("读取：{}", reads.join("、")));
    }
    out.extend(lines);
    const COLUMNS: &str = "item_id | 名称 | 时间 | 方向 | 级别 | 模式 | 大小";
    for (batch, revision, total, items) in &batches {
        out.push(format!(
            "批次 {batch}（revision {revision}，共 {total} 条）读到的产品（{COLUMNS}）："
        ));
        out.extend(items.iter().cloned());
    }
    // A batch the run froze but never read: its newest products.
    for (batch, items) in frozen {
        if batches.iter().any(|(read, ..)| read == batch) {
            continue;
        }
        let mut items: Vec<&Value> = items.iter().collect();
        items.sort_by(|a, b| text(b, "acquisition_datetime").cmp(&text(a, "acquisition_datetime")));
        out.push(format!(
            "批次 {batch}（revision 0，共 {} 条，本轮未读取）最新的产品（{COLUMNS}）：",
            items.len()
        ));
        out.extend(
            items
                .into_iter()
                .take(ITEMS_PER_BATCH)
                .filter_map(item_line),
        );
    }
    out
}

fn assistant_block(turn: &Turn, full: bool) -> String {
    let mut parts = Vec::new();
    if full {
        let said: Vec<String> = turn
            .assistant
            .iter()
            .map(|text| insar_agent_model::agent::without_think(text))
            .map(|text| clip(&text, MAX_TEXT_CHARS))
            .filter(|text| !text.is_empty())
            .collect();
        if !said.is_empty() {
            parts.push(said.join("\n"));
        }
    }
    let limit = if full {
        MAX_TEXT_CHARS
    } else {
        SHORT_TEXT_CHARS
    };
    match (&turn.conclusion, &turn.error) {
        (Some(answer), _) => {
            let clean = insar_agent_model::agent::without_think(answer);
            let clipped = clip(&clean, limit);
            if !clipped.is_empty() {
                parts.push(clipped);
            }
        }
        (None, Some(code)) => parts.push(format!("[上一轮未完成] 错误 {code}")),
        // Cut short (interrupted, or still unanswered): say so rather than
        // send an empty assistant message.
        (None, None) => parts.push("[上一轮未完成]".to_owned()),
    }
    if full {
        let record = record_lines(&turn.events, &turn.batches);
        if !record.is_empty() {
            parts.push(format!(
                "（以下为主机自动附加的历史归档摘要，仅供核对过往批次与任务编号；严禁在回复正文中模仿或编写此类记录，本轮任何切换工作区、启动检索或建任务操作都必须通过真实的 function tool_calls 调用 propose_plan）：\n{}",
                record.join("；")
            ));
        }
    }
    if parts.is_empty() {
        parts.push("[上一轮未完成]".to_owned());
    }
    outbound_text(&parts.join("\n\n"))
}

/// Legacy two-message digest for a single historical run lacking `turn_messages.json`.
pub fn single_turn_legacy_messages(turn: &Turn) -> Vec<Message> {
    vec![
        Message {
            role: "user".into(),
            content: outbound_text(&clip(&turn.user, MAX_TEXT_CHARS * 4)),
        },
        Message {
            role: "assistant".into(),
            content: assistant_block(turn, true),
        },
    ]
}

/// Builds a frozen `(user, assistant)` pair summarizing compacted older turns
/// for hysteresis compaction (`APPEND_HISTORY_COMPACT_TRIGGER_CHARS` ->
/// `APPEND_HISTORY_COMPACT_TARGET_CHARS`). Keeps host-verified run facts
/// (batches, products, plans, approvals, tasks) and short intent snippets
/// while omitting verbatim old assistant prose so old answers never bloat the
/// summary or leak across compaction boundaries.
pub fn build_compaction_summary_pair(
    total_compacted_turns: usize,
    prior_assistant_digest: Option<&str>,
    newly_compacted_turns: &[Turn],
) -> Vec<Message> {
    let mut fact_lines: Vec<String> = Vec::new();
    if let Some(prev) = prior_assistant_digest {
        let stripped = prev
            .strip_prefix("（以下为主机自动归档的历史摘要，仅供核对过往批次与任务编号）：\n")
            .unwrap_or(prev)
            .trim();
        if !stripped.is_empty() && !stripped.starts_with("已归档更早的") {
            fact_lines.push(clip(stripped, COMPACT_SUMMARY_MAX_CHARS / 2));
        }
    }
    for turn in newly_compacted_turns {
        let intent = outbound_text(&clip(&turn.user, 120));
        let facts = record_lines(&turn.events, &turn.batches);
        if !facts.is_empty() {
            let joined = facts.join("；");
            if intent.is_empty() {
                fact_lines.push(format!("- {joined}"));
            } else {
                fact_lines.push(format!("- 请求「{intent}」：{joined}"));
            }
        } else if let Some(ref code) = turn.error {
            if !intent.is_empty() {
                fact_lines.push(format!("- 请求「{intent}」未完成（错误 {code}）"));
            }
        } else if !intent.is_empty() {
            fact_lines.push(format!("- 已处理请求：{intent}"));
        }
    }
    let user_text = outbound_text(&format!(
        "（更早的 {total_compacted_turns} 轮对话超出长度上限，已归档压缩并省略原始细节）\n请结合以下已归档的历史操作事实继续后续对话。"
    ));
    let assistant_raw = if fact_lines.is_empty() {
        format!("已归档更早的 {total_compacted_turns} 轮对话摘要，请继续基于最新轮次处理当前请求。")
    } else {
        let body = clip(
            &fact_lines.join("\n"),
            COMPACT_SUMMARY_MAX_CHARS.saturating_sub(64),
        );
        format!("（以下为主机自动归档的历史摘要，仅供核对过往批次与任务编号）：\n{body}")
    };
    let assistant_text = outbound_text(&clip(&assistant_raw, COMPACT_SUMMARY_MAX_CHARS));
    vec![
        Message {
            role: "user".into(),
            content: user_text,
        },
        Message {
            role: "assistant".into(),
            content: if assistant_text.trim().is_empty() {
                format!("已归档更早的 {total_compacted_turns} 轮对话摘要。")
            } else {
                assistant_text
            },
        },
    ]
}

/// User and assistant messages for the earlier turns, oldest first, within
/// [`HISTORY_BUDGET_CHARS`].
pub fn messages(turns: &[Turn]) -> Vec<Message> {
    messages_loading(&mut turns.to_vec(), |_| {})
}

/// [`messages`] for turns whose run record is read on demand: `load` fills a
/// turn's `events` and `batches` just before its digest is built, newest
/// first, so the runs older than the budget reaches are never read.
pub fn messages_loading(turns: &mut [Turn], mut load: impl FnMut(&mut Turn)) -> Vec<Message> {
    let mut kept: Vec<(String, String)> = Vec::new();
    let mut used = 0usize;
    let mut dropped = 0usize;
    for index in (0..turns.len()).rev() {
        load(&mut turns[index]);
        let turn = &turns[index];
        let user = outbound_text(&clip(&turn.user, MAX_TEXT_CHARS * 4));
        let full = (user.clone(), assistant_block(turn, true));
        let size = |pair: &(String, String)| pair.0.chars().count() + pair.1.chars().count();
        if used + size(&full) <= HISTORY_BUDGET_CHARS {
            used += size(&full);
            kept.push(full);
            continue;
        }
        let short = (
            outbound_text(&clip(&turn.user, SHORT_TEXT_CHARS)),
            assistant_block(turn, false),
        );
        if used + size(&short) <= HISTORY_BUDGET_CHARS {
            used += size(&short);
            kept.push(short);
            continue;
        }
        dropped = index + 1;
        break;
    }
    kept.reverse();
    if dropped > 0 {
        if let Some((user, _)) = kept.first_mut() {
            *user = format!("（更早的 {dropped} 轮对话超出长度上限，已省略）\n{user}");
        }
    }
    kept.into_iter()
        .flat_map(|(user, assistant)| {
            [
                Message {
                    role: "user".into(),
                    content: user,
                },
                Message {
                    role: "assistant".into(),
                    content: assistant,
                },
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review P2: a turn with neither conclusion nor error (cut short) gave an
    /// assistant message with no content.
    #[test]
    fn a_turn_without_an_answer_says_it_did_not_finish() {
        let messages = messages(&[Turn {
            user: "找最新一景".into(),
            ..Turn::default()
        }]);
        assert_eq!(messages.len(), 2);
        assert!(
            messages[1].content.contains("[上一轮未完成]"),
            "{:?}",
            messages[1]
        );
    }

    #[test]
    fn a_path_or_marker_word_is_replaced_and_plain_text_is_kept() {
        assert_eq!(outbound_text("找最新一景"), "找最新一景");
        assert_eq!(
            outbound_text("下载到 D:\\data\\s1 这个目录"),
            "下载到 [local-reference] 这个目录"
        );
        assert_eq!(outbound_text("/help 一下"), "[local-reference] 一下");
        assert_eq!(outbound_text("见 E:/out 吧"), "见 [local-reference] 吧");
    }

    #[test]
    fn newest_turns_are_kept_and_older_ones_beyond_the_budget_dropped() {
        let turns: Vec<Turn> = (0..10)
            .map(|index| Turn {
                user: format!("第{index}轮 {}", "很".repeat(7_000)),
                conclusion: Some(format!("结论{index} {}", "好".repeat(7_000))),
                ..Turn::default()
            })
            .collect();
        let messages = messages(&turns);
        let total: usize = messages.iter().map(|m| m.content.chars().count()).sum();
        // The note on the oldest kept message is the only text over budget.
        assert!(total <= HISTORY_BUDGET_CHARS + 64, "{total}");
        let last = &messages[messages.len() - 1];
        assert_eq!(last.role, "assistant");
        assert!(last.content.contains("结论9"), "{}", last.content);
        assert!(
            messages[messages.len() - 2].content.contains("第9轮"),
            "the newest turn keeps its user text"
        );
        assert!(
            messages[0].content.contains("已省略"),
            "{}",
            messages[0].content
        );
        assert!(!messages.iter().any(|m| m.content.contains("第0轮")));
        assert!(messages
            .chunks(2)
            .all(|pair| pair[0].role == "user" && pair[1].role == "assistant"));
    }

    /// Review P2: the run records of turns beyond the budget are not read.
    #[test]
    fn run_records_are_read_newest_first_only_while_the_budget_lasts() {
        let mut turns: Vec<Turn> = (0..30)
            .map(|index| Turn {
                user: format!("第{index}轮 {}", "长".repeat(7_000)),
                ..Turn::default()
            })
            .collect();
        let mut loaded = Vec::new();
        let messages = messages_loading(&mut turns, |turn| {
            loaded.push(turn.user.chars().take(3).collect::<String>())
        });
        assert!(!messages.is_empty());
        assert!(loaded.len() < 30, "{loaded:?}");
        assert_eq!(loaded[0], "第29", "{loaded:?}");
    }

    #[test]
    fn thinking_blocks_are_stripped_from_turn_history_digest() {
        let messages = messages(&[Turn {
            user: "查四川省数据".into(),
            assistant: vec!["<think>内部推理草稿</think>正在查询四川省工作区。".into()],
            conclusion: Some("<think>最后核对一遍</think>共找到 4 景影像。".into()),
            ..Turn::default()
        }]);
        assert_eq!(messages.len(), 2);
        assert!(!messages[1].content.contains("内部推理草稿"));
        assert!(!messages[1].content.contains("最后核对一遍"));
        assert!(messages[1].content.contains("正在查询四川省工作区。"));
        assert!(messages[1].content.contains("共找到 4 景影像。"));
    }

    #[test]
    fn assistant_block_puts_prose_first_and_never_emits_imitable_bracket_headers() {
        use crate::run::TraceEvent;
        use serde_json::json;

        let messages = messages(&[Turn {
            user: "切换到四川省".into(),
            conclusion: Some("已将工作区切换至四川省。".into()),
            events: vec![TraceEvent {
                sequence: 1,
                kind: "plan".into(),
                payload: json!({"plan_id": "plan-1", "steps": 1, "max_tier": 1}),
                previous_sha256: String::new(),
                sha256: "0".repeat(64),
            }],
            ..Turn::default()
        }]);
        assert_eq!(messages.len(), 2);
        let content = &messages[1].content;
        assert!(
            content.starts_with("已将工作区切换至四川省。"),
            "natural prose must appear first: {content}"
        );
        assert!(!content.contains("[上一轮运行记录]"), "{content}");
        assert!(!content.contains("[上一轮结论]"), "{content}");
        assert!(!content.contains("[上一轮过程说明]"), "{content}");
        assert!(!content.contains("提出计划"), "{content}");
        assert!(content.contains("plan-1"), "{content}");
        assert!(
            content.contains("严禁在回复正文中模仿或编写此类记录"),
            "{content}"
        );
    }

    #[test]
    fn legacy_tool_results_are_preserved_as_readonly_history_context() {
        use crate::run::TraceEvent;
        use serde_json::json;

        let messages = messages(&[Turn {
            user: "分析去年哨兵二号结果".into(),
            conclusion: Some("此前已用旧版插件完成分析。".into()),
            events: vec![TraceEvent {
                sequence: 1,
                kind: "tool_result".into(),
                payload: json!({
                    "tool_id": "ext:legacy_retired_analyzer",
                    "summary": {"ok": true, "dataset": "S2_SR", "count": 4, "status": "done"}
                }),
                previous_sha256: String::new(),
                sha256: "0".repeat(64),
            }],
            ..Turn::default()
        }]);
        assert_eq!(messages.len(), 2);
        let content = &messages[1].content;
        assert!(content.contains("ext:legacy_retired_analyzer"), "{content}");
        assert!(content.contains("仅作只读历史参考"), "{content}");
        assert!(content.contains("dataset=S2_SR"), "{content}");
        assert!(content.contains("count=4"), "{content}");
    }
}
