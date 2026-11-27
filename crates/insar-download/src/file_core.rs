use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use insar_core::{
    plan_desktop_target, plan_sar_catalog_target as plan_core_sar_catalog_target, ContractError,
    DesktopProductFamily, DesktopProductLayout, DownloadLayout,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::disk_guard::{FreeSpaceProbe, VolumeSpace};
use crate::safe_promotion::{
    promote_verified_closed_artifact, PromotionError, PromotionOutcome, VerifiedClosedArtifact,
};
#[cfg(test)]
use crate::safe_promotion::{
    promote_verified_closed_with_failpoint_for_test, SafePromotionTestFailpoint,
};
use crate::{verify_file_checksum, ChecksumError, ExpectedChecksum};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilePreparation {
    SkipComplete { bytes: u64 },
    FinalizePart { bytes: u64 },
    Resume { offset: u64 },
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedPartContext {
    RecoveredAfterRestart,
    CurrentExecution,
}

#[derive(Debug, Error)]
pub enum DownloadFileError {
    #[error("下载目标路径无效")]
    InvalidTarget(#[source] ContractError),
    #[error("无法访问下载文件")]
    Filesystem(#[source] std::io::Error),
    #[error("下载文件大小校验失败")]
    IntegrityMismatch,
    #[error("下载文件准备状态无效")]
    InvalidPreparation,
    #[error("下载文件恢复状态需要人工处理")]
    RecoveryRequired,
    #[error("无法安全替换下载文件")]
    SafePromotionFailed,
}

impl DownloadFileError {
    pub fn code(&self) -> &str {
        match self {
            Self::InvalidTarget(error) => &error.code,
            Self::IntegrityMismatch => "DL002",
            Self::Filesystem(_)
            | Self::InvalidPreparation
            | Self::RecoveryRequired
            | Self::SafePromotionFailed => "DL005",
        }
    }

    pub fn reason_zh(&self) -> &'static str {
        match self {
            Self::InvalidTarget(_) => "下载目标文件名或目录布局无效。",
            Self::Filesystem(_) => "无法读取或写入下载目录，请检查目录权限和磁盘状态。",
            Self::IntegrityMismatch => "下载文件大小与预期不一致，未生成最终文件。",
            Self::InvalidPreparation => "下载文件状态已变化，请重新检查后重试。",
            Self::RecoveryRequired => "下载文件恢复状态需要人工处理，已保留现有恢复证据。",
            Self::SafePromotionFailed => "无法安全替换下载文件，已保留可恢复文件。",
        }
    }
}

impl From<std::io::Error> for DownloadFileError {
    fn from(error: std::io::Error) -> Self {
        Self::Filesystem(error)
    }
}

pub fn plan_download_target(
    output_root: &Path,
    layout: &DownloadLayout,
    file_name: &str,
) -> Result<PathBuf, DownloadFileError> {
    let (family, desktop_layout) = match layout {
        DownloadLayout::Direct => (DesktopProductFamily::Sentinel1, DesktopProductLayout::Flat),
        DownloadLayout::Sentinel1Slc => (
            DesktopProductFamily::Sentinel1,
            DesktopProductLayout::ProductSubdirectory,
        ),
        DownloadLayout::SentinelOrbitAuxPoeorb => (
            DesktopProductFamily::Orbit,
            DesktopProductLayout::ProductSubdirectory,
        ),
        DownloadLayout::SarDatasetProcessing => {
            return Err(DownloadFileError::InvalidTarget(
                ContractError::new(
                    "V3_INVALID_TARGET_PATH",
                    "generic SAR layout requires dataset metadata",
                    serde_json::Map::new(),
                )
                .expect("empty target-path context is safe"),
            ));
        }
    };
    plan_desktop_target(output_root, file_name, family, desktop_layout)
        .map_err(DownloadFileError::InvalidTarget)
}

pub fn plan_sar_catalog_target(
    output_root: &Path,
    dataset: &str,
    processing_level: &str,
    file_name: &str,
) -> Result<PathBuf, DownloadFileError> {
    plan_core_sar_catalog_target(output_root, dataset, processing_level, file_name)
        .map_err(DownloadFileError::InvalidTarget)
}

pub fn part_path(destination: &Path) -> PathBuf {
    let mut value = destination.as_os_str().to_os_string();
    value.push(".part");
    PathBuf::from(value)
}

/// Returns the verified completed byte count on disk for `destination`'s `.part` file,
/// reporting `total_segmented_written()` for pre-allocated segmented `.part` files.
pub fn part_completed_bytes(destination: &Path) -> u64 {
    let size = fs::metadata(part_path(destination))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if let Some(meta) = PartMetaCheckpoint::load(destination) {
        if meta.segmented.is_some() {
            if meta.is_incomplete_segmented(size) {
                return meta.total_segmented_written();
            }
            if meta.is_segmented_complete() && meta.file_len == size {
                return size;
            }
            return 0;
        }
    }
    size
}

pub fn inspect_destination(
    destination: &Path,
    expected_size: Option<u64>,
) -> Result<FilePreparation, DownloadFileError> {
    if let Some(metadata) = optional_file_metadata(destination)? {
        let size = metadata.len();
        // Downloader D3: only a known expected size can prove an existing
        // file complete; an unknown size used to skip any file, even an empty
        // one.
        if expected_size == Some(size) {
            return Ok(FilePreparation::SkipComplete { bytes: size });
        }
    }

    let part = part_path(destination);
    let Some(metadata) = optional_file_metadata(&part)? else {
        PartMetaCheckpoint::clear(destination);
        return Ok(FilePreparation::Restart);
    };
    let size = metadata.len();
    let part_meta = PartMetaCheckpoint::load(destination);
    if let Some(meta) = part_meta.as_ref() {
        if meta.segmented.is_some() {
            if meta.is_incomplete_segmented(size) {
                return Ok(FilePreparation::Resume {
                    offset: meta.total_segmented_written(),
                });
            }
            if !meta.is_segmented_complete() || meta.file_len != size {
                PartMetaCheckpoint::clear(destination);
                return Ok(FilePreparation::Restart);
            }
        }
    }
    match expected_size {
        Some(expected) if size == expected => Ok(FilePreparation::FinalizePart { bytes: size }),
        Some(expected) if size > expected => {
            fs::remove_file(part)?;
            PartMetaCheckpoint::clear(destination);
            Ok(FilePreparation::Restart)
        }
        _ if size > 0 => Ok(FilePreparation::Resume { offset: size }),
        _ => {
            PartMetaCheckpoint::clear(destination);
            Ok(FilePreparation::Restart)
        }
    }
}

pub fn inspect_resolved_destination(
    destination: &Path,
    expected_size: Option<u64>,
    checksum: Option<&ExpectedChecksum>,
    part_context: ResolvedPartContext,
) -> Result<FilePreparation, DownloadFileError> {
    if let Some(metadata) = optional_file_metadata(destination)? {
        let size = metadata.len();
        if expected_size.is_some_and(|expected| expected != size) {
            return Err(DownloadFileError::IntegrityMismatch);
        }
        verify_resolved_checksum(destination, checksum)?;
        return Ok(FilePreparation::SkipComplete { bytes: size });
    }

    let part = part_path(destination);
    let Some(metadata) = optional_file_metadata(&part)? else {
        PartMetaCheckpoint::clear(destination);
        return Ok(FilePreparation::Restart);
    };
    let size = metadata.len();
    if size == 0 {
        PartMetaCheckpoint::clear(destination);
        return Ok(FilePreparation::Restart);
    }
    if expected_size.is_some_and(|expected| size > expected) {
        return Err(DownloadFileError::IntegrityMismatch);
    }
    let part_meta = PartMetaCheckpoint::load(destination);
    if let Some(meta) = part_meta.as_ref() {
        if meta.segmented.is_some() {
            if meta.is_incomplete_segmented(size) {
                return Ok(match (part_context, checksum.is_some()) {
                    (ResolvedPartContext::CurrentExecution, _)
                    | (ResolvedPartContext::RecoveredAfterRestart, true) => {
                        FilePreparation::Resume {
                            offset: meta.total_segmented_written(),
                        }
                    }
                    (ResolvedPartContext::RecoveredAfterRestart, false) => FilePreparation::Restart,
                });
            }
            if !meta.is_segmented_complete() || meta.file_len != size {
                PartMetaCheckpoint::clear(destination);
                return Ok(FilePreparation::Restart);
            }
        }
    }
    if expected_size == Some(size) {
        if checksum.is_some() {
            verify_resolved_checksum(&part, checksum)?;
            return Ok(FilePreparation::FinalizePart { bytes: size });
        }
        return Ok(match part_context {
            ResolvedPartContext::CurrentExecution => FilePreparation::FinalizePart { bytes: size },
            ResolvedPartContext::RecoveredAfterRestart => FilePreparation::Restart,
        });
    }
    Ok(match (part_context, checksum.is_some()) {
        (ResolvedPartContext::CurrentExecution, _)
        | (ResolvedPartContext::RecoveredAfterRestart, true) => {
            FilePreparation::Resume { offset: size }
        }
        (ResolvedPartContext::RecoveredAfterRestart, false) => FilePreparation::Restart,
    })
}

fn verify_resolved_checksum(
    path: &Path,
    checksum: Option<&ExpectedChecksum>,
) -> Result<(), DownloadFileError> {
    let Some(checksum) = checksum else {
        return Ok(());
    };
    verify_file_checksum(path, checksum).map_err(|error| match error {
        ChecksumError::Io(error) => DownloadFileError::Filesystem(error),
        ChecksumError::UnsupportedAlgorithm
        | ChecksumError::InvalidValue
        | ChecksumError::Mismatch => DownloadFileError::IntegrityMismatch,
    })
}

pub fn open_part_for_transfer(
    destination: &Path,
    preparation: FilePreparation,
) -> Result<File, DownloadFileError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let part = part_path(destination);
    match preparation {
        FilePreparation::Resume { offset } => {
            let file = open_part_append_nofollow(&part)?;
            let metadata = guarded_file_metadata(&file)?;
            if metadata.len() != offset {
                return Err(DownloadFileError::InvalidPreparation);
            }
            Ok(file)
        }
        FilePreparation::Restart => {
            PartMetaCheckpoint::clear(destination);
            let mut file = open_part_restart_nofollow(&part, true)?;
            guarded_file_metadata(&file)?;
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            Ok(file)
        }
        FilePreparation::SkipComplete { .. } | FilePreparation::FinalizePart { .. } => {
            Err(DownloadFileError::InvalidPreparation)
        }
    }
}

pub fn finalize_part(
    destination: &Path,
    expected_size: Option<u64>,
) -> Result<u64, DownloadFileError> {
    finalize_part_with(destination, expected_size, None, |artifact, target| {
        promote_verified_closed_artifact(artifact, target, None)
    })
}

pub(crate) fn finalize_part_with_sha256(
    destination: &Path,
    expected_size: Option<u64>,
    source_sha256: &str,
) -> Result<u64, DownloadFileError> {
    finalize_part_with(
        destination,
        expected_size,
        Some(source_sha256),
        |artifact, target| promote_verified_closed_artifact(artifact, target, None),
    )
}

/// Runs finalization through the D4A1 qualification failpoint seam.
///
/// This is an integration-test-only adapter. Production always uses
/// finalize_part, which invokes the operating-system promotion primitive.
#[cfg(test)]
pub(crate) fn finalize_part_with_failpoint_for_test(
    destination: &Path,
    expected_size: Option<u64>,
    failpoint: SafePromotionTestFailpoint,
) -> Result<u64, DownloadFileError> {
    finalize_part_with(destination, expected_size, None, |artifact, target| {
        promote_verified_closed_with_failpoint_for_test(artifact, target, failpoint)
    })
}

fn finalize_part_with(
    destination: &Path,
    expected_size: Option<u64>,
    source_sha256: Option<&str>,
    promote: impl FnOnce(VerifiedClosedArtifact, &Path) -> Result<PromotionOutcome, PromotionError>,
) -> Result<u64, DownloadFileError> {
    let part = part_path(destination);
    let metadata = optional_file_metadata(&part)?.ok_or(DownloadFileError::InvalidPreparation)?;
    let size = metadata.len();
    if expected_size.is_some_and(|expected| expected != size) {
        return Err(DownloadFileError::IntegrityMismatch);
    }

    let artifact = match source_sha256 {
        Some(source_sha256) => {
            VerifiedClosedArtifact::after_streaming_integrity(part, size, source_sha256.to_owned())
        }
        None => VerifiedClosedArtifact::after_finalization_verification(part, size),
    }
    .map_err(|_| DownloadFileError::SafePromotionFailed)?;
    match promote(artifact, destination) {
        Ok(PromotionOutcome::Installed | PromotionOutcome::InstalledWithBackupResidue) => {
            PartMetaCheckpoint::clear(destination);
            Ok(size)
        }
        Err(PromotionError::RecoveryRequired) => Err(DownloadFileError::RecoveryRequired),
        Err(_) => Err(DownloadFileError::SafePromotionFailed),
    }
}

/// Reuse the transfer's no-follow and single-link checks for local recovery evidence.
pub(crate) fn open_checked_download_read(path: &Path) -> Result<File, DownloadFileError> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(DownloadFileError::RecoveryRequired);
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(DownloadFileError::RecoveryRequired);
    }
    let file = open_download_file_nofollow(path)?;
    guarded_file_metadata(&file)?;
    Ok(file)
}

pub(crate) fn open_checked_download_write(path: &Path) -> Result<File, DownloadFileError> {
    let file = open_part_restart_nofollow(path, false)?;
    guarded_file_metadata(&file)?;
    Ok(file)
}

fn optional_file_metadata(path: &Path) -> Result<Option<fs::Metadata>, DownloadFileError> {
    match open_download_file_nofollow(path) {
        Ok(file) => guarded_file_metadata(&file).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DownloadFileError::Filesystem(error)),
    }
}

fn guarded_file_metadata(file: &File) -> Result<fs::Metadata, DownloadFileError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || file_link_count(file)? != 1 {
        return Err(DownloadFileError::Filesystem(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "download path is not a single-link regular file",
        )));
    }
    Ok(metadata)
}

#[cfg(windows)]
fn open_download_file_nofollow(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0001 | 0x0000_0002 | 0x0000_0004)
        .custom_flags(0x0020_0000)
        .open(path)
}

#[cfg(windows)]
fn open_part_append_nofollow(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .append(true)
        .share_mode(0x0000_0001)
        .custom_flags(0x0020_0000)
        .open(path)
}

#[cfg(windows)]
fn open_part_restart_nofollow(path: &Path, create: bool) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .create(create)
        .truncate(false)
        .read(true)
        .write(true)
        .share_mode(0x0000_0001)
        .custom_flags(0x0020_0000)
        .open(path)
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn open_download_file_nofollow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(download_o_nofollow())
        .open(path)
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn open_part_append_nofollow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .append(true)
        .custom_flags(download_o_nofollow())
        .open(path)
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn open_part_restart_nofollow(path: &Path, create: bool) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .create(create)
        .truncate(false)
        .read(true)
        .write(true)
        .custom_flags(download_o_nofollow())
        .open(path)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn open_download_file_nofollow(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe download file opening is unsupported on this Unix target",
    ))
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn open_part_append_nofollow(path: &Path) -> std::io::Result<File> {
    open_download_file_nofollow(path)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn open_part_restart_nofollow(path: &Path, _create: bool) -> std::io::Result<File> {
    open_download_file_nofollow(path)
}

#[cfg(all(not(windows), not(unix)))]
fn open_download_file_nofollow(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe download file opening is unsupported",
    ))
}

#[cfg(all(not(windows), not(unix)))]
fn open_part_append_nofollow(_path: &Path) -> std::io::Result<File> {
    open_download_file_nofollow(_path)
}

#[cfg(all(not(windows), not(unix)))]
fn open_part_restart_nofollow(_path: &Path, _create: bool) -> std::io::Result<File> {
    open_download_file_nofollow(_path)
}

#[cfg(windows)]
fn file_link_count(file: &File) -> std::io::Result<u64> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) };
    if succeeded == 0 || info.dwFileAttributes & 0x0000_0400 != 0 {
        return Err(std::io::Error::other("unsafe download file identity"));
    }
    Ok(u64::from(info.nNumberOfLinks))
}

#[cfg(unix)]
fn file_link_count(file: &File) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    Ok(file.metadata()?.nlink())
}

#[cfg(all(not(windows), not(unix)))]
fn file_link_count(_file: &File) -> std::io::Result<u64> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "download file identity is unsupported",
    ))
}

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "m68k",
        target_arch = "powerpc",
        target_arch = "powerpc64"
    )
))]
const fn download_o_nofollow() -> i32 {
    0x0000_8000
}

#[cfg(all(
    target_os = "linux",
    not(any(
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "m68k",
        target_arch = "powerpc",
        target_arch = "powerpc64"
    ))
))]
const fn download_o_nofollow() -> i32 {
    0x0002_0000
}

#[cfg(all(
    target_os = "android",
    any(target_arch = "arm", target_arch = "aarch64")
))]
const fn download_o_nofollow() -> i32 {
    0x0000_8000
}

#[cfg(all(
    target_os = "android",
    any(target_arch = "x86", target_arch = "x86_64")
))]
const fn download_o_nofollow() -> i32 {
    0x0002_0000
}

#[cfg(all(target_os = "android", target_arch = "riscv64"))]
const fn download_o_nofollow() -> i32 {
    0x0040_0000
}

#[cfg(any(target_os = "solaris", target_os = "illumos"))]
const fn download_o_nofollow() -> i32 {
    0x0002_0000
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
const fn download_o_nofollow() -> i32 {
    0x0000_0100
}

/// 16 MiB sequential promotion buffer for transferring verified `.part` files from SSD staging.
pub const SSD_STAGING_COPY_BUFFER_BYTES: usize = 16 * 1024 * 1024;

/// Returns the `.part.meta.json` sidecar path associated with `destination`.
pub fn part_meta_path(destination: &Path) -> PathBuf {
    let mut os = OsString::from(part_path(destination).as_os_str());
    os.push(".meta.json");
    PathBuf::from(os)
}

/// Hashes a URL with SHA-256 so `.part.meta.json` never stores signed query parameters.
pub fn part_url_sha256(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

/// Extracts `(secs, nanos)` modification timestamp from file metadata when available.
pub fn file_mtime_tuple(metadata: &fs::Metadata) -> Option<(u64, u32)> {
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(UNIX_EPOCH).ok()?;
    Some((duration.as_secs(), duration.subsec_nanos()))
}

/// Byte progress of one segmented HTTP Range slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentProgressEntry {
    pub start: u64,
    pub end: u64,
    pub written: u64,
}

impl SegmentProgressEntry {
    pub fn remaining(&self) -> u64 {
        self.end
            .saturating_sub(self.start.saturating_add(self.written))
    }

    pub fn next_offset(&self) -> u64 {
        self.start.saturating_add(self.written).min(self.end)
    }

    pub fn is_complete(&self) -> bool {
        self.start.saturating_add(self.written) >= self.end
    }
}

/// Segmented transfer bitmap / range progress stored inside `.part.meta.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentedProgressMeta {
    pub total_length: u64,
    pub segments: Vec<SegmentProgressEntry>,
}

impl SegmentedProgressMeta {
    /// Verifies that `segments` form a sorted-contiguous, non-overlapping partition of `[0, total_length)`
    /// with valid byte progress per entry.
    pub fn is_valid_partition(&self) -> bool {
        if self.total_length == 0 || self.segments.is_empty() {
            return false;
        }
        let mut sorted = self.segments.clone();
        sorted.sort_by_key(|seg| (seg.start, seg.end));
        let mut cursor = 0_u64;
        for seg in &sorted {
            if seg.start != cursor || seg.end <= seg.start || seg.written > seg.end - seg.start {
                return false;
            }
            cursor = seg.end;
        }
        cursor == self.total_length
    }
}

/// Fast-resume metadata checkpoint stored beside `.part` as `<destination>.part.meta.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartMetaCheckpoint {
    pub schema_version: u32,
    pub file_len: u64,
    pub mtime_secs: u64,
    pub mtime_nanos: u32,
    pub url_sha256: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub segmented: Option<SegmentedProgressMeta>,
}

impl PartMetaCheckpoint {
    pub fn load(destination: &Path) -> Option<Self> {
        let raw = fs::read_to_string(part_meta_path(destination)).ok()?;
        let parsed: Self = serde_json::from_str(&raw).ok()?;
        (parsed.schema_version == 1).then_some(parsed)
    }

    pub fn save(&self, destination: &Path) -> Result<(), std::io::Error> {
        let path = part_meta_path(destination);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut temp_os = OsString::from(path.as_os_str());
        temp_os.push(".tmp");
        let temp_path = PathBuf::from(temp_os);
        let payload = serde_json::to_string(self)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)?;
        file.write_all(payload.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, &path)
    }

    pub fn clear(destination: &Path) {
        let _ = fs::remove_file(part_meta_path(destination));
    }

    /// Returns `true` when `.part` length, mtime, URL digest, and optional ETag match cleanly.
    pub fn matches_single_stream_resume(
        &self,
        part_metadata: &fs::Metadata,
        expected_part_len: u64,
        url_sha256: &str,
        etag: Option<&str>,
    ) -> bool {
        if self.segmented.is_some()
            || self.file_len != expected_part_len
            || part_metadata.len() != expected_part_len
            || self.url_sha256 != url_sha256
        {
            return false;
        }
        if file_mtime_tuple(part_metadata) != Some((self.mtime_secs, self.mtime_nanos)) {
            return false;
        }
        match (self.etag.as_deref(), etag) {
            (Some(saved), Some(current)) => saved == current,
            (None, None) => true,
            _ => false,
        }
    }

    pub fn total_segmented_written(&self) -> u64 {
        self.segmented.as_ref().map_or(0, |seg| {
            seg.segments
                .iter()
                .map(|entry| entry.written.min(entry.end.saturating_sub(entry.start)))
                .sum()
        })
    }

    pub fn is_incomplete_segmented(&self, part_len: u64) -> bool {
        let Some(seg) = &self.segmented else {
            return false;
        };
        seg.total_length == part_len
            && seg.is_valid_partition()
            && seg.segments.iter().any(|entry| !entry.is_complete())
    }

    pub fn is_segmented_complete(&self) -> bool {
        self.segmented.as_ref().is_some_and(|seg| {
            seg.is_valid_partition() && seg.segments.iter().all(SegmentProgressEntry::is_complete)
        })
    }
}

/// Builds and persists a `.part.meta.json` checkpoint from the current `.part` file handle.
pub fn save_part_meta_checkpoint(
    destination: &Path,
    part_file: &File,
    url_sha256: &str,
    etag: Option<String>,
    last_modified: Option<String>,
    segmented: Option<SegmentedProgressMeta>,
) -> Result<(), std::io::Error> {
    let metadata = part_file.metadata()?;
    let (mtime_secs, mtime_nanos) = file_mtime_tuple(&metadata).unwrap_or((0, 0));
    let checkpoint = PartMetaCheckpoint {
        schema_version: 1,
        file_len: metadata.len(),
        mtime_secs,
        mtime_nanos,
        url_sha256: url_sha256.to_owned(),
        etag,
        last_modified,
        segmented,
    };
    checkpoint.save(destination)
}

/// Opens (or creates) `.part` with read/write no-follow flags and pre-allocates `total_length`.
#[cfg(windows)]
fn set_sparse_file_best_effort(file: &File) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let mut bytes_returned: u32 = 0;
    // Mark multi-GB `.part` files sparse on NTFS before extending EOF so initial
    // preallocation does not materialize or lock gigabytes of zeroed clusters,
    // preventing multi-second handle hangs on task pause/cancel.
    unsafe {
        let _ = DeviceIoControl(
            file.as_raw_handle() as _,
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut bytes_returned,
            std::ptr::null_mut(),
        );
    }
}

pub fn open_part_preallocated_for_segmented(
    destination: &Path,
    total_length: u64,
    reset: bool,
) -> Result<File, DownloadFileError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let part = part_path(destination);
    let file = open_part_restart_nofollow(&part, true)?;
    let metadata = guarded_file_metadata(&file)?;
    if total_length > 0 {
        #[cfg(windows)]
        set_sparse_file_best_effort(&file);
    }
    if reset {
        PartMetaCheckpoint::clear(destination);
        file.set_len(0)?;
        file.set_len(total_length)?;
    } else if metadata.len() != total_length {
        file.set_len(total_length)?;
    }
    Ok(file)
}

/// Cross-platform positional file write helper used by concurrent HTTP Range segment workers.
pub fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        while !buf.is_empty() {
            match file.write_at(buf, offset) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "failed to write whole positional buffer",
                    ));
                }
                Ok(written) => {
                    buf = &buf[written..];
                    offset = offset.checked_add(written as u64).ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "positional write offset overflow",
                        )
                    })?;
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            match file.seek_write(buf, offset) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "failed to write whole positional buffer",
                    ));
                }
                Ok(written) => {
                    buf = &buf[written..];
                    offset = offset.checked_add(written as u64).ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "positional write offset overflow",
                        )
                    })?;
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }
}

/// Cross-platform positional exact-read helper used for resume overlap verification.
pub fn read_exact_at(
    file: &File,
    mut buf: &mut [u8],
    mut offset: u64,
) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        while !buf.is_empty() {
            match file.read_at(buf, offset) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "unexpected EOF during positional read",
                    ));
                }
                Ok(read_bytes) => {
                    buf = &mut buf[read_bytes..];
                    offset = offset.checked_add(read_bytes as u64).ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "positional read offset overflow",
                        )
                    })?;
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            match file.seek_read(buf, offset) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "unexpected EOF during positional read",
                    ));
                }
                Ok(read_bytes) => {
                    buf = &mut buf[read_bytes..];
                    offset = offset.checked_add(read_bytes as u64).ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "positional read offset overflow",
                        )
                    })?;
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = (file, buf, offset);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "positional read is unsupported on this target",
        ))
    }
}

/// Checks available space inside `staging_dir` via [`FreeSpaceProbe`].
pub fn check_ssd_staging_space(
    probe: &dyn FreeSpaceProbe,
    staging_dir: &Path,
    required_bytes: u64,
) -> Result<Option<VolumeSpace>, DownloadFileError> {
    fs::create_dir_all(staging_dir)?;
    let Some(space) = probe.volume_space(staging_dir) else {
        return Ok(None);
    };
    let floor = required_bytes.max(1);
    if space.free_bytes < floor {
        return Err(DownloadFileError::Filesystem(std::io::Error::other(
            format!(
                "insufficient SSD staging free space: free={} required={}",
                space.free_bytes, floor
            ),
        )));
    }
    Ok(Some(space))
}

/// Computes an isolated staging destination path inside `staging_dir` for `destination`.
pub fn staging_target_path(staging_dir: &Path, destination: &Path) -> PathBuf {
    let digest = Sha256::digest(destination.to_string_lossy().as_bytes());
    let mut prefix = String::with_capacity(16);
    for byte in &digest[..8] {
        use std::fmt::Write as _;
        let _ = write!(&mut prefix, "{byte:02x}");
    }
    let leaf = destination
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("artifact.bin");
    staging_dir
        .join(".eostage-staging")
        .join(format!("{prefix}-{leaf}"))
}

/// Heuristic slow-volume check identifying network shares, UNC paths, and external mount points.
pub fn is_slow_target_volume(destination: &Path) -> bool {
    let raw = destination.to_string_lossy();
    if raw.starts_with("\\\\") || raw.starts_with("//") {
        return true;
    }
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("/mnt/")
        || lower.starts_with("/media/")
        || lower.starts_with("/net/")
        || lower.starts_with("/smb/")
        || lower.starts_with("/cifs/")
    {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        if lower.starts_with("/volumes/") {
            return true;
        }
    }
    false
}

/// Sequentially copies the verified `.part` under `staged_destination` into `final_destination`'s
/// `.part` using a `16 MiB` buffer, atomically promotes `final_destination`, then removes staged files.
pub fn promote_staged_part_to_destination(
    staged_destination: &Path,
    final_destination: &Path,
    expected_size: Option<u64>,
    source_sha256: Option<&str>,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<u64, DownloadFileError> {
    if let Some(parent) = final_destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let staged_part = part_path(staged_destination);
    let mut reader = BufReader::with_capacity(
        SSD_STAGING_COPY_BUFFER_BYTES,
        open_checked_download_read(&staged_part)?,
    );
    let target_part = part_path(final_destination);
    let target_file = open_part_restart_nofollow(&target_part, true)?;
    guarded_file_metadata(&target_file)?;
    target_file.set_len(0)?;
    let mut writer = BufWriter::with_capacity(SSD_STAGING_COPY_BUFFER_BYTES, target_file);
    let mut copy_buf = vec![0_u8; SSD_STAGING_COPY_BUFFER_BYTES];
    loop {
        if is_cancelled() {
            drop(writer);
            let _ = fs::remove_file(&target_part);
            return Err(DownloadFileError::Filesystem(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "staged promotion cancelled",
            )));
        }
        let n = match reader.read(&mut copy_buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => {
                drop(writer);
                let _ = fs::remove_file(&target_part);
                return Err(DownloadFileError::Filesystem(err));
            }
        };
        if let Err(err) = writer.write_all(&copy_buf[..n]) {
            drop(writer);
            let _ = fs::remove_file(&target_part);
            return Err(DownloadFileError::Filesystem(err));
        }
    }
    writer.flush()?;
    writer.get_ref().sync_data()?;
    drop(writer);
    drop(reader);

    let promoted = match source_sha256 {
        Some(sha) => finalize_part_with_sha256(final_destination, expected_size, sha)?,
        None => finalize_part(final_destination, expected_size)?,
    };

    let _ = fs::remove_file(&staged_part);
    PartMetaCheckpoint::clear(staged_destination);
    Ok(promoted)
}

#[cfg(test)]
mod promotion_mapping_tests {
    use super::*;
    use crate::safe_promotion::{
        promotion_backup_path_for_qualification_test, SafePromotionTestFailpoint,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "insar-file-core-{label}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn promotion_failure_preserves_previous_destination_and_part() {
        let temp = TempRoot::new("failure");
        let destination = temp.0.join("scene.zip");
        let part = part_path(&destination);
        fs::write(&destination, b"previous-valid").unwrap();
        fs::write(&part, b"verified-new").unwrap();
        let error = finalize_part_with_failpoint_for_test(
            &destination,
            Some(12),
            SafePromotionTestFailpoint::FailBeforeInstall,
        )
        .unwrap_err();
        assert_eq!(error.code(), "DL005");
        assert_eq!(fs::read(&destination).unwrap(), b"previous-valid");
        assert_eq!(fs::read(&part).unwrap(), b"verified-new");
    }

    #[test]
    fn backup_residue_and_recovery_required_preserve_survivors() {
        let temp = TempRoot::new("residue");
        let destination = temp.0.join("scene.zip");
        let part = part_path(&destination);
        fs::write(&destination, b"previous-valid").unwrap();
        fs::write(&part, b"verified-new").unwrap();
        assert_eq!(
            finalize_part_with_failpoint_for_test(
                &destination,
                Some(12),
                SafePromotionTestFailpoint::InstallThenFailBackupCleanup,
            )
            .unwrap(),
            12
        );
        let backup = promotion_backup_path_for_qualification_test(&destination);
        assert_eq!(fs::read(&backup).unwrap(), b"previous-valid");

        let destination = temp.0.join("recovery.zip");
        let part = part_path(&destination);
        fs::write(&destination, b"previous-valid").unwrap();
        fs::write(&part, b"verified-new").unwrap();
        assert!(finalize_part_with_failpoint_for_test(
            &destination,
            Some(12),
            SafePromotionTestFailpoint::MoveDestinationToBackupThenRestoreFails,
        )
        .is_err());
        // Windows moved the destination to the backup; Linux linked it, so
        // the name keeps its previous contents.
        if cfg!(windows) {
            assert!(!destination.exists());
        } else {
            assert_eq!(fs::read(&destination).unwrap(), b"previous-valid");
        }
        assert_eq!(fs::read(&part).unwrap(), b"verified-new");
        assert_eq!(
            fs::read(promotion_backup_path_for_qualification_test(&destination)).unwrap(),
            b"previous-valid"
        );
    }
}
