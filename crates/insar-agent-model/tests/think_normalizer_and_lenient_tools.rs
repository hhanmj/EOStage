//! Comprehensive Slice A unit tests for Always-On `ThinkNormalizer`,
//! 6-shape reasoning extraction, 12-byte chunk-boundary lookahead,
//! orphan `</think>` recovery, unclosed `<think>` quarantine,
//! triple-backtick code-fence immunity, monotonic `thinking_ms`,
//! lenient `tool_calls` JSON repair, and `<tool_call>` XML fallback.

use std::time::Duration;

use insar_agent_model::{
    agent::{parse_answer, without_think},
    replay,
    transport::ModelRoute,
    Completion, ModelEvent, Protocol, StreamDecoder, ToolCall,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn chat_chunk(delta: serde_json::Value) -> String {
    format!("data: {}\n\n", json!({"choices": [{"delta": delta}]}))
}

fn decode_chat_chunks(model: &str, chunks: &[serde_json::Value]) -> (Completion, Vec<ModelEvent>) {
    let route = ModelRoute::new(
        Protocol::ChatCompletions,
        "https://example.invalid/v1",
        model,
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let mut decoder = StreamDecoder::for_route(&route);
    let mut events = Vec::new();
    for chunk in chunks {
        let frame = chat_chunk(chunk.clone());
        decoder
            .feed(&frame, &cancel, &mut |e| events.push(e))
            .unwrap();
    }
    decoder
        .feed("data: [DONE]\n\n", &cancel, &mut |e| events.push(e))
        .unwrap();
    let completion = decoder.finish(&cancel, &mut |e| events.push(e)).unwrap();
    (completion, events)
}

#[test]
fn agnes_3_0_flash_chunk_split_think_and_thinking_tags_never_leak() {
    // Splits `<think>`, `</think>`, `<thinking>`, and `</thinking>` across
    // arbitrary 1-to-5 byte chunk boundaries on `agnes-3.0-flash`.
    let chunks = [
        json!({"content": "  \n<t"}),
        json!({"content": "hi"}),
        json!({"content": "nk>"}),
        json!({"content": "用户想切换到"}),
        json!({"content": "四川省（510000）"}),
        json!({"content": "</t"}),
        json!({"content": "hink"}),
        json!({"content": ">\n\n"}),
        json!({"content": "### 研究区已切换\n\n已将当前研究区更新为 **四川省**。"}),
    ];
    let (completion, events) = decode_chat_chunks("agnes-3.0-flash", &chunks);

    assert_eq!(
        completion.text,
        "### 研究区已切换\n\n已将当前研究区更新为 **四川省**。"
    );
    assert_eq!(completion.reasoning_text, "用户想切换到四川省（510000）");
    assert!(completion.thinking_ms.is_some_and(|ms| ms > 0));
    assert!(!completion.think_block_unclosed);
    assert!(!completion.orphan_think_reclassified);

    for event in &events {
        if let ModelEvent::Text(t) = event {
            assert!(!t.contains("<think"), "leaked opening tag fragment: {t}");
            assert!(!t.contains("</think"), "leaked closing tag fragment: {t}");
            assert!(!t.contains("510000"), "leaked reasoning body: {t}");
        }
    }

    // Verify `<thinking>...</thinking>` case-insensitive variant as well.
    let thinking_chunks = [
        json!({"content": "<THIN"}),
        json!({"content": "KING>step 1: check S1 VV+VH</THINK"}),
        json!({"content": "ING>\n检索完成。"}),
    ];
    let (c2, _) = decode_chat_chunks("agnes-3.0-flash", &thinking_chunks);
    assert_eq!(c2.text, "检索完成。");
    assert_eq!(c2.reasoning_text, "step 1: check S1 VV+VH");
    assert!(c2.thinking_ms.is_some_and(|ms| ms > 0));
}

#[test]
fn six_dedicated_sse_reasoning_shapes_extract_without_double_counting() {
    // 1) Chat `reasoning_content` + aliased `reasoning` on same delta (OpenRouter style)
    let (c_openrouter, _) = decode_chat_chunks(
        "deepseek/deepseek-r1",
        &[
            json!({"reasoning_content": "先查四川", "reasoning": "先查四川"}),
            json!({"thought": "再查哨兵1号"}),
            json!({"thinking": "极化选 VV+VH"}),
            json!({"content": "共 24 景。"}),
        ],
    );
    assert_eq!(c_openrouter.text, "共 24 景。");
    assert_eq!(
        c_openrouter.reasoning_text,
        "先查四川再查哨兵1号极化选 VV+VH"
    );
    assert!(c_openrouter.thinking_ms.is_some_and(|ms| ms > 0));

    // 2) OpenAI Responses `response.reasoning_summary_text.delta` & `response.reasoning_text.delta`
    let responses_frames = concat!(
        "event: response.reasoning_summary_text.delta\ndata: {\"delta\":\"Check AOI \"}\n\n",
        "event: response.reasoning_text.delta\ndata: {\"delta\":\"Sichuan.\"}\n\n",
        "event: response.output_text.delta\ndata: {\"delta\":\"Done.\"}\n\n",
        "event: response.completed\ndata: {\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":8}}}\n\n"
    );
    let mut r_events = Vec::new();
    let c_responses = replay(
        Protocol::Responses,
        responses_frames,
        &CancellationToken::new(),
        |e| r_events.push(e),
    )
    .unwrap();
    assert_eq!(c_responses.text, "Done.");
    assert_eq!(c_responses.reasoning_text, "Check AOI Sichuan.");
    assert!(c_responses.thinking_ms.is_some_and(|ms| ms > 0));

    // 3) Anthropic `thinking_delta`
    let anthropic_frames = concat!(
        "event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":10}}}\n\n",
        "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\" deliberating...\"}}\n\n",
        "event: content_block_start\ndata: {\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"Result ready.\"}}\n\n",
        "event: message_stop\ndata: {}\n\n"
    );
    let c_anthropic = replay(
        Protocol::AnthropicMessages,
        anthropic_frames,
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(c_anthropic.text, "Result ready.");
    assert_eq!(c_anthropic.reasoning_text, " deliberating...");
    assert!(c_anthropic.thinking_ms.is_some_and(|ms| ms > 0));
}

#[test]
fn non_thinking_turn_emits_zero_thinking_events_and_none_timer() {
    let (completion, events) = decode_chat_chunks(
        "agnes-3.0-flash",
        &[
            json!({"role": "assistant", "content": "", "reasoning_content": ""}),
            json!({"content": "正常回复，没有思考过程。"}),
        ],
    );
    assert_eq!(completion.text, "正常回复，没有思考过程。");
    assert_eq!(completion.reasoning_text, "");
    assert_eq!(completion.thinking_ms, None);
    assert_eq!(completion.usage.reasoning_text_chars, None);
    assert!(events.iter().all(|e| matches!(e, ModelEvent::Text(_))));
}

#[test]
fn orphan_closing_think_within_8192_bytes_reclassifies_prefix_into_reasoning() {
    // Simulates vLLM / DeepSeek / agnes server chat template pre-filling `<think>\n`
    // so the stream begins directly with reasoning prose followed by split `</think>`.
    let (completion, events) = decode_chat_chunks(
        "agnes-3.0-flash",
        &[
            json!({"content": "首先分析用户意图：需要将西藏切换为四川。"}),
            json!({"content": "然后调用 admin_boundary_search。</thi"}),
            json!({"content": "nk>\n\n### 操作完成\n已切换至四川省。"}),
        ],
    );
    assert!(completion.orphan_think_reclassified);
    assert!(!completion.think_block_unclosed);
    assert_eq!(completion.text, "### 操作完成\n已切换至四川省。");
    assert_eq!(
        completion.reasoning_text,
        "首先分析用户意图：需要将西藏切换为四川。然后调用 admin_boundary_search。"
    );
    assert!(completion.thinking_ms.is_some_and(|ms| ms > 0));
    assert!(events
        .iter()
        .any(|e| matches!(e, ModelEvent::ThinkingStart { .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, ModelEvent::ThinkingEnd { .. })));

    // Beyond 8192 bytes, a stray `</think>` is NOT treated as an orphan header.
    let big_prefix = "A".repeat(8250);
    let (c_beyond, _) = decode_chat_chunks(
        "agnes-3.0-flash",
        &[
            json!({"content": big_prefix}),
            json!({"content": "</think> tail"}),
        ],
    );
    assert!(!c_beyond.orphan_think_reclassified);
    assert_eq!(c_beyond.reasoning_text, "");
    assert!(c_beyond.text.ends_with("</think> tail"));
}

#[test]
fn unclosed_think_quarantine_never_leaks_into_completion_text() {
    // Leading unclosed `<think>`
    let (c1, e1) = decode_chat_chunks(
        "agnes-3.0-flash",
        &[
            json!({"content": "<think>正在思考四川省边界坐标"}),
            json!({"content": "，但输出因 max_tokens 中断"}),
        ],
    );
    assert_eq!(c1.text, "");
    assert_eq!(
        c1.reasoning_text,
        "正在思考四川省边界坐标，但输出因 max_tokens 中断"
    );
    assert!(c1.think_block_unclosed);
    assert!(c1.thinking_ms.is_some_and(|ms| ms > 0));
    assert!(e1.iter().all(|e| !matches!(e, ModelEvent::Text(_))));

    // Mid-stream unclosed `<think>` after initial prose preserves prose before `<think>`
    // while quarantining the unclosed thought tail.
    let (c2, _) = decode_chat_chunks(
        "agnes-3.0-flash",
        &[json!({"content": "先给初步说明。\n<think>再补充思考但未闭合"})],
    );
    assert_eq!(c2.text, "先给初步说明。\n");
    assert_eq!(c2.reasoning_text, "再补充思考但未闭合");
    assert!(c2.think_block_unclosed);
}

#[test]
fn triple_backtick_code_fence_protects_literal_think_and_orphan_close_tags() {
    // Even when triple backticks and `<think>` tags split across chunk boundaries!
    let (completion, _) = decode_chat_chunks(
        "agnes-3.0-flash",
        &[
            json!({"content": "下面是标签示例：\n``"}),
            json!({"content": "`xml\n<thi"}),
            json!({"content": "nk>\n示例思考内容\n</thi"}),
            json!({"content": "nk>\n``"}),
            json!({"content": "`\n以上代码块应原样保留。"}),
        ],
    );
    let expected =
        "下面是标签示例：\n```xml\n<think>\n示例思考内容\n</think>\n```\n以上代码块应原样保留。";
    assert_eq!(completion.text, expected);
    assert_eq!(completion.reasoning_text, "");
    assert_eq!(completion.thinking_ms, None);
    assert!(!completion.orphan_think_reclassified);
    assert!(!completion.think_block_unclosed);
    assert_eq!(without_think(expected), expected);
}

#[test]
fn monotonic_thinking_ms_measures_real_elapsed_deliberation_time() {
    let route = ModelRoute::new(
        Protocol::ChatCompletions,
        "https://example.invalid/v1",
        "agnes-3.0-flash",
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let mut decoder = StreamDecoder::for_route(&route);
    let mut events = Vec::new();

    decoder
        .feed(
            &chat_chunk(json!({"content": "<think>Step 1..."})),
            &cancel,
            &mut |e| events.push(e),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(25));
    decoder
        .feed(
            &chat_chunk(json!({"content": " Step 2</think>\nFinal answer."})),
            &cancel,
            &mut |e| events.push(e),
        )
        .unwrap();
    // Sleep AFTER `</think>` to prove `thinking_ms` stopped when thinking ended!
    std::thread::sleep(Duration::from_millis(20));
    decoder
        .feed("data: [DONE]\n\n", &cancel, &mut |e| events.push(e))
        .unwrap();
    let completion = decoder.finish(&cancel, &mut |e| events.push(e)).unwrap();

    let ms = completion.thinking_ms.expect("thinking_ms recorded");
    assert!(ms >= 20, "expected >= 20ms, got {ms}ms");
    assert_eq!(completion.text, "Final answer.");
    assert_eq!(completion.reasoning_text, "Step 1... Step 2");

    let end_ms = events
        .iter()
        .find_map(|e| match e {
            ModelEvent::ThinkingEnd { duration_ms } => Some(*duration_ms),
            _ => None,
        })
        .expect("ThinkingEnd emitted");
    assert_eq!(ms, end_ms);
}

#[test]
fn lenient_tool_calls_repairs_fences_trailing_commas_and_bad_ids() {
    let route = ModelRoute::new(
        Protocol::ChatCompletions,
        "https://example.invalid/v1",
        "ollama/qwen2.5",
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let mut decoder = StreamDecoder::for_route(&route).with_step(2);
    let mut events = Vec::new();

    let frames = concat!(
        // Tool 0: empty id + ```json fenced arguments with trailing commas and `,}` inside a string literal
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"\",\"function\":{\"name\":\"working_aoi_set\",\"arguments\":\"```json\\n{\\\"admin_code\\\": \\\"510000\\\", \\\"display_name\\\": \\\"四川省,}\\\",}\\n```\"}}]}}]}\n\n",
        // Tool 1: invalid UUID-with-colons id + empty arguments string
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"urn:uuid:abc:123\",\"function\":{\"name\":\"working_aoi_get\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: [DONE]\n\n"
    );
    decoder
        .feed(frames, &cancel, &mut |e| events.push(e))
        .unwrap();
    let completion = decoder.finish(&cancel, &mut |e| events.push(e)).unwrap();

    assert_eq!(completion.tool_calls.len(), 2);
    assert_eq!(completion.tool_calls[0].id, "call_2_0");
    assert_eq!(completion.tool_calls[0].name, "working_aoi_set");
    assert_eq!(completion.tool_calls[0].arguments["admin_code"], "510000");
    assert_eq!(
        completion.tool_calls[0].arguments["display_name"], "四川省,}",
        "comma-brace inside a JSON string literal must not be corrupted"
    );

    assert_eq!(completion.tool_calls[1].id, "call_2_1");
    assert_eq!(completion.tool_calls[1].name, "working_aoi_get");
    assert_eq!(completion.tool_calls[1].arguments, json!({}));
}

#[test]
fn xml_tool_call_fallback_extracts_calls_and_strips_tags_when_tool_array_empty() {
    let (completion, events) = decode_chat_chunks(
        "qwen2.5-coder",
        &[
            json!({"content": "<think>需要先查四川省编码</think>\n正在查询行政区：\n<tool_call>\n{\"name\": \"admin_boundary_search\", \"arguments\": {\"query\": \"四川省\",}}\n</tool_call>"}),
        ],
    );
    assert_eq!(completion.reasoning_text, "需要先查四川省编码");
    assert_eq!(completion.text, "正在查询行政区：");
    assert_eq!(
        completion.tool_calls,
        vec![ToolCall {
            id: "call_0_0".into(),
            name: "admin_boundary_search".into(),
            arguments: json!({"query": "四川省"}),
        }]
    );
    assert!(events.iter().any(|e| matches!(
        e,
        ModelEvent::ToolCall(tc) if tc.name == "admin_boundary_search" && tc.id == "call_0_0"
    )));
}

#[test]
fn parse_answer_handles_markdown_and_json_shapes_harmoniously() {
    // 1) Clean Markdown answer
    let md = parse_answer("### 结论\n\n- **区域**：四川省\n- **极化**：`VV+VH`");
    assert!(!md.structured);
    assert_eq!(
        md.answer.text,
        "### 结论\n\n- **区域**：四川省\n- **极化**：`VV+VH`"
    );
    assert!(md.answer.claims.is_empty());

    // 2) JSON wrapped in ```json fences
    let fenced = parse_answer(
        "```json\n{\"text\": \"已找到 **10 景** 数据\", \"claims\": [{\"statement\": \"10 景\", \"refs\": [\"batch:b9\"]}]}\n```",
    );
    assert!(fenced.structured);
    assert_eq!(fenced.answer.text, "已找到 **10 景** 数据");
    assert_eq!(fenced.answer.claims.len(), 1);
    assert_eq!(fenced.answer.claims[0].refs, vec!["batch:b9"]);
}

#[test]
fn agnes_parenthetical_chinese_thinking_block_with_nested_parens_routes_to_reasoning() {
    let chunks = [
        json!({"content": "（思"}),
        json!({"content": "考：用户消息附带了四川省成都市锦江区的区域（context_aoi 即此区域）。\n先调用 admin_boundary_search。\n）"}),
        json!({"content": "\n\n已完成成都市锦江区检索，共找到 **5 景** 影像。"}),
    ];
    let (completion, events) = decode_chat_chunks("agnes-3.0-flash", &chunks);
    assert_eq!(
        completion.text,
        "已完成成都市锦江区检索，共找到 **5 景** 影像。"
    );
    assert_eq!(
        completion.reasoning_text,
        "用户消息附带了四川省成都市锦江区的区域（context_aoi 即此区域）。\n先调用 admin_boundary_search。\n"
    );
    assert!(!completion.think_block_unclosed);
    for event in &events {
        if let ModelEvent::Text(t) = event {
            assert!(!t.contains("思考："), "leaked parenthetical thought: {t}");
            assert!(!t.contains("context_aoi"), "leaked internal reasoning: {t}");
        }
    }
}
