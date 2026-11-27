use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::safe_promotion::{
    promote_verified_closed_artifact, promote_verified_closed_with_failpoint_for_test,
    promotion_backup_path_for_qualification_test, PromotionError, PromotionOutcome,
    SafePromotionTestFailpoint, VerifiedClosedArtifact,
};
#[cfg(windows)]
use crate::safe_promotion::{promotion_read_audit_for_test, reset_promotion_read_audit_for_test};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("insar-safe-promotion-{label}-{id}"));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_file(path: &Path, contents: &[u8]) {
    fs::write(path, contents).unwrap();
}

fn qualified_artifact(path: &Path) -> VerifiedClosedArtifact {
    VerifiedClosedArtifact::for_qualification_test(path.to_path_buf())
}

fn assert_no_reservation(root: &TempRoot) {
    assert!(fs::read_dir(root.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".reserve")));
}

/// The destination name after a backup whose restore failed. Windows moves
/// the destination to the backup, so the name is gone; Linux backs up with a
/// second link, so the name keeps the old contents.
fn assert_destination_after_unrestored_backup(destination: &Path, old: &[u8]) {
    if cfg!(windows) {
        assert!(!destination.exists());
    } else {
        assert_eq!(fs::read(destination).unwrap(), old);
    }
}

fn qualification_backup_slot_path(destination: &Path, slot: usize) -> PathBuf {
    let slot_zero = promotion_backup_path_for_qualification_test(destination);
    if slot == 0 {
        return slot_zero;
    }

    let base = slot_zero
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".backup"))
        .unwrap();
    destination
        .parent()
        .unwrap()
        .join(format!("{base}-{slot}.backup"))
}

fn qualification_reservation_slot_path(destination: &Path, slot: usize) -> PathBuf {
    qualification_backup_slot_path(destination, slot).with_extension("reserve")
}

fn promote(source: &Path, destination: &Path) -> Result<PromotionOutcome, PromotionError> {
    promote_verified_closed_artifact(qualified_artifact(source), destination, None)
}

fn promote_with_failpoint(
    source: &Path,
    destination: &Path,
    failpoint: SafePromotionTestFailpoint,
) -> Result<PromotionOutcome, PromotionError> {
    promote_verified_closed_with_failpoint_for_test(
        qualified_artifact(source),
        destination,
        failpoint,
    )
}

/// Linux cleans its slot by verified identity since the owner decision of
/// 2026-11-01; before that it kept every backup and reported residue.
fn normal_install_outcome() -> PromotionOutcome {
    PromotionOutcome::Installed
}

#[cfg(windows)]
#[test]
fn streaming_integrity_attestation_removes_one_promotion_source_scan() {
    let contents = b"stream-verified-artifact";

    let legacy_root = TempRoot::new("legacy-read-audit");
    let legacy_source = legacy_root.path().join("legacy.part");
    let legacy_destination = legacy_root.path().join("legacy.bin");
    write_file(&legacy_source, contents);
    reset_promotion_read_audit_for_test();
    let legacy = VerifiedClosedArtifact::after_finalization_verification(
        legacy_source,
        contents.len() as u64,
    )
    .unwrap();
    assert_eq!(
        promote_verified_closed_artifact(legacy, &legacy_destination, None),
        Ok(PromotionOutcome::Installed)
    );
    assert_eq!(
        promotion_read_audit_for_test(),
        crate::safe_promotion::PromotionReadAudit {
            full_scans: 3,
            bytes_read: 3 * contents.len() as u64,
        }
    );

    let streaming_root = TempRoot::new("streaming-read-audit");
    let streaming_source = streaming_root.path().join("streaming.part");
    let streaming_destination = streaming_root.path().join("streaming.bin");
    write_file(&streaming_source, contents);
    let source_sha256 = format!("{:x}", Sha256::digest(contents));
    reset_promotion_read_audit_for_test();
    let streaming = VerifiedClosedArtifact::after_streaming_integrity(
        streaming_source,
        contents.len() as u64,
        source_sha256,
    )
    .unwrap();
    assert_eq!(
        promote_verified_closed_artifact(streaming, &streaming_destination, None),
        Ok(PromotionOutcome::Installed)
    );
    assert_eq!(
        promotion_read_audit_for_test(),
        crate::safe_promotion::PromotionReadAudit {
            full_scans: 2,
            bytes_read: 2 * contents.len() as u64,
        }
    );
}

#[test]
fn destination_absent_installs_without_a_backup() {
    let root = TempRoot::new("absent");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");

    let outcome = promote(&source, &destination);
    assert_eq!(
        outcome,
        Ok(normal_install_outcome()),
        "source_exists={} destination_exists={} backup_exists={}",
        source.exists(),
        destination.exists(),
        promotion_backup_path_for_qualification_test(&destination).exists(),
    );
    assert_eq!(fs::read(&destination).unwrap(), b"verified-new");
    assert!(!source.exists());
    assert!(!promotion_backup_path_for_qualification_test(&destination).exists());
    assert_no_reservation(&root);
}

#[test]
fn scientific_token_binds_sha_and_promotes_only_matching_identity() {
    let root = TempRoot::new("scientific-token");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-science");
    let digest = format!("{:x}", Sha256::digest(b"verified-science"));
    let artifact =
        VerifiedClosedArtifact::after_scientific_validation(source.clone(), digest, &|| false)
            .unwrap();
    assert_eq!(
        promote_verified_closed_artifact(artifact, &destination, None),
        Ok(normal_install_outcome())
    );
    assert_eq!(fs::read(destination).unwrap(), b"verified-science");

    let mismatch = root.path().join("mismatch.bin");
    write_file(&mismatch, b"different");
    assert_eq!(
        VerifiedClosedArtifact::after_scientific_validation(mismatch, "0".repeat(64), &|| false,)
            .unwrap_err(),
        PromotionError::SourceUnavailable
    );

    let cancelled = root.path().join("cancelled.bin");
    write_file(&cancelled, &[3_u8; 128 * 1024]);
    let digest = format!("{:x}", Sha256::digest([3_u8; 128 * 1024]));
    assert_eq!(
        VerifiedClosedArtifact::after_scientific_validation(cancelled, digest, &|| true)
            .unwrap_err(),
        PromotionError::SourceUnavailable
    );

    let rewritten = root.path().join("rewritten.bin");
    let rewritten_destination = root.path().join("rewritten-final.bin");
    write_file(&rewritten, b"before-identity");
    let digest = format!("{:x}", Sha256::digest(b"before-identity"));
    let artifact =
        VerifiedClosedArtifact::after_scientific_validation(rewritten.clone(), digest, &|| false)
            .unwrap();
    let rewrite_result = fs::write(&rewritten, b"after--identity");

    #[cfg(windows)]
    {
        assert!(rewrite_result.is_err());
        assert_eq!(fs::read(&rewritten).unwrap(), b"before-identity");
        assert_eq!(
            promote_verified_closed_artifact(artifact, &rewritten_destination, None),
            Ok(PromotionOutcome::Installed)
        );
        assert_eq!(fs::read(rewritten_destination).unwrap(), b"before-identity");
    }

    #[cfg(not(windows))]
    {
        rewrite_result.unwrap();
        assert_eq!(
            promote_verified_closed_artifact(artifact, &rewritten_destination, None),
            Err(PromotionError::SourceUnavailable)
        );
        assert!(!rewritten_destination.exists());
        assert_eq!(fs::read(rewritten).unwrap(), b"after--identity");
    }
}

#[test]
fn source_path_replacement_after_final_hash_never_reports_installed() {
    let root = TempRoot::new("post-hash-source-replacement");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    let artifact = qualified_artifact(&source);

    assert_eq!(
        promote_verified_closed_with_failpoint_for_test(
            artifact,
            &destination,
            SafePromotionTestFailpoint::ReplaceSourceAfterVerification,
        ),
        Err(PromotionError::SourceUnavailable)
    );
    assert!(!destination.exists());
    assert_eq!(
        fs::read(source.with_extension("verified-displaced")).unwrap(),
        b"verified-new"
    );
}

#[test]
fn source_replacement_after_verification_preserves_existing_destination_and_artifact() {
    let root = TempRoot::new("post-hash-existing-destination");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");

    assert_eq!(
        promote_with_failpoint(
            &source,
            &destination,
            SafePromotionTestFailpoint::ReplaceSourceAfterVerification,
        ),
        Err(PromotionError::SourceUnavailable)
    );
    assert_eq!(fs::read(&destination).unwrap(), b"old-valid");
    assert_eq!(fs::read(&source).unwrap(), b"attacker-replacement");
    assert_eq!(
        fs::read(source.with_extension("verified-displaced")).unwrap(),
        b"verified-new"
    );
    assert!(!promotion_backup_path_for_qualification_test(&destination).exists());
    assert_no_reservation(&root);
}

#[test]
fn post_install_verification_failure_restores_existing_destination_and_source() {
    let root = TempRoot::new("post-install-verification-existing");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");

    assert_eq!(
        promote_with_failpoint(
            &source,
            &destination,
            SafePromotionTestFailpoint::FailPostInstallVerification,
        ),
        Err(PromotionError::ReplacementFailedRecovered)
    );
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    assert_eq!(fs::read(&destination).unwrap(), b"old-valid");
    assert!(!promotion_backup_path_for_qualification_test(&destination).exists());
    assert_no_reservation(&root);
}

#[test]
fn post_install_verification_failure_restores_absent_destination_source() {
    let root = TempRoot::new("post-install-verification-absent");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");

    assert_eq!(
        promote_with_failpoint(
            &source,
            &destination,
            SafePromotionTestFailpoint::FailPostInstallVerification,
        ),
        Err(PromotionError::PromotionFailed)
    );
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    assert!(!destination.exists());
    assert_no_reservation(&root);
}

#[test]
fn destination_present_replaces_the_old_file() {
    let root = TempRoot::new("present");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");

    let outcome = promote(&source, &destination);
    assert_eq!(
        outcome,
        Ok(normal_install_outcome()),
        "source_exists={} destination_exists={} backup_exists={}",
        source.exists(),
        destination.exists(),
        promotion_backup_path_for_qualification_test(&destination).exists(),
    );
    assert_eq!(fs::read(&destination).unwrap(), b"verified-new");
    assert!(!source.exists());
    assert!(!promotion_backup_path_for_qualification_test(&destination).exists());
    assert_no_reservation(&root);
}

#[test]
fn missing_source_preserves_the_existing_destination() {
    let root = TempRoot::new("missing-source");
    let source = root.path().join("missing.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    let artifact = qualified_artifact(&source);
    fs::remove_file(&source).unwrap();
    write_file(&destination, b"old-valid");

    assert_eq!(
        promote_verified_closed_artifact(artifact, &destination, None),
        Err(PromotionError::SourceUnavailable)
    );
    assert_eq!(fs::read(&destination).unwrap(), b"old-valid");
}

#[test]
fn parent_mismatch_and_destination_directory_are_rejected_without_destruction() {
    let root = TempRoot::new("preflight");
    let source_parent = root.path().join("source-parent");
    let destination_parent = root.path().join("destination-parent");
    fs::create_dir_all(&source_parent).unwrap();
    fs::create_dir_all(&destination_parent).unwrap();
    let source = source_parent.join("source.bin");
    let mismatch_destination = destination_parent.join("destination.bin");
    write_file(&source, b"verified-new");

    assert_eq!(
        promote(&source, &mismatch_destination),
        Err(PromotionError::ParentMismatch)
    );
    assert!(source.exists());
    assert!(!mismatch_destination.exists());

    let destination_directory = source_parent.join("destination-directory");
    fs::create_dir(&destination_directory).unwrap();
    assert_eq!(
        promote(&source, &destination_directory),
        Err(PromotionError::DestinationNotRegular)
    );
    assert!(destination_directory.is_dir());
    assert!(source.exists());
}

#[test]
fn symlink_destination_is_rejected_without_touching_source_or_target() {
    let root = TempRoot::new("symlink-destination");
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&target, b"old-valid");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &destination).unwrap();
    #[cfg(windows)]
    if let Err(error) = std::os::windows::fs::symlink_file(&target, &destination) {
        assert_eq!(error.raw_os_error(), Some(1314));
        assert!(include_str!("safe_promotion.rs").contains("fs::symlink_metadata(path)"));
        return;
    }

    assert_eq!(
        promote(&source, &destination),
        Err(PromotionError::DestinationNotRegular)
    );
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    assert_eq!(fs::read(&target).unwrap(), b"old-valid");
    assert!(fs::symlink_metadata(&destination)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn hardlinked_destination_is_rejected_without_touching_either_link() {
    let root = TempRoot::new("hardlink-destination");
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&target, b"old-valid");
    fs::hard_link(&target, &destination).unwrap();

    assert_eq!(
        promote(&source, &destination),
        Err(PromotionError::DestinationNotRegular)
    );
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    assert_eq!(fs::read(&target).unwrap(), b"old-valid");
    assert_eq!(fs::read(&destination).unwrap(), b"old-valid");
}

#[test]
fn preexisting_backup_residue_is_preserved_and_next_slot_is_used() {
    let root = TempRoot::new("backup-residue-next-slot");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");
    let backup = promotion_backup_path_for_qualification_test(&destination);
    write_file(&backup, b"existing-backup");

    assert_eq!(promote(&source, &destination), Ok(normal_install_outcome()));
    assert_eq!(fs::read(&destination).unwrap(), b"verified-new");
    assert!(!source.exists());
    assert_eq!(fs::read(&backup).unwrap(), b"existing-backup");
    assert_no_reservation(&root);
}

#[test]
fn failure_before_replacement_preserves_the_old_destination() {
    let root = TempRoot::new("pre-replacement-failure");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");

    assert_eq!(
        promote_with_failpoint(
            &source,
            &destination,
            SafePromotionTestFailpoint::FailBeforeInstall,
        ),
        Err(PromotionError::PromotionFailed)
    );
    assert_eq!(fs::read(&destination).unwrap(), b"old-valid");
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    assert_no_reservation(&root);
}

#[test]
fn simulated_windows_partial_failure_restores_the_old_destination() {
    let root = TempRoot::new("recovered-replacement-failure");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");

    assert_eq!(
        promote_with_failpoint(
            &source,
            &destination,
            SafePromotionTestFailpoint::MoveDestinationToBackupThenFail,
        ),
        Err(PromotionError::ReplacementFailedRecovered)
    );
    assert_eq!(fs::read(&destination).unwrap(), b"old-valid");
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    assert!(!promotion_backup_path_for_qualification_test(&destination).exists());
    assert_no_reservation(&root);
}

#[test]
fn failed_restore_returns_recovery_required_and_preserves_survivors() {
    let root = TempRoot::new("recovery-required");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");

    assert_eq!(
        promote_with_failpoint(
            &source,
            &destination,
            SafePromotionTestFailpoint::MoveDestinationToBackupThenRestoreFails,
        ),
        Err(PromotionError::RecoveryRequired)
    );
    let backup = promotion_backup_path_for_qualification_test(&destination);
    assert_destination_after_unrestored_backup(&destination, b"old-valid");
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    assert_eq!(fs::read(&backup).unwrap(), b"old-valid");
    assert_no_reservation(&root);
}

#[test]
fn unresolved_recovery_backup_blocks_a_later_destination_absent_promotion() {
    let root = TempRoot::new("recovery-residue-blocks-absent");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");

    assert_eq!(
        promote_with_failpoint(
            &source,
            &destination,
            SafePromotionTestFailpoint::MoveDestinationToBackupThenRestoreFails,
        ),
        Err(PromotionError::RecoveryRequired)
    );

    assert_eq!(
        promote(&source, &destination),
        Err(PromotionError::RecoveryRequired)
    );
    let backup = promotion_backup_path_for_qualification_test(&destination);
    assert_destination_after_unrestored_backup(&destination, b"old-valid");
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    assert_eq!(fs::read(&backup).unwrap(), b"old-valid");
}

/// Checkpoint files are replaced many times per transfer; every verified
/// replacement must clean its own slot instead of filling all 32.
#[test]
fn repeated_replacement_leaves_no_promotion_residue() {
    let root = TempRoot::new("repeated-replacement");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&destination, b"generation-0");
    for generation in 1..=40 {
        let body = format!("generation-{generation}");
        write_file(&source, body.as_bytes());
        assert_eq!(
            promote(&source, &destination),
            Ok(PromotionOutcome::Installed),
            "generation {generation}"
        );
        assert_eq!(fs::read(&destination).unwrap(), body.as_bytes());
        assert!(!source.exists());
    }
    let residue: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".insar-promotion"))
        .collect();
    assert!(residue.is_empty(), "{residue:?}");
}

#[test]
fn backup_cleanup_failure_is_success_with_residue() {
    let root = TempRoot::new("backup-residue");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");

    assert_eq!(
        promote_with_failpoint(
            &source,
            &destination,
            SafePromotionTestFailpoint::InstallThenFailBackupCleanup,
        ),
        Ok(PromotionOutcome::InstalledWithBackupResidue)
    );
    let backup = promotion_backup_path_for_qualification_test(&destination);
    assert_eq!(fs::read(&destination).unwrap(), b"verified-new");
    assert!(!source.exists());
    assert_eq!(fs::read(&backup).unwrap(), b"old-valid");
    assert_no_reservation(&root);
}

#[test]
fn cleanup_residue_uses_a_different_free_slot_on_the_next_promotion() {
    let root = TempRoot::new("cleanup-residue-next-slot");
    let first_source = root.path().join("first-source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&first_source, b"verified-first");
    write_file(&destination, b"old-valid");

    assert_eq!(
        promote_with_failpoint(
            &first_source,
            &destination,
            SafePromotionTestFailpoint::InstallThenFailBackupCleanup,
        ),
        Ok(PromotionOutcome::InstalledWithBackupResidue)
    );

    let second_source = root.path().join("second-source.bin");
    write_file(&second_source, b"verified-second");
    assert_eq!(
        promote(&second_source, &destination),
        Ok(normal_install_outcome())
    );
    assert_eq!(fs::read(&destination).unwrap(), b"verified-second");
    assert_eq!(
        fs::read(promotion_backup_path_for_qualification_test(&destination)).unwrap(),
        b"old-valid"
    );
    assert!(!second_source.exists());
}

#[test]
fn stale_reservation_occupies_only_one_slot() {
    let root = TempRoot::new("stale-reservation-next-slot");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");
    let reservation = qualification_reservation_slot_path(&destination, 0);
    write_file(&reservation, b"stale-reservation");

    assert_eq!(promote(&source, &destination), Ok(normal_install_outcome()));
    assert_eq!(fs::read(&destination).unwrap(), b"verified-new");
    assert!(!source.exists());
    assert_eq!(fs::read(&reservation).unwrap(), b"stale-reservation");
}

#[test]
fn bounded_slot_exhaustion_is_typed_and_non_destructive() {
    let root = TempRoot::new("slot-exhaustion");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");
    for slot in 0..32 {
        write_file(
            &qualification_backup_slot_path(&destination, slot),
            b"occupied-backup",
        );
    }

    assert_eq!(
        promote(&source, &destination),
        Err(PromotionError::BackupSlotsExhausted)
    );
    assert_eq!(fs::read(&destination).unwrap(), b"old-valid");
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    assert_eq!(
        fs::read(qualification_backup_slot_path(&destination, 31)).unwrap(),
        b"occupied-backup"
    );
}

#[test]
fn documented_windows_failure_kinds_require_matching_post_state() {
    for failpoint in [
        SafePromotionTestFailpoint::WindowsUnableToRemoveReplaced,
        SafePromotionTestFailpoint::WindowsUnableToMoveReplacement,
    ] {
        let root = TempRoot::new("windows-original-names");
        let source = root.path().join("source.bin");
        let destination = root.path().join("destination.bin");
        write_file(&source, b"verified-new");
        write_file(&destination, b"old-valid");

        assert_eq!(
            promote_with_failpoint(&source, &destination, failpoint),
            Err(PromotionError::PromotionFailed)
        );
        assert_eq!(fs::read(&destination).unwrap(), b"old-valid");
        assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    }

    let root = TempRoot::new("windows-backup-restore");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");
    assert_eq!(
        promote_with_failpoint(
            &source,
            &destination,
            SafePromotionTestFailpoint::WindowsUnableToMoveReplacement2,
        ),
        Err(PromotionError::ReplacementFailedRecovered)
    );
    assert_eq!(fs::read(&destination).unwrap(), b"old-valid");
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");

    write_file(&destination, b"old-valid-again");
    assert_eq!(
        promote_with_failpoint(
            &source,
            &destination,
            SafePromotionTestFailpoint::WindowsOriginalNameFailureWithMovedBackup,
        ),
        Err(PromotionError::RecoveryRequired)
    );
    assert_destination_after_unrestored_backup(&destination, b"old-valid-again");
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
}

#[test]
fn typed_values_redact_fixture_paths() {
    let root = TempRoot::new("private-root");
    let source = root.path().join("private-source.bin");
    let destination = root
        .path()
        .join("other-parent")
        .join("private-destination.bin");
    write_file(&source, b"verified-new");
    let artifact = qualified_artifact(&source);
    let artifact_text = format!("{artifact:?}");
    drop(artifact);
    let error = promote(&source, &destination).unwrap_err();
    let windows_destination = root.path().join("private-windows-destination.bin");
    write_file(&windows_destination, b"old-valid");
    let windows_error = promote_with_failpoint(
        &source,
        &windows_destination,
        SafePromotionTestFailpoint::WindowsUnableToRemoveReplaced,
    )
    .unwrap_err();

    let root_text = root.path().to_string_lossy();
    let rendered = format!("{artifact_text} {error:?} {error} {windows_error:?} {windows_error}");
    assert!(!rendered.contains(root_text.as_ref()));
    assert!(!rendered.contains("private-source"));
    assert!(!rendered.contains("private-destination"));
    assert!(!rendered.contains("1175"));
    assert!(!rendered.contains("1176"));
    assert!(!rendered.contains("1177"));
}

#[test]
fn qualified_promotion_has_no_predelete_and_one_finalization_call_site() {
    let promotion_source = include_str!("../src/safe_promotion.rs");
    let file_core_source = include_str!("../src/file_core.rs");
    let library_surface = include_str!("../src/lib.rs");
    assert!(!promotion_source.contains("remove_file(destination"));
    assert!(!promotion_source.contains("remove_file(&destination"));
    assert!(promotion_source.contains("SetFileInformationByHandle"));
    assert!(promotion_source.contains(".sync_all()"));
    assert!(!promotion_source.contains("MoveFileExW"));
    assert!(!promotion_source.contains("fn remove_bound_path"));
    assert!(!promotion_source.contains("fs::remove_file(&self.path)"));
    assert!(promotion_source.contains("fn release_reservation"));
    assert!(promotion_source.contains("fn renameat2("));
    assert!(promotion_source.contains("rename_unix_noreplace"));
    assert!(promotion_source.contains("rename_unix_exchange"));
    assert!(!promotion_source.contains("fs::rename(staging, destination)"));
    assert!(!promotion_source.contains("REPLACEFILE_WRITE_THROUGH"));
    assert!(!file_core_source.contains("fs::remove_file(destination)"));
    assert!(file_core_source.contains("promote_verified_closed_artifact"));
    assert!(!library_surface.contains("pub use safe_promotion::{"));
    assert!(!library_surface.contains("promote_verified_closed_artifact,"));
    assert!(!library_surface.contains("VerifiedClosedArtifact,"));
    let qualification = promotion_source
        .find("pub(crate) fn for_qualification_test")
        .expect("qualification constructor remains present");
    let cfg_test = promotion_source[..qualification]
        .rfind("#[cfg(test)]")
        .expect("qualification constructor remains test-only");
    let doc_hidden = promotion_source[..cfg_test]
        .rfind("#[doc(hidden)]")
        .expect("qualification constructor remains hidden");
    assert!(doc_hidden < cfg_test && cfg_test < qualification);

    for production_source in [
        include_str!("../src/http_transfer.rs"),
        include_str!("../src/production_worker.rs"),
        include_str!("../src/resolver_execution.rs"),
    ] {
        assert!(!production_source.contains("promote_verified_closed_artifact"));
    }
}

#[cfg(windows)]
#[test]
fn windows_handle_bound_rename_replaces_an_existing_destination() {
    let root = TempRoot::new("windows-replace-file");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");

    assert_eq!(
        promote(&source, &destination),
        Ok(PromotionOutcome::Installed)
    );
    assert_eq!(fs::read(&destination).unwrap(), b"verified-new");
}

#[cfg(windows)]
#[test]
fn windows_destination_share_mode_denial_preserves_source_and_destination() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    let root = TempRoot::new("windows-destination-share-mode");
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination.bin");
    write_file(&source, b"verified-new");
    write_file(&destination, b"old-valid");
    let held = OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0001)
        .open(&destination)
        .unwrap();

    assert_eq!(
        promote(&source, &destination),
        Err(PromotionError::DestinationNotRegular)
    );
    assert_eq!(fs::read(&source).unwrap(), b"verified-new");
    assert_eq!(fs::read(&destination).unwrap(), b"old-valid");

    drop(held);
    assert_eq!(
        promote(&source, &destination),
        Ok(PromotionOutcome::Installed)
    );
    assert_eq!(fs::read(&destination).unwrap(), b"verified-new");
}
