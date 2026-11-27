use crate::{StoreError, TaskStore};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const SCHEMA_V32_SQL: &str = r#"
CREATE TABLE agent_plans (
    plan_id TEXT PRIMARY KEY NOT NULL CHECK(length(plan_id) BETWEEN 1 AND 128),
    session_id TEXT NOT NULL CHECK(length(session_id) BETWEEN 1 AND 128),
    run_id TEXT NOT NULL CHECK(length(run_id) BETWEEN 1 AND 128),
    created_at TEXT NOT NULL,
    model_route TEXT NOT NULL,
    plan_json TEXT NOT NULL CHECK(json_valid(plan_json)),
    plan_hash TEXT NOT NULL CHECK(length(plan_hash) = 64 AND plan_hash NOT GLOB '*[^0-9a-f]*'),
    max_tier INTEGER NOT NULL CHECK(max_tier BETWEEN 0 AND 2),
    status TEXT NOT NULL CHECK(status IN ('draft','proposed','approved','rejected','executing','completed','failed','cancelled','expired')),
    expires_at TEXT NOT NULL,
    store_revision_seen INTEGER NOT NULL CHECK(store_revision_seen >= 0)
) STRICT;
CREATE TABLE agent_approvals (
    approval_id TEXT PRIMARY KEY NOT NULL CHECK(length(approval_id) BETWEEN 1 AND 128),
    plan_id TEXT NOT NULL REFERENCES agent_plans(plan_id) ON DELETE CASCADE,
    plan_hash_seen TEXT NOT NULL CHECK(length(plan_hash_seen) = 64),
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json)),
    decision TEXT NOT NULL CHECK(decision IN ('approved','rejected')),
    decided_at TEXT NOT NULL,
    decided_via TEXT NOT NULL CHECK(decided_via = 'desktop_card')
) STRICT;
CREATE INDEX agent_approvals_plan_idx ON agent_approvals(plan_id, decided_at);
CREATE TABLE agent_runs (
    run_id TEXT PRIMARY KEY NOT NULL CHECK(length(run_id) BETWEEN 1 AND 128),
    session_id TEXT NOT NULL CHECK(length(session_id) BETWEEN 1 AND 128),
    origin TEXT NOT NULL CHECK(origin IN ('in_app','mcp','manual')),
    client_label TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    status TEXT NOT NULL CHECK(status IN ('running','completed','failed','cancelled','interrupted')),
    model_route TEXT,
    catalog_version INTEGER NOT NULL CHECK(catalog_version >= 1),
    catalog_sha256 TEXT NOT NULL CHECK(length(catalog_sha256) = 64),
    app_version TEXT NOT NULL,
    source_commit TEXT NOT NULL,
    store_revision_start INTEGER NOT NULL CHECK(store_revision_start >= 0),
    store_revision_end INTEGER,
    record_dir TEXT NOT NULL CHECK(length(record_dir) BETWEEN 1 AND 256),
    trace_sha256 TEXT,
    reproduces_run_id TEXT
) STRICT;
CREATE INDEX agent_runs_session_idx ON agent_runs(session_id, started_at);
"#;

/// Permission modes (A1 design §3.1, Owner 2026-10-25): an approval is either
/// a desktop card click or an approval by the user's automatic mode, and every
/// approval records the mode in effect. SQLite cannot relax a CHECK in place,
/// so the table is rebuilt; rows decided before schema 34 keep a NULL mode.
/// The rows are copied out and back instead of renaming a new table, because
/// `ALTER TABLE ... RENAME` re-validates every trigger in the schema.
const SCHEMA_V34_SQL: &str = r#"
CREATE TABLE agent_approvals_v33_rows AS SELECT * FROM agent_approvals;
DROP TABLE agent_approvals;
CREATE TABLE agent_approvals (
    approval_id TEXT PRIMARY KEY NOT NULL CHECK(length(approval_id) BETWEEN 1 AND 128),
    plan_id TEXT NOT NULL REFERENCES agent_plans(plan_id) ON DELETE CASCADE,
    plan_hash_seen TEXT NOT NULL CHECK(length(plan_hash_seen) = 64),
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json)),
    decision TEXT NOT NULL CHECK(decision IN ('approved','rejected')),
    decided_at TEXT NOT NULL,
    decided_via TEXT NOT NULL CHECK(decided_via IN ('desktop_card','auto_by_mode')),
    mode TEXT CHECK(mode IS NULL OR mode IN ('per_item','within_quota','full_access')),
    CHECK(decided_via = 'desktop_card' OR (decision = 'approved' AND mode IS NOT NULL AND mode IN ('within_quota','full_access')))
) STRICT;
INSERT INTO agent_approvals(approval_id,plan_id,plan_hash_seen,snapshot_json,decision,decided_at,decided_via,mode)
    SELECT approval_id,plan_id,plan_hash_seen,snapshot_json,decision,decided_at,decided_via,NULL FROM agent_approvals_v33_rows;
DROP TABLE agent_approvals_v33_rows;
CREATE INDEX agent_approvals_plan_idx ON agent_approvals(plan_id, decided_at);
"#;

/// Re-entrant: a table that already has the `mode` column is left as is, so a
/// repeated or faked upgrade converges without touching its rows.
pub(crate) fn upgrade_schema_v34(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), StoreError> {
    let has_mode: i64 = transaction.query_row(
        "SELECT count(*) FROM pragma_table_info('agent_approvals') WHERE name='mode'",
        [],
        |row| row.get(0),
    )?;
    if has_mode == 0 {
        transaction.execute_batch(SCHEMA_V34_SQL)?;
    }
    Ok(())
}

/// Simplified permission modes (Owner 2026-10-26, A1 design §3.1): the modes
/// are `confirm_changes`, `auto_approve` and `full_access`. Stored W3e values
/// are renamed in place (`per_item` → `confirm_changes`, `within_quota` →
/// `auto_approve`). An approval by mode needs a mode but no longer an
/// automatic one: `confirm_changes` approves a disclosed tier 1 plan (a
/// search) without a click, which `decide_agent_plan` checks against the
/// plan's tier. `rewritten_from_plan_hash` keeps the hash of the plan the card
/// showed when the user chose another output directory on it and the plan was
/// rewritten before approval. Rebuilt like schema 34, for the same reasons.
const SCHEMA_V35_SQL: &str = r#"
CREATE TABLE agent_approvals_v34_rows AS SELECT * FROM agent_approvals;
DROP TABLE agent_approvals;
CREATE TABLE agent_approvals (
    approval_id TEXT PRIMARY KEY NOT NULL CHECK(length(approval_id) BETWEEN 1 AND 128),
    plan_id TEXT NOT NULL REFERENCES agent_plans(plan_id) ON DELETE CASCADE,
    plan_hash_seen TEXT NOT NULL CHECK(length(plan_hash_seen) = 64),
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json)),
    decision TEXT NOT NULL CHECK(decision IN ('approved','rejected')),
    decided_at TEXT NOT NULL,
    decided_via TEXT NOT NULL CHECK(decided_via IN ('desktop_card','auto_by_mode')),
    mode TEXT CHECK(mode IS NULL OR mode IN ('confirm_changes','auto_approve','full_access')),
    rewritten_from_plan_hash TEXT CHECK(rewritten_from_plan_hash IS NULL OR (length(rewritten_from_plan_hash) = 64 AND rewritten_from_plan_hash NOT GLOB '*[^0-9a-f]*')),
    CHECK(decided_via = 'desktop_card' OR (decision = 'approved' AND mode IS NOT NULL)),
    CHECK(rewritten_from_plan_hash IS NULL OR (decided_via = 'desktop_card' AND decision = 'approved'))
) STRICT;
INSERT INTO agent_approvals(approval_id,plan_id,plan_hash_seen,snapshot_json,decision,decided_at,decided_via,mode,rewritten_from_plan_hash)
    SELECT approval_id,plan_id,plan_hash_seen,snapshot_json,decision,decided_at,decided_via,
        CASE mode WHEN 'per_item' THEN 'confirm_changes' WHEN 'within_quota' THEN 'auto_approve' ELSE mode END,
        NULL
    FROM agent_approvals_v34_rows;
DROP TABLE agent_approvals_v34_rows;
CREATE INDEX agent_approvals_plan_idx ON agent_approvals(plan_id, decided_at);
"#;

/// Re-entrant: a table that already has `rewritten_from_plan_hash` is left as
/// is. Runs after `upgrade_schema_v34`, so the `mode` column exists.
pub(crate) fn upgrade_schema_v35(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), StoreError> {
    let has_column: i64 = transaction.query_row(
        "SELECT count(*) FROM pragma_table_info('agent_approvals') WHERE name='rewritten_from_plan_hash'",
        [],
        |row| row.get(0),
    )?;
    if has_column == 0 {
        transaction.execute_batch(SCHEMA_V35_SQL)?;
    }
    Ok(())
}

const APPROVAL_MODES: &[&str] = &["confirm_changes", "auto_approve", "full_access"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPlanRow {
    pub plan_id: String,
    pub session_id: String,
    pub run_id: String,
    pub created_at: String,
    pub model_route: String,
    pub plan_json: String,
    pub plan_hash: String,
    pub max_tier: u8,
    pub status: String,
    pub expires_at: String,
    pub store_revision_seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentApprovalRow {
    pub approval_id: String,
    pub plan_id: String,
    pub plan_hash_seen: String,
    pub snapshot_json: String,
    pub decision: String,
    pub decided_at: String,
    pub decided_via: String,
    /// Permission mode in effect when the plan was decided (schema 34, names
    /// of schema 35); `None` on rows decided before schema 34.
    #[serde(default)]
    pub mode: Option<String>,
    /// Hash of the plan the card showed, when the user chose another output
    /// directory on the card and the plan was rewritten before this approval
    /// (schema 35); `plan_hash_seen` is then the rewritten plan's hash.
    #[serde(default)]
    pub rewritten_from_plan_hash: Option<String>,
}

impl TaskStore {
    pub fn save_agent_plan(&mut self, row: &AgentPlanRow) -> Result<(), StoreError> {
        let value: serde_json::Value = serde_json::from_str(&row.plan_json)
            .map_err(|_| StoreError::invalid("agent plan JSON invalid"))?;
        let canonical = serde_json::to_string(&value)
            .map_err(|_| StoreError::invalid("agent plan JSON invalid"))?;
        if !value.is_object()
            || canonical != row.plan_json
            || format!("{:x}", Sha256::digest(canonical.as_bytes())) != row.plan_hash
        {
            return Err(StoreError::invalid(
                "agent plan hash must match canonical JSON",
            ));
        }
        self.connection.execute(
            "INSERT INTO agent_plans(plan_id,session_id,run_id,created_at,model_route,plan_json,plan_hash,max_tier,status,expires_at,store_revision_seen) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![row.plan_id, row.session_id, row.run_id, row.created_at, row.model_route, row.plan_json, row.plan_hash, row.max_tier, row.status, row.expires_at, row.store_revision_seen],
        )?;
        Ok(())
    }

    pub fn agent_plan(&self, plan_id: &str) -> Result<Option<AgentPlanRow>, StoreError> {
        self.connection.query_row("SELECT plan_id,session_id,run_id,created_at,model_route,plan_json,plan_hash,max_tier,status,expires_at,store_revision_seen FROM agent_plans WHERE plan_id=?1", [plan_id], |r| {
            Ok(AgentPlanRow { plan_id:r.get(0)?,session_id:r.get(1)?,run_id:r.get(2)?,created_at:r.get(3)?,model_route:r.get(4)?,plan_json:r.get(5)?,plan_hash:r.get(6)?,max_tier:r.get(7)?,status:r.get(8)?,expires_at:r.get(9)?,store_revision_seen:r.get(10)? })
        }).optional().map_err(StoreError::from)
    }

    pub fn decide_agent_plan(
        &mut self,
        approval: &AgentApprovalRow,
        expected_hash: &str,
        current_revision: i64,
        now: &str,
    ) -> Result<(), StoreError> {
        self.decide_plan(approval, expected_hash, None, current_revision, now)
    }

    /// The desktop card approved the plan after choosing another output
    /// directory: in one transaction the plan the card showed (`expected_hash`,
    /// also `approval.rewritten_from_plan_hash`) is replaced by
    /// `rewritten_json` and approved; `approval.plan_hash_seen` is the
    /// rewritten plan's hash.
    pub fn decide_rewritten_agent_plan(
        &mut self,
        approval: &AgentApprovalRow,
        expected_hash: &str,
        rewritten_json: &str,
        current_revision: i64,
        now: &str,
    ) -> Result<(), StoreError> {
        self.decide_plan(
            approval,
            expected_hash,
            Some(rewritten_json),
            current_revision,
            now,
        )
    }

    fn decide_plan(
        &mut self,
        approval: &AgentApprovalRow,
        expected_hash: &str,
        rewritten_json: Option<&str>,
        current_revision: i64,
        now: &str,
    ) -> Result<(), StoreError> {
        let tx = self.connection.transaction()?;
        let plan: Option<(String,String,i64,String,i64)> = tx.query_row("SELECT plan_hash,plan_json,store_revision_seen,expires_at,max_tier FROM agent_plans WHERE plan_id=?1 AND status IN ('draft','proposed')", [&approval.plan_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((hash, json, seen, expiry, max_tier)) = plan else {
            return Err(StoreError::NotFound);
        };
        let value: serde_json::Value = serde_json::from_str(&json)
            .map_err(|_| StoreError::invalid("agent plan JSON invalid"))?;
        let canonical = serde_json::to_string(&value)
            .map_err(|_| StoreError::invalid("agent plan JSON invalid"))?;
        // The hash of the plan the decider saw: the stored plan itself, or
        // for a rewrite the plan before the rewrite.
        let shown = match rewritten_json {
            None => approval.plan_hash_seen.as_str(),
            Some(_) => approval
                .rewritten_from_plan_hash
                .as_deref()
                .unwrap_or_default(),
        };
        if canonical != json
            || format!("{:x}", Sha256::digest(canonical.as_bytes())) != hash
            || hash != expected_hash
            || hash != shown
            || seen != current_revision
            || expiry.as_str() <= now
            || json.is_empty()
        {
            tx.execute(
                "UPDATE agent_plans SET status='expired' WHERE plan_id=?1",
                [&approval.plan_id],
            )?;
            tx.commit()?;
            return Err(StoreError::RevisionConflict);
        }
        match rewritten_json {
            None if approval.rewritten_from_plan_hash.is_some() => {
                return Err(StoreError::invalid(
                    "only a rewritten plan names the plan it was rewritten from",
                ));
            }
            None => {}
            Some(rewritten) => {
                let value: serde_json::Value = serde_json::from_str(rewritten)
                    .map_err(|_| StoreError::invalid("rewritten agent plan JSON invalid"))?;
                let canonical = serde_json::to_string(&value)
                    .map_err(|_| StoreError::invalid("rewritten agent plan JSON invalid"))?;
                let rewritten_hash = format!("{:x}", Sha256::digest(canonical.as_bytes()));
                if !value.is_object()
                    || canonical != rewritten
                    || rewritten_hash != approval.plan_hash_seen
                    || rewritten_hash == hash
                    || approval.decided_via != "desktop_card"
                    || approval.decision != "approved"
                {
                    return Err(StoreError::invalid(
                        "a rewritten plan is approved on the desktop card under its own hash",
                    ));
                }
                tx.execute(
                    "UPDATE agent_plans SET plan_json=?1, plan_hash=?2 WHERE plan_id=?3",
                    params![rewritten, rewritten_hash, approval.plan_id],
                )?;
            }
        }
        if approval
            .mode
            .as_deref()
            .is_some_and(|mode| !APPROVAL_MODES.contains(&mode))
        {
            return Err(StoreError::invalid("unknown agent permission mode"));
        }
        match approval.decided_via.as_str() {
            "desktop_card" => {}
            "auto_by_mode" => {
                let allowed = match approval.mode.as_deref() {
                    Some("auto_approve" | "full_access") => true,
                    // confirm_changes runs a disclosed search without a click
                    // but never approves a tier 2 plan.
                    Some("confirm_changes") => max_tier <= 1,
                    _ => false,
                };
                if approval.decision != "approved" || !allowed {
                    return Err(StoreError::invalid(
                        "an approval by mode is an approval the mode in effect allows",
                    ));
                }
                // A1 §6.1 is unchanged: a plan from an MCP (or manual) run is
                // approved on the desktop card only.
                let origin: Option<String> = tx
                    .query_row(
                        "SELECT r.origin FROM agent_plans p JOIN agent_runs r ON r.run_id=p.run_id WHERE p.plan_id=?1",
                        [&approval.plan_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if origin.as_deref() != Some("in_app") {
                    return Err(StoreError::invalid(
                        "only an in-app run's plan can be approved by mode",
                    ));
                }
            }
            _ => {
                return Err(StoreError::invalid(
                    "agent approval must come from desktop card or the permission mode",
                ))
            }
        }
        tx.execute("INSERT INTO agent_approvals(approval_id,plan_id,plan_hash_seen,snapshot_json,decision,decided_at,decided_via,mode,rewritten_from_plan_hash) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![approval.approval_id, approval.plan_id, approval.plan_hash_seen, approval.snapshot_json, approval.decision, approval.decided_at, approval.decided_via, approval.mode, approval.rewritten_from_plan_hash])?;
        tx.execute(
            "UPDATE agent_plans SET status=?1 WHERE plan_id=?2",
            params![
                if approval.decision == "approved" {
                    "approved"
                } else {
                    "rejected"
                },
                approval.plan_id
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn expire_agent_plan(&mut self, plan_id: &str) -> Result<(), StoreError> {
        self.connection.execute("UPDATE agent_plans SET status='expired' WHERE plan_id=?1 AND status NOT IN ('completed','failed','cancelled')", [plan_id])?;
        Ok(())
    }

    /// Removes every plan of one session and its approvals in one transaction
    /// (their snapshots carry output paths and area names). Tasks, batches and
    /// files are never touched. Returns the number of plans removed.
    pub fn delete_agent_session_plans(&mut self, session_id: &str) -> Result<usize, StoreError> {
        let tx = self.connection.transaction()?;
        tx.execute(
            "DELETE FROM agent_approvals WHERE plan_id IN (SELECT plan_id FROM agent_plans WHERE session_id=?1)",
            [session_id],
        )?;
        let removed = tx.execute("DELETE FROM agent_plans WHERE session_id=?1", [session_id])?;
        tx.commit()?;
        Ok(removed)
    }

    pub fn agent_approval(&self, plan_id: &str) -> Result<Option<AgentApprovalRow>, StoreError> {
        self.connection.query_row("SELECT approval_id,plan_id,plan_hash_seen,snapshot_json,decision,decided_at,decided_via,mode,rewritten_from_plan_hash FROM agent_approvals WHERE plan_id=?1 ORDER BY decided_at DESC LIMIT 1", [plan_id], |r| Ok(AgentApprovalRow {approval_id:r.get(0)?,plan_id:r.get(1)?,plan_hash_seen:r.get(2)?,snapshot_json:r.get(3)?,decision:r.get(4)?,decided_at:r.get(5)?,decided_via:r.get(6)?,mode:r.get(7)?,rewritten_from_plan_hash:r.get(8)?})).optional().map_err(StoreError::from)
    }
}
