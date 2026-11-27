use std::path::{Path, PathBuf};

use insar_core::{
    validate_frozen_task_snapshot_v3, DownloadLayout, FrozenOrbitAssociationV1, FrozenTaskSnapshot,
    FrozenTaskSnapshotV3, FrozenTaskSnapshotV3ValidationError, SnapshotItem,
    TaskOperationDescriptor,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{TaskSnapshotRowEnvelopeIssue, V3SnapshotValidationIssue};
use crate::StoreError;

pub const COMPACT_TASK_PERSISTENCE_VERSION: u64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompactTaskSnapshotV2 {
    pub persistence_version: u64,
    pub schema_version: u64,
    pub task_id: String,
    pub batch_id: String,
    pub provider_id: String,
    pub created_at: String,
    pub aoi_name: String,
    #[serde(default)]
    pub aoi_geojson: Option<Map<String, Value>>,
    pub query: Map<String, Value>,
    pub selected_products: Vec<SnapshotItem>,
    /// Source SearchBatch product IDs for task-scoped explicit items.
    ///
    /// Empty preserves the ordinary compact-task invariant: every selected
    /// product must equal the candidate frozen in the source SearchBatch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_item_ids: Vec<String>,
    pub initial_selected_item_ids: Vec<String>,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    #[serde(default)]
    pub operation: Option<TaskOperationDescriptor>,
    #[serde(default)]
    pub orbit_association: Option<FrozenOrbitAssociationV1>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StoredTaskSnapshot {
    LegacyV2(FrozenTaskSnapshot),
    CompactV2(CompactTaskSnapshotV2),
    V3(FrozenTaskSnapshotV3),
}

#[derive(Clone, Copy)]
pub struct TaskSnapshotRowEnvelope<'a> {
    pub task_id: &'a str,
    pub batch_id: &'a str,
    pub provider_id: &'a str,
    pub created_at: &'a str,
    pub output_root: &'a Path,
    pub layout: &'a DownloadLayout,
}

pub fn encode_task_snapshot(snapshot: &StoredTaskSnapshot) -> Result<String, StoreError> {
    match snapshot {
        StoredTaskSnapshot::LegacyV2(snapshot) => {
            validate_legacy_snapshot(snapshot)?;
            serde_json::to_string(snapshot)
                .map_err(|_| StoreError::invalid("task snapshot JSON cannot be encoded"))
        }
        StoredTaskSnapshot::CompactV2(snapshot) => {
            validate_compact_snapshot(snapshot)?;
            serde_json::to_string(snapshot)
                .map_err(|_| StoreError::invalid("task snapshot JSON cannot be encoded"))
        }
        StoredTaskSnapshot::V3(snapshot) => {
            validate_v3_snapshot(snapshot)?;
            serde_json::to_string(snapshot)
                .map_err(|_| StoreError::invalid_v3_snapshot(V3SnapshotValidationIssue::Structural))
        }
    }
}

/// Decodes every version known to the dormant Store codec boundary.
///
/// This version-aware entry owns its own safe error semantics. Existing V2
/// business readers must use decode_v2_only_snapshot instead so their
/// historical row-corruption classification remains intact.
pub fn decode_any_snapshot(snapshot_json: &str) -> Result<StoredTaskSnapshot, StoreError> {
    let value: Value = serde_json::from_str(snapshot_json)
        .map_err(|_| StoreError::invalid("task snapshot JSON is invalid"))?;
    let Some(persistence_version) = value.get("persistence_version") else {
        let snapshot: FrozenTaskSnapshot = serde_json::from_value(value)
            .map_err(|_| StoreError::invalid("legacy task snapshot is invalid"))?;
        validate_legacy_snapshot(&snapshot)?;
        return Ok(StoredTaskSnapshot::LegacyV2(snapshot));
    };

    match persistence_version.as_i64() {
        Some(2) => {
            let snapshot: CompactTaskSnapshotV2 = serde_json::from_value(value)
                .map_err(|_| StoreError::invalid("compact V2 task snapshot is invalid"))?;
            validate_compact_snapshot(&snapshot)?;
            Ok(StoredTaskSnapshot::CompactV2(snapshot))
        }
        Some(3) => decode_v3_snapshot(value),
        _ => Err(StoreError::unsupported_snapshot_version()),
    }
}

/// Decodes the V2 formats used by existing business readers.
///
/// This compatibility boundary deliberately preserves the pre-B2 distinction
/// between corrupt JSON or serde shapes and business-invalid V2 snapshots.
/// Valid V3 remains fail-closed after its structural and graph checks pass.
pub fn decode_v2_only_snapshot(snapshot_json: &str) -> Result<StoredTaskSnapshot, StoreError> {
    let value: Value = serde_json::from_str(snapshot_json).map_err(StoreError::Corrupt)?;
    let Some(persistence_version) = value.get("persistence_version") else {
        let snapshot: FrozenTaskSnapshot =
            serde_json::from_value(value).map_err(StoreError::Corrupt)?;
        validate_legacy_snapshot(&snapshot)?;
        return Ok(StoredTaskSnapshot::LegacyV2(snapshot));
    };

    if persistence_version.as_i64() == Some(3) {
        let snapshot: FrozenTaskSnapshotV3 = serde_json::from_value(value)
            .map_err(|_| StoreError::invalid_v3_snapshot(V3SnapshotValidationIssue::Structural))?;
        validate_v3_snapshot(&snapshot)?;
        return Err(StoreError::v3_snapshot_rejected_by_v2_reader());
    }

    let snapshot: CompactTaskSnapshotV2 =
        serde_json::from_value(value).map_err(StoreError::Corrupt)?;
    if snapshot.persistence_version != COMPACT_TASK_PERSISTENCE_VERSION {
        return Err(StoreError::invalid(
            "unsupported compact task persistence version",
        ));
    }
    validate_compact_snapshot(&snapshot)?;
    Ok(StoredTaskSnapshot::CompactV2(snapshot))
}

pub fn validate_task_snapshot_row_envelope(
    envelope: TaskSnapshotRowEnvelope<'_>,
    snapshot: &StoredTaskSnapshot,
) -> Result<(), StoreError> {
    match snapshot {
        StoredTaskSnapshot::LegacyV2(snapshot) => {
            validate_task_snapshot_row_envelope_values(envelope, snapshot)
        }
        StoredTaskSnapshot::CompactV2(snapshot) => {
            validate_task_snapshot_row_envelope_values(envelope, snapshot)
        }
        StoredTaskSnapshot::V3(snapshot) => {
            validate_task_snapshot_row_envelope_values(envelope, snapshot)
        }
    }
}

fn validate_task_snapshot_row_envelope_values(
    envelope: TaskSnapshotRowEnvelope<'_>,
    snapshot: &impl SnapshotRowFields,
) -> Result<(), StoreError> {
    if envelope.task_id != snapshot.task_id() {
        return Err(StoreError::snapshot_row_envelope_mismatch(
            TaskSnapshotRowEnvelopeIssue::TaskId,
        ));
    }
    if envelope.batch_id != snapshot.batch_id() {
        return Err(StoreError::snapshot_row_envelope_mismatch(
            TaskSnapshotRowEnvelopeIssue::BatchId,
        ));
    }
    if envelope.provider_id != snapshot.provider_id() {
        return Err(StoreError::snapshot_row_envelope_mismatch(
            TaskSnapshotRowEnvelopeIssue::ProviderId,
        ));
    }
    if envelope.created_at != snapshot.created_at() {
        return Err(StoreError::snapshot_row_envelope_mismatch(
            TaskSnapshotRowEnvelopeIssue::CreatedAt,
        ));
    }
    if envelope.output_root != snapshot.output_root() {
        return Err(StoreError::snapshot_row_envelope_mismatch(
            TaskSnapshotRowEnvelopeIssue::OutputRoot,
        ));
    }
    if envelope.layout != snapshot.layout() {
        return Err(StoreError::snapshot_row_envelope_mismatch(
            TaskSnapshotRowEnvelopeIssue::Layout,
        ));
    }
    Ok(())
}

trait SnapshotRowFields {
    fn task_id(&self) -> &str;
    fn batch_id(&self) -> &str;
    fn provider_id(&self) -> &str;
    fn created_at(&self) -> &str;
    fn output_root(&self) -> &Path;
    fn layout(&self) -> &DownloadLayout;
}

macro_rules! impl_snapshot_row_fields {
    ($type:ty) => {
        impl SnapshotRowFields for $type {
            fn task_id(&self) -> &str {
                &self.task_id
            }

            fn batch_id(&self) -> &str {
                &self.batch_id
            }

            fn provider_id(&self) -> &str {
                &self.provider_id
            }

            fn created_at(&self) -> &str {
                &self.created_at
            }

            fn output_root(&self) -> &Path {
                &self.output_root
            }

            fn layout(&self) -> &DownloadLayout {
                &self.layout
            }
        }
    };
}

impl_snapshot_row_fields!(FrozenTaskSnapshot);
impl_snapshot_row_fields!(CompactTaskSnapshotV2);
impl_snapshot_row_fields!(FrozenTaskSnapshotV3);

fn decode_v3_snapshot(value: Value) -> Result<StoredTaskSnapshot, StoreError> {
    let snapshot: FrozenTaskSnapshotV3 = serde_json::from_value(value)
        .map_err(|_| StoreError::invalid_v3_snapshot(V3SnapshotValidationIssue::Structural))?;
    validate_v3_snapshot(&snapshot)?;
    Ok(StoredTaskSnapshot::V3(snapshot))
}

fn validate_legacy_snapshot(snapshot: &FrozenTaskSnapshot) -> Result<(), StoreError> {
    snapshot
        .validate()
        .map_err(|_| StoreError::invalid("legacy task snapshot is invalid"))
}

fn validate_compact_snapshot(snapshot: &CompactTaskSnapshotV2) -> Result<(), StoreError> {
    if snapshot.persistence_version != COMPACT_TASK_PERSISTENCE_VERSION {
        return Err(StoreError::unsupported_snapshot_version());
    }
    if !snapshot.source_item_ids.is_empty() {
        let source_ids = snapshot
            .source_item_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        if source_ids.len() != snapshot.source_item_ids.len() {
            return Err(StoreError::invalid(
                "explicit task source item IDs must be unique",
            ));
        }
        let mut referenced = std::collections::HashSet::new();
        for item in &snapshot.selected_products {
            let source_item_id = item
                .metadata
                .get("source_item_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    StoreError::invalid("explicit task item lacks its source item ID")
                })?;
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
    }
    for item in &snapshot.selected_products {
        crate::nisar_subset_plan::validate_nisar_subset_plan_v2_metadata(&item.metadata)?;
    }
    let hydrated_shape = FrozenTaskSnapshot {
        schema_version: snapshot.schema_version,
        task_id: snapshot.task_id.clone(),
        batch_id: snapshot.batch_id.clone(),
        provider_id: snapshot.provider_id.clone(),
        created_at: snapshot.created_at.clone(),
        aoi_name: snapshot.aoi_name.clone(),
        aoi_geojson: snapshot.aoi_geojson.clone(),
        query: snapshot.query.clone(),
        candidates: snapshot.selected_products.clone(),
        initial_selected_item_ids: snapshot.initial_selected_item_ids.clone(),
        output_root: snapshot.output_root.clone(),
        layout: snapshot.layout.clone(),
        operation: snapshot.operation.clone(),
        orbit_association: snapshot.orbit_association.clone(),
    };
    validate_legacy_snapshot(&hydrated_shape)
}

fn validate_v3_snapshot(snapshot: &FrozenTaskSnapshotV3) -> Result<(), StoreError> {
    validate_frozen_task_snapshot_v3(snapshot).map_err(|error| match error {
        FrozenTaskSnapshotV3ValidationError::Structural => {
            StoreError::invalid_v3_snapshot(V3SnapshotValidationIssue::Structural)
        }
        FrozenTaskSnapshotV3ValidationError::Graph(_) => {
            StoreError::invalid_v3_snapshot(V3SnapshotValidationIssue::Graph)
        }
    })
}
