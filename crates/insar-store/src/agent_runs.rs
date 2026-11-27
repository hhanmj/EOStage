use crate::{StoreError, TaskStore};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRunRow {
    pub run_id: String,
    pub session_id: String,
    pub origin: String,
    pub client_label: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: String,
    pub model_route: Option<String>,
    pub catalog_version: i64,
    pub catalog_sha256: String,
    pub app_version: String,
    pub source_commit: String,
    pub store_revision_start: i64,
    pub store_revision_end: Option<i64>,
    pub record_dir: String,
    pub trace_sha256: Option<String>,
    pub reproduces_run_id: Option<String>,
    /// A provider refused the channel's reasoning-off switch in this run and
    /// the request was resent without it (schema 36; false before).
    #[serde(default)]
    pub reasoning_off_refused: bool,
    /// A model reply of this run was cut off at the output limit (schema 36).
    #[serde(default)]
    pub truncated: bool,
    /// Prompt tokens the channel reported over this run's replies (schema
    /// 37); null when no reply reported them and on runs sealed before 37.
    #[serde(default)]
    pub usage_prompt: Option<u64>,
    /// Completion tokens likewise (schema 37).
    #[serde(default)]
    pub usage_completion: Option<u64>,
}

/// What the sealed run record shows about its model replies (schema 36;
/// usage totals schema 37).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentRunFlags {
    pub reasoning_off_refused: bool,
    pub truncated: bool,
    pub usage_prompt: Option<u64>,
    pub usage_completion: Option<u64>,
}

/// Re-entrant: a column that already exists is left as is. NULL is a run
/// sealed before schema 37 or whose channel reported no usage.
pub(crate) fn upgrade_schema_v37(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), StoreError> {
    for column in ["usage_prompt", "usage_completion"] {
        let present: i64 = transaction.query_row(
            "SELECT count(*) FROM pragma_table_info('agent_runs') WHERE name=?1",
            [column],
            |row| row.get(0),
        )?;
        if present == 0 {
            transaction.execute_batch(&format!(
                "ALTER TABLE agent_runs ADD COLUMN {column} INTEGER CHECK({column} IS NULL OR {column} >= 0);"
            ))?;
        }
    }
    Ok(())
}

/// Channel-reported counts are not trusted: one beyond SQLite's range is
/// kept as the largest it holds, so sealing, interruption and opening the
/// host never fail on it.
fn usage_sql(value: Option<u64>) -> Option<i64> {
    value.map(|count| i64::try_from(count).unwrap_or(i64::MAX))
}
fn usage_column(value: Option<i64>) -> rusqlite::Result<Option<u64>> {
    value
        .map(|count| u64::try_from(count).map_err(|_| rusqlite::Error::InvalidQuery))
        .transpose()
}

/// Re-entrant: a column that already exists is left as is.
pub(crate) fn upgrade_schema_v36(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), StoreError> {
    for column in ["reasoning_off_refused", "truncated"] {
        let present: i64 = transaction.query_row(
            "SELECT count(*) FROM pragma_table_info('agent_runs') WHERE name=?1",
            [column],
            |row| row.get(0),
        )?;
        if present == 0 {
            transaction.execute_batch(&format!(
                "ALTER TABLE agent_runs ADD COLUMN {column} INTEGER NOT NULL DEFAULT 0 CHECK({column} IN (0,1));"
            ))?;
        }
    }
    Ok(())
}

impl TaskStore {
    pub fn start_agent_run(&mut self, run: &AgentRunRow) -> Result<(), StoreError> {
        if run.status != "running"
            || run.ended_at.is_some()
            || run.trace_sha256.is_some()
            || run.reasoning_off_refused
            || run.truncated
            || run.usage_prompt.is_some()
            || run.usage_completion.is_some()
            || run.record_dir != format!("agent/runs/{}", run.run_id)
            || run.run_id.is_empty()
            || !run
                .run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(StoreError::invalid("invalid initial Agent run"));
        }
        self.connection.execute("INSERT INTO agent_runs(run_id,session_id,origin,client_label,started_at,ended_at,status,model_route,catalog_version,catalog_sha256,app_version,source_commit,store_revision_start,store_revision_end,record_dir,trace_sha256,reproduces_run_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)", params![run.run_id,run.session_id,run.origin,run.client_label,run.started_at,run.ended_at,run.status,run.model_route,run.catalog_version,run.catalog_sha256,run.app_version,run.source_commit,run.store_revision_start,run.store_revision_end,run.record_dir,run.trace_sha256,run.reproduces_run_id])?;
        Ok(())
    }
    pub fn agent_run(&self, run_id: &str) -> Result<Option<AgentRunRow>, StoreError> {
        self.connection.query_row("SELECT run_id,session_id,origin,client_label,started_at,ended_at,status,model_route,catalog_version,catalog_sha256,app_version,source_commit,store_revision_start,store_revision_end,record_dir,trace_sha256,reproduces_run_id,reasoning_off_refused,truncated,usage_prompt,usage_completion FROM agent_runs WHERE run_id=?1", [run_id], |r| Ok(AgentRunRow {run_id:r.get(0)?,session_id:r.get(1)?,origin:r.get(2)?,client_label:r.get(3)?,started_at:r.get(4)?,ended_at:r.get(5)?,status:r.get(6)?,model_route:r.get(7)?,catalog_version:r.get(8)?,catalog_sha256:r.get(9)?,app_version:r.get(10)?,source_commit:r.get(11)?,store_revision_start:r.get(12)?,store_revision_end:r.get(13)?,record_dir:r.get(14)?,trace_sha256:r.get(15)?,reproduces_run_id:r.get(16)?,reasoning_off_refused:r.get(17)?,truncated:r.get(18)?,usage_prompt:usage_column(r.get(19)?)?,usage_completion:usage_column(r.get(20)?)?})).optional().map_err(StoreError::from)
    }
    /// Run ids of one session, oldest first (`agent_runs_session_idx`).
    pub fn agent_session_run_ids(&self, session_id: &str) -> Result<Vec<String>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT run_id FROM agent_runs WHERE session_id=?1 ORDER BY started_at, run_id",
        )?;
        let run_ids = statement
            .query_map([session_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(run_ids)
    }
    pub fn finish_agent_run(
        &mut self,
        run_id: &str,
        status: &str,
        ended_at: &str,
        revision: i64,
        trace_sha256: &str,
        flags: AgentRunFlags,
    ) -> Result<(), StoreError> {
        if !matches!(status, "completed" | "failed" | "cancelled" | "interrupted")
            || trace_sha256.len() != 64
            || !trace_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(StoreError::invalid("invalid Agent run seal"));
        }
        let changed = self.connection.execute("UPDATE agent_runs SET status=?1,ended_at=?2,store_revision_end=?3,trace_sha256=?4,reasoning_off_refused=?5,truncated=?6,usage_prompt=?7,usage_completion=?8 WHERE run_id=?9 AND status='running'",params![status,ended_at,revision,trace_sha256,flags.reasoning_off_refused,flags.truncated,usage_sql(flags.usage_prompt),usage_sql(flags.usage_completion),run_id])?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        Ok(())
    }
    pub fn set_interrupted_trace_hash(
        &mut self,
        run_id: &str,
        hash: &str,
        flags: AgentRunFlags,
    ) -> Result<(), StoreError> {
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(StoreError::invalid("invalid interrupted trace hash"));
        }
        self.connection.execute(
            "UPDATE agent_runs SET trace_sha256=?1,reasoning_off_refused=?2,truncated=?3,usage_prompt=?4,usage_completion=?5 WHERE run_id=?6 AND status='interrupted'",
            params![
                hash,
                flags.reasoning_off_refused,
                flags.truncated,
                usage_sql(flags.usage_prompt),
                usage_sql(flags.usage_completion),
                run_id
            ],
        )?;
        Ok(())
    }
    pub fn delete_agent_run(&mut self, run_id: &str) -> Result<(), StoreError> {
        self.connection.execute("DELETE FROM agent_approvals WHERE plan_id IN (SELECT plan_id FROM agent_plans WHERE run_id=?1)", [run_id])?;
        self.connection
            .execute("DELETE FROM agent_plans WHERE run_id=?1", [run_id])?;
        self.connection
            .execute("DELETE FROM agent_runs WHERE run_id=?1", [run_id])?;
        Ok(())
    }
    pub fn interrupt_agent_runs(&mut self, at: &str) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT run_id FROM agent_runs WHERE status='running' ORDER BY run_id")?;
        let run_ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        self.connection.execute("UPDATE agent_runs SET status='interrupted',ended_at=?1,store_revision_end=store_revision_start WHERE status='running'", [at])?;
        Ok(run_ids)
    }
}
