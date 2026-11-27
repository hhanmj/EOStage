use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const METADATA_KEY: &str = "nisar_archived_recovery_v2";
pub(crate) const SCHEMA_SQL: &str = r#"
CREATE TABLE nisar_archived_recovery_v2 (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES task_snapshots(task_id) ON DELETE RESTRICT,
    source_task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE RESTRICT,
    source_item_id TEXT NOT NULL,
    proof_sha256 TEXT NOT NULL CHECK(length(proof_sha256) = 64),
    canonical_json TEXT NOT NULL CHECK(length(canonical_json) BETWEEN 2 AND 41943040),
    UNIQUE(source_task_id, source_item_id)
) STRICT;
CREATE TRIGGER nisar_archived_recovery_is_immutable
BEFORE UPDATE ON nisar_archived_recovery_v2
BEGIN SELECT RAISE(ABORT, 'NISAR recovery provenance is immutable'); END;
CREATE TRIGGER nisar_archived_recovery_cannot_be_deleted
BEFORE DELETE ON nisar_archived_recovery_v2
BEGIN SELECT RAISE(ABORT, 'NISAR recovery provenance must be retained'); END;
"#;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StoreCreateNisarArchivedRecoveryV2Request {
    pub source_task_id: String,
    pub source_item_id: String,
    pub expected_source_revision: u64,
    pub new_task_id: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StoredNisarArchivedRecoveryV2 {
    pub schema_version: u32,
    pub new_task_id: String,
    pub source_snapshot: FrozenTaskSnapshot,
    pub source_item_id: String,
    pub source_revision: u64,
    pub source_anchor: StoredNisarSubsetExecutionAnchorV2,
    pub proof_sha256: String,
}

impl StoredNisarArchivedRecoveryV2 {
    fn digest(&self) -> Result<String, StoreError> {
        let bytes = serde_json::to_vec(&(
            "NisarArchivedRecoveryV2",
            self.schema_version,
            &self.new_task_id,
            &self.source_snapshot,
            &self.source_item_id,
            self.source_revision,
            &self.source_anchor,
        ))
        .map_err(StoreError::Corrupt)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    fn marker(&self) -> Value {
        json!({"schema_version": 1, "source_task_id": self.source_snapshot.task_id,
            "source_item_id": self.source_item_id, "proof_sha256": self.proof_sha256})
    }

    pub(super) fn validate_destination_item(&self, item: &SnapshotItem) -> Result<(), StoreError> {
        if self.schema_version != 1 || self.proof_sha256 != self.digest()? {
            return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
        }
        let original = self
            .source_snapshot
            .candidates
            .iter()
            .find(|v| v.item_id == self.source_item_id)
            .ok_or(StoreError::NotFound)?;
        let mut expected = original.clone();
        expected
            .metadata
            .insert(METADATA_KEY.to_owned(), self.marker());
        if item != &expected {
            return Err(StoreError::NisarSubsetExecutionPlanInvalid);
        }
        Ok(())
    }
}

fn source_authority(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
    revision: u64,
) -> Result<(StoredTask, StoredNisarSubsetExecutionAnchorV2), StoreError> {
    if row_exists(
        connection,
        "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id=?1)",
        task_id,
    )? || !row_exists(
        connection,
        "SELECT EXISTS(SELECT 1 FROM task_history WHERE task_id=?1)",
        task_id,
    )? {
        return Err(StoreError::NotFound);
    }
    let source = load_task_connection(connection, task_id)?.ok_or(StoreError::NotFound)?;
    if source.runtime.revision != revision {
        return Err(StoreError::RevisionConflict);
    }
    if source.runtime.status != TaskLifecycleStatus::Failed
        || !source.runtime.active_item_ids.is_empty()
        || source.runtime.failed_item_ids != [item_id]
        || source.snapshot.candidates.len() != 1
        || source.snapshot.operation.is_some()
        || source.snapshot.orbit_association.is_some()
    {
        return Err(StoreError::NisarArchivedRecoverySourceIneligible);
    }
    let item = source
        .snapshot
        .candidates
        .iter()
        .find(|v| v.item_id == item_id)
        .ok_or(StoreError::NotFound)?;
    if item.metadata.contains_key(METADATA_KEY) {
        return Err(StoreError::invalid(
            "nested archived recovery is not supported",
        ));
    }
    crate::nisar_subset_plan::parse_nisar_subset_prepared_intent_v2_envelope(&item.metadata)?
        .ok_or(StoreError::NisarSubsetPreparedIntentMissing)?;
    crate::nisar_subset_plan::parse_nisar_subset_plan_v2_envelope(&item.metadata)?
        .ok_or(StoreError::NisarSubsetExecutionPlanInvalid)?;
    let failure: Option<String> = connection.query_row(
        "SELECT json_extract(result_json, '$.failure.code') FROM download_results WHERE task_id=?1 AND item_id=?2 AND outcome='failed'",
        params![task_id,item_id], |row| row.get(0),
    ).optional()?.flatten();
    if !matches!(
        failure.as_deref(),
        Some("NISAR_SUBSET_V2_AUTH_REFRESH_FAILED" | "DL004")
    ) {
        return Err(StoreError::NisarArchivedRecoverySourceIneligible);
    }
    let writer: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM nisar_subset_execution_anchors_v2 WHERE task_id=?1 AND item_id=?2 AND anchor_kind=?3)",
        params![task_id,item_id,NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND], |r| r.get(0),
    )?;
    if writer {
        return Err(StoreError::invalid(
            "published or pending writer cannot be inherited",
        ));
    }
    let anchor = connection.query_row(
        "SELECT lineage_key_sha256,record_sha256,canonical_json FROM nisar_subset_execution_anchors_v2 WHERE task_id=?1 AND item_id=?2 AND anchor_kind=?3",
        params![task_id,item_id,NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND],
        |r| Ok(StoredNisarSubsetExecutionAnchorV2 { task_id:task_id.to_owned(),item_id:item_id.to_owned(),
            anchor_kind:NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND.to_owned(),
            lineage_key_sha256:r.get(0)?,record_sha256:r.get(1)?,canonical_json:r.get(2)? }),
    ).optional()?.ok_or(StoreError::NisarSubsetExecutionAnchorInvalid)?;
    validate_nisar_subset_execution_anchor_material(
        &anchor.lineage_key_sha256,
        Some(&anchor.record_sha256),
        &anchor.record_sha256,
        &anchor.canonical_json,
    )?;
    let value: Value = serde_json::from_str(&anchor.canonical_json).map_err(StoreError::Corrupt)?;
    if value.get("pending_transition") != Some(&Value::Null)
        || value
            .get("committed_checkpoint_sha256")
            .and_then(Value::as_str)
            .is_none_or(|v| v.len() != 64)
        || value.get("lineage_key_sha256").and_then(Value::as_str)
            != Some(anchor.lineage_key_sha256.as_str())
        || value.get("record_sha256").and_then(Value::as_str) != Some(anchor.record_sha256.as_str())
    {
        return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
    }
    Ok((source, anchor))
}

pub(crate) fn validate_recovery_receipt_target(
    connection: &rusqlite::Connection,
    request: &StoreCreateNisarArchivedRecoveryV2Request,
) -> Result<StoredTask, StoreError> {
    if row_exists(
        connection,
        "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id=?1)",
        &request.new_task_id,
    )? {
        return Err(StoreError::NotFound);
    }
    let existing =
        load_task_connection(connection, &request.new_task_id)?.ok_or(StoreError::NotFound)?;
    let (canonical, digest): (String, String) = connection.query_row(
        "SELECT canonical_json, proof_sha256 FROM nisar_archived_recovery_v2 WHERE task_id=?1 AND source_task_id=?2 AND source_item_id=?3",
        params![request.new_task_id, request.source_task_id, request.source_item_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?.ok_or(StoreError::NisarSubsetExecutionAnchorInvalid)?;
    let proof: StoredNisarArchivedRecoveryV2 =
        serde_json::from_str(&canonical).map_err(StoreError::Corrupt)?;
    if proof.new_task_id != request.new_task_id
        || proof.source_snapshot.task_id != request.source_task_id
        || proof.source_item_id != request.source_item_id
        || proof.source_revision != request.expected_source_revision
        || proof.proof_sha256 != digest
        || serde_json::to_string(&proof).map_err(StoreError::Corrupt)? != canonical
    {
        return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
    }
    proof.validate_destination_item(
        existing
            .snapshot
            .candidates
            .first()
            .ok_or(StoreError::NotFound)?,
    )?;
    Ok(existing)
}

impl TaskStore {
    pub fn nisar_archived_recovery_receipt_status(
        &self,
        request: &StoreCreateNisarArchivedRecoveryV2Request,
    ) -> Result<crate::NisarArchivedRecoveryReceiptStatus, StoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let status = crate::source_receipts::nisar_archived_recovery_status(&transaction, request)?;
        if status == crate::NisarArchivedRecoveryReceiptStatus::Committed {
            validate_recovery_receipt_target(&transaction, request)?;
        }
        transaction.commit()?;
        Ok(status)
    }
    pub fn inspect_nisar_archived_recovery_source_v2(
        &self,
        task_id: &str,
    ) -> Result<StoredTask, StoreError> {
        let source =
            load_task_connection(&self.connection, task_id)?.ok_or(StoreError::NotFound)?;
        let item = source
            .snapshot
            .candidates
            .first()
            .ok_or(StoreError::NotFound)?;
        source_authority(
            &self.connection,
            task_id,
            &item.item_id,
            source.runtime.revision,
        )
        .map(|(task, _)| task)
    }

    pub fn create_nisar_archived_recovery_v2(
        &mut self,
        request: &StoreCreateNisarArchivedRecoveryV2Request,
    ) -> Result<StoredTask, StoreError> {
        if request.new_task_id == request.source_task_id
            || request.source_task_id.trim().is_empty()
            || request.source_item_id.trim().is_empty()
            || request.created_at.trim().is_empty()
            || request.new_task_id.is_empty()
            || request.new_task_id.len() > 128
            || !request
                .new_task_id
                .bytes()
                .all(|v| v.is_ascii_alphanumeric() || v == b'-' || v == b'_')
        {
            return Err(StoreError::invalid("invalid recovery task identity"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        match crate::source_receipts::nisar_archived_recovery_status(&transaction, request)? {
            crate::NisarArchivedRecoveryReceiptStatus::Committed => {
                let existing = validate_recovery_receipt_target(&transaction, request)?;
                transaction.commit()?;
                return Ok(existing);
            }
            crate::NisarArchivedRecoveryReceiptStatus::Conflict => {
                return Err(StoreError::AlreadyExists)
            }
            crate::NisarArchivedRecoveryReceiptStatus::Unknown => {}
        }
        if row_exists(
            &transaction,
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id=?1)",
            &request.new_task_id,
        )? {
            return Err(StoreError::NotFound);
        }
        let (source, anchor) = source_authority(
            &transaction,
            &request.source_task_id,
            &request.source_item_id,
            request.expected_source_revision,
        )?;
        let mut proof = StoredNisarArchivedRecoveryV2 {
            schema_version: 1,
            new_task_id: request.new_task_id.clone(),
            source_snapshot: source.snapshot.clone(),
            source_item_id: request.source_item_id.clone(),
            source_revision: request.expected_source_revision,
            source_anchor: anchor,
            proof_sha256: String::new(),
        };
        proof.proof_sha256 = proof.digest()?;
        if transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM nisar_archived_recovery_v2 WHERE source_task_id=?1 AND source_item_id=?2)",
            params![request.source_task_id,request.source_item_id], |r| r.get::<_, bool>(0),
        )? {
            // The legacy proof does not bind created_at and cannot certify the
            // complete external request.
            return Err(StoreError::AlreadyExists);
        }
        if row_exists(
            &transaction,
            "SELECT EXISTS(SELECT 1 FROM task_snapshots WHERE task_id=?1)",
            &request.new_task_id,
        )? {
            return Err(StoreError::AlreadyExists);
        }
        let mut item = source.snapshot.candidates[0].clone();
        item.metadata
            .insert(METADATA_KEY.to_owned(), proof.marker());
        let source_ids = load_explicit_task_source_item_ids(&transaction, &request.source_task_id)?
            .ok_or(StoreError::NotFound)?;
        let create = StoreCreateTaskRequest {
            task_id: request.new_task_id.clone(),
            batch_id: source.snapshot.batch_id.clone(),
            selected_item_ids: vec![item.item_id.clone()],
            checked_item_ids: vec![item.item_id.clone()],
            page_index: 0,
            page_size: source.runtime.page_size,
            output_root: source.snapshot.output_root.join(&request.new_task_id),
            layout: source.snapshot.layout.clone(),
            concurrency: 1,
            created_at: request.created_at.clone(),
        };
        let items = vec![item];
        let stored = create_task_in_transaction_with_candidates_authorized(
            &transaction,
            &create,
            TaskCandidateMode::Explicit {
                source_item_ids: &source_ids,
                task_items: &items,
            },
            None,
            None,
            true,
            Some(&proof),
        )?;
        if stored.runtime.status != TaskLifecycleStatus::Paused
            || !stored.runtime.task_paused
            || !stored.runtime.queued_item_ids.is_empty()
            || stored.runtime.paused_item_ids.len() != stored.snapshot.candidates.len()
        {
            return Err(StoreError::invalid("NISAR recovery is not fully paused"));
        }
        transaction.execute(
            "INSERT INTO nisar_archived_recovery_v2(task_id,source_task_id,source_item_id,proof_sha256,canonical_json) VALUES(?1,?2,?3,?4,?5)",
            params![request.new_task_id,request.source_task_id,request.source_item_id,proof.proof_sha256,
                serde_json::to_string(&proof).map_err(StoreError::Corrupt)?],
        )?;
        crate::source_receipts::insert_nisar_archived_recovery_receipt(
            &transaction,
            request,
            stored.runtime.revision,
        )?;
        transaction.commit()?;
        Ok(stored)
    }

    pub fn load_nisar_archived_recovery_v2(
        &self,
        task_id: &str,
        item_id: &str,
    ) -> Result<Option<StoredNisarArchivedRecoveryV2>, StoreError> {
        if row_exists(
            &self.connection,
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id=?1)",
            task_id,
        )? {
            return Err(StoreError::NotFound);
        }
        let current =
            load_task_connection(&self.connection, task_id)?.ok_or(StoreError::NotFound)?;
        let item = current
            .snapshot
            .candidates
            .iter()
            .find(|v| v.item_id == item_id)
            .ok_or(StoreError::NotFound)?;
        let row=self.connection.query_row(
            "SELECT canonical_json,proof_sha256 FROM nisar_archived_recovery_v2 WHERE task_id=?1",
            [task_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)),
        ).optional()?;
        let Some((canonical, digest)) = row else {
            if item.metadata.contains_key(METADATA_KEY) {
                return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
            }
            return Ok(None);
        };
        let proof: StoredNisarArchivedRecoveryV2 =
            serde_json::from_str(&canonical).map_err(StoreError::Corrupt)?;
        if proof.new_task_id != task_id
            || proof.source_item_id != item_id
            || proof.proof_sha256 != digest
            || serde_json::to_string(&proof).map_err(StoreError::Corrupt)? != canonical
        {
            return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
        }
        proof.validate_destination_item(item)?;
        let (source, anchor) = source_authority(
            &self.connection,
            &proof.source_snapshot.task_id,
            item_id,
            proof.source_revision,
        )?;
        if source.snapshot != proof.source_snapshot
            || anchor != proof.source_anchor
            || current.snapshot.output_root != source.snapshot.output_root.join(task_id)
        {
            return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
        }
        Ok(Some(proof))
    }
}
