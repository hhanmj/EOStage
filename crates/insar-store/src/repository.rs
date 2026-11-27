use std::collections::{HashMap, HashSet};

use insar_core::{
    catalog_search_cache_key, AssetExecutionAuthority, CatalogSearchQuery,
    FrozenOrbitAssociationV1, FrozenSearchBatch, FrozenSearchStats, FrozenTaskSnapshot,
    NisarSubsetPlanV2, RegionAoiState, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreUpdateWorkbenchRequest, TaskLifecycleStatus,
    TaskOperationDescriptor, TaskRuntimeSnapshot, WorkspaceContextState,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use insar_core::SearchBatchProgressState;

use crate::{
    snapshot_codec::{
        decode_v2_only_snapshot, encode_task_snapshot,
        CompactTaskSnapshotV2 as PersistedTaskSnapshotV2, StoredTaskSnapshot,
        COMPACT_TASK_PERSISTENCE_VERSION,
    },
    StoreDemSourceIdentity, StoreError, TaskStore,
};

pub const DEFAULT_WORKSPACE_ID: &str = "workspace-default";

/// #22 B slice 3(b): how many staged candidate rows the streaming seal reads
/// per batch. One batch of JSON text plus its decoded items is the only
/// in-memory candidate buffer the seal owns.
pub(crate) const SEAL_READ_BATCH: usize = 256;

/// #22 B slice 3(b): serializes a `FrozenSearchBatch` candidate-by-candidate
/// into `search_batches.snapshot_json` without holding the whole candidate
/// list. The header (with the final statistics and an empty candidate array)
/// is serialized by serde itself and its fixed `,"candidates":[]}` tail is
/// cut off, so the streamed output stays byte-identical to
/// `serde_json::to_string` of the same complete batch.
#[derive(Debug)]
struct StreamingBatchBlob {
    buffer: String,
}

impl StreamingBatchBlob {
    /// `search_stats` is the final statistic the seal will publish; the staged
    /// header always carries `None` and the classic seal assigns it just
    /// before serializing, so this mirrors that exact order.
    fn new(
        header: &FrozenSearchBatch,
        search_stats: Option<&FrozenSearchStats>,
    ) -> Result<Self, StoreError> {
        let head = FrozenSearchBatch {
            search_stats: search_stats.cloned(),
            candidates: Vec::new(),
            ..header.clone()
        };
        let serialized = serde_json::to_string(&head).map_err(StoreError::Corrupt)?;
        // serde emits the struct fields in declaration order, so the empty
        // candidate array is always this exact suffix. If a future struct
        // change breaks that assumption, fail closed instead of panicking:
        // the seal must never publish a malformed snapshot blob.
        let prefix = serialized
            .strip_suffix(",\"candidates\":[]}")
            .ok_or_else(|| {
                StoreError::invalid(
                    "serialized batch header does not end with the empty candidates array",
                )
            })?
            .to_owned();
        Ok(Self {
            buffer: format!("{prefix},\"candidates\":["),
        })
    }

    /// #22 B slice 3(b) review fix 6: test-only constructor from a raw
    /// serialized header, so the suffix-strip fail-closed path is testable
    /// without depending on serde field order of the real struct.
    #[cfg(test)]
    fn from_serialized_header(serialized: &str) -> Result<Self, StoreError> {
        let prefix = serialized
            .strip_suffix(",\"candidates\":[]}")
            .ok_or_else(|| {
                StoreError::invalid(
                    "serialized batch header does not end with the empty candidates array",
                )
            })?
            .to_owned();
        Ok(Self {
            buffer: format!("{prefix},\"candidates\":["),
        })
    }

    fn push_candidate(&mut self, candidate: &SnapshotItem) -> Result<(), StoreError> {
        if !self.buffer.ends_with('[') {
            self.buffer.push(',');
        }
        // One bounded per-item serialization; serde_json::to_string of a
        // single item is byte-identical to that item's slice of the whole
        // batch's serialization (compact formatter, same field order).
        let item = serde_json::to_string(candidate).map_err(StoreError::Corrupt)?;
        self.buffer.push_str(&item);
        Ok(())
    }

    fn finish(mut self) -> String {
        self.buffer.push_str("]}");
        self.buffer
    }
}

#[path = "nisar_archived_recovery.rs"]
pub(crate) mod nisar_archived_recovery;
pub use nisar_archived_recovery::{
    StoreCreateNisarArchivedRecoveryV2Request, StoredNisarArchivedRecoveryV2,
};

#[path = "nisar_budget_upgrade.rs"]
pub(crate) mod nisar_budget_upgrade;

#[derive(Debug, Clone, PartialEq)]
pub struct StoredWorkbench {
    pub batch: FrozenSearchBatch,
    pub state: SearchWorkbenchState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredTask {
    pub snapshot: FrozenTaskSnapshot,
    pub runtime: TaskRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoreSearchBatchStagingRequest {
    pub request_id: String,
    pub batch_header: FrozenSearchBatch,
    pub workbench: SearchWorkbenchState,
    pub max_candidates: u64,
}

impl StoreSearchBatchStagingRequest {
    fn validate(&self) -> Result<(), StoreError> {
        if self.request_id.is_empty()
            || self.request_id.len() > 128
            || self.request_id != self.request_id.trim()
            || self.max_candidates == 0
            || !self.batch_header.candidates.is_empty()
            || self.batch_header.search_stats.is_some()
            || self.batch_header.batch_id != self.workbench.batch_id
        {
            return Err(StoreError::invalid(
                "invalid hidden search batch staging request",
            ));
        }
        self.batch_header.validate().map_err(StoreError::invalid)?;
        self.workbench.validate().map_err(StoreError::invalid)?;
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StoreCreateHistoricalExplicitTaskRequest {
    pub source_task_id: String,
    pub expected_source_task_revision: u64,
    pub task: StoreCreateTaskRequest,
    pub source_item_ids: Vec<String>,
    pub task_items: Vec<SnapshotItem>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StoreAttachNisarSubsetExecutionPlanV2Request {
    pub task_id: String,
    pub item_id: String,
    pub prepared_intent_sha256: String,
    pub expected_revision: u64,
    pub occurred_at: String,
    pub plan: NisarSubsetPlanV2,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoreFreezeNisarSubsetSourceSizeV2Request {
    pub task_id: String,
    pub item_id: String,
    pub provider_id: String,
    pub collection_id: String,
    pub provider_product_id: String,
    pub provider_product_key: String,
    pub asset_key: String,
    pub stable_asset_key: String,
    pub prepared_intent_sha256: String,
    pub expected_revision: u64,
    pub occurred_at: String,
    pub expected_size_bytes: u64,
}

pub const NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND: &str = "science_checkpoint_v2";
pub const NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND: &str = "writer_recovery_v2";
const NISAR_SUBSET_EXECUTION_ANCHOR_MAX_CANONICAL_BYTES: usize = 40 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StoredNisarSubsetExecutionAnchorV2 {
    pub task_id: String,
    pub item_id: String,
    pub anchor_kind: String,
    pub lineage_key_sha256: String,
    pub record_sha256: String,
    pub canonical_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
    pub task_id: String,
    pub item_id: String,
    pub anchor_kind: String,
    pub lineage_key_sha256: String,
    pub expected_record_sha256: Option<String>,
    pub record_sha256: String,
    pub canonical_json: String,
}

fn burst_successor_replay(
    transaction: &rusqlite::Transaction<'_>,
    identity: &crate::StoreBurstSuccessorIdentity,
) -> Result<Option<StoredTask>, StoreError> {
    match crate::source_receipts::burst_successor_status(transaction, identity)? {
        crate::BurstSuccessorReceiptStatus::Committed => {
            if row_exists(
                transaction,
                "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
                &identity.new_task_id,
            )? {
                return Err(StoreError::NotFound);
            }
            Ok(Some(
                load_task_connection(transaction, &identity.new_task_id)?
                    .ok_or(StoreError::NotFound)?,
            ))
        }
        crate::BurstSuccessorReceiptStatus::Conflict => Err(StoreError::AlreadyExists),
        crate::BurstSuccessorReceiptStatus::Unknown => {
            if row_exists(transaction, "SELECT EXISTS(SELECT 1 FROM task_snapshots WHERE task_id = ?1 UNION ALL SELECT 1 FROM task_tombstones WHERE task_id = ?1)", &identity.new_task_id)? {
                return Err(StoreError::AlreadyExists);
            }
            Ok(None)
        }
    }
}

impl TaskStore {
    #[allow(
        clippy::too_many_arguments,
        reason = "save_region_aoi preserves the compatibility API while the workspace-aware method owns the production path"
    )]
    pub fn save_region_aoi(
        &mut self,
        region_id: &str,
        expected_revision: Option<u64>,
        source_kind: &str,
        geometry: &serde_json::Map<String, serde_json::Value>,
        bbox: &[f64],
        crs: &str,
        display_name: &str,
        bound_at: &str,
        updated_at: &str,
    ) -> Result<RegionAoiState, StoreError> {
        self.save_region_aoi_for_workspace(
            DEFAULT_WORKSPACE_ID,
            region_id,
            expected_revision,
            source_kind,
            geometry,
            bbox,
            crs,
            display_name,
            bound_at,
            updated_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_region_aoi_for_workspace(
        &mut self,
        workspace_id: &str,
        region_id: &str,
        expected_revision: Option<u64>,
        source_kind: &str,
        geometry: &serde_json::Map<String, serde_json::Value>,
        bbox: &[f64],
        crs: &str,
        display_name: &str,
        bound_at: &str,
        updated_at: &str,
    ) -> Result<RegionAoiState, StoreError> {
        if workspace_id.trim().is_empty() || region_id.trim().is_empty() {
            return Err(StoreError::invalid("workspace and region IDs are required"));
        }
        let existing = self.load_region_aoi_for_workspace(workspace_id, region_id)?;
        let revision = if let Some(current) = &existing {
            if let Some(expected) = expected_revision {
                if current.revision != expected {
                    return Err(StoreError::RevisionConflict);
                }
            }
            current
                .revision
                .checked_add(1)
                .ok_or_else(|| StoreError::invalid("revision overflow"))?
        } else {
            if let Some(expected) = expected_revision {
                if expected != 0 {
                    return Err(StoreError::RevisionConflict);
                }
            }
            0
        };

        let authoritative = insar_core::validated_authoritative_polygonal_geojson(geometry)
            .ok_or_else(|| StoreError::invalid("Region AOI geometry is invalid"))?;
        let exact_bbox = authoritative.bbox;
        if bbox.len() != 4
            || bbox.iter().any(|coordinate| !coordinate.is_finite())
            || bbox
                .iter()
                .zip(exact_bbox.iter())
                .any(|(provided, exact)| (provided - exact).abs() > 1.0e-9)
        {
            return Err(StoreError::invalid(
                "Region AOI bbox does not match its geometry",
            ));
        }
        let state = RegionAoiState {
            region_id: region_id.to_owned(),
            revision,
            source_kind: source_kind.to_owned(),
            geometry: authoritative.geojson,
            bbox: exact_bbox.to_vec(),
            crs: crs.to_owned(),
            display_name: display_name.to_owned(),
            bound_at: bound_at.to_owned(),
            updated_at: updated_at.to_owned(),
        };
        state.validate().map_err(StoreError::invalid)?;

        let geometry_json = serde_json::to_string(&state.geometry).map_err(StoreError::Corrupt)?;
        let bbox_json = serde_json::to_string(&state.bbox).map_err(StoreError::Corrupt)?;
        let revision_i64 =
            i64::try_from(revision).map_err(|_| StoreError::invalid("revision bounds"))?;

        self.connection.execute(
            "INSERT INTO region_aoi_state (workspace_id, region_id, revision, source_kind, geometry_json, bbox_json, crs, display_name, bound_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT(workspace_id, region_id) DO UPDATE SET \
             region_id = excluded.region_id, \
             revision = excluded.revision, \
             source_kind = excluded.source_kind, \
             geometry_json = excluded.geometry_json, \
             bbox_json = excluded.bbox_json, \
             crs = excluded.crs, \
             display_name = excluded.display_name, \
             bound_at = excluded.bound_at, \
             updated_at = excluded.updated_at",
            params![
                workspace_id,
                region_id,
                revision_i64,
                source_kind,
                geometry_json,
                bbox_json,
                crs,
                display_name,
                bound_at,
                updated_at,
            ],
        )?;

        Ok(state)
    }

    pub fn load_region_aoi(&self) -> Result<Option<RegionAoiState>, StoreError> {
        self.load_region_aoi_for_workspace(DEFAULT_WORKSPACE_ID, "")
    }

    pub fn load_region_aoi_for_workspace(
        &self,
        workspace_id: &str,
        region_id: &str,
    ) -> Result<Option<RegionAoiState>, StoreError> {
        let (query, parameters): (&str, Vec<&str>) = if region_id.is_empty() {
            (
                "SELECT region_id, revision, source_kind, geometry_json, bbox_json, crs, display_name, bound_at, updated_at \
                 FROM region_aoi_state WHERE workspace_id = ?1 ORDER BY region_id LIMIT 1",
                vec![workspace_id],
            )
        } else {
            (
                "SELECT region_id, revision, source_kind, geometry_json, bbox_json, crs, display_name, bound_at, updated_at \
                 FROM region_aoi_state WHERE workspace_id = ?1 AND region_id = ?2",
                vec![workspace_id, region_id],
            )
        };
        let row = self
            .connection
            .query_row(query, rusqlite::params_from_iter(parameters), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .optional()?;

        let Some((
            region_id,
            revision_i64,
            source_kind,
            geometry_json,
            bbox_json,
            crs,
            display_name,
            bound_at,
            updated_at,
        )) = row
        else {
            return Ok(None);
        };

        let revision =
            u64::try_from(revision_i64).map_err(|_| StoreError::invalid("invalid revision"))?;
        let geometry: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&geometry_json).map_err(StoreError::Corrupt)?;
        let bbox: Vec<f64> = serde_json::from_str(&bbox_json).map_err(StoreError::Corrupt)?;

        let state = RegionAoiState {
            region_id,
            revision,
            source_kind,
            geometry,
            bbox,
            crs,
            display_name,
            bound_at,
            updated_at,
        };
        state.validate().map_err(StoreError::invalid)?;
        Ok(Some(state))
    }

    pub fn load_region_aois(&self, workspace_id: &str) -> Result<Vec<RegionAoiState>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT region_id, revision, source_kind, geometry_json, bbox_json, crs, display_name, bound_at, updated_at \
             FROM region_aoi_state WHERE workspace_id = ?1 ORDER BY region_id",
        )?;
        let rows = statement.query_map([workspace_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?;
        rows.map(|row| {
            let (
                region_id,
                revision_i64,
                source_kind,
                geometry_json,
                bbox_json,
                crs,
                display_name,
                bound_at,
                updated_at,
            ) = row?;
            let revision =
                u64::try_from(revision_i64).map_err(|_| StoreError::invalid("invalid revision"))?;
            let geometry = serde_json::from_str(&geometry_json).map_err(StoreError::Corrupt)?;
            let bbox = serde_json::from_str(&bbox_json).map_err(StoreError::Corrupt)?;
            let state = RegionAoiState {
                region_id,
                revision,
                source_kind,
                geometry,
                bbox,
                crs,
                display_name,
                bound_at,
                updated_at,
            };
            state.validate().map_err(StoreError::invalid)?;
            Ok(state)
        })
        .collect()
    }

    pub fn clear_region_aoi(&mut self, expected_revision: Option<u64>) -> Result<(), StoreError> {
        let current = self.load_region_aoi()?;
        if let Some(expected) = expected_revision {
            if current
                .as_ref()
                .is_none_or(|state| state.revision != expected)
            {
                return Err(StoreError::RevisionConflict);
            }
        }
        self.connection.execute(
            "DELETE FROM region_aoi_state WHERE workspace_id = ?1",
            [DEFAULT_WORKSPACE_ID],
        )?;
        Ok(())
    }

    pub fn clear_region_aoi_for_workspace(
        &mut self,
        workspace_id: &str,
        region_id: &str,
        expected_revision: Option<u64>,
    ) -> Result<(), StoreError> {
        if let Some(expected) = expected_revision {
            let current = self.load_region_aoi_for_workspace(workspace_id, region_id)?;
            if let Some(state) = current {
                if state.revision != expected {
                    return Err(StoreError::RevisionConflict);
                }
            } else {
                return Err(StoreError::RevisionConflict);
            }
        }
        self.connection.execute(
            "DELETE FROM region_aoi_state WHERE workspace_id = ?1 AND region_id = ?2",
            rusqlite::params![workspace_id, region_id],
        )?;
        Ok(())
    }

    pub fn load_workspace_context(&self) -> Result<Option<WorkspaceContextState>, StoreError> {
        let json = self
            .connection
            .query_row(
                "SELECT context_json FROM workspace_context WHERE slot = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|value| {
            let context: WorkspaceContextState =
                serde_json::from_str(&value).map_err(StoreError::Corrupt)?;
            context.validate().map_err(StoreError::invalid)?;
            validate_workspace_context_hierarchy(&context)?;
            Ok(context)
        })
        .transpose()
    }

    pub fn save_workspace_context(
        &mut self,
        context: &WorkspaceContextState,
    ) -> Result<(), StoreError> {
        context.validate().map_err(StoreError::invalid)?;
        validate_workspace_context_hierarchy(context)?;
        let context_json = serde_json::to_string(context).map_err(StoreError::Corrupt)?;
        self.connection.execute(
            "INSERT INTO workspace_context(slot, context_json) VALUES (1, ?1) \
             ON CONFLICT(slot) DO UPDATE SET context_json = excluded.context_json",
            [context_json],
        )?;
        Ok(())
    }

    pub fn save_search_batch(
        &mut self,
        request: &StoreSearchBatchRequest,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save_search_batch_in_transaction(&transaction, request)?;
        crate::cache_maintenance::prune_disposable_search_batches_in_tx(
            &transaction,
            Some(&request.batch.batch_id),
            None,
            crate::cache_maintenance::SEARCH_CACHE_DEFAULT_TTL_SECONDS,
            crate::cache_maintenance::SEARCH_CACHE_MAX_ENTRIES,
            crate::cache_maintenance::SEARCH_CACHE_MAX_BYTES,
            false,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn begin_search_batch_staging(
        &mut self,
        request: &StoreSearchBatchStagingRequest,
    ) -> Result<(), StoreError> {
        request.validate()?;
        validate_workbench(&request.batch_header, &request.workbench)?;
        let max_candidates = sqlite_integer(request.max_candidates, "staging candidate limit")?;
        let batch_header_json =
            serde_json::to_string(&request.batch_header).map_err(StoreError::Corrupt)?;
        let workbench_json =
            serde_json::to_string(&request.workbench).map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if row_exists(
            &transaction,
            "SELECT EXISTS(SELECT 1 FROM search_batches WHERE batch_id = ?1)",
            &request.batch_header.batch_id,
        )? || row_exists(
            &transaction,
            "SELECT EXISTS(SELECT 1 FROM search_batch_staging WHERE request_id = ?1)",
            &request.request_id,
        )? || row_exists(
            &transaction,
            "SELECT EXISTS(SELECT 1 FROM search_batch_staging WHERE batch_id = ?1)",
            &request.batch_header.batch_id,
        )? {
            return Err(StoreError::AlreadyExists);
        }
        transaction.execute(
            "INSERT INTO search_batch_staging(
                 request_id, batch_id, max_candidates, batch_header_json, workbench_json
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                request.request_id,
                request.batch_header.batch_id,
                max_candidates,
                batch_header_json,
                workbench_json
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn append_search_batch_staging_candidates(
        &mut self,
        request_id: &str,
        candidates: &[SnapshotItem],
    ) -> Result<(), StoreError> {
        if request_id.is_empty() || request_id.len() > 128 || request_id != request_id.trim() {
            return Err(StoreError::invalid(
                "invalid search batch staging request ID",
            ));
        }
        if candidates.is_empty() {
            return Ok(());
        }
        for candidate in candidates {
            candidate.validate().map_err(StoreError::invalid)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let max_candidates = transaction
            .query_row(
                "SELECT max_candidates FROM search_batch_staging WHERE request_id = ?1",
                [request_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let current_count = transaction.query_row(
            "SELECT COUNT(*) FROM search_batch_staging_candidates WHERE request_id = ?1",
            [request_id],
            |row| row.get::<_, i64>(0),
        )?;
        let appended_count = i64::try_from(candidates.len())
            .map_err(|_| StoreError::invalid("staging candidate count exceeds SQLite range"))?;
        if current_count
            .checked_add(appended_count)
            .is_none_or(|count| count > max_candidates)
        {
            return Err(StoreError::invalid(
                "search batch staging candidate limit exceeded",
            ));
        }
        for candidate in candidates {
            let duplicate: bool = transaction.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM search_batch_staging_candidates
                     WHERE request_id = ?1 AND item_id = ?2
                 )",
                params![request_id, candidate.item_id],
                |row| row.get(0),
            )?;
            if duplicate {
                return Err(StoreError::AlreadyExists);
            }
            let item_json = serde_json::to_string(candidate).map_err(StoreError::Corrupt)?;
            transaction.execute(
                "INSERT INTO search_batch_staging_candidates(request_id, item_id, item_json)
                 VALUES (?1, ?2, ?3)",
                params![request_id, candidate.item_id, item_json],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn seal_search_batch_staging(
        &mut self,
        request_id: &str,
        search_stats: Option<FrozenSearchStats>,
        activate_workbench: bool,
    ) -> Result<(), StoreError> {
        self.seal_search_batch_staging_with_cache_pointer(
            request_id,
            search_stats,
            activate_workbench,
            None,
        )
    }

    /// #22 B slice 3(b): seal the staged candidates without ever holding the
    /// whole candidate list in memory. Reads the staged rows in bounded
    /// batches ordered by item ID (the classic seal's frozen order), streams
    /// them into the snapshot blob, copies them into the published candidate
    /// rows with `INSERT ... SELECT`, and writes header/index/workbench/cache
    /// pointer — all in one transaction that rolls back completely on
    /// failure. The published blob is byte-identical to the classic seal's.
    pub fn seal_search_batch_staging_streaming(
        &mut self,
        request_id: &str,
        search_stats: Option<FrozenSearchStats>,
        activate_workbench: bool,
    ) -> Result<(), StoreError> {
        self.seal_search_batch_staging_streaming_with_cache_pointer(
            request_id,
            search_stats,
            activate_workbench,
            None,
        )
    }

    /// Seal a progressively staged search publication unconditionally eligible
    /// for per-source view indexing (`view_eligible = 1`), optionally updating
    /// the single active Workbench slot when `activate_workbench_slot` is true.
    pub fn seal_search_batch_staging_publication(
        &mut self,
        request_id: &str,
        search_stats: Option<FrozenSearchStats>,
        activate_workbench_slot: bool,
        terminal: SearchBatchProgressState,
    ) -> Result<(), StoreError> {
        if terminal == SearchBatchProgressState::Loading {
            return Err(StoreError::invalid(
                "sealed search batch publication requires a terminal progress state",
            ));
        }
        if matches!(
            terminal,
            SearchBatchProgressState::Cancelled
                | SearchBatchProgressState::Failed
                | SearchBatchProgressState::Interrupted
        ) && !search_stats.as_ref().is_some_and(|stats| stats.truncated)
        {
            return Err(StoreError::invalid(
                "a cancelled or failed search batch publication must mark truncated=true",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        seal_staging_streaming_in_transaction_with_eligibility(
            &transaction,
            &self.seal_read_batch_cursor,
            request_id,
            search_stats,
            activate_workbench_slot,
            true,
            None,
            terminal,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn seal_search_batch_staging_streaming_with_cache_pointer(
        &mut self,
        request_id: &str,
        search_stats: Option<FrozenSearchStats>,
        activate_workbench: bool,
        cache_pointer: Option<(&str, i64)>,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        seal_staging_streaming_in_transaction(
            &transaction,
            &self.seal_read_batch_cursor,
            request_id,
            search_stats,
            activate_workbench,
            cache_pointer,
            SearchBatchProgressState::Complete,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn seal_search_batch_staging_with_cache_pointer(
        &mut self,
        request_id: &str,
        search_stats: Option<FrozenSearchStats>,
        activate_workbench: bool,
        cache_pointer: Option<(&str, i64)>,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let staged = transaction
            .query_row(
                "SELECT batch_header_json, workbench_json, max_candidates
                 FROM search_batch_staging WHERE request_id = ?1",
                [request_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let mut batch: FrozenSearchBatch =
            serde_json::from_str(&staged.0).map_err(StoreError::Corrupt)?;
        let workbench: SearchWorkbenchState =
            serde_json::from_str(&staged.1).map_err(StoreError::Corrupt)?;
        let candidates = {
            let mut statement = transaction.prepare(
                "SELECT item_json FROM search_batch_staging_candidates
                 WHERE request_id = ?1 ORDER BY item_id ASC",
            )?;
            let candidates = statement
                .query_map([request_id], |row| row.get::<_, String>(0))?
                .map(|row| {
                    let json = row?;
                    serde_json::from_str::<SnapshotItem>(&json).map_err(StoreError::Corrupt)
                })
                .collect::<Result<Vec<_>, StoreError>>()?;
            candidates
        };
        let candidate_count = i64::try_from(candidates.len())
            .map_err(|_| StoreError::invalid("staged candidate count exceeds SQLite range"))?;
        if candidate_count > staged.2
            || search_stats
                .as_ref()
                .is_some_and(|stats| stats.frozen_candidate_count != candidates.len() as u64)
        {
            return Err(StoreError::invalid(
                "sealed search batch staging counts are inconsistent",
            ));
        }
        batch.search_stats = search_stats;
        batch.candidates = candidates;
        let request = StoreSearchBatchRequest {
            batch,
            workbench,
            activate_workbench,
        };
        save_search_batch_in_transaction(&transaction, &request)?;
        if let Some((cache_key, updated_at_epoch)) = cache_pointer {
            let frozen_query: CatalogSearchQuery =
                serde_json::from_value(serde_json::Value::Object(request.batch.query.clone()))
                    .map_err(StoreError::Corrupt)?;
            let expected_key = catalog_search_cache_key(&frozen_query)
                .map_err(|error| StoreError::invalid(error.user_message))?;
            if cache_key != expected_key {
                return Err(StoreError::invalid(
                    "catalogue cache key does not match the frozen typed query",
                ));
            }
            if updated_at_epoch < 0
                || cache_key.len() != 64
                || !cache_key
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(StoreError::invalid(
                    "catalogue cache pointer is not canonical",
                ));
            }
            transaction.execute(
                "INSERT INTO sar_search_batch_cache(\
                     cache_key, source_batch_id, updated_at_epoch, complete\
                 ) VALUES (?1, ?2, ?3, 1) \
                 ON CONFLICT(cache_key) DO UPDATE SET \
                     source_batch_id = excluded.source_batch_id, \
                     updated_at_epoch = excluded.updated_at_epoch, \
                     complete = 1",
                params![cache_key, request.batch.batch_id, updated_at_epoch],
            )?;
        }
        crate::batch_progress::record_sealed_state(
            &transaction,
            request_id,
            SearchBatchProgressState::Complete,
            request
                .batch
                .search_stats
                .as_ref()
                .and_then(|stats| stats.matched_count),
        )?;
        transaction.execute(
            "DELETE FROM search_batch_staging WHERE request_id = ?1",
            [request_id],
        )?;
        crate::cache_maintenance::prune_disposable_search_batches_in_tx(
            &transaction,
            Some(&request.batch.batch_id),
            cache_pointer.and_then(|(_, epoch)| u64::try_from(epoch).ok()),
            crate::cache_maintenance::SEARCH_CACHE_DEFAULT_TTL_SECONDS,
            crate::cache_maintenance::SEARCH_CACHE_MAX_ENTRIES,
            crate::cache_maintenance::SEARCH_CACHE_MAX_BYTES,
            false,
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Test-only: the raw `snapshot_json` of a stored batch, for byte-exact
    /// blob comparisons. Production readers decode the batch instead.
    #[doc(hidden)]
    pub fn query_snapshot_json(&self, batch_id: &str) -> Result<Option<String>, StoreError> {
        self.connection
            .query_row(
                "SELECT snapshot_json FROM search_batches WHERE batch_id = ?1",
                [batch_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// Test-only: how many rows remain in the staging area.
    #[doc(hidden)]
    pub fn count_staging_rows(&self) -> Result<usize, StoreError> {
        let count: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM search_batch_staging_candidates",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::from)?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// Test-only: how many search batches have been published.
    #[doc(hidden)]
    pub fn count_search_batches(&self) -> Result<usize, StoreError> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM search_batches", [], |row| row.get(0))
            .map_err(StoreError::from)?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// Test-only: how many published candidate rows a batch has.
    #[doc(hidden)]
    pub fn count_batch_candidate_rows(&self, batch_id: &str) -> Result<usize, StoreError> {
        let count: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM search_batch_candidates WHERE batch_id = ?1",
                [batch_id],
                |row| row.get(0),
            )
            .map_err(StoreError::from)?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// Test-only: how many times the streaming seal decoded the whole
    /// snapshot blob; proves the seal never falls back to a full decode.
    #[doc(hidden)]
    pub fn take_seal_blob_decode_count(&mut self) -> u64 {
        self.seal_blob_decode_count
            .lock()
            .map(|mut count| std::mem::take(&mut *count))
            .unwrap_or(0)
    }

    /// Test-only: the largest single read batch the streaming seal has used;
    /// proves the seal's in-memory candidate buffer stays bounded.
    #[doc(hidden)]
    pub fn take_max_seal_read_batch(&mut self) -> usize {
        self.seal_read_batch_cursor
            .lock()
            .map(|mut cursor| std::mem::take(&mut *cursor))
            .unwrap_or(0)
    }

    /// Drop a request's staged rows. #22 B slice 3(c): a progressive
    /// publication that never reached a terminal state is withdrawn with
    /// them, so an aborted request leaves no batch, rows or loading state.
    pub fn abort_search_batch_staging(&mut self, request_id: &str) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::batch_progress::withdraw_loading(&transaction, request_id)?;
        transaction.execute(
            "DELETE FROM search_batch_staging WHERE request_id = ?1",
            [request_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_search_batch_and_create_task(
        &mut self,
        batch_request: &StoreSearchBatchRequest,
        task_request: &StoreCreateTaskRequest,
    ) -> Result<StoredTask, StoreError> {
        self.save_search_batch_and_create_task_with_projection(
            batch_request,
            task_request,
            None,
            None,
        )
    }

    pub fn save_search_batch_and_create_task_with_projection(
        &mut self,
        batch_request: &StoreSearchBatchRequest,
        task_request: &StoreCreateTaskRequest,
        operation: Option<&TaskOperationDescriptor>,
        orbit_association: Option<&FrozenOrbitAssociationV1>,
    ) -> Result<StoredTask, StoreError> {
        self.save_search_batch_and_create_task_with_projection_and_dem_identity(
            batch_request,
            task_request,
            operation,
            orbit_association,
            None,
        )
    }

    /// Atomically freeze the operation task and, for an admitted DEM download
    /// or preparation, confirm the exact selected catalogue identity.
    pub fn save_search_batch_and_create_task_with_projection_and_dem_identity(
        &mut self,
        batch_request: &StoreSearchBatchRequest,
        task_request: &StoreCreateTaskRequest,
        operation: Option<&TaskOperationDescriptor>,
        orbit_association: Option<&FrozenOrbitAssociationV1>,
        dem_source_identity: Option<&StoreDemSourceIdentity>,
    ) -> Result<StoredTask, StoreError> {
        if batch_request.batch.batch_id != task_request.batch_id
            || batch_request.workbench.batch_id != task_request.batch_id
        {
            return Err(StoreError::invalid(
                "derived batch, workbench, and task must share one batch ID",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save_search_batch_in_transaction(&transaction, batch_request)?;
        let stored =
            create_task_in_transaction(&transaction, task_request, operation, orbit_association)?;
        if let Some(identity) = dem_source_identity {
            crate::core_restore::save_dem_source_identity_in_transaction(&transaction, identity)?;
        }
        transaction.commit()?;
        Ok(stored)
    }

    /// Freeze a user-mediated GACOS request without ever publishing schedulable
    /// work. Sending a form and importing its result use separate guarded APIs.
    pub fn create_paused_gacos_request_task(
        &mut self,
        batch_request: &StoreSearchBatchRequest,
        task_request: &StoreCreateTaskRequest,
        operation: &TaskOperationDescriptor,
    ) -> Result<StoredTask, StoreError> {
        insar_core::validate_gacos_manual_request_operation(operation)
            .map_err(StoreError::invalid)?;
        if operation.kind != "gacos"
            || operation.phase != "request"
            || operation.approved_worker_operation != "gacos_manual_request"
            || batch_request.batch.provider_id != "gacos.webform"
            || batch_request.activate_workbench
            || batch_request.batch.batch_id != task_request.batch_id
            || batch_request.workbench.batch_id != task_request.batch_id
            || task_request.concurrency != 1
            || task_request.layout != insar_core::DownloadLayout::Direct
            || batch_request.batch.candidates.iter().any(|item| {
                item.provider_id != "gacos.webform"
                    || item.asset_access.is_some()
                    || item.execution_policy.is_some()
            })
        {
            return Err(StoreError::invalid(
                "GACOS manual task authority is invalid",
            ));
        }
        let candidate_ids = batch_request
            .batch
            .candidates
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<HashSet<_>>();
        let selected_ids = task_request
            .selected_item_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if candidate_ids != selected_ids || candidate_ids.is_empty() {
            return Err(StoreError::invalid(
                "GACOS task must freeze every prepared request exactly",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save_search_batch_in_transaction(&transaction, batch_request)?;
        let stored = create_task_in_transaction_with_candidates(
            &transaction,
            task_request,
            TaskCandidateMode::SourceBatch,
            Some(operation),
            None,
            true,
        )?;
        transaction.commit()?;
        Ok(stored)
    }

    pub fn update_workbench(
        &mut self,
        state: &SearchWorkbenchState,
        expected_revision: u64,
        updated_at: &str,
    ) -> Result<(), StoreError> {
        state.validate().map_err(StoreError::invalid)?;
        let next_revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("workbench revision overflow"))?;
        if state.revision != next_revision || updated_at.trim().is_empty() {
            return Err(StoreError::invalid(
                "invalid workbench revision or timestamp",
            ));
        }
        let revision = sqlite_integer(state.revision, "workbench revision")?;
        let expected_revision = sqlite_integer(expected_revision, "expected workbench revision")?;
        let batch = load_batch(&self.connection, &state.batch_id)?
            .ok_or_else(|| StoreError::invalid("workbench batch does not exist"))?;
        validate_workbench(&batch, state)?;
        let state_json = serde_json::to_string(state).map_err(StoreError::Corrupt)?;
        let changed = self.connection.execute(
            "UPDATE workbench_state SET revision = ?1, updated_at = ?2, state_json = ?3 \
             WHERE slot = 1 AND batch_id = ?4 AND revision = ?5",
            params![
                revision,
                updated_at,
                state_json,
                state.batch_id,
                expected_revision
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        Ok(())
    }

    pub fn update_workbench_request(
        &mut self,
        request: &StoreUpdateWorkbenchRequest,
    ) -> Result<(), StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        self.update_workbench(
            &request.state,
            request.expected_revision,
            &request.updated_at,
        )
    }

    pub fn create_task(
        &mut self,
        request: &StoreCreateTaskRequest,
    ) -> Result<StoredTask, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored = create_task_in_transaction(&transaction, request, None, None)?;
        transaction.commit()?;
        Ok(stored)
    }

    /// Atomically create a task whose executable items were explicitly frozen
    /// from products in the active immutable SearchBatch.
    ///
    /// The task keeps `request.batch_id` as its source batch identity. The
    /// supplied items are task-scoped execution candidates; `source_item_ids`
    /// are the source products used by history/source-page projections.
    pub fn create_task_with_explicit_items(
        &mut self,
        request: &StoreCreateTaskRequest,
        expected_workbench_revision: u64,
        source_item_ids: &[String],
        task_items: &[SnapshotItem],
    ) -> Result<StoredTask, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::core_restore::load_active_workbench_state(
            &transaction,
            &request.batch_id,
            expected_workbench_revision,
        )?;
        let stored = create_task_with_explicit_items_in_transaction(
            &transaction,
            request,
            source_item_ids,
            task_items,
            None,
            None,
        )?;
        transaction.commit()?;
        Ok(stored)
    }

    /// The unified-search variant of `create_task_with_explicit_items`: it
    /// freezes task-scoped explicit items from an exact frozen-batch
    /// selection without reading or requiring the mutable Workbench slot,
    /// matching `create_task`'s admission contract.
    pub fn create_task_with_explicit_items_from_frozen_batch(
        &mut self,
        request: &StoreCreateTaskRequest,
        source_item_ids: &[String],
        task_items: &[SnapshotItem],
    ) -> Result<StoredTask, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored = create_task_with_explicit_items_in_transaction(
            &transaction,
            request,
            source_item_ids,
            task_items,
            None,
            None,
        )?;
        transaction.commit()?;
        Ok(stored)
    }

    /// Atomically freezes explicit task membership in a controlled paused state.
    /// This is used when an immutable prepared intent is durable but dependency
    /// evidence and the final execution plan are not yet frozen.
    pub fn create_paused_task_with_explicit_items(
        &mut self,
        request: &StoreCreateTaskRequest,
        expected_workbench_revision: u64,
        source_item_ids: &[String],
        task_items: &[SnapshotItem],
    ) -> Result<StoredTask, StoreError> {
        let freezes_nisar_v2_intent = task_items.iter().any(|item| {
            item.metadata.contains_key(
                crate::nisar_subset_plan::NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_METADATA_KEY,
            )
        });
        if freezes_nisar_v2_intent {
            let source_ids = source_item_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            if source_item_ids.len() != task_items.len()
                || source_ids.len() != source_item_ids.len()
            {
                return Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch);
            }
            let mut represented_source_ids = HashSet::with_capacity(task_items.len());
            for item in task_items {
                crate::nisar_subset_plan::validate_nisar_subset_prepared_intent_only_metadata(
                    &item.metadata,
                )?;
                let envelope =
                    crate::nisar_subset_plan::parse_nisar_subset_prepared_intent_v2_envelope(
                        &item.metadata,
                    )?
                    .ok_or(StoreError::NisarSubsetPreparedIntentMissing)?;
                let source_item_id = item
                    .metadata
                    .get("source_item_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(StoreError::NisarSubsetPreparedIntentIdentityMismatch)?;
                if source_item_id
                    != envelope
                        .source_product_identity
                        .provider_product_key
                        .as_str()
                    || item.provider_id != envelope.source_product_identity.provider_id
                    || !represented_source_ids.insert(source_item_id)
                {
                    return Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch);
                }
            }
            if represented_source_ids != source_ids {
                return Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch);
            }
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if freezes_nisar_v2_intent
            && row_exists(
                &transaction,
                "SELECT EXISTS( \
                    SELECT 1 FROM task_snapshots WHERE task_id = ?1 \
                    UNION ALL \
                    SELECT 1 FROM task_tombstones WHERE task_id = ?1 \
                )",
                &request.task_id,
            )?
        {
            return Err(StoreError::AlreadyExists);
        }
        let active_batch: Option<String> = transaction
            .query_row(
                "SELECT batch_id FROM workbench_state WHERE slot = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if active_batch.as_deref() == Some(request.batch_id.as_str()) {
            crate::core_restore::load_active_workbench_state(
                &transaction,
                &request.batch_id,
                expected_workbench_revision,
            )?;
        } else if row_exists(
            &transaction,
            "SELECT EXISTS(SELECT 1 FROM search_batches WHERE batch_id = ?1)",
            &request.batch_id,
        )? {
            if expected_workbench_revision != 0 {
                return Err(StoreError::RevisionConflict);
            }
        } else {
            return Err(crate::batch_progress::missing_batch_error(
                &transaction,
                &request.batch_id,
            ));
        }
        let stored = create_task_in_transaction_with_candidates(
            &transaction,
            request,
            TaskCandidateMode::Explicit {
                source_item_ids,
                task_items,
            },
            None,
            None,
            true,
        )?;
        transaction.commit()?;
        Ok(stored)
    }

    pub fn freeze_nisar_subset_source_size_v2(
        &mut self,
        request: &StoreFreezeNisarSubsetSourceSizeV2Request,
    ) -> Result<StoredTask, StoreError> {
        let identity_fields = [
            request.task_id.as_str(),
            request.item_id.as_str(),
            request.provider_id.as_str(),
            request.collection_id.as_str(),
            request.provider_product_id.as_str(),
            request.provider_product_key.as_str(),
            request.asset_key.as_str(),
            request.stable_asset_key.as_str(),
        ];
        if identity_fields.iter().any(|value| {
            value.is_empty() || value.trim() != *value || value.chars().any(char::is_control)
        }) || request.task_id.len() > 128
            || request.item_id.len() > 512
            || request.prepared_intent_sha256.len() != 64
            || !request
                .prepared_intent_sha256
                .bytes()
                .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
            || request.occurred_at.is_empty()
            || request.occurred_at.len() > 30
            || request.occurred_at.trim() != request.occurred_at
            || request.occurred_at.chars().any(char::is_control)
            || request.expected_size_bytes == 0
            || i64::try_from(request.expected_size_bytes).is_err()
        {
            return Err(StoreError::NisarSubsetSourceSizeInvalid);
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut task =
            load_task_connection(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        if task.runtime.revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        if task.runtime.status != TaskLifecycleStatus::Running
            || task.runtime.task_paused
            || !task
                .runtime
                .selected_item_ids
                .iter()
                .any(|item_id| item_id == &request.item_id)
            || !task
                .runtime
                .active_item_ids
                .iter()
                .any(|item_id| item_id == &request.item_id)
        {
            return Err(StoreError::NisarSubsetSourceSizeStateInvalid);
        }

        let snapshot_json: String = transaction.query_row(
            "SELECT snapshot_json FROM task_snapshots WHERE task_id = ?1",
            [&request.task_id],
            |row| row.get(0),
        )?;
        let mut compact = match decode_v2_only_snapshot(&snapshot_json)? {
            StoredTaskSnapshot::CompactV2(compact) => compact,
            StoredTaskSnapshot::LegacyV2(_) | StoredTaskSnapshot::V3(_) => {
                return Err(StoreError::NisarSubsetSourceSizeIdentityMismatch)
            }
        };
        if compact.provider_id != request.provider_id
            || !compact
                .source_item_ids
                .iter()
                .any(|source_item_id| source_item_id == &request.item_id)
        {
            return Err(StoreError::NisarSubsetSourceSizeIdentityMismatch);
        }
        let item = compact
            .selected_products
            .iter_mut()
            .find(|item| item.item_id == request.item_id)
            .ok_or(StoreError::NisarSubsetSourceSizeIdentityMismatch)?;
        let intent = crate::nisar_subset_plan::parse_nisar_subset_prepared_intent_v2_envelope(
            &item.metadata,
        )?
        .ok_or(StoreError::NisarSubsetPreparedIntentMissing)?;
        let source_item_id = item
            .metadata
            .get("source_item_id")
            .and_then(serde_json::Value::as_str);
        let delivery_kind = item
            .metadata
            .get("delivery_kind")
            .and_then(serde_json::Value::as_str);
        let access = item
            .asset_access
            .as_mut()
            .ok_or(StoreError::NisarSubsetSourceSizeIdentityMismatch)?;
        let resolver_authoritative = item.execution_policy.as_ref().is_some_and(|policy| {
            policy.authority == AssetExecutionAuthority::ResolverAuthoritative
        });
        if item.provider_id != request.provider_id
            || source_item_id != Some(request.provider_product_key.as_str())
            || delivery_kind != Some("remote_derived_subset_v2")
            || !resolver_authoritative
            || intent.canonical_intent_sha256 != request.prepared_intent_sha256
            || intent.source_product_identity.provider_id != request.provider_id
            || intent.source_product_identity.collection_id != request.collection_id
            || intent.source_product_identity.provider_product_id != request.provider_product_id
            || intent.source_product_identity.provider_product_key != request.provider_product_key
            || intent.source_asset_identity.asset_key != request.asset_key
            || intent.source_asset_identity.stable_asset_key != request.stable_asset_key
            || access.reference.provider_id != request.provider_id
            || access.reference.collection_id != request.collection_id
            || access.reference.provider_product_id != request.provider_product_id
            || access.reference.provider_product_key != request.provider_product_key
            || access.reference.asset_key != request.asset_key
            || access.reference.stable_asset_key != request.stable_asset_key
            || access.file_name.as_deref() != Some(intent.source_asset_identity.file_name.as_str())
        {
            return Err(StoreError::NisarSubsetSourceSizeIdentityMismatch);
        }
        match access.expected_size_bytes {
            Some(existing) if existing == request.expected_size_bytes => {
                transaction.commit()?;
                return Ok(task);
            }
            Some(_) => return Err(StoreError::NisarSubsetSourceSizeAlreadyFrozenConflict),
            None => access.expected_size_bytes = Some(request.expected_size_bytes),
        }

        let persisted_json = encode_task_snapshot(&StoredTaskSnapshot::CompactV2(compact))?;
        let changed = transaction.execute(
            "UPDATE task_snapshots SET snapshot_json = ?1 WHERE task_id = ?2 AND snapshot_json = ?3",
            params![persisted_json, request.task_id, snapshot_json],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }

        let revision = request
            .expected_revision
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("task revision overflow"))?;
        task.runtime.revision = revision;
        task.runtime.updated_at = request.occurred_at.clone();
        task.runtime.validate().map_err(StoreError::invalid)?;
        let runtime_json = serde_json::to_string(&task.runtime).map_err(StoreError::Corrupt)?;
        let changed = transaction.execute(
            "UPDATE task_runtime SET revision = ?1, updated_at = ?2, state_json = ?3 \
             WHERE task_id = ?4 AND revision = ?5",
            params![
                sqlite_integer(revision, "task revision")?,
                request.occurred_at,
                runtime_json,
                request.task_id,
                sqlite_integer(request.expected_revision, "expected task revision")?,
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        let payload = serde_json::to_string(&serde_json::json!({
            "provider_id": request.provider_id,
            "provider_product_key": request.provider_product_key,
            "stable_asset_key": request.stable_asset_key,
            "prepared_intent_sha256": request.prepared_intent_sha256,
            "expected_size_bytes": request.expected_size_bytes,
        }))
        .map_err(StoreError::Corrupt)?;
        transaction.execute(
            "INSERT INTO task_events( \
                event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json \
             ) VALUES (?1, ?2, ?3, ?4, 'nisar_subset_source_size_frozen_v2', ?5, ?6, NULL)",
            params![
                format!("{}:{}:nisar-subset-source-size-v2", request.task_id, request.item_id),
                request.task_id,
                sqlite_integer(revision, "task revision")?,
                request.occurred_at,
                request.item_id,
                payload,
            ],
        )?;
        transaction.commit()?;
        self.load_task_record(&request.task_id)?
            .ok_or(StoreError::NotFound)
    }

    pub fn attach_nisar_subset_execution_plan_v2(
        &mut self,
        request: &StoreAttachNisarSubsetExecutionPlanV2Request,
    ) -> Result<StoredTask, StoreError> {
        if request.task_id.is_empty()
            || request.task_id.len() > 128
            || request.task_id.trim() != request.task_id
            || request.item_id.is_empty()
            || request.item_id.len() > 512
            || request.item_id.trim() != request.item_id
            || request.prepared_intent_sha256.len() != 64
            || !request
                .prepared_intent_sha256
                .bytes()
                .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
            || request.occurred_at.is_empty()
            || request.occurred_at.len() > 30
            || request.occurred_at.trim() != request.occurred_at
            || request.occurred_at.chars().any(char::is_control)
        {
            return Err(StoreError::NisarSubsetExecutionPlanInvalid);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut task =
            load_task_connection(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        if task.runtime.revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        if !task
            .snapshot
            .candidates
            .iter()
            .any(|item| item.item_id == request.item_id)
            || task.runtime.status != TaskLifecycleStatus::Running
            || task.runtime.task_paused
            || !task
                .runtime
                .selected_item_ids
                .iter()
                .any(|item_id| item_id == &request.item_id)
            || !task
                .runtime
                .active_item_ids
                .iter()
                .any(|item_id| item_id == &request.item_id)
        {
            return Err(StoreError::NisarSubsetExecutionPlanAttachStateInvalid);
        }

        let snapshot_json: String = transaction.query_row(
            "SELECT snapshot_json FROM task_snapshots WHERE task_id = ?1",
            [&request.task_id],
            |row| row.get(0),
        )?;
        let mut compact = match decode_v2_only_snapshot(&snapshot_json)? {
            StoredTaskSnapshot::CompactV2(compact) => compact,
            StoredTaskSnapshot::LegacyV2(_) | StoredTaskSnapshot::V3(_) => {
                return Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch)
            }
        };
        if !compact
            .source_item_ids
            .iter()
            .any(|source_item_id| source_item_id == &request.item_id)
        {
            return Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch);
        }
        let item = compact
            .selected_products
            .iter_mut()
            .find(|item| item.item_id == request.item_id)
            .ok_or(StoreError::NisarSubsetPreparedIntentIdentityMismatch)?;
        let intent = crate::nisar_subset_plan::parse_nisar_subset_prepared_intent_v2_envelope(
            &item.metadata,
        )?
        .ok_or(StoreError::NisarSubsetPreparedIntentMissing)?;
        let source_item_id = item
            .metadata
            .get("source_item_id")
            .and_then(serde_json::Value::as_str);
        if item.item_id != request.item_id
            || source_item_id != Some(intent.source_product_identity.provider_product_key.as_str())
            || item.provider_id != intent.source_product_identity.provider_id
            || intent.canonical_intent_sha256 != request.prepared_intent_sha256
        {
            return Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch);
        }

        let proposed =
            crate::nisar_subset_plan::NisarSubsetExecutionPlanEnvelopeV2::from_plan(&request.plan)?;
        if proposed.prepared_intent_sha256 != request.prepared_intent_sha256 {
            return Err(StoreError::NisarSubsetExecutionPlanIntentMismatch);
        }
        crate::nisar_subset_plan::validate_plan_matches_prepared_intent(&proposed, &intent)?;
        if let Some(existing) =
            crate::nisar_subset_plan::parse_nisar_subset_plan_v2_envelope(&item.metadata)?
        {
            if existing.canonical_semantic_plan_digest == proposed.canonical_semantic_plan_digest {
                transaction.commit()?;
                return Ok(task);
            }
            return Err(StoreError::NisarSubsetExecutionPlanAlreadyFrozenConflict);
        }

        crate::nisar_subset_plan::embed_nisar_subset_plan_v2_envelope(
            &mut item.metadata,
            &request.plan,
        )?;
        crate::nisar_subset_plan::validate_nisar_subset_plan_v2_metadata(&item.metadata)?;
        let persisted_json = encode_task_snapshot(&StoredTaskSnapshot::CompactV2(compact))?;
        transaction.execute(
            "UPDATE task_snapshots SET snapshot_json = ?1 WHERE task_id = ?2",
            params![persisted_json, request.task_id],
        )?;

        let revision = request
            .expected_revision
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("task revision overflow"))?;
        task.runtime.revision = revision;
        task.runtime.updated_at = request.occurred_at.clone();
        task.runtime.validate().map_err(StoreError::invalid)?;
        let runtime_json = serde_json::to_string(&task.runtime).map_err(StoreError::Corrupt)?;
        let changed = transaction.execute(
            "UPDATE task_runtime SET revision = ?1, updated_at = ?2, state_json = ?3 \
             WHERE task_id = ?4 AND revision = ?5",
            params![
                sqlite_integer(revision, "task revision")?,
                request.occurred_at,
                runtime_json,
                request.task_id,
                sqlite_integer(request.expected_revision, "expected task revision")?,
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        let payload = serde_json::to_string(&serde_json::json!({
            "prepared_intent_sha256": proposed.prepared_intent_sha256,
            "canonical_semantic_plan_digest": proposed.canonical_semantic_plan_digest,
        }))
        .map_err(StoreError::Corrupt)?;
        transaction.execute(
            "INSERT INTO task_events( \
                event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json \
             ) VALUES (?1, ?2, ?3, ?4, 'nisar_subset_execution_plan_attached_v2', ?5, ?6, NULL)",
            params![
                format!(
                    "{}:{}:nisar-subset-plan-v2",
                    request.task_id, request.item_id
                ),
                request.task_id,
                sqlite_integer(revision, "task revision")?,
                request.occurred_at,
                request.item_id,
                payload,
            ],
        )?;
        transaction.commit()?;
        self.load_task_record(&request.task_id)?
            .ok_or(StoreError::NotFound)
    }

    pub fn load_nisar_subset_execution_anchor_v2(
        &self,
        task_id: &str,
        item_id: &str,
        anchor_kind: &str,
    ) -> Result<Option<StoredNisarSubsetExecutionAnchorV2>, StoreError> {
        validate_nisar_subset_execution_anchor_identity(task_id, item_id, anchor_kind)?;
        let row = self
            .connection
            .query_row(
                "SELECT lineage_key_sha256, record_sha256, canonical_json \
                 FROM nisar_subset_execution_anchors_v2 \
                 WHERE task_id = ?1 AND item_id = ?2 AND anchor_kind = ?3",
                params![task_id, item_id, anchor_kind],
                |row| {
                    Ok(StoredNisarSubsetExecutionAnchorV2 {
                        task_id: task_id.to_owned(),
                        item_id: item_id.to_owned(),
                        anchor_kind: anchor_kind.to_owned(),
                        lineage_key_sha256: row.get(0)?,
                        record_sha256: row.get(1)?,
                        canonical_json: row.get(2)?,
                    })
                },
            )
            .optional()?;
        row.map(|anchor| {
            validate_nisar_subset_execution_anchor_material(
                &anchor.lineage_key_sha256,
                Some(&anchor.record_sha256),
                &anchor.record_sha256,
                &anchor.canonical_json,
            )?;
            Ok(anchor)
        })
        .transpose()
    }

    pub fn compare_and_swap_nisar_subset_execution_anchor_v2(
        &mut self,
        request: &StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request,
    ) -> Result<bool, StoreError> {
        validate_nisar_subset_execution_anchor_identity(
            &request.task_id,
            &request.item_id,
            &request.anchor_kind,
        )?;
        validate_nisar_subset_execution_anchor_material(
            &request.lineage_key_sha256,
            request.expected_record_sha256.as_deref(),
            &request.record_sha256,
            &request.canonical_json,
        )?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task =
            load_task_connection(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        let item = task
            .snapshot
            .candidates
            .iter()
            .find(|candidate| candidate.item_id == request.item_id)
            .ok_or(StoreError::NotFound)?;
        if crate::nisar_subset_plan::parse_nisar_subset_plan_v2_envelope(&item.metadata)?.is_none()
        {
            return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
        }

        let existing = transaction
            .query_row(
                "SELECT lineage_key_sha256, record_sha256, canonical_json \
                 FROM nisar_subset_execution_anchors_v2 \
                 WHERE task_id = ?1 AND item_id = ?2 AND anchor_kind = ?3",
                params![request.task_id, request.item_id, request.anchor_kind],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        match existing {
            None => {
                if request.expected_record_sha256.is_some() {
                    transaction.commit()?;
                    return Ok(false);
                }
                transaction.execute(
                    "INSERT INTO nisar_subset_execution_anchors_v2( \
                         task_id, item_id, anchor_kind, lineage_key_sha256, record_sha256, canonical_json \
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        request.task_id,
                        request.item_id,
                        request.anchor_kind,
                        request.lineage_key_sha256,
                        request.record_sha256,
                        request.canonical_json,
                    ],
                )?;
            }
            Some((lineage_key_sha256, record_sha256, canonical_json)) => {
                validate_nisar_subset_execution_anchor_material(
                    &lineage_key_sha256,
                    Some(&record_sha256),
                    &record_sha256,
                    &canonical_json,
                )?;
                if lineage_key_sha256 != request.lineage_key_sha256 {
                    return Err(StoreError::NisarSubsetExecutionAnchorLineageConflict);
                }
                if request.expected_record_sha256.as_deref() != Some(record_sha256.as_str()) {
                    transaction.commit()?;
                    return Ok(false);
                }
                let changed = transaction.execute(
                    "UPDATE nisar_subset_execution_anchors_v2 \
                     SET record_sha256 = ?1, canonical_json = ?2 \
                     WHERE task_id = ?3 AND item_id = ?4 AND anchor_kind = ?5 \
                       AND lineage_key_sha256 = ?6 AND record_sha256 = ?7",
                    params![
                        request.record_sha256,
                        request.canonical_json,
                        request.task_id,
                        request.item_id,
                        request.anchor_kind,
                        request.lineage_key_sha256,
                        record_sha256,
                    ],
                )?;
                if changed != 1 {
                    transaction.commit()?;
                    return Ok(false);
                }
            }
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn load_workbench(&self) -> Result<Option<StoredWorkbench>, StoreError> {
        let row = self
            .connection
            .query_row(
                "SELECT b.snapshot_json, w.state_json, c.matched_count \
                 FROM workbench_state w JOIN search_batches b ON b.batch_id = w.batch_id \
                 LEFT JOIN sar_search_batch_count c ON c.batch_id = b.batch_id \
                 WHERE w.slot = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(batch, state, matched_count)| {
            let mut workbench = decode_workbench(&batch, &state)?;
            if let Some(matched_count) = matched_count {
                let matched_count = u64::try_from(matched_count)
                    .map_err(|_| StoreError::invalid("SAR matched count is invalid"))?;
                let stats =
                    workbench.batch.search_stats.as_mut().ok_or_else(|| {
                        StoreError::invalid("SAR count update lacks frozen stats")
                    })?;
                stats.matched_count = Some(matched_count);
                stats.truncated = matched_count > stats.frozen_candidate_count
                    || stats.provider_returned_count > stats.frozen_candidate_count;
                workbench.batch.validate().map_err(StoreError::invalid)?;
            }
            Ok(workbench)
        })
        .transpose()
    }

    pub fn load_search_batch(
        &self,
        batch_id: &str,
    ) -> Result<Option<FrozenSearchBatch>, StoreError> {
        load_batch(&self.connection, batch_id)
    }

    /// Load a task record irrespective of terminal history materialization.
    /// Tombstones remain unreadable through this business-facing authority.
    pub fn load_task_record(&self, task_id: &str) -> Result<Option<StoredTask>, StoreError> {
        if row_exists(
            &self.connection,
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            task_id,
        )? {
            return Ok(None);
        }
        load_task_connection(&self.connection, task_id)
    }

    /// Read a task and its referenced batch from the same SQLite snapshot.
    /// This decodes the full batch and cannot be nested in another transaction
    /// on this connection. A later write must recheck its authority separately.
    pub fn load_task_with_source_batch_snapshot(
        &self,
        task_id: &str,
    ) -> Result<Option<(StoredTask, FrozenSearchBatch)>, StoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        if row_exists(
            &transaction,
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            task_id,
        )? {
            transaction.commit()?;
            return Ok(None);
        }
        let task = load_task_connection(&transaction, task_id)?;
        let result = match task {
            Some(task) => {
                load_batch(&transaction, &task.snapshot.batch_id)?.map(|batch| (task, batch))
            }
            None => None,
        };
        transaction.commit()?;
        Ok(result)
    }

    /// Create a fresh explicit-item task from a terminal task's immutable source
    /// batch. This never reads, validates, or activates the main Workbench.
    pub fn create_historical_explicit_task(
        &mut self,
        request: &StoreCreateHistoricalExplicitTaskRequest,
    ) -> Result<StoredTask, StoreError> {
        self.create_historical_explicit_task_with_paused(request, false, None, None)
            .map(|(task, _)| task)
    }

    pub fn create_historical_burst_task_with_receipt(
        &mut self,
        request: &StoreCreateHistoricalExplicitTaskRequest,
        source_parents: Option<&[SnapshotItem]>,
        identity: &crate::StoreBurstSuccessorIdentity,
    ) -> Result<StoredTask, StoreError> {
        let selected: HashSet<_> = identity.selected_product_ids.iter().collect();
        let sources: HashSet<_> = request.source_item_ids.iter().collect();
        if identity.request_id.trim().is_empty()
            || identity.source_task_id.trim().is_empty()
            || identity.new_task_id.trim().is_empty()
            || identity.source_task_id == identity.new_task_id
            || identity.output_root.as_os_str().is_empty()
            || identity.layout != insar_core::DownloadLayout::Direct
            || !(1..=8).contains(&identity.concurrency)
            || identity.created_at.trim().is_empty()
            || selected.is_empty()
            || identity
                .selected_product_ids
                .iter()
                .any(|id| id.trim().is_empty())
            || selected.len() != identity.selected_product_ids.len()
            || sources.len() != request.source_item_ids.len()
            || selected != sources
            || identity.source_task_id != request.source_task_id
            || identity.expected_source_task_revision != request.expected_source_task_revision
            || identity.new_task_id != request.task.task_id
            || identity.output_root != request.task.output_root
            || identity.layout != request.task.layout
            || identity.concurrency != request.task.concurrency
            || identity.created_at != request.task.created_at
        {
            return Err(StoreError::invalid(
                "Burst successor identity differs from request",
            ));
        }
        if let Some(parents) = source_parents {
            self.create_historical_progressive_burst_task_inner(request, parents, Some(identity))
        } else {
            self.create_historical_explicit_task_with_paused(request, false, None, Some(identity))
                .map(|(task, _)| task)
        }
    }

    /// Create a bounded progressive Burst successor from a sealed original.
    /// The service freezes executable asset items; this transaction verifies
    /// that the complete parent evidence still matches the source authority.
    pub fn create_historical_progressive_burst_task(
        &mut self,
        request: &StoreCreateHistoricalExplicitTaskRequest,
        source_parents: &[SnapshotItem],
    ) -> Result<StoredTask, StoreError> {
        self.create_historical_progressive_burst_task_inner(request, source_parents, None)
    }

    fn create_historical_progressive_burst_task_inner(
        &mut self,
        request: &StoreCreateHistoricalExplicitTaskRequest,
        source_parents: &[SnapshotItem],
        receipt: Option<&crate::StoreBurstSuccessorIdentity>,
    ) -> Result<StoredTask, StoreError> {
        if request.source_task_id.trim().is_empty()
            || request.source_task_id == request.task.task_id
            || source_parents.is_empty()
            || source_parents.len() > crate::MAX_WORKBENCH_PAGE_SIZE as usize
            || request.source_item_ids.len() != source_parents.len()
            || request.source_item_ids.iter().collect::<HashSet<_>>().len() != source_parents.len()
            || request.task_items.is_empty()
            || request.task_items.len() > source_parents.len() * 2
            || request.task.layout != insar_core::DownloadLayout::Direct
            || !(1..=8).contains(&request.task.concurrency)
        {
            return Err(StoreError::invalid(
                "progressive Burst successor selection is invalid",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(identity) = receipt {
            if let Some(stored) = burst_successor_replay(&transaction, identity)? {
                transaction.commit()?;
                return Ok(stored);
            }
        }
        if row_exists(
            &transaction,
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            &request.source_task_id,
        )? || !crate::core_restore::task_is_actionable(&transaction, &request.source_task_id)?
        {
            return Err(StoreError::NotFound);
        }
        if load_explicit_task_source_item_ids(&transaction, &request.source_task_id)?.is_none() {
            return Err(StoreError::invalid(
                "progressive Burst source must be explicit",
            ));
        }
        let source = load_task_connection(&transaction, &request.source_task_id)?
            .ok_or(StoreError::NotFound)?;
        if source.runtime.revision != request.expected_source_task_revision {
            return Err(StoreError::RevisionConflict);
        }
        if !matches!(
            source.runtime.status,
            TaskLifecycleStatus::Completed
                | TaskLifecycleStatus::Cancelled
                | TaskLifecycleStatus::Failed
        ) || request.task.batch_id != source.snapshot.batch_id
        {
            return Err(StoreError::invalid(
                "progressive Burst historical source is invalid",
            ));
        }
        let source_id =
            crate::loading_admission::verified_progressive_source(&transaction, &source.snapshot)?
                .ok_or_else(|| StoreError::invalid("progressive Burst source is required"))?;
        let mut parents = crate::batch_candidates::load_batch_candidate_items_connection(
            &transaction,
            &source_id,
            &request.source_item_ids,
        )?
        .ok_or(StoreError::NotFound)?;
        let evidence = source_parents
            .iter()
            .map(|item| (item.item_id.as_str(), item))
            .collect::<HashMap<_, _>>();
        if evidence.len() != source_parents.len()
            || parents.len() != source_parents.len()
            || parents.iter().any(|item| {
                item.metadata.contains_key("source_item_id")
                    || evidence.get(item.item_id.as_str()).copied() != Some(item)
            })
        {
            return Err(StoreError::invalid(
                "progressive Burst successor source differs",
            ));
        }
        let requested =
            serde_json::to_string(&request.source_item_ids).map_err(StoreError::Corrupt)?;
        let conflict: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM batch_task_items i JOIN task_runtime r ON r.task_id = i.task_id \
             LEFT JOIN task_tombstones d ON d.task_id = i.task_id WHERE i.source_batch_id = ?1 \
             AND i.source_item_id IN (SELECT value FROM json_each(?2)) AND i.task_id != ?3 AND d.task_id IS NULL \
             AND r.status IN ('queued','running','paused','interrupted'))",
            params![source_id, requested, request.task.task_id], |row| row.get(0),
        )?;
        if conflict {
            return Err(StoreError::PartialActiveTaskOverlap);
        }
        insar_core::validate_burst_task_items(
            &parents,
            &source.snapshot.candidates,
            &request.task_items,
        )
        .map_err(|_| {
            StoreError::invalid("progressive Burst assets differ from source or policy")
        })?;
        for item in &mut parents {
            item.metadata.insert(
                "source_item_id".to_owned(),
                serde_json::Value::String(item.item_id.clone()),
            );
        }
        // The derived identity binds the history source, its revision and the
        // full request/evidence, so an existing task ID cannot replay a
        // different historical selection as the same creation.
        let identity =
            serde_json::to_vec(&(request, source_parents)).map_err(StoreError::Corrupt)?;
        let digest = Sha256::digest(identity);
        let batch_id = format!("progressive-successor-{digest:x}");
        let mut batch =
            load_batch(&transaction, &source.snapshot.batch_id)?.ok_or(StoreError::NotFound)?;
        batch.batch_id = batch_id.clone();
        batch.created_at = request.task.created_at.clone();
        batch.candidates = parents;
        save_search_batch_in_transaction(
            &transaction,
            &StoreSearchBatchRequest {
                batch,
                workbench: SearchWorkbenchState {
                    batch_id: batch_id.clone(),
                    checked_item_ids: Vec::new(),
                    page_index: 0,
                    page_size: source.runtime.page_size,
                    revision: 0,
                },
                activate_workbench: false,
            },
        )?;
        let mut task_request = request.task.clone();
        task_request.batch_id = batch_id;
        let stored = create_task_in_transaction_with_candidates(
            &transaction,
            &task_request,
            TaskCandidateMode::Explicit {
                source_item_ids: &request.source_item_ids,
                task_items: &request.task_items,
            },
            None,
            None,
            false,
        )?;
        if let Some(identity) = receipt {
            crate::source_receipts::insert_burst_successor_receipt(
                &transaction,
                identity,
                stored.runtime.revision,
            )?;
        }
        transaction.commit()?;
        Ok(stored)
    }

    /// Create a fresh explicit-item task from terminal history in the same
    /// paused state used by a newly confirmed NISAR subset task. The request
    /// and persistence schema are shared with the queued historical path; only
    /// the initial lifecycle partition differs.
    pub fn create_historical_paused_explicit_task(
        &mut self,
        request: &StoreCreateHistoricalExplicitTaskRequest,
    ) -> Result<StoredTask, StoreError> {
        self.create_historical_explicit_task_with_paused(request, true, None, None)
            .map(|(task, _)| task)
    }

    /// Persist the complete NISAR successor identity in the creation transaction.
    pub fn create_historical_paused_explicit_task_with_receipt(
        &mut self,
        request: &StoreCreateHistoricalExplicitTaskRequest,
        identity: &crate::StoreNisarSuccessorIdentity,
    ) -> Result<(StoredTask, bool), StoreError> {
        if identity.source_task_id != request.source_task_id
            || identity.expected_task_revision != request.expected_source_task_revision
            || identity.source_item_ids != request.source_item_ids
            || identity.new_task_id != request.task.task_id
            || identity.output_root != request.task.output_root
            || identity.layout != request.task.layout
            || identity.concurrency != request.task.concurrency
            || identity.created_at != request.task.created_at
        {
            return Err(StoreError::invalid(
                "successor identity differs from creation request",
            ));
        }
        self.create_historical_explicit_task_with_paused(request, true, Some(identity), None)
    }

    fn create_historical_explicit_task_with_paused(
        &mut self,
        request: &StoreCreateHistoricalExplicitTaskRequest,
        initially_paused: bool,
        receipt: Option<&crate::StoreNisarSuccessorIdentity>,
        burst_receipt: Option<&crate::StoreBurstSuccessorIdentity>,
    ) -> Result<(StoredTask, bool), StoreError> {
        if request.source_task_id.trim().is_empty()
            || request.source_task_id == request.task.task_id
        {
            return Err(StoreError::invalid(
                "historical explicit task IDs are invalid",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(identity) = burst_receipt {
            if let Some(stored) = burst_successor_replay(&transaction, identity)? {
                transaction.commit()?;
                return Ok((stored, true));
            }
        }
        if let Some(identity) = receipt {
            match crate::source_receipts::nisar_successor_status(&transaction, identity)? {
                crate::NisarSuccessorReceiptStatus::Committed => {
                    if row_exists(
                        &transaction,
                        "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
                        &request.task.task_id,
                    )? {
                        return Err(StoreError::NotFound);
                    }
                    let stored = load_task_connection(&transaction, &request.task.task_id)?
                        .ok_or(StoreError::NotFound)?;
                    transaction.commit()?;
                    return Ok((stored, true));
                }
                crate::NisarSuccessorReceiptStatus::Conflict => {
                    return Err(StoreError::AlreadyExists)
                }
                crate::NisarSuccessorReceiptStatus::Unknown => {}
            }
        }
        // The legacy creation receipt does not include the historical source
        // task or its revision. It cannot establish that an existing target
        // was created by this request, even if the inner task payload matches.
        if row_exists(
            &transaction,
            "SELECT EXISTS(SELECT 1 FROM task_snapshots WHERE task_id = ?1)",
            &request.task.task_id,
        )? {
            return Err(StoreError::AlreadyExists);
        }
        if row_exists(
            &transaction,
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            &request.source_task_id,
        )? {
            return Err(StoreError::NotFound);
        }
        if load_explicit_task_source_item_ids(&transaction, &request.source_task_id)?.is_none() {
            return Err(StoreError::invalid(
                "historical source is not an explicit-item task",
            ));
        }
        let source = load_task_connection(&transaction, &request.source_task_id)?
            .ok_or(StoreError::NotFound)?;
        if source.runtime.revision != request.expected_source_task_revision {
            return Err(StoreError::RevisionConflict);
        }
        if !matches!(
            source.runtime.status,
            TaskLifecycleStatus::Completed
                | TaskLifecycleStatus::Cancelled
                | TaskLifecycleStatus::Failed
        ) {
            return Err(StoreError::invalid(
                "historical explicit source task is not terminal",
            ));
        }
        if request.task.batch_id != source.snapshot.batch_id {
            return Err(StoreError::invalid(
                "historical successor must retain the source task batch",
            ));
        }
        if burst_receipt.is_some() {
            if crate::loading_admission::verified_progressive_source(
                &transaction,
                &source.snapshot,
            )?
            .is_some()
            {
                return Err(StoreError::invalid(
                    "progressive Burst source evidence is required",
                ));
            }
            let batch =
                load_batch(&transaction, &source.snapshot.batch_id)?.ok_or(StoreError::NotFound)?;
            let requested =
                serde_json::to_string(&request.source_item_ids).map_err(StoreError::Corrupt)?;
            let conflict: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM batch_task_items i JOIN task_runtime r ON r.task_id = i.task_id \
                 LEFT JOIN task_tombstones d ON d.task_id = i.task_id WHERE i.source_batch_id = ?1 \
                 AND i.source_item_id IN (SELECT value FROM json_each(?2)) AND d.task_id IS NULL \
                 AND r.status IN ('queued','running','paused','interrupted'))",
                params![source.snapshot.batch_id, requested], |row| row.get(0),
            )?;
            if conflict {
                return Err(StoreError::PartialActiveTaskOverlap);
            }
            let selected: HashSet<_> = request.source_item_ids.iter().map(String::as_str).collect();
            let parents = batch
                .candidates
                .into_iter()
                .filter(|item| selected.contains(item.item_id.as_str()))
                .collect::<Vec<_>>();
            if parents.len() != selected.len() {
                return Err(StoreError::invalid(
                    "Burst successor source products differ",
                ));
            }
            insar_core::validate_burst_task_items(
                &parents,
                &source.snapshot.candidates,
                &request.task_items,
            )
            .map_err(|_| {
                StoreError::invalid("Burst successor assets differ from source or policy")
            })?;
        }
        let stored = create_task_in_transaction_with_candidates(
            &transaction,
            &request.task,
            TaskCandidateMode::Explicit {
                source_item_ids: &request.source_item_ids,
                task_items: &request.task_items,
            },
            None,
            None,
            initially_paused,
        )?;
        if let Some(identity) = receipt {
            // A receipt certifies a fully admitted paused successor, not just
            // the existence of rows that the service later rejects.
            if stored.runtime.status != TaskLifecycleStatus::Paused
                || !stored.runtime.task_paused
                || !stored.runtime.queued_item_ids.is_empty()
                || stored.runtime.paused_item_ids.len() != stored.snapshot.candidates.len()
            {
                return Err(StoreError::invalid("NISAR successor is not fully paused"));
            }
            crate::source_receipts::insert_nisar_successor_receipt(
                &transaction,
                identity,
                stored.runtime.revision,
            )?;
        }
        if let Some(identity) = burst_receipt {
            crate::source_receipts::insert_burst_successor_receipt(
                &transaction,
                identity,
                stored.runtime.revision,
            )?;
        }
        transaction.commit()?;
        Ok((stored, false))
    }

    pub fn load_task(&self, task_id: &str) -> Result<Option<StoredTask>, StoreError> {
        let task = load_task_connection(&self.connection, task_id)?;
        if task.is_some() && !crate::core_restore::task_is_actionable(&self.connection, task_id)? {
            return Ok(None);
        }
        Ok(task)
    }

    /// Load only unarchived task records for the download scheduler.
    ///
    /// This deliberately avoids the active workbench and, for compact task
    /// snapshots, hydrates only the products frozen into the task. A large
    /// catalogue batch must not be deserialized on every scheduler tick.
    pub fn load_scheduler_tasks(&self) -> Result<Vec<StoredTask>, StoreError> {
        self.load_compact_tasks()
    }

    /// Load validated runtime control state for a task that is already live in
    /// the in-process scheduler.
    ///
    /// Unlike the scheduler task queries, this intentionally does not apply
    /// lifecycle, history, or tombstone filtering: a terminal cancellation must
    /// still reach a WorkerControl that started before the task left the
    /// schedulable set. It reads no task snapshot or search batch.
    pub fn load_scheduler_control_runtime(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskRuntimeSnapshot>, StoreError> {
        let json = self
            .connection
            .query_row(
                "SELECT state_json FROM task_runtime WHERE task_id = ?1",
                [task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|value| {
            let runtime: TaskRuntimeSnapshot =
                serde_json::from_str(&value).map_err(StoreError::Corrupt)?;
            if runtime.task_id != task_id {
                return Err(StoreError::invalid("runtime task ID differs"));
            }
            runtime.validate().map_err(StoreError::invalid)?;
            Ok(runtime)
        })
        .transpose()
    }

    fn load_compact_tasks(&self) -> Result<Vec<StoredTask>, StoreError> {
        let sql = "SELECT s.task_id, s.snapshot_json, r.state_json, \
                    v.checked_item_ids_json, v.page_index, v.page_size \
             FROM task_snapshots s \
             JOIN task_runtime r ON r.task_id = s.task_id \
             LEFT JOIN task_view_state v ON v.task_id = s.task_id \
             LEFT JOIN task_history h ON h.task_id = s.task_id \
             LEFT JOIN task_tombstones d ON d.task_id = s.task_id \
             WHERE h.task_id IS NULL AND d.task_id IS NULL \
               AND r.status IN ('queued', 'running', 'paused') \
             ORDER BY s.created_at, s.task_id";
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        })?;
        rows.map(|row| {
            let (task_id, snapshot, runtime, checked_item_ids, page_index, page_size) = row?;
            if !crate::core_restore::task_is_actionable(&self.connection, &task_id)? {
                return Ok(None);
            }
            let additions = load_task_membership_additions(&self.connection, &task_id)?;
            let task = decode_scheduler_task(&snapshot, &runtime, &additions)?;
            let task = crate::source_index::apply_task_item_rebinds(&self.connection, task)?;
            apply_task_view(task, checked_item_ids, page_index, page_size).map(Some)
        })
        .filter_map(|row| row.transpose())
        .collect()
    }

    pub fn load_scheduler_task(&self, task_id: &str) -> Result<Option<StoredTask>, StoreError> {
        load_scheduler_task_connection(&self.connection, task_id)
    }
}

pub(crate) fn save_search_batch_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    request: &StoreSearchBatchRequest,
) -> Result<(), StoreError> {
    request.validate().map_err(StoreError::invalid)?;
    validate_workbench(&request.batch, &request.workbench)?;
    let revision = sqlite_integer(request.workbench.revision, "workbench revision")?;
    let batch_json = serde_json::to_string(&request.batch).map_err(StoreError::Corrupt)?;
    let state_json = serde_json::to_string(&request.workbench).map_err(StoreError::Corrupt)?;
    if row_exists(
        transaction,
        "SELECT EXISTS(SELECT 1 FROM search_batches WHERE batch_id = ?1)",
        &request.batch.batch_id,
    )? {
        let existing = load_batch(transaction, &request.batch.batch_id)?
            .ok_or_else(|| StoreError::invalid("existing batch row cannot be loaded"))?;
        if existing != request.batch {
            return Err(StoreError::AlreadyExists);
        }
    } else {
        transaction.execute(
            "INSERT INTO search_batches(batch_id, provider_id, created_at, snapshot_json) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                request.batch.batch_id,
                request.batch.provider_id,
                request.batch.created_at,
                batch_json
            ],
        )?;
        crate::source_index::insert_batch_header(
            transaction,
            &request.batch,
            request.activate_workbench,
        )?;
        crate::batch_candidates::insert_batch_candidates(transaction, &request.batch)?;
    }
    if request.activate_workbench {
        upsert_workbench_state(transaction, request, revision, &state_json)?;
    }
    Ok(())
}

pub(crate) fn create_task_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    request: &StoreCreateTaskRequest,
    operation: Option<&TaskOperationDescriptor>,
    orbit_association: Option<&FrozenOrbitAssociationV1>,
) -> Result<StoredTask, StoreError> {
    create_task_in_transaction_with_candidates(
        transaction,
        request,
        TaskCandidateMode::SourceBatch,
        operation,
        orbit_association,
        false,
    )
}

pub(crate) fn create_task_with_explicit_items_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    request: &StoreCreateTaskRequest,
    source_item_ids: &[String],
    task_items: &[SnapshotItem],
    operation: Option<&TaskOperationDescriptor>,
    orbit_association: Option<&FrozenOrbitAssociationV1>,
) -> Result<StoredTask, StoreError> {
    create_task_in_transaction_with_candidates(
        transaction,
        request,
        TaskCandidateMode::Explicit {
            source_item_ids,
            task_items,
        },
        operation,
        orbit_association,
        false,
    )
}

#[derive(Clone, Copy)]
enum TaskCandidateMode<'a> {
    SourceBatch,
    Explicit {
        source_item_ids: &'a [String],
        task_items: &'a [SnapshotItem],
    },
}

fn create_task_in_transaction_with_candidates(
    transaction: &rusqlite::Transaction<'_>,
    request: &StoreCreateTaskRequest,
    candidate_mode: TaskCandidateMode<'_>,
    operation: Option<&TaskOperationDescriptor>,
    orbit_association: Option<&FrozenOrbitAssociationV1>,
    initially_paused: bool,
) -> Result<StoredTask, StoreError> {
    create_task_in_transaction_with_candidates_authorized(
        transaction,
        request,
        candidate_mode,
        operation,
        orbit_association,
        initially_paused,
        None,
    )
}

fn create_task_in_transaction_with_candidates_authorized(
    transaction: &rusqlite::Transaction<'_>,
    request: &StoreCreateTaskRequest,
    candidate_mode: TaskCandidateMode<'_>,
    operation: Option<&TaskOperationDescriptor>,
    orbit_association: Option<&FrozenOrbitAssociationV1>,
    initially_paused: bool,
    recovery: Option<&StoredNisarArchivedRecoveryV2>,
) -> Result<StoredTask, StoreError> {
    request.validate().map_err(StoreError::invalid)?;
    if request.output_root.as_os_str().is_empty() || request.selected_item_ids.is_empty() {
        return Err(StoreError::invalid(
            "task output and selection are required",
        ));
    }
    if row_exists(
        transaction,
        "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
        &request.task_id,
    )? {
        return Err(StoreError::AlreadyExists);
    }
    if let Some(operation) = operation.filter(|operation| {
        operation.kind == "gacos"
            || operation.phase == "request"
            || operation.approved_worker_operation.starts_with("gacos_")
    }) {
        if operation.approved_worker_operation == "gacos_import_result" {
            insar_core::validate_gacos_import_operation(operation).map_err(StoreError::invalid)?;
        } else if operation.approved_worker_operation == "gacos_fetch_result" {
            insar_core::validate_gacos_result_operation(operation).map_err(StoreError::invalid)?;
        } else {
            insar_core::validate_gacos_manual_request_operation(operation)
                .map_err(StoreError::invalid)?;
            if !initially_paused {
                return Err(StoreError::GacosManualActionRequired);
            }
        }
    }
    let create_payload = match (candidate_mode, initially_paused) {
        (TaskCandidateMode::SourceBatch, false)
            if operation.is_none() && orbit_association.is_none() =>
        {
            serde_json::to_string(request).map_err(StoreError::Corrupt)?
        }
        (TaskCandidateMode::SourceBatch, false) => {
            serde_json::to_string(&(request, operation, orbit_association))
                .map_err(StoreError::Corrupt)?
        }
        (
            TaskCandidateMode::Explicit {
                source_item_ids,
                task_items,
            },
            false,
        ) => serde_json::to_string(&(
            request,
            source_item_ids,
            task_items,
            operation,
            orbit_association,
        ))
        .map_err(StoreError::Corrupt)?,
        (
            TaskCandidateMode::Explicit {
                source_item_ids,
                task_items,
            },
            true,
        ) => serde_json::to_string(&(
            "initially_paused_v2",
            request,
            source_item_ids,
            task_items,
            operation,
            orbit_association,
        ))
        .map_err(StoreError::Corrupt)?,
        (TaskCandidateMode::SourceBatch, true) => {
            if !operation.is_some_and(|operation| {
                operation.kind == "gacos"
                    && operation.phase == "request"
                    && operation.approved_worker_operation == "gacos_manual_request"
            }) || orbit_association.is_some()
            {
                return Err(StoreError::invalid(
                    "only GACOS manual source-batch tasks may start paused",
                ));
            }
            serde_json::to_string(&("gacos_manual_request_v1", request, operation))
                .map_err(StoreError::Corrupt)?
        }
    };
    if let Some(existing) = load_task_connection(transaction, &request.task_id)? {
        let stored_payload = transaction.query_row(
            "SELECT payload_json FROM task_events WHERE task_id = ?1 AND sequence = 0",
            [&request.task_id],
            |row| row.get::<_, String>(0),
        )?;
        if stored_payload == create_payload {
            return Ok(existing);
        }
        return Err(StoreError::AlreadyExists);
    }
    if row_exists(
        transaction,
        "SELECT EXISTS(SELECT 1 FROM task_snapshots WHERE task_id = ?1)",
        &request.task_id,
    )? {
        return Err(StoreError::invalid("partial task rows cannot be replayed"));
    }
    let batch = load_batch(transaction, &request.batch_id)?
        .ok_or_else(|| StoreError::invalid("task batch does not exist"))?;
    let batch_candidate_ids = candidate_ids(&batch)?;
    let (task_candidates, selected_products, source_item_ids) = match candidate_mode {
        TaskCandidateMode::SourceBatch => {
            validate_id_subset(
                &request.selected_item_ids,
                &batch_candidate_ids,
                "selected item IDs",
            )?;
            validate_id_subset(
                &request.checked_item_ids,
                &batch_candidate_ids,
                "checked item IDs",
            )?;
            let selected_ids: HashSet<&str> = request
                .selected_item_ids
                .iter()
                .map(String::as_str)
                .collect();
            let selected_products = batch
                .candidates
                .iter()
                .filter(|item| selected_ids.contains(item.item_id.as_str()))
                .cloned()
                .collect();
            (batch.candidates.clone(), selected_products, Vec::new())
        }
        TaskCandidateMode::Explicit {
            source_item_ids,
            task_items,
        } => {
            validate_explicit_task_items(
                &batch,
                &batch_candidate_ids,
                request,
                source_item_ids,
                task_items,
            )?;
            (
                task_items.to_vec(),
                task_items.to_vec(),
                source_item_ids.to_vec(),
            )
        }
    };
    for item in &selected_products {
        if let Some(proof) = recovery {
            if !initially_paused || proof.new_task_id != request.task_id {
                return Err(StoreError::NisarSubsetExecutionPlanInvalid);
            }
            proof.validate_destination_item(item)?;
            crate::nisar_subset_plan::validate_nisar_subset_plan_v2_metadata(&item.metadata)?;
        } else {
            if item
                .metadata
                .contains_key(nisar_archived_recovery::METADATA_KEY)
            {
                return Err(StoreError::NisarSubsetExecutionPlanInvalid);
            }
            validate_nisar_subset_task_creation_metadata(&item.metadata, initially_paused)?;
        }
    }
    let aoi_geojson = batch.aoi_geojson.clone();
    let snapshot = FrozenTaskSnapshot {
        schema_version: batch.schema_version,
        task_id: request.task_id.clone(),
        batch_id: batch.batch_id.clone(),
        provider_id: batch.provider_id.clone(),
        created_at: request.created_at.clone(),
        aoi_name: batch.aoi_name.clone(),
        aoi_geojson,
        query: batch.query.clone(),
        candidates: task_candidates,
        initial_selected_item_ids: request.selected_item_ids.clone(),
        output_root: request.output_root.clone(),
        layout: request.layout.clone(),
        operation: operation.cloned(),
        orbit_association: orbit_association.cloned(),
    };
    snapshot.validate().map_err(StoreError::invalid)?;
    let persisted_snapshot = PersistedTaskSnapshotV2 {
        persistence_version: COMPACT_TASK_PERSISTENCE_VERSION,
        schema_version: snapshot.schema_version,
        task_id: snapshot.task_id.clone(),
        batch_id: snapshot.batch_id.clone(),
        provider_id: snapshot.provider_id.clone(),
        created_at: snapshot.created_at.clone(),
        aoi_name: snapshot.aoi_name.clone(),
        aoi_geojson: snapshot.aoi_geojson.clone(),
        query: snapshot.query.clone(),
        selected_products,
        source_item_ids,
        initial_selected_item_ids: snapshot.initial_selected_item_ids.clone(),
        output_root: snapshot.output_root.clone(),
        layout: snapshot.layout.clone(),
        operation: snapshot.operation.clone(),
        orbit_association: snapshot.orbit_association.clone(),
    };
    let runtime = TaskRuntimeSnapshot {
        task_id: request.task_id.clone(),
        status: if initially_paused {
            TaskLifecycleStatus::Paused
        } else {
            TaskLifecycleStatus::Queued
        },
        selected_item_ids: request.selected_item_ids.clone(),
        checked_item_ids: request.checked_item_ids.clone(),
        page_index: request.page_index,
        page_size: request.page_size,
        active_item_ids: Vec::new(),
        paused_item_ids: if initially_paused {
            request.selected_item_ids.clone()
        } else {
            Vec::new()
        },
        queued_item_ids: if initially_paused {
            Vec::new()
        } else {
            request.selected_item_ids.clone()
        },
        completed_item_ids: Vec::new(),
        failed_item_ids: Vec::new(),
        skipped_item_ids: Vec::new(),
        cancelled_item_ids: Vec::new(),
        task_paused: initially_paused,
        concurrency: request.concurrency,
        revision: 0,
        updated_at: request.created_at.clone(),
    };
    runtime.validate().map_err(StoreError::invalid)?;
    let snapshot_json = serde_json::to_string(&persisted_snapshot).map_err(StoreError::Corrupt)?;
    let runtime_json = serde_json::to_string(&runtime).map_err(StoreError::Corrupt)?;
    let checked_item_ids_json =
        serde_json::to_string(&runtime.checked_item_ids).map_err(StoreError::Corrupt)?;
    let layout = serde_json::to_value(&snapshot.layout)
        .map_err(StoreError::Corrupt)?
        .as_str()
        .expect("generated layout serializes as a string")
        .to_string();
    transaction.execute(
        "INSERT INTO task_snapshots( \
            task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json \
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            snapshot.task_id,
            snapshot.batch_id,
            snapshot.provider_id,
            snapshot.created_at,
            snapshot.output_root.to_string_lossy(),
            layout,
            snapshot_json
        ],
    )?;
    transaction.execute(
        "INSERT INTO task_runtime(task_id, status, revision, updated_at, state_json) \
         VALUES (?1, ?2, 0, ?3, ?4)",
        params![
            runtime.task_id,
            if initially_paused { "paused" } else { "queued" },
            runtime.updated_at,
            runtime_json
        ],
    )?;
    transaction.execute(
        "INSERT INTO task_view_state( \
            task_id, checked_item_ids_json, page_index, page_size, updated_at \
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            runtime.task_id,
            checked_item_ids_json,
            sqlite_integer(runtime.page_index, "task view page index")?,
            sqlite_integer(runtime.page_size, "task view page size")?,
            request.created_at
        ],
    )?;
    transaction.execute(
        "INSERT INTO task_events( \
            event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json \
         ) VALUES (?1, ?2, 0, ?3, 'task_created', NULL, ?4, NULL)",
        params![
            format!("{}:created", snapshot.task_id),
            snapshot.task_id,
            snapshot.created_at,
            create_payload
        ],
    )?;
    crate::source_index::index_task_items(
        transaction,
        &snapshot,
        matches!(candidate_mode, TaskCandidateMode::Explicit { .. }),
        &runtime.selected_item_ids,
    )?;
    Ok(StoredTask { snapshot, runtime })
}

fn validate_nisar_subset_task_creation_metadata(
    metadata: &serde_json::Map<String, serde_json::Value>,
    initially_paused: bool,
) -> Result<(), StoreError> {
    if metadata.contains_key(
        crate::nisar_subset_plan::NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_METADATA_KEY,
    ) {
        if !initially_paused {
            return Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch);
        }
        crate::nisar_subset_plan::validate_nisar_subset_prepared_intent_only_metadata(metadata)
    } else {
        crate::nisar_subset_plan::validate_nisar_subset_plan_v2_metadata(metadata)
    }
}

fn validate_explicit_task_items(
    batch: &FrozenSearchBatch,
    batch_candidate_ids: &HashSet<&str>,
    request: &StoreCreateTaskRequest,
    source_item_ids: &[String],
    task_items: &[SnapshotItem],
) -> Result<(), StoreError> {
    if source_item_ids.is_empty() || task_items.is_empty() {
        return Err(StoreError::invalid(
            "explicit task items and source item IDs are required",
        ));
    }
    validate_id_subset(source_item_ids, batch_candidate_ids, "source item IDs")?;
    let source_ids: HashSet<&str> = source_item_ids.iter().map(String::as_str).collect();
    if source_ids.len() != source_item_ids.len() {
        return Err(StoreError::invalid("source item IDs must be unique"));
    }
    let task_candidate_ids = task_items
        .iter()
        .map(|item| item.item_id.as_str())
        .collect::<HashSet<_>>();
    if task_candidate_ids.len() != task_items.len()
        || task_items
            .iter()
            .any(|item| item.provider_id != batch.provider_id)
    {
        return Err(StoreError::invalid(
            "explicit task items must be unique and match the source provider",
        ));
    }
    validate_id_subset(
        &request.selected_item_ids,
        &task_candidate_ids,
        "selected explicit item IDs",
    )?;
    validate_id_subset(
        &request.checked_item_ids,
        &task_candidate_ids,
        "checked explicit item IDs",
    )?;
    let selected_ids = request
        .selected_item_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if selected_ids != task_candidate_ids {
        return Err(StoreError::invalid(
            "every explicit task item must be initially selected",
        ));
    }
    let mut referenced = HashSet::new();
    for item in task_items {
        crate::nisar_subset_plan::validate_nisar_subset_plan_v2_metadata(&item.metadata)?;
        let source_item_id = item
            .metadata
            .get("source_item_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| StoreError::invalid("explicit task item lacks its source item ID"))?;
        if !source_ids.contains(source_item_id) {
            return Err(StoreError::invalid(
                "explicit task item refers outside its source products",
            ));
        }
        referenced.insert(source_item_id);
    }
    if referenced != source_ids {
        return Err(StoreError::invalid(
            "explicit task source products are not represented exactly",
        ));
    }
    Ok(())
}

fn validate_nisar_subset_execution_anchor_identity(
    task_id: &str,
    item_id: &str,
    anchor_kind: &str,
) -> Result<(), StoreError> {
    if task_id.is_empty()
        || task_id.len() > 128
        || task_id.trim() != task_id
        || item_id.is_empty()
        || item_id.len() > 512
        || item_id.trim() != item_id
        || !matches!(
            anchor_kind,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
                | NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND
        )
    {
        return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
    }
    Ok(())
}

fn validate_nisar_subset_execution_anchor_material(
    lineage_key_sha256: &str,
    expected_record_sha256: Option<&str>,
    record_sha256: &str,
    canonical_json: &str,
) -> Result<(), StoreError> {
    if !valid_lowercase_sha256(lineage_key_sha256)
        || expected_record_sha256.is_some_and(|digest| !valid_lowercase_sha256(digest))
        || !valid_lowercase_sha256(record_sha256)
        || canonical_json.len() < 2
        || canonical_json.len() > NISAR_SUBSET_EXECUTION_ANCHOR_MAX_CANONICAL_BYTES
        || !compact_json_has_no_insignificant_whitespace(canonical_json)
        || serde_json::from_str::<serde_json::Value>(canonical_json).is_err()
    {
        return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
    }
    Ok(())
}

fn valid_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn compact_json_has_no_insignificant_whitespace(value: &str) -> bool {
    let mut inside_string = false;
    let mut escaped = false;
    for byte in value.bytes() {
        if inside_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                inside_string = false;
            }
        } else if byte == b'"' {
            inside_string = true;
        } else if byte.is_ascii_whitespace() {
            return false;
        }
    }
    !inside_string && !escaped
}

fn upsert_workbench_state(
    transaction: &rusqlite::Transaction<'_>,
    request: &StoreSearchBatchRequest,
    revision: i64,
    state_json: &str,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO workbench_state(slot, batch_id, revision, updated_at, state_json) \
         VALUES (1, ?1, ?2, ?3, ?4) \
         ON CONFLICT(slot) DO UPDATE SET \
             batch_id = excluded.batch_id, revision = excluded.revision, \
             updated_at = excluded.updated_at, state_json = excluded.state_json",
        params![
            request.workbench.batch_id,
            revision,
            request.batch.created_at,
            state_json
        ],
    )?;
    Ok(())
}

pub(crate) fn load_task_connection(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<StoredTask>, StoreError> {
    let row = connection
        .query_row(
            "SELECT s.snapshot_json, b.snapshot_json, r.state_json, \
                    v.checked_item_ids_json, v.page_index, v.page_size \
                 FROM task_snapshots s \
                 JOIN search_batches b ON b.batch_id = s.batch_id \
                 JOIN task_runtime r ON r.task_id = s.task_id \
                 LEFT JOIN task_view_state v ON v.task_id = s.task_id \
                 WHERE s.task_id = ?1",
            [task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(snapshot, batch, runtime, checked_item_ids, page_index, page_size)| {
            let additions = load_task_membership_additions(connection, task_id)?;
            let task = decode_task(&snapshot, &batch, &runtime, &additions)?;
            let task = crate::source_index::apply_task_item_rebinds(connection, task)?;
            apply_task_view(task, checked_item_ids, page_index, page_size)
        },
    )
    .transpose()
}

pub(crate) fn load_compact_task_connection(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<StoredTask>, StoreError> {
    let row = connection
        .query_row(
            "SELECT s.snapshot_json, r.state_json, \
                    v.checked_item_ids_json, v.page_index, v.page_size \
             FROM task_snapshots s \
             JOIN task_runtime r ON r.task_id = s.task_id \
             LEFT JOIN task_view_state v ON v.task_id = s.task_id \
             WHERE s.task_id = ?1",
            [task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(snapshot, runtime, checked_item_ids, page_index, page_size)| {
            let additions = load_task_membership_additions(connection, task_id)?;
            let task = decode_scheduler_task(&snapshot, &runtime, &additions)?;
            let task = crate::source_index::apply_task_item_rebinds(connection, task)?;
            apply_task_view(task, checked_item_ids, page_index, page_size)
        },
    )
    .transpose()
}

pub(crate) fn load_explicit_task_source_item_ids(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<Vec<String>>, StoreError> {
    let snapshot = connection
        .query_row(
            "SELECT snapshot_json FROM task_snapshots WHERE task_id = ?1",
            [task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    snapshot
        .map(|snapshot| match decode_v2_only_snapshot(&snapshot)? {
            StoredTaskSnapshot::CompactV2(compact) if !compact.source_item_ids.is_empty() => {
                Ok(Some(compact.source_item_ids))
            }
            StoredTaskSnapshot::CompactV2(_) | StoredTaskSnapshot::LegacyV2(_) => Ok(None),
            StoredTaskSnapshot::V3(_) => Err(StoreError::v3_snapshot_rejected_by_v2_reader()),
        })
        .transpose()
        .map(Option::flatten)
}

pub(crate) fn load_scheduler_task_connection(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<StoredTask>, StoreError> {
    if !crate::core_restore::task_is_actionable(connection, task_id)? {
        return Ok(None);
    }
    let row = connection
        .query_row(
            "SELECT s.snapshot_json, r.state_json, \
                    v.checked_item_ids_json, v.page_index, v.page_size \
             FROM task_snapshots s \
             JOIN task_runtime r ON r.task_id = s.task_id \
             LEFT JOIN task_view_state v ON v.task_id = s.task_id \
             LEFT JOIN task_history h ON h.task_id = s.task_id \
             LEFT JOIN task_tombstones d ON d.task_id = s.task_id \
             WHERE s.task_id = ?1 AND h.task_id IS NULL AND d.task_id IS NULL \
               AND r.status IN ('queued', 'running', 'paused')",
            [task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(snapshot, runtime, checked_item_ids, page_index, page_size)| {
            let additions = load_task_membership_additions(connection, task_id)?;
            let task = decode_scheduler_task(&snapshot, &runtime, &additions)?;
            let task = crate::source_index::apply_task_item_rebinds(connection, task)?;
            apply_task_view(task, checked_item_ids, page_index, page_size)
        },
    )
    .transpose()
}

fn apply_task_view(
    mut task: StoredTask,
    checked_item_ids: Option<String>,
    page_index: Option<i64>,
    page_size: Option<i64>,
) -> Result<StoredTask, StoreError> {
    if let (Some(checked_item_ids), Some(page_index), Some(page_size)) =
        (checked_item_ids, page_index, page_size)
    {
        task.runtime.checked_item_ids =
            serde_json::from_str(&checked_item_ids).map_err(StoreError::Corrupt)?;
        task.runtime.page_index = u64::try_from(page_index)
            .map_err(|_| StoreError::invalid("negative task view page index"))?;
        task.runtime.page_size = u64::try_from(page_size)
            .map_err(|_| StoreError::invalid("negative task view page size"))?;
        task.runtime.validate().map_err(StoreError::invalid)?;
        validate_id_subset(
            &task.runtime.checked_item_ids,
            &candidate_ids_from_task(&task.snapshot)?,
            "task view checked IDs",
        )?;
    }
    Ok(task)
}

fn row_exists(connection: &Connection, sql: &str, id: &str) -> Result<bool, StoreError> {
    Ok(connection.query_row(sql, [id], |row| row.get(0))?)
}

fn validate_workspace_context_hierarchy(context: &WorkspaceContextState) -> Result<(), StoreError> {
    let mut project_ids = HashSet::<&str>::new();
    let mut region_ids = HashSet::<&str>::new();
    for project in &context.projects {
        if project.workspace_id != context.workspace_id || !project_ids.insert(&project.project_id)
        {
            return Err(StoreError::invalid(
                "workspace project hierarchy is invalid",
            ));
        }
        for region in &project.regions {
            if region.project_id != project.project_id || !region_ids.insert(&region.region_id) {
                return Err(StoreError::invalid("workspace region hierarchy is invalid"));
            }
        }
    }
    if context.regions.len() != region_ids.len()
        || context
            .regions
            .iter()
            .any(|region| !region_ids.contains(region.region_id.as_str()))
    {
        return Err(StoreError::invalid(
            "workspace region projection is not canonical",
        ));
    }
    if let Some(project_id) = context.project_id.as_deref() {
        if !project_ids.contains(project_id) {
            return Err(StoreError::invalid("selected project is outside workspace"));
        }
        if let Some(region_id) = context.region_id.as_deref() {
            let selected = context
                .projects
                .iter()
                .find(|project| project.project_id == project_id)
                .is_some_and(|project| {
                    project
                        .regions
                        .iter()
                        .any(|region| region.region_id == region_id)
                });
            if !selected {
                return Err(StoreError::invalid(
                    "selected region is outside selected project",
                ));
            }
        }
    } else if context.region_id.is_some() {
        return Err(StoreError::invalid(
            "a region cannot be selected without a project",
        ));
    }
    Ok(())
}

pub(crate) fn sqlite_integer(value: u64, label: &str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::invalid(format!("{label} exceeds SQLite range")))
}

pub(crate) fn load_batch(
    connection: &Connection,
    batch_id: &str,
) -> Result<Option<FrozenSearchBatch>, StoreError> {
    let json = connection
        .query_row(
            "SELECT snapshot_json FROM search_batches WHERE batch_id = ?1",
            [batch_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    json.map(|value| {
        let batch: FrozenSearchBatch = serde_json::from_str(&value).map_err(StoreError::Corrupt)?;
        batch.validate().map_err(StoreError::invalid)?;
        Ok(batch)
    })
    .transpose()
}

fn decode_workbench(batch: &str, state: &str) -> Result<StoredWorkbench, StoreError> {
    let batch: FrozenSearchBatch = serde_json::from_str(batch).map_err(StoreError::Corrupt)?;
    let state: SearchWorkbenchState = serde_json::from_str(state).map_err(StoreError::Corrupt)?;
    batch.validate().map_err(StoreError::invalid)?;
    state.validate().map_err(StoreError::invalid)?;
    validate_workbench(&batch, &state)?;
    Ok(StoredWorkbench { batch, state })
}

fn decode_task(
    snapshot: &str,
    batch: &str,
    runtime: &str,
    additions: &[SnapshotItem],
) -> Result<StoredTask, StoreError> {
    let batch: FrozenSearchBatch = serde_json::from_str(batch).map_err(StoreError::Corrupt)?;
    batch.validate().map_err(StoreError::invalid)?;
    let snapshot = match decode_v2_only_snapshot(snapshot)? {
        StoredTaskSnapshot::LegacyV2(legacy) => {
            validate_legacy_task_batch(&legacy, &batch)?;
            legacy
        }
        StoredTaskSnapshot::CompactV2(compact) => {
            let explicit = !compact.source_item_ids.is_empty();
            let mut snapshot = hydrate_compact_task(compact, &batch)?;
            if explicit || crate::loading_admission::is_progressive_admission_query(&snapshot.query)
            {
                let existing = snapshot
                    .candidates
                    .iter()
                    .map(|item| item.item_id.as_str())
                    .collect::<HashSet<_>>();
                if additions
                    .iter()
                    .any(|addition| existing.contains(addition.item_id.as_str()))
                {
                    return Err(StoreError::invalid(
                        "explicit task additions overlap its initially frozen items",
                    ));
                }
                snapshot.candidates.extend_from_slice(additions);
            }
            snapshot
        }
        StoredTaskSnapshot::V3(_) => return Err(StoreError::v3_snapshot_rejected_by_v2_reader()),
    };
    let runtime: TaskRuntimeSnapshot =
        serde_json::from_str(runtime).map_err(StoreError::Corrupt)?;
    snapshot.validate().map_err(StoreError::invalid)?;
    validate_runtime_item_graph(&snapshot.candidates, &runtime)?;
    if snapshot.task_id != runtime.task_id {
        return Err(StoreError::invalid("snapshot and runtime task IDs differ"));
    }
    let candidates = candidate_ids_from_task(&snapshot)?;
    validate_id_subset(
        &runtime.selected_item_ids,
        &candidates,
        "runtime selected IDs",
    )?;
    validate_id_subset(
        &runtime.checked_item_ids,
        &candidates,
        "runtime checked IDs",
    )?;
    Ok(StoredTask { snapshot, runtime })
}

fn decode_scheduler_task(
    snapshot: &str,
    runtime: &str,
    additions: &[SnapshotItem],
) -> Result<StoredTask, StoreError> {
    let snapshot = decode_v2_only_snapshot(snapshot)?;
    let runtime: TaskRuntimeSnapshot =
        serde_json::from_str(runtime).map_err(StoreError::Corrupt)?;
    let snapshot = hydrate_task_snapshot_for_runtime(snapshot, additions, &runtime)?;
    Ok(StoredTask { snapshot, runtime })
}

pub(crate) fn hydrate_task_snapshot_for_runtime(
    snapshot: StoredTaskSnapshot,
    additions: &[SnapshotItem],
    runtime: &TaskRuntimeSnapshot,
) -> Result<FrozenTaskSnapshot, StoreError> {
    let snapshot = match snapshot {
        StoredTaskSnapshot::CompactV2(mut compact) => {
            if compact.persistence_version != COMPACT_TASK_PERSISTENCE_VERSION {
                return Err(StoreError::unsupported_snapshot_version());
            }
            let selected_ids: HashSet<&str> = compact
                .selected_products
                .iter()
                .map(|item| item.item_id.as_str())
                .collect();
            validate_id_subset(
                &compact.initial_selected_item_ids,
                &selected_ids,
                "compact task selected IDs",
            )?;
            if additions
                .iter()
                .any(|addition| selected_ids.contains(addition.item_id.as_str()))
            {
                return Err(StoreError::invalid(
                    "task membership additions overlap its initial compact products",
                ));
            }
            compact.selected_products.extend_from_slice(additions);
            FrozenTaskSnapshot {
                schema_version: compact.schema_version,
                task_id: compact.task_id,
                batch_id: compact.batch_id,
                provider_id: compact.provider_id,
                created_at: compact.created_at,
                aoi_name: compact.aoi_name,
                aoi_geojson: compact.aoi_geojson,
                query: compact.query,
                candidates: compact.selected_products,
                initial_selected_item_ids: compact.initial_selected_item_ids,
                output_root: compact.output_root,
                layout: compact.layout,
                operation: compact.operation,
                orbit_association: compact.orbit_association,
            }
        }
        StoredTaskSnapshot::LegacyV2(snapshot) => {
            let by_id = snapshot
                .candidates
                .iter()
                .map(|item| (item.item_id.as_str(), item))
                .collect::<std::collections::HashMap<_, _>>();
            if additions
                .iter()
                .any(|addition| by_id.get(addition.item_id.as_str()).copied() != Some(addition))
            {
                return Err(StoreError::invalid(
                    "legacy task membership addition differs from its frozen candidates",
                ));
            }
            snapshot
        }
        StoredTaskSnapshot::V3(_) => return Err(StoreError::v3_snapshot_rejected_by_v2_reader()),
    };
    snapshot.validate().map_err(StoreError::invalid)?;
    validate_runtime_item_graph(&snapshot.candidates, runtime)?;
    if snapshot.task_id != runtime.task_id {
        return Err(StoreError::invalid("snapshot and runtime task IDs differ"));
    }
    let candidates = candidate_ids_from_task(&snapshot)?;
    validate_id_subset(
        &runtime.selected_item_ids,
        &candidates,
        "runtime selected IDs",
    )?;
    validate_id_subset(
        &runtime.checked_item_ids,
        &candidates,
        "runtime checked IDs",
    )?;
    Ok(snapshot)
}

pub(crate) fn load_task_membership_additions(
    connection: &Connection,
    task_id: &str,
) -> Result<Vec<SnapshotItem>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT item_id, item_json
         FROM task_membership_additions
         WHERE task_id = ?1
         ORDER BY added_revision, position",
    )?;
    let rows = statement.query_map([task_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.map(|row| {
        let (item_id, item_json) = row?;
        decode_task_membership_addition(&item_id, &item_json)
    })
    .collect()
}

pub(crate) fn load_task_membership_additions_batched(
    connection: &Connection,
    task_ids: &[String],
) -> Result<HashMap<String, Option<Vec<SnapshotItem>>>, StoreError> {
    if task_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let task_ids_json = serde_json::to_string(task_ids).map_err(StoreError::Corrupt)?;
    let mut statement = connection.prepare(
        "SELECT addition.task_id, addition.item_id, addition.item_json
         FROM task_membership_additions addition
         JOIN json_each(?1) requested ON requested.value = addition.task_id
         ORDER BY addition.task_id, addition.added_revision, addition.position",
    )?;
    let rows = statement.query_map([task_ids_json], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut by_task = HashMap::<String, Option<Vec<SnapshotItem>>>::new();
    for row in rows {
        let (task_id, item_id, item_json) = row?;
        if matches!(by_task.get(&task_id), Some(None)) {
            continue;
        }
        match decode_task_membership_addition(&item_id, &item_json) {
            Ok(item) => by_task
                .entry(task_id)
                .or_insert_with(|| Some(Vec::new()))
                .as_mut()
                .expect("valid addition accumulator")
                .push(item),
            Err(_) => {
                by_task.insert(task_id, None);
            }
        }
    }
    Ok(by_task)
}

fn decode_task_membership_addition(
    item_id: &str,
    item_json: &str,
) -> Result<SnapshotItem, StoreError> {
    let item: SnapshotItem = serde_json::from_str(item_json).map_err(StoreError::Corrupt)?;
    if item.item_id != item_id {
        return Err(StoreError::invalid(
            "task membership row item ID differs from its frozen product",
        ));
    }
    Ok(item)
}

/// Shared Store authority for the runtime/item graph used by CoreRestore,
/// task-page reads, scheduler reads, and materialized terminal history.
pub(crate) fn validate_runtime_item_graph(
    candidates: &[SnapshotItem],
    runtime: &TaskRuntimeSnapshot,
) -> Result<(), StoreError> {
    validate_runtime_lifecycle_partition(runtime)?;
    let candidate_ids = candidates
        .iter()
        .map(|item| item.item_id.as_str())
        .collect::<HashSet<_>>();
    let selected = runtime
        .selected_item_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let checked = runtime
        .checked_item_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if candidates.is_empty()
        || selected.is_empty()
        || candidate_ids.len() != candidates.len()
        || !selected.is_subset(&candidate_ids)
        || !checked.is_subset(&selected)
    {
        return Err(StoreError::invalid(
            "task runtime item authority differs from the frozen candidates",
        ));
    }
    Ok(())
}

pub(crate) fn validate_runtime_lifecycle_partition(
    runtime: &TaskRuntimeSnapshot,
) -> Result<(), StoreError> {
    runtime.validate().map_err(StoreError::invalid)?;
    let lifecycle_count = runtime
        .active_item_ids
        .len()
        .checked_add(runtime.paused_item_ids.len())
        .and_then(|value| value.checked_add(runtime.queued_item_ids.len()))
        .and_then(|value| value.checked_add(runtime.completed_item_ids.len()))
        .and_then(|value| value.checked_add(runtime.failed_item_ids.len()))
        .and_then(|value| value.checked_add(runtime.skipped_item_ids.len()))
        .and_then(|value| value.checked_add(runtime.cancelled_item_ids.len()))
        .ok_or_else(|| StoreError::invalid("task lifecycle item count overflow"))?;
    if lifecycle_count != runtime.selected_item_ids.len() {
        return Err(StoreError::invalid(
            "every selected task item must have exactly one lifecycle state",
        ));
    }
    Ok(())
}

fn hydrate_compact_task(
    compact: PersistedTaskSnapshotV2,
    batch: &FrozenSearchBatch,
) -> Result<FrozenTaskSnapshot, StoreError> {
    if compact.persistence_version != COMPACT_TASK_PERSISTENCE_VERSION {
        return Err(StoreError::invalid(
            "unsupported compact task persistence version",
        ));
    }
    if compact.batch_id != batch.batch_id
        || compact.provider_id != batch.provider_id
        || compact.schema_version != batch.schema_version
        || compact.aoi_name != batch.aoi_name
        || compact.aoi_geojson != batch.aoi_geojson
        || compact.query != batch.query
    {
        return Err(StoreError::invalid(
            "compact task does not match its search batch",
        ));
    }
    let batch_by_id = batch
        .candidates
        .iter()
        .map(|item| (item.item_id.as_str(), item))
        .collect::<std::collections::HashMap<_, _>>();
    if compact.source_item_ids.is_empty() {
        for selected in &compact.selected_products {
            if batch_by_id.get(selected.item_id.as_str()).copied() != Some(selected) {
                return Err(StoreError::invalid(
                    "compact task selected product differs from its search batch",
                ));
            }
        }
    } else {
        let source_ids = compact
            .source_item_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if source_ids.len() != compact.source_item_ids.len()
            || !source_ids
                .iter()
                .all(|item_id| batch_by_id.contains_key(item_id))
        {
            return Err(StoreError::invalid(
                "explicit task source products differ from its search batch",
            ));
        }
    }
    let selected_ids: HashSet<&str> = compact
        .selected_products
        .iter()
        .map(|item| item.item_id.as_str())
        .collect();
    validate_id_subset(
        &compact.initial_selected_item_ids,
        &selected_ids,
        "compact task selected IDs",
    )?;
    let candidates = if compact.source_item_ids.is_empty() {
        batch.candidates.clone()
    } else {
        compact.selected_products.clone()
    };
    Ok(FrozenTaskSnapshot {
        schema_version: compact.schema_version,
        task_id: compact.task_id,
        batch_id: compact.batch_id,
        provider_id: compact.provider_id,
        created_at: compact.created_at,
        aoi_name: compact.aoi_name,
        aoi_geojson: compact.aoi_geojson,
        query: compact.query,
        candidates,
        initial_selected_item_ids: compact.initial_selected_item_ids,
        output_root: compact.output_root,
        layout: compact.layout,
        operation: compact.operation,
        orbit_association: compact.orbit_association,
    })
}

fn validate_legacy_task_batch(
    legacy: &FrozenTaskSnapshot,
    batch: &FrozenSearchBatch,
) -> Result<(), StoreError> {
    if legacy.batch_id != batch.batch_id
        || legacy.provider_id != batch.provider_id
        || legacy.schema_version != batch.schema_version
        || legacy.aoi_name != batch.aoi_name
        || legacy.aoi_geojson != batch.aoi_geojson
        || legacy.query != batch.query
        || legacy.candidates != batch.candidates
    {
        return Err(StoreError::invalid(
            "legacy task does not match its search batch",
        ));
    }
    Ok(())
}

fn validate_workbench(
    batch: &FrozenSearchBatch,
    state: &SearchWorkbenchState,
) -> Result<(), StoreError> {
    if batch.batch_id != state.batch_id {
        return Err(StoreError::invalid("workbench batch ID differs"));
    }
    validate_id_subset(
        &state.checked_item_ids,
        &candidate_ids(batch)?,
        "workbench checked IDs",
    )
}

fn candidate_ids(batch: &FrozenSearchBatch) -> Result<HashSet<&str>, StoreError> {
    let ids: HashSet<&str> = batch
        .candidates
        .iter()
        .map(|item| item.item_id.as_str())
        .collect();
    if ids.len() != batch.candidates.len() {
        return Err(StoreError::invalid("candidate IDs are not unique"));
    }
    Ok(ids)
}

pub(crate) fn candidate_ids_from_task(
    snapshot: &FrozenTaskSnapshot,
) -> Result<HashSet<&str>, StoreError> {
    let ids: HashSet<&str> = snapshot
        .candidates
        .iter()
        .map(|item| item.item_id.as_str())
        .collect();
    if ids.len() != snapshot.candidates.len() {
        return Err(StoreError::invalid("candidate IDs are not unique"));
    }
    Ok(ids)
}

pub(crate) fn validate_id_subset(
    values: &[String],
    candidates: &HashSet<&str>,
    label: &str,
) -> Result<(), StoreError> {
    let unique: HashSet<&str> = values.iter().map(String::as_str).collect();
    if unique.len() != values.len() || !unique.is_subset(candidates) {
        return Err(StoreError::invalid(format!(
            "{label} must be a unique candidate subset"
        )));
    }
    Ok(())
}

/// #22 B slice 3(b)/3(c): the streaming seal inside a caller-owned
/// transaction. Reads the staged rows in bounded batches ordered by item ID
/// (the classic seal's frozen order), streams them into the snapshot blob,
/// copies them into the published candidate rows with `INSERT ... SELECT`,
/// writes header/index/workbench/cache pointer, clears the staging and, for
/// a progressively published search (#22 B slice 3(c)), records `terminal`
/// as the batch's delivery state. Nothing is committed here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn seal_staging_streaming_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    cursor: &std::sync::Mutex<usize>,
    request_id: &str,
    search_stats: Option<FrozenSearchStats>,
    activate_workbench: bool,
    cache_pointer: Option<(&str, i64)>,
    terminal: SearchBatchProgressState,
) -> Result<(), StoreError> {
    let view_eligible =
        activate_workbench || matches!(terminal, SearchBatchProgressState::Interrupted);
    seal_staging_streaming_in_transaction_with_eligibility(
        transaction,
        cursor,
        request_id,
        search_stats,
        activate_workbench,
        view_eligible,
        cache_pointer,
        terminal,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn seal_staging_streaming_in_transaction_with_eligibility(
    transaction: &rusqlite::Transaction<'_>,
    cursor: &std::sync::Mutex<usize>,
    request_id: &str,
    search_stats: Option<FrozenSearchStats>,
    activate_workbench_slot: bool,
    view_eligible: bool,
    cache_pointer: Option<(&str, i64)>,
    terminal: SearchBatchProgressState,
) -> Result<(), StoreError> {
    let staged = transaction
        .query_row(
            "SELECT batch_header_json, workbench_json, max_candidates
             FROM search_batch_staging WHERE request_id = ?1",
            [request_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::NotFound)?;
    let mut batch: FrozenSearchBatch =
        serde_json::from_str(&staged.0).map_err(StoreError::Corrupt)?;
    let workbench: SearchWorkbenchState =
        serde_json::from_str(&staged.1).map_err(StoreError::Corrupt)?;
    batch.candidates = Vec::new();

    // Stream the staged rows in bounded batches, building the snapshot
    // blob with serde_json's streaming serializer so the serialized form
    // stays byte-identical to `serde_json::to_string` of the whole batch.
    let mut blob = StreamingBatchBlob::new(&batch, search_stats.as_ref())?;
    let mut statement = transaction.prepare(
        "SELECT item_id, item_json FROM search_batch_staging_candidates
         WHERE request_id = ?1 ORDER BY item_id ASC",
    )?;
    let mut rows = statement.query([request_id])?;
    let mut batch_buffer: Vec<SnapshotItem> = Vec::with_capacity(SEAL_READ_BATCH);
    let mut candidate_count: u64 = 0;
    loop {
        batch_buffer.clear();
        while batch_buffer.len() < SEAL_READ_BATCH {
            match rows.next()? {
                Some(row) => {
                    let json: String = row.get(1)?;
                    batch_buffer.push(serde_json::from_str(&json).map_err(StoreError::Corrupt)?);
                }
                None => break,
            }
        }
        if batch_buffer.is_empty() {
            break;
        }
        for candidate in &batch_buffer {
            candidate.validate().map_err(StoreError::invalid)?;
            blob.push_candidate(candidate)?;
            candidate_count = candidate_count.checked_add(1).ok_or_else(|| {
                StoreError::invalid("staged candidate count exceeds platform range")
            })?;
        }
        let observed = batch_buffer.len();
        cursor
            .lock()
            .map(|mut cursor| {
                if observed > *cursor {
                    *cursor = observed;
                }
            })
            .map_err(|_| StoreError::invalid("seal read-batch tracking poisoned"))?;
    }
    drop(rows);
    drop(statement);
    let snapshot_json = blob.finish();

    let candidate_count = i64::try_from(candidate_count)
        .map_err(|_| StoreError::invalid("staged candidate count exceeds SQLite range"))?;
    if candidate_count > staged.2
        || search_stats
            .as_ref()
            .is_some_and(|stats| stats.frozen_candidate_count != candidate_count as u64)
    {
        return Err(StoreError::invalid(
            "sealed search batch staging counts are inconsistent",
        ));
    }

    let batch_id = batch.batch_id.clone();
    let provider_id = batch.provider_id.clone();
    let created_at = batch.created_at.clone();
    // #22 B slice 3(b) review fix 5: validate the exact structure that
    // will be persisted WITHOUT decoding the whole snapshot blob. Each
    // staged candidate was already validated row-by-row above; the batch
    // header (with the final statistics) is validated in Rust here. The
    // frozen-candidate uniqueness and provider identity that
    // FrozenSearchBatch::validate would re-check on a decoded blob are
    // guaranteed by the staging table's per-request unique item_id and
    // re-checked below in SQL.
    let mut header = batch.clone();
    header.search_stats = search_stats.clone();
    header.candidates = Vec::new();
    header.validate().map_err(StoreError::invalid)?;
    // Staged rows are unique per request by schema; provider identity of
    // every staged row must match the batch provider, checked in SQL.
    let mismatched_providers: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM search_batch_staging_candidates AS s              WHERE s.request_id = ?1                AND json_extract(s.item_json, '$.provider_id') IS NOT ?2",
        params![request_id, provider_id],
        |row| row.get(0),
    )?;
    if mismatched_providers != 0 {
        return Err(StoreError::invalid(
            "candidate provider IDs must match the batch provider",
        ));
    }
    let revision = sqlite_integer(workbench.revision, "workbench revision")?;
    let state_json = serde_json::to_string(&workbench).map_err(StoreError::Corrupt)?;
    // The staged workbench selection must be a subset of the staged ids.
    // An empty selection (the streaming path's only shape) is trivially
    // valid; a nonempty one is checked in SQL so no id list is loaded.
    if !workbench.checked_item_ids.is_empty() {
        let selection_json =
            serde_json::to_string(&workbench.checked_item_ids).map_err(StoreError::Corrupt)?;
        let invalid_selection: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM json_each(?1) AS checked                  WHERE NOT EXISTS(                      SELECT 1 FROM search_batch_staging_candidates AS s                      WHERE s.request_id = ?2 AND s.item_id = checked.value                  )",
            params![selection_json, request_id],
            |row| row.get(0),
        )?;
        if invalid_selection != 0 {
            return Err(StoreError::invalid(
                "workbench checked IDs must be a unique candidate subset",
            ));
        }
    }
    if row_exists(
        transaction,
        "SELECT EXISTS(SELECT 1 FROM search_batches WHERE batch_id = ?1)",
        &batch_id,
    )? {
        return Err(StoreError::AlreadyExists);
    }
    transaction.execute(
        "INSERT INTO search_batches(batch_id, provider_id, created_at, snapshot_json)              VALUES (?1, ?2, ?3, ?4)",
        params![batch_id, provider_id, created_at, snapshot_json],
    )?;
    // The published candidate rows repeat the staged rows in the same
    // frozen order; INSERT ... SELECT never materializes them in Rust.
    transaction.execute(
        "INSERT INTO search_batch_candidates(batch_id, ordinal, item_id, item_json)              SELECT ?1,                     (ROW_NUMBER() OVER (ORDER BY s.item_id) - 1),                     s.item_id, s.item_json              FROM search_batch_staging_candidates AS s              WHERE s.request_id = ?2",
        params![batch_id, request_id],
    )?;
    crate::source_index::insert_batch_header_with_count(
        transaction,
        &header,
        candidate_count as u64,
        view_eligible,
    )?;
    if activate_workbench_slot {
        upsert_workbench_state(
            transaction,
            &StoreSearchBatchRequest {
                batch: header.clone(),
                workbench: workbench.clone(),
                activate_workbench: true,
            },
            revision,
            &state_json,
        )?;
    }
    if let Some((cache_key, updated_at_epoch)) = cache_pointer {
        let frozen_query: CatalogSearchQuery =
            serde_json::from_value(serde_json::Value::Object(header.query.clone()))
                .map_err(StoreError::Corrupt)?;
        let expected_key = catalog_search_cache_key(&frozen_query)
            .map_err(|error| StoreError::invalid(error.user_message))?;
        if cache_key != expected_key {
            return Err(StoreError::invalid(
                "catalogue cache key does not match the frozen typed query",
            ));
        }
        if updated_at_epoch < 0
            || cache_key.len() != 64
            || !cache_key
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(StoreError::invalid(
                "catalogue cache pointer is not canonical",
            ));
        }
        transaction.execute(
            "INSERT INTO sar_search_batch_cache(\
                 cache_key, source_batch_id, updated_at_epoch, complete\
             ) VALUES (?1, ?2, ?3, 1) \
             ON CONFLICT(cache_key) DO UPDATE SET \
                 source_batch_id = excluded.source_batch_id, \
                 updated_at_epoch = excluded.updated_at_epoch, \
                 complete = 1",
            params![cache_key, batch_id, updated_at_epoch],
        )?;
    }
    crate::batch_progress::record_sealed_state(
        transaction,
        request_id,
        terminal,
        header
            .search_stats
            .as_ref()
            .and_then(|stats| stats.matched_count),
    )?;
    transaction.execute(
        "DELETE FROM search_batch_staging WHERE request_id = ?1",
        [request_id],
    )?;
    crate::cache_maintenance::prune_disposable_search_batches_in_tx(
        transaction,
        Some(&batch_id),
        cache_pointer.and_then(|(_, epoch)| u64::try_from(epoch).ok()),
        crate::cache_maintenance::SEARCH_CACHE_DEFAULT_TTL_SECONDS,
        crate::cache_maintenance::SEARCH_CACHE_MAX_ENTRIES,
        crate::cache_maintenance::SEARCH_CACHE_MAX_BYTES,
        false,
    )?;
    Ok(())
}

#[cfg(test)]
mod streaming_batch_blob_tests {
    use super::*;

    /// #22 B slice 3(b) review fix 6: a serialized header that does not end
    /// with the empty-candidates suffix must fail closed, never panic.
    #[test]
    fn streaming_batch_blob_rejects_a_header_without_the_candidates_suffix() {
        let error =
            StreamingBatchBlob::from_serialized_header("{\"schema_version\":1,\"batch_id\":\"b\"}")
                .expect_err("a header without the candidates suffix must be rejected");
        assert!(matches!(error, StoreError::InvalidSnapshot { .. }));

        let blob = StreamingBatchBlob::from_serialized_header(
            "{\"schema_version\":1,\"batch_id\":\"b\",\"candidates\":[]}",
        )
        .expect("the canonical empty-candidates header is accepted");
        assert!(
            blob.buffer.ends_with("\"candidates\":["),
            "the blob must open the candidates array"
        );
    }
}
