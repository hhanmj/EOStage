use std::collections::BTreeMap;
#[path = "support/cdse_spatial_freeze.rs"]
mod cdse_spatial_freeze;
#[path = "support/landsat_asset_batch.rs"]
mod landsat_asset_batch;
#[path = "support/landsat_bundle_batch.rs"]
mod landsat_bundle_batch;
#[path = "support/landsat_official_match.rs"]
mod landsat_official_match;
#[path = "support/landsat_source_identity.rs"]
mod landsat_source_identity;
#[path = "support/optical_preparation.rs"]
mod optical_preparation;
#[path = "support/sentinel2_band_batch.rs"]
mod sentinel2_band_batch;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationCatalogSearchAndFreezeRequest,
    ApplicationSarBackgroundCountRequest, ApplicationSarSearchAndFreezeRequest, ApplicationService,
    ApplicationServiceConfig, ApplicationSetWorkingAoiRequest, CdseNetworkMode,
    LandsatAssetDetailsRequest, LandsatBrowseImage, LandsatBrowseRequest,
    LandsatCreateAssetTaskRequest, LandsatCreateBundleTaskRequest, LiveLandsatAssetManifest,
    LiveLandsatBrowse, LiveLandsatBundleOptions, LiveSentinel2BandEvidence, SarSearchProgressStage,
    Sentinel2CreateArchiveBatchTaskRequest, Sentinel2CreateArchiveTaskRequest,
    Sentinel2CreateBandBatchTaskRequest, Sentinel2CreateBandTaskRequest,
};
use insar_core::{
    provider_product_key, stable_asset_key, AsfRemoteSearchRequest, AsfRemoteSearchResponse,
    AssetRole, BBox, CatalogSearchQuery, CatalogSearchResponse, ContractError,
    DownloadItemProgress, DownloadItemResult, DownloadLayout, DownloadOutcome, ProviderProduct,
    ProviderResult, SarRemoteSearchRequest, SarRemoteSearchResponse, SarSearchCountRequest,
    SarSearchCountResponse, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreTaskSourceBatchPageRequest,
    StoreUpdateTaskRequest, TaskStateCommandKind,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::asf::search::{AsfProvider, MetadataTransport, SarSearchProviderProgress};
use insar_provider::catalog::{MpcSentinel2BandAsset, MpcSentinel2BandManifest, RouteUse};
use insar_provider::cdse::loopback_client::LoopbackODataClient;
use insar_provider::cdse::odata_mapper::map_odata_page;
use insar_provider::cdse::odata_types::ODataPage;
use insar_provider::cdse::query::build_odata_request_for_use;
use insar_provider::planetary_computer::{
    PlanetaryComputerError, PlanetaryComputerLandsatAsset, PlanetaryComputerLandsatManifest,
};
use insar_provider::planetary_computer_asset::PlanetaryComputerReviewedAssetEvidence;
use insar_provider::usgs_m2m::{
    UsgsM2mBundleAvailability, UsgsM2mFullBundleOption, LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM,
    LANDSAT_C2_L2_BUNDLE_OPTION_ID, LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE,
    LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::{StoreWorkbenchPageRequest, TaskStore};
use serde_json::{json, Map, Value};

mod common;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "insar-catalogue-freeze-{label}-{}-{id}.sqlite3",
            std::process::id()
        )))
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

struct LoopbackServer {
    endpoint: String,
    worker: Option<thread::JoinHandle<()>>,
}

impl LoopbackServer {
    fn two_pages(first: String, second: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = format!("http://{address}/odata/v1/");
        let first = first.replace("http://127.0.0.1:38081/odata/v1/", &endpoint);
        let worker = thread::spawn(move || {
            for body in [first, second] {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = read_request(&mut stream);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        Self {
            endpoint,
            worker: Some(worker),
        }
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
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

#[derive(Default)]
struct FixtureAsf;

impl LiveAsfSearch for FixtureAsf {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
    }
}

struct LegacyAsfFixture {
    source: String,
    calls: Option<Arc<AtomicU64>>,
}

struct CancelledAsfFixture;

struct DeterministicSentinelMatrixTransport {
    metadata_calls: Arc<Mutex<Vec<BTreeMap<String, String>>>>,
    count_calls: Arc<AtomicU64>,
}

struct DeterministicSentinelMatrixFixture {
    provider: AsfProvider<DeterministicSentinelMatrixTransport>,
}

fn deterministic_sentinel_feature(index: usize) -> Value {
    json!({
        "type": "Feature",
        "properties": {
            "sceneName": format!(
                "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_{:06}_064ABC_{:04X}",
                52_000 + index,
                index,
            ),
            "pathNumber": "88",
            "frameNumber": "456",
            "flightDirection": "ASCENDING",
            "processingLevel": "SLC",
            "platform": "Sentinel-1A"
        },
        "geometry": {
            "type": "Polygon",
            "coordinates": [[[100.0, 40.0], [100.1, 40.0], [100.1, 40.1], [100.0, 40.1], [100.0, 40.0]]]
        }
    })
}

impl MetadataTransport for DeterministicSentinelMatrixTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.metadata_calls.lock().unwrap().push(params.clone());
        let limit = params["maxResults"]
            .parse::<usize>()
            .expect("deterministic transport limit is numeric");
        Ok(json!({
            "type": "FeatureCollection",
            "features": (0..limit)
                .map(deterministic_sentinel_feature)
                .collect::<Vec<_>>()
        }))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        self.count_calls.fetch_add(1, Ordering::Relaxed);
        Ok(Some(50_000))
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("deterministic Sentinel matrix must not route to CMR")
    }
}

impl LiveAsfSearch for DeterministicSentinelMatrixFixture {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
    }

    fn search_sar(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.provider.search_sar(request, cancelled)
    }

    fn search_sar_with_progress(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(SarSearchProviderProgress),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.provider
            .search_sar_with_progress(request, cancelled, progress)
    }
}

struct BackgroundCountAsfFixture {
    search_calls: Arc<AtomicU64>,
    count_calls: Arc<AtomicU64>,
    total_count: u64,
}

impl LiveAsfSearch for BackgroundCountAsfFixture {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
    }

    fn search_sar(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.search_calls.fetch_add(1, Ordering::Relaxed);
        let mut response = LegacyAsfFixture {
            source: "ASF".to_owned(),
            calls: None,
        }
        .search_sar(request, cancelled)?;
        response.stats.total_count = None;
        response.stats.matched_count = None;
        response.stats.truncated = Some(false);
        Ok(response)
    }

    fn count_sar(
        &self,
        _request: &SarSearchCountRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarSearchCountResponse> {
        self.count_calls.fetch_add(1, Ordering::Relaxed);
        Ok(SarSearchCountResponse {
            total_count: Some(self.total_count),
        })
    }
}

impl LiveAsfSearch for CancelledAsfFixture {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("DL001", "fixture cancelled", Map::new()).unwrap())
    }

    fn search_sar(
        &self,
        _request: &SarRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        Err(ContractError::new("DL001", "fixture cancelled", Map::new()).unwrap())
    }
}

fn legacy_asf_fixture_ids() -> Vec<String> {
    ["Z", "A", "M"]
        .into_iter()
        .map(str::to_owned)
        .chain((3..20).map(|index| format!("S1-FIXTURE-{index:02}")))
        .collect()
}

impl LiveAsfSearch for LegacyAsfFixture {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
    }

    fn search_sar(
        &self,
        _request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        if let Some(calls) = &self.calls {
            calls.fetch_add(1, Ordering::Relaxed);
        }
        if cancelled() {
            return Err(ContractError::new(
                "CATALOGUE_REQUEST_CANCELLED",
                "fixture cancelled",
                Map::new(),
            )
            .unwrap());
        }
        Ok(serde_json::from_value(json!({
            "products": legacy_asf_fixture_ids().into_iter().map(|id| json!({
                "product_key": format!("PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::{id}"),
                "provider_product_key": format!("PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::{id}"),
                "provider_product_id": id.clone(),
                "provider": "asf.catalog",
                "dataset": "SENTINEL-1",
                "product_family": "SLC",
                "processing_level": "SLC",
                "platform": "S1A",
                "acquisition_start": "2026-07-01T00:00:00Z",
                "file_name": format!("{id}.zip"),
                "file_size_remote": 4,
                "download_url": format!("http://127.0.0.1:18080/{id}.zip"),
                "product_id": id
            })).collect::<Vec<_>>(),
            "errors": [],
            "stats": {
                "requested_limit": 20,
                "query_limit": 20,
                "total_count": 51594,
                "candidate_count": 20,
                "returned_count": 20,
                "source": self.source.clone(),
                "matched_count": 51594,
                "fetched_record_count": 20,
                "mapped_record_count": 20,
                "truncated": true
            }
        }))
        .expect("legacy ASF fixture response"))
    }

    fn search_sar_with_progress(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(SarSearchProviderProgress),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        progress(SarSearchProviderProgress {
            provider_request_count: 1,
            transport_part_count: 1,
            raw_fetched_count: 10,
            mapped_count: 10,
            unique_admitted_count: 10,
            remote_total: Some(51_594),
            requested_limit: request.query.max_results,
            fetched_count: 10,
            total_count: Some(51_594),
            ..SarSearchProviderProgress::default()
        });
        let response = self.search_sar(request, cancelled)?;
        progress(SarSearchProviderProgress {
            provider_request_count: 2,
            transport_part_count: 2,
            raw_fetched_count: 20,
            mapped_count: 20,
            unique_admitted_count: 20,
            remote_total: Some(51_594),
            requested_limit: request.query.max_results,
            fetched_count: 20,
            total_count: Some(51_594),
            ..SarSearchProviderProgress::default()
        });
        Ok(response)
    }
}

#[derive(Clone)]
struct IncrementalAsfFixture {
    requests: Arc<Mutex<Vec<SarRemoteSearchRequest>>>,
}

impl LiveAsfSearch for IncrementalAsfFixture {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
    }

    fn search_sar(
        &self,
        request: &SarRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        let call_index = {
            let mut requests = self.requests.lock().unwrap();
            let call_index = requests.len();
            requests.push(request.clone());
            call_index
        };
        let (rows, matched_count) = if call_index == 0 {
            (
                vec![("Z", "2024-02-03T00:00:00Z"), ("A", "2024-02-02T00:00:00Z")],
                4,
            )
        } else {
            (
                vec![("M", "2024-02-01T00:00:00Z"), ("B", "2024-01-31T00:00:00Z")],
                2,
            )
        };
        let returned_count = rows.len() as u64;
        Ok(serde_json::from_value(json!({
            "products": rows.into_iter().map(|(id, acquired)| json!({
                "product_key": format!("PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::{id}"),
                "provider_product_key": format!("PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::{id}"),
                "provider_product_id": id,
                "provider": "asf.catalog",
                "dataset": "SENTINEL-1",
                "product_family": "SLC",
                "processing_level": "SLC",
                "platform": "S1A",
                "acquisition_start": acquired,
                "file_name": format!("{id}.zip"),
                "file_size_remote": 4,
                "download_url": format!("http://127.0.0.1:18080/{id}.zip"),
                "product_id": id
            })).collect::<Vec<_>>(),
            "errors": [],
            "stats": {
                "requested_limit": returned_count,
                "query_limit": returned_count,
                "total_count": matched_count,
                "candidate_count": returned_count,
                "returned_count": returned_count,
                "source": "ASF_BATCH",
                "matched_count": matched_count,
                "fetched_record_count": returned_count,
                "mapped_record_count": returned_count,
                "truncated": matched_count > returned_count
            }
        }))
        .expect("incremental ASF fixture response"))
    }
}

struct FixtureWorker;

impl DownloadWorker for FixtureWorker {
    fn execute(
        &self,
        _work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        panic!("catalogue freeze never creates download work")
    }
}

#[derive(Default)]
struct RecordingWorker {
    items: Mutex<Vec<SnapshotItem>>,
}

impl DownloadWorker for RecordingWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.items.lock().unwrap().push(work.item.clone());
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root.join("fixture-complete.zip"),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-07-27T00:00:01Z".to_owned(),
        }
    }
}

struct StaticCatalog {
    response: CatalogSearchResponse,
}

#[derive(Default)]
struct RecordingLandsatBrowse {
    calls: Mutex<Vec<(CatalogSearchQuery, String)>>,
}

impl LiveLandsatBrowse for RecordingLandsatBrowse {
    fn fetch_landsat_browse(
        &self,
        query: &CatalogSearchQuery,
        entity_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<LandsatBrowseImage>, insar_provider::usgs_m2m::UsgsM2mError> {
        assert!(!cancelled());
        self.calls
            .lock()
            .unwrap()
            .push((query.clone(), entity_id.to_owned()));
        Ok(Some(
            LandsatBrowseImage::from_parts("image/jpeg", vec![0xff, 0xd8, 0xff, 0xd9])
                .expect("bounded synthetic JPEG"),
        ))
    }
}

type Sentinel2ManifestCall = (String, Option<String>, Vec<String>);

#[derive(Clone)]
struct StaticSentinel2BandEvidence {
    manifest: MpcSentinel2BandManifest,
    sizes: BTreeMap<String, u64>,
    manifest_calls: Arc<Mutex<Vec<Sentinel2ManifestCall>>>,
    probe_calls: Arc<Mutex<Vec<(String, String, String)>>>,
}

impl LiveSentinel2BandEvidence for StaticSentinel2BandEvidence {
    fn resolve_band_manifest(
        &self,
        safe_name: &str,
        acquisition_datetime: Option<&str>,
        asset_keys: &[String],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<MpcSentinel2BandManifest, ContractError> {
        assert!(!cancelled());
        assert_eq!(safe_name, self.manifest.safe_name);
        let requested = asset_keys.to_vec();
        self.manifest_calls.lock().unwrap().push((
            safe_name.to_owned(),
            acquisition_datetime.map(str::to_owned),
            requested.clone(),
        ));
        assert_eq!(
            requested,
            self.manifest
                .assets
                .iter()
                .map(|asset| asset.asset_key.clone())
                .collect::<Vec<_>>()
        );
        Ok(self.manifest.clone())
    }

    fn probe_band_asset(
        &self,
        provider_product_id: &str,
        asset_key: &str,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerReviewedAssetEvidence, ContractError> {
        assert!(!cancelled());
        assert_eq!(provider_product_id, self.manifest.item_id);
        let asset = self
            .manifest
            .assets
            .iter()
            .find(|asset| asset.asset_key == asset_key)
            .expect("fixture band asset");
        assert_eq!(stable_href, asset.stable_href);
        self.probe_calls.lock().unwrap().push((
            provider_product_id.to_owned(),
            asset_key.to_owned(),
            stable_href.to_owned(),
        ));
        Ok(PlanetaryComputerReviewedAssetEvidence {
            file_name: asset.file_name.clone(),
            expected_size_bytes: *self.sizes.get(asset_key).expect("fixture band size"),
        })
    }
}

struct BlockingLandsatBrowse {
    entered: AtomicBool,
}

#[derive(Default)]
struct RecordingLandsatBundleOptions {
    calls: Mutex<Vec<(String, String, String)>>,
    conflict: bool,
}

#[derive(Default)]
struct RecordingLandsatAssetManifest {
    calls: Mutex<Vec<(String, String, String)>>,
    conflict: bool,
}

impl LiveLandsatAssetManifest for RecordingLandsatAssetManifest {
    fn landsat_asset_manifest(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerLandsatManifest, PlanetaryComputerError> {
        assert!(!cancelled());
        self.calls.lock().unwrap().push((
            dataset_alias.to_owned(),
            entity_id.to_owned(),
            display_id.to_owned(),
        ));
        let mut manifest = planetary_computer_manifest();
        if self.conflict {
            manifest.entity_id = "LC80000000000000000000".to_owned();
        } else {
            manifest.entity_id = entity_id.to_owned();
        }
        Ok(manifest)
    }
}

impl LiveLandsatBundleOptions for RecordingLandsatBundleOptions {
    fn full_bundle_option(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mFullBundleOption, insar_provider::usgs_m2m::UsgsM2mError> {
        assert!(!cancelled());
        self.calls.lock().unwrap().push((
            dataset_alias.to_owned(),
            entity_id.to_owned(),
            display_id.to_owned(),
        ));
        let mut option = landsat_bundle_option();
        if self.conflict {
            option.entity_id = "LC80000000000000000000".to_owned();
        }
        Ok(option)
    }
}

impl LiveLandsatBrowse for BlockingLandsatBrowse {
    fn fetch_landsat_browse(
        &self,
        _query: &CatalogSearchQuery,
        _entity_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<LandsatBrowseImage>, insar_provider::usgs_m2m::UsgsM2mError> {
        self.entered.store(true, Ordering::Release);
        while !cancelled() {
            thread::sleep(Duration::from_millis(2));
        }
        Ok(Some(
            LandsatBrowseImage::from_parts("image/png", vec![0x89, b'P', b'N', b'G'])
                .expect("bounded synthetic PNG"),
        ))
    }
}

impl LiveCatalogSearch for StaticCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        if cancelled() {
            return Err(ContractError::new(
                "CATALOGUE_REQUEST_CANCELLED",
                "fixture catalogue cancelled",
                Map::new(),
            )
            .unwrap());
        }
        Ok(self.response.clone())
    }
}

struct FailingCatalog;

impl LiveCatalogSearch for FailingCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        Err(ContractError::new(
            "CATALOGUE_RESULT_LIMIT_EXCEEDED",
            "fixture exhaustive record protection",
            Map::from_iter([("reason".to_owned(), json!("record_budget"))]),
        )
        .unwrap())
    }
}

struct SelectiveSentinel2ArchiveCatalog {
    response: CatalogSearchResponse,
    fail_spacecraft: &'static str,
}

impl LiveCatalogSearch for SelectiveSentinel2ArchiveCatalog {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        if cancelled() {
            return Err(ContractError::new(
                "CATALOGUE_REQUEST_CANCELLED",
                "fixture catalogue cancelled",
                Map::new(),
            )
            .unwrap());
        }
        let spacecraft = query
            .filters
            .get("spacecraft")
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(Value::as_str)
            .unwrap_or_default();
        if spacecraft == self.fail_spacecraft {
            return Err(ContractError::new(
                "CATALOGUE_UPSTREAM_TIMEOUT",
                "fixture provider timeout",
                Map::new(),
            )
            .unwrap());
        }
        Ok(self.response.clone())
    }
}

struct BlockingCatalog {
    response: CatalogSearchResponse,
    entered: AtomicBool,
    release: AtomicBool,
}

impl LiveCatalogSearch for BlockingCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.entered.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) {
            if cancelled() {
                return Err(ContractError::new(
                    "CATALOGUE_REQUEST_CANCELLED",
                    "fixture catalogue cancelled",
                    Map::new(),
                )
                .unwrap());
            }
            thread::sleep(Duration::from_millis(2));
        }
        Ok(self.response.clone())
    }
}

struct SnapshotProbeCatalog {
    response: CatalogSearchResponse,
    service: Mutex<Option<Weak<ApplicationService>>>,
    observed_store_read: AtomicBool,
}

impl LiveCatalogSearch for SnapshotProbeCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let service = self
            .service
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade);
        self.observed_store_read.store(
            service.is_some_and(|service| service.snapshot(true).is_ok()),
            Ordering::Release,
        );
        Ok(self.response.clone())
    }
}

struct PublicationGate {
    entered: AtomicBool,
    release: AtomicBool,
}

impl PublicationGate {
    fn wait(&self) {
        self.entered.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

fn aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn bind_aoi(service: &ApplicationService, label: &str) {
    let region_id = common::ensure_fixture_region(service, label);
    let request = ApplicationBindRegionAoiRequest {
        request_id: format!("{label}-bind-aoi"),
        region_id,
        expected_revision: None,
        source_kind: "manual_bbox".to_owned(),
        geometry: aoi(),
        bbox: vec![110.0, 30.0, 111.0, 31.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "Catalogue fixture AOI".to_owned(),
        bound_at: "2026-07-29T00:00:00Z".to_owned(),
    };
    service.bind_region_aoi(&request).unwrap();
}

fn search() -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: "optical.sentinel-2.l2a".to_owned(),
        provider_id: "copernicus.dataspace.odata".to_owned(),
        native_collection_id: "SENTINEL-2".to_owned(),
        aoi_geojson: Some(aoi()),
        cursor: None,
        result_limit: Some(2),
        filters: json!({
            "product_type": "S2MSI2A",
            "cloud_cover_lte": 4.2,
            "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z"}
        })
        .as_object()
        .unwrap()
        .clone(),
    }
}

fn request(request_id: &str, batch_id: &str) -> ApplicationCatalogSearchAndFreezeRequest {
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: request_id.to_owned(),
        search: search(),
        batch_id: batch_id.to_owned(),
        created_at: "2026-07-26T00:00:00Z".to_owned(),
        aoi_name: "Fixture AOI".to_owned(),
        aoi_geojson: Some(aoi()),
        workbench_page_size: 24,
    }
}

fn legacy_asf_request() -> ApplicationSarSearchAndFreezeRequest {
    let aoi_geojson = aoi();
    ApplicationSarSearchAndFreezeRequest {
        request_id: "legacy-asf-search-stats".to_owned(),
        search: serde_json::from_value(json!({
            "query": {
                "provider": "asf",
                "dataset": "SENTINEL-1",
                "aoi_geojson": aoi_geojson,
                "max_results": 20
            }
        }))
        .expect("legacy ASF fixture query"),
        batch_id: "legacy-asf-search-stats-batch".to_owned(),
        created_at: "2026-08-17T00:00:00Z".to_owned(),
        aoi_name: "Legacy ASF fixture AOI".to_owned(),
        aoi_geojson: Some(aoi_geojson),
        page_size: 24,
    }
}

fn no_aoi_sar_request(
    target: u64,
    request_id: &str,
    batch_id: &str,
) -> ApplicationSarSearchAndFreezeRequest {
    ApplicationSarSearchAndFreezeRequest {
        request_id: request_id.to_owned(),
        search: serde_json::from_value(json!({
            "query": {
                "provider": "asf",
                "dataset": "SENTINEL-1",
                "processing_levels": ["SLC"],
                "max_results": target
            }
        }))
        .expect("no-AOI typed SAR query"),
        batch_id: batch_id.to_owned(),
        created_at: "2026-08-18T00:00:00Z".to_owned(),
        aoi_name: "No AOI".to_owned(),
        aoi_geojson: None,
        page_size: 24,
    }
}

#[test]
fn frozen_sar_batch_preserves_provider_search_stats() {
    let database = TempStore::new("legacy-asf-search-stats");
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(LegacyAsfFixture {
            source: "ASF_BATCH".to_owned(),
            calls: None,
        }),
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "legacy-asf-search-stats");

    let snapshot = service
        .sar_search_and_freeze_legacy(&legacy_asf_request())
        .expect("legacy ASF response freezes atomically");
    let batch = snapshot
        .restore
        .workbench_batch
        .expect("frozen legacy ASF batch");
    assert_eq!(batch.candidates.len(), 20);
    let frozen_ids = batch
        .candidates
        .iter()
        .take(3)
        .map(|candidate| candidate.item_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        frozen_ids,
        [
            "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::Z",
            "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::A",
            "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::M",
        ]
    );
    let mut lexicographic_ids = frozen_ids.clone();
    lexicographic_ids.sort_unstable();
    assert_eq!(
        lexicographic_ids,
        [
            "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::A",
            "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::M",
            "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::Z",
        ],
        "this fixture must fail if a lexical key sort replaces the provider response order"
    );

    let page = service
        .workbench_page(&StoreWorkbenchPageRequest {
            batch_id: "legacy-asf-search-stats-batch".to_owned(),
            expected_revision: 0,
            page_index: 0,
            page_size: 24,
        })
        .expect("ranked frozen batch page is readable")
        .page;
    let paged_ids = page
        .items
        .iter()
        .take(3)
        .map(|item| item.item.item_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paged_ids, frozen_ids);

    let stats =
        serde_json::to_value(&batch).expect("frozen batch serializes")["search_stats"].clone();
    assert_eq!(stats["matched_count"], 51_594);
    assert_eq!(stats["fetched_record_count"], 20);
    assert_eq!(stats["mapped_record_count"], 20);
    assert_eq!(stats["provider_returned_count"], 20);
    assert_eq!(stats["frozen_candidate_count"], 20);
    // #22 B slice 3(a): the finite target (below the ceiling) was met, so the
    // frozen batch is not flagged; matched_count still records 51,594.
    assert_eq!(stats["truncated"], false);
    assert_eq!(stats["source"], "ASF_BATCH");
    let serialized_stats = stats.to_string().to_ascii_lowercase();
    for forbidden in [
        "http://",
        "https://",
        "authorization",
        "credential",
        "geometry",
    ] {
        assert!(
            !serialized_stats.contains(forbidden),
            "frozen search statistics must remain bounded: {forbidden}"
        );
    }
    service.shutdown().unwrap();
}

#[test]
fn numeric_search_freezes_first_then_background_count_updates_only_the_same_batch_revision() {
    let database = TempStore::new("numeric-background-count");
    let search_calls = Arc::new(AtomicU64::new(0));
    let count_calls = Arc::new(AtomicU64::new(0));
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(BackgroundCountAsfFixture {
            search_calls: Arc::clone(&search_calls),
            count_calls: Arc::clone(&count_calls),
            total_count: 11_976,
        }),
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "numeric-background-count");
    let request = legacy_asf_request();
    let foreground = service
        .sar_search_and_freeze(&request)
        .expect("numeric foreground search freezes first");
    assert_eq!(search_calls.load(Ordering::Relaxed), 1);
    assert_eq!(count_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        foreground
            .core
            .workbench
            .as_ref()
            .and_then(|workbench| workbench.search_stats.as_ref())
            .and_then(|stats| stats.matched_count),
        None,
    );

    let counted = service
        .sar_background_count_and_update(&ApplicationSarBackgroundCountRequest {
            request_id: "numeric-background-count-only".to_owned(),
            batch_id: request.batch_id.clone(),
            expected_workbench_revision: 0,
        })
        .expect("background count is admitted");
    assert!(counted.applied);
    assert_eq!(counted.total_count, Some(11_976));
    assert_eq!(count_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        counted
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.core.workbench.as_ref())
            .and_then(|workbench| workbench.search_stats.as_ref())
            .and_then(|stats| stats.matched_count),
        Some(11_976),
    );

    let mut next = request.clone();
    next.request_id = "numeric-background-count-next".to_owned();
    next.batch_id = "numeric-background-count-next-batch".to_owned();
    next.created_at = "2026-08-17T00:00:01Z".to_owned();
    service
        .sar_search_and_freeze(&next)
        .expect("newer batch becomes active");
    let stale = service
        .sar_background_count_and_update(&ApplicationSarBackgroundCountRequest {
            request_id: "numeric-background-count-stale".to_owned(),
            batch_id: request.batch_id,
            expected_workbench_revision: 0,
        })
        .expect("stale count is discarded without failure");
    assert!(!stale.applied);
    assert_eq!(stale.snapshot, None);
    assert_eq!(
        count_calls.load(Ordering::Relaxed),
        1,
        "stale batch never reaches Provider"
    );
}

#[test]
fn typed_sar_search_reports_real_application_stages_in_order() {
    let database = TempStore::new("typed-sar-progress");
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(LegacyAsfFixture {
            source: "ASF_BATCH".to_owned(),
            calls: None,
        }),
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "typed-sar-progress");
    let updates = Mutex::new(Vec::new());

    service
        .sar_search_and_freeze_with_progress(&legacy_asf_request(), |update| {
            updates.lock().unwrap().push(update);
        })
        .expect("typed SAR search freezes with progress");

    let updates = updates.into_inner().unwrap();
    assert_eq!(
        updates
            .iter()
            .map(|update| update.stage.clone())
            .collect::<Vec<_>>(),
        vec![
            SarSearchProgressStage::Validating,
            SarSearchProgressStage::Planning,
            SarSearchProgressStage::WaitingProvider,
            SarSearchProgressStage::Fetching,
            SarSearchProgressStage::Fetching,
            SarSearchProgressStage::Filtering,
            SarSearchProgressStage::Freezing,
            SarSearchProgressStage::Completed,
        ]
    );
    assert_eq!(
        updates
            .iter()
            .map(|update| update.sequence)
            .collect::<Vec<_>>(),
        (1..=updates.len() as u64).collect::<Vec<_>>()
    );
    assert_eq!(updates[0].provider_request_count, None);
    assert_eq!(updates[0].raw_fetched_count, None);
    assert_eq!(updates[0].admitted_count, None);
    let measured = updates
        .iter()
        .find(|update| update.stage == SarSearchProgressStage::Fetching)
        .expect("provider observations own quantitative facts before final filtering");
    assert_eq!(measured.admitted_count, Some(10));
    assert_eq!(measured.requested_limit, Some(20));
    assert_eq!(measured.message, "已载入 10 / 20");
    let normalized = updates
        .iter()
        .find(|update| update.stage == SarSearchProgressStage::Filtering)
        .expect("finite filtering keeps remote count separate from the working-set target");
    assert_eq!(normalized.admitted_count, Some(20));
    assert_eq!(normalized.requested_limit, Some(20));
    assert_eq!(normalized.message, "候选总量 51594 · 已载入 20");
    let post_provider = updates
        .iter()
        .filter(|update| {
            matches!(
                update.stage,
                SarSearchProgressStage::Filtering
                    | SarSearchProgressStage::Freezing
                    | SarSearchProgressStage::Completed
            )
        })
        .collect::<Vec<_>>();
    assert!(
        post_provider.iter().all(|update| {
            update.provider_request_count == Some(2)
                && update.raw_fetched_count == Some(20)
                && update.mapped_count == Some(20)
                && update.exact_matched_count == Some(20)
                && update.admitted_count == Some(20)
                && update.requested_limit == Some(20)
                && update.remote_total == Some(51_594)
        }),
        "filtering, freezing, and terminal must retain the latest truthful observations"
    );
    service.shutdown().unwrap();
}

#[test]
fn cancelled_typed_sar_search_publishes_cancelled_instead_of_failed() {
    let database = TempStore::new("typed-sar-cancelled-progress");
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(CancelledAsfFixture),
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "typed-sar-cancelled-progress");
    let updates = Mutex::new(Vec::new());

    let error = service
        .sar_search_and_freeze_with_progress(&legacy_asf_request(), |update| {
            updates.lock().unwrap().push(update);
        })
        .expect_err("cancelled provider request remains terminal");

    assert_eq!(error.code(), "DL001");
    let updates = updates.into_inner().unwrap();
    assert_eq!(
        updates
            .iter()
            .rev()
            .take(2)
            .map(|update| update.stage.clone())
            .collect::<Vec<_>>(),
        vec![
            SarSearchProgressStage::Cancelled,
            SarSearchProgressStage::Cancelling,
        ]
    );
    assert_eq!(
        updates
            .iter()
            .map(|update| update.sequence)
            .collect::<Vec<_>>(),
        (1..=updates.len() as u64).collect::<Vec<_>>()
    );
    service.shutdown().unwrap();
}

#[test]
fn deterministic_sentinel_rectangle_and_inner_mongolia_multipolygon_n30_n250_matrix() {
    let geometries = [
        (
            "rectangle",
            json!({
                "type": "Polygon",
                "coordinates": [[[97.0, 37.0], [126.0, 37.0], [126.0, 53.0], [97.0, 53.0], [97.0, 37.0]]]
            })
            .as_object()
            .unwrap()
            .clone(),
            vec![97.0, 37.0, 126.0, 53.0],
            "POLYGON",
        ),
        (
            "inner-mongolia-multipolygon",
            json!({
                "type": "MultiPolygon",
                "coordinates": [
                    [[[97.0, 37.0], [108.0, 37.0], [108.0, 45.0], [97.0, 45.0], [97.0, 37.0]]],
                    [[[108.5, 42.0], [126.0, 42.0], [126.0, 53.0], [108.5, 53.0], [108.5, 42.0]]]
                ]
            })
            .as_object()
            .unwrap()
            .clone(),
            vec![97.0, 37.0, 126.0, 53.0],
            "MULTIPOLYGON",
        ),
    ];

    for (geometry_name, geometry, bbox, wkt_prefix) in geometries {
        for (requested_limit, provider_limit) in [(30_u64, 120_u64), (250, 750)] {
            let label = format!("sentinel-matrix-{geometry_name}-{requested_limit}");
            let database = TempStore::new(&label);
            let metadata_calls = Arc::new(Mutex::new(Vec::new()));
            let count_calls = Arc::new(AtomicU64::new(0));
            let provider = AsfProvider::with_transport(
                DeterministicSentinelMatrixTransport {
                    metadata_calls: Arc::clone(&metadata_calls),
                    count_calls: Arc::clone(&count_calls),
                },
                "2026-08-29T00:00:00Z",
            );
            let service = ApplicationService::with_catalogue_fixture_business_runtime(
                ApplicationServiceConfig::business_store(&database.0),
                Arc::new(DeterministicSentinelMatrixFixture { provider }),
                Arc::new(StaticCatalog {
                    response: valid_response(),
                }),
                Arc::new(FixtureWorker),
            );
            let region_id = common::ensure_fixture_region(&service, &label);
            service
                .bind_region_aoi(&ApplicationBindRegionAoiRequest {
                    request_id: format!("{label}-bind"),
                    region_id,
                    expected_revision: None,
                    source_kind: "manual_bbox".to_owned(),
                    geometry: geometry.clone(),
                    bbox: bbox.clone(),
                    crs: "EPSG:4326".to_owned(),
                    display_name: format!("Deterministic {geometry_name}"),
                    bound_at: "2026-08-29T00:00:00Z".to_owned(),
                })
                .expect("matrix AOI binds");
            let request = ApplicationSarSearchAndFreezeRequest {
                request_id: format!("{label}-request"),
                search: serde_json::from_value(json!({
                    "query": {
                        "provider": "asf",
                        "dataset": "SENTINEL-1",
                        "processing_levels": ["SLC"],
                        "aoi_geojson": geometry,
                        "start": "2024-01-01T00:00:00Z",
                        "end": "2024-01-02T00:00:00Z",
                        "max_results": requested_limit
                    }
                }))
                .expect("matrix query validates"),
                batch_id: format!("{label}-batch"),
                created_at: "2026-08-29T00:00:00Z".to_owned(),
                aoi_name: format!("Deterministic {geometry_name}"),
                aoi_geojson: Some(geometry.clone()),
                page_size: 24,
            };
            let progress = Mutex::new(Vec::new());
            service
                .sar_search_and_freeze_with_progress(&request, |update| {
                    progress.lock().unwrap().push(update);
                })
                .expect("deterministic Sentinel search freezes");
            let snapshot = service.snapshot(true).expect("matrix snapshot restores");

            let calls = metadata_calls.lock().unwrap();
            assert_eq!(calls.len(), 1, "finite working set stops after one page");
            assert_eq!(calls[0]["maxResults"], provider_limit.to_string());
            assert_ne!(provider_limit, requested_limit);
            assert!(calls[0]["intersectsWith"].starts_with(wkt_prefix));
            assert_eq!(count_calls.load(Ordering::Relaxed), 0);

            let batch = snapshot
                .restore
                .workbench_batch
                .expect("matrix batch freezes atomically");
            assert_eq!(batch.candidates.len(), requested_limit as usize);
            assert_eq!(batch.query["aoi_geojson"], Value::Object(geometry.clone()));
            assert_eq!(batch.query["max_results"], requested_limit);
            let workbench = snapshot
                .restore
                .workbench
                .expect("matrix workbench activates");
            assert_eq!(workbench.batch_id, batch.batch_id);
            assert_eq!(workbench.page_size, 24);

            let progress = progress.into_inner().unwrap();
            assert_eq!(
                progress
                    .iter()
                    .map(|update| update.sequence)
                    .collect::<Vec<_>>(),
                (1..=progress.len() as u64).collect::<Vec<_>>()
            );
            assert_eq!(progress.first().unwrap().provider_request_count, None);
            let terminal = progress.last().unwrap();
            assert_eq!(terminal.stage, SarSearchProgressStage::Completed);
            assert!(terminal.terminal);
            assert_eq!(terminal.provider_request_count, Some(1));
            assert_eq!(terminal.raw_fetched_count, Some(provider_limit));
            assert_eq!(terminal.mapped_count, Some(provider_limit));
            assert_eq!(terminal.exact_matched_count, Some(provider_limit));
            assert_eq!(terminal.admitted_count, Some(requested_limit));
            assert_eq!(terminal.requested_limit, Some(requested_limit));
            assert_eq!(terminal.remote_total, None);
            service.shutdown().unwrap();
        }
    }
}

#[test]
fn repeated_typed_sar_search_reuses_the_immutable_batch_pointer_after_reopen() {
    let database = TempStore::new("typed-sar-cache-reuse");
    let calls = Arc::new(AtomicU64::new(0));
    let fixture = || {
        Arc::new(LegacyAsfFixture {
            source: "ASF_BATCH".to_owned(),
            calls: Some(Arc::clone(&calls)),
        })
    };
    let mut first = legacy_asf_request();
    first.request_id = "typed-sar-cache-first".to_owned();
    first.batch_id = "typed-sar-cache-first-batch".to_owned();
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        fixture(),
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "typed-sar-cache-reuse");
    service
        .sar_search_and_freeze(&first)
        .expect("first typed SAR search freezes");

    let mut second = first.clone();
    second.request_id = "typed-sar-cache-second".to_owned();
    second.batch_id = "typed-sar-cache-second-batch".to_owned();
    second.created_at = "2026-08-17T00:00:01Z".to_owned();
    let snapshot = service
        .sar_search_and_freeze_legacy(&second)
        .expect("identical typed SAR search reuses cache");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let batch = snapshot
        .restore
        .workbench_batch
        .expect("second active batch");
    assert_eq!(batch.batch_id, second.batch_id);
    assert_eq!(
        batch
            .candidates
            .iter()
            .take(3)
            .map(|candidate| candidate.item_id.rsplit("::").next().unwrap())
            .collect::<Vec<_>>(),
        vec!["Z", "A", "M"]
    );
    assert_eq!(
        batch
            .search_stats
            .as_ref()
            .and_then(|stats| stats.matched_count),
        Some(51_594)
    );
    service.shutdown().unwrap();
    drop(service);

    let reopened = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        fixture(),
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
        Arc::new(FixtureWorker),
    );
    let mut third = second.clone();
    third.request_id = "typed-sar-cache-third".to_owned();
    third.batch_id = "typed-sar-cache-third-batch".to_owned();
    third.created_at = "2026-08-17T00:00:02Z".to_owned();
    let snapshot = reopened
        .sar_search_and_freeze_legacy(&third)
        .expect("reopened service reuses cache pointer");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        snapshot.restore.workbench_batch.unwrap().batch_id,
        third.batch_id
    );
    reopened.shutdown().unwrap();
}

#[test]
fn larger_no_aoi_target_fetches_only_the_strictly_older_missing_window() {
    let database = TempStore::new("typed-sar-cache-incremental");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(IncrementalAsfFixture {
            requests: Arc::clone(&requests),
        }),
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
        Arc::new(FixtureWorker),
    );
    service
        .sar_search_and_freeze(&no_aoi_sar_request(
            2,
            "typed-sar-incremental-first",
            "typed-sar-incremental-first-batch",
        ))
        .expect("initial bounded no-AOI search");
    let second = no_aoi_sar_request(
        4,
        "typed-sar-incremental-second",
        "typed-sar-incremental-second-batch",
    );
    let snapshot = service
        .sar_search_and_freeze_legacy(&second)
        .expect("larger target extends the immutable batch");
    let request_log = requests.lock().unwrap();
    assert_eq!(request_log.len(), 2);
    assert_eq!(request_log[0].query.max_results, Some(2));
    assert_eq!(request_log[1].query.max_results, Some(2));
    assert_eq!(
        request_log[1].query.end.as_deref(),
        Some("2024-02-01T23:59:59.999999999Z")
    );
    drop(request_log);

    let batch = snapshot
        .restore
        .workbench_batch
        .expect("extended active batch");
    assert_eq!(batch.batch_id, second.batch_id);
    assert_eq!(
        batch
            .candidates
            .iter()
            .map(|candidate| candidate.item_id.rsplit("::").next().unwrap())
            .collect::<Vec<_>>(),
        vec!["Z", "A", "M", "B"]
    );
    let stats = batch.search_stats.expect("extended search statistics");
    assert_eq!(stats.matched_count, Some(4));
    assert_eq!(stats.fetched_record_count, Some(4));
    assert_eq!(stats.mapped_record_count, Some(4));
    assert_eq!(stats.provider_returned_count, 4);
    assert_eq!(stats.frozen_candidate_count, 4);
    assert!(!stats.truncated);

    let third = no_aoi_sar_request(
        4,
        "typed-sar-incremental-third",
        "typed-sar-incremental-third-batch",
    );
    service
        .sar_search_and_freeze(&third)
        .expect("complete extended target reuses without Provider");
    assert_eq!(requests.lock().unwrap().len(), 2);
    service.shutdown().unwrap();
}

#[test]
fn frozen_sar_batch_canonicalizes_untrusted_provider_search_stat_sources() {
    for (label, source, expected) in [
        ("source-cmr", "CMR".to_owned(), "ASF_CMR"),
        (
            "source-url",
            "https://example.invalid/search".to_owned(),
            "ASF",
        ),
        ("source-credential", "Bearer credential".to_owned(), "ASF"),
        ("source-oversize", "x".repeat(257), "ASF"),
    ] {
        let database = TempStore::new(label);
        let service = ApplicationService::with_catalogue_fixture_business_runtime(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(LegacyAsfFixture {
                source,
                calls: None,
            }),
            Arc::new(StaticCatalog {
                response: valid_response(),
            }),
            Arc::new(FixtureWorker),
        );
        bind_aoi(&service, label);

        let snapshot = service
            .sar_search_and_freeze_legacy(&legacy_asf_request())
            .expect("legacy ASF response freezes atomically");
        let batch = snapshot
            .restore
            .workbench_batch
            .expect("frozen legacy ASF batch");
        assert_eq!(
            batch
                .search_stats
                .as_ref()
                .map(|stats| stats.source.as_str()),
            Some(expected),
            "{label}"
        );
        service.shutdown().unwrap();
    }
}

fn approved_product_search() -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: "optical.sentinel-2.l2a".to_owned(),
        provider_id: "copernicus.dataspace.odata".to_owned(),
        native_collection_id: "SENTINEL-2".to_owned(),
        aoi_geojson: Some(aoi()),
        cursor: None,
        result_limit: Some(24),
        filters: json!({
            "product_type":"S2MSI2A",
            "cloud_cover_lte":100.0,
            "datetime":{"start":"2026-07-26T12:51:21Z","end":"2026-07-26T12:55:21Z"}
        })
        .as_object()
        .unwrap()
        .clone(),
    }
}

fn approved_product_request() -> ApplicationCatalogSearchAndFreezeRequest {
    let search = approved_product_search();
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: "production-omitted-collection".to_owned(),
        batch_id: "production-omitted-collection-batch".to_owned(),
        created_at: "2026-07-27T00:00:00Z".to_owned(),
        aoi_name: "Synthetic production AOI".to_owned(),
        aoi_geojson: search.aoi_geojson.clone(),
        search,
        workbench_page_size: 24,
    }
}

fn approved_product_response_without_collection_echo() -> CatalogSearchResponse {
    let query = approved_product_search();
    let facts = build_odata_request_for_use(&query, RouteUse::Production)
        .expect("production query facts")
        .facts()
        .clone();
    let page: ODataPage = serde_json::from_value(json!({
        "@odata.count": 1,
        "value": [{
            "Id":"165ba4c9-997c-4cea-970f-33ab9bdb8084",
            "Name":"S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE",
            "ContentLength":867907004,
            "ContentDate":{"Start":"2026-07-26T12:53:21Z"},
            "GeoFootprint":aoi(),
            "Online":true,
            "Checksum":[{"Algorithm":"MD5","Value":"00000000000000000000000000000000"}],
            "Attributes":[
                {"Name":"productType","Value":"S2MSI2A"},
                {"Name":"cloudCover","Value":1.0}
            ]
        }]
    }))
    .expect("synthetic approved product page");
    map_odata_page(page, &facts, None).expect("production mapper accepts omitted collection echo")
}

#[allow(clippy::too_many_arguments)]
fn catalog_search_stats(
    requested_limit: u64,
    matched_count: u64,
    fetched_record_count: u64,
    mapped_record_count: u64,
    profile_filtered_record_count: u64,
    exact_duplicate_record_count: u64,
    unique_provider_product_count: u64,
    exact_matched_count: u64,
    returned_count: u64,
    truncated: bool,
) -> Value {
    json!({
        "requested_limit": requested_limit,
        "matched_count": matched_count,
        "request_count": 1,
        "transport_part_count": 1,
        "fetched_record_count": fetched_record_count,
        "mapped_record_count": mapped_record_count,
        "profile_filtered_record_count": profile_filtered_record_count,
        "exact_duplicate_record_count": exact_duplicate_record_count,
        "unique_provider_product_count": unique_provider_product_count,
        "exact_matched_count": exact_matched_count,
        "returned_count": returned_count,
        "remote_exhausted": true,
        "working_set_satisfied": true,
        "elapsed_ms": 1,
        "truncated": truncated
    })
}

fn valid_response() -> CatalogSearchResponse {
    serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": "copernicus.dataspace.odata",
            "native_collection_id": "SENTINEL-2",
            "collection_profile_id": "cdse.sentinel-2.l2a.collection.v1",
            "identity_rule_id": "cdse.sentinel-2.l2a.identity.v1",
            "filename_rule_id": "cdse.sentinel-2.l2a.archive-filename.v1",
            "delivery_model": "archive_first",
            "production_enabled": false
        },
        "products": [{
            "provider_id": "copernicus.dataspace.odata",
            "product_id": "00000000-0000-4000-8000-000000000011",
            "display_name": "S2_FIXTURE_VALID.SAFE",
            "source_kind": "sentinel2",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2026-01-01T10:00:00Z",
            "footprint_geojson": {"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},
            "assets": [{
                "asset_key": "archive",
                "stable_asset_key": "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::00000000-0000-4000-8000-000000000011::ASSET::archive",
                "role": "primary",
                "native_roles": ["archive"],
                "size_bytes": 2048
            }],
            "properties": {"product_type":"S2MSI2A", "online":true, "cloud_cover":4.2},
            "payload": {}
        }],
        "stats": catalog_search_stats(2, 1, 1, 1, 0, 0, 1, 1, 1, false),
        "item_errors": []
    }))
    .unwrap()
}

fn exact_archive_product(product_id: &str) -> ProviderProduct {
    let mut product = valid_response().products.remove(0);
    product.product_id = product_id.to_owned();
    let product_key =
        provider_product_key("copernicus.dataspace.odata", "SENTINEL-2", product_id).unwrap();
    product.assets[0].stable_asset_key = Some(stable_asset_key(&product_key, "archive").unwrap());
    product
}

fn complete_response(products: Vec<ProviderProduct>) -> CatalogSearchResponse {
    let mut response = valid_response();
    let count = products.len() as u64;
    response.products = products;
    response.stats.matched_count = Some(count);
    response.stats.fetched_record_count = count;
    response.stats.mapped_record_count = count;
    response.stats.exact_duplicate_record_count = 0;
    response.stats.unique_provider_product_count = count;
    response.stats.returned_count = count;
    response.stats.truncated = false;
    response.item_errors.clear();
    response.next_cursor = None;
    response
}

fn assert_catalogue_response_rejected(label: &str, response: CatalogSearchResponse) {
    let database = TempStore::new(label);
    let service = fixture_service(&database, Arc::new(StaticCatalog { response }));
    assert!(
        service
            .catalog_search_and_freeze_legacy(&request(label, &format!("{label}-batch")))
            .is_err(),
        "{label}"
    );
    assert_empty_store(&service);
    service.shutdown().unwrap();
}

fn fixture_service(
    database: &TempStore,
    catalog: Arc<dyn LiveCatalogSearch>,
) -> ApplicationService {
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        catalog,
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "fixture");
    service
}

fn production_fixture_service(
    database: &TempStore,
    catalog: Arc<dyn LiveCatalogSearch>,
) -> ApplicationService {
    let service = ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        catalog,
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "fixture");
    service
}

fn production_response() -> CatalogSearchResponse {
    let mut response = valid_response();
    response.collection.production_enabled = true;
    let product = response.products.first_mut().expect("fixture product");
    product.properties.insert(
        "checksums".to_owned(),
        json!([{"algorithm": "MD5", "value": "00000000000000000000000000000000"}]),
    );
    product.properties.insert(
        "s3_path".to_owned(),
        json!("s3://provider-only/never-frozen"),
    );
    response
}

fn assert_empty_store(service: &ApplicationService) {
    let snapshot = service.snapshot(true).unwrap();
    assert!(snapshot.restore.workbench_batch.is_none());
    assert!(snapshot.restore.workbench.is_none());
    assert!(snapshot.restore.tasks.is_empty());
}

fn landsat_search() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": "optical.landsat.c2.l2",
        "provider_id": "usgs.eros.m2m",
        "native_collection_id": "landsat_ot_c2_l2",
        "aoi_geojson": {
            "type": "MultiPolygon",
            "coordinates": [[[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]]
        },
        "result_limit": 25,
        "filters": {
            "datetime": {"start":"2026-01-01T00:00:00Z", "end":"2026-01-03T00:00:00Z"},
            "cloud_cover_lte": 40.0,
            "spacecraft": ["landsat_8", "landsat_9"],
            "processing_levels": ["l2sp", "l2sr"]
        }
    }))
    .expect("closed Landsat scene query")
}

fn landsat_response() -> CatalogSearchResponse {
    serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.landsat.c2.l2",
            "provider_id": "usgs.eros.m2m",
            "native_collection_id": "landsat_ot_c2_l2",
            "collection_profile_id": "usgs.eros.m2m.landsat.c2.l2.scene.v1",
            "identity_rule_id": "usgs.eros.m2m.landsat.c2.l2.identity.v1",
            "filename_rule_id": "usgs.eros.m2m.landsat.c2.l2.full-bundle-filename.v1",
            "delivery_model": "archive_first",
            "production_enabled": false
        },
        "products": [{
            "provider_id": "usgs.eros.m2m",
            "product_id": "LC91230452026001LGN00",
            "display_name": "LC09_L2SP_123045_20260101_20260103_02_T1",
            "source_kind": "landsat",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2026-01-01T00:00:00Z",
            "footprint_geojson": {
                "type":"Polygon",
                "coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]
            },
            "assets": [],
            "properties": {
                "entity_id": "LC91230452026001LGN00",
                "landsat_product_id": "LC09_L2SP_123045_20260101_20260103_02_T1",
                "landsat_scene_id": "LC91230452026001LGN00",
                "dataset_alias": "landsat_ot_c2_l2",
                "spacecraft": "landsat_9",
                "processing_level": "l2sp",
                "data_type_l2": "OLI_TIRS_L2SP",
                "collection_category": "T1",
                "cloud_cover": 12.5,
                "wrs_path": "123",
                "wrs_row": "045",
                "acquisition_date": "2026/01/01",
                "acquisition_precision": "day",
                "browse_available": true
            },
            "payload": {}
        }],
        "stats": catalog_search_stats(25, 2, 2, 2, 0, 0, 2, 1, 1, false),
        "next_cursor": null,
        "item_errors": []
    }))
    .expect("valid scene-only Landsat response")
}

fn landsat_temporal_response(
    acquisition_datetime: &str,
    acquisition_date: &str,
    acquisition_precision: &str,
) -> CatalogSearchResponse {
    let mut response = landsat_response();
    let product = response
        .products
        .first_mut()
        .expect("Landsat fixture product");
    product.acquisition_datetime = Some(acquisition_datetime.to_owned());
    product.properties.insert(
        "acquisition_date".to_owned(),
        Value::String(acquisition_date.to_owned()),
    );
    product.properties.insert(
        "acquisition_precision".to_owned(),
        Value::String(acquisition_precision.to_owned()),
    );
    response
}

fn assert_landsat_response_rejected(label: &str, response: CatalogSearchResponse) {
    let database = TempStore::new(label);
    let service = fixture_service(&database, Arc::new(StaticCatalog { response }));
    let mut request = landsat_request();
    request.request_id = format!("{label}-request");
    request.batch_id = format!("{label}-batch");
    assert!(
        service.catalog_search_and_freeze_legacy(&request).is_err(),
        "{label}"
    );
    assert_empty_store(&service);
    service.shutdown().unwrap();
}

fn landsat_request() -> ApplicationCatalogSearchAndFreezeRequest {
    let search = landsat_search();
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: "landsat-p4a-2a-freeze".to_owned(),
        batch_id: "landsat-p4a-2a-batch".to_owned(),
        created_at: "2026-07-28T00:00:00Z".to_owned(),
        aoi_name: "Landsat synthetic AOI".to_owned(),
        aoi_geojson: search.aoi_geojson.clone(),
        search,
        workbench_page_size: 25,
    }
}

fn mpc_landsat_request() -> ApplicationCatalogSearchAndFreezeRequest {
    let mut request = landsat_request();
    request.request_id = "landsat-mpc-freeze".to_owned();
    request.batch_id = "landsat-mpc-batch".to_owned();
    request.search.provider_id = "microsoft.planetary-computer".to_owned();
    request.search.native_collection_id = "landsat-c2-l2".to_owned();
    request
}

fn mpc_landsat_response() -> CatalogSearchResponse {
    let mut response = landsat_response();
    response.collection.provider_id = "microsoft.planetary-computer".to_owned();
    response.collection.native_collection_id = "landsat-c2-l2".to_owned();
    response.collection.collection_profile_id =
        "planetary-computer.landsat-c2-l2.scene.v1".to_owned();
    response.collection.identity_rule_id =
        "planetary-computer.landsat-c2-l2.identity.v1".to_owned();
    response.collection.filename_rule_id =
        "planetary-computer.landsat-c2-l2.asset-filename.v1".to_owned();
    response.collection.production_enabled = true;
    let product = response.products.first_mut().expect("Landsat product");
    product.provider_id = "microsoft.planetary-computer".to_owned();
    product.product_id = "LC09_L2SP_123045_20260101_02_T1".to_owned();
    product.properties.insert(
        "entity_id".to_owned(),
        Value::String(product.product_id.clone()),
    );
    product.properties.insert(
        "dataset_alias".to_owned(),
        Value::String("landsat-c2-l2".to_owned()),
    );
    product.properties.insert(
        "landsat_scene_id".to_owned(),
        Value::String("LC91230452026001LGN00".to_owned()),
    );
    response
}

fn landsat_browse_service(
    database: &TempStore,
    browse: Arc<dyn LiveLandsatBrowse>,
) -> ApplicationService {
    let mut response = landsat_response();
    response.collection.production_enabled = true;
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_browse(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog { response }),
        browse,
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "landsat-browse");
    service
}

fn landsat_bundle_option() -> UsgsM2mFullBundleOption {
    UsgsM2mFullBundleOption {
        dataset_alias: "landsat_ot_c2_l2".to_owned(),
        dataset_id: "5e83d14f2fc39685".to_owned(),
        entity_id: "LC91230452026001LGN00".to_owned(),
        display_id: "LC09_L2SP_123045_20260101_20260103_02_T1".to_owned(),
        option_id: LANDSAT_C2_L2_BUNDLE_OPTION_ID.to_owned(),
        product_name: LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME.to_owned(),
        product_code: LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE.to_owned(),
        download_system: LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM.to_owned(),
        availability: UsgsM2mBundleAvailability::Available,
        expected_file_name: "LC09_L2SP_123045_20260101_20260103_02_T1.tar".to_owned(),
        expected_size_bytes: Some(958_605_073),
        checksum_algorithm: Some("sha512".to_owned()),
        checksum_value: Some("a".repeat(128)),
    }
}

fn planetary_computer_manifest() -> PlanetaryComputerLandsatManifest {
    let display_id = "LC09_L2SP_123045_20260101_20260103_02_T1";
    let definitions = [
        ("coastal", "_SR_B1.TIF", AssetRole::Primary),
        ("blue", "_SR_B2.TIF", AssetRole::Primary),
        ("green", "_SR_B3.TIF", AssetRole::Primary),
        ("red", "_SR_B4.TIF", AssetRole::Primary),
        ("nir08", "_SR_B5.TIF", AssetRole::Primary),
        ("swir16", "_SR_B6.TIF", AssetRole::Primary),
        ("swir22", "_SR_B7.TIF", AssetRole::Primary),
        ("lwir11", "_ST_B10.TIF", AssetRole::Primary),
        ("drad", "_ST_DRAD.TIF", AssetRole::Primary),
        ("emis", "_ST_EMIS.TIF", AssetRole::Primary),
        ("emsd", "_ST_EMSD.TIF", AssetRole::Primary),
        ("trad", "_ST_TRAD.TIF", AssetRole::Primary),
        ("urad", "_ST_URAD.TIF", AssetRole::Primary),
        ("atran", "_ST_ATRAN.TIF", AssetRole::Primary),
        ("cdist", "_ST_CDIST.TIF", AssetRole::Primary),
        ("qa", "_ST_QA.TIF", AssetRole::Auxiliary),
        ("qa_pixel", "_QA_PIXEL.TIF", AssetRole::Auxiliary),
        ("qa_radsat", "_QA_RADSAT.TIF", AssetRole::Auxiliary),
        ("qa_aerosol", "_SR_QA_AEROSOL.TIF", AssetRole::Auxiliary),
        ("ang", "_ANG.txt", AssetRole::Metadata),
        ("mtl.txt", "_MTL.txt", AssetRole::Metadata),
        ("mtl.xml", "_MTL.xml", AssetRole::Metadata),
    ];
    let assets = definitions
        .into_iter()
        .enumerate()
        .map(|(index, (asset_key, suffix, role))| {
            let file_name = format!("{display_id}{suffix}");
            PlanetaryComputerLandsatAsset {
                asset_key: asset_key.to_owned(),
                stable_href: format!(
                    "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2026/123/045/{display_id}/{file_name}"
                ),
                file_name,
                role,
                expected_size_bytes: 10_000 + index as u64,
            }
        })
        .collect::<Vec<_>>();
    PlanetaryComputerLandsatManifest {
        entity_id: "LC91230452026001LGN00".to_owned(),
        display_id: display_id.to_owned(),
        stac_item_id: "LC09_L2SP_123045_20260101_02_T1".to_owned(),
        processing_level: "l2sp".to_owned(),
        total_size_bytes: assets.iter().map(|asset| asset.expected_size_bytes).sum(),
        assets,
    }
}

fn landsat_delivery_service(
    database: &TempStore,
    options: Arc<dyn LiveLandsatBundleOptions>,
) -> ApplicationService {
    let mut response = landsat_response();
    response.collection.production_enabled = true;
    let mut second = response.products[0].clone();
    second.product_id = "LC91230462026002LGN00".to_owned();
    second.display_name = "LC09_L2SP_123046_20260102_20260104_02_T1".to_owned();
    second.acquisition_datetime = Some("2026-01-02T00:00:00Z".to_owned());
    second.properties["entity_id"] = Value::String(second.product_id.clone());
    second.properties["landsat_scene_id"] = Value::String(second.product_id.clone());
    second.properties["landsat_product_id"] = Value::String(second.display_name.clone());
    second.properties["wrs_row"] = Value::String("046".to_owned());
    second.properties["acquisition_date"] = Value::String("2026/01/02".to_owned());
    response.products.push(second);
    response.stats.returned_count = 2;
    response.stats.unique_provider_product_count = 2;
    response.stats.exact_matched_count = 2;
    response.stats.mapped_record_count = 2;
    let service =
        ApplicationService::with_catalogue_production_business_runtime_and_landsat_delivery(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(StaticCatalog { response }),
            options,
            Arc::new(RecordingWorker::default()),
        );
    bind_aoi(&service, "landsat-delivery");
    service
}

fn landsat_asset_service(
    database: &TempStore,
    manifest: Arc<dyn LiveLandsatAssetManifest>,
) -> ApplicationService {
    let mut response = landsat_response();
    response.collection.production_enabled = true;
    let mut second = response.products[0].clone();
    second.product_id = "LC91230462026002LGN00".to_owned();
    second.display_name = "LC09_L2SP_123046_20260102_20260104_02_T1".to_owned();
    second.acquisition_datetime = Some("2026-01-02T00:00:00Z".to_owned());
    second.properties["entity_id"] = Value::String(second.product_id.clone());
    second.properties["landsat_scene_id"] = Value::String(second.product_id.clone());
    second.properties["landsat_product_id"] = Value::String(second.display_name.clone());
    second.properties["wrs_row"] = Value::String("046".to_owned());
    second.properties["acquisition_date"] = Value::String("2026/01/02".to_owned());
    response.products.push(second);
    response.stats.returned_count = 2;
    response.stats.unique_provider_product_count = 2;
    response.stats.exact_matched_count = 2;
    response.stats.mapped_record_count = 2;
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_assets(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog { response }),
        manifest,
        Arc::new(RecordingWorker::default()),
    );
    bind_aoi(&service, "landsat-assets");
    service
}

fn mpc_landsat_asset_service(
    database: &TempStore,
    manifest: Arc<dyn LiveLandsatAssetManifest>,
) -> ApplicationService {
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_assets(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: mpc_landsat_response(),
        }),
        manifest,
        Arc::new(RecordingWorker::default()),
    );
    bind_aoi(&service, "landsat-mpc-assets");
    service
}

fn frozen_mpc_landsat_item_id() -> &'static str {
    "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC09_L2SP_123045_20260101_02_T1"
}

fn frozen_landsat_item_id() -> &'static str {
    "PROVIDER::usgs.eros.m2m::COLLECTION::landsat_ot_c2_l2::ITEM::LC91230452026001LGN00"
}

#[test]
fn landsat_full_bundle_task_derives_one_executable_candidate_and_preserves_the_source() {
    let database = TempStore::new("landsat-derived-bundle");
    let options = Arc::new(RecordingLandsatBundleOptions::default());
    let service = landsat_delivery_service(&database, options.clone());
    let source = service
        .catalog_search_and_freeze_legacy(&landsat_request())
        .expect("scene batch");
    let source_batch = source.restore.workbench_batch.unwrap();
    assert_eq!(source_batch.candidates.len(), 2);
    assert!(source_batch
        .candidates
        .iter()
        .all(|candidate| candidate.asset_access.is_none() && candidate.execution_policy.is_none()));
    let selected_output_root = database.0.parent().unwrap().join("landsat-bundle-output");

    let snapshot = service
        .create_landsat_bundle_task_legacy(&LandsatCreateBundleTaskRequest {
            request_id: "landsat-bundle-options-1".to_owned(),
            source_batch_id: source_batch.batch_id.clone(),
            item_id: frozen_landsat_item_id().to_owned(),
            task_id: "landsat-bundle-task-1".to_owned(),
            output_root: selected_output_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-29T08:00:00Z".to_owned(),
        })
        .expect("derived batch and task");

    assert_eq!(
        options.calls.lock().unwrap().as_slice(),
        [(
            "landsat_ot_c2_l2".to_owned(),
            "LC91230452026001LGN00".to_owned(),
            "LC09_L2SP_123045_20260101_20260103_02_T1".to_owned(),
        )]
    );
    let main_batch = snapshot.restore.workbench_batch.as_ref().unwrap();
    assert_eq!(main_batch.batch_id, source_batch.batch_id);
    assert_eq!(main_batch, &source_batch);

    let derived = service
        .task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: "landsat-bundle-task-1".to_owned(),
            page_index: 0,
            page_size: 24,
        })
        .expect("task-scoped derived source batch");
    assert_ne!(derived.source_batch_id, source_batch.batch_id);
    assert_eq!(derived.total_count as usize, source_batch.candidates.len());

    let selected = derived
        .items
        .iter()
        .map(|row| &row.item)
        .find(|candidate| candidate.item_id == frozen_landsat_item_id())
        .unwrap();
    let source_selected = source_batch
        .candidates
        .iter()
        .find(|candidate| candidate.item_id == selected.item_id)
        .unwrap();
    assert_eq!(selected.metadata, source_selected.metadata);
    assert_eq!(selected.payload, source_selected.payload);
    assert_eq!(
        selected.footprint_geojson,
        source_selected.footprint_geojson
    );
    let access = selected.asset_access.as_ref().unwrap();
    assert_eq!(access.reference.asset_key, LANDSAT_C2_L2_BUNDLE_OPTION_ID);
    assert_eq!(
        access.reference.provider_product_id,
        "LC91230452026001LGN00"
    );
    assert_eq!(access.role, AssetRole::Primary);
    assert_eq!(
        access.file_name.as_deref(),
        Some("LC09_L2SP_123045_20260101_20260103_02_T1.tar")
    );
    assert_eq!(access.expected_size_bytes, Some(958_605_073));
    assert_eq!(access.checksum_algorithm.as_deref(), Some("sha512"));
    assert_eq!(
        access.descriptor.credential_profile.as_deref(),
        Some("usgs.eros.m2m.session.v1")
    );
    assert_eq!(
        serde_json::to_value(&access.descriptor.scheme).unwrap(),
        "usgs_m2m"
    );
    assert!(access
        .descriptor
        .stable_href
        .as_deref()
        .unwrap()
        .starts_with("urn:usgs:m2m:landsat:"));
    assert_eq!(
        selected.execution_policy.as_ref().unwrap().authority,
        insar_core::AssetExecutionAuthority::ResolverAuthoritative
    );

    let source_unselected = &source_batch.candidates[1];
    let derived_unselected = derived
        .items
        .iter()
        .map(|row| &row.item)
        .find(|candidate| candidate.item_id == source_unselected.item_id)
        .unwrap();
    assert_eq!(derived_unselected, source_unselected);

    let task = snapshot
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "landsat-bundle-task-1")
        .unwrap();
    assert_eq!(task.batch_id, derived.source_batch_id);
    assert_eq!(task.initial_selected_item_ids, [frozen_landsat_item_id()]);
    assert_eq!(task.layout, DownloadLayout::Direct);
    assert_eq!(
        task.output_root,
        selected_output_root.join("LC09_L2SP_123045_20260101_20260103_02_T1")
    );

    let persisted = serde_json::to_string(&snapshot.restore)
        .unwrap()
        .to_ascii_lowercase();
    for forbidden in [
        "requestsignature",
        "x-auth-token",
        "authorization",
        "downloadid",
        "https://landsatlook.usgs.gov",
        "raw_response",
    ] {
        assert!(!persisted.contains(forbidden), "{forbidden}");
    }
    service.shutdown().unwrap();
}

#[test]
fn conflicting_bundle_option_leaves_the_original_batch_and_no_task() {
    let database = TempStore::new("landsat-derived-bundle-conflict");
    let options = Arc::new(RecordingLandsatBundleOptions {
        calls: Mutex::new(Vec::new()),
        conflict: true,
    });
    let service = landsat_delivery_service(&database, options.clone());
    service
        .catalog_search_and_freeze_legacy(&landsat_request())
        .expect("scene batch");

    let error = service
        .create_landsat_bundle_task_legacy(&LandsatCreateBundleTaskRequest {
            request_id: "landsat-bundle-options-conflict".to_owned(),
            source_batch_id: "landsat-p4a-2a-batch".to_owned(),
            item_id: frozen_landsat_item_id().to_owned(),
            task_id: "landsat-bundle-task-conflict".to_owned(),
            output_root: database.0.parent().unwrap().join("landsat-bundle-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-29T08:00:00Z".to_owned(),
        })
        .unwrap_err();
    assert_eq!(error.code(), "USGS_LANDSAT_BUNDLE_FROZEN_EVIDENCE_INVALID");
    assert_eq!(options.calls.lock().unwrap().len(), 1);
    let snapshot = service.snapshot(true).unwrap();
    assert_eq!(
        snapshot.restore.workbench_batch.unwrap().batch_id,
        "landsat-p4a-2a-batch"
    );
    assert!(snapshot.restore.tasks.is_empty());
    service.shutdown().unwrap();
}

#[test]
fn landsat_public_asset_task_freezes_all_files_without_replacing_the_scene_workbench() {
    let database = TempStore::new("landsat-public-assets");
    let manifest = Arc::new(RecordingLandsatAssetManifest::default());
    let service = mpc_landsat_asset_service(&database, manifest.clone());
    let source = service
        .catalog_search_and_freeze_legacy(&mpc_landsat_request())
        .expect("scene batch");
    let source_batch = source.restore.workbench_batch.unwrap();
    let selected_output_root = database.0.parent().unwrap().join("landsat-public-output");

    let snapshot = service
        .create_landsat_asset_task_legacy(&LandsatCreateBundleTaskRequest {
            request_id: "landsat-public-assets-1".to_owned(),
            source_batch_id: source_batch.batch_id.clone(),
            item_id: source_batch.candidates[0].item_id.clone(),
            task_id: "landsat-public-asset-task-1".to_owned(),
            output_root: selected_output_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 4,
            created_at: "2026-07-29T08:00:00Z".to_owned(),
        })
        .expect("complete public Landsat asset task");

    assert_eq!(
        manifest.calls.lock().unwrap().as_slice(),
        [(
            "landsat-c2-l2".to_owned(),
            "LC09_L2SP_123045_20260101_02_T1".to_owned(),
            "LC09_L2SP_123045_20260101_20260103_02_T1".to_owned(),
        )]
    );
    assert_eq!(
        snapshot.restore.workbench_batch.as_ref().unwrap(),
        &source_batch
    );
    let task = snapshot
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "landsat-public-asset-task-1")
        .expect("derived asset task");
    assert_eq!(task.query["derived_from_batch_id"], source_batch.batch_id);
    assert_eq!(
        task.query["delivery_kind"],
        "landsat_c2_l2_complete_asset_set"
    );
    assert_eq!(task.candidates.len(), 22);
    assert_eq!(task.initial_selected_item_ids.len(), 22);
    assert_eq!(
        task.output_root,
        selected_output_root.join("LC09_L2SP_123045_20260101_20260103_02_T1")
    );
    assert_eq!(
        task.candidates
            .iter()
            .filter(|item| item.asset_access.as_ref().unwrap().role == AssetRole::Primary)
            .count(),
        15
    );
    assert_eq!(
        task.candidates
            .iter()
            .filter(|item| item.asset_access.as_ref().unwrap().role == AssetRole::Auxiliary)
            .count(),
        4
    );
    assert_eq!(
        task.candidates
            .iter()
            .filter(|item| item.asset_access.as_ref().unwrap().role == AssetRole::Metadata)
            .count(),
        3
    );
    for item in &task.candidates {
        let access = item.asset_access.as_ref().unwrap();
        assert_eq!(item.item_id, access.reference.stable_asset_key);
        assert_eq!(item.display_name, access.file_name.as_deref().unwrap());
        assert_eq!(
            access.descriptor.resolver_id,
            "planetary-computer.landsat-c2-l2.assets.v1"
        );
        assert_eq!(
            access.descriptor.scheme,
            insar_core::AssetAccessScheme::PublicHttps
        );
        assert!(!access.descriptor.requires_credentials);
        assert!(access.descriptor.credential_profile.is_none());
        assert!(access.expected_size_bytes.is_some_and(|size| size > 0));
        assert!(access.checksum_algorithm.is_none());
        assert!(access.checksum_value.is_none());
        let href = access.descriptor.stable_href.as_deref().unwrap();
        assert!(href.starts_with("https://landsateuwest.blob.core.windows.net/landsat-c2/"));
        assert!(!href.contains('?'));
        assert_eq!(
            item.execution_policy.as_ref().unwrap().policy_id,
            "planetary-computer.landsat-c2-l2.asset-set.v2"
        );
    }
    let frozen_total_bytes = task
        .candidates
        .iter()
        .map(|item| {
            item.asset_access
                .as_ref()
                .unwrap()
                .expected_size_bytes
                .unwrap()
        })
        .sum::<u64>();
    let core = service.core_restore(true).expect("bounded core restore");
    let core_task = core
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == "landsat-public-asset-task-1")
        .expect("derived task summary");
    assert!(
        core_task.total_bytes.is_some_and(|total| total > 0),
        "frozen Landsat asset sizes must make task progress determinate before every queued item starts",
    );
    assert!(
        core_task.total_bytes.unwrap() <= frozen_total_bytes,
        "live transfer evidence may refine a frozen upper estimate but must not return to an unknown total",
    );

    let persisted = serde_json::to_string(task).unwrap().to_ascii_lowercase();
    for forbidden in ["sig=", "x-auth-token", "authorization", "requestsignature"] {
        assert!(!persisted.contains(forbidden), "{forbidden}");
    }

    // 22 local HTTP asset downloads can take longer than 5 s when other tests
    // share the process; keep the bound generous so parallel test execution
    // cannot make this completion assertion flaky.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let engine = service
            .download_snapshot("landsat-public-asset-task-1")
            .expect("asset task runtime");
        if engine.results.len() == 22 {
            assert!(engine
                .results
                .iter()
                .all(|result| result.outcome == DownloadOutcome::Completed));
            break;
        }
        assert!(Instant::now() < deadline, "asset task did not complete");
        thread::sleep(Duration::from_millis(10));
    }
    service.shutdown().unwrap();

    let reopened = ApplicationService::new(ApplicationServiceConfig::business_store(&database.0));
    let restored = reopened
        .download_snapshot("landsat-public-asset-task-1")
        .expect("completed asset task restores");
    assert_eq!(restored.results.len(), 22);
    let restore = reopened.snapshot(true).unwrap();
    assert_eq!(
        restore.restore.workbench_batch.as_ref().unwrap(),
        &source_batch
    );
    reopened.shutdown().unwrap();
}

#[test]
fn landsat_asset_selection_freezes_only_requested_files_inside_the_product_folder() {
    let database = TempStore::new("landsat-selected-assets");
    let manifest = Arc::new(RecordingLandsatAssetManifest::default());
    let service = mpc_landsat_asset_service(&database, manifest.clone());
    let source = service
        .catalog_search_and_freeze_legacy(&mpc_landsat_request())
        .expect("freeze Microsoft Landsat scene");
    let source_batch = source.restore.workbench_batch.expect("source batch");
    let selected_output_root = database.0.parent().unwrap().join("landsat-selected-output");

    service
        .create_landsat_asset_selection_task(&LandsatCreateAssetTaskRequest {
            request_id: "landsat-selected-assets-1".to_owned(),
            source_batch_id: source_batch.batch_id.clone(),
            item_id: frozen_mpc_landsat_item_id().to_owned(),
            asset_keys: vec!["red".to_owned(), "nir08".to_owned()],
            task_id: "landsat-selected-asset-task-1".to_owned(),
            output_root: selected_output_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-07-29T08:00:00Z".to_owned(),
        })
        .expect("selected Landsat asset task");
    let snapshot = service.snapshot(true).expect("selected task snapshot");

    assert_eq!(
        snapshot.restore.workbench_batch.as_ref().unwrap(),
        &source_batch,
        "asset selection must not replace the source search workbench",
    );
    assert_eq!(manifest.calls.lock().unwrap().len(), 1);
    let task = snapshot
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "landsat-selected-asset-task-1")
        .expect("selected asset task");
    assert_eq!(task.query["delivery_kind"], "landsat_c2_l2_asset_selection");
    assert_eq!(task.query["delivery_asset_count"], 2);
    assert_eq!(task.candidates.len(), 2);
    assert_eq!(task.initial_selected_item_ids.len(), 2);
    assert_eq!(
        task.output_root,
        selected_output_root.join("LC09_L2SP_123045_20260101_20260103_02_T1")
    );
    let selected_keys = task
        .candidates
        .iter()
        .map(|item| {
            item.metadata
                .get("delivery_asset_key")
                .and_then(Value::as_str)
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(selected_keys, ["red", "nir08"]);
    assert!(task.candidates.iter().all(|item| item
        .asset_access
        .as_ref()
        .unwrap()
        .expected_size_bytes
        .is_some()));

    service.shutdown().unwrap();
}

#[test]
fn landsat_asset_details_are_read_only_safe_projection_of_the_frozen_microsoft_scene() {
    let database = TempStore::new("landsat-public-asset-details");
    let manifest = Arc::new(RecordingLandsatAssetManifest::default());
    let service = mpc_landsat_asset_service(&database, manifest.clone());
    let source = service
        .catalog_search_and_freeze_legacy(&mpc_landsat_request())
        .expect("freeze Microsoft Landsat scene");
    let source_batch = source.restore.workbench_batch.expect("source batch");
    assert!(source.restore.tasks.is_empty());

    let details = service
        .landsat_asset_details(LandsatAssetDetailsRequest {
            request_id: "landsat-asset-details-1".to_owned(),
            batch_id: source_batch.batch_id.clone(),
            item_id: frozen_mpc_landsat_item_id().to_owned(),
        })
        .expect("safe asset details");

    assert_eq!(
        manifest.calls.lock().unwrap().as_slice(),
        [(
            "landsat-c2-l2".to_owned(),
            "LC09_L2SP_123045_20260101_02_T1".to_owned(),
            "LC09_L2SP_123045_20260101_20260103_02_T1".to_owned(),
        )]
    );
    assert_eq!(details.product_key, frozen_mpc_landsat_item_id());
    assert_eq!(details.processing_level, "l2sp");
    assert_eq!(details.assets.len(), 22);
    assert_eq!(
        details.total_size_bytes,
        details
            .assets
            .iter()
            .map(|asset| asset.size_bytes)
            .sum::<u64>()
    );
    assert_eq!(
        details
            .assets
            .iter()
            .find(|asset| asset.asset_key == "red")
            .map(|asset| asset.role.as_str()),
        Some("primary")
    );
    assert_eq!(
        details
            .assets
            .iter()
            .find(|asset| asset.asset_key == "mtl.xml")
            .map(|asset| asset.role.as_str()),
        Some("metadata")
    );
    let snapshot = service.snapshot(true).expect("read-only snapshot");
    assert_eq!(
        snapshot.restore.workbench_batch.as_ref(),
        Some(&source_batch)
    );
    assert!(snapshot.restore.tasks.is_empty());
    service.shutdown().unwrap();
}

#[test]
fn landsat_asset_details_fail_closed_for_non_microsoft_source_or_manifest_identity_conflict() {
    let legacy_db = TempStore::new("landsat-asset-details-legacy-source");
    let legacy_manifest = Arc::new(RecordingLandsatAssetManifest::default());
    let legacy = landsat_asset_service(&legacy_db, legacy_manifest.clone());
    let source = legacy
        .catalog_search_and_freeze_legacy(&landsat_request())
        .expect("legacy Landsat batch");
    let source_batch = source.restore.workbench_batch.expect("legacy source batch");
    let error = legacy
        .landsat_asset_details(LandsatAssetDetailsRequest {
            request_id: "landsat-asset-details-legacy".to_owned(),
            batch_id: source_batch.batch_id,
            item_id: frozen_landsat_item_id().to_owned(),
        })
        .unwrap_err();
    assert_eq!(error.code(), "LANDSAT_PUBLIC_ASSET_FROZEN_EVIDENCE_INVALID");
    assert!(legacy_manifest.calls.lock().unwrap().is_empty());
    legacy.shutdown().unwrap();

    let conflict_db = TempStore::new("landsat-asset-details-conflict");
    let conflict_manifest = Arc::new(RecordingLandsatAssetManifest {
        calls: Mutex::new(Vec::new()),
        conflict: true,
    });
    let conflict = mpc_landsat_asset_service(&conflict_db, conflict_manifest);
    let source = conflict
        .catalog_search_and_freeze_legacy(&mpc_landsat_request())
        .expect("Microsoft Landsat batch");
    let source_batch = source
        .restore
        .workbench_batch
        .expect("Microsoft source batch");
    let error = conflict
        .landsat_asset_details(LandsatAssetDetailsRequest {
            request_id: "landsat-asset-details-conflict".to_owned(),
            batch_id: source_batch.batch_id,
            item_id: frozen_mpc_landsat_item_id().to_owned(),
        })
        .unwrap_err();
    assert_eq!(error.code(), "LANDSAT_PUBLIC_ASSET_FROZEN_EVIDENCE_INVALID");
    let snapshot = conflict
        .snapshot(true)
        .expect("conflict leaves Store readable");
    assert!(snapshot.restore.tasks.is_empty());
    conflict.shutdown().unwrap();
}

#[test]
fn conflicting_public_landsat_manifest_keeps_source_workbench_and_creates_no_task() {
    let database = TempStore::new("landsat-public-assets-conflict");
    let manifest = Arc::new(RecordingLandsatAssetManifest {
        calls: Mutex::new(Vec::new()),
        conflict: true,
    });
    let service = landsat_asset_service(&database, manifest);
    service
        .catalog_search_and_freeze_legacy(&landsat_request())
        .expect("scene batch");
    let error = service
        .create_landsat_asset_task_legacy(&LandsatCreateBundleTaskRequest {
            request_id: "landsat-public-assets-conflict".to_owned(),
            source_batch_id: "landsat-p4a-2a-batch".to_owned(),
            item_id: frozen_landsat_item_id().to_owned(),
            task_id: "landsat-public-assets-conflict".to_owned(),
            output_root: database.0.parent().unwrap().join("landsat-public-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-29T08:00:00Z".to_owned(),
        })
        .unwrap_err();
    assert_eq!(error.code(), "LANDSAT_PUBLIC_ASSET_FROZEN_EVIDENCE_INVALID");
    let snapshot = service.snapshot(true).unwrap();
    assert_eq!(
        snapshot.restore.workbench_batch.unwrap().batch_id,
        "landsat-p4a-2a-batch"
    );
    assert!(snapshot.restore.tasks.is_empty());
    service.shutdown().unwrap();
}

#[test]
fn landsat_browse_uses_persisted_source_after_workbench_and_task_change() {
    let database = TempStore::new("landsat-browse-historical");
    let browse = Arc::new(RecordingLandsatBrowse::default());
    let service = landsat_browse_service(&database, browse.clone());
    let source_request = landsat_request();
    let source_query = source_request.search.clone();
    service
        .catalog_search_and_freeze_legacy(&source_request)
        .expect("publish the scene-only Landsat source batch");

    let response = service
        .landsat_browse(LandsatBrowseRequest {
            request_id: "landsat-browse-historical-source".to_owned(),
            batch_id: source_request.batch_id.clone(),
            item_id: frozen_landsat_item_id().to_owned(),
        })
        .expect("the exact frozen scene resolves through the injected port");
    assert_eq!(response.media_type, "image/jpeg");
    assert_eq!(response.byte_length, 4);
    assert_eq!(response.data_url, "data:image/jpeg;base64,/9j/2Q==");

    let moved_aoi = json!({
        "type": "Polygon",
        "coordinates": [[[120.0, 35.0], [121.0, 35.0], [121.0, 36.0], [120.0, 35.0]]]
    })
    .as_object()
    .unwrap()
    .clone();
    service
        .set_working_aoi(&ApplicationSetWorkingAoiRequest {
            source_kind: "manual_bbox".to_owned(),
            geometry: moved_aoi.clone(),
            bbox: vec![120.0, 35.0, 121.0, 36.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "Moved working AOI".to_owned(),
        })
        .expect("the mutable working AOI can change independently");

    let mut active_request = landsat_request();
    active_request.request_id = "landsat-browse-current-freeze".to_owned();
    active_request.batch_id = "landsat-p4a-2b-batch".to_owned();
    active_request.created_at = "2026-07-28T00:00:01Z".to_owned();
    active_request
        .search
        .filters
        .insert("cloud_cover_lte".to_owned(), json!(1.0));
    service
        .catalog_search_and_freeze_legacy(&active_request)
        .expect("publish a second batch and make it the active Workbench batch");

    let historical_response = service
        .landsat_browse(LandsatBrowseRequest {
            request_id: "landsat-browse-after-workbench-switch".to_owned(),
            batch_id: source_request.batch_id.clone(),
            item_id: frozen_landsat_item_id().to_owned(),
        })
        .expect("historical browse remains bound to the persisted source batch");
    assert_eq!(historical_response.byte_length, 4);
    {
        let calls = browse.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, source_query);
        assert_eq!(calls[1].0, source_query);
        assert_eq!(calls[0].1, "LC91230452026001LGN00");
        assert_eq!(calls[1].1, "LC91230452026001LGN00");
    }

    service.shutdown().unwrap();
    let mut store = TaskStore::open(&database.0).expect("reopen historical store");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "landsat-browse-terminal-task".to_owned(),
            batch_id: source_request.batch_id.clone(),
            selected_item_ids: vec![frozen_landsat_item_id().to_owned()],
            checked_item_ids: vec![frozen_landsat_item_id().to_owned()],
            page_index: 0,
            page_size: 25,
            output_root: std::env::temp_dir().join("landsat-browse-terminal-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-28T00:00:02Z".to_owned(),
        })
        .expect("create a historical task tied to the source batch");
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "landsat-browse-terminal-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec![frozen_landsat_item_id().to_owned()],
            },
            event_id: "landsat-browse-terminal-task-complete".to_owned(),
            occurred_at: "2026-07-28T00:00:03Z".to_owned(),
        })
        .expect("mark the source task terminal without changing its source batch");
    let terminal = store
        .load_task_record("landsat-browse-terminal-task")
        .expect("read terminal task")
        .expect("terminal task exists");
    assert_eq!(
        terminal.runtime.status,
        insar_core::TaskLifecycleStatus::Completed
    );
    drop(store);

    let reopened = landsat_browse_service(&database, browse.clone());
    let terminal_response = reopened
        .landsat_browse(LandsatBrowseRequest {
            request_id: "landsat-browse-after-terminal-task".to_owned(),
            batch_id: source_request.batch_id,
            item_id: frozen_landsat_item_id().to_owned(),
        })
        .expect("terminal task history still resolves its immutable source browse");
    assert_eq!(terminal_response.byte_length, 4);
    assert_eq!(browse.calls.lock().unwrap().len(), 3);

    for (request_id, batch_id, item_id) in [
        (
            "landsat-browse-wrong-batch",
            "another-batch",
            frozen_landsat_item_id(),
        ),
        (
            "landsat-browse-wrong-item",
            "landsat-p4a-2a-batch",
            "PROVIDER::usgs.eros.m2m::COLLECTION::landsat_ot_c2_l2::ITEM::UNKNOWN",
        ),
        (
            "landsat-browse-wrong-provider",
            "landsat-p4a-2a-batch",
            "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::UNKNOWN",
        ),
    ] {
        let error = reopened
            .landsat_browse(LandsatBrowseRequest {
                request_id: request_id.to_owned(),
                batch_id: batch_id.to_owned(),
                item_id: item_id.to_owned(),
            })
            .err()
            .expect("unknown or cross-provider source evidence must fail closed");
        assert_eq!(error.code(), "USGS_LANDSAT_BROWSE_FROZEN_EVIDENCE_INVALID");
    }
    assert_eq!(browse.calls.lock().unwrap().len(), 3);
    reopened.shutdown().unwrap();
}

#[test]
fn landsat_browse_unavailable_never_invokes_the_provider_port() {
    let database = TempStore::new("landsat-browse-unavailable");
    let browse = Arc::new(RecordingLandsatBrowse::default());
    let mut response = landsat_response();
    response.collection.production_enabled = true;
    response.products[0]
        .properties
        .insert("browse_available".to_owned(), Value::Bool(false));
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_browse(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog { response }),
        browse.clone(),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "landsat-browse-unavailable");
    service
        .catalog_search_and_freeze_legacy(&landsat_request())
        .expect("publish the browse-unavailable scene");

    let error = service
        .landsat_browse(LandsatBrowseRequest {
            request_id: "landsat-browse-unavailable".to_owned(),
            batch_id: "landsat-p4a-2a-batch".to_owned(),
            item_id: frozen_landsat_item_id().to_owned(),
        })
        .err()
        .expect("the frozen unavailable state is terminal");
    assert_eq!(error.code(), "USGS_LANDSAT_BROWSE_UNAVAILABLE");
    assert!(browse.calls.lock().unwrap().is_empty());
    service.shutdown().unwrap();
}

#[test]
fn landsat_browse_duplicate_and_cancellation_share_one_request_generation() {
    let database = TempStore::new("landsat-browse-cancel");
    let browse = Arc::new(BlockingLandsatBrowse {
        entered: AtomicBool::new(false),
    });
    let service = Arc::new(landsat_browse_service(&database, browse.clone()));
    service
        .catalog_search_and_freeze_legacy(&landsat_request())
        .expect("publish the scene before preview starts");

    let worker_service = Arc::clone(&service);
    let worker = thread::spawn(move || {
        worker_service.landsat_browse(LandsatBrowseRequest {
            request_id: "landsat-browse-shared-generation".to_owned(),
            batch_id: "landsat-p4a-2a-batch".to_owned(),
            item_id: frozen_landsat_item_id().to_owned(),
        })
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !browse.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(browse.entered.load(Ordering::Acquire));

    let duplicate = service
        .landsat_browse(LandsatBrowseRequest {
            request_id: "landsat-browse-shared-generation".to_owned(),
            batch_id: "landsat-p4a-2a-batch".to_owned(),
            item_id: frozen_landsat_item_id().to_owned(),
        })
        .err()
        .expect("an active request ID cannot be overwritten");
    assert_eq!(duplicate.code(), "USGS_LANDSAT_BROWSE_REQUEST_DUPLICATE");
    assert!(service
        .cancel_request("landsat-browse-shared-generation")
        .unwrap());
    let cancelled = worker
        .join()
        .unwrap()
        .err()
        .expect("the same request generation observes cancellation");
    assert_eq!(cancelled.code(), "USGS_M2M_CANCELLED");
    service.shutdown().unwrap();
}

#[test]
fn landsat_scene_only_batch_restores_after_store_reopen_without_tasks_or_assets() {
    let database = TempStore::new("landsat-restart-restore");
    let request = landsat_request();
    landsat_response()
        .validate()
        .expect("filtered Landsat diagnostics remain contract-valid");
    let service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: landsat_response(),
        }),
    );
    let snapshot = service.catalog_search_and_freeze_legacy(&request).unwrap();
    let batch = snapshot
        .restore
        .workbench_batch
        .expect("Landsat SearchBatch is published");
    assert_eq!(batch.provider_id, "usgs.eros.m2m");
    assert_eq!(batch.aoi_geojson, request.aoi_geojson);
    assert_eq!(batch.candidates.len(), 1);
    let candidate = &batch.candidates[0];
    assert_eq!(
        candidate.item_id,
        "PROVIDER::usgs.eros.m2m::COLLECTION::landsat_ot_c2_l2::ITEM::LC91230452026001LGN00"
    );
    assert_eq!(candidate.metadata["source_kind"], "landsat");
    assert_eq!(
        candidate.metadata["properties"]["entity_id"],
        "LC91230452026001LGN00"
    );
    assert_eq!(
        candidate.metadata["properties"]["landsat_product_id"],
        "LC09_L2SP_123045_20260101_20260103_02_T1"
    );
    assert_eq!(
        candidate.metadata["properties"]["landsat_scene_id"],
        "LC91230452026001LGN00"
    );
    assert_eq!(candidate.metadata["properties"]["wrs_path"], "123");
    assert_eq!(candidate.metadata["properties"]["wrs_row"], "045");
    assert_eq!(
        candidate.acquisition_datetime.as_deref(),
        Some("2026-01-01T00:00:00Z")
    );
    assert_eq!(
        candidate.metadata["properties"]["acquisition_date"],
        "2026/01/01"
    );
    assert_eq!(
        candidate.metadata["properties"]["acquisition_precision"],
        "day"
    );
    assert_eq!(candidate.metadata["properties"]["browse_available"], true);
    assert_eq!(candidate.metadata["assets"], json!([]));
    assert!(candidate.payload.is_empty());
    assert!(candidate.asset_access.is_none());
    assert!(candidate.execution_policy.is_none());
    assert!(snapshot.restore.tasks.is_empty());
    let frozen_json = serde_json::to_string(&batch).unwrap();
    for forbidden in ["https://", "http://", "download", "resolver_id"] {
        assert!(!frozen_json.to_ascii_lowercase().contains(forbidden));
    }
    service.shutdown().unwrap();

    let reopened = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: landsat_response(),
        }),
    );
    let restored = reopened.snapshot(true).unwrap();
    let restored_batch = restored
        .restore
        .workbench_batch
        .expect("the same Landsat SearchBatch restores after restart");
    assert_eq!(serde_json::to_string(&restored_batch).unwrap(), frozen_json);
    assert_eq!(
        restored.restore.workbench.unwrap().batch_id,
        "landsat-p4a-2a-batch"
    );
    assert!(restored.restore.tasks.is_empty());
    reopened.shutdown().unwrap();
}

#[test]
fn landsat_live_accepted_production_runtime_is_dataset_closed() {
    let database = TempStore::new("landsat-live-accepted-production");
    let mut response = landsat_response();
    response.collection.production_enabled = true;
    let service = production_fixture_service(&database, Arc::new(StaticCatalog { response }));

    let frozen = service
        .catalog_search_and_freeze_legacy(&landsat_request())
        .expect("the closed candidate traverses production validation and Store publication");
    assert_eq!(frozen.restore.workbench_batch.unwrap().candidates.len(), 1);
    assert!(frozen.restore.tasks.is_empty());
    service.shutdown().unwrap();

    let wrong_database = TempStore::new("landsat-live-accepted-wrong-dataset");
    let wrong = production_fixture_service(
        &wrong_database,
        Arc::new(StaticCatalog {
            response: production_response(),
        }),
    );
    let mut wrong_request = landsat_request();
    wrong_request.request_id = "live-accepted-wrong-dataset".to_owned();
    wrong_request.batch_id = "live-accepted-wrong-dataset-batch".to_owned();
    assert!(wrong
        .catalog_search_and_freeze_legacy(&wrong_request)
        .is_err());
    assert_empty_store(&wrong);
    wrong.shutdown().unwrap();
}

#[test]
fn landsat_scene_freeze_accepts_timestamp_and_rejects_ambiguous_temporal_metadata() {
    let database = TempStore::new("landsat-timestamp-freeze");
    let response =
        landsat_temporal_response("2026-01-01T10:15:30.1675Z", "2026/01/01", "timestamp");
    let service = fixture_service(&database, Arc::new(StaticCatalog { response }));
    let mut request = landsat_request();
    request.request_id = "landsat-timestamp-freeze-request".to_owned();
    request.batch_id = "landsat-timestamp-freeze-batch".to_owned();
    let snapshot = service
        .catalog_search_and_freeze_legacy(&request)
        .expect("a valid Landsat RFC3339 timestamp with provider fractional precision remains authoritative");
    let candidate = &snapshot.restore.workbench_batch.unwrap().candidates[0];
    assert_eq!(
        candidate.acquisition_datetime.as_deref(),
        Some("2026-01-01T10:15:30.167500Z")
    );
    assert_eq!(
        candidate.metadata["properties"]["acquisition_date"],
        "2026/01/01"
    );
    assert_eq!(
        candidate.metadata["properties"]["acquisition_precision"],
        "timestamp"
    );
    service.shutdown().unwrap();

    assert_landsat_response_rejected(
        "landsat-day-non-midnight",
        landsat_temporal_response("2026-01-01T10:15:30Z", "2026/01/01", "day"),
    );
    assert_landsat_response_rejected(
        "landsat-unknown-precision",
        landsat_temporal_response("2026-01-01T10:15:30Z", "2026/01/01", "instant"),
    );
    assert_landsat_response_rejected(
        "landsat-conflicting-date",
        landsat_temporal_response("2026-01-01T10:15:30Z", "2025/12/31", "timestamp"),
    );
}

#[test]
fn loopback_two_pages_freeze_three_non_executable_candidates_atomically() {
    let first = include_str!(
        "../../../tests/fixtures/catalog/cdse/sentinel2_l2a/odata_collection_page_1.json"
    )
    .replace("$top=2", "$top=100");
    let second = include_str!(
        "../../../tests/fixtures/catalog/cdse/sentinel2_l2a/odata_collection_page_2.json"
    )
    .to_owned();
    let server = LoopbackServer::two_pages(first, second);
    let database = TempStore::new("loopback-freeze");
    let catalogue =
        Arc::new(LoopbackODataClient::new(&server.endpoint, Duration::from_secs(2)).unwrap());
    let service = fixture_service(&database, catalogue);
    let mut operation = request("loopback-freeze", "catalogue-three-items");
    operation.search.result_limit = None;

    let snapshot = service
        .catalog_search_and_freeze_legacy(&operation)
        .unwrap();
    let batch = snapshot.restore.workbench_batch.unwrap();
    assert_eq!(batch.candidates.len(), 3);
    assert_eq!(batch.aoi_geojson, operation.aoi_geojson);
    assert_eq!(
        batch.query,
        serde_json::to_value(&operation.search)
            .unwrap()
            .as_object()
            .unwrap()
            .clone()
    );
    assert_eq!(batch.candidates[0].item_id, "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::00000000-0000-4000-8000-000000000001");
    assert!(batch.candidates.iter().all(|candidate| {
        candidate.payload.is_empty()
            && candidate.asset_access.is_none()
            && candidate.execution_policy.is_none()
            && !candidate.metadata.contains_key("url")
            && !candidate.metadata.contains_key("access_descriptor")
            && !candidate.metadata.contains_key("file_name")
    }));
    let serialized = serde_json::to_string(&batch).unwrap().to_ascii_lowercase();
    for forbidden in [
        "token",
        "cookie",
        "authorization",
        "credential",
        "http://",
        "https://",
        "c:\\\\users",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    let workbench = snapshot.restore.workbench.unwrap();
    assert!(workbench.checked_item_ids.is_empty());
    assert_eq!(workbench.page_index, 0);
    assert_eq!(workbench.page_size, 24);
    assert_eq!(workbench.revision, 0);
    service.shutdown().unwrap();
}

#[test]
fn microsoft_sentinel2_freezes_search_only_source_without_cdse_archive_authority() {
    let database = TempStore::new("mpc-sentinel2-search-only-freeze");
    let mut response = valid_response();
    response.collection = serde_json::from_value(json!({
        "logical_dataset":"optical.sentinel-2.l2a",
        "provider_id":"microsoft.planetary-computer",
        "native_collection_id":"sentinel-2-l2a",
        "collection_profile_id":"planetary-computer.sentinel-2-l2a.scene.v1",
        "identity_rule_id":"planetary-computer.sentinel-2-l2a.identity.v1",
        "filename_rule_id":"planetary-computer.sentinel-2-l2a.product-uri.v1",
        "delivery_model":"archive_first",
        "production_enabled":true
    }))
    .unwrap();
    response.products = vec![serde_json::from_value(json!({
        "provider_id":"microsoft.planetary-computer",
        "product_id":"S2A_MSIL2A_20260101T100000_R046_T51RUH_20260101T120000",
        "display_name":"S2A_MSIL2A_20260101T100000_N0511_R046_T51RUH_20260101T120000.SAFE",
        "source_kind":"sentinel2",
        "product_kind":"optical_scene",
        "acquisition_datetime":"2026-01-01T10:00:00Z",
        "footprint_geojson":{"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},
        "assets":[],
        "properties":{"product_type":"S2MSI2A","cloud_cover":4.2},
        "payload":{}
    }))
    .unwrap()];
    // Microsoft Planetary Computer STAC does not guarantee numberMatched.
    // A bounded working set with no provider/item errors is still complete
    // enough to freeze; the application must not invent a remote total.
    // Spatial fallback may see the same scene at adjacent request-tile
    // boundaries, so raw duplicate accounting is truthful while persistence
    // still freezes the provider-scoped product only once.
    response.stats.matched_count = None;
    response.stats.remote_exhausted = false;
    response.stats.working_set_satisfied = true;
    response.stats.requested_limit = Some(1);
    response.stats.fetched_record_count = 2;
    response.stats.mapped_record_count = 2;
    response.stats.exact_duplicate_record_count = 1;
    response.stats.unique_provider_product_count = 1;
    response.stats.exact_matched_count = 1;
    response.stats.returned_count = 1;

    let service = fixture_service(&database, Arc::new(StaticCatalog { response }));
    let mut operation = request(
        "mpc-sentinel2-search-only",
        "mpc-sentinel2-search-only-batch",
    );
    operation.search.provider_id = "microsoft.planetary-computer".to_owned();
    operation.search.native_collection_id = "sentinel-2-l2a".to_owned();
    operation.search.result_limit = Some(1);
    let snapshot = service
        .catalog_search_and_freeze_legacy(&operation)
        .expect("reviewed Microsoft Sentinel-2 search-only result freezes");
    let batch = snapshot.restore.workbench_batch.unwrap();
    assert_eq!(batch.query["provider_id"], "microsoft.planetary-computer");
    assert_eq!(batch.candidates.len(), 1);
    let candidate = &batch.candidates[0];
    assert_eq!(candidate.provider_id, "microsoft.planetary-computer");
    assert_eq!(candidate.metadata["native_collection_id"], "sentinel-2-l2a");
    assert_eq!(
        candidate.metadata["resolver_policy_id"],
        "planetary-computer.sentinel-2-l2a.search-only.v1"
    );
    assert_eq!(candidate.metadata["assets"], json!([]));
    assert!(candidate.payload.is_empty());
    assert!(candidate.asset_access.is_none());
    assert!(candidate.execution_policy.is_none());
    service.shutdown().unwrap();
}

#[test]
fn microsoft_sentinel2_analysis_band_task_freezes_only_selected_cogs_without_mutating_source_workbench(
) {
    const ITEM_ID: &str = "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916";
    const SAFE_NAME: &str = "S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE";
    const B02_HREF: &str = "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R10m/T48SYC_20260706T032511_B02_10m.tif";
    const B11_HREF: &str = "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R20m/T48SYC_20260706T032511_B11_20m.tif";

    let database = TempStore::new("mpc-sentinel2-band-delivery");
    let mut response = valid_response();
    response.collection = serde_json::from_value(json!({
        "logical_dataset":"optical.sentinel-2.l2a",
        "provider_id":"microsoft.planetary-computer",
        "native_collection_id":"sentinel-2-l2a",
        "collection_profile_id":"planetary-computer.sentinel-2-l2a.scene.v1",
        "identity_rule_id":"planetary-computer.sentinel-2-l2a.identity.v1",
        "filename_rule_id":"planetary-computer.sentinel-2-l2a.product-uri.v1",
        "delivery_model":"archive_first",
        "production_enabled":true
    }))
    .unwrap();
    response.products = vec![serde_json::from_value(json!({
        "provider_id":"microsoft.planetary-computer",
        "product_id":ITEM_ID,
        "display_name":SAFE_NAME,
        "source_kind":"sentinel2",
        "product_kind":"optical_scene",
        "acquisition_datetime":"2026-07-06T03:25:11Z",
        "footprint_geojson":{"type":"Polygon","coordinates":[[[108,31],[109,31],[109,32],[108,31]]]},
        "assets":[],
        "properties":{"product_type":"S2MSI2A","cloud_cover":4.2},
        "payload":{}
    }))
    .unwrap()];
    response.stats.matched_count = None;
    response.stats.remote_exhausted = false;
    response.stats.working_set_satisfied = true;
    response.stats.requested_limit = Some(1);
    response.stats.fetched_record_count = 1;
    response.stats.mapped_record_count = 1;
    response.stats.unique_provider_product_count = 1;
    response.stats.exact_matched_count = 1;
    response.stats.returned_count = 1;

    let worker = Arc::new(RecordingWorker::default());
    let service = ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog { response }),
        worker.clone(),
    );
    bind_aoi(&service, "mpc-sentinel2-band-delivery");
    let mut source_request = request(
        "mpc-s2-band-delivery-source",
        "mpc-s2-band-delivery-source-batch",
    );
    source_request.search.provider_id = "microsoft.planetary-computer".to_owned();
    source_request.search.native_collection_id = "sentinel-2-l2a".to_owned();
    source_request.search.result_limit = Some(1);
    let source = service
        .catalog_search_and_freeze_legacy(&source_request)
        .expect("freeze Microsoft Sentinel-2 search-only source");
    let source_batch = source.restore.workbench_batch.unwrap();
    let source_item = source_batch.candidates[0].clone();

    let evidence = StaticSentinel2BandEvidence {
        manifest: MpcSentinel2BandManifest {
            item_id: ITEM_ID.to_owned(),
            safe_name: SAFE_NAME.to_owned(),
            assets: vec![
                MpcSentinel2BandAsset {
                    asset_key: "B02".to_owned(),
                    stable_href: B02_HREF.to_owned(),
                    file_name: "T48SYC_20260706T032511_B02_10m.tif".to_owned(),
                    resolution_m: 10,
                },
                MpcSentinel2BandAsset {
                    asset_key: "B11".to_owned(),
                    stable_href: B11_HREF.to_owned(),
                    file_name: "T48SYC_20260706T032511_B11_20m.tif".to_owned(),
                    resolution_m: 20,
                },
            ],
        },
        sizes: BTreeMap::from([
            ("B02".to_owned(), 118_000_001),
            ("B11".to_owned(), 31_000_002),
        ]),
        manifest_calls: Arc::new(Mutex::new(Vec::new())),
        probe_calls: Arc::new(Mutex::new(Vec::new())),
    };
    let selected_output_root = database.0.parent().unwrap().join("mpc-s2-band-output");
    let created = service
        .create_sentinel2_band_task_with_evidence_for_test(
            &Sentinel2CreateBandTaskRequest {
                request_id: "mpc-s2-band-delivery-1".to_owned(),
                source_batch_id: source_batch.batch_id.clone(),
                item_id: source_item.item_id.clone(),
                asset_keys: vec!["B02".to_owned(), "B11".to_owned()],
                task_id: "mpc-s2-band-task-1".to_owned(),
                output_root: selected_output_root.clone(),
                layout: DownloadLayout::Direct,
                concurrency: 2,
                created_at: "2026-09-21T08:00:00Z".to_owned(),
            },
            &evidence,
        )
        .expect("selected Sentinel-2 analysis bands freeze");

    assert_eq!(
        evidence.manifest_calls.lock().unwrap().as_slice(),
        [(
            SAFE_NAME.to_owned(),
            Some("2026-07-06T03:25:11Z".to_owned()),
            vec!["B02".to_owned(), "B11".to_owned()],
        )]
    );
    assert_eq!(evidence.probe_calls.lock().unwrap().len(), 2);
    let snapshot = service.snapshot(true).expect("band task snapshot");
    assert_eq!(
        snapshot.restore.workbench_batch.as_ref().unwrap(),
        &source_batch,
        "analysis-band delivery must not replace the search workbench",
    );
    let task = snapshot
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "mpc-s2-band-task-1")
        .expect("Sentinel-2 band task");
    assert_eq!(
        task.query["delivery_kind"],
        "sentinel2_l2a_analysis_band_set"
    );
    assert_eq!(task.query["delivery_safe_name"], SAFE_NAME);
    assert_eq!(task.query["delivery_asset_count"], 2);
    assert_eq!(task.candidates.len(), 2);
    assert_eq!(task.initial_selected_item_ids.len(), 2);
    assert_eq!(task.output_root, selected_output_root.join(ITEM_ID));
    assert_eq!(task.provider_id, "microsoft.planetary-computer");
    let selected_keys = task
        .candidates
        .iter()
        .map(|item| item.metadata["delivery_asset_key"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(selected_keys, ["B02", "B11"]);
    for item in &task.candidates {
        let access = item.asset_access.as_ref().expect("band access");
        assert_eq!(
            access.descriptor.resolver_id,
            "planetary-computer.sentinel-2-l2a.band-cog.v1"
        );
        assert_eq!(
            item.execution_policy.as_ref().unwrap().policy_id,
            "planetary-computer.sentinel-2-l2a.band-cog.asset.v1"
        );
        assert!(!access.descriptor.requires_credentials);
        assert!(access.expected_size_bytes.is_some_and(|size| size > 0));
        assert!(access
            .descriptor
            .stable_href
            .as_deref()
            .is_some_and(|href| href
                .starts_with("https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/")));
        assert!(!access
            .descriptor
            .stable_href
            .as_deref()
            .unwrap()
            .contains('?'));
    }
    let persisted = serde_json::to_string(task).unwrap().to_ascii_lowercase();
    for forbidden in ["sig=", "token=", "authorization", "requestsignature"] {
        assert!(!persisted.contains(forbidden), "{forbidden}");
    }
    assert_eq!(
        created
            .core
            .tasks
            .iter()
            .find(|task| task.task_id == "mpc-s2-band-task-1")
            .unwrap()
            .selected_count,
        2
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if worker.items.lock().unwrap().len() == 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "band task worker did not receive two items"
        );
        thread::sleep(Duration::from_millis(10));
    }
    service.shutdown().unwrap();
}

#[test]
fn cdse_sentinel2_scene_can_derive_mpc_analysis_band_task_without_changing_safe_authority() {
    const ITEM_ID: &str = "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916";
    const SAFE_NAME: &str = "S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE";
    const B08_HREF: &str = "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R10m/T48SYC_20260706T032511_B08_10m.tif";

    let database = TempStore::new("cdse-sentinel2-band-delivery");
    let mut product = exact_archive_product("4b29ea58-874a-45d5-ae3e-0ab714cbafcc");
    product.display_name = SAFE_NAME.to_owned();
    product.acquisition_datetime = Some("2026-07-06T03:25:11Z".to_owned());
    product.footprint_geojson = Some(
        json!({"type":"Polygon","coordinates":[[[108,31],[109,31],[109,32],[108,31]]]})
            .as_object()
            .unwrap()
            .clone(),
    );
    product
        .properties
        .insert("product_type".to_owned(), json!("S2MSI2A"));
    product.properties.insert("online".to_owned(), json!(true));
    product.properties.insert(
        "checksums".to_owned(),
        json!([{"algorithm":"MD5","value":"00000000000000000000000000000000"}]),
    );
    let mut response = complete_response(vec![product]);
    response.collection.production_enabled = true;
    response.stats.requested_limit = Some(2);
    response.stats.matched_count = Some(1);
    response.stats.remote_exhausted = true;
    response.stats.working_set_satisfied = true;

    let worker = Arc::new(RecordingWorker::default());
    let service = ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog { response }),
        worker,
    );
    bind_aoi(&service, "cdse-sentinel2-band-delivery");
    let source = service
        .catalog_search_and_freeze_legacy(&request(
            "cdse-s2-band-delivery-source",
            "cdse-s2-band-delivery-source-batch",
        ))
        .expect("freeze CDSE Sentinel-2 source");
    let source_batch = source.restore.workbench_batch.unwrap();
    let source_item = source_batch.candidates[0].clone();
    assert_eq!(source_item.provider_id, "copernicus.dataspace.odata");
    assert!(source_item.asset_access.is_some());
    assert!(source_item.execution_policy.is_some());

    let evidence = StaticSentinel2BandEvidence {
        manifest: MpcSentinel2BandManifest {
            item_id: ITEM_ID.to_owned(),
            safe_name: SAFE_NAME.to_owned(),
            assets: vec![MpcSentinel2BandAsset {
                asset_key: "B08".to_owned(),
                stable_href: B08_HREF.to_owned(),
                file_name: "T48SYC_20260706T032511_B08_10m.tif".to_owned(),
                resolution_m: 10,
            }],
        },
        sizes: BTreeMap::from([("B08".to_owned(), 119_000_003)]),
        manifest_calls: Arc::new(Mutex::new(Vec::new())),
        probe_calls: Arc::new(Mutex::new(Vec::new())),
    };
    let selected_output_root = database.0.parent().unwrap().join("cdse-s2-band-output");
    service
        .create_sentinel2_band_task_with_evidence_for_test(
            &Sentinel2CreateBandTaskRequest {
                request_id: "cdse-s2-band-delivery-1".to_owned(),
                source_batch_id: source_batch.batch_id.clone(),
                item_id: source_item.item_id.clone(),
                asset_keys: vec!["B08".to_owned()],
                task_id: "cdse-s2-band-task-1".to_owned(),
                output_root: selected_output_root.clone(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-09-21T08:05:00Z".to_owned(),
            },
            &evidence,
        )
        .expect("CDSE scene derives one MPC analysis band");

    let snapshot = service.snapshot(true).expect("CDSE band task snapshot");
    assert_eq!(
        snapshot.restore.workbench_batch.as_ref().unwrap(),
        &source_batch
    );
    let task = snapshot
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "cdse-s2-band-task-1")
        .expect("CDSE-derived band task");
    assert_eq!(task.provider_id, "microsoft.planetary-computer");
    assert_eq!(task.query["derived_from_batch_id"], source_batch.batch_id);
    assert_eq!(task.query["delivery_safe_name"], SAFE_NAME);
    assert_eq!(task.candidates.len(), 1);
    assert_eq!(task.output_root, selected_output_root.join(ITEM_ID));
    let band = &task.candidates[0];
    assert_eq!(band.metadata["delivery_asset_key"], "B08");
    assert_eq!(band.provider_id, "microsoft.planetary-computer");
    assert_eq!(
        band.asset_access.as_ref().unwrap().descriptor.resolver_id,
        "planetary-computer.sentinel-2-l2a.band-cog.v1"
    );
    assert_eq!(
        band.execution_policy.as_ref().unwrap().policy_id,
        "planetary-computer.sentinel-2-l2a.band-cog.asset.v1"
    );
    assert_eq!(evidence.manifest_calls.lock().unwrap().len(), 1);
    assert_eq!(evidence.probe_calls.lock().unwrap().len(), 1);
    service.shutdown().unwrap();
}

fn seed_microsoft_sentinel2_archive_source(
    database: &TempStore,
) -> (insar_core::FrozenSearchBatch, SnapshotItem) {
    let mut source_response = valid_response();
    source_response.collection = serde_json::from_value(json!({
        "logical_dataset":"optical.sentinel-2.l2a",
        "provider_id":"microsoft.planetary-computer",
        "native_collection_id":"sentinel-2-l2a",
        "collection_profile_id":"planetary-computer.sentinel-2-l2a.scene.v1",
        "identity_rule_id":"planetary-computer.sentinel-2-l2a.identity.v1",
        "filename_rule_id":"planetary-computer.sentinel-2-l2a.product-uri.v1",
        "delivery_model":"archive_first",
        "production_enabled":true
    }))
    .unwrap();
    source_response.products = vec![serde_json::from_value(json!({
        "provider_id":"microsoft.planetary-computer",
        "product_id":"S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916",
        "display_name":"S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE",
        "source_kind":"sentinel2",
        "product_kind":"optical_scene",
        "acquisition_datetime":"2026-07-06T03:25:11Z",
        "footprint_geojson":{"type":"Polygon","coordinates":[[[108,31],[109,31],[109,32],[108,31]]]},
        "assets":[],
        "properties":{"product_type":"S2MSI2A","cloud_cover":4.2},
        "payload":{}
    }))
    .unwrap()];
    source_response.stats.matched_count = None;
    source_response.stats.remote_exhausted = false;
    source_response.stats.working_set_satisfied = true;
    source_response.stats.requested_limit = Some(1);
    source_response.stats.fetched_record_count = 1;
    source_response.stats.mapped_record_count = 1;
    source_response.stats.unique_provider_product_count = 1;
    source_response.stats.exact_matched_count = 1;
    source_response.stats.returned_count = 1;

    let source_service = fixture_service(
        database,
        Arc::new(StaticCatalog {
            response: source_response,
        }),
    );
    let mut source_request = request("mpc-s2-delivery-source", "mpc-s2-delivery-source-batch");
    source_request.search.provider_id = "microsoft.planetary-computer".to_owned();
    source_request.search.native_collection_id = "sentinel-2-l2a".to_owned();
    source_request.search.result_limit = Some(1);
    let source_snapshot = source_service
        .catalog_search_and_freeze_legacy(&source_request)
        .expect("Microsoft search-only batch freezes");
    let source_batch = source_snapshot.restore.workbench_batch.unwrap();
    let source_item = source_batch.candidates[0].clone();
    assert!(source_item.asset_access.is_none());
    assert!(source_item.execution_policy.is_none());
    source_service.shutdown().unwrap();

    (source_batch, source_item)
}

fn seed_two_microsoft_sentinel2_archive_sources(
    database: &TempStore,
) -> (insar_core::FrozenSearchBatch, Vec<SnapshotItem>) {
    let (mut source_batch, first) = seed_microsoft_sentinel2_archive_source(database);
    let second_product_id = "S2B_MSIL2A_20260918T033531_R061_T48SXB_20260918T141517";
    let second_safe_name = "S2B_MSIL2A_20260918T033531_N0512_R061_T48SXB_20260918T141517.SAFE";
    let mut second = first.clone();
    second.item_id = provider_product_key(
        "microsoft.planetary-computer",
        "sentinel-2-l2a",
        second_product_id,
    )
    .unwrap();
    second.display_name = second_safe_name.to_owned();
    second.acquisition_datetime = Some("2026-09-18T03:35:31.025Z".to_owned());
    second.footprint_geojson = json!({
        "type":"Polygon",
        "coordinates":[[[109,31],[110,31],[110,32],[109,31]]]
    })
    .as_object()
    .cloned();
    second.metadata.insert(
        "provider_product_id".to_owned(),
        Value::String(second_product_id.to_owned()),
    );
    second.metadata.insert(
        "provider_product_key".to_owned(),
        Value::String(second.item_id.clone()),
    );
    second.metadata.insert(
        "stac_item_id".to_owned(),
        Value::String(second_product_id.to_owned()),
    );
    second.validate().unwrap();

    source_batch.batch_id = "mpc-s2-delivery-source-batch-multi".to_owned();
    source_batch.search_stats = None;
    source_batch.candidates = vec![first.clone(), second.clone()];
    source_batch.validate().unwrap();
    let mut store = TaskStore::open(&database.0).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: source_batch.clone(),
            workbench: SearchWorkbenchState {
                batch_id: source_batch.batch_id.clone(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: 100,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    (source_batch, vec![first, second])
}

#[test]
fn microsoft_sentinel2_download_re_resolves_exact_safe_to_cdse_without_mutating_source_workbench() {
    let database = TempStore::new("mpc-sentinel2-cdse-delivery");
    let (source_batch, source_item) = seed_microsoft_sentinel2_archive_source(&database);

    let mut delivery_product = exact_archive_product("4b29ea58-874a-45d5-ae3e-0ab714cbafcc");
    delivery_product.display_name = source_item.display_name.clone();
    delivery_product.acquisition_datetime = source_item.acquisition_datetime.clone();
    delivery_product.footprint_geojson = source_item.footprint_geojson.clone();
    delivery_product
        .properties
        .insert("product_type".to_owned(), json!("S2MSI2A"));
    delivery_product
        .properties
        .insert("online".to_owned(), json!(true));
    delivery_product.properties.insert(
        "checksums".to_owned(),
        json!([{"algorithm":"MD5","value":"00000000000000000000000000000000"}]),
    );
    let mut delivery_response = complete_response(vec![delivery_product]);
    delivery_response.collection.production_enabled = true;
    delivery_response.stats.requested_limit = Some(100);
    delivery_response.stats.matched_count = Some(1);
    delivery_response.stats.remote_exhausted = true;
    delivery_response.stats.working_set_satisfied = true;

    let delivery_worker = Arc::new(RecordingWorker::default());
    let service = ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: delivery_response,
        }),
        delivery_worker,
    );
    bind_aoi(&service, "mpc-sentinel2-cdse-delivery");
    let created = service
        .create_sentinel2_archive_task(&Sentinel2CreateArchiveTaskRequest {
            request_id: "mpc-s2-cdse-delivery".to_owned(),
            source_batch_id: source_batch.batch_id.clone(),
            item_id: source_item.item_id.clone(),
            task_id: "mpc-s2-cdse-task".to_owned(),
            output_root: database.0.parent().unwrap().join("mpc-s2-cdse-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-09-18T00:00:00Z".to_owned(),
        })
        .expect("exact Microsoft SAFE identity re-resolves to CDSE");

    let full = service.snapshot(true).unwrap();
    let active_batch = full.restore.workbench_batch.as_ref().unwrap();
    assert_eq!(active_batch, &source_batch);
    let derived = service
        .task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: "mpc-s2-cdse-task".to_owned(),
            page_index: 0,
            page_size: 24,
        })
        .expect("task owns a derived CDSE source batch");
    assert_ne!(derived.source_batch_id, source_batch.batch_id);
    assert_eq!(derived.total_count, 1);
    let delivery = &derived.items[0].item;
    assert_eq!(delivery.provider_id, "copernicus.dataspace.odata");
    assert_eq!(delivery.display_name, source_item.display_name);
    assert!(delivery.asset_access.is_some());
    assert!(delivery.execution_policy.is_some());
    assert_eq!(
        delivery
            .asset_access
            .as_ref()
            .unwrap()
            .descriptor
            .credential_profile
            .as_deref(),
        Some("copernicus-dataspace-default")
    );
    let task = created
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == "mpc-s2-cdse-task")
        .unwrap();
    assert_eq!(task.batch_id, derived.source_batch_id);
    assert_eq!(task.selected_count, 1);

    let persisted = serde_json::to_string(&full.restore)
        .unwrap()
        .to_ascii_lowercase();
    for forbidden in [
        "authorization",
        "token=",
        "password",
        "requestsignature",
        "x-amz-signature",
        "sig=",
    ] {
        assert!(!persisted.contains(forbidden), "{forbidden}");
    }
    service.shutdown().unwrap();
}

#[test]
fn sentinel2_archive_batch_keeps_partial_admission_visible_in_one_real_task() {
    let database = TempStore::new("mpc-sentinel2-cdse-batch-admission");
    let (source_batch, source_items) = seed_two_microsoft_sentinel2_archive_sources(&database);

    let mut delivery_product = exact_archive_product("4b29ea58-874a-45d5-ae3e-0ab714cbafcc");
    delivery_product.display_name = source_items[0].display_name.clone();
    delivery_product.acquisition_datetime = source_items[0].acquisition_datetime.clone();
    delivery_product.footprint_geojson = source_items[0].footprint_geojson.clone();
    delivery_product
        .properties
        .insert("product_type".to_owned(), json!("S2MSI2A"));
    delivery_product
        .properties
        .insert("online".to_owned(), json!(true));
    delivery_product.properties.insert(
        "checksums".to_owned(),
        json!([{"algorithm":"MD5","value":"00000000000000000000000000000000"}]),
    );
    let mut delivery_response = complete_response(vec![delivery_product]);
    delivery_response.collection.production_enabled = true;
    delivery_response.stats.requested_limit = Some(100);
    delivery_response.stats.matched_count = Some(1);
    delivery_response.stats.remote_exhausted = true;
    delivery_response.stats.working_set_satisfied = true;

    let service = ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: delivery_response,
        }),
        Arc::new(RecordingWorker::default()),
    );
    bind_aoi(&service, "mpc-sentinel2-cdse-batch-admission");
    let current = service.core_restore(true).unwrap().core.workbench.unwrap();
    let selected = service
        .update_workbench_selection(&insar_core::WorkbenchSelectionCommand {
            batch_id: current.batch_id.clone(),
            expected_revision: current.revision,
            kind: insar_core::WorkbenchSelectionCommandKind::SelectAll,
            item_ids: vec![],
            page_index: 0,
            page_size: 1,
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
    let visible = service
        .workbench_page(&StoreWorkbenchPageRequest {
            batch_id: current.batch_id.clone(),
            expected_revision: selected.revision,
            page_index: 0,
            page_size: 1,
        })
        .unwrap();
    assert_eq!(visible.page.items.len(), 1);
    assert_eq!(visible.page.checked_count, 2);
    let result = service
        .create_sentinel2_archive_batch_task(&Sentinel2CreateArchiveBatchTaskRequest {
            request_id: "mpc-s2-cdse-batch".to_owned(),
            source_batch_id: source_batch.batch_id.clone(),
            expected_workbench_revision: selected.revision,
            item_ids: vec![],
            task_id: "mpc-s2-cdse-batch-task".to_owned(),
            output_root: database
                .0
                .parent()
                .unwrap()
                .join("mpc-s2-cdse-batch-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-19T00:00:00Z".to_owned(),
        })
        .expect("one available SAFE should still create one honest batch task");

    assert_eq!(result.admission.requested_count, 2);
    assert_eq!(result.admission.admitted_count, 1);
    assert_eq!(result.admission.rejected_count, 1);
    assert_eq!(result.admission.rejections.len(), 1);
    assert_eq!(
        result.admission.rejections[0].source_item_id,
        source_items[1].item_id
    );
    assert_eq!(
        result.admission.rejections[0].reason_code,
        "SENTINEL2_ARCHIVE_DELIVERY_UNAVAILABLE"
    );

    let task = result
        .snapshot
        .as_ref()
        .expect("admitted SAFE creates a snapshot")
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == "mpc-s2-cdse-batch-task")
        .expect("one real Store task is created");
    assert_eq!(task.selected_count, 1);
    assert_eq!(task.delivery_requested_count, Some(2));
    assert_eq!(task.delivery_rejected_count, Some(1));
    assert_eq!(task.failed_count, 0);

    let derived = service
        .task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: "mpc-s2-cdse-batch-task".to_owned(),
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(derived.total_count, 1);
    assert_eq!(
        derived.items[0].item.display_name,
        source_items[0].display_name
    );
    let persisted_batch = TaskStore::open(&database.0)
        .unwrap()
        .load_search_batch(&derived.source_batch_id)
        .unwrap()
        .expect("derived archive batch persists");
    assert_eq!(persisted_batch.query["delivery_requested_count"], 2);
    assert_eq!(persisted_batch.query["delivery_admitted_count"], 1);
    assert_eq!(persisted_batch.query["delivery_rejected_count"], 1);
    assert_eq!(
        persisted_batch.query["delivery_admission_rejections"][0]["source_item_id"],
        source_items[1].item_id
    );
    assert_eq!(
        persisted_batch.query["delivery_admission_rejections"][0]["reason_code"],
        "SENTINEL2_ARCHIVE_DELIVERY_UNAVAILABLE"
    );

    let full = service.snapshot(true).unwrap();
    assert_eq!(
        full.restore.workbench_batch.as_ref().unwrap(),
        &source_batch,
        "batch delivery must not replace the active search Workbench"
    );
    service.shutdown().unwrap();

    let reopened = TaskStore::open(&database.0).unwrap();
    let restored = reopened.core_restore(true).unwrap();
    let restored_task = restored
        .tasks
        .iter()
        .find(|task| task.task_id == "mpc-s2-cdse-batch-task")
        .expect("restart preserves the batch task summary");
    assert_eq!(restored_task.selected_count, 1);
    assert_eq!(restored_task.delivery_requested_count, Some(2));
    assert_eq!(restored_task.delivery_rejected_count, Some(1));
    assert_eq!(restored_task.failed_count, 0);
}

#[test]
fn sentinel2_archive_batch_isolates_one_provider_timeout_as_an_admission_rejection() {
    let database = TempStore::new("mpc-sentinel2-cdse-batch-timeout");
    let (source_batch, source_items) = seed_two_microsoft_sentinel2_archive_sources(&database);

    let mut delivery_product = exact_archive_product("4b29ea58-874a-45d5-ae3e-0ab714cbafcc");
    delivery_product.display_name = source_items[0].display_name.clone();
    delivery_product.acquisition_datetime = source_items[0].acquisition_datetime.clone();
    delivery_product.footprint_geojson = source_items[0].footprint_geojson.clone();
    delivery_product
        .properties
        .insert("product_type".to_owned(), json!("S2MSI2A"));
    delivery_product
        .properties
        .insert("online".to_owned(), json!(true));
    delivery_product.properties.insert(
        "checksums".to_owned(),
        json!([{"algorithm":"MD5","value":"00000000000000000000000000000000"}]),
    );
    let mut delivery_response = complete_response(vec![delivery_product]);
    delivery_response.collection.production_enabled = true;
    delivery_response.stats.requested_limit = Some(100);
    delivery_response.stats.matched_count = Some(1);
    delivery_response.stats.remote_exhausted = true;
    delivery_response.stats.working_set_satisfied = true;

    let service = production_fixture_service(
        &database,
        Arc::new(SelectiveSentinel2ArchiveCatalog {
            response: delivery_response,
            fail_spacecraft: "sentinel_2b",
        }),
    );
    let result = service
        .create_sentinel2_archive_batch_task(&Sentinel2CreateArchiveBatchTaskRequest {
            request_id: "mpc-s2-cdse-batch-timeout".to_owned(),
            source_batch_id: source_batch.batch_id.clone(),
            expected_workbench_revision: 0,
            item_ids: source_items
                .iter()
                .map(|item| item.item_id.clone())
                .collect(),
            task_id: "mpc-s2-cdse-batch-timeout-task".to_owned(),
            output_root: database
                .0
                .parent()
                .unwrap()
                .join("mpc-s2-cdse-batch-timeout-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-19T00:00:01Z".to_owned(),
        })
        .expect("one provider timeout must not discard another admitted SAFE");

    assert_eq!(result.admission.requested_count, 2);
    assert_eq!(result.admission.admitted_count, 1);
    assert_eq!(result.admission.rejected_count, 1);
    assert_eq!(result.admission.rejections.len(), 1);
    assert_eq!(
        result.admission.rejections[0].source_item_id,
        source_items[1].item_id
    );
    assert_eq!(
        result.admission.rejections[0].reason_code,
        "SENTINEL2_ARCHIVE_LOOKUP_FAILED"
    );
    let task = result
        .snapshot
        .as_ref()
        .expect("admitted SAFE creates a snapshot")
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == "mpc-s2-cdse-batch-timeout-task")
        .unwrap();
    assert_eq!(task.selected_count, 1);
    assert_eq!(task.delivery_requested_count, Some(2));
    assert_eq!(task.delivery_rejected_count, Some(1));
    assert_eq!(task.failed_count, 0);
    service.shutdown().unwrap();
}

#[test]
fn explicit_optical_selection_admits_the_active_workbench_batch_at_its_current_revision() {
    // An explicit item selection from the batch that currently owns the active
    // Workbench slot must match that slot's current revision, not be forced to
    // revision 0: the UI downloads explicitly selected items from the active
    // unified-search batch at the revision it last saw.
    let database = TempStore::new("mpc-s2-explicit-active-revision");
    let mut source_response = valid_response();
    source_response.collection = serde_json::from_value(json!({
        "logical_dataset":"optical.sentinel-2.l2a",
        "provider_id":"microsoft.planetary-computer",
        "native_collection_id":"sentinel-2-l2a",
        "collection_profile_id":"planetary-computer.sentinel-2-l2a.scene.v1",
        "identity_rule_id":"planetary-computer.sentinel-2-l2a.identity.v1",
        "filename_rule_id":"planetary-computer.sentinel-2-l2a.product-uri.v1",
        "delivery_model":"archive_first",
        "production_enabled":true
    }))
    .unwrap();
    source_response.products = vec![serde_json::from_value(json!({
        "provider_id":"microsoft.planetary-computer",
        "product_id":"S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916",
        "display_name":"S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE",
        "source_kind":"sentinel2",
        "product_kind":"optical_scene",
        "acquisition_datetime":"2026-07-06T03:25:11Z",
        "footprint_geojson":{"type":"Polygon","coordinates":[[[108,31],[109,31],[109,32],[108,31]]]},
        "assets":[],
        "properties":{"product_type":"S2MSI2A","cloud_cover":4.2},
        "payload":{}
    }))
    .unwrap()];
    source_response.stats.matched_count = None;
    source_response.stats.remote_exhausted = false;
    source_response.stats.working_set_satisfied = true;
    source_response.stats.requested_limit = Some(1);
    source_response.stats.fetched_record_count = 1;
    source_response.stats.mapped_record_count = 1;
    source_response.stats.unique_provider_product_count = 1;
    source_response.stats.exact_matched_count = 1;
    source_response.stats.returned_count = 1;

    let service = ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: source_response,
        }),
        Arc::new(RecordingWorker::default()),
    );
    bind_aoi(&service, "mpc-s2-explicit-active-revision");
    let mut source_request = request(
        "mpc-s2-explicit-active-source",
        "mpc-s2-explicit-active-source-batch",
    );
    source_request.search.provider_id = "microsoft.planetary-computer".to_owned();
    source_request.search.native_collection_id = "sentinel-2-l2a".to_owned();
    source_request.search.result_limit = Some(1);
    let source = service
        .catalog_search_and_freeze_legacy(&source_request)
        .expect("Microsoft search-only batch freezes");
    let source_batch = source.restore.workbench_batch.clone().unwrap();
    let source_item = source_batch.candidates[0].clone();
    assert!(source_item.asset_access.is_none());

    let evidence = StaticSentinel2BandEvidence {
        manifest: MpcSentinel2BandManifest {
            item_id: "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916".to_owned(),
            safe_name: source_item.display_name.clone(),
            assets: vec![MpcSentinel2BandAsset {
                asset_key: "B02".to_owned(),
                stable_href: "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R10m/T48SYC_20260706T032511_B02_10m.tif"
                    .to_owned(),
                file_name: "T48SYC_20260706T032511_B02_10m.tif".to_owned(),
                resolution_m: 10,
            }],
        },
        sizes: BTreeMap::from([("B02".to_owned(), 118_000_001)]),
        manifest_calls: Arc::new(Mutex::new(Vec::new())),
        probe_calls: Arc::new(Mutex::new(Vec::new())),
    };
    // One selection command moves the active slot to revision 1.
    let current = service.core_restore(true).unwrap().core.workbench.unwrap();
    assert_eq!(current.batch_id, source_batch.batch_id);
    assert_eq!(current.revision, 0);
    let selected = service
        .update_workbench_selection(&insar_core::WorkbenchSelectionCommand {
            batch_id: current.batch_id.clone(),
            expected_revision: current.revision,
            kind: insar_core::WorkbenchSelectionCommandKind::SelectAll,
            item_ids: vec![],
            page_index: 0,
            page_size: 1,
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
    assert_eq!(selected.revision, 1);

    let request = Sentinel2CreateBandBatchTaskRequest {
        request_id: "mpc-s2-explicit-active-revision".to_owned(),
        source_batch_id: source_batch.batch_id.clone(),
        expected_workbench_revision: selected.revision,
        item_ids: vec![source_item.item_id.clone()],
        asset_keys: vec!["B02".to_owned()],
        task_id: "mpc-s2-explicit-active-task".to_owned(),
        output_root: database
            .0
            .parent()
            .unwrap()
            .join("mpc-s2-explicit-active-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-19T00:00:00Z".to_owned(),
    };
    let created = service
        .create_sentinel2_band_batch_task_with_evidence_for_test(&request, &evidence)
        .expect("an explicit selection from the active slot admits at the slot revision");
    assert_eq!(created.admission.admitted_count, 1);
    assert!(created.snapshot.is_some(), "admitted band creates a task");

    // A stale explicit request naming the old revision must be rejected: the
    // slot the batch owns has moved on.
    let mut stale = request.clone();
    stale.request_id = "mpc-s2-explicit-active-stale".to_owned();
    stale.task_id = "mpc-s2-explicit-active-stale-task".to_owned();
    stale.expected_workbench_revision = 0;
    let error = service
        .create_sentinel2_band_batch_task_with_evidence_for_test(&stale, &evidence)
        .unwrap_err();
    assert_eq!(
        error.code(),
        "SENTINEL2_BAND_FROZEN_EVIDENCE_INVALID",
        "a stale slot revision must be rejected for the active batch"
    );
    service.shutdown().unwrap();
}

#[test]
fn explicit_optical_selection_admits_an_inactive_batch_only_at_frozen_revision_zero() {
    // An explicit item selection from a unified-search batch that is NOT the
    // active Workbench slot admits from that frozen batch at revision 0 only;
    // a nonzero slot revision borrowed from another batch must be rejected.
    let database = TempStore::new("mpc-s2-explicit-inactive-revision");
    let mut source_response = valid_response();
    source_response.collection = serde_json::from_value(json!({
        "logical_dataset":"optical.sentinel-2.l2a",
        "provider_id":"microsoft.planetary-computer",
        "native_collection_id":"sentinel-2-l2a",
        "collection_profile_id":"planetary-computer.sentinel-2-l2a.scene.v1",
        "identity_rule_id":"planetary-computer.sentinel-2-l2a.identity.v1",
        "filename_rule_id":"planetary-computer.sentinel-2-l2a.product-uri.v1",
        "delivery_model":"archive_first",
        "production_enabled":true
    }))
    .unwrap();
    source_response.products = vec![serde_json::from_value(json!({
        "provider_id":"microsoft.planetary-computer",
        "product_id":"S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916",
        "display_name":"S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE",
        "source_kind":"sentinel2",
        "product_kind":"optical_scene",
        "acquisition_datetime":"2026-07-06T03:25:11Z",
        "footprint_geojson":{"type":"Polygon","coordinates":[[[108,31],[109,31],[109,32],[108,31]]]},
        "assets":[],
        "properties":{"product_type":"S2MSI2A","cloud_cover":4.2},
        "payload":{}
    }))
    .unwrap()];
    source_response.stats.matched_count = None;
    source_response.stats.remote_exhausted = false;
    source_response.stats.working_set_satisfied = true;
    source_response.stats.requested_limit = Some(1);
    source_response.stats.fetched_record_count = 1;
    source_response.stats.mapped_record_count = 1;
    source_response.stats.unique_provider_product_count = 1;
    source_response.stats.exact_matched_count = 1;
    source_response.stats.returned_count = 1;

    let service = ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: source_response,
        }),
        Arc::new(RecordingWorker::default()),
    );
    bind_aoi(&service, "mpc-s2-explicit-inactive-revision");
    let mut source_request = request(
        "mpc-s2-explicit-inactive-source",
        "mpc-s2-explicit-inactive-source-batch",
    );
    source_request.search.provider_id = "microsoft.planetary-computer".to_owned();
    source_request.search.native_collection_id = "sentinel-2-l2a".to_owned();
    source_request.search.result_limit = Some(1);
    let source = service
        .catalog_search_and_freeze_legacy(&source_request)
        .expect("Microsoft search-only batch freezes");
    let source_batch = source.restore.workbench_batch.clone().unwrap();
    let source_item = source_batch.candidates[0].clone();
    assert!(source_item.asset_access.is_none());

    // Replace the active slot with another source's search so the named batch
    // is no longer the active Workbench slot.
    let mut other_request = request(
        "mpc-s2-explicit-inactive-other",
        "mpc-s2-explicit-inactive-other-batch",
    );
    other_request.search.provider_id = "microsoft.planetary-computer".to_owned();
    other_request.search.native_collection_id = "sentinel-2-l2a".to_owned();
    other_request.search.result_limit = Some(1);
    service
        .catalog_search_and_freeze_legacy(&other_request)
        .expect("another source's search replaces the active slot");
    let active = service.core_restore(true).unwrap().core.workbench.unwrap();
    assert_ne!(active.batch_id, source_batch.batch_id);
    // Move the new slot's revision so a nonzero revision exists in the store.
    let selected = service
        .update_workbench_selection(&insar_core::WorkbenchSelectionCommand {
            batch_id: active.batch_id.clone(),
            expected_revision: active.revision,
            kind: insar_core::WorkbenchSelectionCommandKind::SelectAll,
            item_ids: vec![],
            page_index: 0,
            page_size: 1,
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
    assert_eq!(selected.revision, 1);

    let evidence = StaticSentinel2BandEvidence {
        manifest: MpcSentinel2BandManifest {
            item_id: "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916".to_owned(),
            safe_name: source_item.display_name.clone(),
            assets: vec![MpcSentinel2BandAsset {
                asset_key: "B02".to_owned(),
                stable_href: "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R10m/T48SYC_20260706T032511_B02_10m.tif"
                    .to_owned(),
                file_name: "T48SYC_20260706T032511_B02_10m.tif".to_owned(),
                resolution_m: 10,
            }],
        },
        sizes: BTreeMap::from([("B02".to_owned(), 118_000_001)]),
        manifest_calls: Arc::new(Mutex::new(Vec::new())),
        probe_calls: Arc::new(Mutex::new(Vec::new())),
    };
    let request = Sentinel2CreateBandBatchTaskRequest {
        request_id: "mpc-s2-explicit-inactive".to_owned(),
        source_batch_id: source_batch.batch_id.clone(),
        expected_workbench_revision: 0,
        item_ids: vec![source_item.item_id.clone()],
        asset_keys: vec!["B02".to_owned()],
        task_id: "mpc-s2-explicit-inactive-task".to_owned(),
        output_root: database
            .0
            .parent()
            .unwrap()
            .join("mpc-s2-explicit-inactive-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-19T00:00:00Z".to_owned(),
    };
    // Revision 0 admits: the inactive batch was frozen at revision 0.
    let created = service
        .create_sentinel2_band_batch_task_with_evidence_for_test(&request, &evidence)
        .expect("an explicit selection from an inactive batch admits at frozen revision 0");
    assert_eq!(created.admission.admitted_count, 1);
    assert!(created.snapshot.is_some(), "admitted band creates a task");

    // A nonzero revision borrowed from the other batch's slot must be rejected.
    let mut rejected_request = request.clone();
    rejected_request.request_id = "mpc-s2-explicit-inactive-stale".to_owned();
    rejected_request.task_id = "mpc-s2-explicit-inactive-stale-task".to_owned();
    rejected_request.expected_workbench_revision = selected.revision;
    let error = service
        .create_sentinel2_band_batch_task_with_evidence_for_test(&rejected_request, &evidence)
        .unwrap_err();
    assert_eq!(
        error.code(),
        "SENTINEL2_BAND_FROZEN_EVIDENCE_INVALID",
        "an inactive batch must never admit at another slot's revision"
    );
    service.shutdown().unwrap();
}

#[test]
fn sentinel2_archive_batch_duplicate_request_and_cancel_publish_no_task() {
    let database = TempStore::new("mpc-sentinel2-cdse-batch-cancel");
    let (source_batch, source_items) = seed_two_microsoft_sentinel2_archive_sources(&database);
    let catalogue = Arc::new(BlockingCatalog {
        response: production_response(),
        entered: AtomicBool::new(false),
        release: AtomicBool::new(false),
    });
    let service = Arc::new(production_fixture_service(&database, catalogue.clone()));
    let operation = Sentinel2CreateArchiveBatchTaskRequest {
        request_id: "mpc-s2-cdse-batch-cancel".to_owned(),
        source_batch_id: source_batch.batch_id.clone(),
        expected_workbench_revision: 0,
        item_ids: source_items
            .iter()
            .map(|item| item.item_id.clone())
            .collect(),
        task_id: "mpc-s2-cdse-batch-cancel-task".to_owned(),
        output_root: database
            .0
            .parent()
            .unwrap()
            .join("mpc-s2-cdse-batch-cancel-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-19T00:00:02Z".to_owned(),
    };
    let pending = {
        let service = Arc::clone(&service);
        let operation = operation.clone();
        thread::spawn(move || service.create_sentinel2_archive_batch_task(&operation))
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while !catalogue.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    if !catalogue.entered.load(Ordering::Acquire) {
        catalogue.release.store(true, Ordering::Release);
        let result = pending.join().unwrap();
        panic!("batch source lookup did not start: {result:?}");
    }

    let duplicate = service
        .create_sentinel2_archive_batch_task(&operation)
        .expect_err("same request identity must not create a second batch task");
    assert_eq!(duplicate.code(), "SENTINEL2_ARCHIVE_REQUEST_DUPLICATE");
    assert!(service.cancel_request(&operation.request_id).unwrap());

    catalogue.release.store(true, Ordering::Release);
    let cancelled = pending.join().unwrap().unwrap_err();
    assert!(
        cancelled.code().contains("CANCEL"),
        "batch cancellation must stay typed: {}",
        cancelled.code()
    );
    let snapshot = service.snapshot(true).unwrap();
    assert!(snapshot.restore.tasks.is_empty());
    assert_eq!(
        snapshot.restore.workbench_batch.as_ref(),
        Some(&source_batch)
    );
    service.shutdown().unwrap();
}

#[test]
fn sentinel2_archive_batch_all_rejected_creates_no_empty_task() {
    let database = TempStore::new("mpc-sentinel2-cdse-batch-all-rejected");
    let (source_batch, source_items) = seed_two_microsoft_sentinel2_archive_sources(&database);
    let source_item = &source_items[0];
    let mut unrelated = exact_archive_product("4b29ea58-874a-45d5-ae3e-0ab714cbafcc");
    unrelated.display_name =
        "S2C_MSIL2A_20260917T033531_N0512_R061_T48SWB_20260917T141518.SAFE".to_owned();
    unrelated.acquisition_datetime = source_item.acquisition_datetime.clone();
    unrelated.footprint_geojson = source_item.footprint_geojson.clone();
    unrelated
        .properties
        .insert("product_type".to_owned(), json!("S2MSI2A"));
    unrelated
        .properties
        .insert("online".to_owned(), json!(true));
    unrelated.properties.insert(
        "checksums".to_owned(),
        json!([{"algorithm":"MD5","value":"00000000000000000000000000000000"}]),
    );
    let mut response = complete_response(vec![unrelated]);
    response.collection.production_enabled = true;
    response.stats.requested_limit = Some(100);
    response.stats.matched_count = Some(1);
    response.stats.remote_exhausted = true;
    response.stats.working_set_satisfied = true;
    let service = production_fixture_service(&database, Arc::new(StaticCatalog { response }));

    let error = service
        .create_sentinel2_archive_batch_task(&Sentinel2CreateArchiveBatchTaskRequest {
            request_id: "mpc-s2-cdse-batch-all-rejected".to_owned(),
            source_batch_id: source_batch.batch_id.clone(),
            expected_workbench_revision: 0,
            item_ids: vec![source_item.item_id.clone()],
            task_id: "mpc-s2-cdse-batch-all-rejected-task".to_owned(),
            output_root: database
                .0
                .parent()
                .unwrap()
                .join("mpc-s2-cdse-batch-all-rejected-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-09-19T00:00:03Z".to_owned(),
        })
        .expect("all rejections must return an inspectable report without a task");
    let report = serde_json::to_value(error).unwrap();
    assert!(report["snapshot"].is_null());
    assert_eq!(report["admission"]["requested_count"], 1);
    assert_eq!(report["admission"]["admitted_count"], 0);
    assert_eq!(report["admission"]["rejected_count"], 1);
    assert_eq!(
        report["admission"]["rejections"][0]["source_item_id"],
        source_item.item_id
    );
    assert_eq!(
        report["admission"]["rejections"][0]["reason_code"],
        "SENTINEL2_ARCHIVE_DELIVERY_UNAVAILABLE"
    );
    let snapshot = service.snapshot(true).unwrap();
    assert!(snapshot.restore.tasks.is_empty());
    assert_eq!(
        snapshot.restore.workbench_batch.as_ref(),
        Some(&source_batch)
    );
    service.shutdown().unwrap();
}

#[test]
fn sentinel2_archive_cancellation_reaches_pending_source_lookup_without_store_publication() {
    let database = TempStore::new("archive-cancel-pending-lookup");
    let (source_batch, source_item) = seed_microsoft_sentinel2_archive_source(&database);
    let catalogue = Arc::new(BlockingCatalog {
        response: production_response(),
        entered: AtomicBool::new(false),
        release: AtomicBool::new(false),
    });
    let service = Arc::new(production_fixture_service(&database, catalogue.clone()));
    let operation = Sentinel2CreateArchiveTaskRequest {
        request_id: "archive-parent-cancel".to_owned(),
        source_batch_id: source_batch.batch_id.clone(),
        item_id: source_item.item_id,
        task_id: "cancelled-archive-task".to_owned(),
        output_root: database.0.with_extension("output"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-09-25T00:00:00Z".to_owned(),
    };
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let pending = {
        let service = Arc::clone(&service);
        thread::spawn(move || {
            let result = service.create_sentinel2_archive_task(&operation);
            let _ = finished_tx.send(());
            result
        })
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while !catalogue.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    if !catalogue.entered.load(Ordering::Acquire) {
        catalogue.release.store(true, Ordering::Release);
        let result = pending.join().unwrap();
        panic!("archive source lookup did not start: {result:?}");
    }

    // A pending provider must not hold the Store/runtime lock needed by the UI.
    let (read_tx, read_rx) = std::sync::mpsc::channel();
    let reader = {
        let service = Arc::clone(&service);
        thread::spawn(move || {
            let _ = read_tx.send(service.core_restore(false));
        })
    };
    let read = read_rx.recv_timeout(Duration::from_secs(2));
    if read.is_err() {
        catalogue.release.store(true, Ordering::Release);
        let _ = pending.join();
        let _ = reader.join();
        panic!("pending source lookup blocked the UI Store projection");
    }
    assert!(read.unwrap().unwrap().core.tasks.is_empty());
    reader.join().unwrap();

    let accepted = service.cancel_request("archive-parent-cancel").unwrap();
    let completed_before_release = finished_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    // Always release/join our test double, including RED, so this test cannot leak a thread.
    catalogue.release.store(true, Ordering::Release);
    let result = pending.join().unwrap();
    let unchanged = service.snapshot(true).unwrap();
    service.shutdown().unwrap();

    assert!(accepted, "the public request must remain cancellable");
    assert!(
        completed_before_release,
        "cancel was acknowledged but the nested Provider kept waiting"
    );
    assert!(result.unwrap_err().code().contains("CANCELLED"));
    assert!(unchanged.restore.tasks.is_empty());
    assert_eq!(
        unchanged.restore.workbench_batch.as_ref(),
        Some(&source_batch)
    );
}

#[test]
fn provider_work_does_not_hold_store_lock_and_cancellation_wins_before_publication() {
    let database = TempStore::new("lock-probe");
    let probe = Arc::new(SnapshotProbeCatalog {
        response: valid_response(),
        service: Mutex::new(None),
        observed_store_read: AtomicBool::new(false),
    });
    let service = Arc::new(fixture_service(&database, probe.clone()));
    *probe.service.lock().unwrap() = Some(Arc::downgrade(&service));
    service
        .catalog_search_and_freeze_legacy(&request("lock-probe", "lock-probe-batch"))
        .unwrap();
    assert!(probe.observed_store_read.load(Ordering::Acquire));
    service.shutdown().unwrap();

    let database = TempStore::new("cancel-provider");
    let catalogue = Arc::new(BlockingCatalog {
        response: valid_response(),
        entered: AtomicBool::new(false),
        release: AtomicBool::new(false),
    });
    let service = Arc::new(fixture_service(&database, catalogue.clone()));
    let operation = request("cancel-provider", "cancel-provider-batch");
    let worker = {
        let service = Arc::clone(&service);
        thread::spawn(move || service.catalog_search_and_freeze_legacy(&operation))
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while !catalogue.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(catalogue.entered.load(Ordering::Acquire));
    assert!(service.cancel_request("cancel-provider").unwrap());
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.code(), "CATALOGUE_REQUEST_CANCELLED");
    assert_empty_store(&service);
    service.shutdown().unwrap();
}

#[test]
fn cancellation_after_provider_return_aborts_before_store_publication() {
    let database = TempStore::new("cancel-publication");
    let gate = Arc::new(PublicationGate {
        entered: AtomicBool::new(false),
        release: AtomicBool::new(false),
    });
    let hook_gate = Arc::clone(&gate);
    let hook: Arc<dyn Fn() + Send + Sync> = Arc::new(move || hook_gate.wait());
    let service = Arc::new(
        ApplicationService::with_catalogue_fixture_business_runtime_with_publication_hook(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(StaticCatalog {
                response: valid_response(),
            }),
            Arc::new(FixtureWorker),
            Some(hook),
        ),
    );
    bind_aoi(service.as_ref(), "cancel-publication");
    let operation = request("cancel-publication", "cancel-publication-batch");
    let worker = {
        let service = Arc::clone(&service);
        thread::spawn(move || service.catalog_search_and_freeze_legacy(&operation))
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while !gate.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(gate.entered.load(Ordering::Acquire));
    assert!(service.cancel_request("cancel-publication").unwrap());
    gate.release.store(true, Ordering::Release);
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.code(), "CATALOGUE_REQUEST_CANCELLED");
    assert_empty_store(&service);
    service.shutdown().unwrap();
}

#[test]
fn publication_winning_the_race_rejects_late_cancellation_and_commits_once() {
    let database = TempStore::new("publication-wins");
    let gate = Arc::new(PublicationGate {
        entered: AtomicBool::new(false),
        release: AtomicBool::new(false),
    });
    let hook_gate = Arc::clone(&gate);
    let after_hook: Arc<dyn Fn() + Send + Sync> = Arc::new(move || hook_gate.wait());
    let service = Arc::new(
        ApplicationService::with_catalogue_fixture_business_runtime_with_publication_hooks(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(StaticCatalog {
                response: valid_response(),
            }),
            Arc::new(FixtureWorker),
            None,
            Some(after_hook),
        ),
    );
    bind_aoi(service.as_ref(), "publication-wins");
    let operation = request("publication-wins", "publication-wins-batch");
    let worker = {
        let service = Arc::clone(&service);
        thread::spawn(move || service.catalog_search_and_freeze_legacy(&operation))
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while !gate.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(gate.entered.load(Ordering::Acquire));
    assert!(!service.cancel_request("publication-wins").unwrap());
    gate.release.store(true, Ordering::Release);
    let snapshot = worker.join().unwrap().unwrap();
    assert_eq!(
        snapshot.restore.workbench_batch.unwrap().batch_id,
        "publication-wins-batch"
    );
    service.shutdown().unwrap();
}

#[test]
fn production_business_constructor_is_lazy_and_does_not_contact_catalogue_or_mutate_store() {
    let database = TempStore::new("production-isolation");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database.0));
    assert_eq!(
        service.status().unwrap(),
        insar_application_service::ServiceStatus::Created
    );
    assert!(!database.0.exists());
    assert_empty_store(&service);
    service.shutdown().unwrap();
}

#[test]
fn production_route_freezes_one_executable_archive_without_quicklook_metadata_offline() {
    let database = TempStore::new("production-freeze");
    let service = production_fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: production_response(),
        }),
    );

    let snapshot = service
        .catalog_search_and_freeze_legacy(&request("production-freeze", "production-freeze-batch"))
        .expect("injected production route freezes without public network");
    let candidate = &snapshot
        .restore
        .workbench_batch
        .expect("frozen batch")
        .candidates[0];
    let access = candidate
        .asset_access
        .as_ref()
        .expect("executable archive access");
    assert_eq!(access.descriptor.resolver_id, "cdse.archive.resolver.v1");
    assert_eq!(
        access.file_name.as_deref(),
        Some("S2_FIXTURE_VALID.SAFE.zip")
    );
    assert_eq!(access.expected_size_bytes, Some(2048));
    assert_eq!(access.checksum_algorithm.as_deref(), Some("MD5"));
    assert_eq!(
        candidate
            .execution_policy
            .as_ref()
            .map(|policy| policy.policy_id.as_str()),
        Some("cdse.sentinel-2.l2a.archive-resolver-policy.v1")
    );

    let assets = candidate.metadata["assets"]
        .as_array()
        .expect("closed asset list");
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0]["asset_key"], "archive");
    assert!(assets[0].get("href").is_none());
    assert!(assets[0].get("media_type").is_none());
    assert!(candidate.metadata["properties"].get("s3_path").is_none());
    let serialized = serde_json::to_string(candidate).expect("candidate serializes");
    assert!(!serialized.contains("provider-only"));
    assert!(!serialized.to_ascii_lowercase().contains("quicklook"));
    assert!(!serialized.to_ascii_lowercase().contains("thumbnail"));
    service.shutdown().unwrap();
}

#[test]
fn production_route_freezes_the_approved_archive_when_collection_echo_is_absent() {
    let database = TempStore::new("production-omitted-collection");
    let response = approved_product_response_without_collection_echo();
    assert!(response.item_errors.is_empty());
    let service = production_fixture_service(&database, Arc::new(StaticCatalog { response }));

    let snapshot = service
        .catalog_search_and_freeze_legacy(&approved_product_request())
        .expect("query-bound collection proof freezes the approved archive");
    let batch = snapshot.restore.workbench_batch.expect("production batch");
    assert_eq!(batch.candidates.len(), 1);
    let candidate = &batch.candidates[0];
    assert_eq!(candidate.item_id, "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::165ba4c9-997c-4cea-970f-33ab9bdb8084");
    assert_eq!(
        candidate.display_name,
        "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE"
    );
    assert!(candidate.footprint_geojson.is_some());
    let access = candidate
        .asset_access
        .as_ref()
        .expect("executable archive access");
    assert_eq!(access.expected_size_bytes, Some(867907004));
    assert_eq!(access.checksum_algorithm.as_deref(), Some("MD5"));
    assert_eq!(
        access.file_name.as_deref(),
        Some("S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE.zip")
    );
    service.shutdown().expect("production service shuts down");
}

#[test]
fn production_route_rejects_downloadable_archive_without_checksum_before_store_publication() {
    let database = TempStore::new("production-missing-checksum");
    let mut response = valid_response();
    response.collection.production_enabled = true;
    let service = production_fixture_service(&database, Arc::new(StaticCatalog { response }));

    assert!(service
        .catalog_search_and_freeze_legacy(&request(
            "production-missing-checksum",
            "production-missing-checksum-batch",
        ))
        .is_err());
    assert_empty_store(&service);
    service.shutdown().unwrap();
}

#[test]
fn production_frozen_sentinel2_selection_creates_and_restores_one_store_owned_task() {
    let database = TempStore::new("production-store-task");
    let worker = Arc::new(RecordingWorker::default());
    let service = ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: production_response(),
        }),
        worker.clone(),
    );
    bind_aoi(&service, "production-store-task");
    let frozen = service
        .catalog_search_and_freeze_legacy(&request(
            "production-store-task",
            "production-store-batch",
        ))
        .expect("production fixture freezes one executable archive");
    let batch = frozen.restore.workbench_batch.expect("frozen batch");
    let item_id = batch.candidates[0].item_id.clone();

    let created = service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: "production-store-task-item".to_owned(),
            batch_id: batch.batch_id,
            selected_item_ids: vec![item_id.clone()],
            checked_item_ids: vec![item_id.clone()],
            page_index: 0,
            page_size: 24,
            output_root: database.0.with_extension("output"),
            layout: DownloadLayout::SarDatasetProcessing,
            concurrency: 1,
            created_at: "2026-07-27T00:00:00Z".to_owned(),
        })
        .expect("frozen Sentinel-2 item creates a Store task");
    let task = created
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "production-store-task-item")
        .expect("created Store task");
    assert_eq!(task.initial_selected_item_ids, vec![item_id.clone()]);
    let access = task.candidates[0]
        .asset_access
        .as_ref()
        .expect("task retains frozen archive access");
    assert_eq!(access.descriptor.resolver_id, "cdse.archive.resolver.v1");
    assert_eq!(
        access.file_name.as_deref(),
        Some("S2_FIXTURE_VALID.SAFE.zip")
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.items.lock().unwrap().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    let executed = worker.items.lock().unwrap();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].item_id, item_id);
    assert_eq!(
        executed[0]
            .asset_access
            .as_ref()
            .and_then(|access| access.file_name.as_deref()),
        Some("S2_FIXTURE_VALID.SAFE.zip")
    );
    drop(executed);
    service.shutdown().unwrap();

    let restarted = ApplicationService::new(ApplicationServiceConfig::business_store(&database.0));
    let restored = restarted
        .restore(true)
        .expect("Store task restores after restart");
    let task = restored
        .tasks
        .iter()
        .find(|task| task.task_id == "production-store-task-item")
        .expect("restored Store task");
    assert_eq!(task.initial_selected_item_ids, vec![item_id]);
    assert_eq!(
        task.candidates[0]
            .asset_access
            .as_ref()
            .and_then(|access| access.descriptor.stable_href.as_deref()),
        Some(
            "https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000011)/$value"
        )
    );
    restarted.shutdown().unwrap();
}

#[test]
fn hundred_product_creation_preserves_counts_while_search_and_transfer_are_pending() {
    // The real Service/Store/Scheduler run; only external catalogue and transfer
    // work are controlled doubles. No network or output file is created.
    #[derive(Default)]
    struct ActiveProgressWorker {
        release: AtomicBool,
        started: AtomicU64,
        ticks: AtomicU64,
        release_observed: AtomicU64,
        exited: AtomicU64,
    }
    impl DownloadWorker for ActiveProgressWorker {
        fn execute(
            &self,
            work: DownloadWork,
            control: WorkerControl,
            progress: &mut dyn FnMut(DownloadItemProgress),
        ) -> DownloadItemResult {
            let ordinal = self.started.fetch_add(1, Ordering::AcqRel);
            // This is only a fixture fail-safe while the main thread deliberately
            // keeps Provider/search and Store/UI stress work in flight. Full
            // workspace/package validation can make those 20 rounds exceed 20 s
            // on a loaded Windows builder. The real post-release convergence SLA
            // remains the separate 20 s worker_deadline below.
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut bytes = 0;
            loop {
                bytes = (bytes + 1).min(1024);
                progress(DownloadItemProgress {
                    task_id: work.task.task_id.clone(),
                    item_id: work.item.item_id.clone(),
                    status: insar_core::DownloadItemStatus::Active,
                    bytes_completed: bytes,
                    total_bytes: Some(2048),
                    bytes_per_second: 100.0,
                    attempt: 1,
                    updated_at: chrono::Utc::now().to_rfc3339(),
                });
                self.ticks.fetch_add(1, Ordering::AcqRel);
                if self.release.load(Ordering::Acquire) {
                    self.release_observed.fetch_add(1, Ordering::AcqRel);
                    break;
                }
                if control.is_cancelled() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "test transfer gate was not released"
                );
                thread::sleep(Duration::from_millis(10));
            }
            self.exited.fetch_add(1, Ordering::AcqRel);
            let failed = ordinal >= 78;
            DownloadItemResult {
                task_id: work.task.task_id,
                item_id: work.item.item_id,
                outcome: if control.is_cancelled() {
                    DownloadOutcome::Cancelled
                } else if failed {
                    DownloadOutcome::Failed
                } else {
                    DownloadOutcome::Completed
                },
                destination: work.task.output_root.join(format!("fixture-{ordinal}.zip")),
                bytes_written: if failed { bytes } else { 2048 },
                attempt_count: 1,
                failure: failed.then(|| insar_core::DownloadFailure {
                    code: "FIXTURE_TRANSFER_FAILURE".to_owned(),
                    reason_zh: "可控传输失败".to_owned(),
                    retryable: false,
                }),
                finished_at: chrono::Utc::now().to_rfc3339(),
            }
        }
    }
    struct ReleaseOnDrop(Arc<ActiveProgressWorker>, Arc<BlockingCatalog>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.release.store(true, Ordering::Release);
            self.1.release.store(true, Ordering::Release);
        }
    }

    let database = TempStore::new("active-100-products");
    let products = (0..100)
        .map(|index| {
            let mut product =
                exact_archive_product(&format!("00000000-0000-4000-8000-{index:012}"));
            product.display_name = format!("S2_ACTIVE_{index:03}.SAFE");
            product.properties.insert(
                "checksums".to_owned(),
                json!([{"algorithm":"MD5","value":"00000000000000000000000000000000"}]),
            );
            product
        })
        .collect();
    let mut response = complete_response(products);
    response.collection.production_enabled = true;
    // complete_response derives returned/fetched/mapped counts from products.
    // Keep the requested and exact-match counts explicit; positional numeric
    // arguments previously confused returned_count with the one HTTP request.
    response.stats.requested_limit = Some(100);
    response.stats.exact_matched_count = response.products.len() as u64;
    response
        .validate()
        .expect("100-product fixture must satisfy the production response contract");
    let catalogue = Arc::new(BlockingCatalog {
        response,
        entered: AtomicBool::new(false),
        release: AtomicBool::new(true),
    });
    let worker = Arc::new(ActiveProgressWorker::default());
    let service = Arc::new(
        ApplicationService::with_catalogue_production_business_runtime(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            catalogue.clone(),
            worker.clone(),
        ),
    );
    let _release_on_failure = ReleaseOnDrop(worker.clone(), catalogue.clone());
    bind_aoi(&service, "active-100-products");
    let mut initial = request("active-100-initial", "active-100-source");
    initial.search.result_limit = Some(100);
    let frozen = service.catalog_search_and_freeze(&initial).unwrap();
    let summary = frozen.core.workbench.unwrap();
    assert_eq!(summary.candidate_count, 100);
    let selected = service
        .update_workbench_selection(&insar_core::WorkbenchSelectionCommand {
            batch_id: summary.batch_id.clone(),
            expected_revision: summary.revision,
            kind: insar_core::WorkbenchSelectionCommandKind::SelectAll,
            item_ids: vec![],
            page_index: 0,
            page_size: 24,
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
    // Expand the query beyond the cached source interval. Reusing the exact
    // initial query would test a cache hit, not an in-flight Provider request.
    catalogue.release.store(false, Ordering::Release);
    catalogue.entered.store(false, Ordering::Release);
    let mut pending_request = request("active-100-pending-search", "unpublished-search");
    pending_request.search.result_limit = Some(100);
    pending_request.search.filters.insert(
        "datetime".to_owned(),
        json!({"start":"2026-01-01T00:00:00Z", "end":"2026-01-03T00:00:00Z"}),
    );
    let searching = {
        let service = Arc::clone(&service);
        thread::spawn(move || service.catalog_search_and_freeze(&pending_request))
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    while !catalogue.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        catalogue.entered.load(Ordering::Acquire),
        "the expanded search must reach the blocked Provider, not reuse the cache"
    );
    assert!(!searching.is_finished());

    let create_started = Instant::now();
    let created = service
        .create_task_from_workbench(&insar_core::CreateTaskFromWorkbenchRequest {
            task_id: "active-100-task".to_owned(),
            batch_id: summary.batch_id.clone(),
            expected_workbench_revision: selected.revision,
            exact_item_id: None,
            output_root: database.0.with_extension("output"),
            layout: DownloadLayout::Direct,
            concurrency: 4,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
    let create_ms = create_started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(created.core.tasks.len(), 1);
    assert_eq!(created.core.tasks[0].selected_count, 100);

    assert!(
        !searching.is_finished(),
        "creation must complete while the independent Provider remains pending"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while worker.ticks.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(worker.ticks.load(Ordering::Acquire) > 0);
    let ticks_before = worker.ticks.load(Ordering::Acquire);
    let mut core_ms = Vec::new();
    let mut page_ms = Vec::new();
    let mut select_ms = Vec::new();
    for round in 0..20 {
        let clock = Instant::now();
        let restored = service.core_restore(false).unwrap();
        core_ms.push(clock.elapsed().as_secs_f64() * 1000.0);
        let task = restored
            .core
            .tasks
            .iter()
            .find(|task| task.task_id == "active-100-task")
            .unwrap();
        assert_eq!(task.selected_count, 100);
        assert_eq!(
            task.active_count
                + task.queued_count
                + task.paused_count
                + task.completed_count
                + task.failed_count
                + task.skipped_count
                + task.cancelled_count,
            100
        );
        let current = restored.core.workbench.unwrap();
        assert_eq!(current.batch_id, summary.batch_id);
        let clock = Instant::now();
        let selection = service
            .update_workbench_selection(&insar_core::WorkbenchSelectionCommand {
                batch_id: current.batch_id.clone(),
                expected_revision: current.revision,
                kind: if round % 2 == 0 {
                    insar_core::WorkbenchSelectionCommandKind::ClearAll
                } else {
                    insar_core::WorkbenchSelectionCommandKind::SelectAll
                },
                item_ids: vec![],
                page_index: 0,
                page_size: 24,
                updated_at: chrono::Utc::now().to_rfc3339(),
            })
            .unwrap();
        select_ms.push(clock.elapsed().as_secs_f64() * 1000.0);
        let clock = Instant::now();
        let page = service
            .workbench_page(&StoreWorkbenchPageRequest {
                batch_id: current.batch_id,
                expected_revision: selection.revision,
                page_index: 0,
                page_size: 24,
            })
            .unwrap();
        page_ms.push(clock.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(page.page.total_count, 100);
        assert_eq!(page.page.items.len(), 24);
        assert_eq!(
            page.page.checked_count,
            if round % 2 == 0 { 0 } else { 100 }
        );
    }
    assert!(
        worker.ticks.load(Ordering::Acquire) > ticks_before,
        "real scheduler progress must continue"
    );
    assert!(service.cancel_request("active-100-pending-search").unwrap());
    let search_result = searching.join().unwrap();
    assert_eq!(
        search_result.unwrap_err().code(),
        "CATALOGUE_REQUEST_CANCELLED"
    );
    worker.release.store(true, Ordering::Release);

    // First prove Scheduler handoff without competing for the Store lock from
    // the verifier. Product UI status polling is separate and much slower.
    let settle_started = Instant::now();
    let worker_deadline = settle_started + Duration::from_secs(20);
    while worker.exited.load(Ordering::Acquire) < 100 {
        assert!(
            Instant::now() < worker_deadline,
            "100 controlled workers did not all run after release: started={}, release={}, release_observed={}, exited={}, ticks={}, create_ms={create_ms}, core_ms={core_ms:?}, selection_ms={select_ms:?}, page_ms={page_ms:?}",
            worker.started.load(Ordering::Acquire),
            worker.release.load(Ordering::Acquire),
            worker.release_observed.load(Ordering::Acquire),
            worker.exited.load(Ordering::Acquire),
            worker.ticks.load(Ordering::Acquire)
        );
        thread::sleep(Duration::from_millis(10));
    }

    // Then prove the finite Finished backlog is fully persisted. 500 ms is still
    // more aggressive than the product's 1.2 s status polling cadence.
    let terminal_deadline = Instant::now() + Duration::from_secs(10);
    let terminal = loop {
        let snapshot = service.core_restore(true).unwrap();
        let task = snapshot
            .core
            .tasks
            .into_iter()
            .find(|task| task.task_id == "active-100-task")
            .unwrap();
        if task.completed_count + task.failed_count == 100 {
            break task;
        }
        assert!(
            Instant::now() < terminal_deadline,
            "100 controlled results did not settle after all workers exited: status={:?}, selected={}, completed={}, failed={}, active={}, queued={}, paused={}, skipped={}, cancelled={}, started={}, release_observed={}, exited={}",
            task.status,
            task.selected_count,
            task.completed_count,
            task.failed_count,
            task.active_count,
            task.queued_count,
            task.paused_count,
            task.skipped_count,
            task.cancelled_count,
            worker.started.load(Ordering::Acquire),
            worker.release_observed.load(Ordering::Acquire),
            worker.exited.load(Ordering::Acquire)
        );
        thread::sleep(Duration::from_millis(500));
    };
    let settle_ms = settle_started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(terminal.selected_count, 100);
    assert_eq!(terminal.completed_count, 78);
    assert_eq!(terminal.failed_count, 22);
    assert_eq!(worker.started.load(Ordering::Acquire), 100);
    assert_eq!(
        service
            .core_restore(false)
            .unwrap()
            .core
            .workbench
            .unwrap()
            .batch_id,
        summary.batch_id
    );
    service.shutdown().unwrap();
    let reopened = ApplicationService::new(ApplicationServiceConfig::existing_store(&database.0));
    let restored = reopened.core_restore(true).unwrap();
    assert_eq!(restored.core.tasks[0].selected_count, 100);
    assert_eq!(restored.core.tasks[0].completed_count, 78);
    assert_eq!(restored.core.tasks[0].failed_count, 22);
    reopened.shutdown().unwrap();
    eprintln!(
        "OA02_ACTIVE_METRICS {}",
        json!({
            "mode":"native integration test; not desktop paint", "selected":100,
            "created":100, "completed":78, "transfer_failed":22,
            "creation_overlapped_pending_provider":true,
            "transfer_outcomes":"controlled fixture; not Owner admission failures",
            "create_ms":create_ms, "settle_ms":settle_ms,
            "core_ms":core_ms, "selection_ms":select_ms, "page_ms":page_ms,
            "progress_ticks":worker.ticks.load(Ordering::Acquire),
            "release_observed":worker.release_observed.load(Ordering::Acquire),
            "worker_exited":worker.exited.load(Ordering::Acquire),
        })
    );
}

#[test]
fn cdse_network_policy_keeps_the_live_catalogue_dispatcher_cancellable() {
    let database = TempStore::new("network-policy-catalogue");
    let catalogue = Arc::new(BlockingCatalog {
        response: production_response(),
        entered: AtomicBool::new(false),
        release: AtomicBool::new(false),
    });
    let service = Arc::new(production_fixture_service(&database, catalogue.clone()));
    let operation = request("network-policy-catalogue", "network-policy-catalogue-batch");
    let worker = {
        let service = Arc::clone(&service);
        thread::spawn(move || service.catalog_search_and_freeze_legacy(&operation))
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while !catalogue.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(catalogue.entered.load(Ordering::Acquire));

    // Ordinary routes default to Direct since W1 (2026-10-24); request the
    // other mode so the switch is not a no-op and the busy gate is exercised.
    let error = service
        .set_cdse_network_policy(CdseNetworkMode::System)
        .expect_err("network policy must not replace a dispatcher with a live catalogue lease");
    assert_eq!(error.code(), "CDSE_NETWORK_POLICY_BUSY");
    assert!(service.cancel_request("network-policy-catalogue").unwrap());
    let error = worker
        .join()
        .unwrap()
        .expect_err("cancelled catalogue request");
    assert_eq!(error.code(), "CATALOGUE_REQUEST_CANCELLED");

    let status = service
        .set_cdse_network_policy(CdseNetworkMode::System)
        .expect("policy may change after the catalogue lease ends");
    assert_eq!(status.network_mode, CdseNetworkMode::System);
    service.shutdown().unwrap();
}

#[test]
fn invalid_or_incomplete_catalogue_inputs_leave_store_unchanged() {
    let cases = [
        ("input-cursor", None, true),
        (
            "next-cursor",
            Some(
                json!({"next_cursor": "opaque", "stats": catalog_search_stats(2, 1, 1, 1, 0, 0, 1, 1, 1, true)}),
            ),
            false,
        ),
        (
            "item-error",
            Some(
                json!({"item_errors": [{"code":"FIXTURE","user_message":"fixture error","debug_context":{}}]}),
            ),
            false,
        ),
        (
            "count-mismatch",
            Some(json!({"stats": catalog_search_stats(2, 2, 1, 1, 0, 0, 1, 1, 1, false)})),
            false,
        ),
        (
            "collection-mismatch",
            Some(
                json!({"collection": {"logical_dataset":"optical.sentinel-2.l2a","provider_id":"wrong.provider","native_collection_id":"SENTINEL-2","collection_profile_id":"cdse.sentinel-2.l2a.collection.v1","identity_rule_id":"cdse.sentinel-2.l2a.identity.v1","filename_rule_id":"cdse.sentinel-2.l2a.archive-filename.v1","delivery_model":"archive_first","production_enabled":false}}),
            ),
            false,
        ),
        (
            "executable-asset",
            Some(
                json!({"products": [{"provider_id":"copernicus.dataspace.odata","product_id":"00000000-0000-4000-8000-000000000011","display_name":"S2_FIXTURE_VALID.SAFE","source_kind":"sentinel2","product_kind":"optical_scene","acquisition_datetime":"2026-01-01T10:00:00Z","footprint_geojson":{"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},"assets":[{"asset_key":"archive","stable_asset_key":"stable","role":"primary","native_roles":["archive"],"url":"https://example.invalid/value"}],"properties":{"product_type":"S2MSI2A","online":true},"payload":{}}]}),
            ),
            false,
        ),
        (
            "opaque-payload",
            Some(
                json!({"products": [{"provider_id":"copernicus.dataspace.odata","product_id":"00000000-0000-4000-8000-000000000011","display_name":"S2_FIXTURE_VALID.SAFE","source_kind":"sentinel2","product_kind":"optical_scene","acquisition_datetime":"2026-01-01T10:00:00Z","footprint_geojson":{"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},"assets":[{"asset_key":"archive","stable_asset_key":"stable","role":"primary","native_roles":["archive"]}],"properties":{"product_type":"S2MSI2A","online":true},"payload":{"runtime_url":"http://example.invalid"}}]}),
            ),
            false,
        ),
    ];
    for (label, patch, input_cursor) in cases {
        let database = TempStore::new(label);
        let mut response_value = serde_json::to_value(valid_response()).unwrap();
        if let Some(patch) = patch {
            merge_object(
                response_value.as_object_mut().unwrap(),
                patch.as_object().unwrap(),
            );
        }
        let response = serde_json::from_value(response_value).unwrap();
        let service = fixture_service(&database, Arc::new(StaticCatalog { response }));
        let mut operation = request(label, &format!("{label}-batch"));
        if input_cursor {
            operation.search.cursor = Some("forbidden".to_owned());
        }
        assert!(
            service
                .catalog_search_and_freeze_legacy(&operation)
                .is_err(),
            "{label}"
        );
        assert_empty_store(&service);
        service.shutdown().unwrap();
    }
}

#[test]
fn catalogue_freeze_revalidates_exact_archive_identity_before_persistence() {
    assert_catalogue_response_rejected(
        "non-uuid-product-id",
        complete_response(vec![exact_archive_product("not-an-odata-uuid")]),
    );
    assert_catalogue_response_rejected(
        "malformed-uuid-spelling",
        complete_response(vec![exact_archive_product(
            "{00000000-0000-4000-8000-000000000011}",
        )]),
    );

    let database = TempStore::new("uppercase-authoritative-uuid");
    let uppercase_uuid = "00000000-0000-4000-8000-0000000000AB";
    let service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: complete_response(vec![exact_archive_product(uppercase_uuid)]),
        }),
    );
    let snapshot = service
        .catalog_search_and_freeze_legacy(&request(
            "uppercase-authoritative-uuid",
            "uppercase-authoritative-uuid-batch",
        ))
        .unwrap();
    assert_eq!(
        snapshot.restore.workbench_batch.unwrap().candidates[0].metadata["provider_product_id"],
        uppercase_uuid
    );
    service.shutdown().unwrap();

    let mut mismatched_stable_key = exact_archive_product("00000000-0000-4000-8000-000000000011");
    mismatched_stable_key.assets[0].stable_asset_key = Some("attacker-chosen-key".to_owned());
    assert_catalogue_response_rejected(
        "mismatched-stable-key",
        complete_response(vec![mismatched_stable_key]),
    );

    let mut wrong_asset_key = exact_archive_product("00000000-0000-4000-8000-000000000011");
    wrong_asset_key.assets[0].asset_key = Some("data".to_owned());
    let product_key = provider_product_key(
        "copernicus.dataspace.odata",
        "SENTINEL-2",
        &wrong_asset_key.product_id,
    )
    .unwrap();
    wrong_asset_key.assets[0].stable_asset_key =
        Some(stable_asset_key(&product_key, "data").unwrap());
    assert_catalogue_response_rejected("wrong-asset-key", complete_response(vec![wrong_asset_key]));

    let mut extra_asset = exact_archive_product("00000000-0000-4000-8000-000000000011");
    let mut metadata_asset = extra_asset.assets[0].clone();
    metadata_asset.asset_key = Some("metadata".to_owned());
    let product_key = provider_product_key(
        "copernicus.dataspace.odata",
        "SENTINEL-2",
        &extra_asset.product_id,
    )
    .unwrap();
    metadata_asset.stable_asset_key = Some(stable_asset_key(&product_key, "metadata").unwrap());
    metadata_asset.role = AssetRole::Metadata;
    metadata_asset.native_roles = vec!["metadata".to_owned()];
    extra_asset.assets.push(metadata_asset);
    assert_catalogue_response_rejected("extra-asset", complete_response(vec![extra_asset]));

    let mut wrong_role = exact_archive_product("00000000-0000-4000-8000-000000000011");
    wrong_role.assets[0].role = AssetRole::Metadata;
    assert_catalogue_response_rejected("wrong-asset-role", complete_response(vec![wrong_role]));

    let mut open_native_role = exact_archive_product("00000000-0000-4000-8000-000000000011");
    open_native_role.assets[0]
        .native_roles
        .push("metadata".to_owned());
    assert_catalogue_response_rejected(
        "open-native-role",
        complete_response(vec![open_native_role]),
    );

    let mut media_type = exact_archive_product("00000000-0000-4000-8000-000000000011");
    media_type.assets[0].media_type = Some("application/zip".to_owned());
    assert_catalogue_response_rejected("media-type", complete_response(vec![media_type]));
}

#[test]
fn catalogue_freeze_uses_real_canonical_utc_and_preserves_fractional_precision() {
    let mut invalid_calendar = exact_archive_product("00000000-0000-4000-8000-000000000011");
    invalid_calendar.acquisition_datetime = Some("9999-99-99T99:99:99Z".to_owned());
    assert_catalogue_response_rejected(
        "invalid-calendar",
        complete_response(vec![invalid_calendar]),
    );

    let mut noncanonical_utc = exact_archive_product("00000000-0000-4000-8000-000000000011");
    noncanonical_utc.acquisition_datetime = Some("2026-01-01T10:00:00.1200Z".to_owned());
    assert_catalogue_response_rejected(
        "noncanonical-utc",
        complete_response(vec![noncanonical_utc]),
    );

    let mut non_utc_offset = exact_archive_product("00000000-0000-4000-8000-000000000011");
    non_utc_offset.acquisition_datetime = Some("2026-01-01T11:00:00+01:00".to_owned());
    assert_catalogue_response_rejected("non-utc-offset", complete_response(vec![non_utc_offset]));

    let mut end_before_start = exact_archive_product("00000000-0000-4000-8000-000000000011");
    end_before_start.properties.insert(
        "content_date_end".to_owned(),
        Value::String("2026-01-01T09:59:59Z".to_owned()),
    );
    assert_catalogue_response_rejected(
        "end-before-start",
        complete_response(vec![end_before_start]),
    );

    let database = TempStore::new("fractional-utc");
    let mut fractional = exact_archive_product("00000000-0000-4000-8000-000000000011");
    fractional.acquisition_datetime = Some("2026-01-01T10:00:00.120Z".to_owned());
    fractional.properties.insert(
        "content_date_end".to_owned(),
        Value::String("2026-01-01T10:00:00.121Z".to_owned()),
    );
    let service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: complete_response(vec![fractional]),
        }),
    );
    let snapshot = service
        .catalog_search_and_freeze_legacy(&request("fractional-utc", "fractional-utc-batch"))
        .unwrap();
    let candidate = &snapshot.restore.workbench_batch.unwrap().candidates[0];
    assert_eq!(
        candidate.acquisition_datetime.as_deref(),
        Some("2026-01-01T10:00:00.120Z")
    );
    assert_eq!(
        candidate.metadata["properties"]["content_date_end"],
        Value::String("2026-01-01T10:00:00.121Z".to_owned())
    );
    service.shutdown().unwrap();
}

#[test]
fn catalogue_freeze_preserves_missing_footprint_and_rejects_bbox_substitution() {
    let database = TempStore::new("missing-footprint");
    let mut missing_footprint = exact_archive_product("00000000-0000-4000-8000-000000000011");
    missing_footprint.footprint_geojson = None;
    let service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: complete_response(vec![missing_footprint]),
        }),
    );
    let snapshot = service
        .catalog_search_and_freeze_legacy(&request("missing-footprint", "missing-footprint-batch"))
        .unwrap();
    assert!(snapshot.restore.workbench_batch.unwrap().candidates[0]
        .footprint_geojson
        .is_none());
    service.shutdown().unwrap();

    let mut bbox_substitution = exact_archive_product("00000000-0000-4000-8000-000000000011");
    bbox_substitution.footprint_geojson = None;
    bbox_substitution.footprint_bbox = Some(BBox {
        west: 110.0,
        east: 111.0,
        south: 30.0,
        north: 31.0,
        crs: "EPSG:4326".to_owned(),
    });
    assert_catalogue_response_rejected(
        "bbox-substitution",
        complete_response(vec![bbox_substitution]),
    );
}

#[test]
fn catalogue_freeze_rejects_locator_like_values_and_orders_closed_checksums() {
    let mut locator_display_name = exact_archive_product("00000000-0000-4000-8000-000000000011");
    locator_display_name.display_name = "https://example.invalid/value".to_owned();
    assert_catalogue_response_rejected(
        "locator-display-name",
        complete_response(vec![locator_display_name]),
    );

    let mut account_display_name = exact_archive_product("00000000-0000-4000-8000-000000000011");
    account_display_name.display_name = "S2_ACCOUNT_TOKEN".to_owned();
    assert_catalogue_response_rejected(
        "account-display-name",
        complete_response(vec![account_display_name]),
    );

    let mut path_display_name = exact_archive_product("00000000-0000-4000-8000-000000000011");
    path_display_name.display_name = "S2_relative_path".to_owned();
    assert_catalogue_response_rejected(
        "path-display-name",
        complete_response(vec![path_display_name]),
    );

    let mut checksum_url = exact_archive_product("00000000-0000-4000-8000-000000000011");
    checksum_url.properties.insert(
        "checksums".to_owned(),
        json!([{"algorithm":"MD5","value":"https://example.invalid/value"}]),
    );
    assert_catalogue_response_rejected("checksum-url", complete_response(vec![checksum_url]));

    let database = TempStore::new("closed-checksums");
    let mut checksums = exact_archive_product("00000000-0000-4000-8000-000000000011");
    checksums.properties.insert(
        "checksums".to_owned(),
        json!([
            {"algorithm":"SHA256","value":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            {"algorithm":"MD5","value":"00000000000000000000000000000000"}
        ]),
    );
    let service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: complete_response(vec![checksums]),
        }),
    );
    let snapshot = service
        .catalog_search_and_freeze_legacy(&request("closed-checksums", "closed-checksums-batch"))
        .unwrap();
    assert_eq!(
        snapshot.restore.workbench_batch.unwrap().candidates[0].metadata["properties"]["checksums"],
        json!([
            {"algorithm":"MD5","value":"00000000000000000000000000000000"},
            {"algorithm":"SHA256","value":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
        ])
    );
    service.shutdown().unwrap();
}

#[test]
fn catalogue_freeze_enforces_the_injected_candidate_bound_and_rejects_duplicate_diagnostics() {
    let products = (1..=501)
        .map(|index| exact_archive_product(&format!("00000000-0000-4000-8000-{index:012}")))
        .collect();
    assert_catalogue_response_rejected("too-many-products", complete_response(products));

    let mut exact_duplicate = complete_response(vec![exact_archive_product(
        "00000000-0000-4000-8000-000000000011",
    )]);
    exact_duplicate.stats.matched_count = Some(2);
    exact_duplicate.stats.fetched_record_count = 2;
    exact_duplicate.stats.mapped_record_count = 2;
    exact_duplicate.stats.exact_duplicate_record_count = 1;
    exact_duplicate.stats.unique_provider_product_count = 1;
    exact_duplicate.stats.returned_count = 1;
    assert_catalogue_response_rejected("exact-duplicate", exact_duplicate);
}

#[test]
fn catalogue_freeze_persists_explicit_profile_and_identity_rule_metadata() {
    let database = TempStore::new("catalogue-metadata");
    let service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
    );
    let snapshot = service
        .catalog_search_and_freeze_legacy(&request(
            "catalogue-metadata",
            "catalogue-metadata-batch",
        ))
        .unwrap();
    let metadata = &snapshot.restore.workbench_batch.unwrap().candidates[0].metadata;
    assert_eq!(metadata["logical_dataset"], "optical.sentinel-2.l2a");
    assert_eq!(metadata["native_collection_id"], "SENTINEL-2");
    assert_eq!(
        metadata["collection_profile_id"],
        "cdse.sentinel-2.l2a.collection.v1"
    );
    assert_eq!(
        metadata["identity_rule_id"],
        "cdse.sentinel-2.l2a.identity.v1"
    );
    assert_eq!(
        metadata["filename_rule_id"],
        "cdse.sentinel-2.l2a.archive-filename.v1"
    );
    assert_eq!(metadata["delivery_model"], "archive_first");
    service.shutdown().unwrap();
}

#[test]
fn exhaustive_failure_preserves_the_previous_active_workbench() {
    let database = TempStore::new("all-failure-preserves-active");
    let first_service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
    );
    let first = request("all-preserve-first", "all-preserve-first-batch");
    first_service
        .catalog_search_and_freeze_legacy(&first)
        .expect("finite baseline batch is active");
    let before = first_service.snapshot(true).unwrap().restore;
    let before_workbench =
        serde_json::to_value((&before.workbench, &before.workbench_batch)).unwrap();
    first_service.shutdown().unwrap();

    let failing_service = fixture_service(&database, Arc::new(FailingCatalog));
    let mut all = request("all-preserve-failing", "all-preserve-failing-batch");
    all.search.result_limit = None;
    let error = failing_service
        .catalog_search_and_freeze_legacy(&all)
        .expect_err("ALL protection must fail without partial publication");

    assert_eq!(error.code(), "CATALOGUE_RESULT_LIMIT_EXCEEDED");
    let after = failing_service.snapshot(true).unwrap().restore;
    assert_eq!(
        serde_json::to_value((&after.workbench, &after.workbench_batch)).unwrap(),
        before_workbench
    );
    failing_service.shutdown().unwrap();
}

#[test]
fn exhaustive_cancellation_preserves_the_previous_active_workbench() {
    let database = TempStore::new("all-cancel-preserves-active");
    let first_service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
    );
    first_service
        .catalog_search_and_freeze_legacy(&request("all-cancel-first", "all-cancel-first-batch"))
        .expect("finite baseline batch is active");
    let before = first_service.snapshot(true).unwrap().restore;
    let before_workbench =
        serde_json::to_value((&before.workbench, &before.workbench_batch)).unwrap();
    first_service.shutdown().unwrap();

    let catalogue = Arc::new(BlockingCatalog {
        response: valid_response(),
        entered: AtomicBool::new(false),
        release: AtomicBool::new(false),
    });
    let service = Arc::new(fixture_service(&database, catalogue.clone()));
    let mut all = request("all-cancel-failing", "all-cancel-failing-batch");
    all.search.result_limit = None;
    let worker = {
        let service = Arc::clone(&service);
        thread::spawn(move || service.catalog_search_and_freeze_legacy(&all))
    };
    while !catalogue.entered.load(Ordering::Acquire) {
        thread::yield_now();
    }
    assert!(service.cancel_request("all-cancel-failing").unwrap());
    let error = worker.join().unwrap().unwrap_err();

    assert_eq!(error.code(), "CATALOGUE_REQUEST_CANCELLED");
    let after = service.snapshot(true).unwrap().restore;
    assert_eq!(
        serde_json::to_value((&after.workbench, &after.workbench_batch)).unwrap(),
        before_workbench
    );
    service.shutdown().unwrap();
}

#[test]
fn duplicate_identity_profile_mismatch_and_store_already_exists_preserve_existing_batch() {
    let database = TempStore::new("existing-batch");
    let service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: valid_response(),
        }),
    );
    let first = request("existing-first", "existing-batch");
    let before = service.catalog_search_and_freeze_legacy(&first).unwrap();
    let before_value = serde_json::to_value(&before.restore).unwrap();

    let mut second = request("existing-second", "existing-batch");
    second.aoi_name = "Changed AOI name".to_owned();
    assert!(service.catalog_search_and_freeze_legacy(&second).is_err());
    assert_eq!(
        serde_json::to_value(&service.snapshot(true).unwrap().restore).unwrap(),
        before_value
    );
    service.shutdown().unwrap();

    let database = TempStore::new("duplicate-and-profile");
    let mut duplicate = valid_response();
    duplicate.products.push(duplicate.products[0].clone());
    duplicate.stats.matched_count = Some(2);
    duplicate.stats.fetched_record_count = 2;
    duplicate.stats.mapped_record_count = 2;
    duplicate.stats.unique_provider_product_count = 2;
    duplicate.stats.returned_count = 2;
    let service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: duplicate,
        }),
    );
    assert!(service
        .catalog_search_and_freeze_legacy(&request("duplicate-product", "duplicate-product-batch"))
        .is_err());
    assert_empty_store(&service);
    service.shutdown().unwrap();

    let database = TempStore::new("profile-mismatch");
    let mut profile_mismatch = valid_response();
    profile_mismatch.collection.collection_profile_id = "wrong.profile".to_owned();
    let service = fixture_service(
        &database,
        Arc::new(StaticCatalog {
            response: profile_mismatch,
        }),
    );
    assert!(service
        .catalog_search_and_freeze_legacy(&request("profile-mismatch", "profile-mismatch-batch"))
        .is_err());
    assert_empty_store(&service);
    service.shutdown().unwrap();
}

fn merge_object(target: &mut Map<String, Value>, patch: &Map<String, Value>) {
    for (key, value) in patch {
        target.insert(key.clone(), value.clone());
    }
}
