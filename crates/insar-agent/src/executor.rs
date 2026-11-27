//! Local policy for a future ApplicationService tool adapter. No model input
//! can directly name a Tauri command or operate the task Store without approval.
use crate::{
    catalog,
    plan::{self, PlanError},
    run::{RunError, RunWriter},
};
use insar_store::TaskStore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    Transient,
    InvalidParameters,
    Permission,
    Authentication,
    Unsupported,
    ResultUnknown,
    Business,
    Internal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    RetryBounded,
    FixInput,
    Confirm,
    Login,
    ExplainUnsupported,
    ReadTask,
    Stop,
}
pub fn classify(code: &str) -> ErrorClass {
    match code {
        "transient" | "timeout" => ErrorClass::Transient,
        "invalid" => ErrorClass::InvalidParameters,
        "permission" => ErrorClass::Permission,
        "auth" => ErrorClass::Authentication,
        "unsupported" => ErrorClass::Unsupported,
        "unknown" => ErrorClass::ResultUnknown,
        "business" => ErrorClass::Business,
        _ => ErrorClass::Internal,
    }
}
pub fn recovery(class: ErrorClass) -> Recovery {
    match class {
        ErrorClass::Transient => Recovery::RetryBounded,
        ErrorClass::InvalidParameters => Recovery::FixInput,
        ErrorClass::Permission => Recovery::Confirm,
        ErrorClass::Authentication => Recovery::Login,
        ErrorClass::Unsupported => Recovery::ExplainUnsupported,
        ErrorClass::ResultUnknown => Recovery::ReadTask,
        ErrorClass::Business | ErrorClass::Internal => Recovery::Stop,
    }
}
#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("tool missing from catalog or cannot be dispatched")]
    ToolDenied,
    /// The service refused a call that passed the catalog and input checks;
    /// the class says why (the host tells the model, L4 r7).
    #[error("the service refused the call: {0:?}")]
    Service(ErrorClass),
    #[error("result unknown: read task by id; do not replay")]
    ResultUnknown,
    #[error("same tool result repeated: stop and explain the blocker")]
    Stalled,
    #[error("task or landed artifact is not verified")]
    Unverified,
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Run(#[from] RunError),
    #[error(transparent)]
    Store(#[from] insar_store::StoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct ToolResult {
    pub summary: Value,
    pub observed_at: String,
    pub source: String,
}
/// Desktop host composes this adapter from its one ApplicationService instance;
/// no independent Store opener, CLI runner or generic command passthrough.
pub trait ToolPort {
    fn read(&mut self, target_command: &str, input: &Value) -> Result<ToolResult, ErrorClass>;
    fn submit(&mut self, target_command: &str, input: &Value) -> Result<ToolResult, ErrorClass>;
    fn task_state(&mut self, task_id: &str, event_id: &str) -> Result<Option<Value>, ErrorClass>;
}

pub struct Executor<P> {
    port: P,
    last_result: Option<(String, String)>,
    repeat: u8,
    /// Tasks whose `execution` event said `created=true`, in order, until
    /// the host takes them (`take_created_tasks`).
    created_tasks: Vec<String>,
}
impl<P: ToolPort> Executor<P> {
    pub fn new(port: P) -> Self {
        Self {
            port,
            last_result: None,
            repeat: 0,
            created_tasks: Vec::new(),
        }
    }
    /// Tasks this executor recorded as created since the last call, taken
    /// at the moment their `execution` event was written (so also when a
    /// later write of the same step failed).
    pub fn take_created_tasks(&mut self) -> Vec<String> {
        std::mem::take(&mut self.created_tasks)
    }
    pub fn into_port(self) -> P {
        self.port
    }
    pub fn call_read(
        &mut self,
        tool_id: &str,
        input: &Value,
        trace: &mut RunWriter,
    ) -> Result<Value, ExecutionError> {
        let tool = catalog::resolve(tool_id).map_err(|_| ExecutionError::ToolDenied)?;
        // Disclosing an external request requires a separate session approval;
        // this API is restricted to tier 0 until that approval is available.
        if tool.effect != "read" || tool.confirmation != "none" {
            return Err(ExecutionError::ToolDenied);
        }
        let bounded = crate::host::bounded_input(tool_id, input);
        if !bounded {
            return Err(ExecutionError::ToolDenied);
        }
        if tool_id != "credential_readiness" {
            trace.append("tool_call",&json!({"tool_id":tool_id,"version":tool.version,"tier":tool.confirmation,"input":input}))?;
        }
        let response = self
            .port
            .read(&tool.target_command, input)
            .map_err(ExecutionError::Service)?;
        let summary = if tool_id == "credential_readiness" {
            catalog::credential_categories(&response.summary)
        } else {
            response.summary
        };
        if tool_id == "credential_readiness" {
            trace.append(
                "tool_call",
                &json!({"tool_id":tool_id,"version":tool.version,"tier":"none"}),
            )?;
        }
        self.record(
            tool_id,
            &summary,
            trace,
            &response.observed_at,
            &response.source,
        )
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "approval context, Store revision and run trace are independent safety inputs"
    )]
    pub fn submit_approved(
        &mut self,
        store: &mut TaskStore,
        plan_id: &str,
        current_revision: i64,
        now: &str,
        tool_id: &str,
        input: &Value,
        trace: &mut RunWriter,
    ) -> Result<Value, ExecutionError> {
        let tool = authorize_step(store, plan_id, current_revision, now, tool_id, input)?;
        self.dispatch_authorized(&tool, input, trace, now)
    }

    /// Second half of `submit_approved`: call the port for a step that
    /// `authorize_step` accepted. A desktop host releases its Store lock between
    /// the two halves because the service itself takes that lock.
    pub fn dispatch_authorized(
        &mut self,
        tool: &catalog::Tool,
        input: &Value,
        trace: &mut RunWriter,
        now: &str,
    ) -> Result<Value, ExecutionError> {
        let task_id = input.get("task_id").and_then(Value::as_str);
        let event_id = input.get("event_id").and_then(Value::as_str);
        trace.append(
            "tool_call",
            &json!({"tool_id":tool.tool_id,"tier":tool.confirmation,"task_id":task_id,"event_id":event_id}),
        )?;
        let response = match self.port.submit(&tool.target_command, input) {
            Ok(value) => value,
            Err(ErrorClass::ResultUnknown) => {
                trace.append(
                    "execution",
                    &json!({"task_id":task_id,"event_id":event_id,"result":"unknown; readback"}),
                )?;
                let (Some(task_id), Some(event_id)) = (task_id, event_id) else {
                    return Err(ExecutionError::ResultUnknown);
                };
                match self.port.task_state(task_id, event_id) {
                    Ok(Some(value))
                        if value.get("last_event_id").and_then(Value::as_str) == Some(event_id) =>
                    {
                        ToolResult {
                            summary: value,
                            observed_at: now.into(),
                            source: "store_task_readback".into(),
                        }
                    }
                    _ => return Err(ExecutionError::ResultUnknown),
                }
            }
            Err(class) => return Err(ExecutionError::Service(class)),
        };
        if tool.confirmation == "plan_confirm" {
            let landed = response
                .summary
                .get("task_id")
                .and_then(Value::as_str)
                .or(task_id);
            // `created` tells a task this run created from one the service
            // reused (`agent_stop_all` cancels only the former).
            let created = response.summary.get("created").cloned();
            trace.append(
                "execution",
                &json!({"tool_id":tool.tool_id,"task_id":landed,"event_id":event_id,"created":created}),
            )?;
            if tool.tool_id == "create_task" && created == Some(json!(true)) {
                if let Some(landed) = landed {
                    self.created_tasks.push(landed.to_owned());
                }
            }
        }
        self.record(
            &tool.tool_id,
            &response.summary,
            trace,
            &response.observed_at,
            &response.source,
        )
    }
    fn record(
        &mut self,
        id: &str,
        value: &Value,
        trace: &mut RunWriter,
        at: &str,
        source: &str,
    ) -> Result<Value, ExecutionError> {
        let safe = crate::run::safe_value(value)?;
        let semantic = crate::guardrails::strip_volatile_fields(&safe);
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&semantic).map_err(RunError::Json)?)
        );
        let non_terminal_search_poll =
            id == "search_progress" && value.get("terminal") == Some(&Value::Bool(false));
        if !non_terminal_search_poll
            && self.last_result.as_ref() == Some(&(id.into(), fingerprint.clone()))
        {
            self.repeat = self.repeat.saturating_add(1);
            if self.repeat >= 2 {
                trace.append("gap", &json!({"tool_id":id,"reason":"repeated_result"}))?;
                return Err(ExecutionError::Stalled);
            }
        } else if !non_terminal_search_poll {
            self.repeat = 0;
            self.last_result = Some((id.into(), fingerprint.clone()));
        }
        trace.append("tool_result",&json!({"tool_id":id,"summary":safe,"sha256":fingerprint,"observed_at":at,"source":source}))?;
        Ok(safe)
    }

    pub(crate) fn note_cached_tool(&mut self, id: &str) {
        if self.last_result.as_ref().map(|(prev, _)| prev.as_str()) != Some(id) {
            self.repeat = 0;
            self.last_result = Some((id.to_owned(), "__cached__".to_owned()));
        }
    }
}

/// Store-only half of an approved step: the plan must be approved (desktop
/// card or permission mode), the step must be frozen in it verbatim, and every Store
/// object the step names must still be at the revision the plan saw.
pub fn authorize_step(
    store: &mut TaskStore,
    plan_id: &str,
    current_revision: i64,
    now: &str,
    tool_id: &str,
    input: &Value,
) -> Result<catalog::Tool, ExecutionError> {
    let approved = plan::require_approved(store, plan_id, current_revision, now)?;
    let intent: Value =
        serde_json::from_str(&approved.plan_json).map_err(|_| ExecutionError::ToolDenied)?;
    let steps = intent
        .get("steps")
        .and_then(Value::as_array)
        .ok_or(ExecutionError::ToolDenied)?;
    let tool = catalog::resolve(tool_id).map_err(|_| ExecutionError::ToolDenied)?;
    let needs_event = tool.confirmation == "plan_confirm";
    if !steps.iter().any(|step| {
        step.get("tool_id").and_then(Value::as_str) == Some(tool_id)
            && step.get("input") == Some(input)
            && (!needs_event
                || (step.get("event_id").and_then(Value::as_str).is_some()
                    && step.get("event_id").and_then(Value::as_str)
                        == input.get("event_id").and_then(Value::as_str)))
    }) {
        return Err(ExecutionError::ToolDenied);
    }
    if tool.confirmation == "none" || !crate::host::bounded_input(tool_id, input) {
        return Err(ExecutionError::ToolDenied);
    }
    // The task revision belongs to the Store, not to the model/adapter's
    // `current_revision` argument. An old approved control must expire
    // before it reaches a provider or scheduler side effect.
    let revision_field = match tool_id {
        "update_download_task" => Some("expected_revision"),
        "task_action" => Some("revision"),
        _ => None,
    };
    if let Some(field) = revision_field {
        let task_id = input
            .get("task_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or(ExecutionError::ToolDenied)?;
        let task = store
            .load_task_record(task_id)?
            .ok_or(ExecutionError::ToolDenied)?;
        let expected = input
            .get(field)
            .and_then(Value::as_u64)
            .ok_or(ExecutionError::ToolDenied)?;
        if task.runtime.revision != expected {
            store.expire_agent_plan(plan_id)?;
            return Err(ExecutionError::Plan(PlanError::Expired));
        }
    }
    if tool_id == "create_task" {
        let batch_id = input
            .get("batch_id")
            .and_then(Value::as_str)
            .ok_or(ExecutionError::ToolDenied)?;
        // #22 B slice 2: existence only; no candidate array is decoded.
        if !store.search_batch_exists(batch_id)? {
            store.expire_agent_plan(plan_id)?;
            return Err(ExecutionError::Plan(PlanError::Expired));
        }
    }
    Ok(tool)
}

pub struct FileEvidence {
    pub bytes: u64,
    pub sha256: String,
}
pub fn check_file(
    path: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<FileEvidence, ExecutionError> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() != expected_bytes {
        return Err(ExecutionError::Unverified);
    }
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    let mut bytes = 0u64;
    loop {
        let count = file.read(&mut buf)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .ok_or(ExecutionError::Unverified)?;
        hasher.update(&buf[..count]);
    }
    let sha256 = format!("{:x}", hasher.finalize());
    if bytes != expected_bytes || sha256 != expected_sha256 {
        return Err(ExecutionError::Unverified);
    }
    Ok(FileEvidence { bytes, sha256 })
}
/// Completion is never inferred from model text or a trace-only claim.
pub fn verify_task_file(
    store: &TaskStore,
    task_id: &str,
    path: &Path,
    bytes: u64,
    sha256: &str,
) -> Result<FileEvidence, ExecutionError> {
    let task = store
        .load_task(task_id)?
        .ok_or(ExecutionError::Unverified)?;
    if serde_json::to_value(&task.runtime.status).map_err(RunError::Json)? != "completed" {
        return Err(ExecutionError::Unverified);
    }
    let stored_result = store.load_download_snapshot(task_id)?;
    if !stored_result.results.iter().any(|result| {
        result.destination == path
            && result.bytes_written == bytes
            && serde_json::to_value(&result.outcome).ok().as_ref() == Some(&json!("completed"))
    }) {
        return Err(ExecutionError::Unverified);
    }
    let evidence = check_file(path, bytes, sha256)?;
    if let Some(ledger) = store.current_landed_artifact(&path.to_string_lossy())? {
        if ledger.source_task_id.as_deref() != Some(task_id)
            || ledger.size_bytes != bytes
            || ledger
                .sha256
                .as_deref()
                .is_some_and(|stored| stored != sha256)
        {
            return Err(ExecutionError::Unverified);
        }
    }
    Ok(evidence)
}
