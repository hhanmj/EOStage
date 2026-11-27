//! Bounded materialization of a manually supplied GACOS result archive.
//!
//! This module is deliberately narrower than the request and Store services.  A
//! caller has already obtained a result archive (or a single GeoTIFF) and owns
//! an output staging directory.  We validate archive structure while streaming
//! each member, copy only canonical GACOS product names into the flat staging
//! directory, and return the exact file sizes and SHA-256 values for the later
//! scientific admission boundary.  No task state, URL, credential, or source
//! directory is changed here.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use bzip2::read::BzDecoder;
use chrono::NaiveDate;
use flate2::read::MultiGzDecoder;
use insar_download::WorkerControl;
use insar_provider::gacos_artifact::{
    GACOS_MAX_ARTIFACT_BYTES, GACOS_MAX_DATES_PER_ADMISSION, GACOS_MAX_FILES_PER_ADMISSION,
    GACOS_MAX_RSC_HEADER_BYTES,
};
use sha2::{Digest, Sha256};
use tar::Archive;
#[cfg(test)]
use tar::EntryType;
use thiserror::Error;
use zip::ZipArchive;

use super::gacos_import_worker::FrozenGacosFile;

const COPY_CHUNK_BYTES: usize = 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = GACOS_MAX_ARTIFACT_BYTES;
const MAX_PRODUCT_BYTES: u64 = GACOS_MAX_ARTIFACT_BYTES;
const MAX_RSC_HEADER_BYTES: u64 = GACOS_MAX_RSC_HEADER_BYTES as u64;
const MAX_PRODUCTS: usize = GACOS_MAX_FILES_PER_ADMISSION;
const MAX_DATES: usize = GACOS_MAX_DATES_PER_ADMISSION;
const MAX_MEMBER_PATH_CHARS: usize = 32_767;
const MAX_MEMBERS: usize = 256;
const MAX_MEMBER_PATH_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ZIP_CENTRAL_DIRECTORY_BYTES: u64 = 2 * 1024 * 1024;
const ZIP_EOCD_BYTES: usize = 22;
const ZIP_EOCD_MAX_COMMENT_BYTES: usize = u16::MAX as usize;
const ZIP_EOCD_SEARCH_BYTES: usize = ZIP_EOCD_BYTES + ZIP_EOCD_MAX_COMMENT_BYTES;
const ZIP64_LOCATOR_BYTES: usize = 20;
const ZIP64_EOCD_BYTES: usize = 56;
const CONTROL_POLL: Duration = Duration::from_millis(25);
const TAR_TRAILER_BLOCK_BYTES: u64 = 512;

/// Typed failures from the archive materialization boundary.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(super) enum GacosResultArchiveError {
    #[error("GACOS result archive name is invalid")]
    InvalidArchiveName,
    #[error("GACOS result archive format is unsupported")]
    UnsupportedArchive,
    #[error("GACOS result archive magic does not match its declared container")]
    ContainerMagicMismatch,
    #[error("GACOS result archive container magic is unknown")]
    UnknownContainerFormat,
    #[error("GACOS result archive source is unavailable")]
    SourceUnavailable,
    #[error("GACOS result archive destination is unavailable")]
    DestinationUnavailable,
    #[error("GACOS result archive destination conflicts with an existing file")]
    DestinationConflict,
    #[error("GACOS result archive contains an unsafe member path or file type")]
    UnsafeMember,
    #[error("GACOS result archive contains a duplicate member")]
    DuplicateMember,
    #[error("GACOS result archive contains a duplicate flat product name")]
    DuplicateProductName,
    #[error("GACOS result archive contains an invalid product name")]
    InvalidProductName,
    #[error("GACOS result archive contains no supported products")]
    NoProducts,
    #[error("GACOS result archive exceeds the compressed-size bound")]
    CompressedTooLarge,
    #[error("GACOS result archive exceeds the expanded-size bound")]
    ExpandedTooLarge,
    #[error("GACOS result product exceeds the per-file bound")]
    ProductTooLarge,
    #[error("GACOS result RSC header exceeds the bounded header size")]
    RscHeaderTooLarge,
    #[error("GACOS result archive contains more than 40 products")]
    TooManyProducts,
    #[error("GACOS result archive contains more than 20 product dates")]
    TooManyDates,
    #[error("GACOS result archive contains more than 256 members")]
    TooManyMembers,
    #[error("GACOS result archive member paths exceed the metadata bound")]
    MemberPathTooLarge,
    #[error("GACOS result ZIP central directory exceeds the metadata bound")]
    CentralDirectoryTooLarge,
    #[error("GACOS result archive contains an encrypted ZIP member")]
    EncryptedZip,
    #[error("GACOS result archive is corrupt")]
    ArchiveCorrupt,
    #[error("GACOS result archive or product is truncated")]
    Truncated,
    #[error("GACOS result member size does not match its declared size")]
    MemberSizeMismatch,
    #[error("GACOS result TIFF signature is invalid")]
    InvalidTiff,
    #[error("GACOS result materialization was cancelled")]
    Cancelled,
}

impl GacosResultArchiveError {
    /// Stable application-facing code for the parent worker's failure mapper.
    pub(super) const fn code(self) -> &'static str {
        match self {
            Self::InvalidArchiveName | Self::UnsupportedArchive | Self::InvalidProductName => {
                "NATIVE_GACOS_ARCHIVE_INVALID"
            }
            Self::ContainerMagicMismatch => "NATIVE_GACOS_ARCHIVE_MAGIC_MISMATCH",
            Self::UnknownContainerFormat => "NATIVE_GACOS_ARCHIVE_FORMAT_UNKNOWN",
            Self::SourceUnavailable => "NATIVE_GACOS_ARCHIVE_SOURCE_UNAVAILABLE",
            Self::DestinationUnavailable => "NATIVE_GACOS_ARCHIVE_DESTINATION_UNAVAILABLE",
            Self::DestinationConflict => "NATIVE_GACOS_ARCHIVE_DESTINATION_CONFLICT",
            Self::UnsafeMember => "NATIVE_GACOS_ARCHIVE_UNSAFE_MEMBER",
            Self::DuplicateMember | Self::DuplicateProductName => "NATIVE_GACOS_ARCHIVE_DUPLICATE",
            Self::NoProducts => "NATIVE_GACOS_ARCHIVE_NO_PRODUCTS",
            Self::CompressedTooLarge
            | Self::ExpandedTooLarge
            | Self::ProductTooLarge
            | Self::RscHeaderTooLarge
            | Self::TooManyProducts
            | Self::TooManyDates
            | Self::TooManyMembers
            | Self::MemberPathTooLarge
            | Self::CentralDirectoryTooLarge => "NATIVE_GACOS_ARCHIVE_BOUNDS",
            Self::EncryptedZip => "NATIVE_GACOS_ARCHIVE_ENCRYPTED",
            Self::ArchiveCorrupt | Self::MemberSizeMismatch => "NATIVE_GACOS_ARCHIVE_CORRUPT",
            Self::Truncated => "NATIVE_GACOS_ARCHIVE_TRUNCATED",
            Self::InvalidTiff => "NATIVE_GACOS_ARCHIVE_TIFF_INVALID",
            Self::Cancelled => "NATIVE_GACOS_ARCHIVE_CANCELLED",
        }
    }

    /// Whether a caller may safely retry the same materialization operation.
    pub(super) const fn retryable(self) -> bool {
        matches!(
            self,
            Self::SourceUnavailable | Self::DestinationUnavailable | Self::Cancelled
        )
    }
}

/// Materialize one manually supplied result archive into caller-owned staging.
///
/// `original_name` is a fixed, secret-free transport name supplied by the
/// caller. It is used only to choose the container decoder; the result URL and
/// its basename never reach this module. `destination` may contain matching files from a
/// previous interrupted attempt, which are read and hash-compared but never
/// overwritten.  All newly created files use `create_new`.
pub(super) fn materialize_result(
    archive: &Path,
    original_name: &str,
    destination: &Path,
    control: &WorkerControl,
) -> Result<Vec<FrozenGacosFile>, GacosResultArchiveError> {
    materialize_result_with_control(archive, original_name, destination, control)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceKind {
    Auto,
    Zip,
    Tar,
    TarGz,
    TarBz2,
    SingleTiff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContainerMagic {
    Zip,
    Tar,
    Gzip,
    Bzip2,
    Tiff,
}

fn source_kind(original_name: &str) -> Result<SourceKind, GacosResultArchiveError> {
    validate_simple_name(original_name)?;
    let lower = original_name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Ok(SourceKind::TarGz)
    } else if lower.ends_with(".tar.bz2") {
        Ok(SourceKind::TarBz2)
    } else if lower.ends_with(".tar") {
        Ok(SourceKind::Tar)
    } else if lower.ends_with(".zip") {
        Ok(SourceKind::Zip)
    } else if lower.ends_with(".tif") {
        Ok(SourceKind::SingleTiff)
    } else if original_name == "gacos-result.archive" {
        Ok(SourceKind::Auto)
    } else {
        Err(GacosResultArchiveError::UnsupportedArchive)
    }
}

fn sniff_container_magic(source: &mut File) -> Result<ContainerMagic, GacosResultArchiveError> {
    let mut probe = source
        .try_clone()
        .map_err(|_| GacosResultArchiveError::SourceUnavailable)?;
    // Start the probe at byte zero. On Windows, the cloned handle shares the
    // file pointer with the original; the original is rewound after probing
    // below so every decoder starts at byte zero.
    probe
        .seek(SeekFrom::Start(0))
        .map_err(|_| GacosResultArchiveError::SourceUnavailable)?;
    let mut prefix = [0_u8; 263];
    let mut read = 0_usize;
    while read < prefix.len() {
        let count = probe
            .read(&mut prefix[read..])
            .map_err(|_| GacosResultArchiveError::SourceUnavailable)?;
        if count == 0 {
            break;
        }
        read += count;
    }
    source
        .seek(SeekFrom::Start(0))
        .map_err(|_| GacosResultArchiveError::SourceUnavailable)?;
    if read >= 4 {
        let first = &prefix[..4];
        if first == b"PK\x03\x04" || first == b"PK\x05\x06" || first == b"PK\x07\x08" {
            return Ok(ContainerMagic::Zip);
        }
        if first == b"II*\0" || first == b"MM\0*" {
            return Ok(ContainerMagic::Tiff);
        }
    }
    if read >= 3 && &prefix[..3] == b"BZh" {
        return Ok(ContainerMagic::Bzip2);
    }
    if read >= 2 && prefix[..2] == [0x1f, 0x8b] {
        return Ok(ContainerMagic::Gzip);
    }
    if read >= 263 && (&prefix[257..263] == b"ustar\0" || &prefix[257..263] == b"ustar ") {
        return Ok(ContainerMagic::Tar);
    }
    Err(GacosResultArchiveError::UnknownContainerFormat)
}

fn reconcile_container_kind(
    expected: SourceKind,
    magic: ContainerMagic,
) -> Result<SourceKind, GacosResultArchiveError> {
    let actual = match magic {
        ContainerMagic::Zip => SourceKind::Zip,
        ContainerMagic::Tar => SourceKind::Tar,
        ContainerMagic::Gzip => SourceKind::TarGz,
        ContainerMagic::Bzip2 => SourceKind::TarBz2,
        ContainerMagic::Tiff => SourceKind::SingleTiff,
    };
    if expected == SourceKind::Auto {
        return match actual {
            SourceKind::SingleTiff => Err(GacosResultArchiveError::UnsupportedArchive),
            _ => Ok(actual),
        };
    }
    if expected != actual {
        return Err(GacosResultArchiveError::ContainerMagicMismatch);
    }
    Ok(expected)
}

fn validate_simple_name(value: &str) -> Result<(), GacosResultArchiveError> {
    if value.is_empty()
        || value.chars().count() > MAX_MEMBER_PATH_CHARS
        || value.chars().any(char::is_control)
        || value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || value == "."
        || value == ".."
        || value.ends_with('.')
        || value.ends_with(' ')
    {
        return Err(GacosResultArchiveError::InvalidArchiveName);
    }
    Ok(())
}

fn ensure_destination(destination: &Path) -> Result<(), GacosResultArchiveError> {
    if !destination.is_absolute() {
        return Err(GacosResultArchiveError::DestinationUnavailable);
    }
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(GacosResultArchiveError::DestinationUnavailable);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(destination)
                .map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
            let metadata = fs::symlink_metadata(destination)
                .map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(GacosResultArchiveError::DestinationUnavailable);
            }
        }
        Err(_) => return Err(GacosResultArchiveError::DestinationUnavailable),
    }
    Ok(())
}

fn open_source(path: &Path) -> Result<(File, u64), GacosResultArchiveError> {
    let link_metadata =
        fs::symlink_metadata(path).map_err(|_| GacosResultArchiveError::SourceUnavailable)?;
    if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
        return Err(GacosResultArchiveError::SourceUnavailable);
    }
    let file = File::open(path).map_err(|_| GacosResultArchiveError::SourceUnavailable)?;
    let metadata = file
        .metadata()
        .map_err(|_| GacosResultArchiveError::SourceUnavailable)?;
    if !metadata.is_file() {
        return Err(GacosResultArchiveError::SourceUnavailable);
    }
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(GacosResultArchiveError::CompressedTooLarge);
    }
    Ok((file, metadata.len()))
}

fn check_control<C: ArchiveControl + ?Sized>(control: &C) -> Result<(), GacosResultArchiveError> {
    loop {
        if control.is_cancelled() {
            return Err(GacosResultArchiveError::Cancelled);
        }
        if !control.is_paused() {
            return Ok(());
        }
        thread::sleep(CONTROL_POLL);
    }
}

trait ArchiveControl {
    fn is_paused(&self) -> bool;
    fn is_cancelled(&self) -> bool;
}

impl ArchiveControl for WorkerControl {
    fn is_paused(&self) -> bool {
        WorkerControl::is_paused(self)
    }

    fn is_cancelled(&self) -> bool {
        WorkerControl::is_cancelled(self)
    }
}

struct MaterializeState<'a, C: ArchiveControl + ?Sized> {
    destination: &'a Path,
    control: &'a C,
    seen_members: BTreeSet<String>,
    member_count: usize,
    member_path_bytes: u64,
    product_names: BTreeSet<String>,
    product_dates: BTreeSet<NaiveDate>,
    products: Vec<FrozenGacosFile>,
    compressed_bytes: u64,
    expanded_bytes: u64,
}

impl<'a, C: ArchiveControl + ?Sized> MaterializeState<'a, C> {
    fn new(destination: &'a Path, control: &'a C, compressed_bytes: u64) -> Self {
        Self {
            destination,
            control,
            seen_members: BTreeSet::new(),
            member_count: 0,
            member_path_bytes: 0,
            product_names: BTreeSet::new(),
            product_dates: BTreeSet::new(),
            products: Vec::new(),
            compressed_bytes,
            expanded_bytes: 0,
        }
    }

    fn check(&self) -> Result<(), GacosResultArchiveError> {
        check_control(self.control)
    }

    fn note_compressed(&mut self, bytes: u64) -> Result<(), GacosResultArchiveError> {
        self.compressed_bytes = self
            .compressed_bytes
            .checked_add(bytes)
            .ok_or(GacosResultArchiveError::CompressedTooLarge)?;
        if self.compressed_bytes > MAX_ARCHIVE_BYTES {
            return Err(GacosResultArchiveError::CompressedTooLarge);
        }
        Ok(())
    }

    fn note_expanded(&mut self, bytes: u64) -> Result<(), GacosResultArchiveError> {
        if bytes > MAX_PRODUCT_BYTES {
            return Err(GacosResultArchiveError::ProductTooLarge);
        }
        self.expanded_bytes = self
            .expanded_bytes
            .checked_add(bytes)
            .ok_or(GacosResultArchiveError::ExpandedTooLarge)?;
        if self.expanded_bytes > MAX_ARCHIVE_BYTES {
            return Err(GacosResultArchiveError::ExpandedTooLarge);
        }
        Ok(())
    }

    fn register_member(
        &mut self,
        raw_name: &str,
        is_directory: bool,
    ) -> Result<SafeMemberPath, GacosResultArchiveError> {
        self.member_count = self
            .member_count
            .checked_add(1)
            .ok_or(GacosResultArchiveError::TooManyMembers)?;
        if self.member_count > MAX_MEMBERS {
            return Err(GacosResultArchiveError::TooManyMembers);
        }
        self.member_path_bytes = self
            .member_path_bytes
            .checked_add(raw_name.len() as u64)
            .ok_or(GacosResultArchiveError::MemberPathTooLarge)?;
        if self.member_path_bytes > MAX_MEMBER_PATH_BYTES {
            return Err(GacosResultArchiveError::MemberPathTooLarge);
        }
        let member = validate_member_path(raw_name, is_directory)?;
        if !self.seen_members.insert(member.normalized.clone()) {
            return Err(GacosResultArchiveError::DuplicateMember);
        }
        Ok(member)
    }

    fn register_product(
        &mut self,
        name: &str,
    ) -> Result<Option<ProductKind>, GacosResultArchiveError> {
        let Some((kind, product_date)) = classify_product_name(name)? else {
            return Ok(None);
        };
        if !self.product_names.insert(name.to_owned()) {
            return Err(GacosResultArchiveError::DuplicateProductName);
        }
        if self.product_names.len() > MAX_PRODUCTS {
            return Err(GacosResultArchiveError::TooManyProducts);
        }
        if self.product_dates.insert(product_date) && self.product_dates.len() > MAX_DATES {
            return Err(GacosResultArchiveError::TooManyDates);
        }
        if kind == ProductKind::Rsc {
            // The actual size is checked by `materialize_product`; this branch
            // only documents that RSC has its narrower bound.
        }
        Ok(Some(kind))
    }

    fn materialize_product<R: Read>(
        &mut self,
        reader: &mut R,
        name: &str,
        kind: ProductKind,
        expected_size: u64,
        source_kind: ReadSource,
    ) -> Result<(), GacosResultArchiveError> {
        if expected_size == 0 {
            return Err(GacosResultArchiveError::InvalidProductName);
        }
        if expected_size > MAX_PRODUCT_BYTES {
            return Err(GacosResultArchiveError::ProductTooLarge);
        }
        if kind == ProductKind::Rsc && expected_size > MAX_RSC_HEADER_BYTES {
            return Err(GacosResultArchiveError::RscHeaderTooLarge);
        }
        self.note_expanded(expected_size)?;
        self.check()?;

        let output_path = self.destination.join(name);
        let partial_path = product_partial_path(&output_path);
        let existing = match fs::symlink_metadata(&output_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(GacosResultArchiveError::DestinationConflict);
                }
                true
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(_) => return Err(GacosResultArchiveError::DestinationUnavailable),
        };

        let (actual_size, sha256) = if existing {
            // A completed product and a leftover temporary product are
            // ambiguous. Keep the temporary file and fail closed rather than
            // deleting a file that may belong to another attempt.
            if existing_product_partial_len(&partial_path, expected_size)?.is_some() {
                return Err(GacosResultArchiveError::DestinationConflict);
            }
            let (actual_size, sha256) = stream_reader(
                reader,
                expected_size,
                None,
                self.control,
                source_kind,
                kind == ProductKind::Tiff,
            )?;
            let (existing_size, existing_hash) = hash_existing(&output_path, self.control)?;
            if existing_size != actual_size || existing_hash != sha256 {
                return Err(GacosResultArchiveError::DestinationConflict);
            }
            (actual_size, sha256)
        } else {
            let prefix_len = existing_product_partial_len(&partial_path, expected_size)?;
            let result = stream_product_to_partial(
                reader,
                expected_size,
                &partial_path,
                prefix_len,
                self.control,
                source_kind,
                kind == ProductKind::Tiff,
            )?;
            promote_product_partial(
                &partial_path,
                &output_path,
                result.0,
                &result.1,
                self.control,
            )?;
            result
        };

        self.products.push(FrozenGacosFile {
            path: output_path,
            name: name.to_owned(),
            size_bytes: actual_size,
            sha256,
        });
        Ok(())
    }

    fn consume_non_product<R: Read>(
        &mut self,
        reader: &mut R,
        expected_size: u64,
    ) -> Result<(), GacosResultArchiveError> {
        self.note_expanded(expected_size)?;
        stream_reader(
            reader,
            expected_size,
            None,
            self.control,
            ReadSource::Archive,
            false,
        )?;
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<FrozenGacosFile>, GacosResultArchiveError> {
        if self.products.is_empty() {
            return Err(GacosResultArchiveError::NoProducts);
        }
        validate_destination_entries(self.destination, &self.product_names)?;
        self.products
            .sort_by(|left, right| left.name.cmp(&right.name));
        Ok(self.products)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProductKind {
    Ztd,
    Rsc,
    Tiff,
}

fn classify_product_name(
    name: &str,
) -> Result<Option<(ProductKind, NaiveDate)>, GacosResultArchiveError> {
    let lower = name.to_ascii_lowercase();
    let (kind, suffix) = if name.ends_with(".ztd.rsc") {
        (ProductKind::Rsc, ".ztd.rsc")
    } else if name.ends_with(".ztd") {
        (ProductKind::Ztd, ".ztd")
    } else if name.ends_with(".ztd.tif") {
        (ProductKind::Tiff, ".ztd.tif")
    } else if name.ends_with(".tif") {
        (ProductKind::Tiff, ".tif")
    } else if lower.ends_with(".ztd.rsc")
        || lower.ends_with(".ztd")
        || lower.ends_with(".ztd.tif")
        || lower.ends_with(".tif")
    {
        return Err(GacosResultArchiveError::InvalidProductName);
    } else {
        return Ok(None);
    };
    let date_token = name
        .strip_suffix(suffix)
        .ok_or(GacosResultArchiveError::InvalidProductName)?;
    if date_token.len() != 8 || !date_token.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GacosResultArchiveError::InvalidProductName);
    }
    let product_date = NaiveDate::parse_from_str(date_token, "%Y%m%d")
        .map_err(|_| GacosResultArchiveError::InvalidProductName)?;
    Ok(Some((kind, product_date)))
}

struct SafeMemberPath {
    normalized: String,
    basename: String,
}

fn validate_member_path(
    raw_name: &str,
    is_directory: bool,
) -> Result<SafeMemberPath, GacosResultArchiveError> {
    if raw_name.is_empty()
        || raw_name.chars().count() > MAX_MEMBER_PATH_CHARS
        || raw_name.chars().any(char::is_control)
        || raw_name.contains('\\')
        || raw_name.contains(':')
        || raw_name.starts_with('/')
    {
        return Err(GacosResultArchiveError::UnsafeMember);
    }
    // Archive directory headers are allowed to omit the conventional trailing
    // slash. A regular-file header with a trailing slash is ambiguous and is
    // rejected rather than flattened into a product name.
    if !is_directory && raw_name.ends_with('/') {
        return Err(GacosResultArchiveError::UnsafeMember);
    }
    let path_without_trailing_slash = raw_name.strip_suffix('/').unwrap_or(raw_name);
    if path_without_trailing_slash.is_empty() {
        return Err(GacosResultArchiveError::UnsafeMember);
    }
    let mut components = Vec::new();
    for component in path_without_trailing_slash.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with('.')
            || component.ends_with(' ')
        {
            return Err(GacosResultArchiveError::UnsafeMember);
        }
        components.push(component);
    }
    let basename = components
        .last()
        .ok_or(GacosResultArchiveError::UnsafeMember)?
        .to_string();
    Ok(SafeMemberPath {
        normalized: components.join("/"),
        basename,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadSource {
    Archive,
    Source,
}

fn map_read_error(error: io::Error, source: ReadSource) -> GacosResultArchiveError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        GacosResultArchiveError::Truncated
    } else if source == ReadSource::Archive {
        GacosResultArchiveError::ArchiveCorrupt
    } else {
        GacosResultArchiveError::SourceUnavailable
    }
}

fn stream_reader<R: Read, C: ArchiveControl + ?Sized>(
    reader: &mut R,
    expected_size: u64,
    mut output: Option<&mut File>,
    control: &C,
    source: ReadSource,
    require_tiff_magic: bool,
) -> Result<(u64, String), GacosResultArchiveError> {
    let mut buffer = vec![0_u8; COPY_CHUNK_BYTES];
    let mut bytes_read = 0_u64;
    let mut hasher = Sha256::new();

    // Validate the TIFF signature before creating any destination bytes. The
    // archive reader is not necessarily seekable, so consume the prefix and
    // hash/write it exactly once before streaming the remainder.
    if require_tiff_magic {
        if expected_size < 4 {
            return Err(GacosResultArchiveError::Truncated);
        }
        let mut magic = [0_u8; 4];
        let mut magic_bytes = 0_usize;
        while magic_bytes < magic.len() {
            check_control(control)?;
            let count = reader
                .read(&mut magic[magic_bytes..])
                .map_err(|error| map_read_error(error, source))?;
            if count == 0 {
                return Err(GacosResultArchiveError::Truncated);
            }
            magic_bytes += count;
        }
        if magic != *b"II*\0" && magic != *b"MM\0*" {
            return Err(GacosResultArchiveError::InvalidTiff);
        }
        if let Some(handle) = output.as_mut() {
            handle
                .write_all(&magic)
                .map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
        }
        hasher.update(magic);
        bytes_read = 4;
    }

    while bytes_read < expected_size {
        check_control(control)?;
        let count = reader
            .read(&mut buffer)
            .map_err(|error| map_read_error(error, source))?;
        if count == 0 {
            return Err(GacosResultArchiveError::Truncated);
        }
        let next = bytes_read
            .checked_add(count as u64)
            .ok_or(GacosResultArchiveError::ExpandedTooLarge)?;
        if next > expected_size {
            return Err(GacosResultArchiveError::MemberSizeMismatch);
        }
        if let Some(handle) = output.as_mut() {
            handle
                .write_all(&buffer[..count])
                .map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
        }
        hasher.update(&buffer[..count]);
        bytes_read = next;
    }
    // One bounded read past the declared member size is required for ZIP's
    // reader to perform its CRC check, and also catches a lying size field.
    check_control(control)?;
    let mut extra = [0_u8; 1];
    let extra_count = reader
        .read(&mut extra)
        .map_err(|error| map_read_error(error, source))?;
    if extra_count != 0 {
        return Err(GacosResultArchiveError::MemberSizeMismatch);
    }
    Ok((bytes_read, format!("{:x}", hasher.finalize())))
}

fn product_partial_path(output: &Path) -> PathBuf {
    let mut value = output.as_os_str().to_os_string();
    value.push(".part");
    PathBuf::from(value)
}

fn existing_product_partial_len(
    path: &Path,
    expected_size: u64,
) -> Result<Option<u64>, GacosResultArchiveError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() > expected_size
            {
                return Err(GacosResultArchiveError::DestinationConflict);
            }
            Ok(Some(metadata.len()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(GacosResultArchiveError::DestinationUnavailable),
    }
}

fn stream_product_to_partial<R: Read, C: ArchiveControl + ?Sized>(
    reader: &mut R,
    expected_size: u64,
    partial: &Path,
    prefix_len: Option<u64>,
    control: &C,
    source: ReadSource,
    require_tiff_magic: bool,
) -> Result<(u64, String), GacosResultArchiveError> {
    if require_tiff_magic && expected_size < 4 {
        return Err(GacosResultArchiveError::Truncated);
    }
    let existing_partial = prefix_len.is_some();
    let prefix_len = prefix_len.unwrap_or_default();
    let mut prefix_file = if prefix_len > 0 {
        let metadata = fs::symlink_metadata(partial)
            .map_err(|_| GacosResultArchiveError::DestinationConflict)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != prefix_len
        {
            return Err(GacosResultArchiveError::DestinationConflict);
        }
        Some(File::open(partial).map_err(|_| GacosResultArchiveError::DestinationConflict)?)
    } else {
        None
    };
    let mut output = if !existing_partial {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(partial)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(GacosResultArchiveError::DestinationConflict);
            }
            Err(_) => return Err(GacosResultArchiveError::DestinationUnavailable),
        }
    } else {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(partial)
            .map_err(|_| GacosResultArchiveError::DestinationConflict)?;
        let metadata = file
            .metadata()
            .map_err(|_| GacosResultArchiveError::DestinationConflict)?;
        if !metadata.is_file() || metadata.len() != prefix_len {
            return Err(GacosResultArchiveError::DestinationConflict);
        }
        file
    };
    output
        .seek(SeekFrom::Start(prefix_len))
        .map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;

    let mut prefix_buffer = vec![0_u8; COPY_CHUNK_BYTES];
    let mut buffer = vec![0_u8; COPY_CHUNK_BYTES];
    let mut hasher = Sha256::new();
    let mut bytes_read = 0_u64;

    if require_tiff_magic {
        let mut magic = [0_u8; 4];
        reader
            .read_exact(&mut magic)
            .map_err(|error| map_read_error(error, source))?;
        if magic != *b"II*\0" && magic != *b"MM\0*" {
            return Err(GacosResultArchiveError::InvalidTiff);
        }
        consume_product_bytes(
            &magic,
            0,
            prefix_len,
            &mut prefix_file,
            &mut output,
            &mut prefix_buffer,
            &mut hasher,
        )?;
        bytes_read = magic.len() as u64;
    }

    while bytes_read < expected_size {
        check_control(control)?;
        let count = reader
            .read(&mut buffer)
            .map_err(|error| map_read_error(error, source))?;
        if count == 0 {
            return Err(GacosResultArchiveError::Truncated);
        }
        let next = bytes_read
            .checked_add(count as u64)
            .ok_or(GacosResultArchiveError::ExpandedTooLarge)?;
        if next > expected_size {
            return Err(GacosResultArchiveError::MemberSizeMismatch);
        }
        consume_product_bytes(
            &buffer[..count],
            bytes_read,
            prefix_len,
            &mut prefix_file,
            &mut output,
            &mut prefix_buffer,
            &mut hasher,
        )?;
        bytes_read = next;
    }

    check_control(control)?;
    let mut extra = [0_u8; 1];
    let extra_count = reader
        .read(&mut extra)
        .map_err(|error| map_read_error(error, source))?;
    if extra_count != 0 {
        return Err(GacosResultArchiveError::MemberSizeMismatch);
    }
    output
        .flush()
        .map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
    output
        .sync_all()
        .map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
    Ok((bytes_read, format!("{:x}", hasher.finalize())))
}

fn consume_product_bytes(
    bytes: &[u8],
    offset: u64,
    prefix_len: u64,
    prefix_file: &mut Option<File>,
    output: &mut File,
    prefix_buffer: &mut Vec<u8>,
    hasher: &mut Sha256,
) -> Result<(), GacosResultArchiveError> {
    let end = offset
        .checked_add(bytes.len() as u64)
        .ok_or(GacosResultArchiveError::ExpandedTooLarge)?;
    let compare_end = end.min(prefix_len);
    if compare_end > offset {
        let compare_len = usize::try_from(compare_end - offset)
            .map_err(|_| GacosResultArchiveError::DestinationConflict)?;
        let expected = prefix_file
            .as_mut()
            .ok_or(GacosResultArchiveError::DestinationConflict)?;
        prefix_buffer.resize(compare_len, 0);
        expected
            .read_exact(&mut prefix_buffer[..compare_len])
            .map_err(|_| GacosResultArchiveError::DestinationConflict)?;
        if prefix_buffer[..compare_len] != bytes[..compare_len] {
            return Err(GacosResultArchiveError::DestinationConflict);
        }
    }
    if end > prefix_len {
        let write_start = usize::try_from(prefix_len.saturating_sub(offset))
            .map_err(|_| GacosResultArchiveError::DestinationConflict)?;
        output
            .write_all(&bytes[write_start..])
            .map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
    }
    hasher.update(bytes);
    Ok(())
}

fn promote_product_partial<C: ArchiveControl + ?Sized>(
    partial: &Path,
    destination: &Path,
    expected_size: u64,
    expected_hash: &str,
    control: &C,
) -> Result<(), GacosResultArchiveError> {
    check_control(control)?;
    let metadata =
        fs::symlink_metadata(partial).map_err(|_| GacosResultArchiveError::DestinationConflict)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != expected_size {
        return Err(GacosResultArchiveError::DestinationConflict);
    }
    match fs::symlink_metadata(destination) {
        Ok(existing) => {
            if existing.file_type().is_symlink() || !existing.is_file() {
                return Err(GacosResultArchiveError::DestinationConflict);
            }
            let (size, hash) = hash_existing(destination, control)?;
            if size != expected_size || hash != expected_hash {
                return Err(GacosResultArchiveError::DestinationConflict);
            }
            let (partial_size, partial_hash) = hash_existing(partial, control)?;
            if partial_size != expected_size || partial_hash != expected_hash {
                return Err(GacosResultArchiveError::DestinationConflict);
            }
            fs::remove_file(partial).map_err(|_| GacosResultArchiveError::DestinationUnavailable)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::rename(partial, destination)
            .map_err(|_| GacosResultArchiveError::DestinationConflict),
        Err(_) => Err(GacosResultArchiveError::DestinationUnavailable),
    }
}

fn hash_existing<C: ArchiveControl + ?Sized>(
    path: &Path,
    control: &C,
) -> Result<(u64, String), GacosResultArchiveError> {
    let mut file = File::open(path).map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
    let metadata = file
        .metadata()
        .map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
    if !metadata.is_file() {
        return Err(GacosResultArchiveError::DestinationConflict);
    }
    if metadata.len() > MAX_PRODUCT_BYTES {
        return Err(GacosResultArchiveError::ProductTooLarge);
    }
    stream_reader(
        &mut file,
        metadata.len(),
        None,
        control,
        ReadSource::Source,
        false,
    )
}

fn validate_destination_entries(
    destination: &Path,
    product_names: &BTreeSet<String>,
) -> Result<(), GacosResultArchiveError> {
    let entries =
        fs::read_dir(destination).map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
    for entry in entries {
        let entry = entry.map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| GacosResultArchiveError::DestinationUnavailable)?;
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or(GacosResultArchiveError::DestinationConflict)?;
        let is_product = product_names.contains(name);
        let is_product_partial = name
            .strip_suffix(".part")
            .is_some_and(|base| product_names.contains(base));
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || (!is_product && !is_product_partial)
        {
            return Err(GacosResultArchiveError::DestinationConflict);
        }
    }
    Ok(())
}

fn materialize_result_with_control<C: ArchiveControl + ?Sized>(
    archive: &Path,
    original_name: &str,
    destination: &Path,
    control: &C,
) -> Result<Vec<FrozenGacosFile>, GacosResultArchiveError> {
    check_control(control)?;
    let expected_kind = source_kind(original_name)?;
    ensure_destination(destination)?;
    let (mut source, source_size) = open_source(archive)?;
    let kind = match sniff_container_magic(&mut source) {
        Ok(magic) => reconcile_container_kind(expected_kind, magic)?,
        Err(GacosResultArchiveError::UnknownContainerFormat)
            if expected_kind == SourceKind::SingleTiff =>
        {
            SourceKind::SingleTiff
        }
        Err(error) => return Err(error),
    };
    match kind {
        SourceKind::Auto => Err(GacosResultArchiveError::UnknownContainerFormat),
        SourceKind::SingleTiff => {
            materialize_single_tiff(source, source_size, original_name, destination, control)
        }
        SourceKind::Zip => materialize_zip(source, source_size, destination, control),
        SourceKind::Tar => materialize_tar(source, source_size, destination, control),
        SourceKind::TarGz => materialize_tar_gz(source, source_size, destination, control),
        SourceKind::TarBz2 => materialize_tar_bz2(source, source_size, destination, control),
    }
}

fn materialize_single_tiff<C: ArchiveControl + ?Sized>(
    mut source: File,
    source_size: u64,
    original_name: &str,
    destination: &Path,
    control: &C,
) -> Result<Vec<FrozenGacosFile>, GacosResultArchiveError> {
    let Some((kind, _)) = classify_product_name(original_name)? else {
        return Err(GacosResultArchiveError::InvalidProductName);
    };
    if kind != ProductKind::Tiff {
        return Err(GacosResultArchiveError::InvalidProductName);
    }
    if source_size < 4 {
        return Err(GacosResultArchiveError::Truncated);
    }
    let mut magic = [0_u8; 4];
    source
        .read_exact(&mut magic)
        .map_err(|error| map_read_error(error, ReadSource::Source))?;
    if magic != *b"II*\0" && magic != *b"MM\0*" {
        return Err(GacosResultArchiveError::InvalidTiff);
    }
    let mut reader = Cursor::new(magic.to_vec()).chain(source);
    let mut state = MaterializeState::new(destination, control, source_size);
    state.register_member(original_name, false)?;
    let kind = state
        .register_product(original_name)?
        .ok_or(GacosResultArchiveError::InvalidProductName)?;
    state.materialize_product(
        &mut reader,
        original_name,
        kind,
        source_size,
        ReadSource::Source,
    )?;
    state.finish()
}

fn read_exact_at(
    source: &File,
    offset: u64,
    output: &mut [u8],
) -> Result<(), GacosResultArchiveError> {
    let mut probe = source
        .try_clone()
        .map_err(|_| GacosResultArchiveError::SourceUnavailable)?;
    probe
        .seek(SeekFrom::Start(offset))
        .map_err(|_| GacosResultArchiveError::SourceUnavailable)?;
    probe.read_exact(output).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            GacosResultArchiveError::Truncated
        } else {
            GacosResultArchiveError::SourceUnavailable
        }
    })
}

fn little_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn little_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn little_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

fn validate_zip_directory_bounds(
    source_size: u64,
    entry_count: u64,
    central_directory_size: u64,
    central_directory_offset: u64,
) -> Result<(), GacosResultArchiveError> {
    if entry_count > MAX_MEMBERS as u64 {
        return Err(GacosResultArchiveError::TooManyMembers);
    }
    if central_directory_size > MAX_ZIP_CENTRAL_DIRECTORY_BYTES {
        return Err(GacosResultArchiveError::CentralDirectoryTooLarge);
    }
    if central_directory_offset > source_size
        || central_directory_size > source_size - central_directory_offset
    {
        return Err(GacosResultArchiveError::ArchiveCorrupt);
    }
    Ok(())
}

fn validate_zip_central_directory(
    source: &File,
    source_size: u64,
) -> Result<(), GacosResultArchiveError> {
    if source_size < ZIP_EOCD_BYTES as u64 {
        return Err(GacosResultArchiveError::Truncated);
    }
    let tail_size = source_size.min(ZIP_EOCD_SEARCH_BYTES as u64) as usize;
    let tail_start = source_size - tail_size as u64;
    let mut tail = vec![0_u8; tail_size];
    read_exact_at(source, tail_start, &mut tail)?;
    let eocd_index = (0..=tail.len() - ZIP_EOCD_BYTES)
        .rev()
        .find(|&index| {
            if &tail[index..index + 4] != b"PK\x05\x06" {
                return false;
            }
            let comment_length = little_u16(&tail[index + 20..index + 22]) as usize;
            index
                .checked_add(ZIP_EOCD_BYTES)
                .and_then(|end| end.checked_add(comment_length))
                == Some(tail.len())
        })
        .ok_or(GacosResultArchiveError::Truncated)?;
    let eocd = &tail[eocd_index..eocd_index + ZIP_EOCD_BYTES];
    let eocd_absolute = tail_start + eocd_index as u64;
    let disk_number = little_u16(&eocd[4..6]);
    let central_directory_disk = little_u16(&eocd[6..8]);
    if disk_number != 0 || central_directory_disk != 0 {
        return Err(GacosResultArchiveError::ArchiveCorrupt);
    }
    let entry_count = little_u16(&eocd[10..12]) as u64;
    let total_entries = little_u16(&eocd[8..10]) as u64;
    let central_directory_size = little_u32(&eocd[12..16]) as u64;
    let central_directory_offset = little_u32(&eocd[16..20]) as u64;
    let needs_zip64 = entry_count == u16::MAX as u64
        || total_entries == u16::MAX as u64
        || central_directory_size == u32::MAX as u64
        || central_directory_offset == u32::MAX as u64;
    if !needs_zip64 {
        if entry_count != total_entries {
            return Err(GacosResultArchiveError::ArchiveCorrupt);
        }
        return validate_zip_directory_bounds(
            source_size,
            total_entries,
            central_directory_size,
            central_directory_offset,
        );
    }

    if eocd_absolute < ZIP64_LOCATOR_BYTES as u64 {
        return Err(GacosResultArchiveError::Truncated);
    }
    let locator_offset = eocd_absolute - ZIP64_LOCATOR_BYTES as u64;
    let mut locator = [0_u8; ZIP64_LOCATOR_BYTES];
    read_exact_at(source, locator_offset, &mut locator)?;
    if &locator[..4] != b"PK\x06\x07"
        || little_u32(&locator[4..8]) != 0
        || little_u32(&locator[16..20]) != 1
    {
        return Err(GacosResultArchiveError::ArchiveCorrupt);
    }
    let zip64_offset = little_u64(&locator[8..16]);
    let mut zip64 = [0_u8; ZIP64_EOCD_BYTES];
    read_exact_at(source, zip64_offset, &mut zip64)?;
    let zip64_record_size = little_u64(&zip64[4..12]);
    if &zip64[..4] != b"PK\x06\x06" || zip64_record_size < 44 {
        return Err(GacosResultArchiveError::ArchiveCorrupt);
    }
    if zip64_record_size > MAX_ZIP_CENTRAL_DIRECTORY_BYTES {
        return Err(GacosResultArchiveError::CentralDirectoryTooLarge);
    }
    let zip64_end = zip64_offset
        .checked_add(12)
        .and_then(|offset| offset.checked_add(zip64_record_size))
        .ok_or(GacosResultArchiveError::CentralDirectoryTooLarge)?;
    if zip64_end > source_size {
        return Err(GacosResultArchiveError::Truncated);
    }
    if little_u32(&zip64[16..20]) != 0 || little_u32(&zip64[20..24]) != 0 {
        return Err(GacosResultArchiveError::ArchiveCorrupt);
    }
    let zip64_entries_on_disk = little_u64(&zip64[24..32]);
    let zip64_total_entries = little_u64(&zip64[32..40]);
    if zip64_entries_on_disk != zip64_total_entries {
        return Err(GacosResultArchiveError::ArchiveCorrupt);
    }
    validate_zip_directory_bounds(
        source_size,
        zip64_total_entries,
        little_u64(&zip64[40..48]),
        little_u64(&zip64[48..56]),
    )
}

fn materialize_zip<C: ArchiveControl + ?Sized>(
    source: File,
    source_size: u64,
    destination: &Path,
    control: &C,
) -> Result<Vec<FrozenGacosFile>, GacosResultArchiveError> {
    validate_zip_central_directory(&source, source_size)?;
    let mut archive =
        ZipArchive::new(source).map_err(|_| GacosResultArchiveError::ArchiveCorrupt)?;
    let mut state = MaterializeState::new(destination, control, 0);
    for index in 0..archive.len() {
        state.check()?;
        let mut member = archive
            .by_index(index)
            .map_err(|_| GacosResultArchiveError::ArchiveCorrupt)?;
        if member.encrypted() {
            return Err(GacosResultArchiveError::EncryptedZip);
        }
        if member.enclosed_name().is_none() {
            return Err(GacosResultArchiveError::UnsafeMember);
        }
        let raw_name = member
            .name()
            .map_err(|_| GacosResultArchiveError::UnsafeMember)?
            .into_owned();
        let is_directory = member.is_dir();
        if member.is_symlink()
            || member.unix_mode().is_some_and(|mode| {
                let file_type = mode & 0o170_000;
                if is_directory {
                    file_type != 0 && file_type != 0o040_000
                } else {
                    file_type != 0 && file_type != 0o100_000
                }
            })
        {
            return Err(GacosResultArchiveError::UnsafeMember);
        }
        let safe = state.register_member(&raw_name, is_directory)?;
        state.note_compressed(member.compressed_size())?;
        let size = member.size();
        if is_directory {
            if size != 0 {
                return Err(GacosResultArchiveError::ArchiveCorrupt);
            }
            continue;
        }
        let product = state.register_product(&safe.basename)?;
        match product {
            Some(kind) => state.materialize_product(
                &mut member,
                &safe.basename,
                kind,
                size,
                ReadSource::Archive,
            )?,
            None => state.consume_non_product(&mut member, size)?,
        }
    }
    // ZipArchive validates member CRCs while each member is read.  Keep the
    // source-size bound as an explicit check even when the central directory
    // reports less compressed data than the containing file.
    if source_size > MAX_ARCHIVE_BYTES {
        return Err(GacosResultArchiveError::CompressedTooLarge);
    }
    state.finish()
}

fn materialize_tar<C: ArchiveControl + ?Sized>(
    source: File,
    source_size: u64,
    destination: &Path,
    control: &C,
) -> Result<Vec<FrozenGacosFile>, GacosResultArchiveError> {
    materialize_tar_reader(source, source_size, destination, control)
}

fn materialize_tar_gz<C: ArchiveControl + ?Sized>(
    source: File,
    source_size: u64,
    destination: &Path,
    control: &C,
) -> Result<Vec<FrozenGacosFile>, GacosResultArchiveError> {
    materialize_tar_reader(
        MultiGzDecoder::new(source),
        source_size,
        destination,
        control,
    )
}

fn materialize_tar_bz2<C: ArchiveControl + ?Sized>(
    source: File,
    source_size: u64,
    destination: &Path,
    control: &C,
) -> Result<Vec<FrozenGacosFile>, GacosResultArchiveError> {
    materialize_tar_reader(BzDecoder::new(source), source_size, destination, control)
}

fn materialize_tar_reader<R: Read, C: ArchiveControl + ?Sized>(
    reader: R,
    source_size: u64,
    destination: &Path,
    control: &C,
) -> Result<Vec<FrozenGacosFile>, GacosResultArchiveError> {
    let mut archive = Archive::new(reader);
    let mut state = MaterializeState::new(destination, control, source_size);
    {
        let entries = archive
            .entries()
            .map_err(|_| GacosResultArchiveError::ArchiveCorrupt)?
            .raw(true);
        for entry_result in entries {
            state.check()?;
            let mut entry = entry_result.map_err(|_| GacosResultArchiveError::ArchiveCorrupt)?;
            let entry_type = entry.header().entry_type();
            // tar's normal iterator eagerly reads GNU long-name and PAX
            // extension payloads with an unbounded read_all(). Raw iteration
            // exposes those headers so they can be rejected before any such
            // payload is allocated.
            if entry_type.is_gnu_longname()
                || entry_type.is_gnu_longlink()
                || entry_type.is_pax_local_extensions()
                || entry_type.is_pax_global_extensions()
            {
                return Err(GacosResultArchiveError::UnsafeMember);
            }
            let raw_name = entry
                .path()
                .map_err(|_| GacosResultArchiveError::UnsafeMember)?
                .into_owned();
            let raw_name = raw_name
                .to_str()
                .ok_or(GacosResultArchiveError::UnsafeMember)?
                .to_owned();
            let is_directory = entry_type.is_dir();
            let safe = state.register_member(&raw_name, is_directory)?;
            if !is_directory
                && (!entry_type.is_file()
                    || entry_type.is_symlink()
                    || entry_type.is_hard_link()
                    || entry_type.is_character_special()
                    || entry_type.is_block_special()
                    || entry_type.is_fifo()
                    || entry_type.is_contiguous()
                    || entry_type.is_gnu_sparse())
            {
                return Err(GacosResultArchiveError::UnsafeMember);
            }
            let size = entry.size();
            if is_directory {
                if size != 0 {
                    return Err(GacosResultArchiveError::ArchiveCorrupt);
                }
                continue;
            }
            let product = state.register_product(&safe.basename)?;
            match product {
                Some(kind) => state.materialize_product(
                    &mut entry,
                    &safe.basename,
                    kind,
                    size,
                    ReadSource::Archive,
                )?,
                None => state.consume_non_product(&mut entry, size)?,
            }
        }
    }
    // Consume the decoder tail so gzip/bzip2 checksums and truncated streams
    // are observed even when the tar reader reaches its end markers first.
    let mut reader = archive.into_inner();
    let mut tail_bytes = 0_u64;
    drain_reader(&mut reader, control, &mut tail_bytes)?;
    state.finish()
}

fn drain_reader<R: Read, C: ArchiveControl + ?Sized>(
    reader: &mut R,
    control: &C,
    tail_bytes: &mut u64,
) -> Result<(), GacosResultArchiveError> {
    let mut buffer = vec![0_u8; COPY_CHUNK_BYTES];
    loop {
        check_control(control)?;
        let count = reader
            .read(&mut buffer)
            .map_err(|error| map_read_error(error, ReadSource::Archive))?;
        if count == 0 {
            if *tail_bytes < TAR_TRAILER_BLOCK_BYTES {
                return Err(GacosResultArchiveError::Truncated);
            }
            return Ok(());
        }
        if buffer[..count].iter().any(|byte| *byte != 0) {
            return Err(GacosResultArchiveError::ArchiveCorrupt);
        }
        *tail_bytes = tail_bytes
            .checked_add(count as u64)
            .ok_or(GacosResultArchiveError::ExpandedTooLarge)?;
        if *tail_bytes > MAX_ARCHIVE_BYTES {
            return Err(GacosResultArchiveError::ExpandedTooLarge);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::thread;

    use bzip2::write::BzEncoder;
    use flate2::write::GzEncoder;
    use tar::{Builder, Header};
    use zip::write::{SimpleFileOptions, ZipWriter};
    use zip::CompressionMethod;

    static NEXT_DIR: AtomicU64 = AtomicU64::new(1);

    #[derive(Clone)]
    struct TestControl {
        paused: Arc<AtomicBool>,
        cancelled: Arc<AtomicBool>,
    }

    impl TestControl {
        fn new(paused: bool, cancelled: bool) -> Self {
            Self {
                paused: Arc::new(AtomicBool::new(paused)),
                cancelled: Arc::new(AtomicBool::new(cancelled)),
            }
        }
    }

    impl ArchiveControl for TestControl {
        fn is_paused(&self) -> bool {
            self.paused.load(Ordering::SeqCst)
        }

        fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::SeqCst)
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("insar_gacos_archive_{label}_{id}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn product_tiff() -> Vec<u8> {
        b"II*\0native-test-tiff".to_vec()
    }

    fn write_zip(path: &Path, members: &[(&str, &[u8])]) {
        write_zip_with_method(path, members, CompressionMethod::Deflated);
    }

    fn write_zip_with_method(path: &Path, members: &[(&str, &[u8])], method: CompressionMethod) {
        let file = File::create(path).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(method);
        for (name, bytes) in members {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    fn tar_bytes(member_name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        {
            let mut builder = Builder::new(&mut data);
            let mut header = Header::new_gnu();
            header.set_path(member_name).unwrap();
            header.set_size(bytes.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, member_name, bytes)
                .unwrap();
            builder.finish().unwrap();
        }
        data
    }

    fn write_tar(path: &Path, member_name: &str, bytes: &[u8]) {
        fs::write(path, tar_bytes(member_name, bytes)).unwrap();
    }

    fn write_tar_gz(path: &Path, member_name: &str, bytes: &[u8]) {
        let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes(member_name, bytes)).unwrap();
        fs::write(path, encoder.finish().unwrap()).unwrap();
    }

    fn write_tar_bz2(path: &Path, member_name: &str, bytes: &[u8]) {
        let mut encoder = BzEncoder::new(Vec::new(), bzip2::Compression::default());
        encoder.write_all(&tar_bytes(member_name, bytes)).unwrap();
        fs::write(path, encoder.finish().unwrap()).unwrap();
    }

    fn assert_one_tiff(result: Result<Vec<FrozenGacosFile>, GacosResultArchiveError>) {
        let files = result.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "20240101.tif");
        assert_eq!(files[0].size_bytes, product_tiff().len() as u64);
        assert_eq!(files[0].path.read_bytes_for_test(), product_tiff());
    }

    trait ReadBytesForTest {
        fn read_bytes_for_test(&self) -> Vec<u8>;
    }

    impl ReadBytesForTest for Path {
        fn read_bytes_for_test(&self) -> Vec<u8> {
            fs::read(self).unwrap()
        }
    }

    #[test]
    fn materializes_zip_and_ignores_bounded_document() {
        let root = temp_dir("zip");
        let archive = root.join("result.zip");
        let destination = root.join("stage");
        let bytes = product_tiff();
        write_zip(
            &archive,
            &[
                ("docs/readme.txt", &b"ok"[..]),
                ("nested/20240101.tif", bytes.as_slice()),
            ],
        );
        assert_one_tiff(materialize_result_with_control(
            &archive,
            "result.zip",
            &destination,
            &TestControl::new(false, false),
        ));
    }

    #[test]
    fn materializes_tar_gz_and_tar_bz2_and_plain_tar() {
        for (label, suffix, writer) in [
            ("tar", "tar", 0_u8),
            ("gz", "tar.gz", 1_u8),
            ("bz2", "tar.bz2", 2_u8),
        ] {
            let root = temp_dir(label);
            let archive = root.join(format!("result.{suffix}"));
            let destination = root.join("stage");
            let bytes = product_tiff();
            match writer {
                0 => write_tar(&archive, "20240101.tif", bytes.as_slice()),
                1 => write_tar_gz(&archive, "20240101.tif", bytes.as_slice()),
                _ => write_tar_bz2(&archive, "20240101.tif", bytes.as_slice()),
            }
            assert_one_tiff(materialize_result_with_control(
                &archive,
                &format!("result.{suffix}"),
                &destination,
                &TestControl::new(false, false),
            ));
        }
    }

    #[test]
    fn materializes_single_tiff_and_rejects_bad_magic() {
        let root = temp_dir("single");
        let source = root.join("20240101.tif");
        fs::write(&source, product_tiff()).unwrap();
        assert_one_tiff(materialize_result_with_control(
            &source,
            "20240101.tif",
            &root.join("stage"),
            &TestControl::new(false, false),
        ));

        let ztd_tiff = root.join("20240101.ztd.tif");
        fs::write(&ztd_tiff, product_tiff()).unwrap();
        let ztd_result = materialize_result_with_control(
            &ztd_tiff,
            "20240101.ztd.tif",
            &root.join("stage-ztd-tiff"),
            &TestControl::new(false, false),
        )
        .unwrap();
        assert_eq!(ztd_result.len(), 1);
        assert_eq!(ztd_result[0].name, "20240101.ztd.tif");
        assert_eq!(ztd_result[0].path.read_bytes_for_test(), product_tiff());

        let bad = root.join("bad.tif");
        fs::write(&bad, b"not-tiff").unwrap();
        assert_eq!(
            materialize_result_with_control(
                &bad,
                "20240101.tif",
                &root.join("bad-stage"),
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::InvalidTiff)
        );
    }

    #[test]
    fn rejects_unsafe_paths_and_flat_duplicate_products() {
        let root = temp_dir("unsafe");
        let archive = root.join("unsafe.zip");
        let bytes = product_tiff();
        write_zip(&archive, &[("../20240101.tif", bytes.as_slice())]);
        assert_eq!(
            materialize_result_with_control(
                &archive,
                "unsafe.zip",
                &root.join("stage-unsafe"),
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::UnsafeMember)
        );

        write_zip(
            &archive,
            &[
                ("a/20240101.tif", bytes.as_slice()),
                ("b/20240101.tif", bytes.as_slice()),
            ],
        );
        assert_eq!(
            materialize_result_with_control(
                &archive,
                "unsafe.zip",
                &root.join("stage-duplicate"),
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::DuplicateProductName)
        );
    }

    #[test]
    fn rejects_tar_symlink_and_hard_link_entries() {
        for (label, entry_type) in [
            ("symlink", EntryType::symlink()),
            ("hardlink", EntryType::hard_link()),
        ] {
            let root = temp_dir(label);
            let archive = root.join("unsafe.tar");
            let mut bytes = Vec::new();
            {
                let mut builder = Builder::new(&mut bytes);
                let mut header = Header::new_gnu();
                header.set_path("20240101.tif").unwrap();
                header.set_entry_type(entry_type);
                header.set_link_name("target").unwrap();
                header.set_size(0);
                header.set_cksum();
                builder
                    .append_data(&mut header, "20240101.tif", &[][..])
                    .unwrap();
                builder.finish().unwrap();
            }
            fs::write(&archive, bytes).unwrap();
            assert_eq!(
                materialize_result_with_control(
                    &archive,
                    "unsafe.tar",
                    &root.join("stage"),
                    &TestControl::new(false, false),
                ),
                Err(GacosResultArchiveError::UnsafeMember)
            );
        }
    }

    #[test]
    fn rejects_declared_oversize_and_truncated_tar() {
        let root = temp_dir("bounds");
        let oversized = root.join("oversized.tar");
        let mut header = Header::new_gnu();
        header.set_path("20240101.ztd").unwrap();
        header.set_size(MAX_PRODUCT_BYTES + 1);
        header.set_cksum();
        let mut bytes = header.as_bytes().to_vec();
        bytes.extend_from_slice(&[0_u8; 1024]);
        fs::write(&oversized, bytes).unwrap();
        assert_eq!(
            materialize_result_with_control(
                &oversized,
                "oversized.tar",
                &root.join("stage-size"),
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::ProductTooLarge)
        );

        let truncated = root.join("truncated.tar");
        let mut header = Header::new_gnu();
        header.set_path("20240101.tif").unwrap();
        header.set_size(32);
        header.set_cksum();
        let mut bytes = header.as_bytes().to_vec();
        bytes.extend_from_slice(b"II*\0short");
        fs::write(&truncated, bytes).unwrap();
        assert_eq!(
            materialize_result_with_control(
                &truncated,
                "truncated.tar",
                &root.join("stage-truncated"),
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::Truncated)
        );
    }

    #[test]
    fn rejects_zip_crc_failure_and_windows_trailing_space_path() {
        let root = temp_dir("zip-errors");
        let archive = root.join("bad.zip");
        let bytes = product_tiff();
        write_zip_with_method(
            &archive,
            &[("20240101.tif", bytes.as_slice())],
            CompressionMethod::Stored,
        );
        let mut raw = fs::read(&archive).unwrap();
        let position = raw
            .windows(bytes.len())
            .position(|window| window == bytes.as_slice())
            .unwrap();
        raw[position + 5] ^= 0x01;
        fs::write(&archive, raw).unwrap();
        assert_eq!(
            materialize_result_with_control(
                &archive,
                "bad.zip",
                &root.join("stage-crc"),
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::ArchiveCorrupt)
        );
        assert!(!root.join("stage-crc/20240101.tif").exists());
        assert!(root.join("stage-crc/20240101.tif.part").is_file());

        write_zip(&archive, &[("dir/20240101.tif ", bytes.as_slice())]);
        assert_eq!(
            materialize_result_with_control(
                &archive,
                "bad.zip",
                &root.join("stage-path"),
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::UnsafeMember)
        );
    }

    #[test]
    fn auto_detects_fixed_archive_and_rejects_suffix_conflicts() {
        let root = temp_dir("magic");
        let archive = root.join("payload.bin");
        let bytes = product_tiff();
        write_zip(&archive, &[("20240101.tif", bytes.as_slice())]);
        assert_one_tiff(materialize_result_with_control(
            &archive,
            "gacos-result.archive",
            &root.join("stage-auto"),
            &TestControl::new(false, false),
        ));
        assert_eq!(
            materialize_result_with_control(
                &archive,
                "gacos-result.tar",
                &root.join("stage-conflict"),
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::ContainerMagicMismatch)
        );
    }

    #[test]
    fn rejects_zip_member_count_before_constructing_decoder() {
        let root = temp_dir("zip-members");
        let archive = root.join("many.zip");
        let file = File::create(&archive).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for index in 0..=MAX_MEMBERS {
            writer
                .start_file(format!("docs/member-{index:03}.txt"), options)
                .unwrap();
        }
        writer.finish().unwrap();
        assert_eq!(
            materialize_result_with_control(
                &archive,
                "gacos-result.zip",
                &root.join("stage"),
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::TooManyMembers)
        );
    }

    #[test]
    fn pause_waits_for_resume_and_cancel_is_observed_before_writing() {
        let root = temp_dir("control");
        let archive = root.join("result.zip");
        let bytes = product_tiff();
        write_zip(&archive, &[("20240101.tif", bytes.as_slice())]);

        let destination = root.join("paused");
        let control = TestControl::new(true, false);
        let worker_control = control.clone();
        let worker_archive = archive.clone();
        let worker_destination = destination.clone();
        let worker = thread::spawn(move || {
            materialize_result_with_control(
                &worker_archive,
                "result.zip",
                &worker_destination,
                &worker_control,
            )
        });
        thread::sleep(Duration::from_millis(50));
        control.paused.store(false, Ordering::SeqCst);
        assert_one_tiff(worker.join().unwrap());

        let cancelled_destination = root.join("cancelled");
        assert_eq!(
            materialize_result_with_control(
                &archive,
                "result.zip",
                &cancelled_destination,
                &TestControl::new(false, true),
            ),
            Err(GacosResultArchiveError::Cancelled)
        );
        assert!(!cancelled_destination.exists());
    }

    #[test]
    fn resumes_matching_partial_product_and_promotes_atomically() {
        let root = temp_dir("partial-resume");
        let archive = root.join("result.zip");
        let destination = root.join("stage");
        let bytes = product_tiff();
        write_zip(&archive, &[("20240101.tif", bytes.as_slice())]);
        fs::create_dir_all(&destination).unwrap();
        let output = destination.join("20240101.tif");
        let partial = product_partial_path(&output);
        fs::write(&partial, &bytes[..5]).unwrap();

        let files = materialize_result_with_control(
            &archive,
            "result.zip",
            &destination,
            &TestControl::new(false, false),
        )
        .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(fs::read(&output).unwrap(), bytes);
        assert!(!partial.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_and_preserves_mismatched_partial_product() {
        let root = temp_dir("partial-conflict");
        let archive = root.join("result.zip");
        let destination = root.join("stage");
        let bytes = product_tiff();
        write_zip(&archive, &[("20240101.tif", bytes.as_slice())]);
        fs::create_dir_all(&destination).unwrap();
        let output = destination.join("20240101.tif");
        let partial = product_partial_path(&output);
        let mut prefix = bytes[..5].to_vec();
        prefix[0] ^= 0x01;
        fs::write(&partial, &prefix).unwrap();

        assert_eq!(
            materialize_result_with_control(
                &archive,
                "result.zip",
                &destination,
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::DestinationConflict)
        );
        assert_eq!(fs::read(&partial).unwrap(), prefix);
        assert!(!output.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn matching_existing_product_is_reused_but_mismatch_is_rejected() {
        let root = temp_dir("existing");
        let archive = root.join("result.zip");
        let destination = root.join("stage");
        let bytes = product_tiff();
        write_zip(&archive, &[("20240101.tif", bytes.as_slice())]);
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("20240101.tif"), bytes.as_slice()).unwrap();
        assert_one_tiff(materialize_result_with_control(
            &archive,
            "result.zip",
            &destination,
            &TestControl::new(false, false),
        ));
        fs::write(destination.join("20240101.tif"), b"different").unwrap();
        assert_eq!(
            materialize_result_with_control(
                &archive,
                "result.zip",
                &destination,
                &TestControl::new(false, false),
            ),
            Err(GacosResultArchiveError::DestinationConflict)
        );
    }
}
