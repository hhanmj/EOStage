//! Orbit scheduled-fetch S2 (schema 40): the inert `orbit_recheck_schedule`
//! and `companion_preferences` tables plus their Store API.
//!
//! `orbit_recheck_schedule` keeps one row per waiting companion precise-orbit
//! item (task, item, scene, orbit type, acquisition time, attempt counter,
//! next check time, deadline, state). It deliberately stores no URL, no
//! credential and no response body: the recheck re-resolves through the
//! provider exactly like a first attempt. Nothing in production writes these
//! rows in this slice; the scheduler wiring is slice S3.
//!
//! `companion_preferences` keeps the per-source "remember the companion orbit
//! choice" bit and the application-wide auto-fetch switch (Owner 11-01
//! decisions 6 and 7: both default to on when no row exists).

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use insar_core::{StoreUpdateTaskRequest, TaskStateCommandKind};

use crate::{
    canonical_utc_identity_timestamp, repository::load_compact_task_connection,
    runtime::update_task_in_transaction, StoreError, StoredTask, TaskStore,
};

pub(crate) const SCHEMA_V40_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS orbit_recheck_schedule (
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE
        CHECK(length(task_id) BETWEEN 1 AND 128 AND trim(task_id) = task_id),
    item_id TEXT NOT NULL
        CHECK(length(item_id) BETWEEN 1 AND 2048 AND trim(item_id) = item_id),
    scene_id TEXT NOT NULL
        CHECK(length(scene_id) BETWEEN 1 AND 256 AND trim(scene_id) = scene_id),
    orbit_type TEXT NOT NULL CHECK(orbit_type IN ('POEORB')),
    acquisition_at TEXT NOT NULL CHECK(
        acquisition_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
        OR acquisition_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9]Z'
        OR acquisition_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9]Z'
        OR acquisition_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9][0-9]Z'
    ),
    attempt INTEGER NOT NULL CHECK(attempt >= 0),
    next_check_at TEXT NOT NULL CHECK(
        next_check_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
        OR next_check_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9]Z'
        OR next_check_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9]Z'
        OR next_check_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9][0-9]Z'
    ),
    deadline_at TEXT NOT NULL CHECK(
        deadline_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
        OR deadline_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9]Z'
        OR deadline_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9]Z'
        OR deadline_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9][0-9]Z'
    ),
    state TEXT NOT NULL CHECK(state IN ('waiting', 'exhausted', 'cancelled')),
    last_error_code TEXT CHECK(last_error_code IS NULL OR (
        length(last_error_code) BETWEEN 1 AND 128
        AND trim(last_error_code) = last_error_code
        AND last_error_code NOT GLOB '*[^A-Za-z0-9_.:-]*'
    )),
    updated_at TEXT NOT NULL CHECK(
        updated_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
        OR updated_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9]Z'
        OR updated_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9]Z'
        OR updated_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9][0-9]Z'
    ),
    PRIMARY KEY(task_id, item_id)
) STRICT;

CREATE INDEX IF NOT EXISTS orbit_recheck_next_idx
    ON orbit_recheck_schedule(state, next_check_at, task_id, item_id);

CREATE TABLE IF NOT EXISTS companion_preferences (
    source_id TEXT PRIMARY KEY NOT NULL
        CHECK(length(source_id) BETWEEN 1 AND 128 AND trim(source_id) = source_id),
    include_precise_orbit INTEGER NOT NULL CHECK(include_precise_orbit IN (0, 1)),
    auto_fetch_orbit INTEGER NOT NULL CHECK(auto_fetch_orbit IN (0, 1)),
    updated_at TEXT NOT NULL CHECK(
        updated_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
        OR updated_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9]Z'
        OR updated_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9]Z'
        OR updated_at GLOB '2[0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9][0-9]Z'
    )
) STRICT;
"#;

/// The schedule row state. `waiting` rows are gated by `next_check_at`;
/// `exhausted` rows passed the deadline and only a manual retry can revive
/// the item; `cancelled` rows are kept for their audit trail only — the
/// scheduler never issues another request for them.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreOrbitRecheckState {
    Waiting,
    Exhausted,
    Cancelled,
}

impl StoreOrbitRecheckState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Exhausted => "exhausted",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "waiting" => Self::Waiting,
            "exhausted" => Self::Exhausted,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// One schedule row. Timestamps are canonical UTC RFC3339 (whole seconds or
/// one to three fractional digits, `Z` suffix); anything else is rejected
/// instead of stored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoreOrbitRecheckRequest {
    pub task_id: String,
    pub item_id: String,
    pub scene_id: String,
    pub orbit_type: String,
    pub acquisition_at: String,
    pub attempt: u64,
    pub next_check_at: String,
    pub deadline_at: String,
    pub state: StoreOrbitRecheckState,
    #[serde(default)]
    pub last_error_code: Option<String>,
    pub updated_at: String,
}

/// A stored schedule row, in the same shape the caller upserted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoreOrbitRecheckRow {
    pub task_id: String,
    pub item_id: String,
    pub scene_id: String,
    pub orbit_type: String,
    pub acquisition_at: String,
    pub attempt: u64,
    pub next_check_at: String,
    pub deadline_at: String,
    pub state: StoreOrbitRecheckState,
    pub last_error_code: Option<String>,
    pub updated_at: String,
}

impl StoreOrbitRecheckRow {
    /// The contract projection of this row (orbit scheduled-fetch S4). The
    /// EOF file name is not part of the persisted row: it only exists once
    /// the provider has resolved the scene, so it projects as `None` here
    /// (SAR-REG-006: never guessed from the acquisition time).
    pub fn recheck_info(&self) -> insar_core::OrbitRecheckInfo {
        insar_core::OrbitRecheckInfo {
            orbit_type: self.orbit_type.clone(),
            state: self.state.as_str().to_owned(),
            attempt: self.attempt,
            next_check_at: self.next_check_at.clone(),
            deadline_at: self.deadline_at.clone(),
            expected_file_name: None,
        }
    }
}

/// The per-source companion preference. A missing row means "include precise
/// orbit" and "auto-fetch on" — the defaults the Owner accepted on 11-01.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoreCompanionPreferences {
    pub source_id: String,
    pub include_precise_orbit: bool,
    pub auto_fetch_orbit: bool,
    /// `None` while the source still uses the defaults (no row stored).
    pub updated_at: Option<String>,
}

fn normalize_timestamp(field: &str, value: &str) -> Result<String, StoreError> {
    canonical_utc_identity_timestamp(value).ok_or_else(|| {
        StoreError::invalid(format!(
            "orbit recheck {field} must be a UTC RFC3339 timestamp"
        ))
    })
}

fn bounded_scalar(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.trim() == value
}

impl TaskStore {
    /// Insert or replace one schedule row and return what was stored. The
    /// caller (the scheduler coordinator, slice S3) owns the write; a row for
    /// a task that does not exist is refused by the foreign key.
    pub fn upsert_orbit_recheck(
        &mut self,
        request: &StoreOrbitRecheckRequest,
    ) -> Result<StoreOrbitRecheckRow, StoreError> {
        if !bounded_scalar(&request.task_id, 128)
            || !bounded_scalar(&request.item_id, 2048)
            || !bounded_scalar(&request.scene_id, 256)
            || request.orbit_type != "POEORB"
        {
            return Err(StoreError::invalid(
                "orbit recheck identity or orbit type is invalid",
            ));
        }
        let acquisition_at = normalize_timestamp("acquisition_at", &request.acquisition_at)?;
        let next_check_at = normalize_timestamp("next_check_at", &request.next_check_at)?;
        let deadline_at = normalize_timestamp("deadline_at", &request.deadline_at)?;
        let updated_at = normalize_timestamp("updated_at", &request.updated_at)?;
        let attempt = i64::try_from(request.attempt)
            .map_err(|_| StoreError::invalid("orbit recheck attempt exceeds SQLite range"))?;
        let last_error_code = request
            .last_error_code
            .as_deref()
            .map(|code| {
                if bounded_scalar(code, 128)
                    && !code.contains(|c: char| !c.is_ascii_alphanumeric() && !"_ .:-".contains(c))
                {
                    Ok(code.to_owned())
                } else {
                    Err(StoreError::invalid(
                        "orbit recheck last error code is invalid",
                    ))
                }
            })
            .transpose()?;

        self.connection.execute(
            "INSERT INTO orbit_recheck_schedule(
                 task_id, item_id, scene_id, orbit_type, acquisition_at,
                 attempt, next_check_at, deadline_at, state, last_error_code, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(task_id, item_id) DO UPDATE SET
                 scene_id = excluded.scene_id,
                 orbit_type = excluded.orbit_type,
                 acquisition_at = excluded.acquisition_at,
                 attempt = excluded.attempt,
                 next_check_at = excluded.next_check_at,
                 deadline_at = excluded.deadline_at,
                 state = excluded.state,
                 last_error_code = excluded.last_error_code,
                 updated_at = excluded.updated_at",
            params![
                request.task_id,
                request.item_id,
                request.scene_id,
                request.orbit_type,
                acquisition_at,
                attempt,
                next_check_at,
                deadline_at,
                request.state.as_str(),
                last_error_code,
                updated_at,
            ],
        )?;
        self.load_orbit_recheck(&request.task_id, &request.item_id)?
            .ok_or(StoreError::NotFound)
    }

    /// Every schedule row, the rows nearest due first (`next_check_at`
    /// ascending, then task/item for a stable order). One query per scheduler
    /// pass; no per-task joins.
    pub fn load_orbit_rechecks(&self) -> Result<Vec<StoreOrbitRecheckRow>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT task_id, item_id, scene_id, orbit_type, acquisition_at, \
                    attempt, next_check_at, deadline_at, state, last_error_code, updated_at \
             FROM orbit_recheck_schedule \
             ORDER BY next_check_at, task_id, item_id",
        )?;
        let rows = statement
            .query_map([], decode_recheck_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// One schedule row by its owning task and item. `None` when absent.
    pub fn load_orbit_recheck(
        &self,
        task_id: &str,
        item_id: &str,
    ) -> Result<Option<StoreOrbitRecheckRow>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT task_id, item_id, scene_id, orbit_type, acquisition_at, \
                    attempt, next_check_at, deadline_at, state, last_error_code, updated_at \
             FROM orbit_recheck_schedule WHERE task_id = ?1 AND item_id = ?2",
        )?;
        let row = statement
            .query_map(params![task_id, item_id], decode_recheck_row)?
            .next()
            .transpose()?;
        Ok(row)
    }

    /// Every schedule row of one task (orbit scheduled-fetch S4: the task
    /// page projection). One bounded query per page read, ordered for a
    /// stable projection; rows whose task has none return empty.
    pub fn load_orbit_rechecks_for_task(
        &self,
        task_id: &str,
    ) -> Result<Vec<StoreOrbitRecheckRow>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT task_id, item_id, scene_id, orbit_type, acquisition_at, \
                    attempt, next_check_at, deadline_at, state, last_error_code, updated_at \
             FROM orbit_recheck_schedule WHERE task_id = ?1 \
             ORDER BY item_id",
        )?;
        let rows = statement
            .query_map(params![task_id], decode_recheck_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Remove one schedule row. Removing an absent row is a no-op, matching
    /// the "cancel clears the gate" semantics of slice S3.
    pub fn clear_orbit_recheck(&mut self, task_id: &str, item_id: &str) -> Result<(), StoreError> {
        self.connection.execute(
            "DELETE FROM orbit_recheck_schedule WHERE task_id = ?1 AND item_id = ?2",
            params![task_id, item_id],
        )?;
        Ok(())
    }

    /// Orbit scheduled-fetch S4 (review findings 1 and 5): apply a task
    /// command and clear the persisted recheck rows of exactly the items the
    /// command revives, in ONE transaction.
    ///
    /// A retry must not clear the Waiting rows of still-queued items — those
    /// scenes would dispatch immediately with their attempt/backoff reset.
    /// The revived set is what the store's own command expansion retries:
    /// the named items for `retry_items`/`reacquire_items`, and the task's
    /// failed ∪ cancelled items for `retry_task` (read in this transaction,
    /// before the update applies). Doing both steps on one transaction means
    /// a crash can neither leave the retry committed with stale rows nor
    /// leave a queued item gated by a cleared-then-lost row.
    pub fn update_task_and_clear_orbit_rechecks(
        &mut self,
        request: &StoreUpdateTaskRequest,
    ) -> Result<StoredTask, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        if request.occurred_at.trim().is_empty() {
            return Err(StoreError::invalid("event timestamp is required"));
        }
        let request_json = serde_json::to_string(request).map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let event_recorded: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_events WHERE event_id = ?1)",
            [&request.event_id],
            |row| row.get(0),
        )?;
        let already_recorded = event_recorded
            || crate::source_receipts::has_reacquire_receipt(&transaction, &request.event_id)?;
        // The retry_task expansion revives the task's current failed and
        // cancelled items; read them before the update applies.
        let revived: Vec<String> = if already_recorded {
            Vec::new()
        } else {
            prepare_manual_orbit_item_action(&transaction, request)?;
            match request.command.kind {
                TaskStateCommandKind::RetryTask => {
                    let task = load_compact_task_connection(&transaction, &request.task_id)?
                        .ok_or(StoreError::NotFound)?;
                    task.runtime
                        .failed_item_ids
                        .iter()
                        .chain(task.runtime.cancelled_item_ids.iter())
                        .cloned()
                        .collect()
                }
                TaskStateCommandKind::RetryItems | TaskStateCommandKind::ReacquireItems => {
                    request.command.item_ids.clone()
                }
                _ => Vec::new(),
            }
        };
        let task = update_task_in_transaction(&transaction, request, &request_json)?;
        // A replay must not erase a schedule created after the original retry.
        // update_task_in_transaction validates the event identity in either case.
        if !already_recorded && !revived.is_empty() {
            for item_id in &revived {
                transaction.execute(
                    "DELETE FROM orbit_recheck_schedule WHERE task_id = ?1 AND item_id = ?2",
                    params![request.task_id, item_id],
                )?;
            }
        }
        let task = crate::source_index::apply_task_item_rebinds(&transaction, task)?;
        transaction.commit()?;
        Ok(task)
    }

    /// Remove every schedule row of one task (item cancel paths and whole-task
    /// cancel; TASK-REG-012 keeps terminal tasks from being revived).
    pub fn cancel_orbit_rechecks_for_task(&mut self, task_id: &str) -> Result<(), StoreError> {
        self.connection.execute(
            "DELETE FROM orbit_recheck_schedule WHERE task_id = ?1",
            params![task_id],
        )?;
        Ok(())
    }

    /// The stored preference for one source, or the on-by-default row when no
    /// preference has been saved yet.
    pub fn load_companion_preferences(
        &self,
        source_id: &str,
    ) -> Result<StoreCompanionPreferences, StoreError> {
        if !bounded_scalar(source_id, 128) {
            return Err(StoreError::invalid(
                "companion preference source is invalid",
            ));
        }
        let stored: Option<(i64, i64, String)> = self
            .connection
            .query_row(
                "SELECT include_precise_orbit, auto_fetch_orbit, updated_at \
                 FROM companion_preferences WHERE source_id = ?1",
                [source_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(match stored {
            Some((include, auto, updated_at)) => StoreCompanionPreferences {
                source_id: source_id.to_owned(),
                include_precise_orbit: include != 0,
                auto_fetch_orbit: auto != 0,
                updated_at: Some(updated_at),
            },
            // Owner 11-01 decisions 6 and 7: the companion orbit choice and
            // the auto-fetch switch default to on until the user opts out.
            None => StoreCompanionPreferences {
                source_id: source_id.to_owned(),
                include_precise_orbit: true,
                auto_fetch_orbit: true,
                updated_at: None,
            },
        })
    }

    /// Insert or replace the preference row of one source.
    pub fn save_companion_preferences(
        &mut self,
        preferences: &StoreCompanionPreferences,
    ) -> Result<(), StoreError> {
        if !bounded_scalar(&preferences.source_id, 128) {
            return Err(StoreError::invalid(
                "companion preference source is invalid",
            ));
        }
        let updated_at = normalize_timestamp(
            "updated_at",
            preferences.updated_at.as_deref().ok_or_else(|| {
                StoreError::invalid("companion preference update timestamp is required")
            })?,
        )?;
        self.connection.execute(
            "INSERT INTO companion_preferences(source_id, include_precise_orbit, auto_fetch_orbit, updated_at) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(source_id) DO UPDATE SET \
                 include_precise_orbit = excluded.include_precise_orbit, \
                 auto_fetch_orbit = excluded.auto_fetch_orbit, \
                 updated_at = excluded.updated_at",
            params![
                preferences.source_id,
                i64::from(preferences.include_precise_orbit),
                i64::from(preferences.auto_fetch_orbit),
                updated_at,
            ],
        )?;
        Ok(())
    }
}

/// Orbit scheduled-fetch S7: when a row-level retry comes from an explicit
/// manual orbit action (`orbit-resorb-`, `orbit-upgrade-`, `orbit-check-`),
/// update the candidate's requested orbit type and, when the item is currently
/// sitting queued behind a `Waiting` schedule row or `Completed` with a
/// temporary `RESORB`, stage it into `failed_item_ids` in the same transaction
/// so `reduce_task_state`'s `RetryItems` transition moves it cleanly to `queued`.
fn prepare_manual_orbit_item_action(
    transaction: &rusqlite::Transaction<'_>,
    request: &StoreUpdateTaskRequest,
) -> Result<(), StoreError> {
    if request.command.kind != TaskStateCommandKind::RetryItems {
        return Ok(());
    }
    let is_resorb = request.event_id.starts_with("orbit-resorb-");
    let is_upgrade = request.event_id.starts_with("orbit-upgrade-");
    let is_check =
        request.event_id.starts_with("orbit-check-") || request.event_id.starts_with("orbit-row-");
    if !is_resorb && !is_upgrade && !is_check {
        return Ok(());
    }

    if is_upgrade {
        for item_id in &request.command.item_ids {
            transaction.execute(
                "DELETE FROM download_results WHERE task_id = ?1 AND item_id = ?2",
                params![request.task_id, item_id],
            )?;
        }
        transaction.execute(
            "DELETE FROM task_history WHERE task_id = ?1",
            params![request.task_id],
        )?;
    }

    if let Some(mut task) = load_compact_task_connection(transaction, &request.task_id)? {
        if task.runtime.revision == request.expected_revision {
            let mut staged = false;
            for item_id in &request.command.item_ids {
                let was_queued = if let Some(pos) = task
                    .runtime
                    .queued_item_ids
                    .iter()
                    .position(|id| id == item_id)
                {
                    task.runtime.queued_item_ids.remove(pos);
                    true
                } else {
                    false
                };
                let was_completed = if is_upgrade {
                    if let Some(pos) = task
                        .runtime
                        .completed_item_ids
                        .iter()
                        .position(|id| id == item_id)
                    {
                        task.runtime.completed_item_ids.remove(pos);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if (was_queued || was_completed)
                    && !task.runtime.failed_item_ids.iter().any(|id| id == item_id)
                {
                    task.runtime.failed_item_ids.push(item_id.clone());
                    staged = true;
                }
            }
            if staged {
                task.runtime.status = insar_core::TaskLifecycleStatus::Failed;
                let runtime_json =
                    serde_json::to_string(&task.runtime).map_err(StoreError::Corrupt)?;
                let expected_sql = i64::try_from(request.expected_revision)
                    .map_err(|_| StoreError::invalid("expected task revision overflow"))?;
                transaction.execute(
                    "UPDATE task_runtime SET status = 'failed', state_json = ?1 \
                     WHERE task_id = ?2 AND revision = ?3",
                    params![runtime_json, request.task_id, expected_sql],
                )?;
            }
        }
    }

    Ok(())
}

fn decode_recheck_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoreOrbitRecheckRow> {
    let state: String = row.get(8)?;
    let state = StoreOrbitRecheckState::parse(&state).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Text,
            "orbit recheck state is invalid".into(),
        )
    })?;
    let attempt: i64 = row.get(5)?;
    if attempt < 0 {
        return Err(rusqlite::Error::InvalidParameterName(
            "orbit recheck attempt is negative".to_owned(),
        ));
    }
    Ok(StoreOrbitRecheckRow {
        task_id: row.get(0)?,
        item_id: row.get(1)?,
        scene_id: row.get(2)?,
        orbit_type: row.get(3)?,
        acquisition_at: row.get(4)?,
        attempt: attempt as u64,
        next_check_at: row.get(6)?,
        deadline_at: row.get(7)?,
        state,
        last_error_code: row.get(9)?,
        updated_at: row.get(10)?,
    })
}
