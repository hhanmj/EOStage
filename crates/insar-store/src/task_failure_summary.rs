use std::collections::HashMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{StoreError, TaskStore};

/// Persisted failure of one task for the new-shell task overview: the stable
/// codes of its failed items in the order their current failures were
/// recorded, and the first recorded human message. Read from `task_runtime`,
/// `task_events` and `download_results`, so it survives a restart. The legacy
/// restore's `failure_codes` (sorted, bounded, codes only) is a separate read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskFailureSummary {
    pub codes: Vec<String>,
    pub message_zh: Option<String>,
    pub failed_items: u64,
    pub total_items: u64,
}

/// Same failure authority as the legacy `TASK_FAILURE_CODES_SQL`: an item's
/// latest `mark_failed` event wins only for event-scoped (NISAR subset) tasks
/// and only when it carries a coded failure; otherwise the result row applies.
const TASK_FAILURE_ITEMS_SQL: &str = r#"
    WITH included(task_id) AS (SELECT value FROM json_each(?1)),
         event_scoped(task_id) AS (SELECT value FROM json_each(?2)),
         failed AS (
           SELECT r.task_id, member.value AS item_id, CAST(member.key AS INTEGER) AS position
           FROM included i
           JOIN task_runtime r ON r.task_id = i.task_id
           JOIN json_each(
             CASE WHEN json_valid(r.state_json) THEN r.state_json ELSE '{}' END,
             '$.failed_item_ids'
           ) member
           WHERE member.type = 'text'
         ),
         latest_failed_event AS (
           SELECT e.task_id, e.item_id, MAX(e.sequence) AS sequence
           FROM included i
           JOIN task_events e ON e.task_id = i.task_id
           WHERE e.event_type = 'mark_failed' AND e.item_id IS NOT NULL
           GROUP BY e.task_id, e.item_id
         )
    SELECT f.task_id,
           latest.sequence,
           f.position,
           COALESCE(
             CASE WHEN json_valid(failure_event.error_json) THEN
               CASE WHEN json_extract(failure_event.error_json, '$.code') = 'SCHEDULER_GENERATION_UNCERTAIN'
                 THEN json_object('code', 'SCHEDULER_GENERATION_UNCERTAIN',
                                  'reason_zh', '无法确认取消/重试代际，需重新检查任务记录',
                                  'retryable', json('false'))
                 WHEN f.task_id IN (SELECT task_id FROM event_scoped)
                      AND json_type(failure_event.error_json, '$.code') = 'text'
                 THEN failure_event.error_json END
             END,
             CASE WHEN json_valid(d.result_json) THEN
               CASE WHEN json_type(d.result_json, '$.failure') = 'object'
                 THEN json_extract(d.result_json, '$.failure') END
             END
           ) AS failure_json
    FROM failed f
    LEFT JOIN download_results d ON d.task_id = f.task_id AND d.item_id = f.item_id
    LEFT JOIN latest_failed_event latest
      ON latest.task_id = f.task_id AND latest.item_id = f.item_id
    LEFT JOIN task_events failure_event
      ON failure_event.task_id = latest.task_id
     AND failure_event.sequence = latest.sequence
"#;

/// The legacy restore's code rule (`core_restore::valid_task_failure_code`):
/// a stable upper-case code, else the same unknown marker.
const UNKNOWN_FAILURE_CODE: &str = "UNKNOWN_FAILURE_CODE";

fn valid_failure_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 128
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// A failed item's latest `mark_failed` sequence, runtime position and
/// failure JSON.
type FailedItemRow = (Option<i64>, i64, Option<String>);

const TASK_FAILURE_TOTALS_SQL: &str = r#"
    SELECT r.task_id,
           r.status,
           CASE WHEN json_valid(r.state_json)
             THEN COALESCE(json_array_length(r.state_json, '$.selected_item_ids'), 0) END,
           CASE WHEN json_valid(r.state_json)
             THEN COALESCE(json_array_length(r.state_json, '$.failed_item_ids'), 0) END
    FROM json_each(?1) i
    JOIN task_runtime r ON r.task_id = i.value
"#;

impl TaskStore {
    /// A summary for every listed task that has a failed item or is itself
    /// failed; other tasks are absent. `event_scoped_task_ids` names the NISAR
    /// subset tasks whose `mark_failed` events may carry their current failure.
    pub fn load_task_failure_summaries(
        &self,
        task_ids: &[String],
        event_scoped_task_ids: &[String],
    ) -> Result<HashMap<String, StoreTaskFailureSummary>, StoreError> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let task_ids_json = serde_json::to_string(task_ids).map_err(StoreError::Corrupt)?;
        let event_scoped_json =
            serde_json::to_string(event_scoped_task_ids).map_err(StoreError::Corrupt)?;

        let mut summaries = HashMap::new();
        let mut totals = self.connection.prepare(TASK_FAILURE_TOTALS_SQL)?;
        let rows = totals.query_map(params![task_ids_json], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        for row in rows {
            // An unreadable runtime row has no trustworthy counts: that task's
            // failure is unknown (null); the rest of the overview stays readable.
            let (task_id, status, Some(total), Some(failed)) = row? else {
                continue;
            };
            if failed > 0 || status == "failed" {
                summaries.insert(
                    task_id,
                    StoreTaskFailureSummary {
                        codes: Vec::new(),
                        message_zh: None,
                        failed_items: u64::try_from(failed).unwrap_or(0),
                        total_items: u64::try_from(total).unwrap_or(0),
                    },
                );
            }
        }
        if summaries.is_empty() {
            return Ok(summaries);
        }

        let mut statement = self.connection.prepare(TASK_FAILURE_ITEMS_SQL)?;
        let rows = statement.query_map(params![task_ids_json, event_scoped_json], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut items_by_task: HashMap<String, Vec<FailedItemRow>> = HashMap::new();
        for row in rows {
            let (task_id, sequence, position, failure_json) = row?;
            items_by_task
                .entry(task_id)
                .or_default()
                .push((sequence, position, failure_json));
        }
        for (task_id, mut items) in items_by_task {
            let Some(summary) = summaries.get_mut(&task_id) else {
                continue;
            };
            // Recorded order: items whose failure event exists first, by event
            // sequence; an item without one keeps its runtime position after them.
            items.sort_by_key(|(sequence, position, _)| (sequence.is_none(), *sequence, *position));
            for (_, _, failure_json) in items {
                let failure = failure_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
                let code = failure
                    .as_ref()
                    .and_then(|value| value.get("code"))
                    .and_then(serde_json::Value::as_str)
                    .filter(|code| valid_failure_code(code))
                    .unwrap_or(UNKNOWN_FAILURE_CODE);
                if !summary.codes.iter().any(|known| known == code) {
                    summary.codes.push(code.to_owned());
                }
                if summary.message_zh.is_none() {
                    summary.message_zh = failure
                        .as_ref()
                        .and_then(|value| value.get("reason_zh"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|message| !message.is_empty())
                        .map(str::to_owned);
                }
            }
        }
        Ok(summaries)
    }
}
