//! Local reuse decisions, format checks and conflict renames on real files.

use std::path::{Path, PathBuf};

use super::probe::{lookup3, probe_file};
use super::quarantine::{conflict_name, conflict_stamp, quarantine_conflict};
use super::*;
use crate::checksum::ChecksumAlgorithm;
use crate::ExpectedChecksum;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("insar-local-reuse-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn never() -> bool {
    false
}

fn probe(path: &Path, format: ProbeFormat, product: Option<&str>) -> ProbeOutcome {
    let file = std::fs::File::open(path).unwrap();
    let size = file.metadata().unwrap().len();
    probe_file(&file, size, format, product).unwrap()
}

// ---------------------------------------------------------------- fixtures

/// A stored (uncompressed) ZIP of the given entries; `zip64` writes the
/// ZIP64 end records with saturated classic fields.
fn zip_bytes(entries: &[(&str, &[u8])], zip64: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&0_u32.to_le_bytes()); // crc (not checked)
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let directory_offset = out.len() as u64;
    let directory_size = central.len() as u64;
    out.extend_from_slice(&central);
    let count = entries.len() as u64;
    if zip64 {
        let zip64_offset = out.len() as u64;
        out.extend_from_slice(&0x0606_4b50_u32.to_le_bytes());
        out.extend_from_slice(&44_u64.to_le_bytes());
        out.extend_from_slice(&[45, 0, 45, 0]);
        out.extend_from_slice(&0_u32.to_le_bytes());
        out.extend_from_slice(&0_u32.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&directory_size.to_le_bytes());
        out.extend_from_slice(&directory_offset.to_le_bytes());
        out.extend_from_slice(&0x0706_4b50_u32.to_le_bytes());
        out.extend_from_slice(&0_u32.to_le_bytes());
        out.extend_from_slice(&zip64_offset.to_le_bytes());
        out.extend_from_slice(&1_u32.to_le_bytes());
    }
    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    if zip64 {
        out.extend_from_slice(&[0xff; 4]);
        out.extend_from_slice(&[0xff; 4]);
        out.extend_from_slice(&[0xff; 4]);
    } else {
        out.extend_from_slice(&(count as u16).to_le_bytes());
        out.extend_from_slice(&(count as u16).to_le_bytes());
        out.extend_from_slice(&(directory_size as u32).to_le_bytes());
        out.extend_from_slice(&(directory_offset as u32).to_le_bytes());
    }
    out.extend_from_slice(&0_u16.to_le_bytes());
    out
}

/// A little-endian TIFF with one strip at the end of the file.
fn tiff_bytes(strip: &[u8], strip_offset_shift: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42_u16.to_le_bytes());
    out.extend_from_slice(&8_u32.to_le_bytes());
    let entries: [(u16, u16, u32, u32); 4] = [
        (256, 3, 1, 4),
        (257, 3, 1, 4),
        (273, 4, 1, 8 + 2 + 4 * 12 + 4 + strip_offset_shift),
        (279, 4, 1, strip.len() as u32),
    ];
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, kind, count, value) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(strip);
    out
}

/// A ustar archive with one member.
fn tar_bytes(name: &str, data: &[u8]) -> Vec<u8> {
    let mut header = [0_u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..108].copy_from_slice(b"0000644\0");
    header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
    header[136..148].copy_from_slice(b"00000000000\0");
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[148..156].copy_from_slice(b"        ");
    let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    let mut out = header.to_vec();
    out.extend_from_slice(data);
    out.resize(512 + data.len().div_ceil(512) * 512, 0);
    out.extend_from_slice(&[0_u8; 1024]);
    out
}

const ORBIT: &str = "<?xml version=\"1.0\"?>\n<Earth_Explorer_File>\n<Earth_Explorer_Header><Fixed_Header><File_Type>AUX_POEORB</File_Type><Validity_Period><Validity_Start>UTC=2026-01-01T22:59:42</Validity_Start><Validity_Stop>UTC=2026-01-03T00:59:42</Validity_Stop></Validity_Period></Fixed_Header></Earth_Explorer_Header>\n<Data_Block/>\n</Earth_Explorer_File>\n";

// ---------------------------------------------------------------- format checks

#[test]
fn lookup3_matches_the_reference_vectors() {
    assert_eq!(lookup3(b"", 0), 0xdead_beef);
    assert_eq!(lookup3(b"Four score and seven years ago", 0), 0x1777_0551);
    assert_eq!(lookup3(b"Four score and seven years ago", 1), 0xcd62_8161);
}

#[test]
fn a_zip_is_readable_only_when_its_directory_is_complete() {
    let scratch = Scratch::new("zip");
    let complete = zip_bytes(&[("a.txt", b"alpha"), ("b/c.bin", &[7_u8; 300])], false);
    let path = scratch.write("product.zip", &complete);
    assert_eq!(probe(&path, ProbeFormat::Zip, None), ProbeOutcome::Readable);
    let zip64 = scratch.write("big.zip", &zip_bytes(&[("a.txt", b"alpha")], true));
    assert_eq!(
        probe(&zip64, ProbeFormat::Zip, None),
        ProbeOutcome::Readable
    );
    for cut in [1_usize, 10, 30, complete.len() / 2] {
        let truncated = scratch.write("truncated.zip", &complete[..complete.len() - cut]);
        assert_eq!(
            probe(&truncated, ProbeFormat::Zip, None),
            ProbeOutcome::Unreadable,
            "cut {cut}"
        );
    }
    let mut garbled = complete.clone();
    let directory_at = complete.len() - 22 - 2 * 46 - "a.txt".len() - "b/c.bin".len();
    garbled[directory_at] ^= 0xff;
    let garbled = scratch.write("garbled.zip", &garbled);
    assert_eq!(
        probe(&garbled, ProbeFormat::Zip, None),
        ProbeOutcome::Unreadable
    );
    let empty = scratch.write("empty.zip", b"");
    assert_eq!(
        probe(&empty, ProbeFormat::Zip, None),
        ProbeOutcome::Unreadable
    );
}

#[test]
fn a_sentinel1_safe_zip_must_contain_its_manifest() {
    let scratch = Scratch::new("safe");
    let product = "S1A_IW_SLC__1SDV_20260101T000000_20260101T000027_000001_000001_0001";
    let manifest = format!("{product}.SAFE/manifest.safe");
    let with_manifest = scratch.write(
        "with.zip",
        &zip_bytes(&[(manifest.as_str(), b"<xfdu/>"), ("x", b"1")], false),
    );
    assert_eq!(
        probe(&with_manifest, ProbeFormat::Zip, Some(product)),
        ProbeOutcome::Readable
    );
    let without = scratch.write("without.zip", &zip_bytes(&[("x", b"1")], false));
    assert_eq!(
        probe(&without, ProbeFormat::Zip, Some(product)),
        ProbeOutcome::Unreadable
    );
    // Not a Sentinel-1 name: no manifest requirement.
    assert_eq!(
        probe(&without, ProbeFormat::Zip, Some("ALPSRP123")),
        ProbeOutcome::Readable
    );
}

#[test]
fn an_hdf5_file_is_readable_only_up_to_its_end_address() {
    let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("hdf5");
    for (name, latest) in [("v0.h5", false), ("latest.h5", true)] {
        let path = scratch.0.join(name);
        {
            let file = if latest {
                hdf5::File::with_options()
                    .with_fapl(|fapl| fapl.libver_latest())
                    .create(crate::long_path::native_path(&path))
                    .unwrap()
            } else {
                hdf5::File::create(crate::long_path::native_path(&path)).unwrap()
            };
            file.new_dataset::<u32>()
                .shape([64, 64])
                .create("science")
                .unwrap()
                .write(&ndarray::Array2::<u32>::from_elem((64, 64), 9))
                .unwrap();
        }
        assert_eq!(
            probe(&path, ProbeFormat::Hdf5, None),
            ProbeOutcome::Readable,
            "{name}"
        );
        let bytes = std::fs::read(&path).unwrap();
        let truncated = scratch.write("truncated.h5", &bytes[..bytes.len() - 100]);
        assert_eq!(
            probe(&truncated, ProbeFormat::Hdf5, None),
            ProbeOutcome::Unreadable,
            "{name}"
        );
        if latest {
            let mut garbled = bytes.clone();
            garbled[13] ^= 0x01; // a byte covered by the superblock checksum
            let garbled = scratch.write("garbled.h5", &garbled);
            assert_eq!(
                probe(&garbled, ProbeFormat::Hdf5, None),
                ProbeOutcome::Unreadable
            );
        }
    }
    let other = scratch.write("other.h5", b"not an hdf5 file at all, just text");
    assert_eq!(
        probe(&other, ProbeFormat::Hdf5, None),
        ProbeOutcome::Unreadable
    );
}

#[test]
fn a_tiff_is_readable_only_when_every_strip_is_inside_the_file() {
    let scratch = Scratch::new("tiff");
    let complete = scratch.write("dem.tif", &tiff_bytes(&[1_u8; 32], 0));
    assert_eq!(
        probe(&complete, ProbeFormat::Tiff, None),
        ProbeOutcome::Readable
    );
    let bytes = tiff_bytes(&[1_u8; 32], 0);
    let truncated = scratch.write("truncated.tif", &bytes[..bytes.len() - 8]);
    assert_eq!(
        probe(&truncated, ProbeFormat::Tiff, None),
        ProbeOutcome::Unreadable
    );
    let outside = scratch.write("outside.tif", &tiff_bytes(&[1_u8; 32], 1000));
    assert_eq!(
        probe(&outside, ProbeFormat::Tiff, None),
        ProbeOutcome::Unreadable
    );
    let other = scratch.write("other.tif", b"GIF89a....");
    assert_eq!(
        probe(&other, ProbeFormat::Tiff, None),
        ProbeOutcome::Unreadable
    );
}

#[test]
fn a_tar_is_readable_only_with_its_members_and_end_blocks() {
    let scratch = Scratch::new("tar");
    let bytes = tar_bytes("LC09_L2SP_MTL.txt", &[b'x'; 700]);
    let complete = scratch.write("bundle.tar", &bytes);
    assert_eq!(
        probe(&complete, ProbeFormat::Tar, None),
        ProbeOutcome::Readable
    );
    let truncated = scratch.write("truncated.tar", &bytes[..bytes.len() - 600]);
    assert_eq!(
        probe(&truncated, ProbeFormat::Tar, None),
        ProbeOutcome::Unreadable
    );
    let mut garbled = bytes.clone();
    garbled[3] ^= 0x20; // the header checksum no longer matches
    let garbled = scratch.write("garbled.tar", &garbled);
    assert_eq!(
        probe(&garbled, ProbeFormat::Tar, None),
        ProbeOutcome::Unreadable
    );
}

#[test]
fn text_formats_require_their_closing_structure() {
    let scratch = Scratch::new("text");
    let orbit = scratch.write("S1A_OPER_AUX_POEORB.EOF", ORBIT.as_bytes());
    assert_eq!(
        probe(&orbit, ProbeFormat::PreciseOrbit, None),
        ProbeOutcome::Readable
    );
    let cut = scratch.write("cut.EOF", &ORBIT.as_bytes()[..ORBIT.len() - 30]);
    assert_eq!(
        probe(&cut, ProbeFormat::PreciseOrbit, None),
        ProbeOutcome::Unreadable
    );
    let mtl = scratch.write(
        "X_MTL.txt",
        b"GROUP = LANDSAT_METADATA_FILE\n  A = 1\nEND_GROUP = LANDSAT_METADATA_FILE\nEND\n",
    );
    assert_eq!(
        probe(&mtl, ProbeFormat::MetadataText, None),
        ProbeOutcome::Readable
    );
    let mtl_cut = scratch.write("Y_MTL.txt", b"GROUP = LANDSAT_METADATA_FILE\n  A = 1\n");
    assert_eq!(
        probe(&mtl_cut, ProbeFormat::MetadataText, None),
        ProbeOutcome::Unreadable
    );
    let xml = scratch.write("a.xml", b"<?xml version=\"1.0\"?><root><a/></root>");
    assert_eq!(probe(&xml, ProbeFormat::Xml, None), ProbeOutcome::Readable);
    let xml_cut = scratch.write("b.xml", b"<?xml version=\"1.0\"?><root><a/>");
    assert_eq!(
        probe(&xml_cut, ProbeFormat::Xml, None),
        ProbeOutcome::Unreadable
    );
    let json = scratch.write("a.json", b"{\"bursts\": [1, 2]}");
    assert_eq!(
        probe(&json, ProbeFormat::Json, None),
        ProbeOutcome::Readable
    );
    let json_cut = scratch.write("b.json", b"{\"bursts\": [1, 2");
    assert_eq!(
        probe(&json_cut, ProbeFormat::Json, None),
        ProbeOutcome::Unreadable
    );
    let other = scratch.write("data.bin", b"payload");
    assert_eq!(
        probe(&other, ProbeFormat::for_file_name("data.bin"), None),
        ProbeOutcome::Unsupported
    );
}

#[test]
fn formats_follow_the_planned_file_name() {
    assert_eq!(ProbeFormat::for_file_name("S1A.zip"), ProbeFormat::Zip);
    assert_eq!(ProbeFormat::for_file_name("NISAR.H5"), ProbeFormat::Hdf5);
    assert_eq!(ProbeFormat::for_file_name("dem.TIF"), ProbeFormat::Tiff);
    assert_eq!(ProbeFormat::for_file_name("LC09.tar"), ProbeFormat::Tar);
    assert_eq!(
        ProbeFormat::for_file_name("S1A_OPER_AUX_POEORB_V1.EOF"),
        ProbeFormat::PreciseOrbit
    );
    assert_eq!(
        ProbeFormat::for_file_name("LC09_MTL.txt"),
        ProbeFormat::MetadataText
    );
    assert_eq!(
        ProbeFormat::for_file_name("notes.txt"),
        ProbeFormat::Unsupported
    );
}

// ---------------------------------------------------------------- conflict rename

#[test]
fn a_conflict_is_renamed_aside_without_overwriting_anything() {
    let scratch = Scratch::new("quarantine");
    let destination = scratch.write("scene.zip", b"old content");
    let stamp = conflict_stamp("2026-10-20T01:02:03Z");
    assert_eq!(stamp, "20261020T010203Z");
    // An earlier conflict with the same time keeps its name and content.
    let earlier = scratch.write(&conflict_name("scene.zip", &stamp, 1), b"earlier");
    let renamed = quarantine_conflict(&destination, &stamp).unwrap();
    assert_eq!(
        renamed,
        scratch
            .0
            .join("scene.zip.eostage-conflict-20261020T010203Z-2")
    );
    assert!(
        !destination.exists(),
        "the planned name is free for the download"
    );
    assert_eq!(std::fs::read(&renamed).unwrap(), b"old content");
    assert_eq!(std::fs::read(&earlier).unwrap(), b"earlier");
}

// ---------------------------------------------------------------- decisions

fn decide(
    destination: &Path,
    size: Option<u64>,
    checksum: Option<&ExpectedChecksum>,
    ledger: Option<&LedgerEvidence>,
) -> LocalReuseDecision {
    let file_name = destination.file_name().unwrap().to_str().unwrap();
    decide_local_reuse(&LocalReuseRequest {
        destination,
        authoritative_size: size,
        checksum,
        format: ProbeFormat::for_file_name(file_name),
        product: None,
        ledger,
        allow_digest: true,
        stop: &never,
    })
}

fn grade(decision: &LocalReuseDecision) -> Option<ReuseGrade> {
    match decision {
        LocalReuseDecision::Reuse(reuse) => Some(reuse.grade),
        _ => None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn md5_hex(bytes: &[u8]) -> String {
    use md5::Digest;
    md5::Md5::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn ledger_for(path: &Path, sha256: Option<String>) -> LedgerEvidence {
    let status = crate::file_identity::file_status(&std::fs::File::open(path).unwrap()).unwrap();
    LedgerEvidence {
        platform: status.platform.to_owned(),
        volume_id: status.volume_id,
        file_id: status.file_id,
        size: status.size,
        modified: status.modified,
        changed: status.changed,
        stable: status.strength == crate::file_identity::IdentityStrength::Stable,
        sha256,
        service_checksum: None,
    }
}

#[test]
fn a_missing_destination_downloads_and_a_directory_is_unsafe() {
    let scratch = Scratch::new("missing");
    assert_eq!(
        decide(&scratch.0.join("absent.zip"), Some(5), None, None),
        LocalReuseDecision::Missing
    );
    std::fs::create_dir_all(scratch.0.join("folder.zip")).unwrap();
    assert_eq!(
        decide(&scratch.0.join("folder.zip"), Some(5), None, None),
        LocalReuseDecision::Unsafe
    );
}

#[test]
fn without_evidence_only_the_authoritative_size_and_a_readable_format_reuse() {
    let scratch = Scratch::new("r4");
    let bytes = zip_bytes(&[("a.txt", b"alpha")], false);
    let path = scratch.write("product.zip", &bytes);
    let size = bytes.len() as u64;
    let decision = decide(&path, Some(size), None, None);
    assert_eq!(grade(&decision), Some(ReuseGrade::SizeAndFormat));
    assert_eq!(
        decide(&path, Some(size + 1), None, None),
        LocalReuseDecision::Conflict(ConflictReason::SizeMismatch)
    );
    assert_eq!(
        decide(&path, None, None, None),
        LocalReuseDecision::Conflict(ConflictReason::UnknownSize)
    );
    let truncated = scratch.write("cut.zip", &bytes[..bytes.len() - 4]);
    assert_eq!(
        decide(&truncated, Some(bytes.len() as u64 - 4), None, None),
        LocalReuseDecision::Conflict(ConflictReason::FormatUnreadable)
    );
    let unknown = scratch.write("payload.dat", b"12345");
    assert_eq!(
        decide(&unknown, Some(5), None, None),
        LocalReuseDecision::Conflict(ConflictReason::FormatUnsupported)
    );
    let empty = scratch.write("empty.zip", b"");
    assert_eq!(
        decide(&empty, Some(0), None, None),
        LocalReuseDecision::Conflict(ConflictReason::FormatUnreadable)
    );
}

#[test]
fn a_service_checksum_is_compared_locally() {
    let scratch = Scratch::new("r1");
    let path = scratch.write("payload.dat", b"service-verified payload");
    let md5 = ExpectedChecksum::new("md5", &md5_hex(b"service-verified payload")).unwrap();
    match decide(&path, Some(24), Some(&md5), None) {
        LocalReuseDecision::Reuse(reuse) => {
            assert_eq!(reuse.grade, ReuseGrade::ServiceChecksum);
            assert_eq!(
                reuse.sha256.as_deref(),
                Some(sha256_hex(b"service-verified payload").as_str())
            );
        }
        other => panic!("{other:?}"),
    }
    let other = ExpectedChecksum::new("md5", &md5_hex(b"another payload")).unwrap();
    assert_eq!(
        decide(&path, Some(24), Some(&other), None),
        LocalReuseDecision::Conflict(ConflictReason::ChecksumMismatch)
    );
    // Unknown size with a checksum still proves the content.
    assert_eq!(
        grade(&decide(&path, None, Some(&md5), None)),
        Some(ReuseGrade::ServiceChecksum)
    );
    // A stop during the read leaves the file undecided.
    let stopped = decide_local_reuse(&LocalReuseRequest {
        destination: &path,
        authoritative_size: Some(24),
        checksum: Some(&md5),
        format: ProbeFormat::Unsupported,
        product: None,
        ledger: None,
        allow_digest: true,
        stop: &|| true,
    });
    assert_eq!(stopped, LocalReuseDecision::Stopped);
}

#[test]
fn the_ledger_reuses_an_unchanged_file_without_reading_it() {
    let scratch = Scratch::new("r2");
    // Content no format check accepts: only the ledger identity can reuse it.
    let path = scratch.write("product.zip", b"not a zip, but verified at landing");
    let ledger = ledger_for(
        &path,
        Some(sha256_hex(b"not a zip, but verified at landing")),
    );
    if !ledger.stable {
        return; // a weak-identity volume never reuses by identity
    }
    let decision = decide(&path, Some(34), None, Some(&ledger));
    assert_eq!(grade(&decision), Some(ReuseGrade::LedgerIdentity));
    // A checksum the ledger did not verify needs the content.
    let md5 =
        ExpectedChecksum::new("md5", &md5_hex(b"not a zip, but verified at landing")).unwrap();
    assert_eq!(
        grade(&decide(&path, Some(34), Some(&md5), Some(&ledger))),
        Some(ReuseGrade::ServiceChecksum)
    );
    let mut verified = ledger.clone();
    verified.service_checksum = Some((ChecksumAlgorithm::Md5, md5.value().to_owned()));
    assert_eq!(
        grade(&decide(&path, Some(34), Some(&md5), Some(&verified))),
        Some(ReuseGrade::LedgerIdentity)
    );
}

#[test]
fn a_changed_identity_is_rehashed_against_the_ledger() {
    let scratch = Scratch::new("r3");
    let path = scratch.write("product.zip", b"landed bytes");
    let ledger = ledger_for(&path, Some(sha256_hex(b"landed bytes")));
    // Same content, new identity (copied over the original).
    let copy = scratch.write("copy.tmp", b"landed bytes");
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&copy, &path).unwrap();
    assert_eq!(
        grade(&decide(&path, Some(12), None, Some(&ledger))),
        Some(ReuseGrade::LedgerRehash)
    );
    // Different content of the same size.
    std::fs::write(&path, b"LANDED BYTES").unwrap();
    assert_eq!(
        decide(&path, Some(12), None, Some(&ledger)),
        LocalReuseDecision::Conflict(ConflictReason::LedgerMismatch)
    );
    // A ledger without a hash falls back to the format check.
    let unhashed = ledger_for(&path, None);
    let mut moved = unhashed.clone();
    moved.file_id ^= 1;
    assert_eq!(
        decide(&path, Some(12), None, Some(&moved)),
        LocalReuseDecision::Conflict(ConflictReason::FormatUnreadable)
    );
}
