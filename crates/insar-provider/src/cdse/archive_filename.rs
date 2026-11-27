use insar_core::{ContractError, ProviderResult};
use serde_json::{Map, Value};

use crate::catalog::CDSE_SENTINEL2_L2A_ARCHIVE_FILENAME_RULE_ID;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveFilenameEvidence {
    pub rule_id: &'static str,
    pub odata_name: String,
    pub basename: String,
}

fn error(code: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        "Sentinel-2 归档文件名证据无效",
        Map::from_iter([("reason".to_string(), Value::String(reason.to_string()))]),
    )
    .expect("bounded filename evidence context is safe")
}

fn safe_basename(value: &str) -> bool {
    let device_name = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved_device = matches!(
        device_name.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    !value.is_empty()
        && value == value.trim()
        && value.encode_utf16().count() <= 255
        && value != "."
        && value != ".."
        && !value.ends_with(['.', ' '])
        && !value.starts_with("\\\\")
        && !value.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*'])
        && !value.chars().any(|character| character.is_control())
        && !reserved_device
}

/// Validate the fixed CDSE Sentinel-2 and Sentinel-3 OData product-directory name.
///
/// The OData `Name` is an authoritative `.SAFE` or `.SEN3` directory basename;
/// it is not already an archive filename and may not carry a duplicated suffix.
pub fn safe_cdse_odata_name(value: &str) -> bool {
    let stem = if let Some(stem) = value.strip_suffix(".SAFE") {
        stem
    } else if let Some(stem) = value.strip_suffix(".SEN3") {
        stem
    } else {
        return false;
    };
    safe_basename(value) && !stem.is_empty() && !stem.ends_with(".SAFE") && !stem.ends_with(".SEN3")
}

/// Validate the only archive basename derived by the Sentinel-2 profile.
pub fn safe_cdse_archive_basename(value: &str) -> bool {
    value.strip_suffix(".zip").is_some_and(safe_cdse_odata_name)
}

/// Derive the sole archive basename from the validated OData product name.
/// Unlike `evaluate_archive_filename`, this does not inspect a response header;
/// it is the immutable catalogue naming rule used before a transfer begins.
pub fn archive_filename_from_odata_name(
    odata_name: &str,
) -> ProviderResult<ArchiveFilenameEvidence> {
    archive_filename_from_odata_name_for_rule(
        odata_name,
        CDSE_SENTINEL2_L2A_ARCHIVE_FILENAME_RULE_ID,
    )
}

/// Derive the archive basename for a specific reviewed CDSE filename rule
/// (`.SAFE.zip` for Sentinel-2 L2A/L1C or `.SEN3.zip` for Sentinel-3).
pub fn archive_filename_from_odata_name_for_rule(
    odata_name: &str,
    rule_id: &'static str,
) -> ProviderResult<ArchiveFilenameEvidence> {
    if !safe_cdse_odata_name(odata_name) {
        return Err(error("CDSE_ARCHIVE_FILENAME_UNSAFE", "unsafe_odata_name"));
    }
    Ok(ArchiveFilenameEvidence {
        rule_id,
        odata_name: odata_name.to_string(),
        basename: format!("{odata_name}.zip"),
    })
}

fn quoted_filename(header: &str) -> ProviderResult<String> {
    if header != header.trim() || header.chars().any(char::is_control) {
        return Err(error("CDSE_ARCHIVE_FILENAME_UNSAFE", "header_whitespace"));
    }
    let mut parts = header.split(';');
    if !parts
        .next()
        .is_some_and(|value| value.eq_ignore_ascii_case("attachment"))
    {
        return Err(error("CDSE_ARCHIVE_FILENAME_UNSAFE", "disposition"));
    }
    let mut filename = None;
    for parameter in parts {
        let parameter = parameter.trim();
        let (name, value) = parameter
            .split_once('=')
            .ok_or_else(|| error("CDSE_ARCHIVE_FILENAME_UNSAFE", "parameter"))?;
        if name.eq_ignore_ascii_case("filename*") {
            return Err(error("CDSE_ARCHIVE_FILENAME_UNSAFE", "filename_star"));
        }
        if !name.eq_ignore_ascii_case("filename") {
            continue;
        }
        if filename.is_some() {
            return Err(error("CDSE_ARCHIVE_FILENAME_UNSAFE", "duplicate_filename"));
        }
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .filter(|value| !value.is_empty() && !value.contains('"'))
            .ok_or_else(|| error("CDSE_ARCHIVE_FILENAME_UNSAFE", "quoted_filename"))?;
        filename = Some(value.to_string());
    }
    filename.ok_or_else(|| error("CDSE_ARCHIVE_FILENAME_MISSING", "missing_filename"))
}

/// Evaluate only fixed `Content-Disposition` evidence. This function makes no
/// request and never infers a filename from OData `Name` alone.
pub fn evaluate_archive_filename(
    odata_name: &str,
    content_disposition: Option<&str>,
) -> ProviderResult<ArchiveFilenameEvidence> {
    if !safe_cdse_odata_name(odata_name) {
        return Err(error("CDSE_ARCHIVE_FILENAME_UNSAFE", "unsafe_odata_name"));
    }
    let header = content_disposition
        .ok_or_else(|| error("CDSE_ARCHIVE_FILENAME_MISSING", "missing_header"))?;
    let basename = quoted_filename(header)?;
    if !safe_cdse_archive_basename(&basename) {
        return Err(error(
            "CDSE_ARCHIVE_FILENAME_UNSAFE",
            "unsafe_header_filename",
        ));
    }
    let expected = format!("{odata_name}.zip");
    if basename != expected {
        return Err(error(
            "CDSE_ARCHIVE_FILENAME_CONFLICT",
            "profile_rule_mismatch",
        ));
    }
    Ok(ArchiveFilenameEvidence {
        rule_id: CDSE_SENTINEL2_L2A_ARCHIVE_FILENAME_RULE_ID,
        odata_name: odata_name.to_string(),
        basename,
    })
}
