use super::*;
use insar_store::StoredWorkbench;

/// Product admission is independent of the number of transfer files.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpticalDeliveryAdmissionRejection {
    pub source_item_id: String,
    pub reason_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_detail: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpticalDeliveryAdmissionSummary {
    pub requested_count: u64,
    pub admitted_count: u64,
    pub rejected_count: u64,
    pub rejections: Vec<OpticalDeliveryAdmissionRejection>,
}

impl OpticalDeliveryAdmissionSummary {
    pub(super) fn validate(&self) -> Result<(), ApplicationServiceError> {
        if self.requested_count == 0
            || self.requested_count > 250
            || self.admitted_count.checked_add(self.rejected_count) != Some(self.requested_count)
            || self.rejections.len() as u64 != self.rejected_count
            || self.rejections.iter().any(|item| {
                !valid_catalogue_opaque_id(&item.source_item_id)
                    || item.reason_code.is_empty()
                    || item.reason_code.len() > 128
                    || !item
                        .reason_code
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                    || item
                        .reason_detail
                        .as_deref()
                        .is_some_and(|detail| !valid_admission_reason_detail(detail))
            })
            || self
                .rejections
                .iter()
                .map(|item| &item.source_item_id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.rejections.len()
        {
            return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
        }
        Ok(())
    }
}

/// Non-zero page size for admitting from a named frozen batch that is not the
/// active Workbench slot. It is the page size the unified search freezes with
/// and only satisfies the frozen-evidence wrapper's non-zero requirement; the
/// mutable slot page size has no authority over the named batch.
const LANDSAT_BATCH_NAMED_PAGE_SIZE: u64 = 100;

const ADMISSION_REASON_DETAIL_KEYS: [&str; 6] = [
    "reason",
    "http_status",
    "retry_after_seconds",
    "retry_after",
    "transport",
    "attempts",
];

/// Persisted with the task report, so only allowlisted `key=value` pairs pass:
/// never a URL, header value or credential.
fn valid_admission_reason_detail(detail: &str) -> bool {
    let mut seen = BTreeSet::new();
    !detail.is_empty()
        && detail.len() <= 128
        && detail.split(';').all(|pair| {
            let Some((key, value)) = pair.split_once('=') else {
                return false;
            };
            seen.insert(key)
                && match key {
                    "reason" => {
                        (1..=32).contains(&value.len())
                            && value.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                    }
                    "http_status" => {
                        value.len() == 3
                            && value.bytes().all(|b| b.is_ascii_digit())
                            && value
                                .parse::<u16>()
                                .is_ok_and(|status| (100..=599).contains(&status))
                    }
                    "retry_after_seconds" => {
                        (1..=4).contains(&value.len())
                            && value.bytes().all(|b| b.is_ascii_digit())
                            && value.parse::<u16>().is_ok_and(|seconds| seconds <= 3_600)
                    }
                    "retry_after" => matches!(value, "http_date" | "invalid"),
                    "transport" => matches!(value, "timeout" | "connect" | "other"),
                    "attempts" => matches!(value.as_bytes(), [b'1'..=b'9']),
                    _ => false,
                }
        })
}

/// Only MPC sign failures carry a detail; anything outside the allowlist is dropped.
fn admission_reason_detail(error: &ApplicationServiceError) -> Option<String> {
    let ApplicationServiceError::Provider(error) = error else {
        return None;
    };
    if !error.code.starts_with("MPC_ASSET_SIGN_") {
        return None;
    }
    let mut parts = Vec::new();
    for key in ADMISSION_REASON_DETAIL_KEYS {
        match error.debug_context.get(key) {
            None => {}
            Some(Value::String(value)) => parts.push(format!("{key}={value}")),
            Some(_) => return None,
        }
    }
    let detail = parts.join(";");
    valid_admission_reason_detail(&detail).then_some(detail)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LandsatCreateAssetBatchTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    pub expected_workbench_revision: u64,
    pub item_ids: Vec<String>,
    #[serde(default)]
    pub asset_keys: Vec<String>,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct LandsatCreateAssetBatchTaskResult {
    /// None means all products were rejected and no Store task was written.
    pub snapshot: Option<CoreServiceSnapshot>,
    pub admission: OpticalDeliveryAdmissionSummary,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sentinel2CreateBandBatchTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    pub expected_workbench_revision: u64,
    pub item_ids: Vec<String>,
    pub asset_keys: Vec<String>,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

pub type Sentinel2CreateBandBatchTaskResult = LandsatCreateAssetBatchTaskResult;
pub type Sentinel2L1cCreateBandBatchTaskRequest = Sentinel2CreateBandBatchTaskRequest;
pub type Sentinel2L1cCreateBandBatchTaskResult = LandsatCreateAssetBatchTaskResult;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HlsCreateBandBatchTaskRequest {
    pub request_id: String,
    pub source_batch_id: String,
    pub expected_workbench_revision: u64,
    pub item_ids: Vec<String>,
    pub asset_keys: Vec<String>,
    pub task_id: String,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    pub created_at: String,
}

pub type HlsCreateBandBatchTaskResult = LandsatCreateAssetBatchTaskResult;

/// The archive command shares the bounded batch envelope, never the delivery route.
pub type LandsatCreateBundleBatchTaskRequest = LandsatCreateAssetBatchTaskRequest;
pub type LandsatCreateBundleBatchTaskResult = LandsatCreateAssetBatchTaskResult;

#[derive(Clone, Copy)]
enum OpticalAssetBatchKind<'a> {
    Landsat {
        keys: &'a [String],
    },
    LandsatArchive,
    Sentinel2 {
        keys: &'a [String],
        evidence: &'a dyn LiveSentinel2BandEvidence,
    },
    Sentinel2L1c {
        keys: &'a [String],
        network_policy: &'a ProviderNetworkPolicy,
    },
    Hls {
        keys: &'a [String],
    },
}

impl OpticalAssetBatchKind<'_> {
    fn invalid(self) -> ApplicationServiceError {
        match self {
            Self::Landsat { .. } => ApplicationServiceError::LandsatAssetEvidenceInvalid,
            Self::LandsatArchive => ApplicationServiceError::LandsatBundleEvidenceInvalid,
            Self::Sentinel2 { .. } | Self::Sentinel2L1c { .. } => {
                ApplicationServiceError::Sentinel2BandEvidenceInvalid
            }
            Self::Hls { .. } => ApplicationServiceError::InvalidRequest {
                reason: "HLS multi-band delivery evidence is invalid".to_owned(),
            },
        }
    }
    fn cancelled(self) -> ApplicationServiceError {
        match self {
            Self::Landsat { .. } => ApplicationServiceError::LandsatAssetCancelled,
            Self::LandsatArchive => ApplicationServiceError::LandsatBundleCancelled,
            Self::Sentinel2 { .. } | Self::Sentinel2L1c { .. } => {
                ApplicationServiceError::Sentinel2BandCancelled
            }
            Self::Hls { .. } => ApplicationServiceError::LandsatAssetCancelled,
        }
    }
    fn duplicate(self) -> ApplicationServiceError {
        match self {
            Self::Landsat { .. } => ApplicationServiceError::LandsatAssetRequestDuplicate,
            Self::LandsatArchive => ApplicationServiceError::LandsatBundleRequestDuplicate,
            Self::Sentinel2 { .. } | Self::Sentinel2L1c { .. } => {
                ApplicationServiceError::Sentinel2BandRequestDuplicate
            }
            Self::Hls { .. } => ApplicationServiceError::InvalidRequest {
                reason: "HLS multi-band delivery request duplicate".to_owned(),
            },
        }
    }
    fn prefix(self) -> &'static str {
        match self {
            Self::Landsat { .. } => "landsat-assets-batch-",
            Self::LandsatArchive => "landsat-bundles-batch-",
            Self::Sentinel2 { .. } => "sentinel2-bands-batch-",
            Self::Sentinel2L1c { .. } => "sentinel2-l1c-bands-batch-",
            Self::Hls { .. } => "hls-bands-batch-",
        }
    }
    fn delivery_kind(self) -> &'static str {
        match self {
            Self::Landsat { .. } => "landsat_c2_l2_asset_batch",
            Self::LandsatArchive => "landsat_c2_l2_bundle_batch",
            Self::Sentinel2 { .. } => "sentinel2_l2a_analysis_band_batch",
            Self::Sentinel2L1c { .. } => "sentinel2_l1c_band_batch",
            Self::Hls { .. } => "hls_multiband_batch",
        }
    }
    fn source_allowed(self, provider: &str) -> bool {
        match self {
            Self::Landsat { .. } => provider == insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID,
            Self::LandsatArchive => matches!(
                provider,
                LANDSAT_C2_L2_PROVIDER_ID
                    | insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
                    | insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID
            ),
            Self::Sentinel2 { .. } => matches!(
                provider,
                "microsoft.planetary-computer"
                    | "copernicus.dataspace.odata"
                    | insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
            ),
            Self::Sentinel2L1c { .. } => provider == "copernicus.dataspace.odata",
            Self::Hls { .. } => provider == insar_provider::catalog::MPC_HLS_PROVIDER_ID,
        }
    }
    fn prepare(
        self,
        service: &ApplicationService,
        source: &FrozenSearchBatch,
        page_size: u64,
        item: &SnapshotItem,
        cancelled: &AtomicBool,
    ) -> Result<Vec<SnapshotItem>, ApplicationServiceError> {
        match self {
            Self::Landsat { keys } => {
                let evidence = frozen_landsat_scene_evidence(source, page_size, item)?;
                Ok(service
                    .prepare_landsat_asset_items(&evidence, item, keys, cancelled)?
                    .items)
            }
            Self::LandsatArchive => {
                let evidence = frozen_landsat_archive_source_evidence(source, page_size, item)?;
                let (prepared, selected_id) =
                    service.prepare_landsat_archive_source(&evidence, &item.item_id, cancelled)?;
                let mut archive = prepared
                    .candidates
                    .into_iter()
                    .find(|candidate| candidate.item_id == selected_id)
                    .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
                archive.metadata.insert(
                    "source_item_id".to_owned(),
                    Value::String(item.item_id.clone()),
                );
                Ok(vec![archive])
            }
            Self::Sentinel2 { keys, evidence } => {
                let identity = frozen_sentinel2_analysis_source(source, item)?;
                let (directory, mut items) = sentinel2_band_delivery::prepare_sentinel2_band_items(
                    item, &identity, keys, evidence, cancelled,
                )?;
                for asset in &mut items {
                    asset.metadata.insert(
                        "source_item_id".to_owned(),
                        Value::String(item.item_id.clone()),
                    );
                    asset.metadata.insert(
                        "delivery_product_directory".to_owned(),
                        Value::String(directory.clone()),
                    );
                }
                Ok(items)
            }
            Self::Sentinel2L1c {
                keys,
                network_policy,
            } => {
                if source.query.get("logical_dataset").and_then(Value::as_str)
                    != Some("optical.sentinel-2.l1c")
                {
                    return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
                }
                let (_, items) = sentinel2_band_delivery::prepare_sentinel2_l1c_band_items(
                    item,
                    keys,
                    *network_policy,
                    cancelled,
                )?;
                Ok(items)
            }
            Self::Hls { keys } => {
                crate::hls_catalogue::prepare_hls_band_items(item, keys, cancelled)
            }
        }
    }
}

pub(super) struct PreparedLandsatAssets {
    pub items: Vec<SnapshotItem>,
    pub display_id: String,
    pub complete: bool,
}

impl ApplicationService {
    /// Single and batch archives share exact product matching, option validation
    /// and frozen execution policy. No asset-set fallback is permitted here.
    pub(super) fn prepare_landsat_archive_source(
        &self,
        evidence: &FrozenLandsatBundleEvidence,
        source_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<(FrozenSearchBatch, String), ApplicationServiceError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::LandsatBundleCancelled);
        }
        let mut batch = self.resolve_landsat_archive_source(evidence, source_id, cancelled)?;
        let selected_id = if evidence.source_batch.provider_id == LANDSAT_C2_L2_PROVIDER_ID {
            source_id.to_owned()
        } else {
            match batch.candidates.as_slice() {
                [item] if item.display_name == evidence.display_id => item.item_id.clone(),
                _ => return Err(ApplicationServiceError::LandsatBundleEvidenceInvalid),
            }
        };
        let item = batch
            .candidates
            .iter()
            .find(|item| item.item_id == selected_id)
            .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
        let official = frozen_landsat_bundle_evidence(&batch, evidence.page_size, item)?;
        let option = self
            .landsat_bundle_options_port()?
            .full_bundle_option(
                &official.dataset_alias,
                &official.entity_id,
                &official.display_id,
                &|| cancelled.load(Ordering::Acquire),
            )
            .map_err(ApplicationServiceError::UsgsM2m)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::LandsatBundleCancelled);
        }
        let access =
            build_landsat_c2_l2_bundle_access(item, &option, CatalogPolicyContext::Production)
                .map_err(|_| ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
        let item = batch
            .candidates
            .iter_mut()
            .find(|item| item.item_id == selected_id)
            .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
        item.asset_access = Some(access);
        item.execution_policy = Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: USGS_LANDSAT_C2_L2_POLICY_ID.to_owned(),
            collection_profile_id: Some(LANDSAT_C2_L2_SCENE_PROFILE_ID.to_owned()),
        });
        Ok((batch, selected_id))
    }

    pub(super) fn resolve_landsat_archive_source(
        &self,
        evidence: &FrozenLandsatBundleEvidence,
        source_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<FrozenSearchBatch, ApplicationServiceError> {
        if evidence.source_batch.provider_id == LANDSAT_C2_L2_PROVIDER_ID {
            return Ok(evidence.source_batch.clone());
        }
        if !matches!(
            evidence.source_batch.provider_id.as_str(),
            insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
                | insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID
        ) {
            return Err(ApplicationServiceError::LandsatBundleEvidenceInvalid);
        }
        let source_item = evidence
            .source_batch
            .candidates
            .iter()
            .find(|item| item.item_id == source_id)
            .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
        let product = self
            .landsat_bundle_options_port()?
            .resolve_official_product(&evidence.display_id, &|| cancelled.load(Ordering::Acquire))
            .map_err(ApplicationServiceError::UsgsM2m)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::LandsatBundleCancelled);
        }
        let source_properties = source_item
            .metadata
            .get("properties")
            .and_then(Value::as_object)
            .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
        if product.provider_id != LANDSAT_C2_L2_PROVIDER_ID
            || product.display_name != evidence.display_id
            || [
                "landsat_product_id",
                "spacecraft",
                "processing_level",
                "collection_category",
                "wrs_path",
                "wrs_row",
                "acquisition_date",
            ]
            .iter()
            .any(|key| {
                product.properties.get(*key).is_none()
                    || product.properties.get(*key) != source_properties.get(*key)
            })
        {
            return Err(ApplicationServiceError::LandsatBundleEvidenceInvalid);
        }
        // Provider-specific data_type_l2 labels are not cross-source identity:
        // MPC uses LANDSAT_9_L2SP, USGS uses OLI_TIRS_L2SP. Both are independently
        // validated against processing_level by their existing source mappers.
        let mut query: CatalogSearchQuery =
            serde_json::from_value(Value::Object(evidence.source_batch.query.clone()))
                .map_err(|_| ApplicationServiceError::LandsatBundleEvidenceInvalid)?;
        query.provider_id = LANDSAT_C2_L2_PROVIDER_ID.to_owned();
        query.native_collection_id =
            insar_provider::catalog::LANDSAT_C2_L2_NATIVE_COLLECTION_ID.to_owned();
        let mut official_item = map_landsat_scene_product(product, &query)?;
        official_item.metadata.insert(
            "source_item_id".to_owned(),
            Value::String(source_id.to_owned()),
        );
        let mut batch = evidence.source_batch.clone();
        batch.provider_id = LANDSAT_C2_L2_PROVIDER_ID.to_owned();
        batch.query = serde_json::to_value(query)
            .map_err(|_| ApplicationServiceError::LandsatBundleEvidenceInvalid)?
            .as_object()
            .ok_or(ApplicationServiceError::LandsatBundleEvidenceInvalid)?
            .clone();
        batch.search_stats = None;
        batch.candidates = vec![official_item];
        Ok(batch)
    }

    /// Shared with the established single-scene route: no relaxed identity,
    /// manifest, unsigned-locator or execution-policy validation for batches.
    pub(super) fn prepare_landsat_asset_items(
        &self,
        evidence: &FrozenLandsatBundleEvidence,
        source_item: &SnapshotItem,
        asset_keys: &[String],
        cancelled: &AtomicBool,
    ) -> Result<PreparedLandsatAssets, ApplicationServiceError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::LandsatAssetCancelled);
        }
        let manifest = self
            .landsat_asset_manifest_port()?
            .landsat_asset_manifest(
                &evidence.dataset_alias,
                &evidence.entity_id,
                &evidence.display_id,
                &|| cancelled.load(Ordering::Acquire),
            )
            .map_err(ApplicationServiceError::PlanetaryComputer)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::LandsatAssetCancelled);
        }
        let source_profile = CollectionProfileRegistry
            .for_source(
                LANDSAT_C2_L2_LOGICAL_DATASET,
                &source_item.provider_id,
                &evidence.dataset_alias,
            )
            .ok_or_else(|| {
                crate::landsat_asset_diagnostic(&evidence.display_id, "prepare_profile_missing");
                ApplicationServiceError::LandsatAssetEvidenceInvalid
            })?;
        let policy_id =
            if source_profile.provider_id == insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID {
                insar_provider::catalog::MPC_LANDSAT_ASSET_POLICY_ID
            } else {
                PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID
            };
        let selected_assets = manifest
            .assets
            .iter()
            .filter(|asset| asset_keys.is_empty() || asset_keys.contains(&asset.asset_key))
            .collect::<Vec<_>>();
        if selected_assets.is_empty()
            || (!asset_keys.is_empty() && selected_assets.len() != asset_keys.len())
        {
            crate::landsat_asset_diagnostic(
                &evidence.display_id,
                "prepare_asset_selection_mismatch",
            );
            return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
        }
        let mut items = Vec::with_capacity(selected_assets.len());
        for asset in selected_assets {
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::LandsatAssetCancelled);
            }
            let access =
                build_planetary_computer_landsat_asset_access(source_item, &manifest, asset)
                    .map_err(|error| {
                        crate::landsat_asset_diagnostic(
                            &evidence.display_id,
                            &format!(
                                "prepare_access_build_failed:{:?}::{}",
                                error, asset.asset_key
                            ),
                        );
                        ApplicationServiceError::LandsatAssetEvidenceInvalid
                    })?;
            let mut metadata = source_item.metadata.clone();
            metadata.insert(
                "source_item_id".to_owned(),
                Value::String(source_item.item_id.clone()),
            );
            metadata.insert(
                "delivery_asset_key".to_owned(),
                Value::String(asset.asset_key.clone()),
            );
            metadata.insert(
                "delivery_asset_role".to_owned(),
                serde_json::to_value(&asset.role)
                    .map_err(|_| ApplicationServiceError::LandsatAssetEvidenceInvalid)?,
            );
            metadata.insert(
                "delivery_source".to_owned(),
                Value::String("planetary_computer".to_owned()),
            );
            items.push(SnapshotItem {
                item_id: access.reference.stable_asset_key.clone(),
                provider_id: source_item.provider_id.clone(),
                display_name: asset.file_name.clone(),
                acquisition_datetime: source_item.acquisition_datetime.clone(),
                footprint_geojson: source_item.footprint_geojson.clone(),
                metadata,
                payload: Map::new(),
                asset_access: Some(access),
                execution_policy: Some(AssetExecutionPolicy {
                    authority: AssetExecutionAuthority::ResolverAuthoritative,
                    policy_id: policy_id.to_owned(),
                    collection_profile_id: Some(source_profile.profile_id.to_owned()),
                }),
            });
        }
        Ok(PreparedLandsatAssets {
            complete: asset_keys.is_empty() || asset_keys.len() == manifest.assets.len(),
            display_id: manifest.display_id,
            items,
        })
    }

    pub fn create_landsat_asset_batch_task(
        &self,
        request: &LandsatCreateAssetBatchTaskRequest,
    ) -> Result<LandsatCreateAssetBatchTaskResult, ApplicationServiceError> {
        if request.asset_keys.len() > 24
            || request.asset_keys.iter().any(|k| k.trim().is_empty())
            || request.asset_keys.iter().collect::<BTreeSet<_>>().len() != request.asset_keys.len()
        {
            return Err(invalid_request(
                "Landsat batch asset_keys selection is invalid",
            ));
        }
        self.create_optical_asset_batch_task(
            request,
            OpticalAssetBatchKind::Landsat {
                keys: &request.asset_keys,
            },
        )
    }

    pub fn create_landsat_bundle_batch_task(
        &self,
        request: &LandsatCreateBundleBatchTaskRequest,
    ) -> Result<LandsatCreateBundleBatchTaskResult, ApplicationServiceError> {
        self.ensure_usgs_controls_available()?;
        if matches!(
            self.business_factory.as_ref(),
            Some(BusinessRuntimeFactory::Production { .. })
        ) {
            self.ensure_usgs_for_admission()?;
        }
        self.create_optical_asset_batch_task(request, OpticalAssetBatchKind::LandsatArchive)
    }

    pub fn create_sentinel2_band_batch_task(
        &self,
        request: &Sentinel2CreateBandBatchTaskRequest,
    ) -> Result<Sentinel2CreateBandBatchTaskResult, ApplicationServiceError> {
        self.ensure_running()?;
        let policy = self.cdse_session_runtime()?.network_policy;
        let evidence =
            Sentinel2BandEvidenceRuntime::new(policy).map_err(ApplicationServiceError::Provider)?;
        self.create_sentinel2_band_batch_task_with_evidence_for_test(request, &evidence)
    }

    #[doc(hidden)]
    pub fn create_sentinel2_band_batch_task_with_evidence_for_test(
        &self,
        request: &Sentinel2CreateBandBatchTaskRequest,
        evidence: &dyn LiveSentinel2BandEvidence,
    ) -> Result<Sentinel2CreateBandBatchTaskResult, ApplicationServiceError> {
        if request.request_id.len() > 80
            || request.task_id.len() > 72
            || request.asset_keys.is_empty()
            || request.asset_keys.len() > 12
            || request
                .asset_keys
                .iter()
                .any(|key| sentinel2_l2a_reflectance_band_resolution(key).is_none())
            || request.asset_keys.iter().collect::<BTreeSet<_>>().len() != request.asset_keys.len()
        {
            return Err(invalid_request(
                "Sentinel-2 analysis-band selection is invalid",
            ));
        }
        let common = LandsatCreateAssetBatchTaskRequest {
            request_id: request.request_id.clone(),
            source_batch_id: request.source_batch_id.clone(),
            expected_workbench_revision: request.expected_workbench_revision,
            item_ids: request.item_ids.clone(),
            asset_keys: Vec::new(),
            task_id: request.task_id.clone(),
            output_root: request.output_root.clone(),
            layout: request.layout.clone(),
            concurrency: request.concurrency,
            created_at: request.created_at.clone(),
        };
        self.create_optical_asset_batch_task(
            &common,
            OpticalAssetBatchKind::Sentinel2 {
                keys: &request.asset_keys,
                evidence,
            },
        )
    }

    pub fn create_sentinel2_l1c_band_batch_task(
        &self,
        request: &Sentinel2L1cCreateBandBatchTaskRequest,
    ) -> Result<Sentinel2L1cCreateBandBatchTaskResult, ApplicationServiceError> {
        self.ensure_running()?;
        if request.request_id.len() > 80
            || request.task_id.len() > 72
            || request.asset_keys.is_empty()
            || request.asset_keys.len() > sentinel2_band_delivery::SENTINEL2_L1C_BAND_ORDER.len()
            || request
                .asset_keys
                .iter()
                .any(|key| sentinel2_band_delivery::sentinel2_l1c_band_resolution(key).is_none())
            || request
                .asset_keys
                .iter()
                .map(|key| key.to_ascii_uppercase())
                .collect::<BTreeSet<_>>()
                .len()
                != request.asset_keys.len()
        {
            return Err(invalid_request("Sentinel-2 L1C band selection is invalid"));
        }
        self.apply_pending_network_policy();
        let network_policy = self.cdse_session_runtime()?.network_policy;
        let common = LandsatCreateAssetBatchTaskRequest {
            request_id: request.request_id.clone(),
            source_batch_id: request.source_batch_id.clone(),
            expected_workbench_revision: request.expected_workbench_revision,
            item_ids: request.item_ids.clone(),
            asset_keys: Vec::new(),
            task_id: request.task_id.clone(),
            output_root: request.output_root.clone(),
            layout: request.layout.clone(),
            concurrency: request.concurrency,
            created_at: request.created_at.clone(),
        };
        self.create_optical_asset_batch_task(
            &common,
            OpticalAssetBatchKind::Sentinel2L1c {
                keys: &request.asset_keys,
                network_policy: &network_policy,
            },
        )
    }

    pub fn create_hls_band_batch_task(
        &self,
        request: &HlsCreateBandBatchTaskRequest,
    ) -> Result<HlsCreateBandBatchTaskResult, ApplicationServiceError> {
        self.ensure_running()?;
        if request.request_id.len() > 80
            || request.task_id.len() > 72
            || request.asset_keys.is_empty()
            || request.asset_keys.len() > 14
            || request.asset_keys.iter().collect::<BTreeSet<_>>().len() != request.asset_keys.len()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "HLS multi-band selection is invalid".to_owned(),
            });
        }
        let common = LandsatCreateAssetBatchTaskRequest {
            request_id: request.request_id.clone(),
            source_batch_id: request.source_batch_id.clone(),
            expected_workbench_revision: request.expected_workbench_revision,
            item_ids: request.item_ids.clone(),
            asset_keys: Vec::new(),
            task_id: request.task_id.clone(),
            output_root: request.output_root.clone(),
            layout: request.layout.clone(),
            concurrency: request.concurrency,
            created_at: request.created_at.clone(),
        };
        self.create_optical_asset_batch_task(
            &common,
            OpticalAssetBatchKind::Hls {
                keys: &request.asset_keys,
            },
        )
    }

    /// Optical deliveries share one admission pass and one Store transaction.
    fn create_optical_asset_batch_task(
        &self,
        request: &LandsatCreateAssetBatchTaskRequest,
        kind: OpticalAssetBatchKind<'_>,
    ) -> Result<LandsatCreateAssetBatchTaskResult, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(&request.request_id)
            || !valid_catalogue_opaque_id(&request.source_batch_id)
            || !valid_catalogue_opaque_id(&request.task_id)
            || request.task_id.len() > 96
            || request.output_root.as_os_str().is_empty()
            || request.layout != DownloadLayout::Direct
            || !(1..=8).contains(&request.concurrency)
            || parse_normalized_utc(&request.created_at).is_none()
            || request.item_ids.len() > 250
            || request
                .item_ids
                .iter()
                .any(|id| !valid_catalogue_opaque_id(id))
            || request.item_ids.iter().collect::<BTreeSet<_>>().len() != request.item_ids.len()
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "Optical asset batch task input is invalid".to_owned(),
            });
        }
        if self
            .dispatcher_handle()?
            .catalogue_route_use()
            .map_err(ApplicationServiceError::Provider)?
            != RouteUse::Production
        {
            return Err(kind.invalid());
        }
        let cancelled = self
            .quicklook_cancellations
            .try_register(&request.request_id)
            .ok_or_else(|| kind.duplicate())?;
        let result = (|| {
            let (source, selected_item_ids) = self.with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                // Unified search keeps one frozen batch per source and the
                // mutable Workbench slot may hold another source's results by
                // the time the user downloads (burst-fix pattern: admit from
                // the exact frozen batch named by the request). The slot is
                // still authoritative for the Store-owned checked selection
                // and its revision, so an empty explicit selection keeps the
                // previous slot-based admission.
                let source = if request.item_ids.is_empty() {
                    let source = store.load_workbench()?.ok_or_else(|| kind.invalid())?;
                    if source.batch.batch_id != request.source_batch_id
                        || source.state.revision != request.expected_workbench_revision
                        || !kind.source_allowed(&source.batch.provider_id)
                    {
                        return Err(kind.invalid());
                    }
                    source
                } else {
                    let batch = store
                        .load_search_batch(&request.source_batch_id)?
                        .ok_or_else(|| kind.invalid())?;
                    if !kind.source_allowed(&batch.provider_id) {
                        return Err(kind.invalid());
                    }
                    // An explicit selection admits from the exact frozen batch
                    // the request names. If that batch is the active Workbench
                    // slot, the request revision must equal the slot's current
                    // revision; an inactive unified-search batch was frozen at
                    // revision 0 and only accepts revision 0. Anything else
                    // means the caller assumes a revision this batch never
                    // owned.
                    let active = store.load_workbench()?;
                    let expected_revision = if active
                        .as_ref()
                        .is_some_and(|slot| slot.batch.batch_id == batch.batch_id)
                    {
                        active.as_ref().expect("checked above").state.revision
                    } else {
                        0
                    };
                    if request.expected_workbench_revision != expected_revision {
                        return Err(kind.invalid());
                    }
                    StoredWorkbench {
                        state: SearchWorkbenchState {
                            batch_id: batch.batch_id.clone(),
                            checked_item_ids: Vec::new(),
                            page_index: 0,
                            page_size: LANDSAT_BATCH_NAMED_PAGE_SIZE,
                            revision: expected_revision,
                        },
                        batch,
                    }
                };
                let selected_item_ids = if request.item_ids.is_empty() {
                    source.state.checked_item_ids.clone()
                } else {
                    request.item_ids.clone()
                };
                if selected_item_ids.is_empty()
                    || selected_item_ids.len() > 250
                    || selected_item_ids
                        .iter()
                        .any(|id| !valid_catalogue_opaque_id(id))
                    || selected_item_ids.iter().collect::<BTreeSet<_>>().len()
                        != selected_item_ids.len()
                {
                    return Err(kind.invalid());
                }
                Ok((source, selected_item_ids))
            })?;
            let progress = self
                .optical_preparations
                .begin(&request.request_id, selected_item_ids.len())?;
            let by_id = source
                .batch
                .candidates
                .iter()
                .map(|item| (item.item_id.as_str(), item))
                .collect::<BTreeMap<_, _>>();
            let mut items = Vec::new();
            let mut admission = OpticalDeliveryAdmissionSummary {
                requested_count: selected_item_ids.len() as u64,
                admitted_count: 0,
                rejected_count: 0,
                rejections: Vec::new(),
            };
            // Bound both live manifests and the temporary per-scene evidence clones.
            for chunk in selected_item_ids.chunks(2) {
                if cancelled.load(Ordering::Acquire) {
                    return Err(kind.cancelled());
                }
                let outcomes = std::thread::scope(|scope| {
                    let handles = chunk
                        .iter()
                        .map(|id| {
                            let source = &source;
                            let item = by_id.get(id.as_str()).copied();
                            let cancelled = &cancelled;
                            let progress = &progress;
                            scope.spawn(move || {
                                let result = item.ok_or_else(|| kind.invalid()).and_then(|item| {
                                    kind.prepare(
                                        self,
                                        &source.batch,
                                        source.state.page_size,
                                        item,
                                        cancelled,
                                    )
                                });
                                progress.record(result.is_ok())?;
                                result
                            })
                        })
                        .collect::<Vec<_>>();
                    handles
                        .into_iter()
                        .map(|handle| {
                            handle
                                .join()
                                .map_err(|_| ApplicationServiceError::RuntimeInitialization)
                        })
                        .collect::<Result<Vec<_>, _>>()
                })?;
                for (id, outcome) in chunk.iter().zip(outcomes) {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(kind.cancelled());
                    }
                    match outcome {
                        Ok(prepared) => {
                            admission.admitted_count += 1;
                            items.extend(prepared);
                        }
                        Err(error) => {
                            admission.rejected_count += 1;
                            admission
                                .rejections
                                .push(OpticalDeliveryAdmissionRejection {
                                    source_item_id: id.clone(),
                                    reason_code: error.code().to_owned(),
                                    reason_detail: admission_reason_detail(&error),
                                });
                        }
                    }
                }
            }
            admission.validate().map_err(|_| kind.invalid())?;
            if admission.admitted_count == 0 {
                return Ok(LandsatCreateAssetBatchTaskResult {
                    snapshot: None,
                    admission,
                });
            }
            progress.committing()?;
            let ids = items
                .iter()
                .map(|item| item.item_id.clone())
                .collect::<Vec<_>>();
            if ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
                || items
                    .iter()
                    .map(|item| {
                        format!(
                            "{}/{}",
                            item.metadata
                                .get("delivery_product_directory")
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                            item.display_name
                        )
                        .to_ascii_lowercase()
                    })
                    .collect::<BTreeSet<_>>()
                    .len()
                    != items.len()
            {
                return Err(kind.invalid());
            }
            let mut batch = source.batch.clone();
            batch.batch_id = format!("{}{}", kind.prefix(), request.task_id);
            batch.created_at = request.created_at.clone();
            batch.search_stats = None;
            batch.candidates = items;
            if matches!(kind, OpticalAssetBatchKind::LandsatArchive) {
                let mut query: CatalogSearchQuery =
                    serde_json::from_value(Value::Object(batch.query.clone()))
                        .map_err(|_| kind.invalid())?;
                query.provider_id = LANDSAT_C2_L2_PROVIDER_ID.to_owned();
                query.native_collection_id =
                    insar_provider::catalog::LANDSAT_C2_L2_NATIVE_COLLECTION_ID.to_owned();
                query.cursor = None;
                query.validate().map_err(|_| kind.invalid())?;
                batch.provider_id = query.provider_id.clone();
                batch.query = serde_json::to_value(query)
                    .map_err(|_| kind.invalid())?
                    .as_object()
                    .cloned()
                    .ok_or_else(|| kind.invalid())?;
            }
            if let OpticalAssetBatchKind::Sentinel2 { keys, .. } = kind {
                let mut query: CatalogSearchQuery =
                    serde_json::from_value(Value::Object(batch.query.clone()))
                        .map_err(|_| kind.invalid())?;
                if source.batch.provider_id
                    == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
                {
                    query.provider_id =
                        insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID.to_owned();
                    query.native_collection_id =
                        insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID
                            .to_owned();
                } else {
                    query.provider_id = PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned();
                    query.native_collection_id =
                        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog
                            .collection_id()
                            .to_owned();
                }
                query.cursor = None;
                query.result_limit = Some(100);
                query.filters = Map::new();
                query.validate().map_err(|_| kind.invalid())?;
                batch.provider_id = query.provider_id.clone();
                batch.query = serde_json::to_value(query)
                    .map_err(|_| kind.invalid())?
                    .as_object()
                    .cloned()
                    .ok_or_else(|| kind.invalid())?;
                batch.query.insert(
                    "delivery_band_keys".to_owned(),
                    serde_json::to_value(keys).map_err(|_| kind.invalid())?,
                );
            }
            if let OpticalAssetBatchKind::Sentinel2L1c { keys, .. } = kind {
                batch.query.insert(
                    "delivery_band_keys".to_owned(),
                    serde_json::to_value(keys).map_err(|_| kind.invalid())?,
                );
            }
            if let OpticalAssetBatchKind::Landsat { keys } = kind {
                if !keys.is_empty() {
                    batch.query.insert(
                        "delivery_band_keys".to_owned(),
                        serde_json::to_value(keys).map_err(|_| kind.invalid())?,
                    );
                }
            }
            if let OpticalAssetBatchKind::Hls { keys } = kind {
                batch.query.insert(
                    "delivery_band_keys".to_owned(),
                    serde_json::to_value(keys).map_err(|_| kind.invalid())?,
                );
            }
            batch.query.insert(
                "derived_from_batch_id".to_owned(),
                Value::String(request.source_batch_id.clone()),
            );
            batch.query.insert(
                "delivery_kind".to_owned(),
                Value::String(kind.delivery_kind().to_owned()),
            );
            batch.query.insert(
                "delivery_asset_count".to_owned(),
                Value::from(ids.len() as u64),
            );
            batch.query.insert(
                "delivery_requested_count".to_owned(),
                Value::from(admission.requested_count),
            );
            batch.query.insert(
                "delivery_admitted_count".to_owned(),
                Value::from(admission.admitted_count),
            );
            batch.query.insert(
                "delivery_rejected_count".to_owned(),
                Value::from(admission.rejected_count),
            );
            batch.query.insert(
                "delivery_admission_rejections".to_owned(),
                serde_json::to_value(&admission.rejections).map_err(|_| kind.invalid())?,
            );
            batch.validate().map_err(|_| kind.invalid())?;
            validate_debug_context(&batch.query).map_err(|_| kind.invalid())?;
            let task = StoreCreateTaskRequest {
                task_id: request.task_id.clone(),
                batch_id: batch.batch_id.clone(),
                selected_item_ids: ids.clone(),
                checked_item_ids: ids.clone(),
                page_index: 0,
                page_size: source.state.page_size,
                output_root: request.output_root.clone(),
                layout: request.layout.clone(),
                concurrency: request.concurrency,
                created_at: request.created_at.clone(),
            };
            let batch = StoreSearchBatchRequest {
                workbench: SearchWorkbenchState {
                    batch_id: batch.batch_id.clone(),
                    checked_item_ids: ids,
                    page_index: 0,
                    page_size: source.state.page_size,
                    revision: 0,
                },
                batch,
                activate_workbench: false,
            };
            let snapshot = self.with_business_runtime(|runtime| {
                {
                    let mut store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    // Re-read the named source batch inside the write
                    // transaction and require it unchanged. For an explicit
                    // item selection the batch may be the active Workbench
                    // slot: the slot must still hold this batch at the same
                    // revision the request was admitted with (another
                    // source's search replacing the slot is irrelevant for an
                    // inactive batch, but a same-batch revision change means
                    // the selection identity moved).
                    let current = store
                        .load_search_batch(&request.source_batch_id)?
                        .ok_or_else(|| kind.invalid())?;
                    if current != source.batch {
                        return Err(kind.invalid());
                    }
                    if let Some(slot) = store.load_workbench()? {
                        if slot.batch.batch_id == source.batch.batch_id
                            && slot.state.revision != source.state.revision
                        {
                            return Err(kind.invalid());
                        }
                    }
                    if cancelled.load(Ordering::Acquire) {
                        return Err(kind.cancelled());
                    }
                    store.save_search_batch_and_create_task(&batch, &task)?;
                }
                runtime
                    .scheduler
                    .as_ref()
                    .ok_or(ApplicationServiceError::BusinessRuntimeUnavailable)?
                    .wake();
                Self::snapshot_from_runtime(runtime)
            })?;
            Ok(LandsatCreateAssetBatchTaskResult {
                snapshot: Some(snapshot),
                admission,
            })
        })();
        self.quicklook_cancellations.finish(&request.request_id);
        result
    }

    /// Read the durable admission report only when opening a task, not on every poll.
    pub fn task_delivery_admission(
        &self,
        task_id: &str,
    ) -> Result<Option<OpticalDeliveryAdmissionSummary>, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(task_id) {
            return Err(invalid_request("invalid task identity"));
        }
        self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let task = store
                .load_task(task_id)?
                .ok_or_else(|| invalid_request("task does not exist"))?;
            let query = &task.snapshot.query;
            if !matches!(
                query.get("delivery_kind").and_then(Value::as_str),
                Some(
                    "landsat_c2_l2_asset_batch"
                        | "landsat_c2_l2_bundle_batch"
                        | "sentinel2_cdse_safe_archive_batch"
                        | "sentinel2_l2a_analysis_band_batch"
                        | "sentinel2_l1c_band_batch"
                        | "hls_multiband_batch"
                )
            ) {
                return Ok(None);
            }
            let value = serde_json::json!({
                "requested_count": query.get("delivery_requested_count"),
                "admitted_count": query.get("delivery_admitted_count"),
                "rejected_count": query.get("delivery_rejected_count"),
                "rejections": query.get("delivery_admission_rejections"),
            });
            let report: OpticalDeliveryAdmissionSummary = serde_json::from_value(value)
                .map_err(|_| ApplicationServiceError::LandsatAssetEvidenceInvalid)?;
            report.validate()?;
            Ok(Some(report))
        })
    }
}

#[cfg(test)]
mod tests {
    use insar_core::ContractError;
    use serde_json::{Map, Value};

    use super::*;
    use crate::ApplicationServiceError;

    fn provider_error(code: &str, context: &[(&str, Value)]) -> ApplicationServiceError {
        let context = context
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()));
        ApplicationServiceError::Provider(
            ContractError::new(code, "fixture", Map::from_iter(context)).expect("safe fixture"),
        )
    }

    #[test]
    fn only_a_complete_valid_mpc_sign_context_becomes_an_admission_detail() {
        let text = |value: &str| Value::String(value.to_owned());
        assert_eq!(
            admission_reason_detail(&provider_error(
                "MPC_ASSET_SIGN_REJECTED",
                &[
                    ("attempts", text("1")),
                    ("http_status", text("403")),
                    ("reason", text("http_status_terminal")),
                ],
            )),
            Some("reason=http_status_terminal;http_status=403;attempts=1".to_owned())
        );
        for error in [
            provider_error("STAC_TIMEOUT", &[("reason", text("timeout"))]),
            provider_error(
                "MPC_ASSET_SIGN_UNAVAILABLE",
                &[
                    ("reason", text("http_status_transient")),
                    ("http_status", text("600")),
                ],
            ),
            provider_error(
                "MPC_ASSET_SIGN_TRANSPORT",
                &[("reason", text("transport")), ("attempts", Value::from(2))],
            ),
            provider_error("MPC_ASSET_SIGN_TRANSPORT", &[]),
            ApplicationServiceError::Sentinel2BandEvidenceInvalid,
        ] {
            assert_eq!(admission_reason_detail(&error), None, "{error:?}");
        }
    }

    fn summary_with_detail(detail: &str) -> OpticalDeliveryAdmissionSummary {
        OpticalDeliveryAdmissionSummary {
            requested_count: 2,
            admitted_count: 1,
            rejected_count: 1,
            rejections: vec![OpticalDeliveryAdmissionRejection {
                source_item_id: "scene-b".to_owned(),
                reason_code: "MPC_ASSET_SIGN_RATE_LIMITED".to_owned(),
                reason_detail: Some(detail.to_owned()),
            }],
        }
    }

    #[test]
    fn admission_reason_detail_is_a_bounded_allowlist_without_urls_or_secrets() {
        for detail in [
            "reason=http_status_transient;http_status=429;retry_after_seconds=3;attempts=4",
            "reason=http_status_terminal;http_status=403;attempts=1",
            "reason=transport;transport=timeout;attempts=2",
            "reason=http_status_transient;http_status=429;retry_after=http_date;attempts=4",
            "reason=response_json",
        ] {
            assert!(summary_with_detail(detail).validate().is_ok(), "{detail}");
        }
        let overlong_reason = format!("reason={}", "a".repeat(33));
        for detail in [
            "",
            "https://planetarycomputer.microsoft.com/api/sas/v1/sign?href=x",
            "reason=http_status_transient;sig=secret",
            "token=abc",
            "reason=Transport",
            "reason=http_status_transient;reason=transport",
            "http_status=99",
            "http_status=600",
            "retry_after_seconds=3601",
            "retry_after=tomorrow",
            "transport=dns",
            "attempts=0",
            "attempts=10",
            "reason=response_json;",
            overlong_reason.as_str(),
        ] {
            assert!(
                summary_with_detail(detail).validate().is_err(),
                "{detail:?}"
            );
        }
    }
}
