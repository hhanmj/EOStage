#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hdf5::file::FileCloseDegree;
use hdf5::File;
use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy,
    AssetResolverParameters, AssetRole, FrozenAssetAccess,
    NisarSubsetDependencyEvidenceAuthorityV2, NisarSubsetDependencyKindV2,
    NisarSubsetDependencyRelationV2, NisarSubsetFrequency, NisarSubsetFrequencyCapabilityV2,
    NisarSubsetPreparedIntentBuildInputV2, NisarSubsetPreparedIntentV2, NisarSubsetQualityStatusV2,
    NisarSubsetSourceAssetIdentityV2, NisarSubsetSourceCapabilityV2,
    NisarSubsetSourceDependencyNodeKindV2, NisarSubsetSourceProductIdentityV2,
    NisarSubsetSourceValidatorV1, NisarSubsetTransferState, NisarSubsetValidatorKind, SnapshotItem,
    StableAssetReference,
};
use serde_json::{json, Map};

use super::{
    CFloat32, CandidateFileGuard, NisarSubsetMetadataSnapshotV1,
    NisarSubsetVerifiedMetadataDatasetKindV2, NisarSubsetVerifiedMetadataInventoryV2,
    NisarSubsetWriteError, OfflineNisarSubsetWriter, WriterFailpoint,
};
use crate::hdf5_range_transfer::{
    qualify_source_dataset_identity, supported_datatype_sha256, ApprovedHdf5ChunkSetV1,
    Hdf5ChunkAllocationV1, Hdf5ChunkDatatypeV1, Hdf5ChunkFilterPipelineV1,
    Hdf5ChunkPhysicalRangeV1, Hdf5FillValueDefinitionV1, Hdf5RangePlanV1, Hdf5RangeWorkspaceV1,
    OfflineHdf5RangeTransfer, VerifiedHdf5ChunkAllocationV1, VerifiedHdf5ChunkBlockV1,
};
use crate::{
    derive_nisar_live_semantic_plan_sha256, DestinationLeaseController, HttpTransfer, ProxyPolicy,
    RemoteValidator, ResolvedAsset, ResolvedChecksum, ResolvedMethod, TransferOptions,
};

const ETAG: &str = "\"nisar-writer-fixture-v1\"";
const TIMESTAMP: &str = "2026-08-01T00:00:00Z";
const SCIENCE_PATH: &str = "/science/LSAR/RSLC/swaths/frequencyA/HH";
const MASK_PATH: &str = "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask";
const B_HH_PATH: &str = "/science/LSAR/RSLC/swaths/frequencyB/HH";
const B_HV_PATH: &str = "/science/LSAR/RSLC/swaths/frequencyB/HV";
const B_VV_PATH: &str = "/science/LSAR/RSLC/swaths/frequencyB/VV";
const B_MASK_PATH: &str = "/science/LSAR/RSLC/swaths/frequencyB/inputDataExceptionMask";
const OUTPUT_SCIENCE_PATH: &str =
    "/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA/HH";
const OUTPUT_MASK_PATH: &str = "/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA/inputDataExceptionMask";

#[test]
#[ignore = "requires INSAR_SSAR_SPARSE_PATH pointing at preserved real SSAR sparse evidence"]
fn ssar_real_identification_identity_probe() {
    let path =
        PathBuf::from(std::env::var_os("INSAR_SSAR_SPARSE_PATH").expect("INSAR_SSAR_SPARSE_PATH"));
    let file = File::open(path).unwrap();
    let identification = file.group("/science/SSAR/identification").unwrap();
    for name in [
        "missionId",
        "granuleId",
        "productType",
        "productLevel",
        "radarBand",
        "productSpecificationVersion",
    ] {
        let dataset = identification.dataset(name).unwrap_or_else(|error| {
            panic!("SSAR_IDENTIFICATION_DATASET name={name} error={error:?}")
        });
        let value = super::read_fixed_ascii_dataset(&dataset)
            .unwrap_or_else(|error| panic!("SSAR_IDENTIFICATION_READ name={name} error={error:?}"));
        eprintln!("SSAR_IDENTIFICATION_FIELD name={name} value={value:?}");
    }
}

pub struct TempDirectory(
    pub PathBuf,
    #[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
    #[allow(dead_code)]
    crate::hdf5_plugin_isolation::Hdf5TestLockGuard,
);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-nisar-subset-writer-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        #[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
        {
            Self(
                path,
                crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
        }
        #[cfg(all(not(test), not(feature = "offline-nisar-subset-test-support")))]
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn v2_contiguous_mask_uses_science_chunk_shape_for_derived_output() {
    let root = TempDirectory::new("v2-contiguous-mask-derived-chunk");
    let path = root.0.join("mask-layout.h5");
    let file = File::create(&path).unwrap();
    let contiguous = file
        .new_dataset::<u8>()
        .shape((16, 24))
        .create("contiguousMask")
        .unwrap();
    assert_eq!(contiguous.chunk(), None);
    assert_eq!(
        super::derived_v2_mask_chunk_shape(&contiguous, [512, 512]).unwrap(),
        [512, 512]
    );

    let chunked = file
        .new_dataset::<u8>()
        .shape((16, 24))
        .chunk((4, 8))
        .create("chunkedMask")
        .unwrap();
    assert_eq!(
        super::derived_v2_mask_chunk_shape(&chunked, [512, 512]).unwrap(),
        [4, 8]
    );
    assert!(super::derived_v2_mask_chunk_shape(&contiguous, [0, 512]).is_err());
}

struct RangeFileServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl RangeFileServer {
    fn new(source: PathBuf, total: u64, expected_requests: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            for _ in 0..expected_requests {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                let (start, end_exclusive) = parse_range(&request);
                captured.lock().unwrap().push(request);
                let mut source_file = fs::File::open(&source).unwrap();
                source_file.seek(SeekFrom::Start(start)).unwrap();
                let mut body = vec![0_u8; usize::try_from(end_exclusive - start).unwrap()];
                source_file.read_exact(&mut body).unwrap();
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nETag: {ETAG}\r\nContent-Range: bytes {start}-{}/{total}\r\nContent-Length: {}\r\n\r\n",
                    end_exclusive - 1,
                    body.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(&body).unwrap();
                stream.flush().unwrap();
            }
        });
        Self {
            url: format!("http://{address}/fixture.h5"),
            requests,
            worker: Some(worker),
        }
    }

    fn finish(mut self) -> Vec<String> {
        self.worker.take().unwrap().join().unwrap();
        Arc::try_unwrap(self.requests)
            .unwrap()
            .into_inner()
            .unwrap()
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
        request.push_str(&line);
    }
    request
}

fn parse_range(request: &str) -> (u64, u64) {
    let line = request
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("range: bytes="))
        .unwrap();
    let value = line.split_once('=').unwrap().1.trim();
    let (start, end) = value.split_once('-').unwrap();
    (start.parse().unwrap(), end.parse::<u64>().unwrap() + 1)
}

pub struct ReadyCase {
    // Declared before `_root` so the Windows source pin is released before the
    // temporary directory cleanup runs.
    _source_pin: Option<super::TestSourcePin>,
    pub _root: TempDirectory,
    pub source_path: PathBuf,
    pub source_before: Vec<u8>,
    pub request: insar_core::NisarSubsetRequestManifestV1,
    pub plan: Hdf5RangePlanV1,
    pub transfer: crate::Hdf5RangeTransferResultV1,
    pub workspace: Hdf5RangeWorkspaceV1,
    pub metadata: NisarSubsetMetadataSnapshotV1,
}

fn ready_case(label: &str) -> ReadyCase {
    ready_case_with_components(label, 1)
}

fn ready_case_with_components(label: &str, component_count: u16) -> ReadyCase {
    ready_case_with_selection(
        label,
        component_count,
        NisarSubsetFrequency::A,
        "HH",
        SCIENCE_PATH,
        MASK_PATH,
    )
}

pub fn ready_case_with_selection(
    label: &str,
    component_count: u16,
    frequency: NisarSubsetFrequency,
    polarization: &str,
    science_path: &str,
    mask_path: &str,
) -> ReadyCase {
    ready_case_with_selection_impl(
        label,
        component_count,
        frequency,
        polarization,
        science_path,
        mask_path,
        false,
        false,
        false,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
pub fn ready_case_with_selection_and_source_mutation(
    label: &str,
    component_count: u16,
    frequency: NisarSubsetFrequency,
    polarization: &str,
    science_path: &str,
    mask_path: &str,
    deferred_science_layout: bool,
    mutate_source: impl FnOnce(&File),
) -> ReadyCase {
    ready_case_with_selection_impl(
        label,
        component_count,
        frequency,
        polarization,
        science_path,
        mask_path,
        deferred_science_layout,
        false,
        true,
        mutate_source,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn ready_case_with_selection_for_v2(
    label: &str,
    component_count: u16,
    frequency: NisarSubsetFrequency,
    polarization: &str,
    science_path: &str,
    mask_path: &str,
    deferred_science_layout: bool,
    mutate_source: impl FnOnce(&File),
) -> ReadyCase {
    ready_case_with_selection_impl(
        label,
        component_count,
        frequency,
        polarization,
        science_path,
        mask_path,
        deferred_science_layout,
        true,
        true,
        mutate_source,
    )
}

#[allow(clippy::too_many_arguments)]
fn ready_case_with_selection_impl(
    label: &str,
    component_count: u16,
    frequency: NisarSubsetFrequency,
    polarization: &str,
    science_path: &str,
    mask_path: &str,
    deferred_science_layout: bool,
    compact_v2_inventory_ballast: bool,
    apply_source_mutation: bool,
    mutate_source: impl FnOnce(&File),
) -> ReadyCase {
    assert!((1..=2).contains(&component_count));
    let root = TempDirectory::new(label);
    let source_path = root.0.join("source.h5");
    let source_pin = create_source_fixture_with_science_layout(
        &source_path,
        deferred_science_layout,
        compact_v2_inventory_ballast,
    );
    if apply_source_mutation {
        let source = hdf5_fixture_file_builder().open_rw(&source_path).unwrap();
        mutate_source(&source);
        source.flush().unwrap();
        source.close().unwrap();
    }
    let source_before = fs::read(&source_path).unwrap();
    let source_size = source_before.len() as u64;
    let mut request = request(source_size, 1, &source_before);
    request.request.frequency = frequency.clone();
    request.request.polarization = polarization.to_owned();
    request.mapping.windows[0].frequency = frequency;
    request.mapping.windows[0].polarization = polarization.to_owned();
    request.mapping.windows[0].dataset_path = science_path.to_owned();
    let mut chunks = source_chunk_evidence(&source_path, science_path, mask_path);
    let remote_bytes = chunks.iter().fold(0_u64, |total, chunk| {
        total
            + match chunk.allocation {
                Hdf5ChunkAllocationV1::Allocated { stored_size, .. } => stored_size,
                Hdf5ChunkAllocationV1::Fill { .. } => 0,
            }
    });
    request.expected_output.estimated_remote_bytes = remote_bytes;
    request.validate().unwrap();
    if component_count == 2 {
        let mut second = request.mapping.windows[0].clone();
        second.component_id = 1;
        request.request.aoi_component_count = 2;
        request.mapping.windows.push(second);
        let mut second_component = chunks.clone();
        for chunk in &mut second_component {
            chunk.component_id = 1;
        }
        chunks.extend(second_component);
        request.validate().unwrap();
    }
    let source_validator = NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: ETAG.to_owned(),
        content_length: source_size,
    };
    let metadata_result = NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
        &request,
        source_validator,
        &source_path,
        &|| false,
    );
    let metadata = metadata_result.unwrap_or_else(|error| {
        let diagnostic = super::metadata_capture_diagnostic_for_test();
        let source_after = fs::read(&source_path);
        let environment = [
            "HDF5_PLUGIN_PATH",
            "HDF5_PLUGIN_PRELOAD",
            "HDF5_VOL_CONNECTOR",
            "HDF5_DRIVER",
            "HDF5_DRIVER_CONFIG",
            "HDF5_USE_FILE_LOCKING",
        ]
        .map(|name| (name, crate::hdf5_plugin_isolation::crt_env::get(name)));
        panic!(
            "metadata capture failed: error={error:?}; diagnostic={diagnostic:?}; thread={:?}; source={}; exists={}; source_read={:?}; source_unchanged={:?}; plugin_loading_disabled={}; hdf5_test_lock_held={}; environment={environment:?}",
            std::thread::current().id(),
            source_path.display(),
            source_path.is_file(),
            source_after.as_ref().map(Vec::len),
            source_after.as_ref().map(|bytes| bytes == &source_before),
            crate::hdf5_plugin_loading_disabled(),
            crate::hdf5_plugin_isolation::HDF5_TEST_LOCK.held_by_current_thread(),
        )
    });
    let plan = Hdf5RangePlanV1::from_offline_evidence(
        &request,
        ApprovedHdf5ChunkSetV1 {
            request_manifest_sha256: request.sha256().unwrap(),
            source_length: source_size,
            layout_evidence_digest:
                "86a615c78d3a9d1407ec3b5f4ddc76a10a5aff157b473bba48d9d31865e71500".to_owned(),
            metadata_evidence_sha256: metadata.evidence_sha256().to_owned(),
            chunks,
        },
    )
    .unwrap();
    let server = RangeFileServer::new(source_path.clone(), source_size, plan.ranges().len());
    let workspace = Hdf5RangeWorkspaceV1::new(root.0.join("transfer")).unwrap();
    let runtime = OfflineHdf5RangeTransfer::new(
        HttpTransfer::new_with_proxy_policy(
            TransferOptions {
                max_attempts: 1,
                backoff: Duration::ZERO,
                timeout: Duration::from_secs(2),
                chunk_size: 8,
                ..TransferOptions::default()
            },
            ProxyPolicy::Direct,
        )
        .unwrap(),
        &request,
        &snapshot(&request),
        resolved_asset(&server.url, &request),
        workspace.clone(),
        DestinationLeaseController::new_for_qualification_test(),
    )
    .unwrap();
    let transfer = runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    assert_eq!(
        transfer.manifest().state,
        NisarSubsetTransferState::RangesComplete
    );
    assert_eq!(server.finish().len(), plan.ranges().len());
    ReadyCase {
        _source_pin: Some(source_pin),
        _root: root,
        source_path,
        source_before,
        request,
        plan,
        transfer,
        workspace,
        metadata,
    }
}

#[allow(dead_code)] // Exported only to the feature-gated cross-crate acceptance test.
pub fn rebind_ready_case_request(
    case: ReadyCase,
    request: insar_core::NisarSubsetRequestManifestV1,
) -> ReadyCase {
    let source_size = case.source_before.len() as u64;
    assert_eq!(request.source.expected_size_bytes, source_size);
    assert_eq!(request.mapping.windows, case.request.mapping.windows);
    assert_eq!(
        request.expected_output.estimated_remote_bytes,
        case.request.expected_output.estimated_remote_bytes
    );
    let source_validator = NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: ETAG.to_owned(),
        content_length: source_size,
    };
    let metadata = NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
        &request,
        source_validator,
        &case.source_path,
        &|| false,
    )
    .unwrap();
    let plan = Hdf5RangePlanV1::from_offline_evidence(
        &request,
        ApprovedHdf5ChunkSetV1 {
            request_manifest_sha256: request.sha256().unwrap(),
            source_length: source_size,
            layout_evidence_digest:
                "86a615c78d3a9d1407ec3b5f4ddc76a10a5aff157b473bba48d9d31865e71500".to_owned(),
            metadata_evidence_sha256: metadata.evidence_sha256().to_owned(),
            chunks: case.plan.chunks().to_vec(),
        },
    )
    .unwrap();
    let server = RangeFileServer::new(case.source_path.clone(), source_size, plan.ranges().len());
    let workspace = Hdf5RangeWorkspaceV1::new(case._root.0.join("transfer-rebound")).unwrap();
    let runtime = OfflineHdf5RangeTransfer::new(
        HttpTransfer::new_with_proxy_policy(
            TransferOptions {
                max_attempts: 1,
                backoff: Duration::ZERO,
                timeout: Duration::from_secs(2),
                chunk_size: 8,
                ..TransferOptions::default()
            },
            ProxyPolicy::Direct,
        )
        .unwrap(),
        &request,
        &snapshot(&request),
        resolved_asset(&server.url, &request),
        workspace.clone(),
        DestinationLeaseController::new_for_qualification_test(),
    )
    .unwrap();
    let transfer = runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    assert_eq!(
        transfer.manifest().state,
        NisarSubsetTransferState::RangesComplete
    );
    assert_eq!(server.finish().len(), plan.ranges().len());
    ReadyCase {
        _source_pin: case._source_pin,
        _root: case._root,
        source_path: case.source_path,
        source_before: case.source_before,
        request,
        plan,
        transfer,
        workspace,
        metadata,
    }
}

/// A V2-only source fixture: the source file of `ready_case_with_selection_for_v2`
/// after an arbitrary caller mutation, without the V1 Range transfer whose fixed
/// request window describes only the original 4x4 science layout. Its groups
/// carry no HDF5 object timestamps, so the source bytes and every provenance
/// digest derived from them are reproducible across runs.
pub struct V2SourceCase {
    // Declared before `root` so the Windows source pin is released before the
    // temporary directory cleanup runs.
    _source_pin: super::TestSourcePin,
    pub root: TempDirectory,
    pub source_path: PathBuf,
    pub request: insar_core::NisarSubsetRequestManifestV1,
}

pub fn v2_source_case(label: &str, mutate_source: impl FnOnce(&File)) -> V2SourceCase {
    let root = TempDirectory::new(label);
    let source_path = root.0.join("source.h5");
    let source_pin =
        create_source_fixture_with_object_times(&source_path, false, true, false, false);
    let source = hdf5_fixture_file_builder().open_rw(&source_path).unwrap();
    mutate_source(&source);
    source.flush().unwrap();
    source.close().unwrap();
    let source_before = fs::read(&source_path).unwrap();
    let request = request(source_before.len() as u64, 1, &source_before);
    V2SourceCase {
        _source_pin: source_pin,
        root,
        source_path,
        request,
    }
}

fn create_source_fixture(path: &Path) -> super::TestSourcePin {
    create_source_fixture_with_science_layout(path, false, false)
}

pub(crate) fn create_ssar_source_fixture_with_science_layout(
    path: &Path,
    deferred_science_layout: bool,
    compact_v2_inventory_ballast: bool,
) -> super::TestSourcePin {
    create_source_fixture_with_profile(
        path,
        deferred_science_layout,
        compact_v2_inventory_ballast,
        true,
    )
}

fn create_source_fixture_with_science_layout(
    path: &Path,
    deferred_science_layout: bool,
    compact_v2_inventory_ballast: bool,
) -> super::TestSourcePin {
    create_source_fixture_with_profile(
        path,
        deferred_science_layout,
        compact_v2_inventory_ballast,
        false,
    )
}

fn create_source_fixture_with_profile(
    path: &Path,
    deferred_science_layout: bool,
    compact_v2_inventory_ballast: bool,
    ssar: bool,
) -> super::TestSourcePin {
    create_source_fixture_with_object_times(
        path,
        deferred_science_layout,
        compact_v2_inventory_ballast,
        ssar,
        true,
    )
}

/// `track_object_times=false` creates the root and every group without HDF5
/// object timestamps, so the source bytes are reproducible across runs.
fn create_source_fixture_with_object_times(
    path: &Path,
    deferred_science_layout: bool,
    compact_v2_inventory_ballast: bool,
    ssar: bool,
    track_object_times: bool,
) -> super::TestSourcePin {
    let science_root = if ssar {
        "/science/SSAR"
    } else {
        "/science/LSAR"
    };
    let frequency_a_polarizations = if ssar { ["RH", "RV"] } else { ["HH", "HV"] };
    let frequency_b_polarizations = if ssar {
        ["RH", "RV", "LV"]
    } else {
        ["HH", "HV", "VV"]
    };
    let route_ballast_polarization = if ssar { "LH" } else { "RV" };
    let radar_band = if ssar { "S" } else { "L" };
    let product_specification_version = if ssar { "1.2.1" } else { "1.5.0" };
    let granule_id = if ssar {
        "NISAR_S1_PR_RSLC_029_063_D_079_2500_CRNA_A_20260829T135915_20260829T135945_P00500_M_P_I_001"
    } else {
        "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001"
    };
    let mut builder = hdf5_fixture_file_builder();
    if !track_object_times {
        builder.with_fcpl(|fcpl| fcpl.obj_track_times(false));
    }
    let file = builder.create_excl(path).unwrap();
    let root = file.group("/").unwrap();
    let convention = hdf5::types::FixedAscii::<32>::from_ascii("NISAR_RSLC_1.5.0").unwrap();
    root.new_attr::<hdf5::types::FixedAscii<32>>()
        .create("Conventions")
        .unwrap()
        .write_scalar(&convention)
        .unwrap();
    let frequency = create_fixture_group(
        &file,
        track_object_times,
        &format!("{science_root}/RSLC/swaths/frequencyA"),
    )
    .unwrap();
    let science = (0..16)
        .map(|index| CFloat32 {
            r: index as f32 + 0.25,
            i: -(index as f32) - 0.5,
        })
        .collect::<Vec<_>>();
    for polarization in frequency_a_polarizations {
        let dataset = frequency
            .new_dataset::<CFloat32>()
            .shape((4, 4))
            .chunk((2, 2))
            .create(polarization)
            .unwrap();
        if !deferred_science_layout {
            dataset.write_raw(&science).unwrap();
        }
    }
    let mask = (0_u8..36).collect::<Vec<_>>();
    {
        let dataset = frequency
            .new_dataset::<u8>()
            .shape((6, 6))
            .chunk((1, 2))
            .create("inputDataExceptionMask")
            .unwrap();
        if !deferred_science_layout {
            dataset.write_raw(&mask).unwrap();
        }
    }
    let route_ballast = deferred_science_layout.then(|| {
        frequency
            .new_dataset::<u8>()
            .shape((4 * 1024 * 1024,))
            .create(route_ballast_polarization)
            .unwrap()
    });
    super::write_vector(
        &frequency,
        "slantRange",
        &[800_000.0_f64, 800_010.0, 800_020.0, 800_030.0],
    )
    .unwrap();
    super::write_matrix(
        &frequency,
        "validSamplesSubSwath1",
        4,
        2,
        &[0_u32, 3, 0, 3, 0, 3, 0, 3],
    )
    .unwrap();
    super::write_scalar(&frequency, "slantRangeSpacing", &10.0_f64).unwrap();
    let frequency_b = create_fixture_group(
        &file,
        track_object_times,
        &format!("{science_root}/RSLC/swaths/frequencyB"),
    )
    .unwrap();
    for polarization in frequency_b_polarizations {
        let dataset = frequency_b
            .new_dataset::<CFloat32>()
            .shape((4, 4))
            .chunk((2, 2))
            .create(polarization)
            .unwrap();
        if !deferred_science_layout {
            dataset.write_raw(&science).unwrap();
        }
    }
    {
        let dataset = frequency_b
            .new_dataset::<u8>()
            .shape((6, 6))
            .chunk((1, 2))
            .create("inputDataExceptionMask")
            .unwrap();
        if !deferred_science_layout {
            dataset.write_raw(&mask).unwrap();
        }
    }
    super::write_vector(
        &frequency_b,
        "slantRange",
        &[900_000.0_f64, 900_010.0, 900_020.0, 900_030.0],
    )
    .unwrap();
    super::write_matrix(
        &frequency_b,
        "validSamplesSubSwath1",
        4,
        2,
        &[0_u32, 3, 0, 3, 0, 3, 0, 3],
    )
    .unwrap();
    super::write_scalar(&frequency_b, "slantRangeSpacing", &10.0_f64).unwrap();
    let identification = create_fixture_group(
        &file,
        track_object_times,
        &format!("{science_root}/identification"),
    )
    .unwrap();
    super::write_ascii_scalar::<32>(&identification, "missionId", "NISAR").unwrap();
    super::write_ascii_scalar::<256>(&identification, "granuleId", granule_id).unwrap();
    super::write_ascii_scalar::<1024>(
        &identification,
        "boundingPolygon",
        "POLYGON ((36 9, 39 9, 39 12, 36 12, 36 9))",
    )
    .unwrap();
    super::write_ascii_scalar::<29>(
        &identification,
        "zeroDopplerStartTime",
        "2026-07-26T15:58:03.000000Z",
    )
    .unwrap();
    super::write_ascii_scalar::<29>(
        &identification,
        "zeroDopplerEndTime",
        "2026-07-26T15:58:04.500000Z",
    )
    .unwrap();
    super::write_scalar(&identification, "isFullFrame", &1_u8).unwrap();
    for (name, value) in [
        ("productType", "RSLC"),
        ("productLevel", "L1"),
        ("radarBand", radar_band),
        ("productSpecificationVersion", product_specification_version),
    ] {
        super::write_ascii_scalar::<32>(&identification, name, value).unwrap();
    }
    let orbit = create_fixture_group(
        &file,
        track_object_times,
        &format!("{science_root}/RSLC/metadata/orbit"),
    )
    .unwrap();
    super::write_vector(&orbit, "time", &[0.0_f64, 1.0]).unwrap();
    super::write_matrix(
        &orbit,
        "position",
        2,
        3,
        &[1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
    )
    .unwrap();
    super::write_matrix(
        &orbit,
        "velocity",
        2,
        3,
        &[0.1_f64, 0.2, 0.3, 0.4, 0.5, 0.6],
    )
    .unwrap();
    let attitude = create_fixture_group(
        &file,
        track_object_times,
        &format!("{science_root}/RSLC/metadata/attitude"),
    )
    .unwrap();
    super::write_vector(&attitude, "time", &[0.0_f64, 1.0]).unwrap();
    super::write_matrix(
        &attitude,
        "quaternions",
        2,
        4,
        &[1.0_f64, 0.0, 0.0, 0.0, 0.99, 0.01, 0.0, 0.0],
    )
    .unwrap();
    let processing = create_fixture_group(
        &file,
        track_object_times,
        &format!("{science_root}/RSLC/metadata/processingInformation/parameters/frequencyA"),
    )
    .unwrap();
    super::write_matrix(
        &processing,
        "dopplerCentroid",
        2,
        2,
        &[1.0_f32, 2.0, 3.0, 4.0],
    )
    .unwrap();
    let processing_b = create_fixture_group(
        &file,
        track_object_times,
        &format!("{science_root}/RSLC/metadata/processingInformation/parameters/frequencyB"),
    )
    .unwrap();
    super::write_matrix(
        &processing_b,
        "dopplerCentroid",
        2,
        2,
        &[1.5_f32, 2.5, 3.5, 4.5],
    )
    .unwrap();
    let calibration = create_fixture_group(
        &file,
        track_object_times,
        &format!("{science_root}/RSLC/metadata/calibrationInformation/geometry"),
    )
    .unwrap();
    super::write_matrix(&calibration, "beta0", 2, 2, &[0.5_f32, 0.6, 0.7, 0.8]).unwrap();
    let geolocation = create_fixture_group(
        &file,
        track_object_times,
        &format!("{science_root}/RSLC/metadata/geolocationGrid"),
    )
    .unwrap();
    super::write_scalar(&geolocation, "epsg", &4326_i32).unwrap();
    let coordinate_x = [
        36.0_f64, 37.0, 36.0, 37.0, 36.1, 37.1, 36.1, 37.1, 36.2, 37.2, 36.2, 37.2,
    ];
    let coordinate_y = [
        9.0_f64, 9.0, 10.0, 10.0, 9.1, 9.1, 10.1, 10.1, 9.2, 9.2, 10.2, 10.2,
    ];
    let coordinate_x_dataset = geolocation
        .new_dataset::<f64>()
        .shape((3, 2, 2))
        .chunk((1, 2, 2))
        .shuffle()
        .deflate(6)
        .create("coordinateX")
        .unwrap();
    coordinate_x_dataset.write_raw(&coordinate_x).unwrap();
    let coordinate_y_dataset = geolocation
        .new_dataset::<f64>()
        .shape((3, 2, 2))
        .chunk((1, 2, 2))
        .shuffle()
        .deflate(6)
        .create("coordinateY")
        .unwrap();
    coordinate_y_dataset.write_raw(&coordinate_y).unwrap();
    super::write_vector(
        &geolocation,
        "heightAboveEllipsoid",
        &[0.0_f64, 2_000.0, 4_000.0],
    )
    .unwrap();
    super::write_vector(&geolocation, "zeroDopplerTime", &[1000.0_f64, 1001.5]).unwrap();
    super::write_vector(&geolocation, "slantRange", &[800_000.0_f64, 800_030.0]).unwrap();
    let height = geolocation.dataset("heightAboveEllipsoid").unwrap();
    let geo_time = geolocation.dataset("zeroDopplerTime").unwrap();
    let geo_range = geolocation.dataset("slantRange").unwrap();
    for (dataset, name) in [
        (&height, "heightAboveEllipsoid"),
        (&geo_time, "zeroDopplerTime"),
        (&geo_range, "slantRange"),
    ] {
        assert_eq!(super::dimension_scales::set_scale(dataset, name), Ok(()));
    }
    for coordinate in [&coordinate_x_dataset, &coordinate_y_dataset] {
        for (dimension, scale) in [(0_usize, &height), (1, &geo_time), (2, &geo_range)] {
            assert_eq!(
                super::dimension_scales::attach_scale(&file, coordinate, &scale.name(), dimension,),
                Ok(())
            );
        }
    }
    let swaths = file.group(&format!("{science_root}/RSLC/swaths")).unwrap();
    super::write_vector(
        &swaths,
        "zeroDopplerTime",
        &[1000.0_f64, 1000.5, 1001.0, 1001.5],
    )
    .unwrap();
    super::write_scalar(&swaths, "zeroDopplerTimeSpacing", &0.5_f64).unwrap();
    // Preserve the historical V1 fixture identity. V2 execution fixtures use a
    // path containing both frequency components, so this zero-semantic dataset-
    // count ballast is outside either selected-frequency dependency closure.
    let inventory_path = if compact_v2_inventory_ballast {
        format!("{science_root}/RSLC/metadata/RFI/phase2c2Fixture/frequencyA/frequencyB")
    } else {
        format!("{science_root}/RSLC/metadata/RFI/officialInventory")
    };
    let inventory = create_fixture_group(&file, track_object_times, &inventory_path).unwrap();
    let existing = source_dataset_count(&file);
    assert!(existing < super::FIRST_SCENE_SOURCE_DATASET_COUNT);
    for index in existing..super::FIRST_SCENE_SOURCE_DATASET_COUNT {
        let name = format!("dataset{index:03}");
        inventory
            .new_dataset::<f32>()
            .create(name.as_str())
            .unwrap()
            .write_scalar(&(index as f32 + 0.25))
            .unwrap();
    }
    assert_eq!(
        source_dataset_count(&file),
        super::FIRST_SCENE_SOURCE_DATASET_COUNT
    );
    if deferred_science_layout {
        for path in [
            format!(
                "{science_root}/RSLC/swaths/frequencyA/{}",
                frequency_a_polarizations[0]
            ),
            format!(
                "{science_root}/RSLC/swaths/frequencyA/{}",
                frequency_a_polarizations[1]
            ),
            format!(
                "{science_root}/RSLC/swaths/frequencyB/{}",
                frequency_b_polarizations[0]
            ),
            format!(
                "{science_root}/RSLC/swaths/frequencyB/{}",
                frequency_b_polarizations[1]
            ),
            format!(
                "{science_root}/RSLC/swaths/frequencyB/{}",
                frequency_b_polarizations[2]
            ),
        ] {
            file.dataset(&path).unwrap().write_raw(&science).unwrap();
        }
        file.dataset(&format!(
            "{science_root}/RSLC/swaths/frequencyA/inputDataExceptionMask"
        ))
        .unwrap()
        .write_raw(&mask)
        .unwrap();
        file.dataset(&format!(
            "{science_root}/RSLC/swaths/frequencyB/inputDataExceptionMask"
        ))
        .unwrap()
        .write_raw(&mask)
        .unwrap();
        route_ballast
            .unwrap()
            .write_raw(&vec![0_u8; 4 * 1024 * 1024])
            .unwrap();
    }
    file.flush().unwrap();
    let source_pin = super::TestSourcePin::acquire_while_hdf5_open(path).unwrap();
    file.close().unwrap();
    source_pin
}

/// Creates `path` and its missing parents. Untimed groups are created one
/// component at a time with object time tracking disabled.
fn create_fixture_group(
    file: &File,
    track_object_times: bool,
    path: &str,
) -> hdf5::Result<hdf5::Group> {
    if track_object_times {
        return file.create_group(path);
    }
    let mut current = String::new();
    for component in path.split('/').filter(|component| !component.is_empty()) {
        let parent = if current.is_empty() {
            file.group("/")?
        } else {
            file.group(&current)?
        };
        current.push('/');
        current.push_str(component);
        if parent.link_exists(component) {
            continue;
        }
        let name = std::ffi::CString::new(component).unwrap();
        let group = hdf5::sync::sync(|| unsafe {
            let gcpl = hdf5_sys::h5p::H5Pcreate(*hdf5::globals::H5P_GROUP_CREATE);
            assert!(gcpl >= 0);
            assert!(hdf5_sys::h5p::H5Pset_obj_track_times(gcpl, 0) >= 0);
            let group = hdf5_sys::h5g::H5Gcreate2(
                parent.id(),
                name.as_ptr(),
                hdf5_sys::h5p::H5P_DEFAULT,
                gcpl,
                hdf5_sys::h5p::H5P_DEFAULT,
            );
            hdf5_sys::h5p::H5Pclose(gcpl);
            group
        });
        assert!(group >= 0, "untimed fixture group {current}");
        hdf5::sync::sync(|| unsafe { hdf5_sys::h5g::H5Gclose(group) });
    }
    file.group(path)
}

fn hdf5_fixture_file_builder() -> hdf5::FileBuilder {
    let mut builder = File::with_options();
    builder.with_fapl(|fapl| fapl.fclose_degree(FileCloseDegree::Strong));
    builder
}

fn source_dataset_count(file: &File) -> usize {
    fn count(group: &hdf5::Group) -> usize {
        group
            .member_names()
            .unwrap()
            .into_iter()
            .map(
                |name| match group.loc_info_by_name(&name).unwrap().loc_type {
                    hdf5::LocationType::Group => count(&group.group(&name).unwrap()),
                    hdf5::LocationType::Dataset => 1,
                    _ => 0,
                },
            )
            .sum()
    }
    count(&file.group("/").unwrap())
}

fn v2_prepared_intent(
    case: &ReadyCase,
    frequency: NisarSubsetFrequency,
    selected_polarizations: &[&str],
) -> NisarSubsetPreparedIntentV2 {
    NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: NisarSubsetSourceProductIdentityV2 {
            provider_id: "asf.nasa.earthdata".to_owned(),
            collection_id: "C2853145197-ASF".to_owned(),
            collection_profile_id: "asf.nisar.l.rslc.provisional.v1".to_owned(),
            provider_product_id: "G9900000001-ASF".to_owned(),
            provider_product_key: "NISAR_FIXTURE_SOURCE".to_owned(),
            product_specification_version: case
                .request
                .source
                .product_specification_version
                .clone(),
            crid: "P05023".to_owned(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        },
        source_asset_identity: NisarSubsetSourceAssetIdentityV2 {
            asset_key: "science-rslc-hdf5".to_owned(),
            stable_asset_key: "NISAR_FIXTURE_SOURCE::ASSET::science-rslc-hdf5".to_owned(),
            file_name: case.request.source.file_name.clone(),
            checksum: case.request.source.checksum.clone(),
        },
        source_capability: NisarSubsetSourceCapabilityV2 {
            evidence_sha256: "b".repeat(64),
            frequencies: vec![
                NisarSubsetFrequencyCapabilityV2 {
                    frequency: NisarSubsetFrequency::A,
                    available_polarizations: vec![
                        "HH".to_owned(),
                        "HV".to_owned(),
                        "VH".to_owned(),
                    ],
                },
                NisarSubsetFrequencyCapabilityV2 {
                    frequency: NisarSubsetFrequency::B,
                    available_polarizations: vec![
                        "HH".to_owned(),
                        "HV".to_owned(),
                        "VH".to_owned(),
                        "VV".to_owned(),
                    ],
                },
            ],
        },
        source_footprint: json!({
            "type": "Polygon",
            "coordinates": [[[36.0, 9.0], [39.0, 9.0], [39.0, 12.0], [36.0, 12.0], [36.0, 9.0]]]
        })
        .as_object()
        .unwrap()
        .clone(),
        requested_geometry: json!({
            "type": "Polygon",
            "coordinates": [[[36.5, 9.5], [38.5, 9.5], [38.5, 11.5], [36.5, 11.5], [36.5, 9.5]]]
        })
        .as_object()
        .unwrap()
        .clone(),
        selected_frequency: frequency,
        selected_polarizations: selected_polarizations
            .iter()
            .map(|polarization| (*polarization).to_owned())
            .collect(),
    })
    .unwrap()
}

fn v2_source_validator(path: &Path) -> NisarSubsetSourceValidatorV1 {
    NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: ETAG.to_owned(),
        content_length: fs::metadata(path).unwrap().len(),
    }
}

fn capture_v2_inventory(
    case: &ReadyCase,
    frequency: NisarSubsetFrequency,
    selected_polarizations: &[&str],
) -> NisarSubsetVerifiedMetadataInventoryV2 {
    let intent = v2_prepared_intent(case, frequency, selected_polarizations);
    NisarSubsetVerifiedMetadataInventoryV2::from_verified_offline_metadata_hdf5(
        &intent,
        v2_source_validator(&case.source_path),
        &case.source_path,
        &|| false,
    )
    .unwrap()
}

#[test]
fn verified_inventory_v2_single_pol_captures_real_metadata_authority() {
    let case = ready_case("verified-inventory-v2-single-pol");
    let inventory = capture_v2_inventory(&case, NisarSubsetFrequency::A, &["HH"]);

    assert_eq!(inventory.selection().selected_polarizations(), ["HH"]);
    assert_eq!(inventory.selected_science().len(), 1);
    assert_eq!(inventory.selected_science()[0].source_path(), SCIENCE_PATH);
    assert_eq!(inventory.validity_mask_paths(), [MASK_PATH]);
    assert_eq!(
        inventory.valid_samples_paths(),
        ["/science/LSAR/RSLC/swaths/frequencyA/validSamplesSubSwath1"]
    );
    assert_eq!(inventory.evidence().source_dataset_count(), 207);
    assert_eq!(
        inventory.evidence().source_size(),
        case.source_before.len() as u64
    );
    assert_eq!(inventory.evidence().inventory_sha256().len(), 64);
    assert_eq!(inventory.evidence().evidence_sha256().len(), 64);
    assert_eq!(inventory.evidence().source_validator_sha256().len(), 64);
    assert!(inventory.datasets().iter().any(|dataset| {
        dataset.path() == SCIENCE_PATH
            && dataset.kind() == NisarSubsetVerifiedMetadataDatasetKindV2::SelectedScience
            && dataset.shape() == [4, 4]
    }));
    assert!(!inventory
        .datasets()
        .iter()
        .any(|dataset| dataset.path().ends_with("/frequencyA/HV")));
    assert!(inventory
        .identification_metadata_paths()
        .iter()
        .any(|path| path.ends_with("/missionId")));
    assert!(inventory
        .provenance_metadata_paths()
        .iter()
        .any(|path| path.contains("/processingInformation/")));
    assert!(inventory
        .groups()
        .iter()
        .find(|group| group.path() == "/")
        .unwrap()
        .attributes()
        .iter()
        .any(|attribute| attribute.name() == "Conventions"));
    let coordinate = inventory
        .datasets()
        .iter()
        .find(|dataset| dataset.path().ends_with("/geolocationGrid/coordinateX"))
        .unwrap();
    assert_eq!(coordinate.dimension_scales().len(), 3);
    assert!(coordinate
        .dimension_scales()
        .iter()
        .all(|scales| scales.len() == 1));
}

#[test]
fn verified_inventory_v2_multi_pol_and_frequency_b_are_selection_complete() {
    let case = ready_case("verified-inventory-v2-multi-pol");
    let frequency_a = capture_v2_inventory(&case, NisarSubsetFrequency::A, &["HV", "HH"]);
    assert_eq!(
        frequency_a.selection().selected_polarizations(),
        ["HH", "HV"]
    );
    assert_eq!(
        frequency_a
            .selected_science()
            .iter()
            .map(|science| science.source_path())
            .collect::<Vec<_>>(),
        [
            "/science/LSAR/RSLC/swaths/frequencyA/HH",
            "/science/LSAR/RSLC/swaths/frequencyA/HV",
        ]
    );

    let frequency_b = capture_v2_inventory(&case, NisarSubsetFrequency::B, &["VV", "HV"]);
    assert_eq!(
        frequency_b.selection().selected_polarizations(),
        ["HV", "VV"]
    );
    assert_eq!(frequency_b.validity_mask_paths(), [B_MASK_PATH]);
    assert_eq!(
        frequency_b.valid_samples_paths(),
        ["/science/LSAR/RSLC/swaths/frequencyB/validSamplesSubSwath1"]
    );
    assert!(frequency_b
        .selected_science()
        .iter()
        .any(|science| science.source_path() == B_HV_PATH));
    assert!(frequency_b
        .selected_science()
        .iter()
        .any(|science| science.source_path() == B_VV_PATH));
    assert!(frequency_b.datasets().iter().any(|dataset| {
        dataset
            .path()
            .ends_with("/parameters/frequencyB/dopplerCentroid")
            && dataset.kind() == NisarSubsetVerifiedMetadataDatasetKindV2::ProcessingMetadata
    }));
}

#[test]
fn verified_inventory_v2_fails_closed_for_missing_or_unsupported_selected_node() {
    let case = ready_case("verified-inventory-v2-missing-node");
    let unsupported = v2_prepared_intent(&case, NisarSubsetFrequency::A, &["VH"]);
    assert_eq!(
        NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
            &unsupported,
            v2_source_validator(&case.source_path),
            &case.source_path,
            &|| false,
        ),
        Err(NisarSubsetWriteError::MetadataInvalid)
    );

    let file = File::open_rw(&case.source_path).unwrap();
    file.unlink("/science/LSAR/RSLC/swaths/frequencyA/HV")
        .unwrap();
    file.flush().unwrap();
    drop(file);
    let missing = v2_prepared_intent(&case, NisarSubsetFrequency::A, &["HH", "HV"]);
    assert_eq!(
        NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
            &missing,
            v2_source_validator(&case.source_path),
            &case.source_path,
            &|| false,
        ),
        Err(NisarSubsetWriteError::MetadataInvalid)
    );
}

#[test]
fn verified_inventory_v2_digest_is_canonical_and_binds_real_attributes() {
    let case = ready_case("verified-inventory-v2-digest");
    let intent = v2_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let original = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        &intent,
        v2_source_validator(&case.source_path),
        &case.source_path,
        &|| false,
    )
    .unwrap();
    let repeated = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        &intent,
        v2_source_validator(&case.source_path),
        &case.source_path,
        &|| false,
    )
    .unwrap();
    assert_eq!(
        original.evidence().inventory_sha256(),
        repeated.evidence().inventory_sha256()
    );

    let file = File::open_rw(&case.source_path).unwrap();
    let dataset = file.dataset(SCIENCE_PATH).unwrap();
    dataset
        .new_attr::<u32>()
        .create("phase2c1DatasetAttribute")
        .unwrap()
        .write_scalar(&42_u32)
        .unwrap();
    file.flush().unwrap();
    drop(dataset);
    drop(file);
    let mutated = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        &intent,
        v2_source_validator(&case.source_path),
        &case.source_path,
        &|| false,
    )
    .unwrap();
    let selected_science = mutated
        .datasets()
        .iter()
        .find(|dataset| dataset.path() == SCIENCE_PATH)
        .unwrap();
    let captured_attribute = selected_science
        .attributes()
        .iter()
        .find(|attribute| attribute.name() == "phase2c1DatasetAttribute")
        .unwrap();
    assert!(captured_attribute.shape().is_empty());
    assert_eq!(captured_attribute.value_sha256().len(), 64);
    assert_ne!(
        original.evidence().inventory_sha256(),
        mutated.evidence().inventory_sha256()
    );
}

#[test]
fn verified_inventory_v2_builds_canonical_actual_node_dependency_graph() {
    let case = ready_case("verified-graph-v2-single-pol");
    let inventory = capture_v2_inventory(&case, NisarSubsetFrequency::A, &["HH"]);

    let graph = super::build_nisar_subset_source_dependency_graph_v2(&inventory).unwrap();
    assert_eq!(
        graph.evidence_authority,
        inventory.evidence().evidence_authority()
    );
    assert_eq!(
        graph.evidence_sha256,
        inventory.evidence().evidence_sha256()
    );
    assert!(graph
        .nodes
        .windows(2)
        .all(|pair| pair[0].source_path < pair[1].source_path));
    assert!(graph.edges.windows(2).all(|pair| {
        (
            &pair[0].dependent_path,
            &pair[0].dependency_path,
            pair[0].relation,
        ) < (
            &pair[1].dependent_path,
            &pair[1].dependency_path,
            pair[1].relation,
        )
    }));
    let inventory_paths = inventory
        .datasets()
        .iter()
        .map(|dataset| dataset.path())
        .chain(inventory.groups().iter().map(|group| group.path()))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(graph
        .nodes
        .iter()
        .all(|node| inventory_paths.contains(node.source_path.as_str())));
    let attribute_owner = graph
        .nodes
        .iter()
        .find(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::Attribute)
        .unwrap();
    assert_eq!(attribute_owner.source_path, "/");
    assert_eq!(
        attribute_owner.attribute_name.as_deref(),
        Some("Conventions")
    );
    assert!(inventory
        .groups()
        .iter()
        .find(|group| group.path() == attribute_owner.source_path)
        .unwrap()
        .attributes()
        .iter()
        .any(|attribute| attribute.name() == "Conventions"));

    let coordinate_x_path = "/science/LSAR/RSLC/metadata/geolocationGrid/coordinateX";
    let coordinate_x = inventory
        .datasets()
        .iter()
        .find(|dataset| dataset.path() == coordinate_x_path)
        .unwrap();
    let attached_scales = coordinate_x
        .dimension_scales()
        .iter()
        .flatten()
        .collect::<std::collections::BTreeSet<_>>();
    assert!(!attached_scales.is_empty());
    for scale_path in &attached_scales {
        assert!(graph.edges.iter().any(|edge| {
            edge.dependent_path == coordinate_x_path
                && edge.dependency_path.as_str() == scale_path.as_str()
                && edge.relation == NisarSubsetDependencyRelationV2::DimensionScale
        }));
    }
    assert!(!graph.edges.iter().any(|edge| {
        edge.dependent_path == SCIENCE_PATH
            && edge.relation == NisarSubsetDependencyRelationV2::DimensionScale
            && edge
                .dependency_path
                .starts_with("/science/LSAR/RSLC/metadata/geolocationGrid/")
    }));
    let graph_scale_paths = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::DimensionScale)
        .map(|node| &node.source_path)
        .collect::<std::collections::BTreeSet<_>>();
    let actual_referenced_scales = graph
        .nodes
        .iter()
        .filter_map(|node| {
            inventory
                .datasets()
                .iter()
                .find(|dataset| dataset.path() == node.source_path)
        })
        .flat_map(|dataset| dataset.dimension_scales().iter().flatten())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(graph_scale_paths, actual_referenced_scales);
    for relation in [
        NisarSubsetDependencyRelationV2::Coordinate,
        NisarSubsetDependencyRelationV2::DimensionScale,
        NisarSubsetDependencyRelationV2::ValidityMask,
        NisarSubsetDependencyRelationV2::RequiredMetadata,
        NisarSubsetDependencyRelationV2::Provenance,
        NisarSubsetDependencyRelationV2::RequiredAttribute,
        NisarSubsetDependencyRelationV2::OtherScientificDependency,
    ] {
        assert!(graph.edges.iter().any(|edge| edge.relation == relation));
    }
    let closure = insar_core::derive_nisar_subset_dependency_closure_v2(
        &graph,
        inventory.selection().selected_frequency(),
        inventory.selection().selected_polarizations(),
    )
    .unwrap();
    for kind in [
        NisarSubsetDependencyKindV2::SelectedScienceDataset,
        NisarSubsetDependencyKindV2::CoordinateDataset,
        NisarSubsetDependencyKindV2::DimensionScale,
        NisarSubsetDependencyKindV2::ValidityMask,
        NisarSubsetDependencyKindV2::IdentificationMetadata,
        NisarSubsetDependencyKindV2::ProvenanceMetadata,
        NisarSubsetDependencyKindV2::Attribute,
        NisarSubsetDependencyKindV2::OtherScientificDependency,
    ] {
        assert!(closure.items.iter().any(|item| item.kind == kind));
    }
}

#[test]
fn verified_inventory_v2_graph_keeps_all_multi_pol_and_frequency_b_roots() {
    let case = ready_case("verified-graph-v2-multi-pol");
    for (frequency, polarizations, expected_roots) in [
        (
            NisarSubsetFrequency::A,
            vec!["HV", "HH"],
            vec![
                "/science/LSAR/RSLC/swaths/frequencyA/HH",
                "/science/LSAR/RSLC/swaths/frequencyA/HV",
            ],
        ),
        (
            NisarSubsetFrequency::B,
            vec!["VV", "HV"],
            vec![B_HV_PATH, B_VV_PATH],
        ),
    ] {
        let inventory = capture_v2_inventory(&case, frequency.clone(), &polarizations);
        let graph = super::build_nisar_subset_source_dependency_graph_v2(&inventory).unwrap();
        let roots = graph
            .nodes
            .iter()
            .filter(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::ScienceDataset)
            .map(|node| node.source_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(roots, expected_roots);
        let (selected_processing, unselected_processing) = match frequency {
            NisarSubsetFrequency::A => (
                "/science/LSAR/RSLC/metadata/processingInformation/parameters/frequencyA/dopplerCentroid",
                "/science/LSAR/RSLC/metadata/processingInformation/parameters/frequencyB/dopplerCentroid",
            ),
            NisarSubsetFrequency::B => (
                "/science/LSAR/RSLC/metadata/processingInformation/parameters/frequencyB/dopplerCentroid",
                "/science/LSAR/RSLC/metadata/processingInformation/parameters/frequencyA/dopplerCentroid",
            ),
        };
        assert!(graph.nodes.iter().any(|node| {
            node.source_path == selected_processing
                && node.kind == NisarSubsetSourceDependencyNodeKindV2::ProvenanceMetadata
        }));
        assert!(!graph
            .nodes
            .iter()
            .any(|node| node.source_path == unselected_processing));
        let closure = insar_core::derive_nisar_subset_dependency_closure_v2(
            &graph,
            &frequency,
            inventory.selection().selected_polarizations(),
        )
        .unwrap();
        assert_eq!(
            closure
                .items
                .iter()
                .filter(|item| item.kind == NisarSubsetDependencyKindV2::SelectedScienceDataset)
                .count(),
            2
        );
    }
}

#[test]
fn verified_inventory_v2_graph_keeps_actual_shared_processing_provenance() {
    let case = ready_case("verified-graph-v2-shared-processing");
    let file = File::open_rw(&case.source_path).unwrap();
    let processing = file
        .group("/science/LSAR/RSLC/metadata/processingInformation")
        .unwrap();
    let shared = processing.create_group("sharedAuthority").unwrap();
    super::write_ascii_scalar::<32>(&shared, "algorithmVersion", "shared-v1").unwrap();
    file.flush().unwrap();
    drop(shared);
    drop(processing);
    drop(file);
    let shared_path =
        "/science/LSAR/RSLC/metadata/processingInformation/sharedAuthority/algorithmVersion";

    for (frequency, polarizations) in [
        (NisarSubsetFrequency::A, &["HH"][..]),
        (NisarSubsetFrequency::B, &["HV"][..]),
    ] {
        let intent = v2_prepared_intent(&case, frequency, polarizations);
        let inventory = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
            &intent,
            v2_source_validator(&case.source_path),
            &case.source_path,
            &|| false,
        )
        .unwrap();
        assert_eq!(
            inventory
                .datasets()
                .iter()
                .find(|dataset| dataset.path() == shared_path)
                .unwrap()
                .kind(),
            NisarSubsetVerifiedMetadataDatasetKindV2::ProcessingMetadata
        );
        let graph = super::build_nisar_subset_source_dependency_graph_v2(&inventory).unwrap();
        assert!(graph.nodes.iter().any(|node| {
            node.source_path == shared_path
                && node.kind == NisarSubsetSourceDependencyNodeKindV2::ProvenanceMetadata
        }));
    }
}

#[test]
fn verified_inventory_v2_graph_order_is_deterministic() {
    let case = ready_case("verified-graph-v2-deterministic");
    let inventory = capture_v2_inventory(&case, NisarSubsetFrequency::B, &["VV", "HV"]);
    assert_eq!(
        super::build_nisar_subset_source_dependency_graph_v2(&inventory).unwrap(),
        super::build_nisar_subset_source_dependency_graph_v2(&inventory).unwrap()
    );
}

#[test]
fn verified_inventory_v2_graph_validator_rejects_unknown_missing_and_orphan_nodes() {
    let case = ready_case("verified-graph-v2-invalid");
    let inventory = capture_v2_inventory(&case, NisarSubsetFrequency::A, &["HH"]);
    let graph = super::build_nisar_subset_source_dependency_graph_v2(&inventory).unwrap();

    let mut unknown = graph.clone();
    unknown
        .nodes
        .iter_mut()
        .find(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::OtherScientificDependency)
        .unwrap()
        .kind = NisarSubsetSourceDependencyNodeKindV2::Unknown;
    assert_eq!(
        super::validate_nisar_subset_source_dependency_graph_v2(&inventory, &unknown),
        Err(super::NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)
    );

    let mut missing = graph.clone();
    missing.edges[0].dependency_path = "/science/LSAR/does-not-exist".to_owned();
    assert_eq!(
        super::validate_nisar_subset_source_dependency_graph_v2(&inventory, &missing),
        Err(super::NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)
    );

    let mut orphan = graph.clone();
    let orphan_path = orphan
        .nodes
        .iter()
        .find(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::OtherScientificDependency)
        .unwrap()
        .source_path
        .clone();
    orphan
        .edges
        .retain(|edge| edge.dependency_path != orphan_path);
    assert_eq!(
        super::validate_nisar_subset_source_dependency_graph_v2(&inventory, &orphan),
        Err(super::NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)
    );

    let actual_scale_edge = graph
        .edges
        .iter()
        .find(|edge| edge.relation == NisarSubsetDependencyRelationV2::DimensionScale)
        .unwrap()
        .clone();
    let mut false_incidence = graph.clone();
    false_incidence
        .edges
        .iter_mut()
        .find(|edge| {
            edge.dependent_path == actual_scale_edge.dependent_path
                && edge.dependency_path == actual_scale_edge.dependency_path
                && edge.relation == actual_scale_edge.relation
        })
        .unwrap()
        .dependent_path = SCIENCE_PATH.to_owned();
    assert_eq!(
        super::validate_nisar_subset_source_dependency_graph_v2(&inventory, &false_incidence),
        Err(super::NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)
    );

    let mut missing_incidence = graph.clone();
    missing_incidence
        .edges
        .retain(|edge| edge != &actual_scale_edge);
    assert_eq!(
        super::validate_nisar_subset_source_dependency_graph_v2(&inventory, &missing_incidence),
        Err(super::NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)
    );

    let mut untyped_scale = graph.clone();
    untyped_scale
        .nodes
        .iter_mut()
        .find(|node| node.source_path == actual_scale_edge.dependency_path)
        .unwrap()
        .kind = NisarSubsetSourceDependencyNodeKindV2::OtherScientificDependency;
    assert_eq!(
        super::validate_nisar_subset_source_dependency_graph_v2(&inventory, &untyped_scale),
        Err(super::NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)
    );

    let mut missing_attribute_name = graph.clone();
    missing_attribute_name
        .nodes
        .iter_mut()
        .find(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::Attribute)
        .unwrap()
        .attribute_name = None;
    assert_eq!(
        super::validate_nisar_subset_source_dependency_graph_v2(
            &inventory,
            &missing_attribute_name,
        ),
        Err(super::NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)
    );

    let mut mutated_attribute_name = graph.clone();
    mutated_attribute_name
        .nodes
        .iter_mut()
        .find(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::Attribute)
        .unwrap()
        .attribute_name = Some("OtherConvention".to_owned());
    assert_eq!(
        super::validate_nisar_subset_source_dependency_graph_v2(
            &inventory,
            &mutated_attribute_name,
        ),
        Err(super::NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)
    );

    let mut wrong_frequency = graph;
    let other_processing =
        "/science/LSAR/RSLC/metadata/processingInformation/parameters/frequencyB/dopplerCentroid";
    wrong_frequency
        .nodes
        .push(insar_core::NisarSubsetSourceDependencyNodeV2 {
            source_path: other_processing.to_owned(),
            kind: NisarSubsetSourceDependencyNodeKindV2::ProvenanceMetadata,
            attribute_name: None,
        });
    wrong_frequency
        .edges
        .push(insar_core::NisarSubsetSourceDependencyEdgeV2 {
            dependent_path: SCIENCE_PATH.to_owned(),
            dependency_path: other_processing.to_owned(),
            relation: NisarSubsetDependencyRelationV2::Provenance,
        });
    assert_eq!(
        super::validate_nisar_subset_source_dependency_graph_v2(&inventory, &wrong_frequency),
        Err(super::NisarSubsetSourceDependencyGraphBuildErrorV2::GraphInvalid)
    );
}

#[test]
fn verified_inventory_v2_graph_digest_tracks_real_inventory_mutation() {
    let case = ready_case("verified-graph-v2-digest-mutation");
    let intent = v2_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let original_inventory =
        NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
            &intent,
            v2_source_validator(&case.source_path),
            &case.source_path,
            &|| false,
        )
        .unwrap();
    let original_graph =
        super::build_nisar_subset_source_dependency_graph_v2(&original_inventory).unwrap();

    let file = File::open_rw(&case.source_path).unwrap();
    let dataset = file.dataset(SCIENCE_PATH).unwrap();
    dataset
        .new_attr::<u32>()
        .create("phase2c1GraphDigestMutation")
        .unwrap()
        .write_scalar(&7_u32)
        .unwrap();
    file.flush().unwrap();
    drop(dataset);
    drop(file);
    let mutated_inventory =
        NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
            &intent,
            v2_source_validator(&case.source_path),
            &case.source_path,
            &|| false,
        )
        .unwrap();
    let mutated_graph =
        super::build_nisar_subset_source_dependency_graph_v2(&mutated_inventory).unwrap();
    assert_ne!(
        original_graph.evidence_sha256,
        mutated_graph.evidence_sha256
    );
    assert_ne!(
        insar_core::derive_nisar_subset_dependency_closure_v2(
            &original_graph,
            original_inventory.selection().selected_frequency(),
            original_inventory.selection().selected_polarizations(),
        )
        .unwrap()
        .source_graph_sha256,
        insar_core::derive_nisar_subset_dependency_closure_v2(
            &mutated_graph,
            mutated_inventory.selection().selected_frequency(),
            mutated_inventory.selection().selected_polarizations(),
        )
        .unwrap()
        .source_graph_sha256
    );
}

#[test]
fn v1_writer_round_trips_unnamed_dimension_scale_without_inventing_name() {
    const HEIGHT: &str = "/science/LSAR/RSLC/metadata/geolocationGrid/heightAboveEllipsoid";
    const COORDINATE_X: &str = "/science/LSAR/RSLC/metadata/geolocationGrid/coordinateX";
    let case = ready_case_with_selection_and_source_mutation(
        "writer-v1-unnamed-scale-round-trip",
        1,
        NisarSubsetFrequency::A,
        "HH",
        SCIENCE_PATH,
        MASK_PATH,
        false,
        |file| {
            let height = file.dataset(HEIGHT).unwrap();
            assert_eq!(
                super::dimension_scales::remove_scale_name_for_test(&height),
                Ok(())
            );
        },
    );
    let result = writer(&case._root.0.join("writer-v1-unnamed-scale-output"))
        .write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        )
        .unwrap();
    let output = File::open(result.artifact_path()).unwrap();
    let height = output.dataset(HEIGHT).unwrap();
    let height_graph = super::read_dimension_scale_graph(&height).unwrap();
    assert!(height_graph.3);
    assert_eq!(height_graph.2, None);
    assert!(height.attr("NAME").is_err());
    let coordinate_graph =
        super::read_dimension_scale_graph(&output.dataset(COORDINATE_X).unwrap()).unwrap();
    assert_eq!(coordinate_graph.1[0], [HEIGHT]);
}

#[test]
fn unix_published_artifact_link_contract_follows_proc_fd_without_empty_path() {
    let contract = super::v2_unix_publish_link_contract_for_test();
    assert_eq!(contract.source_prefix, "/proc/self/fd/");
    assert_ne!(contract.flags & contract.at_symlink_follow, 0);
    assert_eq!(contract.flags & contract.at_empty_path, 0);

    let source = include_str!("nisar_subset_writer.rs");
    let start = source
        .find("fn publish_unnamed_v2_artifact(")
        .expect("Linux/Android publication helper must exist");
    let end = source[start..]
        .find("fn promote_validated_hdf5_core(")
        .map(|offset| start + offset)
        .expect("publication helper audit boundary must exist");
    let helper = &source[start..end];
    assert!(helper.contains("libc::AT_FDCWD"));
    assert!(helper.contains("V2_PUBLISH_LINK_FLAGS"));
    assert!(!helper.contains("libc::AT_EMPTY_PATH"));
}

#[test]
fn verified_inventory_v2_unnamed_scale_remains_dimension_scale_graph_evidence() {
    let case = ready_case("verified-graph-v2-unnamed-scale");
    let file = File::open_rw(&case.source_path).unwrap();
    let unnamed_path = "/science/LSAR/RSLC/metadata/geolocationGrid/heightAboveEllipsoid";
    let unnamed = file.dataset(unnamed_path).unwrap();
    assert_eq!(
        super::dimension_scales::remove_scale_name_for_test(&unnamed),
        Ok(())
    );
    file.flush().unwrap();
    drop(unnamed);
    drop(file);

    let intent = v2_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let inventory = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        &intent,
        v2_source_validator(&case.source_path),
        &case.source_path,
        &|| false,
    )
    .unwrap();
    let dataset = inventory
        .datasets()
        .iter()
        .find(|dataset| dataset.path() == unnamed_path)
        .unwrap();
    assert_eq!(dataset.scale_name(), None);
    assert_eq!(
        dataset.kind(),
        NisarSubsetVerifiedMetadataDatasetKindV2::DimensionScale
    );
    let graph = super::build_nisar_subset_source_dependency_graph_v2(&inventory).unwrap();
    assert!(graph.nodes.iter().any(|node| {
        node.source_path == unnamed_path
            && node.kind == NisarSubsetSourceDependencyNodeKindV2::DimensionScale
    }));
}

#[test]
fn verified_inventory_v2_graph_requires_exact_root_conventions_authority() {
    let case = ready_case("verified-graph-v2-root-conventions");
    let mut inventory = capture_v2_inventory(&case, NisarSubsetFrequency::A, &["HH"]);
    let root = inventory
        .groups
        .iter_mut()
        .find(|group| group.path == "/")
        .unwrap();
    root.attributes
        .retain(|attribute| attribute.name != "Conventions");
    assert_eq!(
        super::build_nisar_subset_source_dependency_graph_v2(&inventory),
        Err(
            super::NisarSubsetSourceDependencyGraphBuildErrorV2::MissingDependency(
                "verified root Conventions attribute owner".to_owned()
            )
        )
    );
}

#[test]
fn verified_inventory_v2_graph_maps_valid_samples_and_shared_selected_metadata_only() {
    let case = ready_case("verified-graph-v2-complete-kind-mapping");
    let inventory = capture_v2_inventory(&case, NisarSubsetFrequency::A, &["HH", "HV"]);
    assert_eq!(inventory.validity_mask_paths(), [MASK_PATH]);
    let valid_samples = "/science/LSAR/RSLC/swaths/frequencyA/validSamplesSubSwath1";
    assert_eq!(inventory.valid_samples_paths(), [valid_samples]);
    assert_eq!(
        inventory
            .datasets()
            .iter()
            .find(|dataset| dataset.path() == valid_samples)
            .unwrap()
            .kind(),
        NisarSubsetVerifiedMetadataDatasetKindV2::ValidSamples
    );

    let graph = super::build_nisar_subset_source_dependency_graph_v2(&inventory).unwrap();
    assert!(graph.nodes.iter().any(|node| {
        node.source_path == valid_samples
            && node.kind == NisarSubsetSourceDependencyNodeKindV2::OtherScientificDependency
    }));
    let selected_processing =
        "/science/LSAR/RSLC/metadata/processingInformation/parameters/frequencyA/dopplerCentroid";
    let unselected_processing =
        "/science/LSAR/RSLC/metadata/processingInformation/parameters/frequencyB/dopplerCentroid";
    assert!(graph
        .nodes
        .iter()
        .any(|node| node.source_path == selected_processing));
    assert!(inventory
        .datasets()
        .iter()
        .any(|dataset| dataset.path() == unselected_processing));
    assert!(!graph
        .nodes
        .iter()
        .any(|node| node.source_path == unselected_processing));
    let shared_other = inventory
        .datasets()
        .iter()
        .find(|dataset| {
            dataset.kind() == NisarSubsetVerifiedMetadataDatasetKindV2::OtherMetadata
                && !dataset.path().contains("/frequencyA/")
                && !dataset.path().contains("/frequencyB/")
        })
        .unwrap();
    assert!(graph
        .nodes
        .iter()
        .any(|node| node.source_path == shared_other.path()));
}

#[test]
fn verified_inventory_v2_full_evidence_digest_binds_validator_size_count_and_authority() {
    let case = ready_case("verified-graph-v2-full-evidence");
    let intent = v2_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let original = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        &intent,
        v2_source_validator(&case.source_path),
        &case.source_path,
        &|| false,
    )
    .unwrap();
    let mut changed_validator = v2_source_validator(&case.source_path);
    changed_validator.value = "\"different-strong-etag\"".to_owned();
    let changed = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        &intent,
        changed_validator,
        &case.source_path,
        &|| false,
    )
    .unwrap();
    assert_eq!(
        original.evidence().inventory_sha256(),
        changed.evidence().inventory_sha256()
    );
    assert_ne!(
        original.evidence().evidence_sha256(),
        changed.evidence().evidence_sha256()
    );
    assert_ne!(
        super::build_nisar_subset_source_dependency_graph_v2(&original)
            .unwrap()
            .evidence_sha256,
        super::build_nisar_subset_source_dependency_graph_v2(&changed)
            .unwrap()
            .evidence_sha256
    );

    let evidence = original.evidence();
    let baseline = evidence.evidence_sha256();
    assert_ne!(
        baseline,
        super::verified_metadata_evidence_sha256(
            evidence.inventory_sha256(),
            evidence.prepared_intent_sha256(),
            evidence.source_validator(),
            evidence.source_size() + 1,
            evidence.source_dataset_count(),
            evidence.evidence_authority(),
        )
        .unwrap()
    );
    assert_ne!(
        baseline,
        super::verified_metadata_evidence_sha256(
            evidence.inventory_sha256(),
            evidence.prepared_intent_sha256(),
            evidence.source_validator(),
            evidence.source_size(),
            evidence.source_dataset_count() + 1,
            evidence.evidence_authority(),
        )
        .unwrap()
    );
    assert_ne!(
        baseline,
        super::verified_metadata_evidence_sha256(
            evidence.inventory_sha256(),
            evidence.prepared_intent_sha256(),
            evidence.source_validator(),
            evidence.source_size(),
            evidence.source_dataset_count(),
            NisarSubsetDependencyEvidenceAuthorityV2::ReviewedSourceEvidenceV1,
        )
        .unwrap()
    );
}

#[test]
fn v1_metadata_adapter_matches_pre_refactor_golden_identity() {
    let case = ready_case("verified-inventory-v1-parity");
    // HDF5 object timestamps make the fixture's complete-file checksum vary by
    // process. The live scanner intentionally does not hash the sparse source,
    // so freeze that request field before comparing the complete V1 identity.
    let mut request = case.request.clone();
    request.source.checksum.as_mut().unwrap().value = "a".repeat(64);
    request.validate().unwrap();
    let metadata = NisarSubsetMetadataSnapshotV1::from_verified_live_metadata_hdf5(
        &request,
        v2_source_validator(&case.source_path),
        &case.source_path,
        &|| false,
    )
    .unwrap();
    // These literals were measured twice against the unmodified pre-Task1
    // scanner in a detached worktree carrying the same Phase 2B.1 dirty state.
    assert_eq!(
        metadata.source_inventory_sha256(),
        "4250fd0307fae41a4e380d6a6f18b686318b56a2c684f9d4a49c68efaa8847c9"
    );
    assert_eq!(
        metadata.evidence_sha256(),
        "8aca8354732afe39da3613c357b2a5ac850c0e4284bd760a26fd8c3954c10038"
    );
    assert_eq!(metadata.source_dataset_count(), 207);
    assert_eq!(metadata.metadata_group_binding_count(), 19);
    assert_eq!(metadata.metadata_dataset_binding_count(), 199);
    assert_eq!(metadata.metadata_binding_count(), 218);
}

fn source_chunk_evidence(
    path: &Path,
    science_path: &str,
    mask_path: &str,
) -> Vec<Hdf5ChunkPhysicalRangeV1> {
    let file = hdf5_fixture_file_builder().open(path).unwrap();
    let mut chunks = Vec::new();
    for (dataset_path, datatype, element_size) in [
        (
            science_path,
            Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
            8_u64,
        ),
        (mask_path, Hdf5ChunkDatatypeV1::UnsignedU8, 1_u64),
    ] {
        let dataset = file.dataset(dataset_path).unwrap();
        assert!(dataset.filters().is_empty());
        let shape = dataset.shape();
        let chunk_shape = dataset.dcpl().unwrap().get_chunk().unwrap().unwrap();
        let (datatype_sha256, fill_value_definition, fill_value_hex, _filter_pipeline) =
            qualify_source_dataset_identity(&dataset, datatype).unwrap();
        for index in 0..dataset.num_chunks().unwrap() {
            let info = dataset.chunk_info(index).unwrap();
            if info.offset[0] >= 3
                || info.offset[0] + chunk_shape[0] as u64 <= 1
                || info.offset[1] >= 3
                || info.offset[1] + chunk_shape[1] as u64 <= 1
            {
                continue;
            }
            assert_eq!(
                info.size,
                (chunk_shape[0] * chunk_shape[1]) as u64 * element_size
            );
            chunks.push(Hdf5ChunkPhysicalRangeV1 {
                dataset_path: dataset_path.to_owned(),
                component_id: 0,
                logical_chunk: [
                    info.offset[0] / chunk_shape[0] as u64,
                    info.offset[1] / chunk_shape[1] as u64,
                ],
                dataset_shape: [shape[0] as u64, shape[1] as u64],
                chunk_shape: [chunk_shape[0] as u64, chunk_shape[1] as u64],
                datatype,
                datatype_sha256: datatype_sha256.clone(),
                fill_value_definition,
                fill_value_hex: fill_value_hex.clone(),
                filter_pipeline: Hdf5ChunkFilterPipelineV1::None,
                allocation: Hdf5ChunkAllocationV1::Allocated {
                    filter_mask: info.filter_mask,
                    stored_size: info.size,
                    start: info.addr,
                    end_exclusive: info.addr + info.size,
                },
            });
        }
    }
    file.close().unwrap();
    chunks
}

fn request(
    source_size: u64,
    remote_bytes: u64,
    source_bytes: &[u8],
) -> insar_core::NisarSubsetRequestManifestV1 {
    let path = format!(
        "{}/../../docs/fixtures/asf-nisar-subset-p1/planner-request-v1.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut request: insar_core::NisarSubsetRequestManifestV1 =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    request.source.expected_size_bytes = source_size;
    request.source.checksum.as_mut().unwrap().value = super::sha256_bytes(source_bytes);
    request.expected_output.full_product_bytes = source_size;
    request.expected_output.estimated_remote_bytes = remote_bytes;
    request.expected_output.estimated_output_bytes = 1;
    let window = &mut request.mapping.windows[0];
    window.line_start = 1;
    window.line_end_exclusive = 3;
    window.sample_start = 1;
    window.sample_end_exclusive = 3;
    window.source_shape = [4, 4];
    window.expected_shape = [2, 2];
    window.chunk_shape = [2, 2];
    request.validate().unwrap();
    request
}

pub fn snapshot(request: &insar_core::NisarSubsetRequestManifestV1) -> SnapshotItem {
    let checksum = request.source.checksum.as_ref().unwrap();
    SnapshotItem {
        item_id: request.source.provider_product_key.clone(),
        provider_id: request.source.provider_id.clone(),
        display_name: request.source.file_name.trim_end_matches(".h5").to_owned(),
        acquisition_datetime: Some("2026-07-26T15:58:03Z".to_owned()),
        footprint_geojson: Some(
            json!({
                "type": "Polygon",
                "coordinates": [[[36.0, 9.0], [39.0, 9.0], [39.0, 12.0], [36.0, 12.0], [36.0, 9.0]]]
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        metadata: Map::from_iter([
            ("band".to_owned(), json!("L")),
            ("processing_level".to_owned(), json!("RSLC")),
            ("science_level".to_owned(), json!("L1")),
            ("production_configuration".to_owned(), json!("PR")),
            ("crid".to_owned(), json!(request.source.crid)),
            (
                "product_specification_version".to_owned(),
                json!(request.source.product_specification_version),
            ),
            (
                "collection_profile_id".to_owned(),
                json!(request.source.collection_profile_id),
            ),
            ("frequency_a_polarization".to_owned(), json!("HH+HV")),
            ("frequency_b_polarization".to_owned(), json!("HH+HV+VV")),
        ]),
        payload: Map::new(),
        asset_access: Some(FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: request.source.provider_id.clone(),
                collection_id: request.source.collection_id.clone(),
                provider_product_id: request.source.provider_product_id.clone(),
                provider_product_key: request.source.provider_product_key.clone(),
                canonical_product_key: None,
                asset_key: "science-rslc-hdf5".to_owned(),
                stable_asset_key: request.source.stable_asset_key.clone(),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: "earthdata_https".to_owned(),
                scheme: AssetAccessScheme::EarthdataHttps,
                stable_href: None,
                requires_credentials: false,
                credential_profile: None,
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some(request.source.file_name.clone()),
            expected_size_bytes: Some(request.source.expected_size_bytes),
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: Some(checksum.value.clone()),
        }),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: "asf.nisar.earthdata-complete-hdf5.v2".to_owned(),
            collection_profile_id: Some(request.source.collection_profile_id.clone()),
        }),
    }
}

fn resolved_asset(url: &str, request: &insar_core::NisarSubsetRequestManifestV1) -> ResolvedAsset {
    let checksum = request.source.checksum.as_ref().unwrap();
    ResolvedAsset::new(url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(request.source.expected_size_bytes))
        .with_checksum(Some(
            ResolvedChecksum::new("sha256", &checksum.value).unwrap(),
        ))
        .with_remote_validator(RemoteValidator {
            etag: Some(ETAG.to_owned()),
            last_modified: None,
            content_length: Some(request.source.expected_size_bytes),
        })
}

#[derive(Clone)]
struct DurableWriterRecoveryAnchorFixtureV2 {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl DurableWriterRecoveryAnchorFixtureV2 {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Arc::new(Mutex::new(())),
        }
    }

    fn load_file(
        path: &Path,
    ) -> Result<Option<super::NisarSubsetWriterRecoveryAnchorRecordV2>, NisarSubsetWriteError> {
        match fs::read(path) {
            Ok(bytes) if bytes.len() <= 1024 * 1024 => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| NisarSubsetWriteError::RecoveryInvalid),
            Ok(_) => Err(NisarSubsetWriteError::RecoveryInvalid),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(NisarSubsetWriteError::RecoveryInvalid),
        }
    }
}

impl super::writer_recovery_anchor_sealed::Sealed for DurableWriterRecoveryAnchorFixtureV2 {}

impl super::NisarSubsetWriterRecoveryAnchorV2 for DurableWriterRecoveryAnchorFixtureV2 {
    fn load(
        &self,
        lineage_key_sha256: &str,
    ) -> Result<Option<super::NisarSubsetWriterRecoveryAnchorRecordV2>, NisarSubsetWriteError> {
        let _guard = self.lock.lock().unwrap();
        let record = Self::load_file(&self.path)?;
        if record
            .as_ref()
            .is_some_and(|value| value.lineage_key_sha256() != lineage_key_sha256)
        {
            return Err(NisarSubsetWriteError::RecoveryInvalid);
        }
        Ok(record)
    }

    fn compare_and_swap(
        &self,
        lineage_key_sha256: &str,
        expected: Option<&super::NisarSubsetWriterRecoveryAnchorRecordV2>,
        replacement: &super::NisarSubsetWriterRecoveryAnchorRecordV2,
    ) -> Result<bool, NisarSubsetWriteError> {
        let _guard = self.lock.lock().unwrap();
        let current = Self::load_file(&self.path)?;
        if current.as_ref() != expected
            || replacement.lineage_key_sha256() != lineage_key_sha256
            || expected.is_some_and(|value| value.lineage_key_sha256() != lineage_key_sha256)
        {
            return Ok(false);
        }
        let canonical =
            serde_json::to_vec(replacement).map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?
            .as_nanos();
        let temp = self.path.with_extension(format!("anchor-{nonce}.next"));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        file.write_all(&canonical)
            .and_then(|_| file.sync_all())
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        drop(file);
        let verified = super::VerifiedClosedArtifact::after_scientific_validation(
            temp,
            super::sha256_bytes(&canonical),
            &|| false,
        )
        .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        super::promote_verified_closed_artifact(verified, &self.path, None)
            .map_err(|_| NisarSubsetWriteError::RecoveryInvalid)?;
        Ok(true)
    }
}

pub(crate) fn reset_regular_building_recovery_hits_for_test() {
    super::V2_REGULAR_BUILDING_RECOVERY_HITS.store(0, Ordering::Relaxed);
}

pub(crate) fn regular_building_recovery_hits_for_test() -> usize {
    super::V2_REGULAR_BUILDING_RECOVERY_HITS.load(Ordering::Relaxed)
}

pub(crate) fn recovery_anchor_state_for_test(root: &Path) -> Option<&'static str> {
    let path = root.join(".nisar-subset-writer-v2-recovery-anchor.json");
    DurableWriterRecoveryAnchorFixtureV2::load_file(&path)
        .ok()
        .flatten()
        .map(|record| match record.state {
            super::NisarSubsetWriterRecoveryStateV2::CandidateBuilding => "CandidateBuilding",
            super::NisarSubsetWriterRecoveryStateV2::CandidateValidated => "CandidateValidated",
            super::NisarSubsetWriterRecoveryStateV2::ArtifactPromoted => "ArtifactPromoted",
            super::NisarSubsetWriterRecoveryStateV2::Complete => "Complete",
        })
}

pub(crate) fn recovery_anchor_has_distinct_promoted_identity_for_test(root: &Path) -> bool {
    let path = root.join(".nisar-subset-writer-v2-recovery-anchor.json");
    DurableWriterRecoveryAnchorFixtureV2::load_file(&path)
        .ok()
        .flatten()
        .is_some_and(|record| {
            matches!(
                record.state,
                super::NisarSubsetWriterRecoveryStateV2::ArtifactPromoted
                    | super::NisarSubsetWriterRecoveryStateV2::Complete
            ) && record
                .artifact_identity
                .as_ref()
                .is_some_and(|identity| !record.candidate_identity.same_object(identity))
        })
}

pub fn writer(root: &Path) -> OfflineNisarSubsetWriter {
    // TempDirectory owns the process-global HDF5 test lock for the complete
    // fixture lifetime, including this writer operation.
    crate::establish_hdf5_plugin_isolation()
        .expect("app-owned HDF5 plugin isolation is required by the writer tests");
    let writer = OfflineNisarSubsetWriter::new(
        root.to_path_buf(),
        DestinationLeaseController::new_for_qualification_test(),
    )
    .unwrap();
    writer.with_writer_recovery_anchor_v2(Arc::new(DurableWriterRecoveryAnchorFixtureV2::new(
        root.join(".nisar-subset-writer-v2-recovery-anchor.json"),
    )))
}

/// A writer over `root` with its own durable recovery anchor at `anchor_path`
/// and a caller-shared destination lease: several items of one process (one
/// task or several) writing into the same output directory.
#[cfg(test)]
pub(crate) fn writer_with_anchor(
    root: &Path,
    anchor_path: &Path,
    destination_lease: Arc<DestinationLeaseController>,
) -> OfflineNisarSubsetWriter {
    crate::establish_hdf5_plugin_isolation()
        .expect("app-owned HDF5 plugin isolation is required by the writer tests");
    OfflineNisarSubsetWriter::new(root.to_path_buf(), destination_lease)
        .unwrap()
        .with_writer_recovery_anchor_v2(Arc::new(DurableWriterRecoveryAnchorFixtureV2::new(
            anchor_path.to_path_buf(),
        )))
}

#[test]
fn official_single_scene_facts_map_to_the_writer_contract() {
    let evidence: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/fixtures/asf-nisar-subset-p0/official-single-scene-result.json"
    )))
    .unwrap();
    assert_eq!(
        evidence["metadata"]["source_dataset_path_count"],
        serde_json::json!(super::FIRST_SCENE_SOURCE_DATASET_COUNT)
    );
    assert_eq!(
        evidence["science"]["datasets"],
        serde_json::json!([
            "frequencyA/HH",
            "frequencyA/HV",
            "frequencyB/HH",
            "frequencyB/HV"
        ])
    );
    assert_eq!(
        evidence["science"]["qa_masks"],
        serde_json::json!([
            "frequencyA/inputDataExceptionMask",
            "frequencyB/inputDataExceptionMask"
        ])
    );
    assert_eq!(
        evidence["metadata"]["dimension_scales_verified"],
        serde_json::json!(true)
    );
    let case = ready_case("official-facts");
    assert_eq!(
        source_dataset_count(&File::open(&case.source_path).unwrap()),
        super::FIRST_SCENE_SOURCE_DATASET_COUNT
    );
    assert_eq!(
        case.metadata
            .dataset_bindings
            .iter()
            .filter(|binding| binding.action == super::SourceDatasetActionV1::SelectedScience)
            .count(),
        1
    );
    assert_eq!(case.metadata.evidence.geolocation_shape, [3, 2, 2]);
    assert_eq!(case.metadata.evidence.height_above_ellipsoid.len(), 3);
    assert_eq!(
        super::source_mask_path(&case.request.request.frequency),
        MASK_PATH
    );
}

#[test]
fn loopback_blocks_write_a_client_derived_scientifically_equivalent_artifact() {
    let case = ready_case("equivalence");
    let output_root = case._root.0.join("output");
    let result = writer(&output_root)
        .write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        )
        .unwrap();
    assert_eq!(result.manifest().output_authority, "client_derived");
    assert!(result.manifest().complete);
    result
        .manifest()
        .validate_against_request_and_transfer(&case.request, case.transfer.manifest())
        .unwrap();
    let output = File::open(result.artifact_path()).unwrap();
    let science = output
        .dataset(OUTPUT_SCIENCE_PATH)
        .unwrap()
        .read_raw::<CFloat32>()
        .unwrap();
    let mask = output.dataset(OUTPUT_MASK_PATH).unwrap();
    assert_eq!(mask.dcpl().unwrap().get_chunk().unwrap().unwrap(), [1, 2]);
    let mask = mask.read_raw::<u8>().unwrap();
    let source = File::open(&case.source_path).unwrap();
    let source_science = source
        .dataset(SCIENCE_PATH)
        .unwrap()
        .read_raw::<CFloat32>()
        .unwrap();
    let source_mask = source.dataset(MASK_PATH).unwrap().read_raw::<u8>().unwrap();
    let selected_science = [5_usize, 6, 9, 10]
        .into_iter()
        .map(|index| source_science[index])
        .collect::<Vec<_>>();
    let selected_mask = [7_usize, 8, 13, 14]
        .into_iter()
        .map(|index| source_mask[index])
        .collect::<Vec<_>>();
    assert_eq!(science, selected_science);
    assert_eq!(mask, selected_mask);
    assert_eq!(
        science,
        vec![
            CFloat32 { r: 5.25, i: -5.5 },
            CFloat32 { r: 6.25, i: -6.5 },
            CFloat32 { r: 9.25, i: -9.5 },
            CFloat32 { r: 10.25, i: -10.5 },
        ]
    );
    assert_eq!(mask, vec![7, 8, 13, 14]);
    assert_eq!(
        output
            .dataset(SCIENCE_PATH)
            .unwrap()
            .read_raw::<CFloat32>()
            .unwrap(),
        selected_science
    );
    assert_eq!(
        output.dataset(MASK_PATH).unwrap().read_raw::<u8>().unwrap(),
        selected_mask
    );
    assert_eq!(
        output
            .dataset("/science/LSAR/RSLC/swaths/zeroDopplerTime")
            .unwrap()
            .shape(),
        [2]
    );
    assert_eq!(
        output
            .dataset("/science/LSAR/RSLC/swaths/frequencyA/slantRange")
            .unwrap()
            .shape(),
        [2]
    );
    assert_eq!(
        output
            .dataset("/science/LSAR/RSLC/swaths/frequencyA/validSamplesSubSwath1")
            .unwrap()
            .read_raw::<u32>()
            .unwrap(),
        vec![0, 1, 0, 1]
    );
    let coordinate_x = output
        .dataset("/science/LSAR/RSLC/metadata/geolocationGrid/coordinateX")
        .unwrap();
    assert_eq!(coordinate_x.shape(), [3, 2, 2]);
    assert_eq!(
        coordinate_x.filters(),
        vec![
            hdf5::filters::Filter::Shuffle,
            hdf5::filters::Filter::Deflate(6)
        ]
    );
    let graph = super::read_dimension_scale_graph(&coordinate_x).unwrap();
    assert_eq!(
        graph.1,
        vec![
            vec!["/science/LSAR/RSLC/metadata/geolocationGrid/heightAboveEllipsoid".to_owned()],
            vec!["/science/LSAR/RSLC/metadata/geolocationGrid/zeroDopplerTime".to_owned()],
            vec!["/science/LSAR/RSLC/metadata/geolocationGrid/slantRange".to_owned()],
        ]
    );
    let identification = output.group("/science/LSAR/identification").unwrap();
    assert_eq!(
        super::read_fixed_ascii_dataset(&identification.dataset("granuleId").unwrap()).unwrap(),
        case.request.expected_output.derived_artifact_id
    );
    assert_eq!(
        identification
            .dataset("isFullFrame")
            .unwrap()
            .read_scalar::<u8>()
            .unwrap(),
        0
    );
    assert_ne!(
        super::read_fixed_ascii_dataset(&identification.dataset("boundingPolygon").unwrap())
            .unwrap(),
        "POLYGON ((36 9, 39 9, 39 12, 36 12, 36 9))"
    );
    assert!(source_dataset_count(&output) > super::FIRST_SCENE_SOURCE_DATASET_COUNT);
    assert_eq!(fs::read(&case.source_path).unwrap(), case.source_before);
    let sidecar = fs::read_to_string(result.manifest_path()).unwrap();
    assert!(!sidecar.contains("http://"));
    assert!(!sidecar.contains("Authorization"));
}

#[test]
fn writer_never_reopens_the_complete_source_after_metadata_capture() {
    let mut case = ready_case("detached-metadata");
    let _ = case._source_pin.take();
    fs::remove_file(&case.source_path).unwrap();
    let output_root = case._root.0.join("detached-output");
    let result = writer(&output_root)
        .write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        )
        .unwrap();
    assert!(result.manifest().complete);
    assert_eq!(
        result.manifest().remote_bytes,
        case.transfer.manifest().received_bytes
    );
}

#[test]
fn official_multi_frequency_inventory_omits_unselected_science_payload() {
    let case = ready_case("official-multi-frequency");
    let output_root = case._root.0.join("official-multi-output");
    let result = writer(&output_root)
        .write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        )
        .unwrap();
    let artifact = File::open(result.artifact_path()).unwrap();
    assert!(artifact.dataset(SCIENCE_PATH).is_ok());
    assert!(artifact
        .dataset("/science/LSAR/RSLC/swaths/frequencyA/HV")
        .is_err());
    assert!(artifact
        .group("/science/LSAR/RSLC/swaths/frequencyB")
        .is_err());
}

#[test]
fn selected_frequency_b_hv_writes_science_and_frequency_level_mask() {
    let case = ready_case_with_selection(
        "frequency-b-hv",
        1,
        NisarSubsetFrequency::B,
        "HV",
        B_HV_PATH,
        B_MASK_PATH,
    );
    let output_root = case._root.0.join("frequency-b-hv-output");
    let result = writer(&output_root)
        .write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        )
        .unwrap();
    let artifact = File::open(result.artifact_path()).unwrap();
    assert!(artifact
        .dataset("/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyB/HV",)
        .is_ok());
    assert!(artifact
        .dataset(
            "/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyB/inputDataExceptionMask",
        )
        .is_ok());
    assert!(artifact.dataset(B_HV_PATH).is_ok());
    assert!(artifact.dataset(B_MASK_PATH).is_ok());
}

#[test]
fn multi_component_windows_share_physical_ranges_but_produce_separate_outputs() {
    let case = ready_case_with_components("multi-component", 2);
    assert!(!case.plan.ranges().is_empty());
    assert!(case
        .plan
        .ranges()
        .iter()
        .all(|range| range.member_chunk_indices().len().is_multiple_of(2)));
    assert_eq!(
        case.plan
            .ranges()
            .iter()
            .map(|range| range.member_chunk_indices().len())
            .sum::<usize>(),
        case.plan.chunks().len()
    );
    let (workspace_lock, blocks) = case
        .workspace
        .acquire_verified_chunk_blocks(&case.request, &case.plan, &case.transfer, &|| false)
        .unwrap();
    let science = blocks
        .iter()
        .filter(|block| block.dataset_path == SCIENCE_PATH && block.logical_chunk == [0, 0])
        .collect::<Vec<_>>();
    assert_eq!(science.len(), 2);
    let (
        VerifiedHdf5ChunkAllocationV1::Allocated { bytes: left },
        VerifiedHdf5ChunkAllocationV1::Allocated { bytes: right },
    ) = (&science[0].allocation, &science[1].allocation)
    else {
        panic!("multi-component science blocks must be allocated")
    };
    assert!(Arc::ptr_eq(left, right));
    drop(blocks);
    drop(workspace_lock);
    let output_root = case._root.0.join("multi-component-output");
    let result = writer(&output_root)
        .write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        )
        .unwrap();
    let output = File::open(result.artifact_path()).unwrap();
    for component in ["component000", "component001"] {
        let path = format!(
            "/science/LSAR/RSLC/derivedSubsets/{component}/window00{}/swaths/frequencyA/HH",
            if component == "component000" { 0 } else { 1 }
        );
        assert_eq!(output.dataset(&path).unwrap().shape(), [2, 2]);
    }
}

#[test]
fn validation_failures_never_promote_or_emit_a_complete_manifest() {
    let case = ready_case("validation-failures");
    for (label, failpoint) in [
        ("science", WriterFailpoint::CorruptScienceBeforeValidation),
        ("mask", WriterFailpoint::RemoveMaskBeforeValidation),
        ("metadata", WriterFailpoint::CorruptMetadataBeforeValidation),
        (
            "identification",
            WriterFailpoint::CorruptIdentificationBeforeValidation,
        ),
        (
            "attribute",
            WriterFailpoint::AddUnexpectedAttributeBeforeValidation,
        ),
        (
            "science-shape",
            WriterFailpoint::CorruptScienceShapeBeforeValidation,
        ),
        (
            "mask-chunk-geometry",
            WriterFailpoint::CorruptMaskChunkGeometryBeforeValidation,
        ),
    ] {
        let output_root = case._root.0.join(format!("invalid-{label}"));
        let writer = writer(&output_root);
        let failpoint_result = writer.write_with_failpoint_for_test(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
            failpoint,
        );

        assert!(matches!(
            failpoint_result,
            Err(NisarSubsetWriteError::ValidationFailed)
        ));
        assert!(!output_root
            .join(&case.request.expected_output.file_name)
            .exists());
        assert!(!output_root
            .join(format!(
                "{}.artifact-manifest-v1.json",
                case.request.expected_output.file_name
            ))
            .exists());
    }
}

#[test]
fn candidate_path_replacement_after_validation_never_promotes() {
    let case = ready_case("candidate-identity");
    let output_root = case._root.0.join("candidate-identity-output");
    let result = writer(&output_root).write_with_failpoint_for_test(
        &case.request,
        &case.plan,
        &case.transfer,
        &case.workspace,
        &case.metadata,
        &|| false,
        WriterFailpoint::ReplaceCandidateAfterValidation,
    );
    assert!(result.is_err());
    assert!(!output_root
        .join(&case.request.expected_output.file_name)
        .exists());
    assert!(!output_root
        .join(format!(
            "{}.artifact-manifest-v1.json",
            case.request.expected_output.file_name
        ))
        .exists());
}

#[test]
fn restart_never_treats_staging_as_complete_and_recovers_post_promotion_gap() {
    let case = ready_case("recovery");
    let staging_root = case._root.0.join("staging-restart");
    let staging_writer = writer(&staging_root);
    assert!(matches!(
        staging_writer.write_with_failpoint_for_test(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
            WriterFailpoint::AfterWriteBeforeValidation,
        ),
        Err(NisarSubsetWriteError::InjectedFailure)
    ));
    assert!(!staging_root
        .join(&case.request.expected_output.file_name)
        .exists());
    let completed = staging_writer
        .write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        )
        .unwrap();
    assert!(completed.manifest().complete);

    let promotion_root = case._root.0.join("promotion-restart");
    let promotion_writer = writer(&promotion_root);
    assert!(matches!(
        promotion_writer.write_with_failpoint_for_test(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
            WriterFailpoint::AfterArtifactPromotion,
        ),
        Err(NisarSubsetWriteError::InjectedFailure)
    ));
    assert!(promotion_root
        .join(&case.request.expected_output.file_name)
        .is_file());
    let sidecar = promotion_root.join(format!(
        "{}.artifact-manifest-v1.json",
        case.request.expected_output.file_name
    ));
    assert!(!sidecar.exists());
    let recovered = promotion_writer
        .write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        )
        .unwrap();
    assert!(recovered.manifest().complete);
    assert!(sidecar.is_file());
}

#[test]
fn tampered_transfer_part_is_rejected_before_writer_output() {
    let case = ready_case("tampered-part");
    let range = &case.transfer.manifest().ranges[0];
    let mut part = fs::OpenOptions::new()
        .write(true)
        .open(case.workspace.part_path())
        .unwrap();
    part.seek(SeekFrom::Start(range.start)).unwrap();
    part.write_all(b"X").unwrap();
    part.sync_all().unwrap();
    drop(part);
    let output_root = case._root.0.join("tampered-output");
    assert!(matches!(
        writer(&output_root).write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        ),
        Err(NisarSubsetWriteError::BlockEvidenceInvalid)
    ));
    assert!(!output_root
        .join(&case.request.expected_output.file_name)
        .exists());
}

#[test]
fn metadata_digest_and_cancellation_are_bound_before_any_output() {
    let case = ready_case("metadata-cancel");
    assert!(matches!(
        NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
            &case.request,
            NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: ETAG.to_owned(),
                content_length: case.request.source.expected_size_bytes,
            },
            &case.source_path,
            &|| true,
        ),
        Err(NisarSubsetWriteError::Cancelled)
    ));
    let changed_path = case._root.0.join("changed-source.h5");
    fs::copy(&case.source_path, &changed_path).unwrap();
    let changed_file = File::open_rw(&changed_path).unwrap();
    let doppler = changed_file
        .dataset(
            "/science/LSAR/RSLC/metadata/processingInformation/parameters/frequencyA/dopplerCentroid",
        )
        .unwrap();
    let mut values = doppler.read_raw::<f32>().unwrap();
    values[0] += 1.0;
    doppler.write_raw(&values).unwrap();
    changed_file.flush().unwrap();
    drop(doppler);
    drop(changed_file);
    let changed_bytes = fs::read(&changed_path).unwrap();
    let changed_request = request(changed_bytes.len() as u64, 1, &changed_bytes);
    let changed = NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
        &changed_request,
        NisarSubsetSourceValidatorV1 {
            kind: NisarSubsetValidatorKind::StrongEtag,
            value: ETAG.to_owned(),
            content_length: changed_request.source.expected_size_bytes,
        },
        &changed_path,
        &|| false,
    )
    .unwrap();
    assert_ne!(
        changed.evidence.doppler_lut,
        case.metadata.evidence.doppler_lut
    );
    assert_ne!(changed.evidence_sha256(), case.metadata.evidence_sha256());

    let probes = AtomicUsize::new(0);
    let cancel = || probes.fetch_add(1, Ordering::SeqCst) >= 1;
    let cancel_root = case._root.0.join("cancelled");
    assert!(matches!(
        writer(&cancel_root).write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &cancel,
        ),
        Err(NisarSubsetWriteError::Cancelled)
    ));
    assert!(!cancel_root
        .join(&case.request.expected_output.file_name)
        .exists());
}

#[test]
fn completed_artifact_checksum_or_science_tampering_blocks_recovery() {
    let case = ready_case("artifact-tamper");
    let output_root = case._root.0.join("artifact-tamper-output");
    let writer = writer(&output_root);
    let completed = writer
        .write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        )
        .unwrap();
    let file = File::open_rw(completed.artifact_path()).unwrap();
    let dataset = file.dataset(OUTPUT_SCIENCE_PATH).unwrap();
    let mut values = dataset.read_raw::<CFloat32>().unwrap();
    values[0].i += 2.0;
    dataset.write_raw(&values).unwrap();
    file.flush().unwrap();
    drop(file);
    assert!(matches!(
        writer.write_and_validate(
            &case.request,
            &case.plan,
            &case.transfer,
            &case.workspace,
            &case.metadata,
            &|| false,
        ),
        Err(NisarSubsetWriteError::RecoveryInvalid)
    ));
}

#[test]
fn verified_fill_chunks_materialize_exact_science_and_mask_values() {
    let science = VerifiedHdf5ChunkBlockV1 {
        dataset_path: SCIENCE_PATH.to_owned(),
        component_id: 0,
        logical_chunk: [0, 0],
        dataset_shape: [2, 2],
        chunk_shape: [2, 2],
        datatype: Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
        datatype_sha256: supported_datatype_sha256(Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi)
            .unwrap(),
        fill_value_definition: Hdf5FillValueDefinitionV1::Default,
        fill_value_hex: Some("0000000000000000".to_owned()),
        filter_pipeline: Hdf5ChunkFilterPipelineV1::None,
        part_path: PathBuf::new(),
        allocation: VerifiedHdf5ChunkAllocationV1::Fill {
            element_fill_hex: "0000000000000000".to_owned(),
        },
    };
    let mask = VerifiedHdf5ChunkBlockV1 {
        dataset_path: MASK_PATH.to_owned(),
        component_id: 0,
        logical_chunk: [0, 0],
        dataset_shape: [2, 2],
        chunk_shape: [2, 2],
        datatype: Hdf5ChunkDatatypeV1::UnsignedU8,
        datatype_sha256: supported_datatype_sha256(Hdf5ChunkDatatypeV1::UnsignedU8).unwrap(),
        fill_value_definition: Hdf5FillValueDefinitionV1::UserDefined,
        fill_value_hex: Some("ff".to_owned()),
        filter_pipeline: Hdf5ChunkFilterPipelineV1::None,
        part_path: PathBuf::new(),
        allocation: VerifiedHdf5ChunkAllocationV1::Fill {
            element_fill_hex: "ff".to_owned(),
        },
    };
    assert_eq!(
        super::decode_science_chunk(&science).unwrap(),
        vec![CFloat32::default(); 4]
    );
    assert_eq!(super::decode_mask_chunk(&mask).unwrap(), vec![255_u8; 4]);
}

#[test]
fn writer_working_set_rejects_an_oversized_decoded_chunk_before_materialization() {
    let root = TempDirectory::new("working-set");
    let source = root.0.join("source.h5");
    let _source_pin = create_source_fixture(&source);
    let source_bytes = fs::read(&source).unwrap();
    let mut request = request(source_bytes.len() as u64, 1, &source_bytes);
    let window = &mut request.mapping.windows[0];
    window.line_start = 0;
    window.line_end_exclusive = 1;
    window.sample_start = 0;
    window.sample_end_exclusive = 1;
    window.source_shape = [4096, 4096];
    window.expected_shape = [1, 1];
    window.chunk_shape = [4096, 4096];
    window.validity_mask_required = false;
    let block = VerifiedHdf5ChunkBlockV1 {
        dataset_path: window.dataset_path.clone(),
        component_id: 0,
        logical_chunk: [0, 0],
        dataset_shape: [4096, 4096],
        chunk_shape: [4096, 4096],
        datatype: Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
        datatype_sha256: supported_datatype_sha256(Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi)
            .unwrap(),
        fill_value_definition: Hdf5FillValueDefinitionV1::Default,
        fill_value_hex: Some("0000000000000000".to_owned()),
        filter_pipeline: Hdf5ChunkFilterPipelineV1::None,
        part_path: PathBuf::new(),
        allocation: VerifiedHdf5ChunkAllocationV1::Fill {
            element_fill_hex: "0000000000000000".to_owned(),
        },
    };
    let metadata = NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
        &request,
        NisarSubsetSourceValidatorV1 {
            kind: NisarSubsetValidatorKind::StrongEtag,
            value: ETAG.to_owned(),
            content_length: source_bytes.len() as u64,
        },
        &source,
        &|| false,
    )
    .unwrap();
    assert!(matches!(
        super::build_window_materials(&request, &[block], &metadata),
        Err(NisarSubsetWriteError::BlockEvidenceInvalid)
    ));
}

#[test]
fn writer_surface_has_no_network_or_credential_capability() {
    let source = include_str!("nisar_subset_writer.rs");
    assert!(!source.contains("use reqwest"));
    assert!(!source.contains("ResolvedAsset"));
    assert!(!source.contains("CredentialMaterial"));
    assert!(!source.contains("effective_url"));
    assert!(!source.contains("source_guard"));
    assert!(!source.contains("File::open(metadata."));
    assert!(!source.contains("H5DS"));
    let dimension_scale_boundary = include_str!("nisar_hdf5_dimension_scales.rs");
    assert_eq!(
        dimension_scale_boundary
            .matches("unsafe extern \"C\" {")
            .count(),
        1
    );
    assert!(!dimension_scale_boundary.contains("pub(crate) fn raw"));
    assert!(!dimension_scale_boundary.contains("pub(crate) fn id"));
    let actual_symbols = dimension_scale_boundary
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|token| token.starts_with("H5DS") || *token == "H5Iget_name")
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actual_symbols,
        std::collections::BTreeSet::from([
            "H5DSis_scale",
            "H5DSset_scale",
            "H5DSget_scale_name",
            "H5DSget_num_scales",
            "H5DSiterate_scales",
            "H5DSattach_scale",
            "H5DSget_label",
            "H5DSset_label",
            "H5Iget_name",
        ])
    );
    assert_eq!(
        OfflineNisarSubsetWriter::WRITER_ID,
        "project_native_subset_writer"
    );
    assert_eq!(
        NisarSubsetWriteError::TransferIncomplete.code(),
        "NISAR_SUBSET_INCOMPLETE"
    );
    assert_eq!(OfflineNisarSubsetWriter::OUTPUT_AUTHORITY, "client_derived");
    assert_ne!(OfflineNisarSubsetWriter::DERIVED_PRODUCT_TYPE, "RSLC");
}

#[test]
fn metadata_snapshot_rejects_a_same_size_source_with_the_wrong_sha256_identity() {
    let root = TempDirectory::new("metadata-source-sha");
    let source = root.0.join("source.h5");
    let _source_pin = create_source_fixture(&source);
    let source_bytes = fs::read(&source).unwrap();
    let mut request = request(source_bytes.len() as u64, 1, &source_bytes);
    request.source.checksum.as_mut().unwrap().value = "0".repeat(64);
    request.validate().unwrap();
    assert!(matches!(
        NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
            &request,
            NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: ETAG.to_owned(),
                content_length: source_bytes.len() as u64,
            },
            &source,
            &|| false,
        ),
        Err(NisarSubsetWriteError::MetadataInvalid)
    ));
}

#[test]
fn every_approved_frequency_and_polarization_writes_end_to_end() {
    for (label, frequency, polarization, science_path, mask_path) in [
        (
            "a-hh",
            NisarSubsetFrequency::A,
            "HH",
            SCIENCE_PATH,
            MASK_PATH,
        ),
        (
            "a-hv",
            NisarSubsetFrequency::A,
            "HV",
            "/science/LSAR/RSLC/swaths/frequencyA/HV",
            MASK_PATH,
        ),
        (
            "b-hh",
            NisarSubsetFrequency::B,
            "HH",
            "/science/LSAR/RSLC/swaths/frequencyB/HH",
            B_MASK_PATH,
        ),
        (
            "b-hv",
            NisarSubsetFrequency::B,
            "HV",
            B_HV_PATH,
            B_MASK_PATH,
        ),
        (
            "b-vv",
            NisarSubsetFrequency::B,
            "VV",
            B_VV_PATH,
            B_MASK_PATH,
        ),
    ] {
        let case = ready_case_with_selection(
            &format!("approved-polarization-{label}"),
            1,
            frequency,
            polarization,
            science_path,
            mask_path,
        );
        let output_root = case._root.0.join("output");
        let result = writer(&output_root)
            .write_and_validate(
                &case.request,
                &case.plan,
                &case.transfer,
                &case.workspace,
                &case.metadata,
                &|| false,
            )
            .unwrap();
        let output = File::open(result.artifact_path()).unwrap();
        let derived_science = format!(
            "/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequency{}/{polarization}",
            match case.request.request.frequency {
                NisarSubsetFrequency::A => "A",
                NisarSubsetFrequency::B => "B",
            }
        );
        assert!(output.dataset(&derived_science).is_ok());
    }
}

#[test]
fn request_identity_never_controls_temporary_output_paths() {
    let case = ready_case("request-path-identity");
    let (_workspace_lock, blocks) = case
        .workspace
        .acquire_verified_chunk_blocks(&case.request, &case.plan, &case.transfer, &|| false)
        .unwrap();
    let mut request = case.request.clone();
    request.expected_output.derived_artifact_id = "../../outside".to_owned();
    let output_root = case._root.0.join("safe-output");
    let result = writer(&output_root).write_verified(
        &request,
        &case.transfer,
        &case.metadata,
        &blocks,
        &|| false,
        WriterFailpoint::None,
    );
    assert!(matches!(result, Err(NisarSubsetWriteError::OutputUnsafe)));
    assert!(fs::read_dir(&output_root).unwrap().next().is_none());
    assert!(!case._root.0.join("outside").exists());
}

#[test]
fn whole_file_digest_observes_cancellation_between_bounded_reads() {
    let root = TempDirectory::new("sha-cancel");
    let candidate = root.0.join("candidate.bin");
    fs::write(&candidate, vec![7_u8; 256 * 1024]).unwrap();
    let guard = CandidateFileGuard::open(&candidate).unwrap();
    let probes = AtomicUsize::new(0);
    let cancelled = || probes.fetch_add(1, Ordering::SeqCst) >= 2;
    assert!(matches!(
        guard.sha256(&cancelled),
        Err(NisarSubsetWriteError::Cancelled)
    ));
    assert!(probes.load(Ordering::SeqCst) >= 3);
}

#[test]
fn metadata_shape_budget_rejects_sparse_oversize_before_read_raw() {
    let root = TempDirectory::new("metadata-budget");
    let source = root.0.join("source.h5");
    let _source_pin = create_source_fixture(&source);
    let file = File::open_rw(&source).unwrap();
    let path = "/science/LSAR/RSLC/metadata/orbit/time";
    file.unlink(path).unwrap();
    file.group("/science/LSAR/RSLC/metadata/orbit")
        .unwrap()
        .new_dataset::<f64>()
        .shape((super::MAX_METADATA_VALUES + 1,))
        .create("time")
        .unwrap();
    file.flush().unwrap();
    drop(file);
    let source_bytes = fs::read(&source).unwrap();
    let request = request(source_bytes.len() as u64, 1, &source_bytes);
    assert!(matches!(
        NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
            &request,
            NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: ETAG.to_owned(),
                content_length: source_bytes.len() as u64,
            },
            &source,
            &|| false,
        ),
        Err(NisarSubsetWriteError::MetadataInvalid)
    ));
}

#[test]
fn metadata_reader_rejects_unknown_filter_before_value_reads() {
    let root = TempDirectory::new("metadata-filter");
    let source = root.0.join("source.h5");
    let _source_pin = create_source_fixture(&source);
    let file = File::open_rw(&source).unwrap();
    let orbit = file.group("/science/LSAR/RSLC/metadata/orbit").unwrap();
    orbit.unlink("time").unwrap();
    let time = orbit
        .new_dataset::<f64>()
        .shape((2,))
        .chunk((2,))
        .scale_offset(hdf5::filters::ScaleOffset::FloatDScale(2))
        .create("time")
        .unwrap();
    time.write_raw(&[0.0_f64, 1.0]).unwrap();
    file.flush().unwrap();
    drop(time);
    drop(orbit);
    drop(file);
    let source_bytes = fs::read(&source).unwrap();
    let request = request(source_bytes.len() as u64, 1, &source_bytes);
    assert!(matches!(
        NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
            &request,
            NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: ETAG.to_owned(),
                content_length: source_bytes.len() as u64,
            },
            &source,
            &|| false,
        ),
        Err(NisarSubsetWriteError::MetadataInvalid)
    ));
}

#[test]
fn metadata_reader_rejects_rank_two_geolocation() {
    let root = TempDirectory::new("rank-two-geolocation");
    let source = root.0.join("source.h5");
    let _source_pin = create_source_fixture(&source);
    let file = File::open_rw(&source).unwrap();
    let geolocation = file
        .group("/science/LSAR/RSLC/metadata/geolocationGrid")
        .unwrap();
    for name in ["coordinateX", "coordinateY"] {
        geolocation.unlink(name).unwrap();
        geolocation
            .new_dataset::<f64>()
            .shape((2, 2))
            .create(name)
            .unwrap()
            .write_raw(&[36.0_f64, 37.0, 36.0, 37.0])
            .unwrap();
    }
    file.flush().unwrap();
    drop(geolocation);
    drop(file);
    let source_bytes = fs::read(&source).unwrap();
    let request = request(source_bytes.len() as u64, 1, &source_bytes);
    assert!(matches!(
        NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
            &request,
            NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: ETAG.to_owned(),
                content_length: source_bytes.len() as u64,
            },
            &source,
            &|| false,
        ),
        Err(NisarSubsetWriteError::MetadataInvalid)
    ));
}

#[test]
fn metadata_reader_binds_embedded_granule_and_product_identity() {
    let root = TempDirectory::new("metadata-source-identity");
    let source = root.0.join("source.h5");
    let _source_pin = create_source_fixture(&source);
    let file = File::open_rw(&source).unwrap();
    let granule = file
        .dataset("/science/LSAR/identification/granuleId")
        .unwrap();
    granule
        .write_scalar(
            &hdf5::types::FixedAscii::<256>::from_ascii(
                "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_999",
            )
            .unwrap(),
        )
        .unwrap();
    file.flush().unwrap();
    drop(granule);
    drop(file);
    let source_bytes = fs::read(&source).unwrap();
    let request = request(source_bytes.len() as u64, 1, &source_bytes);
    assert!(matches!(
        NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
            &request,
            NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: ETAG.to_owned(),
                content_length: source_bytes.len() as u64,
            },
            &source,
            &|| false,
        ),
        Err(NisarSubsetWriteError::MetadataInvalid)
    ));
}

#[test]
fn metadata_reader_rejects_missing_official_inventory_path() {
    let root = TempDirectory::new("missing-inventory");
    let source = root.0.join("source.h5");
    let _source_pin = create_source_fixture(&source);
    let file = File::open_rw(&source).unwrap();
    file.unlink("/science/LSAR/RSLC/metadata/geolocationGrid/heightAboveEllipsoid")
        .unwrap();
    file.flush().unwrap();
    drop(file);
    let source_bytes = fs::read(&source).unwrap();
    let request = request(source_bytes.len() as u64, 1, &source_bytes);
    assert!(matches!(
        NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
            &request,
            NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: ETAG.to_owned(),
                content_length: source_bytes.len() as u64,
            },
            &source,
            &|| false,
        ),
        Err(NisarSubsetWriteError::MetadataInvalid)
    ));
}

#[test]
fn metadata_reader_rejects_broken_dimension_scale_shape() {
    let root = TempDirectory::new("broken-dimension-scale");
    let source = root.0.join("source.h5");
    let _source_pin = create_source_fixture(&source);
    let file = File::open_rw(&source).unwrap();
    let coordinate = file
        .dataset("/science/LSAR/RSLC/metadata/geolocationGrid/coordinateX")
        .unwrap();
    let wrong_scale = file
        .dataset("/science/LSAR/RSLC/swaths/zeroDopplerTime")
        .unwrap();
    assert_eq!(
        super::dimension_scales::set_scale(&wrong_scale, "wrongScale"),
        Ok(())
    );
    assert_eq!(
        super::dimension_scales::attach_scale(
            &file,
            &coordinate,
            "/science/LSAR/RSLC/swaths/zeroDopplerTime",
            0,
        ),
        Ok(())
    );
    file.flush().unwrap();
    drop(wrong_scale);
    drop(coordinate);
    drop(file);
    let source_bytes = fs::read(&source).unwrap();
    let request = request(source_bytes.len() as u64, 1, &source_bytes);
    assert!(matches!(
        NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
            &request,
            NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: ETAG.to_owned(),
                content_length: source_bytes.len() as u64,
            },
            &source,
            &|| false,
        ),
        Err(NisarSubsetWriteError::MetadataInvalid)
    ));
}

#[test]
fn metadata_reader_rejects_external_links_before_opening_target_data() {
    let root = TempDirectory::new("metadata-external-link");
    let source = root.0.join("source.h5");
    let external = root.0.join("external.h5");
    let _source_pin = create_source_fixture(&source);
    let external_file = File::create(&external).unwrap();
    external_file
        .new_dataset::<f64>()
        .shape((2,))
        .create("time")
        .unwrap()
        .write_raw(&[0.0_f64, 1.0])
        .unwrap();
    external_file.flush().unwrap();
    drop(external_file);

    let file = File::open_rw(&source).unwrap();
    let orbit = file.group("/science/LSAR/RSLC/metadata/orbit").unwrap();
    orbit.unlink("time").unwrap();
    orbit
        .link_external(external.to_str().unwrap(), "/time", "time")
        .unwrap();
    file.flush().unwrap();
    drop(orbit);
    drop(file);
    let source_bytes = fs::read(&source).unwrap();
    let request = request(source_bytes.len() as u64, 1, &source_bytes);
    assert!(matches!(
        NisarSubsetMetadataSnapshotV1::from_verified_offline_metadata_hdf5(
            &request,
            NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: ETAG.to_owned(),
                content_length: source_bytes.len() as u64,
            },
            &source,
            &|| false,
        ),
        Err(NisarSubsetWriteError::MetadataInvalid)
    ));
}

fn create_compressed_science_fixture(path: &Path) -> (Vec<CFloat32>, Vec<u8>) {
    let file = File::create_excl(path).unwrap();
    let root = file.group("/").unwrap();
    let convention = hdf5::types::FixedAscii::<32>::from_ascii("NISAR_RSLC_1.5.0").unwrap();
    root.new_attr::<hdf5::types::FixedAscii<32>>()
        .create("Conventions")
        .unwrap()
        .write_scalar(&convention)
        .unwrap();
    let frequency = file
        .create_group("/science/LSAR/RSLC/swaths/frequencyA")
        .unwrap();
    let science = (0..16)
        .map(|index| CFloat32 {
            r: index as f32 + 0.25,
            i: -(index as f32) - 0.5,
        })
        .collect::<Vec<_>>();
    frequency
        .new_dataset::<CFloat32>()
        .shape((4, 4))
        .chunk((2, 2))
        .shuffle()
        .deflate(4)
        .create("HH")
        .and_then(|dataset| dataset.write_raw(&science))
        .unwrap();
    let mask = (0_u8..36).map(|value| value % 3).collect::<Vec<_>>();
    frequency
        .new_dataset::<u8>()
        .shape((6, 6))
        .chunk((1, 2))
        .shuffle()
        .deflate(4)
        .create("inputDataExceptionMask")
        .and_then(|dataset| dataset.write_raw(&mask))
        .unwrap();
    (science, mask)
}

#[test]
fn hdf5_native_decode_matches_shuffle_deflate4_fixture_values() {
    let temp = TempDirectory::new("compressed-decode");
    let fixture_path = temp.0.join("compressed.h5");
    let (science, mask) = create_compressed_science_fixture(&fixture_path);
    // Verify dataset filters are exactly Shuffle + Deflate(4).
    {
        let file = File::open(&fixture_path).unwrap();
        let hh = file
            .dataset("science/LSAR/RSLC/swaths/frequencyA/HH")
            .unwrap();
        assert_eq!(
            hh.filters(),
            vec![
                hdf5::filters::Filter::Shuffle,
                hdf5::filters::Filter::Deflate(4)
            ]
        );
    }
    // Science chunk (0,0) is a 2x2 block of the first four complex values.
    let science_block = VerifiedHdf5ChunkBlockV1 {
        dataset_path: "science/LSAR/RSLC/swaths/frequencyA/HH".to_owned(),
        component_id: 0,
        logical_chunk: [0, 0],
        dataset_shape: [4, 4],
        chunk_shape: [2, 2],
        datatype: Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
        datatype_sha256: supported_datatype_sha256(Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi)
            .unwrap(),
        fill_value_definition: Hdf5FillValueDefinitionV1::Undefined,
        fill_value_hex: None,
        filter_pipeline: Hdf5ChunkFilterPipelineV1::ShuffleDeflate { level: 4 },
        part_path: fixture_path.clone(),
        allocation: VerifiedHdf5ChunkAllocationV1::Allocated {
            bytes: Arc::from(vec![0_u8; 0]),
        },
    };
    let decoded = super::decode_science_chunk(&science_block).unwrap();
    assert_eq!(decoded.len(), 4);
    // 2x2 chunk at logical (0,0): row-major positions (0,0),(0,1),(1,0),(1,1).
    let expected_indices = [0_usize, 1, 4, 5];
    for (position, value) in decoded.iter().enumerate() {
        let expected = &science[expected_indices[position]];
        assert_eq!(value.r.to_bits(), expected.r.to_bits());
        assert_eq!(value.i.to_bits(), expected.i.to_bits());
    }
    // Mask chunk (0,0) is a 1x2 block of the first two mask bytes.
    let mask_block = VerifiedHdf5ChunkBlockV1 {
        dataset_path: "science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask".to_owned(),
        component_id: 0,
        logical_chunk: [0, 0],
        dataset_shape: [6, 6],
        chunk_shape: [1, 2],
        datatype: Hdf5ChunkDatatypeV1::UnsignedU8,
        datatype_sha256: supported_datatype_sha256(Hdf5ChunkDatatypeV1::UnsignedU8).unwrap(),
        fill_value_definition: Hdf5FillValueDefinitionV1::Undefined,
        fill_value_hex: None,
        filter_pipeline: Hdf5ChunkFilterPipelineV1::ShuffleDeflate { level: 4 },
        part_path: fixture_path,
        allocation: VerifiedHdf5ChunkAllocationV1::Allocated {
            bytes: Arc::from(vec![0_u8; 0]),
        },
    };
    let decoded_mask = super::decode_mask_chunk(&mask_block).unwrap();
    assert_eq!(decoded_mask, mask[..2]);
}

#[test]
fn verified_coverage_prefix_resident_and_straddling_chunks_materialize() {
    let case = ready_case("prefix-coverage");
    let chunks = case.plan.chunks().to_vec();
    let mut intervals = chunks
        .iter()
        .filter_map(|chunk| match chunk.allocation {
            Hdf5ChunkAllocationV1::Allocated {
                start,
                end_exclusive,
                ..
            } => Some((start, end_exclusive)),
            Hdf5ChunkAllocationV1::Fill { .. } => None,
        })
        .collect::<Vec<_>>();
    intervals.sort_unstable();
    assert!(intervals.len() >= 2);
    // Prefix owns the first chunk completely and splits the second chunk, so
    // chunk0 is prefix-resident (Case B) and chunk1 straddles the boundary
    // (Case C). The union of the verified prefix and science ranges must cover
    // both full physical chunk intervals.
    let prefix = (intervals[0].1 + intervals[1].0) / 2;
    assert!(prefix > intervals[0].0 && prefix < intervals[1].1);
    let mut request = case.request.clone();
    let remote = prefix
        + chunks
            .iter()
            .filter_map(|chunk| match chunk.allocation {
                Hdf5ChunkAllocationV1::Allocated {
                    start,
                    end_exclusive,
                    ..
                } => {
                    if end_exclusive <= prefix {
                        Some(0)
                    } else {
                        Some(end_exclusive - start.max(prefix))
                    }
                }
                Hdf5ChunkAllocationV1::Fill { .. } => None,
            })
            .sum::<u64>();
    request.expected_output.estimated_remote_bytes = remote;
    request.validate().unwrap();
    let plan = Hdf5RangePlanV1::from_evidence_with_metadata_prefix_for_test(
        &request,
        ApprovedHdf5ChunkSetV1 {
            request_manifest_sha256: request.sha256().unwrap(),
            source_length: case.source_before.len() as u64,
            layout_evidence_digest:
                "86a615c78d3a9d1407ec3b5f4ddc76a10a5aff157b473bba48d9d31865e71500".to_owned(),
            metadata_evidence_sha256: case.metadata.evidence_sha256().to_owned(),
            chunks,
        },
        prefix,
    )
    .unwrap();
    assert_eq!(plan.ranges()[0].start, 0);
    assert_eq!(plan.ranges()[0].end_exclusive, prefix);
    assert!(plan.ranges()[0].member_chunk_indices().is_empty());
    let server = RangeFileServer::new(
        case.source_path.clone(),
        case.source_before.len() as u64,
        plan.ranges().len(),
    );
    let workspace = Hdf5RangeWorkspaceV1::new(case._root.0.join("prefix-transfer")).unwrap();
    let runtime = OfflineHdf5RangeTransfer::new(
        HttpTransfer::new_with_proxy_policy(
            TransferOptions {
                max_attempts: 1,
                backoff: Duration::ZERO,
                timeout: Duration::from_secs(5),
                chunk_size: 8,
                ..TransferOptions::default()
            },
            ProxyPolicy::Direct,
        )
        .unwrap(),
        &request,
        &snapshot(&request),
        resolved_asset(&server.url, &request),
        workspace.clone(),
        DestinationLeaseController::new_for_qualification_test(),
    )
    .unwrap();
    let transfer = runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    assert_eq!(transfer.manifest().received_bytes, remote);
    assert_eq!(server.finish().len(), plan.ranges().len());
    let (_lock, blocks) = workspace
        .acquire_verified_chunk_blocks(&request, &plan, &transfer, &|| false)
        .unwrap();
    let physical = blocks
        .iter()
        .map(|block| match &block.allocation {
            VerifiedHdf5ChunkAllocationV1::Allocated { bytes } => bytes.len(),
            VerifiedHdf5ChunkAllocationV1::Fill { .. } => 0,
        })
        .sum::<usize>();
    let expected_physical = case
        .plan
        .chunks()
        .iter()
        .filter_map(|chunk| match chunk.allocation {
            Hdf5ChunkAllocationV1::Allocated {
                start,
                end_exclusive,
                ..
            } => Some(usize::try_from(end_exclusive - start).unwrap()),
            Hdf5ChunkAllocationV1::Fill { .. } => None,
        })
        .sum::<usize>();
    assert_eq!(physical, expected_physical);
    assert!(blocks
        .iter()
        .any(|block| block.dataset_path == SCIENCE_PATH));
    assert!(blocks.iter().any(|block| block.dataset_path == MASK_PATH));
}

#[test]
fn verified_digest_mismatch_never_materializes_blocks() {
    let case = ready_case("coverage-digest");
    let range = case.plan.ranges()[0].clone();
    let part_path = case.workspace.part_path();
    let mut part = fs::OpenOptions::new().write(true).open(&part_path).unwrap();
    part.seek(SeekFrom::Start(range.start)).unwrap();
    let zeros = vec![0_u8; usize::try_from(range.end_exclusive - range.start).unwrap()];
    part.write_all(&zeros).unwrap();
    part.sync_all().unwrap();
    drop(part);
    assert!(matches!(
        case.workspace.acquire_verified_chunk_blocks(
            &case.request,
            &case.plan,
            &case.transfer,
            &|| false
        ),
        Err(crate::Hdf5RangeTransferError::RecoveryInvalid)
    ));
}

#[test]
fn sparse_hole_never_becomes_valid_science() {
    let case = ready_case("coverage-hole");
    // Zero-fill only the middle of the first completed science interval: the
    // remaining planned bytes still exist, so this simulates a sparse hole in
    // the part rather than a truncated file.
    let range = case.plan.ranges()[0].clone();
    let hole_start = range.start + (range.end_exclusive - range.start) / 4;
    let hole_end = range.start + (range.end_exclusive - range.start) / 2;
    let part_path = case.workspace.part_path();
    let mut part = fs::OpenOptions::new().write(true).open(&part_path).unwrap();
    part.seek(SeekFrom::Start(hole_start)).unwrap();
    part.write_all(&vec![0_u8; usize::try_from(hole_end - hole_start).unwrap()])
        .unwrap();
    part.sync_all().unwrap();
    drop(part);
    assert!(matches!(
        case.workspace.acquire_verified_chunk_blocks(
            &case.request,
            &case.plan,
            &case.transfer,
            &|| false
        ),
        Err(crate::Hdf5RangeTransferError::RecoveryInvalid)
    ));
}

fn create_edge_compressed_fixture(path: &Path) -> (Vec<CFloat32>, Vec<u8>) {
    let file = File::create_excl(path).unwrap();
    let frequency = file
        .create_group("/science/LSAR/RSLC/swaths/frequencyA")
        .unwrap();
    let science = (0..35)
        .map(|index| {
            let row = index / 7;
            let column = index % 7;
            CFloat32 {
                r: row as f32 * 1000.0 + column as f32 + 0.25,
                i: -(row as f32 * 10.0 + column as f32) - 0.5,
            }
        })
        .collect::<Vec<_>>();
    frequency
        .new_dataset::<CFloat32>()
        .shape((5, 7))
        .chunk((4, 4))
        .shuffle()
        .deflate(4)
        .create("HH")
        .and_then(|dataset| dataset.write_raw(&science))
        .unwrap();
    let mask = (0..35).map(|index| (index % 3) as u8).collect::<Vec<_>>();
    frequency
        .new_dataset::<u8>()
        .shape((5, 7))
        .chunk((4, 4))
        .shuffle()
        .deflate(4)
        .create("inputDataExceptionMask")
        .and_then(|dataset| dataset.write_raw(&mask))
        .unwrap();
    (science, mask)
}

fn compressed_edge_block(
    fixture_path: &Path,
    logical_chunk: [u64; 2],
    datatype: Hdf5ChunkDatatypeV1,
) -> VerifiedHdf5ChunkBlockV1 {
    let dataset_path = match datatype {
        Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi => {
            "science/LSAR/RSLC/swaths/frequencyA/HH".to_owned()
        }
        Hdf5ChunkDatatypeV1::UnsignedU8 => {
            "science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask".to_owned()
        }
    };
    VerifiedHdf5ChunkBlockV1 {
        dataset_path,
        component_id: 0,
        logical_chunk,
        dataset_shape: [5, 7],
        chunk_shape: [4, 4],
        datatype,
        datatype_sha256: supported_datatype_sha256(datatype).unwrap(),
        fill_value_definition: Hdf5FillValueDefinitionV1::Undefined,
        fill_value_hex: None,
        filter_pipeline: Hdf5ChunkFilterPipelineV1::ShuffleDeflate { level: 4 },
        part_path: fixture_path.to_path_buf(),
        allocation: VerifiedHdf5ChunkAllocationV1::Allocated {
            bytes: Arc::from(vec![0_u8; 0]),
        },
    }
}

#[test]
fn edge_chunk_hdf5_native_decode_is_bounds_clamped() {
    let temp = TempDirectory::new("edge-decode");
    let fixture_path = temp.0.join("edge.h5");
    let (science, mask) = create_edge_compressed_fixture(&fixture_path);
    // shape 5x7, chunk 4x4: logical chunk (1,0) is a 1x4 final-row partial
    // chunk, (0,1) is a 4x3 final-column partial chunk, and (1,1) is the 1x3
    // bottom-right partial chunk.
    for (logical, rows, columns) in [
        ([0_u64, 0], 4_usize, 4_usize),
        ([1, 0], 1, 4),
        ([0, 1], 4, 3),
        ([1, 1], 1, 3),
    ] {
        let science_block = compressed_edge_block(
            &fixture_path,
            logical,
            Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
        );
        let decoded = super::decode_science_chunk(&science_block).unwrap_or_else(|error| {
            panic!("science edge chunk {logical:?} failed: {error:?}");
        });
        assert_eq!(decoded.len(), rows * columns);
        let row_start = (logical[0] as usize) * 4;
        let column_start = (logical[1] as usize) * 4;
        for (position, value) in decoded.iter().enumerate() {
            let row = row_start + position / columns;
            let column = column_start + position % columns;
            let expected = &science[row * 7 + column];
            assert_eq!(value.r.to_bits(), expected.r.to_bits());
            assert_eq!(value.i.to_bits(), expected.i.to_bits());
        }
        let mask_block =
            compressed_edge_block(&fixture_path, logical, Hdf5ChunkDatatypeV1::UnsignedU8);
        let decoded_mask = super::decode_mask_chunk(&mask_block).unwrap_or_else(|error| {
            panic!("mask edge chunk {logical:?} failed: {error:?}");
        });
        assert_eq!(decoded_mask.len(), rows * columns);
        for (position, value) in decoded_mask.iter().enumerate() {
            let row = row_start + position / columns;
            let column = column_start + position % columns;
            assert_eq!(*value, mask[row * 7 + column]);
        }
    }
    let out_of_bounds = compressed_edge_block(
        &fixture_path,
        [2, 0],
        Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
    );
    assert!(matches!(
        super::decode_science_chunk(&out_of_bounds),
        Err(NisarSubsetWriteError::BlockEvidenceInvalid)
    ));
}

#[test]
fn unapproved_filter_pipelines_fail_closed_in_source_qualification() {
    let temp = TempDirectory::new("filter-negative");
    let fixture_path = temp.0.join("filters.h5");
    let file = File::create_excl(&fixture_path).unwrap();
    let frequency = file
        .create_group("/science/LSAR/RSLC/swaths/frequencyA")
        .unwrap();
    let science = (0..16)
        .map(|index| CFloat32 {
            r: index as f32 + 0.25,
            i: -(index as f32) - 0.5,
        })
        .collect::<Vec<_>>();
    frequency
        .new_dataset::<CFloat32>()
        .shape((4, 4))
        .chunk((2, 2))
        .shuffle()
        .deflate(4)
        .create("accepted")
        .and_then(|dataset| dataset.write_raw(&science))
        .unwrap();
    for (name, build) in [
        ("deflate_only", "deflate4"),
        ("shuffle_only", "shuffle"),
        ("wrong_level", "deflate6"),
        ("shuffle_wrong_level", "shuffle_deflate6"),
        ("checksum_after_deflate", "reversed"),
        ("additional_fletcher", "extra"),
        ("fletcher_only", "fletcher"),
    ] {
        let builder = frequency
            .new_dataset::<CFloat32>()
            .shape((4, 4))
            .chunk((2, 2));
        let dataset = match build {
            "deflate4" => builder.deflate(4).create(name).unwrap(),
            "shuffle" => builder.shuffle().create(name).unwrap(),
            "deflate6" => builder.deflate(6).create(name).unwrap(),
            "shuffle_deflate6" => builder.shuffle().deflate(6).create(name).unwrap(),
            "reversed" => builder.deflate(4).fletcher32().create(name).unwrap(),
            "extra" => builder
                .shuffle()
                .deflate(4)
                .fletcher32()
                .create(name)
                .unwrap(),
            "fletcher" => builder.fletcher32().create(name).unwrap(),
            _ => unreachable!(),
        };
        dataset.write_raw(&science).unwrap();
    }
    drop(frequency);
    drop(file);
    let accepted = File::open(&fixture_path).unwrap();
    let accepted_ds = accepted
        .dataset("science/LSAR/RSLC/swaths/frequencyA/accepted")
        .unwrap();
    assert!(qualify_source_dataset_identity(
        &accepted_ds,
        Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
    )
    .is_ok());
    for name in [
        "deflate_only",
        "shuffle_only",
        "wrong_level",
        "shuffle_wrong_level",
        "checksum_after_deflate",
        "additional_fletcher",
        "fletcher_only",
    ] {
        let path = format!("science/LSAR/RSLC/swaths/frequencyA/{name}");
        let dataset = accepted.dataset(path.as_str()).unwrap();
        assert!(
            matches!(
                qualify_source_dataset_identity(
                    &dataset,
                    Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
                ),
                Err(crate::Hdf5RangeTransferError::RangeInvalid)
            ),
            "pipeline {name} must fail closed"
        );
    }
}

#[test]
fn plan_rejects_nonzero_chunk_filter_mask() {
    let case = ready_case("filter-mask");
    let mut chunks = case.plan.chunks().to_vec();
    match &mut chunks[0].allocation {
        Hdf5ChunkAllocationV1::Allocated { filter_mask, .. } => *filter_mask = 1,
        Hdf5ChunkAllocationV1::Fill { .. } => panic!("fixture first chunk must be allocated"),
    }
    let result = Hdf5RangePlanV1::from_evidence_with_metadata_prefix_for_test(
        &case.request,
        ApprovedHdf5ChunkSetV1 {
            request_manifest_sha256: case.request.sha256().unwrap(),
            source_length: case.source_before.len() as u64,
            layout_evidence_digest:
                "86a615c78d3a9d1407ec3b5f4ddc76a10a5aff157b473bba48d9d31865e71500".to_owned(),
            metadata_evidence_sha256: case.metadata.evidence_sha256().to_owned(),
            chunks,
        },
        0,
    );
    assert!(matches!(
        result,
        Err(crate::Hdf5RangeTransferError::RangeInvalid)
    ));
}

fn semantic_test_plan(
    case: &ReadyCase,
    request: &insar_core::NisarSubsetRequestManifestV1,
    metadata_evidence_sha256: &str,
    chunks: Vec<Hdf5ChunkPhysicalRangeV1>,
    metadata_prefix_bytes: u64,
) -> Hdf5RangePlanV1 {
    let remote_bytes = metadata_prefix_bytes
        + case
            .plan
            .ranges()
            .iter()
            .filter(|range| range.end_exclusive > metadata_prefix_bytes)
            .map(|range| range.end_exclusive - range.start.max(metadata_prefix_bytes))
            .sum::<u64>();
    let mut request = request.clone();
    request.expected_output.estimated_remote_bytes = remote_bytes;
    request.validate().unwrap();
    Hdf5RangePlanV1::from_evidence_with_metadata_prefix_for_test(
        &request,
        ApprovedHdf5ChunkSetV1 {
            request_manifest_sha256: request.sha256().unwrap(),
            source_length: case.source_before.len() as u64,
            layout_evidence_digest: case.plan.layout_evidence_digest().to_owned(),
            metadata_evidence_sha256: metadata_evidence_sha256.to_owned(),
            chunks,
        },
        metadata_prefix_bytes,
    )
    .unwrap()
}

fn authority_hash(case: &ReadyCase) -> String {
    derive_nisar_live_semantic_plan_sha256(&case.request, &case.metadata, &case.plan).unwrap()
}

#[test]
fn metadata_snapshot_reports_source_dataset_count_separately_from_bindings() {
    let case = ready_case("semantic-counts");
    let source_count = source_dataset_count(&File::open(&case.source_path).unwrap());
    assert_eq!(source_count, 207);
    assert_eq!(case.metadata.source_dataset_count(), source_count as u64);
    assert_eq!(
        case.metadata.metadata_binding_count(),
        case.metadata
            .metadata_group_binding_count()
            .checked_add(case.metadata.metadata_dataset_binding_count())
            .unwrap()
    );
}

#[test]
fn semantic_authority_is_invariant_to_catalogue_and_validator_provenance() {
    let case = ready_case("semantic-provenance");
    let original = authority_hash(&case);

    let mut request = case.request.clone();
    request.source.collection_id = "cmr-concept-id-changed".to_owned();
    request.source.provider_product_id = "provider-product-id-changed".to_owned();
    request.source.provider_product_key = "provider-product-key-changed".to_owned();
    request.source.stable_asset_key = "stable-asset-key-changed".to_owned();
    request.validate().unwrap();

    let mut metadata = case.metadata.clone();
    metadata.request_manifest_sha256 = request.sha256().unwrap();
    metadata.source_identity = request.source.clone();
    metadata.source_validator.value = "\"changed-validator\"".to_owned();
    metadata.evidence_sha256 = "d".repeat(64);
    let plan = semantic_test_plan(
        &case,
        &request,
        &metadata.evidence_sha256,
        case.plan.chunks().to_vec(),
        case.plan.metadata_prefix_bytes(),
    );

    assert_ne!(case.plan.plan_sha256(), plan.plan_sha256());
    assert_eq!(
        original,
        derive_nisar_live_semantic_plan_sha256(&request, &metadata, &plan).unwrap()
    );
}

#[test]
fn semantic_authority_changes_for_source_request_inventory_and_physical_mutations() {
    let case = ready_case("semantic-mutations");
    let original = authority_hash(&case);

    let mut checksum_request = case.request.clone();
    checksum_request.source.checksum.as_mut().unwrap().value = "f".repeat(64);
    checksum_request.validate().unwrap();
    let checksum_plan = semantic_test_plan(
        &case,
        &checksum_request,
        case.metadata.evidence_sha256(),
        case.plan.chunks().to_vec(),
        case.plan.metadata_prefix_bytes(),
    );
    assert_ne!(
        original,
        derive_nisar_live_semantic_plan_sha256(&checksum_request, &case.metadata, &checksum_plan)
            .unwrap()
    );

    let mut size_request = case.request.clone();
    size_request.source.expected_size_bytes += 1;
    size_request.expected_output.full_product_bytes += 1;
    size_request.validate().unwrap();
    let size_plan = Hdf5RangePlanV1::from_evidence_with_metadata_prefix_for_test(
        &size_request,
        ApprovedHdf5ChunkSetV1 {
            request_manifest_sha256: size_request.sha256().unwrap(),
            source_length: case.source_before.len() as u64 + 1,
            layout_evidence_digest: case.plan.layout_evidence_digest().to_owned(),
            metadata_evidence_sha256: case.metadata.evidence_sha256().to_owned(),
            chunks: case.plan.chunks().to_vec(),
        },
        case.plan.metadata_prefix_bytes(),
    )
    .unwrap();
    assert_ne!(
        original,
        derive_nisar_live_semantic_plan_sha256(&size_request, &case.metadata, &size_plan).unwrap()
    );

    let mut aoi_mapping_request = case.request.clone();
    aoi_mapping_request.request.aoi_sha256 = "a".repeat(64);
    aoi_mapping_request.mapping.evidence_digest = "b".repeat(64);
    aoi_mapping_request.validate().unwrap();
    let aoi_mapping_plan = semantic_test_plan(
        &case,
        &aoi_mapping_request,
        case.metadata.evidence_sha256(),
        case.plan.chunks().to_vec(),
        case.plan.metadata_prefix_bytes(),
    );
    assert_ne!(
        original,
        derive_nisar_live_semantic_plan_sha256(
            &aoi_mapping_request,
            &case.metadata,
            &aoi_mapping_plan
        )
        .unwrap()
    );

    let changed_inventory = {
        let mut metadata = case.metadata.clone();
        metadata.source_dataset_count += 1;
        metadata.source_inventory_sha256 = "e".repeat(64);
        metadata
    };
    assert_ne!(
        original,
        derive_nisar_live_semantic_plan_sha256(&case.request, &changed_inventory, &case.plan)
            .unwrap()
    );

    let prefix_plan = semantic_test_plan(
        &case,
        &case.request,
        case.metadata.evidence_sha256(),
        case.plan.chunks().to_vec(),
        1,
    );
    let mut prefix_request = case.request.clone();
    prefix_request.expected_output.estimated_remote_bytes = prefix_plan.expected_remote_bytes();
    prefix_request.validate().unwrap();
    assert_ne!(case.plan.ranges(), prefix_plan.ranges());
    assert_ne!(
        original,
        derive_nisar_live_semantic_plan_sha256(&prefix_request, &case.metadata, &prefix_plan)
            .unwrap()
    );
}

#[test]
fn semantic_authority_changes_for_frequency_polarization_datatype_and_filter() {
    let case = ready_case("semantic-datatype-filter");
    let original = authority_hash(&case);
    let frequency_case = ready_case_with_selection(
        "semantic-frequency",
        1,
        NisarSubsetFrequency::B,
        "HH",
        B_HH_PATH,
        B_MASK_PATH,
    );
    assert_ne!(
        original,
        derive_nisar_live_semantic_plan_sha256(
            &frequency_case.request,
            &frequency_case.metadata,
            &frequency_case.plan
        )
        .unwrap()
    );

    let datatype_plan = case.plan.with_test_datatype_mutation();
    assert_ne!(
        original,
        derive_nisar_live_semantic_plan_sha256(&case.request, &case.metadata, &datatype_plan)
            .unwrap()
    );
    let filter_plan = case.plan.with_test_filter_mutation();
    assert_ne!(
        original,
        derive_nisar_live_semantic_plan_sha256(&case.request, &case.metadata, &filter_plan)
            .unwrap()
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
mod local_qualification {
    use super::{
        parse_range, read_request, ready_case, rebind_ready_case_request, snapshot,
        source_dataset_count, AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority,
        AssetExecutionPolicy, AssetResolverParameters, AssetRole, CFloat32, FrozenAssetAccess,
        Hdf5ChunkAllocationV1, SnapshotItem, StableAssetReference, ETAG, OUTPUT_MASK_PATH,
        OUTPUT_SCIENCE_PATH, SCIENCE_PATH,
    };
    use crate::{
        build_live_nisar_subset_request, derive_nisar_live_semantic_plan_sha256,
        discover_live_nisar_metadata_chunks, live_nisar_range_plan_from_sparse_hdf5,
        prefetch_live_nisar_metadata, seed_live_metadata_range, validate_nisar_live_admission,
        AssetResolver, AssetResolverRegistry, Clock, CredentialMaterial, CredentialProvider,
        CredentialProviderError, DestinationLeaseController, DownloadWorker, Hdf5RangePlanV1,
        Hdf5RangeWorkspaceV1, HttpTransfer, NisarLiveNetworkBudgetV1,
        NisarLiveTransportAdmissionV1, NisarSubsetMetadataSnapshotV1, OfflineHdf5RangeTransfer,
        OfflineNisarSubsetWriter, ProductionDownloadWorker, ProxyPolicy, RemoteValidator,
        ResolveContext, ResolveError, ResolvedAsset, ResolvedChecksum, ResolvedMethod,
        ResolverExecutionRuntime, ShadowAccessIntent, ShadowResolveContext, TransferOptions,
        WorkerControl, NISAR_LIVE_METADATA_PREFIX_BYTES,
        NISAR_LIVE_PLAN_SEMANTIC_AUTHORITY_CONTRACT_V1,
        NISAR_M2D_B2_SEMANTIC_PLAN_AUTHORITY_SHA256,
    };
    use hdf5::File;
    use insar_core::{
        DownloadOutcome, NisarSubsetFrequency, NisarSubsetSourceValidatorV1,
        NisarSubsetTransferState, NisarSubsetValidatorKind,
    };
    use insar_provider::asf::nisar_subset::{
        build_live_geolocation_evidence, nearest_radar_axis_indices,
    };
    use ndarray::Array2;
    use serde_json::{json, Map, Value};
    use sha2::{Digest as Sha2Digest, Sha256};
    use std::fs;
    use std::io::{Read, Seek, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const GRANULE_ID: &str =
        "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001";
    const FILE_NAME: &str =
        "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001.h5";
    const SOURCE_BYTES: u64 = 14_252_244_992;
    const SOURCE_SHA256: &str = "ee3de9b9877b4f8e35153369725dcb4174ef5f2423af167e77f843e08860a9a0";
    const SOURCE_MD5: &str = "059ada6521bff1ef314fb1141a89cd10";
    const INVENTORY_V2_SHA256: &str =
        "827ed8174e83736eba483dbf4a901598f08b2d02d1f178e9ef8885ef5c048aa8";
    const DONOR_OUTPUT_SHA256: &str =
        "3c2c408ff8fab29cc1035eab2a19f4bd7b6496af13f53bf8f4aaf64a61e23912";
    const REFERENCE_LINE_START: u64 = 26_112;
    const REFERENCE_LINE_END: u64 = 26_624;
    const REFERENCE_SAMPLE_START: u64 = 12_800;
    const REFERENCE_SAMPLE_END: u64 = 13_312;

    fn approved_transport() -> NisarLiveTransportAdmissionV1 {
        NisarLiveTransportAdmissionV1 {
            final_host_class: "earthdata_asf_v1",
            final_path_class: "approved_nisar_hdf5_object",
            range_status_206: true,
            content_range_exact: true,
            response_length_exact: true,
            full_file_fallback_rejected: true,
            unauthorized_redirect_rejected: true,
        }
    }

    fn admit_ready_case(
        case: &super::ReadyCase,
        plan: &Hdf5RangePlanV1,
    ) -> Result<(), crate::NisarLiveAdmissionError> {
        let source_validator = case.metadata.source_validator().clone();
        let budget =
            NisarLiveNetworkBudgetV1::new(case.request.source.expected_size_bytes).unwrap();
        validate_nisar_live_admission(
            &snapshot(&case.request),
            &case.request,
            &case.metadata,
            plan,
            &source_validator,
            &approved_transport(),
            &budget,
        )
    }

    #[test]
    fn generic_live_admission_accepts_a_consistent_scene_without_a_fixed_semantic_sha() {
        let original = ready_case("generic-admission-original");
        let original_sha = derive_nisar_live_semantic_plan_sha256(
            &original.request,
            &original.metadata,
            &original.plan,
        )
        .unwrap();
        let mut request = original.request.clone();
        request.source.provider_product_id = "G4244267663-ASF".to_owned();
        request.source.provider_product_key =
            "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G4244267663-ASF"
                .to_owned();
        request.source.stable_asset_key =
            "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G4244267663-ASF::ASSET::science-rslc-hdf5"
                .to_owned();
        request.request.aoi_sha256 = "a".repeat(64);
        request.mapping.evidence_digest = "b".repeat(64);
        request.validate().unwrap();
        let rebound = rebind_ready_case_request(original, request);
        let rebound_sha = derive_nisar_live_semantic_plan_sha256(
            &rebound.request,
            &rebound.metadata,
            &rebound.plan,
        )
        .unwrap();
        assert_ne!(rebound_sha, original_sha);
        assert_ne!(rebound_sha, NISAR_M2D_B2_SEMANTIC_PLAN_AUTHORITY_SHA256);
        admit_ready_case(&rebound, &rebound.plan).unwrap();
    }

    #[test]
    fn generic_live_admission_fails_closed_on_explicit_invalid_facts() {
        let case = ready_case("generic-admission-invalid-facts");
        for plan in [
            case.plan.with_test_source_bound_mutation(),
            case.plan.with_test_malformed_range_mutation(),
            case.plan.with_test_unqualified_filter_mutation(),
            case.plan.with_test_incomplete_coverage_mutation(),
        ] {
            assert!(admit_ready_case(&case, &plan).is_err());
        }

        let mut invalid_request = case.request.clone();
        invalid_request.request.polarization = "VV".to_owned();
        assert!(validate_nisar_live_admission(
            &snapshot(&case.request),
            &invalid_request,
            &case.metadata,
            &case.plan,
            case.metadata.source_validator(),
            &approved_transport(),
            &NisarLiveNetworkBudgetV1::new(case.request.source.expected_size_bytes).unwrap(),
        )
        .is_err());

        let mut invalid_mapping = case.request.clone();
        invalid_mapping.mapping.algorithm = "opaque_mapping_v0".to_owned();
        assert!(validate_nisar_live_admission(
            &snapshot(&case.request),
            &invalid_mapping,
            &case.metadata,
            &case.plan,
            case.metadata.source_validator(),
            &approved_transport(),
            &NisarLiveNetworkBudgetV1::new(case.request.source.expected_size_bytes).unwrap(),
        )
        .is_err());
    }

    fn aoi() -> Map<String, Value> {
        json!({
            "type": "Polygon",
            "coordinates": [[
                [37.720222204639484, 10.857787104635449],
                [37.721296026021705, 10.857533983552843],
                [37.721144257142576, 10.85690570018474],
                [37.720070447767895, 10.857158813483217],
                [37.720222204639484, 10.857787104635449]
            ]]
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn snapshot_item() -> SnapshotItem {
        let reference = StableAssetReference {
            provider_id: "asf.nasa.earthdata".to_owned(),
            collection_id: "C2853145197-ASF".to_owned(),
            provider_product_id: "G9900000001-ASF".to_owned(),
            provider_product_key:
                "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF"
                    .to_owned(),
            canonical_product_key: None,
            asset_key: "science-rslc-hdf5".to_owned(),
            stable_asset_key:
                "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF::ASSET::science-rslc-hdf5"
                    .to_owned(),
        };
        SnapshotItem {
            item_id: reference.provider_product_key.clone(),
            provider_id: reference.provider_id.clone(),
            display_name: GRANULE_ID.to_owned(),
            acquisition_datetime: Some("2026-07-26T15:58:03Z".to_owned()),
            footprint_geojson: Some(
                json!({
                    "type": "Polygon",
                    "coordinates": [[[36.0, 9.0], [39.0, 9.0], [39.0, 12.0], [36.0, 12.0], [36.0, 9.0]]]
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
            metadata: Map::from_iter([
                ("band".to_owned(), json!("L")),
                ("processing_level".to_owned(), json!("RSLC")),
                ("science_level".to_owned(), json!("L1")),
                ("production_configuration".to_owned(), json!("PR")),
                ("crid".to_owned(), json!("P05023")),
                ("product_specification_version".to_owned(), json!("1.5.0")),
                (
                    "collection_profile_id".to_owned(),
                    json!("asf.nisar.l.rslc.provisional.v1"),
                ),
                ("frequency_a_polarization".to_owned(), json!("HH+HV")),
                ("frequency_b_polarization".to_owned(), json!("HH")),
            ]),
            payload: Map::new(),
            asset_access: Some(FrozenAssetAccess {
                reference: reference.clone(),
                descriptor: AssetAccessDescriptor {
                    resolver_id: "earthdata_https".to_owned(),
                    scheme: AssetAccessScheme::EarthdataHttps,
                    stable_href: None,
                    requires_credentials: false,
                    credential_profile: None,
                    resolver_parameters: AssetResolverParameters {},
                },
                role: AssetRole::Primary,
                file_name: Some(FILE_NAME.to_owned()),
                expected_size_bytes: Some(SOURCE_BYTES),
                checksum_algorithm: Some("md5".to_owned()),
                checksum_value: Some(SOURCE_MD5.to_owned()),
            }),
            execution_policy: Some(AssetExecutionPolicy {
                authority: AssetExecutionAuthority::ResolverAuthoritative,
                policy_id: "asf.nisar.earthdata-complete-hdf5.v2".to_owned(),
                collection_profile_id: Some("asf.nisar.l.rslc.provisional.v1".to_owned()),
            }),
        }
    }

    struct LocalSourceRangeServer {
        url: String,
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        stop: std::sync::Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl LocalSourceRangeServer {
        fn start(source: PathBuf, total: u64) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let captured = std::sync::Arc::clone(&requests);
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let stop_flag = std::sync::Arc::clone(&stop);
            let worker = std::thread::spawn(move || {
                loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // The listener is nonblocking for the stop-flag
                            // poll; accepted streams must go back to blocking
                            // mode so large Range bodies stream reliably.
                            let _ = stream.set_nonblocking(false);
                            let request = read_request(&mut stream);
                            let (start, end_exclusive) = parse_range(&request);
                            captured.lock().unwrap().push(request);
                            let mut source_file = fs::File::open(&source).unwrap();
                            source_file.seek(std::io::SeekFrom::Start(start)).unwrap();
                            let mut body =
                                vec![0_u8; usize::try_from(end_exclusive - start).unwrap()];
                            source_file.read_exact(&mut body).unwrap();
                            let header = format!(
                                "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nETag: {ETAG}\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{}/{total}\r\nContent-Length: {}\r\n\r\n",
                                end_exclusive - 1,
                                body.len()
                            );
                            stream.write_all(header.as_bytes()).unwrap();
                            stream.write_all(&body).unwrap();
                            stream.flush().unwrap();
                        }
                        Err(_) => {
                            if stop_flag.load(Ordering::SeqCst) {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                }
            });
            Self {
                url: format!("http://{address}/oracle.h5"),
                requests,
                stop,
                worker: Some(worker),
            }
        }

        fn request_count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }

        fn captured_requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Drop for LocalSourceRangeServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn hash_file(path: &Path, algorithm: &str) -> String {
        let mut file = fs::File::open(path).unwrap();
        let mut buffer = [0_u8; 256 * 1024];
        match algorithm {
            "sha256" => {
                let mut hasher = Sha256::new();
                loop {
                    let count = file.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    hasher.update(&buffer[..count]);
                }
                format!("{:x}", hasher.finalize())
            }
            "md5" => {
                use md5::{Digest as Md5Digest, Md5};
                let mut hasher = Md5::new();
                loop {
                    let count = file.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    hasher.update(&buffer[..count]);
                }
                format!("{:x}", hasher.finalize())
            }
            _ => unreachable!(),
        }
    }

    fn read_axis(file: &File, path: &str) -> Vec<f64> {
        file.dataset(path).unwrap().read_raw().unwrap()
    }

    struct QualificationClock;

    impl Clock for QualificationClock {
        fn now(&self) -> SystemTime {
            UNIX_EPOCH
        }
    }

    #[derive(Default)]
    struct QualificationCredentials;

    impl CredentialProvider for QualificationCredentials {
        fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
            assert_eq!(profile, "earthdata-default");
            CredentialMaterial::bearer("LOCAL_QUALIFICATION_TOKEN")
        }
    }

    struct QualificationEarthdataResolver {
        url: String,
    }

    impl AssetResolver for QualificationEarthdataResolver {
        fn resolver_id(&self) -> &'static str {
            "earthdata_https"
        }

        fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
            &[AssetAccessScheme::EarthdataHttps]
        }

        fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
            descriptor.scheme == AssetAccessScheme::EarthdataHttps
        }

        fn resolve(
            &self,
            _reference: &StableAssetReference,
            descriptor: &AssetAccessDescriptor,
            context: &ResolveContext<'_>,
        ) -> Result<ResolvedAsset, ResolveError> {
            context.check_cancelled()?;
            let profile = descriptor
                .credential_profile
                .as_deref()
                .ok_or_else(ResolveError::descriptor_invalid)?;
            let credential = context
                .credential_provider()
                .resolve(profile)
                .map_err(|_| ResolveError::credentials_missing())?;
            let mut asset = ResolvedAsset::new(&self.url, ResolvedMethod::Get)?;
            credential.apply_to(&mut asset)?;
            Ok(asset)
        }

        fn plan_shadow(
            &self,
            _frozen: &FrozenAssetAccess,
            _context: &ShadowResolveContext<'_>,
        ) -> Result<ShadowAccessIntent, ResolveError> {
            Err(ResolveError::failed())
        }
    }

    fn local_production_runtime(server_url: String) -> ResolverExecutionRuntime {
        let mut registry = AssetResolverRegistry::new();
        registry
            .register(std::sync::Arc::new(QualificationEarthdataResolver {
                url: server_url,
            }))
            .unwrap();
        ResolverExecutionRuntime::with_dependencies(
            registry,
            std::sync::Arc::new(QualificationCredentials),
            std::sync::Arc::new(QualificationClock),
        )
    }

    fn local_production_worker(server_url: String) -> ProductionDownloadWorker {
        let runtime = local_production_runtime(server_url);
        let options = TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(60),
            chunk_size: 64 * 1024,
            ..TransferOptions::default()
        };
        let transfer = std::sync::Arc::new(
            HttpTransfer::new_with_proxy_policy(options, ProxyPolicy::Direct).unwrap(),
        );
        ProductionDownloadWorker::new(options)
            .unwrap()
            .with_resolver_runtime_and_transfer(runtime, transfer)
    }

    fn assert_no_secret_in_textual_evidence(root: &Path, secret: &str) {
        fn walk(path: &Path, secret: &[u8]) {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, secret);
                } else if matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some("json" | "log" | "txt" | "md")
                ) {
                    let body = fs::read(&path).unwrap();
                    assert!(
                        !body.windows(secret.len()).any(|window| window == secret),
                        "credential leaked into evidence file: {}",
                        path.display()
                    );
                }
            }
        }
        walk(root, secret.as_bytes());
    }

    fn expected_science_request_count(plan: &Hdf5RangePlanV1) -> usize {
        plan.ranges()
            .iter()
            .skip(1)
            .map(|range| {
                let bytes = range.end_exclusive - range.start;
                usize::try_from(
                    bytes.div_ceil(crate::hdf5_range_transfer::HDF5_RANGE_NETWORK_SLICE_BYTES),
                )
                .unwrap()
            })
            .sum()
    }

    #[test]
    #[ignore = "requires the verified local full NISAR RSLC oracle and is not an ordinary workspace regression"]
    fn local_production_worker_remote_subset_qualification() {
        crate::establish_hdf5_plugin_isolation()
            .expect("app-owned HDF5 plugin isolation is required by the qualification harness");
        let source = std::env::var("NISAR_LOCAL_RSLC_SOURCE").unwrap_or_else(|_| {
            r"C:\Data\EOStage\NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001.h5"
                .to_owned()
        });
        let source_path = PathBuf::from(&source);
        assert_eq!(
            fs::metadata(&source_path)
                .expect("local full source must exist")
                .len(),
            SOURCE_BYTES
        );
        if std::env::var("NISAR_LOCAL_RSLC_TRUST_VERIFIED")
            .ok()
            .as_deref()
            != Some("1")
        {
            assert_eq!(hash_file(&source_path, "sha256"), SOURCE_SHA256);
            assert_eq!(hash_file(&source_path, "md5"), SOURCE_MD5);
        }

        let root = super::TempDirectory::new("local-production-worker-qualification");
        let output_root = root.0.join("output");
        // Keep the exact frozen source identity used by the M2C candidate,
        // while enriching the historical local qualification item with the
        // full production resolver graph needed by ProductionWorker.
        let mut item = snapshot_item();
        let mut production_metadata =
            crate::test_suites::nisar_support::frozen_item_with_source_identity(
                SOURCE_BYTES,
                SOURCE_SHA256.to_owned(),
            )
            .metadata;
        let legacy_product_id = item
            .asset_access
            .as_ref()
            .unwrap()
            .reference
            .provider_product_id
            .clone();
        let legacy_product_key = item.item_id.clone();
        let official_href = format!(
            "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/NISAR_L1_RSLC_PROVISIONAL_V1/{GRANULE_ID}/{FILE_NAME}"
        );
        if let Some(access) = item.asset_access.as_mut() {
            access.descriptor.requires_credentials = true;
            access.descriptor.credential_profile = Some("earthdata-default".to_owned());
            access.descriptor.stable_href = Some(official_href.clone());
            access.checksum_algorithm = Some("MD5".to_owned());
        }
        if let Some(properties) = production_metadata
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            properties.insert("provider_product_id".to_owned(), json!(legacy_product_id));
            properties.insert(
                "provider_product_key".to_owned(),
                json!(item.item_id.clone()),
            );
            properties.insert(
                "cmr_granule_concept_id".to_owned(),
                json!(legacy_product_id),
            );
            properties.insert("producer_granule_id".to_owned(), json!(GRANULE_ID));
            properties.insert("asf_product_id".to_owned(), json!(GRANULE_ID));
        }
        if let Some(assets) = production_metadata
            .get_mut("assets")
            .and_then(Value::as_array_mut)
        {
            for asset in assets {
                if let Some(asset) = asset.as_object_mut() {
                    let asset_key = asset
                        .get("asset_key")
                        .and_then(Value::as_str)
                        .unwrap()
                        .to_owned();
                    asset.insert(
                        "stable_asset_key".to_owned(),
                        json!(
                            insar_core::stable_asset_key(&legacy_product_key, &asset_key).unwrap()
                        ),
                    );
                    if asset.get("role").and_then(Value::as_str) == Some("primary") {
                        asset.insert("file_name".to_owned(), json!(FILE_NAME));
                        asset.insert(
                            "checksum".to_owned(),
                            json!({"algorithm": "MD5", "value": SOURCE_MD5}),
                        );
                        asset.insert(
                            "routes".to_owned(),
                            json!([
                                {"scheme": "https", "locator": official_href.clone()},
                                {"scheme": "s3", "locator": format!("s3://sds-n-cumulus-prod-nisar-products/NISAR/{FILE_NAME}")}
                            ]),
                        );
                        if let Some(evidence) = asset
                            .get_mut("native_evidence")
                            .and_then(Value::as_object_mut)
                        {
                            evidence.insert("archive_name".to_owned(), json!(FILE_NAME));
                        }
                    }
                }
            }
        }
        production_metadata.insert(
            "provider_product_id".to_owned(),
            json!(item
                .asset_access
                .as_ref()
                .unwrap()
                .reference
                .provider_product_id
                .clone()),
        );
        production_metadata.insert(
            "provider_product_key".to_owned(),
            json!(item.item_id.clone()),
        );
        item.metadata = production_metadata;
        // The local oracle has the original subgrid footprint and filename;
        // the enriched resolver graph must retain that same source identity.
        let selection_aoi = aoi();
        item.metadata
            .insert("delivery_kind".to_owned(), json!("remote_derived_subset"));
        item.metadata
            .insert("subset_frequency".to_owned(), json!("A"));
        item.metadata
            .insert("subset_polarization".to_owned(), json!("HH"));
        item.metadata.insert(
            "subset_aoi_geojson".to_owned(),
            Value::Object(selection_aoi.clone()),
        );
        for (key, value) in [
            ("band", json!("L")),
            ("science_level", json!("L1")),
            ("production_configuration", json!("PR")),
            ("crid", json!("P05023")),
            ("product_specification_version", json!("1.5.0")),
            ("frequency_a_polarization", json!("HH")),
            ("frequency_b_polarization", json!("VV")),
        ] {
            item.metadata.insert(key.to_owned(), value);
        }
        let task_id = "production-worker-local";
        let work = crate::test_suites::nisar_support::work(&output_root, item.clone(), task_id);
        let server = LocalSourceRangeServer::start(source_path.clone(), SOURCE_BYTES);
        let worker = local_production_worker(server.url.clone());
        let result = worker.execute(work, WorkerControl::default(), &mut |_progress| {});
        assert_eq!(
            result.outcome,
            DownloadOutcome::Completed,
            "production worker failure: {:?}",
            result.failure
        );
        let expected_destination = output_root.join(format!(
            "{}_subset_v1.h5",
            FILE_NAME.strip_suffix(".h5").unwrap()
        ));
        assert_eq!(
            result.destination.file_name(),
            expected_destination.file_name()
        );
        let destination = result.destination.clone();
        assert!(destination.is_file());
        assert!(fs::read_dir(&output_root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains("writing.h5")));

        let workspace = Hdf5RangeWorkspaceV1::new(
            output_root
                .join(".insar-nisar-range")
                .join("nisar-production-worker-local"),
        )
        .unwrap();
        let part = workspace.part_path();
        let validator = NisarSubsetSourceValidatorV1 {
            kind: NisarSubsetValidatorKind::StrongEtag,
            value: ETAG.to_owned(),
            content_length: SOURCE_BYTES,
        };
        let request = build_live_nisar_subset_request(
            &item,
            &selection_aoi,
            NisarSubsetFrequency::A,
            "HH",
            "LSAR_RSLC",
            &part,
        )
        .expect("worker must use the production request builder");
        let metadata = NisarSubsetMetadataSnapshotV1::from_verified_live_metadata_hdf5(
            &request,
            validator,
            &part,
            &|| false,
        )
        .expect("worker must leave verifiable metadata evidence");
        let plan =
            live_nisar_range_plan_from_sparse_hdf5(&request, &part, metadata.evidence_sha256())
                .expect("worker must leave a verifiable physical Range plan");
        let semantic_plan_sha256 =
            derive_nisar_live_semantic_plan_sha256(&request, &metadata, &plan)
                .expect("worker must leave a verifiable semantic authority");
        let semantic_snapshot = crate::semantic_plan_canonical_snapshot(&request, &metadata, &plan)
            .expect("worker must expose the canonical semantic authority");
        eprintln!(
            "NISAR_QUALIFICATION_SEMANTIC={}",
            serde_json::to_string(&semantic_snapshot)
                .expect("canonical semantic authority serializes")
        );
        assert_eq!(
            plan.plan_sha256(),
            "05a98f570c2cd3e95421c350499edc640846f77e80aa7767063e72a7c2146a24"
        );
        assert_eq!(
            semantic_plan_sha256,
            NISAR_M2D_B2_SEMANTIC_PLAN_AUTHORITY_SHA256
        );
        assert_eq!(
            NISAR_LIVE_PLAN_SEMANTIC_AUTHORITY_CONTRACT_V1,
            "NisarLivePlanSemanticAuthorityV1"
        );
        assert_eq!(source_dataset_count(&File::open(&part).unwrap()), 207);
        assert_eq!(metadata.source_inventory_sha256(), INVENTORY_V2_SHA256);

        let window = &request.mapping.windows[0];
        assert_eq!(
            (
                window.line_start,
                window.line_end_exclusive,
                window.sample_start,
                window.sample_end_exclusive,
            ),
            (26_112, 26_624, 11_776, 13_824)
        );
        assert!(
            window.line_start <= REFERENCE_LINE_START
                && window.line_end_exclusive >= REFERENCE_LINE_END
                && window.sample_start <= REFERENCE_SAMPLE_START
                && window.sample_end_exclusive >= REFERENCE_SAMPLE_END
        );

        let requests = server.captured_requests();
        let spans = requests
            .iter()
            .map(|request| parse_range(request))
            .collect::<Vec<_>>();
        assert!(
            spans.len() >= 3,
            "prefix, discovery, and science are all required"
        );
        assert_eq!(
            spans[0],
            (0, 1),
            "preflight must be the first Range request"
        );
        assert_eq!(
            spans[1],
            (0, NISAR_LIVE_METADATA_PREFIX_BYTES),
            "the fixed metadata prefix must be fetched before discovery"
        );
        let science_request_count = expected_science_request_count(&plan);
        assert!(science_request_count > 0);
        assert!(spans.len() > 2 + science_request_count);
        let discovery_request_count = spans.len() - 2 - science_request_count;
        assert!(
            discovery_request_count > 0,
            "metadata discovery was omitted"
        );
        assert!(spans[2..2 + discovery_request_count]
            .iter()
            .any(|(start, _)| *start >= NISAR_LIVE_METADATA_PREFIX_BYTES));
        assert!(requests.iter().all(|request| request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("range: bytes="))));
        assert!(requests.iter().all(|request| !request.contains('?')));
        let response_body_bytes = spans.iter().map(|(start, end)| end - start).sum::<u64>();
        assert!(spans.len() as u64 <= 256);
        assert!(response_body_bytes <= 402_653_184);
        assert!(response_body_bytes <= SOURCE_BYTES * 5 / 100);
        assert_no_secret_in_textual_evidence(&output_root, "LOCAL_QUALIFICATION_TOKEN");

        let source_file = File::open(&source_path).unwrap();
        let output_file = File::open(&destination).unwrap();
        let source_science: Array2<CFloat32> = source_file
            .dataset(SCIENCE_PATH)
            .unwrap()
            .read_slice_2d((
                window.line_start as usize..window.line_end_exclusive as usize,
                window.sample_start as usize..window.sample_end_exclusive as usize,
            ))
            .unwrap();
        let produced_science: Array2<CFloat32> = output_file
            .dataset(OUTPUT_SCIENCE_PATH)
            .unwrap()
            .read_2d()
            .unwrap();
        assert_eq!(source_science, produced_science);
        let source_mask: Array2<u8> = source_file
            .dataset(super::MASK_PATH)
            .unwrap()
            .read_slice_2d((
                window.line_start as usize..window.line_end_exclusive as usize,
                window.sample_start as usize..window.sample_end_exclusive as usize,
            ))
            .unwrap();
        let produced_mask: Array2<u8> = output_file
            .dataset(OUTPUT_MASK_PATH)
            .unwrap()
            .read_2d()
            .unwrap();
        assert_eq!(source_mask, produced_mask);
    }

    #[test]
    #[ignore = "diagnoses one preserved live Store item against the verified local source"]
    fn local_production_worker_live_store_item_forensic() {
        crate::establish_hdf5_plugin_isolation()
            .expect("app-owned HDF5 plugin isolation is required by the forensic harness");
        let source_path = PathBuf::from(
            std::env::var("NISAR_LOCAL_RSLC_SOURCE").expect("verified local source path"),
        );
        assert_eq!(fs::metadata(&source_path).unwrap().len(), SOURCE_BYTES);
        let item: SnapshotItem = serde_json::from_slice(
            &fs::read(
                std::env::var("NISAR_LIVE_STORE_ITEM_JSON")
                    .expect("preserved live Store item JSON path"),
            )
            .expect("preserved live Store item reads"),
        )
        .expect("preserved live Store item parses");
        let root = super::TempDirectory::new("live-store-item-forensic");
        let output_root = root.0.join("output");
        let server = LocalSourceRangeServer::start(source_path, SOURCE_BYTES);
        let worker = local_production_worker(server.url.clone());
        let result = worker.execute(
            crate::test_suites::nisar_support::work(&output_root, item, "live-store-item-forensic"),
            WorkerControl::default(),
            &mut |_progress| {},
        );
        eprintln!("NISAR_LIVE_STORE_ITEM_RESULT={result:?}");
        assert_eq!(
            result.outcome,
            DownloadOutcome::Completed,
            "live Store item worker failure: {:?}",
            result.failure
        );
    }

    #[test]
    #[ignore = "replays one preserved completed live Range workspace without network access"]
    fn preserved_live_range_workspace_writer_forensic() {
        crate::establish_hdf5_plugin_isolation()
            .expect("forensic host establishes app-owned HDF5 plugin isolation");
        let item: SnapshotItem = serde_json::from_slice(
            &fs::read(
                std::env::var("NISAR_LIVE_STORE_ITEM_JSON")
                    .expect("preserved live Store item JSON path"),
            )
            .expect("preserved live Store item reads"),
        )
        .expect("preserved live Store item parses");
        let aoi = item
            .metadata
            .get("subset_aoi_geojson")
            .and_then(Value::as_object)
            .cloned()
            .expect("preserved live subset AOI");
        let workspace = Hdf5RangeWorkspaceV1::new(PathBuf::from(
            std::env::var("NISAR_PRESERVED_TRANSFER_WORKSPACE")
                .expect("preserved live transfer workspace path"),
        ))
        .expect("preserved live transfer workspace");
        let manifest = workspace
            .load_manifest()
            .expect("preserved live transfer manifest")
            .expect("completed live transfer manifest");
        let part = workspace.part_path();
        let request = build_live_nisar_subset_request(
            &item,
            &aoi,
            NisarSubsetFrequency::A,
            "HH",
            "LSAR_RSLC",
            &part,
        )
        .expect("preserved live request rebuilds");
        let metadata = NisarSubsetMetadataSnapshotV1::from_verified_live_metadata_hdf5(
            &request,
            manifest.source_validator.clone(),
            &part,
            &|| false,
        )
        .expect("preserved live metadata snapshot");
        let plan =
            live_nisar_range_plan_from_sparse_hdf5(&request, &part, metadata.evidence_sha256())
                .expect("preserved live physical plan");
        let transfer = workspace
            .recover_completed_result_read_only_for_test(&plan)
            .expect("preserved live completed transfer recovery");
        if let Ok(candidate_path) = std::env::var("NISAR_PRESERVED_WRITER_CANDIDATE") {
            let (_workspace_lock, blocks) = workspace
                .acquire_verified_chunk_blocks(&request, &plan, &transfer, &|| false)
                .expect("preserved live verified chunk blocks");
            let materials = super::super::build_window_materials(&request, &blocks, &metadata)
                .expect("preserved live candidate window materials");
            let candidate_path = PathBuf::from(candidate_path);
            let candidate = super::super::CandidateFileGuard::open(&candidate_path)
                .expect("preserved live writer candidate opens read-only");
            let validation = super::super::validate_candidate(
                &candidate,
                &request,
                &transfer,
                &metadata,
                &materials,
                &blocks,
                &|| false,
            )
            .unwrap_or_else(|error| panic!("PRESERVED_LIVE_CANDIDATE_ERROR={error:?}"));
            let artifact_size_bytes = validation.artifact_size_bytes;
            let artifact_sha256 = validation.artifact_sha256.clone();
            let artifact_manifest =
                super::super::build_artifact_manifest(&request, &transfer, &materials, validation)
                    .unwrap_or_else(|error| panic!("PRESERVED_LIVE_MANIFEST_ERROR={error:?}"));
            eprintln!(
                "PRESERVED_LIVE_CANDIDATE_VALID size={} sha256={} manifest_complete={}",
                artifact_size_bytes, artifact_sha256, artifact_manifest.complete
            );
        }
        let output_root = PathBuf::from(
            std::env::var("NISAR_PRESERVED_WRITER_OUTPUT")
                .expect("preserved live writer output path"),
        );
        let writer = OfflineNisarSubsetWriter::new(
            output_root,
            DestinationLeaseController::new_for_qualification_test(),
        )
        .expect("preserved live writer");
        let written = writer
            .write_and_validate(&request, &plan, &transfer, &workspace, &metadata, &|| false)
            .unwrap_or_else(|error| panic!("PRESERVED_LIVE_WRITER_ERROR={error:?}"));
        eprintln!(
            "PRESERVED_LIVE_WRITER_OUTPUT={} complete={}",
            written.artifact_path().display(),
            written.manifest().complete
        );
    }

    #[test]
    #[ignore = "requires the preserved C2 sparse metadata file; performs no network access"]
    fn c2_preserved_sparse_fixture_generalizes_and_emits_oracle_input() {
        crate::establish_hdf5_plugin_isolation()
            .expect("app-owned HDF5 plugin isolation is required by the diagnostic");
        let source = PathBuf::from(
            std::env::var("NISAR_C2_SPARSE_SOURCE")
                .expect("NISAR_C2_SPARSE_SOURCE must name the preserved sparse file"),
        );
        let file = File::open(&source).expect("preserved sparse HDF5 must open read-only");
        let science = file
            .dataset("/science/LSAR/RSLC/swaths/frequencyA/HH")
            .expect("C2 A/HH science dataset");
        let shape = science.shape();
        let chunk = science
            .dcpl()
            .unwrap()
            .get_chunk()
            .unwrap()
            .expect("chunked C2 science dataset");
        let group = file
            .group("/science/LSAR/RSLC/metadata/geolocationGrid")
            .unwrap();
        let height_values: Vec<f64> = group
            .dataset("heightAboveEllipsoid")
            .unwrap()
            .read_raw()
            .unwrap();
        let coordinate_x = group.dataset("coordinateX").unwrap();
        let coordinate_y = group.dataset("coordinateY").unwrap();
        let x_shape = coordinate_x.shape();
        let x_values: Vec<f64> = coordinate_x.read_raw().unwrap();
        let y_values: Vec<f64> = coordinate_y.read_raw().unwrap();
        let stride = x_shape[1] * x_shape[2];
        let longitudes = x_values
            .chunks_exact(stride)
            .map(<[f64]>::to_vec)
            .collect::<Vec<_>>();
        let latitudes = y_values
            .chunks_exact(stride)
            .map(<[f64]>::to_vec)
            .collect::<Vec<_>>();
        let grid_time = read_axis(
            &file,
            "/science/LSAR/RSLC/metadata/geolocationGrid/zeroDopplerTime",
        );
        let grid_range = read_axis(
            &file,
            "/science/LSAR/RSLC/metadata/geolocationGrid/slantRange",
        );
        let swath_time = read_axis(&file, "/science/LSAR/RSLC/swaths/zeroDopplerTime");
        let swath_range = read_axis(&file, "/science/LSAR/RSLC/swaths/frequencyA/slantRange");
        let aoi = json!({
            "type": "Polygon",
            "coordinates": [[
                [-55.00, -61.36],
                [-54.78, -61.36],
                [-54.78, -61.16],
                [-55.00, -61.16],
                [-55.00, -61.36]
            ]]
        })
        .as_object()
        .unwrap()
        .clone();
        let evidence = build_live_geolocation_evidence(
            &aoi,
            NisarSubsetFrequency::A,
            "HH",
            "LSAR_RSLC",
            &grid_time,
            &grid_range,
            &swath_time,
            &swath_range,
            height_values.clone(),
            longitudes,
            latitudes,
            [shape[0], shape[1]],
            [chunk[0], chunk[1]],
            *height_values.first().unwrap(),
            *height_values.last().unwrap(),
        )
        .unwrap_or_else(|error| panic!("C2_MAPPING_ERROR={error:?}"));
        eprintln!("C2_MAPPING_WINDOWS={:?}", evidence.windows);
        // Release every HDF5 handle before the production-equivalent transfer
        // recovery opens the sparse part with the exclusive Windows identity
        // boundary used by the real worker.
        drop(coordinate_x);
        drop(coordinate_y);
        drop(group);
        drop(science);
        drop(file);
        let store_path = PathBuf::from(
            std::env::var("NISAR_C2_STORE")
                .expect("NISAR_C2_STORE must name the preserved C2 task store"),
        );
        let store = insar_store::TaskStore::open(&store_path).expect("C2 task store");
        let workbench = store
            .load_workbench()
            .unwrap()
            .expect("preserved C2 workbench");
        let item = workbench
            .batch
            .candidates
            .iter()
            .find(|item| {
                item.display_name
                    .starts_with("NISAR_L1_PR_RSLC_025_017_A_147_7700")
            })
            .expect("C2 source item");
        let request = build_live_nisar_subset_request(
            item,
            &aoi,
            NisarSubsetFrequency::A,
            "HH",
            "LSAR_RSLC",
            &source,
        )
        .unwrap_or_else(|error| panic!("C2_REQUEST_ERROR={error:?}"));
        eprintln!("C2_REQUEST_WINDOWS={:?}", request.mapping.windows);
        let transfer_workspace = std::env::var("NISAR_C2_TRANSFER_WORKSPACE")
            .ok()
            .map(PathBuf::from)
            .map(Hdf5RangeWorkspaceV1::new)
            .transpose()
            .expect("preserved C2 transfer workspace");
        let source_validator = if let Some(workspace) = transfer_workspace.as_ref() {
            workspace
                .load_manifest()
                .expect("preserved C2 transfer manifest")
                .expect("completed C2 transfer manifest")
                .source_validator
        } else {
            NisarSubsetSourceValidatorV1 {
                kind: NisarSubsetValidatorKind::StrongEtag,
                value: "\"c2-local-oracle-input\"".to_owned(),
                content_length: request.source.expected_size_bytes,
            }
        };
        let metadata = NisarSubsetMetadataSnapshotV1::from_verified_live_metadata_hdf5(
            &request,
            source_validator.clone(),
            &source,
            &|| false,
        )
        .expect("C2 single-frequency metadata inventory must be structurally admitted");
        assert_eq!(metadata.source_dataset_count(), 138);
        assert_eq!(request.expected_output.estimated_remote_bytes, 465_986_898);
        let plan =
            live_nisar_range_plan_from_sparse_hdf5(&request, &source, metadata.evidence_sha256())
                .expect("C2 physical plan");
        assert_eq!(plan.ranges().len(), 79);
        let nominal_science_requests = plan
            .ranges()
            .iter()
            .skip(1)
            .map(|range| {
                (range.end_exclusive - range.start)
                    .div_ceil(crate::hdf5_range_transfer::HDF5_RANGE_NETWORK_SLICE_BYTES)
            })
            .sum::<u64>();
        assert_eq!(nominal_science_requests, 78);
        assert!(7 + 2 * nominal_science_requests <= crate::NISAR_LIVE_MAX_REQUESTS);
        let budget = NisarLiveNetworkBudgetV1::new(request.source.expected_size_bytes).unwrap();
        for _ in 0..7 {
            budget.reserve_request().unwrap();
        }
        budget
            .reserve_response_body_bytes(NISAR_LIVE_METADATA_PREFIX_BYTES + 1)
            .unwrap();
        budget
            .validate_planned_science(&plan)
            .expect("C2 4 MiB science slicing must fit the request and byte budgets");
        let gate = crate::NisarIsce3LiveGeometryGateV1 {
            input_path: PathBuf::from(
                std::env::var("NISAR_C2_ORACLE_INPUT").expect("C2 oracle input path"),
            ),
            output_path: PathBuf::from(
                std::env::var("NISAR_C2_ORACLE_OUTPUT").expect("C2 oracle output path"),
            ),
            comparison_path: PathBuf::from(
                std::env::var("NISAR_C2_GEOMETRY_COMPARISON").expect("C2 geometry comparison path"),
            ),
        };
        let gate_result = crate::nisar_isce3_oracle::enforce_isce3_geometry_gate(
            &gate, &request, &metadata, &source, &aoi,
        );
        if gate.output_path.is_file() {
            gate_result.expect("C2 ISCE3 result must contain the Studio window");
            assert!(gate.comparison_path.is_file());
        } else {
            assert_eq!(
                gate_result,
                Err(crate::nisar_isce3_oracle::NisarIsce3GeometryGateError::OracleRequired)
            );
        }
        assert!(gate.input_path.is_file());

        // Optional C2 writer replay. This read-only recovery validates the
        // preserved plan, manifest, metrics, part identity, and every completed
        // Range digest without opening an HTTP execution path.
        if let Some(workspace) = transfer_workspace {
            let lease = DestinationLeaseController::new_for_qualification_test();
            let transfer = workspace
                .recover_completed_result_read_only_for_test(&plan)
                .expect("C2 completed transfer read-only recovery");
            let output_root = PathBuf::from(
                std::env::var("NISAR_C2_WRITER_OUTPUT")
                    .expect("NISAR_C2_WRITER_OUTPUT is required for writer replay"),
            );
            let writer = OfflineNisarSubsetWriter::new(output_root, lease).unwrap();
            let written = writer
                .write_and_validate(&request, &plan, &transfer, &workspace, &metadata, &|| false)
                .unwrap_or_else(|error| panic!("C2_WRITER_ERROR={error:?}"));
            eprintln!(
                "C2_WRITER_OUTPUT={} complete={} sha256={}",
                written.artifact_path().display(),
                written.manifest().complete,
                hash_file(written.artifact_path(), "sha256")
            );
        }
    }

    #[test]
    #[ignore = "requires the verified local full NISAR RSLC source (~14.25 GB read) and runs the production-equivalent Range chain against it"]
    fn local_full_source_production_equivalent_chain_qualifies() {
        crate::establish_hdf5_plugin_isolation()
            .expect("app-owned HDF5 plugin isolation is required by the qualification harness");
        let started = Instant::now();
        let source = std::env::var("NISAR_LOCAL_RSLC_SOURCE").unwrap_or_else(|_| {
            r"C:\Data\EOStage\NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001.h5"
                .to_owned()
        });
        let source_path = PathBuf::from(&source);
        let length = fs::metadata(&source_path)
            .expect("local full source must exist")
            .len();
        assert_eq!(length, SOURCE_BYTES, "local source size mismatch");
        let sha256 = hash_file(&source_path, "sha256");
        assert_eq!(sha256, SOURCE_SHA256, "local source SHA-256 mismatch");
        let md5 = hash_file(&source_path, "md5");
        assert_eq!(md5, SOURCE_MD5, "local source CMR MD5 mismatch");

        let root = super::TempDirectory::new("local-qualification");
        let workspace = Hdf5RangeWorkspaceV1::new(root.0.join("transfer")).unwrap();
        let http = HttpTransfer::new_with_proxy_policy(
            TransferOptions {
                max_attempts: 1,
                backoff: Duration::ZERO,
                timeout: Duration::from_secs(60),
                chunk_size: 64 * 1024,
                ..TransferOptions::default()
            },
            ProxyPolicy::Direct,
        )
        .unwrap();
        let server = LocalSourceRangeServer::start(source_path.clone(), SOURCE_BYTES);
        let asset = ResolvedAsset::new(&server.url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(SOURCE_BYTES))
            .with_checksum(Some(ResolvedChecksum::new("md5", SOURCE_MD5).unwrap()))
            .with_remote_validator(RemoteValidator {
                etag: Some(ETAG.to_owned()),
                last_modified: None,
                content_length: Some(SOURCE_BYTES),
            });
        let validator = NisarSubsetSourceValidatorV1 {
            kind: NisarSubsetValidatorKind::StrongEtag,
            value: ETAG.to_owned(),
            content_length: SOURCE_BYTES,
        };
        let cancelled = || false;

        // 1. Simulated verified metadata prefix acquisition (loopback Range).
        let prefetch = prefetch_live_nisar_metadata(
            &http,
            &asset,
            &workspace,
            &validator,
            SOURCE_BYTES,
            &cancelled,
        )
        .expect("prefix acquisition");
        assert_eq!(
            prefetch.response_body_bytes(),
            NISAR_LIVE_METADATA_PREFIX_BYTES
        );

        // 2. Metadata discovery / materialization of payloads past the prefix.
        let discovery = discover_live_nisar_metadata_chunks(
            &http,
            &asset,
            &workspace,
            &validator,
            SOURCE_BYTES,
            &cancelled,
        )
        .expect("metadata discovery");

        // 3. Real radar-axis binding diagnostics from the sparse part.
        let part = workspace.part_path();
        {
            let file = File::open(&part).unwrap();
            let grid_time = read_axis(
                &file,
                "/science/LSAR/RSLC/metadata/geolocationGrid/zeroDopplerTime",
            );
            let grid_range = read_axis(
                &file,
                "/science/LSAR/RSLC/metadata/geolocationGrid/slantRange",
            );
            let swath_time = read_axis(&file, "/science/LSAR/RSLC/swaths/zeroDopplerTime");
            let swath_range = read_axis(&file, "/science/LSAR/RSLC/swaths/frequencyA/slantRange");
            let (rows, row_diag) = nearest_radar_axis_indices(&grid_time, &swath_time)
                .expect("radar time-axis binding must be value-proven");
            let (columns, column_diag) = nearest_radar_axis_indices(&grid_range, &swath_range)
                .expect("radar slant-range-axis binding must be value-proven");
            eprintln!(
                "RADAR_AXIS time={} full={} max_residual={:.3e} mapped_first={} mapped_last={} increasing={} median_spacing={:.6e} kept_grid=[{},{}] margin_dropped={}",
                row_diag.grid_length,
                row_diag.full_length,
                row_diag.max_nearest_residual,
                row_diag.mapped_first_index,
                row_diag.mapped_last_index,
                row_diag.grid_increasing,
                row_diag.median_full_spacing.unwrap_or(f64::NAN),
                row_diag.kept_first_grid_index,
                row_diag.kept_last_grid_index,
                row_diag.margin_nodes_dropped
            );
            eprintln!(
                "RADAR_AXIS range={} full={} max_residual={:.3e} mapped_first={} mapped_last={} increasing={} median_spacing={:.6e} kept_grid=[{},{}] margin_dropped={}",
                column_diag.grid_length,
                column_diag.full_length,
                column_diag.max_nearest_residual,
                column_diag.mapped_first_index,
                column_diag.mapped_last_index,
                column_diag.grid_increasing,
                column_diag.median_full_spacing.unwrap_or(f64::NAN),
                column_diag.kept_first_grid_index,
                column_diag.kept_last_grid_index,
                column_diag.margin_nodes_dropped
            );
            assert!(row_diag.strictly_increasing);
            assert!(column_diag.strictly_increasing);
            assert_eq!(
                rows.len(),
                row_diag.kept_last_grid_index - row_diag.kept_first_grid_index + 1
            );
            assert_eq!(
                columns.len(),
                column_diag.kept_last_grid_index - column_diag.kept_first_grid_index + 1
            );
        }

        // 4. Request, metadata snapshot, plan (production logic).
        let request = build_live_nisar_subset_request(
            &snapshot_item(),
            &aoi(),
            NisarSubsetFrequency::A,
            "HH",
            "LSAR_RSLC",
            &part,
        )
        .expect("live request construction");
        let metadata_snapshot = NisarSubsetMetadataSnapshotV1::from_verified_live_metadata_hdf5(
            &request,
            validator.clone(),
            &part,
            &cancelled,
        )
        .expect("metadata snapshot");
        let plan = live_nisar_range_plan_from_sparse_hdf5(
            &request,
            &part,
            metadata_snapshot.evidence_sha256(),
        )
        .expect("live range plan");
        let semantic_plan_sha256 =
            crate::derive_nisar_live_semantic_plan_sha256(&request, &metadata_snapshot, &plan)
                .expect("semantic plan authority");
        seed_live_metadata_range(
            &workspace,
            &request,
            &plan,
            &validator,
            prefetch.prefix_sha256(),
            "2026-08-07T00:00:01Z",
        )
        .expect("prefix seed");
        eprintln!("INVENTORY count=207 required=207");
        assert_eq!(source_dataset_count(&File::open(&part).unwrap()), 207);
        eprintln!(
            "INVENTORY v2_sha={} required={}",
            metadata_snapshot.source_inventory_sha256(),
            INVENTORY_V2_SHA256
        );
        eprintln!(
            "INVENTORY source_dataset_count={} metadata_group_binding_count={} metadata_dataset_binding_count={} metadata_binding_count={}",
            metadata_snapshot.source_dataset_count(),
            metadata_snapshot.metadata_group_binding_count(),
            metadata_snapshot.metadata_dataset_binding_count(),
            metadata_snapshot.metadata_binding_count()
        );
        eprintln!(
            "SEMANTIC_PLAN contract={} sha={}",
            crate::NISAR_LIVE_PLAN_SEMANTIC_AUTHORITY_CONTRACT_V1,
            semantic_plan_sha256
        );
        assert_eq!(
            metadata_snapshot.source_inventory_sha256(),
            INVENTORY_V2_SHA256
        );

        // 5. Geometry reference check: production windows must conservatively
        // contain the independent ISCE3 reference window.
        let windows = &request.mapping.windows;
        assert!(!windows.is_empty());
        let window = &windows[0];
        eprintln!(
            "GEOMETRY window line=[{}, {}) sample=[{}, {})",
            window.line_start,
            window.line_end_exclusive,
            window.sample_start,
            window.sample_end_exclusive
        );
        assert!(
            window.line_start <= REFERENCE_LINE_START
                && window.line_end_exclusive >= REFERENCE_LINE_END
                && window.sample_start <= REFERENCE_SAMPLE_START
                && window.sample_end_exclusive >= REFERENCE_SAMPLE_END,
            "production window must conservatively contain the ISCE3 reference"
        );
        let line_overlap = (window.line_end_exclusive - window.line_start) as f64
            / (REFERENCE_LINE_END - REFERENCE_LINE_START) as f64;
        let sample_overlap = (window.sample_end_exclusive - window.sample_start) as f64
            / (REFERENCE_SAMPLE_END - REFERENCE_SAMPLE_START) as f64;
        eprintln!(
            "GEOMETRY line_overcoverage={sample_overlap:.3} sample_overcoverage={line_overlap:.3} area_ratio={:.3}",
            sample_overlap * line_overlap
        );

        // 6. Simulated science Range transfer into the sparse part.
        let lease = DestinationLeaseController::new_for_qualification_test();
        let transfer_http = HttpTransfer::new_with_proxy_policy(
            TransferOptions {
                max_attempts: 1,
                backoff: Duration::ZERO,
                timeout: Duration::from_secs(60),
                chunk_size: 64 * 1024,
                ..TransferOptions::default()
            },
            ProxyPolicy::Direct,
        )
        .unwrap();
        let range_transfer = OfflineHdf5RangeTransfer::new(
            transfer_http,
            &request,
            &snapshot_item(),
            asset,
            workspace.clone(),
            std::sync::Arc::clone(&lease),
        )
        .expect("range transfer bind");
        let transfer_result = range_transfer
            .execute(&request, &plan, "2026-08-07T00:00:02Z", &cancelled)
            .expect("science range transfer");
        assert_eq!(
            transfer_result.manifest().state,
            NisarSubsetTransferState::RangesComplete
        );
        assert_eq!(
            transfer_result.manifest().received_bytes,
            plan.expected_remote_bytes()
        );
        let science_bytes = plan
            .ranges()
            .iter()
            .skip(1)
            .map(|range| range.end_exclusive - range.start)
            .sum::<u64>();
        eprintln!(
            "PLAN sha={} remote_bytes={} prefix_bytes={} science_bytes={} requests={}",
            plan.plan_sha256(),
            plan.expected_remote_bytes(),
            plan.metadata_prefix_bytes(),
            science_bytes,
            transfer_result.metrics().request_count
        );
        eprintln!(
            "DISCOVERY requests={} bytes={} ranges={}",
            discovery.request_count,
            discovery.response_body_bytes,
            discovery.discovered_ranges.len()
        );

        // 7. Writer + scientific validation.

        let output_root = root.0.join("output");
        let writer = OfflineNisarSubsetWriter::new(output_root.clone(), lease).unwrap();
        let result = writer
            .write_and_validate(
                &request,
                &plan,
                &transfer_result,
                &workspace,
                &metadata_snapshot,
                &cancelled,
            )
            .expect("writer scientific validation");
        assert!(result.manifest().complete);
        let output_size = fs::metadata(result.artifact_path()).unwrap().len();
        let output_sha256 = hash_file(result.artifact_path(), "sha256");
        eprintln!(
            "OUTPUT size={output_size} sha256={output_sha256} donor_sha256={DONOR_OUTPUT_SHA256} equality={}",
            output_sha256 == DONOR_OUTPUT_SHA256
        );

        // 8. Exact science parity against the local full source window.
        let source_file = File::open(&source_path).unwrap();
        let output_file = File::open(result.artifact_path()).unwrap();
        let mut complex_mismatch = 0_u64;
        let mut amplitude_mismatch = 0_u64;
        let mut phase_mismatch = 0_u64;
        let mut mask_mismatch = 0_u64;
        let science_path = "/science/LSAR/RSLC/swaths/frequencyA/HH";
        let output_science = OUTPUT_SCIENCE_PATH;
        let source_science: Array2<CFloat32> = source_file
            .dataset(science_path)
            .unwrap()
            .read_slice_2d((
                window.line_start as usize..window.line_end_exclusive as usize,
                window.sample_start as usize..window.sample_end_exclusive as usize,
            ))
            .unwrap();
        let produced_science: Array2<CFloat32> = output_file
            .dataset(output_science)
            .unwrap()
            .read_2d()
            .unwrap();
        assert_eq!(source_science.shape(), produced_science.shape());
        for (expected, actual) in source_science.iter().zip(produced_science.iter()) {
            if expected.r.to_bits() != actual.r.to_bits()
                || expected.i.to_bits() != actual.i.to_bits()
            {
                complex_mismatch += 1;
            }
            let expected_amplitude = (expected.r * expected.r + expected.i * expected.i).sqrt();
            let actual_amplitude = (actual.r * actual.r + actual.i * actual.i).sqrt();
            if expected_amplitude.to_bits() != actual_amplitude.to_bits() {
                amplitude_mismatch += 1;
            }
            let expected_phase = expected.i.atan2(expected.r);
            let actual_phase = actual.i.atan2(actual.r);
            if expected_phase.to_bits() != actual_phase.to_bits() {
                phase_mismatch += 1;
            }
        }
        let source_mask: Array2<u8> = source_file
            .dataset("/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask")
            .unwrap()
            .read_slice_2d((
                window.line_start as usize..window.line_end_exclusive as usize,
                window.sample_start as usize..window.sample_end_exclusive as usize,
            ))
            .unwrap();
        let produced_mask: Array2<u8> = output_file
            .dataset(OUTPUT_MASK_PATH)
            .unwrap()
            .read_2d()
            .unwrap();
        assert_eq!(source_mask.shape(), produced_mask.shape());
        for (expected, actual) in source_mask.iter().zip(produced_mask.iter()) {
            if expected != actual {
                mask_mismatch += 1;
            }
        }
        eprintln!(
            "SCIENCE complex_mismatch={complex_mismatch} amplitude_mismatch={amplitude_mismatch} phase_mismatch={phase_mismatch} mask_mismatch={mask_mismatch} output_shape={:?}",
            produced_science.shape()
        );
        assert_eq!(complex_mismatch, 0);
        assert_eq!(amplitude_mismatch, 0);
        assert_eq!(phase_mismatch, 0);
        assert_eq!(mask_mismatch, 0);

        // 9. Memory / working-set sanity numbers.
        let largest_compressed = plan
            .chunks()
            .iter()
            .filter_map(|chunk| match chunk.allocation {
                Hdf5ChunkAllocationV1::Allocated { stored_size, .. } => Some(stored_size),
                Hdf5ChunkAllocationV1::Fill { .. } => None,
            })
            .max()
            .unwrap_or(0);
        let largest_logical = plan
            .chunks()
            .iter()
            .map(|chunk| {
                chunk.chunk_shape[0]
                    .saturating_mul(chunk.chunk_shape[1])
                    .saturating_mul(chunk.datatype.element_size() as u64)
            })
            .max()
            .unwrap_or(0);
        let retained_evidence = plan
            .chunks()
            .iter()
            .filter_map(|chunk| match chunk.allocation {
                Hdf5ChunkAllocationV1::Allocated { stored_size, .. } => Some(stored_size),
                Hdf5ChunkAllocationV1::Fill { .. } => None,
            })
            .sum::<u64>();
        eprintln!(
            "MEMORY largest_compressed={largest_compressed} largest_logical={largest_logical} retained_evidence={retained_evidence} chunk_count={} server_requests={}",
            plan.chunks().len(),
            server.request_count()
        );
        eprintln!("ELAPSED_SECONDS={:.1}", started.elapsed().as_secs_f64());
    }
}

/// Issue #8 slice 6: the writer streams every window, so the former 384 MiB
/// whole-window working set no longer bounds the AOI; the shared 2 GiB output
/// bound does, checked before any bulk read with the same stable code.
#[test]
fn v2_writer_output_bound_admits_june29_and_rejects_huge_windows_with_stable_code() {
    use crate::nisar_subset_stream_v2::{
        v2_check_output_bytes, v2_derived_output_logical_bytes_of, V2DerivedWindowExtent,
    };
    let window = |rows: u64, cols: u64| V2DerivedWindowExtent {
        component_id: 0,
        ordinal: 0,
        line_start: 0,
        line_end_exclusive: rows,
        sample_start: 0,
        sample_end_exclusive: cols,
    };
    // The retained June29 window (14848 x 8192, one polarization) needed about
    // 1044 MiB of science and mask and failed the old 384 MiB capture limit.
    let june29 = v2_derived_output_logical_bytes_of([window(14_848, 8_192)]).unwrap();
    assert_eq!(june29, 14_848 * 8_192 * 9 + (14_848 + 8_192) * 8);
    assert_eq!(v2_check_output_bytes(Some(june29)), Ok(june29));
    let refusal = v2_check_output_bytes(v2_derived_output_logical_bytes_of([window(
        100_000, 100_000,
    )]))
    .unwrap_err();
    let error = super::map_v2_output_refusal(refusal);
    assert_eq!(error, NisarSubsetWriteError::LogicalCaptureLimitExceeded);
    assert_eq!(error.code(), "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT");
}

#[test]
fn v2_axis_windows_never_read_the_full_source_axis_and_cancel_between_bounded_slices() {
    let root = TempDirectory::new("v2-axis-window-bounded");
    let source = root.0.join("source.h5");
    let file = File::create_excl(&source).unwrap();
    let source_axis_values =
        super::MAX_WRITER_WORKING_SET_BYTES / std::mem::size_of::<f64>() + 1_024;
    let axis = file
        .new_dataset::<f64>()
        .shape((source_axis_values,))
        .chunk((1_024,))
        .fill_value(7.5_f64)
        .create("axis")
        .unwrap();

    super::reset_v2_axis_read_observation_for_test();
    let selected_science_extent = 3_u64;
    assert!(source_axis_values as u64 > selected_science_extent);
    let window = super::read_v2_axis_window(
        &axis,
        selected_science_extent - 2,
        selected_science_extent,
        &|| false,
    )
    .unwrap();
    assert_eq!(window, vec![7.5, 7.5]);
    assert_eq!(super::v2_axis_max_read_values_for_test(), 2);

    super::reset_v2_axis_read_observation_for_test();
    let probes = AtomicUsize::new(0);
    let cancelled = || probes.fetch_add(1, Ordering::SeqCst) >= 2;
    let error = super::read_v2_axis_window(
        &axis,
        0,
        (super::V2_AXIS_READ_CHUNK_VALUES * 2 + 1) as u64,
        &cancelled,
    )
    .unwrap_err();
    assert_eq!(error, NisarSubsetWriteError::Cancelled);
    assert!(probes.load(Ordering::SeqCst) >= 3);
    assert_eq!(
        super::v2_axis_max_read_values_for_test(),
        super::V2_AXIS_READ_CHUNK_VALUES
    );
}
