//! Explicitly opted-in live CDSE acceptance harness.
//!
//! The ignored live test never runs in ordinary or offline suites. Its pure
//! validation checks exist only to prove that one user-approved archive will
//! use the frozen CDSE Resolver and the existing `HttpTransfer` implementation.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use insar_core::{provider_product_key, stable_asset_key, DownloadOutcome, SnapshotItem};
use insar_provider::catalog::{
    CDSE_SENTINEL2_L2A_ARCHIVE_FILENAME_RULE_ID, CDSE_SENTINEL2_L2A_IDENTITY_RULE_ID,
    SENTINEL2_L2A_PROFILE_ID,
};
use insar_provider::cdse::archive_filename::{
    evaluate_archive_filename, safe_cdse_archive_basename, safe_cdse_odata_name,
    ArchiveFilenameEvidence,
};
use insar_provider::cdse::odata_types::{ODataChecksum, ODataPage, ODataProduct};
use reqwest::blocking::{Client, Response};
use reqwest::header::{
    HeaderValue, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, ETAG, LAST_MODIFIED, LOCATION,
    RANGE, RETRY_AFTER,
};
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use serde_json::{json, Map, Value};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::asset_resolver::cdse::{
    stable_cdse_archive_href, valid_cdse_checksum, CDSE_ARCHIVE_ASSET_KEY, CDSE_COLLECTION_ID,
    CDSE_PROVIDER_ID,
};
use crate::{
    build_offline_cdse_archive_access, part_path, verify_file_checksum, AssetResolverRegistry,
    AuthRefreshPolicyId, CdseArchiveAssetResolver, CdseArchiveEvidence, CredentialMaterial,
    CredentialProvider, CredentialProviderError, ExpectedChecksum, HttpTransfer, ProxyPolicy,
    ResolveContext, ResolvedTransferResult, SystemClock, TransferOptions, TransferRequest,
};

const LIVE_ACCEPTANCE_FLAG: &str = "INSAR_CDSE_LIVE_ACCEPTANCE";
const LIVE_ACCESS_TOKEN: &str = "INSAR_CDSE_LIVE_ACCESS_TOKEN";
const LIVE_PRODUCT_ID: &str = "INSAR_CDSE_LIVE_PRODUCT_ID";
const LIVE_DESTINATION_DIR: &str = "INSAR_CDSE_LIVE_DESTINATION_DIR";
const LIVE_MAX_BYTES: &str = "INSAR_CDSE_LIVE_MAX_BYTES";
const MAX_METADATA_BODY_BYTES: usize = 1024 * 1024;
const MAX_PREFLIGHT_HEADER_BYTES: usize = 1024;
const LIVE_CONTROL_TIMEOUT: Duration = Duration::from_secs(45);
const LIVE_ARCHIVE_TRANSFER_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);

struct LiveInput {
    access_token: Zeroizing<String>,
    product_id: String,
    destination_dir: PathBuf,
    max_bytes: u64,
}

struct LiveCredentials(Zeroizing<String>);

impl CredentialProvider for LiveCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        if profile != "copernicus-dataspace-default" {
            return Err(CredentialProviderError::Missing);
        }
        CredentialMaterial::bearer(self.0.as_str())
    }
}

struct LiveMetadata {
    product_id: String,
    display_name: String,
    expected_size_bytes: u64,
    checksum_algorithm: String,
    checksum_value: String,
}

struct LiveReport {
    product_id: String,
    display_name: String,
    archive_basename: String,
    final_size_bytes: u64,
    checksum_algorithm: String,
}

struct PreflightEvidence {
    filename_evidence: ArchiveFilenameEvidence,
    filename_class: PreflightFilenameClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreflightDiagnosticReason {
    EvidenceValid,
    StatusUnexpected,
    EffectiveUrlMismatch,
    ContentRangeMissing,
    ContentRangeMalformed,
    ContentRangeTotalMismatch,
    ContentLengthMismatch,
    ContentDispositionNonUtf8,
    FilenameEvidenceRejected,
    BodyShapeUnexpected,
}

impl PreflightDiagnosticReason {
    const fn code(self) -> &'static str {
        match self {
            Self::EvidenceValid => "PREFLIGHT_EVIDENCE_VALID",
            Self::StatusUnexpected => "PREFLIGHT_STATUS_UNEXPECTED",
            Self::EffectiveUrlMismatch => "PREFLIGHT_EFFECTIVE_URL_MISMATCH",
            Self::ContentRangeMissing => "PREFLIGHT_CONTENT_RANGE_MISSING",
            Self::ContentRangeMalformed => "PREFLIGHT_CONTENT_RANGE_MALFORMED",
            Self::ContentRangeTotalMismatch => "PREFLIGHT_CONTENT_RANGE_TOTAL_MISMATCH",
            Self::ContentLengthMismatch => "PREFLIGHT_CONTENT_LENGTH_MISMATCH",
            Self::ContentDispositionNonUtf8 => "PREFLIGHT_CONTENT_DISPOSITION_NON_UTF8",
            Self::FilenameEvidenceRejected => "PREFLIGHT_FILENAME_EVIDENCE_REJECTED",
            Self::BodyShapeUnexpected => "PREFLIGHT_BODY_SHAPE_UNEXPECTED",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreflightFilenameClass {
    Missing,
    Exact,
    Conflicting,
    Malformed,
}

impl PreflightFilenameClass {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Exact => "exact",
            Self::Conflicting => "conflicting",
            Self::Malformed => "malformed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreflightDiagnosticFacts {
    status: u16,
    effective_url_matches: bool,
    content_disposition_present: bool,
    content_length_present: bool,
    content_range_present: bool,
    etag_present: bool,
    last_modified_present: bool,
    content_length: Option<u64>,
    content_range_start: Option<u64>,
    content_range_end: Option<u64>,
    content_range_total: Option<u64>,
    content_range_total_matches_catalogue: Option<bool>,
    expected_catalogue_content_length: u64,
    filename_class: PreflightFilenameClass,
    body_bytes_observed: u8,
    response_host_class: &'static str,
    response_path_class: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreflightDiagnostic {
    reason: PreflightDiagnosticReason,
    facts: PreflightDiagnosticFacts,
}

struct PreflightInspection {
    evidence: PreflightEvidence,
    facts: PreflightDiagnosticFacts,
}

#[derive(Clone, Copy)]
enum PreflightHeaderText<'a> {
    Missing,
    NonUtf8,
    Text(&'a str),
}

#[derive(Clone, Copy)]
struct ParsedContentRange {
    start: u64,
    end: u64,
    total: u64,
}

#[derive(Clone, Copy, Debug)]
struct LiveStop {
    code: &'static str,
    status: Option<u16>,
    retry_after_seconds: Option<u64>,
    response_host_class: &'static str,
    response_path_class: &'static str,
}

impl LiveStop {
    const fn input_required() -> Self {
        Self {
            code: "LIVE_INPUT_REQUIRED",
            status: None,
            retry_after_seconds: None,
            response_host_class: "none",
            response_path_class: "none",
        }
    }

    const fn code(code: &'static str) -> Self {
        Self {
            code,
            status: None,
            retry_after_seconds: None,
            response_host_class: "none",
            response_path_class: "none",
        }
    }

    const fn response(code: &'static str, status: u16) -> Self {
        Self {
            code,
            status: Some(status),
            retry_after_seconds: None,
            response_host_class: "approved-download",
            response_path_class: "exact-value",
        }
    }

    const fn catalogue_response(code: &'static str, status: u16) -> Self {
        Self {
            code,
            status: Some(status),
            retry_after_seconds: None,
            response_host_class: "catalogue",
            response_path_class: "exact-product",
        }
    }
}

enum LiveOutcome {
    Completed(LiveReport),
    Stopped(LiveStop),
}

enum LivePreflightDiagnosticOutcome {
    Observed(PreflightDiagnostic),
    Stopped(LiveStop),
}

fn required_environment(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty() && value == value.trim())
}

fn canonical_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .map(|parsed| parsed.hyphenated().to_string() == value)
        .unwrap_or(false)
}

fn repository_root() -> Option<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .ok()
}

fn live_input_from_environment() -> Result<LiveInput, LiveStop> {
    if std::env::var(LIVE_ACCEPTANCE_FLAG).ok().as_deref() != Some("1") {
        return Err(LiveStop::input_required());
    }
    let access_token = required_environment(LIVE_ACCESS_TOKEN)
        .map(Zeroizing::new)
        .ok_or_else(LiveStop::input_required)?;
    let product_id = required_environment(LIVE_PRODUCT_ID).ok_or_else(LiveStop::input_required)?;
    if !canonical_uuid(&product_id) {
        return Err(LiveStop::code("LIVE_INPUT_INVALID"));
    }
    let destination_dir = PathBuf::from(
        required_environment(LIVE_DESTINATION_DIR).ok_or_else(LiveStop::input_required)?,
    )
    .canonicalize()
    .map_err(|_| LiveStop::code("LIVE_DESTINATION_UNSAFE"))?;
    let repository_root = repository_root().ok_or_else(|| LiveStop::code("LIVE_INTERNAL_ERROR"))?;
    if !destination_dir.is_dir() || destination_dir.starts_with(repository_root) {
        return Err(LiveStop::code("LIVE_DESTINATION_UNSAFE"));
    }
    let max_bytes = required_environment(LIVE_MAX_BYTES)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(LiveStop::input_required)?;
    Ok(LiveInput {
        access_token,
        product_id,
        destination_dir,
        max_bytes,
    })
}

fn exact_catalogue_metadata_url(product_id: &str) -> Option<Url> {
    if !canonical_uuid(product_id) {
        return None;
    }
    let mut url = Url::parse("https://catalogue.dataspace.copernicus.eu/odata/v1/Products").ok()?;
    let filter = format!("Id eq {product_id} and Collection/Name eq '{CDSE_COLLECTION_ID}'");
    url.query_pairs_mut()
        .append_pair("$filter", &filter)
        .append_pair("$expand", "Attributes")
        .append_pair("$count", "true")
        .append_pair("$top", "2");
    Some(url)
}

fn exact_download_product_url(product_id: &str) -> Option<Url> {
    stable_cdse_archive_href(product_id).and_then(|value| Url::parse(&value).ok())
}

fn is_fixed_origin_and_path(url: &Url, expected_host: &str, expected_path: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(expected_host)
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.path() == expected_path
}

fn is_exact_url(url: &Url, expected_host: &str, expected_path: &str) -> bool {
    is_fixed_origin_and_path(url, expected_host, expected_path) && url.query().is_none()
}

fn is_exact_catalogue_metadata_url(url: &Url, expected: &Url) -> bool {
    url == expected
        && is_fixed_origin_and_path(
            url,
            "catalogue.dataspace.copernicus.eu",
            "/odata/v1/Products",
        )
}

fn live_client() -> Result<Client, LiveStop> {
    Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .cookie_store(false)
        .referer(false)
        .timeout(LIVE_CONTROL_TIMEOUT)
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| LiveStop::code("LIVE_TRANSPORT_UNAVAILABLE"))
}

fn read_bounded_json(response: Response) -> Result<Value, LiveStop> {
    if response
        .content_length()
        .is_some_and(|size| size > MAX_METADATA_BODY_BYTES as u64)
    {
        return Err(LiveStop::code("LIVE_METADATA_TOO_LARGE"));
    }
    let mut body = Vec::new();
    response
        .take((MAX_METADATA_BODY_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| LiveStop::code("LIVE_METADATA_UNAVAILABLE"))?;
    if body.len() > MAX_METADATA_BODY_BYTES {
        return Err(LiveStop::code("LIVE_METADATA_TOO_LARGE"));
    }
    serde_json::from_slice(&body).map_err(|_| LiveStop::code("LIVE_METADATA_INVALID"))
}

fn selected_checksum(checksums: &[ODataChecksum]) -> Result<(String, String), LiveStop> {
    let mut accepted = BTreeMap::<String, String>::new();
    for checksum in checksums {
        let (Some(algorithm), Some(value)) = (&checksum.algorithm, &checksum.value) else {
            continue;
        };
        if !matches!(algorithm.as_str(), "MD5" | "SHA256") {
            continue;
        }
        if !valid_cdse_checksum(Some(algorithm), Some(value))
            || accepted.insert(algorithm.clone(), value.clone()).is_some()
        {
            return Err(LiveStop::code("LIVE_METADATA_INTEGRITY_REJECTED"));
        }
    }
    accepted
        .remove("SHA256")
        .map(|value| ("SHA256".to_owned(), value))
        .or_else(|| {
            accepted
                .remove("MD5")
                .map(|value| ("MD5".to_owned(), value))
        })
        .ok_or_else(|| LiveStop::code("LIVE_METADATA_INTEGRITY_REJECTED"))
}

fn validate_live_metadata(
    product: ODataProduct,
    requested_product_id: &str,
    max_bytes: u64,
    collection_proven_by_query: bool,
) -> Result<LiveMetadata, LiveStop> {
    let product_id = product
        .id
        .as_deref()
        .filter(|value| canonical_uuid(value) && *value == requested_product_id)
        .ok_or_else(|| LiveStop::code("LIVE_METADATA_IDENTITY_REJECTED"))?;
    if !collection_proven_by_query
        || product
            .collection
            .as_ref()
            .is_some_and(|collection| collection.name.as_deref() != Some(CDSE_COLLECTION_ID))
    {
        return Err(LiveStop::code("LIVE_METADATA_IDENTITY_REJECTED"));
    }
    let display_name = product
        .name
        .as_deref()
        .filter(|value| safe_cdse_odata_name(value))
        .ok_or_else(|| LiveStop::code("LIVE_METADATA_INTEGRITY_REJECTED"))?;
    if product.online != Some(true) {
        return Err(LiveStop::code("LIVE_METADATA_OFFLINE"));
    }
    let expected_size_bytes = product
        .content_length
        .filter(|size| *size > 0 && *size <= max_bytes)
        .ok_or_else(|| LiveStop::code("LIVE_METADATA_SIZE_REJECTED"))?;
    let product_type = product
        .attributes
        .iter()
        .filter(|attribute| attribute.name.as_deref() == Some("productType"))
        .map(|attribute| attribute.value.as_str())
        .collect::<Vec<_>>();
    if product_type.as_slice() != [Some("S2MSI2A")] {
        return Err(LiveStop::code("LIVE_METADATA_IDENTITY_REJECTED"));
    }
    let (checksum_algorithm, checksum_value) = selected_checksum(&product.checksums)?;
    Ok(LiveMetadata {
        product_id: product_id.to_owned(),
        display_name: display_name.to_owned(),
        expected_size_bytes,
        checksum_algorithm,
        checksum_value,
    })
}

fn metadata_from_exact_collection_page(
    payload: Value,
    requested_product_id: &str,
    max_bytes: u64,
) -> Result<LiveMetadata, LiveStop> {
    let page = serde_json::from_value::<ODataPage>(payload)
        .map_err(|_| LiveStop::code("LIVE_METADATA_INVALID"))?;
    if page.matched_count != Some(1) || page.next_link.is_some() || page.value.len() != 1 {
        return Err(LiveStop::code("LIVE_METADATA_IDENTITY_REJECTED"));
    }
    let product = serde_json::from_value::<ODataProduct>(
        page.value
            .into_iter()
            .next()
            .expect("one validated metadata record"),
    )
    .map_err(|_| LiveStop::code("LIVE_METADATA_INVALID"))?;
    validate_live_metadata(product, requested_product_id, max_bytes, true)
}

fn fetch_live_metadata(
    client: &Client,
    product_id: &str,
    max_bytes: u64,
) -> Result<LiveMetadata, LiveStop> {
    let url = exact_catalogue_metadata_url(product_id)
        .ok_or_else(|| LiveStop::code("LIVE_INTERNAL_ERROR"))?;
    let response = client
        .get(url.clone())
        .send()
        .map_err(|_| LiveStop::code("LIVE_METADATA_UNAVAILABLE"))?;
    let status = response.status();
    if !status.is_success() || !is_exact_catalogue_metadata_url(response.url(), &url) {
        return Err(LiveStop::catalogue_response(
            "LIVE_METADATA_REJECTED",
            status.as_u16(),
        ));
    }
    let payload = read_bounded_json(response)?;
    metadata_from_exact_collection_page(payload, product_id, max_bytes)
}

fn frozen_snapshot(metadata: &LiveMetadata) -> Result<SnapshotItem, LiveStop> {
    let product_key =
        provider_product_key(CDSE_PROVIDER_ID, CDSE_COLLECTION_ID, &metadata.product_id)
            .map_err(|_| LiveStop::code("LIVE_METADATA_IDENTITY_REJECTED"))?;
    let archive_key = stable_asset_key(&product_key, CDSE_ARCHIVE_ASSET_KEY)
        .map_err(|_| LiveStop::code("LIVE_METADATA_IDENTITY_REJECTED"))?;
    let item = SnapshotItem {
        item_id: product_key.clone(),
        provider_id: CDSE_PROVIDER_ID.to_owned(),
        display_name: metadata.display_name.clone(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::from_iter([
            (
                "logical_dataset".to_owned(),
                Value::String("optical.sentinel-2.l2a".to_owned()),
            ),
            (
                "native_collection_id".to_owned(),
                Value::String(CDSE_COLLECTION_ID.to_owned()),
            ),
            (
                "collection_profile_id".to_owned(),
                Value::String(SENTINEL2_L2A_PROFILE_ID.to_owned()),
            ),
            (
                "identity_rule_id".to_owned(),
                Value::String(CDSE_SENTINEL2_L2A_IDENTITY_RULE_ID.to_owned()),
            ),
            (
                "filename_rule_id".to_owned(),
                Value::String(CDSE_SENTINEL2_L2A_ARCHIVE_FILENAME_RULE_ID.to_owned()),
            ),
            (
                "delivery_model".to_owned(),
                Value::String("archive_first".to_owned()),
            ),
            (
                "provider_product_id".to_owned(),
                Value::String(metadata.product_id.clone()),
            ),
            (
                "provider_product_key".to_owned(),
                Value::String(product_key),
            ),
            (
                "properties".to_owned(),
                json!({
                    "product_type": "S2MSI2A",
                    "online": true,
                    "checksums": [{"algorithm": metadata.checksum_algorithm, "value": metadata.checksum_value}]
                }),
            ),
            (
                "assets".to_owned(),
                json!([{
                    "asset_key": CDSE_ARCHIVE_ASSET_KEY,
                    "stable_asset_key": archive_key,
                    "role": "primary",
                    "native_roles": ["archive"],
                    "size_bytes": metadata.expected_size_bytes
                }]),
            ),
        ]),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    item.validate()
        .map_err(|_| LiveStop::code("LIVE_METADATA_INTEGRITY_REJECTED"))?;
    Ok(item)
}

fn checked_destination(directory: &Path, file_name: &str) -> Result<PathBuf, LiveStop> {
    let destination = directory.join(file_name);
    if destination.exists() || part_path(&destination).exists() {
        return Err(LiveStop::code("LIVE_DESTINATION_OCCUPIED"));
    }
    Ok(destination)
}

fn url_classes(url: &Url) -> (&'static str, &'static str) {
    let host_class = match url.host_str() {
        Some("download.dataspace.copernicus.eu") => "approved-download",
        Some("catalogue.dataspace.copernicus.eu") => "catalogue",
        Some(_) => "other",
        None => "invalid",
    };
    let path_class = if url.path().ends_with("/$value") {
        "value"
    } else if url.path().starts_with("/odata/v1/") {
        "odata-other"
    } else {
        "other"
    };
    (host_class, path_class)
}

fn location_classes(response: &Response, initial: &Url) -> (&'static str, &'static str) {
    let Some(location) = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= MAX_PREFLIGHT_HEADER_BYTES)
    else {
        return ("missing", "missing");
    };
    let Ok(target) = initial.join(location) else {
        return ("invalid", "invalid");
    };
    url_classes(&target)
}

fn preflight_header_text(value: Option<&HeaderValue>) -> PreflightHeaderText<'_> {
    match value {
        None => PreflightHeaderText::Missing,
        Some(value) => match value.to_str() {
            Ok(value) => PreflightHeaderText::Text(value),
            Err(_) => PreflightHeaderText::NonUtf8,
        },
    }
}

fn strict_u64(value: &str) -> Option<u64> {
    (!value.is_empty()
        && value.len() <= MAX_PREFLIGHT_HEADER_BYTES
        && value.as_bytes().iter().all(u8::is_ascii_digit))
    .then(|| value.parse::<u64>().ok())
    .flatten()
}

fn parse_content_range(value: &str) -> Option<ParsedContentRange> {
    if !value.is_ascii() || value.len() > MAX_PREFLIGHT_HEADER_BYTES {
        return None;
    }
    let (unit, remaining) = value.split_once(' ')?;
    if unit != "bytes" || remaining.contains(' ') {
        return None;
    }
    let (range, total) = remaining.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = strict_u64(start)?;
    let end = strict_u64(end)?;
    let total = strict_u64(total)?;
    (start <= end).then_some(ParsedContentRange { start, end, total })
}

fn preflight_diagnostic(
    reason: PreflightDiagnosticReason,
    facts: PreflightDiagnosticFacts,
) -> Box<PreflightDiagnostic> {
    Box::new(PreflightDiagnostic { reason, facts })
}

fn initial_preflight_facts(
    response: &Response,
    expected_url: &Url,
    expected_size_bytes: u64,
) -> PreflightDiagnosticFacts {
    let (response_host_class, response_path_class) = url_classes(response.url());
    PreflightDiagnosticFacts {
        status: response.status().as_u16(),
        effective_url_matches: is_exact_url(
            response.url(),
            "download.dataspace.copernicus.eu",
            expected_url.path(),
        ),
        content_disposition_present: response.headers().contains_key(CONTENT_DISPOSITION),
        content_length_present: response.headers().contains_key(CONTENT_LENGTH),
        content_range_present: response.headers().contains_key(CONTENT_RANGE),
        etag_present: response.headers().contains_key(ETAG),
        last_modified_present: response.headers().contains_key(LAST_MODIFIED),
        content_length: None,
        content_range_start: None,
        content_range_end: None,
        content_range_total: None,
        content_range_total_matches_catalogue: None,
        expected_catalogue_content_length: expected_size_bytes,
        filename_class: PreflightFilenameClass::Missing,
        body_bytes_observed: 0,
        response_host_class,
        response_path_class,
    }
}

fn evaluate_preflight_filename(
    display_name: &str,
    content_disposition: PreflightHeaderText<'_>,
    facts: &mut PreflightDiagnosticFacts,
) -> Result<Option<ArchiveFilenameEvidence>, PreflightDiagnosticReason> {
    let header = match content_disposition {
        PreflightHeaderText::Missing => {
            facts.filename_class = PreflightFilenameClass::Missing;
            return Ok(None);
        }
        PreflightHeaderText::NonUtf8 => {
            facts.filename_class = PreflightFilenameClass::Malformed;
            return Err(PreflightDiagnosticReason::ContentDispositionNonUtf8);
        }
        PreflightHeaderText::Text(header) => header,
    };
    match evaluate_archive_filename(display_name, Some(header)) {
        Ok(evidence) => {
            facts.filename_class = PreflightFilenameClass::Exact;
            Ok(Some(evidence))
        }
        Err(error) if error.code == "CDSE_ARCHIVE_FILENAME_CONFLICT" => {
            facts.filename_class = PreflightFilenameClass::Conflicting;
            Err(PreflightDiagnosticReason::FilenameEvidenceRejected)
        }
        Err(_) => {
            facts.filename_class = PreflightFilenameClass::Malformed;
            Err(PreflightDiagnosticReason::FilenameEvidenceRejected)
        }
    }
}

fn derived_archive_filename_evidence(
    display_name: &str,
) -> Result<ArchiveFilenameEvidence, PreflightDiagnosticReason> {
    let basename = format!("{display_name}.zip");
    if !safe_cdse_odata_name(display_name) || !safe_cdse_archive_basename(&basename) {
        return Err(PreflightDiagnosticReason::FilenameEvidenceRejected);
    }
    Ok(ArchiveFilenameEvidence {
        rule_id: CDSE_SENTINEL2_L2A_ARCHIVE_FILENAME_RULE_ID,
        odata_name: display_name.to_owned(),
        basename,
    })
}

fn classify_preflight_headers(
    mut facts: PreflightDiagnosticFacts,
    metadata: &LiveMetadata,
    content_disposition: PreflightHeaderText<'_>,
    content_length: PreflightHeaderText<'_>,
    content_range: PreflightHeaderText<'_>,
) -> Result<PreflightInspection, Box<PreflightDiagnostic>> {
    if !matches!(facts.status, 200 | 206) {
        return Err(preflight_diagnostic(
            PreflightDiagnosticReason::StatusUnexpected,
            facts,
        ));
    }
    if !facts.effective_url_matches {
        return Err(preflight_diagnostic(
            PreflightDiagnosticReason::EffectiveUrlMismatch,
            facts,
        ));
    }

    if facts.status == StatusCode::PARTIAL_CONTENT.as_u16() {
        let parsed = match content_range {
            PreflightHeaderText::Missing => {
                return Err(preflight_diagnostic(
                    PreflightDiagnosticReason::ContentRangeMissing,
                    facts,
                ));
            }
            PreflightHeaderText::NonUtf8 => {
                return Err(preflight_diagnostic(
                    PreflightDiagnosticReason::ContentRangeMalformed,
                    facts,
                ));
            }
            PreflightHeaderText::Text(value) => match parse_content_range(value) {
                Some(value) => value,
                None => {
                    return Err(preflight_diagnostic(
                        PreflightDiagnosticReason::ContentRangeMalformed,
                        facts,
                    ));
                }
            },
        };
        facts.content_range_start = Some(parsed.start);
        facts.content_range_end = Some(parsed.end);
        facts.content_range_total = Some(parsed.total);
        facts.content_range_total_matches_catalogue =
            Some(parsed.total == metadata.expected_size_bytes);
        if parsed.total != metadata.expected_size_bytes {
            return Err(preflight_diagnostic(
                PreflightDiagnosticReason::ContentRangeTotalMismatch,
                facts,
            ));
        }
        if parsed.start != 0 || parsed.end != 0 {
            return Err(preflight_diagnostic(
                PreflightDiagnosticReason::BodyShapeUnexpected,
                facts,
            ));
        }
    } else if let PreflightHeaderText::Text(value) = content_length {
        if let Some(parsed) = strict_u64(value) {
            facts.content_length = Some(parsed);
            if parsed != metadata.expected_size_bytes {
                return Err(preflight_diagnostic(
                    PreflightDiagnosticReason::ContentLengthMismatch,
                    facts,
                ));
            }
        }
    }

    let filename_evidence =
        evaluate_preflight_filename(&metadata.display_name, content_disposition, &mut facts)
            .map_err(|reason| preflight_diagnostic(reason, facts.clone()))?
            .map_or_else(
                || derived_archive_filename_evidence(&metadata.display_name),
                Ok,
            )
            .map_err(|reason| preflight_diagnostic(reason, facts.clone()))?;
    Ok(PreflightInspection {
        evidence: PreflightEvidence {
            filename_evidence,
            filename_class: facts.filename_class,
        },
        facts,
    })
}

fn observe_one_body_byte(reader: &mut impl Read) -> Result<u8, ()> {
    let mut one_byte = [0_u8; 1];
    reader
        .read(&mut one_byte)
        .map(|count| count as u8)
        .map_err(|_| ())
}

fn inspect_preflight_response(
    response: &mut Response,
    expected_url: &Url,
    metadata: &LiveMetadata,
) -> Result<PreflightInspection, Box<PreflightDiagnostic>> {
    let facts = initial_preflight_facts(response, expected_url, metadata.expected_size_bytes);
    let mut inspection = {
        let headers = response.headers();
        classify_preflight_headers(
            facts,
            metadata,
            preflight_header_text(headers.get(CONTENT_DISPOSITION)),
            preflight_header_text(headers.get(CONTENT_LENGTH)),
            preflight_header_text(headers.get(CONTENT_RANGE)),
        )
    }?;
    let observed = observe_one_body_byte(response).map_err(|_| {
        preflight_diagnostic(
            PreflightDiagnosticReason::BodyShapeUnexpected,
            inspection.facts.clone(),
        )
    })?;
    inspection.facts.body_bytes_observed = observed;
    if observed != 1 {
        return Err(preflight_diagnostic(
            PreflightDiagnosticReason::BodyShapeUnexpected,
            inspection.facts,
        ));
    }
    Ok(inspection)
}

fn bounded_retry_after_seconds(response: &Response) -> Option<u64> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= MAX_PREFLIGHT_HEADER_BYTES && value.is_ascii())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value <= 24 * 60 * 60)
}

fn send_preflight_request(
    client: &Client,
    access_token: &Zeroizing<String>,
    metadata: &LiveMetadata,
) -> Result<(Url, Response), LiveStop> {
    let url = exact_download_product_url(&metadata.product_id)
        .ok_or_else(|| LiveStop::code("LIVE_INTERNAL_ERROR"))?;
    let response = client
        .get(url.clone())
        .bearer_auth(access_token.as_str())
        .header(RANGE, "bytes=0-0")
        .send()
        .map_err(|_| LiveStop::code("LIVE_PREFLIGHT_UNAVAILABLE"))?;
    Ok((url, response))
}

fn preflight_status_stop(response: &Response, initial: &Url) -> Option<LiveStop> {
    let status = response.status();
    if status.is_redirection() {
        let (response_host_class, response_path_class) = location_classes(response, initial);
        return Some(LiveStop {
            code: "LIVE_REDIRECT_REVIEW_REQUIRED",
            status: Some(status.as_u16()),
            retry_after_seconds: None,
            response_host_class,
            response_path_class,
        });
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return Some(LiveStop::response(
            "LIVE_CREDENTIAL_OR_PERMISSION_REJECTED",
            status.as_u16(),
        ));
    }
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        let mut stop = LiveStop::response("LIVE_REMOTE_RETRY_REQUIRED", status.as_u16());
        stop.retry_after_seconds = bounded_retry_after_seconds(response);
        return Some(stop);
    }
    None
}

fn run_live_preflight_diagnostic() -> LivePreflightDiagnosticOutcome {
    let input = match live_input_from_environment() {
        Ok(input) => input,
        Err(stop) => return LivePreflightDiagnosticOutcome::Stopped(stop),
    };
    let client = match live_client() {
        Ok(client) => client,
        Err(stop) => return LivePreflightDiagnosticOutcome::Stopped(stop),
    };
    let metadata = match fetch_live_metadata(&client, &input.product_id, input.max_bytes) {
        Ok(metadata) => metadata,
        Err(stop) => return LivePreflightDiagnosticOutcome::Stopped(stop),
    };
    let (url, mut response) = match send_preflight_request(&client, &input.access_token, &metadata)
    {
        Ok(value) => value,
        Err(stop) => return LivePreflightDiagnosticOutcome::Stopped(stop),
    };
    if let Some(stop) = preflight_status_stop(&response, &url) {
        return LivePreflightDiagnosticOutcome::Stopped(stop);
    }
    match inspect_preflight_response(&mut response, &url, &metadata) {
        Ok(inspection) => LivePreflightDiagnosticOutcome::Observed(*preflight_diagnostic(
            PreflightDiagnosticReason::EvidenceValid,
            inspection.facts,
        )),
        Err(diagnostic) => LivePreflightDiagnosticOutcome::Observed(*diagnostic),
    }
}

fn direct_archive_filename_evidence(
    metadata: &LiveMetadata,
) -> Result<ArchiveFilenameEvidence, LiveStop> {
    derived_archive_filename_evidence(&metadata.display_name)
        .map_err(|_| LiveStop::code("LIVE_METADATA_INTEGRITY_REJECTED"))
}

fn enter_direct_transfer<T>(
    metadata: LiveMetadata,
    transfer: impl FnOnce(LiveMetadata, ArchiveFilenameEvidence) -> T,
) -> Result<T, LiveStop> {
    let filename_evidence = direct_archive_filename_evidence(&metadata)?;
    Ok(transfer(metadata, filename_evidence))
}

fn live_archive_transfer_options() -> TransferOptions {
    TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: LIVE_ARCHIVE_TRANSFER_TIMEOUT,
        chunk_size: 64 * 1024,
        max_accepted_polls: 1,
        accepted_poll_timeout: LIVE_ARCHIVE_TRANSFER_TIMEOUT,
    }
}

fn run_full_transfer(
    input: LiveInput,
    metadata: LiveMetadata,
    destination: PathBuf,
    filename_evidence: ArchiveFilenameEvidence,
) -> LiveOutcome {
    let archive_basename = filename_evidence.basename.clone();
    let snapshot = match frozen_snapshot(&metadata) {
        Ok(snapshot) => snapshot,
        Err(stop) => return LiveOutcome::Stopped(stop),
    };
    let frozen = match build_offline_cdse_archive_access(
        &snapshot,
        CdseArchiveEvidence {
            filename_evidence: Some(filename_evidence),
            expected_size_bytes: Some(metadata.expected_size_bytes),
            checksum_algorithm: Some(metadata.checksum_algorithm.clone()),
            checksum_value: Some(metadata.checksum_value.clone()),
        },
    ) {
        Ok(frozen) => frozen,
        Err(_) => return LiveOutcome::Stopped(LiveStop::code("LIVE_FROZEN_ACCESS_REJECTED")),
    };
    let credentials = LiveCredentials(input.access_token);
    let cancelled = || false;
    let clock = SystemClock;
    let mut resolvers = AssetResolverRegistry::new();
    if resolvers
        .register(Arc::new(CdseArchiveAssetResolver::new()))
        .is_err()
    {
        return LiveOutcome::Stopped(LiveStop::code("LIVE_INTERNAL_ERROR"));
    }
    let resolved = match resolvers.resolve_frozen(
        &frozen,
        &ResolveContext::new(&credentials, &cancelled, &clock)
            .with_proxy_policy(ProxyPolicy::Direct),
    ) {
        Ok(resolved) => resolved,
        Err(_) => return LiveOutcome::Stopped(LiveStop::code("LIVE_RESOLUTION_REJECTED")),
    };
    let exact_download_url = match exact_download_product_url(&metadata.product_id) {
        Some(url) if resolved.effective_url() == &url => url,
        _ => return LiveOutcome::Stopped(LiveStop::code("LIVE_RESOLUTION_REJECTED")),
    };
    let request = match TransferRequest::new(
        "cdse-live-acceptance",
        &metadata.product_id,
        exact_download_url.as_str(),
        destination.clone(),
        Some(metadata.expected_size_bytes),
    ) {
        Ok(request) => request,
        Err(_) => return LiveOutcome::Stopped(LiveStop::code("LIVE_DESTINATION_UNSAFE")),
    };
    let transfer = match HttpTransfer::new_without_system_proxy(live_archive_transfer_options()) {
        Ok(transfer) => transfer,
        Err(_) => return LiveOutcome::Stopped(LiveStop::code("LIVE_TRANSPORT_UNAVAILABLE")),
    };
    let mut session = transfer.resolved_session();
    let result = transfer.download_resolved(
        &request,
        &resolved,
        AuthRefreshPolicyId::CdseOAuthBearerV1,
        &mut session,
        &cancelled,
        &|| false,
        &mut |_| {},
    );
    match result {
        ResolvedTransferResult::AuthRejected(rejection) => {
            LiveOutcome::Stopped(LiveStop::response(
                "LIVE_CREDENTIAL_OR_PERMISSION_REJECTED",
                rejection.status_code,
            ))
        }
        ResolvedTransferResult::Finished(result)
            if result.outcome == DownloadOutcome::Completed =>
        {
            let checksum =
                match ExpectedChecksum::new(&metadata.checksum_algorithm, &metadata.checksum_value)
                {
                    Ok(checksum) => checksum,
                    Err(_) => return LiveOutcome::Stopped(LiveStop::code("LIVE_INTERNAL_ERROR")),
                };
            let verified = fs::metadata(&destination)
                .map(|value| value.len() == metadata.expected_size_bytes)
                .unwrap_or(false)
                && verify_file_checksum(&destination, &checksum).is_ok()
                && !part_path(&destination).exists();
            if !verified {
                return LiveOutcome::Stopped(LiveStop::code("LIVE_TRANSFER_INTEGRITY_REJECTED"));
            }
            LiveOutcome::Completed(LiveReport {
                product_id: metadata.product_id,
                display_name: metadata.display_name,
                archive_basename,
                final_size_bytes: metadata.expected_size_bytes,
                checksum_algorithm: metadata.checksum_algorithm,
            })
        }
        ResolvedTransferResult::Finished(_) => {
            LiveOutcome::Stopped(LiveStop::code("LIVE_TRANSFER_FAILED"))
        }
    }
}

fn run_live_acceptance() -> LiveOutcome {
    let input = match live_input_from_environment() {
        Ok(input) => input,
        Err(stop) => return LiveOutcome::Stopped(stop),
    };
    let client = match live_client() {
        Ok(client) => client,
        Err(stop) => return LiveOutcome::Stopped(stop),
    };
    let metadata = match fetch_live_metadata(&client, &input.product_id, input.max_bytes) {
        Ok(metadata) => metadata,
        Err(stop) => return LiveOutcome::Stopped(stop),
    };
    let destination = match checked_destination(
        &input.destination_dir,
        &format!("{}.zip", metadata.display_name),
    ) {
        Ok(destination) => destination,
        Err(stop) => return LiveOutcome::Stopped(stop),
    };
    match enter_direct_transfer(metadata, |metadata, filename_evidence| {
        run_full_transfer(input, metadata, destination, filename_evidence)
    }) {
        Ok(outcome) => outcome,
        Err(stop) => LiveOutcome::Stopped(stop),
    }
}

fn format_completed_outcome(report: &LiveReport) -> String {
    format!(
        "CDSE_LIVE_ACCEPTANCE outcome=completed product_id={} product_name={} archive_basename={} catalogue_content_length={} final_bytes={} checksum_algorithm={} checksum_verified=true",
        report.product_id,
        report.display_name,
        report.archive_basename,
        report.final_size_bytes,
        report.final_size_bytes,
        report.checksum_algorithm,
    )
}

fn format_stopped_outcome(stop: LiveStop) -> String {
    format!(
        "CDSE_LIVE_ACCEPTANCE outcome=stopped code={} status={} retry_after_seconds={} response_host_class={} response_path_class={}",
        stop.code,
        stop.status.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        stop.retry_after_seconds
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        stop.response_host_class,
        stop.response_path_class,
    )
}

fn emit_outcome(outcome: &LiveOutcome) {
    match outcome {
        LiveOutcome::Completed(report) => eprintln!("{}", format_completed_outcome(report)),
        LiveOutcome::Stopped(stop) => eprintln!("{}", format_stopped_outcome(*stop)),
    }
}

fn stopped_acceptance_panic_message(stop: LiveStop) -> String {
    format!(
        "CDSE_LIVE_ACCEPTANCE stopped code={} status={}",
        stop.code,
        stop.status
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
    )
}

fn finish_live_acceptance(outcome: LiveOutcome) {
    let stopped = match &outcome {
        LiveOutcome::Completed(_) => None,
        LiveOutcome::Stopped(stop) => Some(*stop),
    };
    emit_outcome(&outcome);
    if let Some(stop) = stopped {
        panic!("{}", stopped_acceptance_panic_message(stop));
    }
}

fn diagnostic_number(value: Option<u64>) -> String {
    value.map_or_else(|| "none".to_owned(), |value| value.to_string())
}

fn format_preflight_diagnostic(diagnostic: &PreflightDiagnostic) -> String {
    let facts = &diagnostic.facts;
    format!(
        "CDSE_LIVE_PREFLIGHT_DIAGNOSTIC outcome=observed reason={} status={} effective_url_matches={} content_disposition_present={} content_length_present={} content_range_present={} etag_present={} last_modified_present={} content_length={} content_range_start={} content_range_end={} content_range_total={} content_range_total_matches_catalogue={} expected_catalogue_content_length={} filename_class={} body_bytes_observed={} response_host_class={} response_path_class={}",
        diagnostic.reason.code(),
        facts.status,
        facts.effective_url_matches,
        facts.content_disposition_present,
        facts.content_length_present,
        facts.content_range_present,
        facts.etag_present,
        facts.last_modified_present,
        diagnostic_number(facts.content_length),
        diagnostic_number(facts.content_range_start),
        diagnostic_number(facts.content_range_end),
        diagnostic_number(facts.content_range_total),
        facts
            .content_range_total_matches_catalogue
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        facts.expected_catalogue_content_length,
        facts.filename_class.as_str(),
        facts.body_bytes_observed,
        facts.response_host_class,
        facts.response_path_class,
    )
}

fn emit_preflight_diagnostic_outcome(outcome: LivePreflightDiagnosticOutcome) {
    match outcome {
        LivePreflightDiagnosticOutcome::Observed(diagnostic) => {
            eprintln!("{}", format_preflight_diagnostic(&diagnostic));
        }
        LivePreflightDiagnosticOutcome::Stopped(stop) => eprintln!(
            "CDSE_LIVE_PREFLIGHT_DIAGNOSTIC outcome=stopped code={} status={} retry_after_seconds={} response_host_class={} response_path_class={}",
            stop.code,
            stop.status.map_or_else(|| "none".to_owned(), |value| value.to_string()),
            stop.retry_after_seconds
                .map_or_else(|| "none".to_owned(), |value| value.to_string()),
            stop.response_host_class,
            stop.response_path_class,
        ),
    }
}

#[test]
fn acceptance_transfer_has_no_proxy_and_a_two_hour_archive_timeout() {
    let options = live_archive_transfer_options();
    assert_eq!(LIVE_CONTROL_TIMEOUT, Duration::from_secs(45));
    assert_eq!(
        LIVE_ARCHIVE_TRANSFER_TIMEOUT,
        Duration::from_secs(2 * 60 * 60)
    );
    assert_eq!(options.max_attempts, 1);
    assert_eq!(options.timeout, LIVE_ARCHIVE_TRANSFER_TIMEOUT);
    assert_eq!(options.accepted_poll_timeout, LIVE_ARCHIVE_TRANSFER_TIMEOUT);
    let _transfer =
        HttpTransfer::new_without_system_proxy(options).expect("acceptance transfer constructor");
}

#[test]
fn live_harness_uses_only_fixed_catalogue_and_resolver_derived_value_origins() {
    let product_id = "00000000-0000-4000-8000-000000000003";
    let catalogue = exact_catalogue_metadata_url(product_id).expect("catalogue endpoint");
    assert!(is_exact_catalogue_metadata_url(&catalogue, &catalogue));
    let query = catalogue
        .query_pairs()
        .into_owned()
        .collect::<BTreeMap<_, _>>();
    let expected_filter =
        format!("Id eq {product_id} and Collection/Name eq '{CDSE_COLLECTION_ID}'");
    assert_eq!(query.len(), 4);
    assert_eq!(query.get("$filter"), Some(&expected_filter));
    assert_eq!(query.get("$expand").map(String::as_str), Some("Attributes"));
    assert_eq!(query.get("$count").map(String::as_str), Some("true"));
    assert_eq!(query.get("$top").map(String::as_str), Some("2"));
    assert!(exact_catalogue_metadata_url("not-a-uuid").is_none());
    let download = exact_download_product_url(product_id).expect("resolver-derived endpoint");
    assert!(is_exact_url(
        &download,
        "download.dataspace.copernicus.eu",
        &format!("/odata/v1/Products({product_id})/$value"),
    ));
}

fn metadata_record(value: Value) -> ODataProduct {
    serde_json::from_value(value).expect("synthetic metadata shape")
}

fn diagnostic_metadata() -> LiveMetadata {
    LiveMetadata {
        product_id: "00000000-0000-4000-8000-000000000005".to_owned(),
        display_name: "S2_PREFLIGHT_DIAGNOSTIC_FIXTURE.SAFE".to_owned(),
        expected_size_bytes: 2048,
        checksum_algorithm: "SHA256".to_owned(),
        checksum_value: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_owned(),
    }
}

fn diagnostic_header_present(header: PreflightHeaderText<'_>) -> bool {
    !matches!(header, PreflightHeaderText::Missing)
}

fn diagnostic_facts(
    status: u16,
    effective_url_matches: bool,
    content_disposition: PreflightHeaderText<'_>,
    content_length: PreflightHeaderText<'_>,
    content_range: PreflightHeaderText<'_>,
) -> PreflightDiagnosticFacts {
    let (response_host_class, response_path_class) = if effective_url_matches {
        ("approved-download", "value")
    } else {
        ("other", "other")
    };
    PreflightDiagnosticFacts {
        status,
        effective_url_matches,
        content_disposition_present: diagnostic_header_present(content_disposition),
        content_length_present: diagnostic_header_present(content_length),
        content_range_present: diagnostic_header_present(content_range),
        etag_present: true,
        last_modified_present: true,
        content_length: None,
        content_range_start: None,
        content_range_end: None,
        content_range_total: None,
        content_range_total_matches_catalogue: None,
        expected_catalogue_content_length: 2048,
        filename_class: PreflightFilenameClass::Missing,
        body_bytes_observed: 0,
        response_host_class,
        response_path_class,
    }
}

fn classify_diagnostic_headers(
    status: u16,
    effective_url_matches: bool,
    content_disposition: PreflightHeaderText<'_>,
    content_length: PreflightHeaderText<'_>,
    content_range: PreflightHeaderText<'_>,
) -> Result<PreflightInspection, Box<PreflightDiagnostic>> {
    let metadata = diagnostic_metadata();
    let facts = diagnostic_facts(
        status,
        effective_url_matches,
        content_disposition,
        content_length,
        content_range,
    );
    classify_preflight_headers(
        facts,
        &metadata,
        content_disposition,
        content_length,
        content_range,
    )
}

const DIAGNOSTIC_CONTENT_DISPOSITION: &str =
    "attachment; filename=\"S2_PREFLIGHT_DIAGNOSTIC_FIXTURE.SAFE.zip\"";

#[test]
fn preflight_diagnostic_classifies_partial_content_range_evidence() {
    let exact = classify_diagnostic_headers(
        206,
        true,
        PreflightHeaderText::Text(DIAGNOSTIC_CONTENT_DISPOSITION),
        PreflightHeaderText::Missing,
        PreflightHeaderText::Text("bytes 0-0/2048"),
    )
    .expect("exact partial preflight evidence is accepted");
    assert_eq!(exact.facts.content_range_start, Some(0));
    assert_eq!(exact.facts.content_range_end, Some(0));
    assert_eq!(exact.facts.content_range_total, Some(2048));
    assert_eq!(
        exact.facts.content_range_total_matches_catalogue,
        Some(true)
    );
    assert_eq!(
        exact.evidence.filename_evidence.basename,
        "S2_PREFLIGHT_DIAGNOSTIC_FIXTURE.SAFE.zip"
    );

    let derived = classify_diagnostic_headers(
        206,
        true,
        PreflightHeaderText::Missing,
        PreflightHeaderText::Missing,
        PreflightHeaderText::Text("bytes 0-0/2048"),
    )
    .expect("a missing corroborating filename falls back to the safe OData name");
    assert_eq!(
        derived.evidence.filename_class,
        PreflightFilenameClass::Missing
    );
    assert_eq!(
        derived.evidence.filename_evidence.basename,
        "S2_PREFLIGHT_DIAGNOSTIC_FIXTURE.SAFE.zip"
    );

    for (content_range, reason) in [
        (
            PreflightHeaderText::Missing,
            PreflightDiagnosticReason::ContentRangeMissing,
        ),
        (
            PreflightHeaderText::Text("bytes nope"),
            PreflightDiagnosticReason::ContentRangeMalformed,
        ),
        (
            PreflightHeaderText::NonUtf8,
            PreflightDiagnosticReason::ContentRangeMalformed,
        ),
        (
            PreflightHeaderText::Text("bytes 1-1/2048"),
            PreflightDiagnosticReason::BodyShapeUnexpected,
        ),
        (
            PreflightHeaderText::Text("bytes 0-1/2048"),
            PreflightDiagnosticReason::BodyShapeUnexpected,
        ),
        (
            PreflightHeaderText::Text("bytes 0-0/2047"),
            PreflightDiagnosticReason::ContentRangeTotalMismatch,
        ),
    ] {
        let diagnostic = classify_diagnostic_headers(
            206,
            true,
            PreflightHeaderText::Text(DIAGNOSTIC_CONTENT_DISPOSITION),
            PreflightHeaderText::Missing,
            content_range,
        )
        .err()
        .expect("invalid partial preflight evidence is rejected");
        assert_eq!(diagnostic.reason, reason);
    }
}

#[test]
fn preflight_diagnostic_treats_full_response_length_as_optional_but_checks_conflicts() {
    let exact = classify_diagnostic_headers(
        200,
        true,
        PreflightHeaderText::Text(DIAGNOSTIC_CONTENT_DISPOSITION),
        PreflightHeaderText::Text("2048"),
        PreflightHeaderText::Missing,
    )
    .expect("exact full-response preflight evidence is accepted");
    assert_eq!(exact.facts.content_length, Some(2048));

    for content_length in [
        PreflightHeaderText::Missing,
        PreflightHeaderText::Text("two-kib"),
        PreflightHeaderText::NonUtf8,
    ] {
        let optional = classify_diagnostic_headers(
            200,
            true,
            PreflightHeaderText::Missing,
            content_length,
            PreflightHeaderText::Missing,
        )
        .expect("a missing or unusable full-response length is advisory");
        assert_eq!(optional.facts.content_length, None);
        assert_eq!(
            optional.evidence.filename_class,
            PreflightFilenameClass::Missing
        );
    }

    let mismatch = classify_diagnostic_headers(
        200,
        true,
        PreflightHeaderText::Text(DIAGNOSTIC_CONTENT_DISPOSITION),
        PreflightHeaderText::Text("2047"),
        PreflightHeaderText::Missing,
    )
    .err()
    .expect("a present parseable full-response length conflict is rejected");
    assert_eq!(
        mismatch.reason,
        PreflightDiagnosticReason::ContentLengthMismatch
    );
}

#[test]
fn preflight_diagnostic_classifies_filename_url_and_status_without_raw_headers() {
    let missing = classify_diagnostic_headers(
        200,
        true,
        PreflightHeaderText::Missing,
        PreflightHeaderText::Text("2048"),
        PreflightHeaderText::Missing,
    )
    .expect("missing content disposition derives only the exact safe OData basename");
    assert_eq!(
        missing.evidence.filename_class,
        PreflightFilenameClass::Missing
    );
    assert_eq!(
        missing.evidence.filename_evidence.basename,
        "S2_PREFLIGHT_DIAGNOSTIC_FIXTURE.SAFE.zip"
    );

    let non_utf8 = classify_diagnostic_headers(
        200,
        true,
        PreflightHeaderText::NonUtf8,
        PreflightHeaderText::Text("2048"),
        PreflightHeaderText::Missing,
    )
    .err()
    .expect("non-UTF8 content disposition is rejected");
    assert_eq!(
        non_utf8.reason,
        PreflightDiagnosticReason::ContentDispositionNonUtf8
    );
    assert_eq!(
        non_utf8.facts.filename_class,
        PreflightFilenameClass::Malformed
    );

    let conflicting = classify_diagnostic_headers(
        200,
        true,
        PreflightHeaderText::Text("attachment; filename=\"S2_OTHER_FIXTURE.SAFE.zip\""),
        PreflightHeaderText::Text("2048"),
        PreflightHeaderText::Missing,
    )
    .err()
    .expect("safe conflicting filename is rejected");
    assert_eq!(
        conflicting.reason,
        PreflightDiagnosticReason::FilenameEvidenceRejected
    );
    assert_eq!(
        conflicting.facts.filename_class,
        PreflightFilenameClass::Conflicting
    );

    let malformed = classify_diagnostic_headers(
        200,
        true,
        PreflightHeaderText::Text("inline; filename=\"S2_PREFLIGHT_DIAGNOSTIC_FIXTURE.SAFE.zip\""),
        PreflightHeaderText::Text("2048"),
        PreflightHeaderText::Missing,
    )
    .err()
    .expect("malformed content disposition is rejected");
    assert_eq!(
        malformed.reason,
        PreflightDiagnosticReason::FilenameEvidenceRejected
    );
    assert_eq!(
        malformed.facts.filename_class,
        PreflightFilenameClass::Malformed
    );

    let url_mismatch = classify_diagnostic_headers(
        200,
        false,
        PreflightHeaderText::Text(DIAGNOSTIC_CONTENT_DISPOSITION),
        PreflightHeaderText::Text("2048"),
        PreflightHeaderText::Missing,
    )
    .err()
    .expect("effective URL mismatch is rejected");
    assert_eq!(
        url_mismatch.reason,
        PreflightDiagnosticReason::EffectiveUrlMismatch
    );

    let status = classify_diagnostic_headers(
        418,
        true,
        PreflightHeaderText::Text(DIAGNOSTIC_CONTENT_DISPOSITION),
        PreflightHeaderText::Text("2048"),
        PreflightHeaderText::Missing,
    )
    .err()
    .expect("unexpected status is rejected");
    assert_eq!(status.reason, PreflightDiagnosticReason::StatusUnexpected);
}

#[test]
fn direct_acceptance_enters_transfer_without_a_range_preflight() {
    let metadata = diagnostic_metadata();
    let mut transfer_started = false;
    let result = enter_direct_transfer(metadata, |metadata, evidence| {
        transfer_started = true;
        (metadata, evidence)
    });
    let (metadata, evidence) = result.expect("valid metadata enters the sealed transfer callback");
    assert!(transfer_started);
    assert_eq!(
        metadata.display_name,
        "S2_PREFLIGHT_DIAGNOSTIC_FIXTURE.SAFE"
    );
    assert_eq!(
        evidence.basename,
        "S2_PREFLIGHT_DIAGNOSTIC_FIXTURE.SAFE.zip"
    );

    let mut rejected_transfer_started = false;
    let invalid_metadata = LiveMetadata {
        display_name: "S2_INVALID_NAME".to_owned(),
        ..diagnostic_metadata()
    };
    let rejected = enter_direct_transfer(invalid_metadata, |_, _| {
        rejected_transfer_started = true;
    });
    assert_eq!(
        rejected
            .expect_err("unsafe filename evidence is rejected before transfer")
            .code,
        "LIVE_METADATA_INTEGRITY_REJECTED"
    );
    assert!(!rejected_transfer_started);

    let source = include_str!("cdse_live_acceptance.rs");
    let acceptance_start = source
        .find("fn run_live_acceptance")
        .expect("live acceptance entry exists");
    let acceptance_end = source[acceptance_start..]
        .find("\nfn format_completed_outcome")
        .map(|offset| acceptance_start + offset)
        .expect("live acceptance ends before output formatting");
    let acceptance_entry = &source[acceptance_start..acceptance_end];
    assert!(acceptance_entry.contains("enter_direct_transfer("));
    assert!(acceptance_entry.contains("run_full_transfer("));
    for forbidden in [
        "send_preflight_request",
        "preflight_archive",
        "inspect_preflight_response",
        "run_live_preflight_diagnostic",
        "RANGE",
    ] {
        assert!(
            !acceptance_entry.contains(forbidden),
            "full acceptance must not invoke {forbidden}"
        );
    }

    let transfer_start = source
        .find("fn run_full_transfer(")
        .expect("direct transfer entry exists");
    let transfer_end = source[transfer_start..]
        .find("\nfn run_live_acceptance")
        .map(|offset| transfer_start + offset)
        .expect("direct transfer ends before acceptance entry");
    let transfer_entry = &source[transfer_start..transfer_end];
    for required in [
        "frozen_snapshot(&metadata)",
        "build_offline_cdse_archive_access",
        "CdseArchiveAssetResolver::new()",
        "resolve_frozen(",
        "exact_download_product_url(&metadata.product_id)",
        "HttpTransfer::new_without_system_proxy",
    ] {
        assert!(
            transfer_entry.contains(required),
            "direct transfer must retain {required}"
        );
    }
}

struct RecordingOneByteReader {
    reads: usize,
    largest_buffer: usize,
    next_result: Result<usize, std::io::ErrorKind>,
}

impl Read for RecordingOneByteReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.reads += 1;
        self.largest_buffer = self.largest_buffer.max(buffer.len());
        match self.next_result {
            Ok(count) => Ok(count),
            Err(kind) => Err(std::io::Error::from(kind)),
        }
    }
}

#[test]
fn preflight_diagnostic_body_observation_is_capped_at_one_byte() {
    let mut reader = RecordingOneByteReader {
        reads: 0,
        largest_buffer: 0,
        next_result: Ok(1),
    };
    assert_eq!(observe_one_body_byte(&mut reader), Ok(1));
    assert_eq!(reader.reads, 1);
    assert_eq!(reader.largest_buffer, 1);

    let mut empty_reader = RecordingOneByteReader {
        reads: 0,
        largest_buffer: 0,
        next_result: Ok(0),
    };
    assert_eq!(observe_one_body_byte(&mut empty_reader), Ok(0));
    assert_eq!(empty_reader.largest_buffer, 1);
}

#[test]
fn preflight_diagnostic_output_is_redacted_and_read_only_by_structure() {
    let diagnostic = PreflightDiagnostic {
        reason: PreflightDiagnosticReason::ContentRangeTotalMismatch,
        facts: PreflightDiagnosticFacts {
            status: 206,
            effective_url_matches: true,
            content_disposition_present: true,
            content_length_present: false,
            content_range_present: true,
            etag_present: true,
            last_modified_present: true,
            content_length: None,
            content_range_start: Some(0),
            content_range_end: Some(0),
            content_range_total: Some(2047),
            content_range_total_matches_catalogue: Some(false),
            expected_catalogue_content_length: 2048,
            filename_class: PreflightFilenameClass::Conflicting,
            body_bytes_observed: 0,
            response_host_class: "approved-download",
            response_path_class: "value",
        },
    };
    let output = format_preflight_diagnostic(&diagnostic);
    let synthetic_token = "SYNTHETIC_CDSE_ACCESS_TOKEN";
    let raw_header = "attachment; filename=\"S2_PRIVATE.SAFE.zip\"";
    assert!(output.contains("PREFLIGHT_CONTENT_RANGE_TOTAL_MISMATCH"));
    assert!(!output.contains(synthetic_token));
    assert!(!output.contains(raw_header));
    assert!(!format!("{diagnostic:?}").contains(synthetic_token));
    assert!(!format!("{diagnostic:?}").contains(raw_header));

    let source = include_str!("cdse_live_acceptance.rs");
    let start = source
        .find("fn run_live_preflight_diagnostic")
        .expect("diagnostic entry exists");
    let end = source[start..]
        .find("\nfn direct_archive_filename_evidence")
        .map(|offset| start + offset)
        .expect("diagnostic entry ends before the direct-transfer gate");
    let diagnostic_entry = &source[start..end];
    for forbidden in [
        "HttpTransfer",
        "TransferRequest",
        "checked_destination",
        "frozen_snapshot",
        "build_offline_cdse_archive_access",
    ] {
        assert!(
            !diagnostic_entry.contains(forbidden),
            "diagnostic mode must not invoke {forbidden}"
        );
    }
}

#[test]
fn completed_acceptance_output_has_only_safe_final_facts() {
    let report = LiveReport {
        product_id: "00000000-0000-4000-8000-000000000006".to_owned(),
        display_name: "S2_COMPLETION_FIXTURE.SAFE".to_owned(),
        archive_basename: "S2_COMPLETION_FIXTURE.SAFE.zip".to_owned(),
        final_size_bytes: 2048,
        checksum_algorithm: "SHA256".to_owned(),
    };
    let output = format_completed_outcome(&report);
    let synthetic_token = "SYNTHETIC_CDSE_ACCESS_TOKEN";
    let raw_header = "attachment; filename=\"S2_PRIVATE.SAFE.zip\"";
    let checksum_value = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert!(output.contains("outcome=completed"));
    assert!(output.contains("archive_basename=S2_COMPLETION_FIXTURE.SAFE.zip"));
    assert!(output.contains("catalogue_content_length=2048"));
    assert!(output.contains("final_bytes=2048"));
    assert!(output.contains("checksum_algorithm=SHA256"));
    assert!(output.contains("checksum_verified=true"));
    assert!(!output.contains("preflight_"));
    assert!(!output.contains(synthetic_token));
    assert!(!output.contains(raw_header));
    assert!(!output.contains(checksum_value));
}

#[test]
fn stopped_acceptance_fails_with_only_typed_stop_facts() {
    let stop = LiveStop::response("LIVE_TRANSFER_FAILED", 502);
    let message = stopped_acceptance_panic_message(stop);
    assert!(message.contains("LIVE_TRANSFER_FAILED"));
    assert!(message.contains("status=502"));
    assert!(!message.contains("SYNTHETIC_CDSE_LIVE_BEARER"));

    let failure = std::panic::catch_unwind(|| finish_live_acceptance(LiveOutcome::Stopped(stop)))
        .expect_err("a stopped acceptance must fail the ignored test");
    let panic_message = failure
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| failure.downcast_ref::<&str>().copied())
        .expect("fixed panic message");
    assert_eq!(panic_message, message);
}

#[test]
fn live_metadata_is_strict_but_does_not_carry_s3_or_locator_fields_into_the_snapshot() {
    let product_id = "00000000-0000-4000-8000-000000000001";
    let metadata = validate_live_metadata(
        metadata_record(json!({
            "Id": product_id,
            "Name": "S2_LIVE_ACCEPTANCE_FIXTURE.SAFE",
            "Collection": {"Name": "SENTINEL-2"},
            "ContentLength": 2048,
            "Online": true,
            "S3Path": "s3://untrusted/ignored",
            "Checksum": [{"Algorithm": "SHA256", "Value": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],
            "Attributes": [{"Name": "productType", "Value": "S2MSI2A"}]
        })),
        product_id,
        4096,
        true,
    )
    .expect("safe synthetic metadata");
    assert_eq!(metadata.display_name, "S2_LIVE_ACCEPTANCE_FIXTURE.SAFE");
    assert_eq!(metadata.checksum_algorithm, "SHA256");
    let snapshot = frozen_snapshot(&metadata).expect("frozen snapshot");
    let serialized = serde_json::to_string(&snapshot).expect("snapshot serializes");
    assert!(!serialized.contains("s3://"));
    assert!(!serialized.contains("http://"));
    assert!(!serialized.contains("https://"));
}

#[test]
fn live_metadata_rejects_unsafe_names_duplicate_accepted_checksums_and_byte_cap_overruns() {
    let product_id = "00000000-0000-4000-8000-000000000002";
    let base = json!({
        "Id": product_id,
        "Name": "S2_LIVE_ACCEPTANCE_FIXTURE.SAFE",
        "Collection": {"Name": "SENTINEL-2"},
        "ContentLength": 2048,
        "Online": true,
        "Checksum": [{"Algorithm": "MD5", "Value": "00000000000000000000000000000000"}],
        "Attributes": [{"Name": "productType", "Value": "S2MSI2A"}]
    });

    let mut unsafe_name = base.clone();
    unsafe_name["Name"] = json!("S2_NO_SAFE_SUFFIX");
    let unsafe_name = validate_live_metadata(metadata_record(unsafe_name), product_id, 4096, true)
        .err()
        .expect("unsafe name is rejected");
    assert_eq!(unsafe_name.code, "LIVE_METADATA_INTEGRITY_REJECTED");

    let mut duplicate_checksum = base.clone();
    duplicate_checksum["Checksum"] = json!([
        {"Algorithm": "MD5", "Value": "00000000000000000000000000000000"},
        {"Algorithm": "MD5", "Value": "ffffffffffffffffffffffffffffffff"}
    ]);
    let duplicate_checksum =
        validate_live_metadata(metadata_record(duplicate_checksum), product_id, 4096, true)
            .err()
            .expect("duplicate accepted checksum is rejected");
    assert_eq!(duplicate_checksum.code, "LIVE_METADATA_INTEGRITY_REJECTED");

    let size = validate_live_metadata(metadata_record(base), product_id, 1024, true)
        .err()
        .expect("byte-cap overrun is rejected");
    assert_eq!(size.code, "LIVE_METADATA_SIZE_REJECTED");
}

#[test]
fn live_metadata_uses_bounded_server_proof_when_collection_is_omitted_from_the_record() {
    let product_id = "00000000-0000-4000-8000-000000000004";
    let record = json!({
        "Id": product_id,
        "Name": "S2_LIVE_ACCEPTANCE_FIXTURE.SAFE",
        "ContentLength": 2048,
        "Online": true,
        "Checksum": [{"Algorithm": "SHA256", "Value": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],
        "Attributes": [{"Name": "productType", "Value": "S2MSI2A"}]
    });
    let metadata = metadata_from_exact_collection_page(
        json!({"@odata.count": 1, "value": [record.clone()]}),
        product_id,
        4096,
    )
    .expect("the fixed server-side collection filter proves the omitted field");
    assert_eq!(metadata.product_id, product_id);

    let direct_without_collection =
        validate_live_metadata(metadata_record(record.clone()), product_id, 4096, false)
            .err()
            .expect("an omitted collection needs the fixed query proof");
    assert_eq!(
        direct_without_collection.code,
        "LIVE_METADATA_IDENTITY_REJECTED"
    );

    for payload in [
        json!({"value": [record.clone()]}),
        json!({"@odata.count": 2, "value": [record.clone()]}),
        json!({"@odata.count": 1, "@odata.nextLink": "https://untrusted.invalid", "value": [record.clone()]}),
        json!({"@odata.count": 1, "value": []}),
    ] {
        let error = metadata_from_exact_collection_page(payload, product_id, 4096)
            .err()
            .expect("ambiguous collection proof is rejected");
        assert_eq!(error.code, "LIVE_METADATA_IDENTITY_REJECTED");
    }

    let mismatched_collection = metadata_from_exact_collection_page(
        json!({
            "@odata.count": 1,
            "value": [{
                "Id": product_id,
                "Name": "S2_LIVE_ACCEPTANCE_FIXTURE.SAFE",
                "Collection": {"Name": "SENTINEL-1"},
                "ContentLength": 2048,
                "Online": true,
                "Checksum": [{"Algorithm": "SHA256", "Value": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],
                "Attributes": [{"Name": "productType", "Value": "S2MSI2A"}]
            }]
        }),
        product_id,
        4096,
    )
    .err()
    .expect("an explicit conflicting collection is rejected");
    assert_eq!(
        mismatched_collection.code,
        "LIVE_METADATA_IDENTITY_REJECTED"
    );
}

#[test]
fn live_harness_credential_material_is_redacted_before_any_transport_request() {
    let secret = "SYNTHETIC_CDSE_LIVE_BEARER";
    let credentials = LiveCredentials(Zeroizing::new(secret.to_owned()));
    let material = credentials
        .resolve("copernicus-dataspace-default")
        .expect("synthetic bearer material");
    assert!(!format!("{material:?}").contains(secret));
    assert!(credentials.resolve("other-profile").is_err());
}

#[test]
#[ignore = "requires explicit local CDSE access token and an approved single product"]
fn controlled_single_product_cdse_acceptance() {
    finish_live_acceptance(run_live_acceptance());
}

#[test]
#[ignore = "requires explicit local CDSE access token and a one-shot approved preflight"]
fn controlled_single_product_cdse_preflight_diagnosis() {
    emit_preflight_diagnostic_outcome(run_live_preflight_diagnostic());
}
