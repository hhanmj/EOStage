//! Offline-only writer and scientific validator for client-derived NISAR subsets.
//!
//! This module deliberately accepts no URL, HTTP client, resolver, or credential
//! capability. It consumes only an immutable Request, a sealed completed Range
//! result, verified local chunk blocks, and bounded offline metadata evidence.
#![allow(non_local_definitions)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_int, CString};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hdf5::filters::Filter;
use hdf5::globals::H5P_DATASET_CREATE;
use hdf5::plist::dataset_create::Layout;
use hdf5::types::{FixedAscii, H5Type, TypeDescriptor};
use hdf5::{Dataset, File, Group, Location, LocationType};
use hdf5_sys::h5a::{H5Aclose, H5Acreate2, H5Aread, H5Awrite};
use hdf5_sys::h5d::{H5Dclose, H5Dcreate2, H5Dread, H5Dwrite};
use hdf5_sys::h5l::{H5L_info_t, H5Lget_info2, H5L_TYPE_HARD};
use hdf5_sys::h5p::{
    H5Pclose, H5Pcreate, H5Pget_external_count, H5Pset_chunk, H5Pset_filter,
    H5Pset_obj_track_times, H5P_DEFAULT,
};
use hdf5_sys::h5s::{
    H5S_seloper_t, H5Sclose, H5Screate, H5Screate_simple, H5Sselect_hyperslab, H5S_ALL, H5S_SCALAR,
};
use hdf5_sys::h5t::{H5Tclose, H5Tdecode2, H5Tencode, H5Topen2};
use hdf5_sys::h5z::{H5Z_FILTER_DEFLATE, H5Z_FILTER_FLETCHER32, H5Z_FILTER_SHUFFLE};
use insar_core::{
    derive_nisar_subset_dependency_closure_v2, NisarSubsetActualExtentV1,
    NisarSubsetArtifactManifestV1, NisarSubsetCheckStatus, NisarSubsetChecksumAlgorithm,
    NisarSubsetDependencyEvidenceAuthorityV2, NisarSubsetDependencyRelationV2,
    NisarSubsetFrequency, NisarSubsetLogicalWindowV1, NisarSubsetPreparedIntentV2,
    NisarSubsetRequestManifestV1, NisarSubsetScienceValidationV1,
    NisarSubsetSourceDependencyEdgeV2, NisarSubsetSourceDependencyGraphV2,
    NisarSubsetSourceDependencyNodeKindV2, NisarSubsetSourceDependencyNodeV2,
    NisarSubsetSourceIdentityV1, NisarSubsetSourceValidatorV1, NisarSubsetTransferState,
    NisarSubsetValidationCheckV1, NisarSubsetValidatorKind, NisarSubsetValidatorPolicyV1,
    NisarSubsetValidityMaskV1, NisarSubsetWriterV1, NISAR_SUBSET_ARTIFACT_MANIFEST_CONTRACT,
    NISAR_SUBSET_OUTPUT_AUTHORITY, NISAR_SUBSET_SCHEMA_VERSION,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[cfg(feature = "nisar-subset-production")]
use insar_core::NisarSubsetPlanV2;
#[cfg(feature = "nisar-subset-production")]
use insar_store::parse_nisar_subset_prepared_intent_v2_envelope;
use thiserror::Error;

use crate::hdf5_range_transfer::{
    acquire_hdf5_workspace_lock, hdf5_datatype_sha256, supported_datatype_sha256,
    Hdf5ChunkDatatypeV1, Hdf5ChunkFilterPipelineV1, Hdf5RangePlanV1, Hdf5RangeTransferResultV1,
    Hdf5RangeWorkspaceV1, VerifiedHdf5ChunkAllocationV1, VerifiedHdf5ChunkBlockV1,
};
use crate::nisar_hdf5_dimension_scales as dimension_scales;
use crate::nisar_hdf5_validation::validate_derived_hdf5_security;
#[cfg(feature = "nisar-subset-production")]
use crate::nisar_planning_v2::{NisarSubsetPhysicalExecutionPlanV2, NisarSubsetPlanningCompleteV2};
use crate::nisar_science_root::NisarScienceRoot;
#[cfg(feature = "nisar-subset-production")]
use crate::nisar_science_transfer_v2::{
    revalidate_complete_science_for_writer_v2, NisarSubsetScienceAuthorityV2,
    NisarSubsetScienceTransferErrorV2, NisarSubsetScienceTransferResultV2, WriterMaterialCheckV2,
};
#[cfg(feature = "nisar-subset-production")]
use crate::nisar_subset_stream_v2::{
    v2_check_disk_space, v2_check_output_limit, v2_stream_band_budget, v2_stream_copy,
    v2_stream_tiles, v2_stream_verify, V2OutputAdmissionRefusal, V2StreamElement, V2StreamError,
    V2StreamTile,
};
#[cfg(feature = "nisar-subset-production")]
use crate::nisar_writer_admission_v2::{
    admit_nisar_writer_v2, NisarWriterAdmissionEventV2, NisarWriterOutputSlotsV2,
    NISAR_WRITER_V2_DEFAULT_OUTPUT_SLOTS, NISAR_WRITER_V2_LONG_WAIT,
};
#[cfg(feature = "nisar-subset-production")]
use crate::nisar_writer_timings_v2::{NisarWriterStageTimerV2, NisarWriterStageV2};
#[cfg(feature = "nisar-subset-production")]
use crate::pinned_directory::PinnedDirectoryPublication;
use crate::pinned_directory::{safe_component, PinnedDirectory};
use crate::safe_promotion::{promote_verified_closed_artifact, VerifiedClosedArtifact};
use crate::{DestinationLeaseController, DestinationLeaseError};
#[cfg(feature = "nisar-subset-production")]
use crate::{DownloadWork, WorkerControl};

const MAX_METADATA_VALUES: usize = 8_000_000;
// The real NISAR RSLC complete metadata value inventory (logical bytes:
// geolocation grids, RFI tables, orbit/attitude, identification) is about
// 270 MB, larger than the old 192 MB fixture-era bound. The value data is
// physically ~170 MB and mostly inside the 195 MB prefix; the bound must
// admit the real source's logical metadata payload so the evidence snapshot
// can be computed faithfully.
const MAX_METADATA_BYTES: usize = 512 * 1024 * 1024;
const MAX_ATTRIBUTE_BYTES: usize = 1024 * 1024;
const MAX_DATASET_HASH_SLAB_BYTES: usize = 2 * 1024 * 1024;
// The first qualified dual-frequency scene has 207 datasets. That count is
// retained as fixture evidence, not as a product-wide invariant: valid P05023
// single-frequency products have a smaller inventory. Source admission is
// instead based on the required structural paths below plus this traversal
// ceiling and the source-bound inventory fingerprint.
#[allow(
    dead_code,
    reason = "used by the offline fixture module, which is also compiled as dependency test support"
)]
pub(crate) const FIRST_SCENE_SOURCE_DATASET_COUNT: usize = 207;
pub(crate) const MAX_SUPPORTED_SOURCE_DATASET_COUNT: usize = 512;
const MAX_STREAM_WINDOW_BYTES: usize = 32 * 1024 * 1024;
const MAX_OUTPUT_LOGICAL_BYTES: usize = 2 * 1024 * 1024 * 1024;
const MAX_DECODED_CHUNK_BYTES: usize = 16 * 1024 * 1024;
const MAX_WRITER_WORKING_SET_BYTES: usize = 384 * 1024 * 1024;
#[cfg(feature = "nisar-subset-production")]
const V2_AXIS_READ_CHUNK_VALUES: usize = 64 * 1024;
#[cfg(test)]
static V2_AXIS_MAX_READ_VALUES_OBSERVED: AtomicUsize = AtomicUsize::new(0);
const DERIVED_ROOT: &str = "/science/LSAR/RSLC/derivedSubsets";
const IDENTIFICATION_PATH: &str = "/science/LSAR/identification";
const ORBIT_PATH: &str = "/science/LSAR/RSLC/metadata/orbit";
const ATTITUDE_PATH: &str = "/science/LSAR/RSLC/metadata/attitude";
const GEOLOCATION_PATH: &str = "/science/LSAR/RSLC/metadata/geolocationGrid";
const PROCESSING_PATH: &str = "/science/LSAR/RSLC/metadata/processingInformation";
const CALIBRATION_PATH: &str = "/science/LSAR/RSLC/metadata/calibrationInformation";
const PROVENANCE_PATH: &str = "/science/LSAR/RSLC/metadata/remoteSubsetProvenance";
#[cfg(feature = "nisar-subset-live-acceptance")]
pub(crate) fn nisar_science_root_for_oracle(swath: &str) -> Option<&'static str> {
    match swath {
        "LSAR_RSLC" => Some("/science/LSAR"),
        "SSAR_RSLC" => Some("/science/SSAR"),
        _ => None,
    }
}
static NEXT_TEMP_NONCE: AtomicU64 = AtomicU64::new(0);
#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
static V2_REGULAR_BUILDING_RECOVERY_HITS: AtomicUsize = AtomicUsize::new(0);

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
#[derive(Clone, Debug, Default)]
pub(crate) struct MetadataCaptureDiagnosticForTest {
    pub(crate) stage: &'static str,
    pub(crate) object_path: Option<String>,
    pub(crate) detail: Option<String>,
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
thread_local! {
    static METADATA_CAPTURE_DIAGNOSTIC_FOR_TEST: std::cell::RefCell<MetadataCaptureDiagnosticForTest> =
        std::cell::RefCell::new(MetadataCaptureDiagnosticForTest::default());
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
fn record_metadata_capture_stage_for_test(stage: &'static str, object_path: Option<String>) {
    METADATA_CAPTURE_DIAGNOSTIC_FOR_TEST.with(|diagnostic| {
        *diagnostic.borrow_mut() = MetadataCaptureDiagnosticForTest {
            stage,
            object_path,
            detail: None,
        };
    });
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
fn record_metadata_capture_detail_for_test(detail: String) {
    METADATA_CAPTURE_DIAGNOSTIC_FOR_TEST.with(|diagnostic| {
        diagnostic.borrow_mut().detail = Some(detail);
    });
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
pub(crate) fn metadata_capture_diagnostic_for_test() -> MetadataCaptureDiagnosticForTest {
    METADATA_CAPTURE_DIAGNOSTIC_FOR_TEST.with(|diagnostic| diagnostic.borrow().clone())
}

macro_rules! metadata_capture_stage {
    ($stage:expr) => {
        #[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
        record_metadata_capture_stage_for_test($stage, None);
    };
    ($stage:expr, $path:expr) => {
        #[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
        record_metadata_capture_stage_for_test($stage, Some($path.to_string()));
    };
}

macro_rules! metadata_capture_detail {
    ($detail:expr) => {{
        #[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
        {
            record_metadata_capture_detail_for_test(format!("{:?}", $detail));
        }
        #[cfg(not(any(test, feature = "offline-nisar-subset-test-support")))]
        {
            let _ = || {
                let _ = &$detail;
            };
        }
    }};
}
#[cfg(any(feature = "nisar-subset-production", test))]
const V2_PUBLISH_PROC_FD_PREFIX: &str = "/proc/self/fd/";
#[cfg(any(feature = "nisar-subset-production", test))]
const V2_PUBLISH_LINK_FLAGS: i32 = 0x400;
#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
const _: () = assert!(V2_PUBLISH_LINK_FLAGS == libc::AT_SYMLINK_FOLLOW);
#[cfg(test)]
const V2_AT_EMPTY_PATH_FOR_AUDIT: i32 = 0x1000;

#[cfg(test)]
pub(crate) struct V2UnixPublishLinkContractForTest {
    pub(crate) source_prefix: &'static str,
    pub(crate) flags: i32,
    pub(crate) at_symlink_follow: i32,
    pub(crate) at_empty_path: i32,
}

#[cfg(test)]
pub(crate) fn v2_unix_publish_link_contract_for_test() -> V2UnixPublishLinkContractForTest {
    V2UnixPublishLinkContractForTest {
        source_prefix: V2_PUBLISH_PROC_FD_PREFIX,
        flags: V2_PUBLISH_LINK_FLAGS,
        at_symlink_follow: 0x400,
        at_empty_path: V2_AT_EMPTY_PATH_FOR_AUDIT,
    }
}

#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
struct CandidateFileIdentity {
    platform: String,
    device_id: u64,
    file_id: u64,
    size: u64,
}

#[cfg(feature = "nisar-subset-production")]
impl CandidateFileIdentity {
    fn same_object(&self, other: &Self) -> bool {
        self.platform == other.platform
            && self.device_id == other.device_id
            && self.file_id == other.file_id
    }
}

#[derive(Debug)]
struct CandidateFileGuard {
    path: PathBuf,
    file: fs::File,
    identity: CandidateFileIdentity,
}

#[cfg(all(windows, any(test, feature = "offline-nisar-subset-test-support")))]
static PINNED_TEST_SOURCES: std::sync::LazyLock<std::sync::Mutex<BTreeMap<PathBuf, fs::File>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(BTreeMap::new()));

/// Test-fixture ownership that keeps a Windows source path continuously pinned
/// across the HDF5 writer-to-reader handoff. The retained handle permits the
/// fixture's planned HDF5 mutations, but denies delete/rename sharing so an
/// external scanner cannot occupy the zero-handle close/open window.
#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
pub(crate) struct TestSourcePin {
    #[cfg(windows)]
    path: PathBuf,
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
impl TestSourcePin {
    pub(crate) fn acquire_while_hdf5_open(path: &Path) -> std::io::Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;

            let file = OpenOptions::new()
                .read(true)
                .share_mode(0x0000_0001 | 0x0000_0002)
                .custom_flags(0x0020_0000)
                .open(path)?;
            candidate_file_identity(&file)
                .map_err(|_| std::io::Error::other("unsafe HDF5 test source identity"))?;
            let mut pins = PINNED_TEST_SOURCES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if pins.contains_key(path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "HDF5 test source is already pinned",
                ));
            }
            pins.insert(path.to_path_buf(), file);
            Ok(Self {
                path: path.to_path_buf(),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Ok(Self {})
        }
    }
}

#[cfg(all(windows, any(test, feature = "offline-nisar-subset-test-support")))]
impl Drop for TestSourcePin {
    fn drop(&mut self) {
        PINNED_TEST_SOURCES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.path);
    }
}

#[cfg(all(windows, any(test, feature = "offline-nisar-subset-test-support")))]
fn clone_pinned_test_source(path: &Path) -> std::io::Result<Option<fs::File>> {
    PINNED_TEST_SOURCES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(path)
        .map(fs::File::try_clone)
        .transpose()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputEntryKind {
    Missing,
    Regular,
    Other,
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
pub(crate) enum NisarSubsetWriterRecoveryStateV2 {
    CandidateBuilding,
    CandidateValidated,
    ArtifactPromoted,
    Complete,
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetWriterRecoveryAnchorRecordV2 {
    contract: String,
    schema_version: u32,
    lineage_key_sha256: String,
    task_id: String,
    item_id: String,
    prepared_intent_sha256: String,
    final_semantic_plan_sha256: String,
    physical_plan_sha256: String,
    writer_selection_sha256: String,
    derived_artifact_id: String,
    artifact_file_name: String,
    candidate_identity: CandidateFileIdentity,
    candidate_size_bytes: Option<u64>,
    candidate_sha256: Option<String>,
    artifact_identity: Option<CandidateFileIdentity>,
    artifact_size_bytes: Option<u64>,
    artifact_sha256: Option<String>,
    state: NisarSubsetWriterRecoveryStateV2,
    record_sha256: String,
}

#[cfg(feature = "nisar-subset-production")]
impl NisarSubsetWriterRecoveryAnchorRecordV2 {
    const CONTRACT: &'static str = "NisarSubsetWriterRecoveryAnchorV2";
    const SCHEMA_VERSION: u32 = 1;

    pub(crate) fn lineage_key_sha256(&self) -> &str {
        &self.lineage_key_sha256
    }

    pub(crate) const fn state(&self) -> NisarSubsetWriterRecoveryStateV2 {
        self.state
    }

    pub(crate) fn from_canonical_json(canonical: &str) -> Result<Self, NisarSubsetWriteError> {
        if canonical.is_empty() || canonical.len() > MAX_METADATA_BYTES {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        let record: Self =
            serde_json::from_str(canonical).map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        record.validate_integrity()?;
        if record.canonical_json()? != canonical {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        Ok(record)
    }

    pub(crate) fn canonical_json(&self) -> Result<String, NisarSubsetWriteError> {
        self.validate_integrity()?;
        let canonical =
            serde_json::to_string(self).map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        if canonical.len() > MAX_METADATA_BYTES {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        Ok(canonical)
    }

    pub(crate) fn record_sha256(&self) -> &str {
        &self.record_sha256
    }

    fn recompute_sha256(&self) -> Result<String, NisarSubsetWriteError> {
        serde_json::to_vec(&(
            &self.contract,
            self.schema_version,
            &self.lineage_key_sha256,
            &self.task_id,
            &self.item_id,
            &self.prepared_intent_sha256,
            &self.final_semantic_plan_sha256,
            &self.physical_plan_sha256,
            &self.writer_selection_sha256,
            &self.derived_artifact_id,
            &self.artifact_file_name,
            (
                &self.candidate_identity,
                self.candidate_size_bytes,
                &self.candidate_sha256,
                &self.artifact_identity,
                self.artifact_size_bytes,
                &self.artifact_sha256,
                self.state,
            ),
        ))
        .map(|canonical| sha256_bytes(&canonical))
        .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)
    }

    fn validate_integrity(&self) -> Result<(), NisarSubsetWriteError> {
        let candidate_material_valid = self.candidate_size_bytes.is_some_and(|size| size > 0)
            && self
                .candidate_sha256
                .as_deref()
                .is_some_and(valid_v2_sha256);
        let artifact_material_valid = self.artifact_identity.is_some()
            && self.artifact_size_bytes.is_some_and(|size| size > 0)
            && self.artifact_sha256.as_deref().is_some_and(valid_v2_sha256);
        let state_valid = match self.state {
            NisarSubsetWriterRecoveryStateV2::CandidateBuilding => {
                self.candidate_size_bytes.is_none()
                    && self.candidate_sha256.is_none()
                    && self.artifact_identity.is_none()
                    && self.artifact_size_bytes.is_none()
                    && self.artifact_sha256.is_none()
            }
            NisarSubsetWriterRecoveryStateV2::CandidateValidated => {
                candidate_material_valid
                    && self.artifact_identity.is_none()
                    && self.artifact_size_bytes.is_none()
                    && self.artifact_sha256.is_none()
            }
            NisarSubsetWriterRecoveryStateV2::ArtifactPromoted
            | NisarSubsetWriterRecoveryStateV2::Complete => {
                candidate_material_valid
                    && artifact_material_valid
                    && self.candidate_size_bytes == self.artifact_size_bytes
                    && self.candidate_sha256 == self.artifact_sha256
            }
        };
        if self.contract != Self::CONTRACT
            || self.schema_version != Self::SCHEMA_VERSION
            || !valid_v2_sha256(&self.lineage_key_sha256)
            || !valid_v2_sha256(&self.writer_selection_sha256)
            || !valid_v2_sha256(&self.record_sha256)
            || self.task_id.is_empty()
            || self.item_id.is_empty()
            || self.derived_artifact_id.is_empty()
            || self.artifact_file_name.is_empty()
            || !state_valid
            || self.recompute_sha256()? != self.record_sha256
        {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        Ok(())
    }

    fn transition_complete(&self) -> Result<Self, NisarSubsetWriteError> {
        self.validate_integrity()?;
        if self.state != NisarSubsetWriterRecoveryStateV2::ArtifactPromoted {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        let mut replacement = self.clone();
        replacement.state = NisarSubsetWriterRecoveryStateV2::Complete;
        replacement.record_sha256 = replacement.recompute_sha256()?;
        replacement.validate_integrity()?;
        Ok(replacement)
    }
}

#[cfg(feature = "nisar-subset-production")]
pub(crate) mod writer_recovery_anchor_sealed {
    pub(crate) trait Sealed {}
}

#[cfg(feature = "nisar-subset-production")]
pub(crate) trait NisarSubsetWriterRecoveryAnchorV2:
    writer_recovery_anchor_sealed::Sealed + Send + Sync
{
    fn load(
        &self,
        lineage_key_sha256: &str,
    ) -> Result<Option<NisarSubsetWriterRecoveryAnchorRecordV2>, NisarSubsetWriteError>;

    fn compare_and_swap(
        &self,
        lineage_key_sha256: &str,
        expected: Option<&NisarSubsetWriterRecoveryAnchorRecordV2>,
        replacement: &NisarSubsetWriterRecoveryAnchorRecordV2,
    ) -> Result<bool, NisarSubsetWriteError>;
}

#[cfg(all(test, feature = "nisar-subset-production"))]
thread_local! {
    static V2_CANDIDATE_HASH_READ_FAULT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static V2_CANDIDATE_OPEN_FAULT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test-only one-shot faults on this thread: the next whole-file hash of a
/// candidate fails to read, or the next HDF5 open of a candidate fails.
#[cfg(all(test, feature = "nisar-subset-production"))]
pub(crate) struct V2CandidateFaultForTest(());

#[cfg(all(test, feature = "nisar-subset-production"))]
impl V2CandidateFaultForTest {
    pub(crate) fn hash_read() -> Self {
        V2_CANDIDATE_HASH_READ_FAULT.with(|fault| fault.set(true));
        Self(())
    }

    pub(crate) fn hdf5_open() -> Self {
        V2_CANDIDATE_OPEN_FAULT.with(|fault| fault.set(true));
        Self(())
    }
}

#[cfg(all(test, feature = "nisar-subset-production"))]
impl Drop for V2CandidateFaultForTest {
    fn drop(&mut self) {
        V2_CANDIDATE_HASH_READ_FAULT.with(|fault| fault.set(false));
        V2_CANDIDATE_OPEN_FAULT.with(|fault| fault.set(false));
    }
}

/// Review of slice 7 (P2-1): consecutive local read failures, in this
/// process, while a kept validated candidate is revalidated, by candidate
/// path. The count clears when the candidate validates again.
#[cfg(feature = "nisar-subset-production")]
static V2_KEPT_CANDIDATE_READ_FAILURES: std::sync::Mutex<std::collections::BTreeMap<PathBuf, u32>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// A kept candidate that cannot be read in this many consecutive
/// revalidations is removed and rebuilt from the verified science.
#[cfg(feature = "nisar-subset-production")]
const V2_KEPT_CANDIDATE_READ_FAILURE_LIMIT: u32 = 2;

#[cfg(feature = "nisar-subset-production")]
fn note_v2_kept_candidate_read_failure(path: &Path) -> u32 {
    let mut failures = V2_KEPT_CANDIDATE_READ_FAILURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let count = failures.entry(path.to_path_buf()).or_insert(0);
    *count = count.saturating_add(1);
    *count
}

#[cfg(feature = "nisar-subset-production")]
fn clear_v2_kept_candidate_read_failures(path: &Path) {
    V2_KEPT_CANDIDATE_READ_FAILURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(path);
}

/// A kept validated candidate could not be read (open, whole-file hash or
/// streamed validation). The failure is retryable; at the second consecutive
/// failure the anchor returns to CandidateBuilding first, and only then is
/// the candidate removed, so the next run rewrites it from the verified
/// science without any download. A CAS of unknown outcome keeps the
/// candidate: the next run sees either anchor and proceeds safely.
#[cfg(feature = "nisar-subset-production")]
fn v2_kept_candidate_unreadable(
    temp_path: &Path,
    anchored: &NisarSubsetWriterRecoveryAnchorRecordV2,
    recovery_anchor: &dyn NisarSubsetWriterRecoveryAnchorV2,
    recovery_lineage_key: &str,
    cleanup: &mut V2CandidateCleanup,
) -> NisarSubsetWriteError {
    if note_v2_kept_candidate_read_failure(temp_path) < V2_KEPT_CANDIDATE_READ_FAILURE_LIMIT {
        return NisarSubsetWriteError::Persistence;
    }
    let reset = match anchored.reset_unreadable_validated() {
        Ok(reset) => reset,
        Err(error) => return error,
    };
    match recovery_anchor.compare_and_swap(recovery_lineage_key, Some(anchored), &reset) {
        Ok(true) => {
            clear_v2_kept_candidate_read_failures(temp_path);
            cleanup.discard();
            NisarSubsetWriteError::Persistence
        }
        Ok(false) => NisarSubsetWriteError::RecoveryInvalid,
        Err(error) => error,
    }
}

#[cfg(feature = "nisar-subset-production")]
struct V2CandidateCleanup {
    path: PathBuf,
    expected_identity: Option<CandidateFileIdentity>,
    preserve_for_crash: bool,
}

#[cfg(feature = "nisar-subset-production")]
impl V2CandidateCleanup {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            expected_identity: None,
            preserve_for_crash: false,
        }
    }

    fn bind_identity(&mut self, identity: &CandidateFileIdentity) {
        self.expected_identity = Some(identity.clone());
    }

    fn preserve_for_crash(&mut self) {
        self.preserve_for_crash = true;
    }

    /// Reviews of Issue #8 slices 6 and 7: a candidate anchored as
    /// CandidateValidated is what the next run resumes from, so it is kept
    /// on every error by default (a pause, a read failure, a failed promotion,
    /// a CAS of unknown outcome, a science revalidation failure).
    fn keep(&mut self) {
        self.preserve_for_crash = true;
    }

    /// Removes a kept candidate after all: on proof that it is not the
    /// anchored candidate (its size or SHA-256 contradicts the anchor), or
    /// after its anchor was reset because it stayed unreadable. Only a file
    /// with the bound identity is ever removed.
    fn discard(&mut self) {
        self.preserve_for_crash = false;
    }
}

#[cfg(feature = "nisar-subset-production")]
impl Drop for V2CandidateCleanup {
    fn drop(&mut self) {
        if !self.preserve_for_crash {
            if let Some(expected) = &self.expected_identity {
                let _ = remove_candidate_if_identity_matches(&self.path, expected);
            }
        }
    }
}

#[cfg(feature = "nisar-subset-production")]
struct V2CandidateFileGuard {
    named_path: Option<PathBuf>,
    file: fs::File,
    identity: CandidateFileIdentity,
    publication: Option<PinnedDirectoryPublication>,
}

#[cfg(feature = "nisar-subset-production")]
impl V2CandidateFileGuard {
    fn open_named_immutable(path: &Path) -> Result<Self, NisarSubsetWriteError> {
        let file = open_immutable_v2_candidate_file(path)?;
        let identity = candidate_file_identity(&file)?;
        Ok(Self {
            named_path: Some(path.to_path_buf()),
            file,
            identity,
            publication: None,
        })
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn from_unnamed(file: fs::File) -> Result<Self, NisarSubsetWriteError> {
        let identity = unnamed_candidate_file_identity(&file)?;
        Ok(Self {
            named_path: None,
            file,
            identity,
            publication: None,
        })
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn from_published(
        file: fs::File,
        publication: &PinnedDirectoryPublication,
    ) -> Result<Self, NisarSubsetWriteError> {
        let identity = candidate_file_identity(&file)?;
        let guard = Self {
            named_path: None,
            file,
            identity,
            publication: Some(
                publication
                    .try_clone()
                    .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?,
            ),
        };
        guard.verify_identity()?;
        Ok(guard)
    }

    fn bind_publication(
        mut self,
        publication: &PinnedDirectoryPublication,
    ) -> Result<Self, NisarSubsetWriteError> {
        self.publication = Some(
            publication
                .try_clone()
                .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?,
        );
        self.verify_identity()?;
        Ok(self)
    }

    fn hdf5_path(&self) -> PathBuf {
        candidate_hdf5_path(
            self.named_path.as_deref().unwrap_or_else(|| Path::new("")),
            &self.file,
        )
    }

    fn sha256(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<String, NisarSubsetWriteError> {
        self.verify_identity()?;
        #[cfg(test)]
        if V2_CANDIDATE_HASH_READ_FAULT.with(|fault| fault.replace(false)) {
            return Err(NisarSubsetWriteError::Persistence);
        }
        let mut file = self
            .file
            .try_clone()
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        let digest = sha256_reader(&mut file, cancelled)?;
        self.verify_identity()?;
        Ok(digest)
    }

    fn verify_identity(&self) -> Result<(), NisarSubsetWriteError> {
        let retained = if self.identity.platform == "unix-unnamed" {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            {
                unnamed_candidate_file_identity(&self.file)?
            }
            #[cfg(not(any(target_os = "linux", target_os = "android")))]
            return Err(NisarSubsetWriteError::OutputUnsafe);
        } else {
            candidate_file_identity(&self.file)?
        };
        if retained != self.identity {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }

        if let Some(publication) = &self.publication {
            publication
                .verify()
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            #[cfg(any(windows, target_os = "macos"))]
            {
                let exact_path = publication
                    .exact_child_path()
                    .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
                if self.named_path.as_deref() != Some(exact_path.as_path()) {
                    return Err(NisarSubsetWriteError::RecoveryInvalid);
                }
                let reopened = open_candidate_file(&exact_path)?;
                if candidate_file_identity(&reopened)? != self.identity {
                    return Err(NisarSubsetWriteError::RecoveryInvalid);
                }
                return Ok(());
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            {
                let reopened = open_published_v2_child(publication)?;
                if candidate_file_identity(&reopened)? != self.identity {
                    return Err(NisarSubsetWriteError::RecoveryInvalid);
                }
                return Ok(());
            }
            #[cfg(not(any(
                windows,
                target_os = "linux",
                target_os = "android",
                target_os = "macos"
            )))]
            return Err(NisarSubsetWriteError::OutputUnsafe);
        }

        if let Some(path) = &self.named_path {
            let reopened = open_candidate_file(path)?;
            if candidate_file_identity(&reopened)? != self.identity {
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
        } else if self.identity.platform != "unix-unnamed" {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        Ok(())
    }
}

#[cfg(feature = "nisar-subset-production")]
struct V2CandidateBootstrap {
    named_path: Option<PathBuf>,
    file: Option<fs::File>,
    identity: CandidateFileIdentity,
}

#[cfg(feature = "nisar-subset-production")]
impl V2CandidateBootstrap {
    fn create(path: &Path) -> Result<Self, NisarSubsetWriteError> {
        create_v2_candidate_bootstrap(path)
    }

    fn identity(&self) -> &CandidateFileIdentity {
        &self.identity
    }

    fn hdf5_path(&self) -> PathBuf {
        #[cfg(any(windows, target_os = "macos"))]
        {
            self.named_path
                .clone()
                .expect("named bootstrap retains its stable path")
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            candidate_hdf5_path(
                self.named_path.as_deref().unwrap_or_else(|| Path::new("")),
                self.file.as_ref().expect("bootstrap descriptor retained"),
            )
        }
    }

    fn persist_for_write(&mut self) -> Result<(), NisarSubsetWriteError> {
        persist_v2_candidate_bootstrap(self)
    }

    fn sync(&self) -> Result<(), NisarSubsetWriteError> {
        sync_v2_candidate_bootstrap(self)
    }

    fn seal(self) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
        seal_v2_candidate_bootstrap(self)
    }
}

#[cfg(all(feature = "nisar-subset-production", target_os = "macos"))]
impl Drop for V2CandidateBootstrap {
    fn drop(&mut self) {
        if self.file.take().is_some() {
            if let Some(path) = &self.named_path {
                let _ = remove_candidate_if_identity_matches(path, &self.identity);
            }
        }
    }
}

impl CandidateFileGuard {
    fn open(path: &Path) -> Result<Self, NisarSubsetWriteError> {
        #[cfg(all(windows, any(test, feature = "offline-nisar-subset-test-support")))]
        if let Some(file) = clone_pinned_test_source(path).map_err(|error| {
            metadata_capture_detail!(format!("test source pin clone: {error:?}"));
            NisarSubsetWriteError::OutputUnsafe
        })? {
            metadata_capture_stage!("candidate_file.test_source_pin", path.display());
            let identity = candidate_file_identity(&file)?;
            return Ok(Self {
                path: path.to_path_buf(),
                file,
                identity,
            });
        }
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
        if let Some(opened) = open_descriptor_candidate(path) {
            metadata_capture_stage!("candidate_file.descriptor", path.display());
            let (file, identity) = opened?;
            file.try_lock()
                .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
            return Ok(Self {
                path: path.to_path_buf(),
                file,
                identity,
            });
        }
        metadata_capture_stage!("candidate_file.os_open", path.display());
        let file = open_candidate_file(path)?;
        #[cfg(not(windows))]
        file.try_lock()
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        metadata_capture_stage!("candidate_file.identity", path.display());
        let identity = candidate_file_identity(&file)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity,
        })
    }

    #[cfg(feature = "nisar-subset-production")]
    fn open_immutable_v2(path: &Path) -> Result<Self, NisarSubsetWriteError> {
        let file = open_immutable_v2_candidate_file(path)?;
        let identity = candidate_file_identity(&file)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity,
        })
    }

    fn hdf5_path(&self) -> PathBuf {
        candidate_hdf5_path(&self.path, &self.file)
    }

    fn sha256(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<String, NisarSubsetWriteError> {
        let mut file = self
            .file
            .try_clone()
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        let digest = sha256_reader(&mut file, cancelled)?;
        self.verify_path_identity()?;
        Ok(digest)
    }

    fn verify_path_identity(&self) -> Result<(), NisarSubsetWriteError> {
        #[cfg(all(windows, any(test, feature = "offline-nisar-subset-test-support")))]
        if let Some(reopened) =
            clone_pinned_test_source(&self.path).map_err(|_| NisarSubsetWriteError::OutputUnsafe)?
        {
            if candidate_file_identity(&reopened)? != self.identity {
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
            return Ok(());
        }
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
        if let Some(reopened) = open_descriptor_candidate(&self.path) {
            let (_, identity) = reopened?;
            if identity != self.identity {
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
            return Ok(());
        }
        let reopened = open_candidate_file(&self.path)?;
        if candidate_file_identity(&reopened)? != self.identity {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        Ok(())
    }
}

impl Drop for CandidateFileGuard {
    fn drop(&mut self) {
        #[cfg(not(windows))]
        {
            let _ = self.file.unlock();
        }
    }
}

/// The writer reads its pinned source view on Unix through `/proc/self/fd/<n>`
/// or `/dev/fd/<n>` of this process. That descriptor path names an open file,
/// not a pathname that can be swapped, so it is opened without `O_NOFOLLOW`
/// (which refuses every link). The file is then held to the identity rules
/// of its kind: an unnamed file must have no link, a named one exactly one.
/// `None` when `path` is not such a descriptor path.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn open_descriptor_candidate(
    path: &Path,
) -> Option<Result<(fs::File, CandidateFileIdentity), NisarSubsetWriteError>> {
    use std::os::unix::fs::MetadataExt;

    let text = path.to_str()?;
    let descriptor = text
        .strip_prefix("/proc/self/fd/")
        .or_else(|| text.strip_prefix("/dev/fd/"))?;
    if descriptor.is_empty() || !descriptor.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((|| {
        let file = OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let metadata = file
            .metadata()
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        if !metadata.is_file() {
            return Err(NisarSubsetWriteError::OutputUnsafe);
        }
        let identity = if metadata.nlink() == 0 {
            CandidateFileIdentity {
                platform: "unix-unnamed".to_owned(),
                device_id: metadata.dev(),
                file_id: metadata.ino(),
                size: metadata.len(),
            }
        } else {
            candidate_file_identity(&file)?
        };
        Ok((file, identity))
    })())
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, hdf5::H5Type)]
struct CFloat32 {
    r: f32,
    i: f32,
}

/// V2 validation compares complex samples by their bit patterns: a NaN source
/// sample must equal the same NaN in the artifact (derived `PartialEq` says
/// NaN != NaN), and `-0.0` must not pass for `0.0`.
#[cfg(feature = "nisar-subset-production")]
impl V2StreamElement for CFloat32 {
    fn same_bits(&self, other: &Self) -> bool {
        self.r.to_bits() == other.r.to_bits() && self.i.to_bits() == other.i.to_bits()
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NisarSubsetMetadataEvidenceV1 {
    pub(crate) mission_id: String,
    pub(crate) source_granule_id: String,
    pub(crate) bounding_polygon: String,
    pub(crate) source_zero_doppler_start_time: String,
    pub(crate) source_zero_doppler_end_time: String,
    pub(crate) selected_frequency: String,
    pub(crate) orbit_time: Vec<f64>,
    pub(crate) orbit_position: Vec<[f64; 3]>,
    pub(crate) orbit_velocity: Vec<[f64; 3]>,
    pub(crate) attitude_time: Vec<f64>,
    pub(crate) attitude_quaternions: Vec<[f64; 4]>,
    pub(crate) doppler_shape: [usize; 2],
    pub(crate) doppler_lut: Vec<f32>,
    pub(crate) calibration_shape: [usize; 2],
    pub(crate) calibration_lut: Vec<f32>,
    pub(crate) geolocation_shape: [usize; 3],
    pub(crate) geolocation_x: Vec<f64>,
    pub(crate) geolocation_y: Vec<f64>,
    pub(crate) height_above_ellipsoid: Vec<f64>,
    pub(crate) geolocation_epsg: i32,
    pub(crate) zero_doppler_time: Vec<f64>,
    pub(crate) slant_ranges: BTreeMap<String, Vec<f64>>,
    pub(crate) valid_samples: BTreeMap<String, Vec<[u32; 2]>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct SourceAttributeBindingV1 {
    name: String,
    datatype_sha256: String,
    shape: Vec<u64>,
    value_sha256: String,
    #[serde(skip)]
    datatype: TypeDescriptor,
    #[serde(skip)]
    datatype_encoded: Vec<u8>,
    #[serde(skip)]
    value: Arc<[u8]>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct SourceGroupBindingV1 {
    path: String,
    attributes: Vec<SourceAttributeBindingV1>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
enum SourceFilterBindingV1 {
    Shuffle,
    Deflate(u8),
    Fletcher32,
}

impl SourceFilterBindingV1 {
    fn to_filter(&self) -> Filter {
        match self {
            Self::Shuffle => Filter::Shuffle,
            Self::Deflate(level) => Filter::Deflate(*level),
            Self::Fletcher32 => Filter::Fletcher32,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
enum SourceDatasetActionV1 {
    PreserveExact,
    SelectedScience,
    FrequencyMask,
    ZeroDopplerTime,
    FrequencySlantRange,
    FrequencyValidSamples,
    DerivedIdentification,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct SourceMetadataDatasetBindingV1 {
    path: String,
    datatype_sha256: String,
    shape: Vec<u64>,
    chunk_shape: Option<Vec<u64>>,
    filters: Vec<SourceFilterBindingV1>,
    attributes: Vec<SourceAttributeBindingV1>,
    dimension_labels: Vec<String>,
    dimension_scales: Vec<Vec<String>>,
    scale_name: Option<String>,
    #[serde(skip)]
    is_dimension_scale: bool,
    action: SourceDatasetActionV1,
    value_sha256: Option<String>,
    #[serde(skip)]
    datatype: TypeDescriptor,
    #[serde(skip)]
    datatype_encoded: Vec<u8>,
    #[serde(skip)]
    value: Option<Arc<[u8]>>,
}

/// Immutable selection authority used by the shared HDF5 scanner for V2.
///
/// The values are copied only from a validated PreparedIntent. In particular,
/// this type has no conversion from the historical V1 task metadata fields.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NisarSubsetMetadataSelectionV2 {
    prepared_intent_sha256: String,
    selected_frequency: NisarSubsetFrequency,
    selected_polarizations: Vec<String>,
}

impl NisarSubsetMetadataSelectionV2 {
    pub(crate) fn from_prepared_intent(
        prepared_intent: &NisarSubsetPreparedIntentV2,
    ) -> Result<Self, NisarSubsetWriteError> {
        prepared_intent
            .validate()
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
        Ok(Self {
            prepared_intent_sha256: prepared_intent.canonical_intent_sha256.clone(),
            selected_frequency: prepared_intent.selected_frequency.clone(),
            selected_polarizations: prepared_intent.selected_polarizations.clone(),
        })
    }

    pub fn prepared_intent_sha256(&self) -> &str {
        &self.prepared_intent_sha256
    }

    pub fn selected_frequency(&self) -> &NisarSubsetFrequency {
        &self.selected_frequency
    }

    pub fn selected_polarizations(&self) -> &[String] {
        &self.selected_polarizations
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NisarSubsetVerifiedMetadataDatasetKindV2 {
    SelectedScience,
    ValidityMask,
    ValidSamples,
    Coordinate,
    DimensionScale,
    IdentificationMetadata,
    OrbitMetadata,
    AttitudeMetadata,
    ProcessingMetadata,
    CalibrationMetadata,
    GeolocationMetadata,
    ProvenanceMetadata,
    OtherMetadata,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NisarSubsetVerifiedMetadataAttributeV2 {
    name: String,
    datatype_sha256: String,
    shape: Vec<u64>,
    value_sha256: String,
}

impl NisarSubsetVerifiedMetadataAttributeV2 {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn datatype_sha256(&self) -> &str {
        &self.datatype_sha256
    }

    pub fn shape(&self) -> &[u64] {
        &self.shape
    }

    pub fn value_sha256(&self) -> &str {
        &self.value_sha256
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NisarSubsetVerifiedMetadataGroupV2 {
    path: String,
    attributes: Vec<NisarSubsetVerifiedMetadataAttributeV2>,
}

impl NisarSubsetVerifiedMetadataGroupV2 {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn attributes(&self) -> &[NisarSubsetVerifiedMetadataAttributeV2] {
        &self.attributes
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NisarSubsetVerifiedMetadataDatasetV2 {
    path: String,
    kind: NisarSubsetVerifiedMetadataDatasetKindV2,
    datatype_sha256: String,
    shape: Vec<u64>,
    attributes: Vec<NisarSubsetVerifiedMetadataAttributeV2>,
    dimension_labels: Vec<String>,
    dimension_scales: Vec<Vec<String>>,
    scale_name: Option<String>,
}

impl NisarSubsetVerifiedMetadataDatasetV2 {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn kind(&self) -> NisarSubsetVerifiedMetadataDatasetKindV2 {
        self.kind
    }

    pub fn datatype_sha256(&self) -> &str {
        &self.datatype_sha256
    }

    pub fn shape(&self) -> &[u64] {
        &self.shape
    }

    pub fn attributes(&self) -> &[NisarSubsetVerifiedMetadataAttributeV2] {
        &self.attributes
    }

    pub fn dimension_labels(&self) -> &[String] {
        &self.dimension_labels
    }

    pub fn dimension_scales(&self) -> &[Vec<String>] {
        &self.dimension_scales
    }

    pub fn scale_name(&self) -> Option<&str> {
        self.scale_name.as_deref()
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NisarSubsetVerifiedSelectedScienceV2 {
    frequency: NisarSubsetFrequency,
    polarization: String,
    source_path: String,
}

impl NisarSubsetVerifiedSelectedScienceV2 {
    pub fn frequency(&self) -> &NisarSubsetFrequency {
        &self.frequency
    }

    pub fn polarization(&self) -> &str {
        &self.polarization
    }

    pub fn source_path(&self) -> &str {
        &self.source_path
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct NisarSubsetVerifiedMetadataEvidenceV2 {
    prepared_intent_sha256: String,
    source_validator: NisarSubsetSourceValidatorV1,
    source_validator_sha256: String,
    source_size: u64,
    source_dataset_count: u64,
    inventory_sha256: String,
    evidence_sha256: String,
    evidence_authority: NisarSubsetDependencyEvidenceAuthorityV2,
}

impl NisarSubsetVerifiedMetadataEvidenceV2 {
    pub fn prepared_intent_sha256(&self) -> &str {
        &self.prepared_intent_sha256
    }

    pub fn source_validator(&self) -> &NisarSubsetSourceValidatorV1 {
        &self.source_validator
    }

    pub fn source_validator_sha256(&self) -> &str {
        &self.source_validator_sha256
    }

    pub fn source_size(&self) -> u64 {
        self.source_size
    }

    pub fn source_dataset_count(&self) -> u64 {
        self.source_dataset_count
    }

    pub fn inventory_sha256(&self) -> &str {
        &self.inventory_sha256
    }

    pub fn evidence_sha256(&self) -> &str {
        &self.evidence_sha256
    }

    pub fn evidence_authority(&self) -> NisarSubsetDependencyEvidenceAuthorityV2 {
        self.evidence_authority
    }
}

/// Selection-aware metadata authority captured from one real, read-only HDF5
/// source by the same scanner used by `NisarSubsetMetadataSnapshotV1`.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct NisarSubsetVerifiedMetadataInventoryV2 {
    selection: NisarSubsetMetadataSelectionV2,
    groups: Vec<NisarSubsetVerifiedMetadataGroupV2>,
    datasets: Vec<NisarSubsetVerifiedMetadataDatasetV2>,
    selected_science: Vec<NisarSubsetVerifiedSelectedScienceV2>,
    validity_mask_paths: Vec<String>,
    valid_samples_paths: Vec<String>,
    identification_metadata_paths: Vec<String>,
    provenance_metadata_paths: Vec<String>,
    evidence: NisarSubsetVerifiedMetadataEvidenceV2,
}

impl NisarSubsetVerifiedMetadataInventoryV2 {
    pub fn from_verified_offline_metadata_hdf5(
        prepared_intent: &NisarSubsetPreparedIntentV2,
        source_validator: NisarSubsetSourceValidatorV1,
        source_path: &Path,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Self, NisarSubsetWriteError> {
        capture_verified_metadata_inventory_v2(
            prepared_intent,
            source_validator,
            source_path,
            cancelled,
            true,
        )
    }

    #[cfg(feature = "nisar-subset-production")]
    pub fn from_verified_live_metadata_hdf5(
        prepared_intent: &NisarSubsetPreparedIntentV2,
        source_validator: NisarSubsetSourceValidatorV1,
        source_path: &Path,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Self, NisarSubsetWriteError> {
        capture_verified_metadata_inventory_v2(
            prepared_intent,
            source_validator,
            source_path,
            cancelled,
            false,
        )
    }

    pub fn selection(&self) -> &NisarSubsetMetadataSelectionV2 {
        &self.selection
    }

    pub fn groups(&self) -> &[NisarSubsetVerifiedMetadataGroupV2] {
        &self.groups
    }

    pub fn datasets(&self) -> &[NisarSubsetVerifiedMetadataDatasetV2] {
        &self.datasets
    }

    pub fn selected_science(&self) -> &[NisarSubsetVerifiedSelectedScienceV2] {
        &self.selected_science
    }

    pub fn validity_mask_paths(&self) -> &[String] {
        &self.validity_mask_paths
    }

    pub fn valid_samples_paths(&self) -> &[String] {
        &self.valid_samples_paths
    }

    pub fn identification_metadata_paths(&self) -> &[String] {
        &self.identification_metadata_paths
    }

    pub fn provenance_metadata_paths(&self) -> &[String] {
        &self.provenance_metadata_paths
    }

    pub fn evidence(&self) -> &NisarSubsetVerifiedMetadataEvidenceV2 {
        &self.evidence
    }
}

#[derive(Clone, Debug)]
struct SourceMetadataSelection {
    expected_file_name: String,
    product_specification_version: String,
    primary_frequency: NisarSubsetFrequency,
    science_root: NisarScienceRoot,
    selected_frequency_labels: Vec<String>,
    selected_science: Vec<NisarSubsetVerifiedSelectedScienceV2>,
    mask_paths: Vec<String>,
    required_mask_paths: Vec<String>,
}

impl SourceMetadataSelection {
    fn from_v1(request: &NisarSubsetRequestManifestV1) -> Self {
        let selected_frequency_labels = request
            .mapping
            .windows
            .iter()
            .map(|window| frequency_label(&window.frequency).to_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let selected_science = request
            .mapping
            .windows
            .iter()
            .map(|window| NisarSubsetVerifiedSelectedScienceV2 {
                frequency: window.frequency.clone(),
                polarization: window.polarization.clone(),
                source_path: window.dataset_path.clone(),
            })
            .collect::<Vec<_>>();
        let mask_paths = request
            .mapping
            .windows
            .iter()
            .map(|window| source_mask_path(&window.frequency))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let required_mask_paths = request
            .mapping
            .windows
            .iter()
            .filter(|window| window.validity_mask_required)
            .map(|window| source_mask_path(&window.frequency))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Self {
            expected_file_name: request.source.file_name.clone(),
            product_specification_version: request.source.product_specification_version.clone(),
            primary_frequency: request.request.frequency.clone(),
            science_root: NisarScienceRoot::Lsar,
            selected_frequency_labels,
            selected_science,
            mask_paths,
            required_mask_paths,
        }
    }

    fn from_v2(
        prepared_intent: &NisarSubsetPreparedIntentV2,
    ) -> Result<Self, NisarSubsetWriteError> {
        let frequency = prepared_intent.selected_frequency.clone();
        let frequency_label = frequency_label(&frequency).to_owned();
        let science_root =
            NisarScienceRoot::from_source_identity(&prepared_intent.source_product_identity)
                .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
        let root = format!("{}/frequency{frequency_label}", science_root.swaths_path());
        Ok(Self {
            expected_file_name: prepared_intent.source_asset_identity.file_name.clone(),
            product_specification_version: prepared_intent
                .source_product_identity
                .product_specification_version
                .clone(),
            primary_frequency: frequency.clone(),
            science_root,
            selected_frequency_labels: vec![frequency_label],
            selected_science: prepared_intent
                .selected_polarizations
                .iter()
                .map(|polarization| NisarSubsetVerifiedSelectedScienceV2 {
                    frequency: frequency.clone(),
                    polarization: polarization.clone(),
                    source_path: format!("{root}/{polarization}"),
                })
                .collect(),
            mask_paths: vec![format!("{root}/inputDataExceptionMask")],
            required_mask_paths: vec![format!("{root}/inputDataExceptionMask")],
        })
    }
}

type SourceDimensionScaleGraphV1 = (Vec<String>, Vec<Vec<String>>, Option<String>, bool);

#[derive(Default)]
struct MetadataReadBudget {
    bytes: usize,
}

impl MetadataReadBudget {
    fn charge(&mut self, bytes: usize) -> Result<(), NisarSubsetWriteError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|bytes| *bytes <= MAX_METADATA_BYTES)
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct NisarSubsetMetadataSnapshotV1 {
    request_manifest_sha256: String,
    source_identity: NisarSubsetSourceIdentityV1,
    source_validator: NisarSubsetSourceValidatorV1,
    group_bindings: Vec<SourceGroupBindingV1>,
    dataset_bindings: Vec<SourceMetadataDatasetBindingV1>,
    source_dataset_count: u64,
    source_inventory_sha256: String,
    evidence_sha256: String,
    evidence: NisarSubsetMetadataEvidenceV1,
}

impl NisarSubsetMetadataSnapshotV1 {
    /// Captures bounded, immutable metadata evidence from an already reviewed
    /// offline HDF5 source. This intentionally does not retain the source file
    /// or scan its full science payload: source continuity during transfer is
    /// owned by the strong validator and the verified Range workspace.
    pub fn from_verified_offline_metadata_hdf5(
        request: &NisarSubsetRequestManifestV1,
        source_validator: NisarSubsetSourceValidatorV1,
        source_path: &Path,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Self, NisarSubsetWriteError> {
        capture_verified_metadata_snapshot(request, source_validator, source_path, cancelled, true)
    }

    /// Captures the same bounded metadata evidence from the sparse live Range
    /// workspace. The sparse file is required to have the full logical source
    /// length and the frozen strong validator, but it is intentionally not
    /// hashed as a complete source because only the approved metadata prefix
    /// and selected raw chunks have been received.
    #[cfg(feature = "nisar-subset-production")]
    #[doc(hidden)]
    pub fn from_verified_live_metadata_hdf5(
        request: &NisarSubsetRequestManifestV1,
        source_validator: NisarSubsetSourceValidatorV1,
        source_path: &Path,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Self, NisarSubsetWriteError> {
        capture_verified_metadata_snapshot(request, source_validator, source_path, cancelled, false)
    }

    pub fn evidence_sha256(&self) -> &str {
        &self.evidence_sha256
    }

    /// Bounded digest of the source metadata inventory (group + dataset
    /// bindings). This is the `metadata inventory SHA-256` acceptance value.
    pub fn source_inventory_sha256(&self) -> &str {
        &self.source_inventory_sha256
    }

    /// Actual HDF5 dataset-object count observed while walking the source.
    /// This is intentionally distinct from the bounded metadata bindings
    /// retained for output reconstruction.
    pub fn source_dataset_count(&self) -> u64 {
        self.source_dataset_count
    }

    pub fn metadata_group_binding_count(&self) -> u64 {
        self.group_bindings.len() as u64
    }

    pub fn metadata_dataset_binding_count(&self) -> u64 {
        self.dataset_bindings.len() as u64
    }

    pub fn metadata_binding_count(&self) -> u64 {
        self.metadata_group_binding_count()
            .saturating_add(self.metadata_dataset_binding_count())
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn request_manifest_sha256(&self) -> &str {
        &self.request_manifest_sha256
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn source_identity(&self) -> &NisarSubsetSourceIdentityV1 {
        &self.source_identity
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn source_validator(&self) -> &NisarSubsetSourceValidatorV1 {
        &self.source_validator
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    pub(crate) fn oracle_evidence(&self) -> &NisarSubsetMetadataEvidenceV1 {
        &self.evidence
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    pub(crate) fn oracle_geolocation_axis(
        &self,
        swath: &str,
        zero_doppler_time: bool,
    ) -> Result<Vec<f64>, NisarSubsetWriteError> {
        let shape = self.evidence.geolocation_shape;
        let expected = if zero_doppler_time {
            shape[1]
        } else {
            shape[2]
        };
        let science_root =
            nisar_science_root_for_oracle(swath).ok_or(NisarSubsetWriteError::MetadataInvalid)?;
        let geolocation_path = format!("{science_root}/RSLC/metadata/geolocationGrid");
        let path = if zero_doppler_time {
            format!("{geolocation_path}/zeroDopplerTime")
        } else {
            format!("{geolocation_path}/slantRange")
        };
        let binding = self
            .dataset_bindings
            .iter()
            .find(|binding| binding.path == path)
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
        if binding.shape != [expected as u64]
            || binding.datatype != TypeDescriptor::Float(hdf5::types::FloatSize::U8)
        {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        let bytes = binding
            .value
            .as_deref()
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
        if bytes.len() != expected.saturating_mul(std::mem::size_of::<f64>()) {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        Ok(bytes
            .chunks_exact(8)
            .map(|chunk| f64::from_ne_bytes(chunk.try_into().expect("exact f64 chunk")))
            .collect())
    }
}

fn capture_verified_metadata_snapshot(
    request: &NisarSubsetRequestManifestV1,
    source_validator: NisarSubsetSourceValidatorV1,
    source_path: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
    verify_complete_source_checksum: bool,
) -> Result<NisarSubsetMetadataSnapshotV1, NisarSubsetWriteError> {
    metadata_capture_stage!("request.validate");
    request
        .validate()
        .map_err(|_| NisarSubsetWriteError::RequestInvalid)?;
    metadata_capture_stage!("candidate_file.open", source_path.display());
    let source = Arc::new(
        CandidateFileGuard::open(source_path)
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
    );
    if verify_complete_source_checksum {
        metadata_capture_stage!("source_identity.checksum", source_path.display());
        validate_metadata_source_identity(request, &source_validator, &source, cancelled)?;
    } else {
        #[cfg(feature = "nisar-subset-production")]
        validate_live_metadata_source_identity(request, &source_validator, &source)?;
        #[cfg(not(feature = "nisar-subset-production"))]
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let selection = SourceMetadataSelection::from_v1(request);
    metadata_capture_stage!("metadata_scan.read_source_metadata", source_path.display());
    let (evidence, group_bindings, dataset_bindings, source_dataset_count) =
        read_source_metadata(&selection, &source.hdf5_path(), cancelled)?;
    metadata_capture_stage!("metadata_scan.dataset_count");
    if source_dataset_count == 0 || source_dataset_count > MAX_SUPPORTED_SOURCE_DATASET_COUNT {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    metadata_capture_stage!("source_identity.revalidate_path", source_path.display());
    source
        .verify_path_identity()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    drop(source);
    metadata_capture_stage!("metadata_evidence.validate");
    validate_metadata_evidence(&evidence)?;
    #[derive(Serialize)]
    struct Identity<'a> {
        request_manifest_sha256: &'a str,
        source_identity: &'a NisarSubsetSourceIdentityV1,
        source_validator: &'a NisarSubsetSourceValidatorV1,
        groups: &'a [SourceGroupBindingV1],
        datasets: &'a [SourceMetadataDatasetBindingV1],
        evidence: &'a NisarSubsetMetadataEvidenceV1,
    }
    let request_manifest_sha256 = request
        .sha256()
        .map_err(|_| NisarSubsetWriteError::RequestInvalid)?;
    metadata_capture_stage!("inventory.canonicalize");
    let source_inventory_sha256 = sha256_bytes(
        &serde_json::to_vec(&(&group_bindings, &dataset_bindings))
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
    );
    metadata_capture_stage!("evidence.canonicalize");
    let canonical = serde_json::to_vec(&Identity {
        request_manifest_sha256: &request_manifest_sha256,
        source_identity: &request.source,
        source_validator: &source_validator,
        groups: &group_bindings,
        datasets: &dataset_bindings,
        evidence: &evidence,
    })
    .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    Ok(NisarSubsetMetadataSnapshotV1 {
        request_manifest_sha256,
        source_identity: request.source.clone(),
        source_validator,
        group_bindings,
        dataset_bindings,
        source_dataset_count: source_dataset_count as u64,
        source_inventory_sha256,
        evidence_sha256: sha256_bytes(&canonical),
        evidence,
    })
}

fn capture_verified_metadata_inventory_v2(
    prepared_intent: &NisarSubsetPreparedIntentV2,
    source_validator: NisarSubsetSourceValidatorV1,
    source_path: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
    verify_complete_source_checksum: bool,
) -> Result<NisarSubsetVerifiedMetadataInventoryV2, NisarSubsetWriteError> {
    let selection = NisarSubsetMetadataSelectionV2::from_prepared_intent(prepared_intent)?;
    let scanner_selection = SourceMetadataSelection::from_v2(prepared_intent)?;
    let source = Arc::new(
        CandidateFileGuard::open(source_path)
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
    );
    validate_metadata_source_identity_v2(
        prepared_intent,
        &source_validator,
        &source,
        cancelled,
        verify_complete_source_checksum,
    )?;
    let (_, group_bindings, dataset_bindings, source_dataset_count) =
        read_source_metadata(&scanner_selection, &source.hdf5_path(), cancelled)?;
    if source_dataset_count == 0 || source_dataset_count > MAX_SUPPORTED_SOURCE_DATASET_COUNT {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    source
        .verify_path_identity()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let source_size = source.identity.size;
    drop(source);

    let groups = group_bindings
        .iter()
        .map(|binding| NisarSubsetVerifiedMetadataGroupV2 {
            path: binding.path.clone(),
            attributes: verified_attributes(&binding.attributes),
        })
        .collect::<Vec<_>>();
    let datasets = dataset_bindings
        .iter()
        .map(|binding| NisarSubsetVerifiedMetadataDatasetV2 {
            path: binding.path.clone(),
            kind: verified_dataset_kind(binding, scanner_selection.science_root),
            datatype_sha256: binding.datatype_sha256.clone(),
            shape: binding.shape.clone(),
            attributes: verified_attributes(&binding.attributes),
            dimension_labels: binding.dimension_labels.clone(),
            dimension_scales: binding.dimension_scales.clone(),
            scale_name: binding.scale_name.clone(),
        })
        .collect::<Vec<_>>();
    let validity_mask_paths = datasets
        .iter()
        .filter(|dataset| dataset.kind == NisarSubsetVerifiedMetadataDatasetKindV2::ValidityMask)
        .map(|dataset| dataset.path.clone())
        .collect::<Vec<_>>();
    let valid_samples_paths = datasets
        .iter()
        .filter(|dataset| dataset.kind == NisarSubsetVerifiedMetadataDatasetKindV2::ValidSamples)
        .map(|dataset| dataset.path.clone())
        .collect::<Vec<_>>();
    let identification_metadata_paths = datasets
        .iter()
        .filter(|dataset| {
            dataset.kind == NisarSubsetVerifiedMetadataDatasetKindV2::IdentificationMetadata
        })
        .map(|dataset| dataset.path.clone())
        .collect::<Vec<_>>();
    let provenance_metadata_paths = datasets
        .iter()
        .filter(|dataset| {
            matches!(
                dataset.kind,
                NisarSubsetVerifiedMetadataDatasetKindV2::ProcessingMetadata
                    | NisarSubsetVerifiedMetadataDatasetKindV2::ProvenanceMetadata
            )
        })
        .map(|dataset| dataset.path.clone())
        .collect::<Vec<_>>();

    #[derive(Serialize)]
    struct InventoryIdentity<'a> {
        selection: &'a NisarSubsetMetadataSelectionV2,
        groups: &'a [NisarSubsetVerifiedMetadataGroupV2],
        datasets: &'a [NisarSubsetVerifiedMetadataDatasetV2],
        selected_science: &'a [NisarSubsetVerifiedSelectedScienceV2],
        validity_mask_paths: &'a [String],
        valid_samples_paths: &'a [String],
    }
    let inventory_sha256 = sha256_bytes(
        &serde_json::to_vec(&InventoryIdentity {
            selection: &selection,
            groups: &groups,
            datasets: &datasets,
            selected_science: &scanner_selection.selected_science,
            validity_mask_paths: &validity_mask_paths,
            valid_samples_paths: &valid_samples_paths,
        })
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
    );
    let source_validator_sha256 = sha256_bytes(
        &serde_json::to_vec(&source_validator)
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
    );
    let evidence_authority = NisarSubsetDependencyEvidenceAuthorityV2::Hdf5MetadataInventoryV1;
    let evidence_sha256 = verified_metadata_evidence_sha256(
        &inventory_sha256,
        selection.prepared_intent_sha256(),
        &source_validator,
        source_size,
        source_dataset_count as u64,
        evidence_authority,
    )?;
    Ok(NisarSubsetVerifiedMetadataInventoryV2 {
        selection: selection.clone(),
        groups,
        datasets,
        selected_science: scanner_selection.selected_science,
        validity_mask_paths,
        valid_samples_paths,
        identification_metadata_paths,
        provenance_metadata_paths,
        evidence: NisarSubsetVerifiedMetadataEvidenceV2 {
            prepared_intent_sha256: selection.prepared_intent_sha256,
            source_validator,
            source_validator_sha256,
            source_size,
            source_dataset_count: source_dataset_count as u64,
            inventory_sha256,
            evidence_sha256,
            evidence_authority,
        },
    })
}

fn verified_metadata_evidence_sha256(
    inventory_sha256: &str,
    prepared_intent_sha256: &str,
    source_validator: &NisarSubsetSourceValidatorV1,
    source_size: u64,
    source_dataset_count: u64,
    evidence_authority: NisarSubsetDependencyEvidenceAuthorityV2,
) -> Result<String, NisarSubsetWriteError> {
    #[derive(Serialize)]
    struct EvidenceIdentity<'a> {
        inventory_sha256: &'a str,
        prepared_intent_sha256: &'a str,
        source_validator: &'a NisarSubsetSourceValidatorV1,
        source_size: u64,
        source_dataset_count: u64,
        evidence_authority: NisarSubsetDependencyEvidenceAuthorityV2,
    }
    serde_json::to_vec(&EvidenceIdentity {
        inventory_sha256,
        prepared_intent_sha256,
        source_validator,
        source_size,
        source_dataset_count,
        evidence_authority,
    })
    .map(|canonical| sha256_bytes(&canonical))
    .map_err(|_| NisarSubsetWriteError::MetadataInvalid)
}

fn validate_metadata_source_identity_v2(
    prepared_intent: &NisarSubsetPreparedIntentV2,
    source_validator: &NisarSubsetSourceValidatorV1,
    source: &CandidateFileGuard,
    cancelled: &(dyn Fn() -> bool + Sync),
    verify_complete_source_checksum: bool,
) -> Result<(), NisarSubsetWriteError> {
    if source_validator.kind != NisarSubsetValidatorKind::StrongEtag
        || source_validator.value.starts_with("W/")
        || !source_validator.value.starts_with('"')
        || !source_validator.value.ends_with('"')
        || source_validator.value.len() < 3
        || source_validator.content_length != source.identity.size
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    if verify_complete_source_checksum {
        let checksum = prepared_intent
            .source_asset_identity
            .checksum
            .as_ref()
            .filter(|checksum| checksum.algorithm == NisarSubsetChecksumAlgorithm::Sha256)
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
        let actual_sha256 = source.sha256(cancelled).map_err(|error| match error {
            NisarSubsetWriteError::Cancelled => NisarSubsetWriteError::Cancelled,
            _ => NisarSubsetWriteError::MetadataInvalid,
        })?;
        if actual_sha256 != checksum.value {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
    }
    Ok(())
}

fn verified_attributes(
    attributes: &[SourceAttributeBindingV1],
) -> Vec<NisarSubsetVerifiedMetadataAttributeV2> {
    attributes
        .iter()
        .map(|attribute| NisarSubsetVerifiedMetadataAttributeV2 {
            name: attribute.name.clone(),
            datatype_sha256: attribute.datatype_sha256.clone(),
            shape: attribute.shape.clone(),
            value_sha256: attribute.value_sha256.clone(),
        })
        .collect()
}

fn verified_dataset_kind(
    binding: &SourceMetadataDatasetBindingV1,
    science_root: NisarScienceRoot,
) -> NisarSubsetVerifiedMetadataDatasetKindV2 {
    if binding.is_dimension_scale {
        return NisarSubsetVerifiedMetadataDatasetKindV2::DimensionScale;
    }
    match binding.action {
        SourceDatasetActionV1::SelectedScience => {
            NisarSubsetVerifiedMetadataDatasetKindV2::SelectedScience
        }
        SourceDatasetActionV1::FrequencyMask => {
            NisarSubsetVerifiedMetadataDatasetKindV2::ValidityMask
        }
        SourceDatasetActionV1::FrequencyValidSamples => {
            NisarSubsetVerifiedMetadataDatasetKindV2::ValidSamples
        }
        SourceDatasetActionV1::ZeroDopplerTime | SourceDatasetActionV1::FrequencySlantRange => {
            NisarSubsetVerifiedMetadataDatasetKindV2::Coordinate
        }
        SourceDatasetActionV1::DerivedIdentification => {
            NisarSubsetVerifiedMetadataDatasetKindV2::IdentificationMetadata
        }
        SourceDatasetActionV1::PreserveExact => {
            let identification_path = science_root.identification_path();
            let orbit_path = science_root.orbit_path();
            let attitude_path = science_root.attitude_path();
            let processing_path = science_root.processing_path();
            let calibration_path = science_root.calibration_path();
            let geolocation_path = science_root.geolocation_path();
            let provenance_path = science_root.provenance_path();
            if binding.path.starts_with(&identification_path) {
                NisarSubsetVerifiedMetadataDatasetKindV2::IdentificationMetadata
            } else if binding.path.starts_with(&orbit_path) {
                NisarSubsetVerifiedMetadataDatasetKindV2::OrbitMetadata
            } else if binding.path.starts_with(&attitude_path) {
                NisarSubsetVerifiedMetadataDatasetKindV2::AttitudeMetadata
            } else if binding.path.starts_with(&processing_path) {
                NisarSubsetVerifiedMetadataDatasetKindV2::ProcessingMetadata
            } else if binding.path.starts_with(&calibration_path) {
                NisarSubsetVerifiedMetadataDatasetKindV2::CalibrationMetadata
            } else if binding.path == format!("{geolocation_path}/coordinateX")
                || binding.path == format!("{geolocation_path}/coordinateY")
            {
                NisarSubsetVerifiedMetadataDatasetKindV2::Coordinate
            } else if binding.path.starts_with(&geolocation_path) {
                NisarSubsetVerifiedMetadataDatasetKindV2::GeolocationMetadata
            } else if binding.path.starts_with(&provenance_path) {
                NisarSubsetVerifiedMetadataDatasetKindV2::ProvenanceMetadata
            } else {
                NisarSubsetVerifiedMetadataDatasetKindV2::OtherMetadata
            }
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NisarSubsetSourceDependencyGraphBuildErrorV2 {
    #[error("verified metadata inventory evidence is invalid")]
    EvidenceInvalid,
    #[error("verified metadata inventory selection is inconsistent")]
    SelectionMismatch,
    #[error("required verified dependency is missing: {0}")]
    MissingDependency(String),
    #[error("verified dependency node has conflicting semantics: {0}")]
    ConflictingNode(String),
    #[error("derived source dependency graph is invalid")]
    GraphInvalid,
}

fn metadata_path_is_selected_or_shared(path: &str, selected: &NisarSubsetFrequency) -> bool {
    let mut frequency_a = false;
    let mut frequency_b = false;
    for component in path.split('/') {
        frequency_a |= component == "frequencyA";
        frequency_b |= component == "frequencyB";
    }
    matches!(
        (frequency_a, frequency_b, selected),
        (false, false, _)
            | (true, false, NisarSubsetFrequency::A)
            | (false, true, NisarSubsetFrequency::B)
    )
}

/// Builds the sole production V2 dependency graph from a sealed inventory.
///
/// This boundary performs no file or network I/O. Every graph node is resolved
/// to an object already captured by the shared HDF5 scanner, and the graph is
/// admitted by Core's canonical closure validator before it is returned.
pub fn build_nisar_subset_source_dependency_graph_v2(
    inventory: &NisarSubsetVerifiedMetadataInventoryV2,
) -> Result<NisarSubsetSourceDependencyGraphV2, NisarSubsetSourceDependencyGraphBuildErrorV2> {
    use NisarSubsetDependencyRelationV2 as Relation;
    use NisarSubsetSourceDependencyNodeKindV2 as NodeKind;
    use NisarSubsetVerifiedMetadataDatasetKindV2 as InventoryKind;

    let selection = inventory.selection();
    let evidence = inventory.evidence();
    let expected_evidence_sha256 = verified_metadata_evidence_sha256(
        evidence.inventory_sha256(),
        selection.prepared_intent_sha256(),
        evidence.source_validator(),
        evidence.source_size(),
        evidence.source_dataset_count(),
        evidence.evidence_authority(),
    )
    .map_err(|_| NisarSubsetSourceDependencyGraphBuildErrorV2::EvidenceInvalid)?;
    if evidence.prepared_intent_sha256() != selection.prepared_intent_sha256()
        || evidence.inventory_sha256().len() != 64
        || evidence.evidence_sha256().len() != 64
        || expected_evidence_sha256 != evidence.evidence_sha256()
    {
        return Err(NisarSubsetSourceDependencyGraphBuildErrorV2::EvidenceInvalid);
    }
    let science_root = if inventory
        .selected_science()
        .iter()
        .all(|science| science.source_path().starts_with("/science/LSAR/"))
    {
        NisarScienceRoot::Lsar
    } else if inventory
        .selected_science()
        .iter()
        .all(|science| science.source_path().starts_with("/science/SSAR/"))
    {
        NisarScienceRoot::Ssar
    } else {
        return Err(NisarSubsetSourceDependencyGraphBuildErrorV2::SelectionMismatch);
    };
    if inventory.selected_science().is_empty() {
        return Err(NisarSubsetSourceDependencyGraphBuildErrorV2::SelectionMismatch);
    }
    let frequency = frequency_label(selection.selected_frequency());
    let frequency_root = format!("{}/frequency{frequency}", science_root.swaths_path());
    let expected_science = selection
        .selected_polarizations()
        .iter()
        .map(|polarization| {
            (
                selection.selected_frequency().clone(),
                polarization.clone(),
                format!("{frequency_root}/{polarization}"),
            )
        })
        .collect::<BTreeSet<_>>();
    let actual_science = inventory
        .selected_science()
        .iter()
        .map(|science| {
            (
                science.frequency().clone(),
                science.polarization().to_owned(),
                science.source_path().to_owned(),
            )
        })
        .collect::<BTreeSet<_>>();
    let selected_dataset_paths = inventory
        .datasets()
        .iter()
        .filter(|dataset| dataset.kind() == InventoryKind::SelectedScience)
        .map(|dataset| dataset.path().to_owned())
        .collect::<BTreeSet<_>>();
    let expected_science_paths = expected_science
        .iter()
        .map(|(_, _, path)| path.clone())
        .collect::<BTreeSet<_>>();
    if actual_science != expected_science || selected_dataset_paths != expected_science_paths {
        return Err(NisarSubsetSourceDependencyGraphBuildErrorV2::SelectionMismatch);
    }

    let datasets = inventory
        .datasets()
        .iter()
        .map(|dataset| (dataset.path(), dataset))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = BTreeMap::<String, NodeKind>::new();
    let mut direct_dependencies = Vec::<(String, NodeKind, Relation)>::new();
    for path in &expected_science_paths {
        if !datasets.contains_key(path.as_str()) {
            return Err(
                NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(path.clone()),
            );
        }
        nodes.insert(path.clone(), NodeKind::ScienceDataset);
    }

    let exact_mask_path = format!("{frequency_root}/inputDataExceptionMask");
    match datasets.get(exact_mask_path.as_str()) {
        Some(dataset) if dataset.kind() == InventoryKind::ValidityMask => {}
        _ => {
            return Err(
                NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(exact_mask_path),
            );
        }
    }
    direct_dependencies.push((
        exact_mask_path,
        NodeKind::ValidityMask,
        Relation::ValidityMask,
    ));

    let geolocation_path = science_root.geolocation_path();
    for path in [
        science_root.zero_doppler_time_path(),
        format!("{frequency_root}/slantRange"),
        format!("{geolocation_path}/coordinateX"),
        format!("{geolocation_path}/coordinateY"),
    ] {
        match datasets.get(path.as_str()) {
            Some(dataset) if dataset.kind() == InventoryKind::Coordinate => {}
            _ => {
                return Err(NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(path));
            }
        }
        direct_dependencies.push((path, NodeKind::CoordinateDataset, Relation::Coordinate));
    }

    let attribute_owner = inventory
        .groups()
        .iter()
        .find(|group| {
            group.path() == "/"
                && group
                    .attributes()
                    .iter()
                    .any(|attribute| attribute.name() == "Conventions")
        })
        .ok_or_else(|| {
            NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(
                "verified root Conventions attribute owner".to_owned(),
            )
        })?;
    direct_dependencies.push((
        attribute_owner.path().to_owned(),
        NodeKind::Attribute,
        Relation::RequiredAttribute,
    ));

    let mut identification_count = 0_usize;
    let mut provenance_count = 0_usize;
    let mut other_scientific_count = 0_usize;
    for dataset in inventory.datasets() {
        match dataset.kind() {
            InventoryKind::IdentificationMetadata => {
                identification_count += 1;
                direct_dependencies.push((
                    dataset.path().to_owned(),
                    NodeKind::IdentificationMetadata,
                    Relation::RequiredMetadata,
                ));
            }
            InventoryKind::ProcessingMetadata | InventoryKind::ProvenanceMetadata
                if metadata_path_is_selected_or_shared(
                    dataset.path(),
                    selection.selected_frequency(),
                ) =>
            {
                provenance_count += 1;
                direct_dependencies.push((
                    dataset.path().to_owned(),
                    NodeKind::ProvenanceMetadata,
                    Relation::Provenance,
                ));
            }
            InventoryKind::ValidSamples
            | InventoryKind::OrbitMetadata
            | InventoryKind::AttitudeMetadata
            | InventoryKind::CalibrationMetadata
            | InventoryKind::GeolocationMetadata
                if metadata_path_is_selected_or_shared(
                    dataset.path(),
                    selection.selected_frequency(),
                ) =>
            {
                other_scientific_count += 1;
                direct_dependencies.push((
                    dataset.path().to_owned(),
                    NodeKind::OtherScientificDependency,
                    Relation::OtherScientificDependency,
                ));
            }
            InventoryKind::OtherMetadata
                if metadata_path_is_selected_or_shared(
                    dataset.path(),
                    selection.selected_frequency(),
                ) =>
            {
                other_scientific_count += 1;
                direct_dependencies.push((
                    dataset.path().to_owned(),
                    NodeKind::OtherScientificDependency,
                    Relation::OtherScientificDependency,
                ));
            }
            InventoryKind::SelectedScience
            | InventoryKind::ValidityMask
            | InventoryKind::Coordinate
            | InventoryKind::DimensionScale
            | InventoryKind::ProcessingMetadata
            | InventoryKind::ProvenanceMetadata
            | InventoryKind::ValidSamples
            | InventoryKind::OrbitMetadata
            | InventoryKind::AttitudeMetadata
            | InventoryKind::CalibrationMetadata
            | InventoryKind::GeolocationMetadata
            | InventoryKind::OtherMetadata => {}
        }
    }
    for (label, count) in [
        ("identification metadata", identification_count),
        ("provenance metadata", provenance_count),
        ("other scientific dependency", other_scientific_count),
    ] {
        if count == 0 {
            return Err(
                NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(label.to_owned()),
            );
        }
    }

    direct_dependencies.sort();
    direct_dependencies.dedup();
    for (path, kind, _) in &direct_dependencies {
        match nodes.insert(path.clone(), *kind) {
            Some(existing) if existing != *kind => {
                return Err(
                    NisarSubsetSourceDependencyGraphBuildErrorV2::ConflictingNode(path.clone()),
                );
            }
            _ => {}
        }
    }

    let mut edges = BTreeSet::new();
    for science_path in &expected_science_paths {
        for (dependency_path, _, relation) in &direct_dependencies {
            edges.insert((science_path.clone(), dependency_path.clone(), *relation));
        }
    }

    let included_dataset_paths = nodes
        .keys()
        .filter(|path| path.as_str() != "/")
        .cloned()
        .collect::<Vec<_>>();
    let mut dimension_scale_count = 0_usize;
    for dependent_path in included_dataset_paths {
        let dependent = datasets.get(dependent_path.as_str()).ok_or_else(|| {
            NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(dependent_path.clone())
        })?;
        for scale_path in dependent.dimension_scales().iter().flatten() {
            let scale = datasets.get(scale_path.as_str()).ok_or_else(|| {
                NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(scale_path.clone())
            })?;
            if scale.kind() != InventoryKind::DimensionScale {
                return Err(
                    NisarSubsetSourceDependencyGraphBuildErrorV2::ConflictingNode(
                        scale_path.clone(),
                    ),
                );
            }
            match nodes.insert(scale_path.clone(), NodeKind::DimensionScale) {
                Some(existing) if existing != NodeKind::DimensionScale => {
                    return Err(
                        NisarSubsetSourceDependencyGraphBuildErrorV2::ConflictingNode(
                            scale_path.clone(),
                        ),
                    );
                }
                _ => {}
            }
            dimension_scale_count += 1;
            edges.insert((
                dependent_path.clone(),
                scale_path.clone(),
                Relation::DimensionScale,
            ));
        }
    }
    if science_root == NisarScienceRoot::Lsar && dimension_scale_count == 0 {
        return Err(
            NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(
                "referenced dimension scale".to_owned(),
            ),
        );
    }

    let graph = NisarSubsetSourceDependencyGraphV2 {
        graph_contract: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT.to_owned(),
        schema_version: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
        evidence_authority: inventory.evidence().evidence_authority(),
        evidence_sha256: inventory.evidence().evidence_sha256().to_owned(),
        nodes: nodes
            .into_iter()
            .map(|(source_path, kind)| NisarSubsetSourceDependencyNodeV2 {
                source_path,
                kind,
                attribute_name: (kind == NisarSubsetSourceDependencyNodeKindV2::Attribute)
                    .then(|| "Conventions".to_owned()),
            })
            .collect(),
        edges: edges
            .into_iter()
            .map(
                |(dependent_path, dependency_path, relation)| NisarSubsetSourceDependencyEdgeV2 {
                    dependent_path,
                    dependency_path,
                    relation,
                },
            )
            .collect(),
    };
    validate_nisar_subset_source_dependency_graph_v2(inventory, &graph)?;
    Ok(graph)
}

fn validate_nisar_subset_source_dependency_graph_v2(
    inventory: &NisarSubsetVerifiedMetadataInventoryV2,
    graph: &NisarSubsetSourceDependencyGraphV2,
) -> Result<(), NisarSubsetSourceDependencyGraphBuildErrorV2> {
    use NisarSubsetDependencyRelationV2 as Relation;
    use NisarSubsetSourceDependencyNodeKindV2 as NodeKind;
    use NisarSubsetVerifiedMetadataDatasetKindV2 as InventoryKind;

    if graph.evidence_authority != inventory.evidence().evidence_authority()
        || graph.evidence_sha256 != inventory.evidence().evidence_sha256()
    {
        return Err(NisarSubsetSourceDependencyGraphBuildErrorV2::EvidenceInvalid);
    }
    let datasets = inventory
        .datasets()
        .iter()
        .map(|dataset| (dataset.path(), dataset))
        .collect::<BTreeMap<_, _>>();
    let groups = inventory
        .groups()
        .iter()
        .map(|group| (group.path(), group))
        .collect::<BTreeMap<_, _>>();
    for node in &graph.nodes {
        if node.kind == NodeKind::Attribute {
            let proven = node.source_path == "/"
                && node.attribute_name.as_deref() == Some("Conventions")
                && groups.get("/").is_some_and(|group| {
                    group
                        .attributes()
                        .iter()
                        .any(|attribute| attribute.name() == "Conventions")
                });
            if !proven {
                return Err(NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid);
            }
            continue;
        }
        let dataset = datasets.get(node.source_path.as_str()).ok_or_else(|| {
            NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(
                node.source_path.clone(),
            )
        })?;
        let proven = match node.kind {
            NodeKind::ScienceDataset => dataset.kind() == InventoryKind::SelectedScience,
            NodeKind::CoordinateDataset => dataset.kind() == InventoryKind::Coordinate,
            NodeKind::DimensionScale => dataset.kind() == InventoryKind::DimensionScale,
            NodeKind::ValidityMask => dataset.kind() == InventoryKind::ValidityMask,
            NodeKind::IdentificationMetadata => {
                dataset.kind() == InventoryKind::IdentificationMetadata
            }
            NodeKind::ProvenanceMetadata => {
                matches!(
                    dataset.kind(),
                    InventoryKind::ProcessingMetadata | InventoryKind::ProvenanceMetadata
                ) && metadata_path_is_selected_or_shared(
                    dataset.path(),
                    inventory.selection().selected_frequency(),
                )
            }
            NodeKind::OtherScientificDependency => match dataset.kind() {
                InventoryKind::OtherMetadata => metadata_path_is_selected_or_shared(
                    dataset.path(),
                    inventory.selection().selected_frequency(),
                ),
                InventoryKind::ValidSamples
                | InventoryKind::OrbitMetadata
                | InventoryKind::AttitudeMetadata
                | InventoryKind::CalibrationMetadata
                | InventoryKind::GeolocationMetadata => true,
                _ => false,
            },
            NodeKind::Attribute | NodeKind::Unknown => false,
        };
        if !proven {
            return Err(NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid);
        }
    }

    let graph_nodes = graph
        .nodes
        .iter()
        .map(|node| (node.source_path.as_str(), node.kind))
        .collect::<BTreeMap<_, _>>();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.relation == Relation::DimensionScale)
    {
        let dependent = datasets
            .get(edge.dependent_path.as_str())
            .ok_or(NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)?;
        let is_actual_attachment = dependent
            .dimension_scales()
            .iter()
            .flatten()
            .any(|scale_path| scale_path == &edge.dependency_path);
        if !is_actual_attachment {
            return Err(NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid);
        }
    }
    for node in graph
        .nodes
        .iter()
        .filter(|node| node.kind != NodeKind::Attribute)
    {
        let dataset = datasets
            .get(node.source_path.as_str())
            .ok_or(NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)?;
        for scale_path in dataset.dimension_scales().iter().flatten() {
            let scale = datasets.get(scale_path.as_str()).ok_or_else(|| {
                NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(scale_path.clone())
            })?;
            if scale.kind() != InventoryKind::DimensionScale
                || graph_nodes.get(scale_path.as_str()) != Some(&NodeKind::DimensionScale)
                || !graph.edges.iter().any(|edge| {
                    edge.dependent_path == node.source_path
                        && edge.dependency_path == *scale_path
                        && edge.relation == Relation::DimensionScale
                })
            {
                return Err(NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid);
            }
        }
    }

    derive_nisar_subset_dependency_closure_v2(
        graph,
        inventory.selection().selected_frequency(),
        inventory.selection().selected_polarizations(),
    )
    .map_err(|_| NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum NisarSubsetWriteError {
    #[error("NISAR subset Request is invalid")]
    RequestInvalid,
    #[error("NISAR subset transfer is not complete")]
    TransferIncomplete,
    #[error("NISAR subset verified block evidence is invalid")]
    BlockEvidenceInvalid,
    #[error("NISAR subset metadata evidence is invalid")]
    MetadataInvalid,
    #[error("NISAR subset output location is unsafe")]
    OutputUnsafe,
    #[error("NISAR subset output could not be persisted")]
    Persistence,
    #[error("NISAR subset scientific validation failed")]
    ValidationFailed,
    #[error("NISAR subset writing was cancelled")]
    Cancelled,
    #[error("NISAR subset recovery evidence is invalid")]
    RecoveryInvalid,
    #[error("NISAR V2 science authority is incomplete or invalid")]
    IncompleteScienceAuthority,
    #[error("NISAR V2 Writer binding is invalid")]
    WriterBinding,
    #[error("selected NISAR V2 science payload is missing")]
    SelectedScienceMissing,
    #[error("NISAR V2 multi-polarization Writer selection is invalid")]
    MultiPolarizationInvalid,
    #[error("NISAR V2 post-write science validation failed")]
    PostWriteScienceValidation,
    #[error("NISAR V2 manifest or sidecar is invalid")]
    ManifestInvalid,
    #[error("NISAR V2 derived output exceeds the bounded Writer output or tile limit")]
    LogicalCaptureLimitExceeded,
    #[error("NISAR V2 output volume lacks free space ({available} of {required} bytes)")]
    InsufficientDiskSpace { required: u64, available: u64 },
    /// Another writer holds the output directory's writer lock (transient).
    #[error("NISAR V2 output directory is being written by another writer")]
    OutputBusy,
    #[error("NISAR V2 atomic promotion failed")]
    AtomicPromotion,
    #[error("NISAR subset writer stopped at an injected test boundary")]
    InjectedFailure,
}

impl NisarSubsetWriteError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::TransferIncomplete | Self::Cancelled | Self::InjectedFailure => {
                "NISAR_SUBSET_INCOMPLETE"
            }
            Self::ValidationFailed => "NISAR_SUBSET_VALIDATION_FAILED",
            Self::IncompleteScienceAuthority => "NISAR_SUBSET_V2_SCIENCE_AUTHORITY_INVALID",
            Self::WriterBinding => "NISAR_SUBSET_V2_WRITER_BINDING",
            Self::SelectedScienceMissing => "NISAR_SUBSET_V2_SELECTED_SCIENCE_MISSING",
            Self::MultiPolarizationInvalid => "NISAR_SUBSET_V2_MULTI_POL_WRITER_INVALID",
            Self::PostWriteScienceValidation => "NISAR_SUBSET_V2_POST_WRITE_SCIENCE_VALIDATION",
            Self::ManifestInvalid => "NISAR_SUBSET_V2_MANIFEST_INVALID",
            Self::LogicalCaptureLimitExceeded => "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT",
            Self::InsufficientDiskSpace { .. } => "NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT",
            Self::OutputBusy => "NISAR_SUBSET_V2_OUTPUT_BUSY",
            Self::AtomicPromotion => "NISAR_SUBSET_V2_ATOMIC_PROMOTION_FAILED",
            Self::RequestInvalid
            | Self::BlockEvidenceInvalid
            | Self::MetadataInvalid
            | Self::OutputUnsafe
            | Self::Persistence
            | Self::RecoveryInvalid => "NISAR_SUBSET_ARTIFACT_INVALID",
        }
    }
}

#[derive(Clone, Debug)]
pub struct NisarSubsetWriteResultV1 {
    manifest: NisarSubsetArtifactManifestV1,
    artifact_path: PathBuf,
    manifest_path: PathBuf,
}

impl NisarSubsetWriteResultV1 {
    pub fn manifest(&self) -> &NisarSubsetArtifactManifestV1 {
        &self.manifest
    }

    pub fn artifact_path(&self) -> &Path {
        &self.artifact_path
    }

    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }
}

pub struct OfflineNisarSubsetWriter {
    output_root: Arc<PinnedDirectory>,
    destination_lease: Arc<DestinationLeaseController>,
    #[cfg(feature = "nisar-subset-production")]
    writer_recovery_anchor_v2: Option<Arc<dyn NisarSubsetWriterRecoveryAnchorV2>>,
    /// Process-wide output slots (downloader enhancement D1); a writer built
    /// without the worker's slots gets its own.
    #[cfg(feature = "nisar-subset-production")]
    output_slots_v2: Arc<NisarWriterOutputSlotsV2>,
    /// How long a writer waits for its output before it reports why
    /// (downloader enhancement D1b).
    #[cfg(feature = "nisar-subset-production")]
    long_wait_after_v2: std::time::Duration,
}

impl OfflineNisarSubsetWriter {
    pub const WRITER_ID: &'static str = "project_native_subset_writer";
    pub const OUTPUT_AUTHORITY: &'static str = NISAR_SUBSET_OUTPUT_AUTHORITY;
    pub const DERIVED_PRODUCT_TYPE: &'static str = "RSLC_SUBSET_DERIVED";

    pub fn new(
        output_root: PathBuf,
        destination_lease: Arc<DestinationLeaseController>,
    ) -> Result<Self, NisarSubsetWriteError> {
        let output_root = Arc::new(
            PinnedDirectory::create(&output_root)
                .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?,
        );
        Ok(Self {
            output_root,
            destination_lease,
            #[cfg(feature = "nisar-subset-production")]
            writer_recovery_anchor_v2: None,
            #[cfg(feature = "nisar-subset-production")]
            output_slots_v2: NisarWriterOutputSlotsV2::new(NISAR_WRITER_V2_DEFAULT_OUTPUT_SLOTS),
            #[cfg(feature = "nisar-subset-production")]
            long_wait_after_v2: NISAR_WRITER_V2_LONG_WAIT,
        })
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn with_writer_recovery_anchor_v2(
        mut self,
        anchor: Arc<dyn NisarSubsetWriterRecoveryAnchorV2>,
    ) -> Self {
        self.writer_recovery_anchor_v2 = Some(anchor);
        self
    }

    /// Shares the process-wide output slots of the worker (downloader
    /// enhancement D1).
    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn with_output_slots_v2(mut self, slots: Arc<NisarWriterOutputSlotsV2>) -> Self {
        self.output_slots_v2 = slots;
        self
    }

    /// Shortens the long-wait threshold (tests only).
    #[cfg(all(test, feature = "nisar-subset-production"))]
    pub(crate) fn with_long_wait_after_v2(mut self, threshold: std::time::Duration) -> Self {
        self.long_wait_after_v2 = threshold;
        self
    }

    pub fn write_and_validate(
        &self,
        request: &NisarSubsetRequestManifestV1,
        plan: &Hdf5RangePlanV1,
        transfer: &Hdf5RangeTransferResultV1,
        transfer_workspace: &Hdf5RangeWorkspaceV1,
        metadata: &NisarSubsetMetadataSnapshotV1,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<NisarSubsetWriteResultV1, NisarSubsetWriteError> {
        self.write_and_validate_inner(
            request,
            plan,
            transfer,
            transfer_workspace,
            metadata,
            cancelled,
            WriterFailpoint::None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write_and_validate_inner(
        &self,
        request: &NisarSubsetRequestManifestV1,
        plan: &Hdf5RangePlanV1,
        transfer: &Hdf5RangeTransferResultV1,
        transfer_workspace: &Hdf5RangeWorkspaceV1,
        metadata: &NisarSubsetMetadataSnapshotV1,
        cancelled: &(dyn Fn() -> bool + Sync),
        failpoint: WriterFailpoint,
    ) -> Result<NisarSubsetWriteResultV1, NisarSubsetWriteError> {
        request
            .validate()
            .map_err(|_| NisarSubsetWriteError::RequestInvalid)?;
        if transfer.manifest().state != NisarSubsetTransferState::RangesComplete
            || transfer.manifest().received_bytes != transfer.manifest().expected_remote_bytes
            || transfer.plan_sha256() != plan.plan_sha256()
        {
            return Err(NisarSubsetWriteError::TransferIncomplete);
        }
        let request_sha = request
            .sha256()
            .map_err(|_| NisarSubsetWriteError::RequestInvalid)?;
        if metadata.request_manifest_sha256 != request_sha
            || metadata.source_identity != request.source
            || metadata.source_validator != transfer.manifest().source_validator
            || metadata.dataset_bindings.is_empty()
            || metadata.evidence_sha256 != plan.metadata_evidence_sha256()
        {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let (_workspace_lock, blocks) = transfer_workspace
            .acquire_verified_chunk_blocks(request, plan, transfer, cancelled)
            .map_err(|error| match error {
                crate::Hdf5RangeTransferError::Cancelled => NisarSubsetWriteError::Cancelled,
                _ => NisarSubsetWriteError::BlockEvidenceInvalid,
            })?;
        self.write_verified(request, transfer, metadata, &blocks, cancelled, failpoint)
    }

    fn write_verified(
        &self,
        request: &NisarSubsetRequestManifestV1,
        transfer: &Hdf5RangeTransferResultV1,
        metadata: &NisarSubsetMetadataSnapshotV1,
        blocks: &[VerifiedHdf5ChunkBlockV1],
        cancelled: &(dyn Fn() -> bool + Sync),
        failpoint: WriterFailpoint,
    ) -> Result<NisarSubsetWriteResultV1, NisarSubsetWriteError> {
        if !safe_component(&request.expected_output.derived_artifact_id)
            || !safe_component(&request.expected_output.file_name)
        {
            return Err(NisarSubsetWriteError::OutputUnsafe);
        }
        self.output_root
            .verify()
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let materials = build_window_materials(request, blocks, metadata)?;
        let lock_path = self
            .output_root
            .join_component(".nisar-subset-writer-v1.lock")
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let _output_workspace_lock = acquire_hdf5_workspace_lock(&lock_path)
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let artifact_path = self
            .output_root
            .join_component(&request.expected_output.file_name)
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let artifact_result_path = self
            .output_root
            .canonical_component(&request.expected_output.file_name)
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let manifest_path = self
            .output_root
            .join_component(&format!(
                "{}.artifact-manifest-v1.json",
                request.expected_output.file_name
            ))
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let manifest_result_path = self
            .output_root
            .canonical_component(&format!(
                "{}.artifact-manifest-v1.json",
                request.expected_output.file_name
            ))
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let _destination_guard = self
            .destination_lease
            .acquire_for_runtime(&artifact_result_path, cancelled)
            .map_err(map_destination_lease_error)?;
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }

        let artifact_entry = output_entry_kind(&artifact_path)?;
        let manifest_entry = output_entry_kind(&manifest_path)?;
        if artifact_entry != OutputEntryKind::Missing || manifest_entry != OutputEntryKind::Missing
        {
            if artifact_entry != OutputEntryKind::Regular
                || manifest_entry == OutputEntryKind::Other
            {
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
            let candidate = CandidateFileGuard::open(&artifact_path)
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            let validation = validate_candidate(
                &candidate, request, transfer, metadata, &materials, blocks, cancelled,
            )
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            let manifest = build_artifact_manifest(request, transfer, &materials, validation)?;
            if manifest_entry == OutputEntryKind::Regular {
                let stored = read_guarded_text(&manifest_path)?;
                let stored = serde_json::from_str::<NisarSubsetArtifactManifestV1>(&stored)
                    .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
                if stored != manifest {
                    return Err(NisarSubsetWriteError::RecoveryInvalid);
                }
            } else {
                persist_artifact_manifest(&manifest_path, &manifest)?;
            }
            return Ok(NisarSubsetWriteResultV1 {
                manifest,
                artifact_path: artifact_result_path,
                manifest_path: manifest_result_path,
            });
        }

        let temp_path = self
            .output_root
            .join_component(&format!(".nisar-subset-writer-{}.writing.h5", temp_nonce()))
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        write_candidate(
            &temp_path, request, transfer, metadata, &materials, blocks, cancelled,
        )?;
        sync_candidate_path(&temp_path)?;
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        match failpoint {
            #[cfg(test)]
            WriterFailpoint::AfterWriteBeforeValidation => {
                return Err(NisarSubsetWriteError::InjectedFailure);
            }
            #[cfg(test)]
            WriterFailpoint::CorruptScienceBeforeValidation => {
                corrupt_science_for_test(&temp_path, &materials)?;
            }
            #[cfg(test)]
            WriterFailpoint::RemoveMaskBeforeValidation => {
                remove_mask_for_test(&temp_path, &materials)?;
            }
            #[cfg(test)]
            WriterFailpoint::CorruptMetadataBeforeValidation => {
                corrupt_metadata_for_test(&temp_path)?;
            }
            #[cfg(test)]
            WriterFailpoint::CorruptIdentificationBeforeValidation => {
                corrupt_identification_for_test(&temp_path)?;
            }
            #[cfg(test)]
            WriterFailpoint::AddUnexpectedAttributeBeforeValidation => {
                add_unexpected_attribute_for_test(&temp_path)?;
            }
            #[cfg(test)]
            WriterFailpoint::CorruptScienceShapeBeforeValidation => {
                corrupt_science_shape_for_test(&temp_path, &materials)?;
            }
            #[cfg(test)]
            WriterFailpoint::CorruptMaskChunkGeometryBeforeValidation => {
                corrupt_mask_chunk_geometry_for_test(&temp_path, &materials)?;
            }
            WriterFailpoint::None => {}
            #[cfg(test)]
            WriterFailpoint::AfterArtifactPromotion
            | WriterFailpoint::ReplaceCandidateAfterValidation => {}
        }
        let candidate = CandidateFileGuard::open(&temp_path)?;
        let validation = validate_candidate(
            &candidate, request, transfer, metadata, &materials, blocks, cancelled,
        )?;
        #[cfg(test)]
        if failpoint == WriterFailpoint::ReplaceCandidateAfterValidation {
            let displaced = temp_path.with_extension("validated-original");
            if fs::rename(&temp_path, &displaced).is_err() {
                return Err(NisarSubsetWriteError::InjectedFailure);
            }
            fs::write(&temp_path, b"attacker-replacement")
                .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
        }
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let manifest = build_artifact_manifest(request, transfer, &materials, validation)?;
        candidate.verify_path_identity()?;
        drop(candidate);
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        promote_validated_hdf5_core(
            temp_path,
            manifest.artifact_sha256.clone(),
            &artifact_path,
            cancelled,
        )
        .map_err(|error| match error {
            SharedWriterPromotionError::Cancelled => NisarSubsetWriteError::Cancelled,
            SharedWriterPromotionError::Failed => NisarSubsetWriteError::Persistence,
        })?;
        self.output_root
            .verify()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        sync_parent_directory(&artifact_path)?;
        #[cfg(test)]
        if failpoint == WriterFailpoint::AfterArtifactPromotion {
            return Err(NisarSubsetWriteError::InjectedFailure);
        }
        let installed = CandidateFileGuard::open(&artifact_path)
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        if installed.sha256(cancelled)? != manifest.artifact_sha256 {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        persist_artifact_manifest(&manifest_path, &manifest)?;
        self.output_root
            .verify()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        if installed.sha256(cancelled)? != manifest.artifact_sha256 {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        Ok(NisarSubsetWriteResultV1 {
            manifest,
            artifact_path: artifact_result_path,
            manifest_path: manifest_result_path,
        })
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn write_with_failpoint_for_test(
        &self,
        request: &NisarSubsetRequestManifestV1,
        plan: &Hdf5RangePlanV1,
        transfer: &Hdf5RangeTransferResultV1,
        transfer_workspace: &Hdf5RangeWorkspaceV1,
        metadata: &NisarSubsetMetadataSnapshotV1,
        cancelled: &(dyn Fn() -> bool + Sync),
        failpoint: WriterFailpoint,
    ) -> Result<NisarSubsetWriteResultV1, NisarSubsetWriteError> {
        self.write_and_validate_inner(
            request,
            plan,
            transfer,
            transfer_workspace,
            metadata,
            cancelled,
            failpoint,
        )
    }
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct VerifiedScienceAccountingBindingV2 {
    planning_request_count: u64,
    planning_response_body_bytes: u64,
    reserved_science_request_count: u64,
    reserved_science_response_body_bytes: u64,
    issued_science_request_count: u64,
    observed_science_response_body_bytes: u64,
    committed_science_request_count: u64,
    committed_science_response_body_bytes: u64,
    failed_science_request_count: u64,
    indeterminate_science_request_count: u64,
    indeterminate_science_response_body_bytes: u64,
    resumed_bytes: u64,
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug, Serialize, PartialEq)]
pub(crate) struct VerifiedSubsetWriteSelection {
    task_id: String,
    item_id: String,
    prepared_intent: NisarSubsetPreparedIntentV2,
    frozen_plan: NisarSubsetPlanV2,
    physical_plan: NisarSubsetPhysicalExecutionPlanV2,
    completed_range_request_counts: Vec<u64>,
    science_checkpoint_sha256: String,
    completed_range_evidence_sha256: String,
    science_accounting: VerifiedScienceAccountingBindingV2,
    derived_artifact_id: String,
    artifact_file_name: String,
    selection_sha256: String,
}

#[cfg(feature = "nisar-subset-production")]
impl VerifiedSubsetWriteSelection {
    fn from_revalidated(
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        science: &crate::nisar_science_transfer_v2::RevalidatedCompleteScienceForWriterV2,
    ) -> Result<Self, NisarSubsetWriteError> {
        let prepared_intent = parse_nisar_subset_prepared_intent_v2_envelope(&work.item.metadata)
            .map_err(|_| NisarSubsetWriteError::WriterBinding)?
            .ok_or(NisarSubsetWriteError::WriterBinding)?
            .intent()
            .map_err(|_| NisarSubsetWriteError::WriterBinding)?;
        let frozen_plan = completed.frozen_plan().clone();
        let physical_plan = science.plan().clone();
        let accounting = science.result().accounting();
        let derived_artifact_id = frozen_plan.provenance.derived_artifact_id.clone();
        let artifact_file_name = format!("{derived_artifact_id}.h5");
        let mut selection = Self {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            prepared_intent,
            frozen_plan,
            physical_plan,
            completed_range_request_counts: science
                .result()
                .completed_range_request_counts()
                .to_vec(),
            science_checkpoint_sha256: science.checkpoint_sha256().to_owned(),
            completed_range_evidence_sha256: science.completed_range_evidence_sha256().to_owned(),
            science_accounting: VerifiedScienceAccountingBindingV2 {
                planning_request_count: accounting.planning_request_count(),
                planning_response_body_bytes: accounting.planning_response_body_bytes(),
                reserved_science_request_count: accounting.reserved_science_request_count(),
                reserved_science_response_body_bytes: accounting
                    .reserved_science_response_body_bytes(),
                issued_science_request_count: accounting.science_request_count(),
                observed_science_response_body_bytes: accounting
                    .observed_science_response_body_bytes(),
                committed_science_request_count: accounting.committed_science_request_count(),
                committed_science_response_body_bytes: accounting
                    .committed_science_response_body_bytes(),
                failed_science_request_count: accounting.failed_science_request_count(),
                indeterminate_science_request_count: accounting
                    .indeterminate_science_request_count(),
                indeterminate_science_response_body_bytes: accounting
                    .indeterminate_science_response_body_bytes(),
                resumed_bytes: accounting.resumed_bytes(),
            },
            derived_artifact_id,
            artifact_file_name,
            selection_sha256: String::new(),
        };
        selection.selection_sha256 = selection.recompute_sha256()?;
        selection.validate()?;
        Ok(selection)
    }

    fn science_root(&self) -> Result<NisarScienceRoot, NisarSubsetWriteError> {
        NisarScienceRoot::from_source_identity(&self.frozen_plan.source_product_identity)
            .ok_or(NisarSubsetWriteError::WriterBinding)
    }

    fn recompute_sha256(&self) -> Result<String, NisarSubsetWriteError> {
        serde_json::to_vec(&(
            &self.task_id,
            &self.item_id,
            &self.prepared_intent,
            &self.frozen_plan,
            &self.physical_plan,
            &self.completed_range_request_counts,
            &self.science_checkpoint_sha256,
            &self.completed_range_evidence_sha256,
            &self.science_accounting,
            &self.derived_artifact_id,
            &self.artifact_file_name,
        ))
        .map(|canonical| sha256_bytes(&canonical))
        .map_err(|_| NisarSubsetWriteError::WriterBinding)
    }

    fn validate(&self) -> Result<(), NisarSubsetWriteError> {
        self.prepared_intent
            .validate()
            .map_err(|_| NisarSubsetWriteError::WriterBinding)?;
        self.frozen_plan
            .validate()
            .map_err(|_| NisarSubsetWriteError::WriterBinding)?;
        self.physical_plan
            .validate()
            .map_err(|_| NisarSubsetWriteError::WriterBinding)?;
        self.science_root()?;
        let execution_transfer_ranges = self
            .physical_plan
            .execution_transfer_ranges()
            .map_err(|_| NisarSubsetWriteError::WriterBinding)?;
        let estimated_execution_transfer_response_bytes = self
            .physical_plan
            .estimated_execution_transfer_response_bytes()
            .map_err(|_| NisarSubsetWriteError::WriterBinding)?;
        let completed_science_request_count = self
            .completed_range_request_counts
            .iter()
            .try_fold(0_u64, |total, count| total.checked_add(*count));
        if self.task_id.is_empty()
            || self.item_id.is_empty()
            || self.prepared_intent.canonical_intent_sha256
                != self.frozen_plan.prepared_intent_sha256
            || self.physical_plan.prepared_intent_sha256()
                != self.frozen_plan.prepared_intent_sha256
            || self.physical_plan.final_plan_sha256()
                != self.frozen_plan.canonical_semantic_plan_digest
            || self.physical_plan.selected_frequency() != &self.frozen_plan.selected_frequency
            || self.physical_plan.selected_polarizations()
                != self.frozen_plan.selected_polarizations
            || self.derived_artifact_id != self.frozen_plan.provenance.derived_artifact_id
            || self.artifact_file_name != format!("{}.h5", self.derived_artifact_id)
            || !safe_component(&self.derived_artifact_id)
            || !safe_component(&self.artifact_file_name)
            || !valid_v2_sha256(&self.science_checkpoint_sha256)
            || !valid_v2_sha256(&self.completed_range_evidence_sha256)
            || !valid_v2_sha256(&self.selection_sha256)
            || self.recompute_sha256()? != self.selection_sha256
            || self.completed_range_request_counts.len() != execution_transfer_ranges.len()
            || self.completed_range_request_counts.contains(&0)
            || completed_science_request_count
                != Some(self.science_accounting.committed_science_request_count)
            || self
                .science_accounting
                .committed_science_response_body_bytes
                != estimated_execution_transfer_response_bytes
            || self.science_accounting.observed_science_response_body_bytes
                < self
                    .science_accounting
                    .committed_science_response_body_bytes
            || self.science_accounting.reserved_science_response_body_bytes
                < self.science_accounting.observed_science_response_body_bytes
            // Owner decision 2026-10-17 (option B): an indeterminate charge is
            // accepted with the revalidated science authority; it must be a
            // coherent ledger fact (both counters or neither) with bounds that
            // are representable (indeterminate_science_charge).
            || (self.science_accounting.indeterminate_science_request_count == 0)
                != (self
                    .science_accounting
                    .indeterminate_science_response_body_bytes
                    == 0)
            || self.science_accounting.issued_science_request_count
                != self
                    .science_accounting
                    .committed_science_request_count
                    .checked_add(self.science_accounting.failed_science_request_count)
                    .ok_or(NisarSubsetWriteError::WriterBinding)?
        {
            return Err(NisarSubsetWriteError::WriterBinding);
        }
        self.indeterminate_science_charge()?;
        Ok(())
    }

    /// Owner decision 2026-10-17 (option B): the bounds of an indeterminate
    /// science charge that the artifact manifest records; `None` when the
    /// charge is determinate, so a determinate manifest is unchanged. The lower
    /// bounds are what is known to have happened (committed and failed
    /// requests, observed bytes); the upper bounds add the indeterminate
    /// reservation, which a killed process may or may not have used.
    fn indeterminate_science_charge(
        &self,
    ) -> Result<Option<NisarSubsetIndeterminateScienceChargeV2>, NisarSubsetWriteError> {
        let accounting = &self.science_accounting;
        if accounting.indeterminate_science_request_count == 0
            && accounting.indeterminate_science_response_body_bytes == 0
        {
            return Ok(None);
        }
        let science_request_count_lower_bound = accounting
            .committed_science_request_count
            .checked_add(accounting.failed_science_request_count)
            .ok_or(NisarSubsetWriteError::WriterBinding)?;
        let science_response_body_bytes_lower_bound =
            accounting.observed_science_response_body_bytes;
        Ok(Some(NisarSubsetIndeterminateScienceChargeV2 {
            indeterminate_science_request_count: accounting.indeterminate_science_request_count,
            indeterminate_science_response_body_bytes: accounting
                .indeterminate_science_response_body_bytes,
            science_request_count_lower_bound,
            science_request_count_upper_bound: science_request_count_lower_bound
                .checked_add(accounting.indeterminate_science_request_count)
                .ok_or(NisarSubsetWriteError::WriterBinding)?,
            science_response_body_bytes_lower_bound,
            science_response_body_bytes_upper_bound: science_response_body_bytes_lower_bound
                .checked_add(accounting.indeterminate_science_response_body_bytes)
                .ok_or(NisarSubsetWriteError::WriterBinding)?,
        }))
    }
}

/// Fourth review (2026-10-17): one incoherent accounting fact per variant,
/// applied to a real revalidated selection and resealed, so the binding
/// check itself (not the selection digest) must refuse it.
#[cfg(all(test, feature = "nisar-subset-production"))]
#[derive(Clone, Copy, Debug)]
pub(crate) enum WriterBindingTamperForTest {
    IndeterminateRequestsWithoutBytes,
    IndeterminateBytesWithoutRequests,
    IssuedIsNotCommittedPlusFailed,
    ObservedBelowCommittedBytes,
    ReservedBelowObservedBytes,
    ZeroCompletedRangeRequestCount,
}

#[cfg(all(test, feature = "nisar-subset-production"))]
pub(crate) fn validate_writer_binding_for_test(
    work: &DownloadWork,
    completed: &NisarSubsetPlanningCompleteV2,
    reported_science: &NisarSubsetScienceTransferResultV2,
    authority: &dyn NisarSubsetScienceAuthorityV2,
    tamper: Option<WriterBindingTamperForTest>,
) -> Result<(), NisarSubsetWriteError> {
    let first = revalidate_complete_science_for_writer_v2(
        work,
        completed,
        reported_science,
        authority,
        &WorkerControl::default(),
    )
    .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    let mut selection = VerifiedSubsetWriteSelection::from_revalidated(work, completed, &first)?;
    let Some(tamper) = tamper else {
        return selection.validate();
    };
    let accounting = &mut selection.science_accounting;
    match tamper {
        WriterBindingTamperForTest::IndeterminateRequestsWithoutBytes => {
            accounting.indeterminate_science_request_count = 1;
            accounting.indeterminate_science_response_body_bytes = 0;
        }
        WriterBindingTamperForTest::IndeterminateBytesWithoutRequests => {
            accounting.indeterminate_science_request_count = 0;
            accounting.indeterminate_science_response_body_bytes = 1;
        }
        WriterBindingTamperForTest::IssuedIsNotCommittedPlusFailed => {
            accounting.issued_science_request_count += 1;
        }
        WriterBindingTamperForTest::ObservedBelowCommittedBytes => {
            accounting.observed_science_response_body_bytes =
                accounting.committed_science_response_body_bytes - 1;
        }
        WriterBindingTamperForTest::ReservedBelowObservedBytes => {
            accounting.reserved_science_response_body_bytes =
                accounting.observed_science_response_body_bytes - 1;
        }
        WriterBindingTamperForTest::ZeroCompletedRangeRequestCount => {
            selection.completed_range_request_counts[0] = 0;
        }
    }
    selection.selection_sha256 = selection.recompute_sha256()?;
    selection.validate()
}

#[cfg(feature = "nisar-subset-production")]
fn writer_recovery_lineage_key_v2(
    selection: &VerifiedSubsetWriteSelection,
) -> Result<String, NisarSubsetWriteError> {
    serde_json::to_vec(&(
        "NisarSubsetWriterRecoveryLineageV2",
        &selection.task_id,
        &selection.item_id,
        &selection.prepared_intent.canonical_intent_sha256,
        &selection.frozen_plan.canonical_semantic_plan_digest,
        selection.physical_plan.physical_plan_sha256(),
        &selection.selection_sha256,
        &selection.derived_artifact_id,
        &selection.artifact_file_name,
    ))
    .map(|canonical| sha256_bytes(&canonical))
    .map_err(|_| NisarSubsetWriteError::WriterBinding)
}

#[cfg(feature = "nisar-subset-production")]
impl NisarSubsetWriterRecoveryAnchorRecordV2 {
    fn candidate_building(
        selection: &VerifiedSubsetWriteSelection,
        candidate_identity: CandidateFileIdentity,
    ) -> Result<Self, NisarSubsetWriteError> {
        let mut record = Self {
            contract: Self::CONTRACT.to_owned(),
            schema_version: Self::SCHEMA_VERSION,
            lineage_key_sha256: writer_recovery_lineage_key_v2(selection)?,
            task_id: selection.task_id.clone(),
            item_id: selection.item_id.clone(),
            prepared_intent_sha256: selection.prepared_intent.canonical_intent_sha256.clone(),
            final_semantic_plan_sha256: selection
                .frozen_plan
                .canonical_semantic_plan_digest
                .clone(),
            physical_plan_sha256: selection.physical_plan.physical_plan_sha256().to_owned(),
            writer_selection_sha256: selection.selection_sha256.clone(),
            derived_artifact_id: selection.derived_artifact_id.clone(),
            artifact_file_name: selection.artifact_file_name.clone(),
            candidate_identity,
            candidate_size_bytes: None,
            candidate_sha256: None,
            artifact_identity: None,
            artifact_size_bytes: None,
            artifact_sha256: None,
            state: NisarSubsetWriterRecoveryStateV2::CandidateBuilding,
            record_sha256: String::new(),
        };
        record.record_sha256 = record.recompute_sha256()?;
        record.validate_for_selection(selection)?;
        Ok(record)
    }

    fn rebind_building(
        &self,
        candidate_identity: CandidateFileIdentity,
    ) -> Result<Self, NisarSubsetWriteError> {
        self.validate_integrity()?;
        if self.state != NisarSubsetWriterRecoveryStateV2::CandidateBuilding {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        let mut replacement = self.clone();
        replacement.candidate_identity = candidate_identity;
        replacement.record_sha256 = replacement.recompute_sha256()?;
        replacement.validate_integrity()?;
        Ok(replacement)
    }

    /// A validated candidate that no longer exists (an unnamed Linux
    /// candidate ends with its process) is rebuilt as a new candidate. Its
    /// size and SHA-256 belonged to the lost file: the writer stores
    /// wall-clock times in group object headers, so a rebuild is not
    /// byte-identical once a second has passed. The rebuild is validated
    /// from the same revalidated selection and recorded afresh.
    fn rebuild_lost_validated(
        &self,
        candidate_identity: CandidateFileIdentity,
    ) -> Result<Self, NisarSubsetWriteError> {
        self.validate_integrity()?;
        if self.state != NisarSubsetWriterRecoveryStateV2::CandidateValidated {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        let mut replacement = self.clone();
        replacement.candidate_identity = candidate_identity;
        replacement.candidate_size_bytes = None;
        replacement.candidate_sha256 = None;
        replacement.state = NisarSubsetWriterRecoveryStateV2::CandidateBuilding;
        replacement.record_sha256 = replacement.recompute_sha256()?;
        replacement.validate_integrity()?;
        Ok(replacement)
    }

    fn mark_candidate_validated(
        &self,
        candidate_identity: CandidateFileIdentity,
        candidate_size_bytes: u64,
        candidate_sha256: &str,
    ) -> Result<Self, NisarSubsetWriteError> {
        self.validate_integrity()?;
        if self.state == NisarSubsetWriterRecoveryStateV2::CandidateValidated
            && (self.candidate_size_bytes != Some(candidate_size_bytes)
                || self.candidate_sha256.as_deref() != Some(candidate_sha256))
        {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        if !matches!(
            self.state,
            NisarSubsetWriterRecoveryStateV2::CandidateBuilding
                | NisarSubsetWriterRecoveryStateV2::CandidateValidated
        ) {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        let mut replacement = self.clone();
        replacement.candidate_identity = candidate_identity;
        replacement.candidate_size_bytes = Some(candidate_size_bytes);
        replacement.candidate_sha256 = Some(candidate_sha256.to_owned());
        replacement.state = NisarSubsetWriterRecoveryStateV2::CandidateValidated;
        replacement.record_sha256 = replacement.recompute_sha256()?;
        replacement.validate_integrity()?;
        Ok(replacement)
    }

    /// Review of slice 7 (P2-1): a validated candidate that stayed unreadable
    /// is removed after this transition; its lineage returns to
    /// CandidateBuilding and the next run writes a new candidate.
    fn reset_unreadable_validated(&self) -> Result<Self, NisarSubsetWriteError> {
        self.validate_integrity()?;
        if self.state != NisarSubsetWriterRecoveryStateV2::CandidateValidated {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        let mut replacement = self.clone();
        replacement.candidate_size_bytes = None;
        replacement.candidate_sha256 = None;
        replacement.state = NisarSubsetWriterRecoveryStateV2::CandidateBuilding;
        replacement.record_sha256 = replacement.recompute_sha256()?;
        replacement.validate_integrity()?;
        Ok(replacement)
    }

    fn mark_artifact_promoted(
        &self,
        artifact_identity: CandidateFileIdentity,
        artifact_size_bytes: u64,
        artifact_sha256: &str,
    ) -> Result<Self, NisarSubsetWriteError> {
        self.validate_integrity()?;
        if self.state != NisarSubsetWriterRecoveryStateV2::CandidateValidated
            || self.candidate_size_bytes != Some(artifact_size_bytes)
            || self.candidate_sha256.as_deref() != Some(artifact_sha256)
        {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        let mut replacement = self.clone();
        replacement.artifact_identity = Some(artifact_identity);
        replacement.artifact_size_bytes = Some(artifact_size_bytes);
        replacement.artifact_sha256 = Some(artifact_sha256.to_owned());
        replacement.state = NisarSubsetWriterRecoveryStateV2::ArtifactPromoted;
        replacement.record_sha256 = replacement.recompute_sha256()?;
        replacement.validate_integrity()?;
        Ok(replacement)
    }

    fn validate_for_selection(
        &self,
        selection: &VerifiedSubsetWriteSelection,
    ) -> Result<(), NisarSubsetWriteError> {
        self.validate_integrity()?;
        if self.lineage_key_sha256 != writer_recovery_lineage_key_v2(selection)?
            || self.task_id != selection.task_id
            || self.item_id != selection.item_id
            || self.prepared_intent_sha256 != selection.prepared_intent.canonical_intent_sha256
            || self.final_semantic_plan_sha256
                != selection.frozen_plan.canonical_semantic_plan_digest
            || self.physical_plan_sha256 != selection.physical_plan.physical_plan_sha256()
            || self.writer_selection_sha256 != selection.selection_sha256
            || self.derived_artifact_id != selection.derived_artifact_id
            || self.artifact_file_name != selection.artifact_file_name
        {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        Ok(())
    }

    fn validate_candidate_material(
        &self,
        identity: &CandidateFileIdentity,
        size_bytes: u64,
        sha256: &str,
    ) -> Result<(), NisarSubsetWriteError> {
        self.validate_integrity()?;
        if self.state != NisarSubsetWriterRecoveryStateV2::CandidateValidated
            || self.candidate_identity != *identity
            || self.candidate_size_bytes != Some(size_bytes)
            || self.candidate_sha256.as_deref() != Some(sha256)
        {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        Ok(())
    }

    fn validate_artifact_recovery(
        &self,
        identity: &CandidateFileIdentity,
        size_bytes: u64,
        sha256: &str,
    ) -> Result<(), NisarSubsetWriteError> {
        self.validate_integrity()?;
        let valid = match self.state {
            NisarSubsetWriterRecoveryStateV2::CandidateValidated => {
                self.candidate_size_bytes == Some(size_bytes)
                    && self.candidate_sha256.as_deref() == Some(sha256)
            }
            NisarSubsetWriterRecoveryStateV2::ArtifactPromoted
            | NisarSubsetWriterRecoveryStateV2::Complete => {
                self.artifact_identity.as_ref() == Some(identity)
                    && self.artifact_size_bytes == Some(size_bytes)
                    && self.artifact_sha256.as_deref() == Some(sha256)
            }
            NisarSubsetWriterRecoveryStateV2::CandidateBuilding => false,
        };
        valid
            .then_some(())
            .ok_or(NisarSubsetWriteError::RecoveryInvalid)
    }
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetArtifactWindowV2 {
    component_id: u16,
    component_sha256: String,
    window_sha256: String,
    polarization: String,
    source_dataset_path: String,
    line_start: u64,
    line_end_exclusive: u64,
    sample_start: u64,
    sample_end_exclusive: u64,
    shape: [u64; 2],
    chunk_shape: [u64; 2],
}

/// Owner decision 2026-10-17 (option B): the science charge of an artifact
/// whose ledger carries an indeterminate charge (a Range attempt a killed
/// process left Reserved). Every completed Range was verified against its
/// response digest and material before the artifact was written; the request
/// and response-byte charge is only known within these bounds.
#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetIndeterminateScienceChargeV2 {
    indeterminate_science_request_count: u64,
    indeterminate_science_response_body_bytes: u64,
    science_request_count_lower_bound: u64,
    science_request_count_upper_bound: u64,
    science_response_body_bytes_lower_bound: u64,
    science_response_body_bytes_upper_bound: u64,
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetArtifactManifestV2 {
    manifest_contract: String,
    schema_version: u32,
    task_id: String,
    item_id: String,
    output_authority: String,
    quality_status: String,
    derived_artifact_id: String,
    artifact_file_name: String,
    artifact_size_bytes: u64,
    artifact_sha256: String,
    prepared_intent_sha256: String,
    final_semantic_plan_sha256: String,
    physical_plan_sha256: String,
    writer_selection_sha256: String,
    science_checkpoint_sha256: String,
    completed_range_evidence_sha256: String,
    source_product_id: String,
    source_asset_key: String,
    source_validator_sha256: String,
    source_size: u64,
    requested_geometry_sha256: String,
    effective_geometry_sha256: String,
    selected_frequency: String,
    selected_polarizations: Vec<String>,
    inventory_sha256: String,
    inventory_evidence_sha256: String,
    dependency_graph_sha256: String,
    dataset_windows: Vec<NisarSubsetArtifactWindowV2>,
    complete: bool,
    /// Present only when the science charge is indeterminate (option B of
    /// 2026-10-17); a determinate manifest keeps its previous canonical form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    indeterminate_science_charge: Option<NisarSubsetIndeterminateScienceChargeV2>,
}

#[cfg(feature = "nisar-subset-production")]
impl NisarSubsetArtifactManifestV2 {
    pub(crate) fn selected_polarizations(&self) -> &[String] {
        &self.selected_polarizations
    }

    pub(crate) fn output_authority(&self) -> &str {
        &self.output_authority
    }

    pub(crate) fn quality_status(&self) -> &str {
        &self.quality_status
    }

    pub(crate) fn dataset_window_count(&self) -> usize {
        self.dataset_windows.len()
    }

    pub(crate) const fn complete(&self) -> bool {
        self.complete
    }

    pub(crate) const fn artifact_size_bytes(&self) -> u64 {
        self.artifact_size_bytes
    }

    pub(crate) fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }

    /// Returns the aggregate logical extent represented by the derived
    /// dataset windows. Every selected polarization is expected to carry the
    /// same geometry, but computing the envelope here keeps evidence
    /// projection independent of the order in which the writer serializes
    /// those windows.
    pub(crate) fn actual_extent(&self) -> Option<(u64, u64, u64, u64)> {
        self.dataset_windows.iter().fold(None, |extent, window| {
            Some(match extent {
                None => (
                    window.line_start,
                    window.line_end_exclusive,
                    window.sample_start,
                    window.sample_end_exclusive,
                ),
                Some((line_start, line_end, sample_start, sample_end)) => (
                    line_start.min(window.line_start),
                    line_end.max(window.line_end_exclusive),
                    sample_start.min(window.sample_start),
                    sample_end.max(window.sample_end_exclusive),
                ),
            })
        })
    }
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug)]
pub(crate) struct NisarSubsetWriteResultV2 {
    manifest: NisarSubsetArtifactManifestV2,
    artifact_path: PathBuf,
    manifest_path: PathBuf,
}

#[cfg(feature = "nisar-subset-production")]
impl NisarSubsetWriteResultV2 {
    pub(crate) fn manifest(&self) -> &NisarSubsetArtifactManifestV2 {
        &self.manifest
    }

    pub(crate) fn artifact_path(&self) -> &Path {
        &self.artifact_path
    }

    pub(crate) fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug)]
struct VerifiedWriterMetadataClosureV2 {
    group_bindings: Vec<SourceGroupBindingV1>,
    dataset_bindings: Vec<SourceMetadataDatasetBindingV1>,
}

#[cfg(feature = "nisar-subset-production")]
/// A metadata read the worker's pause or cancellation stopped stays a stop;
/// any other failure is a binding failure.
#[cfg(feature = "nisar-subset-production")]
fn stopped_or_writer_binding(error: NisarSubsetWriteError) -> NisarSubsetWriteError {
    if error == NisarSubsetWriteError::Cancelled {
        error
    } else {
        NisarSubsetWriteError::WriterBinding
    }
}

#[cfg(feature = "nisar-subset-production")]
fn capture_v2_writer_metadata_closure(
    selection: &VerifiedSubsetWriteSelection,
    part_path: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<VerifiedWriterMetadataClosureV2, NisarSubsetWriteError> {
    let inventory = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        &selection.prepared_intent,
        selection.physical_plan.source_validator().clone(),
        part_path,
        cancelled,
    )
    .map_err(stopped_or_writer_binding)?;
    if inventory.evidence().inventory_sha256() != selection.physical_plan.inventory_sha256()
        || inventory.evidence().evidence_sha256()
            != selection.physical_plan.inventory_evidence_sha256()
        || inventory.evidence().source_validator_sha256()
            != selection.physical_plan.source_validator_sha256()
        || inventory.evidence().source_size() != selection.physical_plan.source_size()
    {
        return Err(NisarSubsetWriteError::WriterBinding);
    }
    let scanner_selection = SourceMetadataSelection::from_v2(&selection.prepared_intent)
        .map_err(|_| NisarSubsetWriteError::WriterBinding)?;
    let (_, group_bindings, dataset_bindings, source_dataset_count) =
        read_source_metadata(&scanner_selection, part_path, cancelled)
            .map_err(stopped_or_writer_binding)?;
    if source_dataset_count as u64 != inventory.evidence().source_dataset_count() {
        return Err(NisarSubsetWriteError::WriterBinding);
    }
    let dataset_bindings = dataset_bindings
        .into_iter()
        .filter(|binding| binding.action == SourceDatasetActionV1::PreserveExact)
        .collect::<Vec<_>>();
    if group_bindings.is_empty() || dataset_bindings.is_empty() {
        return Err(NisarSubsetWriteError::WriterBinding);
    }
    let copied_paths = dataset_bindings
        .iter()
        .map(|binding| binding.path.as_str())
        .collect::<BTreeSet<_>>();
    if dataset_bindings.iter().any(|binding| {
        binding
            .dimension_scales
            .iter()
            .flatten()
            .any(|scale| !copied_paths.contains(scale.as_str()))
    }) {
        return Err(NisarSubsetWriteError::WriterBinding);
    }
    Ok(VerifiedWriterMetadataClosureV2 {
        group_bindings,
        dataset_bindings,
    })
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug)]
struct DerivedWindowMaterialV2 {
    component_id: u16,
    component_sha256: String,
    window_sha256: String,
    polarization: String,
    source_dataset_path: String,
    root_path: String,
    science_path: String,
    line_start: u64,
    line_end_exclusive: u64,
    sample_start: u64,
    sample_end_exclusive: u64,
    source_shape: [u64; 2],
    shape: [u64; 2],
    chunk_shape: [u64; 2],
    line_scale_name: String,
    sample_scale_name: String,
    mask_name: String,
}

/// A derived window ready for streaming (Issue #8, slice 6): its descriptor,
/// the captured axis windows, the mask chunk layout and the tile plans of its
/// science and mask datasets. Science and mask samples are never retained;
/// they are streamed from the pinned source in both writer passes.
#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug)]
struct CapturedDerivedWindowMaterialV2 {
    descriptor: DerivedWindowMaterialV2,
    line_axis: Vec<f64>,
    sample_axis: Vec<f64>,
    mask_chunk_shape: [u64; 2],
    science_tiles: Vec<V2StreamTile>,
    mask_tiles: Vec<V2StreamTile>,
}

#[cfg(feature = "nisar-subset-production")]
impl std::ops::Deref for CapturedDerivedWindowMaterialV2 {
    type Target = DerivedWindowMaterialV2;

    fn deref(&self) -> &Self::Target {
        &self.descriptor
    }
}

/// Logical bytes of the copied metadata closure: its canonical binding record
/// plus the encoded datatype and value of every copied dataset and attribute.
/// The closure is the writer's only retained bulk input; its capture is
/// bounded by `MAX_METADATA_BYTES`.
#[cfg(feature = "nisar-subset-production")]
fn v2_metadata_closure_bytes(
    metadata: &VerifiedWriterMetadataClosureV2,
) -> Result<u64, NisarSubsetWriteError> {
    fn add(total: u64, value: usize) -> Result<u64, NisarSubsetWriteError> {
        u64::try_from(value)
            .ok()
            .and_then(|value| total.checked_add(value))
            .ok_or(NisarSubsetWriteError::LogicalCaptureLimitExceeded)
    }

    let canonical_metadata =
        serde_json::to_vec(&(&metadata.group_bindings, &metadata.dataset_bindings))
            .map_err(|_| NisarSubsetWriteError::LogicalCaptureLimitExceeded)?;
    let mut total = add(0, canonical_metadata.len())?;
    for group in &metadata.group_bindings {
        for attribute in &group.attributes {
            total = add(total, attribute.datatype_encoded.len())?;
            total = add(total, attribute.value.len())?;
        }
    }
    for dataset in &metadata.dataset_bindings {
        total = add(total, dataset.datatype_encoded.len())?;
        total = add(total, dataset.value.as_deref().map_or(0, <[u8]>::len))?;
        for attribute in &dataset.attributes {
            total = add(total, attribute.datatype_encoded.len())?;
            total = add(total, attribute.value.len())?;
        }
    }
    Ok(total)
}

/// A refused output keeps the stable writer codes: the output bound is
/// `NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT`, missing free space is its own
/// resource failure that freeing space resolves.
#[cfg(feature = "nisar-subset-production")]
fn map_v2_output_refusal(refusal: V2OutputAdmissionRefusal) -> NisarSubsetWriteError {
    match refusal {
        V2OutputAdmissionRefusal::OutputLimit { .. } => {
            NisarSubsetWriteError::LogicalCaptureLimitExceeded
        }
        V2OutputAdmissionRefusal::DiskSpace {
            required,
            available,
        } => NisarSubsetWriteError::InsufficientDiskSpace {
            required,
            available,
        },
    }
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriterFailpointV2 {
    None,
    #[cfg(test)]
    AfterCandidateBootstrapBeforeBuildingAnchor,
    #[cfg(test)]
    AfterBuildingAnchorBeforeWrite,
    #[cfg(test)]
    AfterCandidateWriteBeforeSync,
    #[cfg(test)]
    AfterCandidateSyncBeforeSeal,
    #[cfg(test)]
    AfterCandidateSealBeforeSemantic,
    #[cfg(test)]
    AfterCandidateSemanticBeforeHash,
    #[cfg(test)]
    AfterCandidateHashBeforeValidatedAnchor,
    #[cfg(test)]
    AfterArtifactPromotionBeforeAnchor,
    #[cfg(test)]
    ReplacePublishedArtifactBeforeArtifactAnchor,
    #[cfg(test)]
    ForceCopyPromotionToNewInode,
    #[cfg(test)]
    ReplacePartBeforeMaterialCapture,
    #[cfg(test)]
    SymlinkPartBeforeMaterialCapture,
    #[cfg(test)]
    HardlinkPartBeforeMaterialCapture,
    #[cfg(test)]
    WritePartBeforeMaterialCapture,
    /// Truncates the part by one byte where an in-place write would happen
    /// (downloader enhancement D2: truncation is refused like a write).
    #[cfg(test)]
    TruncatePartBeforeMaterialCapture,
    /// Truncates the part into its first completed Range, extends it back to
    /// its size and restores its timestamps (review of D1/D2, D1b P2-6).
    #[cfg(test)]
    TruncateRestorePartBeforeMaterialCapture,
    #[cfg(test)]
    WritePartImmediatelyBeforePromotion,
    #[cfg(test)]
    MutateSourceDuringHdf5ReadAndRestore,
    #[cfg(test)]
    ReplaceSnapshotPathDuringHdf5Read,
    #[cfg(test)]
    MutateCandidateAfterSemanticBeforeHash,
    #[cfg(test)]
    AddUnselectedSourcePayloadBeforeValidation,
    #[cfg(test)]
    AddDuplicateAncillaryBeforeValidation,
    #[cfg(test)]
    AddExtraGroupBeforeValidation,
    #[cfg(test)]
    AfterValidationBeforePromotion,
    /// Pauses the worker control right after the CandidateValidated anchor.
    #[cfg(test)]
    PauseAfterValidatedAnchor,
    /// A promotion that fails before the candidate is renamed.
    #[cfg(test)]
    AtomicPromotionFailureBeforeRename,
    /// The CandidateValidated anchor is committed but its reply reports a
    /// stop, as when the worker pauses while awaiting the coordinator.
    #[cfg(test)]
    ValidatedAnchorStoppedAfterCommit,
    /// The CandidateValidated anchor reports a stop before it is committed.
    #[cfg(test)]
    ValidatedAnchorStoppedBeforeCommit,
    /// Writes the pinned part right after a candidate passed its validation.
    #[cfg(test)]
    WritePartAfterCandidateValidation,
    /// Pauses the worker before artifact recovery revalidates authority again.
    #[cfg(test)]
    PauseBeforeArtifactRecoveryRevalidation,
    /// Pauses the worker right before the metadata closure is read.
    #[cfg(test)]
    PauseBeforeMetadataCapture,
    /// Pauses the worker before the metadata closure is read and resumes it
    /// right after the read stopped, as a scheduler resuming the item at once.
    #[cfg(test)]
    PauseThenResumeDuringMetadataCapture,
    #[cfg(test)]
    CorruptScienceBeforeValidation,
    #[cfg(test)]
    CorruptScienceShapeBeforeValidation,
    #[cfg(test)]
    CorruptLastScienceSampleBeforeValidation,
    #[cfg(test)]
    CorruptLastMaskSampleBeforeValidation,
    #[cfg(test)]
    CorruptCoordinateBeforeValidation,
    #[cfg(test)]
    CorruptScaleBeforeValidation,
    #[cfg(test)]
    CorruptMaskBeforeValidation,
    #[cfg(test)]
    CorruptProvenanceBeforeValidation,
    #[cfg(test)]
    CorruptQualityBeforeValidation,
    #[cfg(test)]
    CorruptAuthorityBeforeValidation,
    #[cfg(test)]
    CorruptMetadataBeforeValidation,
    #[cfg(test)]
    AfterArtifactPromotion,
}

#[cfg(feature = "nisar-subset-production")]
impl OfflineNisarSubsetWriter {
    pub(crate) fn write_and_validate_v2(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        reported_science: &NisarSubsetScienceTransferResultV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetWriteResultV2, NisarSubsetWriteError> {
        self.write_and_validate_v2_observed(
            work,
            completed,
            reported_science,
            authority,
            control,
            &mut |_| {},
        )
    }

    /// `admission` hears when the writer waits for its output and when it is
    /// admitted after waiting (downloader enhancement D1).
    pub(crate) fn write_and_validate_v2_observed(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        reported_science: &NisarSubsetScienceTransferResultV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
        admission: &mut dyn FnMut(NisarWriterAdmissionEventV2),
    ) -> Result<NisarSubsetWriteResultV2, NisarSubsetWriteError> {
        self.write_and_validate_v2_inner(
            work,
            completed,
            reported_science,
            authority,
            control,
            WriterFailpointV2::None,
            admission,
        )
    }

    // The failpoint is test-only; the admission observer is the production seam.
    #[allow(clippy::too_many_arguments)]
    fn write_and_validate_v2_inner(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        reported_science: &NisarSubsetScienceTransferResultV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
        failpoint: WriterFailpointV2,
        admission: &mut dyn FnMut(NisarWriterAdmissionEventV2),
    ) -> Result<NisarSubsetWriteResultV2, NisarSubsetWriteError> {
        // Downloader enhancement D2: every run reports its stage timings on
        // the local diagnostic stream, whatever its outcome.
        let mut timer = NisarWriterStageTimerV2::start();
        let result = self.write_and_validate_v2_stages(
            work,
            completed,
            reported_science,
            authority,
            control,
            failpoint,
            admission,
            &mut timer,
        );
        let timings = timer.finish();
        let outcome = match &result {
            Ok(_) => "completed",
            Err(NisarSubsetWriteError::Cancelled) => "stopped",
            Err(error) => error.code(),
        };
        eprintln!(
            "{}",
            timings.diagnostic_line(&work.task.task_id, &work.item.item_id, outcome)
        );
        #[cfg(test)]
        crate::nisar_writer_timings_v2::record_last_writer_timings_for_test(timings);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn write_and_validate_v2_stages(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        reported_science: &NisarSubsetScienceTransferResultV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
        failpoint: WriterFailpointV2,
        admission: &mut dyn FnMut(NisarWriterAdmissionEventV2),
        timer: &mut NisarWriterStageTimerV2,
    ) -> Result<NisarSubsetWriteResultV2, NisarSubsetWriteError> {
        let cancelled = || control.is_cancelled() || control.is_paused();
        // Downloader enhancement D1: the artifact lease and locks and an
        // output slot are taken first, before any full material hash, and
        // held until this writer returns. A wait observes pause and cancel.
        timer.enter(NisarWriterStageV2::Admission);
        let _admission = admit_nisar_writer_v2(
            &self.output_root,
            &self.destination_lease,
            &self.output_slots_v2,
            &completed.frozen_plan().provenance.derived_artifact_id,
            self.long_wait_after_v2,
            &cancelled,
            admission,
        )?;
        // Downloader enhancement D2: the first of the two digests of the
        // completed material; the checks until the final digest compare the
        // identities of the pinned handles and the workspace child instead.
        timer.enter(NisarWriterStageV2::EntryRevalidation);
        let first = revalidate_complete_science_for_writer_v2(
            work,
            completed,
            reported_science,
            authority,
            control,
        )
        .map_err(|error| {
            v2_science_revalidation_error(error, NisarSubsetWriteError::IncompleteScienceAuthority)
        })?;
        let selection = VerifiedSubsetWriteSelection::from_revalidated(work, completed, &first)?;
        #[cfg(test)]
        if matches!(
            failpoint,
            WriterFailpointV2::ReplacePartBeforeMaterialCapture
                | WriterFailpointV2::SymlinkPartBeforeMaterialCapture
                | WriterFailpointV2::HardlinkPartBeforeMaterialCapture
                | WriterFailpointV2::WritePartBeforeMaterialCapture
                | WriterFailpointV2::TruncatePartBeforeMaterialCapture
                | WriterFailpointV2::TruncateRestorePartBeforeMaterialCapture
        ) {
            attack_v2_part_for_test(
                first.part_path(),
                failpoint,
                selection
                    .physical_plan
                    .science_ranges()
                    .first()
                    .map(|range| range.start())
                    .ok_or(NisarSubsetWriteError::IncompleteScienceAuthority)?,
            )?;
        }
        first
            .verify_part_identity()
            .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
        let pinned_source_path = first.pinned_hdf5_path();
        #[cfg(test)]
        if failpoint == WriterFailpointV2::MutateSourceDuringHdf5ReadAndRestore {
            let mutation = mutate_source_during_hdf5_read_and_restore_for_test(
                first.part_path(),
                &pinned_source_path,
                selection
                    .physical_plan
                    .science_ranges()
                    .first()
                    .map(|range| range.start())
                    .ok_or(NisarSubsetWriteError::IncompleteScienceAuthority)?,
            )?;
            record_part_attack_outcome_for_test(part_attack_outcome_for_test(mutation));
            let mutation_succeeded = mutation.is_ok();
            if mutation_succeeded && first.verify_part_identity().is_ok() {
                return Err(NisarSubsetWriteError::PostWriteScienceValidation);
            }
            return Err(NisarSubsetWriteError::IncompleteScienceAuthority);
        }
        #[cfg(test)]
        if failpoint == WriterFailpointV2::ReplaceSnapshotPathDuringHdf5Read {
            #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
            replace_snapshot_path_during_hdf5_read_for_test(
                first.part_path(),
                &first.writer_snapshot_path_v2_for_test(),
                &pinned_source_path,
            )?;
            #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
            return Err(NisarSubsetWriteError::IncompleteScienceAuthority);
        }
        let descriptors = build_v2_window_materials(&selection)?;
        // Issue #8, slice 6: the derived output is checked against the shared
        // output bound before any bulk read. The windows are then streamed
        // from the pinned source, so the working set no longer grows with the
        // AOI; free space is checked before a new candidate is created.
        let output_bytes =
            v2_check_output_limit(&selection.physical_plan).map_err(map_v2_output_refusal)?;
        #[cfg(test)]
        if matches!(
            failpoint,
            WriterFailpointV2::PauseBeforeMetadataCapture
                | WriterFailpointV2::PauseThenResumeDuringMetadataCapture
        ) {
            control.set_paused_for_test(true);
        }
        timer.enter(NisarWriterStageV2::MetadataCapture);
        // A pause while the metadata closure is read is a pause, not a
        // binding failure (found by the downloader enhancement D1 tests).
        let metadata =
            capture_v2_writer_metadata_closure(&selection, &pinned_source_path, &cancelled);
        #[cfg(test)]
        if failpoint == WriterFailpointV2::PauseThenResumeDuringMetadataCapture {
            control.set_paused_for_test(false);
        }
        // The stop is read from the error, not from the worker flags, which
        // the scheduler clears when the item is resumed at once (review of
        // D1/D2, downloader enhancement D1b).
        let metadata = metadata?;
        let required_output_bytes = output_bytes
            .checked_add(v2_metadata_closure_bytes(&metadata)?)
            .ok_or(NisarSubsetWriteError::LogicalCaptureLimitExceeded)?;
        first
            .verify_part_identity()
            .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
        timer.enter(NisarWriterStageV2::WindowCapture);
        let materials =
            capture_v2_window_materials(&selection, &pinned_source_path, descriptors, &cancelled)?;
        first
            .verify_part_identity()
            .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
        self.output_root
            .verify()
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let artifact_publication = self
            .output_root
            .publication_capability(&selection.artifact_file_name)
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        artifact_publication
            .verify()
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        if artifact_publication.component() != selection.artifact_file_name {
            return Err(NisarSubsetWriteError::OutputUnsafe);
        }
        let artifact_path = self
            .output_root
            .join_component(&selection.artifact_file_name)
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let artifact_result_path = self
            .output_root
            .canonical_component(&selection.artifact_file_name)
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let sidecar_name = format!("{}.artifact-manifest-v2.json", selection.artifact_file_name);
        let manifest_path = self
            .output_root
            .join_component(&sidecar_name)
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let manifest_result_path = self
            .output_root
            .canonical_component(&sidecar_name)
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        let candidate_name = format!(
            ".nisar-subset-writer-v2-{}.writing",
            selection.derived_artifact_id
        );
        let temp_path = self
            .output_root
            .join_component(&candidate_name)
            .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }

        let candidate_entry = output_entry_kind(&temp_path)?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if candidate_entry != OutputEntryKind::Missing {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        let artifact_entry = output_entry_kind(&artifact_path)?;
        let manifest_entry = output_entry_kind(&manifest_path)?;
        let recovery_anchor = self
            .writer_recovery_anchor_v2
            .as_deref()
            .ok_or(NisarSubsetWriteError::WriterBinding)?;
        let recovery_lineage_key = writer_recovery_lineage_key_v2(&selection)?;
        let mut recovery_record = recovery_anchor.load(&recovery_lineage_key)?;
        if let Some(record) = &recovery_record {
            record.validate_for_selection(&selection)?;
        }
        if artifact_entry != OutputEntryKind::Missing || manifest_entry != OutputEntryKind::Missing
        {
            timer.enter(NisarWriterStageV2::ArtifactRecovery);
            if artifact_entry != OutputEntryKind::Regular
                || manifest_entry == OutputEntryKind::Other
                || candidate_entry != OutputEntryKind::Missing
            {
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
            first
                .revalidate_authority(
                    work,
                    completed,
                    reported_science,
                    authority,
                    control,
                    WriterMaterialCheckV2::Identity,
                )
                .map_err(|error| {
                    v2_science_revalidation_error(error, NisarSubsetWriteError::RecoveryInvalid)
                })?;
            let artifact_guard = open_v2_artifact_immutable(&artifact_path, &artifact_publication)
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            let validation = validate_v2_candidate(
                &artifact_guard,
                &selection,
                &metadata,
                &materials,
                &pinned_source_path,
                &cancelled,
                WriterFailpointV2::None,
            )
            .map_err(|error| match error {
                // A pause or a transient read error during the streamed
                // validation is not a recovery failure; the next run
                // validates again.
                NisarSubsetWriteError::Cancelled | NisarSubsetWriteError::Persistence => error,
                _ => NisarSubsetWriteError::RecoveryInvalid,
            })?;
            first
                .verify_part_identity()
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            artifact_guard
                .verify_identity()
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            let anchored = recovery_record
                .clone()
                .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
            anchored.validate_artifact_recovery(
                &artifact_guard.identity,
                validation.artifact_size_bytes,
                &validation.artifact_sha256,
            )?;
            if anchored.state() == NisarSubsetWriterRecoveryStateV2::CandidateValidated {
                artifact_guard
                    .verify_identity()
                    .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
                let promoted = anchored.mark_artifact_promoted(
                    artifact_guard.identity.clone(),
                    validation.artifact_size_bytes,
                    &validation.artifact_sha256,
                )?;
                if !recovery_anchor.compare_and_swap(
                    &recovery_lineage_key,
                    Some(&anchored),
                    &promoted,
                )? {
                    return Err(NisarSubsetWriteError::RecoveryInvalid);
                }
                artifact_guard
                    .verify_identity()
                    .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
                recovery_record = Some(promoted);
            } else if !matches!(
                anchored.state(),
                NisarSubsetWriterRecoveryStateV2::ArtifactPromoted
                    | NisarSubsetWriterRecoveryStateV2::Complete
            ) {
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
            #[cfg(test)]
            if failpoint == WriterFailpointV2::PauseBeforeArtifactRecoveryRevalidation {
                control.set_paused_for_test(true);
            }
            // A pause here is a pause, not a recovery failure (review of
            // slice 7, P2-2): the next run recovers the artifact again.
            // Downloader enhancement D2: the second and last digest of the
            // completed material, before the recovered artifact completes.
            timer.enter(NisarWriterStageV2::FinalRevalidation);
            first
                .revalidate_authority(
                    work,
                    completed,
                    reported_science,
                    authority,
                    control,
                    WriterMaterialCheckV2::FinalDigest,
                )
                .map_err(|error| {
                    v2_science_revalidation_error(error, NisarSubsetWriteError::RecoveryInvalid)
                })?;
            timer.enter(NisarWriterStageV2::Publication);
            let manifest = build_v2_manifest(&selection, &materials, validation)?;
            artifact_guard
                .verify_identity()
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            if manifest_entry == OutputEntryKind::Regular {
                let stored = serde_json::from_str::<NisarSubsetArtifactManifestV2>(
                    &read_guarded_text(&manifest_path)?,
                )
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
                if stored != manifest {
                    return Err(NisarSubsetWriteError::RecoveryInvalid);
                }
            } else {
                persist_v2_manifest(&manifest_path, &manifest)?;
            }
            artifact_guard
                .verify_identity()
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            validate_v2_manifest_sidecar(&manifest_path, &manifest)?;
            artifact_guard
                .verify_identity()
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            let anchored = recovery_record
                .clone()
                .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
            artifact_guard
                .verify_identity()
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            if anchored.state() == NisarSubsetWriterRecoveryStateV2::ArtifactPromoted {
                let complete = anchored.transition_complete()?;
                if !recovery_anchor.compare_and_swap(
                    &recovery_lineage_key,
                    Some(&anchored),
                    &complete,
                )? {
                    return Err(NisarSubsetWriteError::RecoveryInvalid);
                }
                artifact_guard
                    .verify_identity()
                    .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            } else if anchored.state() != NisarSubsetWriterRecoveryStateV2::Complete {
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
            artifact_guard
                .verify_identity()
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
            return Ok(NisarSubsetWriteResultV2 {
                manifest,
                artifact_path: artifact_result_path,
                manifest_path: manifest_result_path,
            });
        }

        #[cfg(any(windows, target_os = "macos"))]
        let candidate_reused = candidate_entry == OutputEntryKind::Regular
            && recovery_record.as_ref().is_some_and(|record| {
                record.state() == NisarSubsetWriterRecoveryStateV2::CandidateValidated
            });
        #[cfg(not(any(windows, target_os = "macos")))]
        let candidate_reused = false;
        let mut _candidate_cleanup = V2CandidateCleanup::new(temp_path.clone());
        #[cfg(any(windows, target_os = "macos"))]
        if candidate_entry == OutputEntryKind::Other {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        #[cfg(any(windows, target_os = "macos"))]
        if candidate_entry == OutputEntryKind::Regular && !candidate_reused {
            let anchored = recovery_record
                .as_ref()
                .filter(|record| {
                    record.state() == NisarSubsetWriterRecoveryStateV2::CandidateBuilding
                })
                .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
            #[cfg(test)]
            V2_REGULAR_BUILDING_RECOVERY_HITS.fetch_add(1, Ordering::Relaxed);
            if !remove_candidate_if_identity_matches(&temp_path, &anchored.candidate_identity)? {
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
        }
        #[cfg(any(windows, target_os = "macos"))]
        if candidate_entry == OutputEntryKind::Missing
            && recovery_record.as_ref().is_some_and(|record| {
                record.state() == NisarSubsetWriterRecoveryStateV2::CandidateValidated
            })
        {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        first
            .revalidate_authority(
                work,
                completed,
                reported_science,
                authority,
                control,
                WriterMaterialCheckV2::Identity,
            )
            .map_err(|error| {
                v2_science_revalidation_error(
                    error,
                    NisarSubsetWriteError::IncompleteScienceAuthority,
                )
            })?;

        timer.enter(NisarWriterStageV2::CandidateWrite);
        let candidate = if candidate_reused {
            let anchored = recovery_record
                .as_ref()
                .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
            _candidate_cleanup.bind_identity(&anchored.candidate_identity);
            // Kept by default (review of slice 7, P1-2): only proof that this
            // is not the anchored candidate removes it.
            _candidate_cleanup.keep();
            let candidate = match V2CandidateFileGuard::open_named_immutable(&temp_path) {
                Ok(candidate) => candidate,
                Err(_) => {
                    return Err(v2_kept_candidate_unreadable(
                        &temp_path,
                        anchored,
                        recovery_anchor,
                        &recovery_lineage_key,
                        &mut _candidate_cleanup,
                    ))
                }
            };
            // Compare the whole file with the anchored size and SHA-256 before
            // anything else: a mismatch proves corruption; a read failure is
            // local and retryable.
            let sha256 = match candidate.sha256(&cancelled) {
                Ok(sha256) => sha256,
                Err(NisarSubsetWriteError::Cancelled) => {
                    return Err(NisarSubsetWriteError::Cancelled)
                }
                Err(NisarSubsetWriteError::Persistence) => {
                    return Err(v2_kept_candidate_unreadable(
                        &temp_path,
                        anchored,
                        recovery_anchor,
                        &recovery_lineage_key,
                        &mut _candidate_cleanup,
                    ))
                }
                Err(_) => return Err(NisarSubsetWriteError::RecoveryInvalid),
            };
            if anchored
                .validate_candidate_material(&candidate.identity, candidate.identity.size, &sha256)
                .is_err()
            {
                _candidate_cleanup.discard();
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
            candidate
        } else {
            if recovery_record.as_ref().is_some_and(|record| {
                matches!(
                    record.state(),
                    NisarSubsetWriterRecoveryStateV2::ArtifactPromoted
                        | NisarSubsetWriterRecoveryStateV2::Complete
                )
            }) {
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
            // Free space is required only before a new candidate is created;
            // recovering an existing artifact or candidate writes no samples.
            v2_check_disk_space(self.output_root.path(), required_output_bytes)
                .map_err(map_v2_output_refusal)?;
            let mut bootstrap = V2CandidateBootstrap::create(&temp_path)?;
            #[cfg(test)]
            if failpoint == WriterFailpointV2::AfterCandidateBootstrapBeforeBuildingAnchor {
                return Err(NisarSubsetWriteError::InjectedFailure);
            }
            let building = match recovery_record.as_ref() {
                None => NisarSubsetWriterRecoveryAnchorRecordV2::candidate_building(
                    &selection,
                    bootstrap.identity().clone(),
                )?,
                Some(record)
                    if record.state() == NisarSubsetWriterRecoveryStateV2::CandidateBuilding =>
                {
                    record.rebind_building(bootstrap.identity().clone())?
                }
                Some(record)
                    if record.state() == NisarSubsetWriterRecoveryStateV2::CandidateValidated =>
                {
                    record.rebuild_lost_validated(bootstrap.identity().clone())?
                }
                Some(_) => return Err(NisarSubsetWriteError::RecoveryInvalid),
            };
            if recovery_record.as_ref() != Some(&building) {
                if !recovery_anchor.compare_and_swap(
                    &recovery_lineage_key,
                    recovery_record.as_ref(),
                    &building,
                )? {
                    return Err(NisarSubsetWriteError::RecoveryInvalid);
                }
                recovery_record = Some(building);
            }
            bootstrap.persist_for_write()?;
            #[cfg(any(windows, target_os = "macos"))]
            _candidate_cleanup.bind_identity(bootstrap.identity());
            #[cfg(test)]
            if failpoint == WriterFailpointV2::AfterBuildingAnchorBeforeWrite {
                _candidate_cleanup.preserve_for_crash();
                return Err(NisarSubsetWriteError::InjectedFailure);
            }
            write_v2_candidate(
                &bootstrap.hdf5_path(),
                &selection,
                &metadata,
                &materials,
                &pinned_source_path,
                &cancelled,
            )?;
            first
                .verify_part_identity()
                .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
            #[cfg(test)]
            if failpoint == WriterFailpointV2::AfterCandidateWriteBeforeSync {
                _candidate_cleanup.preserve_for_crash();
                return Err(NisarSubsetWriteError::InjectedFailure);
            }
            #[cfg(test)]
            mutate_v2_candidate_before_seal_for_test(
                &bootstrap.hdf5_path(),
                &materials,
                failpoint,
            )?;
            bootstrap.sync()?;
            #[cfg(test)]
            if failpoint == WriterFailpointV2::AfterCandidateSyncBeforeSeal {
                _candidate_cleanup.preserve_for_crash();
                return Err(NisarSubsetWriteError::InjectedFailure);
            }
            let candidate = bootstrap.seal()?;
            #[cfg(any(windows, target_os = "macos"))]
            _candidate_cleanup.bind_identity(&candidate.identity);
            #[cfg(test)]
            if failpoint == WriterFailpointV2::AfterCandidateSealBeforeSemantic {
                _candidate_cleanup.preserve_for_crash();
                return Err(NisarSubsetWriteError::InjectedFailure);
            }
            candidate
        };
        #[cfg(test)]
        if candidate_reused && is_v2_preseal_mutation_failpoint(failpoint) {
            return Err(NisarSubsetWriteError::PostWriteScienceValidation);
        }
        #[cfg(test)]
        if matches!(
            failpoint,
            WriterFailpointV2::AfterCandidateSemanticBeforeHash
                | WriterFailpointV2::AfterCandidateHashBeforeValidatedAnchor
        ) {
            _candidate_cleanup.preserve_for_crash();
        }
        timer.enter(NisarWriterStageV2::CandidateValidation);
        let validation = match validate_v2_candidate(
            &candidate,
            &selection,
            &metadata,
            &materials,
            &pinned_source_path,
            &cancelled,
            failpoint,
        ) {
            Ok(validation) => validation,
            // A pause stays a pause; a local read failure stays retryable
            // (a kept candidate unreadable twice in a row is rebuilt).
            Err(NisarSubsetWriteError::Persistence) if candidate_reused => {
                let anchored = recovery_record
                    .as_ref()
                    .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
                return Err(v2_kept_candidate_unreadable(
                    &temp_path,
                    anchored,
                    recovery_anchor,
                    &recovery_lineage_key,
                    &mut _candidate_cleanup,
                ));
            }
            Err(
                error @ (NisarSubsetWriteError::Cancelled | NisarSubsetWriteError::Persistence),
            ) => return Err(error),
            // A reused candidate matched its anchored SHA-256, so any other
            // failure refuses this recovery but leaves the candidate in place.
            Err(_) if candidate_reused => return Err(NisarSubsetWriteError::RecoveryInvalid),
            Err(error) => return Err(error),
        };
        if candidate_reused {
            clear_v2_kept_candidate_read_failures(&temp_path);
        }
        #[cfg(test)]
        if failpoint == WriterFailpointV2::WritePartAfterCandidateValidation {
            attack_v2_part_for_test(
                first.part_path(),
                failpoint,
                selection
                    .physical_plan
                    .science_ranges()
                    .first()
                    .map(|range| range.start())
                    .ok_or(NisarSubsetWriteError::IncompleteScienceAuthority)?,
            )?;
        }
        first
            .verify_part_identity()
            .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
        #[cfg(test)]
        if failpoint == WriterFailpointV2::AfterCandidateHashBeforeValidatedAnchor {
            return Err(NisarSubsetWriteError::InjectedFailure);
        }
        let anchored = recovery_record
            .clone()
            .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
        if candidate_reused {
            if let Err(error) = anchored.validate_candidate_material(
                &candidate.identity,
                validation.artifact_size_bytes,
                &validation.artifact_sha256,
            ) {
                _candidate_cleanup.discard();
                return Err(error);
            }
        } else {
            let validated = anchored.mark_candidate_validated(
                candidate.identity.clone(),
                validation.artifact_size_bytes,
                &validation.artifact_sha256,
            )?;
            let validated_cas = || {
                recovery_anchor.compare_and_swap(&recovery_lineage_key, Some(&anchored), &validated)
            };
            #[cfg(test)]
            let validated_cas = match failpoint {
                WriterFailpointV2::ValidatedAnchorStoppedAfterCommit => {
                    validated_cas().and(Err(NisarSubsetWriteError::Cancelled))
                }
                WriterFailpointV2::ValidatedAnchorStoppedBeforeCommit => {
                    Err(NisarSubsetWriteError::Cancelled)
                }
                _ => validated_cas(),
            };
            #[cfg(not(test))]
            let validated_cas = validated_cas();
            // Review of slice 7 (P1-1): the coordinator may commit a CAS
            // whose reply reports a stop (a pause while awaiting it) or a
            // local persistence failure. The outcome is unknown, so the
            // candidate is kept: the next run reuses it under a committed
            // CandidateValidated anchor, or removes and rewrites it under a
            // CandidateBuilding anchor.
            let validated_cas = match validated_cas {
                Ok(swapped) => swapped,
                Err(
                    error @ (NisarSubsetWriteError::Cancelled | NisarSubsetWriteError::Persistence),
                ) => {
                    _candidate_cleanup.keep();
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            if !validated_cas {
                return Err(NisarSubsetWriteError::RecoveryInvalid);
            }
            recovery_record = Some(validated);
            // From here on the candidate is the anchored recovery state.
            _candidate_cleanup.keep();
        }
        #[cfg(test)]
        if failpoint == WriterFailpointV2::PauseAfterValidatedAnchor {
            control.set_paused_for_test(true);
        }
        let manifest = build_v2_manifest(&selection, &materials, validation)?;
        candidate
            .verify_identity()
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        #[cfg(test)]
        if failpoint == WriterFailpointV2::AfterValidationBeforePromotion {
            _candidate_cleanup.preserve_for_crash();
            return Err(NisarSubsetWriteError::InjectedFailure);
        }
        #[cfg(test)]
        if failpoint == WriterFailpointV2::WritePartImmediatelyBeforePromotion {
            attack_v2_part_for_test(
                first.part_path(),
                failpoint,
                selection
                    .physical_plan
                    .science_ranges()
                    .first()
                    .map(|range| range.start())
                    .ok_or(NisarSubsetWriteError::IncompleteScienceAuthority)?,
            )?;
        }
        // From here on the candidate is anchored as CandidateValidated and
        // kept on any error. Downloader enhancement D2: the second and last
        // digest of the completed material, right before promotion.
        timer.enter(NisarWriterStageV2::FinalRevalidation);
        first
            .revalidate_authority(
                work,
                completed,
                reported_science,
                authority,
                control,
                WriterMaterialCheckV2::FinalDigest,
            )
            .map_err(|error| {
                v2_science_revalidation_error(
                    error,
                    NisarSubsetWriteError::IncompleteScienceAuthority,
                )
            })?;
        timer.enter(NisarWriterStageV2::Publication);
        #[cfg(test)]
        if failpoint == WriterFailpointV2::AtomicPromotionFailureBeforeRename {
            return Err(NisarSubsetWriteError::AtomicPromotion);
        }
        #[cfg(test)]
        let force_copy_promotion = failpoint == WriterFailpointV2::ForceCopyPromotionToNewInode;
        #[cfg(not(test))]
        let force_copy_promotion = false;
        let installed = promote_v2_candidate(
            candidate,
            &temp_path,
            &manifest.artifact_sha256,
            &artifact_path,
            &artifact_publication,
            &cancelled,
            force_copy_promotion,
        )?;
        // The published artifact is the recovery state from here on. A source
        // candidate left behind by a copy promotion is removed (as before the
        // keep rules): next to a published artifact it would make the next
        // run's recovery refuse.
        _candidate_cleanup.discard();
        self.output_root
            .verify()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        sync_parent_directory(&artifact_path)
            .map_err(|_| NisarSubsetWriteError::AtomicPromotion)?;
        let installed_sha256 = installed.sha256(&cancelled)?;
        #[cfg(test)]
        if failpoint == WriterFailpointV2::ReplacePublishedArtifactBeforeArtifactAnchor {
            let replaced =
                replace_published_artifact_after_open_for_test(&artifact_path, &installed)?;
            let still_bound = installed.verify_identity().is_ok();
            if replaced == still_bound {
                return Err(NisarSubsetWriteError::InjectedFailure);
            }
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        #[cfg(test)]
        if matches!(
            failpoint,
            WriterFailpointV2::AfterArtifactPromotionBeforeAnchor
                | WriterFailpointV2::ForceCopyPromotionToNewInode
        ) {
            return Err(NisarSubsetWriteError::InjectedFailure);
        }
        let anchored = recovery_record
            .clone()
            .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
        installed
            .verify_identity()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        let promoted = anchored.mark_artifact_promoted(
            installed.identity.clone(),
            manifest.artifact_size_bytes,
            &installed_sha256,
        )?;
        if !recovery_anchor.compare_and_swap(&recovery_lineage_key, Some(&anchored), &promoted)? {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        installed
            .verify_identity()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        recovery_record = Some(promoted);
        #[cfg(test)]
        if failpoint == WriterFailpointV2::AfterArtifactPromotion {
            return Err(NisarSubsetWriteError::InjectedFailure);
        }
        installed
            .verify_identity()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        persist_v2_manifest(&manifest_path, &manifest)?;
        installed
            .verify_identity()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        validate_v2_manifest_sidecar(&manifest_path, &manifest)?;
        installed
            .verify_identity()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        self.output_root
            .verify()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        if installed.sha256(&cancelled)? != manifest.artifact_sha256 {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        let anchored = recovery_record
            .as_ref()
            .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
        if anchored.state() != NisarSubsetWriterRecoveryStateV2::ArtifactPromoted {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        installed
            .verify_identity()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        let complete = anchored.transition_complete()?;
        if !recovery_anchor.compare_and_swap(&recovery_lineage_key, Some(anchored), &complete)? {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        installed
            .verify_identity()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        drop(first);
        installed
            .verify_identity()
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        Ok(NisarSubsetWriteResultV2 {
            manifest,
            artifact_path: artifact_result_path,
            manifest_path: manifest_result_path,
        })
    }

    #[cfg(test)]
    pub(crate) fn write_and_validate_v2_with_failpoint_for_test(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        reported_science: &NisarSubsetScienceTransferResultV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
        failpoint: WriterFailpointV2,
    ) -> Result<NisarSubsetWriteResultV2, NisarSubsetWriteError> {
        self.write_and_validate_v2_inner(
            work,
            completed,
            reported_science,
            authority,
            control,
            failpoint,
            &mut |_| {},
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_and_validate_v2_observed_with_failpoint_for_test(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        reported_science: &NisarSubsetScienceTransferResultV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
        failpoint: WriterFailpointV2,
        admission: &mut dyn FnMut(NisarWriterAdmissionEventV2),
    ) -> Result<NisarSubsetWriteResultV2, NisarSubsetWriteError> {
        self.write_and_validate_v2_inner(
            work,
            completed,
            reported_science,
            authority,
            control,
            failpoint,
            admission,
        )
    }
}

#[cfg(feature = "nisar-subset-production")]
fn build_v2_window_materials(
    selection: &VerifiedSubsetWriteSelection,
) -> Result<Vec<DerivedWindowMaterialV2>, NisarSubsetWriteError> {
    let plan = &selection.physical_plan;
    let science_root = selection.science_root()?;
    let derived_root = science_root.derived_root();
    if plan.dataset_mappings().len() != plan.selected_polarizations().len() {
        return Err(NisarSubsetWriteError::MultiPolarizationInvalid);
    }
    let mut line_names = BTreeMap::<(String, u64, u64), String>::new();
    let mut sample_names = BTreeMap::<(String, u64, u64), String>::new();
    let mut mask_names = BTreeMap::<(String, u64, u64, u64, u64), String>::new();
    let mut materials = Vec::new();
    for mapping in plan.dataset_mappings() {
        if mapping.source_dataset_path()
            != science_root.selected_science_path(
                frequency_label(plan.selected_frequency()),
                mapping.polarization(),
            )
        {
            return Err(NisarSubsetWriteError::WriterBinding);
        }
        for (ordinal, window) in mapping.windows().iter().enumerate() {
            let root_path = format!(
                "{derived_root}/component{:03}/window{ordinal:03}/swaths/frequency{}",
                window.component_id(),
                frequency_label(plan.selected_frequency())
            );
            let line_key = (
                root_path.clone(),
                window.line_start(),
                window.line_end_exclusive(),
            );
            let line_ordinal = line_names
                .keys()
                .filter(|(root, _, _)| root == &root_path)
                .count();
            let line_scale_name = line_names
                .entry(line_key)
                .or_insert_with(|| {
                    if line_ordinal == 0 {
                        "zeroDopplerTime".to_owned()
                    } else {
                        format!("zeroDopplerTime{line_ordinal:03}")
                    }
                })
                .clone();
            let sample_key = (
                root_path.clone(),
                window.sample_start(),
                window.sample_end_exclusive(),
            );
            let sample_ordinal = sample_names
                .keys()
                .filter(|(root, _, _)| root == &root_path)
                .count();
            let sample_scale_name = sample_names
                .entry(sample_key)
                .or_insert_with(|| {
                    if sample_ordinal == 0 {
                        "slantRange".to_owned()
                    } else {
                        format!("slantRange{sample_ordinal:03}")
                    }
                })
                .clone();
            let mask_key = (
                root_path.clone(),
                window.line_start(),
                window.line_end_exclusive(),
                window.sample_start(),
                window.sample_end_exclusive(),
            );
            let mask_ordinal = mask_names
                .keys()
                .filter(|(root, _, _, _, _)| root == &root_path)
                .count();
            let mask_name = mask_names
                .entry(mask_key)
                .or_insert_with(|| {
                    if mask_ordinal == 0 {
                        "inputDataExceptionMask".to_owned()
                    } else {
                        format!("inputDataExceptionMask{mask_ordinal:03}")
                    }
                })
                .clone();
            let shape = [
                window.line_end_exclusive() - window.line_start(),
                window.sample_end_exclusive() - window.sample_start(),
            ];
            materials.push(DerivedWindowMaterialV2 {
                component_id: window.component_id(),
                component_sha256: window.component_sha256().to_owned(),
                window_sha256: window.window_sha256().to_owned(),
                polarization: mapping.polarization().to_owned(),
                source_dataset_path: mapping.source_dataset_path().to_owned(),
                science_path: format!("{root_path}/{}", mapping.polarization()),
                root_path,
                line_start: window.line_start(),
                line_end_exclusive: window.line_end_exclusive(),
                sample_start: window.sample_start(),
                sample_end_exclusive: window.sample_end_exclusive(),
                source_shape: mapping.dataset_shape(),
                shape,
                chunk_shape: mapping.chunk_shape(),
                line_scale_name,
                sample_scale_name,
                mask_name,
            });
        }
    }
    materials.sort_by(|left, right| {
        (
            left.component_id,
            left.root_path.as_str(),
            left.polarization.as_str(),
        )
            .cmp(&(
                right.component_id,
                right.root_path.as_str(),
                right.polarization.as_str(),
            ))
    });
    let actual = materials
        .iter()
        .map(|material| material.polarization.as_str())
        .collect::<BTreeSet<_>>();
    let expected = plan
        .selected_polarizations()
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if materials.is_empty() || actual != expected {
        return Err(NisarSubsetWriteError::MultiPolarizationInvalid);
    }
    Ok(materials)
}

#[cfg(feature = "nisar-subset-production")]
fn ensure_v2_group(file: &File, path: &str) -> Result<Group, NisarSubsetWriteError> {
    if let Ok(group) = file.group(path) {
        return Ok(group);
    }
    let mut current = String::new();
    for component in path.split('/').filter(|component| !component.is_empty()) {
        current.push('/');
        current.push_str(component);
        if file.group(&current).is_err() {
            file.create_group(&current)
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
        }
    }
    file.group(path)
        .map_err(|_| NisarSubsetWriteError::Persistence)
}

#[cfg(test)]
fn reset_v2_axis_read_observation_for_test() {
    V2_AXIS_MAX_READ_VALUES_OBSERVED.store(0, Ordering::SeqCst);
}

#[cfg(test)]
fn v2_axis_max_read_values_for_test() -> usize {
    V2_AXIS_MAX_READ_VALUES_OBSERVED.load(Ordering::SeqCst)
}

#[cfg(feature = "nisar-subset-production")]
fn read_v2_axis_window(
    dataset: &Dataset,
    start: u64,
    end_exclusive: u64,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<f64>, NisarSubsetWriteError> {
    let shape = dataset.shape();
    if shape.len() != 1 {
        return Err(NisarSubsetWriteError::WriterBinding);
    }
    let source_values = shape[0];
    let start = usize::try_from(start).map_err(|_| NisarSubsetWriteError::WriterBinding)?;
    let end_exclusive =
        usize::try_from(end_exclusive).map_err(|_| NisarSubsetWriteError::WriterBinding)?;
    if start >= end_exclusive
        || end_exclusive > source_values
        || dataset
            .dtype()
            .map_err(|_| NisarSubsetWriteError::WriterBinding)?
            != hdf5::Datatype::from_type::<f64>()
                .map_err(|_| NisarSubsetWriteError::WriterBinding)?
    {
        return Err(NisarSubsetWriteError::WriterBinding);
    }
    let requested_values = end_exclusive - start;
    if requested_values
        .checked_mul(std::mem::size_of::<f64>())
        .is_none_or(|bytes| bytes > MAX_WRITER_WORKING_SET_BYTES)
    {
        return Err(NisarSubsetWriteError::LogicalCaptureLimitExceeded);
    }
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    let mut values = Vec::with_capacity(requested_values);
    let mut cursor = start;
    while cursor < end_exclusive {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let next = cursor
            .saturating_add(V2_AXIS_READ_CHUNK_VALUES)
            .min(end_exclusive);
        let read_values = next - cursor;
        #[cfg(test)]
        V2_AXIS_MAX_READ_VALUES_OBSERVED.fetch_max(read_values, Ordering::SeqCst);
        let slice = dataset
            .read_slice_1d::<f64, _>(cursor..next)
            .map_err(|_| NisarSubsetWriteError::SelectedScienceMissing)?;
        if slice.len() != read_values {
            return Err(NisarSubsetWriteError::WriterBinding);
        }
        values.extend(slice.iter().copied());
        cursor = next;
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
    }
    Ok(values)
}

#[cfg(feature = "nisar-subset-production")]
fn read_v2_axes(
    source: &File,
    selection: &VerifiedSubsetWriteSelection,
    material: &DerivedWindowMaterialV2,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(Vec<f64>, Vec<f64>), NisarSubsetWriteError> {
    let science_root = selection.science_root()?;
    let line_axis = source
        .dataset(&science_root.zero_doppler_time_path())
        .map_err(|_| NisarSubsetWriteError::SelectedScienceMissing)?;
    let line_window = read_v2_axis_window(
        &line_axis,
        material.line_start,
        material.line_end_exclusive,
        cancelled,
    )?;
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    let sample_axis = source
        .dataset(&science_root.slant_range_path(frequency_label(
            selection.physical_plan.selected_frequency(),
        )))
        .map_err(|_| NisarSubsetWriteError::SelectedScienceMissing)?;
    let sample_window = read_v2_axis_window(
        &sample_axis,
        material.sample_start,
        material.sample_end_exclusive,
        cancelled,
    )?;
    Ok((line_window, sample_window))
}

#[cfg(feature = "nisar-subset-production")]
fn open_v2_source_science(
    source: &File,
    material: &DerivedWindowMaterialV2,
) -> Result<Dataset, NisarSubsetWriteError> {
    let dataset = source
        .dataset(&material.source_dataset_path)
        .map_err(|_| NisarSubsetWriteError::SelectedScienceMissing)?;
    if dataset.shape()
        != [
            usize::try_from(material.source_shape[0]).unwrap_or(usize::MAX),
            usize::try_from(material.source_shape[1]).unwrap_or(usize::MAX),
        ]
    {
        return Err(NisarSubsetWriteError::WriterBinding);
    }
    if !v2_dataset_covers_window(&dataset, material) {
        return Err(NisarSubsetWriteError::SelectedScienceMissing);
    }
    Ok(dataset)
}

#[cfg(feature = "nisar-subset-production")]
fn open_v2_source_mask(
    source: &File,
    selection: &VerifiedSubsetWriteSelection,
) -> Result<Dataset, NisarSubsetWriteError> {
    let path = selection.science_root()?.mask_path(frequency_label(
        selection.physical_plan.selected_frequency(),
    ));
    source
        .dataset(&path)
        .map_err(|_| NisarSubsetWriteError::SelectedScienceMissing)
}

/// A source dataset must hold the whole window that the writer streams.
#[cfg(feature = "nisar-subset-production")]
fn v2_dataset_covers_window(dataset: &Dataset, material: &DerivedWindowMaterialV2) -> bool {
    let shape = dataset.shape();
    shape.len() == 2
        && material.line_start < material.line_end_exclusive
        && material.sample_start < material.sample_end_exclusive
        && u64::try_from(shape[0]).is_ok_and(|rows| material.line_end_exclusive <= rows)
        && u64::try_from(shape[1]).is_ok_and(|cols| material.sample_end_exclusive <= cols)
}

/// Writer pass errors: a source read keeps the selected-science code, a
/// candidate write is a local persistence failure.
#[cfg(feature = "nisar-subset-production")]
fn map_v2_stream_write_error(error: V2StreamError) -> NisarSubsetWriteError {
    match error {
        V2StreamError::Cancelled => NisarSubsetWriteError::Cancelled,
        V2StreamError::SourceRead => NisarSubsetWriteError::SelectedScienceMissing,
        V2StreamError::CandidateWrite | V2StreamError::CandidateRead | V2StreamError::Mismatch => {
            NisarSubsetWriteError::Persistence
        }
    }
}

/// Validation pass errors: a pause stays a pause and a read failure is a
/// local, retryable I/O failure; only differing bits (or a write in the
/// validation pass) fail the candidate as a post-write validation failure.
#[cfg(feature = "nisar-subset-production")]
fn map_v2_stream_verify_error(error: V2StreamError) -> NisarSubsetWriteError {
    match error {
        V2StreamError::Cancelled => NisarSubsetWriteError::Cancelled,
        V2StreamError::SourceRead | V2StreamError::CandidateRead => {
            NisarSubsetWriteError::Persistence
        }
        V2StreamError::CandidateWrite | V2StreamError::Mismatch => {
            NisarSubsetWriteError::PostWriteScienceValidation
        }
    }
}

/// A science revalidation the worker's pause or cancellation stopped is a
/// pause, not missing authority or a recovery failure. The stop is read from
/// the error, never from the worker flags afterwards: the scheduler clears the
/// pause of a still-running worker when the item is resumed at once, which
/// used to turn such a pause into a non-retryable failure (review of D1/D2,
/// downloader enhancement D1b).
#[cfg(feature = "nisar-subset-production")]
fn v2_science_revalidation_error(
    error: NisarSubsetScienceTransferErrorV2,
    fallback: NisarSubsetWriteError,
) -> NisarSubsetWriteError {
    match error {
        NisarSubsetScienceTransferErrorV2::Paused
        | NisarSubsetScienceTransferErrorV2::Cancelled => NisarSubsetWriteError::Cancelled,
        _ => fallback,
    }
}

#[cfg(feature = "nisar-subset-production")]
fn derived_v2_mask_chunk_shape(
    source_mask: &Dataset,
    science_chunk_shape: [u64; 2],
) -> Result<[u64; 2], NisarSubsetWriteError> {
    if science_chunk_shape.contains(&0) {
        return Err(NisarSubsetWriteError::WriterBinding);
    }
    match source_mask.chunk() {
        Some(chunk) if chunk.len() == 2 && chunk.iter().all(|value| *value > 0) => {
            Ok([chunk[0] as u64, chunk[1] as u64])
        }
        Some(_) => Err(NisarSubsetWriteError::WriterBinding),
        None => Ok(science_chunk_shape),
    }
}

/// Captures what streaming needs from the source before any candidate byte:
/// each window's axis values, the mask chunk layout and both tile plans, and
/// checks that the selected science and the mask hold the window. Samples are
/// streamed later in the write and validation passes and never retained.
#[cfg(feature = "nisar-subset-production")]
fn capture_v2_window_materials(
    selection: &VerifiedSubsetWriteSelection,
    source_path: &Path,
    descriptors: Vec<DerivedWindowMaterialV2>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<CapturedDerivedWindowMaterialV2>, NisarSubsetWriteError> {
    let source =
        crate::hdf5_file::open(source_path).map_err(|_| NisarSubsetWriteError::WriterBinding)?;
    let source_mask = open_v2_source_mask(&source, selection)?;
    let budget = v2_stream_band_budget();
    let mut captured = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let (line_axis, sample_axis) = read_v2_axes(&source, selection, &descriptor, cancelled)?;
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        drop(open_v2_source_science(&source, &descriptor)?);
        if !v2_dataset_covers_window(&source_mask, &descriptor) {
            return Err(NisarSubsetWriteError::SelectedScienceMissing);
        }
        let mask_chunk_shape = derived_v2_mask_chunk_shape(&source_mask, descriptor.chunk_shape)?;
        let science_tiles = v2_stream_tiles(
            descriptor.shape,
            descriptor.chunk_shape,
            std::mem::size_of::<CFloat32>(),
            budget,
        )
        .ok_or(NisarSubsetWriteError::LogicalCaptureLimitExceeded)?;
        let mask_tiles = v2_stream_tiles(
            descriptor.shape,
            mask_chunk_shape,
            std::mem::size_of::<u8>(),
            budget,
        )
        .ok_or(NisarSubsetWriteError::LogicalCaptureLimitExceeded)?;
        captured.push(CapturedDerivedWindowMaterialV2 {
            descriptor,
            line_axis,
            sample_axis,
            mask_chunk_shape,
            science_tiles,
            mask_tiles,
        });
    }
    drop(source_mask);
    drop(source);
    Ok(captured)
}

/// Writes the candidate. Each science and mask window is streamed tile by
/// tile from the pinned source (`source_path`); objects are created in the
/// same order as before, so the bytes do not depend on the tile budget.
#[cfg(feature = "nisar-subset-production")]
fn write_v2_candidate(
    path: &Path,
    selection: &VerifiedSubsetWriteSelection,
    metadata: &VerifiedWriterMetadataClosureV2,
    materials: &[CapturedDerivedWindowMaterialV2],
    source_path: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), NisarSubsetWriteError> {
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    let source =
        crate::hdf5_file::open(source_path).map_err(|_| NisarSubsetWriteError::WriterBinding)?;
    let source_mask = open_v2_source_mask(&source, selection)?;
    let native = crate::long_path::native_path(path);
    let file = crate::hdf5_file::builder(&native)
        .with_fcpl(|plist| plist.obj_track_times(false))
        .create(&native)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    create_source_groups(&file, &metadata.group_bindings, cancelled)?;
    for binding in &metadata.dataset_bindings {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let (parent_path, name) = dataset_parent_and_name(&binding.path)?;
        let parent = file
            .group(parent_path)
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        let dataset = create_raw_dataset_like(
            &parent,
            name,
            binding,
            binding
                .value
                .as_deref()
                .ok_or(NisarSubsetWriteError::WriterBinding)?,
        )?;
        copy_attributes(&dataset, &binding.attributes)?;
    }
    restore_dimension_scale_graph(&file, &metadata.dataset_bindings, cancelled)?;
    let science_root = selection.science_root()?;
    let identification_path = science_root.identification_path();
    let provenance_path = science_root.provenance_path();
    let identification = ensure_v2_group(&file, &identification_path)?;
    write_ascii_scalar::<256>(&identification, "granuleId", &selection.derived_artifact_id)?;
    write_ascii_scalar::<32>(
        &identification,
        "outputAuthority",
        NISAR_SUBSET_OUTPUT_AUTHORITY,
    )?;
    write_ascii_scalar::<32>(&identification, "qualityStatus", "PROVISIONAL")?;
    write_ascii_scalar::<64>(
        &identification,
        "derivedProductType",
        OfflineNisarSubsetWriter::DERIVED_PRODUCT_TYPE,
    )?;
    write_ascii_scalar::<256>(
        &identification,
        "sourceProductId",
        &selection
            .frozen_plan
            .source_product_identity
            .provider_product_id,
    )?;
    let provenance = ensure_v2_group(&file, &provenance_path)?;
    for (name, value, capacity) in [
        (
            "preparedIntentSha256",
            selection.frozen_plan.prepared_intent_sha256.as_str(),
            96,
        ),
        (
            "finalSemanticPlanSha256",
            selection
                .frozen_plan
                .canonical_semantic_plan_digest
                .as_str(),
            96,
        ),
        (
            "physicalPlanSha256",
            selection.physical_plan.physical_plan_sha256(),
            96,
        ),
        (
            "writerSelectionSha256",
            selection.selection_sha256.as_str(),
            96,
        ),
        (
            "scienceCheckpointSha256",
            selection.science_checkpoint_sha256.as_str(),
            96,
        ),
        (
            "completedRangeEvidenceSha256",
            selection.completed_range_evidence_sha256.as_str(),
            96,
        ),
        (
            "requestedGeometrySha256",
            selection
                .frozen_plan
                .canonical_geometry_digest
                .requested_sha256
                .as_str(),
            96,
        ),
        (
            "effectiveGeometrySha256",
            selection
                .frozen_plan
                .canonical_geometry_digest
                .effective_sha256
                .as_str(),
            96,
        ),
        (
            "inventorySha256",
            selection.physical_plan.inventory_sha256(),
            96,
        ),
        (
            "inventoryEvidenceSha256",
            selection.physical_plan.inventory_evidence_sha256(),
            96,
        ),
        (
            "dependencyGraphSha256",
            selection.physical_plan.source_graph_sha256(),
            96,
        ),
        (
            "sourceValidatorSha256",
            selection.physical_plan.source_validator_sha256(),
            96,
        ),
        (
            "derivedArtifactId",
            selection.derived_artifact_id.as_str(),
            256,
        ),
        ("outputAuthority", NISAR_SUBSET_OUTPUT_AUTHORITY, 32),
        ("qualityStatus", "PROVISIONAL", 32),
        ("writerId", OfflineNisarSubsetWriter::WRITER_ID, 64),
    ] {
        match capacity {
            32 => write_ascii_scalar::<32>(&provenance, name, value)?,
            64 => write_ascii_scalar::<64>(&provenance, name, value)?,
            96 => write_ascii_scalar::<96>(&provenance, name, value)?,
            256 => write_ascii_scalar::<256>(&provenance, name, value)?,
            _ => return Err(NisarSubsetWriteError::Persistence),
        }
    }
    write_ascii_scalar::<16>(
        &provenance,
        "selectedFrequency",
        frequency_label(selection.physical_plan.selected_frequency()),
    )?;
    write_ascii_scalar::<64>(
        &provenance,
        "selectedPolarizations",
        &selection.physical_plan.selected_polarizations().join("+"),
    )?;
    write_ascii_scalar::<256>(
        &provenance,
        "sourceAssetKey",
        &selection.frozen_plan.source_asset_identity.stable_asset_key,
    )?;
    write_ascii_scalar::<128>(
        &provenance,
        "sourceValidator",
        &selection.physical_plan.source_validator().value,
    )?;
    write_scalar(
        &provenance,
        "sourceSize",
        &selection.physical_plan.source_size(),
    )?;

    let mut initialized_roots = BTreeSet::new();
    for material in materials {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let group = ensure_v2_group(&file, &material.root_path)?;
        let line_path = format!("{}/{}", material.root_path, material.line_scale_name);
        if file.dataset(&line_path).is_err() {
            write_vector(&group, &material.line_scale_name, &material.line_axis)?;
            let scale = file
                .dataset(&line_path)
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
            dimension_scales::set_scale(&scale, "zeroDopplerTime")
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
        }
        let sample_path = format!("{}/{}", material.root_path, material.sample_scale_name);
        if file.dataset(&sample_path).is_err() {
            write_vector(&group, &material.sample_scale_name, &material.sample_axis)?;
            let scale = file
                .dataset(&sample_path)
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
            dimension_scales::set_scale(&scale, "slantRange")
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
        }
        let rows =
            usize::try_from(material.shape[0]).map_err(|_| NisarSubsetWriteError::WriterBinding)?;
        let cols =
            usize::try_from(material.shape[1]).map_err(|_| NisarSubsetWriteError::WriterBinding)?;
        let science = group
            .new_dataset::<CFloat32>()
            .shape((rows, cols))
            .chunk((
                rows.min(usize::try_from(material.chunk_shape[0]).unwrap_or(rows)),
                cols.min(usize::try_from(material.chunk_shape[1]).unwrap_or(cols)),
            ))
            .create(material.polarization.as_str())
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        let source_science = open_v2_source_science(&source, material)?;
        v2_stream_copy::<CFloat32>(
            &source_science,
            [material.line_start, material.sample_start],
            &science,
            &material.science_tiles,
            cancelled,
        )
        .map_err(map_v2_stream_write_error)?;
        drop(source_science);
        dimension_scales::set_label(&science, 0, "zeroDopplerTime")
            .and_then(|_| dimension_scales::set_label(&science, 1, "slantRange"))
            .and_then(|_| dimension_scales::attach_scale(&file, &science, &line_path, 0))
            .and_then(|_| dimension_scales::attach_scale(&file, &science, &sample_path, 1))
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        let mask_path = format!("{}/{}", material.root_path, material.mask_name);
        if file.dataset(&mask_path).is_err() {
            let mask = group
                .new_dataset::<u8>()
                .shape((rows, cols))
                .chunk((
                    rows.min(usize::try_from(material.mask_chunk_shape[0]).unwrap_or(rows)),
                    cols.min(usize::try_from(material.mask_chunk_shape[1]).unwrap_or(cols)),
                ))
                .create(material.mask_name.as_str())
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
            v2_stream_copy::<u8>(
                &source_mask,
                [material.line_start, material.sample_start],
                &mask,
                &material.mask_tiles,
                cancelled,
            )
            .map_err(map_v2_stream_write_error)?;
            dimension_scales::set_label(&mask, 0, "zeroDopplerTime")
                .and_then(|_| dimension_scales::set_label(&mask, 1, "slantRange"))
                .and_then(|_| dimension_scales::attach_scale(&file, &mask, &line_path, 0))
                .and_then(|_| dimension_scales::attach_scale(&file, &mask, &sample_path, 1))
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
        }
        if initialized_roots.insert(material.root_path.clone()) {
            write_scalar(&group, "componentId", &material.component_id)?;
            write_ascii_scalar::<96>(&group, "componentSha256", &material.component_sha256)?;
            write_ascii_scalar::<64>(
                &group,
                "selectedPolarizations",
                &selection.physical_plan.selected_polarizations().join("+"),
            )?;
        }
        let mapping = ensure_v2_group(
            &file,
            &format!(
                "{}/sourceMappings/{}",
                material.root_path, material.polarization
            ),
        )?;
        write_ascii_scalar::<512>(&mapping, "sourceDatasetPath", &material.source_dataset_path)?;
        write_ascii_scalar::<96>(&mapping, "windowSha256", &material.window_sha256)?;
        write_scalar(&mapping, "lineStart", &material.line_start)?;
        write_scalar(&mapping, "lineEndExclusive", &material.line_end_exclusive)?;
        write_scalar(&mapping, "sampleStart", &material.sample_start)?;
        write_scalar(
            &mapping,
            "sampleEndExclusive",
            &material.sample_end_exclusive,
        )?;
        write_vector(&mapping, "sourceShape", &material.source_shape)?;
        write_vector(&mapping, "outputShape", &material.shape)?;
        write_vector(&mapping, "chunkShape", &material.chunk_shape)?;
    }
    file.flush()
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    drop(file);
    drop(source_mask);
    drop(source);
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
struct CandidateValidationV2 {
    artifact_size_bytes: u64,
    artifact_sha256: String,
}

#[cfg(feature = "nisar-subset-production")]
fn validate_v2_candidate(
    candidate: &V2CandidateFileGuard,
    selection: &VerifiedSubsetWriteSelection,
    metadata: &VerifiedWriterMetadataClosureV2,
    materials: &[CapturedDerivedWindowMaterialV2],
    source_path: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
    failpoint: WriterFailpointV2,
) -> Result<CandidateValidationV2, NisarSubsetWriteError> {
    validate_derived_hdf5_security(&candidate.hdf5_path())
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
    candidate
        .verify_identity()
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
    #[cfg(test)]
    let open_fault = V2_CANDIDATE_OPEN_FAULT.with(|fault| fault.replace(false));
    #[cfg(not(test))]
    let open_fault = false;
    // The candidate passed the security and identity checks above; failing to
    // open it is a local, retryable read failure (review of slice 7, P1-2).
    let file = if open_fault {
        Err(hdf5::Error::Internal(
            "injected candidate open failure".to_owned(),
        ))
    } else {
        crate::hdf5_file::open(candidate.hdf5_path())
    }
    .map_err(|_| NisarSubsetWriteError::Persistence)?;
    if !file.is_read_only() {
        return Err(NisarSubsetWriteError::PostWriteScienceValidation);
    }
    validate_v2_metadata_closure(&file, metadata)?;
    // The expected samples are re-read from the pinned source tile by tile;
    // the caller revalidates the source material around this pass.
    let source = crate::hdf5_file::open(source_path)
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
    let source_mask = open_v2_source_mask(&source, selection)
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
    let science_root = selection.science_root()?;
    let identification_path = science_root.identification_path();
    let provenance_path = science_root.provenance_path();
    let identification = file
        .group(&identification_path)
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
    for (name, expected, capacity) in [
        ("granuleId", selection.derived_artifact_id.as_str(), 256),
        ("outputAuthority", NISAR_SUBSET_OUTPUT_AUTHORITY, 32),
        ("qualityStatus", "PROVISIONAL", 32),
        (
            "derivedProductType",
            OfflineNisarSubsetWriter::DERIVED_PRODUCT_TYPE,
            64,
        ),
        (
            "sourceProductId",
            selection
                .frozen_plan
                .source_product_identity
                .provider_product_id
                .as_str(),
            256,
        ),
    ] {
        if read_ascii_with_capacity(&identification, name, capacity)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
            != expected
        {
            return Err(NisarSubsetWriteError::PostWriteScienceValidation);
        }
    }
    let provenance = file
        .group(&provenance_path)
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
    let selected_polarizations = selection.physical_plan.selected_polarizations().join("+");
    for (name, expected, capacity) in [
        (
            "preparedIntentSha256",
            selection.frozen_plan.prepared_intent_sha256.as_str(),
            96,
        ),
        (
            "finalSemanticPlanSha256",
            selection
                .frozen_plan
                .canonical_semantic_plan_digest
                .as_str(),
            96,
        ),
        (
            "physicalPlanSha256",
            selection.physical_plan.physical_plan_sha256(),
            96,
        ),
        (
            "writerSelectionSha256",
            selection.selection_sha256.as_str(),
            96,
        ),
        (
            "scienceCheckpointSha256",
            selection.science_checkpoint_sha256.as_str(),
            96,
        ),
        (
            "completedRangeEvidenceSha256",
            selection.completed_range_evidence_sha256.as_str(),
            96,
        ),
        (
            "requestedGeometrySha256",
            selection
                .frozen_plan
                .canonical_geometry_digest
                .requested_sha256
                .as_str(),
            96,
        ),
        (
            "effectiveGeometrySha256",
            selection
                .frozen_plan
                .canonical_geometry_digest
                .effective_sha256
                .as_str(),
            96,
        ),
        (
            "inventorySha256",
            selection.physical_plan.inventory_sha256(),
            96,
        ),
        (
            "inventoryEvidenceSha256",
            selection.physical_plan.inventory_evidence_sha256(),
            96,
        ),
        (
            "dependencyGraphSha256",
            selection.physical_plan.source_graph_sha256(),
            96,
        ),
        (
            "sourceValidatorSha256",
            selection.physical_plan.source_validator_sha256(),
            96,
        ),
        (
            "derivedArtifactId",
            selection.derived_artifact_id.as_str(),
            256,
        ),
        (
            "selectedFrequency",
            frequency_label(selection.physical_plan.selected_frequency()),
            16,
        ),
        ("selectedPolarizations", selected_polarizations.as_str(), 64),
        (
            "sourceAssetKey",
            selection
                .frozen_plan
                .source_asset_identity
                .stable_asset_key
                .as_str(),
            256,
        ),
        (
            "sourceValidator",
            selection.physical_plan.source_validator().value.as_str(),
            128,
        ),
        ("outputAuthority", NISAR_SUBSET_OUTPUT_AUTHORITY, 32),
        ("qualityStatus", "PROVISIONAL", 32),
        ("writerId", OfflineNisarSubsetWriter::WRITER_ID, 64),
    ] {
        if read_ascii_with_capacity(&provenance, name, capacity)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
            != expected
        {
            return Err(NisarSubsetWriteError::PostWriteScienceValidation);
        }
    }
    if read_scalar_exact::<u64>(&provenance, "sourceSize")
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
        != selection.physical_plan.source_size()
    {
        return Err(NisarSubsetWriteError::PostWriteScienceValidation);
    }
    let selected = selection
        .physical_plan
        .selected_polarizations()
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for material in materials {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        if !selected.contains(material.polarization.as_str()) {
            return Err(NisarSubsetWriteError::MultiPolarizationInvalid);
        }
        let dataset = file
            .dataset(&material.science_path)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        validate_dataset_shape_type::<CFloat32>(&dataset, material.shape, material.chunk_shape)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        let source_science = open_v2_source_science(&source, material)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        v2_stream_verify::<CFloat32>(
            &source_science,
            [material.line_start, material.sample_start],
            &dataset,
            &material.science_tiles,
            cancelled,
        )
        .map_err(map_v2_stream_verify_error)?;
        drop(source_science);
        let graph = dimension_scales::read_graph(&dataset)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        if graph.labels != ["zeroDopplerTime", "slantRange"]
            || graph.scales
                != [
                    vec![format!(
                        "{}/{}",
                        material.root_path, material.line_scale_name
                    )],
                    vec![format!(
                        "{}/{}",
                        material.root_path, material.sample_scale_name
                    )],
                ]
        {
            return Err(NisarSubsetWriteError::PostWriteScienceValidation);
        }
        let expected_line = &material.line_axis;
        let expected_sample = &material.sample_axis;
        let group = file
            .group(&material.root_path)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        let line_scale = group
            .dataset(&material.line_scale_name)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        let sample_scale = group
            .dataset(&material.sample_scale_name)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        let actual_line =
            read_exact::<f64>(&group, &material.line_scale_name, &[expected_line.len()])
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        let actual_sample = read_exact::<f64>(
            &group,
            &material.sample_scale_name,
            &[expected_sample.len()],
        )
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        if !f64_bitwise_eq(&actual_line, expected_line)
            || !f64_bitwise_eq(&actual_sample, expected_sample)
            || read_dimension_scale_graph(&line_scale)
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != (
                    vec![String::new()],
                    vec![Vec::new()],
                    Some("zeroDopplerTime".to_owned()),
                    true,
                )
            || read_dimension_scale_graph(&sample_scale)
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != (
                    vec![String::new()],
                    vec![Vec::new()],
                    Some("slantRange".to_owned()),
                    true,
                )
        {
            return Err(NisarSubsetWriteError::PostWriteScienceValidation);
        }
        let mask_path = format!("{}/{}", material.root_path, material.mask_name);
        let mask_dataset = file
            .dataset(&mask_path)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        validate_dataset_shape_type::<u8>(&mask_dataset, material.shape, material.mask_chunk_shape)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        let mask_graph = dimension_scales::read_graph(&mask_dataset)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        if mask_graph.labels != ["zeroDopplerTime", "slantRange"]
            || mask_graph.scales
                != [
                    vec![format!(
                        "{}/{}",
                        material.root_path, material.line_scale_name
                    )],
                    vec![format!(
                        "{}/{}",
                        material.root_path, material.sample_scale_name
                    )],
                ]
        {
            return Err(NisarSubsetWriteError::PostWriteScienceValidation);
        }
        v2_stream_verify::<u8>(
            &source_mask,
            [material.line_start, material.sample_start],
            &mask_dataset,
            &material.mask_tiles,
            cancelled,
        )
        .map_err(map_v2_stream_verify_error)?;
        if read_scalar_exact::<u16>(&group, "componentId")
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
            != material.component_id
            || read_ascii_with_capacity(&group, "componentSha256", 96)
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != material.component_sha256
            || read_ascii_with_capacity(&group, "selectedPolarizations", 64)
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != selected_polarizations
        {
            return Err(NisarSubsetWriteError::PostWriteScienceValidation);
        }
        let mapping = file
            .group(&format!(
                "{}/sourceMappings/{}",
                material.root_path, material.polarization
            ))
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        if read_ascii_with_capacity(&mapping, "sourceDatasetPath", 512)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
            != material.source_dataset_path
            || read_ascii_with_capacity(&mapping, "windowSha256", 96)
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != material.window_sha256
            || read_scalar_exact::<u64>(&mapping, "lineStart")
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != material.line_start
            || read_scalar_exact::<u64>(&mapping, "lineEndExclusive")
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != material.line_end_exclusive
            || read_scalar_exact::<u64>(&mapping, "sampleStart")
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != material.sample_start
            || read_scalar_exact::<u64>(&mapping, "sampleEndExclusive")
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != material.sample_end_exclusive
            || read_exact::<u64>(&mapping, "sourceShape", &[2])
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != material.source_shape
            || read_exact::<u64>(&mapping, "outputShape", &[2])
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != material.shape
            || read_exact::<u64>(&mapping, "chunkShape", &[2])
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != material.chunk_shape
        {
            return Err(NisarSubsetWriteError::PostWriteScienceValidation);
        }
    }
    let root = file
        .group("/")
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
    let mut actual_graph = V2ObjectGraph::default();
    actual_graph.groups.insert("/".to_owned());
    collect_v2_object_paths(&root, "", &mut actual_graph)?;
    let expected_graph = expected_v2_object_graph(selection, metadata, materials)?;
    if actual_graph != expected_graph {
        return Err(NisarSubsetWriteError::PostWriteScienceValidation);
    }
    drop(file);
    drop(source_mask);
    drop(source);
    candidate
        .verify_identity()
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
    #[cfg(test)]
    if failpoint == WriterFailpointV2::AfterCandidateSemanticBeforeHash {
        return Err(NisarSubsetWriteError::InjectedFailure);
    }
    #[cfg(test)]
    if failpoint == WriterFailpointV2::MutateCandidateAfterSemanticBeforeHash {
        if mutate_candidate_after_semantic_before_hash_for_test(&candidate.hdf5_path())? {
            return Err(NisarSubsetWriteError::InjectedFailure);
        }
        return Err(NisarSubsetWriteError::PostWriteScienceValidation);
    }
    #[cfg(not(test))]
    let _ = failpoint;
    // A read failure of the whole-file hash is local and retryable, not a
    // validation failure (review of slice 7, P1-2).
    let artifact_sha256 = candidate.sha256(cancelled).map_err(|error| match error {
        NisarSubsetWriteError::Cancelled | NisarSubsetWriteError::Persistence => error,
        _ => NisarSubsetWriteError::PostWriteScienceValidation,
    })?;
    Ok(CandidateValidationV2 {
        artifact_size_bytes: candidate.identity.size,
        artifact_sha256,
    })
}

#[cfg(feature = "nisar-subset-production")]
fn validate_v2_metadata_closure(
    file: &File,
    metadata: &VerifiedWriterMetadataClosureV2,
) -> Result<(), NisarSubsetWriteError> {
    for binding in &metadata.group_bindings {
        let group = file
            .group(&binding.path)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        if read_attribute_bindings(&group)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
            != binding.attributes
        {
            return Err(NisarSubsetWriteError::PostWriteScienceValidation);
        }
    }
    for binding in &metadata.dataset_bindings {
        let dataset = file
            .dataset(&binding.path)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        let datatype = dataset
            .dtype()
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        let decoded_id = decode_datatype_hid(&binding.datatype_encoded)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        let equal =
            hdf5::sync::sync(|| unsafe { hdf5_sys::h5t::H5Tequal(decoded_id, datatype.id()) });
        hdf5::sync::sync(|| unsafe { H5Tclose(decoded_id) });
        let expected_shape = binding
            .shape
            .iter()
            .map(|value| usize::try_from(*value).unwrap_or(usize::MAX))
            .collect::<Vec<_>>();
        if equal != 1
            || dataset.shape() != expected_shape
            || source_filter_bindings(&dataset)
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != binding.filters
            || read_attribute_bindings(&dataset)
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != binding.attributes
            || hash_dataset_values(&dataset, &|| false)
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != binding
                    .value
                    .as_deref()
                    .map(sha256_bytes)
                    .ok_or(NisarSubsetWriteError::PostWriteScienceValidation)?
            || read_dimension_scale_graph(&dataset)
                .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?
                != (
                    binding.dimension_labels.clone(),
                    binding.dimension_scales.clone(),
                    binding.scale_name.clone(),
                    binding.is_dimension_scale,
                )
        {
            return Err(NisarSubsetWriteError::PostWriteScienceValidation);
        }
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Default, Debug, PartialEq, Eq)]
struct V2ObjectGraph {
    groups: BTreeSet<String>,
    datasets: BTreeSet<String>,
}

#[cfg(feature = "nisar-subset-production")]
fn collect_v2_object_paths(
    group: &Group,
    parent: &str,
    graph: &mut V2ObjectGraph,
) -> Result<(), NisarSubsetWriteError> {
    let mut names = group
        .member_names()
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
    names.sort();
    for name in names {
        let path = format!("{parent}/{name}");
        let info = group
            .loc_info_by_name(&name)
            .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
        match info.loc_type {
            LocationType::Group => {
                if !graph.groups.insert(path.clone()) {
                    return Err(NisarSubsetWriteError::PostWriteScienceValidation);
                }
                let child = group
                    .group(&name)
                    .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
                collect_v2_object_paths(&child, &path, graph)?;
            }
            LocationType::Dataset => {
                if !graph.datasets.insert(path) {
                    return Err(NisarSubsetWriteError::PostWriteScienceValidation);
                }
            }
            _ => return Err(NisarSubsetWriteError::PostWriteScienceValidation),
        }
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
fn expected_v2_object_graph(
    selection: &VerifiedSubsetWriteSelection,
    metadata: &VerifiedWriterMetadataClosureV2,
    materials: &[CapturedDerivedWindowMaterialV2],
) -> Result<V2ObjectGraph, NisarSubsetWriteError> {
    let science_root = selection.science_root()?;
    let identification_path = science_root.identification_path();
    let provenance_path = science_root.provenance_path();
    let mut graph = V2ObjectGraph::default();
    graph.groups.insert("/".to_owned());
    for binding in &metadata.group_bindings {
        insert_v2_expected_group(&mut graph, &binding.path)?;
    }
    for binding in &metadata.dataset_bindings {
        insert_v2_expected_dataset(&mut graph, &binding.path)?;
    }
    for (group, names) in [
        (
            identification_path.as_str(),
            &[
                "granuleId",
                "outputAuthority",
                "qualityStatus",
                "derivedProductType",
                "sourceProductId",
            ][..],
        ),
        (
            provenance_path.as_str(),
            &[
                "preparedIntentSha256",
                "finalSemanticPlanSha256",
                "physicalPlanSha256",
                "writerSelectionSha256",
                "scienceCheckpointSha256",
                "completedRangeEvidenceSha256",
                "requestedGeometrySha256",
                "effectiveGeometrySha256",
                "inventorySha256",
                "inventoryEvidenceSha256",
                "dependencyGraphSha256",
                "sourceValidatorSha256",
                "derivedArtifactId",
                "outputAuthority",
                "qualityStatus",
                "writerId",
                "selectedFrequency",
                "selectedPolarizations",
                "sourceAssetKey",
                "sourceValidator",
                "sourceSize",
            ][..],
        ),
    ] {
        insert_v2_expected_group(&mut graph, group)?;
        for name in names {
            insert_v2_expected_dataset(&mut graph, &format!("{group}/{name}"))?;
        }
    }
    for material in materials {
        insert_v2_expected_group(&mut graph, &material.root_path)?;
        for path in [
            material.science_path.clone(),
            format!("{}/{}", material.root_path, material.line_scale_name),
            format!("{}/{}", material.root_path, material.sample_scale_name),
            format!("{}/{}", material.root_path, material.mask_name),
            format!("{}/componentId", material.root_path),
            format!("{}/componentSha256", material.root_path),
            format!("{}/selectedPolarizations", material.root_path),
        ] {
            insert_v2_expected_dataset(&mut graph, &path)?;
        }
        let mapping = format!(
            "{}/sourceMappings/{}",
            material.root_path, material.polarization
        );
        insert_v2_expected_group(&mut graph, &mapping)?;
        for name in [
            "sourceDatasetPath",
            "windowSha256",
            "lineStart",
            "lineEndExclusive",
            "sampleStart",
            "sampleEndExclusive",
            "sourceShape",
            "outputShape",
            "chunkShape",
        ] {
            insert_v2_expected_dataset(&mut graph, &format!("{mapping}/{name}"))?;
        }
    }
    Ok(graph)
}

#[cfg(feature = "nisar-subset-production")]
fn insert_v2_expected_group(
    graph: &mut V2ObjectGraph,
    path: &str,
) -> Result<(), NisarSubsetWriteError> {
    if path == "/" {
        graph.groups.insert("/".to_owned());
        return Ok(());
    }
    if !path.starts_with('/') || path.ends_with('/') || path.contains("//") {
        return Err(NisarSubsetWriteError::WriterBinding);
    }
    let mut current = String::new();
    for component in path.split('/').skip(1) {
        if component.is_empty() || component == "." || component == ".." {
            return Err(NisarSubsetWriteError::WriterBinding);
        }
        current.push('/');
        current.push_str(component);
        graph.groups.insert(current.clone());
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
fn insert_v2_expected_dataset(
    graph: &mut V2ObjectGraph,
    path: &str,
) -> Result<(), NisarSubsetWriteError> {
    let (parent, _) = path
        .rsplit_once('/')
        .filter(|(_, name)| !name.is_empty())
        .ok_or(NisarSubsetWriteError::WriterBinding)?;
    insert_v2_expected_group(graph, if parent.is_empty() { "/" } else { parent })?;
    graph.datasets.insert(path.to_owned());
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
fn build_v2_manifest(
    selection: &VerifiedSubsetWriteSelection,
    materials: &[CapturedDerivedWindowMaterialV2],
    validation: CandidateValidationV2,
) -> Result<NisarSubsetArtifactManifestV2, NisarSubsetWriteError> {
    let manifest = NisarSubsetArtifactManifestV2 {
        manifest_contract: "NisarSubsetArtifactManifestV2".to_owned(),
        schema_version: 2,
        task_id: selection.task_id.clone(),
        item_id: selection.item_id.clone(),
        output_authority: NISAR_SUBSET_OUTPUT_AUTHORITY.to_owned(),
        quality_status: "PROVISIONAL".to_owned(),
        derived_artifact_id: selection.derived_artifact_id.clone(),
        artifact_file_name: selection.artifact_file_name.clone(),
        artifact_size_bytes: validation.artifact_size_bytes,
        artifact_sha256: validation.artifact_sha256,
        prepared_intent_sha256: selection.frozen_plan.prepared_intent_sha256.clone(),
        final_semantic_plan_sha256: selection.frozen_plan.canonical_semantic_plan_digest.clone(),
        physical_plan_sha256: selection.physical_plan.physical_plan_sha256().to_owned(),
        writer_selection_sha256: selection.selection_sha256.clone(),
        science_checkpoint_sha256: selection.science_checkpoint_sha256.clone(),
        completed_range_evidence_sha256: selection.completed_range_evidence_sha256.clone(),
        source_product_id: selection
            .frozen_plan
            .source_product_identity
            .provider_product_id
            .clone(),
        source_asset_key: selection
            .frozen_plan
            .source_asset_identity
            .stable_asset_key
            .clone(),
        source_validator_sha256: selection.physical_plan.source_validator_sha256().to_owned(),
        source_size: selection.physical_plan.source_size(),
        requested_geometry_sha256: selection
            .frozen_plan
            .canonical_geometry_digest
            .requested_sha256
            .clone(),
        effective_geometry_sha256: selection
            .frozen_plan
            .canonical_geometry_digest
            .effective_sha256
            .clone(),
        selected_frequency: frequency_label(selection.physical_plan.selected_frequency())
            .to_owned(),
        selected_polarizations: selection.physical_plan.selected_polarizations().to_vec(),
        inventory_sha256: selection.physical_plan.inventory_sha256().to_owned(),
        inventory_evidence_sha256: selection
            .physical_plan
            .inventory_evidence_sha256()
            .to_owned(),
        dependency_graph_sha256: selection.physical_plan.source_graph_sha256().to_owned(),
        dataset_windows: materials
            .iter()
            .map(|material| NisarSubsetArtifactWindowV2 {
                component_id: material.component_id,
                component_sha256: material.component_sha256.clone(),
                window_sha256: material.window_sha256.clone(),
                polarization: material.polarization.clone(),
                source_dataset_path: material.source_dataset_path.clone(),
                line_start: material.line_start,
                line_end_exclusive: material.line_end_exclusive,
                sample_start: material.sample_start,
                sample_end_exclusive: material.sample_end_exclusive,
                shape: material.shape,
                chunk_shape: material.chunk_shape,
            })
            .collect(),
        complete: true,
        indeterminate_science_charge: selection.indeterminate_science_charge()?,
    };
    if !valid_v2_sha256(&manifest.artifact_sha256)
        || manifest.artifact_size_bytes == 0
        || manifest.dataset_windows.is_empty()
        || manifest.selected_polarizations.is_empty()
    {
        return Err(NisarSubsetWriteError::ManifestInvalid);
    }
    Ok(manifest)
}

#[cfg(feature = "nisar-subset-production")]
fn validate_v2_manifest_sidecar(
    path: &Path,
    expected: &NisarSubsetArtifactManifestV2,
) -> Result<(), NisarSubsetWriteError> {
    let text = read_guarded_text(path).map_err(|_| NisarSubsetWriteError::ManifestInvalid)?;
    let stored = serde_json::from_str::<NisarSubsetArtifactManifestV2>(&text)
        .map_err(|_| NisarSubsetWriteError::ManifestInvalid)?;
    if &stored != expected
        || stored.artifact_size_bytes == 0
        || !valid_v2_sha256(&stored.artifact_sha256)
    {
        return Err(NisarSubsetWriteError::ManifestInvalid);
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
fn persist_v2_manifest(
    destination: &Path,
    manifest: &NisarSubsetArtifactManifestV2,
) -> Result<(), NisarSubsetWriteError> {
    let canonical =
        serde_json::to_string(manifest).map_err(|_| NisarSubsetWriteError::ManifestInvalid)?;
    let parent = destination
        .parent()
        .ok_or(NisarSubsetWriteError::OutputUnsafe)?;
    let temp = parent.join(format!(".artifact-manifest-v2-{}.writing", temp_nonce()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    file.write_all(canonical.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    drop(file);
    promote_verified_closed_artifact(
        VerifiedClosedArtifact::after_scientific_validation(
            temp,
            sha256_bytes(canonical.as_bytes()),
            &|| false,
        )
        .map_err(|_| NisarSubsetWriteError::ManifestInvalid)?,
        destination,
        None,
    )
    .map_err(|_| NisarSubsetWriteError::ManifestInvalid)?;
    sync_parent_directory(destination).map_err(|_| NisarSubsetWriteError::ManifestInvalid)
}

#[cfg(feature = "nisar-subset-production")]
fn valid_v2_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SharedWriterPromotionError {
    Cancelled,
    Failed,
}

#[cfg(all(feature = "nisar-subset-production", any(windows, target_os = "macos")))]
fn open_v2_artifact_immutable(
    path: &Path,
    publication: &PinnedDirectoryPublication,
) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
    publication
        .verify()
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    V2CandidateFileGuard::open_named_immutable(path)?.bind_publication(publication)
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn open_published_v2_child(
    publication: &PinnedDirectoryPublication,
) -> Result<fs::File, NisarSubsetWriteError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    publication
        .verify()
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let directory = publication
        .try_clone_directory_handle()
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let component =
        CString::new(publication.component()).map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let fd = unsafe {
        // SAFETY: the retained verified directory descriptor and bounded exact
        // component remain live for this no-follow child open.
        libc::openat(
            directory.as_raw_fd(),
            component.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(NisarSubsetWriteError::OutputUnsafe);
    }
    let file = unsafe {
        // SAFETY: ownership of the newly returned descriptor is transferred once.
        fs::File::from_raw_fd(fd)
    };
    candidate_file_identity(&file)?;
    Ok(file)
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn open_v2_artifact_immutable(
    _path: &Path,
    publication: &PinnedDirectoryPublication,
) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
    use std::os::unix::fs::PermissionsExt;

    let file = open_published_v2_child(publication)?;
    enable_candidate_fs_verity(&file)?;
    file.set_permissions(fs::Permissions::from_mode(0o400))
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    V2CandidateFileGuard::from_published(file, publication)
}

#[cfg(all(
    feature = "nisar-subset-production",
    not(any(
        windows,
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    ))
))]
fn open_v2_artifact_immutable(
    _path: &Path,
    _publication: &PinnedDirectoryPublication,
) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(all(feature = "nisar-subset-production", any(windows, target_os = "macos")))]
fn promote_v2_candidate(
    candidate: V2CandidateFileGuard,
    candidate_path: &Path,
    candidate_sha256: &str,
    artifact_path: &Path,
    artifact_publication: &PinnedDirectoryPublication,
    cancelled: &(dyn Fn() -> bool + Sync),
    force_copy: bool,
) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
    candidate.verify_identity()?;
    let promotion_source = if force_copy {
        let parent = artifact_path
            .parent()
            .ok_or(NisarSubsetWriteError::OutputUnsafe)?;
        let copy_path = parent.join(format!(
            ".nisar-subset-writer-v2-copy-promotion-{}",
            temp_nonce()
        ));
        fs::copy(candidate.hdf5_path(), &copy_path)
            .map_err(|_| NisarSubsetWriteError::AtomicPromotion)?;
        sync_candidate_path(&copy_path)?;
        copy_path
    } else {
        candidate_path.to_path_buf()
    };
    drop(candidate);
    let promoted = promote_validated_hdf5_core(
        promotion_source.clone(),
        candidate_sha256.to_owned(),
        artifact_path,
        cancelled,
    );
    if promoted.is_err() && force_copy {
        let _ = fs::remove_file(&promotion_source);
    }
    promoted.map_err(|error| match error {
        SharedWriterPromotionError::Cancelled => NisarSubsetWriteError::Cancelled,
        SharedWriterPromotionError::Failed => NisarSubsetWriteError::AtomicPromotion,
    })?;
    V2CandidateFileGuard::open_named_immutable(artifact_path)
        .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?
        .bind_publication(artifact_publication)
        .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn promote_v2_candidate(
    candidate: V2CandidateFileGuard,
    _candidate_path: &Path,
    candidate_sha256: &str,
    _artifact_path: &Path,
    artifact_publication: &PinnedDirectoryPublication,
    cancelled: &(dyn Fn() -> bool + Sync),
    _force_copy: bool,
) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
    candidate.verify_identity()?;
    let mut installed = create_v2_promotion_bootstrap(artifact_publication)?;
    installed.persist_for_write()?;
    let mut source = candidate
        .file
        .try_clone()
        .map_err(|_| NisarSubsetWriteError::AtomicPromotion)?;
    let mut destination = installed
        .file
        .as_ref()
        .ok_or(NisarSubsetWriteError::AtomicPromotion)?
        .try_clone()
        .map_err(|_| NisarSubsetWriteError::AtomicPromotion)?;
    source
        .seek(SeekFrom::Start(0))
        .and_then(|_| destination.seek(SeekFrom::Start(0)))
        .and_then(|_| destination.set_len(0))
        .map_err(|_| NisarSubsetWriteError::AtomicPromotion)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let count = source
            .read(&mut buffer)
            .map_err(|_| NisarSubsetWriteError::AtomicPromotion)?;
        if count == 0 {
            break;
        }
        destination
            .write_all(&buffer[..count])
            .map_err(|_| NisarSubsetWriteError::AtomicPromotion)?;
    }
    destination
        .sync_all()
        .map_err(|_| NisarSubsetWriteError::AtomicPromotion)?;
    drop(destination);
    let installed = installed.seal()?;
    if installed.sha256(cancelled)? != candidate_sha256 {
        return Err(NisarSubsetWriteError::AtomicPromotion);
    }
    let published = publish_unnamed_v2_artifact(&installed.file, artifact_publication)?;
    drop(candidate);
    drop(installed);
    let installed = V2CandidateFileGuard::from_published(published, artifact_publication)?;
    if installed.sha256(cancelled)? != candidate_sha256 {
        return Err(NisarSubsetWriteError::AtomicPromotion);
    }
    Ok(installed)
}

#[cfg(all(
    feature = "nisar-subset-production",
    not(any(
        windows,
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    ))
))]
fn promote_v2_candidate(
    _candidate: V2CandidateFileGuard,
    _candidate_path: &Path,
    _candidate_sha256: &str,
    _artifact_path: &Path,
    _artifact_publication: &PinnedDirectoryPublication,
    _cancelled: &(dyn Fn() -> bool + Sync),
    _force_copy: bool,
) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn publish_unnamed_v2_artifact(
    file: &fs::File,
    publication: &PinnedDirectoryPublication,
) -> Result<fs::File, NisarSubsetWriteError> {
    use std::os::fd::AsRawFd;

    publication
        .verify()
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let directory = publication
        .try_clone_directory_handle()
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let name =
        CString::new(publication.component()).map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let source = CString::new(format!("{V2_PUBLISH_PROC_FD_PREFIX}{}", file.as_raw_fd()))
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let unnamed_identity = unnamed_candidate_file_identity(file)?;
    let linked = unsafe {
        // SAFETY: the exact proc descriptor path names the retained unnamed inode;
        // the verified destination dirfd and bounded component remain live for this
        // no-replace link. Following only the proc fd symlink avoids privileged
        // empty-path linking and never resolves the destination through procfs.
        libc::linkat(
            libc::AT_FDCWD,
            source.as_ptr(),
            directory.as_raw_fd(),
            name.as_ptr(),
            V2_PUBLISH_LINK_FLAGS,
        )
    };
    if linked != 0 {
        return Err(NisarSubsetWriteError::AtomicPromotion);
    }
    directory
        .sync_all()
        .map_err(|_| NisarSubsetWriteError::AtomicPromotion)?;
    let published = open_published_v2_child(publication)?;
    let published_identity = candidate_file_identity(&published)?;
    if unnamed_identity.device_id != published_identity.device_id
        || unnamed_identity.file_id != published_identity.file_id
    {
        return Err(NisarSubsetWriteError::RecoveryInvalid);
    }
    Ok(published)
}

fn promote_validated_hdf5_core(
    candidate_path: PathBuf,
    candidate_sha256: String,
    artifact_path: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), SharedWriterPromotionError> {
    let verified = VerifiedClosedArtifact::after_scientific_validation(
        candidate_path,
        candidate_sha256,
        cancelled,
    )
    .map_err(|_| {
        if cancelled() {
            SharedWriterPromotionError::Cancelled
        } else {
            SharedWriterPromotionError::Failed
        }
    })?;
    promote_verified_closed_artifact(verified, artifact_path, Some(cancelled))
        .map(|_| ())
        .map_err(|_| {
            if cancelled() {
                SharedWriterPromotionError::Cancelled
            } else {
                SharedWriterPromotionError::Failed
            }
        })
}

fn map_destination_lease_error(error: DestinationLeaseError) -> NisarSubsetWriteError {
    match error {
        DestinationLeaseError::Cancelled => NisarSubsetWriteError::Cancelled,
        DestinationLeaseError::InvalidIdentity
        | DestinationLeaseError::Unavailable
        | DestinationLeaseError::InvariantViolation => NisarSubsetWriteError::OutputUnsafe,
    }
}

fn output_entry_kind(path: &Path) -> Result<OutputEntryKind, NisarSubsetWriteError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(OutputEntryKind::Other),
        Ok(metadata) if metadata.is_file() => Ok(OutputEntryKind::Regular),
        Ok(_) => Ok(OutputEntryKind::Other),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(OutputEntryKind::Missing),
        Err(_) => Err(NisarSubsetWriteError::RecoveryInvalid),
    }
}

fn read_guarded_text(path: &Path) -> Result<String, NisarSubsetWriteError> {
    const MAX_SIDECAR_BYTES: u64 = 4 * 1024 * 1024;
    let guard =
        CandidateFileGuard::open(path).map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
    if guard.identity.size > MAX_SIDECAR_BYTES {
        return Err(NisarSubsetWriteError::RecoveryInvalid);
    }
    let mut file = guard
        .file
        .try_clone()
        .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
    let mut text = String::with_capacity(guard.identity.size as usize);
    file.read_to_string(&mut text)
        .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
    guard
        .verify_path_identity()
        .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
    Ok(text)
}

#[derive(Clone)]
struct DerivedWindowMaterial {
    window: NisarSubsetLogicalWindowV1,
    root_path: String,
    science_path: String,
    mask_path: Option<String>,
    mask_chunk_shape: Option<[u64; 2]>,
    line_coordinates: Vec<f64>,
    sample_coordinates: Vec<f64>,
}

fn build_window_materials(
    request: &NisarSubsetRequestManifestV1,
    blocks: &[VerifiedHdf5ChunkBlockV1],
    metadata: &NisarSubsetMetadataSnapshotV1,
) -> Result<Vec<DerivedWindowMaterial>, NisarSubsetWriteError> {
    let mut block_map = BTreeMap::new();
    for block in blocks {
        let key = (
            block.component_id,
            block.dataset_path.clone(),
            block.logical_chunk,
        );
        if block_map.insert(key, block).is_some() {
            return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
        }
    }
    let mut windows = request.mapping.windows.clone();
    windows.sort_by(|left, right| {
        (
            left.component_id,
            left.dataset_path.as_str(),
            left.line_start,
            left.line_end_exclusive,
            left.sample_start,
            left.sample_end_exclusive,
        )
            .cmp(&(
                right.component_id,
                right.dataset_path.as_str(),
                right.line_start,
                right.line_end_exclusive,
                right.sample_start,
                right.sample_end_exclusive,
            ))
    });
    let mut raw_identities = BTreeSet::new();
    let mut raw_block_bytes = 0_usize;
    let mut source_blocks = BTreeMap::<(String, [u64; 2]), &VerifiedHdf5ChunkBlockV1>::new();
    let mut output_logical_bytes = 0_usize;
    let mut largest_stream_bytes = 0_usize;
    let mut largest_chunk_bytes = 0_usize;
    let metadata_bytes = metadata_working_set_bytes(metadata)?;

    // Preflight every allocation before decoding source chunks or materializing
    // output windows. This keeps the working-set limit an admission bound, not
    // a post-allocation observation.
    for block in blocks {
        if let VerifiedHdf5ChunkAllocationV1::Allocated { bytes } = &block.allocation {
            let identity = (bytes.as_ptr() as usize, bytes.len());
            if raw_identities.insert(identity) {
                raw_block_bytes = raw_block_bytes
                    .checked_add(bytes.len())
                    .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
            }
        }
        let bytes = chunk_elements(block)?
            .checked_mul(block.datatype.element_size())
            .filter(|bytes| *bytes <= MAX_DECODED_CHUNK_BYTES)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        largest_chunk_bytes = largest_chunk_bytes.max(bytes);
        let key = (block.dataset_path.clone(), block.logical_chunk);
        match source_blocks.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(block);
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if !same_component_agnostic_verified_block(entry.get(), block) {
                    return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
                }
            }
        }
    }

    for window in &windows {
        let rows = usize::try_from(window.expected_shape[0])
            .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let cols = usize::try_from(window.expected_shape[1])
            .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let elements = rows
            .checked_mul(cols)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let science_bytes = elements
            .checked_mul(std::mem::size_of::<CFloat32>())
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let mask_bytes = if window.validity_mask_required {
            elements
        } else {
            0
        };
        let coordinate_bytes = rows
            .checked_add(cols)
            .and_then(|count| count.checked_mul(std::mem::size_of::<f64>()))
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let window_bytes = science_bytes
            .checked_add(mask_bytes)
            .and_then(|bytes| bytes.checked_add(coordinate_bytes))
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        // The selected science and mask are represented both at their
        // standard RSLC paths and under the explicit derived-subset namespace.
        // Bound the logical artifact size independently from the streaming
        // working set so large but legitimate AOIs never require whole-window
        // materialization.
        output_logical_bytes = output_logical_bytes
            .checked_add(
                window_bytes
                    .checked_mul(2)
                    .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?,
            )
            .filter(|bytes| *bytes <= MAX_OUTPUT_LOGICAL_BYTES)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let science_row_bytes = cols
            .checked_mul(std::mem::size_of::<CFloat32>())
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        if science_row_bytes == 0 || science_row_bytes > MAX_STREAM_WINDOW_BYTES {
            return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
        }
        largest_stream_bytes = largest_stream_bytes.max(science_bytes.min(MAX_STREAM_WINDOW_BYTES));
        if window.validity_mask_required {
            largest_stream_bytes =
                largest_stream_bytes.max(mask_bytes.min(MAX_STREAM_WINDOW_BYTES));
        }
    }
    raw_block_bytes
        .checked_add(largest_stream_bytes.saturating_mul(2))
        .and_then(|bytes| bytes.checked_add(largest_chunk_bytes.saturating_mul(2)))
        .and_then(|bytes| bytes.checked_add(metadata_bytes))
        .filter(|bytes| *bytes <= MAX_WRITER_WORKING_SET_BYTES)
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;

    let mut materials = Vec::with_capacity(windows.len());
    for (ordinal, window) in windows.into_iter().enumerate() {
        let frequency = frequency_label(&window.frequency);
        let root_path = format!(
            "{DERIVED_ROOT}/component{:03}/window{ordinal:03}/swaths/frequency{frequency}",
            window.component_id
        );
        let science_path = format!("{root_path}/{}", window.polarization);
        let (mask_path, mask_chunk_shape) = if window.validity_mask_required {
            let source_mask_path = source_mask_path(&window.frequency);
            let mask_chunk_shape = block_map
                .values()
                .find(|block| {
                    block.component_id == window.component_id
                        && block.dataset_path == source_mask_path
                })
                .map(|block| block.chunk_shape)
                .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
            (
                Some(format!("{root_path}/inputDataExceptionMask")),
                Some(mask_chunk_shape),
            )
        } else {
            (None, None)
        };
        let line_coordinates = coordinate_window(
            &metadata.evidence.zero_doppler_time,
            window.line_start,
            window.line_end_exclusive,
        )?;
        let sample_coordinates = coordinate_window(
            metadata
                .evidence
                .slant_ranges
                .get(frequency)
                .ok_or(NisarSubsetWriteError::MetadataInvalid)?,
            window.sample_start,
            window.sample_end_exclusive,
        )?;
        materials.push(DerivedWindowMaterial {
            window,
            root_path,
            science_path,
            mask_path,
            mask_chunk_shape,
            line_coordinates,
            sample_coordinates,
        });
    }
    Ok(materials)
}

fn metadata_working_set_bytes(
    metadata: &NisarSubsetMetadataSnapshotV1,
) -> Result<usize, NisarSubsetWriteError> {
    let evidence = &metadata.evidence;
    let f64_values = evidence
        .orbit_time
        .len()
        .checked_add(evidence.orbit_position.len().saturating_mul(3))
        .and_then(|count| count.checked_add(evidence.orbit_velocity.len().saturating_mul(3)))
        .and_then(|count| count.checked_add(evidence.attitude_time.len()))
        .and_then(|count| count.checked_add(evidence.attitude_quaternions.len().saturating_mul(4)))
        .and_then(|count| count.checked_add(evidence.geolocation_x.len()))
        .and_then(|count| count.checked_add(evidence.geolocation_y.len()))
        .and_then(|count| count.checked_add(evidence.height_above_ellipsoid.len()))
        .and_then(|count| count.checked_add(evidence.zero_doppler_time.len()))
        .and_then(|count| {
            evidence
                .slant_ranges
                .values()
                .try_fold(count, |total, values| total.checked_add(values.len()))
        })
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    let f32_values = evidence
        .doppler_lut
        .len()
        .checked_add(evidence.calibration_lut.len())
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    let valid_sample_values = evidence
        .valid_samples
        .values()
        .try_fold(0_usize, |total, values| {
            total.checked_add(values.len().saturating_mul(2))
        })
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    f64_values
        .checked_mul(std::mem::size_of::<f64>())
        .and_then(|bytes| {
            f32_values
                .checked_mul(std::mem::size_of::<f32>())
                .and_then(|other| bytes.checked_add(other))
        })
        .and_then(|bytes| {
            valid_sample_values
                .checked_mul(std::mem::size_of::<u32>())
                .and_then(|other| bytes.checked_add(other))
        })
        .filter(|bytes| *bytes <= MAX_METADATA_BYTES)
        .ok_or(NisarSubsetWriteError::MetadataInvalid)
}

fn same_component_agnostic_verified_block(
    left: &VerifiedHdf5ChunkBlockV1,
    right: &VerifiedHdf5ChunkBlockV1,
) -> bool {
    left.dataset_path == right.dataset_path
        && left.logical_chunk == right.logical_chunk
        && left.dataset_shape == right.dataset_shape
        && left.chunk_shape == right.chunk_shape
        && left.datatype == right.datatype
        && left.datatype_sha256 == right.datatype_sha256
        && left.fill_value_definition == right.fill_value_definition
        && left.fill_value_hex == right.fill_value_hex
        && left.filter_pipeline == right.filter_pipeline
        && left.allocation == right.allocation
}

fn coordinate_window(
    source: &[f64],
    start: u64,
    end_exclusive: u64,
) -> Result<Vec<f64>, NisarSubsetWriteError> {
    let start = usize::try_from(start).map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let end = usize::try_from(end_exclusive).map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    source
        .get(start..end)
        .map(<[f64]>::to_vec)
        .ok_or(NisarSubsetWriteError::MetadataInvalid)
}

fn stream_band_rows(
    output_columns: usize,
    element_size: usize,
) -> Result<usize, NisarSubsetWriteError> {
    let row_bytes = output_columns
        .checked_mul(element_size)
        .filter(|bytes| *bytes > 0 && *bytes <= MAX_STREAM_WINDOW_BYTES)
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    Ok((MAX_STREAM_WINDOW_BYTES / row_bytes).max(1))
}

fn reconstruct_science_band(
    material: &DerivedWindowMaterial,
    blocks: &[VerifiedHdf5ChunkBlockV1],
    line_start: u64,
    line_end_exclusive: u64,
) -> Result<Vec<CFloat32>, NisarSubsetWriteError> {
    let mut window = material.window.clone();
    window.line_start = line_start;
    window.line_end_exclusive = line_end_exclusive;
    window.expected_shape[0] = line_end_exclusive
        .checked_sub(line_start)
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let rows = usize::try_from(window.expected_shape[0])
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let cols = usize::try_from(window.expected_shape[1])
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let length = rows
        .checked_mul(cols)
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let mut result = vec![CFloat32::default(); length];
    let mut written = 0_usize;
    for block in blocks.iter().filter(|block| {
        block.component_id == window.component_id
            && block.dataset_path == window.dataset_path
            && block.logical_chunk[0].saturating_mul(block.chunk_shape[0])
                < window.line_end_exclusive
            && block.logical_chunk[0]
                .saturating_add(1)
                .saturating_mul(block.chunk_shape[0])
                > window.line_start
    }) {
        if block.dataset_shape != window.source_shape || block.chunk_shape != window.chunk_shape {
            return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
        }
        let values = decode_science_chunk(block)?;
        written = written
            .checked_add(copy_chunk_overlap(
                &window,
                block,
                &values,
                &mut result,
                rows,
                cols,
            )?)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    }
    if written != length {
        return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
    }
    Ok(result)
}

fn reconstruct_mask_band(
    material: &DerivedWindowMaterial,
    dataset_path: &str,
    blocks: &[VerifiedHdf5ChunkBlockV1],
    line_start: u64,
    line_end_exclusive: u64,
) -> Result<Vec<u8>, NisarSubsetWriteError> {
    let mut window = material.window.clone();
    window.line_start = line_start;
    window.line_end_exclusive = line_end_exclusive;
    window.expected_shape[0] = line_end_exclusive
        .checked_sub(line_start)
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let rows = usize::try_from(window.expected_shape[0])
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let cols = usize::try_from(window.expected_shape[1])
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let length = rows
        .checked_mul(cols)
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let mut result = vec![0_u8; length];
    let layout = blocks
        .iter()
        .find(|block| {
            block.component_id == window.component_id && block.dataset_path == dataset_path
        })
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let mut written = 0_usize;
    for block in blocks.iter().filter(|block| {
        block.component_id == window.component_id
            && block.dataset_path == dataset_path
            && block.logical_chunk[0].saturating_mul(block.chunk_shape[0])
                < window.line_end_exclusive
            && block.logical_chunk[0]
                .saturating_add(1)
                .saturating_mul(block.chunk_shape[0])
                > window.line_start
    }) {
        if block.dataset_shape != layout.dataset_shape
            || block.chunk_shape != layout.chunk_shape
            || window.line_end_exclusive > block.dataset_shape[0]
            || window.sample_end_exclusive > block.dataset_shape[1]
        {
            return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
        }
        let values = decode_mask_chunk(block)?;
        written = written
            .checked_add(copy_chunk_overlap(
                &window,
                block,
                &values,
                &mut result,
                rows,
                cols,
            )?)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    }
    if written != length {
        return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
    }
    Ok(result)
}

fn copy_chunk_overlap<T: Copy>(
    window: &NisarSubsetLogicalWindowV1,
    block: &VerifiedHdf5ChunkBlockV1,
    source: &[T],
    destination: &mut [T],
    output_rows: usize,
    output_cols: usize,
) -> Result<usize, NisarSubsetWriteError> {
    let chunk_line_start = block.logical_chunk[0]
        .checked_mul(block.chunk_shape[0])
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let chunk_sample_start = block.logical_chunk[1]
        .checked_mul(block.chunk_shape[1])
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let line_start = window.line_start.max(chunk_line_start);
    let line_end = window
        .line_end_exclusive
        .min(chunk_line_start.saturating_add(block.chunk_shape[0]))
        .min(block.dataset_shape[0]);
    let sample_start = window.sample_start.max(chunk_sample_start);
    let sample_end = window
        .sample_end_exclusive
        .min(chunk_sample_start.saturating_add(block.chunk_shape[1]))
        .min(block.dataset_shape[1]);
    if line_start >= line_end || sample_start >= sample_end {
        return Ok(0);
    }
    let chunk_cols = usize::try_from(
        block.dataset_shape[1]
            .saturating_sub(chunk_sample_start)
            .min(block.chunk_shape[1]),
    )
    .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let copy_cols = usize::try_from(sample_end - sample_start)
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let mut copied = 0_usize;
    for line in line_start..line_end {
        let source_row = usize::try_from(line - chunk_line_start)
            .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let source_col = usize::try_from(sample_start - chunk_sample_start)
            .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let output_row = usize::try_from(line - window.line_start)
            .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let output_col = usize::try_from(sample_start - window.sample_start)
            .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
        if output_row >= output_rows || output_col + copy_cols > output_cols {
            return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
        }
        let source_offset = source_row
            .checked_mul(chunk_cols)
            .and_then(|offset| offset.checked_add(source_col))
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let output_offset = output_row
            .checked_mul(output_cols)
            .and_then(|offset| offset.checked_add(output_col))
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let source_slice = source
            .get(source_offset..source_offset + copy_cols)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let output_slice = destination
            .get_mut(output_offset..output_offset + copy_cols)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        output_slice.copy_from_slice(source_slice);
        copied = copied
            .checked_add(copy_cols)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    }
    Ok(copied)
}

fn decode_science_chunk(
    block: &VerifiedHdf5ChunkBlockV1,
) -> Result<Vec<CFloat32>, NisarSubsetWriteError> {
    if !block_has_supported_datatype(block, Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi) {
        return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
    }
    if block.filter_pipeline != Hdf5ChunkFilterPipelineV1::None {
        // HDF5-native decode: the bundled HDF5 library applies the reviewed
        // built-in pipeline (Shuffle -> Deflate) while reading the logical
        // chunk region from the materialized sparse part. The transfer plan
        // already proved every required physical chunk byte is present.
        let bytes = read_hdf5_chunk_u8(block)?;
        if bytes.len() % 8 != 0 {
            return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
        }
        return Ok(bytes
            .chunks_exact(8)
            .map(|value| CFloat32 {
                r: f32::from_le_bytes(value[..4].try_into().expect("four-byte real")),
                i: f32::from_le_bytes(value[4..].try_into().expect("four-byte imaginary")),
            })
            .collect());
    }
    let elements = chunk_elements(block)?;
    match &block.allocation {
        VerifiedHdf5ChunkAllocationV1::Allocated { bytes } => {
            if bytes.len() != elements * 8 {
                return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
            }
            Ok(bytes
                .chunks_exact(8)
                .map(|value| CFloat32 {
                    r: f32::from_le_bytes(value[..4].try_into().expect("four-byte real")),
                    i: f32::from_le_bytes(value[4..].try_into().expect("four-byte imaginary")),
                })
                .collect())
        }
        VerifiedHdf5ChunkAllocationV1::Fill { element_fill_hex } => {
            if block.fill_value_hex.as_deref() != Some(element_fill_hex) {
                return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
            }
            let fill = decode_hex(element_fill_hex)?;
            if fill.len() != 8 {
                return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
            }
            let value = CFloat32 {
                r: f32::from_le_bytes(fill[..4].try_into().expect("four-byte fill real")),
                i: f32::from_le_bytes(fill[4..].try_into().expect("four-byte fill imaginary")),
            };
            Ok(vec![value; elements])
        }
    }
}

fn decode_mask_chunk(block: &VerifiedHdf5ChunkBlockV1) -> Result<Vec<u8>, NisarSubsetWriteError> {
    if !block_has_supported_datatype(block, Hdf5ChunkDatatypeV1::UnsignedU8) {
        return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
    }
    if block.filter_pipeline != Hdf5ChunkFilterPipelineV1::None {
        return read_hdf5_chunk_u8(block);
    }
    let elements = chunk_elements(block)?;
    match &block.allocation {
        VerifiedHdf5ChunkAllocationV1::Allocated { bytes } if bytes.len() == elements => {
            Ok(bytes.to_vec())
        }
        VerifiedHdf5ChunkAllocationV1::Fill { element_fill_hex } => {
            if block.fill_value_hex.as_deref() != Some(element_fill_hex) {
                return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
            }
            let fill = decode_hex(element_fill_hex)?;
            if fill.len() != 1 {
                return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
            }
            Ok(vec![fill[0]; elements])
        }
        _ => Err(NisarSubsetWriteError::BlockEvidenceInvalid),
    }
}

/// HDF5-native read of one planned chunk logical region from the
/// materialized sparse part. The bundled HDF5 library applies the reviewed
/// built-in filter pipeline (Shuffle -> Deflate) during `H5Dread`; this
/// function never performs network I/O and requires the transfer plan to
/// have proven the chunk physical bytes are fully materialized.
fn read_hdf5_chunk_u8(block: &VerifiedHdf5ChunkBlockV1) -> Result<Vec<u8>, NisarSubsetWriteError> {
    if !matches!(
        block.filter_pipeline,
        Hdf5ChunkFilterPipelineV1::ShuffleDeflate { level: 4 }
    ) {
        return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
    }
    let file = crate::hdf5_file::open(&block.part_path)
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let dataset = file
        .dataset(&block.dataset_path)
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let row_start_u = block.logical_chunk[0]
        .checked_mul(block.chunk_shape[0])
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let col_start_u = block.logical_chunk[1]
        .checked_mul(block.chunk_shape[1])
        .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
    // A final dataset-edge chunk has a logical extent smaller than the
    // nominal chunk shape. Never issue an HDF5 hyperslab beyond dataset
    // bounds: clamp every dimension to `dataset_shape - start` and require a
    // nonzero extent.
    if row_start_u >= block.dataset_shape[0] || col_start_u >= block.dataset_shape[1] {
        return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
    }
    let rows_u = (block.dataset_shape[0] - row_start_u).min(block.chunk_shape[0]);
    let cols_u = (block.dataset_shape[1] - col_start_u).min(block.chunk_shape[1]);
    if rows_u == 0 || cols_u == 0 {
        return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
    }
    let row_start =
        usize::try_from(row_start_u).map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let col_start =
        usize::try_from(col_start_u).map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let rows = usize::try_from(rows_u).map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let cols = usize::try_from(cols_u).map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    // HDF5-native decode: the bundled HDF5 library applies the reviewed
    // built-in Shuffle -> Deflate pipeline while reading the chunk region.
    match block.datatype {
        Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi => {
            let values: ndarray::Array2<CFloat32> = dataset
                .read_slice_2d((row_start..row_start + rows, col_start..col_start + cols))
                .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
            let mut bytes = Vec::with_capacity(rows * cols * 8);
            for value in values.iter() {
                bytes.extend_from_slice(&value.r.to_le_bytes());
                bytes.extend_from_slice(&value.i.to_le_bytes());
            }
            Ok(bytes)
        }
        Hdf5ChunkDatatypeV1::UnsignedU8 => {
            let values: ndarray::Array2<u8> = dataset
                .read_slice_2d((row_start..row_start + rows, col_start..col_start + cols))
                .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
            Ok(values.iter().copied().collect())
        }
    }
}

fn chunk_elements(block: &VerifiedHdf5ChunkBlockV1) -> Result<usize, NisarSubsetWriteError> {
    usize::try_from(
        block.chunk_shape[0]
            .checked_mul(block.chunk_shape[1])
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?,
    )
    .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)
}

fn block_has_supported_datatype(
    block: &VerifiedHdf5ChunkBlockV1,
    expected: Hdf5ChunkDatatypeV1,
) -> bool {
    block.datatype == expected
        && supported_datatype_sha256(expected).ok().as_deref()
            == Some(block.datatype_sha256.as_str())
}

fn decode_hex(value: &str) -> Result<Vec<u8>, NisarSubsetWriteError> {
    if !value.len().is_multiple_of(2) {
        return Err(NisarSubsetWriteError::BlockEvidenceInvalid);
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)
        })
        .collect()
}

fn frequency_label(frequency: &NisarSubsetFrequency) -> &'static str {
    match frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    }
}

fn source_mask_path(frequency: &NisarSubsetFrequency) -> String {
    format!(
        "/science/LSAR/RSLC/swaths/frequency{}/inputDataExceptionMask",
        frequency_label(frequency)
    )
}

fn write_candidate(
    path: &Path,
    request: &NisarSubsetRequestManifestV1,
    transfer: &Hdf5RangeTransferResultV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    materials: &[DerivedWindowMaterial],
    blocks: &[VerifiedHdf5ChunkBlockV1],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), NisarSubsetWriteError> {
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    // Disable HDF5 object track times at the file level so groups and
    // raw-copied datasets do not embed wall-clock metadata: the derived
    // artifact must be byte-for-byte reproducible.
    let native = crate::long_path::native_path(path);
    let file = crate::hdf5_file::builder(&native)
        .with_fcpl(|plist| plist.obj_track_times(false))
        .create_excl(&native)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    create_source_groups(&file, &metadata.group_bindings, cancelled)?;
    for binding in &metadata.dataset_bindings {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        write_source_standard_dataset(
            &file, request, metadata, materials, blocks, binding, cancelled,
        )?;
    }
    restore_dimension_scale_graph(&file, &metadata.dataset_bindings, cancelled)?;

    let identification = file
        .group(IDENTIFICATION_PATH)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    write_ascii_scalar::<256>(
        &identification,
        "sourceGranuleId",
        &metadata.evidence.source_granule_id,
    )?;
    write_ascii_scalar::<512>(&identification, "sourceFileName", &request.source.file_name)?;
    write_ascii_scalar::<256>(
        &identification,
        "sourceProviderProductId",
        &request.source.provider_product_id,
    )?;
    write_ascii_scalar::<32>(
        &identification,
        "outputAuthority",
        NISAR_SUBSET_OUTPUT_AUTHORITY,
    )?;
    write_ascii_scalar::<32>(
        &identification,
        "derivedProductType",
        OfflineNisarSubsetWriter::DERIVED_PRODUCT_TYPE,
    )?;

    let provenance = file
        .create_group(PROVENANCE_PATH)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    write_ascii_scalar::<96>(
        &provenance,
        "requestManifestSha256",
        &request
            .sha256()
            .map_err(|_| NisarSubsetWriteError::RequestInvalid)?,
    )?;
    write_ascii_scalar::<96>(
        &provenance,
        "transferManifestSha256",
        &transfer
            .manifest()
            .sha256()
            .map_err(|_| NisarSubsetWriteError::TransferIncomplete)?,
    )?;
    write_ascii_scalar::<96>(
        &provenance,
        "metadataEvidenceSha256",
        &metadata.evidence_sha256,
    )?;
    write_ascii_scalar::<256>(
        &provenance,
        "sourceProductId",
        &request.source.provider_product_id,
    )?;
    write_ascii_scalar::<32>(
        &provenance,
        "outputAuthority",
        NISAR_SUBSET_OUTPUT_AUTHORITY,
    )?;
    write_ascii_scalar::<64>(&provenance, "writerId", OfflineNisarSubsetWriter::WRITER_ID)?;
    write_ascii_scalar::<96>(
        &provenance,
        "sourceInventorySha256",
        &metadata.source_inventory_sha256,
    )?;
    write_ascii_scalar::<32>(&provenance, "statisticsScope", "derived_subset")?;

    for material in materials {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let group = file
            .create_group(&material.root_path)
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        let rows = usize::try_from(material.window.expected_shape[0])
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        let cols = usize::try_from(material.window.expected_shape[1])
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        let science_chunk_rows = usize::try_from(material.window.chunk_shape[0])
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        let science_chunk_cols = usize::try_from(material.window.chunk_shape[1])
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        let science_name = material
            .science_path
            .rsplit_once('/')
            .map(|(_, name)| name)
            .ok_or(NisarSubsetWriteError::Persistence)?;
        let science = group
            .new_dataset::<CFloat32>()
            .shape((rows, cols))
            .chunk((rows.min(science_chunk_rows), cols.min(science_chunk_cols)))
            .create(science_name)
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        write_science_window_streaming(&science, material, blocks, cancelled)?;
        if let Some(mask_path) = &material.mask_path {
            let mask_chunk_shape = material
                .mask_chunk_shape
                .ok_or(NisarSubsetWriteError::Persistence)?;
            let mask_chunk_rows = usize::try_from(mask_chunk_shape[0])
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
            let mask_chunk_cols = usize::try_from(mask_chunk_shape[1])
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
            let mask_name = mask_path
                .rsplit_once('/')
                .map(|(_, name)| name)
                .ok_or(NisarSubsetWriteError::Persistence)?;
            let mask = group
                .new_dataset::<u8>()
                .shape((rows, cols))
                .chunk((rows.min(mask_chunk_rows), cols.min(mask_chunk_cols)))
                .create(mask_name)
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
            write_mask_window_streaming(&mask, material, blocks, cancelled)?;
        }
        write_vector(&group, "zeroDopplerTime", &material.line_coordinates)?;
        write_vector(&group, "slantRange", &material.sample_coordinates)?;
        write_ascii_scalar::<512>(&group, "sourceDatasetPath", &material.window.dataset_path)?;
        write_scalar(&group, "lineStart", &material.window.line_start)?;
        write_scalar(
            &group,
            "lineEndExclusive",
            &material.window.line_end_exclusive,
        )?;
        write_scalar(&group, "sampleStart", &material.window.sample_start)?;
        write_scalar(
            &group,
            "sampleEndExclusive",
            &material.window.sample_end_exclusive,
        )?;
        write_scalar(&group, "componentId", &material.window.component_id)?;
    }
    file.flush()
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    drop(file);
    Ok(())
}

fn write_science_window_streaming(
    dataset: &Dataset,
    material: &DerivedWindowMaterial,
    blocks: &[VerifiedHdf5ChunkBlockV1],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), NisarSubsetWriteError> {
    let rows = usize::try_from(material.window.expected_shape[0])
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let cols = usize::try_from(material.window.expected_shape[1])
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let band_rows = stream_band_rows(cols, std::mem::size_of::<CFloat32>())?;
    for output_start in (0..rows).step_by(band_rows) {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let output_end = output_start.saturating_add(band_rows).min(rows);
        let line_start = material
            .window
            .line_start
            .checked_add(output_start as u64)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let line_end = material
            .window
            .line_start
            .checked_add(output_end as u64)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let values = reconstruct_science_band(material, blocks, line_start, line_end)?;
        let view = ndarray::ArrayView2::from_shape((output_end - output_start, cols), &values)
            .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
        dataset
            .write_slice(view, (output_start..output_end, 0..cols))
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
    }
    Ok(())
}

fn write_mask_window_streaming(
    dataset: &Dataset,
    material: &DerivedWindowMaterial,
    blocks: &[VerifiedHdf5ChunkBlockV1],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), NisarSubsetWriteError> {
    let rows = usize::try_from(material.window.expected_shape[0])
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let cols = usize::try_from(material.window.expected_shape[1])
        .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
    let band_rows = stream_band_rows(cols, std::mem::size_of::<u8>())?;
    let source_path = source_mask_path(&material.window.frequency);
    for output_start in (0..rows).step_by(band_rows) {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let output_end = output_start.saturating_add(band_rows).min(rows);
        let line_start = material
            .window
            .line_start
            .checked_add(output_start as u64)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let line_end = material
            .window
            .line_start
            .checked_add(output_end as u64)
            .ok_or(NisarSubsetWriteError::BlockEvidenceInvalid)?;
        let values = reconstruct_mask_band(material, &source_path, blocks, line_start, line_end)?;
        let view = ndarray::ArrayView2::from_shape((output_end - output_start, cols), &values)
            .map_err(|_| NisarSubsetWriteError::BlockEvidenceInvalid)?;
        dataset
            .write_slice(view, (output_start..output_end, 0..cols))
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
    }
    Ok(())
}

fn create_source_groups(
    destination: &File,
    bindings: &[SourceGroupBindingV1],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), NisarSubsetWriteError> {
    for binding in bindings {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let destination_group = if binding.path == "/" {
            destination
                .group("/")
                .map_err(|_| NisarSubsetWriteError::Persistence)?
        } else {
            destination
                .create_group(&binding.path)
                .map_err(|_| NisarSubsetWriteError::Persistence)?
        };
        copy_attributes(&destination_group, &binding.attributes)?;
    }
    Ok(())
}

/// Exact HDF5 datatype representation. `TypeDescriptor` loses properties
/// that are part of the encoded HDF5 type (fixed-string padding, compound
/// member order, array details), so the writer binds the exact
/// H5Tencode bytes from the source and decodes them when writing. This keeps
/// the datatype identity hash meaningful and validation closed against
/// encoding drift.
fn encode_datatype(datatype: &hdf5::Datatype) -> Result<Vec<u8>, NisarSubsetWriteError> {
    const MAX_ENCODED_DATATYPE_BYTES: usize = 64 * 1024;
    let mut encoded_len = 0_usize;
    let size_status = hdf5::sync::sync(|| unsafe {
        H5Tencode(datatype.id(), std::ptr::null_mut(), &mut encoded_len)
    });
    if size_status < 0 || encoded_len == 0 || encoded_len > MAX_ENCODED_DATATYPE_BYTES {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let mut encoded = vec![0_u8; encoded_len];
    let mut written_len = encoded_len;
    let status = hdf5::sync::sync(|| unsafe {
        H5Tencode(datatype.id(), encoded.as_mut_ptr().cast(), &mut written_len)
    });
    if status < 0 || written_len != encoded_len {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok(encoded)
}

/// Decode a captured H5Tencode byte string into an owned raw datatype id.
/// The caller must close the returned id with `H5Tclose`.
fn decode_datatype_hid(encoded: &[u8]) -> Result<i64, NisarSubsetWriteError> {
    if encoded.is_empty() || encoded.len() > 64 * 1024 {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let id = hdf5::sync::sync(|| unsafe { H5Tdecode2(encoded.as_ptr().cast(), encoded.len()) });
    if id < 0 {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok(id)
}

fn copy_attributes(
    destination: &Location,
    expected: &[SourceAttributeBindingV1],
) -> Result<(), NisarSubsetWriteError> {
    for binding in expected {
        let shape = binding
            .shape
            .iter()
            .map(|value| {
                usize::try_from(*value).map_err(|_| NisarSubsetWriteError::MetadataInvalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let datatype_id = decode_datatype_hid(&binding.datatype_encoded)?;
        let datatype_size = hdf5::sync::sync(|| unsafe { hdf5_sys::h5t::H5Tget_size(datatype_id) });
        if sha256_bytes(&binding.datatype_encoded) != binding.datatype_sha256
            || binding.value.len()
                != shape
                    .iter()
                    .try_fold(1_usize, |total, value| total.checked_mul(*value))
                    .unwrap_or(0)
                    .max(1)
                    .checked_mul(datatype_size as usize)
                    .ok_or(NisarSubsetWriteError::MetadataInvalid)?
            || sha256_bytes(&binding.value) != binding.value_sha256
        {
            hdf5::sync::sync(|| unsafe { H5Tclose(datatype_id) });
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        let created = create_attribute_raw_id(destination, binding, datatype_id, &shape);
        hdf5::sync::sync(|| unsafe { H5Tclose(datatype_id) });
        created?;
    }
    if read_attribute_bindings(destination)? != expected {
        return Err(NisarSubsetWriteError::Persistence);
    }
    Ok(())
}

fn write_source_standard_dataset(
    destination: &File,
    request: &NisarSubsetRequestManifestV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    materials: &[DerivedWindowMaterial],
    blocks: &[VerifiedHdf5ChunkBlockV1],
    binding: &SourceMetadataDatasetBindingV1,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), NisarSubsetWriteError> {
    let (parent_path, name) = dataset_parent_and_name(&binding.path)?;
    let parent = destination
        .group(parent_path)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    let dataset = match binding.action {
        SourceDatasetActionV1::PreserveExact => create_raw_dataset_like(
            &parent,
            name,
            binding,
            binding
                .value
                .as_deref()
                .ok_or(NisarSubsetWriteError::MetadataInvalid)?,
        )?,
        SourceDatasetActionV1::SelectedScience => {
            let material = unique_science_material(materials, &binding.path)?;
            let dataset = create_typed_dataset_like_empty::<CFloat32>(
                &parent,
                name,
                binding,
                &[
                    usize::try_from(material.window.expected_shape[0])
                        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
                    usize::try_from(material.window.expected_shape[1])
                        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
                ],
            )?;
            write_science_window_streaming(&dataset, material, blocks, cancelled)?;
            dataset
        }
        SourceDatasetActionV1::FrequencyMask => {
            let material = unique_mask_material(materials, &binding.path)?;
            let dataset = create_typed_dataset_like_empty::<u8>(
                &parent,
                name,
                binding,
                &[
                    usize::try_from(material.window.expected_shape[0])
                        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
                    usize::try_from(material.window.expected_shape[1])
                        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
                ],
            )?;
            write_mask_window_streaming(&dataset, material, blocks, cancelled)?;
            dataset
        }
        SourceDatasetActionV1::ZeroDopplerTime => {
            let material = unique_standard_window(materials)?;
            create_typed_dataset_like(
                &parent,
                name,
                binding,
                &[material.line_coordinates.len()],
                &material.line_coordinates,
            )?
        }
        SourceDatasetActionV1::FrequencySlantRange => {
            let material = unique_frequency_material(materials, &binding.path)?;
            create_typed_dataset_like(
                &parent,
                name,
                binding,
                &[material.sample_coordinates.len()],
                &material.sample_coordinates,
            )?
        }
        SourceDatasetActionV1::FrequencyValidSamples => {
            let material = unique_frequency_material(materials, &binding.path)?;
            let frequency = frequency_from_standard_path(&binding.path)?;
            let values = derive_valid_samples(metadata, material, frequency)?;
            create_typed_dataset_like(&parent, name, binding, &[values.len() / 2, 2], &values)?
        }
        SourceDatasetActionV1::DerivedIdentification => {
            write_derived_identification(&parent, name, binding, request, metadata, materials)?
        }
    };
    copy_attributes(&dataset, &binding.attributes)?;
    Ok(())
}

fn dataset_parent_and_name(path: &str) -> Result<(&str, &str), NisarSubsetWriteError> {
    let (parent, name) = path
        .rsplit_once('/')
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    if parent.is_empty() || name.is_empty() {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok((parent, name))
}

fn create_typed_dataset_like<T: H5Type>(
    parent: &Group,
    name: &str,
    binding: &SourceMetadataDatasetBindingV1,
    shape: &[usize],
    values: &[T],
) -> Result<Dataset, NisarSubsetWriteError> {
    let expected_elements = shape
        .iter()
        .try_fold(1_usize, |total, value| total.checked_mul(*value))
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    if expected_elements != values.len() {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let dataset = create_typed_dataset_like_empty::<T>(parent, name, binding, shape)?;
    dataset
        .write_raw(values)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    Ok(dataset)
}

fn create_typed_dataset_like_empty<T: H5Type>(
    parent: &Group,
    name: &str,
    binding: &SourceMetadataDatasetBindingV1,
    shape: &[usize],
) -> Result<Dataset, NisarSubsetWriteError> {
    let filters = binding
        .filters
        .iter()
        .map(SourceFilterBindingV1::to_filter)
        .collect::<Vec<_>>();
    let mut builder = parent.new_dataset::<T>().shape(shape);
    if let Some(source_chunk) = &binding.chunk_shape {
        if source_chunk.len() != shape.len() {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        let chunk = source_chunk
            .iter()
            .zip(shape)
            .map(|(source, target)| {
                usize::try_from(*source)
                    .ok()
                    .map(|source| source.min(*target))
                    .filter(|value| *value > 0)
                    .ok_or(NisarSubsetWriteError::MetadataInvalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        builder = builder.chunk(chunk);
    } else if !filters.is_empty() {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let dataset = builder
        .set_filters(&filters)
        .create(name)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    Ok(dataset)
}

fn create_raw_dataset_like(
    parent: &Group,
    name: &str,
    binding: &SourceMetadataDatasetBindingV1,
    values: &[u8],
) -> Result<Dataset, NisarSubsetWriteError> {
    if sha256_bytes(&binding.datatype_encoded) != binding.datatype_sha256 {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let datatype_id = decode_datatype_hid(&binding.datatype_encoded)?;
    let datatype_size =
        hdf5::sync::sync(|| unsafe { hdf5_sys::h5t::H5Tget_size(datatype_id) }) as usize;
    let elements = binding
        .shape
        .iter()
        .try_fold(1_usize, |total, value| {
            usize::try_from(*value)
                .ok()
                .and_then(|value| total.checked_mul(value))
        })
        .unwrap_or(0)
        .max(1);
    if elements.checked_mul(datatype_size) != Some(values.len()) {
        hdf5::sync::sync(|| unsafe { H5Tclose(datatype_id) });
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let shape = binding
        .shape
        .iter()
        .map(|value| usize::try_from(*value).map_err(|_| NisarSubsetWriteError::MetadataInvalid))
        .collect::<Result<Vec<_>, _>>()?;
    let filters = binding
        .filters
        .iter()
        .map(SourceFilterBindingV1::to_filter)
        .collect::<Vec<_>>();
    let dataset = create_dataset_raw(parent, name, binding, datatype_id, &shape, &filters, values);
    hdf5::sync::sync(|| unsafe { H5Tclose(datatype_id) });
    dataset
}

fn create_attribute_raw_id(
    destination: &Location,
    binding: &SourceAttributeBindingV1,
    datatype_id: i64,
    shape: &[usize],
) -> Result<(), NisarSubsetWriteError> {
    let space = if shape.is_empty() {
        hdf5::sync::sync(|| unsafe { H5Screate(H5S_SCALAR) })
    } else {
        let dims = shape
            .iter()
            .map(|value| u64::try_from(*value).map_err(|_| NisarSubsetWriteError::MetadataInvalid))
            .collect::<Result<Vec<_>, _>>()?;
        hdf5::sync::sync(|| unsafe {
            H5Screate_simple(dims.len() as c_int, dims.as_ptr(), std::ptr::null())
        })
    };
    if space < 0 {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let name =
        CString::new(binding.name.as_str()).map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let attribute = hdf5::sync::sync(|| unsafe {
        H5Acreate2(
            destination.id(),
            name.as_ptr(),
            datatype_id,
            space,
            H5P_DEFAULT,
            H5P_DEFAULT,
        )
    });
    if attribute < 0 {
        hdf5::sync::sync(|| unsafe { H5Sclose(space) });
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let wrote = hdf5::sync::sync(|| unsafe {
        H5Awrite(attribute, datatype_id, binding.value.as_ptr().cast())
    });
    let status = hdf5::sync::sync(|| unsafe { H5Aclose(attribute) });
    hdf5::sync::sync(|| unsafe { H5Sclose(space) });
    if wrote < 0 || status < 0 {
        return Err(NisarSubsetWriteError::Persistence);
    }
    Ok(())
}

fn create_dataset_raw(
    parent: &Group,
    name: &str,
    binding: &SourceMetadataDatasetBindingV1,
    datatype_id: i64,
    shape: &[usize],
    filters: &[Filter],
    values: &[u8],
) -> Result<Dataset, NisarSubsetWriteError> {
    let space = if shape.is_empty() {
        hdf5::sync::sync(|| unsafe { H5Screate(H5S_SCALAR) })
    } else {
        let dims = shape
            .iter()
            .map(|value| u64::try_from(*value).map_err(|_| NisarSubsetWriteError::MetadataInvalid))
            .collect::<Result<Vec<_>, _>>()?;
        hdf5::sync::sync(|| unsafe {
            H5Screate_simple(dims.len() as c_int, dims.as_ptr(), std::ptr::null())
        })
    };
    if space < 0 {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let dcpl = hdf5::sync::sync(|| unsafe { H5Pcreate(*H5P_DATASET_CREATE) });
    if dcpl < 0 {
        hdf5::sync::sync(|| unsafe { H5Sclose(space) });
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    // Match the high-level writer's deterministic layout: disable object
    // track times so the derived artifact is byte-for-byte reproducible and
    // does not embed wall-clock metadata into raw-copied datasets.
    hdf5::sync::sync(|| unsafe { H5Pset_obj_track_times(dcpl, 0) });
    let chunk_dims;
    if let Some(chunk) = &binding.chunk_shape {
        if chunk.len() != shape.len() || shape.is_empty() {
            hdf5::sync::sync(|| unsafe { H5Pclose(dcpl) });
            hdf5::sync::sync(|| unsafe { H5Sclose(space) });
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        chunk_dims = chunk.clone();
        if hdf5::sync::sync(|| unsafe {
            H5Pset_chunk(dcpl, chunk_dims.len() as c_int, chunk_dims.as_ptr())
        }) < 0
        {
            hdf5::sync::sync(|| unsafe { H5Pclose(dcpl) });
            hdf5::sync::sync(|| unsafe { H5Sclose(space) });
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
    } else if !filters.is_empty() {
        hdf5::sync::sync(|| unsafe { H5Pclose(dcpl) });
        hdf5::sync::sync(|| unsafe { H5Sclose(space) });
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    for filter in filters {
        let (filter_id, cd_nelmts, cd_values) = match filter {
            Filter::Shuffle => (H5Z_FILTER_SHUFFLE, 0, Vec::<u32>::new()),
            Filter::Deflate(level) => (H5Z_FILTER_DEFLATE, 1, vec![*level as u32]),
            Filter::Fletcher32 => (H5Z_FILTER_FLETCHER32, 0, Vec::<u32>::new()),
            _ => {
                hdf5::sync::sync(|| unsafe { H5Pclose(dcpl) });
                hdf5::sync::sync(|| unsafe { H5Sclose(space) });
                return Err(NisarSubsetWriteError::MetadataInvalid);
            }
        };
        if hdf5::sync::sync(|| unsafe {
            H5Pset_filter(dcpl, filter_id, 0, cd_nelmts, cd_values.as_ptr().cast())
        }) < 0
        {
            hdf5::sync::sync(|| unsafe { H5Pclose(dcpl) });
            hdf5::sync::sync(|| unsafe { H5Sclose(space) });
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
    }
    let name_c = CString::new(name).map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let dataset_id = hdf5::sync::sync(|| unsafe {
        H5Dcreate2(
            parent.id(),
            name_c.as_ptr(),
            datatype_id,
            space,
            H5P_DEFAULT,
            dcpl,
            H5P_DEFAULT,
        )
    });
    if dataset_id < 0 {
        hdf5::sync::sync(|| unsafe { H5Pclose(dcpl) });
        hdf5::sync::sync(|| unsafe { H5Sclose(space) });
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let wrote = hdf5::sync::sync(|| unsafe {
        H5Dwrite(
            dataset_id,
            datatype_id,
            H5S_ALL,
            H5S_ALL,
            H5P_DEFAULT,
            values.as_ptr().cast(),
        )
    });
    let closed = hdf5::sync::sync(|| unsafe { H5Dclose(dataset_id) });
    hdf5::sync::sync(|| unsafe { H5Pclose(dcpl) });
    hdf5::sync::sync(|| unsafe { H5Sclose(space) });
    if wrote < 0 || closed < 0 {
        return Err(NisarSubsetWriteError::Persistence);
    }
    parent
        .dataset(name)
        .map_err(|_| NisarSubsetWriteError::Persistence)
}

fn unique_standard_window(
    materials: &[DerivedWindowMaterial],
) -> Result<&DerivedWindowMaterial, NisarSubsetWriteError> {
    let first = materials
        .first()
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    if materials.iter().any(|other| {
        other.window.line_start != first.window.line_start
            || other.window.line_end_exclusive != first.window.line_end_exclusive
    }) {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok(first)
}

fn unique_science_material<'a>(
    materials: &'a [DerivedWindowMaterial],
    path: &str,
) -> Result<&'a DerivedWindowMaterial, NisarSubsetWriteError> {
    let matches = materials
        .iter()
        .filter(|material| material.window.dataset_path == path)
        .collect::<Vec<_>>();
    let first = *matches
        .first()
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    if matches.iter().any(|other| {
        other.window.line_start != first.window.line_start
            || other.window.line_end_exclusive != first.window.line_end_exclusive
            || other.window.sample_start != first.window.sample_start
            || other.window.sample_end_exclusive != first.window.sample_end_exclusive
    }) {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok(first)
}

fn unique_mask_material<'a>(
    materials: &'a [DerivedWindowMaterial],
    path: &str,
) -> Result<&'a DerivedWindowMaterial, NisarSubsetWriteError> {
    let frequency = frequency_from_standard_path(path)?;
    let matches = materials
        .iter()
        .filter(|material| {
            frequency_label(&material.window.frequency) == frequency && material.mask_path.is_some()
        })
        .collect::<Vec<_>>();
    let first = *matches
        .first()
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    if matches.iter().any(|other| {
        other.window.line_start != first.window.line_start
            || other.window.line_end_exclusive != first.window.line_end_exclusive
            || other.window.sample_start != first.window.sample_start
            || other.window.sample_end_exclusive != first.window.sample_end_exclusive
            || other.mask_chunk_shape != first.mask_chunk_shape
    }) {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok(first)
}

fn unique_frequency_material<'a>(
    materials: &'a [DerivedWindowMaterial],
    path: &str,
) -> Result<&'a DerivedWindowMaterial, NisarSubsetWriteError> {
    let frequency = frequency_from_standard_path(path)?;
    let matches = materials
        .iter()
        .filter(|material| frequency_label(&material.window.frequency) == frequency)
        .collect::<Vec<_>>();
    let first = *matches
        .first()
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    if matches.iter().any(|other| {
        other.window.sample_start != first.window.sample_start
            || other.window.sample_end_exclusive != first.window.sample_end_exclusive
            || other.window.line_start != first.window.line_start
            || other.window.line_end_exclusive != first.window.line_end_exclusive
    }) {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok(first)
}

fn frequency_from_standard_path(path: &str) -> Result<&str, NisarSubsetWriteError> {
    let value = path
        .strip_prefix("/science/LSAR/RSLC/swaths/frequency")
        .and_then(|value| value.split_once('/').map(|(frequency, _)| frequency))
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    if matches!(value, "A" | "B") {
        Ok(value)
    } else {
        Err(NisarSubsetWriteError::MetadataInvalid)
    }
}

fn derive_valid_samples(
    metadata: &NisarSubsetMetadataSnapshotV1,
    material: &DerivedWindowMaterial,
    frequency: &str,
) -> Result<Vec<u32>, NisarSubsetWriteError> {
    let source = metadata
        .evidence
        .valid_samples
        .get(frequency)
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    let start = usize::try_from(material.window.line_start)
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let end = usize::try_from(material.window.line_end_exclusive)
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let sample_start = u32::try_from(material.window.sample_start)
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let sample_end = u32::try_from(material.window.sample_end_exclusive)
        .ok()
        .and_then(|value| value.checked_sub(1))
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    let mut output = Vec::with_capacity((end - start).saturating_mul(2));
    for [valid_start, valid_end] in source
        .get(start..end)
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?
    {
        let clipped_start = (*valid_start).max(sample_start);
        let clipped_end = (*valid_end).min(sample_end);
        if clipped_start > clipped_end {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        output.push(clipped_start - sample_start);
        output.push(clipped_end - sample_start);
    }
    Ok(output)
}

fn write_derived_identification(
    parent: &Group,
    name: &str,
    binding: &SourceMetadataDatasetBindingV1,
    request: &NisarSubsetRequestManifestV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    materials: &[DerivedWindowMaterial],
) -> Result<Dataset, NisarSubsetWriteError> {
    if name == "isFullFrame" {
        // The real product stores `isFullFrame` as a scalar integer whose
        // element width differs from the synthetic fixture. Derive the
        // subset semantics (not a full frame) with a correctly-sized zero
        // byte buffer so the PreserveExact byte-length invariant holds.
        let zero = vec![0_u8; binding.datatype.size()];
        return create_raw_dataset_like(parent, name, binding, &zero);
    }
    let value = match name {
        "granuleId" => request.expected_output.derived_artifact_id.clone(),
        "boundingPolygon" => derive_bounding_polygon(metadata, materials)?,
        "zeroDopplerStartTime" | "zeroDopplerEndTime" => {
            derive_zero_doppler_time(metadata, materials, name.ends_with("EndTime"))?
        }
        _ => return Err(NisarSubsetWriteError::MetadataInvalid),
    };
    let TypeDescriptor::FixedAscii(capacity) = &binding.datatype else {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    };
    if value.len() > *capacity || !value.is_ascii() || binding.shape != Vec::<u64>::new() {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let mut bytes = vec![0_u8; *capacity];
    bytes[..value.len()].copy_from_slice(value.as_bytes());
    create_raw_dataset_like(parent, name, binding, &bytes)
}

fn derive_bounding_polygon(
    metadata: &NisarSubsetMetadataSnapshotV1,
    materials: &[DerivedWindowMaterial],
) -> Result<String, NisarSubsetWriteError> {
    let material = unique_standard_window(materials)?;
    let [height_count, grid_lines, grid_samples] = metadata.evidence.geolocation_shape;
    let source_lines = usize::try_from(material.window.source_shape[0])
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let source_samples = usize::try_from(material.window.source_shape[1])
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let line_indices = [
        material.window.line_start,
        material.window.line_end_exclusive - 1,
    ]
    .map(|value| scale_index(value, source_lines, grid_lines));
    let sample_indices = [
        material.window.sample_start,
        material.window.sample_end_exclusive - 1,
    ]
    .map(|value| scale_index(value, source_samples, grid_samples));
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for height in 0..height_count {
        for line in line_indices {
            for sample in sample_indices {
                let index = height
                    .checked_mul(grid_lines)
                    .and_then(|value| value.checked_add(line))
                    .and_then(|value| value.checked_mul(grid_samples))
                    .and_then(|value| value.checked_add(sample))
                    .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
                let x = *metadata
                    .evidence
                    .geolocation_x
                    .get(index)
                    .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
                let y = *metadata
                    .evidence
                    .geolocation_y
                    .get(index)
                    .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
                if !x.is_finite() || !y.is_finite() {
                    return Err(NisarSubsetWriteError::MetadataInvalid);
                }
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x >= max_x || min_y >= max_y {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok(format!(
        "POLYGON (({min_x:.12} {min_y:.12}, {max_x:.12} {min_y:.12}, {max_x:.12} {max_y:.12}, {min_x:.12} {max_y:.12}, {min_x:.12} {min_y:.12}))"
    ))
}

fn scale_index(value: u64, source_length: usize, grid_length: usize) -> usize {
    if source_length <= 1 || grid_length <= 1 {
        return 0;
    }
    let value = usize::try_from(value)
        .unwrap_or(source_length - 1)
        .min(source_length - 1);
    value.saturating_mul(grid_length - 1) / (source_length - 1)
}

fn derive_zero_doppler_time(
    metadata: &NisarSubsetMetadataSnapshotV1,
    materials: &[DerivedWindowMaterial],
    end: bool,
) -> Result<String, NisarSubsetWriteError> {
    let material = unique_standard_window(materials)?;
    let coordinate = if end {
        *material
            .line_coordinates
            .last()
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?
    } else {
        *material
            .line_coordinates
            .first()
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?
    };
    let source_axis_start = *metadata
        .evidence
        .zero_doppler_time
        .first()
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    let delta_seconds = coordinate - source_axis_start;
    let instant = parse_utc_rfc3339_nanos(&metadata.evidence.source_zero_doppler_start_time)?;
    let delta_nanos = (delta_seconds * 1_000_000_000.0).round() as i128;
    format_utc_rfc3339_nanos(
        instant
            .checked_add(delta_nanos)
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?,
    )
}

fn read_fixed_ascii_dataset(dataset: &Dataset) -> Result<String, NisarSubsetWriteError> {
    let descriptor = dataset
        .dtype()
        .and_then(|datatype| datatype.to_descriptor())
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let TypeDescriptor::FixedAscii(capacity) = descriptor else {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    };
    if !dataset.shape().is_empty() || capacity == 0 || capacity > 64 * 1024 {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let datatype = dataset
        .dtype()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let mut bytes = vec![0_u8; capacity];
    let status = hdf5::sync::sync(|| unsafe {
        H5Dread(
            dataset.id(),
            datatype.id(),
            H5S_ALL,
            H5S_ALL,
            H5P_DEFAULT,
            bytes.as_mut_ptr().cast(),
        )
    });
    if status < 0 {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..length])
        .map(str::to_owned)
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)
}

fn restore_dimension_scale_graph(
    file: &File,
    bindings: &[SourceMetadataDatasetBindingV1],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), NisarSubsetWriteError> {
    for binding in bindings {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        if binding.is_dimension_scale {
            let dataset = file
                .dataset(&binding.path)
                .map_err(|_| NisarSubsetWriteError::Persistence)?;
            match binding.scale_name.as_deref() {
                Some(scale_name) => dimension_scales::set_scale(&dataset, scale_name),
                None => dimension_scales::set_scale_unnamed(&dataset),
            }
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        }
    }
    for binding in bindings {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let dataset = file
            .dataset(&binding.path)
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        for (dimension, label) in binding.dimension_labels.iter().enumerate() {
            if !label.is_empty() {
                dimension_scales::set_label(&dataset, dimension, label)
                    .map_err(|_| NisarSubsetWriteError::Persistence)?;
            }
            for scale_path in &binding.dimension_scales[dimension] {
                dimension_scales::attach_scale(file, &dataset, scale_path, dimension)
                    .map_err(|_| NisarSubsetWriteError::Persistence)?;
            }
        }
    }
    Ok(())
}

fn parse_utc_rfc3339_nanos(value: &str) -> Result<i128, NisarSubsetWriteError> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || bytes
            .last()
            .is_some_and(|byte| !byte.is_ascii_digit() && *byte != b'Z')
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let number = |start: usize, end: usize| -> Result<i64, NisarSubsetWriteError> {
        value
            .get(start..end)
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?
            .parse::<i64>()
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let mut fraction = 0_i128;
    if bytes.len() > 20 {
        if bytes.get(19) != Some(&b'.') {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        // The real NISAR zero-Doppler timestamps carry 9 fractional digits
        // without a trailing 'Z' (UTC is implicit); accept both forms.
        let digits_end = if bytes.last() == Some(&b'Z') {
            bytes.len() - 1
        } else {
            bytes.len()
        };
        let digits = value
            .get(20..digits_end)
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
        if digits.is_empty()
            || digits.len() > 9
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        fraction = digits
            .parse::<i128>()
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?
            * 10_i128.pow((9 - digits.len()) as u32);
    }
    let seconds = i128::from(days_from_civil(year, month, day))
        .checked_mul(86_400)
        .and_then(|value| value.checked_add(i128::from(hour * 3_600 + minute * 60 + second)))
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(fraction))
        .ok_or(NisarSubsetWriteError::MetadataInvalid)
}

fn format_utc_rfc3339_nanos(nanoseconds: i128) -> Result<String, NisarSubsetWriteError> {
    let seconds = nanoseconds.div_euclid(1_000_000_000);
    let nanos = nanoseconds.rem_euclid(1_000_000_000) as u32;
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400) as i64;
    let days = i64::try_from(days).map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let (year, month, day) = civil_from_days(days);
    if !(0..=9999).contains(&year) {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:06}Z",
        nanos / 1_000
    ))
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn write_ascii_scalar<const N: usize>(
    group: &Group,
    name: &str,
    value: &str,
) -> Result<(), NisarSubsetWriteError> {
    let value =
        FixedAscii::<N>::from_ascii(value).map_err(|_| NisarSubsetWriteError::Persistence)?;
    group
        .new_dataset::<FixedAscii<N>>()
        .create(name)
        .and_then(|dataset| dataset.write_scalar(&value))
        .map_err(|_| NisarSubsetWriteError::Persistence)
}

fn write_scalar<T: H5Type>(
    group: &Group,
    name: &str,
    value: &T,
) -> Result<(), NisarSubsetWriteError> {
    group
        .new_dataset::<T>()
        .create(name)
        .and_then(|dataset| dataset.write_scalar(value))
        .map_err(|_| NisarSubsetWriteError::Persistence)
}

fn write_vector<T: H5Type>(
    group: &Group,
    name: &str,
    values: &[T],
) -> Result<(), NisarSubsetWriteError> {
    group
        .new_dataset::<T>()
        .shape((values.len(),))
        .create(name)
        .and_then(|dataset| dataset.write_raw(values))
        .map_err(|_| NisarSubsetWriteError::Persistence)
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
fn write_matrix<T: H5Type>(
    group: &Group,
    name: &str,
    rows: usize,
    cols: usize,
    values: &[T],
) -> Result<(), NisarSubsetWriteError> {
    if rows.checked_mul(cols) != Some(values.len()) {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    group
        .new_dataset::<T>()
        .shape((rows, cols))
        .create(name)
        .and_then(|dataset| dataset.write_raw(values))
        .map_err(|_| NisarSubsetWriteError::Persistence)
}

fn flatten_f64_3(values: &[[f64; 3]]) -> Vec<f64> {
    values
        .iter()
        .flat_map(|value| value.iter().copied())
        .collect()
}

fn flatten_f64_4(values: &[[f64; 4]]) -> Vec<f64> {
    values
        .iter()
        .flat_map(|value| value.iter().copied())
        .collect()
}

struct CandidateValidation {
    science: NisarSubsetScienceValidationV1,
    mask_sha256: Vec<String>,
    artifact_size_bytes: u64,
    artifact_sha256: String,
}

#[derive(Default)]
struct ScienceDigestState {
    complex: Sha256,
    amplitude: Sha256,
    phase: Sha256,
}

fn validate_science_window_streaming(
    dataset: &Dataset,
    material: &DerivedWindowMaterial,
    blocks: &[VerifiedHdf5ChunkBlockV1],
    mut digests: Option<&mut ScienceDigestState>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), NisarSubsetWriteError> {
    let rows = usize::try_from(material.window.expected_shape[0])
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let cols = usize::try_from(material.window.expected_shape[1])
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let band_rows = stream_band_rows(cols, std::mem::size_of::<CFloat32>())
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    for output_start in (0..rows).step_by(band_rows) {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let output_end = output_start.saturating_add(band_rows).min(rows);
        let line_start = material
            .window
            .line_start
            .checked_add(output_start as u64)
            .ok_or(NisarSubsetWriteError::ValidationFailed)?;
        let line_end = material
            .window
            .line_start
            .checked_add(output_end as u64)
            .ok_or(NisarSubsetWriteError::ValidationFailed)?;
        let expected =
            reconstruct_science_band(material, blocks, line_start, line_end).map_err(|error| {
                match error {
                    NisarSubsetWriteError::Cancelled => NisarSubsetWriteError::Cancelled,
                    _ => NisarSubsetWriteError::ValidationFailed,
                }
            })?;
        let actual: ndarray::Array2<CFloat32> = dataset
            .read_slice_2d((output_start..output_end, 0..cols))
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        if actual.len() != expected.len() {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
        for (left, right) in actual.iter().zip(&expected) {
            if left.r.to_bits() != right.r.to_bits() || left.i.to_bits() != right.i.to_bits() {
                return Err(NisarSubsetWriteError::ValidationFailed);
            }
            if let Some(digests) = digests.as_deref_mut() {
                digests.complex.update(left.r.to_le_bytes());
                digests.complex.update(left.i.to_le_bytes());
                let actual_amplitude = left.r.hypot(left.i);
                let expected_amplitude = right.r.hypot(right.i);
                let actual_phase = left.i.atan2(left.r);
                let expected_phase = right.i.atan2(right.r);
                if [left.r, left.i, right.r, right.i]
                    .into_iter()
                    .all(f32::is_finite)
                {
                    let phase_residual = (actual_phase - expected_phase + std::f32::consts::PI)
                        .rem_euclid(std::f32::consts::TAU)
                        - std::f32::consts::PI;
                    if (actual_amplitude - expected_amplitude).abs() > 1e-6
                        || phase_residual.abs() > 1e-6
                    {
                        return Err(NisarSubsetWriteError::ValidationFailed);
                    }
                    digests.amplitude.update([0_u8]);
                    digests.amplitude.update(actual_amplitude.to_le_bytes());
                    digests.phase.update([0_u8]);
                    digests.phase.update(actual_phase.to_le_bytes());
                } else {
                    digests.amplitude.update([1_u8]);
                    digests.amplitude.update(left.r.to_le_bytes());
                    digests.amplitude.update(left.i.to_le_bytes());
                    digests.phase.update([1_u8]);
                    digests.phase.update(left.r.to_le_bytes());
                    digests.phase.update(left.i.to_le_bytes());
                }
            }
        }
    }
    Ok(())
}

fn validate_mask_window_streaming(
    dataset: &Dataset,
    material: &DerivedWindowMaterial,
    blocks: &[VerifiedHdf5ChunkBlockV1],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<String, NisarSubsetWriteError> {
    let rows = usize::try_from(material.window.expected_shape[0])
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let cols = usize::try_from(material.window.expected_shape[1])
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let band_rows = stream_band_rows(cols, std::mem::size_of::<u8>())
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let source_path = source_mask_path(&material.window.frequency);
    let mut digest = Sha256::new();
    for output_start in (0..rows).step_by(band_rows) {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let output_end = output_start.saturating_add(band_rows).min(rows);
        let line_start = material
            .window
            .line_start
            .checked_add(output_start as u64)
            .ok_or(NisarSubsetWriteError::ValidationFailed)?;
        let line_end = material
            .window
            .line_start
            .checked_add(output_end as u64)
            .ok_or(NisarSubsetWriteError::ValidationFailed)?;
        let expected = reconstruct_mask_band(material, &source_path, blocks, line_start, line_end)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        let actual: ndarray::Array2<u8> = dataset
            .read_slice_2d((output_start..output_end, 0..cols))
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        if actual.len() != expected.len()
            || actual
                .iter()
                .zip(&expected)
                .any(|(left, right)| left != right)
        {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
        digest.update(
            actual
                .as_slice()
                .ok_or(NisarSubsetWriteError::ValidationFailed)?,
        );
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_candidate(
    candidate: &CandidateFileGuard,
    request: &NisarSubsetRequestManifestV1,
    transfer: &Hdf5RangeTransferResultV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    materials: &[DerivedWindowMaterial],
    blocks: &[VerifiedHdf5ChunkBlockV1],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<CandidateValidation, NisarSubsetWriteError> {
    let hdf5_path = candidate.hdf5_path();
    validate_derived_hdf5_security(&hdf5_path)
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    #[cfg(any(test, feature = "nisar-subset-live-acceptance"))]
    eprintln!("NISAR_WRITER_STAGE=security");

    candidate.verify_path_identity()?;
    let file =
        crate::hdf5_file::open(&hdf5_path).map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    if !file.is_read_only() {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }

    validate_exact_structure(&file, metadata, materials)?;
    #[cfg(any(test, feature = "nisar-subset-live-acceptance"))]
    eprintln!("NISAR_WRITER_STAGE=structure");

    validate_source_inventory(&file, metadata, materials, blocks)?;
    #[cfg(any(test, feature = "nisar-subset-live-acceptance"))]
    eprintln!("NISAR_WRITER_STAGE=source_inventory");

    validate_identification(&file, request, metadata, materials)?;
    #[cfg(any(test, feature = "nisar-subset-live-acceptance"))]
    eprintln!("NISAR_WRITER_STAGE=identification");

    validate_metadata(&file, metadata)?;
    #[cfg(any(test, feature = "nisar-subset-live-acceptance"))]
    eprintln!("NISAR_WRITER_STAGE=metadata");

    validate_provenance(&file, request, transfer, metadata)?;
    #[cfg(any(test, feature = "nisar-subset-live-acceptance"))]
    eprintln!("NISAR_WRITER_STAGE=provenance");

    let mut science_digests = ScienceDigestState::default();
    let mut coordinate_digest = Sha256::new();
    let mut mask_sha256 = Vec::new();
    for material in materials {
        let science = file
            .dataset(&material.science_path)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        validate_dataset_shape_type::<CFloat32>(
            &science,
            material.window.expected_shape,
            material.window.chunk_shape,
        )?;
        validate_science_window_streaming(
            &science,
            material,
            blocks,
            Some(&mut science_digests),
            cancelled,
        )?;
        let group = file
            .group(&material.root_path)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        let line_coordinates = read_exact::<f64>(
            &group,
            "zeroDopplerTime",
            &[material.line_coordinates.len()],
        )?;
        let sample_coordinates =
            read_exact::<f64>(&group, "slantRange", &[material.sample_coordinates.len()])?;
        if !f64_bitwise_eq(&line_coordinates, &material.line_coordinates)
            || !f64_bitwise_eq(&sample_coordinates, &material.sample_coordinates)
        {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
        for value in line_coordinates.iter().chain(&sample_coordinates) {
            coordinate_digest.update(value.to_le_bytes());
        }
        if let Some(mask_path) = &material.mask_path {
            let mask = file
                .dataset(mask_path)
                .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
            validate_dataset_shape_type::<u8>(
                &mask,
                material.window.expected_shape,
                material
                    .mask_chunk_shape
                    .ok_or(NisarSubsetWriteError::ValidationFailed)?,
            )?;
            mask_sha256.push(validate_mask_window_streaming(
                &mask, material, blocks, cancelled,
            )?);
        }
        validate_window_identity(&group, material)?;
        #[cfg(any(test, feature = "nisar-subset-live-acceptance"))]
        eprintln!("NISAR_WRITER_STAGE=science_window");
    }
    let structure = passed_check(
        "structure",
        expected_object_paths(metadata, materials).join("\n"),
    );
    let complex_values = passed_check("complex-values", science_digests.complex.finalize());
    let amplitude = passed_check("amplitude", science_digests.amplitude.finalize());
    let phase = passed_check("phase", science_digests.phase.finalize());
    let coordinates = passed_check("coordinates", coordinate_digest.finalize());
    let metadata_check = passed_check("metadata", metadata.evidence_sha256.as_bytes());
    let provenance = passed_check(
        "provenance",
        format!(
            "{}:{}:{}",
            request
                .sha256()
                .map_err(|_| NisarSubsetWriteError::RequestInvalid)?,
            transfer
                .manifest()
                .sha256()
                .map_err(|_| NisarSubsetWriteError::TransferIncomplete)?,
            metadata.evidence_sha256
        )
        .as_bytes(),
    );
    let recovery = passed_check("recovery", b"closed-before-promotion-v1");
    drop(file);
    let artifact_sha256 = candidate.sha256(cancelled)?;
    Ok(CandidateValidation {
        science: NisarSubsetScienceValidationV1 {
            structure,
            complex_values,
            phase,
            amplitude,
            coordinates,
            metadata: metadata_check,
            provenance,
            recovery,
        },
        mask_sha256,
        artifact_size_bytes: candidate.identity.size,
        artifact_sha256,
    })
}

fn validate_dataset_shape_type<T: H5Type>(
    dataset: &Dataset,
    shape: [u64; 2],
    source_chunk_shape: [u64; 2],
) -> Result<(), NisarSubsetWriteError> {
    let expected_shape = [
        usize::try_from(shape[0]).map_err(|_| NisarSubsetWriteError::ValidationFailed)?,
        usize::try_from(shape[1]).map_err(|_| NisarSubsetWriteError::ValidationFailed)?,
    ];
    let expected_chunk = vec![
        expected_shape[0].min(
            usize::try_from(source_chunk_shape[0])
                .map_err(|_| NisarSubsetWriteError::ValidationFailed)?,
        ),
        expected_shape[1].min(
            usize::try_from(source_chunk_shape[1])
                .map_err(|_| NisarSubsetWriteError::ValidationFailed)?,
        ),
    ];
    if dataset.shape() != expected_shape
        || dataset
            .dtype()
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
            != hdf5::Datatype::from_type::<T>()
                .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
        || dataset
            .dcpl()
            .and_then(|dcpl| dcpl.get_chunk())
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
            .as_deref()
            != Some(expected_chunk.as_slice())
    {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    Ok(())
}

fn validate_source_inventory(
    file: &File,
    metadata: &NisarSubsetMetadataSnapshotV1,
    materials: &[DerivedWindowMaterial],
    blocks: &[VerifiedHdf5ChunkBlockV1],
) -> Result<(), NisarSubsetWriteError> {
    for binding in &metadata.group_bindings {
        let group = file
            .group(&binding.path)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        if read_attribute_bindings(&group).map_err(|_| NisarSubsetWriteError::ValidationFailed)?
            != binding.attributes
        {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
    }
    for binding in &metadata.dataset_bindings {
        let dataset = file
            .dataset(&binding.path)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        let datatype = dataset
            .dtype()
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        let decoded_id = decode_datatype_hid(&binding.datatype_encoded)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        let equal =
            hdf5::sync::sync(|| unsafe { hdf5_sys::h5t::H5Tequal(decoded_id, datatype.id()) });
        hdf5::sync::sync(|| unsafe { H5Tclose(decoded_id) });
        if equal != 1
            || source_filter_bindings(&dataset)
                .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
                != binding.filters
            || read_attribute_bindings(&dataset)
                .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
                != binding.attributes
        {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
        let expected_shape = expected_output_shape(binding, materials)?;
        if dataset.shape() != expected_shape {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
        let actual_chunk = dataset
            .dcpl()
            .and_then(|dcpl| dcpl.get_chunk())
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        let expected_chunk = binding
            .chunk_shape
            .as_ref()
            .map(|chunk| {
                chunk
                    .iter()
                    .zip(&expected_shape)
                    .map(|(source, target)| {
                        usize::try_from(*source)
                            .map(|source| source.min(*target))
                            .map_err(|_| NisarSubsetWriteError::ValidationFailed)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        if actual_chunk != expected_chunk {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
        let graph = read_dimension_scale_graph(&dataset)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        if graph.0 != binding.dimension_labels
            || graph.1 != binding.dimension_scales
            || graph.2 != binding.scale_name
            || graph.3 != binding.is_dimension_scale
        {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
        match binding.action {
            SourceDatasetActionV1::PreserveExact => {
                if hash_dataset_values(&dataset, &|| false)
                    .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
                    != binding
                        .value_sha256
                        .as_deref()
                        .ok_or(NisarSubsetWriteError::ValidationFailed)?
                {
                    return Err(NisarSubsetWriteError::ValidationFailed);
                }
            }
            SourceDatasetActionV1::SelectedScience => {
                let material = unique_science_material(materials, &binding.path)?;
                validate_science_window_streaming(&dataset, material, blocks, None, &|| false)?;
            }
            SourceDatasetActionV1::FrequencyMask => {
                let material = unique_mask_material(materials, &binding.path)?;
                validate_mask_window_streaming(&dataset, material, blocks, &|| false)?;
            }
            SourceDatasetActionV1::ZeroDopplerTime => {
                let material = unique_standard_window(materials)?;
                let values = dataset
                    .read_raw::<f64>()
                    .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
                if !f64_bitwise_eq(&values, &material.line_coordinates) {
                    return Err(NisarSubsetWriteError::ValidationFailed);
                }
            }
            SourceDatasetActionV1::FrequencySlantRange => {
                let material = unique_frequency_material(materials, &binding.path)?;
                let values = dataset
                    .read_raw::<f64>()
                    .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
                if !f64_bitwise_eq(&values, &material.sample_coordinates) {
                    return Err(NisarSubsetWriteError::ValidationFailed);
                }
            }
            SourceDatasetActionV1::FrequencyValidSamples => {
                let material = unique_frequency_material(materials, &binding.path)?;
                let expected = derive_valid_samples(
                    metadata,
                    material,
                    frequency_from_standard_path(&binding.path)?,
                )?;
                let values = dataset
                    .read_raw::<u32>()
                    .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
                if values != expected {
                    return Err(NisarSubsetWriteError::ValidationFailed);
                }
            }
            SourceDatasetActionV1::DerivedIdentification => {}
        }
    }
    Ok(())
}

fn expected_output_shape(
    binding: &SourceMetadataDatasetBindingV1,
    materials: &[DerivedWindowMaterial],
) -> Result<Vec<usize>, NisarSubsetWriteError> {
    match binding.action {
        SourceDatasetActionV1::PreserveExact | SourceDatasetActionV1::DerivedIdentification => {
            binding
                .shape
                .iter()
                .map(|value| {
                    usize::try_from(*value).map_err(|_| NisarSubsetWriteError::ValidationFailed)
                })
                .collect()
        }
        SourceDatasetActionV1::SelectedScience => {
            let material = unique_science_material(materials, &binding.path)?;
            Ok(material
                .window
                .expected_shape
                .into_iter()
                .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
                .collect())
        }
        SourceDatasetActionV1::FrequencyMask => {
            let material = unique_mask_material(materials, &binding.path)?;
            Ok(material
                .window
                .expected_shape
                .into_iter()
                .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
                .collect())
        }
        SourceDatasetActionV1::ZeroDopplerTime => Ok(vec![unique_standard_window(materials)?
            .line_coordinates
            .len()]),
        SourceDatasetActionV1::FrequencySlantRange => {
            Ok(vec![unique_frequency_material(materials, &binding.path)?
                .sample_coordinates
                .len()])
        }
        SourceDatasetActionV1::FrequencyValidSamples => Ok(vec![
            unique_frequency_material(materials, &binding.path)?
                .line_coordinates
                .len(),
            2,
        ]),
    }
}

fn f64_bitwise_eq(left: &[f64], right: &[f64]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.to_bits() == right.to_bits())
}

fn f32_bitwise_eq(left: &[f32], right: &[f32]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.to_bits() == right.to_bits())
}

fn validate_identification(
    file: &File,
    request: &NisarSubsetRequestManifestV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    materials: &[DerivedWindowMaterial],
) -> Result<(), NisarSubsetWriteError> {
    let group = file
        .group(IDENTIFICATION_PATH)
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let expected_bounding_polygon = derive_bounding_polygon(metadata, materials)?;
    for (name, expected) in [
        ("missionId", metadata.evidence.mission_id.as_str()),
        ("productLevel", "L1"),
        ("radarBand", "L"),
        (
            "granuleId",
            request.expected_output.derived_artifact_id.as_str(),
        ),
        (
            "sourceGranuleId",
            metadata.evidence.source_granule_id.as_str(),
        ),
        ("sourceFileName", request.source.file_name.as_str()),
        (
            "sourceProviderProductId",
            request.source.provider_product_id.as_str(),
        ),
        (
            "productSpecificationVersion",
            request.source.product_specification_version.as_str(),
        ),
        ("boundingPolygon", expected_bounding_polygon.as_str()),
        ("outputAuthority", NISAR_SUBSET_OUTPUT_AUTHORITY),
        (
            "derivedProductType",
            OfflineNisarSubsetWriter::DERIVED_PRODUCT_TYPE,
        ),
    ] {
        let dataset = group
            .dataset(name)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        let actual = read_fixed_ascii_dataset(&dataset)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        if actual != expected {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
    }
    let is_full_dataset = group
        .dataset("isFullFrame")
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    // The real product stores `isFullFrame` as a scalar integer whose width
    // differs from the synthetic fixture. The derived subset always writes
    // zero bytes (not a full frame); validate the semantic value across the
    // actual scalar width instead of hard-coding u8.
    let is_full_datatype = is_full_dataset
        .dtype()
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let is_full_bytes = {
        let mut bytes = vec![0_u8; is_full_datatype.size()];
        let status = hdf5::sync::sync(|| unsafe {
            H5Dread(
                is_full_dataset.id(),
                is_full_datatype.id(),
                H5S_ALL,
                H5S_ALL,
                H5P_DEFAULT,
                bytes.as_mut_ptr().cast(),
            )
        });
        if status < 0 {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
        bytes
    };

    if is_full_bytes.iter().any(|byte| *byte != 0) {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    for (name, end) in [
        ("zeroDopplerStartTime", false),
        ("zeroDopplerEndTime", true),
    ] {
        let dataset = group
            .dataset(name)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        let actual = read_fixed_ascii_dataset(&dataset)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        let expected = derive_zero_doppler_time(metadata, materials, end)?;
        if actual != expected {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
    }
    Ok(())
}

fn validate_window_identity(
    group: &Group,
    material: &DerivedWindowMaterial,
) -> Result<(), NisarSubsetWriteError> {
    if read_ascii_with_capacity(group, "sourceDatasetPath", 512)? != material.window.dataset_path
        || read_scalar_exact::<u64>(group, "lineStart")? != material.window.line_start
        || read_scalar_exact::<u64>(group, "lineEndExclusive")?
            != material.window.line_end_exclusive
        || read_scalar_exact::<u64>(group, "sampleStart")? != material.window.sample_start
        || read_scalar_exact::<u64>(group, "sampleEndExclusive")?
            != material.window.sample_end_exclusive
        || read_scalar_exact::<u16>(group, "componentId")? != material.window.component_id
    {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    Ok(())
}

fn read_scalar_exact<T: H5Type + Copy>(
    group: &Group,
    name: &str,
) -> Result<T, NisarSubsetWriteError> {
    let dataset = group
        .dataset(name)
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    validate_exact_dataset_type_shape::<T>(&dataset, &[])?;
    dataset
        .read_scalar::<T>()
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)
}

fn read_ascii_with_capacity(
    group: &Group,
    name: &str,
    capacity: usize,
) -> Result<String, NisarSubsetWriteError> {
    // The real NISAR RSLC identification fields are fixed-length ASCII of
    // varying sizes (missionId=5, granuleId=91, boundingPolygon=2094). Read
    // the actual FixedAscii capacity and enforce it against the caller's
    // upper bound instead of requiring one exact fixture size.
    if capacity == 0 || capacity > 64 * 1024 {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    let dataset = group
        .dataset(name)
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    if !dataset.shape().is_empty() {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    let descriptor = dataset
        .dtype()
        .and_then(|datatype| datatype.to_descriptor())
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let TypeDescriptor::FixedAscii(size) = descriptor else {
        return Err(NisarSubsetWriteError::ValidationFailed);
    };
    if size == 0 || size > capacity {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    let datatype = dataset
        .dtype()
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let mut bytes = vec![0_u8; size];
    let status = hdf5::sync::sync(|| unsafe {
        H5Dread(
            dataset.id(),
            datatype.id(),
            H5S_ALL,
            H5S_ALL,
            H5P_DEFAULT,
            bytes.as_mut_ptr().cast(),
        )
    });
    if status < 0 {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..length])
        .map(str::to_owned)
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)
}

fn validate_metadata(
    file: &File,
    metadata: &NisarSubsetMetadataSnapshotV1,
) -> Result<(), NisarSubsetWriteError> {
    let orbit = file
        .group(ORBIT_PATH)
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    if !f64_bitwise_eq(
        &read_exact::<f64>(&orbit, "time", &[metadata.evidence.orbit_time.len()])?,
        &metadata.evidence.orbit_time,
    ) || !f64_bitwise_eq(
        &read_exact::<f64>(
            &orbit,
            "position",
            &[metadata.evidence.orbit_position.len(), 3],
        )?,
        &flatten_f64_3(&metadata.evidence.orbit_position),
    ) || !f64_bitwise_eq(
        &read_exact::<f64>(
            &orbit,
            "velocity",
            &[metadata.evidence.orbit_velocity.len(), 3],
        )?,
        &flatten_f64_3(&metadata.evidence.orbit_velocity),
    ) {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    let attitude = file
        .group(ATTITUDE_PATH)
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    if !f64_bitwise_eq(
        &read_exact::<f64>(&attitude, "time", &[metadata.evidence.attitude_time.len()])?,
        &metadata.evidence.attitude_time,
    ) || !f64_bitwise_eq(
        &read_exact::<f64>(
            &attitude,
            "quaternions",
            &[metadata.evidence.attitude_quaternions.len(), 4],
        )?,
        &flatten_f64_4(&metadata.evidence.attitude_quaternions),
    ) {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    let processing = file
        .group(&format!(
            "{PROCESSING_PATH}/parameters/frequency{}",
            metadata.evidence.selected_frequency
        ))
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    // The real NISAR RSLC stores these LUTs as float64 while the reviewed
    // evidence contract keeps float32. Read with the actual stored dtype and
    // narrow to the same float32 value bits before comparing.
    let doppler_dataset = processing
        .dataset("dopplerCentroid")
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    if !lut_shape_matches(&doppler_dataset, &metadata.evidence.doppler_shape)
        || !f32_bitwise_eq(
            &read_lut_f32(&doppler_dataset)?,
            &metadata.evidence.doppler_lut,
        )
    {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    let calibration = file
        .group(&format!("{CALIBRATION_PATH}/geometry"))
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let beta0_dataset = calibration
        .dataset("beta0")
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    if !lut_shape_matches(&beta0_dataset, &metadata.evidence.calibration_shape)
        || !f32_bitwise_eq(
            &read_lut_f32(&beta0_dataset)?,
            &metadata.evidence.calibration_lut,
        )
    {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    let geolocation = file
        .group(GEOLOCATION_PATH)
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let epsg_dataset = geolocation
        .dataset("epsg")
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    validate_exact_dataset_type_shape::<i32>(&epsg_dataset, &[])?;
    if epsg_dataset
        .read_scalar::<i32>()
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
        != metadata.evidence.geolocation_epsg
        || !f64_bitwise_eq(
            &read_exact::<f64>(
                &geolocation,
                "coordinateX",
                &metadata.evidence.geolocation_shape,
            )?,
            &metadata.evidence.geolocation_x,
        )
        || !f64_bitwise_eq(
            &read_exact::<f64>(
                &geolocation,
                "coordinateY",
                &metadata.evidence.geolocation_shape,
            )?,
            &metadata.evidence.geolocation_y,
        )
        || !f64_bitwise_eq(
            &read_exact::<f64>(
                &geolocation,
                "heightAboveEllipsoid",
                &[metadata.evidence.height_above_ellipsoid.len()],
            )?,
            &metadata.evidence.height_above_ellipsoid,
        )
    {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    Ok(())
}

/// True if the dataset shape matches the expected shape without requiring a
/// specific numeric dtype (the real product stores some LUTs as float64).
fn lut_shape_matches(dataset: &Dataset, expected_shape: &[usize]) -> bool {
    dataset.shape() == expected_shape
}

/// Read a 2-D numeric LUT with its actual stored dtype (float32 or float64)
/// and narrow to the reviewed float32 evidence contract with the same value
/// bits the writer's evidence capture produced.
fn read_lut_f32(dataset: &Dataset) -> Result<Vec<f32>, NisarSubsetWriteError> {
    let datatype = dataset
        .dtype()
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let f32_type =
        hdf5::Datatype::from_type::<f32>().map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let f64_type =
        hdf5::Datatype::from_type::<f64>().map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    if datatype == f32_type {
        return dataset
            .read_raw::<f32>()
            .map_err(|_| NisarSubsetWriteError::ValidationFailed);
    }
    if datatype == f64_type {
        let values = dataset
            .read_raw::<f64>()
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        return Ok(values.into_iter().map(|value| value as f32).collect());
    }
    Err(NisarSubsetWriteError::ValidationFailed)
}

fn read_exact<T: H5Type>(
    group: &Group,
    name: &str,
    shape: &[usize],
) -> Result<Vec<T>, NisarSubsetWriteError> {
    let dataset = group
        .dataset(name)
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    validate_exact_dataset_type_shape::<T>(&dataset, shape)?;
    dataset
        .read_raw::<T>()
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)
}

fn validate_exact_dataset_type_shape<T: H5Type>(
    dataset: &Dataset,
    shape: &[usize],
) -> Result<(), NisarSubsetWriteError> {
    if dataset.shape() != shape
        || dataset
            .dtype()
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
            != hdf5::Datatype::from_type::<T>()
                .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
    {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    Ok(())
}

fn validate_provenance(
    file: &File,
    request: &NisarSubsetRequestManifestV1,
    transfer: &Hdf5RangeTransferResultV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
) -> Result<(), NisarSubsetWriteError> {
    let group = file
        .group(PROVENANCE_PATH)
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let request_sha = request
        .sha256()
        .map_err(|_| NisarSubsetWriteError::RequestInvalid)?;
    let transfer_sha = transfer
        .manifest()
        .sha256()
        .map_err(|_| NisarSubsetWriteError::TransferIncomplete)?;
    for (name, expected, capacity) in [
        ("requestManifestSha256", request_sha.as_str(), 96),
        ("transferManifestSha256", transfer_sha.as_str(), 96),
        (
            "metadataEvidenceSha256",
            metadata.evidence_sha256.as_str(),
            96,
        ),
        (
            "sourceProductId",
            request.source.provider_product_id.as_str(),
            256,
        ),
        ("outputAuthority", NISAR_SUBSET_OUTPUT_AUTHORITY, 32),
        ("writerId", OfflineNisarSubsetWriter::WRITER_ID, 64),
        (
            "sourceInventorySha256",
            metadata.source_inventory_sha256.as_str(),
            96,
        ),
        ("statisticsScope", "derived_subset", 32),
    ] {
        if read_ascii_with_capacity(&group, name, capacity)? != expected {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
    }
    Ok(())
}

fn expected_object_paths(
    metadata: &NisarSubsetMetadataSnapshotV1,
    materials: &[DerivedWindowMaterial],
) -> Vec<String> {
    let mut datasets = metadata
        .dataset_bindings
        .iter()
        .map(|binding| binding.path.clone())
        .collect::<Vec<_>>();
    datasets.extend([
        format!("{IDENTIFICATION_PATH}/sourceGranuleId"),
        format!("{IDENTIFICATION_PATH}/sourceFileName"),
        format!("{IDENTIFICATION_PATH}/sourceProviderProductId"),
        format!("{IDENTIFICATION_PATH}/outputAuthority"),
        format!("{IDENTIFICATION_PATH}/derivedProductType"),
        format!("{PROVENANCE_PATH}/requestManifestSha256"),
        format!("{PROVENANCE_PATH}/transferManifestSha256"),
        format!("{PROVENANCE_PATH}/metadataEvidenceSha256"),
        format!("{PROVENANCE_PATH}/sourceProductId"),
        format!("{PROVENANCE_PATH}/outputAuthority"),
        format!("{PROVENANCE_PATH}/writerId"),
        format!("{PROVENANCE_PATH}/sourceInventorySha256"),
        format!("{PROVENANCE_PATH}/statisticsScope"),
    ]);
    for material in materials {
        datasets.extend([
            material.science_path.clone(),
            format!("{}/zeroDopplerTime", material.root_path),
            format!("{}/slantRange", material.root_path),
            format!("{}/sourceDatasetPath", material.root_path),
            format!("{}/lineStart", material.root_path),
            format!("{}/lineEndExclusive", material.root_path),
            format!("{}/sampleStart", material.root_path),
            format!("{}/sampleEndExclusive", material.root_path),
            format!("{}/componentId", material.root_path),
        ]);
        if let Some(mask_path) = &material.mask_path {
            datasets.push(mask_path.clone());
        }
    }
    let mut objects = BTreeSet::new();
    for binding in &metadata.group_bindings {
        if binding.path != "/" {
            objects.insert(binding.path.clone());
        }
    }
    for dataset in datasets {
        objects.insert(dataset.clone());
        let mut current = dataset.as_str();
        while let Some((parent, _)) = current.rsplit_once('/') {
            if parent.is_empty() {
                break;
            }
            objects.insert(parent.to_owned());
            current = parent;
        }
    }
    objects.into_iter().collect()
}

fn validate_exact_structure(
    file: &File,
    metadata: &NisarSubsetMetadataSnapshotV1,
    materials: &[DerivedWindowMaterial],
) -> Result<(), NisarSubsetWriteError> {
    let mut actual = BTreeSet::new();
    let root = file
        .group("/")
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let source_objects = metadata
        .group_bindings
        .iter()
        .map(|binding| binding.path.clone())
        .chain(
            metadata
                .dataset_bindings
                .iter()
                .map(|binding| binding.path.clone()),
        )
        .collect::<BTreeSet<_>>();
    collect_object_paths(&root, "/", &source_objects, &mut actual)?;
    let expected = expected_object_paths(metadata, materials)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(NisarSubsetWriteError::ValidationFailed);
    }
    Ok(())
}

fn collect_object_paths(
    group: &Group,
    parent: &str,
    source_objects: &BTreeSet<String>,
    paths: &mut BTreeSet<String>,
) -> Result<(), NisarSubsetWriteError> {
    let mut names = group
        .member_names()
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    names.sort();
    for name in names {
        let path = if parent == "/" {
            format!("/{name}")
        } else {
            format!("{parent}/{name}")
        };
        if !paths.insert(path.clone()) {
            return Err(NisarSubsetWriteError::ValidationFailed);
        }
        let info = group
            .loc_info_by_name(&name)
            .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
        match info.loc_type {
            LocationType::Group => {
                let child = group
                    .group(&name)
                    .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
                if !source_objects.contains(&path)
                    && !child
                        .attr_names()
                        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
                        .is_empty()
                {
                    return Err(NisarSubsetWriteError::ValidationFailed);
                }
                collect_object_paths(&child, &path, source_objects, paths)?;
            }
            LocationType::Dataset => {
                let dataset = group
                    .dataset(&name)
                    .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
                if !source_objects.contains(&path)
                    && !dataset
                        .attr_names()
                        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?
                        .is_empty()
                {
                    return Err(NisarSubsetWriteError::ValidationFailed);
                }
            }
            _ => return Err(NisarSubsetWriteError::ValidationFailed),
        }
    }
    Ok(())
}

fn passed_check(label: &str, evidence: impl AsRef<[u8]>) -> NisarSubsetValidationCheckV1 {
    NisarSubsetValidationCheckV1 {
        status: NisarSubsetCheckStatus::Passed,
        evidence: format!("{label}:sha256:{}", sha256_bytes(evidence.as_ref())),
    }
}

fn build_artifact_manifest(
    request: &NisarSubsetRequestManifestV1,
    transfer: &Hdf5RangeTransferResultV1,
    materials: &[DerivedWindowMaterial],
    validation: CandidateValidation,
) -> Result<NisarSubsetArtifactManifestV1, NisarSubsetWriteError> {
    let artifact_size_bytes = validation.artifact_size_bytes;
    let artifact_sha256 = validation.artifact_sha256.clone();
    let line_start = materials
        .iter()
        .map(|material| material.window.line_start)
        .min()
        .ok_or(NisarSubsetWriteError::ValidationFailed)?;
    let line_end_exclusive = materials
        .iter()
        .map(|material| material.window.line_end_exclusive)
        .max()
        .ok_or(NisarSubsetWriteError::ValidationFailed)?;
    let sample_start = materials
        .iter()
        .map(|material| material.window.sample_start)
        .min()
        .ok_or(NisarSubsetWriteError::ValidationFailed)?;
    let sample_end_exclusive = materials
        .iter()
        .map(|material| material.window.sample_end_exclusive)
        .max()
        .ok_or(NisarSubsetWriteError::ValidationFailed)?;
    let mut masks = Vec::new();
    let mut digest_index = 0_usize;
    for material in materials {
        if material.mask_path.is_some() {
            let sha256 = validation
                .mask_sha256
                .get(digest_index)
                .cloned()
                .ok_or(NisarSubsetWriteError::ValidationFailed)?;
            digest_index += 1;
            masks.push(NisarSubsetValidityMaskV1 {
                component_id: material.window.component_id,
                dataset_path: source_mask_path(&material.window.frequency),
                frequency: material.window.frequency.clone(),
                swath: material.window.swath.clone(),
                line_start: material.window.line_start,
                line_end_exclusive: material.window.line_end_exclusive,
                sample_start: material.window.sample_start,
                sample_end_exclusive: material.window.sample_end_exclusive,
                shape: material.window.expected_shape,
                sha256,
                semantics: "source_validity_mask".to_owned(),
            });
        }
    }
    let manifest = NisarSubsetArtifactManifestV1 {
        manifest_contract: NISAR_SUBSET_ARTIFACT_MANIFEST_CONTRACT.to_owned(),
        schema_version: NISAR_SUBSET_SCHEMA_VERSION,
        request_manifest_sha256: request
            .sha256()
            .map_err(|_| NisarSubsetWriteError::RequestInvalid)?,
        transfer_manifest_sha256: transfer
            .manifest()
            .sha256()
            .map_err(|_| NisarSubsetWriteError::TransferIncomplete)?,
        output_authority: NISAR_SUBSET_OUTPUT_AUTHORITY.to_owned(),
        artifact_id: request.expected_output.derived_artifact_id.clone(),
        artifact_file_name: request.expected_output.file_name.clone(),
        artifact_size_bytes,
        artifact_sha256,
        source_product_identity: request.source.clone(),
        actual_extent: NisarSubsetActualExtentV1 {
            line_start,
            line_end_exclusive,
            sample_start,
            sample_end_exclusive,
            component_count: request.request.aoi_component_count,
        },
        dataset_windows: request.mapping.windows.clone(),
        validity_masks: masks,
        writer: NisarSubsetWriterV1 {
            writer_id: OfflineNisarSubsetWriter::WRITER_ID.to_owned(),
            version: 1,
        },
        validator: NisarSubsetValidatorPolicyV1 {
            policy_id: "nisar.rslc.subset".to_owned(),
            version: 1,
            product_specification_version: request.source.product_specification_version.clone(),
        },
        science_validation: validation.science,
        remote_bytes: transfer.manifest().received_bytes,
        full_product_bytes: request.expected_output.full_product_bytes,
        complete: true,
    };
    manifest
        .validate_against_request_and_transfer(request, transfer.manifest())
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    Ok(manifest)
}

fn persist_artifact_manifest(
    destination: &Path,
    manifest: &NisarSubsetArtifactManifestV1,
) -> Result<(), NisarSubsetWriteError> {
    let parent = destination
        .parent()
        .ok_or(NisarSubsetWriteError::OutputUnsafe)?;
    let temp = parent.join(format!(".artifact-manifest-{}.writing", temp_nonce()));
    let canonical = manifest
        .canonical_json()
        .map_err(|_| NisarSubsetWriteError::ValidationFailed)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    file.write_all(canonical.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    drop(file);
    promote_verified_closed_artifact(
        VerifiedClosedArtifact::after_scientific_validation(
            temp,
            sha256_bytes(canonical.as_bytes()),
            &|| false,
        )
        .map_err(|_| NisarSubsetWriteError::Persistence)?,
        destination,
        None,
    )
    .map_err(|_| NisarSubsetWriteError::Persistence)?;
    sync_parent_directory(destination)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), NisarSubsetWriteError> {
    let parent = path.parent().ok_or(NisarSubsetWriteError::OutputUnsafe)?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| NisarSubsetWriteError::Persistence)
}

#[cfg(windows)]
fn sync_parent_directory(_path: &Path) -> Result<(), NisarSubsetWriteError> {
    // Both the destination-absent and replacement branches in safe_promotion
    // use the corresponding Win32 WRITE_THROUGH flag before reporting success.
    Ok(())
}

#[cfg(all(not(windows), not(unix)))]
fn sync_parent_directory(_path: &Path) -> Result<(), NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::Persistence)
}

fn sha256_reader(
    file: &mut fs::File,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<String, NisarSubsetWriteError> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let count = file
            .read(&mut buffer)
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(all(feature = "nisar-subset-production", windows))]
fn set_windows_delete_pending(file: &fs::File, delete: bool) -> Result<(), NisarSubsetWriteError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
    };

    let disposition = FILE_DISPOSITION_INFO { DeleteFile: delete };
    let succeeded = unsafe {
        // SAFETY: the exact retained handle and fixed-size disposition structure
        // remain live for the complete synchronous request.
        SetFileInformationByHandle(
            file.as_raw_handle() as _,
            FileDispositionInfo,
            (&raw const disposition).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    (succeeded != 0)
        .then_some(())
        .ok_or(NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(all(feature = "nisar-subset-production", windows))]
fn create_v2_candidate_bootstrap(
    path: &Path,
) -> Result<V2CandidateBootstrap, NisarSubsetWriteError> {
    use std::os::windows::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000)
        .share_mode(0x0000_0001 | 0x0000_0002 | 0x0000_0004)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let identity = candidate_file_identity(&file)?;
    set_windows_delete_pending(&file, true)?;
    Ok(V2CandidateBootstrap {
        named_path: Some(path.to_path_buf()),
        file: Some(file),
        identity,
    })
}

#[cfg(all(feature = "nisar-subset-production", windows))]
fn persist_v2_candidate_bootstrap(
    bootstrap: &mut V2CandidateBootstrap,
) -> Result<(), NisarSubsetWriteError> {
    let file = bootstrap
        .file
        .take()
        .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
    set_windows_delete_pending(&file, false)?;
    drop(file);
    Ok(())
}

#[cfg(all(feature = "nisar-subset-production", windows))]
fn sync_v2_candidate_bootstrap(
    bootstrap: &V2CandidateBootstrap,
) -> Result<(), NisarSubsetWriteError> {
    use std::os::windows::fs::OpenOptionsExt;

    let path = bootstrap
        .named_path
        .as_deref()
        .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    if !candidate_file_identity(&file)?.same_object(&bootstrap.identity) {
        return Err(NisarSubsetWriteError::RecoveryInvalid);
    }
    file.sync_all()
        .map_err(|_| NisarSubsetWriteError::Persistence)
}

#[cfg(all(feature = "nisar-subset-production", windows))]
fn seal_v2_candidate_bootstrap(
    mut bootstrap: V2CandidateBootstrap,
) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
    bootstrap.sync()?;
    let path = bootstrap
        .named_path
        .take()
        .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
    let sealed = V2CandidateFileGuard::open_named_immutable(&path)?;
    if !sealed.identity.same_object(&bootstrap.identity) {
        return Err(NisarSubsetWriteError::RecoveryInvalid);
    }
    Ok(sealed)
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn create_v2_promotion_bootstrap(
    publication: &PinnedDirectoryPublication,
) -> Result<V2CandidateBootstrap, NisarSubsetWriteError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    publication
        .verify()
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let directory = publication
        .try_clone_directory_handle()
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let current = CString::new(".").map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let fd = unsafe {
        // SAFETY: the exact retained directory descriptor and fixed current-directory
        // component remain live for creation of one unnamed same-directory inode.
        libc::openat(
            directory.as_raw_fd(),
            current.as_ptr(),
            libc::O_TMPFILE | libc::O_CLOEXEC | libc::O_RDWR,
            0o600,
        )
    };
    if fd < 0 {
        return Err(NisarSubsetWriteError::OutputUnsafe);
    }
    let file = unsafe {
        // SAFETY: ownership of the newly returned descriptor is transferred once.
        fs::File::from_raw_fd(fd)
    };
    let identity = unnamed_candidate_file_identity(&file)?;
    Ok(V2CandidateBootstrap {
        named_path: None,
        file: Some(file),
        identity,
    })
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn create_v2_candidate_bootstrap(
    path: &Path,
) -> Result<V2CandidateBootstrap, NisarSubsetWriteError> {
    use std::os::unix::fs::OpenOptionsExt;

    let parent = path.parent().ok_or(NisarSubsetWriteError::OutputUnsafe)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_TMPFILE | libc::O_CLOEXEC)
        .open(parent)
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let identity = unnamed_candidate_file_identity(&file)?;
    Ok(V2CandidateBootstrap {
        named_path: None,
        file: Some(file),
        identity,
    })
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn persist_v2_candidate_bootstrap(
    _bootstrap: &mut V2CandidateBootstrap,
) -> Result<(), NisarSubsetWriteError> {
    Ok(())
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn sync_v2_candidate_bootstrap(
    bootstrap: &V2CandidateBootstrap,
) -> Result<(), NisarSubsetWriteError> {
    bootstrap
        .file
        .as_ref()
        .ok_or(NisarSubsetWriteError::RecoveryInvalid)?
        .sync_all()
        .map_err(|_| NisarSubsetWriteError::Persistence)
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn seal_v2_candidate_bootstrap(
    mut bootstrap: V2CandidateBootstrap,
) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::PermissionsExt;

    bootstrap.sync()?;
    let writable = bootstrap
        .file
        .take()
        .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
    let descriptor_path = PathBuf::from(format!("/proc/self/fd/{}", writable.as_raw_fd()));
    let file = OpenOptions::new()
        .read(true)
        .open(descriptor_path)
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    drop(writable);
    enable_candidate_fs_verity(&file)?;
    file.set_permissions(fs::Permissions::from_mode(0o400))
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    V2CandidateFileGuard::from_unnamed(file)
}

#[cfg(all(feature = "nisar-subset-production", target_os = "macos"))]
fn create_v2_candidate_bootstrap(
    path: &Path,
) -> Result<V2CandidateBootstrap, NisarSubsetWriteError> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(CANDIDATE_O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let identity = candidate_file_identity(&file)?;
    Ok(V2CandidateBootstrap {
        named_path: Some(path.to_path_buf()),
        file: Some(file),
        identity,
    })
}

#[cfg(all(feature = "nisar-subset-production", target_os = "macos"))]
fn persist_v2_candidate_bootstrap(
    bootstrap: &mut V2CandidateBootstrap,
) -> Result<(), NisarSubsetWriteError> {
    let file = bootstrap
        .file
        .take()
        .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
    drop(file);
    Ok(())
}

#[cfg(all(feature = "nisar-subset-production", target_os = "macos"))]
fn sync_v2_candidate_bootstrap(
    bootstrap: &V2CandidateBootstrap,
) -> Result<(), NisarSubsetWriteError> {
    use std::os::unix::fs::OpenOptionsExt;

    let path = bootstrap
        .named_path
        .as_deref()
        .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(CANDIDATE_O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    if !candidate_file_identity(&file)?.same_object(&bootstrap.identity) {
        return Err(NisarSubsetWriteError::RecoveryInvalid);
    }
    file.sync_all()
        .map_err(|_| NisarSubsetWriteError::Persistence)
}

#[cfg(all(feature = "nisar-subset-production", target_os = "macos"))]
fn seal_v2_candidate_bootstrap(
    mut bootstrap: V2CandidateBootstrap,
) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
    bootstrap.sync()?;
    let path = bootstrap
        .named_path
        .take()
        .ok_or(NisarSubsetWriteError::RecoveryInvalid)?;
    let sealed = V2CandidateFileGuard::open_named_immutable(&path)?;
    if !sealed.identity.same_object(&bootstrap.identity) {
        return Err(NisarSubsetWriteError::RecoveryInvalid);
    }
    Ok(sealed)
}

#[cfg(all(
    feature = "nisar-subset-production",
    not(any(
        windows,
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    ))
))]
fn create_v2_candidate_bootstrap(
    _path: &Path,
) -> Result<V2CandidateBootstrap, NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(all(
    feature = "nisar-subset-production",
    not(any(
        windows,
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    ))
))]
fn persist_v2_candidate_bootstrap(
    _bootstrap: &mut V2CandidateBootstrap,
) -> Result<(), NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(all(
    feature = "nisar-subset-production",
    not(any(
        windows,
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    ))
))]
fn sync_v2_candidate_bootstrap(
    _bootstrap: &V2CandidateBootstrap,
) -> Result<(), NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(all(
    feature = "nisar-subset-production",
    not(any(
        windows,
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    ))
))]
fn seal_v2_candidate_bootstrap(
    _bootstrap: V2CandidateBootstrap,
) -> Result<V2CandidateFileGuard, NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(windows)]
fn open_candidate_file(path: &Path) -> Result<fs::File, NisarSubsetWriteError> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        // HDF5 may open a second read handle, but rename/delete remains denied
        // while validation and hashing retain this identity handle.
        .share_mode(0x0000_0001)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|error| {
            metadata_capture_detail!(format!(
                "open_candidate_file: kind={:?}; raw_os_error={:?}",
                error.kind(),
                error.raw_os_error()
            ));
            NisarSubsetWriteError::OutputUnsafe
        })
}

#[cfg(all(feature = "nisar-subset-production", any(windows, target_os = "macos")))]
fn open_immutable_v2_candidate_file(path: &Path) -> Result<fs::File, NisarSubsetWriteError> {
    open_candidate_file(path)
}

#[cfg(all(feature = "nisar-subset-production", windows))]
fn remove_candidate_if_identity_matches(
    path: &Path,
    expected: &CandidateFileIdentity,
) -> Result<bool, NisarSubsetWriteError> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
    };

    let file = OpenOptions::new()
        .read(true)
        .access_mode(0x8000_0000 | 0x0001_0000)
        .share_mode(0x0000_0001 | 0x0000_0004)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    if !candidate_file_identity(&file)?.same_object(expected) {
        return Ok(false);
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let succeeded = unsafe {
        // SAFETY: the retained exact-identity handle and fixed-size disposition
        // structure remain live for the complete Windows deletion request.
        SetFileInformationByHandle(
            file.as_raw_handle() as _,
            FileDispositionInfo,
            (&raw const disposition).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if succeeded == 0 {
        return Err(NisarSubsetWriteError::OutputUnsafe);
    }
    Ok(true)
}

#[cfg(all(feature = "nisar-subset-production", target_os = "macos"))]
fn remove_candidate_if_identity_matches(
    path: &Path,
    expected: &CandidateFileIdentity,
) -> Result<bool, NisarSubsetWriteError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;

    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(NisarSubsetWriteError::OutputUnsafe);
    };
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let name = CString::new(name.as_bytes()).map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(parent)
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `directory` is an open directory fd, `name` is a NUL-terminated
    // component, and `status` is writable storage for `fstatat`.
    let found = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            status.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if found != 0 {
        return Err(NisarSubsetWriteError::OutputUnsafe);
    }
    // SAFETY: `fstatat` returned 0, so `status` is initialized.
    let status = unsafe { status.assume_init() };
    #[allow(clippy::unnecessary_cast)]
    let same_single_link = status.st_mode & libc::S_IFMT == libc::S_IFREG
        && status.st_nlink == 1
        && expected.platform == "unix"
        && status.st_dev as u64 == expected.device_id
        && status.st_ino as u64 == expected.file_id;
    if !same_single_link {
        return Ok(false);
    }
    // SAFETY: same live directory fd and component; flag 0 unlinks a regular file.
    let removed = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) == 0 };
    if !removed {
        return Err(NisarSubsetWriteError::OutputUnsafe);
    }
    Ok(true)
}

#[cfg(all(
    feature = "nisar-subset-production",
    not(any(windows, target_os = "macos"))
))]
fn remove_candidate_if_identity_matches(
    _path: &Path,
    _expected: &CandidateFileIdentity,
) -> Result<bool, NisarSubsetWriteError> {
    // Linux/Android V2 candidates are unnamed descriptors and other Unix targets
    // fail before candidate construction. There is deliberately no pathname cleanup.
    Ok(false)
}

#[cfg(windows)]
fn sync_candidate_path(path: &Path) -> Result<(), NisarSubsetWriteError> {
    use std::os::windows::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    candidate_file_identity(&file)?;
    file.sync_all()
        .map_err(|_| NisarSubsetWriteError::Persistence)
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
const CANDIDATE_O_NOFOLLOW: i32 = 0x0000_8000;
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
const CANDIDATE_O_NOFOLLOW: i32 = 0x0002_0000;
#[cfg(all(
    target_os = "android",
    any(target_arch = "arm", target_arch = "aarch64")
))]
const CANDIDATE_O_NOFOLLOW: i32 = 0x0000_8000;
#[cfg(all(
    target_os = "android",
    any(target_arch = "x86", target_arch = "x86_64")
))]
const CANDIDATE_O_NOFOLLOW: i32 = 0x0002_0000;
#[cfg(all(target_os = "android", target_arch = "riscv64"))]
const CANDIDATE_O_NOFOLLOW: i32 = 0x0040_0000;
#[cfg(any(target_os = "solaris", target_os = "illumos"))]
const CANDIDATE_O_NOFOLLOW: i32 = 0x0002_0000;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
const CANDIDATE_O_NOFOLLOW: i32 = 0x0000_0100;

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
fn open_candidate_file(path: &Path) -> Result<fs::File, NisarSubsetWriteError> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(CANDIDATE_O_NOFOLLOW)
        .open(path)
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn enable_candidate_fs_verity(file: &fs::File) -> Result<(), NisarSubsetWriteError> {
    use std::os::fd::AsRawFd;

    #[repr(C)]
    struct FsVerityEnableArg {
        version: u32,
        hash_algorithm: u32,
        block_size: u32,
        salt_size: u32,
        salt_ptr: u64,
        sig_size: u32,
        reserved1: u32,
        sig_ptr: u64,
        reserved2: [u64; 11],
    }
    let argument = FsVerityEnableArg {
        version: 1,
        hash_algorithm: 1,
        block_size: 0,
        salt_size: 0,
        salt_ptr: 0,
        sig_size: 0,
        reserved1: 0,
        sig_ptr: 0,
        reserved2: [0; 11],
    };
    const FS_IOC_ENABLE_VERITY: libc::c_ulong = 0x4080_6685;
    let status = unsafe {
        // SAFETY: the argument uses the kernel fs-verity ABI and both it and the
        // retained regular-file descriptor remain live for the synchronous ioctl.
        libc::ioctl(file.as_raw_fd(), FS_IOC_ENABLE_VERITY, &argument)
    };
    if status == 0 {
        return Ok(());
    }
    // Owner decision 2026-11-02: a filesystem without fs-verity keeps the
    // read-only candidate; its content is still digested before promotion.
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EEXIST)
        || crate::hdf5_range_transfer::verity_unsupported(&error)
    {
        Ok(())
    } else {
        Err(NisarSubsetWriteError::OutputUnsafe)
    }
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn open_immutable_v2_candidate_file(path: &Path) -> Result<fs::File, NisarSubsetWriteError> {
    use std::os::unix::fs::PermissionsExt;

    let file = open_candidate_file(path)?;
    candidate_file_identity(&file)?;
    enable_candidate_fs_verity(&file)?;
    file.set_permissions(fs::Permissions::from_mode(0o400))
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    candidate_file_identity(&file)?;
    Ok(file)
}

#[cfg(all(
    feature = "nisar-subset-production",
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn open_immutable_v2_candidate_file(_path: &Path) -> Result<fs::File, NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(all(feature = "nisar-subset-production", not(any(windows, unix))))]
fn open_immutable_v2_candidate_file(_path: &Path) -> Result<fs::File, NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
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
fn sync_candidate_path(path: &Path) -> Result<(), NisarSubsetWriteError> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(CANDIDATE_O_NOFOLLOW)
        .open(path)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    candidate_file_identity(&file)?;
    file.sync_all()
        .map_err(|_| NisarSubsetWriteError::Persistence)
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
fn open_candidate_file(_path: &Path) -> Result<fs::File, NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
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
fn sync_candidate_path(_path: &Path) -> Result<(), NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::Persistence)
}

#[cfg(all(not(windows), not(unix)))]
fn open_candidate_file(_path: &Path) -> Result<fs::File, NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(all(not(windows), not(unix)))]
fn sync_candidate_path(_path: &Path) -> Result<(), NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::Persistence)
}

#[cfg(windows)]
fn candidate_file_identity(
    file: &fs::File,
) -> Result<CandidateFileIdentity, NisarSubsetWriteError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) };
    if succeeded == 0 || info.dwFileAttributes & 0x0000_0400 != 0 || info.nNumberOfLinks != 1 {
        metadata_capture_detail!(format!(
            "candidate_file_identity: succeeded={succeeded}; attributes=0x{:08x}; links={}",
            info.dwFileAttributes, info.nNumberOfLinks
        ));
        return Err(NisarSubsetWriteError::OutputUnsafe);
    }
    Ok(CandidateFileIdentity {
        platform: "windows".to_owned(),
        device_id: u64::from(info.dwVolumeSerialNumber),
        file_id: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
    })
}

#[cfg(all(
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android")
))]
fn unnamed_candidate_file_identity(
    file: &fs::File,
) -> Result<CandidateFileIdentity, NisarSubsetWriteError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file
        .metadata()
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    if !metadata.is_file() || metadata.nlink() != 0 {
        return Err(NisarSubsetWriteError::OutputUnsafe);
    }
    Ok(CandidateFileIdentity {
        platform: "unix-unnamed".to_owned(),
        device_id: metadata.dev(),
        file_id: metadata.ino(),
        size: metadata.len(),
    })
}

#[cfg(unix)]
fn candidate_file_identity(
    file: &fs::File,
) -> Result<CandidateFileIdentity, NisarSubsetWriteError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file
        .metadata()
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(NisarSubsetWriteError::OutputUnsafe);
    }
    Ok(CandidateFileIdentity {
        platform: "unix".to_owned(),
        device_id: metadata.dev(),
        file_id: metadata.ino(),
        size: metadata.len(),
    })
}

#[cfg(all(not(windows), not(unix)))]
fn candidate_file_identity(
    _file: &fs::File,
) -> Result<CandidateFileIdentity, NisarSubsetWriteError> {
    Err(NisarSubsetWriteError::OutputUnsafe)
}

#[cfg(windows)]
fn candidate_hdf5_path(path: &Path, _file: &fs::File) -> PathBuf {
    path.to_path_buf()
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "solaris",
    target_os = "illumos"
))]
fn candidate_hdf5_path(_path: &Path, file: &fs::File) -> PathBuf {
    use std::os::unix::io::AsRawFd;
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn candidate_hdf5_path(_path: &Path, file: &fs::File) -> PathBuf {
    use std::os::unix::io::AsRawFd;
    PathBuf::from(format!("/dev/fd/{}", file.as_raw_fd()))
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
fn candidate_hdf5_path(path: &Path, _file: &fs::File) -> PathBuf {
    path.to_path_buf()
}

#[cfg(all(not(windows), not(unix)))]
fn candidate_hdf5_path(path: &Path, _file: &fs::File) -> PathBuf {
    path.to_path_buf()
}

fn sha256_bytes(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn temp_nonce() -> String {
    let counter = NEXT_TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}-{nanos}-{counter}", std::process::id())
}

#[cfg(all(
    test,
    feature = "nisar-subset-production",
    any(target_os = "linux", target_os = "android", target_os = "macos")
))]
fn replace_snapshot_path_during_hdf5_read_for_test(
    source_path: &Path,
    detached_snapshot_path: &Path,
    descriptor_path: &Path,
) -> Result<(), NisarSubsetWriteError> {
    let hdf5 = crate::hdf5_file::open(descriptor_path)
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    let root = hdf5
        .group("/")
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    root.member_names()
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    if output_entry_kind(detached_snapshot_path)? != OutputEntryKind::Missing {
        return Err(NisarSubsetWriteError::IncompleteScienceAuthority);
    }
    let displaced_snapshot = detached_snapshot_path.with_extension("attacker-displaced.h5");
    match fs::rename(detached_snapshot_path, &displaced_snapshot) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(()) | Err(_) => return Err(NisarSubsetWriteError::IncompleteScienceAuthority),
    }
    fs::copy(source_path, detached_snapshot_path)
        .and_then(|_| OpenOptions::new().read(true).open(detached_snapshot_path))
        .and_then(|file| file.sync_all())
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    let replacement = crate::hdf5_file::open_rw(detached_snapshot_path)
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    let science = replacement
        .dataset("/science/LSAR/RSLC/swaths/frequencyA/HH")
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    let mut values = science
        .read_raw::<CFloat32>()
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    if values.is_empty() {
        return Err(NisarSubsetWriteError::IncompleteScienceAuthority);
    }
    for value in &mut values {
        value.r += 10_000.0;
    }
    science
        .write_raw(&values)
        .and_then(|_| replacement.flush())
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    drop(replacement);
    root.member_names()
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    drop(hdf5);
    Ok(())
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn mutate_source_during_hdf5_read_and_restore_for_test(
    source_path: &Path,
    immutable_view_path: &Path,
    material_offset: u64,
) -> Result<Result<(), Option<i32>>, NisarSubsetWriteError> {
    use std::sync::mpsc;

    let hdf5 = crate::hdf5_file::open(immutable_view_path)
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    let root = hdf5
        .group("/")
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    root.member_names()
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    let path = source_path.to_path_buf();
    let (ready_tx, ready_rx) = mpsc::sync_channel(0);
    let (mutate_tx, mutate_rx) = mpsc::sync_channel(0);
    let (mutated_tx, mutated_rx) = mpsc::sync_channel(0);
    let (restore_tx, restore_rx) = mpsc::sync_channel(0);
    let attacker = std::thread::spawn(move || {
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .and_then(|mut file| {
                file.seek(SeekFrom::Start(material_offset))?;
                let mut original = [0_u8; 1];
                file.read_exact(&mut original)?;
                ready_tx.send(true).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "ready receiver closed")
                })?;
                mutate_rx.recv().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "mutate sender closed")
                })?;
                file.seek(SeekFrom::Start(material_offset))?;
                file.write_all(&[original[0] ^ 0x5a])?;
                file.sync_all()?;
                mutated_tx.send(()).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "mutated receiver closed")
                })?;
                restore_rx.recv().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "restore sender closed")
                })?;
                file.seek(SeekFrom::Start(material_offset))?;
                file.write_all(&original)?;
                file.sync_all()
            });
        if opened.is_err() {
            let _ = ready_tx.send(false);
        }
        opened.map_err(|error| error.raw_os_error())
    });
    let can_mutate = ready_rx
        .recv()
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    if can_mutate {
        mutate_tx
            .send(())
            .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
        mutated_rx
            .recv()
            .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
        root.member_names()
            .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
        restore_tx
            .send(())
            .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    }
    let mutation = attacker
        .join()
        .map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)?;
    drop(hdf5);
    Ok(mutation)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn mutate_candidate_after_semantic_before_hash_for_test(
    path: &Path,
) -> Result<bool, NisarSubsetWriteError> {
    use std::sync::mpsc;

    let path = path.to_path_buf();
    let (ready_tx, ready_rx) = mpsc::sync_channel(0);
    let (go_tx, go_rx) = mpsc::sync_channel(0);
    let attacker = std::thread::spawn(move || {
        let attempted = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .and_then(|mut file| {
                let mut original = [0_u8; 1];
                file.read_exact(&mut original)?;
                ready_tx.send(()).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "ready receiver closed")
                })?;
                go_rx.recv().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "go sender closed")
                })?;
                file.seek(SeekFrom::Start(0))?;
                file.write_all(&[original[0] ^ 0x5a])?;
                file.sync_all()?;
                file.seek(SeekFrom::Start(0))?;
                file.write_all(&original)?;
                file.sync_all()
            });
        if attempted.is_err() {
            let _ = ready_tx.send(());
        }
        attempted.is_ok()
    });
    ready_rx
        .recv()
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)?;
    let _ = go_tx.send(());
    attacker
        .join()
        .map_err(|_| NisarSubsetWriteError::PostWriteScienceValidation)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn replace_published_artifact_after_open_for_test(
    artifact_path: &Path,
    installed: &V2CandidateFileGuard,
) -> Result<bool, NisarSubsetWriteError> {
    let parent = artifact_path
        .parent()
        .ok_or(NisarSubsetWriteError::OutputUnsafe)?;
    let nonce = temp_nonce();
    let replacement = parent.join(format!(".writer-v2-published-replacement-{nonce}"));
    let displaced = parent.join(format!(".writer-v2-published-displaced-{nonce}"));
    fs::copy(installed.hdf5_path(), &replacement)
        .map_err(|_| NisarSubsetWriteError::Persistence)?;
    // On Unix fs::copy carries the sealed artifact's read-only mode; the
    // attacker's own replacement stays writable so it can be synced.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600))
            .map_err(|_| NisarSubsetWriteError::Persistence)?;
    }
    sync_candidate_path(&replacement)?;
    if fs::rename(artifact_path, &displaced).is_err() {
        let _ = fs::remove_file(&replacement);
        return Ok(false);
    }
    if fs::rename(&replacement, artifact_path).is_err() {
        let _ = fs::rename(&displaced, artifact_path);
        let _ = fs::remove_file(&replacement);
        return Err(NisarSubsetWriteError::Persistence);
    }
    let replacement_file = open_candidate_file(artifact_path)?;
    if candidate_file_identity(&replacement_file)?.same_object(&installed.identity) {
        return Err(NisarSubsetWriteError::InjectedFailure);
    }
    Ok(true)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn attack_v2_part_for_test(
    path: &Path,
    failpoint: WriterFailpointV2,
    material_offset: u64,
) -> Result<(), NisarSubsetWriteError> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(NisarSubsetWriteError::IncompleteScienceAuthority)?;
    let sibling = path.with_file_name(format!(".{file_name}-writer-attack"));
    let attack =
        match failpoint {
            WriterFailpointV2::ReplacePartBeforeMaterialCapture => fs::rename(path, &sibling)
                .and_then(|_| {
                    let length = fs::metadata(&sibling)?.len();
                    let replacement = OpenOptions::new()
                        .create_new(true)
                        .read(true)
                        .write(true)
                        .open(path)?;
                    replacement.set_len(length)?;
                    replacement.sync_all()
                }),
            WriterFailpointV2::SymlinkPartBeforeMaterialCapture => fs::rename(path, &sibling)
                .and_then(|_| create_part_symlink_for_test(&sibling, path)),
            WriterFailpointV2::HardlinkPartBeforeMaterialCapture => fs::hard_link(path, &sibling),
            WriterFailpointV2::TruncatePartBeforeMaterialCapture => {
                OpenOptions::new().write(true).open(path).and_then(|part| {
                    let length = part.metadata()?.len();
                    part.set_len(length.saturating_sub(1))?;
                    part.sync_all()
                })
            }
            WriterFailpointV2::TruncateRestorePartBeforeMaterialCapture => {
                OpenOptions::new().write(true).open(path).and_then(|part| {
                    let before = part.metadata()?;
                    part.set_len(material_offset)?;
                    part.set_len(before.len())?;
                    part.sync_all()?;
                    part.set_times(restored_times_for_test(&before)?)
                })
            }
            WriterFailpointV2::WritePartBeforeMaterialCapture
            | WriterFailpointV2::WritePartImmediatelyBeforePromotion
            | WriterFailpointV2::WritePartAfterCandidateValidation => OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .and_then(|mut part| {
                    part.seek(SeekFrom::Start(material_offset))?;
                    let mut byte = [0_u8; 1];
                    part.read_exact(&mut byte)?;
                    byte[0] ^= 0x5a;
                    part.seek(SeekFrom::Start(material_offset))?;
                    part.write_all(&byte)?;
                    part.sync_all()
                }),
            _ => return Err(NisarSubsetWriteError::InjectedFailure),
        };
    record_part_attack_outcome_for_test(part_attack_outcome_for_test(
        attack
            .as_ref()
            .map(|_| ())
            .map_err(std::io::Error::raw_os_error),
    ));
    attack.map_err(|_| NisarSubsetWriteError::IncompleteScienceAuthority)
}

/// Classifies a test attack (downloader enhancement D1c, review P3): only a
/// sharing or lock violation is the OS refusing it because the writer pins
/// the part; any other error is reported with its code, so a test cannot take
/// an unrelated failure (a missing path, a permission) for the pin.
#[cfg(all(test, feature = "nisar-subset-production"))]
fn part_attack_outcome_for_test(attack: Result<(), Option<i32>>) -> PartAttackOutcomeForTest {
    // ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION.
    const SHARING_VIOLATIONS: [i32; 2] = [32, 33];
    match attack {
        Ok(()) => PartAttackOutcomeForTest::Applied,
        Err(Some(code)) if cfg!(windows) && SHARING_VIOLATIONS.contains(&code) => {
            PartAttackOutcomeForTest::RefusedByOs
        }
        Err(code) => PartAttackOutcomeForTest::FailedOtherwise(code),
    }
}

/// What a test attack on the pinned part did (review of D1/D2, downloader
/// enhancement D1b, P2-6). The writer refuses both with the same code; a test
/// tells them apart: the OS refused the attack because the writer pins the
/// part (Windows denies write and delete sharing), or the attack was applied
/// and an identity or digest check of the writer refused the part afterwards.
#[cfg(all(test, feature = "nisar-subset-production"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartAttackOutcomeForTest {
    RefusedByOs,
    Applied,
    /// The attack failed for another reason, with its OS error code.
    FailedOtherwise(Option<i32>),
}

#[cfg(all(test, feature = "nisar-subset-production"))]
thread_local! {
    static LAST_PART_ATTACK_FOR_TEST: std::cell::Cell<Option<PartAttackOutcomeForTest>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn record_part_attack_outcome_for_test(outcome: PartAttackOutcomeForTest) {
    LAST_PART_ATTACK_FOR_TEST.with(|slot| slot.set(Some(outcome)));
}

/// The outcome of the last part attack on this thread, cleared by reading it.
#[cfg(all(test, feature = "nisar-subset-production"))]
pub(crate) fn take_last_part_attack_outcome_for_test() -> Option<PartAttackOutcomeForTest> {
    LAST_PART_ATTACK_FOR_TEST.with(std::cell::Cell::take)
}

/// The modified, accessed (and on Windows created) times of `metadata`, to put
/// back after an attack.
#[cfg(all(test, feature = "nisar-subset-production"))]
fn restored_times_for_test(metadata: &std::fs::Metadata) -> std::io::Result<std::fs::FileTimes> {
    let times = std::fs::FileTimes::new()
        .set_modified(metadata.modified()?)
        .set_accessed(metadata.accessed()?);
    #[cfg(windows)]
    let times = {
        use std::os::windows::fs::FileTimesExt;
        times.set_created(metadata.created()?)
    };
    Ok(times)
}

#[cfg(all(test, feature = "nisar-subset-production", windows))]
fn create_part_symlink_for_test(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(all(test, feature = "nisar-subset-production", unix))]
fn create_part_symlink_for_test(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(all(test, feature = "nisar-subset-production", not(any(windows, unix))))]
fn create_part_symlink_for_test(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "symlink attack unsupported on this target",
    ))
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn is_v2_preseal_mutation_failpoint(failpoint: WriterFailpointV2) -> bool {
    matches!(
        failpoint,
        WriterFailpointV2::CorruptScienceBeforeValidation
            | WriterFailpointV2::CorruptScienceShapeBeforeValidation
            | WriterFailpointV2::CorruptLastScienceSampleBeforeValidation
            | WriterFailpointV2::CorruptLastMaskSampleBeforeValidation
            | WriterFailpointV2::CorruptCoordinateBeforeValidation
            | WriterFailpointV2::CorruptScaleBeforeValidation
            | WriterFailpointV2::CorruptMaskBeforeValidation
            | WriterFailpointV2::CorruptProvenanceBeforeValidation
            | WriterFailpointV2::CorruptQualityBeforeValidation
            | WriterFailpointV2::CorruptAuthorityBeforeValidation
            | WriterFailpointV2::CorruptMetadataBeforeValidation
            | WriterFailpointV2::AddUnselectedSourcePayloadBeforeValidation
            | WriterFailpointV2::AddDuplicateAncillaryBeforeValidation
            | WriterFailpointV2::AddExtraGroupBeforeValidation
    )
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn mutate_v2_candidate_before_seal_for_test(
    path: &Path,
    materials: &[CapturedDerivedWindowMaterialV2],
    failpoint: WriterFailpointV2,
) -> Result<(), NisarSubsetWriteError> {
    match failpoint {
        WriterFailpointV2::CorruptScienceBeforeValidation => {
            corrupt_v2_science_for_test(path, materials)
        }
        WriterFailpointV2::CorruptScienceShapeBeforeValidation => {
            corrupt_v2_science_shape_for_test(path, materials)
        }
        WriterFailpointV2::CorruptLastScienceSampleBeforeValidation => {
            corrupt_v2_last_sample_for_test::<CFloat32>(path, materials, |material| {
                material.science_path.clone()
            })
        }
        WriterFailpointV2::CorruptLastMaskSampleBeforeValidation => {
            corrupt_v2_last_sample_for_test::<u8>(path, materials, |material| {
                format!("{}/{}", material.root_path, material.mask_name)
            })
        }
        WriterFailpointV2::CorruptCoordinateBeforeValidation => {
            corrupt_v2_coordinate_for_test(path, materials)
        }
        WriterFailpointV2::CorruptScaleBeforeValidation => {
            corrupt_v2_scale_for_test(path, materials)
        }
        WriterFailpointV2::CorruptMaskBeforeValidation => corrupt_v2_mask_for_test(path, materials),
        WriterFailpointV2::CorruptProvenanceBeforeValidation => {
            corrupt_v2_provenance_for_test(path)
        }
        WriterFailpointV2::CorruptQualityBeforeValidation => {
            corrupt_v2_identification_for_test(path, "qualityStatus", "COMPLETE")
        }
        WriterFailpointV2::CorruptAuthorityBeforeValidation => {
            corrupt_v2_identification_for_test(path, "outputAuthority", "provider")
        }
        WriterFailpointV2::CorruptMetadataBeforeValidation => corrupt_metadata_for_test(path),
        WriterFailpointV2::AddUnselectedSourcePayloadBeforeValidation => {
            add_v2_unselected_source_payload_for_test(path)
        }
        WriterFailpointV2::AddDuplicateAncillaryBeforeValidation => {
            add_v2_duplicate_ancillary_for_test(path, materials)
        }
        WriterFailpointV2::AddExtraGroupBeforeValidation => add_v2_extra_group_for_test(path),
        _ => Ok(()),
    }
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn add_v2_unselected_source_payload_for_test(path: &Path) -> Result<(), NisarSubsetWriteError> {
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let group = file
        .group("/science/LSAR/RSLC/swaths/frequencyA")
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    group
        .new_dataset::<u8>()
        .shape(1)
        .create("VV")
        .and_then(|dataset| dataset.write_raw(&[1_u8]))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn add_v2_duplicate_ancillary_for_test(
    path: &Path,
    materials: &[CapturedDerivedWindowMaterialV2],
) -> Result<(), NisarSubsetWriteError> {
    let material = materials
        .first()
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let group = file
        .group(&material.root_path)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    group
        .new_dataset::<f64>()
        .shape(material.line_axis.len())
        .create("duplicateZeroDopplerTime")
        .and_then(|dataset| dataset.write_raw(&material.line_axis))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn add_v2_extra_group_for_test(path: &Path) -> Result<(), NisarSubsetWriteError> {
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.create_group("/coherentUnexpectedGroup")
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn corrupt_v2_science_for_test(
    path: &Path,
    materials: &[CapturedDerivedWindowMaterialV2],
) -> Result<(), NisarSubsetWriteError> {
    let material = materials
        .first()
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let dataset = file
        .dataset(&material.science_path)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let mut values = dataset
        .read_raw::<CFloat32>()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    values[0].r += 1.0;
    dataset
        .write_raw(&values)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn corrupt_v2_science_shape_for_test(
    path: &Path,
    materials: &[CapturedDerivedWindowMaterialV2],
) -> Result<(), NisarSubsetWriteError> {
    let material = materials
        .first()
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let (parent, name) = material
        .science_path
        .rsplit_once('/')
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let original = file
        .dataset(&material.science_path)
        .and_then(|dataset| dataset.read_raw::<CFloat32>())
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.unlink(&material.science_path)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.group(parent)
        .and_then(|group| {
            group
                .new_dataset::<CFloat32>()
                .shape((1, original.len()))
                .create(name)
        })
        .and_then(|dataset| dataset.write_raw(&original))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

/// Flips one bit of the last sample of the first window, which a streamed
/// validation reaches only in its last tile.
#[cfg(all(test, feature = "nisar-subset-production"))]
trait V2LastSampleCorruptionForTest: V2StreamElement {
    fn flip_one_bit(&mut self);
}

#[cfg(all(test, feature = "nisar-subset-production"))]
impl V2LastSampleCorruptionForTest for CFloat32 {
    fn flip_one_bit(&mut self) {
        self.i = f32::from_bits(self.i.to_bits() ^ 1);
    }
}

#[cfg(all(test, feature = "nisar-subset-production"))]
impl V2LastSampleCorruptionForTest for u8 {
    fn flip_one_bit(&mut self) {
        *self ^= 1;
    }
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn corrupt_v2_last_sample_for_test<T: V2LastSampleCorruptionForTest>(
    path: &Path,
    materials: &[CapturedDerivedWindowMaterialV2],
    dataset_path: impl Fn(&CapturedDerivedWindowMaterialV2) -> String,
) -> Result<(), NisarSubsetWriteError> {
    let material = materials
        .first()
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let rows =
        usize::try_from(material.shape[0]).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let cols =
        usize::try_from(material.shape[1]).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let dataset = file
        .dataset(&dataset_path(material))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let selection = (rows - 1..rows, cols - 1..cols);
    let mut value = dataset
        .read_slice_2d::<T, _>(selection.clone())
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    value[[0, 0]].flip_one_bit();
    dataset
        .write_slice(&value, selection)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn corrupt_v2_coordinate_for_test(
    path: &Path,
    materials: &[CapturedDerivedWindowMaterialV2],
) -> Result<(), NisarSubsetWriteError> {
    let material = materials
        .first()
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let dataset = file
        .dataset(&format!(
            "{}/{}",
            material.root_path, material.line_scale_name
        ))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let mut values = dataset
        .read_raw::<f64>()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    values[0] += 1.0;
    dataset
        .write_raw(&values)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn corrupt_v2_scale_for_test(
    path: &Path,
    materials: &[CapturedDerivedWindowMaterialV2],
) -> Result<(), NisarSubsetWriteError> {
    let material = materials
        .first()
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let scale_path = format!("{}/{}", material.root_path, material.line_scale_name);
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.unlink(&scale_path)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn corrupt_v2_mask_for_test(
    path: &Path,
    materials: &[CapturedDerivedWindowMaterialV2],
) -> Result<(), NisarSubsetWriteError> {
    let material = materials
        .first()
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let dataset = file
        .dataset(&format!("{}/{}", material.root_path, material.mask_name))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let mut values = dataset
        .read_raw::<u8>()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    values[0] ^= 1;
    dataset
        .write_raw(&values)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn corrupt_v2_provenance_for_test(path: &Path) -> Result<(), NisarSubsetWriteError> {
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let dataset = file
        .dataset(&format!("{PROVENANCE_PATH}/writerSelectionSha256"))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    overwrite_fixed_ascii_for_test(&dataset, "0")?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(all(test, feature = "nisar-subset-production"))]
fn corrupt_v2_identification_for_test(
    path: &Path,
    name: &str,
    value: &str,
) -> Result<(), NisarSubsetWriteError> {
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let dataset = file
        .dataset(&format!("{IDENTIFICATION_PATH}/{name}"))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    overwrite_fixed_ascii_for_test(&dataset, value)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(test)]
fn overwrite_fixed_ascii_for_test(
    dataset: &Dataset,
    value: &str,
) -> Result<(), NisarSubsetWriteError> {
    let descriptor = dataset
        .dtype()
        .and_then(|datatype| datatype.to_descriptor())
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let TypeDescriptor::FixedAscii(capacity) = descriptor else {
        return Err(NisarSubsetWriteError::InjectedFailure);
    };
    if value.len() > capacity {
        return Err(NisarSubsetWriteError::InjectedFailure);
    }
    let mut bytes = vec![0_u8; capacity];
    bytes[..value.len()].copy_from_slice(value.as_bytes());
    let datatype = dataset
        .dtype()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let status = hdf5::sync::sync(|| unsafe {
        H5Dwrite(
            dataset.id(),
            datatype.id(),
            H5S_ALL,
            H5S_ALL,
            H5P_DEFAULT,
            bytes.as_ptr().cast(),
        )
    });
    if status < 0 {
        return Err(NisarSubsetWriteError::InjectedFailure);
    }
    Ok(())
}

#[cfg(test)]
fn corrupt_science_for_test(
    path: &Path,
    materials: &[DerivedWindowMaterial],
) -> Result<(), NisarSubsetWriteError> {
    let material = materials
        .first()
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let dataset = file
        .dataset(&material.science_path)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let mut values = dataset
        .read_raw::<CFloat32>()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    values[0].r += 1.0;
    dataset
        .write_raw(&values)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(test)]
fn remove_mask_for_test(
    path: &Path,
    materials: &[DerivedWindowMaterial],
) -> Result<(), NisarSubsetWriteError> {
    let mask_path = materials
        .iter()
        .find_map(|material| material.mask_path.as_deref())
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.unlink(mask_path)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(test)]
fn corrupt_metadata_for_test(path: &Path) -> Result<(), NisarSubsetWriteError> {
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let dataset = file
        .dataset(&format!("{ORBIT_PATH}/time"))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let mut values = dataset
        .read_raw::<f64>()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    values[0] += 1.0;
    dataset
        .write_raw(&values)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(test)]
fn corrupt_identification_for_test(path: &Path) -> Result<(), NisarSubsetWriteError> {
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let dataset = file
        .dataset(&format!("{IDENTIFICATION_PATH}/granuleId"))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let descriptor = dataset
        .dtype()
        .and_then(|datatype| datatype.to_descriptor())
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let TypeDescriptor::FixedAscii(capacity) = descriptor else {
        return Err(NisarSubsetWriteError::InjectedFailure);
    };
    let stale = b"NISAR_L1_PR_RSLC_STALE_FULL_FRAME";
    if stale.len() > capacity {
        return Err(NisarSubsetWriteError::InjectedFailure);
    }
    let mut bytes = vec![0_u8; capacity];
    bytes[..stale.len()].copy_from_slice(stale);
    let datatype = dataset
        .dtype()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let status = hdf5::sync::sync(|| unsafe {
        H5Dwrite(
            dataset.id(),
            datatype.id(),
            H5S_ALL,
            H5S_ALL,
            H5P_DEFAULT,
            bytes.as_ptr().cast(),
        )
    });
    if status < 0 {
        return Err(NisarSubsetWriteError::InjectedFailure);
    }
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(test)]
fn add_unexpected_attribute_for_test(path: &Path) -> Result<(), NisarSubsetWriteError> {
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let root = file
        .group("/")
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    root.new_attr::<u8>()
        .create("unexpected")
        .and_then(|attribute| attribute.write_scalar(&1_u8))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(test)]
fn corrupt_science_shape_for_test(
    path: &Path,
    materials: &[DerivedWindowMaterial],
) -> Result<(), NisarSubsetWriteError> {
    let material = materials
        .first()
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let (parent, name) = material
        .science_path
        .rsplit_once('/')
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let original = file
        .dataset(&material.science_path)
        .and_then(|dataset| dataset.read_raw::<CFloat32>())
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.unlink(&material.science_path)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.group(parent)
        .and_then(|group| {
            group
                .new_dataset::<CFloat32>()
                .shape((1, original.len()))
                .create(name)
        })
        .and_then(|dataset| dataset.write_raw(&original))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[cfg(test)]
fn corrupt_mask_chunk_geometry_for_test(
    path: &Path,
    materials: &[DerivedWindowMaterial],
) -> Result<(), NisarSubsetWriteError> {
    let material = materials
        .iter()
        .find(|material| material.mask_path.is_some())
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let mask_path = material
        .mask_path
        .as_deref()
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let (parent, name) = mask_path
        .rsplit_once('/')
        .ok_or(NisarSubsetWriteError::InjectedFailure)?;
    let file =
        crate::hdf5_file::open_rw(path).map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let dataset = file
        .dataset(mask_path)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let values = dataset
        .read_raw::<u8>()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    let shape = dataset.shape();
    file.unlink(mask_path)
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.group(parent)
        .and_then(|group| {
            group
                .new_dataset::<u8>()
                .shape((shape[0], shape[1]))
                .chunk((shape[0].min(2), 1))
                .create(name)
        })
        .and_then(|dataset| dataset.write_raw(&values))
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)?;
    file.flush()
        .map_err(|_| NisarSubsetWriteError::InjectedFailure)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriterFailpoint {
    None,
    #[cfg(test)]
    AfterWriteBeforeValidation,
    #[cfg(test)]
    CorruptScienceBeforeValidation,
    #[cfg(test)]
    RemoveMaskBeforeValidation,
    #[cfg(test)]
    CorruptMetadataBeforeValidation,
    #[cfg(test)]
    CorruptIdentificationBeforeValidation,
    #[cfg(test)]
    AddUnexpectedAttributeBeforeValidation,
    #[cfg(test)]
    CorruptScienceShapeBeforeValidation,
    #[cfg(test)]
    CorruptMaskChunkGeometryBeforeValidation,
    #[cfg(test)]
    AfterArtifactPromotion,
    #[cfg(test)]
    ReplaceCandidateAfterValidation,
}

fn validate_metadata_source_identity(
    request: &NisarSubsetRequestManifestV1,
    source_validator: &NisarSubsetSourceValidatorV1,
    source: &CandidateFileGuard,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), NisarSubsetWriteError> {
    let checksum = request
        .source
        .checksum
        .as_ref()
        .filter(|checksum| checksum.algorithm == NisarSubsetChecksumAlgorithm::Sha256)
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    if source.identity.size != request.source.expected_size_bytes
        || source_validator.kind != NisarSubsetValidatorKind::StrongEtag
        || source_validator.value.starts_with("W/")
        || !source_validator.value.starts_with('"')
        || !source_validator.value.ends_with('"')
        || source_validator.value.len() < 3
        || source_validator.content_length != request.source.expected_size_bytes
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let actual_sha256 = source.sha256(cancelled).map_err(|error| match error {
        NisarSubsetWriteError::Cancelled => NisarSubsetWriteError::Cancelled,
        _ => NisarSubsetWriteError::MetadataInvalid,
    })?;
    if actual_sha256 != checksum.value {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
fn validate_live_metadata_source_identity(
    request: &NisarSubsetRequestManifestV1,
    source_validator: &NisarSubsetSourceValidatorV1,
    source: &CandidateFileGuard,
) -> Result<(), NisarSubsetWriteError> {
    if source.identity.size != request.source.expected_size_bytes
        || source_validator.kind != NisarSubsetValidatorKind::StrongEtag
        || source_validator.value.starts_with("W/")
        || !source_validator.value.starts_with('"')
        || !source_validator.value.ends_with('"')
        || source_validator.value.len() < 3
        || source_validator.content_length != request.source.expected_size_bytes
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok(())
}

fn read_source_metadata(
    selection: &SourceMetadataSelection,
    source_path: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<
    (
        NisarSubsetMetadataEvidenceV1,
        Vec<SourceGroupBindingV1>,
        Vec<SourceMetadataDatasetBindingV1>,
        usize,
    ),
    NisarSubsetWriteError,
> {
    metadata_capture_stage!("hdf5.open_read_only", source_path.display());
    let file = crate::hdf5_file::open(source_path).map_err(|error| {
        metadata_capture_detail!(error);
        NisarSubsetWriteError::MetadataInvalid
    })?;
    metadata_capture_stage!("hdf5.verify_read_only", source_path.display());
    if !file.is_read_only() {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    let mut budget = MetadataReadBudget::default();
    metadata_capture_stage!("inventory.collect", "/");
    let (group_bindings, dataset_bindings, source_dataset_count) =
        collect_source_inventory(&file, selection, cancelled, &mut budget)?;
    let identification_path = selection.science_root.identification_path();
    let orbit_path = selection.science_root.orbit_path();
    let attitude_path = selection.science_root.attitude_path();
    let processing_path = selection.science_root.processing_path();
    let calibration_path = selection.science_root.calibration_path();
    let geolocation_path = selection.science_root.geolocation_path();
    metadata_capture_stage!("identification.validate_group_link", &identification_path);
    validate_hard_link_path(&file, &identification_path)?;
    metadata_capture_stage!("identification.open_group", &identification_path);
    let identification = file.group(&identification_path).map_err(|error| {
        metadata_capture_detail!(error);
        NisarSubsetWriteError::MetadataInvalid
    })?;
    for name in [
        "missionId",
        "granuleId",
        "boundingPolygon",
        "zeroDopplerStartTime",
        "zeroDopplerEndTime",
    ] {
        let dataset = open_metadata_dataset(&file, &format!("{identification_path}/{name}"))?;
        validate_metadata_dataset_storage(&file, &dataset)?;
    }
    metadata_capture_stage!("identification.read_scalar_fields", &identification_path);
    budget.charge(32 + 256 + 4096)?;
    let mission_id = read_ascii_with_capacity(&identification, "missionId", 32)?;
    let source_granule_id = read_ascii_with_capacity(&identification, "granuleId", 256)?;
    let bounding_polygon = read_ascii_with_capacity(&identification, "boundingPolygon", 4096)?;
    let source_zero_doppler_start_time = read_fixed_ascii_dataset(
        &identification
            .dataset("zeroDopplerStartTime")
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
    )?;
    let source_zero_doppler_end_time = read_fixed_ascii_dataset(
        &identification
            .dataset("zeroDopplerEndTime")
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
    )?;
    let expected_granule_id = selection
        .expected_file_name
        .strip_suffix(".h5")
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    metadata_capture_stage!(
        "identification.validate_identity_fields",
        &identification_path
    );
    for (name, expected) in [
        ("missionId", "NISAR"),
        ("granuleId", expected_granule_id),
        ("productType", "RSLC"),
        ("productLevel", "L1"),
        ("radarBand", selection.science_root.radar_band()),
        (
            "productSpecificationVersion",
            selection.product_specification_version.as_str(),
        ),
    ] {
        metadata_capture_stage!(
            "identification.validate_identity_field",
            &format!("{identification_path}/{name}")
        );
        let actual = read_fixed_ascii_dataset(
            &identification
                .dataset(name)
                .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
        )?;
        if actual != expected {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
    }
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }

    metadata_capture_stage!("orbit.read", &orbit_path);
    let (orbit_time, orbit_time_shape) =
        read_f64_dataset(&file, &format!("{orbit_path}/time"), &mut budget, cancelled)?;
    let (orbit_position_flat, orbit_position_shape) = read_f64_dataset(
        &file,
        &format!("{orbit_path}/position"),
        &mut budget,
        cancelled,
    )?;
    let (orbit_velocity_flat, orbit_velocity_shape) = read_f64_dataset(
        &file,
        &format!("{orbit_path}/velocity"),
        &mut budget,
        cancelled,
    )?;
    metadata_capture_stage!("attitude.read", &attitude_path);
    let (attitude_time, attitude_time_shape) = read_f64_dataset(
        &file,
        &format!("{attitude_path}/time"),
        &mut budget,
        cancelled,
    )?;
    let (attitude_flat, attitude_shape) = read_f64_dataset(
        &file,
        &format!("{attitude_path}/quaternions"),
        &mut budget,
        cancelled,
    )?;
    metadata_capture_stage!("processing.read", &processing_path);
    let (doppler_lut, doppler_shape) = read_f32_dataset(
        &file,
        &format!(
            "{processing_path}/parameters/frequency{}/dopplerCentroid",
            frequency_label(&selection.primary_frequency)
        ),
        &mut budget,
        cancelled,
    )?;
    let (calibration_lut, calibration_shape) = read_f32_dataset(
        &file,
        &format!("{calibration_path}/geometry/beta0"),
        &mut budget,
        cancelled,
    )?;
    metadata_capture_stage!("geolocation.read", &geolocation_path);
    let (geolocation_x, geolocation_shape) = read_f64_dataset_with_ranks(
        &file,
        &format!("{geolocation_path}/coordinateX"),
        &[3],
        &mut budget,
        cancelled,
    )?;
    let (geolocation_y, geolocation_y_shape) = read_f64_dataset_with_ranks(
        &file,
        &format!("{geolocation_path}/coordinateY"),
        &[3],
        &mut budget,
        cancelled,
    )?;
    let (height_above_ellipsoid, height_shape) = read_f64_dataset(
        &file,
        &format!("{geolocation_path}/heightAboveEllipsoid"),
        &mut budget,
        cancelled,
    )?;
    let (zero_doppler_time, zero_doppler_shape) = read_f64_dataset(
        &file,
        &selection.science_root.zero_doppler_time_path(),
        &mut budget,
        cancelled,
    )?;
    let mut slant_ranges = BTreeMap::new();
    let mut valid_samples = BTreeMap::new();
    for frequency in &selection.selected_frequency_labels {
        let slant_path = selection.science_root.slant_range_path(frequency);
        let (slant_range, slant_range_shape) =
            read_f64_dataset(&file, &slant_path, &mut budget, cancelled)?;
        if slant_range_shape.len() != 1 {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        slant_ranges.insert(frequency.clone(), slant_range);
        let valid_path = format!(
            "{}/frequency{frequency}/validSamplesSubSwath1",
            selection.science_root.swaths_path()
        );
        let (values, shape) = read_u32_dataset(&file, &valid_path, &mut budget, cancelled)?;
        if shape.len() != 2 || shape[1] != 2 || values.len() != shape[0].saturating_mul(2) {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        valid_samples.insert(
            frequency.clone(),
            values
                .chunks_exact(2)
                .map(|pair| [pair[0], pair[1]])
                .collect(),
        );
    }
    let epsg_dataset = open_metadata_dataset(&file, &format!("{geolocation_path}/epsg"))?;
    charge_numeric_dataset::<i32>(&file, &epsg_dataset, &[0], &mut budget, cancelled)?;
    let geolocation_epsg = epsg_dataset
        .read_scalar::<i32>()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;

    if orbit_time_shape.len() != 1
        || orbit_position_shape != [orbit_time.len(), 3]
        || orbit_velocity_shape != [orbit_time.len(), 3]
        || attitude_time_shape.len() != 1
        || attitude_shape != [attitude_time.len(), 4]
        || doppler_shape.len() != 2
        || calibration_shape.len() != 2
        || geolocation_shape.len() != 3
        || geolocation_y_shape != geolocation_shape
        || height_shape != [geolocation_shape[0]]
        || geolocation_shape[0] < 3
        || zero_doppler_shape.len() != 1
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }

    let orbit_position = triples(&orbit_position_flat)?;
    let orbit_velocity = triples(&orbit_velocity_flat)?;
    let attitude_quaternions = quads(&attitude_flat)?;
    let evidence = NisarSubsetMetadataEvidenceV1 {
        mission_id,
        source_granule_id,
        bounding_polygon,
        source_zero_doppler_start_time,
        source_zero_doppler_end_time,
        selected_frequency: frequency_label(&selection.primary_frequency).to_owned(),
        orbit_time,
        orbit_position,
        orbit_velocity,
        attitude_time,
        attitude_quaternions,
        doppler_shape: [doppler_shape[0], doppler_shape[1]],
        doppler_lut,
        calibration_shape: [calibration_shape[0], calibration_shape[1]],
        calibration_lut,
        geolocation_shape: [
            geolocation_shape[0],
            geolocation_shape[1],
            geolocation_shape[2],
        ],
        geolocation_x,
        geolocation_y,
        height_above_ellipsoid,
        geolocation_epsg,
        zero_doppler_time,
        slant_ranges,
        valid_samples,
    };
    Ok((
        evidence,
        group_bindings,
        dataset_bindings,
        source_dataset_count,
    ))
}

fn collect_source_inventory(
    file: &File,
    selection: &SourceMetadataSelection,
    cancelled: &(dyn Fn() -> bool + Sync),
    budget: &mut MetadataReadBudget,
) -> Result<
    (
        Vec<SourceGroupBindingV1>,
        Vec<SourceMetadataDatasetBindingV1>,
        usize,
    ),
    NisarSubsetWriteError,
> {
    metadata_capture_stage!("inventory.open_root", "/");
    let root = file.group("/").map_err(|error| {
        metadata_capture_detail!(error);
        NisarSubsetWriteError::MetadataInvalid
    })?;
    let mut groups = Vec::new();
    let mut datasets = Vec::new();
    let mut source_dataset_count = 0_usize;
    collect_source_group(
        file,
        selection,
        &root,
        "/",
        cancelled,
        budget,
        &mut groups,
        &mut datasets,
        &mut source_dataset_count,
    )?;
    groups.sort_by(|left, right| left.path.cmp(&right.path));
    datasets.sort_by(|left, right| left.path.cmp(&right.path));
    let by_path = datasets
        .iter()
        .map(|binding| (binding.path.as_str(), binding))
        .collect::<BTreeMap<_, _>>();
    for binding in &datasets {
        if binding.dimension_labels.len() != binding.shape.len()
            || binding.dimension_scales.len() != binding.shape.len()
        {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        for (dimension, scales) in binding.dimension_scales.iter().enumerate() {
            for scale_path in scales {
                let scale = by_path
                    .get(scale_path.as_str())
                    .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
                // A scale must be one-dimensional and exactly match the
                // attached dimension length. A missing scale NAME is legal
                // provider semantics (real NISAR geolocation-grid scales carry
                // no NAME) and does not weaken the shape/length binding.
                if scale.shape.len() != 1 || scale.shape[0] != binding.shape[dimension] {
                    return Err(NisarSubsetWriteError::MetadataInvalid);
                }
            }
        }
    }
    let paths = datasets
        .iter()
        .map(|binding| binding.path.as_str())
        .collect::<BTreeSet<_>>();
    let identification_path = selection.science_root.identification_path();
    let orbit_path = selection.science_root.orbit_path();
    let attitude_path = selection.science_root.attitude_path();
    let processing_path = selection.science_root.processing_path();
    let calibration_path = selection.science_root.calibration_path();
    let geolocation_path = selection.science_root.geolocation_path();
    let swaths_path = selection.science_root.swaths_path();
    for required in [
        format!("{identification_path}/missionId"),
        format!("{identification_path}/granuleId"),
        format!("{identification_path}/boundingPolygon"),
        format!("{identification_path}/isFullFrame"),
        format!("{identification_path}/zeroDopplerStartTime"),
        format!("{identification_path}/zeroDopplerEndTime"),
        format!("{orbit_path}/time"),
        format!("{orbit_path}/position"),
        format!("{orbit_path}/velocity"),
        format!("{attitude_path}/time"),
        format!("{attitude_path}/quaternions"),
        format!(
            "{processing_path}/parameters/frequency{}/dopplerCentroid",
            frequency_label(&selection.primary_frequency)
        ),
        format!("{calibration_path}/geometry/beta0"),
        format!("{geolocation_path}/epsg"),
        format!("{geolocation_path}/coordinateX"),
        format!("{geolocation_path}/coordinateY"),
        format!("{geolocation_path}/heightAboveEllipsoid"),
        format!("{geolocation_path}/zeroDopplerTime"),
        format!("{geolocation_path}/slantRange"),
        format!("{swaths_path}/zeroDopplerTime"),
    ] {
        if !paths.contains(required.as_str()) {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
    }
    for science in &selection.selected_science {
        if !paths.contains(science.source_path.as_str()) {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
    }
    for frequency in &selection.selected_frequency_labels {
        for required in [
            format!("{swaths_path}/frequency{frequency}/slantRange"),
            format!("{swaths_path}/frequency{frequency}/validSamplesSubSwath1"),
        ] {
            if !paths.contains(required.as_str()) {
                return Err(NisarSubsetWriteError::MetadataInvalid);
            }
        }
    }
    for mask_path in &selection.required_mask_paths {
        if !paths.contains(mask_path.as_str()) {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
    }
    groups.retain(|group| {
        group.path == "/"
            || datasets.iter().any(|dataset| {
                dataset.path.starts_with(&group.path)
                    && dataset.path.as_bytes().get(group.path.len()) == Some(&b'/')
            })
    });
    Ok((groups, datasets, source_dataset_count))
}

#[allow(clippy::too_many_arguments)]
fn collect_source_group(
    file: &File,
    selection: &SourceMetadataSelection,
    group: &Group,
    path: &str,
    cancelled: &(dyn Fn() -> bool + Sync),
    budget: &mut MetadataReadBudget,
    groups: &mut Vec<SourceGroupBindingV1>,
    datasets: &mut Vec<SourceMetadataDatasetBindingV1>,
    source_dataset_count: &mut usize,
) -> Result<(), NisarSubsetWriteError> {
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    if groups.len() >= 512 || datasets.len() > MAX_SUPPORTED_SOURCE_DATASET_COUNT {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    metadata_capture_stage!("inventory.group.attributes", path);
    let attributes = read_attribute_bindings(group)?;
    metadata_capture_stage!("inventory.group.attribute_budget", path);
    charge_attribute_bindings(budget, &attributes)?;
    groups.push(SourceGroupBindingV1 {
        path: path.to_owned(),
        attributes,
    });
    metadata_capture_stage!("inventory.group.member_names", path);
    let mut names = group.member_names().map_err(|error| {
        metadata_capture_detail!(error);
        NisarSubsetWriteError::MetadataInvalid
    })?;
    names.sort();
    for name in names {
        let child_path = if path == "/" {
            format!("/{name}")
        } else {
            format!("{path}/{name}")
        };
        metadata_capture_stage!("inventory.child.validate_hard_link", &child_path);
        validate_hard_link_path(file, &child_path)?;
        metadata_capture_stage!("inventory.child.loc_info", &child_path);
        let info = match group.loc_info_by_name(&name) {
            Ok(info) => info,
            Err(error) => {
                if source_named_datatype_exists(group, &name)? {
                    continue;
                }
                metadata_capture_detail!(error);
                return Err(NisarSubsetWriteError::MetadataInvalid);
            }
        };
        match info.loc_type {
            LocationType::Group => {
                metadata_capture_stage!("inventory.child.open_group", &child_path);
                let child = group.group(&name).map_err(|error| {
                    metadata_capture_detail!(error);
                    NisarSubsetWriteError::MetadataInvalid
                })?;
                collect_source_group(
                    file,
                    selection,
                    &child,
                    &child_path,
                    cancelled,
                    budget,
                    groups,
                    datasets,
                    source_dataset_count,
                )?;
            }
            LocationType::Dataset => {
                *source_dataset_count = source_dataset_count
                    .checked_add(1)
                    .filter(|count| *count <= MAX_SUPPORTED_SOURCE_DATASET_COUNT)
                    .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
                metadata_capture_stage!("inventory.child.open_dataset", &child_path);
                let dataset = group.dataset(&name).map_err(|error| {
                    metadata_capture_detail!(error);
                    NisarSubsetWriteError::MetadataInvalid
                })?;
                metadata_capture_stage!("inventory.child.bind_dataset", &child_path);
                if let Some(binding) =
                    bind_source_dataset(file, selection, &child_path, &dataset, cancelled, budget)?
                {
                    datasets.push(binding);
                }
            }
            LocationType::NamedDatatype => {
                if !source_named_datatype_exists(group, &name)? {
                    return Err(NisarSubsetWriteError::MetadataInvalid);
                }
            }
            _ => return Err(NisarSubsetWriteError::MetadataInvalid),
        }
    }
    Ok(())
}

fn source_named_datatype_exists(group: &Group, name: &str) -> Result<bool, NisarSubsetWriteError> {
    let name = CString::new(name).map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let datatype_id =
        hdf5::sync::sync(|| unsafe { H5Topen2(group.id(), name.as_ptr(), H5P_DEFAULT) });
    if datatype_id < 0 {
        return Ok(false);
    }
    let close_status = hdf5::sync::sync(|| unsafe { H5Tclose(datatype_id) });
    if close_status < 0 {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    Ok(true)
}

fn bind_source_dataset(
    file: &File,
    selection: &SourceMetadataSelection,
    path: &str,
    dataset: &Dataset,
    cancelled: &(dyn Fn() -> bool + Sync),
    budget: &mut MetadataReadBudget,
) -> Result<Option<SourceMetadataDatasetBindingV1>, NisarSubsetWriteError> {
    metadata_capture_stage!("inventory.dataset.validate_storage", path);
    validate_metadata_dataset_storage(file, dataset)?;
    metadata_capture_stage!("inventory.dataset.classify", path);
    let Some(action) = classify_source_dataset(selection, path)? else {
        return Ok(None);
    };
    metadata_capture_stage!("inventory.dataset.datatype", path);
    let datatype = dataset.dtype().map_err(|error| {
        metadata_capture_detail!(error);
        NisarSubsetWriteError::MetadataInvalid
    })?;
    metadata_capture_stage!("inventory.dataset.validate_datatype", path);
    validate_fixed_datatype(&datatype)?;
    let shape = dataset
        .shape()
        .into_iter()
        .map(|dimension| {
            u64::try_from(dimension).map_err(|_| NisarSubsetWriteError::MetadataInvalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if shape.len() > 8 || shape.contains(&0) {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    metadata_capture_stage!("inventory.dataset.dimension_graph", path);
    let (dimension_labels, dimension_scales, scale_name, is_dimension_scale) =
        read_dimension_scale_graph(dataset)?;
    metadata_capture_stage!("inventory.dataset.attributes", path);
    let attributes = read_attribute_bindings(dataset)?;
    metadata_capture_stage!("inventory.dataset.attribute_budget", path);
    charge_attribute_bindings(budget, &attributes)?;
    metadata_capture_stage!("inventory.dataset.value", path);
    let value = if action == SourceDatasetActionV1::PreserveExact {
        Some(Arc::<[u8]>::from(read_dataset_values_bounded(
            dataset, budget, cancelled,
        )?))
    } else {
        None
    };
    let value_sha256 = value.as_ref().map(|value| sha256_bytes(value));
    Ok(Some(SourceMetadataDatasetBindingV1 {
        path: path.to_owned(),
        datatype_sha256: {
            metadata_capture_stage!("inventory.dataset.datatype_hash", path);
            hdf5_datatype_sha256(&datatype).map_err(|_| NisarSubsetWriteError::MetadataInvalid)?
        },
        shape,
        chunk_shape: {
            metadata_capture_stage!("inventory.dataset.chunk_shape", path);
            dataset
                .dcpl()
                .and_then(|dcpl| dcpl.get_chunk())
                .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?
                .map(|shape| shape.into_iter().map(|value| value as u64).collect())
        },
        filters: {
            metadata_capture_stage!("inventory.dataset.filters", path);
            source_filter_bindings(dataset)?
        },
        attributes,
        dimension_labels,
        dimension_scales,
        scale_name,
        is_dimension_scale,
        action,
        value_sha256,
        datatype: {
            metadata_capture_stage!("inventory.dataset.datatype_descriptor", path);
            datatype
                .to_descriptor()
                .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?
        },
        datatype_encoded: {
            metadata_capture_stage!("inventory.dataset.datatype_encode", path);
            encode_datatype(&datatype)?
        },
        value,
    }))
}

fn classify_source_dataset(
    selection: &SourceMetadataSelection,
    path: &str,
) -> Result<Option<SourceDatasetActionV1>, NisarSubsetWriteError> {
    if selection
        .selected_science
        .iter()
        .any(|science| science.source_path == path)
    {
        return Ok(Some(SourceDatasetActionV1::SelectedScience));
    }
    if selection
        .mask_paths
        .iter()
        .any(|mask_path| mask_path == path)
    {
        return Ok(Some(SourceDatasetActionV1::FrequencyMask));
    }
    let swaths_path = selection.science_root.swaths_path();
    if path == format!("{swaths_path}/zeroDopplerTime") {
        return Ok(Some(SourceDatasetActionV1::ZeroDopplerTime));
    }
    for frequency in &selection.selected_frequency_labels {
        let root = format!("{swaths_path}/frequency{frequency}");
        if path == format!("{root}/slantRange") {
            return Ok(Some(SourceDatasetActionV1::FrequencySlantRange));
        }
        if path == format!("{root}/validSamplesSubSwath1") {
            return Ok(Some(SourceDatasetActionV1::FrequencyValidSamples));
        }
    }
    let frequency_prefix = format!("{swaths_path}/frequency");
    if let Some((frequency, _)) = path
        .strip_prefix(&frequency_prefix)
        .and_then(|value| value.split_once('/'))
    {
        if selection
            .selected_frequency_labels
            .iter()
            .any(|selected| selected == frequency)
            && is_allowed_swath_metadata_path(path, selection.science_root)
        {
            return Ok(Some(SourceDatasetActionV1::PreserveExact));
        }
        // The official source can contain multiple frequencies and
        // polarizations. A dataset-level Request emits only its selected
        // frequency science, axes and QA mask; unrelated full-resolution
        // source payload is intentionally omitted rather than rejected or
        // copied into the derived artifact.
        return Ok(None);
    }
    let identification_path = selection.science_root.identification_path();
    if [
        "boundingPolygon",
        "granuleId",
        "isFullFrame",
        "zeroDopplerStartTime",
        "zeroDopplerEndTime",
    ]
    .iter()
    .any(|name| path == format!("{identification_path}/{name}"))
    {
        return Ok(Some(SourceDatasetActionV1::DerivedIdentification));
    }
    let metadata_prefix = format!("{}/", selection.science_root.metadata_path());
    if path.starts_with(&metadata_prefix)
        || is_allowed_identification_path(path, selection.science_root)
        || is_allowed_swath_metadata_path(path, selection.science_root)
    {
        return Ok(Some(SourceDatasetActionV1::PreserveExact));
    }
    Err(NisarSubsetWriteError::MetadataInvalid)
}

fn is_allowed_identification_path(path: &str, science_root: NisarScienceRoot) -> bool {
    let prefix = format!("{}/", science_root.identification_path());
    let Some(name) = path.strip_prefix(&prefix) else {
        return false;
    };
    matches!(
        name,
        "absoluteOrbitNumber"
            | "compositeReleaseId"
            | "diagnosticModeFlag"
            | "frameNumber"
            | "hasInputDataException"
            | "instrumentName"
            | "isDithered"
            | "isGeocoded"
            | "isJointObservation"
            | "isMixedMode"
            | "isUrgentObservation"
            | "listOfFrequencies"
            | "lookDirection"
            | "missionId"
            | "orbitPassDirection"
            | "plannedDatatakeId"
            | "plannedObservationId"
            | "platformName"
            | "processingCenter"
            | "processingDateTime"
            | "processingType"
            | "productDoi"
            | "productLevel"
            | "productSpecificationVersion"
            | "productType"
            | "productVersion"
            | "radarBand"
            | "trackNumber"
    )
}

fn is_allowed_swath_metadata_path(path: &str, science_root: NisarScienceRoot) -> bool {
    let swaths_path = science_root.swaths_path();
    if path == format!("{swaths_path}/zeroDopplerTimeSpacing") {
        return true;
    }
    let frequency_prefix = format!("{swaths_path}/frequency");
    let Some((_, name)) = path
        .strip_prefix(&frequency_prefix)
        .and_then(|value| value.split_once('/'))
    else {
        return false;
    };
    matches!(
        name,
        "acquiredCenterFrequency"
            | "acquiredRangeBandwidth"
            | "listOfPolarizations"
            | "nominalAcquisitionPRF"
            | "numberOfSubSwaths"
            | "processedAzimuthBandwidth"
            | "processedCenterFrequency"
            | "processedRangeBandwidth"
            | "sceneCenterAlongTrackSpacing"
            | "sceneCenterGroundRangeSpacing"
            | "slantRangeSpacing"
    )
}

fn source_filter_bindings(
    dataset: &Dataset,
) -> Result<Vec<SourceFilterBindingV1>, NisarSubsetWriteError> {
    dataset
        .filters()
        .into_iter()
        .map(|filter| match filter {
            Filter::Shuffle => Ok(SourceFilterBindingV1::Shuffle),
            Filter::Deflate(level) if level <= 9 => Ok(SourceFilterBindingV1::Deflate(level)),
            Filter::Fletcher32 => Ok(SourceFilterBindingV1::Fletcher32),
            _ => Err(NisarSubsetWriteError::MetadataInvalid),
        })
        .collect()
}

fn validate_fixed_datatype(datatype: &hdf5::Datatype) -> Result<(), NisarSubsetWriteError> {
    let descriptor = datatype
        .to_descriptor()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    if fixed_type_descriptor(&descriptor) && datatype.size() > 0 && datatype.size() <= 64 * 1024 {
        Ok(())
    } else {
        Err(NisarSubsetWriteError::MetadataInvalid)
    }
}

fn fixed_type_descriptor(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::VarLenArray(_)
        | TypeDescriptor::VarLenAscii
        | TypeDescriptor::VarLenUnicode
        | TypeDescriptor::Reference(_) => false,
        TypeDescriptor::Compound(compound) => compound
            .fields
            .iter()
            .all(|field| fixed_type_descriptor(&field.ty)),
        TypeDescriptor::FixedArray(inner, _) => fixed_type_descriptor(inner),
        _ => true,
    }
}

fn read_attribute_bindings(
    location: &Location,
) -> Result<Vec<SourceAttributeBindingV1>, NisarSubsetWriteError> {
    let mut names = location
        .attr_names()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    names.sort();
    let mut bindings = Vec::new();
    for name in names {
        if is_dimension_scale_internal_attribute(&name) {
            continue;
        }
        let attribute = location
            .attr(&name)
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
        let datatype = attribute
            .dtype()
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
        validate_fixed_datatype(&datatype)?;
        let shape = attribute.shape();
        let elements = shape
            .iter()
            .try_fold(1_usize, |total, value| total.checked_mul(*value))
            .unwrap_or(0)
            .max(1);
        let byte_count = elements
            .checked_mul(datatype.size())
            .filter(|bytes| *bytes <= MAX_ATTRIBUTE_BYTES)
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
        let mut bytes = vec![0_u8; byte_count];
        let status = hdf5::sync::sync(|| unsafe {
            H5Aread(attribute.id(), datatype.id(), bytes.as_mut_ptr().cast())
        });
        if status < 0 {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        bindings.push(SourceAttributeBindingV1 {
            name,
            datatype_sha256: hdf5_datatype_sha256(&datatype)
                .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
            shape: shape
                .into_iter()
                .map(|value| {
                    u64::try_from(value).map_err(|_| NisarSubsetWriteError::MetadataInvalid)
                })
                .collect::<Result<_, _>>()?,
            value_sha256: sha256_bytes(&bytes),
            datatype: datatype
                .to_descriptor()
                .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?,
            datatype_encoded: encode_datatype(&datatype)?,
            value: Arc::from(bytes),
        });
    }
    Ok(bindings)
}

fn charge_attribute_bindings(
    budget: &mut MetadataReadBudget,
    bindings: &[SourceAttributeBindingV1],
) -> Result<(), NisarSubsetWriteError> {
    bindings
        .iter()
        .try_for_each(|binding| budget.charge(binding.value.len()))
}

fn read_dataset_values_bounded(
    dataset: &Dataset,
    budget: &mut MetadataReadBudget,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<u8>, NisarSubsetWriteError> {
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    let datatype = dataset
        .dtype()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    validate_fixed_datatype(&datatype)?;
    let elements = dataset
        .shape()
        .iter()
        .try_fold(1_usize, |total, value| total.checked_mul(*value))
        .unwrap_or(0)
        .max(1);
    let byte_count = elements
        .checked_mul(datatype.size())
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    budget.charge(byte_count)?;
    let mut bytes = vec![0_u8; byte_count];
    let status = hdf5::sync::sync(|| unsafe {
        H5Dread(
            dataset.id(),
            datatype.id(),
            H5S_ALL,
            H5S_ALL,
            H5P_DEFAULT,
            bytes.as_mut_ptr().cast(),
        )
    });
    // Read the stop once: a second read could see a pause the scheduler has
    // already cleared and report a successful read as invalid metadata
    // (review of D1/D2, downloader enhancement D1b).
    let stopped = cancelled();
    if status < 0 || stopped {
        return Err(if stopped {
            NisarSubsetWriteError::Cancelled
        } else {
            NisarSubsetWriteError::MetadataInvalid
        });
    }
    Ok(bytes)
}

fn is_dimension_scale_internal_attribute(name: &str) -> bool {
    matches!(name, "CLASS" | "NAME" | "REFERENCE_LIST" | "DIMENSION_LIST")
}

fn read_dimension_scale_graph(
    dataset: &Dataset,
) -> Result<SourceDimensionScaleGraphV1, NisarSubsetWriteError> {
    let graph = dimension_scales::read_graph(dataset).map_err(|error| {
        metadata_capture_detail!(error);
        NisarSubsetWriteError::MetadataInvalid
    })?;
    Ok((graph.labels, graph.scales, graph.scale_name, graph.is_scale))
}

fn hash_dataset_values(
    dataset: &Dataset,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<String, NisarSubsetWriteError> {
    let datatype = dataset
        .dtype()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    validate_fixed_datatype(&datatype)?;
    let shape = dataset.shape();
    let mut digest = Sha256::new();
    if shape.is_empty() {
        let mut bytes = vec![0_u8; datatype.size()];
        let status = hdf5::sync::sync(|| unsafe {
            H5Dread(
                dataset.id(),
                datatype.id(),
                H5S_ALL,
                H5S_ALL,
                H5P_DEFAULT,
                bytes.as_mut_ptr().cast(),
            )
        });
        if status < 0 {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        digest.update(bytes);
        return Ok(format!("{:x}", digest.finalize()));
    }
    let row_elements = shape[1..]
        .iter()
        .try_fold(1_usize, |total, value| total.checked_mul(*value))
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    let row_bytes = row_elements
        .checked_mul(datatype.size())
        .filter(|bytes| *bytes > 0 && *bytes <= 64 * 1024 * 1024)
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    let rows_per_slab = (MAX_DATASET_HASH_SLAB_BYTES / row_bytes).max(1);
    let file_space = dataset
        .space()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let rank = shape.len();
    let mut row_start = 0_usize;
    while row_start < shape[0] {
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let rows = rows_per_slab.min(shape[0] - row_start);
        let mut start = vec![0_u64; rank];
        start[0] = row_start as u64;
        let mut count = shape.iter().map(|value| *value as u64).collect::<Vec<_>>();
        count[0] = rows as u64;
        let selected = hdf5::sync::sync(|| unsafe {
            H5Sselect_hyperslab(
                file_space.id(),
                H5S_seloper_t::H5S_SELECT_SET,
                start.as_ptr(),
                std::ptr::null(),
                count.as_ptr(),
                std::ptr::null(),
            )
        });
        if selected < 0 {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        let memory_space = hdf5::sync::sync(|| unsafe {
            H5Screate_simple(rank as c_int, count.as_ptr(), std::ptr::null())
        });
        if memory_space < 0 {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        let byte_count = rows
            .checked_mul(row_bytes)
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
        let mut bytes = vec![0_u8; byte_count];
        let status = hdf5::sync::sync(|| unsafe {
            let read = H5Dread(
                dataset.id(),
                datatype.id(),
                memory_space,
                file_space.id(),
                H5P_DEFAULT,
                bytes.as_mut_ptr().cast(),
            );
            let close = H5Sclose(memory_space);
            if read < 0 || close < 0 {
                -1
            } else {
                0
            }
        });
        if status < 0 {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        digest.update(bytes);
        row_start += rows;
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn read_f64_dataset(
    file: &File,
    path: &str,
    budget: &mut MetadataReadBudget,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(Vec<f64>, Vec<usize>), NisarSubsetWriteError> {
    read_f64_dataset_with_ranks(file, path, &[1, 2], budget, cancelled)
}

fn read_f64_dataset_with_ranks(
    file: &File,
    path: &str,
    allowed_ranks: &[usize],
    budget: &mut MetadataReadBudget,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(Vec<f64>, Vec<usize>), NisarSubsetWriteError> {
    let dataset = open_metadata_dataset(file, path)?;
    let shape = charge_numeric_dataset::<f64>(file, &dataset, allowed_ranks, budget, cancelled)?;
    let values = dataset
        .read_raw::<f64>()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    Ok((values, shape))
}

fn read_u32_dataset(
    file: &File,
    path: &str,
    budget: &mut MetadataReadBudget,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(Vec<u32>, Vec<usize>), NisarSubsetWriteError> {
    let dataset = open_metadata_dataset(file, path)?;
    let shape = charge_numeric_dataset::<u32>(file, &dataset, &[2], budget, cancelled)?;
    let values = dataset
        .read_raw::<u32>()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    Ok((values, shape))
}

fn read_f32_dataset(
    file: &File,
    path: &str,
    budget: &mut MetadataReadBudget,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(Vec<f32>, Vec<usize>), NisarSubsetWriteError> {
    let dataset = open_metadata_dataset(file, path)?;
    let datatype = dataset
        .dtype()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let f32_type =
        hdf5::Datatype::from_type::<f32>().map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let f64_type =
        hdf5::Datatype::from_type::<f64>().map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    if datatype == f32_type {
        let shape = charge_numeric_dataset::<f32>(file, &dataset, &[2], budget, cancelled)?;
        let values = dataset
            .read_raw::<f32>()
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        Ok((values, shape))
    } else if datatype == f64_type {
        // The real NISAR RSLC stores dopplerCentroid/beta0 LUTs as float64;
        // the evidence and derived output keep the reviewed float32 contract.
        // Charge and read as float64, then narrow with the same value bits the
        // HDF5 conversion would produce.
        validate_metadata_dataset_storage(file, &dataset)?;
        let shape = dataset.shape();
        if shape.len() != 2 || shape.contains(&0) {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        let elements = shape
            .iter()
            .try_fold(1_usize, |total, dimension| total.checked_mul(*dimension))
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
        if elements > MAX_METADATA_VALUES {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        budget.charge(
            elements
                .checked_mul(std::mem::size_of::<f64>())
                .ok_or(NisarSubsetWriteError::MetadataInvalid)?,
        )?;
        let values64 = dataset
            .read_raw::<f64>()
            .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
        if cancelled() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let values = values64.into_iter().map(|value| value as f32).collect();
        Ok((values, shape))
    } else {
        Err(NisarSubsetWriteError::MetadataInvalid)
    }
}

fn charge_numeric_dataset<T: H5Type>(
    file: &File,
    dataset: &Dataset,
    allowed_ranks: &[usize],
    budget: &mut MetadataReadBudget,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<usize>, NisarSubsetWriteError> {
    if cancelled() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    validate_metadata_dataset_storage(file, dataset)?;
    if dataset
        .dtype()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?
        != hdf5::Datatype::from_type::<T>().map_err(|_| NisarSubsetWriteError::MetadataInvalid)?
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let shape = dataset.shape();
    if !allowed_ranks.contains(&shape.len()) || shape.contains(&0) {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let elements = if shape.is_empty() {
        1
    } else {
        shape
            .iter()
            .try_fold(1_usize, |total, dimension| total.checked_mul(*dimension))
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?
    };
    if elements > MAX_METADATA_VALUES {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    budget.charge(
        elements
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(NisarSubsetWriteError::MetadataInvalid)?,
    )?;
    Ok(shape)
}

fn validate_metadata_dataset_storage(
    file: &File,
    dataset: &Dataset,
) -> Result<(), NisarSubsetWriteError> {
    let dcpl = dataset
        .dcpl()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let layout = dcpl
        .get_layout()
        .map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
    let external_count = hdf5::sync::sync(|| unsafe { H5Pget_external_count(dcpl.id()) });
    if dataset.filename() != file.filename() || layout == Layout::Virtual || external_count != 0 {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    source_filter_bindings(dataset)?;
    Ok(())
}

fn open_metadata_dataset(file: &File, path: &str) -> Result<Dataset, NisarSubsetWriteError> {
    metadata_capture_stage!("dataset.validate_hard_link", path);
    validate_hard_link_path(file, path)?;
    metadata_capture_stage!("dataset.open", path);
    file.dataset(path).map_err(|error| {
        metadata_capture_detail!(error);
        NisarSubsetWriteError::MetadataInvalid
    })
}

fn validate_hard_link_path(file: &File, path: &str) -> Result<(), NisarSubsetWriteError> {
    if !path.starts_with('/') || path.contains("//") {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let mut prefix = String::new();
    for component in path.split('/').filter(|component| !component.is_empty()) {
        if component == "." || component == ".." {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
        prefix.push('/');
        prefix.push_str(component);
        metadata_capture_stage!("link.inspect", &prefix);
        let encoded =
            CString::new(prefix.as_bytes()).map_err(|_| NisarSubsetWriteError::MetadataInvalid)?;
        let mut info = H5L_info_t::default();
        let status = hdf5::sync::sync(|| unsafe {
            H5Lget_info2(file.id(), encoded.as_ptr(), &mut info, H5P_DEFAULT)
        });
        if status < 0 || info.type_ != H5L_TYPE_HARD {
            metadata_capture_detail!((status, info.type_));
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
    }
    Ok(())
}

fn triples(values: &[f64]) -> Result<Vec<[f64; 3]>, NisarSubsetWriteError> {
    values
        .chunks_exact(3)
        .map(|chunk| Ok([chunk[0], chunk[1], chunk[2]]))
        .collect::<Result<Vec<_>, _>>()
        .and_then(|result| {
            if result.len() * 3 == values.len() {
                Ok(result)
            } else {
                Err(NisarSubsetWriteError::MetadataInvalid)
            }
        })
}

fn quads(values: &[f64]) -> Result<Vec<[f64; 4]>, NisarSubsetWriteError> {
    values
        .chunks_exact(4)
        .map(|chunk| Ok([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Result<Vec<_>, _>>()
        .and_then(|result| {
            if result.len() * 4 == values.len() {
                Ok(result)
            } else {
                Err(NisarSubsetWriteError::MetadataInvalid)
            }
        })
}

fn validate_metadata_evidence(
    evidence: &NisarSubsetMetadataEvidenceV1,
) -> Result<(), NisarSubsetWriteError> {
    for value in [
        evidence.mission_id.as_str(),
        evidence.source_granule_id.as_str(),
        evidence.source_zero_doppler_start_time.as_str(),
        evidence.source_zero_doppler_end_time.as_str(),
    ] {
        if value.is_empty() || value.len() > 512 || !value.is_ascii() || value.contains('\0') {
            return Err(NisarSubsetWriteError::MetadataInvalid);
        }
    }
    // The real NISAR RSLC bounding polygon is a WKT POLYGON of 3-D
    // coordinates (lon lat height) and is 2094 bytes; a 512-byte bound
    // rejected the actual provider value. Keep a bounded upper limit that
    // admits the provider format without unbounded growth.
    let bounding_polygon = evidence.bounding_polygon.as_str();
    if bounding_polygon.is_empty()
        || bounding_polygon.len() > 8192
        || !bounding_polygon.is_ascii()
        || bounding_polygon.contains('\0')
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    if parse_utc_rfc3339_nanos(&evidence.source_zero_doppler_end_time)?
        < parse_utc_rfc3339_nanos(&evidence.source_zero_doppler_start_time)?
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let orbit_count = evidence.orbit_time.len();
    let attitude_count = evidence.attitude_time.len();
    if orbit_count == 0
        || orbit_count > MAX_METADATA_VALUES
        || evidence.orbit_position.len() != orbit_count
        || evidence.orbit_velocity.len() != orbit_count
        || attitude_count == 0
        || attitude_count > MAX_METADATA_VALUES
        || evidence.attitude_quaternions.len() != attitude_count
        || !shape_matches_2(evidence.doppler_shape, evidence.doppler_lut.len())
        || !shape_matches_2(evidence.calibration_shape, evidence.calibration_lut.len())
        || !shape_matches_3(evidence.geolocation_shape, evidence.geolocation_x.len())
        || evidence.geolocation_y.len() != evidence.geolocation_x.len()
        || evidence.height_above_ellipsoid.len() != evidence.geolocation_shape[0]
        || evidence.height_above_ellipsoid.len() < 3
        || evidence.geolocation_epsg <= 0
        || evidence.zero_doppler_time.is_empty()
        || evidence.slant_ranges.is_empty()
        || evidence.slant_ranges.values().any(Vec::is_empty)
        || evidence.valid_samples.len() != evidence.slant_ranges.len()
        || evidence
            .valid_samples
            .values()
            .any(|values| values.len() != evidence.zero_doppler_time.len())
        || evidence.zero_doppler_time.len() > MAX_METADATA_VALUES
        || evidence
            .slant_ranges
            .values()
            .any(|values| values.len() > MAX_METADATA_VALUES)
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let finite_f64 = evidence
        .orbit_time
        .iter()
        .chain(evidence.orbit_position.iter().flatten())
        .chain(evidence.orbit_velocity.iter().flatten())
        .chain(evidence.attitude_time.iter())
        .chain(evidence.attitude_quaternions.iter().flatten())
        .chain(evidence.geolocation_x.iter())
        .chain(evidence.geolocation_y.iter())
        .chain(evidence.height_above_ellipsoid.iter())
        .chain(evidence.zero_doppler_time.iter())
        .chain(evidence.slant_ranges.values().flatten())
        .all(|value| value.is_finite());
    if !finite_f64
        || evidence.doppler_lut.iter().any(|value| !value.is_finite())
        || evidence
            .calibration_lut
            .iter()
            .any(|value| !value.is_finite())
    {
        return Err(NisarSubsetWriteError::MetadataInvalid);
    }
    let f64_values = evidence
        .orbit_time
        .len()
        .checked_add(evidence.orbit_position.len().saturating_mul(3))
        .and_then(|count| count.checked_add(evidence.orbit_velocity.len().saturating_mul(3)))
        .and_then(|count| count.checked_add(evidence.attitude_time.len()))
        .and_then(|count| count.checked_add(evidence.attitude_quaternions.len().saturating_mul(4)))
        .and_then(|count| count.checked_add(evidence.geolocation_x.len()))
        .and_then(|count| count.checked_add(evidence.geolocation_y.len()))
        .and_then(|count| count.checked_add(evidence.height_above_ellipsoid.len()))
        .and_then(|count| count.checked_add(evidence.zero_doppler_time.len()))
        .and_then(|count| {
            evidence
                .slant_ranges
                .values()
                .try_fold(count, |total, values| total.checked_add(values.len()))
        })
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    let f32_values = evidence
        .doppler_lut
        .len()
        .checked_add(evidence.calibration_lut.len())
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    f64_values
        .checked_mul(std::mem::size_of::<f64>())
        .and_then(|bytes| {
            f32_values
                .checked_mul(std::mem::size_of::<f32>())
                .and_then(|other| bytes.checked_add(other))
        })
        .filter(|bytes| *bytes <= MAX_METADATA_BYTES)
        .ok_or(NisarSubsetWriteError::MetadataInvalid)?;
    Ok(())
}

fn shape_matches_2(shape: [usize; 2], length: usize) -> bool {
    shape[0] > 0
        && shape[1] > 0
        && shape[0]
            .checked_mul(shape[1])
            .is_some_and(|count| count == length && count <= MAX_METADATA_VALUES)
}

fn shape_matches_3(shape: [usize; 3], length: usize) -> bool {
    shape.into_iter().all(|dimension| dimension > 0)
        && shape
            .into_iter()
            .try_fold(1_usize, |total, dimension| total.checked_mul(dimension))
            .is_some_and(|count| count == length && count <= MAX_METADATA_VALUES)
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
#[path = "nisar_subset_writer_tests.rs"]
pub mod tests;
