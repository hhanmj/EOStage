use insar_core::StoreUpdateTaskViewRequest;
use rusqlite::{params, TransactionBehavior};

use crate::repository::{
    candidate_ids_from_task, load_task_connection, sqlite_integer, validate_id_subset, StoredTask,
};
use crate::{StoreError, TaskStore};

impl TaskStore {
    pub fn update_task_view(
        &mut self,
        request: &StoreUpdateTaskViewRequest,
    ) -> Result<StoredTask, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        if request.updated_at.trim().is_empty() {
            return Err(StoreError::invalid("task view timestamp is required"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !crate::core_restore::task_is_actionable(&transaction, &request.task_id)? {
            return Err(StoreError::NotFound);
        }
        let task =
            load_task_connection(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        validate_id_subset(
            &request.checked_item_ids,
            &candidate_ids_from_task(&task.snapshot)?,
            "task view checked IDs",
        )?;
        let checked_item_ids_json =
            serde_json::to_string(&request.checked_item_ids).map_err(StoreError::Corrupt)?;
        transaction.execute(
            "INSERT INTO task_view_state( \
                task_id, checked_item_ids_json, page_index, page_size, updated_at \
             ) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(task_id) DO UPDATE SET \
                checked_item_ids_json = excluded.checked_item_ids_json, \
                page_index = excluded.page_index, page_size = excluded.page_size, \
                updated_at = excluded.updated_at \
             WHERE excluded.updated_at >= task_view_state.updated_at",
            params![
                request.task_id,
                checked_item_ids_json,
                sqlite_integer(request.page_index, "task view page index")?,
                sqlite_integer(request.page_size, "task view page size")?,
                request.updated_at
            ],
        )?;
        let task =
            load_task_connection(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        transaction.commit()?;
        Ok(task)
    }
}
