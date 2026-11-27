use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{
    parse_canonical_http_date, supported_datatype_sha256, ApprovedHdf5ChunkSetV1,
    Hdf5ChunkAllocationV1, Hdf5ChunkDatatypeV1, Hdf5ChunkFilterPipelineV1,
    Hdf5ChunkPhysicalRangeV1, Hdf5FillValueDefinitionV1, Hdf5RangePlanV1, Hdf5RangeTransferError,
    Hdf5RangeWorkspaceV1, OfflineHdf5RangeTransfer,
};
use crate::{
    CredentialMaterial, DestinationLeaseController, HttpTransfer, ProxyPolicy, RemoteValidator,
    ResolvedAsset, ResolvedChecksum, ResolvedMethod, TransferOptions,
};
use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy,
    AssetResolverParameters, AssetRole, FrozenAssetAccess, NisarSubsetRangeState,
    NisarSubsetRequestManifestV1, NisarSubsetTransferState, SnapshotItem, StableAssetReference,
};
use serde_json::{json, Map};

const ETAG: &str = "\"nisar-range-v1\"";
const TIMESTAMP: &str = "2026-08-01T00:00:00Z";

#[cfg(windows)]
#[test]
fn archived_recovery_owned_zero_length_part_resumes_initialization_without_truncating_data() {
    let root = TempDirectory::new("archived-zero-part");
    let workspace = Hdf5RangeWorkspaceV1::new(root.0.join("workspace")).unwrap();
    let lock = workspace.acquire_lock().unwrap();
    workspace
        .persist_archived_import_marker_v2("[\"fixture-owned-import\"]", &lock)
        .unwrap();
    fs::File::create(workspace.part_path()).unwrap(); // Crash between create_new and set_len.
    let part = workspace
        .open_archived_import_part_v2(4096, &lock)
        .expect("an empty, owned import part can complete its interrupted initialization");
    assert_eq!(part.metadata().unwrap().len(), 4096);
    drop(part);
    assert!(
        workspace.open_archived_import_part_v2(8192, &lock).is_err(),
        "a nonempty mismatched part must never be resized"
    );
    assert_eq!(fs::metadata(workspace.part_path()).unwrap().len(), 4096);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn nisar_live_network_budget_fails_before_overshoot() {
    let source_size = 14_252_244_992_u64;
    let max_response_body_bytes =
        (source_size * 10 / 100).min(super::NISAR_LIVE_MAX_RESPONSE_BODY_BYTES);
    let budget = super::NisarLiveNetworkBudgetV1::new(source_size).unwrap();

    for _ in 0..super::NISAR_LIVE_MAX_REQUESTS {
        budget.reserve_request().unwrap();
    }
    assert_eq!(
        budget.reserve_request(),
        Err(Hdf5RangeTransferError::RangeCountExceeded)
    );
    budget
        .reserve_response_body_bytes(max_response_body_bytes)
        .unwrap();
    assert_eq!(
        budget.reserve_response_body_bytes(1),
        Err(Hdf5RangeTransferError::RangeBytesExceeded)
    );
    assert!(budget
        .validate_actual(super::NISAR_LIVE_MAX_REQUESTS, max_response_body_bytes)
        .is_ok());
}

#[test]
fn p04_success_cleanup_removes_only_the_task_scoped_range_workspace() {
    let root = std::env::temp_dir().join(format!(
        "insar-p04-range-cleanup-{}-{}",
        std::process::id(),
        TIMESTAMP.replace([':', '-'], "")
    ));
    let workspace_root = root.join(".insar-nisar-range").join("task-1");
    let workspace = super::Hdf5RangeWorkspaceV1::new(workspace_root.clone()).unwrap();
    fs::write(workspace.part_path(), b"resume-state").unwrap();

    workspace.cleanup_after_success().unwrap();

    assert!(!workspace_root.exists());
    assert!(
        root.exists(),
        "cleanup must not remove the task output root"
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn c2_network_budget_admits_the_bounded_plan_and_still_enforces_the_ratio() {
    let source_size = 5_280_628_736_u64;
    let planned_response_bytes = 465_986_898_u64;
    let ratio_limit = source_size * 10 / 100;
    assert!(planned_response_bytes < ratio_limit);
    assert!(ratio_limit < super::NISAR_LIVE_MAX_RESPONSE_BODY_BYTES);
    let budget = super::NisarLiveNetworkBudgetV1::new(source_size).unwrap();
    budget
        .reserve_response_body_bytes(planned_response_bytes)
        .unwrap();
    budget
        .reserve_response_body_bytes(ratio_limit - planned_response_bytes)
        .unwrap();
    assert_eq!(
        budget.reserve_response_body_bytes(1),
        Err(Hdf5RangeTransferError::RangeBytesExceeded)
    );
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-hdf5-range-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(windows)]
#[test]
fn planning_part_uses_sparse_allocation_before_ranges_are_materialized() {
    let root = TempDirectory::new("planning-sparse");
    let workspace = Hdf5RangeWorkspaceV1::new(root.0.join("workspace")).unwrap();
    let source_size = 64 * 1024 * 1024_u64;
    let mut part = workspace.reset_planning_part_v2(source_size).unwrap();

    assert_eq!(part.metadata().unwrap().len(), source_size);
    let allocated = super::science_allocated_range_evidence_v2(&mut part, 0, source_size).unwrap();
    assert!(
        allocated.intervals.is_empty(),
        "fresh planning recovery file must not allocate the full source HDF5"
    );
}

struct LocalServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl LocalServer {
    fn new(responses: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                captured.lock().unwrap().push(read_request(&mut stream));
                stream.write_all(&response).unwrap();
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

fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut value = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for (name, content) in headers {
        value.push_str(&format!("{name}: {content}\r\n"));
    }
    value.push_str("\r\n");
    let mut bytes = value.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

#[cfg(windows)]
fn hold_workspace_lock(path: &std::path::Path) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;

    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .share_mode(0x0000_0001 | 0x0000_0002)
        .custom_flags(0x0020_0000)
        .open(path)
        .unwrap();
    file.try_lock().unwrap();
    file
}

#[cfg(not(windows))]
fn hold_workspace_lock(path: &std::path::Path) -> fs::File {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    file.try_lock().unwrap();
    file
}

fn partial_response(start: u64, end_exclusive: u64, body: &[u8], etag: &str) -> Vec<u8> {
    partial_response_with_total(start, end_exclusive, 64, body, etag)
}

fn partial_response_with_total(
    start: u64,
    end_exclusive: u64,
    total: u64,
    body: &[u8],
    etag: &str,
) -> Vec<u8> {
    response(
        "206 Partial Content",
        &[
            ("ETag", etag),
            (
                "Content-Range",
                &format!("bytes {start}-{}/{total}", end_exclusive - 1),
            ),
            ("Content-Length", &(end_exclusive - start).to_string()),
        ],
        body,
    )
}

fn small_request(remote_bytes: u64) -> NisarSubsetRequestManifestV1 {
    let mut request = request();
    request.source.expected_size_bytes = 64;
    request.expected_output.full_product_bytes = 64;
    request.expected_output.estimated_remote_bytes = remote_bytes + 4;
    request.expected_output.estimated_output_bytes = remote_bytes + 4;
    request.validate().unwrap();
    request
}

fn plan(request: &NisarSubsetRequestManifestV1, ranges: &[(u64, u64)]) -> Hdf5RangePlanV1 {
    let mut chunks: Vec<Hdf5ChunkPhysicalRangeV1> = ranges
        .iter()
        .map(|(start, end_exclusive)| physical(0, *start, *end_exclusive))
        .collect();
    let mask = mask_physical(0, 56, 60);
    chunks.push(mask);
    Hdf5RangePlanV1::from_offline_evidence(request, evidence(request, chunks)).unwrap()
}

fn asset(url: &str, etag: Option<&str>) -> ResolvedAsset {
    asset_with_size(url, etag, 64)
}

fn asset_with_size(url: &str, etag: Option<&str>, expected_size: u64) -> ResolvedAsset {
    ResolvedAsset::new(url, ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(expected_size))
        .with_checksum(Some(
            ResolvedChecksum::new(
                "sha256",
                "ee3de9b9877b4f8e35153369725dcb4174ef5f2423af167e77f843e08860a9a0",
            )
            .unwrap(),
        ))
        .with_remote_validator(RemoteValidator {
            etag: etag.map(str::to_owned),
            last_modified: None,
            content_length: Some(expected_size),
        })
}

fn transfer() -> HttpTransfer {
    transfer_with_attempts(1)
}

fn transfer_with_attempts(max_attempts: u32) -> HttpTransfer {
    HttpTransfer::new_with_proxy_policy(
        TransferOptions {
            max_attempts,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            chunk_size: 8,
            ..TransferOptions::default()
        },
        ProxyPolicy::Direct,
    )
    .unwrap()
}

fn create_runtime(
    temp: &TempDirectory,
    request: &NisarSubsetRequestManifestV1,
    asset: ResolvedAsset,
) -> (OfflineHdf5RangeTransfer, Hdf5RangeWorkspaceV1) {
    let workspace = Hdf5RangeWorkspaceV1::new(temp.0.clone()).unwrap();
    let runtime = OfflineHdf5RangeTransfer::new(
        transfer(),
        request,
        &snapshot(request),
        asset,
        workspace.clone(),
        DestinationLeaseController::new_for_qualification_test(),
    )
    .unwrap();
    (runtime, workspace)
}

fn snapshot(request: &NisarSubsetRequestManifestV1) -> SnapshotItem {
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

fn request() -> NisarSubsetRequestManifestV1 {
    let path = format!(
        "{}/../../docs/fixtures/asf-nisar-subset-p1/planner-request-v1.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn physical(component_id: u16, start: u64, end_exclusive: u64) -> Hdf5ChunkPhysicalRangeV1 {
    physical_at(component_id, [51, 25], start, end_exclusive)
}

fn mask_physical(component_id: u16, start: u64, end_exclusive: u64) -> Hdf5ChunkPhysicalRangeV1 {
    let mut chunk = physical(component_id, start, end_exclusive);
    chunk.dataset_path = "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask".to_owned();
    chunk.datatype = Hdf5ChunkDatatypeV1::UnsignedU8;
    chunk.datatype_sha256 = supported_datatype_sha256(Hdf5ChunkDatatypeV1::UnsignedU8).unwrap();
    chunk.fill_value_hex = Some("00".to_owned());
    chunk
}

fn physical_at(
    component_id: u16,
    logical_chunk: [u64; 2],
    start: u64,
    end_exclusive: u64,
) -> Hdf5ChunkPhysicalRangeV1 {
    Hdf5ChunkPhysicalRangeV1 {
        dataset_path: "/science/LSAR/RSLC/swaths/frequencyA/HH".to_owned(),
        component_id,
        logical_chunk,
        dataset_shape: [70_000, 30_000],
        chunk_shape: [512, 512],
        datatype: Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
        datatype_sha256: supported_datatype_sha256(Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi)
            .unwrap(),
        fill_value_definition: Hdf5FillValueDefinitionV1::Default,
        fill_value_hex: Some("0000000000000000".to_owned()),
        filter_pipeline: Hdf5ChunkFilterPipelineV1::None,
        allocation: Hdf5ChunkAllocationV1::Allocated {
            filter_mask: 0,
            stored_size: end_exclusive - start,
            start,
            end_exclusive,
        },
    }
}

fn fill_chunk(component_id: u16, element_fill_hex: &str) -> Hdf5ChunkPhysicalRangeV1 {
    Hdf5ChunkPhysicalRangeV1 {
        dataset_path: "/science/LSAR/RSLC/swaths/frequencyA/HH".to_owned(),
        component_id,
        logical_chunk: [51, 25],
        dataset_shape: [70_000, 30_000],
        chunk_shape: [512, 512],
        datatype: Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
        datatype_sha256: supported_datatype_sha256(Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi)
            .unwrap(),
        fill_value_definition: Hdf5FillValueDefinitionV1::Default,
        fill_value_hex: Some(element_fill_hex.to_owned()),
        filter_pipeline: Hdf5ChunkFilterPipelineV1::None,
        allocation: Hdf5ChunkAllocationV1::Fill {
            element_fill_hex: element_fill_hex.to_owned(),
        },
    }
}

fn evidence(
    request: &NisarSubsetRequestManifestV1,
    chunks: Vec<Hdf5ChunkPhysicalRangeV1>,
) -> ApprovedHdf5ChunkSetV1 {
    ApprovedHdf5ChunkSetV1 {
        request_manifest_sha256: request.sha256().unwrap(),
        source_length: request.expected_output.full_product_bytes,
        layout_evidence_digest: "86a615c78d3a9d1407ec3b5f4ddc76a10a5aff157b473bba48d9d31865e71500"
            .to_owned(),
        metadata_evidence_sha256:
            "96a615c78d3a9d1407ec3b5f4ddc76a10a5aff157b473bba48d9d31865e71500".to_owned(),
        chunks,
    }
}

#[cfg(feature = "nisar-subset-production")]
#[test]
fn ssar_physical_range_catalog_uses_ssar_science_metadata_and_geolocation_only() {
    use crate::nisar_science_root::NisarScienceRoot;

    let temp = TempDirectory::new("ssar-physical-range-catalog");
    let source = temp.0.join("fixture-ssar.h5");
    let file = hdf5::File::create(&source).unwrap();

    let ssar_frequency = file
        .create_group("/science/SSAR/RSLC/swaths/frequencyA")
        .unwrap();
    for (name, seed) in [("RH", 1_u8), ("RV", 9_u8)] {
        let values = vec![seed; 64];
        ssar_frequency
            .new_dataset::<u8>()
            .shape((8, 8))
            .chunk((4, 4))
            .create(name)
            .and_then(|dataset| dataset.write_raw(&values))
            .unwrap();
    }
    ssar_frequency
        .new_dataset::<f64>()
        .shape((8,))
        .create("slantRange")
        .and_then(|dataset| {
            dataset.write_raw(&[
                800_000.0, 800_010.0, 800_020.0, 800_030.0, 800_040.0, 800_050.0, 800_060.0,
                800_070.0,
            ])
        })
        .unwrap();
    let ssar_swaths = file.group("/science/SSAR/RSLC/swaths").unwrap();
    ssar_swaths
        .new_dataset::<f64>()
        .shape((8,))
        .create("zeroDopplerTime")
        .and_then(|dataset| {
            dataset.write_raw(&[
                1000.0, 1000.5, 1001.0, 1001.5, 1002.0, 1002.5, 1003.0, 1003.5,
            ])
        })
        .unwrap();

    let geolocation = file
        .create_group("/science/SSAR/RSLC/metadata/geolocationGrid")
        .unwrap();
    geolocation
        .new_dataset::<f64>()
        .shape((8,))
        .create("zeroDopplerTime")
        .and_then(|dataset| dataset.write_raw(&[0.0_f64; 8]))
        .unwrap();

    let identification = file.create_group("/science/SSAR/identification").unwrap();
    identification
        .new_dataset::<u8>()
        .shape((16,))
        .create("missionId")
        .and_then(|dataset| dataset.write_raw(&[b'N'; 16]))
        .unwrap();

    let lsar_frequency = file
        .create_group("/science/LSAR/RSLC/swaths/frequencyA")
        .unwrap();
    lsar_frequency
        .new_dataset::<u8>()
        .shape((32, 32))
        .chunk((8, 8))
        .create("HH")
        .and_then(|dataset| dataset.write_raw(&vec![77_u8; 32 * 32]))
        .unwrap();

    file.flush().unwrap();
    drop(file);

    let selected = NisarScienceRoot::Ssar.selected_science_path("A", "RH");
    let catalog = super::inspect_nisar_planning_physical_ranges_v2(
        &source,
        &[selected],
        NisarScienceRoot::Ssar,
    )
    .unwrap();

    assert!(!catalog.selected_science_ranges.is_empty());
    assert!(!catalog.all_science_ranges.is_empty());
    assert!(!catalog.metadata_ranges.is_empty());
    assert!(!catalog.geolocation_ranges.is_empty());

    let file = hdf5::File::open(&source).unwrap();
    for path in [
        "/science/SSAR/RSLC/swaths/zeroDopplerTime",
        "/science/SSAR/RSLC/swaths/frequencyA/slantRange",
    ] {
        let mut axis_ranges = Vec::new();
        super::push_dataset_ranges(&file.dataset(path).unwrap(), &mut axis_ranges).unwrap();
        assert!(!axis_ranges.is_empty());
        assert!(axis_ranges.iter().all(|axis| {
            catalog
                .geolocation_ranges
                .iter()
                .any(|range| range.0 <= axis.0 && range.1 >= axis.1)
        }));
    }
    let mut identification_ranges = Vec::new();
    super::push_dataset_ranges(
        &file
            .dataset("/science/SSAR/identification/missionId")
            .unwrap(),
        &mut identification_ranges,
    )
    .unwrap();
    assert!(!identification_ranges.is_empty());
    assert!(identification_ranges.iter().all(|identification| {
        catalog
            .metadata_ranges
            .iter()
            .any(|range| range.0 <= identification.0 && range.1 >= identification.1)
    }));
    let mut lsar_ranges = Vec::new();
    super::push_dataset_ranges(
        &file
            .dataset("/science/LSAR/RSLC/swaths/frequencyA/HH")
            .unwrap(),
        &mut lsar_ranges,
    )
    .unwrap();
    assert!(!lsar_ranges.is_empty());
    assert!(catalog.all_science_ranges.iter().all(|ssar| {
        lsar_ranges
            .iter()
            .all(|lsar| ssar.0 >= lsar.1 || lsar.0 >= ssar.1)
    }));
}

#[test]
fn logical_windows_map_to_sorted_bounded_coalesced_ranges() {
    let mut request = request();
    request.mapping.windows[0].validity_mask_required = false;
    request.mapping.windows[0].line_end_exclusive += 1024;
    request.mapping.windows[0].expected_shape[0] += 1024;
    request.expected_output.estimated_remote_bytes = 12;
    let plan = Hdf5RangePlanV1::from_offline_evidence(
        &request,
        evidence(
            &request,
            vec![
                physical_at(0, [53, 25], 20, 24),
                physical_at(0, [51, 25], 8, 12),
                physical_at(0, [52, 25], 12, 16),
            ],
        ),
    )
    .unwrap();
    assert_eq!(
        plan.ranges()
            .iter()
            .map(|range| (range.start, range.end_exclusive))
            .collect::<Vec<_>>(),
        vec![(8, 16), (20, 24)]
    );
    assert_eq!(plan.expected_remote_bytes(), 12);
    assert_eq!(plan.ranges()[0].member_chunk_indices(), &[0, 1]);
    assert_eq!(plan.ranges()[1].member_chunk_indices(), &[2]);
    assert_eq!(plan.chunks().len(), 3);
    assert_eq!(plan.plan_sha256().len(), 64);

    let reordered = Hdf5RangePlanV1::from_offline_evidence(
        &request,
        evidence(
            &request,
            vec![
                physical_at(0, [52, 25], 12, 16),
                physical_at(0, [53, 25], 20, 24),
                physical_at(0, [51, 25], 8, 12),
            ],
        ),
    )
    .unwrap();
    assert_eq!(reordered.plan_sha256(), plan.plan_sha256());
}

#[test]
fn range_plan_rejects_unknown_windows_overlap_budget_and_count() {
    let mut request = request();
    request.mapping.windows[0].validity_mask_required = false;
    request.expected_output.estimated_remote_bytes = 8;

    let mut unknown = physical(1, 0, 8);
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(&request, evidence(&request, vec![unknown.clone()]))
            .unwrap_err()
            .code(),
        "NISAR_SUBSET_RANGE_INVALID"
    );
    unknown.component_id = 0;
    unknown.dataset_path = "/science/LSAR/RSLC/swaths/frequencyB/HH".to_owned();
    assert!(
        Hdf5RangePlanV1::from_offline_evidence(&request, evidence(&request, vec![unknown]))
            .is_err()
    );

    let mut overlap_request = request.clone();
    overlap_request.mapping.windows[0].line_end_exclusive += 512;
    overlap_request.mapping.windows[0].expected_shape[0] += 512;
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(
            &overlap_request,
            evidence(&overlap_request, {
                let first = physical(0, 0, 6);
                let second = physical_at(0, [52, 25], 4, 8);
                vec![first, second]
            })
        )
        .unwrap_err(),
        Hdf5RangeTransferError::RangeOverlap
    );

    let too_many = (0..=4096)
        .map(|index| physical(0, index * 2, index * 2 + 1))
        .collect();
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(&request, evidence(&request, too_many)).unwrap_err(),
        Hdf5RangeTransferError::RangeCountExceeded
    );

    request.expected_output.estimated_remote_bytes = super::HDF5_RANGE_MAX_REMOTE_BYTES + 1;
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(
            &request,
            evidence(
                &request,
                vec![physical(0, 0, super::HDF5_RANGE_MAX_REMOTE_BYTES + 1)],
            )
        )
        .unwrap_err(),
        Hdf5RangeTransferError::RangeBytesExceeded
    );

    request.expected_output.estimated_remote_bytes = 8;
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(
            &request,
            evidence(&request, vec![physical(0, 0, 4), physical(0, 8, 12)])
        )
        .unwrap_err(),
        Hdf5RangeTransferError::ChunkIdentityDuplicate
    );

    request.expected_output.estimated_remote_bytes = 4;
    let mut filtered = physical(0, 0, 4);
    if let Hdf5ChunkAllocationV1::Allocated { filter_mask, .. } = &mut filtered.allocation {
        *filter_mask = 1;
    }
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(&request, evidence(&request, vec![filtered]))
            .unwrap_err(),
        Hdf5RangeTransferError::RangeInvalid
    );

    let mut wrong_datatype = physical(0, 0, 4);
    wrong_datatype.datatype = Hdf5ChunkDatatypeV1::UnsignedU8;
    wrong_datatype.fill_value_hex = Some("00".to_owned());
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(&request, evidence(&request, vec![wrong_datatype]))
            .unwrap_err(),
        Hdf5RangeTransferError::RangeInvalid
    );

    let mut forged_datatype_digest = physical(0, 0, 4);
    forged_datatype_digest.datatype_sha256 = "2".repeat(64);
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(
            &request,
            evidence(&request, vec![forged_datatype_digest])
        )
        .unwrap_err(),
        Hdf5RangeTransferError::RangeInvalid
    );

    let mut wrong_fill = fill_chunk(0, "0100000000000000");
    wrong_fill.fill_value_hex = Some("0000000000000000".to_owned());
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(&request, evidence(&request, vec![wrong_fill]))
            .unwrap_err(),
        Hdf5RangeTransferError::RangeInvalid
    );
}

#[test]
fn physical_chunks_bind_to_requested_science_or_frequency_level_mask_windows() {
    let mut request = request();
    request.expected_output.estimated_remote_bytes = 8;

    let science = physical(0, 0, 4);
    let mut mask = mask_physical(0, 8, 12);
    assert!(Hdf5RangePlanV1::from_offline_evidence(
        &request,
        evidence(&request, vec![science.clone(), mask.clone()])
    )
    .is_ok());

    mask.dataset_path = "/science/LSAR/RSLC/swaths/frequencyA/HH/inputDataExceptionMask".to_owned();
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(
            &request,
            evidence(&request, vec![science.clone(), mask])
        )
        .unwrap_err(),
        Hdf5RangeTransferError::WindowIdentityMismatch
    );

    let mut outside = physical(0, 0, 4);
    outside.logical_chunk = [0, 0];
    assert_eq!(
        Hdf5RangePlanV1::from_offline_evidence(
            &request,
            evidence(&request, vec![outside, science.clone()])
        )
        .unwrap_err(),
        Hdf5RangeTransferError::WindowIdentityMismatch
    );

    let mut independently_chunked_masks = Vec::new();
    for (offset, logical_chunk) in [[102, 50], [102, 51], [103, 50], [103, 51]]
        .into_iter()
        .enumerate()
    {
        let mut mask = mask_physical(0, 8 + offset as u64, 9 + offset as u64);
        mask.logical_chunk = logical_chunk;
        mask.dataset_shape = [80_000, 40_000];
        mask.chunk_shape = [256, 256];
        independently_chunked_masks.push(mask);
    }
    let mut independent = vec![science.clone()];
    independent.extend(independently_chunked_masks);
    let independent_plan =
        Hdf5RangePlanV1::from_offline_evidence(&request, evidence(&request, independent)).unwrap();
    assert_eq!(independent_plan.expected_remote_bytes(), 8);

    let mut fill_request = request.clone();
    fill_request.expected_output.estimated_remote_bytes = 4;
    let mask = mask_physical(0, 0, 4);
    let fill_plan = Hdf5RangePlanV1::from_offline_evidence(
        &fill_request,
        evidence(&fill_request, vec![fill_chunk(0, "0000000000000000"), mask]),
    )
    .unwrap();
    assert_eq!(fill_plan.chunks().len(), 2);
    assert_eq!(fill_plan.ranges().len(), 1);
    assert_eq!(fill_plan.expected_remote_bytes(), 4);
}

#[test]
fn distinct_windows_may_share_one_physical_science_and_mask_chunk() {
    let mut request = request();
    let original = request.mapping.windows[0].clone();
    let split_line = original.line_start + 256;
    let mut first = original.clone();
    first.line_end_exclusive = split_line;
    first.expected_shape[0] = 256;
    let mut second = original;
    second.line_start = split_line;
    second.expected_shape[0] = 256;
    request.mapping.windows = vec![first, second];
    request.expected_output.estimated_remote_bytes = 8;
    request.expected_output.estimated_output_bytes = 8;
    request.validate().unwrap();

    let science = physical(0, 0, 4);
    let mask = mask_physical(0, 56, 60);
    let plan =
        Hdf5RangePlanV1::from_offline_evidence(&request, evidence(&request, vec![science, mask]))
            .unwrap();
    assert_eq!(plan.chunks().len(), 2);
    assert_eq!(plan.expected_remote_bytes(), 8);
}

#[test]
fn distinct_components_deduplicate_the_same_exact_physical_chunks() {
    let mut request = request();
    let mut second = request.mapping.windows[0].clone();
    second.component_id = 1;
    request.request.aoi_component_count = 2;
    request.mapping.windows.push(second);
    request.expected_output.estimated_remote_bytes = 8;
    request.expected_output.estimated_output_bytes = 8;
    request.validate().unwrap();

    let science_zero = physical(0, 0, 4);
    let mut science_one = science_zero.clone();
    science_one.component_id = 1;
    let mask_zero = mask_physical(0, 56, 60);
    let mut mask_one = mask_zero.clone();
    mask_one.component_id = 1;
    let plan = Hdf5RangePlanV1::from_offline_evidence(
        &request,
        evidence(
            &request,
            vec![science_zero, science_one, mask_zero, mask_one],
        ),
    )
    .unwrap();
    assert_eq!(plan.chunks().len(), 4);
    assert_eq!(plan.ranges().len(), 2);
    assert!(plan
        .ranges()
        .iter()
        .all(|range| range.member_chunk_indices().len() == 2));
    assert_eq!(plan.expected_remote_bytes(), 8);
}

#[test]
fn last_modified_requires_a_canonical_valid_http_date() {
    for valid in [
        "Wed, 01 Jan 2025 00:00:00 GMT",
        "Sun, 06 Nov 1994 08:49:37 GMT",
        "Thu, 29 Feb 2024 23:59:59 GMT",
    ] {
        assert_eq!(parse_canonical_http_date(valid).as_deref(), Some(valid));
    }
    for invalid in [
        "date",
        "Wed, 01 Jan 2025 00:00:00 UTC",
        "Wed, 32 Jan 2025 00:00:00 GMT",
        "Wed, 29 Feb 2023 00:00:00 GMT",
        "Tue, 01 Jan 2025 00:00:00 GMT",
        "Wednesday, 01-Jan-25 00:00:00 GMT",
    ] {
        assert_eq!(parse_canonical_http_date(invalid), None, "{invalid}");
    }
}

#[test]
fn exact_206_writes_only_the_approved_range_and_completes_manifest() {
    let server = LocalServer::new(vec![
        partial_response(4, 8, b"EFGH", ETAG),
        partial_response(56, 60, b"MASK", ETAG),
    ]);
    let request = small_request(4);
    let plan = plan(&request, &[(4, 8)]);
    let temp = TempDirectory::new("exact");
    let mut resolved = asset(&server.url, Some(ETAG));
    CredentialMaterial::bearer("OFFLINE_RANGE_TOKEN")
        .unwrap()
        .apply_to(&mut resolved)
        .unwrap();
    let server_url = server.url.clone();
    let (runtime, workspace) = create_runtime(&temp, &request, resolved);

    let result = runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    assert_eq!(
        result.manifest().state,
        NisarSubsetTransferState::RangesComplete
    );
    assert_eq!(result.metrics().request_count, 2);
    assert_eq!(result.metrics().response_body_bytes, 8);
    assert_eq!(result.metrics().accepted_unique_bytes, 8);

    let mut part = fs::File::open(workspace.part_path()).unwrap();
    part.seek(SeekFrom::Start(4)).unwrap();
    let mut selected = [0_u8; 4];
    part.read_exact(&mut selected).unwrap();
    assert_eq!(&selected, b"EFGH");
    assert_eq!(part.metadata().unwrap().len(), 64);

    let captured = server.finish();
    assert_eq!(captured.len(), 2);
    let request_text = captured[0].to_ascii_lowercase();
    assert!(request_text.contains("range: bytes=4-7"));
    assert!(request_text.contains("if-range: \"nisar-range-v1\""));
    assert!(request_text.contains("accept-encoding: identity"));
    assert!(request_text.contains("authorization: bearer offline_range_token"));
    assert!(captured[1]
        .to_ascii_lowercase()
        .contains("range: bytes=56-59"));

    for sidecar in [
        fs::read_to_string(workspace.manifest_path()).unwrap(),
        fs::read_to_string(workspace.plan_path()).unwrap(),
        fs::read_to_string(workspace.metrics_path()).unwrap(),
        fs::read_to_string(workspace.part_identity_path()).unwrap(),
    ] {
        assert!(!sidecar.contains("OFFLINE_RANGE_TOKEN"));
        assert!(!sidecar.contains(&server_url));
    }
}

#[test]
fn recovery_sidecars_are_bounded_single_link_and_no_follow() {
    let server = LocalServer::new(vec![
        partial_response(4, 8, b"EFGH", ETAG),
        partial_response(56, 60, b"MASK", ETAG),
    ]);
    let request = small_request(4);
    let plan = plan(&request, &[(4, 8)]);
    let temp = TempDirectory::new("guarded-sidecars");
    let (runtime, workspace) = create_runtime(&temp, &request, asset(&server.url, Some(ETAG)));
    runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    server.finish();

    let plan_alias = temp.0.join("plan-hardlink-alias");
    fs::hard_link(workspace.plan_path(), &plan_alias).unwrap();
    assert_eq!(
        workspace.bind_plan(&plan),
        Err(Hdf5RangeTransferError::RecoveryInvalid)
    );
    fs::remove_file(plan_alias).unwrap();

    let manifest_alias = temp.0.join("manifest-hardlink-alias");
    fs::hard_link(workspace.manifest_path(), &manifest_alias).unwrap();
    assert_eq!(
        workspace.load_manifest(),
        Err(Hdf5RangeTransferError::RecoveryInvalid)
    );
    fs::remove_file(manifest_alias).unwrap();

    fs::OpenOptions::new()
        .write(true)
        .open(workspace.metrics_path())
        .unwrap()
        .set_len(64 * 1024 + 1)
        .unwrap();
    assert_eq!(
        workspace.load_metrics(&plan),
        Err(Hdf5RangeTransferError::RecoveryInvalid)
    );
}

#[test]
fn large_logical_range_uses_bounded_exact_network_slices() {
    const SLICE_BYTES: u64 = super::HDF5_RANGE_NETWORK_SLICE_BYTES;
    const SOURCE_BYTES: u64 = 10_000_000;
    const SCIENCE_END: u64 = 2 * SLICE_BYTES + 1;
    const MASK_START: u64 = 9_000_000;
    let server = LocalServer::new(vec![
        partial_response_with_total(
            0,
            SLICE_BYTES,
            SOURCE_BYTES,
            &vec![b'A'; SLICE_BYTES as usize],
            ETAG,
        ),
        partial_response_with_total(
            SLICE_BYTES,
            2 * SLICE_BYTES,
            SOURCE_BYTES,
            &vec![b'B'; SLICE_BYTES as usize],
            ETAG,
        ),
        partial_response_with_total(2 * SLICE_BYTES, SCIENCE_END, SOURCE_BYTES, b"C", ETAG),
        partial_response_with_total(MASK_START, MASK_START + 4, SOURCE_BYTES, b"MASK", ETAG),
    ]);
    let mut request = request();
    request.source.expected_size_bytes = SOURCE_BYTES;
    request.expected_output.full_product_bytes = SOURCE_BYTES;
    request.expected_output.estimated_remote_bytes = SCIENCE_END + 4;
    request.expected_output.estimated_output_bytes = SCIENCE_END + 4;
    request.validate().unwrap();
    let science = physical(0, 0, SCIENCE_END);
    let mask = mask_physical(0, MASK_START, MASK_START + 4);
    let plan =
        Hdf5RangePlanV1::from_offline_evidence(&request, evidence(&request, vec![science, mask]))
            .unwrap();
    let temp = TempDirectory::new("bounded-network-slices");
    let (runtime, workspace) = create_runtime(
        &temp,
        &request,
        asset_with_size(&server.url, Some(ETAG), SOURCE_BYTES),
    );
    let result = runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    assert_eq!(result.metrics().request_count, 4);
    assert_eq!(result.metrics().accepted_unique_bytes, SCIENCE_END + 4);
    let requests = server
        .finish()
        .into_iter()
        .map(|request| request.to_ascii_lowercase())
        .collect::<Vec<_>>();
    assert!(requests[0].contains(&format!("range: bytes=0-{}", SLICE_BYTES - 1)));
    assert!(requests[1].contains(&format!(
        "range: bytes={}-{}",
        SLICE_BYTES,
        2 * SLICE_BYTES - 1
    )));
    assert!(requests[2].contains(&format!(
        "range: bytes={}-{}",
        2 * SLICE_BYTES,
        2 * SLICE_BYTES
    )));
    assert!(requests[3].contains(&format!("range: bytes={}-{}", MASK_START, MASK_START + 3)));
    assert_eq!(
        fs::metadata(workspace.part_path()).unwrap().len(),
        SOURCE_BYTES
    );
}

#[test]
fn ignored_range_and_416_fail_without_full_download_fallback() {
    for (label, response_bytes, expected) in [
        (
            "ignored",
            response(
                "200 OK",
                &[("ETag", ETAG), ("Content-Length", "64")],
                &[7_u8; 64],
            ),
            Hdf5RangeTransferError::RangeUnsupported,
        ),
        (
            "unsatisfiable",
            response(
                "416 Range Not Satisfiable",
                &[("ETag", ETAG), ("Content-Length", "0")],
                b"",
            ),
            Hdf5RangeTransferError::RangeNotSatisfiable,
        ),
    ] {
        let server = LocalServer::new(vec![response_bytes]);
        let request = small_request(4);
        let plan = plan(&request, &[(4, 8)]);
        let temp = TempDirectory::new(label);
        let (runtime, workspace) = create_runtime(&temp, &request, asset(&server.url, Some(ETAG)));
        assert_eq!(
            runtime.execute(&request, &plan, TIMESTAMP, &|| false),
            Err(expected)
        );
        let manifest = workspace.load_manifest().unwrap().unwrap();
        assert_eq!(manifest.state, NisarSubsetTransferState::Failed);
        assert_eq!(manifest.ranges[0].state, NisarSubsetRangeState::Failed);
        assert_ne!(fs::read(workspace.part_path()).unwrap(), vec![7_u8; 64]);
        assert_eq!(
            server.finish().len(),
            1,
            "must not issue a full-file fallback"
        );
    }
}

#[test]
fn transient_status_reuses_bounded_http_retry_classification() {
    let server = LocalServer::new(vec![
        response("503 Service Unavailable", &[("Content-Length", "0")], b""),
        partial_response(0, 4, b"ABCD", ETAG),
        partial_response(56, 60, b"MASK", ETAG),
    ]);
    let request = small_request(4);
    let retry_plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("transient-retry");
    let workspace = Hdf5RangeWorkspaceV1::new(temp.0.clone()).unwrap();
    let runtime = OfflineHdf5RangeTransfer::new(
        transfer_with_attempts(2),
        &request,
        &snapshot(&request),
        asset(&server.url, Some(ETAG)),
        workspace,
        DestinationLeaseController::new_for_qualification_test(),
    )
    .unwrap();
    let result = runtime
        .execute(&request, &retry_plan, TIMESTAMP, &|| false)
        .unwrap();
    assert_eq!(result.metrics().request_count, 3);
    assert_eq!(result.metrics().response_body_bytes, 8);
    let captured = server.finish();
    assert_eq!(captured.len(), 3);
    assert!(captured[..2]
        .iter()
        .all(|request| request.to_ascii_lowercase().contains("range: bytes=0-3")));
    assert!(captured[2]
        .to_ascii_lowercase()
        .contains("range: bytes=56-59"));
}

#[test]
fn retry_exhaustion_is_restartable_and_metrics_survive_reopen() {
    let first = LocalServer::new(vec![
        response("503 Service Unavailable", &[("Content-Length", "0")], b""),
        response("503 Service Unavailable", &[("Content-Length", "0")], b""),
    ]);
    let request = small_request(4);
    let plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("retry-exhausted");
    let workspace = Hdf5RangeWorkspaceV1::new(temp.0.clone()).unwrap();
    let runtime = OfflineHdf5RangeTransfer::new(
        transfer_with_attempts(2),
        &request,
        &snapshot(&request),
        asset(&first.url, Some(ETAG)),
        workspace.clone(),
        DestinationLeaseController::new_for_qualification_test(),
    )
    .unwrap();
    assert_eq!(
        runtime.execute(&request, &plan, TIMESTAMP, &|| false),
        Err(Hdf5RangeTransferError::Transport)
    );
    assert_eq!(
        workspace.load_manifest().unwrap().unwrap().state,
        NisarSubsetTransferState::Interrupted
    );
    assert_eq!(first.finish().len(), 2);

    let second = LocalServer::new(vec![
        partial_response(0, 4, b"ABCD", ETAG),
        partial_response(56, 60, b"MASK", ETAG),
    ]);
    let (runtime, _) = create_runtime(&temp, &request, asset(&second.url, Some(ETAG)));
    let result = runtime
        .execute(&request, &plan, "2026-08-01T00:00:01Z", &|| false)
        .unwrap();
    assert_eq!(result.metrics().request_count, 4);
    assert_eq!(result.metrics().response_body_bytes, 8);
    assert_eq!(result.metrics().accepted_unique_bytes, 8);
    assert_eq!(second.finish().len(), 2);
}

#[test]
fn runtime_requires_frozen_identity_approved_loopback_and_workspace_owner() {
    let request = small_request(4);
    let frozen = snapshot(&request);
    let temp = TempDirectory::new("runtime-boundaries");
    let workspace = Hdf5RangeWorkspaceV1::new(temp.0.clone()).unwrap();
    let lease = || DestinationLeaseController::new_for_qualification_test();

    assert!(OfflineHdf5RangeTransfer::new(
        HttpTransfer::new(TransferOptions::default()).unwrap(),
        &request,
        &frozen,
        asset("http://127.0.0.1:9/fixture.h5", Some(ETAG)),
        workspace.clone(),
        lease(),
    )
    .is_ok());
    for url in [
        "http://localhost:9/fixture.h5",
        "http://127.0.0.1:9/fixture.h5?temporary=secret",
        "http://127.0.0.1:9/fixture.h5#fragment",
    ] {
        assert!(matches!(
            OfflineHdf5RangeTransfer::new(
                transfer(),
                &request,
                &frozen,
                asset(url, Some(ETAG)),
                workspace.clone(),
                lease(),
            ),
            Err(Hdf5RangeTransferError::RangeUnsupported)
        ));
    }

    let mut mismatched = frozen.clone();
    mismatched.provider_id = "not-the-frozen-provider".to_owned();
    assert!(matches!(
        OfflineHdf5RangeTransfer::new(
            transfer(),
            &request,
            &mismatched,
            asset("http://127.0.0.1:9/fixture.h5", Some(ETAG)),
            workspace.clone(),
            lease(),
        ),
        Err(Hdf5RangeTransferError::WindowIdentityMismatch)
    ));

    let lock_guard = hold_workspace_lock(&temp.0.join("range-workspace-v1.lock"));
    let runtime = OfflineHdf5RangeTransfer::new(
        transfer(),
        &request,
        &frozen,
        asset("http://127.0.0.1:9/fixture.h5", Some(ETAG)),
        workspace,
        lease(),
    )
    .unwrap();
    let plan = plan(&request, &[(0, 4)]);
    assert_eq!(
        runtime.execute(&request, &plan, TIMESTAMP, &|| false),
        Err(Hdf5RangeTransferError::WorkspaceBusy)
    );
    drop(lock_guard);
}

#[test]
fn workspace_lock_rejects_hardlink_alias_and_reuses_unlocked_crash_residue() {
    let request = small_request(4);
    let plan = plan(&request, &[(0, 4)]);

    let aliased = TempDirectory::new("workspace-lock-hardlink");
    let unrelated = aliased.0.join("unrelated.bin");
    fs::write(&unrelated, b"must-not-be-truncated").unwrap();
    fs::hard_link(&unrelated, aliased.0.join("range-workspace-v1.lock")).unwrap();
    let (runtime, _) = create_runtime(
        &aliased,
        &request,
        asset("http://127.0.0.1:9/fixture.h5", Some(ETAG)),
    );
    assert_eq!(
        runtime.execute(&request, &plan, TIMESTAMP, &|| false),
        Err(Hdf5RangeTransferError::RecoveryInvalid)
    );
    assert_eq!(fs::read(&unrelated).unwrap(), b"must-not-be-truncated");

    let residue = TempDirectory::new("workspace-lock-crash-residue");
    fs::write(
        residue.0.join("range-workspace-v1.lock"),
        b"incomplete-old-owner",
    )
    .unwrap();
    let server = LocalServer::new(vec![
        partial_response(0, 4, b"ABCD", ETAG),
        partial_response(56, 60, b"MASK", ETAG),
    ]);
    let (runtime, _) = create_runtime(&residue, &request, asset(&server.url, Some(ETAG)));
    runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    assert_eq!(server.finish().len(), 2);
    assert!(residue.0.join("range-workspace-v1.lock").is_file());

    let (reopened, _) = create_runtime(
        &residue,
        &request,
        asset("http://127.0.0.1:9/fixture.h5", Some(ETAG)),
    );
    assert_eq!(
        reopened
            .execute(&request, &plan, "2026-08-01T00:00:01Z", &|| false)
            .unwrap()
            .manifest()
            .state,
        NisarSubsetTransferState::RangesComplete
    );
}

#[test]
fn persisted_plan_digest_rejects_layout_takeover_on_restart() {
    let server = LocalServer::new(vec![
        partial_response(0, 4, b"ABCD", ETAG),
        partial_response(56, 60, b"MASK", ETAG),
    ]);
    let request = small_request(4);
    let plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("plan-binding");
    let (runtime, workspace) = create_runtime(&temp, &request, asset(&server.url, Some(ETAG)));
    let result = runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    assert_eq!(result.plan_sha256(), plan.plan_sha256());
    assert_eq!(server.finish().len(), 2);

    let mask = mask_physical(0, 56, 60);
    let mut changed = evidence(&request, vec![physical(0, 0, 4), mask]);
    changed.layout_evidence_digest =
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned();
    let changed_plan = Hdf5RangePlanV1::from_offline_evidence(&request, changed).unwrap();
    assert_ne!(changed_plan.plan_sha256(), plan.plan_sha256());
    let (runtime, _) = create_runtime(
        &temp,
        &request,
        asset("http://127.0.0.1:9/fixture.h5", Some(ETAG)),
    );
    assert_eq!(
        runtime.execute(&request, &changed_plan, "2026-08-01T00:00:01Z", &|| false),
        Err(Hdf5RangeTransferError::RecoveryInvalid)
    );

    let plan_sidecar = fs::read_to_string(workspace.plan_path()).unwrap();
    assert!(plan_sidecar.contains(plan.plan_sha256()));
    assert!(plan_sidecar.contains("logical_chunk"));
    assert!(plan_sidecar.contains("member_chunk_indices"));
}

#[test]
fn strong_validator_is_required_and_response_drift_is_terminal() {
    let request = small_request(4);
    let frozen = snapshot(&request);
    let temp = TempDirectory::new("validator-construction");
    let workspace = Hdf5RangeWorkspaceV1::new(temp.0.clone()).unwrap();
    for rejected in [None, Some("W/\"weak\"")] {
        assert!(matches!(
            OfflineHdf5RangeTransfer::new(
                transfer(),
                &request,
                &frozen,
                asset("http://127.0.0.1:9/fixture.h5", rejected),
                workspace.clone(),
                DestinationLeaseController::new_for_qualification_test(),
            ),
            Err(Hdf5RangeTransferError::StrongValidatorRequired)
        ));
    }
    assert!(matches!(
        OfflineHdf5RangeTransfer::new(
            transfer(),
            &request,
            &frozen,
            ResolvedAsset::new("http://127.0.0.1:9/fixture.h5", ResolvedMethod::Head)
                .unwrap()
                .with_remote_validator(RemoteValidator {
                    etag: Some(ETAG.to_owned()),
                    last_modified: None,
                    content_length: Some(64),
                }),
            workspace.clone(),
            DestinationLeaseController::new_for_qualification_test(),
        ),
        Err(Hdf5RangeTransferError::RangeUnsupported)
    ));

    for (label, response_etag, expected, state) in [
        (
            "weak-response",
            "W/\"weak\"",
            Hdf5RangeTransferError::WeakEtagRejected,
            NisarSubsetTransferState::Failed,
        ),
        (
            "changed-response",
            "\"nisar-range-v2\"",
            Hdf5RangeTransferError::SourceChanged,
            NisarSubsetTransferState::SourceChanged,
        ),
    ] {
        let server = LocalServer::new(vec![partial_response(0, 4, b"ABCD", response_etag)]);
        let request = small_request(4);
        let plan = plan(&request, &[(0, 4)]);
        let temp = TempDirectory::new(label);
        let (runtime, workspace) = create_runtime(&temp, &request, asset(&server.url, Some(ETAG)));
        assert_eq!(
            runtime.execute(&request, &plan, TIMESTAMP, &|| false),
            Err(expected)
        );
        assert_eq!(workspace.load_manifest().unwrap().unwrap().state, state);
        assert_eq!(server.finish().len(), 1);
    }

    let changed_416 = LocalServer::new(vec![response(
        "416 Range Not Satisfiable",
        &[("ETag", "\"nisar-range-v2\""), ("Content-Length", "0")],
        b"",
    )]);
    let request = small_request(4);
    let changed_416_plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("changed-416");
    let (runtime, workspace) = create_runtime(&temp, &request, asset(&changed_416.url, Some(ETAG)));
    assert_eq!(
        runtime.execute(&request, &changed_416_plan, TIMESTAMP, &|| false),
        Err(Hdf5RangeTransferError::SourceChanged)
    );
    assert_eq!(
        workspace.load_manifest().unwrap().unwrap().state,
        NisarSubsetTransferState::SourceChanged
    );
    assert_eq!(changed_416.finish().len(), 1);

    let malformed = LocalServer::new(vec![response(
        "206 Partial Content",
        &[
            ("ETag", ETAG),
            ("Content-Range", "bytes 0-4/64"),
            ("Content-Length", "4"),
        ],
        b"ABCD",
    )]);
    let malformed_plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("malformed-range");
    let (runtime, workspace) = create_runtime(&temp, &request, asset(&malformed.url, Some(ETAG)));
    assert_eq!(
        runtime.execute(&request, &malformed_plan, TIMESTAMP, &|| false),
        Err(Hdf5RangeTransferError::MalformedContentRange)
    );
    assert_eq!(
        workspace.load_manifest().unwrap().unwrap().state,
        NisarSubsetTransferState::Failed
    );
    assert_eq!(malformed.finish().len(), 1);

    let temp = TempDirectory::new("source-length-drift");
    let workspace = Hdf5RangeWorkspaceV1::new(temp.0.clone()).unwrap();
    let drifted = ResolvedAsset::new("http://127.0.0.1:9/fixture.h5", ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(63))
        .with_remote_validator(RemoteValidator {
            etag: Some(ETAG.to_owned()),
            last_modified: None,
            content_length: Some(63),
        });
    assert!(matches!(
        OfflineHdf5RangeTransfer::new(
            transfer(),
            &request,
            &frozen,
            drifted,
            workspace,
            DestinationLeaseController::new_for_qualification_test(),
        ),
        Err(Hdf5RangeTransferError::SourceChanged)
    ));
}

#[test]
fn encoded_or_ambiguous_206_headers_fail_closed() {
    let cases = [
        (
            response(
                "206 Partial Content",
                &[
                    ("ETag", ETAG),
                    ("Content-Range", "bytes 0-3/64"),
                    ("Content-Length", "4"),
                    ("Content-Encoding", "gzip"),
                ],
                b"ABCD",
            ),
            Hdf5RangeTransferError::MalformedContentRange,
        ),
        (
            response(
                "206 Partial Content",
                &[
                    ("ETag", ETAG),
                    ("ETag", ETAG),
                    ("Content-Range", "bytes 0-3/64"),
                    ("Content-Length", "4"),
                ],
                b"ABCD",
            ),
            Hdf5RangeTransferError::ResponseInvalid,
        ),
        (
            response(
                "206 Partial Content",
                &[
                    ("ETag", ETAG),
                    ("Content-Range", "bytes 0-3/64"),
                    ("Content-Range", "bytes 0-3/64"),
                    ("Content-Length", "4"),
                ],
                b"ABCD",
            ),
            Hdf5RangeTransferError::ResponseInvalid,
        ),
    ];
    for (index, (response_bytes, expected)) in cases.into_iter().enumerate() {
        let server = LocalServer::new(vec![response_bytes]);
        let request = small_request(4);
        let plan = plan(&request, &[(0, 4)]);
        let temp = TempDirectory::new(&format!("ambiguous-header-{index}"));
        let (runtime, workspace) = create_runtime(&temp, &request, asset(&server.url, Some(ETAG)));
        assert_eq!(
            runtime.execute(&request, &plan, TIMESTAMP, &|| false),
            Err(expected)
        );
        assert_eq!(
            workspace.load_manifest().unwrap().unwrap().state,
            NisarSubsetTransferState::Failed
        );
        assert_eq!(server.finish().len(), 1);
    }
}

#[test]
fn partial_response_restarts_with_if_range_and_preserves_completed_digest() {
    let request = small_request(4);
    let plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("restart");
    let first = LocalServer::new(vec![
        partial_response(0, 4, b"ABCD", ETAG),
        partial_response(56, 60, b"MA", ETAG),
    ]);
    let (runtime, workspace) = create_runtime(&temp, &request, asset(&first.url, Some(ETAG)));
    assert_eq!(
        runtime.execute(&request, &plan, TIMESTAMP, &|| false),
        Err(Hdf5RangeTransferError::PartialResponse)
    );
    let interrupted = workspace.load_manifest().unwrap().unwrap();
    assert_eq!(interrupted.state, NisarSubsetTransferState::Interrupted);
    assert_eq!(interrupted.ranges[0].state, NisarSubsetRangeState::Complete);
    assert!(interrupted.ranges[0].sha256.is_some());
    assert_eq!(interrupted.ranges[1].state, NisarSubsetRangeState::Planned);
    assert_eq!(first.finish().len(), 2);

    let second = LocalServer::new(vec![partial_response(56, 60, b"MASK", ETAG)]);
    let (runtime, _) = create_runtime(&temp, &request, asset(&second.url, Some(ETAG)));
    let completed = runtime
        .execute(&request, &plan, "2026-08-01T00:00:01Z", &|| false)
        .unwrap();
    assert_eq!(
        completed.manifest().state,
        NisarSubsetTransferState::RangesComplete
    );
    assert_eq!(completed.manifest().received_bytes, 8);
    assert_eq!(completed.metrics().request_count, 3);
    assert_eq!(completed.metrics().accepted_unique_bytes, 8);
    assert_eq!(completed.metrics().retry_duplicate_bytes, 2);
    let retried = second.finish();
    let retried_request = retried[0].to_ascii_lowercase();
    assert!(retried_request.contains("range: bytes=56-59"));
    assert!(retried_request.contains("if-range: \"nisar-range-v1\""));
}

#[test]
fn cancellation_persists_interrupted_without_false_completion() {
    let server = LocalServer::new(vec![partial_response(0, 4, b"ABCD", ETAG)]);
    let request = small_request(4);
    let plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("cancel");
    let observed_requests = Arc::clone(&server.requests);
    let (runtime, workspace) = create_runtime(&temp, &request, asset(&server.url, Some(ETAG)));
    let cancelled = move || !observed_requests.lock().unwrap().is_empty();
    assert_eq!(
        runtime.execute(&request, &plan, TIMESTAMP, &cancelled),
        Err(Hdf5RangeTransferError::Cancelled)
    );
    let manifest = workspace.load_manifest().unwrap().unwrap();
    assert_eq!(manifest.state, NisarSubsetTransferState::Interrupted);
    assert_ne!(manifest.state, NisarSubsetTransferState::RangesComplete);
    drop(cancelled);
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn slow_progress_uses_idle_timeout_instead_of_a_total_range_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        thread::sleep(Duration::from_millis(350));
        stream
            .write_all(
                b"HTTP/1.1 206 Partial Content\r\nConnection: close\r\nETag: \"nisar-range-v1\"\r\nContent-Range: bytes 0-3/64\r\nContent-Length: 4\r\n\r\n",
            )
            .unwrap();
        stream.flush().unwrap();
        for byte in b"ABCD" {
            thread::sleep(Duration::from_millis(550));
            stream.write_all(&[*byte]).unwrap();
            stream.flush().unwrap();
        }
        let (mut mask_stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut mask_stream);
        mask_stream
            .write_all(&partial_response(56, 60, b"MASK", ETAG))
            .unwrap();
        mask_stream.flush().unwrap();
    });
    let request = small_request(4);
    let plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("range-idle-timeout");
    let (runtime, _) = create_runtime(
        &temp,
        &request,
        asset(&format!("http://{address}/fixture.h5"), Some(ETAG)),
    );
    let result = runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    worker.join().unwrap();
    assert_eq!(
        result.manifest().state,
        NisarSubsetTransferState::RangesComplete
    );
}

#[test]
fn cancellation_is_observed_between_bounded_response_reads() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let server_cancelled = Arc::clone(&cancelled);
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        stream
            .write_all(
                b"HTTP/1.1 206 Partial Content\r\nConnection: close\r\nETag: \"nisar-range-v1\"\r\nContent-Range: bytes 0-3/64\r\nContent-Length: 4\r\n\r\nAB",
            )
            .unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(20));
        server_cancelled.store(true, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(20));
        let _ = stream.write_all(b"CD");
    });
    let request = small_request(4);
    let plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("stream-cancel");
    let (runtime, workspace) = create_runtime(
        &temp,
        &request,
        asset(&format!("http://{address}/fixture.h5"), Some(ETAG)),
    );
    assert_eq!(
        runtime.execute(&request, &plan, TIMESTAMP, &|| cancelled
            .load(Ordering::SeqCst)),
        Err(Hdf5RangeTransferError::Cancelled)
    );
    worker.join().unwrap();
    assert_eq!(
        workspace.load_manifest().unwrap().unwrap().state,
        NisarSubsetTransferState::Interrupted
    );
}

#[test]
fn restart_rejects_tampered_completed_range_before_network_access() {
    let server = LocalServer::new(vec![
        partial_response(0, 4, b"ABCD", ETAG),
        partial_response(56, 60, b"MASK", ETAG),
    ]);
    let request = small_request(4);
    let plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("tamper");
    let (runtime, workspace) = create_runtime(&temp, &request, asset(&server.url, Some(ETAG)));
    runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    assert_eq!(server.finish().len(), 2);

    let mut part = fs::OpenOptions::new()
        .write(true)
        .open(workspace.part_path())
        .unwrap();
    part.write_all(b"WXYZ").unwrap();
    part.sync_all().unwrap();
    drop(part);

    let (runtime, _) = create_runtime(
        &temp,
        &request,
        asset("http://127.0.0.1:9/fixture.h5", Some(ETAG)),
    );
    assert_eq!(
        runtime.execute(&request, &plan, "2026-08-01T00:00:01Z", &|| false),
        Err(Hdf5RangeTransferError::RecoveryInvalid)
    );
}

#[test]
fn part_file_rejects_hardlink_aliases_and_replacement_identity() {
    let request = small_request(4);
    let plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("part-hardlink");
    let target = temp.0.join("unrelated.bin");
    fs::write(&target, [9_u8; 64]).unwrap();
    fs::hard_link(&target, temp.0.join("ranges.part")).unwrap();
    let (runtime, _) = create_runtime(
        &temp,
        &request,
        asset("http://127.0.0.1:9/fixture.h5", Some(ETAG)),
    );
    assert_eq!(
        runtime.execute(&request, &plan, TIMESTAMP, &|| false),
        Err(Hdf5RangeTransferError::RecoveryInvalid)
    );
    assert_eq!(fs::read(&target).unwrap(), vec![9_u8; 64]);

    let server = LocalServer::new(vec![
        partial_response(0, 4, b"ABCD", ETAG),
        partial_response(56, 60, b"MASK", ETAG),
    ]);
    let temp = TempDirectory::new("part-replacement");
    let (runtime, workspace) = create_runtime(&temp, &request, asset(&server.url, Some(ETAG)));
    runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();
    assert_eq!(server.finish().len(), 2);
    fs::remove_file(workspace.part_path()).unwrap();
    fs::write(workspace.part_path(), [0_u8; 64]).unwrap();
    let (runtime, _) = create_runtime(
        &temp,
        &request,
        asset("http://127.0.0.1:9/fixture.h5", Some(ETAG)),
    );
    assert_eq!(
        runtime.execute(&request, &plan, "2026-08-01T00:00:01Z", &|| false),
        Err(Hdf5RangeTransferError::RecoveryInvalid)
    );
}

#[test]
fn cross_origin_loopback_redirect_drops_runtime_access_and_never_persists_it() {
    let target = LocalServer::new(vec![
        partial_response(0, 4, b"ABCD", ETAG),
        partial_response(56, 60, b"MASK", ETAG),
    ]);
    let redirect = response(
        "302 Found",
        &[("Location", &target.url), ("Content-Length", "0")],
        b"",
    );
    let source = LocalServer::new(vec![redirect.clone(), redirect]);
    let request = small_request(4);
    let plan = plan(&request, &[(0, 4)]);
    let temp = TempDirectory::new("redirect");
    let mut resolved = asset(&source.url, Some(ETAG));
    CredentialMaterial::bearer("REDIRECT_ONLY_TOKEN")
        .unwrap()
        .apply_to(&mut resolved)
        .unwrap();
    resolved
        .insert_cookie("offline_session", "REDIRECT_ONLY_COOKIE")
        .unwrap();
    resolved
        .insert_header("X-Offline-Access", "REDIRECT_ONLY_HEADER")
        .unwrap();
    let source_url = source.url.clone();
    let target_url = target.url.clone();
    let (runtime, workspace) = create_runtime(&temp, &request, resolved);
    runtime
        .execute(&request, &plan, TIMESTAMP, &|| false)
        .unwrap();

    let source_requests = source.finish();
    assert_eq!(source_requests.len(), 2);
    let source_request = source_requests[0].to_ascii_lowercase();
    assert!(source_request.contains("authorization: bearer redirect_only_token"));
    assert!(source_request.contains("cookie: offline_session=redirect_only_cookie"));
    assert!(source_request.contains("x-offline-access: redirect_only_header"));
    let target_requests = target.finish();
    assert_eq!(target_requests.len(), 2);
    for target_request in &target_requests {
        let target_request = target_request.to_ascii_lowercase();
        assert!(!target_request.contains("authorization:"));
        assert!(!target_request.contains("cookie:"));
        assert!(!target_request.contains("x-offline-access:"));
        assert!(target_request.contains("if-range: \"nisar-range-v1\""));
    }
    assert!(target_requests[0]
        .to_ascii_lowercase()
        .contains("range: bytes=0-3"));
    assert!(target_requests[1]
        .to_ascii_lowercase()
        .contains("range: bytes=56-59"));

    let sidecar = [
        fs::read_to_string(workspace.manifest_path()).unwrap(),
        fs::read_to_string(workspace.plan_path()).unwrap(),
        fs::read_to_string(workspace.metrics_path()).unwrap(),
        fs::read_to_string(workspace.part_identity_path()).unwrap(),
    ]
    .join("\n");
    for forbidden in [
        "REDIRECT_ONLY_TOKEN",
        "REDIRECT_ONLY_COOKIE",
        "REDIRECT_ONLY_HEADER",
        source_url.as_str(),
        target_url.as_str(),
    ] {
        assert!(!sidecar.contains(forbidden));
    }
}
