//! L4 r7 P1-3: the final replies of a real model (glm-5.3-flash) put the
//! `{text, claims}` object in a fenced block after prose, or bare after
//! prose; every conclusion came out with no claims and the raw JSON shown as
//! text. The loop now reads the object out of the reply.
use std::{future::Future, pin::Pin};

use insar_agent_model::{
    agent::{AgentLoop, Checkpoint, Claim, Dispatcher, Model, RunResult},
    Completion, Message, ModelError, ModelEvent, ToolCall, ToolDescription,
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

const C1_FENCED: &str = include_str!("fixtures/l4r7-c1-fenced.txt");
const C1R2_FENCED_MULTILINE: &str = include_str!("fixtures/l4r7-c1r2-fenced-multiline.txt");
const C2_BARE: &str = include_str!("fixtures/l4r7-c2-bare.txt");

struct Reply(String);
impl Model for Reply {
    fn complete<'a>(
        &'a self,
        _messages: &'a [Message],
        _tools: &'a [ToolDescription],
        _cancel: &'a CancellationToken,
        _emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ModelError>> + Send + 'a>> {
        let text = self.0.clone();
        Box::pin(async move {
            Ok(Completion {
                text,
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
struct NoTools;
impl Dispatcher for NoTools {
    fn execute<'a>(
        &'a self,
        _call: &'a ToolCall,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
        unreachable!("no tool is called")
    }
}

async fn conclude(reply: &str) -> RunResult {
    AgentLoop { max_steps: 2 }
        .run(
            &Reply(reply.to_owned()),
            &NoTools,
            &[],
            Checkpoint::new(vec![Message {
                role: "user".into(),
                content: "找".into(),
            }]),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap()
}

fn refs(claims: &[Claim]) -> Vec<Vec<String>> {
    claims.iter().map(|claim| claim.refs.clone()).collect()
}

#[tokio::test]
async fn a_fenced_answer_after_prose_is_read_with_its_claims() {
    let result = conclude(C1_FENCED).await;
    assert!(result.structured);
    assert!(
        result.answer.text.starts_with("检索完成：茂县叠溪区域"),
        "{}",
        result.answer.text
    );
    assert!(
        !result.answer.text.contains("```"),
        "{}",
        result.answer.text
    );
    assert!(!result.answer.text.contains("\"claims\""));
    assert_eq!(result.answer.claims.len(), 2);
    let batch = "batch:batch-unified-50757268-fbb4-4679-91c6-ece9c8687443";
    assert_eq!(refs(&result.answer.claims), [vec![batch], vec![batch]]);
    assert!(result.answer.claims[1]
        .statement
        .starts_with("最新一景为 S1D_IW_SLC__1SDV_20261018T110831"));
}

#[tokio::test]
async fn a_multiline_fenced_answer_is_read() {
    let result = conclude(C1R2_FENCED_MULTILINE).await;
    assert!(result.structured);
    assert!(
        result.answer.text.starts_with("已完成"),
        "{}",
        result.answer.text
    );
    assert_eq!(result.answer.claims.len(), 2);
    assert_eq!(
        result.answer.claims[1].refs,
        [
            "task:task-unified-65046dba-c55e-4a01-a49f-728eac13d277",
            "batch:batch-unified-9be8bc9d-826f-4a1b-b6a5-757fcdd1e775"
        ]
    );
}

#[tokio::test]
async fn a_bare_answer_after_prose_is_read_and_its_newlines_decoded() {
    let result = conclude(C2_BARE).await;
    assert!(result.structured);
    assert!(
        result.answer.text.starts_with("已完成检索"),
        "{}",
        result.answer.text
    );
    // The JSON escape \n is a line break in the answer, not two characters.
    assert!(
        result.answer.text.contains("4.66 GB。\n\n不过"),
        "{}",
        result.answer.text
    );
    assert!(!result.answer.text.contains("\\n"));
    assert_eq!(result.answer.claims.len(), 3);
    assert_eq!(
        result.answer.claims[1].refs[1],
        "web:https://datapool.asf.alaska.edu/SLC/SD/S1D_IW_SLC__1SDV_20261018T110831_20261018T110858_005069_009970_518E.zip"
    );
    assert!(result.answer.claims[2].refs.is_empty());
}

#[tokio::test]
async fn think_remnants_and_other_claim_shapes_are_tolerated() {
    let reply = "</think>\n<think>再想想</think>结论如下：\n```json\n{\"text\": \"找到 1 景\", \"claims\": [\
        {\"claim\": \"批次里有 1 景\", \"evidence\": [{\"kind\": \"batch\", \"id\": \"batch-a\"}]},\
        {\"text\": \"任务已建\", \"sources\": \"task:task-a\"},\
        \"没有引用的说法\"]}\n```";
    let result = conclude(reply).await;
    assert!(result.structured);
    assert_eq!(result.answer.text, "找到 1 景");
    assert_eq!(
        refs(&result.answer.claims),
        [
            vec!["batch:batch-a".to_owned()],
            vec!["task:task-a".to_owned()],
            vec![]
        ]
    );
    assert_eq!(result.answer.claims[2].statement, "没有引用的说法");
}

#[tokio::test]
async fn prose_without_an_answer_object_is_kept_as_prose() {
    let reply = "<think>想一下</think>检索完成，共 8 景。{不是 JSON}";
    let result = conclude(reply).await;
    assert!(!result.structured);
    assert_eq!(result.answer.text, "检索完成，共 8 景。{不是 JSON}");
    assert!(result.answer.claims.is_empty());
}

#[tokio::test]
async fn natural_markdown_with_tables_and_code_blocks_is_first_class() {
    let md = concat!(
        "<think>先整理四川省 Sentinel-1 检索结果</think>\n\n",
        "### 四川省 Sentinel-1 SLC 检索结果\n\n",
        "已将研究区切换至 **四川省**，共命中 **24 景** 升轨 `VV+VH` 数据：\n\n",
        "| 景号 | 时间 | 极化 |\n",
        "| --- | --- | --- |\n",
        "| `S1A_IW_SLC__1SDV_20261018` | 2026-10-18 | VV+VH |\n\n",
        "```json\n",
        "{\"text\": \"代码示例中的无关对象\", \"extra_field\": 123}\n",
        "```"
    );
    let result = conclude(md).await;
    assert!(!result.structured);
    assert!(result
        .answer
        .text
        .starts_with("### 四川省 Sentinel-1 SLC 检索结果"));
    assert!(result
        .answer
        .text
        .contains("| `S1A_IW_SLC__1SDV_20261018` |"));
    assert!(
        result.answer.text.contains("\"extra_field\": 123"),
        "unrelated JSON code example in Markdown must not be unwrapped as StructuredAnswer"
    );
    assert!(!result.answer.text.contains("<think>"));
    assert_eq!(result.reasoning_text, "先整理四川省 Sentinel-1 检索结果");
}

#[tokio::test]
async fn fenced_json_answer_with_trailing_commas_unwraps_cleanly() {
    let reply = "```json\n{\n  \"text\": \"### 检索结果\\n共找到 **5 景** 数据。\",\n  \"claims\": [\n    {\"statement\": \"5 景命中\", \"refs\": [\"batch:b-1\",],},\n  ],\n}\n```";
    let result = conclude(reply).await;
    assert!(result.structured);
    assert_eq!(result.answer.text, "### 检索结果\n共找到 **5 景** 数据。");
    assert_eq!(result.answer.claims.len(), 1);
    assert_eq!(result.answer.claims[0].refs, vec!["batch:b-1"]);
}

#[tokio::test]
async fn unclosed_think_in_parse_answer_is_quarantined_never_promoted() {
    let reply = "<think>用户想要四川省数据，但我还没说完就被截断了";
    let result = conclude(reply).await;
    assert_eq!(result.answer.text, "");
    assert!(result.think_block_unclosed);
    assert_eq!(
        result.reasoning_text,
        "用户想要四川省数据，但我还没说完就被截断了"
    );
}
