use insar_agent_mcp::{ApprovedTools, McpClient, RemoteTool};
use insar_agent_model::{ErrorKind, ModelError};
use serde_json::{json, Value};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct FakeServer {
    calls: Arc<AtomicUsize>,
    reply: Value,
}
impl McpClient for FakeServer {
    fn list_tools<'a>(
        &'a self,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RemoteTool>, ModelError>> + Send + 'a>> {
        Box::pin(async {
            Ok(vec![
                RemoteTool {
                    name: "search".into(),
                    description: "fixture".into(),
                    input_schema: json!({"type":"object"}),
                },
                RemoteTool {
                    name: "exec_command".into(),
                    description: "blocked".into(),
                    input_schema: json!({}),
                },
            ])
        })
    }
    fn call_tool<'a>(
        &'a self,
        _name: &'a str,
        _arguments: &'a Value,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let result = self.reply.clone();
        Box::pin(async move { Ok(result) })
    }
    fn shutdown<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), ModelError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn unregistered_external_name_is_invisible_and_never_dispatched() {
    let calls = Arc::new(AtomicUsize::new(0));
    let server = ApprovedTools::new(
        FakeServer {
            calls: calls.clone(),
            reply: json!("ok"),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let tools = server.list_tools(&CancellationToken::new()).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "search");
    let error = server
        .call_tool(
            "exec_command",
            &json!({"authorization":"Bearer fixture-only-gate-secret"}),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Capability);
    assert!(!format!("{error:?}").contains("fixture-only-gate-secret"));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn tool_result_is_bounded_and_redacted() {
    let result = json!({"text": "C:\\Users\\private\\key.txt https://example.invalid/file?token=fake-secret", "big": "x".repeat(100_000), "api_key": "fake-secret"});
    let server = ApprovedTools::new(
        FakeServer {
            calls: Arc::new(AtomicUsize::new(0)),
            reply: result,
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let output = server
        .call_tool("search", &json!({}), &CancellationToken::new())
        .await
        .unwrap()
        .to_string();
    assert!(output.len() <= 16_384);
    assert!(!output.contains("fake-secret"));
    assert!(!output.contains("private"));
    assert!(!output.contains("?token"));
}

#[tokio::test]
async fn recognized_unsafe_tool_results_are_rejected_without_echoing_them() {
    let results = [
        "tok_FAKE9876543210abc",
        "sk-proj-FAKEabc123456789",
        "Bearer  fakeSecret123",
        "machine example.invalid login fake-user password fake-pass",
        "/var/private/file.txt",
        "file:///home/fake/file.txt",
        "C:/Users/fake/file.txt",
        "https://example.invalid/search?token=fake-secret",
    ];
    for (index, text) in results.iter().enumerate() {
        let server = ApprovedTools::new(
            FakeServer {
                calls: Arc::new(AtomicUsize::new(0)),
                reply: json!({"note": text}),
            },
            vec!["search".into()],
            Duration::from_secs(1),
        );
        match server
            .call_tool("search", &json!({}), &CancellationToken::new())
            .await
        {
            Err(error) => {
                assert_eq!(error.kind, ErrorKind::Capability, "case {index}");
                assert!(!format!("{error:?}").contains(text), "case {index}");
            }
            Ok(output) => {
                assert!(!output.to_string().contains(text), "case {index}");
                if *text == "Bearer  fakeSecret123" {
                    assert!(!output.to_string().contains("fakeSecret123"));
                }
            }
        }
    }
}

#[tokio::test]
async fn unicode_tool_result_stays_within_byte_bound_after_redaction() {
    for text in [
        "界".repeat(20_000),
        "🙂".repeat(20_000),
        "ascii-界🙂".repeat(4_000),
    ] {
        let result = json!({"text": text});
        let server = ApprovedTools::new(
            FakeServer {
                calls: Arc::new(AtomicUsize::new(0)),
                reply: result,
            },
            vec!["search".into()],
            Duration::from_secs(1),
        );
        let output = server
            .call_tool("search", &json!({}), &CancellationToken::new())
            .await
            .unwrap();
        assert!(output.to_string().len() <= 16_384);
        assert!(output
            .as_str()
            .is_some_and(|text| text.ends_with("[truncated]")));
    }
}

struct RecordingServer {
    calls: Arc<AtomicUsize>,
    arguments: Arc<tokio::sync::Mutex<Option<Value>>>,
}

impl McpClient for RecordingServer {
    fn list_tools<'a>(
        &'a self,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RemoteTool>, ModelError>> + Send + 'a>> {
        Box::pin(async {
            Ok(vec![RemoteTool {
                name: "search".into(),
                description: "Search public scientific catalogues".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "max_tokens": {"type": "integer", "default": 4096},
                        "query": {"type": "string"}
                    }
                }),
            }])
        })
    }

    fn call_tool<'a>(
        &'a self,
        _name: &'a str,
        arguments: &'a Value,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let arguments = arguments.clone();
        let captured = self.arguments.clone();
        Box::pin(async move {
            *captured.lock().await = Some(arguments);
            Ok(json!({"ok": true}))
        })
    }

    fn shutdown<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), ModelError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn mcp_safe_arguments_are_dispatched_unchanged() {
    let calls = Arc::new(AtomicUsize::new(0));
    let arguments = Arc::new(tokio::sync::Mutex::new(None));
    let server = ApprovedTools::new(
        RecordingServer {
            calls: calls.clone(),
            arguments: arguments.clone(),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let input = json!({
        "max_tokens": 4096,
        "question": "Sentinel-1 ascending scenes over Beijing",
        "nested": {"bounds": [115.7, 39.4, 117.4, 41.1], "ascending": true},
        "reference": "https://example.invalid/catalog/item"
    });
    let output = server
        .call_tool("search", &input, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(output, json!({"ok": true}));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(*arguments.lock().await == Some(input));
}

#[tokio::test]
async fn mcp_long_ordinary_search_text_is_never_silently_truncated() {
    let calls = Arc::new(AtomicUsize::new(0));
    let arguments = Arc::new(tokio::sync::Mutex::new(None));
    let server = ApprovedTools::new(
        RecordingServer {
            calls: calls.clone(),
            arguments: arguments.clone(),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let input = json!({"query": "science ".repeat(2_500)});
    match server
        .call_tool("search", &input, &CancellationToken::new())
        .await
    {
        Ok(_) => {
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert!(*arguments.lock().await == Some(input));
        }
        Err(error) => {
            assert_eq!(error.kind, ErrorKind::Parameter);
            assert_eq!(calls.load(Ordering::Relaxed), 0);
            assert!(arguments.lock().await.is_none());
        }
    }
}

#[tokio::test]
async fn mcp_unsafe_arguments_are_rejected_without_dispatch() {
    let calls = Arc::new(AtomicUsize::new(0));
    let arguments = Arc::new(tokio::sync::Mutex::new(None));
    let server = ApprovedTools::new(
        RecordingServer {
            calls: calls.clone(),
            arguments: arguments.clone(),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let input = json!({
        "path": "C:\\Users\\private\\aoi.geojson",
        "authorization": "Bearer fixture-only-token",
        "nested": [{"api_key": "fixture-only-key"}],
        "question": "保留这个问题"
    });
    let result = server
        .call_tool("search", &input, &CancellationToken::new())
        .await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("unsafe arguments must not be dispatched, even if redacted"),
    };
    assert_eq!(error.kind, ErrorKind::Parameter);
    let diagnostic = format!("{error:?}");
    assert!(!diagnostic.contains("fixture-only"));
    assert!(!diagnostic.contains("private"));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(arguments.lock().await.is_none());
}

#[tokio::test]
async fn mcp_unsafe_individual_values_are_rejected_without_dispatch() {
    let inputs = vec![
        (
            json!({"query": "tok_FAKE9876543210abc"}),
            "tok_FAKE9876543210abc",
        ),
        (
            json!({"query": "sk-proj-FAKEabc123456789"}),
            "sk-proj-FAKEabc123456789",
        ),
        (
            json!({"query": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmaXh0dXJlIn0.signature"}),
            "eyJhbGciOiJIUzI1NiJ9",
        ),
        (json!({"query": "Bearer  fakeSecret123"}), "fakeSecret123"),
        (
            json!({"query": "machine example.invalid login fake-user password fake-pass"}),
            "fake-pass",
        ),
        (json!({"query": "/var/private/file.txt"}), "private"),
        (
            json!({"query": "file:///home/fake/file.txt"}),
            "file:///home/fake",
        ),
        (json!({"query": "C:/Users/fake/file.txt"}), "C:/Users/fake"),
        (json!({"query": "\\\\server\\private\\file.txt"}), "private"),
        (
            json!({"query": "https://example.invalid/search?q=public"}),
            "q=public",
        ),
        (
            json!({"query": "https://example.invalid/search?token=fake-secret"}),
            "fake-secret",
        ),
        (json!({"query": "x".repeat(100_000)}), "xxxxxxxxxx"),
        (
            json!({"nested": [{"query": "tok_FAKE9876543210abc"}]}),
            "tok_FAKE9876543210abc",
        ),
    ];
    for (index, (input, sensitive)) in inputs.iter().enumerate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let arguments = Arc::new(tokio::sync::Mutex::new(None));
        let server = ApprovedTools::new(
            RecordingServer {
                calls: calls.clone(),
                arguments: arguments.clone(),
            },
            vec!["search".into()],
            Duration::from_secs(1),
        );
        let result = server
            .call_tool("search", input, &CancellationToken::new())
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("case {index}: unsafe argument was dispatched"),
        };
        assert_eq!(error.kind, ErrorKind::Parameter, "case {index}");
        assert!(!format!("{error:?}").contains(sensitive), "case {index}");
        assert_eq!(calls.load(Ordering::Relaxed), 0, "case {index}");
        assert!(arguments.lock().await.is_none(), "case {index}");
    }
}

#[tokio::test]
async fn mcp_safe_schema_is_returned_unchanged() {
    let server = ApprovedTools::new(
        RecordingServer {
            calls: Arc::new(AtomicUsize::new(0)),
            arguments: Arc::new(tokio::sync::Mutex::new(None)),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let tools = server.list_tools(&CancellationToken::new()).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(
        tools[0].input_schema,
        json!({
            "type": "object",
            "properties": {
                "max_tokens": {"type": "integer", "default": 4096},
                "query": {"type": "string"}
            }
        })
    );
}

#[tokio::test]
async fn embedded_json_and_chinese_credentials_are_not_dispatched() {
    let inputs = [
        json!({"query": r#"{"secret":"fixture"}"#}),
        json!({"query": r#"{"url":"https:\/\/example.invalid\/scene?sig=fixture"}"#}),
        json!({"query": "密码：测试凭据"}),
        json!({"密码": "测试凭据"}),
        json!({"max_tokens": -1}),
        json!({"max_tokens": 1.5}),
    ];
    for (index, input) in inputs.iter().enumerate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let server = ApprovedTools::new(
            RecordingServer {
                calls: calls.clone(),
                arguments: Arc::new(tokio::sync::Mutex::new(None)),
            },
            vec!["search".into()],
            Duration::from_secs(1),
        );
        let error = server
            .call_tool("search", input, &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Parameter, "case {index}");
        assert_eq!(calls.load(Ordering::Relaxed), 0, "case {index}");
    }
}

#[tokio::test]
async fn benign_gis_text_and_token_budget_are_not_changed() {
    let calls = Arc::new(AtomicUsize::new(0));
    let arguments = Arc::new(tokio::sync::Mutex::new(None));
    let server = ApprovedTools::new(
        RecordingServer {
            calls: calls.clone(),
            arguments: arguments.clone(),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let input = json!({"max_tokens": 4096, "query": "Sentinel-1 VV / VH"});
    server
        .call_tool("search", &input, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(*arguments.lock().await, Some(input));
}

#[tokio::test]
async fn pre_cancelled_call_has_no_synchronous_client_side_effect() {
    let calls = Arc::new(AtomicUsize::new(0));
    let server = ApprovedTools::new(
        RecordingServer {
            calls: calls.clone(),
            arguments: Arc::new(tokio::sync::Mutex::new(None)),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = server
        .call_tool("search", &json!({}), &cancel)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn safe_result_is_preserved_and_sensitive_object_keys_are_rejected() {
    let result = json!({"max_tokens": 4096, "tokenizer": "bpe"});
    let safe = ApprovedTools::new(
        FakeServer {
            calls: Arc::new(AtomicUsize::new(0)),
            reply: result.clone(),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    assert_eq!(
        safe.call_tool("search", &json!({}), &CancellationToken::new())
            .await
            .unwrap(),
        result
    );
    for key in ["tok_FAKE9876543210abc", "C:/Users/fake/private"] {
        let mut object = serde_json::Map::new();
        object.insert(key.to_owned(), json!("[redacted]"));
        let server = ApprovedTools::new(
            FakeServer {
                calls: Arc::new(AtomicUsize::new(0)),
                reply: Value::Object(object),
            },
            vec!["search".into()],
            Duration::from_secs(1),
        );
        let error = server
            .call_tool("search", &json!({}), &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Capability);
        assert!(!format!("{error:?}").contains(key));
    }
}

struct SchemaServer {
    tool: RemoteTool,
}

impl McpClient for SchemaServer {
    fn list_tools<'a>(
        &'a self,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RemoteTool>, ModelError>> + Send + 'a>> {
        let tool = self.tool.clone();
        Box::pin(async move { Ok(vec![tool]) })
    }

    fn call_tool<'a>(
        &'a self,
        _name: &'a str,
        _arguments: &'a Value,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
        Box::pin(async { Ok(json!({})) })
    }

    fn shutdown<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), ModelError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn ordinary_schema_is_preserved_and_unsafe_metadata_is_rejected() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema#",
        "type": "object",
        "properties": {"tokenizer": {"type": "string"}}
    });
    let server = ApprovedTools::new(
        SchemaServer {
            tool: RemoteTool {
                name: "search".into(),
                description: "Sentinel-1 VV / VH".into(),
                input_schema: schema.clone(),
            },
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let tools = server.list_tools(&CancellationToken::new()).await.unwrap();
    assert_eq!(tools[0].input_schema, schema);
    assert_eq!(tools[0].description, "Sentinel-1 VV / VH");

    let server = ApprovedTools::new(
        SchemaServer {
            tool: RemoteTool {
                name: "search".into(),
                description: r#"{"token":"fixture-secret"}"#.into(),
                input_schema: json!({"type":"object"}),
            },
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let error = server
        .list_tools(&CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Capability);
    assert!(!format!("{error:?}").contains("fixture-secret"));
}

struct HungOrExited {
    exited: bool,
}

impl McpClient for HungOrExited {
    fn list_tools<'a>(
        &'a self,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RemoteTool>, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            if self.exited {
                Err(ModelError::new(ErrorKind::Transient, None))
            } else {
                std::future::pending().await
            }
        })
    }
    fn call_tool<'a>(
        &'a self,
        _name: &'a str,
        _args: &'a Value,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            if self.exited {
                Err(ModelError::new(ErrorKind::Transient, None))
            } else {
                std::future::pending().await
            }
        })
    }
    fn shutdown<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), ModelError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn unexpected_exit_is_transient_not_an_empty_tool_list() {
    let server = ApprovedTools::new(
        HungOrExited { exited: true },
        vec!["search".into()],
        Duration::from_millis(20),
    );
    let error = server
        .list_tools(&CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Transient);
}

#[tokio::test]
async fn timeout_and_cancellation_do_not_report_success() {
    let server = ApprovedTools::new(
        HungOrExited { exited: false },
        vec!["search".into()],
        Duration::from_millis(20),
    );
    let timeout = server
        .list_tools(&CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(timeout.kind, ErrorKind::Transient);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let interrupted = server
        .call_tool("search", &json!({}), &cancel)
        .await
        .unwrap_err();
    assert_eq!(interrupted.kind, ErrorKind::Cancelled);
}

async fn rejected_result(reply: Value) -> ModelError {
    let server = ApprovedTools::new(
        FakeServer {
            calls: Arc::new(AtomicUsize::new(0)),
            reply,
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    match server
        .call_tool("search", &json!({}), &CancellationToken::new())
        .await
    {
        Err(error) => error,
        Ok(output) => panic!("unsafe result left the boundary: {output}"),
    }
}

#[tokio::test]
async fn secret_shaped_key_names_never_leave_even_when_the_value_is_redacted() {
    for key in [
        "token_ghp_FAKE0123456789abcdef",
        "api_key sk-FAKEabc123456789",
        "secret_C:/Users/fake/private",
        "access_token_f00dfeed0123456789abcdef",
    ] {
        for value in [json!("fixture-value"), json!("[redacted]")] {
            let mut object = serde_json::Map::new();
            object.insert(key.to_owned(), value);
            let error = rejected_result(Value::Object(object)).await;
            assert_eq!(error.kind, ErrorKind::Capability, "{key}");
            assert!(!format!("{error:?}").contains(key), "{key}");
        }
    }
}

#[tokio::test]
async fn plain_sensitive_key_names_keep_only_a_placeholder() {
    let server = ApprovedTools::new(
        FakeServer {
            calls: Arc::new(AtomicUsize::new(0)),
            reply: json!({"api_key": "fixture-value", "access_token": "fixture-value", "scene": "S1A_IW_SLC__1SDV"}),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    let output = server
        .call_tool("search", &json!({}), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        output,
        json!({"api_key": "[redacted]", "access_token": "[redacted]", "scene": "S1A_IW_SLC__1SDV"})
    );
}

#[tokio::test]
async fn tokenizer_in_a_key_name_does_not_hide_a_credential_argument() {
    for key in ["tokenizer_auth_token", "TokenizerSessionToken"] {
        let calls = Arc::new(AtomicUsize::new(0));
        let server = ApprovedTools::new(
            RecordingServer {
                calls: calls.clone(),
                arguments: Arc::new(tokio::sync::Mutex::new(None)),
            },
            vec!["search".into()],
            Duration::from_secs(1),
        );
        let mut arguments = serde_json::Map::new();
        arguments.insert(key.to_owned(), json!("fixtureauthvalue"));
        let error = server
            .call_tool(
                "search",
                &Value::Object(arguments),
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Parameter, "{key}");
        assert_eq!(calls.load(Ordering::Relaxed), 0, "{key}");
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let server = ApprovedTools::new(
        RecordingServer {
            calls: calls.clone(),
            arguments: Arc::new(tokio::sync::Mutex::new(None)),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    server
        .call_tool(
            "search",
            &json!({"tokenizer": "bpe"}),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn an_oversized_result_is_refused_before_it_is_scanned() {
    let error = rejected_result(json!({"text": "x".repeat(2 * 1024 * 1024)})).await;
    assert_eq!(error.kind, ErrorKind::Capability);
}

#[tokio::test]
async fn full_width_and_percent_encoded_forms_do_not_hide_credentials_or_paths() {
    for text in [
        "Ｂｅａｒｅｒ fixture-secret-value",
        "ＡＰＩ＿ＫＥＹ=fixture-secret-value",
        "file:%2F%2F%2Fhome%2Ffake%2Fnotes.txt",
        "C%3A%5CUsers%5Cfake%5Cnotes.txt",
        "%2Fvar%2Fprivate%2Fnotes.txt",
    ] {
        // Since the Linux port the scrubber also decodes percent-encoded Unix
        // paths, so a result may come back fully replaced instead of refused;
        // either way nothing of the secret or the path leaves the boundary.
        let server = ApprovedTools::new(
            FakeServer {
                calls: Arc::new(AtomicUsize::new(0)),
                reply: json!({"note": text}),
            },
            vec!["search".into()],
            Duration::from_secs(1),
        );
        let leaked = match server
            .call_tool("search", &json!({}), &CancellationToken::new())
            .await
        {
            Err(error) => {
                assert_eq!(error.kind, ErrorKind::Capability, "{text}");
                format!("{error:?}")
            }
            Ok(output) => output.to_string(),
        };
        for fragment in [text, "fixture-secret-value", "fake", "private", "notes.txt"] {
            assert!(!leaked.contains(fragment), "{text}: {leaked}");
        }
    }
}

#[tokio::test]
async fn full_width_sensitive_key_names_are_still_sensitive() {
    for key in [
        "ＳＥＣＲＥＴ",
        "ＡＣＣＥＳＳ＿ＴＯＫＥＮ",
        "ＳＥＣＲＥＴ_zx9k4m2q8w7e5r1t",
    ] {
        let mut object = serde_json::Map::new();
        object.insert(key.to_owned(), json!("d3a9f7c2e1b84567"));
        let error = rejected_result(Value::Object(object)).await;
        assert_eq!(error.kind, ErrorKind::Capability, "{key}");
        assert!(!format!("{error:?}").contains("d3a9f7c2e1b84567"), "{key}");
    }
}

#[tokio::test]
async fn a_token_usage_count_is_not_mistaken_for_a_credential() {
    let server = ApprovedTools::new(
        FakeServer {
            calls: Arc::new(AtomicUsize::new(0)),
            reply: json!({"tokens_used": 1234}),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    assert_eq!(
        server
            .call_tool("search", &json!({}), &CancellationToken::new())
            .await
            .unwrap(),
        json!({"tokens_used": 1234})
    );
}

async fn dispatched_arguments(arguments: Value) -> Result<Value, ModelError> {
    let recorded = Arc::new(tokio::sync::Mutex::new(None));
    let server = ApprovedTools::new(
        RecordingServer {
            calls: Arc::new(AtomicUsize::new(0)),
            arguments: recorded.clone(),
        },
        vec!["search".into()],
        Duration::from_secs(1),
    );
    server
        .call_tool("search", &arguments, &CancellationToken::new())
        .await?;
    let sent = recorded.lock().await.clone();
    Ok(sent.expect("the call reached the server"))
}

async fn returned_result(reply: Value) -> Result<Value, ModelError> {
    ApprovedTools::new(
        FakeServer {
            calls: Arc::new(AtomicUsize::new(0)),
            reply,
        },
        vec!["search".into()],
        Duration::from_secs(1),
    )
    .call_tool("search", &json!({}), &CancellationToken::new())
    .await
}

const PUBLIC_CATALOG_QUERIES: [&str; 4] = [
    "https://catalogue.dataspace.copernicus.eu/odata/v1/Products?$filter=Collection/Name%20eq%20'SENTINEL-1'&$top=20",
    "https://cmr.earthdata.nasa.gov/search/granules.json?short_name=SENTINEL-1A_SLC&page_size=20",
    "https://api.daac.asf.alaska.edu/services/search/param?platform=S1&maxResults=10",
    "https://earth-search.aws.element84.com/v1/search?collections=sentinel-2-l2a&limit=5",
];

#[tokio::test]
async fn public_catalog_query_urls_pass_unchanged_both_ways() {
    for url in PUBLIC_CATALOG_QUERIES {
        let arguments = json!({"url": url});
        assert_eq!(
            dispatched_arguments(arguments.clone()).await.unwrap(),
            arguments,
            "{url}"
        );
        let result = json!({"next": url});
        assert_eq!(
            returned_result(result.clone()).await.unwrap(),
            result,
            "{url}"
        );
    }
}

#[tokio::test]
async fn other_query_urls_and_credential_parameters_stay_refused() {
    for url in [
        "https://example.invalid/search?q=sentinel",
        "http://cmr.earthdata.nasa.gov/search/granules.json?page_size=20",
        "https://cmr.earthdata.nasa.gov.example.invalid/search?page_size=20",
        "https://user@cmr.earthdata.nasa.gov/search/granules.json?page_size=20",
        "https://cmr.earthdata.nasa.gov/search/granules.json?page_size=20&signature=fixture",
        "https://cmr.earthdata.nasa.gov/search/granules.json?page_size=20&access_key=fixture",
        "https://cmr.earthdata.nasa.gov/search/granules.json?page_size=20&auth=fixture",
    ] {
        let error = dispatched_arguments(json!({"url": url})).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Parameter, "{url}");
        let returned = returned_result(json!({"next": url})).await;
        assert!(
            returned.map_or(true, |output| !output.to_string().contains(url)),
            "{url}"
        );
    }
}

#[tokio::test]
async fn pagination_cursors_round_trip_but_secret_shaped_cursors_do_not() {
    let result = json!({"next_page_token": "opaqueCursor123", "nextPageToken": "c2NlbmUtNDI"});
    assert_eq!(returned_result(result.clone()).await.unwrap(), result);
    let arguments = json!({"page_token": "opaqueCursor123"});
    assert_eq!(
        dispatched_arguments(arguments.clone()).await.unwrap(),
        arguments
    );
    let error = dispatched_arguments(json!({"next_page_token": "ghp_FAKE0123456789abcdef"}))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Parameter);
    let error = dispatched_arguments(json!({"session_token": "opaqueCursor123"}))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Parameter);
}

#[tokio::test]
async fn catalog_allowlist_cannot_be_borrowed_by_another_host_or_parameter_spelling() {
    for url in [
        "https://cmr.earthdata.nasa.gov%2f@evil.example/x?data=fixture-value",
        "https://cmr.earthdata.nasa.gov%2F%40evil.example/x?data=fixture-value",
        "https:/evil.example?data=fixture-value",
        "https:\\evil.example?data=fixture-value",
        "HTTPS:/\\evil.example?data=fixture-value",
        "https://cmr.earthdata.nasa.gov/search/granules.json?t%6Fken=fixture-value",
        "https://cmr.earthdata.nasa.gov/search/granules.json?q=1,https://evil.example/c?d=fixture-value",
        "https://cmr.earthdata.nasa.gov/search/granules.json?q=1,user:pw@evil.example",
        "https://cmr.earthdata.nasa.gov/search/granules.json?code=fixture-value",
        "https://cmr.earthdata.nasa.gov/search/granules.json?id_jwt=fixture-value",
    ] {
        let error = dispatched_arguments(json!({"url": url})).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Parameter, "{url}");
        let returned = returned_result(json!({"next": url})).await;
        assert!(
            returned.map_or(true, |output| !output.to_string().contains("fixture-value")),
            "{url}"
        );
    }
}

#[tokio::test]
async fn odata_skiptoken_paging_link_round_trips() {
    let url = "https://catalogue.dataspace.copernicus.eu/odata/v1/Products?$filter=Online%20eq%20true&$skiptoken=opaqueCursor123";
    let result = json!({"@odata.nextLink": url});
    assert_eq!(returned_result(result.clone()).await.unwrap(), result);
    let arguments = json!({"url": url});
    assert_eq!(
        dispatched_arguments(arguments.clone()).await.unwrap(),
        arguments
    );
}

#[tokio::test]
async fn labels_after_non_ascii_text_are_checked_without_panicking() {
    for text in ["界token=fixture-value", "说明：token: fixture-value"] {
        let error = rejected_result(json!({"note": text})).await;
        assert_eq!(error.kind, ErrorKind::Capability, "{text}");
    }
    let cursor = json!({"note": "界next_page_token=opaqueCursor123"});
    assert_eq!(returned_result(cursor.clone()).await.unwrap(), cursor);
}

#[tokio::test]
async fn percent_encoded_labels_in_a_catalog_query_value_are_refused() {
    for url in [
        "https://cmr.earthdata.nasa.gov/search/granules.json?q=a%26token%3Dfixture-value",
        "https://cmr.earthdata.nasa.gov/search/granules.json?q=a%2526secret%253Dfixture-value",
    ] {
        let error = dispatched_arguments(json!({"url": url})).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Parameter, "{url}");
    }
}
