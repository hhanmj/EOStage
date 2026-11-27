//! Always-on stream reasoning & inline `<think>` / `<thinking>` normalizer.
//!
//! Normalizes heterogeneous provider streams across OpenAI Chat Completions,
//! OpenAI Responses, Anthropic Messages, OpenRouter, LiteLLM, vLLM, Ollama,
//! DeepSeek, Qwen, GLM, and `agnes-*` models:
//! - Dedicated SSE reasoning deltas (`reasoning_content`, `reasoning`, `thought`,
//!   `thinking`, Responses `reasoning*_text.delta`, Anthropic `thinking_delta`);
//! - Inline `<think>...</think>` and `<thinking>...</thinking>` (case-insensitive)
//!   with a 12-byte chunk-boundary lookahead tail buffer;
//! - Orphan closing `</think>` / `</thinking>` recovery within the first 8192 bytes
//!   before any triple-backtick code fence;
//! - Unclosed `<think>` quarantine on `finish()` (never leaks into answer `text`);
//! - Triple-backtick (```` ``` ````) code-fence immunity in content;
//! - Monotonic `thinking_ms` timer from first reasoning delta to first answer/tool
//!   delta or `finish()`.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::ModelEvent;

/// Maximum lookahead bytes held across chunk boundaries for split tags or fences.
pub const MAX_LOOKAHEAD_BYTES: usize = 12;

/// Maximum leading whitespace bytes held at byte 0 before deciding no `<think>` opened.
const MAX_LEADING_WS_BYTES: usize = 64;

/// Maximum content prefix byte window eligible for orphan closing `</think>` recovery.
pub const ORPHAN_CLOSE_WINDOW_BYTES: usize = 8192;

const OPEN_TAGS: &[&str] = &["<thinking>", "<think>"];
const CLOSE_TAGS: &[&str] = &["</thinking>", "</think>"];
const OPEN_AND_CLOSE_TAGS: &[&str] = &["<thinking>", "<think>", "</thinking>", "</think>"];

/// Leading parenthetical/bracketed reasoning blocks emitted by `agnes-*` and
/// some Chinese chat models (`（思考：...）` with balanced nested parentheses).
const BRACKET_THINK_PREFIXES: &[(&str, char, char)] = &[
    ("（思考：", '（', '）'),
    ("（思考:", '（', '）'),
    ("(思考：", '(', ')'),
    ("(思考:", '(', ')'),
    ("【思考：", '【', '】'),
    ("【思考:", '【', '】'),
    ("[思考：", '[', ']'),
    ("[思考:", '[', ']'),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Start of content stream: optional whitespace before an opening `<think>`.
    LeadingWhitespace,
    /// Inside an inline `<think>...</think>` or `<thinking>...</thinking>` block.
    InThink,
    /// Inside a balanced parenthetical/bracketed thought block (e.g. `（思考：...）`).
    InBracketThink {
        open_char: char,
        close_char: char,
        depth: usize,
    },
    /// Immediately after a closing `</think>` / `</thinking>` / `）` tag.
    AfterThink,
    /// Normal answer content; `fence_ticks` is `Some(n)` (`n >= 3`) inside a fenced block.
    InContent { fence_ticks: Option<usize> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NormalizedOutput {
    pub text: String,
    pub reasoning_text: String,
    pub reasoning_chars: u64,
    pub thinking_ms: Option<u64>,
    pub think_block_unclosed: bool,
    pub orphan_think_reclassified: bool,
}

pub struct ThinkNormalizer {
    state: State,
    tail: String,
    text: String,
    reasoning_text: String,
    reasoning_chars: u64,
    first_delta_at: Option<Instant>,
    span_started_at: Option<Instant>,
    accumulated_thinking: Duration,
    thinking_active: bool,
    has_reasoned: bool,
    has_opened_think: bool,
    seen_code_fence: bool,
    orphan_think_reclassified: bool,
    think_block_unclosed: bool,
}

impl Default for ThinkNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinkNormalizer {
    pub fn new() -> Self {
        Self {
            state: State::LeadingWhitespace,
            tail: String::new(),
            text: String::new(),
            reasoning_text: String::new(),
            reasoning_chars: 0,
            first_delta_at: None,
            span_started_at: None,
            accumulated_thinking: Duration::ZERO,
            thinking_active: false,
            has_reasoned: false,
            has_opened_think: false,
            seen_code_fence: false,
            orphan_think_reclassified: false,
            think_block_unclosed: false,
        }
    }

    pub fn text_len(&self) -> usize {
        self.text.len().saturating_add(self.tail.len())
    }

    pub fn reasoning_len(&self) -> usize {
        self.reasoning_text.len()
    }

    /// Feed a dedicated reasoning delta (`reasoning_content`, `reasoning`, `thought`,
    /// `thinking`, Responses `reasoning*_text.delta`, Anthropic `thinking_delta`).
    pub fn push_reasoning(&mut self, chunk: &str, emit: &mut impl FnMut(ModelEvent)) {
        if chunk.is_empty() {
            return;
        }
        self.mark_first_delta();
        self.emit_reasoning_delta(chunk, emit);
    }

    /// Feed a content delta (`delta.content`, `output_text.delta`, `text_delta`).
    pub fn push_content(&mut self, chunk: &str, emit: &mut impl FnMut(ModelEvent)) {
        if chunk.is_empty() {
            return;
        }
        self.mark_first_delta();
        self.process_content(chunk, false, emit);
    }

    /// Notify that a tool-call delta has arrived so any open thinking timer stops
    /// before tool-argument generation time is counted.
    pub fn on_tool_delta(&mut self, emit: &mut impl FnMut(ModelEvent)) {
        if matches!(self.state, State::InThink | State::InBracketThink { .. })
            && !self.tail.is_empty()
        {
            let pending = std::mem::take(&mut self.tail);
            self.emit_reasoning_delta(&pending, emit);
        }
        self.close_thinking_span(emit);
    }

    /// Flush any held lookahead buffer at end-of-stream, quarantining unclosed
    /// `<think>` blocks into `reasoning_text` without ever leaking into `text`.
    pub fn finish(mut self, emit: &mut impl FnMut(ModelEvent)) -> NormalizedOutput {
        self.process_content("", true, emit);
        self.close_thinking_span(emit);
        let thinking_ms = self
            .has_reasoned
            .then(|| (self.accumulated_thinking.as_millis() as u64).max(1));
        NormalizedOutput {
            text: self.text,
            reasoning_text: self.reasoning_text,
            reasoning_chars: self.reasoning_chars,
            thinking_ms,
            think_block_unclosed: self.think_block_unclosed,
            orphan_think_reclassified: self.orphan_think_reclassified,
        }
    }

    fn mark_first_delta(&mut self) {
        if self.first_delta_at.is_none() {
            self.first_delta_at = Some(Instant::now());
        }
    }

    fn ensure_thinking_started(
        &mut self,
        start_override: Option<Instant>,
        emit: &mut impl FnMut(ModelEvent),
    ) {
        if !self.thinking_active {
            self.thinking_active = true;
            self.span_started_at = Some(start_override.unwrap_or_else(Instant::now));
            emit(ModelEvent::ThinkingStart {
                started_at_ms: now_epoch_ms(),
            });
        }
    }

    fn emit_reasoning_delta(&mut self, delta: &str, emit: &mut impl FnMut(ModelEvent)) {
        if delta.is_empty() {
            return;
        }
        self.ensure_thinking_started(None, emit);
        self.has_reasoned = true;
        self.reasoning_chars = self
            .reasoning_chars
            .saturating_add(delta.chars().count() as u64);
        self.reasoning_text.push_str(delta);
        emit(ModelEvent::ThinkingDelta(delta.to_owned()));
    }

    fn close_thinking_span(&mut self, emit: &mut impl FnMut(ModelEvent)) {
        if self.thinking_active {
            self.thinking_active = false;
            if let Some(started) = self.span_started_at.take() {
                self.accumulated_thinking =
                    self.accumulated_thinking.saturating_add(started.elapsed());
            }
            let duration_ms = (self.accumulated_thinking.as_millis() as u64).max(1);
            emit(ModelEvent::ThinkingEnd { duration_ms });
        }
    }

    fn emit_text(&mut self, slice: &str, emit: &mut impl FnMut(ModelEvent)) {
        if slice.is_empty() {
            return;
        }
        if self.thinking_active && !slice.trim().is_empty() {
            self.close_thinking_span(emit);
        } else if self.thinking_active {
            // Close before emitting any text event so ThinkingStart..ThinkingEnd never interleaves Text.
            self.close_thinking_span(emit);
        }
        self.text.push_str(slice);
        emit(ModelEvent::Text(slice.to_owned()));
    }

    fn orphan_eligible(&self) -> bool {
        !self.has_opened_think
            && !self.orphan_think_reclassified
            && !self.seen_code_fence
            && self.text.len() <= ORPHAN_CLOSE_WINDOW_BYTES
    }

    fn process_content(&mut self, input: &str, is_eof: bool, emit: &mut impl FnMut(ModelEvent)) {
        match self.state {
            State::LeadingWhitespace => {
                self.tail.push_str(input);
                let trimmed = self.tail.trim_start();
                if trimmed.is_empty() {
                    if is_eof {
                        let ws = std::mem::take(&mut self.tail);
                        self.state = State::InContent { fence_ticks: None };
                        self.emit_text(&ws, emit);
                    } else if self.tail.len() > MAX_LEADING_WS_BYTES {
                        let ws = std::mem::take(&mut self.tail);
                        self.state = State::InContent { fence_ticks: None };
                        self.process_in_content(&ws, None, false, emit);
                    }
                    return;
                }
                if let Some(open_len) = match_any_tag_at_start(trimmed, OPEN_TAGS) {
                    let rest = trimmed[open_len..].to_owned();
                    self.tail.clear();
                    self.has_opened_think = true;
                    self.state = State::InThink;
                    if !rest.is_empty() || is_eof {
                        self.process_content(&rest, is_eof, emit);
                    }
                    return;
                }
                if let Some((open_len, open_char, close_char)) =
                    match_bracket_think_at_start(trimmed)
                {
                    let rest = trimmed[open_len..].to_owned();
                    self.tail.clear();
                    self.has_opened_think = true;
                    self.state = State::InBracketThink {
                        open_char,
                        close_char,
                        depth: 1,
                    };
                    if !rest.is_empty() || is_eof {
                        self.process_content(&rest, is_eof, emit);
                    }
                    return;
                }
                if !is_eof
                    && (is_proper_prefix_of_any(trimmed, OPEN_AND_CLOSE_TAGS)
                        || is_proper_prefix_of_bracket_think(trimmed))
                {
                    return;
                }
                let buffered = std::mem::take(&mut self.tail);
                self.state = State::InContent { fence_ticks: None };
                self.process_in_content(&buffered, None, is_eof, emit);
            }
            State::InThink => {
                let mut combined = std::mem::take(&mut self.tail);
                combined.push_str(input);
                if combined.is_empty() && !is_eof {
                    return;
                }
                if let Some((pos, close_len)) = find_earliest_tag(&combined, CLOSE_TAGS) {
                    let thought_part = &combined[..pos];
                    if !thought_part.is_empty() {
                        self.emit_reasoning_delta(thought_part, emit);
                    }
                    self.close_thinking_span(emit);
                    let rest = &combined[pos + close_len..];
                    self.state = State::AfterThink;
                    if !rest.is_empty() || is_eof {
                        self.process_content(rest, is_eof, emit);
                    }
                } else if is_eof {
                    self.think_block_unclosed = true;
                    if !combined.is_empty() {
                        self.emit_reasoning_delta(&combined, emit);
                    }
                    self.close_thinking_span(emit);
                } else {
                    let hold = trailing_hold_bytes(&combined, CLOSE_TAGS, false);
                    let safe_end = combined.len() - hold;
                    if safe_end > 0 {
                        self.emit_reasoning_delta(&combined[..safe_end], emit);
                    }
                    self.tail.push_str(&combined[safe_end..]);
                }
            }
            State::InBracketThink {
                open_char,
                close_char,
                mut depth,
            } => {
                let mut combined = std::mem::take(&mut self.tail);
                combined.push_str(input);
                if combined.is_empty() && !is_eof {
                    return;
                }
                let mut closed_at: Option<(usize, usize)> = None;
                for (idx, ch) in combined.char_indices() {
                    if ch == open_char {
                        depth = depth.saturating_add(1);
                    } else if ch == close_char {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            closed_at = Some((idx, ch.len_utf8()));
                            break;
                        }
                    }
                }
                if let Some((pos, close_len)) = closed_at {
                    let thought_part = &combined[..pos];
                    if !thought_part.is_empty() {
                        self.emit_reasoning_delta(thought_part, emit);
                    }
                    self.close_thinking_span(emit);
                    let rest = &combined[pos + close_len..];
                    self.state = State::AfterThink;
                    if !rest.is_empty() || is_eof {
                        self.process_content(rest, is_eof, emit);
                    }
                } else {
                    self.state = State::InBracketThink {
                        open_char,
                        close_char,
                        depth,
                    };
                    if !combined.is_empty() {
                        self.emit_reasoning_delta(&combined, emit);
                    }
                    if is_eof {
                        self.think_block_unclosed = true;
                        self.close_thinking_span(emit);
                    }
                }
            }
            State::AfterThink => {
                let mut combined = std::mem::take(&mut self.tail);
                combined.push_str(input);
                let trim_leading = self.text.is_empty() || self.text.ends_with(char::is_whitespace);
                if trim_leading {
                    let trimmed = combined.trim_start();
                    if trimmed.is_empty() {
                        return;
                    }
                    if let Some(open_len) = match_any_tag_at_start(trimmed, OPEN_TAGS) {
                        if !self.reasoning_text.is_empty()
                            && !self.reasoning_text.ends_with('\n')
                            && !self.thinking_active
                        {
                            self.reasoning_text.push('\n');
                        }
                        let rest = trimmed[open_len..].to_owned();
                        self.has_opened_think = true;
                        self.state = State::InThink;
                        if !rest.is_empty() || is_eof {
                            self.process_content(&rest, is_eof, emit);
                        }
                        return;
                    }
                    if let Some((open_len, open_char, close_char)) =
                        match_bracket_think_at_start(trimmed)
                    {
                        if !self.reasoning_text.is_empty()
                            && !self.reasoning_text.ends_with('\n')
                            && !self.thinking_active
                        {
                            self.reasoning_text.push('\n');
                        }
                        let rest = trimmed[open_len..].to_owned();
                        self.has_opened_think = true;
                        self.state = State::InBracketThink {
                            open_char,
                            close_char,
                            depth: 1,
                        };
                        if !rest.is_empty() || is_eof {
                            self.process_content(&rest, is_eof, emit);
                        }
                        return;
                    }
                    if !is_eof
                        && (is_proper_prefix_of_any(trimmed, OPEN_TAGS)
                            || is_proper_prefix_of_bracket_think(trimmed))
                    {
                        self.tail.push_str(trimmed);
                        return;
                    }
                    let rest = trimmed.to_owned();
                    self.state = State::InContent { fence_ticks: None };
                    self.process_in_content(&rest, None, is_eof, emit);
                } else {
                    self.state = State::InContent { fence_ticks: None };
                    self.process_in_content(&combined, None, is_eof, emit);
                }
            }
            State::InContent { fence_ticks } => {
                let mut combined = std::mem::take(&mut self.tail);
                combined.push_str(input);
                self.process_in_content(&combined, fence_ticks, is_eof, emit);
            }
        }
    }

    fn process_in_content(
        &mut self,
        combined: &str,
        mut fence_ticks: Option<usize>,
        is_eof: bool,
        emit: &mut impl FnMut(ModelEvent),
    ) {
        if combined.is_empty() {
            self.state = State::InContent { fence_ticks };
            return;
        }
        let hold = if is_eof {
            0
        } else if fence_ticks.is_some() {
            trailing_hold_bytes(combined, &[], true)
        } else if self.orphan_eligible() {
            trailing_hold_bytes(combined, OPEN_AND_CLOSE_TAGS, true)
        } else {
            trailing_hold_bytes(combined, OPEN_TAGS, true)
        };

        let safe_end = combined.len() - hold;
        let processable = &combined[..safe_end];
        let tail_part = &combined[safe_end..];

        let bytes = processable.as_bytes();
        let emit_start = 0usize;
        let mut i = 0usize;

        while i < bytes.len() {
            if bytes[i] == b'`' {
                let mut run = 1usize;
                while i + run < bytes.len() && bytes[i + run] == b'`' {
                    run += 1;
                }
                if run >= 3 {
                    match fence_ticks {
                        None => {
                            fence_ticks = Some(run);
                            self.seen_code_fence = true;
                        }
                        Some(open_n) if run >= open_n => {
                            fence_ticks = None;
                        }
                        Some(_) => {}
                    }
                }
                i += run;
                continue;
            }

            if fence_ticks.is_none() && bytes[i] == b'<' {
                let rest_processable = &processable[i..];
                if let Some(open_len) = match_any_tag_at_start(rest_processable, OPEN_TAGS) {
                    self.emit_text(&processable[emit_start..i], emit);
                    if !self.reasoning_text.is_empty()
                        && !self.reasoning_text.ends_with('\n')
                        && !self.thinking_active
                    {
                        self.reasoning_text.push('\n');
                    }
                    self.has_opened_think = true;
                    self.state = State::InThink;
                    let remainder = format!("{}{tail_part}", &processable[i + open_len..]);
                    self.process_content(&remainder, is_eof, emit);
                    return;
                }

                if self.orphan_eligible()
                    && self.text.len().saturating_add(i - emit_start) <= ORPHAN_CLOSE_WINDOW_BYTES
                {
                    if let Some(close_len) = match_any_tag_at_start(rest_processable, CLOSE_TAGS) {
                        let mut orphan_thought = std::mem::take(&mut self.text);
                        orphan_thought.push_str(&processable[emit_start..i]);
                        self.orphan_think_reclassified = true;
                        if !orphan_thought.is_empty() {
                            let start = self.first_delta_at.unwrap_or_else(Instant::now);
                            self.ensure_thinking_started(Some(start), emit);
                            self.emit_reasoning_delta(&orphan_thought, emit);
                            self.close_thinking_span(emit);
                        }
                        self.state = State::AfterThink;
                        let remainder = format!("{}{tail_part}", &processable[i + close_len..]);
                        self.process_content(&remainder, is_eof, emit);
                        return;
                    }
                }
            }

            let ch_len = processable[i..].chars().next().map_or(1, char::len_utf8);
            i += ch_len;
        }

        self.emit_text(&processable[emit_start..], emit);
        self.state = State::InContent { fence_ticks };
        self.tail.push_str(tail_part);
    }
}

/// Batch helper that strips `<think>` / `<thinking>` blocks (including orphan
/// closing tags and unclosed `<think>` quarantine) while respecting triple-backtick
/// code fences.
pub fn strip_think_blocks(text: &str) -> NormalizedOutput {
    let mut norm = ThinkNormalizer::new();
    norm.push_content(text, &mut |_| {});
    norm.finish(&mut |_| {})
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn match_bracket_think_at_start(text: &str) -> Option<(usize, char, char)> {
    for &(prefix, open_c, close_c) in BRACKET_THINK_PREFIXES {
        if text.starts_with(prefix) {
            return Some((prefix.len(), open_c, close_c));
        }
    }
    None
}

fn is_proper_prefix_of_bracket_think(candidate: &str) -> bool {
    if candidate.is_empty() || candidate.len() > MAX_LOOKAHEAD_BYTES {
        return false;
    }
    BRACKET_THINK_PREFIXES
        .iter()
        .any(|&(pat, _, _)| candidate.len() < pat.len() && pat.starts_with(candidate))
}

fn match_any_tag_at_start(text: &str, tags: &[&str]) -> Option<usize> {
    let bytes = text.as_bytes();
    for tag in tags {
        let tb = tag.as_bytes();
        if bytes.len() >= tb.len() && bytes[..tb.len()].eq_ignore_ascii_case(tb) {
            return Some(tb.len());
        }
    }
    None
}

fn find_earliest_tag(text: &str, tags: &[&str]) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'<' {
            for tag in tags {
                let tb = tag.as_bytes();
                if bytes.len() - i >= tb.len() && bytes[i..i + tb.len()].eq_ignore_ascii_case(tb) {
                    return Some((i, tb.len()));
                }
            }
        }
    }
    None
}

fn is_proper_prefix_of_any(candidate: &str, patterns: &[&str]) -> bool {
    let cb = candidate.as_bytes();
    if cb.is_empty() || cb.len() > MAX_LOOKAHEAD_BYTES {
        return false;
    }
    patterns.iter().any(|pat| {
        let pb = pat.as_bytes();
        cb.len() < pb.len() && pb[..cb.len()].eq_ignore_ascii_case(cb)
    })
}

/// Computes how many trailing ASCII bytes (`<= MAX_LOOKAHEAD_BYTES`, i.e. 12)
/// must be held back because they form an incomplete tag prefix or trailing
/// backtick run across SSE chunk boundaries.
fn trailing_hold_bytes(text: &str, patterns: &[&str], hold_backticks: bool) -> usize {
    let bytes = text.as_bytes();
    let max_k = MAX_LOOKAHEAD_BYTES.min(bytes.len());
    let mut tag_hold = 0usize;
    for k in (1..=max_k).rev() {
        let suffix = &bytes[bytes.len() - k..];
        if patterns.iter().any(|pat| {
            let pb = pat.as_bytes();
            k < pb.len() && pb[..k].eq_ignore_ascii_case(suffix)
        }) {
            tag_hold = k;
            break;
        }
    }
    let bt_hold = if hold_backticks {
        bytes
            .iter()
            .rev()
            .take_while(|&&b| b == b'`')
            .count()
            .min(MAX_LOOKAHEAD_BYTES)
    } else {
        0
    };
    tag_hold.max(bt_hold)
}
