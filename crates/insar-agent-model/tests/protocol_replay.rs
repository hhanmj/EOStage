use insar_agent_model::{replay, ErrorKind, ModelEvent, Protocol, StreamDecoder};
use tokio_util::sync::CancellationToken;

#[test]
fn chat_stream_assembles_split_tool_arguments_and_usage() {
    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello \",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"q\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"world\",\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"dem\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":5}}\n\n",
        "data: [DONE]\n\n"
    );
    let mut events = Vec::new();
    let completion = replay(
        Protocol::ChatCompletions,
        frames,
        &CancellationToken::new(),
        |event| events.push(event),
    )
    .unwrap();
    assert_eq!(completion.text, "Hello world");
    assert_eq!(completion.tool_calls[0].arguments["q"], "dem");
    assert_eq!(completion.usage.input_tokens, Some(3));
    assert!(events.contains(&ModelEvent::Text("Hello ".into())));
}

#[test]
fn responses_stream_assembles_function_call_and_text() {
    let frames = concat!(
        "event: response.output_text.delta\ndata: {\"delta\":\"Found \"}\n\n",
        "event: response.output_text.delta\ndata: {\"delta\":\"data\"}\n\n",
        "event: response.output_item.added\ndata: {\"output_index\":1,\"item\":{\"type\":\"function_call\",\"call_id\":\"r1\",\"name\":\"query\"}}\n\n",
        "event: response.function_call_arguments.delta\ndata: {\"output_index\":1,\"delta\":\"{}\"}\n\n",
        "event: response.completed\ndata: {\"response\":{\"usage\":{\"input_tokens\":9,\"output_tokens\":4}}}\n\n"
    );
    let result = replay(
        Protocol::Responses,
        frames,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.text, "Found data");
    assert_eq!(result.tool_calls[0].name, "query");
    assert_eq!(result.usage.output_tokens, Some(4));
}

#[test]
fn anthropic_stream_assembles_tool_and_usage() {
    let frames = concat!(
        "event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":7}}}\n\n",
        "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"Hi\"}}\n\n",
        "event: content_block_start\ndata: {\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"a1\",\"name\":\"query\",\"input\":{}}}\n\n",
        "event: content_block_delta\ndata: {\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"a\\\":1}\"}}\n\n",
        "event: message_delta\ndata: {\"usage\":{\"output_tokens\":6}}\n\n",
        "event: message_stop\ndata: {}\n\n"
    );
    let result = replay(
        Protocol::AnthropicMessages,
        frames,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.text, "Hi");
    assert_eq!(result.tool_calls[0].arguments["a"], 1);
    assert_eq!(result.usage.input_tokens, Some(7));
}

#[test]
fn anthropic_empty_input_tool_call_is_valid_json() {
    let frames = concat!(
        "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"a0\",\"name\":\"search\",\"input\":{}}}\n\n",
        "event: message_stop\ndata: {}\n\n"
    );
    let result = replay(
        Protocol::AnthropicMessages,
        frames,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.tool_calls[0].arguments, serde_json::json!({}));
}

#[test]
fn responses_done_arguments_does_not_duplicate_prior_delta() {
    let frames = concat!(
        "event: response.output_item.added\ndata: {\"output_index\":1,\"item\":{\"type\":\"function_call\",\"call_id\":\"r1\",\"name\":\"query\"}}\n\n",
        "event: response.function_call_arguments.delta\ndata: {\"output_index\":1,\"delta\":\"{\\\"q\\\":1}\"}\n\n",
        "event: response.function_call_arguments.done\ndata: {\"output_index\":1,\"arguments\":\"{\\\"q\\\":1}\"}\n\n",
        "event: response.completed\ndata: {\"response\":{\"usage\":{\"input_tokens\":9,\"output_tokens\":4}}}\n\n"
    );
    let result = replay(
        Protocol::Responses,
        frames,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.tool_calls[0].arguments["q"], 1);
}

#[test]
fn responses_failed_event_is_a_classified_provider_error() {
    for (code, expected) in [
        ("server_error", ErrorKind::Transient),
        ("rate_limit_exceeded", ErrorKind::Transient),
        ("invalid_api_key", ErrorKind::Authentication),
        ("invalid_request_error", ErrorKind::Parameter),
        ("unsupported_feature", ErrorKind::Capability),
        ("unrecognized_failure", ErrorKind::Unknown),
    ] {
        let frames = format!(
            "event: response.failed\ndata: {}\n\n",
            serde_json::json!({"type":"response.failed","response":{"status":"failed","error":{"code":code,"message":"secret upstream detail"}}})
        );
        let mut decoder = StreamDecoder::new(Protocol::Responses);
        let error = decoder
            .feed(&frames, &CancellationToken::new(), &mut |_| {})
            .expect_err("a provider failure must be reported before waiting for EOF");
        assert_eq!(error.kind, expected, "{code}");
        assert_eq!(error.status, None);
        assert!(!format!("{error:?}").contains("secret upstream detail"));
    }
}

#[test]
fn responses_failed_event_never_releases_a_pending_tool_call() {
    let frames = concat!(
        "event: response.output_item.added\ndata: {\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"r1\",\"name\":\"query\",\"arguments\":\"{}\"}}\n\n",
        "event: response.failed\ndata: {\"response\":{\"status\":\"failed\",\"error\":{\"code\":\"server_error\",\"message\":\"secret upstream detail\"}}}\n\n",
        "data: [DONE]\n\n"
    );
    let mut events = Vec::new();
    let error = replay(
        Protocol::Responses,
        frames,
        &CancellationToken::new(),
        |event| events.push(event),
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Transient);
    assert!(
        events.is_empty(),
        "failed responses must not emit executable calls"
    );
}

#[test]
fn responses_flat_error_event_uses_its_code_without_leaking_detail() {
    let error = replay(
        Protocol::Responses,
        "event: error\ndata: {\"type\":\"error\",\"code\":\"invalid_api_key\",\"message\":\"secret upstream detail\"}\n\n",
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Authentication);
    assert!(!format!("{error:?}").contains("secret upstream detail"));
}

#[test]
fn cancellation_is_not_a_successful_empty_answer() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = replay(
        Protocol::ChatCompletions,
        "data: [DONE]\n\n",
        &cancel,
        |_| {},
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
}

#[test]
fn decoder_rejects_unbounded_partial_event_frames() {
    let mut decoder = StreamDecoder::new(Protocol::ChatCompletions);
    let cancel = CancellationToken::new();
    let big = "x".repeat(1024 * 1024 + 1);
    let error = decoder.feed(&big, &cancel, &mut |_| {}).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Capability);
}

#[test]
fn decoder_rejects_unbounded_tool_arguments_across_small_events() {
    let mut decoder = StreamDecoder::new(Protocol::ChatCompletions);
    let cancel = CancellationToken::new();
    for _ in 0..1500 {
        let event = format!("data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"function\":{{\"arguments\":\"{}\"}}}}]}}}}]}}\n\n", "x".repeat(800));
        if let Err(error) = decoder.feed(&event, &cancel, &mut |_| {}) {
            assert_eq!(error.kind, ErrorKind::Capability);
            return;
        }
    }
    panic!("tool argument accumulator was not bounded");
}

#[test]
fn upstream_error_classification_has_no_secret_message() {
    let result = replay(Protocol::AnthropicMessages, "event: error\ndata: {\"error\":{\"type\":\"authentication_error\",\"message\":\"fake-secret\"}}\n\n", &CancellationToken::new(), |_| {}).unwrap_err();
    assert_eq!(result.kind, ErrorKind::Authentication);
    assert!(!format!("{result:?}").contains("fake-secret"));
}

#[test]
fn chat_stream_emits_reasoning_deltas_and_counts_reasoning_tokens() {
    // Agent 2.0 Slice A: dedicated `reasoning_content` deltas emit structured
    // ThinkingStart / ThinkingDelta / ThinkingEnd events, populate
    // `completion.reasoning_text` + `thinking_ms`, and never enter `completion.text`.
    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"Let me think\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\" about 17*23.\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"391\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":40,\"completion_tokens_details\":{\"reasoning_tokens\":38}}}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":null}\n\n",
        "data: [DONE]\n\n"
    );
    let mut events = Vec::new();
    let completion = replay(
        Protocol::ChatCompletions,
        frames,
        &CancellationToken::new(),
        |event| events.push(event),
    )
    .unwrap();
    assert_eq!(completion.text, "391");
    assert_eq!(completion.reasoning_text, "Let me think about 17*23.");
    assert!(completion.thinking_ms.is_some_and(|ms| ms > 0));
    assert!(
        matches!(events.first(), Some(ModelEvent::ThinkingStart { started_at_ms }) if *started_at_ms > 0)
    );
    assert_eq!(
        events
            .iter()
            .filter_map(|e| match e {
                ModelEvent::ThinkingDelta(s) => Some(s.as_str()),
                _ => None,
            })
            .collect::<String>(),
        "Let me think about 17*23."
    );
    assert!(events
        .iter()
        .any(|e| matches!(e, ModelEvent::ThinkingEnd { duration_ms } if *duration_ms > 0)));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ModelEvent::Text(_)))
            .cloned()
            .collect::<Vec<_>>(),
        vec![ModelEvent::Text("391".into())]
    );
    assert_eq!(completion.usage.input_tokens, Some(12));
    assert_eq!(completion.usage.output_tokens, Some(40));
    assert_eq!(completion.usage.reasoning_tokens, Some(38));
    assert_eq!(completion.usage.reasoning_text_chars, Some(25));
}

#[test]
fn chat_stream_that_only_reasoned_is_an_empty_answer_not_an_error() {
    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking...\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":400,\"completion_tokens_details\":{\"reasoning_tokens\":400}}}\n\n",
        "data: [DONE]\n\n"
    );
    let completion = replay(
        Protocol::ChatCompletions,
        frames,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(completion.text, "");
    assert!(completion.tool_calls.is_empty());
    assert_eq!(completion.usage.reasoning_tokens, Some(400));
    assert_eq!(completion.usage.reasoning_text_chars, Some(11));
    assert!(completion.truncated, "finish_reason length");
}

#[test]
fn responses_stream_counts_reasoning_tokens_from_usage_details() {
    let frames = concat!(
        "event: response.output_text.delta\ndata: {\"delta\":\"ok\"}\n\n",
        "event: response.completed\ndata: {\"response\":{\"usage\":{\"input_tokens\":9,\"output_tokens\":20,\"output_tokens_details\":{\"reasoning_tokens\":16}}}}\n\n"
    );
    let result = replay(
        Protocol::Responses,
        frames,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.text, "ok");
    assert_eq!(result.usage.reasoning_tokens, Some(16));
}

fn chat_frames(contents: &[&str]) -> String {
    let mut frames = contents
        .iter()
        .map(|content| {
            format!(
                "data: {}

",
                serde_json::json!({"choices": [{"delta": {"content": content}}]})
            )
        })
        .collect::<String>();
    frames.push_str(
        "data: [DONE]

",
    );
    frames
}

/// Decodes chat frames the way a live route for `model` does.
fn decode_for(model: &str, contents: &[&str]) -> (insar_agent_model::Completion, Vec<ModelEvent>) {
    let route = insar_agent_model::transport::ModelRoute::new(
        Protocol::ChatCompletions,
        "https://example.invalid/v1",
        model,
    )
    .unwrap();
    let mut events = Vec::new();
    let mut emit = |event| events.push(event);
    let cancel = CancellationToken::new();
    let mut decoder = StreamDecoder::for_route(&route);
    decoder
        .feed(&chat_frames(contents), &cancel, &mut emit)
        .unwrap();
    let completion = decoder.finish(&cancel, &mut emit).unwrap();
    (completion, events)
}

fn replay_chat(contents: &[&str]) -> (insar_agent_model::Completion, Vec<ModelEvent>) {
    decode_for("glm-5.3-flash", contents)
}

#[test]
fn chat_strips_a_leading_inline_think_block_and_counts_it_as_reasoning() {
    // Agent 2.0 Slice A: inline `<think>...</think>` emits ThinkingStart/Delta/End
    // and keeps `completion.text` clean.
    let (completion, events) = replay_chat(&["<think> 17 * 23 = 391 </think> 391"]);
    assert_eq!(completion.text, "391");
    assert_eq!(completion.reasoning_text, " 17 * 23 = 391 ");
    assert!(completion.thinking_ms.is_some_and(|ms| ms > 0));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ModelEvent::Text(_)))
            .cloned()
            .collect::<Vec<_>>(),
        vec![ModelEvent::Text("391".into())]
    );
    assert_eq!(completion.usage.reasoning_text_chars, Some(15));
}

#[test]
fn chat_strips_a_think_block_split_across_chunks() {
    let (completion, events) = replay_chat(&[
        "
<thi",
        "nk>Let me",
        " think</th",
        "ink>

",
        "Ans",
        "wer",
    ]);
    assert_eq!(completion.text, "Answer");
    assert_eq!(completion.reasoning_text, "Let me think");
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ModelEvent::Text(_)))
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            ModelEvent::Text("Ans".into()),
            ModelEvent::Text("wer".into())
        ]
    );
    assert_eq!(completion.usage.reasoning_text_chars, Some(12));
}

#[test]
fn chat_keeps_non_think_tags_and_fenced_think_examples_intact() {
    // Agent 2.0 Slice A: non-think HTML/angle-bracket text is preserved, and
    // literal `<think>` examples inside triple-backtick code fences are immune.
    for contents in [
        &["The tag ```\n<think>\n``` is literal"][..],
        &["<b>bold</b>"][..],
        &["  indented"][..],
        &["<th", "e end"][..],
    ] {
        let (completion, _) = replay_chat(contents);
        assert_eq!(completion.text, contents.concat(), "{contents:?}");
        assert_eq!(completion.usage.reasoning_text_chars, None, "{contents:?}");
        assert_eq!(completion.thinking_ms, None, "{contents:?}");
    }
}

#[test]
fn chat_unclosed_think_block_is_quarantined_and_flagged() {
    // Agent 2.0 Slice A: an unclosed `<think>` block is quarantined into
    // `reasoning_text` and flagged (`think_block_unclosed = true`), never
    // leaked into `completion.text` or `ModelEvent::Text`.
    let (completion, events) = replay_chat(&["<think>never", " closes"]);
    assert_eq!(completion.text, "");
    assert_eq!(completion.reasoning_text, "never closes");
    assert!(events.iter().all(|e| !matches!(e, ModelEvent::Text(_))));
    assert!(completion.think_block_unclosed);
    assert_eq!(completion.usage.reasoning_text_chars, Some(12));
}

#[test]
fn all_routes_and_offline_replay_strip_inline_think_blocks() {
    // Agent 2.0 Slice A: `ThinkNormalizer` is always-on across ALL models and
    // protocols (`agnes-3.0-flash`, `gpt-4o`, `deepseek-chat`, `glm-*`, local
    // proxies, and offline `replay`), removing the old `glm-*`-only restriction.
    let reply = ["<think>literal</think> answer"];
    for model in [
        "agnes-3.0-flash",
        "gpt-4o",
        "deepseek-chat",
        "qwen3-coder",
        "local-model",
        "glm-4.6",
    ] {
        let (completion, _) = decode_for(model, &reply);
        assert_eq!(completion.text, "answer", "{model}");
        assert_eq!(completion.reasoning_text, "literal", "{model}");
        assert!(!completion.think_block_unclosed, "{model}");
        assert_eq!(completion.usage.reasoning_text_chars, Some(7), "{model}");
    }
    let completion = replay(
        Protocol::ChatCompletions,
        &chat_frames(&reply),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(completion.text, "answer");
    assert_eq!(completion.reasoning_text, "literal");
}

#[test]
fn a_reply_cut_at_the_output_limit_is_marked_truncated_on_every_protocol() {
    let chat = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"length\"}]}

",
        "data: [DONE]

"
    );
    let anthropic = concat!(
        "event: content_block_start
data: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"partial\"}}

",
        "event: message_delta
data: {\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":9}}

",
        "event: message_stop
data: {}

"
    );
    let responses = concat!(
        "event: response.output_text.delta
data: {\"delta\":\"partial\"}

",
        "event: response.incomplete
data: {\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"},\"usage\":{\"input_tokens\":3,\"output_tokens\":9}}}

"
    );
    for (protocol, frames) in [
        (Protocol::ChatCompletions, chat),
        (Protocol::AnthropicMessages, anthropic),
        (Protocol::Responses, responses),
    ] {
        let completion = replay(protocol, frames, &CancellationToken::new(), |_| {}).unwrap();
        assert_eq!(completion.text, "partial", "{protocol:?}");
        assert!(completion.truncated, "{protocol:?}");
    }
    let stopped = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}

",
        "data: [DONE]

"
    );
    assert!(
        !replay(
            Protocol::ChatCompletions,
            stopped,
            &CancellationToken::new(),
            |_| {}
        )
        .unwrap()
        .truncated
    );
}

/// Decodes raw frames the way a live `glm-*` route does.
fn decode_glm_frames(frames: &str) -> (insar_agent_model::Completion, Vec<ModelEvent>) {
    let route = insar_agent_model::transport::ModelRoute::new(
        Protocol::ChatCompletions,
        "https://example.invalid/v1",
        "glm-5.3-flash",
    )
    .unwrap();
    let mut events = Vec::new();
    let mut emit = |event| events.push(event);
    let cancel = CancellationToken::new();
    let mut decoder = StreamDecoder::for_route(&route);
    decoder.feed(frames, &cancel, &mut emit).unwrap();
    let completion = decoder.finish(&cancel, &mut emit).unwrap();
    (completion, events)
}

#[test]
fn an_unclosed_think_block_is_always_quarantined_in_reasoning_with_or_without_tool_calls() {
    // Agent 2.0 Slice A: whether or not a tool call follows, an unclosed
    // `<think>` block is quarantined in `reasoning_text` and never leaks into
    // `completion.text` or `ModelEvent::Text`.
    let with_tool = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"<think>call echo\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"echo_number\",\"arguments\":\"{\\\"number\\\":4721}\"}}]}}]}\n\n",
        "data: [DONE]\n\n"
    );
    let (completion, events) = decode_glm_frames(with_tool);
    assert_eq!(completion.text, "");
    assert_eq!(completion.reasoning_text, "call echo");
    assert!(
        events.iter().all(|event| matches!(
            event,
            ModelEvent::ThinkingStart { .. }
                | ModelEvent::ThinkingDelta(_)
                | ModelEvent::ThinkingEnd { .. }
                | ModelEvent::ToolCall(_)
        )),
        "{events:?}"
    );
    assert_eq!(completion.tool_calls.len(), 1);
    assert_eq!(completion.tool_calls[0].arguments["number"], 4721);
    assert!(completion.think_block_unclosed, "the trace keeps the flag");
    assert_eq!(
        completion.usage.reasoning_text_chars,
        Some("call echo".chars().count() as u64)
    );

    let without_tool = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"<think>call echo\"}}]}\n\n",
        "data: [DONE]\n\n"
    );
    let (completion, events) = decode_glm_frames(without_tool);
    assert_eq!(completion.text, "");
    assert_eq!(completion.reasoning_text, "call echo");
    assert!(events.iter().all(|e| !matches!(e, ModelEvent::Text(_))));
    assert!(completion.tool_calls.is_empty());
    assert!(completion.think_block_unclosed);
    assert_eq!(
        completion.usage.reasoning_text_chars,
        Some("call echo".chars().count() as u64)
    );
}

#[test]
fn prompt_cache_tokens_are_extracted_across_all_three_protocols() {
    let openai_chat = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":2000,\"completion_tokens\":50,\"prompt_tokens_details\":{\"cached_tokens\":1500}}}\n\n",
        "data: [DONE]\n\n"
    );
    let c1 = replay(
        Protocol::ChatCompletions,
        openai_chat,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(c1.usage.input_tokens, Some(2000));
    assert_eq!(c1.usage.output_tokens, Some(50));
    assert_eq!(c1.usage.cached_input_tokens, Some(1500));

    let deepseek_chat = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3000,\"completion_tokens\":80,\"prompt_cache_hit_tokens\":2400}}\n\n",
        "data: [DONE]\n\n"
    );
    let c2 = replay(
        Protocol::ChatCompletions,
        deepseek_chat,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(c2.usage.input_tokens, Some(3000));
    assert_eq!(c2.usage.cached_input_tokens, Some(2400));

    let responses = concat!(
        "event: response.output_text.delta\ndata: {\"delta\":\"ok\"}\n\n",
        "event: response.completed\ndata: {\"response\":{\"usage\":{\"input_tokens\":5000,\"output_tokens\":120,\"input_tokens_details\":{\"cached_tokens\":4000}}}}\n\n"
    );
    let c3 = replay(
        Protocol::Responses,
        responses,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(c3.usage.input_tokens, Some(5000));
    assert_eq!(c3.usage.output_tokens, Some(120));
    assert_eq!(c3.usage.cached_input_tokens, Some(4000));

    let anthropic = concat!(
        "event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":400,\"cache_read_input_tokens\":1600,\"cache_creation_input_tokens\":200}}}\n\n",
        "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"ok\"}}\n\n",
        "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":60}}\n\n",
        "event: message_stop\ndata: {}\n\n"
    );
    let c4 = replay(
        Protocol::AnthropicMessages,
        anthropic,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(c4.usage.input_tokens, Some(400));
    assert_eq!(c4.usage.output_tokens, Some(60));
    assert_eq!(c4.usage.cached_input_tokens, Some(1600));
    assert_eq!(c4.usage.cache_creation_input_tokens, Some(200));
}

#[test]
fn google_gemini_sse_parses_thoughts_text_function_calls_and_usage() {
    let frames = concat!(
        "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"thought\":true,\"text\":\"Thinking...\"}]}}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Done\"},{\"functionCall\":{\"name\":\"tool_a\",\"args\":{\"x\":1}}}]}}]}\n\n",
        "data: {\"candidates\":[{\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":20,\"candidatesTokenCount\":10,\"cachedContentTokenCount\":5,\"thoughtsTokenCount\":7}}\n\n"
    );
    let completion = replay(
        Protocol::GoogleGemini,
        frames,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(completion.reasoning_text, "Thinking...");
    assert_eq!(completion.text, "Done");
    assert_eq!(completion.tool_calls.len(), 1);
    assert_eq!(completion.tool_calls[0].name, "tool_a");
    assert_eq!(completion.tool_calls[0].arguments["x"], 1);
    assert_eq!(completion.usage.input_tokens, Some(20));
    assert_eq!(completion.usage.output_tokens, Some(10));
    assert_eq!(completion.usage.cached_input_tokens, Some(5));
    assert_eq!(completion.usage.reasoning_tokens, Some(7));
}

#[test]
fn chat_eof_with_payload_or_finish_reason_completes_without_done_but_empty_eof_errors() {
    let without_done =
        "data: {\"choices\":[{\"delta\":{\"content\":\"pong\"},\"finish_reason\":\"stop\"}]}\n\n";
    let ok = replay(
        Protocol::ChatCompletions,
        without_done,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(ok.text, "pong");

    let empty_err = replay(
        Protocol::ChatCompletions,
        "",
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap_err();
    assert_eq!(empty_err.kind, ErrorKind::Transient);
}

#[test]
fn sse_diagnostics_preserve_sanitized_upstream_detail_handle_keepalives_and_type_fallback() {
    // 1. Responses SSE frame without `event:` header falls back to `value["type"]`
    //    and accurately extracts `cached_tokens` + flushes trailing data without `\n\n`.
    let no_event_header = concat!(
        "data: ping\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":5000,\"output_tokens\":40,\"input_tokens_details\":{\"cached_tokens\":4200}}}}"
    );
    let c = replay(
        Protocol::Responses,
        no_event_header,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(c.text, "你好");
    assert_eq!(c.usage.input_tokens, Some(5000));
    assert_eq!(c.usage.cached_input_tokens, Some(4200));

    // 2. `"error": null` in a normal Chat chunk MUST NOT trigger a false error,
    //    and a trailing partial usage chunk MUST NOT erase earlier `cached_input_tokens`.
    let null_err_and_usage = concat!(
        "data: {\"error\":null,\"choices\":[{\"delta\":{\"content\":\"OK\"}}],\"usage\":{\"prompt_tokens\":1000,\"prompt_tokens_details\":{\"cached_tokens\":800}}}\n\n",
        "data: {\"error\":null,\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1000,\"completion_tokens\":25}}\n\n",
        "data: [DONE]\n\n"
    );
    let c2 = replay(
        Protocol::ChatCompletions,
        null_err_and_usage,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(c2.text, "OK");
    assert_eq!(c2.usage.cached_input_tokens, Some(800));
    assert_eq!(c2.usage.output_tokens, Some(25));

    // 3. SSE upstream error preserves sanitized upstream message in sanitized_summary_zh
    //    while scrubbing secrets and keeping Debug output secret-free.
    let sse_err = replay(
        Protocol::Responses,
        "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_error\",\"message\":\"Upstream overloaded for key sk-proj-ABC1234567890xyz\"}}}\n\n",
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap_err();
    let summary = sse_err.sanitized_summary_zh();
    assert!(summary.contains("Upstream overloaded"), "{summary}");
    assert!(!summary.contains("sk-proj-ABC1234567890xyz"), "{summary}");
    assert!(!format!("{sse_err:?}").contains("sk-proj-ABC1234567890xyz"));
    assert!(!summary.contains("返回未知错误"), "{summary}");
}
