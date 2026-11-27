//! Small local evidence for an identity-bound HTTP part.
//! Never stores an access URL, headers or credentials. This is not task state.
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::file_core::{open_checked_download_read, open_checked_download_write};
use crate::pinned_directory::PinnedDirectory;
use crate::{part_path, DownloadFileError, RemoteValidator};

type Result<T> = std::result::Result<T, DownloadFileError>;
const MAX_RECORD: u64 = 16 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartCheckpoint {
    schema: u32,
    binding: String,
    directory: String,
    file_name: String,
    total: u64,
    pub(crate) bytes: u64,
    /// Primary binding validator. Schema 1 always stores a strong ETag here.
    /// Schema 2 may instead store an empty string when the remote serves no
    /// ETag, in which case `last_modified` plus `total` bind the prefix —
    /// exactly the validator pair `If-Range` itself accepts (RFC 9110 §13.1.5).
    etag: String,
    /// Canonical HTTP date (GMT, 29 chars). Only meaningful in schema 2 when
    /// `etag` is empty; then required.
    last_modified: String,
    pub(crate) prefix_sha256: String,
    digest: String,
}

/// The historical schema-1 record, byte-for-byte: no `last_modified` field.
/// Old interrupted downloads must keep loading and digest-validating with
/// their original serialized shape; schema 1 is migrated to schema 2 only on
/// the next successful save.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartCheckpointV1 {
    pub(crate) schema: u32,
    pub(crate) binding: String,
    pub(crate) directory: String,
    pub(crate) file_name: String,
    pub(crate) total: u64,
    pub(crate) bytes: u64,
    pub(crate) etag: String,
    pub(crate) prefix_sha256: String,
    pub(crate) digest: String,
}

#[derive(Deserialize)]
struct CheckpointSchemaProbe {
    schema: u32,
}

pub(crate) fn strong_etag(value: &str) -> bool {
    (2..=1024).contains(&value.len())
        && value.starts_with('"')
        && value.ends_with('"')
        && value[1..value.len() - 1]
            .bytes()
            .all(|b| b >= 0x21 && b != b'"' && b != 0x7f)
}

/// A canonical HTTP Last-Modified date, the exact shape `parse_canonical_http_date`
/// accepts and the only shape `If-Range` may carry.
pub(crate) fn canonical_http_date(value: &str) -> bool {
    value.len() == 29
        && value.is_ascii()
        && value.as_bytes()[3] == b','
        && value.as_bytes()[4] == b' '
        && value.as_bytes()[7] == b' '
        && value.as_bytes()[11] == b' '
        && value.as_bytes()[16] == b' '
        && value.as_bytes()[19] == b':'
        && value.as_bytes()[22] == b':'
        && &value[25..] == " GMT"
        && value.ends_with(" GMT")
}

fn invalid() -> DownloadFileError {
    DownloadFileError::RecoveryRequired
}

/// Opt-in diagnosis under the shared `INSAR_DIAGNOSTIC_USGS_M2M` gate, in the
/// same shape as the resolve diagnostic. Emitted to stderr only, with fixed
/// event/reason codes, validator kinds and byte counts — never a URL, path,
/// token, credential, raw header value or file name.
pub(crate) fn checkpoint_diagnostic(event: &str, detail: &str) {
    if std::env::var("INSAR_DIAGNOSTIC_USGS_M2M")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
    {
        eprintln!("INSAR_USGS_M2M_RESUME_DIAGNOSTIC {{\"event\":\"{event}\",{detail}}}");
    }
}
fn is_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn names(destination: &Path) -> Result<(PinnedDirectory, String, String)> {
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let directory = PinnedDirectory::create(parent)?;
    let name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(invalid)?
        .to_owned();
    let record_name = format!("{name}.part.resume.json");
    directory.join_component(&record_name)?;
    Ok((directory, name, record_name))
}

impl PartCheckpoint {
    fn hash(&self) -> Result<String> {
        // The digest covers the schema-defined serialized shape: a schema-2
        // record includes last_modified, a schema-1 record never does.
        let value = match self.schema {
            1 => {
                let v1 = PartCheckpointV1 {
                    schema: self.schema,
                    binding: self.binding.clone(),
                    directory: self.directory.clone(),
                    file_name: self.file_name.clone(),
                    total: self.total,
                    bytes: self.bytes,
                    etag: self.etag.clone(),
                    prefix_sha256: self.prefix_sha256.clone(),
                    digest: String::new(),
                };
                serde_json::to_vec(&v1)
            }
            _ => {
                let mut unsigned = self.clone();
                unsigned.digest.clear();
                serde_json::to_vec(&unsigned)
            }
        }
        .map_err(|_| invalid())?;
        Ok(format!("{:x}", Sha256::digest(value)))
    }
    pub(crate) fn validator(&self) -> RemoteValidator {
        RemoteValidator {
            etag: (!self.etag.is_empty()).then(|| self.etag.clone()),
            content_length: Some(self.total),
            last_modified: (!self.last_modified.is_empty()).then(|| self.last_modified.clone()),
        }
    }
    pub(crate) fn matches_response_range(&self, header: Option<&str>, length: Option<u64>) -> bool {
        let parsed = header.and_then(|header| {
            let (interval, total) = header.strip_prefix("bytes ")?.split_once('/')?;
            let (start, end) = interval.split_once('-')?;
            Some((
                start.parse::<u64>().ok()?,
                end.parse::<u64>().ok()?,
                total.parse::<u64>().ok()?,
            ))
        });
        parsed == Some((self.bytes, self.total - 1, self.total))
            && length.is_none_or(|length| length == self.total - self.bytes)
    }

    fn read(directory: &PinnedDirectory, record: &str) -> Result<Option<Self>> {
        let path = directory.join_component(record)?;
        let file = match open_checked_download_read(&path) {
            Ok(file) => file,
            Err(DownloadFileError::Filesystem(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None)
            }
            Err(e) => return Err(e),
        };
        if file.metadata()?.len() > MAX_RECORD {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        file.take(MAX_RECORD + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_RECORD {
            return Err(invalid());
        }
        // Route by the record's own schema so a historical schema-1 record
        // digests over its original field set, not the newer shape.
        let schema = serde_json::from_slice::<CheckpointSchemaProbe>(&bytes)
            .map_err(|_| invalid())?
            .schema;
        let value: Self = match schema {
            1 => {
                let v1: PartCheckpointV1 = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                let digest = v1.digest.clone();
                let mut unsigned = v1.clone();
                unsigned.digest.clear();
                let expected = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&unsigned).map_err(|_| invalid(),)?)
                );
                if digest != expected || !strong_etag(&v1.etag) {
                    return Err(invalid());
                }
                Self {
                    schema: v1.schema,
                    binding: v1.binding,
                    directory: v1.directory,
                    file_name: v1.file_name,
                    total: v1.total,
                    bytes: v1.bytes,
                    etag: v1.etag,
                    last_modified: String::new(),
                    prefix_sha256: v1.prefix_sha256,
                    digest: v1.digest,
                }
            }
            2 => {
                let value: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                let validator_shape_valid = strong_etag(&value.etag)
                    || (value.etag.is_empty() && canonical_http_date(&value.last_modified));
                if !validator_shape_valid {
                    return Err(invalid());
                }
                value
            }
            _ => return Err(invalid()),
        };
        if !is_sha(&value.binding)
            || !is_sha(&value.prefix_sha256)
            || value.bytes == 0
            || value.bytes >= value.total
            || value.digest != value.hash()?
        {
            return Err(invalid());
        }
        directory.verify()?;
        Ok(Some(value))
    }
    fn bound(&self, directory: &PinnedDirectory, name: &str, binding: &str, total: u64) -> bool {
        self.binding == binding
            && self.directory == directory.identity_marker()
            && self.file_name == name
            && self.total == total
    }
    /// Identity-only bound check for remote-authoritative size routes, where
    /// the saved total is validated against the real response instead of a
    /// frozen advisory size.
    fn bound_identity(&self, directory: &PinnedDirectory, name: &str, binding: &str) -> bool {
        self.binding == binding
            && self.directory == directory.identity_marker()
            && self.file_name == name
    }
    pub(crate) fn load(
        destination: &Path,
        binding: &str,
        expected: Option<u64>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<Self>> {
        let (directory, name, record) = names(destination)?;
        let Some(value) = Self::read(&directory, &record)? else {
            checkpoint_diagnostic("checkpoint_absent", "\"reason\":\"no_record\"");
            return Ok(None);
        };
        // A frozen expected size is compared when the route treats it as
        // authoritative. `None` means the route's size authority is the real
        // response (USGS D806 advisory filesize): the saved total is then
        // validated against the response's exact 206 Content-Range instead,
        // never against the advisory value.
        if let Some(expected) = expected {
            if !value.bound(&directory, &name, binding, expected) {
                // Component names and a truncated hash comparison only; no
                // identity values, paths or validators are printed.
                let mut components = Vec::new();
                if value.binding != binding {
                    components.push(format!(
                        "\"binding\":\"~{}/~{}\"",
                        &value.binding[..8.min(value.binding.len())],
                        &binding[..8.min(binding.len())]
                    ));
                }
                if value.directory != directory.identity_marker() {
                    components.push("\"directory\":true".to_owned());
                }
                if value.file_name != name {
                    components.push("\"file_name\":true".to_owned());
                }
                if value.total != expected {
                    components.push(format!(
                        "\"saved_total\":{},\"expected_total\":{}",
                        value.total, expected
                    ));
                }
                if components.is_empty() {
                    components.push("\"unknown\":true".to_owned());
                }
                checkpoint_diagnostic(
                    "checkpoint_rejected",
                    &format!(
                        "\"reason\":\"identity_or_size_changed\",{}",
                        components.join(",")
                    ),
                );
                return Err(invalid());
            }
        } else if !value.bound_identity(&directory, &name, binding) {
            checkpoint_diagnostic("checkpoint_rejected", "\"reason\":\"identity_changed\"");
            return Err(invalid());
        }
        let path = directory.join_component(&format!("{name}.part"))?;
        let mut file = open_checked_download_write(&path)?;
        let part_len = file.metadata()?.len();
        if part_len < value.bytes {
            checkpoint_diagnostic("checkpoint_rejected", "\"reason\":\"part_size_changed\"");
            return Err(invalid());
        }
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 64 * 1024];
        let mut count = 0u64;
        while count < value.bytes {
            if cancelled() {
                return Err(invalid());
            }
            let limit = (value.bytes - count).min(buffer.len() as u64) as usize;
            let n = file.read(&mut buffer[..limit])?;
            if n == 0 {
                break;
            }
            count += n as u64;
            hasher.update(&buffer[..n]);
        }
        if count != value.bytes || format!("{:x}", hasher.finalize()) != value.prefix_sha256 {
            checkpoint_diagnostic("checkpoint_rejected", "\"reason\":\"prefix_hash_mismatch\"");
            return Err(invalid());
        }
        directory.verify()?;
        if cancelled() || file.metadata()?.len() != part_len {
            return Err(invalid());
        }
        let truncated_tail_bytes = part_len - value.bytes;
        if truncated_tail_bytes > 0 {
            // The bound prefix is proven; the unverified tail is refetched
            // under If-Range while the destination lease excludes other writers.
            file.set_len(value.bytes)?;
            file.sync_data()?;
        }
        checkpoint_diagnostic(
            "checkpoint_loaded",
            &format!(
                "\"bytes\":{},\"truncated_tail_bytes\":{},\"validator\":\"{}\"",
                value.bytes,
                truncated_tail_bytes,
                if value.etag.is_empty() {
                    "last_modified"
                } else {
                    "etag"
                }
            ),
        );
        Ok(Some(value))
    }
    pub(crate) fn save(
        destination: &Path,
        binding: &str,
        validator: &RemoteValidator,
        bytes: u64,
        prefix_sha256: String,
    ) -> Result<()> {
        let etag = validator.etag.as_deref().filter(|v| strong_etag(v));
        let last_modified = validator
            .last_modified
            .as_deref()
            .filter(|v| canonical_http_date(v));
        // Exactly the validator pairs `If-Range` accepts (RFC 9110 §13.1.5):
        // a strong ETag, or Last-Modified together with the exact total.
        let (schema, etag_value, last_modified_value) = match (etag, last_modified) {
            (Some(etag), _) => (2u32, etag.to_owned(), String::new()),
            (None, Some(last_modified)) => (2u32, String::new(), last_modified.to_owned()),
            (None, None) => {
                checkpoint_diagnostic("checkpoint_save_skipped", "\"reason\":\"no_validator\"");
                return Ok(());
            }
        };
        let Some(total) = validator
            .content_length
            .filter(|total| bytes > 0 && bytes < *total)
        else {
            checkpoint_diagnostic(
                "checkpoint_save_skipped",
                "\"reason\":\"no_total_or_complete\"",
            );
            return Ok(());
        };
        if !is_sha(binding) || !is_sha(&prefix_sha256) {
            return Err(invalid());
        }
        let (directory, name, record) = names(destination)?;
        if let Some(old) = Self::read(&directory, &record)? {
            // The validator must be the same binding value. A schema-1 ETag
            // record is legitimately migrated to schema 2 when the response
            // keeps the same strong ETag; a changed ETag or date is not.
            let same_validator = old.etag == etag_value
                && (old.last_modified.is_empty()
                    || last_modified_value.is_empty()
                    || old.last_modified == last_modified_value);
            if !old.bound(&directory, &name, binding, total) || !same_validator {
                return Err(invalid());
            }
        }
        let file = open_checked_download_read(&part_path(&directory.join_component(&name)?))?;
        if file.metadata()?.len() != bytes {
            return Err(invalid());
        }
        let mut value = Self {
            schema,
            binding: binding.to_owned(),
            directory: directory.identity_marker(),
            file_name: name,
            total,
            bytes,
            etag: etag_value,
            last_modified: last_modified_value,
            prefix_sha256,
            digest: String::new(),
        };
        value.digest = value.hash()?;
        let bytes = serde_json::to_vec(&value).map_err(|_| invalid())?;
        static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let instant = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_nanos();
        let temp = directory.join_component(&format!(
            ".resume-{}-{instant}-{nonce}.tmp",
            std::process::id()
        ))?;
        // Do not remove a path when create_new failed: it was not created by us.
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = (|| {
            out.write_all(&bytes)?;
            out.sync_all()?;
            drop(out);
            directory.verify()?;
            fs::rename(&temp, directory.join_component(&record)?)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
            return result;
        }
        // Emitted only after the record is durably published.
        checkpoint_diagnostic(
            "checkpoint_saved",
            &format!(
                "\"bytes\":{},\"validator\":\"{}\"",
                value.bytes,
                if value.etag.is_empty() {
                    "last_modified"
                } else {
                    "etag"
                }
            ),
        );
        Ok(())
    }
    pub(crate) fn clear(destination: &Path, binding: &str) -> Result<()> {
        let (directory, name, record) = names(destination)?;
        if let Some(old) = Self::read(&directory, &record)? {
            if !old.bound(&directory, &name, binding, old.total) {
                return Err(invalid());
            }
            fs::remove_file(directory.join_component(&record)?)?;
        }
        Ok(())
    }
}
