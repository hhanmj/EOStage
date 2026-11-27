use crate::catalog;
use insar_store::{AgentApprovalRow, AgentPlanRow, StoreError, TaskStore};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("plan contains invalid intent: {0}")]
    Invalid(&'static str),
    #[error("plan or Store revision has expired")]
    Expired,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// serde_json's sorted map representation gives a stable recursive JSON
/// serialization, including nested maps; arrays retain their semantic order.
pub fn canonical_json(value: &Value) -> Result<String, PlanError> {
    if !value.is_object() {
        return Err(PlanError::Invalid("plan must be an object"));
    }
    serde_json::to_string(value).map_err(|_| PlanError::Invalid("JSON encoding"))
}

pub fn plan_hash(value: &Value) -> Result<String, PlanError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(canonical_json(value)?.as_bytes())
    ))
}

pub fn build_plan(mut row: AgentPlanRow, intent: &Value) -> Result<AgentPlanRow, PlanError> {
    let steps = intent
        .get("steps")
        .and_then(Value::as_array)
        .ok_or(PlanError::Invalid("steps required"))?;
    if steps.is_empty() {
        return Err(PlanError::Invalid("steps required"));
    }
    let mut max_tier = 0;
    for step in steps {
        let id = step
            .get("tool_id")
            .and_then(Value::as_str)
            .ok_or(PlanError::Invalid("each step needs a tool_id"))?;
        let tool = catalog::resolve(id).map_err(|_| PlanError::Invalid("tool not registered"))?;
        let tier = match tool.confirmation.as_str() {
            "none" => 0,
            "disclose" => 1,
            "plan_confirm" => 2,
            _ => return Err(PlanError::Invalid("tool tier denied")),
        };
        max_tier = max_tier.max(tier);
    }
    if row.max_tier < max_tier {
        return Err(PlanError::Invalid("plan tier below tool tier"));
    }
    row.max_tier = max_tier;
    row.plan_json = canonical_json(intent)?;
    row.plan_hash = plan_hash(intent)?;
    row.status = "proposed".into();
    Ok(row)
}

/// The four facts a desktop confirm card must show. `output_dir` is the
/// W4 card name for the same output fact earlier fixtures call `output`.
fn card_fields_present(snapshot: &Value) -> bool {
    ["source", "range", "estimated_bytes"]
        .iter()
        .all(|field| snapshot.get(field).is_some())
        && (snapshot.get("output").is_some() || snapshot.get("output_dir").is_some())
}

/// Fingerprint of the Store objects a plan's steps name (task revisions, batch
/// presence). A plan is only valid while this equals its `store_revision_seen`.
pub fn plan_revision(store: &TaskStore, intent: &Value) -> Result<i64, StoreError> {
    let mut hasher = Sha256::new();
    let mut bound = false;
    for step in intent
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let input = step.get("input").unwrap_or(&Value::Null);
        match step.get("tool_id").and_then(Value::as_str) {
            Some("task_action" | "update_download_task") => {
                let task_id = input.get("task_id").and_then(Value::as_str).unwrap_or("");
                let revision = store
                    .load_task_record(task_id)?
                    .map(|task| task.runtime.revision);
                hasher.update(format!("task:{task_id}:{revision:?};"));
                bound = true;
            }
            Some("create_task") => {
                let batch_id = input.get("batch_id").and_then(Value::as_str).unwrap_or("");
                // #22 B slice 2: existence only; no candidate array is decoded.
                let present = store.search_batch_exists(batch_id)?;
                hasher.update(format!("batch:{batch_id}:{present};"));
                bound = true;
            }
            _ => {}
        }
    }
    if !bound {
        return Ok(0);
    }
    let digest = hasher.finalize();
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest[..8]);
    Ok(i64::from_be_bytes(head) & i64::MAX)
}

pub fn approve(
    store: &mut TaskStore,
    plan_id: &str,
    approval: &AgentApprovalRow,
    current_revision: i64,
    now: &str,
) -> Result<(), PlanError> {
    let hash = shown_plan(store, plan_id, approval, current_revision, now)?;
    store.decide_agent_plan(approval, &hash, current_revision, now)?;
    Ok(())
}

/// The checks of `approve` on the plan the decider saw: the echoed card, the
/// hash, the Store revision and the expiry. Any mismatch expires the plan.
/// Returns the plan's hash.
fn shown_plan(
    store: &mut TaskStore,
    plan_id: &str,
    approval: &AgentApprovalRow,
    current_revision: i64,
    now: &str,
) -> Result<String, PlanError> {
    let row = store.agent_plan(plan_id)?.ok_or(StoreError::NotFound)?;
    let value: Value =
        serde_json::from_str(&row.plan_json).map_err(|_| PlanError::Invalid("stored plan JSON"))?;
    let hash = plan_hash(&value)?;
    let card_matches = if row.max_tier == 2 && approval.decision == "approved" {
        let snapshot: Value = serde_json::from_str(&approval.snapshot_json)
            .map_err(|_| PlanError::Invalid("card snapshot JSON"))?;
        value
            .get("card_snapshot")
            .is_some_and(|frozen| snapshot == *frozen && card_fields_present(&snapshot))
    } else {
        true
    };
    if !card_matches
        || hash != row.plan_hash
        || hash != approval.plan_hash_seen
        || row.store_revision_seen != current_revision
        || row.expires_at.as_str() <= now
        || approval.plan_id != plan_id
    {
        // Persist invalidation even if the canonical bytes were changed in place.
        store.expire_agent_plan(plan_id)?;
        return Err(PlanError::Expired);
    }
    Ok(hash)
}

/// Owner 2026-10-26: the desktop card may choose the output directory. Every
/// `create_task` step of the plan then writes into `dir`, and the frozen card
/// shows it with `output_dir_source = chosen_on_card`. Only a plan whose card
/// describes a `create_task` (its first tier 2 step) can be rewritten.
pub fn rewrite_output_dir(intent: &Value, dir: &str) -> Result<Value, PlanError> {
    let card_step = intent
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|step| step.get("tool_id").and_then(Value::as_str))
        .find(|id| catalog::resolve(id).is_ok_and(|tool| tool.confirmation == "plan_confirm"));
    if card_step != Some("create_task") || intent.get("card_snapshot").is_none() {
        return Err(PlanError::Invalid(
            "the card does not describe a create_task",
        ));
    }
    let mut rewritten = intent.clone();
    for step in rewritten
        .get_mut("steps")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if step.get("tool_id").and_then(Value::as_str) == Some("create_task") {
            step.pointer_mut("/input/options")
                .and_then(Value::as_object_mut)
                .ok_or(PlanError::Invalid("create_task options"))?
                .insert("output_root".into(), Value::String(dir.to_owned()));
        }
    }
    let card = rewritten
        .get_mut("card_snapshot")
        .and_then(Value::as_object_mut)
        .ok_or(PlanError::Invalid("card snapshot"))?;
    card.insert("output_dir".into(), Value::String(dir.to_owned()));
    card.insert(
        "output_dir_source".into(),
        Value::String("chosen_on_card".into()),
    );
    Ok(rewritten)
}

/// A `create_task` step that names no output directory: the card must
/// choose one before the plan can be approved.
pub fn missing_output_dir(intent: &Value) -> bool {
    intent
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|step| step.get("tool_id").and_then(Value::as_str) == Some("create_task"))
        .any(|step| {
            step.pointer("/input/options/output_root")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        })
}

/// `approve` for a card that chose another output directory: the echoed
/// card is checked against the plan the card showed, then that plan is
/// replaced by `rewritten` and approved in one Store transaction. The
/// approval stores the rewritten card and hash, and `rewritten_from_plan_hash`
/// = the shown plan's hash. Returns the rewritten plan's hash.
pub fn approve_rewritten(
    store: &mut TaskStore,
    plan_id: &str,
    approval: &AgentApprovalRow,
    rewritten: &Value,
    current_revision: i64,
    now: &str,
) -> Result<String, PlanError> {
    let hash = shown_plan(store, plan_id, approval, current_revision, now)?;
    let json = canonical_json(rewritten)?;
    let rewritten_hash = plan_hash(rewritten)?;
    let card = rewritten
        .get("card_snapshot")
        .ok_or(PlanError::Invalid("card snapshot"))?;
    let decided = AgentApprovalRow {
        snapshot_json: serde_json::to_string(card)
            .map_err(|_| PlanError::Invalid("card snapshot JSON"))?,
        plan_hash_seen: rewritten_hash.clone(),
        rewritten_from_plan_hash: Some(hash.clone()),
        ..approval.clone()
    };
    store.decide_rewritten_agent_plan(&decided, &hash, &json, current_revision, now)?;
    Ok(rewritten_hash)
}

pub fn require_approved(
    store: &mut TaskStore,
    plan_id: &str,
    current_revision: i64,
    now: &str,
) -> Result<AgentPlanRow, PlanError> {
    let row = store.agent_plan(plan_id)?.ok_or(StoreError::NotFound)?;
    let value: Value =
        serde_json::from_str(&row.plan_json).map_err(|_| PlanError::Invalid("stored plan JSON"))?;
    let approved = store.agent_approval(plan_id)?;
    if row.status != "approved"
        || plan_hash(&value)? != row.plan_hash
        || row.store_revision_seen != current_revision
        || row.expires_at.as_str() <= now
        || !approved.as_ref().is_some_and(|a| {
            let card_matches = row.max_tier != 2
                || serde_json::from_str::<Value>(&a.snapshot_json)
                    .ok()
                    .zip(value.get("card_snapshot"))
                    .is_some_and(|(snapshot, frozen)| {
                        snapshot == *frozen && card_fields_present(&snapshot)
                    });
            a.decision == "approved"
                // A click on the desktop card, or the user's permission mode
                // (the Store admits the latter only for in-app runs).
                && matches!(a.decided_via.as_str(), "desktop_card" | "auto_by_mode")
                && a.plan_hash_seen == row.plan_hash
                && card_matches
        })
    {
        store.expire_agent_plan(plan_id)?;
        return Err(PlanError::Expired);
    }
    Ok(row)
}
