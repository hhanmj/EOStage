//! Refused reasoning switch against a loopback provider: one relay model may
//! land on an upstream that cannot turn reasoning off (w3h-08-live-diagnose:
//! 400 "This model cannot disable reasoning (reasoning_effort=none).", and a
//! relay may word it in any language). The transport never resends; the
//! caller that archives outbound bodies resends once through
//! `Model::without_reasoning_switch`.
use std::{
    io::{ErrorKind as IoErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use insar_agent_model::{
    agent::{CredentialedModel, Model},
    transport::{ModelClient, ModelRoute},
    ErrorKind, Message, ModelEvent, Protocol, ReasoningMode,
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

const REFUSAL: &str = r#"{"error":{"code":400,"message":"This model cannot disable reasoning (reasoning_effort=none).","param":"","type":"upstream_error"}}"#;

fn reply(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn read_body(socket: &mut TcpStream) -> String {
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let len = socket.read(&mut chunk).unwrap();
        request.extend_from_slice(&chunk[..len]);
        if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&request[..head_end]).to_ascii_lowercase();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while request.len() < head_end + length {
        let len = socket.read(&mut chunk).unwrap();
        request.extend_from_slice(&chunk[..len]);
    }
    String::from_utf8_lossy(&request[head_end..head_end + length]).into_owned()
}

/// Serves the scripted replies in order and records every request body until
/// `stop` is set; extra requests get 500.
fn serve(replies: Vec<String>) -> (String, Arc<AtomicBool>, JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        let mut bodies = Vec::new();
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket.set_nonblocking(false).unwrap();
                    bodies.push(serde_json::from_str(&read_body(&mut socket)).unwrap());
                    let answer = replies.get(bodies.len() - 1).cloned().unwrap_or_else(|| {
                        reply("500 Internal Server Error", "text/plain", "unexpected")
                    });
                    socket.write_all(answer.as_bytes()).unwrap();
                }
                Err(error) if error.kind() == IoErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("{error}"),
            }
        }
        bodies
    });
    (base, stop, handle)
}

fn sse(content: &str) -> String {
    let frame = serde_json::json!({"choices": [{"delta": {"content": content}}]});
    reply(
        "200 OK",
        "text/event-stream",
        &format!("data: {frame}\n\ndata: [DONE]\n\n"),
    )
}

fn user() -> Vec<Message> {
    vec![Message {
        role: "user".into(),
        content: "Reply with the single word: pong".into(),
    }]
}

fn finish(stop: Arc<AtomicBool>, server: JoinHandle<Vec<Value>>) -> Vec<Value> {
    thread::sleep(Duration::from_millis(200));
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap()
}

#[tokio::test]
async fn any_400_to_a_body_with_a_reasoning_switch_is_refused_without_resending() {
    for body in [
        REFUSAL,
        r#"{"error":{"message":"该模型不支持关闭思考"}}"#,
        "",
    ] {
        let (base, stop, server) = serve(vec![reply("400 Bad Request", "application/json", body)]);
        let route = ModelRoute::new(Protocol::ChatCompletions, &base, "glm-5.3-flash").unwrap();
        let error = ModelClient::new(Duration::from_secs(5))
            .unwrap()
            .send(
                &route,
                "fixture-only",
                &user(),
                &[],
                &CancellationToken::new(),
                |_| {},
            )
            .await
            .unwrap_err();
        let bodies = finish(stop, server);
        assert_eq!(error.kind, ErrorKind::ReasoningSwitchRefused, "{body}");
        assert_eq!(error.status, Some(400));
        assert_eq!(bodies.len(), 1, "the transport never resends");
        assert_eq!(
            bodies[0]["thinking"],
            serde_json::json!({"type": "disabled"})
        );
    }
}

#[tokio::test]
async fn the_resend_model_sends_the_same_body_minus_the_switch_exactly_once() {
    let (base, stop, server) = serve(vec![
        reply("400 Bad Request", "application/json", REFUSAL),
        sse("<think>The user wants pong</think>pong"),
    ]);
    let client = ModelClient::new(Duration::from_secs(5)).unwrap();
    let route = ModelRoute::new(Protocol::ChatCompletions, &base, "glm-5.3-flash").unwrap();
    let model = CredentialedModel {
        client: &client,
        route: &route,
        key: "fixture-only",
    };
    let cancel = CancellationToken::new();
    let first = model
        .complete(&user(), &[], &cancel, &mut |_| {})
        .await
        .unwrap_err();
    assert_eq!(first.kind, ErrorKind::ReasoningSwitchRefused);
    let resend = model
        .without_reasoning_switch()
        .expect("the route sent a switch");
    assert!(
        resend.without_reasoning_switch().is_none(),
        "one resend only"
    );
    let mut archived_first = model.request_body(&user(), &[]).unwrap().unwrap();
    let archived_resend = resend.request_body(&user(), &[]).unwrap().unwrap();
    let mut events = Vec::new();
    let completion = resend
        .complete(&user(), &[], &cancel, &mut |event| events.push(event))
        .await
        .unwrap();
    let bodies = finish(stop, server);
    assert_eq!(completion.text, "pong");
    assert_eq!(completion.reasoning_text, "The user wants pong");
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ModelEvent::Text(_)))
            .cloned()
            .collect::<Vec<_>>(),
        vec![ModelEvent::Text("pong".into())]
    );
    assert!(
        !completion.reasoning_off_refused,
        "the caller that resent marks it, not the transport"
    );
    assert_eq!(bodies.len(), 2);
    // What each request_body reports is exactly what went out.
    assert_eq!(bodies[0], archived_first);
    assert_eq!(bodies[1], archived_resend);
    archived_first.as_object_mut().unwrap().remove("thinking");
    // W3o: the resend lets the model think, so it has the larger budget.
    assert_eq!(archived_resend["max_tokens"], 8192);
    archived_first["max_tokens"] = serde_json::json!(8192);
    assert_eq!(
        archived_first, archived_resend,
        "only the switch and the budget differ"
    );
}

#[tokio::test]
async fn without_a_reasoning_switch_a_400_stays_a_parameter_error() {
    for (model, mode) in [
        ("glm-5.3-flash", ReasoningMode::ProviderDefault),
        ("gpt-4o", ReasoningMode::Off),
    ] {
        let (base, stop, server) =
            serve(vec![reply("400 Bad Request", "application/json", REFUSAL)]);
        let client = ModelClient::new(Duration::from_secs(5)).unwrap();
        let route = ModelRoute::new(Protocol::ChatCompletions, &base, model)
            .unwrap()
            .with_reasoning(mode);
        let credentialed = CredentialedModel {
            client: &client,
            route: &route,
            key: "fixture-only",
        };
        assert!(credentialed.without_reasoning_switch().is_none(), "{model}");
        let error = credentialed
            .complete(&user(), &[], &CancellationToken::new(), &mut |_| {})
            .await
            .unwrap_err();
        assert_eq!(finish(stop, server).len(), 1, "{model}");
        assert_eq!(error.kind, ErrorKind::Parameter, "{model}");
        assert_eq!(error.status, Some(400), "{model}");
    }
}

#[tokio::test]
async fn a_resend_that_fails_reports_its_own_status() {
    let (base, stop, server) = serve(vec![
        reply("400 Bad Request", "application/json", REFUSAL),
        reply("401 Unauthorized", "application/json", r#"{"error":{}}"#),
    ]);
    let client = ModelClient::new(Duration::from_secs(5)).unwrap();
    let route = ModelRoute::new(Protocol::ChatCompletions, &base, "glm-5.3-flash").unwrap();
    let model = CredentialedModel {
        client: &client,
        route: &route,
        key: "fixture-only",
    };
    let cancel = CancellationToken::new();
    model
        .complete(&user(), &[], &cancel, &mut |_| {})
        .await
        .unwrap_err();
    let error = model
        .without_reasoning_switch()
        .unwrap()
        .complete(&user(), &[], &cancel, &mut |_| {})
        .await
        .unwrap_err();
    assert_eq!(finish(stop, server).len(), 2);
    assert_eq!(error.kind, ErrorKind::Authentication);
    assert_eq!(error.status, Some(401));
}

#[tokio::test]
async fn chat_completions_strips_rejected_prompt_cache_key_and_keyed_model_remembers_in_run() {
    use insar_agent_model::{agent::KeyedModel, Zeroizing};

    let (base, stop, server) = serve(vec![
        reply(
            "400 Bad Request",
            "application/json",
            r#"{"error":{"message":"Unrecognized request argument supplied: prompt_cache_key"}}"#,
        ),
        sse("first ok"),
        sse("second ok"),
    ]);
    let client = ModelClient::new(Duration::from_secs(5)).unwrap();
    let route = ModelRoute::new(Protocol::ChatCompletions, &base, "gpt-4o")
        .unwrap()
        .with_prompt_cache_key(Some("eostage-abc123".into()));
    let model = KeyedModel::new(client, route, Zeroizing::new("fixture-only".into()));
    let cancel = CancellationToken::new();

    let first = model
        .complete(&user(), &[], &cancel, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(first.text, "first ok");
    assert!(first.prompt_cache_key_refused);

    // Second step in same run skips prompt_cache_key immediately (single HTTP call)
    let second = model
        .complete(&user(), &[], &cancel, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(second.text, "second ok");

    let bodies = finish(stop, server);
    assert_eq!(bodies.len(), 3);
    assert_eq!(bodies[0]["prompt_cache_key"], "eostage-abc123");
    assert!(bodies[1].get("prompt_cache_key").is_none());
    assert!(bodies[2].get("prompt_cache_key").is_none());
}
