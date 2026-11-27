use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;

const PROOF_CONTRACT: &str = "nisar_budget_upgrade_proof_v1";
/// Version 2 retains the raw progress row in the context and the exact
/// non-quiescent legacy state the shared recovery reconciled before the
/// upgrade. Any other version is reported as unsupported, never reinterpreted.
const PROOF_SCHEMA_VERSION: u32 = 2;
/// One visible task event per committed upgrade (same transaction as the proof).
const UPGRADE_EVENT_TYPE: &str = "nisar_budget_upgrade";
const UPGRADE_EVENT_CONTRACT: &str = "nisar_budget_upgrade_event_v1";
const REFUSAL_CONTRACT: &str = "nisar_budget_upgrade_refusal_v1";
const CHECKPOINT_MAX_BYTES: usize = 16 * 1024 * 1024;
const OLD_ANCHOR_MAX_BYTES: usize = 64 * 1024;
const RESULT_MAX_BYTES: usize = 1024 * 1024;
// Raw progress rows are small telemetry; this keeps the escaped proof total
// within the existing 68 MiB bound (2 x 16 MiB + 64 KiB + 1 MiB, escaped).
const PROGRESS_MAX_BYTES: usize = 64 * 1024;
const PROGRESS_TEXT_MAX_BYTES: usize = 64;
const PROOF_MAX_BYTES: usize = 68 * 1024 * 1024;
const REFUSAL_MAX_BYTES: usize = 4 * 1024 * 1024;
const REFUSAL_FAILURE_CODE_MAX_BYTES: usize = 512;
const REFUSAL_FAILURE_REASON_MAX_BYTES: usize = 64 * 1024;

pub(crate) const SCHEMA_SQL: &str = r#"
CREATE TABLE nisar_budget_upgrade_v2 (
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE RESTRICT,
    item_id TEXT NOT NULL CHECK(length(CAST(item_id AS BLOB)) BETWEEN 1 AND 512),
    proof_sha256 TEXT NOT NULL CHECK(length(proof_sha256) = 64 AND proof_sha256 NOT GLOB '*[^0-9a-f]*'),
    canonical_json TEXT NOT NULL CHECK(json_valid(canonical_json) AND length(CAST(canonical_json AS BLOB)) BETWEEN 2 AND 71303168),
    PRIMARY KEY(task_id, item_id)
) STRICT;
CREATE TRIGGER nisar_budget_upgrade_is_immutable
BEFORE UPDATE ON nisar_budget_upgrade_v2
BEGIN SELECT RAISE(ABORT, 'NISAR budget upgrade proof is immutable'); END;
CREATE TRIGGER nisar_budget_upgrade_cannot_be_deleted
BEFORE DELETE ON nisar_budget_upgrade_v2
BEGIN SELECT RAISE(ABORT, 'NISAR budget upgrade proof must be retained'); END;
"#;

/// Schema v30: immutable evidence for each refused upgrade. Before a visible
/// local failure replaces an item's download result, the exact replaced result
/// and progress rows (or their absence) are retained here. Idempotent DDL keeps
/// historical-version fixtures that predate this table migratable.
pub(crate) const REFUSAL_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS nisar_budget_upgrade_refusal_v2 (
    event_id TEXT PRIMARY KEY NOT NULL CHECK(length(CAST(event_id AS BLOB)) BETWEEN 1 AND 512),
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE RESTRICT,
    item_id TEXT NOT NULL CHECK(length(CAST(item_id AS BLOB)) BETWEEN 1 AND 512),
    refusal_sha256 TEXT NOT NULL CHECK(length(refusal_sha256) = 64 AND refusal_sha256 NOT GLOB '*[^0-9a-f]*'),
    canonical_json TEXT NOT NULL CHECK(json_valid(canonical_json) AND length(CAST(canonical_json AS BLOB)) BETWEEN 2 AND 4194304)
) STRICT;
CREATE INDEX IF NOT EXISTS nisar_budget_upgrade_refusal_item_idx
ON nisar_budget_upgrade_refusal_v2(task_id, item_id);
CREATE TRIGGER IF NOT EXISTS nisar_budget_upgrade_refusal_is_immutable
BEFORE UPDATE ON nisar_budget_upgrade_refusal_v2
BEGIN SELECT RAISE(ABORT, 'NISAR budget upgrade refusal evidence is immutable'); END;
CREATE TRIGGER IF NOT EXISTS nisar_budget_upgrade_refusal_cannot_be_deleted
BEFORE DELETE ON nisar_budget_upgrade_refusal_v2
BEGIN SELECT RAISE(ABORT, 'NISAR budget upgrade refusal evidence must be retained'); END;
"#;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarBudgetUpgradeLimitsV2 {
    pub max_request_count: u64,
    pub max_science_request_count: u64,
    pub max_response_body_bytes: u64,
}

/// Read through [`PresenceFields`], written as an internally tagged enum.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "presence", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoreNisarBudgetUpgradeDownloadResultV2 {
    Absent,
    Present {
        outcome: String,
        finished_at: String,
        result_json: String,
        last_known_total_bytes: Option<u64>,
        bytes_written: u64,
    },
}

/// The exact download_progress row, or its explicit absence. A row left by an
/// interrupted earlier process is retained as evidence; it is not rewritten or
/// deleted by the upgrade and later ordinary execution owns it as before.
/// Read through [`PresenceFields`], written as an internally tagged enum.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "presence", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoreNisarBudgetUpgradeDownloadProgressV2 {
    Absent,
    Present {
        status: String,
        updated_at: String,
        progress_json: String,
        bytes_completed: u64,
        total_bytes: Option<u64>,
        bytes_per_second: f64,
        last_known_total_bytes: Option<u64>,
    },
}

// L3 gate (2026-10-19): serde buffers the fields of an internally tagged enum
// in its private content type, which cannot hold a number while serde_json's
// arbitrary_precision feature is enabled; a build that includes
// insar-subset-lab enables that feature for every crate, and a proof or
// refusal retaining a present row then failed to parse. These two enums are
// read through serde_json::Value, which accepts the same JSON with and without
// the feature; their serialization above is unchanged.
impl<'de> Deserialize<'de> for StoreNisarBudgetUpgradeDownloadResultV2 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = PresenceFields::read(deserializer)?;
        let value = match fields.presence.as_str() {
            "absent" => Self::Absent,
            "present" => Self::Present {
                outcome: fields.string("outcome")?,
                finished_at: fields.string("finished_at")?,
                result_json: fields.string("result_json")?,
                last_known_total_bytes: fields.optional_u64("last_known_total_bytes")?,
                bytes_written: fields.u64("bytes_written")?,
            },
            _ => return Err(serde::de::Error::custom("unknown presence")),
        };
        fields.finish()?;
        Ok(value)
    }
}

impl<'de> Deserialize<'de> for StoreNisarBudgetUpgradeDownloadProgressV2 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = PresenceFields::read(deserializer)?;
        let value = match fields.presence.as_str() {
            "absent" => Self::Absent,
            "present" => Self::Present {
                status: fields.string("status")?,
                updated_at: fields.string("updated_at")?,
                progress_json: fields.string("progress_json")?,
                bytes_completed: fields.u64("bytes_completed")?,
                total_bytes: fields.optional_u64("total_bytes")?,
                bytes_per_second: fields.f64("bytes_per_second")?,
                last_known_total_bytes: fields.optional_u64("last_known_total_bytes")?,
            },
            _ => return Err(serde::de::Error::custom("unknown presence")),
        };
        fields.finish()?;
        Ok(value)
    }
}

/// The members of a presence-tagged object, each taken once with its exact
/// type; members left over are unknown fields.
struct PresenceFields {
    presence: String,
    members: serde_json::Map<String, Value>,
}

impl PresenceFields {
    fn read<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let Value::Object(mut members) = Value::deserialize(deserializer)? else {
            return Err(D::Error::custom("expected a presence-tagged object"));
        };
        let Some(Value::String(presence)) = members.remove("presence") else {
            return Err(D::Error::custom("missing presence"));
        };
        Ok(Self { presence, members })
    }

    fn string<E: serde::de::Error>(&mut self, name: &str) -> Result<String, E> {
        match self.members.remove(name) {
            Some(Value::String(value)) => Ok(value),
            _ => Err(E::custom(format!("invalid or missing field {name}"))),
        }
    }

    fn u64<E: serde::de::Error>(&mut self, name: &str) -> Result<u64, E> {
        self.members
            .remove(name)
            .as_ref()
            .and_then(Value::as_u64)
            .ok_or_else(|| E::custom(format!("invalid or missing field {name}")))
    }

    /// Absent or null is `None`, as for a derived `Option` field.
    fn optional_u64<E: serde::de::Error>(&mut self, name: &str) -> Result<Option<u64>, E> {
        match self.members.remove(name) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| E::custom(format!("invalid field {name}"))),
        }
    }

    fn f64<E: serde::de::Error>(&mut self, name: &str) -> Result<f64, E> {
        self.members
            .remove(name)
            .as_ref()
            .and_then(Value::as_f64)
            .ok_or_else(|| E::custom(format!("invalid or missing field {name}")))
    }

    fn finish<E: serde::de::Error>(self) -> Result<(), E> {
        match self.members.keys().next() {
            None => Ok(()),
            Some(name) => Err(E::custom(format!("unknown field {name}"))),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StoreNisarBudgetUpgradeContextV2 {
    pub task_id: String,
    pub item_id: String,
    /// Task-level revisions observed when the context was captured. They are
    /// retained as observations only: every progress, result or lifecycle write
    /// of a sibling item advances them, so admission compares this item's own
    /// rows instead (see `same_item_admission_facts`).
    pub runtime_revision: u64,
    pub download_revision: u64,
    pub item: SnapshotItem,
    pub science_anchor: StoredNisarSubsetExecutionAnchorV2,
    pub previous_result: StoreNisarBudgetUpgradeDownloadResultV2,
    pub previous_progress: StoreNisarBudgetUpgradeDownloadProgressV2,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoreNisarBudgetUpgradeEvidenceV2 {
    pub planning_checkpoint_canonical_json: String,
    pub science_checkpoint_canonical_json: String,
    pub old_limits: NisarBudgetUpgradeLimitsV2,
    pub new_limits: NisarBudgetUpgradeLimitsV2,
    pub completed_ranges_sha256: String,
    pub remaining_ranges_sha256: String,
    /// The exact legacy state before reconciliation, or absent when the
    /// predecessor was already quiescent and nothing had to be settled.
    pub pre_reconciliation: Option<StoreNisarBudgetUpgradePreReconciliationV2>,
}

/// The raw non-quiescent legacy state the migration entry found before the
/// executor's shared recovery settled it: the exact Store anchor row (with any
/// pending transition) and the exact local checkpoint text (absent for a pending
/// genesis). Retained as provenance only; never replayed or used as authority.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoreNisarBudgetUpgradePreReconciliationV2 {
    pub science_anchor_record_sha256: String,
    pub science_anchor_canonical_json: String,
    pub local_checkpoint_canonical_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ProofRecord {
    contract: String,
    schema_version: u32,
    context: StoreNisarBudgetUpgradeContextV2,
    occurred_at: String,
    planning_canonical_sha256: String,
    science_canonical_sha256: String,
    evidence: StoreNisarBudgetUpgradeEvidenceV2,
}

/// Immutable provenance, not task state or permission to skip scientific validation.
/// Only the locked download path may produce a migration after verifying material.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredNisarBudgetUpgradeProofV2 {
    record: ProofRecord,
    proof_sha256: String,
}

impl StoredNisarBudgetUpgradeProofV2 {
    pub fn new(
        context: StoreNisarBudgetUpgradeContextV2,
        occurred_at: String,
        evidence: StoreNisarBudgetUpgradeEvidenceV2,
    ) -> Result<Self, StoreError> {
        let record = ProofRecord {
            contract: PROOF_CONTRACT.to_owned(),
            schema_version: PROOF_SCHEMA_VERSION,
            context,
            occurred_at,
            planning_canonical_sha256: digest(
                evidence.planning_checkpoint_canonical_json.as_bytes(),
            ),
            science_canonical_sha256: digest(evidence.science_checkpoint_canonical_json.as_bytes()),
            evidence,
        };
        validate_record(&record)?;
        let canonical = bounded_canonical(&record)?;
        Ok(Self {
            record,
            proof_sha256: digest(canonical.as_bytes()),
        })
    }

    pub fn context(&self) -> &StoreNisarBudgetUpgradeContextV2 {
        &self.record.context
    }
    pub fn evidence(&self) -> &StoreNisarBudgetUpgradeEvidenceV2 {
        &self.record.evidence
    }
    pub fn proof_sha256(&self) -> &str {
        &self.proof_sha256
    }
    pub fn canonical_json(&self) -> Result<String, StoreError> {
        bounded_canonical(&self.record)
    }

    fn parse(canonical: &str, expected_digest: &str) -> Result<Self, StoreError> {
        if canonical.len() > PROOF_MAX_BYTES
            || !valid_lowercase_sha256(expected_digest)
            || digest(canonical.as_bytes()) != expected_digest
        {
            return Err(invalid());
        }
        let record: ProofRecord = match serde_json::from_str(canonical) {
            Ok(record) => record,
            Err(_) => return Err(unsupported_proof_version(canonical).unwrap_or_else(invalid)),
        };
        if record.schema_version != PROOF_SCHEMA_VERSION {
            return Err(StoreError::NisarBudgetUpgradeProofVersionUnsupported {
                found: u64::from(record.schema_version),
            });
        }
        validate_record(&record)?;
        if bounded_canonical(&record)? != canonical {
            return Err(invalid());
        }
        Ok(Self {
            record,
            proof_sha256: expected_digest.to_owned(),
        })
    }
}

/// A digest-intact proof of this contract written by another program version
/// gets an explicit local diagnostic instead of a generic invalid-anchor error.
fn unsupported_proof_version(canonical: &str) -> Option<StoreError> {
    let header: Value = serde_json::from_str(canonical).ok()?;
    if header.get("contract")?.as_str()? != PROOF_CONTRACT {
        return None;
    }
    let found = header.get("schema_version")?.as_u64()?;
    (found != u64::from(PROOF_SCHEMA_VERSION))
        .then_some(StoreError::NisarBudgetUpgradeProofVersionUnsupported { found })
}

fn invalid() -> StoreError {
    StoreError::NisarSubsetExecutionAnchorInvalid
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn bounded_canonical(record: &ProofRecord) -> Result<String, StoreError> {
    bounded_serialization(record, PROOF_MAX_BYTES)
}

/// Serialize without ever materializing more than `limit` bytes; oversized
/// evidence is rejected, never truncated.
fn bounded_serialization(value: &impl Serialize, limit: usize) -> Result<String, StoreError> {
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self
                .bytes
                .len()
                .checked_add(bytes.len())
                .is_none_or(|n| n > self.limit)
            {
                return Err(std::io::Error::other(
                    "NISAR budget evidence exceeds its byte limit",
                ));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut output, value).map_err(|_| invalid())?;
    String::from_utf8(output.bytes).map_err(|_| invalid())
}

fn bounded_json(text: &str, limit: usize) -> Result<Value, StoreError> {
    if text.len() < 2 || text.len() > limit || !compact_json_has_no_insignificant_whitespace(text) {
        return Err(invalid());
    }
    serde_json::from_str(text).map_err(|_| invalid())
}

fn validate_result(context: &StoreNisarBudgetUpgradeContextV2) -> Result<(), StoreError> {
    validate_result_row(
        &context.task_id,
        &context.item_id,
        &context.previous_result,
        true,
    )
}

/// `resumable_only` restricts an upgrade predecessor to a failed, interrupted or
/// cancelled item that was continued again; refusal evidence retains whatever
/// exact row a refusal replaces.
fn validate_result_row(
    task_id: &str,
    item_id: &str,
    row: &StoreNisarBudgetUpgradeDownloadResultV2,
    resumable_only: bool,
) -> Result<(), StoreError> {
    if let StoreNisarBudgetUpgradeDownloadResultV2::Present {
        outcome,
        finished_at,
        result_json,
        bytes_written,
        ..
    } = row
    {
        if result_json.len() > RESULT_MAX_BYTES {
            return Err(invalid());
        }
        let result: insar_core::DownloadItemResult =
            serde_json::from_str(result_json).map_err(|_| invalid())?;
        result.validate().map_err(StoreError::invalid)?;
        if result.task_id != task_id
            || result.item_id != item_id
            || result.finished_at != *finished_at
            || result.bytes_written != *bytes_written
            || serde_json::to_value(&result.outcome).map_err(|_| invalid())? != json!(outcome)
            || (resumable_only
                && !matches!(
                    result.outcome,
                    insar_core::DownloadOutcome::Failed
                        | insar_core::DownloadOutcome::Interrupted
                        | insar_core::DownloadOutcome::Cancelled
                ))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn validate_progress_row(
    task_id: &str,
    item_id: &str,
    row: &StoreNisarBudgetUpgradeDownloadProgressV2,
) -> Result<(), StoreError> {
    if let StoreNisarBudgetUpgradeDownloadProgressV2::Present {
        status,
        updated_at,
        progress_json,
        bytes_per_second,
        ..
    } = row
    {
        if progress_json.len() < 2
            || progress_json.len() > PROGRESS_MAX_BYTES
            || status.is_empty()
            || status.len() > PROGRESS_TEXT_MAX_BYTES
            || updated_at.is_empty()
            || updated_at.len() > PROGRESS_TEXT_MAX_BYTES
            || !bytes_per_second.is_finite()
            || *bytes_per_second < 0.0
        {
            return Err(invalid());
        }
        let progress: insar_core::DownloadItemProgress =
            serde_json::from_str(progress_json).map_err(|_| invalid())?;
        progress.validate().map_err(StoreError::invalid)?;
        if progress.task_id != task_id || progress.item_id != item_id {
            return Err(invalid());
        }
    }
    Ok(())
}

fn load_raw_result(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
) -> Result<StoreNisarBudgetUpgradeDownloadResultV2, StoreError> {
    let result_size: Option<i64> = connection
        .query_row(
            "SELECT length(CAST(result_json AS BLOB)) FROM download_results WHERE task_id=?1 AND item_id=?2",
            params![task_id, item_id],
            |r| r.get(0),
        )
        .optional()?;
    if result_size.is_some_and(|n| n < 0 || n as u64 > RESULT_MAX_BYTES as u64) {
        return Err(invalid());
    }
    let raw = connection.query_row("SELECT outcome,finished_at,result_json,last_known_total_bytes,bytes_written FROM download_results WHERE task_id=?1 AND item_id=?2", params![task_id,item_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<i64>>(3)?,r.get::<_,i64>(4)?))).optional()?;
    Ok(match raw {
        None => StoreNisarBudgetUpgradeDownloadResultV2::Absent,
        Some((outcome, finished_at, result_json, total, bytes)) => {
            StoreNisarBudgetUpgradeDownloadResultV2::Present {
                outcome,
                finished_at,
                result_json,
                last_known_total_bytes: total
                    .map(|n| u64::try_from(n).map_err(|_| invalid()))
                    .transpose()?,
                bytes_written: u64::try_from(bytes).map_err(|_| invalid())?,
            }
        }
    })
}

fn load_raw_progress(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
) -> Result<StoreNisarBudgetUpgradeDownloadProgressV2, StoreError> {
    let progress_size: Option<i64> = connection
        .query_row(
            "SELECT length(CAST(progress_json AS BLOB)) FROM download_progress WHERE task_id=?1 AND item_id=?2",
            params![task_id, item_id],
            |r| r.get(0),
        )
        .optional()?;
    if progress_size.is_some_and(|n| n < 0 || n as u64 > PROGRESS_MAX_BYTES as u64) {
        return Err(invalid());
    }
    let raw = connection
        .query_row(
            "SELECT status,updated_at,progress_json,bytes_completed,total_bytes,bytes_per_second,last_known_total_bytes              FROM download_progress WHERE task_id=?1 AND item_id=?2",
            params![task_id, item_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, f64>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .optional()?;
    let non_negative = |value: i64| u64::try_from(value).map_err(|_| invalid());
    Ok(match raw {
        None => StoreNisarBudgetUpgradeDownloadProgressV2::Absent,
        Some((status, updated_at, progress_json, completed, total, rate, last_known)) => {
            StoreNisarBudgetUpgradeDownloadProgressV2::Present {
                status,
                updated_at,
                progress_json,
                bytes_completed: non_negative(completed)?,
                total_bytes: total.map(non_negative).transpose()?,
                bytes_per_second: rate,
                last_known_total_bytes: last_known.map(non_negative).transpose()?,
            }
        }
    })
}

fn validate_record(record: &ProofRecord) -> Result<(), StoreError> {
    let context = &record.context;
    let evidence = &record.evidence;
    let anchor = &context.science_anchor;
    validate_nisar_subset_execution_anchor_identity(
        &context.task_id,
        &context.item_id,
        &anchor.anchor_kind,
    )?;
    if record.contract != PROOF_CONTRACT
        || record.schema_version != PROOF_SCHEMA_VERSION
        || crate::canonical_utc_identity_timestamp(&record.occurred_at).ok_or_else(invalid)?
            != record.occurred_at
        || anchor.task_id != context.task_id
        || anchor.item_id != context.item_id
        || anchor.anchor_kind != NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
        || context.item.item_id != context.item_id
        || anchor.canonical_json.len() > OLD_ANCHOR_MAX_BYTES
        || !valid_lowercase_sha256(&evidence.completed_ranges_sha256)
        || !valid_lowercase_sha256(&evidence.remaining_ranges_sha256)
    {
        return Err(invalid());
    }
    crate::core_restore::validate_nisar_science_anchor_canonical_v2(
        &anchor.lineage_key_sha256,
        &anchor.record_sha256,
        &anchor.canonical_json,
    )?;
    validate_result(context)?;
    validate_progress_row(
        &context.task_id,
        &context.item_id,
        &context.previous_progress,
    )?;
    if let Some(pre) = &evidence.pre_reconciliation {
        validate_pre_reconciliation(context, pre)?;
    }
    let old_anchor = bounded_json(&anchor.canonical_json, OLD_ANCHOR_MAX_BYTES)?;
    let planning = bounded_json(
        &evidence.planning_checkpoint_canonical_json,
        CHECKPOINT_MAX_BYTES,
    )?;
    let science = bounded_json(
        &evidence.science_checkpoint_canonical_json,
        CHECKPOINT_MAX_BYTES,
    )?;
    let intent = crate::parse_nisar_subset_prepared_intent_v2_envelope(&context.item.metadata)?
        .ok_or_else(invalid)?;
    let plan =
        crate::parse_nisar_subset_plan_v2_envelope(&context.item.metadata)?.ok_or_else(invalid)?;
    let old = &evidence.old_limits;
    let new = &evidence.new_limits;
    let budget = &planning["physical_plan"]["combined_budget"];
    if record.planning_canonical_sha256
        != digest(evidence.planning_checkpoint_canonical_json.as_bytes())
        || record.science_canonical_sha256
            != digest(evidence.science_checkpoint_canonical_json.as_bytes())
        || old_anchor.get("pending_transition") != Some(&Value::Null)
        || science.get("pending_attempt") != Some(&Value::Null)
        || science.get("budget_upgrade_receipt").is_some()
        || !matches!(
            science["state"].as_str(),
            Some("Planned" | "InProgress" | "Interrupted")
        )
        || science["task_id"] != json!(context.task_id)
        || science["item_id"] != json!(context.item_id)
        || planning["task_id"] != science["task_id"]
        || planning["item_id"] != science["item_id"]
        || science["prepared_intent_sha256"] != json!(intent.canonical_intent_sha256)
        || science["final_plan_sha256"] != json!(plan.canonical_semantic_plan_digest)
        || science["physical_plan_sha256"] != planning["physical_plan"]["physical_plan_sha256"]
        || science["lineage_key_sha256"] != json!(anchor.lineage_key_sha256)
        || science["execution_binding_sha256"] != old_anchor["execution_binding_sha256"]
        || science["anchor_generation"] != old_anchor["generation"]
        || science["checkpoint_sha256"] != old_anchor["committed_checkpoint_sha256"]
        || budget["budget_policy"] != json!("physical_execution_union_v1")
        || budget["max_request_count"] != json!(old.max_request_count)
        || budget["remaining_science_request_count"] != json!(old.max_science_request_count)
        || budget["max_response_body_bytes"] != json!(old.max_response_body_bytes)
        || old.max_request_count < old.max_science_request_count
        || old.max_science_request_count == 0
        || new.max_request_count < new.max_science_request_count
        || new.max_request_count - new.max_science_request_count
            != old.max_request_count - old.max_science_request_count
        || new.max_science_request_count < old.max_science_request_count
        || new.max_response_body_bytes < old.max_response_body_bytes
    {
        return Err(invalid());
    }
    Ok(())
}

/// The Store cannot judge scientific bytes; it checks that retained
/// pre-reconciliation evidence is a digest-bound anchor row of the same lineage,
/// a bounded checkpoint text of the same item, and actually non-quiescent.
fn validate_pre_reconciliation(
    context: &StoreNisarBudgetUpgradeContextV2,
    pre: &StoreNisarBudgetUpgradePreReconciliationV2,
) -> Result<(), StoreError> {
    crate::core_restore::validate_nisar_science_anchor_canonical_v2(
        &context.science_anchor.lineage_key_sha256,
        &pre.science_anchor_record_sha256,
        &pre.science_anchor_canonical_json,
    )?;
    let anchor: Value =
        serde_json::from_str(&pre.science_anchor_canonical_json).map_err(|_| invalid())?;
    let local = pre
        .local_checkpoint_canonical_json
        .as_deref()
        .map(|text| bounded_json(text, CHECKPOINT_MAX_BYTES))
        .transpose()?;
    if local.as_ref().is_some_and(|local| {
        local["task_id"] != json!(context.task_id) || local["item_id"] != json!(context.item_id)
    }) {
        return Err(invalid());
    }
    let quiescent = anchor.get("pending_transition") == Some(&Value::Null)
        && local
            .as_ref()
            .is_some_and(|local| local.get("pending_attempt") == Some(&Value::Null));
    // Reconciliation always commits a new anchor record before the upgrade.
    if quiescent || pre.science_anchor_record_sha256 == context.science_anchor.record_sha256 {
        return Err(invalid());
    }
    Ok(())
}

fn load_anchor(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
) -> Result<StoredNisarSubsetExecutionAnchorV2, StoreError> {
    connection.query_row(
        "SELECT lineage_key_sha256,record_sha256,canonical_json FROM nisar_subset_execution_anchors_v2 WHERE task_id=?1 AND item_id=?2 AND anchor_kind=?3 AND length(CAST(canonical_json AS BLOB))<=65536",
        params![task_id, item_id, NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND],
        |row| Ok(StoredNisarSubsetExecutionAnchorV2 { task_id: task_id.to_owned(), item_id: item_id.to_owned(),
            anchor_kind: NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND.to_owned(), lineage_key_sha256: row.get(0)?,
            record_sha256: row.get(1)?, canonical_json: row.get(2)? }),
    ).optional()?.ok_or_else(invalid)
}

fn load_context(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
) -> Result<StoreNisarBudgetUpgradeContextV2, StoreError> {
    validate_nisar_subset_execution_anchor_identity(
        task_id,
        item_id,
        NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
    )?;
    if !crate::core_restore::task_is_actionable(connection, task_id)? {
        return Err(StoreError::NotFound);
    }
    let task = load_task_connection(connection, task_id)?.ok_or(StoreError::NotFound)?;
    if task.runtime.status != TaskLifecycleStatus::Running
        || task.runtime.task_paused
        || !task.runtime.active_item_ids.iter().any(|id| id == item_id)
        || !task
            .runtime
            .selected_item_ids
            .iter()
            .any(|id| id == item_id)
    {
        return Err(invalid());
    }
    let item = task
        .snapshot
        .candidates
        .iter()
        .find(|item| item.item_id == item_id)
        .ok_or(StoreError::NotFound)?
        .clone();
    // Writer authority still forbids an upgrade. A progress row is retained
    // exactly in the context instead; prepare refuses if it changes.
    let writer_authority: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM nisar_subset_execution_anchors_v2 WHERE task_id=?1 AND item_id=?2 AND anchor_kind=?3)",
        params![task_id, item_id, NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND], |row| row.get(0))?;
    if writer_authority {
        return Err(invalid());
    }
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM download_engine_state WHERE task_id=?1",
            [task_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let context = StoreNisarBudgetUpgradeContextV2 {
        task_id: task_id.to_owned(),
        item_id: item_id.to_owned(),
        runtime_revision: task.runtime.revision,
        download_revision: u64::try_from(revision).map_err(|_| invalid())?,
        item,
        science_anchor: load_anchor(connection, task_id, item_id)?,
        previous_result: load_raw_result(connection, task_id, item_id)?,
        previous_progress: load_raw_progress(connection, task_id, item_id)?,
    };
    validate_result(&context)?;
    validate_progress_row(task_id, item_id, &context.previous_progress)?;
    Ok(context)
}

/// Admission compares only this item's own durable facts. Task-level runtime and
/// download revisions stay in the context as observations, but a sibling item
/// advances them with every progress, result or lifecycle write, so they are
/// never an equality gate. `load_context` has already rechecked this item's
/// lifecycle in the same transaction (Running, unpaused, active and selected,
/// no writer authority).
fn same_item_admission_facts(
    current: &StoreNisarBudgetUpgradeContextV2,
    captured: &StoreNisarBudgetUpgradeContextV2,
) -> bool {
    let StoreNisarBudgetUpgradeContextV2 {
        task_id,
        item_id,
        runtime_revision: _,
        download_revision: _,
        item,
        science_anchor,
        previous_result,
        previous_progress,
    } = captured;
    current.task_id == *task_id
        && current.item_id == *item_id
        && current.item == *item
        && current.science_anchor == *science_anchor
        && current.previous_result == *previous_result
        && current.previous_progress == *previous_progress
}

/// Records the committed upgrade as one task event. Task events are sequenced
/// by the runtime revision, so the event advances it exactly like any recorded
/// task transition; the runtime state is otherwise unchanged.
fn record_upgrade_event(
    connection: &Connection,
    proof: &StoredNisarBudgetUpgradeProofV2,
) -> Result<(), StoreError> {
    let context = proof.context();
    let mut task =
        load_task_connection(connection, &context.task_id)?.ok_or(StoreError::NotFound)?;
    let runtime = &mut task.runtime;
    let expected = runtime.revision;
    let revision = expected
        .checked_add(1)
        .ok_or_else(|| StoreError::invalid("task revision overflow"))?;
    runtime.revision = revision;
    if proof.record.occurred_at > runtime.updated_at {
        runtime.updated_at = proof.record.occurred_at.clone();
    }
    runtime.validate().map_err(StoreError::invalid)?;
    let runtime_json = serde_json::to_string(&*runtime).map_err(StoreError::Corrupt)?;
    let revision_sql = sqlite_integer(revision, "task revision")?;
    let changed = connection.execute(
        "UPDATE task_runtime SET revision=?1, updated_at=?2, state_json=?3 WHERE task_id=?4 AND revision=?5",
        params![
            revision_sql,
            runtime.updated_at,
            runtime_json,
            context.task_id,
            sqlite_integer(expected, "expected task revision")?
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::RevisionConflict);
    }
    let payload = json!({
        "contract": UPGRADE_EVENT_CONTRACT,
        "item_id": context.item_id,
        "proof_sha256": proof.proof_sha256,
        "old_limits": proof.evidence().old_limits,
        "new_limits": proof.evidence().new_limits,
    });
    connection.execute(
        "INSERT INTO task_events(event_id,task_id,sequence,occurred_at,event_type,item_id,payload_json,error_json) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL)",
        params![
            format!("nisar-budget-upgrade:{}", proof.proof_sha256),
            context.task_id,
            revision_sql,
            proof.record.occurred_at,
            UPGRADE_EVENT_TYPE,
            context.item_id,
            payload.to_string()
        ],
    )?;
    Ok(())
}

fn validate_next(
    proof: &StoredNisarBudgetUpgradeProofV2,
    next: &StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request,
) -> Result<(), StoreError> {
    let context = proof.context();
    let old_anchor = &context.science_anchor;
    if next.task_id != context.task_id
        || next.item_id != context.item_id
        || next.anchor_kind != old_anchor.anchor_kind
        || next.lineage_key_sha256 != old_anchor.lineage_key_sha256
        || next.expected_record_sha256.as_deref() != Some(&old_anchor.record_sha256)
    {
        return Err(invalid());
    }
    crate::core_restore::validate_nisar_science_anchor_canonical_v2(
        &next.lineage_key_sha256,
        &next.record_sha256,
        &next.canonical_json,
    )?;
    let old_record: Value =
        serde_json::from_str(&old_anchor.canonical_json).map_err(|_| invalid())?;
    let next_record: Value = serde_json::from_str(&next.canonical_json).map_err(|_| invalid())?;
    let pending = &next_record["pending_transition"];
    let old_science = bounded_json(
        &proof.evidence().science_checkpoint_canonical_json,
        CHECKPOINT_MAX_BYTES,
    )?;
    let target_text = pending["checkpoint_canonical_json"]
        .as_str()
        .ok_or_else(invalid)?;
    let mut target = bounded_json(target_text, CHECKPOINT_MAX_BYTES)?;
    if pending["predecessor_checkpoint_sha256"] != old_science["checkpoint_sha256"]
        || pending["predecessor_generation"] != old_science["anchor_generation"]
        || next_record["generation"] != old_record["generation"]
        || next_record["execution_binding_sha256"] != old_record["execution_binding_sha256"]
        || next_record["committed_checkpoint_sha256"] != old_record["committed_checkpoint_sha256"]
        || next_record["committed_recovery_telemetry"] != old_record["committed_recovery_telemetry"]
        || target["anchor_generation"] != pending["checkpoint_generation"]
        || target["checkpoint_sha256"] != pending["checkpoint_sha256"]
        || target["previous_checkpoint_sha256"] != old_science["checkpoint_sha256"]
        || target["budget_upgrade_receipt"]
            != json!({"contract":"nisar_science_budget_upgrade_v1",
            "proof_sha256":proof.proof_sha256(),"limits":proof.evidence().new_limits})
    {
        return Err(invalid());
    }
    let target_object = target.as_object_mut().ok_or_else(invalid)?;
    target_object.remove("budget_upgrade_receipt");
    for key in [
        "anchor_generation",
        "previous_checkpoint_sha256",
        "checkpoint_sha256",
    ] {
        match old_science.get(key) {
            Some(value) => {
                target_object.insert(key.to_owned(), value.clone());
            }
            None => {
                target_object.remove(key);
            }
        }
    }
    if target != old_science {
        return Err(invalid());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RefusalRecord {
    contract: String,
    schema_version: u32,
    event_id: String,
    task_id: String,
    item_id: String,
    occurred_at: String,
    failure: insar_core::DownloadFailure,
    previous_result: StoreNisarBudgetUpgradeDownloadResultV2,
    previous_progress: StoreNisarBudgetUpgradeDownloadProgressV2,
}

/// Immutable evidence of one refused upgrade: the visible local failure and the
/// exact download rows it replaced. Not lifecycle state and not a retry token.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredNisarBudgetUpgradeRefusalV2 {
    record: RefusalRecord,
    refusal_sha256: String,
}

impl StoredNisarBudgetUpgradeRefusalV2 {
    pub fn event_id(&self) -> &str {
        &self.record.event_id
    }
    pub fn occurred_at(&self) -> &str {
        &self.record.occurred_at
    }
    pub fn failure(&self) -> &insar_core::DownloadFailure {
        &self.record.failure
    }
    pub fn previous_result(&self) -> &StoreNisarBudgetUpgradeDownloadResultV2 {
        &self.record.previous_result
    }
    pub fn previous_progress(&self) -> &StoreNisarBudgetUpgradeDownloadProgressV2 {
        &self.record.previous_progress
    }
    pub fn refusal_sha256(&self) -> &str {
        &self.refusal_sha256
    }

    fn parse(canonical: &str, expected_digest: &str) -> Result<Self, StoreError> {
        if canonical.len() > REFUSAL_MAX_BYTES
            || !valid_lowercase_sha256(expected_digest)
            || digest(canonical.as_bytes()) != expected_digest
        {
            return Err(invalid());
        }
        let record: RefusalRecord = serde_json::from_str(canonical).map_err(|_| invalid())?;
        validate_refusal(&record)?;
        if bounded_serialization(&record, REFUSAL_MAX_BYTES)? != canonical {
            return Err(invalid());
        }
        Ok(Self {
            record,
            refusal_sha256: expected_digest.to_owned(),
        })
    }
}

fn validate_refusal(record: &RefusalRecord) -> Result<(), StoreError> {
    validate_nisar_subset_execution_anchor_identity(
        &record.task_id,
        &record.item_id,
        NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
    )?;
    record.failure.validate().map_err(StoreError::invalid)?;
    if record.contract != REFUSAL_CONTRACT
        || record.schema_version != 1
        || record.event_id.is_empty()
        || record.event_id.len() > 512
        || record.occurred_at.trim().is_empty()
        || record.occurred_at.len() > 64
        || record.failure.code.len() > REFUSAL_FAILURE_CODE_MAX_BYTES
        || record.failure.reason_zh.len() > REFUSAL_FAILURE_REASON_MAX_BYTES
    {
        return Err(invalid());
    }
    validate_result_row(
        &record.task_id,
        &record.item_id,
        &record.previous_result,
        false,
    )?;
    validate_progress_row(&record.task_id, &record.item_id, &record.previous_progress)
}

/// Inside the caller's IMMEDIATE runtime transaction: retain the exact rows the
/// visible refusal result is about to replace, then persist that result with the
/// ordinary download-result semantics (revision, totals, progress removal).
pub(crate) fn record_refusal_and_visible_result(
    connection: &Connection,
    event_id: &str,
    occurred_at: &str,
    result: &insar_core::DownloadItemResult,
) -> Result<(), StoreError> {
    let failure = result.failure.clone().ok_or_else(invalid)?;
    if result.outcome != insar_core::DownloadOutcome::Failed {
        return Err(invalid());
    }
    let record = RefusalRecord {
        contract: REFUSAL_CONTRACT.to_owned(),
        schema_version: 1,
        event_id: event_id.to_owned(),
        task_id: result.task_id.clone(),
        item_id: result.item_id.clone(),
        occurred_at: occurred_at.to_owned(),
        failure,
        previous_result: load_raw_result(connection, &result.task_id, &result.item_id)?,
        previous_progress: load_raw_progress(connection, &result.task_id, &result.item_id)?,
    };
    validate_refusal(&record)?;
    let canonical = bounded_serialization(&record, REFUSAL_MAX_BYTES)?;
    connection.execute(
        "INSERT INTO nisar_budget_upgrade_refusal_v2(event_id,task_id,item_id,refusal_sha256,canonical_json) VALUES (?1,?2,?3,?4,?5)",
        params![
            record.event_id,
            record.task_id,
            record.item_id,
            digest(canonical.as_bytes()),
            canonical
        ],
    )?;
    let expected_revision =
        crate::download::current_download_revision(connection, &result.task_id)?;
    crate::download::persist_download_result(connection, result, expected_revision, false)?;
    Ok(())
}

/// Inside the caller's IMMEDIATE runtime transaction, for a rejection whose
/// refusal evidence could not be retained (review 2026-10-17 round 2, P2-1):
/// replace nothing. Only an item without any download result or progress row
/// gets the visible failure result, with the ordinary result semantics.
pub(crate) fn show_unretained_rejection(
    connection: &Connection,
    result: &insar_core::DownloadItemResult,
) -> Result<(), StoreError> {
    if result.outcome != insar_core::DownloadOutcome::Failed || result.failure.is_none() {
        return Err(invalid());
    }
    let has_download_row: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM download_results WHERE task_id=?1 AND item_id=?2) \
             OR EXISTS(SELECT 1 FROM download_progress WHERE task_id=?1 AND item_id=?2)",
        params![result.task_id, result.item_id],
        |row| row.get(0),
    )?;
    if has_download_row {
        return Ok(());
    }
    let expected_revision =
        crate::download::current_download_revision(connection, &result.task_id)?;
    crate::download::persist_download_result(connection, result, expected_revision, false)?;
    Ok(())
}

/// Replay check: the refusal event either recorded evidence (failed) or was a
/// task-level pause conversion that intentionally replaced nothing.
pub(crate) fn refusal_recorded_for_event(
    connection: &Connection,
    event_id: &str,
) -> Result<Option<StoredNisarBudgetUpgradeRefusalV2>, StoreError> {
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT refusal_sha256,canonical_json FROM nisar_budget_upgrade_refusal_v2 WHERE event_id=?1",
            [event_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(sha, canonical)| StoredNisarBudgetUpgradeRefusalV2::parse(&canonical, &sha))
        .transpose()
}

impl TaskStore {
    /// Every retained refusal for one item, oldest first.
    pub fn list_nisar_budget_upgrade_refusals_v2(
        &self,
        task_id: &str,
        item_id: &str,
    ) -> Result<Vec<StoredNisarBudgetUpgradeRefusalV2>, StoreError> {
        validate_nisar_subset_execution_anchor_identity(
            task_id,
            item_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )?;
        let mut statement = self.connection.prepare(
            "SELECT refusal_sha256,canonical_json FROM nisar_budget_upgrade_refusal_v2              WHERE task_id=?1 AND item_id=?2 ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![task_id, item_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (sha, canonical) = row?;
            let refusal = StoredNisarBudgetUpgradeRefusalV2::parse(&canonical, &sha)?;
            if refusal.record.task_id != task_id || refusal.record.item_id != item_id {
                return Err(invalid());
            }
            Ok(refusal)
        })
        .collect()
    }

    pub fn load_nisar_budget_upgrade_context_v2(
        &self,
        task_id: &str,
        item_id: &str,
    ) -> Result<StoreNisarBudgetUpgradeContextV2, StoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let context = load_context(&transaction, task_id, item_id)?;
        transaction.commit()?;
        Ok(context)
    }

    pub fn load_nisar_budget_upgrade_proof_v2(
        &self,
        task_id: &str,
        item_id: &str,
    ) -> Result<Option<StoredNisarBudgetUpgradeProofV2>, StoreError> {
        validate_nisar_subset_execution_anchor_identity(
            task_id,
            item_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )?;
        let row: Option<(String,String)> = self.connection.query_row(
            "SELECT proof_sha256,canonical_json FROM nisar_budget_upgrade_v2 WHERE task_id=?1 AND item_id=?2",
            params![task_id,item_id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        row.map(|(sha, canonical)| {
            let proof = StoredNisarBudgetUpgradeProofV2::parse(&canonical, &sha)?;
            if proof.context().task_id != task_id || proof.context().item_id != item_id {
                return Err(invalid());
            }
            Ok(proof)
        })
        .transpose()
    }

    pub fn prepare_nisar_budget_upgrade_v2(
        &mut self,
        proof: &StoredNisarBudgetUpgradeProofV2,
        next: &StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request,
    ) -> Result<bool, StoreError> {
        validate_record(&proof.record)?;
        let canonical = proof.canonical_json()?;
        if digest(canonical.as_bytes()) != proof.proof_sha256 {
            return Err(invalid());
        }
        validate_next(proof, next)?;
        let context = proof.context();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM nisar_budget_upgrade_v2 WHERE task_id=?1 AND item_id=?2)",
            params![context.task_id, context.item_id],
            |r| r.get(0),
        )?;
        if exists {
            return Ok(false);
        }
        let current = load_context(&transaction, &context.task_id, &context.item_id)?;
        if !same_item_admission_facts(&current, context) {
            return Ok(false);
        }
        transaction.execute("INSERT INTO nisar_budget_upgrade_v2(task_id,item_id,proof_sha256,canonical_json) VALUES (?1,?2,?3,?4)", params![context.task_id,context.item_id,proof.proof_sha256,canonical])?;
        let changed = transaction.execute(
            "UPDATE nisar_subset_execution_anchors_v2 SET record_sha256=?1,canonical_json=?2 WHERE task_id=?3 AND item_id=?4 AND anchor_kind=?5 AND lineage_key_sha256=?6 AND record_sha256=?7 AND canonical_json=?8",
            params![next.record_sha256,next.canonical_json,context.task_id,context.item_id,next.anchor_kind,
                next.lineage_key_sha256,context.science_anchor.record_sha256,context.science_anchor.canonical_json])?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        record_upgrade_event(&transaction, proof)?;
        transaction.commit()?;
        Ok(true)
    }
}
