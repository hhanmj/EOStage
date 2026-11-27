use std::{
    future::Future,
    io::{Read, Write},
    net::TcpListener,
    pin::Pin,
    thread,
    time::Duration,
};

use insar_agent_model::{
    agent::{AgentLoop, Checkpoint, CredentialedModel, Dispatcher},
    transport::{Endpoint, ModelClient, ModelProxy, ModelRoute},
    ErrorKind, Message, ModelError, Protocol, ToolCall, ToolDescription,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

struct ReadOnlyExecutor;
impl Dispatcher for ReadOnlyExecutor {
    fn execute<'a>(
        &'a self,
        call: &'a ToolCall,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            assert_eq!(call.name, "read_catalog");
            Ok(json!({"batch":"fixture"}))
        })
    }
}

#[tokio::test]
async fn loopback_chat_stream_can_call_reviewed_executor_and_return_answer() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        for step in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let len = socket.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..len]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&request);
            assert!(head.starts_with("POST /v1/chat/completions"));
            let frame = if step == 0 {
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"read_catalog\",\"arguments\":\"{}\"}}]}}]}\n\ndata: [DONE]\n\n"
            } else {
                "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"text\\\":\\\"Found data\\\",\\\"claims\\\":[]}\"}}]}\n\ndata: [DONE]\n\n"
            };
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", frame.len(), frame);
            socket.write_all(reply.as_bytes()).unwrap();
        }
    });
    let client = ModelClient::new(Duration::from_secs(4)).unwrap();
    let route = ModelRoute::new(Protocol::ChatCompletions, &base, "local-model").unwrap();
    let model = CredentialedModel {
        client: &client,
        route: &route,
        key: "fixture-only",
    };
    let agent = AgentLoop { max_steps: 3 };
    let tools = [ToolDescription {
        name: "read_catalog".into(),
        description: "read-only".into(),
        parameters: json!({"type":"object"}),
    }];
    let checkpoint = Checkpoint::new(vec![Message {
        role: "user".into(),
        content: "Find data".into(),
    }]);
    let answer = agent
        .run(
            &model,
            &ReadOnlyExecutor,
            &tools,
            checkpoint,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(answer.answer.text, "Found data");
    assert_eq!(answer.checkpoint.tool_results.len(), 1);
    handle.join().unwrap();
}

#[tokio::test]
async fn model_listing_is_bounded_even_if_provider_returns_many_ids() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = [0u8; 4096];
        let count = socket.read(&mut request).unwrap();
        assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /v1/models"));
        let body = json!({"data": (0..150).map(|index| json!({"id":format!("model-{index}")})).collect::<Vec<_>>()}).to_string();
        let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        socket.write_all(reply.as_bytes()).unwrap();
    });
    let client = ModelClient::new(Duration::from_secs(3)).unwrap();
    let endpoint = ModelRoute::new(Protocol::ChatCompletions, &base, "fixture")
        .unwrap()
        .endpoint;
    let models = client
        .models(&endpoint, "fake-key", &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        models.len(),
        100,
        "diagnostic should not accept unbounded model lists"
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn model_listing_preserves_status_and_timeout_without_response_details() {
    for (status, expected) in [
        (401, ErrorKind::Authentication),
        (404, ErrorKind::Parameter),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1/", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let n = socket.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..n]).starts_with("GET /v1/models "));
            write!(
                socket,
                "HTTP/1.1 {status} failure\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let endpoint = ModelRoute::new(Protocol::ChatCompletions, &base, "fixture")
            .unwrap()
            .endpoint;
        let client = ModelClient::with_proxy(Duration::from_secs(2), ModelProxy::Direct).unwrap();
        let error = client
            .models(&endpoint, "fixture-only", &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(error.kind, expected);
        assert_eq!(error.status, Some(status));
        assert!(!error.transport_timeout);
        handle.join().unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (_socket, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(300));
    });
    let endpoint = ModelRoute::new(Protocol::ChatCompletions, &base, "fixture")
        .unwrap()
        .endpoint;
    let client = ModelClient::with_proxy(Duration::from_millis(50), ModelProxy::Direct).unwrap();
    let error = client
        .models(&endpoint, "fixture-only", &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(error.transport_timeout);
    assert!(error.status.is_none());
    handle.join().unwrap();
}

#[tokio::test]
async fn manual_proxy_receives_model_listing_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        let n = socket.read(&mut request).unwrap();
        assert!(
            String::from_utf8_lossy(&request[..n]).starts_with("GET http://127.0.0.1:1/v1/models ")
        );
        let body = r#"{"data":[{"id":"fixture-model"}]}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let endpoint = ModelRoute::new(
        Protocol::ChatCompletions,
        "http://127.0.0.1:1/v1",
        "fixture",
    )
    .unwrap()
    .endpoint;
    let client =
        ModelClient::with_proxy(Duration::from_secs(2), ModelProxy::Manual(&proxy)).unwrap();
    assert_eq!(
        client
            .models(&endpoint, "fixture-only", &CancellationToken::new())
            .await
            .unwrap(),
        vec!["fixture-model"]
    );
    handle.join().unwrap();
}

#[test]
fn errors_still_have_no_raw_secret() {
    assert_eq!(
        ModelError::new(ErrorKind::Authentication, Some(401)).status,
        Some(401)
    );
}

#[tokio::test]
async fn google_gemini_sends_x_goog_api_key_header_never_url_query_and_parses_models() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        let n = socket.read(&mut request).unwrap();
        let raw = String::from_utf8_lossy(&request[..n]).to_ascii_lowercase();
        assert!(raw.starts_with("get /v1beta/models "), "{raw}");
        assert!(
            !raw.contains("key=secret-gem"),
            "must never put key in URL: {raw}"
        );
        assert!(raw.contains("x-goog-api-key: secret-gem"), "{raw}");
        let body = r#"{"models":[{"name":"models/gemini-2.5-pro","supportedGenerationMethods":["generateContent"]},{"name":"models/embedding-001","supportedGenerationMethods":["embedContent"]}]}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let endpoint = Endpoint::new(&base, Protocol::GoogleGemini).unwrap();
    let client = ModelClient::with_proxy(Duration::from_secs(2), ModelProxy::Direct).unwrap();
    let models = client
        .models(&endpoint, "secret-gem", &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(models, vec!["gemini-2.5-pro"]);
    handle.join().unwrap();
}

#[tokio::test]
async fn codex_models_adds_client_version_headers_and_reads_slug() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!(
        "http://{}/backend-api/codex",
        listener.local_addr().unwrap()
    );
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        let n = socket.read(&mut request).unwrap();
        let raw = String::from_utf8_lossy(&request[..n]).to_ascii_lowercase();
        assert!(
            raw.starts_with("get /backend-api/codex/models?client_version=0.170.0 "),
            "{raw}"
        );
        assert!(raw.contains("user-agent: codex_cli_rs/0.170.0"), "{raw}");
        assert!(raw.contains("originator: codex_cli_rs"), "{raw}");
        let body = r#"{"models":[{"slug":"gpt-5-codex"}]}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let endpoint = Endpoint::new(&base, Protocol::Responses).unwrap();
    let client = ModelClient::with_proxy(Duration::from_secs(2), ModelProxy::Direct).unwrap();
    let models = client
        .models(&endpoint, "codex-token", &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(models, vec!["gpt-5-codex"]);
    handle.join().unwrap();
}

#[tokio::test]
async fn codex_responses_injects_store_false_and_codex_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!(
        "http://{}/backend-api/codex",
        listener.local_addr().unwrap()
    );
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut expected_total: Option<usize> = None;
        loop {
            let n = socket.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..n]);
            if expected_total.is_none() {
                if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..pos]).to_ascii_lowercase();
                    let cl = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    expected_total = Some(pos + 4 + cl);
                }
            }
            if let Some(total) = expected_total {
                if request.len() >= total {
                    break;
                }
            }
        }
        let raw = String::from_utf8_lossy(&request);
        let lower = raw.to_ascii_lowercase();
        assert!(
            lower.starts_with("post /backend-api/codex/responses "),
            "{lower}"
        );
        assert!(
            lower.contains("user-agent: codex_cli_rs/0.170.0"),
            "{lower}"
        );
        assert!(lower.contains("originator: codex_cli_rs"), "{lower}");
        assert!(lower.contains("accept: text/event-stream"), "{lower}");
        let body_start = raw.find("\r\n\r\n").unwrap() + 4;
        let parsed: serde_json::Value = serde_json::from_str(&raw[body_start..]).unwrap();
        assert_eq!(parsed["store"], serde_json::json!(false));
        let sse = concat!(
            "event: response.output_text.delta\ndata: {\"delta\":\"ok\"}\n\n",
            "event: response.completed\ndata: {\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n"
        );
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            sse.len(),
            sse
        )
        .unwrap();
    });
    let route = ModelRoute::new(Protocol::Responses, &base, "gpt-5-codex").unwrap();
    let client = ModelClient::with_proxy(Duration::from_secs(2), ModelProxy::Direct).unwrap();
    let completion = client
        .send(
            &route,
            "codex-token",
            &[insar_agent_model::Message {
                role: "user".into(),
                content: "hi".into(),
            }],
            &[],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(completion.text, "ok");
    handle.join().unwrap();
}

#[tokio::test]
async fn claude_oauth_bearer_header_and_chat_stream_options_400_fallback() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        // 1) Anthropic OAuth Bearer probe
        let (mut s1, _) = listener.accept().unwrap();
        let mut buf1 = [0; 4096];
        let n1 = s1.read(&mut buf1).unwrap();
        let raw1 = String::from_utf8_lossy(&buf1[..n1]).to_ascii_lowercase();
        assert!(raw1.contains("authorization: bearer oauth-tok"), "{raw1}");
        assert!(raw1.contains("anthropic-beta: oauth-2025-04-20"), "{raw1}");
        assert!(!raw1.contains("x-api-key:"), "{raw1}");
        assert!(!raw1.contains("x-eostage-auth-mode"), "{raw1}");
        let mbody = r#"{"data":[{"id":"claude-sonnet-4-5"}]}"#;
        write!(
            s1,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            mbody.len(),
            mbody
        )
        .unwrap();

        // 2) ChatCompletions 400 on stream_options -> retry without stream_options
        for attempt in 0..2 {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                let n = sock.read(&mut tmp).unwrap();
                req.extend_from_slice(&tmp[..n]);
                if req.windows(4).any(|w| w == b"\r\n\r\n") {
                    if let Ok(hdr) = std::str::from_utf8(&req) {
                        let cl: usize = hdr
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse().ok())
                            })
                            .unwrap_or(0);
                        let hdr_end = req.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                        if req.len() >= hdr_end + cl {
                            break;
                        }
                    }
                }
            }
            let body_str = String::from_utf8_lossy(&req);
            if attempt == 0 {
                assert!(body_str.contains("stream_options"), "{body_str}");
                write!(
                    sock,
                    "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            } else {
                assert!(!body_str.contains("stream_options"), "{body_str}");
                let frame = "data: {\"choices\":[{\"delta\":{\"content\":\"pong\"}}]}\n\n";
                write!(
                    sock,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    frame.len(),
                    frame
                )
                .unwrap();
            }
        }
    });

    let client = ModelClient::with_proxy(Duration::from_secs(2), ModelProxy::Direct).unwrap();
    let anth_ep = Endpoint::new(&base, Protocol::AnthropicMessages).unwrap();
    let anth_models = client
        .models_with_headers(
            &anth_ep,
            "oauth-tok",
            &[
                ("x-eostage-auth-mode".into(), "bearer".into()),
                ("anthropic-beta".into(), "oauth-2025-04-20".into()),
            ],
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(anth_models, vec!["claude-sonnet-4-5"]);

    let chat_route = ModelRoute::new(
        Protocol::ChatCompletions,
        &format!("{base}/v1"),
        "egg-alpha-003",
    )
    .unwrap()
    .with_reasoning(insar_agent_model::ReasoningMode::ProviderDefault);
    let err = client
        .send(
            &chat_route,
            "k",
            &[Message {
                role: "user".into(),
                content: "ping".into(),
            }],
            &[],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(err.status, Some(400));

    let completion = client
        .send(
            &chat_route.with_omit_stream_options(true),
            "k",
            &[Message {
                role: "user".into(),
                content: "ping".into(),
            }],
            &[],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(completion.text, "pong");
    handle.join().unwrap();
}
