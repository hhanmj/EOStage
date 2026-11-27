use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::json;

use crate::StoreError;

pub const STORE_SCHEMA_VERSION: i64 = 43;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA_V1: &str = r#"
CREATE TABLE search_batches (
    batch_id TEXT PRIMARY KEY NOT NULL,
    provider_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    snapshot_json TEXT NOT NULL CHECK(length(snapshot_json) > 1)
) STRICT;

CREATE TABLE workbench_state (
    slot INTEGER PRIMARY KEY NOT NULL CHECK(slot = 1),
    batch_id TEXT NOT NULL UNIQUE REFERENCES search_batches(batch_id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    updated_at TEXT NOT NULL,
    state_json TEXT NOT NULL CHECK(length(state_json) > 1)
) STRICT;

CREATE TABLE task_snapshots (
    task_id TEXT PRIMARY KEY NOT NULL,
    batch_id TEXT NOT NULL REFERENCES search_batches(batch_id) ON DELETE RESTRICT,
    provider_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    output_root TEXT NOT NULL,
    layout TEXT NOT NULL,
    snapshot_json TEXT NOT NULL CHECK(length(snapshot_json) > 1)
) STRICT;

CREATE TABLE task_runtime (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    updated_at TEXT NOT NULL,
    state_json TEXT NOT NULL CHECK(length(state_json) > 1)
) STRICT;

CREATE TABLE task_events (
    event_id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK(sequence >= 0),
    occurred_at TEXT NOT NULL,
    event_type TEXT NOT NULL,
    item_id TEXT,
    payload_json TEXT NOT NULL CHECK(length(payload_json) > 1),
    error_json TEXT,
    UNIQUE(task_id, sequence)
) STRICT;

CREATE TABLE task_history (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    archived_at TEXT NOT NULL,
    layout TEXT NOT NULL,
    record_json TEXT NOT NULL CHECK(length(record_json) > 1)
) STRICT;

CREATE TABLE task_tombstones (
    task_id TEXT PRIMARY KEY NOT NULL,
    deleted_at TEXT NOT NULL,
    identity_json TEXT NOT NULL CHECK(length(identity_json) > 1)
) STRICT;

CREATE INDEX task_runtime_status_idx ON task_runtime(status);
CREATE INDEX task_events_task_sequence_idx ON task_events(task_id, sequence);
CREATE INDEX task_history_archived_idx ON task_history(archived_at);

CREATE TRIGGER search_batches_are_frozen
BEFORE UPDATE ON search_batches
BEGIN
    SELECT RAISE(ABORT, 'frozen search batch cannot be updated');
END;

CREATE TRIGGER task_snapshots_are_frozen
BEFORE UPDATE ON task_snapshots
BEGIN
    SELECT RAISE(ABORT, 'frozen task snapshot cannot be updated');
END;
"#;

const SCHEMA_V2: &str = r#"
CREATE TABLE download_engine_state (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE download_progress (
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    status TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    progress_json TEXT NOT NULL CHECK(length(progress_json) > 1),
    PRIMARY KEY(task_id, item_id)
) STRICT;

CREATE TABLE download_results (
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    outcome TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    result_json TEXT NOT NULL CHECK(length(result_json) > 1),
    PRIMARY KEY(task_id, item_id)
) STRICT;

CREATE INDEX download_progress_status_idx ON download_progress(status);
CREATE INDEX download_results_outcome_idx ON download_results(outcome);
"#;

const SCHEMA_V3: &str = r#"
CREATE TABLE asf_search_cache (
    cache_key TEXT PRIMARY KEY NOT NULL,
    created_at_epoch INTEGER NOT NULL CHECK(created_at_epoch >= 0),
    updated_at_epoch INTEGER NOT NULL CHECK(updated_at_epoch >= created_at_epoch),
    entry_json TEXT NOT NULL CHECK(length(entry_json) > 1)
) STRICT;

CREATE INDEX asf_search_cache_updated_idx ON asf_search_cache(updated_at_epoch);
"#;

const SCHEMA_V4: &str = r#"
CREATE TABLE task_view_state (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    checked_item_ids_json TEXT NOT NULL CHECK(length(checked_item_ids_json) > 1),
    page_index INTEGER NOT NULL CHECK(page_index >= 0),
    page_size INTEGER NOT NULL CHECK(page_size >= 1),
    updated_at TEXT NOT NULL
) STRICT;
"#;

const SCHEMA_V5: &str = r#"
CREATE TABLE region_aoi_state (
    slot INTEGER PRIMARY KEY NOT NULL CHECK(slot = 1),
    region_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    source_kind TEXT NOT NULL,
    geometry_json TEXT NOT NULL CHECK(length(geometry_json) > 1),
    bbox_json TEXT NOT NULL CHECK(length(bbox_json) > 1),
    crs TEXT NOT NULL,
    display_name TEXT NOT NULL,
    bound_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
"#;

const SCHEMA_V6: &str = r#"
ALTER TABLE region_aoi_state RENAME TO region_aoi_state_v5;

CREATE TABLE region_aoi_state (
    workspace_id TEXT NOT NULL,
    region_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    source_kind TEXT NOT NULL,
    geometry_json TEXT NOT NULL CHECK(length(geometry_json) > 1),
    bbox_json TEXT NOT NULL CHECK(length(bbox_json) > 1),
    crs TEXT NOT NULL,
    display_name TEXT NOT NULL,
    bound_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(workspace_id, region_id)
) STRICT;

INSERT INTO region_aoi_state (
    workspace_id, region_id, revision, source_kind, geometry_json, bbox_json,
    crs, display_name, bound_at, updated_at
)
SELECT
    'workspace-default', region_id, revision, source_kind, geometry_json, bbox_json,
    crs, display_name, bound_at, updated_at
FROM region_aoi_state_v5;

DROP TABLE region_aoi_state_v5;

CREATE TABLE workspace_context (
    slot INTEGER PRIMARY KEY NOT NULL CHECK(slot = 1),
    context_json TEXT NOT NULL CHECK(length(context_json) > 1)
) STRICT;
"#;

const SCHEMA_V7: &str = r#"
ALTER TABLE download_results ADD COLUMN last_known_total_bytes INTEGER
    CHECK(last_known_total_bytes IS NULL OR last_known_total_bytes >= 0);
"#;

const SCHEMA_V8: &str = r#"
ALTER TABLE download_progress ADD COLUMN bytes_completed INTEGER NOT NULL DEFAULT 0 CHECK(bytes_completed >= 0);
ALTER TABLE download_progress ADD COLUMN total_bytes INTEGER CHECK(total_bytes IS NULL OR total_bytes >= 0);
ALTER TABLE download_progress ADD COLUMN bytes_per_second REAL NOT NULL DEFAULT 0 CHECK(bytes_per_second >= 0);
ALTER TABLE download_progress ADD COLUMN last_known_total_bytes INTEGER CHECK(last_known_total_bytes IS NULL OR last_known_total_bytes >= 0);
ALTER TABLE download_results ADD COLUMN bytes_written INTEGER NOT NULL DEFAULT 0 CHECK(bytes_written >= 0);

UPDATE download_progress
SET bytes_completed = CAST(json_extract(progress_json, '$.bytes_completed') AS INTEGER),
    total_bytes = CASE WHEN json_type(progress_json, '$.total_bytes') = 'integer'
        THEN CAST(json_extract(progress_json, '$.total_bytes') AS INTEGER) ELSE NULL END,
    bytes_per_second = CAST(json_extract(progress_json, '$.bytes_per_second') AS REAL),
    last_known_total_bytes = CASE WHEN json_type(progress_json, '$.total_bytes') = 'integer'
        THEN CAST(json_extract(progress_json, '$.total_bytes') AS INTEGER) ELSE NULL END;
UPDATE download_results
SET bytes_written = CAST(json_extract(result_json, '$.bytes_written') AS INTEGER);
"#;

const SCHEMA_V9: &str = r#"
CREATE TABLE task_history_summary (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES task_history(task_id) ON DELETE CASCADE
        CHECK(length(task_id) BETWEEN 1 AND 128),
    archived_at TEXT NOT NULL CHECK(length(archived_at) BETWEEN 20 AND 24),
    archived_at_epoch_ms INTEGER NOT NULL,
    batch_id TEXT NOT NULL CHECK(length(batch_id) BETWEEN 1 AND 128),
    provider_id TEXT NOT NULL CHECK(length(provider_id) BETWEEN 1 AND 128),
    created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 20 AND 30),
    output_root TEXT NOT NULL CHECK(length(output_root) BETWEEN 1 AND 4096),
    layout TEXT NOT NULL CHECK(layout IN ('direct', 'sentinel1_slc', 'sentinel_orbit_aux_poeorb', 'sar_dataset_processing')),
    status TEXT NOT NULL CHECK(status IN ('completed', 'failed', 'cancelled', 'interrupted')),
    revision INTEGER NOT NULL CHECK(revision >= 0),
    updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 20 AND 30),
    selected_count INTEGER NOT NULL CHECK(selected_count >= 1),
    checked_count INTEGER NOT NULL CHECK(checked_count >= 0 AND checked_count <= selected_count),
    active_count INTEGER NOT NULL CHECK(active_count >= 0),
    paused_count INTEGER NOT NULL CHECK(paused_count >= 0),
    queued_count INTEGER NOT NULL CHECK(queued_count >= 0),
    completed_count INTEGER NOT NULL CHECK(completed_count >= 0),
    failed_count INTEGER NOT NULL CHECK(failed_count >= 0),
    skipped_count INTEGER NOT NULL CHECK(skipped_count >= 0),
    cancelled_count INTEGER NOT NULL CHECK(cancelled_count >= 0),
    operation_id TEXT CHECK(operation_id IS NULL OR length(operation_id) BETWEEN 1 AND 128),
    operation_kind TEXT CHECK(operation_kind IS NULL OR length(operation_kind) BETWEEN 1 AND 128),
    operation_phase TEXT CHECK(operation_phase IS NULL OR length(operation_phase) BETWEEN 1 AND 128),
    operation_region_id TEXT CHECK(operation_region_id IS NULL OR length(operation_region_id) BETWEEN 1 AND 128),
    approved_worker_operation TEXT CHECK(approved_worker_operation IS NULL OR length(approved_worker_operation) BETWEEN 1 AND 128),
    CHECK(
        (operation_id IS NULL AND operation_kind IS NULL AND operation_phase IS NULL AND operation_region_id IS NULL AND approved_worker_operation IS NULL)
        OR
        (operation_id IS NOT NULL AND operation_kind IS NOT NULL AND operation_phase IS NOT NULL AND operation_region_id IS NOT NULL AND approved_worker_operation IS NOT NULL)
    ),
    CHECK(active_count + paused_count + queued_count + completed_count + failed_count + skipped_count + cancelled_count = selected_count)
) STRICT;

CREATE INDEX task_history_summary_keyset_idx
    ON task_history_summary(archived_at_epoch_ms DESC, task_id ASC);
"#;

const SCHEMA_V10: &str = r#"
CREATE TABLE task_history_dem_source_authority (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES task_history_summary(task_id) ON DELETE CASCADE
        CHECK(length(task_id) BETWEEN 1 AND 128),
    region_id TEXT NOT NULL CHECK(length(region_id) BETWEEN 1 AND 128),
    dataset TEXT NOT NULL CHECK(length(dataset) BETWEEN 1 AND 128),
    provider TEXT NOT NULL CHECK(length(provider) BETWEEN 1 AND 128),
    resource_id TEXT NOT NULL CHECK(length(resource_id) BETWEEN 1 AND 128),
    capability_id TEXT NOT NULL CHECK(length(capability_id) BETWEEN 1 AND 128),
    input_path TEXT NOT NULL CHECK(length(input_path) BETWEEN 1 AND 4096),
    output_root TEXT NOT NULL CHECK(length(output_root) BETWEEN 1 AND 4096)
) STRICT;

CREATE INDEX task_history_dem_source_identity_idx
    ON task_history_dem_source_authority(
        region_id, dataset, provider, resource_id, capability_id, task_id
    );
"#;

const SCHEMA_V11: &str = r#"
CREATE TABLE dem_source_identity (
    slot INTEGER PRIMARY KEY NOT NULL CHECK(slot = 1),
    dataset TEXT NOT NULL CHECK(length(dataset) BETWEEN 1 AND 128),
    provider TEXT NOT NULL CHECK(length(provider) BETWEEN 1 AND 128),
    updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 20 AND 24),
    CHECK(dataset = upper(dataset) AND trim(dataset) = dataset),
    CHECK(provider = upper(provider) AND trim(provider) = provider)
) STRICT;
"#;

const SCHEMA_V12: &str = r#"
CREATE TABLE standalone_aoi_state (
    slot INTEGER PRIMARY KEY NOT NULL CHECK(slot = 1),
    revision INTEGER NOT NULL CHECK(revision >= 0),
    source_kind TEXT NOT NULL CHECK(source_kind IN (
        'manual_bbox', 'manual_polygon', 'uploaded_geometry', 'administrative_boundary'
    )),
    geometry_json TEXT NOT NULL CHECK(length(geometry_json) BETWEEN 2 AND 262144),
    west REAL NOT NULL CHECK(west >= -180.0 AND west < 180.0),
    south REAL NOT NULL CHECK(south >= -90.0 AND south < 90.0),
    east REAL NOT NULL CHECK(east > -180.0 AND east <= 180.0 AND east > west),
    north REAL NOT NULL CHECK(north > -90.0 AND north <= 90.0 AND north > south),
    crs TEXT NOT NULL CHECK(crs = 'EPSG:4326'),
    display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 2048),
    updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 20 AND 24)
) STRICT;
"#;

const SCHEMA_V13: &str = r#"
ALTER TABLE standalone_aoi_state RENAME TO standalone_aoi_state_v12;

CREATE TABLE standalone_aoi_state (
    slot INTEGER PRIMARY KEY NOT NULL CHECK(slot = 1),
    revision INTEGER NOT NULL CHECK(revision >= 0),
    source_kind TEXT NOT NULL CHECK(source_kind IN (
        'manual_bbox', 'manual_polygon', 'uploaded_geometry', 'administrative_boundary'
    )),
    geometry_json TEXT NOT NULL CHECK(length(geometry_json) BETWEEN 2 AND 8388608),
    west REAL NOT NULL CHECK(west >= -180.0 AND west < 180.0),
    south REAL NOT NULL CHECK(south >= -90.0 AND south < 90.0),
    east REAL NOT NULL CHECK(east > -180.0 AND east <= 180.0 AND east > west),
    north REAL NOT NULL CHECK(north > -90.0 AND north <= 90.0 AND north > south),
    crs TEXT NOT NULL CHECK(crs = 'EPSG:4326'),
    display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 2048),
    updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 20 AND 24)
) STRICT;

INSERT INTO standalone_aoi_state(
    slot, revision, source_kind, geometry_json, west, south, east, north, crs, display_name, updated_at
)
SELECT
    slot, revision, source_kind, geometry_json, west, south, east, north, crs, display_name, updated_at
FROM standalone_aoi_state_v12;

DROP TABLE standalone_aoi_state_v12;
"#;

const SCHEMA_V14: &str = r#"
CREATE TABLE sar_search_batch_cache (
    cache_key TEXT PRIMARY KEY NOT NULL
        CHECK(length(cache_key) = 64 AND cache_key = lower(cache_key)),
    source_batch_id TEXT NOT NULL REFERENCES search_batches(batch_id) ON DELETE CASCADE
        CHECK(length(source_batch_id) BETWEEN 1 AND 128),
    updated_at_epoch INTEGER NOT NULL CHECK(updated_at_epoch >= 0),
    complete INTEGER NOT NULL CHECK(complete IN (0, 1))
) STRICT;

CREATE INDEX sar_search_batch_cache_updated_idx
    ON sar_search_batch_cache(updated_at_epoch DESC, cache_key ASC);
"#;

const SCHEMA_V15_TELEMETRY: &str = r#"
CREATE TABLE task_transfer_telemetry (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE
        CHECK(length(task_id) BETWEEN 1 AND 128),
    transfer_active_millis INTEGER NOT NULL DEFAULT 0 CHECK(transfer_active_millis >= 0),
    updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 20 AND 30)
) STRICT;
"#;

const SCHEMA_V15_HISTORY: &str = r#"
ALTER TABLE task_history_summary
    ADD COLUMN transfer_active_millis INTEGER NOT NULL DEFAULT 0
    CHECK(transfer_active_millis >= 0);
"#;

const SCHEMA_V16_HISTORY_DATASET: &str = r#"
ALTER TABLE task_history_summary
    ADD COLUMN dataset_id TEXT
    CHECK(dataset_id IS NULL OR (
        length(dataset_id) BETWEEN 1 AND 128
        AND trim(dataset_id) = dataset_id
    ));
"#;

const SCHEMA_V17_ADMINISTRATIVE_BOUNDARY_CACHE: &str = r#"
CREATE TABLE administrative_boundary_cache (
    provider TEXT NOT NULL CHECK(provider IN ('datav', 'nominatim')),
    provider_object_id TEXT NOT NULL CHECK(length(provider_object_id) BETWEEN 1 AND 128),
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 2048),
    display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 2048),
    country_code TEXT CHECK(country_code IS NULL OR (
        length(country_code) = 2 AND country_code = upper(country_code)
    )),
    admin_level TEXT CHECK(admin_level IS NULL OR length(admin_level) BETWEEN 1 AND 128),
    geometry_json TEXT NOT NULL CHECK(length(geometry_json) BETWEEN 2 AND 8388608),
    west REAL NOT NULL CHECK(west >= -180.0 AND west < 180.0),
    south REAL NOT NULL CHECK(south >= -90.0 AND south < 90.0),
    east REAL NOT NULL CHECK(east > -180.0 AND east <= 180.0 AND east > west),
    north REAL NOT NULL CHECK(north > -90.0 AND north <= 90.0 AND north > south),
    source TEXT NOT NULL CHECK(length(source) BETWEEN 1 AND 2048),
    attribution TEXT CHECK(attribution IS NULL OR length(attribution) BETWEEN 1 AND 2048),
    cached_at TEXT NOT NULL CHECK(length(cached_at) BETWEEN 20 AND 24),
    PRIMARY KEY(provider, provider_object_id),
    CHECK(
        (provider = 'datav' AND length(provider_object_id) = 6 AND provider_object_id NOT GLOB '*[^0-9]*')
        OR
        (provider = 'nominatim' AND (
            (provider_object_id GLOB 'node:[1-9]*' AND provider_object_id NOT GLOB 'node:*[^0-9]*')
            OR (provider_object_id GLOB 'way:[1-9]*' AND provider_object_id NOT GLOB 'way:*[^0-9]*')
            OR (provider_object_id GLOB 'relation:[1-9]*' AND provider_object_id NOT GLOB 'relation:*[^0-9]*')
        ))
    )
) STRICT;

CREATE INDEX administrative_boundary_cache_cached_at_idx
    ON administrative_boundary_cache(cached_at DESC, provider, provider_object_id);
"#;

const SCHEMA_V17_ADMINISTRATIVE_BOUNDARY_CACHE_INDEX: &str = r#"
CREATE INDEX administrative_boundary_cache_cached_at_idx
    ON administrative_boundary_cache(cached_at DESC, provider, provider_object_id);
"#;

const SCHEMA_V18_SAR_BACKGROUND_COUNT: &str = r#"
CREATE TABLE sar_search_batch_count (
    batch_id TEXT PRIMARY KEY NOT NULL REFERENCES search_batches(batch_id) ON DELETE CASCADE,
    query_fingerprint TEXT NOT NULL
        CHECK(length(query_fingerprint) = 64 AND query_fingerprint = lower(query_fingerprint)),
    observed_workbench_revision INTEGER NOT NULL CHECK(observed_workbench_revision >= 0),
    matched_count INTEGER NOT NULL CHECK(matched_count >= 0),
    updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 20 AND 30)
) STRICT;
"#;

const SCHEMA_V19_TASK_MEMBERSHIP_ADDITIONS: &str = r#"
CREATE TABLE task_membership_additions (
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE
        CHECK(length(task_id) BETWEEN 1 AND 128),
    item_id TEXT NOT NULL CHECK(length(item_id) BETWEEN 1 AND 2048),
    added_revision INTEGER NOT NULL CHECK(added_revision >= 1),
    position INTEGER NOT NULL CHECK(position >= 0),
    added_at TEXT NOT NULL CHECK(length(added_at) BETWEEN 20 AND 30),
    item_json TEXT NOT NULL CHECK(length(item_json) > 1 AND json_valid(item_json)),
    PRIMARY KEY(task_id, item_id),
    UNIQUE(task_id, added_revision, position)
) STRICT;

CREATE INDEX task_membership_additions_order_idx
    ON task_membership_additions(task_id, added_revision, position);

CREATE TRIGGER task_membership_additions_are_frozen
BEFORE UPDATE ON task_membership_additions
BEGIN
    SELECT RAISE(ABORT, 'task membership addition cannot be updated');
END;
"#;

const SCHEMA_V20_SEARCH_BATCH_STAGING: &str = r#"
CREATE TABLE search_batch_staging (
    request_id TEXT PRIMARY KEY NOT NULL
        CHECK(length(request_id) BETWEEN 1 AND 128 AND trim(request_id) = request_id),
    batch_id TEXT NOT NULL UNIQUE
        CHECK(length(batch_id) BETWEEN 1 AND 128 AND trim(batch_id) = batch_id),
    max_candidates INTEGER NOT NULL CHECK(max_candidates >= 1),
    batch_header_json TEXT NOT NULL
        CHECK(length(batch_header_json) > 1 AND json_valid(batch_header_json)),
    workbench_json TEXT NOT NULL
        CHECK(length(workbench_json) > 1 AND json_valid(workbench_json))
) STRICT;

CREATE TABLE search_batch_staging_candidates (
    request_id TEXT NOT NULL REFERENCES search_batch_staging(request_id) ON DELETE CASCADE,
    item_id TEXT NOT NULL CHECK(length(item_id) BETWEEN 1 AND 2048),
    item_json TEXT NOT NULL CHECK(length(item_json) > 1 AND json_valid(item_json)),
    PRIMARY KEY(request_id, item_id)
) STRICT;

CREATE INDEX search_batch_staging_candidates_order_idx
    ON search_batch_staging_candidates(request_id, item_id);
"#;

const SCHEMA_V21_NISAR_EXECUTION_PLAN_FREEZE: &str = r#"
DROP TRIGGER IF EXISTS task_snapshots_are_frozen;

CREATE TRIGGER task_snapshots_are_frozen
BEFORE UPDATE ON task_snapshots
WHEN NOT (
    OLD.task_id = NEW.task_id
    AND OLD.batch_id = NEW.batch_id
    AND OLD.provider_id = NEW.provider_id
    AND OLD.created_at = NEW.created_at
    AND OLD.output_root = NEW.output_root
    AND OLD.layout = NEW.layout
    AND json_valid(OLD.snapshot_json)
    AND json_valid(NEW.snapshot_json)
    AND json_extract(OLD.snapshot_json, '$.persistence_version') = 2
    AND json_extract(NEW.snapshot_json, '$.persistence_version') = 2
    AND json_array_length(json_extract(OLD.snapshot_json, '$.selected_products')) = 1
    AND json_array_length(json_extract(NEW.snapshot_json, '$.selected_products')) = 1
    AND json_type(
        OLD.snapshot_json,
        '$.selected_products[0].metadata.nisar_subset_prepared_intent_v2_envelope'
    ) = 'object'
    AND json_type(
        OLD.snapshot_json,
        '$.selected_products[0].metadata.nisar_subset_plan_v2_envelope'
    ) IS NULL
    AND json_type(
        NEW.snapshot_json,
        '$.selected_products[0].metadata.nisar_subset_plan_v2_envelope'
    ) = 'object'
    AND json_remove(
        OLD.snapshot_json,
        '$.selected_products[0].metadata.nisar_subset_plan_v2_envelope'
    ) = json_remove(
        NEW.snapshot_json,
        '$.selected_products[0].metadata.nisar_subset_plan_v2_envelope'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'frozen task snapshot cannot be updated');
END;
"#;

const SCHEMA_V22_NISAR_EXECUTION_ANCHORS: &str = r#"
CREATE TABLE nisar_subset_execution_anchors_v2 (
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE
        CHECK(length(task_id) BETWEEN 1 AND 128 AND trim(task_id) = task_id),
    item_id TEXT NOT NULL
        CHECK(length(item_id) BETWEEN 1 AND 512 AND trim(item_id) = item_id),
    anchor_kind TEXT NOT NULL CHECK(anchor_kind IN (
        'science_checkpoint_v2',
        'writer_recovery_v2'
    )),
    lineage_key_sha256 TEXT NOT NULL CHECK(length(lineage_key_sha256) = 64),
    record_sha256 TEXT NOT NULL CHECK(length(record_sha256) = 64),
    canonical_json TEXT NOT NULL CHECK(
        length(canonical_json) BETWEEN 2 AND 41943040 AND json_valid(canonical_json)
    ),
    PRIMARY KEY(task_id, item_id, anchor_kind)
) STRICT;
"#;

const SCHEMA_V23_NISAR_MULTI_ITEM_EXECUTION_PLAN_FREEZE: &str = r#"
DROP TRIGGER IF EXISTS task_snapshots_are_frozen;

CREATE TRIGGER task_snapshots_are_frozen
BEFORE UPDATE ON task_snapshots
WHEN NOT (
    OLD.task_id = NEW.task_id
    AND OLD.batch_id = NEW.batch_id
    AND OLD.provider_id = NEW.provider_id
    AND OLD.created_at = NEW.created_at
    AND OLD.output_root = NEW.output_root
    AND OLD.layout = NEW.layout
    AND json_valid(OLD.snapshot_json)
    AND json_valid(NEW.snapshot_json)
    AND json_extract(OLD.snapshot_json, '$.persistence_version') = 2
    AND json_extract(NEW.snapshot_json, '$.persistence_version') = 2
    AND json_array_length(json_extract(OLD.snapshot_json, '$.selected_products')) =
        json_array_length(json_extract(NEW.snapshot_json, '$.selected_products'))
    AND EXISTS (
        SELECT 1
        FROM json_each(OLD.snapshot_json, '$.selected_products') AS old_item
        JOIN json_each(NEW.snapshot_json, '$.selected_products') AS new_item
            ON old_item.key = new_item.key
        WHERE json_type(
            old_item.value,
            '$.metadata.nisar_subset_prepared_intent_v2_envelope'
        ) = 'object'
        AND json_type(
            old_item.value,
            '$.metadata.nisar_subset_plan_v2_envelope'
        ) IS NULL
        AND json_type(
            new_item.value,
            '$.metadata.nisar_subset_plan_v2_envelope'
        ) = 'object'
        AND json_remove(
            OLD.snapshot_json,
            '$.selected_products[' || old_item.key || '].metadata.nisar_subset_plan_v2_envelope'
        ) = json_remove(
            NEW.snapshot_json,
            '$.selected_products[' || old_item.key || '].metadata.nisar_subset_plan_v2_envelope'
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'frozen task snapshot cannot be updated');
END;
"#;

const SCHEMA_V28_NISAR_SOURCE_SIZE_FREEZE: &str = r#"
DROP TRIGGER IF EXISTS task_snapshots_are_frozen;

CREATE TRIGGER task_snapshots_are_frozen
BEFORE UPDATE ON task_snapshots
WHEN NOT (
    OLD.task_id = NEW.task_id
    AND OLD.batch_id = NEW.batch_id
    AND OLD.provider_id = NEW.provider_id
    AND OLD.created_at = NEW.created_at
    AND OLD.output_root = NEW.output_root
    AND OLD.layout = NEW.layout
    AND json_valid(OLD.snapshot_json)
    AND json_valid(NEW.snapshot_json)
    AND json_extract(OLD.snapshot_json, '$.persistence_version') = 2
    AND json_extract(NEW.snapshot_json, '$.persistence_version') = 2
    AND json_array_length(json_extract(OLD.snapshot_json, '$.selected_products')) =
        json_array_length(json_extract(NEW.snapshot_json, '$.selected_products'))
    AND (
        EXISTS (
            SELECT 1
            FROM json_each(OLD.snapshot_json, '$.selected_products') AS old_item
            JOIN json_each(NEW.snapshot_json, '$.selected_products') AS new_item
                ON old_item.key = new_item.key
            WHERE json_type(
                old_item.value,
                '$.metadata.nisar_subset_prepared_intent_v2_envelope'
            ) = 'object'
            AND json_type(
                old_item.value,
                '$.metadata.nisar_subset_plan_v2_envelope'
            ) IS NULL
            AND json_type(
                new_item.value,
                '$.metadata.nisar_subset_plan_v2_envelope'
            ) = 'object'
            AND json_remove(
                OLD.snapshot_json,
                '$.selected_products[' || old_item.key || '].metadata.nisar_subset_plan_v2_envelope'
            ) = json_remove(
                NEW.snapshot_json,
                '$.selected_products[' || old_item.key || '].metadata.nisar_subset_plan_v2_envelope'
            )
        )
        OR EXISTS (
            SELECT 1
            FROM json_each(OLD.snapshot_json, '$.selected_products') AS old_item
            JOIN json_each(NEW.snapshot_json, '$.selected_products') AS new_item
                ON old_item.key = new_item.key
            WHERE json_type(
                old_item.value,
                '$.metadata.nisar_subset_prepared_intent_v2_envelope'
            ) = 'object'
            AND json_type(old_item.value, '$.asset_access') = 'object'
            AND json_type(new_item.value, '$.asset_access') = 'object'
            AND COALESCE(
                json_type(old_item.value, '$.asset_access.expected_size_bytes'),
                'null'
            ) = 'null'
            AND json_type(
                new_item.value,
                '$.asset_access.expected_size_bytes'
            ) = 'integer'
            AND json_extract(
                new_item.value,
                '$.asset_access.expected_size_bytes'
            ) > 0
            AND json_remove(
                OLD.snapshot_json,
                '$.selected_products[' || old_item.key || '].asset_access.expected_size_bytes'
            ) = json_remove(
                NEW.snapshot_json,
                '$.selected_products[' || old_item.key || '].asset_access.expected_size_bytes'
            )
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'frozen task snapshot cannot be updated');
END;
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreHealth {
    pub schema_version: i64,
    pub journal_mode: String,
    pub foreign_keys: bool,
    pub busy_timeout_millis: i64,
    pub synchronous: i64,
}

#[derive(Debug)]
pub struct TaskStore {
    pub(crate) connection: Connection,
    /// #22 B slice 3(b): largest single read batch used by the streaming
    /// seal so far. Test-only evidence that the seal's in-memory candidate
    /// buffer stays bounded; never read by production paths.
    pub(crate) seal_read_batch_cursor: std::sync::Mutex<usize>,
    /// #22 B slice 3(b) review fix 5: how many times the streaming seal has
    /// decoded the whole snapshot blob. Test-only evidence that the seal
    /// stays streaming; never read by production paths.
    pub(crate) seal_blob_decode_count: std::sync::Mutex<u64>,
}

impl TaskStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_internal(path.as_ref(), true)
    }

    /// Open an existing Store without creating a missing file or parent
    /// directory.  Existing databases still follow the normal compatibility
    /// migration path in `open_internal`; the distinction is only whether a
    /// first-run empty database may be created by this caller.
    pub fn open_existing(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_internal(path.as_ref(), false)
    }

    /// Open an existing Store for verification-only reads.
    ///
    /// The database is opened SQLite read-only with the exact supported
    /// schema. No migration, journal-mode, synchronous, foreign-key, or any
    /// other pragma is changed, and no scheduler/Dispatcher/Worker/Provider or
    /// mutation capability is constructed by this profile. It fails closed
    /// when `-wal`/`-shm` sidecars exist because an effective consistent read
    /// cannot then be guaranteed without creating or changing sidecars.
    pub fn open_verification_read_only(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() && metadata.len() > 0 => {}
            Ok(metadata) if metadata.is_file() && metadata.len() == 0 => {
                return Err(StoreError::Empty)
            }
            Ok(_) => return Err(StoreError::Missing),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError::Missing)
            }
            Err(error) => return Err(StoreError::Filesystem(error)),
        }
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.as_os_str().to_os_string();
            sidecar.push(suffix);
            let sidecar = PathBuf::from(&sidecar);
            if sidecar.exists() {
                return Err(StoreError::invalid(
                    "verification requires a consistent read without WAL/SHM sidecars",
                ));
            }
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        let current_version: i64 =
            connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current_version != STORE_SCHEMA_VERSION {
            return Err(StoreError::invalid(
                "verification requires the exact supported Store schema version",
            ));
        }
        verify_source_receipt_schema(&connection)?;
        Ok(Self {
            connection,
            seal_read_batch_cursor: std::sync::Mutex::new(0),
            seal_blob_decode_count: std::sync::Mutex::new(0),
        })
    }

    fn open_internal(path: &Path, allow_create: bool) -> Result<Self, StoreError> {
        if !allow_create {
            match fs::metadata(path) {
                Ok(metadata) if metadata.is_file() && metadata.len() > 0 => {}
                Ok(metadata) if metadata.is_file() && metadata.len() == 0 => {
                    return Err(StoreError::Empty)
                }
                Ok(_) => return Err(StoreError::Missing),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(StoreError::Missing)
                }
                Err(error) => return Err(StoreError::Filesystem(error)),
            }
        }
        if allow_create {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).map_err(StoreError::Filesystem)?;
            }
        }
        let mut connection = if allow_create {
            Connection::open(path)?
        } else {
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?
        };
        connection.busy_timeout(BUSY_TIMEOUT)?;
        // Enable WAL before anything else. On Linux (ext4) the rollback-journal
        // lock taken while creating a new database can make a concurrent first
        // read of user_version fail at once with SQLITE_BUSY instead of
        // waiting; on Windows the byte-range locking usually just blocks.
        // Enabling WAL first, with the same busy retry, also fails closed for
        // unsupported files (read-only media, locked remnants) before any
        // schema decision.
        retry_while_busy(|| {
            connection
                .pragma_update(None, "journal_mode", "WAL")
                .map_err(StoreError::from)
        })?;

        let current_version: i64 = retry_while_busy(|| {
            connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(StoreError::from)
        })?;
        if current_version > STORE_SCHEMA_VERSION {
            return Err(StoreError::NewerSchema {
                found: current_version,
                supported: STORE_SCHEMA_VERSION,
            });
        }
        if current_version < STORE_SCHEMA_VERSION {
            retry_while_busy(|| migrate(&mut connection))?;
        }

        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        // Bound WAL file growth to 16 MiB after checkpoints so WAL never bloats disk unboundedly.
        let _ = connection.pragma_update(None, "journal_size_limit", 16 * 1024 * 1024_i64);
        verify_source_receipt_schema(&connection)?;
        Ok(Self {
            connection,
            seal_read_batch_cursor: std::sync::Mutex::new(0),
            seal_blob_decode_count: std::sync::Mutex::new(0),
        })
    }

    pub fn health(&self) -> Result<StoreHealth, StoreError> {
        let schema_version = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        let journal_mode = self
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        let foreign_keys: i64 =
            self.connection
                .pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
        let busy_timeout_millis =
            self.connection
                .pragma_query_value(None, "busy_timeout", |row| row.get(0))?;
        let synchronous = self
            .connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))?;
        Ok(StoreHealth {
            schema_version,
            journal_mode,
            foreign_keys: foreign_keys == 1,
            busy_timeout_millis,
            synchronous,
        })
    }

    pub fn table_names(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT name FROM sqlite_schema \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let names = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(names)
    }
}

/// Retries `operation` while SQLite reports the database busy or locked,
/// within the connection's busy timeout. SQLite skips its busy handler when
/// waiting could deadlock (two connections escalating their locks); the
/// operation is then repeated from the start. Every other error, and a
/// lock still held at the deadline, is returned as it is.
fn retry_while_busy<T>(
    mut operation: impl FnMut() -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        match operation() {
            Err(StoreError::Database(rusqlite::Error::SqliteFailure(failure, message)))
                if matches!(
                    failure.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) =>
            {
                if Instant::now() >= deadline {
                    return Err(StoreError::Database(rusqlite::Error::SqliteFailure(
                        failure, message,
                    )));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            other => return other,
        }
    }
}

fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Decide under the write lock. The version read before the lock can be
    // stale: another connection may have created or migrated the schema
    // while this one waited, and migrating again from that stale version
    // fails with "table ... already exists" (two connections opening the
    // same new Store at once, TASK-REG-012 reverification 2026-10-20).
    let current_version: i64 =
        transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if current_version > STORE_SCHEMA_VERSION {
        return Err(StoreError::NewerSchema {
            found: current_version,
            supported: STORE_SCHEMA_VERSION,
        });
    }
    if current_version == STORE_SCHEMA_VERSION {
        return Ok(());
    }
    if current_version < 1 {
        transaction.execute_batch(SCHEMA_V1)?;
        transaction.pragma_update(None, "user_version", 1)?;
    }
    if current_version < 2 {
        transaction.execute_batch(SCHEMA_V2)?;
        transaction.pragma_update(None, "user_version", 2)?;
    }
    if current_version < 3 {
        transaction.execute_batch(SCHEMA_V3)?;
        transaction.pragma_update(None, "user_version", 3)?;
    }
    if current_version < 4 {
        transaction.execute_batch(SCHEMA_V4)?;
        transaction.pragma_update(None, "user_version", 4)?;
    }
    if current_version < 5 {
        transaction.execute_batch(SCHEMA_V5)?;
        transaction.pragma_update(None, "user_version", 5)?;
    }
    if current_version < 6 {
        transaction.execute_batch(SCHEMA_V6)?;
        migrate_legacy_aoi_context(&transaction)?;
        transaction.pragma_update(None, "user_version", 6)?;
    }
    if current_version < 7 {
        if table_exists(&transaction, "download_results")? {
            transaction.execute_batch(SCHEMA_V7)?;
        }
        transaction.pragma_update(None, "user_version", 7)?;
    }
    if current_version < 8 {
        if !table_exists(&transaction, "download_progress")?
            || !table_exists(&transaction, "download_results")?
        {
            if current_version >= 6 {
                return Err(StoreError::invalid(
                    "schema v8 migration requires download telemetry tables",
                ));
            }
            transaction.pragma_update(None, "user_version", 8)?;
        } else {
            validate_v8_telemetry_json(&transaction)?;
            transaction.execute_batch(SCHEMA_V8)?;
            transaction.pragma_update(None, "user_version", 8)?;
        }
    }
    if current_version < 9 {
        if current_version >= 8
            && (!table_exists(&transaction, "task_history")?
                || !table_exists(&transaction, "task_snapshots")?
                || !table_exists(&transaction, "task_runtime")?)
        {
            return Err(StoreError::invalid(
                "schema v9 migration requires task history authority tables",
            ));
        }
        transaction.execute_batch(SCHEMA_V9)?;
        crate::history::backfill_history_summaries(&transaction)?;
        transaction.pragma_update(None, "user_version", 9)?;
    }
    if current_version < 10 {
        if current_version >= 9
            && (!table_exists(&transaction, "task_history_summary")?
                || !table_exists(&transaction, "task_history")?
                || !table_exists(&transaction, "task_snapshots")?
                || !table_exists(&transaction, "task_runtime")?)
        {
            return Err(StoreError::invalid(
                "schema v10 migration requires materialized task history authority",
            ));
        }
        transaction.execute_batch(SCHEMA_V10)?;
        crate::history::backfill_dem_source_authorities(&transaction)?;
        transaction.pragma_update(None, "user_version", 10)?;
    }
    if current_version < 11 {
        if current_version >= 10
            && (!table_exists(&transaction, "task_history_dem_source_authority")?
                || !table_exists(&transaction, "task_history_summary")?)
        {
            return Err(StoreError::invalid(
                "schema v11 migration requires DEM history authority",
            ));
        }
        transaction.execute_batch(SCHEMA_V11)?;
        transaction.pragma_update(None, "user_version", 11)?;
    }
    if current_version < 12 {
        if current_version >= 11 && !table_exists(&transaction, "dem_source_identity")? {
            return Err(StoreError::invalid(
                "schema v12 migration requires DEM source identity authority",
            ));
        }
        transaction.execute_batch(SCHEMA_V12)?;
        transaction.pragma_update(None, "user_version", 12)?;
    }
    if current_version < 13 {
        if current_version >= 12 && !table_exists(&transaction, "standalone_aoi_state")? {
            return Err(StoreError::invalid(
                "schema v13 migration requires standalone AOI authority",
            ));
        }
        transaction.execute_batch(SCHEMA_V13)?;
        transaction.pragma_update(None, "user_version", 13)?;
    }
    if current_version < 14 {
        if current_version >= 13 && !table_exists(&transaction, "search_batches")? {
            return Err(StoreError::invalid(
                "schema v14 migration requires immutable search batches",
            ));
        }
        if table_exists(&transaction, "sar_search_batch_cache")? {
            validate_v14_sar_cache_table(&transaction)?;
            transaction.execute_batch(
                "CREATE INDEX IF NOT EXISTS sar_search_batch_cache_updated_idx
                 ON sar_search_batch_cache(updated_at_epoch DESC, cache_key ASC);",
            )?;
        } else {
            transaction.execute_batch(SCHEMA_V14)?;
        }
        transaction.pragma_update(None, "user_version", 14)?;
    }
    if current_version < 15 {
        if current_version >= 14
            && (!table_exists(&transaction, "task_snapshots")?
                || !table_exists(&transaction, "task_runtime")?
                || !table_exists(&transaction, "task_history_summary")?)
        {
            return Err(StoreError::invalid(
                "schema v15 migration requires task and history authority tables",
            ));
        }
        if table_exists(&transaction, "task_transfer_telemetry")? {
            validate_v15_transfer_telemetry_table(&transaction)?;
        } else {
            transaction.execute_batch(SCHEMA_V15_TELEMETRY)?;
        }
        let history_column_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('task_history_summary') WHERE name='transfer_active_millis')",
            [],
            |row| row.get(0),
        )?;
        if history_column_exists {
            validate_v15_history_column(&transaction)?;
        } else {
            transaction.execute_batch(SCHEMA_V15_HISTORY)?;
        }
        transaction.pragma_update(None, "user_version", 15)?;
    }
    if current_version < 16 {
        if current_version >= 15 && !table_exists(&transaction, "task_history_summary")? {
            return Err(StoreError::invalid(
                "schema v16 migration requires materialized task history summaries",
            ));
        }
        let history_dataset_column_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('task_history_summary') WHERE name='dataset_id')",
            [],
            |row| row.get(0),
        )?;
        if history_dataset_column_exists {
            validate_v16_history_dataset_column(&transaction)?;
        } else {
            transaction.execute_batch(SCHEMA_V16_HISTORY_DATASET)?;
        }
        crate::history::backfill_history_dataset_ids(&transaction)?;
        transaction.pragma_update(None, "user_version", 16)?;
    }
    if current_version < 17 {
        ensure_v17_administrative_boundary_cache(&transaction)?;
        transaction.pragma_update(None, "user_version", 17)?;
    }
    if current_version < 18 {
        if !table_exists(&transaction, "sar_search_batch_count")? {
            transaction.execute_batch(SCHEMA_V18_SAR_BACKGROUND_COUNT)?;
        }
        transaction.pragma_update(None, "user_version", 18)?;
    }
    if current_version < 19 {
        if !table_exists(&transaction, "task_membership_additions")? {
            transaction.execute_batch(SCHEMA_V19_TASK_MEMBERSHIP_ADDITIONS)?;
        }
        backfill_task_membership_additions(&transaction)?;
        transaction.pragma_update(None, "user_version", 19)?;
    }
    if current_version < 20 {
        if !table_exists(&transaction, "search_batch_staging")? {
            transaction.execute_batch(SCHEMA_V20_SEARCH_BATCH_STAGING)?;
        }
        transaction.pragma_update(None, "user_version", 20)?;
    }
    if current_version < 21 {
        let task_snapshots_support_plan_freeze = [
            "task_id",
            "batch_id",
            "provider_id",
            "created_at",
            "output_root",
            "layout",
            "snapshot_json",
        ]
        .into_iter()
        .map(|column| column_exists(&transaction, "task_snapshots", column))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .all(|exists| exists);
        if current_version >= 20 && !task_snapshots_support_plan_freeze {
            return Err(StoreError::invalid(
                "schema v21 migration requires immutable task snapshots",
            ));
        }
        if task_snapshots_support_plan_freeze {
            transaction.execute_batch(SCHEMA_V21_NISAR_EXECUTION_PLAN_FREEZE)?;
        }
        transaction.pragma_update(None, "user_version", 21)?;
    }
    if current_version < 22 {
        if current_version >= 21 && !table_exists(&transaction, "task_snapshots")? {
            return Err(StoreError::invalid(
                "schema v22 migration requires task snapshot authority",
            ));
        }
        if !table_exists(&transaction, "nisar_subset_execution_anchors_v2")? {
            transaction.execute_batch(SCHEMA_V22_NISAR_EXECUTION_ANCHORS)?;
        }
        transaction.pragma_update(None, "user_version", 22)?;
    }
    if current_version < 23 {
        if current_version >= 22 && !table_exists(&transaction, "task_snapshots")? {
            return Err(StoreError::invalid(
                "schema v23 migration requires task snapshot authority",
            ));
        }
        if table_exists(&transaction, "task_snapshots")? {
            transaction.execute_batch(SCHEMA_V23_NISAR_MULTI_ITEM_EXECUTION_PLAN_FREEZE)?;
        }
        transaction.pragma_update(None, "user_version", 23)?;
    }
    if current_version < 24 {
        transaction.execute_batch(crate::gacos_submission::GACOS_SUBMISSION_ATTEMPTS_SCHEMA_SQL)?;
        transaction.pragma_update(None, "user_version", 24)?;
    }
    if current_version < 25 {
        // Older stores just created the ledger with the current schema above.
        // Only v24 has an existing ledger whose state CHECK must be widened.
        if current_version == 24 {
            migrate_gacos_submission_attempts_v25(&transaction)?;
        }
        transaction.pragma_update(None, "user_version", 25)?;
    }
    if current_version < 26 {
        // NULL means an older task has no measured network byte counter. Do not
        // backfill it from output sizes: DEM crops and source tiles differ.
        if !column_exists(
            &transaction,
            "task_transfer_telemetry",
            "network_bytes_received",
        )? {
            transaction.execute_batch(
                "ALTER TABLE task_transfer_telemetry ADD COLUMN network_bytes_received INTEGER
                 CHECK(network_bytes_received IS NULL OR network_bytes_received >= 0);",
            )?;
        }
        validate_v15_transfer_telemetry_table(&transaction)?;
        transaction.pragma_update(None, "user_version", 26)?;
    }
    if current_version < 27 {
        transaction.execute_batch(crate::repository::nisar_archived_recovery::SCHEMA_SQL)?;
        transaction.pragma_update(None, "user_version", 27)?;
    }
    if current_version < 28 {
        if current_version >= 27 && !table_exists(&transaction, "task_snapshots")? {
            return Err(StoreError::invalid(
                "schema v28 migration requires task snapshot authority",
            ));
        }
        if table_exists(&transaction, "task_snapshots")? {
            transaction.execute_batch(SCHEMA_V28_NISAR_SOURCE_SIZE_FREEZE)?;
        }
        transaction.pragma_update(None, "user_version", 28)?;
    }
    if current_version < 29 {
        transaction.execute_batch(crate::repository::nisar_budget_upgrade::SCHEMA_SQL)?;
        transaction.pragma_update(None, "user_version", 29)?;
    }
    if current_version < 30 {
        transaction.execute_batch(crate::repository::nisar_budget_upgrade::REFUSAL_SCHEMA_SQL)?;
        transaction.pragma_update(None, "user_version", 30)?;
    }
    if current_version < 31 {
        // Downloader D3: landing ledger, item local reuse decisions, scans.
        transaction.execute_batch(crate::local_reuse::SCHEMA_V31_SQL)?;
        transaction.pragma_update(None, "user_version", 31)?;
    }
    if current_version < 32 {
        transaction.execute_batch(crate::agent_plans::SCHEMA_V32_SQL)?;
        transaction.pragma_update(None, "user_version", 32)?;
    }
    if current_version < 33 {
        // W2: additive per-source views and reverse indexes. Every statement is
        // idempotent so a repeated or partially faked upgrade converges.
        transaction.execute_batch(crate::source_index::SCHEMA_V33_SQL)?;
        crate::source_index::backfill_schema_v33(&transaction)?;
        transaction.pragma_update(None, "user_version", 33)?;
    }
    if current_version < 34 {
        // Agent permission modes: approvals by mode and the mode in effect.
        crate::agent_plans::upgrade_schema_v34(&transaction)?;
        transaction.pragma_update(None, "user_version", 34)?;
    }
    if current_version < 35 {
        // Owner 2026-10-26: three permission modes without limits; approvals
        // keep the plan hash a card's output directory choice rewrote.
        crate::agent_plans::upgrade_schema_v35(&transaction)?;
        transaction.pragma_update(None, "user_version", 35)?;
    }
    if current_version < 36 {
        // Each run row says whether a provider refused the reasoning-off
        // switch in it and whether a reply was truncated (run lists).
        crate::agent_runs::upgrade_schema_v36(&transaction)?;
        transaction.pragma_update(None, "user_version", 36)?;
    }
    if current_version < 37 {
        // Each run row keeps the token usage its channel reported (run lists).
        crate::agent_runs::upgrade_schema_v37(&transaction)?;
        transaction.pragma_update(None, "user_version", 37)?;
    }
    if current_version < 38 {
        // A cancelled worker result the scheduler could not settle yet
        // survives a restart instead of living only in coordinator memory.
        transaction.execute_batch(crate::runtime::SCHEMA_V38_SQL)?;
        transaction.pragma_update(None, "user_version", 38)?;
    }
    if current_version < 39 {
        // #22 B slice 1: one row per frozen search candidate, written beside
        // the snapshot blob. Older batches keep only their blob (no backfill).
        transaction.execute_batch(crate::batch_candidates::SCHEMA_V39_SQL)?;
        transaction.pragma_update(None, "user_version", 39)?;
    }
    if current_version < 40 {
        // Orbit scheduled-fetch S2: the inert recheck schedule and companion
        // preference tables. Structure only; nothing writes them in this
        // slice and no existing data is backfilled.
        transaction.execute_batch(crate::orbit_recheck::SCHEMA_V40_SQL)?;
        transaction.pragma_update(None, "user_version", 40)?;
    }
    if current_version < 41 {
        // #22 B slice 3(c): progressive delivery state for search batches.
        // Structure only; batches frozen before this slice have no row and
        // read as complete.
        transaction.execute_batch(crate::batch_progress::SCHEMA_V41_SQL)?;
        transaction.pragma_update(None, "user_version", 41)?;
    }
    if current_version < 42 {
        // Preserve request identity independently of the task lifecycle.
        transaction.execute_batch(crate::source_receipts::SCHEMA_V42_SQL)?;
        transaction.pragma_update(None, "user_version", 42)?;
    }
    if current_version < 43 {
        if !table_exists(&transaction, "task_events")? {
            return Err(StoreError::invalid(
                "schema v43 migration requires task events",
            ));
        }
        // Receipt-backed event IDs remain reserved after their events are pruned.
        transaction.execute_batch(crate::source_receipts::SCHEMA_V43_SQL)?;
        transaction.pragma_update(None, "user_version", 43)?;
    }
    verify_source_receipt_schema(&transaction)?;
    transaction.commit()?;
    Ok(())
}

fn normalize_schema_sql(sql: &str) -> String {
    let collapsed = sql
        .trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut normalized = collapsed;
    for table in [
        "task_events",
        "source_receipt_event_slots",
        "source_mutation_receipts",
        "source_receipt_schema_marker",
    ] {
        let quoted = format!("\"{table}\"");
        normalized = normalized.replace(&quoted, table);
    }
    normalized
}

// Some development builds used version 43 for a different, unfinished schema.
// Refuse them rather than silently running without event reservation triggers.
fn verify_source_receipt_schema(connection: &Connection) -> Result<(), StoreError> {
    for &(table_name, expected_sql) in crate::source_receipts::EXPECTED_SOURCE_RECEIPT_TABLES {
        let actual_sql: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [table_name],
                |row| row.get(0),
            )
            .optional()?;
        match actual_sql {
            Some(sql) if normalize_schema_sql(&sql) == normalize_schema_sql(expected_sql) => {}
            _ => return Err(StoreError::invalid("source receipt schema is incomplete")),
        }
    }
    for &(tbl_name, trigger_name, expected_sql) in
        crate::source_receipts::EXPECTED_SOURCE_RECEIPT_TRIGGERS
    {
        let actual_sql: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = ?1 AND name = ?2",
                [tbl_name, trigger_name],
                |row| row.get(0),
            )
            .optional()?;
        match actual_sql {
            Some(sql) if normalize_schema_sql(&sql) == normalize_schema_sql(expected_sql) => {}
            _ => return Err(StoreError::invalid("source receipt schema is incomplete")),
        }
    }
    let protocol: Option<i64> = connection
        .query_row(
            "SELECT protocol FROM source_receipt_schema_marker WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if protocol != Some(2) {
        return Err(StoreError::invalid(
            "source receipt schema protocol is incompatible",
        ));
    }
    Ok(())
}

fn backfill_task_membership_additions(connection: &Connection) -> Result<(), StoreError> {
    for (table, columns) in [
        (
            "task_snapshots",
            &["task_id", "batch_id", "snapshot_json"][..],
        ),
        (
            "task_runtime",
            &["task_id", "revision", "updated_at", "state_json"][..],
        ),
        ("search_batches", &["batch_id", "snapshot_json"][..]),
        ("task_history", &["task_id"][..]),
    ] {
        if !table_exists(connection, table)? {
            return Ok(());
        }
        for column in columns {
            if !column_exists(connection, table, column)? {
                return Ok(());
            }
        }
    }
    connection.execute_batch(
        "INSERT OR IGNORE INTO task_membership_additions(
             task_id, item_id, added_revision, position, added_at, item_json
         )
         SELECT
             snapshot.task_id,
             CAST(selected.value AS TEXT),
             runtime.revision,
             CAST(selected.key AS INTEGER),
             runtime.updated_at,
             candidate.value
         FROM task_snapshots snapshot
         JOIN task_runtime runtime ON runtime.task_id = snapshot.task_id
         JOIN search_batches batch ON batch.batch_id = snapshot.batch_id
         LEFT JOIN task_history history ON history.task_id = snapshot.task_id
         JOIN json_each(
             CASE WHEN json_valid(runtime.state_json) THEN runtime.state_json ELSE '{}' END,
             '$.selected_item_ids'
         ) selected
         JOIN json_each(
             CASE WHEN json_valid(batch.snapshot_json) THEN batch.snapshot_json ELSE '{}' END,
             '$.candidates'
         ) candidate
           ON json_extract(candidate.value, '$.item_id') = selected.value
         WHERE json_type(
             CASE WHEN json_valid(snapshot.snapshot_json) THEN snapshot.snapshot_json ELSE '{}' END,
             '$.selected_products'
         ) = 'array'
           AND history.task_id IS NULL
           AND NOT EXISTS (
               SELECT 1
               FROM json_each(
                   CASE WHEN json_valid(snapshot.snapshot_json) THEN snapshot.snapshot_json ELSE '{}' END,
                   '$.selected_products'
               ) initial
               WHERE json_extract(initial.value, '$.item_id') = selected.value
           );",
    )?;
    let expected: i64 = connection.query_row(
        "SELECT COUNT(*)
         FROM task_snapshots snapshot
         JOIN task_runtime runtime ON runtime.task_id = snapshot.task_id
         LEFT JOIN task_history history ON history.task_id = snapshot.task_id
         JOIN json_each(
             CASE WHEN json_valid(runtime.state_json) THEN runtime.state_json ELSE '{}' END,
             '$.selected_item_ids'
         ) selected
         WHERE json_type(
             CASE WHEN json_valid(snapshot.snapshot_json) THEN snapshot.snapshot_json ELSE '{}' END,
             '$.selected_products'
         ) = 'array'
           AND history.task_id IS NULL
           AND NOT EXISTS (
               SELECT 1
               FROM json_each(
                   CASE WHEN json_valid(snapshot.snapshot_json) THEN snapshot.snapshot_json ELSE '{}' END,
                   '$.selected_products'
               ) initial
               WHERE json_extract(initial.value, '$.item_id') = selected.value
           )",
        [],
        |row| row.get(0),
    )?;
    let actual: i64 = connection.query_row(
        "SELECT COUNT(*)
         FROM task_membership_additions addition
         JOIN task_snapshots snapshot ON snapshot.task_id = addition.task_id
         LEFT JOIN task_history history ON history.task_id = addition.task_id
         WHERE json_type(
             CASE WHEN json_valid(snapshot.snapshot_json) THEN snapshot.snapshot_json ELSE '{}' END,
             '$.selected_products'
         ) = 'array'
           AND history.task_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    if expected != actual {
        return Err(StoreError::invalid(
            "schema v19 migration cannot resolve appended task items from their immutable source batches",
        ));
    }
    Ok(())
}

fn validate_v8_telemetry_json(connection: &rusqlite::Connection) -> Result<(), StoreError> {
    let invalid_progress: i64 = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM download_progress WHERE NOT json_valid(progress_json) OR \
         COALESCE(json_type(progress_json, '$.bytes_completed'), '') != 'integer' OR \
         typeof(json_extract(progress_json, '$.bytes_completed')) != 'integer' OR \
         json_extract(progress_json, '$.bytes_completed') < 0 OR \
         COALESCE(json_type(progress_json, '$.bytes_per_second'), '') NOT IN ('integer','real') OR \
         json_extract(progress_json, '$.bytes_per_second') < 0 OR \
         json_extract(progress_json, '$.bytes_per_second') >= 1e999 OR \
         (json_type(progress_json, '$.total_bytes') IS NOT NULL AND \
           (json_type(progress_json, '$.total_bytes') NOT IN ('integer','null') OR \
            (json_type(progress_json, '$.total_bytes') = 'integer' AND typeof(json_extract(progress_json, '$.total_bytes')) != 'integer') OR \
            json_extract(progress_json, '$.total_bytes') < 0)))",
        [], |row| row.get(0),
    )?;
    let invalid_results: i64 = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM download_results WHERE NOT json_valid(result_json) OR \
         COALESCE(json_type(result_json, '$.bytes_written'), '') != 'integer' OR \
         typeof(json_extract(result_json, '$.bytes_written')) != 'integer' OR \
         json_extract(result_json, '$.bytes_written') < 0)",
        [],
        |row| row.get(0),
    )?;
    if invalid_progress != 0 || invalid_results != 0 {
        return Err(StoreError::invalid(
            "schema v8 migration rejects invalid telemetry JSON",
        ));
    }
    Ok(())
}

fn validate_v14_sar_cache_table(connection: &rusqlite::Connection) -> Result<(), StoreError> {
    let mut statement = connection.prepare("PRAGMA table_info('sar_search_batch_cache')")?;
    let columns = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let expected = vec![
        ("cache_key".to_owned(), "TEXT".to_owned(), 1, 1),
        ("source_batch_id".to_owned(), "TEXT".to_owned(), 1, 0),
        ("updated_at_epoch".to_owned(), "INTEGER".to_owned(), 1, 0),
        ("complete".to_owned(), "INTEGER".to_owned(), 1, 0),
    ];
    if columns != expected {
        return Err(StoreError::invalid(
            "schema v14 rejects an incompatible SAR cache table",
        ));
    }
    let table_sql: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name='sar_search_batch_cache'",
        [],
        |row| row.get(0),
    )?;
    for required in [
        "length(cache_key) = 64",
        "cache_key = lower(cache_key)",
        "updated_at_epoch >= 0",
        "complete IN (0, 1)",
        "STRICT",
    ] {
        if !table_sql.contains(required) {
            return Err(StoreError::invalid(
                "schema v14 rejects an unsafe SAR cache table",
            ));
        }
    }
    let foreign_key: Option<(String, String, String)> = connection
        .query_row(
            "SELECT \"table\", \"from\", on_delete
             FROM pragma_foreign_key_list('sar_search_batch_cache')
             WHERE \"from\"='source_batch_id' LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if foreign_key
        != Some((
            "search_batches".to_owned(),
            "source_batch_id".to_owned(),
            "CASCADE".to_owned(),
        ))
    {
        return Err(StoreError::invalid(
            "schema v14 rejects SAR cache without batch ownership",
        ));
    }
    Ok(())
}

fn migrate_gacos_submission_attempts_v25(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), StoreError> {
    if !table_exists(transaction, "gacos_submission_attempts")? {
        transaction.execute_batch(crate::gacos_submission::GACOS_SUBMISSION_ATTEMPTS_SCHEMA_SQL)?;
        return Ok(());
    }

    // SQLite cannot alter a CHECK constraint in place. Rebuild only this
    // ledger table while copying every existing attempt verbatim.
    transaction.execute_batch(
        "DROP INDEX IF EXISTS gacos_submission_attempts_task_item_order_idx;
         DROP INDEX IF EXISTS gacos_submission_attempts_fingerprint_account_idx;
         ALTER TABLE gacos_submission_attempts RENAME TO gacos_submission_attempts_v24;",
    )?;
    transaction.execute_batch(crate::gacos_submission::GACOS_SUBMISSION_ATTEMPTS_SCHEMA_SQL)?;
    transaction.execute_batch(
        "INSERT INTO gacos_submission_attempts(
             attempt_id, task_id, item_id, fingerprint_sha256,
             account_identity_hash_sha256, attempt_number, state,
             reason_code, http_status, automatic_retry_allowed,
             created_at, updated_at
         )
         SELECT attempt_id, task_id, item_id, fingerprint_sha256,
                account_identity_hash_sha256, attempt_number, state,
                reason_code, http_status, automatic_retry_allowed,
                created_at, updated_at
         FROM gacos_submission_attempts_v24;
         DROP TABLE gacos_submission_attempts_v24;",
    )?;
    Ok(())
}

fn table_exists(connection: &rusqlite::Connection, name: &str) -> Result<bool, StoreError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(StoreError::from)
}

fn column_exists(
    connection: &rusqlite::Connection,
    table: &str,
    column: &str,
) -> Result<bool, StoreError> {
    connection
        .query_row(
            "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2",
            [table, column],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(StoreError::from)
}

fn index_exists(connection: &rusqlite::Connection, name: &str) -> Result<bool, StoreError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'index' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(StoreError::from)
}

fn ensure_v17_administrative_boundary_cache(
    connection: &rusqlite::Connection,
) -> Result<(), StoreError> {
    if !table_exists(connection, "administrative_boundary_cache")? {
        connection.execute_batch(SCHEMA_V17_ADMINISTRATIVE_BOUNDARY_CACHE)?;
        return Ok(());
    }

    validate_v17_administrative_boundary_cache_table(connection)?;
    if index_exists(connection, "administrative_boundary_cache_cached_at_idx")? {
        validate_v17_administrative_boundary_cache_index(connection)?;
    } else {
        connection.execute_batch(SCHEMA_V17_ADMINISTRATIVE_BOUNDARY_CACHE_INDEX)?;
    }
    Ok(())
}

fn validate_v17_administrative_boundary_cache_table(
    connection: &rusqlite::Connection,
) -> Result<(), StoreError> {
    let columns = connection
        .prepare(
            "SELECT name, type, \"notnull\", pk
             FROM pragma_table_info('administrative_boundary_cache') ORDER BY cid",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let expected = [
        ("provider".to_owned(), "TEXT".to_owned(), 1, 1),
        ("provider_object_id".to_owned(), "TEXT".to_owned(), 1, 2),
        ("name".to_owned(), "TEXT".to_owned(), 1, 0),
        ("display_name".to_owned(), "TEXT".to_owned(), 1, 0),
        ("country_code".to_owned(), "TEXT".to_owned(), 0, 0),
        ("admin_level".to_owned(), "TEXT".to_owned(), 0, 0),
        ("geometry_json".to_owned(), "TEXT".to_owned(), 1, 0),
        ("west".to_owned(), "REAL".to_owned(), 1, 0),
        ("south".to_owned(), "REAL".to_owned(), 1, 0),
        ("east".to_owned(), "REAL".to_owned(), 1, 0),
        ("north".to_owned(), "REAL".to_owned(), 1, 0),
        ("source".to_owned(), "TEXT".to_owned(), 1, 0),
        ("attribution".to_owned(), "TEXT".to_owned(), 0, 0),
        ("cached_at".to_owned(), "TEXT".to_owned(), 1, 0),
    ];
    if columns != expected {
        return Err(StoreError::invalid(
            "schema v17 rejects an incompatible administrative boundary cache table",
        ));
    }

    let table_sql: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name='administrative_boundary_cache'",
        [],
        |row| row.get(0),
    )?;
    for required in [
        "provider IN ('datav', 'nominatim')",
        "length(provider_object_id) BETWEEN 1 AND 128",
        "length(geometry_json) BETWEEN 2 AND 8388608",
        "west >= -180.0 AND west < 180.0",
        "east > -180.0 AND east <= 180.0 AND east > west",
        "south >= -90.0 AND south < 90.0",
        "north > -90.0 AND north <= 90.0 AND north > south",
        "provider_object_id NOT GLOB '*[^0-9]*'",
        "provider_object_id GLOB 'relation:[1-9]*'",
        "PRIMARY KEY(provider, provider_object_id)",
        "STRICT",
    ] {
        if !table_sql.contains(required) {
            return Err(StoreError::invalid(
                "schema v17 rejects an unsafe administrative boundary cache table",
            ));
        }
    }
    Ok(())
}

fn validate_v17_administrative_boundary_cache_index(
    connection: &rusqlite::Connection,
) -> Result<(), StoreError> {
    let columns = connection
        .prepare(
            "SELECT name FROM pragma_index_info('administrative_boundary_cache_cached_at_idx')
             ORDER BY seqno",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if columns != ["cached_at", "provider", "provider_object_id"] {
        return Err(StoreError::invalid(
            "schema v17 rejects an incompatible administrative boundary cache index",
        ));
    }
    Ok(())
}

fn validate_v16_history_dataset_column(
    connection: &rusqlite::Connection,
) -> Result<(), StoreError> {
    let column: Option<(String, String, i64)> = connection
        .query_row(
            "SELECT name, type, \"notnull\" FROM pragma_table_info('task_history_summary') WHERE name='dataset_id'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if column != Some(("dataset_id".to_owned(), "TEXT".to_owned(), 0)) {
        return Err(StoreError::invalid(
            "schema v16 rejects an incompatible history dataset column",
        ));
    }
    let table_sql: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name='task_history_summary'",
        [],
        |row| row.get(0),
    )?;
    for required in [
        "length(dataset_id) BETWEEN 1 AND 128",
        "trim(dataset_id) = dataset_id",
    ] {
        if !table_sql.contains(required) {
            return Err(StoreError::invalid(
                "schema v16 rejects an unsafe history dataset column",
            ));
        }
    }
    Ok(())
}

fn validate_v15_transfer_telemetry_table(
    connection: &rusqlite::Connection,
) -> Result<(), StoreError> {
    let columns = connection
        .prepare("SELECT name, type, \"notnull\", pk FROM pragma_table_info('task_transfer_telemetry') ORDER BY cid")?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut expected_columns = vec![
        ("task_id".to_owned(), "TEXT".to_owned(), 1, 1),
        (
            "transfer_active_millis".to_owned(),
            "INTEGER".to_owned(),
            1,
            0,
        ),
        ("updated_at".to_owned(), "TEXT".to_owned(), 1, 0),
    ];
    let has_network_measurement = columns.len() == 4;
    if has_network_measurement {
        expected_columns.push((
            "network_bytes_received".to_owned(),
            "INTEGER".to_owned(),
            0,
            0,
        ));
    }
    if columns != expected_columns {
        return Err(StoreError::invalid(
            "schema v15 rejects an incompatible transfer telemetry table",
        ));
    }
    let table_sql: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name='task_transfer_telemetry'",
        [],
        |row| row.get(0),
    )?;
    for required in [
        "length(task_id) BETWEEN 1 AND 128",
        "transfer_active_millis >= 0",
        "length(updated_at) BETWEEN 20 AND 30",
        "STRICT",
    ] {
        if !table_sql.contains(required) {
            return Err(StoreError::invalid(
                "schema v15 rejects an unsafe transfer telemetry table",
            ));
        }
    }
    if has_network_measurement {
        let default_value: Option<String> = connection.query_row(
            "SELECT dflt_value FROM pragma_table_info('task_transfer_telemetry') WHERE name='network_bytes_received'",
            [], |row| row.get(0),
        )?;
        if default_value.is_some()
            || !table_sql.contains("network_bytes_received IS NULL OR network_bytes_received >= 0")
        {
            return Err(StoreError::invalid(
                "schema v26 rejects an unsafe network byte counter",
            ));
        }
    }
    Ok(())
}

fn validate_v15_history_column(connection: &rusqlite::Connection) -> Result<(), StoreError> {
    let column: (String, i64, Option<String>) = connection.query_row(
        "SELECT type, \"notnull\", dflt_value FROM pragma_table_info('task_history_summary') WHERE name='transfer_active_millis'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if column != ("INTEGER".to_owned(), 1, Some("0".to_owned())) {
        return Err(StoreError::invalid(
            "schema v15 rejects an incompatible history transfer telemetry column",
        ));
    }
    let table_sql: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name='task_history_summary'",
        [],
        |row| row.get(0),
    )?;
    if !table_sql.contains("transfer_active_millis >= 0") {
        return Err(StoreError::invalid(
            "schema v15 rejects an unsafe history transfer telemetry column",
        ));
    }
    Ok(())
}

fn migrate_legacy_aoi_context(transaction: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
    let legacy = transaction
        .query_row(
            "SELECT region_id, updated_at FROM region_aoi_state ORDER BY region_id LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((region_id, updated_at)) = legacy else {
        return Ok(());
    };
    let workspace_id = "workspace-default";
    let project_id = "project-workspace-default-migrated";
    let context = json!({
        "schema_version": 1,
        "workspace_id": workspace_id,
        "workspace_root": "",
        "workspace_name": "Migrated Workspace",
        "project_id": project_id,
        "project_name": "Migrated Project",
        "region_id": region_id,
        "region_name": "Migrated Region",
        "revision": 0,
        "updated_at": updated_at,
        "projects": [{
            "workspace_id": workspace_id,
            "project_id": project_id,
            "project_name": "Migrated Project",
            "safe_name": "migrated-project",
            "regions": [{
                "project_id": project_id,
                "region_id": region_id,
                "region_name": "Migrated Region",
                "safe_name": "migrated-region"
            }]
        }],
        "regions": [{
            "project_id": project_id,
            "region_id": region_id,
            "region_name": "Migrated Region",
            "safe_name": "migrated-region"
        }]
    })
    .to_string();
    transaction.execute(
        "INSERT INTO workspace_context(slot, context_json) VALUES (1, ?1) \
         ON CONFLICT(slot) DO NOTHING",
        [context],
    )?;
    Ok(())
}
