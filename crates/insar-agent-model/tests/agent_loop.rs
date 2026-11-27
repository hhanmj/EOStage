use std::{future::Future, pin::Pin};

use insar_agent_model::{
    agent::{AgentLoop, Checkpoint, Dispatcher, Model, StructuredAnswer},
    Completion, ErrorKind, Message, ModelError, ModelEvent, ToolCall, ToolDescription,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

struct FakeModel;
impl Model for FakeModel {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        _tools: &'a [ToolDescription],
        _cancel: &'a CancellationToken,
        _emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            if messages.len() == 1 {
                Ok(Completion {
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "read_catalog".into(),
                        arguments: json!({"q": "DEM"}),
                    }],
                    ..Completion::default()
                })
            } else {
                Ok(Completion { text: r#"{\"text\":\"Found DEM\",\"claims\":[{\"statement\":\"DEM listed\",\"refs\":[\"tool_result:1\"]}]}"#.replace("\\\"", "\""), ..Completion::default() })
            }
        })
    }
    /// In-process fake: nothing goes over the network.
    fn request_body(
        &self,
        _messages: &[Message],
        _tools: &[ToolDescription],
    ) -> Option<Result<Value, ModelError>> {
        None
    }
}
struct FakeExecutor;
impl Dispatcher for FakeExecutor {
    fn execute<'a>(
        &'a self,
        call: &'a ToolCall,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            assert_eq!(call.name, "read_catalog");
            Ok(json!({"batch": "fixture"}))
        })
    }
}

#[tokio::test]
async fn fake_executor_two_turns_produces_structured_answer_with_checkpoint() {
    let agent = AgentLoop { max_steps: 3 };
    let initial = Checkpoint::new(vec![Message {
        role: "user".into(),
        content: "DEM".into(),
    }]);
    let mut seen = Vec::new();
    let result = agent
        .run(
            &FakeModel,
            &FakeExecutor,
            &[ToolDescription {
                name: "read_catalog".into(),
                description: "read only".into(),
                parameters: json!({"type":"object"}),
            }],
            initial,
            &CancellationToken::new(),
            |event| seen.push(event),
        )
        .await
        .unwrap();
    assert_eq!(
        result.answer,
        StructuredAnswer {
            text: "Found DEM".into(),
            claims: vec![insar_agent_model::agent::Claim {
                statement: "DEM listed".into(),
                refs: vec!["tool_result:1".into()]
            }]
        }
    );
    assert_eq!(result.checkpoint.steps, 2);
    assert_eq!(result.checkpoint.tool_results.len(), 1);
    assert_eq!(result.checkpoint.messages[1].role, "assistant_tool_calls");
    assert_eq!(result.checkpoint.messages[2].role, "tool");
    assert_eq!(
        serde_json::from_str::<Value>(&result.checkpoint.messages[2].content).unwrap()["id"],
        "call-1"
    );
    assert!(seen.is_empty()); // FakeModel does not emit streaming events.
}

#[tokio::test]
async fn a_replayed_tool_result_must_not_be_run_again() {
    let agent = AgentLoop { max_steps: 3 };
    let mut checkpoint = Checkpoint::new(vec![Message {
        role: "user".into(),
        content: "DEM".into(),
    }]);
    checkpoint.steps = 1;
    checkpoint.messages.push(Message {
        role: "assistant_tool_calls".into(),
        content:
            json!({"text":"", "tool_calls":[{"id":"call-1","name":"read_catalog","arguments":{}}]})
                .to_string(),
    });
    checkpoint.messages.push(Message {
        role: "tool".into(),
        content: json!({"id":"call-1","result":{"batch":"fixture"}}).to_string(),
    });
    checkpoint.tool_results.push(json!({"batch":"fixture"}));
    let result = agent
        .run(
            &FakeModel,
            &FakeExecutor,
            &[],
            checkpoint,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(result.checkpoint.tool_results.len(), 1);
}

#[tokio::test]
async fn a_failed_executor_returns_recoverable_pending_call_in_error() {
    struct Uncertain;
    impl Dispatcher for Uncertain {
        fn execute<'a>(
            &'a self,
            _call: &'a ToolCall,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
            Box::pin(async { Err(ModelError::new(ErrorKind::Transient, None)) })
        }
    }
    let agent = AgentLoop { max_steps: 3 };
    let tools = [ToolDescription {
        name: "read_catalog".into(),
        description: "read only".into(),
        parameters: json!({"type":"object"}),
    }];
    let start = Checkpoint::new(vec![Message {
        role: "user".into(),
        content: "DEM".into(),
    }]);
    let failed = agent
        .run(
            &FakeModel,
            &Uncertain,
            &tools,
            start,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(failed.kind, ErrorKind::Transient);
    assert_eq!(
        failed.checkpoint.unwrap().pending_call.unwrap().id,
        "call-1"
    );
}

#[tokio::test]
async fn step_limit_does_not_replay_prior_calls_on_resume() {
    let agent = AgentLoop { max_steps: 1 };
    let checkpoint = Checkpoint::new(vec![Message {
        role: "user".into(),
        content: "DEM".into(),
    }]);
    let tools = [ToolDescription {
        name: "read_catalog".into(),
        description: "read only".into(),
        parameters: json!({"type":"object"}),
    }];
    let error = agent
        .run(
            &FakeModel,
            &FakeExecutor,
            &tools,
            checkpoint,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    // L4 r7: the step budget is its own error, not a provider capability
    // (the tool is offered, so the one step is spent on it).
    assert_eq!(error.kind, ErrorKind::StepLimit);
}

/// A model that answers every turn with the same tool call.
struct SameCall(&'static str);
impl Model for SameCall {
    fn complete<'a>(
        &'a self,
        _messages: &'a [Message],
        _tools: &'a [ToolDescription],
        _cancel: &'a CancellationToken,
        _emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ModelError>> + Send + 'a>> {
        let name = self.0;
        Box::pin(async move {
            Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: name.into(),
                    arguments: json!({"q": "DEM"}),
                }],
                ..Completion::default()
            })
        })
    }
    fn request_body(
        &self,
        _messages: &[Message],
        _tools: &[ToolDescription],
    ) -> Option<Result<Value, ModelError>> {
        None
    }
}

#[tokio::test]
async fn a_repeated_or_unoffered_tool_call_has_its_own_error_kind() {
    let tools = [ToolDescription {
        name: "read_catalog".into(),
        description: "read only".into(),
        parameters: json!({"type":"object"}),
    }];
    let start = || {
        Checkpoint::new(vec![Message {
            role: "user".into(),
            content: "DEM".into(),
        }])
    };
    let agent = AgentLoop { max_steps: 5 };
    let repeated = agent
        .run(
            &SameCall("read_catalog"),
            &FakeExecutor,
            &tools,
            start(),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(repeated.kind, ErrorKind::RepeatedCall);
    let unoffered = agent
        .run(
            &SameCall("shell_exec"),
            &FakeExecutor,
            &tools,
            start(),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(unoffered.kind, ErrorKind::ToolNotOffered);
}

#[tokio::test]
async fn top_level_plan_step_calls_coalesce_into_propose_plan_when_offered() {
    use std::sync::{Arc, Mutex};

    struct TwoStepPlanModel;
    impl Model for TwoStepPlanModel {
        fn complete<'a>(
            &'a self,
            messages: &'a [Message],
            _tools: &'a [ToolDescription],
            _cancel: &'a CancellationToken,
            _emit: &'a mut (dyn FnMut(ModelEvent) + Send),
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Completion, ModelError>> + Send + 'a>,
        > {
            let saw_tool = messages.iter().any(|m| m.role == "tool");
            Box::pin(async move {
                if !saw_tool {
                    Ok(Completion {
                        tool_calls: vec![
                            ToolCall {
                                id: "call-aoi".into(),
                                name: "working_aoi_set".into(),
                                arguments: json!({"admin_code": "510000"}),
                            },
                            ToolCall {
                                id: "call-search".into(),
                                name: "search_start".into(),
                                arguments: json!({"resource_id": "science.sentinel1.c.slc.grd.v1"}),
                            },
                        ],
                        ..Completion::default()
                    })
                } else {
                    Ok(Completion {
                        text: r#"{"text":"已完成四川省检索","claims":[]}"#.into(),
                        ..Completion::default()
                    })
                }
            })
        }
        fn request_body(
            &self,
            _messages: &[Message],
            _tools: &[ToolDescription],
        ) -> Option<Result<Value, ModelError>> {
            None
        }
    }

    struct RecordingDispatcher(Arc<Mutex<Vec<ToolCall>>>);
    impl Dispatcher for RecordingDispatcher {
        fn execute<'a>(
            &'a self,
            call: &'a ToolCall,
            _cancel: &'a CancellationToken,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, ModelError>> + Send + 'a>,
        > {
            self.0.lock().unwrap().push(call.clone());
            Box::pin(async move { Ok(json!({"status": "executed"})) })
        }
    }

    let tools = [ToolDescription {
        name: "propose_plan".into(),
        description: "Propose plan".into(),
        parameters: json!({"type":"object"}),
    }];
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let agent = AgentLoop { max_steps: 4 };
    let res = agent
        .run(
            &TwoStepPlanModel,
            &RecordingDispatcher(Arc::clone(&recorded)),
            &tools,
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "检索四川省".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .expect("coalesced propose_plan succeeds");
    assert_eq!(res.answer.text, "已完成四川省检索");

    {
        let calls = recorded.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "propose_plan");
        assert_eq!(calls[0].id, "call-aoi");
        assert_eq!(
            calls[0].arguments,
            json!({
                "steps": [
                    {"tool_id": "working_aoi_set", "input": {"admin_code": "510000"}},
                    {"tool_id": "search_start", "input": {"resource_id": "science.sentinel1.c.slc.grd.v1"}},
                ]
            })
        );
    }

    // Unoffered rogue tool still fails closed even when propose_plan is in tools
    let rogue = agent
        .run(
            &SameCall("shell_exec"),
            &FakeExecutor,
            &tools,
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "rogue".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(rogue.kind, ErrorKind::ToolNotOffered);
}

#[tokio::test]
async fn consecutive_search_progress_polls_do_not_trigger_repeated_call() {
    use std::sync::{Arc, Mutex};

    struct TwoPollSearchModel;
    impl Model for TwoPollSearchModel {
        fn complete<'a>(
            &'a self,
            messages: &'a [Message],
            _: &'a [ToolDescription],
            _: &'a CancellationToken,
            _: &'a mut (dyn FnMut(ModelEvent) + Send),
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Completion, ModelError>> + Send + 'a>,
        > {
            let tool_results = messages.iter().filter(|m| m.role == "tool").count();
            Box::pin(async move {
                if tool_results < 2 {
                    Ok(Completion {
                        tool_calls: vec![ToolCall {
                            id: format!("poll-{tool_results}"),
                            name: "search_progress".into(),
                            arguments: json!({"request_id": "plan-live-1"}),
                        }],
                        ..Completion::default()
                    })
                } else {
                    Ok(Completion {
                        text: r#"{"text":"检索已完成，共命中 250 景","claims":[]}"#.into(),
                        ..Completion::default()
                    })
                }
            })
        }
        fn request_body(
            &self,
            _: &[Message],
            _: &[ToolDescription],
        ) -> Option<Result<Value, ModelError>> {
            None
        }
    }

    struct PollDispatcher(Arc<Mutex<usize>>);
    impl Dispatcher for PollDispatcher {
        fn execute<'a>(
            &'a self,
            _: &'a ToolCall,
            _: &'a CancellationToken,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, ModelError>> + Send + 'a>,
        > {
            let mut count = self.0.lock().unwrap();
            *count += 1;
            let current = *count;
            Box::pin(async move {
                if current == 1 {
                    Ok(json!({"stage": "requesting", "terminal": false}))
                } else {
                    Ok(json!({"stage": "done", "terminal": true, "items_kept": 250}))
                }
            })
        }
    }

    let tools = [ToolDescription {
        name: "search_progress".into(),
        description: "Poll search progress".into(),
        parameters: json!({"type": "object"}),
    }];
    let count = Arc::new(Mutex::new(0));
    let agent = AgentLoop { max_steps: 5 };
    let res = agent
        .run(
            &TwoPollSearchModel,
            &PollDispatcher(Arc::clone(&count)),
            &tools,
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "查四川省".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .expect("consecutive search_progress polls succeed without RepeatedCall");
    assert_eq!(res.answer.text, "检索已完成，共命中 250 景");
    assert_eq!(*count.lock().unwrap(), 2);
}

#[tokio::test]
async fn multi_tool_use_parallel_and_prefixed_calls_normalize_and_unoffered_delegate() {
    use std::sync::{Arc, Mutex};

    struct ProxyQuirkModel;
    impl Model for ProxyQuirkModel {
        fn complete<'a>(
            &'a self,
            messages: &'a [Message],
            _: &'a [ToolDescription],
            _: &'a CancellationToken,
            _: &'a mut (dyn FnMut(ModelEvent) + Send),
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Completion, ModelError>> + Send + 'a>,
        > {
            let tool_turns = messages.iter().filter(|m| m.role == "tool").count();
            Box::pin(async move {
                match tool_turns {
                    0 => Ok(Completion {
                        tool_calls: vec![ToolCall {
                            id: "hallucinated-1".into(),
                            name: "search_sentinel1".into(),
                            arguments: json!({"region": "四川省"}),
                        }],
                        ..Completion::default()
                    }),
                    1 => Ok(Completion {
                        tool_calls: vec![ToolCall {
                            id: "parallel-1".into(),
                            name: "multi_tool_use.parallel".into(),
                            arguments: json!({
                                "tool_uses": [
                                    {
                                        "recipient_name": "functions.working_aoi_set",
                                        "parameters": {"admin_code": "510000"}
                                    },
                                    {
                                        "recipient_name": "default_api:search_start",
                                        "parameters": {"input": {"resource_id": "science.sentinel-1.standard.v1"}}
                                    }
                                ]
                            }),
                        }],
                        ..Completion::default()
                    }),
                    _ => Ok(Completion {
                        text: r#"{"text":"已自纠并完成计划执行","claims":[]}"#.into(),
                        ..Completion::default()
                    }),
                }
            })
        }
        fn request_body(
            &self,
            _: &[Message],
            _: &[ToolDescription],
        ) -> Option<Result<Value, ModelError>> {
            None
        }
    }

    struct TolerantDispatcher(Arc<Mutex<Vec<ToolCall>>>);
    impl Dispatcher for TolerantDispatcher {
        fn execute<'a>(
            &'a self,
            call: &'a ToolCall,
            _: &'a CancellationToken,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, ModelError>> + Send + 'a>,
        > {
            self.0.lock().unwrap().push(call.clone());
            let name = call.name.clone();
            Box::pin(async move {
                if name != "propose_plan" {
                    Ok(json!({"code": "AGENT_TOOL_FORBIDDEN", "reason": "tool_unknown"}))
                } else {
                    Ok(json!({"status": "completed"}))
                }
            })
        }
        fn accepts_unoffered_calls(&self) -> bool {
            true
        }
    }

    let tools = [ToolDescription {
        name: "propose_plan".into(),
        description: "Propose plan".into(),
        parameters: json!({"type": "object"}),
    }];
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let agent = AgentLoop { max_steps: 5 };
    let res = agent
        .run(
            &ProxyQuirkModel,
            &TolerantDispatcher(Arc::clone(&recorded)),
            &tools,
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "检索四川省".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .expect("recoverable tool_unknown + parallel unwrap succeeds");
    assert_eq!(res.answer.text, "已自纠并完成计划执行");

    let calls = recorded.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "search_sentinel1");
    assert_eq!(calls[1].name, "propose_plan");
    assert_eq!(calls[1].id, "parallel-1-0");
    assert_eq!(
        calls[1].arguments,
        json!({
            "steps": [
                {"tool_id": "working_aoi_set", "input": {"admin_code": "510000"}},
                {"tool_id": "search_start", "input": {"resource_id": "science.sentinel-1.standard.v1"}},
            ]
        })
    );
}

#[tokio::test]
async fn xml_gateway_fn_double_underscore_and_stringified_schema_args_normalize_cleanly() {
    use std::sync::{Arc, Mutex};

    struct XmlGatewayModel;
    impl Model for XmlGatewayModel {
        fn complete<'a>(
            &'a self,
            messages: &'a [Message],
            _: &'a [ToolDescription],
            _: &'a CancellationToken,
            _: &'a mut (dyn FnMut(ModelEvent) + Send),
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Completion, ModelError>> + Send + 'a>,
        > {
            let tool_turns = messages.iter().filter(|m| m.role == "tool").count();
            Box::pin(async move {
                match tool_turns {
                    0 => Ok(Completion {
                        tool_calls: vec![ToolCall {
                            id: "xml-1".into(),
                            name: "fn__admin_boundary_search".into(),
                            arguments: json!({"limit": "5", "query": "四川省"}),
                        }],
                        ..Completion::default()
                    }),
                    1 => Ok(Completion {
                        tool_calls: vec![
                            ToolCall {
                                id: "xml-2".into(),
                                name: "fn__working_aoi_set".into(),
                                arguments: json!({"admin_code": "510000", "clear": "false"}),
                            },
                            ToolCall {
                                id: "xml-3".into(),
                                name: "fn__search_start".into(),
                                arguments: json!({
                                    "resource_id": "science.sentinel-1.standard.v1",
                                    "aoi": r#"{"ref":"working_aoi"}"#,
                                    "filters": r#"{"processing_levels":"[\"SLC\"]","relative_orbit":"118","bare_invalid":"SLC"}"#,
                                }),
                            },
                        ],
                        ..Completion::default()
                    }),
                    _ => Ok(Completion {
                        text: r#"{"text":"已完成四川省检索","claims":[]}"#.into(),
                        ..Completion::default()
                    }),
                }
            })
        }
        fn request_body(
            &self,
            _: &[Message],
            _: &[ToolDescription],
        ) -> Option<Result<Value, ModelError>> {
            None
        }
    }

    struct RecordingDispatcher(Arc<Mutex<Vec<ToolCall>>>);
    impl Dispatcher for RecordingDispatcher {
        fn execute<'a>(
            &'a self,
            call: &'a ToolCall,
            _: &'a CancellationToken,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, ModelError>> + Send + 'a>,
        > {
            self.0.lock().unwrap().push(call.clone());
            Box::pin(async move { Ok(json!({"status": "ok"})) })
        }
    }

    let tools = [
        ToolDescription {
            name: "admin_boundary_search".into(),
            description: "Search admin boundaries".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"},
                }
            }),
        },
        ToolDescription {
            name: "propose_plan".into(),
            description: "Propose plan".into(),
            parameters: json!({"type": "object"}),
        },
    ];

    let recorded = Arc::new(Mutex::new(Vec::new()));
    let agent = AgentLoop { max_steps: 5 };
    let res = agent
        .run(
            &XmlGatewayModel,
            &RecordingDispatcher(Arc::clone(&recorded)),
            &tools,
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "切换到四川省并检索".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .expect("fn__ tool names and stringified integer/json args normalize");
    assert_eq!(res.answer.text, "已完成四川省检索");

    let calls = recorded.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "admin_boundary_search");
    assert_eq!(calls[0].arguments, json!({"limit": 5, "query": "四川省"}));
    assert_eq!(calls[1].name, "propose_plan");
    assert_eq!(
        calls[1].arguments,
        json!({
            "steps": [
                {
                    "tool_id": "working_aoi_set",
                    "input": {"admin_code": "510000", "clear": false}
                },
                {
                    "tool_id": "search_start",
                    "input": {
                        "resource_id": "science.sentinel-1.standard.v1",
                        "aoi": {"ref": "working_aoi"},
                        "filters": {
                            "processing_levels": ["SLC"],
                            "relative_orbit": 118,
                            "bare_invalid": "SLC"
                        }
                    }
                }
            ]
        })
    );
}

#[tokio::test]
async fn phantom_plan_ref_and_fake_run_log_nudge_model_to_call_propose_plan() {
    use std::sync::{Arc, Mutex};

    struct PhantomThenRealModel;
    impl Model for PhantomThenRealModel {
        fn complete<'a>(
            &'a self,
            messages: &'a [Message],
            _: &'a [ToolDescription],
            _: &'a CancellationToken,
            _: &'a mut (dyn FnMut(ModelEvent) + Send),
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Completion, ModelError>> + Send + 'a>,
        > {
            let host_nudges = messages
                .iter()
                .filter(|m| m.role == "user" && m.content.starts_with("[host]"))
                .count();
            let tool_turns = messages.iter().filter(|m| m.role == "tool").count();
            Box::pin(async move {
                if host_nudges == 0 {
                    Ok(Completion {
                        text: "[plan-ref] [plan:plan-544381463456d4a1d173e9129438d526]".into(),
                        ..Completion::default()
                    })
                } else if host_nudges == 1 && tool_turns == 0 {
                    Ok(Completion {
                        text: "[上一轮运行记录]\n提出计划 plan-446d7717863-2（2 步）\n计划步骤 1 working_aoi_set：执行完成".into(),
                        ..Completion::default()
                    })
                } else if tool_turns == 0 {
                    Ok(Completion {
                        tool_calls: vec![ToolCall {
                            id: "real-plan-1".into(),
                            name: "propose_plan".into(),
                            arguments: json!({
                                "steps": [
                                    {
                                        "tool_id": "search_start",
                                        "input": {
                                            "resource_id": "science.sentinel-1.standard.v1",
                                            "aoi": {"ref": "working_aoi"}
                                        }
                                    }
                                ]
                            }),
                        }],
                        ..Completion::default()
                    })
                } else {
                    Ok(Completion {
                        text: r#"{"text":"已真实调用检索工具并命中 250 景","claims":[]}"#.into(),
                        ..Completion::default()
                    })
                }
            })
        }
        fn request_body(
            &self,
            _: &[Message],
            _: &[ToolDescription],
        ) -> Option<Result<Value, ModelError>> {
            None
        }
    }

    struct RecordingDispatcher(Arc<Mutex<Vec<ToolCall>>>);
    impl Dispatcher for RecordingDispatcher {
        fn execute<'a>(
            &'a self,
            call: &'a ToolCall,
            _: &'a CancellationToken,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, ModelError>> + Send + 'a>,
        > {
            self.0.lock().unwrap().push(call.clone());
            Box::pin(async move { Ok(json!({"status": "completed"})) })
        }
    }

    let tools = [ToolDescription {
        name: "propose_plan".into(),
        description: "Propose plan".into(),
        parameters: json!({"type": "object"}),
    }];
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let agent = AgentLoop { max_steps: 6 };
    let res = agent
        .run(
            &PhantomThenRealModel,
            &RecordingDispatcher(Arc::clone(&recorded)),
            &tools,
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "检索四川省最近 3 个月的 Sentinel-1 SLC 数据".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .expect("phantom [plan-ref] and fake [上一轮运行记录] are intercepted and recover");
    assert_eq!(res.answer.text, "已真实调用检索工具并命中 250 景");
    assert_eq!(recorded.lock().unwrap().len(), 1);
    assert_eq!(recorded.lock().unwrap()[0].name, "propose_plan");
}

#[tokio::test]
async fn unfulfilled_action_promise_without_tool_calls_nudges_model_to_execute_tools() {
    use std::sync::{Arc, Mutex};

    struct PrematurePromiseThenRealModel;
    impl Model for PrematurePromiseThenRealModel {
        fn complete<'a>(
            &'a self,
            messages: &'a [Message],
            _: &'a [ToolDescription],
            _: &'a CancellationToken,
            _: &'a mut (dyn FnMut(ModelEvent) + Send),
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Completion, ModelError>> + Send + 'a>,
        > {
            let host_nudges = messages
                .iter()
                .filter(|m| m.role == "user" && m.content.starts_with("[host]"))
                .count();
            let tool_turns = messages.iter().filter(|m| m.role == "tool").count();
            Box::pin(async move {
                if host_nudges == 0 {
                    Ok(Completion {
                        text: "已找到「锦江区」的行政区候选（成都 锦江区，行政区划代码 510105）。\n\n按你的要求，下面一次性提交「把研究区切换到成都市锦江区 + 直接检索 Landsat 影像」，不再单独征求确认。".into(),
                        ..Completion::default()
                    })
                } else if tool_turns == 0 {
                    Ok(Completion {
                        tool_calls: vec![ToolCall {
                            id: "adm-1".into(),
                            name: "admin_boundary_search".into(),
                            arguments: json!({"query": "锦江区", "city": "成都市"}),
                        }],
                        ..Completion::default()
                    })
                } else {
                    Ok(Completion {
                        text: "已核实编码 510104 并完成成都市锦江区检索。".into(),
                        ..Completion::default()
                    })
                }
            })
        }
        fn request_body(
            &self,
            _: &[Message],
            _: &[ToolDescription],
        ) -> Option<Result<Value, ModelError>> {
            None
        }
    }

    struct RecordingDispatcher(Arc<Mutex<Vec<ToolCall>>>);
    impl Dispatcher for RecordingDispatcher {
        fn execute<'a>(
            &'a self,
            call: &'a ToolCall,
            _: &'a CancellationToken,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, ModelError>> + Send + 'a>,
        > {
            self.0.lock().unwrap().push(call.clone());
            Box::pin(async move {
                Ok(json!({"candidates": [{"admin_code": "510104", "name": "锦江区"}]}))
            })
        }
    }

    let tools = [
        ToolDescription {
            name: "admin_boundary_search".into(),
            description: "Search admin boundaries".into(),
            parameters: json!({"type": "object"}),
        },
        ToolDescription {
            name: "propose_plan".into(),
            description: "Propose plan".into(),
            parameters: json!({"type": "object"}),
        },
    ];
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let agent = AgentLoop { max_steps: 6 };
    let res = agent
        .run(
            &PrematurePromiseThenRealModel,
            &RecordingDispatcher(Arc::clone(&recorded)),
            &tools,
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "请改为把研究区切换到成都市锦江区，并直接开始检索 Landsat 影像".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .expect("premature action promise must be nudged to invoke real tools");
    assert_eq!(
        res.answer.text,
        "已核实编码 510104 并完成成都市锦江区检索。"
    );
    assert_eq!(recorded.lock().unwrap().len(), 1);
    assert_eq!(recorded.lock().unwrap()[0].name, "admin_boundary_search");
}

#[tokio::test]
async fn propose_plan_recovers_stringified_steps_array_with_trailing_property_leak() {
    use std::sync::{Arc, Mutex};

    struct TrailingStepsStringModel;
    impl Model for TrailingStepsStringModel {
        fn complete<'a>(
            &'a self,
            messages: &'a [Message],
            _: &'a [ToolDescription],
            _: &'a CancellationToken,
            _: &'a mut (dyn FnMut(ModelEvent) + Send),
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Completion, ModelError>> + Send + 'a>,
        > {
            let has_tool = messages.iter().any(|m| m.role == "tool");
            Box::pin(async move {
                if !has_tool {
                    Ok(Completion {
                        tool_calls: vec![ToolCall {
                            id: "call-landsat".into(),
                            name: "propose_plan".into(),
                            arguments: json!({
                                "steps": "[{\"tool_id\":\"working_aoi_set\",\"input\":{\"admin_code\":\"510104\"}},{\"tool_id\":\"search_start\",\"input\":{\"resource_id\":\"science.landsat.c2-l2.v1\",\"aoi\":{\"ref\":\"working_aoi\"},\"filters\":{\"cloud_cover_percent\":\"40\"}}}], \"language\": \"zh\""
                            }),
                        }],
                        ..Completion::default()
                    })
                } else {
                    Ok(Completion {
                        text: "已完成锦江区 Landsat 检索。".into(),
                        ..Completion::default()
                    })
                }
            })
        }
        fn request_body(
            &self,
            _: &[Message],
            _: &[ToolDescription],
        ) -> Option<Result<Value, ModelError>> {
            None
        }
    }

    struct RecordingDispatcher(Arc<Mutex<Vec<ToolCall>>>);
    impl Dispatcher for RecordingDispatcher {
        fn execute<'a>(
            &'a self,
            call: &'a ToolCall,
            _: &'a CancellationToken,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, ModelError>> + Send + 'a>,
        > {
            self.0.lock().unwrap().push(call.clone());
            Box::pin(async move { Ok(json!({"status": "executed"})) })
        }
    }

    let tools = [ToolDescription {
        name: "propose_plan".into(),
        description: "Propose plan".into(),
        parameters: json!({
            "type": "object",
            "required": ["steps"],
            "properties": {
                "steps": {
                    "type": "array",
                    "items": {
                        "anyOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "tool_id": { "type": "string", "enum": ["working_aoi_set"] },
                                    "input": { "type": "object" }
                                }
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "tool_id": { "type": "string", "enum": ["search_start"] },
                                    "input": { "type": "object" }
                                }
                            }
                        ]
                    }
                }
            }
        }),
    }];
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let agent = AgentLoop { max_steps: 4 };
    let res = agent
        .run(
            &TrailingStepsStringModel,
            &RecordingDispatcher(Arc::clone(&recorded)),
            &tools,
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "检索锦江区 Landsat".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .expect("stringified steps array with trailing property leak must coerce cleanly");
    assert_eq!(res.answer.text, "已完成锦江区 Landsat 检索。");
    let calls = recorded.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].arguments["steps"][1]["input"]["filters"]["cloud_cover_percent"],
        json!(40.0)
    );
}

#[tokio::test]
async fn cross_turn_duplicate_tool_call_ids_get_unique_suffixes_without_mutating_prefix() {
    use std::sync::{Arc, Mutex};

    struct CollidingIdModel;
    impl Model for CollidingIdModel {
        fn complete<'a>(
            &'a self,
            messages: &'a [Message],
            _: &'a [ToolDescription],
            _: &'a CancellationToken,
            _: &'a mut (dyn FnMut(ModelEvent) + Send),
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Completion, ModelError>> + Send + 'a>,
        > {
            let turn2_done = messages.iter().filter(|m| m.role == "tool").count() >= 2;
            Box::pin(async move {
                if !turn2_done {
                    Ok(Completion {
                        tool_calls: vec![ToolCall {
                            id: "call_0".into(),
                            name: "task_overview".into(),
                            arguments: json!({}),
                        }],
                        ..Completion::default()
                    })
                } else {
                    Ok(Completion {
                        text: "ok".into(),
                        ..Completion::default()
                    })
                }
            })
        }
        fn request_body(
            &self,
            _: &[Message],
            _: &[ToolDescription],
        ) -> Option<Result<Value, ModelError>> {
            None
        }
    }

    struct EchoDispatcher(Arc<Mutex<Vec<String>>>);
    impl Dispatcher for EchoDispatcher {
        fn execute<'a>(
            &'a self,
            call: &'a ToolCall,
            _: &'a CancellationToken,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, ModelError>> + Send + 'a>,
        > {
            self.0.lock().unwrap().push(call.id.clone());
            Box::pin(async move { Ok(json!({"ok": true})) })
        }
    }

    let prior_messages = vec![
        Message {
            role: "user".into(),
            content: "Turn 1".into(),
        },
        Message {
            role: "assistant_tool_calls".into(),
            content: json!({
                "text": "",
                "tool_calls": [{"id": "call_0", "name": "task_overview", "arguments": {}}]
            })
            .to_string(),
        },
        Message {
            role: "tool".into(),
            content: json!({"id": "call_0", "result": {"ok": true}}).to_string(),
        },
        Message {
            role: "assistant".into(),
            content: "Turn 1 done".into(),
        },
        Message {
            role: "user".into(),
            content: "Turn 2".into(),
        },
    ];
    let prior_snapshot: Vec<(String, String)> = prior_messages
        .iter()
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();
    let seen_ids = Arc::new(Mutex::new(Vec::new()));
    let tools = [ToolDescription {
        name: "task_overview".into(),
        description: "overview".into(),
        parameters: json!({"type": "object"}),
    }];
    let res = AgentLoop { max_steps: 4 }
        .run(
            &CollidingIdModel,
            &EchoDispatcher(Arc::clone(&seen_ids)),
            &tools,
            Checkpoint::new(prior_messages),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(seen_ids.lock().unwrap().as_slice(), &["call_0_r2"]);
    // Prior messages stayed byte-identical!
    for (i, (r, c)) in prior_snapshot.iter().enumerate() {
        assert_eq!(&res.checkpoint.messages[i].role, r);
        assert_eq!(&res.checkpoint.messages[i].content, c);
    }
    let tool_msg: Value = serde_json::from_str(&res.checkpoint.messages[6].content).unwrap();
    assert_eq!(tool_msg["id"], "call_0_r2");
}

#[tokio::test]
async fn step_limit_and_repeated_call_attach_checkpoint_for_host_persistence() {
    struct AlwaysCallsModel;
    impl Model for AlwaysCallsModel {
        fn complete<'a>(
            &'a self,
            _: &'a [Message],
            _: &'a [ToolDescription],
            _: &'a CancellationToken,
            _: &'a mut (dyn FnMut(ModelEvent) + Send),
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Completion, ModelError>> + Send + 'a>,
        > {
            Box::pin(async move {
                Ok(Completion {
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "task_overview".into(),
                        arguments: json!({}),
                    }],
                    ..Completion::default()
                })
            })
        }
        fn request_body(
            &self,
            _: &[Message],
            _: &[ToolDescription],
        ) -> Option<Result<Value, ModelError>> {
            None
        }
    }

    struct OkDispatcher;
    impl Dispatcher for OkDispatcher {
        fn execute<'a>(
            &'a self,
            _: &'a ToolCall,
            _: &'a CancellationToken,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, ModelError>> + Send + 'a>,
        > {
            Box::pin(async move { Ok(json!({"ok": true})) })
        }
    }

    let tools = [ToolDescription {
        name: "task_overview".into(),
        description: "overview".into(),
        parameters: json!({"type": "object"}),
    }];
    let rep_err = AgentLoop { max_steps: 4 }
        .run(
            &AlwaysCallsModel,
            &OkDispatcher,
            &tools,
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "Check".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(rep_err.kind, ErrorKind::RepeatedCall);
    assert!(rep_err.checkpoint.is_some());

    let limit_err = AgentLoop { max_steps: 1 }
        .run(
            &AlwaysCallsModel,
            &OkDispatcher,
            &tools,
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "Check".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(limit_err.kind, ErrorKind::StepLimit);
    let cp = limit_err
        .checkpoint
        .expect("step_limit must attach checkpoint");
    assert_eq!(cp.messages.len(), 3);
}
