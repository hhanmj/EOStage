use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_download::{
    inspect_resolved_destination, part_path, verify_file_checksum, ChecksumAlgorithm,
    ExpectedChecksum, FilePreparation, ResolvedPartContext,
};

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-resolved-integrity-{}-{nonce}-{seq}",
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

fn sha256(value: &str) -> ExpectedChecksum {
    ExpectedChecksum::new("sha256", value).unwrap()
}

#[test]
fn checksum_algorithms_parse_exact_hex_shapes() {
    assert_eq!(
        ChecksumAlgorithm::parse("SHA256").unwrap(),
        ChecksumAlgorithm::Sha256
    );
    assert!(ChecksumAlgorithm::parse("crc32").is_err());
    assert!(ExpectedChecksum::new("sha256", "not-hex").is_err());
    assert!(ExpectedChecksum::new("md5", "aa").is_err());

    for (algorithm, value) in [
        ("md5", "3f3815b756887deb86d3c24adcef64fc"),
        ("sha1", "9351649978c8ff2f5f065277b4af642c2368f3b3"),
        (
            "sha256",
            "bb55a5ab9f6a401bfca2ff147197209c59a002a3994731d8f2d2f45692f41846",
        ),
        (
            "sha512",
            "28fec74a254106656caec0f68996f715c9efa0e940faffe8f32ac7e42d90d0c9dfc9b26fbf7e3843907acdccf7ddbcd158c4f8f766081b15970991bd0a9a8b04",
        ),
    ] {
        let temp = TempDirectory::new();
        let file = temp.path().join("whole-file.bin");
        fs::write(&file, b"prefixabc").unwrap();
        verify_file_checksum(&file, &ExpectedChecksum::new(algorithm, value).unwrap()).unwrap();
    }
}

#[test]
fn existing_final_requires_full_size_and_checksum_before_skip() {
    let temp = TempDirectory::new();
    let destination = temp.path().join("final.bin");
    fs::write(&destination, b"abc").unwrap();
    let expected = sha256("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");

    assert_eq!(
        inspect_resolved_destination(
            &destination,
            Some(3),
            Some(&expected),
            ResolvedPartContext::RecoveredAfterRestart,
        )
        .unwrap(),
        FilePreparation::SkipComplete { bytes: 3 }
    );

    let mismatch = sha256("0000000000000000000000000000000000000000000000000000000000000000");
    assert_eq!(
        inspect_resolved_destination(
            &destination,
            Some(3),
            Some(&mismatch),
            ResolvedPartContext::RecoveredAfterRestart,
        )
        .unwrap_err()
        .code(),
        "DL002"
    );
    assert_eq!(fs::read(&destination).unwrap(), b"abc");
}

#[test]
fn complete_part_is_verified_before_finalize_and_mismatch_is_preserved() {
    let temp = TempDirectory::new();
    let destination = temp.path().join("part.bin");
    let part = part_path(&destination);
    fs::write(&part, b"abc").unwrap();
    let expected = sha256("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");

    assert_eq!(
        inspect_resolved_destination(
            &destination,
            Some(3),
            Some(&expected),
            ResolvedPartContext::RecoveredAfterRestart,
        )
        .unwrap(),
        FilePreparation::FinalizePart { bytes: 3 }
    );

    let mismatch = sha256("0000000000000000000000000000000000000000000000000000000000000000");
    assert_eq!(
        inspect_resolved_destination(
            &destination,
            Some(3),
            Some(&mismatch),
            ResolvedPartContext::RecoveredAfterRestart,
        )
        .unwrap_err()
        .code(),
        "DL002"
    );
    assert!(part.exists());
    assert!(!destination.exists());
}

#[test]
fn restart_recovery_requires_checksum_evidence_for_range() {
    let temp = TempDirectory::new();
    let destination = temp.path().join("resume.bin");
    let part = part_path(&destination);
    fs::write(&part, b"a").unwrap();
    let expected = sha256("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");

    assert_eq!(
        inspect_resolved_destination(
            &destination,
            Some(3),
            Some(&expected),
            ResolvedPartContext::RecoveredAfterRestart,
        )
        .unwrap(),
        FilePreparation::Resume { offset: 1 }
    );
    assert_eq!(
        inspect_resolved_destination(
            &destination,
            Some(3),
            None,
            ResolvedPartContext::RecoveredAfterRestart,
        )
        .unwrap(),
        FilePreparation::Restart
    );
    assert!(part.exists());
    assert_eq!(
        inspect_resolved_destination(
            &destination,
            Some(3),
            None,
            ResolvedPartContext::CurrentExecution,
        )
        .unwrap(),
        FilePreparation::Resume { offset: 1 }
    );
}
