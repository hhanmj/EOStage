use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::{
    channels::ChannelMemory,
    privacy,
    stream::{strip_json_trailing_commas, strip_surrounding_code_fence},
    think_normalizer,
    transport::{ModelClient, ModelRoute},
    Completion, ErrorKind, Message, ModelError, ModelEvent, ToolCall, ToolDescription,
};

/// Borrow the key only for this call from W3a's credential service; never persist it in a checkpoint.
pub struct CredentialedModel<'a> {
    pub client: &'a ModelClient,
    pub route: &'a ModelRoute,
    pub key: &'a str,
}

impl Model for CredentialedModel<'_> {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [ToolDescription],
        cancel: &'a CancellationToken,
        emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ModelError>> + Send + 'a>> {
        Box::pin(
            self.client
                .send(self.route, self.key, messages, tools, cancel, emit),
        )
    }
    fn request_body(
        &self,
        messages: &[Message],
        tools: &[ToolDescription],
    ) -> Option<Result<Value, ModelError>> {
        Some(self.route.request(messages, tools).map(|(_, body)| body))
    }
    fn without_reasoning_switch(&self) -> Option<Box<dyn Model + Send + '_>> {
        ResendModel::of(self.client, self.route, self.key)
    }
    fn reasoning_switch_unsupported(&self) -> bool {
        self.route.reasoning_switch_refused()
    }
}

/// The same route known to refuse its reasoning switch (so without it, and
/// with the larger budget where one is sent), for the one resend after the
/// provider refused it. Offers no further resend.
struct ResendModel<'a> {
    client: &'a ModelClient,
    route: ModelRoute,
    key: &'a str,
}

impl<'a> ResendModel<'a> {
    fn of(
        client: &'a ModelClient,
        route: &ModelRoute,
        key: &'a str,
    ) -> Option<Box<dyn Model + Send + 'a>> {
        route.sends_reasoning_switch().then(|| {
            Box::new(Self {
                client,
                route: route.clone().with_reasoning_switch_unsupported(true),
                key,
            }) as Box<dyn Model + Send + 'a>
        })
    }
}

impl Model for ResendModel<'_> {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [ToolDescription],
        cancel: &'a CancellationToken,
        emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ModelError>> + Send + 'a>> {
        Box::pin(
            self.client
                .send(&self.route, self.key, messages, tools, cancel, emit),
        )
    }
    fn request_body(
        &self,
        messages: &[Message],
        tools: &[ToolDescription],
    ) -> Option<Result<Value, ModelError>> {
        Some(self.route.request(messages, tools).map(|(_, body)| body))
    }
}

/// Owned variant for one background run: the key is read from the system
/// credential store when the run starts and wiped from memory when the run
/// drops it.
pub struct KeyedModel {
    client: ModelClient,
    route: ModelRoute,
    key: Zeroizing<String>,
    memory: Option<ChannelMemory>,
    /// The provider refused the switch during this run (W3o patch: set by
    /// `remember_reasoning_switch_unsupported` after the resend completed,
    /// so the body archived for a call is the body sent for it); from then
    /// on the run's requests carry no switch.
    refused_in_run: AtomicBool,
    /// The provider rejected `prompt_cache_key` / `cache_control` during this
    /// run and succeeded after stripping it; skip sending it for the rest of the run.
    cache_key_refused_in_run: AtomicBool,
}

impl KeyedModel {
    pub fn new(client: ModelClient, route: ModelRoute, key: Zeroizing<String>) -> Self {
        Self {
            client,
            route,
            key,
            memory: None,
            refused_in_run: AtomicBool::new(false),
            cache_key_refused_in_run: AtomicBool::new(false),
        }
    }

    /// The route this call uses: the opened one, known to refuse the switch
    /// once this run saw it refused.
    fn route(&self) -> ModelRoute {
        let mut route = self.route.clone();
        if self.refused_in_run.load(Ordering::SeqCst) {
            route = route.with_reasoning_switch_unsupported(true);
        }
        if self.cache_key_refused_in_run.load(Ordering::SeqCst) {
            route = route.with_prompt_cache_key(None);
        }
        route
    }

    /// Writes a refused reasoning switch back to the channel the route was
    /// opened from (`remember_reasoning_switch_unsupported`).
    pub fn remembering(mut self, memory: ChannelMemory) -> Self {
        self.memory = Some(memory);
        self
    }
}

impl Model for KeyedModel {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [ToolDescription],
        cancel: &'a CancellationToken,
        emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ModelError>> + Send + 'a>> {
        let route = self.route();
        Box::pin(async move {
            let outcome = self
                .client
                .send(&route, &self.key, messages, tools, cancel, emit)
                .await;
            if let Ok(ref completion) = outcome {
                if completion.prompt_cache_key_refused {
                    self.remember_prompt_cache_key_refused();
                }
            }
            outcome
        })
    }
    fn request_body(
        &self,
        messages: &[Message],
        tools: &[ToolDescription],
    ) -> Option<Result<Value, ModelError>> {
        Some(self.route().request(messages, tools).map(|(_, body)| body))
    }
    fn without_reasoning_switch(&self) -> Option<Box<dyn Model + Send + '_>> {
        ResendModel::of(&self.client, &self.route(), &self.key)
    }
    fn reasoning_switch_unsupported(&self) -> bool {
        self.route().reasoning_switch_refused()
    }
    /// For the rest of this run first, then for the channel. Writing the
    /// channel is best effort: a file that cannot be written only means the
    /// next run is refused and resent once more.
    fn remember_reasoning_switch_unsupported(&self) {
        self.refused_in_run.store(true, Ordering::SeqCst);
        if let Some(memory) = &self.memory {
            let _ = memory.remember_reasoning_switch_unsupported();
        }
    }
    fn remember_prompt_cache_key_refused(&self) {
        self.cache_key_refused_in_run.store(true, Ordering::SeqCst);
    }
}

/// W3a implements this trait: it alone checks tool catalog, tier, approval and run records.
pub trait Dispatcher: Sync {
    fn execute<'a>(
        &'a self,
        call: &'a ToolCall,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>>;

    fn accepts_unoffered_calls(&self) -> bool {
        false
    }
}

/// Model and executor interfaces intentionally do not own Store or credentials.
pub trait Model: Sync {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [ToolDescription],
        cancel: &'a CancellationToken,
        emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ModelError>> + Send + 'a>>;

    /// The exact, already scrubbed body `complete` sends for these inputs, so
    /// the host can archive it in the run record before the call. `None`
    /// only for models that send nothing over the network. No default: a
    /// new networked model must not silently skip the archive.
    fn request_body(
        &self,
        messages: &[Message],
        tools: &[ToolDescription],
    ) -> Option<Result<Value, ModelError>>;

    /// After `complete` failed with `ErrorKind::ReasoningSwitchRefused`: the
    /// same model without its reasoning switch, for exactly one resend. The
    /// caller archives the returned model's `request_body` before calling its
    /// `complete`, and marks the completion `reasoning_off_refused`. `None`
    /// when this model sends no switch (the refusal is then final).
    fn without_reasoning_switch(&self) -> Option<Box<dyn Model + Send + '_>> {
        None
    }

    /// Reasoning was asked to be off but this model's channel is known to
    /// refuse the switch: it sends none, so no refusal and no resend happen.
    /// Informational: such replies are not marked `reasoning_off_refused`
    /// (only a refused and resent request is, L4 r10 P2-8).
    fn reasoning_switch_unsupported(&self) -> bool {
        false
    }

    /// After a resend without the switch succeeded: remember for this
    /// model's channel that its provider refuses the switch, so later runs
    /// send none (L4 r9 P2-1). Nothing for models without a channel.
    fn remember_reasoning_switch_unsupported(&self) {}

    /// After a 400/422 fallback stripped `prompt_cache_key` / `cache_control`,
    /// remember for the rest of this run so later steps do not retry it.
    fn remember_prompt_cache_key_refused(&self) {}
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Claim {
    pub statement: String,
    pub refs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StructuredAnswer {
    pub text: String,
    pub claims: Vec<Claim>,
}

/// What the model's final reply holds once its answer is read out of it.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAnswer {
    pub answer: StructuredAnswer,
    /// A `{text, claims}` object was found in the reply (the whole reply, a
    /// fenced code block, or an object after prose). When false, `answer`
    /// is the reply's prose with no claims.
    pub structured: bool,
}

/// Only the last this many bytes of a reply are scanned for an object
/// embedded in prose (each `{` starts a parse attempt).
const SCAN_WINDOW_BYTES: usize = 16 * 1024;

/// Returns `text` with inline `<think>…</think>` / `<thinking>…</thinking>`
/// blocks stripped, orphan closing `</think>` prefixes removed, unclosed
/// `<think>` blocks quarantined (dropped rather than leaked into prose), and
/// triple-backtick fenced code blocks preserved verbatim.
pub fn without_think(text: &str) -> String {
    think_normalizer::strip_think_blocks(text).text
}

fn string_of<'a>(object: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

/// One reference as the host reads it (`task:<id>`, `batch:<id>`,
/// `web:<url>`): a string as given, or an object naming its kind and id.
fn reference_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_owned()).filter(|text| !text.is_empty()),
        Value::Object(object) => {
            if let Some(text) = string_of(object, &["ref", "reference"]) {
                return Some(text.to_owned());
            }
            for (key, kind) in [
                ("task_id", "task"),
                ("batch_id", "batch"),
                ("search_batch_id", "batch"),
                ("url", "web"),
            ] {
                if let Some(id) = object.get(key).and_then(Value::as_str) {
                    return Some(format!("{kind}:{id}"));
                }
            }
            let kind = string_of(object, &["kind", "type"])?;
            let id = string_of(object, &["id", "value"])?;
            Some(format!("{kind}:{id}"))
        }
        _ => None,
    }
}

/// A claim as the model wrote it: an object with its statement under
/// `statement`, `text`, `claim` or `content` and its references under
/// `refs`, `evidence`, `sources`, `references` or `citations` (one value or
/// a list), or a bare statement string.
fn claim_of(value: &Value) -> Option<Claim> {
    let (statement, refs) = match value {
        Value::String(text) => (text.as_str(), Vec::new()),
        Value::Object(object) => {
            let statement = string_of(object, &["statement", "text", "claim", "content"])?;
            let refs = ["refs", "evidence", "sources", "references", "citations"]
                .iter()
                .find_map(|key| object.get(*key))
                .map(|refs| match refs {
                    Value::Array(items) => items.iter().filter_map(reference_of).collect(),
                    other => reference_of(other).into_iter().collect(),
                })
                .unwrap_or_default();
            (statement, refs)
        }
        _ => return None,
    };
    let statement = statement.trim();
    (!statement.is_empty()).then(|| Claim {
        statement: statement.to_owned(),
        refs,
    })
}

/// A `{text, claims}` object (claims may be missing or empty, and no unrelated
/// keys may be present so arbitrary JSON code examples in Markdown answers are
/// never mistaken for a structured conclusion wrapper).
fn answer_of(value: &Value) -> Option<StructuredAnswer> {
    let object = value.as_object()?;
    if !object.keys().all(|k| k == "text" || k == "claims") {
        return None;
    }
    let text = object.get("text")?.as_str()?;
    let claims = match object.get("claims") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().filter_map(claim_of).collect(),
        Some(_) => return None,
    };
    Some(StructuredAnswer {
        text: text.trim().to_owned(),
        claims,
    })
}

fn parse_lenient_answer_json(candidate: &str) -> Option<StructuredAnswer> {
    if let Some(ans) = serde_json::from_str::<Value>(candidate)
        .ok()
        .as_ref()
        .and_then(answer_of)
    {
        return Some(ans);
    }
    let repaired = strip_json_trailing_commas(candidate);
    serde_json::from_str::<Value>(&repaired)
        .ok()
        .as_ref()
        .and_then(answer_of)
}

/// The last `{text, claims}` object in `text`, with the byte span it (and a
/// code fence around it) occupies.
fn embedded_answer(text: &str) -> Option<(StructuredAnswer, usize, usize)> {
    let mut window = text.len().saturating_sub(SCAN_WINDOW_BYTES);
    while !text.is_char_boundary(window) {
        window += 1;
    }
    let mut found = None;
    let mut index = window;
    while let Some(offset) = text[index..].find('{') {
        let start = index + offset;
        let mut values = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
        match values.next() {
            Some(Ok(value)) => {
                let end = start + values.byte_offset();
                if let Some(answer) = answer_of(&value) {
                    found = Some((answer, start, end));
                }
                index = end;
            }
            _ => {
                // Also support trailing commas inside a final JSON object.
                if let Some(close_brace) = matching_brace_end(&text[start..]) {
                    let end = start + close_brace;
                    if let Some(answer) = parse_lenient_answer_json(&text[start..end]) {
                        found = Some((answer, start, end));
                        index = end;
                        continue;
                    }
                }
                index = start + 1;
            }
        }
    }
    let (answer, start, end) = found?;
    // Widen to a surrounding fence (an opening fence with an empty or
    // alphanumeric info string such as json, and a closing fence).
    let before = text[..start].trim_end();
    let open = before.rfind("```").filter(|at| {
        before[at + 3..]
            .trim()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    });
    let after = &text[end..];
    let gap = after.len() - after.trim_start().len();
    let close = after[gap..].starts_with("```").then_some(end + gap + 3);
    match (open, close) {
        (Some(open), Some(close)) => Some((answer, open, close)),
        _ => Some((answer, start, end)),
    }
}

fn matching_brace_end(slice: &str) -> Option<usize> {
    let bytes = slice.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Reads the model's final reply: supports clean natural Markdown first-class
/// (`structured == false`), while remaining backward-compatible when a model
/// outputs `{"text": "...", "claims": [...]}` directly, wrapped in ```` ```json ````
/// fences, or after prose (`structured == true`). Any `<think>` / `<thinking>`
/// remnants outside code fences are stripped.
pub fn parse_answer(reply: &str) -> ParsedAnswer {
    let cleaned = without_think(reply);
    let trimmed = cleaned.trim();
    if let Some(answer) = parse_lenient_answer_json(trimmed) {
        return ParsedAnswer {
            answer,
            structured: true,
        };
    }
    let unfenced = strip_surrounding_code_fence(trimmed);
    if unfenced != trimmed {
        if let Some(answer) = parse_lenient_answer_json(unfenced) {
            return ParsedAnswer {
                answer,
                structured: true,
            };
        }
    }
    if let Some((mut answer, start, end)) = embedded_answer(trimmed) {
        if answer.text.is_empty() {
            // The object carried claims only: its prose is the answer.
            answer.text = format!("{}{}", &trimmed[..start], &trimmed[end..])
                .trim()
                .to_owned();
        }
        return ParsedAnswer {
            answer,
            structured: true,
        };
    }
    ParsedAnswer {
        answer: StructuredAnswer {
            text: trimmed.to_owned(),
            claims: vec![],
        },
        structured: false,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub steps: usize,
    pub messages: Vec<Message>,
    pub tool_results: Vec<Value>,
    /// Mark before invoking the executor; after interruption, caller must reconcile this ID from Store.
    pub pending_call: Option<ToolCall>,
}

impl Checkpoint {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            steps: 0,
            messages,
            tool_results: Vec::new(),
            pending_call: None,
        }
    }
}

const PLAN_STEP_TOOL_NAMES: &[&str] = &[
    "working_aoi_set",
    "search_start",
    "create_task",
    "task_action",
    "update_download_task",
];

fn is_known_canonical_tool(name: &str, tools: &[ToolDescription]) -> bool {
    tools.iter().any(|t| t.name == name) || PLAN_STEP_TOOL_NAMES.contains(&name)
}

fn normalize_tool_name(name: &str, tools: &[ToolDescription]) -> String {
    let trimmed = name.trim();
    if is_known_canonical_tool(trimmed, tools) {
        return trimmed.to_owned();
    }
    for delim in ["__", ".", ":", "/"] {
        if let Some((_, rhs)) = trimmed.rsplit_once(delim) {
            let rhs = rhs.trim();
            if is_known_canonical_tool(rhs, tools) {
                return rhs.to_owned();
            }
        }
    }
    for prefix in ["fn_", "func_", "tool_"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim();
            if is_known_canonical_tool(rest, tools) {
                return rest.to_owned();
            }
        }
    }
    trimmed.to_owned()
}

fn parse_json_container_string(value: &mut Value, expect_object: bool, expect_array: bool) {
    let Some(raw) = value.as_str() else {
        return;
    };
    let s = raw.trim();
    if expect_object && s.starts_with('{') {
        if let Ok(parsed @ Value::Object(_)) = serde_json::from_str::<Value>(s) {
            *value = parsed;
        }
    } else if expect_array && s.starts_with('[') {
        if let Ok(parsed @ Value::Array(_)) = serde_json::from_str::<Value>(s) {
            *value = parsed;
        } else {
            let mut stream = serde_json::Deserializer::from_str(s).into_iter::<Value>();
            if let Some(Ok(parsed @ Value::Array(_))) = stream.next() {
                let rest = s[stream.byte_offset()..].trim_start();
                if rest.is_empty() || rest.starts_with(',') {
                    *value = parsed;
                }
            }
        }
    }
}

fn coerce_scalar_string(value: &mut Value, expected_type: &str) {
    let Some(raw) = value.as_str() else {
        return;
    };
    let s = raw.trim();
    match expected_type {
        "integer" => {
            if let Ok(i) = s.parse::<i64>() {
                *value = Value::Number(i.into());
            } else if let Ok(u) = s.parse::<u64>() {
                *value = Value::Number(u.into());
            }
        }
        "number" => {
            if let Ok(f) = s.parse::<f64>() {
                if f.is_finite() {
                    if let Some(n) = serde_json::Number::from_f64(f) {
                        *value = Value::Number(n);
                    }
                }
            }
        }
        "boolean" => {
            if s.eq_ignore_ascii_case("true") {
                *value = Value::Bool(true);
            } else if s.eq_ignore_ascii_case("false") {
                *value = Value::Bool(false);
            }
        }
        _ => {}
    }
}

fn coerce_plan_step_input_fallbacks(input: &mut Value) {
    parse_json_container_string(input, true, false);
    let Some(map) = input.as_object_mut() else {
        return;
    };
    for obj_key in ["aoi", "time_range", "filters", "options", "command"] {
        if let Some(v) = map.get_mut(obj_key) {
            parse_json_container_string(v, true, false);
        }
    }
    for arr_key in ["bbox", "item_ids"] {
        if let Some(v) = map.get_mut(arr_key) {
            parse_json_container_string(v, false, true);
        }
    }
    if let Some(Value::Array(bbox)) = map.get_mut("bbox") {
        for coord in bbox.iter_mut() {
            coerce_scalar_string(coord, "number");
        }
    }
    for int_key in ["revision", "expected_revision"] {
        if let Some(v) = map.get_mut(int_key) {
            coerce_scalar_string(v, "integer");
        }
    }
    for bool_key in ["clear", "include_precise_orbit"] {
        if let Some(v) = map.get_mut(bool_key) {
            coerce_scalar_string(v, "boolean");
        }
    }
    if let Some(Value::Object(opts)) = map.get_mut("options") {
        if let Some(v) = opts.get_mut("concurrency") {
            coerce_scalar_string(v, "integer");
        }
        if let Some(v) = opts.get_mut("include_precise_orbit") {
            coerce_scalar_string(v, "boolean");
        }
        if let Some(v) = opts.get_mut("output_root") {
            parse_json_container_string(v, true, false);
        }
    }
    if let Some(Value::Object(cmd)) = map.get_mut("command") {
        if let Some(v) = cmd.get_mut("item_ids") {
            parse_json_container_string(v, false, true);
        }
    }
    if let Some(Value::Object(filters)) = map.get_mut("filters") {
        for (_, fv) in filters.iter_mut() {
            parse_json_container_string(fv, true, true);
        }
        for int_key in ["relative_orbit", "frame", "path"] {
            if let Some(fv) = filters.get_mut(int_key) {
                coerce_scalar_string(fv, "integer");
            }
        }
        for num_key in [
            "cloud_cover_lte",
            "cloud_cover_percent",
            "cloud_cover",
            "max_cloud_cover",
            "cloud_cover_max",
        ] {
            if let Some(fv) = filters.get_mut(num_key) {
                coerce_scalar_string(fv, "number");
            }
        }
    }
}

fn coerce_args_by_schema(value: &mut Value, schema: &Value, tools: &[ToolDescription]) {
    if let Some(variants) = schema.get("anyOf").and_then(Value::as_array) {
        parse_json_container_string(value, true, false);
        if let Some(obj) = value.as_object_mut() {
            if let Some(tid_val) = obj.get_mut("tool_id") {
                if let Some(tid_str) = tid_val.as_str() {
                    let norm = normalize_tool_name(tid_str, tools);
                    if norm != tid_str {
                        *tid_val = Value::String(norm);
                    }
                }
            }
        }
        let step_tool_id = value
            .get("tool_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(tid) = step_tool_id {
            if let Some(matched) = variants.iter().find(|variant| {
                variant
                    .pointer("/properties/tool_id/enum")
                    .and_then(Value::as_array)
                    .is_some_and(|enums| enums.iter().any(|e| e.as_str() == Some(tid.as_str())))
            }) {
                coerce_args_by_schema(value, matched, tools);
                if let Some(obj) = value.as_object_mut() {
                    if let Some(inp) = obj.get_mut("input") {
                        coerce_plan_step_input_fallbacks(inp);
                    }
                }
                return;
            }
        }
    }

    let Some(expected_type) = schema.get("type").and_then(Value::as_str) else {
        return;
    };
    match expected_type {
        "object" => {
            parse_json_container_string(value, true, false);
            if let (Some(map), Some(props)) = (
                value.as_object_mut(),
                schema.get("properties").and_then(Value::as_object),
            ) {
                for (key, prop_schema) in props {
                    if let Some(field_val) = map.get_mut(key) {
                        coerce_args_by_schema(field_val, prop_schema, tools);
                    }
                }
            }
        }
        "array" => {
            parse_json_container_string(value, false, true);
            if let (Some(arr), Some(item_schema)) = (value.as_array_mut(), schema.get("items")) {
                for elem in arr.iter_mut() {
                    coerce_args_by_schema(elem, item_schema, tools);
                }
            }
        }
        "integer" | "number" | "boolean" => coerce_scalar_string(value, expected_type),
        _ => {}
    }
}

fn normalize_propose_plan_arguments(arguments: &mut Value, tools: &[ToolDescription]) {
    parse_json_container_string(arguments, true, false);
    if let Some(propose_tool) = tools.iter().find(|t| t.name == "propose_plan") {
        coerce_args_by_schema(arguments, &propose_tool.parameters, tools);
    }
    let Some(steps_val) = arguments.get_mut("steps") else {
        return;
    };
    parse_json_container_string(steps_val, false, true);
    let Some(steps) = steps_val.as_array_mut() else {
        return;
    };
    for step in steps.iter_mut() {
        parse_json_container_string(step, true, false);
        let Some(step_obj) = step.as_object_mut() else {
            continue;
        };
        if let Some(tid_val) = step_obj.get_mut("tool_id") {
            if let Some(tid_str) = tid_val.as_str() {
                let norm = normalize_tool_name(tid_str, tools);
                if norm != tid_str {
                    *tid_val = Value::String(norm);
                }
            }
        }
        if let Some(inp) = step_obj.get_mut("input") {
            coerce_plan_step_input_fallbacks(inp);
        }
    }
}

fn unwrap_parallel_and_prefixes(calls: &[ToolCall], tools: &[ToolDescription]) -> Vec<ToolCall> {
    let mut expanded = Vec::with_capacity(calls.len());
    for call in calls {
        let trimmed_name = call.name.trim();
        let is_parallel_wrapper = (trimmed_name == "multi_tool_use.parallel"
            || trimmed_name.ends_with(".multi_tool_use.parallel")
            || trimmed_name.ends_with(":multi_tool_use.parallel")
            || trimmed_name.ends_with("__multi_tool_use.parallel"))
            && call
                .arguments
                .get("tool_uses")
                .and_then(Value::as_array)
                .is_some_and(|arr| !arr.is_empty());
        if is_parallel_wrapper {
            if let Some(uses) = call.arguments.get("tool_uses").and_then(Value::as_array) {
                for (idx, item) in uses.iter().enumerate() {
                    if let Some(recipient) = item.get("recipient_name").and_then(Value::as_str) {
                        let normalized_name = normalize_tool_name(recipient, tools);
                        let params = item
                            .get("parameters")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}));
                        expanded.push(ToolCall {
                            id: format!("{}-{idx}", call.id),
                            name: normalized_name,
                            arguments: params,
                        });
                    }
                }
                continue;
            }
        }
        expanded.push(ToolCall {
            id: call.id.clone(),
            name: normalize_tool_name(&call.name, tools),
            arguments: call.arguments.clone(),
        });
    }
    for call in &mut expanded {
        if call.name == "propose_plan" {
            normalize_propose_plan_arguments(&mut call.arguments, tools);
        } else if let Some(tool) = tools.iter().find(|t| t.name == call.name) {
            coerce_args_by_schema(&mut call.arguments, &tool.parameters, tools);
        }
    }
    expanded
}

fn unwrap_step_input(arguments: &Value) -> Value {
    let mut candidate = arguments.clone();
    parse_json_container_string(&mut candidate, true, false);
    if let Some(map) = candidate.as_object() {
        let wrapped_only_input = map.len() == 1
            && map.get("input").is_some_and(|v| {
                v.is_object() || v.as_str().is_some_and(|s| s.trim().starts_with('{'))
            });
        let wrapped_tool_and_input = map.len() == 2
            && map.contains_key("tool_id")
            && map.get("input").is_some_and(|v| {
                v.is_object() || v.as_str().is_some_and(|s| s.trim().starts_with('{'))
            });
        if wrapped_only_input || wrapped_tool_and_input {
            if let Some(inner) = map.get("input") {
                let mut unwrapped = inner.clone();
                coerce_plan_step_input_fallbacks(&mut unwrapped);
                return unwrapped;
            }
        }
    }
    coerce_plan_step_input_fallbacks(&mut candidate);
    candidate
}

fn coalesce_plan_step_calls(calls: &[ToolCall], tools: &[ToolDescription]) -> Vec<ToolCall> {
    let pre_normalized = unwrap_parallel_and_prefixes(calls, tools);
    if !tools.iter().any(|t| t.name == "propose_plan") {
        return pre_normalized;
    }
    let is_plan_step =
        |name: &str| PLAN_STEP_TOOL_NAMES.contains(&name) && !tools.iter().any(|t| t.name == name);
    if !pre_normalized.iter().any(|c| is_plan_step(&c.name)) {
        return pre_normalized;
    }
    let mut out: Vec<ToolCall> = Vec::with_capacity(pre_normalized.len());
    let mut pending_id: Option<String> = None;
    let mut pending_steps: Vec<Value> = Vec::new();
    for call in &pre_normalized {
        if is_plan_step(&call.name) {
            if pending_id.is_none() {
                pending_id = Some(call.id.clone());
            }
            pending_steps.push(serde_json::json!({
                "tool_id": call.name,
                "input": unwrap_step_input(&call.arguments),
            }));
        } else {
            if let Some(id) = pending_id.take() {
                let steps = std::mem::take(&mut pending_steps);
                let mut plan_args = serde_json::json!({ "steps": steps });
                normalize_propose_plan_arguments(&mut plan_args, tools);
                out.push(ToolCall {
                    id,
                    name: "propose_plan".to_string(),
                    arguments: plan_args,
                });
            }
            out.push(call.clone());
        }
    }
    if let Some(id) = pending_id.take() {
        let mut plan_args = serde_json::json!({ "steps": pending_steps });
        normalize_propose_plan_arguments(&mut plan_args, tools);
        out.push(ToolCall {
            id,
            name: "propose_plan".to_string(),
            arguments: plan_args,
        });
    }
    out
}

#[derive(Debug)]
pub struct RunResult {
    pub checkpoint: Checkpoint,
    pub answer: StructuredAnswer,
    /// The final reply held a `{text, claims}` object (`parse_answer`).
    pub structured: bool,
    /// Normalized reasoning text from the final turn (empty when none).
    pub reasoning_text: String,
    /// Monotonic thinking duration in milliseconds from the final turn (`None` when zero reasoning).
    pub thinking_ms: Option<u64>,
    /// True if the final turn opened `<think>` / `<thinking>` and never closed it.
    pub think_block_unclosed: bool,
    /// True if an orphan closing `</think>` reclassified leading prose into reasoning.
    pub orphan_think_reclassified: bool,
}

const IMMEDIATE_ACTION_PROMISES: &[&str] = &[
    "下面一次性提交",
    "下面提交",
    "下面我将",
    "下面为您",
    "下面开始检索",
    "下面开始执行",
    "下面立即",
    "下面直接",
    "接下来一次性提交",
    "接下来提交",
    "接下来我将",
    "接下来调用",
    "接下来为您",
    "接下来立即",
    "接下来直接",
    "现在一次性提交",
    "现在为您提交",
    "现在提交计划",
    "现在立即",
    "现在直接检索",
    "现在开始检索",
    "我将立即",
    "我将直接",
    "我将调用",
    "我将提交",
    "马上为您检索",
    "马上为您切换",
    "马上提交",
    "正在为您切换",
    "正在为您检索",
    "正在提交计划",
    "立即提交计划",
    "立即调用",
];

fn is_phantom_tool_or_plan_prose(clean_text: &str, saw_tool_results: bool) -> bool {
    clean_text.contains("[plan-ref]")
        || clean_text.contains("[plan:plan-")
        || clean_text.contains("[上一轮运行记录]")
        || clean_text.contains("[上一轮结论]")
        || clean_text.contains("提出计划 plan-")
        || clean_text.contains("<tool_call>")
        || clean_text.contains("<function=")
        || clean_text.contains("<fn__")
        || (!saw_tool_results && clean_text.contains("计划步骤") && clean_text.contains("执行完成"))
}

fn is_unfulfilled_action_promise(clean_text: &str, saw_tool_results: bool) -> bool {
    let trimmed = clean_text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if IMMEDIATE_ACTION_PROMISES
        .iter()
        .any(|p| trimmed.contains(p))
    {
        return true;
    }
    !saw_tool_results
        && trimmed.contains("行政区划代码")
        && (trimmed.contains("已找到") || trimmed.contains("行政区候选"))
}

fn with_checkpoint(mut error: ModelError, checkpoint: Checkpoint) -> ModelError {
    error.checkpoint = Some(Box::new(checkpoint));
    error
}

/// Ensures newly generated `ToolCall` IDs never collide with prior calls in
/// `existing_messages` (or earlier items in `calls`), without mutating prior
/// messages so KV prompt-cache prefixes stay byte-identical across turns.
fn deduplicate_tool_call_ids(calls: &mut [ToolCall], existing_messages: &[Message]) {
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in existing_messages {
        if msg.role == "assistant_tool_calls" {
            if let Ok(payload) = serde_json::from_str::<Value>(&msg.content) {
                if let Some(arr) = payload.get("tool_calls").and_then(Value::as_array) {
                    for item in arr {
                        if let Some(id) = item.get("id").and_then(Value::as_str) {
                            if !id.is_empty() {
                                seen_ids.insert(id.to_owned());
                            }
                        }
                    }
                }
            }
        } else if msg.role == "tool" {
            if let Ok(payload) = serde_json::from_str::<Value>(&msg.content) {
                if let Some(id) = payload.get("id").and_then(Value::as_str) {
                    if !id.is_empty() {
                        seen_ids.insert(id.to_owned());
                    }
                }
            }
        }
    }
    for call in calls.iter_mut() {
        let base = if call.id.trim().is_empty() {
            "call"
        } else {
            call.id.as_str()
        };
        if !call.id.trim().is_empty() && !seen_ids.contains(&call.id) {
            seen_ids.insert(call.id.clone());
            continue;
        }
        let mut suffix = 2usize;
        loop {
            let candidate = format!("{base}_r{suffix}");
            if !seen_ids.contains(&candidate) {
                call.id = candidate.clone();
                seen_ids.insert(candidate);
                break;
            }
            suffix += 1;
        }
    }
}

pub struct AgentLoop {
    pub max_steps: usize,
}

impl AgentLoop {
    pub async fn run(
        &self,
        model: &impl Model,
        dispatcher: &impl Dispatcher,
        tools: &[ToolDescription],
        mut checkpoint: Checkpoint,
        cancel: &CancellationToken,
        mut emit: impl FnMut(ModelEvent) + Send,
    ) -> Result<RunResult, ModelError> {
        if checkpoint.pending_call.is_some() {
            // Unknown outcome: W3a must query Store and reconcile before resuming.
            return Err(ModelError::new(ErrorKind::Capability, None));
        }
        let mut previous: Option<Vec<(String, Value)>> = None;
        let mut phantom_nudges = 0usize;
        while checkpoint.steps < self.max_steps {
            if cancel.is_cancelled() {
                return Err(with_checkpoint(
                    ModelError::new(ErrorKind::Cancelled, None),
                    checkpoint,
                ));
            }
            let completion = match model
                .complete(&checkpoint.messages, tools, cancel, &mut emit)
                .await
            {
                Ok(completion) => completion,
                Err(mut error) => {
                    if error.checkpoint.is_none()
                        && (checkpoint.steps > 0 || !checkpoint.tool_results.is_empty())
                    {
                        error.checkpoint = Some(Box::new(checkpoint.clone()));
                    }
                    return Err(error);
                }
            };
            checkpoint.steps += 1;
            if completion.tool_calls.is_empty() {
                let inline_norm = think_normalizer::strip_think_blocks(&completion.text);
                let clean_text = inline_norm.text.clone();
                let had_closed_reasoning = (!completion.reasoning_text.is_empty()
                    || !inline_norm.reasoning_text.is_empty())
                    && !completion.think_block_unclosed
                    && !inline_norm.think_block_unclosed
                    && !completion.truncated;
                let saw_tools = !checkpoint.tool_results.is_empty();
                let is_phantom = is_phantom_tool_or_plan_prose(&clean_text, saw_tools);
                let is_thinking_only =
                    !is_phantom && clean_text.trim().is_empty() && had_closed_reasoning;
                let is_unfulfilled = !is_phantom
                    && !is_thinking_only
                    && is_unfulfilled_action_promise(&clean_text, saw_tools);
                if checkpoint.steps < self.max_steps
                    && phantom_nudges < 2
                    && tools.iter().any(|t| t.name == "propose_plan")
                    && (is_phantom || is_thinking_only || is_unfulfilled)
                {
                    phantom_nudges += 1;
                    previous = None;
                    let scrubbed_assistant = privacy::scrub(&Value::String(clean_text))
                        .as_str()
                        .unwrap_or("")
                        .to_owned();
                    if !scrubbed_assistant.trim().is_empty() {
                        checkpoint.messages.push(Message {
                            role: "assistant".into(),
                            content: scrubbed_assistant,
                        });
                    }
                    let nudge = if is_phantom {
                        "[host] 检测到你仅输出了占位符文本（如 [plan-ref] 或伪造的运行记录），实际上并未发起任何 function tool_calls！严禁输出 [plan-ref] 或伪造执行记录。请立即通过 function tool_calls 调用真实工具（例如调用 propose_plan 提交 working_aoi_set 或 search_start 步骤）；若确实无需执行操作，请直接用正常中文回复。"
                    } else if is_thinking_only {
                        "[host] 检测到你刚才仅输出了思考过程，尚未发起任何 function tool_calls 也未输出正式回复。请立即通过 function tool_calls 调用所需工具（如 admin_boundary_search、source_list、propose_plan、map_workspace_action 等），或直接输出面向用户的正式中文结论，严禁将（思考：…）作为最终正文返回。"
                    } else {
                        "[host] 检测到你刚才仅输出了准备提交或即将执行的文字说明，但未发起任何 function tool_calls，本机尚未执行任何操作。注意切勿凭记忆猜测行政区编码或数据源编号（切换行政区必须先调用 admin_boundary_search 获取真实 admin_code，检索请用 source_list 的真实 resource_id），请立即通过 function tool_calls 发起真实工具调用（如 admin_boundary_search、source_list、propose_plan 或 map_workspace_action），不要只发预告文字就结束回复。"
                    };
                    checkpoint.messages.push(Message {
                        role: "user".into(),
                        content: nudge.into(),
                    });
                    continue;
                }
                let fallback_norm = completion.reasoning_text.is_empty().then_some(inline_norm);
                let ParsedAnswer { answer, structured } = parse_answer(&completion.text);
                let reasoning_text = if !completion.reasoning_text.is_empty() {
                    completion.reasoning_text
                } else {
                    fallback_norm
                        .as_ref()
                        .map(|n| n.reasoning_text.clone())
                        .unwrap_or_default()
                };
                let thinking_ms = completion
                    .thinking_ms
                    .or_else(|| fallback_norm.as_ref().and_then(|n| n.thinking_ms));
                let think_block_unclosed = completion.think_block_unclosed
                    || fallback_norm
                        .as_ref()
                        .is_some_and(|n| n.think_block_unclosed);
                let orphan_think_reclassified = completion.orphan_think_reclassified
                    || fallback_norm
                        .as_ref()
                        .is_some_and(|n| n.orphan_think_reclassified);
                return Ok(RunResult {
                    checkpoint,
                    answer,
                    structured,
                    reasoning_text,
                    thinking_ms,
                    think_block_unclosed,
                    orphan_think_reclassified,
                });
            }
            let mut normalized_calls = coalesce_plan_step_calls(&completion.tool_calls, tools);
            deduplicate_tool_call_ids(&mut normalized_calls, &checkpoint.messages);
            let signature = normalized_calls
                .iter()
                .map(|call| (call.name.clone(), call.arguments.clone()))
                .collect::<Vec<_>>();
            let is_async_progress_poll = !normalized_calls.is_empty()
                && normalized_calls
                    .iter()
                    .all(|call| call.name == "search_progress");
            if !is_async_progress_poll && previous.as_ref() == Some(&signature) {
                return Err(with_checkpoint(
                    ModelError::new(ErrorKind::RepeatedCall, None),
                    checkpoint,
                ));
            }
            previous = Some(signature);
            let clean_assistant_text = without_think(&completion.text);
            checkpoint.messages.push(Message {
                role: "assistant_tool_calls".into(),
                content: serde_json::json!({"text": clean_assistant_text, "tool_calls": normalized_calls}).to_string(),
            });
            for call in &normalized_calls {
                if cancel.is_cancelled() {
                    return Err(with_checkpoint(
                        ModelError::new(ErrorKind::Cancelled, None),
                        checkpoint,
                    ));
                }
                if !tools.iter().any(|tool| tool.name == call.name)
                    && !dispatcher.accepts_unoffered_calls()
                {
                    return Err(with_checkpoint(
                        ModelError::new(ErrorKind::ToolNotOffered, None),
                        checkpoint,
                    ));
                }
                checkpoint.pending_call = Some(call.clone());
                let result = dispatcher
                    .execute(call, cancel)
                    .await
                    .map_err(|mut error| {
                        error.checkpoint = Some(Box::new(checkpoint.clone()));
                        error
                    })?;
                checkpoint.pending_call = None;
                checkpoint.messages.push(Message {
                    role: "tool".into(),
                    content: serde_json::json!({"id": call.id, "result": privacy::scrub(&result)})
                        .to_string(),
                });
                checkpoint.tool_results.push(result);
            }
        }
        Err(with_checkpoint(
            ModelError::new(ErrorKind::StepLimit, None),
            checkpoint,
        ))
    }
}
