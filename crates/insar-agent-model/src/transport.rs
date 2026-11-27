use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, Url};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::{
    classify_status, privacy, Completion, ErrorKind, Message, ModelError, ModelEvent, Protocol,
    ReasoningMode, StreamDecoder, ToolDescription,
};

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 4 * 1024;
const MAX_MODEL_IDS: usize = 100;
const CODEX_CLIENT_VERSION: &str = "0.170.0";

fn is_codex_endpoint_url(url: &Url) -> bool {
    url.path().contains("/backend-api/codex") || url.host_str() == Some("chatgpt.com")
}

/// No credentials, URL query, user info or fragment may be part of the route.
#[derive(Clone)]
pub struct Endpoint {
    base: Url,
    pub(crate) protocol: Protocol,
}

impl Endpoint {
    pub fn new(base: &str, protocol: Protocol) -> Result<Self, ModelError> {
        let parsed = Url::parse(base).map_err(|_| ModelError::new(ErrorKind::Parameter, None))?;
        let host = parsed.host_str().unwrap_or("");
        let loopback = host == "localhost" || host == "127.0.0.1" || host == "[::1]";
        if (parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback))
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || host.is_empty()
        {
            return Err(ModelError::new(ErrorKind::Parameter, None));
        }
        Ok(Self {
            base: parsed,
            protocol,
        })
    }

    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    fn url(&self, suffix: &str) -> Result<Url, ModelError> {
        let mut path = self.base.path().trim_end_matches('/');
        if self.protocol == Protocol::GoogleGemini {
            if let Some(idx) = path.find(':') {
                path = path[..idx].trim_end_matches('/');
            }
            if let Some(stripped) = path.strip_suffix("/models") {
                path = stripped;
            }
            if path.is_empty() {
                path = "/v1beta";
            }
        } else if suffix == "models" {
            for endpoint_suffix in ["/chat/completions", "/responses", "/messages"] {
                if let Some(stripped) = path.strip_suffix(endpoint_suffix) {
                    path = stripped;
                    break;
                }
            }
        }
        let path = if self.protocol == Protocol::AnthropicMessages && path.is_empty() {
            "/v1"
        } else {
            path
        };
        let suffix_with_slash = format!("/{suffix}");
        let full_path = if path == suffix_with_slash || path.ends_with(&suffix_with_slash) {
            path.to_owned()
        } else {
            format!("{path}/{suffix}")
        };
        let mut url = self.base.clone();
        url.set_path(&full_path);
        Ok(url)
    }
}

/// Model families whose reasoning switch is known, judged from the model id
/// (the part after the last `/`, case-insensitive).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelFamily {
    Glm,
    /// `gpt-5`, `gpt-5-mini`, `gpt-5-2025-08-07`: lowest effort "minimal".
    Gpt5,
    /// `gpt-5.1` and later point releases, and `gpt-5-chat*`: OpenAI models
    /// whose accepted effort values differ; neither a budget nor an effort is sent.
    Gpt5Point,
    /// `o1`, `o3-mini`, `o4-mini`: lowest effort "low".
    OSeries,
    Other,
}

impl ModelFamily {
    fn of(model: &str) -> Self {
        let id = model
            .rsplit('/')
            .next()
            .unwrap_or(model)
            .to_ascii_lowercase();
        let bytes = id.as_bytes();
        if id.starts_with("glm-") {
            Self::Glm
        } else if id.starts_with("gpt-5-chat") {
            Self::Gpt5Point
        } else if id == "gpt-5" || id.starts_with("gpt-5-") {
            Self::Gpt5
        } else if id.starts_with("gpt-5.") {
            Self::Gpt5Point
        } else if bytes.first() == Some(&b'o')
            && bytes.get(1).is_some_and(u8::is_ascii_digit)
            && bytes[1..]
                .iter()
                .find(|b| !b.is_ascii_digit())
                .is_none_or(|b| *b == b'-')
        {
            Self::OSeries
        } else {
            Self::Other
        }
    }

    fn lowest_effort(self) -> Option<&'static str> {
        match self {
            Self::Gpt5 => Some("minimal"),
            Self::OSeries => Some("low"),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct ModelRoute {
    pub endpoint: Endpoint,
    pub model: String,
    pub reasoning: ReasoningMode,
    /// The channel is known to refuse the reasoning switch (W3o: its
    /// provider answered HTTP 400 to it and the resend without it
    /// succeeded): no switch is sent and, where a budget is sent, it is the
    /// larger one, because the model will think.
    pub reasoning_switch_unsupported: bool,
    /// A thinking level's value chosen for this run: `reasoning_effort`
    /// (chat), `reasoning.effort` (responses), or a token budget for
    /// Anthropic `thinking`. Used with `ProviderDefault` only.
    pub reasoning_effort: Option<String>,
    /// Configured model output cap when supplied by channel metadata (`None` keeps the 4096 baseline).
    pub max_output_tokens: Option<u64>,
    /// Optional provider-specific HTTP headers (for example OAuth account/client headers).
    pub extra_headers: Vec<(String, String)>,
    /// When true on ChatCompletions, omit `stream_options` (used when retrying
    /// gateways that reject `stream_options` with HTTP 400).
    pub omit_stream_options: bool,
    /// Stable per-session cache routing key (ASCII, secret-free, path-free).
    /// Sent as `prompt_cache_key` (and `session_id` header on Codex) on
    /// Responses, and enables `cache_control: {"type":"ephemeral"}` prefix
    /// markers on Anthropic Messages.
    pub prompt_cache_key: Option<String>,
}

impl ModelRoute {
    pub fn new(protocol: Protocol, base: &str, model: &str) -> Result<Self, ModelError> {
        if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
            return Err(ModelError::new(ErrorKind::Parameter, None));
        }
        Ok(Self {
            endpoint: Endpoint::new(base, protocol)?,
            model: model.to_owned(),
            reasoning: ReasoningMode::Off,
            reasoning_switch_unsupported: false,
            reasoning_effort: None,
            max_output_tokens: None,
            extra_headers: Vec::new(),
            omit_stream_options: false,
            prompt_cache_key: None,
        })
    }

    pub fn with_prompt_cache_key(mut self, key: Option<String>) -> Self {
        self.prompt_cache_key = key.and_then(|raw| {
            let trimmed = raw.trim();
            (!trimmed.is_empty()
                && trimmed.len() <= 64
                && trimmed
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
            .then(|| trimmed.to_owned())
        });
        self
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: Option<u64>) -> Self {
        self.max_output_tokens = max_output_tokens.filter(|v| *v >= 1);
        self
    }

    pub fn with_extra_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.extra_headers = headers;
        self
    }

    pub fn with_omit_stream_options(mut self, omit: bool) -> Self {
        self.omit_stream_options = omit;
        self
    }

    /// The same route asking for a thinking level's value (see the field).
    pub fn with_reasoning_effort(mut self, effort: Option<String>) -> Self {
        self.reasoning_effort = effort;
        self
    }

    pub fn with_reasoning(mut self, reasoning: ReasoningMode) -> Self {
        self.reasoning = reasoning;
        self
    }

    /// The same route, known (or not) to refuse the reasoning switch.
    pub fn with_reasoning_switch_unsupported(mut self, unsupported: bool) -> Self {
        self.reasoning_switch_unsupported = unsupported;
        self
    }

    /// Caller supplies only policy-approved tool schemas and already sanitized messages.
    pub fn request(
        &self,
        messages: &[Message],
        tools: &[ToolDescription],
    ) -> Result<(Url, Value), ModelError> {
        let tools: Vec<ToolDescription> = serde_json::from_value(privacy::scrub(&json!(tools)))
            .map_err(|_| ModelError::new(ErrorKind::Parameter, None))?;
        let (mut url, mut body) = match self.endpoint.protocol {
            Protocol::ChatCompletions => {
                let serialized = self.serialize_messages(messages)?;
                let mut obj = json!({
                    "model": self.model, "stream": true, "messages": serialized,
                    "tools": tools.iter().map(|tool| json!({"type": "function", "function": {"name": tool.name, "description": tool.description, "parameters": tool.parameters}})).collect::<Vec<_>>()
                });
                if !self.omit_stream_options {
                    obj.as_object_mut()
                        .expect("known JSON object")
                        .insert("stream_options".into(), json!({"include_usage": true}));
                }
                if let Some(ref cache_key) = self.prompt_cache_key {
                    obj.as_object_mut()
                        .expect("known JSON object")
                        .insert("prompt_cache_key".into(), json!(cache_key));
                }
                (self.endpoint.url("chat/completions")?, obj)
            }
            Protocol::Responses => {
                let instructions = self.join_system_text(messages);
                let input_messages: Vec<&Message> =
                    messages.iter().filter(|m| m.role != "system").collect();
                let serialized = self.serialize_message_refs(&input_messages)?;
                let mut body = json!({
                    "model": self.model, "stream": true, "input": serialized,
                    "tools": tools.iter().map(|tool| json!({"type": "function", "name": tool.name, "description": tool.description, "parameters": tool.parameters})).collect::<Vec<_>>()
                });
                let target = self.endpoint.url("responses")?;
                if is_codex_endpoint_url(&target) {
                    body.as_object_mut()
                        .expect("known JSON object")
                        .insert("store".into(), json!(false));
                }
                if let Some(ref cache_key) = self.prompt_cache_key {
                    body.as_object_mut()
                        .expect("known JSON object")
                        .insert("prompt_cache_key".into(), json!(cache_key));
                }
                if !instructions.trim().is_empty() {
                    body.as_object_mut()
                        .expect("known JSON object")
                        .insert("instructions".into(), json!(instructions));
                }
                (target, body)
            }
            Protocol::AnthropicMessages => {
                let system = self.join_system_text(messages);
                let non_system: Vec<&Message> =
                    messages.iter().filter(|m| m.role != "system").collect();
                let mut conversational = self.serialize_anthropic_messages(&non_system)?;
                let anthropic_max_tokens = self
                    .max_output_tokens
                    .map(|v| v.clamp(256, 131072))
                    .unwrap_or(4096);
                let mut anthropic_tools: Vec<Value> = tools
                    .iter()
                    .map(|tool| {
                        json!({"name": tool.name, "description": tool.description, "input_schema": tool.parameters})
                    })
                    .collect();
                let has_system = !system.trim().is_empty();
                let mut used_breakpoints = 0usize;
                if self.prompt_cache_key.is_some() {
                    if let Some(last_tool) =
                        anthropic_tools.last_mut().and_then(Value::as_object_mut)
                    {
                        last_tool.insert("cache_control".into(), json!({"type": "ephemeral"}));
                        used_breakpoints += 1;
                    }
                    if has_system {
                        used_breakpoints += 1;
                    }
                    let remaining_budget = 4usize.saturating_sub(used_breakpoints);
                    apply_anthropic_rolling_cache_breakpoints(
                        &mut conversational,
                        remaining_budget,
                    );
                }
                let system_value = if self.prompt_cache_key.is_some() && has_system {
                    json!([{
                        "type": "text",
                        "text": system,
                        "cache_control": {"type": "ephemeral"}
                    }])
                } else {
                    json!(system)
                };
                (
                    self.endpoint.url("messages")?,
                    json!({
                        "model": self.model,
                        "stream": true,
                        "max_tokens": anthropic_max_tokens,
                        "system": system_value,
                        "messages": conversational,
                        "tools": anthropic_tools
                    }),
                )
            }
            Protocol::GoogleGemini => {
                let system = self.join_system_text(messages);
                let contents = self.gemini_contents(messages)?;
                let mut body = json!({
                    "contents": contents,
                    "tools": [{
                        "functionDeclarations": tools.iter().map(|tool| {
                            json!({
                                "name": tool.name,
                                "description": tool.description,
                                "parameters": sanitize_gemini_schema(&tool.parameters)
                            })
                        }).collect::<Vec<_>>()
                    }]
                });
                if !system.trim().is_empty() {
                    body.as_object_mut().expect("known JSON object").insert(
                        "systemInstruction".into(),
                        json!({"parts": [{"text": system}]}),
                    );
                }
                let mut target = self
                    .endpoint
                    .url(&format!("models/{}:streamGenerateContent", self.model))?;
                target.set_query(Some("alt=sse"));
                (target, body)
            }
        };
        if tools.is_empty() {
            body.as_object_mut()
                .expect("known JSON object")
                .remove("tools");
        }
        self.apply_generation_options(&mut body);
        let _ = &mut url;
        Ok((url, body))
    }

    fn join_system_text(&self, messages: &[Message]) -> String {
        let joined = messages
            .iter()
            .filter(|message| message.role == "system")
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        privacy::scrub_text(&joined)
    }

    fn serialize_messages(&self, messages: &[Message]) -> Result<Value, ModelError> {
        let refs: Vec<&Message> = messages.iter().collect();
        self.serialize_message_refs(&refs)
    }

    fn serialize_message_refs(&self, messages: &[&Message]) -> Result<Value, ModelError> {
        let serialized = messages
            .iter()
            .map(|message| self.protocol_message(message))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        Ok(privacy::scrub(&Value::Array(serialized)))
    }

    /// Anthropic Messages requires strict user/assistant role alternation:
    /// consecutive `tool_result` (role `user`) blocks from parallel tool calls
    /// must be merged into a single `role: "user"` message with combined content blocks.
    fn serialize_anthropic_messages(&self, messages: &[&Message]) -> Result<Value, ModelError> {
        let raw = messages
            .iter()
            .map(|message| self.protocol_message(message))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut merged: Vec<Value> = Vec::with_capacity(raw.len());
        for item in raw {
            let role = item.get("role").and_then(Value::as_str);
            let prev_same_role = merged
                .last()
                .and_then(|prev| prev.get("role").and_then(Value::as_str))
                == role
                && matches!(role, Some("user" | "assistant"));
            if prev_same_role {
                let Some(prev_obj) = merged.last_mut().and_then(Value::as_object_mut) else {
                    merged.push(item);
                    continue;
                };
                let to_blocks = |v: Value| -> Vec<Value> {
                    match v {
                        Value::Array(arr) => arr,
                        Value::String(s) if !s.is_empty() => {
                            vec![json!({"type": "text", "text": s})]
                        }
                        Value::String(_) => Vec::new(),
                        other => vec![other],
                    }
                };
                let mut combined = prev_obj
                    .remove("content")
                    .map(to_blocks)
                    .unwrap_or_default();
                if let Some(next_content) = item.get("content").cloned() {
                    combined.extend(to_blocks(next_content));
                }
                prev_obj.insert("content".into(), Value::Array(combined));
            } else {
                merged.push(item);
            }
        }
        Ok(privacy::scrub(&Value::Array(merged)))
    }

    fn gemini_contents(&self, messages: &[Message]) -> Result<Value, ModelError> {
        let mut call_names: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut contents: Vec<Value> = Vec::new();
        let mut pending_tool_parts: Vec<Value> = Vec::new();

        let flush_tool_parts = |out: &mut Vec<Value>, parts: &mut Vec<Value>| {
            if !parts.is_empty() {
                out.push(json!({
                    "role": "user",
                    "parts": std::mem::take(parts)
                }));
            }
        };

        for message in messages.iter().filter(|m| m.role != "system") {
            if message.role == "tool" {
                let payload: Value = serde_json::from_str(&message.content)
                    .map_err(|_| ModelError::new(ErrorKind::Parameter, None))?;
                let payload = privacy::scrub(&payload);
                let id = payload["id"]
                    .as_str()
                    .ok_or(ModelError::new(ErrorKind::Parameter, None))?;
                let name = call_names
                    .get(id)
                    .map(String::as_str)
                    .or_else(|| payload.get("name").and_then(Value::as_str))
                    .unwrap_or(id);
                let result = &payload["result"];
                let response_value = if result.is_object() {
                    result.clone()
                } else {
                    json!({ "result": result })
                };
                pending_tool_parts.push(json!({
                    "functionResponse": {
                        "name": name,
                        "response": response_value
                    }
                }));
                continue;
            }
            flush_tool_parts(&mut contents, &mut pending_tool_parts);
            if message.role == "assistant_tool_calls" {
                let payload: Value = serde_json::from_str(&message.content)
                    .map_err(|_| ModelError::new(ErrorKind::Parameter, None))?;
                let calls = payload["tool_calls"]
                    .as_array()
                    .ok_or(ModelError::new(ErrorKind::Parameter, None))?;
                let mut parts = Vec::new();
                if let Some(text) = payload["text"].as_str().filter(|t| !t.is_empty()) {
                    parts.push(json!({"text": text}));
                }
                for call in calls {
                    let call_id = call["id"].as_str().unwrap_or("");
                    let call_name = call["name"].as_str().unwrap_or("");
                    if !call_id.is_empty() && !call_name.is_empty() {
                        call_names.insert(call_id.to_owned(), call_name.to_owned());
                    }
                    parts.push(json!({
                        "functionCall": {
                            "name": call_name,
                            "args": call["arguments"]
                        }
                    }));
                }
                contents.push(json!({
                    "role": "model",
                    "parts": parts
                }));
            } else if message.role == "user_multimodal" {
                let (text, images) = parse_multimodal_user_message(&message.content)?;
                let mut parts = Vec::new();
                if !text.is_empty() || images.is_empty() {
                    parts.push(json!({"text": text}));
                }
                for (mime, data) in images {
                    parts.push(json!({
                        "inline_data": {
                            "mime_type": mime,
                            "data": data
                        }
                    }));
                }
                contents.push(json!({
                    "role": "user",
                    "parts": parts
                }));
            } else {
                let role = if message.role == "assistant" {
                    "model"
                } else {
                    "user"
                };
                contents.push(json!({
                    "role": role,
                    "parts": [{"text": message.content}]
                }));
            }
        }
        flush_tool_parts(&mut contents, &mut pending_tool_parts);
        let mut merged: Vec<Value> = Vec::with_capacity(contents.len());
        for item in contents {
            let role = item.get("role").and_then(Value::as_str);
            let same_role = merged
                .last()
                .and_then(|prev| prev.get("role").and_then(Value::as_str))
                == role
                && matches!(role, Some("user" | "model"));
            if same_role {
                if let (Some(prev_parts), Some(next_parts)) = (
                    merged
                        .last_mut()
                        .and_then(|p| p.get_mut("parts"))
                        .and_then(Value::as_array_mut),
                    item.get("parts").and_then(Value::as_array),
                ) {
                    prev_parts.extend(next_parts.iter().cloned());
                    continue;
                }
            }
            merged.push(item);
        }
        Ok(privacy::scrub(&Value::Array(merged)))
    }

    /// Inline `<think>` / `<thinking>` normalization is active across all
    /// protocols and model families (`ThinkNormalizer`).
    pub fn strips_inline_think(&self) -> bool {
        true
    }

    /// `Off` has a switch for this protocol and model family.
    fn reasoning_switch_applies(&self) -> bool {
        let family = ModelFamily::of(&self.model);
        self.reasoning == ReasoningMode::Off
            && match self.endpoint.protocol {
                Protocol::ChatCompletions => {
                    family == ModelFamily::Glm || family.lowest_effort().is_some()
                }
                Protocol::Responses => family.lowest_effort().is_some(),
                Protocol::AnthropicMessages | Protocol::GoogleGemini => false,
            }
    }

    /// The request carries a reasoning switch: `Off` has one here and the
    /// channel is not known to refuse it.
    pub fn sends_reasoning_switch(&self) -> bool {
        self.reasoning_switch_applies() && !self.reasoning_switch_unsupported
    }

    /// Reasoning was asked to be off, but the channel is known to refuse the
    /// switch: the request carries none and the model thinks.
    pub fn reasoning_switch_refused(&self) -> bool {
        self.reasoning_switch_applies() && self.reasoning_switch_unsupported
    }

    /// Token budget and reasoning switch; the table is on [`ReasoningMode`].
    fn apply_generation_options(&self, body: &mut Value) {
        let object = body.as_object_mut().expect("known JSON object");
        let family = ModelFamily::of(&self.model);
        let off = self.sends_reasoning_switch();
        let configured_output = self.max_output_tokens.map(|v| v.clamp(1, 131072));
        match self.endpoint.protocol {
            Protocol::ChatCompletions => {
                if family == ModelFamily::Glm {
                    // The model thinks when the switch is refused; the
                    // thinking must not use up the answer's room (L4 r9
                    // P1-2).
                    let budget = match (configured_output, self.reasoning_switch_refused()) {
                        (Some(cap), true) => cap.max(8192),
                        (Some(cap), false) => cap,
                        (None, true) => 8192,
                        (None, false) => 4096,
                    };
                    object.insert("max_tokens".into(), json!(budget));
                } else if let Some(cap) = configured_output {
                    object.insert("max_tokens".into(), json!(cap));
                }
                if off && family == ModelFamily::Glm {
                    object.insert("thinking".into(), json!({"type": "disabled"}));
                }
                if let Some(effort) = family.lowest_effort().filter(|_| off) {
                    object.insert("reasoning_effort".into(), json!(effort));
                }
            }
            Protocol::Responses => {
                if let Some(cap) = configured_output {
                    object.insert("max_output_tokens".into(), json!(cap));
                }
                if let Some(effort) = family.lowest_effort().filter(|_| off) {
                    object.insert("reasoning".into(), json!({"effort": effort}));
                }
            }
            Protocol::AnthropicMessages => {}
            Protocol::GoogleGemini => {
                if let Some(cap) = configured_output {
                    let config = object
                        .entry("generationConfig")
                        .or_insert_with(|| json!({}));
                    if let Some(cfg_obj) = config.as_object_mut() {
                        cfg_obj.insert("maxOutputTokens".into(), json!(cap));
                    }
                }
            }
        }
        let Some(effort) = self
            .reasoning_effort
            .as_deref()
            .filter(|_| self.reasoning == ReasoningMode::ProviderDefault)
        else {
            return;
        };
        match self.endpoint.protocol {
            Protocol::ChatCompletions => {
                object.insert("reasoning_effort".into(), json!(effort));
            }
            Protocol::Responses => {
                object.insert("reasoning".into(), json!({"effort": effort}));
            }
            Protocol::AnthropicMessages => {
                if let Ok(budget) = effort.parse::<u64>() {
                    object.insert(
                        "thinking".into(),
                        json!({"type": "enabled", "budget_tokens": budget}),
                    );
                    // The answer still needs room after the thinking.
                    let answer_room = configured_output.unwrap_or(4096);
                    object.insert(
                        "max_tokens".into(),
                        json!(budget.saturating_add(answer_room).min(131072)),
                    );
                }
            }
            Protocol::GoogleGemini => {}
        }
    }

    fn protocol_message(&self, message: &Message) -> Result<Vec<Value>, ModelError> {
        let protocol = self.endpoint.protocol;
        if message.role == "user_multimodal" {
            let (text, images) = parse_multimodal_user_message(&message.content)?;
            if images.is_empty() {
                return Ok(vec![json!({"role": "user", "content": text})]);
            }
            return Ok(vec![match protocol {
                Protocol::ChatCompletions => {
                    let mut parts = Vec::new();
                    if !text.is_empty() {
                        parts.push(json!({"type": "text", "text": text}));
                    }
                    for (mime, data) in images {
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{mime};base64,{data}")
                            }
                        }));
                    }
                    json!({"role": "user", "content": parts})
                }
                Protocol::Responses => {
                    let mut parts = Vec::new();
                    if !text.is_empty() {
                        parts.push(json!({"type": "input_text", "text": text}));
                    }
                    for (mime, data) in images {
                        parts.push(json!({
                            "type": "input_image",
                            "image_url": format!("data:{mime};base64,{data}")
                        }));
                    }
                    json!({"role": "user", "content": parts})
                }
                Protocol::AnthropicMessages => {
                    let mut parts = Vec::new();
                    if !text.is_empty() {
                        parts.push(json!({"type": "text", "text": text}));
                    }
                    for (mime, data) in images {
                        parts.push(json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": mime,
                                "data": data
                            }
                        }));
                    }
                    json!({"role": "user", "content": parts})
                }
                Protocol::GoogleGemini => {
                    let mut parts = Vec::new();
                    if !text.is_empty() {
                        parts.push(json!({"text": text}));
                    }
                    for (mime, data) in images {
                        parts.push(json!({
                            "inline_data": {
                                "mime_type": mime,
                                "data": data
                            }
                        }));
                    }
                    json!({"role": "user", "parts": parts})
                }
            }]);
        }
        if message.role == "assistant_tool_calls" {
            let payload: Value = serde_json::from_str(&message.content)
                .map_err(|_| ModelError::new(ErrorKind::Parameter, None))?;
            let calls = payload["tool_calls"]
                .as_array()
                .ok_or(ModelError::new(ErrorKind::Parameter, None))?;
            return Ok(match protocol {
                Protocol::ChatCompletions => {
                    vec![
                        json!({"role":"assistant", "content": payload["text"], "tool_calls": calls.iter().map(|call| json!({"id":call["id"], "type":"function", "function":{"name":call["name"], "arguments":call["arguments"].to_string()}})).collect::<Vec<_>>()}),
                    ]
                }
                Protocol::Responses => {
                    let mut out = Vec::new();
                    if let Some(text) = payload["text"].as_str().filter(|t| !t.is_empty()) {
                        out.push(json!({"role": "assistant", "content": text}));
                    }
                    out.extend(calls.iter().map(|call| {
                        json!({"type":"function_call", "call_id":call["id"], "name":call["name"], "arguments":call["arguments"].to_string()})
                    }));
                    out
                }
                Protocol::AnthropicMessages => {
                    let mut blocks = Vec::new();
                    if let Some(text) = payload["text"].as_str().filter(|t| !t.is_empty()) {
                        blocks.push(json!({"type": "text", "text": text}));
                    }
                    blocks.extend(calls.iter().map(|call| {
                        json!({"type":"tool_use", "id":call["id"], "name":call["name"], "input":call["arguments"]})
                    }));
                    vec![json!({"role":"assistant", "content": blocks})]
                }
                Protocol::GoogleGemini => {
                    vec![
                        json!({"role":"model", "parts":calls.iter().map(|call| json!({"functionCall":{"name":call["name"], "args":call["arguments"]}})).collect::<Vec<_>>()}),
                    ]
                }
            });
        }
        if message.role == "tool" {
            let payload: Value = serde_json::from_str(&message.content)
                .map_err(|_| ModelError::new(ErrorKind::Parameter, None))?;
            let payload = privacy::scrub(&payload);
            let id = payload["id"]
                .as_str()
                .ok_or(ModelError::new(ErrorKind::Parameter, None))?;
            let result = &payload["result"];
            return Ok(vec![match protocol {
                Protocol::ChatCompletions => {
                    json!({"role":"tool", "tool_call_id":id, "content":result.to_string()})
                }
                Protocol::Responses => {
                    json!({"type":"function_call_output", "call_id":id, "output":result.to_string()})
                }
                Protocol::AnthropicMessages => {
                    json!({"role":"user", "content":[{"type":"tool_result", "tool_use_id":id, "content":result.to_string()}]})
                }
                Protocol::GoogleGemini => {
                    json!({"role":"user", "parts":[{"functionResponse":{"name":id, "response":{"result":result}}}]})
                }
            }]);
        }
        Ok(vec![
            json!({"role":message.role, "content":message.content}),
        ])
    }
}

fn parse_multimodal_user_message(
    content: &str,
) -> Result<(String, Vec<(String, String)>), ModelError> {
    let payload: Value =
        serde_json::from_str(content).map_err(|_| ModelError::new(ErrorKind::Parameter, None))?;
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .map(privacy::scrub_text)
        .unwrap_or_default();
    let mut images = Vec::new();
    if let Some(items) = payload.get("images").and_then(Value::as_array) {
        for item in items {
            let mime = item
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let data = item
                .get("data_base64")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if let Some((norm_mime, norm_b64)) = privacy::normalize_image_attachment(mime, data) {
                images.push((norm_mime, norm_b64));
            }
        }
    }
    Ok((text, images))
}

pub enum ModelProxy<'a> {
    System,
    Direct,
    Manual(&'a str),
}

/// Secrets are borrowed only for one request; never retained, serialized or placed in ModelError.
#[derive(Clone)]
pub struct ModelClient {
    client: Client,
}

impl ModelClient {
    pub fn new(timeout: Duration) -> Result<Self, ModelError> {
        Self::with_proxy(timeout, ModelProxy::System)
    }

    pub fn with_proxy(timeout: Duration, proxy: ModelProxy<'_>) -> Result<Self, ModelError> {
        let connect_timeout = timeout.min(Duration::from_secs(30));
        let builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(connect_timeout)
            .read_timeout(timeout)
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .pool_idle_timeout(Some(Duration::from_secs(90)));
        let builder = match proxy {
            ModelProxy::System => builder,
            ModelProxy::Direct => builder.no_proxy(),
            ModelProxy::Manual(url) => builder.no_proxy().proxy(
                reqwest::Proxy::all(url)
                    .map_err(|_| ModelError::new(ErrorKind::Parameter, None))?,
            ),
        };
        let client = builder
            .build()
            .map_err(|_| ModelError::new(ErrorKind::Unknown, None))?;
        Ok(Self { client })
    }

    fn authenticated(
        &self,
        request: reqwest::RequestBuilder,
        protocol: Protocol,
        key: &str,
        extra_headers: &[(String, String)],
    ) -> reqwest::RequestBuilder {
        let bearer_override = extra_headers.iter().any(|(name, value)| {
            name.trim().eq_ignore_ascii_case("x-eostage-auth-mode")
                && value.trim().eq_ignore_ascii_case("bearer")
        });
        let mut req = match protocol {
            Protocol::AnthropicMessages => {
                let with_ver = request.header("anthropic-version", "2023-06-01");
                if bearer_override {
                    with_ver.bearer_auth(key)
                } else {
                    with_ver.header("x-api-key", key)
                }
            }
            Protocol::GoogleGemini => {
                if bearer_override {
                    request.bearer_auth(key)
                } else {
                    request.header("x-goog-api-key", key)
                }
            }
            _ => request.bearer_auth(key),
        };
        for (name, value) in extra_headers {
            let trimmed_name = name.trim();
            if trimmed_name.is_empty()
                || value.trim().is_empty()
                || trimmed_name.to_ascii_lowercase().starts_with("x-eostage-")
            {
                continue;
            }
            req = req.header(trimmed_name, value.trim());
        }
        req
    }

    /// A 400 to a request that carried a reasoning switch returns
    /// `ErrorKind::ReasoningSwitchRefused` and nothing is resent here: a resend
    /// is a new outbound body, which the caller archives before sending
    /// (`agent::Model::without_reasoning_switch`).
    pub async fn send(
        &self,
        route: &ModelRoute,
        key: &str,
        messages: &[Message],
        tools: &[ToolDescription],
        cancel: &CancellationToken,
        mut emit: impl FnMut(ModelEvent) + Send,
    ) -> Result<Completion, ModelError> {
        let step = messages
            .iter()
            .filter(|m| m.role == "assistant_tool_calls")
            .count();
        let scheme = route.endpoint.base.scheme();
        let mut effective_route = route.clone();
        let mut option_fallbacks = 0u8;
        let mut stripped_prompt_cache_key = false;
        let mut attempt = 0usize;
        loop {
            let mut emitted_any = false;
            let outcome = match self
                .post(&effective_route, key, messages, tools, cancel)
                .await
            {
                Ok(response) if response.status().is_success() => {
                    let mut tracking_emit = |event: ModelEvent| {
                        emitted_any = true;
                        emit(event);
                    };
                    self.decode(&effective_route, step, response, cancel, &mut tracking_emit)
                        .await
                }
                Ok(response) => {
                    let status = response.status().as_u16();
                    let (provider_code, detail) = extract_http_error_diagnostic(response).await;
                    if matches!(status, 400 | 422) && !emitted_any && option_fallbacks < 2 {
                        let diag_lower = format!(
                            "{} {}",
                            provider_code.as_deref().unwrap_or(""),
                            detail.as_deref().unwrap_or("")
                        )
                        .to_ascii_lowercase();
                        if status == 400
                            && effective_route.endpoint.protocol == Protocol::ChatCompletions
                            && !effective_route.omit_stream_options
                            && diag_lower.contains("stream_options")
                        {
                            effective_route.omit_stream_options = true;
                            option_fallbacks += 1;
                            continue;
                        }
                        let cache_key_diag = diag_lower.contains("prompt_cache_key")
                            || (status == 400
                                && !effective_route.sends_reasoning_switch()
                                && (diag_lower.contains("unknown field")
                                    || diag_lower.contains("unrecognized request argument")
                                    || diag_lower.contains("extra_forbidden")
                                    || diag_lower.contains("additional properties")));
                        if matches!(
                            effective_route.endpoint.protocol,
                            Protocol::ChatCompletions | Protocol::Responses
                        ) && effective_route.prompt_cache_key.is_some()
                            && !(effective_route.endpoint.protocol == Protocol::Responses
                                && is_codex_endpoint_url(&effective_route.endpoint.base))
                            && cache_key_diag
                        {
                            effective_route.prompt_cache_key = None;
                            stripped_prompt_cache_key = true;
                            option_fallbacks += 1;
                            continue;
                        }
                        if effective_route.endpoint.protocol == Protocol::AnthropicMessages
                            && effective_route.prompt_cache_key.is_some()
                            && diag_lower.contains("cache_control")
                        {
                            effective_route.prompt_cache_key = None;
                            stripped_prompt_cache_key = true;
                            option_fallbacks += 1;
                            continue;
                        }
                    }
                    let kind = if status == 400 && effective_route.sends_reasoning_switch() {
                        ErrorKind::ReasoningSwitchRefused
                    } else {
                        classify_status(status)
                    };
                    Err(ModelError::new(kind, Some(status)).with_diagnostic(
                        Some("response_status"),
                        None,
                        provider_code.as_deref(),
                        detail.as_deref(),
                    ))
                }
                Err(error) => Err(error),
            };
            match outcome {
                Ok(mut completion) => {
                    if stripped_prompt_cache_key {
                        completion.prompt_cache_key_refused = true;
                    }
                    return Ok(completion);
                }
                Err(err) => {
                    let Some(delay) =
                        pre_stream_transient_retry_delay(scheme, emitted_any, err.kind, attempt)
                    else {
                        return Err(err);
                    };
                    attempt += 1;
                    tokio::select! {
                        () = cancel.cancelled() => {
                            return Err(ModelError::new(ErrorKind::Cancelled, None));
                        }
                        () = tokio::time::sleep(delay) => {}
                    }
                }
            }
        }
    }

    async fn post(
        &self,
        route: &ModelRoute,
        key: &str,
        messages: &[Message],
        tools: &[ToolDescription],
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response, ModelError> {
        if cancel.is_cancelled() {
            return Err(ModelError::new(ErrorKind::Cancelled, None));
        }
        let (url, body) = route.request(messages, tools)?;
        let mut headers = route.extra_headers.clone();
        if is_codex_endpoint_url(&url) {
            if !headers
                .iter()
                .any(|(k, _)| k.trim().eq_ignore_ascii_case("user-agent"))
            {
                headers.push((
                    "User-Agent".to_owned(),
                    format!("codex_cli_rs/{CODEX_CLIENT_VERSION}"),
                ));
            }
            if !headers
                .iter()
                .any(|(k, _)| k.trim().eq_ignore_ascii_case("originator"))
            {
                headers.push(("originator".to_owned(), "codex_cli_rs".to_owned()));
            }
            if !headers
                .iter()
                .any(|(k, _)| k.trim().eq_ignore_ascii_case("accept"))
            {
                headers.push(("Accept".to_owned(), "text/event-stream".to_owned()));
            }
            if let Some(ref cache_key) = route.prompt_cache_key {
                if !headers
                    .iter()
                    .any(|(k, _)| k.trim().eq_ignore_ascii_case("session_id"))
                {
                    headers.push(("session_id".to_owned(), cache_key.clone()));
                }
            }
        }
        let request = self.authenticated(
            self.client.post(url).json(&body),
            route.endpoint.protocol,
            key,
            &headers,
        );
        tokio::select! {
            () = cancel.cancelled() => Err(ModelError::new(ErrorKind::Cancelled, None)),
            response = request.send() => response.map_err(|error| map_transport_error(error, "request_connect")),
        }
    }

    async fn decode(
        &self,
        route: &ModelRoute,
        step: usize,
        response: reqwest::Response,
        cancel: &CancellationToken,
        emit: &mut (impl FnMut(ModelEvent) + Send),
    ) -> Result<Completion, ModelError> {
        let mut decoder = StreamDecoder::for_route(route).with_step(step);
        let mut stream = response.bytes_stream();
        let mut incomplete = Vec::new();
        while let Some(chunk) = tokio::select! {
            () = cancel.cancelled() => return Err(ModelError::new(ErrorKind::Cancelled, None)),
            chunk = stream.next() => chunk,
        } {
            let chunk = chunk.map_err(|error| map_transport_error(error, "stream_read"))?;
            if chunk.len().saturating_add(incomplete.len()) > MAX_RESPONSE_BYTES {
                return Err(ModelError::new(ErrorKind::Capability, None));
            }
            incomplete.extend_from_slice(&chunk);
            let valid = match std::str::from_utf8(&incomplete) {
                Ok(_) => incomplete.len(),
                Err(error) if error.error_len().is_none() => error.valid_up_to(),
                Err(_) => return Err(ModelError::new(ErrorKind::Capability, None)),
            };
            if valid > 0 {
                let text = std::str::from_utf8(&incomplete[..valid])
                    .map_err(|_| ModelError::new(ErrorKind::Capability, None))?
                    .to_owned();
                decoder.feed(&text, cancel, emit)?;
                incomplete.drain(..valid);
            }
        }
        if !incomplete.is_empty() {
            return Err(ModelError::new(ErrorKind::Transient, None).with_diagnostic(
                Some("stream_decode"),
                Some("stream_interrupted"),
                None,
                Some("流式响应在 UTF-8 字符未完整闭合时意外结束"),
            ));
        }
        decoder.finish(cancel, emit)
    }

    /// Fetch model IDs; response has no secret-bearing diagnostic body in errors.
    pub async fn models(
        &self,
        endpoint: &Endpoint,
        key: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<String>, ModelError> {
        self.models_with_headers(endpoint, key, &[], cancel).await
    }

    /// Fetch model IDs with optional provider-specific headers (e.g. OAuth account headers).
    pub async fn models_with_headers(
        &self,
        endpoint: &Endpoint,
        key: &str,
        extra_headers: &[(String, String)],
        cancel: &CancellationToken,
    ) -> Result<Vec<String>, ModelError> {
        let mut target_url = endpoint.url("models")?;
        let mut headers = extra_headers.to_vec();
        let is_codex_endpoint = is_codex_endpoint_url(&target_url);
        if is_codex_endpoint {
            if !target_url.query_pairs().any(|(k, _)| k == "client_version") {
                target_url
                    .query_pairs_mut()
                    .append_pair("client_version", CODEX_CLIENT_VERSION);
            }
            if !headers
                .iter()
                .any(|(k, _)| k.trim().eq_ignore_ascii_case("accept"))
            {
                headers.push(("Accept".to_owned(), "application/json".to_owned()));
            }
            if !headers
                .iter()
                .any(|(k, _)| k.trim().eq_ignore_ascii_case("user-agent"))
            {
                headers.push((
                    "User-Agent".to_owned(),
                    format!("codex_cli_rs/{CODEX_CLIENT_VERSION}"),
                ));
            }
            if !headers
                .iter()
                .any(|(k, _)| k.trim().eq_ignore_ascii_case("originator"))
            {
                headers.push(("originator".to_owned(), "codex_cli_rs".to_owned()));
            }
        }
        let request = self.authenticated(
            self.client.get(target_url),
            endpoint.protocol,
            key,
            &headers,
        );
        let response = tokio::select! {
            () = cancel.cancelled() => return Err(ModelError::new(ErrorKind::Cancelled, None)),
            response = request.send() => response.map_err(|error| map_transport_error(error, "models_connect"))?,
        };
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let (provider_code, detail) = extract_http_error_diagnostic(response).await;
            return Err(
                ModelError::new(classify_status(status), Some(status)).with_diagnostic(
                    Some("models_list"),
                    None,
                    provider_code.as_deref(),
                    detail.as_deref(),
                ),
            );
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
        {
            return Err(ModelError::new(ErrorKind::Capability, None));
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::select! {
            () = cancel.cancelled() => return Err(ModelError::new(ErrorKind::Cancelled, None)),
            chunk = stream.next() => chunk,
        } {
            let chunk = chunk.map_err(|error| map_transport_error(error, "models_read"))?;
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(ModelError::new(ErrorKind::Capability, None));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ModelError::new(ErrorKind::Capability, None))?;
        let values = body["data"]
            .as_array()
            .or_else(|| body["models"].as_array())
            .ok_or(ModelError::new(ErrorKind::Capability, None))?;
        Ok(values
            .iter()
            .filter_map(|model| {
                if endpoint.protocol == Protocol::GoogleGemini {
                    if let Some(methods) = model["supportedGenerationMethods"].as_array() {
                        let supports_gen = methods
                            .iter()
                            .filter_map(Value::as_str)
                            .any(|m| m == "generateContent");
                        if !supports_gen {
                            return None;
                        }
                    }
                    let raw = model["name"]
                        .as_str()
                        .or_else(|| model["id"].as_str())
                        .or_else(|| model.as_str())?;
                    return Some(raw.strip_prefix("models/").unwrap_or(raw));
                }
                model["id"]
                    .as_str()
                    .or_else(|| model["slug"].as_str())
                    .or_else(|| model["model"].as_str())
                    .or_else(|| {
                        model["name"]
                            .as_str()
                            .map(|s| s.strip_prefix("models/").unwrap_or(s))
                    })
                    .or_else(|| model.as_str())
            })
            .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
            .take(MAX_MODEL_IDS)
            .map(str::to_owned)
            .collect())
    }

    pub async fn test_connection(
        &self,
        endpoint: &Endpoint,
        key: &str,
        cancel: &CancellationToken,
    ) -> Result<(), ModelError> {
        self.models(endpoint, key, cancel).await.map(|_| ())
    }
}

fn sanitize_gemini_schema(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if k == "$schema" || k == "additionalProperties" {
                    continue;
                }
                out.insert(k.clone(), sanitize_gemini_schema(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_gemini_schema).collect()),
        other => other.clone(),
    }
}

fn pre_stream_transient_retry_delay(
    scheme: &str,
    emitted_any: bool,
    kind: ErrorKind,
    attempt: usize,
) -> Option<Duration> {
    const HTTPS_TRANSIENT_DELAYS_MS: &[u64] = &[500, 1_200];
    if scheme != "https" || emitted_any || kind != ErrorKind::Transient {
        return None;
    }
    HTTPS_TRANSIENT_DELAYS_MS
        .get(attempt)
        .copied()
        .map(Duration::from_millis)
}

async fn extract_http_error_diagnostic(
    response: reqwest::Response,
) -> (Option<String>, Option<String>) {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(Ok(chunk)) = stream.next().await {
        let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(bytes.len());
        if remaining == 0 {
            break;
        }
        let take = chunk.len().min(remaining);
        bytes.extend_from_slice(&chunk[..take]);
        if bytes.len() >= MAX_ERROR_BODY_BYTES {
            break;
        }
    }
    if bytes.is_empty() {
        return (None, None);
    }
    let text = String::from_utf8_lossy(&bytes);
    if let Ok(val) = serde_json::from_str::<Value>(&text) {
        let root = val.as_array().and_then(|arr| arr.first()).unwrap_or(&val);
        let err_obj = root.get("error").filter(|v| !v.is_null()).unwrap_or(root);
        let code = err_obj["code"]
            .as_str()
            .or_else(|| err_obj["status"].as_str())
            .or_else(|| err_obj["type"].as_str())
            .or_else(|| root["code"].as_str())
            .or_else(|| root["type"].as_str())
            .map(crate::sanitize_diagnostic_text)
            .filter(|s| !s.is_empty());
        let detail = err_obj["message"]
            .as_str()
            .or_else(|| err_obj["error"].as_str())
            .or_else(|| root["message"].as_str())
            .or_else(|| root["detail"].as_str())
            .or_else(|| err_obj["detail"].as_str())
            .or_else(|| err_obj.as_str())
            .map(crate::sanitize_diagnostic_text)
            .filter(|s| !s.is_empty());
        return (code, detail);
    }
    let clean = crate::sanitize_diagnostic_text(text.trim());
    (None, (!clean.is_empty()).then_some(clean))
}

fn map_transport_error(error: reqwest::Error, stage: &str) -> ModelError {
    use std::error::Error as _;
    let top_msg = error.to_string();
    let mut source_parts: Vec<String> = Vec::new();
    let mut cur = error.source();
    while let Some(src) = cur {
        let text = src.to_string();
        if !text.trim().is_empty() && source_parts.last().map(String::as_str) != Some(text.as_str())
        {
            source_parts.push(text);
        }
        cur = src.source();
    }
    let full_chain = if source_parts.is_empty() {
        top_msg.clone()
    } else {
        format!("{top_msg}: {}", source_parts.join(": "))
    };
    let lower = full_chain.to_ascii_lowercase();
    let (cause, kind) = if error.is_timeout() || lower.contains("timed out") {
        ("timeout", ErrorKind::Transient)
    } else if lower.contains("dns")
        || lower.contains("failed to lookup")
        || lower.contains("nodename nor servname")
        || lower.contains("name or service not known")
    {
        ("dns", ErrorKind::Transient)
    } else if lower.contains("tls")
        || lower.contains("ssl")
        || lower.contains("certificate")
        || lower.contains("handshake")
    {
        ("tls", ErrorKind::Unknown)
    } else if lower.contains("proxy") {
        ("proxy", ErrorKind::Transient)
    } else if lower.contains("unexpected eof")
        || lower.contains("incomplete message")
        || lower.contains("connection reset")
        || lower.contains("connection closed")
        || lower.contains("connection aborted")
        || lower.contains("broken pipe")
        || lower.contains("goaway")
        || lower.contains("stream closed")
        || lower.contains("stream reset")
        || lower.contains("reset by peer")
        || lower.contains("eof")
        || error.is_body()
        || error.is_decode()
    {
        ("stream_interrupted", ErrorKind::Transient)
    } else if error.is_connect() || lower.contains("connection refused") {
        ("connect", ErrorKind::Transient)
    } else {
        ("transport", ErrorKind::Unknown)
    };
    let detail = if !source_parts.is_empty() {
        source_parts.join(": ")
    } else {
        let stripped = top_msg.split(": ").skip(1).collect::<Vec<_>>().join(": ");
        if stripped.trim().is_empty() {
            top_msg
        } else {
            stripped
        }
    };
    let mut mapped = ModelError::new(kind, error.status().map(|status| status.as_u16()))
        .with_diagnostic(Some(stage), Some(cause), None, Some(&detail));
    mapped.transport_timeout = error.is_timeout() || cause == "timeout";
    mapped
}

fn attach_ephemeral_cache_control(msg: &mut Value) {
    let Some(obj) = msg.as_object_mut() else {
        return;
    };
    let Some(content) = obj.get_mut("content") else {
        return;
    };
    match content {
        Value::String(s) if !s.trim().is_empty() => {
            let text = std::mem::take(s);
            *content = json!([{
                "type": "text",
                "text": text,
                "cache_control": {"type": "ephemeral"}
            }]);
        }
        Value::Array(arr) => {
            for block in arr.iter_mut().rev() {
                let Some(block_obj) = block.as_object_mut() else {
                    continue;
                };
                let is_empty_text = block_obj.get("type").and_then(Value::as_str) == Some("text")
                    && block_obj
                        .get("text")
                        .and_then(Value::as_str)
                        .is_none_or(|t| t.trim().is_empty());
                if is_empty_text {
                    continue;
                }
                block_obj.insert("cache_control".into(), json!({"type": "ephemeral"}));
                break;
            }
        }
        _ => {}
    }
}

fn apply_anthropic_rolling_cache_breakpoints(conversational: &mut Value, remaining_budget: usize) {
    let Some(msgs) = conversational.as_array_mut() else {
        return;
    };
    let len = msgs.len();
    if len >= 3 && remaining_budget >= 2 {
        attach_ephemeral_cache_control(&mut msgs[len - 3]);
        attach_ephemeral_cache_control(&mut msgs[len - 1]);
    } else if len >= 1 && remaining_budget >= 1 {
        attach_ephemeral_cache_control(&mut msgs[len - 1]);
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn pre_stream_retry_applies_only_to_unstreamed_https_transient_errors() {
        assert_eq!(
            pre_stream_transient_retry_delay("https", false, ErrorKind::Transient, 0),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            pre_stream_transient_retry_delay("https", false, ErrorKind::Transient, 1),
            Some(Duration::from_millis(1_200))
        );
        assert_eq!(
            pre_stream_transient_retry_delay("https", false, ErrorKind::Transient, 2),
            None
        );
        // Loopback HTTP tests remain 100% single-shot
        assert_eq!(
            pre_stream_transient_retry_delay("http", false, ErrorKind::Transient, 0),
            None
        );
        // Never retry once any ModelEvent has been emitted
        assert_eq!(
            pre_stream_transient_retry_delay("https", true, ErrorKind::Transient, 0),
            None
        );
        // Non-transient error classes never retry here
        for kind in [
            ErrorKind::ReasoningSwitchRefused,
            ErrorKind::Parameter,
            ErrorKind::Cancelled,
            ErrorKind::Unknown,
        ] {
            assert_eq!(
                pre_stream_transient_retry_delay("https", false, kind, 0),
                None
            );
        }
    }
}
