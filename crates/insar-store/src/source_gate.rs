//! Inert foundation for durable source-operation staging.
//!
//! This is not yet a cross-window gate: the legacy import barrier cannot be
//! opened here, and existing write paths do not consult these tables. Keep the
//! staging API private until migration, write-path checks and old writers are
//! addressed. The gate SQL is a draft and is not installed by migration.
//! No request payload belongs in diagnostics.
#![allow(dead_code)]

use std::{
    collections::HashSet,
    io::{self, Write},
};

use insar_core::{
    AppendTaskFromSourceBatchRequest, CreateTaskFromHistoricalSourceBatchRequest, DownloadLayout,
    StoreUpdateTaskRequest, TaskStateCommandKind,
};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    StoreBurstAppendIdentity, StoreBurstSuccessorIdentity,
    StoreCreateNisarArchivedRecoveryV2Request, StoreError, StoreNisarAppendIdentity,
    StoreNisarSuccessorIdentity, TaskStore,
};

pub(crate) const SCHEMA_SOURCE_GATE_DRAFT_SQL: &str = r#"
CREATE TABLE source_gate_import_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    state TEXT NOT NULL CHECK(state IN ('pending', 'ready'))
) STRICT;
INSERT INTO source_gate_import_state(singleton, state) VALUES (1, 'pending');
-- No code may open the barrier until the legacy-import protocol is added in a
-- later migration. A lone SQL update must not imply that import completed.
CREATE TRIGGER source_gate_import_no_delete BEFORE DELETE ON source_gate_import_state
BEGIN SELECT RAISE(ABORT, 'source gate import barrier must be retained'); END;
CREATE TRIGGER source_gate_import_no_replace BEFORE INSERT ON source_gate_import_state
WHEN EXISTS(SELECT 1 FROM source_gate_import_state WHERE singleton = NEW.singleton)
BEGIN SELECT RAISE(ABORT, 'source gate import barrier already exists'); END;
CREATE TRIGGER source_gate_import_remains_closed BEFORE UPDATE ON source_gate_import_state
BEGIN SELECT RAISE(ABORT, 'source gate import protocol is not enabled'); END;
CREATE TABLE source_gate_sources (
    source_task_id TEXT PRIMARY KEY NOT NULL CHECK(length(source_task_id) > 0),
    generation INTEGER NOT NULL CHECK(generation >= 0)
) STRICT;
-- Import candidates deliberately have no uniqueness constraint on source or
-- operation key: conflicting legacy observations must remain distinguishable.
-- A failed or oversized import must leave the original client record intact.
CREATE TABLE source_gate_import_candidates (
    candidate_id INTEGER PRIMARY KEY CHECK(candidate_id > 0),
    origin TEXT NOT NULL,
    origin_key TEXT NOT NULL,
    source_task_id TEXT,
    raw_record BLOB NOT NULL,
    disposition TEXT NOT NULL CHECK(disposition IN ('unresolved', 'quarantined', 'imported'))
) STRICT;
CREATE TRIGGER source_gate_candidates_no_delete BEFORE DELETE ON source_gate_import_candidates
BEGIN SELECT RAISE(ABORT, 'source gate import candidates must be retained'); END;
CREATE TRIGGER source_gate_candidates_no_update BEFORE UPDATE ON source_gate_import_candidates
BEGIN SELECT RAISE(ABORT, 'source gate import protocol is not enabled'); END;
CREATE TRIGGER source_gate_candidates_no_replace BEFORE INSERT ON source_gate_import_candidates
WHEN EXISTS(SELECT 1 FROM source_gate_import_candidates WHERE candidate_id = NEW.candidate_id)
BEGIN SELECT RAISE(ABORT, 'source gate import candidate already exists'); END;
CREATE TABLE source_gate_operations (
    key_namespace TEXT NOT NULL CHECK(key_namespace IN ('event', 'task')),
    operation_key TEXT NOT NULL CHECK(length(operation_key) > 0),
    source_task_id TEXT NOT NULL REFERENCES source_gate_sources(source_task_id) ON DELETE RESTRICT,
    generation INTEGER NOT NULL CHECK(generation > 0),
    request_version INTEGER NOT NULL CHECK(request_version = 1),
    request_json BLOB NOT NULL CHECK(length(request_json) BETWEEN 2 AND 16777216),
    salt BLOB NOT NULL CHECK(length(salt) = 32),
    identity_sha256 BLOB NOT NULL CHECK(length(identity_sha256) = 32),
    state TEXT NOT NULL CHECK(state IN ('pending', 'quarantined', 'committed', 'sealed')),
    PRIMARY KEY(key_namespace, operation_key)
) STRICT;
CREATE UNIQUE INDEX source_gate_one_blocking_operation_per_source
ON source_gate_operations(source_task_id) WHERE state IN ('pending', 'quarantined');
CREATE TRIGGER source_gate_sources_no_delete BEFORE DELETE ON source_gate_sources
BEGIN SELECT RAISE(ABORT, 'source gate generations must be retained'); END;
CREATE TRIGGER source_gate_sources_no_replace BEFORE INSERT ON source_gate_sources
WHEN EXISTS(SELECT 1 FROM source_gate_sources WHERE source_task_id = NEW.source_task_id)
BEGIN SELECT RAISE(ABORT, 'source gate generation already exists'); END;
CREATE TRIGGER source_gate_sources_monotonic BEFORE UPDATE ON source_gate_sources
WHEN OLD.source_task_id IS NOT NEW.source_task_id OR NEW.generation != OLD.generation + 1
BEGIN SELECT RAISE(ABORT, 'source gate generation must advance'); END;
CREATE TRIGGER source_gate_operations_no_delete BEFORE DELETE ON source_gate_operations
BEGIN SELECT RAISE(ABORT, 'source gate identities must be retained'); END;
CREATE TRIGGER source_gate_operations_no_state_change BEFORE UPDATE OF state ON source_gate_operations
WHEN OLD.state IS NOT NEW.state
BEGIN SELECT RAISE(ABORT, 'source gate resolution protocol is not enabled'); END;
CREATE TRIGGER source_gate_operations_pending_only BEFORE INSERT ON source_gate_operations
WHEN NEW.state <> 'pending'
BEGIN SELECT RAISE(ABORT, 'source gate resolution protocol is not enabled'); END;
CREATE TRIGGER source_gate_operations_no_replace BEFORE INSERT ON source_gate_operations
WHEN EXISTS(SELECT 1 FROM source_gate_operations WHERE key_namespace = NEW.key_namespace AND operation_key = NEW.operation_key)
  OR (NEW.state IN ('pending', 'quarantined') AND EXISTS(
      SELECT 1 FROM source_gate_operations WHERE source_task_id = NEW.source_task_id
      AND state IN ('pending', 'quarantined')))
BEGIN SELECT RAISE(ABORT, 'source gate identity already exists'); END;
CREATE TRIGGER source_gate_operations_immutable BEFORE UPDATE ON source_gate_operations
WHEN OLD.key_namespace IS NOT NEW.key_namespace OR OLD.operation_key IS NOT NEW.operation_key
  OR OLD.source_task_id IS NOT NEW.source_task_id OR OLD.generation IS NOT NEW.generation
  OR OLD.request_version IS NOT NEW.request_version OR OLD.request_json IS NOT NEW.request_json
  OR OLD.salt IS NOT NEW.salt OR OLD.identity_sha256 IS NOT NEW.identity_sha256
  OR (OLD.state IN ('committed', 'sealed') AND OLD.state IS NOT NEW.state)
BEGIN SELECT RAISE(ABORT, 'source gate identity is immutable'); END;
"#;

const REQUEST_VERSION: i64 = 1;
const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_KEY_BYTES: usize = 1024;

// No Debug: several variants contain a local output directory.
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum FrozenSourceOperation {
    SourceAppend(AppendTaskFromSourceBatchRequest),
    HistoricalSuccessor(CreateTaskFromHistoricalSourceBatchRequest),
    BurstAppend(StoreBurstAppendIdentity),
    BurstSuccessor(StoreBurstSuccessorIdentity),
    NisarAppend(StoreNisarAppendIdentity),
    NisarSuccessor(StoreNisarSuccessorIdentity),
    NisarArchivedRecovery(StoreCreateNisarArchivedRecoveryV2Request),
    Reacquire(StoreUpdateTaskRequest),
}

impl FrozenSourceOperation {
    fn identity(&self) -> (&str, &str, &str) {
        match self {
            Self::SourceAppend(r) => (&r.task_id, "event", &r.event_id),
            Self::HistoricalSuccessor(r) => (&r.source_task_id, "task", &r.new_task_id),
            Self::BurstAppend(r) => (&r.task_id, "event", &r.event_id),
            Self::BurstSuccessor(r) => (&r.source_task_id, "task", &r.new_task_id),
            Self::NisarAppend(r) => (&r.task_id, "event", &r.event_id),
            Self::NisarSuccessor(r) => (&r.source_task_id, "task", &r.new_task_id),
            Self::NisarArchivedRecovery(r) => (&r.source_task_id, "task", &r.new_task_id),
            Self::Reacquire(r) => (&r.task_id, "event", &r.event_id),
        }
    }

    fn validate_staging_identity(&self) -> Result<(), StoreError> {
        let (source, namespace, key) = self.identity();
        if !valid_text(source) || !valid_text(key) || (namespace == "task" && source == key) {
            return Err(StoreError::invalid("invalid source operation identity"));
        }
        // These are necessary structural checks, not a substitute for the
        // application service's per-operation admission and timestamp checks.
        let valid = match self {
            Self::SourceAppend(r) => {
                r.validate().is_ok() && valid_members(&r.item_ids) && valid_text(&r.occurred_at)
            }
            Self::HistoricalSuccessor(r) => {
                r.validate().is_ok()
                    && valid_members(&r.item_ids)
                    && valid_optional_members(&r.reacquire_item_ids)
                    && members_subset(&r.reacquire_item_ids, &r.item_ids)
                    && !r.output_root.as_os_str().is_empty()
                    && valid_text(&r.created_at)
            }
            Self::BurstAppend(r) => {
                valid_text(&r.request_id)
                    && valid_members(&r.selected_product_ids)
                    && valid_text(&r.occurred_at)
            }
            Self::BurstSuccessor(r) => {
                valid_text(&r.request_id)
                    && valid_members(&r.selected_product_ids)
                    && !r.output_root.as_os_str().is_empty()
                    && r.layout == DownloadLayout::Direct
                    && (1..=8).contains(&r.concurrency)
                    && valid_text(&r.created_at)
            }
            Self::NisarAppend(r) => valid_members(&r.source_item_ids) && valid_text(&r.occurred_at),
            Self::NisarSuccessor(r) => {
                valid_members(&r.source_item_ids)
                    && !r.output_root.as_os_str().is_empty()
                    && (1..=8).contains(&r.concurrency)
                    && valid_text(&r.created_at)
            }
            Self::NisarArchivedRecovery(r) => {
                valid_text(&r.source_item_id) && valid_text(&r.created_at)
            }
            Self::Reacquire(r) => {
                r.validate().is_ok()
                    && r.command.kind == TaskStateCommandKind::ReacquireItems
                    && valid_members(&r.command.item_ids)
                    && valid_text(&r.occurred_at)
            }
        };
        if !valid {
            return Err(StoreError::invalid("invalid source operation request"));
        }
        Ok(())
    }
}

fn valid_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= MAX_KEY_BYTES && !value.chars().any(char::is_control)
}

fn valid_optional_members(values: &[String]) -> bool {
    let mut unique = HashSet::new();
    values
        .iter()
        .all(|value| valid_text(value) && unique.insert(value.as_str()))
}

fn valid_members(values: &[String]) -> bool {
    !values.is_empty() && valid_optional_members(values)
}

fn members_subset(subset: &[String], values: &[String]) -> bool {
    let values: HashSet<&str> = values.iter().map(String::as_str).collect();
    subset.iter().all(|value| values.contains(value.as_str()))
}

struct BoundedBuffer(Vec<u8>);
impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_REQUEST_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "source operation too large",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode(request: &FrozenSourceOperation) -> Result<Vec<u8>, StoreError> {
    let mut output = BoundedBuffer(Vec::new());
    serde_json::to_writer(&mut output, request)
        .map_err(|_| StoreError::invalid("source operation cannot be staged"))?;
    Ok(output.0)
}

fn digest(salt: &[u8], request_json: &[u8]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"sceneport/source-gate-request/v1\0");
    hash.update(salt);
    hash.update(request_json);
    hash.finalize().to_vec()
}

fn check_import_ready(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let state: Option<String> = transaction
        .query_row(
            "SELECT state FROM source_gate_import_state WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if state.as_deref() != Some("ready") {
        return Err(StoreError::invalid("source operation import is not ready"));
    }
    Ok(())
}

/// The returned generation identifies the staged slot, not a committed write.
pub(crate) struct StagedSourceOperation {
    pub generation: i64,
}

impl TaskStore {
    /// Stage an exact typed identity under an IMMEDIATE lock. Not connected to
    /// IPC: legacy import remains closed and mutations do not consult it yet.
    pub(crate) fn stage_source_operation(
        &mut self,
        request: &FrozenSourceOperation,
    ) -> Result<StagedSourceOperation, StoreError> {
        let encoded = encode(request)?;
        request.validate_staging_identity()?;
        let (source, namespace, key) = request.identity();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_import_ready(&transaction)?;

        let source_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_snapshots WHERE task_id = ?1) \
             AND NOT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            [source],
            |row| row.get(0),
        )?;
        if !source_exists {
            return Err(StoreError::NotFound);
        }

        type StagedOperationRow = (String, i64, i64, Vec<u8>, Vec<u8>, Vec<u8>, String);
        let existing: Option<StagedOperationRow> = transaction.query_row(
            "SELECT source_task_id, generation, request_version, request_json, salt, identity_sha256, state \
             FROM source_gate_operations WHERE key_namespace = ?1 AND operation_key = ?2",
            params![namespace, key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        ).optional()?;
        if let Some((stored_source, generation, version, payload, salt, expected, state)) = existing
        {
            if version != REQUEST_VERSION
                || salt.len() != 32
                || expected.len() != 32
                || digest(&salt, &payload) != expected
            {
                return Err(StoreError::invalid("source operation record is corrupt"));
            }
            let current_generation: Option<i64> = transaction
                .query_row(
                    "SELECT generation FROM source_gate_sources WHERE source_task_id = ?1",
                    [source],
                    |row| row.get(0),
                )
                .optional()?;
            if stored_source != source
                || state != "pending"
                || payload != encoded
                || current_generation != Some(generation)
            {
                return Err(StoreError::AlreadyExists);
            }
            transaction.commit()?;
            return Ok(StagedSourceOperation { generation });
        }

        let old_key_exists: bool = if namespace == "event" {
            transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM task_events WHERE event_id = ?1) OR EXISTS(\
                 SELECT 1 FROM source_mutation_receipts WHERE operation_key = ?1 AND operation IN \
                 ('source_batch_append', 'burst_asset_append', 'burst_asset_append_event', 'reacquire_items', 'nisar_subset_append_v2', 'nisar_subset_append_event_v2'))",
                [key],
                |row| row.get(0),
            )?
        } else {
            transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM task_snapshots WHERE task_id = ?1) OR \
                 EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1) OR EXISTS(\
                 SELECT 1 FROM source_mutation_receipts WHERE operation_key = ?1 AND operation IN \
                 ('historical_source_successor', 'burst_asset_successor', 'nisar_successor_v2', 'nisar_archived_recovery_v2'))",
                [key], |row| row.get(0),
            )?
        };
        if old_key_exists {
            return Err(StoreError::AlreadyExists);
        }
        let blocking: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM source_gate_operations WHERE source_task_id = ?1 \
             AND state IN ('pending', 'quarantined'))",
            [source],
            |row| row.get(0),
        )?;
        if blocking {
            return Err(StoreError::AlreadyExists);
        }

        transaction.execute(
            "INSERT INTO source_gate_sources(source_task_id, generation) \
             SELECT ?1, 0 WHERE NOT EXISTS(SELECT 1 FROM source_gate_sources WHERE source_task_id = ?1)", [source],
        )?;
        let generation: i64 = transaction.query_row(
            "SELECT generation FROM source_gate_sources WHERE source_task_id = ?1",
            [source],
            |row| row.get(0),
        )?;
        let generation = generation
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("source operation generation exhausted"))?;
        transaction.execute(
            "UPDATE source_gate_sources SET generation = ?2 WHERE source_task_id = ?1",
            params![source, generation],
        )?;
        let salt: Vec<u8> = transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
        let identity = digest(&salt, &encoded);
        transaction.execute(
            "INSERT INTO source_gate_operations(key_namespace, operation_key, source_task_id, generation, request_version, request_json, salt, identity_sha256, state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending')",
            params![namespace, key, source, generation, REQUEST_VERSION, encoded, salt, identity],
        )?;
        transaction.commit()?;
        Ok(StagedSourceOperation { generation })
    }
}
