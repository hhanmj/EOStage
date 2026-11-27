//! Two connections that open the same new Store at the same time must both
//! succeed: schema creation and migration are decided under the write lock.
//! Before the fix, `TaskStore::open` read `user_version` before taking the
//! IMMEDIATE transaction, so a connection that waited for the lock still
//! migrated from version 0 and failed with "table search_batches already
//! exists" (TASK-REG-012 reverification, 2026-10-20).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_store::{TaskStore, STORE_SCHEMA_VERSION};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(round: usize) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-concurrent-open-{round}-{}-{nonce}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

#[test]
fn connections_opening_the_same_new_store_at_once_all_succeed() {
    const OPENERS: usize = 8;
    const ROUNDS: usize = 25;
    let mut failures = Vec::new();
    for round in 0..ROUNDS {
        let database = TempDatabase::new(round);
        let barrier = Arc::new(Barrier::new(OPENERS));
        let handles = (0..OPENERS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let path = database.0.clone();
                thread::spawn(move || {
                    barrier.wait();
                    TaskStore::open(&path)
                        .map(|store| store.health().map(|health| health.schema_version))
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            match handle.join().unwrap() {
                Ok(Ok(version)) => assert_eq!(version, STORE_SCHEMA_VERSION),
                Ok(Err(error)) | Err(error) => failures.push(format!("round {round}: {error:?}")),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failed opens: {:#?}",
        failures.len(),
        failures
    );
}

#[test]
fn a_store_migrated_by_another_connection_is_not_migrated_again() {
    let database = TempDatabase::new(0);
    let first = TaskStore::open(&database.0).unwrap();
    let tables = first.table_names().unwrap();
    // A second connection finds the schema current and changes nothing.
    let second = TaskStore::open(&database.0).unwrap();
    assert_eq!(
        second.health().unwrap().schema_version,
        STORE_SCHEMA_VERSION
    );
    assert_eq!(second.table_names().unwrap(), tables);
}
