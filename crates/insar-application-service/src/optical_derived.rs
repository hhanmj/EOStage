use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use insar_core::{
    validate_debug_context, ContractError, DownloadLayout, FrozenSearchBatch,
    ProviderNetworkPolicy, ProxyPolicy, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest,
};
use insar_gis::{
    precheck_aoi_scene_coverage, redact_source_locator, validate_raw_optical_cog_source,
    OpticalBandAssetInput, OpticalCloudMaskPolicy, OpticalDerivedMode, OpticalDerivedRequest,
    OpticalMosaicStrategy, OpticalPrecheckReport, OpticalPrecheckRequest, OpticalResampling,
    OpticalSceneInput, OpticalSourceKind,
};
use insar_provider::catalog::{
    validate_earth_search_sentinel2_scl_url, EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
    EARTH_SEARCH_STAC_ENDPOINT, LANDSAT_C2_L2_LOGICAL_DATASET, LANDSAT_C2_L2_PROVIDER_ID,
    MPC_HLS_L30_COLLECTION_ID, MPC_HLS_L30_LOGICAL_DATASET, MPC_HLS_S30_COLLECTION_ID,
    MPC_HLS_S30_LOGICAL_DATASET, MPC_LANDSAT_COLLECTION_ID, MPC_LANDSAT_PROVIDER_ID,
};
use insar_provider::planetary_computer::{
    PlanetaryComputerLandsatAsset, PlanetaryComputerLandsatClient, PLANETARY_COMPUTER_STAC_ORIGIN,
};
use insar_provider::planetary_computer_asset::{
    sentinel2_l2a_reflectance_band_resolution, validate_planetary_computer_stable_asset,
    PlanetaryComputerAssetSigner, PlanetaryComputerReviewedAssetClass,
    MPC_SENTINEL2_L2A_BAND_LOGICAL_DATASET, PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{
    frozen_landsat_scene_evidence, frozen_sentinel2_analysis_source, network_routes,
    parse_normalized_utc, valid_catalogue_display_name, valid_catalogue_opaque_id,
    ApplicationNetworkSettings, ApplicationService, ApplicationServiceError, CoreServiceSnapshot,
    LiveSentinel2BandEvidence, Sentinel2BandEvidenceRuntime,
};

const MAX_STAC_ITEM_JSON_BYTES: u64 = 2 * 1024 * 1024;
const STAC_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const SENTINEL2_L2A_COLLECTION_ID: &str = "sentinel-2-l2a";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OpticalDerivedPrecheckBatchRequest {
    pub source_batch_id: String,
    #[serde(default)]
    pub expected_workbench_revision: u64,
    #[serde(default)]
    pub item_ids: Vec<String>,
    #[serde(default)]
    pub mode: Option<OpticalDerivedMode>,
    #[serde(default)]
    pub aoi_bbox_wgs84: Option<[f64; 4]>,
    #[serde(default)]
    pub aoi_rings_wgs84: Option<Vec<Vec<[f64; 2]>>>,
    #[serde(default)]
    pub target_resolution_m: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OpticalDerivedCreateTaskRequest {
    pub request_id: String,
    pub task_id: String,
    pub source_batch_id: String,
    #[serde(default)]
    pub expected_workbench_revision: u64,
    #[serde(default)]
    pub item_ids: Vec<String>,
    pub mode: OpticalDerivedMode,
    #[serde(default)]
    pub aoi_bbox_wgs84: Option<[f64; 4]>,
    #[serde(default)]
    pub aoi_rings_wgs84: Option<Vec<Vec<[f64; 2]>>>,
    pub band_order: Vec<String>,
    #[serde(default)]
    pub target_crs: Option<String>,
    #[serde(default)]
    pub target_resolution_m: Option<f64>,
    #[serde(default)]
    pub resampling: OpticalResampling,
    #[serde(default)]
    pub cloud_mask: OpticalCloudMaskPolicy,
    #[serde(default)]
    pub mosaic_strategy: OpticalMosaicStrategy,
    #[serde(default)]
    pub max_temporal_span_days: Option<f64>,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default = "default_true")]
    pub write_lineage_sidecar: bool,
    #[serde(default = "default_true")]
    pub write_provenance_sidecars: bool,
    pub output_root: PathBuf,
    pub concurrency: u64,
    pub created_at: String,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct OpticalDerivedTaskCreatedResult {
    pub precheck: OpticalPrecheckReport,
    pub task_id: String,
    pub derived_batch_id: String,
    pub derived_item_id: String,
    pub output_tif_path: PathBuf,
    #[serde(default)]
    pub output_tif_paths: Vec<PathBuf>,
    pub snapshot: CoreServiceSnapshot,
}

pub fn provider_network_policy_from_settings(
    settings: &ApplicationNetworkSettings,
) -> ProviderNetworkPolicy {
    network_routes::policy_for_settings(network_routes::ServiceRoute::Ordinary, settings)
}

pub fn read_network_policy_for_store_path(store_path: &Path) -> ProviderNetworkPolicy {
    let settings_path = store_path.with_file_name("network-settings.json");
    let bytes = match std::fs::read(&settings_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return network_routes::default_policy(network_routes::ServiceRoute::Ordinary);
        }
        Err(_) => {
            eprintln!("[optical_derived] network settings read failed; using default policy");
            return network_routes::default_policy(network_routes::ServiceRoute::Ordinary);
        }
    };
    if bytes.len() > 64 * 1024 {
        eprintln!("[optical_derived] network settings exceeded size limit; using default policy");
        return network_routes::default_policy(network_routes::ServiceRoute::Ordinary);
    }
    match serde_json::from_slice::<ApplicationNetworkSettings>(&bytes) {
        Ok(settings) => {
            if let Err(err) = crate::validate_network_settings(&settings) {
                eprintln!(
                    "[optical_derived] network settings validation failed; using default policy: code={}",
                    err.code()
                );
                return network_routes::default_policy(network_routes::ServiceRoute::Ordinary);
            }
            provider_network_policy_from_settings(&settings)
        }
        Err(_) => {
            eprintln!("[optical_derived] network settings JSON invalid; using default policy");
            network_routes::default_policy(network_routes::ServiceRoute::Ordinary)
        }
    }
}

pub fn sign_mpc_optical_request_in_memory(
    request: &mut OpticalDerivedRequest,
    network_policy: ProviderNetworkPolicy,
    interrupted: &(dyn Fn() -> bool + Sync),
) -> Result<(), String> {
    let mut s2_signer: Option<PlanetaryComputerAssetSigner> = None;
    let mut mpc_client: Option<PlanetaryComputerLandsatClient> = None;

    for scene in &mut request.scenes {
        for asset in scene
            .band_assets
            .iter_mut()
            .chain(scene.qa_asset.as_mut())
            .chain(scene.secondary_qa_asset.as_mut())
        {
            sign_single_asset_in_memory(
                scene.source_kind,
                &scene.collection,
                asset,
                network_policy,
                &mut s2_signer,
                &mut mpc_client,
                interrupted,
            )?;
        }
    }
    Ok(())
}

fn sign_single_asset_in_memory(
    source_kind: OpticalSourceKind,
    collection: &str,
    asset: &mut OpticalBandAssetInput,
    network_policy: ProviderNetworkPolicy,
    s2_signer: &mut Option<PlanetaryComputerAssetSigner>,
    mpc_client: &mut Option<PlanetaryComputerLandsatClient>,
    interrupted: &(dyn Fn() -> bool + Sync),
) -> Result<(), String> {
    if interrupted() {
        return Err("OPTICAL_DERIVED_CANCELLED: operation cancelled during signing".to_owned());
    }
    let href = asset.stable_href.trim();
    if href.is_empty() {
        return Err(format!(
            "OPTICAL_RAW_COG_MISSING: band {} has empty href",
            asset.band_id
        ));
    }
    let Ok(parsed) = Url::parse(href) else {
        // Local filesystem path — no SAS signing required.
        asset.resolved_href = None;
        return Ok(());
    };
    if parsed.scheme() == "file" {
        asset.resolved_href = None;
        return Ok(());
    }
    let Some(host) = parsed.host_str() else {
        return Err(format!(
            "OPTICAL_RAW_COG_URI_INVALID: missing host for band {}",
            asset.band_id
        ));
    };

    // Never persist or double-sign loopback / non-MPC fixtures.
    if !host.ends_with(".blob.core.windows.net") {
        asset.resolved_href = None;
        return Ok(());
    }

    match source_kind {
        OpticalSourceKind::Sentinel2L2a => {
            if s2_signer.is_none() {
                *s2_signer = Some(
                    PlanetaryComputerAssetSigner::new(network_policy)
                        .map_err(|e| format!("OPTICAL_MPC_SIGN_INIT_FAILED: {e}"))?,
                );
            }
            let signed = s2_signer
                .as_ref()
                .expect("initialized above")
                .sign_reviewed_asset(
                    PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog,
                    href,
                    interrupted,
                )
                .map_err(|e| format!("OPTICAL_MPC_SIGN_FAILED: {e}"))?;
            let (url, _) = signed.into_parts();
            asset.resolved_href = Some(url.to_string());
        }
        OpticalSourceKind::LandsatC2L2 => {
            if mpc_client.is_none() {
                *mpc_client = Some(
                    PlanetaryComputerLandsatClient::new(network_policy)
                        .map_err(|e| format!("OPTICAL_MPC_SIGN_INIT_FAILED: {e}"))?,
                );
            }
            let signed = mpc_client
                .as_ref()
                .expect("initialized above")
                .sign_asset(href, interrupted)
                .map_err(|e| format!("OPTICAL_MPC_SIGN_FAILED: {e}"))?;
            let (url, _) = signed.into_parts();
            asset.resolved_href = Some(url.to_string());
        }
        OpticalSourceKind::HlsL30 | OpticalSourceKind::HlsS30 => {
            if mpc_client.is_none() {
                *mpc_client = Some(
                    PlanetaryComputerLandsatClient::new(network_policy)
                        .map_err(|e| format!("OPTICAL_MPC_SIGN_INIT_FAILED: {e}"))?,
                );
            }
            let collection_id = match source_kind {
                OpticalSourceKind::HlsL30 => MPC_HLS_L30_COLLECTION_ID,
                OpticalSourceKind::HlsS30 => MPC_HLS_S30_COLLECTION_ID,
                _ => collection,
            };
            let signed = mpc_client
                .as_ref()
                .expect("initialized above")
                .sign_hls_asset(collection_id, href, interrupted)
                .map_err(|e| format!("OPTICAL_MPC_SIGN_FAILED: {e}"))?;
            let (url, _) = signed.into_parts();
            asset.resolved_href = Some(url.to_string());
        }
        OpticalSourceKind::Sentinel2L1c | OpticalSourceKind::OpticalRawCog => {
            // GCS / CDSE L1C URLs or raw external HTTPS COGs require no MPC SAS signing.
            asset.resolved_href = None;
        }
    }
    Ok(())
}

impl ApplicationService {
    pub fn precheck_optical_aoi_coverage(
        &self,
        request: OpticalDerivedPrecheckBatchRequest,
    ) -> Result<OpticalPrecheckReport, ApplicationServiceError> {
        self.ensure_running()?;
        if !valid_catalogue_opaque_id(&request.source_batch_id) {
            return Err(invalid_request(
                "OPTICAL_PRECHECK_INVALID: source_batch_id is invalid",
            ));
        }
        if let Some(res_m) = request.target_resolution_m {
            if !res_m.is_finite() || res_m <= 0.0 {
                return Err(invalid_request(
                    "OPTICAL_PRECHECK_INVALID: target_resolution_m must be finite and positive",
                ));
            }
        }
        let (source_batch, selected_items) = self.load_optical_source_selection(
            &request.source_batch_id,
            request.expected_workbench_revision,
            &request.item_ids,
        )?;
        let (aoi_bbox_wgs84, aoi_rings_wgs84) = resolve_aoi_from_request_or_batch(
            request.aoi_bbox_wgs84,
            request.aoi_rings_wgs84.as_ref(),
            &source_batch,
        )?;

        let mut scenes = Vec::with_capacity(selected_items.len());
        for item in &selected_items {
            let source_kind = classify_item_source_kind(&source_batch, item)?;
            let _ = extract_inline_assets(item)?;
            scenes.push(build_scene_input_stub(
                &source_batch,
                item,
                source_kind,
                Vec::new(),
                None,
                None,
            )?);
        }

        precheck_aoi_scene_coverage(&OpticalPrecheckRequest {
            mode: request.mode,
            aoi_bbox_wgs84,
            aoi_rings_wgs84: Some(aoi_rings_wgs84),
            target_resolution_m: request.target_resolution_m,
            scenes,
        })
        .map_err(|e| invalid_request(e.to_string()))
    }

    pub fn create_optical_derived_task(
        &self,
        request: OpticalDerivedCreateTaskRequest,
    ) -> Result<OpticalDerivedTaskCreatedResult, ApplicationServiceError> {
        self.ensure_running()?;
        validate_create_task_request(&request)?;

        let (source_batch, page_size, selected_items) = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            resolve_source_batch_selection_in_store(
                &store,
                &request.source_batch_id,
                request.expected_workbench_revision,
                &request.item_ids,
            )
        })?;

        let max_scenes = insar_gis::OPTICAL_MOSAIC_MAX_SCENES;
        match request.mode {
            OpticalDerivedMode::SingleSceneClip if selected_items.len() != 1 => {
                return Err(invalid_request(
                    "OPTICAL_DERIVED_SCENE_COUNT_INVALID: single_scene_clip requires exactly 1 scene",
                ));
            }
            OpticalDerivedMode::BatchSingleSceneClip
                if !(1..=max_scenes).contains(&selected_items.len()) =>
            {
                return Err(invalid_request(format!(
                    "OPTICAL_DERIVED_SCENE_COUNT_INVALID: batch_single_scene_clip requires 1..={max_scenes} scenes"
                )));
            }
            OpticalDerivedMode::NearCoincidentSpatialMosaic
                if !(2..=max_scenes).contains(&selected_items.len()) =>
            {
                return Err(invalid_request(format!(
                    "OPTICAL_DERIVED_SCENE_COUNT_INVALID: near_coincident_spatial_mosaic requires 2..={max_scenes} scenes"
                )));
            }
            _ => {}
        }

        let (aoi_bbox_wgs84, aoi_rings_wgs84) = resolve_aoi_from_request_or_batch(
            request.aoi_bbox_wgs84,
            request.aoi_rings_wgs84.as_ref(),
            &source_batch,
        )?;

        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or_else(|| {
                invalid_request("OPTICAL_DERIVED_REQUEST_DUPLICATE: duplicate request_id")
            })?;

        let network_policy = self.network_policy(network_routes::ServiceRoute::Ordinary);
        let result = (|| {
            let mut scenes = Vec::with_capacity(selected_items.len());
            for item in &selected_items {
                if cancelled.load(Ordering::Acquire) {
                    return Err(invalid_request("OPTICAL_DERIVED_CANCELLED: cancelled"));
                }
                let scene = self.resolve_scene_for_execution(
                    &source_batch,
                    item,
                    &request.band_order,
                    request.cloud_mask,
                    network_policy,
                    &|| cancelled.load(Ordering::Acquire),
                )?;
                scenes.push(scene);
            }

            let precheck = precheck_aoi_scene_coverage(&OpticalPrecheckRequest {
                mode: Some(request.mode),
                aoi_bbox_wgs84,
                aoi_rings_wgs84: Some(aoi_rings_wgs84.clone()),
                target_resolution_m: request.target_resolution_m,
                scenes: scenes.clone(),
            })
            .map_err(|e| invalid_request(e.to_string()))?;

            if !precheck.ok {
                let reason = if precheck.blocking_reasons.is_empty() {
                    "OPTICAL_PRECHECK_BLOCKED: precheck failed".to_owned()
                } else {
                    precheck.blocking_reasons.join("; ")
                };
                return Err(invalid_request(reason));
            }

            if request.mode == OpticalDerivedMode::NearCoincidentSpatialMosaic {
                if let Some(span_limit) = request.max_temporal_span_days {
                    if let Some(actual_span) = precheck.temporal_span_days {
                        if actual_span > span_limit {
                            return Err(invalid_request(format!(
                                "OPTICAL_MOSAIC_TEMPORAL_SPAN_EXCEEDED: span {actual_span:.2}d exceeds limit {span_limit:.2}d"
                            )));
                        }
                    }
                }
            }

            let derived_batch_id = format!("optderived-{}", request.task_id);
            if !valid_catalogue_opaque_id(&derived_batch_id) {
                return Err(invalid_request(
                    "OPTICAL_DERIVED_BATCH_ID_INVALID: derived batch identifier is too long",
                ));
            }

            // Ensure zero credentials, SAS tokens, or raw STAC metadata blobs enter SQLite.
            for scene in &mut scenes {
                scene.stac_metadata = None;
                for asset in &mut scene.band_assets {
                    asset.resolved_href = None;
                    let (_, redacted) = redact_source_locator(&asset.stable_href);
                    if redacted > 0 {
                        return Err(invalid_request(
                            "OPTICAL_DERIVED_SECRET_REJECTED: source band locator contains credential query parameters",
                        ));
                    }
                }
                if let Some(qa) = scene.qa_asset.as_mut() {
                    qa.resolved_href = None;
                    let (_, redacted) = redact_source_locator(&qa.stable_href);
                    if redacted > 0 {
                        return Err(invalid_request(
                            "OPTICAL_DERIVED_SECRET_REJECTED: source QA locator contains credential query parameters",
                        ));
                    }
                }
            }

            let delivery_kind = match request.mode {
                OpticalDerivedMode::SingleSceneClip => "optical_aoi_clip_v1",
                OpticalDerivedMode::BatchSingleSceneClip => "optical_aoi_batch_clip_v1",
                OpticalDerivedMode::NearCoincidentSpatialMosaic => "optical_spatial_mosaic_v1",
            };

            let mut derived_items = Vec::new();
            let mut derived_item_ids = Vec::new();
            let mut output_tif_paths = Vec::new();

            if request.mode == OpticalDerivedMode::BatchSingleSceneClip {
                let mut pairs: Vec<(&SnapshotItem, OpticalSceneInput)> = selected_items
                    .iter()
                    .zip(scenes)
                    .enumerate()
                    .filter(|(idx, _)| {
                        precheck
                            .scene_coverages
                            .get(*idx)
                            .map(|p| p.intersects_aoi)
                            .unwrap_or(true)
                    })
                    .map(|(_, pair)| pair)
                    .collect();
                if pairs.is_empty() {
                    return Err(invalid_request(
                        "AOI_SCENE_DISJOINT: 所选光学景均与当前 AOI 不相交",
                    ));
                }
                for (source_item, scene) in pairs.drain(..) {
                    let single_items = [source_item.clone()];
                    let per_filename = build_derived_output_filename_for_mode(
                        &request,
                        OpticalDerivedMode::SingleSceneClip,
                        &single_items,
                    )?;
                    let per_path = request.output_root.join(&per_filename);
                    let per_item_id =
                        build_derived_item_id(&request.task_id, &per_filename, &single_items)?;
                    let per_spec = OpticalDerivedRequest {
                        mode: OpticalDerivedMode::SingleSceneClip,
                        aoi_bbox_wgs84,
                        aoi_rings_wgs84: Some(aoi_rings_wgs84.clone()),
                        band_order: request.band_order.clone(),
                        target_crs: request.target_crs.clone(),
                        target_resolution_m: request.target_resolution_m,
                        resampling: request.resampling,
                        cloud_mask: request.cloud_mask,
                        mosaic_strategy: request.mosaic_strategy,
                        max_temporal_span_days: None,
                        overwrite: request.overwrite,
                        scenes: vec![scene],
                        output_tif_path: per_path.clone(),
                        write_lineage_sidecar: request.write_lineage_sidecar,
                        write_provenance_sidecars: request.write_provenance_sidecars,
                    };
                    let spec_value = serde_json::to_value(&per_spec).map_err(|e| {
                        invalid_request(format!("OPTICAL_DERIVED_SERIALIZE_FAILED: {e}"))
                    })?;
                    let spec_obj = spec_value.as_object().ok_or_else(|| {
                        invalid_request(
                            "OPTICAL_DERIVED_SERIALIZE_FAILED: spec must be a JSON object",
                        )
                    })?;
                    validate_debug_context(spec_obj).map_err(|e| {
                        invalid_request(format!("OPTICAL_DERIVED_SECRET_GUARD_FAILED: {e}"))
                    })?;
                    let mut item_metadata = Map::new();
                    item_metadata.insert(
                        "delivery_kind".to_owned(),
                        Value::String(delivery_kind.to_owned()),
                    );
                    item_metadata.insert(
                        "source_batch_id".to_owned(),
                        Value::String(request.source_batch_id.clone()),
                    );
                    item_metadata.insert(
                        "source_item_ids".to_owned(),
                        Value::Array(vec![Value::String(source_item.item_id.clone())]),
                    );
                    item_metadata.insert(
                        "band_order".to_owned(),
                        Value::Array(
                            request
                                .band_order
                                .iter()
                                .map(|b| Value::String(b.clone()))
                                .collect(),
                        ),
                    );
                    item_metadata.insert(
                        "cloud_mask".to_owned(),
                        serde_json::to_value(request.cloud_mask)
                            .unwrap_or(Value::String("none".to_owned())),
                    );
                    validate_debug_context(&item_metadata).map_err(|e| {
                        invalid_request(format!("OPTICAL_DERIVED_METADATA_INVALID: {e}"))
                    })?;
                    let mut item_payload = Map::new();
                    item_payload.insert("optical_derived_spec".to_owned(), spec_value);
                    validate_debug_context(&item_payload).map_err(|e| {
                        invalid_request(format!("OPTICAL_DERIVED_PAYLOAD_INVALID: {e}"))
                    })?;
                    let derived_item = SnapshotItem {
                        item_id: per_item_id.clone(),
                        provider_id: source_batch.provider_id.clone(),
                        display_name: per_filename,
                        acquisition_datetime: source_item.acquisition_datetime.clone(),
                        footprint_geojson: source_item
                            .footprint_geojson
                            .clone()
                            .or_else(|| source_batch.aoi_geojson.clone()),
                        metadata: item_metadata,
                        payload: item_payload,
                        asset_access: None,
                        execution_policy: None,
                    };
                    derived_item.validate().map_err(|e| {
                        invalid_request(format!("OPTICAL_DERIVED_ITEM_INVALID: {e}"))
                    })?;
                    derived_item_ids.push(per_item_id);
                    output_tif_paths.push(per_path);
                    derived_items.push(derived_item);
                }
            } else {
                let output_filename = build_derived_output_filename(&request, &selected_items)?;
                let output_tif_path = request.output_root.join(&output_filename);
                let derived_item_id =
                    build_derived_item_id(&request.task_id, &output_filename, &selected_items)?;
                let spec = OpticalDerivedRequest {
                    mode: request.mode,
                    aoi_bbox_wgs84,
                    aoi_rings_wgs84: Some(aoi_rings_wgs84),
                    band_order: request.band_order.clone(),
                    target_crs: request.target_crs.clone(),
                    target_resolution_m: request.target_resolution_m,
                    resampling: request.resampling,
                    cloud_mask: request.cloud_mask,
                    mosaic_strategy: request.mosaic_strategy,
                    max_temporal_span_days: request.max_temporal_span_days,
                    overwrite: request.overwrite,
                    scenes,
                    output_tif_path: output_tif_path.clone(),
                    write_lineage_sidecar: request.write_lineage_sidecar,
                    write_provenance_sidecars: request.write_provenance_sidecars,
                };
                let spec_value = serde_json::to_value(&spec).map_err(|e| {
                    invalid_request(format!("OPTICAL_DERIVED_SERIALIZE_FAILED: {e}"))
                })?;
                let spec_obj = spec_value.as_object().ok_or_else(|| {
                    invalid_request("OPTICAL_DERIVED_SERIALIZE_FAILED: spec must be a JSON object")
                })?;
                validate_debug_context(spec_obj).map_err(|e| {
                    invalid_request(format!("OPTICAL_DERIVED_SECRET_GUARD_FAILED: {e}"))
                })?;
                let mut item_metadata = Map::new();
                item_metadata.insert(
                    "delivery_kind".to_owned(),
                    Value::String(delivery_kind.to_owned()),
                );
                item_metadata.insert(
                    "source_batch_id".to_owned(),
                    Value::String(request.source_batch_id.clone()),
                );
                item_metadata.insert(
                    "source_item_ids".to_owned(),
                    Value::Array(
                        selected_items
                            .iter()
                            .map(|item| Value::String(item.item_id.clone()))
                            .collect(),
                    ),
                );
                item_metadata.insert(
                    "band_order".to_owned(),
                    Value::Array(
                        request
                            .band_order
                            .iter()
                            .map(|b| Value::String(b.clone()))
                            .collect(),
                    ),
                );
                item_metadata.insert(
                    "cloud_mask".to_owned(),
                    serde_json::to_value(request.cloud_mask)
                        .unwrap_or(Value::String("none".to_owned())),
                );
                validate_debug_context(&item_metadata).map_err(|e| {
                    invalid_request(format!("OPTICAL_DERIVED_METADATA_INVALID: {e}"))
                })?;
                let mut item_payload = Map::new();
                item_payload.insert("optical_derived_spec".to_owned(), spec_value);
                validate_debug_context(&item_payload).map_err(|e| {
                    invalid_request(format!("OPTICAL_DERIVED_PAYLOAD_INVALID: {e}"))
                })?;
                let acquisition_datetime = selected_items
                    .iter()
                    .filter_map(|item| item.acquisition_datetime.clone())
                    .max();
                let derived_item = SnapshotItem {
                    item_id: derived_item_id.clone(),
                    provider_id: source_batch.provider_id.clone(),
                    display_name: output_filename,
                    acquisition_datetime,
                    footprint_geojson: source_batch.aoi_geojson.clone(),
                    metadata: item_metadata,
                    payload: item_payload,
                    asset_access: None,
                    execution_policy: None,
                };
                derived_item
                    .validate()
                    .map_err(|e| invalid_request(format!("OPTICAL_DERIVED_ITEM_INVALID: {e}")))?;
                derived_item_ids.push(derived_item_id);
                output_tif_paths.push(output_tif_path);
                derived_items.push(derived_item);
            }

            let primary_item_id = derived_item_ids.first().cloned().ok_or_else(|| {
                invalid_request("OPTICAL_DERIVED_ITEMS_EMPTY: no derived items generated")
            })?;
            let primary_output_tif_path = output_tif_paths.first().cloned().ok_or_else(|| {
                invalid_request("OPTICAL_DERIVED_ITEMS_EMPTY: no derived output paths generated")
            })?;

            let mut derived_query = source_batch.query.clone();
            derived_query.insert(
                "derived_from_batch_id".to_owned(),
                Value::String(request.source_batch_id.clone()),
            );
            derived_query.insert(
                "delivery_kind".to_owned(),
                Value::String(delivery_kind.to_owned()),
            );
            derived_query.insert(
                "delivery_source_count".to_owned(),
                Value::Number((selected_items.len() as u64).into()),
            );
            validate_debug_context(&derived_query)
                .map_err(|e| invalid_request(format!("OPTICAL_DERIVED_QUERY_INVALID: {e}")))?;

            let mut derived_batch = source_batch.clone();
            derived_batch.batch_id = derived_batch_id.clone();
            derived_batch.created_at = request.created_at.clone();
            derived_batch.query = derived_query;
            derived_batch.search_stats = None;
            derived_batch.candidates = derived_items;
            derived_batch
                .validate()
                .map_err(|e| invalid_request(format!("OPTICAL_DERIVED_BATCH_INVALID: {e}")))?;

            let batch_request = StoreSearchBatchRequest {
                batch: derived_batch,
                workbench: SearchWorkbenchState {
                    batch_id: derived_batch_id.clone(),
                    checked_item_ids: derived_item_ids.clone(),
                    page_index: 0,
                    page_size,
                    revision: 0,
                },
                activate_workbench: false,
            };
            let task_request = StoreCreateTaskRequest {
                task_id: request.task_id.clone(),
                batch_id: derived_batch_id.clone(),
                selected_item_ids: derived_item_ids.clone(),
                checked_item_ids: derived_item_ids,
                page_index: 0,
                page_size,
                output_root: request.output_root.clone(),
                layout: DownloadLayout::Direct,
                concurrency: request.concurrency,
                created_at: request.created_at.clone(),
            };

            let snapshot = self.with_business_runtime(|runtime| {
                {
                    let mut store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    store
                        .save_search_batch_and_create_task(&batch_request, &task_request)
                        .map_err(ApplicationServiceError::from)?;
                }
                runtime
                    .scheduler
                    .as_ref()
                    .expect("business runtime guard established scheduler availability")
                    .wake();
                Self::snapshot_from_runtime(runtime)
            })?;

            Ok(OpticalDerivedTaskCreatedResult {
                precheck,
                task_id: request.task_id.clone(),
                derived_batch_id,
                derived_item_id: primary_item_id,
                output_tif_path: primary_output_tif_path,
                output_tif_paths,
                snapshot,
            })
        })();

        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    fn load_optical_source_selection(
        &self,
        source_batch_id: &str,
        expected_workbench_revision: u64,
        item_ids: &[String],
    ) -> Result<(FrozenSearchBatch, Vec<SnapshotItem>), ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let (batch, _, items) = resolve_source_batch_selection_in_store(
                &store,
                source_batch_id,
                expected_workbench_revision,
                item_ids,
            )?;
            Ok((batch, items))
        })
    }

    fn resolve_scene_for_execution(
        &self,
        source_batch: &FrozenSearchBatch,
        item: &SnapshotItem,
        band_order: &[String],
        cloud_mask: OpticalCloudMaskPolicy,
        network_policy: ProviderNetworkPolicy,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<OpticalSceneInput, ApplicationServiceError> {
        let source_kind = classify_item_source_kind(source_batch, item)?;
        if source_kind == OpticalSourceKind::Sentinel2L1c
            && cloud_mask == OpticalCloudMaskPolicy::CloudToNodata
        {
            return Err(invalid_request(
                "OPTICAL_CLOUD_MASK_UNSUPPORTED_L1C: Sentinel-2 L1C products do not contain an SCL cloud mask band",
            ));
        }
        let inline_assets = extract_inline_assets(item)?;

        let mut band_assets = Vec::with_capacity(band_order.len());
        let mut qa_asset: Option<OpticalBandAssetInput> = None;
        let mut secondary_qa_asset: Option<OpticalBandAssetInput> = None;

        if !inline_assets.is_empty()
            && all_requested_bands_inline(source_kind, &inline_assets, band_order)
        {
            for band in band_order {
                let found = lookup_asset_with_aliases(source_kind, &inline_assets, band)
                    .ok_or_else(|| {
                        invalid_request(format!(
                            "OPTICAL_DERIVED_BAND_MISSING: scene {} lacks requested band {}",
                            item.item_id, band
                        ))
                    })?;
                let mut normalized = found.clone();
                normalized.band_id = band.clone();
                validate_raw_optical_cog_source(&normalized.stable_href)
                    .map_err(|e| invalid_request(e.to_string()))?;
                band_assets.push(normalized);
            }
            if cloud_mask == OpticalCloudMaskPolicy::CloudToNodata {
                if let Some(qa) = lookup_qa_asset_inline(source_kind, &inline_assets) {
                    validate_raw_optical_cog_source(&qa.stable_href)
                        .map_err(|e| invalid_request(e.to_string()))?;
                    qa_asset = Some(qa.clone());
                } else if source_kind == OpticalSourceKind::Sentinel2L2a {
                    qa_asset = Some(fetch_sentinel2_scl_asset(
                        source_batch,
                        item,
                        network_policy,
                        cancelled,
                    )?);
                } else {
                    return Err(invalid_request(format!(
                        "OPTICAL_CLOUD_MASK_BAND_MISSING: scene {} lacks cloud/QA band for CloudToNodata",
                        item.item_id
                    )));
                }
                if let Some(sec_qa) = lookup_secondary_qa_asset_inline(source_kind, &inline_assets)
                {
                    validate_raw_optical_cog_source(&sec_qa.stable_href)
                        .map_err(|e| invalid_request(e.to_string()))?;
                    secondary_qa_asset = Some(sec_qa.clone());
                }
            }
            return build_scene_input_stub(
                source_batch,
                item,
                source_kind,
                band_assets,
                qa_asset,
                secondary_qa_asset,
            );
        }

        match source_kind {
            OpticalSourceKind::Sentinel2L2a => {
                let source_identity = frozen_sentinel2_analysis_source(source_batch, item)
                    .map_err(|e| {
                        invalid_request(format!(
                            "OPTICAL_DERIVED_EVIDENCE_INVALID: Sentinel-2 identity invalid: {e:?}"
                        ))
                    })?;
                for band in band_order {
                    if sentinel2_l2a_reflectance_band_resolution(band).is_none() {
                        return Err(invalid_request(format!(
                            "OPTICAL_DERIVED_BAND_UNSUPPORTED: Sentinel-2 L2A does not support reflectance band {band}"
                        )));
                    }
                }
                let runtime = Sentinel2BandEvidenceRuntime::new(network_policy)
                    .map_err(ApplicationServiceError::Provider)?;
                let (manifest_provider_id, manifest_item_id) =
                    if let Some(es_item_id) = source_identity.earth_search_item_id.as_deref() {
                        let manifest = runtime
                            .resolve_earth_search_band_manifest(
                                es_item_id,
                                &source_identity.safe_name,
                                item.acquisition_datetime.as_deref(),
                                band_order,
                                cancelled,
                            )
                            .map_err(ApplicationServiceError::Provider)?;
                        for asset in manifest.assets {
                            validate_raw_optical_cog_source(&asset.stable_href)
                                .map_err(|e| invalid_request(e.to_string()))?;
                            band_assets.push(OpticalBandAssetInput {
                                band_id: asset.asset_key,
                                stable_href: asset.stable_href,
                                resolved_href: None,
                                band_resolution_m: Some(f64::from(asset.resolution_m)),
                                role: Some("data".to_owned()),
                            });
                        }
                        (
                            EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID.to_owned(),
                            manifest.item_id,
                        )
                    } else {
                        let manifest = runtime
                            .resolve_band_manifest(
                                &source_identity.safe_name,
                                item.acquisition_datetime.as_deref(),
                                band_order,
                                cancelled,
                            )
                            .map_err(ApplicationServiceError::Provider)?;
                        for asset in manifest.assets {
                            validate_raw_optical_cog_source(&asset.stable_href)
                                .map_err(|e| invalid_request(e.to_string()))?;
                            band_assets.push(OpticalBandAssetInput {
                                band_id: asset.asset_key,
                                stable_href: asset.stable_href,
                                resolved_href: None,
                                band_resolution_m: Some(f64::from(asset.resolution_m)),
                                role: Some("data".to_owned()),
                            });
                        }
                        (
                            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
                            manifest.item_id,
                        )
                    };
                if cloud_mask == OpticalCloudMaskPolicy::CloudToNodata {
                    qa_asset = Some(
                        if let Some(qa) = lookup_qa_asset_inline(source_kind, &inline_assets) {
                            validate_raw_optical_cog_source(&qa.stable_href)
                                .map_err(|e| invalid_request(e.to_string()))?;
                            qa.clone()
                        } else {
                            fetch_sentinel2_scl_asset_for_manifest_item(
                                &manifest_provider_id,
                                &manifest_item_id,
                                network_policy,
                                cancelled,
                            )?
                        },
                    );
                }
            }
            OpticalSourceKind::LandsatC2L2 => {
                let evidence =
                    frozen_landsat_scene_evidence(source_batch, 1, item).map_err(|e| {
                        invalid_request(format!(
                            "OPTICAL_DERIVED_EVIDENCE_INVALID: Landsat identity invalid: {e:?}"
                        ))
                    })?;
                if evidence.dataset_alias != MPC_LANDSAT_COLLECTION_ID {
                    return Err(invalid_request(
                        "OPTICAL_DERIVED_SOURCE_UNSUPPORTED: only Microsoft Planetary Computer Landsat C2 L2 COGs are supported",
                    ));
                }
                let manifest = self
                    .landsat_asset_manifest_port()?
                    .landsat_asset_manifest(
                        &evidence.dataset_alias,
                        &evidence.entity_id,
                        &evidence.display_id,
                        cancelled,
                    )
                    .map_err(ApplicationServiceError::PlanetaryComputer)?;

                for band in band_order {
                    let asset =
                        find_landsat_manifest_asset(&manifest.assets, band).ok_or_else(|| {
                            invalid_request(format!(
                                "OPTICAL_DERIVED_BAND_MISSING: Landsat scene {} lacks band {}",
                                item.item_id, band
                            ))
                        })?;
                    validate_raw_optical_cog_source(&asset.stable_href)
                        .map_err(|e| invalid_request(e.to_string()))?;
                    band_assets.push(OpticalBandAssetInput {
                        band_id: band.clone(),
                        stable_href: asset.stable_href.clone(),
                        resolved_href: None,
                        band_resolution_m: Some(30.0),
                        role: Some("data".to_owned()),
                    });
                }
                if cloud_mask == OpticalCloudMaskPolicy::CloudToNodata {
                    let qa = find_landsat_manifest_asset(&manifest.assets, "QA_PIXEL").ok_or_else(
                        || {
                            invalid_request(format!(
                                "OPTICAL_CLOUD_MASK_BAND_MISSING: Landsat scene {} lacks QA_PIXEL asset",
                                item.item_id
                            ))
                        },
                    )?;
                    validate_raw_optical_cog_source(&qa.stable_href)
                        .map_err(|e| invalid_request(e.to_string()))?;
                    qa_asset = Some(OpticalBandAssetInput {
                        band_id: "QA_PIXEL".to_owned(),
                        stable_href: qa.stable_href.clone(),
                        resolved_href: None,
                        band_resolution_m: Some(30.0),
                        role: Some("cloud".to_owned()),
                    });
                    if let Some(radsat) = find_landsat_manifest_asset(&manifest.assets, "QA_RADSAT")
                    {
                        validate_raw_optical_cog_source(&radsat.stable_href)
                            .map_err(|e| invalid_request(e.to_string()))?;
                        secondary_qa_asset = Some(OpticalBandAssetInput {
                            band_id: "QA_RADSAT".to_owned(),
                            stable_href: radsat.stable_href.clone(),
                            resolved_href: None,
                            band_resolution_m: Some(30.0),
                            role: Some("saturation".to_owned()),
                        });
                    }
                }
            }
            OpticalSourceKind::Sentinel2L1c => {
                let (_, assets) =
                    crate::sentinel2_band_delivery::resolve_sentinel2_l1c_band_assets(
                        item,
                        band_order,
                        network_policy,
                        cancelled,
                    )?;
                for asset in assets {
                    validate_raw_optical_cog_source(&asset.stable_href)
                        .map_err(|e| invalid_request(e.to_string()))?;
                    band_assets.push(OpticalBandAssetInput {
                        band_id: asset.asset_key,
                        stable_href: asset.stable_href,
                        resolved_href: None,
                        band_resolution_m: Some(f64::from(asset.resolution_m)),
                        role: Some("data".to_owned()),
                    });
                }
            }
            OpticalSourceKind::HlsL30
            | OpticalSourceKind::HlsS30
            | OpticalSourceKind::OpticalRawCog => {
                return Err(invalid_request(format!(
                    "OPTICAL_DERIVED_BAND_MISSING: scene {} lacks required inline asset hrefs for requested bands",
                    item.item_id
                )));
            }
        }

        build_scene_input_stub(
            source_batch,
            item,
            source_kind,
            band_assets,
            qa_asset,
            secondary_qa_asset,
        )
    }
}

fn validate_create_task_request(
    request: &OpticalDerivedCreateTaskRequest,
) -> Result<(), ApplicationServiceError> {
    if !valid_catalogue_opaque_id(&request.request_id)
        || request.request_id.len() > 80
        || !valid_catalogue_opaque_id(&request.source_batch_id)
        || !valid_catalogue_opaque_id(&request.task_id)
        || request.task_id.len() > 72
        || !request.output_root.is_absolute()
        || !(1..=8).contains(&request.concurrency)
        || parse_normalized_utc(&request.created_at).is_none()
        || request.band_order.is_empty()
        || request.band_order.len() > 16
    {
        return Err(invalid_request(
            "OPTICAL_DERIVED_REQUEST_INVALID: invalid task identifier, timestamp, concurrency, output_root, or band_order",
        ));
    }
    let unique_bands = request.band_order.iter().collect::<BTreeSet<_>>();
    if unique_bands.len() != request.band_order.len()
        || request
            .band_order
            .iter()
            .any(|band| band.trim().is_empty() || band.len() > 24)
    {
        return Err(invalid_request(
            "OPTICAL_DERIVED_BAND_ORDER_INVALID: band_order contains empty or duplicate entries",
        ));
    }
    if let Some(res) = request.target_resolution_m {
        if !res.is_finite() || res <= 0.0 {
            return Err(invalid_request(
                "OPTICAL_DERIVED_RESOLUTION_INVALID: target_resolution_m must be finite and > 0",
            ));
        }
    }
    Ok(())
}

fn resolve_source_batch_selection_in_store(
    store: &insar_store::TaskStore,
    source_batch_id: &str,
    expected_workbench_revision: u64,
    item_ids: &[String],
) -> Result<(FrozenSearchBatch, u64, Vec<SnapshotItem>), ApplicationServiceError> {
    if item_ids.is_empty() {
        let slot = store
            .load_workbench()
            .map_err(ApplicationServiceError::from)?
            .ok_or_else(|| {
                invalid_request("OPTICAL_SOURCE_BATCH_NOT_FOUND: no active workbench")
            })?;
        if slot.batch.batch_id != source_batch_id
            || slot.state.revision != expected_workbench_revision
        {
            return Err(invalid_request(
                "OPTICAL_SOURCE_REVISION_CONFLICT: workbench revision or batch_id mismatch",
            ));
        }
        let items = collect_ordered_items(&slot.batch, &slot.state.checked_item_ids)?;
        return Ok((slot.batch, slot.state.page_size.max(1), items));
    }

    let batch = store
        .load_search_batch(source_batch_id)
        .map_err(ApplicationServiceError::from)?
        .ok_or_else(|| {
            invalid_request(format!(
                "OPTICAL_SOURCE_BATCH_NOT_FOUND: batch {source_batch_id} not found"
            ))
        })?;
    let active = store
        .load_workbench()
        .map_err(ApplicationServiceError::from)?;
    let expected_revision = if active
        .as_ref()
        .is_some_and(|slot| slot.batch.batch_id == batch.batch_id)
    {
        active.as_ref().expect("checked above").state.revision
    } else {
        0
    };
    if expected_workbench_revision != expected_revision {
        return Err(invalid_request(
            "OPTICAL_SOURCE_REVISION_CONFLICT: workbench revision mismatch",
        ));
    }
    let page_size = active
        .as_ref()
        .map(|slot| slot.state.page_size.max(1))
        .unwrap_or(20);
    let items = collect_ordered_items(&batch, item_ids)?;
    Ok((batch, page_size, items))
}

fn collect_ordered_items(
    batch: &FrozenSearchBatch,
    item_ids: &[String],
) -> Result<Vec<SnapshotItem>, ApplicationServiceError> {
    if item_ids.is_empty() {
        return Err(invalid_request(
            "OPTICAL_DERIVED_NO_SCENES_SELECTED: at least one scene must be selected",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(item_ids.len());
    for id in item_ids {
        if !seen.insert(id.as_str()) {
            return Err(invalid_request(format!(
                "OPTICAL_DERIVED_DUPLICATE_SCENE: duplicate item_id {id}"
            )));
        }
        let item = batch
            .candidates
            .iter()
            .find(|candidate| &candidate.item_id == id)
            .cloned()
            .ok_or_else(|| {
                invalid_request(format!(
                    "OPTICAL_DERIVED_ITEM_NOT_FOUND: item_id {id} is not in batch {}",
                    batch.batch_id
                ))
            })?;
        out.push(item);
    }
    Ok(out)
}

fn classify_item_source_kind(
    batch: &FrozenSearchBatch,
    item: &SnapshotItem,
) -> Result<OpticalSourceKind, ApplicationServiceError> {
    if item.provider_id == LANDSAT_C2_L2_PROVIDER_ID
        || item
            .metadata
            .get("native_collection_id")
            .and_then(Value::as_str)
            == Some("landsat_ot_c2_l2")
    {
        return Err(invalid_request(
            "OPTICAL_DERIVED_SOURCE_UNSUPPORTED: USGS M2M Landsat tar archives are not single-scene COGs and cannot be AOI-clipped cleanly",
        ));
    }

    if item.metadata.get("source_kind").and_then(Value::as_str) == Some("optical_raw_cog") {
        return Ok(OpticalSourceKind::OpticalRawCog);
    }

    let logical_dataset = item
        .metadata
        .get("logical_dataset")
        .and_then(Value::as_str)
        .or_else(|| batch.query.get("logical_dataset").and_then(Value::as_str))
        .unwrap_or("");

    match logical_dataset {
        MPC_SENTINEL2_L2A_BAND_LOGICAL_DATASET => Ok(OpticalSourceKind::Sentinel2L2a),
        "optical.sentinel-2.l1c" => Ok(OpticalSourceKind::Sentinel2L1c),
        LANDSAT_C2_L2_LOGICAL_DATASET => {
            if item.provider_id == MPC_LANDSAT_PROVIDER_ID
                || batch.provider_id == MPC_LANDSAT_PROVIDER_ID
            {
                Ok(OpticalSourceKind::LandsatC2L2)
            } else {
                Err(invalid_request(
                    "OPTICAL_DERIVED_SOURCE_UNSUPPORTED: non-MPC Landsat provider does not expose COG bands",
                ))
            }
        }
        MPC_HLS_L30_LOGICAL_DATASET => Ok(OpticalSourceKind::HlsL30),
        MPC_HLS_S30_LOGICAL_DATASET => Ok(OpticalSourceKind::HlsS30),
        "optical.raw_cog" => Ok(OpticalSourceKind::OpticalRawCog),
        _ => {
            if item.provider_id == PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID {
                Ok(OpticalSourceKind::Sentinel2L2a)
            } else if item.provider_id == MPC_LANDSAT_PROVIDER_ID {
                Ok(OpticalSourceKind::LandsatC2L2)
            } else {
                Err(invalid_request(format!(
                    "OPTICAL_DERIVED_SOURCE_UNSUPPORTED: unsupported optical dataset '{logical_dataset}' / provider '{}'",
                    item.provider_id
                )))
            }
        }
    }
}

type AoiBboxAndRings = ([f64; 4], Vec<Vec<[f64; 2]>>);

fn resolve_aoi_from_request_or_batch(
    bbox_override: Option<[f64; 4]>,
    rings_override: Option<&Vec<Vec<[f64; 2]>>>,
    batch: &FrozenSearchBatch,
) -> Result<AoiBboxAndRings, ApplicationServiceError> {
    if let Some(rings) = rings_override {
        validate_rings(rings)?;
        let bbox = bbox_override.unwrap_or_else(|| rings_bbox(rings));
        validate_wgs84_bbox(bbox)?;
        return Ok((bbox, rings.clone()));
    }
    if let Some(bbox) = bbox_override {
        validate_wgs84_bbox(bbox)?;
        return Ok((bbox, vec![bbox_ring(bbox)]));
    }

    if let Some(geojson) = batch
        .aoi_geojson
        .as_ref()
        .or_else(|| batch.query.get("aoi_geojson").and_then(Value::as_object))
    {
        if let Some(rings) = extract_rings_from_geojson_map(geojson) {
            validate_rings(&rings)?;
            let bbox = rings_bbox(&rings);
            validate_wgs84_bbox(bbox)?;
            return Ok((bbox, rings));
        }
    }

    Err(invalid_request(
        "OPTICAL_AOI_MISSING: a valid WGS84 AOI polygon or bounding box is required",
    ))
}

fn validate_rings(rings: &[Vec<[f64; 2]>]) -> Result<(), ApplicationServiceError> {
    if rings.is_empty() || rings.iter().any(|ring| ring.len() < 4) {
        return Err(invalid_request(
            "OPTICAL_AOI_INVALID: AOI polygon rings must contain at least 4 coordinates",
        ));
    }
    for ring in rings {
        for &[lon, lat] in ring {
            if !lon.is_finite()
                || !lat.is_finite()
                || !(-180.0..=180.0).contains(&lon)
                || !(-90.0..=90.0).contains(&lat)
            {
                return Err(invalid_request(
                    "OPTICAL_AOI_INVALID: AOI coordinate is outside WGS84 bounds",
                ));
            }
        }
    }
    Ok(())
}

fn validate_wgs84_bbox(bbox: [f64; 4]) -> Result<(), ApplicationServiceError> {
    let [min_x, min_y, max_x, max_y] = bbox;
    if !min_x.is_finite()
        || !min_y.is_finite()
        || !max_x.is_finite()
        || !max_y.is_finite()
        || min_x >= max_x
        || min_y >= max_y
        || !(-180.0..=180.0).contains(&min_x)
        || !(-180.0..=180.0).contains(&max_x)
        || !(-90.0..=90.0).contains(&min_y)
        || !(-90.0..=90.0).contains(&max_y)
    {
        return Err(invalid_request(
            "OPTICAL_AOI_INVALID: AOI bounding box must have finite ordered WGS84 coordinates",
        ));
    }
    Ok(())
}

fn bbox_ring(bbox: [f64; 4]) -> Vec<[f64; 2]> {
    let [min_x, min_y, max_x, max_y] = bbox;
    vec![
        [min_x, min_y],
        [max_x, min_y],
        [max_x, max_y],
        [min_x, max_y],
        [min_x, min_y],
    ]
}

fn rings_bbox(rings: &[Vec<[f64; 2]>]) -> [f64; 4] {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for ring in rings {
        for &[x, y] in ring {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    [min_x, min_y, max_x, max_y]
}

fn extract_rings_from_geojson_map(map: &Map<String, Value>) -> Option<Vec<Vec<[f64; 2]>>> {
    let kind = map.get("type")?.as_str()?;
    let coords = map.get("coordinates")?;
    match kind {
        "Polygon" => {
            let rings = coords.as_array()?;
            let outer = rings.first()?.as_array()?;
            Some(vec![parse_ring_coords(outer)?])
        }
        "MultiPolygon" => {
            let polys = coords.as_array()?;
            let mut out = Vec::with_capacity(polys.len());
            for poly in polys {
                let rings = poly.as_array()?;
                let outer = rings.first()?.as_array()?;
                out.push(parse_ring_coords(outer)?);
            }
            (!out.is_empty()).then_some(out)
        }
        _ => None,
    }
}

fn parse_ring_coords(raw: &[Value]) -> Option<Vec<[f64; 2]>> {
    let mut ring = Vec::with_capacity(raw.len());
    for pt in raw {
        let arr = pt.as_array()?;
        let lon = arr.first()?.as_f64()?;
        let lat = arr.get(1)?.as_f64()?;
        ring.push([lon, lat]);
    }
    (ring.len() >= 4).then_some(ring)
}

fn build_scene_input_stub(
    source_batch: &FrozenSearchBatch,
    item: &SnapshotItem,
    source_kind: OpticalSourceKind,
    band_assets: Vec<OpticalBandAssetInput>,
    qa_asset: Option<OpticalBandAssetInput>,
    secondary_qa_asset: Option<OpticalBandAssetInput>,
) -> Result<OpticalSceneInput, ApplicationServiceError> {
    let footprint_rings = item
        .footprint_geojson
        .as_ref()
        .and_then(extract_rings_from_geojson_map);
    let footprint_bbox = footprint_rings.as_ref().map(|rings| rings_bbox(rings));

    let properties = item.metadata.get("properties").and_then(Value::as_object);
    let cloud_cover_pct = item
        .metadata
        .get("cloud_cover")
        .and_then(Value::as_f64)
        .or_else(|| item.metadata.get("eo:cloud_cover").and_then(Value::as_f64))
        .or_else(|| properties.and_then(|p| p.get("eo:cloud_cover")?.as_f64()));

    let native_epsg = item
        .metadata
        .get("proj_epsg")
        .and_then(Value::as_u64)
        .or_else(|| item.metadata.get("epsg").and_then(Value::as_u64))
        .or_else(|| properties.and_then(|p| p.get("proj:epsg")?.as_u64()))
        .and_then(|v| u32::try_from(v).ok());

    let collection = item
        .metadata
        .get("native_collection_id")
        .and_then(Value::as_str)
        .or_else(|| item.metadata.get("collection").and_then(Value::as_str))
        .or_else(|| {
            source_batch
                .query
                .get("native_collection_id")
                .and_then(Value::as_str)
        })
        .unwrap_or_else(|| default_collection_for_kind(source_kind))
        .to_owned();

    Ok(OpticalSceneInput {
        scene_id: item.display_name.clone(),
        source_kind,
        collection,
        acquired_at: item.acquisition_datetime.clone(),
        cloud_cover_pct,
        footprint_bbox_wgs84: footprint_bbox,
        footprint_rings_wgs84: footprint_rings,
        native_epsg,
        band_assets,
        qa_asset,
        secondary_qa_asset,
        stac_metadata: None,
    })
}

const fn default_collection_for_kind(kind: OpticalSourceKind) -> &'static str {
    match kind {
        OpticalSourceKind::Sentinel2L2a => SENTINEL2_L2A_COLLECTION_ID,
        OpticalSourceKind::Sentinel2L1c => "SENTINEL-2",
        OpticalSourceKind::LandsatC2L2 => MPC_LANDSAT_COLLECTION_ID,
        OpticalSourceKind::HlsL30 => MPC_HLS_L30_COLLECTION_ID,
        OpticalSourceKind::HlsS30 => MPC_HLS_S30_COLLECTION_ID,
        OpticalSourceKind::OpticalRawCog => "optical-raw-cog",
    }
}

fn extract_inline_assets(
    item: &SnapshotItem,
) -> Result<Vec<OpticalBandAssetInput>, ApplicationServiceError> {
    let Some(raw_assets) = item.metadata.get("assets") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    match raw_assets {
        Value::Array(arr) => {
            for entry in arr {
                let Some(obj) = entry.as_object() else {
                    continue;
                };
                let Some(band_id) = obj
                    .get("asset_key")
                    .or_else(|| obj.get("band_id"))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                let Some(href) = obj
                    .get("stable_href")
                    .or_else(|| obj.get("url"))
                    .or_else(|| obj.get("href"))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                out.push(OpticalBandAssetInput {
                    band_id: band_id.to_owned(),
                    stable_href: href.to_owned(),
                    resolved_href: None,
                    band_resolution_m: obj
                        .get("band_resolution_m")
                        .or_else(|| obj.get("resolution_m"))
                        .and_then(Value::as_f64),
                    role: obj.get("role").and_then(Value::as_str).map(str::to_owned),
                });
            }
        }
        Value::Object(map) => {
            for (band_id, entry) in map {
                let Some(obj) = entry.as_object() else {
                    continue;
                };
                let Some(href) = obj
                    .get("stable_href")
                    .or_else(|| obj.get("href"))
                    .or_else(|| obj.get("url"))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                out.push(OpticalBandAssetInput {
                    band_id: band_id.clone(),
                    stable_href: href.to_owned(),
                    resolved_href: None,
                    band_resolution_m: obj
                        .get("band_resolution_m")
                        .or_else(|| obj.get("resolution_m"))
                        .and_then(Value::as_f64),
                    role: obj.get("role").and_then(Value::as_str).map(str::to_owned),
                });
            }
        }
        _ => {}
    }
    Ok(out)
}

fn all_requested_bands_inline(
    kind: OpticalSourceKind,
    inline_assets: &[OpticalBandAssetInput],
    band_order: &[String],
) -> bool {
    !band_order.is_empty()
        && band_order
            .iter()
            .all(|band| lookup_asset_with_aliases(kind, inline_assets, band).is_some())
}

fn lookup_asset_with_aliases<'a>(
    kind: OpticalSourceKind,
    assets: &'a [OpticalBandAssetInput],
    requested_band: &str,
) -> Option<&'a OpticalBandAssetInput> {
    if let Some(exact) = assets
        .iter()
        .find(|asset| asset.band_id.eq_ignore_ascii_case(requested_band))
    {
        return Some(exact);
    }
    if kind == OpticalSourceKind::LandsatC2L2 {
        for alias in landsat_band_aliases(requested_band) {
            if let Some(found) = assets
                .iter()
                .find(|asset| asset.band_id.eq_ignore_ascii_case(alias))
            {
                return Some(found);
            }
        }
    }
    None
}

fn lookup_qa_asset_inline(
    kind: OpticalSourceKind,
    assets: &[OpticalBandAssetInput],
) -> Option<&OpticalBandAssetInput> {
    let candidates: &[&str] = match kind {
        OpticalSourceKind::Sentinel2L2a => &["SCL", "scl"],
        OpticalSourceKind::Sentinel2L1c => &[],
        OpticalSourceKind::LandsatC2L2 => &["QA_PIXEL", "qa_pixel"],
        OpticalSourceKind::HlsL30 | OpticalSourceKind::HlsS30 => &["Fmask", "fmask", "FMASK"],
        OpticalSourceKind::OpticalRawCog => {
            &["SCL", "scl", "QA_PIXEL", "qa_pixel", "Fmask", "fmask", "qa"]
        }
    };
    for candidate in candidates {
        if let Some(found) = assets
            .iter()
            .find(|asset| asset.band_id.eq_ignore_ascii_case(candidate))
        {
            return Some(found);
        }
    }
    None
}

fn lookup_secondary_qa_asset_inline(
    kind: OpticalSourceKind,
    assets: &[OpticalBandAssetInput],
) -> Option<&OpticalBandAssetInput> {
    if !matches!(
        kind,
        OpticalSourceKind::LandsatC2L2 | OpticalSourceKind::OpticalRawCog
    ) {
        return None;
    }
    for candidate in ["QA_RADSAT", "qa_radsat", "radsat_qa"] {
        if let Some(found) = assets
            .iter()
            .find(|asset| asset.band_id.eq_ignore_ascii_case(candidate))
        {
            return Some(found);
        }
    }
    None
}

fn find_landsat_manifest_asset<'a>(
    assets: &'a [PlanetaryComputerLandsatAsset],
    requested_band: &str,
) -> Option<&'a PlanetaryComputerLandsatAsset> {
    if let Some(exact) = assets
        .iter()
        .find(|asset| asset.asset_key.eq_ignore_ascii_case(requested_band))
    {
        return Some(exact);
    }
    for alias in landsat_band_aliases(requested_band) {
        if let Some(found) = assets
            .iter()
            .find(|asset| asset.asset_key.eq_ignore_ascii_case(alias))
        {
            return Some(found);
        }
    }
    None
}

fn landsat_band_aliases(band: &str) -> &'static [&'static str] {
    match band.to_ascii_uppercase().as_str() {
        "SR_B1" | "COASTAL" => &["SR_B1", "coastal"],
        "SR_B2" | "BLUE" => &["SR_B2", "blue"],
        "SR_B3" | "GREEN" => &["SR_B3", "green"],
        "SR_B4" | "RED" => &["SR_B4", "red"],
        "SR_B5" | "NIR08" => &["SR_B5", "nir08"],
        "SR_B6" | "SWIR16" => &["SR_B6", "swir16"],
        "SR_B7" | "SWIR22" => &["SR_B7", "swir22"],
        "QA_PIXEL" => &["QA_PIXEL", "qa_pixel"],
        "QA_RADSAT" | "RADSAT_QA" => &["QA_RADSAT", "qa_radsat", "radsat_qa"],
        _ => &[],
    }
}

fn fetch_sentinel2_scl_asset(
    source_batch: &FrozenSearchBatch,
    item: &SnapshotItem,
    network_policy: ProviderNetworkPolicy,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<OpticalBandAssetInput, ApplicationServiceError> {
    let identity = frozen_sentinel2_analysis_source(source_batch, item).map_err(|e| {
        invalid_request(format!(
            "OPTICAL_CLOUD_MASK_BAND_MISSING: Sentinel-2 identity invalid while resolving SCL: {e:?}"
        ))
    })?;
    let runtime = Sentinel2BandEvidenceRuntime::new(network_policy)
        .map_err(ApplicationServiceError::Provider)?;
    let (provider_id, item_id) = if let Some(es_item_id) = identity.earth_search_item_id.as_deref()
    {
        let manifest = runtime
            .resolve_earth_search_band_manifest(
                es_item_id,
                &identity.safe_name,
                item.acquisition_datetime.as_deref(),
                &["B04".to_owned()],
                cancelled,
            )
            .map_err(ApplicationServiceError::Provider)?;
        (
            EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID.to_owned(),
            manifest.item_id,
        )
    } else {
        let manifest = runtime
            .resolve_band_manifest(
                &identity.safe_name,
                item.acquisition_datetime.as_deref(),
                &["B04".to_owned()],
                cancelled,
            )
            .map_err(ApplicationServiceError::Provider)?;
        (
            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
            manifest.item_id,
        )
    };
    fetch_sentinel2_scl_asset_for_manifest_item(&provider_id, &item_id, network_policy, cancelled)
}

fn fetch_sentinel2_scl_asset_for_manifest_item(
    provider_id: &str,
    stac_item_id: &str,
    network_policy: ProviderNetworkPolicy,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<OpticalBandAssetInput, ApplicationServiceError> {
    if cancelled() {
        return Err(invalid_request("OPTICAL_DERIVED_CANCELLED: cancelled"));
    }
    let is_earth_search = provider_id == EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID;
    let stac_origin_str = if is_earth_search {
        EARTH_SEARCH_STAC_ENDPOINT
    } else {
        PLANETARY_COMPUTER_STAC_ORIGIN
    };
    let stac_origin = Url::parse(stac_origin_str)
        .map_err(|_| invalid_request("OPTICAL_CLOUD_MASK_BAND_MISSING: invalid STAC origin"))?;
    let mut item_url = stac_origin;
    item_url
        .path_segments_mut()
        .map_err(|_| invalid_request("OPTICAL_CLOUD_MASK_BAND_MISSING: invalid STAC path"))?
        .extend([
            "collections",
            SENTINEL2_L2A_COLLECTION_ID,
            "items",
            stac_item_id,
        ]);

    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(STAC_REQUEST_TIMEOUT)
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .referer(false)
        .user_agent("EOStage/OpticalDerivedStac");
    match network_policy.route() {
        ProxyPolicy::Direct => {
            builder = builder.no_proxy();
        }
        ProxyPolicy::System => {}
        ProxyPolicy::Manual(manual) => {
            let proxy = reqwest::Proxy::all(manual.url()).map_err(|_| {
                ApplicationServiceError::Provider(
                    ContractError::new("MPC_PROXY_INVALID", "manual proxy invalid", Map::new())
                        .expect("fixed error"),
                )
            })?;
            builder = builder.proxy(proxy);
        }
    }
    let client = builder.build().map_err(|e| {
        invalid_request(format!(
            "OPTICAL_CLOUD_MASK_BAND_MISSING: HTTP client init failed: {e}"
        ))
    })?;
    let response = client.get(item_url).send().map_err(|e| {
        invalid_request(format!(
            "OPTICAL_CLOUD_MASK_BAND_MISSING: failed to fetch Sentinel-2 STAC item for SCL: {e}"
        ))
    })?;
    if !response.status().is_success() {
        return Err(invalid_request(format!(
            "OPTICAL_CLOUD_MASK_BAND_MISSING: Sentinel-2 STAC item HTTP {}",
            response.status()
        )));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_STAC_ITEM_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            invalid_request(format!(
                "OPTICAL_CLOUD_MASK_BAND_MISSING: failed to read Sentinel-2 STAC item body: {e}"
            ))
        })?;
    if bytes.len() as u64 > MAX_STAC_ITEM_JSON_BYTES {
        return Err(invalid_request(
            "OPTICAL_CLOUD_MASK_BAND_MISSING: Sentinel-2 STAC item response exceeded limit",
        ));
    }
    let doc: Value = serde_json::from_slice(&bytes).map_err(|e| {
        invalid_request(format!(
            "OPTICAL_CLOUD_MASK_BAND_MISSING: invalid Sentinel-2 STAC JSON: {e}"
        ))
    })?;
    let scl_href = doc
        .get("assets")
        .and_then(|a| {
            a.get(if is_earth_search { "scl" } else { "SCL" })
                .or_else(|| a.get("SCL"))
        })
        .and_then(|scl| scl.get("href"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            invalid_request(format!(
                "OPTICAL_CLOUD_MASK_BAND_MISSING: Sentinel-2 item {stac_item_id} has no SCL asset"
            ))
        })?;
    if is_earth_search {
        validate_earth_search_sentinel2_scl_url(stac_item_id, scl_href)
            .map_err(ApplicationServiceError::Provider)?;
    } else {
        validate_planetary_computer_stable_asset(
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog,
            scl_href,
        )
        .map_err(ApplicationServiceError::Provider)?;
    }
    validate_raw_optical_cog_source(scl_href).map_err(|e| invalid_request(e.to_string()))?;

    Ok(OpticalBandAssetInput {
        band_id: "SCL".to_owned(),
        stable_href: scl_href.to_owned(),
        resolved_href: None,
        band_resolution_m: Some(20.0),
        role: Some("cloud".to_owned()),
    })
}

fn build_derived_output_filename(
    request: &OpticalDerivedCreateTaskRequest,
    selected_items: &[SnapshotItem],
) -> Result<String, ApplicationServiceError> {
    build_derived_output_filename_for_mode(request, request.mode, selected_items)
}

fn build_derived_output_filename_for_mode(
    request: &OpticalDerivedCreateTaskRequest,
    mode: OpticalDerivedMode,
    selected_items: &[SnapshotItem],
) -> Result<String, ApplicationServiceError> {
    let mut hasher = Sha256::new();
    hasher.update(request.task_id.as_bytes());
    for item in selected_items {
        hasher.update(b"|");
        hasher.update(item.item_id.as_bytes());
    }
    for band in &request.band_order {
        hasher.update(b":");
        hasher.update(band.as_bytes());
    }
    if let Some(crs) = &request.target_crs {
        hasher.update(b"@");
        hasher.update(crs.as_bytes());
    }
    let hash_hex = format!("{:x}", hasher.finalize());
    let hash8 = &hash_hex[..8];

    let bands_tag = {
        let joined = request
            .band_order
            .iter()
            .take(4)
            .map(|b| sanitize_slug_component(b))
            .collect::<Vec<_>>()
            .join("_");
        let suffix = if request.band_order.len() > 4 {
            format!("{joined}_p{}", request.band_order.len() - 4)
        } else {
            joined
        };
        if suffix.is_empty() {
            "bands".to_owned()
        } else if suffix.len() > 24 {
            suffix[..24].to_owned()
        } else {
            suffix
        }
    };

    let candidate = match mode {
        OpticalDerivedMode::SingleSceneClip | OpticalDerivedMode::BatchSingleSceneClip => {
            let mut stem = sanitize_slug_component(&selected_items[0].display_name);
            if stem.len() > 48 {
                stem.truncate(48);
            }
            if stem.is_empty() {
                stem = "scene".to_owned();
            }
            format!("{stem}__aoi_clip_{bands_tag}_{hash8}.tif")
        }
        OpticalDerivedMode::NearCoincidentSpatialMosaic => {
            format!(
                "optical_mosaic_{}scenes_{bands_tag}_{hash8}.tif",
                selected_items.len()
            )
        }
    };
    let clean = scrub_forbidden_display_substrings(&candidate);
    if !valid_catalogue_display_name(&clean)
        || selected_items
            .iter()
            .any(|item| clean.eq_ignore_ascii_case(&item.display_name))
    {
        return Err(invalid_request(
            "OPTICAL_DERIVED_FILENAME_INVALID: could not construct safe derived filename",
        ));
    }
    Ok(clean)
}

fn sanitize_slug_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_underscore = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_underscore = false;
        } else if matches!(ch, '-' | '_') {
            out.push(ch);
            prev_underscore = ch == '_';
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }
    out.trim_matches('_').to_owned()
}

fn scrub_forbidden_display_substrings(input: &str) -> String {
    let mut current = input.to_owned();
    for forbidden in [
        "http",
        "token",
        "account",
        "credential",
        "password",
        "authorization",
        "cookie",
        "bearer",
        "secret",
        "path",
    ] {
        while let Some(idx) = current.to_ascii_lowercase().find(forbidden) {
            current.replace_range(idx..idx + forbidden.len(), "ref");
        }
    }
    current
}

fn build_derived_item_id(
    task_id: &str,
    output_filename: &str,
    selected_items: &[SnapshotItem],
) -> Result<String, ApplicationServiceError> {
    let mut hasher = Sha256::new();
    hasher.update(task_id.as_bytes());
    hasher.update(b":");
    hasher.update(output_filename.as_bytes());
    for item in selected_items {
        hasher.update(b":");
        hasher.update(item.item_id.as_bytes());
    }
    let hex = format!("{:x}", hasher.finalize());
    let candidate = format!("optder-{}", &hex[..16]);
    if !valid_catalogue_opaque_id(&candidate) {
        return Err(invalid_request(
            "OPTICAL_DERIVED_ITEM_ID_INVALID: generated derived item_id is invalid",
        ));
    }
    Ok(candidate)
}

fn invalid_request(reason: impl Into<String>) -> ApplicationServiceError {
    ApplicationServiceError::InvalidRequest {
        reason: reason.into(),
    }
}

#[allow(dead_code)]
fn _assert_debug_clean_keys(_map: &BTreeMap<String, String>) {}

struct StageDirGuard(Option<PathBuf>);

impl Drop for StageDirGuard {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

pub(crate) fn execute_optical_derived_item(
    port: &dyn crate::DemOperationPort,
    transfer_activity: &insar_download::TaskTransferActivityRegistry,
    cdse_session: Option<&(
        std::sync::Arc<insar_download::CdseOAuthSession>,
        ProviderNetworkPolicy,
    )>,
    work: insar_download::DownloadWork,
    control: insar_download::WorkerControl,
    progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
) -> insar_core::DownloadItemResult {
    use chrono::SecondsFormat;
    use insar_core::{
        DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadItemStatus,
        DownloadOutcome,
    };

    let now = || chrono::Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true);
    let task_id = work.task.task_id.clone();
    let item_id = work.item.item_id.clone();
    let fallback_dest = work.task.output_root.join(&work.item.display_name);

    let Some(spec_value) = work.item.payload.get("optical_derived_spec").cloned() else {
        return DownloadItemResult {
            task_id,
            item_id,
            outcome: DownloadOutcome::Failed,
            destination: fallback_dest,
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(DownloadFailure {
                code: "OPTICAL_DERIVED_SPEC_MISSING".to_owned(),
                reason_zh: "光学裁切任务缺少执行规格".to_owned(),
                retryable: false,
            }),
            finished_at: now(),
        };
    };

    let mut spec: OpticalDerivedRequest = match serde_json::from_value(spec_value.clone()) {
        Ok(spec) => spec,
        Err(_) => {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Failed,
                destination: fallback_dest,
                bytes_written: 0,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: "OPTICAL_DERIVED_SPEC_INVALID".to_owned(),
                    reason_zh: "光学裁切任务规格无法解析".to_owned(),
                    retryable: false,
                }),
                finished_at: now(),
            };
        }
    };

    let destination = spec.output_tif_path.clone();
    if !destination.is_absolute() || !destination.starts_with(&work.task.output_root) {
        return DownloadItemResult {
            task_id,
            item_id,
            outcome: DownloadOutcome::Failed,
            destination,
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(DownloadFailure {
                code: "OPTICAL_DERIVED_DESTINATION_INVALID".to_owned(),
                reason_zh: "光学导出文件路径不属于当前任务目录".to_owned(),
                retryable: false,
            }),
            finished_at: now(),
        };
    }

    progress(DownloadItemProgress {
        task_id: task_id.clone(),
        item_id: item_id.clone(),
        status: DownloadItemStatus::Active,
        bytes_completed: 0,
        total_bytes: None,
        bytes_per_second: 0.0,
        attempt: 1,
        updated_at: now(),
    });

    if control.is_cancelled() {
        return DownloadItemResult {
            task_id,
            item_id,
            outcome: DownloadOutcome::Cancelled,
            destination,
            bytes_written: 0,
            attempt_count: 0,
            failure: None,
            finished_at: now(),
        };
    }
    if control.is_paused() {
        return DownloadItemResult {
            task_id,
            item_id,
            outcome: DownloadOutcome::Interrupted,
            destination,
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: now(),
        };
    }

    let mut stage_dir_guard = StageDirGuard(None);
    let needs_cdse_stage = spec.scenes.iter().any(|scene| {
        scene
            .band_assets
            .iter()
            .chain(scene.qa_asset.as_ref())
            .chain(scene.secondary_qa_asset.as_ref())
            .any(|asset| {
                asset
                    .stable_href
                    .starts_with("https://download.dataspace.copernicus.eu/")
            })
    });
    if needs_cdse_stage {
        let Some((session, network_policy)) = cdse_session else {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Failed,
                destination,
                bytes_written: 0,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: "CDSE_CREDENTIALS_MISSING".to_owned(),
                    reason_zh: "该最新 L1C 影像尚未同步至公开镜像，需要配置并登录 CDSE 账号后裁剪"
                        .to_owned(),
                    retryable: false,
                }),
                finished_at: now(),
            };
        };
        let client = match crate::sentinel2_band_delivery::build_optical_http_client(
            *network_policy,
            "EOStage/OpticalDerivedCdseStage",
            Duration::from_secs(300),
        ) {
            Ok(client) => client,
            Err(_) => {
                return DownloadItemResult {
                    task_id,
                    item_id,
                    outcome: DownloadOutcome::Failed,
                    destination,
                    bytes_written: 0,
                    attempt_count: 1,
                    failure: Some(DownloadFailure {
                        code: "OPTICAL_CDSE_HTTP_UNAVAILABLE".to_owned(),
                        reason_zh: "无法初始化 CDSE 临时缓存 HTTP 客户端".to_owned(),
                        retryable: false,
                    }),
                    finished_at: now(),
                };
            }
        };
        let dir = work.task.output_root.join(format!(
            ".eostage-optical-stage-{}",
            uuid::Uuid::new_v4().simple()
        ));
        if let Err(err) = std::fs::create_dir_all(&dir) {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Failed,
                destination,
                bytes_written: 0,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: "OPTICAL_DERIVED_STAGE_FAILED".to_owned(),
                    reason_zh: format!("无法创建临时下载缓存目录: {err}"),
                    retryable: false,
                }),
                finished_at: now(),
            };
        }
        stage_dir_guard.0 = Some(dir.clone());
        let mut total_staged = 0u64;
        for (scene_idx, scene) in spec.scenes.iter_mut().enumerate() {
            for asset in scene
                .band_assets
                .iter_mut()
                .chain(scene.qa_asset.as_mut())
                .chain(scene.secondary_qa_asset.as_mut())
            {
                if !asset
                    .stable_href
                    .starts_with("https://download.dataspace.copernicus.eu/")
                {
                    continue;
                }
                let local_file = dir.join(format!(
                    "s{}_{}.jp2",
                    scene_idx,
                    sanitize_slug_component(&asset.band_id)
                ));
                let _ = transfer_activity.body_started(
                    &task_id,
                    &item_id,
                    1,
                    std::time::Instant::now(),
                );
                let dl =
                    crate::sentinel2_band_delivery::download_http_asset_to_path_with_cdse_retry(
                        &client,
                        &asset.stable_href,
                        &local_file,
                        Some(session),
                        true,
                        &|| control.is_cancelled() || control.is_paused(),
                        &mut |delta| {
                            total_staged = total_staged.saturating_add(delta);
                            let _ = transfer_activity.observe_bytes(
                                &task_id,
                                &item_id,
                                1,
                                delta,
                                std::time::Instant::now(),
                            );
                            progress(DownloadItemProgress {
                                task_id: task_id.clone(),
                                item_id: item_id.clone(),
                                status: DownloadItemStatus::Active,
                                bytes_completed: total_staged,
                                total_bytes: None,
                                bytes_per_second: transfer_activity
                                    .member_bytes_per_second(&task_id, &item_id, 1)
                                    .unwrap_or(0.0),
                                attempt: 1,
                                updated_at: now(),
                            });
                        },
                    );
                let _ = transfer_activity.body_stopped(
                    &task_id,
                    &item_id,
                    1,
                    std::time::Instant::now(),
                );
                if control.is_cancelled() {
                    return DownloadItemResult {
                        task_id,
                        item_id,
                        outcome: DownloadOutcome::Cancelled,
                        destination,
                        bytes_written: 0,
                        attempt_count: 0,
                        failure: None,
                        finished_at: now(),
                    };
                }
                if control.is_paused() {
                    return DownloadItemResult {
                        task_id,
                        item_id,
                        outcome: DownloadOutcome::Interrupted,
                        destination,
                        bytes_written: 0,
                        attempt_count: 1,
                        failure: None,
                        finished_at: now(),
                    };
                }
                if let Err((code, reason_zh, retryable)) = dl {
                    return DownloadItemResult {
                        task_id,
                        item_id,
                        outcome: DownloadOutcome::Failed,
                        destination,
                        bytes_written: 0,
                        attempt_count: 1,
                        failure: Some(DownloadFailure {
                            code,
                            reason_zh,
                            retryable,
                        }),
                        finished_at: now(),
                    };
                }
                asset.resolved_href = Some(local_file.to_string_lossy().into_owned());
            }
        }
    }
    let staged_spec_value = match serde_json::to_value(&spec) {
        Ok(val) => val,
        Err(err) => {
            return DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Failed,
                destination,
                bytes_written: 0,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: "OPTICAL_DERIVED_SPEC_INVALID".to_owned(),
                    reason_zh: format!("无法序列化执行规格: {err}"),
                    retryable: false,
                }),
                finished_at: now(),
            };
        }
    };

    let mut observed_bytes = 0u64;
    let operation_result = port.call_with_transfer_progress(
        "optical_derived_execute",
        staged_spec_value,
        &|| control.is_cancelled() || control.is_paused(),
        &mut |event| {
            let observed_at = std::time::Instant::now();
            let sample = match event {
                crate::DemTransferEvent::BodyStarted => {
                    transfer_activity.body_started(&task_id, &item_id, 1, observed_at)
                }
                crate::DemTransferEvent::BytesReceived(bytes) => {
                    observed_bytes = observed_bytes.saturating_add(bytes);
                    transfer_activity.observe_bytes(&task_id, &item_id, 1, bytes, observed_at)
                }
                crate::DemTransferEvent::BodyStopped => {
                    transfer_activity.body_stopped(&task_id, &item_id, 1, observed_at)
                }
            };
            if sample.is_ok() {
                progress(DownloadItemProgress {
                    task_id: task_id.clone(),
                    item_id: item_id.clone(),
                    status: DownloadItemStatus::Active,
                    bytes_completed: observed_bytes,
                    total_bytes: None,
                    bytes_per_second: transfer_activity
                        .member_bytes_per_second(&task_id, &item_id, 1)
                        .unwrap_or(0.0),
                    attempt: 1,
                    updated_at: now(),
                });
            }
        },
    );

    drop(stage_dir_guard);

    if control.is_cancelled() {
        return DownloadItemResult {
            task_id,
            item_id,
            outcome: DownloadOutcome::Cancelled,
            destination,
            bytes_written: 0,
            attempt_count: 0,
            failure: None,
            finished_at: now(),
        };
    }
    if control.is_paused() {
        return DownloadItemResult {
            task_id,
            item_id,
            outcome: DownloadOutcome::Interrupted,
            destination,
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: now(),
        };
    }

    match operation_result {
        Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(true) => {
            let bytes_written = result
                .get("bytes_written")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            progress(DownloadItemProgress {
                task_id: task_id.clone(),
                item_id: item_id.clone(),
                status: DownloadItemStatus::Completed,
                bytes_completed: bytes_written,
                total_bytes: Some(bytes_written),
                bytes_per_second: 0.0,
                attempt: 1,
                updated_at: now(),
            });
            DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Completed,
                destination,
                bytes_written,
                attempt_count: 1,
                failure: None,
                finished_at: now(),
            }
        }
        result => DownloadItemResult {
            task_id,
            item_id,
            outcome: DownloadOutcome::Failed,
            destination,
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(optical_derived_download_failure(
                result.as_ref().err().map(String::as_str),
            )),
            finished_at: now(),
        },
    }
}

fn optical_derived_download_failure(error: Option<&str>) -> insar_core::DownloadFailure {
    let raw = error.unwrap_or("OPTICAL_DERIVED_EXECUTION_FAILED");
    let code = raw
        .split(':')
        .next()
        .map(str::trim)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        })
        .unwrap_or("OPTICAL_DERIVED_EXECUTION_FAILED")
        .to_owned();
    let retryable = code.contains("SIGN") || code.contains("HTTP") || code.contains("TIMEOUT");
    let reason_zh = match code.as_str() {
        "AOI_SCENE_DISJOINT" => "所选影像与 AOI 范围不相交，无法裁切导出".to_owned(),
        "OPTICAL_MOSAIC_TEMPORAL_SPAN_EXCEEDED" => {
            "所选多景影像时间跨度超过 16 天上限，请先缩窄时间范围".to_owned()
        }
        "OPTICAL_CLOUD_MASK_BAND_MISSING" => {
            "缺少云掩膜质量波段，无法执行云像素转 NoData".to_owned()
        }
        "OPTICAL_CLOUD_MASK_UNSUPPORTED_L1C" => {
            "Sentinel-2 L1C 顶层大气产品不含 SCL 云掩膜波段，不支持将云像素置为 NoData".to_owned()
        }
        "OPTICAL_OUTPUT_ALREADY_EXISTS" => "目标导出文件已存在且未允许覆盖".to_owned(),
        _ => format!("光学 AOI 裁切或镶嵌执行失败（{code}）"),
    };
    insar_core::DownloadFailure {
        code,
        reason_zh,
        retryable,
    }
}

#[cfg(test)]
mod risk_audit_tests {
    use super::{read_network_policy_for_store_path, StageDirGuard};
    use crate::network_routes::{default_policy, ServiceRoute};

    #[test]
    fn stage_dir_guard_cleans_temporary_directory_on_drop() {
        let root = std::env::temp_dir().join(format!(
            "insar-optical-stage-guard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let stage = root.join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(stage.join("band.jp2"), b"partial").unwrap();

        {
            let guard = StageDirGuard(Some(stage.clone()));
            assert!(guard.0.is_some());
        }
        assert!(
            !stage.exists(),
            "StageDirGuard must remove stage directory on drop"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_network_policy_distinguishes_missing_and_invalid_settings_without_panicking() {
        let root = std::env::temp_dir().join(format!(
            "insar-optical-policy-read-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store_path = root.join("tasks.sqlite3");
        let expected_default = default_policy(ServiceRoute::Ordinary);

        assert_eq!(
            read_network_policy_for_store_path(&store_path),
            expected_default
        );

        std::fs::write(root.join("network-settings.json"), b"{invalid json").unwrap();
        assert_eq!(
            read_network_policy_for_store_path(&store_path),
            expected_default
        );

        std::fs::write(
            root.join("network-settings.json"),
            br#"{"proxy_enabled":true,"proxy_url":"not-a-valid-url"}"#,
        )
        .unwrap();
        assert_eq!(
            read_network_policy_for_store_path(&store_path),
            expected_default
        );

        let valid_proxy = crate::ApplicationNetworkSettings {
            proxy_enabled: true,
            proxy_url: "http://127.0.0.1:7890".to_owned(),
            ..Default::default()
        };
        std::fs::write(
            root.join("network-settings.json"),
            serde_json::to_vec(&valid_proxy).unwrap(),
        )
        .unwrap();
        let expected_manual = insar_core::ProviderNetworkPolicy::manual(
            insar_core::ManualProxy::new("127.0.0.1", 7890).unwrap(),
        );
        let parsed_manual = read_network_policy_for_store_path(&store_path);
        assert_ne!(parsed_manual, expected_default);
        assert_eq!(parsed_manual, expected_manual);

        let _ = std::fs::remove_dir_all(&root);
    }
}
