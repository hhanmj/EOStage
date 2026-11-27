//! Controlled P0.4A live acceptance for one fixed ALOS-2 archive.
//!
//! The test is ignored by default.  It deliberately uses the production
//! ApplicationService constructor so the catalogue, Store, Scheduler,
//! ProductionWorker, Earthdata resolver, transfer validation, promotion, and
//! reopen path remain the authority under test.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationCatalogSearchAndFreezeRequest, ApplicationService,
    ApplicationServiceConfig,
};
use insar_core::{
    CatalogSearchQuery, DownloadLayout, DownloadOutcome, EffectiveCapabilityState,
    StoreCreateTaskRequest, TaskLifecycleStatus,
};
use insar_download::{
    desktop_earthdata_readiness, desktop_earthdata_status, part_path, EarthdataCredentialReadiness,
};
use insar_provider::asf::query::build_alos2_cmr_query;
use insar_provider::catalog::{
    CollectionProfileRegistry, ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID,
    ALOS2_PROVIDER_ID,
};
use serde_json::{json, Map, Value};

mod common;
mod support;

use support::p04a_evidence::{
    add_object_field, create_external_attempt_dir, evidence_base, geojson_bbox, sha256_hex,
    write_immutable_json,
};

const LIVE_FLAG: &str = "INSAR_P04A_ALOS2_LIVE_ACCEPTANCE";
const CATALOGUE_DIAGNOSTIC_FLAG: &str = "INSAR_P04A_ALOS2_CATALOGUE_DIAGNOSTIC";
const CATALOGUE_QUERY_VARIANT_ENV: &str = "INSAR_P04A_ALOS2_QUERY_VARIANT";
const ATTEMPT_ID_ENV: &str = "INSAR_P04A_ALOS2_ATTEMPT_ID";
const ATTEMPT_DIR_ENV: &str = "INSAR_P04A_ALOS2_ATTEMPT_DIR";
const DESTINATION_ENV: &str = "INSAR_P04A_ALOS2_DESTINATION_DIR";
const TASK_ID: &str = "p04a-alos2-live-task";
const BATCH_ID: &str = "p04a-alos2-live-batch";
const RESOURCE_ID: &str = "science.alos-2.palsar-2.scansar-l1.1.v1";
const CAPABILITY_ID: &str = "capability.alos2-scansar-l1.1.full-archive.v1";
const PROVIDER_ID: &str = ALOS2_PROVIDER_ID;
const PRODUCT_ID: &str = "G3463311647-ASF";
const PRODUCT_NAME: &str = "ALOS2478123150-230331-WBDR1.1__D";
const FILE_NAME: &str = "ALOS2478123150-230331-WBDR1.1__D.zip";
const EXPECTED_SIZE_BYTES: u64 = 60_018_405_388;
const EXPECTED_MD5: &str = "0f558597e19d964f24ca3a4bff11696c";
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

struct EnvironmentCleanup;

impl Drop for EnvironmentCleanup {
    fn drop(&mut self) {
        for name in [
            LIVE_FLAG,
            CATALOGUE_DIAGNOSTIC_FLAG,
            CATALOGUE_QUERY_VARIANT_ENV,
            ATTEMPT_ID_ENV,
            ATTEMPT_DIR_ENV,
            DESTINATION_ENV,
        ] {
            std::env::remove_var(name);
        }
    }
}

struct TempStore {
    path: PathBuf,
}

impl TempStore {
    fn new() -> Self {
        Self {
            path: std::env::temp_dir().join(format!(
                "insar-p04a-alos2-production-live-{}.sqlite3",
                std::process::id()
            )),
        }
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.path.display(), suffix));
        }
    }
}

struct LiveInput {
    attempt_id: String,
    attempt_dir: PathBuf,
    destination: PathBuf,
    credential_readiness: EarthdataCredentialReadiness,
    credential_status: String,
}

fn required_environment(name: &str) -> Result<String, &'static str> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty() && value == value.trim())
        .ok_or("LIVE_INPUT_REQUIRED")
}

fn valid_attempt_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn repo_root() -> Result<PathBuf, &'static str> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .ok_or("LIVE_DESTINATION_UNSAFE")?
        .canonicalize()
        .map_err(|_| "LIVE_DESTINATION_UNSAFE")
}

fn external_writable_destination(value: String) -> Result<PathBuf, &'static str> {
    let destination = PathBuf::from(value)
        .canonicalize()
        .map_err(|_| "LIVE_DESTINATION_UNSAFE")?;
    if !destination.is_dir() || destination.starts_with(repo_root()?) {
        return Err("LIVE_DESTINATION_UNSAFE");
    }
    let probe = destination.join(format!(
        ".insar-p04a-alos2-write-probe-{}",
        std::process::id()
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|_| "LIVE_DESTINATION_UNSAFE")?;
    drop(file);
    fs::remove_file(probe).map_err(|_| "LIVE_DESTINATION_UNSAFE")?;
    Ok(destination)
}

fn live_input() -> Result<LiveInput, &'static str> {
    if std::env::var(LIVE_FLAG).ok().as_deref() != Some("1") {
        return Err("LIVE_INPUT_REQUIRED");
    }
    let attempt_id = required_environment(ATTEMPT_ID_ENV)?;
    if !valid_attempt_id(&attempt_id) {
        return Err("LIVE_INPUT_INVALID");
    }
    let destination = external_writable_destination(required_environment(DESTINATION_ENV)?)?;
    let attempt_dir = create_external_attempt_dir(required_environment(ATTEMPT_DIR_ENV)?)?;
    let credential_readiness = desktop_earthdata_readiness();
    let credential_status = desktop_earthdata_status();
    if credential_readiness != EarthdataCredentialReadiness::Ready {
        let mut failure = evidence_base(
            RESOURCE_ID,
            CAPABILITY_ID,
            PROVIDER_ID,
            PRODUCT_ID,
            &attempt_id,
        );
        add_object_field(&mut failure, "attempt_status", json!("stopped"));
        add_object_field(
            &mut failure,
            "failure_code",
            json!("EARTHDATA_CREDENTIAL_NOT_READY"),
        );
        add_object_field(
            &mut failure,
            "credential_route_class",
            json!("earthdata.auto.netrc-keyring-env"),
        );
        add_object_field(
            &mut failure,
            "credential_readiness",
            json!(match credential_readiness {
                EarthdataCredentialReadiness::Ready => "ready",
                EarthdataCredentialReadiness::Missing => "missing",
                EarthdataCredentialReadiness::Invalid => "invalid",
                EarthdataCredentialReadiness::Unavailable => "unavailable",
            }),
        );
        add_object_field(
            &mut failure,
            "failure_observation",
            json!({
                "network_started": false,
                "credential_material_written": false,
                "credential_values_logged": false,
            }),
        );
        let _ = write_immutable_json(&attempt_dir, "failure.json", &failure);
        return Err("EARTHDATA_CREDENTIAL_NOT_READY");
    }
    Ok(LiveInput {
        attempt_id,
        attempt_dir,
        destination,
        credential_readiness,
        credential_status,
    })
}

fn aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[-114.0, 21.0], [-109.0, 21.0], [-109.0, 26.0], [-114.0, 26.0], [-114.0, 21.0]]]
    })
    .as_object()
    .expect("fixed ALOS-2 AOI is an object")
    .clone()
}

fn bind_aoi(service: &ApplicationService) -> Result<(), &'static str> {
    bind_aoi_geometry(service, aoi())
}

fn bind_aoi_geometry(
    service: &ApplicationService,
    geometry: Map<String, Value>,
) -> Result<(), &'static str> {
    let region_id = common::ensure_fixture_region(service, "alos2-live");
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "p04a-alos2-bind-aoi".to_owned(),
            region_id,
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry: geometry.clone(),
            bbox: geojson_bbox(&geometry)?,
            crs: "EPSG:4326".to_owned(),
            display_name: "P0.4A fixed ALOS-2 acceptance AOI".to_owned(),
            bound_at: "2026-08-10T00:00:00Z".to_owned(),
        })
        .map(|_| ())
        .map_err(|_| "LIVE_AOI_BIND_FAILED")
}

fn query() -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
        aoi_geojson: Some(aoi()),
        cursor: None,
        result_limit: Some(24),
        filters: json!({
            "datetime": {"start": "2023-03-31T00:00:00Z", "end": "2023-04-01T00:00:00Z"},
            "beam_mode": "WBD",
            "polarizations": ["HH", "HV"],
            "flight_direction": "DESCENDING",
            "look_direction": "RIGHT",
            "path": 161,
            "frame": 3150
        })
        .as_object()
        .expect("fixed ALOS-2 query filters are an object")
        .clone(),
    }
}

fn discovery_query() -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
        aoi_geojson: Some(aoi()),
        cursor: None,
        result_limit: Some(24),
        filters: json!({
            "datetime": {"start": "2023-03-31T00:00:00Z", "end": "2023-04-01T00:00:00Z"},
            "beam_mode": "WBD",
            "flight_direction": "DESCENDING",
            "look_direction": "RIGHT",
            "path": 161,
            "frame": 3150
        })
        .as_object()
        .expect("discovery query filters are an object")
        .clone(),
    }
}

fn fixed_known_product_hole_discovery_query() -> CatalogSearchQuery {
    let mut query = discovery_query();
    query.aoi_geojson = Some(
        json!({
            "type": "Polygon",
            "coordinates": [
                [[-114.0, 21.0], [-109.0, 21.0], [-109.0, 26.0], [-114.0, 26.0], [-114.0, 21.0]],
                [[-113.9, 25.5], [-113.5, 25.5], [-113.5, 25.8], [-113.9, 25.8], [-113.9, 25.5]]
            ]
        })
        .as_object()
        .expect("fixed ALOS-2 hole AOI is an object")
        .clone(),
    );
    query
}

fn fixed_known_product_multipolygon_discovery_query() -> CatalogSearchQuery {
    let mut query = discovery_query();
    query.aoi_geojson = Some(
        json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[-114.0, 21.0], [-109.0, 21.0], [-109.0, 26.0], [-114.0, 26.0], [-114.0, 21.0]]],
                [[[10.0, 10.0], [10.5, 10.0], [10.5, 10.5], [10.0, 10.5], [10.0, 10.0]]]
            ]
        })
        .as_object()
        .expect("fixed ALOS-2 MultiPolygon AOI is an object")
        .clone(),
    );
    query
}

fn owner_tianjin_unrestricted_discovery_query() -> CatalogSearchQuery {
    let geometry = json!({
        "type": "Polygon",
        "coordinates": [[[116.7, 38.55], [118.1, 38.55], [118.1, 40.25], [116.7, 40.25], [116.7, 38.55]]]
    })
    .as_object()
    .expect("fixed Tianjin diagnostic AOI is an object")
    .clone();
    CatalogSearchQuery {
        logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
        aoi_geojson: Some(geometry),
        cursor: None,
        result_limit: Some(30),
        filters: Map::new(),
    }
}

fn owner_china_unrestricted_discovery_query() -> CatalogSearchQuery {
    let geometry = json!({
        "type": "Polygon",
        "coordinates": [[[73.0, 18.0], [135.0, 18.0], [135.0, 54.0], [73.0, 54.0], [73.0, 18.0]]]
    })
    .as_object()
    .expect("fixed China diagnostic AOI is an object")
    .clone();
    CatalogSearchQuery {
        logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
        aoi_geojson: Some(geometry),
        cursor: None,
        result_limit: Some(30),
        filters: Map::new(),
    }
}

fn translated_cmr_semantics(search: &CatalogSearchQuery) -> Result<Value, &'static str> {
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .ok_or("LIVE_CMR_PROFILE_MISSING")?;
    let plan = build_alos2_cmr_query(search, profile).map_err(|_| "LIVE_CMR_TRANSLATION_FAILED")?;
    let common_params = plan
        .common_params
        .iter()
        .map(|(key, value)| json!({"key": key, "value": value}))
        .collect::<Vec<_>>();
    Ok(json!({
        "collection_concept_id": ALOS2_NATIVE_COLLECTION_ID,
        "common_params": common_params,
        "polygon_count": plan.polygons.len(),
        "page_size": plan.provider_page_size,
        "is_multi_polygon": plan.is_multi_polygon,
        "complete_batch_local_filters": plan.complete_batch_local_filters,
        "spatial_transport_mode": plan.spatial_transport_mode,
        "requires_exact_spatial_post_filter": plan.requires_exact_spatial_post_filter,
    }))
}

fn catalogue_candidate_evidence(candidate: &insar_core::SnapshotItem) -> Value {
    let archive = candidate.asset_access.as_ref().map(|access| {
        json!({
            "file_name": access.file_name,
            "size_bytes": access.expected_size_bytes,
            "checksum_algorithm": access.checksum_algorithm,
            "checksum_value": access.checksum_value,
        })
    });
    json!({
        "item_id": candidate.item_id,
        "provider_product_id": candidate.metadata.get("provider_product_id"),
        "display_name": candidate.display_name,
        "acquisition_datetime": candidate.acquisition_datetime,
        "product_family": candidate.metadata.get("product_family"),
        "processing_level": candidate.metadata.get("processing_level"),
        "properties": candidate.metadata.get("properties"),
        "archive": archive,
    })
}

fn credential_readiness_label(value: EarthdataCredentialReadiness) -> &'static str {
    match value {
        EarthdataCredentialReadiness::Ready => "ready",
        EarthdataCredentialReadiness::Missing => "missing",
        EarthdataCredentialReadiness::Invalid => "invalid",
        EarthdataCredentialReadiness::Unavailable => "unavailable",
    }
}

fn target_path(destination: &Path) -> PathBuf {
    destination
        .join(ALOS2_LOGICAL_DATASET.to_ascii_uppercase())
        .join("1_1")
        .join(FILE_NAME)
}

fn verify_selected_candidate(
    selected: &insar_core::SnapshotItem,
) -> Result<(u64, String, String), &'static str> {
    if selected.provider_id != PROVIDER_ID
        || selected.item_id.is_empty()
        || selected.display_name != PRODUCT_NAME
        || selected
            .metadata
            .get("provider_product_id")
            .and_then(Value::as_str)
            != Some(PRODUCT_ID)
        || selected.metadata.get("dataset").and_then(Value::as_str) != Some(ALOS2_LOGICAL_DATASET)
        || selected
            .metadata
            .get("product_family")
            .and_then(Value::as_str)
            != Some("ALOS2_L1_PSR2")
        || selected
            .metadata
            .get("processing_level")
            .and_then(Value::as_str)
            != Some("1.1")
    {
        return Err("LIVE_CATALOGUE_IDENTITY_REJECTED");
    }
    let properties = selected
        .metadata
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("LIVE_CATALOGUE_IDENTITY_REJECTED")?;
    for (key, expected) in [
        ("platform", "ALOS-2"),
        ("instrument", "PALSAR-2"),
        ("processing_level", "1.1"),
        ("collection_short_name", "ALOS2_L1_PSR2"),
        ("beam_mode", "WBD"),
        ("flight_direction", "DESCENDING"),
        ("look_direction", "RIGHT"),
    ] {
        if properties.get(key).and_then(Value::as_str) != Some(expected) {
            return Err("LIVE_CATALOGUE_IDENTITY_REJECTED");
        }
    }
    if properties.get("polarizations") != Some(&json!(["HH", "HV"]))
        || properties.get("path").and_then(Value::as_u64) != Some(161)
        || properties.get("frame").and_then(Value::as_u64) != Some(3150)
    {
        return Err("LIVE_CATALOGUE_IDENTITY_REJECTED");
    }
    let assets = selected
        .metadata
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("LIVE_ASSET_SET_REJECTED")?;
    if assets.len() != 1
        || assets[0].get("role").and_then(Value::as_str) != Some("primary")
        || assets[0].get("native_roles") != Some(&json!(["archive"]))
    {
        return Err("LIVE_ASSET_SET_REJECTED");
    }
    let access = selected
        .asset_access
        .as_ref()
        .ok_or("LIVE_ASSET_ACCESS_REJECTED")?;
    let file_name = access
        .file_name
        .as_deref()
        .ok_or("LIVE_ASSET_ACCESS_REJECTED")?;
    let stable_href = access
        .descriptor
        .stable_href
        .as_deref()
        .ok_or("LIVE_ASSET_ACCESS_REJECTED")?;
    let checksum_algorithm = access
        .checksum_algorithm
        .as_deref()
        .ok_or("LIVE_ASSET_ACCESS_REJECTED")?;
    let checksum_value = access
        .checksum_value
        .as_deref()
        .ok_or("LIVE_ASSET_ACCESS_REJECTED")?;
    if file_name != FILE_NAME
        || access.expected_size_bytes != Some(EXPECTED_SIZE_BYTES)
        || access.role != insar_core::AssetRole::Primary
        || access.descriptor.resolver_id != "earthdata_https"
        || access.descriptor.scheme != insar_core::AssetAccessScheme::EarthdataHttps
        || access.descriptor.credential_profile.as_deref() != Some("earthdata-default")
        || checksum_algorithm != "MD5"
        || checksum_value != EXPECTED_MD5
        || stable_href.rsplit('/').next() != Some(FILE_NAME)
    {
        return Err("LIVE_ASSET_ACCESS_REJECTED");
    }
    Ok((
        EXPECTED_SIZE_BYTES,
        checksum_algorithm.to_owned(),
        stable_href.to_owned(),
    ))
}

fn wait_for_completed(
    service: &ApplicationService,
    item_id: &str,
) -> Result<insar_core::DownloadItemResult, TransferDiagnostic> {
    let deadline = Instant::now() + TRANSFER_TIMEOUT;
    loop {
        let snapshot = service
            .download_snapshot(TASK_ID)
            .map_err(|_| TransferDiagnostic::snapshot_failed())?;
        if let Some(result) = snapshot
            .results
            .iter()
            .find(|result| result.item_id == item_id)
        {
            if result.outcome == DownloadOutcome::Completed {
                return Ok(result.clone());
            }
            return Err(TransferDiagnostic::from_result(result));
        }
        if Instant::now() >= deadline {
            return Err(TransferDiagnostic::timeout());
        }
        thread::sleep(Duration::from_secs(1));
    }
}

struct TransferDiagnostic {
    code: &'static str,
    failure_code: Option<String>,
    failure_reason: Option<String>,
    retryable: Option<bool>,
    bytes_written: u64,
    attempt_count: u64,
    part_present: bool,
    part_bytes: Option<u64>,
}

impl TransferDiagnostic {
    fn from_result(result: &insar_core::DownloadItemResult) -> Self {
        let part = part_path(&result.destination);
        Self {
            code: "LIVE_TRANSFER_FAILED",
            failure_code: result.failure.as_ref().map(|failure| failure.code.clone()),
            failure_reason: result
                .failure
                .as_ref()
                .map(|failure| failure.reason_zh.clone()),
            retryable: result.failure.as_ref().map(|failure| failure.retryable),
            bytes_written: result.bytes_written,
            attempt_count: result.attempt_count,
            part_present: part.exists(),
            part_bytes: fs::metadata(part).ok().map(|metadata| metadata.len()),
        }
    }

    fn snapshot_failed() -> Self {
        Self {
            code: "LIVE_TASK_SNAPSHOT_FAILED",
            failure_code: None,
            failure_reason: None,
            retryable: None,
            bytes_written: 0,
            attempt_count: 0,
            part_present: false,
            part_bytes: None,
        }
    }

    fn timeout() -> Self {
        Self {
            code: "LIVE_TRANSFER_TIMEOUT",
            failure_code: None,
            failure_reason: None,
            retryable: Some(true),
            bytes_written: 0,
            attempt_count: 0,
            part_present: false,
            part_bytes: None,
        }
    }
}

fn transfer_diagnostic_value(value: TransferDiagnostic) -> Value {
    json!({
        "worker_failure_code": value.failure_code,
        "worker_failure_reason": value.failure_reason,
        "worker_retryable": value.retryable,
        "bytes_written": value.bytes_written,
        "attempt_count": value.attempt_count,
        "part_present": value.part_present,
        "part_bytes": value.part_bytes,
    })
}

fn run_acceptance(input: LiveInput) -> Result<(), &'static str> {
    let LiveInput {
        attempt_id,
        attempt_dir,
        destination,
        credential_readiness,
        credential_status,
    } = input;
    let store = TempStore::new();
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&store.path));
    let mut transfer_diagnostic = None;
    let result: Result<(), &'static str> = (|| {
        bind_aoi(&service)?;
        let effective = service
            .effective_capability(CAPABILITY_ID)
            .map_err(|_| "LIVE_EFFECTIVE_CAPABILITY_FAILED")?
            .ok_or("LIVE_EFFECTIVE_CAPABILITY_MISSING")?;
        if effective.projection.state != EffectiveCapabilityState::Executable {
            return Err("LIVE_EFFECTIVE_CAPABILITY_BLOCKED");
        }
        let search = query();
        let frozen = service
            .catalog_search_and_freeze_legacy(&ApplicationCatalogSearchAndFreezeRequest {
                request_id: "p04a-alos2-live-search".to_owned(),
                search: search.clone(),
                batch_id: BATCH_ID.to_owned(),
                created_at: "2026-08-10T00:00:00Z".to_owned(),
                aoi_name: "P0.4A fixed ALOS-2 acceptance AOI".to_owned(),
                aoi_geojson: Some(aoi()),
                workbench_page_size: 24,
            })
            .map_err(|_| "LIVE_CATALOGUE_SEARCH_FAILED")?;
        let batch = frozen
            .restore
            .workbench_batch
            .ok_or("LIVE_SEARCH_FREEZE_REJECTED")?;
        let Some(selected) = batch
            .candidates
            .iter()
            .find(|candidate| {
                candidate
                    .metadata
                    .get("provider_product_id")
                    .and_then(Value::as_str)
                    == Some(PRODUCT_ID)
            })
            .cloned()
        else {
            let candidates = batch
                .candidates
                .iter()
                .map(|candidate| {
                    json!({
                        "item_id": candidate.item_id,
                        "provider_product_id": candidate.metadata.get("provider_product_id"),
                        "display_name": candidate.display_name,
                        "acquisition_datetime": candidate.acquisition_datetime,
                        "product_family": candidate.metadata.get("product_family"),
                        "processing_level": candidate.metadata.get("processing_level"),
                        "properties": candidate.metadata.get("properties"),
                    })
                })
                .collect::<Vec<_>>();
            let mut catalogue = evidence_base(
                RESOURCE_ID,
                CAPABILITY_ID,
                PROVIDER_ID,
                PRODUCT_ID,
                &attempt_id,
            );
            add_object_field(
                &mut catalogue,
                "attempt_status",
                json!("catalogue_rejected"),
            );
            add_object_field(
                &mut catalogue,
                "search_query",
                serde_json::to_value(&search).map_err(|_| "LIVE_EVIDENCE_SERIALIZATION_FAILED")?,
            );
            add_object_field(
                &mut catalogue,
                "candidate_count",
                json!(batch.candidates.len()),
            );
            add_object_field(&mut catalogue, "candidates", json!(candidates));
            write_immutable_json(&attempt_dir, "catalogue.json", &catalogue)
                .map_err(|_| "LIVE_EVIDENCE_WRITE_FAILED")?;
            return Err("LIVE_FIXED_PRODUCT_NOT_FOUND");
        };
        let (expected_size, checksum_algorithm, stable_href) =
            verify_selected_candidate(&selected)?;
        let final_path = target_path(&destination);
        if final_path.exists() || part_path(&final_path).exists() {
            return Err("LIVE_DESTINATION_OCCUPIED");
        }

        let mut freeze = evidence_base(
            RESOURCE_ID,
            CAPABILITY_ID,
            PROVIDER_ID,
            PRODUCT_ID,
            &attempt_id,
        );
        add_object_field(
            &mut freeze,
            "attempt_status",
            json!("frozen_before_transfer"),
        );
        add_object_field(
            &mut freeze,
            "provider_product_name",
            json!(selected.display_name),
        );
        add_object_field(
            &mut freeze,
            "acquisition_identity",
            json!({
                "acquisition_datetime": selected.acquisition_datetime,
                "provider_product_id": PRODUCT_ID,
                "native_collection_id": ALOS2_NATIVE_COLLECTION_ID,
                "product_type": "ALOS2_L1_PSR2",
                "processing_level": "1.1",
                "platform": "ALOS-2",
                "instrument": "PALSAR-2",
                "beam_mode": "WBD",
                "polarizations": ["HH", "HV"],
                "flight_direction": "DESCENDING",
                "look_direction": "RIGHT",
                "path": 161,
                "frame": 3150,
            }),
        );
        add_object_field(
            &mut freeze,
            "search_query",
            serde_json::to_value(&search).map_err(|_| "LIVE_EVIDENCE_SERIALIZATION_FAILED")?,
        );
        add_object_field(
            &mut freeze,
            "aoi_geojson",
            serde_json::to_value(&batch.aoi_geojson)
                .map_err(|_| "LIVE_EVIDENCE_SERIALIZATION_FAILED")?,
        );
        add_object_field(
            &mut freeze,
            "expected_asset",
            json!({
                "asset_key": selected.asset_access.as_ref().map(|access| access.reference.asset_key.as_str()),
                "role": "primary",
                "native_role": "archive",
                "file_name": FILE_NAME,
                "media_type": "application/zip",
                "expected_size_bytes": expected_size,
                "checksum_algorithm": checksum_algorithm,
                "checksum_value": EXPECTED_MD5,
            }),
        );
        add_object_field(
            &mut freeze,
            "source_identity",
            json!({
                "frozen": true,
                "stable_locator_sha256": sha256_hex(&stable_href),
                "resolver_policy_id": selected.metadata.get("resolver_policy_id"),
                "credential_route_class": "earthdata.auto.netrc-keyring-env",
                "credential_readiness": credential_readiness_label(credential_readiness),
                "credential_source_status": credential_status.clone(),
            }),
        );
        add_object_field(
            &mut freeze,
            "effective_capability",
            serde_json::to_value(&effective).map_err(|_| "LIVE_EVIDENCE_SERIALIZATION_FAILED")?,
        );
        add_object_field(
            &mut freeze,
            "task_identity",
            json!({
                "task_id": TASK_ID,
                "batch_id": BATCH_ID,
                "item_id": selected.item_id,
                "layout": "sar_dataset_processing",
                "concurrency": 1,
            }),
        );
        write_immutable_json(&attempt_dir, "freeze.json", &freeze)
            .map_err(|_| "LIVE_EVIDENCE_WRITE_FAILED")?;

        service
            .create_download_task_legacy(&StoreCreateTaskRequest {
                task_id: TASK_ID.to_owned(),
                batch_id: BATCH_ID.to_owned(),
                selected_item_ids: vec![selected.item_id.clone()],
                checked_item_ids: vec![selected.item_id.clone()],
                page_index: 0,
                page_size: 24,
                output_root: destination,
                layout: DownloadLayout::SarDatasetProcessing,
                concurrency: 1,
                created_at: "2026-08-10T00:00:01Z".to_owned(),
            })
            .map_err(|_| "LIVE_TASK_CREATE_FAILED")?;
        let transfer = match wait_for_completed(&service, &selected.item_id) {
            Ok(transfer) => transfer,
            Err(diagnostic) => {
                transfer_diagnostic =
                    Some((diagnostic.code, transfer_diagnostic_value(diagnostic)));
                return Err("LIVE_TRANSFER_FAILED");
            }
        };
        if transfer.destination != final_path
            || transfer.bytes_written != expected_size
            || fs::metadata(&final_path)
                .map(|metadata| metadata.len())
                .map_err(|_| "LIVE_TRANSFER_INTEGRITY_REJECTED")?
                != expected_size
            || part_path(&final_path).exists()
        {
            return Err("LIVE_TRANSFER_INTEGRITY_REJECTED");
        }
        let store_snapshot = service
            .snapshot(true)
            .map_err(|_| "LIVE_STORE_COMPLETION_FAILED")?;
        let completed = store_snapshot
            .restore
            .runtimes
            .iter()
            .find(|runtime| runtime.task_id == TASK_ID)
            .is_some_and(|runtime| runtime.status == TaskLifecycleStatus::Completed);
        if !completed {
            return Err("LIVE_STORE_COMPLETION_FAILED");
        }
        service
            .shutdown()
            .map_err(|_| "LIVE_SESSION_CLEANUP_FAILED")?;

        let restarted = ApplicationService::new(ApplicationServiceConfig::existing_business_store(
            &store.path,
        ));
        let restored = restarted
            .download_snapshot(TASK_ID)
            .map_err(|_| "LIVE_TASK_RESTORE_FAILED")?;
        let restored_completed = restored.results.iter().any(|item| {
            item.item_id == selected.item_id && item.outcome == DownloadOutcome::Completed
        });
        let restart_shutdown = restarted.shutdown();
        if !restored_completed || restart_shutdown.is_err() {
            return Err("LIVE_TASK_RESTORE_FAILED");
        }

        let mut evidence = freeze;
        add_object_field(&mut evidence, "attempt_status", json!("accepted"));
        add_object_field(
            &mut evidence,
            "validation",
            json!({
                "source_size_enforced": true,
                "source_checksum_algorithm": checksum_algorithm,
                "source_checksum_value": EXPECTED_MD5,
                "checksum_verified_by_production_transfer": true,
                "archive_structure_validation": "not_supported_by_current_validator",
                "final_artifact_name": FILE_NAME,
                "final_artifact_size_bytes": transfer.bytes_written,
                "final_artifact_checksum": {
                    "algorithm": checksum_algorithm,
                    "value": EXPECTED_MD5,
                    "verified": true,
                },
                "part_present_after_promotion": false,
                "atomic_promotion_observed": true,
            }),
        );
        add_object_field(
            &mut evidence,
            "store_lifecycle",
            json!({
                "completed_after_validation": true,
                "runtime_status": "completed",
                "reopen_completed": true,
            }),
        );
        add_object_field(
            &mut evidence,
            "network_observation",
            json!({
                "catalogue_request_count": Value::Null,
                "catalogue_request_count_observation": "not_exposed_by_ApplicationService_Store_v1",
                "remote_bytes_written": transfer.bytes_written,
                "transfer_attempt_count": transfer.attempt_count,
                "transfer_request_count": Value::Null,
                "transfer_request_count_observation": "not_exposed_by_ApplicationService_Store_v1",
            }),
        );
        add_object_field(
            &mut evidence,
            "recovery_observation",
            json!({
                "live_interruption_intentionally_run": false,
                "offline_bounded_recovery_regression": "crates/insar-application-service/tests/asf_alos2_offline_acceptance.rs",
                "offline_complete_zip_regression": "crates/insar-download/tests/asf_alos2_complete_zip.rs",
                "offline_recovery_contract_accepted": true,
            }),
        );
        write_immutable_json(&attempt_dir, "evidence.json", &evidence)
            .map_err(|_| "LIVE_EVIDENCE_WRITE_FAILED")?;
        Ok(())
    })();
    if let Err(code) = result {
        let mut failure = evidence_base(
            RESOURCE_ID,
            CAPABILITY_ID,
            PROVIDER_ID,
            PRODUCT_ID,
            &attempt_id,
        );
        add_object_field(&mut failure, "attempt_status", json!("stopped"));
        add_object_field(&mut failure, "failure_code", json!(code));
        add_object_field(
            &mut failure,
            "credential_route_class",
            json!("earthdata.auto.netrc-keyring-env"),
        );
        add_object_field(
            &mut failure,
            "credential_readiness",
            json!(credential_readiness_label(credential_readiness)),
        );
        add_object_field(
            &mut failure,
            "credential_source_status",
            json!(credential_status),
        );
        add_object_field(
            &mut failure,
            "failure_observation",
            json!({
                "final_artifact_published": false,
                "credential_material_written": false,
                "credential_values_logged": false,
            }),
        );
        if let Some((code, diagnostic)) = transfer_diagnostic {
            add_object_field(&mut failure, "transfer_failure_code", json!(code));
            add_object_field(&mut failure, "transfer_observation", diagnostic);
        }
        let _ = write_immutable_json(&attempt_dir, "failure.json", &failure);
    }
    if !service.is_shutting_down() {
        let _ = service.shutdown();
    }
    result
}

#[test]
#[ignore = "requires explicit opt-in for a no-transfer ALOS-2 catalogue discovery"]
fn controlled_alos2_catalogue_discovery() {
    let _environment_cleanup = EnvironmentCleanup;
    if std::env::var(CATALOGUE_DIAGNOSTIC_FLAG).ok().as_deref() != Some("1") {
        panic!("ALOS2_CATALOGUE_DISCOVERY outcome=stopped code=LIVE_INPUT_REQUIRED");
    }
    let attempt_id = required_environment(ATTEMPT_ID_ENV).expect("discovery attempt id");
    if !valid_attempt_id(&attempt_id) {
        panic!("ALOS2_CATALOGUE_DISCOVERY outcome=stopped code=LIVE_INPUT_INVALID");
    }
    let attempt_dir = create_external_attempt_dir(
        required_environment(ATTEMPT_DIR_ENV).expect("discovery evidence directory"),
    )
    .expect("discovery evidence directory is external and empty");
    let database = std::env::temp_dir().join(format!(
        "insar-p04a-alos2-catalogue-discovery-{}.sqlite3",
        std::process::id()
    ));
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(database));
    let query_variant = std::env::var(CATALOGUE_QUERY_VARIANT_ENV)
        .unwrap_or_else(|_| "fixed-known-product".to_owned());
    let result: Result<(), &'static str> = (|| {
        let search = match query_variant.as_str() {
            "fixed-known-product" => discovery_query(),
            "fixed-known-product-hole" => fixed_known_product_hole_discovery_query(),
            "fixed-known-product-multipolygon" => {
                fixed_known_product_multipolygon_discovery_query()
            }
            "owner-tianjin-unrestricted" => owner_tianjin_unrestricted_discovery_query(),
            "owner-china-unrestricted" => owner_china_unrestricted_discovery_query(),
            _ => return Err("LIVE_QUERY_VARIANT_INVALID"),
        };
        let search_aoi = search
            .aoi_geojson
            .clone()
            .ok_or("LIVE_SEARCH_AOI_REQUIRED")?;
        bind_aoi_geometry(&service, search_aoi.clone())?;
        let translated = translated_cmr_semantics(&search)?;
        let frozen = service
            .catalog_search_and_freeze_legacy(&ApplicationCatalogSearchAndFreezeRequest {
                request_id: "p04a-alos2-catalogue-discovery-search".to_owned(),
                search: search.clone(),
                batch_id: "p04a-alos2-catalogue-discovery-batch".to_owned(),
                created_at: "2026-08-10T00:00:00Z".to_owned(),
                aoi_name: format!("P0.4A ALOS-2 catalogue discovery AOI ({query_variant})"),
                aoi_geojson: Some(search_aoi),
                workbench_page_size: search.result_limit.expect("finite live result limit"),
            })
            .map_err(|error| {
                let code = error.code();
                let mut failure = evidence_base(
                    RESOURCE_ID,
                    CAPABILITY_ID,
                    PROVIDER_ID,
                    "catalogue-discovery",
                    &attempt_id,
                );
                add_object_field(&mut failure, "attempt_status", json!("stopped"));
                add_object_field(&mut failure, "query_variant", json!(query_variant));
                add_object_field(&mut failure, "failure_code", json!(code));
                add_object_field(&mut failure, "provider_error_code", json!(code));
                add_object_field(&mut failure, "application_service_error_code", json!(code));
                add_object_field(&mut failure, "search_query", json!(search));
                add_object_field(&mut failure, "translated_cmr_semantics", translated.clone());
                add_object_field(
                    &mut failure,
                    "credential_observation",
                    json!({
                        "credential_material_written": false,
                        "credential_values_logged": false,
                    }),
                );
                let _ = write_immutable_json(&attempt_dir, "failure.json", &failure);
                code
            })?;
        let batch = frozen
            .restore
            .workbench_batch
            .ok_or("LIVE_SEARCH_FREEZE_REJECTED")?;
        let candidates = batch
            .candidates
            .iter()
            .map(catalogue_candidate_evidence)
            .collect::<Vec<_>>();
        let mut evidence = evidence_base(
            RESOURCE_ID,
            CAPABILITY_ID,
            PROVIDER_ID,
            "catalogue-discovery",
            &attempt_id,
        );
        add_object_field(
            &mut evidence,
            "attempt_status",
            json!("catalogue_diagnostic"),
        );
        add_object_field(&mut evidence, "query_variant", json!(query_variant));
        add_object_field(
            &mut evidence,
            "search_query",
            serde_json::to_value(&search).map_err(|_| "LIVE_EVIDENCE_SERIALIZATION_FAILED")?,
        );
        add_object_field(&mut evidence, "translated_cmr_semantics", translated);
        add_object_field(&mut evidence, "candidate_count", json!(candidates.len()));
        add_object_field(&mut evidence, "candidates", json!(candidates));
        add_object_field(
            &mut evidence,
            "transfer_observation",
            json!({
                "transfer_started": false,
                "store_task_created": false,
                "credential_material_written": false,
                "credential_values_logged": false,
            }),
        );
        write_immutable_json(&attempt_dir, "catalogue.json", &evidence)
            .map_err(|_| "LIVE_EVIDENCE_WRITE_FAILED")?;
        eprintln!(
            "ALOS2_CATALOGUE_DISCOVERY outcome=completed candidate_count={}",
            candidates.len()
        );
        Ok(())
    })();
    let _ = service.shutdown();
    if let Err(code) = result {
        panic!("ALOS2_CATALOGUE_DISCOVERY outcome=stopped code={code}");
    }
}

#[test]
#[ignore = "requires explicit opt-in, fixed ALOS-2 target, Earthdata readiness, and a real archive transfer"]
fn controlled_production_alos2_acceptance() {
    let _environment_cleanup = EnvironmentCleanup;
    match live_input().and_then(run_acceptance) {
        Ok(()) => eprintln!(
            "ALOS2_PRODUCTION_ACCEPTANCE outcome=completed provider_product_id={} product_name={} size_bytes={} checksum_algorithm=MD5 checksum_verified=true store_reopen=true credential_values_logged=false",
            PRODUCT_ID, PRODUCT_NAME, EXPECTED_SIZE_BYTES
        ),
        Err(code) => panic!("ALOS2_PRODUCTION_ACCEPTANCE outcome=stopped code={code}"),
    }
}

#[test]
fn fixed_alos2_target_contract_is_explicit() {
    assert_eq!(RESOURCE_ID, "science.alos-2.palsar-2.scansar-l1.1.v1");
    assert_eq!(
        CAPABILITY_ID,
        "capability.alos2-scansar-l1.1.full-archive.v1"
    );
    assert_eq!(PRODUCT_ID, "G3463311647-ASF");
    assert_eq!(PRODUCT_NAME, "ALOS2478123150-230331-WBDR1.1__D");
    assert_eq!(FILE_NAME, "ALOS2478123150-230331-WBDR1.1__D.zip");
    assert_eq!(EXPECTED_SIZE_BYTES, 60_018_405_388);
    assert_eq!(EXPECTED_MD5, "0f558597e19d964f24ca3a4bff11696c");
    assert_eq!(ALOS2_PROFILE_ID, "asf.alos-2.palsar-2.scansar.l1.1.v1");
}

#[test]
fn catalogue_discovery_contract_includes_path_for_real_integer_verification() {
    let search = discovery_query();
    assert_eq!(
        search.filters.get("path").and_then(Value::as_u64),
        Some(161)
    );
    assert_eq!(
        search.filters.get("frame").and_then(Value::as_u64),
        Some(3150)
    );
}

#[test]
fn translated_cmr_semantics_encode_path_and_frame_as_integers() {
    let semantics = translated_cmr_semantics(&discovery_query()).expect("fixed query translates");
    let params = semantics["common_params"]
        .as_array()
        .expect("translated common params");
    assert!(params
        .iter()
        .any(|param| { param["key"] == "attribute[]" && param["value"] == "int,PATH_NUMBER,161" }));
    assert!(params.iter().any(|param| {
        param["key"] == "attribute[]" && param["value"] == "int,FRAME_NUMBER,3150"
    }));
}
