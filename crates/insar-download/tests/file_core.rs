use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::DownloadLayout;
use insar_download::{
    finalize_part, inspect_destination, open_part_for_transfer, part_path, plan_download_target,
    plan_sar_catalog_target, FilePreparation,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-download-files-{}-{nonce}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn plans_exact_desktop_layouts_and_rejects_unsafe_names() {
    let root = Path::new(r"E:\用户选择目录");
    assert_eq!(
        plan_download_target(root, &DownloadLayout::Direct, "scene.zip").unwrap(),
        root.join("scene.zip")
    );
    assert_eq!(
        plan_download_target(root, &DownloadLayout::Sentinel1Slc, "scene.zip").unwrap(),
        root.join("SLC").join("scene.zip")
    );
    assert_eq!(
        plan_download_target(root, &DownloadLayout::SentinelOrbitAuxPoeorb, "orbit.EOF",).unwrap(),
        root.join("Sentinel_Orbit")
            .join("AUX_POEORB")
            .join("orbit.EOF")
    );
    assert!(plan_download_target(root, &DownloadLayout::Direct, "../escape.zip").is_err());
    assert!(plan_download_target(root, &DownloadLayout::Direct, "CON.zip").is_err());
}

#[test]
fn generic_sar_layout_uses_dataset_and_native_processing_level() {
    let root = Path::new("E:/downloads");
    assert_eq!(
        plan_sar_catalog_target(root, "ALOS PALSAR", "L1.1", "ALPSRP-test.zip").unwrap(),
        root.join("ALOS_PALSAR")
            .join("L1_1")
            .join("ALPSRP-test.zip")
    );
    assert!(plan_sar_catalog_target(root, "../escape", "L1.1", "product.zip").is_err());
}

#[test]
fn complete_destination_skips_without_creating_a_part() {
    let temp = TempDirectory::new();
    let destination = temp.path().join("完整影像.zip");
    fs::write(&destination, b"abcde").unwrap();

    assert_eq!(
        inspect_destination(&destination, Some(5)).unwrap(),
        FilePreparation::SkipComplete { bytes: 5 }
    );
    assert!(!part_path(&destination).exists());
}

/// Downloader D3 (local-first dedup, defect in file_core.rs:132): an existing
/// destination whose expected size is unknown was taken as complete, even an
/// empty file. Without an expected size nothing proves the file complete.
#[test]
fn an_existing_destination_of_unknown_expected_size_is_never_complete() {
    let temp = TempDirectory::new();
    for (name, content) in [("empty.zip", &b""[..]), ("truncated.zip", &b"abc"[..])] {
        let destination = temp.path().join(name);
        fs::write(&destination, content).unwrap();
        assert_eq!(
            inspect_destination(&destination, None).unwrap(),
            FilePreparation::Restart,
            "{name}"
        );
        assert_eq!(
            fs::read(&destination).unwrap(),
            content,
            "{name} is left untouched"
        );
    }
}

#[test]
fn exact_part_finalizes_and_replaces_only_an_invalid_destination() {
    let temp = TempDirectory::new();
    let destination = temp.path().join("scene.zip");
    fs::write(&destination, b"bad").unwrap();
    fs::write(part_path(&destination), b"abcde").unwrap();

    assert_eq!(
        inspect_destination(&destination, Some(5)).unwrap(),
        FilePreparation::FinalizePart { bytes: 5 }
    );
    finalize_part(&destination, Some(5)).unwrap();

    assert_eq!(fs::read(&destination).unwrap(), b"abcde");
    assert!(!part_path(&destination).exists());
}

#[test]
fn shorter_part_resumes_and_oversized_part_restarts_from_zero() {
    let temp = TempDirectory::new();
    let destination = temp.path().join("scene.zip");
    let part = part_path(&destination);
    fs::write(&part, b"abc").unwrap();
    assert_eq!(
        inspect_destination(&destination, Some(5)).unwrap(),
        FilePreparation::Resume { offset: 3 }
    );
    open_part_for_transfer(&destination, FilePreparation::Resume { offset: 3 })
        .unwrap()
        .write_all(b"de")
        .unwrap();
    finalize_part(&destination, Some(5)).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), b"abcde");

    fs::remove_file(&destination).unwrap();
    fs::write(&part, b"too-large").unwrap();
    assert_eq!(
        inspect_destination(&destination, Some(5)).unwrap(),
        FilePreparation::Restart
    );
    assert!(!part.exists());
}

#[cfg(windows)]
#[test]
fn windows_share_mode_denial_preserves_files_and_retry_promotes_after_release() {
    use std::os::windows::fs::OpenOptionsExt;

    let temp = TempDirectory::new();
    let destination = temp.path().join("scene.zip");
    let part = part_path(&destination);
    fs::write(&destination, b"previous-valid").unwrap();
    fs::write(&part, b"verified-new").unwrap();

    let handle = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&destination)
        .unwrap();
    let error = finalize_part(&destination, Some(12)).unwrap_err();
    assert_eq!(error.code(), "DL005");

    drop(handle);
    assert_eq!(fs::read(&destination).unwrap(), b"previous-valid");
    assert_eq!(fs::read(&part).unwrap(), b"verified-new");
    assert_eq!(finalize_part(&destination, Some(12)).unwrap(), 12);
    assert_eq!(fs::read(&destination).unwrap(), b"verified-new");
    assert!(!part.exists());
}

#[test]
fn size_mismatch_never_promotes_a_partial_file() {
    let temp = TempDirectory::new();
    let destination = temp.path().join("scene.zip");
    fs::write(part_path(&destination), b"abc").unwrap();

    let error = finalize_part(&destination, Some(5)).unwrap_err();
    assert_eq!(error.code(), "DL002");
    assert!(!destination.exists());
    assert!(part_path(&destination).exists());
}

#[test]
fn hardlinked_part_is_rejected_before_append_or_truncate() {
    let temp = TempDirectory::new();
    let destination = temp.path().join("scene.zip");
    let part = part_path(&destination);
    let victim = temp.path().join("victim.bin");
    fs::write(&victim, b"do-not-modify").unwrap();
    fs::hard_link(&victim, &part).unwrap();

    assert!(open_part_for_transfer(&destination, FilePreparation::Restart).is_err());
    assert!(open_part_for_transfer(&destination, FilePreparation::Resume { offset: 13 }).is_err());
    assert_eq!(fs::read(&victim).unwrap(), b"do-not-modify");
}

#[cfg(unix)]
#[test]
fn symlinked_part_is_rejected_before_append_or_truncate() {
    use std::os::unix::fs::symlink;

    let temp = TempDirectory::new();
    let destination = temp.path().join("scene.zip");
    let part = part_path(&destination);
    let victim = temp.path().join("victim.bin");
    fs::write(&victim, b"do-not-modify").unwrap();
    symlink(&victim, &part).unwrap();

    assert!(open_part_for_transfer(&destination, FilePreparation::Restart).is_err());
    assert!(open_part_for_transfer(&destination, FilePreparation::Resume { offset: 13 }).is_err());
    assert_eq!(fs::read(&victim).unwrap(), b"do-not-modify");
}
