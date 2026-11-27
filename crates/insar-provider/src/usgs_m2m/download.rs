//! Closed Landsat Collection 2 Level-2 bundle discovery and M2M queue calls.

use std::fmt;
use std::sync::Arc;

use serde_json::{json, Map, Value};
use zeroize::Zeroizing;

use super::error::{UsgsM2mEndpointClass, UsgsM2mError, UsgsM2mErrorCode};
use super::router::USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY;
use super::session::UsgsM2mSession;

/// Historical USGS bundle option frozen by existing ScenePort tasks.
/// New tasks must select the reviewed bundle semantics from the live option list.
pub const LANDSAT_C2_L2_BUNDLE_OPTION_ID: &str = "5e83d14fec7cae84";
pub const LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME: &str = "Landsat Collection 2 Level-2 Product Bundle";
/// Historical bundle variant retained for old frozen tasks.
pub const LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE: &str = "D694";
pub const LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM: &str = "ls_zip";
/// Current bundle variant observed from the reviewed USGS M2M route.
pub const LANDSAT_C2_L2_CURRENT_BUNDLE_PRODUCT_CODE: &str = "D806";
pub const LANDSAT_C2_L2_CURRENT_BUNDLE_DOWNLOAD_SYSTEM: &str = "dds_ms";
pub const LANDSAT_C2_L2_CURRENT_BUNDLE_OPTION_ID: &str = "632210d4770592cf";
pub const LANDSAT_TM_C2_L2_BUNDLE_PRODUCT_CODE: &str = "D692";
pub const LANDSAT_ETM_C2_L2_BUNDLE_PRODUCT_CODE: &str = "D693";

pub fn landsat_c2_l2_bundle_option_rank(
    option_id: &str,
    product_name: &str,
    product_code: &str,
    download_system: &str,
) -> Option<u8> {
    if product_name != LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME {
        return None;
    }
    match (option_id, product_code, download_system) {
        (
            LANDSAT_C2_L2_CURRENT_BUNDLE_OPTION_ID,
            LANDSAT_C2_L2_CURRENT_BUNDLE_PRODUCT_CODE,
            LANDSAT_C2_L2_CURRENT_BUNDLE_DOWNLOAD_SYSTEM,
        ) => Some(0),
        (
            LANDSAT_C2_L2_BUNDLE_OPTION_ID,
            LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE,
            LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM,
        ) => Some(1),
        (id, LANDSAT_TM_C2_L2_BUNDLE_PRODUCT_CODE, "ls_zip" | "dds_ms")
            if id != LANDSAT_C2_L2_CURRENT_BUNDLE_OPTION_ID
                && id != LANDSAT_C2_L2_BUNDLE_OPTION_ID =>
        {
            Some(2)
        }
        (id, LANDSAT_ETM_C2_L2_BUNDLE_PRODUCT_CODE, "ls_zip" | "dds_ms")
            if id != LANDSAT_C2_L2_CURRENT_BUNDLE_OPTION_ID
                && id != LANDSAT_C2_L2_BUNDLE_OPTION_ID =>
        {
            Some(3)
        }
        _ => None,
    }
}

pub fn is_supported_landsat_c2_l2_bundle_option(
    option_id: &str,
    product_name: &str,
    product_code: &str,
    download_system: &str,
) -> bool {
    landsat_c2_l2_bundle_option_rank(option_id, product_name, product_code, download_system)
        .is_some()
}

const MAX_OPTION_RECORDS: usize = 128;
const MAX_DOWNLOAD_RESULTS: usize = 128;
const MAX_LABEL_BYTES: usize = 96;
const MAX_DOWNLOAD_ID_BYTES: usize = 32;
const MAX_RUNTIME_URL_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsgsM2mBundleAvailability {
    Available,
    Preparable,
}

impl UsgsM2mBundleAvailability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Preparable => "preparable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsgsM2mFullBundleOption {
    pub dataset_alias: String,
    pub dataset_id: String,
    pub entity_id: String,
    pub display_id: String,
    pub option_id: String,
    pub product_name: String,
    pub product_code: String,
    pub download_system: String,
    pub availability: UsgsM2mBundleAvailability,
    pub expected_file_name: String,
    pub expected_size_bytes: Option<u64>,
    pub checksum_algorithm: Option<String>,
    pub checksum_value: Option<String>,
}

pub struct UsgsM2mResolvedDownload {
    download_id: String,
    entity_id: String,
    display_id: Option<String>,
    url: Zeroizing<String>,
    expected_size_bytes: Option<u64>,
    checksum_algorithm: Option<String>,
    checksum_value: Option<String>,
}

impl UsgsM2mResolvedDownload {
    pub fn download_id(&self) -> &str {
        &self.download_id
    }

    pub fn entity_id(&self) -> &str {
        &self.entity_id
    }

    pub fn display_id(&self) -> Option<&str> {
        self.display_id.as_deref()
    }

    pub fn runtime_url(&self) -> &str {
        self.url.as_str()
    }

    pub const fn expected_size_bytes(&self) -> Option<u64> {
        self.expected_size_bytes
    }

    pub fn checksum(&self) -> Option<(&str, &str)> {
        self.checksum_algorithm
            .as_deref()
            .zip(self.checksum_value.as_deref())
    }
}

impl fmt::Debug for UsgsM2mResolvedDownload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsgsM2mResolvedDownload")
            .field("download_id", &self.download_id)
            .field("entity_id", &self.entity_id)
            .field("display_id", &self.display_id)
            .field("url", &"[REDACTED]")
            .field("expected_size_bytes", &self.expected_size_bytes)
            .field("checksum_algorithm", &self.checksum_algorithm)
            .field(
                "checksum_value",
                &self.checksum_value.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsgsM2mPreparingDownload {
    download_id: String,
    entity_id: String,
}

impl UsgsM2mPreparingDownload {
    pub fn download_id(&self) -> &str {
        &self.download_id
    }

    pub fn entity_id(&self) -> &str {
        &self.entity_id
    }
}

#[derive(Debug)]
pub enum UsgsM2mDownloadRequestOutcome {
    Available(UsgsM2mResolvedDownload),
    Preparing(UsgsM2mPreparingDownload),
}

pub struct UsgsM2mDownloadClient {
    session: Arc<UsgsM2mSession>,
}

impl UsgsM2mDownloadClient {
    pub fn new(session: Arc<UsgsM2mSession>) -> Self {
        Self { session }
    }

    pub fn session(&self) -> Arc<UsgsM2mSession> {
        Arc::clone(&self.session)
    }

    pub fn full_bundle_option(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mFullBundleOption, UsgsM2mError> {
        validate_selection(
            dataset_alias,
            entity_id,
            display_id,
            LANDSAT_C2_L2_BUNDLE_OPTION_ID,
        )?;
        let payload = json!({
            "datasetName": dataset_alias,
            "entityIds": [entity_id],
        });
        let data = self.session.request_download_options(&payload, cancelled)?;
        write_resolve_stage_diagnostic("download_options", data.value());
        map_landsat_c2_l2_full_bundle_option(data.value(), dataset_alias, entity_id, display_id)
    }

    pub fn request_full_bundle(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        option_id: &str,
        label: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mDownloadRequestOutcome, UsgsM2mError> {
        validate_selection(dataset_alias, entity_id, display_id, option_id)?;
        validate_label(label, UsgsM2mEndpointClass::DownloadRequest)?;
        let payload = json!({
            "downloads": [{"entityId": entity_id, "productId": option_id}],
            "label": label,
            "returnAvailable": true,
        });
        let data = self.session.request_download(&payload, cancelled)?;
        write_resolve_stage_diagnostic("download_request", data.value());
        parse_download_request(data.value(), entity_id)
    }

    pub fn retrieve_full_bundle(
        &self,
        label: &str,
        preparing: &UsgsM2mPreparingDownload,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<UsgsM2mResolvedDownload>, UsgsM2mError> {
        validate_label(label, UsgsM2mEndpointClass::DownloadRetrieve)?;
        let payload = json!({"label": label});
        let data = self
            .session
            .request_download_retrieve(&payload, cancelled)?;
        write_resolve_stage_diagnostic("download_retrieve", data.value());
        parse_download_retrieve(data.value(), preparing)
    }
}

impl fmt::Debug for UsgsM2mDownloadClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsgsM2mDownloadClient")
            .field("session", &"[RUNTIME SESSION]")
            .finish()
    }
}

#[doc(hidden)]
pub fn map_landsat_c2_l2_full_bundle_option(
    data: &Value,
    dataset_alias: &str,
    entity_id: &str,
    display_id: &str,
) -> Result<UsgsM2mFullBundleOption, UsgsM2mError> {
    validate_selection(
        dataset_alias,
        entity_id,
        display_id,
        LANDSAT_C2_L2_BUNDLE_OPTION_ID,
    )?;
    let identity = super::router::live_identity_for_alias(dataset_alias)
        .or(USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY)
        .ok_or_else(|| option_error(UsgsM2mErrorCode::DownloadOptionInvalid))?;
    let records = data
        .as_array()
        .filter(|records| !records.is_empty() && records.len() <= MAX_OPTION_RECORDS)
        .ok_or_else(|| option_error(UsgsM2mErrorCode::ResponseInvalid))?;
    let mut matching = Vec::new();
    for record in records {
        let record = record
            .as_object()
            .ok_or_else(|| option_error(UsgsM2mErrorCode::ResponseInvalid))?;
        let record_entity = required_text(record, "entityId")?;
        let record_display = required_text(record, "displayId")?;
        let dataset_id = required_text(record, "datasetId")?;
        if record_entity != entity_id
            || record_display != display_id
            || (dataset_id != identity.dataset_id()
                && dataset_id != "5e83d14f2fc39685"
                && dataset_id != "5e83d14f4e24ef5c"
                && dataset_id != "5e83d14fb8e96503")
        {
            return Err(option_error(UsgsM2mErrorCode::DownloadIdentityConflict));
        }
        let option_id = required_text(record, "id")?;
        let product_name = required_text(record, "productName")?;
        let product_code = required_text(record, "productCode")?;
        let download_system = required_text(record, "downloadSystem")?;
        let Some(rank) = landsat_c2_l2_bundle_option_rank(
            option_id,
            product_name,
            product_code,
            download_system,
        ) else {
            continue;
        };
        if !valid_identifier(option_id)
            || record
                .get("downloadName")
                .is_some_and(|value| !value.is_null())
            || record
                .get("fileGroups")
                .is_some_and(|value| !value.is_null())
            || record
                .get("secondaryDownloads")
                .is_some_and(|value| !value.is_array())
        {
            return Err(option_error(UsgsM2mErrorCode::DownloadOptionInvalid));
        }
        let available = required_bool(record, "available")?;
        let bulk_available = required_bool(record, "bulkAvailable")?;
        let availability = match (available, bulk_available) {
            (true, _) => UsgsM2mBundleAvailability::Available,
            (false, true) => UsgsM2mBundleAvailability::Preparable,
            (false, false) => continue,
        };
        matching.push((
            rank,
            record,
            availability,
            option_id,
            product_name,
            product_code,
            download_system,
        ));
    }
    if matching.is_empty() {
        return Err(option_error(UsgsM2mErrorCode::DownloadUnavailable));
    }
    matching.sort_by_key(|candidate| candidate.0);
    if matching
        .get(1)
        .is_some_and(|candidate| candidate.0 == matching[0].0)
    {
        return Err(option_error(UsgsM2mErrorCode::DownloadOptionAmbiguous));
    }
    let (_, record, availability, option_id, product_name, product_code, download_system) =
        matching[0];
    let expected_size_bytes = record
        .get("filesize")
        .and_then(Value::as_u64)
        .filter(|size| *size > 0);
    let (checksum_algorithm, checksum_value) = parse_checksum_array(record.get("checksum"))?;
    if expected_size_bytes.is_none() && checksum_algorithm.is_none() {
        return Err(option_error(UsgsM2mErrorCode::DownloadIntegrityMissing));
    }

    Ok(UsgsM2mFullBundleOption {
        dataset_alias: dataset_alias.to_owned(),
        dataset_id: identity.dataset_id().to_owned(),
        entity_id: entity_id.to_owned(),
        display_id: display_id.to_owned(),
        option_id: option_id.to_owned(),
        product_name: product_name.to_owned(),
        product_code: product_code.to_owned(),
        download_system: download_system.to_owned(),
        availability,
        expected_file_name: format!("{display_id}.tar"),
        expected_size_bytes,
        checksum_algorithm,
        checksum_value,
    })
}

fn parse_download_request(
    data: &Value,
    entity_id: &str,
) -> Result<UsgsM2mDownloadRequestOutcome, UsgsM2mError> {
    parse_download_request_inner(data, entity_id)
        .map_err(|error| error.with_response_shape(response_shape(data)))
}

fn parse_download_request_inner(
    data: &Value,
    entity_id: &str,
) -> Result<UsgsM2mDownloadRequestOutcome, UsgsM2mError> {
    let object = data
        .as_object()
        .ok_or_else(|| request_error(UsgsM2mErrorCode::ResponseInvalid))?;
    require_empty_array(object, "failed", UsgsM2mEndpointClass::DownloadRequest)?;
    let available = bounded_array(
        object,
        "availableDownloads",
        UsgsM2mEndpointClass::DownloadRequest,
    )?;
    let preparing = bounded_array(
        object,
        "preparingDownloads",
        UsgsM2mEndpointClass::DownloadRequest,
    )?;
    if available.len() + preparing.len() != 1 {
        return Err(request_error(UsgsM2mErrorCode::DownloadIdentityConflict));
    }
    if let Some(value) = available.first() {
        return parse_resolved_download(
            value,
            entity_id,
            None,
            UsgsM2mEndpointClass::DownloadRequest,
        )
        .map(UsgsM2mDownloadRequestOutcome::Available);
    }
    parse_preparing_download(
        preparing
            .first()
            .expect("one request result was established above"),
        entity_id,
    )
    .map(UsgsM2mDownloadRequestOutcome::Preparing)
}

fn parse_download_retrieve(
    data: &Value,
    preparing: &UsgsM2mPreparingDownload,
) -> Result<Option<UsgsM2mResolvedDownload>, UsgsM2mError> {
    parse_download_retrieve_inner(data, preparing)
        .map_err(|error| error.with_response_shape(response_shape(data)))
}

fn parse_download_retrieve_inner(
    data: &Value,
    preparing: &UsgsM2mPreparingDownload,
) -> Result<Option<UsgsM2mResolvedDownload>, UsgsM2mError> {
    let object = data
        .as_object()
        .ok_or_else(|| retrieve_error(UsgsM2mErrorCode::ResponseInvalid))?;
    let mut matches = Vec::new();
    let mut observed = 0usize;
    for key in ["available", "requested"] {
        for value in bounded_array(object, key, UsgsM2mEndpointClass::DownloadRetrieve)? {
            observed += 1;
            let Some(download_id) = value.as_object().and_then(|item| item.get("downloadId"))
            else {
                return Err(retrieve_error(UsgsM2mErrorCode::ResponseInvalid));
            };
            if normalized_download_id(download_id).as_deref() == Some(preparing.download_id()) {
                matches.push(value);
            }
        }
    }
    if observed != matches.len() {
        return Err(retrieve_error(UsgsM2mErrorCode::DownloadIdentityConflict));
    }
    match matches.as_slice() {
        [] => Ok(None),
        [value] => parse_resolved_download(
            value,
            preparing.entity_id(),
            Some(preparing.download_id()),
            UsgsM2mEndpointClass::DownloadRetrieve,
        )
        .map(Some),
        _ => Err(retrieve_error(UsgsM2mErrorCode::DownloadIdentityConflict)),
    }
}

fn parse_preparing_download(
    value: &Value,
    expected_entity_id: &str,
) -> Result<UsgsM2mPreparingDownload, UsgsM2mError> {
    let object = value
        .as_object()
        .ok_or_else(|| request_error(UsgsM2mErrorCode::ResponseInvalid))?;
    let entity_id = required_text_at(object, "entityId", UsgsM2mEndpointClass::DownloadRequest)?;
    let download_id = object
        .get("downloadId")
        .and_then(normalized_download_id)
        .ok_or_else(|| request_error(UsgsM2mErrorCode::ResponseInvalid))?;
    if entity_id != expected_entity_id {
        return Err(request_error(UsgsM2mErrorCode::DownloadIdentityConflict));
    }
    Ok(UsgsM2mPreparingDownload {
        download_id,
        entity_id: entity_id.to_owned(),
    })
}

fn parse_resolved_download(
    value: &Value,
    expected_entity_id: &str,
    expected_download_id: Option<&str>,
    endpoint: UsgsM2mEndpointClass,
) -> Result<UsgsM2mResolvedDownload, UsgsM2mError> {
    let object = value
        .as_object()
        .ok_or_else(|| UsgsM2mError::new(UsgsM2mErrorCode::ResponseInvalid, endpoint))?;
    let entity_id = required_text_at(object, "entityId", endpoint)?;
    let download_id = object
        .get("downloadId")
        .and_then(normalized_download_id)
        .ok_or_else(|| UsgsM2mError::new(UsgsM2mErrorCode::ResponseInvalid, endpoint))?;
    if entity_id != expected_entity_id
        || expected_download_id.is_some_and(|expected| expected != download_id)
    {
        return Err(UsgsM2mError::new(
            UsgsM2mErrorCode::DownloadIdentityConflict,
            endpoint,
        ));
    }
    let url = required_text_at(object, "url", endpoint)?;
    if url.len() > MAX_RUNTIME_URL_BYTES || url.chars().any(char::is_control) {
        return Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ResponseInvalid,
            endpoint,
        ));
    }
    let display_id = match object.get("displayId") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if valid_identifier(value) => Some(value.to_owned()),
        Some(_) => {
            return Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ResponseInvalid,
                endpoint,
            ))
        }
    };
    let expected_size_bytes = object
        .get("filesize")
        .or_else(|| object.get("fileSize"))
        .and_then(Value::as_u64)
        .filter(|size| *size > 0);
    let (checksum_algorithm, checksum_value) = parse_checksum_array(object.get("checksum_values"))?;
    Ok(UsgsM2mResolvedDownload {
        download_id,
        entity_id: entity_id.to_owned(),
        display_id,
        url: Zeroizing::new(url.to_owned()),
        expected_size_bytes,
        checksum_algorithm,
        checksum_value,
    })
}

const MAX_RESPONSE_SHAPE_CHARS: usize = 512;

/// A key or code fit for a persisted diagnostic: short ASCII identifier
/// characters only, anything else is replaced, never echoed.
fn shape_token(value: &str) -> &str {
    if !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        value
    } else {
        "?"
    }
}

/// Scheme, host and path shape of a download URL: every path segment becomes
/// `*` (keeping a short alphanumeric extension) and a query is only flagged.
fn url_shape(value: &str) -> String {
    let Ok(url) = url::Url::parse(value) else {
        return "unparsable".to_owned();
    };
    let mut shape = format!("{}://{}", url.scheme(), url.host_str().unwrap_or("?"));
    for segment in url.path_segments().into_iter().flatten() {
        shape.push_str("/*");
        if let Some((_, extension)) = segment.rsplit_once('.') {
            if (1..=5).contains(&extension.len())
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                shape.push('.');
                shape.push_str(extension);
            }
        }
    }
    if url.query().is_some() {
        shape.push_str("?query");
    }
    shape
}

/// Sanitized shape of a download-request or download-retrieve answer, kept
/// with the failure so a task that fails at the start can be reproduced:
/// top-level key names, array lengths, `errorCode` values and URL shapes.
/// Labels, identifiers, URL text and every other value are never recorded.
fn response_shape(data: &Value) -> String {
    let Some(object) = data.as_object() else {
        return "non_object".to_owned();
    };
    let mut keys = object.keys().collect::<Vec<_>>();
    keys.sort();
    let mut parts = vec![format!(
        "keys={}",
        keys.iter()
            .map(|key| shape_token(key))
            .collect::<Vec<_>>()
            .join(",")
    )];
    for key in keys {
        let Some(items) = object.get(key).and_then(Value::as_array) else {
            continue;
        };
        let key = shape_token(key);
        parts.push(format!("{key}[{}]", items.len()));
        let mut codes = std::collections::BTreeSet::new();
        let mut urls = std::collections::BTreeSet::new();
        for item in items
            .iter()
            .take(MAX_DOWNLOAD_RESULTS)
            .filter_map(Value::as_object)
        {
            if let Some(code) = item.get("errorCode").and_then(Value::as_str) {
                codes.insert(shape_token(code).to_owned());
            }
            if let Some(url) = item.get("url").and_then(Value::as_str) {
                urls.insert(url_shape(url));
            }
        }
        if !codes.is_empty() {
            parts.push(format!(
                "{key}.errorCode={}",
                codes.into_iter().collect::<Vec<_>>().join("|")
            ));
        }
        if !urls.is_empty() {
            parts.push(format!(
                "{key}.url={}",
                urls.into_iter().collect::<Vec<_>>().join("|")
            ));
        }
    }
    let mut shape = parts.join(";");
    if shape.len() > MAX_RESPONSE_SHAPE_CHARS {
        let mut end = MAX_RESPONSE_SHAPE_CHARS;
        while !shape.is_char_boundary(end) {
            end -= 1;
        }
        shape.truncate(end);
        shape.push_str("...");
    }
    shape
}

fn validate_selection(
    dataset_alias: &str,
    entity_id: &str,
    display_id: &str,
    option_id: &str,
) -> Result<(), UsgsM2mError> {
    let identity = super::router::live_identity_for_alias(dataset_alias)
        .or(USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY)
        .ok_or_else(|| option_error(UsgsM2mErrorCode::DownloadOptionInvalid))?;
    if (dataset_alias != identity.dataset_alias()
        && dataset_alias != "landsat_ot_c2_l2"
        && dataset_alias != "landsat_tm_c2_l2"
        && dataset_alias != "landsat_etm_c2_l2")
        || !valid_identifier(option_id)
        || !valid_identifier(entity_id)
        || !valid_identifier(display_id)
    {
        return Err(option_error(UsgsM2mErrorCode::DownloadIdentityConflict));
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value == value.trim()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn validate_label(label: &str, endpoint: UsgsM2mEndpointClass) -> Result<(), UsgsM2mError> {
    if label.is_empty()
        || label.len() > MAX_LABEL_BYTES
        || label != label.trim()
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(UsgsM2mError::new(UsgsM2mErrorCode::ConfigInvalid, endpoint));
    }
    Ok(())
}

fn required_text<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, UsgsM2mError> {
    required_text_at(object, key, UsgsM2mEndpointClass::DownloadOptions)
}

fn required_text_at<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    endpoint: UsgsM2mEndpointClass,
) -> Result<&'a str, UsgsM2mError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_RUNTIME_URL_BYTES
                && *value == value.trim()
                && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| UsgsM2mError::new(UsgsM2mErrorCode::ResponseInvalid, endpoint))
}

fn required_bool(object: &Map<String, Value>, key: &str) -> Result<bool, UsgsM2mError> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| option_error(UsgsM2mErrorCode::ResponseInvalid))
}

fn bounded_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    endpoint: UsgsM2mEndpointClass,
) -> Result<&'a [Value], UsgsM2mError> {
    object
        .get(key)
        .and_then(Value::as_array)
        .filter(|values| values.len() <= MAX_DOWNLOAD_RESULTS)
        .map(Vec::as_slice)
        .ok_or_else(|| UsgsM2mError::new(UsgsM2mErrorCode::ResponseInvalid, endpoint))
}

fn require_empty_array(
    object: &Map<String, Value>,
    key: &str,
    endpoint: UsgsM2mEndpointClass,
) -> Result<(), UsgsM2mError> {
    if bounded_array(object, key, endpoint)?.is_empty() {
        Ok(())
    } else {
        Err(UsgsM2mError::new(
            UsgsM2mErrorCode::DownloadUnavailable,
            endpoint,
        ))
    }
}

fn normalized_download_id(value: &Value) -> Option<String> {
    let normalized = match value {
        Value::Number(value) => value.as_u64()?.to_string(),
        Value::String(value) => value.clone(),
        _ => return None,
    };
    (!normalized.is_empty()
        && normalized.len() <= MAX_DOWNLOAD_ID_BYTES
        && normalized.bytes().all(|byte| byte.is_ascii_digit()))
    .then_some(normalized)
}

fn parse_checksum_array(
    value: Option<&Value>,
) -> Result<(Option<String>, Option<String>), UsgsM2mError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok((None, None));
    };
    let values = value
        .as_array()
        .filter(|values| values.len() <= 16)
        .ok_or_else(|| option_error(UsgsM2mErrorCode::DownloadOptionInvalid))?;
    let mut accepted = Vec::new();
    for checksum in values {
        let checksum = checksum
            .as_object()
            .ok_or_else(|| option_error(UsgsM2mErrorCode::DownloadOptionInvalid))?;
        let Some(raw_value) = checksum.get("value").and_then(Value::as_str) else {
            continue;
        };
        let algorithm = checksum
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| option_error(UsgsM2mErrorCode::DownloadOptionInvalid))?;
        let expected_length = match algorithm.as_str() {
            "md5" => 32,
            "sha1" => 40,
            "sha256" => 64,
            "sha512" => 128,
            _ => return Err(option_error(UsgsM2mErrorCode::DownloadOptionInvalid)),
        };
        let normalized = raw_value.to_ascii_lowercase();
        if normalized.len() != expected_length
            || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(option_error(UsgsM2mErrorCode::DownloadOptionInvalid));
        }
        accepted.push((algorithm, normalized));
    }
    accepted.sort_by_key(|(algorithm, _)| match algorithm.as_str() {
        "sha512" => 0,
        "sha256" => 1,
        "sha1" => 2,
        _ => 3,
    });
    accepted.dedup();
    if accepted
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 != pair[1].1)
    {
        return Err(option_error(UsgsM2mErrorCode::DownloadOptionAmbiguous));
    }
    match accepted.first() {
        None => Ok((None, None)),
        Some((algorithm, value)) => Ok((Some(algorithm.clone()), Some(value.clone()))),
    }
}

fn option_error(code: UsgsM2mErrorCode) -> UsgsM2mError {
    UsgsM2mError::new(code, UsgsM2mEndpointClass::DownloadOptions)
}

/// Opt-in resolve-stage diagnosis under the `INSAR_DIAGNOSTIC_USGS_M2M`
/// pattern. Reports the endpoint, the sanitized response shape (key names,
/// array lengths, upstream errorCode values, URL host/path shapes) and the
/// number of available/preparing/ready downloads. Labels, identifiers, the
/// apiKey and full signed URLs are never logged.
fn write_resolve_stage_diagnostic(stage: &str, data: &Value) {
    if !crate::diagnostics::usgs_m2m_login_diagnostic_enabled() {
        return;
    }
    // download-options answers a plain array of option records; the request
    // and retrieve stages answer objects. Describe both shapes without ever
    // recording a record's identifiers or URL text.
    let shape = match data {
        Value::Array(items) => format!("array[{}]", items.len().min(MAX_DOWNLOAD_RESULTS)),
        other => response_shape(other),
    };
    let event = serde_json::json!({
        "event": "usgs_m2m_resolve_stage",
        "stage": stage,
        "response_shape": shape,
    });
    eprintln!("INSAR_USGS_M2M_RESOLVE_DIAGNOSTIC {event}");
}

fn request_error(code: UsgsM2mErrorCode) -> UsgsM2mError {
    UsgsM2mError::new(code, UsgsM2mEndpointClass::DownloadRequest)
}

fn retrieve_error(code: UsgsM2mErrorCode) -> UsgsM2mError {
    UsgsM2mError::new(code, UsgsM2mEndpointClass::DownloadRetrieve)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use insar_core::ProviderNetworkPolicy;

    use super::*;
    use crate::usgs_m2m::endpoint::{SensitiveJson, UsgsM2mSessionEndpoint};
    use crate::usgs_m2m::session::{UsgsM2mApiKey, UsgsM2mApplicationToken, UsgsM2mLoginMaterial};

    struct FixtureEndpoint {
        option_payload: Mutex<Option<Value>>,
        request_payload: Mutex<Option<Value>>,
    }

    impl UsgsM2mSessionEndpoint for FixtureEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            ProviderNetworkPolicy::direct()
        }

        fn login_token(
            &self,
            _material: &UsgsM2mLoginMaterial,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<UsgsM2mApiKey, UsgsM2mError> {
            UsgsM2mApiKey::from_wire("fixture-runtime-key".to_owned())
        }

        fn logout(
            &self,
            _api_key: &UsgsM2mApiKey,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<(), UsgsM2mError> {
            Ok(())
        }

        fn dataset_search(
            &self,
            _api_key: &UsgsM2mApiKey,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<SensitiveJson, UsgsM2mError> {
            Err(option_error(UsgsM2mErrorCode::ConfigInvalid))
        }

        fn download_options(
            &self,
            _api_key: &UsgsM2mApiKey,
            payload: &Value,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<SensitiveJson, UsgsM2mError> {
            *self.option_payload.lock().unwrap() = Some(payload.clone());
            Ok(SensitiveJson::new(json!([{
                "id": LANDSAT_C2_L2_BUNDLE_OPTION_ID,
                "downloadName": null,
                "displayId": "LC09_L2SP_123045_20260101_20260103_02_T1",
                "entityId": "LC91230452026001LGN00",
                "datasetId": "5e83d14f2fc39685",
                "available": true,
                "filesize": 958605073,
                "productName": LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
                "productCode": LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE,
                "bulkAvailable": true,
                "downloadSystem": LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM,
                "secondaryDownloads": [],
                "fileGroups": null,
                "checksum": null
            }])))
        }

        fn download_request(
            &self,
            _api_key: &UsgsM2mApiKey,
            payload: &Value,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<SensitiveJson, UsgsM2mError> {
            *self.request_payload.lock().unwrap() = Some(payload.clone());
            Ok(SensitiveJson::new(json!({
                "availableDownloads": [{
                    "downloadId": 42,
                    "entityId": "LC91230452026001LGN00",
                    "displayId": "LC09_L2SP_123045_20260101_20260103_02_T1",
                    "url": "https://landsatlook.usgs.gov/data/bundle.tar"
                }],
                "preparingDownloads": [],
                "failed": []
            })))
        }
    }

    #[test]
    fn authenticated_client_sends_one_exact_option_request_without_login_material() {
        let endpoint = Arc::new(FixtureEndpoint {
            option_payload: Mutex::new(None),
            request_payload: Mutex::new(None),
        });
        let session = Arc::new(
            UsgsM2mSession::from_endpoint(ProviderNetworkPolicy::direct(), endpoint.clone())
                .unwrap(),
        );
        session
            .login(
                UsgsM2mLoginMaterial::new(
                    "fixture-user",
                    UsgsM2mApplicationToken::new("fixture-application-token").unwrap(),
                )
                .unwrap(),
                &|| false,
            )
            .unwrap();
        let option = UsgsM2mDownloadClient::new(session.clone())
            .full_bundle_option(
                "landsat_ot_c2_l2",
                "LC91230452026001LGN00",
                "LC09_L2SP_123045_20260101_20260103_02_T1",
                &|| false,
            )
            .unwrap();
        assert_eq!(option.option_id, LANDSAT_C2_L2_BUNDLE_OPTION_ID);
        assert_eq!(
            endpoint.option_payload.lock().unwrap().as_ref().unwrap(),
            &json!({
                "datasetName": "landsat_ot_c2_l2",
                "entityIds": ["LC91230452026001LGN00"]
            })
        );
        let serialized = endpoint
            .option_payload
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .to_string();
        assert!(!serialized.contains("fixture-user"));
        assert!(!serialized.contains("fixture-application-token"));
        assert_eq!(session.request_counts().download_options, 1);
    }

    #[test]
    fn download_request_asks_usgs_to_return_immediately_available_urls() {
        let endpoint = Arc::new(FixtureEndpoint {
            option_payload: Mutex::new(None),
            request_payload: Mutex::new(None),
        });
        let session = Arc::new(
            UsgsM2mSession::from_endpoint(ProviderNetworkPolicy::direct(), endpoint.clone())
                .unwrap(),
        );
        session
            .login(
                UsgsM2mLoginMaterial::new(
                    "fixture-user",
                    UsgsM2mApplicationToken::new("fixture-application-token").unwrap(),
                )
                .unwrap(),
                &|| false,
            )
            .unwrap();
        let result = UsgsM2mDownloadClient::new(session.clone())
            .request_full_bundle(
                "landsat_ot_c2_l2",
                "LC91230452026001LGN00",
                "LC09_L2SP_123045_20260101_20260103_02_T1",
                LANDSAT_C2_L2_BUNDLE_OPTION_ID,
                "sceneport-test-label",
                &|| false,
            )
            .unwrap();
        assert!(matches!(
            result,
            UsgsM2mDownloadRequestOutcome::Available(_)
        ));
        assert_eq!(
            endpoint.request_payload.lock().unwrap().as_ref().unwrap(),
            &json!({
                "downloads": [{
                    "entityId": "LC91230452026001LGN00",
                    "productId": LANDSAT_C2_L2_BUNDLE_OPTION_ID
                }],
                "label": "sceneport-test-label",
                "returnAvailable": true
            })
        );
        assert_eq!(session.request_counts().download_request, 1);
    }

    #[test]
    fn request_and_retrieve_parsers_bind_one_exact_download_identity() {
        let immediate = parse_download_request(
            &json!({
                "availableDownloads": [{
                    "downloadId": 42,
                    "entityId": "LC91230452026001LGN00",
                    "displayId": "LC09_L2SP_123045_20260101_20260103_02_T1",
                    "url": "https://landsatlook.usgs.gov/data/bundle.tar",
                    "filesize": 958605073,
                    "checksum_values": [
                        {"id": "sha256", "value": "b".repeat(64)},
                        {"id": "sha512", "value": "a".repeat(128)}
                    ]
                }],
                "preparingDownloads": [],
                "failed": []
            }),
            "LC91230452026001LGN00",
        )
        .expect("one immediate result");
        let UsgsM2mDownloadRequestOutcome::Available(immediate) = immediate else {
            panic!("expected an immediate result");
        };
        assert_eq!(immediate.checksum().map(|value| value.0), Some("sha512"));

        let invalid_display = parse_download_request(
            &json!({
                "availableDownloads": [{
                    "downloadId": 42,
                    "entityId": "LC91230452026001LGN00",
                    "displayId": "../runtime-identity",
                    "url": "https://landsatlook.usgs.gov/data/bundle.tar"
                }],
                "preparingDownloads": [],
                "failed": []
            }),
            "LC91230452026001LGN00",
        )
        .unwrap_err();
        assert_eq!(invalid_display.code(), UsgsM2mErrorCode::ResponseInvalid);

        let preparing = parse_preparing_download(
            &json!({
                "downloadId": "42",
                "entityId": "LC91230452026001LGN00"
            }),
            "LC91230452026001LGN00",
        )
        .expect("one preparing identity");
        let retrieved = parse_download_retrieve(
            &json!({
                "available": [{
                    "downloadId": 42,
                    "entityId": "LC91230452026001LGN00",
                    "url": "https://landsatlook.usgs.gov/data/bundle.tar"
                }],
                "requested": []
            }),
            &preparing,
        )
        .expect("exact retrieve response")
        .expect("resolved retrieve result");
        assert_eq!(retrieved.download_id(), "42");

        assert_eq!(
            parse_download_retrieve(
                &json!({
                    "available": [{
                        "downloadId": 43,
                        "entityId": "LC91230452026001LGN00",
                        "url": "https://landsatlook.usgs.gov/data/foreign.tar"
                    }],
                    "requested": []
                }),
                &preparing,
            )
            .unwrap_err()
            .code(),
            UsgsM2mErrorCode::DownloadIdentityConflict
        );
    }

    /// A request or retrieve answer that fails to parse keeps only its
    /// sanitized shape in the persisted diagnostic.
    /// Recorded production sequence for a not-yet-prepared bundle (D806 /
    /// dds_ms): download-request answers `preparingDownloads` with a numeric
    /// downloadId, download-retrieve is polled with the label, and the same
    /// downloadId reappears under `available` with the runtime URL once the
    /// bundle is ready.
    #[test]
    fn preparing_then_polled_retrieve_resolves_the_same_download_identity() {
        let preparing = parse_download_request(
            &json!({
                "availableDownloads": [],
                "preparingDownloads": [{
                    "downloadId": 4711,
                    "entityId": "LC91230452026001LGN00"
                }],
                "failed": []
            }),
            "LC91230452026001LGN00",
        )
        .expect("one preparing result");
        let UsgsM2mDownloadRequestOutcome::Preparing(preparing) = preparing else {
            panic!("expected a preparing result");
        };
        assert_eq!(preparing.download_id(), "4711");
        assert_eq!(preparing.entity_id(), "LC91230452026001LGN00");

        // First poll: still preparing — both arrays are empty for this label.
        assert!(
            parse_download_retrieve(&json!({"available": [], "requested": []}), &preparing,)
                .expect("valid not-ready answer")
                .is_none()
        );

        // Second poll: the same downloadId is ready under `available`.
        let ready = parse_download_retrieve(
            &json!({
                "available": [{
                    "downloadId": 4711,
                    "entityId": "LC91230452026001LGN00",
                    "displayId": "LC09_L2SP_123045_20260101_20260103_02_T1",
                    "url": "https://landsatlook.usgs.gov/broker?download=opaque-runtime-value",
                    "filesize": 958605073
                }],
                "requested": []
            }),
            &preparing,
        )
        .expect("valid ready answer")
        .expect("resolved download");
        assert_eq!(ready.download_id(), "4711");
        assert_eq!(ready.entity_id(), "LC91230452026001LGN00");
        assert_eq!(
            ready.display_id(),
            Some("LC09_L2SP_123045_20260101_20260103_02_T1")
        );
        assert_eq!(ready.expected_size_bytes(), Some(958605073));
        assert!(ready
            .runtime_url()
            .starts_with("https://landsatlook.usgs.gov/"));
    }

    #[test]
    fn failed_request_and_retrieve_record_only_a_sanitized_response_shape() {
        let secrets = [
            "LC9SECRETENTITY",
            "token=abc123",
            "secretpath",
            "sig=XYZ",
            "bundle-name",
            "user-label",
        ];
        let failed = parse_download_request(
            &json!({
                "availableDownloads": [],
                "preparingDownloads": [],
                "failed": [{
                    "entityId": "LC9SECRETENTITY",
                    "errorCode": "DOWNLOAD_ERROR",
                    "errorMessage": "token=abc123",
                    "url": "https://dds.cr.usgs.gov/secretpath/bundle-name.tar?sig=XYZ"
                }],
                "label": "user-label"
            }),
            "LC91230452026001LGN00",
        )
        .unwrap_err()
        .diagnostic_message();
        assert!(
            failed.contains(
                "response_shape=keys=availableDownloads,failed,label,preparingDownloads;\
                 availableDownloads[0];failed[1];failed.errorCode=DOWNLOAD_ERROR;\
                 failed.url=https://dds.cr.usgs.gov/*/*.tar?query;preparingDownloads[0]"
            ),
            "{failed}"
        );
        let preparing = parse_preparing_download(
            &json!({"downloadId": "42", "entityId": "LC91230452026001LGN00"}),
            "LC91230452026001LGN00",
        )
        .unwrap();
        let foreign = parse_download_retrieve(
            &json!({
                "available": [{
                    "downloadId": 43,
                    "entityId": "LC9SECRETENTITY",
                    "url": "https://landsatlook.usgs.gov/secretpath/bundle-name?sig=XYZ"
                }],
                "requested": []
            }),
            &preparing,
        )
        .unwrap_err()
        .diagnostic_message();
        assert!(
            foreign.contains(
                "response_shape=keys=available,requested;available[1];\
                 available.url=https://landsatlook.usgs.gov/*/*?query;requested[0]"
            ),
            "{foreign}"
        );
        for diagnostic in [&failed, &foreign] {
            for secret in secrets {
                assert!(!diagnostic.contains(secret), "{secret} in {diagnostic}");
            }
        }
    }
}
