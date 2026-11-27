use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    LiveReviewedMpcAssetEvidence, ReviewedMpcCreateAssetTaskRequest,
};
use insar_core::{
    provider_product_key, stable_asset_key, AsfRemoteSearchRequest, AsfRemoteSearchResponse,
    AssetRole, CatalogCollectionDescriptor, CatalogDeliveryModel, CatalogSearchQuery,
    CatalogSearchResponse, CatalogSearchStats, ContractError, DataSourceKind, DownloadItemProgress,
    DownloadItemResult, DownloadLayout, DownloadOutcome, ProductKind, ProviderAsset,
    ProviderProduct, ProviderResult,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::catalog::{
    MPC_COPDEM_GLO30_COLLECTION_ID, MPC_COPDEM_GLO30_FILENAME_RULE_ID,
    MPC_COPDEM_GLO30_LOGICAL_DATASET, MPC_COPDEM_GLO30_PROFILE_ID,
    MPC_REVIEWED_STAC_IDENTITY_RULE_ID, MPC_SENTINEL1_GRD_COLLECTION_ID,
    MPC_SENTINEL1_GRD_FILENAME_RULE_ID, MPC_SENTINEL1_GRD_LOGICAL_DATASET,
    MPC_SENTINEL1_GRD_PROFILE_ID, PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use insar_provider::planetary_computer_asset::{
    PlanetaryComputerResolvedAssetEvidence, PlanetaryComputerReviewedAssetClass,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

const S1_ID: &str = "S1A_IW_GRDH_1SDV_20250123T215225_20250123T215254_057577_0717D2";
const S1_VV_HREF: &str = "https://sentinel1euwest.blob.core.windows.net/s1-grd/GRD/2025/1/23/IW/DV/S1A_IW_GRDH_1SDV_20250123T215225_20250123T215254_057577_0717D2_887E/measurement/iw-vv.tiff";
const DEM_ID: &str = "Copernicus_DSM_COG_10_N25_00_E121_00_DEM";
const DEM_HREF: &str = "https://elevationeuwest.blob.core.windows.net/copernicus-dem/COP30_hh/Copernicus_DSM_COG_10_N25_00_E121_00_DEM.tif";

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-reviewed-mpc-freeze-{label}-{}-{nonce}.sqlite3",
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

struct FixtureWorker;

impl DownloadWorker for FixtureWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root.join(work.item.display_name),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-09-20T00:00:03Z".to_owned(),
        }
    }
}

#[derive(Clone)]
struct FixtureCatalog {
    response: CatalogSearchResponse,
}

impl LiveCatalogSearch for FixtureCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        Ok(self.response.clone())
    }
}

struct FixtureEvidence {
    class: PlanetaryComputerReviewedAssetClass,
    href: &'static str,
    file_name: &'static str,
    size: u64,
}

impl LiveReviewedMpcAssetEvidence for FixtureEvidence {
    fn resolve_reviewed_asset(
        &self,
        class: PlanetaryComputerReviewedAssetClass,
        _provider_product_id: &str,
        _asset_key: &str,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerResolvedAssetEvidence, ContractError> {
        assert_eq!(class, self.class);
        Ok(PlanetaryComputerResolvedAssetEvidence {
            stable_href: self.href.to_owned(),
            file_name: self.file_name.to_owned(),
            expected_size_bytes: self.size,
        })
    }
}

fn aoi() -> Map<String, Value> {
    json!({
        "type":"Polygon",
        "coordinates":[[
            [121.3,24.9],
            [121.7,24.9],
            [121.7,25.3],
            [121.3,25.3],
            [121.3,24.9]
        ]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn fixture_case(
    class: PlanetaryComputerReviewedAssetClass,
) -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    DataSourceKind,
    ProductKind,
) {
    match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => (
            MPC_SENTINEL1_GRD_LOGICAL_DATASET,
            MPC_SENTINEL1_GRD_COLLECTION_ID,
            MPC_SENTINEL1_GRD_PROFILE_ID,
            MPC_SENTINEL1_GRD_FILENAME_RULE_ID,
            S1_ID,
            DataSourceKind::Sentinel1,
            ProductKind::SarGrd,
        ),
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => (
            MPC_COPDEM_GLO30_LOGICAL_DATASET,
            MPC_COPDEM_GLO30_COLLECTION_ID,
            MPC_COPDEM_GLO30_PROFILE_ID,
            MPC_COPDEM_GLO30_FILENAME_RULE_ID,
            DEM_ID,
            DataSourceKind::Dem,
            ProductKind::DemRaster,
        ),
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
            panic!("Sentinel-2 bands require the dedicated catalog_search_freeze delivery fixture")
        }
    }
}

fn product(class: PlanetaryComputerReviewedAssetClass) -> ProviderProduct {
    let (dataset, collection, _, _, product_id, source_kind, product_kind) = fixture_case(class);
    let product_key = provider_product_key(
        PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
        collection,
        product_id,
    )
    .unwrap();
    let (asset_key, href, file_name) = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => ("vv", S1_VV_HREF, "iw-vv.tiff"),
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => (
            "data",
            DEM_HREF,
            "Copernicus_DSM_COG_10_N25_00_E121_00_DEM.tif",
        ),
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
            panic!("Sentinel-2 band delivery is not a generic reviewed catalogue product")
        }
    };
    let _ = dataset;
    ProviderProduct {
        provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
        product_id: product_id.to_owned(),
        display_name: product_id.to_owned(),
        source_kind,
        product_kind,
        acquisition_datetime: matches!(class, PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog)
            .then_some("2025-01-23T21:52:40Z".to_owned()),
        footprint_geojson: Some(aoi()),
        footprint_bbox: None,
        assets: vec![ProviderAsset {
            asset_key: Some(asset_key.to_owned()),
            stable_asset_key: Some(stable_asset_key(&product_key, asset_key).unwrap()),
            access_descriptor: None,
            role: AssetRole::Primary,
            native_roles: vec!["data".to_owned()],
            url: Some(href.to_owned()),
            file_name: Some(file_name.to_owned()),
            media_type: Some("image/tiff; application=geotiff; profile=cloud-optimized".to_owned()),
            size_bytes: None,
        }],
        properties: Map::new(),
        payload: Map::new(),
    }
}

fn search(class: PlanetaryComputerReviewedAssetClass) -> CatalogSearchQuery {
    let (dataset, collection, _, _, _, _, _) = fixture_case(class);
    CatalogSearchQuery {
        logical_dataset: dataset.to_owned(),
        provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
        native_collection_id: collection.to_owned(),
        aoi_geojson: Some(aoi()),
        cursor: None,
        result_limit: Some(1),
        filters: Map::new(),
    }
}

fn response(class: PlanetaryComputerReviewedAssetClass) -> CatalogSearchResponse {
    let (dataset, collection, profile, filename_rule, _, _, _) = fixture_case(class);
    CatalogSearchResponse {
        collection: CatalogCollectionDescriptor {
            logical_dataset: dataset.to_owned(),
            provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
            native_collection_id: collection.to_owned(),
            collection_profile_id: profile.to_owned(),
            identity_rule_id: MPC_REVIEWED_STAC_IDENTITY_RULE_ID.to_owned(),
            filename_rule_id: filename_rule.to_owned(),
            delivery_model: CatalogDeliveryModel::ArchiveFirst,
            production_enabled: true,
        },
        products: vec![product(class)],
        stats: CatalogSearchStats {
            requested_limit: Some(1),
            matched_count: None,
            request_count: 1,
            transport_part_count: 1,
            fetched_record_count: 1,
            mapped_record_count: 1,
            profile_filtered_record_count: 0,
            exact_duplicate_record_count: 0,
            unique_provider_product_count: 1,
            exact_matched_count: 1,
            returned_count: 1,
            remote_exhausted: true,
            working_set_satisfied: true,
            elapsed_ms: 1,
            truncated: false,
        },
        next_cursor: None,
        item_errors: Vec::new(),
    }
}

fn service(database: &TempStore, class: PlanetaryComputerReviewedAssetClass) -> ApplicationService {
    ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(FixtureCatalog {
            response: response(class),
        }),
        Arc::new(FixtureWorker),
    )
}

fn run_case(class: PlanetaryComputerReviewedAssetClass) {
    let database = TempStore::new(match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => "s1",
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => "cop30",
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
            panic!("Sentinel-2 bands must not acquire generic catalogue execution authority")
        }
    });
    let service = service(&database, class);
    let (dataset, collection, _, _, product_id, _, _) = fixture_case(class);
    let search = search(class);
    let batch_id = format!("reviewed-freeze-{}", class.collection_id());
    let frozen = service
        .catalog_search_and_freeze_legacy(&ApplicationCatalogSearchAndFreezeRequest {
            request_id: format!("reviewed-search-{}", class.collection_id()),
            search: search.clone(),
            batch_id: batch_id.clone(),
            created_at: "2026-09-20T00:00:00Z".to_owned(),
            aoi_name: "Reviewed MPC AOI".to_owned(),
            aoi_geojson: search.aoi_geojson.clone(),
            workbench_page_size: 24,
        })
        .unwrap();
    let batch = frozen.restore.workbench_batch.unwrap();
    assert_eq!(batch.batch_id, batch_id);
    assert_eq!(batch.provider_id, PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID);
    assert_eq!(batch.candidates.len(), 1);
    let item = &batch.candidates[0];
    assert_eq!(item.metadata["logical_dataset"], dataset);
    assert_eq!(item.metadata["native_collection_id"], collection);
    assert_eq!(item.metadata["provider_product_id"], product_id);
    assert!(item.asset_access.is_none());
    assert!(item.execution_policy.is_none());
    let frozen_json = serde_json::to_string(item).unwrap();
    assert!(!frozen_json.contains("http"));
    assert!(!frozen_json.contains("sig="));
    assert!(!frozen_json.contains("access_descriptor"));

    let (asset_key, href, file_name, size) = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => {
            ("vv", S1_VV_HREF, "iw-vv.tiff", 648_045_257)
        }
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => (
            "data",
            DEM_HREF,
            "Copernicus_DSM_COG_10_N25_00_E121_00_DEM.tif",
            7_594_159,
        ),
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
            panic!("Sentinel-2 band delivery uses its dedicated evidence boundary")
        }
    };
    service
        .create_reviewed_mpc_asset_task_with_evidence_for_test(
            &ReviewedMpcCreateAssetTaskRequest {
                request_id: format!("reviewed-delivery-{}", class.collection_id()),
                source_batch_id: batch_id,
                item_id: item.item_id.clone(),
                asset_keys: vec![asset_key.to_owned()],
                task_id: format!("reviewed-task-{}", class.collection_id()),
                output_root: std::env::temp_dir().join("reviewed-mpc-freeze-output"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-09-20T00:00:01Z".to_owned(),
            },
            &FixtureEvidence {
                class,
                href,
                file_name,
                size,
            },
        )
        .unwrap();

    let store = TaskStore::open(&database.0).unwrap();
    let task = store
        .load_task_record(&format!("reviewed-task-{}", class.collection_id()))
        .unwrap()
        .expect("derived reviewed task persists");
    let delivery = store
        .load_search_batch(&task.snapshot.batch_id)
        .unwrap()
        .unwrap();
    assert_eq!(delivery.candidates.len(), 1);
    let delivery_item = &delivery.candidates[0];
    assert!(delivery_item.asset_access.is_some());
    assert!(delivery_item.execution_policy.is_some());
    let delivery_json = serde_json::to_string(delivery_item).unwrap();
    assert!(delivery_json.contains(href));
    assert!(!delivery_json.contains("sig="));

    service.shutdown().unwrap();
}

#[test]
fn production_search_freeze_then_delivery_derivation_is_closed_for_sentinel1_grd() {
    run_case(PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog);
}

#[test]
fn production_search_freeze_then_delivery_derivation_is_closed_for_cop30() {
    run_case(PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog);
}
