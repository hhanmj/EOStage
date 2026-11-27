//! Downloader enhancement D1b (review of D1/D2, P2-3): a NISAR V2 writer that
//! has waited for its output longer than its threshold (5 minutes) records one
//! task event naming why it waits, so the task log tells a busy output slot
//! from another item, another program or an earlier build holding the same
//! output. No schema change: one task_events row and the revision step every
//! task event takes; no runtime membership changes. The holder's process id
//! stays in the local event payload; the bounded event page never projects
//! payloads.

use rusqlite::{params, TransactionBehavior};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::repository::{load_task_connection, sqlite_integer};
use crate::{StoreError, TaskStore};

const EVENT_CONTRACT: &str = "nisar_output_wait_event_v2";
const MAX_FIELD_CHARS: usize = 256;

/// Why a NISAR V2 writer waits for its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreNisarOutputWaitReasonV2 {
    /// Another item of this program writes the same artifact.
    SameArtifactInProcess,
    /// Another program holds the lock of the same artifact.
    SameArtifactOtherProcess,
    /// An earlier build writes into the output directory.
    EarlierBuild,
    /// Every output slot of this program is in use.
    OutputSlot,
}

impl StoreNisarOutputWaitReasonV2 {
    /// The task event type recorded for this reason.
    pub const fn event_type(self) -> &'static str {
        match self {
            Self::SameArtifactInProcess => "nisar_output_wait_same_artifact",
            Self::SameArtifactOtherProcess => "nisar_output_wait_other_process",
            Self::EarlierBuild => "nisar_output_wait_earlier_build",
            Self::OutputSlot => "nisar_output_wait_output_slot",
        }
    }

    const fn code(self) -> &'static str {
        match self {
            Self::SameArtifactInProcess => "same_artifact_in_process",
            Self::SameArtifactOtherProcess => "same_artifact_other_process",
            Self::EarlierBuild => "earlier_build",
            Self::OutputSlot => "output_slot",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreNisarOutputWaitEventV2Request {
    pub task_id: String,
    pub item_id: String,
    pub reason: StoreNisarOutputWaitReasonV2,
    /// The holder's process id when it could be read.
    pub holder_pid: Option<u32>,
    pub waited_seconds: u64,
    /// When this wait started: one event per wait.
    pub wait_started_at: String,
    pub occurred_at: String,
}

impl StoreNisarOutputWaitEventV2Request {
    fn validate(&self) -> Result<(), StoreError> {
        let bounded = |value: &str| !value.is_empty() && value.chars().count() <= MAX_FIELD_CHARS;
        if ![
            &self.task_id,
            &self.item_id,
            &self.wait_started_at,
            &self.occurred_at,
        ]
        .into_iter()
        .all(|value| bounded(value))
        {
            return Err(StoreError::invalid("invalid NISAR output wait event"));
        }
        Ok(())
    }

    /// One event per task, item and wait.
    fn event_id(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"NisarOutputWaitEventV2");
        for value in [&self.task_id, &self.item_id, &self.wait_started_at] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
        format!("nisar-output-wait:{:x}", digest.finalize())
    }
}

impl TaskStore {
    /// Records a long output wait once; returns whether a new event was
    /// written. A repeat for the same wait changes nothing.
    pub fn record_nisar_output_wait_event_v2(
        &mut self,
        request: &StoreNisarOutputWaitEventV2Request,
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
                "output wait item is not part of the task",
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
            "reason": request.reason.code(),
            "holder_pid": request.holder_pid,
            "waited_seconds": request.waited_seconds,
            "wait_started_at": request.wait_started_at,
        });
        transaction.execute(
            "INSERT INTO task_events(event_id,task_id,sequence,occurred_at,event_type,item_id,payload_json,error_json) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL)",
            params![
                event_id,
                request.task_id,
                revision_sql,
                request.occurred_at,
                request.reason.event_type(),
                request.item_id,
                payload.to_string()
            ],
        )?;
        transaction.commit()?;
        Ok(true)
    }
}
