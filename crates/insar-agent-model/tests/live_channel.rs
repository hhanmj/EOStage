//! Live probe of one real model channel through the production channel route,
//! transport and Agent loop. Ignored by default; each probe prints
//! "SKIPPED: live env not set" and returns unless all three variables are set:
//! `EOSTAGE_LIVE_MODEL_BASE_URL`, `EOSTAGE_LIVE_MODEL_MODEL` and
//! `EOSTAGE_LIVE_MODEL_KEY_FILE` (a path: the key never enters the
//! environment, the command line or the output; every printed line is
//! scrubbed of it). Run with `-- --ignored --nocapture --test-threads=1`.
//!
//! A refused reasoning switch is resent once the way the desktop host does it
//! (`Model::without_reasoning_switch`); this probe has no run record, so it
//! prints both outbound envelopes instead of archiving them.
use std::{
    future::Future,
    pin::Pin,
    sync::Mutex,
    time::{Duration, Instant},
};

use insar_agent_model::{
    agent::{AgentLoop, Checkpoint, CredentialedModel, Dispatcher, Model},
    channels::ChannelConfig,
    transport::{ModelClient, ModelRoute},
    CancellationToken, Completion, ErrorKind, Message, ModelError, ModelEvent, ReasoningMode,
    ToolCall, ToolDescription, Usage, Zeroizing,
};
use serde_json::{json, Value};

struct Live {
    base_url: String,
    model: String,
    key: Zeroizing<String>,
}

impl Live {
    fn from_env(probe: &str) -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let (Some(base_url), Some(model), Some(path)) = (
            var("EOSTAGE_LIVE_MODEL_BASE_URL"),
            var("EOSTAGE_LIVE_MODEL_MODEL"),
            var("EOSTAGE_LIVE_MODEL_KEY_FILE"),
        ) else {
            println!("[{probe}] SKIPPED: live env not set");
            return None;
        };
        let Ok(raw) = std::fs::read_to_string(&path).map(Zeroizing::new) else {
            println!("[{probe}] SKIPPED: live env not set (key file unreadable)");
            return None;
        };
        let key = Zeroizing::new(raw.trim().to_owned());
        if key.is_empty() {
            println!("[{probe}] SKIPPED: live env not set (key file empty)");
            return None;
        }
        Some(Self {
            base_url,
            model,
            key,
        })
    }

    fn route(&self, reasoning_mode: ReasoningMode) -> ModelRoute {
        ChannelConfig {
            id: "live-probe".into(),
            protocol: "chat_completions".into(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            reasoning_mode,
            reasoning_switch_unsupported: false,
            models: vec![],
        }
        .validated_route()
        .unwrap_or_else(|_| panic!("the live channel settings are not a valid route"))
    }

    fn say(&self, line: String) {
        println!("{}", self.scrub(&line));
    }

    fn scrub(&self, text: &str) -> String {
        text.replace(self.key.as_str(), "[redacted]")
    }

    /// Error text carries only kind and status: a `ModelError` checkpoint
    /// holds conversation messages, which stay out of the log.
    fn fail(&self, probe: &str, error: &ModelError) -> ! {
        panic!(
            "{}",
            self.scrub(&format!(
                "[{probe}] model error kind={:?} status={:?}",
                error.kind, error.status
            ))
        )
    }
}

fn client() -> ModelClient {
    ModelClient::new(Duration::from_secs(180)).expect("http client")
}

fn envelope(body: &Value) -> Value {
    let mut envelope = body.clone();
    if let Some(object) = envelope.as_object_mut() {
        object.remove("messages");
        object.remove("tools");
    }
    envelope
}

fn preview(text: &str) -> String {
    text.chars()
        .take(200)
        .collect::<String>()
        .replace('\n', " ")
}

struct Call {
    elapsed_ms: u128,
    usage: Usage,
    tool_calls: usize,
    reasoning_off_refused: bool,
    truncated: bool,
    think_block_unclosed: bool,
}

/// Times every model call, resends once without the reasoning switch when the
/// provider refuses it, and records what came back.
struct Timed<'m> {
    live: &'m Live,
    probe: &'static str,
    inner: CredentialedModel<'m>,
    calls: Mutex<Vec<Call>>,
}

impl Model for Timed<'_> {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [ToolDescription],
        cancel: &'a CancellationToken,
        emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            let start = Instant::now();
            let result = match self
                .inner
                .complete(messages, tools, cancel, &mut *emit)
                .await
            {
                Err(error) if error.kind == ErrorKind::ReasoningSwitchRefused => {
                    let resend = self
                        .inner
                        .without_reasoning_switch()
                        .expect("a refused route sent a switch");
                    if let Some(Ok(body)) = resend.request_body(messages, tools) {
                        assert!(!body.to_string().contains(self.live.key.as_str()));
                        self.live.say(format!(
                            "[{}] refused status={:?}; resend envelope={}",
                            self.probe,
                            error.status,
                            envelope(&body)
                        ));
                    }
                    resend
                        .complete(messages, tools, cancel, &mut *emit)
                        .await
                        .map(|mut completion| {
                            completion.reasoning_off_refused = true;
                            completion
                        })
                }
                other => other,
            };
            if let Ok(completion) = &result {
                self.calls.lock().unwrap().push(Call {
                    elapsed_ms: start.elapsed().as_millis(),
                    usage: completion.usage.clone(),
                    tool_calls: completion.tool_calls.len(),
                    reasoning_off_refused: completion.reasoning_off_refused,
                    truncated: completion.truncated,
                    think_block_unclosed: completion.think_block_unclosed,
                });
            }
            result
        })
    }
    fn request_body(
        &self,
        messages: &[Message],
        tools: &[ToolDescription],
    ) -> Option<Result<Value, ModelError>> {
        self.inner.request_body(messages, tools)
    }
}

impl Timed<'_> {
    fn report(&self) {
        for (index, call) in self.calls.lock().unwrap().iter().enumerate() {
            self.live.say(format!(
                "[{}] call {index}: elapsed_ms={} tool_calls={} reasoning_off_refused={} truncated={} think_block_unclosed={} usage={:?}",
                self.probe,
                call.elapsed_ms,
                call.tool_calls,
                call.reasoning_off_refused,
                call.truncated,
                call.think_block_unclosed,
                call.usage
            ));
        }
    }
}

struct EchoNumber {
    calls: Mutex<Vec<Value>>,
}

impl Dispatcher for EchoNumber {
    fn execute<'a>(
        &'a self,
        call: &'a ToolCall,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(call.arguments.clone());
            Ok(json!({"number": call.arguments["number"]}))
        })
    }
}

async fn plain_chat(
    live: &Live,
    probe: &'static str,
    mode: ReasoningMode,
    prompt: &str,
) -> Completion {
    let route = live.route(mode);
    let client = client();
    let model = Timed {
        live,
        probe,
        inner: CredentialedModel {
            client: &client,
            route: &route,
            key: &live.key,
        },
        calls: Mutex::new(Vec::new()),
    };
    let messages = [
        Message {
            role: "system".into(),
            content: "You are a concise assistant.".into(),
        },
        Message {
            role: "user".into(),
            content: prompt.into(),
        },
    ];
    let (url, body) = route.request(&messages, &[]).expect("request body");
    assert!(
        !body.to_string().contains(live.key.as_str()),
        "the key must never enter a request body"
    );
    live.say(format!("[{probe}] POST {url} envelope={}", envelope(&body)));
    let mut deltas = 0usize;
    let completion = model
        .complete(&messages, &[], &CancellationToken::new(), &mut |event| {
            if matches!(event, ModelEvent::Text(_)) {
                deltas += 1;
            }
        })
        .await
        .unwrap_or_else(|error| live.fail(probe, &error));
    model.report();
    live.say(format!(
        "[{probe}] text_deltas={deltas} text_chars={} text=\"{}\"",
        completion.text.chars().count(),
        preview(&completion.text)
    ));
    completion
}

#[tokio::test]
#[ignore = "live network probe; needs EOSTAGE_LIVE_MODEL_* variables"]
async fn live_a_streamed_chat_with_reasoning_off_answers() {
    let Some(live) = Live::from_env("a") else {
        return;
    };
    let completion = plain_chat(
        &live,
        "a",
        ReasoningMode::Off,
        "Reply with the single word: pong",
    )
    .await;
    assert!(!completion.truncated, "the answer was cut off");
    assert!(
        !completion.text.trim().is_empty(),
        "reasoning off must leave room for an answer"
    );
}

#[tokio::test]
#[ignore = "live network probe; needs EOSTAGE_LIVE_MODEL_* variables"]
async fn live_b_one_tool_round_returns_the_number() {
    let Some(live) = Live::from_env("b") else {
        return;
    };
    let route = live.route(ReasoningMode::Off);
    let client = client();
    let model = Timed {
        live: &live,
        probe: "b",
        inner: CredentialedModel {
            client: &client,
            route: &route,
            key: &live.key,
        },
        calls: Mutex::new(Vec::new()),
    };
    let tools = [ToolDescription {
        name: "echo_number".into(),
        description: "Returns the given number unchanged.".into(),
        parameters: json!({
            "type": "object",
            "properties": {"number": {"type": "integer", "description": "The number to echo."}},
            "required": ["number"]
        }),
    }];
    let messages = vec![
        Message {
            role: "system".into(),
            content: "You are a concise assistant. Use tools when asked.".into(),
        },
        Message {
            role: "user".into(),
            content: "Call the echo_number tool with number 4721, then reply with only the number the tool returned.".into(),
        },
    ];
    let body = model
        .request_body(&messages, &tools)
        .expect("networked model")
        .expect("request body");
    assert!(!body.to_string().contains(live.key.as_str()));
    live.say(format!("[b] envelope={}", envelope(&body)));
    let echo = EchoNumber {
        calls: Mutex::new(Vec::new()),
    };
    let mut emitted = Vec::new();
    let start = Instant::now();
    let result = AgentLoop { max_steps: 4 }
        .run(
            &model,
            &echo,
            &tools,
            Checkpoint::new(messages),
            &CancellationToken::new(),
            |event| {
                if let ModelEvent::ToolCall(call) = event {
                    emitted.push(call);
                }
            },
        )
        .await
        .unwrap_or_else(|error| live.fail("b", &error));
    let elapsed = start.elapsed().as_millis();
    model.report();
    let calls = echo.calls.lock().unwrap().clone();
    live.say(format!(
        "[b] total_elapsed_ms={elapsed} steps={} tool_arguments={} answer=\"{}\"",
        result.checkpoint.steps,
        json!(calls),
        preview(&result.answer.text)
    ));
    assert!(model
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|call| !call.truncated));
    assert_eq!(emitted.len(), calls.len());
    assert_eq!(
        calls.first().map(|arguments| &arguments["number"]),
        Some(&json!(4721)),
        "the model must call echo_number with parsed integer arguments"
    );
    assert!(
        result.answer.text.contains("4721"),
        "the final answer must carry the tool result"
    );
}

#[tokio::test]
#[ignore = "live network probe; needs EOSTAGE_LIVE_MODEL_* variables"]
async fn live_c_reasoning_default_completes() {
    let Some(live) = Live::from_env("c") else {
        return;
    };
    let completion = plain_chat(
        &live,
        "c",
        ReasoningMode::ProviderDefault,
        "What is 17 * 23? Reply with only the number.",
    )
    .await;
    let reasoned = completion.usage.reasoning_tokens.unwrap_or(0) > 0;
    assert!(
        !completion.text.trim().is_empty() || reasoned,
        "default mode must answer or report reasoning tokens"
    );
}
