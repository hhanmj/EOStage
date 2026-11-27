//! Downloader D3 (local-first dedup): the landing ledger and the local reuse
//! decision of each task item.
//!
//! The ledger keeps, for one planned destination path, the facts about the
//! file last verified there: its size, SHA-256 when known, the service
//! checksum it matched, and its identity on the volume (volume, file id, write
//! and change time). A later decision reuses that file without reading it
//! while its identity is unchanged. The ledger states facts about files; it
//! is not task truth and never decides a lifecycle.
//!
//! A decision row records how one task item met an existing file: reused
//! (with the strength of its evidence), renamed aside as a conflict (the
//! file name it now has), or a reacquisition the user asked for. Task pages
//! project these rows; deleting a task's history deletes them.

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::{StoreError, TaskStore};

pub(crate) const SCHEMA_V31_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS landed_artifacts (
    path_key TEXT PRIMARY KEY NOT NULL CHECK(length(path_key) BETWEEN 1 AND 4096),
    state TEXT NOT NULL CHECK(state IN ('current', 'invalidated')),
    invalidated_reason TEXT CHECK(
        (state = 'current' AND invalidated_reason IS NULL)
        OR (state = 'invalidated' AND length(invalidated_reason) BETWEEN 1 AND 64)
    ),
    provider_id TEXT NOT NULL CHECK(length(provider_id) BETWEEN 1 AND 128),
    product_id TEXT NOT NULL CHECK(length(product_id) BETWEEN 1 AND 512),
    size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
    sha256 TEXT CHECK(sha256 IS NULL OR (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*')),
    service_checksum_algorithm TEXT CHECK(
        service_checksum_algorithm IS NULL
        OR service_checksum_algorithm IN ('md5', 'sha1', 'sha256', 'sha512')
    ),
    service_checksum_value TEXT CHECK(
        (service_checksum_algorithm IS NULL) = (service_checksum_value IS NULL)
        AND (service_checksum_value IS NULL OR service_checksum_value NOT GLOB '*[^0-9a-f]*')
    ),
    identity_platform TEXT NOT NULL CHECK(identity_platform IN ('windows', 'unix')),
    volume_id TEXT NOT NULL CHECK(length(volume_id) BETWEEN 1 AND 20),
    file_id TEXT NOT NULL CHECK(length(file_id) BETWEEN 1 AND 20),
    modified TEXT CHECK(modified IS NULL OR length(modified) BETWEEN 1 AND 40),
    changed TEXT CHECK(changed IS NULL OR length(changed) BETWEEN 1 AND 40),
    identity_stable INTEGER NOT NULL CHECK(identity_stable IN (0, 1)),
    evidence TEXT NOT NULL CHECK(evidence IN ('landing', 'service_checksum', 'ledger_rehash')),
    source_task_id TEXT CHECK(source_task_id IS NULL OR length(source_task_id) BETWEEN 1 AND 256),
    source_item_id TEXT CHECK(source_item_id IS NULL OR length(source_item_id) BETWEEN 1 AND 512),
    recorded_at TEXT NOT NULL CHECK(length(recorded_at) BETWEEN 1 AND 64),
    verified_at TEXT NOT NULL CHECK(length(verified_at) BETWEEN 1 AND 64)
) STRICT;

CREATE TABLE IF NOT EXISTS task_item_local_reuse (
    task_id TEXT NOT NULL CHECK(length(task_id) BETWEEN 1 AND 256),
    item_id TEXT NOT NULL CHECK(length(item_id) BETWEEN 1 AND 512),
    disposition TEXT NOT NULL CHECK(
        disposition IN ('reused', 'renamed_conflict', 'reacquire_requested')
    ),
    grade TEXT CHECK(
        (disposition = 'reused' AND grade IN (
            'service_checksum', 'ledger_identity', 'ledger_rehash', 'size_and_format'
        ))
        OR (disposition <> 'reused' AND grade IS NULL)
    ),
    conflict_reason TEXT CHECK(
        (disposition = 'renamed_conflict' AND length(conflict_reason) BETWEEN 1 AND 64)
        OR (disposition <> 'renamed_conflict' AND conflict_reason IS NULL)
    ),
    conflict_file_name TEXT CHECK(
        (disposition = 'renamed_conflict' AND length(conflict_file_name) BETWEEN 1 AND 512)
        OR (disposition <> 'renamed_conflict' AND conflict_file_name IS NULL)
    ),
    size_bytes INTEGER CHECK(size_bytes IS NULL OR size_bytes >= 0),
    decided_at TEXT NOT NULL CHECK(length(decided_at) BETWEEN 1 AND 64),
    PRIMARY KEY(task_id, item_id)
) STRICT;

CREATE TABLE IF NOT EXISTS task_local_scan (
    task_id TEXT PRIMARY KEY NOT NULL CHECK(length(task_id) BETWEEN 1 AND 256),
    state TEXT NOT NULL CHECK(state IN ('pending', 'scanning', 'done', 'stopped')),
    scanned_items INTEGER NOT NULL CHECK(scanned_items >= 0),
    total_items INTEGER NOT NULL CHECK(total_items >= scanned_items),
    updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 1 AND 64)
) STRICT;
"#;

/// Downloader D3-2: most items one pre-scan batch commits.
pub const STORE_LOCAL_REUSE_BATCH_LIMIT: usize = 50;

/// Downloader D3-2: one queued item the local pre-scan proved reusable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreLocalReuseBatchItem {
    pub item_id: String,
    /// The planned destination (stored in the item's skipped result only).
    pub destination: std::path::PathBuf,
    pub size_bytes: u64,
    /// The evidence grade (service_checksum, ledger_identity, ledger_rehash,
    /// size_and_format).
    pub grade: String,
    /// The ledger row a digest proved, when there is one.
    pub landed: Option<StoreLandedArtifact>,
}

/// Downloader D3-2: items of one task the pre-scan commits together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreLocalReuseBatch {
    pub task_id: String,
    pub items: Vec<StoreLocalReuseBatchItem>,
    pub event_id: String,
    pub occurred_at: String,
}

pub(crate) fn validate_local_reuse_batch(batch: &StoreLocalReuseBatch) -> Result<(), StoreError> {
    let mut seen = std::collections::BTreeSet::new();
    if !bounded(&batch.task_id, 256)
        || !bounded(&batch.event_id, 256)
        || !bounded(&batch.occurred_at, 64)
        || batch.items.is_empty()
        || batch.items.len() > STORE_LOCAL_REUSE_BATCH_LIMIT
        || batch.items.iter().any(|item| {
            !bounded(&item.item_id, 512)
                || !seen.insert(item.item_id.as_str())
                || item.destination.as_os_str().is_empty()
                || !matches!(
                    item.grade.as_str(),
                    "service_checksum" | "ledger_identity" | "ledger_rehash" | "size_and_format"
                )
        })
    {
        return Err(StoreError::invalid("invalid local reuse batch"));
    }
    Ok(())
}

/// Downloader D3-2: where a task's local pre-scan stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreLocalScanState {
    Pending,
    Scanning,
    Done,
    /// Paused, cancelled or failed; the gate released what was left.
    Stopped,
}

impl StoreLocalScanState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Scanning => "scanning",
            Self::Done => "done",
            Self::Stopped => "stopped",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "pending" => Ok(Self::Pending),
            "scanning" => Ok(Self::Scanning),
            "done" => Ok(Self::Done),
            "stopped" => Ok(Self::Stopped),
            _ => Err(StoreError::invalid("unknown local scan state")),
        }
    }
}

/// Downloader D3-2: the progress row of a task's local pre-scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreTaskLocalScan {
    pub task_id: String,
    pub state: StoreLocalScanState,
    pub scanned_items: u64,
    pub total_items: u64,
    pub updated_at: String,
}

/// The identity of a file on its volume, as the ledger records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreFileIdentity {
    /// "windows" or "unix".
    pub platform: String,
    pub volume_id: u64,
    pub file_id: u64,
    /// Last write time in the platform's units.
    pub modified: Option<i128>,
    /// Last change time in the platform's units.
    pub changed: Option<i128>,
    /// Whether the volume keeps identities stable (NTFS, ReFS, local Unix).
    pub stable: bool,
}

/// What verified the file a ledger row describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreLandingEvidence {
    /// The download that installed it verified it while landing.
    Landing,
    /// A local digest matched the Provider's checksum.
    ServiceChecksum,
    /// A local SHA-256 matched the ledger's earlier SHA-256.
    LedgerRehash,
}

impl StoreLandingEvidence {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Landing => "landing",
            Self::ServiceChecksum => "service_checksum",
            Self::LedgerRehash => "ledger_rehash",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "landing" => Ok(Self::Landing),
            "service_checksum" => Ok(Self::ServiceChecksum),
            "ledger_rehash" => Ok(Self::LedgerRehash),
            _ => Err(StoreError::invalid("unknown landing ledger evidence")),
        }
    }
}

/// One ledger row: the verified file at one planned destination path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreLandedArtifact {
    /// The planned destination path, as the download engine plans it.
    pub path_key: String,
    pub provider_id: String,
    pub product_id: String,
    pub size_bytes: u64,
    /// Lowercase hex SHA-256 when it was computed.
    pub sha256: Option<String>,
    /// The Provider checksum (algorithm, lowercase hex value) it matched.
    pub service_checksum: Option<(String, String)>,
    pub identity: StoreFileIdentity,
    pub evidence: StoreLandingEvidence,
    pub source_task_id: Option<String>,
    pub source_item_id: Option<String>,
    pub verified_at: String,
}

/// How one task item met an existing file at its planned destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreLocalReuseDisposition {
    /// Reused without downloading; `grade` names the evidence strength.
    Reused { grade: String },
    /// Renamed aside before downloading; `conflict_file_name` is its new name.
    RenamedConflict {
        reason: String,
        conflict_file_name: String,
    },
    /// The user asked to download it again instead of reusing it.
    ReacquireRequested,
}

/// The local reuse decision of one task item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreTaskItemLocalReuse {
    pub task_id: String,
    pub item_id: String,
    pub disposition: StoreLocalReuseDisposition,
    pub size_bytes: Option<u64>,
    pub decided_at: String,
}

const REUSE_GRADES: [&str; 4] = [
    "service_checksum",
    "ledger_identity",
    "ledger_rehash",
    "size_and_format",
];
const CHECKSUM_ALGORITHMS: [(&str, usize); 4] =
    [("md5", 32), ("sha1", 40), ("sha256", 64), ("sha512", 128)];

fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.chars().count() <= max && !value.contains('\0')
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn code(value: &str) -> bool {
    bounded(value, 64) && value.bytes().all(|byte| matches!(byte, b'a'..=b'z' | b'_'))
}

impl StoreLandedArtifact {
    fn validate(&self) -> Result<(), StoreError> {
        let identity = &self.identity;
        if !bounded(&self.path_key, 4096)
            || !bounded(&self.provider_id, 128)
            || !bounded(&self.product_id, 512)
            || !bounded(&self.verified_at, 64)
            || self.size_bytes > i64::MAX as u64
            || self
                .sha256
                .as_deref()
                .is_some_and(|value| !lower_hex(value, 64))
            || self
                .service_checksum
                .as_ref()
                .is_some_and(|(algorithm, value)| {
                    !CHECKSUM_ALGORITHMS
                        .iter()
                        .any(|(name, length)| name == algorithm && lower_hex(value, *length))
                })
            || !matches!(identity.platform.as_str(), "windows" | "unix")
            || self
                .source_task_id
                .as_deref()
                .is_some_and(|value| !bounded(value, 256))
            || self
                .source_item_id
                .as_deref()
                .is_some_and(|value| !bounded(value, 512))
        {
            return Err(StoreError::invalid("invalid landing ledger row"));
        }
        Ok(())
    }
}

impl StoreTaskItemLocalReuse {
    fn validate(&self) -> Result<(), StoreError> {
        let valid = bounded(&self.task_id, 256)
            && bounded(&self.item_id, 512)
            && bounded(&self.decided_at, 64)
            && self.size_bytes.is_none_or(|size| size <= i64::MAX as u64)
            && match &self.disposition {
                StoreLocalReuseDisposition::Reused { grade } => {
                    REUSE_GRADES.contains(&grade.as_str())
                }
                StoreLocalReuseDisposition::RenamedConflict {
                    reason,
                    conflict_file_name,
                } => {
                    code(reason)
                        && bounded(conflict_file_name, 512)
                        && conflict_file_name.contains(".eostage-conflict-")
                        && !conflict_file_name.contains(['/', '\\', ':'])
                }
                StoreLocalReuseDisposition::ReacquireRequested => true,
            };
        if !valid {
            return Err(StoreError::invalid("invalid local reuse decision"));
        }
        Ok(())
    }
}

fn optional_i128(value: Option<String>) -> Result<Option<i128>, StoreError> {
    value
        .map(|text| {
            text.parse::<i128>()
                .map_err(|_| StoreError::invalid("invalid landing ledger time"))
        })
        .transpose()
}

impl TaskStore {
    /// The current ledger row of a planned destination path, if any.
    pub fn current_landed_artifact(
        &self,
        path_key: &str,
    ) -> Result<Option<StoreLandedArtifact>, StoreError> {
        let row = self
            .connection
            .query_row(
                "SELECT provider_id, product_id, size_bytes, sha256, \
                 service_checksum_algorithm, service_checksum_value, identity_platform, \
                 volume_id, file_id, modified, changed, identity_stable, evidence, \
                 source_task_id, source_item_id, verified_at \
                 FROM landed_artifacts WHERE path_key = ?1 AND state = 'current'",
                [path_key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, i64>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, String>(15)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            provider_id,
            product_id,
            size_bytes,
            sha256,
            algorithm,
            value,
            platform,
            volume_id,
            file_id,
            modified,
            changed,
            stable,
            evidence,
            source_task_id,
            source_item_id,
            verified_at,
        )) = row
        else {
            return Ok(None);
        };
        let parse_u64 = |text: &str| {
            text.parse::<u64>()
                .map_err(|_| StoreError::invalid("invalid landing ledger identity"))
        };
        Ok(Some(StoreLandedArtifact {
            path_key: path_key.to_owned(),
            provider_id,
            product_id,
            size_bytes: u64::try_from(size_bytes)
                .map_err(|_| StoreError::invalid("invalid landing ledger size"))?,
            sha256,
            service_checksum: algorithm.zip(value),
            identity: StoreFileIdentity {
                platform,
                volume_id: parse_u64(&volume_id)?,
                file_id: parse_u64(&file_id)?,
                modified: optional_i128(modified)?,
                changed: optional_i128(changed)?,
                stable: stable == 1,
            },
            evidence: StoreLandingEvidence::parse(&evidence)?,
            source_task_id,
            source_item_id,
            verified_at,
        }))
    }

    /// Records the verified file at a planned destination path; replaces any
    /// earlier row of that path.
    pub fn record_landed_artifact(
        &mut self,
        artifact: &StoreLandedArtifact,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        upsert_landed_artifact(&transaction, artifact)?;
        transaction.commit()?;
        Ok(())
    }

    /// Marks the ledger row of a path invalid (the file was renamed aside,
    /// failed verification or vanished); true when a current row changed.
    pub fn invalidate_landed_artifact(
        &mut self,
        path_key: &str,
        reason: &str,
    ) -> Result<bool, StoreError> {
        if !code(reason) {
            return Err(StoreError::invalid(
                "invalid landing ledger invalidation reason",
            ));
        }
        let changed = self.connection.execute(
            "UPDATE landed_artifacts SET state = 'invalidated', invalidated_reason = ?2 \
             WHERE path_key = ?1 AND state = 'current'",
            params![path_key, reason],
        )?;
        Ok(changed == 1)
    }

    /// Downloader D3-2: records (or replaces) the pre-scan progress row of a task.
    pub fn record_task_local_scan(&mut self, scan: &StoreTaskLocalScan) -> Result<(), StoreError> {
        if !bounded(&scan.task_id, 256)
            || !bounded(&scan.updated_at, 64)
            || scan.scanned_items > scan.total_items
            || i64::try_from(scan.total_items).is_err()
        {
            return Err(StoreError::invalid("invalid local scan progress"));
        }
        self.connection.execute(
            "INSERT INTO task_local_scan(task_id, state, scanned_items, total_items, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(task_id) DO UPDATE SET state = excluded.state, \
             scanned_items = excluded.scanned_items, total_items = excluded.total_items, \
             updated_at = excluded.updated_at",
            params![
                scan.task_id,
                scan.state.as_str(),
                scan.scanned_items as i64,
                scan.total_items as i64,
                scan.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Downloader D3-2: the pre-scan progress row of a task, if any.
    pub fn task_local_scan(&self, task_id: &str) -> Result<Option<StoreTaskLocalScan>, StoreError> {
        self.connection
            .query_row(
                "SELECT state, scanned_items, total_items, updated_at FROM task_local_scan \
                 WHERE task_id = ?1",
                [task_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?
            .map(|(state, scanned, total, updated_at)| {
                Ok(StoreTaskLocalScan {
                    task_id: task_id.to_owned(),
                    state: StoreLocalScanState::parse(&state)?,
                    scanned_items: u64::try_from(scanned)
                        .map_err(|_| StoreError::invalid("negative scanned items"))?,
                    total_items: u64::try_from(total)
                        .map_err(|_| StoreError::invalid("negative total items"))?,
                    updated_at,
                })
            })
            .transpose()
    }

    /// Records (or replaces) the local reuse decision of one task item.
    pub fn record_task_item_local_reuse(
        &mut self,
        decision: &StoreTaskItemLocalReuse,
    ) -> Result<(), StoreError> {
        upsert_task_item_local_reuse(&self.connection, decision)
    }

    /// Removes the local reuse decision of one task item; true when one existed.
    pub fn clear_task_item_local_reuse(
        &mut self,
        task_id: &str,
        item_id: &str,
    ) -> Result<bool, StoreError> {
        let removed = self.connection.execute(
            "DELETE FROM task_item_local_reuse WHERE task_id = ?1 AND item_id = ?2",
            params![task_id, item_id],
        )?;
        Ok(removed == 1)
    }

    /// The local reuse decision of one task item, if any.
    pub fn task_item_local_reuse(
        &self,
        task_id: &str,
        item_id: &str,
    ) -> Result<Option<StoreTaskItemLocalReuse>, StoreError> {
        Ok(self
            .task_local_reuse_decisions_where(task_id, Some(item_id))?
            .into_iter()
            .next())
    }

    /// Every local reuse decision of a task, ordered by item id.
    pub fn task_local_reuse_decisions(
        &self,
        task_id: &str,
    ) -> Result<Vec<StoreTaskItemLocalReuse>, StoreError> {
        self.task_local_reuse_decisions_where(task_id, None)
    }

    fn task_local_reuse_decisions_where(
        &self,
        task_id: &str,
        item_id: Option<&str>,
    ) -> Result<Vec<StoreTaskItemLocalReuse>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT item_id, disposition, grade, conflict_reason, conflict_file_name, \
             size_bytes, decided_at FROM task_item_local_reuse \
             WHERE task_id = ?1 AND (?2 IS NULL OR item_id = ?2) ORDER BY item_id",
        )?;
        let rows = statement
            .query_map(params![task_id, item_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(
                    item_id,
                    disposition,
                    grade,
                    reason,
                    conflict_file_name,
                    size_bytes,
                    decided_at,
                )| {
                    let disposition =
                        match (disposition.as_str(), grade, reason, conflict_file_name) {
                            ("reused", Some(grade), None, None) => {
                                StoreLocalReuseDisposition::Reused { grade }
                            }
                            ("renamed_conflict", None, Some(reason), Some(conflict_file_name)) => {
                                StoreLocalReuseDisposition::RenamedConflict {
                                    reason,
                                    conflict_file_name,
                                }
                            }
                            ("reacquire_requested", None, None, None) => {
                                StoreLocalReuseDisposition::ReacquireRequested
                            }
                            _ => {
                                return Err(StoreError::invalid(
                                    "invalid stored local reuse decision",
                                ))
                            }
                        };
                    Ok(StoreTaskItemLocalReuse {
                        task_id: task_id.to_owned(),
                        item_id,
                        disposition,
                        size_bytes: size_bytes
                            .map(u64::try_from)
                            .transpose()
                            .map_err(|_| StoreError::invalid("invalid stored local reuse size"))?,
                        decided_at,
                    })
                },
            )
            .collect()
    }
}

/// Downloader D3-2: the pre-scan progress and reuse counts of each task
/// that has either (the task card's progress line).
pub(crate) fn load_task_local_scan_summaries(
    connection: &rusqlite::Connection,
    task_ids: &[String],
) -> Result<std::collections::HashMap<String, crate::StoreTaskLocalScanSummary>, StoreError> {
    let mut scans = connection.prepare_cached(
        "SELECT state, scanned_items, total_items FROM task_local_scan WHERE task_id = ?1",
    )?;
    let mut reuse = connection.prepare_cached(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN grade = 'size_and_format' THEN 1 ELSE 0 END), 0) \
         FROM task_item_local_reuse WHERE task_id = ?1 AND disposition = 'reused'",
    )?;
    let mut summaries = std::collections::HashMap::new();
    for task_id in task_ids {
        let scan = scans
            .query_row([task_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .optional()?;
        let (reused, unverified) = reuse.query_row([task_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        let count = |value: i64| {
            u64::try_from(value).map_err(|_| StoreError::invalid("negative local scan count"))
        };
        let reused_count = count(reused)?;
        if scan.is_none() && reused_count == 0 {
            continue;
        }
        let (state, scanned_items, total_items) = match scan {
            Some((state, scanned, total)) => (state, count(scanned)?, count(total)?),
            None => ("none".to_owned(), 0, 0),
        };
        summaries.insert(
            task_id.clone(),
            crate::StoreTaskLocalScanSummary {
                state,
                scanned_items,
                total_items,
                reused_count,
                unverified_count: count(unverified)?,
            },
        );
    }
    Ok(summaries)
}

/// Writes a ledger row inside the caller's transaction (see
/// [`TaskStore::record_landed_artifact`]).
pub(crate) fn upsert_landed_artifact(
    connection: &rusqlite::Connection,
    artifact: &StoreLandedArtifact,
) -> Result<(), StoreError> {
    artifact.validate()?;
    connection.execute(
        "INSERT INTO landed_artifacts(path_key, state, invalidated_reason, provider_id, \
         product_id, size_bytes, sha256, service_checksum_algorithm, service_checksum_value, \
         identity_platform, volume_id, file_id, modified, changed, identity_stable, evidence, \
         source_task_id, source_item_id, recorded_at, verified_at) \
         VALUES (?1, 'current', NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
         ?15, ?16, ?17, ?17) \
         ON CONFLICT(path_key) DO UPDATE SET state = 'current', invalidated_reason = NULL, \
         provider_id = excluded.provider_id, product_id = excluded.product_id, \
         size_bytes = excluded.size_bytes, sha256 = excluded.sha256, \
         service_checksum_algorithm = excluded.service_checksum_algorithm, \
         service_checksum_value = excluded.service_checksum_value, \
         identity_platform = excluded.identity_platform, volume_id = excluded.volume_id, \
         file_id = excluded.file_id, modified = excluded.modified, \
         changed = excluded.changed, identity_stable = excluded.identity_stable, \
         evidence = excluded.evidence, source_task_id = excluded.source_task_id, \
         source_item_id = excluded.source_item_id, recorded_at = excluded.recorded_at, \
         verified_at = excluded.verified_at",
        params![
            artifact.path_key,
            artifact.provider_id,
            artifact.product_id,
            artifact.size_bytes as i64,
            artifact.sha256,
            artifact
                .service_checksum
                .as_ref()
                .map(|(algorithm, _)| algorithm),
            artifact.service_checksum.as_ref().map(|(_, value)| value),
            artifact.identity.platform,
            artifact.identity.volume_id.to_string(),
            artifact.identity.file_id.to_string(),
            artifact.identity.modified.map(|value| value.to_string()),
            artifact.identity.changed.map(|value| value.to_string()),
            i64::from(artifact.identity.stable),
            artifact.evidence.as_str(),
            artifact.source_task_id,
            artifact.source_item_id,
            artifact.verified_at,
        ],
    )?;
    Ok(())
}

/// Writes a decision row inside the caller's transaction (see
/// [`TaskStore::record_task_item_local_reuse`]).
pub(crate) fn upsert_task_item_local_reuse(
    connection: &rusqlite::Connection,
    decision: &StoreTaskItemLocalReuse,
) -> Result<(), StoreError> {
    decision.validate()?;
    let (disposition, grade, reason, conflict_file_name) = match &decision.disposition {
        StoreLocalReuseDisposition::Reused { grade } => {
            ("reused", Some(grade.as_str()), None, None)
        }
        StoreLocalReuseDisposition::RenamedConflict {
            reason,
            conflict_file_name,
        } => (
            "renamed_conflict",
            None,
            Some(reason.as_str()),
            Some(conflict_file_name.as_str()),
        ),
        StoreLocalReuseDisposition::ReacquireRequested => ("reacquire_requested", None, None, None),
    };
    connection.execute(
        "INSERT INTO task_item_local_reuse(task_id, item_id, disposition, grade, \
         conflict_reason, conflict_file_name, size_bytes, decided_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(task_id, item_id) DO UPDATE SET disposition = excluded.disposition, \
         grade = excluded.grade, conflict_reason = excluded.conflict_reason, \
         conflict_file_name = excluded.conflict_file_name, size_bytes = excluded.size_bytes, \
         decided_at = excluded.decided_at",
        params![
            decision.task_id,
            decision.item_id,
            disposition,
            grade,
            reason,
            conflict_file_name,
            decision.size_bytes.map(|size| size as i64),
            decision.decided_at,
        ],
    )?;
    Ok(())
}

/// Records a reacquisition request for each item, inside the caller's
/// transaction (ReacquireItems, or a historical successor created to
/// download items again).
pub(crate) fn record_reacquire_requests(
    transaction: &rusqlite::Transaction<'_>,
    task_id: &str,
    item_ids: &[String],
    requested_at: &str,
) -> Result<(), StoreError> {
    if !bounded(task_id, 256) || !bounded(requested_at, 64) {
        return Err(StoreError::invalid("invalid reacquisition request"));
    }
    for item_id in item_ids {
        if !bounded(item_id, 512) {
            return Err(StoreError::invalid("invalid reacquisition request"));
        }
        transaction.execute(
            "INSERT INTO task_item_local_reuse(task_id, item_id, disposition, grade, \
             conflict_reason, conflict_file_name, size_bytes, decided_at) \
             VALUES (?1, ?2, 'reacquire_requested', NULL, NULL, NULL, NULL, ?3) \
             ON CONFLICT(task_id, item_id) DO UPDATE SET disposition = 'reacquire_requested', \
             grade = NULL, conflict_reason = NULL, conflict_file_name = NULL, \
             size_bytes = NULL, decided_at = excluded.decided_at",
            params![task_id, item_id, requested_at],
        )?;
    }
    Ok(())
}

/// The task page projection of a decision row.
pub(crate) fn local_reuse_projection(
    decision: &StoreTaskItemLocalReuse,
) -> insar_core::StoreTaskItemLocalReuse {
    use insar_core::{LocalReuseDisposition, LocalReuseGrade};
    let grade = |value: &str| match value {
        "service_checksum" => Some(LocalReuseGrade::ServiceChecksum),
        "ledger_identity" => Some(LocalReuseGrade::LedgerIdentity),
        "ledger_rehash" => Some(LocalReuseGrade::LedgerRehash),
        "size_and_format" => Some(LocalReuseGrade::SizeAndFormat),
        _ => None,
    };
    match &decision.disposition {
        StoreLocalReuseDisposition::Reused { grade: value } => {
            insar_core::StoreTaskItemLocalReuse {
                disposition: LocalReuseDisposition::Reused,
                grade: grade(value),
                conflict_reason: None,
                conflict_file_name: None,
            }
        }
        StoreLocalReuseDisposition::RenamedConflict {
            reason,
            conflict_file_name,
        } => insar_core::StoreTaskItemLocalReuse {
            disposition: LocalReuseDisposition::RenamedConflict,
            grade: None,
            conflict_reason: Some(reason.clone()),
            conflict_file_name: Some(conflict_file_name.clone()),
        },
        StoreLocalReuseDisposition::ReacquireRequested => insar_core::StoreTaskItemLocalReuse {
            disposition: LocalReuseDisposition::ReacquireRequested,
            grade: None,
            conflict_reason: None,
            conflict_file_name: None,
        },
    }
}

/// Deletes a task's decision and scan rows and detaches its ledger rows,
/// inside the history deletion transaction. Ledger facts, user files and
/// renamed conflict files stay.
pub(crate) fn forget_task_local_reuse(
    transaction: &rusqlite::Transaction<'_>,
    task_id: &str,
) -> Result<(), StoreError> {
    transaction.execute(
        "DELETE FROM task_item_local_reuse WHERE task_id = ?1",
        [task_id],
    )?;
    transaction.execute("DELETE FROM task_local_scan WHERE task_id = ?1", [task_id])?;
    transaction.execute(
        "UPDATE landed_artifacts SET source_task_id = NULL, source_item_id = NULL \
         WHERE source_task_id = ?1",
        [task_id],
    )?;
    Ok(())
}
