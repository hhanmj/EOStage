//! Desktop Agent host: persistent sessions, one model loop per run, plan and
//! confirm cards, desktop-card approval, approved execution through the
//! catalog `ToolPort`, cited conclusion and sealed run record.
//!
//! Threading: every public method is short and non-blocking for the caller
//! except the model/execution work, which runs on a spawned worker thread.
//! The Store lock is only taken through `StoreAccess::with_store` and is never
//! held across a service call (the service takes the same lock).
use crate::{
    aoi, catalog, citation,
    executor::{self, ErrorClass, ExecutionError, Executor},
    goal_manifest,
    guardrails::{self, S1PolarizationDecision, SearchCircuitBreaker},
    history,
    host::{self, HostPort, UnifiedApplication},
    message_dir, outbound,
    permission::{
        self, CardReason, OutputDir, PermissionMode, PermissionModeView, Step, StopAllResult, Tier,
        Verdict,
    },
    plan::{self, PlanError},
    run::{self, RunWriter, TraceEvent},
};
use insar_agent_model::{
    agent::{without_think, AgentLoop, Checkpoint, Dispatcher, Model, StructuredAnswer},
    block_on, privacy, CancellationToken, Completion, ErrorKind, Message, ModelError, ModelEvent,
    ToolCall, ToolDescription,
};
use insar_core::{SnapshotItem, TaskLifecycleStatus};
use insar_store::{AgentApprovalRow, AgentPlanRow, AgentRunRow, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    fs::{self, OpenOptions},
    future::Future,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

const MAX_TEXT_CHARS: usize = 8_000;
const MAX_PLAN_STEPS: usize = 8;
/// Serialized `SendContext` limit without attachments (an attached AOI geometry dominates it).
const MAX_CONTEXT_BYTES: usize = 256 * 1024;
/// Serialized `SendContext` limit when multimodal attachments are present.
const MAX_CONTEXT_WITH_ATTACHMENTS_BYTES: usize = 16 * 1024 * 1024;
const MAX_ATTACHMENTS: usize = 6;
const MAX_PERSISTED_ATTACHMENT_BASE64_BYTES: usize = 512 * 1024;
/// Accepted `text_content` limit from the composer (matches `#145` / UI `MAX_TEXT_CONTENT_CHARS`).
const MAX_TEXT_ATTACHMENT_INPUT_CHARS: usize = 65_536;
/// Outbound snippet clip per text attachment so even 3-byte CJK stays safely under `privacy::MAX_STRING`.
const MAX_TEXT_ATTACHMENT_OUTBOUND_CHARS: usize = 32_000;
const POLL_PAGE: usize = 200;
const PLAN_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Error)]
pub enum HostError {
    #[error("agent session or run not found")]
    NotFound,
    #[error("invalid agent request")]
    Invalid,
    #[error("a run is already active in this session")]
    Busy,
    #[error("the Store is unavailable")]
    StoreUnavailable,
    #[error("no model channel is ready")]
    ModelUnavailable,
    #[error("plan expired or Store revision changed")]
    Expired,
    #[error("the plan names a tool outside the catalog")]
    Forbidden,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Run(#[from] run::RunError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
impl HostError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "AGENT_NOT_FOUND",
            Self::Invalid => "AGENT_INVALID_REQUEST",
            Self::Busy => "AGENT_RUN_ACTIVE",
            Self::StoreUnavailable | Self::Store(_) => "AGENT_STORE_UNAVAILABLE",
            Self::ModelUnavailable => "AGENT_MODEL_UNAVAILABLE",
            Self::Expired | Self::Plan(PlanError::Expired) => "AGENT_PLAN_EXPIRED",
            Self::Forbidden => "AGENT_TOOL_FORBIDDEN",
            Self::Plan(_) => "AGENT_PLAN_INVALID",
            Self::Io(_) | Self::Run(_) | Self::Json(_) => "AGENT_RECORD_FAILED",
        }
    }
}

pub use insar_store::TaskStore;

/// Short critical section on the desktop's one Store connection.
pub trait StoreAccess {
    fn with_store<T>(&self, operation: impl FnOnce(&mut TaskStore) -> T) -> Result<T, HostError>;
}

/// A model opened for one run. `route` is the label recorded in the run row
/// (adapter and model id only, never a key).
pub struct OpenedModel {
    pub model: Box<dyn Model + Send>,
    pub route: String,
}
pub trait ModelSource: Send + Sync {
    fn open(&self) -> Result<OpenedModel, HostError>;
    /// Opens the model a send chose (channel, model and thinking level);
    /// without a choice, the default channel. Sources without a model list
    /// open their default.
    fn open_selected(&self, selection: Option<&ModelSelection>) -> Result<OpenedModel, HostError> {
        let _ = selection;
        self.open()
    }
    /// Opens the model for a specific session so the transport can derive a
    /// stable per-session `prompt_cache_key` (never containing paths/secrets).
    fn open_for_session(
        &self,
        session_id: &str,
        selection: Option<&ModelSelection>,
    ) -> Result<OpenedModel, HostError> {
        let _ = session_id;
        self.open_selected(selection)
    }
    /// Opens the model for a turn; when `prefer_reasoning_off` is true (e.g.
    /// turn-0 casual greeting fast-path) and the user did not pin an explicit
    /// non-auto thinking level, defaults thinking off for lower latency.
    fn open_for_turn(
        &self,
        session_id: &str,
        selection: Option<&ModelSelection>,
        prefer_reasoning_off: bool,
    ) -> Result<OpenedModel, HostError> {
        let adjusted = if prefer_reasoning_off {
            selection.map(|sel| {
                let mut next = sel.clone();
                let is_auto = next
                    .reasoning_level
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(|lvl| {
                        lvl.is_empty()
                            || lvl.eq_ignore_ascii_case("auto")
                            || lvl.eq_ignore_ascii_case("default")
                    });
                if is_auto {
                    next.reasoning_level = Some("off".to_owned());
                }
                next
            })
        } else {
            None
        };
        self.open_for_session(session_id, adjusted.as_ref().or(selection))
    }
}

/// The channel, model and thinking level the user picked in the composer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelSelection {
    pub channel_id: String,
    pub model_id: String,
    #[serde(default)]
    pub reasoning_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummary {
    pub session_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub run_ids: Vec<String>,
    /// This session's permission mode; `None` uses confirm_changes.
    /// A mode stored by W3e is read under its new name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "permission::stored_session_mode"
    )]
    pub permission_mode: Option<PermissionMode>,
    /// Pinned sessions are listed first, most recently pinned first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_at: Option<String>,
    /// Archived sessions are left out of the list unless asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    /// The workspace the session is filed under; `None` is "unfiled".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Most recent valid local download directory named by the user in this
    /// session or confirmed on a card; local desktop persistence only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remembered_output_dir: Option<String>,
    /// Status of the session's latest run when present (`running`,
    /// `awaiting_approval`, `completed`, `failed`, or `cancelled`). Computed
    /// dynamically on read APIs and omitted when persisted to `session.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<String>,
}

/// A named group of sessions bound to an optional local folder (the default
/// download folder its confirm cards offer; the card still decides).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workspace {
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub folder: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionExportFormat {
    Json,
    Markdown,
}

/// A session rendered for the user to save; the desktop asks where.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionExport {
    pub file_name: String,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamEvent {
    pub seq: u64,
    pub kind: String,
    pub payload: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamPage {
    pub cursor: u64,
    pub events: Vec<StreamEvent>,
}
/// An image or text/file snippet attached to a user message from the composer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SendAttachment {
    /// `"image"`, `"text"`, or `"file"`.
    pub kind: String,
    pub name: String,
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SendContext {
    #[serde(default)]
    pub aoi: Option<Value>,
    #[serde(default)]
    pub time_range: Option<Value>,
    #[serde(default)]
    pub source: Option<String>,
    /// The model this run should use; none uses the default channel.
    #[serde(default)]
    pub model: Option<ModelSelection>,
    /// Optional pasted/attached images or text/file snippets from the composer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<SendAttachment>,
}

/// Real-time host notifications pushed to the Tauri bridge (Slice D) so the UI
/// can receive low-latency stream deltas and map/workbench synchronization
/// without waiting on `stream_poll`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentHostNotification {
    StreamEvent {
        session_id: String,
        event: StreamEvent,
    },
    LiveDelta {
        session_id: String,
        run_id: String,
        step_index: u64,
        kind: String,
        delta: String,
        started_at_ms: Option<u64>,
        duration_ms: Option<u64>,
    },
    AoiChanged {
        session_id: String,
        run_id: String,
        state: String,
        display_name: Option<String>,
        bbox: Option<Vec<f64>>,
    },
    SearchUpdated {
        session_id: String,
        run_id: String,
        request_id: String,
        batch_id: Option<String>,
    },
    WorkspaceSync {
        session_id: String,
        run_id: String,
        payload: Value,
    },
}

pub type AgentHostNotifier = Arc<dyn Fn(AgentHostNotification) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTruncateResult {
    pub session: SessionSummary,
    pub cursor: u64,
    pub aoi_restored: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionWorkspaceState {
    pub session_id: String,
    pub initialized: bool,
    pub working_aoi: Value,
    #[serde(default)]
    pub active_batch_id: Option<String>,
    #[serde(default)]
    pub active_resource_id: Option<String>,
    #[serde(default)]
    pub active_batch_revision: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunSummary {
    pub run_id: String,
    pub session_id: String,
    pub status: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub model_route: Option<String>,
    /// From the sealed run row (Store schema 36): a provider refused the
    /// reasoning-off switch and the request was resent without it.
    #[serde(default)]
    pub reasoning_off_refused: bool,
    /// A model reply was cut off at the output limit (the run failed with
    /// `model_truncated`).
    #[serde(default)]
    pub truncated: bool,
    /// Token usage the channel reported over the run's replies, from the
    /// sealed run row (Store schema 37); None when no reply reported usage.
    #[serde(default)]
    pub usage_total: Option<RunUsage>,
}
/// `AgentRunSummary.usage_total`: prompt and completion tokens summed over
/// the replies that reported them, and their total.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunUsage {
    pub prompt: Option<u64>,
    pub completion: Option<u64>,
    pub total: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
}
/// `agent_activity_status`: what the stop-everything button would stop now,
/// across sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityStatus {
    pub live_runs: u64,
    pub agent_active_tasks: u64,
    pub sessions_with_live_runs: Vec<String>,
}
/// One run of a session as the "任务与成果" panel lists it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkItem {
    pub run: RunSummary,
    /// The user's message that started the run (first 120 characters).
    pub request: Option<String>,
    pub plan_ids: Vec<String>,
    /// Download tasks the run created (not ones it reused).
    pub created_task_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunDetail {
    pub run: RunSummary,
    pub events: Vec<TraceEvent>,
}

/// Model, executor, trace writer and conversation of one run. Parked in memory
/// while its plan waits for the desktop card; a restart interrupts it.
struct RunState<S> {
    run_id: String,
    session_id: String,
    route: String,
    model: Box<dyn Model + Send>,
    executor: Executor<HostPort<S>>,
    writer: RunWriter,
    checkpoint: Checkpoint,
    turn_start_msg_index: usize,
    initial_user_msg: Option<Message>,
    active_toolsets: BTreeSet<String>,
    local: LocalContext,
    /// `agent_runs.origin`; only an in-app run is approved by mode (the
    /// Store refuses any other as well).
    origin: &'static str,
    circuit_breaker: SearchCircuitBreaker,
    consecutive_failures: u8,
    verified_admin_codes: std::collections::HashSet<String>,
    goal_manifest: Option<goal_manifest::GoalManifest>,
    casual_fast_path: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionCompaction {
    compacted_run_ids: Vec<String>,
    summary_messages: Vec<Message>,
    #[serde(default)]
    compacted_toolsets: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SessionToolsetState {
    active_toolsets: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct CompletedToolStep {
    call_id: String,
    tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_summary: Option<String>,
    ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct ResumeCheckpoint {
    run_id: String,
    original_user_goal: String,
    #[serde(default)]
    completed_tool_steps: Vec<CompletedToolStep>,
    answer_delivered: bool,
    #[serde(default)]
    authorized_actions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    interrupt_reason: Option<String>,
    updated_at: String,
}

/// What the user's message names that stays on this machine.
#[derive(Default)]
struct LocalContext {
    /// The attached AOI; the model sees `aoi::context_summary`.
    aoi: Option<Value>,
    /// A directory named in the message text (W3g); the model sees
    /// `message_dir::PLACEHOLDER`.
    message_dir: Option<String>,
    /// User message text (kept local) so guardrails can check whether the user
    /// explicitly asked for polarizations such as HH.
    user_text: String,
    /// Structured checkpoint carried forward when this run resumes an interrupted turn.
    resumed_from: Option<ResumeCheckpoint>,
}
type Cards = Vec<(&'static str, Value)>;
type ProposedPlan = (String, Cards, Vec<Step>, Tier, Option<u64>);

/// A proposed plan and the cards to show once the run is parked, so a card
/// is never visible before the run can accept its approval.
struct Proposal {
    plan_id: String,
    call_id: String,
    cards: Cards,
    steps: Vec<Step>,
    tier: Tier,
    estimated_bytes: Option<u64>,
}
/// Who decides a parked plan: the desktop card, or the permission mode.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Approver {
    Card,
    Mode,
}
/// One decision on a parked plan; `output_dir` is a directory chosen on the
/// card (approvals only).
struct Decision {
    snapshot: Value,
    approved: bool,
    output_dir: Option<String>,
}
/// What a card's output directory choice replaced, for the run record.
struct RewriteRecord {
    from_plan_hash: String,
    from_dir: Option<String>,
    from_source: Option<String>,
}
/// A directory as the run record names it: SHA-256 of the run id, one NUL
/// byte and the path. Salting with the run id keeps a shared run record from
/// confirming a guessed path by an unsalted lookup, while the Store (which
/// holds the path) can still recompute it.
pub fn path_digest(run_id: &str, path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(run_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(path.as_bytes());
    format!("{:x}", hasher.finalize())
}
struct Parked<S> {
    state: RunState<S>,
    plan_id: String,
    call_id: String,
}
struct Live<S> {
    session_id: String,
    cancel: CancellationToken,
    parked: Option<Parked<S>>,
}

pub struct AgentHost<S, M> {
    root: PathBuf,
    service: S,
    models: M,
    files: Mutex<HashMap<String, u64>>,
    live: Mutex<HashMap<String, Live<S>>>,
    counter: AtomicU64,
    /// Tasks Agent runs created, as (session, task): read once from the run
    /// records on first use, then extended when a run's create_task lands.
    agent_tasks: Mutex<Option<Vec<(String, String)>>>,
    /// The activity last announced with `activity_changed`; held while an
    /// announcement is computed and written, so announcements never cross.
    activity: Mutex<Option<ActivityStatus>>,
    max_steps: AtomicUsize,
    /// Optional real-time push callback registered by the Tauri shell.
    notifier: Mutex<Option<AgentHostNotifier>>,
    /// Trace event kinds whose next write fails (fault injection for tests
    /// of the run-record failure paths; empty in the desktop).
    trace_faults: Mutex<Vec<&'static str>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// RFC 3339 UTC seconds; lexicographic order equals time order.
pub fn rfc3339(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}
fn now() -> String {
    rfc3339(SystemTime::now())
}

/// A session title is stored locally and listed in history; it goes through
/// the outbound redaction first (L4 r8 G1), so no credential or local path is
/// kept in it.
fn session_title(text: &str) -> Option<String> {
    Some(
        history::outbound_text(text.trim())
            .chars()
            .take(80)
            .collect::<String>(),
    )
    .filter(|title| !title.trim().is_empty())
}

fn workspace_name(text: &str) -> Option<String> {
    let name = text.trim().chars().take(60).collect::<String>();
    (!name.is_empty()).then_some(name)
}

/// A workspace folder is an absolute local path; the folder need not exist
/// yet (the confirm card and the download create it).
fn workspace_folder(folder: Option<String>) -> Result<Option<String>, HostError> {
    match folder.map(|folder| folder.trim().to_owned()) {
        None => Ok(None),
        Some(folder) if folder.is_empty() => Ok(None),
        Some(folder) if Path::new(&folder).is_absolute() => Ok(Some(folder)),
        Some(_) => Err(HostError::Invalid),
    }
}

fn export_file_stem(title: &str) -> String {
    let stem = title
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .take(60)
        .collect::<String>();
    let stem = stem.trim().trim_end_matches('.');
    if stem.is_empty() {
        "会话".to_owned()
    } else {
        stem.to_owned()
    }
}

/// The conversation as Markdown: what the user asked, what the assistant
/// answered, and each run's conclusion or error.
fn session_markdown(summary: &SessionSummary, events: &[StreamEvent]) -> String {
    let mut out = format!(
        "# {}\n\n创建于 {}，导出于 {}\n",
        summary.title,
        summary.created_at,
        now()
    );
    for event in events {
        let text = |key: &str| event.payload.get(key).and_then(Value::as_str);
        let section = match (event.kind.as_str(), text("role")) {
            ("message", Some("user")) => text("text").map(|t| ("用户", t)),
            ("message", Some("assistant")) => text("text").map(|t| ("助手", t)),
            ("conclusion", _) => text("text").map(|t| ("结论", t)),
            ("error", _) => text("message_zh")
                .or_else(|| text("code"))
                .map(|t| ("出错", t)),
            _ => None,
        };
        if let Some((heading, body)) = section {
            if !body.trim().is_empty() {
                out.push_str(&format!("\n## {heading}\n\n{}\n", body.trim()));
            }
        }
    }
    out
}

fn catalog_sha256() -> String {
    format!(
        "{:x}",
        Sha256::digest(include_bytes!(
            "../../../contracts/v3/agent-tool-catalog.v1.json"
        ))
    )
}

const SYSTEM_PROMPT: &str = "你是 EOStage 的数据准备助手。只能通过提供的只读工具查看本机状态。\
消息末尾的 [context]（intent_mode 为 passive_workspace_snapshot）仅代表用户当前界面上停留的工作区被动快照（地图 AOI、时间窗口、数据源下拉项），绝不代表用户要求你检索、预检或下载这些数据。\
当用户只是打招呼（如「hi」「你好」）、提问概念/原理/架构/排查建议或咨询元问题（如「为什么缓存命中率那么低」「帮我评估当前 Agent 架构」）且问题不依赖本机实时任务或工作台数据时，请直接清晰作答，不要反射性调用 task_overview、workbench_page、capabilities_get、service_readiness、source_list 等工具，也严禁仅因 [context] 里已有 AOI 或数据源就擅自发起检索、预检或下载。\
当用户发送「继续」「接着继续」等续跑指令且消息附带 <resume_checkpoint> 时，直接复用其中的 original_user_goal 与 completed_tool_steps 继续完成或直接输出最终结论；严禁重复调用已完成的只读检查工具，更严禁在 authorized_actions 为空时擅自新建检索或下载任务。\
同一会话里以前的对话按轮附在本条消息之前：每轮的助手消息是主机根据那一轮的运行记录整理的摘要（检索冻结的批次、读到的产品、计划及其结果、建的任务和结论），\
可以沿用其中的批次、产品与任务编号（结论的引用仍只能指向本次运行实际读到的任务或批次）。\
需要设置工作区（working_aoi_set）、检索（search_start）、建任务（create_task）或控制任务（task_action/update_download_task）时，\
调用 propose_plan 提交完整步骤。按用户设置的权限模式，计划会等用户在桌面确认卡上批准，或自动执行（用户可随时取消）；\
工具结果会说明计划是已执行、已失效，还是被用户在确认卡上拒绝（status 为 rejected）。不要声称已执行未执行的操作，也不要请用户批准已经在执行的计划；严禁在回复正文中输出 [plan-ref] 占位符、伪造执行记录，或只输出「下面我将提交/正在为您检索」等预告文字却不发起真实的 function tool_calls；严禁凭记忆猜测 admin_code（尤其是区县编码）或 resource_id。\
当用户指定了新的省市区或地名（与当前工作区不同，或当前没有工作区）时：必须先调用只读工具 admin_boundary_search 查找真实行政区候选获取准确的 admin_code，\
再在同一个 propose_plan 里依次提交 working_aoi_set（填 admin_code）与 search_start（aoi 填 {\"ref\":\"working_aoi\"}），主机在同一轮就会用新行政区的真实多边形边界完成检索，不要沿用旧工作区。\
检索区域不提供几何：search_start 的 aoi 填 {\"ref\":\"context_aoi\"}（用户附带的区域）或 {\"ref\":\"working_aoi\"}（当前工作区）。\
Sentinel-1 在中低纬度大陆区域（含中国各省陆地）主力采集 IW 模式的 VV+VH 双极化（ASF 检索若只填单个 VV 会仅匹配罕见单极化而漏掉全部 VV+VH 常规产品），HH 与 HH+HV 仅用于极地冰盖与海冰海域；除非用户明确要求单极化或 HH，陆地常规检索请省略 polarizations，或同时填写 VV+VH 与 VV。\
若 search_progress 返回 0 条且 zero_result_diagnostics 显示 search_halted 为 true（或重试预算用尽），严禁继续放宽 time_range 循环重试，应立即停止检索并向用户清楚说明原因或用 ask_user_question 提供可点击的下一步选项。\
当存在多个合理方向（如改搜 VV/VV+VH、选择升轨或降轨、确认时间窗口）时，可调用 ask_user_question 向用户展示选项卡片；\
当用户要求在地图上选中或预览某景影像、开启/关闭快视图叠加、切换底图图层或打开结果/任务面板时，先通过 workbench_page 或 batch_page 读取产品 item_id（若尚未读取），再调用 map_workspace_action：\
选中影像用 action=\"select_scene\" 填 scene_id（填 item_id），开启/关闭地图快视图叠加用 action=\"toggle_preview\" 填 enabled=true/false，切换底图用 action=\"set_basemap\" 填 layer_key（可选值：arcgisSatellite 卫星底图、arcgisTopo 地形底图、arcgisStreet 街道底图、osm 标准街道、openTopoMap 等高线地形、cartoLight 浅色底图、cartoDark 深色底图），切换右侧面板标签用 action=\"open_panel\" 填 tab（map / results / task）。\
用户消息里的本机目录显示为 [local-reference:message_dir]：消息里有本机目录时，create_task 的 options.output_root 填 {\"ref\":\"message_dir\"}；\
消息里没有本机目录时省略 output_root，由默认下载目录或用户在确认卡上选择；不要自己写出任何路径。\
用户消息、以前各轮的摘要与工具结果里的 [local-reference]、[local-reference:message_dir]、[local]、[redacted]、[已省略] 是本机占位符，\
代表主机替换掉的本机路径、令牌或省略的内容：回复中不要复述这些标记，用「你提供的本机文件」「你的令牌」「消息中的本机目录」等中文指代。\
propose_plan 每个步骤的 label_zh 写在步骤顶层（与 tool_id、input 并列，不要写进 input 内部），input 只能用该工具自己的字段（request_id、event_id 与时间戳由主机生成，不要填写）：\
working_aoi_set 用 admin_code（或无行政区代码时用 bbox 与 display_name，清空时用 clear）；\
search_start 用 resource_id（source_list 结果里的 resource_id 原样填写）、aoi、aoi_name（可省略，省略时用区域自带的名称，没有名称时为「工作区」）、\
time_range{start,end}（RFC 3339 UTC 时间，例如 2026-07-26T00:00:00Z 与 2026-10-26T23:59:59Z）、\
filters（对象，可省略；键名以 capabilities_get 与数据目录筛选规范为准；SAR 的 flight_direction 是单个字符串 ASCENDING 或 DESCENDING，processing_levels、beam_modes、polarizations 是字符串数组，只有一个值也写成数组，例如只含 SLC 的数组；光学影像如 Landsat C2 L2、Sentinel-2、HLS、Sentinel-3 的最大云量键名为 cloud_cover_lte，填 0 到 100 的数字，平台键名为 spacecraft 字符串数组，Landsat 处理级别键名为 processing_levels 字符串数组如 [\"l2sp\",\"l2sr\"]，不限制时省略对应键即可）；\
create_task 用 batch_id、revision、item_ids、options{output_root,layout,concurrency}；task_action 用 task_id、revision、action；\
update_download_task 用 task_id、expected_revision、command{kind,item_ids}。\
工具结果的 error 为 plan_refused 时，是主机按 reason 拒绝了这个计划，不是用户拒绝，也没有显示确认卡：\
把 message_zh 里的原因如实告诉用户；按 hint_zh 以及 unknown_fields、missing_fields、expected_input 修正后可以重新提交。\
工具结果（或已执行计划的某一步）的 error 为 tool_denied 时，是这次调用没有执行：reason 说明原因（主机检查不通过或服务拒绝），\
把 message_zh 告诉用户，按 hint_zh 修正后再调用；hint_zh 说不要重复时就不要重试同一个调用。\
create_task 执行结果里的 output_dir_source 与 output_dir_note_zh 说明最终下载目录的来源（default / chosen_on_card / from_message / model_suggested），转述下载目录时以它为准；\
计划被拒绝或失效时，结果里的 output_dir_note_zh 说明没有建任务以及计划里的目录来源，同样以它为准。\
面对多要素复合配数目标（例如同时配齐 SAR、DEM、精密轨道或光学数据）时，可使用 goal_manifest（先 define 初始化各交付项，再用真实 batch_id 或 task_id 调用 bind 核验并标为 ready，最后 check 核对）；凡未标为 ready 的待补齐、待确认或能力未接入项，回答时必须如实逐项说明，不得声称已全部配齐。\
最终回答直接用清晰自然的中文 Markdown 写给用户看（支持小标题、列表、表格与加粗重点），主机将自动挂载本次运行核验过的批次与任务证据卡片：\
正文中不要复述内部编号（task-…、run-…、plan-…、batch-…），\
也不要写英文状态词（queued、running、paused、completed、failed、cancelled、interrupted），改用与任务卡相同的说法（排队中、传输中、已暂停、已完成、已失败、已取消、已中断），\
任务与批次用数据源、区域、日期、景数等描述。（如需显式附带结构化引用，仍兼容输出单个 {\"text\":\"…\",\"claims\":[…]} JSON 对象，引用只能指向本次运行实际读到的任务或批次。）";

const CASUAL_SYSTEM_PROMPT: &str = "你是 EOStage 遥感与地球科学智能工作台助手。用户当前处于日常问候阶段：请直接用自然、友好的简短中文（或随用户语言）回复，不要输出 JSON，不要调用任何工具。（消息末尾 [context] 中的 passive_workspace_snapshot 仅作当前界面状态参考，禁止据此主动检索或下载。）";

fn is_pure_casual_greeting(text: &str) -> bool {
    let cleaned = text
        .trim()
        .trim_matches(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '!' | '?'
                        | '.'
                        | ','
                        | '~'
                        | '。'
                        | '！'
                        | '？'
                        | '，'
                        | '～'
                        | '、'
                        | '…'
                        | '👋'
                        | '🙏'
                        | '😊'
                        | '🙂'
                        | '😄'
                        | '🤝'
                )
        })
        .trim();
    if cleaned.is_empty() || cleaned.chars().count() > 24 {
        return false;
    }
    matches!(
        cleaned.to_ascii_lowercase().as_str(),
        "hi" | "hello"
            | "hey"
            | "yo"
            | "hola"
            | "hi there"
            | "hello there"
            | "morning"
            | "good morning"
            | "good afternoon"
            | "good evening"
            | "thanks"
            | "thank you"
            | "thx"
            | "你好"
            | "您好"
            | "你好呀"
            | "你好啊"
            | "嗨"
            | "哈喽"
            | "哈啰"
            | "在吗"
            | "在么"
            | "早"
            | "早上好"
            | "早安"
            | "上午好"
            | "下午好"
            | "晚上好"
            | "晚安"
            | "谢谢"
            | "多谢"
            | "感谢"
    )
}

/// A read tool's input as the model is shown it: the fields, bounds and
/// required set of the type `host::input_problem` checks (L4 r7: a schema
/// without `required` let the model send `{}` to workbench_page).
fn input_schema(tool_id: &str) -> Value {
    if tool_id == "goal_manifest" {
        return goal_manifest::schema();
    }
    let page_size = json!({"type": "integer", "minimum": 1, "maximum": 100});
    let index = json!({"type": "integer", "minimum": 0, "description": "从 0 开始"});
    let id = json!({"type": "string", "minLength": 1, "maxLength": 128});
    let (required, props) = match tool_id {
        "workbench_page" => (
            json!(["page_index", "page_size"]),
            json!({"page_index": index, "page_size": page_size}),
        ),
        "task_page" => (
            json!(["task_id", "page_index", "page_size"]),
            json!({"task_id": id, "page_index": index, "page_size": page_size}),
        ),
        "task_event_page" => (
            json!(["task_id", "page_size"]),
            json!({"task_id": id, "page_size": page_size}),
        ),
        "batch_page" => (
            json!(["batch_id", "revision", "page", "page_size"]),
            json!({"batch_id": id,
                "revision": {"type": "integer", "minimum": 0,
                    "description": "该批次自己的 revision（workbench_page 或 batch_page 结果里的 revision，冻结批次为 0），不是任务的 revision"},
                "page": index, "page_size": page_size}),
        ),
        "search_progress" => (json!(["request_id"]), json!({"request_id": id})),
        "admin_boundary_search" => (
            json!([]),
            json!({
                "query": {"type": "string", "minLength": 1, "maxLength": 160,
                    "description": "检索地名或行政区名称，例如 四川省、成都市 或 Paris"},
                "province": {"type": "string", "maxLength": 160, "description": "可选省级名称"},
                "city": {"type": "string", "maxLength": 160, "description": "可选市级名称"},
                "district": {"type": "string", "maxLength": 160, "description": "可选区县级名称"},
                "country_code": {"type": "string", "minLength": 2, "maxLength": 16, "description": "可选国家代码，例如 CN"},
                "level": {"type": "string", "minLength": 1, "maxLength": 32, "description": "可选行政层级"},
                "provider_object_id": {"type": "string", "minLength": 1, "maxLength": 160, "description": "可选精确行政区编码，例如 510000"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 20, "description": "返回候选上限，1 到 20"},
            }),
        ),
        "ask_user_question" => (
            json!(["question_zh", "options"]),
            json!({
                "question_zh": {"type": "string", "minLength": 1, "maxLength": 240,
                    "description": "向用户提出的简明中文问题"},
                "options": {
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 5,
                    "items": {
                        "type": "object",
                        "required": ["label_zh", "value"],
                        "additionalProperties": false,
                        "properties": {
                            "label_zh": {"type": "string", "minLength": 1, "maxLength": 80},
                            "value": {"type": "string", "minLength": 1, "maxLength": 160},
                            "recommended": {"type": "boolean"},
                        }
                    }
                }
            }),
        ),
        "map_workspace_action" => (
            json!(["action"]),
            json!({
                "action": {
                    "type": "string",
                    "enum": ["select_scene", "toggle_preview", "set_basemap", "fit_bounds", "open_panel"],
                    "description": "地图工作区动作：select_scene 选中指定景；toggle_preview 开启/关闭快视图叠加；set_basemap 切换底图图层；fit_bounds 缩放视野；open_panel 打开右侧面板标签"
                },
                "scene_id": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 256,
                    "description": "select_scene 时填写的影像 item_id / scene_id（来自 workbench_page 或 batch_page）"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "toggle_preview 时填写：true 开启地图影像快视图叠加，false 关闭"
                },
                "layer_key": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 64,
                    "description": "set_basemap 时填写的底图标识：arcgisSatellite（卫星）、arcgisTopo（地形）、arcgisStreet（街道）、osm、openTopoMap、cartoLight、cartoDark"
                },
                "target": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 64,
                    "description": "fit_bounds 时可选填 aoi"
                },
                "tab": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 64,
                    "description": "open_panel 时填写：map、results 或 task"
                },
                "batch_id": {"type": "string", "minLength": 1, "maxLength": 128},
                "display_name": {"type": "string", "minLength": 1, "maxLength": 160},
                "bbox": {
                    "type": "array",
                    "minItems": 4,
                    "maxItems": 4,
                    "items": {"type": "number"}
                }
            }),
        ),
        _ => (json!([]), json!({})),
    };
    let mut schema = json!({"type": "object", "properties": props, "additionalProperties": false});
    if required.as_array().is_some_and(|fields| !fields.is_empty()) {
        schema["required"] = required;
    }
    schema
}

/// Tier-0 read tools run directly; everything else is reachable only as a
/// step of `propose_plan`, which parks the run for the desktop card. Each
/// step variant carries its tool's own input schema (`host::plan_input_schema`),
/// so the model is shown the fields the host will accept.
pub fn model_tools() -> Vec<ToolDescription> {
    let tools = catalog::catalog().tools;
    let steps: Vec<Value> = tools
        .iter()
        .filter(|tool| tool.confirmation != "none")
        .map(|tool| {
            // A plan tool added to the catalog without a schema here stays
            // offered, with an open input as before.
            let input =
                host::plan_input_schema(&tool.tool_id).unwrap_or_else(|| json!({"type": "object"}));
            json!({
                "type": "object", "required": ["tool_id", "input"], "additionalProperties": false,
                "properties": {
                    "tool_id": {"type": "string", "enum": [tool.tool_id]},
                    "input": input,
                    "label_zh": {"type": "string"},
                },
            })
        })
        .collect();
    let mut described: Vec<ToolDescription> = tools
        .iter()
        .filter(|tool| tool.effect == "read" && tool.confirmation == "none")
        .map(|tool| ToolDescription {
            name: tool.tool_id.clone(),
            description: tool.description.clone(),
            parameters: input_schema(&tool.tool_id),
        })
        .collect();
    described.push(ToolDescription {
        name: "propose_plan".into(),
        description: "Propose workspace AOI updates, search, task creation or task control steps for desktop approval. \
            request_id, event_id and occurred_at are filled by the host; do not fill them."
            .into(),
        parameters: json!({
            "type":"object","required":["steps"],
            "properties":{"steps":{"type":"array","minItems":1,"maxItems":MAX_PLAN_STEPS,"items":{"anyOf": steps}}}
        }),
    });
    described.push(ToolDescription {
        name: "ask_user_question".into(),
        description: "Present a concise Chinese clarifying question with 2 to 5 clickable options in the chat stream when the user should choose among alternatives."
            .into(),
        parameters: input_schema("ask_user_question"),
    });
    described.push(ToolDescription {
        name: "map_workspace_action".into(),
        description: "Synchronize the map and workbench UI view state: select a scene, toggle preview, switch basemap, fit bounds or open a panel."
            .into(),
        parameters: input_schema("map_workspace_action"),
    });
    described.push(ToolDescription {
        name: "goal_manifest".into(),
        description: "Define, bind store-backed batch/task evidence, and check completion status for multi-item scientific data preparation checklists."
            .into(),
        parameters: input_schema("goal_manifest"),
    });
    described
}

/// A legacy ASF source's filters are its `SarSearchQuery`, whose `dataset`
/// the service requires (and then binds from the registry itself); the host
/// writes the registry selector so a plan without filters is accepted.
/// Typed catalogue sources and unknown resources are left as given.
fn legacy_dataset(input: &mut Value) {
    let Some(selector) = legacy_selector(input) else {
        return;
    };
    if let Some(filters) = input.get_mut("filters").and_then(Value::as_object_mut) {
        if selector == "SENTINEL-1" {
            if let Some(levels) = filters
                .get_mut("processing_levels")
                .and_then(Value::as_array_mut)
            {
                let mut expanded: Vec<Value> = Vec::with_capacity(levels.len() + 3);
                let mut push_unique = |val: Value| {
                    if let Some(s) = val.as_str() {
                        if !expanded.iter().any(|existing| {
                            existing.as_str().is_some_and(|e| e.eq_ignore_ascii_case(s))
                        }) {
                            expanded.push(Value::String(s.to_owned()));
                        }
                    } else {
                        expanded.push(val);
                    }
                };
                for item in std::mem::take(levels) {
                    if item
                        .as_str()
                        .is_some_and(|s| s.trim().eq_ignore_ascii_case("GRD"))
                    {
                        for sub in ["GRD_HD", "GRD_HS", "GRD_MD", "GRD_MS"] {
                            push_unique(Value::String(sub.to_owned()));
                        }
                    } else {
                        push_unique(item);
                    }
                }
                *levels = expanded;
            }
        }
        filters.insert("dataset".into(), Value::String(selector));
    }
}

/// The registry selector of a legacy ASF source; `None` for any other.
fn legacy_selector(input: &Value) -> Option<String> {
    let resource = input.get("resource_id").and_then(Value::as_str)?;
    let registry = insar_core::ResourceRegistry::embedded_ref().ok()?;
    let Some(insar_core::ResourceDescriptor::ScienceDataset { science, .. }) =
        registry.resource(resource)
    else {
        return None;
    };
    science
        .sar_search
        .as_ref()
        .filter(|search| {
            search.projection_kind == insar_core::SarSearchProjectionKind::LegacyAsfSearch
        })
        .map(|search| search.selector.clone())
}

/// The service's own rule for a legacy ASF source's filters: its
/// `SarSearchQuery` (unknown keys refused) with the request's area and time
/// written in, then validated. A shape the service would refuse after the
/// plan is approved is refused when it is proposed (L4 r7: `flight_direction`
/// as an array, `processing_levels` as a string). `None` for other sources.
fn legacy_filter_problem(input: &Value) -> Option<String> {
    legacy_selector(input)?;
    let filters = input.get("filters").cloned().unwrap_or_else(|| json!({}));
    let mut query: insar_core::SarSearchQuery = match serde_json::from_value(filters) {
        Ok(query) => query,
        Err(error) => return Some(format!("filters: {error}")),
    };
    query.provider = "ASF".to_owned();
    query.aoi_geojson = input.get("aoi").and_then(Value::as_object).cloned();
    query.max_results = Some(250);
    if let Some(range) = input.get("time_range").filter(|range| range.is_object()) {
        let text = |key: &str| range.get(key).and_then(Value::as_str).map(str::to_owned);
        query.start = text("start");
        query.end = text("end");
    }
    query
        .validate()
        .err()
        .map(|error| format!("filters: {error}"))
}

fn optical_schema(input: &Value) -> Option<&'static insar_core::SearchFilterSchema> {
    let resource = input.get("resource_id").and_then(Value::as_str)?;
    let registry = insar_core::ResourceRegistry::embedded_ref().ok()?;
    let Some(insar_core::ResourceDescriptor::ScienceDataset { science, .. }) =
        registry.resource(resource)
    else {
        return None;
    };
    science.search_schema_id.as_ref()?;
    let schemas = insar_core::SearchFilterSchemaCatalog::embedded_ref().ok()?;
    schemas.for_resource(resource)
}

fn normalize_optical_slug(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

/// Normalize natural LLM filter aliases and enum spellings onto the reviewed
/// `SearchFilterSchema` vocabulary of an optical dataset, without injecting
/// omitted defaults into the frozen plan when the caller left them unset.
fn normalize_optical_filters(input: &mut Value) {
    let Some(schema) = optical_schema(input) else {
        return;
    };
    let Some(filters) = input.get_mut("filters").and_then(Value::as_object_mut) else {
        return;
    };
    let declared = schema.query_keys();
    if declared.contains(&"cloud_cover_lte") {
        for alias in [
            "cloud_cover_percent",
            "cloud_cover",
            "max_cloud_cover",
            "cloud_cover_max",
            "max_cloud",
            "eo:cloud_cover",
        ] {
            if !declared.contains(&alias) {
                if let Some(val) = filters.remove(alias) {
                    filters.entry("cloud_cover_lte".to_owned()).or_insert(val);
                }
            }
        }
    }
    if declared.contains(&"spacecraft") {
        for alias in ["platform", "platforms", "satellite", "satellites"] {
            if !declared.contains(&alias) {
                if let Some(val) = filters.remove(alias) {
                    filters.entry("spacecraft".to_owned()).or_insert(val);
                }
            }
        }
    }
    if declared.contains(&"processing_levels") {
        for alias in [
            "processing_level",
            "product_level",
            "product_levels",
            "product_type",
            "product_types",
        ] {
            if !declared.contains(&alias) {
                if let Some(val) = filters.remove(alias) {
                    filters.entry("processing_levels".to_owned()).or_insert(val);
                }
            }
        }
    }
    if declared.contains(&"product_type") {
        for alias in ["product_types", "processing_level", "processing_levels"] {
            if !declared.contains(&alias) {
                if let Some(val) = filters.remove(alias) {
                    filters.entry("product_type".to_owned()).or_insert(val);
                }
            }
        }
    }

    for filter in &schema.filters {
        match filter {
            insar_core::SearchFilter::Number { query_key, .. } => {
                if matches!(filters.get(query_key), Some(Value::Null)) {
                    filters.remove(query_key);
                } else if let Some(raw) = filters
                    .get(query_key)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                {
                    let cleaned = raw.trim().trim_end_matches('%').trim();
                    if cleaned.is_empty() {
                        filters.remove(query_key);
                    } else if let Ok(num) = cleaned.parse::<f64>() {
                        if let Some(json_num) = serde_json::Number::from_f64(num) {
                            filters.insert(query_key.clone(), Value::Number(json_num));
                        }
                    }
                }
            }
            insar_core::SearchFilter::MultiEnum {
                query_key, options, ..
            } => {
                if matches!(filters.get(query_key), Some(Value::Null)) {
                    filters.remove(query_key);
                    continue;
                }
                if let Some(single) = filters
                    .get(query_key)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                {
                    if single.trim().is_empty() {
                        filters.remove(query_key);
                        continue;
                    }
                    filters.insert(query_key.clone(), Value::Array(vec![Value::String(single)]));
                }
                let remove_empty =
                    if let Some(Value::Array(items)) = filters.get_mut(query_key) {
                        if items.is_empty() {
                            true
                        } else {
                            let mut normalized: Vec<Value> = Vec::with_capacity(items.len());
                            for item in std::mem::take(items) {
                                if let Some(s) = item.as_str() {
                                    let slug = normalize_optical_slug(s);
                                    if let Some(matched) = options.iter().find(|opt| {
                                        opt.value.eq_ignore_ascii_case(s.trim())
                                            || normalize_optical_slug(&opt.value) == slug
                                            || normalize_optical_slug(&opt.label) == slug
                                            || opt.registry_platform.as_deref().is_some_and(|rp| {
                                                normalize_optical_slug(rp) == slug
                                            })
                                    }) {
                                        if !normalized.iter().any(|existing| {
                                            existing.as_str() == Some(&matched.value)
                                        }) {
                                            normalized.push(Value::String(matched.value.clone()));
                                        }
                                    } else {
                                        normalized.push(item);
                                    }
                                } else {
                                    normalized.push(item);
                                }
                            }
                            *items = normalized;
                            false
                        }
                    } else {
                        false
                    };
                if remove_empty {
                    filters.remove(query_key);
                }
            }
            insar_core::SearchFilter::Fixed {
                query_key: Some(key),
                options,
                ..
            } => {
                if matches!(filters.get(key), Some(Value::Null)) {
                    filters.remove(key);
                } else if let Some(s) = filters.get(key).and_then(Value::as_str).map(str::to_owned)
                {
                    if let Some(opt) = options.first() {
                        if opt.value.eq_ignore_ascii_case(s.trim())
                            || opt.label.eq_ignore_ascii_case(s.trim())
                        {
                            filters.insert(key.clone(), Value::String(opt.value.clone()));
                        }
                    }
                }
            }
            insar_core::SearchFilter::Fixed {
                query_key: None, ..
            }
            | insar_core::SearchFilter::DateRange { .. } => {}
        }
    }
}

/// Validate an optical dataset's filters against its `SearchFilterSchema`
/// (applying the same `Fixed` and `RestoreAll` defaults that
/// `ApplicationService::search_start` supplies) so invalid keys/values are
/// refused at `propose_plan` time rather than failing after approval.
fn optical_filter_problem(input: &Value) -> Option<String> {
    let schema = optical_schema(input)?;
    let mut filters = match input.get("filters") {
        None | Some(Value::Null) => serde_json::Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => return Some("filters: expected an object".to_owned()),
    };
    if let Some(range) = input.get("time_range").filter(|r| {
        r.get("start").is_some_and(Value::is_string) && r.get("end").is_some_and(Value::is_string)
    }) {
        filters.insert("datetime".to_owned(), range.clone());
    }
    for filter in &schema.filters {
        match filter {
            insar_core::SearchFilter::Fixed {
                query_key: Some(key),
                options,
                ..
            } => {
                if let Some(expected) = options.first().map(|opt| opt.value.clone()) {
                    if filters.get(key).is_none_or(Value::is_null) {
                        filters.insert(key.clone(), Value::String(expected));
                    }
                }
            }
            insar_core::SearchFilter::MultiEnum {
                query_key,
                default,
                empty_selection: insar_core::EmptySelection::RestoreAll,
                ..
            } => {
                let needs_default = match filters.get(query_key) {
                    None | Some(Value::Null) => true,
                    Some(Value::Array(items)) => items.is_empty(),
                    _ => false,
                };
                if needs_default && !default.is_empty() {
                    filters.insert(
                        query_key.clone(),
                        Value::Array(default.iter().cloned().map(Value::String).collect()),
                    );
                }
            }
            _ => {}
        }
    }
    schema
        .validate_query_filters(&filters)
        .err()
        .map(|error| format!("filters: {error}"))
}

/// Expected input per plan tool, for a refusal that names no single tool.
fn plan_schemas() -> Value {
    Value::Object(
        catalog::catalog()
            .tools
            .iter()
            .filter_map(|tool| {
                host::plan_input_schema(&tool.tool_id).map(|schema| (tool.tool_id.clone(), schema))
            })
            .collect(),
    )
}

/// Why the host refused a proposed plan. This is never a user decision: a
/// rejection on the desktop card is an `approval` with decision `rejected`.
struct PlanRefusal {
    /// The `HostError` code of the refusal (`AGENT_INVALID_REQUEST`, ...).
    code: &'static str,
    /// One of `steps_invalid`, `tool_forbidden`, `tool_not_plannable`,
    /// `input_invalid`, `aoi_unavailable`, `output_dir_invalid`,
    /// `selection_invalid`, `task_not_found`, `session_gone`,
    /// `store_unavailable`, `record_failed`, `plan_invalid`.
    reason: &'static str,
    step: Option<usize>,
    tool_id: Option<String>,
    detail: Option<String>,
}
impl PlanRefusal {
    fn new(error: HostError, reason: &'static str) -> Self {
        Self {
            code: error.code(),
            reason,
            step: None,
            tool_id: None,
            detail: None,
        }
    }
    fn at(mut self, step: usize, tool_id: &str) -> Self {
        self.step = Some(step);
        self.tool_id = Some(tool_id.to_owned());
        self
    }
    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
    /// One plain sentence for the user (no implementation terms).
    fn message_zh(&self) -> &'static str {
        match self.reason {
            "steps_invalid" => "计划的步骤不完整或数量不对。",
            "tool_forbidden" => "计划里含有不允许执行的操作。",
            "tool_not_plannable" => "计划里放进了一个只需直接查看、不需要确认的操作。",
            "input_invalid" => "计划里的条件格式不对，缺少或多出了内容。",
            "aoi_unavailable" => {
                "计划要用的区域现在不可用：这条消息没有附带区域，或当前没有工作区。"
            }
            "admin_code_unverified" => {
                "计划填写的行政区编码与用户指定的地区不一致，尚未核实真实边界编码。"
            }
            "s1_land_hh_unavailable" => {
                "Sentinel-1 在中低纬度大陆区域仅采集 VV 或 VV+VH 极化，不提供常规 HH 极化。"
            }
            "circuit_breaker_tripped" => {
                "该区域与数据源的同类检索已触发零结果熔断，不能继续重复检索。"
            }
            "output_dir_invalid" => "计划里的下载目录不是有效的本机文件夹。",
            "selection_invalid" => "所选产品不属于这次检索结果，或有重复。",
            "task_not_found" => "计划要操作的任务不存在。",
            "session_gone" => "这个会话已被删除。",
            "store_unavailable" => "本机数据库暂时不可用。",
            "record_failed" => "运行记录写入失败。",
            _ => "计划无法保存。",
        }
    }
    /// How the model can correct the plan, or that it should not retry.
    fn hint_zh(&self) -> String {
        match self.reason {
            "steps_invalid" => {
                format!("steps 需要 1 到 {MAX_PLAN_STEPS} 个步骤，每步有 tool_id 与 input 对象。")
            }
            "tool_forbidden" => "该工具不在目录里或被禁止，任何权限模式下都不会执行，不要再提交。".into(),
            "tool_not_plannable" => "只读工具请直接调用，不要放进 propose_plan。".into(),
            "input_invalid" => {
                "按 expected_input 修正：去掉 unknown_fields，补上 missing_fields；detail 是具体问题。"
                    .into()
            }
            "aoi_unavailable" => {
                "context_aoi 需要用户在消息里附带区域，working_aoi 需要当前有工作区；请先调用 admin_boundary_search 查找区域并在计划中先执行 working_aoi_set，或请用户先选好区域。"
                    .into()
            }
            "admin_code_unverified" => {
                "请勿凭记忆猜测 admin_code，请先调用只读工具 admin_boundary_search 查询目标地区的真实候选编码，再用返回的 admin_code 重新提交 propose_plan。"
                    .into()
            }
            "s1_land_hh_unavailable" => {
                "陆地常规区域请将 polarizations 改为 VV 或 VV+VH（或省略），不要填写 HH 或 HH+HV。"
                    .into()
            }
            "circuit_breaker_tripped" => {
                "请停止修改时间范围重复检索；请直接向用户说明零结果原因，或调用 ask_user_question 提供改搜极化、换数据源或调整区域等选项。"
                    .into()
            }
            // Plain text: a quoted JSON fragment here would be escaped in
            // the nested result and make it unsendable (see `nested_safe`).
            "output_dir_invalid" => {
                "用户消息里有本机目录时，options.output_root 写成只含 ref 的对象、ref 取 message_dir；否则省略 output_root，使用默认下载目录。不要写出任何路径。"
                    .into()
            }
            "selection_invalid" => {
                "item_ids 只能是 batch_page 读到的该批次产品，且不能重复。".into()
            }
            "task_not_found" => "task_id 用 task_overview 读到的任务。".into(),
            _ => "这不是计划内容的问题，不要重复提交；把原因告诉用户。".into(),
        }
    }
    /// The tool result the model receives. Unknown and missing fields are
    /// computed from the model's own step input against its tool's schema.
    fn result(&self, arguments: &Value) -> Value {
        let tool = self.tool_id.as_deref().unwrap_or("");
        let raw = self
            .step
            .and_then(|index| arguments.get("steps")?.get(index)?.get("input"));
        let (unknown, missing) =
            raw.map_or((vec![], vec![]), |raw| host::input_field_diff(tool, raw));
        let expected = host::plan_input_schema(tool).unwrap_or_else(plan_schemas);
        let detail = self
            .detail
            .as_deref()
            .map(|detail| detail.chars().take(MAX_DETAIL_CHARS).collect::<String>());
        // The detail may quote model input; a value that is not trace-safe
        // would make the next request body unsendable, so it is replaced.
        nested_safe(host::sanitize(&json!({
            "error": "plan_refused",
            "code": self.code,
            "reason": self.reason,
            "message_zh": self.message_zh(),
            "hint_zh": self.hint_zh(),
            "step": self.step,
            "tool_id": self.tool_id,
            "unknown_fields": unknown,
            "missing_fields": missing,
            "detail": detail,
            "expected_input": expected,
        })))
    }
}
const MAX_DETAIL_CHARS: usize = 300;

/// The model receives a refusal inside a JSON string, where a quote after a
/// colon is escaped and reads as a local path to the outbound check, so the
/// next request body would be refused and the run would end. Model-derived
/// text is dropped step by step (detail, unknown fields, tool id) until the
/// nested form is safe; the last resort keeps only the host's own fields.
fn nested_safe(mut result: Value) -> Value {
    let safe = |value: &Value| run::safe_value(&Value::String(value.to_string())).is_ok();
    for (field, blank) in [
        ("detail", Value::Null),
        ("unknown_fields", json!([])),
        ("tool_id", Value::Null),
    ] {
        if safe(&result) {
            return result;
        }
        result[field] = blank;
    }
    if safe(&result) {
        return result;
    }
    json!({
        "error": result["error"],
        "code": result["code"],
        "reason": result["reason"],
        "message_zh": result["message_zh"],
    })
}
/// A Store, record or plan error at a refusal point that names no more
/// specific reason.
impl From<HostError> for PlanRefusal {
    fn from(error: HostError) -> Self {
        let reason = match &error {
            HostError::NotFound => "session_gone",
            HostError::StoreUnavailable
            | HostError::Store(_)
            | HostError::Plan(PlanError::Store(_)) => "store_unavailable",
            HostError::Io(_) | HostError::Run(_) | HostError::Json(_) => "record_failed",
            HostError::Forbidden => "tool_forbidden",
            _ => "plan_invalid",
        };
        Self::new(error, reason)
    }
}
impl From<run::RunError> for PlanRefusal {
    fn from(error: run::RunError) -> Self {
        Self::from(HostError::from(error))
    }
}

/// Why a tool call the model made was not carried out (L4 r7 P1-2): a read
/// tool called directly, or an approved plan step. Like `PlanRefusal`, the
/// model gets the reason and what to fix, and the run record a `tool_denied`
/// event.
struct ToolDenial {
    /// `AGENT_INVALID_REQUEST`, `AGENT_TOOL_FORBIDDEN` or `AGENT_RECORD_FAILED`
    /// for the host's own checks; `AGENT_SERVICE_REFUSED` when the service
    /// refused a call that passed them.
    code: &'static str,
    /// One of `tool_unknown`, `tool_needs_plan`, `input_invalid`,
    /// `service_invalid_input`, `account_required`, `service_unavailable`,
    /// `service_refused`, `result_unknown`, `repeated_result`, `not_authorized`
    /// (a step that no longer matches its approved plan), `store_unavailable`,
    /// `record_failed`.
    reason: &'static str,
    tool_id: String,
    detail: Option<String>,
}
const SERVICE_REFUSED: &str = "AGENT_SERVICE_REFUSED";
const RECORD_FAILED: &str = "AGENT_RECORD_FAILED";
impl ToolDenial {
    fn new(code: &'static str, reason: &'static str, tool_id: &str) -> Self {
        Self {
            code,
            reason,
            tool_id: tool_id.to_owned(),
            detail: None,
        }
    }
    /// The host's checks before a read reaches the service: the tool is in
    /// the catalog, may be called directly, and its input is accepted.
    fn before_read(tool_id: &str, input: &Value) -> Option<Self> {
        let invalid = HostError::Invalid.code();
        match catalog::resolve(tool_id) {
            Err(_) => Some(Self::new(
                HostError::Forbidden.code(),
                "tool_unknown",
                tool_id,
            )),
            Ok(tool) if tool.effect != "read" || tool.confirmation != "none" => {
                Some(Self::new(invalid, "tool_needs_plan", tool_id))
            }
            Ok(_) => host::input_problem(tool_id, input).map(|problem| {
                let mut denial = Self::new(invalid, "input_invalid", tool_id);
                denial.detail = Some(problem);
                denial
            }),
        }
    }
    fn of_service(class: ErrorClass, tool_id: &str) -> Self {
        let reason = match class {
            ErrorClass::InvalidParameters => "service_invalid_input",
            ErrorClass::Authentication => "account_required",
            ErrorClass::Transient | ErrorClass::Unsupported => "service_unavailable",
            ErrorClass::ResultUnknown => "result_unknown",
            ErrorClass::Permission | ErrorClass::Business | ErrorClass::Internal => {
                "service_refused"
            }
        };
        Self::new(SERVICE_REFUSED, reason, tool_id)
    }
    fn of_execution(error: &ExecutionError, tool_id: &str, input: &Value) -> Self {
        match error {
            ExecutionError::Service(class) => Self::of_service(*class, tool_id),
            ExecutionError::ResultUnknown => {
                Self::new(HostError::Invalid.code(), "result_unknown", tool_id)
            }
            ExecutionError::Stalled => {
                Self::new(HostError::Invalid.code(), "repeated_result", tool_id)
            }
            ExecutionError::Run(_) | ExecutionError::Io(_) => {
                Self::new(RECORD_FAILED, "record_failed", tool_id)
            }
            ExecutionError::Store(_) => Self::new(
                HostError::StoreUnavailable.code(),
                "store_unavailable",
                tool_id,
            ),
            _ => Self::before_read(tool_id, input)
                .filter(|denial| denial.reason == "input_invalid")
                .unwrap_or_else(|| Self::new(HostError::Invalid.code(), "not_authorized", tool_id)),
        }
    }
    /// One plain sentence for the user.
    fn message_zh(&self) -> &'static str {
        match self.reason {
            "tool_unknown" => "请求的操作不在可用的工具里。",
            "tool_needs_plan" => "这个操作需要先提交计划，不能直接调用。",
            "input_invalid" => "查询条件格式不对，缺少或多出了内容。",
            "service_invalid_input" => "服务不接受这次请求的参数，例如编号、版本号或页码不对。",
            "account_required" => "所需的账号未登录或凭据不可用。",
            "service_unavailable" => "服务暂时不可用，或不支持这个请求。",
            "result_unknown" => "这一步的结果未知，可能已经生效。",
            "repeated_result" => "同一个操作连续返回了相同的结果。",
            "record_failed" => "运行记录写入失败。",
            "not_authorized" => "这一步与批准的计划不一致，或已无法按批准的内容执行。",
            "store_unavailable" => "本机数据库暂时不可用。",
            _ => "服务拒绝了这个请求。",
        }
    }
    /// How the model can correct the call, or that it should not retry.
    fn hint_zh(&self) -> String {
        match (self.reason, self.tool_id.as_str()) {
            ("tool_unknown", _) => {
                "只调用工具列表里的工具；如需切换工作区（working_aoi_set）、启动检索（search_start）、创建或控制任务（create_task、task_action），请放入 propose_plan 的 steps 中提交。"
                    .into()
            }
            ("tool_needs_plan", _) => {
                "检索、建任务与任务控制放进 propose_plan 的步骤，不要直接调用。".into()
            }
            ("input_invalid", _) => {
                "按 expected_input 修正：去掉 unknown_fields，补上 missing_fields；detail 是具体问题。"
                    .into()
            }
            ("service_invalid_input", "batch_page") => {
                "revision 填该批次自己的 revision（workbench_page 或 batch_page 结果里的 revision，冻结批次为 0），不是任务的 revision；page 从 0 开始。"
                    .into()
            }
            ("service_invalid_input", "task_page" | "task_event_page") => {
                "task_id 用 task_overview 读到的任务；page_index 从 0 开始。".into()
            }
            ("service_invalid_input", "search_start") => {
                "filters 的键与取值类型需符合数据源规范：SAR 的 flight_direction 是单个字符串（ASCENDING 或 DESCENDING），processing_levels、beam_modes、polarizations 是字符串数组；光学数据集最大云量用 cloud_cover_lte（0 到 100），平台用 spacecraft 数组。"
                    .into()
            }
            ("service_invalid_input", "create_task") => {
                "batch_id、revision 与 item_ids 用 batch_page 读到的批次与产品。".into()
            }
            ("service_invalid_input", _) => "核对编号与参数后再试一次。".into(),
            ("account_required", _) => {
                "请用户在设置里登录或保存对应账号后再试，不要重复调用。".into()
            }
            ("result_unknown", _) => {
                "用 task_overview 或 task_page 查看任务的实际状态，不要重复提交。".into()
            }
            ("repeated_result", _) => "不要再重复同一个操作；把卡住的原因告诉用户。".into(),
            ("not_authorized", _) => "重新读取当前状态后提交新的计划。".into(),
            _ => "不要立即重复同一个调用；把原因告诉用户。".into(),
        }
    }
    /// The tool result the model receives. Unknown and missing fields are
    /// computed from the model's own input against the read tool's schema.
    fn result(&self, input: &Value) -> Value {
        let input_problem = self.reason == "input_invalid";
        let schema = input_schema(&self.tool_id);
        let (unknown, missing) = match (input_problem, input.as_object()) {
            (true, Some(given)) => {
                let known = schema["properties"]
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                let mut unknown: Vec<&String> = given
                    .keys()
                    .filter(|key| !known.contains_key(*key))
                    .collect();
                let mut missing: Vec<&str> = schema["required"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|field| !given.contains_key(*field))
                    .collect();
                unknown.sort();
                missing.sort();
                (json!(unknown), json!(missing))
            }
            _ => (json!([]), json!([])),
        };
        let detail = self
            .detail
            .as_deref()
            .map(|detail| detail.chars().take(MAX_DETAIL_CHARS).collect::<String>());
        nested_safe(host::sanitize(&json!({
            "error": "tool_denied",
            "code": self.code,
            "reason": self.reason,
            "message_zh": self.message_zh(),
            "hint_zh": self.hint_zh(),
            "tool_id": self.tool_id,
            "unknown_fields": unknown,
            "missing_fields": missing,
            "detail": detail,
            "expected_input": if input_problem { schema } else { Value::Null },
        })))
    }
    /// The run-record event: the fields of the sanitised result, with the
    /// plan step it belongs to when there is one.
    fn event(&self, result: &Value, step: Option<(&str, usize)>) -> Value {
        let mut event = json!({
            "reason": self.reason, "code": self.code,
            "tool_id": result.get("tool_id").cloned().unwrap_or(Value::Null),
            "detail": result.get("detail").cloned().unwrap_or(Value::Null),
            "unknown_fields": result.get("unknown_fields").cloned().unwrap_or(json!([])),
            "missing_fields": result.get("missing_fields").cloned().unwrap_or(json!([])),
        });
        if let Some((plan_id, index)) = step {
            event["plan_id"] = json!(plan_id);
            event["step"] = json!(index);
        }
        event
    }
}

fn tier(confirmation: &str) -> u8 {
    match confirmation {
        "disclose" => 1,
        "plan_confirm" => 2,
        _ => 0,
    }
}

/// Card time range as `{start, end}` strings (or null), whatever key layout
/// the frozen provider query used.
fn time_range(query: &serde_json::Map<String, Value>) -> Value {
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let from_filters_datetime = || -> (Option<String>, Option<String>) {
        let Some(dt) = query.get("filters").and_then(|f| f.get("datetime")) else {
            return (None, None);
        };
        match dt {
            Value::Object(obj) => (
                text(obj.get("start")).or_else(|| text(obj.get("from"))),
                text(obj.get("end")).or_else(|| text(obj.get("to"))),
            ),
            Value::String(raw) => {
                let parts = raw.split_once('/').or_else(|| raw.split_once(".."));
                match parts {
                    Some((s, e)) => (
                        (!s.trim().is_empty()).then(|| s.trim().to_owned()),
                        (!e.trim().is_empty()).then(|| e.trim().to_owned()),
                    ),
                    None if !raw.trim().is_empty() => (Some(raw.trim().to_owned()), None),
                    None => (None, None),
                }
            }
            Value::Array(arr) => (text(arr.first()), text(arr.get(1))),
            _ => (None, None),
        }
    };
    let (start, end) = match query.get("time_range") {
        Some(Value::Object(range)) => (text(range.get("start")), text(range.get("end"))),
        _ => [("start", "end"), ("start_datetime", "end_datetime")]
            .iter()
            .map(|(start, end)| (text(query.get(*start)), text(query.get(*end))))
            .find(|(start, end)| start.is_some() || end.is_some())
            .unwrap_or_else(from_filters_datetime),
    };
    if start.is_none() && end.is_none() {
        Value::Null
    } else {
        json!({"start": start, "end": end})
    }
}

/// Distinct orbit directions recorded on the products a card covers. Empty
/// when none records one: a query filter is not a fact about the products.
fn orbit_directions<'a>(products: impl Iterator<Item = &'a SnapshotItem>) -> Vec<String> {
    fn label(raw: &str) -> Option<String> {
        let raw = raw.trim();
        match raw.to_ascii_uppercase().as_str() {
            "" => None,
            "ASCENDING" => Some("Ascending".to_owned()),
            "DESCENDING" => Some("Descending".to_owned()),
            _ => Some(raw.to_owned()),
        }
    }
    let mut found: Vec<String> = products
        .filter_map(|item| {
            ["flight_direction", "orbit_direction"]
                .iter()
                .find_map(|key| {
                    item.metadata
                        .get(*key)
                        .and_then(Value::as_str)
                        .and_then(label)
                })
        })
        .collect();
    let rank = |direction: &str| match direction {
        "Ascending" => 0,
        "Descending" => 1,
        _ => 2,
    };
    found.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    found.dedup();
    found
}

/// Recorded size of a frozen selection; `None` when any product has none.
/// The service freezes a product's remote size as `payload.file_size_remote`
/// (ASF and STAC assets; L4 r7: reading only metadata keys left every ASF
/// card at size unknown); metadata keys are the older fallback. #22 B slice
/// 2: the products come from the bounded selection read, so this no longer
/// scans the batch's whole candidate array per id.
fn item_size_bytes(c: &SnapshotItem) -> Option<u64> {
    let from_map = |map: &serde_json::Map<String, Value>, keys: &[&str]| {
        keys.iter()
            .find_map(|key| map.get(*key).and_then(Value::as_u64))
    };
    let sum_assets = |assets: &Value| -> Option<u64> {
        let mut total: u64 = 0;
        let mut any = false;
        let read_entry = |entry: &Value| -> Option<u64> {
            let obj = entry.as_object()?;
            from_map(obj, &["size_bytes", "bytes", "file_size", "file:size"])
        };
        match assets {
            Value::Object(map) => {
                for entry in map.values() {
                    if let Some(bytes) = read_entry(entry) {
                        total = total.saturating_add(bytes);
                        any = true;
                    }
                }
            }
            Value::Array(items) => {
                for entry in items {
                    if let Some(bytes) = read_entry(entry) {
                        total = total.saturating_add(bytes);
                        any = true;
                    }
                }
            }
            _ => {}
        }
        any.then_some(total)
    };
    c.payload
        .get("file_size_remote")
        .and_then(Value::as_u64)
        .or_else(|| {
            ["size_bytes", "bytes", "file_size"]
                .iter()
                .find_map(|key| c.metadata.get(*key).and_then(Value::as_u64))
        })
        .or_else(|| {
            ["size_bytes", "bytes", "file_size"]
                .iter()
                .find_map(|key| c.payload.get(*key).and_then(Value::as_u64))
        })
        .or_else(|| c.payload.get("assets").and_then(sum_assets))
        .or_else(|| c.metadata.get("assets").and_then(sum_assets))
}

fn selection_bytes(items: &[SnapshotItem]) -> Option<u64> {
    items.iter().map(item_size_bytes).sum()
}

fn selected_items_preview(items: &[SnapshotItem]) -> Vec<Value> {
    let meta_str = |c: &SnapshotItem, keys: &[&str]| -> Option<String> {
        keys.iter()
            .find_map(|key| {
                c.metadata
                    .get(*key)
                    .or_else(|| c.payload.get(*key))
                    .and_then(Value::as_str)
            })
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let meta_u64 = |c: &SnapshotItem, keys: &[&str]| -> Option<u64> {
        keys.iter().find_map(|key| {
            let v = c.metadata.get(*key).or_else(|| c.payload.get(*key))?;
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        })
    };
    items
        .iter()
        .map(|c| {
            let scene_id = meta_str(c, &["scene_id", "granule_name", "title"])
                .unwrap_or_else(|| c.item_id.clone());
            let acquisition_time = meta_str(
                c,
                &[
                    "acquisition_time",
                    "start_time",
                    "datetime",
                    "acquisition_date",
                ],
            );
            let orbit_direction = meta_str(c, &["orbit_direction", "flight_direction"]);
            let relative_orbit = meta_u64(c, &["relative_orbit", "path_number", "track_number"]);
            json!({
                "scene_id": scene_id,
                "acquisition_time": acquisition_time,
                "orbit_direction": orbit_direction,
                "relative_orbit": relative_orbit,
                "size_bytes": item_size_bytes(c),
            })
        })
        .collect()
}

fn selected_items(input: &Value) -> Vec<&str> {
    input
        .get("item_ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// Products of a frozen selection already in a live task of the same batch,
/// by the create_task admission rule, asked per product (the selection is
/// bounded to 250 by `bounded_input`).
fn queued_count(store: &TaskStore, batch_id: &str, items: &[&str]) -> Result<u64, HostError> {
    let mut queued = 0u64;
    for id in items {
        if store
            .active_task_for_products(batch_id, &[(*id).to_owned()])?
            .is_some()
        {
            queued += 1;
        }
    }
    Ok(queued)
}

/// Store facts of a frozen create_task for the permission gate: whether
/// every selected product has a recorded size, and whether any is already in
/// a live task (create_task would return that task). #22 B slice 2: the
/// batch header plus exactly the selected products — the bounded selection
/// read — never the whole candidate array.
fn create_task_facts(store: &TaskStore, input: &Value) -> Result<(bool, bool), HostError> {
    let batch_id = input.get("batch_id").and_then(Value::as_str).unwrap_or("");
    let items = selected_items(input);
    let selection = store
        .load_batch_selection(
            batch_id,
            &items.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>(),
        )?
        .ok_or(HostError::Invalid)?;
    let selected: std::collections::HashSet<&str> = selection
        .items
        .iter()
        .map(|item| item.item_id.as_str())
        .collect();
    if selected.len() != items.len() {
        return Err(HostError::Invalid);
    }
    Ok((
        selection_bytes(&selection.items).is_some(),
        queued_count(store, &selection.batch.batch_id, &items)? > 0,
    ))
}

fn approval_view(decision: &str, mode: PermissionMode, reasons: &[CardReason]) -> Value {
    json!({"decision": decision, "mode": mode, "card_reasons": reasons})
}

/// The confirm card facts, computed by trusted host code from the Store (never
/// from model text). Approval must echo this exact object back.
/// `output_dir_source` says where a create_task's directory came from. Also
/// returns the frozen provider id and query the card came from (its source,
/// `source_resource_id`), read in the same Store access.
fn card_snapshot(
    store: &TaskStore,
    step: &Value,
    output_dir_source: Option<&str>,
) -> Result<(Value, String, serde_json::Map<String, Value>), HostError> {
    let input = step.get("input").ok_or(HostError::Invalid)?;
    let field = |name: &str| input.get(name).and_then(Value::as_str).unwrap_or("");
    match step.get("tool_id").and_then(Value::as_str) {
        Some("create_task") => {
            let items = selected_items(input);
            // #22 B slice 2: the bounded selection read (header plus the
            // named products) replaces the full candidate array.
            let selection = store
                .load_batch_selection(
                    field("batch_id"),
                    &items.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>(),
                )?
                .ok_or(HostError::Invalid)?;
            let batch = &selection.batch;
            // Same selection rule as create_task admission: no duplicates and
            // only products of the frozen batch, so the card never counts a
            // selection the service would refuse.
            let distinct: std::collections::HashSet<&str> = items.iter().copied().collect();
            if items.is_empty()
                || distinct.len() != items.len()
                || selection.items.len() != items.len()
            {
                return Err(HostError::Invalid);
            }
            let sizes = selection_bytes(&selection.items);
            let queued = queued_count(store, &batch.batch_id, &items)?;
            let selected = selection.items.iter();
            let preview = selected_items_preview(&selection.items);
            let snapshot = json!({
                "source": batch.provider_id,
                "range": {
                    "aoi_name": batch.aoi_name,
                    "area_km2": aoi::area_km2(batch.aoi_geojson.as_ref()),
                    "time_range": time_range(&batch.query),
                    "product": format!("{} 个产品", items.len()),
                },
                "estimated_bytes": sizes,
                "output_dir": input.get("options").and_then(|o| o.get("output_root")).and_then(Value::as_str),
                "output_dir_source": output_dir_source,
                "result_count": items.len(),
                "already_queued_count": queued,
                "orbit_directions": orbit_directions(selected),
                "selected_items_preview": preview,
            });
            Ok((snapshot, batch.provider_id.clone(), batch.query.clone()))
        }
        Some(tool @ ("task_action" | "update_download_task")) => {
            let task = store
                .load_task_record(field("task_id"))?
                .ok_or(HostError::Invalid)?;
            let product = if tool == "task_action" {
                format!("任务操作：{}", field("action"))
            } else {
                "任务状态更新".to_owned()
            };
            let selected = task
                .snapshot
                .candidates
                .iter()
                .filter(|c| task.runtime.selected_item_ids.contains(&c.item_id));
            let snapshot = json!({
                "source": task.snapshot.provider_id,
                "range": {
                    "aoi_name": task.snapshot.aoi_name,
                    "area_km2": aoi::area_km2(task.snapshot.aoi_geojson.as_ref()),
                    "time_range": time_range(&task.snapshot.query),
                    "product": product,
                },
                "estimated_bytes": Value::Null,
                "output_dir": task.snapshot.output_root.to_string_lossy(),
                "result_count": Value::Null,
                "already_queued_count": Value::Null,
                "orbit_directions": orbit_directions(selected),
                "selected_items_preview": [],
            });
            Ok((snapshot, task.snapshot.provider_id, task.snapshot.query))
        }
        _ => Err(HostError::Invalid),
    }
}

/// The registry resource a card's frozen batch or task came from, by the
/// same rule as the service's source list (unified_sources.rs
/// `source_for_route`): explicit `resource_id`, a logical dataset catalogue
/// source, a SAR source by provider and selector (`query.dataset`), or
/// matching `provider_route_id` / `native_collection_id`.
fn source_resource_id(provider: &str, query: &serde_json::Map<String, Value>) -> Option<String> {
    let registry = insar_core::ResourceRegistry::embedded_ref().ok()?;
    let text = |key: &str| {
        query
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    if let Some(explicit_id) = text("resource_id") {
        if let Some(res) = registry.resource(explicit_id) {
            return Some(res.common().resource_id.clone());
        }
    }
    let by_route_or_dataset = || {
        let provider_candidates: &[&str] = match provider {
            "asf.nasa.earthdata" => &[provider, "asf.catalog"],
            "asf.catalog" => &[provider, "asf.nasa.earthdata"],
            "mpc.stac" => &[provider, "microsoft.planetary-computer"],
            "microsoft.planetary-computer" => &[provider, "mpc.stac"],
            _ => &[provider],
        };
        text("dataset")
            .and_then(|dataset| {
                provider_candidates
                    .iter()
                    .find_map(|p| registry.resource_for_provider_selector(p, dataset))
                    .or_else(|| {
                        registry.resources.iter().find(|res| {
                            let insar_core::ResourceDescriptor::ScienceDataset { science, common } =
                                res
                            else {
                                return false;
                            };
                            !matches!(
                                common.lifecycle,
                                insar_core::StaticCapabilityLifecycle::Reserved
                                    | insar_core::StaticCapabilityLifecycle::Retired
                            ) && science
                                .sar_search
                                .as_ref()
                                .is_some_and(|s| s.selector.eq_ignore_ascii_case(dataset))
                        })
                    })
            })
            .or_else(|| {
                let route_id = text("provider_route_id");
                let native_col = text("native_collection_id");
                if route_id.is_none() && native_col.is_none() {
                    return None;
                }
                registry.resources.iter().find(|res| {
                    res.common().provider_routes.iter().any(|route| {
                        route_id.is_some_and(|id| route.route_id == id)
                            || (provider_candidates.contains(&route.provider_id.as_str())
                                && native_col.is_some_and(|col| {
                                    route.native_collection_id.as_deref() == Some(col)
                                }))
                    })
                })
            })
    };
    let resource = match text("logical_dataset") {
        Some(logical) => text("native_collection_id")
            .and_then(|native_col| {
                registry
                    .catalogue_search_resource_for_source(logical, provider, native_col)
                    .ok()
                    .flatten()
            })
            .or_else(|| {
                registry.resources.iter().find(|res| {
                    let insar_core::ResourceDescriptor::ScienceDataset { science, common } = res
                    else {
                        return false;
                    };
                    if matches!(
                        common.lifecycle,
                        insar_core::StaticCapabilityLifecycle::Reserved
                            | insar_core::StaticCapabilityLifecycle::Retired
                    ) {
                        return false;
                    }
                    science.catalogue_logical_dataset.as_deref() == Some(logical)
                        || science
                            .sar_search
                            .as_ref()
                            .is_some_and(|s| s.logical_dataset.as_deref() == Some(logical))
                        || common.resource_id == logical
                })
            })
            .or_else(by_route_or_dataset),
        None => by_route_or_dataset(),
    }?;
    Some(resource.common().resource_id.clone())
}

/// Products a search tool's (trace-safe) result returned: the items of a batch
/// or workbench page, or the products a finished search kept. `None` for any
/// other tool and for a search that has not finished.
fn result_count(tool_id: &str, summary: &Value) -> Option<u64> {
    match tool_id {
        "batch_page" | "workbench_page" => summary
            .get("items")
            .and_then(Value::as_array)
            .map(|items| items.len() as u64),
        "search_progress" if summary.get("stage").and_then(Value::as_str) == Some("done") => {
            summary.get("items_kept").and_then(Value::as_u64)
        }
        _ => None,
    }
}

fn clip_summary_text(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{collected}…")
    } else {
        collected
    }
}

fn is_cacheable_read_tool(tool_id: &str) -> bool {
    matches!(
        tool_id,
        "skill_read"
            | "gee_catalog_search"
            | "gee_search_catalog"
            | "gee_dataset_advisor"
            | "gee_export_preflight"
            | "gee_check_availability"
            | "source_list"
            | "service_readiness"
            | "admin_boundary_search"
            | "working_aoi_get"
            | "gis_raster_inspect"
            | "global_disaster_search"
            | "aoi_validate_boundary"
    )
}

fn canonical_json_for_cache(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by_key(|(a, _)| *a);
            let body = entries
                .into_iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical_json_for_cache(v)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        Value::Array(items) => {
            let body = items
                .iter()
                .map(canonical_json_for_cache)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{body}]")
        }
        _ => value.to_string(),
    }
}

fn tool_target_label(tool_id: &str, args: &Value, summary: Option<&Value>) -> Option<String> {
    let safe_args = host::sanitize(args);
    let pick_str = |key: &str| -> Option<String> {
        safe_args
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "[local]")
            .map(str::to_owned)
    };
    let raw = match tool_id {
        "skill_read" => {
            let sid = pick_str("skill_id").or_else(|| pick_str("id"));
            let path = pick_str("file_path").or_else(|| pick_str("path"));
            let query = pick_str("query");
            match (sid, path, query) {
                (Some(s), Some(p), _) => Some(format!("{s} / {p}")),
                (Some(s), None, Some(q)) => Some(format!("{s} · 搜索 {q}")),
                (Some(s), None, None) => Some(s),
                (None, Some(p), _) => Some(p),
                (None, None, Some(q)) => Some(format!("搜索 {q}")),
                _ => None,
            }
        }
        "gee_catalog_search" | "gee_search_catalog" => {
            pick_str("query").or_else(|| pick_str("category"))
        }
        "gee_dataset_advisor" => pick_str("task")
            .or_else(|| pick_str("asset_id"))
            .or_else(|| pick_str("mode")),
        "gee_export_preflight"
        | "gee_check_availability"
        | "gee_cloud_analyze"
        | "gee_chunked_download"
        | "gee_download"
        | "gee_script_inspect" => pick_str("asset_id")
            .or_else(|| pick_str("dataset_id"))
            .or_else(|| pick_str("dataset"))
            .or_else(|| pick_str("collection"))
            .or_else(|| pick_str("product")),
        "nisar_subset_preflight" | "nisar_create_subset_task" => pick_str("product_level")
            .or_else(|| pick_str("granule_id"))
            .or_else(|| pick_str("item_id"))
            .or_else(|| pick_str("batch_id"))
            .or_else(|| pick_str("dataset")),
        "toolset_activate" => pick_str("toolset")
            .or_else(|| pick_str("category"))
            .or_else(|| {
                safe_args
                    .get("toolsets")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|s| !s.is_empty())
            }),
        "capabilities_get" => pick_str("resource_id").or_else(|| pick_str("provider")),
        "service_readiness" | "account_readiness" | "credential_readiness" => {
            pick_str("provider").or_else(|| pick_str("service"))
        }
        "search_start" => pick_str("resource_id")
            .or_else(|| pick_str("source_id"))
            .or_else(|| pick_str("product")),
        "search_progress" => pick_str("request_id").or_else(|| {
            summary
                .and_then(|s| s.get("resource_id").or_else(|| s.get("source_id")))
                .and_then(Value::as_str)
                .map(str::to_owned)
        }),
        "batch_page" | "workbench_page" => pick_str("batch_id"),
        "admin_boundary_search" => pick_str("query").or_else(|| pick_str("name")),
        "global_disaster_search" => pick_str("keyword").or_else(|| pick_str("min_alert_level")),
        "aoi_validate_boundary" => safe_args
            .get("bbox")
            .and_then(Value::as_array)
            .filter(|a| a.len() == 4)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_f64)
                    .map(|n| format!("{n:.2}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|s| !s.is_empty()),
        "working_aoi_set" => pick_str("label")
            .or_else(|| pick_str("name"))
            .or_else(|| pick_str("admin_code")),
        "task_detail" | "task_action" | "task_event_page" => pick_str("task_id"),
        "task_page" => pick_str("status").or_else(|| pick_str("bucket")),
        "map_workspace_action" | "goal_manifest" => pick_str("action"),
        "gis_raster_inspect" | "gis_local_process" | "gis_external_execute" => {
            pick_str("operation")
                .or_else(|| pick_str("op"))
                .or_else(|| pick_str("dataset"))
        }
        _ => pick_str("target")
            .or_else(|| pick_str("dataset_id"))
            .or_else(|| pick_str("resource_id"))
            .or_else(|| pick_str("query"))
            .or_else(|| pick_str("name"))
            .or_else(|| pick_str("task_id"))
            .or_else(|| pick_str("batch_id")),
    };
    raw.map(|s| clip_summary_text(&history::outbound_text(&s), 80))
        .filter(|s| !s.is_empty())
}

fn format_compact_value(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => {
            let cleaned = history::outbound_text(s);
            let trimmed = cleaned.trim();
            (!trimmed.is_empty()).then(|| clip_summary_text(trimmed, 120))
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                return Some("[]".to_owned());
            }
            if arr.len() <= 6
                && arr
                    .iter()
                    .all(|item| item.is_string() || item.is_number() || item.is_boolean())
            {
                let items: Vec<String> = arr
                    .iter()
                    .map(|item| match item {
                        Value::String(s) => clip_summary_text(&history::outbound_text(s), 40),
                        other => other.to_string(),
                    })
                    .collect();
                Some(format!("[{}]", items.join(", ")))
            } else {
                Some(format!("[{} 项]", arr.len()))
            }
        }
        other => Some(clip_summary_text(&other.to_string(), 120)),
    }
}

fn tool_input_summary(args: &Value) -> Option<String> {
    let safe = host::sanitize(args);
    let obj = safe.as_object()?;
    if obj.is_empty() {
        return Some("无额外参数".to_owned());
    }
    let mut parts = Vec::new();
    for (k, v) in obj {
        let Some(val_str) = format_compact_value(v) else {
            continue;
        };
        parts.push(format!("{k}: {val_str}"));
    }
    if parts.is_empty() {
        Some("无额外参数".to_owned())
    } else {
        Some(clip_summary_text(&parts.join(" · "), 420))
    }
}

fn format_bytes_short(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn tool_output_summary(tool_id: &str, summary: &Value) -> Option<String> {
    let safe = host::sanitize(summary);
    let cached = safe.get("cached_same_run") == Some(&Value::Bool(true));

    // Surface concrete error / denial diagnostics first so failed tool rows
    // always explain why they failed instead of rendering empty.
    let failed_flag = safe.get("ok") == Some(&Value::Bool(false))
        || safe.get("error").is_some_and(|v| !v.is_null());
    if failed_flag {
        let reason = safe
            .get("reason")
            .or_else(|| safe.get("error"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let msg = safe
            .get("message_zh")
            .or_else(|| safe.get("message"))
            .or_else(|| safe.get("detail"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|m| !m.is_empty() && *m != "[local]");
        let combined = match (reason, msg) {
            (Some(r), Some(m)) => format!("{r}：{m}"),
            (None, Some(m)) => m.to_owned(),
            (Some(r), None) => format!("执行未通过：{r}"),
            (None, None) => "工具调用未成功".to_owned(),
        };
        let clean = clip_summary_text(&history::outbound_text(&combined), 420);
        return (!clean.is_empty()).then_some(clean);
    }

    let body = match tool_id {
        "skill_read" => {
            let sid = safe
                .get("skill_id")
                .and_then(Value::as_str)
                .unwrap_or("skill");
            if let Some(files) = safe.get("files").and_then(Value::as_array) {
                let names: Vec<String> = files
                    .iter()
                    .filter_map(|f| f.get("path").and_then(Value::as_str))
                    .take(4)
                    .map(history::outbound_text)
                    .collect();
                let total_bytes: u64 = files
                    .iter()
                    .filter_map(|f| f.get("bytes").and_then(Value::as_u64))
                    .sum();
                if names.is_empty() {
                    format!("{sid}: 共 {} 个文件", files.len())
                } else {
                    format!(
                        "{sid}: {}（共 {} 个文件, {} B）",
                        names.join(", "),
                        files.len(),
                        total_bytes
                    )
                }
            } else if let Some(matches) = safe.get("matches").and_then(Value::as_array) {
                format!("{sid}: 命中 {} 处匹配片段", matches.len())
            } else {
                let path = safe
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("SKILL.md");
                let bytes = safe.get("bytes").and_then(Value::as_u64);
                match bytes {
                    Some(b) => format!("{sid}/{path} ({b} B)"),
                    None => format!("{sid}/{path}"),
                }
            }
        }
        "source_list" => {
            if let Some(arr) = safe.as_array().or_else(|| {
                safe.get("sources")
                    .or_else(|| safe.get("resources"))
                    .and_then(Value::as_array)
            }) {
                let names: Vec<String> = arr
                    .iter()
                    .filter_map(|item| {
                        item.get("display_name_zh")
                            .or_else(|| item.get("display_name"))
                            .or_else(|| item.get("name"))
                            .or_else(|| item.get("resource_id"))
                            .and_then(Value::as_str)
                    })
                    .take(4)
                    .map(history::outbound_text)
                    .collect();
                if names.is_empty() {
                    format!("共 {} 个可用数据源", arr.len())
                } else {
                    let suffix = if arr.len() > names.len() { " 等" } else { "" };
                    format!("共 {} 个数据源：{}{suffix}", arr.len(), names.join("、"))
                }
            } else {
                String::new()
            }
        }
        "task_overview" => {
            let queue_len = safe.get("queue").and_then(Value::as_array).map(Vec::len);
            let history_len = safe.get("history").and_then(Value::as_array).map(Vec::len);
            let total_queue = safe
                .get("queue_total")
                .and_then(Value::as_u64)
                .or_else(|| queue_len.map(|n| n as u64));
            let total_history = safe
                .get("history_total")
                .and_then(Value::as_u64)
                .or_else(|| history_len.map(|n| n as u64));
            let active_count = safe
                .get("queue")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter(|t| {
                            matches!(
                                t.get("status").and_then(Value::as_str),
                                Some("running" | "queued" | "resuming")
                            )
                        })
                        .count()
                })
                .unwrap_or(0);
            match (total_queue, total_history) {
                (Some(q), Some(h)) if active_count > 0 => {
                    format!("下载队列 {q} 项（活跃 {active_count} 项），历史记录 {h} 项")
                }
                (Some(q), Some(h)) => format!("下载队列 {q} 项，历史记录 {h} 项"),
                _ => String::new(),
            }
        }
        "working_aoi_get" => {
            let state = safe.get("state").and_then(Value::as_str).unwrap_or("none");
            if state == "none" {
                "当前未设置工作区 AOI".to_owned()
            } else {
                let name = safe
                    .get("display_name")
                    .or_else(|| safe.get("name"))
                    .and_then(Value::as_str)
                    .map(history::outbound_text)
                    .unwrap_or_else(|| "工作区".to_owned());
                if let Some(area) = safe.get("area_km2").and_then(Value::as_f64) {
                    format!("当前工作区：{name}（约 {:.1} km²）", area)
                } else {
                    format!("当前工作区：{name}")
                }
            }
        }
        "admin_boundary_search" => {
            let items = safe
                .get("candidates")
                .or_else(|| safe.get("items"))
                .and_then(Value::as_array);
            if let Some(arr) = items {
                let top: Vec<String> = arr
                    .iter()
                    .take(3)
                    .filter_map(|c| {
                        let name = c
                            .get("display_name")
                            .or_else(|| c.get("name"))
                            .and_then(Value::as_str)?;
                        let code = c
                            .get("admin_code")
                            .or_else(|| c.get("code"))
                            .and_then(Value::as_str);
                        Some(match code {
                            Some(cd) => format!("{} ({})", history::outbound_text(name), cd),
                            None => history::outbound_text(name),
                        })
                    })
                    .collect();
                if top.is_empty() {
                    format!("找到 {} 个匹配行政区", arr.len())
                } else {
                    format!("找到 {} 个行政区：{}", arr.len(), top.join("、"))
                }
            } else {
                String::new()
            }
        }
        "workbench_page" | "batch_page" => {
            let total = safe
                .get("total")
                .or_else(|| safe.get("total_count"))
                .and_then(Value::as_u64);
            let items = safe
                .get("items")
                .or_else(|| safe.get("products"))
                .and_then(Value::as_array)
                .map(Vec::len);
            let batches = safe.get("batches").and_then(Value::as_array).map(Vec::len);
            match (items, total, batches) {
                (Some(n), Some(t), _) => format!("本页返回 {n} 条（共 {t} 条）"),
                (Some(n), None, _) => format!("本页返回 {n} 条产品"),
                (None, Some(t), Some(b)) => format!("返回 {b} 个批次（共 {t} 个）"),
                (None, None, Some(b)) => format!("返回 {b} 个检索批次"),
                _ => String::new(),
            }
        }
        "task_page" | "task_event_page" => {
            let items = safe
                .get("items")
                .or_else(|| safe.get("events"))
                .or_else(|| safe.get("tasks"))
                .and_then(Value::as_array)
                .map(Vec::len);
            let total = safe
                .get("total")
                .or_else(|| safe.get("total_count"))
                .and_then(Value::as_u64);
            match (items, total) {
                (Some(n), Some(t)) => format!("本页返回 {n} 条（共 {t} 条）"),
                (Some(n), None) => format!("本页返回 {n} 条记录"),
                _ => String::new(),
            }
        }
        "service_readiness" | "account_readiness" | "credential_readiness" => {
            if let Some(providers) = safe
                .get("providers")
                .or_else(|| safe.get("accounts"))
                .and_then(Value::as_object)
            {
                let mut ready = 0usize;
                for v in providers.values() {
                    if v.get("ready") == Some(&Value::Bool(true))
                        || v.get("authenticated") == Some(&Value::Bool(true))
                        || v.get("configured") == Some(&Value::Bool(true))
                        || v.as_str() == Some("ready")
                    {
                        ready += 1;
                    }
                }
                format!("共检测 {} 项服务/账号（就绪 {ready} 项）", providers.len())
            } else if let Some(arr) = safe
                .get("providers")
                .or_else(|| safe.get("services"))
                .or_else(|| safe.get("accounts"))
                .and_then(Value::as_array)
            {
                let ready = arr
                    .iter()
                    .filter(|v| {
                        v.get("ready") == Some(&Value::Bool(true))
                            || v.get("authenticated") == Some(&Value::Bool(true))
                            || v.get("configured") == Some(&Value::Bool(true))
                    })
                    .count();
                format!("共检测 {} 项服务/账号（就绪 {ready} 项）", arr.len())
            } else {
                String::new()
            }
        }
        "toolset_activate" => {
            let activated = safe
                .get("activated")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            if activated.is_empty() {
                "已更新激活工具包".to_owned()
            } else {
                format!("已解锁工具包：{activated}")
            }
        }
        "nisar_subset_preflight" => {
            let eligible = safe
                .get("eligible_count")
                .and_then(Value::as_u64)
                .or_else(|| {
                    safe.get("eligible_items")
                        .and_then(Value::as_array)
                        .map(|a| a.len() as u64)
                });
            let geom = safe.get("geometry_mode").and_then(Value::as_str);
            let sub_b = safe.get("estimated_subset_bytes").and_then(Value::as_u64);
            let full_b = safe.get("estimated_full_bytes").and_then(Value::as_u64);
            let mut segs = Vec::new();
            if let Some(n) = eligible {
                segs.push(format!("命中 {n} 景可裁剪产品"));
            }
            if let Some(g) = geom {
                segs.push(format!("几何模式: {g}"));
            }
            if let (Some(s), Some(f)) = (sub_b, full_b) {
                segs.push(format!(
                    "预估 {} / 原始 {}",
                    format_bytes_short(s),
                    format_bytes_short(f)
                ));
            }
            segs.join(" · ")
        }
        _ => String::new(),
    };

    let body = if !body.is_empty() {
        body
    } else if let Some(msg) = safe
        .get("message_zh")
        .or_else(|| safe.get("summary_zh"))
        .or_else(|| safe.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty() && *m != "[local]")
    {
        clip_summary_text(&history::outbound_text(msg), 380)
    } else if let Some(obj) = safe.as_object() {
        let mut parts = Vec::new();
        for (k, v) in obj {
            if k == "cached_same_run" || k == "content" || k == "script_preview" || v.is_null() {
                continue;
            }
            let rendered = match v {
                Value::Array(arr) => {
                    if arr.len() <= 4
                        && arr
                            .iter()
                            .all(|x| x.is_string() || x.is_number() || x.is_boolean())
                    {
                        let joined = arr
                            .iter()
                            .map(|x| match x {
                                Value::String(s) => {
                                    clip_summary_text(&history::outbound_text(s), 40)
                                }
                                other => other.to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("[{joined}]")
                    } else {
                        format!("[{} 项]", arr.len())
                    }
                }
                Value::Object(inner) => format!("{{{}}}", inner.len()),
                Value::String(s) => clip_summary_text(&history::outbound_text(s), 100),
                other => other.to_string(),
            };
            parts.push(format!("{k}: {rendered}"));
            if parts.len() >= 8 {
                break;
            }
        }
        if parts.is_empty() {
            return None;
        }
        clip_summary_text(&parts.join(" · "), 420)
    } else {
        clip_summary_text(&safe.to_string(), 320)
    };
    let clean = clip_summary_text(&body, 420);
    if clean.is_empty() {
        None
    } else if cached {
        Some(format!("复用本轮相同参数缓存 · {clean}"))
    } else {
        Some(clean)
    }
}

fn is_allowlisted_open_geodata_agent_dataset(dataset: &str) -> bool {
    matches!(
        dataset.trim().to_ascii_lowercase().as_str(),
        "aws_sentinel1_grd"
            | "sentinel1_grd_aws"
            | "noaa_aws_weather"
            | "noaa_gfs"
            | "noaa_goes"
            | "overture_maps"
            | "overture"
            | "open_dem_tiles"
            | "dem_tiles"
            | "whu_clcd"
            | "tum_eot20"
            | "gebco_2026"
            | "noaa_etopo_2022"
    )
}

/// Extension tools that trigger real downloads / task-creation side effects and
/// MUST obey the session's permission mode (`ConfirmChanges` never auto-starts).
/// For `open_geodata_query`, enforce deny-by-default: ONLY allowlisted read-only
/// open-data catalog datasets without any download/output flags skip the confirm gate.
fn open_geodata_query_has_side_effect(args: &Value) -> bool {
    let dataset = args.get("dataset").and_then(Value::as_str).unwrap_or("");
    if !is_allowlisted_open_geodata_agent_dataset(dataset) {
        return true;
    }
    let has_output_path = ["output_path", "output_root", "output_dir"]
        .iter()
        .any(|k| {
            args.get(*k)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
        })
        || args.get("gee_params").is_some_and(|gp| {
            ["output_path", "output_dir", "output_root"]
                .iter()
                .any(|k| {
                    gp.get(*k)
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty())
                })
        });
    if has_output_path {
        return true;
    }
    if args
        .get("tool_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|t| {
            !t.is_empty() && !t.eq_ignore_ascii_case("search") && !t.eq_ignore_ascii_case("preview")
        })
    {
        return true;
    }
    if args
        .get("keyword")
        .and_then(Value::as_str)
        .is_some_and(|kw| {
            kw.split(['|', ',', ';'])
                .any(|tok| tok.trim().eq_ignore_ascii_case("download"))
        })
    {
        return true;
    }
    args.get("gee_params")
        .and_then(|v| v.get("download"))
        .and_then(Value::as_bool)
        == Some(true)
}

fn is_side_effect_download_extension_tool(tool_id: &str, args: &Value) -> bool {
    match tool_id {
        "nisar_create_subset_task" | "gee_chunked_download" | "gee_download" => true,
        "open_geodata_query" => open_geodata_query_has_side_effect(args),
        _ => false,
    }
}

fn is_side_effect_local_execution_tool(tool_id: &str) -> bool {
    matches!(tool_id, "gis_external_execute")
}

fn is_forbidden_local_exec_output_key(key: &str) -> bool {
    let lower = key.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "output"
            | "output_path"
            | "output_dir"
            | "output_root"
            | "output_file"
            | "output_folder"
            | "output_raster"
            | "output_rasterdataset"
            | "output_vector"
            | "output_layer"
            | "output_table"
            | "output_dataset"
            | "output_feature_class"
            | "output_workspace"
            | "output_location"
            | "output_name"
            | "out_feature_class"
            | "out_raster"
            | "out_rasterdataset"
            | "out_raster_dataset"
            | "out_dataset"
            | "out_table"
            | "out_file"
            | "out_folder"
            | "out_workspace"
            | "out_path"
            | "out_name"
            | "out_dir"
            | "out_location"
            | "raster_dataset_name_with_extension"
            | "out_layer"
            | "out_polygon_features"
            | "out_point_features"
            | "out_polyline_features"
            | "out_fc"
            | "out_shp"
            | "out_tif"
            | "out_tiff"
            | "out_geojson"
    )
}

fn local_execution_has_unverified_output_override(args: &Value) -> bool {
    let bad_filename = args
        .get("output_filename")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|s| {
            !s.is_empty() && (s.contains('/') || s.contains('\\') || s.contains(':'))
        });
    let bad_param = args
        .get("params")
        .and_then(Value::as_object)
        .is_some_and(|obj| obj.keys().any(|k| is_forbidden_local_exec_output_key(k)));
    bad_filename || bad_param
}

fn is_resume_prompt(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("继续刚才中断的任务") || trimmed.starts_with("继续刚才的任务")
    {
        return true;
    }
    let normalized = trimmed
        .trim_end_matches(['。', '.', '!', '！', '~', '，', ','])
        .trim()
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "继续"
            | "请继续"
            | "接着继续"
            | "继续吧"
            | "继续刚才"
            | "继续刚才的分析"
            | "继续上次"
            | "continue"
            | "resume"
            | "go on"
            | "keep going"
    )
}

fn extract_completed_tool_steps(messages: &[Message]) -> Vec<CompletedToolStep> {
    let mut call_meta: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut steps = Vec::new();
    for msg in messages {
        if msg.role == "assistant_tool_calls" {
            if let Ok(payload) = serde_json::from_str::<Value>(&msg.content) {
                if let Some(calls) = payload.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        let Some(id) = call.get("id").and_then(Value::as_str) else {
                            continue;
                        };
                        let name = call
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_owned();
                        let input_summary = call.get("arguments").and_then(tool_input_summary);
                        call_meta.insert(id.to_owned(), (name, input_summary));
                    }
                }
            }
        } else if msg.role == "tool" {
            if let Ok(payload) = serde_json::from_str::<Value>(&msg.content) {
                let Some(id) = payload.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let (tool_id, input_summary) = call_meta.get(id).cloned().unwrap_or_else(|| {
                    (
                        payload
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_owned(),
                        None,
                    )
                });
                let result = payload.get("result").unwrap_or(&Value::Null);
                let ok =
                    result.get("error").is_none() && result.get("ok") != Some(&Value::Bool(false));
                let output_summary = tool_output_summary(&tool_id, result);
                steps.push(CompletedToolStep {
                    call_id: id.to_owned(),
                    tool_id,
                    input_summary,
                    output_summary,
                    ok,
                });
            }
        }
    }
    steps
}

fn format_resume_checkpoint_block(cp: &ResumeCheckpoint) -> String {
    let payload = json!({
        "original_user_goal": history::outbound_text(&cp.original_user_goal),
        "completed_tool_steps": cp.completed_tool_steps,
        "answer_delivered": cp.answer_delivered,
        "authorized_actions": cp.authorized_actions,
        "interrupt_reason": cp.interrupt_reason,
    });
    let body = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_owned());
    format!(
        "<resume_checkpoint>\n{body}\n</resume_checkpoint>\n\
         [续跑指示] 上一轮在交付最终回答前中断。请直接围绕 original_user_goal 并复用 completed_tool_steps 中已核实的结果继续完成或直接交付最终结论；严禁重复调用已完成的只读检查工具，也严禁在 authorized_actions 为空时新建任何检索或下载任务。"
    )
}

/// The service's own sentence about a result (search progress `message_zh`);
/// no other tool result carries one, and none is composed here.
fn summary_zh(tool_id: &str, summary: &Value) -> Option<String> {
    if tool_id != "search_progress" {
        return None;
    }
    summary
        .get("message_zh")
        .and_then(Value::as_str)
        .map(str::trim)
        // `[local]` is the sanitizer's marker for a value that held a path.
        .filter(|message| !message.is_empty() && *message != "[local]")
        .map(str::to_owned)
}

/// The run-record event a shown reference names: the later of the last
/// `tool_result` whose summary carries this task or batch id and, for a task,
/// the last `execution` event naming it; with that event's result count when
/// it was a search read. Web and unknown references name no event.
fn evidence_step(events: &[TraceEvent], kind: &str, id: &str) -> (Option<u64>, Option<u64>) {
    let key = match kind {
        "task" => "task_id",
        "batch" => "search_batch_id",
        _ => return (None, None),
    };
    events
        .iter()
        .rev()
        .find_map(|event| {
            let names = |value: Option<&Value>| {
                value.and_then(|v| v.get(key)).and_then(Value::as_str) == Some(id)
            };
            match event.kind.as_str() {
                "tool_result" if names(event.payload.get("summary")) => {
                    let count = event
                        .payload
                        .get("tool_id")
                        .and_then(Value::as_str)
                        .zip(event.payload.get("summary"))
                        .and_then(|(tool, summary)| result_count(tool, summary));
                    Some((Some(event.sequence), count))
                }
                "execution" if kind == "task" && names(Some(&event.payload)) => {
                    Some((Some(event.sequence), None))
                }
                _ => None,
            }
        })
        .unwrap_or((None, None))
}

/// Model reference strings map to the W3a citation kinds; `web:` has no local
/// evidence and is always reported unverified.
fn reference(raw: &str) -> (citation::Reference, Value) {
    // A bare id the host minted names its kind by its prefix.
    let (kind, id) = match raw.split_once(':') {
        Some(split) => split,
        None if raw.starts_with("task-") => ("task", raw),
        None if raw.starts_with("batch-") => ("batch", raw),
        None => ("", raw),
    };
    let (citation_kind, w4_kind, label) = match kind {
        "task" => ("task_id", "task", format!("任务 {id}")),
        "batch" => ("search_batch_id", "batch", format!("批次 {id}")),
        "web" => ("web", "web", id.to_owned()),
        _ => ("unknown", "web", raw.to_owned()),
    };
    (
        citation::Reference {
            kind: citation_kind.into(),
            id: id.into(),
            sequence: None,
        },
        json!({"kind": w4_kind, "id": id, "label": label}),
    )
}

/// What a create_task result tells the model about its directory, by the
/// frozen plan's `output_dir_source` (after any change on the card).
fn output_dir_note_zh(source: Option<&str>) -> &'static str {
    match source {
        Some("from_message") => "下载到用户在消息里指定的目录。",
        Some("chosen_on_card") => "下载目录由用户在确认卡上选择，不是用户消息里指定的目录。",
        Some("default") => "下载到全局默认下载目录，不是用户消息里指定的目录。",
        Some("model_suggested") => {
            "下载到计划里建议、用户在确认卡上确认的目录，不是用户消息里指定的目录。"
        }
        _ => "下载目录来源未知，不要说成用户在消息里指定的目录。",
    }
}

/// What a plan that did not run (rejected on the card, or expired) tells
/// the model. With a create_task step it also says that no task was created
/// and where the plan's directory came from (L4 r10 P2-5: after a rejected
/// plan whose `message_dir` named no directory, the model said the
/// directory was the one the user gave).
fn unexecuted_plan_result(plan_id: &str, status: &str, plan_json: &str) -> Value {
    let mut result = json!({"plan_id": plan_id, "status": status});
    let intent: Value = serde_json::from_str(plan_json).unwrap_or(Value::Null);
    let creates = intent
        .get("steps")
        .and_then(Value::as_array)
        .is_some_and(|steps| {
            steps
                .iter()
                .any(|step| step.get("tool_id").and_then(Value::as_str) == Some("create_task"))
        });
    if creates {
        let source = intent
            .pointer("/card_snapshot/output_dir_source")
            .and_then(Value::as_str);
        result["output_dir_source"] = json!(source);
        result["output_dir_note_zh"] = json!(unexecuted_dir_note_zh(source));
    }
    result
}

/// The directory sentence for a create_task plan that did not run.
fn unexecuted_dir_note_zh(source: Option<&str>) -> String {
    let directory = match source {
        Some("from_message") => "计划里的下载目录是用户在消息里指定的目录。",
        Some("chosen_on_card") => {
            "计划里的下载目录是用户在确认卡上选择的目录，不是用户消息里指定的目录。"
        }
        Some("default") => "计划里的下载目录是全局默认下载目录，不是用户消息里指定的目录。",
        Some("model_suggested") => "计划里的下载目录是你建议的目录，不是用户消息里指定的目录。",
        _ => "计划里没有确定的下载目录，不要说成用户在消息里指定的目录。",
    };
    format!("计划没有执行，没有建任务，也没有开始下载。{directory}")
}

/// The user's sentence for a code that ends a run by the host's own decision
/// (L4 r7 P1-4: these all read as 「模型服务拒绝了该请求」 before); `None` for
/// provider codes, which the desktop explains by code.
fn host_error_zh(code: &str) -> Option<&'static str> {
    Some(match code {
        "outbound_blocked" => {
            "请求里有无法安全发出的内容（如本机路径或凭据样式的文字），没有发给模型服务，本次运行已停止。"
        }
        "step_limit" => {
            "本次运行用完了模型调用次数仍未得出结论，已停止；可以把问题拆小或说得更具体后再试。"
        }
        "repeated_tool_call" => "模型连续两次发出了完全相同的工具调用，本次运行已停止。",
        "tool_not_offered" => "模型调用了不在可用列表里的工具，本次运行已停止。",
        "tool_repeated_result" => "同一个查询连续返回相同的结果，本次运行已停止以免空转。",
        "model_truncated" => "模型回复超出长度上限，本次运行未生成结论。",
        "record_failed" => "运行记录写入失败，本次运行已停止。",
        _ => return None,
    })
}

/// Archives each request body in the run's `outbound/` directory, with a
/// `model_call` hash event, before the call is made: every body that leaves
/// the machine is first archived byte for byte. A body that cannot be archived
/// exactly as it would be sent is not sent; `refused` keeps the error code for
/// the run's error event.
///
/// When the provider refuses the reasoning switch (HTTP 400 to a body that
/// carried one), this is where the single resend is decided: a
/// `model_resend {resent_without: "reasoning_switch", status}` event, then the
/// derived body (the same request without the switch) is archived as a second
/// `model_call`, then sent. A reply that carries a flag (`reasoning_off_refused`,
/// `truncated`, `think_block_unclosed`) or reports usage (`usage {prompt,
/// completion, total}`) gets a `model_reply` event; a truncated reply fails
/// the run with `model_truncated` instead of becoming a conclusion.
///
/// A successful resend is remembered for the model's channel
/// (`Model::remember_reasoning_switch_unsupported`, W3o); a model whose
/// channel is already known to refuse the switch sends none, so there is no
/// refusal and no resend, and its replies are not marked
/// `reasoning_off_refused` (L4 r10 P2-8: only a run that was refused and
/// resent says the channel could not turn thinking off).
fn tool_required_toolset(tool_id: &str) -> Option<&'static str> {
    if tool_id.starts_with("gee_") {
        Some("gee")
    } else if tool_id.starts_with("nisar_") {
        Some("nisar")
    } else if tool_id.starts_with("gis_") {
        Some("local_gis")
    } else if tool_id.starts_with("mcp__") {
        Some("mcp")
    } else {
        None
    }
}

fn detect_toolsets_from_text(text: &str) -> BTreeSet<String> {
    let lower = text.to_lowercase();
    let mut active = BTreeSet::new();
    if [
        "gee",
        "earth engine",
        "google earth",
        "云端计算",
        "ndvi",
        "ndwi",
        "mndwi",
        "nbr",
        "遥感指数",
        "变化检测",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
    {
        active.insert("gee".to_owned());
    }
    if [
        "nisar",
        "rslc",
        "gslc",
        "gcov",
        "gunw",
        "goff",
        "rifg",
        "runw",
        "roff",
        "bhoonidhi",
        "range v2",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
    {
        active.insert("nisar".to_owned());
    }
    if [
        "gdal",
        "qgis",
        "arcgis",
        "conda",
        "rustgis",
        "geolibre",
        "坡度",
        "坡向",
        "山体阴影",
        "重投影",
        "栅格处理",
        "矢量缓冲",
        "地形因子",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
    {
        active.insert("local_gis".to_owned());
    }
    if ["mcp", "连接器", "外部工具"]
        .iter()
        .any(|kw| lower.contains(kw))
    {
        active.insert("mcp".to_owned());
    }
    active
}

struct RecordingModel<'m> {
    model: &'m (dyn Model + Send),
    writer: &'m Mutex<RunWriter>,
    refused: &'m Mutex<Option<&'static str>>,
    /// The part of a body the outbound check refused (`outbound::field_class`).
    blocked: &'m Mutex<Option<&'static str>>,
    step_calls: &'m AtomicU64,
    current_step: &'m AtomicU64,
    max_steps: usize,
    force_wrap_up: &'m std::sync::atomic::AtomicBool,
    active_toolsets: &'m Arc<Mutex<BTreeSet<String>>>,
}
impl RecordingModel<'_> {
    fn refuse(&self, code: &'static str) -> ModelError {
        *lock(self.refused) = Some(code);
        ModelError::new(ErrorKind::Capability, None)
    }

    /// A body the outbound check refuses even after `outbound::messages`:
    /// not sent; the run fails with `outbound_blocked` naming the part of the
    /// body by class (L4 r7 P1-4: this used to read as the provider refusing).
    fn block(&self, body: &Value) -> ModelError {
        let class = outbound::field_class(body);
        *lock(self.blocked) = Some(class);
        let _ = lock(self.writer).append("outbound_blocked", &json!({"field_class": class}));
        self.refuse("outbound_blocked")
    }

    fn archive(&self, body: Option<Result<Value, ModelError>>) -> Result<(), ModelError> {
        let Some(body) = body else {
            return Ok(());
        };
        let body = body?;
        // What is archived must be byte-for-byte what is sent.
        match run::safe_value(&body) {
            Ok(safe) if safe == body => {}
            Ok(_) | Err(run::RunError::Unsafe) => return Err(self.block(&body)),
            Err(_) => return Err(self.refuse("record_failed")),
        }
        let archived = lock(self.writer).outbound(&body).map(|_| ());
        archived.map_err(|error| match error {
            run::RunError::Unsafe => self.block(&body),
            _ => self.refuse("record_failed"),
        })
    }

    fn trace(&self, kind: &str, payload: Value) -> Result<(), ModelError> {
        lock(self.writer)
            .append(kind, &payload)
            .map(|_| ())
            .map_err(|_| self.refuse("record_failed"))
    }
}
impl Model for RecordingModel<'_> {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [ToolDescription],
        cancel: &'a CancellationToken,
        emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            let step_index = self.step_calls.fetch_add(1, Ordering::Relaxed);
            self.current_step.store(step_index, Ordering::Relaxed);
            let wrap_up = self.force_wrap_up.load(Ordering::Relaxed)
                || (self.max_steps >= 6 && (step_index as usize) + 1 == self.max_steps);
            // What the host can make safe is made safe before the check
            // (`outbound`); the same messages are archived and sent.
            let mut prepared = outbound::messages(messages);
            if wrap_up {
                prepared.push(Message {
                    role: "user".into(),
                    content: "[系统提示] 已达单轮工具调用上限，请停止调用工具，直接基于已有信息向用户输出中文总结结论。".into(),
                });
            }
            let messages = prepared.as_slice();
            // Keep the exact tool array stable even on wrap-up so the final,
            // largest-context call never busts the KV prefix cache.
            let disclosed_tools: Vec<ToolDescription> = {
                let active = lock(self.active_toolsets);
                tools
                    .iter()
                    .filter(|td| match tool_required_toolset(&td.name) {
                        None => true,
                        Some(group) => active.contains(group),
                    })
                    .cloned()
                    .collect()
            };
            let effective_tools: &[ToolDescription] = &disclosed_tools;
            self.archive(self.model.request_body(messages, effective_tools))?;
            let completion = match self
                .model
                .complete(messages, effective_tools, cancel, &mut *emit)
                .await
            {
                Err(error) if error.kind == ErrorKind::ReasoningSwitchRefused => {
                    let Some(resend) = self.model.without_reasoning_switch() else {
                        return Err(ModelError::new(ErrorKind::Parameter, error.status));
                    };
                    self.trace(
                        "model_resend",
                        json!({"resent_without": "reasoning_switch", "status": error.status}),
                    )?;
                    self.archive(resend.request_body(messages, effective_tools))?;
                    let mut completion = resend
                        .complete(messages, effective_tools, cancel, &mut *emit)
                        .await
                        .map_err(|error| match error.kind {
                            // The resend carried no switch; a refusal cannot repeat.
                            ErrorKind::ReasoningSwitchRefused => {
                                ModelError::new(ErrorKind::Parameter, error.status)
                            }
                            _ => error,
                        })?;
                    completion.reasoning_off_refused = true;
                    self.model.remember_reasoning_switch_unsupported();
                    if completion.prompt_cache_key_refused {
                        self.model.remember_prompt_cache_key_refused();
                    }
                    completion
                }
                // A model already known to refuse the switch sent none: its
                // reply is not marked refused (L4 r10 P2-8).
                other => {
                    let completion = other?;
                    if completion.prompt_cache_key_refused {
                        self.model.remember_prompt_cache_key_refused();
                    }
                    completion
                }
            };
            // The usage a channel reports goes on this reply's model_reply
            // (the model_call before it was written before the request).
            let usage = (completion.usage.input_tokens.is_some()
                || completion.usage.output_tokens.is_some()
                || completion.usage.cached_input_tokens.is_some()
                || completion.usage.cache_creation_input_tokens.is_some())
            .then(|| {
                // Keys without "token": the run record refuses those.
                let mut map = serde_json::Map::new();
                map.insert("prompt".into(), json!(completion.usage.input_tokens));
                map.insert("completion".into(), json!(completion.usage.output_tokens));
                // Channel counts are not trusted: never overflow.
                map.insert(
                    "total".into(),
                    json!(completion
                        .usage
                        .input_tokens
                        .unwrap_or(0)
                        .saturating_add(completion.usage.output_tokens.unwrap_or(0))),
                );
                if let Some(cached) = completion.usage.cached_input_tokens {
                    map.insert("cached".into(), json!(cached));
                }
                if let Some(cache_creation) = completion.usage.cache_creation_input_tokens {
                    map.insert("cache_creation".into(), json!(cache_creation));
                }
                if let Some(reasoning) = completion.usage.reasoning_tokens {
                    map.insert("reasoning".into(), json!(reasoning));
                }
                Value::Object(map)
            });
            let has_thinking =
                completion.thinking_ms.is_some() || !completion.reasoning_text.is_empty();
            if completion.reasoning_off_refused
                || completion.truncated
                || completion.think_block_unclosed
                || usage.is_some()
                || has_thinking
            {
                let mut reply = json!({
                    "reasoning_off_refused": completion.reasoning_off_refused,
                    "truncated": completion.truncated,
                    "think_block_unclosed": completion.think_block_unclosed,
                });
                if let Some(usage) = usage {
                    reply["usage"] = usage;
                }
                if has_thinking {
                    reply["thinking_ms"] = json!(completion.thinking_ms);
                    reply["reasoning_chars"] = json!(completion.reasoning_text.chars().count());
                }
                self.trace("model_reply", reply)?;
            }
            if completion.truncated {
                return Err(self.refuse("model_truncated"));
            }
            Ok(completion)
        })
    }
    fn request_body(
        &self,
        messages: &[Message],
        tools: &[ToolDescription],
    ) -> Option<Result<Value, ModelError>> {
        self.model.request_body(messages, tools)
    }
}

fn validate_question_card_args(run_id: &str, arguments: &Value) -> Result<Value, String> {
    let Some(map) = arguments.as_object() else {
        return Err("arguments must be an object".into());
    };
    for key in map.keys() {
        if !matches!(key.as_str(), "question_zh" | "options") {
            return Err(format!("unknown field {key}"));
        }
    }
    let question_zh = map
        .get("question_zh")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty() && q.chars().count() <= 240)
        .ok_or_else(|| "question_zh must be a non-empty string up to 240 chars".to_owned())?;
    let options = map
        .get("options")
        .and_then(Value::as_array)
        .filter(|items| (2..=5).contains(&items.len()))
        .ok_or_else(|| "options must be an array of 2 to 5 items".to_owned())?;
    let mut normalized = Vec::with_capacity(options.len());
    for (idx, item) in options.iter().enumerate() {
        let Some(opt) = item.as_object() else {
            return Err(format!("options[{idx}] must be an object"));
        };
        for key in opt.keys() {
            if !matches!(key.as_str(), "label_zh" | "value" | "recommended") {
                return Err(format!("options[{idx}]: unknown field {key}"));
            }
        }
        let label_zh = opt
            .get("label_zh")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= 80)
            .ok_or_else(|| format!("options[{idx}].label_zh must be 1..=80 chars"))?;
        let value = opt
            .get("value")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= 160)
            .ok_or_else(|| format!("options[{idx}].value must be 1..=160 chars"))?;
        let recommended = match opt.get("recommended") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(flag)) => *flag,
            Some(_) => return Err(format!("options[{idx}].recommended must be boolean")),
        };
        normalized.push(json!({
            "label_zh": history::outbound_text(label_zh),
            "value": history::outbound_text(value),
            "recommended": recommended,
        }));
    }
    Ok(json!({
        "run_id": run_id,
        "question_zh": history::outbound_text(question_zh),
        "options": normalized,
    }))
}

fn validate_workspace_sync_args(
    run_id: &str,
    arguments: &Value,
) -> Result<(String, Value), String> {
    let Some(map) = arguments.as_object() else {
        return Err("arguments must be an object".into());
    };
    const KNOWN: &[&str] = &[
        "action",
        "scene_id",
        "enabled",
        "layer_key",
        "target",
        "tab",
        "batch_id",
        "display_name",
        "bbox",
    ];
    for key in map.keys() {
        if !KNOWN.contains(&key.as_str()) {
            return Err(format!("unknown field {key}"));
        }
    }
    let action = map
        .get("action")
        .and_then(Value::as_str)
        .filter(|a| {
            matches!(
                *a,
                "select_scene" | "toggle_preview" | "set_basemap" | "fit_bounds" | "open_panel"
            )
        })
        .ok_or_else(|| {
            "action must be one of select_scene, toggle_preview, set_basemap, fit_bounds or open_panel"
                .to_owned()
        })?;
    let mut out = serde_json::Map::new();
    out.insert("run_id".into(), json!(run_id));
    out.insert("action".into(), json!(action));
    for (key, max_len) in [
        ("scene_id", 256usize),
        ("layer_key", 64),
        ("target", 64),
        ("tab", 64),
        ("batch_id", 128),
        ("display_name", 160),
    ] {
        match map.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::String(text)) => {
                let trimmed = text.trim();
                if trimmed.is_empty() || trimmed.chars().count() > max_len {
                    return Err(format!("{key} must be 1..={max_len} chars"));
                }
                out.insert(key.into(), json!(history::outbound_text(trimmed)));
            }
            Some(_) => return Err(format!("{key} must be a string")),
        }
    }
    match map.get("enabled") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(flag)) => {
            out.insert("enabled".into(), json!(flag));
        }
        Some(_) => return Err("enabled must be a boolean".into()),
    }
    match map.get("bbox") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) if items.len() == 4 => {
            let coords = items
                .iter()
                .map(|v| v.as_f64().filter(|n| n.is_finite()))
                .collect::<Option<Vec<f64>>>()
                .ok_or_else(|| "bbox must contain 4 finite numbers".to_owned())?;
            if coords[0] >= coords[2] || coords[1] >= coords[3] {
                return Err("bbox requires west < east and south < north".into());
            }
            out.insert("bbox".into(), json!(coords));
        }
        Some(_) => return Err("bbox must be [west, south, east, north]".into()),
    }
    Ok((action.to_owned(), Value::Object(out)))
}

struct TurnDispatcher<'h, S, M> {
    host: &'h AgentHost<S, M>,
    run_id: String,
    session_id: String,
    route: String,
    executor: Mutex<Executor<HostPort<S>>>,
    writer: Mutex<RunWriter>,
    proposed: Mutex<Option<Proposal>>,
    local: LocalContext,
    /// A host code that ended the turn from inside a tool call.
    failure: Mutex<Option<&'static str>>,
    circuit_breaker: Mutex<SearchCircuitBreaker>,
    consecutive_failures: AtomicU64,
    force_wrap_up: std::sync::atomic::AtomicBool,
    wrap_up_violations: AtomicU64,
    current_step: Arc<AtomicU64>,
    max_steps: usize,
    verified_admin_codes: Mutex<std::collections::HashSet<String>>,
    goal_manifest: Mutex<Option<goal_manifest::GoalManifest>>,
    read_call_cache: Mutex<HashMap<(String, String), Value>>,
    active_toolsets: Arc<Mutex<BTreeSet<String>>>,
    casual_fast_path: bool,
}

impl<S, M> Dispatcher for TurnDispatcher<'_, S, M>
where
    S: UnifiedApplication + StoreAccess + Clone + Sync + 'static,
    M: ModelSource + 'static,
{
    fn execute<'a>(
        &'a self,
        call: &'a ToolCall,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
        let result = self.handle(call);
        Box::pin(async move { result })
    }

    fn accepts_unoffered_calls(&self) -> bool {
        !self.casual_fast_path
    }
}

impl<S, M> TurnDispatcher<'_, S, M>
where
    S: UnifiedApplication + StoreAccess + Clone + Sync + 'static,
    M: ModelSource + 'static,
{
    fn note_failure(&self) {
        let count = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if count >= 3 {
            self.force_wrap_up.store(true, Ordering::Relaxed);
        }
    }

    fn reset_failures(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    fn handle(&self, call: &ToolCall) -> Result<Value, ModelError> {
        if self.force_wrap_up.load(Ordering::Relaxed) {
            let violations = self.wrap_up_violations.fetch_add(1, Ordering::Relaxed) + 1;
            if violations > 1 {
                return Err(ModelError::new(ErrorKind::StepLimit, None));
            }
            return Ok(json!({
                "error": "wrap_up_required",
                "message_zh": "连续多次调用失败，请停止调用工具并直接向用户总结当前进展。"
            }));
        }
        if self.max_steps >= 6
            && (self.current_step.load(Ordering::Relaxed) as usize) + 1 >= self.max_steps
            && (call.name == "propose_plan"
                || is_side_effect_download_extension_tool(&call.name, &call.arguments)
                || is_side_effect_local_execution_tool(&call.name))
        {
            return Ok(json!({
                "error": "final_step_wrap_up",
                "message_zh": "已达本轮步数上限，暂不发起新的写操作或下载计划，请直接总结当前结果并告知用户下一步操作。"
            }));
        }
        if call.name == "propose_plan" {
            let mut proposed = lock(&self.proposed);
            if proposed.is_some() {
                return Err(ModelError::new(ErrorKind::Capability, None));
            }
            let mut writer = lock(&self.writer);
            let mut breaker = lock(&self.circuit_breaker);
            let verified_codes = lock(&self.verified_admin_codes).clone();
            return match self.host.propose(
                &self.run_id,
                &self.session_id,
                &self.route,
                &call.arguments,
                &self.local,
                &verified_codes,
                &mut breaker,
                &mut writer,
            ) {
                Ok((plan_id, cards, steps, tier, estimated_bytes)) => {
                    self.reset_failures();
                    *proposed = Some(Proposal {
                        plan_id,
                        call_id: call.id.clone(),
                        cards,
                        steps,
                        tier,
                        estimated_bytes,
                    });
                    // Stops this turn; the loop hands back a checkpoint to park.
                    Err(ModelError::new(ErrorKind::Capability, None))
                }
                Err(refusal) => {
                    self.note_failure();
                    let result = refusal.result(&call.arguments);
                    let full = json!({
                        "reason": refusal.reason, "code": refusal.code,
                        "step": result["step"], "tool_id": result["tool_id"],
                        "detail": result["detail"],
                        "unknown_fields": result["unknown_fields"],
                        "missing_fields": result["missing_fields"],
                    });
                    // A payload the run record refuses still leaves the reason.
                    if self.host.trace(&mut writer, "plan_refused", &full).is_err() {
                        let _ = self.host.trace(
                            &mut writer,
                            "plan_refused",
                            &json!({"reason": refusal.reason, "code": refusal.code}),
                        );
                    }
                    Ok(result)
                }
            };
        }
        // Start time and duration are the host's own clock around the call;
        // the trace keeps no call start, only the result's observation time.
        let started_at = now();
        let clock = std::time::Instant::now();
        let input_summary = tool_input_summary(&call.arguments);
        let round = |status: &str, ended: bool, summary: Option<&Value>| {
            let duration_ms =
                ended.then(|| u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX));
            let target_label = tool_target_label(&call.name, &call.arguments, summary);
            let output_summary = summary.and_then(|s| tool_output_summary(&call.name, s));
            let _ = self.host.append_event(
                &self.session_id,
                "tool_round",
                json!({
                    "run_id": self.run_id, "call_id": call.id, "tool_id": call.name, "status": status,
                    "started_at": started_at, "duration_ms": duration_ms,
                    "result_count": summary.and_then(|s| result_count(&call.name, s)),
                    "summary_zh": summary.and_then(|s| summary_zh(&call.name, s)),
                    "target_label": target_label,
                    "input_summary": input_summary,
                    "output_summary": output_summary,
                }),
            );
        };
        round("running", false, None);
        let mut ext_args = call.arguments.clone();
        if ext_args.get("bbox").is_none() || ext_args.get("bbox") == Some(&Value::Null) {
            if let Some(local_aoi) = &self.local.aoi {
                let candidate = aoi::bbox(aoi::geometry(local_aoi));
                if candidate.is_array() {
                    if let Some(obj) = ext_args.as_object_mut() {
                        obj.insert("bbox".into(), candidate);
                    }
                }
            }
        }
        let read_cache_key = is_cacheable_read_tool(&call.name)
            .then(|| (call.name.clone(), canonical_json_for_cache(&ext_args)));
        if let Some(ref key) = read_cache_key {
            if let Some(mut cached) = lock(&self.read_call_cache).get(key).cloned() {
                if let Some(obj) = cached.as_object_mut() {
                    obj.insert("cached_same_run".into(), Value::Bool(true));
                }
                lock(&self.executor).note_cached_tool(&call.name);
                self.reset_failures();
                round("done", true, Some(&cached));
                return Ok(cached);
            }
        }
        if call.name == "toolset_activate" {
            let mut requested = Vec::new();
            if let Some(arr) = call.arguments.get("toolsets").and_then(Value::as_array) {
                for item in arr {
                    if let Some(s) = item.as_str().map(str::trim) {
                        if matches!(s, "gee" | "nisar" | "local_gis" | "mcp")
                            && !requested.iter().any(|existing| existing == s)
                        {
                            requested.push(s.to_owned());
                        }
                    }
                }
            } else if let Some(single) = call
                .arguments
                .get("toolset")
                .or_else(|| call.arguments.get("category"))
                .and_then(Value::as_str)
                .map(str::trim)
            {
                if matches!(single, "gee" | "nisar" | "local_gis" | "mcp") {
                    requested.push(single.to_owned());
                }
            }
            if requested.is_empty() {
                self.note_failure();
                let mut writer = lock(&self.writer);
                let mut denial =
                    ToolDenial::new(HostError::Invalid.code(), "input_invalid", &call.name);
                denial.detail =
                    Some("toolsets must include one of: gee, nisar, local_gis, mcp".into());
                let result = denial.result(&call.arguments);
                self.host
                    .record_denial(&mut writer, &denial.event(&result, None));
                round("failed", true, Some(&result));
                return Ok(result);
            }
            let (all_active, persisted_toolsets): (Vec<String>, BTreeSet<String>) = {
                let mut slot = lock(&self.active_toolsets);
                for ts in &requested {
                    slot.insert(ts.clone());
                }
                (slot.iter().cloned().collect(), slot.clone())
            };
            self.host
                .save_session_toolsets(&self.session_id, &persisted_toolsets);
            let summary = json!({
                "ok": true,
                "activated": requested,
                "active_toolsets": all_active,
            });
            let mut writer = lock(&self.writer);
            let _ = self.host.trace(
                &mut writer,
                "tool_call",
                &json!({"tool_id": "toolset_activate", "tier": "none", "activated": summary["activated"]}),
            );
            let _ = self.host.trace(
                &mut writer,
                "tool_result",
                &json!({"tool_id": "toolset_activate", "summary": summary, "observed_at": now(), "source": "progressive_disclosure"}),
            );
            self.reset_failures();
            round("done", true, Some(&summary));
            return Ok(summary);
        }
        if call.name == "ask_user_question" {
            let mut writer = lock(&self.writer);
            return match validate_question_card_args(&self.run_id, &call.arguments) {
                Ok(payload) => {
                    self.reset_failures();
                    let option_count = payload["options"].as_array().map_or(0, Vec::len);
                    let summary =
                        json!({"status": "question_card_shown", "option_count": option_count});
                    let _ = self.host.trace(
                        &mut writer,
                        "tool_call",
                        &json!({"tool_id": "ask_user_question", "tier": "none", "option_count": option_count}),
                    );
                    let _ = self.host.trace(
                        &mut writer,
                        "tool_result",
                        &json!({"tool_id": "ask_user_question", "summary": summary, "observed_at": now(), "source": "synthetic"}),
                    );
                    let _ = self
                        .host
                        .append_event(&self.session_id, "question_card", payload);
                    round("done", true, Some(&summary));
                    Ok(summary)
                }
                Err(problem) => {
                    self.note_failure();
                    let mut denial =
                        ToolDenial::new(HostError::Invalid.code(), "input_invalid", &call.name);
                    denial.detail = Some(problem);
                    let result = denial.result(&call.arguments);
                    self.host
                        .record_denial(&mut writer, &denial.event(&result, None));
                    round("failed", true, Some(&result));
                    Ok(result)
                }
            };
        }
        if call.name == "map_workspace_action" {
            let mut writer = lock(&self.writer);
            return match validate_workspace_sync_args(&self.run_id, &call.arguments) {
                Ok((action, payload)) => {
                    self.reset_failures();
                    let summary = json!({"status": "synced", "action": action});
                    let _ = self.host.trace(
                        &mut writer,
                        "tool_call",
                        &json!({"tool_id": "map_workspace_action", "tier": "none", "action": action}),
                    );
                    let _ = self.host.trace(
                        &mut writer,
                        "tool_result",
                        &json!({"tool_id": "map_workspace_action", "summary": summary, "observed_at": now(), "source": "synthetic"}),
                    );
                    let _ =
                        self.host
                            .append_event(&self.session_id, "workspace_sync", payload.clone());
                    self.host.notify(AgentHostNotification::WorkspaceSync {
                        session_id: self.session_id.clone(),
                        run_id: self.run_id.clone(),
                        payload,
                    });
                    round("done", true, Some(&summary));
                    Ok(summary)
                }
                Err(problem) => {
                    self.note_failure();
                    let mut denial =
                        ToolDenial::new(HostError::Invalid.code(), "input_invalid", &call.name);
                    denial.detail = Some(problem);
                    let result = denial.result(&call.arguments);
                    self.host
                        .record_denial(&mut writer, &denial.event(&result, None));
                    round("failed", true, Some(&result));
                    Ok(result)
                }
            };
        }
        if call.name == "goal_manifest" {
            let outcome = {
                let mut slot = lock(&self.goal_manifest);
                goal_manifest::apply_with_store(&mut slot, &call.arguments, |batch_id, task_id| {
                    self.host
                        .service
                        .with_store(|store| {
                            goal_manifest::store_has_evidence(store, batch_id, task_id)
                        })
                        .map_err(|err| err.to_string())?
                })
            };
            let mut writer = lock(&self.writer);
            return match outcome {
                Ok(goal_manifest::ApplyOutcome::Summary(summary)) => {
                    self.reset_failures();
                    let action = call
                        .arguments
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let _ = self.host.trace(
                        &mut writer,
                        "tool_call",
                        &json!({"tool_id": "goal_manifest", "tier": "none", "action": action}),
                    );
                    let _ = self.host.trace(
                        &mut writer,
                        "tool_result",
                        &json!({"tool_id": "goal_manifest", "summary": summary, "observed_at": now(), "source": "synthetic"}),
                    );
                    round("done", true, Some(&summary));
                    Ok(summary)
                }
                Ok(goal_manifest::ApplyOutcome::Refused(refused)) => {
                    let action = call
                        .arguments
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let _ = self.host.trace(
                        &mut writer,
                        "tool_call",
                        &json!({"tool_id": "goal_manifest", "tier": "none", "action": action}),
                    );
                    let _ = self.host.trace(
                        &mut writer,
                        "tool_result",
                        &json!({"tool_id": "goal_manifest", "summary": refused, "observed_at": now(), "source": "synthetic"}),
                    );
                    round("done", true, Some(&refused));
                    Ok(refused)
                }
                Err(problem) => {
                    self.note_failure();
                    let mut denial =
                        ToolDenial::new(HostError::Invalid.code(), "input_invalid", &call.name);
                    denial.detail = Some(problem);
                    let result = denial.result(&call.arguments);
                    self.host
                        .record_denial(&mut writer, &denial.event(&result, None));
                    round("failed", true, Some(&result));
                    Ok(result)
                }
            };
        }
        // Uniform side-effect authorization gate: extension download/task-creation
        // tools MUST obey the session's PermissionMode identically to `create_task`
        // plans (`ConfirmChanges` never auto-starts downloads without approval, and
        // model-suggested custom output directories are never auto-approved).
        if is_side_effect_download_extension_tool(&call.name, &call.arguments) {
            let mode = self.host.effective_mode(&self.session_id);
            let custom_dir_requested = ["output_root", "output_dir"].iter().any(|k| {
                call.arguments
                    .get(*k)
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty())
            }) || call.arguments.get("gee_params").is_some_and(|gp| {
                ["output_path", "output_dir", "output_root"]
                    .iter()
                    .any(|k| {
                        gp.get(*k)
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.trim().is_empty())
                    })
            }) || (call.name == "open_geodata_query"
                && call
                    .arguments
                    .get("output_path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .is_some_and(|s| {
                        !s.is_empty() && (s.contains('/') || s.contains('\\') || s.contains(':'))
                    }));
            let block_reason = if mode == PermissionMode::ConfirmChanges {
                Some((
                    "requires_user_confirmation",
                    "当前会话处于「变更前确认」权限模式，未获得用户明确批准前不能直接创建或启动下载任务。",
                    "请先向用户说明待下载的数据源、空间范围与预估体积（或先调用只读预检工具核实），不要重复直接调用该下载工具；标准下载请提交 propose_plan 等待用户在确认卡上批准，或在用户明确切换到自动批准模式后再执行。",
                ))
            } else if custom_dir_requested {
                Some((
                    "output_dir_unverified",
                    "模型直接指定的输出目录未经用户确认，已拒绝自动启动下载。",
                    "请省略 output_root / output_dir 以使用默认下载目录，或让用户在桌面确认卡上指定目录。",
                ))
            } else {
                None
            };
            if let Some((reason, msg_zh, hint_zh)) = block_reason {
                self.note_failure();
                let mut writer = lock(&self.writer);
                let result = json!({
                    "ok": false,
                    "error": "tool_denied",
                    "code": "AGENT_APPROVAL_REQUIRED",
                    "reason": reason,
                    "tool_id": call.name,
                    "requires_confirmation": true,
                    "permission_mode": mode.as_str(),
                    "message_zh": msg_zh,
                    "hint_zh": hint_zh,
                });
                self.host.record_denial(
                    &mut writer,
                    &json!({
                        "tool_id": call.name,
                        "code": "AGENT_APPROVAL_REQUIRED",
                        "reason": reason,
                        "permission_mode": mode.as_str(),
                    }),
                );
                round("failed", true, Some(&result));
                return Ok(result);
            }
        }
        // Uniform side-effect authorization gate for external local execution tools
        // (`gis_external_execute`): require explicit user confirmation under
        // `ConfirmChanges` and always block unverified custom output path overrides.
        if is_side_effect_local_execution_tool(&call.name) {
            let mode = self.host.effective_mode(&self.session_id);
            let unverified_output = local_execution_has_unverified_output_override(&call.arguments);
            let block_reason = if mode == PermissionMode::ConfirmChanges {
                Some((
                    "requires_user_confirmation",
                    "当前会话处于「变更前确认」权限模式，未获得用户明确批准前不能直接调用本机外部 GIS / 科学计算工具链。",
                    "请不要在同一轮重复调用该工具；请先向用户清晰说明待执行的工具链、操作类型、算法、输入文件与输出文件名，待用户明确确认或切换到自动批准/完全访问模式后再执行。",
                ))
            } else if unverified_output {
                Some((
                    "output_dir_unverified",
                    "模型直接指定了未经授权的输出路径或 params 输出覆盖参数，已拒绝执行外部 GIS 工具。",
                    "请移除 params 中的输出路径字段，仅通过纯文件名形式的 output_filename 将结果写入受控 gis_outputs 目录。",
                ))
            } else {
                None
            };
            if let Some((reason, msg_zh, hint_zh)) = block_reason {
                self.note_failure();
                let mut writer = lock(&self.writer);
                let result = json!({
                    "ok": false,
                    "error": "tool_denied",
                    "code": "AGENT_APPROVAL_REQUIRED",
                    "reason": reason,
                    "tool_id": call.name,
                    "requires_confirmation": true,
                    "permission_mode": mode.as_str(),
                    "message_zh": msg_zh,
                    "hint_zh": hint_zh,
                });
                self.host.record_denial(
                    &mut writer,
                    &json!({
                        "tool_id": call.name,
                        "code": "AGENT_APPROVAL_REQUIRED",
                        "reason": reason,
                        "permission_mode": mode.as_str(),
                    }),
                );
                round("failed", true, Some(&result));
                return Ok(result);
            }
        }
        if let Some(ext_outcome) = self
            .host
            .service
            .application_call_extension_tool(&call.name, &ext_args)
        {
            let mut writer = lock(&self.writer);
            return match ext_outcome {
                Ok(summary) => {
                    let safe_in = run::safe_value(&ext_args).unwrap_or_else(|_| json!({}));
                    let safe_out = run::safe_value(&summary)
                        .unwrap_or_else(|_| json!({"ok": true, "note": "result_redacted"}));
                    let is_ok = safe_out.get("ok") != Some(&Value::Bool(false));
                    if is_ok {
                        if let Some(group) = tool_required_toolset(&call.name) {
                            let persisted = {
                                let mut slot = lock(&self.active_toolsets);
                                let inserted = slot.insert(group.to_owned());
                                inserted.then(|| slot.clone())
                            };
                            if let Some(toolsets) = persisted {
                                self.host.save_session_toolsets(&self.session_id, &toolsets);
                            }
                        }
                        self.reset_failures();
                        if let Some(key) = read_cache_key {
                            lock(&self.read_call_cache).insert(key, safe_out.clone());
                        }
                    } else {
                        self.note_failure();
                    }
                    let observed = now();
                    let fp = format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(&safe_out).unwrap_or_default())
                    );
                    let _ = self.host.trace(
                        &mut writer,
                        "tool_call",
                        &json!({"tool_id": call.name, "version": 1, "tier": "none", "input": safe_in}),
                    );
                    let _ = self.host.trace(
                        &mut writer,
                        "tool_result",
                        &json!({
                            "tool_id": call.name,
                            "summary": safe_out,
                            "sha256": fp,
                            "observed_at": observed,
                            "source": "gee_cloud_suite",
                        }),
                    );
                    if is_ok {
                        if let Some(ev) = build_gee_artifact_event_payload(
                            &self.run_id,
                            &call.id,
                            &call.name,
                            &safe_in,
                            &safe_out,
                            &ext_args,
                        ) {
                            let _ = self.host.append_event(&self.session_id, "gee_artifact", ev);
                        }
                    }
                    round(if is_ok { "done" } else { "failed" }, true, Some(&safe_out));
                    Ok(safe_out)
                }
                Err(class) => {
                    self.note_failure();
                    let denial = ToolDenial::of_execution(
                        &ExecutionError::Service(class),
                        &call.name,
                        &call.arguments,
                    );
                    let result = denial.result(&call.arguments);
                    self.host
                        .record_denial(&mut writer, &denial.event(&result, None));
                    round("failed", true, Some(&result));
                    Ok(result)
                }
            };
        }
        let mut writer = lock(&self.writer);
        if call.name == "working_aoi_get" {
            if let Some(denial) = ToolDenial::before_read(&call.name, &call.arguments) {
                self.note_failure();
                let result = denial.result(&call.arguments);
                self.host
                    .record_denial(&mut writer, &denial.event(&result, None));
                round("failed", true, Some(&result));
                return Ok(result);
            }
            if let Some(session_aoi) = self
                .host
                .session_working_aoi_if_initialized(&self.session_id)
            {
                self.reset_failures();
                let raw_summary = aoi::working_summary(&session_aoi);
                let summary = crate::run::safe_value(&host::sanitize(&raw_summary))
                    .unwrap_or_else(|_| json!({"state": "none"}));
                let observed = now();
                let fp = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&summary).unwrap_or_default())
                );
                let _ = self.host.trace(
                    &mut writer,
                    "tool_call",
                    &json!({"tool_id": "working_aoi_get", "version": 1, "tier": "read", "input": call.arguments}),
                );
                let _ = self.host.trace(
                    &mut writer,
                    "tool_result",
                    &json!({
                        "tool_id": "working_aoi_get",
                        "summary": summary,
                        "sha256": fp,
                        "observed_at": observed,
                        "source": "session_workspace",
                    }),
                );
                round("done", true, Some(&summary));
                if let Some(key) = read_cache_key {
                    lock(&self.read_call_cache).insert(key, summary.clone());
                }
                return Ok(summary);
            }
        }
        let outcome = match ToolDenial::before_read(&call.name, &call.arguments) {
            Some(denial) => Err(denial),
            None => {
                match lock(&self.executor).call_read(&call.name, &call.arguments, &mut writer) {
                    Ok(summary) => Ok(summary),
                    Err(ExecutionError::Stalled) => {
                        let stall_summary = json!({
                            "ok": false,
                            "error": "tool_repeated_result",
                            "message_zh": "同一个查询连续返回相同结果，已触发防循环保护停止重试。",
                        });
                        round("failed", true, Some(&stall_summary));
                        *lock(&self.failure) = Some("tool_repeated_result");
                        return Err(ModelError::new(ErrorKind::Capability, None));
                    }
                    Err(error) => Err(ToolDenial::of_execution(
                        &error,
                        &call.name,
                        &call.arguments,
                    )),
                }
            }
        };
        match outcome {
            Ok(mut summary) => {
                self.reset_failures();
                if let Some(key) = read_cache_key {
                    lock(&self.read_call_cache).insert(key, summary.clone());
                }
                if call.name == "admin_boundary_search" {
                    let mut codes = lock(&self.verified_admin_codes);
                    for arr_key in ["candidates", "items"] {
                        if let Some(items) = summary.get(arr_key).and_then(Value::as_array) {
                            for item in items {
                                for field in ["admin_code", "code"] {
                                    if let Some(c) = item
                                        .get(field)
                                        .and_then(Value::as_str)
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty())
                                    {
                                        codes.insert(c.to_owned());
                                        if let Some(bare) = c.strip_prefix("datav:") {
                                            codes.insert(bare.to_owned());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if call.name == "search_progress" {
                    self.host.record_search_batch(
                        &self.session_id,
                        &self.run_id,
                        &summary,
                        &mut writer,
                    );
                    let observed_req_id = summary
                        .get("request_id")
                        .and_then(Value::as_str)
                        .or_else(|| call.arguments.get("request_id").and_then(Value::as_str))
                        .map(str::to_owned);
                    if let Some(req_id) = observed_req_id {
                        lock(&self.circuit_breaker).observe_search_progress(&req_id, &mut summary);
                    }
                }
                round("done", true, Some(&summary));
                Ok(summary)
            }
            Err(denial) => {
                self.note_failure();
                let result = denial.result(&call.arguments);
                self.host
                    .record_denial(&mut writer, &denial.event(&result, None));
                round("failed", true, Some(&result));
                Ok(result)
            }
        }
    }
}

fn user_intent_slice(msg: &Message) -> String {
    let raw = if msg.role == "user_multimodal" {
        serde_json::from_str::<Value>(&msg.content)
            .ok()
            .and_then(|v| v.get("text").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default()
    } else {
        msg.content.clone()
    };
    raw.split("\n\n[context] ").next().unwrap_or("").to_owned()
}

fn validate_turn_messages_safe(messages: &[Message]) -> Result<(), run::RunError> {
    for msg in messages {
        match msg.role.as_str() {
            "user" | "assistant" => {
                let val = Value::String(msg.content.clone());
                if run::safe_value(&val)? != val {
                    return Err(run::RunError::Unsafe);
                }
            }
            "assistant_tool_calls" | "tool" => {
                let val: Value =
                    serde_json::from_str(&msg.content).map_err(|_| run::RunError::Unsafe)?;
                if run::safe_value(&val)? != val {
                    return Err(run::RunError::Unsafe);
                }
            }
            _ => return Err(run::RunError::Unsafe),
        }
    }
    Ok(())
}

fn downgrade_user_multimodal(msg: Message) -> Message {
    if msg.role != "user_multimodal" {
        return msg;
    }
    let (text, img_count) = serde_json::from_str::<Value>(&msg.content)
        .ok()
        .map(|v| {
            let text = v
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let n = v
                .get("images")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            (text, n)
        })
        .unwrap_or_else(|| (msg.content, 0));
    let trimmed = text.trim();
    let content = if img_count > 0 {
        if trimmed.is_empty() {
            format!("[附带 {img_count} 张图片，历史轮次仅保留文字]")
        } else {
            format!("{text}\n[附带 {img_count} 张图片，历史轮次仅保留文字]")
        }
    } else if trimmed.is_empty() {
        "[附带图片或文件]".to_owned()
    } else {
        text
    };
    Message {
        role: "user".into(),
        content,
    }
}

fn sanitize_incomplete_tool_tail(msgs: &mut Vec<Message>) {
    loop {
        let tail_tool_start = msgs
            .iter()
            .rposition(|m| m.role != "tool")
            .map_or(0, |idx| idx + 1);
        if tail_tool_start < msgs.len() {
            if tail_tool_start == 0 {
                msgs.clear();
                break;
            }
            let prev = &msgs[tail_tool_start - 1];
            if prev.role != "assistant_tool_calls" {
                msgs.truncate(tail_tool_start);
                continue;
            }
            let expected_ids: Vec<String> = serde_json::from_str::<Value>(&prev.content)
                .ok()
                .and_then(|v| v.get("tool_calls").and_then(Value::as_array).cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|c| c.get("id").and_then(Value::as_str).map(str::to_owned))
                .collect();
            let mut got_ids = std::collections::HashSet::new();
            for tool_msg in &msgs[tail_tool_start..] {
                if let Ok(v) = serde_json::from_str::<Value>(&tool_msg.content) {
                    if let Some(id) = v.get("id").and_then(Value::as_str) {
                        got_ids.insert(id.to_owned());
                    }
                }
            }
            let complete =
                !expected_ids.is_empty() && expected_ids.iter().all(|id| got_ids.contains(id));
            if !complete {
                msgs.truncate(tail_tool_start - 1);
                continue;
            }
            break;
        } else if msgs
            .last()
            .is_some_and(|m| m.role == "assistant_tool_calls")
        {
            msgs.pop();
        } else {
            break;
        }
    }
}

fn clip_oversized_single_turn(msgs: &mut Vec<Message>, max_chars: usize) {
    let char_len =
        |slice: &[Message]| -> usize { slice.iter().map(|m| m.content.chars().count()).sum() };
    if char_len(msgs) <= max_chars {
        return;
    }
    for msg in msgs.iter_mut() {
        if msg.role == "tool" && msg.content.chars().count() > 2_000 {
            if let Ok(mut payload) = serde_json::from_str::<Value>(&msg.content) {
                let snippet = clip_summary_text(&payload["result"].to_string(), 1_200);
                payload["result"] = json!({"truncated_for_budget": true, "summary": snippet});
                msg.content = payload.to_string();
            }
        }
    }
    while char_len(msgs) > max_chars {
        if let Some(pos) = msgs.iter().position(|m| m.role == "assistant_tool_calls") {
            let mut end = pos + 1;
            while end < msgs.len() && msgs[end].role == "tool" {
                end += 1;
            }
            msgs.drain(pos..end);
        } else {
            break;
        }
    }
    while char_len(msgs) > max_chars && msgs.len() > 2 {
        msgs.remove(1);
    }
    if char_len(msgs) > max_chars {
        let divisor = msgs.len().max(1);
        let piece_budget = (max_chars / divisor).max(512);
        for msg in msgs.iter_mut() {
            msg.content = clip_summary_text(&msg.content, piece_budget);
        }
    }
}

fn collect_toolsets_from_messages(messages: &[Message]) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for msg in messages {
        if matches!(msg.role.as_str(), "user" | "user_multimodal") {
            let intent = user_intent_slice(msg);
            found.extend(detect_toolsets_from_text(&intent));
        } else if msg.role == "assistant_tool_calls" {
            if let Ok(payload) = serde_json::from_str::<Value>(&msg.content) {
                if let Some(calls) = payload.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        let name = call.get("name").and_then(Value::as_str).unwrap_or("");
                        if let Some(group) = tool_required_toolset(name) {
                            found.insert(group.to_owned());
                        } else if name == "toolset_activate" {
                            let args = &call["arguments"];
                            if let Some(arr) = args.get("toolsets").and_then(Value::as_array) {
                                for item in arr {
                                    if let Some(s) = item.as_str().map(str::trim) {
                                        if matches!(s, "gee" | "nisar" | "local_gis" | "mcp") {
                                            found.insert(s.to_owned());
                                        }
                                    }
                                }
                            } else if let Some(single) = args
                                .get("toolset")
                                .or_else(|| args.get("category"))
                                .and_then(Value::as_str)
                                .map(str::trim)
                            {
                                if matches!(single, "gee" | "nisar" | "local_gis" | "mcp") {
                                    found.insert(single.to_owned());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    found
}

impl<S, M> AgentHost<S, M>
where
    S: UnifiedApplication + StoreAccess + Clone + Sync + 'static,
    M: ModelSource + 'static,
{
    /// `root` is the application data directory; sessions live under
    /// `agent/sessions`, run records under `agent/runs` (W3a layout).
    pub fn open(root: PathBuf, service: S, models: M) -> Result<Arc<Self>, HostError> {
        fs::create_dir_all(root.join("agent").join("sessions"))?;
        // A run left `running` by a previous process cannot resume: seal it.
        service.with_store(|store| run::mark_interrupted(&root, store, &now()))??;
        let host = Arc::new(Self {
            root,
            service,
            models,
            files: Mutex::new(HashMap::new()),
            live: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            agent_tasks: Mutex::new(None),
            activity: Mutex::new(None),
            max_steps: AtomicUsize::new(8),
            notifier: Mutex::new(None),
            trace_faults: Mutex::new(Vec::new()),
        });
        Ok(host)
    }

    pub fn max_steps(&self) -> usize {
        self.max_steps.load(Ordering::Relaxed)
    }

    pub fn set_max_steps(&self, steps: usize) {
        self.max_steps.store(steps.clamp(1, 32), Ordering::Relaxed);
    }

    pub fn set_notifier(&self, notifier: AgentHostNotifier) {
        *lock(&self.notifier) = Some(notifier);
    }

    pub fn clear_notifier(&self) {
        *lock(&self.notifier) = None;
    }

    fn notify(&self, event: AgentHostNotification) {
        let cb = lock(&self.notifier).clone();
        if let Some(cb) = cb {
            cb(event);
        }
    }

    /// Makes the next run-record write of `kind` fail, as a full or failing
    /// disk would. Test support only.
    #[doc(hidden)]
    pub fn inject_trace_fault(&self, kind: &'static str) {
        lock(&self.trace_faults).push(kind);
    }
    fn trace(
        &self,
        writer: &mut RunWriter,
        kind: &'static str,
        payload: &Value,
    ) -> Result<(), run::RunError> {
        {
            let mut faults = lock(&self.trace_faults);
            if let Some(index) = faults.iter().position(|fault| *fault == kind) {
                faults.remove(index);
                return Err(run::RunError::Io(std::io::Error::other(
                    "injected run-record fault",
                )));
            }
        }
        writer.append(kind, payload).map(|_| ())
    }

    /// A `tool_denied` run event; a payload the run record refuses still
    /// leaves the reason and code.
    fn record_denial(&self, writer: &mut RunWriter, event: &Value) {
        if self.trace(writer, "tool_denied", event).is_err() {
            let _ = self.trace(
                writer,
                "tool_denied",
                &json!({"reason": event["reason"], "code": event["code"]}),
            );
        }
    }

    fn new_id(&self, prefix: &str) -> String {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!(
            "{prefix}-{millis:x}-{}",
            self.counter.fetch_add(1, Ordering::Relaxed)
        )
    }
    fn sessions_dir(&self) -> PathBuf {
        self.root.join("agent").join("sessions")
    }
    fn session_dir(&self, session_id: &str) -> Result<PathBuf, HostError> {
        if !valid_id(session_id) {
            return Err(HostError::Invalid);
        }
        let dir = self.sessions_dir().join(session_id);
        if !dir.join("session.json").is_file()
            || self
                .sessions_dir()
                .join(format!("{session_id}.deleted"))
                .exists()
        {
            return Err(HostError::NotFound);
        }
        Ok(dir)
    }
    /// Checked inside the Store critical section that writes a plan or a run
    /// row: deletion writes its marker inside its own critical section, so
    /// either the write lands first (and is deleted) or it sees the marker.
    fn session_gone(&self, session_id: &str) -> bool {
        self.session_dir(session_id).is_err()
    }
    fn load_session(&self, session_id: &str) -> Result<SessionSummary, HostError> {
        let dir = self.session_dir(session_id)?;
        Ok(serde_json::from_slice(&fs::read(
            dir.join("session.json"),
        )?)?)
    }
    fn save_session(&self, summary: &SessionSummary) -> Result<(), HostError> {
        let dir = self.sessions_dir().join(&summary.session_id);
        let tmp = dir.join("session.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(summary)?)?;
        fs::rename(tmp, dir.join("session.json"))?;
        Ok(())
    }

    /// Sessions that are not archived, pinned first (most recently pinned
    /// first), then by last activity.
    pub fn sessions_list(&self) -> Result<Vec<SessionSummary>, HostError> {
        self.sessions_list_with(false)
    }

    fn populate_last_run_statuses(&self, sessions: &mut [SessionSummary]) -> Result<(), HostError> {
        let mut live_by_session: HashMap<String, &'static str> = HashMap::new();
        {
            let live = lock(&self.live);
            for entry in live.values() {
                let status = if entry.parked.is_some() {
                    "awaiting_approval"
                } else {
                    "running"
                };
                live_by_session
                    .entry(entry.session_id.clone())
                    .and_modify(|existing| {
                        if status == "awaiting_approval" {
                            *existing = status;
                        }
                    })
                    .or_insert(status);
            }
        }

        let mut missing_indices: Vec<usize> = Vec::new();
        for (idx, summary) in sessions.iter_mut().enumerate() {
            if let Some(status) = live_by_session.get(&summary.session_id) {
                summary.last_run_status = Some((*status).to_owned());
            } else if summary.run_ids.is_empty() {
                summary.last_run_status = None;
            } else {
                missing_indices.push(idx);
            }
        }

        if missing_indices.is_empty() {
            return Ok(());
        }

        self.service.with_store(|store| -> Result<(), HostError> {
            for idx in missing_indices {
                let summary = &mut sessions[idx];
                summary.last_run_status = None;
                for run_id in summary.run_ids.iter().rev() {
                    if let Some(row) = store.agent_run(run_id)? {
                        let mapped = match row.status.as_str() {
                            "running" => "running",
                            "awaiting_approval" => "awaiting_approval",
                            "completed" => "completed",
                            "failed" => "failed",
                            "cancelled" | "interrupted" => "cancelled",
                            _ => continue,
                        };
                        summary.last_run_status = Some(mapped.to_owned());
                        break;
                    }
                }
            }
            Ok(())
        })??;
        Ok(())
    }

    pub fn sessions_list_with(
        &self,
        include_archived: bool,
    ) -> Result<Vec<SessionSummary>, HostError> {
        let mut sessions = {
            let _files = lock(&self.files);
            let mut sessions = Vec::new();
            for entry in fs::read_dir(self.sessions_dir())? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type()?.is_dir() && valid_id(&name) {
                    if let Ok(summary) = self.load_session(&name) {
                        if include_archived || summary.archived_at.is_none() {
                            sessions.push(summary);
                        }
                    }
                }
            }
            sessions
        };
        self.populate_last_run_statuses(&mut sessions)?;
        sessions.sort_by(|a, b| {
            b.pinned_at
                .is_some()
                .cmp(&a.pinned_at.is_some())
                .then_with(|| b.pinned_at.cmp(&a.pinned_at))
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        Ok(sessions)
    }

    pub fn session_create(&self, title: Option<String>) -> Result<SessionSummary, HostError> {
        self.session_create_in(title, None)
    }

    /// Creates a session, filed under `workspace_id` when one is given.
    pub fn session_create_in(
        &self,
        title: Option<String>,
        workspace_id: Option<String>,
    ) -> Result<SessionSummary, HostError> {
        let title = title
            .as_deref()
            .and_then(session_title)
            .unwrap_or_else(|| "新会话".to_owned());
        if let Some(id) = workspace_id.as_deref() {
            self.workspace(id)?;
        }
        let _files = lock(&self.files);
        let session_id = self.new_id("session");
        let dir = self.sessions_dir().join(&session_id);
        fs::create_dir(&dir)?;
        fs::File::create(dir.join("events.jsonl"))?;
        let at = now();
        let summary = SessionSummary {
            session_id,
            title,
            created_at: at.clone(),
            updated_at: at,
            run_ids: vec![],
            permission_mode: None,
            pinned_at: None,
            archived_at: None,
            workspace_id,
            remembered_output_dir: None,
            last_run_status: None,
        };
        self.save_session(&summary)?;
        Ok(summary)
    }

    /// Loads, changes and saves one session summary under the file lock.
    fn update_session(
        &self,
        session_id: &str,
        change: impl FnOnce(&mut SessionSummary),
    ) -> Result<SessionSummary, HostError> {
        let mut summary = {
            let _files = lock(&self.files);
            let mut summary = self.load_session(session_id)?;
            change(&mut summary);
            summary.last_run_status = None;
            self.save_session(&summary)?;
            summary
        };
        self.populate_last_run_statuses(std::slice::from_mut(&mut summary))?;
        Ok(summary)
    }

    pub fn session_rename(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<SessionSummary, HostError> {
        let title = session_title(title).ok_or(HostError::Invalid)?;
        self.update_session(session_id, |summary| summary.title = title)
    }

    pub fn session_set_pinned(
        &self,
        session_id: &str,
        pinned: bool,
    ) -> Result<SessionSummary, HostError> {
        self.update_session(session_id, |summary| {
            summary.pinned_at = pinned.then(now);
        })
    }

    pub fn session_set_archived(
        &self,
        session_id: &str,
        archived: bool,
    ) -> Result<SessionSummary, HostError> {
        self.update_session(session_id, |summary| {
            summary.archived_at = archived.then(now);
        })
    }

    /// Files the session under a workspace, or back to "unfiled" with `None`.
    pub fn session_move(
        &self,
        session_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<SessionSummary, HostError> {
        if let Some(id) = workspace_id {
            self.workspace(id)?;
        }
        self.update_session(session_id, |summary| {
            summary.workspace_id = workspace_id.map(str::to_owned);
        })
    }

    /// The session as a file for the user to save: its summary and events
    /// (JSON), or the conversation as text (Markdown). Nothing is written here.
    pub fn session_export(
        &self,
        session_id: &str,
        format: SessionExportFormat,
    ) -> Result<SessionExport, HostError> {
        let summary = self.load_session(session_id)?;
        let dir = self.session_dir(session_id)?;
        let events: Vec<StreamEvent> = {
            let _files = lock(&self.files);
            BufReader::new(fs::File::open(dir.join("events.jsonl"))?)
                .lines()
                .map_while(Result::ok)
                .filter_map(|line| serde_json::from_str(&line).ok())
                .collect()
        };
        let stem = export_file_stem(&summary.title);
        Ok(match format {
            SessionExportFormat::Json => SessionExport {
                file_name: format!("{stem}.json"),
                content: serde_json::to_string_pretty(&json!({
                    "session": summary,
                    "events": events,
                }))?,
            },
            SessionExportFormat::Markdown => SessionExport {
                file_name: format!("{stem}.md"),
                content: session_markdown(&summary, &events),
            },
        })
    }

    /// Writes the export into a folder the user just chose, never over an
    /// existing file ("name (2).md" and so on); returns the file's path.
    pub fn session_export_to(
        &self,
        session_id: &str,
        format: SessionExportFormat,
        directory: &str,
    ) -> Result<String, HostError> {
        let directory = Path::new(directory);
        if !directory.is_absolute() || !directory.is_dir() {
            return Err(HostError::Invalid);
        }
        let export = self.session_export(session_id, format)?;
        let (stem, extension) = export
            .file_name
            .rsplit_once('.')
            .unwrap_or((export.file_name.as_str(), ""));
        for attempt in 1..=999 {
            let name = if attempt == 1 {
                export.file_name.clone()
            } else {
                format!("{stem} ({attempt}).{extension}")
            };
            let path = directory.join(name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(export.content.as_bytes())?;
                    return Ok(path.to_string_lossy().into_owned());
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(HostError::Invalid)
    }

    fn workspaces_path(&self) -> PathBuf {
        self.root.join("agent").join("workspaces.json")
    }

    fn load_workspaces(&self) -> Result<Vec<Workspace>, HostError> {
        match fs::read(self.workspaces_path()) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(error) => Err(error.into()),
        }
    }

    fn save_workspaces(&self, workspaces: &[Workspace]) -> Result<(), HostError> {
        let path = self.workspaces_path();
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(workspaces)?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    fn workspace(&self, workspace_id: &str) -> Result<Workspace, HostError> {
        self.load_workspaces()?
            .into_iter()
            .find(|workspace| workspace.workspace_id == workspace_id)
            .ok_or(HostError::NotFound)
    }

    /// Workspaces in the order they were created.
    pub fn workspaces_list(&self) -> Result<Vec<Workspace>, HostError> {
        let _files = lock(&self.files);
        self.load_workspaces()
    }

    pub fn workspace_create(
        &self,
        name: &str,
        folder: Option<String>,
    ) -> Result<Workspace, HostError> {
        let name = workspace_name(name).ok_or(HostError::Invalid)?;
        let folder = workspace_folder(folder)?;
        let _files = lock(&self.files);
        let mut workspaces = self.load_workspaces()?;
        let workspace = Workspace {
            workspace_id: self.new_id("workspace"),
            name,
            folder,
            created_at: now(),
        };
        workspaces.push(workspace.clone());
        self.save_workspaces(&workspaces)?;
        Ok(workspace)
    }

    /// Renames a workspace and, when `folder` is given, rebinds its folder
    /// (`Some(None)` clears it).
    pub fn workspace_update(
        &self,
        workspace_id: &str,
        name: Option<&str>,
        folder: Option<Option<String>>,
    ) -> Result<Workspace, HostError> {
        let name = name
            .map(|name| workspace_name(name).ok_or(HostError::Invalid))
            .transpose()?;
        let folder = folder.map(workspace_folder).transpose()?;
        let _files = lock(&self.files);
        let mut workspaces = self.load_workspaces()?;
        let workspace = workspaces
            .iter_mut()
            .find(|workspace| workspace.workspace_id == workspace_id)
            .ok_or(HostError::NotFound)?;
        if let Some(name) = name {
            workspace.name = name;
        }
        if let Some(folder) = folder {
            workspace.folder = folder;
        }
        let updated = workspace.clone();
        self.save_workspaces(&workspaces)?;
        Ok(updated)
    }

    /// Deletes the workspace only; its sessions go back to "unfiled" and
    /// nothing in its folder is touched.
    pub fn workspace_delete(&self, workspace_id: &str) -> Result<(), HostError> {
        let _files = lock(&self.files);
        let mut workspaces = self.load_workspaces()?;
        let before = workspaces.len();
        workspaces.retain(|workspace| workspace.workspace_id != workspace_id);
        if workspaces.len() == before {
            return Err(HostError::NotFound);
        }
        for entry in fs::read_dir(self.sessions_dir())? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if !valid_id(&name) {
                continue;
            }
            if let Ok(mut summary) = self.load_session(&name) {
                if summary.workspace_id.as_deref() == Some(workspace_id) {
                    summary.workspace_id = None;
                    self.save_session(&summary)?;
                }
            }
        }
        self.save_workspaces(&workspaces)
    }

    /// Sessions persist until the user deletes them. Deletion removes the
    /// conversation and its run records (not tasks or downloaded files) and
    /// leaves a marker that holds only the id and deletion time. A deletion
    /// that failed half way (marker written, directory left) can be retried.
    pub fn session_delete(&self, session_id: &str) -> Result<(), HostError> {
        if !valid_id(session_id) {
            return Err(HostError::Invalid);
        }
        let dir = self.sessions_dir().join(session_id);
        if !dir.join("session.json").is_file() {
            return Err(HostError::NotFound);
        }
        // Parked runs are sealed here; a running worker seals itself on cancel.
        let parked: Vec<Parked<S>> = {
            let mut live = lock(&self.live);
            let ids: Vec<String> = live
                .iter()
                .filter(|(_, run)| run.session_id == session_id)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter()
                .filter_map(|id| live.remove(id))
                .filter_map(|run| {
                    run.cancel.cancel();
                    run.parked
                })
                .collect()
        };
        for run in parked {
            self.seal(run.state, "cancelled", None, false)?;
        }
        // Runs, plans and approvals are found by session id in the Store, not
        // by the summary's run list, which can miss a run whose summary write
        // failed. The marker is written in the same critical section.
        let marker = self.sessions_dir().join(format!("{session_id}.deleted"));
        let mut marked = false;
        let deleted = self.service.with_store(|store| -> Result<(), HostError> {
            fs::write(
                &marker,
                serde_json::to_vec(&json!({"session_id": session_id, "deleted_at": now()}))?,
            )?;
            marked = true;
            for run_id in store.agent_session_run_ids(session_id)? {
                match run::delete_run(&self.root, store, &run_id) {
                    Ok(()) | Err(run::RunError::Store(StoreError::NotFound)) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            store.delete_agent_session_plans(session_id)?;
            Ok(())
        });
        if !marked {
            return deleted.and_then(|inner| inner);
        }
        // With the marker written the session is deleted: its tasks are no
        // longer the Agent's (as a fresh read of the run records would
        // find), even if a later step below fails and the deletion is retried.
        if let Some(tasks) = lock(&self.agent_tasks).as_mut() {
            tasks.retain(|(session, _)| session != session_id);
        }
        let removed = deleted.and_then(|inner| inner).and_then(|()| {
            let mut files = lock(&self.files);
            for name in [
                "events.jsonl",
                "events.jsonl.tmp",
                "session.json",
                "session.json.tmp",
                "turn_snapshots.json",
                "turn_snapshots.json.tmp",
                "workspace_state.json",
                "workspace_state.json.tmp",
                "compaction.json",
                "compaction.json.tmp",
                "toolset_state.json",
                "toolset_state.json.tmp",
                "resume_checkpoint.json",
                "resume_checkpoint.json.tmp",
            ] {
                let path = dir.join(name);
                if path.is_file() {
                    fs::remove_file(path)?;
                }
            }
            fs::remove_dir(&dir)?;
            files.remove(session_id);
            Ok(())
        });
        self.announce_activity(&[]);
        removed
    }

    fn save_turn_aoi_snapshot(&self, session_id: &str, seq: u64, working_aoi: &Value) {
        let Ok(dir) = self.session_dir(session_id) else {
            return;
        };
        let _files = lock(&self.files);
        let path = dir.join("turn_snapshots.json");
        let mut entries: Vec<Value> = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        entries.push(json!({
            "seq": seq,
            "working_aoi": working_aoi,
        }));
        let tmp = dir.join("turn_snapshots.json.tmp");
        if let Ok(bytes) = serde_json::to_vec(&entries) {
            if fs::write(&tmp, bytes).is_ok() {
                let _ = fs::rename(tmp, path);
            }
        }
    }

    /// Truncates the session stream to strictly before `before_seq` (for edit &
    /// resend or rewind), updates `session.json`'s `run_ids`, and optionally
    /// restores the working AOI captured immediately prior to the truncated turn.
    pub fn session_truncate(
        &self,
        session_id: &str,
        before_seq: u64,
        restore_aoi: bool,
    ) -> Result<SessionTruncateResult, HostError> {
        if before_seq == 0 {
            return Err(HostError::Invalid);
        }
        let dir = self.session_dir(session_id)?;
        let parked: Vec<Parked<S>> = {
            let mut live = lock(&self.live);
            let ids: Vec<String> = live
                .iter()
                .filter(|(_, run)| run.session_id == session_id)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter()
                .filter_map(|id| live.remove(id))
                .filter_map(|run| {
                    run.cancel.cancel();
                    run.parked
                })
                .collect()
        };
        for run in parked {
            let _ = self.seal(run.state, "cancelled", None, false);
        }
        let (summary, cursor, snapshot_to_restore, kept_runs, dropped_runs) = {
            let mut files = lock(&self.files);
            let events_path = dir.join("events.jsonl");
            let mut kept: Vec<StreamEvent> = Vec::new();
            for line in BufReader::new(fs::File::open(&events_path)?).lines() {
                let event: StreamEvent = serde_json::from_str(&line?)?;
                if event.seq < before_seq {
                    kept.push(event);
                }
            }
            let tmp_events = dir.join("events.jsonl.tmp");
            {
                let mut out = fs::File::create(&tmp_events)?;
                for event in &kept {
                    serde_json::to_writer(&mut out, event)?;
                    out.write_all(b"\n")?;
                }
            }
            fs::rename(&tmp_events, &events_path)?;
            let cursor = kept.last().map_or(0, |event| event.seq);
            files.insert(session_id.to_owned(), cursor);

            let mut kept_runs = std::collections::HashSet::new();
            for event in &kept {
                if event.kind == "message"
                    && event.payload.get("role").and_then(Value::as_str) == Some("user")
                {
                    if let Some(run_id) = event.payload.get("run_id").and_then(Value::as_str) {
                        kept_runs.insert(run_id.to_owned());
                    }
                }
            }
            let mut summary = self.load_session(session_id)?;
            let dropped_runs: Vec<String> = summary
                .run_ids
                .iter()
                .filter(|id| !kept_runs.contains(id.as_str()))
                .cloned()
                .collect();
            summary.run_ids.retain(|id| kept_runs.contains(id));
            summary.updated_at = now();
            self.save_session(&summary)?;

            let snap_path = dir.join("turn_snapshots.json");
            let snapshots: Vec<Value> = fs::read(&snap_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            let mut earliest_removed: Option<(u64, Value)> = None;
            let mut kept_snaps = Vec::new();
            for item in snapshots {
                let seq = item.get("seq").and_then(Value::as_u64).unwrap_or(0);
                if seq < before_seq {
                    kept_snaps.push(item);
                } else if let Some(aoi_val) = item.get("working_aoi").cloned() {
                    if earliest_removed
                        .as_ref()
                        .is_none_or(|(best, _)| seq < *best)
                    {
                        earliest_removed = Some((seq, aoi_val));
                    }
                }
            }
            let tmp_snap = dir.join("turn_snapshots.json.tmp");
            if let Ok(bytes) = serde_json::to_vec(&kept_snaps) {
                if fs::write(&tmp_snap, bytes).is_ok() {
                    let _ = fs::rename(tmp_snap, snap_path);
                }
            }
            let _ = fs::remove_file(dir.join("resume_checkpoint.json"));
            let _ = fs::remove_file(dir.join("resume_checkpoint.json.tmp"));
            let _ = fs::remove_file(dir.join("toolset_state.json"));
            let _ = fs::remove_file(dir.join("toolset_state.json.tmp"));
            let comp_path = dir.join("compaction.json");
            if let Ok(bytes) = fs::read(&comp_path) {
                let valid = serde_json::from_slice::<SessionCompaction>(&bytes)
                    .ok()
                    .is_some_and(|c| {
                        c.compacted_run_ids.len() <= summary.run_ids.len()
                            && summary.run_ids.starts_with(&c.compacted_run_ids)
                    });
                if !valid {
                    let _ = fs::remove_file(&comp_path);
                    let _ = fs::remove_file(dir.join("compaction.json.tmp"));
                }
            }
            (
                summary,
                cursor,
                earliest_removed.map(|(_, aoi)| aoi),
                kept_runs,
                dropped_runs,
            )
        };
        let deleted_runs = self.service.with_store(|store| -> Result<(), HostError> {
            let mut to_delete: BTreeSet<String> = dropped_runs.into_iter().collect();
            for rid in store.agent_session_run_ids(session_id)? {
                if !kept_runs.contains(&rid) {
                    to_delete.insert(rid);
                }
            }
            for run_id in to_delete {
                match run::delete_run(&self.root, store, &run_id) {
                    Ok(()) | Err(run::RunError::Store(StoreError::NotFound)) => {}
                    Err(error) => {
                        let missing_dir = matches!(
                            &error,
                            run::RunError::Io(io_err) if io_err.kind() == std::io::ErrorKind::NotFound
                        );
                        if missing_dir {
                            let _ = store.delete_agent_run(&run_id);
                        } else {
                            return Err(error.into());
                        }
                    }
                }
            }
            Ok(())
        });
        deleted_runs.and_then(|inner| inner)?;
        *lock(&self.agent_tasks) = None;

        let mut aoi_restored = false;
        if restore_aoi {
            if let Some(snapshot_aoi) = snapshot_to_restore {
                if self
                    .service
                    .application_restore_working_aoi(&snapshot_aoi)
                    .is_ok()
                {
                    aoi_restored = true;
                    let _ = self.update_workspace_state(session_id, |ws| {
                        ws.session_id = session_id.to_owned();
                        ws.initialized = true;
                        ws.working_aoi = snapshot_aoi.clone();
                    });
                    let view = aoi::working_summary(&snapshot_aoi);
                    let state = view
                        .get("state")
                        .and_then(Value::as_str)
                        .unwrap_or("none")
                        .to_owned();
                    let display_name = view
                        .get("display_name")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let bbox = view
                        .get("bbox")
                        .and_then(Value::as_array)
                        .and_then(|arr| arr.iter().map(Value::as_f64).collect::<Option<Vec<_>>>());
                    self.notify(AgentHostNotification::AoiChanged {
                        session_id: session_id.to_owned(),
                        run_id: String::new(),
                        state,
                        display_name,
                        bbox,
                    });
                }
            }
        }
        self.announce_activity(&[session_id]);
        Ok(SessionTruncateResult {
            session: summary,
            cursor,
            aoi_restored,
        })
    }

    fn read_workspace_state_unlocked(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionWorkspaceState>, HostError> {
        let path = self.session_dir(session_id)?.join("workspace_state.json");
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = fs::read(path)?;
        let parsed: SessionWorkspaceState = serde_json::from_slice(&bytes)?;
        Ok(Some(parsed))
    }

    fn write_workspace_state_unlocked(
        &self,
        session_id: &str,
        state: &SessionWorkspaceState,
    ) -> Result<(), HostError> {
        let dir = self.session_dir(session_id)?;
        let path = dir.join("workspace_state.json");
        let tmp = dir.join("workspace_state.json.tmp");
        let mut bytes = serde_json::to_vec_pretty(state)?;
        bytes.push(b'\n');
        fs::write(&tmp, bytes)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    fn update_workspace_state(
        &self,
        session_id: &str,
        mutate: impl FnOnce(&mut SessionWorkspaceState),
    ) -> Result<SessionWorkspaceState, HostError> {
        let _files = lock(&self.files);
        let mut state = self
            .read_workspace_state_unlocked(session_id)?
            .unwrap_or_else(|| SessionWorkspaceState {
                session_id: session_id.to_owned(),
                initialized: false,
                working_aoi: Value::Null,
                active_batch_id: None,
                active_resource_id: None,
                active_batch_revision: None,
            });
        state.session_id = session_id.to_owned();
        mutate(&mut state);
        self.write_workspace_state_unlocked(session_id, &state)?;
        Ok(state)
    }

    fn ensure_workspace_state_initialized(
        &self,
        session_id: &str,
    ) -> Result<SessionWorkspaceState, HostError> {
        {
            let _files = lock(&self.files);
            if let Some(existing) = self.read_workspace_state_unlocked(session_id)? {
                if existing.initialized {
                    return Ok(existing);
                }
            }
        }
        let snapshot = self
            .service
            .application_working_aoi()
            .unwrap_or_else(|_| json!({"state": "none"}));
        let _files = lock(&self.files);
        let mut state = self
            .read_workspace_state_unlocked(session_id)?
            .unwrap_or_else(|| SessionWorkspaceState {
                session_id: session_id.to_owned(),
                initialized: false,
                working_aoi: Value::Null,
                active_batch_id: None,
                active_resource_id: None,
                active_batch_revision: None,
            });
        if !state.initialized {
            state.session_id = session_id.to_owned();
            state.initialized = true;
            state.working_aoi = snapshot;
            self.write_workspace_state_unlocked(session_id, &state)?;
        }
        Ok(state)
    }

    fn session_working_aoi_if_initialized(&self, session_id: &str) -> Option<Value> {
        let _files = lock(&self.files);
        self.read_workspace_state_unlocked(session_id)
            .ok()
            .flatten()
            .filter(|state| state.initialized)
            .map(|state| state.working_aoi)
    }

    pub fn session_workspace_state(
        &self,
        session_id: &str,
    ) -> Result<SessionWorkspaceState, HostError> {
        if !valid_id(session_id) {
            return Err(HostError::Invalid);
        }
        let _ = self.session_dir(session_id)?;
        {
            let _files = lock(&self.files);
            if let Some(state) = self.read_workspace_state_unlocked(session_id)? {
                return Ok(state);
            }
        }
        // Reconstruct active search batch metadata from older sessions that
        // predate `workspace_state.json` without overriding global AOI.
        let mut state = SessionWorkspaceState {
            session_id: session_id.to_owned(),
            initialized: false,
            working_aoi: Value::Null,
            active_batch_id: None,
            active_resource_id: None,
            active_batch_revision: None,
        };
        if let Ok(page) = self.stream_poll(session_id, 0) {
            for event in page.events.iter().rev() {
                if event.kind == "workspace_sync"
                    && event.payload.get("action").and_then(Value::as_str) == Some("search_updated")
                {
                    if let Some(batch_id) = event.payload.get("batch_id").and_then(Value::as_str) {
                        state.active_batch_id = Some(batch_id.to_owned());
                        state.active_resource_id = event
                            .payload
                            .get("resource_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        break;
                    }
                }
            }
        }
        if state.active_batch_id.is_none() {
            if let Ok(summary) = self.load_session(session_id) {
                'runs: for rid in summary.run_ids.iter().rev() {
                    if let Ok(trace) =
                        run::read_trace(&self.root.join("agent").join("runs").join(rid))
                    {
                        for event in trace.iter().rev() {
                            if event.kind == "search_batch" {
                                if let Some(batch_id) =
                                    event.payload.get("batch_id").and_then(Value::as_str)
                                {
                                    state.active_batch_id = Some(batch_id.to_owned());
                                    break 'runs;
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(batch_id) = state.active_batch_id.as_deref() {
            if let Ok(Some(header)) = self
                .service
                .with_store(|store| store.load_batch_header(batch_id))?
            {
                if state.active_resource_id.is_none() {
                    state.active_resource_id =
                        source_resource_id(&header.provider_id, &header.query);
                }
                state.active_batch_revision.get_or_insert(0);
            }
        }
        Ok(state)
    }

    pub fn session_set_working_aoi(
        &self,
        session_id: &str,
        request: Option<Value>,
        clear: Option<bool>,
        active_batch_id: Option<String>,
        active_resource_id: Option<String>,
        active_batch_revision: Option<u64>,
    ) -> Result<SessionWorkspaceState, HostError> {
        if !valid_id(session_id) {
            return Err(HostError::Invalid);
        }
        let _ = self.session_dir(session_id)?;
        let resolved_aoi = if clear == Some(true) {
            Some(json!({"state": "none"}))
        } else if let Some(req) = request {
            if req.get("geometry").is_some_and(Value::is_object) {
                Some(json!({
                    "state": "active",
                    "source_kind": req.get("source_kind").cloned().unwrap_or(Value::Null),
                    "geometry": req.get("geometry").cloned().unwrap_or(Value::Null),
                    "bbox": req.get("bbox").cloned().unwrap_or(Value::Null),
                    "crs": req.get("crs").cloned().unwrap_or_else(|| json!("EPSG:4326")),
                    "display_name": req.get("display_name").cloned().unwrap_or(Value::Null),
                }))
            } else {
                let preview = self
                    .service
                    .application_preview_working_aoi_set(&req)
                    .map_err(|_| HostError::Invalid)?;
                Some(preview)
            }
        } else {
            None
        };
        let aoi_mutated = resolved_aoi.is_some();
        let updated = self.update_workspace_state(session_id, |ws| {
            if let Some(aoi) = resolved_aoi {
                ws.initialized = true;
                ws.working_aoi = aoi;
            }
            if let Some(batch) = active_batch_id {
                let trimmed = batch.trim();
                if trimmed.is_empty() {
                    ws.active_batch_id = None;
                    if active_batch_revision.is_none() {
                        ws.active_batch_revision = None;
                    }
                } else {
                    ws.active_batch_id = Some(trimmed.to_owned());
                }
            }
            if let Some(res) = active_resource_id {
                let trimmed = res.trim();
                ws.active_resource_id = (!trimmed.is_empty()).then(|| trimmed.to_owned());
            }
            if active_batch_revision.is_some() {
                ws.active_batch_revision = active_batch_revision;
            }
        })?;
        if aoi_mutated {
            self.supersede_parked_in_session(session_id, "plan_superseded_by_aoi_change");
        }
        Ok(updated)
    }

    fn append_event(&self, session_id: &str, kind: &str, payload: Value) -> Result<u64, HostError> {
        let dir = self.session_dir(session_id)?;
        let event = {
            let mut files = lock(&self.files);
            let path = dir.join("events.jsonl");
            let seq = match files.get(session_id) {
                Some(last) => last + 1,
                None => {
                    BufReader::new(fs::File::open(&path)?)
                        .lines()
                        .map_while(Result::ok)
                        .filter_map(|line| serde_json::from_str::<StreamEvent>(&line).ok())
                        .map(|existing| existing.seq)
                        .max()
                        .unwrap_or(0)
                        + 1
                }
            };
            let event = StreamEvent {
                seq,
                kind: kind.into(),
                payload,
            };
            let mut file = OpenOptions::new().append(true).open(&path)?;
            serde_json::to_writer(&mut file, &event)?;
            file.write_all(b"\n")?;
            files.insert(session_id.to_owned(), seq);
            event
        };
        let seq = event.seq;
        self.notify(AgentHostNotification::StreamEvent {
            session_id: session_id.to_owned(),
            event,
        });
        Ok(seq)
    }

    /// Events with `seq > cursor`, oldest first, at most one page.
    pub fn stream_poll(&self, session_id: &str, cursor: u64) -> Result<StreamPage, HostError> {
        let dir = self.session_dir(session_id)?;
        let _files = lock(&self.files);
        let mut events = Vec::new();
        for line in BufReader::new(fs::File::open(dir.join("events.jsonl"))?).lines() {
            let event: StreamEvent = serde_json::from_str(&line?)?;
            if event.seq > cursor {
                events.push(event);
                if events.len() == POLL_PAGE {
                    break;
                }
            }
        }
        Ok(StreamPage {
            cursor: events.last().map_or(cursor, |event| event.seq),
            events,
        })
    }

    fn load_resume_checkpoint(&self, session_id: &str) -> Option<ResumeCheckpoint> {
        let dir = self.session_dir(session_id).ok()?;
        let _files = lock(&self.files);
        let bytes = fs::read(dir.join("resume_checkpoint.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn save_resume_checkpoint(&self, session_id: &str, cp: &ResumeCheckpoint) {
        let Ok(dir) = self.session_dir(session_id) else {
            return;
        };
        let Ok(val) = serde_json::to_value(cp) else {
            return;
        };
        let Ok(safe) = run::safe_value(&val) else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec_pretty(&safe) else {
            return;
        };
        let _files = lock(&self.files);
        let tmp = dir.join("resume_checkpoint.json.tmp");
        if fs::write(&tmp, bytes).is_ok() {
            let _ = fs::rename(tmp, dir.join("resume_checkpoint.json"));
        }
    }

    fn clear_resume_checkpoint(&self, session_id: &str) {
        if let Ok(dir) = self.session_dir(session_id) {
            let _files = lock(&self.files);
            let _ = fs::remove_file(dir.join("resume_checkpoint.json"));
        }
    }

    pub fn send(
        self: &Arc<Self>,
        session_id: &str,
        text: &str,
        context: SendContext,
    ) -> Result<String, HostError> {
        let text = text.trim();
        if (text.is_empty() && context.attachments.is_empty())
            || text.chars().count() > MAX_TEXT_CHARS
            || context.attachments.len() > MAX_ATTACHMENTS
        {
            return Err(HostError::Invalid);
        }
        let non_attachment_ctx = SendContext {
            aoi: context.aoi.clone(),
            time_range: context.time_range.clone(),
            source: context.source.clone(),
            model: context.model.clone(),
            attachments: Vec::new(),
        };
        if serde_json::to_vec(&non_attachment_ctx)?.len() > MAX_CONTEXT_BYTES {
            return Err(HostError::Invalid);
        }
        let max_total = if context.attachments.is_empty() {
            MAX_CONTEXT_BYTES
        } else {
            MAX_CONTEXT_WITH_ATTACHMENTS_BYTES
        };
        if serde_json::to_vec(&context)?.len() > max_total {
            return Err(HostError::Invalid);
        }
        for att in &context.attachments {
            let kind = att.kind.trim();
            if !matches!(kind, "image" | "text" | "file")
                || att.name.trim().is_empty()
                || att.name.chars().count() > 256
            {
                return Err(HostError::Invalid);
            }
            if kind == "image" {
                let raw = att.data_base64.as_deref().ok_or(HostError::Invalid)?;
                if insar_agent_model::privacy::normalize_image_attachment(&att.mime_type, raw)
                    .is_none()
                {
                    return Err(HostError::Invalid);
                }
            } else if let Some(ref content) = att.text_content {
                if content.chars().count() > MAX_TEXT_ATTACHMENT_INPUT_CHARS {
                    return Err(HostError::Invalid);
                }
            }
        }
        let mut summary = self.load_session(session_id)?;
        let run_id = self.new_id("run");
        let cancel = CancellationToken::new();
        // Busy check and registration are one critical section, so two
        // concurrent sends cannot both pass the check while a model opens.
        // If the session only has a run parked on an unapproved card, supersede
        // that parked plan immediately so the user's new turn can proceed.
        let superseded: Vec<Parked<S>> = {
            let mut live = lock(&self.live);
            if live
                .values()
                .any(|run| run.session_id == session_id && run.parked.is_none())
            {
                return Err(HostError::Busy);
            }
            let popped = Self::take_parked_for_session_locked(&mut live, session_id);
            live.insert(
                run_id.clone(),
                Live {
                    session_id: session_id.to_owned(),
                    cancel: cancel.clone(),
                    parked: None,
                },
            );
            popped
        };
        for parked in superseded {
            self.finalize_superseded_parked(parked, "plan_superseded_by_new_turn");
        }
        match self.begin_run(&run_id, &mut summary, text, context) {
            Ok(state) => {
                let host = Arc::clone(self);
                std::thread::spawn(move || host.drive(state, cancel));
                self.announce_activity(&[session_id]);
                Ok(run_id)
            }
            Err(error) => {
                lock(&self.live).remove(&run_id);
                // Another announcement may have counted the placeholder
                // while the model opened; say it is gone.
                self.announce_activity(&[session_id]);
                Err(error)
            }
        }
    }

    /// The session's earlier runs for the next model request: user text,
    /// assistant text and conclusion or error code from the session's events,
    /// facts from each run record (`history::messages` digests them). A run
    /// record that cannot be read leaves that run without facts.
    fn load_turn_messages(&self, run_id: &str) -> Option<Vec<Message>> {
        if !valid_id(run_id) {
            return None;
        }
        let path = self
            .root
            .join("agent")
            .join("runs")
            .join(run_id)
            .join("turn_messages.json");
        let bytes = fs::read(path).ok()?;
        let msgs: Vec<Message> = serde_json::from_slice(&bytes).ok()?;
        if msgs.len() < 2
            || msgs.first().map(|m| m.role.as_str()) != Some("user")
            || msgs.last().map(|m| m.role.as_str()) != Some("assistant")
            || msgs.last().is_some_and(|m| m.content.trim().is_empty())
            || validate_turn_messages_safe(&msgs).is_err()
        {
            return None;
        }
        Some(msgs)
    }

    fn persist_turn_messages(
        &self,
        run_id: &str,
        checkpoint: &Checkpoint,
        turn_start_msg_index: usize,
        initial_user_msg: Option<&Message>,
        closing_assistant: Option<&str>,
    ) {
        if !valid_id(run_id) {
            return;
        }
        let raw_slice = checkpoint
            .messages
            .get(turn_start_msg_index..)
            .unwrap_or(&[]);
        let mut turn_msgs: Vec<Message> = if !raw_slice.is_empty() {
            raw_slice.to_vec()
        } else if let Some(u) = initial_user_msg {
            vec![u.clone()]
        } else {
            return;
        };
        turn_msgs = turn_msgs
            .into_iter()
            .map(downgrade_user_multimodal)
            .collect();
        sanitize_incomplete_tool_tail(&mut turn_msgs);
        if turn_msgs.first().map(|m| m.role.as_str()) != Some("user") {
            return;
        }
        let clean_closing = closing_assistant.map(|text| {
            let trimmed = without_think(text).trim().to_owned();
            if trimmed.is_empty() {
                "（已完成）".to_owned()
            } else {
                trimmed
            }
        });
        if let Some(last) = turn_msgs.last_mut().filter(|m| m.role == "assistant") {
            if let Some(closing) = clean_closing {
                if last.content.trim().is_empty() {
                    last.content = closing;
                } else if last.content.trim() != closing {
                    last.content = format!("{}\n\n{}", last.content.trim_end(), closing);
                }
            } else if last.content.trim().is_empty() {
                last.content = "[上一轮未完成]".to_owned();
            }
        } else {
            turn_msgs.push(Message {
                role: "assistant".into(),
                content: clean_closing.unwrap_or_else(|| "[上一轮未完成]".to_owned()),
            });
        }
        let scrubbed = outbound::messages(&turn_msgs);
        if validate_turn_messages_safe(&scrubbed).is_err() {
            return;
        }
        let dir = self.root.join("agent").join("runs").join(run_id);
        if !dir.is_dir() {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(&scrubbed) else {
            return;
        };
        let tmp = dir.join("turn_messages.json.tmp");
        if fs::write(&tmp, bytes).is_ok() {
            let _ = fs::rename(tmp, dir.join("turn_messages.json"));
        }
    }

    fn load_compaction(&self, session_id: &str) -> Option<SessionCompaction> {
        let dir = self.session_dir(session_id).ok()?;
        let _files = lock(&self.files);
        let bytes = fs::read(dir.join("compaction.json")).ok()?;
        let parsed: SessionCompaction = serde_json::from_slice(&bytes).ok()?;
        if parsed.summary_messages.len() != 2
            || parsed.summary_messages[0].role != "user"
            || parsed.summary_messages[1].role != "assistant"
            || parsed.summary_messages[1].content.trim().is_empty()
            || validate_turn_messages_safe(&parsed.summary_messages).is_err()
        {
            return None;
        }
        Some(parsed)
    }

    fn save_compaction(&self, session_id: &str, compaction: &SessionCompaction) {
        let Ok(dir) = self.session_dir(session_id) else {
            return;
        };
        if validate_turn_messages_safe(&compaction.summary_messages).is_err() {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(compaction) else {
            return;
        };
        let _files = lock(&self.files);
        let tmp = dir.join("compaction.json.tmp");
        if fs::write(&tmp, bytes).is_ok() {
            let _ = fs::rename(tmp, dir.join("compaction.json"));
        }
    }

    fn load_session_toolsets(&self, session_id: &str) -> Option<BTreeSet<String>> {
        let dir = self.session_dir(session_id).ok()?;
        let _files = lock(&self.files);
        let bytes = fs::read(dir.join("toolset_state.json")).ok()?;
        let parsed: SessionToolsetState = serde_json::from_slice(&bytes).ok()?;
        Some(parsed.active_toolsets)
    }

    fn save_session_toolsets(&self, session_id: &str, toolsets: &BTreeSet<String>) {
        let Ok(dir) = self.session_dir(session_id) else {
            return;
        };
        let state = SessionToolsetState {
            active_toolsets: toolsets.clone(),
        };
        let Ok(bytes) = serde_json::to_vec(&state) else {
            return;
        };
        let _files = lock(&self.files);
        let tmp = dir.join("toolset_state.json.tmp");
        if fs::write(&tmp, bytes).is_ok() {
            let _ = fs::rename(tmp, dir.join("toolset_state.json"));
        }
    }

    fn load_earlier_messages(
        &self,
        session_id: &str,
        run_ids: &[String],
    ) -> (Vec<Message>, usize, BTreeSet<String>) {
        if run_ids.is_empty() {
            return (Vec::new(), 0, BTreeSet::new());
        }
        let (compacted_prefix_len, mut summary_messages, mut compacted_toolsets) = self
            .load_compaction(session_id)
            .filter(|c| {
                !c.compacted_run_ids.is_empty()
                    && c.compacted_run_ids.len() <= run_ids.len()
                    && run_ids.starts_with(&c.compacted_run_ids)
            })
            .map_or((0usize, Vec::new(), BTreeSet::new()), |c| {
                (
                    c.compacted_run_ids.len(),
                    c.summary_messages,
                    c.compacted_toolsets,
                )
            });

        let uncompacted_ids = &run_ids[compacted_prefix_len..];
        let mut uncompacted_turns: Vec<Vec<Message>> = vec![Vec::new(); uncompacted_ids.len()];
        let mut missing_indices = Vec::new();
        for (idx, rid) in uncompacted_ids.iter().enumerate() {
            if let Some(msgs) = self.load_turn_messages(rid) {
                uncompacted_turns[idx] = msgs;
            } else {
                missing_indices.push(idx);
            }
        }
        if !missing_indices.is_empty() {
            let missing_ids: Vec<String> = missing_indices
                .iter()
                .map(|&idx| uncompacted_ids[idx].clone())
                .collect();
            let mut legacy_turns = self.earlier_turns(session_id, &missing_ids);
            for turn in &mut legacy_turns {
                self.load_turn_record(turn);
            }
            for &idx in &missing_indices {
                let rid = &uncompacted_ids[idx];
                if let Some(turn) = legacy_turns.iter().find(|t| &t.run_id == rid) {
                    uncompacted_turns[idx] = history::single_turn_legacy_messages(turn);
                }
            }
        }

        let summary_chars: usize = summary_messages
            .iter()
            .map(|m| m.content.chars().count())
            .sum();
        let turn_chars: Vec<usize> = uncompacted_turns
            .iter()
            .map(|msgs| msgs.iter().map(|m| m.content.chars().count()).sum())
            .collect();
        let total_chars = summary_chars + turn_chars.iter().sum::<usize>();
        let non_empty_turns = usize::from(!summary_messages.is_empty())
            + uncompacted_turns.iter().filter(|m| !m.is_empty()).count();
        let history_turns_count = if non_empty_turns > 0 {
            run_ids.len()
        } else {
            0
        };

        if total_chars <= history::APPEND_HISTORY_COMPACT_TRIGGER_CHARS {
            let mut out = summary_messages;
            for msgs in uncompacted_turns {
                out.extend(msgs);
            }
            return (out, history_turns_count, compacted_toolsets);
        }

        let mut kept_start = uncompacted_turns.len();
        let mut kept_chars = 0usize;
        for idx in (0..uncompacted_turns.len()).rev() {
            let tc = turn_chars[idx];
            if tc == 0 && kept_start == uncompacted_turns.len() {
                kept_start = idx;
                continue;
            }
            let must_keep_latest = kept_start == uncompacted_turns.len()
                || uncompacted_turns[kept_start..].iter().all(Vec::is_empty);
            if must_keep_latest || kept_chars + tc <= history::APPEND_HISTORY_COMPACT_TARGET_CHARS {
                kept_chars += tc;
                kept_start = idx;
            } else {
                break;
            }
        }

        if kept_start == 0 {
            // Single uncompacted turn (or empty tail) alone exceeds trigger: trim it in place
            // rather than emitting a bogus 0-turn compaction summary.
            let budget = history::APPEND_HISTORY_COMPACT_TRIGGER_CHARS
                .saturating_sub(summary_chars)
                .max(16_000);
            for msgs in &mut uncompacted_turns {
                if !msgs.is_empty() {
                    clip_oversized_single_turn(msgs, budget);
                }
            }
            let mut out = summary_messages;
            for msgs in uncompacted_turns {
                out.extend(msgs);
            }
            return (out, history_turns_count, compacted_toolsets);
        }

        for msgs in &uncompacted_turns[..kept_start] {
            compacted_toolsets.extend(collect_toolsets_from_messages(msgs));
        }
        let to_compact_ids: Vec<String> = uncompacted_ids[..kept_start].to_vec();
        let mut newly_compacted_turns = self.earlier_turns(session_id, &to_compact_ids);
        for turn in &mut newly_compacted_turns {
            self.load_turn_record(turn);
            compacted_toolsets.extend(detect_toolsets_from_text(&turn.user));
        }
        let prior_assistant_digest = summary_messages
            .get(1)
            .filter(|m| m.role == "assistant")
            .map(|m| m.content.as_str());
        let total_compacted_turns = compacted_prefix_len + kept_start;
        let raw_summary = history::build_compaction_summary_pair(
            total_compacted_turns,
            prior_assistant_digest,
            &newly_compacted_turns,
        );
        let scrubbed_summary = outbound::messages(&raw_summary);
        if validate_turn_messages_safe(&scrubbed_summary).is_ok() {
            let compaction = SessionCompaction {
                compacted_run_ids: run_ids[..total_compacted_turns].to_vec(),
                summary_messages: scrubbed_summary.clone(),
                compacted_toolsets: compacted_toolsets.clone(),
            };
            self.save_compaction(session_id, &compaction);
            summary_messages = scrubbed_summary;
        }
        let summary_chars_after: usize = summary_messages
            .iter()
            .map(|m| m.content.chars().count())
            .sum();
        let mut tail_turns = uncompacted_turns.split_off(kept_start);
        let tail_chars: usize = tail_turns
            .iter()
            .flatten()
            .map(|m| m.content.chars().count())
            .sum();
        if summary_chars_after + tail_chars > history::APPEND_HISTORY_COMPACT_TRIGGER_CHARS {
            let budget = history::APPEND_HISTORY_COMPACT_TRIGGER_CHARS
                .saturating_sub(summary_chars_after)
                .max(16_000);
            for msgs in &mut tail_turns {
                if !msgs.is_empty() {
                    clip_oversized_single_turn(msgs, budget);
                }
            }
        }
        let mut out = summary_messages;
        for msgs in tail_turns {
            out.extend(msgs);
        }
        (out, history_turns_count, compacted_toolsets)
    }

    fn earlier_turns(&self, session_id: &str, run_ids: &[String]) -> Vec<history::Turn> {
        let mut turns: Vec<(String, history::Turn)> = run_ids
            .iter()
            .map(|id| (id.clone(), history::Turn::default()))
            .collect();
        if let Ok(dir) = self.session_dir(session_id) {
            let _files = lock(&self.files);
            if let Ok(file) = fs::File::open(dir.join("events.jsonl")) {
                for line in BufReader::new(file).lines().map_while(Result::ok) {
                    let Ok(event) = serde_json::from_str::<StreamEvent>(&line) else {
                        continue;
                    };
                    let run_id = event.payload.get("run_id").and_then(Value::as_str);
                    let Some((_, turn)) =
                        turns.iter_mut().find(|(id, _)| Some(id.as_str()) == run_id)
                    else {
                        continue;
                    };
                    let field = |key: &str| {
                        event
                            .payload
                            .get(key)
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    };
                    match (event.kind.as_str(), field("role").as_deref()) {
                        ("message", Some("user")) => {
                            let raw_text = field("text").unwrap_or_default();
                            if raw_text.trim().is_empty()
                                && event
                                    .payload
                                    .pointer("/context/attachments")
                                    .and_then(Value::as_array)
                                    .is_some_and(|a| !a.is_empty())
                            {
                                turn.user = "[附带图片或文件]".to_owned();
                            } else {
                                turn.user = raw_text;
                            }
                        }
                        ("message", Some("assistant")) => turn.assistant.extend(field("text")),
                        ("conclusion", _) => turn.conclusion = field("text"),
                        ("progress_card", _)
                            if field("code")
                                .as_deref()
                                .is_some_and(|c| c.starts_with("plan_superseded")) =>
                        {
                            turn.error = field("code");
                        }
                        ("error", _) => turn.error = field("code"),
                        _ => {}
                    }
                }
            }
        }
        turns
            .into_iter()
            .filter(|(_, turn)| !turn.user.trim().is_empty())
            .map(|(run_id, mut turn)| {
                turn.run_id = run_id;
                turn
            })
            .collect()
    }

    /// Reads one earlier turn's run record and the batches it names; called
    /// by `history::messages_loading` only for the turns the budget reaches.
    fn load_turn_record(&self, turn: &mut history::Turn) {
        turn.events = run::read_trace(&self.root.join("agent").join("runs").join(&turn.run_id))
            .unwrap_or_default();
        let named = history::named_batches(&turn.events);
        // #22 B slice 2: the digest needs the products the record actually
        // names (the read items) plus, for a batch the run froze but never
        // read, its newest products. Both stay bounded: the named ids come
        // from the record itself, and the unread case reads one page (at
        // most 250) instead of the whole frozen candidate array.
        let named_ids = history::named_items(&turn.events);
        turn.batches = self
            .service
            .with_store(|store| {
                named
                    .iter()
                    .filter_map(|batch_id| {
                        let header = store.load_batch_header(batch_id).ok()??;
                        let mut items: Vec<serde_json::Value> = store
                            .load_batch_candidate_items(
                                batch_id,
                                &named_ids.get(batch_id).cloned().unwrap_or_default(),
                            )
                            .ok()
                            .flatten()?
                            .into_iter()
                            .filter_map(|item| serde_json::to_value(item).ok())
                            .collect();
                        if items.is_empty() {
                            items = store
                                .load_batch_candidate_page(
                                    batch_id,
                                    0,
                                    insar_store::MAX_WORKBENCH_PAGE_SIZE,
                                )
                                .ok()?
                                .items
                                .into_iter()
                                .filter_map(|item| serde_json::to_value(item).ok())
                                .collect();
                        }
                        Some((header.batch_id, items))
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    /// Opens the model (system credential read) and the run record for a run
    /// whose placeholder is already registered.
    fn begin_run(
        &self,
        run_id: &str,
        summary: &mut SessionSummary,
        text: &str,
        context: SendContext,
    ) -> Result<RunState<S>, HostError> {
        let session_id = summary.session_id.clone();
        // W3g: a directory named in the message is taken out before anything
        // is built for the model, which sees a placeholder instead.
        let (message_dir, outbound_text) = match message_dir::extract(text) {
            Some((dir, scrubbed)) => (Some(dir), scrubbed),
            None => (None, text.to_owned()),
        };
        // Earlier turns of this session, read before this message is added.
        let (earlier, history_turns_count, compacted_toolsets) =
            self.load_earlier_messages(&session_id, &summary.run_ids);
        let resumed_from = if is_resume_prompt(text) {
            self.load_resume_checkpoint(&session_id)
                .filter(|cp| !cp.answer_delivered)
        } else {
            self.clear_resume_checkpoint(&session_id);
            None
        };
        let casual_fast_path = earlier.is_empty()
            && context.attachments.is_empty()
            && resumed_from.is_none()
            && !outbound_text.trim().is_empty()
            && is_pure_casual_greeting(&outbound_text);
        let opened =
            self.models
                .open_for_turn(&session_id, context.model.as_ref(), casual_fast_path)?;
        let active_toolsets = if casual_fast_path {
            BTreeSet::new()
        } else {
            let stored_toolsets = self.load_session_toolsets(&session_id);
            let mut set = stored_toolsets.clone().unwrap_or_else(|| {
                let mut reconstructed = compacted_toolsets;
                reconstructed.extend(collect_toolsets_from_messages(&earlier));
                reconstructed
            });
            set.extend(detect_toolsets_from_text(text));
            if let Some(ref cp) = resumed_from {
                set.extend(detect_toolsets_from_text(&cp.original_user_goal));
            }
            if stored_toolsets.as_ref() != Some(&set) && !set.is_empty() {
                self.save_session_toolsets(&session_id, &set);
            }
            set
        };
        let row = AgentRunRow {
            run_id: run_id.to_owned(),
            session_id: session_id.clone(),
            origin: "in_app".into(),
            client_label: None,
            started_at: now(),
            ended_at: None,
            status: "running".into(),
            model_route: Some(opened.route.clone()),
            catalog_version: 1,
            catalog_sha256: catalog_sha256(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            source_commit: "desktop".into(),
            store_revision_start: 0,
            store_revision_end: None,
            record_dir: format!("agent/runs/{run_id}"),
            trace_sha256: None,
            reproduces_run_id: None,
            reasoning_off_refused: false,
            truncated: false,
            usage_prompt: None,
            usage_completion: None,
        };
        let pre_turn_aoi = self
            .ensure_workspace_state_initialized(&session_id)
            .map(|ws| ws.working_aoi)
            .unwrap_or_else(|_| {
                self.service
                    .application_working_aoi()
                    .unwrap_or_else(|_| json!({"state": "none"}))
            });
        let resolved_context_aoi = context.aoi.as_ref().map(|ctx_aoi| {
            let is_working_ref = ctx_aoi
                .get("geometry")
                .and_then(Value::as_object)
                .is_some_and(|g| {
                    g.len() == 1 && g.get("ref").and_then(Value::as_str) == Some(aoi::WORKING_REF)
                });
            if is_working_ref {
                if let Some(work_geom) = aoi::geometry(&pre_turn_aoi) {
                    let mut hydrated = ctx_aoi.clone();
                    if let Some(obj) = hydrated.as_object_mut() {
                        obj.insert("geometry".to_owned(), Value::Object(work_geom.clone()));
                    }
                    return hydrated;
                }
            }
            ctx_aoi.clone()
        });
        let mut writer = self.service.with_store(|store| {
            if self.session_gone(&session_id) {
                return Err(HostError::NotFound);
            }
            Ok(RunWriter::start(&self.root, store, &row)?)
        })??;
        let prepared = (|| -> Result<(Checkpoint, usize, Option<Message>), HostError> {
            // The trace keeps sizes, not user prose that may embed local paths;
            // a directory taken from the message is named by its salted hash.
            let mut request = json!({
                "chars": text.chars().count(),
                "has_aoi": resolved_context_aoi.is_some(),
                "attachment_count": context.attachments.len(),
            });
            if let Some(dir) = &message_dir {
                request["message_dir_sha256"] = json!(path_digest(run_id, dir));
            }
            // Sizes only: what history went with this message.
            request["history_turns"] = json!(history_turns_count);
            request["history_chars"] = json!(earlier
                .iter()
                .map(|message| message.content.chars().count())
                .sum::<usize>());
            if casual_fast_path {
                request["fast_path"] = json!("casual_greeting");
                request["tool_count"] = json!(0);
            }
            writer.append("request", &request)?;
            // Re-read under the files lock: a mode set while the model opened
            // must not be overwritten by the summary loaded before.
            {
                let _files = lock(&self.files);
                let mut current = self.load_session(&session_id)?;
                current.run_ids.push(run_id.to_owned());
                current.updated_at = now();
                if let Some(dir) = message_dir
                    .as_deref()
                    .filter(|dir| permission::valid_output_dir(dir))
                {
                    current.remembered_output_dir = Some(dir.to_owned());
                }
                self.save_session(&current)?;
                *summary = current;
            }
            let mut outbound_images: Vec<Value> = Vec::new();
            let mut outbound_text_attachments: Vec<String> = Vec::new();
            let mut persisted_attachments: Vec<SendAttachment> =
                Vec::with_capacity(context.attachments.len());
            for att in &context.attachments {
                let safe_name = history::outbound_text(
                    att.name
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or(&att.name)
                        .trim(),
                );
                let safe_name = if safe_name.is_empty() {
                    "attachment".to_owned()
                } else {
                    safe_name
                };
                if att.kind.trim() == "image" {
                    if let Some(raw_b64) = att.data_base64.as_deref() {
                        if let Some((norm_mime, norm_b64)) =
                            insar_agent_model::privacy::normalize_image_attachment(
                                &att.mime_type,
                                raw_b64,
                            )
                        {
                            outbound_images.push(json!({
                                "mime_type": norm_mime,
                                "data_base64": norm_b64,
                            }));
                            let keep_b64 = (norm_b64.len()
                                <= MAX_PERSISTED_ATTACHMENT_BASE64_BYTES)
                                .then_some(norm_b64);
                            persisted_attachments.push(SendAttachment {
                                kind: "image".to_owned(),
                                name: safe_name,
                                mime_type: norm_mime,
                                data_base64: keep_b64,
                                text_content: None,
                                size_bytes: att.size_bytes,
                            });
                        }
                    }
                } else {
                    let snippet = att
                        .text_content
                        .as_deref()
                        .map(outbound::redact_words)
                        .unwrap_or_default();
                    let mime = history::outbound_text(att.mime_type.trim());
                    if !snippet.trim().is_empty() {
                        outbound_text_attachments.push(format!(
                            "[附件：{safe_name} ({mime})]\n{}",
                            clip_summary_text(&snippet, MAX_TEXT_ATTACHMENT_OUTBOUND_CHARS)
                        ));
                    } else {
                        outbound_text_attachments.push(format!("[附件：{safe_name} ({mime})]"));
                    }
                    persisted_attachments.push(SendAttachment {
                        kind: att.kind.trim().to_owned(),
                        name: safe_name,
                        mime_type: mime,
                        data_base64: None,
                        text_content: (!snippet.is_empty())
                            .then(|| clip_summary_text(&snippet, 4096)),
                        size_bytes: att.size_bytes,
                    });
                }
            }
            let persisted_context = SendContext {
                aoi: context.aoi.clone(),
                time_range: context.time_range.clone(),
                source: context.source.clone(),
                model: context.model.clone(),
                attachments: persisted_attachments,
            };
            let user_seq = self.append_event(
                &session_id,
                "message",
                json!({"role":"user","run_id":run_id,"text":text,"context":persisted_context}),
            )?;
            self.save_turn_aoi_snapshot(&session_id, user_seq, &pre_turn_aoi);
            // The model gets the AOI's name, bbox, area and a reference, never
            // its geometry; search steps name it as {"ref":"context_aoi"}.
            let context_line = serde_json::to_string(&json!({
                "intent_mode": "passive_workspace_snapshot",
                "aoi": resolved_context_aoi.as_ref().map(aoi::context_summary),
                "time_range": context.time_range,
                "source": context.source,
            }))?;
            let sys_prompt = if casual_fast_path {
                CASUAL_SYSTEM_PROMPT.to_owned()
            } else {
                match self.service.application_extension_system_prompt() {
                    Some(ext) if !ext.trim().is_empty() => format!("{SYSTEM_PROMPT}\n{ext}"),
                    _ => SYSTEM_PROMPT.to_owned(),
                }
            };
            let mut messages = vec![Message {
                role: "system".into(),
                content: sys_prompt,
            }];
            messages.extend(earlier);
            let turn_start_msg_index = messages.len();
            let mut user_body = if outbound_text.is_empty() && !context.attachments.is_empty() {
                "请结合附带的图片或文件内容进行分析与回答。".to_owned()
            } else {
                outbound_text
            };
            if !outbound_text_attachments.is_empty() {
                user_body.push_str("\n\n");
                user_body.push_str(&outbound_text_attachments.join("\n\n"));
            }
            let has_workspace_context = resolved_context_aoi.is_some()
                || context.time_range.is_some()
                || context.source.is_some();
            if !casual_fast_path || has_workspace_context {
                user_body.push_str("\n\n[context] ");
                user_body.push_str(&context_line);
            }
            if let Some(ref cp) = resumed_from {
                user_body.push_str("\n\n");
                user_body.push_str(&format_resume_checkpoint_block(cp));
            }
            if outbound_images.is_empty() {
                messages.push(Message {
                    role: "user".into(),
                    content: user_body,
                });
            } else {
                messages.push(Message {
                    role: "user_multimodal".into(),
                    content: json!({
                        "text": user_body,
                        "images": outbound_images,
                    })
                    .to_string(),
                });
            }
            let initial_user_msg = messages.last().cloned();
            Ok((
                Checkpoint::new(messages),
                turn_start_msg_index,
                initial_user_msg,
            ))
        })();
        let (checkpoint, turn_start_msg_index, initial_user_msg) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                // The run row exists: seal it rather than leave it running.
                let _ = self
                    .service
                    .with_store(|store| writer.finish(store, run_id, "failed", &now(), 0));
                return Err(error);
            }
        };
        Ok(RunState {
            run_id: run_id.to_owned(),
            session_id,
            route: opened.route,
            model: opened.model,
            executor: Executor::new(HostPort::new(self.service.clone())),
            writer,
            checkpoint,
            turn_start_msg_index,
            initial_user_msg,
            active_toolsets,
            local: LocalContext {
                aoi: resolved_context_aoi,
                message_dir,
                user_text: text.to_owned(),
                resumed_from,
            },
            origin: "in_app",
            circuit_breaker: SearchCircuitBreaker::default(),
            consecutive_failures: 0,
            verified_admin_codes: std::collections::HashSet::new(),
            goal_manifest: None,
            casual_fast_path,
        })
    }

    fn drive(self: &Arc<Self>, state: RunState<S>, cancel: CancellationToken) {
        #[derive(Default)]
        struct StepStreamBuf {
            step: Option<u64>,
            text: String,
            thinking: String,
            started_at_ms: Option<u64>,
            thinking_ms: Option<u64>,
            thinking_started: bool,
            thinking_ended: bool,
        }

        let RunState {
            run_id,
            session_id,
            route,
            model,
            executor,
            writer,
            checkpoint,
            turn_start_msg_index,
            initial_user_msg,
            active_toolsets,
            local,
            origin,
            circuit_breaker,
            consecutive_failures,
            verified_admin_codes,
            goal_manifest,
            casual_fast_path,
        } = state;
        let max_steps = self.max_steps();
        let initial_steps = checkpoint.steps as u64;
        let active_toolsets = Arc::new(Mutex::new(active_toolsets));
        let step_calls = AtomicU64::new(initial_steps);
        let current_step = Arc::new(AtomicU64::new(initial_steps));
        let dispatcher = TurnDispatcher {
            host: &**self,
            run_id: run_id.clone(),
            session_id: session_id.clone(),
            route: route.clone(),
            executor: Mutex::new(executor),
            writer: Mutex::new(writer),
            proposed: Mutex::new(None),
            local,
            failure: Mutex::new(None),
            circuit_breaker: Mutex::new(circuit_breaker),
            consecutive_failures: AtomicU64::new(u64::from(consecutive_failures)),
            force_wrap_up: std::sync::atomic::AtomicBool::new(consecutive_failures >= 3),
            wrap_up_violations: AtomicU64::new(0),
            current_step: Arc::clone(&current_step),
            max_steps,
            verified_admin_codes: Mutex::new(verified_admin_codes),
            goal_manifest: Mutex::new(goal_manifest),
            read_call_cache: Mutex::new(HashMap::new()),
            active_toolsets: Arc::clone(&active_toolsets),
            casual_fast_path,
        };
        let buffer = Mutex::new(StepStreamBuf::default());
        let close_open_thinking = |buf: &mut StepStreamBuf, step_idx: u64| {
            if buf.thinking_started && !buf.thinking_ended {
                let clean = history::outbound_text(&buf.thinking);
                let _ = self.append_event(
                    &session_id,
                    "thinking",
                    json!({
                        "run_id": run_id,
                        "step_index": step_idx,
                        "status": "done",
                        "text": clean,
                        "started_at_ms": buf.started_at_ms,
                        "thinking_ms": buf.thinking_ms,
                    }),
                );
                buf.thinking_ended = true;
            }
        };
        let flush = |buffer: &Mutex<StepStreamBuf>| {
            let mut buf = std::mem::take(&mut *lock(buffer));
            let step_idx = current_step.load(Ordering::Relaxed);
            if let Some(prev_step) = buf.step.filter(|&s| s != step_idx) {
                close_open_thinking(&mut buf, prev_step);
                return;
            }
            close_open_thinking(&mut buf, step_idx);
            let clean_text = without_think(&buf.text);
            if !clean_text.trim().is_empty() {
                let mut payload = json!({
                    "role": "assistant",
                    "run_id": run_id,
                    "text": clean_text,
                    "context": null,
                });
                let clean_thinking = history::outbound_text(buf.thinking.trim());
                if !clean_thinking.is_empty() {
                    payload["thinking_text"] = json!(clean_thinking);
                    payload["thinking_ms"] = json!(buf.thinking_ms);
                }
                let _ = self.append_event(&session_id, "message", payload);
            }
        };
        let tools = if casual_fast_path {
            Vec::new()
        } else {
            let mut all_tools = model_tools();
            all_tools.extend(self.service.application_extension_tools());
            all_tools
        };
        let refused = Mutex::new(None);
        let blocked = Mutex::new(None);
        let outcome = block_on(AgentLoop { max_steps }.run(
            &RecordingModel {
                model: &*model,
                writer: &dispatcher.writer,
                refused: &refused,
                blocked: &blocked,
                step_calls: &step_calls,
                current_step: &current_step,
                max_steps,
                force_wrap_up: &dispatcher.force_wrap_up,
                active_toolsets: &active_toolsets,
            },
            &dispatcher,
            &tools,
            checkpoint,
            &cancel,
            |event| {
                let step_idx = current_step.load(Ordering::Relaxed);
                match event {
                    ModelEvent::ThinkingStart { started_at_ms } => {
                        {
                            let mut buf = lock(&buffer);
                            if let Some(prev_step) = buf.step.filter(|&s| s != step_idx) {
                                close_open_thinking(&mut buf, prev_step);
                                buf.text.clear();
                            }
                            buf.step = Some(step_idx);
                            buf.thinking.clear();
                            buf.started_at_ms = Some(started_at_ms);
                            buf.thinking_ms = None;
                            buf.thinking_started = true;
                            buf.thinking_ended = false;
                        }
                        let _ = self.append_event(
                            &session_id,
                            "thinking",
                            json!({
                                "run_id": run_id,
                                "step_index": step_idx,
                                "status": "thinking",
                                "text": "",
                                "started_at_ms": started_at_ms,
                                "thinking_ms": null,
                            }),
                        );
                        self.notify(AgentHostNotification::LiveDelta {
                            session_id: session_id.clone(),
                            run_id: run_id.clone(),
                            step_index: step_idx,
                            kind: "thinking_start".into(),
                            delta: String::new(),
                            started_at_ms: Some(started_at_ms),
                            duration_ms: None,
                        });
                    }
                    ModelEvent::ThinkingDelta(delta) => {
                        let started_at_ms = {
                            let mut buf = lock(&buffer);
                            if let Some(prev_step) = buf.step.filter(|&s| s != step_idx) {
                                close_open_thinking(&mut buf, prev_step);
                                buf.text.clear();
                                buf.thinking.clear();
                            }
                            buf.step = Some(step_idx);
                            buf.thinking.push_str(&delta);
                            buf.started_at_ms
                        };
                        self.notify(AgentHostNotification::LiveDelta {
                            session_id: session_id.clone(),
                            run_id: run_id.clone(),
                            step_index: step_idx,
                            kind: "thinking".into(),
                            delta: history::outbound_text(&delta),
                            started_at_ms,
                            duration_ms: None,
                        });
                    }
                    ModelEvent::ThinkingEnd { duration_ms } => {
                        let (clean, started_at_ms) = {
                            let mut buf = lock(&buffer);
                            buf.step = Some(step_idx);
                            buf.thinking_ms = Some(duration_ms);
                            buf.thinking_ended = true;
                            (history::outbound_text(&buf.thinking), buf.started_at_ms)
                        };
                        let _ = self.append_event(
                            &session_id,
                            "thinking",
                            json!({
                                "run_id": run_id,
                                "step_index": step_idx,
                                "status": "done",
                                "text": clean,
                                "started_at_ms": started_at_ms,
                                "thinking_ms": duration_ms,
                            }),
                        );
                        self.notify(AgentHostNotification::LiveDelta {
                            session_id: session_id.clone(),
                            run_id: run_id.clone(),
                            step_index: step_idx,
                            kind: "thinking_end".into(),
                            delta: String::new(),
                            started_at_ms,
                            duration_ms: Some(duration_ms),
                        });
                    }
                    ModelEvent::Text(delta) => {
                        {
                            let mut buf = lock(&buffer);
                            if let Some(prev_step) = buf.step.filter(|&s| s != step_idx) {
                                close_open_thinking(&mut buf, prev_step);
                                buf.text.clear();
                                buf.thinking.clear();
                                buf.started_at_ms = None;
                                buf.thinking_ms = None;
                                buf.thinking_started = false;
                                buf.thinking_ended = false;
                            }
                            buf.step = Some(step_idx);
                            buf.text.push_str(&delta);
                        }
                        self.notify(AgentHostNotification::LiveDelta {
                            session_id: session_id.clone(),
                            run_id: run_id.clone(),
                            step_index: step_idx,
                            kind: "text".into(),
                            delta: history::outbound_text(&delta),
                            started_at_ms: None,
                            duration_ms: None,
                        });
                    }
                    ModelEvent::ToolCall(_) => flush(&buffer),
                }
            },
        ));
        let TurnDispatcher {
            executor,
            writer,
            proposed,
            local,
            failure,
            circuit_breaker,
            consecutive_failures,
            verified_admin_codes,
            goal_manifest,
            ..
        } = dispatcher;
        let executor = executor.into_inner().unwrap_or_else(|e| e.into_inner());
        let writer = writer.into_inner().unwrap_or_else(|e| e.into_inner());
        let proposed = proposed.into_inner().unwrap_or_else(|e| e.into_inner());
        let circuit_breaker = circuit_breaker
            .into_inner()
            .unwrap_or_else(|e| e.into_inner());
        let verified_admin_codes = verified_admin_codes
            .into_inner()
            .unwrap_or_else(|e| e.into_inner());
        let goal_manifest = goal_manifest
            .into_inner()
            .unwrap_or_else(|e| e.into_inner());
        let consecutive_failures =
            u8::try_from(consecutive_failures.load(Ordering::Relaxed)).unwrap_or(u8::MAX);
        let resume_user_text = local.user_text.clone();
        let resume_prev = local.resumed_from.clone();
        let persisted_toolsets = lock(&active_toolsets).clone();
        let rebuild = |checkpoint: Checkpoint, executor, writer| RunState {
            run_id: run_id.clone(),
            session_id: session_id.clone(),
            route: route.clone(),
            model,
            executor,
            writer,
            checkpoint,
            turn_start_msg_index,
            initial_user_msg: initial_user_msg.clone(),
            active_toolsets: persisted_toolsets,
            local,
            origin,
            circuit_breaker,
            consecutive_failures,
            verified_admin_codes,
            goal_manifest,
            casual_fast_path,
        };
        match outcome {
            Ok(result) => {
                {
                    let mut buf = lock(&buffer);
                    close_open_thinking(&mut buf, current_step.load(Ordering::Relaxed));
                }
                let state = rebuild(result.checkpoint, executor, writer);
                if let Err(error) = self.conclude(
                    state,
                    result.answer,
                    result.structured,
                    &result.reasoning_text,
                    result.thinking_ms,
                ) {
                    let _ = self.append_event(
                        &session_id,
                        "error",
                        json!({"run_id":run_id,"code":error.code(),"status":"failed","plan_id":null}),
                    );
                    lock(&self.live).remove(&run_id);
                    self.announce_activity(&[session_id.as_str()]);
                }
            }
            Err(error) => {
                flush(&buffer);
                let diagnostic_summary = error.sanitized_summary_zh();
                match (proposed, error.checkpoint) {
                    (Some(proposal), Some(checkpoint)) if !cancel.is_cancelled() => {
                        self.gate(rebuild(*checkpoint, executor, writer), proposal);
                    }
                    (proposed, checkpoint) => {
                        if let Some(Proposal { plan_id, .. }) = proposed {
                            let _ = self
                                .service
                                .with_store(|store| store.expire_agent_plan(&plan_id));
                        }
                        let cancelled = cancel.is_cancelled() || error.kind == ErrorKind::Cancelled;
                        let completed_steps_list = checkpoint
                            .as_ref()
                            .and_then(|cp| cp.messages.get(turn_start_msg_index..))
                            .map(extract_completed_tool_steps)
                            .unwrap_or_default();
                        let completed_tool_steps = checkpoint
                            .as_ref()
                            .map_or(0, |cp| cp.tool_results.len())
                            .max(completed_steps_list.len());
                        let refused = refused.into_inner().unwrap_or_else(|e| e.into_inner());
                        let failure = failure.into_inner().unwrap_or_else(|e| e.into_inner());
                        let code = if cancelled {
                            "cancelled".to_owned()
                        } else if let Some(code) = refused.or(failure) {
                            code.to_owned()
                        } else {
                            match error.kind {
                                ErrorKind::StepLimit => "step_limit".to_owned(),
                                ErrorKind::RepeatedCall => "repeated_tool_call".to_owned(),
                                ErrorKind::ToolNotOffered => "tool_not_offered".to_owned(),
                                kind => format!("model_{kind:?}").to_lowercase(),
                            }
                        };
                        let resume_cp = {
                            let (goal, mut merged_steps, auth_actions) =
                                if let Some(prev) = resume_prev.as_ref() {
                                    (
                                        prev.original_user_goal.clone(),
                                        prev.completed_tool_steps.clone(),
                                        prev.authorized_actions.clone(),
                                    )
                                } else {
                                    (
                                        history::outbound_text(&resume_user_text),
                                        Vec::new(),
                                        Vec::new(),
                                    )
                                };
                            for step in completed_steps_list {
                                let dup = merged_steps.iter().any(|existing| {
                                    existing.tool_id == step.tool_id
                                        && existing.input_summary == step.input_summary
                                        && existing.ok == step.ok
                                });
                                if !dup {
                                    merged_steps.push(step);
                                }
                            }
                            ResumeCheckpoint {
                                run_id: run_id.clone(),
                                original_user_goal: goal,
                                completed_tool_steps: merged_steps,
                                answer_delivered: false,
                                authorized_actions: auth_actions,
                                interrupt_reason: Some(code.clone()),
                                updated_at: now(),
                            }
                        };
                        if !resume_cp.original_user_goal.trim().is_empty() {
                            self.save_resume_checkpoint(&session_id, &resume_cp);
                        }
                        let state = rebuild(
                            checkpoint.map_or_else(|| Checkpoint::new(vec![]), |c| *c),
                            executor,
                            writer,
                        );
                        let status = if cancelled { "cancelled" } else { "failed" };
                        let closing = if cancelled {
                            "[上一轮已取消]".to_owned()
                        } else {
                            format!("[上一轮未完成] 错误 {code}")
                        };
                        let _ =
                            self.seal(state, status, Some(&closing), code != "outbound_blocked");
                        let mut payload =
                            json!({"run_id":run_id,"code":code,"status":status,"plan_id":null});
                        let message = if let Some(host_msg) = host_error_zh(&code) {
                            Some(host_msg.to_owned())
                        } else if !cancelled {
                            let diag = history::outbound_text(diagnostic_summary.trim());
                            if diag.is_empty() {
                                None
                            } else if completed_tool_steps > 0 {
                                Some(format!(
                                    "{diag}（本轮已完成 {completed_tool_steps} 项工具调用并已保留上下文，可点击「接着继续」或「换备用模型继续」）"
                                ))
                            } else {
                                Some(diag)
                            }
                        } else {
                            None
                        };
                        if let Some(message) = message {
                            payload["message_zh"] = json!(message);
                        }
                        if code == "outbound_blocked" {
                            let class = blocked.into_inner().unwrap_or_else(|e| e.into_inner());
                            payload["field_class"] = json!(class);
                        }
                        let _ = self.append_event(&session_id, "error", payload);
                    }
                }
            }
        }
    }

    fn seal(
        &self,
        state: RunState<S>,
        status: &str,
        closing_assistant: Option<&str>,
        persist_transcript: bool,
    ) -> Result<(), HostError> {
        let run_id = state.run_id.clone();
        let session_id = state.session_id.clone();
        if persist_transcript {
            self.persist_turn_messages(
                &run_id,
                &state.checkpoint,
                state.turn_start_msg_index,
                state.initial_user_msg.as_ref(),
                closing_assistant,
            );
        }
        lock(&self.live).remove(&run_id);
        let sealed = self
            .service
            .with_store(|store| state.writer.finish(store, &run_id, status, &now(), 0));
        self.announce_activity(&[session_id.as_str()]);
        sealed??;
        Ok(())
    }

    /// Freezes and saves a proposed plan, or says why the host refuses it
    /// (`PlanRefusal`): nothing is saved or shown for a refused plan.
    #[allow(clippy::too_many_arguments)]
    fn propose(
        &self,
        run_id: &str,
        session_id: &str,
        route: &str,
        arguments: &Value,
        local: &LocalContext,
        verified_admin_codes: &std::collections::HashSet<String>,
        circuit_breaker: &mut SearchCircuitBreaker,
        writer: &mut RunWriter,
    ) -> Result<ProposedPlan, PlanRefusal> {
        let steps = arguments
            .get("steps")
            .and_then(Value::as_array)
            .filter(|steps| (1..=MAX_PLAN_STEPS).contains(&steps.len()))
            .ok_or_else(|| PlanRefusal::new(HostError::Invalid, "steps_invalid"))?;
        // The forbidden tier (any tool outside the catalog) is refused in
        // every mode, before anything is planned.
        let forbidden = steps.iter().enumerate().find_map(|(index, step)| {
            step.get("tool_id")
                .and_then(Value::as_str)
                .filter(|id| Tier::of_tool(id) == Tier::Forbidden)
                .map(|id| (index, id))
        });
        if let Some((index, tool_id)) = forbidden {
            let mode = self.effective_mode(session_id);
            let verdict = permission::decide(mode, Tier::Forbidden, &[]);
            debug_assert_eq!(verdict, Verdict::Refuse);
            writer.append(
                "approval",
                &json!({"plan_id": null, "decision": "refused_forbidden", "mode": mode}),
            )?;
            let _ = self.append_event(
                session_id,
                "error",
                json!({"run_id": run_id, "code": "plan_refused_forbidden", "status": null,
                    "plan_id": null, "approval": approval_view("refused_forbidden", mode, &[])}),
            );
            return Err(PlanRefusal::new(HostError::Forbidden, "tool_forbidden").at(index, tool_id));
        }
        let plan_id = self.new_id("plan");
        let mut frozen = Vec::new();
        let mut cards = Vec::new();
        // Where each create_task's output directory came from (by step).
        let mut sources: Vec<Option<&'static str>> = Vec::new();
        // Working AOI staged by an earlier `working_aoi_set` step in this same plan.
        let mut staged_working_aoi: Option<Value> = None;
        let mut pending_searches: Vec<(String, String, bool)> = Vec::new();
        for (index, step) in steps.iter().enumerate() {
            let tool_id = step.get("tool_id").and_then(Value::as_str).ok_or_else(|| {
                let mut refusal = PlanRefusal::new(HostError::Invalid, "steps_invalid");
                refusal.step = Some(index);
                refusal
            })?;
            let refuse = |reason| PlanRefusal::new(HostError::Invalid, reason).at(index, tool_id);
            let tool = catalog::resolve(tool_id).map_err(|_| refuse("steps_invalid"))?;
            if tool.confirmation == "none" {
                return Err(refuse("tool_not_plannable"));
            }
            let mut input = step
                .get("input")
                .filter(|input| input.is_object())
                .cloned()
                .ok_or_else(|| refuse("input_invalid").detail("input must be an object"))?;
            // Identity keys are minted by the host, not chosen by the model.
            let key = format!("{plan_id}-{index}");
            if tool.confirmation == "plan_confirm" {
                input["event_id"] = json!(key);
            }
            if tool_id == "update_download_task" {
                input["occurred_at"] = json!(now());
            }
            if tool_id == "working_aoi_set" {
                if let Some(problem) = host::input_problem(tool_id, &input) {
                    return Err(refuse("input_invalid").detail(problem));
                }
                let preview = self
                    .service
                    .application_preview_working_aoi_set(&input)
                    .map_err(|_| {
                        refuse("aoi_unavailable")
                            .detail("working_aoi_set: requested area could not be resolved")
                    })?;
                if let Some(raw_code) = input.get("admin_code").and_then(Value::as_str) {
                    let trimmed_code = raw_code.trim();
                    let bare_code = trimmed_code.strip_prefix("datav:").unwrap_or(trimmed_code);
                    let verified_by_search = verified_admin_codes.contains(trimmed_code)
                        || verified_admin_codes.contains(bare_code);
                    if !verified_by_search {
                        if let Some(resolved_name) = aoi::name(&preview) {
                            let leaf = resolved_name
                                .rsplit(['/', '·', ' ', ',', '-', '，'])
                                .map(str::trim)
                                .find(|s| !s.is_empty())
                                .unwrap_or(resolved_name.trim());
                            let mut stem = leaf;
                            for suffix in [
                                "特别行政区",
                                "维吾尔自治区",
                                "壮族自治区",
                                "回族自治区",
                                "自治区",
                                "自治州",
                                "自治县",
                                "地区",
                                "省",
                                "市",
                                "区",
                                "县",
                                "盟",
                                "旗",
                            ] {
                                if let Some(stripped) = stem.strip_suffix(suffix) {
                                    if stripped.chars().count() >= 2 {
                                        stem = stripped;
                                    }
                                    break;
                                }
                            }
                            let user_lower = local.user_text.to_lowercase();
                            let stem_lower = stem.to_lowercase();
                            if stem.chars().count() >= 2 && !user_lower.contains(&stem_lower) {
                                return Err(refuse("admin_code_unverified").detail(format!(
                                    "admin_code {trimmed_code} resolved to {leaf}, not verified via admin_boundary_search"
                                )));
                            }
                        }
                    }
                }
                staged_working_aoi = Some(preview);
            }
            if tool_id == "search_start" {
                input["request_id"] = json!(key.clone());
                // No filters means none: the service's request type needs the key.
                if input.get("filters").is_none_or(Value::is_null) {
                    input["filters"] = json!({});
                }
                legacy_dataset(&mut input);
                normalize_optical_filters(&mut input);
                // The model names an area only by reference, never by geometry.
                let reference = input
                    .get("aoi")
                    .and_then(Value::as_object)
                    .is_some_and(|aoi| {
                        aoi.len() == 1 && aoi.get("ref").is_some_and(Value::is_string)
                    });
                if !reference {
                    return Err(refuse("input_invalid")
                        .detail("aoi must be an area reference: ref context_aoi or working_aoi"));
                }
                self.resolve_aoi_ref(
                    session_id,
                    &mut input,
                    local.aoi.as_ref(),
                    staged_working_aoi.as_ref(),
                )
                .map_err(|detail| refuse("aoi_unavailable").detail(detail))?;
                if let Some(problem) = legacy_filter_problem(&input) {
                    return Err(refuse("input_invalid").detail(problem));
                }
                if let Some(problem) = optical_filter_problem(&input) {
                    return Err(refuse("input_invalid").detail(problem));
                }
                if let Some(problem) = host::input_problem(tool_id, &input) {
                    return Err(refuse("input_invalid").detail(problem));
                }
                let bbox_val = aoi::bbox(aoi::geometry(&input["aoi"]));
                let bbox_arr = bbox_val
                    .as_array()
                    .filter(|arr| arr.len() == 4)
                    .and_then(|arr| {
                        Some([
                            arr[0].as_f64()?,
                            arr[1].as_f64()?,
                            arr[2].as_f64()?,
                            arr[3].as_f64()?,
                        ])
                    });
                let bucket_key = guardrails::semantic_search_bucket_key(&input, bbox_arr);
                if let Some(tripped) = circuit_breaker.check_before_search(&bucket_key) {
                    let code = tripped
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or("ZERO_RESULT_RETRY_EXHAUSTED");
                    let guidance = tripped
                        .get("guidance")
                        .and_then(Value::as_str)
                        .unwrap_or("circuit_breaker_tripped");
                    let _ = self.trace(
                        writer,
                        "search_circuit_breaker",
                        &json!({"bucket": bucket_key, "code": code, "step": index}),
                    );
                    return Err(
                        refuse("circuit_breaker_tripped").detail(format!("{code} - {guidance}"))
                    );
                }
                let resource_id = input
                    .get("resource_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let deterministic_s1_hh = match guardrails::evaluate_s1_land_polarization(
                    resource_id,
                    bbox_arr,
                    &input,
                    &local.user_text,
                ) {
                    S1PolarizationDecision::Allow => false,
                    S1PolarizationDecision::AllowOnceMarkDeterministic => true,
                    S1PolarizationDecision::RefuseUnpromptedLandHh => {
                        return Err(refuse("s1_land_hh_unavailable").detail(
                            "S1_LAND_HH_UNAVAILABLE - continental non-polar land uses VV or VV+VH",
                        ));
                    }
                };
                pending_searches.push((key.clone(), bucket_key, deterministic_s1_hh));
            }
            let source = if tool_id == "create_task" {
                let has_options = input.get("options").is_some_and(Value::is_object);
                self.resolve_output_dir(session_id, &mut input, local.message_dir.as_deref())
                    .map_err(|_| {
                        if has_options {
                            refuse("output_dir_invalid")
                        } else {
                            refuse("input_invalid").detail("options must be an object")
                        }
                    })?
            } else {
                None
            };
            if tool_id != "search_start" && tool_id != "working_aoi_set" {
                if let Some(problem) = host::input_problem(tool_id, &input) {
                    return Err(refuse("input_invalid").detail(problem));
                }
            }
            let default_label = if tool_id == "working_aoi_set" {
                if input.get("clear") == Some(&json!(true)) {
                    Some("清空当前工作区".to_owned())
                } else {
                    staged_working_aoi
                        .as_ref()
                        .and_then(aoi::name)
                        .map(|name| format!("设置工作区：{name}"))
                }
            } else {
                None
            };
            let label = step
                .get("label_zh")
                .and_then(Value::as_str)
                .map(|label| label.chars().take(80).collect::<String>())
                .filter(|label| !label.trim().is_empty())
                .or(default_label)
                .unwrap_or_else(|| tool.description.clone());
            let mut card = json!({"tool_id": tool_id, "label_zh": label, "tier": tier(&tool.confirmation), "status": "pending"});
            // A search runs without a click in every mode; the card discloses
            // which source it asks and for what (never the geometry).
            if tool_id == "search_start" {
                let name = input
                    .get("resource_id")
                    .and_then(Value::as_str)
                    .and_then(|id| self.source_display_name(id));
                card["disclosure"] = json!({
                    "resource_id": input.get("resource_id"),
                    "aoi_name": input.get("aoi_name"),
                    "time_range": input.get("time_range").cloned().unwrap_or(Value::Null),
                    "display_name_zh": name,
                });
            }
            let mut frozen_step = json!({"tool_id": tool_id, "input": input, "label_zh": label});
            if tool.confirmation == "plan_confirm" {
                frozen_step["event_id"] = json!(key);
            }
            cards.push(card);
            frozen.push(frozen_step);
            sources.push(source);
        }
        let confirm_step = frozen.iter().position(|step| {
            step.get("tool_id")
                .and_then(Value::as_str)
                .and_then(|id| catalog::resolve(id).ok())
                .is_some_and(|tool| tool.confirmation == "plan_confirm")
        });
        let mut intent = json!({ "steps": &frozen });
        if let Some(index) = confirm_step {
            let tool_id = frozen[index]["tool_id"].as_str().unwrap_or("").to_owned();
            let snapshot = match self
                .service
                .with_store(|store| card_snapshot(store, &frozen[index], sources[index]))
            {
                Ok(Ok((mut snapshot, provider, query))) => {
                    let resource = source_resource_id(&provider, &query);
                    // The source's name for the user, from the same source
                    // list the search plan card discloses (outside the Store
                    // lock: the service takes it). Part of the frozen card.
                    snapshot["source_resource_id"] = json!(resource);
                    snapshot["source_display_name_zh"] = json!(resource
                        .as_deref()
                        .and_then(|id| self.source_display_name(id)));
                    snapshot
                }
                Ok(Err(error)) | Err(error) => {
                    // The card names a frozen batch selection or a task;
                    // an input the Store cannot match is that reason.
                    let invalid = matches!(error, HostError::Invalid);
                    let refusal = match (invalid, tool_id.as_str()) {
                        (true, "create_task") => PlanRefusal::new(error, "selection_invalid"),
                        (true, _) => PlanRefusal::new(error, "task_not_found"),
                        (false, _) => PlanRefusal::from(error),
                    };
                    return Err(refusal.at(index, &tool_id));
                }
            };
            intent["card_snapshot"] = snapshot;
        }
        let revision = self
            .service
            .with_store(|store| plan::plan_revision(store, &intent))?
            .map_err(HostError::from)?;
        let facts = self
            .plan_facts(session_id, run_id, &frozen, &sources)
            .map_err(|error| match error {
                HostError::Invalid => PlanRefusal::new(error, "selection_invalid"),
                error => PlanRefusal::from(error),
            })?;
        let estimated_bytes = intent
            .get("card_snapshot")
            .and_then(|s| s.get("estimated_bytes"))
            .and_then(Value::as_u64);
        let created = SystemTime::now();
        let row = plan::build_plan(
            AgentPlanRow {
                plan_id: plan_id.clone(),
                session_id: session_id.to_owned(),
                run_id: run_id.to_owned(),
                created_at: rfc3339(created),
                model_route: route.to_owned(),
                plan_json: String::new(),
                plan_hash: String::new(),
                max_tier: 2,
                status: "draft".into(),
                expires_at: rfc3339(created + PLAN_TTL),
                store_revision_seen: revision,
            },
            &intent,
        )
        .map_err(HostError::from)?;
        self.service.with_store(|store| {
            if self.session_gone(session_id) {
                return Err(HostError::NotFound);
            }
            Ok(store.save_agent_plan(&row)?)
        })??;
        for (req_id, bucket_key, det_s1_hh) in pending_searches {
            circuit_breaker.register_batch(&req_id, bucket_key, det_s1_hh);
        }
        writer.append(
            "plan",
            &json!({"plan_id": plan_id, "plan_hash": row.plan_hash, "max_tier": row.max_tier, "steps": cards.len()}),
        )?;
        let mut shown = vec![(
            "plan_card",
            json!({"run_id": run_id, "plan_id": plan_id, "plan_hash": row.plan_hash, "max_tier": row.max_tier, "steps": cards}),
        )];
        if let Some(snapshot) = intent.get("card_snapshot") {
            shown.push((
                "confirm_card",
                json!({"run_id": run_id, "plan_id": plan_id, "plan_hash": row.plan_hash, "snapshot": snapshot}),
            ));
        }
        let tier = match row.max_tier {
            1 => Tier::Disclose,
            _ => Tier::Confirm,
        };
        Ok((plan_id, shown, facts, tier, estimated_bytes))
    }

    /// The source's display name from the service's source list
    /// (SourceDescriptor.display_name_zh); `None` when the list cannot be
    /// read or does not name the source.
    fn source_display_name(&self, resource_id: &str) -> Option<String> {
        let sources = self.service.application_list_sources().ok()?;
        sources
            .as_array()?
            .iter()
            .find(|source| source.get("resource_id").and_then(Value::as_str) == Some(resource_id))?
            .get("display_name_zh")?
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
    }

    /// Owner 2026-10-26, review ruling: a directory in `options.output_root`
    /// came from the model (`model_suggested`; the host cannot tell it from
    /// the user's message, so the card confirms it in every mode). A
    /// `{"ref": "message_dir"}` names the directory the host took from the
    /// user's message (W3g, `from_message`); the host resolves it, like
    /// `context_aoi`, and the gate treats it as known. Without either the
    /// default download directory is written into the frozen input
    /// (`default`). With none of these, or a `message_dir` reference while
    /// the message named no usable directory (never replaced by the
    /// default: the user meant a directory of their own), the directory
    /// stays unknown (`None`) and the card must choose one in every mode.
    fn resolve_output_dir(
        &self,
        session_id: &str,
        input: &mut Value,
        message_dir: Option<&str>,
    ) -> Result<Option<&'static str>, HostError> {
        let options = input
            .get_mut("options")
            .and_then(Value::as_object_mut)
            .ok_or(HostError::Invalid)?;
        let summary = self.load_session(session_id).ok();
        let session_remembered = summary
            .as_ref()
            .and_then(|s| s.remembered_output_dir.as_deref())
            .filter(|dir| permission::valid_output_dir(dir))
            .map(str::to_owned);
        match options.get("output_root") {
            Some(Value::String(dir)) if !dir.is_empty() => {
                return if permission::valid_output_dir(dir) {
                    Ok(Some("model_suggested"))
                } else {
                    Err(HostError::Invalid)
                };
            }
            Some(Value::Object(reference)) => {
                let named = reference.len() == 1
                    && reference.get("ref").and_then(Value::as_str) == Some(message_dir::REF);
                if !named {
                    return Err(HostError::Invalid);
                }
                let resolved = message_dir
                    .filter(|dir| permission::valid_output_dir(dir))
                    .map(str::to_owned)
                    .or(session_remembered);
                return Ok(match resolved {
                    // A directory the message named (or remembered earlier in
                    // this same session) is used only where it is a usable
                    // output directory on this host; never substitute
                    // workspace folder or global default for `{"ref": "message_dir"}`.
                    Some(dir) => {
                        options.insert("output_root".into(), Value::String(dir));
                        Some("from_message")
                    }
                    None => {
                        options.remove("output_root");
                        None
                    }
                });
            }
            None | Some(Value::Null) => {}
            Some(Value::String(_)) => {}
            Some(_) => return Err(HostError::Invalid),
        }
        options.remove("output_root");
        if let Some(dir) = session_remembered {
            options.insert("output_root".into(), Value::String(dir));
            return Ok(Some("from_message"));
        }
        let workspace_folder = summary
            .as_ref()
            .and_then(|s| s.workspace_id.as_deref())
            .and_then(|wid| self.workspace(wid).ok())
            .and_then(|ws| ws.folder)
            .filter(|dir| permission::valid_output_dir(dir));
        if let Some(dir) = workspace_folder {
            options.insert("output_root".into(), Value::String(dir));
            return Ok(Some("default"));
        }
        let default = self
            .service
            .application_default_output_root()
            .ok()
            .flatten()
            .filter(|dir| permission::valid_output_dir(dir));
        Ok(default.map(|dir| {
            options.insert("output_root".into(), Value::String(dir));
            "default"
        }))
    }

    /// Host facts of each frozen step for the permission gate (A1 §3.1
    /// matrix): Store facts, the run record's own batches, the service's
    /// account readiness and the Agent-created tasks.
    fn plan_facts(
        &self,
        session_id: &str,
        run_id: &str,
        frozen: &[Value],
        sources: &[Option<&'static str>],
    ) -> Result<Vec<Step>, HostError> {
        fn tool_of(step: &Value) -> &str {
            step.get("tool_id").and_then(Value::as_str).unwrap_or("")
        }
        let creates = frozen.iter().any(|step| tool_of(step) == "create_task");
        let controls = frozen
            .iter()
            .any(|step| matches!(tool_of(step), "task_action" | "update_download_task"));
        let run_batches = if creates {
            self.run_batches(session_id, run_id)
        } else {
            Default::default()
        };
        let agent_tasks: std::collections::HashSet<String> = if controls {
            self.agent_created_tasks()
                .into_iter()
                .map(|(_, task_id)| task_id)
                .collect()
        } else {
            Default::default()
        };
        let mut facts = Vec::new();
        for (step, source) in frozen.iter().zip(sources) {
            let input = step.get("input").unwrap_or(&Value::Null);
            let field = |name: &str| input.get(name).and_then(Value::as_str).unwrap_or("");
            match tool_of(step) {
                "working_aoi_set" => facts.push(Step::WorkspaceAoi),
                "search_start" => facts.push(Step::Search),
                "create_task" => {
                    let (size_known, reopens_existing_task) = self
                        .service
                        .with_store(|store| create_task_facts(store, input))??;
                    let items: Vec<String> = selected_items(input)
                        .into_iter()
                        .map(str::to_owned)
                        .collect();
                    let accounts_ready = self
                        .service
                        .application_selection_accounts_ready(field("batch_id"), &items)
                        .unwrap_or(false);
                    // The default download directory and one the host took
                    // from the user's message (PM ruling, W3g) are known.
                    let output_dir = match source {
                        Some("default" | "from_message") => OutputDir::Known,
                        Some(_) => OutputDir::ModelSuggested,
                        None => OutputDir::Unknown,
                    };
                    facts.push(Step::CreateTask {
                        output_dir,
                        size_known,
                        reopens_existing_task,
                        accounts_ready,
                        selection_from_run: run_batches.contains(field("batch_id")),
                    });
                }
                tool @ ("task_action" | "update_download_task") => facts.push(Step::TaskControl {
                    archive: tool == "task_action" && field("action") == "archive",
                    agent_task: agent_tasks.contains(field("task_id")),
                    ledger_edit: tool == "update_download_task"
                        && !input
                            .pointer("/command/kind")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| permission::STEERING_COMMANDS.contains(&kind)),
                }),
                // Any other tier 1/2 tool: no facts, so the card in every mode.
                _ => facts.push(Step::Unmapped),
            }
        }
        Ok(facts)
    }

    /// Batches frozen by searches this run started (A1 §3.1 matrix,
    /// `selection_not_from_run`). The run record is the source: each
    /// `search_batch` event the host wrote when one of the run's searches
    /// finished. A search of the run that has no such event yet (e.g. it
    /// finished without a progress read) is asked of the service by its
    /// request id; that lookup is in memory and lost on restart.
    fn run_batches(&self, session_id: &str, run_id: &str) -> std::collections::HashSet<String> {
        let collect_for_run = |rid: &str| -> (std::collections::HashSet<String>, bool) {
            let Ok(events) = run::read_trace(&self.root.join("agent").join("runs").join(rid))
            else {
                return (Default::default(), false);
            };
            let mut batches = std::collections::HashSet::new();
            let mut recorded = std::collections::HashSet::new();
            for event in &events {
                if event.kind == "search_batch" {
                    if let (Some(request), Some(batch)) = (
                        event.payload.get("request_id").and_then(Value::as_str),
                        event.payload.get("batch_id").and_then(Value::as_str),
                    ) {
                        recorded.insert(request.to_owned());
                        batches.insert(batch.to_owned());
                    }
                }
            }
            let searches = run_searches(&events);
            let started_search = !searches.is_empty();
            for request in searches {
                if recorded.contains(&request) {
                    continue;
                }
                if let Ok(Some(batch)) = self.service.application_search_batch(&request) {
                    batches.insert(batch);
                }
            }
            (batches, started_search)
        };
        let (own_batches, started_own_search) = collect_for_run(run_id);
        if started_own_search || !own_batches.is_empty() {
            return own_batches;
        }
        // Multi-turn workflow: when this turn did not run `search_start` itself,
        // accept batches frozen by earlier turns in the same session.
        let Ok(summary) = self.load_session(session_id) else {
            return own_batches;
        };
        let mut session_batches = std::collections::HashSet::new();
        for rid in &summary.run_ids {
            let (batches, _) = collect_for_run(rid);
            session_batches.extend(batches);
        }
        session_batches
    }

    /// After a successful `search_progress` read: when it reports a finished
    /// search this run started, the batch it froze goes into the run record
    /// as a `search_batch` event (the service is asked once, by request id).
    fn record_search_batch(
        &self,
        session_id: &str,
        run_id: &str,
        summary: &Value,
        writer: &mut RunWriter,
    ) {
        let finished = summary.get("stage").and_then(Value::as_str) == Some("done");
        let Some(request) = summary.get("request_id").and_then(Value::as_str) else {
            return;
        };
        if !finished {
            return;
        }
        let Ok(events) = run::read_trace(&self.root.join("agent").join("runs").join(run_id)) else {
            return;
        };
        let already = events.iter().any(|event| {
            event.kind == "search_batch"
                && event.payload.get("request_id").and_then(Value::as_str) == Some(request)
        });
        if already || !run_searches(&events).iter().any(|own| own == request) {
            return;
        }
        if let Ok(Some(batch)) = self.service.application_search_batch(request) {
            let _ = self.trace(
                writer,
                "search_batch",
                &json!({"request_id": request, "batch_id": batch}),
            );
            let header = self
                .service
                .with_store(|store| store.load_batch_header(&batch))
                .ok()
                .and_then(|res| res.ok())
                .flatten();
            let resource_id = header
                .as_ref()
                .and_then(|h| source_resource_id(&h.provider_id, &h.query));
            let loaded_count = header
                .as_ref()
                .and_then(|h| h.search_stats.as_ref().map(|s| s.frozen_candidate_count))
                .or_else(|| summary.get("items_kept").and_then(Value::as_u64));
            let total_count = header
                .as_ref()
                .and_then(|h| h.search_stats.as_ref().and_then(|s| s.matched_count))
                .or_else(|| summary.get("matched_count").and_then(Value::as_u64))
                .or(loaded_count);
            let _ = self.update_workspace_state(session_id, |ws| {
                ws.session_id = session_id.to_owned();
                ws.active_batch_id = Some(batch.clone());
                if resource_id.is_some() {
                    ws.active_resource_id = resource_id.clone();
                }
                ws.active_batch_revision = Some(0);
            });
            let _ = self.append_event(
                session_id,
                "workspace_sync",
                json!({
                    "run_id": run_id,
                    "action": "search_updated",
                    "batch_id": batch,
                    "resource_id": resource_id,
                    "loaded_count": loaded_count,
                    "total_count": total_count,
                }),
            );
            self.notify(AgentHostNotification::SearchUpdated {
                session_id: session_id.to_owned(),
                run_id: run_id.to_owned(),
                request_id: request.to_owned(),
                batch_id: Some(batch),
            });
        }
    }

    /// Replaces a search step's `{"ref": "context_aoi" | "working_aoi"}` with
    /// the local geometry, so the frozen plan holds the real area while the
    /// model only ever saw its summary. When an earlier `working_aoi_set` step
    /// in the same plan staged a new working AOI, `staged_working_aoi` takes
    /// precedence over the pre-plan service state.
    fn resolve_aoi_ref(
        &self,
        session_id: &str,
        input: &mut Value,
        context_aoi: Option<&Value>,
        staged_working_aoi: Option<&Value>,
    ) -> Result<(), &'static str> {
        let reference = match input.get("aoi").and_then(Value::as_object) {
            Some(aoi) if aoi.len() == 1 => aoi.get("ref").and_then(Value::as_str),
            _ => None,
        };
        let source = match reference {
            None => return Err("aoi must be an area reference"),
            Some(aoi::CONTEXT_REF) => context_aoi
                .cloned()
                .ok_or("context_aoi: this message has no attached area")?,
            Some(aoi::WORKING_REF) => match staged_working_aoi {
                Some(staged) => staged.clone(),
                None => match self.session_working_aoi_if_initialized(session_id) {
                    Some(session_aoi) => session_aoi,
                    None => self
                        .service
                        .application_working_aoi()
                        .map_err(|_| "working_aoi: the working area could not be read")?,
                },
            },
            Some(_) => return Err("aoi.ref must be context_aoi or working_aoi"),
        };
        let geometry = aoi::geometry(&source)
            .ok_or("the referenced area has no geometry (no working area is set)")?
            .clone();
        input["aoi"] = Value::Object(geometry);
        let named = input
            .get("aoi_name")
            .and_then(Value::as_str)
            .is_some_and(|name| !name.trim().is_empty());
        if !named {
            input["aoi_name"] = json!(aoi::name(&source).unwrap_or_else(|| "工作区".into()));
        }
        Ok(())
    }

    /// The tier 1/2 confirmation point (A1 §3.1): the session's mode at this
    /// moment decides whether the plan waits for the desktop card or is
    /// approved by the mode. Tier 0 never reaches here; the forbidden tier is
    /// refused when the plan is proposed.
    fn gate(self: &Arc<Self>, mut state: RunState<S>, proposal: Proposal) {
        let session_id = state.session_id.clone();
        let run_id = state.run_id.clone();
        let mode = self.effective_mode(&session_id);
        let verdict = permission::decide_with_cap(
            mode,
            proposal.tier,
            &proposal.steps,
            proposal.estimated_bytes,
        );
        let reasons = match &verdict {
            Verdict::Card { reasons } => reasons.clone(),
            _ => vec![],
        };
        let auto = verdict == Verdict::Auto && state.origin == "in_app";
        // An approval by mode the run record cannot show is not given: the
        // plan waits for the card instead.
        let recorded = self
            .trace(
                &mut state.writer,
                "permission",
                &json!({"plan_id": proposal.plan_id, "mode": mode,
                    "verdict": if auto { "auto_by_mode" } else { "card" }, "card_reasons": reasons}),
            )
            .is_ok();
        let auto = auto && recorded;
        let parked = Parked {
            state,
            plan_id: proposal.plan_id,
            call_id: proposal.call_id,
        };
        if auto {
            if lock(&self.live).contains_key(&run_id) {
                let _ = self.decide_parked(
                    parked,
                    Decision {
                        snapshot: Value::Null,
                        approved: true,
                        output_dir: None,
                    },
                    mode,
                    Approver::Mode,
                    Some(proposal.cards),
                );
            }
            return;
        }
        let shown = match lock(&self.live).get_mut(&run_id) {
            Some(entry) => {
                entry.parked = Some(parked);
                true
            }
            None => false,
        };
        if shown {
            let approval = approval_view("manual", mode, &reasons);
            self.show_cards(&session_id, proposal.cards, &approval, false);
        }
    }

    fn show_cards(&self, session_id: &str, cards: Cards, approval: &Value, auto_executed: bool) {
        for (kind, mut payload) in cards {
            payload["approval"] = approval.clone();
            payload["auto_executed"] = json!(auto_executed);
            let _ = self.append_event(session_id, kind, payload);
        }
    }

    /// The mode a decision in this session uses now (read at each decision,
    /// so a change applies to the next plan of a run in progress).
    fn effective_mode(&self, session_id: &str) -> PermissionMode {
        self.load_session(session_id)
            .ok()
            .and_then(|summary| summary.permission_mode)
            .unwrap_or_default()
    }

    fn take_parked(&self, plan_id: &str) -> Result<Parked<S>, HostError> {
        let mut live = lock(&self.live);
        live.values_mut()
            .find(|run| run.parked.as_ref().is_some_and(|p| p.plan_id == plan_id))
            .and_then(|run| run.parked.take())
            .ok_or(HostError::NotFound)
    }
    /// Puts a run taken for a decision back to waiting for its card, unless
    /// it was cancelled meanwhile, in which case it is sealed and the
    /// cancel `run_cancel` could not report (the run was not parked) is.
    fn restore_parked(&self, parked: Parked<S>) -> bool {
        let mut live = lock(&self.live);
        match live.get_mut(&parked.state.run_id) {
            Some(entry) if entry.parked.is_none() && !entry.cancel.is_cancelled() => {
                entry.parked = Some(parked);
                true
            }
            _ => {
                drop(live);
                let Parked { state, plan_id, .. } = parked;
                let (run_id, session_id) = (state.run_id.clone(), state.session_id.clone());
                let _ = self
                    .service
                    .with_store(|store| store.expire_agent_plan(&plan_id));
                let _ = self.seal(state, "cancelled", Some("[上一轮已取消]"), true);
                let _ = self.append_event(
                    &session_id,
                    "error",
                    json!({"run_id": run_id, "code": "cancelled", "status": "cancelled", "plan_id": plan_id}),
                );
                false
            }
        }
    }

    /// Expires, seals and emits a neutral `progress_card` (`status: "expired"`,
    /// never a red `error` card) when a parked confirm card is superseded by a
    /// newer user message or working-AOI change.
    fn finalize_superseded_parked(&self, parked: Parked<S>, code: &'static str) {
        let Parked { state, plan_id, .. } = parked;
        let (run_id, session_id) = (state.run_id.clone(), state.session_id.clone());
        let _ = self
            .service
            .with_store(|store| store.expire_agent_plan(&plan_id));
        let _ = self.seal(
            state,
            "cancelled",
            Some("[上一轮未完成] 计划已被新消息取代"),
            true,
        );
        let _ = self.append_event(
            &session_id,
            "progress_card",
            json!({
                "run_id": run_id,
                "plan_id": plan_id,
                "step_index": null,
                "tool_id": null,
                "status": "expired",
                "code": code,
            }),
        );
    }

    fn take_parked_for_session_locked(
        live: &mut HashMap<String, Live<S>>,
        session_id: &str,
    ) -> Vec<Parked<S>> {
        let parked_ids: Vec<String> = live
            .iter()
            .filter(|(_, run)| run.session_id == session_id && run.parked.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        let mut popped = Vec::with_capacity(parked_ids.len());
        for id in parked_ids {
            if let Some(mut entry) = live.remove(&id) {
                entry.cancel.cancel();
                if let Some(parked) = entry.parked.take() {
                    popped.push(parked);
                }
            }
        }
        popped
    }

    fn supersede_parked_in_session(&self, session_id: &str, code: &'static str) {
        let superseded = Self::take_parked_for_session_locked(&mut lock(&self.live), session_id);
        for parked in superseded {
            self.finalize_superseded_parked(parked, code);
        }
    }
    /// A run no longer registered (cancelled, sealed or its session deleted)
    /// gets an already cancelled token, so nothing starts for it.
    fn cancel_token(&self, run_id: &str) -> CancellationToken {
        lock(&self.live)
            .get(run_id)
            .map(|run| run.cancel.clone())
            .unwrap_or_else(|| {
                let token = CancellationToken::new();
                token.cancel();
                token
            })
    }

    /// Desktop card approval. Tier 2 requires `snapshot` to equal the confirm
    /// card exactly; tier 1 (disclosure) ignores it. `output_dir` is a
    /// directory the user chose on a create_task card: the plan is rewritten
    /// to write there before it is approved (Owner 2026-10-26). A plan whose
    /// create_task has no directory cannot be approved without one; it keeps
    /// waiting for the card.
    pub fn plan_approve(
        self: &Arc<Self>,
        plan_id: &str,
        snapshot: Value,
        output_dir: Option<String>,
    ) -> Result<(), HostError> {
        self.decide(
            plan_id,
            Decision {
                snapshot,
                approved: true,
                output_dir,
            },
        )
    }
    pub fn plan_reject(self: &Arc<Self>, plan_id: &str) -> Result<(), HostError> {
        self.decide(
            plan_id,
            Decision {
                snapshot: json!({}),
                approved: false,
                output_dir: None,
            },
        )
    }

    fn decide(self: &Arc<Self>, plan_id: &str, decision: Decision) -> Result<(), HostError> {
        if !valid_id(plan_id)
            || decision
                .output_dir
                .as_deref()
                .is_some_and(|dir| !permission::valid_output_dir(dir))
        {
            return Err(HostError::Invalid);
        }
        let parked = self.take_parked(plan_id)?;
        let mode = self.effective_mode(&parked.state.session_id);
        self.decide_parked(parked, decision, mode, Approver::Card, None)
    }

    /// Records one decision. An approval by mode stores the card the user is
    /// shown as its snapshot (`decided_via = auto_by_mode`), then shows the
    /// cards marked as executed automatically; if it cannot be recorded the
    /// run waits for the desktop card instead.
    fn decide_parked(
        self: &Arc<Self>,
        mut parked: Parked<S>,
        decision: Decision,
        mode: PermissionMode,
        approver: Approver,
        auto_cards: Option<Cards>,
    ) -> Result<(), HostError> {
        let plan_id = parked.plan_id.clone();
        let plan_id = plan_id.as_str();
        let approved = decision.approved;
        let via = match approver {
            Approver::Card => "desktop_card",
            Approver::Mode => "auto_by_mode",
        };
        let fall_back = |parked: Parked<S>, auto_cards: Option<Cards>| {
            let session_id = parked.state.session_id.clone();
            if self.restore_parked(parked) {
                if let Some(cards) = auto_cards {
                    let approval = approval_view("manual", mode, &[]);
                    self.show_cards(&session_id, cards, &approval, false);
                }
            }
        };
        let at = now();
        let outcome = self
            .service
            .with_store(|store| -> Result<_, HostError> {
                let row = store.agent_plan(plan_id)?.ok_or(HostError::NotFound)?;
                let shown_hash = row.plan_hash.clone();
                let intent: Value = serde_json::from_str(&row.plan_json)?;
                let snapshot = match approver {
                    Approver::Card => decision.snapshot.clone(),
                    Approver::Mode => intent
                        .get("card_snapshot")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                };
                let rewritten = match (&decision.output_dir, approved) {
                    (Some(dir), true) => Some(
                        plan::rewrite_output_dir(&intent, dir).map_err(|_| HostError::Invalid)?,
                    ),
                    (None, true) if plan::missing_output_dir(&intent) => {
                        return Err(HostError::Invalid)
                    }
                    _ => None,
                };
                let approval = AgentApprovalRow {
                    approval_id: format!("approval-{plan_id}"),
                    plan_id: plan_id.to_owned(),
                    plan_hash_seen: row.plan_hash.clone(),
                    snapshot_json: serde_json::to_string(&snapshot)?,
                    decision: if approved { "approved" } else { "rejected" }.into(),
                    decided_at: at.clone(),
                    decided_via: via.into(),
                    mode: Some(mode.as_str().into()),
                    rewritten_from_plan_hash: None,
                };
                let current = plan::plan_revision(store, &intent)?;
                let decided = match &rewritten {
                    Some(rewritten) => {
                        plan::approve_rewritten(store, plan_id, &approval, rewritten, current, &at)
                            .map(Some)
                    }
                    None => plan::approve(store, plan_id, &approval, current, &at).map(|()| None),
                };
                // After a rewrite the steps run from the rewritten plan, built
                // from what was committed (no second read that could fail
                // after the approval is already in the Store).
                let row = match (&decided, &rewritten) {
                    (Ok(Some(hash)), Some(rewritten)) => AgentPlanRow {
                        plan_json: plan::canonical_json(rewritten)?,
                        plan_hash: hash.clone(),
                        status: "approved".into(),
                        ..row
                    },
                    _ => row,
                };
                let rewrite = rewritten.map(|_| {
                    let card = intent.get("card_snapshot");
                    let text =
                        |key: &str| card.and_then(|card| card.get(key)).and_then(Value::as_str);
                    RewriteRecord {
                        from_plan_hash: shown_hash,
                        from_dir: text("output_dir").map(str::to_owned),
                        from_source: text("output_dir_source").map(str::to_owned),
                    }
                });
                Ok((decided, row, rewrite))
            })
            .and_then(|outcome| outcome);
        // These errors all happen before the decision is written: the run
        // goes back to waiting for the same card.
        let (decided, row, rewrite) = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                fall_back(parked, auto_cards);
                return Err(error);
            }
        };
        // A Store failure while writing the decision rolled back: retryable,
        // not an expiry. A conflict or a plan no longer awaiting a decision
        // is the Store saying the plan expired.
        let decided = match decided {
            Err(PlanError::Store(error))
                if !matches!(error, StoreError::RevisionConflict | StoreError::NotFound) =>
            {
                fall_back(parked, auto_cards);
                return Err(HostError::Store(error));
            }
            decided => decided,
        };
        let status = match (&decided, approved) {
            (Ok(_), true) => "approved",
            (Ok(_), false) => "rejected",
            (Err(_), _) => "expired",
        };
        let rewritten_to = match &decided {
            Ok(Some(hash)) => Some(hash.clone()),
            _ => None,
        };
        let run_id = parked.state.run_id.clone();
        let recorded = self
            .trace(
                &mut parked.state.writer,
                "approval",
                &json!({"plan_id": plan_id, "decision": status, "via": via, "mode": mode,
                    "rewritten_from_plan_hash": rewritten_to.as_ref().and(rewrite.as_ref()).map(|r| &r.from_plan_hash)}),
            )
            .and_then(|()| match (&rewrite, &rewritten_to, &decision.output_dir) {
                // Run records never hold local paths: the directories are
                // named by their SHA-256.
                (Some(from), Some(to_hash), Some(dir)) => self.trace(
                    &mut parked.state.writer,
                    "plan_rewritten",
                    &json!({"plan_id": plan_id, "field": "output_dir",
                        "from": {"source": from.from_source,
                            "sha256": from.from_dir.as_deref().map(|dir| path_digest(&run_id, dir))},
                        "to": {"source": "chosen_on_card", "sha256": path_digest(&run_id, dir)},
                        "from_plan_hash": from.from_plan_hash, "to_plan_hash": to_hash}),
                ),
                _ => Ok(()),
            });
        if let Err(error) = recorded {
            // The decision is in the Store but the run record cannot follow
            // it: the run stops, and the session says so.
            let (run_id, session_id) =
                (parked.state.run_id.clone(), parked.state.session_id.clone());
            let _ = self.seal(parked.state, "failed", Some("[上一轮未完成]"), true);
            let _ = self.append_event(
                &session_id,
                "error",
                json!({"run_id": run_id, "code": "record_failed", "status": "failed", "plan_id": plan_id}),
            );
            return Err(error.into());
        }
        if let Some(cards) = auto_cards {
            let approval = approval_view("auto_by_mode", mode, &[]);
            self.show_cards(
                &parked.state.session_id,
                cards,
                &approval,
                status == "approved",
            );
        }
        let host = Arc::clone(self);
        let cancel = self.cancel_token(&parked.state.run_id);
        if status == "approved" {
            if let Some(dir) = decision
                .output_dir
                .as_deref()
                .filter(|dir| permission::valid_output_dir(dir))
            {
                let _ = self.update_session(&parked.state.session_id, |summary| {
                    summary.remembered_output_dir = Some(dir.to_owned());
                });
            }
            std::thread::spawn(move || host.execute_plan(parked, row, cancel));
        } else {
            let _ = self.append_event(
                &parked.state.session_id,
                "progress_card",
                json!({"run_id": parked.state.run_id, "plan_id": plan_id, "step_index": null, "tool_id": null, "status": status, "code": format!("plan_{status}")}),
            );
            if status == "expired" {
                let _ = self.append_event(
                    &parked.state.session_id,
                    "error",
                    json!({"run_id": parked.state.run_id, "code": "plan_expired", "status": null, "plan_id": plan_id}),
                );
            }
            let result = unexecuted_plan_result(plan_id, status, &row.plan_json);
            std::thread::spawn(move || host.resume(parked, result, cancel));
        }
        match decided {
            Ok(_) => Ok(()),
            Err(PlanError::Expired) => Err(HostError::Expired),
            Err(error) => Err(error.into()),
        }
    }

    fn execute_plan(
        self: &Arc<Self>,
        mut parked: Parked<S>,
        row: AgentPlanRow,
        cancel: CancellationToken,
    ) {
        let plan_id = parked.plan_id.clone();
        let session_id = parked.state.session_id.clone();
        let run_id = parked.state.run_id.clone();
        let intent: Value = serde_json::from_str(&row.plan_json).unwrap_or(Value::Null);
        let steps = intent
            .get("steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let progress = |index: usize, tool_id: &str, status: &str, created: Vec<&str>| {
            let _ = self.append_event(
                &session_id,
                "progress_card",
                json!({"run_id": run_id, "plan_id": plan_id, "step_index": index, "tool_id": tool_id,
                    "status": status, "code": null, "created_task_ids": created}),
            );
        };
        let mut results = Vec::new();
        let mut status = "completed";
        for (index, step) in steps.iter().enumerate() {
            let tool_id = step.get("tool_id").and_then(Value::as_str).unwrap_or("");
            let input = step.get("input").cloned().unwrap_or(Value::Null);
            if cancel.is_cancelled() {
                status = "cancelled";
                break;
            }
            progress(index, tool_id, "running", vec![]);
            let at = now();
            // The first step re-reads the Store fingerprint: a change after the
            // approval expires the plan. Later steps are bounded by their own
            // per-task revision check in `authorize_step`.
            let authorized = self.service.with_store(|store| {
                let current = if index == 0 {
                    plan::plan_revision(store, &intent).map_err(ExecutionError::from)?
                } else {
                    row.store_revision_seen
                };
                executor::authorize_step(store, &plan_id, current, &at, tool_id, &input)
            });
            let outcome = match authorized {
                Ok(Ok(tool)) => parked.state.executor.dispatch_authorized(
                    &tool,
                    &input,
                    &mut parked.state.writer,
                    &at,
                ),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(ExecutionError::ToolDenied),
            };
            let landed = parked.state.executor.take_created_tasks();
            if !landed.is_empty() {
                self.note_agent_tasks(&session_id, &landed);
                self.announce_activity(&[session_id.as_str()]);
            }
            match outcome {
                Ok(mut summary) => {
                    let created = if tool_id == "create_task"
                        && summary.get("created") == Some(&json!(true))
                    {
                        summary
                            .get("task_id")
                            .and_then(Value::as_str)
                            .into_iter()
                            .collect()
                    } else {
                        vec![]
                    };
                    progress(index, tool_id, "done", created);
                    if tool_id == "working_aoi_set" {
                        let persisted_aoi = if input.get("clear") == Some(&json!(true)) {
                            json!({"state": "none"})
                        } else {
                            self.service
                                .application_preview_working_aoi_set(&input)
                                .or_else(|_| self.service.application_working_aoi())
                                .unwrap_or_else(|_| summary.clone())
                        };
                        let _ = self.update_workspace_state(&session_id, |ws| {
                            ws.session_id = session_id.clone();
                            ws.initialized = true;
                            ws.working_aoi = persisted_aoi;
                        });
                        let state_str = summary
                            .get("state")
                            .and_then(Value::as_str)
                            .unwrap_or("none")
                            .to_owned();
                        let display_name = summary
                            .get("display_name")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        let bbox = summary
                            .get("bbox")
                            .and_then(Value::as_array)
                            .and_then(|arr| {
                                arr.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>()
                            });
                        let _ = self.append_event(
                            &session_id,
                            "workspace_sync",
                            json!({
                                "run_id": run_id,
                                "action": "aoi_changed",
                                "display_name": display_name,
                                "bbox": bbox,
                            }),
                        );
                        self.notify(AgentHostNotification::AoiChanged {
                            session_id: session_id.clone(),
                            run_id: run_id.clone(),
                            state: state_str,
                            display_name,
                            bbox,
                        });
                    } else if tool_id == "search_start"
                        && summary.get("stage").and_then(Value::as_str) == Some("done")
                    {
                        self.record_search_batch(
                            &session_id,
                            &run_id,
                            &summary,
                            &mut parked.state.writer,
                        );
                        if let Some(req_id) = summary
                            .get("request_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                        {
                            parked
                                .state
                                .circuit_breaker
                                .observe_search_progress(&req_id, &mut summary);
                        }
                    }
                    let mut done = json!({"tool_id": tool_id, "status": "done", "result": summary});
                    if tool_id == "search_start"
                        && done["result"].get("terminal") == Some(&Value::Bool(false))
                    {
                        done["next_action_zh"] = json!(
                            "检索仍在后台进行，请立即调用 search_progress 传入该 request_id 获取最终检索批次，切勿直接结束回答。"
                        );
                    }
                    if tool_id == "create_task" {
                        // L4 r8 D4: say where the directory came from, so
                        // the model does not guess (never the path itself).
                        let source = intent
                            .pointer("/card_snapshot/output_dir_source")
                            .and_then(Value::as_str);
                        done["output_dir_source"] = json!(source);
                        done["output_dir_note_zh"] = json!(output_dir_note_zh(source));
                    }
                    results.push(done);
                }
                Err(ExecutionError::Plan(PlanError::Expired)) => {
                    progress(index, tool_id, "expired", vec![]);
                    let _ = self.append_event(
                        &session_id,
                        "error",
                        json!({"run_id": run_id, "code": "plan_expired", "status": null, "plan_id": plan_id}),
                    );
                    results.push(json!({"tool_id": tool_id, "status": "expired"}));
                    status = "expired";
                    break;
                }
                Err(error) => {
                    progress(index, tool_id, "failed", vec![]);
                    // L4 r7: the model got only {"code":"denied"}; the step
                    // result now says why, and the run record keeps it.
                    let denial = ToolDenial::of_execution(&error, tool_id, &input);
                    let mut result = denial.result(&input);
                    self.record_denial(
                        &mut parked.state.writer,
                        &denial.event(&result, Some((&plan_id, index))),
                    );
                    result["tool_id"] = json!(tool_id);
                    result["status"] = json!("failed");
                    results.push(result);
                    status = "failed";
                    break;
                }
            }
        }
        if status == "cancelled" {
            let _ = self
                .service
                .with_store(|store| store.expire_agent_plan(&plan_id));
            let _ = self.seal(parked.state, "cancelled", Some("[上一轮已取消]"), true);
            let _ = self.append_event(
                &session_id,
                "error",
                json!({"run_id": run_id, "code": "cancelled", "status": "cancelled", "plan_id": plan_id}),
            );
            return;
        }
        self.resume(
            parked,
            json!({"plan_id": plan_id, "status": status, "steps": results}),
            cancel,
        );
    }

    fn resume(self: &Arc<Self>, parked: Parked<S>, result: Value, cancel: CancellationToken) {
        let Parked {
            mut state, call_id, ..
        } = parked;
        state.checkpoint.pending_call = None;
        state.checkpoint.messages.push(Message {
            role: "tool".into(),
            content: json!({"id": call_id, "result": privacy::scrub(&result)}).to_string(),
        });
        state.checkpoint.tool_results.push(result);
        self.drive(state, cancel);
    }

    fn humanize_store_ref_label(&self, store: &TaskStore, kind: &str, id: &str) -> Option<String> {
        match kind {
            "batch" => {
                let count = store.count_batch_candidate_rows(id).ok();
                let ds_name = store
                    .query_snapshot_json(id)
                    .ok()
                    .flatten()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .and_then(|snap| {
                        let provider = snap
                            .get("provider_id")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let query_map = snap
                            .get("query")
                            .and_then(Value::as_object)
                            .or_else(|| snap.as_object())?;
                        if let Some(res_id) = source_resource_id(provider, query_map) {
                            if let Some(zh) = self.source_display_name(&res_id) {
                                return Some(zh);
                            }
                        }
                        query_map
                            .get("logical_dataset")
                            .or_else(|| query_map.get("dataset"))
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                    });
                match (ds_name, count) {
                    (Some(name), Some(n)) => Some(format!("{name} · {n} 景")),
                    (Some(name), None) => Some(name),
                    (None, Some(n)) => Some(format!("检索结果 · {n} 景")),
                    (None, None) => None,
                }
            }
            "task" => {
                let task = store.load_task_record(id).ok().flatten()?;
                let count = task.runtime.selected_item_ids.len();
                let ds_name = source_resource_id(&task.snapshot.provider_id, &task.snapshot.query)
                    .and_then(|res_id| self.source_display_name(&res_id))
                    .or_else(|| {
                        task.snapshot
                            .query
                            .get("logical_dataset")
                            .or_else(|| task.snapshot.query.get("dataset"))
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                    });
                match (ds_name, count > 0) {
                    (Some(name), true) => Some(format!("{name}（{count} 景）")),
                    (Some(name), false) => Some(name),
                    (None, true) => Some(format!("下载任务（{count} 景）")),
                    (None, false) => None,
                }
            }
            _ => None,
        }
    }

    /// Natural Markdown is accepted first-class (`structured == false`); only a
    /// broken JSON wrapper (`{...` or ```` ```json ```` that failed to parse)
    /// records `conclusion_unparsed`. When `answer.claims` is empty on a clean
    /// Markdown conclusion, verified provenance claims are auto-derived from
    /// batches and tasks observed during this run.
    fn conclude(
        &self,
        mut state: RunState<S>,
        mut answer: StructuredAnswer,
        structured: bool,
        reasoning_text: &str,
        thinking_ms: Option<u64>,
    ) -> Result<(), HostError> {
        let broken_json = {
            let trimmed = answer.text.trim_start();
            !structured && (trimmed.starts_with('{') || trimmed.starts_with("```json"))
        };
        if !structured && !broken_json {
            if let Some(manifest) = state.goal_manifest.as_ref() {
                if manifest.needs_disclosure_in(&answer.text) {
                    if let Some(suffix) = manifest.honest_disclosure_suffix() {
                        if answer.text.trim().is_empty() {
                            answer.text = suffix;
                        } else {
                            answer.text = format!("{}\n\n{}", answer.text.trim_end(), suffix);
                        }
                    }
                }
            }
        }
        let mut shown = Vec::new();
        let mut claims: Vec<citation::Claim> = answer
            .claims
            .iter()
            .map(|claim| {
                let (refs, labels): (Vec<_>, Vec<_>) =
                    claim.refs.iter().map(|raw| reference(raw)).unzip();
                shown.push(labels);
                citation::Claim {
                    statement: claim.statement.clone(),
                    category: "observed".into(),
                    refs,
                }
            })
            .collect();
        let checked = self.service.with_store(|store| {
            let events = run::read_sealed(&self.root, store, &state.run_id)?;
            if !structured && !broken_json && claims.is_empty() {
                let mut seen_batches = std::collections::HashSet::new();
                let mut seen_tasks = std::collections::HashSet::new();
                for event in &events {
                    if event.kind == "tool_result" {
                        if let Some(batch_id) = event
                            .payload
                            .pointer("/summary/search_batch_id")
                            .and_then(Value::as_str)
                        {
                            if seen_batches.insert(batch_id.to_owned())
                                && store.search_batch_exists(batch_id).unwrap_or(false)
                            {
                                let (r, label) = reference(&format!("batch:{batch_id}"));
                                shown.push(vec![label]);
                                claims.push(citation::Claim {
                                    statement: "本次检索结果批次".into(),
                                    category: "observed".into(),
                                    refs: vec![r],
                                });
                            }
                        }
                        if let Some(task_id) = event
                            .payload
                            .pointer("/summary/task_id")
                            .and_then(Value::as_str)
                        {
                            if seen_tasks.insert(task_id.to_owned())
                                && store
                                    .load_task_record(task_id)
                                    .is_ok_and(|task| task.is_some())
                            {
                                let (r, label) = reference(&format!("task:{task_id}"));
                                shown.push(vec![label]);
                                claims.push(citation::Claim {
                                    statement: "本次下载任务状态".into(),
                                    category: "observed".into(),
                                    refs: vec![r],
                                });
                            }
                        }
                    } else if event.kind == "execution" {
                        if let Some(task_id) = event.payload.get("task_id").and_then(Value::as_str)
                        {
                            if seen_tasks.insert(task_id.to_owned())
                                && store
                                    .load_task_record(task_id)
                                    .is_ok_and(|task| task.is_some())
                            {
                                let (r, label) = reference(&format!("task:{task_id}"));
                                shown.push(vec![label]);
                                claims.push(citation::Claim {
                                    statement: "本次下载任务状态".into(),
                                    category: "observed".into(),
                                    refs: vec![r],
                                });
                            }
                        }
                    }
                }
            }
            let checked = citation::check_answer(
                &self.root,
                store,
                &state.run_id,
                &citation::Answer {
                    text: answer.text.clone(),
                    claims: Some(claims),
                },
            )?;
            for claim_refs in &mut shown {
                for shown_ref in claim_refs.iter_mut() {
                    let kind = shown_ref["kind"].as_str().unwrap_or("").to_owned();
                    let id = shown_ref["id"].as_str().unwrap_or("").to_owned();
                    if let Some(human_label) = self.humanize_store_ref_label(store, &kind, &id) {
                        shown_ref["label"] = json!(human_label);
                    }
                }
            }
            Ok::<_, citation::CitationError>((checked, events))
        })?;
        let (checked, events) = checked.map_err(|error| match error {
            citation::CitationError::Run(error) => error,
        })?;
        let fact_report = if !structured && !broken_json {
            let report = crate::fact_verifier::verify_trace_reply(&events, &answer.text);
            if let Some(suffix) = report.audit_suffix_zh() {
                if answer.text.trim().is_empty() {
                    answer.text = suffix;
                } else {
                    answer.text = format!("{}\n\n{}", answer.text.trim_end(), suffix);
                }
            }
            Some(report)
        } else {
            None
        };
        let verified = checked.claims.iter().filter(|c| c.verified).count();
        if broken_json {
            state.writer.append(
                "conclusion_unparsed",
                &json!({"chars": answer.text.chars().count()}),
            )?;
        }
        let clean_thinking = history::outbound_text(reasoning_text.trim());
        let mut answer_trace = json!({"claims": checked.claims.len(), "verified": verified});
        if let Some(report) = &fact_report {
            answer_trace["fact_checked_claims"] = json!(report.checked_claims);
            answer_trace["fact_mismatches"] = json!(report.mismatches.len());
        }
        if !clean_thinking.is_empty() || thinking_ms.is_some() {
            answer_trace["thinking_ms"] = json!(thinking_ms);
            answer_trace["reasoning_chars"] = json!(clean_thinking.chars().count());
        }
        state.writer.append("answer", &answer_trace)?;
        let session_id = state.session_id.clone();
        let run_id = state.run_id.clone();
        let closing_text = answer.text.clone();
        self.seal(state, "completed", Some(&closing_text), true)?;
        let claims: Vec<Value> = checked
            .claims
            .iter()
            .zip(shown)
            .map(|(claim, refs)| {
                let refs: Vec<Value> = refs
                    .into_iter()
                    .map(|mut shown_ref| {
                        let kind = shown_ref["kind"].as_str().unwrap_or("");
                        let id = shown_ref["id"].as_str().unwrap_or("");
                        let (step, count) = evidence_step(&events, kind, id);
                        shown_ref["trace_sequence"] = json!(step);
                        shown_ref["response_count"] = json!(count);
                        shown_ref
                    })
                    .collect();
                json!({"statement": claim.statement, "verified": claim.verified, "refs": refs})
            })
            .collect();
        // From the sealed trace, so a resend before a parked plan card counts.
        let reasoning_off_refused = run::run_flags(&events).reasoning_off_refused;
        let mut conclusion_payload = json!({
            "run_id": run_id,
            "text": answer.text,
            "claims": claims,
            "reasoning_off_refused": reasoning_off_refused,
        });
        if !clean_thinking.is_empty() {
            conclusion_payload["thinking_text"] = json!(clean_thinking);
            conclusion_payload["thinking_ms"] = json!(thinking_ms);
        }
        self.append_event(&session_id, "conclusion", conclusion_payload)?;
        self.clear_resume_checkpoint(&session_id);
        Ok(())
    }

    /// Cancels a running or parked run. A parked run is sealed `cancelled`
    /// here; a running one is sealed by its worker when the model loop stops.
    /// Only a crash or restart produces `interrupted`.
    pub fn run_cancel(&self, run_id: &str) -> Result<bool, HostError> {
        if !valid_id(run_id) {
            return Err(HostError::Invalid);
        }
        let parked = {
            let mut live = lock(&self.live);
            let Some(run) = live.get_mut(run_id) else {
                return Ok(false);
            };
            run.cancel.cancel();
            run.parked.take()
        };
        if let Some(parked) = parked {
            let session_id = parked.state.session_id.clone();
            let parked_plan = parked.plan_id.clone();
            // Seal even when the plan cannot be marked expired (as a cancel
            // during execution does): the run is no longer parked, so its
            // card can never be approved, and no running row or busy session
            // may be left behind.
            let _ = self
                .service
                .with_store(|store| store.expire_agent_plan(&parked.plan_id));
            let sealed = self.seal(parked.state, "cancelled", Some("[上一轮已取消]"), true);
            self.append_event(
                &session_id,
                "error",
                json!({"run_id": run_id, "code": "cancelled", "status": "cancelled", "plan_id": parked_plan}),
            )?;
            sealed?;
        }
        Ok(true)
    }

    pub fn runs_list(&self, session_id: &str) -> Result<Vec<RunSummary>, HostError> {
        let summary = self.load_session(session_id)?;
        self.service.with_store(|store| -> Result<_, HostError> {
            let mut runs = Vec::new();
            for run_id in summary.run_ids.iter().rev() {
                if let Some(row) = store.agent_run(run_id)? {
                    runs.push(run_summary(&self.root, row));
                }
            }
            Ok(runs)
        })?
    }

    /// The session's work for the "任务与成果" panel, newest run first: each
    /// run with the request that started it, the plans it proposed and the
    /// download tasks it created. Read from the session's own events.
    pub fn session_work(&self, session_id: &str) -> Result<Vec<WorkItem>, HostError> {
        let runs = self.runs_list(session_id)?;
        let dir = self.session_dir(session_id)?;
        let events: Vec<StreamEvent> = {
            let _files = lock(&self.files);
            BufReader::new(fs::File::open(dir.join("events.jsonl"))?)
                .lines()
                .map_while(Result::ok)
                .filter_map(|line| serde_json::from_str(&line).ok())
                .collect()
        };
        Ok(runs
            .into_iter()
            .map(|run| {
                let mut item = WorkItem {
                    request: None,
                    plan_ids: vec![],
                    created_task_ids: vec![],
                    run,
                };
                for event in &events {
                    let payload = &event.payload;
                    if payload.get("run_id").and_then(Value::as_str) != Some(&item.run.run_id) {
                        continue;
                    }
                    match event.kind.as_str() {
                        "message"
                            if payload.get("role").and_then(Value::as_str) == Some("user")
                                && item.request.is_none() =>
                        {
                            item.request = payload
                                .get("text")
                                .and_then(Value::as_str)
                                .map(|text| text.chars().take(120).collect());
                        }
                        "plan_card" => {
                            if let Some(plan_id) = payload.get("plan_id").and_then(Value::as_str) {
                                if !item.plan_ids.iter().any(|id| id == plan_id) {
                                    item.plan_ids.push(plan_id.to_owned());
                                }
                            }
                        }
                        "progress_card" => {
                            for task_id in payload
                                .get("created_task_ids")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                            {
                                if !item.created_task_ids.iter().any(|id| id == task_id) {
                                    item.created_task_ids.push(task_id.to_owned());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                item
            })
            .collect())
    }

    pub fn run_open(&self, run_id: &str) -> Result<RunDetail, HostError> {
        if !valid_id(run_id) {
            return Err(HostError::Invalid);
        }
        self.service.with_store(|store| -> Result<_, HostError> {
            let row = store.agent_run(run_id)?.ok_or(HostError::NotFound)?;
            let events = run::read_sealed(&self.root, store, run_id)?;
            Ok(RunDetail {
                run: run_summary(&self.root, row),
                events,
            })
        })?
    }

    /// What the stop-everything button would stop now, across sessions:
    /// live runs (running or waiting for a card) and the still active tasks
    /// Agent runs created. Reads the in-memory run table, the collected task
    /// ids and one runtime row per such task; never a run record (after the
    /// first load).
    pub fn activity_status(&self) -> Result<ActivityStatus, HostError> {
        let mut sessions: Vec<String> = lock(&self.live)
            .values()
            .map(|run| run.session_id.clone())
            .collect();
        let live_runs = sessions.len() as u64;
        sessions.sort();
        sessions.dedup();
        let tasks: Vec<String> = self
            .agent_created_tasks()
            .into_iter()
            .map(|(_, task_id)| task_id)
            .collect();
        let agent_active_tasks = if tasks.is_empty() {
            0
        } else {
            self.service.with_store(|store| {
                tasks
                    .iter()
                    .filter(|task_id| {
                        matches!(
                            store.load_scheduler_control_runtime(task_id),
                            Ok(Some(runtime)) if active_status(&runtime.status)
                        )
                    })
                    .count() as u64
            })?
        };
        Ok(ActivityStatus {
            live_runs,
            agent_active_tasks,
            sessions_with_live_runs: sessions,
        })
    }

    /// Appends `activity_changed` with the current status when it differs
    /// from the last one announced: to `concerned`, to every session with a
    /// live run and to every session that had one at the last announcement
    /// (so the session a run just left hears it). A status that cannot be
    /// read is not announced. Callers hold none of the host's locks.
    fn announce_activity(&self, concerned: &[&str]) {
        let mut last = lock(&self.activity);
        let Ok(status) = self.activity_status() else {
            return;
        };
        if last.as_ref() == Some(&status) {
            return;
        }
        let Ok(payload) = serde_json::to_value(&status) else {
            return;
        };
        let mut targets: std::collections::BTreeSet<String> =
            concerned.iter().map(|id| (*id).to_owned()).collect();
        targets.extend(status.sessions_with_live_runs.iter().cloned());
        if let Some(previous) = last.as_ref() {
            targets.extend(previous.sessions_with_live_runs.iter().cloned());
        }
        for session_id in targets {
            let _ = self.append_event(&session_id, "activity_changed", payload.clone());
        }
        *last = Some(status);
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn permission_mode_get(
        &self,
        session_id: Option<&str>,
    ) -> Result<PermissionModeView, HostError> {
        let global_default = PermissionMode::ConfirmChanges;
        let session_override = match session_id {
            Some(id) => self.load_session(id)?.permission_mode,
            None => None,
        };
        Ok(PermissionModeView {
            effective: session_override.unwrap_or(global_default),
            session_override,
            global_default,
        })
    }

    pub fn permission_mode_set_global(
        &self,
        _mode: PermissionMode,
    ) -> Result<PermissionModeView, HostError> {
        Err(HostError::Invalid)
    }

    pub fn permission_mode_set_session(
        &self,
        session_id: &str,
        mode: Option<PermissionMode>,
    ) -> Result<PermissionModeView, HostError> {
        {
            let _files = lock(&self.files);
            let mut summary = self.load_session(session_id)?;
            summary.permission_mode = mode;
            self.save_session(&summary)?;
        }
        self.permission_mode_get(Some(session_id))
    }

    /// The "stop everything" button: cancels every in-progress run (the
    /// `run_cancel` path) and every still active task an Agent run created
    /// (`create_task` with `created=true` in a run record), through the
    /// service's task cancel. One failure never stops the rest: a run or task
    /// that could not be stopped is listed as failed, a task cancel that lost
    /// a revision race is retried once at the new revision, and each affected
    /// session gets one `stop_all` event with what actually happened.
    pub fn stop_all(&self) -> Result<StopAllResult, HostError> {
        let mut result = StopAllResult::default();
        let mut sessions = std::collections::BTreeSet::new();
        let live: Vec<(String, String)> = lock(&self.live)
            .iter()
            .map(|(run_id, run)| (run_id.clone(), run.session_id.clone()))
            .collect();
        for (run_id, session_id) in live {
            match self.run_cancel(&run_id) {
                Ok(true) => result.cancelled_runs.push(run_id),
                Ok(false) => continue,
                Err(_) => result.failed_runs.push(run_id),
            }
            sessions.insert(session_id);
        }
        for (session_id, task_id) in self.agent_created_tasks() {
            match self.cancel_agent_task(&task_id) {
                Some(true) => result.cancelled_tasks.push(task_id),
                Some(false) => result.failed_tasks.push(task_id),
                None => continue,
            }
            sessions.insert(session_id);
        }
        let payload = serde_json::to_value(&result)?;
        for session_id in &sessions {
            let _ = self.append_event(session_id, "stop_all", payload.clone());
        }
        let sessions: Vec<&str> = sessions.iter().map(String::as_str).collect();
        self.announce_activity(&sessions);
        Ok(result)
    }

    /// `None` when the task is no longer active (nothing to stop); otherwise
    /// whether the cancel landed. A refused cancel is retried once when the
    /// task moved to a new revision meanwhile and is still active.
    fn cancel_agent_task(&self, task_id: &str) -> Option<bool> {
        let active = |task_id: &str| -> Result<Option<u64>, ()> {
            let task = self
                .service
                .with_store(|store| store.load_task_record(task_id))
                .map_err(|_| ())?
                .map_err(|_| ())?;
            Ok(task
                .filter(|task| {
                    matches!(
                        task.runtime.status,
                        TaskLifecycleStatus::Queued
                            | TaskLifecycleStatus::Running
                            | TaskLifecycleStatus::Paused
                            | TaskLifecycleStatus::Interrupted
                    )
                })
                .map(|task| task.runtime.revision))
        };
        let revision = match active(task_id) {
            Ok(Some(revision)) => revision,
            Ok(None) => return None,
            Err(()) => return Some(false),
        };
        let cancel = |revision: u64| {
            self.service
                .application_task_action(task_id, revision, "cancel", &self.new_id("stop-all"))
                .is_ok()
        };
        if cancel(revision) {
            return Some(true);
        }
        match active(task_id) {
            Ok(Some(now)) if now != revision => Some(cancel(now)),
            Ok(None) => None,
            _ => Some(false),
        }
    }

    /// Tasks Agent runs created, as (session, task). Read from the run
    /// records once, on first use; afterwards kept current by
    /// `note_agent_tasks` when a run's create_task lands and by session
    /// deletion. A scan that could not read a run record (or the sessions
    /// directory) is used for this call only and not kept, so the next call
    /// reads the records again rather than missing a task for the rest of
    /// the process; until it reads cleanly every call scans.
    fn agent_created_tasks(&self) -> Vec<(String, String)> {
        let mut cache = lock(&self.agent_tasks);
        if let Some(tasks) = cache.as_ref() {
            return tasks.clone();
        }
        let (found, complete) = self.scan_agent_created_tasks();
        if complete {
            *cache = Some(found.clone());
        }
        found
    }

    /// Adds tasks a run of `session_id` just created. Before the first load
    /// nothing is kept: that load reads them from the run record.
    fn note_agent_tasks(&self, session_id: &str, task_ids: &[String]) {
        if let Some(tasks) = lock(&self.agent_tasks).as_mut() {
            for task_id in task_ids {
                if !tasks.iter().any(|(_, id)| id == task_id) {
                    tasks.push((session_id.to_owned(), task_id.clone()));
                }
            }
        }
    }

    /// Tasks Agent runs created, from the run records of every session: an
    /// `execution` event of `create_task` whose result said `created=true`.
    /// A task the service reused (`created=false`) belongs to the user.
    /// `complete` is false when the sessions directory or a listed run
    /// record could not be read (that record is skipped). A session whose
    /// summary cannot be read is not listed by `sessions_list` and so not
    /// seen here; that is not detected as an error.
    fn scan_agent_created_tasks(&self) -> (Vec<(String, String)>, bool) {
        let mut found: Vec<(String, String)> = Vec::new();
        let Ok(sessions) = self.sessions_list() else {
            return (found, false);
        };
        let mut complete = true;
        for summary in sessions {
            for run_id in &summary.run_ids {
                if !valid_id(run_id) {
                    continue;
                }
                let dir = self.root.join("agent").join("runs").join(run_id);
                let Ok(events) = run::read_trace(&dir) else {
                    complete = false;
                    continue;
                };
                for event in events {
                    let payload = &event.payload;
                    if event.kind != "execution"
                        || payload.get("tool_id").and_then(Value::as_str) != Some("create_task")
                        || payload.get("created") != Some(&json!(true))
                    {
                        continue;
                    }
                    if let Some(task_id) = payload.get("task_id").and_then(Value::as_str) {
                        if !found.iter().any(|(_, id)| id == task_id) {
                            found.push((summary.session_id.clone(), task_id.to_owned()));
                        }
                    }
                }
            }
        }
        (found, complete)
    }
}

/// A task the stop-everything button can still stop (the same states
/// `cancel_agent_task` cancels).
fn active_status(status: &TaskLifecycleStatus) -> bool {
    matches!(
        status,
        TaskLifecycleStatus::Queued
            | TaskLifecycleStatus::Running
            | TaskLifecycleStatus::Paused
            | TaskLifecycleStatus::Interrupted
    )
}

/// Request ids of the searches a run started: its executed `search_start`
/// steps, from their results in the run record.
fn run_searches(events: &[TraceEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| {
            event.kind == "tool_result"
                && event.payload.get("tool_id").and_then(Value::as_str) == Some("search_start")
        })
        .filter_map(|event| {
            event
                .payload
                .pointer("/summary/request_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

fn run_summary(root: &Path, row: AgentRunRow) -> RunSummary {
    let (cached, cache_creation, reasoning) = run::run_extra_usage(root, &row.run_id);
    let has_usage = row.usage_prompt.is_some()
        || row.usage_completion.is_some()
        || cached.is_some()
        || cache_creation.is_some();
    RunSummary {
        run_id: row.run_id,
        session_id: row.session_id,
        status: row.status,
        started_at: row.started_at,
        ended_at: row.ended_at,
        model_route: row.model_route,
        reasoning_off_refused: row.reasoning_off_refused,
        truncated: row.truncated,
        usage_total: has_usage.then(|| RunUsage {
            prompt: row.usage_prompt,
            completion: row.usage_completion,
            total: row
                .usage_prompt
                .unwrap_or(0)
                .saturating_add(row.usage_completion.unwrap_or(0)),
            cached,
            cache_creation,
            reasoning,
        }),
    }
}

fn build_gee_artifact_event_payload(
    run_id: &str,
    call_id: &str,
    tool_id: &str,
    safe_in: &Value,
    safe_out: &Value,
    ext_args: &Value,
) -> Option<Value> {
    let should_emit = tool_id == "gee_export_preflight"
        || safe_out.get("artifact_id").is_some()
        || safe_out.get("approval_id").is_some()
        || safe_out.get("series").is_some()
        || safe_out
            .get("quota_confirm_required")
            .and_then(Value::as_bool)
            == Some(true);
    if !should_emit {
        return None;
    }
    let mut summary_ev = safe_out.clone();
    if let Some(obj) = summary_ev.as_object_mut() {
        if let Some(series) = safe_out
            .get("series")
            .or_else(|| safe_out.pointer("/stats/series"))
        {
            obj.insert("series".to_owned(), series.clone());
        }
    }
    let mut ev = json!({
        "run_id": run_id,
        "call_id": call_id,
        "tool_id": tool_id,
        "tool_args": safe_in,
        "summary": summary_ev,
    });
    if tool_id == "gee_script_inspect" {
        ev["script_preview"] = ext_args
            .get("script")
            .and_then(Value::as_str)
            .map(|s| Value::String(crate::history::outbound_text(s)))
            .unwrap_or(Value::Null);
    }
    Some(ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal() -> Value {
        json!({"error": "plan_refused", "code": "AGENT_INVALID_REQUEST", "reason": "input_invalid",
            "message_zh": "计划里的条件格式不对，缺少或多出了内容。", "hint_zh": "按 expected_input 修正。",
            "step": 0, "tool_id": "search_start", "unknown_fields": ["source"],
            "missing_fields": ["resource_id"], "detail": "unknown field", "expected_input": {}})
    }

    /// A key the model invented with a quote after a colon is the same text
    /// at the same nesting in a Chat Completions request's tool-call
    /// arguments, so it cannot be reached end to end there (the arguments
    /// make the body unsendable first); Anthropic sends the arguments as an
    /// object, so the result is the only place it would be escaped.
    #[test]
    fn invented_keys_that_read_as_a_path_once_nested_are_dropped() {
        let mut result = refusal();
        result["unknown_fields"] = json!(["a:\""]);
        let safe = nested_safe(result);
        assert_eq!(safe["unknown_fields"], json!([]), "{safe}");
        assert_eq!(safe["tool_id"], "search_start");
        assert_eq!(safe["missing_fields"], json!(["resource_id"]));
    }

    #[test]
    fn a_tool_id_that_reads_as_a_path_once_nested_is_blanked() {
        let mut result = refusal();
        result["tool_id"] = json!("x:\"y");
        let safe = nested_safe(result);
        assert!(safe["tool_id"].is_null(), "{safe}");
        assert_eq!(safe["reason"], "input_invalid");
    }

    #[test]
    fn a_result_still_unsafe_keeps_only_the_hosts_own_fields() {
        let mut result = refusal();
        result["missing_fields"] = json!(["b:\""]);
        assert_eq!(
            nested_safe(result),
            json!({"error": "plan_refused", "code": "AGENT_INVALID_REQUEST",
                "reason": "input_invalid", "message_zh": "计划里的条件格式不对，缺少或多出了内容。"})
        );
    }

    #[test]
    fn a_safe_result_is_unchanged_and_a_long_detail_is_capped() {
        assert_eq!(nested_safe(refusal()), refusal());
        let mut long = PlanRefusal::new(HostError::Invalid, "input_invalid");
        long.detail = Some("x".repeat(1_000));
        let result = long.result(&json!({}));
        assert_eq!(
            result["detail"].as_str().unwrap().chars().count(),
            MAX_DETAIL_CHARS
        );
    }

    #[test]
    fn legacy_dataset_expands_bare_grd_for_sentinel1_and_keeps_non_array_invalid() {
        let aoi = json!({
            "type": "Polygon",
            "coordinates": [[[103.9, 30.5], [104.2, 30.5], [104.2, 30.8], [103.9, 30.8], [103.9, 30.5]]]
        });
        let mut input = json!({
            "resource_id": "science.sentinel-1.standard.v1",
            "aoi": aoi,
            "filters": {
                "processing_levels": ["GRD", "SLC"]
            }
        });
        legacy_dataset(&mut input);
        assert_eq!(input["filters"]["dataset"], json!("SENTINEL-1"));
        assert_eq!(
            input["filters"]["processing_levels"],
            json!(["GRD_HD", "GRD_HS", "GRD_MD", "GRD_MS", "SLC"])
        );
        assert!(legacy_filter_problem(&input).is_none());

        // Non-array processing_levels stays untouched so legacy_filter_problem still catches it
        let mut bad = json!({
            "resource_id": "science.sentinel-1.standard.v1",
            "aoi": aoi,
            "filters": {
                "processing_levels": "GRD"
            }
        });
        legacy_dataset(&mut bad);
        assert!(legacy_filter_problem(&bad).is_some());
    }

    #[test]
    fn optical_filters_normalize_cloud_cover_and_platform_aliases_and_reject_unknown_keys() {
        let mut landsat = json!({
            "resource_id": "science.landsat.c2-l2.v1",
            "time_range": {
                "start": "2024-09-01T00:00:00Z",
                "end": "2024-11-30T23:59:59Z"
            },
            "filters": {
                "cloud_cover_percent": "40%",
                "platform": ["Landsat-8", "Landsat 9"],
                "product_level": "L2SP"
            }
        });
        normalize_optical_filters(&mut landsat);
        assert_eq!(
            landsat["filters"],
            json!({
                "cloud_cover_lte": 40.0,
                "spacecraft": ["landsat_8", "landsat_9"],
                "processing_levels": ["l2sp"]
            })
        );
        assert_eq!(optical_filter_problem(&landsat), None);

        // Minimal Landsat C2 L2 query with only cloud_cover_percent passes validation via RestoreAll defaults
        let mut minimal = json!({
            "resource_id": "science.landsat.c2-l2.v1",
            "filters": { "cloud_cover_percent": 40 }
        });
        normalize_optical_filters(&mut minimal);
        assert_eq!(minimal["filters"], json!({ "cloud_cover_lte": 40 }));
        assert_eq!(optical_filter_problem(&minimal), None);

        // Unknown SAR-only filter on optical dataset is rejected at plan time
        let mut invalid = json!({
            "resource_id": "science.landsat.c2-l2.v1",
            "filters": { "polarizations": ["VV"] }
        });
        normalize_optical_filters(&mut invalid);
        let err = optical_filter_problem(&invalid).expect("must reject SAR filter on Landsat");
        assert!(err.contains("polarizations"), "{err}");
    }

    #[test]
    fn gee_artifact_event_scrubs_script_preview_and_includes_series() {
        let raw_args = json!({
            "script": "secret_path = '/Users/alice/private/key.pem'\ncol = ee.ImageCollection('COPERNICUS/S2_SR_HARMONIZED')\nRESULT = {'ok': 1}"
        });
        let safe_out = json!({
            "ok": true,
            "approval_id": "gee-appr-1234567890abcdef",
            "blocked": false
        });
        let ev = build_gee_artifact_event_payload(
            "run-1",
            "call-1",
            "gee_script_inspect",
            &json!({}),
            &safe_out,
            &raw_args,
        )
        .expect("gee_script_inspect emits gee_artifact");
        let preview = ev["script_preview"]
            .as_str()
            .expect("script_preview string");
        assert!(!preview.contains("/Users/alice"));
        assert!(preview.contains("COPERNICUS/S2_SR_HARMONIZED"));
        assert!(crate::run::safe_value(&ev).is_ok());

        let ts_out = json!({
            "ok": true,
            "operation": "time_series",
            "artifact_id": "gee-art-ts001",
            "series": [
                {"window_start": "2025-05-01", "window_end": "2025-06-01", "scene_count": 3, "mean": 0.42}
            ]
        });
        let ts_ev = build_gee_artifact_event_payload(
            "run-2",
            "call-2",
            "gee_cloud_analyze",
            &json!({"operation": "time_series"}),
            &ts_out,
            &json!({"operation": "time_series"}),
        )
        .expect("time_series emits gee_artifact");
        assert_eq!(ts_ev["summary"]["series"].as_array().unwrap().len(), 1);
        assert_eq!(ts_ev["summary"]["series"][0]["scene_count"], 3);

        for (label, pre_out) in [
            (
                "auto",
                json!({
                    "ok": true,
                    "ready": true,
                    "recommended_route": "local_chunked_geotiff_auto",
                    "quota_confirm_required": false,
                    "raw_total_chunks": 4
                }),
            ),
            (
                "external",
                json!({
                    "ok": true,
                    "ready": true,
                    "recommended_route": "external_batch_export",
                    "tile_safety": "tile_unsafe",
                    "quota_confirm_required": false,
                    "raw_total_chunks": 200
                }),
            ),
            (
                "missing",
                json!({
                    "ok": true,
                    "ready": false,
                    "recommended_route": "needs_parameters",
                    "missing_parameters": ["bbox"],
                    "quota_confirm_required": false
                }),
            ),
        ] {
            let pre_ev = build_gee_artifact_event_payload(
                "run-3",
                label,
                "gee_export_preflight",
                &json!({"asset_id": "COPERNICUS/S2_SR_HARMONIZED"}),
                &pre_out,
                &json!({"asset_id": "COPERNICUS/S2_SR_HARMONIZED"}),
            )
            .unwrap_or_else(|| panic!("gee_export_preflight ({label}) must emit gee_artifact"));
            assert!(crate::run::safe_value(&pre_ev).is_ok());
        }
    }

    #[test]
    fn source_resource_id_resolves_logical_dataset_without_native_collection_id() {
        let s1 = json!({"logical_dataset": "sar.sentinel1.slc.v1", "dataset": "SENTINEL-1"});
        assert_eq!(
            source_resource_id("asf.catalog", s1.as_object().unwrap()).as_deref(),
            Some("science.sentinel-1.standard.v1")
        );
        let s1_no_logical = json!({"dataset": "SENTINEL-1"});
        assert_eq!(
            source_resource_id("asf.nasa.earthdata", s1_no_logical.as_object().unwrap()).as_deref(),
            Some("science.sentinel-1.standard.v1")
        );
        let s2 = json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "native_collection_id": "sentinel-2-l2a"
        });
        assert_eq!(
            source_resource_id("microsoft.planetary-computer", s2.as_object().unwrap()).as_deref(),
            Some("science.sentinel-2.l2a.v1")
        );
        let s2_alias = json!({
            "native_collection_id": "sentinel-2-l2a"
        });
        assert_eq!(
            source_resource_id("mpc.stac", s2_alias.as_object().unwrap()).as_deref(),
            Some("science.sentinel-2.l2a.v1")
        );
    }

    #[test]
    fn gis_external_execute_is_gated_as_local_execution_and_blocks_output_overrides() {
        assert!(is_side_effect_local_execution_tool("gis_external_execute"));
        assert!(!is_side_effect_local_execution_tool("gis_toolchains_status"));
        assert!(!is_side_effect_local_execution_tool("gis_raster_inspect"));

        assert!(local_execution_has_unverified_output_override(&json!({
            "params": { "OUTPUT": "/tmp/evil.tif" }
        })));
        assert!(local_execution_has_unverified_output_override(&json!({
            "params": { "out_raster": "C:\\evil.tif" }
        })));
        assert!(local_execution_has_unverified_output_override(&json!({
            "params": { "out_rasterdataset": "C:\\evil.tif" }
        })));
        assert!(local_execution_has_unverified_output_override(&json!({
            "output_filename": "../escaped/out.geojson"
        })));
        assert!(!local_execution_has_unverified_output_override(&json!({
            "toolchain": "arcgis_pro",
            "action": "run_algorithm",
            "algorithm": "management.Project",
            "output_filename": "hefei_wgs84.shp",
            "params": { "out_coor_system": "EPSG:4326", "out_cell_size": 30 }
        })));
        assert!(!local_execution_has_unverified_output_override(&json!({
            "toolchain": "qgis",
            "action": "run_algorithm",
            "algorithm": "native:buffer",
            "output_filename": "hefei_buf.geojson",
            "params": { "DISTANCE": 500 }
        })));
    }
}
