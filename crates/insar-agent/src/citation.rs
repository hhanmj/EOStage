//! Evidence references are verified against the current run and the one Store,
//! never against model prose or a detached, user-supplied list of identifiers.
use crate::run::{read_sealed, TraceEvent};
use insar_store::TaskStore;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reference {
    pub kind: String,
    pub id: String,
    pub sequence: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Claim {
    pub statement: String,
    pub category: String,
    pub refs: Vec<Reference>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    pub claims: Option<Vec<Claim>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckedClaim {
    pub statement: String,
    pub verified: bool,
    pub refs: Vec<Reference>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckedAnswer {
    pub text: String,
    pub unreferenced: bool,
    pub claims: Vec<CheckedClaim>,
}
#[derive(Debug, Error)]
pub enum CitationError {
    #[error(transparent)]
    Run(#[from] crate::run::RunError),
}

pub fn check_answer(
    root: &Path,
    store: &TaskStore,
    run_id: &str,
    answer: &Answer,
) -> Result<CheckedAnswer, CitationError> {
    let events = read_sealed(root, store, run_id)?;
    let Some(claims) = &answer.claims else {
        return Ok(CheckedAnswer {
            text: answer.text.clone(),
            unreferenced: true,
            claims: vec![],
        });
    };
    let checked = claims
        .iter()
        .map(|claim| CheckedClaim {
            statement: claim.statement.clone(),
            verified: !claim.refs.is_empty()
                && claim
                    .refs
                    .iter()
                    .all(|reference| valid_ref(store, &events, &claim.category, reference)),
            refs: claim.refs.clone(),
        })
        .collect();
    Ok(CheckedAnswer {
        text: answer.text.clone(),
        unreferenced: false,
        claims: checked,
    })
}
fn valid_ref(
    store: &TaskStore,
    events: &[TraceEvent],
    category: &str,
    reference: &Reference,
) -> bool {
    match reference.kind.as_str() {
        "tool_result" | "verification" => {
            let Some(event) = reference
                .sequence
                .and_then(|sequence| events.get(sequence as usize))
            else {
                return false;
            };
            if event.kind != reference.kind
                || event.payload.get("id").and_then(serde_json::Value::as_str)
                    != Some(reference.id.as_str())
            {
                return false;
            }
            if category == "completed" {
                return false;
            }
            if category == "verified" {
                return event.kind == "verification"
                    && event
                        .payload
                        .get("verified")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true);
            }
            true
        }
        "task_id" => {
            let Ok(Some(task)) = store.load_task_record(&reference.id) else {
                return false;
            };
            match category {
                "completed" => {
                    serde_json::to_value(&task.runtime.status).ok().as_ref()
                        == Some(&serde_json::json!("completed"))
                        && events.iter().any(|event| {
                            event.kind == "execution"
                                && event
                                    .payload
                                    .get("task_id")
                                    .and_then(serde_json::Value::as_str)
                                    == Some(reference.id.as_str())
                        })
                }
                "verified" => false,
                _ => true,
            }
        }
        "event_id" => {
            if matches!(category, "completed" | "verified") {
                return false;
            }
            // Event IDs are only valid for a task that was observed by this run.
            events
                .iter()
                .filter(|event| event.kind == "execution")
                .filter_map(|event| {
                    if event
                        .payload
                        .get("event_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(reference.id.as_str())
                    {
                        event
                            .payload
                            .get("task_id")
                            .and_then(serde_json::Value::as_str)
                    } else {
                        None
                    }
                })
                .any(|task_id| {
                    store
                        .list_task_events(task_id)
                        .is_ok_and(|rows| rows.iter().any(|event| event.event_id == reference.id))
                })
        }
        "search_batch_id" => {
            if matches!(category, "completed" | "verified") {
                return false;
            }
            // #22 B slice 2: an existence check must not decode the whole
            // frozen candidate array.
            store.search_batch_exists(&reference.id).unwrap_or(false)
                && events.iter().any(|event| {
                    event.kind == "tool_result"
                        && event
                            .payload
                            .get("summary")
                            .and_then(|summary| summary.get("search_batch_id"))
                            .and_then(serde_json::Value::as_str)
                            == Some(reference.id.as_str())
                })
        }
        "file_sha256" => {
            if category != "verified" {
                return false;
            }
            let Some(sequence) = reference.sequence else {
                return false;
            };
            events.get(sequence as usize).is_some_and(|event| {
                event.kind == "verification"
                    && event
                        .payload
                        .get("sha256")
                        .and_then(serde_json::Value::as_str)
                        == Some(reference.id.as_str())
                    && event
                        .payload
                        .get("verified")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
            })
        }
        _ => false,
    }
}
