use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::StoreError;

pub const MAX_EXPERIMENT_PROJECT_ITEMS: usize = 32;
pub const DEFAULT_EXPERIMENT_PROJECT_ID: &str = "default";
pub const DEFAULT_EXPERIMENT_PROJECT_TITLE: &str = "默认实验工程";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExperimentProjectRecord {
    pub project_id: String,
    pub title: String,
    pub aoi_display_name: Option<String>,
    pub aoi_bbox_json: Option<String>,
    pub aoi_geojson: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExperimentProjectItemRecord {
    pub item_id: String,
    pub project_id: String,
    pub task_id: String,
    pub provider_id: String,
    pub dataset_id: String,
    pub task_type: Option<String>,
    pub output_root: String,
    pub primary_artifact_path: String,
    pub provenance_path: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub bbox_json: Option<String>,
    pub license_id: String,
    pub added_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentProjectUpsertItem {
    pub item_id: String,
    pub project_id: String,
    pub task_id: String,
    pub provider_id: String,
    pub dataset_id: String,
    pub task_type: Option<String>,
    pub output_root: String,
    pub primary_artifact_path: String,
    pub provenance_path: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub bbox_json: Option<String>,
    pub license_id: String,
    pub timestamp: String,
}

pub struct ExperimentProjectStore {
    connection: Connection,
}

impl ExperimentProjectStore {
    pub fn sibling_path_for_task_store(task_store_path: &Path) -> PathBuf {
        task_store_path.with_file_name("experiment_projects.sqlite3")
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(StoreError::Filesystem)?;
            }
        }
        let connection = Connection::open(path).map_err(StoreError::Database)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA synchronous = NORMAL;
                 CREATE TABLE IF NOT EXISTS experiment_projects (
                     project_id TEXT PRIMARY KEY,
                     title TEXT NOT NULL,
                     aoi_display_name TEXT,
                     aoi_bbox_json TEXT,
                     aoi_geojson TEXT,
                     created_at TEXT NOT NULL,
                     updated_at TEXT NOT NULL
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS experiment_project_items (
                     item_id TEXT PRIMARY KEY,
                     project_id TEXT NOT NULL REFERENCES experiment_projects(project_id) ON DELETE CASCADE,
                     task_id TEXT NOT NULL,
                     provider_id TEXT NOT NULL,
                     dataset_id TEXT NOT NULL,
                     task_type TEXT,
                     output_root TEXT NOT NULL,
                     primary_artifact_path TEXT NOT NULL,
                     provenance_path TEXT,
                     size_bytes INTEGER NOT NULL,
                     sha256 TEXT NOT NULL,
                     bbox_json TEXT,
                     license_id TEXT NOT NULL,
                     added_at TEXT NOT NULL,
                     UNIQUE(project_id, task_id)
                 ) STRICT;
                 CREATE INDEX IF NOT EXISTS idx_experiment_project_items_project
                     ON experiment_project_items(project_id, added_at, item_id);",
            )
            .map_err(StoreError::Database)?;
        Ok(Self { connection })
    }

    pub fn load_or_init_default(
        &mut self,
        now: &str,
    ) -> Result<(ExperimentProjectRecord, Vec<ExperimentProjectItemRecord>), StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Database)?;
        ensure_default_project_tx(&tx, now)?;
        let result = load_project_and_items_tx(&tx, DEFAULT_EXPERIMENT_PROJECT_ID)?;
        tx.commit().map_err(StoreError::Database)?;
        Ok(result)
    }

    pub fn update_project_aoi(
        &mut self,
        project_id: &str,
        aoi_display_name: Option<&str>,
        aoi_bbox_json: Option<&str>,
        aoi_geojson: Option<&str>,
        updated_at: &str,
    ) -> Result<(), StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Database)?;
        ensure_project_tx(&tx, project_id, updated_at)?;
        tx.execute(
            "UPDATE experiment_projects
             SET aoi_display_name = ?2,
                 aoi_bbox_json = ?3,
                 aoi_geojson = ?4,
                 updated_at = ?5
             WHERE project_id = ?1",
            params![
                project_id,
                aoi_display_name,
                aoi_bbox_json,
                aoi_geojson,
                updated_at,
            ],
        )
        .map_err(StoreError::Database)?;
        tx.commit().map_err(StoreError::Database)?;
        Ok(())
    }

    pub fn upsert_item(
        &mut self,
        input: &ExperimentProjectUpsertItem,
    ) -> Result<(ExperimentProjectRecord, Vec<ExperimentProjectItemRecord>), StoreError> {
        if input.task_id.trim().is_empty() || input.primary_artifact_path.trim().is_empty() {
            return Err(StoreError::invalid(
                "experiment project item requires non-empty task_id and primary_artifact_path",
            ));
        }
        let size_i64 = i64::try_from(input.size_bytes).map_err(|_| {
            StoreError::invalid("experiment project item size exceeds SQLite INTEGER range")
        })?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Database)?;
        ensure_project_tx(&tx, &input.project_id, &input.timestamp)?;

        let existing_item_id = tx
            .query_row(
                "SELECT item_id FROM experiment_project_items WHERE project_id = ?1 AND task_id = ?2",
                params![input.project_id, input.task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::Database)?;

        if let Some(existing_id) = existing_item_id {
            tx.execute(
                "UPDATE experiment_project_items
                 SET provider_id = ?2,
                     dataset_id = ?3,
                     task_type = ?4,
                     output_root = ?5,
                     primary_artifact_path = ?6,
                     provenance_path = ?7,
                     size_bytes = ?8,
                     sha256 = ?9,
                     bbox_json = ?10,
                     license_id = ?11,
                     added_at = ?12
                 WHERE item_id = ?1",
                params![
                    existing_id,
                    input.provider_id,
                    input.dataset_id,
                    input.task_type,
                    input.output_root,
                    input.primary_artifact_path,
                    input.provenance_path,
                    size_i64,
                    input.sha256,
                    input.bbox_json,
                    input.license_id,
                    input.timestamp,
                ],
            )
            .map_err(StoreError::Database)?;
        } else {
            let count = tx
                .query_row(
                    "SELECT COUNT(*) FROM experiment_project_items WHERE project_id = ?1",
                    params![input.project_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(StoreError::Database)?;
            if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_EXPERIMENT_PROJECT_ITEMS {
                return Err(StoreError::invalid("实验工程最多支持纳管 32 个数据项"));
            }
            tx.execute(
                "INSERT INTO experiment_project_items (
                     item_id, project_id, task_id, provider_id, dataset_id, task_type,
                     output_root, primary_artifact_path, provenance_path, size_bytes,
                     sha256, bbox_json, license_id, added_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    input.item_id,
                    input.project_id,
                    input.task_id,
                    input.provider_id,
                    input.dataset_id,
                    input.task_type,
                    input.output_root,
                    input.primary_artifact_path,
                    input.provenance_path,
                    size_i64,
                    input.sha256,
                    input.bbox_json,
                    input.license_id,
                    input.timestamp,
                ],
            )
            .map_err(StoreError::Database)?;
        }

        tx.execute(
            "UPDATE experiment_projects SET updated_at = ?2 WHERE project_id = ?1",
            params![input.project_id, input.timestamp],
        )
        .map_err(StoreError::Database)?;

        let result = load_project_and_items_tx(&tx, &input.project_id)?;
        tx.commit().map_err(StoreError::Database)?;
        Ok(result)
    }

    pub fn remove_item(
        &mut self,
        project_id: &str,
        item_id_or_task_id: &str,
        updated_at: &str,
    ) -> Result<(ExperimentProjectRecord, Vec<ExperimentProjectItemRecord>), StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Database)?;
        ensure_project_tx(&tx, project_id, updated_at)?;
        let affected = tx
            .execute(
                "DELETE FROM experiment_project_items
                 WHERE project_id = ?1 AND (item_id = ?2 OR task_id = ?2)",
                params![project_id, item_id_or_task_id],
            )
            .map_err(StoreError::Database)?;
        if affected > 0 {
            tx.execute(
                "UPDATE experiment_projects SET updated_at = ?2 WHERE project_id = ?1",
                params![project_id, updated_at],
            )
            .map_err(StoreError::Database)?;
        }
        let result = load_project_and_items_tx(&tx, project_id)?;
        tx.commit().map_err(StoreError::Database)?;
        Ok(result)
    }
}

fn ensure_default_project_tx(tx: &rusqlite::Transaction<'_>, now: &str) -> Result<(), StoreError> {
    ensure_project_tx(tx, DEFAULT_EXPERIMENT_PROJECT_ID, now)
}

fn ensure_project_tx(
    tx: &rusqlite::Transaction<'_>,
    project_id: &str,
    now: &str,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO experiment_projects (
             project_id, title, aoi_display_name, aoi_bbox_json, aoi_geojson, created_at, updated_at
         ) VALUES (?1, ?2, NULL, NULL, NULL, ?3, ?3)",
        params![project_id, DEFAULT_EXPERIMENT_PROJECT_TITLE, now],
    )
    .map_err(StoreError::Database)?;
    Ok(())
}

fn load_project_and_items_tx(
    tx: &rusqlite::Transaction<'_>,
    project_id: &str,
) -> Result<(ExperimentProjectRecord, Vec<ExperimentProjectItemRecord>), StoreError> {
    let project = tx
        .query_row(
            "SELECT project_id, title, aoi_display_name, aoi_bbox_json, aoi_geojson, created_at, updated_at
             FROM experiment_projects
             WHERE project_id = ?1",
            params![project_id],
            |row| {
                Ok(ExperimentProjectRecord {
                    project_id: row.get(0)?,
                    title: row.get(1)?,
                    aoi_display_name: row.get(2)?,
                    aoi_bbox_json: row.get(3)?,
                    aoi_geojson: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        )
        .map_err(StoreError::Database)?;

    let mut stmt = tx
        .prepare(
            "SELECT item_id, project_id, task_id, provider_id, dataset_id, task_type,
                    output_root, primary_artifact_path, provenance_path, size_bytes,
                    sha256, bbox_json, license_id, added_at
             FROM experiment_project_items
             WHERE project_id = ?1
             ORDER BY added_at ASC, item_id ASC",
        )
        .map_err(StoreError::Database)?;
    let mut rows = stmt
        .query(params![project_id])
        .map_err(StoreError::Database)?;
    let mut items = Vec::new();
    while let Some(row) = rows.next().map_err(StoreError::Database)? {
        let size_i64: i64 = row.get(9).map_err(StoreError::Database)?;
        let size_bytes = u64::try_from(size_i64).unwrap_or(0);
        items.push(ExperimentProjectItemRecord {
            item_id: row.get(0).map_err(StoreError::Database)?,
            project_id: row.get(1).map_err(StoreError::Database)?,
            task_id: row.get(2).map_err(StoreError::Database)?,
            provider_id: row.get(3).map_err(StoreError::Database)?,
            dataset_id: row.get(4).map_err(StoreError::Database)?,
            task_type: row.get(5).map_err(StoreError::Database)?,
            output_root: row.get(6).map_err(StoreError::Database)?,
            primary_artifact_path: row.get(7).map_err(StoreError::Database)?,
            provenance_path: row.get(8).map_err(StoreError::Database)?,
            size_bytes,
            sha256: row.get(10).map_err(StoreError::Database)?,
            bbox_json: row.get(11).map_err(StoreError::Database)?,
            license_id: row.get(12).map_err(StoreError::Database)?,
            added_at: row.get(13).map_err(StoreError::Database)?,
        });
    }
    Ok((project, items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experiment_project_store_enforces_idempotency_cap_and_remove() {
        let dir =
            std::env::temp_dir().join(format!("insar-store-exp-proj-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db_path =
            ExperimentProjectStore::sibling_path_for_task_store(&dir.join("tasks.sqlite3"));
        assert!(db_path.ends_with("experiment_projects.sqlite3"));

        let mut store = ExperimentProjectStore::open(&db_path).unwrap();
        let (proj, items) = store.load_or_init_default("2026-11-14T00:00:00Z").unwrap();
        assert_eq!(proj.project_id, DEFAULT_EXPERIMENT_PROJECT_ID);
        assert!(items.is_empty());

        for i in 0..MAX_EXPERIMENT_PROJECT_ITEMS {
            let (_p, current) = store
                .upsert_item(&ExperimentProjectUpsertItem {
                    item_id: format!("item-{i}"),
                    project_id: DEFAULT_EXPERIMENT_PROJECT_ID.to_owned(),
                    task_id: format!("task-{i}"),
                    provider_id: "MPC".to_owned(),
                    dataset_id: "sentinel-2-l2a".to_owned(),
                    task_type: Some("optical_derived".to_owned()),
                    output_root: dir.to_string_lossy().to_string(),
                    primary_artifact_path: dir
                        .join(format!("scene-{i}.tif"))
                        .to_string_lossy()
                        .to_string(),
                    provenance_path: None,
                    size_bytes: 128 + i as u64,
                    sha256: format!("sha-{i:064}"),
                    bbox_json: None,
                    license_id: "CC-BY-4.0 / Open".to_owned(),
                    timestamp: format!("2026-11-14T00:00:{i:02}Z"),
                })
                .unwrap();
            assert_eq!(current.len(), i + 1);
        }

        // Re-upserting an existing task_id stays idempotent at 32 items
        let (_p, same_cap) = store
            .upsert_item(&ExperimentProjectUpsertItem {
                item_id: "item-dup".to_owned(),
                project_id: DEFAULT_EXPERIMENT_PROJECT_ID.to_owned(),
                task_id: "task-0".to_owned(),
                provider_id: "FABDEM".to_owned(),
                dataset_id: "dem.fabdem.v1".to_owned(),
                task_type: Some("dem".to_owned()),
                output_root: dir.to_string_lossy().to_string(),
                primary_artifact_path: dir.join("fabdem.tif").to_string_lossy().to_string(),
                provenance_path: None,
                size_bytes: 256,
                sha256: "updated-sha".to_owned(),
                bbox_json: None,
                license_id: "CC BY-NC-SA 4.0".to_owned(),
                timestamp: "2026-11-14T01:00:00Z".to_owned(),
            })
            .unwrap();
        assert_eq!(same_cap.len(), MAX_EXPERIMENT_PROJECT_ITEMS);
        assert!(same_cap
            .iter()
            .any(|x| x.task_id == "task-0" && x.sha256 == "updated-sha"));

        // 33rd distinct task_id fails closed
        let overflow = store.upsert_item(&ExperimentProjectUpsertItem {
            item_id: "item-overflow".to_owned(),
            project_id: DEFAULT_EXPERIMENT_PROJECT_ID.to_owned(),
            task_id: "task-overflow".to_owned(),
            provider_id: "MPC".to_owned(),
            dataset_id: "sentinel-2-l2a".to_owned(),
            task_type: None,
            output_root: dir.to_string_lossy().to_string(),
            primary_artifact_path: dir.join("overflow.tif").to_string_lossy().to_string(),
            provenance_path: None,
            size_bytes: 100,
            sha256: "overflow".to_owned(),
            bbox_json: None,
            license_id: "CC-BY-4.0 / Open".to_owned(),
            timestamp: "2026-11-14T01:01:00Z".to_owned(),
        });
        assert!(overflow.is_err());

        // Remove by task_id reduces count to 31
        let (_p, after_remove) = store
            .remove_item(
                DEFAULT_EXPERIMENT_PROJECT_ID,
                "task-0",
                "2026-11-14T01:02:00Z",
            )
            .unwrap();
        assert_eq!(after_remove.len(), MAX_EXPERIMENT_PROJECT_ITEMS - 1);

        let _ = fs::remove_dir_all(&dir);
    }
}
