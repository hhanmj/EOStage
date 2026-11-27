//! Internal D4A1 qualification primitive for safe artifact promotion.

use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[cfg(test)]
thread_local! {
    static PROMOTION_READ_AUDIT: std::cell::Cell<(u64, u64)> = const {
        std::cell::Cell::new((0, 0))
    };
}

#[cfg(all(test, windows))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PromotionReadAudit {
    pub full_scans: u64,
    pub bytes_read: u64,
}

#[cfg(all(test, windows))]
pub(crate) fn reset_promotion_read_audit_for_test() {
    PROMOTION_READ_AUDIT.with(|audit| audit.set((0, 0)));
}

#[cfg(all(test, windows))]
pub(crate) fn promotion_read_audit_for_test() -> PromotionReadAudit {
    PROMOTION_READ_AUDIT.with(|audit| {
        let (full_scans, bytes_read) = audit.get();
        PromotionReadAudit {
            full_scans,
            bytes_read,
        }
    })
}

/// Crate-private capability for a verified, closed, identity-bound artifact.
///
/// Production constructors are available only after an owning verifier has
/// completed. Construction captures a no-follow, single-link file identity;
/// the scientific constructor also verifies the caller's SHA-256 before the
/// promotion primitive rechecks the same identity.
#[doc(hidden)]
pub(crate) struct VerifiedClosedArtifact {
    source: PathBuf,
    source_guard: File,
    source_identity: PromotionSourceIdentity,
    source_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PromotionSourceIdentity {
    platform: &'static str,
    device_id: u64,
    file_id: u64,
    size: u64,
}

impl VerifiedClosedArtifact {
    /// Constructs the typed caller assertion used by D4A1 qualification tests.
    ///
    /// This does not perform verification. The caller is responsible for the
    /// verified-and-closed precondition.
    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) fn for_qualification_test(source: PathBuf) -> Self {
        Self::qualified(source, None, None)
            .expect("qualification source must be a safe regular file")
    }

    /// Constructs the finalization assertion after the caller has completed its
    /// own size and integrity checks and closed the source artifact.
    pub(crate) fn after_finalization_verification(
        source: PathBuf,
        expected_size: u64,
    ) -> Result<Self, PromotionError> {
        let artifact = Self::qualified(source, None, None)?;
        if artifact.source_identity.size != expected_size {
            return Err(PromotionError::SourceUnavailable);
        }
        Ok(artifact)
    }

    pub(crate) fn after_streaming_integrity(
        source: PathBuf,
        expected_size: u64,
        source_sha256: String,
    ) -> Result<Self, PromotionError> {
        if source_sha256.len() != 64
            || !source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(PromotionError::SourceUnavailable);
        }
        let file = open_promotion_source(&source)?;
        let source_identity = promotion_source_identity(&file)?;
        if source_identity.size != expected_size {
            return Err(PromotionError::SourceUnavailable);
        }
        Ok(Self {
            source,
            source_guard: file,
            source_identity,
            source_sha256,
        })
    }

    pub(crate) fn after_scientific_validation(
        source: PathBuf,
        source_sha256: String,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Self, PromotionError> {
        if source_sha256.len() != 64
            || !source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(PromotionError::SourceUnavailable);
        }
        Self::qualified(source, Some(source_sha256), Some(cancelled))
    }

    fn qualified(
        source: PathBuf,
        source_sha256: Option<String>,
        cancelled: Option<&(dyn Fn() -> bool + Sync)>,
    ) -> Result<Self, PromotionError> {
        let file = open_promotion_source(&source)?;
        let source_identity = promotion_source_identity(&file)?;
        let actual_sha256 = sha256_file(&file, cancelled)?;
        if let Some(expected) = &source_sha256 {
            if actual_sha256 != *expected {
                return Err(PromotionError::SourceUnavailable);
            }
        }
        Ok(Self {
            source,
            source_guard: file,
            source_identity,
            source_sha256: source_sha256.unwrap_or(actual_sha256),
        })
    }

    fn source(&self) -> &Path {
        &self.source
    }
}

fn verify_promotion_source(
    artifact: &VerifiedClosedArtifact,
    cancelled: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<File, PromotionError> {
    let file = artifact
        .source_guard
        .try_clone()
        .map_err(|_| PromotionError::SourceUnavailable)?;
    #[cfg(not(windows))]
    file.try_lock()
        .map_err(|_| PromotionError::SourceUnavailable)?;
    // On Windows `open_promotion_source` itself is the exclusive writer lock:
    // the handle denies FILE_SHARE_WRITE while retaining FILE_SHARE_DELETE so
    // ReplaceFileW can move the identity-bound inode. LockFileEx would block
    // ReplaceFileW after it has already moved the destination to its backup.
    if promotion_source_identity(&file)? != artifact.source_identity {
        return Err(PromotionError::SourceUnavailable);
    }
    if sha256_file(&file, cancelled)? != artifact.source_sha256 {
        return Err(PromotionError::SourceUnavailable);
    }
    verify_source_path_binding(artifact)?;
    Ok(file)
}

fn verify_source_path_binding(artifact: &VerifiedClosedArtifact) -> Result<(), PromotionError> {
    let current_path = open_installed_artifact(artifact.source())
        .map_err(|_| PromotionError::SourceUnavailable)?;
    if promotion_source_identity(&current_path)? != artifact.source_identity {
        return Err(PromotionError::SourceUnavailable);
    }
    Ok(())
}

#[cfg(windows)]
fn open_promotion_source(path: &Path) -> Result<File, PromotionError> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000)
        // Permit the owning promotion rename while denying all write sharing.
        .share_mode(0x0000_0001 | 0x0000_0004)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|_| PromotionError::SourceUnavailable)
}

#[cfg(windows)]
fn open_installed_artifact(path: &Path) -> Result<File, PromotionError> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .access_mode(0x8000_0000 | 0x0001_0000)
        // The retained source handle has GENERIC_WRITE, so this verification
        // handle must share writes even though it requests no write access.
        // The retained source handle still denies every new writer.
        .share_mode(0x0000_0001 | 0x0000_0002 | 0x0000_0004)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|_| PromotionError::RecoveryRequired)
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
const PROMOTION_O_NOFOLLOW: i32 = 0x0000_8000;
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
const PROMOTION_O_NOFOLLOW: i32 = 0x0002_0000;
#[cfg(all(
    target_os = "android",
    any(target_arch = "arm", target_arch = "aarch64")
))]
const PROMOTION_O_NOFOLLOW: i32 = 0x0000_8000;
#[cfg(all(
    target_os = "android",
    any(target_arch = "x86", target_arch = "x86_64")
))]
const PROMOTION_O_NOFOLLOW: i32 = 0x0002_0000;
#[cfg(all(target_os = "android", target_arch = "riscv64"))]
const PROMOTION_O_NOFOLLOW: i32 = 0x0040_0000;
#[cfg(target_os = "macos")]
const PROMOTION_O_NOFOLLOW: i32 = 0x0000_0100;
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn open_promotion_source(path: &Path) -> Result<File, PromotionError> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(PROMOTION_O_NOFOLLOW)
        .open(path)
        .map_err(|_| PromotionError::SourceUnavailable)
}

#[cfg(unix)]
fn open_installed_artifact(path: &Path) -> Result<File, PromotionError> {
    open_promotion_source(path)
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn open_promotion_source(_path: &Path) -> Result<File, PromotionError> {
    Err(PromotionError::SourceUnavailable)
}

#[cfg(all(not(windows), not(unix)))]
fn open_installed_artifact(_path: &Path) -> Result<File, PromotionError> {
    Err(PromotionError::RecoveryRequired)
}

#[cfg(all(not(windows), not(unix)))]
fn open_promotion_source(_path: &Path) -> Result<File, PromotionError> {
    Err(PromotionError::SourceUnavailable)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PromotionFileStatus {
    identity: PromotionSourceIdentity,
    links: u64,
}

/// The promotion view of the shared file identity (downloader D3 moved the
/// operating-system calls to crate::file_identity, also used by local reuse).
fn promotion_file_status(file: &File) -> Result<PromotionFileStatus, PromotionError> {
    let status =
        crate::file_identity::file_status(file).map_err(|_| PromotionError::SourceUnavailable)?;
    Ok(PromotionFileStatus {
        identity: PromotionSourceIdentity {
            platform: status.platform,
            device_id: status.volume_id,
            file_id: status.file_id,
            size: status.size,
        },
        links: status.links,
    })
}

fn promotion_bound_identity(file: &File) -> Result<PromotionSourceIdentity, PromotionError> {
    Ok(promotion_file_status(file)?.identity)
}

fn promotion_source_identity(file: &File) -> Result<PromotionSourceIdentity, PromotionError> {
    let status = promotion_file_status(file)?;
    if status.links != 1 {
        return Err(PromotionError::SourceUnavailable);
    }
    Ok(status.identity)
}

fn sha256_file(
    file: &File,
    cancelled: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<String, PromotionError> {
    let mut file = file
        .try_clone()
        .map_err(|_| PromotionError::SourceUnavailable)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| PromotionError::SourceUnavailable)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    #[cfg(test)]
    let mut bytes_read = 0_u64;
    loop {
        if cancelled.is_some_and(|cancelled| cancelled()) {
            return Err(PromotionError::SourceUnavailable);
        }
        let count = file
            .read(&mut buffer)
            .map_err(|_| PromotionError::SourceUnavailable)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        #[cfg(test)]
        {
            bytes_read += count as u64;
        }
    }
    #[cfg(test)]
    PROMOTION_READ_AUDIT.with(|audit| {
        let (scans, bytes) = audit.get();
        audit.set((scans + 1, bytes + bytes_read));
    });
    Ok(format!("{:x}", digest.finalize()))
}

impl fmt::Debug for VerifiedClosedArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedClosedArtifact { redacted: true }")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub(crate) enum PromotionOutcome {
    Installed,
    InstalledWithBackupResidue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub(crate) enum PromotionError {
    SourceUnavailable,
    ParentMismatch,
    DestinationNotRegular,
    BackupCollision,
    BackupSlotsExhausted,
    PromotionFailed,
    ReplacementFailedRecovered,
    RecoveryRequired,
}

impl fmt::Display for PromotionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::SourceUnavailable => "safe promotion source is unavailable",
            Self::ParentMismatch => "safe promotion requires one destination parent",
            Self::DestinationNotRegular => "safe promotion destination is not a regular file",
            Self::BackupCollision => "safe promotion backup name is already occupied",
            Self::BackupSlotsExhausted => "safe promotion backup slots are occupied",
            Self::PromotionFailed => "safe promotion replacement failed",
            Self::ReplacementFailedRecovered => {
                "safe promotion replacement failed and the prior destination was restored"
            }
            Self::RecoveryRequired => "safe promotion requires manual recovery",
        };
        formatter.write_str(message)
    }
}

impl Error for PromotionError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[doc(hidden)]
#[cfg(test)]
pub(crate) enum SafePromotionTestFailpoint {
    FailBeforeInstall,
    MoveDestinationToBackupThenFail,
    MoveDestinationToBackupThenRestoreFails,
    InstallThenFailBackupCleanup,
    WindowsUnableToRemoveReplaced,
    WindowsUnableToMoveReplacement,
    WindowsUnableToMoveReplacement2,
    WindowsOriginalNameFailureWithMovedBackup,
    ReplaceSourceAfterVerification,
    FailPostInstallVerification,
}

/// Promotes a caller-certified verified, closed source artifact.
///
/// The source and destination must be regular siblings in one resolved parent.
/// No source/destination/backup path is returned or formatted by this API.
#[doc(hidden)]
pub(crate) fn promote_verified_closed_artifact(
    artifact: VerifiedClosedArtifact,
    destination: &Path,
    cancelled: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<PromotionOutcome, PromotionError> {
    let mut hooks = NoopPromotionHooks;
    promote_with_hooks(&artifact, destination, cancelled, &mut hooks)
}

/// Runs the same primitive through a private operation seam for qualification
/// cases that a normal filesystem cannot reproduce deterministically.
#[doc(hidden)]
#[cfg(test)]
pub(crate) fn promote_verified_closed_with_failpoint_for_test(
    artifact: VerifiedClosedArtifact,
    destination: &Path,
    failpoint: SafePromotionTestFailpoint,
) -> Result<PromotionOutcome, PromotionError> {
    let mut hooks = FailpointPromotionHooks { failpoint };
    promote_with_hooks(&artifact, destination, None, &mut hooks)
}

/// Returns the deterministic opaque backup sibling used only by focused tests.
#[doc(hidden)]
#[cfg(test)]
pub(crate) fn promotion_backup_path_for_qualification_test(destination: &Path) -> PathBuf {
    backup_paths(destination)
        .map(|(backup, _)| backup)
        .unwrap_or_else(|| PathBuf::from(".insar-promotion-unavailable"))
}

fn promote_with_hooks(
    artifact: &VerifiedClosedArtifact,
    destination: &Path,
    cancelled: Option<&(dyn Fn() -> bool + Sync)>,
    hooks: &mut impl PromotionHooks,
) -> Result<PromotionOutcome, PromotionError> {
    if !promotion_platform_is_audited() {
        return Err(PromotionError::PromotionFailed);
    }
    let source = artifact.source();
    let source_guard = verify_promotion_source(artifact, cancelled)?;
    ensure_same_parent(source, destination)?;
    hooks.after_source_verification(source)?;
    verify_source_path_binding(artifact)?;

    let destination_kind = entry_kind(destination)?;
    if destination_kind == EntryKind::Other {
        // An unrestored Linux backup is a second link to the destination;
        // that is an unfinished promotion, not a foreign file.
        #[cfg(not(windows))]
        if has_unresolved_backup_residue(destination).unwrap_or(true) {
            return Err(PromotionError::RecoveryRequired);
        }
        return Err(PromotionError::DestinationNotRegular);
    }
    if destination_kind == EntryKind::Missing && has_unresolved_backup_residue(destination)? {
        return Err(PromotionError::RecoveryRequired);
    }
    if hooks.fail_before_destination_change() {
        return Err(PromotionError::PromotionFailed);
    }

    let (slot, mut reservation) = reserve_next_promotion_slot(destination)?;
    let backup = if destination_kind == EntryKind::Regular {
        let retained =
            retain_single_link_regular(destination, PromotionError::DestinationNotRegular)?;
        Some(create_bound_backup(retained, destination, &slot.backup)?)
    } else {
        None
    };

    if hooks.fail_after_backup() {
        let recovered =
            !hooks.fail_restore() && recover_before_install(destination, backup.as_ref(), &slot);
        let _ = reservation.release();
        return if recovered {
            Err(PromotionError::ReplacementFailedRecovered)
        } else {
            Err(PromotionError::RecoveryRequired)
        };
    }

    if install_bound_source(
        artifact,
        &source_guard,
        destination,
        &slot.staging,
        backup.as_ref(),
        cancelled,
    )
    .is_err()
    {
        let recovered =
            !hooks.fail_restore() && recover_before_install(destination, backup.as_ref(), &slot);
        let _ = reservation.release();
        return if backup.is_some() && recovered {
            Err(PromotionError::ReplacementFailedRecovered)
        } else if backup.is_none() && recovered {
            Err(PromotionError::PromotionFailed)
        } else {
            Err(PromotionError::RecoveryRequired)
        };
    }

    let installed_verified = !hooks.fail_post_install_verification()
        && verify_installed_artifact(artifact, &source_guard, destination, cancelled).is_ok();
    if !installed_verified {
        let recovered = !hooks.fail_restore()
            && recover_after_install(artifact, &source_guard, destination, backup.as_ref(), &slot);
        let _ = reservation.release();
        return if recovered && backup.is_some() {
            Err(PromotionError::ReplacementFailedRecovered)
        } else if recovered {
            Err(PromotionError::PromotionFailed)
        } else {
            Err(PromotionError::RecoveryRequired)
        };
    }

    if !finalize_source_after_success(artifact, &source_guard, destination, &slot) {
        let recovered = !hooks.fail_restore()
            && recover_after_install(artifact, &source_guard, destination, backup.as_ref(), &slot);
        let _ = reservation.release();
        return if recovered && backup.is_some() {
            Err(PromotionError::ReplacementFailedRecovered)
        } else {
            Err(PromotionError::RecoveryRequired)
        };
    }

    if !is_installed_post_state(artifact, &source_guard, destination, &slot) {
        let recovered = !hooks.fail_restore()
            && recover_after_install(artifact, &source_guard, destination, backup.as_ref(), &slot);
        let _ = reservation.release();
        return if recovered && backup.is_some() {
            Err(PromotionError::ReplacementFailedRecovered)
        } else {
            Err(PromotionError::RecoveryRequired)
        };
    }

    let backup_removed = match backup {
        Some(backup) if hooks.fail_backup_cleanup() => {
            drop(backup);
            false
        }
        Some(backup) => cleanup_bound_backup(backup, &slot),
        None => true,
    };
    let reservation_removed = reservation.release();
    if backup_removed
        && reservation_removed
        && !source_cleanup_residue(artifact, &source_guard, &slot)
    {
        Ok(PromotionOutcome::Installed)
    } else {
        Ok(PromotionOutcome::InstalledWithBackupResidue)
    }
}

fn verify_installed_artifact(
    artifact: &VerifiedClosedArtifact,
    source_guard: &File,
    destination: &Path,
    cancelled: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<(), PromotionError> {
    let installed = open_installed_artifact(destination)?;
    if promotion_bound_identity(source_guard)? != artifact.source_identity {
        return Err(PromotionError::RecoveryRequired);
    }
    let installed_status = promotion_file_status(&installed)?;
    #[cfg(windows)]
    if installed_status.identity != artifact.source_identity || installed_status.links != 1 {
        return Err(PromotionError::RecoveryRequired);
    }
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    if installed_status.identity.size != artifact.source_identity.size
        || installed_status.links != 1
    {
        return Err(PromotionError::RecoveryRequired);
    }
    #[cfg(all(
        not(windows),
        not(any(target_os = "linux", target_os = "android", target_os = "macos"))
    ))]
    return Err(PromotionError::RecoveryRequired);
    if sha256_file(&installed, cancelled)? != artifact.source_sha256 {
        return Err(PromotionError::RecoveryRequired);
    }
    Ok(())
}

#[cfg(windows)]
fn is_installed_post_state(
    artifact: &VerifiedClosedArtifact,
    _source_guard: &File,
    destination: &Path,
    _slot: &PromotionSlotPaths,
) -> bool {
    matches!(entry_kind(artifact.source()), Ok(EntryKind::Missing))
        && path_matches_single_link(destination, &artifact.source_identity)
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn live_source_identity_matches(
    source_guard: &File,
    artifact: &VerifiedClosedArtifact,
) -> Option<PromotionSourceIdentity> {
    let live = promotion_source_identity(source_guard).ok()?;
    if live.platform == artifact.source_identity.platform
        && live.device_id == artifact.source_identity.device_id
        && live.size == artifact.source_identity.size
    {
        Some(live)
    } else {
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn is_installed_post_state(
    artifact: &VerifiedClosedArtifact,
    source_guard: &File,
    destination: &Path,
    slot: &PromotionSlotPaths,
) -> bool {
    let Some(live_identity) = live_source_identity_matches(source_guard, artifact) else {
        return false;
    };
    matches!(entry_kind(artifact.source()), Ok(EntryKind::Missing))
        && path_matches_single_link_with_sha(destination, artifact, None)
        && path_matches_identity(&slot.restore, &live_identity)
}

#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn is_installed_post_state(
    _artifact: &VerifiedClosedArtifact,
    _source_guard: &File,
    _destination: &Path,
    _slot: &PromotionSlotPaths,
) -> bool {
    false
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum EntryKind {
    Missing,
    Regular,
    Other,
}

fn entry_kind(path: &Path) -> Result<EntryKind, PromotionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Ok(EntryKind::Other)
        }
        Ok(_) => {
            match open_promotion_source(path).and_then(|file| promotion_source_identity(&file)) {
                Ok(_) => Ok(EntryKind::Regular),
                Err(_) => Ok(EntryKind::Other),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(EntryKind::Missing),
        Err(_) => Err(PromotionError::PromotionFailed),
    }
}

fn ensure_same_parent(source: &Path, destination: &Path) -> Result<(), PromotionError> {
    let source_parent = canonical_parent(source)?;
    let destination_parent = canonical_parent(destination)?;
    if source_parent == destination_parent {
        Ok(())
    } else {
        Err(PromotionError::ParentMismatch)
    }
}

fn canonical_parent(path: &Path) -> Result<PathBuf, PromotionError> {
    let parent = path.parent().ok_or(PromotionError::ParentMismatch)?;
    fs::canonicalize(parent).map_err(|_| PromotionError::ParentMismatch)
}

const BACKUP_SLOT_COUNT: usize = 32;

#[cfg(test)]
fn backup_paths(destination: &Path) -> Option<(PathBuf, PathBuf)> {
    let slot = promotion_slot_paths(destination, 0)?;
    Some((slot.backup, slot.reservation))
}

struct PromotionSlotPaths {
    backup: PathBuf,
    reservation: PathBuf,
    staging: PathBuf,
    restore: PathBuf,
}

fn promotion_slot_paths(destination: &Path, slot: usize) -> Option<PromotionSlotPaths> {
    let parent = destination.parent()?;
    let file_name = destination.file_name()?;
    let digest = Sha256::digest(file_name.as_encoded_bytes());
    let suffix = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let slot_suffix = if slot == 0 {
        String::new()
    } else {
        format!("-{slot}")
    };
    let prefix = format!(".insar-promotion-{suffix}{slot_suffix}");
    Some(PromotionSlotPaths {
        backup: parent.join(format!("{prefix}.backup")),
        reservation: parent.join(format!("{prefix}.reserve")),
        staging: parent.join(format!("{prefix}.staging")),
        restore: parent.join(format!("{prefix}.restore")),
    })
}

/// Classifies a promotion slot entry. On Linux a retained backup and the
/// displaced destination left in `staging` are one inode linked twice, so a
/// regular file with several links is still `Regular` residue there.
/// Windows backups are moved, never linked, so it keeps `entry_kind`.
#[cfg(windows)]
fn residue_kind(path: &Path) -> Result<EntryKind, PromotionError> {
    entry_kind(path)
}

#[cfg(not(windows))]
fn residue_kind(path: &Path) -> Result<EntryKind, PromotionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Ok(EntryKind::Other)
        }
        Ok(_) => match open_promotion_source(path).and_then(|file| promotion_file_status(&file)) {
            Ok(_) => Ok(EntryKind::Regular),
            Err(_) => Ok(EntryKind::Other),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(EntryKind::Missing),
        Err(_) => Err(PromotionError::PromotionFailed),
    }
}

fn has_unresolved_backup_residue(destination: &Path) -> Result<bool, PromotionError> {
    for slot in 0..BACKUP_SLOT_COUNT {
        let paths =
            promotion_slot_paths(destination, slot).ok_or(PromotionError::ParentMismatch)?;
        for path in [&paths.backup, &paths.staging, &paths.restore] {
            match residue_kind(path) {
                Ok(EntryKind::Missing) => {}
                Ok(EntryKind::Regular) => return Ok(true),
                Ok(EntryKind::Other) | Err(_) => return Err(PromotionError::RecoveryRequired),
            }
        }
    }
    Ok(false)
}

fn reserve_next_promotion_slot(
    destination: &Path,
) -> Result<(PromotionSlotPaths, BackupReservation), PromotionError> {
    for slot in 0..BACKUP_SLOT_COUNT {
        let paths =
            promotion_slot_paths(destination, slot).ok_or(PromotionError::ParentMismatch)?;
        let kinds = [
            residue_kind(&paths.backup),
            residue_kind(&paths.reservation),
            residue_kind(&paths.staging),
            residue_kind(&paths.restore),
        ];
        if kinds
            .iter()
            .any(|kind| matches!(kind, Ok(EntryKind::Other)))
        {
            return Err(PromotionError::PromotionFailed);
        }
        if kinds.iter().any(Result::is_err) {
            return Err(PromotionError::PromotionFailed);
        }
        if kinds
            .iter()
            .all(|kind| matches!(kind, Ok(EntryKind::Missing)))
        {
            match BackupReservation::acquire(&paths.reservation) {
                Ok(reservation) => return Ok((paths, reservation)),
                Err(PromotionError::BackupCollision) => continue,
                Err(error) => return Err(error),
            }
        }
    }

    Err(PromotionError::BackupSlotsExhausted)
}

// ReplaceFileW needs the backup path itself to remain absent. This distinct
// sibling reservation serializes the deterministic backup name without
// overwriting an existing file.
struct BackupReservation {
    path: PathBuf,
    guard: Option<File>,
    identity: PromotionSourceIdentity,
    active: bool,
}

impl BackupReservation {
    fn acquire(path: &Path) -> Result<Self, PromotionError> {
        match acquire_reservation_file(path) {
            Ok(file) => {
                let identity = promotion_source_identity(&file)
                    .map_err(|_| PromotionError::PromotionFailed)?;
                Ok(Self {
                    path: path.to_path_buf(),
                    guard: Some(file),
                    identity,
                    active: true,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(PromotionError::BackupCollision)
            }
            Err(_) => Err(PromotionError::PromotionFailed),
        }
    }

    fn release(&mut self) -> bool {
        if !self.active {
            return true;
        }
        let released = release_reservation(self);
        self.active = false;
        released
    }
}

impl Drop for BackupReservation {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

#[cfg(windows)]
fn acquire_reservation_file(path: &Path) -> Result<File, std::io::Error> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000)
        .share_mode(0x0000_0001 | 0x0000_0002 | 0x0000_0004)
        .custom_flags(0x0020_0000)
        .open(path)
}

#[cfg(not(windows))]
fn acquire_reservation_file(path: &Path) -> Result<File, std::io::Error> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(windows)]
fn release_reservation(reservation: &mut BackupReservation) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
    };

    let Some(guard) = reservation.guard.take() else {
        return false;
    };
    if promotion_bound_identity(&guard).ok().as_ref() != Some(&reservation.identity) {
        return false;
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let deleted = unsafe {
        SetFileInformationByHandle(
            guard.as_raw_handle() as _,
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            u32::try_from(std::mem::size_of::<FILE_DISPOSITION_INFO>()).unwrap_or(0),
        )
    } != 0;
    drop(guard);
    deleted && matches!(entry_kind(&reservation.path), Ok(EntryKind::Missing))
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn release_reservation(reservation: &mut BackupReservation) -> bool {
    let Some(guard) = reservation.guard.take() else {
        return false;
    };
    if promotion_bound_identity(&guard).ok().as_ref() != Some(&reservation.identity) {
        return false;
    }
    drop(guard);
    unlink_verified_residue(&reservation.path, &reservation.identity)
}

/// Removes one private promotion-slot entry only while its name still refers
/// to the inode `identity` describes (owner decision 2026-11-01).
///
/// POSIX has no unlink-by-retained-handle, so this checks the name with
/// `fstatat(AT_SYMLINK_NOFOLLOW)` and removes it with `unlinkat` through the
/// same directory descriptor. The window between the two calls can only be
/// used by a process that can already write the application's directory,
/// that is the same user, and it could only make this remove another file
/// at a hidden `.insar-promotion-*` slot name. Returns whether the name is
/// absent afterwards.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn unlink_verified_residue(path: &Path, identity: &PromotionSourceIdentity) -> bool {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;

    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let Ok(name) = CString::new(name.as_bytes()) else {
        return false;
    };
    let Ok(directory) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(parent)
    else {
        return false;
    };
    let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `directory` is an open directory descriptor, `name` a live C
    // string and `status` a writable stat buffer for this synchronous call.
    let found = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            status.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if found != 0 {
        return std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound;
    }
    // SAFETY: fstatat succeeded, so it initialised the whole buffer.
    let status = unsafe { status.assume_init() };
    // dev_t and ino_t widths differ between targets.
    #[allow(clippy::unnecessary_cast)]
    let same_inode = status.st_mode & libc::S_IFMT == libc::S_IFREG
        && status.st_dev as u64 == identity.device_id
        && status.st_ino as u64 == identity.file_id;
    if !same_inode {
        return false;
    }
    // SAFETY: same live descriptor and name; flag 0 removes a non-directory.
    unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) == 0 }
}

/// The identity of whatever `path` names now, for residue the protocol
/// created without retaining a handle (the staged copy).
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn current_identity(path: &Path) -> Option<PromotionSourceIdentity> {
    open_installed_artifact(path)
        .and_then(|file| promotion_bound_identity(&file))
        .ok()
}

#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn release_reservation(reservation: &mut BackupReservation) -> bool {
    drop(reservation.guard.take());
    false
}

#[cfg(windows)]
fn rename_windows_handle(
    source_guard: &File,
    destination: &Path,
    replace_existing: bool,
) -> Result<(), PromotionError> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileRenameInfo, SetFileInformationByHandle, FILE_RENAME_INFO,
    };

    // A long destination is renamed to through its extended-length form; the
    // final component is not resolved (L3 gate 2026-10-19, see long_path).
    let destination = crate::long_path::native_path(destination);
    let destination = destination.as_os_str().encode_wide().collect::<Vec<_>>();
    if destination.is_empty() || destination.contains(&0) {
        return Err(PromotionError::PromotionFailed);
    }
    source_guard
        .sync_all()
        .map_err(|_| PromotionError::PromotionFailed)?;
    let extra_bytes = destination
        .len()
        .checked_add(1)
        .ok_or(PromotionError::PromotionFailed)?
        .checked_mul(std::mem::size_of::<u16>())
        .ok_or(PromotionError::PromotionFailed)?;
    let buffer_size = std::mem::size_of::<FILE_RENAME_INFO>()
        .checked_add(extra_bytes)
        .and_then(|size| u32::try_from(size).ok())
        .ok_or(PromotionError::PromotionFailed)?;
    let mut buffer = vec![0_u8; buffer_size as usize];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        // SAFETY: `buffer` is sized for FILE_RENAME_INFO plus the complete
        // UTF-16 flexible array and remains alive through the synchronous call.
        (*info).Anonymous.ReplaceIfExists = replace_existing;
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength =
            u32::try_from(destination.len() * 2).map_err(|_| PromotionError::PromotionFailed)?;
        std::ptr::copy_nonoverlapping(
            destination.as_ptr(),
            (*info).FileName.as_mut_ptr(),
            destination.len(),
        );
    }
    let renamed = unsafe {
        // SAFETY: the source handle is live and owns DELETE access; the input
        // buffer is initialized as documented above.
        SetFileInformationByHandle(
            source_guard.as_raw_handle() as _,
            FileRenameInfo,
            buffer.as_ptr().cast(),
            buffer_size,
        )
    };
    if renamed == 0 {
        return Err(PromotionError::PromotionFailed);
    }
    source_guard
        .sync_all()
        .map_err(|_| PromotionError::PromotionFailed)
}

struct RetainedEntry {
    guard: File,
    identity: PromotionSourceIdentity,
}

fn retain_single_link_regular(
    path: &Path,
    error: PromotionError,
) -> Result<RetainedEntry, PromotionError> {
    let guard = open_promotion_source(path).map_err(|_| error)?;
    let identity = promotion_source_identity(&guard).map_err(|_| error)?;
    Ok(RetainedEntry { guard, identity })
}

struct BoundBackup {
    path: PathBuf,
    guard: File,
    identity: PromotionSourceIdentity,
    #[cfg(target_os = "macos")]
    moved: bool,
}

#[cfg(all(test, target_os = "macos"))]
std::thread_local! {
    static FORCE_MOVED_BACKUP_FOR_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(all(test, target_os = "macos"))]
struct ForceMovedBackupGuard(bool);

#[cfg(all(test, target_os = "macos"))]
impl ForceMovedBackupGuard {
    fn enter() -> Self {
        let previous = FORCE_MOVED_BACKUP_FOR_TEST.with(|flag| flag.replace(true));
        Self(previous)
    }
}

#[cfg(all(test, target_os = "macos"))]
impl Drop for ForceMovedBackupGuard {
    fn drop(&mut self) {
        FORCE_MOVED_BACKUP_FOR_TEST.with(|flag| flag.set(self.0));
    }
}

#[cfg(target_os = "macos")]
fn parent_supports_hard_links(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let Some(parent) = path.parent() else {
        return true;
    };
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let Ok(parent_cstr) = CString::new(parent.as_os_str().as_bytes()) else {
        return true;
    };
    // SAFETY: `parent_cstr` is a live NUL-terminated path buffer for the synchronous syscall.
    let link_max = unsafe { libc::pathconf(parent_cstr.as_ptr(), libc::_PC_LINK_MAX) };
    link_max != 1
}

#[cfg(target_os = "macos")]
fn use_hard_link_backup(destination: &Path) -> bool {
    #[cfg(test)]
    if FORCE_MOVED_BACKUP_FOR_TEST.with(|flag| flag.get()) {
        return false;
    }
    parent_supports_hard_links(destination)
}

#[cfg(target_os = "macos")]
fn live_backup_identity_matches(backup: &BoundBackup) -> Option<PromotionSourceIdentity> {
    let live = promotion_source_identity(&backup.guard).ok()?;
    if live.platform == backup.identity.platform
        && live.device_id == backup.identity.device_id
        && live.size == backup.identity.size
    {
        Some(live)
    } else {
        None
    }
}

#[cfg(windows)]
fn create_bound_backup(
    retained: RetainedEntry,
    _destination: &Path,
    backup: &Path,
) -> Result<BoundBackup, PromotionError> {
    rename_windows_handle(&retained.guard, backup, false)?;
    if !path_matches_identity(backup, &retained.identity) {
        return Err(PromotionError::RecoveryRequired);
    }
    Ok(BoundBackup {
        path: backup.to_path_buf(),
        guard: retained.guard,
        identity: retained.identity,
    })
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn create_bound_backup(
    retained: RetainedEntry,
    destination: &Path,
    backup: &Path,
) -> Result<BoundBackup, PromotionError> {
    #[cfg(target_os = "macos")]
    if !use_hard_link_backup(destination) {
        if entry_kind(backup)? != EntryKind::Missing {
            return Err(PromotionError::RecoveryRequired);
        }
        rename_unix_noreplace(destination, backup)?;
        // macOS exFAT updates directory-entry `st_ino` on `rename()`; re-read the
        // retained descriptor and verify immutable volume/platform/size invariants.
        let live_identity = promotion_source_identity(&retained.guard)
            .map_err(|_| PromotionError::RecoveryRequired)?;
        if live_identity.platform != retained.identity.platform
            || live_identity.device_id != retained.identity.device_id
            || live_identity.size != retained.identity.size
            || !path_matches_single_link(backup, &live_identity)
            || !matches!(entry_kind(destination), Ok(EntryKind::Missing))
        {
            return Err(PromotionError::RecoveryRequired);
        }
        return Ok(BoundBackup {
            path: backup.to_path_buf(),
            guard: retained.guard,
            identity: live_identity,
            moved: true,
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = destination;
    link_retained_file(&retained.guard, backup)?;
    let status = open_promotion_source(backup)
        .and_then(|file| promotion_file_status(&file))
        .map_err(|_| PromotionError::RecoveryRequired)?;
    if status.identity != retained.identity || status.links != 2 {
        return Err(PromotionError::RecoveryRequired);
    }
    Ok(BoundBackup {
        path: backup.to_path_buf(),
        guard: retained.guard,
        identity: retained.identity,
        #[cfg(target_os = "macos")]
        moved: false,
    })
}

#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn create_bound_backup(
    _retained: RetainedEntry,
    _destination: &Path,
    _backup: &Path,
) -> Result<BoundBackup, PromotionError> {
    Err(PromotionError::PromotionFailed)
}

#[cfg(windows)]
fn install_bound_source(
    _artifact: &VerifiedClosedArtifact,
    source_guard: &File,
    destination: &Path,
    _staging: &Path,
    _backup: Option<&BoundBackup>,
    _cancelled: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<(), PromotionError> {
    rename_windows_handle(source_guard, destination, false)
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn install_bound_source(
    artifact: &VerifiedClosedArtifact,
    source_guard: &File,
    destination: &Path,
    staging: &Path,
    backup: Option<&BoundBackup>,
    cancelled: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<(), PromotionError> {
    copy_retained_artifact(artifact, source_guard, staging, cancelled)?;
    let status = open_promotion_source(staging)
        .and_then(|file| promotion_file_status(&file))
        .map_err(|_| PromotionError::RecoveryRequired)?;
    if status.identity.size != artifact.source_identity.size || status.links != 1 {
        return Err(PromotionError::RecoveryRequired);
    }
    let use_exchange = match backup {
        #[cfg(target_os = "macos")]
        Some(b) if b.moved => None,
        other => other,
    };
    match use_exchange {
        Some(backup) => {
            rename_unix_exchange(staging, destination)?;
            if path_matches_single_link_with_sha(destination, artifact, cancelled)
                && path_matches_identity(staging, &backup.identity)
            {
                Ok(())
            } else {
                let _ = rename_unix_exchange(staging, destination);
                Err(PromotionError::RecoveryRequired)
            }
        }
        None => rename_unix_noreplace(staging, destination).and_then(|()| {
            if path_matches_single_link_with_sha(destination, artifact, cancelled)
                && matches!(entry_kind(staging), Ok(EntryKind::Missing))
            {
                Ok(())
            } else {
                Err(PromotionError::RecoveryRequired)
            }
        }),
    }
}

#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn install_bound_source(
    _artifact: &VerifiedClosedArtifact,
    _source_guard: &File,
    _destination: &Path,
    _staging: &Path,
    _backup: Option<&BoundBackup>,
    _cancelled: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<(), PromotionError> {
    Err(PromotionError::PromotionFailed)
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn link_retained_file(file: &File, target: &Path) -> Result<(), PromotionError> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::raw::{c_char, c_int};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn linkat(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: c_int,
        ) -> c_int;
    }

    if entry_kind(target)? != EntryKind::Missing {
        return Err(PromotionError::RecoveryRequired);
    }
    let expected_identity = promotion_bound_identity(file)?;
    let target_cstr =
        CString::new(target.as_os_str().as_bytes()).map_err(|_| PromotionError::PromotionFailed)?;
    for root in ["/proc/self/fd", "/dev/fd"] {
        let source = CString::new(format!("{root}/{}", file.as_raw_fd()))
            .map_err(|_| PromotionError::PromotionFailed)?;
        let linked = unsafe {
            linkat(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target_cstr.as_ptr(),
                libc::AT_SYMLINK_FOLLOW,
            )
        };
        if linked == 0 {
            if path_matches_identity(target, &expected_identity) {
                return Ok(());
            }
            return Err(PromotionError::RecoveryRequired);
        }
        if entry_kind(target)? != EntryKind::Missing {
            return Err(PromotionError::RecoveryRequired);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut path_buf = vec![0 as c_char; libc::PATH_MAX as usize];
        // SAFETY: `file` is a valid open file descriptor and `path_buf` is
        // sized to `PATH_MAX` bytes as required by Darwin `F_GETPATH`.
        let got_path =
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, path_buf.as_mut_ptr()) };
        if got_path == 0 {
            // SAFETY: `path_buf` was NUL-terminated by `F_GETPATH` and `target_cstr`
            // is a live NUL-terminated path; flag 0 refuses to follow symlinks.
            let linked = unsafe {
                linkat(
                    libc::AT_FDCWD,
                    path_buf.as_ptr(),
                    libc::AT_FDCWD,
                    target_cstr.as_ptr(),
                    0,
                )
            };
            if linked == 0 {
                if path_matches_identity(target, &expected_identity) {
                    return Ok(());
                }
                if let Some(actual) = current_identity(target) {
                    let _ = unlink_verified_residue(target, &actual);
                }
                return Err(PromotionError::RecoveryRequired);
            }
            if entry_kind(target)? != EntryKind::Missing {
                return Err(PromotionError::RecoveryRequired);
            }
        }
    }
    Err(PromotionError::PromotionFailed)
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn copy_retained_artifact(
    artifact: &VerifiedClosedArtifact,
    source_guard: &File,
    staging: &Path,
    cancelled: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<(), PromotionError> {
    use std::io::Write;

    if entry_kind(staging)? != EntryKind::Missing {
        return Err(PromotionError::RecoveryRequired);
    }
    let mut source = source_guard
        .try_clone()
        .map_err(|_| PromotionError::SourceUnavailable)?;
    source
        .seek(SeekFrom::Start(0))
        .map_err(|_| PromotionError::SourceUnavailable)?;
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staging)
        .map_err(|_| PromotionError::PromotionFailed)?;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        if cancelled.is_some_and(|cancelled| cancelled()) {
            return Err(PromotionError::PromotionFailed);
        }
        let count = source
            .read(&mut buffer)
            .map_err(|_| PromotionError::SourceUnavailable)?;
        if count == 0 {
            break;
        }
        destination
            .write_all(&buffer[..count])
            .map_err(|_| PromotionError::PromotionFailed)?;
    }
    destination
        .sync_all()
        .map_err(|_| PromotionError::PromotionFailed)?;
    drop(destination);
    if !path_matches_single_link_with_sha(staging, artifact, cancelled) {
        return Err(PromotionError::RecoveryRequired);
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_unix_noreplace(source: &Path, destination: &Path) -> Result<(), PromotionError> {
    rename_unix(source, destination, 1)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_unix_exchange(left: &Path, right: &Path) -> Result<(), PromotionError> {
    rename_unix(left, right, 2)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_unix(source: &Path, destination: &Path, flags: u32) -> Result<(), PromotionError> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn renameat2(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: u32,
        ) -> c_int;
    }

    const AT_FDCWD: c_int = -100;
    let source =
        CString::new(source.as_os_str().as_bytes()).map_err(|_| PromotionError::PromotionFailed)?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| PromotionError::PromotionFailed)?;
    let renamed = unsafe {
        // SAFETY: both C strings are live absolute/relative path buffers for
        // the duration of this synchronous syscall wrapper. Callers use only
        // RENAME_NOREPLACE (1) or RENAME_EXCHANGE (2).
        renameat2(
            AT_FDCWD,
            source.as_ptr(),
            AT_FDCWD,
            destination.as_ptr(),
            flags,
        )
    };
    if renamed == 0 {
        Ok(())
    } else {
        Err(PromotionError::PromotionFailed)
    }
}

#[cfg(target_os = "macos")]
fn rename_unix_noreplace(source: &Path, destination: &Path) -> Result<(), PromotionError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // Darwin RENAME_EXCL = 0x0000_0004
    match rename_unix_raw(source, destination, 0x0000_0004) {
        Ok(()) => Ok(()),
        // exFAT / FAT32 filesystems on macOS return ENOTSUP (errno 45) for `renamex_np(..., RENAME_EXCL)`.
        // Fail closed if `destination` already exists (including symlinks/case-folded entries), then use `libc::rename`.
        Err(Some(libc::ENOTSUP | libc::ENOSYS | libc::EINVAL)) => {
            if !matches!(entry_kind(destination)?, EntryKind::Missing) {
                return Err(PromotionError::PromotionFailed);
            }
            let src = CString::new(source.as_os_str().as_bytes())
                .map_err(|_| PromotionError::PromotionFailed)?;
            let dst = CString::new(destination.as_os_str().as_bytes())
                .map_err(|_| PromotionError::PromotionFailed)?;
            let renamed = unsafe { libc::rename(src.as_ptr(), dst.as_ptr()) };
            if renamed == 0 {
                Ok(())
            } else {
                Err(PromotionError::PromotionFailed)
            }
        }
        Err(_) => Err(PromotionError::PromotionFailed),
    }
}

#[cfg(target_os = "macos")]
fn rename_unix_exchange(left: &Path, right: &Path) -> Result<(), PromotionError> {
    // Darwin RENAME_SWAP = 0x0000_0002
    rename_unix(left, right, 0x0000_0002)
}

#[cfg(target_os = "macos")]
fn rename_unix(source: &Path, destination: &Path, flags: u32) -> Result<(), PromotionError> {
    rename_unix_raw(source, destination, flags).map_err(|_| PromotionError::PromotionFailed)
}

#[cfg(target_os = "macos")]
fn rename_unix_raw(source: &Path, destination: &Path, flags: u32) -> Result<(), Option<i32>> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: u32) -> c_int;
    }

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| None)?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| None)?;
    let renamed = unsafe {
        // SAFETY: both C strings are live path buffers for the duration of this
        // synchronous Darwin syscall wrapper. Callers pass only RENAME_EXCL (4)
        // or RENAME_SWAP (2).
        renamex_np(source.as_ptr(), destination.as_ptr(), flags)
    };
    if renamed == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().raw_os_error())
    }
}

fn path_matches_identity(path: &Path, identity: &PromotionSourceIdentity) -> bool {
    open_installed_artifact(path)
        .and_then(|file| promotion_bound_identity(&file))
        .map(|actual| actual == *identity)
        .unwrap_or(false)
}

fn path_matches_single_link(path: &Path, identity: &PromotionSourceIdentity) -> bool {
    open_installed_artifact(path)
        .and_then(|file| promotion_source_identity(&file))
        .map(|actual| actual == *identity)
        .unwrap_or(false)
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn path_matches_single_link_with_sha(
    path: &Path,
    artifact: &VerifiedClosedArtifact,
    cancelled: Option<&(dyn Fn() -> bool + Sync)>,
) -> bool {
    open_installed_artifact(path)
        .and_then(|file| {
            let status = promotion_file_status(&file)?;
            if status.links != 1 || status.identity.size != artifact.source_identity.size {
                return Err(PromotionError::RecoveryRequired);
            }
            sha256_file(&file, cancelled)
        })
        .map(|sha256| sha256 == artifact.source_sha256)
        .unwrap_or(false)
}

#[cfg(windows)]
fn recover_before_install(
    destination: &Path,
    backup: Option<&BoundBackup>,
    _slot: &PromotionSlotPaths,
) -> bool {
    match backup {
        Some(backup) => {
            rename_windows_handle(&backup.guard, destination, false).is_ok()
                && path_matches_single_link(destination, &backup.identity)
                && matches!(entry_kind(&backup.path), Ok(EntryKind::Missing))
        }
        None => true,
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn recover_before_install(
    destination: &Path,
    backup: Option<&BoundBackup>,
    slot: &PromotionSlotPaths,
) -> bool {
    let Some(backup) = backup else {
        return true;
    };
    #[cfg(target_os = "macos")]
    if backup.moved {
        let staging_clear = match entry_kind(&slot.staging) {
            Ok(EntryKind::Missing) => true,
            Ok(EntryKind::Regular) => remove_staged_copy(slot),
            Ok(EntryKind::Other) | Err(_) => false,
        };
        return staging_clear
            && rename_unix_noreplace(&backup.path, destination).is_ok()
            && live_backup_identity_matches(backup)
                .is_some_and(|live| path_matches_single_link(destination, &live))
            && matches!(entry_kind(&backup.path), Ok(EntryKind::Missing));
    }
    let restored = if path_matches_identity(destination, &backup.identity) {
        path_matches_identity(&backup.path, &backup.identity)
    } else {
        restore_unix_backup(destination, backup, slot)
    };
    // A recovered destination is single-linked again, as on Windows where
    // the backup is moved back rather than linked.
    restored
        && unlink_verified_residue(&backup.path, &backup.identity)
        && path_matches_single_link(destination, &backup.identity)
}

#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn recover_before_install(
    _destination: &Path,
    _backup: Option<&BoundBackup>,
    _slot: &PromotionSlotPaths,
) -> bool {
    false
}

#[cfg(windows)]
fn recover_after_install(
    artifact: &VerifiedClosedArtifact,
    source_guard: &File,
    destination: &Path,
    backup: Option<&BoundBackup>,
    slot: &PromotionSlotPaths,
) -> bool {
    let source_restored = rename_windows_handle(source_guard, artifact.source(), false).is_ok()
        && path_matches_single_link(artifact.source(), &artifact.source_identity);
    if !source_restored {
        let _ = rename_windows_handle(source_guard, &slot.staging, false);
    }
    let destination_restored = match backup {
        Some(backup) => {
            rename_windows_handle(&backup.guard, destination, false).is_ok()
                && path_matches_single_link(destination, &backup.identity)
        }
        None => matches!(entry_kind(destination), Ok(EntryKind::Missing)),
    };
    source_restored && destination_restored
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn recover_after_install(
    artifact: &VerifiedClosedArtifact,
    source_guard: &File,
    destination: &Path,
    backup: Option<&BoundBackup>,
    slot: &PromotionSlotPaths,
) -> bool {
    #[cfg(target_os = "macos")]
    let moved_mode = backup.is_some_and(|b| b.moved) || !use_hard_link_backup(destination);
    #[cfg(not(target_os = "macos"))]
    let moved_mode = false;

    let source_restored = if moved_mode {
        if matches!(entry_kind(artifact.source()), Ok(EntryKind::Missing)) {
            rename_unix_noreplace(&slot.restore, artifact.source()).is_ok()
                && live_source_identity_matches(source_guard, artifact)
                    .is_some_and(|live| path_matches_identity(artifact.source(), &live))
                && matches!(entry_kind(&slot.restore), Ok(EntryKind::Missing))
        } else {
            live_source_identity_matches(source_guard, artifact)
                .is_some_and(|live| path_matches_identity(artifact.source(), &live))
                && matches!(entry_kind(&slot.restore), Ok(EntryKind::Missing))
        }
    } else {
        let restored = if matches!(entry_kind(artifact.source()), Ok(EntryKind::Missing)) {
            link_retained_file(source_guard, artifact.source()).is_ok()
                && path_matches_identity(artifact.source(), &artifact.source_identity)
        } else {
            path_matches_identity(artifact.source(), &artifact.source_identity)
        };
        // A source re-linked after finalization also still has its `restore` name.
        restored && unlink_verified_residue(&slot.restore, &artifact.source_identity)
    };
    if !source_restored {
        return false;
    }
    match backup {
        #[cfg(target_os = "macos")]
        Some(backup) if backup.moved => {
            rename_unix_noreplace(destination, &slot.staging).is_ok()
                && matches!(entry_kind(destination), Ok(EntryKind::Missing))
                && path_matches_single_link_with_sha(&slot.staging, artifact, None)
                && remove_staged_copy(slot)
                && rename_unix_noreplace(&backup.path, destination).is_ok()
                && live_backup_identity_matches(backup)
                    .is_some_and(|live| path_matches_single_link(destination, &live))
                && matches!(entry_kind(&backup.path), Ok(EntryKind::Missing))
        }
        Some(backup) => {
            rename_unix_exchange(&slot.staging, destination).is_ok()
                && path_matches_identity(destination, &backup.identity)
                && path_matches_identity(&backup.path, &backup.identity)
                && path_matches_single_link_with_sha(&slot.staging, artifact, None)
                && remove_staged_copy(slot)
                && unlink_verified_residue(&backup.path, &backup.identity)
                && path_matches_single_link(destination, &backup.identity)
        }
        None => {
            rename_unix_noreplace(destination, &slot.staging).is_ok()
                && matches!(entry_kind(destination), Ok(EntryKind::Missing))
                && path_matches_single_link_with_sha(&slot.staging, artifact, None)
                && remove_staged_copy(slot)
        }
    }
}

/// The uninstalled copy in `staging` was verified by content just before;
/// it is removed by the identity it has now.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn remove_staged_copy(slot: &PromotionSlotPaths) -> bool {
    current_identity(&slot.staging)
        .is_some_and(|identity| unlink_verified_residue(&slot.staging, &identity))
}

#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn recover_after_install(
    _artifact: &VerifiedClosedArtifact,
    _source_guard: &File,
    _destination: &Path,
    _backup: Option<&BoundBackup>,
    _slot: &PromotionSlotPaths,
) -> bool {
    false
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn restore_unix_backup(
    destination: &Path,
    backup: &BoundBackup,
    slot: &PromotionSlotPaths,
) -> bool {
    if !matches!(entry_kind(destination), Ok(EntryKind::Missing))
        || !matches!(entry_kind(&slot.restore), Ok(EntryKind::Missing))
        || link_retained_file(&backup.guard, &slot.restore).is_err()
        || !path_matches_identity(&slot.restore, &backup.identity)
        || rename_unix_noreplace(&slot.restore, destination).is_err()
        || !path_matches_identity(destination, &backup.identity)
    {
        return false;
    }
    path_matches_identity(&backup.path, &backup.identity)
        && path_matches_identity(destination, &backup.identity)
}

#[cfg(windows)]
fn finalize_source_after_success(
    _artifact: &VerifiedClosedArtifact,
    _source_guard: &File,
    _destination: &Path,
    _slot: &PromotionSlotPaths,
) -> bool {
    true
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn finalize_source_after_success(
    artifact: &VerifiedClosedArtifact,
    source_guard: &File,
    _destination: &Path,
    slot: &PromotionSlotPaths,
) -> bool {
    // POSIX has no compare-and-unlink-by-open-handle primitive. Move the
    // source name into the private promotion slot and retain it as cleanup
    // residue instead of performing an identity-check/pathname-unlink pair.
    // Checking the live `source_guard` handle preserves binding even on
    // macOS exFAT where `rename()` updates the directory-entry `st_ino`.
    matches!(entry_kind(&slot.restore), Ok(EntryKind::Missing))
        && rename_unix_noreplace(artifact.source(), &slot.restore).is_ok()
        && live_source_identity_matches(source_guard, artifact)
            .is_some_and(|live_identity| path_matches_identity(&slot.restore, &live_identity))
        && matches!(entry_kind(artifact.source()), Ok(EntryKind::Missing))
}

#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn finalize_source_after_success(
    _artifact: &VerifiedClosedArtifact,
    _source_guard: &File,
    _destination: &Path,
    _slot: &PromotionSlotPaths,
) -> bool {
    false
}

#[cfg(windows)]
fn cleanup_bound_backup(backup: BoundBackup, _slot: &PromotionSlotPaths) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
    };

    if !path_matches_single_link(&backup.path, &backup.identity) {
        return false;
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let deleted = unsafe {
        SetFileInformationByHandle(
            backup.guard.as_raw_handle() as _,
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            u32::try_from(std::mem::size_of::<FILE_DISPOSITION_INFO>()).unwrap_or(0),
        )
    } != 0;
    drop(backup.guard);
    deleted && matches!(entry_kind(&backup.path), Ok(EntryKind::Missing))
}

/// After a verified install the backup link and the displaced destination
/// left in `staging` are the same old inode; both names go.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn cleanup_bound_backup(backup: BoundBackup, slot: &PromotionSlotPaths) -> bool {
    #[cfg(target_os = "macos")]
    if backup.moved {
        drop(backup.guard);
        return matches!(entry_kind(&slot.staging), Ok(EntryKind::Missing))
            && unlink_verified_residue(&backup.path, &backup.identity);
    }
    drop(backup.guard);
    let staging_removed = unlink_verified_residue(&slot.staging, &backup.identity);
    unlink_verified_residue(&backup.path, &backup.identity) && staging_removed
}

#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn cleanup_bound_backup(_backup: BoundBackup, _slot: &PromotionSlotPaths) -> bool {
    false
}

/// Whether source residue remains after a verified install.
#[cfg(windows)]
fn source_cleanup_residue(
    _artifact: &VerifiedClosedArtifact,
    _source_guard: &File,
    _slot: &PromotionSlotPaths,
) -> bool {
    false
}

/// `finalize_source_after_success` moved the source name into the slot's
/// `restore`; once the install is verified it is removed by identity.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn source_cleanup_residue(
    artifact: &VerifiedClosedArtifact,
    source_guard: &File,
    slot: &PromotionSlotPaths,
) -> bool {
    live_source_identity_matches(source_guard, artifact)
        .is_none_or(|identity| !unlink_verified_residue(&slot.restore, &identity))
}

#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn source_cleanup_residue(
    _artifact: &VerifiedClosedArtifact,
    _source_guard: &File,
    _slot: &PromotionSlotPaths,
) -> bool {
    false
}

#[cfg(any(
    windows,
    target_os = "linux",
    target_os = "android",
    target_os = "macos"
))]
fn promotion_platform_is_audited() -> bool {
    true
}

#[cfg(not(any(
    windows,
    target_os = "linux",
    target_os = "android",
    target_os = "macos"
)))]
fn promotion_platform_is_audited() -> bool {
    false
}

trait PromotionHooks {
    fn after_source_verification(&mut self, _source: &Path) -> Result<(), PromotionError> {
        Ok(())
    }
    fn fail_before_destination_change(&self) -> bool {
        false
    }
    fn fail_after_backup(&self) -> bool {
        false
    }
    fn fail_restore(&self) -> bool {
        false
    }
    fn fail_post_install_verification(&self) -> bool {
        false
    }
    fn fail_backup_cleanup(&self) -> bool {
        false
    }
}

struct NoopPromotionHooks;

impl PromotionHooks for NoopPromotionHooks {}

#[cfg(test)]
struct FailpointPromotionHooks {
    failpoint: SafePromotionTestFailpoint,
}

#[cfg(test)]
impl PromotionHooks for FailpointPromotionHooks {
    fn after_source_verification(&mut self, source: &Path) -> Result<(), PromotionError> {
        if self.failpoint == SafePromotionTestFailpoint::ReplaceSourceAfterVerification {
            let displaced = source.with_extension("verified-displaced");
            fs::rename(source, displaced).map_err(|_| PromotionError::RecoveryRequired)?;
            fs::write(source, b"attacker-replacement")
                .map_err(|_| PromotionError::RecoveryRequired)?;
        }
        Ok(())
    }

    fn fail_before_destination_change(&self) -> bool {
        matches!(
            self.failpoint,
            SafePromotionTestFailpoint::FailBeforeInstall
                | SafePromotionTestFailpoint::WindowsUnableToRemoveReplaced
                | SafePromotionTestFailpoint::WindowsUnableToMoveReplacement
        )
    }

    fn fail_after_backup(&self) -> bool {
        matches!(
            self.failpoint,
            SafePromotionTestFailpoint::MoveDestinationToBackupThenFail
                | SafePromotionTestFailpoint::MoveDestinationToBackupThenRestoreFails
                | SafePromotionTestFailpoint::WindowsUnableToMoveReplacement2
                | SafePromotionTestFailpoint::WindowsOriginalNameFailureWithMovedBackup
        )
    }

    fn fail_restore(&self) -> bool {
        matches!(
            self.failpoint,
            SafePromotionTestFailpoint::MoveDestinationToBackupThenRestoreFails
                | SafePromotionTestFailpoint::WindowsOriginalNameFailureWithMovedBackup
        )
    }

    fn fail_post_install_verification(&self) -> bool {
        self.failpoint == SafePromotionTestFailpoint::FailPostInstallVerification
    }

    fn fail_backup_cleanup(&self) -> bool {
        self.failpoint == SafePromotionTestFailpoint::InstallThenFailBackupCleanup
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_noreplace_tests {
    use super::*;

    #[test]
    fn rename_unix_noreplace_refuses_existing_destination_and_promotes_missing_target() {
        let dir = std::env::temp_dir().join(format!(
            "insar-safe-promo-noreplace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("source.part");
        let dst = dir.join("dest.zip");
        fs::write(&src, b"new-payload").unwrap();
        fs::write(&dst, b"existing-payload").unwrap();

        assert!(rename_unix_noreplace(&src, &dst).is_err());
        assert_eq!(fs::read(&src).unwrap(), b"new-payload");
        assert_eq!(fs::read(&dst).unwrap(), b"existing-payload");

        let fresh = dir.join("fresh.zip");
        assert!(rename_unix_noreplace(&src, &fresh).is_ok());
        assert_eq!(fs::read(&fresh).unwrap(), b"new-payload");
        assert!(!src.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "insar-safe-promo-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn no_residue(dir: &Path) -> bool {
        fs::read_dir(dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".insar-promotion-")
        })
    }

    #[test]
    fn moved_backup_overwrites_existing_destination_repeatedly_with_no_residue() {
        let _guard = ForceMovedBackupGuard::enter();
        let dir = temp_dir("moved-overwrite");
        let dst = dir.join("planning-prefix-v2.json");
        fs::write(&dst, b"initial").unwrap();

        for round in 1..=3 {
            let src = dir.join(format!("planning-prefix-v2.json.next.{round}"));
            let payload = format!("updated-checkpoint-{round}");
            fs::write(&src, payload.as_bytes()).unwrap();
            let outcome = promote_verified_closed_artifact(
                VerifiedClosedArtifact::for_qualification_test(src.clone()),
                &dst,
                None,
            )
            .unwrap();
            assert_eq!(outcome, PromotionOutcome::Installed);
            assert_eq!(fs::read(&dst).unwrap(), payload.as_bytes());
            assert!(!src.exists());
            assert!(no_residue(&dir));
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn moved_backup_failpoints_recover_and_fail_closed_cleanly() {
        let _guard = ForceMovedBackupGuard::enter();

        // 1. MoveDestinationToBackupThenFail restores original destination with zero residue.
        let dir = temp_dir("moved-fail-after-backup");
        let src = dir.join("source.json");
        let dst = dir.join("dest.json");
        fs::write(&src, b"replacement").unwrap();
        fs::write(&dst, b"original").unwrap();
        let err = promote_verified_closed_with_failpoint_for_test(
            VerifiedClosedArtifact::for_qualification_test(src.clone()),
            &dst,
            SafePromotionTestFailpoint::MoveDestinationToBackupThenFail,
        )
        .unwrap_err();
        assert_eq!(err, PromotionError::ReplacementFailedRecovered);
        assert_eq!(fs::read(&dst).unwrap(), b"original");
        assert_eq!(fs::read(&src).unwrap(), b"replacement");
        assert!(no_residue(&dir));
        let _ = fs::remove_dir_all(&dir);

        // 2. FailPostInstallVerification restores both original destination and source.
        let dir = temp_dir("moved-fail-post-verify");
        let src = dir.join("source.json");
        let dst = dir.join("dest.json");
        fs::write(&src, b"replacement").unwrap();
        fs::write(&dst, b"original").unwrap();
        let err = promote_verified_closed_with_failpoint_for_test(
            VerifiedClosedArtifact::for_qualification_test(src.clone()),
            &dst,
            SafePromotionTestFailpoint::FailPostInstallVerification,
        )
        .unwrap_err();
        assert_eq!(err, PromotionError::ReplacementFailedRecovered);
        assert_eq!(fs::read(&dst).unwrap(), b"original");
        assert_eq!(fs::read(&src).unwrap(), b"replacement");
        assert!(no_residue(&dir));
        let _ = fs::remove_dir_all(&dir);

        // 3. InstallThenFailBackupCleanup installs replacement while reporting residue.
        let dir = temp_dir("moved-fail-cleanup");
        let src = dir.join("source.json");
        let dst = dir.join("dest.json");
        fs::write(&src, b"replacement").unwrap();
        fs::write(&dst, b"original").unwrap();
        let outcome = promote_verified_closed_with_failpoint_for_test(
            VerifiedClosedArtifact::for_qualification_test(src.clone()),
            &dst,
            SafePromotionTestFailpoint::InstallThenFailBackupCleanup,
        )
        .unwrap();
        assert_eq!(outcome, PromotionOutcome::InstalledWithBackupResidue);
        assert_eq!(fs::read(&dst).unwrap(), b"replacement");
        assert!(!src.exists());
        let _ = fs::remove_dir_all(&dir);

        // 4. MoveDestinationToBackupThenRestoreFails leaves backup residue and blocks follow-up promotion.
        let dir = temp_dir("moved-fail-restore");
        let src = dir.join("source.json");
        let dst = dir.join("dest.json");
        fs::write(&src, b"replacement").unwrap();
        fs::write(&dst, b"original").unwrap();
        let err = promote_verified_closed_with_failpoint_for_test(
            VerifiedClosedArtifact::for_qualification_test(src.clone()),
            &dst,
            SafePromotionTestFailpoint::MoveDestinationToBackupThenRestoreFails,
        )
        .unwrap_err();
        assert_eq!(err, PromotionError::RecoveryRequired);
        assert!(!dst.exists());
        let retry_src = dir.join("retry.json");
        fs::write(&retry_src, b"retry").unwrap();
        let retry_err = promote_verified_closed_artifact(
            VerifiedClosedArtifact::for_qualification_test(retry_src),
            &dst,
            None,
        )
        .unwrap_err();
        assert_eq!(retry_err, PromotionError::RecoveryRequired);
        let _ = fs::remove_dir_all(&dir);
    }
}
