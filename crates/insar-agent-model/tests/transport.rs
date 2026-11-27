use insar_agent_model::{
    transport::{Endpoint, ModelRoute},
    Message, Protocol, ReasoningMode, ToolDescription,
};
use serde_json::json;

#[test]
fn request_bodies_include_native_tool_schema_without_key() {
    let route = ModelRoute::new(
        Protocol::ChatCompletions,
        "https://example.invalid/v1",
        "local-model",
    )
    .unwrap();
    let messages = [Message {
        role: "user".into(),
        content: "Find DEM".into(),
    }];
    let tools = [ToolDescription {
        name: "search".into(),
        description: "read-only".into(),
        parameters: json!({"type":"object"}),
    }];
    let (url, body) = route.request(&messages, &tools).unwrap();
    assert_eq!(url.as_str(), "https://example.invalid/v1/chat/completions");
    assert_eq!(body["tools"][0]["function"]["name"], "search");
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert!(body.to_string().contains("Find DEM"));
    assert!(!body.to_string().contains("fake-key"));

    let responses = ModelRoute::new(
        Protocol::Responses,
        "https://example.invalid/v1",
        "local-model",
    )
    .unwrap();
    assert_eq!(
        responses.request(&messages, &tools).unwrap().1["tools"][0]["type"],
        "function"
    );
    let anthropic = ModelRoute::new(
        Protocol::AnthropicMessages,
        "https://example.invalid",
        "claude",
    )
    .unwrap();
    let (url, body) = anthropic.request(&messages, &tools).unwrap();
    assert_eq!(url.as_str(), "https://example.invalid/v1/messages");
    assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    let (url, _) = ModelRoute::new(
        Protocol::AnthropicMessages,
        "https://example.invalid/v1",
        "claude",
    )
    .unwrap()
    .request(&messages, &tools)
    .unwrap();
    assert_eq!(url.as_str(), "https://example.invalid/v1/messages");
}

#[test]
fn outbound_scrub_removes_sensitive_punctuated_path_and_url_without_disclosing_key() {
    let route =
        ModelRoute::new(Protocol::ChatCompletions, "https://example.invalid/v1", "m").unwrap();
    let (_, body) = route.request(&[Message { role: "user".into(), content: "Inspect (C:\\Users\\example\\private.tif), see https://example.invalid/data?signature=fixture-only".into() }], &[]).unwrap();
    let text = body.to_string();
    assert!(!text.contains("private.tif"));
    assert!(!text.contains("signature="));
    assert!(text.contains("[local-reference]"));
}

#[test]
fn outbound_scrub_does_not_leak_a_secret_from_structured_model_messages() {
    let route =
        ModelRoute::new(Protocol::ChatCompletions, "https://example.invalid/v1", "m").unwrap();
    let (_, body) = route
        .request(
            &[Message {
                role: "tool".into(),
                content: json!({"id":"call-1","result":{"password":"not-a-real-secret"}})
                    .to_string(),
            }],
            &[],
        )
        .unwrap();
    assert!(!body.to_string().contains("not-a-real-secret"));
}

#[test]
fn outbound_scrub_removes_nested_json_secret_from_user_text() {
    let route =
        ModelRoute::new(Protocol::ChatCompletions, "https://example.invalid/v1", "m").unwrap();
    let text = json!({"password":"not-a-real-secret","question":"find data"}).to_string();
    let (_, body) = route
        .request(
            &[Message {
                role: "user".into(),
                content: text,
            }],
            &[],
        )
        .unwrap();
    assert!(!body.to_string().contains("not-a-real-secret"));
}

#[test]
fn route_rejects_insecure_non_loopback_and_url_credentials() {
    assert!(ModelRoute::new(Protocol::ChatCompletions, "http://example.invalid/v1", "m").is_err());
    assert!(ModelRoute::new(
        Protocol::ChatCompletions,
        "https://user:pass@example.invalid/v1",
        "m"
    )
    .is_err());
    assert!(ModelRoute::new(
        Protocol::ChatCompletions,
        "https://example.invalid/v1?token=fake",
        "m"
    )
    .is_err());
    assert!(ModelRoute::new(Protocol::ChatCompletions, "http://127.0.0.1:11434/v1", "m").is_ok());
    assert!(Endpoint::new("https://example.invalid/v1", Protocol::Responses).is_ok());
}

#[test]
fn anthropic_system_message_is_not_sent_in_messages_array() {
    let route =
        ModelRoute::new(Protocol::AnthropicMessages, "https://example.invalid", "m").unwrap();
    let messages = [
        Message {
            role: "system".into(),
            content: "Catalog policy".into(),
        },
        Message {
            role: "user".into(),
            content: "Find data".into(),
        },
    ];
    let body = route.request(&messages, &[]).unwrap().1;
    assert_eq!(body["system"], "Catalog policy");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
}

#[test]
fn tool_reply_is_bound_to_call_id_on_each_protocol() {
    let messages = [Message { role: "user".into(), content: "Find data".into() },
        Message { role: "assistant_tool_calls".into(), content: serde_json::json!({"text":"", "tool_calls":[{"id":"call-1", "name":"search", "arguments":{}}]}).to_string() },
        Message { role: "tool".into(), content: serde_json::json!({"id":"call-1", "result":{"ok":true}}).to_string() }];
    let chat = ModelRoute::new(Protocol::ChatCompletions, "https://example.invalid/v1", "m")
        .unwrap()
        .request(&messages, &[])
        .unwrap()
        .1;
    assert_eq!(chat["messages"][2]["tool_call_id"], "call-1");
    let responses = ModelRoute::new(Protocol::Responses, "https://example.invalid/v1", "m")
        .unwrap()
        .request(&messages, &[])
        .unwrap()
        .1;
    assert_eq!(responses["input"][1]["type"], "function_call");
    assert_eq!(responses["input"][1]["call_id"], "call-1");
    assert_eq!(responses["input"][2]["call_id"], "call-1");
    let anthropic = ModelRoute::new(Protocol::AnthropicMessages, "https://example.invalid", "m")
        .unwrap()
        .request(&messages, &[])
        .unwrap()
        .1;
    assert_eq!(
        anthropic["messages"][2]["content"][0]["tool_use_id"],
        "call-1"
    );
}

#[test]
fn keyed_model_reports_the_exact_scrubbed_body_and_keeps_protocol_fields() {
    let route = ModelRoute::new(
        Protocol::AnthropicMessages,
        "https://example.invalid",
        "claude-fixture",
    )
    .unwrap();
    let messages = [Message {
        role: "user".into(),
        content: "Inspect C:\\Users\\example\\private.tif".into(),
    }];
    let model = insar_agent_model::agent::KeyedModel::new(
        insar_agent_model::transport::ModelClient::new(std::time::Duration::from_secs(1)).unwrap(),
        route.clone(),
        insar_agent_model::Zeroizing::new("fixture-only-key".to_owned()),
    );
    let body = insar_agent_model::agent::Model::request_body(&model, &messages, &[])
        .expect("a networked model reports its request body")
        .unwrap();
    assert_eq!(body, route.request(&messages, &[]).unwrap().1);
    // The protocol envelope is not user content: scrubbing must not turn the
    // token budget into "[redacted]".
    assert_eq!(body["max_tokens"], 4096);
    let text = body.to_string();
    assert!(!text.contains("private.tif"));
    assert!(!text.contains("fixture-only-key"));
}

fn body_for(protocol: Protocol, model: &str, mode: ReasoningMode) -> serde_json::Value {
    let base = if protocol == Protocol::AnthropicMessages {
        "https://example.invalid"
    } else {
        "https://example.invalid/v1"
    };
    ModelRoute::new(protocol, base, model)
        .unwrap()
        .with_reasoning(mode)
        .request(
            &[Message {
                role: "user".into(),
                content: "Find data".into(),
            }],
            &[],
        )
        .unwrap()
        .1
}

#[test]
fn new_routes_default_to_reasoning_off() {
    let route =
        ModelRoute::new(Protocol::ChatCompletions, "https://example.invalid/v1", "m").unwrap();
    assert_eq!(route.reasoning, ReasoningMode::Off);
}

#[test]
fn chat_completions_disables_glm_thinking_only_when_reasoning_is_off() {
    use ReasoningMode::{Off, ProviderDefault};
    for model in ["glm-5.3-flash", "GLM-4.6", "zhipu/glm-4.5-air"] {
        let off = body_for(Protocol::ChatCompletions, model, Off);
        assert_eq!(off["thinking"], json!({"type": "disabled"}), "{model}");
        assert_eq!(off["max_tokens"], 4096, "{model}");
        assert!(off.get("reasoning_effort").is_none(), "{model}");
        let default = body_for(Protocol::ChatCompletions, model, ProviderDefault);
        assert!(default.get("thinking").is_none(), "{model}");
        assert_eq!(default["max_tokens"], 4096, "{model}");
    }
    // Other families keep the provider's own budget and get no switch.
    for model in ["gpt-4o", "deepseek-chat", "local-model", "ollama/llama3"] {
        for mode in [Off, ProviderDefault] {
            let body = body_for(Protocol::ChatCompletions, model, mode);
            assert!(body.get("max_tokens").is_none(), "{model}");
            assert!(body.get("thinking").is_none(), "{model}");
            assert!(body.get("reasoning_effort").is_none(), "{model}");
        }
    }
}

#[test]
fn chat_completions_uses_reasoning_effort_for_openai_reasoning_models_without_max_tokens() {
    use ReasoningMode::{Off, ProviderDefault};
    // These models refuse `max_tokens` on chat completions; the provider
    // default budget applies (`max_completion_tokens` is not sent because the
    // run record refuses token-named fields other than `max_tokens`).
    for (model, effort) in [
        ("gpt-5", Some("minimal")),
        ("gpt-5-mini", Some("minimal")),
        ("o3-mini", Some("low")),
        ("o1", Some("low")),
        ("o4-mini", Some("low")),
        ("gpt-5.1", None),
    ] {
        let off = body_for(Protocol::ChatCompletions, model, Off);
        assert!(off.get("max_tokens").is_none(), "{model}");
        assert!(off.get("thinking").is_none(), "{model}");
        assert_eq!(
            off.get("reasoning_effort").and_then(|v| v.as_str()),
            effort,
            "{model}"
        );
        let default = body_for(Protocol::ChatCompletions, model, ProviderDefault);
        assert!(default.get("max_tokens").is_none(), "{model}");
        assert!(default.get("reasoning_effort").is_none(), "{model}");
    }
}

#[test]
fn responses_sends_the_lowest_reasoning_effort_only_to_reasoning_models_when_off() {
    use ReasoningMode::{Off, ProviderDefault};
    assert_eq!(
        body_for(Protocol::Responses, "gpt-5", Off)["reasoning"],
        json!({"effort": "minimal"})
    );
    assert_eq!(
        body_for(Protocol::Responses, "o4-mini", Off)["reasoning"],
        json!({"effort": "low"})
    );
    for (model, mode) in [
        ("gpt-5", ProviderDefault),
        ("gpt-4o", Off),
        ("gpt-5.1", Off),
        ("glm-5.3-flash", Off),
    ] {
        let body = body_for(Protocol::Responses, model, mode);
        assert!(body.get("reasoning").is_none(), "{model} {mode:?}");
        assert!(body.get("thinking").is_none(), "{model} {mode:?}");
        assert!(body.get("max_tokens").is_none(), "{model} {mode:?}");
    }
}

#[test]
fn anthropic_never_sends_thinking_in_either_mode() {
    for mode in [ReasoningMode::Off, ReasoningMode::ProviderDefault] {
        let body = body_for(Protocol::AnthropicMessages, "claude-sonnet-4-5", mode);
        assert!(body.get("thinking").is_none(), "{mode:?}");
        assert_eq!(body["max_tokens"], 4096);
    }
}

#[test]
fn gpt_5_chat_is_matched_before_the_gpt_5_reasoning_family() {
    use ReasoningMode::Off;
    // `gpt-5-chat-latest` also starts with `gpt-5-`; it must not get an effort.
    for model in ["gpt-5-chat-latest", "openai/GPT-5-chat"] {
        let chat = body_for(Protocol::ChatCompletions, model, Off);
        assert!(chat.get("reasoning_effort").is_none(), "{model}");
        assert!(chat.get("max_tokens").is_none(), "{model}");
        assert!(
            body_for(Protocol::Responses, model, Off)
                .get("reasoning")
                .is_none(),
            "{model}"
        );
    }
    assert_eq!(
        body_for(Protocol::ChatCompletions, "gpt-5-mini", Off)["reasoning_effort"],
        "minimal"
    );
}

#[test]
fn the_family_is_read_from_the_id_after_the_last_slash() {
    use ReasoningMode::Off;
    assert_eq!(
        body_for(Protocol::ChatCompletions, "openai/o3-mini", Off)["reasoning_effort"],
        "low"
    );
    assert_eq!(
        body_for(Protocol::ChatCompletions, "relay/zhipu/glm-4.6", Off)["thinking"],
        json!({"type": "disabled"})
    );
    // A family name earlier in the path, or an `o` word that is not `o<digits>`,
    // does not count.
    for model in ["glm/gpt-4o", "o3/llama3", "omni-moderation", "o3x-mini"] {
        let body = body_for(Protocol::ChatCompletions, model, Off);
        assert!(body.get("thinking").is_none(), "{model}");
        assert!(body.get("reasoning_effort").is_none(), "{model}");
        assert!(body.get("max_tokens").is_none(), "{model}");
    }
}

/// W3o item 2: once a channel is known to refuse the reasoning switch, its
/// requests carry no switch and a larger output budget (`max_tokens` 8192:
/// the model thinks, and the thinking must not use up the answer's room).
/// `max_tokens` rather than `max_completion_tokens`: the latter is the
/// OpenAI name for reasoning models, which these routes do not send a budget
/// to at all, and the run record refuses token-named fields other than
/// `max_tokens`. A channel that never asked to turn thinking off, or a
/// family with no switch, is unchanged.
#[test]
fn a_channel_known_to_refuse_the_switch_sends_none_and_a_larger_budget() {
    use ReasoningMode::{Off, ProviderDefault};
    let body = |protocol: Protocol, model: &str, mode: ReasoningMode| {
        let base = if protocol == Protocol::AnthropicMessages {
            "https://example.invalid"
        } else {
            "https://example.invalid/v1"
        };
        let route = ModelRoute::new(protocol, base, model)
            .unwrap()
            .with_reasoning(mode)
            .with_reasoning_switch_unsupported(true);
        let sends = route.sends_reasoning_switch();
        let refused = route.reasoning_switch_refused();
        (route.request(&[], &[]).unwrap().1, sends, refused)
    };
    let (glm, sends, refused) = body(Protocol::ChatCompletions, "glm-5.3-flash", Off);
    assert!(!sends && refused);
    assert!(glm.get("thinking").is_none(), "{glm}");
    assert_eq!(glm["max_tokens"], 8192, "{glm}");
    let (glm, sends, refused) = body(Protocol::ChatCompletions, "glm-5.3-flash", ProviderDefault);
    assert!(!sends && !refused, "thinking was not asked to be off");
    assert_eq!(glm["max_tokens"], 4096, "{glm}");
    let (gpt, sends, refused) = body(Protocol::ChatCompletions, "gpt-5", Off);
    assert!(!sends && refused);
    assert!(gpt.get("reasoning_effort").is_none(), "{gpt}");
    assert!(gpt.get("max_tokens").is_none(), "{gpt}");
    let (claude, sends, refused) = body(Protocol::AnthropicMessages, "claude-x", Off);
    assert!(!sends && !refused, "anthropic never sends a switch");
    assert_eq!(claude["max_tokens"], 4096, "{claude}");
    // Without the flag nothing changes.
    let plain = body_for(Protocol::ChatCompletions, "glm-5.3-flash", Off);
    assert_eq!(plain["max_tokens"], 4096);
    assert_eq!(plain["thinking"], json!({"type": "disabled"}));
}

#[test]
fn google_gemini_request_builds_v1beta_sse_url_sanitizes_schemas_and_maps_tools() {
    let route = ModelRoute::new(
        Protocol::GoogleGemini,
        "https://generativelanguage.googleapis.com",
        "gemini-2.5-flash",
    )
    .unwrap()
    .with_max_output_tokens(Some(64));
    let messages = [
        Message {
            role: "system".into(),
            content: "Be concise".into(),
        },
        Message {
            role: "user".into(),
            content: "Check status".into(),
        },
        Message {
            role: "assistant_tool_calls".into(),
            content: json!({
                "text": "",
                "tool_calls": [{"id": "c1", "name": "ping_tool", "arguments": {"q": "a"}}]
            })
            .to_string(),
        },
        Message {
            role: "tool".into(),
            content: json!({"id": "c1", "result": {"ok": true}}).to_string(),
        },
    ];
    let tools = [ToolDescription {
        name: "ping_tool".into(),
        description: "ping".into(),
        parameters: json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": false,
            "properties": {"q": {"type": "string"}}
        }),
    }];
    let (url, body) = route.request(&messages, &tools).unwrap();
    assert_eq!(
        url.as_str(),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
    assert!(url.query().is_some_and(|q| !q.contains("key=")));
    assert_eq!(body["systemInstruction"]["parts"][0]["text"], "Be concise");
    assert_eq!(body["contents"][0]["role"], "user");
    assert_eq!(
        body["contents"][1]["parts"][0]["functionCall"]["name"],
        "ping_tool"
    );
    assert_eq!(
        body["contents"][2]["parts"][0]["functionResponse"]["name"],
        "ping_tool"
    );
    let decl = &body["tools"][0]["functionDeclarations"][0]["parameters"];
    assert!(decl.get("$schema").is_none());
    assert!(decl.get("additionalProperties").is_none());
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 64);

    // Responses also maps system prompt into top-level instructions
    let resp_route = ModelRoute::new(
        Protocol::Responses,
        "https://chatgpt.com/backend-api/codex",
        "gpt-5-codex",
    )
    .unwrap();
    let (_, resp_body) = resp_route.request(&messages[..2], &[]).unwrap();
    assert_eq!(resp_body["instructions"], "Be concise");
    assert_eq!(resp_body["input"].as_array().unwrap().len(), 1);
}

#[test]
fn multimodal_image_serializes_across_all_four_protocols_and_rejects_fake_base64_secrets() {
    // Valid 1x1 transparent PNG
    let valid_png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    // Fake base64 payload (plain ASCII "sk-proj-fake1234567890" encoded, no PNG header)
    let fake_secret_b64 = "c2stcHJvai1mYWtlMTIzNDU2Nzg5MA==";
    let messages = [Message {
        role: "user_multimodal".into(),
        content: json!({
            "text": "请分析这张图，注意令牌 Bearer secret-leak-value 不得发出",
            "images": [
                {"mime_type": "image/png", "data_base64": valid_png_b64},
                {"mime_type": "image/png", "data_base64": fake_secret_b64}
            ]
        })
        .to_string(),
    }];

    let chat = ModelRoute::new(
        Protocol::ChatCompletions,
        "https://example.invalid/v1",
        "gpt-4o",
    )
    .unwrap();
    let (_, chat_body) = chat.request(&messages, &[]).unwrap();
    let chat_parts = chat_body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(chat_parts.len(), 2, "fake image dropped, valid PNG kept");
    assert!(!chat_parts[0]["text"]
        .as_str()
        .unwrap()
        .contains("secret-leak-value"));
    assert_eq!(chat_parts[1]["type"], "image_url");
    assert_eq!(
        chat_parts[1]["image_url"]["url"],
        format!("data:image/png;base64,{valid_png_b64}")
    );

    let responses =
        ModelRoute::new(Protocol::Responses, "https://example.invalid/v1", "gpt-5").unwrap();
    let (_, resp_body) = responses.request(&messages, &[]).unwrap();
    let resp_parts = resp_body["input"][0]["content"].as_array().unwrap();
    assert_eq!(resp_parts.len(), 2);
    assert_eq!(resp_parts[0]["type"], "input_text");
    assert_eq!(resp_parts[1]["type"], "input_image");
    assert_eq!(
        resp_parts[1]["image_url"],
        format!("data:image/png;base64,{valid_png_b64}")
    );

    let anthropic = ModelRoute::new(
        Protocol::AnthropicMessages,
        "https://example.invalid/v1",
        "claude-sonnet-4",
    )
    .unwrap();
    let (_, anth_body) = anthropic.request(&messages, &[]).unwrap();
    let anth_parts = anth_body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(anth_parts.len(), 2);
    assert_eq!(anth_parts[1]["type"], "image");
    assert_eq!(anth_parts[1]["source"]["type"], "base64");
    assert_eq!(anth_parts[1]["source"]["media_type"], "image/png");
    assert_eq!(anth_parts[1]["source"]["data"], valid_png_b64);

    let gemini = ModelRoute::new(
        Protocol::GoogleGemini,
        "https://generativelanguage.googleapis.com",
        "gemini-2.5-flash",
    )
    .unwrap();
    let (_, gem_body) = gemini.request(&messages, &[]).unwrap();
    let gem_parts = gem_body["contents"][0]["parts"].as_array().unwrap();
    assert_eq!(gem_parts.len(), 2);
    assert_eq!(gem_parts[1]["inline_data"]["mime_type"], "image/png");
    assert_eq!(gem_parts[1]["inline_data"]["data"], valid_png_b64);
}

#[test]
fn prompt_cache_key_wires_responses_and_anthropic_cache_control_and_merges_parallel_tool_results() {
    let tools = [ToolDescription {
        name: "task_overview".into(),
        description: "read queue".into(),
        parameters: json!({"type": "object"}),
    }];
    let messages = [
        Message {
            role: "system".into(),
            content: "Static prompt prefix".into(),
        },
        Message {
            role: "user".into(),
            content: "Check".into(),
        },
        Message {
            role: "assistant_tool_calls".into(),
            content: json!({
                "text": "checking",
                "tool_calls": [
                    {"id": "c1", "name": "task_overview", "arguments": {}},
                    {"id": "c2", "name": "task_overview", "arguments": {}}
                ]
            })
            .to_string(),
        },
        Message {
            role: "tool".into(),
            content: json!({"id": "c1", "result": {"ok": true, "n": 1}}).to_string(),
        },
        Message {
            role: "tool".into(),
            content: json!({"id": "c2", "result": {"ok": true, "n": 2}}).to_string(),
        },
    ];

    let codex = ModelRoute::new(
        Protocol::Responses,
        "https://chatgpt.com/backend-api/codex",
        "gpt-5-codex",
    )
    .unwrap()
    .with_prompt_cache_key(Some("eostage-0123456789abcdef01234567".into()));
    let (_, codex_body) = codex.request(&messages, &tools).unwrap();
    assert_eq!(codex_body["store"], false);
    assert_eq!(
        codex_body["prompt_cache_key"],
        "eostage-0123456789abcdef01234567"
    );

    let anthropic = ModelRoute::new(
        Protocol::AnthropicMessages,
        "https://example.invalid/v1",
        "claude-sonnet-4",
    )
    .unwrap()
    .with_prompt_cache_key(Some("eostage-0123456789abcdef01234567".into()));
    let (_, anth_body) = anthropic.request(&messages, &tools).unwrap();
    assert_eq!(anth_body["system"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(anth_body["tools"][0]["cache_control"]["type"], "ephemeral");
    // Consecutive tool_result messages MUST merge into one role=user message
    let anth_msgs = anth_body["messages"].as_array().unwrap();
    assert_eq!(
        anth_msgs.len(),
        3,
        "user -> assistant -> merged tool_results"
    );
    assert_eq!(anth_msgs[1]["role"], "assistant");
    assert_eq!(anth_msgs[1]["content"][0]["type"], "text");
    assert_eq!(anth_msgs[1]["content"][1]["type"], "tool_use");
    assert_eq!(anth_msgs[2]["role"], "user");
    let merged_results = anth_msgs[2]["content"].as_array().unwrap();
    assert_eq!(merged_results.len(), 2);
    assert_eq!(merged_results[0]["tool_use_id"], "c1");
    assert_eq!(merged_results[1]["tool_use_id"], "c2");
}

#[test]
fn chat_completions_carries_prompt_cache_key_and_anthropic_uses_four_rolling_breakpoints() {
    let tools = [ToolDescription {
        name: "task_overview".into(),
        description: "read queue".into(),
        parameters: json!({"type": "object"}),
    }];
    let valid_png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let messages = [
        Message {
            role: "system".into(),
            content: "Static prompt prefix".into(),
        },
        Message {
            role: "user".into(),
            content: "Turn 1 user".into(),
        },
        Message {
            role: "assistant".into(),
            content: "Turn 1 reply".into(),
        },
        Message {
            role: "user".into(),
            content: "Turn 2 user".into(),
        },
        Message {
            role: "assistant".into(),
            content: "".into(),
        },
        Message {
            role: "user_multimodal".into(),
            content: json!({
                "text": "",
                "images": [{"mime_type": "image/png", "data_base64": valid_png_b64}]
            })
            .to_string(),
        },
    ];

    let chat = ModelRoute::new(
        Protocol::ChatCompletions,
        "https://example.invalid/v1",
        "glm-5.3-flash",
    )
    .unwrap()
    .with_prompt_cache_key(Some("eostage-chatkey123".into()));
    let (_, chat_body) = chat.request(&messages, &tools).unwrap();
    assert_eq!(chat_body["prompt_cache_key"], "eostage-chatkey123");

    let anth = ModelRoute::new(
        Protocol::AnthropicMessages,
        "https://example.invalid/v1",
        "claude-sonnet-4",
    )
    .unwrap()
    .with_prompt_cache_key(Some("eostage-anthkey123".into()));
    let (_, anth_body) = anth.request(&messages, &tools).unwrap();
    let anth_msgs = anth_body["messages"].as_array().unwrap();
    assert_eq!(anth_msgs.len(), 5);
    assert_eq!(anth_body["tools"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(anth_body["system"][0]["cache_control"]["type"], "ephemeral");
    // len - 3 = index 2 (Turn 2 user), len - 1 = index 4 (multimodal image block)
    assert_eq!(
        anth_msgs[2]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    let last_img = &anth_msgs[4]["content"][0];
    assert_eq!(last_img["type"], "image");
    assert_eq!(last_img["cache_control"]["type"], "ephemeral");
    assert!(
        insar_agent_model::privacy::is_safe_multimodal_image_block(last_img),
        "Anthropic image block with cache_control must stay safe for outbound check"
    );
    assert_eq!(insar_agent_model::privacy::scrub(last_img), *last_img);

    // Without prompt_cache_key, ZERO cache_control fields are emitted anywhere
    let anth_no_key = ModelRoute::new(
        Protocol::AnthropicMessages,
        "https://example.invalid/v1",
        "claude-sonnet-4",
    )
    .unwrap();
    let (_, no_key_body) = anth_no_key.request(&messages, &tools).unwrap();
    assert!(!no_key_body.to_string().contains("cache_control"));
}

#[test]
fn gemini_contents_merges_consecutive_same_role_messages() {
    let route = ModelRoute::new(
        Protocol::GoogleGemini,
        "https://generativelanguage.googleapis.com",
        "gemini-2.5-flash",
    )
    .unwrap();
    let messages = [
        Message {
            role: "assistant_tool_calls".into(),
            content: json!({
                "text": "",
                "tool_calls": [{"id": "c1", "name": "ping_tool", "arguments": {}}]
            })
            .to_string(),
        },
        Message {
            role: "tool".into(),
            content: json!({"id": "c1", "result": {"ok": true}}).to_string(),
        },
        Message {
            role: "user".into(),
            content: "[系统提示] 已达单轮工具调用上限".into(),
        },
    ];
    let (_, body) = route.request(&messages, &[]).unwrap();
    let contents = body["contents"].as_array().unwrap();
    assert_eq!(contents.len(), 2, "tool + user nudge must merge: {body}");
    assert_eq!(contents[0]["role"], "model");
    assert_eq!(contents[1]["role"], "user");
    let parts = contents[1]["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["functionResponse"]["name"], "ping_tool");
    assert_eq!(parts[1]["text"], "[系统提示] 已达单轮工具调用上限");
}
