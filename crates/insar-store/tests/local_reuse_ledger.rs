//! Downloader D3 (local-first dedup): the v31 landing ledger and the local
//! reuse decision rows of task items.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreArchiveTaskRequest,
    StoreCreateTaskRequest, StoreDeleteHistoryRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskStateCommandKind,
};
use insar_store::{
    StoreFileIdentity, StoreLandedArtifact, StoreLandingEvidence, StoreLocalReuseDisposition,
    StoreTaskItemLocalReuse, TaskStore, STORE_SCHEMA_VERSION,
};
use serde_json::{json, Map};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-local-reuse-{name}-{}-{nonce}.sqlite3",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn artifact(path_key: &str, size: u64, sha256: Option<&str>) -> StoreLandedArtifact {
    StoreLandedArtifact {
        path_key: path_key.to_owned(),
        provider_id: "asf.sentinel1".to_owned(),
        product_id: "S1-A".to_owned(),
        size_bytes: size,
        sha256: sha256.map(str::to_owned),
        service_checksum: Some((
            "md5".to_owned(),
            "0123456789abcdef0123456789abcdef".to_owned(),
        )),
        identity: StoreFileIdentity {
            platform: "windows".to_owned(),
            volume_id: u64::MAX,
            file_id: 0x0001_0000_0000_002a,
            modified: Some(i128::from(i64::MAX) + 7),
            changed: Some(134_051_234_567_890_123),
            stable: true,
        },
        evidence: StoreLandingEvidence::Landing,
        source_task_id: Some("task-history".to_owned()),
        source_item_id: Some("S1-A".to_owned()),
        verified_at: "2026-10-20T01:00:00Z".to_owned(),
    }
}

fn decision(item_id: &str, disposition: StoreLocalReuseDisposition) -> StoreTaskItemLocalReuse {
    StoreTaskItemLocalReuse {
        task_id: "task-history".to_owned(),
        item_id: item_id.to_owned(),
        disposition,
        size_bytes: Some(4_200_000_000),
        decided_at: "2026-10-20T01:00:01Z".to_owned(),
    }
}

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const LOCAL_REUSE_TABLES: [&str; 3] = [
    "landed_artifacts",
    "task_item_local_reuse",
    "task_local_scan",
];

#[test]
fn a_new_store_has_the_local_reuse_tables_and_a_v30_store_gains_them() {
    let database = TempDatabase::new("migration");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(STORE_SCHEMA_VERSION, 43);
        let tables = store.table_names().unwrap();
        for table in LOCAL_REUSE_TABLES {
            assert!(tables.iter().any(|name| name == table), "{table}");
        }
    }
    {
        // The v30 shape: the same database without the three tables.
        let connection = rusqlite::Connection::open(database.path()).unwrap();
        for table in LOCAL_REUSE_TABLES {
            connection
                .execute_batch(&format!("DROP TABLE {table};"))
                .unwrap();
        }
        // Reconstruct a real v30 shape, including the later Agent tables.
        connection
            .execute_batch(
                "DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;",
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 30).unwrap();
    }
    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    let tables = store.table_names().unwrap();
    for table in LOCAL_REUSE_TABLES {
        assert!(
            tables.iter().any(|name| name == table),
            "{table} after v30 -> v31"
        );
    }
}

#[test]
fn the_ledger_keeps_one_current_row_per_path() {
    let database = TempDatabase::new("ledger");
    let mut store = TaskStore::open(database.path()).unwrap();
    let path = r"D:\下载\SLC\S1-A.zip";
    assert_eq!(store.current_landed_artifact(path).unwrap(), None);
    let first = artifact(path, 4_200_000_000, Some(SHA_A));
    store.record_landed_artifact(&first).unwrap();
    assert_eq!(
        store.current_landed_artifact(path).unwrap(),
        Some(first.clone())
    );

    let mut replaced = artifact(path, 4_200_000_001, Some(SHA_B));
    replaced.service_checksum = None;
    replaced.evidence = StoreLandingEvidence::LedgerRehash;
    replaced.identity.modified = None;
    store.record_landed_artifact(&replaced).unwrap();
    assert_eq!(store.current_landed_artifact(path).unwrap(), Some(replaced));

    assert!(store
        .invalidate_landed_artifact(path, "quarantined")
        .unwrap());
    assert_eq!(store.current_landed_artifact(path).unwrap(), None);
    assert!(!store
        .invalidate_landed_artifact(path, "quarantined")
        .unwrap());
    // A new landing at the same path is current again.
    store.record_landed_artifact(&first).unwrap();
    assert_eq!(store.current_landed_artifact(path).unwrap(), Some(first));

    for invalid in [
        StoreLandedArtifact {
            sha256: Some("ABC".to_owned()),
            ..artifact(path, 1, None)
        },
        StoreLandedArtifact {
            sha256: Some(SHA_A.to_uppercase()),
            ..artifact(path, 1, None)
        },
        StoreLandedArtifact {
            service_checksum: Some(("crc32".to_owned(), "00000000".to_owned())),
            ..artifact(path, 1, None)
        },
        StoreLandedArtifact {
            service_checksum: Some(("md5".to_owned(), SHA_A.to_owned())),
            ..artifact(path, 1, None)
        },
        StoreLandedArtifact {
            path_key: String::new(),
            ..artifact(path, 1, None)
        },
        StoreLandedArtifact {
            identity: StoreFileIdentity {
                platform: "mac".to_owned(),
                ..artifact(path, 1, None).identity
            },
            ..artifact(path, 1, None)
        },
    ] {
        assert!(
            store.record_landed_artifact(&invalid).is_err(),
            "{invalid:?}"
        );
    }
    assert!(store
        .invalidate_landed_artifact(path, "Not A Code")
        .is_err());
}

#[test]
fn decisions_are_recorded_per_item_and_validated() {
    let database = TempDatabase::new("decisions");
    let mut store = TaskStore::open(database.path()).unwrap();
    let reused = decision(
        "S1-B",
        StoreLocalReuseDisposition::Reused {
            grade: "size_and_format".to_owned(),
        },
    );
    let renamed = decision(
        "S1-A",
        StoreLocalReuseDisposition::RenamedConflict {
            reason: "size_mismatch".to_owned(),
            conflict_file_name: "S1-A.zip.eostage-conflict-20261020T010203Z".to_owned(),
        },
    );
    store.record_task_item_local_reuse(&reused).unwrap();
    store.record_task_item_local_reuse(&renamed).unwrap();
    assert_eq!(
        store.task_local_reuse_decisions("task-history").unwrap(),
        vec![renamed.clone(), reused.clone()]
    );
    assert_eq!(
        store.task_item_local_reuse("task-history", "S1-B").unwrap(),
        Some(reused)
    );
    let reacquire = decision("S1-B", StoreLocalReuseDisposition::ReacquireRequested);
    store.record_task_item_local_reuse(&reacquire).unwrap();
    assert_eq!(
        store.task_item_local_reuse("task-history", "S1-B").unwrap(),
        Some(reacquire)
    );
    assert!(store
        .clear_task_item_local_reuse("task-history", "S1-B")
        .unwrap());
    assert!(!store
        .clear_task_item_local_reuse("task-history", "S1-B")
        .unwrap());
    assert_eq!(
        store.task_local_reuse_decisions("task-history").unwrap(),
        vec![renamed]
    );

    for invalid in [
        StoreLocalReuseDisposition::Reused {
            grade: "guess".to_owned(),
        },
        StoreLocalReuseDisposition::RenamedConflict {
            reason: "size_mismatch".to_owned(),
            conflict_file_name: r"..\S1-A.zip.eostage-conflict-20261020T010203Z".to_owned(),
        },
        StoreLocalReuseDisposition::RenamedConflict {
            reason: "size_mismatch".to_owned(),
            conflict_file_name: "S1-A.zip.old".to_owned(),
        },
        StoreLocalReuseDisposition::RenamedConflict {
            reason: "Size Mismatch".to_owned(),
            conflict_file_name: "S1-A.zip.eostage-conflict-20261020T010203Z".to_owned(),
        },
    ] {
        assert!(store
            .record_task_item_local_reuse(&decision("S1-C", invalid))
            .is_err());
    }
}

fn seed_completed_task(store: &mut TaskStore, output_root: &Path) {
    let candidates = vec![SnapshotItem {
        item_id: "S1-A".to_owned(),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: "S1-A".to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }];
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "batch-history".to_owned(),
                provider_id: "asf.sentinel1".to_owned(),
                created_at: "2024-01-15T00:00:00Z".to_owned(),
                aoi_name: "测试区域".to_owned(),
                aoi_geojson: Some(
                    json!({
                        "type": "Polygon",
                        "coordinates": [[[103.0, 26.8], [104.0, 26.8], [104.0, 27.8], [103.0, 27.8], [103.0, 26.8]]]
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
                query: json!({"product_type": "SLC"}).as_object().unwrap().clone(),
                search_stats: None,
                candidates,
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-history".to_owned(),
                checked_item_ids: vec!["S1-A".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-history".to_owned(),
            batch_id: "batch-history".to_owned(),
            selected_item_ids: vec!["S1-A".to_owned()],
            checked_item_ids: vec!["S1-A".to_owned()],
            page_index: 0,
            page_size: 24,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Sentinel1Slc,
            concurrency: 1,
            created_at: "2024-01-15T01:00:00Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-history".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkSkipped,
                item_ids: vec!["S1-A".to_owned()],
            },
            event_id: "skip-1".to_owned(),
            occurred_at: "2024-01-15T02:00:00Z".to_owned(),
        })
        .unwrap();
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-history".to_owned(),
            expected_revision: 1,
            archived_at: "2024-01-15T02:00:01Z".to_owned(),
        })
        .unwrap();
}

#[test]
fn deleting_history_forgets_decisions_and_detaches_the_ledger() {
    let database = TempDatabase::new("history");
    let output_root = database.path().parent().unwrap().join("local-reuse-output");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_completed_task(&mut store, &output_root);
    let path = output_root.join("SLC").join("S1-A.zip");
    let path = path.to_string_lossy().into_owned();
    store
        .record_landed_artifact(&artifact(&path, 10, Some(SHA_A)))
        .unwrap();
    store
        .record_task_item_local_reuse(&decision(
            "S1-A",
            StoreLocalReuseDisposition::Reused {
                grade: "ledger_identity".to_owned(),
            },
        ))
        .unwrap();
    store
        .delete_history(&StoreDeleteHistoryRequest {
            task_id: "task-history".to_owned(),
            deleted_at: "2024-01-15T02:00:02Z".to_owned(),
        })
        .unwrap();
    assert!(store
        .task_local_reuse_decisions("task-history")
        .unwrap()
        .is_empty());
    let kept = store
        .current_landed_artifact(&path)
        .unwrap()
        .expect("ledger facts stay");
    assert_eq!(kept.source_task_id, None);
    assert_eq!(kept.source_item_id, None);
    assert_eq!(kept.sha256.as_deref(), Some(SHA_A));
}
