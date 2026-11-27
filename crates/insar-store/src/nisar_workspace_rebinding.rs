//! Long-term audit of an identity-verified NISAR workspace rebinding (design
//! the workspace rebinding contract §4). The local
//! evidence file is removed with the workspace after success; this records
//! the first accepted rebinding as one task event. No schema change: one
//! task_events row and the revision step every task event takes; no runtime
//! membership changes. Paths stay in the local event payload; the bounded
//! event page never projects payloads.

use rusqlite::{params, TransactionBehavior};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::repository::{load_task_connection, sqlite_integer};
use crate::{StoreError, TaskStore};

pub const NISAR_WORKSPACE_REBINDING_EVENT_TYPE: &str = "nisar_workspace_rebinding";
const EVENT_CONTRACT: &str = "nisar_workspace_rebinding_event_v2";
const MAX_FIELD_CHARS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreNisarWorkspaceRebindingEventV2Request {
    pub task_id: String,
    pub item_id: String,
    pub recorded_path: String,
    pub current_path: String,
    pub identity: String,
    pub volume_file_system: Option<String>,
    pub planning_checkpoint_sha256: Option<String>,
    pub science_checkpoint_sha256: Option<String>,
    pub anchor_record_sha256: Option<String>,
    pub anchor_generation: Option<u64>,
    pub occurred_at: String,
}

impl StoreNisarWorkspaceRebindingEventV2Request {
    fn validate(&self) -> Result<(), StoreError> {
        let bounded = |value: &str| !value.is_empty() && value.chars().count() <= MAX_FIELD_CHARS;
        let optional = |value: &Option<String>| value.as_deref().is_none_or(bounded);
        if ![
            &self.task_id,
            &self.item_id,
            &self.recorded_path,
            &self.current_path,
            &self.identity,
            &self.occurred_at,
        ]
        .into_iter()
        .all(|value| bounded(value))
            || self.recorded_path == self.current_path
            || !optional(&self.volume_file_system)
            || !optional(&self.planning_checkpoint_sha256)
            || !optional(&self.science_checkpoint_sha256)
            || !optional(&self.anchor_record_sha256)
        {
            return Err(StoreError::invalid(
                "invalid NISAR workspace rebinding event",
            ));
        }
        Ok(())
    }

    /// One event per task, item, directory identity and path pair.
    fn event_id(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"NisarWorkspaceRebindingEventV2");
        for value in [
            &self.task_id,
            &self.item_id,
            &self.identity,
            &self.recorded_path,
            &self.current_path,
        ] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
        format!("nisar-workspace-rebinding:{:x}", digest.finalize())
    }
}

impl TaskStore {
    /// Records an accepted workspace rebinding once; returns whether a new
    /// event was written. A repeat for the same rebinding changes nothing.
    pub fn record_nisar_workspace_rebinding_event_v2(
        &mut self,
        request: &StoreNisarWorkspaceRebindingEventV2Request,
    ) -> Result<bool, StoreError> {
        request.validate()?;
        let event_id = request.event_id();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::runtime::reject_closed_task(&transaction, &request.task_id)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_events WHERE event_id = ?1)",
            [&event_id],
            |row| row.get(0),
        )?;
        if exists {
            transaction.commit()?;
            return Ok(false);
        }
        let mut task =
            load_task_connection(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        if !task
            .snapshot
            .candidates
            .iter()
            .any(|candidate| candidate.item_id == request.item_id)
        {
            return Err(StoreError::invalid(
                "workspace rebinding item is not part of the task",
            ));
        }
        let runtime = &mut task.runtime;
        let expected = runtime.revision;
        let revision = expected
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("task revision overflow"))?;
        runtime.revision = revision;
        if request.occurred_at > runtime.updated_at {
            runtime.updated_at = request.occurred_at.clone();
        }
        runtime.validate().map_err(StoreError::invalid)?;
        let runtime_json = serde_json::to_string(&*runtime).map_err(StoreError::Corrupt)?;
        let revision_sql = sqlite_integer(revision, "task revision")?;
        let changed = transaction.execute(
            "UPDATE task_runtime SET revision=?1, updated_at=?2, state_json=?3 WHERE task_id=?4 AND revision=?5",
            params![
                revision_sql,
                runtime.updated_at,
                runtime_json,
                request.task_id,
                sqlite_integer(expected, "expected task revision")?
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        let payload = json!({
            "contract": EVENT_CONTRACT,
            "item_id": request.item_id,
            "recorded_path": request.recorded_path,
            "current_path": request.current_path,
            "identity": request.identity,
            "volume_file_system": request.volume_file_system,
            "planning_checkpoint_sha256": request.planning_checkpoint_sha256,
            "science_checkpoint_sha256": request.science_checkpoint_sha256,
            "anchor_record_sha256": request.anchor_record_sha256,
            "anchor_generation": request.anchor_generation,
        });
        transaction.execute(
            "INSERT INTO task_events(event_id,task_id,sequence,occurred_at,event_type,item_id,payload_json,error_json) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL)",
            params![
                event_id,
                request.task_id,
                revision_sql,
                request.occurred_at,
                NISAR_WORKSPACE_REBINDING_EVENT_TYPE,
                request.item_id,
                payload.to_string()
            ],
        )?;
        transaction.commit()?;
        Ok(true)
    }
}
