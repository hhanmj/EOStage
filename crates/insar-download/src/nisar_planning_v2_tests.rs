use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::{
    stable_asset_key, AssetAccessDescriptor, AssetAccessScheme, AssetRole, DownloadLayout,
    DownloadOutcome, FrozenAssetAccess, FrozenSearchBatch, FrozenTaskSnapshot,
    NisarSubsetChecksumAlgorithm, NisarSubsetChecksumV1, NisarSubsetFrequency,
    NisarSubsetFrequencyCapabilityV2, NisarSubsetPreparedIntentBuildInputV2,
    NisarSubsetPreparedIntentV2, NisarSubsetQualityStatusV2, NisarSubsetSourceAssetIdentityV2,
    NisarSubsetSourceCapabilityV2, NisarSubsetSourceProductIdentityV2, SearchWorkbenchState,
    SnapshotItem, StableAssetReference, StoreCreateTaskRequest, StoreSearchBatchRequest,
    StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus, TaskStateCommandKind,
};
use insar_store::{
    embed_nisar_subset_plan_v2_envelope, embed_nisar_subset_prepared_intent_v2_envelope,
    parse_nisar_subset_plan_v2_envelope, parse_nisar_subset_prepared_intent_v2_envelope,
    StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request, TaskStore,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::*;
use crate::nisar_subset_writer::tests::{
    create_ssar_source_fixture_with_science_layout, ready_case_with_selection_for_v2, ReadyCase,
};
use crate::{
    AssetResolver, AssetResolverRegistry, Clock, CredentialMaterial, CredentialProvider,
    CredentialProviderError, DownloadScheduler, DownloadWorker, HttpTransfer,
    ProductionDownloadWorker, ProxyPolicy, RemoteValidator, ResolveContext, ResolveError,
    ResolvedAsset, ResolvedChecksum, ResolvedMethod, ResolverExecutionRuntime, ShadowAccessIntent,
    ShadowResolveContext, SharedTaskStore, TransferOptions,
};

#[path = "nisar_sband_resume_tests.rs"]
mod sband_resume;

#[path = "nisar_planning_commit_tests.rs"]
mod planning_commit;

#[path = "nisar_science_retry_tests.rs"]
mod science_retry;

#[cfg(windows)]
#[path = "nisar_workspace_rebinding_tests.rs"]
mod workspace_rebinding;

#[path = "nisar_writer_baseline_tests.rs"]
mod writer_baseline;

#[path = "nisar_writer_streaming_tests.rs"]
mod writer_streaming;

#[path = "nisar_output_admission_tests.rs"]
mod output_admission;

#[path = "nisar_writer_retry_tests.rs"]
mod writer_retry;

#[path = "nisar_worker_progress_tests.rs"]
mod worker_progress;

#[path = "nisar_writer_review_gap_tests.rs"]
mod writer_review_gaps;

#[path = "nisar_writer_resume_tests.rs"]
mod writer_resume;

#[path = "nisar_writer_admission_tests.rs"]
mod writer_admission;

#[path = "nisar_writer_digest_tests.rs"]
mod writer_digest;

#[allow(non_local_definitions)]
#[repr(C)]
#[derive(Clone, Copy, hdf5::H5Type)]
struct TestCFloat32 {
    r: f32,
    i: f32,
}

const SBAND_FIXTURE_GRANULE: &str =
    "NISAR_S1_PR_RSLC_029_063_D_079_2500_CRNA_A_20260829T135915_20260829T135945_P00500_M_P_I_001";

struct EofProbeReadErrorFixture {
    body: std::io::Cursor<Vec<u8>>,
}

impl EofProbeReadErrorFixture {
    fn new(body: &[u8]) -> Self {
        Self {
            body: std::io::Cursor::new(body.to_vec()),
        }
    }
}

impl Read for EofProbeReadErrorFixture {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.body.position() < self.body.get_ref().len() as u64 {
            self.body.read(buffer)
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset))
        }
    }
}

#[test]
fn nisar_v2_eof_probe_read_error_retains_closed_runtime_diagnostic() {
    let mut reader = EofProbeReadErrorFixture::new(b"data");
    let mut body = [0_u8; 4];
    reader.read_exact(&mut body).unwrap();
    assert_eq!(&body, b"data");
    let probe = post_body_eof_probe(&mut reader);
    let PostBodyEofProbeV2::ReadFailed(io_error_class) = probe else {
        panic!("scripted EOF probe did not retain its closed read failure");
    };
    let mut failure = range_parse_failure(
        NisarSubsetPlanningErrorV2::BootstrapTransport,
        10,
        14,
        100,
        "sent_response_rejected",
        1,
        Some(206),
        Some(4),
        Some("a".repeat(64)),
    );
    retain_post_body_eof_probe_read_diagnostic(&mut failure, io_error_class);

    assert_eq!(
        failure.error,
        NisarSubsetPlanningErrorV2::BootstrapTransport
    );
    assert_eq!(failure.observation.http_status, Some(206));
    assert_eq!(failure.observation.response_body_bytes, Some(4));
    assert_eq!(
        failure.observation.transport_diagnostic,
        Some(NisarSubsetTransportDiagnosticV2::PostBodyEofProbeRead {
            io_error_class: NisarSubsetIoErrorClassV2::Connection,
        })
    );
    let diagnostic = failure.observation.transport_diagnostic.unwrap();
    assert_eq!(diagnostic.cause(), "post_body_eof_probe_read");
    assert_eq!(diagnostic.io_error_class().unwrap().as_str(), "connection");
}

fn test_file_sha256(path: &std::path::Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn writer_v2_candidate_paths(root: &std::path::Path) -> Vec<PathBuf> {
    let mut candidates = fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| {
                    name.starts_with(".nisar-subset-writer-v2-") && name.ends_with(".writing")
                })
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
}

fn prepared_intent(
    case: &ReadyCase,
    frequency: NisarSubsetFrequency,
    polarizations: &[&str],
) -> NisarSubsetPreparedIntentV2 {
    prepared_intent_with_requested_geometry(
        case,
        frequency,
        polarizations,
        json!({
            "type": "Polygon",
            "coordinates": [[[36.1, 9.1], [36.9, 9.1], [36.9, 9.9], [36.1, 9.9], [36.1, 9.1]]]
        }),
    )
}

fn prepared_intent_with_requested_geometry(
    case: &ReadyCase,
    frequency: NisarSubsetFrequency,
    polarizations: &[&str],
    requested_geometry: Value,
) -> NisarSubsetPreparedIntentV2 {
    prepared_intent_for_source_request(&case.request, frequency, polarizations, requested_geometry)
}

fn prepared_intent_for_source_request(
    source_request: &insar_core::NisarSubsetRequestManifestV1,
    frequency: NisarSubsetFrequency,
    polarizations: &[&str],
    requested_geometry: Value,
) -> NisarSubsetPreparedIntentV2 {
    NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: NisarSubsetSourceProductIdentityV2 {
            provider_id: "asf.nasa.earthdata".to_owned(),
            collection_id: "C2853145197-ASF".to_owned(),
            collection_profile_id: "asf.nisar.l.rslc.provisional.v1".to_owned(),
            provider_product_id: "G9900000001-ASF".to_owned(),
            provider_product_key: "NISAR_FIXTURE_SOURCE".to_owned(),
            product_specification_version: source_request
                .source
                .product_specification_version
                .clone(),
            crid: "P05023".to_owned(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        },
        source_asset_identity: NisarSubsetSourceAssetIdentityV2 {
            asset_key: "science-rslc-hdf5".to_owned(),
            stable_asset_key: "NISAR_FIXTURE_SOURCE::ASSET::science-rslc-hdf5".to_owned(),
            file_name: source_request.source.file_name.clone(),
            checksum: source_request.source.checksum.clone(),
        },
        source_capability: NisarSubsetSourceCapabilityV2 {
            evidence_sha256: "b".repeat(64),
            frequencies: vec![
                NisarSubsetFrequencyCapabilityV2 {
                    frequency: NisarSubsetFrequency::A,
                    available_polarizations: vec!["HH".to_owned(), "HV".to_owned()],
                },
                NisarSubsetFrequencyCapabilityV2 {
                    frequency: NisarSubsetFrequency::B,
                    available_polarizations: vec![
                        "HH".to_owned(),
                        "HV".to_owned(),
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
        requested_geometry: requested_geometry.as_object().unwrap().clone(),
        selected_frequency: frequency,
        selected_polarizations: polarizations
            .iter()
            .map(|polarization| (*polarization).to_owned())
            .collect(),
    })
    .unwrap()
}

fn prepared_sband_intent(source_path: &Path) -> NisarSubsetPreparedIntentV2 {
    NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: NisarSubsetSourceProductIdentityV2 {
            provider_id: "isro.nrsc.bhoonidhi".to_owned(),
            collection_id: "NISAR_SSAR_RSLC".to_owned(),
            collection_profile_id: "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.v1".to_owned(),
            provider_product_id: SBAND_FIXTURE_GRANULE.to_owned(),
            provider_product_key: format!(
                "PROVIDER::isro.nrsc.bhoonidhi::COLLECTION::NISAR_SSAR_RSLC::ITEM::{SBAND_FIXTURE_GRANULE}"
            ),
            product_specification_version: "1.2.1".to_owned(),
            crid: "P00500".to_owned(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        },
        source_asset_identity: NisarSubsetSourceAssetIdentityV2 {
            asset_key: "science-rslc-hdf5".to_owned(),
            stable_asset_key: format!(
                "PROVIDER::isro.nrsc.bhoonidhi::COLLECTION::NISAR_SSAR_RSLC::ITEM::{SBAND_FIXTURE_GRANULE}::ASSET::science-rslc-hdf5"
            ),
            file_name: format!("{SBAND_FIXTURE_GRANULE}.h5"),
            checksum: Some(NisarSubsetChecksumV1 {
                algorithm: NisarSubsetChecksumAlgorithm::Sha256,
                value: test_file_sha256(source_path),
            }),
        },
        source_capability: NisarSubsetSourceCapabilityV2 {
            evidence_sha256: "e".repeat(64),
            frequencies: vec![NisarSubsetFrequencyCapabilityV2 {
                frequency: NisarSubsetFrequency::A,
                available_polarizations: vec!["RH".to_owned(), "RV".to_owned()],
            }],
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
            "coordinates": [[[36.1, 9.1], [36.9, 9.1], [36.9, 9.9], [36.1, 9.9], [36.1, 9.1]]]
        })
        .as_object()
        .unwrap()
        .clone(),
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations: vec!["RH".to_owned()],
    })
    .unwrap()
}

fn work(case: &ReadyCase, intent: &NisarSubsetPreparedIntentV2) -> DownloadWork {
    work_for_source_request(&case.request, &case._root.0, intent)
}

fn work_for_source_request(
    source_request: &insar_core::NisarSubsetRequestManifestV1,
    root: &Path,
    intent: &NisarSubsetPreparedIntentV2,
) -> DownloadWork {
    let mut metadata = Map::new();
    metadata.insert(
        "delivery_kind".to_owned(),
        Value::String(NISAR_SUBSET_V2_DELIVERY_KIND.to_owned()),
    );
    metadata.insert(
        "source_item_id".to_owned(),
        Value::String("NISAR_FIXTURE_SOURCE".to_owned()),
    );
    metadata.insert("subset_frequency".to_owned(), json!("B"));
    metadata.insert("subset_polarization".to_owned(), json!("VV"));
    metadata.insert(
        "subset_aoi_geojson".to_owned(),
        json!({"type":"Polygon","coordinates":[]}),
    );
    embed_nisar_subset_prepared_intent_v2_envelope(&mut metadata, intent).unwrap();
    let item = SnapshotItem {
        item_id: "NISAR_FIXTURE_SOURCE".to_owned(),
        provider_id: intent.source_product_identity.provider_id.clone(),
        display_name: source_request.source.file_name.clone(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    DownloadWork {
        task: FrozenTaskSnapshot {
            schema_version: 1,
            task_id: "nisar-v2-task".to_owned(),
            batch_id: "nisar-v2-batch".to_owned(),
            provider_id: intent.source_product_identity.provider_id.clone(),
            created_at: "2026-08-30T00:00:00Z".to_owned(),
            aoi_name: "fixture".to_owned(),
            aoi_geojson: None,
            query: Map::new(),
            candidates: vec![item.clone()],
            initial_selected_item_ids: vec![item.item_id.clone()],
            output_root: root.join("output"),
            layout: DownloadLayout::Direct,
            operation: None,
            orbit_association: None,
        },
        item,
    }
}

fn concrete_fixture(
    label: &str,
    frequency: NisarSubsetFrequency,
    polarizations: &[&str],
) -> (
    ReadyCase,
    NisarSubsetPreparedIntentV2,
    Arc<NisarSubsetLocalHdf5MetadataBootstrapV2>,
    PathBuf,
) {
    concrete_fixture_with_source_mutation(label, frequency, polarizations, |_| {})
}

fn concrete_sband_fixture(
    label: &str,
) -> (
    ReadyCase,
    NisarSubsetPreparedIntentV2,
    Arc<NisarSubsetLocalHdf5MetadataBootstrapV2>,
    PathBuf,
    crate::nisar_subset_writer::TestSourcePin,
) {
    let (case, _lband_intent, _lband_bootstrap, _lband_source_path) =
        concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
    let source_path = case._root.0.join("ssar-source.h5");
    let source_pin = create_ssar_source_fixture_with_science_layout(&source_path, false, true);
    let source = hdf5::File::open_rw(&source_path).unwrap();
    let science_ballast = vec![0_u8; 4 * 1024 * 1024];
    source
        .group("/science/SSAR/RSLC/swaths/frequencyA")
        .unwrap()
        .new_dataset::<u8>()
        .shape((science_ballast.len(),))
        .chunk((64 * 1024,))
        .create("LH")
        .and_then(|dataset| dataset.write_raw(&science_ballast))
        .unwrap();
    source.flush().unwrap();
    source.close().unwrap();

    let intent = prepared_sband_intent(&source_path);
    let hdf5_extent = fs::metadata(&source_path).unwrap().len();
    let workspace =
        Hdf5RangeWorkspaceV1::new(case._root.0.join(format!("{label}-ssar-bootstrap"))).unwrap();
    let bootstrap = Arc::new(
        NisarSubsetLocalHdf5MetadataBootstrapV2::new_for_test(
            source_path.clone(),
            hdf5_extent,
            workspace,
        )
        .unwrap(),
    );
    (case, intent, bootstrap, source_path, source_pin)
}

fn concrete_fixture_with_source_mutation(
    label: &str,
    frequency: NisarSubsetFrequency,
    polarizations: &[&str],
    mutate: impl FnOnce(&hdf5::File),
) -> (
    ReadyCase,
    NisarSubsetPreparedIntentV2,
    Arc<NisarSubsetLocalHdf5MetadataBootstrapV2>,
    PathBuf,
) {
    let (fixture_pol, science, mask) = match frequency {
        NisarSubsetFrequency::A => (
            "HH",
            "/science/LSAR/RSLC/swaths/frequencyA/HH",
            "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask",
        ),
        NisarSubsetFrequency::B => (
            "HV",
            "/science/LSAR/RSLC/swaths/frequencyB/HV",
            "/science/LSAR/RSLC/swaths/frequencyB/inputDataExceptionMask",
        ),
    };
    let fixture_frequency = frequency.clone();
    let case = ready_case_with_selection_for_v2(
        label,
        1,
        frequency.clone(),
        fixture_pol,
        science,
        mask,
        false,
        move |file| {
            if fixture_frequency == NisarSubsetFrequency::B {
                file.dataset("/science/LSAR/RSLC/swaths/frequencyB/slantRange")
                    .unwrap()
                    .write_raw(&[800_000.0_f64, 800_010.0, 800_020.0, 800_030.0])
                    .unwrap();
            }
            // A real, uncompressed unselected science dataset makes the fixture's
            // metadata-to-source ratio representative enough to exercise the reviewed
            // production 10% bootstrap budget without a test-only budget override.
            let science_ballast = vec![0_u8; 4 * 1024 * 1024];
            file.group("/science/LSAR/RSLC/swaths/frequencyA")
                .unwrap()
                .new_dataset::<u8>()
                .shape((science_ballast.len(),))
                .chunk((64 * 1024,))
                .create("RV")
                .and_then(|dataset| dataset.write_raw(&science_ballast))
                .unwrap();
            mutate(file);
        },
    );
    let intent = prepared_intent(&case, frequency, polarizations);
    let hdf5_extent = fs::metadata(&case.source_path).unwrap().len();
    let workspace =
        Hdf5RangeWorkspaceV1::new(case._root.0.join(format!("{label}-bootstrap"))).unwrap();
    let bootstrap = Arc::new(
        NisarSubsetLocalHdf5MetadataBootstrapV2::new_for_test(
            case.source_path.clone(),
            hdf5_extent,
            workspace,
        )
        .unwrap(),
    );
    let source_path = case.source_path.clone();
    (case, intent, bootstrap, source_path)
}

fn read_science_request(stream: &mut TcpStream) -> String {
    use std::io::{BufRead, BufReader};

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

struct ScienceServer {
    url_root: String,
    worker: Option<thread::JoinHandle<Vec<String>>>,
}

impl ScienceServer {
    fn new(responses: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_nodelay(true).unwrap();
                requests.push(read_science_request(&mut stream));
                stream.write_all(&response).unwrap();
                stream.flush().unwrap();
            }
            requests
        });
        Self {
            url_root: format!("http://{address}"),
            worker: Some(worker),
        }
    }

    fn finish(mut self) -> Vec<String> {
        self.worker.take().unwrap().join().unwrap()
    }
}

#[cfg(test)]
impl Drop for ScienceServer {
    fn drop(&mut self) {
        eprintln!(
            "[nisar-diagnostic] ScienceServer dropped; worker_joined={}",
            self.worker.is_none()
        );
    }
}

fn science_response(status: &str, headers: &[(&str, String)], body: &[u8]) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn exact_science_response(
    range: &NisarSubsetPhysicalRangeV2,
    source_size: u64,
    etag: &str,
    body: &[u8],
) -> Vec<u8> {
    science_response(
        "206 Partial Content",
        &[
            ("ETag", etag.to_owned()),
            (
                "Content-Range",
                format!(
                    "bytes {}-{}/{source_size}",
                    range.start(),
                    range.end_exclusive() - 1
                ),
            ),
            ("Content-Length", range.len().unwrap().to_string()),
        ],
        body,
    )
}

fn source_range_bytes(
    source_path: &std::path::Path,
    range: &NisarSubsetPhysicalRangeV2,
) -> Vec<u8> {
    let mut source = fs::File::open(source_path).unwrap();
    source.seek(SeekFrom::Start(range.start())).unwrap();
    let mut body = vec![0_u8; range.len().unwrap() as usize];
    source.read_exact(&mut body).unwrap();
    body
}

fn exact_science_responses(
    source_path: &std::path::Path,
    physical: &NisarSubsetPhysicalExecutionPlanV2,
    skip: usize,
) -> Vec<Vec<u8>> {
    physical
        .execution_transfer_ranges()
        .unwrap()
        .into_iter()
        .skip(skip)
        .map(|range| {
            exact_science_response(
                &range,
                physical.source_size(),
                &physical.source_validator().value,
                &source_range_bytes(source_path, &range),
            )
        })
        .collect()
}

fn science_request_header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (header, value) = line.split_once(':')?;
        header.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

fn science_http() -> HttpTransfer {
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
    .unwrap()
}

fn science_asset(
    url_root: &str,
    physical: &NisarSubsetPhysicalExecutionPlanV2,
    etag: Option<&str>,
    size: Option<u64>,
    file_name: Option<&str>,
) -> ResolvedAsset {
    let size = size.unwrap_or_else(|| physical.source_size());
    ResolvedAsset::new(
        &format!(
            "{url_root}/{}",
            file_name.unwrap_or(&physical.source_asset_identity().file_name)
        ),
        ResolvedMethod::Get,
    )
    .unwrap()
    .with_expected_size(Some(size))
    .with_checksum(
        physical
            .source_asset_identity()
            .checksum
            .as_ref()
            .map(|checksum| {
                let algorithm = match checksum.algorithm {
                    insar_core::NisarSubsetChecksumAlgorithm::Md5 => "md5",
                    insar_core::NisarSubsetChecksumAlgorithm::Sha1 => "sha1",
                    insar_core::NisarSubsetChecksumAlgorithm::Sha256 => "sha256",
                };
                ResolvedChecksum::new(algorithm, &checksum.value).unwrap()
            }),
    )
    .with_remote_validator(RemoteValidator {
        etag: etag.map(str::to_owned),
        last_modified: None,
        content_length: Some(size),
    })
}

fn science_asset_with_bearer(
    url_root: &str,
    physical: &NisarSubsetPhysicalExecutionPlanV2,
    etag: &str,
    token: &str,
) -> ResolvedAsset {
    let mut asset = science_asset(url_root, physical, Some(etag), None, None);
    asset.set_bearer_authorization_for_test(token).unwrap();
    asset
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScienceAnchorPendingFailpoint {
    Genesis,
    Reservation,
    ResponseRecorded,
    FailedOutcome,
    Interrupted,
    Complete,
}

#[derive(Default)]
struct ScienceCheckpointAnchorFixture {
    state: Mutex<
        Option<(
            String,
            crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2,
        )>,
    >,
    failpoint: Mutex<Option<ScienceAnchorPendingFailpoint>>,
    pause_after_reservation: Mutex<Option<WorkerControl>>,
}

impl ScienceCheckpointAnchorFixture {
    fn fail_after_pending_cas(&self, failpoint: ScienceAnchorPendingFailpoint) {
        *self.failpoint.lock().unwrap() = Some(failpoint);
    }

    fn pending_matches(
        replacement: &crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2,
        failpoint: ScienceAnchorPendingFailpoint,
    ) -> bool {
        let Some(canonical) = replacement.pending_checkpoint_canonical_json() else {
            return false;
        };
        let Ok(checkpoint) = serde_json::from_str::<Value>(canonical) else {
            return false;
        };
        let state = checkpoint["state"].as_str();
        let pending_state = checkpoint["pending_attempt"]["state"].as_str();
        let failed = checkpoint["failed_science_request_count"]
            .as_u64()
            .unwrap_or(0);
        match failpoint {
            ScienceAnchorPendingFailpoint::Genesis => {
                checkpoint["anchor_generation"] == json!(0)
                    && state == Some("Planned")
                    && checkpoint["pending_attempt"].is_null()
            }
            ScienceAnchorPendingFailpoint::Reservation => pending_state == Some("Reserved"),
            ScienceAnchorPendingFailpoint::ResponseRecorded => {
                pending_state == Some("ResponseRecorded")
            }
            ScienceAnchorPendingFailpoint::FailedOutcome => {
                state == Some("Interrupted")
                    && checkpoint["pending_attempt"].is_null()
                    && failed > 0
            }
            ScienceAnchorPendingFailpoint::Interrupted => {
                state == Some("Interrupted")
                    && checkpoint["pending_attempt"].is_null()
                    && failed == 0
            }
            ScienceAnchorPendingFailpoint::Complete => state == Some("Complete"),
        }
    }

    fn corrupt_pending_payload_without_reseal(&self) {
        let mut state = self.state.lock().unwrap();
        let (_, record) = state.as_mut().expect("anchored record");
        record.corrupt_pending_payload_for_test();
    }

    fn coherently_mismatch_pending_predecessor(&self) {
        let mut state = self.state.lock().unwrap();
        let (_, record) = state.as_mut().expect("anchored record");
        record.coherently_mismatch_pending_predecessor_for_test();
    }

    fn coherently_set_pending_generation_max(&self) {
        let mut state = self.state.lock().unwrap();
        let (_, record) = state.as_mut().expect("anchored record");
        record.coherently_set_pending_generation_max_for_test();
    }

    fn has_pending(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|(_, record)| record.has_pending_checkpoint())
    }
}

impl crate::nisar_science_transfer_v2::science_checkpoint_anchor_sealed::Sealed
    for ScienceCheckpointAnchorFixture
{
}

impl crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorV2
    for ScienceCheckpointAnchorFixture
{
    fn load(
        &self,
        lineage_key_sha256: &str,
    ) -> Result<
        Option<crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2>,
        crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2,
    > {
        let state = self.state.lock().unwrap();
        match state.as_ref() {
            None => Ok(None),
            Some((stored_key, record)) if stored_key == lineage_key_sha256 => {
                Ok(Some(record.clone()))
            }
            Some(_) => Err(
                crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::AuthorityMismatch,
            ),
        }
    }

    fn compare_and_swap(
        &self,
        lineage_key_sha256: &str,
        expected: Option<
            &crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2,
        >,
        replacement: &crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2,
    ) -> Result<bool, crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2> {
        if replacement.lineage_key_sha256() != lineage_key_sha256
            || expected.is_some_and(|record| record.lineage_key_sha256() != lineage_key_sha256)
        {
            return Err(
                crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::AuthorityMismatch,
            );
        }
        let mut state = self.state.lock().unwrap();
        match state.as_ref() {
            Some((stored_key, _)) if stored_key != lineage_key_sha256 => {
                return Err(
                    crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::AuthorityMismatch,
                );
            }
            _ => {}
        }
        if state.as_ref().map(|(_, record)| record) != expected {
            return Ok(false);
        }
        *state = Some((lineage_key_sha256.to_owned(), replacement.clone()));
        if !replacement.has_pending_checkpoint()
            && expected.is_some_and(|record| {
                Self::pending_matches(record, ScienceAnchorPendingFailpoint::Reservation)
            })
        {
            if let Some(control) = self.pause_after_reservation.lock().unwrap().take() {
                control.set_paused_for_test(true);
            }
        }
        let mut failpoint = self.failpoint.lock().unwrap();
        if failpoint
            .as_ref()
            .is_some_and(|candidate| Self::pending_matches(replacement, *candidate))
        {
            *failpoint = None;
            return Err(
                crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::Persistence,
            );
        }
        Ok(true)
    }
}

fn science_authority<'a>(
    bootstrap: &'a NisarSubsetLocalHdf5MetadataBootstrapV2,
    asset: &ResolvedAsset,
    work: &DownloadWork,
    completed: &NisarSubsetPlanningCompleteV2,
    anchor: &'a ScienceCheckpointAnchorFixture,
) -> crate::nisar_science_transfer_v2::NisarSubsetScienceAuthorityBindingV2<'a> {
    let physical = completed.physical_execution_plan();
    crate::nisar_science_transfer_v2::NisarSubsetScienceAuthorityBindingV2::new(
        bootstrap,
        bootstrap.science_workspace_v2(),
        crate::nisar_science_transfer_v2::NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
            asset.effective_url(),
            work,
            completed,
            physical,
        )
        .unwrap(),
        anchor,
    )
}

struct CountingPlanningAuthority<'a> {
    inner: &'a NisarSubsetLocalHdf5MetadataBootstrapV2,
    reopen_count: Arc<AtomicUsize>,
}

impl super::sealed::Sealed for CountingPlanningAuthority<'_> {}

impl NisarSubsetMetadataBootstrapV2 for CountingPlanningAuthority<'_> {
    fn bootstrap(
        &self,
        work: &DownloadWork,
        prepared_intent: &NisarSubsetPreparedIntentV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetMetadataBootstrapOutputV2, NisarSubsetPlanningErrorV2> {
        self.inner.bootstrap(work, prepared_intent, control)
    }

    fn revalidate_before_freeze(
        &self,
        work: &DownloadWork,
        startup_prepared_intent_sha256: &str,
        startup_source_validator_sha256: &str,
        control: &WorkerControl,
    ) -> Result<NisarSubsetBootstrapRevalidationV2, NisarSubsetPlanningErrorV2> {
        self.inner.revalidate_before_freeze(
            work,
            startup_prepared_intent_sha256,
            startup_source_validator_sha256,
            control,
        )
    }

    fn build_and_checkpoint_physical_plan(
        &self,
        work: &DownloadWork,
        frozen_plan: &NisarSubsetPlanV2,
        inventory: &NisarSubsetVerifiedMetadataInventoryV2,
        statistics: &NisarSubsetBootstrapStatisticsV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetPhysicalExecutionPlanV2, NisarSubsetPlanningErrorV2> {
        self.inner.build_and_checkpoint_physical_plan(
            work,
            frozen_plan,
            inventory,
            statistics,
            control,
        )
    }

    fn reopen_planning_workspace(
        &self,
        work: &DownloadWork,
        frozen_plan: &NisarSubsetPlanV2,
        control: &WorkerControl,
    ) -> Result<
        (
            NisarSubsetPhysicalExecutionPlanV2,
            NisarSubsetBootstrapStatisticsV2,
            NisarSubsetVerifiedMetadataInventoryV2,
        ),
        NisarSubsetPlanningErrorV2,
    > {
        let reopen_count = self.reopen_count.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(
            reopen_count <= 2,
            "production performed a third full planning workspace rebuild before completion"
        );
        self.inner
            .reopen_planning_workspace(work, frozen_plan, control)
    }
}

struct StaticScienceAuthorityFixture<'a> {
    plan: NisarSubsetPhysicalExecutionPlanV2,
    statistics: NisarSubsetBootstrapStatisticsV2,
    workspace: &'a Hdf5RangeWorkspaceV1,
    remote_source: crate::nisar_science_transfer_v2::NisarSubsetScienceRemoteSourceV2,
    anchor: &'a ScienceCheckpointAnchorFixture,
}

impl crate::nisar_science_transfer_v2::science_authority_sealed::Sealed
    for StaticScienceAuthorityFixture<'_>
{
}

impl crate::nisar_science_transfer_v2::NisarSubsetScienceAuthorityV2
    for StaticScienceAuthorityFixture<'_>
{
    fn revalidate(
        &self,
        _work: &DownloadWork,
        _completed: &NisarSubsetPlanningCompleteV2,
        _control: &WorkerControl,
    ) -> Result<
        (
            NisarSubsetPhysicalExecutionPlanV2,
            NisarSubsetBootstrapStatisticsV2,
        ),
        NisarSubsetPlanningErrorV2,
    > {
        Ok((self.plan.clone(), self.statistics.clone()))
    }

    fn workspace(&self) -> &Hdf5RangeWorkspaceV1 {
        self.workspace
    }

    fn remote_source(&self) -> &crate::nisar_science_transfer_v2::NisarSubsetScienceRemoteSourceV2 {
        &self.remote_source
    }

    fn checkpoint_anchor(
        &self,
    ) -> &dyn crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorV2 {
        self.anchor
    }
}

struct PauseDuringScienceRevalidateAuthorityFixture<'a> {
    plan: NisarSubsetPhysicalExecutionPlanV2,
    statistics: NisarSubsetBootstrapStatisticsV2,
    workspace: &'a Hdf5RangeWorkspaceV1,
    remote_source: crate::nisar_science_transfer_v2::NisarSubsetScienceRemoteSourceV2,
    anchor: &'a ScienceCheckpointAnchorFixture,
    calls: AtomicUsize,
    propagate_pause: bool,
}

impl crate::nisar_science_transfer_v2::science_authority_sealed::Sealed
    for PauseDuringScienceRevalidateAuthorityFixture<'_>
{
}

impl crate::nisar_science_transfer_v2::NisarSubsetScienceAuthorityV2
    for PauseDuringScienceRevalidateAuthorityFixture<'_>
{
    fn revalidate(
        &self,
        _work: &DownloadWork,
        _completed: &NisarSubsetPlanningCompleteV2,
        control: &WorkerControl,
    ) -> Result<
        (
            NisarSubsetPhysicalExecutionPlanV2,
            NisarSubsetBootstrapStatisticsV2,
        ),
        NisarSubsetPlanningErrorV2,
    > {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call >= 2 {
            if self.propagate_pause {
                control.set_paused_for_test(true);
            }
            return Err(NisarSubsetPlanningErrorV2::ExecutionStopped);
        }
        Ok((self.plan.clone(), self.statistics.clone()))
    }

    fn workspace(&self) -> &Hdf5RangeWorkspaceV1 {
        self.workspace
    }

    fn remote_source(&self) -> &crate::nisar_science_transfer_v2::NisarSubsetScienceRemoteSourceV2 {
        &self.remote_source
    }

    fn checkpoint_anchor(
        &self,
    ) -> &dyn crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorV2 {
        self.anchor
    }
}

fn execute_production_science_binding_with_rebuild_counter(label: &str) -> (usize, usize) {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceAuthorityBindingV2, NisarSubsetScienceRangeExecutorV2,
        NisarSubsetScienceRemoteSourceV2,
    };

    let (case, intent, bootstrap, source_path) =
        concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let ranges = physical.execution_transfer_ranges().unwrap();
    assert!(ranges.len() > 1, "the regression needs multiple ranges");
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let stable_url = server.url_root.clone();
    let asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let full_rebuild_count = Arc::new(AtomicUsize::new(0));
    let planning_authority = CountingPlanningAuthority {
        inner: bootstrap.as_ref(),
        reopen_count: Arc::clone(&full_rebuild_count),
    };
    let authority = NisarSubsetScienceAuthorityBindingV2::new(
        &planning_authority,
        bootstrap.science_workspace_v2(),
        NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
            asset.effective_url(),
            &work,
            &completed,
            physical,
        )
        .unwrap(),
        &anchor,
    );
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();

    let result = executor
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    assert!(result.complete());
    assert_eq!(server.finish().len(), ranges.len());
    (ranges.len(), full_rebuild_count.load(Ordering::SeqCst))
}

#[test]
fn science_range_v2_production_binding_full_rebuild_count_is_constant_for_n_ranges() {
    let (range_count, full_rebuild_count) = execute_production_science_binding_with_rebuild_counter(
        "science-v2-linear-revalidation-red",
    );
    assert!(range_count > 1);
    assert_eq!(
        full_rebuild_count, 2,
        "production must do one full rebuild at entry and one at completion, not one per range"
    );
}

#[test]
fn science_range_v2_completed_material_hash_work_is_linear_for_n_ranges() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceRemoteSourceV2,
    };

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-linear-material-hash-red",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let ranges = physical.execution_transfer_ranges().unwrap();
    assert!(ranges.len() > 1, "the regression needs multiple ranges");
    let total_range_bytes = ranges.iter().map(|range| range.len().unwrap()).sum::<u64>();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let stable_url = server.url_root.clone();
    let asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = StaticScienceAuthorityFixture {
        plan: physical.clone(),
        statistics: completed.bootstrap_statistics().clone(),
        workspace: bootstrap.science_workspace_v2(),
        remote_source: NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
            asset.effective_url(),
            &work,
            &completed,
            physical,
        )
        .unwrap(),
        anchor: &anchor,
    };
    let completed_material_hash_bytes = Arc::new(AtomicU64::new(0));
    let hash_bytes_at_commit = Arc::clone(&completed_material_hash_bytes);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset)
        .unwrap()
        .with_completed_material_hash_counter_for_test(Arc::clone(
            &completed_material_hash_bytes,
        ))
        .with_after_commit_hook_for_test(Arc::new(move |index| {
            if index == 1 {
                assert_eq!(
                    hash_bytes_at_commit.load(Ordering::SeqCst),
                    0,
                    "fresh completed material must not be cumulatively rehashed inside the range loop"
                );
            }
        }));

    let result = executor
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    assert!(result.complete());
    assert_eq!(server.finish().len(), ranges.len());
    assert_eq!(
        completed_material_hash_bytes.load(Ordering::SeqCst),
        total_range_bytes,
        "a fresh transfer should hash each completed range once at the final full boundary"
    );
}

#[test]
fn science_range_v2_final_boundary_rejects_checkpoint_bytes_and_path_tampering() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceRemoteSourceV2,
        NisarSubsetScienceTransferErrorV2 as Error,
    };

    for attack in ["checkpoint", "early-range-bytes", "part-path"] {
        let (case, intent, bootstrap, source_path) = concrete_fixture(
            &format!("science-v2-final-boundary-{attack}"),
            NisarSubsetFrequency::A,
            &["HH"],
        );
        let mut work = work(&case, &intent);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan())
            .unwrap();
        let physical = completed.physical_execution_plan();
        let ranges = physical.execution_transfer_ranges().unwrap();
        assert!(ranges.len() > 1, "the regression needs an early range");
        let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
        let stable_url = server.url_root.clone();
        let asset = science_asset(
            &stable_url,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        );
        let anchor = ScienceCheckpointAnchorFixture::default();
        let authority = StaticScienceAuthorityFixture {
            plan: physical.clone(),
            statistics: completed.bootstrap_statistics().clone(),
            workspace: bootstrap.science_workspace_v2(),
            remote_source: NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
                asset.effective_url(),
                &work,
                &completed,
                physical,
            )
            .unwrap(),
            anchor: &anchor,
        };
        let first_range = ranges[0].clone();
        let science_checkpoint_path = bootstrap
            .science_workspace_v2()
            .science_checkpoint_path_v2();
        let science_part_path = bootstrap.science_workspace_v2().part_path();
        let replacement_source_path = source_path.clone();
        let attack_label = attack.to_owned();
        let injected_attack = attack_label.clone();
        let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset)
            .unwrap()
            .with_before_final_revalidation_hook_for_test(Arc::new(move || match injected_attack
                .as_str()
            {
                "checkpoint" => {
                    fs::write(&science_checkpoint_path, b"not-json").unwrap();
                }
                "early-range-bytes" => {
                    let mut part = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(&science_part_path)
                        .unwrap();
                    part.seek(SeekFrom::Start(first_range.start())).unwrap();
                    let mut byte = [0_u8; 1];
                    part.read_exact(&mut byte).unwrap();
                    byte[0] ^= 0xff;
                    part.seek(SeekFrom::Start(first_range.start())).unwrap();
                    part.write_all(&byte).unwrap();
                    part.sync_all().unwrap();
                }
                "part-path" => {
                    fs::remove_file(&science_part_path).unwrap();
                    fs::copy(&replacement_source_path, &science_part_path).unwrap();
                }
                _ => unreachable!(),
            }));

        assert_eq!(
            executor.execute(&work, &completed, &authority, &WorkerControl::default()),
            Err(Error::RecoveryInvalid),
            "attack {attack_label}"
        );
        assert_eq!(server.finish().len(), ranges.len());
    }
}

#[test]
fn science_range_v2_production_binding_final_boundary_rejects_corrupt_planning_checkpoint() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceAuthorityBindingV2, NisarSubsetScienceRangeExecutorV2,
        NisarSubsetScienceRemoteSourceV2, NisarSubsetScienceTransferErrorV2 as Error,
    };

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-final-boundary-planning-checkpoint",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let ranges = physical.execution_transfer_ranges().unwrap();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let stable_url = server.url_root.clone();
    let asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = NisarSubsetScienceAuthorityBindingV2::new(
        bootstrap.as_ref(),
        bootstrap.science_workspace_v2(),
        NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
            asset.effective_url(),
            &work,
            &completed,
            physical,
        )
        .unwrap(),
        &anchor,
    );
    let planning_checkpoint_path = bootstrap.planning_checkpoint_path_for_test();
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset)
        .unwrap()
        .with_before_final_revalidation_hook_for_test(Arc::new(move || {
            fs::write(&planning_checkpoint_path, b"not-json").unwrap();
        }));

    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::RecoveryInvalid)
    );
    assert_eq!(server.finish().len(), ranges.len());
}

#[test]
fn science_range_v2_pause_during_range_revalidation_persists_interrupted_checkpoint() {
    assert_science_pause_revalidation_checkpoint(true);
}

#[test]
fn science_range_v2_store_pause_before_control_propagation_persists_interrupted_checkpoint() {
    assert_science_pause_revalidation_checkpoint(false);
}

fn assert_science_pause_revalidation_checkpoint(propagate_pause: bool) {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceRemoteSourceV2,
        NisarSubsetScienceTransferErrorV2 as Error,
    };

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-pause-during-range-revalidation",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let first_range = physical.execution_transfer_ranges().unwrap()[0].clone();
    let first_range_bytes = first_range.len().unwrap();
    let source_responses = exact_science_responses(&source_path, physical, 0);
    let server = ScienceServer::new(vec![source_responses[0].clone()]);
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = PauseDuringScienceRevalidateAuthorityFixture {
        plan: physical.clone(),
        statistics: completed.bootstrap_statistics().clone(),
        workspace: bootstrap.science_workspace_v2(),
        remote_source: NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
            asset.effective_url(),
            &work,
            &completed,
            physical,
        )
        .unwrap(),
        anchor: &anchor,
        calls: AtomicUsize::new(0),
        propagate_pause,
    };
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::Paused)
    );

    let checkpoint_text = bootstrap
        .science_workspace_v2()
        .load_science_checkpoint_v2()
        .unwrap()
        .expect("pause persists the science checkpoint");
    let checkpoint: Value = serde_json::from_str(&checkpoint_text).unwrap();
    assert_eq!(checkpoint["state"], Value::String("Interrupted".to_owned()));
    assert!(checkpoint["pending_attempt"].is_null());
    assert_eq!(checkpoint["completed_ranges"].as_array().unwrap().len(), 1);
    assert_eq!(
        checkpoint["completed_ranges"][0]["start"],
        Value::from(first_range.start())
    );
    assert_eq!(
        checkpoint["completed_ranges"][0]["end_exclusive"],
        Value::from(first_range.end_exclusive())
    );
    assert_eq!(
        checkpoint["committed_science_response_body_bytes"],
        Value::from(first_range_bytes)
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn science_range_v2_exposes_remote_authority_and_external_anchor_contracts() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceAuthorityBindingV2, NisarSubsetScienceAuthorityV2,
        NisarSubsetScienceCheckpointAnchorV2, NisarSubsetScienceRemoteSourceV2,
    };

    fn require_authority(_value: &dyn NisarSubsetScienceAuthorityV2) {}
    fn require_anchor(_value: &dyn NisarSubsetScienceCheckpointAnchorV2) {}

    let _binding_constructor = NisarSubsetScienceAuthorityBindingV2::new;
    let _remote_constructor = NisarSubsetScienceRemoteSourceV2::from_frozen_authority;
    let _anchor_pending_payload =
        crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2::pending_checkpoint_canonical_json;
    let _stable_lineage_key =
        crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2::lineage_key_sha256;
    let _execution_binding =
        crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2::execution_binding_sha256;
    let _charged_bytes = crate::nisar_science_transfer_v2::NisarSubsetScienceAccountingV2::charged_science_response_body_bytes;
    let _indeterminate_bytes = crate::nisar_science_transfer_v2::NisarSubsetScienceAccountingV2::indeterminate_science_response_body_bytes;
    let _observed_bytes = crate::nisar_science_transfer_v2::NisarSubsetScienceAccountingV2::observed_science_response_body_bytes;
    let _ = require_authority;
    let _ = require_anchor;
}

#[test]
fn science_range_v2_all_zero_hole_punch_invalidates_allocation_evidence() {
    use crate::hdf5_range_transfer::{
        punch_science_hole_v2_for_test, science_allocated_range_evidence_v2,
        validate_science_allocated_range_evidence_v2,
    };

    const FILE_BYTES: u64 = 256 * 1024;
    const HOLE_START: u64 = 64 * 1024;
    const HOLE_END: u64 = 128 * 1024;

    let (case, _, _, _) =
        concrete_fixture("science-v2-all-zero-hole", NisarSubsetFrequency::A, &["HH"]);
    let path = case._root.0.join("all-zero-science.part");
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.write_all(&vec![0_u8; FILE_BYTES as usize]).unwrap();
    file.sync_all().unwrap();

    let before = science_allocated_range_evidence_v2(&mut file, 0, FILE_BYTES).unwrap();
    assert!(before.covers_exactly(0, FILE_BYTES));

    punch_science_hole_v2_for_test(&mut file, HOLE_START, HOLE_END).unwrap();
    file.sync_all().unwrap();
    let after = science_allocated_range_evidence_v2(&mut file, 0, FILE_BYTES).unwrap();
    assert!(!after.covers_exactly(0, FILE_BYTES));
    assert_ne!(after, before);
    assert!(
        validate_science_allocated_range_evidence_v2(&mut file, 0, FILE_BYTES, &before,).is_err()
    );
}

#[test]
fn science_range_v2_coarse_allocation_capability_is_typed_fail_closed() {
    assert_eq!(
        crate::hdf5_range_transfer::coarse_science_allocation_evidence_v2_for_test(),
        Err(
            crate::hdf5_range_transfer::NisarScienceAllocationEvidenceErrorV2::AllocationEvidenceUnsupported,
        )
    );
}

fn partial_science_fixture(
    label: &str,
) -> (
    ReadyCase,
    DownloadWork,
    NisarSubsetPlanningCompleteV2,
    Arc<NisarSubsetLocalHdf5MetadataBootstrapV2>,
    Arc<ScienceCheckpointAnchorFixture>,
    String,
    PathBuf,
) {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as Error;

    let (case, intent, bootstrap, source_path) =
        concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(vec![
        exact_science_responses(&source_path, physical, 0)[0].clone()
    ]);
    let stable_url = server.url_root.clone();
    let control = WorkerControl::default();
    let hook_control = control.clone();
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = Arc::new(ScienceCheckpointAnchorFixture::default());
    let authority = science_authority(
        bootstrap.as_ref(),
        &asset,
        &work,
        &completed,
        anchor.as_ref(),
    );
    let executor = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        asset,
    )
    .unwrap()
    .with_after_commit_hook_for_test(Arc::new(move |index| {
        if index == 0 {
            hook_control.set_paused_for_test(true);
        }
    }));
    assert_eq!(
        executor.execute(&work, &completed, &authority, &control),
        Err(Error::Paused)
    );
    assert_eq!(server.finish().len(), 1);
    (
        case,
        work,
        completed,
        bootstrap,
        anchor,
        stable_url,
        source_path,
    )
}

#[test]
fn science_range_v2_transfers_canonical_dependency_and_science_ranges_with_exact_http_evidence() {
    let (case, intent, bootstrap, source_path) =
        concrete_fixture("science-v2-exact", NisarSubsetFrequency::A, &["HH", "HV"]);
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();

    let physical = completed.physical_execution_plan();
    let execution_ranges = physical.execution_transfer_ranges().unwrap();
    assert!(execution_ranges.len() > physical.science_ranges().len());
    let execution_bytes = execution_ranges
        .iter()
        .map(|range| range.len().unwrap())
        .sum::<u64>();
    let budget = physical.budget_projection();
    assert_eq!(
        budget.total_request_count() - budget.committed_planning_request_count(),
        execution_ranges.len() as u64
    );
    assert_eq!(
        budget.total_response_body_bytes() - budget.committed_planning_response_body_bytes(),
        execution_bytes
    );
    let execution_owners = execution_ranges
        .iter()
        .flat_map(|range| range.logical_owners())
        .map(|owner| owner.dataset_path.as_str())
        .collect::<BTreeSet<_>>();
    assert!(
        execution_owners.contains("/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask")
    );
    assert!(execution_owners.contains("/science/LSAR/RSLC/swaths/frequencyA/validSamplesSubSwath1"));
    let planned = execution_ranges
        .iter()
        .map(|range| (range.start(), range.end_exclusive()))
        .collect::<Vec<_>>();
    let etag = physical.source_validator().value.clone();
    let source_size = physical.source_size();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server_plan = planned.clone();
    let server_etag = etag.clone();
    let worker = thread::spawn(move || {
        let mut source = fs::File::open(source_path).unwrap();
        let mut requests = Vec::new();
        for (start, end_exclusive) in server_plan {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).unwrap();
            requests.push(read_science_request(&mut stream));
            let mut body = vec![0_u8; (end_exclusive - start) as usize];
            source.seek(SeekFrom::Start(start)).unwrap();
            source.read_exact(&mut body).unwrap();
            write!(
                stream,
                "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nETag: {server_etag}\r\nContent-Range: bytes {start}-{}/{source_size}\r\nContent-Length: {}\r\n\r\n",
                end_exclusive - 1,
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        }
        requests
    });
    let http = HttpTransfer::new_with_proxy_policy(
        TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            chunk_size: 8,
            ..TransferOptions::default()
        },
        ProxyPolicy::Direct,
    )
    .unwrap();
    let asset = ResolvedAsset::new(
        &format!(
            "http://{address}/{}",
            physical.source_asset_identity().file_name
        ),
        ResolvedMethod::Get,
    )
    .unwrap()
    .with_expected_size(Some(source_size))
    .with_checksum(
        physical
            .source_asset_identity()
            .checksum
            .as_ref()
            .map(|checksum| {
                let algorithm = match checksum.algorithm {
                    insar_core::NisarSubsetChecksumAlgorithm::Md5 => "md5",
                    insar_core::NisarSubsetChecksumAlgorithm::Sha1 => "sha1",
                    insar_core::NisarSubsetChecksumAlgorithm::Sha256 => "sha256",
                };
                ResolvedChecksum::new(algorithm, &checksum.value).unwrap()
            }),
    )
    .with_remote_validator(RemoteValidator {
        etag: Some(etag.clone()),
        last_modified: None,
        content_length: Some(source_size),
    });
    assert!(!bootstrap
        .science_workspace_v2()
        .science_checkpoint_path_v2()
        .exists());
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor =
        crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(http, asset)
            .unwrap();
    let mut progress_deltas = Vec::new();
    let mut transfer_complete_after_bytes = None;
    let result = executor
        .execute_with_progress(
            &work,
            &completed,
            &authority,
            &WorkerControl::default(),
            &mut |event| match event {
                crate::nisar_science_transfer_v2::NisarScienceTransferProgressV2::Bytes(delta) => {
                    progress_deltas.push(delta)
                }
                crate::nisar_science_transfer_v2::NisarScienceTransferProgressV2::TransferComplete => {
                    transfer_complete_after_bytes = Some(progress_deltas.len())
                }
            },
        )
        .unwrap();
    assert_eq!(
        transfer_complete_after_bytes,
        Some(progress_deltas.len()),
        "the end of the transfer is reported once, after every byte"
    );

    assert!(result.complete());
    assert_eq!(result.science_request_count(), planned.len() as u64);
    assert_eq!(
        result.science_response_body_bytes(),
        execution_ranges
            .iter()
            .map(|range| range.len().unwrap())
            .sum::<u64>()
    );
    assert_eq!(result.resumed_bytes(), 0);
    assert!(progress_deltas.len() >= planned.len());
    assert!(progress_deltas.len() > 1);
    assert!(progress_deltas.iter().all(|delta| *delta > 0));
    assert_eq!(progress_deltas.iter().sum::<u64>(), execution_bytes);
    let requests = worker.join().unwrap();
    assert_eq!(requests.len(), planned.len());
    for (request, (start, end_exclusive)) in requests.iter().zip(planned) {
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains(&format!("range: bytes={start}-{}", end_exclusive - 1)));
        assert!(lower.contains(&format!("if-range: {}", etag.to_ascii_lowercase())));
    }
}

#[test]
fn science_range_v2_requests_shared_physical_range_once_and_retains_every_owner() {
    let (case, intent, bootstrap, source_path) = concrete_fixture_with_source_mutation(
        "science-v2-shared-owner",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
        |file| {
            file.unlink("/science/LSAR/RSLC/swaths/frequencyA/HV")
                .unwrap();
            file.link_hard(
                "/science/LSAR/RSLC/swaths/frequencyA/HH",
                "/science/LSAR/RSLC/swaths/frequencyA/HV",
            )
            .unwrap();
        },
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    assert_eq!(physical.science_ranges().len(), 4);
    assert!(physical.science_ranges().iter().all(|range| {
        range
            .logical_owners()
            .iter()
            .filter_map(|owner| owner.polarization.as_deref())
            .collect::<BTreeSet<_>>()
            == BTreeSet::from(["HH", "HV"])
    }));
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        asset,
    )
    .unwrap();
    let result = executor
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    let execution_range_count = physical.execution_transfer_ranges().unwrap().len();
    assert_eq!(result.science_request_count(), execution_range_count as u64);
    let requests = server.finish();
    assert_eq!(requests.len(), execution_range_count);
    let checkpoint = fs::read_to_string(
        bootstrap
            .science_workspace_v2()
            .science_checkpoint_path_v2(),
    )
    .unwrap();
    assert!(checkpoint.contains("\"HH\""));
    assert!(checkpoint.contains("\"HV\""));

    let output_root = case._root.0.join("writer-v2-shared-owner-output");
    let written = crate::nisar_subset_writer::tests::writer(&output_root)
        .write_and_validate_v2(
            &work,
            &completed,
            &result,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap();
    let artifact = hdf5::File::open(written.artifact_path()).unwrap();
    let root = "/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA";
    assert!(artifact.dataset(&format!("{root}/HH")).is_ok());
    assert!(artifact.dataset(&format!("{root}/HV")).is_ok());
    assert_eq!(written.manifest().dataset_window_count(), 2);
    assert_eq!(
        fs::read_dir(output_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5"))
            .count(),
        1
    );
}

fn execute_v2_writer_fixture(
    case: &ReadyCase,
    intent: &NisarSubsetPreparedIntentV2,
    bootstrap: &Arc<NisarSubsetLocalHdf5MetadataBootstrapV2>,
    source_path: &std::path::Path,
    output_label: &str,
) -> crate::nisar_subset_writer::NisarSubsetWriteResultV2 {
    let mut work = work(case, intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(source_path, physical, 0));
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let science = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        asset,
    )
    .unwrap()
    .execute(&work, &completed, &authority, &WorkerControl::default())
    .unwrap();
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );
    crate::nisar_subset_writer::tests::writer(&case._root.0.join(output_label))
        .write_and_validate_v2(
            &work,
            &completed,
            &science,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap()
}

#[test]
fn writer_v2_sband_a_rh_promotes_ssar_hdf5_without_lsar_alias() {
    let (case, intent, bootstrap, source_path, _source_pin) =
        concrete_sband_fixture("writer-v2-sband-a-rh");
    let selected_path =
        crate::nisar_science_root::NisarScienceRoot::Ssar.selected_science_path("A", "RH");
    let physical_catalog = crate::hdf5_range_transfer::inspect_nisar_planning_physical_ranges_v2(
        &source_path,
        &[selected_path],
        crate::nisar_science_root::NisarScienceRoot::Ssar,
    )
    .expect("SSAR physical catalog must be readable before V2 bootstrap");
    assert!(!physical_catalog.selected_science_ranges.is_empty());
    assert!(!physical_catalog.metadata_ranges.is_empty());
    let source_extent = fs::metadata(&source_path).unwrap().len();
    let materialization =
        complement_ranges(&physical_catalog.all_science_ranges, source_extent).unwrap();
    for required in physical_catalog
        .metadata_ranges
        .iter()
        .chain(&physical_catalog.geolocation_ranges)
    {
        assert!(
            materialization
                .iter()
                .any(|available| available.0 <= required.0 && available.1 >= required.1),
            "required SSAR metadata range {required:?} must be materialized outside science payload"
        );
    }
    let source_validator = NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: format!("\"sha256-{}\"", test_file_sha256(&source_path)),
        content_length: source_extent,
    };
    let inventory =
        crate::nisar_subset_writer::NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
            &intent,
            source_validator,
            &source_path,
            &|| false,
        );
    if let Err(error) = &inventory {
        let diagnostic = crate::nisar_subset_writer::metadata_capture_diagnostic_for_test();
        panic!(
            "complete SSAR source metadata inventory failed: {error:?}; stage={}; path={:?}; detail={:?}",
            diagnostic.stage,
            diagnostic.object_path,
            diagnostic.detail
        );
    }
    let inventory = inventory
        .expect("complete SSAR source must yield a V2 metadata inventory before sparse bootstrap");
    if let Err(error) =
        crate::nisar_subset_writer::build_nisar_subset_source_dependency_graph_v2(&inventory)
    {
        panic!("complete SSAR source dependency graph failed: {error:?}");
    }
    let result = execute_v2_writer_fixture(
        &case,
        &intent,
        &bootstrap,
        &source_path,
        "writer-v2-sband-a-rh-output",
    );

    let artifact = hdf5::File::open(result.artifact_path()).unwrap();
    let derived = "/science/SSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA/RH";
    assert!(artifact.dataset(derived).is_ok());
    assert!(artifact.group("/science/LSAR").is_err());
    assert!(artifact.group("/science/SSAR/identification").is_ok());
    let radar_band = artifact
        .dataset("/science/SSAR/identification/radarBand")
        .unwrap()
        .read_scalar::<hdf5::types::FixedAscii<32>>()
        .unwrap();
    assert_eq!(radar_band.as_str(), "S");
    assert_eq!(result.manifest().selected_polarizations(), ["RH"]);
    assert_eq!(result.manifest().output_authority(), "client_derived");
    assert_eq!(result.manifest().quality_status(), "PROVISIONAL");
}

#[test]
fn writer_v2_a_hh_promotes_one_client_derived_hdf5() {
    let (case, intent, bootstrap, source_path) =
        concrete_fixture("writer-v2-a-hh", NisarSubsetFrequency::A, &["HH"]);
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let science = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        asset,
    )
    .unwrap()
    .execute(&work, &completed, &authority, &WorkerControl::default())
    .unwrap();
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );

    let output_root = case._root.0.join("writer-v2-a-hh-output");
    let result = crate::nisar_subset_writer::tests::writer(&output_root)
        .write_and_validate_v2(
            &work,
            &completed,
            &science,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap();

    assert!(result.artifact_path().is_file());
    assert!(result.manifest_path().is_file());
    assert_eq!(
        fs::read_dir(&output_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5"))
            .count(),
        1
    );
    let artifact = hdf5::File::open(result.artifact_path()).unwrap();
    assert!(artifact
        .dataset("/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA/HH",)
        .is_ok());
    assert_eq!(result.manifest().selected_polarizations(), ["HH"]);
    assert_eq!(result.manifest().output_authority(), "client_derived");
    assert_eq!(result.manifest().quality_status(), "PROVISIONAL");
}

#[test]
fn writer_v2_a_hh_hv_promotes_one_hdf5_with_only_selected_polarizations() {
    let (case, intent, bootstrap, source_path) =
        concrete_fixture("writer-v2-a-hh-hv", NisarSubsetFrequency::A, &["HH", "HV"]);
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let science = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        asset,
    )
    .unwrap()
    .execute(&work, &completed, &authority, &WorkerControl::default())
    .unwrap();
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );

    let output_root = case._root.0.join("writer-v2-a-hh-hv-output");
    let result = crate::nisar_subset_writer::tests::writer(&output_root)
        .write_and_validate_v2(
            &work,
            &completed,
            &science,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap();

    assert_eq!(
        fs::read_dir(&output_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5"))
            .count(),
        1
    );
    let artifact = hdf5::File::open(result.artifact_path()).unwrap();
    let root = "/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA";
    assert!(artifact.dataset(&format!("{root}/HH")).is_ok());
    assert!(artifact.dataset(&format!("{root}/HV")).is_ok());
    assert!(artifact.dataset(&format!("{root}/VH")).is_err());
    assert!(artifact.dataset(&format!("{root}/VV")).is_err());
    assert!(artifact
        .dataset("/science/LSAR/RSLC/swaths/frequencyA/HH")
        .is_err());
    assert!(artifact
        .dataset("/science/LSAR/RSLC/swaths/frequencyA/HV")
        .is_err());
    assert!(artifact
        .dataset("/science/LSAR/RSLC/swaths/frequencyA/RV")
        .is_err());
    assert_eq!(result.manifest().selected_polarizations(), ["HH", "HV"]);
}

#[test]
fn writer_v2_preserves_legal_unnamed_metadata_scale_and_attachments() {
    const HEIGHT: &str = "/science/LSAR/RSLC/metadata/geolocationGrid/heightAboveEllipsoid";
    const COORDINATE_X: &str = "/science/LSAR/RSLC/metadata/geolocationGrid/coordinateX";
    let (case, intent, bootstrap, source_path) = concrete_fixture_with_source_mutation(
        "writer-v2-unnamed-scale",
        NisarSubsetFrequency::A,
        &["HH"],
        |file| {
            let height = file.dataset(HEIGHT).unwrap();
            assert_eq!(
                crate::nisar_hdf5_dimension_scales::remove_scale_name_for_test(&height),
                Ok(())
            );
        },
    );
    let result = execute_v2_writer_fixture(
        &case,
        &intent,
        &bootstrap,
        &source_path,
        "writer-v2-unnamed-scale-output",
    );
    let output = hdf5::File::open(result.artifact_path()).unwrap();
    let height = output.dataset(HEIGHT).unwrap();
    let height_graph = crate::nisar_hdf5_dimension_scales::read_graph(&height).unwrap();
    assert!(height_graph.is_scale);
    assert_eq!(height_graph.scale_name, None);
    assert!(height.attr("NAME").is_err());
    let coordinate_graph =
        crate::nisar_hdf5_dimension_scales::read_graph(&output.dataset(COORDINATE_X).unwrap())
            .unwrap();
    assert_eq!(coordinate_graph.scales[0], [HEIGHT]);
}

#[test]
fn writer_v2_frequency_b_hv_preserves_frequency_and_dependency_metadata() {
    let (case, intent, bootstrap, source_path) =
        concrete_fixture("writer-v2-b-hv", NisarSubsetFrequency::B, &["HV"]);
    let result = execute_v2_writer_fixture(
        &case,
        &intent,
        &bootstrap,
        &source_path,
        "writer-v2-b-hv-output",
    );
    let artifact = hdf5::File::open(result.artifact_path()).unwrap();
    assert!(artifact
        .dataset("/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyB/HV",)
        .is_ok());
    assert!(artifact
        .dataset("/science/LSAR/RSLC/metadata/orbit/time")
        .is_ok());
    assert_eq!(result.manifest().selected_polarizations(), ["HV"]);
}

#[test]
fn writer_v2_hh_hv_distinct_shapes_and_chunks_write_independently() {
    const HV: &str = "/science/LSAR/RSLC/swaths/frequencyA/HV";
    let (case, intent, bootstrap, source_path) = concrete_fixture_with_source_mutation(
        "writer-v2-distinct-layout",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
        |file| {
            file.unlink(HV).unwrap();
            let values = (0..12)
                .map(|index| TestCFloat32 {
                    r: 100.0 + index as f32,
                    i: -100.0 - index as f32,
                })
                .collect::<Vec<_>>();
            file.group("/science/LSAR/RSLC/swaths/frequencyA")
                .unwrap()
                .new_dataset::<TestCFloat32>()
                .shape((4, 3))
                .chunk((1, 3))
                .create("HV")
                .and_then(|dataset| dataset.write_raw(&values))
                .unwrap();
        },
    );
    let result = execute_v2_writer_fixture(
        &case,
        &intent,
        &bootstrap,
        &source_path,
        "writer-v2-distinct-layout-output",
    );
    let artifact = hdf5::File::open(result.artifact_path()).unwrap();
    let root = "/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA";
    let hh = artifact.dataset(&format!("{root}/HH")).unwrap();
    let hv = artifact.dataset(&format!("{root}/HV")).unwrap();
    assert_ne!(hh.shape(), hv.shape());
    assert_ne!(hh.chunk(), hv.chunk());
    assert_eq!(hv.shape()[1], 3);
}

#[test]
fn writer_v2_multipolygon_hh_hv_remains_one_hdf5_with_complete_ownership() {
    let (case, _intent, bootstrap, source_path) = concrete_fixture(
        "writer-v2-multipolygon",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
    );
    let intent = prepared_intent_with_requested_geometry(
        &case,
        NisarSubsetFrequency::A,
        &["HH", "HV"],
        json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[36.10, 9.10], [36.30, 9.10], [36.30, 9.30], [36.10, 9.30], [36.10, 9.10]]],
                [[[36.70, 9.70], [36.90, 9.70], [36.90, 9.90], [36.70, 9.90], [36.70, 9.70]]]
            ]
        }),
    );
    let result = execute_v2_writer_fixture(
        &case,
        &intent,
        &bootstrap,
        &source_path,
        "writer-v2-multipolygon-output",
    );
    let artifact = hdf5::File::open(result.artifact_path()).unwrap();
    for (component, window) in [(0, 0), (1, 1)] {
        let root = format!(
            "/science/LSAR/RSLC/derivedSubsets/component{component:03}/window{window:03}/swaths/frequencyA"
        );
        assert!(artifact.dataset(&format!("{root}/HH")).is_ok());
        assert!(artifact.dataset(&format!("{root}/HV")).is_ok());
    }
    assert_eq!(result.manifest().dataset_window_count(), 4);
    assert_eq!(
        fs::read_dir(case._root.0.join("writer-v2-multipolygon-output"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5"))
            .count(),
        1
    );
}

#[test]
fn writer_v2_validator_rejects_corruption_and_recovery_closes_promotion_gaps() {
    use crate::nisar_subset_writer::{NisarSubsetWriteError as Error, WriterFailpointV2};

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "writer-v2-validator-recovery",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let science = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        asset,
    )
    .unwrap()
    .execute(&work, &completed, &authority, &WorkerControl::default())
    .unwrap();
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );

    let published_replacement_root = case
        ._root
        .0
        .join("writer-v2-published-path-replacement-before-anchor");
    assert_eq!(
        crate::nisar_subset_writer::tests::writer(&published_replacement_root)
            .write_and_validate_v2_with_failpoint_for_test(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
                WriterFailpointV2::ReplacePublishedArtifactBeforeArtifactAnchor,
            )
            .unwrap_err(),
        Error::RecoveryInvalid
    );
    assert_eq!(
        crate::nisar_subset_writer::tests::recovery_anchor_state_for_test(
            &published_replacement_root
        ),
        Some("CandidateValidated")
    );
    assert!(
        fs::read_dir(&published_replacement_root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .ends_with(".artifact-manifest-v2.json")),
        "a replaced final directory entry must never receive a sidecar"
    );
    assert!(writer_v2_candidate_paths(&published_replacement_root).is_empty());

    for (label, failpoint) in [
        ("science", WriterFailpointV2::CorruptScienceBeforeValidation),
        (
            "science-shape",
            WriterFailpointV2::CorruptScienceShapeBeforeValidation,
        ),
        (
            "coordinate",
            WriterFailpointV2::CorruptCoordinateBeforeValidation,
        ),
        ("scale", WriterFailpointV2::CorruptScaleBeforeValidation),
        ("mask", WriterFailpointV2::CorruptMaskBeforeValidation),
        (
            "provenance-digest",
            WriterFailpointV2::CorruptProvenanceBeforeValidation,
        ),
        ("quality", WriterFailpointV2::CorruptQualityBeforeValidation),
        (
            "client-derived-authority",
            WriterFailpointV2::CorruptAuthorityBeforeValidation,
        ),
        (
            "metadata",
            WriterFailpointV2::CorruptMetadataBeforeValidation,
        ),
        (
            "candidate-post-semantic-pre-hash-race",
            WriterFailpointV2::MutateCandidateAfterSemanticBeforeHash,
        ),
        (
            "extra-unselected-source-payload",
            WriterFailpointV2::AddUnselectedSourcePayloadBeforeValidation,
        ),
        (
            "duplicate-ancillary",
            WriterFailpointV2::AddDuplicateAncillaryBeforeValidation,
        ),
        (
            "extra-group",
            WriterFailpointV2::AddExtraGroupBeforeValidation,
        ),
    ] {
        let output_root = case._root.0.join(format!("writer-v2-corrupt-{label}"));
        let error = crate::nisar_subset_writer::tests::writer(&output_root)
            .write_and_validate_v2_with_failpoint_for_test(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
                failpoint,
            )
            .unwrap_err();
        assert_eq!(error, Error::PostWriteScienceValidation, "{label}");
        assert_eq!(
            fs::read_dir(&output_root)
                .unwrap()
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5")
                )
                .count(),
            0,
            "{label}"
        );
        assert!(
            writer_v2_candidate_paths(&output_root).is_empty(),
            "{label}"
        );
    }

    let before_root = case._root.0.join("writer-v2-crash-before-promotion");
    let before_writer = crate::nisar_subset_writer::tests::writer(&before_root);
    assert_eq!(
        before_writer
            .write_and_validate_v2_with_failpoint_for_test(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
                WriterFailpointV2::AfterValidationBeforePromotion,
            )
            .unwrap_err(),
        Error::InjectedFailure
    );
    assert_eq!(
        fs::read_dir(&before_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5"))
            .count(),
        0
    );
    let crashed_candidates = writer_v2_candidate_paths(&before_root);
    #[cfg(any(windows, target_os = "macos"))]
    assert_eq!(crashed_candidates.len(), 1);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    assert!(crashed_candidates.is_empty());
    #[cfg(any(windows, target_os = "macos"))]
    let stable_candidate_name = crashed_candidates[0].file_name().unwrap().to_owned();
    drop(before_writer);
    let restarted_before_writer = crate::nisar_subset_writer::tests::writer(&before_root);
    assert!(restarted_before_writer
        .write_and_validate_v2(
            &work,
            &completed,
            &science,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap()
        .manifest_path()
        .is_file());
    assert!(writer_v2_candidate_paths(&before_root).is_empty());
    #[cfg(any(windows, target_os = "macos"))]
    assert!(!before_root.join(stable_candidate_name).exists());

    #[cfg(any(windows, target_os = "macos"))]
    for attack in ["corruption", "replacement"] {
        let recovery_root = case
            ._root
            .0
            .join(format!("writer-v2-stable-candidate-{attack}"));
        let recovery_writer = crate::nisar_subset_writer::tests::writer(&recovery_root);
        assert_eq!(
            recovery_writer
                .write_and_validate_v2_with_failpoint_for_test(
                    &work,
                    &completed,
                    &science,
                    &authority,
                    &WorkerControl::default(),
                    WriterFailpointV2::AfterValidationBeforePromotion,
                )
                .unwrap_err(),
            Error::InjectedFailure,
            "{attack}"
        );
        let candidates = writer_v2_candidate_paths(&recovery_root);
        assert_eq!(candidates.len(), 1, "{attack}");
        let candidate = &candidates[0];
        let displaced = recovery_root.join(format!(".{attack}-displaced-candidate"));
        let replacement_inode = if attack == "replacement" {
            let anchored_sha256 = test_file_sha256(candidate);
            fs::rename(candidate, &displaced).unwrap();
            fs::copy(&displaced, candidate).unwrap();
            // The candidate path can exceed MAX_PATH under a deep TEMP (L3 gate).
            let replacement =
                hdf5::File::open_rw(crate::long_path::native_path(candidate)).unwrap();
            replacement
                .create_group("/__coherent_replacement_free_space")
                .unwrap();
            replacement
                .unlink("/__coherent_replacement_free_space")
                .unwrap();
            replacement.flush().unwrap();
            drop(replacement);
            assert_ne!(anchored_sha256, test_file_sha256(candidate));
            true
        } else {
            let corrupted_in_place = (|| -> std::io::Result<()> {
                let mut candidate_file =
                    OpenOptions::new().read(true).write(true).open(candidate)?;
                let mut byte = [0_u8; 1];
                candidate_file.read_exact(&mut byte)?;
                candidate_file.seek(SeekFrom::Start(0))?;
                candidate_file.write_all(&[byte[0] ^ 0x5a])?;
                candidate_file.sync_all()
            })()
            .is_ok();
            if !corrupted_in_place {
                fs::rename(candidate, &displaced).unwrap();
                let replacement = OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .write(true)
                    .open(candidate)
                    .unwrap();
                replacement
                    .set_len(fs::metadata(&displaced).unwrap().len())
                    .unwrap();
                replacement.sync_all().unwrap();
                true
            } else {
                false
            }
        };
        assert_eq!(
            recovery_writer
                .write_and_validate_v2(
                    &work,
                    &completed,
                    &science,
                    &authority,
                    &WorkerControl::default(),
                )
                .unwrap_err(),
            Error::RecoveryInvalid,
            "{attack}"
        );
        if replacement_inode {
            assert_eq!(
                writer_v2_candidate_paths(&recovery_root),
                vec![candidate.clone()]
            );
            assert!(candidate.is_file(), "replacement inode must not be deleted");
            fs::remove_file(candidate).unwrap();
        } else {
            assert!(
                writer_v2_candidate_paths(&recovery_root).is_empty(),
                "{attack}"
            );
        }
        assert_eq!(
            fs::read_dir(&recovery_root)
                .unwrap()
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5")
                )
                .count(),
            0,
            "{attack}"
        );
        if displaced.exists() {
            fs::remove_file(displaced).unwrap();
        }
    }

    let after_root = case._root.0.join("writer-v2-crash-after-artifact");
    let after_writer = crate::nisar_subset_writer::tests::writer(&after_root);
    assert_eq!(
        after_writer
            .write_and_validate_v2_with_failpoint_for_test(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
                WriterFailpointV2::AfterArtifactPromotion,
            )
            .unwrap_err(),
        Error::InjectedFailure
    );
    assert_eq!(
        fs::read_dir(&after_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5"))
            .count(),
        1
    );
    let recovered = after_writer
        .write_and_validate_v2(
            &work,
            &completed,
            &science,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap();
    assert!(recovered.artifact_path().is_file());
    assert!(recovered.manifest_path().is_file());

    let coherent_final_root = case._root.0.join("writer-v2-coherent-final-replacement");
    let coherent_final_writer = crate::nisar_subset_writer::tests::writer(&coherent_final_root);
    assert_eq!(
        coherent_final_writer
            .write_and_validate_v2_with_failpoint_for_test(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
                WriterFailpointV2::AfterArtifactPromotion,
            )
            .unwrap_err(),
        Error::InjectedFailure
    );
    let coherent_artifact = fs::read_dir(&coherent_final_root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("h5"))
        .unwrap();
    let anchored_sha256 = test_file_sha256(&coherent_artifact);
    let anchored_original = coherent_final_root.join(".anchored-original.h5");
    fs::rename(&coherent_artifact, &anchored_original).unwrap();
    fs::copy(&anchored_original, &coherent_artifact).unwrap();
    // On Unix fs::copy carries the sealed artifact's read-only mode; the
    // replacement this attack edits is its own writable file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&coherent_artifact, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let coherent_replacement =
        hdf5::File::open_rw(crate::long_path::native_path(&coherent_artifact)).unwrap();
    coherent_replacement
        .create_group("/__coherent_final_free_space")
        .unwrap();
    coherent_replacement
        .unlink("/__coherent_final_free_space")
        .unwrap();
    coherent_replacement.flush().unwrap();
    drop(coherent_replacement);
    assert_ne!(
        anchored_sha256,
        test_file_sha256(&coherent_artifact),
        "coherent replacement fixture must have different physical bytes"
    );
    drop(coherent_final_writer);
    assert_eq!(
        crate::nisar_subset_writer::tests::writer(&coherent_final_root)
            .write_and_validate_v2(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
            )
            .unwrap_err(),
        Error::RecoveryInvalid
    );
    assert!(
        coherent_artifact.is_file(),
        "replacement final must not be deleted"
    );
    assert!(anchored_original.is_file());
    fs::remove_file(coherent_artifact).unwrap();
    fs::remove_file(anchored_original).unwrap();

    fs::remove_file(recovered.manifest_path()).unwrap();
    // A same-user attacker on Unix first lifts the sealed artifact's
    // read-only mode, as only root could write it without that.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(recovered.artifact_path(), fs::Permissions::from_mode(0o600)).unwrap();
    }
    let artifact =
        hdf5::File::open_rw(crate::long_path::native_path(recovered.artifact_path())).unwrap();
    artifact
        .create_group("/coherentUnexpectedRecoveryGroup")
        .unwrap();
    artifact.flush().unwrap();
    drop(artifact);
    assert_eq!(
        after_writer
            .write_and_validate_v2(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
            )
            .unwrap_err(),
        Error::RecoveryInvalid
    );
}

#[test]
fn writer_v2_revalidates_caller_physical_checkpoint_and_part_authority_before_candidate() {
    use crate::nisar_subset_writer::NisarSubsetWriteError as Error;

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "writer-v2-authority-revalidation",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let science = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        asset,
    )
    .unwrap()
    .execute(&work, &completed, &authority, &WorkerControl::default())
    .unwrap();
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );

    let mut caller_incomplete = science.clone();
    caller_incomplete.mark_incomplete_for_test();
    let incomplete_root = case._root.0.join("writer-v2-incomplete-caller");
    assert_eq!(
        crate::nisar_subset_writer::tests::writer(&incomplete_root)
            .write_and_validate_v2(
                &work,
                &completed,
                &caller_incomplete,
                &authority,
                &WorkerControl::default(),
            )
            .unwrap_err(),
        Error::IncompleteScienceAuthority
    );

    let mut physical_drift = completed.clone();
    physical_drift.physical_execution_plan.science_ranges[0].remove_logical_owner_for_test();
    let physical_root = case._root.0.join("writer-v2-physical-drift");
    assert_eq!(
        crate::nisar_subset_writer::tests::writer(&physical_root)
            .write_and_validate_v2(
                &work,
                &physical_drift,
                &science,
                &authority,
                &WorkerControl::default(),
            )
            .unwrap_err(),
        Error::IncompleteScienceAuthority
    );

    crate::nisar_science_transfer_v2::mutate_science_checkpoint_v2_for_test(
        bootstrap.science_workspace_v2(),
        crate::nisar_science_transfer_v2::ScienceCheckpointMutationV2ForTest::DecrementCommittedScienceBytes,
    );
    let checkpoint_root = case._root.0.join("writer-v2-checkpoint-drift");
    assert_eq!(
        crate::nisar_subset_writer::tests::writer(&checkpoint_root)
            .write_and_validate_v2(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
            )
            .unwrap_err(),
        Error::IncompleteScienceAuthority
    );
    for root in [incomplete_root, physical_root, checkpoint_root] {
        assert_eq!(
            fs::read_dir(root)
                .unwrap()
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5")
                )
                .count(),
            0
        );
    }
}

#[test]
fn writer_v2_pins_part_identity_and_material_across_every_read_and_promotion() {
    use crate::nisar_subset_writer::PartAttackOutcomeForTest as PartAttack;
    use crate::nisar_subset_writer::{NisarSubsetWriteError as Error, WriterFailpointV2};

    for (label, failpoint) in [
        (
            "path-replacement",
            WriterFailpointV2::ReplacePartBeforeMaterialCapture,
        ),
        (
            "symlink-replacement",
            WriterFailpointV2::SymlinkPartBeforeMaterialCapture,
        ),
        (
            "hardlink-alias",
            WriterFailpointV2::HardlinkPartBeforeMaterialCapture,
        ),
        (
            "in-place-write",
            WriterFailpointV2::WritePartBeforeMaterialCapture,
        ),
        (
            "truncation",
            WriterFailpointV2::TruncatePartBeforeMaterialCapture,
        ),
        (
            "truncate-restore-size-and-times",
            WriterFailpointV2::TruncateRestorePartBeforeMaterialCapture,
        ),
        (
            "mutate-during-hdf5-read-restore",
            WriterFailpointV2::MutateSourceDuringHdf5ReadAndRestore,
        ),
        (
            "promotion-race",
            WriterFailpointV2::WritePartImmediatelyBeforePromotion,
        ),
    ] {
        let fixture_label = format!("writer-v2-pinned-part-{label}");
        let (case, intent, bootstrap, source_path) =
            concrete_fixture(&fixture_label, NisarSubsetFrequency::A, &["HH"]);
        let mut work = work(&case, &intent);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan())
            .unwrap();
        let physical = completed.physical_execution_plan();
        let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
        let asset = science_asset(
            &server.url_root,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        );
        let anchor = ScienceCheckpointAnchorFixture::default();
        let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
        let science = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
            science_http(),
            asset,
        )
        .unwrap()
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
        assert_eq!(
            server.finish().len(),
            physical.execution_transfer_ranges().unwrap().len()
        );
        let output_root = case
            ._root
            .0
            .join(format!("writer-v2-pinned-part-{label}-output"));
        assert_eq!(
            crate::nisar_subset_writer::tests::writer(&output_root)
                .write_and_validate_v2_with_failpoint_for_test(
                    &work,
                    &completed,
                    &science,
                    &authority,
                    &WorkerControl::default(),
                    failpoint,
                )
                .unwrap_err(),
            Error::IncompleteScienceAuthority,
            "{label}"
        );
        // Review of D1/D2 (downloader enhancement D1b, P2-6): which mechanism
        // refused the attack. Windows pins the part denying write and delete
        // sharing, so the OS refuses every attack but a hard link (which may
        // or may not be refused; the identity check sees its link count).
        // Elsewhere the attack is applied and the writer's identity check
        // (inode, size and change-time marker) refuses the part.
        let outcome = crate::nisar_subset_writer::take_last_part_attack_outcome_for_test()
            .expect("the attack ran");
        eprintln!("pinned-part attack {label}: {outcome:?}");
        // D1c: RefusedByOs now means a sharing or lock violation only; any
        // other failure (FailedOtherwise) fails this matrix.
        if !cfg!(windows) {
            assert_eq!(outcome, PartAttack::Applied, "{label}");
        } else if failpoint != WriterFailpointV2::HardlinkPartBeforeMaterialCapture {
            assert_eq!(outcome, PartAttack::RefusedByOs, "{label}");
        } else {
            assert!(
                matches!(outcome, PartAttack::Applied | PartAttack::RefusedByOs),
                "{label}: {outcome:?}"
            );
        }
        assert_eq!(
            fs::read_dir(&output_root)
                .unwrap()
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5")
                )
                .count(),
            0,
            "{label}"
        );
        // Review of slice 7 (Issue 8 slice 7b, requirement change): the promotion
        // race is detected after the CandidateValidated anchor. That candidate is
        // the anchored recovery state and is kept, because the part, not the
        // candidate, was changed; it used to be removed. The earlier attacks
        // happen before any anchor and still leave no candidate.
        if cfg!(any(windows, target_os = "macos"))
            && failpoint == WriterFailpointV2::WritePartImmediatelyBeforePromotion
        {
            assert_eq!(
                crate::nisar_subset_writer::tests::recovery_anchor_state_for_test(&output_root),
                Some("CandidateValidated"),
                "{label}"
            );
            assert_eq!(writer_v2_candidate_paths(&output_root).len(), 1, "{label}");
        } else {
            assert!(
                writer_v2_candidate_paths(&output_root).is_empty(),
                "{label}"
            );
        }
    }
}

#[test]
fn writer_v2_recovery_anchor_contract_is_sealed_and_durable() {
    use crate::nisar_subset_writer::{
        NisarSubsetWriterRecoveryAnchorRecordV2, NisarSubsetWriterRecoveryAnchorV2,
    };

    fn require_anchor(_anchor: &dyn NisarSubsetWriterRecoveryAnchorV2) {}
    let _record_lineage = NisarSubsetWriterRecoveryAnchorRecordV2::lineage_key_sha256;
    let _record_state = NisarSubsetWriterRecoveryAnchorRecordV2::state;
    let _ = require_anchor;
}

#[test]
fn writer_v2_crash_windows_rebuild_without_second_plan_and_copy_promotion_reanchors() {
    use crate::nisar_subset_writer::{NisarSubsetWriteError as Error, WriterFailpointV2};

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "writer-v2-lifecycle-crash-windows",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let science = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        asset,
    )
    .unwrap()
    .execute(&work, &completed, &authority, &WorkerControl::default())
    .unwrap();
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );

    crate::nisar_subset_writer::tests::reset_regular_building_recovery_hits_for_test();

    for (label, failpoint) in [
        (
            "pre-building-cas",
            WriterFailpointV2::AfterCandidateBootstrapBeforeBuildingAnchor,
        ),
        (
            "post-building-cas",
            WriterFailpointV2::AfterBuildingAnchorBeforeWrite,
        ),
        (
            "post-write",
            WriterFailpointV2::AfterCandidateWriteBeforeSync,
        ),
        ("post-sync", WriterFailpointV2::AfterCandidateSyncBeforeSeal),
        (
            "post-seal",
            WriterFailpointV2::AfterCandidateSealBeforeSemantic,
        ),
        (
            "post-semantic",
            WriterFailpointV2::AfterCandidateSemanticBeforeHash,
        ),
        (
            "post-hash",
            WriterFailpointV2::AfterCandidateHashBeforeValidatedAnchor,
        ),
        (
            "post-validated-cas",
            WriterFailpointV2::AfterValidationBeforePromotion,
        ),
        (
            "post-install-pre-artifact-cas",
            WriterFailpointV2::AfterArtifactPromotionBeforeAnchor,
        ),
        (
            "post-artifact-cas",
            WriterFailpointV2::AfterArtifactPromotion,
        ),
    ] {
        let output_root = case._root.0.join(format!("writer-v2-lifecycle-{label}"));
        assert_eq!(
            crate::nisar_subset_writer::tests::writer(&output_root)
                .write_and_validate_v2_with_failpoint_for_test(
                    &work,
                    &completed,
                    &science,
                    &authority,
                    &WorkerControl::default(),
                    failpoint,
                )
                .unwrap_err(),
            Error::InjectedFailure,
            "{label}"
        );
        let prevalidated = matches!(
            label,
            "post-building-cas"
                | "post-write"
                | "post-sync"
                | "post-seal"
                | "post-semantic"
                | "post-hash"
        );
        let expected_anchor_state = match label {
            "pre-building-cas" => None,
            "post-building-cas" | "post-write" | "post-sync" | "post-seal" | "post-semantic"
            | "post-hash" => Some("CandidateBuilding"),
            "post-validated-cas" | "post-install-pre-artifact-cas" => Some("CandidateValidated"),
            "post-artifact-cas" => Some("ArtifactPromoted"),
            _ => unreachable!(),
        };
        assert_eq!(
            crate::nisar_subset_writer::tests::recovery_anchor_state_for_test(&output_root),
            expected_anchor_state,
            "{label}"
        );
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert!(
            writer_v2_candidate_paths(&output_root).is_empty(),
            "{label}"
        );
        #[cfg(any(windows, target_os = "macos"))]
        {
            let expected_candidates = usize::from(prevalidated || label == "post-validated-cas");
            assert_eq!(
                writer_v2_candidate_paths(&output_root).len(),
                expected_candidates,
                "{label}"
            );
        }
        let hits_before =
            crate::nisar_subset_writer::tests::regular_building_recovery_hits_for_test();
        let recovered = crate::nisar_subset_writer::tests::writer(&output_root)
            .write_and_validate_v2(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
            )
            .unwrap();
        let hits_after =
            crate::nisar_subset_writer::tests::regular_building_recovery_hits_for_test();
        #[cfg(any(windows, target_os = "macos"))]
        assert_eq!(
            hits_after,
            hits_before + usize::from(prevalidated),
            "{label}"
        );
        #[cfg(not(any(windows, target_os = "macos")))]
        assert_eq!(hits_after, hits_before, "{label}");
        assert!(recovered.artifact_path().is_file(), "{label}");
        assert!(
            writer_v2_candidate_paths(&output_root).is_empty(),
            "{label}"
        );
        assert_eq!(
            crate::nisar_subset_writer::tests::recovery_anchor_state_for_test(&output_root),
            Some("Complete"),
            "{label}"
        );
    }

    let copy_root = case._root.0.join("writer-v2-copy-promotion-new-inode");
    let copy_writer = crate::nisar_subset_writer::tests::writer(&copy_root);
    assert_eq!(
        copy_writer
            .write_and_validate_v2_with_failpoint_for_test(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
                WriterFailpointV2::ForceCopyPromotionToNewInode,
            )
            .unwrap_err(),
        Error::InjectedFailure
    );
    assert!(writer_v2_candidate_paths(&copy_root).is_empty());
    assert_eq!(
        fs::read_dir(&copy_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5"))
            .count(),
        1
    );
    drop(copy_writer);
    let recovered = crate::nisar_subset_writer::tests::writer(&copy_root)
        .write_and_validate_v2(
            &work,
            &completed,
            &science,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap();
    assert!(recovered.artifact_path().is_file());
    assert!(
        crate::nisar_subset_writer::tests::recovery_anchor_has_distinct_promoted_identity_for_test(
            &copy_root
        ),
        "copy promotion must persist the installed artifact identity, not candidate identity"
    );
}

#[test]
fn writer_v2_source_snapshot_path_replacement_remains_descriptor_bound() {
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    use crate::nisar_subset_writer::NisarSubsetWriteError as Error;
    use crate::nisar_subset_writer::WriterFailpointV2;

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "writer-v2-source-snapshot-path-replacement",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let science = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        asset,
    )
    .unwrap()
    .execute(&work, &completed, &authority, &WorkerControl::default())
    .unwrap();
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );
    let output_root = case._root.0.join("writer-v2-source-snapshot-output");
    let result = crate::nisar_subset_writer::tests::writer(&output_root)
        .write_and_validate_v2_with_failpoint_for_test(
            &work,
            &completed,
            &science,
            &authority,
            &WorkerControl::default(),
            WriterFailpointV2::ReplaceSnapshotPathDuringHdf5Read,
        );
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    {
        let result = result.unwrap();
        assert!(result.artifact_path().is_file());
        let artifact = hdf5::File::open(result.artifact_path()).unwrap();
        let values = artifact
            .dataset(
                "/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequencyA/HH",
            )
            .unwrap()
            .read_raw::<TestCFloat32>()
            .unwrap();
        assert!(
            values.iter().all(|value| value.r < 10_000.0),
            "descriptor-bound read must not observe coherent replacement science"
        );
        assert!(
            bootstrap
                .science_workspace_v2()
                .writer_snapshot_path_v2_for_test()
                .is_file(),
            "replacement inode must not be removed by immutable-view teardown"
        );
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    assert_eq!(result.unwrap_err(), Error::IncompleteScienceAuthority);
}

#[test]
fn science_range_v2_rejects_unsupported_malformed_incomplete_and_validator_drift() {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as Error;

    for case_kind in [
        "http-200",
        "redirect",
        "wrong-content-range",
        "malformed-content-range",
        "short-body",
        "long-body",
        "weak-etag",
        "changed-etag",
        "missing-etag",
    ] {
        let label = format!("science-v2-protocol-{case_kind}");
        let (case, intent, bootstrap, source_path) =
            concrete_fixture(&label, NisarSubsetFrequency::A, &["HH"]);
        let mut work = work(&case, &intent);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan())
            .unwrap();
        let physical = completed.physical_execution_plan();
        let range = &physical.execution_transfer_ranges().unwrap()[0];
        let body = source_range_bytes(&source_path, range);
        let etag = &physical.source_validator().value;
        let expected = match case_kind {
            "http-200" => Error::RangeUnsupported,
            "short-body" => Error::ScienceTransferIncomplete,
            "changed-etag" => Error::StrongValidatorChanged,
            _ => Error::RangeResponseInvalid,
        };
        let response = match case_kind {
            "http-200" => science_response(
                "200 OK",
                &[("Content-Length", physical.source_size().to_string())],
                &body,
            ),
            "redirect" => science_response(
                "302 Found",
                &[
                    ("Location", "http://localhost:9/redirected".to_owned()),
                    ("Content-Length", "0".to_owned()),
                ],
                &[],
            ),
            "wrong-content-range" => science_response(
                "206 Partial Content",
                &[
                    ("ETag", etag.clone()),
                    (
                        "Content-Range",
                        format!(
                            "bytes {}-{}/{}",
                            range.start() + 1,
                            range.end_exclusive(),
                            physical.source_size()
                        ),
                    ),
                    ("Content-Length", range.len().unwrap().to_string()),
                ],
                &body,
            ),
            "malformed-content-range" => science_response(
                "206 Partial Content",
                &[
                    ("ETag", etag.clone()),
                    ("Content-Range", "not-a-range".to_owned()),
                    ("Content-Length", range.len().unwrap().to_string()),
                ],
                &body,
            ),
            "short-body" => {
                exact_science_response(range, physical.source_size(), etag, &body[..body.len() - 1])
            }
            "long-body" => science_response(
                "206 Partial Content",
                &[
                    ("ETag", etag.clone()),
                    (
                        "Content-Range",
                        format!(
                            "bytes {}-{}/{}",
                            range.start(),
                            range.end_exclusive() - 1,
                            physical.source_size()
                        ),
                    ),
                    ("Content-Length", (range.len().unwrap() + 1).to_string()),
                ],
                &[body.as_slice(), &[0_u8]].concat(),
            ),
            "weak-etag" => {
                exact_science_response(range, physical.source_size(), "W/\"weak\"", &body)
            }
            "changed-etag" => {
                exact_science_response(range, physical.source_size(), "\"changed\"", &body)
            }
            "missing-etag" => science_response(
                "206 Partial Content",
                &[
                    (
                        "Content-Range",
                        format!(
                            "bytes {}-{}/{}",
                            range.start(),
                            range.end_exclusive() - 1,
                            physical.source_size()
                        ),
                    ),
                    ("Content-Length", range.len().unwrap().to_string()),
                ],
                &body,
            ),
            _ => unreachable!(),
        };
        let expected = if matches!(case_kind, "redirect" | "weak-etag" | "missing-etag") {
            Error::ResponseInvalid
        } else {
            expected
        };
        // Design 2026-10-17 §2: a short body is transient, so it is retried up to
        // three attempts (first plus two automatic retries) before it is final;
        // every other case here is terminal after one attempt.
        let attempts: u64 = if case_kind == "short-body" { 3 } else { 1 };
        let server = ScienceServer::new(vec![response; attempts as usize]);
        let asset = science_asset(&server.url_root, physical, Some(etag), None, None);
        let anchor = ScienceCheckpointAnchorFixture::default();
        let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
        let executor = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
            science_http(),
            asset,
        )
        .unwrap()
        .with_retry_backoff_for_test(Arc::new(|_, _| Duration::ZERO));
        assert_eq!(
            executor.execute(&work, &completed, &authority, &WorkerControl::default(),),
            Err(expected),
            "case {case_kind}"
        );
        assert_ne!(expected.code(), "DL005");
        let checkpoint: Value = serde_json::from_str(
            &fs::read_to_string(
                bootstrap
                    .science_workspace_v2()
                    .science_checkpoint_path_v2(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_ne!(checkpoint["state"], json!("Complete"));
        assert_eq!(checkpoint["committed_science_request_count"], json!(0));
        assert_eq!(
            checkpoint["committed_science_response_body_bytes"],
            json!(0)
        );
        assert_eq!(
            checkpoint["reserved_science_request_count"],
            json!(attempts)
        );
        assert_eq!(
            checkpoint["reserved_science_response_body_bytes"],
            json!(attempts * range.len().unwrap())
        );
        assert_eq!(checkpoint["issued_science_request_count"], json!(attempts));
        assert_eq!(
            checkpoint["observed_science_response_body_bytes"],
            json!(if case_kind == "short-body" {
                attempts * (body.len() as u64 - 1)
            } else {
                0
            })
        );
        let requests = server.finish();
        assert_eq!(requests.len() as u64, attempts, "case {case_kind}");
        assert!(requests
            .iter()
            .all(|request| request.to_ascii_lowercase().contains(&format!(
                "range: bytes={}-{}",
                range.start(),
                range.end_exclusive() - 1
            ))));
    }
}

#[test]
fn science_range_v2_repeated_failed_gets_remain_charged_and_budget_exhaustion_is_pre_io() {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as Error;

    let (case, intent, bootstrap, _source_path) = concrete_fixture(
        "science-v2-repeated-failure-budget",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let budget = physical.budget_projection();
    let range_bytes = physical.execution_transfer_ranges().unwrap()[0]
        .len()
        .unwrap();
    let allowed_failures = budget
        .max_science_request_count()
        .min(budget.max_request_count() - budget.committed_planning_request_count())
        .min(
            (budget.max_response_body_bytes() - budget.committed_planning_response_body_bytes())
                / range_bytes,
        );
    assert!(allowed_failures > 0);
    let response = science_response(
        "200 OK",
        &[("Content-Length", physical.source_size().to_string())],
        &[],
    );
    let server = ScienceServer::new(vec![response; allowed_failures as usize]);
    let stable_url = server.url_root.clone();
    let authority_asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = StaticScienceAuthorityFixture {
        plan: physical.clone(),
        statistics: completed.bootstrap_statistics().clone(),
        workspace: bootstrap.science_workspace_v2(),
        remote_source: crate::nisar_science_transfer_v2::NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
            authority_asset.effective_url(),
            &work,
            &completed,
            physical,
        )
        .unwrap(),
        anchor: &anchor,
    };
    for _ in 0..allowed_failures {
        let executor = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
            science_http(),
            science_asset(
                &stable_url,
                physical,
                Some(&physical.source_validator().value),
                None,
                None,
            ),
        )
        .unwrap();
        assert_eq!(
            executor.execute(&work, &completed, &authority, &WorkerControl::default(),),
            Err(Error::RangeUnsupported)
        );
    }
    assert_eq!(server.finish().len(), allowed_failures as usize);
    let denied = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        science_asset(
            &stable_url,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        ),
    )
    .unwrap();
    assert_eq!(
        denied.execute(&work, &completed, &authority, &WorkerControl::default(),),
        Err(Error::CombinedBudgetExceeded)
    );
    let checkpoint: Value = serde_json::from_str(
        &fs::read_to_string(
            bootstrap
                .science_workspace_v2()
                .science_checkpoint_path_v2(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        checkpoint["reserved_science_request_count"],
        json!(allowed_failures)
    );
    assert_eq!(
        checkpoint["issued_science_request_count"],
        json!(allowed_failures)
    );
    assert_eq!(
        checkpoint["failed_science_request_count"],
        json!(allowed_failures)
    );
    assert_eq!(checkpoint["committed_science_request_count"], json!(0));
}

#[test]
fn science_range_v2_last_request_slot_rejects_redirect_before_second_hop() {
    assert_last_request_slots_reject_extra_hops(false, false);
}

#[test]
fn science_range_v2_last_request_slots_reject_auth_refresh_after_redirect() {
    assert_last_request_slots_reject_extra_hops(true, false);
}

#[test]
fn science_range_v2_last_request_slots_are_shared_across_auth_retry_redirect() {
    assert_last_request_slots_reject_extra_hops(true, true);
}

fn assert_last_request_slots_reject_extra_hops(auth_retry: bool, retry_redirect: bool) {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };
    let (case, intent, bootstrap, _) = concrete_fixture(
        "science-v2-last-request-slot",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let budget = physical.budget_projection();
    let count = budget
        .max_science_request_count()
        .min(budget.max_request_count() - budget.committed_planning_request_count());
    let bytes = physical.execution_transfer_ranges().unwrap()[0]
        .len()
        .unwrap();
    assert!(
        count * bytes
            <= budget.max_response_body_bytes() - budget.committed_planning_response_body_bytes()
    );
    let rejected = science_response(
        "200 OK",
        &[("Content-Length", physical.source_size().to_string())],
        &[],
    );
    let last_attempt_requests = if retry_redirect {
        3
    } else if auth_retry {
        2
    } else {
        1
    };
    let mut responses = vec![rejected; (count - last_attempt_requests) as usize];
    responses.push(science_response(
        "302 Found",
        &[
            ("Location", "/accepted-redirect.h5".to_owned()),
            ("Content-Length", "0".to_owned()),
        ],
        &[],
    ));
    if auth_retry {
        responses.push(science_response(
            "401 Unauthorized",
            &[("Content-Length", "0".to_owned())],
            &[],
        ));
    }
    if retry_redirect {
        responses.push(science_response(
            "302 Found",
            &[
                ("Location", "/retry-redirect.h5".to_owned()),
                ("Content-Length", "0".to_owned()),
            ],
            &[],
        ));
    }
    let server = ScienceServer::new(responses);
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = StaticScienceAuthorityFixture {
        plan: physical.clone(), statistics: completed.bootstrap_statistics().clone(),
        workspace: bootstrap.science_workspace_v2(),
        remote_source: crate::nisar_science_transfer_v2::NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
            asset.effective_url(), &work, &completed, physical,
        ).unwrap(), anchor: &anchor,
    };
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();
    for _ in 0..count - last_attempt_requests {
        assert_eq!(
            executor.execute(&work, &completed, &authority, &WorkerControl::default()),
            Err(Error::RangeUnsupported)
        );
    }
    let mut refresh_calls = 0;
    let mut refresh = |_: &ResolvedAsset, _: &WorkerControl| {
        refresh_calls += 1;
        Ok(science_asset(
            &server.url_root,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        ))
    };
    assert_eq!(
        executor.execute_with_progress_and_auth_refresh(
            &work,
            &completed,
            &authority,
            &WorkerControl::default(),
            &mut |_| {},
            &mut refresh,
        ),
        Err(Error::CombinedBudgetExceeded)
    );
    assert_eq!(
        refresh_calls,
        usize::from(retry_redirect),
        "authentication must respect the shared durable allowance"
    );
    let checkpoint: Value = serde_json::from_str(
        &fs::read_to_string(
            bootstrap
                .science_workspace_v2()
                .science_checkpoint_path_v2(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(checkpoint["reserved_science_request_count"], json!(count));
    assert_eq!(checkpoint["issued_science_request_count"], json!(count));
    assert!(checkpoint["pending_attempt"].is_null());
    // Re-entry sees valid exhausted evidence, not RecoveryInvalid.
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::CombinedBudgetExceeded)
    );
    assert_eq!(server.finish().len(), count as usize);
}

#[test]
fn science_range_v2_zero_hop_pause_settles_one_charge_and_reopens() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };
    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-zero-hop-pause",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let control = WorkerControl::default();
    *anchor.pause_after_reservation.lock().unwrap() = Some(control.clone());
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();
    assert_eq!(
        executor.execute(&work, &completed, &authority, &control),
        Err(Error::Paused)
    );
    let checkpoint: Value = serde_json::from_str(
        &fs::read_to_string(
            bootstrap
                .science_workspace_v2()
                .science_checkpoint_path_v2(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(checkpoint["reserved_science_request_count"], json!(1));
    assert_eq!(checkpoint["unissued_science_reservation_count"], json!(1));
    assert_eq!(checkpoint["issued_science_request_count"], json!(0));
    assert_eq!(checkpoint["failed_science_request_count"], json!(0));
    assert!(checkpoint["pending_attempt"].is_null());
    let resumed = executor
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    assert!(resumed.complete());
    assert_eq!(
        resumed.accounting().reserved_science_request_count(),
        resumed.accounting().science_request_count() + 1
    );
    assert_eq!(
        resumed.accounting().charged_science_response_body_bytes(),
        physical
            .estimated_execution_transfer_response_bytes()
            .unwrap()
            + physical.execution_transfer_ranges().unwrap()[0]
                .len()
                .unwrap()
    );
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );
}

#[test]
fn science_range_v2_genesis_anchor_prepare_recovers_missing_local_checkpoint() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-genesis-anchor-recovery",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let stable_url = server.url_root.clone();
    let asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();

    // Model a successful external genesis prepare whose acknowledgement is lost
    // before the local checkpoint is materialized.
    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Genesis);
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::Persistence)
    );
    assert!(!bootstrap
        .science_workspace_v2()
        .science_checkpoint_path_v2()
        .exists());

    let result = executor
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    assert!(result.complete());
    assert_eq!(
        result.science_request_count(),
        physical.execution_transfer_ranges().unwrap().len() as u64
    );
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );
}

#[test]
fn science_range_v2_reservation_prepare_recovers_as_indeterminate_charge() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-reservation-anchor-recovery",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let first_range_bytes = physical.execution_transfer_ranges().unwrap()[0]
        .len()
        .unwrap();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let stable_url = server.url_root.clone();
    let asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();

    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Reservation);
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::Persistence)
    );
    let result = executor
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    let accounting = result.accounting();
    assert!(result.complete());
    let reserved_allowance = physical
        .budget_projection()
        .max_science_request_count()
        .min(2 * crate::http_transfer::MAX_REQUESTS_PER_TRUSTED_REDIRECT_SEND);
    assert_eq!(
        accounting.indeterminate_science_request_count(),
        reserved_allowance
    );
    assert_eq!(
        accounting.indeterminate_science_response_body_bytes(),
        first_range_bytes
    );
    assert_eq!(
        accounting.charged_science_response_body_bytes(),
        physical
            .estimated_execution_transfer_response_bytes()
            .unwrap()
            + first_range_bytes
    );
    assert_eq!(
        accounting.observed_science_response_body_bytes(),
        physical
            .estimated_execution_transfer_response_bytes()
            .unwrap()
    );
    assert_eq!(
        accounting.combined_request_count(),
        accounting.planning_request_count()
            + physical.execution_transfer_ranges().unwrap().len() as u64
            + reserved_allowance
    );
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );
}

/// A science transfer completed after a killed process left its first Range
/// attempt Reserved (the reservation was anchored, the request outcome is
/// unknown); recovery charged that attempt as indeterminate.
fn completed_science_with_indeterminate_charge(
    label: &str,
) -> (
    ReadyCase,
    DownloadWork,
    NisarSubsetPlanningCompleteV2,
    Arc<NisarSubsetLocalHdf5MetadataBootstrapV2>,
    ScienceCheckpointAnchorFixture,
    crate::nisar_science_transfer_v2::NisarSubsetScienceTransferResultV2,
    String,
) {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };
    let (case, intent, bootstrap, source_path) =
        concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let url_root = server.url_root.clone();
    let asset = science_asset(
        &url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();
    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Reservation);
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::Persistence)
    );
    let science = executor
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    assert!(science.complete());
    assert!(science.accounting().indeterminate_science_request_count() > 0);
    assert!(
        science
            .accounting()
            .indeterminate_science_response_body_bytes()
            > 0
    );
    // The Reserved attempt sent nothing here: every Range was fetched once.
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );
    drop(authority);
    (case, work, completed, bootstrap, anchor, science, url_root)
}

/// The writer's science authority for the same frozen source URL.
fn same_source_asset(url_root: &str, completed: &NisarSubsetPlanningCompleteV2) -> ResolvedAsset {
    let physical = completed.physical_execution_plan();
    science_asset(
        url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    )
}

/// Owner decision 2026-10-17 (option B, replacing decision A): when every
/// completed Range passes its response digest and material revalidation, the
/// writer accepts an indeterminate charge as an accounting fact and the artifact
/// manifest records the charge bounds (lower: committed + failed requests and
/// observed bytes; upper: plus the indeterminate reservation). A determinate
/// write keeps its manifest without that block.
#[test]
fn writer_v2_accepts_a_verified_indeterminate_charge_and_records_its_bounds() {
    let (case, work, completed, bootstrap, anchor, science, url_root) =
        completed_science_with_indeterminate_charge("writer-v2-indeterminate-charge");
    let asset = same_source_asset(&url_root, &completed);
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let output_root = case._root.0.join("writer-v2-indeterminate-output");
    let written = crate::nisar_subset_writer::tests::writer(&output_root)
        .write_and_validate_v2(
            &work,
            &completed,
            &science,
            &authority,
            &WorkerControl::default(),
        )
        .expect("verified material makes the indeterminate charge an accounting fact");
    assert!(written.manifest().complete());
    assert!(hdf5::File::open(written.artifact_path()).is_ok());
    let accounting = science.accounting();
    let indeterminate_requests = accounting.indeterminate_science_request_count();
    let indeterminate_bytes = accounting.indeterminate_science_response_body_bytes();
    let lower_requests =
        accounting.committed_science_request_count() + accounting.failed_science_request_count();
    let lower_bytes = accounting.observed_science_response_body_bytes();
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(written.manifest_path()).unwrap()).unwrap();
    assert_eq!(
        manifest["indeterminate_science_charge"],
        json!({
            "indeterminate_science_request_count": indeterminate_requests,
            "indeterminate_science_response_body_bytes": indeterminate_bytes,
            "science_request_count_lower_bound": lower_requests,
            "science_request_count_upper_bound": lower_requests + indeterminate_requests,
            "science_response_body_bytes_lower_bound": lower_bytes,
            "science_response_body_bytes_upper_bound": lower_bytes + indeterminate_bytes,
        })
    );

    let (determinate_case, determinate_intent, determinate_bootstrap, determinate_source) =
        concrete_fixture(
            "writer-v2-determinate-manifest",
            NisarSubsetFrequency::A,
            &["HH"],
        );
    let determinate = execute_v2_writer_fixture(
        &determinate_case,
        &determinate_intent,
        &determinate_bootstrap,
        &determinate_source,
        "writer-v2-determinate-output",
    );
    let text = fs::read_to_string(determinate.manifest_path()).unwrap();
    assert!(
        !text.contains("indeterminate_science_charge"),
        "a determinate write keeps its manifest unchanged: {text}"
    );
}

/// Owner decision 2026-10-17 (option B) accepts an indeterminate charge only
/// with fully verified material: one corrupted byte of a completed Range is
/// still refused by the material revalidation and nothing is published.
#[test]
fn writer_v2_refuses_an_indeterminate_charge_when_completed_material_mismatches() {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as Error;
    use crate::nisar_subset_writer::NisarSubsetWriteError as WriteError;
    let (case, work, completed, bootstrap, anchor, science, url_root) =
        completed_science_with_indeterminate_charge("writer-v2-indeterminate-corrupt");
    let offset = completed
        .physical_execution_plan()
        .execution_transfer_ranges()
        .unwrap()[0]
        .start();
    let part_path = bootstrap.science_workspace_v2().part_path();
    let mut part = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&part_path)
        .unwrap();
    let mut byte = [0_u8; 1];
    part.seek(SeekFrom::Start(offset)).unwrap();
    part.read_exact(&mut byte).unwrap();
    part.seek(SeekFrom::Start(offset)).unwrap();
    part.write_all(&[!byte[0]]).unwrap();
    part.sync_all().unwrap();
    drop(part);
    let asset = same_source_asset(&url_root, &completed);
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    assert!(matches!(
        crate::nisar_science_transfer_v2::revalidate_complete_science_for_writer_v2(
            &work,
            &completed,
            &science,
            &authority,
            &WorkerControl::default(),
        ),
        Err(Error::RecoveryInvalid)
    ));
    let output_root = case._root.0.join("writer-v2-indeterminate-corrupt-output");
    assert_eq!(
        crate::nisar_subset_writer::tests::writer(&output_root)
            .write_and_validate_v2(
                &work,
                &completed,
                &science,
                &authority,
                &WorkerControl::default(),
            )
            .unwrap_err(),
        WriteError::IncompleteScienceAuthority
    );
    assert!(writer_v2_candidate_paths(&output_root).is_empty());
    assert_eq!(
        fs::read_dir(&output_root)
            .map(|entries| entries
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5")
                )
                .count())
            .unwrap_or(0),
        0
    );
}

#[test]
fn science_range_v2_failed_outcome_prepare_recovers_without_refund_or_duplicate_cas() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-failed-outcome-anchor-recovery",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let first_range_bytes = physical.execution_transfer_ranges().unwrap()[0]
        .len()
        .unwrap();
    let mut responses = vec![science_response(
        "200 OK",
        &[("Content-Length", physical.source_size().to_string())],
        &[],
    )];
    responses.extend(exact_science_responses(&source_path, physical, 0));
    let server = ScienceServer::new(responses);
    let stable_url = server.url_root.clone();
    let asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();

    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::FailedOutcome);
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::Persistence)
    );
    let result = executor
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    let accounting = result.accounting();
    assert!(result.complete());
    assert_eq!(accounting.failed_science_request_count(), 1);
    assert_eq!(accounting.indeterminate_science_request_count(), 0);
    assert_eq!(
        accounting.charged_science_response_body_bytes(),
        physical
            .estimated_execution_transfer_response_bytes()
            .unwrap()
            + first_range_bytes
    );
    assert_eq!(
        accounting.observed_science_response_body_bytes(),
        physical
            .estimated_execution_transfer_response_bytes()
            .unwrap()
    );
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len() + 1
    );
}

#[test]
fn science_range_v2_response_recorded_multi_request_recovers_actual_charge() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };
    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-response-recorded-multi-request",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let ranges = physical.execution_transfer_ranges().unwrap();
    let first_bytes = ranges[0].len().unwrap();
    let mut responses = vec![
        science_response(
            "401 Unauthorized",
            &[("Content-Length", "0".to_owned())],
            &[],
        ),
        exact_science_response(
            &ranges[0],
            physical.source_size(),
            &physical.source_validator().value,
            &source_range_bytes(&source_path, &ranges[0]),
        ),
    ];
    responses.extend(exact_science_responses(&source_path, physical, 0));
    let server = ScienceServer::new(responses);
    let asset = science_asset_with_bearer(
        &server.url_root,
        physical,
        &physical.source_validator().value,
        "fixture-old-bearer",
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();
    let mut refresh = |_: &ResolvedAsset, _: &WorkerControl| {
        Ok(science_asset_with_bearer(
            &server.url_root,
            physical,
            &physical.source_validator().value,
            "fixture-new-bearer",
        ))
    };
    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::ResponseRecorded);
    assert_eq!(
        executor.execute_with_progress_and_auth_refresh(
            &work,
            &completed,
            &authority,
            &WorkerControl::default(),
            &mut |_| {},
            &mut refresh,
        ),
        Err(Error::Persistence)
    );
    let resumed = executor
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    assert!(resumed.complete());
    assert_eq!(resumed.accounting().failed_science_request_count(), 2);
    assert_eq!(
        resumed.accounting().indeterminate_science_request_count(),
        0
    );
    assert_eq!(
        resumed.accounting().reserved_science_request_count(),
        ranges.len() as u64 + 2
    );
    assert_eq!(
        resumed.accounting().charged_science_response_body_bytes(),
        physical
            .estimated_execution_transfer_response_bytes()
            .unwrap()
            + first_bytes
    );
    assert_eq!(server.finish().len(), ranges.len() + 2);
}

#[test]
fn science_range_v2_completion_prepare_after_part_sync_recovers_without_refetch() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-completion-anchor-recovery",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let stable_url = server.url_root.clone();
    let asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();

    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Complete);
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::Persistence)
    );
    let result = executor
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    assert!(result.complete());
    assert_eq!(
        result.science_request_count(),
        physical.execution_transfer_ranges().unwrap().len() as u64
    );
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len()
    );
}

#[test]
fn science_range_v2_corrupt_pending_payload_fails_before_any_get() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };

    let (case, intent, bootstrap, _) = concrete_fixture(
        "science-v2-corrupt-pending-anchor",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let asset = science_asset(
        "http://127.0.0.1:9",
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();

    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Reservation);
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::Persistence)
    );
    anchor.corrupt_pending_payload_without_reseal();
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::RecoveryInvalid)
    );
}

#[test]
fn science_range_v2_coherent_pending_predecessor_mismatch_fails_before_local_commit() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };

    let (case, intent, bootstrap, _) = concrete_fixture(
        "science-v2-pending-predecessor-mismatch",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let asset = science_asset(
        "http://127.0.0.1:9",
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();
    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Reservation);
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::Persistence)
    );
    let checkpoint_path = bootstrap
        .science_workspace_v2()
        .science_checkpoint_path_v2();
    fs::remove_file(&checkpoint_path).unwrap();
    anchor.coherently_mismatch_pending_predecessor();

    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::RecoveryInvalid)
    );
    assert!(!checkpoint_path.exists());
    assert!(anchor.has_pending());
}

#[test]
fn science_range_v2_coherent_max_generation_pending_fails_before_local_commit() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };

    let (case, intent, bootstrap, _) = concrete_fixture(
        "science-v2-pending-generation-overflow",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let asset = science_asset(
        "http://127.0.0.1:9",
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();
    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Reservation);
    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::Persistence)
    );
    let checkpoint_path = bootstrap
        .science_workspace_v2()
        .science_checkpoint_path_v2();
    let predecessor_local = fs::read_to_string(&checkpoint_path).unwrap();
    anchor.coherently_set_pending_generation_max();

    assert_eq!(
        executor.execute(&work, &completed, &authority, &WorkerControl::default()),
        Err(Error::RecoveryInvalid)
    );
    assert_eq!(
        fs::read_to_string(&checkpoint_path).unwrap(),
        predecessor_local
    );
    assert!(anchor.has_pending());
}

#[test]
fn science_range_v2_stable_lineage_rejects_url_and_workspace_replacement() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceRemoteSourceV2,
        NisarSubsetScienceTransferErrorV2 as Error,
    };

    let (case, intent, bootstrap, _) = concrete_fixture(
        "science-v2-stable-anchor-lineage",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let original_asset = science_asset(
        "http://127.0.0.1:9",
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let original_authority = science_authority(
        bootstrap.as_ref(),
        &original_asset,
        &work,
        &completed,
        &anchor,
    );
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), original_asset).unwrap();
    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Reservation);
    assert_eq!(
        executor.execute(
            &work,
            &completed,
            &original_authority,
            &WorkerControl::default(),
        ),
        Err(Error::Persistence)
    );
    let (stable_key, original_execution_binding) = {
        let state = anchor.state.lock().unwrap();
        let (key, record) = state.as_ref().unwrap();
        (key.clone(), record.execution_binding_sha256().to_owned())
    };

    let replacement_url_asset = science_asset(
        "http://127.0.0.1:10",
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let replacement_url_authority = science_authority(
        bootstrap.as_ref(),
        &replacement_url_asset,
        &work,
        &completed,
        &anchor,
    );
    let replacement_url_executor =
        NisarSubsetScienceRangeExecutorV2::new(science_http(), replacement_url_asset).unwrap();
    assert_eq!(
        replacement_url_executor.execute(
            &work,
            &completed,
            &replacement_url_authority,
            &WorkerControl::default(),
        ),
        Err(Error::AuthorityMismatch)
    );

    let replacement_workspace =
        Hdf5RangeWorkspaceV1::new(case._root.0.join("replacement-science-workspace")).unwrap();
    drop(
        replacement_workspace
            .reset_planning_part_v2(physical.source_size())
            .unwrap(),
    );
    let original_url_asset = science_asset(
        "http://127.0.0.1:9",
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let replacement_workspace_authority = StaticScienceAuthorityFixture {
        plan: physical.clone(),
        statistics: completed.bootstrap_statistics().clone(),
        workspace: &replacement_workspace,
        remote_source: NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
            original_url_asset.effective_url(),
            &work,
            &completed,
            physical,
        )
        .unwrap(),
        anchor: &anchor,
    };
    let replacement_workspace_executor =
        NisarSubsetScienceRangeExecutorV2::new(science_http(), original_url_asset).unwrap();
    assert_eq!(
        replacement_workspace_executor.execute(
            &work,
            &completed,
            &replacement_workspace_authority,
            &WorkerControl::default(),
        ),
        Err(Error::AuthorityMismatch)
    );

    let state = anchor.state.lock().unwrap();
    let (key, record) = state.as_ref().unwrap();
    assert_eq!(key, &stable_key);
    assert_eq!(record.lineage_key_sha256(), stable_key);
    assert_eq!(
        record.execution_binding_sha256(),
        original_execution_binding
    );
}

#[test]
fn science_asset_identity_accepts_only_exact_bhoonidhi_broker_identity() {
    use crate::asset_resolver::{ResolvedHostPolicy, BHOONIDHI_DOWNLOAD_URL};
    use crate::nisar_science_transfer_v2::resolved_asset_identity_matches;

    let (_case, intent, bootstrap, _) = concrete_fixture(
        "science-v2-bhoonidhi-broker-identity",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let work = work(&_case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    let mut physical = completed.physical_execution_plan().clone();
    physical.source_product_identity.provider_id =
        insar_provider::bhoonidhi::BHOONIDHI_PROVIDER_ID.to_owned();
    physical.source_product_identity.collection_id =
        insar_provider::bhoonidhi::BHOONIDHI_COLLECTION_ID.to_owned();
    physical.source_product_identity.provider_product_id = "ITEM_123".to_owned();
    physical.source_asset_identity.file_name = "source-scene.h5".to_owned();

    let exact_url = format!(
        "{BHOONIDHI_DOWNLOAD_URL}?id={}&collection={}",
        physical.source_product_identity.provider_product_id,
        physical.source_product_identity.collection_id
    );
    let exact = ResolvedAsset::new(&exact_url, ResolvedMethod::Get)
        .unwrap()
        .with_host_policy(ResolvedHostPolicy::BhoonidhiDownloadV1);
    assert!(resolved_asset_identity_matches(&exact, &physical));

    let wrong_id = ResolvedAsset::new(
        &format!(
            "{BHOONIDHI_DOWNLOAD_URL}?id=OTHER_ITEM&collection={}",
            physical.source_product_identity.collection_id
        ),
        ResolvedMethod::Get,
    )
    .unwrap()
    .with_host_policy(ResolvedHostPolicy::BhoonidhiDownloadV1);
    assert!(!resolved_asset_identity_matches(&wrong_id, &physical));

    let mut wrong_provider = physical.clone();
    wrong_provider.source_product_identity.provider_id = "nasa.asf".to_owned();
    assert!(!resolved_asset_identity_matches(&exact, &wrong_provider));
}

#[test]
fn science_range_v2_pause_cancel_prepare_recovers_pending_interruption() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceRemoteSourceV2,
        NisarSubsetScienceTransferErrorV2 as Error,
    };

    for (label, cancel) in [("pause", false), ("cancel", true)] {
        let (case, intent, bootstrap, source_path) = concrete_fixture(
            &format!("science-v2-{label}-anchor-recovery"),
            NisarSubsetFrequency::A,
            &["HH"],
        );
        let mut work = work(&case, &intent);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan())
            .unwrap();
        let physical = completed.physical_execution_plan();
        let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
        let stable_url = server.url_root.clone();
        let asset = science_asset(
            &stable_url,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        );
        let anchor = ScienceCheckpointAnchorFixture::default();
        let authority = StaticScienceAuthorityFixture {
            plan: physical.clone(),
            statistics: completed.bootstrap_statistics().clone(),
            workspace: bootstrap.science_workspace_v2(),
            remote_source: NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
                asset.effective_url(),
                &work,
                &completed,
                physical,
            )
            .unwrap(),
            anchor: &anchor,
        };
        let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();
        let stopped = WorkerControl::default();
        if cancel {
            stopped.cancel_for_test();
        } else {
            stopped.set_paused_for_test(true);
        }
        anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Interrupted);
        assert_eq!(
            executor.execute(&work, &completed, &authority, &stopped),
            Err(Error::Persistence),
            "{label} prepare"
        );
        let result = executor
            .execute(&work, &completed, &authority, &WorkerControl::default())
            .unwrap();
        assert!(result.complete());
        assert_eq!(
            server.finish().len(),
            physical.execution_transfer_ranges().unwrap().len()
        );
    }
}

#[test]
fn science_range_v2_hard_mid_body_loss_recovers_as_indeterminate_upper_bound() {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2;

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-hard-mid-body-loss",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let first_range_bytes = physical.execution_transfer_ranges().unwrap()[0]
        .len()
        .unwrap();
    let exact = exact_science_responses(&source_path, physical, 0);
    let mut responses = vec![exact[0].clone()];
    responses.extend(exact);
    let server = ScienceServer::new(responses);
    let stable_url = server.url_root.clone();
    let authority_asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(
        bootstrap.as_ref(),
        &authority_asset,
        &work,
        &completed,
        &anchor,
    );
    let activity = Arc::new(crate::TaskTransferActivityRegistry::default());
    let binding = crate::transfer_activity::TaskBodyActivity::new(
        Arc::clone(&activity),
        work.task.task_id.clone(),
        work.item.item_id.clone(),
    );
    let crashing = NisarSubsetScienceRangeExecutorV2::new(science_http(), authority_asset)
        .unwrap()
        .with_body_activity(binding.clone())
        .with_after_body_read_hook_for_test(
            1,
            Arc::new(|observed| {
                assert_eq!(observed, 1);
                panic!("simulated hard process loss after a partial body read");
            }),
        );
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = crashing.execute(&work, &completed, &authority, &WorkerControl::default());
    }));
    assert!(crashed.is_err());
    let interrupted_sample = activity.sample(&work.task.task_id, Instant::now()).unwrap();
    assert_eq!(
        interrupted_sample.network_bytes_received, 1,
        "even a read followed by panic must be counted exactly once"
    );
    assert_eq!(
        interrupted_sample.active_count, 0,
        "unwinding must stop body activity"
    );
    thread::sleep(Duration::from_millis(10));
    assert_eq!(
        activity.sample(&work.task.task_id, Instant::now()).unwrap(),
        interrupted_sample
    );

    let restarted = NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        science_asset(
            &stable_url,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        ),
    )
    .unwrap();
    let result = restarted
        .with_body_activity(binding)
        .execute(&work, &completed, &authority, &WorkerControl::default())
        .unwrap();
    let accounting = result.accounting();
    assert!(result.complete());
    let final_sample = activity.sample(&work.task.task_id, Instant::now()).unwrap();
    assert_eq!(final_sample.active_count, 0);
    assert_eq!(
        final_sample.network_bytes_received,
        physical
            .estimated_execution_transfer_response_bytes()
            .unwrap()
            + 1,
        "network telemetry must use actual partial reads, not charged upper bounds"
    );
    assert_eq!(
        accounting.indeterminate_science_request_count(),
        physical
            .budget_projection()
            .max_science_request_count()
            .min(2 * crate::http_transfer::MAX_REQUESTS_PER_TRUSTED_REDIRECT_SEND)
    );
    assert_eq!(
        accounting.indeterminate_science_response_body_bytes(),
        first_range_bytes
    );
    assert_eq!(
        accounting.observed_science_response_body_bytes(),
        physical
            .estimated_execution_transfer_response_bytes()
            .unwrap()
    );
    assert_eq!(
        accounting.charged_science_response_body_bytes(),
        physical
            .estimated_execution_transfer_response_bytes()
            .unwrap()
            + first_range_bytes
    );
    assert_eq!(
        server.finish().len(),
        physical.execution_transfer_ranges().unwrap().len() + 1
    );
}

#[test]
fn science_range_v2_refreshes_one_rejected_bearer_without_changing_range_authority() {
    let (case, intent, bootstrap, source_path) =
        concrete_fixture("science-v2-auth-refresh", NisarSubsetFrequency::A, &["HH"]);
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let mut responses = vec![science_response(
        "401 Unauthorized",
        &[("Content-Length", "0".to_owned())],
        &[],
    )];
    responses.extend(exact_science_responses(&source_path, physical, 0));
    let server = ScienceServer::new(responses);
    let stable_url = server.url_root.clone();
    let strong_etag = physical.source_validator().value.clone();
    let initial_asset =
        science_asset_with_bearer(&stable_url, physical, &strong_etag, "fixture-old-bearer");
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(
        bootstrap.as_ref(),
        &initial_asset,
        &work,
        &completed,
        &anchor,
    );
    let executor = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        initial_asset,
    )
    .unwrap();
    let refresh_calls = Arc::new(AtomicUsize::new(0));
    let refresh_counter = Arc::clone(&refresh_calls);
    let mut refresh = |_rejected: &ResolvedAsset, _control: &WorkerControl| {
        refresh_counter.fetch_add(1, Ordering::SeqCst);
        Ok(science_asset_with_bearer(
            &stable_url,
            physical,
            &strong_etag,
            "fixture-new-bearer",
        ))
    };
    let result = executor
        .execute_with_progress_and_auth_refresh(
            &work,
            &completed,
            &authority,
            &WorkerControl::default(),
            &mut |_| {},
            &mut refresh,
        )
        .unwrap();
    assert!(result.complete());
    assert_eq!(refresh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        result.science_request_count(),
        physical.execution_transfer_ranges().unwrap().len() as u64 + 1
    );

    let requests = server.finish();
    assert!(requests.len() >= 2);
    assert_eq!(
        science_request_header(&requests[0], "Range"),
        science_request_header(&requests[1], "Range"),
        "the rejected Range must be retried verbatim after auth refresh"
    );
    assert_eq!(
        science_request_header(&requests[0], "Authorization"),
        Some("Bearer fixture-old-bearer")
    );
    assert_eq!(
        science_request_header(&requests[1], "Authorization"),
        Some("Bearer fixture-new-bearer")
    );
}

#[test]
fn science_range_v2_restart_keeps_completed_ranges_and_refreshes_expired_bearer() {
    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-resume-auth-refresh",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let ranges = physical.execution_transfer_ranges().unwrap();
    assert!(ranges.len() > 1);

    let strong_etag = physical.source_validator().value.clone();
    let mut responses = vec![exact_science_response(
        &ranges[0],
        physical.source_size(),
        &strong_etag,
        &source_range_bytes(&source_path, &ranges[0]),
    )];
    responses.push(science_response(
        "401 Unauthorized",
        &[("Content-Length", "0".to_owned())],
        &[],
    ));
    responses.extend(exact_science_responses(&source_path, physical, 1));
    let server = ScienceServer::new(responses);
    let stable_url = server.url_root.clone();
    let old_asset = science_asset_with_bearer(
        &stable_url,
        physical,
        &strong_etag,
        "fixture-expired-bearer",
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &old_asset, &work, &completed, &anchor);

    let first_control = WorkerControl::default();
    let first_hook_control = first_control.clone();
    let first_executor = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        old_asset,
    )
    .unwrap()
    .with_after_commit_hook_for_test(Arc::new(move |index| {
        if index == 0 {
            first_hook_control.set_paused_for_test(true);
        }
    }));
    assert_eq!(
        first_executor
            .execute(&work, &completed, &authority, &first_control)
            .unwrap_err(),
        crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::Paused
    );

    let resumed_asset = science_asset_with_bearer(
        &stable_url,
        physical,
        &strong_etag,
        "fixture-expired-bearer",
    );
    let resumed_executor =
        crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
            science_http(),
            resumed_asset,
        )
        .unwrap();
    let refresh_calls = Arc::new(AtomicUsize::new(0));
    let refresh_counter = Arc::clone(&refresh_calls);
    let mut refresh = |_rejected: &ResolvedAsset, _control: &WorkerControl| {
        refresh_counter.fetch_add(1, Ordering::SeqCst);
        Ok(science_asset_with_bearer(
            &stable_url,
            physical,
            &strong_etag,
            "fixture-refreshed-bearer",
        ))
    };
    let resumed = resumed_executor
        .execute_with_progress_and_auth_refresh(
            &work,
            &completed,
            &authority,
            &WorkerControl::default(),
            &mut |_| {},
            &mut refresh,
        )
        .unwrap();
    assert!(resumed.complete());
    assert_eq!(refresh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        resumed.accounting().resumed_bytes(),
        ranges[0].len().unwrap()
    );
    assert_eq!(
        resumed.science_request_count(),
        ranges.len() as u64 + 1,
        "one rejected auth request is accounted in addition to one successful request per range"
    );

    let requests = server.finish();
    assert_eq!(requests.len(), ranges.len() + 1);
    let first_range = science_request_header(&requests[0], "Range").unwrap();
    let resumed_range = science_request_header(&requests[1], "Range").unwrap();
    let retried_range = science_request_header(&requests[2], "Range").unwrap();
    assert_ne!(
        first_range, resumed_range,
        "completed Range 0 must not be fetched again"
    );
    assert_eq!(
        resumed_range, retried_range,
        "only the rejected resumed Range is retried"
    );
    assert_eq!(
        science_request_header(&requests[1], "Authorization"),
        Some("Bearer fixture-expired-bearer")
    );
    assert_eq!(
        science_request_header(&requests[2], "Authorization"),
        Some("Bearer fixture-refreshed-bearer")
    );
}

#[cfg(windows)]
#[test]
fn archived_recovery_forks_verified_ranges_preserves_failed_budget_and_source() {
    use crate::nisar_science_transfer_v2::{
        finalize_archived_nisar_recovery_v2, prepare_archived_nisar_recovery_v2,
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceRemoteSourceV2,
    };
    let (case, intent, bootstrap, source_path) =
        concrete_fixture("archived-recovery-fork", NisarSubsetFrequency::A, &["HH"]);
    let mut source_work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &source_work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut source_work.item.metadata, completed.frozen_plan())
        .unwrap();
    let physical = completed.physical_execution_plan();
    let ranges = physical.execution_transfer_ranges().unwrap();
    assert!(ranges.len() > 1);
    let responses = vec![
        exact_science_response(
            &ranges[0],
            physical.source_size(),
            &physical.source_validator().value,
            &source_range_bytes(&source_path, &ranges[0]),
        ),
        science_response(
            "401 Unauthorized",
            &[("Content-Length", "0".to_owned())],
            &[],
        ),
    ];
    let server = ScienceServer::new(responses);
    let asset = science_asset(
        &server.url_root,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let source_anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(
        bootstrap.as_ref(),
        &asset,
        &source_work,
        &completed,
        &source_anchor,
    );
    assert!(
        NisarSubsetScienceRangeExecutorV2::new(science_http(), asset)
            .unwrap()
            .execute(
                &source_work,
                &completed,
                &authority,
                &WorkerControl::default()
            )
            .is_err()
    );
    assert_eq!(server.finish().len(), 2);
    let source_workspace = bootstrap.science_workspace_v2();
    let old_science = source_workspace
        .load_science_checkpoint_v2()
        .unwrap()
        .unwrap();
    let old_planning = source_workspace
        .load_planning_checkpoint_v2()
        .unwrap()
        .unwrap();
    let before: Value = serde_json::from_str(&old_science).unwrap();
    assert_eq!(before["completed_ranges"].as_array().unwrap().len(), 1);
    assert!(before["failed_science_request_count"].as_u64().unwrap() > 0);
    assert!(
        before["reserved_science_response_body_bytes"]
            .as_u64()
            .unwrap()
            > before["committed_science_response_body_bytes"]
                .as_u64()
                .unwrap(),
        "the failed request's reserved bytes remain charged"
    );
    let old_part = fs::read(source_workspace.part_path()).unwrap();
    let record = source_anchor
        .state
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .1
        .clone();
    let anchor_json = record.canonical_json().unwrap();
    let mut new_work = source_work.clone();
    new_work.task.task_id = "explicit-archived-recovery".to_owned();
    new_work.task.output_root = case._root.0.join("recovered-output");
    let destination = Hdf5RangeWorkspaceV1::new(case._root.0.join("recovery-workspace")).unwrap();
    assert!(prepare_archived_nisar_recovery_v2(
        &source_work,
        &new_work,
        &anchor_json,
        &"0".repeat(64),
        source_workspace,
        &destination,
        &WorkerControl::default()
    )
    .is_err());
    assert!(
        !destination.part_path().exists(),
        "bad authority cannot create material"
    );
    let offset = ranges[0].start();
    let original_byte = old_part[offset as usize];
    {
        let mut part = OpenOptions::new()
            .write(true)
            .open(source_workspace.part_path())
            .unwrap();
        part.seek(SeekFrom::Start(offset)).unwrap();
        part.write_all(&[original_byte ^ 1]).unwrap();
        part.sync_all().unwrap();
    }
    assert!(
        prepare_archived_nisar_recovery_v2(
            &source_work,
            &new_work,
            &anchor_json,
            record.record_sha256(),
            source_workspace,
            &destination,
            &WorkerControl::default(),
        )
        .is_err(),
        "a valid Store anchor cannot excuse corrupted source material"
    );
    assert!(!destination.part_path().exists());
    {
        let mut part = OpenOptions::new()
            .write(true)
            .open(source_workspace.part_path())
            .unwrap();
        part.seek(SeekFrom::Start(offset)).unwrap();
        part.write_all(&[original_byte]).unwrap();
        part.sync_all().unwrap();
    }
    let token = prepare_archived_nisar_recovery_v2(
        &source_work,
        &new_work,
        &anchor_json,
        record.record_sha256(),
        source_workspace,
        &destination,
        &WorkerControl::default(),
    )
    .expect("archived committed ranges must import into a new lineage");
    assert!(
        std::fs::OpenOptions::new()
            .write(true)
            .open(source_workspace.part_path())
            .is_err(),
        "source material remains deny-write pinned throughout import"
    );
    drop(token); // Crash after planning commit, before science genesis.
    let copied_part = fs::read(destination.part_path()).unwrap();
    let mut unrelated = new_work.clone();
    unrelated.task.task_id = "different-recovery-task".to_owned();
    assert!(prepare_archived_nisar_recovery_v2(
        &source_work,
        &unrelated,
        &anchor_json,
        record.record_sha256(),
        source_workspace,
        &destination,
        &WorkerControl::default()
    )
    .is_err());
    assert_eq!(
        fs::read(destination.part_path()).unwrap(),
        copied_part,
        "another task cannot claim or overwrite a partial import"
    );
    let token = prepare_archived_nisar_recovery_v2(
        &source_work,
        &new_work,
        &anchor_json,
        record.record_sha256(),
        source_workspace,
        &destination,
        &WorkerControl::default(),
    )
    .unwrap();
    let next_bootstrap = NisarSubsetLocalHdf5MetadataBootstrapV2::new_for_test(
        source_path.clone(),
        physical.source_size(),
        destination.clone(),
    )
    .unwrap();
    let next = plan_and_freeze_nisar_subset_v2_with(
        &new_work,
        &WorkerControl::default(),
        &next_bootstrap,
        |_, _| panic!("frozen final plan must not be replanned"),
    )
    .unwrap();
    let remaining = ScienceServer::new(exact_science_responses(&source_path, physical, 1));
    let next_asset = science_asset(
        &remaining.url_root,
        next.physical_execution_plan(),
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let remote = NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
        next_asset.effective_url(),
        &new_work,
        &next,
        next.physical_execution_plan(),
    )
    .unwrap();
    let next_anchor = ScienceCheckpointAnchorFixture::default();
    next_anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::FailedOutcome);
    assert!(finalize_archived_nisar_recovery_v2(
        token,
        &new_work,
        &next,
        &remote,
        &next_anchor,
        &destination,
        &WorkerControl::default(),
    )
    .is_err());
    assert!(
        next_anchor.has_pending(),
        "crash persists prepared imported genesis"
    );
    assert!(destination.load_science_checkpoint_v2().unwrap().is_none());
    let token = prepare_archived_nisar_recovery_v2(
        &source_work,
        &new_work,
        &anchor_json,
        record.record_sha256(),
        source_workspace,
        &destination,
        &WorkerControl::default(),
    )
    .unwrap();
    finalize_archived_nisar_recovery_v2(
        token,
        &new_work,
        &next,
        &remote,
        &next_anchor,
        &destination,
        &WorkerControl::default(),
    )
    .unwrap();
    let imported: Value =
        serde_json::from_str(&destination.load_science_checkpoint_v2().unwrap().unwrap()).unwrap();
    for key in [
        "reserved_science_request_count",
        "reserved_science_response_body_bytes",
        "issued_science_request_count",
        "observed_science_response_body_bytes",
        "failed_science_request_count",
        "unissued_science_reservation_count",
        "indeterminate_science_request_count",
        "indeterminate_science_response_body_bytes",
        "committed_planning_request_count",
        "committed_planning_response_body_bytes",
        "committed_science_request_count",
        "committed_science_response_body_bytes",
    ] {
        assert_eq!(
            imported[key], before[key],
            "budget field {key} must be inherited"
        );
    }
    assert_eq!(imported["anchor_generation"], json!(0));
    assert_ne!(imported["lineage_key_sha256"], before["lineage_key_sha256"]);
    assert_ne!(
        imported["part_identity_sha256"],
        before["part_identity_sha256"]
    );
    let next_authority =
        science_authority(&next_bootstrap, &next_asset, &new_work, &next, &next_anchor);
    let result = NisarSubsetScienceRangeExecutorV2::new(science_http(), next_asset)
        .unwrap()
        .execute(&new_work, &next, &next_authority, &WorkerControl::default())
        .unwrap();
    assert!(result.complete());
    assert_eq!(
        remaining.finish().len(),
        ranges.len() - 1,
        "verified first range is reused"
    );
    let complete_science = destination.load_science_checkpoint_v2().unwrap().unwrap();
    let complete_part = fs::read(destination.part_path()).unwrap();
    let complete_anchor = next_anchor
        .state
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .1
        .canonical_json()
        .unwrap();
    let token = prepare_archived_nisar_recovery_v2(
        &source_work,
        &new_work,
        &anchor_json,
        record.record_sha256(),
        source_workspace,
        &destination,
        &WorkerControl::default(),
    )
    .unwrap();
    finalize_archived_nisar_recovery_v2(
        token,
        &new_work,
        &next,
        &remote,
        &next_anchor,
        &destination,
        &WorkerControl::default(),
    )
    .unwrap();
    assert_eq!(
        destination.load_science_checkpoint_v2().unwrap().unwrap(),
        complete_science,
        "replayed import must never replace later checkpoint generations"
    );
    assert_eq!(fs::read(destination.part_path()).unwrap(), complete_part);
    assert_eq!(
        next_anchor
            .state
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .1
            .canonical_json()
            .unwrap(),
        complete_anchor
    );
    assert_eq!(
        source_workspace
            .load_science_checkpoint_v2()
            .unwrap()
            .unwrap(),
        old_science
    );
    assert_eq!(
        source_workspace
            .load_planning_checkpoint_v2()
            .unwrap()
            .unwrap(),
        old_planning
    );
    assert_eq!(fs::read(source_workspace.part_path()).unwrap(), old_part);
    assert_eq!(
        source_anchor
            .state
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .1
            .canonical_json()
            .unwrap(),
        anchor_json
    );
}

#[test]
fn science_range_v2_pause_cancel_and_restart_preserve_verified_ranges_without_refetch() {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as Error;

    for (label, cancel) in [("pause", false), ("cancel", true)] {
        let fixture_label = format!("science-v2-{label}-resume");
        let (case, intent, bootstrap, source_path) =
            concrete_fixture(&fixture_label, NisarSubsetFrequency::A, &["HH"]);
        let mut work = work(&case, &intent);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan())
            .unwrap();
        let physical = completed.physical_execution_plan();
        let first_range_bytes = physical.execution_transfer_ranges().unwrap()[0]
            .len()
            .unwrap();
        let control = WorkerControl::default();
        let hook_control = control.clone();
        let responses = exact_science_responses(&source_path, physical, 0);
        let remaining_count = responses.len() - 1;
        let server = ScienceServer::new(responses);
        let stable_url = server.url_root.clone();
        let asset = science_asset(
            &stable_url,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        );
        let anchor = ScienceCheckpointAnchorFixture::default();
        let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
        let executor = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
            science_http(),
            asset,
        )
        .unwrap()
        .with_after_commit_hook_for_test(Arc::new(move |index| {
            if index == 0 {
                if cancel {
                    hook_control.cancel_for_test();
                } else {
                    hook_control.set_paused_for_test(true);
                }
            }
        }));
        assert_eq!(
            executor.execute(&work, &completed, &authority, &control),
            Err(if cancel {
                Error::Cancelled
            } else {
                Error::Paused
            })
        );

        let restart_asset = science_asset(
            &stable_url,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        );
        let executor = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
            science_http(),
            restart_asset,
        )
        .unwrap();
        let result = executor
            .execute(&work, &completed, &authority, &WorkerControl::default())
            .unwrap();
        assert!(result.complete());
        assert_eq!(result.resumed_bytes(), first_range_bytes);
        assert_eq!(
            result.science_request_count(),
            physical.execution_transfer_ranges().unwrap().len() as u64
        );
        assert_eq!(server.finish().len(), remaining_count + 1);
        let accounting = result.accounting();
        assert_eq!(
            accounting.planning_request_count(),
            completed
                .bootstrap_statistics()
                .total_planning_request_count()
        );
        assert_eq!(
            accounting.planning_response_body_bytes(),
            completed
                .bootstrap_statistics()
                .total_planning_response_body_bytes()
        );
        assert_eq!(
            accounting.combined_request_count(),
            accounting.planning_request_count() + accounting.science_request_count()
        );
        assert_eq!(
            accounting.combined_response_body_bytes(),
            accounting.planning_response_body_bytes() + accounting.science_response_body_bytes()
        );
        assert_eq!(
            accounting.science_response_body_bytes(),
            physical
                .estimated_execution_transfer_response_bytes()
                .unwrap()
        );
        assert_eq!(
            result.physical_plan_sha256(),
            physical.physical_plan_sha256()
        );

        let no_network = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
            science_http(),
            science_asset(
                &stable_url,
                physical,
                Some(&physical.source_validator().value),
                None,
                None,
            ),
        )
        .unwrap();
        let recovered_complete = no_network
            .execute(&work, &completed, &authority, &WorkerControl::default())
            .unwrap();
        assert!(recovered_complete.complete());
        assert_eq!(
            recovered_complete.resumed_bytes(),
            physical
                .estimated_execution_transfer_response_bytes()
                .unwrap()
        );
    }
}

#[test]
fn nisar_subset_committed_recovery_telemetry_tracks_reusable_and_reused_bytes() {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as Error;

    let (case, intent, bootstrap, source_path) = concrete_fixture(
        "science-v2-committed-recovery-telemetry",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let physical = completed.physical_execution_plan();
    let ranges = physical.execution_transfer_ranges().unwrap();
    assert!(ranges.len() > 1);
    let first_range_bytes = ranges[0].len().unwrap();
    let total_range_bytes = physical
        .estimated_execution_transfer_response_bytes()
        .unwrap();
    let server = ScienceServer::new(exact_science_responses(&source_path, physical, 0));
    let stable_url = server.url_root.clone();
    let asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let anchor = ScienceCheckpointAnchorFixture::default();
    let authority = science_authority(bootstrap.as_ref(), &asset, &work, &completed, &anchor);
    let control = WorkerControl::default();
    let hook_control = control.clone();
    let executor = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        asset,
    )
    .unwrap()
    .with_after_commit_hook_for_test(Arc::new(move |index| {
        if index == 0 {
            hook_control.set_paused_for_test(true);
        }
    }));
    assert_eq!(
        executor.execute(&work, &completed, &authority, &control),
        Err(Error::Paused)
    );

    let paused_anchor = anchor
        .state
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .1
        .canonical_json()
        .unwrap();
    let paused_anchor: Value = serde_json::from_str(&paused_anchor).unwrap();
    let paused_telemetry = &paused_anchor["committed_recovery_telemetry"];
    assert_eq!(
        paused_telemetry["contract"],
        json!("nisar_subset_committed_recovery_telemetry_v1")
    );
    assert_eq!(
        paused_telemetry["checkpoint_sha256"],
        paused_anchor["committed_checkpoint_sha256"]
    );
    assert_eq!(
        paused_telemetry["reusable_verified_bytes"],
        json!(first_range_bytes)
    );
    assert_eq!(paused_telemetry["reused_verified_bytes"], json!(0));

    let resumed_asset = science_asset(
        &stable_url,
        physical,
        Some(&physical.source_validator().value),
        None,
        None,
    );
    let resumed = crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
        science_http(),
        resumed_asset,
    )
    .unwrap()
    .execute(&work, &completed, &authority, &WorkerControl::default())
    .unwrap();
    assert!(resumed.complete());
    assert_eq!(resumed.resumed_bytes(), first_range_bytes);

    let complete_anchor = anchor
        .state
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .1
        .canonical_json()
        .unwrap();
    let complete_anchor: Value = serde_json::from_str(&complete_anchor).unwrap();
    let complete_telemetry = &complete_anchor["committed_recovery_telemetry"];
    assert_eq!(
        complete_telemetry["checkpoint_sha256"],
        complete_anchor["committed_checkpoint_sha256"]
    );
    assert_eq!(
        complete_telemetry["reusable_verified_bytes"],
        json!(total_range_bytes)
    );
    assert_eq!(
        complete_telemetry["reused_verified_bytes"],
        json!(first_range_bytes)
    );
    assert_eq!(server.finish().len(), ranges.len());
}

#[test]
fn science_range_v2_recovery_rejects_checkpoint_part_plan_and_material_attacks_before_io() {
    use crate::nisar_science_transfer_v2::{
        mutate_science_checkpoint_v2_for_test, NisarSubsetScienceRangeExecutorV2,
        NisarSubsetScienceTransferErrorV2 as Error, ScienceCheckpointMutationV2ForTest,
    };
    use sha2::{Digest, Sha256};

    for attack in [
        "science-checkpoint-missing",
        "science-checkpoint-corrupt",
        "science-owner-rehash",
        "science-accounting-rehash",
        "completed-material-coherent-rehash",
        "science-part-missing",
        "science-part-replaced",
        "science-part-truncated",
        "science-part-extended",
        "planning-checkpoint-missing",
        "planning-checkpoint-corrupt",
    ] {
        let (_case, work, completed, bootstrap, anchor, stable_url, source_path) =
            partial_science_fixture(&format!("science-v2-recovery-{attack}"));
        let physical = completed.physical_execution_plan();
        let workspace = bootstrap.science_workspace_v2();
        match attack {
            "science-checkpoint-missing" => {
                fs::remove_file(workspace.science_checkpoint_path_v2()).unwrap();
            }
            "science-checkpoint-corrupt" => {
                fs::write(workspace.science_checkpoint_path_v2(), b"not-json").unwrap();
            }
            "science-owner-rehash" => mutate_science_checkpoint_v2_for_test(
                workspace,
                ScienceCheckpointMutationV2ForTest::RemoveFirstPlannedOwner,
            ),
            "science-accounting-rehash" => mutate_science_checkpoint_v2_for_test(
                workspace,
                ScienceCheckpointMutationV2ForTest::DecrementCommittedScienceBytes,
            ),
            "completed-material-coherent-rehash" => {
                let range = &physical.execution_transfer_ranges().unwrap()[0];
                let mut part = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(workspace.part_path())
                    .unwrap();
                let mut bytes = vec![0_u8; range.len().unwrap() as usize];
                part.seek(SeekFrom::Start(range.start())).unwrap();
                part.read_exact(&mut bytes).unwrap();
                bytes[0] ^= 0xff;
                part.seek(SeekFrom::Start(range.start())).unwrap();
                part.write_all(&bytes).unwrap();
                part.sync_all().unwrap();
                drop(part);
                let digest = format!("{:x}", Sha256::digest(&bytes));
                mutate_science_checkpoint_v2_for_test(
                    workspace,
                    ScienceCheckpointMutationV2ForTest::FirstCompletedDigest(digest),
                );
            }
            "science-part-missing" => {
                fs::remove_file(workspace.part_path()).unwrap();
            }
            "science-part-replaced" => {
                fs::remove_file(workspace.part_path()).unwrap();
                fs::copy(&source_path, workspace.part_path()).unwrap();
            }
            "science-part-truncated" => {
                OpenOptions::new()
                    .write(true)
                    .open(workspace.part_path())
                    .unwrap()
                    .set_len(physical.source_size() - 1)
                    .unwrap();
            }
            "science-part-extended" => {
                OpenOptions::new()
                    .write(true)
                    .open(workspace.part_path())
                    .unwrap()
                    .set_len(physical.source_size() + 1)
                    .unwrap();
            }
            "planning-checkpoint-missing" => {
                fs::remove_file(bootstrap.planning_checkpoint_path_for_test()).unwrap();
            }
            "planning-checkpoint-corrupt" => {
                fs::write(bootstrap.planning_checkpoint_path_for_test(), b"not-json").unwrap();
            }
            _ => unreachable!(),
        }
        let asset = science_asset(
            &stable_url,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        );
        let authority = science_authority(
            bootstrap.as_ref(),
            &asset,
            &work,
            &completed,
            anchor.as_ref(),
        );
        let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), asset).unwrap();
        assert_eq!(
            executor.execute(&work, &completed, &authority, &WorkerControl::default(),),
            Err(Error::RecoveryInvalid),
            "attack {attack}"
        );
    }
}

#[test]
fn science_range_v2_revalidates_source_asset_and_physical_authority_before_io() {
    use crate::nisar_science_transfer_v2::{
        NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as Error,
    };

    for attack in [
        "asset-file",
        "asset-url",
        "asset-size",
        "asset-etag",
        "physical-plan",
        "source-drift",
    ] {
        let (case, intent, bootstrap, source_path) = concrete_fixture(
            &format!("science-v2-authority-{attack}"),
            NisarSubsetFrequency::A,
            &["HH"],
        );
        let mut work = work(&case, &intent);
        let mut completed = plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan())
            .unwrap();
        if attack == "physical-plan" {
            completed.physical_execution_plan.science_ranges[0].remove_logical_owner_for_test();
        }
        if attack == "source-drift" {
            OpenOptions::new()
                .append(true)
                .open(&source_path)
                .unwrap()
                .write_all(&[1])
                .unwrap();
        }
        let physical = completed.physical_execution_plan();
        let expected = match attack {
            "asset-file" | "asset-url" | "physical-plan" => Error::AuthorityMismatch,
            "asset-size" | "asset-etag" | "source-drift" => Error::StrongValidatorChanged,
            _ => unreachable!(),
        };
        let authority_asset = science_asset(
            "http://127.0.0.1:9",
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        );
        let anchor = ScienceCheckpointAnchorFixture::default();
        let authority = science_authority(
            bootstrap.as_ref(),
            &authority_asset,
            &work,
            &completed,
            &anchor,
        );
        let execution_asset = science_asset(
            if attack == "asset-url" {
                "http://127.0.0.1:10"
            } else {
                "http://127.0.0.1:9"
            },
            physical,
            Some(if attack == "asset-etag" {
                "\"changed\""
            } else {
                &physical.source_validator().value
            }),
            (attack == "asset-size").then_some(physical.source_size() - 1),
            (attack == "asset-file").then_some("different.h5"),
        );
        let executor =
            NisarSubsetScienceRangeExecutorV2::new(science_http(), execution_asset).unwrap();
        assert_eq!(
            executor.execute(&work, &completed, &authority, &WorkerControl::default(),),
            Err(expected),
            "attack {attack}"
        );
    }
}

#[test]
fn adapter_reservation_rejects_over_budget_range_before_copy_and_rolls_back_failures() {
    let (case, _intent, bootstrap, _source) =
        concrete_fixture("reservation-before-copy", NisarSubsetFrequency::A, &["HH"]);
    let counter = Arc::new(AtomicU64::new(0));
    let bootstrap = Arc::try_unwrap(bootstrap)
        .ok()
        .unwrap()
        .with_copy_counter_for_test(Arc::clone(&counter));
    let source_size = fs::metadata(&case.source_path).unwrap().len();
    let selected_science_start = source_size - 1;
    let mut ledger = BootstrapLedger::new(
        source_size,
        "a".repeat(64),
        vec![(selected_science_start, source_size)],
    )
    .unwrap();
    let over_budget_end = ledger.max_response_body_bytes + 1;
    assert!(over_budget_end < selected_science_start);
    let mut target = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(bootstrap.workspace.part_path())
        .unwrap();
    target.set_len(source_size).unwrap();

    let denied = bootstrap.copy_reserved_range(
        &mut ledger,
        &mut target,
        BootstrapCategory::MetadataDiscovery,
        0,
        over_budget_end,
        &WorkerControl::default(),
    );
    assert!(matches!(
        denied,
        Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    assert!(ledger.events.is_empty());
    assert!(ledger.pending.is_none());

    let paused = WorkerControl::default();
    paused.set_paused_for_test(true);
    let stopped = bootstrap.copy_reserved_range(
        &mut ledger,
        &mut target,
        BootstrapCategory::MetadataDiscovery,
        0,
        1,
        &paused,
    );
    assert!(matches!(
        stopped,
        Err(NisarSubsetPlanningErrorV2::ExecutionStopped)
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    assert!(ledger.events.is_empty());
    assert!(ledger.pending.is_none());
}

#[test]
fn ledger_rejects_the_257th_request_before_any_over_cap_copy_attempt() {
    let source_size = 100_000;
    let mut ledger =
        BootstrapLedger::new(source_size, "a".repeat(64), vec![(99_000, 99_001)]).unwrap();
    let mut copied_requests = 0_u64;
    let mut copied_bytes = 0_u64;
    for index in 0..NISAR_LIVE_MAX_REQUESTS {
        let start = index * 2;
        let reservation = ledger
            .reserve(BootstrapCategory::MetadataDiscovery, start, start + 1, 1)
            .unwrap();
        copied_requests += 1;
        copied_bytes += 1;
        ledger.commit(reservation, 1).unwrap();
    }
    let denied = ledger.reserve(
        BootstrapCategory::MetadataDiscovery,
        NISAR_LIVE_MAX_REQUESTS * 2,
        NISAR_LIVE_MAX_REQUESTS * 2 + 1,
        1,
    );
    assert!(matches!(
        denied,
        Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)
    ));
    assert_eq!(copied_requests, NISAR_LIVE_MAX_REQUESTS);
    assert_eq!(copied_bytes, NISAR_LIVE_MAX_REQUESTS);
    assert_eq!(ledger.events.len() as u64, NISAR_LIVE_MAX_REQUESTS);
    assert!(ledger.pending.is_none());
}

#[test]
fn concrete_bootstrap_records_actual_ranges_and_freezes_store_ack_for_a_multi_and_b() {
    for (label, frequency, polarizations) in [
        (
            "actual-ledger-a-multi",
            NisarSubsetFrequency::A,
            vec!["HH", "HV"],
        ),
        (
            "actual-ledger-b-multi",
            NisarSubsetFrequency::B,
            vec!["HV", "VV"],
        ),
    ] {
        let (case, intent, bootstrap, _source) = concrete_fixture(label, frequency, &polarizations);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &work(&case, &intent),
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| {
                let proposed = ready.proposed_plan;
                let mut ack = proposed.clone();
                ack.warnings.push("Store canonical ack.".to_owned());
                ack.warnings.sort();
                ack.validate().unwrap();
                assert_eq!(
                    ack.canonical_semantic_plan_digest,
                    proposed.canonical_semantic_plan_digest
                );
                assert_ne!(ack.warnings, proposed.warnings);
                Ok(ack)
            },
        )
        .unwrap();
        let stats = completed.bootstrap_statistics();
        assert_eq!(stats.preflight().request_count(), 1);
        assert_eq!(stats.revalidation().request_count(), 1);
        assert_eq!(stats.revalidation().response_body_bytes(), 1);
        assert!(stats.metadata_prefix().response_body_bytes() > 0);
        assert_eq!(
            stats.metadata_discovery().request_count()
                + stats.additional_geolocation().request_count(),
            2
        );
        assert_eq!(
            stats.metadata_discovery().response_body_bytes()
                + stats.additional_geolocation().response_body_bytes(),
            99_565
        );
        assert!(stats.additional_geolocation().request_count() > 0);
        assert_eq!(stats.science_request_count(), 0);
        assert_eq!(stats.science_response_body_bytes(), 0);
        assert!(stats.total_planning_response_body_bytes() <= stats.source_size());
        assert!(completed.frozen_plan().transfer_estimate.is_none());
        assert!(completed.frozen_plan().derived_estimate.is_none());
        assert!(completed
            .frozen_plan()
            .warnings
            .contains(&"Store canonical ack.".to_owned()));
    }
}

#[test]
fn concrete_revalidation_records_probe_and_rejects_changed_source() {
    let (case, intent, bootstrap, source) = concrete_fixture(
        "actual-ledger-validator-change",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let bootstrap = Arc::try_unwrap(bootstrap)
        .ok()
        .unwrap()
        .with_revalidation_hook_for_test(Arc::new(move || {
            OpenOptions::new()
                .append(true)
                .open(&source)
                .unwrap()
                .write_all(&[1])
                .unwrap();
        }));
    let result = plan_and_freeze_nisar_subset_v2_with(
        &work(&case, &intent),
        &WorkerControl::default(),
        &bootstrap,
        |_, _| panic!("changed validator must not reach PlanReady"),
    );
    assert!(matches!(
        result,
        Err(NisarSubsetPlanningErrorV2::SourceValidatorChanged)
    ));
}

#[test]
fn concrete_pause_and_cancel_stop_before_plan_ready() {
    for (label, cancel) in [
        ("actual-ledger-pause-before-freeze", false),
        ("actual-ledger-cancel-before-freeze", true),
    ] {
        let (case, intent, bootstrap, _source) =
            concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
        let control = WorkerControl::default();
        let hook_control = control.clone();
        let bootstrap = Arc::try_unwrap(bootstrap)
            .ok()
            .unwrap()
            .with_revalidation_hook_for_test(Arc::new(move || {
                if cancel {
                    hook_control.cancel_for_test();
                } else {
                    hook_control.set_paused_for_test(true);
                }
            }));
        let result = plan_and_freeze_nisar_subset_v2_with(
            &work(&case, &intent),
            &control,
            &bootstrap,
            |_, _| panic!("stopped planning must not reach PlanReady"),
        );
        assert!(matches!(
            result,
            Err(NisarSubsetPlanningErrorV2::ExecutionStopped)
        ));
    }
}

#[test]
fn live_inventory_callback_cancellation_maps_to_execution_stopped_at_every_planning_boundary() {
    let (_case, intent, _bootstrap, source_path) = concrete_fixture(
        "live-inventory-cancel-mapping",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let source_validator = NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: format!("\"sha256-{}\"", test_file_sha256(&source_path)),
        content_length: fs::metadata(&source_path).unwrap().len(),
    };
    let error = crate::nisar_subset_writer::NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        &intent,
        source_validator,
        &source_path,
        &|| true,
    )
    .expect_err("the real inventory callback must surface typed cancellation");
    assert_eq!(error, crate::NisarSubsetWriteError::Cancelled);

    for fallback in [
        NisarSubsetPlanningErrorV2::RecoveryInvalid,
        NisarSubsetPlanningErrorV2::BootstrapFailed,
        NisarSubsetPlanningErrorV2::InventoryMismatch,
    ] {
        assert_eq!(
            map_live_inventory_error(error, fallback),
            NisarSubsetPlanningErrorV2::ExecutionStopped
        );
    }
    assert_eq!(
        map_live_inventory_error(
            crate::NisarSubsetWriteError::MetadataInvalid,
            NisarSubsetPlanningErrorV2::InventoryMismatch,
        ),
        NisarSubsetPlanningErrorV2::InventoryMismatch
    );
}

#[test]
fn crash_before_freeze_restarts_concrete_bootstrap_and_then_freezes() {
    let (case, intent, bootstrap, _source) = concrete_fixture(
        "actual-ledger-crash-before-freeze",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
    );
    let work = work(&case, &intent);
    let first = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |_, _| Err(NisarSubsetPlanningErrorV2::FreezeFailed),
    );
    assert!(matches!(
        first,
        Err(NisarSubsetPlanningErrorV2::FreezeFailed)
    ));

    let replanned = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    assert_eq!(
        replanned.bootstrap_statistics().preflight().request_count(),
        1
    );
    assert_eq!(
        replanned
            .bootstrap_statistics()
            .revalidation()
            .request_count(),
        1
    );
    assert_eq!(replanned.bootstrap_statistics().science_request_count(), 0);
}

#[test]
fn frozen_plan_fast_path_uses_canonical_source_estimate_without_bootstrap_or_attach() {
    let (case, intent, bootstrap, _source) =
        concrete_fixture("actual-ledger-fast-path", NisarSubsetFrequency::A, &["HH"]);
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    assert_eq!(
        completed
            .physical_execution_plan()
            .combined_budget
            .budget_policy,
        Some(NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1)
    );
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let fast = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |_, _| panic!("fast path must not attach"),
    )
    .unwrap();
    assert!(fast.bootstrap_statistics().frozen_plan_fast_path_used());
    assert_eq!(
        fast.bootstrap_statistics().source_size(),
        fast.frozen_plan().source_estimate.as_ref().unwrap().bytes
    );
    assert_eq!(
        fast.physical_execution_plan().physical_plan_sha256(),
        completed.physical_execution_plan().physical_plan_sha256()
    );
    assert!(fast.planning_workspace_checkpoint_reused());
}

#[test]
fn frozen_plan_fast_path_preserves_legacy_absent_budget_policy() {
    let (case, intent, bootstrap, _source) = concrete_fixture(
        "physical-fast-path-legacy-budget-policy",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();

    let checkpoint_path = bootstrap.planning_checkpoint_path_for_test();
    let mut checkpoint: NisarSubsetPlanningWorkspaceCheckpointV2 =
        serde_json::from_str(&fs::read_to_string(&checkpoint_path).unwrap()).unwrap();
    let (
        committed_planning_request_count,
        committed_planning_response_body_bytes,
        projected_science_request_count,
        projected_science_response_body_bytes,
    ) = {
        let budget = &checkpoint.physical_plan.combined_budget;
        (
            budget.committed_planning_request_count,
            budget.committed_planning_response_body_bytes,
            budget.projected_science_request_count,
            budget.projected_science_response_body_bytes,
        )
    };
    checkpoint.physical_plan.combined_budget = project_combined_budget_with_policy_v2(
        committed_planning_request_count,
        committed_planning_response_body_bytes,
        projected_science_request_count,
        projected_science_response_body_bytes,
        checkpoint.source_size,
        total_request_limit_v2(NisarScienceRoot::Lsar).unwrap(),
        None,
    )
    .unwrap();
    checkpoint.physical_plan.physical_plan_sha256 = checkpoint
        .physical_plan
        .recompute_physical_plan_sha256()
        .unwrap();
    checkpoint.checkpoint_sha256 = checkpoint.recompute_sha256().unwrap();
    let legacy_json = serde_json::to_value(&checkpoint).unwrap();
    assert!(legacy_json["physical_plan"]["combined_budget"]
        .get("budget_policy")
        .is_none());
    fs::write(&checkpoint_path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();

    let reopened = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |_, _| panic!("legacy fast path must not attach"),
    )
    .unwrap();
    assert!(reopened.planning_workspace_checkpoint_reused());
    assert_eq!(
        reopened
            .physical_execution_plan()
            .combined_budget
            .budget_policy,
        None
    );
}

#[test]
fn physical_plan_maps_every_selected_polarization_and_supported_frequency() {
    for (label, frequency, polarizations) in [
        ("physical-plan-a-hh", NisarSubsetFrequency::A, vec!["HH"]),
        (
            "physical-plan-a-hh-hv",
            NisarSubsetFrequency::A,
            vec!["HH", "HV"],
        ),
        (
            "physical-plan-b-hv-vv",
            NisarSubsetFrequency::B,
            vec!["HV", "VV"],
        ),
    ] {
        let (case, intent, bootstrap, _source) =
            concrete_fixture(label, frequency.clone(), &polarizations);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &work(&case, &intent),
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        let physical = completed.physical_execution_plan();
        assert_eq!(physical.selected_frequency(), &frequency);
        assert_eq!(
            physical.selected_polarizations(),
            polarizations
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(physical.dataset_mappings().len(), polarizations.len());
        assert_eq!(
            physical
                .dataset_mappings()
                .iter()
                .map(|mapping| mapping.polarization())
                .collect::<Vec<_>>(),
            polarizations
        );
        assert!(physical
            .science_ranges()
            .iter()
            .all(|range| !range.logical_owners().is_empty()));
        assert!(physical.estimated_science_response_bytes() > 0);
        assert!(physical.estimated_derived_artifact_bytes() > 0);
        let expected_dependency_paths = completed
            .frozen_plan()
            .required_dependencies
            .items
            .iter()
            .filter(|dependency| {
                dependency.kind != NisarSubsetDependencyKindV2::SelectedScienceDataset
                    && dependency.kind != NisarSubsetDependencyKindV2::Attribute
            })
            .map(|dependency| dependency.source_path.clone())
            .collect::<BTreeSet<_>>();
        let actual_dependency_paths = physical
            .dependency_ranges
            .iter()
            .flat_map(|range| &range.logical_owners)
            .map(|owner| owner.dataset_path.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(actual_dependency_paths, expected_dependency_paths);
        assert_eq!(physical.non_dataset_dependencies.len(), 1);
        assert_eq!(physical.non_dataset_dependencies[0].source_path, "/");
        assert!(physical.non_dataset_dependencies[0]
            .attribute_names
            .contains(&"Conventions".to_owned()));
        assert!(physical.validate().is_ok());
    }
}

#[test]
fn physical_plan_digest_is_deterministic_and_rejects_mutation() {
    let (case, intent, bootstrap, _source) =
        concrete_fixture("physical-digest", NisarSubsetFrequency::A, &["HH", "HV"]);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work(&case, &intent),
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    let physical = completed.physical_execution_plan();
    assert_eq!(
        physical.physical_plan_sha256(),
        physical.recompute_physical_plan_sha256_for_test().unwrap()
    );
    let mut mutated = physical.clone();
    mutated.estimated_derived_artifact_bytes += 1;
    assert_eq!(
        mutated.validate(),
        Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)
    );
}

#[test]
fn contiguous_validity_mask_maps_window_to_single_bounded_envelope_range() {
    let root = PrefixLedgerTestRoot::new("contiguous-validity-mask");
    let path = root.0.join("mask.h5");
    let file = hdf5::File::create(&path).unwrap();
    let dataset = file
        .new_dataset::<u8>()
        .shape((6, 10))
        .create("mask")
        .unwrap();
    dataset.write_raw(&vec![1_u8; 60]).unwrap();
    file.flush().unwrap();
    let source_size = fs::metadata(&path).unwrap().len();
    let offset = hdf5::sync::sync(|| unsafe { hdf5_sys::h5d::H5Dget_offset(dataset.id()) });
    assert_ne!(offset, u64::MAX);

    let window = NisarSubsetPhysicalWindowV2 {
        component_id: 0,
        component_sha256: "a".repeat(64),
        window_sha256: "b".repeat(64),
        line_start: 1,
        line_end_exclusive: 4,
        sample_start: 2,
        sample_end_exclusive: 5,
    };
    let ranges = physical_dataset_ranges_for_windows_v2(&dataset, &[&window], source_size).unwrap();

    assert_eq!(ranges, vec![(offset + 12, offset + 35)]);
    assert!(ranges[0].1 <= source_size);
}

#[test]
fn combined_budget_accepts_exact_limit_and_rejects_request_257_and_byte_overrun() {
    let source_size = 10_000;
    let hard_bytes = nisar_live_budget_response_limit(source_size).unwrap();
    let exact = project_combined_budget_with_policy_v2(
        255,
        hard_bytes - 1,
        1,
        1,
        source_size,
        NISAR_LIVE_MAX_REQUESTS,
        None,
    )
    .unwrap();
    assert_eq!(exact.total_request_count(), 256);
    assert_eq!(exact.total_response_body_bytes(), hard_bytes);

    assert_eq!(
        project_combined_budget_with_policy_v2(
            256,
            1,
            1,
            1,
            source_size,
            NISAR_LIVE_MAX_REQUESTS,
            None,
        ),
        Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)
    );
    assert_eq!(
        project_combined_budget_with_policy_v2(
            1,
            hard_bytes,
            1,
            1,
            source_size,
            NISAR_LIVE_MAX_REQUESTS,
            None,
        ),
        Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)
    );
}

#[test]
fn budget_policy_v2_accounts_for_redirect_and_auth_requests_without_rewriting_v1() {
    let policy: NisarSubsetCombinedBudgetPolicyV2 =
        serde_json::from_value(json!("physical_execution_union_v2"))
            .expect("fresh production must recognize the versioned HTTP-aware budget");
    let projected = project_combined_budget_with_policy_v2(
        31,
        195_035_138,
        467,
        495_865_474,
        15_699_279_872,
        NISAR_LIVE_MAX_REQUESTS,
        Some(policy),
    )
    .unwrap();
    assert_eq!(projected.max_request_count(), 20_579);
    assert_eq!(projected.max_science_request_count(), 20_548);
    assert_eq!(projected.max_response_body_bytes(), 1_186_766_086);
    assert_eq!(
        projected.remaining_science_response_body_bytes(),
        991_730_948
    );

    // The literal is the pre-upgrade V1 wire representation, not a new builder.
    let old_json = r#"{"max_request_count":965,"max_response_body_bytes":1186766086,"committed_planning_request_count":31,"committed_planning_response_body_bytes":195035138,"projected_science_request_count":467,"projected_science_response_body_bytes":495865474,"remaining_science_request_count":934,"remaining_science_response_body_bytes":991730948,"budget_policy":"physical_execution_union_v1"}"#;
    let old: NisarSubsetCombinedBudgetProjectionV2 = serde_json::from_str(old_json).unwrap();
    old.validate(15_699_279_872, NISAR_LIVE_MAX_REQUESTS)
        .unwrap();
    assert_eq!(serde_json::to_string(&old).unwrap(), old_json);
    assert_eq!(old.max_science_request_count(), 934);
}

#[test]
fn budget_policy_v2_rejects_invalid_union_and_overflow() {
    let policy: NisarSubsetCombinedBudgetPolicyV2 =
        serde_json::from_value(json!("physical_execution_union_v2")).unwrap();
    for (requests, planning_bytes, count, bytes, source) in [
        (31, 1, 0, 1, 100),
        (
            31,
            1,
            NISAR_SUBSET_PHYSICAL_MAX_EXECUTION_RANGES_V2 as u64 + 1,
            1,
            100,
        ),
        (31, 1, 1, 0, 100),
        (31, 1, 1, 101, 100),
        (u64::MAX - 43, 1, 1, 1, 100),
        (31, u64::MAX - 1, 1, 1, 100),
        (31, 1, 1, u64::MAX, u64::MAX),
    ] {
        assert_eq!(
            project_combined_budget_with_policy_v2(
                requests,
                planning_bytes,
                count,
                bytes,
                source,
                NISAR_LIVE_MAX_REQUESTS,
                Some(policy)
            ),
            Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded),
        );
    }
}

#[test]
fn production_execution_budget_admits_owner_scale_union() {
    let source_size = 4_000_000_000;
    let committed_planning_bytes = 195_035_136;
    let projected_execution_bytes = 1_073_741_824;
    assert!(
        committed_planning_bytes + projected_execution_bytes
            > nisar_live_budget_response_limit(source_size).unwrap()
    );

    let projected = project_combined_budget_v2(
        200,
        committed_planning_bytes,
        48,
        projected_execution_bytes,
        source_size,
        total_request_limit_v2(NisarScienceRoot::Ssar).unwrap(),
    )
    .expect("a verified production execution union needs one full transfer plus one total retry");

    assert_eq!(
        projected.max_response_body_bytes(),
        committed_planning_bytes + 2 * projected_execution_bytes
    );
    assert_eq!(
        projected.remaining_science_response_body_bytes(),
        2 * projected_execution_bytes
    );
    assert_eq!(projected.max_science_request_count(), 96);
    assert_eq!(projected.max_request_count(), 296);
    assert_eq!(
        serde_json::to_value(&projected).unwrap()["budget_policy"],
        json!("physical_execution_union_v1")
    );
}

#[test]
fn production_execution_union_admits_more_than_legacy_256_ranges() {
    let source_size = 2_000;
    let science_ranges = (0..493_u64)
        .map(|index| NisarSubsetPhysicalRangeV2 {
            start: index * 2,
            end_exclusive: index * 2 + 1,
            logical_owners: vec![NisarSubsetPhysicalLogicalOwnerV2 {
                dataset_path: "/science/SSAR/RSLC/swaths/frequencyA/HH".to_owned(),
                polarization: Some("HH".to_owned()),
                component_id: Some(0),
                window_sha256: Some("a".repeat(64)),
                logical_chunk: Some([index, 0]),
            }],
        })
        .collect::<Vec<_>>();

    let union = canonical_execution_transfer_ranges_v2(&[], &science_ranges, source_size)
        .expect("the reviewed production plan has 493 bounded execution ranges");
    assert_eq!(union.len(), 493);
    assert_eq!(
        canonical_execution_transfer_ranges_with_policy_v2(&[], &science_ranges, source_size, None,),
        Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded),
        "legacy checkpoints must retain their original 256-range authorization"
    );
}

#[test]
fn production_execution_union_rejects_one_range_above_response_hard_ceiling() {
    let source_size = crate::NISAR_LIVE_MAX_RESPONSE_BODY_BYTES + 1;
    let huge_range = NisarSubsetPhysicalRangeV2 {
        start: 0,
        end_exclusive: source_size,
        logical_owners: vec![NisarSubsetPhysicalLogicalOwnerV2 {
            dataset_path: "/science/SSAR/RSLC/swaths/frequencyA/HH".to_owned(),
            polarization: Some("HH".to_owned()),
            component_id: Some(0),
            window_sha256: Some("a".repeat(64)),
            logical_chunk: Some([0, 0]),
        }],
    };

    for (dependency_ranges, science_ranges) in [
        (vec![huge_range.clone()], Vec::new()),
        (Vec::new(), vec![huge_range.clone()]),
    ] {
        assert_eq!(
            canonical_execution_transfer_ranges_v2(
                &dependency_ranges,
                &science_ranges,
                source_size,
            ),
            Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)
        );
    }
    assert_eq!(
        validate_physical_ranges(&[huge_range], source_size, 1),
        Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)
    );
}

#[test]
fn production_execution_budget_is_checked_and_legacy_summary_shape_is_unchanged() {
    let source_size = 10_000;
    let legacy = project_combined_budget_with_policy_v2(
        1,
        1,
        1,
        1,
        source_size,
        NISAR_LIVE_MAX_REQUESTS,
        None,
    )
    .unwrap();
    assert!(serde_json::to_value(&legacy)
        .unwrap()
        .get("budget_policy")
        .is_none());
    assert_eq!(
        legacy.max_science_request_count(),
        NISAR_LIVE_MAX_REQUESTS - 1
    );

    for (planning_bytes, science_bytes) in [(1, source_size + 1), (u64::MAX, 1)] {
        assert_eq!(
            project_combined_budget_with_policy_v2(
                1,
                planning_bytes,
                1,
                science_bytes,
                source_size,
                NISAR_LIVE_MAX_REQUESTS,
                Some(NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1),
            ),
            Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)
        );
    }
    assert_eq!(
        project_combined_budget_with_policy_v2(
            1,
            1,
            NISAR_SUBSET_PHYSICAL_MAX_EXECUTION_RANGES_V2 as u64 + 1,
            1,
            source_size,
            NISAR_LIVE_MAX_REQUESTS,
            Some(NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1),
        ),
        Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)
    );
}

#[test]
fn ssar_planning_request_budget_is_scoped_without_relaxing_science_cap() {
    assert_eq!(
        planning_request_limit_v2(NisarScienceRoot::Lsar),
        NISAR_LIVE_MAX_REQUESTS
    );
    assert_eq!(
        planning_request_limit_v2(NisarScienceRoot::Ssar),
        NISAR_SSAR_METADATA_PLANNING_MAX_REQUESTS_V2
    );
    assert_eq!(
        total_request_limit_v2(NisarScienceRoot::Lsar).unwrap(),
        NISAR_LIVE_MAX_REQUESTS
    );
    assert_eq!(
        total_request_limit_v2(NisarScienceRoot::Ssar).unwrap(),
        NISAR_SSAR_METADATA_PLANNING_MAX_REQUESTS_V2 + NISAR_LIVE_MAX_REQUESTS
    );

    let source_size = 10_000_000;
    let max_bytes = nisar_live_budget_response_limit(source_size).unwrap();
    let ssar_total = total_request_limit_v2(NisarScienceRoot::Ssar).unwrap();
    let projected = project_combined_budget_with_policy_v2(
        NISAR_SSAR_METADATA_PLANNING_MAX_REQUESTS_V2,
        1,
        NISAR_LIVE_MAX_REQUESTS,
        1,
        source_size,
        ssar_total,
        None,
    )
    .unwrap();
    assert_eq!(projected.total_request_count(), ssar_total);
    assert!(projected.total_response_body_bytes() <= max_bytes);

    assert_eq!(
        project_combined_budget_with_policy_v2(
            1,
            1,
            NISAR_LIVE_MAX_REQUESTS + 1,
            1,
            source_size,
            ssar_total,
            None,
        ),
        Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)
    );
}

#[test]
fn bootstrap_checkpoint_request_budget_is_bound_to_science_root() {
    fn valid_statistics(max_request_count: u64) -> NisarSubsetBootstrapStatisticsV2 {
        let mut ledger =
            BootstrapLedger::new_preflight_with_request_limit(10_000, max_request_count).unwrap();
        let preflight = ledger
            .reserve(BootstrapCategory::Preflight, 0, 1, 1)
            .unwrap();
        ledger.bind_source_validator("a".repeat(64)).unwrap();
        ledger.commit(preflight, 1).unwrap();
        ledger
            .bind_selected_science_ranges(vec![(9_000, 9_001)])
            .unwrap();
        let revalidation = ledger
            .reserve(BootstrapCategory::Revalidation, 0, 1, 1)
            .unwrap();
        ledger.commit(revalidation, 1).unwrap();
        ledger.statistics().unwrap()
    }

    let lsar = valid_statistics(NISAR_LIVE_MAX_REQUESTS);
    assert!(lsar
        .validate_for_science_root(NisarScienceRoot::Lsar)
        .is_ok());
    assert_eq!(
        lsar.validate_for_science_root(NisarScienceRoot::Ssar),
        Err(NisarSubsetPlanningErrorV2::RecoveryInvalid)
    );

    let ssar = valid_statistics(NISAR_SSAR_METADATA_PLANNING_MAX_REQUESTS_V2);
    assert!(ssar
        .validate_for_science_root(NisarScienceRoot::Ssar)
        .is_ok());
    assert_eq!(
        ssar.validate_for_science_root(NisarScienceRoot::Lsar),
        Err(NisarSubsetPlanningErrorV2::RecoveryInvalid)
    );
}

#[test]
fn frozen_plan_fast_path_requires_the_durable_workspace_checkpoint() {
    let (case, intent, bootstrap, _source) = concrete_fixture(
        "physical-fast-path-missing-checkpoint",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    fs::remove_file(bootstrap.planning_checkpoint_path_for_test()).unwrap();

    assert!(matches!(
        plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |_, _| panic!("fast path must not attach"),
        ),
        Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)
    ));
}

#[test]
fn frozen_plan_fast_path_rejects_corrupt_and_mismatched_workspace_checkpoint() {
    for (label, corrupt) in [
        ("physical-fast-path-corrupt", true),
        ("physical-fast-path-mismatch", false),
    ] {
        let (case, intent, bootstrap, _source) =
            concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
        let mut work = work(&case, &intent);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan())
            .unwrap();
        if corrupt {
            fs::write(bootstrap.planning_checkpoint_path_for_test(), b"not-json").unwrap();
        } else {
            work.task.task_id.push_str("-different");
        }
        let error = plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |_, _| panic!("fast path must not attach"),
        )
        .unwrap_err();
        if corrupt {
            assert!(matches!(error, NisarSubsetPlanningErrorV2::RecoveryInvalid));
        } else {
            assert!(matches!(
                error,
                NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch
            ));
        }
    }
}

#[test]
fn different_polarization_chunk_layouts_are_mapped_independently_and_unselected_science_is_absent()
{
    let (case, intent, bootstrap, _source) = concrete_fixture_with_source_mutation(
        "physical-different-layouts",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
        |file| {
            let path = "/science/LSAR/RSLC/swaths/frequencyA/HV";
            file.unlink(path).unwrap();
            let values = (0..16)
                .map(|index| TestCFloat32 {
                    r: index as f32,
                    i: -(index as f32),
                })
                .collect::<Vec<_>>();
            file.group("/science/LSAR/RSLC/swaths/frequencyA")
                .unwrap()
                .new_dataset::<TestCFloat32>()
                .shape((4, 4))
                .chunk((1, 4))
                .create("HV")
                .and_then(|dataset| dataset.write_raw(&values))
                .unwrap();
        },
    );
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work(&case, &intent),
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    let physical = completed.physical_execution_plan();
    let hh = physical
        .dataset_mappings
        .iter()
        .find(|mapping| mapping.polarization == "HH")
        .unwrap();
    let hv = physical
        .dataset_mappings
        .iter()
        .find(|mapping| mapping.polarization == "HV")
        .unwrap();
    assert_eq!(hh.chunk_shape, [2, 2]);
    assert_eq!(hv.chunk_shape, [1, 4]);
    assert_ne!(hh.windows[0].window_sha256, hv.windows[0].window_sha256);
    assert_eq!(
        hh.windows[0].component_sha256,
        hv.windows[0].component_sha256
    );
    assert_eq!(
        physical.exact_selected_science_dataset_paths,
        vec![
            "/science/LSAR/RSLC/swaths/frequencyA/HH".to_owned(),
            "/science/LSAR/RSLC/swaths/frequencyA/HV".to_owned(),
        ]
    );
    assert!(physical
        .exact_selected_science_dataset_paths
        .iter()
        .all(|path| !path.ends_with("/RV") && !path.contains("frequencyB")));
}

#[test]
fn byte_identical_science_ranges_deduplicate_and_preserve_all_logical_owners() {
    let (case, intent, bootstrap, _source) = concrete_fixture_with_source_mutation(
        "physical-identical-dedupe",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
        |file| {
            file.unlink("/science/LSAR/RSLC/swaths/frequencyA/HV")
                .unwrap();
            file.link_hard(
                "/science/LSAR/RSLC/swaths/frequencyA/HH",
                "/science/LSAR/RSLC/swaths/frequencyA/HV",
            )
            .unwrap();
        },
    );
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work(&case, &intent),
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    let physical = completed.physical_execution_plan();
    assert_eq!(physical.science_ranges.len(), 4);
    for range in &physical.science_ranges {
        let owners = range
            .logical_owners
            .iter()
            .filter_map(|owner| owner.polarization.as_deref())
            .collect::<BTreeSet<_>>();
        assert_eq!(owners, BTreeSet::from(["HH", "HV"]));
    }
}

#[test]
fn missing_selected_science_fails_before_any_bootstrap_copy() {
    let (case, intent, _bootstrap, _source) = concrete_fixture(
        "physical-selected-missing",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
    );
    let file = hdf5::File::open_rw(&case.source_path).unwrap();
    file.unlink("/science/LSAR/RSLC/swaths/frequencyA/HV")
        .unwrap();
    file.flush().unwrap();
    drop(file);
    let counter = Arc::new(AtomicU64::new(0));
    let bootstrap = NisarSubsetLocalHdf5MetadataBootstrapV2::new_for_test(
        case.source_path.clone(),
        fs::metadata(&case.source_path).unwrap().len(),
        Hdf5RangeWorkspaceV1::new(case._root.0.join("missing-bootstrap")).unwrap(),
    )
    .unwrap()
    .with_copy_counter_for_test(Arc::clone(&counter));
    let result = plan_and_freeze_nisar_subset_v2_with(
        &work(&case, &intent),
        &WorkerControl::default(),
        &bootstrap,
        |_, _| panic!("missing science must fail before PlanReady"),
    );
    assert!(matches!(
        result,
        Err(NisarSubsetPlanningErrorV2::SelectedScienceMissing)
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

#[test]
fn durable_workspace_reopens_across_owner_restart_and_rejects_path_replacement() {
    for (label, replace_workspace) in [
        ("physical-reopen-new-owner", false),
        ("physical-reopen-replaced", true),
    ] {
        let (case, intent, bootstrap, _source) =
            concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
        let mut work = work(&case, &intent);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        let digest = completed
            .physical_execution_plan()
            .physical_plan_sha256()
            .to_owned();
        embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan())
            .unwrap();
        let workspace_root = durable_test_path(bootstrap.workspace.part_path().parent().unwrap());
        drop(bootstrap);
        if replace_workspace {
            let replacement_source = case._root.0.join(format!("{label}-old-workspace"));
            fs::rename(&workspace_root, &replacement_source).unwrap();
            fs::create_dir(&workspace_root).unwrap();
            fs::copy(
                replacement_source.join("planning-workspace-v2.json"),
                workspace_root.join("planning-workspace-v2.json"),
            )
            .unwrap();
        }
        let reopened = NisarSubsetLocalHdf5MetadataBootstrapV2::new_for_test(
            case.source_path.clone(),
            fs::metadata(&case.source_path).unwrap().len(),
            Hdf5RangeWorkspaceV1::new(workspace_root.to_path_buf()).unwrap(),
        )
        .unwrap();
        let result = plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            &reopened,
            |_, _| panic!("fast path must not attach"),
        );
        if replace_workspace {
            assert!(matches!(
                result,
                Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch)
            ));
        } else {
            let fast = result.unwrap();
            assert!(fast.planning_workspace_checkpoint_reused());
            assert_eq!(
                fast.physical_execution_plan().physical_plan_sha256(),
                digest
            );
        }
    }
}

#[test]
fn durable_workspace_rejects_changed_validator_without_science_io() {
    let (case, intent, bootstrap, _source) = concrete_fixture(
        "physical-reopen-validator-change",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let workspace_root = durable_test_path(bootstrap.workspace.part_path().parent().unwrap());
    drop(bootstrap);
    OpenOptions::new()
        .append(true)
        .open(&case.source_path)
        .unwrap()
        .write_all(&[0])
        .unwrap();
    let reopened = NisarSubsetLocalHdf5MetadataBootstrapV2::new_for_test(
        case.source_path.clone(),
        fs::metadata(&case.source_path).unwrap().len(),
        Hdf5RangeWorkspaceV1::new(workspace_root.to_path_buf()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        plan_and_freeze_nisar_subset_v2_with(
            &work,
            &WorkerControl::default(),
            &reopened,
            |_, _| panic!("fast path must not attach"),
        ),
        Err(NisarSubsetPlanningErrorV2::ValidatorChanged)
    ));
}

#[test]
fn durable_fast_path_rejects_missing_replaced_truncated_and_corrupt_sparse_part() {
    for (label, mutation, expected) in [
        (
            "physical-part-missing",
            "missing",
            NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing,
        ),
        (
            "physical-part-replaced",
            "replaced",
            NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch,
        ),
        (
            "physical-part-truncated",
            "truncated",
            NisarSubsetPlanningErrorV2::RecoveryInvalid,
        ),
        (
            "physical-part-corrupt",
            "corrupt",
            NisarSubsetPlanningErrorV2::RecoveryInvalid,
        ),
    ] {
        let (case, intent, bootstrap, _source) =
            concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
        let mut frozen_work = work(&case, &intent);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &frozen_work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        embed_nisar_subset_plan_v2_envelope(
            &mut frozen_work.item.metadata,
            completed.frozen_plan(),
        )
        .unwrap();
        let part_path = bootstrap.workspace.part_path();
        let source_size = completed.bootstrap_statistics().source_size();
        match mutation {
            "missing" => fs::remove_file(&part_path).unwrap(),
            "replaced" => {
                let old = part_path.with_extension("part.old");
                fs::rename(&part_path, old).unwrap();
                fs::copy(&case.source_path, &part_path).unwrap();
            }
            "truncated" => OpenOptions::new()
                .write(true)
                .open(&part_path)
                .unwrap()
                .set_len(source_size - 1)
                .unwrap(),
            "corrupt" => {
                let mut part = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&part_path)
                    .unwrap();
                let mut byte = [0_u8; 1];
                part.read_exact(&mut byte).unwrap();
                part.seek(SeekFrom::Start(0)).unwrap();
                part.write_all(&[byte[0] ^ 0xff]).unwrap();
                part.sync_all().unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            plan_and_freeze_nisar_subset_v2_with(
                &frozen_work,
                &WorkerControl::default(),
                bootstrap.as_ref(),
                |_, _| panic!("invalid sparse part must not attach"),
            )
            .unwrap_err(),
            expected
        );
    }
}

#[test]
fn coherent_checkpoint_accounting_rewrite_is_rejected_by_external_anchor() {
    let (case, intent, bootstrap, _source) = concrete_fixture(
        "physical-accounting-coherent-rewrite",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut frozen_work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &frozen_work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut frozen_work.item.metadata, completed.frozen_plan())
        .unwrap();
    let checkpoint_path = bootstrap.planning_checkpoint_path_for_test();
    let text = fs::read_to_string(&checkpoint_path).unwrap();
    let mut checkpoint: NisarSubsetPlanningWorkspaceCheckpointV2 =
        serde_json::from_str(&text).unwrap();
    let removed = checkpoint
        .bootstrap_statistics
        .preflight
        .requested_ranges
        .pop()
        .unwrap();
    let removed_bytes = removed.len();
    checkpoint.bootstrap_statistics.preflight.request_count -= 1;
    checkpoint
        .bootstrap_statistics
        .preflight
        .response_body_bytes -= removed_bytes;
    checkpoint.bootstrap_statistics.total_planning_request_count -= 1;
    checkpoint
        .bootstrap_statistics
        .total_planning_response_body_bytes -= removed_bytes;
    checkpoint
        .physical_plan
        .combined_budget
        .committed_planning_request_count -= 1;
    checkpoint
        .physical_plan
        .combined_budget
        .committed_planning_response_body_bytes -= removed_bytes;
    checkpoint
        .physical_plan
        .combined_budget
        .remaining_science_request_count += 1;
    checkpoint
        .physical_plan
        .combined_budget
        .remaining_science_response_body_bytes += removed_bytes;
    checkpoint.physical_plan.physical_plan_sha256 = checkpoint
        .physical_plan
        .recompute_physical_plan_sha256()
        .unwrap();
    checkpoint.checkpoint_sha256 = checkpoint.recompute_sha256().unwrap();
    fs::write(&checkpoint_path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();

    assert_eq!(
        plan_and_freeze_nisar_subset_v2_with(
            &frozen_work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |_, _| panic!("rewritten accounting must not attach"),
        )
        .unwrap_err(),
        NisarSubsetPlanningErrorV2::RecoveryInvalid
    );
}

#[test]
fn coherent_empty_science_plan_rewrite_is_rejected_by_source_authority() {
    let (case, intent, bootstrap, _source) = concrete_fixture(
        "physical-empty-science-coherent-rewrite",
        NisarSubsetFrequency::A,
        &["HH"],
    );
    let mut frozen_work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &frozen_work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut frozen_work.item.metadata, completed.frozen_plan())
        .unwrap();
    let checkpoint_path = bootstrap.planning_checkpoint_path_for_test();
    let mut checkpoint: NisarSubsetPlanningWorkspaceCheckpointV2 =
        serde_json::from_str(&fs::read_to_string(&checkpoint_path).unwrap()).unwrap();
    checkpoint.physical_plan.science_ranges.clear();
    checkpoint.physical_plan.estimated_science_response_bytes = 0;
    checkpoint
        .physical_plan
        .combined_budget
        .projected_science_request_count = 0;
    checkpoint
        .physical_plan
        .combined_budget
        .projected_science_response_body_bytes = 0;
    checkpoint.physical_plan.physical_plan_sha256 = checkpoint
        .physical_plan
        .recompute_physical_plan_sha256()
        .unwrap();
    checkpoint.checkpoint_sha256 = checkpoint.recompute_sha256().unwrap();
    fs::write(&checkpoint_path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();

    assert_eq!(
        plan_and_freeze_nisar_subset_v2_with(
            &frozen_work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |_, _| panic!("empty rewritten science plan must not attach"),
        )
        .unwrap_err(),
        NisarSubsetPlanningErrorV2::RecoveryInvalid
    );
}

#[test]
fn coherent_window_owner_and_derived_rewrites_are_rejected_by_source_authority() {
    for mutation in ["window", "owner", "derived"] {
        let label = format!("physical-{mutation}-coherent-rewrite");
        let (case, intent, bootstrap, _source) =
            concrete_fixture(&label, NisarSubsetFrequency::A, &["HH"]);
        let mut frozen_work = work(&case, &intent);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &frozen_work,
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| Ok(ready.proposed_plan),
        )
        .unwrap();
        embed_nisar_subset_plan_v2_envelope(
            &mut frozen_work.item.metadata,
            completed.frozen_plan(),
        )
        .unwrap();
        let checkpoint_path = bootstrap.planning_checkpoint_path_for_test();
        let mut checkpoint: NisarSubsetPlanningWorkspaceCheckpointV2 =
            serde_json::from_str(&fs::read_to_string(&checkpoint_path).unwrap()).unwrap();
        match mutation {
            "window" => {
                let mapping = checkpoint
                    .physical_plan
                    .dataset_mappings
                    .first_mut()
                    .unwrap();
                let window = mapping.windows.first_mut().unwrap();
                let old_window_sha256 = window.window_sha256.clone();
                window.line_end_exclusive -= 1;
                window.window_sha256 = digest_serializable(&(
                    &window.component_sha256,
                    &mapping.source_dataset_path,
                    window.line_start,
                    window.line_end_exclusive,
                    window.sample_start,
                    window.sample_end_exclusive,
                    mapping.dataset_shape,
                    mapping.chunk_shape,
                ))
                .unwrap();
                for owner in checkpoint
                    .physical_plan
                    .science_ranges
                    .iter_mut()
                    .flat_map(|range| &mut range.logical_owners)
                    .filter(|owner| {
                        owner.window_sha256.as_deref() == Some(old_window_sha256.as_str())
                    })
                {
                    owner.window_sha256 = Some(window.window_sha256.clone());
                }
            }
            "owner" => {
                let owner = checkpoint
                    .physical_plan
                    .science_ranges
                    .first_mut()
                    .unwrap()
                    .logical_owners
                    .first_mut()
                    .unwrap();
                owner.logical_chunk.as_mut().unwrap()[0] += 1;
            }
            "derived" => {
                checkpoint.physical_plan.estimated_derived_artifact_bytes += 1;
            }
            _ => unreachable!(),
        }
        checkpoint.physical_plan.physical_plan_sha256 = checkpoint
            .physical_plan
            .recompute_physical_plan_sha256()
            .unwrap();
        checkpoint.checkpoint_sha256 = checkpoint.recompute_sha256().unwrap();
        fs::write(&checkpoint_path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();

        assert_eq!(
            plan_and_freeze_nisar_subset_v2_with(
                &frozen_work,
                &WorkerControl::default(),
                bootstrap.as_ref(),
                |_, _| panic!("rewritten physical plan must not attach"),
            )
            .unwrap_err(),
            NisarSubsetPlanningErrorV2::RecoveryInvalid,
            "mutation {mutation} must fail closed"
        );
    }
}

#[test]
fn selected_science_group_is_typed_missing_before_any_copy() {
    let (case, intent, bootstrap, _source) = concrete_fixture_with_source_mutation(
        "physical-selected-group",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
        |file| {
            file.unlink("/science/LSAR/RSLC/swaths/frequencyA/HV")
                .unwrap();
            file.group("/science/LSAR/RSLC/swaths/frequencyA")
                .unwrap()
                .create_group("HV")
                .unwrap();
        },
    );
    let counter = Arc::new(AtomicU64::new(0));
    let bootstrap = Arc::try_unwrap(bootstrap)
        .ok()
        .unwrap()
        .with_copy_counter_for_test(Arc::clone(&counter));
    assert_eq!(
        plan_and_freeze_nisar_subset_v2_with(
            &work(&case, &intent),
            &WorkerControl::default(),
            &bootstrap,
            |_, _| panic!("non-dataset science path must not freeze"),
        )
        .unwrap_err(),
        NisarSubsetPlanningErrorV2::SelectedScienceMissing
    );
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

#[test]
fn dataset_backed_frozen_dependency_must_resolve_and_have_allocation() {
    const DEPENDENCY: &str =
        "/science/LSAR/RSLC/metadata/processingInformation/parameters/frequencyA/dopplerCentroid";
    for (label, recreate_unallocated) in [
        ("physical-dependency-missing", false),
        ("physical-dependency-unallocated", true),
    ] {
        let (case, intent, bootstrap, _source) =
            concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
        let part_path = bootstrap.workspace.part_path();
        let result = plan_and_freeze_nisar_subset_v2_with(
            &work(&case, &intent),
            &WorkerControl::default(),
            bootstrap.as_ref(),
            |ready, _| {
                assert!(ready
                    .proposed_plan
                    .required_dependencies
                    .items
                    .iter()
                    .any(|item| item.source_path == DEPENDENCY));
                let file = hdf5::File::open_rw(&part_path).unwrap();
                file.unlink(DEPENDENCY).unwrap();
                if recreate_unallocated {
                    file.group(
                        "/science/LSAR/RSLC/metadata/processingInformation/parameters/frequencyA",
                    )
                    .unwrap()
                    .new_dataset::<f64>()
                    .shape((2, 2))
                    .chunk((2, 2))
                    .create("dopplerCentroid")
                    .unwrap();
                }
                file.flush().unwrap();
                drop(file);
                Ok(ready.proposed_plan)
            },
        );
        // Mapping is now checkpointed before Store freeze. Mutation during the
        // acknowledgement must therefore fail recovery integrity before execution.
        assert_eq!(
            result.unwrap_err(),
            if recreate_unallocated {
                NisarSubsetPlanningErrorV2::InventoryMismatch
            } else {
                NisarSubsetPlanningErrorV2::RecoveryInvalid
            }
        );
    }
}

#[test]
fn multipolygon_components_have_exact_deterministic_cross_polarization_coverage() {
    let (case, _intent, bootstrap, _source) = concrete_fixture(
        "physical-multipolygon-components",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
    );
    let intent = prepared_intent_with_requested_geometry(
        &case,
        NisarSubsetFrequency::A,
        &["HH", "HV"],
        json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[36.10, 9.10], [36.30, 9.10], [36.30, 9.30], [36.10, 9.30], [36.10, 9.10]]],
                [[[36.70, 9.70], [36.90, 9.70], [36.90, 9.90], [36.70, 9.90], [36.70, 9.70]]]
            ]
        }),
    );
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work(&case, &intent),
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    assert_eq!(
        completed.frozen_plan().effective_geometry.component_count,
        2
    );
    let physical = completed.physical_execution_plan();
    let component_sets = physical
        .dataset_mappings
        .iter()
        .map(|mapping| {
            mapping
                .windows
                .iter()
                .map(|window| (window.component_id, window.component_sha256.clone()))
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(component_sets.len(), 2);
    assert_eq!(component_sets[0], component_sets[1]);
    assert_eq!(component_sets[0].len(), 2);
    let component_windows = physical
        .dataset_mappings
        .iter()
        .map(|mapping| {
            mapping
                .windows
                .iter()
                .map(|window| {
                    (
                        window.component_id,
                        window.component_sha256.clone(),
                        window.line_start,
                        window.line_end_exclusive,
                        window.sample_start,
                        window.sample_end_exclusive,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(component_windows[0], component_windows[1]);
    assert_eq!(
        component_sets[0]
            .iter()
            .map(|(component_id, _)| *component_id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([0, 1])
    );
    for mapping in &physical.dataset_mappings {
        assert_eq!(mapping.windows.len(), 2);
        assert_eq!(
            mapping
                .windows
                .iter()
                .map(|window| window.window_sha256.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
    }
}

const ROUTE_ETAG: &str = "\"route-v2-etag\"";

#[test]
fn metadata_prefix_v2_partition_is_deterministic_contiguous_and_exact() {
    let ranges =
        partition_metadata_prefix_subranges_v2(crate::NISAR_LIVE_METADATA_PREFIX_BYTES).unwrap();

    assert_eq!(ranges.len(), 12);
    assert_eq!(ranges.first(), Some(&(0, 16 * 1024 * 1024)));
    assert_eq!(ranges.last(), Some(&(11 * 16 * 1024 * 1024, 195_035_136)));
    assert_eq!(ranges.first().unwrap().0, 0);
    assert_eq!(ranges.last().unwrap().1, 195_035_136);
    assert!(ranges
        .windows(2)
        .all(|pair| pair[0].1 == pair[1].0 && pair[0].0 < pair[0].1));
    assert!(ranges.last().unwrap().0 < ranges.last().unwrap().1);
}

#[test]
fn metadata_prefix_v2_rejects_source_smaller_than_frozen_logical_prefix() {
    assert_eq!(
        production_metadata_prefix_ranges_v2(crate::NISAR_LIVE_METADATA_PREFIX_BYTES - 1),
        Err(NisarSubsetPlanningErrorV2::BootstrapResponseInvalid)
    );
    for source_size in [
        crate::NISAR_LIVE_METADATA_PREFIX_BYTES,
        crate::NISAR_LIVE_METADATA_PREFIX_BYTES + 1,
    ] {
        let ranges = production_metadata_prefix_ranges_v2(source_size).unwrap();
        assert_eq!(ranges.first(), Some(&(0, 16 * 1024 * 1024)));
        assert_eq!(ranges.last().map(|range| range.1), Some(195_035_136));
        assert_eq!(
            ranges.iter().map(|range| range.1 - range.0).sum::<u64>(),
            195_035_136
        );
    }
}

#[test]
fn ssar_v1_object_header_accepts_total_message_count_spanning_continuation() {
    let root = PrefixLedgerTestRoot::new("ssar-v1-continuation-count");
    let path = root.0.join("sparse.h5");
    let source_size = 1024_u64;
    let object_addr = 128_u64;
    let continuation_addr = 512_u64;
    let continuation_size = 8_u64;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.set_len(source_size).unwrap();

    let mut header = Vec::new();
    header.push(1); // v1 object header
    header.push(0);
    header.extend_from_slice(&2_u16.to_le_bytes()); // total includes continuation messages
    header.extend_from_slice(&1_u32.to_le_bytes());
    header.extend_from_slice(&24_u32.to_le_bytes()); // one continuation message in this chunk
    header.extend_from_slice(&0_u32.to_le_bytes());
    header.extend_from_slice(&0x0010_u16.to_le_bytes());
    header.extend_from_slice(&16_u16.to_le_bytes());
    header.extend_from_slice(&[0_u8; 4]);
    header.extend_from_slice(&continuation_addr.to_le_bytes());
    header.extend_from_slice(&continuation_size.to_le_bytes());
    assert_eq!(header.len(), 40);
    file.seek(SeekFrom::Start(object_addr)).unwrap();
    file.write_all(&header).unwrap();
    file.flush().unwrap();
    drop(file);

    let mut missing = Vec::new();
    let probe = probe_ssar_v1_object_header_v2(
        &path,
        object_addr,
        source_size,
        &[(0, source_size)],
        &mut missing,
    )
    .unwrap();
    let SsarV1ObjectHeaderProbeV2::Ready(metadata) = probe else {
        panic!("expected fully materialized object header");
    };
    assert_eq!(metadata, SsarV1ObjectHeaderMetadataV2::default());
    assert!(missing.is_empty());
}

#[test]
fn ssar_v1_chunk_layout_v3_extracts_reviewed_btree_address() {
    let source_size = 10_000_u64;
    let btree_addr = 4_096_u64;
    let mut payload = vec![3_u8, 2_u8, 3_u8];
    payload.extend_from_slice(&btree_addr.to_le_bytes());
    payload.extend_from_slice(&512_u32.to_le_bytes());
    payload.extend_from_slice(&512_u32.to_le_bytes());
    payload.extend_from_slice(&8_u32.to_le_bytes());

    assert_eq!(
        parse_ssar_v1_chunk_layout_v2(&payload, source_size).unwrap(),
        Some(SsarV1ChunkIndexV2 {
            btree_addr,
            dimensionality: 3,
        })
    );
}

#[test]
fn ssar_v1_chunk_btree_leaf_materializes_only_index_node_not_raw_chunk() {
    let root = PrefixLedgerTestRoot::new("ssar-v1-chunk-btree-leaf");
    let path = root.0.join("sparse.h5");
    let source_size = 4_096_u64;
    let btree_addr = 1_024_u64;
    let raw_chunk_addr = 3_900_u64;
    let node_bytes =
        ssar_v1_chunk_btree_node_bytes_v2(3, NISAR_SSAR_V1_DEFAULT_INDEXED_STORAGE_K_V2).unwrap();
    let mut node = Vec::with_capacity(node_bytes);
    node.extend_from_slice(b"TREE");
    node.push(1); // raw-data chunk tree
    node.push(0); // leaf level
    node.extend_from_slice(&1_u16.to_le_bytes());
    node.extend_from_slice(&u64::MAX.to_le_bytes());
    node.extend_from_slice(&u64::MAX.to_le_bytes());
    node.extend_from_slice(&[0_u8; 32]);
    node.extend_from_slice(&raw_chunk_addr.to_le_bytes());
    node.extend_from_slice(&[0_u8; 32]);
    node.resize(node_bytes, 0);

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.set_len(source_size).unwrap();
    file.seek(SeekFrom::Start(btree_addr)).unwrap();
    file.write_all(&node).unwrap();
    file.flush().unwrap();
    drop(file);

    let material_end = btree_addr + u64::try_from(node_bytes).unwrap();
    let mut missing = Vec::new();
    let mut visited = BTreeSet::new();
    inspect_ssar_v1_chunk_btree_v2(
        &path,
        SsarV1ChunkIndexV2 {
            btree_addr,
            dimensionality: 3,
        },
        NISAR_SSAR_V1_DEFAULT_INDEXED_STORAGE_K_V2,
        source_size,
        &[(btree_addr, material_end)],
        &mut missing,
        &mut visited,
    )
    .unwrap();

    assert!(missing.is_empty());
    assert_eq!(visited, BTreeSet::from([btree_addr]));
}

#[test]
fn ssar_deferred_fixture_metadata_discovery_reaches_physical_catalog_without_science_bytes() {
    let root = PrefixLedgerTestRoot::new("ssar-deferred-discovery");
    let source_path = root.0.join("source.h5");
    let _pin = create_ssar_source_fixture_with_science_layout(&source_path, true, true);
    let source_size = fs::metadata(&source_path).unwrap().len();
    let selected_paths = ["RH", "RV"]
        .map(|polarization| format!("/science/SSAR/RSLC/swaths/frequencyA/{polarization}"));
    let full_catalog = crate::hdf5_range_transfer::inspect_nisar_planning_physical_ranges_v2(
        &source_path,
        &selected_paths,
        NisarScienceRoot::Ssar,
    )
    .unwrap();
    assert!(!full_catalog.selected_science_ranges.is_empty());
    assert!(
        discover_ssar_v1_metadata_ranges_v2(&source_path, source_size, &[(0, source_size)],)
            .unwrap()
            .is_empty()
    );

    let sparse_path = root.0.join("sparse.h5");
    let mut source = File::open(&source_path).unwrap();
    let mut sparse = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&sparse_path)
        .unwrap();
    sparse.set_len(source_size).unwrap();
    let mut material_ranges = remote_planning_initial_material_ranges(source_size).unwrap();
    let mut copy_range = |range: (u64, u64)| {
        let mut remaining = range.1 - range.0;
        let mut buffer = [0_u8; 64 * 1024];
        source.seek(SeekFrom::Start(range.0)).unwrap();
        sparse.seek(SeekFrom::Start(range.0)).unwrap();
        while remaining > 0 {
            let count = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
            source.read_exact(&mut buffer[..count]).unwrap();
            sparse.write_all(&buffer[..count]).unwrap();
            remaining -= count as u64;
        }
    };
    for range in material_ranges.iter().copied() {
        copy_range(range);
    }
    sparse.sync_all().unwrap();
    drop(sparse);

    let mut converged = false;
    for _ in 0..NISAR_SSAR_METADATA_DISCOVERY_MAX_PASSES_V2 {
        let missing =
            discover_ssar_v1_metadata_ranges_v2(&sparse_path, source_size, &material_ranges)
                .unwrap();
        if missing.is_empty() {
            converged = true;
            break;
        }
        assert!(missing.iter().all(|range| {
            full_catalog
                .all_science_ranges
                .iter()
                .all(|science| !tuple_ranges_overlap(*range, *science))
        }));
        let mut sparse = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&sparse_path)
            .unwrap();
        for range in missing.iter().copied() {
            let mut remaining = range.1 - range.0;
            let mut buffer = [0_u8; 64 * 1024];
            source.seek(SeekFrom::Start(range.0)).unwrap();
            sparse.seek(SeekFrom::Start(range.0)).unwrap();
            while remaining > 0 {
                let count = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
                source.read_exact(&mut buffer[..count]).unwrap();
                sparse.write_all(&buffer[..count]).unwrap();
                remaining -= count as u64;
            }
        }
        sparse.sync_all().unwrap();
        drop(sparse);
        material_ranges.extend(missing);
        material_ranges = canonical_material_ranges_v2(material_ranges, source_size).unwrap();
    }
    assert!(converged);

    let sparse_catalog = crate::hdf5_range_transfer::inspect_nisar_planning_physical_ranges_v2(
        &sparse_path,
        &selected_paths,
        NisarScienceRoot::Ssar,
    )
    .unwrap();
    assert_eq!(
        sparse_catalog.selected_science_ranges,
        full_catalog.selected_science_ranges
    );
}

#[test]
fn bootstrap_ledger_allows_disjoint_materialization_in_discovery_order() {
    let mut ledger = BootstrapLedger::new(1_000, "a".repeat(64), vec![(900, 950)]).unwrap();

    let later = ledger
        .reserve(BootstrapCategory::MetadataDiscovery, 700, 720, 20)
        .unwrap();
    ledger.commit(later, 20).unwrap();

    let earlier = ledger
        .reserve(BootstrapCategory::AdditionalGeolocation, 300, 340, 40)
        .unwrap();
    ledger.commit(earlier, 40).unwrap();

    assert_eq!(ledger.events.len(), 2);
    assert_eq!(ledger.events[0].range.start, 700);
    assert_eq!(ledger.events[1].range.start, 300);
}

#[test]
#[ignore = "requires INSAR_SSAR_SPARSE_PATH pointing at preserved real SSAR sparse evidence"]
fn ssar_real_sparse_metadata_discovery_probe() {
    let path =
        PathBuf::from(std::env::var_os("INSAR_SSAR_SPARSE_PATH").expect("INSAR_SSAR_SPARSE_PATH"));
    let source_size = fs::metadata(&path).unwrap().len();
    let material_ranges = production_metadata_prefix_ranges_v2(source_size).unwrap();
    let file = hdf5::File::open(&path).expect("open preserved SSAR sparse HDF5");
    let science = file.group("/science").expect("open /science");
    let ssar_addr = ssar_hard_link_object_address_v2(&file, &science, "SSAR")
        .unwrap_or_else(|error| panic!("SSAR_ADDRESS_ERROR={error:?}"));
    eprintln!("SSAR_OBJECT_ADDR={ssar_addr}");

    let mut object_missing = Vec::new();
    match probe_ssar_v1_object_header_v2(
        &path,
        ssar_addr,
        source_size,
        &material_ranges,
        &mut object_missing,
    )
    .unwrap_or_else(|error| panic!("SSAR_OBJECT_HEADER_ERROR={error:?}"))
    {
        SsarV1ObjectHeaderProbeV2::Missing => {
            eprintln!("SSAR_OBJECT_HEADER_MISSING={object_missing:?}");
        }
        SsarV1ObjectHeaderProbeV2::Ready(metadata) => {
            eprintln!("SSAR_OBJECT_HEADER_READY={metadata:?}");
        }
    }

    let mut group_missing = Vec::new();
    let group_ready = inspect_ssar_v1_group_structure_v2(
        &path,
        ssar_addr,
        source_size,
        &material_ranges,
        &mut group_missing,
    )
    .unwrap_or_else(|error| panic!("SSAR_GROUP_STRUCTURE_ERROR={error:?}"));
    eprintln!("SSAR_GROUP_STRUCTURE_READY={group_ready} MISSING={group_missing:?}");

    let mut queue = VecDeque::from([("/science/SSAR".to_owned(), ssar_addr)]);
    let mut visited = BTreeSet::new();
    let mut traversal_missing = Vec::new();
    while let Some((group_path, object_addr)) = queue.pop_front() {
        if !visited.insert(object_addr) {
            continue;
        }
        eprintln!("SSAR_TRAVERSE_GROUP path={group_path} addr={object_addr}");
        let ready = inspect_ssar_v1_group_structure_v2(
            &path,
            object_addr,
            source_size,
            &material_ranges,
            &mut traversal_missing,
        )
        .unwrap_or_else(|error| {
            panic!("SSAR_TRAVERSE_GROUP_ERROR path={group_path} addr={object_addr} error={error:?}")
        });
        eprintln!(
            "SSAR_TRAVERSE_GROUP_STATE path={group_path} ready={ready} missing={traversal_missing:?}"
        );
        if !ready {
            continue;
        }
        let group = file.group(&group_path).unwrap_or_else(|error| {
            panic!("SSAR_OPEN_GROUP_ERROR path={group_path} error={error:?}")
        });
        let member_names = group.member_names().unwrap_or_else(|error| {
            panic!("SSAR_MEMBER_NAMES_ERROR path={group_path} error={error:?}")
        });
        eprintln!("SSAR_MEMBERS path={group_path} names={member_names:?}");
        for name in member_names {
            let child_addr =
                ssar_hard_link_object_address_v2(&file, &group, &name).unwrap_or_else(|error| {
                    panic!("SSAR_CHILD_ADDRESS_ERROR path={group_path}/{name} error={error:?}")
                });
            let mut child_missing = Vec::new();
            let child_probe = probe_ssar_v1_object_header_v2(
                &path,
                child_addr,
                source_size,
                &material_ranges,
                &mut child_missing,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "SSAR_CHILD_HEADER_ERROR path={group_path}/{name} addr={child_addr} error={error:?}"
                )
            });
            eprintln!(
                "SSAR_CHILD path={group_path}/{name} addr={child_addr} probe_missing={} missing={child_missing:?}",
                matches!(child_probe, SsarV1ObjectHeaderProbeV2::Missing)
            );
            if matches!(child_probe, SsarV1ObjectHeaderProbeV2::Missing) {
                traversal_missing.extend(child_missing);
                continue;
            }
            let child_path = format!("{group_path}/{name}");
            if file.group(&child_path).is_ok() {
                queue.push_back((child_path, child_addr));
            }
        }
    }
    eprintln!("SSAR_TRAVERSAL_MISSING={traversal_missing:?}");

    let discovered = discover_ssar_v1_metadata_ranges_v2(&path, source_size, &material_ranges)
        .unwrap_or_else(|error| panic!("SSAR_DISCOVERY_ERROR={error:?}"));
    let expected =
        canonical_material_ranges_v2(traversal_missing, source_size).expect("canonical missing");
    eprintln!("SSAR_DISCOVERY_MISSING={discovered:?}");
    assert_eq!(discovered, expected);
    assert!(!discovered.is_empty());
}

#[test]
#[ignore = "requires INSAR_SSAR_SPARSE_PATH pointing at preserved real SSAR sparse evidence"]
fn ssar_real_sparse_metadata_discovery_replay() {
    let path =
        PathBuf::from(std::env::var_os("INSAR_SSAR_SPARSE_PATH").expect("INSAR_SSAR_SPARSE_PATH"));
    let source_size = fs::metadata(&path).unwrap().len();
    let mut material_ranges = production_metadata_prefix_ranges_v2(source_size).unwrap();
    let mut total_ranges = 0_usize;
    let mut total_bytes = 0_u64;

    for pass in 0..NISAR_SSAR_METADATA_DISCOVERY_MAX_PASSES_V2 {
        let missing =
            discover_ssar_v1_metadata_ranges_v2(&path, source_size, &material_ranges).unwrap();
        let pass_bytes = missing.iter().map(|(start, end)| end - start).sum::<u64>();
        eprintln!(
            "SSAR_REPLAY pass={pass} ranges={} bytes={pass_bytes} missing={missing:?}",
            missing.len()
        );
        if missing.is_empty() {
            eprintln!("SSAR_REPLAY_DONE passes={pass} ranges={total_ranges} bytes={total_bytes}");
            return;
        }
        total_ranges += missing.len();
        total_bytes += pass_bytes;
        material_ranges.extend(missing);
        material_ranges = canonical_material_ranges_v2(material_ranges, source_size).unwrap();
    }
    panic!("SSAR_REPLAY_NOT_CONVERGED ranges={total_ranges} bytes={total_bytes}");
}

#[test]
#[ignore = "requires INSAR_SSAR_SPARSE_PATH pointing at preserved real SSAR sparse evidence"]
fn ssar_real_sparse_progressive_catalog_replay() {
    let source_path =
        PathBuf::from(std::env::var_os("INSAR_SSAR_SPARSE_PATH").expect("INSAR_SSAR_SPARSE_PATH"));
    let source_size = fs::metadata(&source_path).unwrap().len();
    let root = PrefixLedgerTestRoot::new("ssar-progressive-catalog");
    let replay_path = root.0.join("ranges.part");
    let replay = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&replay_path)
        .unwrap();
    replay.set_len(source_size).unwrap();
    drop(replay);

    let mut materialized = production_metadata_prefix_ranges_v2(source_size).unwrap();
    let copy_ranges = |ranges: &[(u64, u64)]| {
        let mut output = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&replay_path)
            .unwrap();
        for &(start, end) in ranges {
            let bytes = read_sparse_materialized_range_v2(&source_path, (start, end)).unwrap();
            output.seek(SeekFrom::Start(start)).unwrap();
            output.write_all(&bytes).unwrap();
        }
        output.sync_all().unwrap();
    };
    copy_ranges(&materialized);

    let mut converged = false;
    for pass in 0..NISAR_SSAR_METADATA_DISCOVERY_MAX_PASSES_V2 {
        let missing =
            discover_ssar_v1_metadata_ranges_v2(&replay_path, source_size, &materialized).unwrap();
        eprintln!(
            "SSAR_PROGRESSIVE_REPLAY pass={pass} ranges={} bytes={}",
            missing.len(),
            missing.iter().map(|(start, end)| end - start).sum::<u64>()
        );
        if missing.is_empty() {
            converged = true;
            break;
        }
        copy_ranges(&missing);
        materialized.extend(missing);
        materialized = canonical_material_ranges_v2(materialized, source_size).unwrap();
    }
    assert!(converged);

    let selected_paths = selected_science_paths_from_selection(
        NisarScienceRoot::Ssar,
        &NisarSubsetFrequency::A,
        &["VH".to_owned(), "VV".to_owned()],
    );
    let catalog = crate::hdf5_range_transfer::inspect_nisar_planning_physical_ranges_v2(
        &replay_path,
        &selected_paths,
        NisarScienceRoot::Ssar,
    )
    .unwrap_or_else(|error| panic!("SSAR_PROGRESSIVE_CATALOG error={error:?}"));
    eprintln!(
        "SSAR_PROGRESSIVE_CATALOG_OK selected={} all={} metadata={} geolocation={}",
        catalog.selected_science_ranges.len(),
        catalog.all_science_ranges.len(),
        catalog.metadata_ranges.len(),
        catalog.geolocation_ranges.len()
    );
}

#[test]
#[ignore = "requires INSAR_SSAR_SPARSE_PATH and INSAR_SSAR_PREPARED_INTENT_PATH"]
fn ssar_real_sparse_physical_mapping_probe() {
    let sparse_path =
        PathBuf::from(std::env::var_os("INSAR_SSAR_SPARSE_PATH").expect("INSAR_SSAR_SPARSE_PATH"));
    let intent_path = PathBuf::from(
        std::env::var_os("INSAR_SSAR_PREPARED_INTENT_PATH")
            .expect("INSAR_SSAR_PREPARED_INTENT_PATH"),
    );
    let intent: NisarSubsetPreparedIntentV2 =
        serde_json::from_str(&fs::read_to_string(intent_path).unwrap()).unwrap();
    let source_size = fs::metadata(&sparse_path).unwrap().len();
    let science_root = science_root_for_identity(&intent.source_product_identity).unwrap();
    let selected_paths = selected_science_paths_from_selection(
        science_root,
        &intent.selected_frequency,
        &intent.selected_polarizations,
    );
    let catalog = crate::hdf5_range_transfer::inspect_nisar_planning_physical_ranges_v2(
        &sparse_path,
        &selected_paths,
        science_root,
    )
    .unwrap_or_else(|error| panic!("SSAR_PHYSICAL_PROBE_CATALOG error={error:?}"));
    eprintln!(
        "SSAR_PHYSICAL_PROBE_CATALOG_OK selected_science={} all_science={} metadata={} geolocation={}",
        catalog.selected_science_ranges.len(),
        catalog.all_science_ranges.len(),
        catalog.metadata_ranges.len(),
        catalog.geolocation_ranges.len()
    );
    let mut materialized_ranges = production_metadata_prefix_ranges_v2(source_size).unwrap();
    for _ in 0..NISAR_SSAR_METADATA_DISCOVERY_MAX_PASSES_V2 {
        let missing =
            discover_ssar_v1_metadata_ranges_v2(&sparse_path, source_size, &materialized_ranges)
                .unwrap();
        if missing.is_empty() {
            break;
        }
        materialized_ranges.extend(missing);
        materialized_ranges =
            canonical_material_ranges_v2(materialized_ranges, source_size).unwrap();
    }
    let mut required = Vec::new();
    for &(start, end) in &catalog.metadata_ranges {
        required.extend(
            missing_material_ranges_v2((start, end), &materialized_ranges, source_size).unwrap(),
        );
    }
    for &(start, end) in &catalog.geolocation_ranges {
        required.extend(
            missing_material_ranges_v2((start, end), &materialized_ranges, source_size).unwrap(),
        );
    }
    let required = canonical_material_ranges_v2(required, source_size).unwrap();
    assert!(required.iter().all(|required_range| {
        !catalog
            .all_science_ranges
            .iter()
            .any(|science| tuple_ranges_overlap(*required_range, *science))
    }));
    eprintln!(
        "SSAR_PHYSICAL_PROBE_REQUIRED_METADATA ranges={} bytes={} detail={required:?}",
        required.len(),
        required.iter().map(|(start, end)| end - start).sum::<u64>()
    );
    let probe_file = hdf5::File::open(&sparse_path).unwrap();
    let mission_id = probe_file
        .dataset("/science/SSAR/identification/missionId")
        .unwrap();
    let mission_id_ranges = physical_dataset_ranges_v2(&mission_id).unwrap();
    eprintln!("SSAR_PHYSICAL_PROBE_MISSION_ID_RANGES={mission_id_ranges:?}");
    let root_group = probe_file.group("/").unwrap();
    eprintln!(
        "SSAR_PHYSICAL_PROBE_ROOT_MEMBERS={:?}",
        root_group.member_names().unwrap()
    );
    if root_group.link_exists("complex64") {
        let complex64_addr =
            ssar_hard_link_object_address_v2(&probe_file, &root_group, "complex64").unwrap();
        let prefix_ranges = production_metadata_prefix_ranges_v2(source_size).unwrap();
        let mut complex64_missing = Vec::new();
        let complex64_probe = probe_ssar_v1_object_header_v2(
            &sparse_path,
            complex64_addr,
            source_size,
            &prefix_ranges,
            &mut complex64_missing,
        )
        .unwrap();
        eprintln!(
            "SSAR_PHYSICAL_PROBE_COMPLEX64 addr={complex64_addr} missing={} ranges={complex64_missing:?}",
            matches!(complex64_probe, SsarV1ObjectHeaderProbeV2::Missing)
        );
    }
    drop(root_group);
    drop(probe_file);
    let source_validator = NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: "\"offline-r15-physical-probe\"".to_owned(),
        content_length: source_size,
    };
    let inventory =
        crate::nisar_subset_writer::NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
            &intent,
            source_validator.clone(),
            &sparse_path,
            &|| false,
        )
        .unwrap_or_else(|error| {
            let diagnostic = crate::nisar_subset_writer::metadata_capture_diagnostic_for_test();
            panic!(
                "SSAR_PHYSICAL_PROBE_INVENTORY error={error:?} stage={} path={:?} detail={:?}",
                diagnostic.stage, diagnostic.object_path, diagnostic.detail
            )
        });
    let scale_nodes = inventory
        .datasets()
        .iter()
        .filter(|dataset| {
            dataset.kind()
                == crate::nisar_subset_writer::NisarSubsetVerifiedMetadataDatasetKindV2::DimensionScale
        })
        .map(|dataset| dataset.path().to_owned())
        .collect::<Vec<_>>();
    let scale_refs = inventory
        .datasets()
        .iter()
        .filter(|dataset| {
            dataset
                .dimension_scales()
                .iter()
                .any(|axis| !axis.is_empty())
        })
        .map(|dataset| {
            (
                dataset.path().to_owned(),
                dataset.dimension_scales().to_vec(),
            )
        })
        .collect::<Vec<_>>();
    eprintln!("SSAR_PHYSICAL_PROBE_SCALE_NODES={scale_nodes:?}");
    eprintln!("SSAR_PHYSICAL_PROBE_SCALE_REFS={scale_refs:?}");
    let graph =
        crate::nisar_subset_writer::build_nisar_subset_source_dependency_graph_v2(&inventory)
            .unwrap_or_else(|error| panic!("SSAR_PHYSICAL_PROBE_GRAPH error={error:?}"));
    let frozen_plan = NisarSubsetPlanV2::build_from_prepared_intent(
        intent.clone(),
        graph,
        NisarSubsetPlanMetadataEvidenceV2 {
            source_estimate: Some(NisarSubsetByteEstimateV2 {
                bytes: source_size,
                evidence_sha256: inventory.evidence().evidence_sha256().to_owned(),
            }),
            transfer_estimate: None,
            derived_estimate: None,
            warnings: vec![
                "Physical subset byte estimates are deferred until verified window/chunk mapping."
                    .to_owned(),
            ],
        },
    )
    .unwrap();
    let science_root = science_root_for_identity(&frozen_plan.source_product_identity).unwrap();
    assert_eq!(science_root, NisarScienceRoot::Ssar);
    let file = hdf5::File::open(&sparse_path).unwrap();
    let geolocation =
        read_physical_geolocation_inputs_v2(&file, science_root, &frozen_plan.selected_frequency)
            .unwrap_or_else(|error| panic!("SSAR_PHYSICAL_PROBE_GEO error={error:?}"));
    eprintln!(
        "SSAR_PHYSICAL_PROBE_GEO_OK grid_time={} grid_range={} swath_time={} swath_range={} layers={}",
        geolocation.grid_zero_doppler_time.len(),
        geolocation.grid_slant_range.len(),
        geolocation.swath_zero_doppler_time.len(),
        geolocation.swath_slant_range.len(),
        geolocation.height_layers_m.len()
    );
    for (label, grid, swath) in [
        (
            "time",
            geolocation.grid_zero_doppler_time.as_slice(),
            geolocation.swath_zero_doppler_time.as_slice(),
        ),
        (
            "range",
            geolocation.grid_slant_range.as_slice(),
            geolocation.swath_slant_range.as_slice(),
        ),
    ] {
        let grid_inc = grid.windows(2).all(|pair| pair[0] < pair[1]);
        let grid_dec = grid.windows(2).all(|pair| pair[0] > pair[1]);
        let swath_inc = swath.windows(2).all(|pair| pair[0] < pair[1]);
        let swath_dec = swath.windows(2).all(|pair| pair[0] > pair[1]);
        let mapping = insar_provider::asf::nisar_subset::nearest_radar_axis_indices(grid, swath);
        eprintln!(
            "SSAR_PHYSICAL_PROBE_AXIS label={label} grid_first={} grid_last={} grid_inc={grid_inc} grid_dec={grid_dec} swath_first={} swath_last={} swath_inc={swath_inc} swath_dec={swath_dec} mapping={:?}",
            grid[0],
            grid[grid.len() - 1],
            swath[0],
            swath[swath.len() - 1],
            mapping.as_ref().map(|(indices, diagnostics)| (
                indices.len(),
                diagnostics.mapped_first_index,
                diagnostics.mapped_last_index,
                diagnostics.margin_nodes_dropped,
                diagnostics.max_nearest_residual,
            ))
        );
    }

    let mut science_candidates = Vec::new();
    let mut selected_windows = Vec::new();
    let mut window_count = 0_usize;
    for selected in inventory.selected_science() {
        let dataset_path = selected.source_path().to_owned();
        let dataset = file.dataset(&dataset_path).unwrap_or_else(|error| {
            panic!("SSAR_PHYSICAL_PROBE_DATASET path={dataset_path} error={error:?}")
        });
        let shape = dataset.shape();
        let chunk = dataset.dcpl().unwrap().get_chunk().unwrap().unwrap();
        let chunk_shape = [chunk[0] as u64, chunk[1] as u64];
        let evidence = insar_provider::asf::nisar_subset::build_live_geolocation_evidence(
            &frozen_plan.effective_geometry.canonical_geojson,
            frozen_plan.selected_frequency.clone(),
            selected.polarization(),
            science_root.swath(),
            &geolocation.grid_zero_doppler_time,
            &geolocation.grid_slant_range,
            &geolocation.swath_zero_doppler_time,
            &geolocation.swath_slant_range,
            geolocation.height_layers_m.clone(),
            geolocation.longitudes.clone(),
            geolocation.latitudes.clone(),
            shape.clone().try_into().unwrap(),
            chunk.clone().try_into().unwrap(),
            geolocation.minimum_hae_m,
            geolocation.maximum_hae_m,
        )
        .unwrap_or_else(|error| {
            panic!(
                "SSAR_PHYSICAL_PROBE_WINDOW polarization={} error={error:?}",
                selected.polarization()
            )
        });
        for window in &evidence.windows {
            eprintln!(
                "SSAR_PHYSICAL_PROBE_WINDOW_OK polarization={} component={} line={}..{} sample={}..{}",
                selected.polarization(),
                window.component_id,
                window.line_start,
                window.line_end_exclusive,
                window.sample_start,
                window.sample_end_exclusive
            );
            window_count += 1;
            let physical_window = NisarSubsetPhysicalWindowV2 {
                component_id: window.component_id,
                component_sha256: digest_serializable(&(
                    &frozen_plan.canonical_geometry_digest.effective_sha256,
                    window.component_id,
                ))
                .unwrap(),
                window_sha256: "0".repeat(64),
                line_start: window.line_start,
                line_end_exclusive: window.line_end_exclusive,
                sample_start: window.sample_start,
                sample_end_exclusive: window.sample_end_exclusive,
            };
            selected_windows.push(physical_window.clone());
            collect_science_chunk_candidates_v2(
                &dataset,
                &physical_window,
                selected.polarization(),
                &dataset_path,
                chunk_shape,
                source_size,
                &mut science_candidates,
                &|| false,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "SSAR_PHYSICAL_PROBE_CHUNKS polarization={} window={:?} error={error:?}",
                    selected.polarization(),
                    (
                        window.line_start,
                        window.line_end_exclusive,
                        window.sample_start,
                        window.sample_end_exclusive
                    )
                )
            });
        }
    }
    eprintln!(
        "SSAR_PHYSICAL_PROBE_CHUNKS_OK windows={window_count} candidates={}",
        science_candidates.len()
    );
    let science_ranges = canonicalize_physical_candidates_v2(
        science_candidates,
        source_size,
        NISAR_LIVE_MAX_REQUESTS as usize,
    )
    .unwrap_or_else(|error| panic!("SSAR_PHYSICAL_PROBE_CANONICAL error={error:?}"));
    eprintln!(
        "SSAR_PHYSICAL_PROBE_SCIENCE_OK ranges={} bytes={}",
        science_ranges.len(),
        science_ranges
            .iter()
            .map(|range| range.end_exclusive - range.start)
            .sum::<u64>()
    );
    let selected_window_refs = selected_windows.iter().collect::<Vec<_>>();
    for dependency in &frozen_plan.required_dependencies.items {
        if dependency.kind == NisarSubsetDependencyKindV2::SelectedScienceDataset {
            continue;
        }
        if dependency.kind == NisarSubsetDependencyKindV2::Attribute {
            let root = file.group("/").unwrap();
            let attribute_names = frozen_plan
                .source_dependency_graph
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == NisarSubsetSourceDependencyNodeKindV2::Attribute
                        && node.source_path == dependency.source_path
                })
                .filter_map(|node| node.attribute_name.clone())
                .collect::<Vec<_>>();
            eprintln!(
                "SSAR_PHYSICAL_PROBE_DEPENDENCY_ATTRIBUTE path={} names={attribute_names:?} present={}",
                dependency.source_path,
                attribute_names.iter().all(|name| root.attr(name).is_ok())
            );
            continue;
        }
        let dataset = file
            .dataset(&dependency.source_path)
            .unwrap_or_else(|error| {
                panic!(
                    "SSAR_PHYSICAL_PROBE_DEPENDENCY_OPEN path={} kind={:?} error={error:?}",
                    dependency.source_path, dependency.kind
                )
            });
        let ranges = if dependency.kind == NisarSubsetDependencyKindV2::ValidityMask {
            physical_dataset_ranges_for_windows_v2(&dataset, &selected_window_refs, source_size)
        } else {
            physical_dataset_ranges_v2(&dataset)
        }
        .unwrap_or_else(|error| {
            panic!(
                "SSAR_PHYSICAL_PROBE_DEPENDENCY_RANGES path={} kind={:?} error={error:?}",
                dependency.source_path, dependency.kind
            )
        });
        eprintln!(
            "SSAR_PHYSICAL_PROBE_DEPENDENCY_OK path={} kind={:?} ranges={} bytes={}",
            dependency.source_path,
            dependency.kind,
            ranges.len(),
            ranges.iter().map(|(start, end)| end - start).sum::<u64>()
        );
    }
}

/// The pathname a test may keep after the workspace is dropped. On Linux a
/// pinned workspace names its files through its open directory descriptor
/// (`/proc/self/fd/<n>/...`), which ends with that descriptor; this resolves
/// it to the directory's real path while it is still open. Other paths are
/// returned unchanged.
fn durable_test_path(path: &Path) -> PathBuf {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if let Ok(rest) = path.strip_prefix("/proc/self/fd") {
        let mut components = rest.components();
        if let Some(descriptor) = components.next() {
            let directory = fs::read_link(Path::new("/proc/self/fd").join(descriptor)).unwrap();
            return directory.join(components.as_path());
        }
    }
    path.to_path_buf()
}

struct PrefixLedgerTestRoot(PathBuf);

impl PrefixLedgerTestRoot {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "insar-nisar-prefix-ledger-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for PrefixLedgerTestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn prefix_validator_digest(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn prefix_retry_failure_observation(
    range: (u64, u64),
    source_size: u64,
    response_body_bytes: u64,
) -> NisarSubsetRangeFailureObservationV2 {
    NisarSubsetRangeFailureObservationV2 {
        stage: "metadata_prefix".to_owned(),
        start: range.0,
        end_exclusive: range.1,
        source_size_bytes: source_size,
        disposition: "sent_outcome_indeterminate",
        request_count: 1,
        http_status: Some(206),
        response_body_bytes: Some(response_body_bytes),
        transport_diagnostic: None,
        validator_sha256: None,
        failure_code: "NISAR_SUBSET_V2_PARTIAL_RESPONSE",
        redirect_occurred: false,
        redirect_hop_index: None,
        redirect_source_host_class: None,
        redirect_destination_host_class: None,
        redirect_source_path_class: None,
        redirect_destination_path_class: None,
        redirect_same_origin: None,
        redirect_scheme_transition: None,
        redirect_policy_result: None,
        redirect_rejection_reason: None,
    }
}

fn prefix_terminal_failure_observation(
    range: (u64, u64),
    source_size: u64,
) -> NisarSubsetRangeFailureObservationV2 {
    let mut observation = prefix_retry_failure_observation(range, source_size, 0);
    observation.http_status = Some(404);
    observation.response_body_bytes = None;
    observation.failure_code = "NISAR_SUBSET_V2_SOURCE_UNAVAILABLE";
    observation
}

#[test]
fn nisar_v2_durable_prefix_restore_drops_runtime_transport_diagnostic_without_hash_change() {
    let mut with_diagnostic = prefix_retry_failure_observation((0, 4), 12, 0);
    with_diagnostic.http_status = Some(503);
    with_diagnostic.response_body_bytes = None;
    with_diagnostic.failure_code = "NISAR_SUBSET_V2_TRANSPORT_FAILED";
    with_diagnostic.transport_diagnostic = Some(NisarSubsetTransportDiagnosticV2::ExactRangeSend(
        crate::http_transfer::ExactRangeTransportCause::RetryableHttpStatus,
    ));
    let mut without_diagnostic = with_diagnostic.clone();
    without_diagnostic.transport_diagnostic = None;

    let durable_with = NisarPrefixDurableFailureObservationV2::from_runtime(&with_diagnostic);
    let durable_without = NisarPrefixDurableFailureObservationV2::from_runtime(&without_diagnostic);
    assert_eq!(durable_with, durable_without);
    assert_eq!(
        serde_json::to_string(&durable_with).unwrap(),
        serde_json::to_string(&durable_without).unwrap()
    );

    let checkpoint = |observation: NisarPrefixDurableFailureObservationV2| {
        let mut checkpoint = NisarPrefixMaterializationCheckpointV2 {
            checkpoint_contract: NISAR_PREFIX_MATERIALIZATION_CHECKPOINT_V2_CONTRACT.to_owned(),
            schema_version: NISAR_PREFIX_MATERIALIZATION_CHECKPOINT_V2_SCHEMA_VERSION,
            source_size: 12,
            source_validator_sha256: "a".repeat(64),
            source_identity_sha256: "b".repeat(64),
            planned_ranges: vec![NisarSubsetBootstrapRangeV2::new(0, 4).unwrap()],
            completed_ranges: Vec::new(),
            historical_failure_observations: vec![observation],
            attempt_states: Vec::new(),
            checkpoint_sha256: String::new(),
        };
        checkpoint.checkpoint_sha256 = checkpoint.recompute_sha256().unwrap();
        checkpoint
    };
    let checkpoint_with = checkpoint(durable_with.clone());
    let restored = durable_with.to_runtime().unwrap();
    assert_eq!(restored.transport_diagnostic, None);
    assert_eq!(restored.http_status, Some(503));
    assert_eq!(restored.failure_code, "NISAR_SUBSET_V2_TRANSPORT_FAILED");
    let durable_after_restore = NisarPrefixDurableFailureObservationV2::from_runtime(&restored);
    assert_eq!(durable_after_restore, durable_without);
    let checkpoint_without = checkpoint(durable_after_restore);
    assert_eq!(
        checkpoint_with.checkpoint_sha256,
        checkpoint_without.checkpoint_sha256
    );
    assert_eq!(
        checkpoint_with.canonical_json().unwrap(),
        checkpoint_without.canonical_json().unwrap()
    );
}

const PREFIX_SOURCE_IDENTITY_SHA256: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ALTERNATE_PREFIX_SOURCE_IDENTITY_SHA256: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn prefix_test_ranges(end_exclusive: u64, chunk_bytes: u64) -> Vec<(u64, u64)> {
    let mut ranges = Vec::new();
    let mut start = 0_u64;
    while start < end_exclusive {
        let end = start.saturating_add(chunk_bytes).min(end_exclusive);
        ranges.push((start, end));
        start = end;
    }
    ranges
}

#[test]
fn metadata_prefix_v2_partial_middle_chunk_preserves_only_prior_verified_chunks() {
    let root = PrefixLedgerTestRoot::new("partial-middle");
    let workspace = Hdf5RangeWorkspaceV1::new(root.0.join("workspace")).unwrap();
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &workspace,
        12,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();

    session.materialize_verified_range((0, 4), b"abcd").unwrap();
    session.materialize_verified_range((4, 8), b"efgh").unwrap();
    assert_eq!(
        session.materialize_verified_range((8, 12), b"ijk"),
        Err(NisarSubsetPlanningErrorV2::BootstrapPartialResponse)
    );
    drop(session);

    let checkpoint: Value = serde_json::from_str(
        &fs::read_to_string(workspace.planning_prefix_checkpoint_path_v2_for_test()).unwrap(),
    )
    .unwrap();
    let completed = checkpoint["completed_ranges"].as_array().unwrap();
    assert_eq!(completed.len(), 2);
    assert_eq!(completed[0]["start"], json!(0));
    assert_eq!(completed[0]["end_exclusive"], json!(4));
    assert_eq!(completed[0]["response_bytes"], json!(4));
    assert_eq!(completed[0]["source_size"], json!(12));
    assert_eq!(completed[0]["source_validator_sha256"], json!(validator));
    assert_eq!(completed[0]["materialized"], json!(true));
    assert_eq!(completed[0]["verified"], json!(true));
    assert_eq!(completed[1]["start"], json!(4));
    assert_eq!(completed[1]["end_exclusive"], json!(8));
    assert!(completed.iter().all(|range| range["payload_sha256"]
        .as_str()
        .is_some_and(|digest| digest.len() == 64)));
}

#[test]
fn metadata_prefix_v2_reopen_resumes_only_incomplete_chunks() {
    let root = PrefixLedgerTestRoot::new("restart");
    let workspace_root = root.0.join("workspace");
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session.materialize_verified_range((0, 4), b"abcd").unwrap();
        session.materialize_verified_range((4, 8), b"efgh").unwrap();
    }

    let reopened_workspace = Hdf5RangeWorkspaceV1::new(workspace_root.to_path_buf()).unwrap();
    let reopened = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &reopened_workspace,
        12,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();
    assert_eq!(reopened.verified_ranges(), &[(0, 4), (4, 8)]);
    assert_eq!(reopened.incomplete_ranges(), vec![(8, 12)]);
}

#[test]
fn metadata_prefix_v2_reopen_preserves_failed_attempt_budget() {
    let root = PrefixLedgerTestRoot::new("attempt-budget");
    let workspace_root = root.0.join("workspace");
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session
            .record_failed_attempt(&prefix_retry_failure_observation((0, 4), 12, 3), true)
            .unwrap();
        assert_eq!(session.attempts_for_range((0, 4)).unwrap(), 1);
        let checkpoint: Value = serde_json::from_str(
            &fs::read_to_string(workspace.planning_prefix_checkpoint_path_v2_for_test()).unwrap(),
        )
        .unwrap();
        assert_eq!(checkpoint["attempt_states"][0]["start"], json!(0));
        assert_eq!(checkpoint["attempt_states"][0]["end_exclusive"], json!(4));
        assert_eq!(checkpoint["attempt_states"][0]["attempts"], json!(1));
        assert_eq!(
            checkpoint["attempt_states"][0]["failed_request_count"],
            json!(1)
        );
        assert_eq!(
            checkpoint["attempt_states"][0]["failed_response_body_bytes"],
            json!(3)
        );
    }

    let reopened_workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
    let mut reopened = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &reopened_workspace,
        12,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();
    assert_eq!(reopened.attempts_for_range((0, 4)).unwrap(), 1);
    reopened
        .record_failed_attempt(&prefix_retry_failure_observation((0, 4), 12, 2), true)
        .unwrap();
    assert_eq!(reopened.attempts_for_range((0, 4)).unwrap(), 2);
}

#[test]
fn metadata_prefix_v2_reopen_restores_failure_ledger_and_observations() {
    let case = route_fixture("prefix-retry-cross-restart", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let workspace_root = case._root.0.join("prefix-retry-cross-restart-workspace");
    let ranges = remote_planning_initial_material_ranges(source.len() as u64).unwrap();
    let validator = NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: ROUTE_ETAG.to_owned(),
        content_length: source.len() as u64,
    };
    let validator_sha256 = digest_serializable(&validator).unwrap();
    let source_identity_sha256 = prefix_source_identity_sha256(&intent).unwrap();
    let expected_failure = prefix_retry_failure_observation(ranges[0], source.len() as u64, 3);
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize_deferred_fixture(
            &workspace,
            source.len() as u64,
            &validator_sha256,
            &ranges,
            &source_identity_sha256,
        )
        .unwrap();
        session
            .record_failed_attempt(&expected_failure, true)
            .unwrap();
    }

    let resumed_server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let (resumed_result, observations) =
        run_prefix_bootstrap_probe(&case, &intent, &resumed_server.url, &workspace_root);
    let output = resumed_result.unwrap();
    let statistics = output.ledger.statistics().unwrap();
    assert_eq!(statistics.charged_failure_requests(), 1);
    assert_eq!(statistics.charged_failure_response_bytes(), 3);
    assert_eq!(observations, vec![expected_failure]);
    let resumed_requests = resumed_server.finish();
    assert_eq!(requests_for_range(&resumed_requests, ranges[0]), 1);
}

#[test]
fn metadata_prefix_v2_reopens_legacy_control_stop_without_losing_network_budget() {
    let case = route_fixture("prefix-legacy-control-stop", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let workspace_root = case._root.0.join("prefix-legacy-control-stop-workspace");
    let ranges = remote_planning_initial_material_ranges(source.len() as u64).unwrap();
    let validator = NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: ROUTE_ETAG.to_owned(),
        content_length: source.len() as u64,
    };
    let failed_body_bytes = ranges[0].1 - ranges[0].0;
    let mut stopped =
        prefix_retry_failure_observation(ranges[0], source.len() as u64, failed_body_bytes);
    stopped.failure_code = "NISAR_SUBSET_V2_EXECUTION_STOPPED";
    stopped.validator_sha256 = Some(digest_serializable(&validator).unwrap());
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize_deferred_fixture(
            &workspace,
            source.len() as u64,
            &digest_serializable(&validator).unwrap(),
            &ranges,
            &prefix_source_identity_sha256(&intent).unwrap(),
        )
        .unwrap();
        // Preserve the old on-disk shape, independently of how new stops are recorded.
        // The response arrived, but control stopped execution before any verified write.
        session.checkpoint.schema_version = 2;
        session.checkpoint.attempt_states = vec![NisarPrefixRangeAttemptStateV2 {
            start: ranges[0].0,
            end_exclusive: ranges[0].1,
            attempts: 1,
            retryable: false,
            failed_request_count: 1,
            failed_response_body_bytes: failed_body_bytes,
            failure_observations: vec![NisarPrefixDurableFailureObservationV2::from_runtime(
                &stopped,
            )],
        }];
        session.checkpoint.checkpoint_sha256 = session.checkpoint.recompute_sha256().unwrap();
        workspace
            .persist_planning_prefix_checkpoint_v2(&session.checkpoint.canonical_json().unwrap())
            .unwrap();
        assert!(session.verified_ranges().is_empty());
    }

    let resumed_server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let (resumed_result, observations) =
        run_prefix_bootstrap_probe(&case, &intent, &resumed_server.url, &workspace_root);
    let resumed_requests = resumed_server.finish();
    let output =
        resumed_result.expect("an explicit active execution must recover a prior control stop");
    let statistics = output.ledger.statistics().unwrap();
    assert_eq!(statistics.charged_failure_requests(), 1);
    assert_eq!(
        statistics.charged_failure_response_bytes(),
        failed_body_bytes
    );
    assert_eq!(observations, vec![stopped]);
    assert_eq!(requests_for_range(&resumed_requests, ranges[0]), 1);
}

#[test]
fn metadata_prefix_v2_control_stop_does_not_auto_retry_but_explicit_resume_continues() {
    let case = route_fixture("prefix-control-stop-resume", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let ranges = remote_planning_initial_material_ranges(source.len() as u64).unwrap();
    let workspace_root = case._root.0.join("prefix-control-stop-resume-workspace");
    let first_server = RouteRangeServer::start(source.clone(), ROUTE_ETAG, RouteServerMode::Exact);
    let work = route_ready_work(&case, &intent);
    let url = format!(
        "{}{}",
        first_server.url.trim_end_matches("source.h5"),
        case.request.source.file_name
    );
    let asset = ResolvedAsset::new(&url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(case.request.source.expected_size_bytes))
        .with_suggested_filename(Some(case.request.source.file_name.clone()));
    let options = TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 64 * 1024,
        ..TransferOptions::default()
    };
    let http = Arc::new(HttpTransfer::new_with_proxy_policy(options, ProxyPolicy::Direct).unwrap());
    let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
    let bootstrap = NisarSubsetHttpMetadataBootstrapV2::new(http, asset, workspace).unwrap();
    let control = WorkerControl::default();
    let mut observed_bytes = 0_u64;
    let stopped_result =
        bootstrap.bootstrap_with_progress(&work, &intent, &control, &mut |delta| {
            observed_bytes += delta;
            // The preflight consumes one byte; pause only after actual prefix body data.
            if observed_bytes > 1 {
                control.set_paused_for_test(true);
            }
        });
    let observations = bootstrap.take_range_failure_observations();
    let first_requests = first_server.finish();
    assert!(matches!(
        stopped_result,
        Err(NisarSubsetPlanningErrorV2::ExecutionStopped)
    ));
    assert_eq!(observations.len(), 1);
    assert_eq!(
        observations[0].failure_code,
        "NISAR_SUBSET_V2_EXECUTION_STOPPED"
    );
    assert_eq!(requests_for_range(&first_requests, ranges[0]), 1);
    for &range in &ranges[1..] {
        assert_eq!(requests_for_range(&first_requests, range), 0);
    }
    let (failed_requests, failed_bytes) = actual_failure_charge_v2(&observations[0]);
    assert_eq!(failed_requests, 1);
    assert!(failed_bytes > 0);
    assert_eq!(observed_bytes, 1 + failed_bytes);
    drop(bootstrap);

    let resumed_server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let (resumed_result, resumed_observations) =
        run_prefix_bootstrap_probe(&case, &intent, &resumed_server.url, &workspace_root);
    let resumed_requests = resumed_server.finish();
    let output = resumed_result.expect("a new active execution must resume the unverified range");
    let statistics = output.ledger.statistics().unwrap();
    assert_eq!(statistics.charged_failure_requests(), failed_requests);
    assert_eq!(statistics.charged_failure_response_bytes(), failed_bytes);
    assert_eq!(resumed_observations, observations);
    assert_eq!(requests_for_range(&resumed_requests, ranges[0]), 1);
}

#[test]
fn metadata_prefix_v2_reopen_rejects_persisted_terminal_failure_before_io() {
    let root = PrefixLedgerTestRoot::new("terminal-failure");
    let workspace_root = root.0.join("workspace");
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session
            .record_failed_attempt(&prefix_terminal_failure_observation((0, 4), 12), false)
            .unwrap();
    }

    let reopened_workspace = Hdf5RangeWorkspaceV1::new(workspace_root).unwrap();
    let reopened = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &reopened_workspace,
        12,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();
    assert_eq!(
        reopened.attempts_for_range((0, 4)),
        Err(NisarSubsetPlanningErrorV2::BootstrapSourceUnavailable)
    );
}

#[test]
fn metadata_prefix_v2_reopen_terminal_failure_skips_range_request() {
    let case = route_fixture("prefix-terminal-cross-restart", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let workspace_root = case._root.0.join("prefix-terminal-cross-restart-workspace");
    let ranges = remote_planning_initial_material_ranges(source.len() as u64).unwrap();
    let validator = NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: ROUTE_ETAG.to_owned(),
        content_length: source.len() as u64,
    };
    let validator_sha256 = digest_serializable(&validator).unwrap();
    let source_identity_sha256 = prefix_source_identity_sha256(&intent).unwrap();
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize_deferred_fixture(
            &workspace,
            source.len() as u64,
            &validator_sha256,
            &ranges,
            &source_identity_sha256,
        )
        .unwrap();
        session
            .record_failed_attempt(
                &prefix_terminal_failure_observation(ranges[0], source.len() as u64),
                false,
            )
            .unwrap();
    }

    let resumed_server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let (resumed_result, _) =
        run_prefix_bootstrap_probe(&case, &intent, &resumed_server.url, &workspace_root);
    assert!(matches!(
        resumed_result,
        Err(NisarSubsetPlanningErrorV2::BootstrapSourceUnavailable)
    ));
    let resumed_requests = resumed_server.finish();
    assert_eq!(requests_for_range(&resumed_requests, ranges[0]), 0);
}

#[test]
fn metadata_prefix_v2_reopen_accepts_attempt_checkpoint_without_historical_failures() {
    let root = PrefixLedgerTestRoot::new("legacy-attempt-completed");
    let workspace_root = root.0.join("workspace");
    let checkpoint_path;
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        checkpoint_path = workspace.planning_prefix_checkpoint_path_v2_for_test();
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session.materialize_verified_range((0, 4), b"abcd").unwrap();
        session.materialize_verified_range((4, 8), b"efgh").unwrap();
        session
            .materialize_verified_range((8, 12), b"ijkl")
            .unwrap();
        let mut legacy = session.checkpoint.clone();
        legacy.checkpoint_sha256 = legacy
            .recompute_sha256_without_historical_failure_observations()
            .unwrap();
        fs::write(
            &checkpoint_path,
            legacy
                .canonical_json_without_historical_failure_observations()
                .unwrap(),
        )
        .unwrap();
    }

    let reopened_workspace = Hdf5RangeWorkspaceV1::new(workspace_root).unwrap();
    let reopened = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &reopened_workspace,
        12,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();
    assert!(reopened.incomplete_ranges().is_empty());
}

#[test]
fn metadata_prefix_v2_reopen_accepts_legacy_completed_checkpoint_without_attempt_states() {
    let root = PrefixLedgerTestRoot::new("legacy-completed");
    let workspace_root = root.0.join("workspace");
    let checkpoint_path;
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        checkpoint_path = workspace.planning_prefix_checkpoint_path_v2_for_test();
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session.materialize_verified_range((0, 4), b"abcd").unwrap();
        session.materialize_verified_range((4, 8), b"efgh").unwrap();
        session
            .materialize_verified_range((8, 12), b"ijkl")
            .unwrap();
        let mut legacy = session.checkpoint.clone();
        legacy.attempt_states.clear();
        legacy.checkpoint_sha256 = legacy.recompute_sha256_without_attempt_states().unwrap();
        fs::write(
            &checkpoint_path,
            legacy.canonical_json_without_attempt_states().unwrap(),
        )
        .unwrap();
    }

    let reopened_workspace = Hdf5RangeWorkspaceV1::new(workspace_root).unwrap();
    let reopened = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &reopened_workspace,
        12,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();
    assert!(reopened.incomplete_ranges().is_empty());
}

#[test]
fn metadata_prefix_v2_reopen_rejects_validator_drift() {
    let root = PrefixLedgerTestRoot::new("validator-drift");
    let workspace_root = root.0.join("workspace");
    let ranges = prefix_test_ranges(12, 4);
    let original_validator = prefix_validator_digest("etag-a");
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &original_validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session.materialize_verified_range((0, 4), b"abcd").unwrap();
        session.materialize_verified_range((4, 8), b"efgh").unwrap();
    }

    let reopened_workspace = Hdf5RangeWorkspaceV1::new(workspace_root.to_path_buf()).unwrap();
    assert!(matches!(
        NisarPrefixMaterializationSessionV2::open_or_initialize(
            &reopened_workspace,
            12,
            &prefix_validator_digest("etag-b"),
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        ),
        Err(NisarSubsetPlanningErrorV2::SourceValidatorChanged)
    ));
}

#[test]
fn metadata_prefix_v2_reopen_rejects_frozen_source_identity_drift() {
    let root = PrefixLedgerTestRoot::new("source-identity-drift");
    let workspace_root = root.0.join("workspace");
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session.materialize_verified_range((0, 4), b"abcd").unwrap();
    }

    let reopened_workspace = Hdf5RangeWorkspaceV1::new(workspace_root.to_path_buf()).unwrap();
    assert!(matches!(
        NisarPrefixMaterializationSessionV2::open_or_initialize(
            &reopened_workspace,
            12,
            &validator,
            &ranges,
            ALTERNATE_PREFIX_SOURCE_IDENTITY_SHA256,
        ),
        Err(NisarSubsetPlanningErrorV2::RecoveryInvalid)
    ));
}

#[test]
fn metadata_prefix_v2_rejects_gapped_materialization_plan() {
    let root = PrefixLedgerTestRoot::new("gapped-plan");
    let workspace = Hdf5RangeWorkspaceV1::new(root.0.join("workspace")).unwrap();
    assert!(matches!(
        NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &prefix_validator_digest("etag-a"),
            &[(0, 4), (5, 12)],
            PREFIX_SOURCE_IDENTITY_SHA256,
        ),
        Err(NisarSubsetPlanningErrorV2::RecoveryInvalid)
    ));
    assert!(!workspace.part_path().exists());
}

#[test]
fn metadata_prefix_v2_reopen_ignores_interrupted_next_checkpoint() {
    let root = PrefixLedgerTestRoot::new("interrupted-next");
    let workspace_root = root.0.join("workspace");
    let ranges = prefix_test_ranges(8, 4);
    let validator = prefix_validator_digest("etag-a");
    let next_path;
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        next_path =
            durable_test_path(&workspace.planning_prefix_next_checkpoint_path_v2_for_test());
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            8,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session.materialize_verified_range((0, 4), b"abcd").unwrap();
    }
    fs::write(&next_path, b"interrupted checkpoint candidate").unwrap();

    let reopened_workspace = Hdf5RangeWorkspaceV1::new(workspace_root.to_path_buf()).unwrap();
    let mut reopened = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &reopened_workspace,
        8,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();
    assert_eq!(reopened.verified_ranges(), &[(0, 4)]);
    assert_eq!(reopened.incomplete_ranges(), vec![(4, 8)]);
    reopened
        .materialize_verified_range((4, 8), b"efgh")
        .unwrap();
    assert!(!next_path.exists());
}

#[test]
fn metadata_prefix_v2_reopen_rejects_corrupt_completed_bytes() {
    let root = PrefixLedgerTestRoot::new("corrupt-bytes");
    let workspace_root = root.0.join("workspace");
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    let part_path;
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        part_path = durable_test_path(&workspace.part_path());
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session.materialize_verified_range((0, 4), b"abcd").unwrap();
    }
    let mut corrupt = OpenOptions::new().write(true).open(part_path).unwrap();
    corrupt.seek(SeekFrom::Start(1)).unwrap();
    corrupt.write_all(b"Z").unwrap();
    corrupt.sync_all().unwrap();
    drop(corrupt);

    let reopened_workspace = Hdf5RangeWorkspaceV1::new(workspace_root.to_path_buf()).unwrap();
    assert!(matches!(
        NisarPrefixMaterializationSessionV2::open_or_initialize(
            &reopened_workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        ),
        Err(NisarSubsetPlanningErrorV2::RecoveryInvalid)
    ));
}

#[test]
fn metadata_prefix_v2_reopen_rejects_noncanonical_checkpoint() {
    let root = PrefixLedgerTestRoot::new("noncanonical-checkpoint");
    let workspace_root = root.0.join("workspace");
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    let checkpoint_path;
    {
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
        checkpoint_path =
            durable_test_path(&workspace.planning_prefix_checkpoint_path_v2_for_test());
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session.materialize_verified_range((0, 4), b"abcd").unwrap();
    }
    let checkpoint: Value =
        serde_json::from_str(&fs::read_to_string(&checkpoint_path).unwrap()).unwrap();
    fs::write(
        &checkpoint_path,
        serde_json::to_string_pretty(&checkpoint).unwrap(),
    )
    .unwrap();

    let reopened_workspace = Hdf5RangeWorkspaceV1::new(workspace_root.to_path_buf()).unwrap();
    assert!(matches!(
        NisarPrefixMaterializationSessionV2::open_or_initialize(
            &reopened_workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        ),
        Err(NisarSubsetPlanningErrorV2::RecoveryInvalid)
    ));
}

#[test]
fn metadata_prefix_v2_final_identity_is_stable_for_production_partition() {
    let source_size = crate::NISAR_LIVE_METADATA_PREFIX_BYTES;
    let validator = prefix_validator_digest("etag-a");
    let ranges = production_metadata_prefix_ranges_v2(source_size).unwrap();
    let root = PrefixLedgerTestRoot::new("production-final-identity");
    let workspace = Hdf5RangeWorkspaceV1::new(root.0.join("workspace")).unwrap();
    let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &workspace,
        source_size,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();
    let mut expected = Sha256::new();
    let mut buffer = vec![0_u8; NISAR_SUBSET_METADATA_PREFIX_SUBRANGE_BYTES_V2 as usize];

    for &(start, end) in &ranges {
        let length = usize::try_from(end - start).unwrap();
        for (index, byte) in buffer[..length].iter_mut().enumerate() {
            *byte = ((start + index as u64) % 251) as u8;
        }
        expected.update(&buffer[..length]);
        session
            .materialize_verified_range((start, end), &buffer[..length])
            .unwrap();
    }

    assert_eq!(
        session.final_prefix_sha256().unwrap(),
        format!("{:x}", expected.finalize())
    );
}

#[derive(Clone, Copy)]
enum RouteServerMode {
    Exact,
    FullResponse,
    Missing,
    Retryable503OnMetadataDiscovery,
    MalformedContentRange,
    PartialBody,
    PartialOnceOnThirdPrefix,
    AlwaysPartialOnThirdPrefix,
    RedirectPartialOnceOnThirdPrefix,
    TransportFailure,
    ValidatorDriftAfter(usize),
    /// After `science_phase` is set: the first non-probe Range answers 503.
    Science503Once,
    /// After `science_phase` is set: the first non-probe Range connection is
    /// closed without any response.
    ScienceResetOnce,
    /// The first request, the preflight Range (bytes 0-0), is closed without
    /// any response (downloader enhancement D1c).
    PreflightResetOnce,
    /// The first metadata-discovery Range (neither the preflight nor a
    /// metadata prefix Range) is closed without any response (downloader
    /// enhancement D1d).
    MetadataDiscoveryResetOnce,
}

// A yield-only polling loop can immediately reacquire the Store mutex on Windows
// and starve the scheduler that must publish the state being awaited. Block briefly
// between polls so both the scheduler and local HTTP worker can make progress.
fn wait_for_route_progress() {
    thread::sleep(Duration::from_millis(20));
}

struct RouteFreezeGate {
    revalidation_received: AtomicBool,
    released: AtomicBool,
}

impl RouteFreezeGate {
    fn wait_until_received(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.revalidation_received.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "freeze revalidation request was not observed"
            );
            wait_for_route_progress();
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }
}

struct RouteRangeServer {
    url: String,
    done: Arc<AtomicBool>,
    validator_drift_enabled: Arc<AtomicBool>,
    delayed_body: Arc<AtomicBool>,
    /// Set by a test once planning finished, so science modes spare planning.
    science_phase: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl RouteRangeServer {
    fn start(source: Vec<u8>, etag: &str, mode: RouteServerMode) -> Self {
        Self::start_internal(source, etag, mode, None)
    }

    fn start_with_freeze_gate(source: Vec<u8>, etag: &'static str) -> (Self, Arc<RouteFreezeGate>) {
        let gate = Arc::new(RouteFreezeGate {
            revalidation_received: AtomicBool::new(false),
            released: AtomicBool::new(false),
        });
        (
            Self::start_internal(
                source,
                etag,
                RouteServerMode::Exact,
                Some(Arc::clone(&gate)),
            ),
            gate,
        )
    }

    fn start_internal(
        source: Vec<u8>,
        etag: &str,
        mode: RouteServerMode,
        freeze_gate: Option<Arc<RouteFreezeGate>>,
    ) -> Self {
        let etag = etag.to_owned();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = Arc::clone(&done);
        let validator_drift_enabled = Arc::new(AtomicBool::new(false));
        let worker_validator_drift_enabled = Arc::clone(&validator_drift_enabled);
        let delayed_body = Arc::new(AtomicBool::new(false));
        let worker_delayed_body = Arc::clone(&delayed_body);
        let science_phase = Arc::new(AtomicBool::new(false));
        let worker_science_phase = Arc::clone(&science_phase);
        let mut science_failure_sent = false;
        let mut discovery_reset_sent = false;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let metadata_prefix_ranges =
            remote_planning_initial_material_ranges(source.len() as u64).unwrap();
        let worker = thread::spawn(move || {
            let mut request_index = 0_usize;
            let mut third_prefix_response_count = 0_usize;
            let deadline = Instant::now() + Duration::from_secs(180);
            while !worker_done.load(Ordering::SeqCst) {
                assert!(
                    Instant::now() < deadline,
                    "route range server exceeded bounded lifetime"
                );
                let (mut stream, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        wait_for_route_progress();
                        continue;
                    }
                    Err(error) => panic!("route range accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_nodelay(true).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let request = read_science_request(&mut stream);
                captured.lock().unwrap().push(request.clone());
                request_index += 1;
                let request_path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or_default();
                let range = request.lines().find_map(|line| {
                    line.strip_prefix("Range: bytes=")
                        .or_else(|| line.strip_prefix("range: bytes="))
                        .and_then(|value| value.trim().split_once('-'))
                        .and_then(|(start, end)| {
                            Some((start.parse::<u64>().ok()?, end.parse::<u64>().ok()?))
                        })
                });
                if request_index > 1 && range == Some((0, 0)) {
                    if let Some(gate) = freeze_gate.as_ref() {
                        gate.revalidation_received.store(true, Ordering::SeqCst);
                        let gate_deadline = Instant::now() + Duration::from_secs(30);
                        while !gate.released.load(Ordering::SeqCst) {
                            assert!(
                                Instant::now() < gate_deadline,
                                "freeze revalidation gate was not released"
                            );
                            wait_for_route_progress();
                        }
                    }
                }
                if matches!(mode, RouteServerMode::TransportFailure) {
                    continue;
                }
                if matches!(mode, RouteServerMode::PreflightResetOnce)
                    && request_index == 1
                    && range == Some((0, 0))
                {
                    continue;
                }
                if matches!(
                    mode,
                    RouteServerMode::Science503Once | RouteServerMode::ScienceResetOnce
                ) && !science_failure_sent
                    && worker_science_phase.load(Ordering::SeqCst)
                    && range.is_some_and(|bounds| bounds != (0, 0))
                {
                    science_failure_sent = true;
                    if matches!(mode, RouteServerMode::Science503Once) {
                        let response = science_response(
                            "503 Service Unavailable",
                            &[("Content-Length", "0".to_owned())],
                            &[],
                        );
                        stream.write_all(&response).unwrap();
                        stream.flush().unwrap();
                    }
                    continue;
                }
                if matches!(mode, RouteServerMode::MetadataDiscoveryResetOnce)
                    && !discovery_reset_sent
                    && range.is_some_and(|(start, end_inclusive)| {
                        let end_exclusive = end_inclusive.saturating_add(1);
                        (start, end_inclusive) != (0, 0)
                            && !metadata_prefix_ranges.contains(&(start, end_exclusive))
                    })
                {
                    discovery_reset_sent = true;
                    continue;
                }
                let retryable_metadata_discovery =
                    matches!(mode, RouteServerMode::Retryable503OnMetadataDiscovery)
                        && range.is_some_and(|(start, end_inclusive)| {
                            let end_exclusive = end_inclusive.saturating_add(1);
                            (start, end_inclusive) != (0, 0)
                                && !metadata_prefix_ranges.contains(&(start, end_exclusive))
                        });
                if retryable_metadata_discovery {
                    let response = science_response(
                        "503 Service Unavailable",
                        &[("Content-Length", "0".to_owned())],
                        &[],
                    );
                    stream.write_all(&response).unwrap();
                    stream.flush().unwrap();
                    continue;
                }
                if matches!(mode, RouteServerMode::RedirectPartialOnceOnThirdPrefix)
                    && request_path != "/redirected.h5"
                {
                    let response = science_response(
                        "302 Found",
                        &[
                            ("Location", "/redirected.h5".to_owned()),
                            ("Content-Length", "0".to_owned()),
                        ],
                        &[],
                    );
                    stream.write_all(&response).unwrap();
                    stream.flush().unwrap();
                    continue;
                }
                let response = match mode {
                    RouteServerMode::Missing => science_response(
                        "404 Not Found",
                        &[("Content-Length", "0".to_owned())],
                        &[],
                    ),
                    RouteServerMode::FullResponse => science_response(
                        "200 OK",
                        &[
                            ("ETag", etag.to_owned()),
                            ("Content-Length", "0".to_owned()),
                        ],
                        &[],
                    ),
                    RouteServerMode::Exact
                    | RouteServerMode::Retryable503OnMetadataDiscovery
                    | RouteServerMode::Science503Once
                    | RouteServerMode::ScienceResetOnce
                    | RouteServerMode::MalformedContentRange
                    | RouteServerMode::PartialBody
                    | RouteServerMode::PartialOnceOnThirdPrefix
                    | RouteServerMode::AlwaysPartialOnThirdPrefix
                    | RouteServerMode::RedirectPartialOnceOnThirdPrefix
                    | RouteServerMode::PreflightResetOnce
                    | RouteServerMode::MetadataDiscoveryResetOnce
                    | RouteServerMode::ValidatorDriftAfter(_) => {
                        let (start, end_inclusive) =
                            range.expect("every V2 request must be exact Range");
                        let end_exclusive = end_inclusive.checked_add(1).unwrap();
                        assert!(start < end_exclusive && end_exclusive <= source.len() as u64);
                        let response_etag = if worker_validator_drift_enabled.load(Ordering::SeqCst)
                            || matches!(
                                mode,
                                RouteServerMode::ValidatorDriftAfter(after)
                                    if request_index > after
                            ) {
                            "\"route-v2-drift\""
                        } else {
                            etag.as_str()
                        };
                        let total = if matches!(mode, RouteServerMode::MalformedContentRange) {
                            source.len() + 1
                        } else {
                            source.len()
                        };
                        let third_prefix = NISAR_DEFERRED_FIXTURE_STRUCTURAL_RANGES_V2[2]
                            == (start, end_exclusive);
                        if third_prefix {
                            third_prefix_response_count += 1;
                        }
                        let partial = matches!(mode, RouteServerMode::PartialBody)
                            || (third_prefix
                                && (matches!(mode, RouteServerMode::AlwaysPartialOnThirdPrefix)
                                    || (matches!(
                                        mode,
                                        RouteServerMode::PartialOnceOnThirdPrefix
                                            | RouteServerMode::RedirectPartialOnceOnThirdPrefix
                                    ) && third_prefix_response_count == 1)));
                        let body_end = if partial {
                            start + ((end_exclusive - start) * 95 / 100)
                        } else {
                            end_exclusive
                        };
                        science_response(
                            "206 Partial Content",
                            &[
                                ("ETag", response_etag.to_owned()),
                                (
                                    "Content-Range",
                                    format!("bytes {start}-{end_inclusive}/{total}"),
                                ),
                                ("Content-Length", (end_exclusive - start).to_string()),
                            ],
                            &source[start as usize..body_end as usize],
                        )
                    }
                    RouteServerMode::TransportFailure => unreachable!(),
                };
                if worker_delayed_body.load(Ordering::SeqCst) {
                    let body_start = response
                        .windows(4)
                        .position(|bytes| bytes == b"\r\n\r\n")
                        .unwrap()
                        + 4;
                    stream.write_all(&response[..body_start]).unwrap();
                    stream.flush().unwrap();
                    // Ensure the real client read has measurable body time even
                    // when this loopback fixture's tiny ranges fit in one packet.
                    thread::sleep(Duration::from_millis(5));
                    stream.write_all(&response[body_start..]).unwrap();
                } else {
                    stream.write_all(&response).unwrap();
                }
                stream.flush().unwrap();
            }
        });
        Self {
            url: format!("http://{address}/source.h5"),
            done,
            validator_drift_enabled,
            delayed_body,
            science_phase,
            requests,
            worker: Some(worker),
        }
    }

    fn enable_validator_drift(&self) {
        self.validator_drift_enabled.store(true, Ordering::SeqCst);
    }

    fn finish(mut self) -> Vec<String> {
        self.done.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
        Arc::try_unwrap(self.requests)
            .unwrap()
            .into_inner()
            .unwrap()
    }
}

struct RouteClock;

impl Clock for RouteClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

struct RouteCredentials;

impl CredentialProvider for RouteCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "earthdata-default");
        CredentialMaterial::bearer("ROUTE_V2_TOKEN")
    }
}

struct RouteResolver {
    url: String,
    source_size: u64,
    checksum: String,
    resolve_count: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

impl AssetResolver for RouteResolver {
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
        if let Some(count) = &self.resolve_count {
            count.fetch_add(1, Ordering::SeqCst);
        }
        context.check_cancelled()?;
        let profile = descriptor
            .credential_profile
            .as_deref()
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let credential = context
            .credential_provider()
            .resolve(profile)
            .map_err(|_| ResolveError::credentials_missing())?;
        let mut asset = ResolvedAsset::new(&self.url, ResolvedMethod::Get)?
            .with_expected_size(Some(self.source_size))
            .with_checksum(Some(ResolvedChecksum::new("SHA256", &self.checksum)?))
            .with_asset_role(AssetRole::Primary);
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

fn route_fixture(label: &str, frequency: NisarSubsetFrequency) -> ReadyCase {
    let (fixture_polarization, science_path, mask_path) = match frequency {
        NisarSubsetFrequency::A => (
            "HH",
            "/science/LSAR/RSLC/swaths/frequencyA/HH",
            "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask",
        ),
        NisarSubsetFrequency::B => (
            "HV",
            "/science/LSAR/RSLC/swaths/frequencyB/HV",
            "/science/LSAR/RSLC/swaths/frequencyB/inputDataExceptionMask",
        ),
    };
    let fixture_frequency = frequency.clone();
    ready_case_with_selection_for_v2(
        label,
        1,
        frequency,
        fixture_polarization,
        science_path,
        mask_path,
        true,
        move |file| {
            if fixture_frequency == NisarSubsetFrequency::B {
                file.dataset("/science/LSAR/RSLC/swaths/frequencyB/slantRange")
                    .unwrap()
                    .write_raw(&[800_000.0_f64, 800_010.0, 800_020.0, 800_030.0])
                    .unwrap();
            }
        },
    )
}

fn route_prepared_intent(
    case: &ReadyCase,
    frequency: NisarSubsetFrequency,
    polarizations: &[&str],
) -> NisarSubsetPreparedIntentV2 {
    NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: NisarSubsetSourceProductIdentityV2 {
            provider_id: case.request.source.provider_id.clone(),
            collection_id: case.request.source.collection_id.clone(),
            collection_profile_id: case.request.source.collection_profile_id.clone(),
            provider_product_id: case.request.source.provider_product_id.clone(),
            provider_product_key: case.request.source.provider_product_key.clone(),
            product_specification_version: case
                .request
                .source
                .product_specification_version
                .clone(),
            crid: case.request.source.crid.clone(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        },
        source_asset_identity: NisarSubsetSourceAssetIdentityV2 {
            asset_key: "science-rslc-hdf5".to_owned(),
            stable_asset_key: case.request.source.stable_asset_key.clone(),
            file_name: case.request.source.file_name.clone(),
            checksum: case.request.source.checksum.clone(),
        },
        source_capability: NisarSubsetSourceCapabilityV2 {
            evidence_sha256: "b".repeat(64),
            frequencies: vec![
                NisarSubsetFrequencyCapabilityV2 {
                    frequency: NisarSubsetFrequency::A,
                    available_polarizations: vec!["HH".to_owned(), "HV".to_owned()],
                },
                NisarSubsetFrequencyCapabilityV2 {
                    frequency: NisarSubsetFrequency::B,
                    available_polarizations: vec![
                        "HH".to_owned(),
                        "HV".to_owned(),
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
            "coordinates": [[[36.1, 9.1], [36.9, 9.1], [36.9, 9.9], [36.1, 9.9], [36.1, 9.1]]]
        })
        .as_object()
        .unwrap()
        .clone(),
        selected_frequency: frequency,
        selected_polarizations: polarizations
            .iter()
            .map(|polarization| (*polarization).to_owned())
            .collect(),
    })
    .unwrap()
}

#[path = "nisar_budget_upgrade_route_tests.rs"]
mod budget_upgrade_route;

fn route_recovery_root(work: &DownloadWork) -> PathBuf {
    let mut digest = Sha256::new();
    digest.update(b"NisarSubsetV2RecoveryWorkspaceIdentity");
    for value in [&work.task.task_id, &work.item.item_id] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    work.task
        .output_root
        .join(".nisar-subset-v2-recovery")
        .join(format!("{:x}", digest.finalize()))
}

fn route_ready_work(case: &ReadyCase, intent: &NisarSubsetPreparedIntentV2) -> DownloadWork {
    let mut work = work(case, intent);
    let mut item = crate::nisar_subset_writer::tests::snapshot(&case.request);
    let product_key = item.item_id.clone();
    let product_id = item
        .asset_access
        .as_ref()
        .unwrap()
        .reference
        .provider_product_id
        .clone();
    let granule = case.request.source.file_name.trim_end_matches(".h5");
    let official_href = format!(
        "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/NISAR_L1_RSLC_PROVISIONAL_V1/{granule}/{}",
        case.request.source.file_name
    );
    let mut metadata = crate::test_suites::nisar_support::frozen_item_with_source_identity(
        case.request.source.expected_size_bytes,
        case.request.source.checksum.as_ref().unwrap().value.clone(),
    )
    .metadata;
    if let Some(properties) = metadata
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.insert("provider_product_id".to_owned(), json!(product_id.clone()));
        properties.insert(
            "provider_product_key".to_owned(),
            json!(product_key.clone()),
        );
        properties.insert(
            "cmr_granule_concept_id".to_owned(),
            json!(product_id.clone()),
        );
        properties.insert("producer_granule_id".to_owned(), json!(granule));
        properties.insert("asf_product_id".to_owned(), json!(granule));
    }
    if let Some(assets) = metadata.get_mut("assets").and_then(Value::as_array_mut) {
        for asset in assets {
            let Some(asset) = asset.as_object_mut() else {
                continue;
            };
            let asset_key = asset
                .get("asset_key")
                .and_then(Value::as_str)
                .unwrap()
                .to_owned();
            asset.insert(
                "stable_asset_key".to_owned(),
                json!(stable_asset_key(&product_key, &asset_key).unwrap()),
            );
            if asset.get("role").and_then(Value::as_str) == Some("primary") {
                asset.insert("file_name".to_owned(), json!(case.request.source.file_name));
                asset.insert(
                    "size_bytes".to_owned(),
                    json!(case.request.source.expected_size_bytes),
                );
                asset.insert(
                    "checksum".to_owned(),
                    json!({"algorithm":"SHA-256","value":case.request.source.checksum.as_ref().unwrap().value}),
                );
                asset.insert(
                    "routes".to_owned(),
                    json!([{"scheme":"https","locator":official_href.clone()}]),
                );
                if let Some(evidence) = asset
                    .get_mut("native_evidence")
                    .and_then(Value::as_object_mut)
                {
                    evidence.insert(
                        "archive_name".to_owned(),
                        json!(case.request.source.file_name),
                    );
                }
            }
        }
    }
    metadata.insert("provider_product_id".to_owned(), json!(product_id));
    metadata.insert("provider_product_key".to_owned(), json!(product_key));
    metadata.insert(
        "delivery_kind".to_owned(),
        json!(NISAR_SUBSET_V2_DELIVERY_KIND),
    );
    metadata.insert("source_item_id".to_owned(), json!(item.item_id));
    metadata.insert("subset_frequency".to_owned(), json!("B"));
    metadata.insert("subset_polarization".to_owned(), json!("VV"));
    metadata.insert(
        "subset_aoi_geojson".to_owned(),
        json!({"type":"Polygon","coordinates":[]}),
    );
    embed_nisar_subset_prepared_intent_v2_envelope(&mut metadata, intent).unwrap();
    item.metadata = metadata;
    let access = item.asset_access.as_mut().unwrap();
    access.descriptor.requires_credentials = true;
    access.descriptor.credential_profile = Some("earthdata-default".to_owned());
    access.checksum_algorithm = Some("SHA256".to_owned());
    access.descriptor.stable_href = Some(official_href);
    work.item = item.clone();
    work.task.candidates = vec![item.clone()];
    work.task.initial_selected_item_ids = vec![item.item_id.clone()];
    work.task.output_root = case._root.0.join("route-output");
    work
}

fn route_worker(
    server_url: String,
    case: &ReadyCase,
    item: &SnapshotItem,
) -> ProductionDownloadWorker {
    route_worker_with_resolve_counter(server_url, case, item, None)
}

fn route_worker_with_resolve_counter(
    server_url: String,
    case: &ReadyCase,
    item: &SnapshotItem,
    resolve_count: Option<Arc<std::sync::atomic::AtomicUsize>>,
) -> ProductionDownloadWorker {
    let server_url = format!(
        "{}{}",
        server_url.trim_end_matches("source.h5"),
        case.request.source.file_name
    );
    let checksum = case.request.source.checksum.as_ref().unwrap().value.clone();
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(RouteResolver {
            url: server_url,
            source_size: case.request.source.expected_size_bytes,
            checksum,
            resolve_count,
        }))
        .unwrap();
    let runtime = ResolverExecutionRuntime::with_dependencies(
        registry,
        Arc::new(RouteCredentials),
        Arc::new(RouteClock),
    );
    runtime
        .resolve_for_nisar_remote_subset(item, &|| false)
        .unwrap_or_else(|error| panic!("route resolver probe failed: {error:?}"));
    let options = TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 64 * 1024,
        ..TransferOptions::default()
    };
    let transfer =
        Arc::new(HttpTransfer::new_with_proxy_policy(options, ProxyPolicy::Direct).unwrap());
    let backend: Arc<dyn crate::TransferBackend> = transfer.clone();
    ProductionDownloadWorker::new(options)
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, backend)
        .with_nisar_subset_v2_http_for_test(transfer)
}

fn run_prefix_bootstrap_probe(
    case: &ReadyCase,
    intent: &NisarSubsetPreparedIntentV2,
    server_url: &str,
    workspace_root: &Path,
) -> (
    Result<NisarSubsetMetadataBootstrapOutputV2, NisarSubsetPlanningErrorV2>,
    Vec<NisarSubsetRangeFailureObservationV2>,
) {
    run_prefix_bootstrap_probe_with_progress(case, intent, server_url, workspace_root, &mut |_| {})
}

/// The same probe with a recorded planning retry backoff (downloader D1d).
fn run_prefix_bootstrap_probe_with_backoff(
    case: &ReadyCase,
    intent: &NisarSubsetPreparedIntentV2,
    server_url: &str,
    workspace_root: &Path,
    backoff: Arc<crate::nisar_science_transfer_v2::ScienceRetryBackoff>,
) -> (
    Result<NisarSubsetMetadataBootstrapOutputV2, NisarSubsetPlanningErrorV2>,
    Vec<NisarSubsetRangeFailureObservationV2>,
) {
    run_prefix_bootstrap_probe_inner(
        case,
        intent,
        server_url,
        workspace_root,
        &mut |_| {},
        backoff,
    )
}

fn run_prefix_bootstrap_probe_with_progress(
    case: &ReadyCase,
    intent: &NisarSubsetPreparedIntentV2,
    server_url: &str,
    workspace_root: &Path,
    transfer_progress: &mut dyn FnMut(u64),
) -> (
    Result<NisarSubsetMetadataBootstrapOutputV2, NisarSubsetPlanningErrorV2>,
    Vec<NisarSubsetRangeFailureObservationV2>,
) {
    // A retry's backoff is not slept here (downloader D1d: the metadata
    // prefix waits the bounded planning backoff before a retry).
    run_prefix_bootstrap_probe_inner(
        case,
        intent,
        server_url,
        workspace_root,
        transfer_progress,
        Arc::new(|_, _| Duration::ZERO),
    )
}

fn run_prefix_bootstrap_probe_inner(
    case: &ReadyCase,
    intent: &NisarSubsetPreparedIntentV2,
    server_url: &str,
    workspace_root: &Path,
    transfer_progress: &mut dyn FnMut(u64),
    backoff: Arc<crate::nisar_science_transfer_v2::ScienceRetryBackoff>,
) -> (
    Result<NisarSubsetMetadataBootstrapOutputV2, NisarSubsetPlanningErrorV2>,
    Vec<NisarSubsetRangeFailureObservationV2>,
) {
    let work = route_ready_work(case, intent);
    let url = format!(
        "{}{}",
        server_url.trim_end_matches("source.h5"),
        case.request.source.file_name
    );
    let asset = ResolvedAsset::new(&url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(case.request.source.expected_size_bytes))
        .with_suggested_filename(Some(case.request.source.file_name.clone()));
    let options = TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(5),
        chunk_size: 64 * 1024,
        ..TransferOptions::default()
    };
    let http = Arc::new(HttpTransfer::new_with_proxy_policy(options, ProxyPolicy::Direct).unwrap());
    let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.to_path_buf()).unwrap();
    let bootstrap = NisarSubsetHttpMetadataBootstrapV2::new(http, asset, workspace)
        .unwrap()
        .with_planning_retry_backoff_for_test(backoff);
    let result = bootstrap.bootstrap_with_progress(
        &work,
        intent,
        &WorkerControl::default(),
        transfer_progress,
    );
    let observations = bootstrap.take_range_failure_observations();
    (result, observations)
}

fn requests_for_range(requests: &[String], range: (u64, u64)) -> usize {
    requests
        .iter()
        .filter(|request| request_range_bounds(request) == range)
        .count()
}

#[test]
fn metadata_prefix_v2_reports_real_network_body_progress() {
    let case = route_fixture("prefix-real-progress", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let workspace_root = case._root.0.join("prefix-real-progress-workspace");
    let mut progress_deltas = Vec::new();

    let (result, observations) = run_prefix_bootstrap_probe_with_progress(
        &case,
        &intent,
        &server.url,
        &workspace_root,
        &mut |delta| progress_deltas.push(delta),
    );

    let output = result.unwrap();
    assert!(observations.is_empty());
    assert!(progress_deltas.len() > 1);
    assert!(progress_deltas.iter().all(|delta| *delta > 0));
    assert_eq!(
        progress_deltas.iter().sum::<u64>(),
        output
            .ledger
            .statistics()
            .unwrap()
            .total_planning_response_body_bytes()
    );
    assert!(!server.finish().is_empty());
}

#[test]
fn metadata_prefix_v2_retries_only_partial_current_chunk_and_accepts_it_once() {
    let case = route_fixture("prefix-retry-current", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(
        source,
        ROUTE_ETAG,
        RouteServerMode::PartialOnceOnThirdPrefix,
    );
    let workspace_root = case._root.0.join("prefix-retry-current-workspace");

    let (result, observations) =
        run_prefix_bootstrap_probe(&case, &intent, &server.url, &workspace_root);
    let output = result.unwrap();
    assert_eq!(output.ledger.charged_failure_requests, 1);
    assert_eq!(observations.len(), 1);
    let statistics = output.ledger.statistics().unwrap();
    let requests = server.finish();
    let [first, second, third] = NISAR_DEFERRED_FIXTURE_STRUCTURAL_RANGES_V2;
    assert_eq!(requests_for_range(&requests, first), 1);
    assert_eq!(requests_for_range(&requests, second), 1);
    assert_eq!(requests_for_range(&requests, third), 2);
    assert_eq!(
        statistics
            .metadata_prefix()
            .requested_ranges()
            .iter()
            .filter(|range| (range.start, range.end_exclusive) == third)
            .count(),
        1
    );
}

/// Downloader enhancement D1d (D1c report risk 5): a transient failure of a
/// metadata prefix Range was retried at once. It now waits the bounded
/// planning backoff (5 s, then 20 s; here recorded and not slept) first.
#[test]
fn metadata_prefix_v2_waits_the_bounded_backoff_before_a_retry() {
    let case = route_fixture("prefix-retry-backoff", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(
        source,
        ROUTE_ETAG,
        RouteServerMode::PartialOnceOnThirdPrefix,
    );
    let workspace_root = case._root.0.join("prefix-retry-backoff-workspace");
    let backoffs = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&backoffs);
    let (result, observations) = run_prefix_bootstrap_probe_with_backoff(
        &case,
        &intent,
        &server.url,
        &workspace_root,
        Arc::new(move |retry, http_status| {
            recorded.lock().unwrap().push((retry, http_status));
            Duration::ZERO
        }),
    );
    result.unwrap();
    assert_eq!(observations.len(), 1);
    let third = NISAR_DEFERRED_FIXTURE_STRUCTURAL_RANGES_V2[2];
    assert_eq!(requests_for_range(&server.finish(), third), 2);
    assert_eq!(
        *backoffs.lock().unwrap(),
        vec![(1, Some(206))],
        "the retry of the partial prefix Range waited the first backoff"
    );
}

#[test]
fn metadata_prefix_v2_redirect_partial_retry_counts_four_requests_and_partial_bytes() {
    let case = route_fixture("prefix-retry-redirect", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(
        source,
        ROUTE_ETAG,
        RouteServerMode::RedirectPartialOnceOnThirdPrefix,
    );
    let workspace_root = case._root.0.join("prefix-retry-redirect-workspace");

    let (result, observations) =
        run_prefix_bootstrap_probe(&case, &intent, &server.url, &workspace_root);
    let output = result.unwrap();
    let third = NISAR_DEFERRED_FIXTURE_STRUCTURAL_RANGES_V2[2];
    let partial_bytes = (third.1 - third.0) * 95 / 100;
    assert_eq!(output.ledger.charged_failure_requests, 2);
    assert_eq!(output.ledger.charged_failure_response_bytes, partial_bytes);
    assert_eq!(observations[0].request_count, 2);
    assert_eq!(observations[0].response_body_bytes, Some(partial_bytes));
    let statistics = output.ledger.statistics().unwrap();
    let accepted_index = statistics
        .metadata_prefix()
        .requested_ranges()
        .iter()
        .position(|range| (range.start, range.end_exclusive) == third)
        .unwrap();
    assert_eq!(
        statistics.metadata_prefix().request_counts()[accepted_index],
        2
    );
    let requests = server.finish();
    assert_eq!(requests_for_range(&requests, third), 4);
}

#[test]
fn metadata_prefix_v2_restart_after_redirect_success_preserves_request_counts() {
    let case = route_fixture(
        "prefix-retry-redirect-success-restart",
        NisarSubsetFrequency::A,
    );
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let workspace_root = case
        ._root
        .0
        .join("prefix-retry-redirect-success-restart-workspace");
    let first_server = RouteRangeServer::start(
        source.clone(),
        ROUTE_ETAG,
        RouteServerMode::RedirectPartialOnceOnThirdPrefix,
    );
    let (first_result, _) =
        run_prefix_bootstrap_probe(&case, &intent, &first_server.url, &workspace_root);
    let first_output = first_result.unwrap();
    let first_statistics = first_output.ledger.statistics().unwrap();
    assert!(first_statistics.metadata_prefix().request_count() > 1);
    let first_requests = first_server.finish();
    let first_range = NISAR_DEFERRED_FIXTURE_STRUCTURAL_RANGES_V2[0];
    assert_eq!(requests_for_range(&first_requests, first_range), 2);

    let resumed_server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let (resumed_result, _) =
        run_prefix_bootstrap_probe(&case, &intent, &resumed_server.url, &workspace_root);
    let resumed_output = resumed_result.unwrap();
    let resumed_statistics = resumed_output.ledger.statistics().unwrap();
    assert_eq!(
        resumed_statistics.metadata_prefix().request_count(),
        first_statistics.metadata_prefix().request_count()
    );
    let resumed_requests = resumed_server.finish();
    assert_eq!(resumed_requests.len(), 1);
    assert_eq!(request_range_bounds(&resumed_requests[0]), (0, 1));
}

#[test]
fn metadata_prefix_v2_restart_after_success_preserves_failure_history() {
    let case = route_fixture("prefix-retry-success-restart", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let workspace_root = case._root.0.join("prefix-retry-success-restart-workspace");
    let first_server = RouteRangeServer::start(
        source.clone(),
        ROUTE_ETAG,
        RouteServerMode::PartialOnceOnThirdPrefix,
    );

    let (first_result, first_observations) =
        run_prefix_bootstrap_probe(&case, &intent, &first_server.url, &workspace_root);
    let first_output = first_result.unwrap();
    assert_eq!(first_observations.len(), 1);
    assert_eq!(first_output.ledger.charged_failure_requests, 1);
    let first_failure = first_observations[0].clone();
    let first_requests = first_server.finish();
    let [first, second, third] = NISAR_DEFERRED_FIXTURE_STRUCTURAL_RANGES_V2;
    assert_eq!(requests_for_range(&first_requests, first), 1);
    assert_eq!(requests_for_range(&first_requests, second), 1);
    assert_eq!(requests_for_range(&first_requests, third), 2);

    let resumed_server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let (resumed_result, resumed_observations) =
        run_prefix_bootstrap_probe(&case, &intent, &resumed_server.url, &workspace_root);
    let resumed_output = resumed_result.unwrap();
    assert_eq!(resumed_output.ledger.charged_failure_requests, 1);
    assert_eq!(
        resumed_output.ledger.charged_failure_response_bytes,
        first_output.ledger.charged_failure_response_bytes
    );
    assert_eq!(resumed_observations, vec![first_failure]);
    let resumed_requests = resumed_server.finish();
    assert_eq!(requests_for_range(&resumed_requests, first), 0);
    assert_eq!(requests_for_range(&resumed_requests, second), 0);
    assert_eq!(requests_for_range(&resumed_requests, third), 0);
}

#[test]
fn metadata_prefix_v2_restart_after_exhaustion_reuses_prior_chunks() {
    let case = route_fixture("prefix-retry-restart", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let workspace_root = case._root.0.join("prefix-retry-restart-workspace");
    let first_server = RouteRangeServer::start(
        source.clone(),
        ROUTE_ETAG,
        RouteServerMode::AlwaysPartialOnThirdPrefix,
    );

    let (first_result, first_observations) =
        run_prefix_bootstrap_probe(&case, &intent, &first_server.url, &workspace_root);
    assert!(matches!(
        first_result,
        Err(NisarSubsetPlanningErrorV2::BootstrapPartialResponse)
    ));
    assert_eq!(first_observations.len(), 3);
    let first_requests = first_server.finish();
    let [first, second, third] = NISAR_DEFERRED_FIXTURE_STRUCTURAL_RANGES_V2;
    assert_eq!(requests_for_range(&first_requests, first), 1);
    assert_eq!(requests_for_range(&first_requests, second), 1);
    assert_eq!(requests_for_range(&first_requests, third), 3);

    let resumed_server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let (resumed_result, _) =
        run_prefix_bootstrap_probe(&case, &intent, &resumed_server.url, &workspace_root);
    assert!(matches!(
        resumed_result,
        Err(NisarSubsetPlanningErrorV2::BootstrapTransport)
    ));
    let resumed_requests = resumed_server.finish();
    assert_eq!(requests_for_range(&resumed_requests, first), 0);
    assert_eq!(requests_for_range(&resumed_requests, second), 0);
    assert_eq!(requests_for_range(&resumed_requests, third), 0);
}

#[test]
fn bootstrap_ledger_allows_adjacent_materialization_ranges() {
    let mut ledger = BootstrapLedger::new(
        1_000_000_000,
        "a".repeat(64),
        vec![(999_999_999, 1_000_000_000)],
    )
    .unwrap();
    let first = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 0, 4, 4)
        .unwrap();
    ledger.commit(first, 4).unwrap();
    let second = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 4, 8, 4)
        .unwrap();
    ledger.commit(second, 4).unwrap();
    assert_eq!(
        ledger
            .statistics()
            .unwrap()
            .metadata_prefix()
            .request_count(),
        2
    );
}

#[test]
fn prefix_retry_request_257_is_denied_before_io() {
    let mut ledger = BootstrapLedger::new(
        1_000_000_000,
        "a".repeat(64),
        vec![(999_999_999, 1_000_000_000)],
    )
    .unwrap();
    let observation = NisarSubsetRangeFailureObservationV2 {
        stage: "metadata_prefix".to_owned(),
        start: 0,
        end_exclusive: 1,
        source_size_bytes: 1_000_000_000,
        disposition: "sent_response_rejected",
        request_count: 1,
        http_status: Some(503),
        response_body_bytes: None,
        transport_diagnostic: None,
        validator_sha256: None,
        failure_code: "NISAR_SUBSET_V2_TRANSPORT_FAILED",
        redirect_occurred: false,
        redirect_hop_index: None,
        redirect_source_host_class: None,
        redirect_destination_host_class: None,
        redirect_source_path_class: None,
        redirect_destination_path_class: None,
        redirect_same_origin: None,
        redirect_scheme_transition: None,
        redirect_policy_result: None,
        redirect_rejection_reason: None,
    };
    let io_count = AtomicU64::new(0);
    for _ in 0..NISAR_LIVE_MAX_REQUESTS {
        let reservation = ledger
            .reserve(BootstrapCategory::MetadataPrefix, 0, 1, 1)
            .unwrap();
        io_count.fetch_add(1, Ordering::SeqCst);
        ledger.fail(reservation, &observation).unwrap();
    }
    assert!(matches!(
        ledger.reserve(BootstrapCategory::MetadataPrefix, 0, 1, 1),
        Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)
    ));
    assert_eq!(io_count.load(Ordering::SeqCst), NISAR_LIVE_MAX_REQUESTS);
}

#[test]
fn prefix_retry_response_overshoot_is_denied_before_io() {
    let source_size = 1_000_000;
    let mut ledger = BootstrapLedger::new(
        source_size,
        "a".repeat(64),
        vec![(source_size - 1, source_size)],
    )
    .unwrap();
    let reservation = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 0, 1, 1)
        .unwrap();
    ledger
        .fail_with_charge(reservation, 1, ledger.max_response_body_bytes - 1)
        .unwrap();
    assert!(matches!(
        ledger.reserve(BootstrapCategory::MetadataPrefix, 0, 2, 2),
        Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)
    ));
}

#[test]
fn prefix_failure_charge_uses_actual_multi_request_body_and_is_atomic_on_overflow() {
    let mut ledger = BootstrapLedger::new(
        1_000_000_000,
        "a".repeat(64),
        vec![(999_999_999, 1_000_000_000)],
    )
    .unwrap();
    let reservation = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 0, 100, 100)
        .unwrap();
    let observation = NisarSubsetRangeFailureObservationV2 {
        stage: "metadata_prefix".to_owned(),
        start: 0,
        end_exclusive: 100,
        source_size_bytes: 1_000_000_000,
        disposition: "sent_response_rejected",
        request_count: 2,
        http_status: Some(206),
        response_body_bytes: Some(95),
        transport_diagnostic: None,
        validator_sha256: None,
        failure_code: "NISAR_SUBSET_V2_PARTIAL_RESPONSE",
        redirect_occurred: true,
        redirect_hop_index: Some(1),
        redirect_source_host_class: None,
        redirect_destination_host_class: None,
        redirect_source_path_class: None,
        redirect_destination_path_class: None,
        redirect_same_origin: Some(true),
        redirect_scheme_transition: None,
        redirect_policy_result: Some("accepted".to_owned()),
        redirect_rejection_reason: None,
    };
    ledger.fail(reservation, &observation).unwrap();
    assert_eq!(ledger.charged_failure_requests, 2);
    assert_eq!(ledger.charged_failure_response_bytes, 95);

    let overflow_reservation = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 100, 200, 100)
        .unwrap();
    ledger.charged_failure_requests = u64::MAX;
    let original_bytes = ledger.charged_failure_response_bytes;
    assert_eq!(
        ledger.fail_with_charge(overflow_reservation, 1, 1),
        Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)
    );
    assert!(ledger.pending.is_some());
    assert_eq!(ledger.charged_failure_requests, u64::MAX);
    assert_eq!(ledger.charged_failure_response_bytes, original_bytes);
}

#[test]
fn bootstrap_ledger_retry_charge_stays_pending_and_success_is_accepted_once() {
    let mut ledger = BootstrapLedger::new(
        1_000_000_000,
        "a".repeat(64),
        vec![(999_999_999, 1_000_000_000)],
    )
    .unwrap();
    let reservation = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 0, 100, 100)
        .unwrap();

    ledger
        .charge_retryable_attempt(&reservation, 2, 95)
        .unwrap();
    assert!(ledger.pending.is_some());
    ledger.check_attempt_budget(&reservation, 2, 100).unwrap();
    ledger
        .commit_with_request_count(reservation, 100, 2)
        .unwrap();

    let statistics = ledger.statistics().unwrap();
    assert_eq!(statistics.metadata_prefix().requested_ranges().len(), 1);
    assert_eq!(statistics.metadata_prefix().request_count(), 2);
    assert_eq!(statistics.metadata_prefix().response_body_bytes(), 100);
    assert_eq!(statistics.total_planning_request_count(), 2);
    assert_eq!(statistics.total_planning_response_body_bytes(), 100);
    assert_eq!(statistics.charged_failure_requests(), 2);
    assert_eq!(statistics.charged_failure_response_bytes(), 95);
}

#[test]
fn bootstrap_ledger_retry_charges_accumulate_without_logical_range_length_cap() {
    let mut ledger = BootstrapLedger::new(
        1_000_000_000,
        "a".repeat(64),
        vec![(999_999_999, 1_000_000_000)],
    )
    .unwrap();
    let reservation = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 0, 100, 100)
        .unwrap();

    ledger
        .charge_retryable_attempt(&reservation, 2, 95)
        .unwrap();
    ledger
        .charge_retryable_attempt(&reservation, 3, 250)
        .unwrap();

    assert_eq!(ledger.charged_failure_requests, 5);
    assert_eq!(ledger.charged_failure_response_bytes, 345);
    assert!(ledger.pending.is_some());
}

#[test]
fn bootstrap_ledger_denies_next_retry_request_charge_before_io() {
    let mut ledger = BootstrapLedger::new(
        1_000_000_000,
        "a".repeat(64),
        vec![(999_999_999, 1_000_000_000)],
    )
    .unwrap();
    let reservation = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 0, 1, 1)
        .unwrap();
    ledger
        .charge_retryable_attempt(&reservation, NISAR_LIVE_MAX_REQUESTS - 1, 1)
        .unwrap();
    let io_count = AtomicU64::new(0);

    assert_eq!(
        ledger.check_attempt_budget(&reservation, 2, 1),
        Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)
    );
    assert_eq!(io_count.load(Ordering::SeqCst), 0);
    assert_eq!(ledger.charged_failure_requests, NISAR_LIVE_MAX_REQUESTS - 1);
}

#[test]
fn bootstrap_ledger_denies_next_retry_byte_charge_before_io() {
    let source_size = 10 * 1024 * 1024;
    let mut ledger = BootstrapLedger::new(
        source_size,
        "a".repeat(64),
        vec![(source_size - 1, source_size)],
    )
    .unwrap();
    let reservation = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 0, 1, 1)
        .unwrap();
    ledger
        .charge_retryable_attempt(&reservation, 1, ledger.max_response_body_bytes - 1)
        .unwrap();
    let io_count = AtomicU64::new(0);

    assert_eq!(
        ledger.check_attempt_budget(&reservation, 1, 2),
        Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)
    );
    assert_eq!(io_count.load(Ordering::SeqCst), 0);
    assert_eq!(
        ledger.charged_failure_response_bytes,
        ledger.max_response_body_bytes - 1
    );
}

#[test]
fn bootstrap_ledger_retry_charge_overflow_is_atomic() {
    let mut ledger = BootstrapLedger::new(
        1_000_000_000,
        "a".repeat(64),
        vec![(999_999_999, 1_000_000_000)],
    )
    .unwrap();
    let reservation = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 0, 1, 1)
        .unwrap();
    ledger.charged_failure_requests = u64::MAX;
    ledger.charged_failure_response_bytes = 17;

    assert_eq!(
        ledger.charge_retryable_attempt(&reservation, 1, 1),
        Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)
    );
    assert_eq!(ledger.charged_failure_requests, u64::MAX);
    assert_eq!(ledger.charged_failure_response_bytes, 17);
    assert!(ledger.pending.is_some());
}

#[test]
fn bootstrap_ledger_final_failure_consumes_retryable_reservation_and_exposes_totals() {
    let mut ledger = BootstrapLedger::new(
        1_000_000_000,
        "a".repeat(64),
        vec![(999_999_999, 1_000_000_000)],
    )
    .unwrap();
    let reservation = ledger
        .reserve(BootstrapCategory::MetadataPrefix, 0, 100, 100)
        .unwrap();
    ledger
        .charge_retryable_attempt(&reservation, 2, 95)
        .unwrap();
    assert!(ledger.pending.is_some());

    ledger.fail_with_charge(reservation, 1, 40).unwrap();
    assert!(ledger.pending.is_none());
    let statistics = ledger.statistics().unwrap();
    assert_eq!(statistics.charged_failure_requests(), 3);
    assert_eq!(statistics.charged_failure_response_bytes(), 135);
    assert_eq!(statistics.total_planning_request_count(), 0);
    assert_eq!(statistics.total_planning_response_body_bytes(), 0);
}

#[test]
fn production_worker_routes_only_v2_to_fail_closed_authority_branch() {
    let (case, intent, _bootstrap, _source) =
        concrete_fixture("production-route-v2-only", NisarSubsetFrequency::A, &["HH"]);
    let worker = ProductionDownloadWorker::new(TransferOptions::default()).unwrap();
    let authority = NisarExecutionPlanFreezeAuthority::disconnected_for_test();
    let v2 = worker.execute_with_authority(
        work(&case, &intent),
        WorkerControl::default(),
        &authority,
        &mut |_| {},
    );
    assert_eq!(v2.bytes_written, 0);
    assert_eq!(
        v2.failure.as_ref().map(|failure| failure.code.as_str()),
        Some("NISAR_SUBSET_V2_COORDINATOR_UNAVAILABLE")
    );

    let mut direct_work = work(&case, &intent);
    direct_work
        .item
        .metadata
        .insert("delivery_kind".to_owned(), json!("complete_product"));
    let mut authority_work = work(&case, &intent);
    authority_work
        .item
        .metadata
        .insert("delivery_kind".to_owned(), json!("complete_product"));
    let direct = worker.execute(direct_work, WorkerControl::default(), &mut |_| {});
    let routed = worker.execute_with_authority(
        authority_work,
        WorkerControl::default(),
        &authority,
        &mut |_| {},
    );
    assert_eq!(direct.outcome, routed.outcome);
    assert_eq!(direct.bytes_written, routed.bytes_written);
    assert_eq!(
        direct.failure.as_ref().map(|failure| failure.code.as_str()),
        routed.failure.as_ref().map(|failure| failure.code.as_str())
    );
    assert_ne!(
        routed.failure.as_ref().map(|failure| failure.code.as_str()),
        Some("NISAR_SUBSET_V2_COORDINATOR_UNAVAILABLE")
    );
}

fn seed_scheduler_task(store: &mut TaskStore, work: &DownloadWork) {
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: work.task.batch_id.clone(),
                provider_id: work.task.provider_id.clone(),
                created_at: work.task.created_at.clone(),
                aoi_name: work.task.aoi_name.clone(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates: vec![work.item.clone()],
            },
            workbench: SearchWorkbenchState {
                batch_id: work.task.batch_id.clone(),
                checked_item_ids: vec![work.item.item_id.clone()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_paused_task_with_explicit_items(
            &StoreCreateTaskRequest {
                task_id: work.task.task_id.clone(),
                batch_id: work.task.batch_id.clone(),
                selected_item_ids: vec![work.item.item_id.clone()],
                checked_item_ids: vec![work.item.item_id.clone()],
                page_index: 0,
                page_size: 24,
                output_root: work.task.output_root.clone(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: work.task.created_at.clone(),
            },
            0,
            std::slice::from_ref(&work.item.item_id),
            std::slice::from_ref(&work.item),
        )
        .unwrap();
}

fn resume_scheduler_task(store: &mut TaskStore, work: &DownloadWork) {
    for (revision, kind, ids) in [
        (0, TaskStateCommandKind::ResumeTask, vec![]),
        (
            1,
            TaskStateCommandKind::ResumeItems,
            vec![work.item.item_id.clone()],
        ),
    ] {
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: work.task.task_id.clone(),
                expected_revision: revision,
                command: StoreTaskCommand {
                    kind,
                    item_ids: ids,
                },
                event_id: format!("{}:{revision}", work.task.task_id),
                occurred_at: "2026-08-30T00:00:02Z".to_owned(),
            })
            .unwrap();
    }
}

#[test]
fn actual_scheduler_store_cas_freezes_reloads_and_returns_canonical_plan_to_production_worker() {
    let (case, intent, bootstrap, _source) = concrete_fixture(
        "actual-scheduler-store",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
    );
    let work = work(&case, &intent);
    let database = case._root.0.join("actual-scheduler-store.sqlite");
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    let paused = store.load_task_record(&work.task.task_id).unwrap().unwrap();
    assert!(paused.runtime.task_paused);
    assert!(parse_nisar_subset_prepared_intent_v2_envelope(
        &paused.snapshot.candidates[0].metadata
    )
    .unwrap()
    .is_some());
    assert!(
        parse_nisar_subset_plan_v2_envelope(&paused.snapshot.candidates[0].metadata)
            .unwrap()
            .is_none()
    );
    resume_scheduler_task(&mut store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let (planned_tx, planned_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let worker = Arc::new(
        ProductionDownloadWorker::new(TransferOptions::default())
            .unwrap()
            .with_nisar_subset_v2_planning_fixture(bootstrap, planned_tx, finish_rx),
    );
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(25));
    scheduler.wake();
    let planned = planned_rx
        .recv_timeout(Duration::from_secs(20))
        .unwrap()
        .unwrap();
    assert_eq!(planned.bootstrap_statistics().science_request_count(), 0);
    let running = store
        .lock()
        .unwrap()
        .load_scheduler_task(&work.task.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(running.runtime.status, TaskLifecycleStatus::Running);
    assert_eq!(
        running.runtime.active_item_ids,
        std::slice::from_ref(&work.item.item_id)
    );
    let envelope = parse_nisar_subset_plan_v2_envelope(&running.snapshot.candidates[0].metadata)
        .unwrap()
        .unwrap();
    let reloaded = envelope.plan().unwrap();
    assert_eq!(reloaded, *planned.frozen_plan());
    assert_eq!(
        reloaded.canonical_json().unwrap(),
        planned.frozen_plan().canonical_json().unwrap()
    );
    finish_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = store
            .lock()
            .unwrap()
            .load_task_record(&work.task.task_id)
            .unwrap()
            .unwrap()
            .runtime
            .status;
        if status == TaskLifecycleStatus::Completed {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "scheduler did not complete fixture"
        );
        wait_for_route_progress();
    }
    scheduler.shutdown();
}

fn assert_production_route_v2_completion(
    label: &str,
    frequency: NisarSubsetFrequency,
    polarizations: &[&str],
) {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let frequency_label = match &frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    };
    let case = route_fixture(label, frequency.clone());
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, frequency.clone(), polarizations);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let work = route_ready_work(&case, &intent);
    let eligibility = crate::evaluate_resolver_cutover_eligibility(
        &work.item,
        &crate::AsfCollectionProfileRegistry,
        &crate::AssetExecutionPolicyRegistry,
    );
    assert!(
        matches!(eligibility, crate::ResolverCutoverEligibility::Eligible(_)),
        "route fixture must exercise resolver authority: {eligibility:?}"
    );
    let database = case._root.0.join(format!("{label}.sqlite"));
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    resume_scheduler_task(&mut store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let worker = Arc::new(route_worker(server.url.clone(), &case, &work.item));
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(25));
    scheduler.wake();

    let deadline = Instant::now() + Duration::from_secs(180);
    let completed = loop {
        let task = store
            .lock()
            .unwrap()
            .load_task_record(&work.task.task_id)
            .unwrap()
            .unwrap();
        if matches!(
            task.runtime.status,
            TaskLifecycleStatus::Completed | TaskLifecycleStatus::Failed
        ) {
            break task;
        }
        assert!(
            Instant::now() < deadline,
            "production route did not terminate"
        );
        wait_for_route_progress();
    };
    scheduler.shutdown();
    let requests = server.finish();
    let diagnostics = store
        .lock()
        .unwrap()
        .load_download_snapshot(&work.task.task_id);
    assert_eq!(
        completed.runtime.status,
        TaskLifecycleStatus::Completed,
        "production route result: {diagnostics:?}; requests={requests:?}"
    );
    assert!(!requests.is_empty());
    for (index, request) in requests.iter().enumerate() {
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("range: bytes="), "{request}");
        if index == 0 {
            assert!(!lower.contains("if-range:"), "{request}");
        } else {
            assert!(lower.contains("if-range: \"route-v2-etag\""), "{request}");
        }
        assert!(
            lower.contains("authorization: bearer route_v2_token"),
            "{request}"
        );
    }
    let recovery_workspace = Hdf5RangeWorkspaceV1::new(
        crate::production_worker::nisar_subset_v2_recovery_workspace_root(&work),
    )
    .unwrap();
    let planning_checkpoint: NisarSubsetPlanningWorkspaceCheckpointV2 = serde_json::from_str(
        &recovery_workspace
            .load_planning_checkpoint_v2()
            .unwrap()
            .expect("durable planning checkpoint"),
    )
    .unwrap();
    let planning_material_ranges = [
        planning_checkpoint.bootstrap_statistics.metadata_prefix(),
        planning_checkpoint
            .bootstrap_statistics
            .metadata_discovery(),
        planning_checkpoint
            .bootstrap_statistics
            .additional_geolocation(),
    ]
    .into_iter()
    .flat_map(|category| category.requested_ranges())
    .map(|range| (range.start(), range.end_exclusive()))
    .collect::<Vec<_>>();
    let planning_request_count = usize::try_from(
        planning_checkpoint
            .bootstrap_statistics
            .total_planning_request_count(),
    )
    .unwrap();
    assert!(planning_request_count < requests.len());
    let science_request_ranges = requests[planning_request_count..]
        .iter()
        .map(|request| request_range_bounds(request))
        .collect::<Vec<_>>();
    for late_dependency_path in [
        format!("/science/LSAR/RSLC/swaths/frequency{frequency_label}/inputDataExceptionMask"),
        "/science/LSAR/RSLC/metadata/orbit/time".to_owned(),
    ] {
        let ranges = planning_checkpoint
            .physical_plan
            .dependency_ranges()
            .iter()
            .filter(|range| {
                range
                    .logical_owners()
                    .iter()
                    .any(|owner| owner.dataset_path == late_dependency_path)
            })
            .map(|range| (range.start(), range.end_exclusive()))
            .collect::<Vec<_>>();
        assert!(
            !ranges.is_empty(),
            "missing late dependency: {late_dependency_path}"
        );
        for range in ranges {
            assert!(
                planning_material_ranges
                    .iter()
                    .all(|material| range.1 <= material.0 || range.0 >= material.1),
                "late dependency was planning-materialized: {late_dependency_path} {range:?} {planning_material_ranges:?}"
            );
            assert!(
                science_request_ranges.contains(&range),
                "late dependency was not fetched by execution transfer: {late_dependency_path} {range:?}"
            );
        }
    }
    let stored = store.lock().unwrap();
    let final_task = stored
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        final_task.snapshot.candidates[0].metadata["subset_frequency"],
        json!("B")
    );
    assert_eq!(
        final_task.snapshot.candidates[0].metadata["subset_polarization"],
        json!("VV")
    );
    let plan = parse_nisar_subset_plan_v2_envelope(&final_task.snapshot.candidates[0].metadata)
        .unwrap()
        .unwrap()
        .plan()
        .unwrap();
    assert_eq!(plan.selected_frequency, frequency);
    assert_eq!(
        plan.selected_polarizations,
        polarizations
            .iter()
            .map(|polarization| (*polarization).to_owned())
            .collect::<Vec<_>>()
    );
    let science_anchor = stored
        .load_nisar_subset_execution_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )
        .unwrap()
        .unwrap();
    let science_json: Value = serde_json::from_str(&science_anchor.canonical_json).unwrap();
    assert!(science_json["committed_checkpoint_sha256"].is_string());
    assert!(science_json["pending_transition"].is_null());
    let writer_anchor = stored
        .load_nisar_subset_execution_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            insar_store::NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND,
        )
        .unwrap()
        .unwrap();
    let writer_json: Value = serde_json::from_str(&writer_anchor.canonical_json).unwrap();
    assert_eq!(writer_json["state"], json!("Complete"));
    drop(stored);

    let mut artifacts = fs::read_dir(&work.task.output_root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("h5"))
        .collect::<Vec<_>>();
    artifacts.sort();
    assert_eq!(artifacts.len(), 1);
    let artifact = hdf5::File::open(&artifacts[0]).unwrap();
    let available = match frequency_label {
        "A" => &["HH", "HV"][..],
        "B" => &["HH", "HV", "VV"][..],
        _ => unreachable!(),
    };
    for polarization in available {
        let path = format!(
            "/science/LSAR/RSLC/derivedSubsets/component000/window000/swaths/frequency{frequency_label}/{polarization}"
        );
        assert_eq!(
            artifact.dataset(&path).is_ok(),
            polarizations.contains(polarization),
            "selected/unselected artifact mismatch at {path}"
        );
    }
    let sidecar = work.task.output_root.join(format!(
        "{}.artifact-manifest-v2.json",
        artifacts[0].file_name().unwrap().to_string_lossy()
    ));
    assert!(sidecar.is_file());
    assert_eq!(
        fs::read_dir(&work.task.output_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".artifact-manifest-v2.json")
            })
            .count(),
        1
    );
}

#[test]
fn production_route_v2_a_hh_runs_scheduler_remote_ranges_writer_and_store_completion() {
    assert_production_route_v2_completion(
        "production-route-a-hh",
        NisarSubsetFrequency::A,
        &["HH"],
    );
}

#[test]
fn production_route_v2_a_hh_hv_writes_one_multi_polarization_hdf5_and_sidecar() {
    assert_production_route_v2_completion(
        "production-route-a-hh-hv",
        NisarSubsetFrequency::A,
        &["HH", "HV"],
    );
}

#[test]
fn production_route_v2_b_hv_uses_the_same_scheduler_worker_and_range_chain() {
    assert_production_route_v2_completion(
        "production-route-b-hv",
        NisarSubsetFrequency::B,
        &["HV"],
    );
}

fn assert_production_route_v2_fail_closed(
    label: &str,
    mode: RouteServerMode,
    expected_code: &str,
) -> insar_core::DownloadFailure {
    let case = route_fixture(label, NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, mode);
    let work = route_ready_work(&case, &intent);
    let database = case._root.0.join(format!("{label}.sqlite"));
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    resume_scheduler_task(&mut store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    // D1c: a transient preflight failure is retried; the backoff is not slept.
    let worker = Arc::new(
        route_worker(server.url.clone(), &case, &work.item)
            .with_nisar_subset_v2_science_retry_backoff_for_test(Arc::new(|_, _| Duration::ZERO)),
    );
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(25));
    scheduler.wake();

    let deadline = Instant::now() + Duration::from_secs(20);
    let failed = loop {
        let task = store
            .lock()
            .unwrap()
            .load_task_record(&work.task.task_id)
            .unwrap()
            .unwrap();
        if task.runtime.status == TaskLifecycleStatus::Failed {
            break task;
        }
        assert_ne!(task.runtime.status, TaskLifecycleStatus::Completed);
        assert!(
            Instant::now() < deadline,
            "fail-closed route did not terminate"
        );
        wait_for_route_progress();
    };
    scheduler.shutdown();
    let requests = server.finish();
    let snapshot = store
        .lock()
        .unwrap()
        .load_download_snapshot(&work.task.task_id)
        .unwrap();
    assert_eq!(snapshot.results.len(), 1);
    let failure = snapshot.results[0]
        .failure
        .clone()
        .expect("failed route must retain a typed failure");
    assert_eq!(
        Some(failure.code.as_str()),
        Some(expected_code),
        "requests={requests:?}"
    );
    assert_eq!(failed.runtime.status, TaskLifecycleStatus::Failed);
    assert!(!requests.is_empty());
    // Every preflight attempt (bytes 0-0, before the source is bound) is
    // unbound; every later request carries the bound validator.
    let preflight_attempts = requests
        .iter()
        .take_while(|request| request.to_ascii_lowercase().contains("range: bytes=0-0"))
        .count()
        .max(1);
    assert!(requests.iter().enumerate().all(|(index, request)| {
        let lower = request.to_ascii_lowercase();
        lower.contains("range: bytes=")
            && if index < preflight_attempts {
                !lower.contains("if-range:")
            } else {
                lower.contains("if-range: \"route-v2-etag\"")
            }
            && lower.contains("authorization: bearer route_v2_token")
    }));
    let stored = store.lock().unwrap();
    let final_task = stored
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    assert!(
        parse_nisar_subset_plan_v2_envelope(&final_task.snapshot.candidates[0].metadata)
            .unwrap()
            .is_none()
    );
    assert!(stored
        .load_nisar_subset_execution_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )
        .unwrap()
        .is_none());
    drop(stored);
    assert_eq!(
        fs::read_dir(&work.task.output_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("h5"))
            .count(),
        0
    );
    failure
}

#[test]
fn production_route_v2_preserves_distinct_http_failures_without_v1_fallback() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    for (label, mode, expected_code) in [
        (
            "production-route-missing",
            RouteServerMode::Missing,
            "NISAR_SUBSET_V2_SOURCE_UNAVAILABLE",
        ),
        (
            "production-route-200",
            RouteServerMode::FullResponse,
            "NISAR_SUBSET_V2_RANGE_UNSUPPORTED",
        ),
        (
            "production-route-malformed-content-range",
            RouteServerMode::MalformedContentRange,
            "NISAR_SUBSET_V2_RESPONSE_INVALID",
        ),
        (
            "production-route-partial-body",
            RouteServerMode::PartialBody,
            "NISAR_SUBSET_V2_PARTIAL_RESPONSE",
        ),
        (
            "production-route-transport",
            RouteServerMode::TransportFailure,
            "NISAR_SUBSET_V2_TRANSPORT_FAILED",
        ),
        (
            "production-route-etag-drift",
            RouteServerMode::ValidatorDriftAfter(1),
            "NISAR_SUBSET_V2_VALIDATOR_CHANGED",
        ),
    ] {
        let failure = assert_production_route_v2_fail_closed(label, mode, expected_code);
        // D1c: a transport failure of the preflight is retried twice; the
        // failure names the three attempts.
        if matches!(mode, RouteServerMode::TransportFailure) {
            assert!(
                failure.reason_zh.contains("stage=preflight")
                    && failure.reason_zh.contains("attempts=3"),
                "{}",
                failure.reason_zh
            );
        }
    }
}

/// L4 S-band regression (2026-10-18, downloader enhancement D1c): one
/// request timeout of the preflight Range (bytes 0-0, stage=preflight) failed
/// the item at once. A transient preflight failure is now retried like a
/// science Range: up to three attempts, waiting the bounded science backoff
/// (5 s, then 20 s; here recorded and not slept) before each retry.
#[test]
fn production_route_v2_retries_a_transient_preflight_failure() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let label = "production-route-preflight-retry";
    let case = route_fixture(label, NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::PreflightResetOnce);
    let work = route_ready_work(&case, &intent);
    let database = case._root.0.join(format!("{label}.sqlite"));
    let mut store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut store, &work);
    resume_scheduler_task(&mut store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(store));
    let backoffs = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&backoffs);
    let worker = Arc::new(
        route_worker(server.url.clone(), &case, &work.item)
            .with_nisar_subset_v2_science_retry_backoff_for_test(Arc::new(
                move |retry, http_status| {
                    recorded.lock().unwrap().push((retry, http_status));
                    Duration::ZERO
                },
            )),
    );
    let scheduler =
        DownloadScheduler::start_with_store(Arc::clone(&store), worker, Duration::from_millis(25));
    scheduler.wake();
    wait_for_route_completed(&store, &work.task.task_id);
    scheduler.shutdown();
    let requests = server.finish();
    let preflights = requests
        .iter()
        .take(2)
        .filter(|request| request.to_ascii_lowercase().contains("range: bytes=0-0"))
        .count();
    assert_eq!(preflights, 2, "the preflight was sent again: {requests:?}");
    assert!(
        requests
            .iter()
            .take(2)
            .all(|request| !request.to_ascii_lowercase().contains("if-range:")),
        "a preflight attempt is unbound: {requests:?}"
    );
    assert_eq!(
        backoffs.lock().unwrap().first().copied(),
        Some((1, None)),
        "the first retry waited the first science backoff"
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoutePauseBoundary {
    AfterPlanning,
    AfterFirstScienceRange,
}

fn update_route_item_state(
    store: &mut TaskStore,
    work: &DownloadWork,
    kind: TaskStateCommandKind,
    label: &str,
) {
    let revision = store
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap()
        .runtime
        .revision;
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: work.task.task_id.clone(),
            expected_revision: revision,
            command: StoreTaskCommand {
                item_ids: if matches!(
                    kind,
                    TaskStateCommandKind::PauseTask
                        | TaskStateCommandKind::ResumeTask
                        | TaskStateCommandKind::CancelTask
                ) {
                    vec![]
                } else {
                    vec![work.item.item_id.clone()]
                },
                kind,
            },
            event_id: format!("{}:{label}:{revision}", work.task.task_id),
            occurred_at: "2026-08-30T00:00:03Z".to_owned(),
        })
        .unwrap();
}

fn wait_for_route_outcome(store: &SharedTaskStore, task_id: &str, expected: DownloadOutcome) {
    // A post-promotion outcome follows every exact dependency + science transfer;
    // keep the bounded wait aligned with the complete production-route deadline.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let snapshot = store
            .lock()
            .unwrap()
            .load_download_snapshot(task_id)
            .unwrap();
        if snapshot
            .results
            .iter()
            .any(|result| result.outcome == expected)
        {
            return;
        }
        if Instant::now() >= deadline {
            let task = store
                .lock()
                .unwrap()
                .load_task_record(task_id)
                .unwrap()
                .unwrap();
            panic!(
                "route outcome was not persisted: snapshot={snapshot:?} runtime={:?}",
                task.runtime
            );
        }
        wait_for_route_progress();
    }
}

fn wait_for_route_completed(store: &SharedTaskStore, task_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let status = store
            .lock()
            .unwrap()
            .load_task_record(task_id)
            .unwrap()
            .unwrap()
            .runtime
            .status;
        if status == TaskLifecycleStatus::Completed {
            return;
        }
        if status == TaskLifecycleStatus::Failed {
            let snapshot = store
                .lock()
                .unwrap()
                .load_download_snapshot(task_id)
                .unwrap();
            panic!("recovered route failed: {snapshot:?}");
        }
        if Instant::now() >= deadline {
            let snapshot = store
                .lock()
                .unwrap()
                .load_download_snapshot(task_id)
                .unwrap();
            panic!("recovered route did not complete: status={status:?} snapshot={snapshot:?}");
        }
        wait_for_route_progress();
    }
}

fn request_range_value(request: &str) -> String {
    request
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("range: bytes=")
                .map(str::to_owned)
        })
        .expect("exact Range request")
}

fn request_range_bounds(request: &str) -> (u64, u64) {
    let value = request_range_value(request);
    let (start, inclusive_end) = value.split_once('-').expect("bounded Range request");
    (
        start.parse().expect("Range start"),
        inclusive_end
            .parse::<u64>()
            .expect("Range inclusive end")
            .checked_add(1)
            .expect("Range exclusive end"),
    )
}

fn assert_production_route_v2_pause_reopen_resume(boundary: RoutePauseBoundary) {
    let label = match boundary {
        RoutePauseBoundary::AfterPlanning => "production-route-reopen-after-planning",
        RoutePauseBoundary::AfterFirstScienceRange => "production-route-reopen-after-science",
    };
    let case = route_fixture(label, NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let work = route_ready_work(&case, &intent);
    let database = case._root.0.join(format!("{label}.sqlite"));
    let server_url = server.url.clone();
    let item = work.item.clone();
    assert_route_v2_pause_reopen_resume(boundary, false, work, database, server, || {
        route_worker(server_url.clone(), &case, &item)
    });
}

/// Share the lifecycle assertions, not the provider or scientific fixture.
fn assert_route_v2_pause_reopen_resume(
    boundary: RoutePauseBoundary,
    task_level: bool,
    work: DownloadWork,
    database: PathBuf,
    server: RouteRangeServer,
    new_worker: impl Fn() -> ProductionDownloadWorker,
) -> Vec<String> {
    server.delayed_body.store(true, Ordering::SeqCst);
    let mut initial_store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut initial_store, &work);
    resume_scheduler_task(&mut initial_store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(initial_store));
    let (boundary_tx, boundary_rx) = mpsc::channel();
    let worker = new_worker();
    let worker = match boundary {
        RoutePauseBoundary::AfterPlanning => worker
            .with_nisar_subset_v2_after_planning_hook_for_test(Arc::new(move |control| {
                boundary_tx.send(()).unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while !control.is_paused() {
                    assert!(Instant::now() < deadline, "planning pause was not observed");
                    wait_for_route_progress();
                }
            })),
        RoutePauseBoundary::AfterFirstScienceRange => worker
            .with_nisar_subset_v2_after_science_commit_hook_for_test(Arc::new(
                move |index, control| {
                    assert_eq!(index, 0);
                    boundary_tx.send(()).unwrap();
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while !control.is_paused() {
                        assert!(Instant::now() < deadline, "science pause was not observed");
                        wait_for_route_progress();
                    }
                },
            )),
    };
    let worker = Arc::new(worker);
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&store),
        worker.clone(),
        Duration::from_millis(25),
    );
    scheduler.wake();
    boundary_rx.recv_timeout(Duration::from_secs(20)).unwrap();
    let body_sample = worker
        .task_transfer_sample(&work.task.task_id)
        .expect("actual V2 HTTP bodies must publish task telemetry");
    assert!(body_sample.network_bytes_received > 0);
    assert!(body_sample.active_millis > 0);
    assert_eq!(
        body_sample.active_count, 0,
        "planning/commit CPU is not body time"
    );
    thread::sleep(Duration::from_millis(25));
    assert_eq!(
        worker.task_transfer_sample(&work.task.task_id).unwrap(),
        body_sample
    );
    let first_science_range = (boundary == RoutePauseBoundary::AfterFirstScienceRange)
        .then(|| request_range_value(server.requests.lock().unwrap().last().unwrap()));
    update_route_item_state(
        &mut store.lock().unwrap(),
        &work,
        if task_level {
            TaskStateCommandKind::PauseTask
        } else {
            TaskStateCommandKind::PauseItems
        },
        "pause",
    );
    scheduler.wake();
    wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Interrupted);
    let paused = store
        .lock()
        .unwrap()
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    assert_ne!(paused.runtime.status, TaskLifecycleStatus::Completed);
    if task_level {
        assert!(paused.runtime.task_paused);
    } else {
        assert!(paused.runtime.paused_item_ids.contains(&work.item.item_id));
    }
    let first_frozen_plan =
        parse_nisar_subset_plan_v2_envelope(&paused.snapshot.candidates[0].metadata)
            .unwrap()
            .unwrap()
            .plan()
            .unwrap()
            .canonical_json()
            .unwrap();
    let science_anchor_before = store
        .lock()
        .unwrap()
        .load_nisar_subset_execution_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )
        .unwrap();
    assert_eq!(
        science_anchor_before.is_some(),
        first_science_range.is_some()
    );
    if let Some(anchor) = &science_anchor_before {
        let checkpoint: Value = serde_json::from_slice(
            &fs::read(route_recovery_root(&work).join("science-checkpoint-v2.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            checkpoint["state"],
            json!("Interrupted"),
            "the Store-backed anchor must permit the durable pause transition"
        );
        assert!(checkpoint["pending_attempt"].is_null());
        assert_eq!(checkpoint["completed_ranges"].as_array().unwrap().len(), 1);
        let record: Value = serde_json::from_str(&anchor.canonical_json).unwrap();
        assert_eq!(
            record["committed_checkpoint_sha256"],
            checkpoint["checkpoint_sha256"]
        );
    }
    scheduler.shutdown();

    let persisted_before = store
        .lock()
        .unwrap()
        .task_transfer_checkpoint_baseline(&work.task.task_id)
        .unwrap();
    assert_eq!(
        persisted_before,
        (
            body_sample.active_millis,
            body_sample.network_bytes_received
        )
    );
    drop(worker);

    let reopened = Arc::try_unwrap(store).ok().unwrap().into_inner().unwrap();
    drop(reopened);
    let mut reopened = TaskStore::open(&database).unwrap();
    update_route_item_state(
        &mut reopened,
        &work,
        if task_level {
            TaskStateCommandKind::ResumeTask
        } else {
            TaskStateCommandKind::ResumeItems
        },
        "resume",
    );
    if task_level {
        // ApplicationService::update_download_task resumes the interrupted
        // items after ResumeTask. This lower-level Store/Worker fixture must
        // execute that same pair, not invent a stronger core ResumeTask command.
        update_route_item_state(
            &mut reopened,
            &work,
            TaskStateCommandKind::ResumeItems,
            "resume-interrupted-item",
        );
    }
    let reopened: SharedTaskStore = Arc::new(Mutex::new(reopened));
    let (resumed_planning_tx, resumed_planning_rx) = mpsc::channel();
    let (resumed_science_tx, resumed_science_rx) = mpsc::channel();
    let resumed_worker = new_worker()
        .with_nisar_subset_v2_after_planning_hook_for_test(Arc::new(move |_| {
            resumed_planning_tx.send(()).unwrap();
        }))
        .with_nisar_subset_v2_after_science_commit_hook_for_test(Arc::new(move |index, _| {
            resumed_science_tx.send(index).unwrap();
        }));
    let resumed_scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&reopened),
        Arc::new(resumed_worker),
        Duration::from_millis(25),
    );
    resumed_scheduler.wake();
    resumed_planning_rx
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|error| {
            panic!(
                "resumed planning did not reopen: {error:?}; requests={:?}",
                server.requests.lock().unwrap()
            )
        });
    let expected_resumed_range_index = match boundary {
        RoutePauseBoundary::AfterPlanning => 0,
        RoutePauseBoundary::AfterFirstScienceRange => 1,
    };
    assert_eq!(
        resumed_science_rx
            .recv_timeout(Duration::from_secs(30))
            .unwrap_or_else(|error| {
                panic!(
                    "resumed science did not issue its first range: {error:?}; requests={:?}",
                    server.requests.lock().unwrap()
                )
            }),
        expected_resumed_range_index
    );
    wait_for_route_completed(&reopened, &work.task.task_id);
    resumed_scheduler.shutdown();
    let requests = server.finish();

    let persisted_after = reopened
        .lock()
        .unwrap()
        .task_transfer_checkpoint_baseline(&work.task.task_id)
        .unwrap();
    assert!(persisted_after.0 > persisted_before.0);
    assert!(persisted_after.1 > persisted_before.1);
    let physically_requested_bytes: u64 = requests
        .iter()
        .map(|request| {
            let range = request_range_value(request);
            let (start, end) = range.split_once('-').unwrap();
            end.parse::<u64>().unwrap() - start.parse::<u64>().unwrap() + 1
        })
        .sum();
    assert_eq!(
        persisted_after.1, physically_requested_bytes,
        "cold resume must accumulate only bytes actually read, excluding checkpoint reuse"
    );

    let completed = reopened
        .lock()
        .unwrap()
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    let final_frozen_plan =
        parse_nisar_subset_plan_v2_envelope(&completed.snapshot.candidates[0].metadata)
            .unwrap()
            .unwrap()
            .plan()
            .unwrap()
            .canonical_json()
            .unwrap();
    assert_eq!(first_frozen_plan, final_frozen_plan);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request_range_value(request) == "0-0")
            .count(),
        2,
        "frozen-plan restart must not perform a second semantic bootstrap"
    );
    if let Some(first_science_range) = first_science_range {
        assert_eq!(
            requests
                .iter()
                .filter(|request| request_range_value(request) == first_science_range)
                .count(),
            1,
            "verified science range was fetched again after reopen"
        );
    }
    requests
}

#[test]
fn production_route_v2_freeze_boundary_pause_and_cancel_are_nonfailed_terminal_controls() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    for (label, command, outcome, status) in [
        (
            "production-route-freeze-pause",
            TaskStateCommandKind::PauseItems,
            DownloadOutcome::Interrupted,
            TaskLifecycleStatus::Paused,
        ),
        (
            "production-route-freeze-cancel",
            TaskStateCommandKind::CancelItems,
            DownloadOutcome::Cancelled,
            TaskLifecycleStatus::Cancelled,
        ),
    ] {
        let case = route_fixture(label, NisarSubsetFrequency::A);
        let source = fs::read(&case.source_path).unwrap();
        let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
        let (server, freeze_gate) = RouteRangeServer::start_with_freeze_gate(source, ROUTE_ETAG);
        let work = route_ready_work(&case, &intent);
        let database = case._root.0.join(format!("{label}.sqlite"));
        let mut initial_store = TaskStore::open(&database).unwrap();
        seed_scheduler_task(&mut initial_store, &work);
        resume_scheduler_task(&mut initial_store, &work);
        let store: SharedTaskStore = Arc::new(Mutex::new(initial_store));
        let (scheduler, handler_gate) =
            DownloadScheduler::start_with_store_and_nisar_freeze_handler_gate_for_test(
                Arc::clone(&store),
                Arc::new(route_worker(server.url.clone(), &case, &work.item)),
                Duration::from_millis(25),
            );
        scheduler.wake();
        freeze_gate.wait_until_received();
        freeze_gate.release();
        handler_gate.wait_until_entered();
        update_route_item_state(
            &mut store.lock().unwrap(),
            &work,
            command,
            "freeze-boundary-control",
        );
        handler_gate.release();
        wait_for_route_outcome(&store, &work.task.task_id, outcome.clone());
        scheduler.shutdown();
        let requests = server.finish();
        let stored = store.lock().unwrap();
        let final_task = stored
            .load_task_record(&work.task.task_id)
            .unwrap()
            .unwrap();
        assert_eq!(final_task.runtime.status, status);
        if outcome == DownloadOutcome::Interrupted {
            assert!(final_task
                .runtime
                .paused_item_ids
                .contains(&work.item.item_id));
        } else {
            assert!(final_task
                .runtime
                .cancelled_item_ids
                .contains(&work.item.item_id));
        }
        assert!(
            parse_nisar_subset_plan_v2_envelope(&final_task.snapshot.candidates[0].metadata)
                .unwrap()
                .is_none()
        );
        let snapshot = stored.load_download_snapshot(&work.task.task_id).unwrap();
        let result = snapshot.results.last().unwrap();
        assert_eq!(result.outcome, outcome);
        assert!(
            result.failure.is_none(),
            "pause/cancel must not persist a synthetic provider failure"
        );
        assert!(!matches!(
            final_task.runtime.status,
            TaskLifecycleStatus::Failed | TaskLifecycleStatus::Completed
        ));
        assert!(requests.len() > 1);
        assert!(!requests[0].to_ascii_lowercase().contains("if-range:"));
        assert!(requests[1..].iter().all(|request| request
            .to_ascii_lowercase()
            .contains("if-range: \"route-v2-etag\"")));
    }
}

fn pause_route_after_planning_once(
    case: &ReadyCase,
    server_url: &str,
    work: &DownloadWork,
    database: &Path,
) {
    let mut initial_store = TaskStore::open(database).unwrap();
    seed_scheduler_task(&mut initial_store, work);
    resume_scheduler_task(&mut initial_store, work);
    let store: SharedTaskStore = Arc::new(Mutex::new(initial_store));
    let (boundary_tx, boundary_rx) = mpsc::channel();
    let worker = route_worker(server_url.to_owned(), case, &work.item)
        .with_nisar_subset_v2_after_planning_hook_for_test(Arc::new(move |control| {
            boundary_tx.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !control.is_paused() {
                assert!(Instant::now() < deadline, "planning pause was not observed");
                wait_for_route_progress();
            }
        }));
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&store),
        Arc::new(worker),
        Duration::from_millis(25),
    );
    scheduler.wake();
    boundary_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    update_route_item_state(
        &mut store.lock().unwrap(),
        work,
        TaskStateCommandKind::PauseItems,
        "isolation-pause",
    );
    scheduler.wake();
    wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Interrupted);
    scheduler.shutdown();
}

#[test]
fn production_route_v2_recovery_workspace_is_task_item_scoped_under_shared_output_root() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let case = route_fixture(
        "production-route-workspace-isolation",
        NisarSubsetFrequency::A,
    );
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let work_a = route_ready_work(&case, &intent);
    let mut work_b = work_a.clone();
    work_b.task.task_id = "production-route-workspace-task-b".to_owned();
    work_b.task.batch_id = "production-route-workspace-batch-b".to_owned();
    assert_eq!(work_a.task.output_root, work_b.task.output_root);
    let root_a = route_recovery_root(&work_a);
    let root_b = route_recovery_root(&work_b);
    assert_ne!(root_a, root_b);
    let mut changed_item = work_a.clone();
    changed_item.item.item_id = "production-route-workspace-item-b".to_owned();
    assert_ne!(root_a, route_recovery_root(&changed_item));

    pause_route_after_planning_once(
        &case,
        &server.url,
        &work_a,
        &case._root.0.join("production-route-workspace-a.sqlite"),
    );
    let checkpoint_a_path = root_a.join("planning-workspace-v2.json");
    let checkpoint_a = fs::read(&checkpoint_a_path).unwrap();
    pause_route_after_planning_once(
        &case,
        &server.url,
        &work_b,
        &case._root.0.join("production-route-workspace-b.sqlite"),
    );
    assert_eq!(fs::read(&checkpoint_a_path).unwrap(), checkpoint_a);
    let checkpoint_b = fs::read(root_b.join("planning-workspace-v2.json")).unwrap();
    let checkpoint_a_json: Value = serde_json::from_slice(&checkpoint_a).unwrap();
    let checkpoint_b_json: Value = serde_json::from_slice(&checkpoint_b).unwrap();
    assert_eq!(checkpoint_a_json["task_id"], json!(work_a.task.task_id));
    assert_eq!(checkpoint_b_json["task_id"], json!(work_b.task.task_id));
    assert_eq!(checkpoint_a_json["item_id"], json!(work_a.item.item_id));
    assert_eq!(checkpoint_b_json["item_id"], json!(work_b.item.item_id));
    for checkpoint in [&checkpoint_a_json, &checkpoint_b_json] {
        let budget = &checkpoint["physical_plan"]["combined_budget"];
        assert_eq!(
            budget["budget_policy"],
            json!("physical_execution_union_v2")
        );
        let count = budget["projected_science_request_count"].as_u64().unwrap();
        let planning = budget["committed_planning_request_count"].as_u64().unwrap();
        assert_eq!(
            budget["max_request_count"].as_u64().unwrap(),
            planning + 44 * count
        );
    }
    assert_ne!(checkpoint_a, checkpoint_b);
    let _ = server.finish();
}

#[test]
fn production_route_v2_pause_shutdown_reopen_resume_reuses_plan_and_verified_ranges() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    for boundary in [
        RoutePauseBoundary::AfterPlanning,
        RoutePauseBoundary::AfterFirstScienceRange,
    ] {
        assert_production_route_v2_pause_reopen_resume(boundary);
    }
}

#[test]
fn production_route_v2_post_promotion_crash_recovers_sidecar_before_store_completion() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    let case = route_fixture("production-route-post-promotion", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let work = route_ready_work(&case, &intent);
    let database = case._root.0.join("production-route-post-promotion.sqlite");
    let mut initial_store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut initial_store, &work);
    resume_scheduler_task(&mut initial_store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(initial_store));
    let worker = route_worker(server.url.clone(), &case, &work.item)
        .with_nisar_subset_v2_writer_failpoint_for_test(
            crate::nisar_subset_writer::WriterFailpointV2::AfterArtifactPromotion,
        );
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&store),
        Arc::new(worker),
        Duration::from_millis(25),
    );
    scheduler.wake();
    wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Interrupted);
    update_route_item_state(
        &mut store.lock().unwrap(),
        &work,
        TaskStateCommandKind::PauseItems,
        "post-promotion-pause",
    );
    scheduler.wake();
    let interrupted = store
        .lock()
        .unwrap()
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    assert_ne!(interrupted.runtime.status, TaskLifecycleStatus::Completed);
    let writer_anchor = store
        .lock()
        .unwrap()
        .load_nisar_subset_execution_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            insar_store::NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND,
        )
        .unwrap()
        .unwrap();
    let writer_json: Value = serde_json::from_str(&writer_anchor.canonical_json).unwrap();
    assert_eq!(writer_json["state"], json!("ArtifactPromoted"));
    let artifact = fs::read_dir(&work.task.output_root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("h5"))
        .unwrap();
    let sidecar = work.task.output_root.join(format!(
        "{}.artifact-manifest-v2.json",
        artifact.file_name().unwrap().to_string_lossy()
    ));
    assert!(!sidecar.exists());
    let requests_before_restart = server.requests.lock().unwrap().len();
    scheduler.shutdown();

    let reopened = Arc::try_unwrap(store).ok().unwrap().into_inner().unwrap();
    drop(reopened);
    let mut reopened = TaskStore::open(&database).unwrap();
    update_route_item_state(
        &mut reopened,
        &work,
        TaskStateCommandKind::ResumeItems,
        "post-promotion-resume",
    );
    let reopened: SharedTaskStore = Arc::new(Mutex::new(reopened));
    let resumed_scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&reopened),
        Arc::new(route_worker(server.url.clone(), &case, &work.item)),
        Duration::from_millis(25),
    );
    resumed_scheduler.wake();
    wait_for_route_completed(&reopened, &work.task.task_id);
    resumed_scheduler.shutdown();
    let requests = server.finish();
    assert_eq!(requests.len(), requests_before_restart);
    assert!(artifact.is_file());
    assert!(sidecar.is_file());
    let final_anchor = reopened
        .lock()
        .unwrap()
        .load_nisar_subset_execution_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            insar_store::NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND,
        )
        .unwrap()
        .unwrap();
    let final_json: Value = serde_json::from_str(&final_anchor.canonical_json).unwrap();
    assert_eq!(final_json["state"], json!("Complete"));
}

#[derive(Clone, Copy, Debug)]
enum RouteRecoveryCorruption {
    CorruptPlanningCheckpoint,
    MissingSparsePart,
    CorruptSparsePart,
    CorruptScienceAnchor,
    ValidatorDrift,
}

impl RouteRecoveryCorruption {
    fn label(self) -> &'static str {
        match self {
            Self::CorruptPlanningCheckpoint => "corrupt-planning-checkpoint",
            Self::MissingSparsePart => "missing-sparse-part",
            Self::CorruptSparsePart => "corrupt-sparse-part",
            Self::CorruptScienceAnchor => "corrupt-science-anchor",
            Self::ValidatorDrift => "validator-drift-after-freeze",
        }
    }

    fn boundary(self) -> RoutePauseBoundary {
        match self {
            Self::CorruptPlanningCheckpoint | Self::MissingSparsePart | Self::ValidatorDrift => {
                RoutePauseBoundary::AfterPlanning
            }
            Self::CorruptSparsePart | Self::CorruptScienceAnchor => {
                RoutePauseBoundary::AfterFirstScienceRange
            }
        }
    }

    fn expected_code(self) -> &'static str {
        match self {
            Self::CorruptPlanningCheckpoint
            | Self::MissingSparsePart
            | Self::CorruptSparsePart
            | Self::CorruptScienceAnchor => "NISAR_SUBSET_V2_RECOVERY_INVALID",
            Self::ValidatorDrift => "NISAR_SUBSET_V2_VALIDATOR_CHANGED",
        }
    }
}

fn assert_production_route_v2_corrupt_recovery_fails_closed(corruption: RouteRecoveryCorruption) {
    let label = format!("production-route-{}", corruption.label());
    let case = route_fixture(&label, NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let work = route_ready_work(&case, &intent);
    let database = case._root.0.join(format!("{label}.sqlite"));
    let mut initial_store = TaskStore::open(&database).unwrap();
    seed_scheduler_task(&mut initial_store, &work);
    resume_scheduler_task(&mut initial_store, &work);
    let store: SharedTaskStore = Arc::new(Mutex::new(initial_store));
    let (boundary_tx, boundary_rx) = mpsc::channel();
    let worker = route_worker(server.url.clone(), &case, &work.item);
    let worker = match corruption.boundary() {
        RoutePauseBoundary::AfterPlanning => worker
            .with_nisar_subset_v2_after_planning_hook_for_test(Arc::new(move |control| {
                boundary_tx.send(()).unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while !control.is_paused() {
                    assert!(Instant::now() < deadline, "planning pause was not observed");
                    wait_for_route_progress();
                }
            })),
        RoutePauseBoundary::AfterFirstScienceRange => worker
            .with_nisar_subset_v2_after_science_commit_hook_for_test(Arc::new(
                move |index, control| {
                    assert_eq!(index, 0);
                    boundary_tx.send(()).unwrap();
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while !control.is_paused() {
                        assert!(Instant::now() < deadline, "science pause was not observed");
                        wait_for_route_progress();
                    }
                },
            )),
    };
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&store),
        Arc::new(worker),
        Duration::from_millis(25),
    );
    scheduler.wake();
    boundary_rx.recv_timeout(Duration::from_secs(20)).unwrap();
    update_route_item_state(
        &mut store.lock().unwrap(),
        &work,
        TaskStateCommandKind::PauseItems,
        "corruption-pause",
    );
    scheduler.wake();
    wait_for_route_outcome(&store, &work.task.task_id, DownloadOutcome::Interrupted);
    scheduler.shutdown();
    if matches!(corruption, RouteRecoveryCorruption::ValidatorDrift) {
        // Drift only after the frozen source-version authority exists. This is
        // independent of the number of planning requests required by a fixture.
        server.enable_validator_drift();
    }

    let recovery_root = route_recovery_root(&work);
    match corruption {
        RouteRecoveryCorruption::CorruptPlanningCheckpoint => {
            fs::write(recovery_root.join("planning-workspace-v2.json"), b"{}").unwrap();
        }
        RouteRecoveryCorruption::MissingSparsePart => {
            fs::remove_file(recovery_root.join("ranges.part")).unwrap();
        }
        RouteRecoveryCorruption::CorruptSparsePart => {
            let first_science_start =
                request_range_value(server.requests.lock().unwrap().last().unwrap())
                    .split_once('-')
                    .unwrap()
                    .0
                    .parse::<u64>()
                    .unwrap();
            let mut part = OpenOptions::new()
                .read(true)
                .write(true)
                .open(recovery_root.join("ranges.part"))
                .unwrap();
            part.seek(SeekFrom::Start(first_science_start)).unwrap();
            let mut byte = [0_u8; 1];
            part.read_exact(&mut byte).unwrap();
            byte[0] ^= 0xff;
            part.seek(SeekFrom::Start(first_science_start)).unwrap();
            part.write_all(&byte).unwrap();
            part.sync_all().unwrap();
        }
        RouteRecoveryCorruption::CorruptScienceAnchor => {
            let existing = store
                .lock()
                .unwrap()
                .load_nisar_subset_execution_anchor_v2(
                    &work.task.task_id,
                    &work.item.item_id,
                    insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
                )
                .unwrap()
                .unwrap();
            assert!(store
                .lock()
                .unwrap()
                .compare_and_swap_nisar_subset_execution_anchor_v2(
                    &StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
                        task_id: work.task.task_id.clone(),
                        item_id: work.item.item_id.clone(),
                        anchor_kind: insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
                            .to_owned(),
                        lineage_key_sha256: existing.lineage_key_sha256,
                        expected_record_sha256: Some(existing.record_sha256),
                        record_sha256: "0".repeat(64),
                        canonical_json: "{}".to_owned(),
                    },
                )
                .unwrap());
        }
        RouteRecoveryCorruption::ValidatorDrift => {}
    }

    let reopened = Arc::try_unwrap(store).ok().unwrap().into_inner().unwrap();
    drop(reopened);
    let mut reopened = TaskStore::open(&database).unwrap();
    update_route_item_state(
        &mut reopened,
        &work,
        TaskStateCommandKind::ResumeItems,
        "corruption-resume",
    );
    let reopened: SharedTaskStore = Arc::new(Mutex::new(reopened));
    let resumed_scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&reopened),
        Arc::new(route_worker(server.url.clone(), &case, &work.item)),
        Duration::from_millis(25),
    );
    resumed_scheduler.wake();
    wait_for_route_outcome(&reopened, &work.task.task_id, DownloadOutcome::Failed);
    resumed_scheduler.shutdown();
    let requests = server.finish();
    let final_task = reopened
        .lock()
        .unwrap()
        .load_task_record(&work.task.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(final_task.runtime.status, TaskLifecycleStatus::Failed);
    let snapshot = reopened
        .lock()
        .unwrap()
        .load_download_snapshot(&work.task.task_id)
        .unwrap();
    let failure = snapshot
        .results
        .iter()
        .rev()
        .find_map(|result| result.failure.as_ref())
        .unwrap();
    assert_eq!(
        failure.code,
        corruption.expected_code(),
        "requests={requests:?}"
    );
}

#[test]
fn production_route_v2_corrupt_recovery_authority_fails_closed_without_store_completion() {
    let _hdf5_guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::establish_hdf5_plugin_isolation().unwrap();
    for corruption in [
        RouteRecoveryCorruption::CorruptPlanningCheckpoint,
        RouteRecoveryCorruption::MissingSparsePart,
        RouteRecoveryCorruption::CorruptSparsePart,
        RouteRecoveryCorruption::CorruptScienceAnchor,
        RouteRecoveryCorruption::ValidatorDrift,
    ] {
        assert_production_route_v2_corrupt_recovery_fails_closed(corruption);
    }
}

#[test]
fn metadata_prefix_v2_four_explicit_stops_preserve_charges_without_retry_exhaustion() {
    let root = PrefixLedgerTestRoot::new("four-explicit-stops");
    let workspace = Hdf5RangeWorkspaceV1::new(root.0.join("workspace")).unwrap();
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    let mut stopped = prefix_retry_failure_observation((0, 4), 12, 2);
    stopped.failure_code = "NISAR_SUBSET_V2_EXECUTION_STOPPED";
    for count in 1..=4 {
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        assert_eq!(session.attempts_for_range((0, 4)).unwrap(), 0);
        session.record_failed_attempt(&stopped, false).unwrap();
        let state = &session.checkpoint.attempt_states[0];
        assert_eq!(state.attempts, count);
        assert_eq!(state.failed_request_count, count);
        assert_eq!(state.failed_response_body_bytes, count * 2);
        assert_eq!(state.failure_observations.len() as u64, count);
    }
    let reopened = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &workspace,
        12,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();
    assert_eq!(reopened.attempts_for_range((0, 4)).unwrap(), 0);
    assert_eq!(
        reopened.persisted_failure_observations().unwrap(),
        vec![stopped; 4]
    );
}

#[test]
fn metadata_prefix_v2_three_network_failures_interleaved_with_stops_block_next_request() {
    let case = route_fixture("prefix-three-failures-with-stops", NisarSubsetFrequency::A);
    let source = fs::read(&case.source_path).unwrap();
    let intent = route_prepared_intent(&case, NisarSubsetFrequency::A, &["HH"]);
    let ranges = remote_planning_initial_material_ranges(source.len() as u64).unwrap();
    let workspace_root = case
        ._root
        .0
        .join("prefix-three-failures-with-stops-workspace");
    let validator = NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: ROUTE_ETAG.to_owned(),
        content_length: source.len() as u64,
    };
    let failure = prefix_retry_failure_observation(ranges[0], source.len() as u64, 3);
    let mut stopped = failure.clone();
    stopped.failure_code = "NISAR_SUBSET_V2_EXECUTION_STOPPED";
    let workspace = Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
    for count in 1..=3 {
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize_deferred_fixture(
            &workspace,
            source.len() as u64,
            &digest_serializable(&validator).unwrap(),
            &ranges,
            &prefix_source_identity_sha256(&intent).unwrap(),
        )
        .unwrap();
        session.record_failed_attempt(&failure, true).unwrap();
        if count < 3 {
            session.record_failed_attempt(&stopped, false).unwrap();
        }
        assert_eq!(session.attempts_for_range(ranges[0]).unwrap(), count);
    }
    let server = RouteRangeServer::start(source, ROUTE_ETAG, RouteServerMode::Exact);
    let (result, _) = run_prefix_bootstrap_probe(&case, &intent, &server.url, &workspace_root);
    let requests = server.finish();
    assert!(matches!(
        result,
        Err(NisarSubsetPlanningErrorV2::BootstrapTransport)
    ));
    for range in ranges {
        assert_eq!(
            requests_for_range(&requests, range),
            0,
            "explicit resume must not grant a fourth metadata-prefix network attempt"
        );
    }
}

#[test]
fn metadata_prefix_v2_terminal_failure_cannot_be_reopened_by_appending_control_stop() {
    let root = PrefixLedgerTestRoot::new("terminal-then-stop");
    let workspace = Hdf5RangeWorkspaceV1::new(root.0.join("workspace")).unwrap();
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &workspace,
        12,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();
    session
        .record_failed_attempt(&prefix_terminal_failure_observation((0, 4), 12), false)
        .unwrap();
    let path = workspace.planning_prefix_checkpoint_path_v2_for_test();
    let before = fs::read(&path).unwrap();
    let mut stopped = prefix_retry_failure_observation((0, 4), 12, 0);
    stopped.failure_code = "NISAR_SUBSET_V2_EXECUTION_STOPPED";
    assert_eq!(
        session.record_failed_attempt(&stopped, false),
        Err(NisarSubsetPlanningErrorV2::RecoveryInvalid)
    );
    assert_eq!(
        fs::read(path).unwrap(),
        before,
        "rejected append must preserve durable evidence"
    );
    assert_eq!(
        session.attempts_for_range((0, 4)),
        Err(NisarSubsetPlanningErrorV2::BootstrapSourceUnavailable)
    );
}

#[test]
fn metadata_prefix_v2_completed_history_preserves_stop_and_network_failure_on_reopen() {
    let root = PrefixLedgerTestRoot::new("completed-stop-history");
    let workspace = Hdf5RangeWorkspaceV1::new(root.0.join("workspace")).unwrap();
    let ranges = prefix_test_ranges(12, 4);
    let validator = prefix_validator_digest("etag-a");
    let failure = prefix_retry_failure_observation((0, 4), 12, 3);
    let mut stopped = prefix_retry_failure_observation((0, 4), 12, 2);
    stopped.failure_code = "NISAR_SUBSET_V2_EXECUTION_STOPPED";
    {
        let mut session = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        session.record_failed_attempt(&failure, true).unwrap();
        session.record_failed_attempt(&stopped, false).unwrap();
    }
    {
        let mut resumed = NisarPrefixMaterializationSessionV2::open_or_initialize(
            &workspace,
            12,
            &validator,
            &ranges,
            PREFIX_SOURCE_IDENTITY_SHA256,
        )
        .unwrap();
        assert_eq!(resumed.attempts_for_range((0, 4)).unwrap(), 1);
        resumed.materialize_verified_range((0, 4), b"abcd").unwrap();
    }
    let reopened = NisarPrefixMaterializationSessionV2::open_or_initialize(
        &workspace,
        12,
        &validator,
        &ranges,
        PREFIX_SOURCE_IDENTITY_SHA256,
    )
    .unwrap();
    assert_eq!(reopened.verified_ranges(), &[(0, 4)]);
    assert_eq!(reopened.incomplete_ranges(), vec![(4, 8), (8, 12)]);
    assert!(reopened.checkpoint.attempt_states.is_empty());
    assert_eq!(
        reopened.persisted_failure_observations().unwrap(),
        vec![failure, stopped]
    );
    let charges = reopened
        .persisted_failure_observations()
        .unwrap()
        .iter()
        .map(actual_failure_charge_v2)
        .fold((0_u64, 0_u64), |(requests, bytes), charge| {
            (requests + charge.0, bytes + charge.1)
        });
    assert_eq!(charges, (2, 5));
}

#[test]
fn metadata_prefix_v2_body_pause_after_first_64k_stops_before_full_16mib_response() {
    const BODY_BYTES: u64 = 16 * 1024 * 1024;
    const PAUSE_AFTER: u64 = 64 * 1024;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let paused = Arc::new(AtomicBool::new(false));
    let server_paused = Arc::clone(&paused);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let request = read_science_request(&mut stream);
        assert!(request
            .to_ascii_lowercase()
            .contains("range: bytes=0-16777215"));
        let headers = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {BODY_BYTES}\r\nContent-Range: bytes 0-{}/{BODY_BYTES}\r\nETag: {ROUTE_ETAG}\r\nConnection: close\r\n\r\n", BODY_BYTES - 1);
        stream.write_all(headers.as_bytes()).unwrap();
        let chunk = [7_u8; PAUSE_AFTER as usize];
        for _ in 0..(BODY_BYTES / PAUSE_AFTER) {
            if let Err(error) = stream.write_all(&chunk) {
                assert!(
                    server_paused.load(Ordering::SeqCst),
                    "server write failed before the client requested pause: {error}"
                );
                break;
            }
        }
    });
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let response = client
        .get(format!("http://{address}/source.h5"))
        .header("Range", "bytes=0-16777215")
        .send()
        .unwrap();
    let control = WorkerControl::default();
    let mut observed_bytes = 0_u64;
    let activity = Arc::new(crate::TaskTransferActivityRegistry::default());
    let binding = crate::transfer_activity::TaskBodyActivity::new(
        Arc::clone(&activity),
        "prefix-body-task".to_owned(),
        "prefix-body-item".to_owned(),
    );
    let result = parse_exact_range_response(
        crate::http_transfer::ExactRangeSendOutcome::Response {
            response,
            request_count: 1,
        },
        0,
        BODY_BYTES,
        BODY_BYTES,
        Some(ROUTE_ETAG),
        &control,
        &mut |delta| {
            observed_bytes += delta;
            let sample = activity.sample("prefix-body-task", Instant::now()).unwrap();
            assert_eq!(sample.active_count, 1);
            assert_eq!(sample.network_bytes_received, observed_bytes);
            if observed_bytes >= PAUSE_AFTER {
                paused.store(true, Ordering::SeqCst);
                control.set_paused_for_test(true);
            }
        },
        Some(&binding),
    );
    server.join().unwrap();
    let failure = result.expect_err("pause must return a stopped observation");
    assert_eq!(failure.error, NisarSubsetPlanningErrorV2::ExecutionStopped);
    let stopped_sample = activity.sample("prefix-body-task", Instant::now()).unwrap();
    assert_eq!(stopped_sample.network_bytes_received, observed_bytes);
    assert_eq!(stopped_sample.active_count, 0);
    thread::sleep(Duration::from_millis(10));
    assert_eq!(
        activity.sample("prefix-body-task", Instant::now()).unwrap(),
        stopped_sample
    );
    assert!(observed_bytes >= PAUSE_AFTER);
    assert!(observed_bytes < PAUSE_AFTER * 2,
        "pause must stop within the first callback that crosses 64KiB, not drain the 16MiB body: {observed_bytes}");
    assert_eq!(
        failure.observation.response_body_bytes,
        Some(observed_bytes)
    );
    assert_eq!(
        actual_failure_charge_v2(&failure.observation),
        (1, observed_bytes)
    );
}
