//! Durable, network-free state for one-shot GACOS web-form submissions.
//!
//! This module deliberately stores only the immutable request identity and a
//! small outcome ledger. It does not store an e-mail address, request body,
//! URL, credential, response body, or a fabricated task completion. The
//! caller must reserve an attempt before performing the non-idempotent POST;
//! an interrupted `reserved` or `submitting` attempt is recovered as
//! `submission_uncertain` and therefore requires manual verification.

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::{canonical_utc_identity_timestamp, StoreError, TaskStore};

pub const GACOS_SUBMISSION_PROVIDER_ID: &str = "gacos.webform";
pub const GACOS_SUBMISSION_OPERATION_KIND: &str = "gacos";
pub const GACOS_SUBMISSION_OPERATION_PHASE: &str = "request";
pub const GACOS_SUBMISSION_OPERATION_WORKER: &str = "gacos_manual_request";
pub const GACOS_MAX_ATTEMPT_HISTORY: u64 = 256;

/// Schema for the optional GACOS attempt ledger.
///
/// Store schema registration is intentionally left to the parent integration:
/// tests and migrations can execute this SQL explicitly on the existing Store
/// connection. There is no second database and no trigger that changes task
/// lifecycle state.
pub const GACOS_SUBMISSION_ATTEMPTS_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS gacos_submission_attempts (
    attempt_id TEXT PRIMARY KEY NOT NULL
        CHECK(length(attempt_id) BETWEEN 1 AND 256 AND trim(attempt_id) = attempt_id),
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE
        CHECK(length(task_id) BETWEEN 1 AND 128 AND trim(task_id) = task_id),
    item_id TEXT NOT NULL
        CHECK(length(item_id) BETWEEN 1 AND 2048 AND trim(item_id) = item_id),
    fingerprint_sha256 TEXT NOT NULL
        CHECK(length(fingerprint_sha256) = 64
          AND fingerprint_sha256 = lower(fingerprint_sha256)
          AND fingerprint_sha256 NOT GLOB '*[^0-9a-f]*'),
    account_identity_hash_sha256 TEXT NOT NULL
        CHECK(length(account_identity_hash_sha256) = 64
          AND account_identity_hash_sha256 = lower(account_identity_hash_sha256)
          AND account_identity_hash_sha256 NOT GLOB '*[^0-9a-f]*'),
    attempt_number INTEGER NOT NULL CHECK(attempt_number >= 0),
    state TEXT NOT NULL CHECK(state IN (
        'reserved', 'submitting', 'manual_verification_required',
        'submission_uncertain', 'submitted', 'rejected'
    )),
    reason_code TEXT CHECK(reason_code IS NULL OR (
        length(reason_code) BETWEEN 1 AND 128
        AND trim(reason_code) = reason_code
        AND reason_code NOT GLOB '*[^A-Za-z0-9_.:-]*'
    )),
    http_status INTEGER CHECK(http_status IS NULL OR http_status BETWEEN 100 AND 599),
    automatic_retry_allowed INTEGER NOT NULL DEFAULT 0
        CHECK(automatic_retry_allowed = 0),
    created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 20 AND 24),
    updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 20 AND 24),
    CHECK(
        (state IN ('reserved', 'submitting')
            AND reason_code IS NULL AND http_status IS NULL)
        OR (state = 'manual_verification_required'
            AND reason_code IS NOT NULL AND http_status BETWEEN 200 AND 299)
        OR (state = 'submission_uncertain'
            AND reason_code IS NOT NULL
            AND (http_status IS NULL OR http_status BETWEEN 300 AND 399 OR http_status = 408 OR http_status = 429
                 OR http_status BETWEEN 500 AND 599))
        OR (state = 'submitted'
            AND reason_code IS NOT NULL
            AND http_status IS NOT NULL
            AND http_status BETWEEN 200 AND 299)
        OR (state = 'rejected'
            AND reason_code IS NOT NULL
            AND (http_status IS NULL OR (
                http_status BETWEEN 100 AND 499
                AND http_status NOT IN (408, 429)
            )))
    )
) STRICT;

CREATE INDEX IF NOT EXISTS gacos_submission_attempts_task_item_order_idx
    ON gacos_submission_attempts(task_id, item_id, attempt_number, created_at, attempt_id);

CREATE INDEX IF NOT EXISTS gacos_submission_attempts_fingerprint_account_idx
    ON gacos_submission_attempts(fingerprint_sha256, account_identity_hash_sha256,
                                 created_at, attempt_id);
"#;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GacosSubmissionState {
    Reserved,
    Submitting,
    ManualVerificationRequired,
    SubmissionUncertain,
    Submitted,
    Rejected,
}

impl GacosSubmissionState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Submitting => "submitting",
            Self::ManualVerificationRequired => "manual_verification_required",
            Self::SubmissionUncertain => "submission_uncertain",
            Self::Submitted => "submitted",
            Self::Rejected => "rejected",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "reserved" => Self::Reserved,
            "submitting" => Self::Submitting,
            "manual_verification_required" => Self::ManualVerificationRequired,
            "submission_uncertain" => Self::SubmissionUncertain,
            "submitted" => Self::Submitted,
            "rejected" => Self::Rejected,
            _ => return None,
        })
    }

    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::ManualVerificationRequired
                | Self::SubmissionUncertain
                | Self::Submitted
                | Self::Rejected
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GacosSubmissionAttemptIdentity {
    pub task_id: String,
    pub item_id: String,
    pub attempt_id: String,
    pub fingerprint_sha256: String,
    pub account_identity_hash_sha256: String,
    pub attempt_number: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GacosSubmissionReserveRequest {
    pub identity: GacosSubmissionAttemptIdentity,
    pub occurred_at: String,
    #[serde(default)]
    pub force_reapply: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GacosSubmissionSubmittingRequest {
    pub identity: GacosSubmissionAttemptIdentity,
    pub occurred_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GacosSubmissionOutcomeRequest {
    pub identity: GacosSubmissionAttemptIdentity,
    pub occurred_at: String,
    pub outcome: GacosSubmissionOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum GacosSubmissionOutcome {
    Submitted {
        reason_code: String,
        http_status: u16,
    },
    ManualVerificationRequired {
        reason_code: String,
        http_status: u16,
    },
    SubmissionUncertain {
        reason_code: String,
        http_status: Option<u16>,
    },
    Rejected {
        reason_code: String,
        http_status: Option<u16>,
    },
}

impl GacosSubmissionOutcome {
    fn state(&self) -> GacosSubmissionState {
        match self {
            Self::Submitted { .. } => GacosSubmissionState::Submitted,
            Self::ManualVerificationRequired { .. } => {
                GacosSubmissionState::ManualVerificationRequired
            }
            Self::SubmissionUncertain { .. } => GacosSubmissionState::SubmissionUncertain,
            Self::Rejected { .. } => GacosSubmissionState::Rejected,
        }
    }

    fn parts(&self) -> (&str, Option<u16>) {
        match self {
            Self::Submitted {
                reason_code,
                http_status,
            }
            | Self::ManualVerificationRequired {
                reason_code,
                http_status,
            } => (reason_code, Some(*http_status)),
            Self::SubmissionUncertain {
                reason_code,
                http_status,
            }
            | Self::Rejected {
                reason_code,
                http_status,
            } => (reason_code, *http_status),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GacosSubmissionAttempt {
    pub attempt_id: String,
    pub task_id: String,
    pub item_id: String,
    pub fingerprint_sha256: String,
    pub account_identity_hash_sha256: String,
    pub attempt_number: u64,
    pub state: GacosSubmissionState,
    pub reason_code: Option<String>,
    pub http_status: Option<u16>,
    pub automatic_retry_allowed: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl TaskStore {
    /// Reserve an immutable, one-shot attempt before the external POST.
    ///
    /// The same attempt identity is idempotent. A different attempt with the
    /// same request fingerprint and account is rejected unless the caller
    /// explicitly sets `force_reapply`; that override creates a new history
    /// row and never rewrites the old one.
    pub fn reserve_gacos_submission_attempt(
        &mut self,
        request: &GacosSubmissionReserveRequest,
    ) -> Result<GacosSubmissionAttempt, StoreError> {
        let identity = normalize_identity(&request.identity)?;
        let occurred_at = normalize_timestamp(&request.occurred_at)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_gacos_task_item(&transaction, &identity.task_id, &identity.item_id, true)?;

        if let Some(existing) = load_attempt(&transaction, &identity.attempt_id)? {
            ensure_same_identity(&existing, &identity)?;
            transaction.commit()?;
            return Ok(existing);
        }
        if !request.force_reapply
            && attempt_exists_for_fingerprint_account(
                &transaction,
                &identity.fingerprint_sha256,
                &identity.account_identity_hash_sha256,
            )?
        {
            return Err(StoreError::AlreadyExists);
        }
        if attempt_number_exists(
            &transaction,
            &identity.task_id,
            &identity.item_id,
            identity.attempt_number,
        )? {
            return Err(StoreError::AlreadyExists);
        }

        transaction.execute(
            "INSERT INTO gacos_submission_attempts(
                 attempt_id, task_id, item_id, fingerprint_sha256,
                 account_identity_hash_sha256, attempt_number, state,
                 reason_code, http_status, automatic_retry_allowed,
                 created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, 0, ?8, ?8)",
            params![
                identity.attempt_id,
                identity.task_id,
                identity.item_id,
                identity.fingerprint_sha256,
                identity.account_identity_hash_sha256,
                sqlite_integer(identity.attempt_number)?,
                GacosSubmissionState::Reserved.as_str(),
                occurred_at,
            ],
        )?;
        let stored =
            load_attempt(&transaction, &identity.attempt_id)?.ok_or(StoreError::NotFound)?;
        transaction.commit()?;
        Ok(stored)
    }

    /// Claim a reserved attempt exactly once immediately before the POST.
    /// Only the caller performing this transition may send. A repeated claim
    /// fails, including while another caller is still submitting.
    pub fn mark_gacos_submission_submitting(
        &mut self,
        request: &GacosSubmissionSubmittingRequest,
    ) -> Result<GacosSubmissionAttempt, StoreError> {
        let identity = normalize_identity(&request.identity)?;
        let occurred_at = normalize_timestamp(&request.occurred_at)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_gacos_task_item(&transaction, &identity.task_id, &identity.item_id, true)?;
        let existing =
            load_attempt(&transaction, &identity.attempt_id)?.ok_or(StoreError::NotFound)?;
        ensure_same_identity(&existing, &identity)?;
        match existing.state {
            GacosSubmissionState::Reserved => {
                transaction.execute(
                    "UPDATE gacos_submission_attempts
                     SET state = ?1, updated_at = ?2
                     WHERE attempt_id = ?3 AND state = ?4",
                    params![
                        GacosSubmissionState::Submitting.as_str(),
                        occurred_at,
                        identity.attempt_id,
                        GacosSubmissionState::Reserved.as_str(),
                    ],
                )?;
            }
            _ => return Err(StoreError::AlreadyExists),
        }
        let stored =
            load_attempt(&transaction, &identity.attempt_id)?.ok_or(StoreError::NotFound)?;
        transaction.commit()?;
        Ok(stored)
    }

    /// Persist the externally observed result without changing task status.
    /// A terminal row accepts only an identical replay, retaining its original
    /// timestamp and response summary.
    pub fn record_gacos_submission_outcome(
        &mut self,
        request: &GacosSubmissionOutcomeRequest,
    ) -> Result<GacosSubmissionAttempt, StoreError> {
        let identity = normalize_identity(&request.identity)?;
        let occurred_at = normalize_timestamp(&request.occurred_at)?;
        let (state, reason_code, http_status) = normalize_outcome(&request.outcome)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_gacos_task_item(&transaction, &identity.task_id, &identity.item_id, false)?;
        let existing =
            load_attempt(&transaction, &identity.attempt_id)?.ok_or(StoreError::NotFound)?;
        ensure_same_identity(&existing, &identity)?;
        if existing.state.is_terminal() {
            if existing.state == state
                && existing.reason_code.as_deref() == Some(reason_code.as_str())
                && existing.http_status == http_status
            {
                transaction.commit()?;
                return Ok(existing);
            }
            return Err(StoreError::AlreadyExists);
        }
        transaction.execute(
            "UPDATE gacos_submission_attempts
             SET state = ?1, reason_code = ?2, http_status = ?3, updated_at = ?4
             WHERE attempt_id = ?5 AND state IN ('reserved', 'submitting')",
            params![
                state.as_str(),
                reason_code,
                http_status.map(i64::from),
                occurred_at,
                identity.attempt_id,
            ],
        )?;
        let stored =
            load_attempt(&transaction, &identity.attempt_id)?.ok_or(StoreError::NotFound)?;
        transaction.commit()?;
        Ok(stored)
    }

    /// Convert attempts left in an active state by a crash or process stop to
    /// an explicitly uncertain state. The operation is monotone and can be
    /// safely repeated during startup recovery.
    pub fn recover_gacos_submitting_attempts(
        &mut self,
        task_id: Option<&str>,
        occurred_at: &str,
    ) -> Result<u64, StoreError> {
        let occurred_at = normalize_timestamp(occurred_at)?;
        let task_id = task_id.map(|value| canonical_scalar("task id", value, 128));
        let task_id = match task_id {
            Some(result) => Some(result?),
            None => None,
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(task_id) = task_id.as_deref() {
            ensure_gacos_task(&transaction, task_id, false)?;
        }
        let changed = if let Some(task_id) = task_id.as_deref() {
            transaction.execute(
                "UPDATE gacos_submission_attempts
                 SET state = 'submission_uncertain', reason_code = 'RECOVERED_AFTER_RESTART',
                     http_status = NULL, updated_at = ?1
                 WHERE task_id = ?2 AND state IN ('reserved', 'submitting')
                   AND EXISTS (
                       SELECT 1 FROM task_snapshots snapshot
                       WHERE snapshot.task_id = gacos_submission_attempts.task_id
                         AND snapshot.provider_id = ?3
                         AND json_valid(snapshot.snapshot_json)
                         AND json_extract(snapshot.snapshot_json, '$.operation.kind') = ?4
                         AND json_extract(snapshot.snapshot_json, '$.operation.phase') = ?5
                         AND json_extract(snapshot.snapshot_json, '$.operation.approved_worker_operation') = ?6
                   )",
                params![
                    occurred_at,
                    task_id,
                    GACOS_SUBMISSION_PROVIDER_ID,
                    GACOS_SUBMISSION_OPERATION_KIND,
                    GACOS_SUBMISSION_OPERATION_PHASE,
                    GACOS_SUBMISSION_OPERATION_WORKER,
                ],
            )?
        } else {
            transaction.execute(
                "UPDATE gacos_submission_attempts
                 SET state = 'submission_uncertain', reason_code = 'RECOVERED_AFTER_RESTART',
                     http_status = NULL, updated_at = ?1
                 WHERE state IN ('reserved', 'submitting')
                   AND EXISTS (
                       SELECT 1 FROM task_snapshots snapshot
                       WHERE snapshot.task_id = gacos_submission_attempts.task_id
                         AND snapshot.provider_id = ?2
                         AND json_valid(snapshot.snapshot_json)
                         AND json_extract(snapshot.snapshot_json, '$.operation.kind') = ?3
                         AND json_extract(snapshot.snapshot_json, '$.operation.phase') = ?4
                         AND json_extract(snapshot.snapshot_json, '$.operation.approved_worker_operation') = ?5
                   )",
                params![
                    occurred_at,
                    GACOS_SUBMISSION_PROVIDER_ID,
                    GACOS_SUBMISSION_OPERATION_KIND,
                    GACOS_SUBMISSION_OPERATION_PHASE,
                    GACOS_SUBMISSION_OPERATION_WORKER,
                ],
            )?
        };
        transaction.commit()?;
        u64::try_from(changed).map_err(|_| StoreError::invalid("attempt recovery count overflow"))
    }

    /// Latest provider submission activity across all tasks. Only a timestamp
    /// leaves the Store; this preserves pacing when the application restarts.
    pub fn last_gacos_submission_activity_at(&self) -> Result<Option<String>, StoreError> {
        self.connection.query_row(
            "SELECT updated_at FROM gacos_submission_attempts
             WHERE state != 'reserved' AND (reason_code IS NULL OR reason_code != 'CANCELLED_BEFORE_SEND')
             ORDER BY julianday(updated_at) DESC LIMIT 1", [], |row| row.get(0))
            .optional().map_err(StoreError::from)
    }

    /// Read a bounded attempt history for a real task. No request material or
    /// credential data is returned by this projection.
    pub fn read_gacos_submission_attempts(
        &self,
        task_id: &str,
        item_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<GacosSubmissionAttempt>, StoreError> {
        let task_id = canonical_scalar("task id", task_id, 128)?;
        if !(1..=GACOS_MAX_ATTEMPT_HISTORY).contains(&limit) {
            return Err(StoreError::invalid(
                "GACOS attempt history limit is invalid",
            ));
        }
        let item_id = item_id
            .map(|value| canonical_scalar("item id", value, 2048))
            .transpose()?;
        ensure_gacos_task(&self.connection, &task_id, false)?;
        if let Some(item_id) = item_id.as_deref() {
            ensure_task_membership(&self.connection, &task_id, item_id)?;
        }
        let mut statement = self.connection.prepare(
            "SELECT attempt_id, task_id, item_id, fingerprint_sha256,
                    account_identity_hash_sha256, attempt_number, state,
                    reason_code, http_status, automatic_retry_allowed,
                    created_at, updated_at
             FROM gacos_submission_attempts
             WHERE task_id = ?1 AND (?2 IS NULL OR item_id = ?2)
             ORDER BY attempt_number ASC, created_at ASC, attempt_id ASC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![task_id, item_id, sqlite_integer(limit)?],
            decode_attempt,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Read the newest ledger row for each item in a real task.
    ///
    /// This projection intentionally does not use the bounded history page:
    /// request status must derive completion from every frozen item, even when
    /// an item has more than `GACOS_MAX_ATTEMPT_HISTORY` historical attempts.
    pub fn read_latest_gacos_submission_attempts(
        &self,
        task_id: &str,
    ) -> Result<Vec<GacosSubmissionAttempt>, StoreError> {
        let task_id = canonical_scalar("task id", task_id, 128)?;
        ensure_gacos_task(&self.connection, &task_id, false)?;
        let mut statement = self.connection.prepare(
            "SELECT attempt_id, task_id, item_id, fingerprint_sha256,
                    account_identity_hash_sha256, attempt_number, state,
                    reason_code, http_status, automatic_retry_allowed,
                    created_at, updated_at
             FROM (
                 SELECT attempt_id, task_id, item_id, fingerprint_sha256,
                        account_identity_hash_sha256, attempt_number, state,
                        reason_code, http_status, automatic_retry_allowed,
                        created_at, updated_at,
                        ROW_NUMBER() OVER (
                            PARTITION BY item_id
                            ORDER BY attempt_number DESC, created_at DESC, attempt_id DESC
                        ) AS attempt_rank
                 FROM gacos_submission_attempts
                 WHERE task_id = ?1
             ) ranked
             WHERE attempt_rank = 1
             ORDER BY item_id ASC",
        )?;
        let rows = statement.query_map([task_id], decode_attempt)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn ensure_no_active_gacos_submission(&self, task_id: &str) -> Result<(), StoreError> {
        ensure_no_active_gacos_submission(&self.connection, task_id)
    }
}
pub(crate) fn ensure_no_active_gacos_submission(
    connection: &Connection,
    task_id: &str,
) -> Result<(), StoreError> {
    let active: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM gacos_submission_attempts WHERE task_id = ?1 AND state IN ('reserved', 'submitting'))",
        [task_id], |row| row.get(0))?;
    if active {
        return Err(StoreError::GacosSubmissionInFlight);
    }
    Ok(())
}

fn normalize_identity(
    identity: &GacosSubmissionAttemptIdentity,
) -> Result<GacosSubmissionAttemptIdentity, StoreError> {
    Ok(GacosSubmissionAttemptIdentity {
        task_id: canonical_scalar("task id", &identity.task_id, 128)?,
        item_id: canonical_scalar("item id", &identity.item_id, 2048)?,
        attempt_id: canonical_attempt_id(&identity.attempt_id)?,
        fingerprint_sha256: canonical_sha256("fingerprint", &identity.fingerprint_sha256)?,
        account_identity_hash_sha256: canonical_sha256(
            "account identity hash",
            &identity.account_identity_hash_sha256,
        )?,
        attempt_number: identity.attempt_number,
    })
}

fn normalize_timestamp(value: &str) -> Result<String, StoreError> {
    canonical_utc_identity_timestamp(value)
        .ok_or_else(|| StoreError::invalid("GACOS submission timestamp is invalid"))
}

fn normalize_outcome(
    outcome: &GacosSubmissionOutcome,
) -> Result<(GacosSubmissionState, String, Option<u16>), StoreError> {
    let state = outcome.state();
    let (reason_code, http_status) = outcome.parts();
    let reason_code = canonical_reason_code(reason_code)?;
    let valid_status = match state {
        GacosSubmissionState::ManualVerificationRequired => {
            matches!(http_status, Some(status) if (200..=299).contains(&status))
        }
        GacosSubmissionState::SubmissionUncertain => {
            matches!(http_status, None | Some(300..=399 | 408 | 429 | 500..=599))
        }
        GacosSubmissionState::Rejected => matches!(
            http_status,
            None | Some(100..=407 | 409..=428 | 430..=499)
        ),
        GacosSubmissionState::Submitted => {
            matches!(http_status, Some(status) if (200..=299).contains(&status))
        }
        GacosSubmissionState::Reserved | GacosSubmissionState::Submitting => false,
    };
    if !valid_status {
        return Err(StoreError::invalid(
            "GACOS submission outcome status is inconsistent with its state",
        ));
    }
    Ok((state, reason_code, http_status))
}

fn canonical_scalar(label: &str, value: &str, max_chars: usize) -> Result<String, StoreError> {
    if value.is_empty()
        || value.trim() != value
        || value.chars().count() > max_chars
        || value.chars().any(char::is_control)
    {
        return Err(StoreError::invalid(format!("{label} is invalid")));
    }
    Ok(value.to_owned())
}

fn canonical_attempt_id(value: &str) -> Result<String, StoreError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
    {
        return Err(StoreError::invalid("GACOS attempt id must be opaque"));
    }
    Ok(value.to_owned())
}

fn canonical_sha256(label: &str, value: &str) -> Result<String, StoreError> {
    let value = value.trim();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StoreError::invalid(format!("{label} SHA-256 is invalid")));
    }
    Ok(value.to_ascii_lowercase())
}

fn canonical_reason_code(value: &str) -> Result<String, StoreError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
    {
        return Err(StoreError::invalid(
            "GACOS submission reason code is invalid",
        ));
    }
    Ok(value.to_owned())
}

fn sqlite_integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::invalid("GACOS submission integer is too large"))
}

fn ensure_same_identity(
    existing: &GacosSubmissionAttempt,
    identity: &GacosSubmissionAttemptIdentity,
) -> Result<(), StoreError> {
    if existing.task_id != identity.task_id
        || existing.item_id != identity.item_id
        || existing.fingerprint_sha256 != identity.fingerprint_sha256
        || existing.account_identity_hash_sha256 != identity.account_identity_hash_sha256
        || existing.attempt_number != identity.attempt_number
    {
        return Err(StoreError::AlreadyExists);
    }
    Ok(())
}

fn attempt_exists_for_fingerprint_account(
    connection: &Connection,
    fingerprint_sha256: &str,
    account_identity_hash_sha256: &str,
) -> Result<bool, StoreError> {
    Ok(connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM gacos_submission_attempts
             WHERE fingerprint_sha256 = ?1 AND account_identity_hash_sha256 = ?2
         )",
        params![fingerprint_sha256, account_identity_hash_sha256],
        |row| row.get(0),
    )?)
}

fn attempt_number_exists(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
    attempt_number: u64,
) -> Result<bool, StoreError> {
    Ok(connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM gacos_submission_attempts
             WHERE task_id = ?1 AND item_id = ?2 AND attempt_number = ?3
         )",
        params![task_id, item_id, sqlite_integer(attempt_number)?],
        |row| row.get(0),
    )?)
}

fn load_attempt(
    connection: &Connection,
    attempt_id: &str,
) -> Result<Option<GacosSubmissionAttempt>, StoreError> {
    connection
        .query_row(
            "SELECT attempt_id, task_id, item_id, fingerprint_sha256,
                    account_identity_hash_sha256, attempt_number, state,
                    reason_code, http_status, automatic_retry_allowed,
                    created_at, updated_at
             FROM gacos_submission_attempts
             WHERE attempt_id = ?1",
            [attempt_id],
            decode_attempt,
        )
        .optional()
        .map_err(StoreError::from)
}

fn decode_attempt(row: &Row<'_>) -> rusqlite::Result<GacosSubmissionAttempt> {
    let state_text: String = row.get(6)?;
    let state = GacosSubmissionState::parse(&state_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unknown GACOS submission state",
            )),
        )
    })?;
    let attempt_number = u64::try_from(row.get::<_, i64>(5)?).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "negative GACOS attempt number",
            )),
        )
    })?;
    let http_status = row
        .get::<_, Option<i64>>(8)?
        .map(|value| {
            u16::try_from(value).map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Integer,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid GACOS HTTP status",
                    )),
                )
            })
        })
        .transpose()?;
    let automatic_retry_allowed = match row.get::<_, i64>(9)? {
        0 => false,
        1 => true,
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Integer,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid GACOS retry flag",
                )),
            ));
        }
    };
    Ok(GacosSubmissionAttempt {
        attempt_id: row.get(0)?,
        task_id: row.get(1)?,
        item_id: row.get(2)?,
        fingerprint_sha256: row.get(3)?,
        account_identity_hash_sha256: row.get(4)?,
        attempt_number,
        state,
        reason_code: row.get(7)?,
        http_status,
        automatic_retry_allowed,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}
fn ensure_gacos_task(
    connection: &Connection,
    task_id: &str,
    require_paused: bool,
) -> Result<(), StoreError> {
    let exists: bool = if require_paused {
        connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM task_snapshots snapshot
                 WHERE snapshot.task_id = ?1
                   AND snapshot.provider_id = ?2
                   AND json_valid(snapshot.snapshot_json)
                   AND json_extract(snapshot.snapshot_json, '$.operation.kind') = ?3
                   AND json_extract(snapshot.snapshot_json, '$.operation.phase') = ?4
                   AND json_extract(snapshot.snapshot_json, '$.operation.approved_worker_operation') = ?5
                   AND NOT EXISTS (
                       SELECT 1 FROM task_history history
                       WHERE history.task_id = snapshot.task_id
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM task_tombstones tombstone
                       WHERE tombstone.task_id = snapshot.task_id
                   )
                   AND EXISTS (
                       SELECT 1 FROM task_runtime runtime
                       WHERE runtime.task_id = snapshot.task_id
                         AND runtime.status = 'paused'
                         AND json_valid(runtime.state_json)
                         AND json_extract(runtime.state_json, '$.task_paused') = 1
                   )
             )",
            params![
                task_id,
                GACOS_SUBMISSION_PROVIDER_ID,
                GACOS_SUBMISSION_OPERATION_KIND,
                GACOS_SUBMISSION_OPERATION_PHASE,
                GACOS_SUBMISSION_OPERATION_WORKER,
            ],
            |row| row.get(0),
        )?
    } else {
        connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM task_snapshots snapshot
                 WHERE snapshot.task_id = ?1
                   AND snapshot.provider_id = ?2
                   AND json_valid(snapshot.snapshot_json)
                   AND json_extract(snapshot.snapshot_json, '$.operation.kind') = ?3
                   AND json_extract(snapshot.snapshot_json, '$.operation.phase') = ?4
                   AND json_extract(snapshot.snapshot_json, '$.operation.approved_worker_operation') = ?5
                   AND NOT EXISTS (
                       SELECT 1 FROM task_tombstones tombstone
                       WHERE tombstone.task_id = snapshot.task_id
                   )
             )",
            params![
                task_id,
                GACOS_SUBMISSION_PROVIDER_ID,
                GACOS_SUBMISSION_OPERATION_KIND,
                GACOS_SUBMISSION_OPERATION_PHASE,
                GACOS_SUBMISSION_OPERATION_WORKER,
            ],
            |row| row.get(0),
        )?
    };
    if exists {
        Ok(())
    } else {
        Err(StoreError::NotFound)
    }
}

fn ensure_gacos_task_item(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
    require_paused: bool,
) -> Result<(), StoreError> {
    ensure_gacos_task(connection, task_id, require_paused)?;
    ensure_task_membership(connection, task_id, item_id)
}

fn ensure_task_membership(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
) -> Result<(), StoreError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM task_snapshots snapshot
             CROSS JOIN json_each(
                 CASE
                     WHEN json_valid(snapshot.snapshot_json)
                          AND json_type(snapshot.snapshot_json, '$.selected_products') = 'array'
                     THEN json_extract(snapshot.snapshot_json, '$.selected_products')
                     ELSE '[]'
                 END
             ) selected
             WHERE snapshot.task_id = ?1
               AND json_extract(selected.value, '$.item_id') = ?2
         )",
        params![task_id, item_id],
        |row| row.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::NotFound)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const AT: &str = "2026-09-06T00:00:00Z";
    const AT_1: &str = "2026-09-06T00:00:01Z";
    const AT_2: &str = "2026-09-06T00:00:02Z";
    const AT_3: &str = "2026-09-06T00:00:03Z";
    const AT_4: &str = "2026-09-06T00:00:04Z";

    fn store() -> TaskStore {
        let connection = Connection::open_in_memory().expect("in-memory SQLite");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE task_snapshots(
                     task_id TEXT PRIMARY KEY NOT NULL,
                     provider_id TEXT NOT NULL,
                     snapshot_json TEXT NOT NULL
                 ) STRICT;
                 CREATE TABLE task_runtime(
                     task_id TEXT PRIMARY KEY NOT NULL,
                     status TEXT NOT NULL,
                     state_json TEXT NOT NULL
                 ) STRICT;
                 CREATE TABLE task_history(task_id TEXT PRIMARY KEY NOT NULL) STRICT;
                 CREATE TABLE task_tombstones(task_id TEXT PRIMARY KEY NOT NULL) STRICT;
                 CREATE TABLE task_membership_additions(
                     task_id TEXT NOT NULL,
                     item_id TEXT NOT NULL,
                     item_json TEXT NOT NULL,
                     PRIMARY KEY(task_id, item_id)
                 ) STRICT;",
            )
            .expect("test authority tables");
        connection
            .execute_batch(GACOS_SUBMISSION_ATTEMPTS_SCHEMA_SQL)
            .expect("GACOS attempt schema");
        for task_id in ["task-a", "task-b"] {
            connection
                .execute(
                    "INSERT INTO task_snapshots(task_id, provider_id, snapshot_json)
                     VALUES (?1, ?2, ?3)",
                    params![
                        task_id,
                        GACOS_SUBMISSION_PROVIDER_ID,
                        r#"{
                            "operation": {
                                "operation_id": "request",
                                "kind": "gacos",
                                "phase": "request",
                                "region_id": "region-1",
                                "approved_worker_operation": "gacos_manual_request",
                                "payload": {}
                            },
                            "selected_products": [
                                {"item_id": "gacos-item", "asset_access": null, "execution_policy": null}
                            ]
                        }"#,
                    ],
                )
                .expect("test task snapshot");
            connection
                .execute(
                    "INSERT INTO task_runtime(task_id, status, state_json)
                     VALUES (?1, 'paused', ?2)",
                    params![task_id, r#"{"task_paused":true}"#],
                )
                .expect("test task runtime");
        }
        TaskStore {
            connection,
            seal_read_batch_cursor: std::sync::Mutex::new(0),
            seal_blob_decode_count: std::sync::Mutex::new(0),
        }
    }

    fn identity(
        task_id: &str,
        attempt_id: &str,
        attempt_number: u64,
        fingerprint_sha256: &str,
    ) -> GacosSubmissionAttemptIdentity {
        GacosSubmissionAttemptIdentity {
            task_id: task_id.to_owned(),
            item_id: "gacos-item".to_owned(),
            attempt_id: attempt_id.to_owned(),
            fingerprint_sha256: fingerprint_sha256.to_owned(),
            account_identity_hash_sha256: "b".repeat(64),
            attempt_number,
        }
    }

    fn reserve(
        task_id: &str,
        attempt_id: &str,
        attempt_number: u64,
        fingerprint_sha256: &str,
    ) -> GacosSubmissionReserveRequest {
        GacosSubmissionReserveRequest {
            identity: identity(task_id, attempt_id, attempt_number, fingerprint_sha256),
            occurred_at: AT.to_owned(),
            force_reapply: false,
        }
    }

    #[test]
    fn reserve_submit_outcome_and_identical_replay_are_monotone() {
        let mut store = store();
        let fingerprint = "A".repeat(64);
        let request = reserve("task-a", "attempt-a", 0, &fingerprint);
        let reserved = store
            .reserve_gacos_submission_attempt(&request)
            .expect("reserve");
        assert_eq!(reserved.state, GacosSubmissionState::Reserved);
        assert_eq!(reserved.fingerprint_sha256, "a".repeat(64));
        assert!(!reserved.automatic_retry_allowed);
        assert_eq!(reserved.created_at, AT);
        assert_eq!(reserved.updated_at, AT);

        let replay = store
            .reserve_gacos_submission_attempt(&GacosSubmissionReserveRequest {
                occurred_at: AT_1.to_owned(),
                ..request.clone()
            })
            .expect("idempotent reserve replay");
        assert_eq!(replay, reserved);

        let submitting = store
            .mark_gacos_submission_submitting(&GacosSubmissionSubmittingRequest {
                identity: request.identity.clone(),
                occurred_at: AT_2.to_owned(),
            })
            .expect("mark submitting");
        assert_eq!(submitting.state, GacosSubmissionState::Submitting);
        assert_eq!(submitting.updated_at, AT_2);
        assert!(matches!(
            store.mark_gacos_submission_submitting(&GacosSubmissionSubmittingRequest {
                identity: request.identity.clone(),
                occurred_at: AT_2.to_owned(),
            }),
            Err(StoreError::AlreadyExists)
        ));

        store.connection.execute(r#"UPDATE task_runtime SET status = 'cancelled', state_json = '{"task_paused":false}' WHERE task_id = 'task-a'"#, []).unwrap();
        let outcome = GacosSubmissionOutcomeRequest {
            identity: request.identity.clone(),
            occurred_at: AT_3.to_owned(),
            outcome: GacosSubmissionOutcome::ManualVerificationRequired {
                reason_code: "ACCEPTED".to_owned(),
                http_status: 202,
            },
        };
        let completed = store
            .record_gacos_submission_outcome(&outcome)
            .expect("manual outcome");
        assert_eq!(
            completed.state,
            GacosSubmissionState::ManualVerificationRequired
        );
        assert_eq!(completed.reason_code.as_deref(), Some("ACCEPTED"));
        assert_eq!(completed.http_status, Some(202));
        assert!(!completed.automatic_retry_allowed);
        assert_eq!(completed.updated_at, AT_3);

        let outcome_replay = store
            .record_gacos_submission_outcome(&GacosSubmissionOutcomeRequest {
                occurred_at: AT_4.to_owned(),
                ..outcome
            })
            .expect("idempotent outcome replay");
        assert_eq!(outcome_replay, completed);
        assert!(!matches!(
            outcome_replay.state,
            GacosSubmissionState::Reserved | GacosSubmissionState::Submitting
        ));
        let history = store
            .read_gacos_submission_attempts("task-a", Some("gacos-item"), 8)
            .expect("bounded read");
        assert_eq!(history, vec![completed]);
    }

    #[test]
    fn submitted_outcome_is_terminal_and_replayed_without_mutation() {
        let mut store = store();
        let request = reserve("task-a", "attempt-submitted", 0, &"a".repeat(64));
        store
            .reserve_gacos_submission_attempt(&request)
            .expect("reserve");
        let outcome = GacosSubmissionOutcomeRequest {
            identity: request.identity.clone(),
            occurred_at: AT_1.to_owned(),
            outcome: GacosSubmissionOutcome::Submitted {
                reason_code: "EXPLICIT_SUCCESS_EVIDENCE".to_owned(),
                http_status: 200,
            },
        };
        let submitted = store
            .record_gacos_submission_outcome(&outcome)
            .expect("submitted outcome");
        assert_eq!(submitted.state, GacosSubmissionState::Submitted);
        assert_eq!(
            submitted.reason_code.as_deref(),
            Some("EXPLICIT_SUCCESS_EVIDENCE")
        );
        assert_eq!(submitted.http_status, Some(200));
        assert!(!submitted.automatic_retry_allowed);
        assert_eq!(submitted.updated_at, AT_1);

        let replay = store
            .record_gacos_submission_outcome(&GacosSubmissionOutcomeRequest {
                occurred_at: AT_2.to_owned(),
                ..outcome
            })
            .expect("idempotent submitted replay");
        assert_eq!(replay, submitted);
        assert!(matches!(replay.state, GacosSubmissionState::Submitted));

        let invalid = store.record_gacos_submission_outcome(&GacosSubmissionOutcomeRequest {
            identity: identity("task-a", "attempt-invalid", 1, &"b".repeat(64)),
            occurred_at: AT_3.to_owned(),
            outcome: GacosSubmissionOutcome::Submitted {
                reason_code: "EXPLICIT_SUCCESS_EVIDENCE".to_owned(),
                http_status: 500,
            },
        });
        assert!(matches!(invalid, Err(StoreError::InvalidSnapshot { .. })));
    }

    #[test]
    fn latest_attempt_query_ignores_bounded_history_and_newer_terminal_states() {
        let store = store();
        for (task_id, latest_state, latest_reason, latest_status) in [
            ("task-a", "rejected", "PROVIDER_REJECTED", Some(400_i64)),
            (
                "task-b",
                "submission_uncertain",
                "ACCEPTANCE_UNCERTAIN",
                None,
            ),
        ] {
            for attempt_number in 0..=256_i64 {
                let (state, reason_code, http_status) = if attempt_number == 255 {
                    ("submitted", "EXPLICIT_SUCCESS_EVIDENCE", Some(200_i64))
                } else if attempt_number == 256 {
                    (latest_state, latest_reason, latest_status)
                } else {
                    ("rejected", "PROVIDER_REJECTED", Some(400_i64))
                };
                store
                    .connection
                    .execute(
                        "INSERT INTO gacos_submission_attempts(
                             attempt_id, task_id, item_id, fingerprint_sha256,
                             account_identity_hash_sha256, attempt_number, state,
                             reason_code, http_status, automatic_retry_allowed,
                             created_at, updated_at
                         ) VALUES (?1, ?2, 'gacos-item', ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?9)",
                        params![
                            format!("{task_id}-attempt-{attempt_number}"),
                            task_id,
                            format!("{:064x}", attempt_number + 1),
                            format!("{:064x}", attempt_number + 1),
                            attempt_number,
                            state,
                            reason_code,
                            http_status,
                            AT,
                        ],
                    )
                    .expect("insert attempt history");
            }

            let history = store
                .read_gacos_submission_attempts(task_id, None, 256)
                .expect("bounded history");
            assert_eq!(history.len(), 256);
            assert_eq!(history.last().unwrap().attempt_number, 255);
            assert_eq!(
                history.last().unwrap().state,
                GacosSubmissionState::Submitted
            );

            let latest = store
                .read_latest_gacos_submission_attempts(task_id)
                .expect("latest attempt projection");
            assert_eq!(latest.len(), 1);
            assert_eq!(latest[0].attempt_number, 256);
            assert_eq!(
                latest[0].state,
                GacosSubmissionState::parse(latest_state).unwrap()
            );
        }
    }

    #[test]
    fn duplicate_fingerprint_is_blocked_unless_force_creates_history() {
        let mut store = store();
        let fingerprint = "c".repeat(64);
        let first = store
            .reserve_gacos_submission_attempt(&reserve("task-a", "attempt-a", 0, &fingerprint))
            .expect("first reserve");
        let duplicate = store.reserve_gacos_submission_attempt(&reserve(
            "task-b",
            "attempt-b",
            0,
            &fingerprint,
        ));
        assert!(matches!(duplicate, Err(StoreError::AlreadyExists)));

        let forced = store
            .reserve_gacos_submission_attempt(&GacosSubmissionReserveRequest {
                identity: identity("task-b", "attempt-b", 0, &fingerprint),
                occurred_at: AT_1.to_owned(),
                force_reapply: true,
            })
            .expect("explicit force reapply");
        assert_eq!(forced.task_id, "task-b");
        assert_eq!(
            store
                .read_gacos_submission_attempts("task-a", None, 8)
                .expect("first history"),
            vec![first]
        );
        assert_eq!(
            store
                .read_gacos_submission_attempts("task-b", None, 8)
                .expect("forced history"),
            vec![forced]
        );

        let conflicting_replay =
            store.reserve_gacos_submission_attempt(&GacosSubmissionReserveRequest {
                identity: identity("task-b", "attempt-a", 1, &"d".repeat(64)),
                occurred_at: AT_2.to_owned(),
                force_reapply: true,
            });
        assert!(matches!(conflicting_replay, Err(StoreError::AlreadyExists)));
    }

    #[test]
    fn restart_recovery_marks_reserved_and_submitting_uncertain_once() {
        let mut store = store();
        let reserved_request = reserve("task-a", "attempt-a", 0, &"e".repeat(64));
        store
            .reserve_gacos_submission_attempt(&reserved_request)
            .expect("reserved attempt");
        let submitting_request = reserve("task-a", "attempt-b", 1, &"f".repeat(64));
        store
            .reserve_gacos_submission_attempt(&submitting_request)
            .expect("second attempt");
        store
            .mark_gacos_submission_submitting(&GacosSubmissionSubmittingRequest {
                identity: submitting_request.identity.clone(),
                occurred_at: AT_1.to_owned(),
            })
            .expect("submitting attempt");

        assert_eq!(
            store
                .recover_gacos_submitting_attempts(Some("task-a"), AT_2)
                .expect("recover active attempts"),
            2
        );
        assert_eq!(
            store
                .recover_gacos_submitting_attempts(Some("task-a"), AT_3)
                .expect("idempotent recovery"),
            0
        );
        let history = store
            .read_gacos_submission_attempts("task-a", None, 8)
            .expect("recovered history");
        assert_eq!(history.len(), 2);
        assert!(history.iter().all(|attempt| {
            attempt.state == GacosSubmissionState::SubmissionUncertain
                && attempt.reason_code.as_deref() == Some("RECOVERED_AFTER_RESTART")
                && attempt.http_status.is_none()
                && !attempt.automatic_retry_allowed
                && attempt.updated_at == AT_2
        }));
    }

    #[test]
    fn invalid_inputs_and_unselected_items_fail_closed() {
        let mut store = store();
        let mut invalid = reserve("task-a", "attempt-a", 0, "not-a-sha");
        assert!(matches!(
            store.reserve_gacos_submission_attempt(&invalid),
            Err(StoreError::InvalidSnapshot { .. })
        ));
        invalid.identity = identity("task-a", "attempt-a", 0, &"1".repeat(64));
        invalid.identity.item_id = "not-selected".to_owned();
        assert!(matches!(
            store.reserve_gacos_submission_attempt(&invalid),
            Err(StoreError::NotFound)
        ));

        let valid = reserve("task-a", "attempt-a", 0, &"2".repeat(64));
        store
            .reserve_gacos_submission_attempt(&valid)
            .expect("valid reserve");
        let bad_manual = store.record_gacos_submission_outcome(&GacosSubmissionOutcomeRequest {
            identity: valid.identity.clone(),
            occurred_at: AT_1.to_owned(),
            outcome: GacosSubmissionOutcome::ManualVerificationRequired {
                reason_code: "ACCEPTED".to_owned(),
                http_status: 500,
            },
        });
        assert!(matches!(
            bad_manual,
            Err(StoreError::InvalidSnapshot { .. })
        ));
        let bad_uncertain = store.record_gacos_submission_outcome(&GacosSubmissionOutcomeRequest {
            identity: valid.identity,
            occurred_at: AT_1.to_owned(),
            outcome: GacosSubmissionOutcome::SubmissionUncertain {
                reason_code: "BAD".to_owned(),
                http_status: Some(200),
            },
        });
        assert!(matches!(
            bad_uncertain,
            Err(StoreError::InvalidSnapshot { .. })
        ));
        assert!(matches!(
            store.read_gacos_submission_attempts("task-a", None, 0),
            Err(StoreError::InvalidSnapshot { .. })
        ));
        assert!(matches!(
            store.read_gacos_submission_attempts("task-a", None, GACOS_MAX_ATTEMPT_HISTORY + 1),
            Err(StoreError::InvalidSnapshot { .. })
        ));
    }
}
