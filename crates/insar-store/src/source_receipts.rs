use std::path::PathBuf;

use insar_core::{
    AppendTaskFromSourceBatchRequest, CreateTaskFromHistoricalSourceBatchRequest, DownloadLayout,
    StoreUpdateTaskRequest,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{StoreError, TaskStore};

pub(crate) const SCHEMA_V42_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS source_mutation_receipts (
    operation TEXT NOT NULL,
    operation_key TEXT NOT NULL,
    identity_version INTEGER NOT NULL,
    salt BLOB NOT NULL CHECK(length(salt) = 32),
    identity_sha256 BLOB NOT NULL CHECK(length(identity_sha256) = 32),
    committed_revision INTEGER NOT NULL CHECK(committed_revision >= 0),
    PRIMARY KEY(operation, operation_key)
) STRICT;
"#;

// Reserve the event IDs and known task/revision slots of receipt-backed
// operations after event pruning. The event is inserted before its receipt
// and slot in the same transaction.
pub(crate) const SCHEMA_V43_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS source_receipt_schema_marker (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    protocol INTEGER NOT NULL CHECK(protocol = 2)
) STRICT;
INSERT OR IGNORE INTO source_receipt_schema_marker(singleton, protocol) VALUES (1, 2);
CREATE TABLE IF NOT EXISTS source_receipt_event_slots (
    event_id TEXT NOT NULL PRIMARY KEY,
    task_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    UNIQUE(task_id, sequence)
) STRICT, WITHOUT ROWID;
-- Reserve observed collisions conservatively, not as proof of old identity.
-- Existing receipts whose event is already gone cannot be reconstructed here.
INSERT OR IGNORE INTO source_receipt_event_slots(event_id, task_id, sequence)
SELECT DISTINCT e.event_id, e.task_id, e.sequence FROM task_events e
JOIN source_mutation_receipts r ON r.operation_key = e.event_id
WHERE r.operation IN ('source_batch_append', 'burst_asset_append', 'burst_asset_append_event', 'reacquire_items', 'nisar_subset_append_v2', 'nisar_subset_append_event_v2')
  AND NOT EXISTS (
      SELECT 1 FROM source_receipt_event_slots s
      WHERE s.event_id = e.event_id OR (s.task_id = e.task_id AND s.sequence = e.sequence)
  );
DROP TRIGGER IF EXISTS source_receipt_slot_no_replace;
CREATE TRIGGER source_receipt_slot_no_replace BEFORE INSERT ON source_receipt_event_slots
WHEN EXISTS(SELECT 1 FROM source_receipt_event_slots
            WHERE event_id = NEW.event_id OR (task_id = NEW.task_id AND sequence = NEW.sequence))
BEGIN SELECT RAISE(ABORT, 'source receipt event slot already exists'); END;
DROP TRIGGER IF EXISTS source_receipt_slot_no_delete;
CREATE TRIGGER source_receipt_slot_no_delete BEFORE DELETE ON source_receipt_event_slots
BEGIN SELECT RAISE(ABORT, 'source receipt event slots must be retained'); END;
DROP TRIGGER IF EXISTS source_receipt_slot_no_update;
CREATE TRIGGER source_receipt_slot_no_update BEFORE UPDATE ON source_receipt_event_slots
BEGIN SELECT RAISE(ABORT, 'source receipt event slots are immutable'); END;
-- Receipt identities are append-only; INSERT OR REPLACE must not erase them
-- even when SQLite recursive delete triggers are disabled.
DROP TRIGGER IF EXISTS source_receipt_no_replace;
CREATE TRIGGER source_receipt_no_replace BEFORE INSERT ON source_mutation_receipts
WHEN EXISTS(SELECT 1 FROM source_mutation_receipts
            WHERE (operation = NEW.operation AND operation_key = NEW.operation_key)
               OR rowid = NEW.rowid)
BEGIN SELECT RAISE(ABORT, 'source receipt identity already exists'); END;
DROP TRIGGER IF EXISTS source_receipt_no_delete;
CREATE TRIGGER source_receipt_no_delete BEFORE DELETE ON source_mutation_receipts
BEGIN SELECT RAISE(ABORT, 'source receipts must be retained'); END;
DROP TRIGGER IF EXISTS source_receipt_no_update;
CREATE TRIGGER source_receipt_no_update BEFORE UPDATE ON source_mutation_receipts
BEGIN SELECT RAISE(ABORT, 'source receipts are immutable'); END;
-- Old connections also create the slot in the receipt transaction. Burst
-- writes two receipt domains for one event; they must agree on the slot.
DROP TRIGGER IF EXISTS source_receipt_reserve_slot;
CREATE TRIGGER source_receipt_reserve_slot AFTER INSERT ON source_mutation_receipts
WHEN NEW.operation IN ('source_batch_append', 'burst_asset_append', 'burst_asset_append_event', 'reacquire_items', 'nisar_subset_append_v2', 'nisar_subset_append_event_v2')
BEGIN
    SELECT RAISE(ABORT, 'source receipt event is missing') WHERE NOT EXISTS (
        SELECT 1 FROM task_events WHERE event_id = NEW.operation_key AND sequence = NEW.committed_revision
    );
    SELECT RAISE(ABORT, 'source receipt event slot disagrees') WHERE EXISTS (
        SELECT 1 FROM source_receipt_event_slots s JOIN task_events e ON e.event_id = NEW.operation_key
        WHERE s.event_id = NEW.operation_key AND (s.task_id IS NOT e.task_id OR s.sequence IS NOT e.sequence)
    );
    INSERT INTO source_receipt_event_slots(event_id, task_id, sequence)
    SELECT e.event_id, e.task_id, e.sequence FROM task_events e
    WHERE e.event_id = NEW.operation_key
      AND NOT EXISTS(SELECT 1 FROM source_receipt_event_slots s WHERE s.event_id = e.event_id);
END;
DROP TRIGGER IF EXISTS source_receipt_event_reservation_insert;
CREATE TRIGGER source_receipt_event_reservation_insert
BEFORE INSERT ON task_events
WHEN EXISTS (
    SELECT 1 FROM source_receipt_event_slots
    WHERE event_id = NEW.event_id OR (task_id = NEW.task_id AND sequence = NEW.sequence)
) OR EXISTS (
    SELECT 1 FROM source_mutation_receipts
    WHERE operation IN
      ('source_batch_append', 'burst_asset_append', 'burst_asset_append_event', 'reacquire_items', 'nisar_subset_append_v2', 'nisar_subset_append_event_v2')
      AND (operation_key = NEW.event_id OR operation_key IN (
          SELECT event_id FROM task_events
          WHERE (task_id = NEW.task_id AND sequence = NEW.sequence) OR rowid = NEW.rowid
      ))
)
BEGIN SELECT RAISE(ABORT, 'event identity is reserved by a source receipt'); END;
DROP TRIGGER IF EXISTS source_receipt_event_reservation_update;
CREATE TRIGGER source_receipt_event_reservation_update
BEFORE UPDATE ON task_events
WHEN EXISTS (
    SELECT 1 FROM source_receipt_event_slots
    WHERE event_id IN (OLD.event_id, NEW.event_id)
       OR (task_id = OLD.task_id AND sequence = OLD.sequence)
       OR (task_id = NEW.task_id AND sequence = NEW.sequence)
) OR EXISTS (
    SELECT 1 FROM source_mutation_receipts
    WHERE operation IN
      ('source_batch_append', 'burst_asset_append', 'burst_asset_append_event', 'reacquire_items', 'nisar_subset_append_v2', 'nisar_subset_append_event_v2')
      AND (operation_key IN (OLD.event_id, NEW.event_id) OR operation_key IN (
          SELECT event_id FROM task_events
          WHERE (task_id = NEW.task_id AND sequence = NEW.sequence) OR rowid = NEW.rowid
      ))
)
BEGIN SELECT RAISE(ABORT, 'event identity is reserved by a source receipt'); END;
"#;

pub(crate) const EXPECTED_SOURCE_RECEIPT_TABLES: &[(&str, &str)] = &[
    (
        "source_mutation_receipts",
        r#"CREATE TABLE source_mutation_receipts (
            operation TEXT NOT NULL,
            operation_key TEXT NOT NULL,
            identity_version INTEGER NOT NULL,
            salt BLOB NOT NULL CHECK(length(salt) = 32),
            identity_sha256 BLOB NOT NULL CHECK(length(identity_sha256) = 32),
            committed_revision INTEGER NOT NULL CHECK(committed_revision >= 0),
            PRIMARY KEY(operation, operation_key)
        ) STRICT"#,
    ),
    (
        "source_receipt_schema_marker",
        r#"CREATE TABLE source_receipt_schema_marker (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            protocol INTEGER NOT NULL CHECK(protocol = 2)
        ) STRICT"#,
    ),
    (
        "source_receipt_event_slots",
        r#"CREATE TABLE source_receipt_event_slots (
            event_id TEXT NOT NULL PRIMARY KEY,
            task_id TEXT NOT NULL,
            sequence INTEGER NOT NULL,
            UNIQUE(task_id, sequence)
        ) STRICT, WITHOUT ROWID"#,
    ),
];

pub(crate) const EXPECTED_SOURCE_RECEIPT_TRIGGERS: &[(&str, &str, &str)] = &[
    (
        "source_receipt_event_slots",
        "source_receipt_slot_no_replace",
        r#"CREATE TRIGGER source_receipt_slot_no_replace BEFORE INSERT ON source_receipt_event_slots
        WHEN EXISTS(SELECT 1 FROM source_receipt_event_slots
                    WHERE event_id = NEW.event_id OR (task_id = NEW.task_id AND sequence = NEW.sequence))
        BEGIN SELECT RAISE(ABORT, 'source receipt event slot already exists'); END"#,
    ),
    (
        "source_receipt_event_slots",
        "source_receipt_slot_no_delete",
        r#"CREATE TRIGGER source_receipt_slot_no_delete BEFORE DELETE ON source_receipt_event_slots
        BEGIN SELECT RAISE(ABORT, 'source receipt event slots must be retained'); END"#,
    ),
    (
        "source_receipt_event_slots",
        "source_receipt_slot_no_update",
        r#"CREATE TRIGGER source_receipt_slot_no_update BEFORE UPDATE ON source_receipt_event_slots
        BEGIN SELECT RAISE(ABORT, 'source receipt event slots are immutable'); END"#,
    ),
    (
        "source_mutation_receipts",
        "source_receipt_no_replace",
        r#"CREATE TRIGGER source_receipt_no_replace BEFORE INSERT ON source_mutation_receipts
        WHEN EXISTS(SELECT 1 FROM source_mutation_receipts
                    WHERE (operation = NEW.operation AND operation_key = NEW.operation_key)
                       OR rowid = NEW.rowid)
        BEGIN SELECT RAISE(ABORT, 'source receipt identity already exists'); END"#,
    ),
    (
        "source_mutation_receipts",
        "source_receipt_no_delete",
        r#"CREATE TRIGGER source_receipt_no_delete BEFORE DELETE ON source_mutation_receipts
        BEGIN SELECT RAISE(ABORT, 'source receipts must be retained'); END"#,
    ),
    (
        "source_mutation_receipts",
        "source_receipt_no_update",
        r#"CREATE TRIGGER source_receipt_no_update BEFORE UPDATE ON source_mutation_receipts
        BEGIN SELECT RAISE(ABORT, 'source receipts are immutable'); END"#,
    ),
    (
        "source_mutation_receipts",
        "source_receipt_reserve_slot",
        r#"CREATE TRIGGER source_receipt_reserve_slot AFTER INSERT ON source_mutation_receipts
        WHEN NEW.operation IN ('source_batch_append', 'burst_asset_append', 'burst_asset_append_event', 'reacquire_items', 'nisar_subset_append_v2', 'nisar_subset_append_event_v2')
        BEGIN
            SELECT RAISE(ABORT, 'source receipt event is missing') WHERE NOT EXISTS (
                SELECT 1 FROM task_events WHERE event_id = NEW.operation_key AND sequence = NEW.committed_revision
            );
            SELECT RAISE(ABORT, 'source receipt event slot disagrees') WHERE EXISTS (
                SELECT 1 FROM source_receipt_event_slots s JOIN task_events e ON e.event_id = NEW.operation_key
                WHERE s.event_id = NEW.operation_key AND (s.task_id IS NOT e.task_id OR s.sequence IS NOT e.sequence)
            );
            INSERT INTO source_receipt_event_slots(event_id, task_id, sequence)
            SELECT e.event_id, e.task_id, e.sequence FROM task_events e
            WHERE e.event_id = NEW.operation_key
              AND NOT EXISTS(SELECT 1 FROM source_receipt_event_slots s WHERE s.event_id = e.event_id);
        END"#,
    ),
    (
        "task_events",
        "source_receipt_event_reservation_insert",
        r#"CREATE TRIGGER source_receipt_event_reservation_insert
        BEFORE INSERT ON task_events
        WHEN EXISTS (
            SELECT 1 FROM source_receipt_event_slots
            WHERE event_id = NEW.event_id OR (task_id = NEW.task_id AND sequence = NEW.sequence)
        ) OR EXISTS (
            SELECT 1 FROM source_mutation_receipts
            WHERE operation IN
              ('source_batch_append', 'burst_asset_append', 'burst_asset_append_event', 'reacquire_items', 'nisar_subset_append_v2', 'nisar_subset_append_event_v2')
              AND (operation_key = NEW.event_id OR operation_key IN (
                  SELECT event_id FROM task_events
                  WHERE (task_id = NEW.task_id AND sequence = NEW.sequence) OR rowid = NEW.rowid
              ))
        )
        BEGIN SELECT RAISE(ABORT, 'event identity is reserved by a source receipt'); END"#,
    ),
    (
        "task_events",
        "source_receipt_event_reservation_update",
        r#"CREATE TRIGGER source_receipt_event_reservation_update
        BEFORE UPDATE ON task_events
        WHEN EXISTS (
            SELECT 1 FROM source_receipt_event_slots
            WHERE event_id IN (OLD.event_id, NEW.event_id)
               OR (task_id = OLD.task_id AND sequence = OLD.sequence)
               OR (task_id = NEW.task_id AND sequence = NEW.sequence)
        ) OR EXISTS (
            SELECT 1 FROM source_mutation_receipts
            WHERE operation IN
              ('source_batch_append', 'burst_asset_append', 'burst_asset_append_event', 'reacquire_items', 'nisar_subset_append_v2', 'nisar_subset_append_event_v2')
              AND (operation_key IN (OLD.event_id, NEW.event_id) OR operation_key IN (
                  SELECT event_id FROM task_events
                  WHERE (task_id = NEW.task_id AND sequence = NEW.sequence) OR rowid = NEW.rowid
              ))
        )
        BEGIN SELECT RAISE(ABORT, 'event identity is reserved by a source receipt'); END"#,
    ),
];

const NISAR_SUCCESSOR: &str = "nisar_successor_v2";
const SOURCE_APPEND: &str = "source_batch_append";
const REACQUIRE: &str = "reacquire_items";
const HISTORICAL_SUCCESSOR: &str = "historical_source_successor";
const BURST_APPEND: &str = "burst_asset_append";
const BURST_APPEND_EVENT: &str = "burst_asset_append_event";
const BURST_SUCCESSOR: &str = "burst_asset_successor";
const NISAR_ARCHIVED_RECOVERY: &str = "nisar_archived_recovery_v2";
const IDENTITY_VERSION: i64 = 1;

// Field order, JSON encoding and the domain are part of identity version 1.
// Do not normalize paths or reorder source IDs when reading an old receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreNisarSuccessorIdentity {
    pub source_task_id: String,
    pub expected_task_revision: u64,
    pub source_item_ids: Vec<String>,
    pub new_task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
    pub recreate_original_items: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NisarSuccessorReceiptStatus {
    Committed,
    Conflict,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAppendReceiptStatus {
    Committed,
    LegacyCommitted,
    Conflict,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoricalSuccessorReceiptStatus {
    Committed,
    Conflict,
    Unknown,
}

/// The service's complete outer request, before parent rows are reordered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreBurstAppendIdentity {
    pub request_id: String,
    pub task_id: String,
    pub selected_product_ids: Vec<String>,
    pub expected_task_revision: u64,
    pub event_id: String,
    pub occurred_at: String,
}

/// Exact external NISAR append request, including source selection order.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreNisarAppendIdentity {
    pub task_id: String,
    pub expected_task_revision: u64,
    pub source_item_ids: Vec<String>,
    pub event_id: String,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NisarAppendReceiptStatus {
    Committed,
    Conflict,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BurstAppendReceiptStatus {
    Committed,
    Conflict,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreBurstSuccessorIdentity {
    pub request_id: String,
    pub source_task_id: String,
    pub expected_source_task_revision: u64,
    pub selected_product_ids: Vec<String>,
    pub new_task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BurstSuccessorReceiptStatus {
    Committed,
    Conflict,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NisarArchivedRecoveryReceiptStatus {
    Committed,
    Conflict,
    Unknown,
}

/// A receipt proves a commit, independently of the current target's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMutationCommitStatus {
    Committed,
    LegacyCommitted,
    /// An exact activity event exists, but there is no independent receipt.
    EventCommitted,
    Conflict,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMutationTargetStatus {
    /// Snapshot and runtime rows exist; this does not imply an actionable or
    /// decodable task, nor that it is currently displayed by the UI.
    Present,
    Tombstoned,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceMutationStatus {
    pub commit: SourceMutationCommitStatus,
    pub target: SourceMutationTargetStatus,
}

/// The exact frozen request is required; query results do not contain it.
pub enum SourceMutationQuery<'a> {
    SourceAppend(&'a AppendTaskFromSourceBatchRequest),
    HistoricalSuccessor(&'a CreateTaskFromHistoricalSourceBatchRequest),
    BurstAppend(&'a StoreBurstAppendIdentity),
    BurstSuccessor(&'a StoreBurstSuccessorIdentity),
    NisarAppend(&'a StoreNisarAppendIdentity),
    NisarSuccessor(&'a StoreNisarSuccessorIdentity),
    NisarArchivedRecovery(&'a crate::StoreCreateNisarArchivedRecoveryV2Request),
    Reacquire(&'a StoreUpdateTaskRequest),
}

fn source_mutation_target_status(
    connection: &Connection,
    task_id: &str,
) -> Result<SourceMutationTargetStatus, StoreError> {
    let (tombstone, snapshot, runtime): (bool, bool, bool) = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1), \
         EXISTS(SELECT 1 FROM task_snapshots WHERE task_id = ?1), \
         EXISTS(SELECT 1 FROM task_runtime WHERE task_id = ?1)",
        [task_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if tombstone {
        Ok(SourceMutationTargetStatus::Tombstoned)
    } else if snapshot && runtime {
        Ok(SourceMutationTargetStatus::Present)
    } else if !snapshot && !runtime {
        Ok(SourceMutationTargetStatus::Missing)
    } else {
        Err(StoreError::invalid("incomplete source mutation target"))
    }
}

fn nisar_archived_recovery_digest(
    request: &crate::StoreCreateNisarArchivedRecoveryV2Request,
    salt: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let encoded = serde_json::to_vec(request).map_err(StoreError::Corrupt)?;
    let mut hash = Sha256::new();
    hash.update(b"sceneport/nisar-archived-recovery/v1\0");
    hash.update(salt);
    hash.update(encoded);
    Ok(hash.finalize().to_vec())
}

pub(crate) fn nisar_archived_recovery_status(
    connection: &Connection,
    request: &crate::StoreCreateNisarArchivedRecoveryV2Request,
) -> Result<NisarArchivedRecoveryReceiptStatus, StoreError> {
    let stored = connection.query_row(
        "SELECT identity_version, salt, identity_sha256 FROM source_mutation_receipts WHERE operation = ?1 AND operation_key = ?2",
        params![NISAR_ARCHIVED_RECOVERY, request.new_task_id],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?)),
    ).optional()?;
    match stored {
        Some((version, salt, expected))
            if version == IDENTITY_VERSION && salt.len() == 32 && expected.len() == 32 =>
        {
            Ok(
                if nisar_archived_recovery_digest(request, &salt)? == expected {
                    NisarArchivedRecoveryReceiptStatus::Committed
                } else {
                    NisarArchivedRecoveryReceiptStatus::Conflict
                },
            )
        }
        None => Ok(NisarArchivedRecoveryReceiptStatus::Unknown),
        Some(_) => Err(StoreError::invalid(
            "unsupported or corrupt source mutation receipt",
        )),
    }
}

pub(crate) fn insert_nisar_archived_recovery_receipt(
    transaction: &Transaction<'_>,
    request: &crate::StoreCreateNisarArchivedRecoveryV2Request,
    revision: u64,
) -> Result<(), StoreError> {
    let revision = i64::try_from(revision)
        .map_err(|_| StoreError::invalid("receipt revision is out of range"))?;
    let salt: Vec<u8> = transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
    let digest = nisar_archived_recovery_digest(request, &salt)?;
    transaction.execute(
        "INSERT INTO source_mutation_receipts(operation, operation_key, identity_version, salt, identity_sha256, committed_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![NISAR_ARCHIVED_RECOVERY, request.new_task_id, IDENTITY_VERSION, salt, digest, revision],
    )?;
    Ok(())
}

fn burst_successor_digest(
    identity: &StoreBurstSuccessorIdentity,
    salt: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let encoded = serde_json::to_vec(identity).map_err(StoreError::Corrupt)?;
    let mut hash = Sha256::new();
    hash.update(b"sceneport/burst-asset-successor/v1\0");
    hash.update(salt);
    hash.update(encoded);
    Ok(hash.finalize().to_vec())
}

pub(crate) fn burst_successor_status(
    connection: &Connection,
    identity: &StoreBurstSuccessorIdentity,
) -> Result<BurstSuccessorReceiptStatus, StoreError> {
    let stored = connection.query_row(
        "SELECT identity_version, salt, identity_sha256 FROM source_mutation_receipts WHERE operation = ?1 AND operation_key = ?2",
        params![BURST_SUCCESSOR, identity.new_task_id],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?)),
    ).optional()?;
    match stored {
        Some((version, salt, expected))
            if version == IDENTITY_VERSION && salt.len() == 32 && expected.len() == 32 =>
        {
            Ok(if burst_successor_digest(identity, &salt)? == expected {
                BurstSuccessorReceiptStatus::Committed
            } else {
                BurstSuccessorReceiptStatus::Conflict
            })
        }
        None => Ok(BurstSuccessorReceiptStatus::Unknown),
        Some(_) => Err(StoreError::invalid(
            "unsupported or corrupt source mutation receipt",
        )),
    }
}

pub(crate) fn insert_burst_successor_receipt(
    transaction: &Transaction<'_>,
    identity: &StoreBurstSuccessorIdentity,
    revision: u64,
) -> Result<(), StoreError> {
    let revision = i64::try_from(revision)
        .map_err(|_| StoreError::invalid("receipt revision is out of range"))?;
    let salt: Vec<u8> = transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
    let digest = burst_successor_digest(identity, &salt)?;
    transaction.execute(
        "INSERT INTO source_mutation_receipts(operation, operation_key, identity_version, salt, identity_sha256, committed_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![BURST_SUCCESSOR, identity.new_task_id, IDENTITY_VERSION, salt, digest, revision],
    )?;
    Ok(())
}

// An occupied task/revision slot contradicts a missing original event. Older
// receipts may have lost their event before migration and lack a reserved slot.
fn reject_conflicting_event_slot(
    connection: &Connection,
    task_id: &str,
    sequence: i64,
) -> Result<(), StoreError> {
    let occupied: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_events WHERE task_id = ?1 AND sequence = ?2)",
        params![task_id, sequence],
        |row| row.get(0),
    )?;
    if occupied {
        return Err(StoreError::invalid("receipt event sequence is occupied"));
    }
    Ok(())
}

type DualReceiptEventRow = (
    Option<i64>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<i64>,
    Option<i64>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

type SingleReceiptEventRow = (
    Option<i64>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

const NISAR_APPEND: &str = "nisar_subset_append_v2";
const NISAR_APPEND_EVENT: &str = "nisar_subset_append_event_v2";

fn nisar_append_digest(
    identity: &StoreNisarAppendIdentity,
    salt: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let encoded = serde_json::to_vec(identity).map_err(StoreError::Corrupt)?;
    let mut hash = Sha256::new();
    hash.update(b"sceneport/nisar-subset-append/v1\0");
    hash.update(salt);
    hash.update(encoded);
    Ok(hash.finalize().to_vec())
}

fn nisar_append_event_digest(payload: &str, salt: &[u8]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"sceneport/nisar-subset-append-event/v1\0");
    hash.update(salt);
    hash.update(payload.as_bytes());
    hash.finalize().to_vec()
}

pub(crate) fn nisar_append_status(
    connection: &Connection,
    identity: &StoreNisarAppendIdentity,
) -> Result<NisarAppendReceiptStatus, StoreError> {
    let (version, salt, expected, revision, eversion, esalt, edigest, erevision, task, kind, payload, time, sequence): DualReceiptEventRow = connection.query_row(
        "SELECT r.identity_version,r.salt,r.identity_sha256,r.committed_revision,b.identity_version,b.salt,b.identity_sha256,b.committed_revision,e.task_id,e.event_type,e.payload_json,e.occurred_at,e.sequence FROM (SELECT 1) x LEFT JOIN source_mutation_receipts r ON r.operation=?1 AND r.operation_key=?2 LEFT JOIN source_mutation_receipts b ON b.operation=?3 AND b.operation_key=?2 LEFT JOIN task_events e ON e.event_id=?2",
        params![NISAR_APPEND, identity.event_id, NISAR_APPEND_EVENT],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?,row.get(11)?,row.get(12)?)),
    )?;
    match (version, salt, expected) {
        (Some(version), Some(salt), Some(expected))
            if version == IDENTITY_VERSION && salt.len() == 32 && expected.len() == 32 =>
        {
            if nisar_append_digest(identity, &salt)? != expected {
                return Ok(NisarAppendReceiptStatus::Conflict);
            }
            let expected_revision = identity.expected_task_revision.checked_add(1);
            if expected_revision.is_none()
                || revision.and_then(|value| u64::try_from(value).ok()) != expected_revision
                || erevision != revision
            {
                return Err(StoreError::invalid(
                    "NISAR append receipt revision disagrees",
                ));
            }
            let (esalt, edigest) = match (eversion, esalt, edigest) {
                (Some(v), Some(salt), Some(digest))
                    if v == IDENTITY_VERSION && salt.len() == 32 && digest.len() == 32 =>
                {
                    (salt, digest)
                }
                _ => {
                    return Err(StoreError::invalid(
                        "missing or corrupt NISAR append event receipt",
                    ))
                }
            };
            match (task, kind, payload, time, sequence) {
                (Some(task), Some(kind), Some(payload), Some(time), Some(sequence))
                    if task == identity.task_id
                        && kind == "append_items"
                        && time == identity.occurred_at
                        && Some(sequence) == revision
                        && nisar_append_event_digest(&payload, &esalt) == edigest => {}
                (None, None, None, None, None) => reject_conflicting_event_slot(
                    connection,
                    &identity.task_id,
                    revision.ok_or_else(|| StoreError::invalid("missing receipt revision"))?,
                )?,
                _ => {
                    return Err(StoreError::invalid(
                        "NISAR append receipt and event disagree",
                    ))
                }
            }
            Ok(NisarAppendReceiptStatus::Committed)
        }
        (None, None, None) => match (
            revision, eversion, esalt, edigest, erevision, task, kind, payload, time, sequence,
        ) {
            (None, None, None, None, None, None, None, None, None, None) => {
                Ok(NisarAppendReceiptStatus::Unknown)
            }
            (None, None, None, None, None, Some(_), Some(_), Some(_), Some(_), Some(_)) => {
                Ok(NisarAppendReceiptStatus::Conflict)
            }
            _ => Err(StoreError::invalid("corrupt NISAR append receipt")),
        },
        _ => Err(StoreError::invalid(
            "unsupported or corrupt NISAR append receipt",
        )),
    }
}

pub(crate) fn insert_nisar_append_receipt(
    transaction: &Transaction<'_>,
    identity: &StoreNisarAppendIdentity,
    event_payload: &str,
    revision: u64,
) -> Result<(), StoreError> {
    let revision = i64::try_from(revision)
        .map_err(|_| StoreError::invalid("receipt revision is out of range"))?;
    let salt: Vec<u8> = transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
    let digest = nisar_append_digest(identity, &salt)?;
    transaction.execute("INSERT INTO source_mutation_receipts(operation,operation_key,identity_version,salt,identity_sha256,committed_revision) VALUES (?1,?2,?3,?4,?5,?6)", params![NISAR_APPEND, identity.event_id, IDENTITY_VERSION, salt, digest, revision])?;
    let salt: Vec<u8> = transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
    let digest = nisar_append_event_digest(event_payload, &salt);
    transaction.execute("INSERT INTO source_mutation_receipts(operation,operation_key,identity_version,salt,identity_sha256,committed_revision) VALUES (?1,?2,?3,?4,?5,?6)", params![NISAR_APPEND_EVENT, identity.event_id, IDENTITY_VERSION, salt, digest, revision])?;
    Ok(())
}

fn burst_append_digest(
    identity: &StoreBurstAppendIdentity,
    salt: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let encoded = serde_json::to_vec(identity).map_err(StoreError::Corrupt)?;
    let mut hash = Sha256::new();
    hash.update(b"sceneport/burst-asset-append/v1\0");
    hash.update(salt);
    hash.update(encoded);
    Ok(hash.finalize().to_vec())
}

fn burst_event_digest(payload: &str, salt: &[u8]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"sceneport/burst-asset-append-event/v1\0");
    hash.update(salt);
    hash.update(payload.as_bytes());
    hash.finalize().to_vec()
}

pub(crate) fn burst_append_status(
    connection: &Connection,
    identity: &StoreBurstAppendIdentity,
) -> Result<BurstAppendReceiptStatus, StoreError> {
    let (version, salt, expected, revision, event_version, event_salt, event_digest, event_revision, event_task, event_type, event_payload, event_time, event_sequence): DualReceiptEventRow = connection.query_row(
        "SELECT r.identity_version, r.salt, r.identity_sha256, r.committed_revision, b.identity_version, b.salt, b.identity_sha256, b.committed_revision, e.task_id, e.event_type, e.payload_json, e.occurred_at, e.sequence FROM (SELECT 1) x \
         LEFT JOIN source_mutation_receipts r ON r.operation = ?1 AND r.operation_key = ?2 \
         LEFT JOIN source_mutation_receipts b ON b.operation = ?3 AND b.operation_key = ?2 \
         LEFT JOIN task_events e ON e.event_id = ?2",
        params![BURST_APPEND, identity.event_id, BURST_APPEND_EVENT],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?)),
    )?;
    match (version, salt, expected) {
        (Some(version), Some(salt), Some(expected))
            if version == IDENTITY_VERSION && salt.len() == 32 && expected.len() == 32 =>
        {
            if burst_append_digest(identity, &salt)? != expected {
                return Ok(BurstAppendReceiptStatus::Conflict);
            }
            let expected_revision = identity.expected_task_revision.checked_add(1);
            if revision.and_then(|value| u64::try_from(value).ok()) != expected_revision
                || expected_revision.is_none()
                || event_revision != revision
            {
                return Err(StoreError::invalid(
                    "Burst append receipt revision disagrees",
                ));
            }
            let (event_salt, event_digest) = match (event_version, event_salt, event_digest) {
                (Some(version), Some(salt), Some(digest))
                    if version == IDENTITY_VERSION && salt.len() == 32 && digest.len() == 32 =>
                {
                    (salt, digest)
                }
                _ => {
                    return Err(StoreError::invalid(
                        "missing or corrupt Burst event receipt",
                    ))
                }
            };
            match (
                event_task,
                event_type,
                event_payload,
                event_time,
                event_sequence,
            ) {
                (Some(task), Some(kind), Some(payload), Some(time), Some(sequence))
                    if task == identity.task_id
                        && kind == "append_items"
                        && time == identity.occurred_at
                        && Some(sequence) == revision
                        && burst_event_digest(&payload, &event_salt) == event_digest => {}
                (None, None, None, None, None) => {
                    reject_conflicting_event_slot(
                        connection,
                        &identity.task_id,
                        revision.ok_or_else(|| StoreError::invalid("missing receipt revision"))?,
                    )?;
                }
                _ => {
                    return Err(StoreError::invalid(
                        "Burst append receipt and event disagree",
                    ))
                }
            }
            Ok(BurstAppendReceiptStatus::Committed)
        }
        (None, None, None) => match (
            event_version,
            event_salt,
            event_digest,
            event_revision,
            event_task,
            event_type,
            event_payload,
            event_time,
            event_sequence,
        ) {
            (None, None, None, None, None, None, None, None, None) => {
                Ok(BurstAppendReceiptStatus::Unknown)
            }
            (None, None, None, None, Some(_), Some(_), Some(_), Some(_), Some(_)) => {
                Ok(BurstAppendReceiptStatus::Conflict)
            }
            _ => Err(StoreError::invalid("corrupt Burst append event")),
        },
        _ => Err(StoreError::invalid(
            "unsupported or corrupt source mutation receipt",
        )),
    }
}

pub(crate) fn insert_burst_append_receipt(
    transaction: &Transaction<'_>,
    identity: &StoreBurstAppendIdentity,
    event_payload: &str,
    revision: u64,
) -> Result<(), StoreError> {
    let revision = i64::try_from(revision)
        .map_err(|_| StoreError::invalid("receipt revision is out of range"))?;
    let salt: Vec<u8> = transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
    let digest = burst_append_digest(identity, &salt)?;
    transaction.execute(
        "INSERT INTO source_mutation_receipts(operation, operation_key, identity_version, salt, identity_sha256, committed_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![BURST_APPEND, identity.event_id, IDENTITY_VERSION, salt, digest, revision],
    )?;
    let event_salt: Vec<u8> =
        transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
    let event_digest = burst_event_digest(event_payload, &event_salt);
    transaction.execute(
        "INSERT INTO source_mutation_receipts(operation, operation_key, identity_version, salt, identity_sha256, committed_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![BURST_APPEND_EVENT, identity.event_id, IDENTITY_VERSION, event_salt, event_digest, revision],
    )?;
    Ok(())
}

fn historical_successor_digest(
    request: &CreateTaskFromHistoricalSourceBatchRequest,
    salt: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let encoded = serde_json::to_vec(request).map_err(StoreError::Corrupt)?;
    let mut hash = Sha256::new();
    hash.update(b"sceneport/historical-source-successor/v1\0");
    hash.update(salt);
    hash.update(encoded);
    Ok(hash.finalize().to_vec())
}

pub(crate) fn historical_successor_status(
    connection: &Connection,
    request: &CreateTaskFromHistoricalSourceBatchRequest,
) -> Result<HistoricalSuccessorReceiptStatus, StoreError> {
    request.validate().map_err(StoreError::invalid)?;
    if request.output_root.as_os_str().is_empty() {
        return Err(StoreError::invalid("task output is required"));
    }
    let stored = connection.query_row(
        "SELECT identity_version, salt, identity_sha256 FROM source_mutation_receipts WHERE operation = ?1 AND operation_key = ?2",
        params![HISTORICAL_SUCCESSOR, request.new_task_id],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?)),
    ).optional()?;
    match stored {
        Some((version, salt, expected))
            if version == IDENTITY_VERSION && salt.len() == 32 && expected.len() == 32 =>
        {
            Ok(
                if historical_successor_digest(request, &salt)? == expected {
                    HistoricalSuccessorReceiptStatus::Committed
                } else {
                    HistoricalSuccessorReceiptStatus::Conflict
                },
            )
        }
        Some(_) => Err(StoreError::invalid(
            "unsupported or corrupt source mutation receipt",
        )),
        None => Ok(HistoricalSuccessorReceiptStatus::Unknown),
    }
}

pub(crate) fn insert_historical_successor_receipt(
    transaction: &Transaction<'_>,
    request: &CreateTaskFromHistoricalSourceBatchRequest,
    revision: u64,
) -> Result<(), StoreError> {
    let revision = i64::try_from(revision)
        .map_err(|_| StoreError::invalid("receipt revision is out of range"))?;
    let salt: Vec<u8> = transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
    let identity_sha256 = historical_successor_digest(request, &salt)?;
    transaction.execute(
        "INSERT INTO source_mutation_receipts(operation, operation_key, identity_version, salt, identity_sha256, committed_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![HISTORICAL_SUCCESSOR, request.new_task_id, IDENTITY_VERSION, salt, identity_sha256, revision],
    )?;
    Ok(())
}

// Reacquire receipts survive task and event deletion. The original ordered
// request is retained as an identity digest, not as a recoverable payload.
pub(crate) fn reacquire_receipt_status(
    connection: &Connection,
    request: &StoreUpdateTaskRequest,
) -> Result<crate::ReacquireEventStatus, StoreError> {
    use crate::ReacquireEventStatus;
    // Reuse event validation and inspect any surviving event in this snapshot.
    let event = crate::runtime::reacquire_event_identity_status(connection, request)?;
    let stored: Option<(i64, Vec<u8>, Vec<u8>, i64)> = connection.query_row(
        "SELECT identity_version, salt, identity_sha256, committed_revision FROM source_mutation_receipts WHERE operation = ?1 AND operation_key = ?2",
        params![REACQUIRE, request.event_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional()?;
    let Some((version, salt, expected, revision)) = stored else {
        return Ok(ReacquireEventStatus::Unknown);
    };
    if version != IDENTITY_VERSION || salt.len() != 32 || expected.len() != 32 {
        return Err(StoreError::invalid(
            "unsupported or corrupt reacquire receipt",
        ));
    }
    if reacquire_digest(request, &salt)? != expected {
        return Ok(ReacquireEventStatus::Conflict);
    }
    if request.expected_revision.checked_add(1) != u64::try_from(revision).ok()
        || request.expected_revision.checked_add(1).is_none()
        || event == ReacquireEventStatus::Conflict
    {
        return Err(StoreError::invalid("reacquire receipt and event disagree"));
    }
    if event == ReacquireEventStatus::Unknown {
        reject_conflicting_event_slot(connection, &request.task_id, revision)?;
    }
    Ok(ReacquireEventStatus::Committed)
}

fn reacquire_digest(request: &StoreUpdateTaskRequest, salt: &[u8]) -> Result<Vec<u8>, StoreError> {
    let encoded = serde_json::to_vec(request).map_err(StoreError::Corrupt)?;
    let mut hash = Sha256::new();
    hash.update(b"sceneport/reacquire-items/v1\0");
    hash.update(salt);
    hash.update(encoded);
    Ok(hash.finalize().to_vec())
}

pub(crate) fn insert_reacquire_receipt(
    transaction: &Transaction<'_>,
    request: &StoreUpdateTaskRequest,
    revision: u64,
) -> Result<(), StoreError> {
    let revision = i64::try_from(revision)
        .map_err(|_| StoreError::invalid("receipt revision is out of range"))?;
    let salt: Vec<u8> = transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
    let identity = reacquire_digest(request, &salt)?;
    transaction.execute(
        "INSERT INTO source_mutation_receipts(operation, operation_key, identity_version, salt, identity_sha256, committed_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![REACQUIRE, request.event_id, IDENTITY_VERSION, salt, identity, revision],
    )?;
    Ok(())
}

pub(crate) fn has_reacquire_receipt(
    connection: &Connection,
    event_id: &str,
) -> Result<bool, StoreError> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM source_mutation_receipts WHERE operation = ?1 AND operation_key = ?2)",
        params![REACQUIRE, event_id], |row| row.get(0),
    ).map_err(StoreError::from)
}

fn source_append_digest(
    request: &AppendTaskFromSourceBatchRequest,
    salt: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let encoded = serde_json::to_vec(request).map_err(StoreError::Corrupt)?;
    let mut hash = Sha256::new();
    hash.update(b"sceneport/source-batch-append/v1\0");
    hash.update(salt);
    hash.update(encoded);
    Ok(hash.finalize().to_vec())
}

pub(crate) fn source_append_status(
    connection: &Connection,
    request: &AppendTaskFromSourceBatchRequest,
) -> Result<SourceAppendReceiptStatus, StoreError> {
    request.validate().map_err(StoreError::invalid)?;
    if request.occurred_at.trim().is_empty() {
        return Err(StoreError::invalid("append timestamp is required"));
    }
    // One statement gives the receipt and legacy event a consistent SQLite
    // read snapshot, even when another connection is committing the request.
    let (version, salt, expected, revision, event_task, event_type, event_payload, event_time, event_sequence): SingleReceiptEventRow = connection.query_row(
        "SELECT r.identity_version, r.salt, r.identity_sha256, r.committed_revision, e.task_id, e.event_type, e.payload_json, e.occurred_at, e.sequence \
         FROM (SELECT 1) x LEFT JOIN source_mutation_receipts r ON r.operation = ?1 AND r.operation_key = ?2 \
         LEFT JOIN task_events e ON e.event_id = ?2",
        params![SOURCE_APPEND, request.event_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?)),
    )?;
    match (version, salt, expected) {
        (Some(version), Some(salt), Some(expected))
            if version == IDENTITY_VERSION && salt.len() == 32 && expected.len() == 32 =>
        {
            if source_append_digest(request, &salt)? != expected {
                return Ok(SourceAppendReceiptStatus::Conflict);
            }
            let expected_revision = request.expected_task_revision.checked_add(1);
            if expected_revision.is_none()
                || revision.and_then(|value| u64::try_from(value).ok()) != expected_revision
            {
                return Err(StoreError::invalid("append receipt revision disagrees"));
            }
            match (
                event_task,
                event_type,
                event_payload,
                event_time,
                event_sequence,
            ) {
                (Some(task), Some(kind), Some(payload), Some(time), Some(sequence)) => {
                    let request_json =
                        serde_json::to_string(request).map_err(StoreError::Corrupt)?;
                    if task != request.task_id
                        || kind != "append_items"
                        || payload != request_json
                        || time != request.occurred_at
                        || Some(sequence) != revision
                    {
                        return Err(StoreError::invalid("append receipt and event disagree"));
                    }
                }
                // A receipt is retained independently if an event is ever pruned.
                (None, None, None, None, None) => {
                    reject_conflicting_event_slot(
                        connection,
                        &request.task_id,
                        revision.ok_or_else(|| StoreError::invalid("missing receipt revision"))?,
                    )?;
                }
                _ => return Err(StoreError::invalid("corrupt append event")),
            }
            Ok(SourceAppendReceiptStatus::Committed)
        }
        (None, None, None) => match (
            revision,
            event_task,
            event_type,
            event_payload,
            event_time,
            event_sequence,
        ) {
            (None, Some(task), Some(kind), Some(payload), Some(time), Some(sequence)) => {
                let request_json = serde_json::to_string(request).map_err(StoreError::Corrupt)?;
                Ok(
                    if task == request.task_id
                        && kind == "append_items"
                        && payload == request_json
                        && time == request.occurred_at
                        && u64::try_from(sequence).ok()
                            == request.expected_task_revision.checked_add(1)
                        && request.expected_task_revision.checked_add(1).is_some()
                    {
                        SourceAppendReceiptStatus::LegacyCommitted
                    } else {
                        SourceAppendReceiptStatus::Conflict
                    },
                )
            }
            (None, None, None, None, None, None) => Ok(SourceAppendReceiptStatus::Unknown),
            _ => Err(StoreError::invalid("corrupt legacy append event")),
        },
        _ => Err(StoreError::invalid(
            "unsupported or corrupt source mutation receipt",
        )),
    }
}

pub(crate) fn insert_source_append_receipt(
    transaction: &Transaction<'_>,
    request: &AppendTaskFromSourceBatchRequest,
    revision: u64,
) -> Result<(), StoreError> {
    let revision = i64::try_from(revision)
        .map_err(|_| StoreError::invalid("receipt revision is out of range"))?;
    let salt: Vec<u8> = transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
    let identity_sha256 = source_append_digest(request, &salt)?;
    transaction.execute(
        "INSERT INTO source_mutation_receipts(operation, operation_key, identity_version, salt, identity_sha256, committed_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![SOURCE_APPEND, request.event_id, IDENTITY_VERSION, salt, identity_sha256, revision],
    )?;
    Ok(())
}

fn digest(identity: &StoreNisarSuccessorIdentity, salt: &[u8]) -> Result<Vec<u8>, StoreError> {
    let encoded = serde_json::to_vec(identity).map_err(StoreError::Corrupt)?;
    let mut hash = Sha256::new();
    hash.update(b"sceneport/nisar-successor/v1\0");
    hash.update(salt);
    hash.update(encoded);
    Ok(hash.finalize().to_vec())
}

pub(crate) fn nisar_successor_status(
    connection: &Connection,
    identity: &StoreNisarSuccessorIdentity,
) -> Result<NisarSuccessorReceiptStatus, StoreError> {
    let stored = connection.query_row(
        "SELECT identity_version, salt, identity_sha256 FROM source_mutation_receipts WHERE operation = ?1 AND operation_key = ?2",
        params![NISAR_SUCCESSOR, identity.new_task_id],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?)),
    ).optional()?;
    match stored {
        Some((version, salt, expected))
            if version == IDENTITY_VERSION && salt.len() == 32 && expected.len() == 32 =>
        {
            Ok(if digest(identity, &salt)? == expected {
                NisarSuccessorReceiptStatus::Committed
            } else {
                NisarSuccessorReceiptStatus::Conflict
            })
        }
        Some(_) => Err(StoreError::invalid(
            "unsupported or corrupt source mutation receipt",
        )),
        None => Ok(NisarSuccessorReceiptStatus::Unknown),
    }
}

pub(crate) fn insert_nisar_successor_receipt(
    transaction: &Transaction<'_>,
    identity: &StoreNisarSuccessorIdentity,
    revision: u64,
) -> Result<(), StoreError> {
    let revision = i64::try_from(revision)
        .map_err(|_| StoreError::invalid("receipt revision is out of range"))?;
    let salt: Vec<u8> = transaction.query_row("SELECT randomblob(32)", [], |row| row.get(0))?;
    let identity_sha256 = digest(identity, &salt)?;
    transaction.execute(
        "INSERT INTO source_mutation_receipts(operation, operation_key, identity_version, salt, identity_sha256, committed_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![NISAR_SUCCESSOR, identity.new_task_id, IDENTITY_VERSION, salt, identity_sha256, revision],
    )?;
    Ok(())
}

impl TaskStore {
    /// Read identity and target state from one SQLite snapshot. Unknown means
    /// only that no receipt was visible in this snapshot, not that a writer
    /// has stopped or that the caller may discard its frozen request.
    pub fn query_source_mutation_status(
        &self,
        query: SourceMutationQuery<'_>,
    ) -> Result<SourceMutationStatus, StoreError> {
        macro_rules! map_receipt {
            ($status:expr, $kind:ident) => {
                match $status {
                    $kind::Committed => SourceMutationCommitStatus::Committed,
                    $kind::Conflict => SourceMutationCommitStatus::Conflict,
                    $kind::Unknown => SourceMutationCommitStatus::Unknown,
                }
            };
        }
        let transaction = self.connection.unchecked_transaction()?;
        let archived_request = match &query {
            SourceMutationQuery::NisarArchivedRecovery(request) => Some(*request),
            _ => None,
        };
        let (commit, task_id) = match query {
            SourceMutationQuery::SourceAppend(request) => {
                let status = match source_append_status(&transaction, request)? {
                    SourceAppendReceiptStatus::Committed => SourceMutationCommitStatus::Committed,
                    SourceAppendReceiptStatus::LegacyCommitted => {
                        SourceMutationCommitStatus::LegacyCommitted
                    }
                    SourceAppendReceiptStatus::Conflict => SourceMutationCommitStatus::Conflict,
                    SourceAppendReceiptStatus::Unknown => SourceMutationCommitStatus::Unknown,
                };
                (status, request.task_id.as_str())
            }
            SourceMutationQuery::HistoricalSuccessor(request) => (
                map_receipt!(
                    historical_successor_status(&transaction, request)?,
                    HistoricalSuccessorReceiptStatus
                ),
                request.new_task_id.as_str(),
            ),
            SourceMutationQuery::BurstAppend(identity) => (
                map_receipt!(
                    burst_append_status(&transaction, identity)?,
                    BurstAppendReceiptStatus
                ),
                identity.task_id.as_str(),
            ),
            SourceMutationQuery::BurstSuccessor(identity) => (
                map_receipt!(
                    burst_successor_status(&transaction, identity)?,
                    BurstSuccessorReceiptStatus
                ),
                identity.new_task_id.as_str(),
            ),
            SourceMutationQuery::NisarAppend(identity) => (
                map_receipt!(
                    nisar_append_status(&transaction, identity)?,
                    NisarAppendReceiptStatus
                ),
                identity.task_id.as_str(),
            ),
            SourceMutationQuery::NisarSuccessor(identity) => (
                map_receipt!(
                    nisar_successor_status(&transaction, identity)?,
                    NisarSuccessorReceiptStatus
                ),
                identity.new_task_id.as_str(),
            ),
            SourceMutationQuery::NisarArchivedRecovery(request) => (
                map_receipt!(
                    nisar_archived_recovery_status(&transaction, request)?,
                    NisarArchivedRecoveryReceiptStatus
                ),
                request.new_task_id.as_str(),
            ),
            SourceMutationQuery::Reacquire(request) => (
                match reacquire_receipt_status(&transaction, request)? {
                    crate::ReacquireEventStatus::Committed => SourceMutationCommitStatus::Committed,
                    crate::ReacquireEventStatus::Conflict => SourceMutationCommitStatus::Conflict,
                    crate::ReacquireEventStatus::Unknown => {
                        match crate::runtime::reacquire_event_identity_status(
                            &transaction,
                            request,
                        )? {
                            crate::ReacquireEventStatus::Committed => {
                                SourceMutationCommitStatus::EventCommitted
                            }
                            crate::ReacquireEventStatus::Conflict => {
                                SourceMutationCommitStatus::Conflict
                            }
                            crate::ReacquireEventStatus::Unknown => {
                                SourceMutationCommitStatus::Unknown
                            }
                        }
                    }
                },
                request.task_id.as_str(),
            ),
        };
        let target = source_mutation_target_status(&transaction, task_id)?;
        if commit == SourceMutationCommitStatus::Committed
            && target == SourceMutationTargetStatus::Present
        {
            if let Some(request) = archived_request {
                crate::repository::nisar_archived_recovery::validate_recovery_receipt_target(
                    &transaction,
                    request,
                )?;
            }
        }
        transaction.commit()?;
        Ok(SourceMutationStatus { commit, target })
    }

    pub fn nisar_append_receipt_status(
        &self,
        identity: &StoreNisarAppendIdentity,
    ) -> Result<NisarAppendReceiptStatus, StoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let status = nisar_append_status(&transaction, identity)?;
        if status == NisarAppendReceiptStatus::Committed {
            let tombstoned: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
                [&identity.task_id],
                |row| row.get(0),
            )?;
            if tombstoned
                || !crate::core_restore::task_is_actionable(&transaction, &identity.task_id)?
            {
                return Err(StoreError::NotFound);
            }
            let task = crate::repository::load_task_connection(&transaction, &identity.task_id)?
                .ok_or(StoreError::NotFound)?;
            if task.runtime.revision <= identity.expected_task_revision
                || task.runtime.selected_item_ids.len() != task.snapshot.candidates.len()
            {
                return Err(StoreError::invalid("NISAR append target is inconsistent"));
            }
        }
        transaction.commit()?;
        Ok(status)
    }

    pub fn burst_successor_receipt_status(
        &self,
        identity: &StoreBurstSuccessorIdentity,
    ) -> Result<BurstSuccessorReceiptStatus, StoreError> {
        burst_successor_status(&self.connection, identity)
    }
    pub fn burst_append_receipt_status(
        &self,
        identity: &StoreBurstAppendIdentity,
    ) -> Result<BurstAppendReceiptStatus, StoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let status = burst_append_status(&transaction, identity)?;
        transaction.commit()?;
        Ok(status)
    }
    pub fn historical_successor_receipt_status(
        &self,
        request: &CreateTaskFromHistoricalSourceBatchRequest,
    ) -> Result<HistoricalSuccessorReceiptStatus, StoreError> {
        historical_successor_status(&self.connection, request)
    }

    pub fn source_append_receipt_status(
        &self,
        request: &AppendTaskFromSourceBatchRequest,
    ) -> Result<SourceAppendReceiptStatus, StoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let status = source_append_status(&transaction, request)?;
        transaction.commit()?;
        Ok(status)
    }

    pub fn nisar_successor_receipt_status(
        &self,
        identity: &StoreNisarSuccessorIdentity,
    ) -> Result<NisarSuccessorReceiptStatus, StoreError> {
        nisar_successor_status(&self.connection, identity)
    }
}
