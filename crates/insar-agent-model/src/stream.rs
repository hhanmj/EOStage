use std::collections::BTreeMap;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::{
    think_normalizer::ThinkNormalizer, transport::ModelRoute, Completion, ErrorKind, ModelError,
    ModelEvent, Protocol, ToolCall,
};

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_TOOL_ID_LEN: usize = 64;

#[derive(Default)]
struct PendingTool {
    id: String,
    name: String,
    arguments: String,
}

pub struct StreamDecoder {
    protocol: Protocol,
    step: usize,
    buffer: String,
    event: String,
    data: String,
    tools: BTreeMap<usize, PendingTool>,
    normalizer: ThinkNormalizer,
    completion: Completion,
    finished: bool,
    saw_terminal_marker: bool,
}

impl StreamDecoder {
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            step: 0,
            buffer: String::new(),
            event: String::new(),
            data: String::new(),
            tools: BTreeMap::new(),
            normalizer: ThinkNormalizer::new(),
            completion: Completion::default(),
            finished: false,
            saw_terminal_marker: false,
        }
    }

    /// Always-on [`ThinkNormalizer`] across all routes and model families.
    pub fn for_route(route: &ModelRoute) -> Self {
        let _ = route.strips_inline_think();
        Self::new(route.endpoint.protocol)
    }

    /// Sets the 0-based turn step index used when normalizing empty or
    /// non-conforming tool call IDs (`call_{step}_{idx}`).
    pub fn with_step(mut self, step: usize) -> Self {
        self.step = step;
        self
    }

    pub fn feed(
        &mut self,
        bytes: &str,
        cancel: &CancellationToken,
        emit: &mut impl FnMut(ModelEvent),
    ) -> Result<(), ModelError> {
        if cancel.is_cancelled() {
            return Err(ModelError::new(ErrorKind::Cancelled, None));
        }
        if self.buffer.len().saturating_add(bytes.len()) > MAX_FRAME_BYTES {
            return Err(
                ModelError::new(ErrorKind::Capability, None).with_diagnostic(
                    Some("stream_decode"),
                    None,
                    None,
                    Some("单条流式数据帧体积超过上限"),
                ),
            );
        }
        self.buffer.push_str(bytes);
        while let Some(position) = self.buffer.find('\n') {
            let line = self.buffer[..position].trim_end_matches('\r').to_owned();
            self.buffer.drain(..=position);
            if line.is_empty() {
                self.dispatch(emit)?;
            } else if let Some(event) = line.strip_prefix("event:") {
                self.event = event.trim().to_owned();
            } else if let Some(data) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(data.trim_start());
                if self.data.len() > MAX_FRAME_BYTES {
                    return Err(
                        ModelError::new(ErrorKind::Capability, None).with_diagnostic(
                            Some("stream_decode"),
                            None,
                            None,
                            Some("单条流式事件载荷超过上限"),
                        ),
                    );
                }
            }
            if cancel.is_cancelled() {
                return Err(ModelError::new(ErrorKind::Cancelled, None));
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, emit: &mut impl FnMut(ModelEvent)) -> Result<(), ModelError> {
        if self.data.is_empty() {
            self.event.clear();
            return Ok(());
        }
        let data = std::mem::take(&mut self.data);
        let event = std::mem::take(&mut self.event);
        let trimmed = data.trim();
        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("ping")
            || trimmed.eq_ignore_ascii_case("keepalive")
            || trimmed.eq_ignore_ascii_case("[ping]")
            || trimmed.eq_ignore_ascii_case("[keepalive]")
        {
            return Ok(());
        }
        if trimmed == "[DONE]" {
            self.finished = true;
            return Ok(());
        }
        let value: Value = serde_json::from_str(trimmed).map_err(|_| {
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                ModelError::new(ErrorKind::Transient, None).with_diagnostic(
                    Some("stream_decode"),
                    Some("stream_interrupted"),
                    None,
                    Some("上游流式 JSON 数据帧在接收完成前中断"),
                )
            } else {
                let preview = crate::sanitize_diagnostic_text(trimmed);
                let detail = if preview.is_empty() {
                    "上游返回了无法解析的非 JSON 流式帧".to_owned()
                } else {
                    format!("上游返回非 JSON 流式内容：{preview}")
                };
                ModelError::new(ErrorKind::Unknown, None).with_diagnostic(
                    Some("stream_decode"),
                    None,
                    None,
                    Some(&detail),
                )
            }
        })?;
        let effective_event = if event.is_empty() || event == "message" {
            value.get("type").and_then(Value::as_str).unwrap_or("")
        } else {
            event.as_str()
        };
        let has_non_null_error = value.get("error").is_some_and(|e| !e.is_null());
        if effective_event == "error"
            || has_non_null_error
            || (self.protocol == Protocol::Responses && effective_event == "response.failed")
        {
            let error =
                if self.protocol == Protocol::Responses && effective_event == "response.failed" {
                    value
                        .pointer("/response/error")
                        .filter(|v| !v.is_null())
                        .or_else(|| value.get("error").filter(|v| !v.is_null()))
                        .unwrap_or(&value)
                } else {
                    value
                        .get("error")
                        .filter(|v| !v.is_null())
                        .unwrap_or(&value)
                };
            let numeric_status = error["status"]
                .as_u64()
                .or_else(|| error["code"].as_u64())
                .and_then(|n| u16::try_from(n).ok())
                .filter(|s| (400..=599).contains(s));
            let err_type = error["type"].as_str().unwrap_or("");
            let err_code = error["code"].as_str().unwrap_or("");
            let err_status_str = error["status"].as_str().unwrap_or("");
            let mut kind = classify_type(err_type);
            if kind == ErrorKind::Unknown {
                kind = classify_type(err_code);
            }
            if kind == ErrorKind::Unknown {
                kind = classify_type(err_status_str);
            }
            if kind == ErrorKind::Unknown {
                if let Some(status) = numeric_status {
                    kind = classify_status(status);
                }
            }
            let raw_code = error["code"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| error["type"].as_str().filter(|s| !s.trim().is_empty()))
                .or_else(|| error["status"].as_str().filter(|s| !s.trim().is_empty()))
                .or_else(|| value["code"].as_str().filter(|s| !s.trim().is_empty()))
                .or_else(|| {
                    value["type"].as_str().filter(|s| {
                        !s.trim().is_empty() && *s != "error" && *s != "response.failed"
                    })
                });
            let raw_message = error["message"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| error["detail"].as_str().filter(|s| !s.trim().is_empty()))
                .or_else(|| error.as_str().filter(|s| !s.trim().is_empty()))
                .or_else(|| value["message"].as_str().filter(|s| !s.trim().is_empty()))
                .or_else(|| value["detail"].as_str().filter(|s| !s.trim().is_empty()));
            return Err(ModelError::new(kind, numeric_status).with_diagnostic(
                Some("stream_event"),
                None,
                raw_code,
                raw_message,
            ));
        }
        match self.protocol {
            Protocol::ChatCompletions => self.chat(&value, emit),
            Protocol::Responses => self.responses(effective_event, &value, emit),
            Protocol::AnthropicMessages => self.anthropic(effective_event, &value, emit),
            Protocol::GoogleGemini => self.gemini(&value, emit),
        }
        if self.normalizer.text_len() > MAX_FRAME_BYTES
            || self.normalizer.reasoning_len() > MAX_FRAME_BYTES
            || self.tools.len() > 64
            || self.tools.values().any(|tool| {
                tool.id.len() > 256
                    || tool.name.len() > 256
                    || tool.arguments.len() > MAX_FRAME_BYTES
            })
        {
            return Err(
                ModelError::new(ErrorKind::Capability, None).with_diagnostic(
                    Some("stream_decode"),
                    None,
                    None,
                    Some("模型单步流式输出或工具调用体积超过安全上限"),
                ),
            );
        }
        Ok(())
    }

    fn chat(&mut self, value: &Value, emit: &mut impl FnMut(ModelEvent)) {
        // Some providers send `"usage": null` on every chunk; only an object counts.
        // Merge non-null fields so a trailing partial usage chunk never wipes
        // previously reported cached_tokens.
        if let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) {
            if let Some(v) = usage
                .get("prompt_tokens")
                .or_else(|| usage.get("input_tokens"))
                .and_then(Value::as_u64)
            {
                self.completion.usage.input_tokens = Some(v);
            }
            if let Some(v) = usage
                .get("completion_tokens")
                .or_else(|| usage.get("output_tokens"))
                .and_then(Value::as_u64)
            {
                self.completion.usage.output_tokens = Some(v);
            }
            if let Some(v) = usage
                .pointer("/prompt_tokens_details/cached_tokens")
                .or_else(|| usage.get("prompt_cache_hit_tokens"))
                .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
                .or_else(|| usage.get("cache_read_input_tokens"))
                .or_else(|| usage.get("cached_tokens"))
                .and_then(Value::as_u64)
            {
                self.completion.usage.cached_input_tokens = Some(v);
            }
            if let Some(v) = usage
                .get("cache_creation_input_tokens")
                .or_else(|| usage.pointer("/prompt_tokens_details/cache_write_tokens"))
                .and_then(Value::as_u64)
            {
                self.completion.usage.cache_creation_input_tokens = Some(v);
            }
            if let Some(v) = usage
                .pointer("/completion_tokens_details/reasoning_tokens")
                .or_else(|| usage.pointer("/output_tokens_details/reasoning_tokens"))
                .and_then(Value::as_u64)
            {
                self.completion.usage.reasoning_tokens = Some(v);
            }
        }
        for choice in value
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(finish_reason) = choice.get("finish_reason").and_then(Value::as_str) {
                if !finish_reason.is_empty() {
                    self.saw_terminal_marker = true;
                }
                if finish_reason == "length" {
                    self.completion.truncated = true;
                }
            }
            let delta = &choice["delta"];
            // Dedicated SSE reasoning fields across DeepSeek/Qwen/GLM/vLLM (`reasoning_content`),
            // OpenRouter/LiteLLM (`reasoning`), and local proxies (`thought`, `thinking`).
            // Use `find_map` over non-empty aliases so proxies emitting both `reasoning`
            // and `reasoning_content` on the same delta are never double-counted.
            if let Some(thought) = ["reasoning_content", "reasoning", "thought", "thinking"]
                .iter()
                .find_map(|field| {
                    delta
                        .get(*field)
                        .and_then(|v| {
                            v.as_str().or_else(|| {
                                v.get("text")
                                    .or_else(|| v.get("content"))
                                    .and_then(Value::as_str)
                            })
                        })
                        .filter(|s| !s.is_empty())
                })
            {
                self.normalizer.push_reasoning(thought, emit);
            }
            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                self.normalizer.push_content(content, emit);
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                if !tool_calls.is_empty() {
                    self.normalizer.on_tool_delta(emit);
                }
                for (pos, tool) in tool_calls.iter().enumerate() {
                    let index = tool
                        .get("index")
                        .and_then(Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok())
                        .unwrap_or(pos);
                    let pending = self.tools.entry(index).or_default();
                    if let Some(id) = tool.get("id").and_then(Value::as_str) {
                        pending.id.push_str(id);
                    }
                    if let Some(name) = tool.pointer("/function/name").and_then(Value::as_str) {
                        pending.name.push_str(name);
                    }
                    if let Some(args) = tool.pointer("/function/arguments").and_then(Value::as_str)
                    {
                        pending.arguments.push_str(args);
                    }
                }
            }
        }
    }

    fn responses(&mut self, event: &str, value: &Value, emit: &mut impl FnMut(ModelEvent)) {
        match event {
            "response.output_text.delta" => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    self.normalizer.push_content(delta, emit);
                }
            }
            e if e == "response.reasoning_text.delta"
                || e == "response.reasoning_summary_text.delta"
                || (e.starts_with("response.reasoning") && e.ends_with("_text.delta")) =>
            {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    self.normalizer.push_reasoning(delta, emit);
                }
            }
            "response.output_item.added" => {
                if let Some(index) = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                {
                    let item = &value["item"];
                    if item["type"] == "function_call" {
                        self.normalizer.on_tool_delta(emit);
                        let pending = self.tools.entry(index).or_default();
                        pending.id = item["call_id"].as_str().unwrap_or("").to_owned();
                        pending.name = item["name"].as_str().unwrap_or("").to_owned();
                        pending.arguments = item["arguments"].as_str().unwrap_or("").to_owned();
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(index) = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                {
                    self.normalizer.on_tool_delta(emit);
                    self.tools
                        .entry(index)
                        .or_default()
                        .arguments
                        .push_str(value["delta"].as_str().unwrap_or(""));
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(index) = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                {
                    let pending = self.tools.entry(index).or_default();
                    if pending.arguments.is_empty() {
                        if let Some(args) = value.get("arguments").and_then(Value::as_str) {
                            pending.arguments.push_str(args);
                        }
                    }
                }
            }
            "response.incomplete" => {
                self.completion.truncated = value
                    .pointer("/response/incomplete_details/reason")
                    .or_else(|| value.pointer("/incomplete_details/reason"))
                    .and_then(Value::as_str)
                    == Some("max_output_tokens");
                self.merge_responses_usage(value);
                self.finished = true;
            }
            "response.completed" | "response.done" => {
                self.merge_responses_usage(value);
                self.finished = true;
            }
            _ => {}
        }
    }

    fn merge_responses_usage(&mut self, value: &Value) {
        let Some(usage) = value
            .pointer("/response/usage")
            .filter(|u| u.is_object())
            .or_else(|| value.get("usage").filter(|u| u.is_object()))
        else {
            return;
        };
        if let Some(v) = usage
            .get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .and_then(Value::as_u64)
        {
            self.completion.usage.input_tokens = Some(v);
        }
        if let Some(v) = usage
            .get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .and_then(Value::as_u64)
        {
            self.completion.usage.output_tokens = Some(v);
        }
        if let Some(v) = usage
            .pointer("/input_tokens_details/cached_tokens")
            .or_else(|| usage.pointer("/prompt_tokens_details/cached_tokens"))
            .or_else(|| usage.get("prompt_cache_hit_tokens"))
            .or_else(|| usage.get("cache_read_input_tokens"))
            .or_else(|| usage.get("cached_tokens"))
            .and_then(Value::as_u64)
        {
            self.completion.usage.cached_input_tokens = Some(v);
        }
        if let Some(v) = usage
            .get("cache_creation_input_tokens")
            .or_else(|| usage.pointer("/input_tokens_details/cache_write_tokens"))
            .or_else(|| usage.pointer("/prompt_tokens_details/cache_write_tokens"))
            .and_then(Value::as_u64)
        {
            self.completion.usage.cache_creation_input_tokens = Some(v);
        }
        if let Some(v) = usage
            .pointer("/output_tokens_details/reasoning_tokens")
            .or_else(|| usage.pointer("/completion_tokens_details/reasoning_tokens"))
            .and_then(Value::as_u64)
        {
            self.completion.usage.reasoning_tokens = Some(v);
        }
    }

    fn anthropic(&mut self, event: &str, value: &Value, emit: &mut impl FnMut(ModelEvent)) {
        match event {
            "message_start" => {
                self.completion.usage.input_tokens = value
                    .pointer("/message/usage/input_tokens")
                    .and_then(Value::as_u64);
                if let Some(cached) = value
                    .pointer("/message/usage/cache_read_input_tokens")
                    .or_else(|| value.pointer("/message/usage/input_tokens_details/cached_tokens"))
                    .and_then(Value::as_u64)
                {
                    self.completion.usage.cached_input_tokens = Some(cached);
                }
                if let Some(cache_creation) = value
                    .pointer("/message/usage/cache_creation_input_tokens")
                    .and_then(Value::as_u64)
                {
                    self.completion.usage.cache_creation_input_tokens = Some(cache_creation);
                }
            }
            "content_block_start" => {
                let index = value["index"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok());
                let block = &value["content_block"];
                if block["type"] == "thinking" {
                    if let Some(thinking) = block.get("thinking").and_then(Value::as_str) {
                        self.normalizer.push_reasoning(thinking, emit);
                    }
                } else if block["type"] == "text" {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        self.normalizer.push_content(text, emit);
                    }
                } else if block["type"] == "tool_use" {
                    self.normalizer.on_tool_delta(emit);
                    if let Some(index) = index {
                        let pending = self.tools.entry(index).or_default();
                        pending.id = block["id"].as_str().unwrap_or("").to_owned();
                        pending.name = block["name"].as_str().unwrap_or("").to_owned();
                        pending.arguments = block
                            .get("input")
                            .filter(|input| !input.is_null() && **input != json!({}))
                            .map(Value::to_string)
                            .unwrap_or_else(|| "{}".to_owned());
                    }
                }
            }
            "content_block_delta" => {
                let delta = &value["delta"];
                if delta["type"] == "thinking_delta" {
                    if let Some(thinking) = delta
                        .get("thinking")
                        .or_else(|| delta.get("text"))
                        .and_then(Value::as_str)
                    {
                        self.normalizer.push_reasoning(thinking, emit);
                    }
                } else if delta["type"] == "text_delta" {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        self.normalizer.push_content(text, emit);
                    }
                } else if delta["type"] == "input_json_delta" {
                    self.normalizer.on_tool_delta(emit);
                    if let Some(index) = value["index"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                    {
                        let pending = self.tools.entry(index).or_default();
                        if pending.arguments == "{}" {
                            pending.arguments.clear();
                        }
                        pending
                            .arguments
                            .push_str(delta["partial_json"].as_str().unwrap_or(""));
                    }
                }
            }
            "message_delta" => {
                if value.pointer("/delta/stop_reason").and_then(Value::as_str) == Some("max_tokens")
                {
                    self.completion.truncated = true;
                }
                if let Some(input) = value.pointer("/usage/input_tokens").and_then(Value::as_u64) {
                    self.completion.usage.input_tokens = Some(input);
                }
                if let Some(cached) = value
                    .pointer("/usage/cache_read_input_tokens")
                    .or_else(|| value.pointer("/usage/input_tokens_details/cached_tokens"))
                    .and_then(Value::as_u64)
                {
                    self.completion.usage.cached_input_tokens = Some(cached);
                }
                if let Some(cache_creation) = value
                    .pointer("/usage/cache_creation_input_tokens")
                    .and_then(Value::as_u64)
                {
                    self.completion.usage.cache_creation_input_tokens = Some(cache_creation);
                }
                self.completion.usage.output_tokens = value
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_u64);
            }
            "message_stop" => self.finished = true,
            _ => {}
        }
    }

    fn gemini(&mut self, value: &Value, emit: &mut impl FnMut(ModelEvent)) {
        if let Some(usage) = value.get("usageMetadata").filter(|u| u.is_object()) {
            if let Some(input) = usage.get("promptTokenCount").and_then(Value::as_u64) {
                self.completion.usage.input_tokens = Some(input);
            }
            if let Some(output) = usage.get("candidatesTokenCount").and_then(Value::as_u64) {
                self.completion.usage.output_tokens = Some(output);
            }
            if let Some(cached) = usage.get("cachedContentTokenCount").and_then(Value::as_u64) {
                self.completion.usage.cached_input_tokens = Some(cached);
            }
            if let Some(thoughts) = usage.get("thoughtsTokenCount").and_then(Value::as_u64) {
                self.completion.usage.reasoning_tokens = Some(thoughts);
            }
        }
        for candidate in value
            .get("candidates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(parts) = candidate
                .pointer("/content/parts")
                .and_then(Value::as_array)
            {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        if part.get("thought").and_then(Value::as_bool) == Some(true) {
                            self.normalizer.push_reasoning(text, emit);
                        } else {
                            self.normalizer.push_content(text, emit);
                        }
                    }
                    if let Some(call) = part.get("functionCall").filter(|c| c.is_object()) {
                        let Some(name) = call
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        else {
                            continue;
                        };
                        self.normalizer.on_tool_delta(emit);
                        let idx = self.tools.len();
                        let id = call
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let args = call
                            .get("args")
                            .filter(|a| !a.is_null())
                            .map(Value::to_string)
                            .unwrap_or_else(|| "{}".to_owned());
                        self.tools.insert(
                            idx,
                            PendingTool {
                                id,
                                name: name.to_owned(),
                                arguments: args,
                            },
                        );
                    }
                }
            }
            if let Some(reason) = candidate
                .get("finishReason")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                if reason == "MAX_TOKENS" {
                    self.completion.truncated = true;
                }
                self.saw_terminal_marker = true;
                self.finished = true;
            }
        }
    }

    pub fn finish(
        mut self,
        cancel: &CancellationToken,
        emit: &mut impl FnMut(ModelEvent),
    ) -> Result<Completion, ModelError> {
        if cancel.is_cancelled() {
            return Err(ModelError::new(ErrorKind::Cancelled, None));
        }
        // Flush a final complete SSE line/event when an upstream proxy closes
        // cleanly right after the final `data:` line without a trailing blank line.
        if !self.buffer.is_empty() {
            let tail = self.buffer.trim_end_matches('\r').trim();
            if tail.is_empty() || tail.starts_with(':') {
                self.buffer.clear();
            } else if let Some(data) = tail.strip_prefix("data:") {
                let payload = data.trim();
                let complete =
                    payload == "[DONE]" || serde_json::from_str::<Value>(payload).is_ok();
                if complete {
                    if !self.data.is_empty() {
                        self.data.push('\n');
                    }
                    self.data.push_str(payload);
                    self.buffer.clear();
                }
            }
        }
        if self.buffer.is_empty() && !self.data.is_empty() {
            self.dispatch(emit)?;
        }
        let has_payload = self.normalizer.text_len() > 0
            || self.normalizer.reasoning_len() > 0
            || !self.tools.is_empty()
            || self.saw_terminal_marker;
        if !self.buffer.is_empty() || !self.data.is_empty() || (!self.finished && !has_payload) {
            return Err(ModelError::new(ErrorKind::Transient, None).with_diagnostic(
                Some("stream_read"),
                Some("stream_interrupted"),
                None,
                Some("上游模型流式连接在输出结束前意外断开或未返回有效内容"),
            ));
        }
        let norm = self.normalizer.finish(emit);
        self.completion.text = norm.text;
        self.completion.reasoning_text = norm.reasoning_text;
        self.completion.thinking_ms = norm.thinking_ms;
        self.completion.think_block_unclosed = norm.think_block_unclosed;
        self.completion.orphan_think_reclassified = norm.orphan_think_reclassified;
        if norm.reasoning_chars > 0 {
            self.completion.usage.reasoning_text_chars = Some(norm.reasoning_chars);
        }

        for (idx, (_, pending)) in self.tools.into_iter().enumerate() {
            let name = pending.name.trim().to_owned();
            if name.is_empty() {
                return Err(
                    ModelError::new(ErrorKind::Capability, None).with_diagnostic(
                        Some("stream_decode"),
                        None,
                        None,
                        Some("模型返回了缺少名称的工具调用片段"),
                    ),
                );
            }
            let id = normalize_tool_call_id(&pending.id, self.step, idx);
            let arguments = parse_lenient_tool_arguments(&pending.arguments)?;
            let call = ToolCall {
                id,
                name,
                arguments,
            };
            emit(ModelEvent::ToolCall(call.clone()));
            self.completion.tool_calls.push(call);
        }

        if self.completion.tool_calls.is_empty() {
            let (cleaned_text, xml_calls) =
                extract_xml_tool_calls(&self.completion.text, self.step);
            if !xml_calls.is_empty() {
                self.completion.text = cleaned_text;
                for call in xml_calls {
                    emit(ModelEvent::ToolCall(call.clone()));
                    self.completion.tool_calls.push(call);
                }
            }
        }

        Ok(self.completion)
    }
}

/// Normalizes missing or non-conforming `tool_call.id` values (e.g. empty `""`
/// or UUIDs with colons from Ollama/vLLM) into deterministic `call_{step}_{idx}`.
pub(crate) fn normalize_tool_call_id(raw_id: &str, step: usize, idx: usize) -> String {
    let trimmed = raw_id.trim();
    if !trimmed.is_empty()
        && trimmed.len() <= MAX_TOOL_ID_LEN
        && trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        trimmed.to_owned()
    } else {
        format!("call_{step}_{idx}")
    }
}

/// Strips accidental ```` ```json ... ``` ```` wrappers and string-outside
/// trailing commas `,}` / `,]` before parsing tool arguments JSON.
pub(crate) fn parse_lenient_tool_arguments(raw: &str) -> Result<Value, ModelError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Value::String(inner) = &value {
            if let Ok(obj) = serde_json::from_str::<Value>(inner.trim()) {
                if obj.is_object() {
                    return Ok(obj);
                }
            }
        }
        return Ok(value);
    }
    let unfenced = strip_surrounding_code_fence(trimmed);
    if let Ok(value) = serde_json::from_str::<Value>(unfenced) {
        return Ok(value);
    }
    let repaired = strip_json_trailing_commas(unfenced);
    serde_json::from_str::<Value>(&repaired).map_err(|_| {
        ModelError::new(ErrorKind::Parameter, None).with_diagnostic(
            Some("stream_decode"),
            None,
            None,
            Some("模型生成的工具调用参数不是有效的 JSON 对象"),
        )
    })
}

pub(crate) fn strip_surrounding_code_fence(input: &str) -> &str {
    let trimmed = input.trim();
    if let Some(after_open) = trimmed.strip_prefix("```") {
        // Strip optional language identifier on the opening fence line (`json`, `xml`, etc.).
        let body_start = match after_open.find('\n') {
            Some(nl)
                if after_open[..nl]
                    .trim()
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
            {
                &after_open[nl + 1..]
            }
            _ => {
                let tag_len = after_open
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                    .map(char::len_utf8)
                    .sum::<usize>();
                &after_open[tag_len..]
            }
        };
        if let Some(inner) = body_start.trim_end().strip_suffix("```") {
            return inner.trim();
        }
    }
    trimmed
}

/// Removes trailing commas before `}` or `]` outside JSON string literals.
pub(crate) fn strip_json_trailing_commas(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            let ch = input[i..].chars().next().expect("valid utf-8");
            out.push(ch);
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += ch.len_utf8();
            continue;
        }
        if b == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        if b == b',' {
            let mut look = i + 1;
            while look < bytes.len() && bytes[look].is_ascii_whitespace() {
                look += 1;
            }
            if look < bytes.len() && (bytes[look] == b'}' || bytes[look] == b']') {
                i += 1;
                continue;
            }
        }
        let ch = input[i..].chars().next().expect("valid utf-8");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Extracts Qwen / Hermes-style `<tool_call>{"name": ..., "arguments": ...}</tool_call>`
/// blocks when a model emits XML tool calls inside text instead of `delta.tool_calls`.
fn extract_xml_tool_calls(text: &str, step: usize) -> (String, Vec<ToolCall>) {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";

    if !text.contains(OPEN) || !text.contains(CLOSE) {
        return (text.to_owned(), Vec::new());
    }

    let mut calls = Vec::new();
    let mut kept = String::with_capacity(text.len());
    let mut cursor = 0usize;

    while let Some(rel_open) = text[cursor..].find(OPEN) {
        let open_at = cursor + rel_open;
        let inner_start = open_at + OPEN.len();
        let Some(rel_close) = text[inner_start..].find(CLOSE) else {
            break;
        };
        let close_end = inner_start + rel_close + CLOSE.len();
        let raw_inner = &text[inner_start..inner_start + rel_close];

        if let Some(call) = parse_single_xml_tool_call(raw_inner, step, calls.len()) {
            // Also strip a directly surrounding ```xml ... ``` fence if present around only this tag.
            let before = &text[cursor..open_at];
            let before_trimmed = before.trim_end();
            let after = &text[close_end..];
            let after_trimmed = after.trim_start();
            if let (Some(fence_at), true) = (
                before_trimmed.rfind("```").filter(|&at| {
                    before_trimmed[at + 3..]
                        .trim()
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                }),
                after_trimmed.starts_with("```"),
            ) {
                kept.push_str(&before_trimmed[..fence_at]);
                let fence_close_offset = (after.len() - after_trimmed.len()) + 3;
                cursor = close_end + fence_close_offset;
            } else {
                kept.push_str(before);
                cursor = close_end;
            }
            calls.push(call);
        } else {
            kept.push_str(&text[cursor..close_end]);
            cursor = close_end;
        }
    }

    if calls.is_empty() {
        return (text.to_owned(), Vec::new());
    }
    kept.push_str(&text[cursor..]);
    let cleaned = kept.trim().to_owned();
    (cleaned, calls)
}

fn parse_single_xml_tool_call(raw_inner: &str, step: usize, idx: usize) -> Option<ToolCall> {
    let parsed = parse_lenient_tool_arguments(raw_inner).ok()?;
    let obj = parsed.as_object()?;
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| parsed.pointer("/function/name").and_then(Value::as_str))?
        .trim()
        .to_owned();
    if name.is_empty() {
        return None;
    }
    let raw_args = obj
        .get("arguments")
        .or_else(|| obj.get("parameters"))
        .or_else(|| parsed.pointer("/function/arguments"));
    let arguments = match raw_args {
        None | Some(Value::Null) => json!({}),
        Some(Value::String(s)) => parse_lenient_tool_arguments(s).ok()?,
        Some(Value::Object(_)) => raw_args?.clone(),
        Some(_) => return None,
    };
    let raw_id = obj.get("id").and_then(Value::as_str).unwrap_or("");
    let id = normalize_tool_call_id(raw_id, step, idx);
    Some(ToolCall {
        id,
        name,
        arguments,
    })
}

pub fn classify_status(status: u16) -> ErrorKind {
    match status {
        401 | 403 => ErrorKind::Authentication,
        400 | 404 | 413 | 422 => ErrorKind::Parameter,
        405 | 415 | 501 => ErrorKind::Capability,
        408 | 409 | 425 | 429 | 500..=599 => ErrorKind::Transient,
        _ => ErrorKind::Unknown,
    }
}

fn classify_type(error: &str) -> ErrorKind {
    match error.trim().to_ascii_lowercase().as_str() {
        "authentication_error"
        | "permission_error"
        | "permission_denied"
        | "invalid_api_key"
        | "unauthorized"
        | "unauthenticated"
        | "invalid_token"
        | "token_expired"
        | "insufficient_quota"
        | "quota_exceeded"
        | "billing_hard_limit_reached" => ErrorKind::Authentication,
        "invalid_request_error"
        | "invalid_request"
        | "invalid_argument"
        | "bad_request"
        | "context_length_exceeded"
        | "max_tokens_exceeded"
        | "prompt_too_long"
        | "model_not_found"
        | "not_found_error" => ErrorKind::Parameter,
        "rate_limit_error"
        | "rate_limit_exceeded"
        | "too_many_requests"
        | "resource_exhausted"
        | "overloaded_error"
        | "server_error"
        | "service_unavailable"
        | "internal_error"
        | "internal"
        | "unavailable"
        | "timeout"
        | "deadline_exceeded"
        | "request_timeout"
        | "bad_gateway"
        | "gateway_timeout"
        | "upstream_error"
        | "api_error" => ErrorKind::Transient,
        "unsupported_feature"
        | "not_supported"
        | "content_filter"
        | "content_policy_violation"
        | "safety" => ErrorKind::Capability,
        _ => ErrorKind::Unknown,
    }
}
