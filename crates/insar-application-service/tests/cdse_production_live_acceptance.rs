//! Explicit, session-only production acceptance for one CDSE Sentinel-2 archive.
//!
//! This test remains ignored in ordinary suites. Its launcher supplies a
//! short-lived user session through a current-user, one-shot local pipe; the
//! test neither accepts an access token nor writes credentials to the Store.

use std::fs::{self, OpenOptions};
#[cfg(windows)]
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationCatalogSearchAndFreezeRequest, ApplicationService,
    ApplicationServiceConfig, ApplicationServiceError, CdseLoginRequest, CdseNetworkMode,
    CdseQuicklookRequest,
};
use insar_core::{
    CatalogSearchQuery, ContractError, DownloadLayout, DownloadOutcome, EffectiveCapabilityState,
    StoreCreateTaskRequest, TaskLifecycleStatus,
};
use insar_download::part_path;
use insar_store::StoreWorkbenchPageRequest;
use serde::de::Visitor;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Map, Value};
use uuid::Uuid;
use zeroize::Zeroizing;

mod common;
mod support;

use support::p04a_evidence::{
    add_object_field, create_external_attempt_dir, evidence_base, geojson_bbox, sha256_hex,
    write_immutable_json,
};

const LIVE_ACCEPTANCE_FLAG: &str = "INSAR_CDSE_LIVE_ACCEPTANCE";
const LIVE_ANONYMOUS_SEARCH_ACCEPTANCE_FLAG: &str = "INSAR_CDSE_ANONYMOUS_SEARCH_ACCEPTANCE";
const LIVE_SECRET_PIPE: &str = "INSAR_CDSE_LIVE_SECRET_PIPE";
const LIVE_ATTEMPT_ID: &str = "INSAR_CDSE_LIVE_ATTEMPT_ID";
const LIVE_ATTEMPT_DIR: &str = "INSAR_CDSE_LIVE_ATTEMPT_DIR";
const LIVE_PRODUCT_ID: &str = "INSAR_CDSE_LIVE_PRODUCT_ID";
const LIVE_DESTINATION_DIR: &str = "INSAR_CDSE_LIVE_DESTINATION_DIR";
const LIVE_MAX_BYTES: &str = "INSAR_CDSE_LIVE_MAX_BYTES";
const LIVE_AOI_GEOJSON: &str = "INSAR_CDSE_LIVE_AOI_GEOJSON";
const LIVE_START_UTC: &str = "INSAR_CDSE_LIVE_START_UTC";
const LIVE_END_UTC: &str = "INSAR_CDSE_LIVE_END_UTC";
const LIVE_TASK_ID: &str = "cdse-live-system-task";
const RESOURCE_ID: &str = "science.sentinel-2.l2a.v1";
const CAPABILITY_ID: &str = "capability.sentinel-2.l2a.full-archive.v1";
const PROVIDER_ID: &str = "copernicus.dataspace.odata";
const S2_ACCEPTANCE_GATE: &str = "P0.4A-S2";
const MAX_ACCEPTED_ARCHIVE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const LIVE_TRANSFER_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
#[cfg(windows)]
const LIVE_SECRET_PIPE_TIMEOUT: Duration = Duration::from_secs(45);
#[cfg(windows)]
const MAX_LIVE_SECRET_BYTES: u64 = 16 * 1024;
const MAX_LIVE_SECRET_FIELD_BYTES: usize = 4 * 1024;

struct EnvironmentCleanup;

impl Drop for EnvironmentCleanup {
    fn drop(&mut self) {
        for name in [
            LIVE_ACCEPTANCE_FLAG,
            LIVE_ANONYMOUS_SEARCH_ACCEPTANCE_FLAG,
            LIVE_SECRET_PIPE,
            LIVE_ATTEMPT_ID,
            LIVE_ATTEMPT_DIR,
            LIVE_PRODUCT_ID,
            LIVE_DESTINATION_DIR,
            LIVE_MAX_BYTES,
            LIVE_AOI_GEOJSON,
            LIVE_START_UTC,
            LIVE_END_UTC,
            "INSAR_CDSE_LIVE_ACCESS_TOKEN",
        ] {
            std::env::remove_var(name);
        }
    }
}

/// Deserializes the one-shot local pipe payload directly into zeroizing
/// storage. The public desktop bridge remains the only production login path.
struct LiveSecret(Zeroizing<String>);

impl<'de> Deserialize<'de> for LiveSecret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SecretVisitor;

        impl<'de> Visitor<'de> for SecretVisitor {
            type Value = Zeroizing<String>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a bounded one-shot CDSE secret string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Zeroizing::new(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Zeroizing::new(value))
            }
        }

        deserializer.deserialize_string(SecretVisitor).map(Self)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveSecretInput {
    username: LiveSecret,
    password: LiveSecret,
    #[serde(default)]
    totp: Option<LiveSecret>,
}

impl LiveSecretInput {
    fn into_login_request(mut self) -> CdseLoginRequest {
        let username = std::mem::take(&mut *self.username.0);
        let password = std::mem::take(&mut *self.password.0);
        let totp = self
            .totp
            .as_mut()
            .map(|value| std::mem::take(&mut *value.0))
            .filter(|value| !value.is_empty());
        CdseLoginRequest {
            username,
            password,
            totp,
        }
    }

    fn valid(&self) -> bool {
        let username = self.username.0.as_str();
        let password = self.password.0.as_str();
        let valid_totp = self.totp.as_ref().is_none_or(|value| {
            let value = value.0.as_str();
            value.is_empty()
                || ((6..=12).contains(&value.len())
                    && value.as_bytes().iter().all(u8::is_ascii_digit))
        });
        !username.is_empty()
            && !username.trim().is_empty()
            && username.len() <= MAX_LIVE_SECRET_FIELD_BYTES
            && !username.chars().any(char::is_control)
            && !password.is_empty()
            && password.len() <= MAX_LIVE_SECRET_FIELD_BYTES
            && !password.chars().any(char::is_control)
            && valid_totp
    }
}

struct TempStore {
    path: PathBuf,
}

impl TempStore {
    fn new() -> Self {
        Self {
            path: std::env::temp_dir().join(format!(
                "insar-cdse-production-live-{}-{}.sqlite3",
                std::process::id(),
                std::thread::current().name().unwrap_or("acceptance")
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
    secret: LiveSecretInput,
    attempt_id: String,
    attempt_dir: PathBuf,
    product_id: String,
    aoi_geojson: Map<String, Value>,
    start_utc: String,
    end_utc: String,
    destination: PathBuf,
    max_bytes: u64,
}

struct AnonymousSearchInput {
    product_id: Option<String>,
    aoi_geojson: Map<String, Value>,
    start_utc: String,
    end_utc: String,
}

fn valid_attempt_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_pipe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(windows)]
fn decode_live_secret_pipe(pipe_name: String) -> Result<LiveSecretInput, &'static str> {
    let path = format!(r"\\.\pipe\{pipe_name}");
    let pipe = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|_| "LIVE_SECRET_TRANSPORT_UNAVAILABLE")?;
    let mut bytes = Zeroizing::new(Vec::new());
    pipe.take(MAX_LIVE_SECRET_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "LIVE_SECRET_TRANSPORT_UNAVAILABLE")?;
    if bytes.len() as u64 > MAX_LIVE_SECRET_BYTES {
        return Err("LIVE_SECRET_TRANSPORT_INVALID");
    }
    let secret = serde_json::from_slice::<LiveSecretInput>(&bytes)
        .map_err(|_| "LIVE_SECRET_TRANSPORT_INVALID")?;
    secret
        .valid()
        .then_some(secret)
        .ok_or("LIVE_SECRET_TRANSPORT_INVALID")
}

#[cfg(windows)]
fn read_live_secret_pipe() -> Result<LiveSecretInput, &'static str> {
    let pipe_name = required_environment(LIVE_SECRET_PIPE)?;
    if !valid_pipe_name(&pipe_name) {
        return Err("LIVE_SECRET_TRANSPORT_INVALID");
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(decode_live_secret_pipe(pipe_name));
    });
    receiver
        .recv_timeout(LIVE_SECRET_PIPE_TIMEOUT)
        .map_err(|_| "LIVE_SECRET_TRANSPORT_UNAVAILABLE")?
}

#[cfg(not(windows))]
fn read_live_secret_pipe() -> Result<LiveSecretInput, &'static str> {
    Err("LIVE_SECRET_TRANSPORT_UNAVAILABLE")
}

struct LiveReport {
    product_id: String,
    product_name: String,
    checksum_algorithm: String,
    checksum_value: String,
    content_length: u64,
    final_bytes: u64,
    quicklook_fetched: bool,
    quicklook_bytes: u64,
}

fn required_environment(name: &str) -> Result<String, &'static str> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty() && value == value.trim())
        .ok_or("LIVE_INPUT_REQUIRED")
}

fn canonical_product_id(value: String) -> Result<String, &'static str> {
    let parsed = Uuid::parse_str(&value).map_err(|_| "LIVE_INPUT_INVALID")?;
    let canonical = parsed.hyphenated().to_string();
    (canonical == value)
        .then_some(canonical)
        .ok_or("LIVE_INPUT_INVALID")
}

fn valid_utc_range(start: &str, end: &str) -> bool {
    let start = match DateTime::parse_from_rfc3339(start) {
        Ok(value) if value.offset().local_minus_utc() == 0 => value,
        _ => return false,
    };
    let end = match DateTime::parse_from_rfc3339(end) {
        Ok(value) if value.offset().local_minus_utc() == 0 => value,
        _ => return false,
    };
    start < end
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
    let probe = destination.join(format!(".insar-cdse-write-probe-{}", std::process::id()));
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
    if std::env::var(LIVE_ACCEPTANCE_FLAG).ok().as_deref() != Some("1") {
        return Err("LIVE_INPUT_REQUIRED");
    }
    let aoi_geojson = serde_json::from_str::<Value>(&required_environment(LIVE_AOI_GEOJSON)?)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or("LIVE_INPUT_INVALID")?;
    let max_bytes = required_environment(LIVE_MAX_BYTES)?
        .parse::<u64>()
        .ok()
        .filter(|value| (1..=MAX_ACCEPTED_ARCHIVE_BYTES).contains(value))
        .ok_or("LIVE_INPUT_INVALID")?;
    let product_id = canonical_product_id(required_environment(LIVE_PRODUCT_ID)?)?;
    let start_utc = required_environment(LIVE_START_UTC)?;
    let end_utc = required_environment(LIVE_END_UTC)?;
    if !valid_utc_range(&start_utc, &end_utc) {
        return Err("LIVE_INPUT_INVALID");
    }
    let destination = external_writable_destination(required_environment(LIVE_DESTINATION_DIR)?)?;
    let attempt_id = required_environment(LIVE_ATTEMPT_ID)?;
    if !valid_attempt_id(&attempt_id) {
        return Err("LIVE_INPUT_INVALID");
    }
    // Read the one-shot secret only after every user-controllable nonsecret
    // input has passed validation, so malformed launch input never consumes it.
    let secret = read_live_secret_pipe()?;
    let attempt_dir = create_external_attempt_dir(required_environment(LIVE_ATTEMPT_DIR)?)?;
    Ok(LiveInput {
        secret,
        attempt_id,
        attempt_dir,
        product_id,
        aoi_geojson,
        start_utc,
        end_utc,
        destination,
        max_bytes,
    })
}

fn anonymous_search_input() -> Result<AnonymousSearchInput, &'static str> {
    if std::env::var(LIVE_ANONYMOUS_SEARCH_ACCEPTANCE_FLAG)
        .ok()
        .as_deref()
        != Some("1")
    {
        return Err("LIVE_INPUT_REQUIRED");
    }
    let aoi_geojson = serde_json::from_str::<Value>(&required_environment(LIVE_AOI_GEOJSON)?)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or("LIVE_INPUT_INVALID")?;
    let product_id = std::env::var(LIVE_PRODUCT_ID)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(canonical_product_id)
        .transpose()?;
    let start_utc = required_environment(LIVE_START_UTC)?;
    let end_utc = required_environment(LIVE_END_UTC)?;
    if !valid_utc_range(&start_utc, &end_utc) {
        return Err("LIVE_INPUT_INVALID");
    }
    Ok(AnonymousSearchInput {
        product_id,
        aoi_geojson,
        start_utc,
        end_utc,
    })
}

fn live_query(
    aoi_geojson: Map<String, Value>,
    start_utc: String,
    end_utc: String,
) -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: "optical.sentinel-2.l2a".to_owned(),
        provider_id: "copernicus.dataspace.odata".to_owned(),
        native_collection_id: "SENTINEL-2".to_owned(),
        aoi_geojson: Some(aoi_geojson),
        cursor: None,
        result_limit: Some(24),
        filters: json!({
            "product_type": "S2MSI2A",
            "cloud_cover_lte": 100.0,
            "datetime": {"start": start_utc, "end": end_utc}
        })
        .as_object()
        .expect("fixed live query serializes as an object")
        .clone(),
    }
}

fn bind_processing_aoi(
    service: &ApplicationService,
    geometry: &Map<String, Value>,
    request_id: &str,
    display_name: &str,
) -> Result<(), &'static str> {
    let bbox = geojson_bbox(geometry)?;
    let region_id = common::ensure_fixture_region(service, request_id);
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: request_id.to_owned(),
            region_id,
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry: geometry.clone(),
            bbox,
            crs: "EPSG:4326".to_owned(),
            display_name: display_name.to_owned(),
            bound_at: "2026-08-10T00:00:00Z".to_owned(),
        })
        .map_err(|_| "LIVE_AOI_BIND_FAILED")?;
    Ok(())
}

fn current_source_commit() -> Result<String, &'static str> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|_| "LIVE_SOURCE_COMMIT_UNAVAILABLE")?;
    if !output.status.success() {
        return Err("LIVE_SOURCE_COMMIT_UNAVAILABLE");
    }
    let commit = String::from_utf8(output.stdout)
        .map_err(|_| "LIVE_SOURCE_COMMIT_UNAVAILABLE")?
        .trim()
        .to_owned();
    (commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(commit)
        .ok_or("LIVE_SOURCE_COMMIT_UNAVAILABLE")
}

fn mark_s2_evidence_gate(evidence: &mut Value) {
    add_object_field(evidence, "gate", json!(S2_ACCEPTANCE_GATE));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LiveSearchFailure {
    code: &'static str,
    status_code: Option<u16>,
}

impl LiveSearchFailure {
    const fn plain(code: &'static str) -> Self {
        Self {
            code,
            status_code: None,
        }
    }
}

fn bounded_status_code(error: &ContractError) -> Option<u16> {
    error
        .debug_context
        .get("status_code")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| (100..=599).contains(value))
}

fn classify_catalogue_product_error(error: &ContractError) -> &'static str {
    match error.debug_context.get("reason").and_then(Value::as_str) {
        Some("product") | Some("provider_product_key") | Some("stable_asset_key") => {
            "LIVE_SEARCH_PRODUCT_IDENTITY_REJECTED"
        }
        Some("acquisition_datetime") | Some("properties") => {
            "LIVE_SEARCH_PRODUCT_METADATA_REJECTED"
        }
        Some("assets") | Some("archive_asset") => "LIVE_SEARCH_PRODUCT_ARCHIVE_REJECTED",
        Some("archive_downloadability") => "LIVE_SEARCH_PRODUCT_DOWNLOADABILITY_REJECTED",
        Some("checksums") => "LIVE_SEARCH_PRODUCT_CHECKSUM_REJECTED",
        Some("archive_filename") => "LIVE_SEARCH_PRODUCT_FILENAME_REJECTED",
        Some("archive_access") | Some("snapshot_item") => "LIVE_SEARCH_PRODUCT_POLICY_REJECTED",
        Some("candidate_size") => "LIVE_SEARCH_PRODUCT_SIZE_REJECTED",
        _ => "LIVE_SEARCH_PRODUCT_INVALID",
    }
}

fn classify_live_search_error(error: &ApplicationServiceError) -> LiveSearchFailure {
    let ApplicationServiceError::Provider(error) = error else {
        return LiveSearchFailure::plain("LIVE_SEARCH_FAILED");
    };
    let code = match error.code.as_str() {
        "CDSE_ODATA_ROUTE_INVALID" | "CATALOGUE_CAPABILITY_UNAVAILABLE" => {
            "LIVE_SEARCH_ROUTE_REJECTED"
        }
        "CDSE_ODATA_CANCELLED" | "CATALOGUE_REQUEST_CANCELLED" => "LIVE_SEARCH_CANCELLED",
        "CDSE_ODATA_TIMEOUT" | "CDSE_ODATA_HTTP_FAILURE" => "LIVE_SEARCH_TRANSPORT_FAILED",
        "CDSE_ODATA_HTTP_UNAUTHORIZED" | "CDSE_ODATA_HTTP_FORBIDDEN" => {
            "LIVE_SEARCH_ACCESS_REJECTED"
        }
        "CDSE_ODATA_HTTP_NOT_FOUND" => "LIVE_SEARCH_ENDPOINT_REJECTED",
        "CDSE_ODATA_HTTP_PROXY_AUTH_REQUIRED" => "LIVE_SEARCH_PROXY_AUTH_REQUIRED",
        "CDSE_ODATA_HTTP_RATE_LIMITED" => "LIVE_SEARCH_RATE_LIMITED",
        "CDSE_ODATA_HTTP_SERVER" => "LIVE_SEARCH_SERVER_REJECTED",
        "CDSE_ODATA_HTTP_STATUS" => "LIVE_SEARCH_HTTP_STATUS_REJECTED",
        "CDSE_ODATA_HTTP_REDIRECT" => "LIVE_SEARCH_REDIRECT_REJECTED",
        "CDSE_ODATA_JSON_INVALID" => "LIVE_SEARCH_JSON_REJECTED",
        "CDSE_ODATA_PAGE_INVALID"
        | "CDSE_ODATA_CURSOR_UNTRUSTED"
        | "CDSE_ODATA_RESPONSE_TOO_LARGE" => "LIVE_SEARCH_PAGE_REJECTED",
        "CDSE_ODATA_ITEM_INVALID" | "CDSE_ODATA_UUID_CONFLICT" => "LIVE_SEARCH_ITEM_REJECTED",
        "CATALOGUE_RESPONSE_INVALID" => "LIVE_SEARCH_RESPONSE_INVALID",
        "CATALOGUE_RESPONSE_INCOMPLETE" => "LIVE_SEARCH_RESPONSE_INCOMPLETE",
        "CATALOGUE_PRODUCT_INVALID" => classify_catalogue_product_error(error),
        _ => "LIVE_SEARCH_FAILED",
    };
    LiveSearchFailure {
        code,
        status_code: bounded_status_code(error),
    }
}

fn verify_live_search_candidate(
    candidate: &insar_core::SnapshotItem,
    product_id: &str,
) -> Result<(u64, String), &'static str> {
    if candidate.footprint_geojson.is_none()
        || candidate
            .metadata
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get("online"))
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err("LIVE_METADATA_INTEGRITY_REJECTED");
    }
    let access = candidate
        .asset_access
        .as_ref()
        .ok_or("LIVE_METADATA_INTEGRITY_REJECTED")?;
    let expected_size = access
        .expected_size_bytes
        .ok_or("LIVE_METADATA_INTEGRITY_REJECTED")?;
    let checksum_algorithm = access
        .checksum_algorithm
        .as_deref()
        .filter(|value| matches!(*value, "MD5" | "SHA256"))
        .ok_or("LIVE_METADATA_INTEGRITY_REJECTED")?
        .to_owned();
    if candidate
        .metadata
        .get("provider_product_id")
        .and_then(Value::as_str)
        != Some(product_id)
    {
        return Err("LIVE_PRODUCT_NOT_IN_QUERY");
    }
    let assets = candidate
        .metadata
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("LIVE_METADATA_INTEGRITY_REJECTED")?;
    if assets.len() != 1
        || assets[0].get("asset_key").and_then(Value::as_str) != Some("archive")
        || assets[0].get("role").and_then(Value::as_str) != Some("primary")
    {
        return Err("LIVE_METADATA_INTEGRITY_REJECTED");
    }
    Ok((expected_size, checksum_algorithm))
}

fn candidate_provider_product_id(
    candidate: &insar_core::SnapshotItem,
) -> Result<String, &'static str> {
    let product_id = candidate
        .metadata
        .get("provider_product_id")
        .and_then(Value::as_str)
        .ok_or("LIVE_METADATA_INTEGRITY_REJECTED")?;
    canonical_product_id(product_id.to_owned())
}

fn select_live_search_candidate<'a>(
    candidates: &'a [insar_core::SnapshotItem],
    requested_product_id: Option<&str>,
) -> Result<(&'a insar_core::SnapshotItem, String, u64, String), &'static str> {
    if let Some(requested_product_id) = requested_product_id {
        let candidate = candidates
            .iter()
            .find(|candidate| {
                candidate
                    .metadata
                    .get("provider_product_id")
                    .and_then(Value::as_str)
                    == Some(requested_product_id)
            })
            .ok_or("LIVE_PRODUCT_NOT_IN_QUERY")?;
        let (expected_size, checksum_algorithm) =
            verify_live_search_candidate(candidate, requested_product_id)?;
        return Ok((
            candidate,
            requested_product_id.to_owned(),
            expected_size,
            checksum_algorithm,
        ));
    }

    candidates
        .iter()
        .filter_map(|candidate| {
            let product_id = candidate_provider_product_id(candidate).ok()?;
            let (expected_size, checksum_algorithm) =
                verify_live_search_candidate(candidate, &product_id).ok()?;
            Some((candidate, product_id, expected_size, checksum_algorithm))
        })
        .min_by(|left, right| left.2.cmp(&right.2).then_with(|| left.1.cmp(&right.1)))
        .ok_or("LIVE_SEARCH_NO_QUALIFIED_PRODUCT")
}

fn load_frozen_candidates(
    service: &ApplicationService,
    batch: &insar_store::StoreWorkbenchSummary,
) -> Result<Vec<insar_core::SnapshotItem>, &'static str> {
    let page_size = batch.page_size.clamp(1, 200);
    let page_count = (batch.candidate_count as u64).div_ceil(page_size);
    let mut candidates = Vec::with_capacity(batch.candidate_count);
    for page_index in 0..page_count {
        let page = service
            .workbench_page(&StoreWorkbenchPageRequest {
                batch_id: batch.batch_id.clone(),
                expected_revision: batch.revision,
                page_index,
                page_size,
            })
            .map_err(|_| "LIVE_SEARCH_PAGE_FAILED")?;
        candidates.extend(page.page.items.into_iter().map(|item| item.item));
    }
    if candidates.len() != batch.candidate_count {
        return Err("LIVE_SEARCH_PAGE_INCOMPLETE");
    }
    Ok(candidates)
}

fn run_anonymous_search_acceptance(
    input: AnonymousSearchInput,
) -> Result<AnonymousSearchReport, LiveSearchFailure> {
    let AnonymousSearchInput {
        product_id,
        aoi_geojson,
        start_utc,
        end_utc,
    } = input;
    let store = TempStore::new();
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&store.path));
    let result = (|| {
        bind_processing_aoi(
            &service,
            &aoi_geojson,
            "cdse-live-system-anonymous-bind-aoi",
            "CDSE anonymous search acceptance AOI",
        )
        .map_err(LiveSearchFailure::plain)?;
        let search = live_query(aoi_geojson.clone(), start_utc, end_utc);
        let frozen = service
            .catalog_search_and_freeze(&ApplicationCatalogSearchAndFreezeRequest {
                request_id: "cdse-live-system-anonymous-search".to_owned(),
                search,
                batch_id: "cdse-live-system-anonymous-batch".to_owned(),
                created_at: "2026-07-27T00:00:00Z".to_owned(),
                aoi_name: "CDSE anonymous search acceptance AOI".to_owned(),
                aoi_geojson: Some(aoi_geojson),
                workbench_page_size: 24,
            })
            .map_err(|error| classify_live_search_error(&error))?;
        let batch = frozen
            .core
            .workbench
            .ok_or_else(|| LiveSearchFailure::plain("LIVE_SEARCH_FREEZE_REJECTED"))?;
        let candidates =
            load_frozen_candidates(&service, &batch).map_err(LiveSearchFailure::plain)?;
        let (selected, selected_product_id, content_length, checksum_algorithm) =
            select_live_search_candidate(&candidates, product_id.as_deref())
                .map_err(LiveSearchFailure::plain)?;
        Ok(AnonymousSearchReport {
            product_id: selected_product_id,
            product_name: selected.display_name.clone(),
            content_length,
            checksum_algorithm,
            quicklook_available: false,
        })
    })();
    let _ = service.shutdown();
    result
}

fn wait_for_completed_task(
    service: &ApplicationService,
    item_id: &str,
    task_id: &str,
) -> Result<insar_core::DownloadItemResult, &'static str> {
    let deadline = Instant::now() + LIVE_TRANSFER_TIMEOUT;
    loop {
        let snapshot = service
            .download_snapshot(task_id)
            .map_err(|_| "LIVE_TASK_SNAPSHOT_FAILED")?;
        if let Some(result) = snapshot
            .results
            .iter()
            .find(|result| result.item_id == item_id)
        {
            return (result.outcome == DownloadOutcome::Completed)
                .then_some(result.clone())
                .ok_or("LIVE_TRANSFER_NOT_COMPLETED");
        }
        if Instant::now() >= deadline {
            return Err("LIVE_TRANSFER_TIMEOUT");
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn run_acceptance(input: LiveInput) -> Result<LiveReport, &'static str> {
    let LiveInput {
        secret,
        attempt_id,
        attempt_dir,
        product_id,
        aoi_geojson,
        start_utc,
        end_utc,
        destination,
        max_bytes,
    } = input;
    let store = TempStore::new();
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&store.path));
    let source_commit = current_source_commit()?;
    let result = (|| {
        let status = service
            .cdse_login(secret.into_login_request())
            .map_err(|_| "LIVE_LOGIN_FAILED")?;
        if !status.authenticated || status.network_mode != CdseNetworkMode::System {
            return Err("LIVE_LOGIN_FAILED");
        }
        bind_processing_aoi(
            &service,
            &aoi_geojson,
            "cdse-live-system-bind-aoi",
            "CDSE live acceptance AOI",
        )?;
        let effective = service
            .effective_capability(CAPABILITY_ID)
            .map_err(|_| "LIVE_EFFECTIVE_CAPABILITY_FAILED")?
            .ok_or("LIVE_EFFECTIVE_CAPABILITY_MISSING")?;
        if effective.projection.state != EffectiveCapabilityState::Executable {
            return Err("LIVE_EFFECTIVE_CAPABILITY_BLOCKED");
        }
        let search = live_query(aoi_geojson.clone(), start_utc, end_utc);
        let frozen = service
            .catalog_search_and_freeze(&ApplicationCatalogSearchAndFreezeRequest {
                request_id: "cdse-live-system-search".to_owned(),
                search: search.clone(),
                batch_id: "cdse-live-system-batch".to_owned(),
                created_at: "2026-07-27T00:00:00Z".to_owned(),
                aoi_name: "CDSE live acceptance AOI".to_owned(),
                aoi_geojson: Some(aoi_geojson),
                workbench_page_size: 24,
            })
            .map_err(|error| classify_live_search_error(&error).code)?;
        let batch = frozen.core.workbench.ok_or("LIVE_SEARCH_FREEZE_REJECTED")?;
        let candidates = load_frozen_candidates(&service, &batch)?;
        let selected = candidates
            .iter()
            .find(|candidate| {
                candidate
                    .metadata
                    .get("provider_product_id")
                    .and_then(Value::as_str)
                    == Some(product_id.as_str())
            })
            .cloned()
            .ok_or("LIVE_PRODUCT_NOT_IN_QUERY")?;
        let (expected_size, checksum_algorithm) =
            verify_live_search_candidate(&selected, &product_id)?;
        let archive_basename = selected
            .asset_access
            .as_ref()
            .and_then(|access| access.file_name.as_deref())
            .ok_or("LIVE_METADATA_INTEGRITY_REJECTED")?;
        if expected_size > max_bytes {
            return Err("LIVE_BYTE_CAP_EXCEEDED");
        }
        let final_path = destination.join(archive_basename);
        if final_path.exists() || part_path(&final_path).exists() {
            return Err("LIVE_DESTINATION_OCCUPIED");
        }

        let stable_href = selected
            .asset_access
            .as_ref()
            .and_then(|access| access.descriptor.stable_href.as_deref())
            .ok_or("LIVE_METADATA_INTEGRITY_REJECTED")?;
        let checksum_value = selected
            .asset_access
            .as_ref()
            .and_then(|access| access.checksum_value.as_deref())
            .ok_or("LIVE_METADATA_INTEGRITY_REJECTED")?
            .to_owned();
        let properties = selected
            .metadata
            .get("properties")
            .and_then(Value::as_object)
            .ok_or("LIVE_METADATA_INTEGRITY_REJECTED")?;
        let mut frozen_evidence = evidence_base(
            RESOURCE_ID,
            CAPABILITY_ID,
            PROVIDER_ID,
            &product_id,
            &attempt_id,
        );
        mark_s2_evidence_gate(&mut frozen_evidence);
        add_object_field(
            &mut frozen_evidence,
            "source_commit",
            json!(source_commit.clone()),
        );
        add_object_field(
            &mut frozen_evidence,
            "attempt_status",
            json!("frozen_before_transfer"),
        );
        add_object_field(&mut frozen_evidence, "transfer_started", json!(false));
        add_object_field(
            &mut frozen_evidence,
            "provider_product_name",
            json!(selected.display_name),
        );
        add_object_field(
            &mut frozen_evidence,
            "acquisition_identity",
            json!({
                "acquisition_datetime": selected.acquisition_datetime,
                "product_type": properties.get("product_type"),
                "processing_level": "L2A",
                "level_proof": "product_type=S2MSI2A",
            }),
        );
        add_object_field(
            &mut frozen_evidence,
            "product_contract",
            json!({
                "sensor": "Sentinel-2",
                "processing_level": "L2A",
                "complete_archive": true,
                "archive_first_delivery": true,
            }),
        );
        add_object_field(
            &mut frozen_evidence,
            "search_query",
            serde_json::to_value(&search).map_err(|_| "LIVE_EVIDENCE_SERIALIZATION_FAILED")?,
        );
        add_object_field(
            &mut frozen_evidence,
            "aoi_geojson",
            serde_json::to_value(&search.aoi_geojson)
                .map_err(|_| "LIVE_EVIDENCE_SERIALIZATION_FAILED")?,
        );
        add_object_field(
            &mut frozen_evidence,
            "expected_asset",
            json!({
                "asset_key": selected.asset_access.as_ref().map(|access| access.reference.asset_key.as_str()),
                "role": "primary",
                "file_name": archive_basename,
                "media_type": "application/zip",
                "expected_size_bytes": expected_size,
                "checksum_algorithm": checksum_algorithm,
                "checksum_value": checksum_value,
            }),
        );
        add_object_field(
            &mut frozen_evidence,
            "downloadability",
            json!({
                "online": properties.get("online"),
                "downloadable": true,
                "expected_destination_class": "external_non_repository_e_drive",
                "max_permitted_bytes": max_bytes,
            }),
        );
        add_object_field(
            &mut frozen_evidence,
            "source_identity",
            json!({
                "frozen": true,
                "stable_locator_sha256": sha256_hex(stable_href),
                "resolver_policy_id": selected.metadata.get("resolver_policy_id"),
                "credential_route_class": "cdse.system-mode.session",
                "credential_readiness": "authenticated_system",
            }),
        );
        add_object_field(
            &mut frozen_evidence,
            "effective_capability",
            serde_json::to_value(&effective).map_err(|_| "LIVE_EVIDENCE_SERIALIZATION_FAILED")?,
        );
        add_object_field(
            &mut frozen_evidence,
            "task_identity",
            json!({
                "task_id": LIVE_TASK_ID,
                "batch_id": batch.batch_id,
                "item_id": selected.item_id,
                "layout": "direct",
                "concurrency": 1,
            }),
        );
        write_immutable_json(&attempt_dir, "freeze.json", &frozen_evidence)
            .map_err(|_| "LIVE_EVIDENCE_WRITE_FAILED")?;

        let (quicklook_fetched, quicklook_bytes) =
            match service.cdse_quicklook(CdseQuicklookRequest {
                request_id: "cdse-live-system-quicklook".to_owned(),
                batch_id: batch.batch_id.clone(),
                item_id: selected.item_id.clone(),
            }) {
                Ok(quicklook) => {
                    if quicklook.byte_length == 0
                        || !matches!(
                            quicklook.media_type.as_str(),
                            "image/jpeg" | "image/png" | "image/webp"
                        )
                    {
                        return Err("LIVE_QUICKLOOK_FAILED");
                    }
                    (true, quicklook.byte_length)
                }
                Err(ApplicationServiceError::CdseQuicklookUnavailable) => (false, 0),
                Err(_) => return Err("LIVE_QUICKLOOK_FAILED"),
            };

        service
            .create_download_task_legacy(&StoreCreateTaskRequest {
                task_id: LIVE_TASK_ID.to_owned(),
                batch_id: batch.batch_id,
                selected_item_ids: vec![selected.item_id.clone()],
                checked_item_ids: vec![selected.item_id.clone()],
                page_index: 0,
                page_size: 24,
                output_root: destination,
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-07-27T00:00:01Z".to_owned(),
            })
            .map_err(|_| "LIVE_TASK_CREATE_FAILED")?;
        let transfer = wait_for_completed_task(&service, &selected.item_id, LIVE_TASK_ID)?;
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
        let runtime_completed = store_snapshot
            .restore
            .runtimes
            .iter()
            .find(|runtime| runtime.task_id == LIVE_TASK_ID)
            .is_some_and(|runtime| runtime.status == TaskLifecycleStatus::Completed);
        if !runtime_completed {
            return Err("LIVE_STORE_COMPLETION_FAILED");
        }

        let product_name = selected.display_name;
        let logged_out = service
            .cdse_logout()
            .map_err(|_| "LIVE_SESSION_CLEANUP_FAILED")?;
        if logged_out.authenticated {
            return Err("LIVE_SESSION_CLEANUP_FAILED");
        }
        let shutdown = service.shutdown();
        if shutdown.is_err() {
            return Err("LIVE_SESSION_CLEANUP_FAILED");
        }
        let restarted = ApplicationService::new(ApplicationServiceConfig::existing_business_store(
            &store.path,
        ));
        let restored = restarted
            .download_snapshot(LIVE_TASK_ID)
            .map_err(|_| "LIVE_TASK_RESTORE_FAILED")?;
        let completed = restored.results.iter().any(|result| {
            result.item_id == selected.item_id && result.outcome == DownloadOutcome::Completed
        });
        let restart_shutdown = restarted.shutdown();
        if !completed || restart_shutdown.is_err() {
            return Err("LIVE_TASK_RESTORE_FAILED");
        }
        let mut final_evidence = frozen_evidence;
        add_object_field(&mut final_evidence, "attempt_status", json!("accepted"));
        add_object_field(&mut final_evidence, "transfer_started", json!(true));
        add_object_field(
            &mut final_evidence,
            "validation",
            json!({
                "source_size_enforced": true,
                "source_checksum_algorithm": checksum_algorithm,
                "source_checksum_value": checksum_value,
                "checksum_verified_by_production_transfer": true,
                "archive_structure_validation": "not_supported_by_current_validator",
                "final_artifact_name": archive_basename,
                "final_artifact_size_bytes": transfer.bytes_written,
                "final_artifact_checksum": {
                    "algorithm": checksum_algorithm,
                    "value": checksum_value,
                    "verified": true,
                },
                "part_present_after_promotion": false,
                "atomic_promotion_observed": true,
            }),
        );
        add_object_field(
            &mut final_evidence,
            "store_lifecycle",
            json!({
                "completed_after_validation": true,
                "runtime_status": "completed",
                "reopen_completed": true,
            }),
        );
        add_object_field(
            &mut final_evidence,
            "network_observation",
            json!({
                "remote_bytes_written": transfer.bytes_written,
                "transfer_attempt_count": transfer.attempt_count,
                "transfer_count": transfer.attempt_count,
                "transfer_count_semantics": "production_worker_attempt_ledger_count",
                "request_count": Value::Null,
                "request_count_observation": "not_exposed_by_ApplicationService_Store_v1",
                "quicklook_fetched": quicklook_fetched,
                "quicklook_bytes": quicklook_bytes,
            }),
        );
        add_object_field(
            &mut final_evidence,
            "recovery_observation",
            json!({
                "live_interruption_intentionally_run": false,
                "offline_bounded_recovery_regression": "crates/insar-download/src/cdse_loopback_transfer.rs",
                "offline_recovery_contract_accepted": true,
            }),
        );
        write_immutable_json(&attempt_dir, "evidence.json", &final_evidence)
            .map_err(|_| "LIVE_EVIDENCE_WRITE_FAILED")?;
        Ok(LiveReport {
            product_id: product_id.clone(),
            product_name,
            checksum_algorithm,
            checksum_value,
            content_length: expected_size,
            final_bytes: transfer.bytes_written,
            quicklook_fetched,
            quicklook_bytes: quicklook_bytes as u64,
        })
    })();
    if let Err(code) = result {
        let mut failure = evidence_base(
            RESOURCE_ID,
            CAPABILITY_ID,
            PROVIDER_ID,
            &product_id,
            &attempt_id,
        );
        mark_s2_evidence_gate(&mut failure);
        add_object_field(&mut failure, "source_commit", json!(source_commit));
        add_object_field(&mut failure, "attempt_status", json!("stopped"));
        add_object_field(&mut failure, "failure_code", json!(code));
        add_object_field(
            &mut failure,
            "credential_route_class",
            json!("cdse.system-mode.session"),
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
        let _ = write_immutable_json(&attempt_dir, "failure.json", &failure);
    }
    if !service.is_shutting_down() {
        let _ = service.cdse_logout();
        let _ = service.shutdown();
    }
    result
}

struct AnonymousSearchReport {
    product_id: String,
    product_name: String,
    content_length: u64,
    checksum_algorithm: String,
    quicklook_available: bool,
}

#[test]
#[ignore = "requires explicit opt-in for one anonymous CDSE production search"]
fn controlled_system_mode_cdse_anonymous_search_acceptance() {
    let _environment_cleanup = EnvironmentCleanup;
    match anonymous_search_input()
        .map_err(LiveSearchFailure::plain)
        .and_then(run_anonymous_search_acceptance)
    {
        Ok(report) => eprintln!(
            "CDSE_ANONYMOUS_SEARCH_ACCEPTANCE outcome=completed product_id={} product_name={} content_length={} checksum_algorithm={} quicklook_available={}",
            report.product_id,
            report.product_name,
            report.content_length,
            report.checksum_algorithm,
            report.quicklook_available,
        ),
        Err(failure) => panic!(
            "CDSE_ANONYMOUS_SEARCH outcome=stopped code={} status_code={}",
            failure.code,
            failure
                .status_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_owned())
        ),
    }
}

#[test]
#[ignore = "requires explicit session-only CDSE credentials, a fixed AOI, and one approved product"]
fn controlled_system_mode_cdse_production_acceptance() {
    let _environment_cleanup = EnvironmentCleanup;
    match live_input().and_then(run_acceptance) {
        Ok(report) => eprintln!(
            "CDSE_PRODUCTION_ACCEPTANCE outcome=completed product_id={} product_name={} content_length={} final_bytes={} checksum_algorithm={} checksum_value={} checksum_verified=true quicklook_fetched={} quicklook_bytes={}",
            report.product_id,
            report.product_name,
            report.content_length,
            report.final_bytes,
            report.checksum_algorithm,
            report.checksum_value,
            report.quicklook_fetched,
            report.quicklook_bytes,
        ),
        Err(code) => panic!("CDSE_PRODUCTION_ACCEPTANCE outcome=stopped code={code}"),
    }
}

#[test]
fn live_search_error_classification_is_fixed_and_redacted() {
    for (code, expected) in [
        ("CDSE_ODATA_ROUTE_INVALID", "LIVE_SEARCH_ROUTE_REJECTED"),
        ("CDSE_ODATA_TIMEOUT", "LIVE_SEARCH_TRANSPORT_FAILED"),
        (
            "CDSE_ODATA_HTTP_PROXY_AUTH_REQUIRED",
            "LIVE_SEARCH_PROXY_AUTH_REQUIRED",
        ),
        ("CDSE_ODATA_HTTP_FORBIDDEN", "LIVE_SEARCH_ACCESS_REJECTED"),
        ("CDSE_ODATA_HTTP_NOT_FOUND", "LIVE_SEARCH_ENDPOINT_REJECTED"),
        ("CDSE_ODATA_HTTP_RATE_LIMITED", "LIVE_SEARCH_RATE_LIMITED"),
        ("CDSE_ODATA_HTTP_SERVER", "LIVE_SEARCH_SERVER_REJECTED"),
        ("CDSE_ODATA_HTTP_STATUS", "LIVE_SEARCH_HTTP_STATUS_REJECTED"),
        ("CDSE_ODATA_HTTP_REDIRECT", "LIVE_SEARCH_REDIRECT_REJECTED"),
        ("CDSE_ODATA_JSON_INVALID", "LIVE_SEARCH_JSON_REJECTED"),
        ("CDSE_ODATA_PAGE_INVALID", "LIVE_SEARCH_PAGE_REJECTED"),
        ("CDSE_ODATA_ITEM_INVALID", "LIVE_SEARCH_ITEM_REJECTED"),
        ("CATALOGUE_RESPONSE_INVALID", "LIVE_SEARCH_RESPONSE_INVALID"),
        (
            "CATALOGUE_RESPONSE_INCOMPLETE",
            "LIVE_SEARCH_RESPONSE_INCOMPLETE",
        ),
        ("CATALOGUE_PRODUCT_INVALID", "LIVE_SEARCH_PRODUCT_INVALID"),
        ("UNREVIEWED_PROVIDER_CODE", "LIVE_SEARCH_FAILED"),
    ] {
        let error = ApplicationServiceError::Provider(
            ContractError::new(code, "fixture", Map::new()).expect("safe fixture error"),
        );
        assert_eq!(classify_live_search_error(&error).code, expected, "{code}");
    }
}

#[test]
fn catalogue_product_failure_classification_stays_bounded() {
    let error = ApplicationServiceError::Provider(
        ContractError::new(
            "CATALOGUE_PRODUCT_INVALID",
            "fixture",
            Map::from_iter([(
                "reason".to_owned(),
                Value::String("archive_downloadability".to_owned()),
            )]),
        )
        .expect("safe fixture error"),
    );
    assert_eq!(
        classify_live_search_error(&error).code,
        "LIVE_SEARCH_PRODUCT_DOWNLOADABILITY_REJECTED"
    );
}

#[test]
fn live_search_status_evidence_is_numeric_and_bounded() {
    let error = ApplicationServiceError::Provider(
        ContractError::new(
            "CDSE_ODATA_HTTP_STATUS",
            "fixture",
            Map::from_iter([
                ("reason".to_owned(), Value::String("status".to_owned())),
                ("status_code".to_owned(), json!(400)),
            ]),
        )
        .expect("safe fixture error"),
    );
    assert_eq!(
        classify_live_search_error(&error),
        LiveSearchFailure {
            code: "LIVE_SEARCH_HTTP_STATUS_REJECTED",
            status_code: Some(400),
        }
    );
    let invalid = ApplicationServiceError::Provider(
        ContractError::new(
            "CDSE_ODATA_HTTP_STATUS",
            "fixture",
            Map::from_iter([("status_code".to_owned(), json!(600))]),
        )
        .expect("safe fixture error"),
    );
    assert_eq!(classify_live_search_error(&invalid).status_code, None);
}

#[test]
fn anonymous_input_allows_catalogue_selection_without_a_preselected_product() {
    let prior = [
        (
            LIVE_ANONYMOUS_SEARCH_ACCEPTANCE_FLAG,
            std::env::var_os(LIVE_ANONYMOUS_SEARCH_ACCEPTANCE_FLAG),
        ),
        (LIVE_AOI_GEOJSON, std::env::var_os(LIVE_AOI_GEOJSON)),
        (LIVE_START_UTC, std::env::var_os(LIVE_START_UTC)),
        (LIVE_END_UTC, std::env::var_os(LIVE_END_UTC)),
        (LIVE_PRODUCT_ID, std::env::var_os(LIVE_PRODUCT_ID)),
    ];
    unsafe {
        std::env::set_var(LIVE_ANONYMOUS_SEARCH_ACCEPTANCE_FLAG, "1");
        std::env::set_var(
            LIVE_AOI_GEOJSON,
            r#"{"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]}"#,
        );
        std::env::set_var(LIVE_START_UTC, "2026-01-01T00:00:00Z");
        std::env::set_var(LIVE_END_UTC, "2026-01-02T00:00:00Z");
        std::env::remove_var(LIVE_PRODUCT_ID);
    }
    let result = anonymous_search_input();
    for (name, value) in prior {
        unsafe {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
    assert!(
        result.is_ok(),
        "a bounded anonymous qualification may select its target from the response"
    );
}

#[test]
fn s2_evidence_uses_the_specific_acceptance_gate() {
    let mut evidence = evidence_base(
        RESOURCE_ID,
        CAPABILITY_ID,
        PROVIDER_ID,
        "00000000-0000-4000-8000-000000000001",
        "s2-fixture-attempt",
    );
    mark_s2_evidence_gate(&mut evidence);
    assert_eq!(evidence["gate"], S2_ACCEPTANCE_GATE);
}

#[test]
fn s2_source_commit_is_bounded_and_current() {
    let commit = current_source_commit().expect("the acceptance runs from a Git checkout");
    assert_eq!(commit.len(), 40);
    assert!(commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[test]
fn one_shot_secret_pipe_names_and_payloads_stay_bounded() {
    assert!(valid_pipe_name("insar-cdse-abc_123"));
    assert!(!valid_pipe_name(""));
    assert!(!valid_pipe_name("pipe/name"));
    assert!(!valid_pipe_name(&"x".repeat(129)));

    let secret = serde_json::from_str::<LiveSecretInput>(
        r#"{"username":"fixture-user","password":"fixture-password","totp":"123456"}"#,
    )
    .expect("synthetic local secret parses");
    assert!(secret.valid());
    let request = secret.into_login_request();
    assert_eq!(request.username, "fixture-user");
    assert_eq!(request.password, "fixture-password");
    assert_eq!(request.totp.as_deref(), Some("123456"));
    assert!(serde_json::from_str::<LiveSecretInput>(
        r#"{"username":"fixture-user","password":"fixture-password","unexpected":true}"#,
    )
    .is_err());
    let invalid_totp = serde_json::from_str::<LiveSecretInput>(
        r#"{"username":"fixture-user","password":"fixture-password","totp":"not-a-totp"}"#,
    )
    .expect("synthetic invalid TOTP still parses before validation");
    assert!(!invalid_totp.valid());
    let blank_username = serde_json::from_str::<LiveSecretInput>(
        r#"{"username":"   ","password":"fixture-password"}"#,
    )
    .expect("synthetic blank username still parses before validation");
    assert!(!blank_username.valid());
    assert!(valid_utc_range(
        "2026-07-26T12:51:21Z",
        "2026-07-26T12:55:21Z"
    ));
    assert!(!valid_utc_range(
        "2026-07-26T12:55:21Z",
        "2026-07-26T12:51:21Z"
    ));
}

#[cfg(windows)]
#[test]
fn windows_named_pipe_delivers_one_bounded_payload_then_eof() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{WriteFile, PIPE_ACCESS_OUTBOUND};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after the Unix epoch")
        .as_nanos();
    let pipe_name = format!("insar-cdse-test-{}-{nonce}", std::process::id());
    let pipe_path = format!(r"\\.\pipe\{pipe_name}");
    let mut wide_path = OsStr::new(&pipe_path).encode_wide().collect::<Vec<u16>>();
    wide_path.push(0);
    let server = unsafe {
        CreateNamedPipeW(
            wide_path.as_ptr(),
            PIPE_ACCESS_OUTBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            MAX_LIVE_SECRET_BYTES as u32,
            MAX_LIVE_SECRET_BYTES as u32,
            0,
            std::ptr::null(),
        )
    };
    assert_ne!(server, INVALID_HANDLE_VALUE, "synthetic pipe server opens");
    let payload = br#"{"username":"fixture-user","password":"fixture-password"}"#.to_vec();
    // Raw Windows handles are not `Send`; this test transfers the owned value
    // to its one writer thread without duplicating or exposing it.
    let server_for_writer = server as usize;
    let writer = thread::spawn(move || {
        let server = server_for_writer as HANDLE;
        let connected = unsafe { ConnectNamedPipe(server, std::ptr::null_mut()) };
        if connected == 0 {
            assert_eq!(unsafe { GetLastError() }, ERROR_PIPE_CONNECTED);
        }
        let mut written = 0_u32;
        assert_ne!(
            unsafe {
                WriteFile(
                    server,
                    payload.as_ptr(),
                    payload.len() as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            },
            0,
            "synthetic pipe payload writes"
        );
        assert_eq!(written as usize, payload.len());
        assert_ne!(unsafe { CloseHandle(server) }, 0, "synthetic pipe closes");
    });

    let secret = decode_live_secret_pipe(pipe_name).expect("bounded pipe payload decodes");
    writer.join().expect("synthetic pipe writer completes");
    assert!(secret.valid());
    let login = secret.into_login_request();
    assert_eq!(login.username, "fixture-user");
    assert_eq!(login.password, "fixture-password");
    assert!(login.totp.is_none());
}
