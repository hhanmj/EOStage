//! Model communication and bounded orchestration. No Store, credential or approval ownership.

use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use tokio_util::sync::CancellationToken;
pub use zeroize::Zeroizing;

mod stream;
pub use stream::{classify_status, StreamDecoder};
pub mod agent;
pub mod channels;
pub mod privacy;
pub mod think_normalizer;
pub use think_normalizer::ThinkNormalizer;
pub mod transport;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    ChatCompletions,
    Responses,
    AnthropicMessages,
    GoogleGemini,
}

/// Whether a reasoning model may deliberate before it answers. `Off` asks the
/// provider to skip its thinking phase where the protocol and model family have
/// a known switch; `ProviderDefault` sends no reasoning field at all. Channels
/// saved before the option existed read as `Off`.
///
/// Request fields per protocol and model family (family = model id after the
/// last `/`, case-insensitive, matched top to bottom):
///
/// | protocol         | family                 | budget            | `Off` adds                       |
/// |------------------|------------------------|-------------------|----------------------------------|
/// | chat_completions | `glm-*`                | `max_tokens` 4096 | `thinking: {type: "disabled"}`   |
/// | chat_completions | `gpt-5-chat*`          | none              | nothing                          |
/// | chat_completions | `gpt-5`, `gpt-5-*`     | none              | `reasoning_effort: "minimal"`    |
/// | chat_completions | `gpt-5.N…`             | none              | nothing                          |
/// | chat_completions | `o1` / `o3` / `o4…`    | none              | `reasoning_effort: "low"`        |
/// | chat_completions | any other              | none              | nothing                          |
/// | responses        | `gpt-5`, `gpt-5-*`     | none              | `reasoning: {effort: "minimal"}` |
/// | responses        | `o1` / `o3` / `o4…`    | none              | `reasoning: {effort: "low"}`     |
/// | responses        | any other              | none              | nothing                          |
/// | anthropic        | any                    | `max_tokens` 4096 | nothing (`thinking` never sent)  |
///
/// A channel known to refuse the switch (`ModelRoute::reasoning_switch_unsupported`,
/// remembered per channel after a refused request was resent successfully,
/// W3o) sends no `Off` field, and chat `glm-*` gets `max_tokens` 8192 instead
/// of 4096 because the model thinks. The budget stays `max_tokens` (not
/// `max_completion_tokens`, OpenAI's name for reasoning models, which get no
/// budget here).
///
/// `ProviderDefault` adds no reasoning field. OpenAI reasoning models refuse
/// `max_tokens` on chat completions; `max_completion_tokens` is not sent
/// because the run record refuses token-named fields other than `max_tokens`.
/// Inline `<think>` / `<thinking>` blocks and dedicated reasoning deltas are
/// normalized across all protocols and model families by [`ThinkNormalizer`].
///
/// When a request that carried an `Off` field is answered with HTTP 400, the
/// transport returns [`ErrorKind::ReasoningSwitchRefused`] instead of sending
/// again; the caller that archives outbound bodies decides whether to resend
/// once without the field ([`agent::Model::without_reasoning_switch`]).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReasoningMode {
    #[serde(rename = "default")]
    ProviderDefault,
    #[default]
    #[serde(rename = "off")]
    Off,
}

impl ReasoningMode {
    pub fn is_off(&self) -> bool {
        *self == Self::Off
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Transient,
    Parameter,
    Authentication,
    Capability,
    Unknown,
    Cancelled,
    /// HTTP 400 to a request that carried a reasoning switch. Nothing was
    /// resent; see [`agent::Model::without_reasoning_switch`].
    ReasoningSwitchRefused,
    /// The loop used all its model calls without a final answer.
    StepLimit,
    /// The model repeated the previous turn's tool calls exactly.
    RepeatedCall,
    /// The model called a tool it was not offered.
    ToolNotOffered,
}

pub struct ModelError {
    pub kind: ErrorKind,
    pub status: Option<u16>,
    pub transport_timeout: bool,
    pub stage: Option<String>,
    pub network_cause: Option<String>,
    pub provider_code: Option<String>,
    pub detail: Option<String>,
    /// Present when a tool outcome may be unknown; W3a persists/reconciles before retrying.
    pub checkpoint: Option<Box<agent::Checkpoint>>,
}

impl std::fmt::Debug for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelError")
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("transport_timeout", &self.transport_timeout)
            .field("stage", &self.stage)
            .field("network_cause", &self.network_cause)
            .field("provider_code", &self.provider_code)
            .field("has_detail", &self.detail.is_some())
            .field("has_checkpoint", &self.checkpoint.is_some())
            .finish()
    }
}

impl ModelError {
    pub fn new(kind: ErrorKind, status: Option<u16>) -> Self {
        Self {
            kind,
            status,
            transport_timeout: false,
            stage: None,
            network_cause: None,
            provider_code: None,
            detail: None,
            checkpoint: None,
        }
    }

    pub fn with_diagnostic(
        mut self,
        stage: Option<&str>,
        network_cause: Option<&str>,
        provider_code: Option<&str>,
        detail: Option<&str>,
    ) -> Self {
        self.stage = stage
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        self.network_cause = network_cause
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        self.provider_code = provider_code
            .map(sanitize_diagnostic_text)
            .filter(|s| !s.is_empty());
        self.detail = detail
            .map(sanitize_diagnostic_text)
            .filter(|s| !s.is_empty());
        self
    }

    pub fn sanitized_summary_zh(&self) -> String {
        let stage_zh = match self.stage.as_deref() {
            Some("request_connect") => Some("发起对话请求"),
            Some("response_status") => Some("接收模型 HTTP 状态"),
            Some("stream_read") => Some("读取流式响应"),
            Some("stream_decode") => Some("解码流式内容"),
            Some("stream_event") => Some("解析上游流式事件"),
            Some("models_connect") => Some("连接模型目录接口"),
            Some("models_list") => Some("读取模型目录状态"),
            Some("models_read") => Some("读取模型目录响应"),
            _ => None,
        };
        let cause_zh = match self.network_cause.as_deref() {
            Some("timeout") => Some("连接或响应超时"),
            Some("dns") => Some("域名解析失败"),
            Some("tls") => Some("TLS/证书握手失败"),
            Some("proxy") => Some("代理连接异常"),
            Some("stream_interrupted") => Some("连接在流式输出结束前意外中断"),
            Some("connect") => Some("网络连接建立失败"),
            Some("transport") => Some("网络传输异常"),
            _ => None,
        };
        let provider_hint_zh = self.provider_code.as_deref().map(provider_code_zh);

        if let Some(status) = self.status {
            let status_zh = http_status_zh(status);
            let mut msg = format!("HTTP {status}（{status_zh}）");
            if let Some(hint) = provider_hint_zh {
                msg.push_str(&format!("，原因：{hint}"));
            }
            if let Some(detail) = self.detail.as_deref() {
                msg.push_str(&format!("：{detail}"));
            }
            return msg;
        }

        if let Some(cause) = cause_zh {
            let mut msg = if let Some(stage) = stage_zh {
                format!("网络通信在「{stage}」阶段受阻：{cause}")
            } else {
                format!("网络通信失败：{cause}")
            };
            if let Some(detail) = self.detail.as_deref() {
                msg.push_str(&format!("（{detail}）"));
            }
            return msg;
        }

        if let Some(hint) = provider_hint_zh {
            let mut msg = if let Some(stage) = stage_zh {
                format!("上游模型在「{stage}」阶段返回错误：{hint}")
            } else {
                format!("上游模型返回错误：{hint}")
            };
            if let Some(detail) = self.detail.as_deref() {
                msg.push_str(&format!("（{detail}）"));
            }
            return msg;
        }

        if let Some(detail) = self.detail.as_deref() {
            return if let Some(stage) = stage_zh {
                format!("在「{stage}」阶段失败：{detail}")
            } else {
                detail.to_owned()
            };
        }

        match self.kind {
            ErrorKind::Authentication => {
                "模型凭据验证失败，请检查密钥、OAuth 授权或 CLI 登录状态".to_owned()
            }
            ErrorKind::Parameter => "请求参数或接口地址不被当前模型渠道接受".to_owned(),
            ErrorKind::Capability => "模型响应超出大小限制或格式不符合预期协议".to_owned(),
            ErrorKind::Transient => "上游模型服务暂时不可用或网络波动中断".to_owned(),
            ErrorKind::Cancelled => "请求已取消".to_owned(),
            ErrorKind::ReasoningSwitchRefused => "当前渠道拒绝思考模式切换参数".to_owned(),
            ErrorKind::StepLimit => "达到单轮最大工具调用步数上限".to_owned(),
            ErrorKind::RepeatedCall => "模型连续重复相同工具调用已中止".to_owned(),
            ErrorKind::ToolNotOffered => "模型尝试调用未授权的工具".to_owned(),
            ErrorKind::Unknown => "模型调用发生未分类异常".to_owned(),
        }
    }
}

fn http_status_zh(status: u16) -> &'static str {
    match status {
        400 => "请求参数不被接受",
        401 => "认证失败或令牌已过期",
        403 => "无权限访问该模型渠道",
        404 => "接口路径或模型不存在",
        408 => "请求超时",
        413 => "请求上下文过大超出限制",
        422 => "请求字段校验未通过",
        429 => "触发限流或配额不足",
        500 => "上游服务端内部错误",
        502 => "上游网关异常",
        503 => "模型服务暂时过载不可用",
        504 => "上游网关响应超时",
        501 | 505..=599 => "上游服务端错误",
        _ => "请求失败",
    }
}

fn provider_code_zh(code: &str) -> String {
    let lower = code.trim().to_ascii_lowercase();
    match lower.as_str() {
        "invalid_api_key"
        | "authentication_error"
        | "unauthorized"
        | "unauthenticated"
        | "permission_error"
        | "permission_denied"
        | "invalid_token"
        | "token_expired" => "API Key 或访问令牌无效/已过期".to_owned(),
        "insufficient_quota"
        | "quota_exceeded"
        | "billing_hard_limit_reached"
        | "resource_exhausted" => "账户配额已耗尽或触发限额".to_owned(),
        "rate_limit_exceeded" | "rate_limit_error" | "too_many_requests" => {
            "请求频率或并发超过渠道限制".to_owned()
        }
        "context_length_exceeded" | "max_tokens_exceeded" | "prompt_too_long" => {
            "输入内容超出该模型上下文长度上限".to_owned()
        }
        "model_not_found" | "not_found_error" => "目标模型不存在或当前账号无权限使用".to_owned(),
        "invalid_request_error" | "invalid_request" | "invalid_argument" | "bad_request" => {
            "上游拒绝了本次请求参数".to_owned()
        }
        "overloaded_error"
        | "server_error"
        | "service_unavailable"
        | "internal_error"
        | "internal"
        | "unavailable"
        | "bad_gateway"
        | "gateway_timeout"
        | "upstream_error"
        | "api_error" => "上游模型服务暂时过载或内部异常".to_owned(),
        "timeout" | "deadline_exceeded" | "request_timeout" => "上游模型响应超时".to_owned(),
        "content_filter" | "content_policy_violation" | "safety" => {
            "上游内容安全策略拦截了本次响应".to_owned()
        }
        "unsupported_feature" | "not_supported" => "当前模型或接口不支持此参数或功能".to_owned(),
        _ => code.trim().replace('_', " "),
    }
}

fn extract_html_error_title(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.starts_with("<!doctype html") || lower.starts_with("<html")) {
        return None;
    }
    let start = lower.find("<title>")? + "<title>".len();
    let end = lower[start..].find("</title>")? + start;
    let title = trimmed[start..end].trim();
    (!title.is_empty()).then(|| title.to_owned())
}

fn redact_diagnostic_secret_tokens(input: &str) -> String {
    const PREFIXES: &[&str] = &[
        "sk-",
        "sk_",
        "tok_",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AIza",
        "ya29.",
        "glpat-",
        "npm_",
        "pypi-",
        "eyJ",
    ];
    let mut out = String::with_capacity(input.len());
    let mut word = String::new();
    let flush = |w: &str, dst: &mut String| {
        if w.is_empty() {
            return;
        }
        let trimmed = w.trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | '(' | ')' | '[' | ']' | ',' | ';' | ':'
            )
        });
        let has_secret_prefix = PREFIXES
            .iter()
            .any(|p| trimmed.strip_prefix(p).is_some_and(|rest| rest.len() >= 8));
        let is_aws = trimmed.starts_with("AKIA") && trimmed.len() >= 20;
        let is_long_hex = trimmed.len() >= 32 && trimmed.bytes().all(|b| b.is_ascii_hexdigit());
        if has_secret_prefix || is_aws || is_long_hex {
            dst.push_str("[redacted]");
        } else {
            dst.push_str(w);
        }
    };
    for ch in input.chars() {
        if ch.is_whitespace() {
            flush(&word, &mut out);
            word.clear();
            out.push(ch);
        } else {
            word.push(ch);
        }
    }
    flush(&word, &mut out);
    out
}

/// Scrubs credentials/tokens/urls, collapses control whitespace, and truncates
/// upstream diagnostic fragments to a safe bounded single-line summary.
pub fn sanitize_diagnostic_text(raw: &str) -> String {
    let extracted_html = extract_html_error_title(raw);
    let effective = extracted_html.as_deref().unwrap_or(raw);
    let stripped_url = strip_url_query_fragments(effective);
    let scrubbed = privacy::scrub_text(&stripped_url);
    let token_scrubbed = redact_diagnostic_secret_tokens(&scrubbed);
    let mut normalized = String::with_capacity(token_scrubbed.len().min(256));
    let mut prev_space = false;
    for ch in token_scrubbed.chars() {
        if ch.is_control() || ch.is_whitespace() {
            if !prev_space && !normalized.is_empty() {
                normalized.push(' ');
                prev_space = true;
            }
        } else {
            normalized.push(ch);
            prev_space = false;
        }
    }
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out: String = trimmed.chars().take(240).collect();
    if trimmed.chars().count() > 240 {
        out.push('…');
    }
    out
}

fn strip_url_query_fragments(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(pos) = rest.find("http://").or_else(|| rest.find("https://")) {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let end = tail
            .find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '"')
            .unwrap_or(tail.len());
        let url_part = &tail[..end];
        let cut = url_part.find('?').unwrap_or(url_part.len());
        out.push_str(&url_part[..cut]);
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDescription {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelEvent {
    ThinkingStart { started_at_ms: u64 },
    ThinkingDelta(String),
    ThinkingEnd { duration_ms: u64 },
    Text(String),
    ToolCall(ToolCall),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Part of `input_tokens` (or prompt cache reads) reported by the provider
    /// as served from prompt cache (`cached_tokens` / `prompt_cache_hit_tokens`
    /// / `cache_read_input_tokens`).
    pub cached_input_tokens: Option<u64>,
    /// Prompt cache creation/write tokens reported by the provider
    /// (`cache_creation_input_tokens`).
    pub cache_creation_input_tokens: Option<u64>,
    /// Part of `output_tokens` the provider reports as spent on reasoning.
    pub reasoning_tokens: Option<u64>,
    /// Characters of thinking the decoder received and kept out of the answer
    /// (dedicated reasoning deltas and inline `<think>` / `<thinking>` blocks).
    /// A size, not a token count: set even when the provider reports no reasoning tokens.
    pub reasoning_text_chars: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    /// Normalized reasoning text extracted from dedicated SSE reasoning fields
    /// and/or inline `<think>` / `<thinking>` blocks.
    pub reasoning_text: String,
    /// Monotonic elapsed thinking time in milliseconds from the first reasoning
    /// delta until the first answer/tool delta or stream completion (`None` when
    /// zero reasoning characters were emitted).
    pub thinking_ms: Option<u64>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    /// Reasoning was off and the provider refused to turn it off: the answer
    /// came from the same request resent without the reasoning switch. A
    /// reply from a channel already known to refuse it sent no switch and is
    /// not marked (L4 r10 P2-8). Set by the caller that resent it, never by
    /// the transport.
    pub reasoning_off_refused: bool,
    /// The provider stopped at its output limit (chat `finish_reason`
    /// "length", Anthropic `stop_reason` "max_tokens", Responses incomplete
    /// for "max_output_tokens"): the text and tool calls may be cut off.
    pub truncated: bool,
    /// A reply opened an inline `<think>` / `<thinking>` block that never closed.
    /// Its body is quarantined in `reasoning_text` and never leaked into `text`.
    pub think_block_unclosed: bool,
    /// An orphan closing `</think>` / `</thinking>` appeared within the first
    /// 8192 bytes before any code fence, reclassifying the streamed prefix into
    /// `reasoning_text` so the host can rewrite any live preview cleanly.
    pub orphan_think_reclassified: bool,
    /// `ModelClient::send` hit HTTP 400/422 complaining about `prompt_cache_key`
    /// or `cache_control` and retried without the cache key; `KeyedModel`
    /// remembers this for the remainder of the run so later steps skip the retry.
    pub prompt_cache_key_refused: bool,
}

/// Offline transport replay; use the same parser for live stream chunks.
pub fn replay(
    protocol: Protocol,
    frames: &str,
    cancel: &CancellationToken,
    mut emit: impl FnMut(ModelEvent),
) -> Result<Completion, ModelError> {
    let mut decoder = StreamDecoder::new(protocol);
    decoder.feed(frames, cancel, &mut emit)?;
    decoder.finish(cancel, &mut emit)
}

/// Drive one model loop to completion on the calling worker thread. Hosts call
/// this from a background thread, never from a window/UI thread.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime")
        .block_on(future)
}
