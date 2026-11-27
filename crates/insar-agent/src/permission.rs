//! Permission modes (Owner 2026-10-25, simplified by the Owner 2026-10-26,
//! A1 design §3.1): the user decides whether tier 1 and tier 2 plans need a
//! click on the desktop card. A mode never adds a tool, never opens the
//! forbidden tier, carries no limits and no directory, and is only set by the
//! user through the desktop commands; the model has no way to read or change it.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Every tier 2 plan waits for the desktop card (new users).
    #[default]
    ConfirmChanges,
    /// Tier 2 plans whose facts are complete and that touch only what Agent
    /// runs made run without a click; the rest wait for the card.
    AutoApprove,
    /// Everything runs without a click, except a create_task whose output
    /// directory is unknown or was suggested by the model, and a tier 1/2
    /// tool the gate has no facts for.
    FullAccess,
}

impl PermissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmChanges => "confirm_changes",
            Self::AutoApprove => "auto_approve",
            Self::FullAccess => "full_access",
        }
    }

    /// A mode as stored by this or an earlier build: the current string, or
    /// the W3e object `{"kind": "per_item" | "within_quota" | "full_access",
    /// ...}` (`per_item` → `confirm_changes`, `within_quota` →
    /// `auto_approve`; its limits are dropped). Anything else is `None`.
    pub fn from_stored(value: &Value) -> Option<Self> {
        let name = match value {
            Value::String(name) => name.as_str(),
            Value::Object(fields) => fields.get("kind").and_then(Value::as_str)?,
            _ => return None,
        };
        match name {
            "confirm_changes" | "per_item" => Some(Self::ConfirmChanges),
            "auto_approve" | "within_quota" => Some(Self::AutoApprove),
            "full_access" => Some(Self::FullAccess),
            _ => None,
        }
    }
}

/// `session.json` reader for `permission_mode`: a stored value this build
/// cannot name decides `confirm_changes` (fail closed) instead of making the
/// whole session unreadable or following the global default.
pub fn stored_session_mode<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PermissionMode>, D::Error> {
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value
        .filter(|value| !value.is_null())
        .map(|value| PermissionMode::from_stored(&value).unwrap_or_default()))
}

/// An output directory a create_task may write to (review 2026-10-26): a
/// local absolute path of the host platform, trimmed, without control
/// characters, `.` or `..` segments, at most 1024 bytes.
///
/// On Windows that is a drive-letter path (`C:\...` or `C:/...`); UNC
/// (`\\server\share`), device and verbatim (`\\?\`, `\\.\`) paths are refused.
/// On Unix it is a `/...` path; `//` (implementation-defined), any backslash
/// and the `/dev`, `/proc` and `/sys` trees are refused. Anything relative is
/// refused on both.
pub fn valid_output_dir(path: &str) -> bool {
    #[cfg(windows)]
    let rooted = drive_letter_root(path);
    #[cfg(not(windows))]
    let rooted = valid_unix_dir(path) && !unix_pseudo_filesystem(path);
    rooted && clean_dir_text(path)
}

/// Kernel and device trees, the Unix counterpart of the Windows device and
/// verbatim namespaces (`\\.\`, `\\?\`) refused above.
#[cfg(not(windows))]
fn unix_pseudo_filesystem(path: &str) -> bool {
    let first = path.trim_start_matches('/').split('/').next().unwrap_or("");
    matches!(first, "dev" | "proc" | "sys")
}

/// The Unix rooted-directory grammar on every platform (review 2026-11-02):
/// a `/...` path that is not `//` (implementation-defined) and holds no
/// backslash, `.` or `..` segment, control character or leading/trailing
/// space, at most 1024 bytes. `message_dir` recognises these in message
/// text the same way everywhere, and `valid_output_dir` then decides
/// whether one is usable on this host (on Unix it also refuses the
/// `/dev`, `/proc` and `/sys` trees).
pub fn valid_unix_dir(path: &str) -> bool {
    path.starts_with('/') && !path.starts_with("//") && !path.contains('\\') && clean_dir_text(path)
}

/// The drive-letter directory grammar on every platform: `message_dir`
/// recognises these in message text the same way everywhere, and
/// `valid_output_dir` then decides whether one is usable on this host.
pub fn valid_drive_letter_dir(path: &str) -> bool {
    drive_letter_root(path) && clean_dir_text(path)
}

fn drive_letter_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

fn clean_dir_text(path: &str) -> bool {
    path.trim() == path
        && path.len() <= 1024
        && !path.chars().any(char::is_control)
        && !path
            .split(['\\', '/'])
            .any(|segment| segment == "." || segment == "..")
}

/// Tier 1 tools the gate knows (disclosed, run in every mode).
pub const DISCLOSED_TOOLS: &[&str] = &["search_start", "working_aoi_set"];
/// Tier 2 tools the gate knows facts for. Any other tier 1/2 tool the
/// catalog may gain is `Step::Unmapped` and always waits for the card.
pub const CONFIRMED_TOOLS: &[&str] = &["create_task", "task_action", "update_download_task"];

/// `update_download_task` commands that only steer transfers (task- or
/// item-level pause / resume / retry / cancel). Everything else edits the
/// task ledger (mark_completed, mark_failed, mark_skipped, reacquire_items,
/// activate_items, ...).
pub const STEERING_COMMANDS: &[&str] = &[
    "pause_task",
    "resume_task",
    "retry_task",
    "cancel_task",
    "pause_items",
    "resume_items",
    "retry_items",
    "cancel_items",
];

/// A1 §3 tiers. `Forbidden` is any step whose tool is not in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Read,
    Disclose,
    Confirm,
    Forbidden,
}
impl Tier {
    pub fn of_confirmation(confirmation: &str) -> Self {
        match confirmation {
            "none" => Self::Read,
            "disclose" => Self::Disclose,
            "plan_confirm" => Self::Confirm,
            _ => Self::Forbidden,
        }
    }
    /// Tier of one model-named step: unregistered tools are forbidden.
    pub fn of_tool(tool_id: &str) -> Self {
        crate::catalog::resolve(tool_id)
            .map(|tool| Self::of_confirmation(&tool.confirmation))
            .unwrap_or(Self::Forbidden)
    }
}

/// Why a plan cannot be approved by `auto_approve` (coordinator matrix and
/// review 2026-10-26). Declaration order is the order shown on the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardReason {
    /// No directory: neither the default download directory nor the plan
    /// names one (or the plan names the message's directory and the message
    /// named none); the card must choose one in every mode.
    OutputDirUnknown,
    /// The directory came from the model (`model_suggested`); the host
    /// cannot tell it from the user's message, so the card confirms it in
    /// every mode.
    OutputDirUnverified,
    /// A selected product has no recorded size.
    SizeUnknown,
    /// A selected product is already in a live task: create_task returns
    /// that task instead of creating one.
    ReopensExistingTask,
    /// An account the selection needs has no saved credential, or its
    /// service is unavailable.
    AccountNotReady,
    /// The batch was not frozen by a search this run started.
    SelectionNotFromRun,
    /// A task control names a task no Agent run created.
    TargetsUserTask,
    /// Archiving moves a task out of the queue.
    ArchiveRequiresConfirm,
    /// An `update_download_task` command that edits the task ledger (marks
    /// items completed / failed / skipped, reacquires or activates items).
    LedgerEditRequiresConfirm,
    /// A tier 1/2 tool the gate has no facts for.
    ToolRequiresConfirm,
}

/// Where a create_task's output directory came from, as far as the gate is
/// concerned. The default download directory and a directory the host took
/// from the user's message (`from_message`, W3g) are known at the gate (a
/// directory chosen on the card exists only once the card approves).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputDir {
    Known,
    ModelSuggested,
    Unknown,
}

/// Host-computed facts of one plan step (from the frozen step, the Store, the
/// run record and the service; never from model text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// `search_start`: an external read, disclosed on the plan card.
    Search,
    /// `working_aoi_set`: a disclosed workspace AOI update, auto-executed in every mode.
    WorkspaceAoi,
    CreateTask {
        output_dir: OutputDir,
        size_known: bool,
        reopens_existing_task: bool,
        accounts_ready: bool,
        selection_from_run: bool,
    },
    /// `task_action` or `update_download_task`. `ledger_edit`: an
    /// `update_download_task` command outside `STEERING_COMMANDS`.
    TaskControl {
        archive: bool,
        agent_task: bool,
        ledger_edit: bool,
    },
    /// A tier 1/2 tool outside `DISCLOSED_TOOLS` and `CONFIRMED_TOOLS`:
    /// the card in every mode.
    Unmapped,
}

impl Step {
    pub fn reasons(&self) -> Vec<CardReason> {
        let mut reasons = Vec::new();
        match *self {
            Self::Search | Self::WorkspaceAoi => {}
            Self::CreateTask {
                output_dir,
                size_known,
                reopens_existing_task,
                accounts_ready,
                selection_from_run,
            } => {
                for (missing, reason) in [
                    (
                        output_dir == OutputDir::Unknown,
                        CardReason::OutputDirUnknown,
                    ),
                    (
                        output_dir == OutputDir::ModelSuggested,
                        CardReason::OutputDirUnverified,
                    ),
                    (!size_known, CardReason::SizeUnknown),
                    (reopens_existing_task, CardReason::ReopensExistingTask),
                    (!accounts_ready, CardReason::AccountNotReady),
                    (!selection_from_run, CardReason::SelectionNotFromRun),
                ] {
                    if missing {
                        reasons.push(reason);
                    }
                }
            }
            Self::TaskControl {
                archive,
                agent_task,
                ledger_edit,
            } => {
                if !agent_task {
                    reasons.push(CardReason::TargetsUserTask);
                }
                if archive {
                    reasons.push(CardReason::ArchiveRequiresConfirm);
                }
                if ledger_edit {
                    reasons.push(CardReason::LedgerEditRequiresConfirm);
                }
            }
            Self::Unmapped => reasons.push(CardReason::ToolRequiresConfirm),
        }
        reasons
    }

    fn needs_card(&self, mode: PermissionMode) -> bool {
        match (self, mode) {
            (Self::Search | Self::WorkspaceAoi, _) => false,
            (Self::Unmapped, _) => true,
            // A directory the host cannot vouch for is confirmed on the card
            // in every mode.
            (
                Self::CreateTask {
                    output_dir: OutputDir::Unknown | OutputDir::ModelSuggested,
                    ..
                },
                _,
            ) => true,
            (
                Self::CreateTask { .. } | Self::TaskControl { .. },
                PermissionMode::ConfirmChanges,
            ) => true,
            (_, PermissionMode::AutoApprove) => !self.reasons().is_empty(),
            (Self::CreateTask { .. } | Self::TaskControl { .. }, PermissionMode::FullAccess) => {
                false
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Tier 0: runs without any card in every mode.
    Direct,
    /// The desktop card decides. `reasons` lists every fact of the plan that
    /// keeps it from approval by `auto_approve` (empty when there is none,
    /// e.g. a complete create_task under `confirm_changes`).
    Card { reasons: Vec<CardReason> },
    /// Approved by the mode; recorded as `auto_by_mode`.
    Auto,
    /// The forbidden tier: refused in every mode.
    Refuse,
}

/// The single confirmation gate. A plan waits for the card when any of its
/// steps needs one under `mode`.
pub fn decide(mode: PermissionMode, tier: Tier, steps: &[Step]) -> Verdict {
    match tier {
        Tier::Forbidden => Verdict::Refuse,
        Tier::Read => Verdict::Direct,
        Tier::Disclose | Tier::Confirm => {
            if steps.iter().any(|step| step.needs_card(mode)) {
                let mut reasons: Vec<CardReason> = steps.iter().flat_map(Step::reasons).collect();
                reasons.sort();
                reasons.dedup();
                Verdict::Card { reasons }
            } else {
                Verdict::Auto
            }
        }
    }
}

/// Scoped `AutoApprove` threshold: downloads of 20 GB or more require
/// explicit user confirmation on the card (`FullAccess` still auto-runs).
pub const AUTO_APPROVE_MAX_BYTES: u64 = 20 * 1_000_000_000;

/// Single confirmation gate with optional byte-cap enforcement for `AutoApprove`.
pub fn decide_with_cap(
    mode: PermissionMode,
    tier: Tier,
    steps: &[Step],
    estimated_bytes: Option<u64>,
) -> Verdict {
    let verdict = decide(mode, tier, steps);
    if matches!(verdict, Verdict::Auto)
        && mode == PermissionMode::AutoApprove
        && steps.iter().any(|s| matches!(s, Step::CreateTask { .. }))
        && estimated_bytes.is_some_and(|bytes| bytes >= AUTO_APPROVE_MAX_BYTES)
    {
        return Verdict::Card {
            reasons: vec![CardReason::ToolRequiresConfirm],
        };
    }
    verdict
}

/// `agent_permission_mode_get`: the mode the next decision in that session
/// uses, the session's own setting if any, and the global default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionModeView {
    pub effective: PermissionMode,
    pub session_override: Option<PermissionMode>,
    pub global_default: PermissionMode,
}

/// `agent_stop_all`: runs cancelled, Agent-created tasks cancelled, and
/// Agent-created tasks whose cancel the Store refused (e.g. a revision race).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopAllResult {
    pub cancelled_runs: Vec<String>,
    pub failed_runs: Vec<String>,
    pub cancelled_tasks: Vec<String>,
    pub failed_tasks: Vec<String>,
}
