use std::collections::HashMap;

use insar_core::{StoreUpdateTaskRequest, TaskPauseReason, TaskStateCommandKind};
use rusqlite::{params, TransactionBehavior};

use crate::{StoreError, StoredTask, TaskStore};

/// Payload key of the typed reason on an engine `pause_task` event.
pub const PAUSE_REASON_PAYLOAD_KEY: &str = "pause_reason";

/// The reason of each listed task that is paused at task level and whose
/// latest `pause_task` event carries one. A later reason-less pause (the
/// user's) replaces it; a resume clears `task_paused`.
const TASK_PAUSE_REASONS_SQL: &str = r#"
    WITH included(task_id) AS (SELECT value FROM json_each(?1)),
         latest AS (
           SELECT e.task_id, MAX(e.sequence) AS sequence
           FROM included i
           JOIN task_events e ON e.task_id = i.task_id
           WHERE e.event_type = 'pause_task'
           GROUP BY e.task_id
         )
    SELECT r.task_id, json_extract(e.payload_json, '$.pause_reason')
    FROM latest l
    JOIN task_runtime r ON r.task_id = l.task_id
    JOIN task_events e ON e.task_id = l.task_id AND e.sequence = l.sequence
    WHERE json_valid(r.state_json)
      AND json_extract(r.state_json, '$.task_paused') = 1
      AND json_valid(e.payload_json)
      AND json_type(e.payload_json, '$.pause_reason') = 'object'
"#;

impl TaskStore {
    /// Engine pause with a typed reason. The transition is the user's 暂停
    /// exactly (one IMMEDIATE transaction through the same reducer, revision
    /// bump and `pause_task` event); only the event payload additionally
    /// records `pause_reason`, so the reason survives a restart without a
    /// schema change.
    pub fn pause_task_with_reason(
        &mut self,
        request: &StoreUpdateTaskRequest,
        reason: &TaskPauseReason,
    ) -> Result<StoredTask, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        reason.validate().map_err(StoreError::invalid)?;
        if request.command.kind != TaskStateCommandKind::PauseTask
            || !request.command.item_ids.is_empty()
        {
            return Err(StoreError::invalid(
                "a pause reason is recorded only on a task-level pause",
            ));
        }
        if request.occurred_at.trim().is_empty() {
            return Err(StoreError::invalid("event timestamp is required"));
        }
        let mut payload = serde_json::to_value(request).map_err(StoreError::Corrupt)?;
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                PAUSE_REASON_PAYLOAD_KEY.to_owned(),
                serde_json::to_value(reason).map_err(StoreError::Corrupt)?,
            );
        }
        let request_json = serde_json::to_string(&payload).map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task =
            crate::runtime::update_task_in_transaction(&transaction, request, &request_json)?;
        transaction.commit()?;
        Ok(task)
    }

    /// Reasons for the listed tasks that have one; other tasks are absent. A
    /// reason that no longer parses is dropped for that task only.
    pub fn load_task_pause_reasons(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, TaskPauseReason>, StoreError> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let task_ids_json = serde_json::to_string(task_ids).map_err(StoreError::Corrupt)?;
        let mut statement = self.connection.prepare(TASK_PAUSE_REASONS_SQL)?;
        let rows = statement.query_map(params![task_ids_json], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut reasons = HashMap::new();
        for row in rows {
            let (task_id, raw) = row?;
            if let Some(reason) = raw
                .as_deref()
                .and_then(|raw| serde_json::from_str::<TaskPauseReason>(raw).ok())
                .filter(|reason| reason.validate().is_ok())
            {
                reasons.insert(task_id, reason);
            }
        }
        Ok(reasons)
    }
}
