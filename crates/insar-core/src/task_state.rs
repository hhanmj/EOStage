use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map};

use crate::{ContractError, ProviderResult};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskSelectionState {
    pub task_id: String,
    pub candidate_ids: BTreeSet<String>,
    pub selected_ids: BTreeSet<String>,
    pub active_ids: BTreeSet<String>,
    pub paused_ids: BTreeSet<String>,
    pub queued_ids: BTreeSet<String>,
    pub completed_ids: BTreeSet<String>,
    pub failed_ids: BTreeSet<String>,
    pub skipped_ids: BTreeSet<String>,
    pub cancelled_ids: BTreeSet<String>,
    pub task_paused: bool,
    pub concurrency: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskStateCommand {
    PauseTask,
    ResumeTask,
    ActivateItems {
        item_ids: BTreeSet<String>,
    },
    PauseItems {
        item_ids: BTreeSet<String>,
    },
    ResumeItems {
        item_ids: BTreeSet<String>,
    },
    RetryItems {
        item_ids: BTreeSet<String>,
    },
    MarkCompleted {
        item_id: String,
    },
    MarkFailed {
        item_id: String,
    },
    MarkSkipped {
        item_id: String,
    },
    CancelItems {
        item_ids: BTreeSet<String>,
    },
    /// Downloader D3: download skipped (reused) items again.
    ReacquireItems {
        item_ids: BTreeSet<String>,
    },
}

fn task_error(
    code: &str,
    user_message: &str,
    task_id: &str,
    item_id: Option<&str>,
) -> ContractError {
    let mut context = Map::from_iter([("task_id".to_string(), json!(task_id))]);
    if let Some(item_id) = item_id {
        context.insert("item_id".to_string(), json!(item_id));
    }
    ContractError::new(code, user_message, context)
        .expect("task-state error context contains only safe identifiers")
}

fn invalid_state(task_id: &str) -> ContractError {
    task_error("V3_INVALID_TASK_STATE", "任务状态无效", task_id, None)
}

fn unknown_item(task_id: &str, item_id: &str) -> ContractError {
    task_error(
        "V3_UNKNOWN_TASK_ITEM",
        "任务中不存在指定影像",
        task_id,
        Some(item_id),
    )
}

fn invalid_transition(task_id: &str, item_id: &str) -> ContractError {
    task_error(
        "V3_INVALID_TASK_TRANSITION",
        "任务状态转换无效",
        task_id,
        Some(item_id),
    )
}

fn lifecycle_sets(state: &TaskSelectionState) -> [&BTreeSet<String>; 7] {
    [
        &state.active_ids,
        &state.paused_ids,
        &state.queued_ids,
        &state.completed_ids,
        &state.failed_ids,
        &state.skipped_ids,
        &state.cancelled_ids,
    ]
}

pub fn validate_task_state(state: &TaskSelectionState) -> ProviderResult<()> {
    if state.task_id.trim().is_empty()
        || !state.selected_ids.is_subset(&state.candidate_ids)
        || state.active_ids.len() > state.concurrency
    {
        return Err(invalid_state(&state.task_id));
    }
    let sets = lifecycle_sets(state);
    if sets.iter().any(|set| !set.is_subset(&state.selected_ids)) {
        return Err(invalid_state(&state.task_id));
    }
    for (index, left) in sets.iter().enumerate() {
        if sets[index + 1..]
            .iter()
            .any(|right| !left.is_disjoint(right))
        {
            return Err(invalid_state(&state.task_id));
        }
    }
    Ok(())
}

fn validate_requested_ids(
    state: &TaskSelectionState,
    item_ids: &BTreeSet<String>,
) -> ProviderResult<()> {
    for item_id in item_ids {
        if !state.candidate_ids.contains(item_id) {
            return Err(unknown_item(&state.task_id, item_id));
        }
        if !state.selected_ids.contains(item_id) {
            return Err(invalid_transition(&state.task_id, item_id));
        }
    }
    Ok(())
}

fn is_terminal(state: &TaskSelectionState, item_id: &str) -> bool {
    state.completed_ids.contains(item_id)
        || state.failed_ids.contains(item_id)
        || state.skipped_ids.contains(item_id)
        || state.cancelled_ids.contains(item_id)
}

fn move_to_terminal(
    state: &mut TaskSelectionState,
    item_id: &str,
    terminal: fn(&mut TaskSelectionState) -> &mut BTreeSet<String>,
) -> ProviderResult<()> {
    validate_requested_ids(state, &BTreeSet::from([item_id.to_string()]))?;
    if is_terminal(state, item_id) {
        if terminal(state).contains(item_id) {
            return Ok(());
        }
        return Err(invalid_transition(&state.task_id, item_id));
    }
    state.active_ids.remove(item_id);
    state.paused_ids.remove(item_id);
    state.queued_ids.remove(item_id);
    terminal(state).insert(item_id.to_string());
    Ok(())
}

pub fn reduce_task_state(
    state: &TaskSelectionState,
    command: &TaskStateCommand,
) -> ProviderResult<TaskSelectionState> {
    validate_task_state(state)?;
    let mut next = state.clone();
    match command {
        TaskStateCommand::PauseTask => next.task_paused = true,
        TaskStateCommand::ResumeTask => next.task_paused = false,
        TaskStateCommand::ActivateItems { item_ids } => {
            validate_requested_ids(&next, item_ids)?;
            if next.task_paused {
                return Err(invalid_state(&next.task_id));
            }
            for item_id in item_ids {
                if !next.queued_ids.remove(item_id) {
                    return Err(invalid_transition(&next.task_id, item_id));
                }
                next.active_ids.insert(item_id.clone());
            }
        }
        TaskStateCommand::PauseItems { item_ids } => {
            validate_requested_ids(&next, item_ids)?;
            for item_id in item_ids {
                if is_terminal(&next, item_id) {
                    return Err(invalid_transition(&next.task_id, item_id));
                }
                next.active_ids.remove(item_id);
                next.queued_ids.remove(item_id);
                next.paused_ids.insert(item_id.clone());
            }
        }
        TaskStateCommand::ResumeItems { item_ids } => {
            validate_requested_ids(&next, item_ids)?;
            for item_id in item_ids {
                if is_terminal(&next, item_id) {
                    return Err(invalid_transition(&next.task_id, item_id));
                }
                if next.paused_ids.remove(item_id) {
                    next.queued_ids.insert(item_id.clone());
                }
            }
        }
        TaskStateCommand::RetryItems { item_ids } => {
            validate_requested_ids(&next, item_ids)?;
            for item_id in item_ids {
                if !next.failed_ids.remove(item_id) && !next.cancelled_ids.remove(item_id) {
                    return Err(invalid_transition(&next.task_id, item_id));
                }
                next.queued_ids.insert(item_id.clone());
            }
        }
        TaskStateCommand::MarkCompleted { item_id } => {
            move_to_terminal(&mut next, item_id, |state| &mut state.completed_ids)?;
        }
        TaskStateCommand::MarkFailed { item_id } => {
            move_to_terminal(&mut next, item_id, |state| &mut state.failed_ids)?;
        }
        TaskStateCommand::MarkSkipped { item_id } => {
            move_to_terminal(&mut next, item_id, |state| &mut state.skipped_ids)?;
        }
        TaskStateCommand::CancelItems { item_ids } => {
            validate_requested_ids(&next, item_ids)?;
            for item_id in item_ids {
                move_to_terminal(&mut next, item_id, |state| &mut state.cancelled_ids)?;
            }
        }
        TaskStateCommand::ReacquireItems { item_ids } => {
            validate_requested_ids(&next, item_ids)?;
            for item_id in item_ids {
                if !next.skipped_ids.remove(item_id) {
                    return Err(invalid_transition(&next.task_id, item_id));
                }
                next.queued_ids.insert(item_id.clone());
            }
        }
    }
    validate_task_state(&next)?;
    Ok(next)
}
