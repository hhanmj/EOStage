//! Local-first dedup (downloader D3). Inside the destination lease, and
//! before any credential, Provider resolution or network admission, a
//! planned destination that already exists is decided by the strength of its
//! evidence:
//!
//! - service checksum: the Provider's frozen checksum matches a local digest;
//! - ledger identity: the Store's landing ledger saw this exact file (volume,
//!   file id, size, write and change time) verified, so it is not read again;
//! - ledger re-hash: the ledger knows the file's SHA-256 but its identity
//!   changed (copied, touched); a local SHA-256 equal to it proves the content;
//! - size and format (no evidence): the authoritative size matches and a
//!   bounded format check reads the file's structures; the task says
//!   "复用已有文件（未做内容校验）" and offers "重新下载".
//!
//! Anything else is a conflict: the file is renamed aside, never deleted or
//! overwritten, and the planned file is downloaded.

mod gate;
mod probe;
mod quarantine;
#[cfg(test)]
mod tests;
mod verify;

use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

pub(crate) use gate::{
    authoritative_reuse_size, pass_local_reuse_gate, record_deferred_reuse, record_local_landing,
    scan_local_reuse, GateRequest, ScanReuse,
};
pub(crate) use probe::{ProbeFormat, ProbeOutcome};
pub(crate) use quarantine::{conflict_stamp, quarantine_conflict, CONFLICT_MARKER};

use crate::checksum::ChecksumAlgorithm;
use crate::file_identity::{file_status, FileStatus, FileStatusError, IdentityStrength};
use crate::ExpectedChecksum;
use verify::{digest_file, DigestError};

/// The evidence a reuse rests on, strongest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReuseGrade {
    ServiceChecksum,
    LedgerIdentity,
    LedgerRehash,
    SizeAndFormat,
}

impl ReuseGrade {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ServiceChecksum => "service_checksum",
            Self::LedgerIdentity => "ledger_identity",
            Self::LedgerRehash => "ledger_rehash",
            Self::SizeAndFormat => "size_and_format",
        }
    }
}

/// Why an existing file was not reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConflictReason {
    SizeMismatch,
    UnknownSize,
    FormatUnreadable,
    FormatUnsupported,
    ChecksumMismatch,
    LedgerMismatch,
}

impl ConflictReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SizeMismatch => "size_mismatch",
            Self::UnknownSize => "unknown_size",
            Self::FormatUnreadable => "format_unreadable",
            Self::FormatUnsupported => "format_unsupported",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::LedgerMismatch => "ledger_mismatch",
        }
    }
}

/// The landing ledger's current row for the destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LedgerEvidence {
    pub(crate) platform: String,
    pub(crate) volume_id: u64,
    pub(crate) file_id: u64,
    pub(crate) size: u64,
    pub(crate) modified: Option<i128>,
    pub(crate) changed: Option<i128>,
    pub(crate) stable: bool,
    pub(crate) sha256: Option<String>,
    /// The service checksum verified when the row was written, if any.
    pub(crate) service_checksum: Option<(ChecksumAlgorithm, String)>,
}

impl LedgerEvidence {
    fn identity_matches(&self, status: &FileStatus) -> bool {
        self.stable
            && status.strength == IdentityStrength::Stable
            && status.changed.is_some()
            && self.platform == status.platform
            && self.volume_id == status.volume_id
            && self.file_id == status.file_id
            && self.size == status.size
            && self.modified == status.modified
            && self.changed == status.changed
    }
}

/// One decision's inputs.
pub(crate) struct LocalReuseRequest<'a> {
    pub(crate) destination: &'a Path,
    /// The size the Provider froze or reported; None when unknown.
    pub(crate) authoritative_size: Option<u64>,
    pub(crate) checksum: Option<&'a ExpectedChecksum>,
    pub(crate) format: ProbeFormat,
    /// The planned product name without extension (SAFE manifest check).
    pub(crate) product: Option<&'a str>,
    pub(crate) ledger: Option<&'a LedgerEvidence>,
    /// False for the pre-scan's cheap phase (downloader D3-2): a decision
    /// that needs a digest answers NeedsDigest instead of reading the file.
    pub(crate) allow_digest: bool,
    pub(crate) stop: &'a dyn Fn() -> bool,
}

/// A reuse and the facts it rests on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalReuse {
    pub(crate) grade: ReuseGrade,
    pub(crate) status: FileStatus,
    /// The SHA-256 known after the decision: computed, or the ledger's.
    pub(crate) sha256: Option<String>,
}

/// The decision for one planned destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LocalReuseDecision {
    /// Nothing exists there: download as usual.
    Missing,
    Reuse(LocalReuse),
    /// The file must be renamed aside before downloading.
    Conflict(ConflictReason),
    /// A link, reparse point, directory or multiply-linked file; left untouched.
    Unsafe,
    /// A pause or cancellation ended a local read.
    Stopped,
    /// The file could not be read.
    ReadFailed,
    /// The format check would exceed its budget.
    OverBudget,
    /// Deciding needs a digest, which this request does not allow.
    NeedsDigest,
}

/// Decides the planned destination of `request`. The decision reads only
/// what its evidence needs: nothing for a ledger identity match, a digest for
/// a checksum or re-hash, bounded structures for a format check.
pub(crate) fn decide_local_reuse(request: &LocalReuseRequest<'_>) -> LocalReuseDecision {
    let metadata = match std::fs::symlink_metadata(request.destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return LocalReuseDecision::Missing,
        Err(_) => return LocalReuseDecision::ReadFailed,
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return LocalReuseDecision::Unsafe;
    }
    let Ok(file) = File::open(crate::long_path::native_path(request.destination)) else {
        return LocalReuseDecision::ReadFailed;
    };
    let status = match file_status(&file) {
        Ok(status) => status,
        Err(FileStatusError::NotRegular) => return LocalReuseDecision::Unsafe,
        Err(FileStatusError::Unavailable) => return LocalReuseDecision::ReadFailed,
    };
    if status.links != 1 {
        return LocalReuseDecision::Unsafe;
    }
    if request
        .authoritative_size
        .is_some_and(|size| size != status.size)
    {
        return LocalReuseDecision::Conflict(ConflictReason::SizeMismatch);
    }
    if let Some(ledger) = request.ledger {
        let checksum_known = request.checksum.is_none_or(|checksum| {
            ledger
                .service_checksum
                .as_ref()
                .is_some_and(|(algorithm, value)| {
                    *algorithm == checksum.algorithm() && value == checksum.value()
                })
        });
        if checksum_known && ledger.identity_matches(&status) {
            return LocalReuseDecision::Reuse(LocalReuse {
                grade: ReuseGrade::LedgerIdentity,
                sha256: ledger.sha256.clone(),
                status,
            });
        }
    }
    if let Some(checksum) = request.checksum {
        if !request.allow_digest {
            return LocalReuseDecision::NeedsDigest;
        }
        let algorithms = [checksum.algorithm(), ChecksumAlgorithm::Sha256];
        return match digest_file(&file, &algorithms, request.stop) {
            Ok(digests) if digests[0] == checksum.value() => {
                LocalReuseDecision::Reuse(LocalReuse {
                    grade: ReuseGrade::ServiceChecksum,
                    sha256: digests.into_iter().nth(1),
                    status,
                })
            }
            Ok(_) => LocalReuseDecision::Conflict(ConflictReason::ChecksumMismatch),
            Err(DigestError::Stopped) => LocalReuseDecision::Stopped,
            Err(DigestError::Io) => LocalReuseDecision::ReadFailed,
        };
    }
    if let Some((ledger, ledger_sha256)) = request
        .ledger
        .and_then(|ledger| ledger.sha256.as_ref().map(|sha256| (ledger, sha256)))
    {
        if ledger.size != status.size {
            return LocalReuseDecision::Conflict(ConflictReason::LedgerMismatch);
        }
        if !request.allow_digest {
            return LocalReuseDecision::NeedsDigest;
        }
        return match digest_file(&file, &[ChecksumAlgorithm::Sha256], request.stop) {
            Ok(digests) if &digests[0] == ledger_sha256 => LocalReuseDecision::Reuse(LocalReuse {
                grade: ReuseGrade::LedgerRehash,
                sha256: digests.into_iter().next(),
                status,
            }),
            Ok(_) => LocalReuseDecision::Conflict(ConflictReason::LedgerMismatch),
            Err(DigestError::Stopped) => LocalReuseDecision::Stopped,
            Err(DigestError::Io) => LocalReuseDecision::ReadFailed,
        };
    }
    if request.authoritative_size.is_none() {
        return LocalReuseDecision::Conflict(ConflictReason::UnknownSize);
    }
    if (request.stop)() {
        return LocalReuseDecision::Stopped;
    }
    match probe::probe_file(&file, status.size, request.format, request.product) {
        Ok(ProbeOutcome::Readable) => LocalReuseDecision::Reuse(LocalReuse {
            grade: ReuseGrade::SizeAndFormat,
            sha256: None,
            status,
        }),
        Ok(ProbeOutcome::Unreadable) => {
            LocalReuseDecision::Conflict(ConflictReason::FormatUnreadable)
        }
        Ok(ProbeOutcome::Unsupported) => {
            LocalReuseDecision::Conflict(ConflictReason::FormatUnsupported)
        }
        Ok(ProbeOutcome::OverBudget) => LocalReuseDecision::OverBudget,
        Err(_) => LocalReuseDecision::ReadFailed,
    }
}
