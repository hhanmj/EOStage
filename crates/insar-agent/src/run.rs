use insar_store::{AgentRunFlags, AgentRunRow, StoreError, TaskStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RunError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("run trace is missing or was modified")]
    Integrity,
    #[error("unsafe local data in Agent trace")]
    Unsafe,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceEvent {
    pub sequence: u64,
    pub previous_sha256: String,
    pub kind: String,
    pub payload: Value,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunRecord {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub model_route: Option<String>,
    #[serde(default)]
    pub usage_prompt: Option<u64>,
    #[serde(default)]
    pub usage_completion: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_cached_input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_cache_creation_input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_reasoning: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_last_prompt: Option<u64>,
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// Fail closed on sensitive names and path/URL forms; a known-safe field set
// can be stored, rather than silently logging raw HTTP/provider responses.

/// Whether `text` holds a `/` that begins a Unix path (review 2026-11-02,
/// privacy-first ruling of the second review): its previous character is
/// none of an ASCII letter, digit, `.`, `~`, `:`, `/` or `%` (so
/// `https://a/b`, `TCP/IP`, `math.pi/2` are not paths), its next character
/// is neither `/` nor whitespace (`//server`, a lone `/`), and the first
/// segment starts with a letter, ASCII or not (`1/2`, `2026/11/02`, `24/7`
/// and a lone `/` are not; `/home`, `/data`, `下载到/数据` are — `他/她`
/// is a false positive the privacy-first ruling accepts). Text after `://`
/// is not read (an address carries its own path).
fn is_unix_root_segment(segment: &str) -> bool {
    let base = segment.split('.').next().unwrap_or(segment);
    matches!(
        base.to_ascii_lowercase().as_str(),
        "users"
            | "home"
            | "tmp"
            | "var"
            | "etc"
            | "opt"
            | "applications"
            | "mnt"
            | "media"
            | "root"
            | "srv"
            | "data"
            | "usr"
            | "bin"
            | "sbin"
            | "dev"
            | "proc"
            | "sys"
            | "run"
            | "private"
            | "library"
            | "system"
            | "volumes"
            | "boot"
            | "lib"
            | "lib64"
            | "snap"
            | "nix"
            | "workspace"
            | "work"
            | "scratch"
            | "help"
    )
}

pub(crate) fn unix_path_after_slash(previous: Option<char>, rest: &str) -> bool {
    let Some(first) = rest.chars().next() else {
        return false;
    };
    if first == '/' || first.is_whitespace() || !first.is_alphabetic() {
        return false;
    }
    if let Some(prev) = previous {
        if prev.is_ascii_alphanumeric()
            || matches!(
                prev,
                '.' | '~'
                    | ':'
                    | '/'
                    | '%'
                    | '\\'
                    | '°'
                    | '℃'
                    | '℉'
                    | '′'
                    | '″'
                    | ')'
                    | ']'
                    | '}'
                    | '>'
                    | '$'
                    | '^'
                    | '_'
                    | '+'
                    | '-'
                    | '*'
                    | '±'
                    | '×'
                    | '÷'
                    | '²'
                    | '³'
            )
        {
            return false;
        }
    }
    let seg_end = rest
        .char_indices()
        .find(|&(_, c)| {
            c.is_whitespace()
                || matches!(
                    c,
                    '/' | '\\'
                        | '\n'
                        | '"'
                        | '\''
                        | '<'
                        | '>'
                        | '|'
                        | ','
                        | ';'
                        | ':'
                        | ')'
                        | ']'
                        | '}'
                        | '。'
                        | '，'
                        | '；'
                        | '：'
                        | '）'
                        | '】'
                        | '”'
                        | '’'
                )
        })
        .map_or(rest.len(), |(idx, _)| idx);
    let first_seg = &rest[..seg_end];
    let has_second_slash = rest[seg_end..].starts_with('/');

    if first.is_ascii_alphabetic() {
        if is_unix_root_segment(first_seg) {
            return true;
        }
        let after_non_path_word = previous
            .is_some_and(|prev| prev.is_alphabetic() && !matches!(prev, '到' | '在' | '至' | '入'));
        !after_non_path_word
            && has_second_slash
            && first_seg.len() >= 2
            && first_seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    } else {
        matches!(previous, Some('到' | '在' | '至' | '入'))
            || (previous == Some('他') && first_seg.starts_with('她'))
            || (has_second_slash
                && matches!(
                    first_seg,
                    "数据" | "下载" | "桌面" | "目录" | "文件" | "用户"
                ))
    }
}

fn unc_path_in(text: &str) -> bool {
    text.match_indices("\\\\").any(|(index, _)| {
        text[..index]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | '[' | '{'))
            && text[index + 2..]
                .trim_start_matches('\\')
                .starts_with(|c: char| c.is_ascii_alphanumeric())
    })
}

fn unix_path_in(text: &str) -> bool {
    text.char_indices().any(|(index, c)| {
        c == '/' && unix_path_after_slash(text[..index].chars().next_back(), &text[index + 1..])
    })
}

/// The left boundary every Unix-path-like form shares (second review): the
/// character before the form is none of an ASCII letter, digit, `.`, `~`,
/// `:`, `/` or `%`.
fn clean_boundary(previous: Option<char>) -> bool {
    previous.is_none_or(|c| !c.is_ascii_alphanumeric() && !matches!(c, '.' | '~' | ':' | '/' | '%'))
}

/// Whether `text` holds a URL-encoded separator (`%2F`, case-insensitive)
/// that begins an encoded path (second review P2, third review: an ASCII
/// letter or a UTF-8 lead byte `%C2`–`%F4` after the code), or another
/// user's home directory `~name/` (P3) — both with the Unix path's left
/// boundary.
fn encoded_or_tilde_path_in(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let encoded = lower.match_indices("%2f").any(|(index, _)| {
        clean_boundary(text[..index].chars().next_back()) && encoded_lead(&text[index + 3..])
    });
    let tilde = text.match_indices('~').any(|(index, _)| {
        clean_boundary(text[..index].chars().next_back())
            && text[index + 1..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
                .is_some_and(|offset| text[index + 1 + offset..].starts_with('/'))
    });
    encoded || tilde
}

/// What may follow a `%2F` separator for it to begin an encoded path
/// (third review): an ASCII letter, or another percent-encoded UTF-8 lead
/// byte `%C2`–`%F4` (case-insensitive).
fn encoded_lead(rest: &str) -> bool {
    match rest.as_bytes().first() {
        Some(b'%') => rest
            .get(1..3)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            .is_some_and(|byte| (0xC2..=0xF4).contains(&byte)),
        Some(first) if first.is_ascii_alphabetic() => true,
        _ => false,
    }
}

pub fn safe_value(value: &Value) -> Result<Value, RunError> {
    match value {
        Value::Object(map) => {
            if insar_agent_model::privacy::is_safe_multimodal_image_block(value) {
                return Ok(value.clone());
            }
            let mut safe = serde_json::Map::new();
            for (key, entry) in map {
                let lower = key.to_ascii_lowercase();
                if [
                    "token",
                    "password",
                    "cookie",
                    "authorization",
                    "secret",
                    "netrc",
                    "api_key",
                    "access_key",
                    "credential",
                ]
                .iter()
                .any(|s| lower.contains(s))
                    && !insar_agent_model::privacy::is_allowed_token_budget_key(&lower)
                {
                    return Err(RunError::Unsafe);
                }
                safe.insert(key.clone(), safe_value(entry)?);
            }
            Ok(Value::Object(safe))
        }
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(safe_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Value::String(text) => {
            let lower = text.to_ascii_lowercase();
            // A marker whose value is already the outbound redaction
            // placeholder carries no secret (scrubbed model request bodies).
            let probe = ["bearer ", "password=", "token=", "cookie="]
                .iter()
                .fold(lower.clone(), |probe, marker| {
                    probe.replace(&format!("{marker}[redacted]"), "")
                });
            if probe.contains("authorization:")
                || probe.contains("bearer ")
                || probe.contains("password=")
                || probe.contains("token=")
                || probe.contains("cookie=")
            {
                return Err(RunError::Unsafe);
            }
            // Only a value that is one web address and nothing else keeps its
            // path part; anything else goes through every check below.
            let single = (text.starts_with("https://") || text.starts_with("http://"))
                && !text.contains(char::is_whitespace);
            let kept = if single {
                text.split(['?', '#']).next().unwrap_or("")
            } else {
                text.as_str()
            };
            let lower = kept.to_ascii_lowercase();
            if lower.contains("signature=") || lower.contains("x-amz-") || lower.contains("x-goog-")
            {
                return Err(RunError::Unsafe);
            }
            // A web URL inside prose or serialized JSON is not a local path,
            // but its query or fragment may carry a signature and its userinfo
            // a credential. Bound each URL at whitespace, quotes, brackets,
            // commas, or JSON escape backslashes so adjacent JSON fields
            // (e.g. `{"url":"https://a.com/x","q":"why?"}` or escaped `\n# H1`)
            // are not mistaken for query/fragment characters of the URL.
            if kept.match_indices("://").any(|(idx, _)| {
                let tail = &kept[idx + 3..];
                let url_end = tail
                    .find(|c: char| {
                        c.is_whitespace()
                            || matches!(c, '"' | '\'' | ',' | '}' | ']' | ')' | '>' | '<' | '\\')
                    })
                    .unwrap_or(tail.len());
                let rest = &tail[..url_end];
                let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
                rest.contains(['?', '#']) || authority.contains('@')
            }) {
                return Err(RunError::Unsafe);
            }
            let stripped = kept.replace("https://", "").replace("http://", "");
            let text = stripped.as_str();
            // Paths may be embedded within prose: conservative refusal is safer
            // than an incomplete regex scrub. Allowed refs must be relative.
            // A Unix path is any `/` that begins one (review 2026-11-02, the
            // scrub's rule: not after a letter, digit, `.`, `~`, `:` or
            // another `/`; not before `/` or whitespace; first segment an
            // ASCII letter, or a non-ASCII one with another `/` after it),
            // and the known local roots (`/home/`, `/Users/`, `/var/`)
            // wherever they sit.
            if text.contains(":\\")
                || text.contains(":/")
                || unc_path_in(text)
                || text.contains("/home/")
                || text.contains("/Users/")
                || text.contains("/var/")
                || text.contains("file://")
                || text.contains(".netrc")
                || unix_path_in(text)
                || encoded_or_tilde_path_in(text)
            {
                return Err(RunError::Unsafe);
            }
            Ok(json!(kept))
        }
        _ => Ok(value.clone()),
    }
}

pub struct RunWriter {
    dir: PathBuf,
    next: u64,
    previous: String,
}
impl RunWriter {
    pub fn start(root: &Path, store: &mut TaskStore, row: &AgentRunRow) -> Result<Self, RunError> {
        if row.record_dir != format!("agent/runs/{}", row.run_id)
            || row.run_id.is_empty()
            || !row
                .run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(RunError::Unsafe);
        }
        let safe_row: AgentRunRow =
            serde_json::from_value(safe_value(&serde_json::to_value(row)?)?)?;
        let dir = root.join(&row.record_dir);
        fs::create_dir_all(dir.parent().ok_or(RunError::Unsafe)?)?;
        fs::create_dir(&dir)?;
        fs::create_dir(dir.join("outbound"))?;
        fs::write(dir.join("run.json"), serde_json::to_vec_pretty(&safe_row)?)?;
        File::create(dir.join("trace.jsonl"))?;
        store.start_agent_run(&safe_row)?;
        Ok(Self {
            dir,
            next: 0,
            previous: "0".repeat(64),
        })
    }
    pub fn append(&mut self, kind: &str, payload: &Value) -> Result<TraceEvent, RunError> {
        if !matches!(
            kind,
            "request"
                | "model_call"
                | "model_resend"
                | "model_reply"
                | "tool_call"
                | "tool_result"
                | "plan"
                | "plan_refused"
                | "tool_denied"
                | "conclusion_unparsed"
                | "outbound_blocked"
                | "permission"
                | "approval"
                | "plan_rewritten"
                | "search_batch"
                | "search_circuit_breaker"
                | "execution"
                | "verification"
                | "gap"
                | "answer"
        ) {
            return Err(RunError::Unsafe);
        }
        let payload = safe_value(payload)?;
        let bytes = serde_json::to_vec(&(self.next, &self.previous, kind, &payload))?;
        let event = TraceEvent {
            sequence: self.next,
            previous_sha256: self.previous.clone(),
            kind: kind.into(),
            payload,
            sha256: sha(&bytes),
        };
        let mut file = OpenOptions::new()
            .append(true)
            .open(self.dir.join("trace.jsonl"))?;
        serde_json::to_writer(&mut file, &event)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        self.next += 1;
        self.previous = event.sha256.clone();
        Ok(event)
    }
    pub fn outbound(&mut self, body: &Value) -> Result<String, RunError> {
        let body = safe_value(body)?;
        let bytes = serde_json::to_vec(&body)?;
        let filename = format!("{:06}.json", self.next);
        let path = self.dir.join("outbound").join(&filename);
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        self.append("model_call", &json!({"file":filename,"sha256":sha(&bytes)}))?;
        Ok(filename)
    }
    pub fn finish(
        self,
        store: &mut TaskStore,
        run_id: &str,
        status: &str,
        ended_at: &str,
        revision: i64,
    ) -> Result<String, RunError> {
        let events = read_trace(&self.dir)?;
        if events.len() as u64 != self.next
            || events.last().map(|event| event.sha256.as_str()) != Some(self.previous.as_str())
        {
            return Err(RunError::Integrity);
        }
        fs::write(self.dir.join("journal.md"), journal(&events))?;
        let row = store.agent_run(run_id)?.ok_or(StoreError::NotFound)?;
        if row.record_dir != format!("agent/runs/{}", run_id) {
            return Err(RunError::Unsafe);
        }
        store.finish_agent_run(
            run_id,
            status,
            ended_at,
            revision,
            &self.previous,
            run_flags(&events),
        )?;
        let sealed = store.agent_run(run_id)?.ok_or(StoreError::NotFound)?;
        let record = enrich_run_record_value(serde_json::to_value(sealed)?, &events);
        fs::write(
            self.dir.join("run.json"),
            serde_json::to_vec_pretty(&safe_value(&record)?)?,
        )?;
        Ok(self.previous)
    }
}

fn sum_trace_usage_key(events: &[TraceEvent], key: &str) -> Option<u64> {
    events
        .iter()
        .filter(|event| event.kind == "model_reply")
        .filter_map(|event| event.payload.pointer(&format!("/usage/{key}"))?.as_u64())
        .fold(None, |sum: Option<u64>, count| {
            Some(sum.unwrap_or(0).saturating_add(count))
        })
}

fn last_trace_usage_prompt(events: &[TraceEvent]) -> Option<u64> {
    events
        .iter()
        .rev()
        .filter(|event| event.kind == "model_reply")
        .find_map(|event| event.payload.pointer("/usage/prompt")?.as_u64())
}

fn enrich_run_record_value(mut value: Value, events: &[TraceEvent]) -> Value {
    if let Some(obj) = value.as_object_mut() {
        if let Some(cached) = sum_trace_usage_key(events, "cached") {
            obj.insert("usage_cached_input".into(), json!(cached));
        }
        if let Some(cache_creation) = sum_trace_usage_key(events, "cache_creation") {
            obj.insert("usage_cache_creation_input".into(), json!(cache_creation));
        }
        if let Some(reasoning) = sum_trace_usage_key(events, "reasoning") {
            obj.insert("usage_reasoning".into(), json!(reasoning));
        }
        if let Some(last_prompt) = last_trace_usage_prompt(events) {
            obj.insert("usage_last_prompt".into(), json!(last_prompt));
        }
    }
    value
}

pub fn run_extra_usage(root: &Path, run_id: &str) -> (Option<u64>, Option<u64>, Option<u64>) {
    let dir = root.join("agent/runs").join(run_id);
    if let Ok(bytes) = fs::read(dir.join("run.json")) {
        if let Ok(record) = serde_json::from_slice::<RunRecord>(&bytes) {
            if record.usage_cached_input.is_some()
                || record.usage_cache_creation_input.is_some()
                || record.usage_reasoning.is_some()
            {
                return (
                    record.usage_cached_input,
                    record.usage_cache_creation_input,
                    record.usage_reasoning,
                );
            }
        }
    }
    if let Ok(events) = read_trace(&dir) {
        return (
            sum_trace_usage_key(&events, "cached"),
            sum_trace_usage_key(&events, "cache_creation"),
            sum_trace_usage_key(&events, "reasoning"),
        );
    }
    (None, None, None)
}

pub fn run_last_prompt(root: &Path, run_id: &str) -> Option<u64> {
    let dir = root.join("agent/runs").join(run_id);
    if let Ok(bytes) = fs::read(dir.join("run.json")) {
        if let Ok(record) = serde_json::from_slice::<RunRecord>(&bytes) {
            if record.usage_last_prompt.is_some() {
                return record.usage_last_prompt;
            }
        }
    }
    read_trace(&dir)
        .ok()
        .and_then(|events| last_trace_usage_prompt(&events))
}

/// What a run record shows about its model replies, for the run row: a
/// `model_resend` (the provider refused the reasoning-off switch and the
/// request was resent without it), a `model_reply` marked `truncated`, and
/// the sums of the `usage` the replies reported (None when none did).
pub fn run_flags(events: &[TraceEvent]) -> AgentRunFlags {
    let usage = |key: &str| sum_trace_usage_key(events, key);
    AgentRunFlags {
        // Only a run that sent the switch and was refused and resent (L4
        // r10 P2-8: runs on a channel already known to refuse it sent no
        // switch, yet each said 「该渠道无法关闭思考」).
        reasoning_off_refused: events.iter().any(|event| event.kind == "model_resend"),
        truncated: events.iter().any(|event| {
            event.kind == "model_reply" && event.payload.get("truncated") == Some(&json!(true))
        }),
        usage_prompt: usage("prompt"),
        usage_completion: usage("completion"),
    }
}

pub fn read_trace(dir: &Path) -> Result<Vec<TraceEvent>, RunError> {
    let file = File::open(dir.join("trace.jsonl"))?;
    let mut events = Vec::new();
    let mut previous = "0".repeat(64);
    for line in BufReader::new(file).lines() {
        let event: TraceEvent = serde_json::from_str(&line?)?;
        if event.sequence != events.len() as u64
            || event.previous_sha256 != previous
            || safe_value(&event.payload)? != event.payload
            || event.sha256
                != sha(&serde_json::to_vec(&(
                    event.sequence,
                    &event.previous_sha256,
                    &event.kind,
                    &event.payload,
                ))?)
        {
            return Err(RunError::Integrity);
        }
        previous = event.sha256.clone();
        events.push(event);
    }
    Ok(events)
}
pub fn read_sealed(
    root: &Path,
    store: &TaskStore,
    run_id: &str,
) -> Result<Vec<TraceEvent>, RunError> {
    let row = store.agent_run(run_id)?.ok_or(StoreError::NotFound)?;
    if row.record_dir != format!("agent/runs/{run_id}")
        || run_id.is_empty()
        || !run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(RunError::Unsafe);
    }
    let events = read_trace(&root.join(&row.record_dir))?;
    let expected = events.last().map(|e| e.sha256.as_str()).unwrap_or("");
    if row.status != "running" && row.trace_sha256.as_deref() != Some(expected) {
        return Err(RunError::Integrity);
    }
    Ok(events)
}

pub fn journal(events: &[TraceEvent]) -> String {
    let mut lines = vec!["# Agent run journal".to_string(), String::new()];
    for event in events {
        lines.push(format!(
            "- {} {} {}",
            event.sequence,
            event.kind,
            serde_json::to_string(&event.payload).unwrap_or_default()
        ));
    }
    lines.join("\n") + "\n"
}

pub fn delete_run(root: &Path, store: &mut TaskStore, run_id: &str) -> Result<(), RunError> {
    let row = store.agent_run(run_id)?.ok_or(StoreError::NotFound)?;
    if run_id.is_empty()
        || !run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || row.record_dir != format!("agent/runs/{run_id}")
    {
        return Err(RunError::Unsafe);
    }
    let base = root.join("agent").join("runs");
    let dir = base.join(run_id);
    if root.is_symlink() || root.join("agent").is_symlink() || base.is_symlink() || dir.is_symlink()
    {
        return Err(RunError::Unsafe);
    }
    // Only remove the generated files in this one run directory. Never follow
    // attacker-provided symlinks/junctions and never walk an output/task tree.
    for filename in [
        "run.json",
        "trace.jsonl",
        "journal.md",
        "turn_messages.json",
        "turn_messages.json.tmp",
    ] {
        let file = dir.join(filename);
        if file.is_symlink() {
            return Err(RunError::Unsafe);
        }
        if file.exists() {
            fs::remove_file(file)?;
        }
    }
    let outbound = dir.join("outbound");
    if outbound.is_symlink() {
        return Err(RunError::Unsafe);
    }
    if outbound.is_dir() {
        for entry in fs::read_dir(&outbound)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.ends_with(".json")
                || !name[..name.len() - 5].bytes().all(|b| b.is_ascii_digit())
                || entry.file_type()?.is_symlink()
                || !entry.file_type()?.is_file()
            {
                return Err(RunError::Unsafe);
            }
            fs::remove_file(entry.path())?;
        }
        fs::remove_dir(outbound)?;
    }
    fs::remove_dir(dir)?;
    store.delete_agent_run(run_id)?;
    Ok(())
}

pub fn mark_interrupted(
    root: &Path,
    store: &mut TaskStore,
    at: &str,
) -> Result<Vec<String>, RunError> {
    let ids = store.interrupt_agent_runs(at)?;
    for id in &ids {
        let dir = root.join("agent/runs").join(id);
        if let Ok(events) = read_trace(&dir) {
            fs::write(dir.join("journal.md"), journal(&events))?;
            if let Some(last) = events.last() {
                store.set_interrupted_trace_hash(id, &last.sha256, run_flags(&events))?;
            }
            if let Some(row) = store.agent_run(id)? {
                let record = enrich_run_record_value(serde_json::to_value(row)?, &events);
                fs::write(
                    dir.join("run.json"),
                    serde_json::to_vec_pretty(&safe_value(&record)?)?,
                )?;
            }
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static RUN_TEST_SEQ: AtomicU64 = AtomicU64::new(1);

    fn make_event(seq: u64, prev: &str, kind: &str, payload: Value) -> TraceEvent {
        let sha_val =
            sha(&serde_json::to_vec(&(seq, &prev.to_owned(), &kind.to_owned(), &payload)).unwrap());
        TraceEvent {
            sequence: seq,
            previous_sha256: prev.to_owned(),
            kind: kind.to_owned(),
            payload,
            sha256: sha_val,
        }
    }

    #[test]
    fn run_extra_usage_extracts_last_prompt_for_new_and_legacy_runs() {
        let root = std::env::temp_dir().join(format!(
            "eostage-run-last-prompt-{}-{}",
            std::process::id(),
            RUN_TEST_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let run_id = "run-multi-call-test";
        let dir = root.join("agent/runs").join(run_id);
        fs::create_dir_all(&dir).unwrap();

        let zero = "0".repeat(64);
        let ev0 = make_event(
            0,
            &zero,
            "model_reply",
            json!({"usage": {"prompt": 13358u64, "completion": 270u64, "cached": 64u64, "reasoning": 204u64}}),
        );
        let ev1 = make_event(
            1,
            &ev0.sha256,
            "model_reply",
            json!({"usage": {"prompt": 102371u64, "completion": 2122u64, "cached": 98624u64, "reasoning": 1093u64}}),
        );
        let events = vec![ev0, ev1];
        assert_eq!(last_trace_usage_prompt(&events), Some(102_371));

        let trace_text = events
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(dir.join("trace.jsonl"), trace_text).unwrap();

        // 1. Legacy run.json (has usage_cached_input, lacks usage_last_prompt): falls back to trace.jsonl
        let legacy_json = json!({
            "run_id": run_id,
            "session_id": "sess-1",
            "status": "completed",
            "started_at": "2026-11-16T00:00:00Z",
            "usage_prompt": 576_653u64,
            "usage_completion": 9_339u64,
            "usage_cached_input": 363_840u64,
            "usage_reasoning": 7_063u64,
        });
        fs::write(
            dir.join("run.json"),
            serde_json::to_vec_pretty(&legacy_json).unwrap(),
        )
        .unwrap();
        let (cached, _, reasoning) = run_extra_usage(&root, run_id);
        let last_prompt = run_last_prompt(&root, run_id);
        assert_eq!(cached, Some(363_840));
        assert_eq!(reasoning, Some(7_063));
        assert_eq!(last_prompt, Some(102_371));

        // 2. Enriched new run.json writes usage_last_prompt directly
        let enriched = enrich_run_record_value(legacy_json, &events);
        assert_eq!(enriched["usage_last_prompt"], json!(102_371u64));
        fs::write(
            dir.join("run.json"),
            serde_json::to_vec_pretty(&enriched).unwrap(),
        )
        .unwrap();
        let fast_last_prompt = run_last_prompt(&root, run_id);
        assert_eq!(fast_last_prompt, Some(102_371));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn safe_value_allows_output_token_budgets_and_blocks_real_secret_tokens() {
        use insar_agent_model::{transport::ModelRoute, Message, Protocol};

        let allowed = json!({
            "max_tokens": 4096,
            "budget_tokens": 1024,
            "max_output_tokens": 16384,
            "max_completion_tokens": 8192,
            "generationConfig": { "maxOutputTokens": 16384 },
        });
        assert_eq!(
            safe_value(&allowed).expect("token budgets allowed"),
            allowed
        );

        for secret_key in [
            "token",
            "api_token",
            "access_token",
            "refresh_token",
            "secret_key",
        ] {
            let bad = json!({ secret_key: "opaque-value" });
            assert!(
                matches!(safe_value(&bad), Err(RunError::Unsafe)),
                "expected secret key `{secret_key}` to remain blocked"
            );
        }

        let msgs = vec![
            Message {
                role: "system".into(),
                content: "你是 EOStage 遥感科研助手。".into(),
            },
            Message {
                role: "user".into(),
                content: "写一段 InSAR 公式".into(),
            },
        ];

        for (protocol, url) in [
            (Protocol::Responses, "https://example.invalid/v1"),
            (
                Protocol::GoogleGemini,
                "https://generativelanguage.googleapis.com/v1beta",
            ),
        ] {
            let route = ModelRoute::new(protocol, url, "test-model")
                .expect("route")
                .with_max_output_tokens(Some(16384));
            let (_, body) = route.request(&msgs, &[]).expect("request body");
            assert_eq!(
                safe_value(&body).expect("outbound safe_value must accept model route budget"),
                body
            );
        }
    }
}
