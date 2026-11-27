use std::path::PathBuf;
#[cfg(test)]
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(feature = "nisar-subset-production")]
use insar_core::NisarSubsetFrequency;
use insar_core::{
    AssetRole, DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadItemStatus,
    DownloadLayout, DownloadOutcome, ProviderAsset, ProviderNetworkPolicy, ProxyPolicy,
};
use insar_provider::asf::client::AsfHttpClient;
use insar_provider::asf::download::{
    resolve_asf_download, resolve_frozen_asf_asset, ResolvedDownloadSource,
};
use insar_provider::orbit_download::OrbitHttpClient;
use insar_provider::usgs_m2m::UsgsM2mSession;
#[cfg(feature = "nisar-subset-production")]
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::local_reuse::{
    authoritative_reuse_size, pass_local_reuse_gate, record_deferred_reuse, record_local_landing,
    scan_local_reuse, GateRequest,
};
#[cfg(test)]
use crate::nisar_subset_writer::WriterFailpointV2;
use crate::scheduler::timestamp_now;
use crate::{
    finalize_part, inspect_resolved_destination, plan_download_target, plan_sar_catalog_target,
    resolve_earthdata_burst_credential, resolve_earthdata_credential, route_frozen_execution,
    run_shadow_then_legacy, AdmittedTransferBackend, BhoonidhiSession,
    BhoonidhiSessionTokenProvider, BurstAuthEndpoints, CdseNetworkFoundation, CdseOAuthSession,
    DestinationLeaseController, DestinationLeaseError, DestinationLeaseGuard, DownloadFileError,
    DownloadWork, DownloadWorker, EarthdataCredentialSource, ExpectedChecksum, FilePreparation,
    HttpTransfer, LegacyTransferIntent, NisarExecutionPlanFreezeAuthority, NisarSourceSizeReadyV2,
    ResolvedAsset, ResolvedPartContext, ResolverExecutionRuntime, ResolverShadowInput,
    ResolverShadowObserver, ResolverShadowRuntime, ShadowAssetClass, ShadowAuthKind,
    ShadowDestinationPolicy, TaskTransferActivityRegistry, TransferAdmissionController,
    TransferAuth, TransferBackend, TransferBackendSelector, TransferIntentFingerprint,
    TransferOptions, TransferRequest, TransferUpdate, WorkerControl, NISAR_SUBSET_V2_DELIVERY_KIND,
};
#[cfg(test)]
use crate::{NisarSubsetMetadataBootstrapV2, NisarSubsetPlanningCompleteV2};

#[cfg(feature = "nisar-subset-production")]
use crate::hdf5_range_transfer::{
    discover_live_nisar_metadata_chunks_with_budget, prefetch_live_nisar_metadata_with_budget,
    preflight_live_nisar_source_with_budget,
};
#[cfg(feature = "nisar-subset-production")]
use crate::{
    build_live_nisar_subset_request, live_nisar_range_plan_from_sparse_hdf5,
    seed_live_metadata_range, Hdf5RangeTransferError, Hdf5RangeWorkspaceV1,
    NisarLiveNetworkBudgetV1, NisarSubsetMetadataSnapshotV1, OfflineHdf5RangeTransfer,
    OfflineNisarSubsetWriter,
};
#[cfg(feature = "nisar-subset-production")]
use crate::{derive_nisar_live_semantic_plan_sha256, validate_nisar_live_admission};
#[cfg(feature = "nisar-subset-live-acceptance")]
use crate::{NisarIsce3LiveGeometryGateV1, NisarLiveAcceptanceEvidenceSink};
#[cfg(feature = "nisar-subset-production")]
use crate::{RemoteValidator, ResolvedChecksum};

#[cfg(not(feature = "nisar-subset-live-acceptance"))]
struct NisarLiveAcceptanceEvidenceSink;

#[cfg(not(feature = "nisar-subset-live-acceptance"))]
impl NisarLiveAcceptanceEvidenceSink {
    fn ok() -> std::io::Result<()> {
        Ok(())
    }

    fn mark_running(&self) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_failure(&self, _: &str) -> std::io::Result<()> {
        Self::ok()
    }

    fn finish_pass(&self) -> std::io::Result<()> {
        Self::ok()
    }

    fn finish_failure(&self, _: &str) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_scope<A, B, C, D, E, F>(
        &self,
        _: A,
        _: B,
        _: C,
        _: D,
        _: E,
        _: F,
    ) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_aoi_digest<A>(&self, _: A) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_source_identity(
        &self,
        input: NisarLiveSourceIdentityInput<'_>,
    ) -> std::io::Result<()> {
        let _ = (
            input.provider_id,
            input.provider_product_id,
            input.provider_product_key,
            input.collection_id,
            input.asset_key,
            input.stable_asset_key,
            input.source_product_identity_sha256,
            input.source_asset_identity_sha256,
        );
        Self::ok()
    }

    fn record_polarizations<A, B>(&self, _: A, _: B) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_geometry_digests<A, B, C>(&self, _: A, _: B, _: C) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_plan_digests(&self, input: NisarLivePlanDigestsInput<'_>) -> std::io::Result<()> {
        let _ = (
            input.prepared_intent_sha256,
            input.final_plan_sha256,
            input.physical_plan_sha256,
            input.inventory_sha256,
            input.inventory_evidence_sha256,
            input.source_graph_sha256,
            input.source_validator_sha256,
        );
        Self::ok()
    }

    fn record_budget_limits(&self, input: NisarLiveBudgetLimitsInput) -> std::io::Result<()> {
        let _ = (
            input.max_requests,
            input.max_response_bytes,
            input.remaining_requests,
            input.remaining_response_bytes,
            input.planning_requests,
            input.planning_response_bytes,
            input.reserved_science_requests,
            input.reserved_science_response_bytes,
            input.committed_science_requests,
            input.committed_science_response_bytes,
        );
        Self::ok()
    }

    fn begin_planning_range_snapshot(
        &self,
        _stage: &str,
        _source_size: u64,
        _validator: &str,
    ) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_range_observation(
        &self,
        input: NisarLiveRangeObservationInput<'_>,
    ) -> std::io::Result<()> {
        let _ = (
            input.stage,
            input.start,
            input.end_exclusive,
            input.source_size_bytes,
            input.request_count,
            input.http_status,
            input.response_body_bytes,
            input.validator_sha256,
        );
        Self::ok()
    }

    fn record_range_failure_observation(
        &self,
        input: NisarLiveRangeFailureObservationInput<'_>,
    ) -> std::io::Result<()> {
        let _ = (
            input.stage,
            input.start,
            input.end_exclusive,
            input.source_size_bytes,
            input.disposition,
            input.request_count,
            input.http_status,
            input.response_body_bytes,
            input.validator_sha256,
            input.failure_code,
            input.redirect_occurred,
            input.redirect_hop_index,
            input.redirect_source_host_class,
            input.redirect_destination_host_class,
            input.redirect_source_path_class,
            input.redirect_destination_path_class,
            input.redirect_same_origin,
            input.redirect_scheme_transition,
            input.redirect_policy_result,
            input.redirect_rejection_reason,
        );
        Self::ok()
    }

    fn record_preflight<A, B, C, D, E, F>(
        &self,
        _: A,
        _: B,
        _: C,
        _: D,
        _: E,
        _: F,
    ) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_metadata_prefix<A, B>(&self, _: A, _: B) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_metadata_discovery<A, B>(&self, _: A, _: B) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_additional_geolocation<A, B>(&self, _: A, _: B) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_planning_revalidation<A, B>(&self, _: A, _: B) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_science_failures(&self, _: u64, _: u64) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_science_range<A, B>(&self, _: A, _: B) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_metadata_inventory<A, B, C, D>(&self, _: A, _: B, _: C, _: D) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_execution_plan<A>(&self, _: A) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_semantic_plan<A>(&self, _: A) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_prepared_intent<A>(&self, _: A) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_geometry<A, B, C, D>(&self, _: A, _: B, _: C, _: D) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_output<A>(&self, _: A) -> std::io::Result<()> {
        Self::ok()
    }

    fn record_output_v2<A>(&self, _: A) -> std::io::Result<()> {
        Self::ok()
    }
}

#[derive(Debug, Error)]
pub enum ProductionWorkerError {
    #[error("无法初始化 Resolver 执行运行时")]
    ResolverRuntime,
    #[error("无法初始化 ASF 下载元数据客户端")]
    AsfClient,
    #[error("无法初始化 Orbit 下载元数据客户端")]
    OrbitClient,
    #[error("无法初始化 Rust HTTP 下载器")]
    TransferClient,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NisarLiveSourceIdentityInput<'a> {
    pub(crate) provider_id: &'a str,
    pub(crate) provider_product_id: &'a str,
    pub(crate) provider_product_key: &'a str,
    pub(crate) collection_id: &'a str,
    pub(crate) asset_key: &'a str,
    pub(crate) stable_asset_key: &'a str,
    pub(crate) source_product_identity_sha256: &'a str,
    pub(crate) source_asset_identity_sha256: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NisarLivePlanDigestsInput<'a> {
    pub(crate) prepared_intent_sha256: &'a str,
    pub(crate) final_plan_sha256: &'a str,
    pub(crate) physical_plan_sha256: &'a str,
    pub(crate) inventory_sha256: &'a str,
    pub(crate) inventory_evidence_sha256: &'a str,
    pub(crate) source_graph_sha256: &'a str,
    pub(crate) source_validator_sha256: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NisarLiveBudgetLimitsInput {
    pub(crate) max_requests: u64,
    pub(crate) max_response_bytes: u64,
    pub(crate) remaining_requests: u64,
    pub(crate) remaining_response_bytes: u64,
    pub(crate) planning_requests: u64,
    pub(crate) planning_response_bytes: u64,
    pub(crate) reserved_science_requests: u64,
    pub(crate) reserved_science_response_bytes: u64,
    pub(crate) committed_science_requests: u64,
    pub(crate) committed_science_response_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NisarLiveRangeFailureObservationInput<'a> {
    pub(crate) stage: &'a str,
    pub(crate) start: u64,
    pub(crate) end_exclusive: u64,
    pub(crate) source_size_bytes: u64,
    pub(crate) disposition: &'a str,
    pub(crate) request_count: u64,
    pub(crate) http_status: Option<u16>,
    pub(crate) response_body_bytes: Option<u64>,
    pub(crate) validator_sha256: Option<&'a str>,
    pub(crate) failure_code: &'a str,
    pub(crate) redirect_occurred: bool,
    pub(crate) redirect_hop_index: Option<u8>,
    pub(crate) redirect_source_host_class: Option<&'a str>,
    pub(crate) redirect_destination_host_class: Option<&'a str>,
    pub(crate) redirect_source_path_class: Option<&'a str>,
    pub(crate) redirect_destination_path_class: Option<&'a str>,
    pub(crate) redirect_same_origin: Option<bool>,
    pub(crate) redirect_scheme_transition: Option<&'a str>,
    pub(crate) redirect_policy_result: Option<&'a str>,
    pub(crate) redirect_rejection_reason: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NisarLiveRangeObservationInput<'a> {
    pub(crate) stage: &'a str,
    pub(crate) start: u64,
    pub(crate) end_exclusive: u64,
    pub(crate) source_size_bytes: u64,
    pub(crate) request_count: u64,
    pub(crate) http_status: u16,
    pub(crate) response_body_bytes: u64,
    pub(crate) validator_sha256: &'a str,
}

#[cfg(test)]
type NisarSubsetV2AfterPlanningHook = Arc<dyn Fn(&WorkerControl) + Send + Sync + 'static>;
#[cfg(test)]
type NisarSubsetV2AfterScienceCommitHook =
    Arc<dyn Fn(usize, &WorkerControl) + Send + Sync + 'static>;

#[cfg(feature = "nisar-subset-live-acceptance")]
#[doc(hidden)]
pub type NisarSubsetV2BodyInterruptionHook =
    Arc<dyn Fn(u64, &WorkerControl) -> bool + Send + Sync + 'static>;

#[cfg(test)]
struct NisarSubsetV2PlanningFixtureRuntime {
    bootstrap: Arc<dyn NisarSubsetMetadataBootstrapV2>,
    planned: mpsc::Sender<Result<NisarSubsetPlanningCompleteV2, String>>,
    finish: Mutex<mpsc::Receiver<()>>,
}

#[cfg(feature = "nisar-subset-production")]
const NISAR_SUBSET_PROGRESS_EMIT_BYTES: u64 = 1024 * 1024;

#[cfg(feature = "nisar-subset-production")]
struct NisarSubsetTransferProgressReporter<'a> {
    task_id: String,
    item_id: String,
    started: Instant,
    observed_bytes: u64,
    last_published_bytes: u64,
    transfer_complete_published: bool,
    waiting_for_output_published: bool,
    sink: &'a mut dyn FnMut(DownloadItemProgress),
}

#[cfg(feature = "nisar-subset-production")]
impl<'a> NisarSubsetTransferProgressReporter<'a> {
    fn new(
        task_id: impl Into<String>,
        item_id: impl Into<String>,
        sink: &'a mut dyn FnMut(DownloadItemProgress),
    ) -> Self {
        Self {
            task_id: task_id.into(),
            item_id: item_id.into(),
            started: Instant::now(),
            observed_bytes: 0,
            last_published_bytes: 0,
            transfer_complete_published: false,
            waiting_for_output_published: false,
            sink,
        }
    }

    fn observe(&mut self, delta: u64) {
        self.observed_bytes = self.observed_bytes.saturating_add(delta);
        if self
            .observed_bytes
            .saturating_sub(self.last_published_bytes)
            >= NISAR_SUBSET_PROGRESS_EMIT_BYTES
        {
            self.publish();
        }
    }

    fn observe_science(
        &mut self,
        event: crate::nisar_science_transfer_v2::NisarScienceTransferProgressV2,
    ) {
        match event {
            crate::nisar_science_transfer_v2::NisarScienceTransferProgressV2::Bytes(delta) => {
                self.observe(delta)
            }
            crate::nisar_science_transfer_v2::NisarScienceTransferProgressV2::TransferComplete => {
                self.mark_transfer_complete()
            }
        }
    }

    fn flush(&mut self) {
        if self.observed_bytes > self.last_published_bytes {
            self.publish();
        }
    }

    fn observed_bytes(&self) -> u64 {
        self.observed_bytes
    }

    /// Science transfer is complete: no transfer is active any more. Publish
    /// one zero-rate row of this run's real bytes, once. Review 2026-10-17
    /// round 2, P2-2: the row carries no total, so no persisted total changes
    /// (a Completed result's total stays its bytes_written). Ordinary rows of
    /// this reporter always carry a positive average rate, so the Store groups
    /// this item as idle and the task card reads that as the output phase; the
    /// scheduler forwards the stop of a transfer at once.
    fn mark_transfer_complete(&mut self) {
        if self.transfer_complete_published {
            return;
        }
        self.transfer_complete_published = true;
        self.publish_with(Some(0.0));
    }

    /// Downloader enhancement D1: the writer waits for another writer of its
    /// artifact, for an earlier build writing in its directory or for an
    /// output slot. One zero-rate row whose status is queued (this run's
    /// bytes, no total) makes the Store count the item as waiting and the
    /// task card show 等待写出.
    fn mark_waiting_for_output(&mut self) {
        if self.waiting_for_output_published {
            return;
        }
        self.waiting_for_output_published = true;
        self.publish_row(DownloadItemStatus::Queued, Some(0.0));
    }

    /// The writer was admitted after waiting: the output phase row again.
    fn mark_output_admitted(&mut self) {
        if std::mem::take(&mut self.waiting_for_output_published) {
            self.publish_with(Some(0.0));
        }
    }

    fn publish(&mut self) {
        self.publish_with(None);
    }

    fn publish_with(&mut self, bytes_per_second: Option<f64>) {
        self.publish_row(DownloadItemStatus::Active, bytes_per_second);
    }

    fn publish_row(&mut self, status: DownloadItemStatus, bytes_per_second: Option<f64>) {
        let elapsed = self.started.elapsed().as_secs_f64().max(0.001);
        (self.sink)(DownloadItemProgress {
            task_id: self.task_id.clone(),
            item_id: self.item_id.clone(),
            status,
            bytes_completed: self.observed_bytes,
            total_bytes: None,
            bytes_per_second: bytes_per_second.unwrap_or(self.observed_bytes as f64 / elapsed),
            attempt: 1,
            updated_at: timestamp_now(),
        });
        self.last_published_bytes = self.observed_bytes;
    }
}

#[cfg(test)]
type NisarSubsetV2ScienceRetryBackoff = Arc<dyn Fn(u32, Option<u16>) -> Duration + Send + Sync>;

pub struct ProductionDownloadWorker {
    asf: AsfHttpClient,
    orbit: OrbitHttpClient,
    /// Existing ASF/Orbit system-route transfer. It remains unchanged when a
    /// CDSE session is attached with an explicit diagnostic route.
    transfer: Arc<HttpTransfer>,
    cdse_transfer: Option<Arc<HttpTransfer>>,
    bhoonidhi_transfer: Option<Arc<HttpTransfer>>,
    usgs_transfer: Option<Arc<HttpTransfer>>,
    /// Reviewed Planetary Computer blob payloads use one direct data-plane
    /// client. Control-plane STAC/SAS requests remain owned by their provider
    /// clients and keep the configured/system network route.
    mpc_transfer: Arc<HttpTransfer>,
    transfer_options: TransferOptions,
    burst_session_ready: Mutex<bool>,
    shadow: Option<Arc<ResolverShadowRuntime>>,
    resolver_runtime: ResolverExecutionRuntime,
    transfer_backends: TransferBackendSelector,
    admission: Arc<TransferAdmissionController>,
    bhoonidhi_admission: Arc<TransferAdmissionController>,
    destination_lease: Arc<DestinationLeaseController>,
    /// Process-wide output slots of the NISAR V2 writer, separate from the
    /// network admission above (downloader enhancement D1).
    #[cfg(feature = "nisar-subset-production")]
    nisar_writer_output_slots: Arc<crate::nisar_writer_admission_v2::NisarWriterOutputSlotsV2>,
    transfer_activity: Arc<TaskTransferActivityRegistry>,
    #[cfg(test)]
    nisar_subset_v2_planning_fixture: Option<NisarSubsetV2PlanningFixtureRuntime>,
    #[cfg(test)]
    nisar_subset_v2_after_planning_hook: Option<NisarSubsetV2AfterPlanningHook>,
    #[cfg(test)]
    nisar_subset_v2_after_science_commit_hook: Option<NisarSubsetV2AfterScienceCommitHook>,
    #[cfg(test)]
    nisar_subset_v2_science_retry_backoff: Option<NisarSubsetV2ScienceRetryBackoff>,
    #[cfg(test)]
    nisar_subset_v2_writer_failpoint: Option<WriterFailpointV2>,
    /// A shorter long-wait threshold of the NISAR V2 writer (tests only).
    #[cfg(test)]
    nisar_writer_long_wait_after: Option<Duration>,
    #[cfg(test)]
    bypass_earthdata_auth_for_test: bool,
    #[cfg(feature = "nisar-subset-production")]
    live_evidence: Option<Arc<NisarLiveAcceptanceEvidenceSink>>,
    #[cfg(feature = "nisar-subset-live-acceptance")]
    nisar_isce3_geometry_gate: Option<NisarIsce3LiveGeometryGateV1>,
    #[cfg(feature = "nisar-subset-live-acceptance")]
    nisar_subset_v2_body_interruption_hook: Option<NisarSubsetV2BodyInterruptionHook>,
}

impl ProductionDownloadWorker {
    pub fn new(options: TransferOptions) -> Result<Self, ProductionWorkerError> {
        let runtime = ResolverExecutionRuntime::production()
            .map_err(|_| ProductionWorkerError::ResolverRuntime)?;
        Self::new_with_runtime(options, runtime, None)
    }

    /// Constructs a worker around one caller-owned CDSE session. The chosen
    /// route is frozen into the CDSE transfer at construction time, so an
    /// in-flight transfer cannot change route when future settings change.
    pub fn new_with_cdse_session(
        options: TransferOptions,
        cdse_session: Arc<CdseOAuthSession>,
        network_policy: ProviderNetworkPolicy,
    ) -> Result<Self, ProductionWorkerError> {
        let runtime =
            ResolverExecutionRuntime::production_with_cdse_session(cdse_session, network_policy)
                .map_err(|_| ProductionWorkerError::ResolverRuntime)?;
        let cdse_transfer = Arc::new(
            CdseNetworkFoundation::new(network_policy)
                .archive_runtime()
                .transfer(options)
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        );
        Self::new_with_runtime(options, runtime, Some(cdse_transfer))
    }

    /// Construct one worker around the caller-owned CDSE and USGS sessions.
    /// The same USGS session is used for catalogue, option discovery, and
    /// just-in-time bundle resolution by the application runtime.
    pub fn new_with_cdse_and_usgs_sessions(
        options: TransferOptions,
        cdse_session: Arc<CdseOAuthSession>,
        network_policy: ProviderNetworkPolicy,
        usgs_session: Arc<UsgsM2mSession>,
    ) -> Result<Self, ProductionWorkerError> {
        let usgs_network_policy = usgs_session.network_policy();
        let runtime = ResolverExecutionRuntime::production_with_cdse_and_usgs_sessions(
            cdse_session,
            network_policy,
            usgs_session,
        )
        .map_err(|_| ProductionWorkerError::ResolverRuntime)?;
        let cdse_transfer = Arc::new(
            CdseNetworkFoundation::new(network_policy)
                .archive_runtime()
                .transfer(options)
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        );
        let mut worker = Self::new_with_runtime(options, runtime, Some(cdse_transfer))?;
        worker.usgs_transfer = Some(Arc::new(
            HttpTransfer::new_with_proxy_policy(options, usgs_network_policy.route())
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        ));
        Ok(worker)
    }

    /// Construct the desktop production worker around caller-owned CDSE and
    /// Bhoonidhi sessions. Both catalogue and resolver paths retain the same
    /// process-owned Bhoonidhi session and admission controller.
    pub fn new_with_cdse_and_bhoonidhi_sessions(
        options: TransferOptions,
        cdse_session: Arc<CdseOAuthSession>,
        network_policy: ProviderNetworkPolicy,
        bhoonidhi_session: Arc<BhoonidhiSession>,
        probe_timeout: Duration,
        bhoonidhi_admission: Arc<TransferAdmissionController>,
    ) -> Result<Self, ProductionWorkerError> {
        let bhoonidhi_policy = bhoonidhi_session.network_policy();
        let runtime = ResolverExecutionRuntime::production_with_cdse_and_bhoonidhi_sessions(
            Arc::clone(&cdse_session),
            network_policy,
            bhoonidhi_session,
            probe_timeout,
        )
        .map_err(|_| ProductionWorkerError::ResolverRuntime)?;
        let cdse_transfer = Arc::new(
            CdseNetworkFoundation::new(network_policy)
                .archive_runtime()
                .transfer(options)
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        );
        let mut worker = Self::new_with_runtime(options, runtime, Some(cdse_transfer))?;
        worker.set_ordinary_network_policy(network_policy)?;
        worker.bhoonidhi_transfer = Some(Arc::new(
            HttpTransfer::new_with_proxy_policy(options, bhoonidhi_policy.route())
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        ));
        worker.bhoonidhi_admission = bhoonidhi_admission;
        Ok(worker)
    }

    /// Construct the worker with the one provider instance shared with the
    /// catalogue router. This preserves one lazy-login gate across both paths.
    pub fn new_with_cdse_and_bhoonidhi_provider(
        options: TransferOptions,
        cdse_session: Arc<CdseOAuthSession>,
        network_policy: ProviderNetworkPolicy,
        bhoonidhi_session: Arc<BhoonidhiSession>,
        bhoonidhi_provider: Arc<BhoonidhiSessionTokenProvider>,
        probe_timeout: Duration,
        bhoonidhi_admission: Arc<TransferAdmissionController>,
    ) -> Result<Self, ProductionWorkerError> {
        let bhoonidhi_policy = bhoonidhi_session.network_policy();
        let runtime = ResolverExecutionRuntime::production_with_cdse_and_bhoonidhi_provider(
            Arc::clone(&cdse_session),
            network_policy,
            bhoonidhi_session,
            bhoonidhi_provider,
            probe_timeout,
        )
        .map_err(|_| ProductionWorkerError::ResolverRuntime)?;
        let cdse_transfer = Arc::new(
            CdseNetworkFoundation::new(network_policy)
                .archive_runtime()
                .transfer(options)
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        );
        let mut worker = Self::new_with_runtime(options, runtime, Some(cdse_transfer))?;
        worker.set_ordinary_network_policy(network_policy)?;
        worker.bhoonidhi_transfer = Some(Arc::new(
            HttpTransfer::new_with_proxy_policy(options, bhoonidhi_policy.route())
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        ));
        worker.bhoonidhi_admission = bhoonidhi_admission;
        Ok(worker)
    }

    /// Construct the exact Bhoonidhi S-RSLC production worker around the
    /// process-owned session shared with catalogue search.
    pub fn new_with_bhoonidhi_session(
        options: TransferOptions,
        bhoonidhi_session: Arc<BhoonidhiSession>,
        probe_timeout: Duration,
        bhoonidhi_admission: Arc<TransferAdmissionController>,
    ) -> Result<Self, ProductionWorkerError> {
        let bhoonidhi_policy = bhoonidhi_session.network_policy();
        let runtime = ResolverExecutionRuntime::production_with_bhoonidhi_session(
            bhoonidhi_session,
            probe_timeout,
        )
        .map_err(|_| ProductionWorkerError::ResolverRuntime)?;
        let mut worker = Self::new_with_runtime(options, runtime, None)?;
        worker.bhoonidhi_transfer = Some(Arc::new(
            HttpTransfer::new_with_proxy_policy(options, bhoonidhi_policy.route())
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        ));
        worker.bhoonidhi_admission = bhoonidhi_admission;
        Ok(worker)
    }

    /// Construct the normal worker with one caller-owned USGS session.
    /// Generic ASF/Orbit transfers retain the System route; USGS data-plane
    /// requests use the route frozen into the injected USGS session.
    pub fn new_with_usgs_session(
        options: TransferOptions,
        usgs_session: Arc<UsgsM2mSession>,
    ) -> Result<Self, ProductionWorkerError> {
        let usgs_network_policy = usgs_session.network_policy();
        let runtime = ResolverExecutionRuntime::production_with_usgs_session(usgs_session)
            .map_err(|_| ProductionWorkerError::ResolverRuntime)?;
        let mut worker = Self::new_with_runtime(options, runtime, None)?;
        worker.usgs_transfer = Some(Arc::new(
            HttpTransfer::new_with_proxy_policy(options, usgs_network_policy.route())
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        ));
        Ok(worker)
    }

    fn new_with_runtime(
        options: TransferOptions,
        resolver_runtime: ResolverExecutionRuntime,
        cdse_transfer: Option<Arc<HttpTransfer>>,
    ) -> Result<Self, ProductionWorkerError> {
        let transfer = Arc::new(
            HttpTransfer::new(options).map_err(|_| ProductionWorkerError::TransferClient)?,
        );
        let mpc_transfer = Arc::new(
            HttpTransfer::new_with_proxy_policy(options, ProxyPolicy::Direct)
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        );
        let native_backend: Arc<dyn TransferBackend> = transfer.clone();
        Ok(Self {
            asf: AsfHttpClient::production().map_err(|_| ProductionWorkerError::AsfClient)?,
            orbit: OrbitHttpClient::production().map_err(|_| ProductionWorkerError::OrbitClient)?,
            transfer,
            cdse_transfer,
            bhoonidhi_transfer: None,
            usgs_transfer: None,
            mpc_transfer,
            transfer_options: options,
            burst_session_ready: Mutex::new(false),
            shadow: None,
            resolver_runtime,
            transfer_backends: TransferBackendSelector::native(native_backend),
            admission: TransferAdmissionController::compatibility(),
            bhoonidhi_admission: TransferAdmissionController::compatibility(),
            destination_lease: DestinationLeaseController::new(),
            // Downloader enhancement D1b: EOSTAGE_NISAR_WRITER_SLOTS (1 to 8,
            // default 1) configures how many NISAR writers produce output at once.
            #[cfg(feature = "nisar-subset-production")]
            nisar_writer_output_slots:
                crate::nisar_writer_admission_v2::nisar_writer_output_slots_from_environment(),
            transfer_activity: Arc::new(TaskTransferActivityRegistry::default()),
            #[cfg(test)]
            nisar_subset_v2_planning_fixture: None,
            #[cfg(test)]
            nisar_subset_v2_after_planning_hook: None,
            #[cfg(test)]
            nisar_subset_v2_after_science_commit_hook: None,
            #[cfg(test)]
            nisar_subset_v2_science_retry_backoff: None,
            #[cfg(test)]
            nisar_subset_v2_writer_failpoint: None,
            #[cfg(test)]
            nisar_writer_long_wait_after: None,
            #[cfg(test)]
            bypass_earthdata_auth_for_test: false,
            #[cfg(feature = "nisar-subset-production")]
            live_evidence: None,
            #[cfg(feature = "nisar-subset-live-acceptance")]
            nisar_isce3_geometry_gate: None,
            #[cfg(feature = "nisar-subset-live-acceptance")]
            nisar_subset_v2_body_interruption_hook: None,
        })
    }

    /// Ordinary ASF/Orbit control requests and ASF/Orbit/MPC file transfer all
    /// follow the saved ordinary route; Bhoonidhi, CDSE and USGS own theirs.
    fn set_ordinary_network_policy(
        &mut self,
        network_policy: ProviderNetworkPolicy,
    ) -> Result<(), ProductionWorkerError> {
        let policy = network_policy.route();
        let transfer = Arc::new(
            HttpTransfer::new_with_proxy_policy(self.transfer_options, policy)
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        );
        let backend: Arc<dyn TransferBackend> = transfer.clone();
        self.transfer = transfer;
        self.transfer_backends = TransferBackendSelector::native(backend);
        self.mpc_transfer = Arc::new(
            HttpTransfer::new_with_proxy_policy(self.transfer_options, policy)
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        );
        self.asf = AsfHttpClient::production_with_network_policy(network_policy)
            .map_err(|_| ProductionWorkerError::AsfClient)?;
        self.orbit = OrbitHttpClient::production_with_network_policy(network_policy)
            .map_err(|_| ProductionWorkerError::OrbitClient)?;
        Ok(())
    }

    #[doc(hidden)]
    pub fn with_shadow_runtime(mut self, runtime: Arc<ResolverShadowRuntime>) -> Self {
        self.shadow = Some(runtime);
        self
    }

    #[doc(hidden)]
    pub fn shadow_runtime_enabled(&self) -> bool {
        self.shadow.is_some()
    }

    /// Startup composition with the application's official archive session.
    pub fn with_usgs_session(
        mut self,
        session: Arc<UsgsM2mSession>,
    ) -> Result<Self, ProductionWorkerError> {
        let route = session.network_policy().route();
        let usgs_transfer = Arc::new(
            HttpTransfer::new_with_proxy_policy(self.transfer_options, route)
                .map_err(|_| ProductionWorkerError::TransferClient)?,
        );
        self.resolver_runtime = self
            .resolver_runtime
            .with_usgs_session(session)
            .map_err(|_| ProductionWorkerError::ResolverRuntime)?;
        self.usgs_transfer = Some(usgs_transfer);
        Ok(self)
    }

    pub fn resolver_runtime_ready(&self) -> bool {
        self.resolver_runtime.is_ready()
    }

    #[doc(hidden)]
    pub fn shares_usgs_session(&self, session: &Arc<UsgsM2mSession>) -> bool {
        self.resolver_runtime.shares_usgs_session(session)
    }

    #[doc(hidden)]
    pub fn shares_bhoonidhi_session(&self, session: &Arc<BhoonidhiSession>) -> bool {
        self.resolver_runtime.shares_bhoonidhi_session(session)
    }

    #[doc(hidden)]
    pub fn shares_bhoonidhi_transfer_admission(
        &self,
        admission: &Arc<TransferAdmissionController>,
    ) -> bool {
        Arc::ptr_eq(&self.bhoonidhi_admission, admission)
    }

    #[doc(hidden)]
    pub fn bhoonidhi_transfer_admission(&self) -> Option<Arc<TransferAdmissionController>> {
        Some(Arc::clone(&self.bhoonidhi_admission))
    }

    #[doc(hidden)]
    pub fn with_resolver_runtime_and_transfer(
        mut self,
        runtime: ResolverExecutionRuntime,
        transfer: Arc<dyn TransferBackend>,
    ) -> Self {
        self.resolver_runtime = runtime;
        self.cdse_transfer = None;
        self.transfer_backends =
            TransferBackendSelector::with_preferred_for_test(self.transfer.clone(), transfer);
        self
    }

    #[doc(hidden)]
    pub fn with_resolver_runtime_and_backends_for_test(
        mut self,
        runtime: ResolverExecutionRuntime,
        native: Arc<dyn TransferBackend>,
        preferred: Arc<dyn TransferBackend>,
    ) -> Self {
        self.resolver_runtime = runtime;
        self.cdse_transfer = None;
        self.transfer_backends =
            TransferBackendSelector::with_preferred_for_test(native, preferred);
        self
    }

    #[doc(hidden)]
    pub fn with_transfer_admission_controller_for_test(
        mut self,
        admission: Arc<TransferAdmissionController>,
    ) -> Self {
        self.admission = admission;
        self
    }

    #[doc(hidden)]
    pub fn with_bhoonidhi_transfer_admission_controller_for_test(
        mut self,
        admission: Arc<TransferAdmissionController>,
    ) -> Self {
        self.bhoonidhi_admission = admission;
        self
    }

    #[doc(hidden)]
    pub fn with_destination_lease_controller_for_test(
        mut self,
        destination_lease: Arc<DestinationLeaseController>,
    ) -> Self {
        self.destination_lease = destination_lease;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_nisar_subset_v2_http_for_test(
        mut self,
        transfer: Arc<HttpTransfer>,
    ) -> Self {
        self.transfer = Arc::clone(&transfer);
        self.bhoonidhi_transfer = Some(transfer);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_nisar_subset_v2_after_planning_hook_for_test(
        mut self,
        hook: NisarSubsetV2AfterPlanningHook,
    ) -> Self {
        self.nisar_subset_v2_after_planning_hook = Some(hook);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_nisar_subset_v2_science_retry_backoff_for_test(
        mut self,
        backoff: NisarSubsetV2ScienceRetryBackoff,
    ) -> Self {
        self.nisar_subset_v2_science_retry_backoff = Some(backoff);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_nisar_subset_v2_after_science_commit_hook_for_test(
        mut self,
        hook: NisarSubsetV2AfterScienceCommitHook,
    ) -> Self {
        self.nisar_subset_v2_after_science_commit_hook = Some(hook);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_nisar_subset_v2_writer_failpoint_for_test(
        mut self,
        failpoint: WriterFailpointV2,
    ) -> Self {
        self.nisar_subset_v2_writer_failpoint = Some(failpoint);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_nisar_writer_long_wait_after_for_test(
        mut self,
        threshold: Duration,
    ) -> Self {
        self.nisar_writer_long_wait_after = Some(threshold);
        self
    }

    /// The worker's NISAR V2 output slots, so a test can hold one as another
    /// writer would.
    #[cfg(test)]
    pub(crate) fn nisar_writer_output_slots_for_test(
        &self,
    ) -> Arc<crate::nisar_writer_admission_v2::NisarWriterOutputSlotsV2> {
        Arc::clone(&self.nisar_writer_output_slots)
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn with_nisar_live_evidence_sink_for_acceptance(
        mut self,
        sink: Arc<NisarLiveAcceptanceEvidenceSink>,
    ) -> Self {
        self.live_evidence = Some(sink);
        self
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn with_nisar_isce3_geometry_gate_for_acceptance(
        mut self,
        gate: NisarIsce3LiveGeometryGateV1,
    ) -> Self {
        self.nisar_isce3_geometry_gate = Some(gate);
        self
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn with_nisar_subset_v2_body_interruption_hook_for_acceptance(
        mut self,
        hook: NisarSubsetV2BodyInterruptionHook,
    ) -> Self {
        self.nisar_subset_v2_body_interruption_hook = Some(hook);
        self
    }

    /// Phase 2C.1 internal fixture seam. The actual ProductionDownloadWorker is
    /// scheduled and executes the same planner/Store authority handshake, while
    /// a bounded test gate supplies the otherwise-unrepresentable post-planning
    /// terminal transition.
    #[cfg(test)]
    pub(crate) fn with_nisar_subset_v2_planning_fixture(
        mut self,
        bootstrap: Arc<dyn NisarSubsetMetadataBootstrapV2>,
        planned: mpsc::Sender<Result<NisarSubsetPlanningCompleteV2, String>>,
        finish: mpsc::Receiver<()>,
    ) -> Self {
        self.nisar_subset_v2_planning_fixture = Some(NisarSubsetV2PlanningFixtureRuntime {
            bootstrap,
            planned,
            finish: Mutex::new(finish),
        });
        self
    }

    /// Selects the HTTP transfer client owning the route for the NISAR V2 subset
    /// provider (`bhoonidhi_transfer` for Bhoonidhi, `transfer` for Earthdata).
    #[cfg_attr(not(feature = "nisar-subset-production"), allow(dead_code))]
    fn nisar_subset_transfer(&self, provider_id: &str) -> Arc<HttpTransfer> {
        if provider_id == insar_provider::catalog::BHOONIDHI_PROVIDER_ID {
            if let Some(transfer) = self.bhoonidhi_transfer.as_ref() {
                return Arc::clone(transfer);
            }
        }
        Arc::clone(&self.transfer)
    }

    #[cfg(feature = "nisar-subset-production")]
    fn execute_nisar_subset_v2(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        authority: &NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let evidence = self.live_evidence.as_deref();
        if let Some(evidence) = evidence {
            if evidence.mark_running().is_err() {
                let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                let _ = evidence.record_failure(code);
                return failure_result(
                    &work,
                    work.task.output_root.clone(),
                    code,
                    reason_zh,
                    retryable,
                );
            }
        }
        let work_for_evidence_failure = work.clone();
        let result = self.execute_nisar_subset_v2_inner(work, control, authority, progress);
        if let Some(evidence) = evidence {
            if result.outcome == DownloadOutcome::Completed && result.failure.is_none() {
                if evidence.finish_pass().is_err() {
                    let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                    let _ = evidence.record_failure(code);
                    return failure_result(
                        &work_for_evidence_failure,
                        result.destination,
                        code,
                        reason_zh,
                        retryable,
                    );
                }
            } else if let Some(failure) = result.failure.as_ref() {
                if evidence.finish_failure(&failure.code).is_err() {
                    let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                    let _ = evidence.record_failure(code);
                    return failure_result(
                        &work_for_evidence_failure,
                        result.destination,
                        code,
                        reason_zh,
                        retryable,
                    );
                }
            }
        }
        result
    }

    #[cfg(feature = "nisar-subset-production")]
    fn execute_nisar_subset_v2_inner(
        &self,
        mut work: DownloadWork,
        control: WorkerControl,
        authority: &NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let destination = work.task.output_root.clone();
        let archived_recovery = match authority.load_archived_recovery_v2(
            &work.task.task_id,
            &work.item.item_id,
            &control,
        ) {
            Ok(proof) => proof,
            Err(crate::scheduler::NisarExecutionAnchorAuthorityError::ExecutionStopped) => {
                return nisar_v2_stopped_result(&work, destination, &control)
            }
            Err(crate::scheduler::NisarExecutionAnchorAuthorityError::CoordinatorUnavailable) => {
                return failure_result(
                    &work,
                    destination,
                    "NISAR_SUBSET_V2_COORDINATOR_UNAVAILABLE",
                    "NISAR V2 Store execution-anchor authority is unavailable.",
                    false,
                )
            }
            Err(crate::scheduler::NisarExecutionAnchorAuthorityError::Store(_)) => {
                return failure_result(
                    &work,
                    destination,
                    "NISAR_SUBSET_V2_ARCHIVED_RECOVERY_INVALID",
                    "归档恢复来源校验失败；原 checkpoint 保持不变。",
                    false,
                )
            }
        };
        let stored_science_anchor = match authority.load_execution_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
            &control,
        ) {
            Ok(anchor) => anchor,
            Err(error) => {
                let code = match error {
                crate::scheduler::NisarExecutionAnchorAuthorityError::CoordinatorUnavailable => {
                    "NISAR_SUBSET_V2_COORDINATOR_UNAVAILABLE"
                }
                crate::scheduler::NisarExecutionAnchorAuthorityError::Store(_) => {
                    "NISAR_SUBSET_V2_ANCHOR_AUTHORITY_FAILED"
                }
                crate::scheduler::NisarExecutionAnchorAuthorityError::ExecutionStopped => {
                    return nisar_v2_stopped_result(&work, destination, &control);
                }
            };
                return failure_result(
                    &work,
                    destination,
                    code,
                    "NISAR V2 Store execution-anchor authority is unavailable.",
                    false,
                );
            }
        };
        let prepared =
            match insar_store::parse_nisar_subset_prepared_intent_v2_envelope(&work.item.metadata)
                .and_then(|envelope| {
                    envelope
                        .ok_or(insar_store::StoreError::NisarSubsetPreparedIntentMissing)?
                        .intent()
                }) {
                Ok(prepared) => prepared,
                Err(_) => {
                    return failure_result(
                        &work,
                        destination,
                        "NISAR_SUBSET_V2_PREPARED_INTENT_INVALID",
                        "NISAR V2 task is missing its validated Store-frozen PreparedIntent.",
                        false,
                    );
                }
            };
        if let Some(evidence) = self.live_evidence.as_deref() {
            if record_nisar_v2_startup_evidence(evidence, &work, &prepared).is_err() {
                let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                let _ = evidence.record_failure(code);
                return failure_result(&work, destination, code, reason_zh, retryable);
            }
        }
        if control.is_cancelled() || control.is_paused() {
            return nisar_v2_stopped_result(&work, destination, &control);
        }
        // Until the synchronous migration decision returns, a failure must not
        // replace the prior result row without immutably retaining it first.
        // Review 2026-10-17 round 3, P2-1: opening the recovery workspace,
        // writing its rebinding evidence and recording the Store rebinding
        // event precede that decision, so the guard already covers them.
        control.begin_nisar_budget_migration();
        // Design 2026-10-17 §6.1: once a Store science anchor exists, the
        // recovery workspace must already exist; open it, never recreate it
        // (a removed old-path junction must not yield a fresh empty root).
        let workspace_root = nisar_subset_v2_recovery_workspace_root(&work);
        let anchored = stored_science_anchor.is_some();
        let opened = if anchored {
            Hdf5RangeWorkspaceV1::open_existing(workspace_root)
        } else {
            Hdf5RangeWorkspaceV1::new(workspace_root)
        };
        let workspace = match opened {
            Ok(workspace) => workspace,
            Err(_) => {
                let failure =
                    crate::nisar_workspace_rebinding_v2::recovery_workspace_failure_v2(anchored);
                return failure_result(
                    &work,
                    destination,
                    failure.code,
                    &failure.reason_zh,
                    failure.retryable,
                );
            }
        };
        // Identity-verified rebinding (design 2026-10-17 §4): an accepted
        // rebinding leaves non-authoritative local evidence once and one
        // Store task event for audit after the workspace is removed.
        let rebinding =
            match crate::nisar_workspace_rebinding_v2::record_nisar_workspace_rebinding_v2(
                &workspace,
                &work,
                stored_science_anchor.as_ref().map(|anchor| {
                    crate::nisar_workspace_rebinding_v2::NisarRebindingAnchorFactsV2 {
                        record_sha256: &anchor.record_sha256,
                        generation: serde_json::from_str::<serde_json::Value>(
                            &anchor.canonical_json,
                        )
                        .ok()
                        .and_then(|record| record["generation"].as_u64()),
                    }
                }),
            ) {
                Ok(rebinding) => rebinding,
                Err(error) => {
                    let failure =
                        crate::nisar_workspace_rebinding_v2::rebinding_evidence_failure_v2(error);
                    return failure_result(
                        &work,
                        destination,
                        failure.code,
                        &failure.reason_zh,
                        failure.retryable,
                    );
                }
            };
        if let Some(observation) = rebinding {
            if let Err(error) = authority.record_workspace_rebinding_v2(
                insar_store::StoreNisarWorkspaceRebindingEventV2Request {
                    task_id: observation.task_id,
                    item_id: observation.item_id,
                    recorded_path: observation.recorded_path,
                    current_path: observation.current_path,
                    identity: observation.identity,
                    volume_file_system: observation.volume_file_system,
                    planning_checkpoint_sha256: observation.planning_checkpoint_sha256,
                    science_checkpoint_sha256: observation.science_checkpoint_sha256,
                    anchor_record_sha256: observation.anchor_record_sha256,
                    anchor_generation: observation.anchor_generation,
                    occurred_at: observation.observed_at,
                },
                &control,
            ) {
                let error = crate::scheduler::science_anchor_authority_error(error, &control);
                return match crate::nisar_workspace_rebinding_v2::rebinding_event_failure_v2(error)
                {
                    Some(failure) => failure_result(
                        &work,
                        destination,
                        failure.code,
                        &failure.reason_zh,
                        failure.retryable,
                    ),
                    None => nisar_v2_stopped_result(&work, destination, &control),
                };
            }
        }
        let migration_anchor = authority.science_checkpoint_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            &control,
        );
        if let Err(failure) = crate::nisar_science_transfer_v2::prepare_nisar_budget_upgrade_v2(
            &work,
            &workspace,
            migration_anchor.as_ref(),
            stored_science_anchor.as_ref(),
            &control,
        ) {
            return nisar_v2_budget_migration_failure(&work, destination, failure, &control);
        }
        control.complete_nisar_budget_migration();
        let asset = match self
            .resolver_runtime
            .resolve_for_nisar_remote_subset(&work.item, &|| {
                control.is_cancelled() || control.is_paused()
            }) {
            Ok(asset) => asset,
            Err(error) if nisar_v2_resolver_error_is_lifecycle_stop(&error) => {
                return nisar_v2_stopped_result(&work, destination, &control);
            }
            Err(error) => {
                let (code, reason_zh, retryable) = nisar_v2_resolver_failure(&error);
                return failure_result(&work, destination, code, &reason_zh, retryable);
            }
        };
        if work.item.provider_id == insar_provider::catalog::BHOONIDHI_PROVIDER_ID {
            let Some(resolved_size) = asset.expected_size().filter(|size| *size > 0) else {
                return failure_result(
                    &work,
                    destination,
                    "NISAR_SUBSET_V2_SOURCE_SIZE_INVALID",
                    "Bhoonidhi resolver did not return a validated positive source size.",
                    false,
                );
            };
            let frozen_size =
                match authority.freeze_source_size(
                    NisarSourceSizeReadyV2 {
                        task_id: work.task.task_id.clone(),
                        item_id: work.item.item_id.clone(),
                        provider_id: prepared.source_product_identity.provider_id.clone(),
                        collection_id: prepared.source_product_identity.collection_id.clone(),
                        provider_product_id: prepared
                            .source_product_identity
                            .provider_product_id
                            .clone(),
                        provider_product_key: prepared
                            .source_product_identity
                            .provider_product_key
                            .clone(),
                        asset_key: prepared.source_asset_identity.asset_key.clone(),
                        stable_asset_key: prepared.source_asset_identity.stable_asset_key.clone(),
                        prepared_intent_sha256: prepared.canonical_intent_sha256.clone(),
                        expected_size_bytes: resolved_size,
                    },
                    &control,
                ) {
                    Ok(size) => size,
                    Err(crate::scheduler::NisarSourceSizeFreezeError::ExecutionStopped) => {
                        return nisar_v2_stopped_result(&work, destination, &control)
                    }
                    Err(
                        crate::scheduler::NisarSourceSizeFreezeError::Store(_)
                        | crate::scheduler::NisarSourceSizeFreezeError::CoordinatorUnavailable,
                    ) => return failure_result(
                        &work,
                        destination,
                        "NISAR_SUBSET_V2_SOURCE_SIZE_FREEZE_FAILED",
                        "NISAR V2 resolver-verified source size could not be frozen in the Store.",
                        false,
                    ),
                };
            let Some(access) = work.item.asset_access.as_mut() else {
                return failure_result(
                    &work,
                    destination,
                    "NISAR_SUBSET_V2_SOURCE_SIZE_FREEZE_FAILED",
                    "NISAR V2 execution snapshot lost its frozen asset access authority.",
                    false,
                );
            };
            access.expected_size_bytes = Some(frozen_size);
        }
        let recovery_prepared = if let Some(proof) = archived_recovery {
            let Some(item) = proof
                .source_snapshot
                .candidates
                .iter()
                .find(|item| item.item_id == proof.source_item_id)
                .cloned()
            else {
                return failure_result(
                    &work,
                    destination,
                    "NISAR_SUBSET_V2_ARCHIVED_RECOVERY_INVALID",
                    "归档恢复项目身份不一致。",
                    false,
                );
            };
            let source_work = DownloadWork {
                task: proof.source_snapshot,
                item,
            };
            let source_workspace = match Hdf5RangeWorkspaceV1::open_existing(
                nisar_subset_v2_recovery_workspace_root(&source_work),
            ) {
                Ok(workspace) => workspace,
                Err(_) => {
                    return failure_result(
                        &work,
                        destination,
                        "NISAR_SUBSET_V2_ARCHIVED_RECOVERY_INVALID",
                        "归档 checkpoint 不存在或路径不安全；未重新下载。",
                        false,
                    )
                }
            };
            match crate::nisar_science_transfer_v2::prepare_archived_nisar_recovery_v2(
                &source_work,
                &work,
                &proof.source_anchor.canonical_json,
                &proof.source_anchor.record_sha256,
                &source_workspace,
                &workspace,
                &control,
            ) {
                Ok(prepared) => Some(prepared),
                Err(error) => return nisar_v2_science_failure(&work, destination, error, &control),
            }
        } else {
            None
        };
        let body_activity = crate::transfer_activity::TaskBodyActivity::new(
            Arc::clone(&self.transfer_activity),
            work.task.task_id.clone(),
            work.item.item_id.clone(),
        );
        let subset_transfer = self.nisar_subset_transfer(&work.item.provider_id);
        let bootstrap = match crate::nisar_planning_v2::NisarSubsetHttpMetadataBootstrapV2::new(
            Arc::clone(&subset_transfer),
            asset,
            workspace,
        ) {
            Ok(bootstrap) => bootstrap.with_body_activity(body_activity.clone()),
            Err(error) => {
                return nisar_v2_planning_failure(&work, destination, error, None, &control)
            }
        };
        // Downloader enhancement D1c/D1d: a planning Range retry waits the
        // same backoff as a science Range, so tests shorten both together.
        #[cfg(test)]
        let bootstrap = match &self.nisar_subset_v2_science_retry_backoff {
            Some(backoff) => bootstrap.with_planning_retry_backoff_for_test(Arc::clone(backoff)),
            None => bootstrap,
        };
        // Issue #8 slice 6: no progress row before planning. A zero-rate row
        // means "science transfer complete, output being produced" to the
        // task card; until this run transfers bytes the item has no row and
        // reads as preparing, or verifying when verified bytes already exist.
        let mut progress_reporter = NisarSubsetTransferProgressReporter::new(
            work.task.task_id.clone(),
            work.item.item_id.clone(),
            progress,
        );
        let completed = match crate::plan_and_freeze_nisar_subset_v2_with_progress(
            &work,
            &control,
            authority,
            &bootstrap,
            &mut |delta| progress_reporter.observe(delta),
        ) {
            Ok(completed) => completed,
            Err(error) => {
                let observations = bootstrap.take_range_failure_observations();
                if let Some(evidence) = self.live_evidence.as_deref() {
                    if record_nisar_v2_range_failure_evidence(evidence, &observations).is_err() {
                        let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                        let _ = evidence.record_failure(code);
                        return failure_result(&work, destination, code, reason_zh, retryable);
                    }
                }
                return nisar_v2_planning_failure_after(
                    &work,
                    destination,
                    error,
                    &observations,
                    &control,
                );
            }
        };
        progress_reporter.flush();
        if let Some(evidence) = self.live_evidence.as_deref() {
            let observations = bootstrap.take_range_failure_observations();
            if record_nisar_v2_range_failure_evidence(evidence, &observations).is_err() {
                let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                let _ = evidence.record_failure(code);
                return failure_result(&work, destination, code, reason_zh, retryable);
            }
        }
        let plan_installation_valid =
            match insar_store::parse_nisar_subset_plan_v2_envelope(&work.item.metadata) {
                Ok(Some(envelope)) => envelope
                    .plan()
                    .is_ok_and(|plan| plan == *completed.frozen_plan()),
                Ok(None) => insar_store::embed_nisar_subset_plan_v2_envelope(
                    &mut work.item.metadata,
                    completed.frozen_plan(),
                )
                .is_ok(),
                Err(_) => false,
            };
        if !plan_installation_valid {
            let failure =
                crate::nisar_output_diagnostics_v2::nisar_v2_frozen_plan_installation_failure();
            return failure_result(
                &work,
                destination,
                failure.code,
                &failure.reason_zh,
                failure.retryable,
            );
        }
        #[cfg(test)]
        if let Some(hook) = &self.nisar_subset_v2_after_planning_hook {
            hook(&control);
        }
        if control.is_cancelled() || control.is_paused() {
            return nisar_v2_stopped_result(&work, destination, &control);
        }
        let physical = completed.physical_execution_plan().clone();
        // Issue #8 slice 6: a frozen plan, including one frozen before output
        // admission existed, is admitted again before any science request.
        if let Err(refusal) = crate::nisar_subset_stream_v2::v2_admit_planned_output(
            &physical,
            &work.task.output_root,
            0,
        ) {
            let failure = crate::nisar_output_diagnostics_v2::nisar_v2_output_refusal(
                "pre_science_admission",
                refusal,
                "本次没有发起科学数据请求；",
            );
            return failure_result(
                &work,
                destination,
                failure.code,
                &failure.reason_zh,
                failure.retryable,
            );
        }
        let asset = match bootstrap.resolved_asset_v2() {
            Ok(asset) => asset,
            Err(error) => {
                return nisar_v2_planning_failure(&work, destination, error, None, &control)
            }
        };
        if let Some(evidence) = self.live_evidence.as_deref() {
            let planning_evidence_result = record_nisar_v2_planning_evidence(
                evidence,
                &work,
                &prepared,
                &completed,
                asset.as_ref(),
                bootstrap.science_workspace_v2().part_path().as_path(),
            );
            if planning_evidence_result.is_err() {
                let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                let _ = evidence.record_failure(code);
                return failure_result(&work, destination, code, reason_zh, retryable);
            }
        }
        let remote_source = match crate::nisar_science_transfer_v2::NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
            asset.effective_url(),
            &work,
            &completed,
            &physical,
        ) {
            Ok(source) => source,
            Err(error) => {
                return nisar_v2_science_failure(&work, destination, error, &control)
            }
        };
        let science_anchor = authority.science_checkpoint_anchor_v2(
            &work.task.task_id,
            &work.item.item_id,
            &control,
        );
        if let Some(prepared) = recovery_prepared {
            if let Err(error) =
                crate::nisar_science_transfer_v2::finalize_archived_nisar_recovery_v2(
                    prepared,
                    &work,
                    &completed,
                    &remote_source,
                    science_anchor.as_ref(),
                    bootstrap.science_workspace_v2(),
                    &control,
                )
            {
                return nisar_v2_science_failure(&work, destination, error, &control);
            }
        }
        let science_authority =
            crate::nisar_science_transfer_v2::NisarSubsetScienceAuthorityBindingV2::new(
                &bootstrap,
                bootstrap.science_workspace_v2(),
                remote_source,
                science_anchor.as_ref(),
            );
        let science_executor =
            match crate::nisar_science_transfer_v2::NisarSubsetScienceRangeExecutorV2::new(
                subset_transfer,
                Arc::clone(&asset),
            ) {
                Ok(executor) => {
                    let executor = executor.with_body_activity(body_activity);
                    if !cfg!(test) {
                        executor.with_parallel_range_workers(nisar_science_parallel_range_workers(
                            &work.item.provider_id,
                        ))
                    } else {
                        executor
                    }
                }
                Err(error) => return nisar_v2_science_failure(&work, destination, error, &control),
            };
        #[cfg(test)]
        let science_executor = match &self.nisar_subset_v2_science_retry_backoff {
            Some(backoff) => science_executor.with_retry_backoff_for_test(Arc::clone(backoff)),
            None => science_executor,
        };
        #[cfg(test)]
        let science_executor = if let Some(hook) = &self.nisar_subset_v2_after_science_commit_hook {
            let hook = Arc::clone(hook);
            let hook_control = control.clone();
            science_executor.with_after_commit_hook_for_test(Arc::new(move |index| {
                hook(index, &hook_control);
            }))
        } else {
            science_executor
        };
        #[cfg(feature = "nisar-subset-live-acceptance")]
        let science_executor = if let Some(hook) = &self.nisar_subset_v2_body_interruption_hook {
            science_executor.with_after_body_read_interruption_hook_for_acceptance(Arc::clone(hook))
        } else {
            science_executor
        };
        let mut science_auth_cause = None;
        let science_result = if work.item.provider_id
            == insar_provider::catalog::BHOONIDHI_PROVIDER_ID
        {
            let mut auth_refresh =
                |rejected_asset: &ResolvedAsset, refresh_control: &WorkerControl| {
                    self.resolver_runtime
                        .refresh_bhoonidhi_nisar_remote_subset(&work.item, rejected_asset, &|| {
                            refresh_control.is_cancelled() || refresh_control.is_paused()
                        })
                        .map_err(|error| {
                            let (mapped, cause) =
                                nisar_v2_science_auth_refresh_failure(&error, refresh_control);
                            science_auth_cause = cause;
                            mapped
                        })
                };
            science_executor.execute_with_progress_and_auth_refresh(
                &work,
                &completed,
                &science_authority,
                &control,
                &mut |event| progress_reporter.observe_science(event),
                &mut auth_refresh,
            )
        } else {
            science_executor.execute_with_progress(
                &work,
                &completed,
                &science_authority,
                &control,
                &mut |event| progress_reporter.observe_science(event),
            )
        };
        let science = match science_result {
            Ok(science) => science,
            Err(error) => {
                return nisar_v2_science_execution_failure(
                    &work,
                    destination,
                    error,
                    &control,
                    science_auth_cause,
                    science_executor.failure_diagnostic_for(error),
                )
            }
        };
        progress_reporter.mark_transfer_complete();
        if let Some(evidence) = self.live_evidence.as_deref() {
            if record_nisar_v2_science_evidence(evidence, &completed, &science).is_err() {
                let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                let _ = evidence.record_failure(code);
                return failure_result(&work, destination, code, reason_zh, retryable);
            }
        }
        let writer_anchor =
            authority.writer_recovery_anchor_v2(&work.task.task_id, &work.item.item_id, &control);
        let writer = match OfflineNisarSubsetWriter::new(
            work.task.output_root.clone(),
            Arc::clone(&self.destination_lease),
        ) {
            Ok(writer) => writer
                .with_writer_recovery_anchor_v2(writer_anchor)
                .with_output_slots_v2(Arc::clone(&self.nisar_writer_output_slots)),
            Err(error) => {
                return nisar_v2_writer_failure(
                    &work,
                    destination,
                    error,
                    Some(&physical),
                    &control,
                )
            }
        };
        #[cfg(test)]
        let writer = match self.nisar_writer_long_wait_after {
            Some(threshold) => writer.with_long_wait_after_v2(threshold),
            None => writer,
        };
        // Downloader enhancement D1: a writer waiting for its output shows
        // 等待写出 until it is admitted. D1b: a wait longer than 5 minutes is
        // recorded once, with its reason, as a task event and a local line.
        let mut wait_started_at = None;
        let mut admission = |event| match event {
            crate::nisar_writer_admission_v2::NisarWriterAdmissionEventV2::Waiting { .. } => {
                wait_started_at = Some(timestamp_now());
                progress_reporter.mark_waiting_for_output()
            }
            crate::nisar_writer_admission_v2::NisarWriterAdmissionEventV2::LongWait {
                reason,
                holder_pid,
                waited,
            } => report_nisar_writer_long_wait(
                authority,
                &work,
                &control,
                reason,
                holder_pid,
                waited,
                wait_started_at.clone().unwrap_or_else(timestamp_now),
            ),
            crate::nisar_writer_admission_v2::NisarWriterAdmissionEventV2::Admitted => {
                progress_reporter.mark_output_admitted()
            }
        };
        #[cfg(test)]
        let write_result = match self.nisar_subset_v2_writer_failpoint {
            Some(failpoint) => writer.write_and_validate_v2_observed_with_failpoint_for_test(
                &work,
                &completed,
                &science,
                &science_authority,
                &control,
                failpoint,
                &mut admission,
            ),
            None => writer.write_and_validate_v2_observed(
                &work,
                &completed,
                &science,
                &science_authority,
                &control,
                &mut admission,
            ),
        };
        #[cfg(not(test))]
        let write_result = writer.write_and_validate_v2_observed(
            &work,
            &completed,
            &science,
            &science_authority,
            &control,
            &mut admission,
        );
        let written = match write_result {
            Ok(written) => written,
            #[cfg(test)]
            Err(crate::NisarSubsetWriteError::InjectedFailure)
                if self.nisar_subset_v2_writer_failpoint.is_some() =>
            {
                control.set_paused_for_test(true);
                return nisar_v2_stopped_result(&work, destination, &control);
            }
            Err(error) => {
                return nisar_v2_writer_failure(
                    &work,
                    destination,
                    error,
                    Some(&physical),
                    &control,
                )
            }
        };
        if let Some(evidence) = self.live_evidence.as_deref() {
            if evidence.record_output_v2(written.manifest()).is_err() {
                let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                let _ = evidence.record_failure(code);
                return failure_result(&work, destination, code, reason_zh, retryable);
            }
        }
        let artifact_size = match std::fs::metadata(written.artifact_path()) {
            Ok(metadata) if metadata.is_file() && metadata.len() > 0 => metadata.len(),
            _ => {
                let failure =
                    crate::nisar_output_diagnostics_v2::nisar_v2_published_output_failure(false);
                return failure_result(
                    &work,
                    destination,
                    failure.code,
                    &failure.reason_zh,
                    failure.retryable,
                );
            }
        };
        if !written.manifest_path().is_file() {
            let failure =
                crate::nisar_output_diagnostics_v2::nisar_v2_published_output_failure(true);
            return failure_result(
                &work,
                destination,
                failure.code,
                &failure.reason_zh,
                failure.retryable,
            );
        }
        let accounted_network_bytes = science.accounting().combined_response_body_bytes();
        let observed_network_bytes = progress_reporter.observed_bytes();
        drop(progress_reporter);
        let completed_bytes = accounted_network_bytes
            .max(observed_network_bytes)
            .saturating_add(artifact_size);
        progress(DownloadItemProgress {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            status: DownloadItemStatus::Completed,
            bytes_completed: completed_bytes,
            total_bytes: Some(completed_bytes),
            bytes_per_second: 0.0,
            attempt: 1,
            updated_at: timestamp_now(),
        });
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: written.artifact_path().to_path_buf(),
            bytes_written: artifact_size,
            attempt_count: 1,
            failure: None,
            finished_at: timestamp_now(),
        }
    }
}

/// Establishes the Earthdata session the worker's ordinary transfers share,
/// so task admission can create it before the first transfer needs it.
pub trait EarthdataSessionOwner: Send + Sync {
    /// `Ok(false)`: no saved username/password (token-only or none), so there is
    /// no session to pre-establish; the transfer reports its own requirement.
    fn ensure_earthdata_session(&self) -> Result<bool, DownloadFailure>;

    /// The saved Earthdata login changed: retire the shared session at once.
    fn invalidate_earthdata_sessions(&self);

    /// Whether the shared session is held right now; reads no credential and
    /// makes no request.
    fn earthdata_session_established(&self) -> bool;
}

impl EarthdataSessionOwner for ProductionDownloadWorker {
    fn ensure_earthdata_session(&self) -> Result<bool, DownloadFailure> {
        let Ok(credential) = resolve_earthdata_burst_credential(None) else {
            return Ok(false);
        };
        self.transfer
            .ensure_earthdata_session_with_credential(&credential)
            .map(|()| true)
    }

    fn invalidate_earthdata_sessions(&self) {
        self.transfer.invalidate_all_earthdata_sessions();
    }

    fn earthdata_session_established(&self) -> bool {
        self.transfer.has_earthdata_session()
    }
}

impl DownloadWorker for ProductionDownloadWorker {
    fn capability_runtime_observations(&self) -> Vec<(String, insar_core::RuntimeReadinessState)> {
        let resolver_state = if self.resolver_runtime_ready() {
            insar_core::RuntimeReadinessState::Ready
        } else {
            insar_core::RuntimeReadinessState::NotReady
        };
        let observations = vec![
            ("production-worker.complete.v1".to_owned(), resolver_state),
            (
                "orbit-download.asf-sentinel-1.v1".to_owned(),
                insar_core::RuntimeReadinessState::Ready,
            ),
        ];
        #[cfg(feature = "nisar-subset-production")]
        let observations = {
            let mut observations = observations;
            observations.push((
                "production-worker.nisar-remote-subset.v1".to_owned(),
                resolver_state,
            ));
            observations
        };
        observations
    }

    fn task_transfer_sample(&self, task_id: &str) -> Option<crate::TaskTransferSample> {
        // No observed body is unknown, not a measured zero-byte transfer.
        self.transfer_activity
            .sample_if_known(task_id, Instant::now())
            .ok()
            .flatten()
    }

    fn task_recent_stall_count(&self, task_id: &str) -> u32 {
        self.transfer_activity
            .recent_stall_count(task_id, Instant::now())
            .unwrap_or(0)
    }

    fn execute_with_authority(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        plan_freeze: &NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        if work
            .item
            .metadata
            .get("delivery_kind")
            .and_then(serde_json::Value::as_str)
            != Some(NISAR_SUBSET_V2_DELIVERY_KIND)
        {
            return self.execute_with_store(work, control, Some(plan_freeze), progress);
        }

        #[cfg(test)]
        if let Some(fixture) = &self.nisar_subset_v2_planning_fixture {
            let planned = crate::plan_and_freeze_nisar_subset_v2(
                &work,
                &control,
                plan_freeze,
                fixture.bootstrap.as_ref(),
            );
            match planned {
                Ok(planned) => {
                    let _ = fixture.planned.send(Ok(planned.clone()));
                    if fixture
                        .finish
                        .lock()
                        .ok()
                        .and_then(|finish| {
                            finish.recv_timeout(std::time::Duration::from_secs(10)).ok()
                        })
                        .is_none()
                    {
                        return failure_result(
                            &work,
                            work.task.output_root.clone(),
                            "NISAR_SUBSET_V2_PLANNING_FIXTURE_TIMEOUT",
                            "NISAR V2 planning fixture did not receive its bounded terminal gate.",
                            false,
                        );
                    }
                    return DownloadItemResult {
                        task_id: work.task.task_id,
                        item_id: work.item.item_id,
                        outcome: DownloadOutcome::Completed,
                        destination: work.task.output_root.join("planning-only.fixture"),
                        bytes_written: 0,
                        attempt_count: 1,
                        failure: None,
                        finished_at: timestamp_now(),
                    };
                }
                Err(error) => {
                    let reason = error.to_string();
                    let _ = fixture.planned.send(Err(reason.clone()));
                    return failure_result(
                        &work,
                        work.task.output_root.clone(),
                        "NISAR_SUBSET_V2_PLANNING_FIXTURE_FAILED",
                        &reason,
                        false,
                    );
                }
            }
        }

        #[cfg(feature = "nisar-subset-production")]
        {
            self.execute_nisar_subset_v2(work, control, plan_freeze, progress)
        }
        #[cfg(not(feature = "nisar-subset-production"))]
        {
            let _ = (control, plan_freeze, progress);
            failure_result(
                &work,
                work.task.output_root.clone(),
                "NISAR_SUBSET_V2_PRODUCTION_FEATURE_REQUIRED",
                "NISAR V2 execution requires the production subset feature.",
                false,
            )
        }
    }

    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.execute_with_store(work, control, None, progress)
    }

    fn supports_local_scan(&self) -> bool {
        true
    }

    /// Downloader D3-2: plans the destination exactly as the worker does,
    /// takes an idle destination lease without waiting, and decides locally.
    /// Only a frozen ASF Sentinel-1 file (Legacy) or a frozen Resolver asset
    /// is planned locally; anything else, and every asset with an artifact
    /// validation plan, is left to the worker.
    fn scan_local(
        &self,
        work: &DownloadWork,
        phase: crate::LocalScanPhase,
        control: &WorkerControl,
        store: Option<&NisarExecutionPlanFreezeAuthority>,
    ) -> crate::LocalScanVerdict {
        use crate::LocalScanVerdict as Verdict;
        if work
            .item
            .metadata
            .get("delivery_kind")
            .and_then(serde_json::Value::as_str)
            == Some(NISAR_SUBSET_V2_DELIVERY_KIND)
            || is_burst_asset(work)
        {
            return Verdict::Release;
        }
        let execution = self.resolver_runtime.classify(&work.item);
        let (file_name, size, checksum, needs_validation) = match &execution {
            crate::FrozenExecution::ResolverAuthoritative(eligible) => {
                let Some(file_name) = eligible.frozen_access.file_name.clone() else {
                    return Verdict::Release;
                };
                (
                    file_name,
                    authoritative_reuse_size(eligible),
                    frozen_resolver_checksum(&eligible.frozen_access),
                    eligible.has_artifact_validation_plan(),
                )
            }
            crate::FrozenExecution::ExplicitLegacy
            | crate::FrozenExecution::MissingPolicyCompatibility => {
                if DownloadProviderKind::from_provider_id(&work.task.provider_id)
                    != Some(DownloadProviderKind::AsfSentinel1)
                {
                    return Verdict::Release;
                }
                match sentinel_resolution_plan(work) {
                    Ok(SentinelResolutionPlan::Frozen {
                        file_name,
                        expected_size,
                        ..
                    }) => (file_name, expected_size, None, false),
                    _ => return Verdict::Release,
                }
            }
            _ => return Verdict::Release,
        };
        // Owner rule: an asset with a policy-bound validator is reused only
        // through that validator, which only the worker's route runs; the
        // scan neither digests nor reuses it.
        if needs_validation {
            return Verdict::Release;
        }
        let Ok(destination) = plan_work_target(work, &file_name) else {
            return Verdict::Release;
        };
        let _lease = match self.destination_lease.try_acquire_for_scan(&destination) {
            Ok(Some(guard)) => guard,
            Ok(None) | Err(_) => return Verdict::Release,
        };
        let request = GateRequest {
            work,
            destination: &destination,
            file_name: &file_name,
            authoritative_size: size,
            checksum: checksum.as_ref(),
            reuse_needs_validation: needs_validation,
            store,
            control,
        };
        match scan_local_reuse(&request, phase == crate::LocalScanPhase::Digest) {
            crate::local_reuse::ScanReuse::Release => Verdict::Release,
            crate::local_reuse::ScanReuse::NeedsDigest => Verdict::NeedsDigest,
            crate::local_reuse::ScanReuse::Reuse(item) => Verdict::Reuse(item),
            crate::local_reuse::ScanReuse::Stopped => Verdict::Stopped,
        }
    }
}

impl ProductionDownloadWorker {
    /// Runs one item outside the NISAR V2 route. `store` is the
    /// Scheduler-owned authority through which the local reuse gate reads
    /// and writes the Store (downloader D3); a direct call has none.
    fn execute_with_store(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        store: Option<&NisarExecutionPlanFreezeAuthority>,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        if work
            .item
            .metadata
            .get("delivery_kind")
            .and_then(serde_json::Value::as_str)
            == Some(NISAR_SUBSET_V2_DELIVERY_KIND)
        {
            return failure_result(
                &work,
                work.task.output_root.clone(),
                "NISAR_SUBSET_V2_AUTHORITY_REQUIRED",
                "NISAR V2 execution requires the Scheduler-owned execution authority path.",
                false,
            );
        }
        let burst_asset = is_burst_asset(&work);
        let execution = self.resolver_runtime.classify(&work.item);
        if burst_asset && !matches!(&execution, crate::FrozenExecution::ResolverAuthoritative(_)) {
            return failure_result(
                &work,
                work.task.output_root.clone(),
                "ASF_BURST_ASSET_INVALID",
                "Burst 资产缺少有效的 Resolver-authoritative 冻结执行策略，未进入 Legacy 下载。",
                false,
            );
        }
        if !burst_asset
            && work
                .item
                .metadata
                .get("delivery_kind")
                .and_then(|value| value.as_str())
                == Some("remote_derived_subset")
        {
            #[cfg(feature = "nisar-subset-production")]
            {
                return self.execute_nisar_remote_subset(work, control, progress);
            }
            #[cfg(not(feature = "nisar-subset-production"))]
            {
                return failure_result(
                    &work,
                    work.task.output_root.clone(),
                    "NISAR_SUBSET_NOT_ENABLED",
                    "当前桌面构建未包含 NISAR Range/writer/Store 运行链路。",
                    false,
                );
            }
        }
        route_frozen_execution(
            execution,
            (work, control, progress),
            |(work, control, progress)| self.execute_legacy(work, control, store, progress),
            |(work, control, progress), eligible| {
                self.execute_resolver_authoritative(work, &eligible, control, store, progress)
            },
            |(work, _control, _progress), error| {
                failure_result(
                    &work,
                    work.task.output_root.clone(),
                    error.code(),
                    error.reason_zh(),
                    error.retryable(),
                )
            },
        )
    }
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_live_evidence_write_failure() -> (&'static str, &'static str, bool) {
    (
        "NISAR_LIVE_EVIDENCE_WRITE_FAILED",
        "NISAR live evidence could not be persisted; acceptance stopped.",
        false,
    )
}

#[cfg(feature = "nisar-subset-production")]
fn record_nisar_v2_range_failure_evidence(
    evidence: &NisarLiveAcceptanceEvidenceSink,
    observations: &[crate::nisar_planning_v2::NisarSubsetRangeFailureObservationV2],
) -> std::io::Result<()> {
    for observation in observations {
        evidence.record_range_failure_observation(NisarLiveRangeFailureObservationInput {
            stage: &observation.stage,
            start: observation.start,
            end_exclusive: observation.end_exclusive,
            source_size_bytes: observation.source_size_bytes,
            disposition: observation.disposition,
            request_count: observation.request_count,
            http_status: observation.http_status,
            response_body_bytes: observation.response_body_bytes,
            validator_sha256: observation.validator_sha256.as_deref(),
            failure_code: observation.failure_code,
            redirect_occurred: observation.redirect_occurred,
            redirect_hop_index: observation.redirect_hop_index,
            redirect_source_host_class: observation.redirect_source_host_class.as_deref(),
            redirect_destination_host_class: observation.redirect_destination_host_class.as_deref(),
            redirect_source_path_class: observation.redirect_source_path_class.as_deref(),
            redirect_destination_path_class: observation.redirect_destination_path_class.as_deref(),
            redirect_same_origin: observation.redirect_same_origin,
            redirect_scheme_transition: observation.redirect_scheme_transition.as_deref(),
            redirect_policy_result: observation.redirect_policy_result.as_deref(),
            redirect_rejection_reason: observation.redirect_rejection_reason.as_deref(),
        })?;
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_subset_checksum_algorithm_name(
    algorithm: &insar_core::NisarSubsetChecksumAlgorithm,
) -> &'static str {
    match algorithm {
        insar_core::NisarSubsetChecksumAlgorithm::Md5 => "MD5",
        insar_core::NisarSubsetChecksumAlgorithm::Sha1 => "SHA1",
        insar_core::NisarSubsetChecksumAlgorithm::Sha256 => "SHA256",
    }
}

#[cfg(feature = "nisar-subset-production")]
fn record_nisar_v2_startup_evidence(
    evidence: &NisarLiveAcceptanceEvidenceSink,
    work: &DownloadWork,
    prepared: &insar_core::NisarSubsetPreparedIntentV2,
) -> std::io::Result<()> {
    let source_size = work
        .item
        .asset_access
        .as_ref()
        .and_then(|asset| asset.expected_size_bytes)
        .unwrap_or_default();
    let (checksum_algorithm, checksum) = prepared
        .source_asset_identity
        .checksum
        .as_ref()
        .map(|checksum| {
            (
                nisar_subset_checksum_algorithm_name(&checksum.algorithm),
                checksum.value.as_str(),
            )
        })
        .unwrap_or(("not_observed", "not_observed"));
    let advertised = prepared
        .source_capability
        .frequencies
        .iter()
        .find(|capability| capability.frequency == prepared.selected_frequency)
        .map(|capability| capability.available_polarizations.as_slice())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing NISAR capability")
        })?;
    evidence.record_scope(
        &work.item.display_name,
        source_size,
        checksum_algorithm,
        checksum,
        prepared.selected_frequency.clone(),
        &prepared.selected_polarizations.join("+"),
    )?;
    evidence.record_source_identity(NisarLiveSourceIdentityInput {
        provider_id: &prepared.source_product_identity.provider_id,
        provider_product_id: &prepared.source_product_identity.provider_product_id,
        provider_product_key: &prepared.source_product_identity.provider_product_key,
        collection_id: &prepared.source_product_identity.collection_id,
        asset_key: &prepared.source_asset_identity.asset_key,
        stable_asset_key: &prepared.source_asset_identity.stable_asset_key,
        source_product_identity_sha256: &prepared.provenance.source_product_identity_sha256,
        source_asset_identity_sha256: &prepared.provenance.source_asset_identity_sha256,
    })?;
    evidence.record_polarizations(advertised, &prepared.selected_polarizations)?;
    evidence.record_geometry_digests(
        &prepared.source_footprint_sha256,
        &prepared.canonical_geometry_digest.requested_sha256,
        &prepared.canonical_geometry_digest.effective_sha256,
    )?;
    evidence.record_prepared_intent(&prepared.canonical_intent_sha256)
}

#[cfg(feature = "nisar-subset-production")]
fn record_nisar_v2_planning_evidence(
    evidence: &NisarLiveAcceptanceEvidenceSink,
    work: &DownloadWork,
    prepared: &insar_core::NisarSubsetPreparedIntentV2,
    completed: &crate::NisarSubsetPlanningCompleteV2,
    asset: &ResolvedAsset,
    sparse_path: &std::path::Path,
) -> std::io::Result<()> {
    let physical = completed.physical_execution_plan();
    let statistics = completed.bootstrap_statistics();
    let budget = physical.budget_projection();
    let source_size = physical.source_size();
    #[cfg(feature = "nisar-subset-live-acceptance")]
    {
        let snapshot = evidence.snapshot();
        if snapshot.charged_failure_requests != statistics.charged_failure_requests()
            || snapshot.charged_failure_response_bytes
                != statistics.charged_failure_response_bytes()
        {
            return Err(std::io::Error::other(
                "NISAR V2 failed Range evidence does not match planning accounting",
            ));
        }
    }
    let combined_planning_requests = statistics
        .total_planning_request_count()
        .checked_add(statistics.charged_failure_requests())
        .ok_or_else(|| std::io::Error::other("NISAR V2 planning request counter overflow"))?;
    let combined_planning_response_bytes = statistics
        .total_planning_response_body_bytes()
        .checked_add(statistics.charged_failure_response_bytes())
        .ok_or_else(|| std::io::Error::other("NISAR V2 planning byte counter overflow"))?;
    let validator_sha256 = physical.source_validator_sha256();
    let validator_hash = nisar_live_sha256(physical.source_validator().value.as_bytes());
    let locator_hash = nisar_live_locator_hash(asset.effective_url());

    evidence.record_preflight(
        statistics.preflight().request_count(),
        statistics.preflight().response_body_bytes(),
        "earthdata_asf_v1",
        "approved_nisar_hdf5_object",
        &locator_hash,
        &validator_hash,
    )?;
    evidence.record_metadata_prefix(
        statistics.metadata_prefix().request_count(),
        statistics.metadata_prefix().response_body_bytes(),
    )?;
    evidence.record_metadata_discovery(
        statistics.metadata_discovery().request_count(),
        statistics.metadata_discovery().response_body_bytes(),
    )?;
    evidence.record_additional_geolocation(
        statistics.additional_geolocation().request_count(),
        statistics.additional_geolocation().response_body_bytes(),
    )?;
    record_nisar_v2_category_ranges(
        evidence,
        "preflight",
        statistics.preflight(),
        source_size,
        validator_sha256,
    )?;
    record_nisar_v2_category_ranges(
        evidence,
        "metadata_prefix",
        statistics.metadata_prefix(),
        source_size,
        validator_sha256,
    )?;
    record_nisar_v2_category_ranges(
        evidence,
        "metadata_discovery",
        statistics.metadata_discovery(),
        source_size,
        validator_sha256,
    )?;
    record_nisar_v2_category_ranges(
        evidence,
        "additional_geolocation",
        statistics.additional_geolocation(),
        source_size,
        validator_sha256,
    )?;
    record_nisar_v2_category_ranges(
        evidence,
        "planning_revalidation",
        statistics.revalidation(),
        source_size,
        validator_sha256,
    )?;
    evidence.record_planning_revalidation(
        statistics.revalidation().request_count(),
        statistics.revalidation().response_body_bytes(),
    )?;

    evidence.record_plan_digests(NisarLivePlanDigestsInput {
        prepared_intent_sha256: &prepared.canonical_intent_sha256,
        final_plan_sha256: physical.final_plan_sha256(),
        physical_plan_sha256: physical.physical_plan_sha256(),
        inventory_sha256: physical.inventory_sha256(),
        inventory_evidence_sha256: physical.inventory_evidence_sha256(),
        source_graph_sha256: physical.source_graph_sha256(),
        source_validator_sha256: validator_sha256,
    })?;

    // Keep the established V1 execution and semantic identities observable on
    // the V2 path too. Rebuild them from the exact materialized part and the
    // primary (HH) window; this is read-only and issues no network requests.
    let aoi = &prepared.requested_geometry.canonical_geojson;
    let primary_polarization = prepared.selected_polarizations.first().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing polarization")
    })?;
    let compatibility_request = crate::build_live_nisar_subset_request(
        &work.item,
        aoi,
        prepared.selected_frequency.clone(),
        primary_polarization,
        "LSAR_RSLC",
        sparse_path,
    )
    .map_err(|error| std::io::Error::other(format!("V1 compatibility request: {error}")))?;
    let compatibility_metadata =
        crate::NisarSubsetMetadataSnapshotV1::from_verified_live_metadata_hdf5(
            &compatibility_request,
            physical.source_validator().clone(),
            sparse_path,
            &|| false,
        )
        .map_err(|error| std::io::Error::other(format!("V1 compatibility metadata: {error}")))?;
    let compatibility_plan = crate::live_nisar_range_plan_from_sparse_hdf5(
        &compatibility_request,
        sparse_path,
        compatibility_metadata.evidence_sha256(),
    )
    .map_err(|error| std::io::Error::other(format!("V1 compatibility plan: {error}")))?;
    let compatibility_semantic = crate::derive_nisar_live_semantic_plan_sha256(
        &compatibility_request,
        &compatibility_metadata,
        &compatibility_plan,
    )
    .map_err(|error| std::io::Error::other(format!("V1 compatibility semantic plan: {error}")))?;
    evidence.record_execution_plan(compatibility_plan.plan_sha256())?;
    evidence.record_semantic_plan(&compatibility_semantic)?;
    evidence.record_metadata_inventory(
        completed
            .verified_inventory_identity()
            .source_dataset_count(),
        completed
            .verified_inventory_identity()
            .metadata_group_binding_count(),
        completed
            .verified_inventory_identity()
            .metadata_dataset_binding_count(),
        physical.inventory_sha256(),
    )?;
    evidence.record_budget_limits(NisarLiveBudgetLimitsInput {
        max_requests: budget.max_request_count(),
        max_response_bytes: budget.max_response_body_bytes(),
        remaining_requests: budget.remaining_science_request_count(),
        remaining_response_bytes: budget.remaining_science_response_body_bytes(),
        planning_requests: combined_planning_requests,
        planning_response_bytes: combined_planning_response_bytes,
        reserved_science_requests: budget.projected_science_request_count(),
        reserved_science_response_bytes: budget.projected_science_response_body_bytes(),
        committed_science_requests: 0,
        committed_science_response_bytes: 0,
    })
}

#[cfg(feature = "nisar-subset-production")]
fn record_nisar_v2_category_ranges(
    evidence: &NisarLiveAcceptanceEvidenceSink,
    stage: &str,
    statistics: &crate::NisarSubsetBootstrapCategoryStatisticsV2,
    source_size: u64,
    validator_sha256: &str,
) -> std::io::Result<()> {
    let request_counts = if statistics.request_counts().is_empty() {
        vec![1_u64; statistics.requested_ranges().len()]
    } else {
        statistics.request_counts().to_vec()
    };
    if request_counts.len() != statistics.requested_ranges().len() {
        return Err(std::io::Error::other(
            "NISAR V2 planning Range count evidence mismatch",
        ));
    }
    evidence.begin_planning_range_snapshot(stage, source_size, validator_sha256)?;
    for (range, request_count) in statistics.requested_ranges().iter().zip(request_counts) {
        let response_bytes = range
            .end_exclusive()
            .checked_sub(range.start())
            .ok_or_else(|| std::io::Error::other("NISAR V2 invalid planning range"))?;
        evidence.record_range_observation(NisarLiveRangeObservationInput {
            stage,
            start: range.start(),
            end_exclusive: range.end_exclusive(),
            source_size_bytes: source_size,
            request_count,
            http_status: 206,
            response_body_bytes: response_bytes,
            validator_sha256,
        })?;
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
fn record_nisar_v2_science_evidence(
    evidence: &NisarLiveAcceptanceEvidenceSink,
    completed: &crate::NisarSubsetPlanningCompleteV2,
    science: &crate::nisar_science_transfer_v2::NisarSubsetScienceTransferResultV2,
) -> std::io::Result<()> {
    let physical = completed.physical_execution_plan();
    let accounting = science.accounting();
    if science.completed_ranges().len() != science.completed_range_request_counts().len()
        || science.completed_range_request_counts().contains(&0)
        || science
            .completed_range_request_counts()
            .iter()
            .try_fold(0_u64, |total, count| total.checked_add(*count))
            != Some(accounting.committed_science_request_count())
    {
        return Err(std::io::Error::other(
            "NISAR V2 science completion count does not match accounting",
        ));
    }
    let mut committed_bytes = 0_u64;
    for (&(start, end_exclusive), &request_count) in science
        .completed_ranges()
        .iter()
        .zip(science.completed_range_request_counts())
    {
        let response_bytes = end_exclusive
            .checked_sub(start)
            .ok_or_else(|| std::io::Error::other("NISAR V2 invalid science range"))?;
        committed_bytes = committed_bytes
            .checked_add(response_bytes)
            .ok_or_else(|| std::io::Error::other("NISAR V2 science byte counter overflow"))?;
        evidence.record_range_observation(NisarLiveRangeObservationInput {
            stage: "science",
            start,
            end_exclusive,
            source_size_bytes: physical.source_size(),
            request_count,
            http_status: 206,
            response_body_bytes: response_bytes,
            validator_sha256: physical.source_validator_sha256(),
        })?;
    }
    if committed_bytes != accounting.committed_science_response_body_bytes() {
        return Err(std::io::Error::other(
            "NISAR V2 science response bytes do not match accounting",
        ));
    }
    let failed_bytes = accounting
        .observed_science_response_body_bytes()
        .checked_sub(accounting.committed_science_response_body_bytes())
        .ok_or_else(|| {
            std::io::Error::other("science observed byte count below committed bytes")
        })?;
    if accounting.science_request_count()
        != accounting
            .committed_science_request_count()
            .checked_add(accounting.failed_science_request_count())
            .ok_or_else(|| std::io::Error::other("science request overflow"))?
        || accounting.indeterminate_science_request_count() != 0
        || accounting.indeterminate_science_response_body_bytes() != 0
    {
        return Err(std::io::Error::other(
            "science failure ledger is incomplete",
        ));
    }
    evidence.record_science_failures(accounting.failed_science_request_count(), failed_bytes)?;
    evidence.record_science_range(
        accounting.committed_science_request_count(),
        accounting.committed_science_response_body_bytes(),
    )?;
    let remaining_requests = science
        .max_request_count()
        .checked_sub(accounting.planning_request_count())
        .and_then(|value| value.checked_sub(accounting.reserved_science_request_count()))
        .ok_or_else(|| std::io::Error::other("NISAR V2 remaining request budget underflow"))?;
    if accounting.reserved_science_request_count() > science.max_science_request_count() {
        return Err(std::io::Error::other(
            "NISAR V2 effective science request budget exceeded",
        ));
    }
    let remaining_response_bytes = science
        .max_response_body_bytes()
        .checked_sub(accounting.planning_response_body_bytes())
        .and_then(|value| value.checked_sub(accounting.reserved_science_response_body_bytes()))
        .ok_or_else(|| std::io::Error::other("NISAR V2 remaining byte budget underflow"))?;
    evidence.record_budget_limits(NisarLiveBudgetLimitsInput {
        max_requests: science.max_request_count(),
        max_response_bytes: science.max_response_body_bytes(),
        remaining_requests,
        remaining_response_bytes,
        planning_requests: accounting.planning_request_count(),
        planning_response_bytes: accounting.planning_response_body_bytes(),
        reserved_science_requests: accounting.reserved_science_request_count(),
        reserved_science_response_bytes: accounting.reserved_science_response_body_bytes(),
        committed_science_requests: accounting.committed_science_request_count(),
        committed_science_response_bytes: accounting.committed_science_response_body_bytes(),
    })
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_live_locator_hash(url: &reqwest::Url) -> String {
    let mut identity = String::new();
    identity.push_str(url.scheme());
    identity.push_str("://");
    identity.push_str(url.host_str().unwrap_or_default());
    identity.push_str(url.path());
    nisar_live_sha256(identity.as_bytes())
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_live_sha256(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

#[cfg(feature = "nisar-subset-production")]
impl ProductionDownloadWorker {
    fn execute_nisar_remote_subset(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let task_id = work.task.task_id.clone();
        let item_id = work.item.item_id.clone();
        let evidence = self.live_evidence.as_deref();
        if let Some(evidence) = evidence {
            if evidence.mark_running().is_err() {
                let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                let _ = evidence.record_failure(code);
                return failure_result(
                    &work,
                    work.task.output_root.clone(),
                    code,
                    reason_zh,
                    retryable,
                );
            }
        }
        let result = self.run_nisar_remote_subset(&work, &control, progress, evidence);
        match result {
            Ok((destination, bytes_written)) => {
                if let Some(evidence) = evidence {
                    if evidence.finish_pass().is_err() {
                        let (code, reason_zh, retryable) = nisar_live_evidence_write_failure();
                        let _ = evidence.record_failure(code);
                        return failure_result(
                            &work,
                            work.task.output_root.clone(),
                            code,
                            reason_zh,
                            retryable,
                        );
                    }
                }
                DownloadItemResult {
                    task_id,
                    item_id,
                    outcome: if control.is_cancelled() {
                        DownloadOutcome::Cancelled
                    } else {
                        DownloadOutcome::Completed
                    },
                    destination,
                    bytes_written,
                    attempt_count: 1,
                    failure: None,
                    finished_at: timestamp_now(),
                }
            }
            Err((code, reason_zh, retryable)) => {
                if let Some(evidence) = evidence {
                    let _ = evidence.finish_failure(code);
                }
                failure_result(
                    &work,
                    work.task.output_root.clone(),
                    code,
                    reason_zh,
                    retryable,
                )
            }
        }
    }

    fn run_nisar_remote_subset(
        &self,
        work: &DownloadWork,
        control: &WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
        evidence: Option<&NisarLiveAcceptanceEvidenceSink>,
    ) -> Result<(PathBuf, u64), (&'static str, &'static str, bool)> {
        let cancelled = || control.is_cancelled();
        let frequency = match work
            .item
            .metadata
            .get("subset_frequency")
            .and_then(|value| value.as_str())
        {
            Some("A") => NisarSubsetFrequency::A,
            Some("B") => NisarSubsetFrequency::B,
            _ => {
                return Err((
                    "NISAR_SUBSET_SELECTION_INVALID",
                    "NISAR 频率选择无效。",
                    false,
                ));
            }
        };
        let polarization = work
            .item
            .metadata
            .get("subset_polarization")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or((
                "NISAR_SUBSET_SELECTION_INVALID",
                "NISAR 极化选择无效。",
                false,
            ))?;
        if let Some(evidence) = evidence {
            let source = work.item.asset_access.as_ref();
            evidence
                .record_scope(
                    &work.item.display_name,
                    source
                        .and_then(|asset| asset.expected_size_bytes)
                        .unwrap_or(0),
                    source
                        .and_then(|asset| asset.checksum_algorithm.as_deref())
                        .unwrap_or("not_observed"),
                    source
                        .and_then(|asset| asset.checksum_value.as_deref())
                        .unwrap_or("not_observed"),
                    frequency.clone(),
                    polarization,
                )
                .map_err(|_| nisar_live_evidence_write_failure())?;
        }
        let aoi = work
            .item
            .metadata
            .get("subset_aoi_geojson")
            .and_then(|value| value.as_object())
            .ok_or((
                "NISAR_SUBSET_REQUEST_INVALID",
                "NISAR 子集任务缺少冻结 AOI。",
                false,
            ))?;
        let expected_size = work
            .item
            .asset_access
            .as_ref()
            .and_then(|asset| asset.expected_size_bytes)
            .ok_or((
                "NISAR_SUBSET_SOURCE_INVALID",
                "NISAR 源产品缺少冻结文件大小。",
                false,
            ))?;
        let checksum_algorithm = work
            .item
            .asset_access
            .as_ref()
            .and_then(|asset| asset.checksum_algorithm.as_deref())
            .ok_or((
                "NISAR_SUBSET_SOURCE_INVALID",
                "NISAR 源产品缺少冻结完整性算法。",
                false,
            ))?;
        let checksum = work
            .item
            .asset_access
            .as_ref()
            .and_then(|asset| asset.checksum_value.as_deref())
            .ok_or((
                "NISAR_SUBSET_SOURCE_INVALID",
                "NISAR 源产品缺少冻结完整性值。",
                false,
            ))?;
        let mut asset = self
            .resolver_runtime
            .resolve_for_nisar_remote_subset(&work.item, &cancelled)
            .map_err(|error| {
                if error.code() == "DL004" {
                    (
                        "NISAR_SUBSET_CREDENTIAL_OR_PERMISSION",
                        "NISAR 下载需要有效的 Earthdata 登录凭据，或当前账户没有该产品权限。",
                        false,
                    )
                } else {
                    (
                        "NISAR_SUBSET_RESOLUTION_FAILED",
                        "NISAR 源产品解析失败，未回退到完整文件下载。",
                        error.retryable(),
                    )
                }
            })?;
        asset = asset
            .with_expected_size(Some(expected_size))
            .with_checksum(Some(
                ResolvedChecksum::new(checksum_algorithm, checksum).map_err(|_| {
                    (
                        "NISAR_SUBSET_SOURCE_INVALID",
                        "NISAR 源产品完整性值格式无效。",
                        false,
                    )
                })?,
            ));
        let network_budget =
            NisarLiveNetworkBudgetV1::new(expected_size).map_err(map_nisar_range_error)?;
        let preflight = preflight_live_nisar_source_with_budget(
            &self.transfer,
            &asset,
            expected_size,
            &cancelled,
            Some(&network_budget),
        )
        .map_err(map_nisar_range_error)?;
        let transport_admission = preflight
            .admission_facts(expected_size)
            .map_err(map_nisar_range_error)?;
        let preflight_requests = preflight.request_count();
        let preflight_response_body_bytes = preflight.response_body_bytes();
        if let Some(evidence) = evidence {
            evidence
                .record_preflight(
                    preflight_requests,
                    preflight_response_body_bytes,
                    preflight.final_host_class(),
                    preflight.final_path_class(),
                    preflight.locator_hash(),
                    preflight.validator_hash(),
                )
                .map_err(|_| nisar_live_evidence_write_failure())?;
        }
        let source_validator = preflight.into_source_validator();
        asset = asset.with_remote_validator(RemoteValidator {
            etag: Some(source_validator.value.clone()),
            last_modified: None,
            content_length: Some(source_validator.content_length),
        });
        if cancelled() {
            return Err(("DL001", "下载已取消。", false));
        }
        let workspace_root = work
            .task
            .output_root
            .join(".insar-nisar-range")
            .join(&work.task.task_id);
        let workspace = Hdf5RangeWorkspaceV1::new(workspace_root).map_err(|_| {
            (
                "NISAR_SUBSET_WORKSPACE_FAILED",
                "NISAR 子集 Range 工作区不可用。",
                false,
            )
        })?;
        let metadata_prefetch = prefetch_live_nisar_metadata_with_budget(
            &self.transfer,
            &asset,
            &workspace,
            &source_validator,
            expected_size,
            &cancelled,
            Some(&network_budget),
        )
        .map_err(map_nisar_range_error)?;
        if let Some(evidence) = evidence {
            evidence
                .record_metadata_prefix(
                    metadata_prefetch.request_count(),
                    metadata_prefetch.response_body_bytes(),
                )
                .map_err(|_| nisar_live_evidence_write_failure())?;
        }
        let metadata_discovery = discover_live_nisar_metadata_chunks_with_budget(
            &self.transfer,
            &asset,
            &workspace,
            &source_validator,
            expected_size,
            &cancelled,
            Some(&network_budget),
        )
        .map_err(map_nisar_range_error)?;
        if let Some(evidence) = evidence {
            evidence
                .record_metadata_discovery(
                    metadata_discovery.request_count,
                    metadata_discovery.response_body_bytes,
                )
                .map_err(|_| nisar_live_evidence_write_failure())?;
        }
        let request = build_live_nisar_subset_request(
            &work.item,
            aoi,
            frequency,
            polarization,
            "LSAR_RSLC",
            &workspace.part_path(),
        )
        .map_err(|error| {
            if error == Hdf5RangeTransferError::RangeBytesExceeded {
                return (
                    "NISAR_SUBSET_RESOURCE_BUDGET_EXCEEDED",
                    "NISAR subset physical byte plan exceeds the bounded transfer budget.",
                    false,
                );
            }
            (
                "NISAR_SUBSET_MAPPING_UNAVAILABLE",
                "NISAR 元数据中没有足够的地理定位网格来映射当前 AOI。",
                false,
            )
        })?;
        if let Some(evidence) = evidence {
            evidence
                .record_aoi_digest(&request.request.aoi_sha256)
                .map_err(|_| nisar_live_evidence_write_failure())?;
            if let Some(window) = request.mapping.windows.first() {
                evidence
                    .record_geometry(
                        window.line_start,
                        window.line_end_exclusive,
                        window.sample_start,
                        window.sample_end_exclusive,
                    )
                    .map_err(|_| nisar_live_evidence_write_failure())?;
            }
        }
        let metadata_snapshot = NisarSubsetMetadataSnapshotV1::from_verified_live_metadata_hdf5(
            &request,
            source_validator.clone(),
            &workspace.part_path(),
            &cancelled,
        )
        .map_err(|_| {
            (
                "NISAR_SUBSET_METADATA_INVALID",
                "NISAR 元数据结构校验失败，未写入最终文件。",
                false,
            )
        })?;
        if let Some(evidence) = evidence {
            evidence
                .record_metadata_inventory(
                    metadata_snapshot.source_dataset_count(),
                    metadata_snapshot.metadata_group_binding_count(),
                    metadata_snapshot.metadata_dataset_binding_count(),
                    metadata_snapshot.source_inventory_sha256(),
                )
                .map_err(|_| nisar_live_evidence_write_failure())?;
        }
        let plan = live_nisar_range_plan_from_sparse_hdf5(
            &request,
            &workspace.part_path(),
            metadata_snapshot.evidence_sha256(),
        )
        .map_err(|_| {
            (
                "NISAR_SUBSET_RANGE_PLAN_INVALID",
                "NISAR 子集 Range 计划与实际 HDF5 布局不一致。",
                false,
            )
        })?;
        if let Some(evidence) = evidence {
            evidence
                .record_execution_plan(plan.plan_sha256())
                .map_err(|_| nisar_live_evidence_write_failure())?;
        }
        let semantic_plan_sha256 =
            derive_nisar_live_semantic_plan_sha256(&request, &metadata_snapshot, &plan).map_err(
                |_| {
                    (
                        "NISAR_SUBSET_SEMANTIC_PLAN_INVALID",
                        "NISAR 瀛愰泦璇箟璁″垝鏃犳硶寤虹珛锛屾湭杩涘叆浼犺緭銆?",
                        false,
                    )
                },
            )?;
        if let Some(evidence) = evidence {
            evidence
                .record_semantic_plan(&semantic_plan_sha256)
                .map_err(|_| nisar_live_evidence_write_failure())?;
        }
        validate_nisar_live_admission(
            &work.item,
            &request,
            &metadata_snapshot,
            &plan,
            &source_validator,
            &transport_admission,
            &network_budget,
        )
        .map_err(|error| {
            (
                error.code(),
                "NISAR live admission failed an explicit source, transport, request, mapping, metadata, physical-plan, or budget invariant.",
                false,
            )
        })?;
        #[cfg(feature = "nisar-subset-live-acceptance")]
        if let Some(gate) = self.nisar_isce3_geometry_gate.as_ref() {
            crate::nisar_isce3_oracle::enforce_isce3_geometry_gate(
                gate,
                &request,
                &metadata_snapshot,
                &workspace.part_path(),
                aoi,
            )
            .map_err(|error| {
                (
                    error.code(),
                    "NISAR independent ISCE3 geometry gate did not admit science Range execution.",
                    false,
                )
            })?;
        }
        seed_live_metadata_range(
            &workspace,
            &request,
            &plan,
            &source_validator,
            metadata_prefetch.prefix_sha256(),
            &timestamp_now(),
        )
        .map_err(|_| {
            (
                "NISAR_SUBSET_METADATA_SEED_FAILED",
                "NISAR 元数据 Range 无法安全写入工作区。",
                false,
            )
        })?;
        let range_transfer = OfflineHdf5RangeTransfer::new_shared(
            Arc::clone(&self.transfer),
            &request,
            &work.item,
            asset,
            workspace.clone(),
            Arc::clone(&self.destination_lease),
        )
        .map_err(|_| {
            (
                "NISAR_SUBSET_TRANSFER_BIND_FAILED",
                "NISAR 子集 Range 传输绑定失败。",
                false,
            )
        })?;
        let transfer_result = range_transfer
            .execute_with_nisar_budget(
                &request,
                &plan,
                &timestamp_now(),
                &cancelled,
                &network_budget,
            )
            .map_err(map_nisar_range_error)?;
        drop(range_transfer);
        // The transfer checkpoint deliberately counts the seeded metadata
        // prefix as materialized bytes.  Network accounting must count only
        // science response bodies here; the prefix is already accounted for
        // by metadata_prefetch above.
        let science_response_body_bytes = transfer_result
            .metrics()
            .response_body_bytes
            .checked_sub(crate::NISAR_LIVE_METADATA_PREFIX_BYTES)
            .ok_or_else(|| map_nisar_range_error(Hdf5RangeTransferError::RecoveryInvalid))?;
        if let Some(evidence) = evidence {
            evidence
                .record_science_range(
                    transfer_result.metrics().request_count,
                    science_response_body_bytes,
                )
                .map_err(|_| nisar_live_evidence_write_failure())?;
        }
        let total_requests = preflight_requests
            .checked_add(metadata_prefetch.request_count())
            .and_then(|value| value.checked_add(metadata_discovery.request_count))
            .and_then(|value| value.checked_add(transfer_result.metrics().request_count))
            .ok_or_else(|| map_nisar_range_error(Hdf5RangeTransferError::RangeCountExceeded))?;
        let total_response_body_bytes = preflight_response_body_bytes
            .checked_add(metadata_prefetch.response_body_bytes())
            .and_then(|value| value.checked_add(metadata_discovery.response_body_bytes))
            .and_then(|value| value.checked_add(science_response_body_bytes))
            .ok_or_else(|| map_nisar_range_error(Hdf5RangeTransferError::RangeBytesExceeded))?;
        network_budget
            .validate_actual(total_requests, total_response_body_bytes)
            .map_err(map_nisar_range_error)?;
        let writer = OfflineNisarSubsetWriter::new(
            work.task.output_root.clone(),
            Arc::clone(&self.destination_lease),
        )
        .map_err(|_| {
            (
                "NISAR_SUBSET_WRITER_INIT_FAILED",
                "NISAR 子集写入器初始化失败。",
                false,
            )
        })?;
        let written = writer
            .write_and_validate(
                &request,
                &plan,
                &transfer_result,
                &workspace,
                &metadata_snapshot,
                &cancelled,
            )
            .map_err(|_| {
                (
                    "NISAR_SUBSET_VALIDATION_FAILED",
                    "NISAR 子集科学结构校验失败，未保留最终产物。",
                    false,
                )
            })?;
        if let Some(evidence) = evidence {
            evidence
                .record_output(written.manifest())
                .map_err(|_| nisar_live_evidence_write_failure())?;
        }
        let bytes_written = std::fs::metadata(written.artifact_path())
            .map_err(|_| {
                (
                    "NISAR_SUBSET_OUTPUT_MISSING",
                    "NISAR 子集最终文件未生成。",
                    false,
                )
            })?
            .len();
        workspace.cleanup_after_success().map_err(|_| {
            (
                "NISAR_SUBSET_CLEANUP_FAILED",
                "NISAR 子集最终产物已验证，但临时 Range 工作区未能安全清理。",
                true,
            )
        })?;
        #[cfg(feature = "nisar-subset-live-acceptance")]
        if let Some(evidence) = evidence {
            evidence
                .mark_cleanup_completed(true)
                .map_err(|_| nisar_live_evidence_write_failure())?;
        }
        progress(DownloadItemProgress {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            status: DownloadItemStatus::Completed,
            bytes_completed: bytes_written,
            total_bytes: Some(bytes_written),
            bytes_per_second: bytes_written as f64,
            attempt: 1,
            updated_at: timestamp_now(),
        });
        Ok((written.artifact_path().to_path_buf(), bytes_written))
    }
}

#[cfg(feature = "nisar-subset-production")]
fn map_nisar_range_error(error: Hdf5RangeTransferError) -> (&'static str, &'static str, bool) {
    match error {
        Hdf5RangeTransferError::Cancelled => ("DL001", "下载已取消。", false),
        Hdf5RangeTransferError::CredentialsRequired => (
            "NISAR_SUBSET_CREDENTIAL_OR_PERMISSION",
            "NISAR 下载需要有效的 Earthdata 登录凭据，或当前账户没有该产品权限。",
            false,
        ),
        Hdf5RangeTransferError::StrongValidatorRequired
        | Hdf5RangeTransferError::WeakEtagRejected => (
            "NISAR_SUBSET_STRONG_VALIDATOR_FAILED",
            "NISAR 子集要求冻结的强 ETag，未接受弱校验器或无校验器响应。",
            false,
        ),
        Hdf5RangeTransferError::SourceChanged => (
            "NISAR_SUBSET_SOURCE_CHANGED",
            "NISAR 源产品强校验器在 Range 过程中发生变化，已终止子集任务。",
            false,
        ),
        Hdf5RangeTransferError::RangeUnsupported => (
            "NISAR_SUBSET_RANGE_UNSUPPORTED",
            "NISAR 源站未返回受支持的精确 Range 响应（包括 HTTP 200）。",
            false,
        ),
        Hdf5RangeTransferError::RangeNotSatisfiable => (
            "NISAR_SUBSET_RANGE_NOT_SATISFIABLE",
            "NISAR 源站拒绝了请求的物理 Range。",
            false,
        ),
        Hdf5RangeTransferError::ResponseInvalid => (
            "NISAR_SUBSET_RESPONSE_INVALID",
            "NISAR Range 响应的 ETag、Content-Range 或响应协议无效。",
            false,
        ),
        Hdf5RangeTransferError::MalformedContentRange => (
            "NISAR_SUBSET_MALFORMED_CONTENT_RANGE",
            "NISAR Range response Content-Range format or values do not match the request.",
            false,
        ),
        Hdf5RangeTransferError::PartialResponse => (
            "NISAR_SUBSET_PARTIAL_RESPONSE",
            "NISAR Range 响应未完整到达，未提升不完整产物。",
            true,
        ),
        Hdf5RangeTransferError::Transport => (
            "NISAR_SUBSET_TRANSPORT_FAILED",
            "NISAR Range 传输发生暂态网络错误，未回退到完整产品。",
            true,
        ),
        Hdf5RangeTransferError::RangeCountExceeded => (
            "NISAR_SUBSET_REQUEST_BUDGET_EXCEEDED",
            "NISAR 子集请求数将在超出受控网络预算前终止。",
            false,
        ),
        Hdf5RangeTransferError::RangeBytesExceeded => (
            "NISAR_SUBSET_BYTE_BUDGET_EXCEEDED",
            "NISAR 子集响应字节将在超出受控网络预算前终止。",
            false,
        ),
        Hdf5RangeTransferError::Persistence
        | Hdf5RangeTransferError::WorkspaceInvalid
        | Hdf5RangeTransferError::WorkspaceBusy
        | Hdf5RangeTransferError::RecoveryInvalid => (
            "NISAR_SUBSET_PERSISTENCE_FAILED",
            "NISAR 子集工作区或恢复证据无法持久化，未提升最终产物。",
            false,
        ),
        Hdf5RangeTransferError::WindowIdentityMismatch
        | Hdf5RangeTransferError::RangeOverlap
        | Hdf5RangeTransferError::ChunkIdentityDuplicate
        | Hdf5RangeTransferError::RangeInvalid
        | Hdf5RangeTransferError::RemoteByteTotalMismatch => (
            "NISAR_SUBSET_RANGE_INVALID",
            "NISAR 子集物理 Range 或窗口身份校验失败。",
            false,
        ),
    }
}

impl ProductionDownloadWorker {
    fn publish_transfer_update(
        &self,
        task_id: &str,
        item_id: &str,
        control: &WorkerControl,
        update: TransferUpdate,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) {
        let now = Instant::now();
        match update {
            TransferUpdate::BodyStarted {
                attempt,
                bytes_completed,
            } => {
                let _ = self.transfer_activity.body_started_at_bytes(
                    task_id,
                    item_id,
                    attempt,
                    bytes_completed,
                    now,
                );
            }
            TransferUpdate::Progress(value) => {
                let _ = self.transfer_activity.observe_bytes(
                    task_id,
                    item_id,
                    value.attempt,
                    value.bytes_completed,
                    now,
                );
                let bytes_per_second = self
                    .transfer_activity
                    .member_bytes_per_second(task_id, item_id, value.attempt)
                    .unwrap_or(0.0);
                progress(DownloadItemProgress {
                    task_id: task_id.to_owned(),
                    item_id: item_id.to_owned(),
                    status: if control.is_paused() {
                        DownloadItemStatus::Paused
                    } else {
                        DownloadItemStatus::Active
                    },
                    bytes_completed: value.bytes_completed,
                    total_bytes: value.total_bytes,
                    bytes_per_second,
                    attempt: u64::from(value.attempt),
                    updated_at: timestamp_now(),
                });
            }
            TransferUpdate::BodyStopped { attempt } => {
                let _ = self
                    .transfer_activity
                    .body_stopped(task_id, item_id, attempt, now);
            }
            TransferUpdate::StallReconnect { .. } => {
                let _ = self.transfer_activity.record_stall_reconnect(task_id, now);
            }
        }
    }

    fn resolver_transfer(&self, policy: crate::AuthRefreshPolicyId) -> &dyn TransferBackend {
        if policy == crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1 {
            if let Some(transfer) = self.usgs_transfer.as_deref() {
                return transfer;
            }
        }
        if matches!(
            policy,
            crate::AuthRefreshPolicyId::PlanetaryComputerSasV1
                | crate::AuthRefreshPolicyId::PublicAnonymousHttpsV1
        ) {
            return self.mpc_transfer.as_ref();
        }
        if policy == crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1 {
            if let Some(transfer) = self.bhoonidhi_transfer.as_deref() {
                return transfer;
            }
        }
        if policy == crate::AuthRefreshPolicyId::CdseOAuthBearerV1 {
            if let Some(transfer) = self.cdse_transfer.as_deref() {
                return transfer;
            }
        }
        self.transfer_backends.select()
    }

    fn acquire_destination_lease(
        &self,
        destination: &std::path::Path,
        control: &WorkerControl,
    ) -> Result<DestinationLeaseGuard, DestinationLeaseError> {
        self.destination_lease
            .acquire_for_worker(destination, control)
    }

    fn destination_lease_failure(
        work: &DownloadWork,
        destination: &std::path::Path,
        error: DestinationLeaseError,
    ) -> DownloadItemResult {
        match error {
            DestinationLeaseError::Cancelled => DownloadItemResult {
                task_id: work.task.task_id.clone(),
                item_id: work.item.item_id.clone(),
                outcome: DownloadOutcome::Cancelled,
                destination: destination.to_path_buf(),
                bytes_written: 0,
                attempt_count: 0,
                failure: None,
                finished_at: timestamp_now(),
            },
            DestinationLeaseError::InvalidIdentity
            | DestinationLeaseError::Unavailable
            | DestinationLeaseError::InvariantViolation => failure_result(
                work,
                destination.to_path_buf(),
                "DL005",
                "下载目标写入所有权不可用，未开始写入文件。",
                false,
            ),
        }
    }

    fn execute_legacy(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        store: Option<&NisarExecutionPlanFreezeAuthority>,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let burst_asset = is_burst_asset(&work);
        let provider = match DownloadProviderKind::from_provider_id(&work.task.provider_id) {
            Some(value) => value,
            None => {
                return failure_result(
                    &work,
                    work.task.output_root.clone(),
                    "DL005",
                    "该数据源暂不支持直接下载",
                    false,
                );
            }
        };
        let resolved = match provider {
            DownloadProviderKind::Orbit => {
                let requested_orbit_type = match work
                    .item
                    .payload
                    .get("requested_orbit_type")
                    .and_then(|v| v.as_str())
                {
                    Some("RESORB") => insar_core::OrbitProductType::RESORB,
                    _ => insar_core::OrbitProductType::POEORB,
                };
                self.orbit.resolve_with_orbit_type(
                    &work.item.item_id,
                    work.item.acquisition_datetime.as_deref(),
                    requested_orbit_type,
                    &|| control.is_cancelled(),
                )
            }
            DownloadProviderKind::AsfSentinel1 => match sentinel_resolution_plan(&work) {
                Ok(SentinelResolutionPlan::Frozen {
                    url,
                    file_name,
                    expected_size,
                }) => resolve_frozen_asf_asset(&work.item.item_id, &url, &file_name, expected_size),
                Ok(SentinelResolutionPlan::Metadata {
                    file_name,
                    product_type,
                }) => resolve_asf_download(
                    &self.asf,
                    &work.item.item_id,
                    &file_name,
                    &product_type,
                    &|| control.is_cancelled(),
                ),
                Err(error) => {
                    return failure_result(
                        &work,
                        work.task.output_root.clone(),
                        error.code,
                        error.reason_zh,
                        false,
                    );
                }
            },
            DownloadProviderKind::AsfCatalog => {
                if is_seasat_work(&work) {
                    match seasat_resolution_plan(&work) {
                        Ok(plan) => resolve_frozen_asf_asset(
                            &work.item.item_id,
                            &plan.url,
                            &plan.file_name,
                            plan.expected_size,
                        ),
                        Err(error) => {
                            return failure_result(
                                &work,
                                work.task.output_root.clone(),
                                error.code,
                                error.reason_zh,
                                false,
                            );
                        }
                    }
                } else {
                    let file_name = if burst_asset {
                        match work
                            .item
                            .payload
                            .get("file_name")
                            .and_then(serde_json::Value::as_str)
                            .filter(|value| !value.trim().is_empty())
                        {
                            Some(value) => value.to_string(),
                            None => {
                                return failure_result(
                                    &work,
                                    work.task.output_root.clone(),
                                    "ASF_BURST_ASSET_INVALID",
                                    "Burst 任务缺少冻结的官方资产文件名，未回退到场景名。",
                                    false,
                                );
                            }
                        }
                    } else {
                        frozen_file_name(&work)
                    };
                    let Some(url) = work
                        .item
                        .payload
                        .get("download_url")
                        .and_then(serde_json::Value::as_str)
                    else {
                        return failure_result(
                            &work,
                            work.task.output_root.clone(),
                            "DL004",
                            "ASF 产品快照缺少下载地址，请重新检索。",
                            false,
                        );
                    };
                    let expected_size = work
                        .item
                        .payload
                        .get("file_size_remote")
                        .and_then(serde_json::Value::as_u64);
                    resolve_frozen_asf_asset(&work.item.item_id, url, &file_name, expected_size)
                }
            }
        };
        let resolved = match resolved {
            Ok(value) => value,
            Err(error) => {
                return failure_result(
                    &work,
                    work.task.output_root.clone(),
                    &error.code,
                    &error.user_message,
                    error.code != "DL003" && error.code != "DL004",
                );
            }
        };
        self.transfer_resolved(work, resolved, provider, control, store, progress)
    }
}

impl ProductionDownloadWorker {
    fn execute_resolver_authoritative(
        &self,
        work: DownloadWork,
        eligible: &crate::EligibleResolverAsset,
        control: WorkerControl,
        store: Option<&NisarExecutionPlanFreezeAuthority>,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let Some(file_name) = eligible.frozen_access.file_name.as_deref() else {
            return failure_result(
                &work,
                work.task.output_root.clone(),
                "DL005",
                "冻结的 Resolver 资产缺少文件名，未回退到 Legacy 下载。",
                false,
            );
        };
        let target = plan_work_target(&work, file_name);
        let destination = match target {
            Ok(destination) => destination,
            Err(error) => {
                return failure_result(
                    &work,
                    work.task.output_root.clone(),
                    error.code(),
                    error.reason_zh(),
                    false,
                );
            }
        };
        let _destination_lease = match self.acquire_destination_lease(&destination, &control) {
            Ok(guard) => guard,
            Err(error) => return Self::destination_lease_failure(&work, &destination, error),
        };
        // Downloader D3: an existing planned file is decided before the
        // session, resolution, credentials and network admission.
        let frozen_checksum = frozen_resolver_checksum(&eligible.frozen_access);
        let gate_request = GateRequest {
            work: &work,
            destination: &destination,
            file_name,
            authoritative_size: authoritative_reuse_size(eligible),
            checksum: frozen_checksum.as_ref(),
            reuse_needs_validation: eligible.has_artifact_validation_plan(),
            store,
            control: &control,
        };
        let passage = match pass_local_reuse_gate(&gate_request) {
            Ok(passage) => passage,
            Err(result) => return *result,
        };
        if !eligible.has_artifact_validation_plan() {
            match complete_local_resolver_artifact(&work, eligible, &destination) {
                Ok(Some(result)) => {
                    if result.outcome == DownloadOutcome::Completed {
                        record_local_landing(&gate_request, passage, None);
                    }
                    return result;
                }
                Ok(None) => {}
                Err(error) => {
                    return failure_result(
                        &work,
                        destination,
                        error.code(),
                        error.reason_zh(),
                        false,
                    );
                }
            }
        }
        let task_id = work.task.task_id.clone();
        let item_id = work.item.item_id.clone();
        let transfer = self.resolver_transfer(eligible.policy.auth_refresh_policy_id);
        let admission = if eligible.policy.auth_refresh_policy_id
            == crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1
        {
            Arc::clone(&self.bhoonidhi_admission)
        } else {
            Arc::clone(&self.admission)
        };
        let admitted = AdmittedTransferBackend::new(transfer, admission);
        let result = self.resolver_runtime.execute_resolver_authoritative(
            &admitted,
            eligible,
            &task_id,
            &item_id,
            destination.clone(),
            &|| control.is_cancelled(),
            &|| control.is_paused(),
            &mut |update| {
                self.publish_transfer_update(&task_id, &item_id, &control, update, progress);
            },
        );
        match result {
            Ok(result) => {
                let outcome =
                    if result.outcome == DownloadOutcome::Interrupted && control.is_cancelled() {
                        DownloadOutcome::Cancelled
                    } else {
                        result.outcome
                    };
                if outcome == DownloadOutcome::Completed {
                    record_local_landing(&gate_request, passage, None);
                } else if outcome == DownloadOutcome::Skipped {
                    record_deferred_reuse(&gate_request, passage, result.bytes_written);
                }
                DownloadItemResult {
                    task_id,
                    item_id,
                    outcome,
                    destination,
                    bytes_written: result.bytes_written,
                    attempt_count: u64::from(result.attempt_count),
                    failure: result.failure,
                    finished_at: timestamp_now(),
                }
            }
            Err(error) => DownloadItemResult {
                task_id,
                item_id,
                outcome: if error.code() == "DL001" {
                    if control.is_cancelled() {
                        DownloadOutcome::Cancelled
                    } else {
                        DownloadOutcome::Interrupted
                    }
                } else {
                    DownloadOutcome::Failed
                },
                destination,
                bytes_written: 0,
                attempt_count: 0,
                failure: Some(DownloadFailure {
                    code: error.code().to_string(),
                    reason_zh: error.diagnostic_reason(),
                    retryable: error.retryable(),
                }),
                finished_at: timestamp_now(),
            },
        }
    }

    fn resolve_standard_earthdata_auth(
        &self,
    ) -> Result<(TransferAuth, ShadowAuthKind), crate::EarthdataCredentialError> {
        #[cfg(test)]
        if self.bypass_earthdata_auth_for_test {
            return Ok((TransferAuth::None, ShadowAuthKind::None));
        }
        resolve_earthdata_credential(EarthdataCredentialSource::Auto, None).map(|value| {
            let kind = value.shadow_auth_kind();
            (value, kind)
        })
    }

    fn transfer_resolved(
        &self,
        work: DownloadWork,
        resolved: ResolvedDownloadSource,
        provider: DownloadProviderKind,
        control: WorkerControl,
        store: Option<&NisarExecutionPlanFreezeAuthority>,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let burst_asset = is_burst_asset(&work);
        let target = plan_work_target(&work, &resolved.file_name);
        let destination = match target {
            Ok(value) => value,
            Err(error) => {
                return failure_result(
                    &work,
                    work.task.output_root.clone(),
                    error.code(),
                    error.reason_zh(),
                    false,
                );
            }
        };
        let _destination_lease = match self.acquire_destination_lease(&destination, &control) {
            Ok(guard) => guard,
            Err(error) => return Self::destination_lease_failure(&work, &destination, error),
        };
        // Downloader D3: an existing planned file is decided before
        // credentials, sessions and network admission.
        let gate_request = GateRequest {
            work: &work,
            destination: &destination,
            file_name: &resolved.file_name,
            authoritative_size: resolved.expected_size,
            checksum: None,
            reuse_needs_validation: false,
            store,
            control: &control,
        };
        let passage = match pass_local_reuse_gate(&gate_request) {
            Ok(passage) => passage,
            Err(result) => return *result,
        };
        let request = match TransferRequest::new(
            work.task.task_id.clone(),
            work.item.item_id.clone(),
            resolved.url(),
            destination.clone(),
            resolved.expected_size,
        ) {
            Ok(value) => value,
            Err(_) => {
                return failure_result(
                    &work,
                    destination,
                    "DL005",
                    "Rust 下载请求无效，请重新检索后重试",
                    false,
                );
            }
        };
        #[cfg(test)]
        let bypass_burst_auth_for_test = burst_asset && self.bypass_earthdata_auth_for_test;
        #[cfg(not(test))]
        let bypass_burst_auth_for_test = false;
        let (auth, legacy_auth_kind) = match provider {
            DownloadProviderKind::Orbit => (TransferAuth::None, ShadowAuthKind::None),
            DownloadProviderKind::AsfCatalog if bypass_burst_auth_for_test => {
                (TransferAuth::None, ShadowAuthKind::EarthdataSessionCookie)
            }
            DownloadProviderKind::AsfCatalog if burst_asset => {
                let credential = match resolve_earthdata_burst_credential(None) {
                    Ok(value) => value,
                    Err(error) => {
                        return failure_result(
                            &work,
                            destination,
                            error.code(),
                            error.reason_zh(),
                            false,
                        );
                    }
                };
                let mut ready = match self.burst_session_ready.lock() {
                    Ok(value) => value,
                    Err(_) => {
                        return failure_result(
                            &work,
                            destination,
                            "DL005",
                            "Burst 下载会话状态不可用，请重启应用后重试。",
                            true,
                        );
                    }
                };
                if !*ready {
                    if let Err(error) = self
                        .transfer
                        .establish_burst_session(&credential, &BurstAuthEndpoints::official())
                    {
                        return failure_result(
                            &work,
                            destination,
                            error.code(),
                            error.reason_zh(),
                            error.code() == "DL005",
                        );
                    }
                    *ready = true;
                }
                (TransferAuth::None, ShadowAuthKind::EarthdataSessionCookie)
            }
            DownloadProviderKind::AsfSentinel1 | DownloadProviderKind::AsfCatalog => {
                match self.resolve_standard_earthdata_auth() {
                    Ok(value) => value,
                    Err(error) => {
                        return failure_result(
                            &work,
                            destination,
                            error.code(),
                            error.reason_zh(),
                            false,
                        );
                    }
                }
            }
        };
        let task_id = work.task.task_id.clone();
        let item_id = work.item.item_id.clone();
        let shadow_input = build_shadow_input(
            &work,
            &resolved,
            &destination,
            legacy_auth_kind,
            provider,
            burst_asset,
            control.is_cancelled(),
        );
        let shadow_observer = self
            .shadow
            .as_deref()
            .map(|runtime| runtime as &dyn ResolverShadowObserver);
        let transfer = self.transfer_backends.select();
        let admitted = AdmittedTransferBackend::new(transfer, Arc::clone(&self.admission));
        let mut result = run_shadow_then_legacy(shadow_observer, shadow_input.as_ref(), || {
            admitted.download_legacy(
                &request,
                &auth,
                &|| control.is_cancelled(),
                &|| control.is_paused(),
                &mut |update| {
                    self.publish_transfer_update(&task_id, &item_id, &control, update, progress);
                },
            )
        });
        if burst_asset
            && result
                .failure
                .as_ref()
                .is_some_and(|failure| failure.code == "DL004")
        {
            if let Ok(mut ready) = self.burst_session_ready.lock() {
                *ready = false;
            }
            result.failure = Some(DownloadFailure {
                code: "ASF_BURST_COOKIE_REQUIRED".to_string(),
                reason_zh: "Burst 下载会话 Cookie 已失效或被拒绝，请检查账号密码或 .netrc 后重试。"
                    .to_string(),
                retryable: false,
            });
        }
        classify_asf_dataset_access_restriction(&work, &mut result.failure);
        let outcome = if result.outcome == DownloadOutcome::Interrupted && control.is_cancelled() {
            DownloadOutcome::Cancelled
        } else {
            result.outcome
        };
        let landed_sha256 = self.transfer.take_landed_sha256(&destination);
        if outcome == DownloadOutcome::Completed {
            record_local_landing(&gate_request, passage, landed_sha256);
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome,
            destination,
            bytes_written: result.bytes_written,
            attempt_count: u64::from(result.attempt_count),
            failure: result.failure,
            finished_at: timestamp_now(),
        }
    }
}

/// The frozen Provider checksum of a resolver asset, when it is well formed.
fn frozen_resolver_checksum(access: &insar_core::FrozenAssetAccess) -> Option<ExpectedChecksum> {
    match (
        access.checksum_algorithm.as_deref(),
        access.checksum_value.as_deref(),
    ) {
        (Some(algorithm), Some(value)) => ExpectedChecksum::new(algorithm, value).ok(),
        _ => None,
    }
}

fn complete_local_resolver_artifact(
    work: &DownloadWork,
    eligible: &crate::EligibleResolverAsset,
    destination: &std::path::Path,
) -> Result<Option<DownloadItemResult>, DownloadFileError> {
    let access = &eligible.frozen_access;
    let checksum = match (
        access.checksum_algorithm.as_deref(),
        access.checksum_value.as_deref(),
    ) {
        (Some(algorithm), Some(value)) => Some(
            ExpectedChecksum::new(algorithm, value)
                .map_err(|_| DownloadFileError::IntegrityMismatch)?,
        ),
        (None, None) => None,
        _ => return Err(DownloadFileError::IntegrityMismatch),
    };
    if checksum.is_none() {
        return Ok(None);
    }
    let preparation = inspect_resolved_destination(
        destination,
        access.expected_size_bytes,
        checksum.as_ref(),
        ResolvedPartContext::RecoveredAfterRestart,
    )?;
    let (outcome, bytes) = match preparation {
        FilePreparation::SkipComplete { bytes } => (DownloadOutcome::Skipped, bytes),
        FilePreparation::FinalizePart { bytes } => {
            finalize_part(destination, access.expected_size_bytes)?;
            (DownloadOutcome::Completed, bytes)
        }
        FilePreparation::Resume { .. } | FilePreparation::Restart => return Ok(None),
    };
    Ok(Some(DownloadItemResult {
        task_id: work.task.task_id.clone(),
        item_id: work.item.item_id.clone(),
        outcome,
        destination: destination.to_path_buf(),
        bytes_written: bytes,
        attempt_count: 0,
        failure: None,
        finished_at: timestamp_now(),
    }))
}

fn invalid_burst_target(reason: &str) -> crate::DownloadFileError {
    insar_core::ContractError::new("V3_INVALID_TARGET_PATH", reason, serde_json::Map::new())
        .map(crate::DownloadFileError::InvalidTarget)
        .unwrap_or(crate::DownloadFileError::InvalidPreparation)
}

fn plan_work_target(
    work: &DownloadWork,
    file_name: &str,
) -> Result<PathBuf, crate::DownloadFileError> {
    if is_burst_asset(work) {
        // ASF names Burst assets by burst index inside the parent SAFE
        // (`.../IW3/VV/0.tiff`), so different bursts share a basename. The
        // burst product ID names one folder per burst; the basename is kept.
        // Requiring the "-BURST" suffix of the CMR native ID also keeps
        // Windows device names such as CON out of the folder name.
        let directory = work
            .item
            .metadata
            .get("product_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| {
                value.len() <= 128
                    && value.ends_with("-BURST")
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            })
            .ok_or_else(|| invalid_burst_target("Burst 产品 ID 不是可用的目录名"))?;
        if work.task.layout != DownloadLayout::Direct {
            return Err(invalid_burst_target("Burst 资产只支持直接目录布局"));
        }
        return plan_download_target(
            &work.task.output_root.join(directory),
            &DownloadLayout::Direct,
            file_name,
        );
    }
    if work
        .task
        .query
        .get("delivery_kind")
        .and_then(serde_json::Value::as_str)
        == Some("sentinel2_l2a_analysis_band_batch")
    {
        let invalid = || crate::DownloadFileError::InvalidPreparation;
        let directory = work
            .item
            .metadata
            .get("delivery_product_directory")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(invalid)?;
        let access = work.item.asset_access.as_ref().ok_or_else(invalid)?;
        if work.task.layout != DownloadLayout::Direct
            || work.task.provider_id != "microsoft.planetary-computer"
            || access.reference.provider_id != work.task.provider_id
            || access.reference.collection_id != "sentinel-2-l2a"
            || directory != access.reference.provider_product_id
            || directory.is_empty()
            || directory.len() > 128
            || !directory
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || access.file_name.as_deref() != Some(file_name)
        {
            return Err(invalid());
        }
        return plan_download_target(
            &work.task.output_root.join(directory),
            &DownloadLayout::Direct,
            file_name,
        );
    }
    if work.task.layout == DownloadLayout::SarDatasetProcessing {
        let dataset = work
            .item
            .metadata
            .get("dataset")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let processing_level = work
            .item
            .metadata
            .get("processing_level")
            .or_else(|| work.item.metadata.get("product_family"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        plan_sar_catalog_target(&work.task.output_root, dataset, processing_level, file_name)
    } else {
        plan_download_target(&work.task.output_root, &work.task.layout, file_name)
    }
}

#[cfg(test)]
#[path = "production_worker_sentinel2_batch_tests.rs"]
mod sentinel2_batch_tests;

#[cfg(test)]
#[path = "production_worker_burst_target_tests.rs"]
mod burst_target_tests;

#[cfg(test)]
#[path = "production_worker_burst_reuse_tests.rs"]
mod burst_reuse_tests;

fn build_shadow_input(
    work: &DownloadWork,
    resolved: &ResolvedDownloadSource,
    destination: &std::path::Path,
    auth_kind: ShadowAuthKind,
    provider: DownloadProviderKind,
    burst_asset: bool,
    cancelled: bool,
) -> Option<ResolverShadowInput> {
    let frozen = work.item.asset_access.clone();
    let asset_role = frozen
        .as_ref()
        .map(|access| access.role.clone())
        .unwrap_or_else(|| legacy_asset_role(work));
    let (checksum_algorithm, checksum_value) = frozen
        .as_ref()
        .map(|access| {
            (
                access.checksum_algorithm.clone(),
                access.checksum_value.clone(),
            )
        })
        .unwrap_or((None, None));
    let legacy = TransferIntentFingerprint::from_legacy(&LegacyTransferIntent {
        provider_id: work.task.provider_id.clone(),
        url: resolved.url().to_string(),
        file_name: resolved.file_name.clone(),
        auth_kind,
        expected_size: resolved.expected_size,
        checksum_algorithm,
        checksum_value,
        destination: destination.to_path_buf(),
        destination_layout: work.task.layout.clone(),
        asset_role,
    })
    .ok()?;
    let dataset = work
        .item
        .metadata
        .get("dataset")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let processing_level = work
        .item
        .metadata
        .get("processing_level")
        .or_else(|| work.item.metadata.get("product_family"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Some(
        ResolverShadowInput::new(
            shadow_asset_class(work, provider, burst_asset, frozen.is_some()),
            frozen,
            legacy,
            // This policy has no per-Burst folder. Burst items freeze no
            // asset_access, so the shadow records them as ineligible before
            // any destination comparison.
            ShadowDestinationPolicy::new(
                work.task.output_root.clone(),
                work.task.layout.clone(),
                dataset,
                processing_level,
            ),
        )
        .with_cancelled(cancelled),
    )
}

fn shadow_asset_class(
    work: &DownloadWork,
    provider: DownloadProviderKind,
    burst_asset: bool,
    has_frozen_access: bool,
) -> ShadowAssetClass {
    if provider == DownloadProviderKind::Orbit {
        return if has_frozen_access {
            ShadowAssetClass::OrbitDirectEof
        } else {
            ShadowAssetClass::OrbitSceneId
        };
    }
    if burst_asset {
        return if legacy_asset_role(work) == AssetRole::Metadata {
            ShadowAssetClass::BurstXml
        } else {
            ShadowAssetClass::BurstTiff
        };
    }
    if !has_frozen_access {
        return ShadowAssetClass::LegacyUrl;
    }
    if work
        .item
        .metadata
        .get("dataset")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SENTINEL-1"))
    {
        ShadowAssetClass::Sentinel1Standard
    } else {
        ShadowAssetClass::HistoricalAsf
    }
}

fn legacy_asset_role(work: &DownloadWork) -> AssetRole {
    match work
        .item
        .metadata
        .get("asset_role")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("primary")
        .to_ascii_lowercase()
        .as_str()
    {
        "metadata" => AssetRole::Metadata,
        "thumbnail" => AssetRole::Thumbnail,
        "browse" => AssetRole::Browse,
        "auxiliary" => AssetRole::Auxiliary,
        "derived" => AssetRole::Derived,
        _ => AssetRole::Primary,
    }
}

fn is_burst_asset(work: &DownloadWork) -> bool {
    work.task.provider_id == "asf.catalog"
        && work
            .item
            .metadata
            .get("dataset")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("SLC-BURST"))
        && work
            .item
            .payload
            .get("burst_asset")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DownloadProviderKind {
    AsfSentinel1,
    AsfCatalog,
    Orbit,
}

impl DownloadProviderKind {
    fn from_provider_id(provider_id: &str) -> Option<Self> {
        match provider_id {
            "asf.sentinel1" => Some(Self::AsfSentinel1),
            "asf.catalog" => Some(Self::AsfCatalog),
            "esa.sentinel1_orbit" | "sentinel1.orbit" => Some(Self::Orbit),
            _ => None,
        }
    }
}

const ASF_DATASET_ACCESS_RESTRICTED: &str = "ASF_DATASET_ACCESS_RESTRICTED";
const JERS_RESTRICTED_REASON_ZH: &str =
    "JERS-1 是受限数据集。有关如何访问这些数据的信息，请联系 ASF 用户支持。";
/// ASF removed ERS, RADARSAT-1 and JERS-1 from its searchable interfaces
/// (Owner evidence 2026-10-31); their Registry resources are retired.
const ASF_REMOVED_LEGACY_SAR_DATASETS: &[&str] = &["ERS", "JERS-1", "RADARSAT-1"];
const ASF_REMOVED_REASON_ZH: &str = "ASF 已从检索接口下线该数据集";

fn classify_asf_dataset_access_restriction(
    work: &DownloadWork,
    failure: &mut Option<DownloadFailure>,
) {
    let Some(dataset) = work
        .item
        .metadata
        .get("dataset")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
    else {
        return;
    };
    if failure.as_ref().is_none_or(|value| value.code != "DL004") {
        return;
    }
    let reason_zh = if ASF_REMOVED_LEGACY_SAR_DATASETS
        .iter()
        .any(|selector| dataset.eq_ignore_ascii_case(selector))
    {
        ASF_REMOVED_REASON_ZH
    } else if dataset.eq_ignore_ascii_case("JERS-1") {
        JERS_RESTRICTED_REASON_ZH
    } else {
        return;
    };
    *failure = Some(DownloadFailure {
        code: ASF_DATASET_ACCESS_RESTRICTED.to_string(),
        reason_zh: reason_zh.to_string(),
        retryable: false,
    });
}

fn frozen_file_name(work: &DownloadWork) -> String {
    for key in ["expected_filename", "file_name"] {
        if let Some(value) = work
            .item
            .payload
            .get(key)
            .and_then(serde_json::Value::as_str)
        {
            return value.to_string();
        }
    }
    if work
        .item
        .display_name
        .to_ascii_lowercase()
        .ends_with(".zip")
    {
        work.item.display_name.clone()
    } else {
        format!("{}.zip", work.item.item_id)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SeasatResolutionPlan {
    url: String,
    file_name: String,
    expected_size: Option<u64>,
    md5: String,
}

#[derive(Debug, PartialEq, Eq)]
struct SeasatResolutionError {
    code: &'static str,
    reason_zh: &'static str,
}

fn is_seasat_work(work: &DownloadWork) -> bool {
    work.task.provider_id == "asf.catalog"
        && work
            .item
            .metadata
            .get("dataset")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SEASAT"))
}

fn is_hdf5_file_name(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.ends_with(".h5") || lower.ends_with(".hdf5")
}

fn seasat_resolution_plan(
    work: &DownloadWork,
) -> Result<SeasatResolutionPlan, SeasatResolutionError> {
    let assets_value = work
        .item
        .metadata
        .get("assets")
        .ok_or(SeasatResolutionError {
            code: "ASF_PRIMARY_ASSET_MISSING",
            reason_zh: "SEASAT 冻结结果缺少 HDF5 主数据资产，未回退到 GeoTIFF。",
        })?;
    let assets: Vec<ProviderAsset> =
        serde_json::from_value(assets_value.clone()).map_err(|_| SeasatResolutionError {
            code: "ASF_PRIMARY_ASSET_INVALID",
            reason_zh: "SEASAT 冻结资产图无效，未开始下载。",
        })?;
    let primary_assets = assets
        .iter()
        .filter(|asset| asset.role == AssetRole::Primary)
        .collect::<Vec<_>>();
    let primary = match primary_assets.as_slice() {
        [] => {
            return Err(SeasatResolutionError {
                code: "ASF_PRIMARY_ASSET_MISSING",
                reason_zh: "SEASAT 冻结结果缺少 HDF5 主数据资产，未回退到 GeoTIFF。",
            });
        }
        [primary] => *primary,
        _ => {
            return Err(SeasatResolutionError {
                code: "ASF_PRIMARY_ASSET_AMBIGUOUS",
                reason_zh: "SEASAT 冻结结果包含多个主数据资产，未开始下载。",
            });
        }
    };

    let top_file_name = work
        .item
        .payload
        .get("file_name")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let top_url = work
        .item
        .payload
        .get("download_url")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let primary_file_name = primary.file_name.as_deref();
    let primary_url = primary.url.as_deref();
    if !top_file_name.is_some_and(is_hdf5_file_name)
        || !primary_file_name.is_some_and(is_hdf5_file_name)
        || top_file_name != primary_file_name
        || top_url != primary_url
    {
        return Err(SeasatResolutionError {
            code: "ASF_PRIMARY_ASSET_INVALID",
            reason_zh: "SEASAT 顶层下载资产与唯一 HDF5 主数据不一致，未开始下载。",
        });
    }

    let expected_size = work
        .item
        .payload
        .get("file_size_remote")
        .and_then(serde_json::Value::as_u64);
    let primary_size = primary
        .size_bytes
        .and_then(|value| u64::try_from(value).ok());
    if expected_size.is_none() || expected_size != primary_size {
        return Err(SeasatResolutionError {
            code: "ASF_PRIMARY_ASSET_INVALID",
            reason_zh: "SEASAT HDF5 主数据的冻结大小证据缺失或不一致，未开始下载。",
        });
    }

    let md5 = work
        .item
        .metadata
        .get("checksum")
        .and_then(serde_json::Value::as_str)
        .filter(|value| ExpectedChecksum::new("md5", value).is_ok())
        .ok_or(SeasatResolutionError {
            code: "ASF_PRIMARY_ASSET_INVALID",
            reason_zh: "SEASAT HDF5 主数据缺少有效的冻结 MD5 证据，未开始下载。",
        })?;
    Ok(SeasatResolutionPlan {
        url: top_url.expect("validated SEASAT URL").to_string(),
        file_name: top_file_name
            .expect("validated SEASAT filename")
            .to_string(),
        expected_size,
        md5: md5.to_ascii_lowercase(),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum SentinelResolutionPlan {
    Frozen {
        url: String,
        file_name: String,
        expected_size: Option<u64>,
    },
    Metadata {
        file_name: String,
        product_type: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
struct SentinelResolutionError {
    code: &'static str,
    reason_zh: &'static str,
}

fn sentinel_resolution_plan(
    work: &DownloadWork,
) -> Result<SentinelResolutionPlan, SentinelResolutionError> {
    let file_name = frozen_file_name(work);
    if let Some(url) = work
        .item
        .payload
        .get("download_url")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(SentinelResolutionPlan::Frozen {
            url: url.to_string(),
            file_name,
            expected_size: work
                .item
                .payload
                .get("file_size_remote")
                .and_then(serde_json::Value::as_u64),
        });
    }
    let Some(product_type) = sentinel_compatibility_product_type(work) else {
        return Err(SentinelResolutionError {
            code: "DL005",
            reason_zh: "旧版 Sentinel-1 任务快照缺少明确产品类型，请重新检索后再下载。",
        });
    };
    Ok(SentinelResolutionPlan::Metadata {
        file_name,
        product_type,
    })
}

fn sentinel_compatibility_product_type(work: &DownloadWork) -> Option<String> {
    for key in ["processing_level", "product_type"] {
        if let Some(value) = work
            .item
            .metadata
            .get(key)
            .and_then(serde_json::Value::as_str)
            .and_then(normalize_sentinel_product_type)
        {
            return Some(value);
        }
    }
    if let Some(value) = work
        .task
        .query
        .get("product_type")
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_sentinel_product_type)
    {
        return Some(value);
    }
    let levels = work
        .task
        .query
        .get("processing_levels")
        .and_then(serde_json::Value::as_array)?;
    let mut normalized = Vec::new();
    for value in levels {
        let raw = value.as_str()?.trim();
        if raw.is_empty() {
            continue;
        }
        let product_type = normalize_sentinel_product_type(raw)?;
        if !normalized.contains(&product_type) {
            normalized.push(product_type);
        }
    }
    (normalized.len() == 1).then(|| normalized.remove(0))
}

fn normalize_sentinel_product_type(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_uppercase();
    let family = if normalized.starts_with("GRD_") {
        "GRD"
    } else {
        normalized.as_str()
    };
    matches!(family, "SLC" | "GRD" | "RAW" | "OCN").then(|| family.to_string())
}

#[cfg(feature = "nisar-subset-production")]
pub(crate) fn nisar_subset_v2_recovery_workspace_root(work: &DownloadWork) -> PathBuf {
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

#[cfg(feature = "nisar-subset-production")]
fn nisar_v2_resolver_failure(
    error: &crate::ResolverExecutionError,
) -> (&'static str, String, bool) {
    (error.code(), error.diagnostic_reason(), error.retryable())
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_v2_resolver_error_is_lifecycle_stop(error: &crate::ResolverExecutionError) -> bool {
    matches!(error, crate::ResolverExecutionError::Cancelled)
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_v2_science_auth_refresh_failure(
    error: &crate::ResolverExecutionError,
    control: &WorkerControl,
) -> (
    crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2,
    Option<&'static str>,
) {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as ScienceError;

    if matches!(error, crate::ResolverExecutionError::Cancelled) {
        let stopped = if control.is_paused() && !control.is_cancelled() {
            ScienceError::Paused
        } else {
            ScienceError::Cancelled
        };
        return (stopped, None);
    }

    // The control flag may race with a genuine provider/auth failure. Only the
    // resolver's typed Cancelled variant is lifecycle control; every other
    // error must keep its allowlisted upstream diagnostic. A transient refresh
    // grant or post-refresh probe failure may be retried (design 2026-10-17 §4).
    let mapped = if error.bhoonidhi_auth_refresh_is_transient() {
        ScienceError::AuthRefreshTransient
    } else {
        ScienceError::AuthRefreshFailed
    };
    (mapped, error.bhoonidhi_auth_diagnostic())
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_v2_stopped_result(
    work: &DownloadWork,
    destination: PathBuf,
    control: &WorkerControl,
) -> DownloadItemResult {
    let cancelled = control.is_cancelled();
    DownloadItemResult {
        task_id: work.task.task_id.clone(),
        item_id: work.item.item_id.clone(),
        outcome: if cancelled {
            DownloadOutcome::Cancelled
        } else {
            DownloadOutcome::Interrupted
        },
        destination,
        bytes_written: 0,
        attempt_count: 1,
        // A cooperative pause/cancel is a task lifecycle transition, not a
        // provider or transport failure. Persisting a synthetic DownloadFailure
        // here makes the task history look like an upstream channel error and
        // can overwrite the user's view of the real provider diagnostic.
        failure: None,
        finished_at: timestamp_now(),
    }
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_v2_planning_failure(
    work: &DownloadWork,
    destination: PathBuf,
    error: crate::NisarSubsetPlanningErrorV2,
    observation: Option<&crate::nisar_planning_v2::NisarSubsetRangeFailureObservationV2>,
    control: &WorkerControl,
) -> DownloadItemResult {
    nisar_v2_planning_failure_after(
        work,
        destination,
        error,
        observation.map_or(&[], std::slice::from_ref),
        control,
    )
}

/// The planning failure after the Range failure observations of this
/// execution; the last one is described. A failure of a retried planning Range
/// also names its attempts, as a science Range failure does (downloader
/// enhancement D1c: the preflight; D1d: metadata prefix, metadata discovery and
/// additional geolocation, each retried up to three times).
#[cfg(feature = "nisar-subset-production")]
fn nisar_v2_planning_failure_after(
    work: &DownloadWork,
    destination: PathBuf,
    error: crate::NisarSubsetPlanningErrorV2,
    observations: &[crate::nisar_planning_v2::NisarSubsetRangeFailureObservationV2],
    control: &WorkerControl,
) -> DownloadItemResult {
    let observation = observations.last();
    if error == crate::NisarSubsetPlanningErrorV2::ExecutionStopped {
        return nisar_v2_stopped_result(work, destination, control);
    }
    // Issue #8 slice 6: an output refused before the freeze is a local result.
    if let Some(refusal) = crate::nisar_subset_stream_v2::v2_output_refusal_of(error) {
        let failure = crate::nisar_output_diagnostics_v2::nisar_v2_output_refusal(
            "planning_admission",
            refusal,
            "计划未冻结，没有发起科学数据请求；",
        );
        return failure_result(
            work,
            destination,
            failure.code,
            &failure.reason_zh,
            failure.retryable,
        );
    }
    let retryable = matches!(
        error,
        crate::NisarSubsetPlanningErrorV2::BootstrapPartialResponse
            | crate::NisarSubsetPlanningErrorV2::BootstrapTransport
    );
    let code = match error {
        crate::NisarSubsetPlanningErrorV2::DeliveryKindInvalid => {
            "NISAR_SUBSET_V2_DELIVERY_KIND_INVALID"
        }
        crate::NisarSubsetPlanningErrorV2::PreparedIntentInvalid
        | crate::NisarSubsetPlanningErrorV2::PreparedIntentChanged => {
            "NISAR_SUBSET_V2_PREPARED_INTENT_INVALID"
        }
        crate::NisarSubsetPlanningErrorV2::FrozenPlanMissing
        | crate::NisarSubsetPlanningErrorV2::FrozenPlanInvalid
        | crate::NisarSubsetPlanningErrorV2::FinalPlanInvalid => {
            "NISAR_SUBSET_V2_FROZEN_PLAN_INVALID"
        }
        crate::NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing => {
            "NISAR_SUBSET_V2_PLANNING_WORKSPACE_MISSING"
        }
        crate::NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch => {
            "NISAR_SUBSET_V2_PLANNING_WORKSPACE_IDENTITY_MISMATCH"
        }
        crate::NisarSubsetPlanningErrorV2::RecoveryInvalid => "NISAR_SUBSET_V2_RECOVERY_INVALID",
        crate::NisarSubsetPlanningErrorV2::BootstrapFailed => "NISAR_SUBSET_V2_BOOTSTRAP_FAILED",
        crate::NisarSubsetPlanningErrorV2::BootstrapSourceUnavailable => {
            "NISAR_SUBSET_V2_SOURCE_UNAVAILABLE"
        }
        crate::NisarSubsetPlanningErrorV2::BootstrapRangeUnsupported => {
            "NISAR_SUBSET_V2_RANGE_UNSUPPORTED"
        }
        crate::NisarSubsetPlanningErrorV2::BootstrapResponseInvalid => {
            "NISAR_SUBSET_V2_RESPONSE_INVALID"
        }
        crate::NisarSubsetPlanningErrorV2::BootstrapPartialResponse => {
            "NISAR_SUBSET_V2_PARTIAL_RESPONSE"
        }
        crate::NisarSubsetPlanningErrorV2::BootstrapTransport => "NISAR_SUBSET_V2_TRANSPORT_FAILED",
        crate::NisarSubsetPlanningErrorV2::BootstrapStrongValidatorRequired => {
            "NISAR_SUBSET_V2_STRONG_VALIDATOR_REQUIRED"
        }
        crate::NisarSubsetPlanningErrorV2::BootstrapRangeNotSatisfiable => {
            "NISAR_SUBSET_V2_RANGE_NOT_SATISFIABLE"
        }
        crate::NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid => {
            "NISAR_SUBSET_V2_BOOTSTRAP_STATISTICS_INVALID"
        }
        crate::NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded
        | crate::NisarSubsetPlanningErrorV2::CombinedBudgetExceeded => {
            "NISAR_SUBSET_V2_COMBINED_BUDGET_EXCEEDED"
        }
        crate::NisarSubsetPlanningErrorV2::ScienceTransferObserved => {
            "NISAR_SUBSET_V2_PLANNING_SCIENCE_OVERLAP"
        }
        crate::NisarSubsetPlanningErrorV2::InventoryBindingInvalid
        | crate::NisarSubsetPlanningErrorV2::InventoryMismatch => {
            "NISAR_SUBSET_V2_INVENTORY_INVALID"
        }
        crate::NisarSubsetPlanningErrorV2::SelectedScienceMissing => {
            "NISAR_SUBSET_V2_SELECTED_SCIENCE_MISSING"
        }
        crate::NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable => {
            "NISAR_SUBSET_V2_PHYSICAL_MAPPING_UNAVAILABLE"
        }
        crate::NisarSubsetPlanningErrorV2::PhysicalPlanInvalid => {
            "NISAR_SUBSET_V2_PHYSICAL_PLAN_INVALID"
        }
        crate::NisarSubsetPlanningErrorV2::ValidatorChanged
        | crate::NisarSubsetPlanningErrorV2::SourceValidatorChanged => {
            "NISAR_SUBSET_V2_VALIDATOR_CHANGED"
        }
        crate::NisarSubsetPlanningErrorV2::DependencyGraphInvalid => {
            "NISAR_SUBSET_V2_DEPENDENCY_GRAPH_INVALID"
        }
        crate::NisarSubsetPlanningErrorV2::FreezeFailed => "NISAR_SUBSET_V2_PLAN_FREEZE_FAILED",
        crate::NisarSubsetPlanningErrorV2::OutputLimitExceeded { .. } => {
            "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT"
        }
        crate::NisarSubsetPlanningErrorV2::OutputDiskSpaceInsufficient { .. } => {
            "NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT"
        }
        crate::NisarSubsetPlanningErrorV2::ExecutionStopped => unreachable!(),
    };
    let mut reason = error.to_string();
    if let Some((observation, diagnostic)) = observation
        .and_then(|observation| {
            observation
                .transport_diagnostic
                .map(|diagnostic| (observation, diagnostic))
        })
        .filter(|(observation, _)| {
            observation.failure_code == code
                && matches!(
                    observation.stage.as_str(),
                    "preflight"
                        | "metadata_prefix"
                        | "metadata_discovery"
                        | "additional_geolocation"
                        | "planning_revalidation"
                )
        })
    {
        if diagnostic.cause()
            == crate::http_transfer::ExactRangeTransportCause::TlsCertificate.as_str()
        {
            reason = format!(
                "{}，断点已保留。{reason}",
                crate::http_transfer::TLS_CERTIFICATE_FAILURE_ZH
            );
        }
        let http_status = observation
            .http_status
            .map_or_else(|| "none".to_owned(), |value| value.to_string());
        let response_body_bytes = observation
            .response_body_bytes
            .map_or_else(|| "none".to_owned(), |value| value.to_string());
        reason.push_str(&format!(
            " [stage={} transport_cause={} http_status={} request_count={} response_body_bytes={} range_start={} range_end_exclusive={}",
            observation.stage,
            diagnostic.cause(),
            http_status,
            observation.request_count,
            response_body_bytes,
            observation.start,
            observation.end_exclusive,
        ));
        if let Some(io_error_class) = diagnostic.io_error_class() {
            reason.push_str(&format!(" io_kind={}", io_error_class.as_str()));
        }
        // The retried planning Ranges name their real attempts (D1c: the
        // preflight; D1d: metadata prefix, metadata discovery and additional
        // geolocation): every observed failure of the same stage and Range.
        if matches!(
            observation.stage.as_str(),
            "preflight" | "metadata_prefix" | "metadata_discovery" | "additional_geolocation"
        ) {
            let attempts = observations
                .iter()
                .filter(|earlier| {
                    earlier.stage == observation.stage
                        && earlier.start == observation.start
                        && earlier.end_exclusive == observation.end_exclusive
                })
                .count();
            reason.push_str(&format!(" attempts={attempts}"));
        }
        if diagnostic.cause()
            == crate::http_transfer::ExactRangeTransportCause::TlsCertificate.as_str()
        {
            reason.push_str(" source=tls_certificate");
        }
        reason.push(']');
    }
    failure_result(work, destination, code, &reason, retryable)
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_science_parallel_range_workers(provider_id: &str) -> usize {
    if provider_id == insar_provider::catalog::BHOONIDHI_PROVIDER_ID {
        1
    } else {
        4
    }
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_v2_science_failure(
    work: &DownloadWork,
    destination: PathBuf,
    error: crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2,
    control: &WorkerControl,
) -> DownloadItemResult {
    nisar_v2_science_failure_with_auth_cause(work, destination, error, control, None)
}

#[cfg(feature = "nisar-subset-production")]
pub(crate) const NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED: &str =
    "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED";

/// A refused one-time budget upgrade is a local validation result, never a
/// provider or transport error. The diagnostic names the local stage and the
/// stable local cause, and states the next step instead of inviting a silent
/// identical retry.
#[cfg(feature = "nisar-subset-production")]
fn nisar_v2_budget_migration_failure(
    work: &DownloadWork,
    destination: PathBuf,
    failure: crate::nisar_science_transfer_v2::NisarBudgetMigrationFailureV2,
    control: &WorkerControl,
) -> DownloadItemResult {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as ScienceError;
    let Some(phase) = failure.refusal else {
        // Review 2026-10-17 round 3, P2-2: a full disk or an I/O error of the
        // local Store is transient; it says how to continue.
        if let Some(reason) = crate::nisar_science_transfer_v2::local_storage_failure_reason_zh(
            failure.error,
            "budget_migration_entry",
            "本次没有发起任何网络请求；",
        ) {
            return failure_result(work, destination, failure.error.code(), &reason, true);
        }
        if failure.error == ScienceError::Persistence {
            // A transient local Store or filesystem failure before any network
            // request is not a validation refusal: keep the ordinary code, mark it
            // retryable and keep the original diagnostic as the trailing note.
            return failure_result(
                work,
                destination,
                failure.error.code(),
                &format!(
                    "本地任务存储或恢复工作区暂时无法访问，本次没有发起任何网络请求；\
                     已下载的片段与断点记录保留在恢复工作区，可稍后继续。\
                     原始诊断：{} [stage=budget_migration_entry source=local]",
                    failure.error
                ),
                true,
            );
        }
        return nisar_v2_science_failure(work, destination, failure.error, control);
    };
    if matches!(
        failure.error,
        ScienceError::Paused | ScienceError::Cancelled
    ) {
        return nisar_v2_stopped_result(work, destination, control);
    }
    // Shared recovery may already have committed an interrupted legacy
    // transition before the refusal; say so instead of claiming nothing changed.
    let reconciled = if failure.reconciled {
        "上次中断遗留的传输记录已先按既有规则结算（计数只重新归类，不退还、不清零），\
         断点记录随之更新；"
    } else {
        ""
    };
    // Review 2026-10-17 round 2, P1-2: a Store refusal that waiting does not
    // change is terminal and keeps the Store's own code as the local diagnostic.
    let store_note = if matches!(failure.error, ScienceError::StoreRecordRejected { .. }) {
        "本地任务存储无法读取或确认升级所需的记录，这不是暂时性故障。"
    } else {
        ""
    };
    let reason = format!(
        "本地校验未通过，本次没有发起任何网络请求：旧任务的请求预算无法安全升级。\
         {store_note}本地阶段：{}（{}）；本地诊断：{}。\
         {reconciled}已下载的片段与断点记录保留在恢复工作区，没有被删除。\
         若再次继续仍出现同一提示，说明该状态无法自动恢复：请保留此任务供显式恢复，或重新创建任务重新下载。",
        phase.description_zh(),
        phase.code(),
        failure.error.code(),
    );
    failure_result(
        work,
        destination,
        NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED,
        &reason,
        false,
    )
}

#[cfg(feature = "nisar-subset-production")]
fn nisar_v2_science_failure_with_auth_cause(
    work: &DownloadWork,
    destination: PathBuf,
    error: crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2,
    control: &WorkerControl,
    auth_cause: Option<&'static str>,
) -> DownloadItemResult {
    if matches!(
        error,
        crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::Paused
            | crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::Cancelled
    ) {
        return nisar_v2_stopped_result(work, destination, control);
    }
    // Review 2026-10-17 round 3, P2-2: a full disk or an I/O error of the local
    // Store is transient with its local diagnostic, not a terminal refusal.
    if let Some(reason) = crate::nisar_science_transfer_v2::local_storage_failure_reason_zh(
        error,
        "science_checkpoint_anchor",
        "任务存储暂时无法记录最新的断点；",
    ) {
        return failure_result(work, destination, error.code(), &reason, true);
    }
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as ScienceError;
    let auth = matches!(
        error,
        ScienceError::AuthRefreshFailed | ScienceError::AuthRefreshTransient
    );
    if auth && is_local_refresh_limit(auth_cause) {
        return failure_result(
            work,
            destination,
            error.code(),
            &format!(
                "{LOCAL_REFRESH_LIMIT_SUMMARY_ZH}，{LOCAL_REFRESH_LIMIT_NEXT_ZH} \
                 [stage=science_auth_refresh source=local cause={}]",
                crate::BhoonidhiAuthError::LocalRefreshRateLimited.code()
            ),
            true,
        );
    }
    let retryable = matches!(
        error,
        ScienceError::Transport
            | ScienceError::ScienceTransferIncomplete
            | ScienceError::AuthRefreshTransient
    );
    let reason = if auth {
        match auth_cause {
            Some(cause) => format!("{error} [stage=science_auth_refresh cause={cause}]"),
            None => format!("{error} [stage=science_auth_refresh]"),
        }
    } else {
        error.to_string()
    };
    failure_result(work, destination, error.code(), &reason, retryable)
}

/// Review 2026-10-17 round 2, P2-3: the process-local hourly limit on automatic
/// Bhoonidhi refresh grants (design 2026-10-17 §4) sent no request, is not a
/// login problem and ends by waiting, so it has its own local text
/// (source=local) instead of "authentication refresh failed, check the login".
#[cfg(feature = "nisar-subset-production")]
const LOCAL_REFRESH_LIMIT_SUMMARY_ZH: &str =
    "Bhoonidhi 自动刷新已达本机每小时上限（9 次），本次没有发送认证请求";
#[cfg(feature = "nisar-subset-production")]
const LOCAL_REFRESH_LIMIT_NEXT_ZH: &str =
    "这是本机的自动刷新频率上限，断点保留，请稍后（最长约 1 小时）再继续";

#[cfg(feature = "nisar-subset-production")]
fn is_local_refresh_limit(auth_cause: Option<&'static str>) -> bool {
    auth_cause == Some(crate::BhoonidhiAuthError::LocalRefreshRateLimited.code())
}

/// Design 2026-10-17 §5: the stable code is unchanged; a Chinese summary names
/// the Range and its real cause, the tail keeps the stage, HTTP status or auth
/// cause and the real attempt count, and `attempt_count` is that real count.
/// Transient classes are retryable; a retry stopped for lack of request budget
/// is terminal budget exhaustion that still names the last real cause.
#[cfg(feature = "nisar-subset-production")]
fn nisar_v2_science_execution_failure(
    work: &DownloadWork,
    destination: PathBuf,
    error: crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2,
    control: &WorkerControl,
    auth_cause: Option<&'static str>,
    diagnostic: Option<crate::nisar_science_transfer_v2::NisarScienceFailureDiagnosticV2>,
) -> DownloadItemResult {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as ScienceError;
    let Some(diagnostic) =
        diagnostic.filter(|_| !matches!(error, ScienceError::Paused | ScienceError::Cancelled))
    else {
        return nisar_v2_science_failure_with_auth_cause(
            work,
            destination,
            error,
            control,
            auth_cause,
        );
    };
    let range = format!(
        "第 {}/{} 段",
        diagnostic.range_index + 1,
        diagnostic.range_count
    );
    let attempts = diagnostic.attempts;
    let retries = attempts.saturating_sub(1);
    let auth = diagnostic.auth_refresh_failure
        || matches!(
            error,
            ScienceError::AuthRefreshFailed | ScienceError::AuthRefreshTransient
        );
    let local_limit = auth && is_local_refresh_limit(auth_cause);
    let tail = if auth {
        format!(
            "[stage=science_auth_refresh{} cause={} range={}/{} attempts={attempts}{}]",
            if local_limit { " source=local" } else { "" },
            auth_cause.unwrap_or("none"),
            diagnostic.range_index + 1,
            diagnostic.range_count,
            if diagnostic.budget_stopped {
                " budget_stopped=true"
            } else {
                ""
            }
        )
    } else {
        format!(
            "[stage=science_range http_status={} transport_cause={} attempts={attempts}{}{}]",
            diagnostic
                .http_status
                .map_or_else(|| "none".to_owned(), |status| status.to_string()),
            diagnostic.transport_cause.unwrap_or("none"),
            if diagnostic.budget_stopped {
                " budget_stopped=true"
            } else {
                ""
            },
            if diagnostic.transport_cause
                == Some(crate::http_transfer::ExactRangeTransportCause::TlsCertificate.as_str())
            {
                " source=tls_certificate"
            } else {
                ""
            }
        )
    };
    let cause = if local_limit {
        LOCAL_REFRESH_LIMIT_SUMMARY_ZH.to_owned()
    } else if auth {
        "Bhoonidhi 认证刷新失败".to_owned()
    } else if let Some(status) = diagnostic.http_status {
        format!("HTTP {status}")
    } else if diagnostic.transport_cause
        == Some(crate::http_transfer::ExactRangeTransportCause::TlsCertificate.as_str())
    {
        crate::http_transfer::TLS_CERTIFICATE_FAILURE_ZH.to_owned()
    } else if diagnostic.transport_cause.is_some() {
        "连接失败或超时".to_owned()
    } else if error == ScienceError::ScienceTransferIncomplete {
        "正文中断或长度不足".to_owned()
    } else {
        error.to_string()
    };
    let (code, summary, retryable) = if diagnostic.budget_stopped {
        (
            ScienceError::CombinedBudgetExceeded.code(),
            format!(
                "{range} {cause}，自动重试所需的请求预算已用尽，未再发送请求；已下载的分段与断点保留"
            ),
            false,
        )
    } else if local_limit {
        (
            error.code(),
            format!("{range} {cause}，{LOCAL_REFRESH_LIMIT_NEXT_ZH}"),
            true,
        )
    } else if diagnostic.transient {
        let next = if auth {
            "可稍后重试；若仍失败请检测登录后重试"
        } else {
            "可稍后重试"
        };
        (
            error.code(),
            format!("{range} {cause}，已自动重试 {retries} 次仍失败，断点保留，{next}"),
            true,
        )
    } else {
        let next = if auth {
            "请检测登录后重试"
        } else {
            "该错误不会自动重试"
        };
        (
            error.code(),
            format!("{range} {cause}，{next}，断点保留（{error}）"),
            false,
        )
    };
    let mut result = failure_result(
        work,
        destination,
        code,
        &format!("{summary} {tail}"),
        retryable,
    );
    result.attempt_count = u64::from(attempts);
    result
}

/// Downloader enhancement D1b (review of D1/D2, P2-3): a NISAR V2 writer has
/// waited longer than its threshold. One local diagnostic line names the
/// reason and the holder's process id when it could be read. Only a holder
/// outside this program's normal queue, another program holding the artifact
/// lock or an earlier build holding the output directory, also records a
/// warning task event (D1c, L4 2026-10-18: one write took 306 s in the debug
/// build, so the next item always waited more than 5 minutes for the output
/// slot, which is ordinary queueing and no warning). A failure to record the
/// event leaves the writer waiting.
#[cfg(feature = "nisar-subset-production")]
fn report_nisar_writer_long_wait(
    authority: &NisarExecutionPlanFreezeAuthority,
    work: &DownloadWork,
    control: &WorkerControl,
    reason: crate::nisar_writer_admission_v2::NisarWriterWaitReasonV2,
    holder_pid: Option<u32>,
    waited: Duration,
    wait_started_at: String,
) {
    use crate::nisar_writer_admission_v2::NisarWriterWaitReasonV2 as Reason;
    use insar_store::StoreNisarOutputWaitReasonV2 as StoreReason;
    eprintln!(
        "NISAR_SUBSET_V2_WRITER_LONG_WAIT task_id={} item_id={} reason={} holder_pid={} waited_s={}",
        work.task.task_id,
        work.item.item_id,
        reason.code(),
        holder_pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string()),
        waited.as_secs()
    );
    let reason = match reason {
        // This program's own queue: the item card already shows 等待写出.
        Reason::SameArtifactInProcess | Reason::OutputSlot => return,
        Reason::SameArtifactOtherProcess => StoreReason::SameArtifactOtherProcess,
        Reason::EarlierBuild => StoreReason::EarlierBuild,
    };
    let request = insar_store::StoreNisarOutputWaitEventV2Request {
        task_id: work.task.task_id.clone(),
        item_id: work.item.item_id.clone(),
        reason,
        holder_pid,
        waited_seconds: waited.as_secs(),
        wait_started_at,
        occurred_at: timestamp_now(),
    };
    if let Err(error) = authority.record_output_wait_v2(request, control) {
        eprintln!(
            "NISAR_SUBSET_V2_WRITER_LONG_WAIT_EVENT_FAILED task_id={} item_id={} error={error}",
            work.task.task_id, work.item.item_id
        );
    }
}

#[cfg(feature = "nisar-subset-production")]
/// Issue #8 slice 6: a writer failure is a local result. Each code gets its
/// Chinese cause, the note that downloaded data and the checkpoint are kept
/// and what a retry does; resource failures (free space, a local write or the
/// final promotion) are retryable and a retry only regenerates the output.
fn nisar_v2_writer_failure(
    work: &DownloadWork,
    destination: PathBuf,
    error: crate::NisarSubsetWriteError,
    plan: Option<&crate::nisar_planning_v2::NisarSubsetPhysicalExecutionPlanV2>,
    control: &WorkerControl,
) -> DownloadItemResult {
    if error == crate::NisarSubsetWriteError::Cancelled {
        return nisar_v2_stopped_result(work, destination, control);
    }
    let failure = crate::nisar_output_diagnostics_v2::nisar_v2_writer_failure_zh(error, plan);
    failure_result(
        work,
        destination,
        failure.code,
        &failure.reason_zh,
        failure.retryable,
    )
}

fn failure_result(
    work: &DownloadWork,
    destination: PathBuf,
    code: &str,
    reason_zh: &str,
    retryable: bool,
) -> DownloadItemResult {
    DownloadItemResult {
        task_id: work.task.task_id.clone(),
        item_id: work.item.item_id.clone(),
        outcome: DownloadOutcome::Failed,
        destination,
        bytes_written: 0,
        attempt_count: 0,
        failure: Some(DownloadFailure {
            code: code.to_string(),
            reason_zh: reason_zh.to_string(),
            retryable,
        }),
        finished_at: timestamp_now(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::thread;
    use std::time::{Duration, Instant};

    use insar_core::{
        stable_asset_key, AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority,
        AssetExecutionPolicy, AssetRole, DownloadFailure, DownloadOutcome, FrozenTaskSnapshot,
        ProviderNetworkPolicy, ProxyPolicy, SnapshotItem, StableAssetReference,
    };
    use insar_provider::usgs_m2m::UsgsM2mSession;
    use serde_json::{json, Value};

    use super::{
        classify_asf_dataset_access_restriction, nisar_v2_planning_failure,
        nisar_v2_writer_failure, seasat_resolution_plan, sentinel_resolution_plan,
        DownloadProviderKind, ProductionDownloadWorker, SeasatResolutionPlan,
        SentinelResolutionPlan,
    };
    use crate::asset_resolver::ResolvedHostPolicy;
    use crate::{
        part_path, AssetResolver, AssetResolverRegistry, AuthRefreshPolicyId,
        CdseNetworkFoundation, CredentialMaterial, CredentialProvider, CredentialProviderError,
        DownloadWork, DownloadWorker, HttpTransfer, ResolveContext, ResolveError, ResolvedAsset,
        ResolvedMethod, ResolvedTransferResult, ResolvedTransferSession, ResolverExecutionRuntime,
        ShadowAccessIntent, ShadowResolveContext, SystemClock, TransferAdmissionController,
        TransferAuth, TransferBackend, TransferBackendKind, TransferOptions, TransferRequest,
        TransferResult, TransferUpdate, WorkerControl, ASF_BURST_ASSET_EXECUTION_POLICY_ID,
        ASF_BURST_COLLECTION_PROFILE_ID, EARTHDATA_BURST_CREDENTIAL_PROFILE,
    };

    const BURST_SOURCE_ITEM_ID: &str =
        "ASF::SLC-BURST::S1_000001_IW2_20260820T000000_VV_A1B2-BURST";
    const BURST_PROVIDER_PRODUCT_ID: &str = "G12345-ASF";
    pub(super) const BURST_NATIVE_PRODUCT_ID: &str = "S1_000001_IW2_20260820T000000_VV_A1B2-BURST";
    pub(super) const BURST_TIFF_NAME: &str = "measurement.tiff";
    const BURST_XML_NAME: &str = "metadata.xml";

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn nisar_v2_resolver_failure_preserves_bhoonidhi_context() {
        let error = crate::ResolverExecutionError::BhoonidhiLogin(
            crate::ResolveError::auth_failed().with_diagnostic("BHOONIDHI_AUTH_TRANSPORT_FAILED"),
        );
        let (code, reason_zh, retryable) = super::nisar_v2_resolver_failure(&error);

        assert_eq!(code, "DL004");
        assert_eq!(reason_zh, "BHOONIDHI_AUTH_TRANSPORT_FAILED");
        assert!(!retryable);
        assert!(!super::nisar_v2_resolver_error_is_lifecycle_stop(&error));
        assert!(super::nisar_v2_resolver_error_is_lifecycle_stop(
            &crate::ResolverExecutionError::Cancelled
        ));
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn bhoonidhi_auth_refresh_maps_only_typed_cancellation_to_lifecycle_stop() {
        use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as ScienceError;

        for pause in [true, false] {
            let control = WorkerControl::default();
            if pause {
                control.set_paused_for_test(true);
            } else {
                control.cancel_for_test();
            }
            let provider_error = crate::ResolverExecutionError::BhoonidhiLogin(
                crate::ResolveError::auth_failed()
                    .with_diagnostic(crate::BhoonidhiAuthError::Timeout.code()),
            );
            let (mapped, cause) =
                super::nisar_v2_science_auth_refresh_failure(&provider_error, &control);
            // Design 2026-10-17 §4: a refresh timeout is transient (retryable),
            // no longer the terminal AuthRefreshFailed; the typed cause is kept.
            assert_eq!(mapped, ScienceError::AuthRefreshTransient);
            assert_eq!(cause, Some(crate::BhoonidhiAuthError::Timeout.code()));
            let rejected = crate::ResolverExecutionError::BhoonidhiLogin(
                crate::ResolveError::auth_failed()
                    .with_diagnostic(crate::BhoonidhiAuthError::Rejected.code()),
            );
            assert_eq!(
                super::nisar_v2_science_auth_refresh_failure(&rejected, &control).0,
                ScienceError::AuthRefreshFailed
            );

            let (mapped, cause) = super::nisar_v2_science_auth_refresh_failure(
                &crate::ResolverExecutionError::Cancelled,
                &control,
            );
            assert_eq!(
                mapped,
                if pause {
                    ScienceError::Paused
                } else {
                    ScienceError::Cancelled
                }
            );
            assert_eq!(cause, None);
        }
    }

    /// Design 2026-10-17 §5: Chinese summary plus the real tail; the stable code
    /// is unchanged, retryable follows the class and attempt_count is real.
    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn nisar_v2_science_execution_failure_reports_range_cause_and_real_attempts() {
        use crate::nisar_science_transfer_v2::{
            NisarScienceFailureDiagnosticV2 as Diagnostic,
            NisarSubsetScienceTransferErrorV2 as Error,
        };
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let exhausted = Diagnostic {
            range_index: 36,
            range_count: 467,
            attempts: 3,
            automatic_retries: 2,
            http_status: Some(503),
            transport_cause: Some("retryable_http_status"),
            transient: true,
            auth_refresh_failure: false,
            budget_stopped: false,
        };
        let result = super::nisar_v2_science_execution_failure(
            &work,
            PathBuf::from("output"),
            Error::Transport,
            &WorkerControl::default(),
            None,
            Some(exhausted.clone()),
        );
        assert_eq!(result.attempt_count, 3);
        let failure = result.failure.unwrap();
        assert_eq!(failure.code, Error::Transport.code());
        assert!(failure.retryable);
        assert_eq!(
            failure.reason_zh,
            "第 37/467 段 HTTP 503，已自动重试 2 次仍失败，断点保留，可稍后重试 \
             [stage=science_range http_status=503 transport_cause=retryable_http_status attempts=3]"
        );

        // L5 recheck 2026-10-19: a certificate verification failure names it.
        let tls = super::nisar_v2_science_execution_failure(
            &work,
            PathBuf::from("output"),
            Error::Transport,
            &WorkerControl::default(),
            None,
            Some(Diagnostic {
                http_status: None,
                transport_cause: Some("tls_certificate"),
                ..exhausted.clone()
            }),
        );
        assert_eq!(
            tls.failure.unwrap().reason_zh,
            "第 37/467 段 无法验证服务器证书（本机证书库无法读取或网络被拦截），已自动重试 2 次仍失败，断点保留，可稍后重试 \
             [stage=science_range http_status=none transport_cause=tls_certificate attempts=3 source=tls_certificate]"
        );

        let stopped = super::nisar_v2_science_execution_failure(
            &work,
            PathBuf::from("output"),
            Error::CombinedBudgetExceeded,
            &WorkerControl::default(),
            None,
            Some(Diagnostic {
                attempts: 1,
                budget_stopped: true,
                ..exhausted.clone()
            }),
        );
        let failure = stopped.failure.unwrap();
        assert_eq!(failure.code, Error::CombinedBudgetExceeded.code());
        assert!(!failure.retryable);
        for expected in [
            "HTTP 503",
            "未再发送请求",
            "budget_stopped=true",
            "attempts=1",
        ] {
            assert!(
                failure.reason_zh.contains(expected),
                "{}",
                failure.reason_zh
            );
        }
        // A refresh cause left by an earlier attempt never relabels a later
        // HTTP failure as an authentication failure.
        let stale_cause = super::nisar_v2_science_execution_failure(
            &work,
            PathBuf::from("output"),
            Error::CombinedBudgetExceeded,
            &WorkerControl::default(),
            Some(crate::BhoonidhiAuthError::Timeout.code()),
            Some(Diagnostic {
                attempts: 1,
                budget_stopped: true,
                ..exhausted.clone()
            }),
        );
        let reason = stale_cause.failure.unwrap().reason_zh;
        assert!(
            reason.contains("HTTP 503") && reason.contains("stage=science_range"),
            "{reason}"
        );

        let auth = super::nisar_v2_science_execution_failure(
            &work,
            PathBuf::from("output"),
            Error::AuthRefreshTransient,
            &WorkerControl::default(),
            Some(crate::BhoonidhiAuthError::Timeout.code()),
            Some(Diagnostic {
                http_status: None,
                transport_cause: None,
                auth_refresh_failure: true,
                ..exhausted.clone()
            }),
        );
        let failure = auth.failure.unwrap();
        assert_eq!(failure.code, "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED");
        assert!(failure.retryable);
        for expected in [
            "已自动重试 2 次仍失败",
            "检测登录",
            "[stage=science_auth_refresh cause=BHOONIDHI_AUTH_TIMEOUT range=37/467 attempts=3]",
        ] {
            assert!(
                failure.reason_zh.contains(expected),
                "{}",
                failure.reason_zh
            );
        }

        let terminal = super::nisar_v2_science_execution_failure(
            &work,
            PathBuf::from("output"),
            Error::RangeUnsupported,
            &WorkerControl::default(),
            None,
            Some(Diagnostic {
                attempts: 1,
                automatic_retries: 0,
                http_status: Some(200),
                transport_cause: None,
                transient: false,
                ..exhausted.clone()
            }),
        );
        let failure = terminal.failure.unwrap();
        assert_eq!(failure.code, Error::RangeUnsupported.code());
        assert!(!failure.retryable);
        assert!(
            failure.reason_zh.contains("HTTP 200"),
            "{}",
            failure.reason_zh
        );
        assert!(
            failure.reason_zh.contains("不会自动重试"),
            "{}",
            failure.reason_zh
        );

        let control = WorkerControl::default();
        control.set_paused_for_test(true);
        let paused = super::nisar_v2_science_execution_failure(
            &work,
            PathBuf::from("output"),
            Error::Paused,
            &control,
            None,
            Some(exhausted),
        );
        assert_eq!(paused.outcome, DownloadOutcome::Interrupted);
        assert!(paused.failure.is_none());
    }

    /// Review 2026-10-17 round 2, P1-2: a Store refusal that waiting does not
    /// change (here an unsupported proof version) is the terminal local refusal
    /// with the Store's own code; only a transient Persistence failure keeps the
    /// ordinary retryable "temporarily unavailable" failure.
    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn nisar_v2_budget_migration_store_refusal_is_terminal_and_keeps_the_store_code() {
        use crate::nisar_science_transfer_v2::{
            NisarBudgetMigrationFailureV2 as Failure, NisarBudgetMigrationPhaseV2 as Phase,
            NisarSubsetScienceTransferErrorV2 as Error,
        };
        const VERSION: &str = "V3_STORE_NISAR_BUDGET_UPGRADE_PROOF_VERSION_UNSUPPORTED";
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let control = WorkerControl::default();
        let refused = super::nisar_v2_budget_migration_failure(
            &work,
            PathBuf::from("output"),
            Failure {
                error: Error::StoreRecordRejected {
                    store_code: VERSION,
                },
                refusal: Some(Phase::ProofReconciliation),
                reconciled: false,
            },
            &control,
        )
        .failure
        .unwrap();
        assert_eq!(refused.code, super::NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED);
        assert!(!refused.retryable);
        for expected in [
            "本地校验未通过",
            "没有发起任何网络请求",
            "不是暂时性故障",
            "proof_reconciliation",
            VERSION,
        ] {
            assert!(
                refused.reason_zh.contains(expected),
                "{}",
                refused.reason_zh
            );
        }
        assert!(!refused.reason_zh.contains("暂时无法访问"));

        let transient = super::nisar_v2_budget_migration_failure(
            &work,
            PathBuf::from("output"),
            Failure {
                error: Error::Persistence,
                refusal: None,
                reconciled: false,
            },
            &control,
        )
        .failure
        .unwrap();
        assert_eq!(transient.code, Error::Persistence.code());
        assert!(transient.retryable);
        assert!(transient.reason_zh.contains("暂时无法访问"));
    }

    /// Review 2026-10-17 round 3, P2-2: a full disk or an I/O error of the
    /// local Store is transient with a local disk diagnostic, both at the
    /// migration entry (where no request was sent) and in the science path,
    /// never the terminal refusal.
    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn nisar_v2_local_storage_failure_is_transient_with_a_local_disk_diagnostic() {
        use crate::nisar_science_transfer_v2::{
            NisarBudgetMigrationFailureV2 as Failure, NisarSubsetScienceTransferErrorV2 as Error,
        };
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let control = WorkerControl::default();
        let error = Error::LocalStorageFailure {
            store_code: "V3_STORE_DATABASE_ERROR",
            cause: "disk_full",
        };
        let entry = super::nisar_v2_budget_migration_failure(
            &work,
            PathBuf::from("output"),
            Failure {
                error,
                refusal: None,
                reconciled: false,
            },
            &control,
        )
        .failure
        .unwrap();
        let science =
            super::nisar_v2_science_failure(&work, PathBuf::from("output"), error, &control)
                .failure
                .unwrap();
        for (failure, stage) in [
            (&entry, "stage=budget_migration_entry"),
            (&science, "stage=science_checkpoint_anchor"),
        ] {
            assert_eq!(failure.code, Error::Persistence.code());
            assert!(failure.retryable, "{}", failure.reason_zh);
            for expected in [
                "本地磁盘空间不足或文件读写失败",
                "释放磁盘空间或检查磁盘后可继续",
                stage,
                "source=local",
                "store_error=V3_STORE_DATABASE_ERROR",
                "cause=disk_full",
            ] {
                assert!(
                    failure.reason_zh.contains(expected),
                    "{}",
                    failure.reason_zh
                );
            }
        }
        assert!(entry.reason_zh.contains("没有发起任何网络请求"));
    }

    /// Review 2026-10-17 round 2, P2-3: the process-local hourly limit on
    /// automatic refresh grants sent no request and is not a login problem.
    /// Its own local text (source=local) replaces "authentication refresh
    /// failed, check the login", with and without a Range diagnostic.
    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn nisar_v2_local_refresh_limit_is_reported_as_local_without_login_guidance() {
        use crate::nisar_science_transfer_v2::{
            NisarScienceFailureDiagnosticV2 as Diagnostic,
            NisarSubsetScienceTransferErrorV2 as Error,
        };
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let limit = Some(crate::BhoonidhiAuthError::LocalRefreshRateLimited.code());
        let with_diagnostic = super::nisar_v2_science_execution_failure(
            &work,
            PathBuf::from("output"),
            Error::AuthRefreshFailed,
            &WorkerControl::default(),
            limit,
            Some(Diagnostic {
                range_index: 36,
                range_count: 467,
                attempts: 1,
                automatic_retries: 0,
                http_status: None,
                transport_cause: None,
                transient: false,
                auth_refresh_failure: true,
                budget_stopped: false,
            }),
        );
        let without_diagnostic = super::nisar_v2_science_failure_with_auth_cause(
            &work,
            PathBuf::from("output"),
            Error::AuthRefreshFailed,
            &WorkerControl::default(),
            limit,
        );
        for result in [with_diagnostic, without_diagnostic] {
            let failure = result.failure.unwrap();
            assert_eq!(failure.code, Error::AuthRefreshFailed.code());
            assert!(failure.retryable, "{}", failure.reason_zh);
            for expected in [
                "本机每小时上限",
                "没有发送认证请求",
                "source=local",
                "cause=BHOONIDHI_REFRESH_LOCAL_RATE_LIMITED",
            ] {
                assert!(
                    failure.reason_zh.contains(expected),
                    "{}",
                    failure.reason_zh
                );
            }
            assert!(
                !failure.reason_zh.contains("检测登录")
                    && !failure.reason_zh.contains("认证刷新失败"),
                "{}",
                failure.reason_zh
            );
        }
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn bhoonidhi_science_auth_failure_reports_refresh_stage() {
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let result = super::nisar_v2_science_failure(
            &work,
            PathBuf::from("output"),
            crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::AuthRefreshFailed,
            &WorkerControl::default(),
        );
        let failure = result.failure.unwrap();
        assert_eq!(failure.code, "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED");
        assert!(!failure.retryable);
        assert!(failure.reason_zh.contains("stage=science_auth_refresh"));
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn bhoonidhi_science_failure_retains_post_refresh_probe_stage() {
        use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as Error;
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let resolver_error = crate::ResolverExecutionError::BhoonidhiLogin(
            crate::ResolveError::auth_failed()
                .with_diagnostic("BHOONIDHI_DOWNLOAD_PROBE_STATUS_401 stage=post_refresh_probe"),
        );
        let result = super::nisar_v2_science_failure_with_auth_cause(
            &work,
            PathBuf::from("output"),
            Error::AuthRefreshFailed,
            &WorkerControl::default(),
            resolver_error.bhoonidhi_auth_diagnostic(),
        );
        let failure = result.failure.unwrap();
        assert!(failure.reason_zh.contains("stage=post_refresh_probe"));
        assert!(failure
            .reason_zh
            .contains("BHOONIDHI_DOWNLOAD_PROBE_STATUS_401"));
        assert!(!failure.retryable);
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn bhoonidhi_science_auth_failure_preserves_only_typed_cause_during_control_race() {
        use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as Error;
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let cause = Some(crate::BhoonidhiAuthError::Timeout.code());
        let result = super::nisar_v2_science_failure_with_auth_cause(
            &work,
            PathBuf::from("output"),
            Error::AuthRefreshFailed,
            &WorkerControl::default(),
            cause,
        );
        let failure = result.failure.unwrap();
        assert_eq!(failure.code, Error::AuthRefreshFailed.code());
        assert_eq!(
            failure.reason_zh,
            format!(
                "{} [stage=science_auth_refresh cause=BHOONIDHI_AUTH_TIMEOUT]",
                Error::AuthRefreshFailed
            )
        );
        assert!(!failure.retryable);
        for error in [
            Error::Transport,
            Error::StrongValidatorChanged,
            Error::AuthorityMismatch,
            Error::CombinedBudgetExceeded,
        ] {
            let actual = super::nisar_v2_science_failure_with_auth_cause(
                &work,
                PathBuf::from("output"),
                error,
                &WorkerControl::default(),
                cause,
            );
            let failure = actual.failure.unwrap();
            assert_eq!(failure.code, error.code());
            assert_eq!(failure.reason_zh, error.to_string());
            assert_eq!(failure.retryable, error == Error::Transport);
        }
        for pause in [true, false] {
            let control = WorkerControl::default();
            if pause {
                control.set_paused_for_test(true);
            } else {
                control.cancel_for_test();
            }
            let actual = super::nisar_v2_science_failure_with_auth_cause(
                &work,
                PathBuf::from("output"),
                Error::AuthRefreshFailed,
                &control,
                cause,
            );
            assert_eq!(actual.outcome, DownloadOutcome::Failed);
            let failure = actual
                .failure
                .expect("real auth failure must remain visible");
            assert_eq!(failure.code, Error::AuthRefreshFailed.code());
            assert!(failure.reason_zh.contains("BHOONIDHI_AUTH_TIMEOUT"));
        }
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn nisar_subset_transfer_progress_reporter_publishes_monotonic_real_bytes() {
        let mut updates = Vec::new();
        let mut sink = |update| updates.push(update);
        {
            let mut reporter = super::NisarSubsetTransferProgressReporter::new(
                "task-progress",
                "item-progress",
                &mut sink,
            );
            reporter.observe(super::NISAR_SUBSET_PROGRESS_EMIT_BYTES - 1);
            reporter.observe(1);
            reporter.observe(7);
            reporter.flush();
            assert_eq!(
                reporter.observed_bytes(),
                super::NISAR_SUBSET_PROGRESS_EMIT_BYTES + 7
            );
        }
        assert_eq!(updates.len(), 2);
        assert_eq!(
            updates[0].bytes_completed,
            super::NISAR_SUBSET_PROGRESS_EMIT_BYTES
        );
        assert_eq!(
            updates[1].bytes_completed,
            super::NISAR_SUBSET_PROGRESS_EMIT_BYTES + 7
        );
        assert!(updates.iter().all(|update| update.bytes_completed > 0));
        assert!(updates
            .windows(2)
            .all(|pair| pair[0].bytes_completed < pair[1].bytes_completed));
    }

    /// L4 finding (2026-10-17): the card kept showing a Range transfer while
    /// the HDF5 output was written. At the end of the science transfer the
    /// reporter publishes one zero-rate row of this run's bytes, also when no
    /// byte moved in this run. Review 2026-10-17 round 2, P2-2: that row has no
    /// total, so it never becomes a persisted total; it is published once.
    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn nisar_subset_transfer_progress_reporter_marks_the_writer_boundary() {
        for observed in [0, 7] {
            let mut updates = Vec::new();
            let mut sink = |update| updates.push(update);
            {
                let mut reporter = super::NisarSubsetTransferProgressReporter::new(
                    "task-progress",
                    "item-progress",
                    &mut sink,
                );
                reporter.observe_science(
                    crate::nisar_science_transfer_v2::NisarScienceTransferProgressV2::Bytes(
                        observed,
                    ),
                );
                reporter.observe_science(
                    crate::nisar_science_transfer_v2::NisarScienceTransferProgressV2::TransferComplete,
                );
                reporter.mark_transfer_complete();
            }
            let markers = updates
                .iter()
                .filter(|update| update.bytes_per_second == 0.0)
                .count();
            assert_eq!(markers, 1, "published once");
            let last = updates.last().unwrap();
            assert_eq!(last.bytes_completed, observed);
            assert_eq!(last.total_bytes, None);
            assert_eq!(last.bytes_per_second, 0.0);
            assert_eq!(last.status, insar_core::DownloadItemStatus::Active);
        }
    }

    pub(super) fn burst_worker_item(role: AssetRole) -> SnapshotItem {
        let (role_name, file_name, media_type, size_bytes) = match role {
            AssetRole::Primary => ("primary", BURST_TIFF_NAME, "image/tiff", Some(1_024_u64)),
            AssetRole::Metadata => ("metadata", BURST_XML_NAME, "application/xml", None),
            _ => unreachable!(),
        };
        let stable_key = stable_asset_key(
            BURST_SOURCE_ITEM_ID,
            &format!("{BURST_NATIVE_PRODUCT_ID}:{file_name}"),
        )
        .unwrap();
        let stable_asset_keys = [BURST_TIFF_NAME, BURST_XML_NAME]
            .map(|name| {
                stable_asset_key(
                    BURST_SOURCE_ITEM_ID,
                    &format!("{BURST_NATIVE_PRODUCT_ID}:{name}"),
                )
                .unwrap()
            })
            .to_vec();
        let href =
            format!("https://sentinel1-burst.asf.alaska.edu/{BURST_NATIVE_PRODUCT_ID}/{file_name}");
        let mut metadata = json!({
            "provider": "asf.catalog",
            "dataset": "SLC-BURST",
            "product_family": "BURST",
            "processing_level": "L1",
            "identity_rule_id": ASF_BURST_COLLECTION_PROFILE_ID,
            "product_key": BURST_SOURCE_ITEM_ID,
            "provider_product_key": BURST_SOURCE_ITEM_ID,
            "provider_product_id": BURST_PROVIDER_PRODUCT_ID,
            "product_id": BURST_NATIVE_PRODUCT_ID,
            "scene_id": BURST_NATIVE_PRODUCT_ID,
            "source_item_id": BURST_SOURCE_ITEM_ID,
            "stable_asset_key": stable_key,
            "stable_asset_keys": stable_asset_keys,
            "platform": "SENTINEL-1A",
            "instrument": "C-SAR",
            "beam_mode": "IW",
            "beam_swath": "IW2",
            "polarization": "VV",
            "flight_direction": "ASCENDING",
            "look_direction": "RIGHT",
            "relative_orbit": 141,
            "absolute_orbit": 57142,
            "path": 141,
            "burst": {
                "burst_id": null,
                "full_burst_id": "141_301495_IW2",
                "relative_burst_id": 301495,
                "absolute_burst_id": 5714273,
                "subswath": "IW2",
                "burst_index": 0
            },
            "native_metadata": {
                "cmr_granule_concept_id": BURST_PROVIDER_PRODUCT_ID,
                "cmr_native_id": BURST_NATIVE_PRODUCT_ID,
                "raw_authority": {},
                "collection": {
                    "concept_id": "C2709161906-ASF",
                    "short_name": "SENTINEL-1_BURSTS",
                    "version": "1",
                    "provider_id": "ASF"
                },
                "burst": {
                    "full_id": "141_301495_IW2",
                    "relative_id": 301495,
                    "absolute_id": 5714273,
                    "index": 0,
                    "subswath": "IW2",
                    "byte_length": 1024
                },
                "scope": {
                    "beam_mode": "IW",
                    "processing_level": "L1",
                    "processing_type": "BURST",
                    "path": 141,
                    "flight_direction": "ASCENDING",
                    "platform": "SENTINEL-1A",
                    "instrument": "C-SAR",
                    "look_direction": "RIGHT"
                },
                "browse": null,
                "preview_available": false
            },
            "asset_role": role_name,
            "official_href": href,
            "asset_basename": file_name,
            "media_type": media_type,
            "size_bytes": size_bytes,
            "burst_asset_policy": "all"
        })
        .as_object()
        .unwrap()
        .clone();
        let mut payload = json!({
            "burst_asset": true,
            "download_url": href,
            "file_name": file_name,
            "media_type": media_type,
            "source_item_id": BURST_SOURCE_ITEM_ID,
            "asset_role": role_name,
            "burst_asset_policy": "all"
        })
        .as_object()
        .unwrap()
        .clone();
        if let Some(size_bytes) = size_bytes {
            payload.insert("file_size_remote".to_owned(), Value::from(size_bytes));
        }
        SnapshotItem {
            item_id: stable_key,
            provider_id: "asf.catalog".to_owned(),
            display_name: file_name.to_owned(),
            acquisition_datetime: Some("2026-08-20T00:00:00Z".to_owned()),
            footprint_geojson: None,
            metadata: std::mem::take(&mut metadata),
            payload,
            asset_access: None,
            execution_policy: Some(AssetExecutionPolicy {
                authority: AssetExecutionAuthority::ResolverAuthoritative,
                policy_id: ASF_BURST_ASSET_EXECUTION_POLICY_ID.to_owned(),
                collection_profile_id: Some(ASF_BURST_COLLECTION_PROFILE_ID.to_owned()),
            }),
        }
    }

    fn burst_worker_task(
        items: &[SnapshotItem],
        output_root: &std::path::Path,
    ) -> FrozenTaskSnapshot {
        serde_json::from_value(json!({
            "schema_version": 1,
            "task_id": "burst-step-7-worker",
            "batch_id": "immutable-source-batch",
            "provider_id": "asf.catalog",
            "created_at": "2026-08-26T00:00:00Z",
            "aoi_name": "fixture",
            "aoi_geojson": null,
            "query": {"dataset": "SLC-BURST"},
            "candidates": items,
            "initial_selected_item_ids": items.iter().map(|item| item.item_id.clone()).collect::<Vec<_>>(),
            "output_root": output_root,
            "layout": "direct"
        }))
        .unwrap()
    }

    #[test]
    fn nisar_v2_route_preserves_physical_and_writer_recovery_error_codes() {
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        for (error, expected) in [
            (
                crate::NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable,
                "NISAR_SUBSET_V2_PHYSICAL_MAPPING_UNAVAILABLE",
            ),
            (
                crate::NisarSubsetPlanningErrorV2::PhysicalPlanInvalid,
                "NISAR_SUBSET_V2_PHYSICAL_PLAN_INVALID",
            ),
        ] {
            let result = nisar_v2_planning_failure(
                &work,
                PathBuf::from("output"),
                error,
                None,
                &WorkerControl::default(),
            );
            assert_eq!(result.failure.unwrap().code, expected);
        }
        let result = nisar_v2_writer_failure(
            &work,
            PathBuf::from("output"),
            crate::NisarSubsetWriteError::RecoveryInvalid,
            None,
            &WorkerControl::default(),
        );
        assert_eq!(
            result.failure.unwrap().code,
            "NISAR_SUBSET_V2_WRITER_RECOVERY_INVALID"
        );
        let paused = WorkerControl::default();
        paused.set_paused_for_test(true);
        let result = nisar_v2_writer_failure(
            &work,
            PathBuf::from("output"),
            crate::NisarSubsetWriteError::RecoveryInvalid,
            None,
            &paused,
        );
        assert_eq!(result.outcome, DownloadOutcome::Failed);
        assert_eq!(
            result.failure.unwrap().code,
            "NISAR_SUBSET_V2_WRITER_RECOVERY_INVALID"
        );
    }

    /// Issue #8 slice 6: an output refused before the freeze is a local result
    /// in Chinese with its sizes; missing space is retryable, the bound is not.
    #[test]
    fn nisar_v2_output_admission_refusals_are_local_chinese_with_sizes() {
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        for (error, code, retryable, sizes) in [
            (
                crate::NisarSubsetPlanningErrorV2::OutputLimitExceeded {
                    required: 3_000_000_000,
                    limit: 2_147_483_648,
                },
                "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT",
                false,
                "required=3000000000 limit=2147483648]",
            ),
            (
                crate::NisarSubsetPlanningErrorV2::OutputDiskSpaceInsufficient {
                    required: 9_000_000,
                    available: 4_096,
                },
                "NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT",
                true,
                "required=9000000 limit=4096]",
            ),
        ] {
            let result = nisar_v2_planning_failure(
                &work,
                PathBuf::from("output"),
                error,
                None,
                &WorkerControl::default(),
            );
            assert_eq!(result.outcome, DownloadOutcome::Failed);
            let failure = result.failure.unwrap();
            assert_eq!(failure.code, code);
            assert_eq!(failure.retryable, retryable, "{code}");
            for expected in [
                "计划未冻结，没有发起科学数据请求",
                "已下载数据与断点已保留",
                "[stage=planning_admission source=local code=",
                sizes,
            ] {
                assert!(
                    failure.reason_zh.contains(expected),
                    "{}",
                    failure.reason_zh
                );
            }
            assert!(
                !failure.reason_zh.contains("NISAR V2"),
                "{}",
                failure.reason_zh
            );
        }
    }

    /// Issue #8 slice 6: every writer failure is local, in Chinese, says that
    /// downloaded data and the checkpoint are kept, and never invites a login
    /// check; resource failures are retryable and size bounds are not.
    #[test]
    fn nisar_v2_writer_failures_are_local_chinese_and_classified_for_retry() {
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        for (error, code, retryable) in [
            (
                crate::NisarSubsetWriteError::LogicalCaptureLimitExceeded,
                "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT",
                false,
            ),
            (
                crate::NisarSubsetWriteError::InsufficientDiskSpace {
                    required: 2_000_000,
                    available: 1_000,
                },
                "NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT",
                true,
            ),
            (
                crate::NisarSubsetWriteError::Persistence,
                "NISAR_SUBSET_ARTIFACT_INVALID",
                true,
            ),
            (
                crate::NisarSubsetWriteError::AtomicPromotion,
                "NISAR_SUBSET_V2_ATOMIC_PROMOTION_FAILED",
                true,
            ),
            (
                crate::NisarSubsetWriteError::SelectedScienceMissing,
                "NISAR_SUBSET_V2_SELECTED_SCIENCE_MISSING",
                false,
            ),
            (
                crate::NisarSubsetWriteError::MultiPolarizationInvalid,
                "NISAR_SUBSET_V2_MULTI_POL_WRITER_INVALID",
                false,
            ),
            (
                crate::NisarSubsetWriteError::PostWriteScienceValidation,
                "NISAR_SUBSET_V2_POST_WRITE_SCIENCE_VALIDATION",
                false,
            ),
            (
                crate::NisarSubsetWriteError::ManifestInvalid,
                "NISAR_SUBSET_V2_MANIFEST_INVALID",
                false,
            ),
            (
                crate::NisarSubsetWriteError::RecoveryInvalid,
                "NISAR_SUBSET_V2_WRITER_RECOVERY_INVALID",
                false,
            ),
            (
                crate::NisarSubsetWriteError::IncompleteScienceAuthority,
                "NISAR_SUBSET_V2_SCIENCE_AUTHORITY_INVALID",
                false,
            ),
            (
                crate::NisarSubsetWriteError::WriterBinding,
                "NISAR_SUBSET_V2_WRITER_BINDING",
                false,
            ),
            (
                crate::NisarSubsetWriteError::OutputUnsafe,
                "NISAR_SUBSET_ARTIFACT_INVALID",
                false,
            ),
            (
                crate::NisarSubsetWriteError::OutputBusy,
                "NISAR_SUBSET_V2_OUTPUT_BUSY",
                true,
            ),
        ] {
            let result = nisar_v2_writer_failure(
                &work,
                PathBuf::from("output"),
                error,
                None,
                &WorkerControl::default(),
            );
            assert_eq!(result.outcome, DownloadOutcome::Failed);
            let failure = result.failure.unwrap();
            assert_eq!(failure.code, code);
            assert_eq!(failure.retryable, retryable, "{code}");
            for expected in [
                "已下载数据与断点已保留",
                "[stage=writer source=local code=",
                code,
            ] {
                assert!(
                    failure.reason_zh.contains(expected),
                    "{}",
                    failure.reason_zh
                );
            }
            assert!(
                !failure.reason_zh.contains("NISAR V2"),
                "{}",
                failure.reason_zh
            );
            assert!(!failure.reason_zh.contains("登录"), "{}", failure.reason_zh);
        }
        let limit = nisar_v2_writer_failure(
            &work,
            PathBuf::from("output"),
            crate::NisarSubsetWriteError::LogicalCaptureLimitExceeded,
            None,
            &WorkerControl::default(),
        )
        .failure
        .unwrap();
        assert!(
            limit
                .reason_zh
                .contains("升级后重试只重新生成文件、不重新下载"),
            "{}",
            limit.reason_zh
        );
        let disk = nisar_v2_writer_failure(
            &work,
            PathBuf::from("output"),
            crate::NisarSubsetWriteError::InsufficientDiskSpace {
                required: 2_000_000,
                available: 1_000,
            },
            None,
            &WorkerControl::default(),
        )
        .failure
        .unwrap();
        assert!(
            disk.reason_zh.contains("required=2000000 limit=1000]"),
            "{}",
            disk.reason_zh
        );
    }

    #[test]
    fn nisar_v2_planning_retryability_matches_durable_prefix_recovery_contract() {
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        for error in [
            crate::NisarSubsetPlanningErrorV2::BootstrapPartialResponse,
            crate::NisarSubsetPlanningErrorV2::BootstrapTransport,
        ] {
            let result = nisar_v2_planning_failure(
                &work,
                PathBuf::from("output"),
                error,
                None,
                &WorkerControl::default(),
            );
            assert_eq!(result.outcome, DownloadOutcome::Failed);
            assert!(result.failure.unwrap().retryable);
        }
        for error in [
            crate::NisarSubsetPlanningErrorV2::RecoveryInvalid,
            crate::NisarSubsetPlanningErrorV2::SourceValidatorChanged,
            crate::NisarSubsetPlanningErrorV2::BootstrapRangeUnsupported,
            crate::NisarSubsetPlanningErrorV2::BootstrapResponseInvalid,
        ] {
            let result = nisar_v2_planning_failure(
                &work,
                PathBuf::from("output"),
                error,
                None,
                &WorkerControl::default(),
            );
            assert_eq!(result.outcome, DownloadOutcome::Failed);
            assert!(!result.failure.unwrap().retryable);
        }
    }

    fn planning_transport_observation(
        stage: &str,
        failure_code: &'static str,
    ) -> crate::nisar_planning_v2::NisarSubsetRangeFailureObservationV2 {
        crate::nisar_planning_v2::NisarSubsetRangeFailureObservationV2 {
            stage: stage.to_owned(),
            start: 20,
            end_exclusive: 24,
            source_size_bytes: 100,
            disposition: "sent_outcome_indeterminate",
            request_count: 1,
            http_status: Some(503),
            response_body_bytes: None,
            transport_diagnostic: Some(
                crate::nisar_planning_v2::NisarSubsetTransportDiagnosticV2::ExactRangeSend(
                    crate::http_transfer::ExactRangeTransportCause::RetryableHttpStatus,
                ),
            ),
            validator_sha256: None,
            failure_code,
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

    /// L5 recheck 2026-10-19: a certificate that cannot be verified is named as
    /// such in the Chinese reason and the tail, not as a connect failure.
    #[test]
    fn nisar_v2_planning_tls_certificate_failure_names_its_cause() {
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let mut observation =
            planning_transport_observation("preflight", "NISAR_SUBSET_V2_TRANSPORT_FAILED");
        observation.http_status = None;
        observation.transport_diagnostic = Some(
            crate::nisar_planning_v2::NisarSubsetTransportDiagnosticV2::ExactRangeSend(
                crate::http_transfer::ExactRangeTransportCause::TlsCertificate,
            ),
        );
        let result = nisar_v2_planning_failure(
            &work,
            PathBuf::from("output"),
            crate::NisarSubsetPlanningErrorV2::BootstrapTransport,
            Some(&observation),
            &WorkerControl::default(),
        );
        let failure = result.failure.unwrap();
        assert_eq!(failure.code, "NISAR_SUBSET_V2_TRANSPORT_FAILED");
        assert!(failure.retryable);
        assert_eq!(
            failure.reason_zh,
            "无法验证服务器证书（本机证书库无法读取或网络被拦截），断点已保留。NISAR V2 Range transport failed [stage=preflight transport_cause=tls_certificate http_status=none request_count=1 response_body_bytes=none range_start=20 range_end_exclusive=24 attempts=1 source=tls_certificate]"
        );
    }

    #[test]
    fn nisar_v2_planning_transport_reason_uses_only_matching_latest_observation() {
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let matching = planning_transport_observation(
            "metadata_discovery",
            "NISAR_SUBSET_V2_TRANSPORT_FAILED",
        );
        let result = nisar_v2_planning_failure(
            &work,
            PathBuf::from("output"),
            crate::NisarSubsetPlanningErrorV2::BootstrapTransport,
            Some(&matching),
            &WorkerControl::default(),
        );
        assert_eq!(
            result.failure.unwrap().reason_zh,
            "NISAR V2 Range transport failed [stage=metadata_discovery transport_cause=retryable_http_status http_status=503 request_count=1 response_body_bytes=none range_start=20 range_end_exclusive=24 attempts=1]"
        );

        let mut eof_probe = planning_transport_observation(
            "metadata_discovery",
            "NISAR_SUBSET_V2_TRANSPORT_FAILED",
        );
        eof_probe.http_status = Some(206);
        eof_probe.response_body_bytes = Some(4);
        eof_probe.transport_diagnostic = Some(
            crate::nisar_planning_v2::NisarSubsetTransportDiagnosticV2::PostBodyEofProbeRead {
                io_error_class: crate::nisar_planning_v2::NisarSubsetIoErrorClassV2::Connection,
            },
        );
        let result = nisar_v2_planning_failure(
            &work,
            PathBuf::from("output"),
            crate::NisarSubsetPlanningErrorV2::BootstrapTransport,
            Some(&eof_probe),
            &WorkerControl::default(),
        );
        assert_eq!(
            result.failure.unwrap().reason_zh,
            "NISAR V2 Range transport failed [stage=metadata_discovery transport_cause=post_body_eof_probe_read http_status=206 request_count=1 response_body_bytes=4 range_start=20 range_end_exclusive=24 io_kind=connection attempts=1]"
        );

        let history = [
            matching,
            planning_transport_observation("metadata_prefix", "NISAR_SUBSET_V2_PARTIAL_RESPONSE"),
        ];
        let result = nisar_v2_planning_failure(
            &work,
            PathBuf::from("output"),
            crate::NisarSubsetPlanningErrorV2::BootstrapTransport,
            history.last(),
            &WorkerControl::default(),
        );
        assert_eq!(
            result.failure.unwrap().reason_zh,
            "NISAR V2 Range transport failed"
        );

        let untrusted_stage = planning_transport_observation(
            "https://secret.example/?token=SECRET Authorization: Bearer SECRET",
            "NISAR_SUBSET_V2_TRANSPORT_FAILED",
        );
        let result = nisar_v2_planning_failure(
            &work,
            PathBuf::from("output"),
            crate::NisarSubsetPlanningErrorV2::BootstrapTransport,
            Some(&untrusted_stage),
            &WorkerControl::default(),
        );
        let reason = result.failure.unwrap().reason_zh;
        assert_eq!(reason, "NISAR V2 Range transport failed");
        for forbidden in ["https://", "token=", "Authorization", "SECRET"] {
            assert!(!reason.contains(forbidden));
        }
    }

    #[test]
    fn nisar_v2_planning_transport_failure_survives_control_race() {
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let observation = planning_transport_observation(
            "metadata_discovery",
            "NISAR_SUBSET_V2_TRANSPORT_FAILED",
        );
        for cancelled in [false, true] {
            let control = WorkerControl::default();
            if cancelled {
                control.cancel_for_test();
            } else {
                control.set_paused_for_test(true);
            }
            let result = nisar_v2_planning_failure(
                &work,
                PathBuf::from("output"),
                crate::NisarSubsetPlanningErrorV2::BootstrapTransport,
                Some(&observation),
                &control,
            );
            assert_eq!(result.outcome, DownloadOutcome::Failed);
            let failure = result
                .failure
                .expect("real transport failure must remain visible");
            assert_eq!(failure.code, "NISAR_SUBSET_V2_TRANSPORT_FAILED");
            assert!(failure.reason_zh.contains("transport_cause="));
            assert!(failure.reason_zh.contains("http_status=503"));
        }
    }

    #[test]
    fn nisar_v2_explicit_stop_errors_are_lifecycle_only() {
        use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as ScienceError;

        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        let paused = WorkerControl::default();
        paused.set_paused_for_test(true);
        let cancelled = WorkerControl::default();
        cancelled.cancel_for_test();
        let results = [
            nisar_v2_planning_failure(
                &work,
                PathBuf::from("output"),
                crate::NisarSubsetPlanningErrorV2::ExecutionStopped,
                None,
                &paused,
            ),
            super::nisar_v2_science_failure(
                &work,
                PathBuf::from("output"),
                ScienceError::Paused,
                &paused,
            ),
            super::nisar_v2_science_failure(
                &work,
                PathBuf::from("output"),
                ScienceError::Cancelled,
                &cancelled,
            ),
            nisar_v2_writer_failure(
                &work,
                PathBuf::from("output"),
                crate::NisarSubsetWriteError::Cancelled,
                None,
                &paused,
            ),
        ];
        assert_eq!(
            results
                .iter()
                .map(|result| result.outcome.clone())
                .collect::<Vec<_>>(),
            vec![
                DownloadOutcome::Interrupted,
                DownloadOutcome::Interrupted,
                DownloadOutcome::Cancelled,
                DownloadOutcome::Interrupted,
            ]
        );
        assert!(results.iter().all(|result| result.failure.is_none()));
    }

    #[test]
    fn nisar_v2_transport_failures_allow_retry_but_scientific_failures_remain_terminal() {
        use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as Error;
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), std::path::Path::new("output"));
        let work = DownloadWork { task, item };
        for error in [Error::Transport, Error::ScienceTransferIncomplete] {
            let result = super::nisar_v2_science_failure(
                &work,
                PathBuf::from("output"),
                error,
                &WorkerControl::default(),
            );
            assert_eq!(result.outcome, DownloadOutcome::Failed);
            let failure = result.failure.unwrap();
            assert_eq!(failure.code, "NISAR_SUBSET_V2_SCIENCE_INCOMPLETE");
            assert!(failure.retryable);
        }
        for error in [
            Error::StrongValidatorChanged,
            Error::AuthorityMismatch,
            Error::RecoveryInvalid,
            Error::RangeUnsupported,
            Error::RangeResponseInvalid,
            Error::ResponseInvalid,
            Error::CombinedBudgetExceeded,
            Error::Persistence,
            Error::AllocationEvidenceUnsupported,
        ] {
            let result = super::nisar_v2_science_failure(
                &work,
                PathBuf::from("output"),
                error,
                &WorkerControl::default(),
            );
            assert_eq!(result.outcome, DownloadOutcome::Failed);
            assert!(!result.failure.unwrap().retryable);
        }
    }

    struct BurstWorkerCredentials;

    impl CredentialProvider for BurstWorkerCredentials {
        fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
            assert_eq!(profile, EARTHDATA_BURST_CREDENTIAL_PROFILE);
            CredentialMaterial::basic("fixture-user", "fixture-password")
        }
    }

    struct BurstWorkerLoopbackResolver {
        base: String,
    }

    impl AssetResolver for BurstWorkerLoopbackResolver {
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
            reference: &StableAssetReference,
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
                .map_err(|_| ResolveError::auth_failed())?;
            let file_name = reference
                .asset_key
                .rsplit(':')
                .next()
                .filter(|value| matches!(*value, BURST_TIFF_NAME | BURST_XML_NAME))
                .ok_or_else(ResolveError::reference_invalid)?;
            let mut resolved =
                ResolvedAsset::new(&format!("{}/{file_name}", self.base), ResolvedMethod::Get)?
                    .with_suggested_filename(Some(file_name.to_owned()))
                    .with_host_policy(ResolvedHostPolicy::EarthdataBurstLoopbackFixture);
            credential.apply_to(&mut resolved)?;
            Ok(resolved)
        }

        fn plan_shadow(
            &self,
            _frozen: &insar_core::FrozenAssetAccess,
            _context: &ShadowResolveContext<'_>,
        ) -> Result<ShadowAccessIntent, ResolveError> {
            Err(ResolveError::descriptor_invalid())
        }
    }

    pub(super) fn burst_loopback_worker(
        base: &str,
        options: TransferOptions,
    ) -> (ProductionDownloadWorker, Arc<TransferAdmissionController>) {
        let mut registry = AssetResolverRegistry::new();
        registry
            .register(Arc::new(BurstWorkerLoopbackResolver {
                base: base.to_owned(),
            }))
            .unwrap();
        let runtime = ResolverExecutionRuntime::with_dependencies(
            registry,
            Arc::new(BurstWorkerCredentials),
            Arc::new(SystemClock),
        );
        let mut transfer = HttpTransfer::new_without_system_proxy(options).unwrap();
        transfer
            .set_resolved_earthdata_session_endpoints_for_test(
                &format!("{base}/token"),
                &format!("{base}/authorize"),
                &format!("{base}/"),
            )
            .unwrap();
        let admission = TransferAdmissionController::compatibility();
        let worker = ProductionDownloadWorker::new(TransferOptions::default())
            .unwrap()
            .with_resolver_runtime_and_transfer(runtime, Arc::new(transfer))
            .with_transfer_admission_controller_for_test(Arc::clone(&admission));
        (worker, admission)
    }

    #[test]
    fn burst_resolver_authoritative_worker_completes_exact_tiff_and_xml_session_chain() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let captured = Arc::new(Mutex::new(Vec::new()));
        let server_requests = Arc::clone(&captured);
        let mut tiff = vec![0_u8; 1_024];
        tiff[..14].copy_from_slice(b"II*\0\x08\0\0\0\0\0\0\0\0\0");
        let expected_tiff = tiff.clone();
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?><burst:product xmlns:burst="urn:asf:burst"><burst:id>S1</burst:id></burst:product>"#
            .to_vec();
        let expected_xml = xml.clone();

        let output_root =
            std::env::temp_dir().join(format!("insar-burst-step-7-worker-{}", std::process::id()));
        let _ = fs::remove_file(
            output_root
                .join(BURST_NATIVE_PRODUCT_ID)
                .join(BURST_TIFF_NAME),
        );
        let _ = fs::remove_file(
            output_root
                .join(BURST_NATIVE_PRODUCT_ID)
                .join(BURST_XML_NAME),
        );
        let _ = fs::remove_file(part_path(
            &output_root
                .join(BURST_NATIVE_PRODUCT_ID)
                .join(BURST_TIFF_NAME),
        ));
        let _ = fs::remove_file(part_path(
            &output_root
                .join(BURST_NATIVE_PRODUCT_ID)
                .join(BURST_XML_NAME),
        ));
        let items = [
            burst_worker_item(AssetRole::Primary),
            burst_worker_item(AssetRole::Metadata),
        ];
        let task = burst_worker_task(&items, &output_root);
        let mut registry = AssetResolverRegistry::new();
        registry
            .register(Arc::new(BurstWorkerLoopbackResolver { base: base.clone() }))
            .unwrap();
        let runtime = ResolverExecutionRuntime::with_dependencies(
            registry,
            Arc::new(BurstWorkerCredentials),
            Arc::new(SystemClock),
        );
        let mut transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();
        transfer
            .set_resolved_earthdata_session_endpoints_for_test(
                &format!("{base}/token"),
                &format!("{base}/authorize"),
                &format!("{base}/"),
            )
            .unwrap();
        let transfer: Arc<dyn TransferBackend> = Arc::new(transfer);
        let admission = TransferAdmissionController::compatibility();
        let worker = ProductionDownloadWorker::new(TransferOptions::default())
            .unwrap()
            .with_resolver_runtime_and_transfer(runtime, transfer)
            .with_transfer_admission_controller_for_test(Arc::clone(&admission));

        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while server_requests.lock().unwrap().len() < 4 && Instant::now() < deadline {
                let (stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("loopback accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    request.push_str(&line);
                }
                let request_line = request.lines().next().unwrap_or_default().to_owned();
                server_requests.lock().unwrap().push(request);
                let mut stream = reader.into_inner();
                if request_line.starts_with("POST /token ") {
                    let body = br#"{"access_token":"fixture-token"}"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(body).unwrap();
                } else if request_line.starts_with("GET /authorize ") {
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                } else {
                    let body = if request_line.starts_with("GET /measurement.tiff ") {
                        tiff.as_slice()
                    } else if request_line.starts_with("GET /metadata.xml ") {
                        xml.as_slice()
                    } else {
                        b"unexpected".as_slice()
                    };
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(body).unwrap();
                }
                stream.flush().unwrap();
            }
        });

        let results = items
            .iter()
            .cloned()
            .map(|item| {
                worker.execute(
                    DownloadWork {
                        task: task.clone(),
                        item,
                    },
                    WorkerControl::default(),
                    &mut |_| {},
                )
            })
            .collect::<Vec<_>>();

        server.join().unwrap();
        assert_eq!(
            results
                .iter()
                .map(|result| result.outcome.clone())
                .collect::<Vec<_>>(),
            [DownloadOutcome::Completed, DownloadOutcome::Completed]
        );
        assert!(results.iter().all(|result| result.attempt_count == 1));
        assert_eq!(results[0].bytes_written, 1_024);
        assert_eq!(results[1].bytes_written, expected_xml.len() as u64);
        assert_eq!(
            fs::read(
                output_root
                    .join(BURST_NATIVE_PRODUCT_ID)
                    .join(BURST_TIFF_NAME)
            )
            .unwrap(),
            expected_tiff
        );
        assert_eq!(
            fs::read(
                output_root
                    .join(BURST_NATIVE_PRODUCT_ID)
                    .join(BURST_XML_NAME)
            )
            .unwrap(),
            expected_xml
        );
        let requests = Arc::try_unwrap(captured).unwrap().into_inner().unwrap();
        // One shared Earthdata session serves both artifacts (W1 batch 2).
        assert_eq!(
            requests.len(),
            4,
            "gate or fallback changed the request chain"
        );
        let normalized = requests
            .iter()
            .map(|request| request.to_ascii_lowercase())
            .collect::<Vec<_>>();
        assert_eq!(
            normalized
                .iter()
                .filter(|request| request.starts_with("post /token "))
                .count(),
            1
        );
        assert_eq!(
            normalized
                .iter()
                .filter(|request| request.starts_with("get /authorize "))
                .count(),
            1
        );
        assert!(normalized.iter().any(|request| {
            request.starts_with("get /measurement.tiff ")
                && request.contains("cookie: asf-urs=fixture-session")
                && request.contains("authorization: bearer fixture-token")
        }));
        assert!(normalized.iter().any(|request| {
            request.starts_with("get /metadata.xml ")
                && request.contains("cookie: asf-urs=fixture-session")
                && request.contains("authorization: bearer fixture-token")
        }));
        assert!(normalized
            .iter()
            .all(|request| !request.contains(".zip") && !request.contains("scene")));
        let admission = admission.snapshot();
        assert_eq!(admission.grants, 2);
        assert_eq!(admission.releases, 2);
        let _ = fs::remove_file(
            output_root
                .join(BURST_NATIVE_PRODUCT_ID)
                .join(BURST_TIFF_NAME),
        );
        let _ = fs::remove_file(
            output_root
                .join(BURST_NATIVE_PRODUCT_ID)
                .join(BURST_XML_NAME),
        );
    }

    #[test]
    fn burst_worker_maps_rejected_or_cookieless_session_to_cookie_required() {
        for (case, reject_token, expected_requests) in
            [("rejected", true, 1_usize), ("cookieless", false, 2_usize)]
        {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let output_root = std::env::temp_dir().join(format!(
                "insar-burst-step-7-worker-{case}-{}",
                std::process::id()
            ));
            let destination = output_root
                .join(BURST_NATIVE_PRODUCT_ID)
                .join(BURST_TIFF_NAME);
            let _ = fs::remove_file(&destination);
            let _ = fs::remove_file(part_path(&destination));
            let item = burst_worker_item(AssetRole::Primary);
            let task = burst_worker_task(std::slice::from_ref(&item), &output_root);
            let mut registry = AssetResolverRegistry::new();
            registry
                .register(Arc::new(BurstWorkerLoopbackResolver { base: base.clone() }))
                .unwrap();
            let runtime = ResolverExecutionRuntime::with_dependencies(
                registry,
                Arc::new(BurstWorkerCredentials),
                Arc::new(SystemClock),
            );
            let mut transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
                max_attempts: 1,
                backoff: Duration::ZERO,
                timeout: Duration::from_secs(2),
                ..TransferOptions::default()
            })
            .unwrap();
            transfer
                .set_resolved_earthdata_session_endpoints_for_test(
                    &format!("{base}/token"),
                    &format!("{base}/authorize"),
                    &format!("{base}/"),
                )
                .unwrap();
            let worker = ProductionDownloadWorker::new(TransferOptions::default())
                .unwrap()
                .with_resolver_runtime_and_transfer(runtime, Arc::new(transfer));

            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);
            let server = thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(3);
                while server_requests.lock().unwrap().len() < expected_requests
                    && Instant::now() < deadline
                {
                    let (stream, _) = match listener.accept() {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        Err(error) => panic!("loopback accept failed: {error}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut request = String::new();
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        request.push_str(&line);
                    }
                    let request_line = request.lines().next().unwrap_or_default().to_owned();
                    server_requests.lock().unwrap().push(request);
                    let mut stream = reader.into_inner();
                    if reject_token {
                        write!(
                            stream,
                            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .unwrap();
                    } else if request_line.starts_with("POST /token ") {
                        let body = br#"{"access_token":"fixture-token"}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(body).unwrap();
                    } else {
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .unwrap();
                    }
                    stream.flush().unwrap();
                }
            });

            let result = worker.execute(
                DownloadWork { task, item },
                WorkerControl::default(),
                &mut |_| {},
            );

            server.join().unwrap();
            assert_eq!(result.outcome, DownloadOutcome::Failed, "{case}");
            assert_eq!(
                result.failure.as_ref().map(|failure| failure.code.as_str()),
                Some("ASF_BURST_COOKIE_REQUIRED"),
                "{case}"
            );
            assert_eq!(result.attempt_count, 0, "{case}");
            assert!(!destination.exists(), "{case}");
            assert!(!part_path(&destination).exists(), "{case}");
            assert_eq!(requests.lock().unwrap().len(), expected_requests, "{case}");
        }
    }

    #[test]
    fn burst_worker_retries_content_inside_one_admission_and_session() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let output_root = std::env::temp_dir().join(format!(
            "insar-burst-step-7-worker-retry-{}",
            std::process::id()
        ));
        let destination = output_root
            .join(BURST_NATIVE_PRODUCT_ID)
            .join(BURST_TIFF_NAME);
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(part_path(&destination));
        let item = burst_worker_item(AssetRole::Primary);
        let task = burst_worker_task(std::slice::from_ref(&item), &output_root);
        let (worker, admission) = burst_loopback_worker(
            &base,
            TransferOptions {
                max_attempts: 2,
                backoff: Duration::ZERO,
                timeout: Duration::from_secs(2),
                ..TransferOptions::default()
            },
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_requests = Arc::clone(&requests);
        let content_attempts = Arc::new(AtomicUsize::new(0));
        let server_content_attempts = Arc::clone(&content_attempts);
        let mut tiff = vec![0_u8; 1_024];
        tiff[..14].copy_from_slice(b"II*\0\x08\0\0\0\0\0\0\0\0\0");
        let expected_tiff = tiff.clone();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            while server_requests.lock().unwrap().len() < 4 && Instant::now() < deadline {
                let (stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("loopback accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    request.push_str(&line);
                }
                let request_line = request.lines().next().unwrap_or_default().to_owned();
                server_requests.lock().unwrap().push(request);
                let mut stream = reader.into_inner();
                if request_line.starts_with("POST /token ") {
                    let body = br#"{"access_token":"fixture-token"}"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(body).unwrap();
                } else if request_line.starts_with("GET /authorize ") {
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                } else if server_content_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    write!(
                        stream,
                        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                } else {
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: image/tiff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        tiff.len()
                    )
                    .unwrap();
                    stream.write_all(&tiff).unwrap();
                }
                stream.flush().unwrap();
            }
        });

        let result = worker.execute(
            DownloadWork { task, item },
            WorkerControl::default(),
            &mut |_| {},
        );

        server.join().unwrap();
        assert_eq!(result.outcome, DownloadOutcome::Completed);
        assert_eq!(result.attempt_count, 2);
        assert_eq!(result.bytes_written, 1_024);
        assert_eq!(fs::read(&destination).unwrap(), expected_tiff);
        assert_eq!(content_attempts.load(Ordering::SeqCst), 2);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("POST /token "))
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("GET /authorize "))
                .count(),
            1
        );
        let admission = admission.snapshot();
        assert_eq!(admission.grants, 1);
        assert_eq!(admission.releases, 1);
        let _ = fs::remove_file(destination);
    }

    #[test]
    fn burst_worker_pause_resume_completes_and_cancel_retains_safe_part() {
        for cancel in [false, true] {
            let case = if cancel { "cancel" } else { "pause-resume" };
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let output_root = std::env::temp_dir().join(format!(
                "insar-burst-step-7-worker-{case}-{}",
                std::process::id()
            ));
            let destination = output_root
                .join(BURST_NATIVE_PRODUCT_ID)
                .join(BURST_TIFF_NAME);
            let part = part_path(&destination);
            let _ = fs::remove_file(&destination);
            let _ = fs::remove_file(&part);
            let item = burst_worker_item(AssetRole::Primary);
            let task = burst_worker_task(std::slice::from_ref(&item), &output_root);
            let (worker, admission) = burst_loopback_worker(
                &base,
                TransferOptions {
                    max_attempts: 1,
                    backoff: Duration::ZERO,
                    timeout: Duration::from_secs(2),
                    chunk_size: 16,
                    ..TransferOptions::default()
                },
            );
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);
            let allow_rest = Arc::new(AtomicBool::new(false));
            let server_allow_rest = Arc::clone(&allow_rest);
            let mut tiff = vec![0_u8; 1_024];
            tiff[..14].copy_from_slice(b"II*\0\x08\0\0\0\0\0\0\0\0\0");
            let expected_tiff = tiff.clone();
            let server = thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(3);
                while server_requests.lock().unwrap().len() < 3 && Instant::now() < deadline {
                    let (stream, _) = match listener.accept() {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        Err(error) => panic!("loopback accept failed: {error}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut request = String::new();
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        request.push_str(&line);
                    }
                    let request_line = request.lines().next().unwrap_or_default().to_owned();
                    server_requests.lock().unwrap().push(request);
                    let mut stream = reader.into_inner();
                    if request_line.starts_with("POST /token ") {
                        let body = br#"{"access_token":"fixture-token"}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(body).unwrap();
                    } else if request_line.starts_with("GET /authorize ") {
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .unwrap();
                    } else {
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: image/tiff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            tiff.len()
                        )
                        .unwrap();
                        stream.write_all(&tiff[..16]).unwrap();
                        stream.flush().unwrap();
                        let rest_deadline = Instant::now() + Duration::from_secs(2);
                        while !server_allow_rest.load(Ordering::SeqCst)
                            && Instant::now() < rest_deadline
                        {
                            thread::sleep(Duration::from_millis(5));
                        }
                        if cancel {
                            thread::sleep(Duration::from_millis(100));
                        }
                        let _ = stream.write_all(&tiff[16..]);
                    }
                    let _ = stream.flush();
                }
            });
            let control = WorkerControl::default();
            let callback_control = control.clone();
            let callback_allow_rest = Arc::clone(&allow_rest);
            let triggered = Arc::new(AtomicBool::new(false));
            let callback_triggered = Arc::clone(&triggered);
            let resumed = Arc::new(AtomicBool::new(false));
            let callback_resumed = Arc::clone(&resumed);
            let started = Instant::now();

            let result =
                worker.execute(DownloadWork { task, item }, control, &mut move |progress| {
                    if progress.bytes_completed == 0
                        || callback_triggered.swap(true, Ordering::SeqCst)
                    {
                        return;
                    }
                    if cancel {
                        callback_control.cancel_for_test();
                        callback_allow_rest.store(true, Ordering::SeqCst);
                    } else {
                        callback_control.set_paused_for_test(true);
                        let resume_control = callback_control.clone();
                        let resume_allow_rest = Arc::clone(&callback_allow_rest);
                        let resume_observed = Arc::clone(&callback_resumed);
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(120));
                            resume_control.set_paused_for_test(false);
                            resume_observed.store(true, Ordering::SeqCst);
                            resume_allow_rest.store(true, Ordering::SeqCst);
                        });
                    }
                });

            server.join().unwrap();
            assert!(triggered.load(Ordering::SeqCst), "{case}");
            assert_eq!(requests.lock().unwrap().len(), 3, "{case}");
            let admission = admission.snapshot();
            assert_eq!(admission.grants, 1, "{case}");
            assert_eq!(admission.releases, 1, "{case}");
            assert_eq!(result.attempt_count, 1, "{case}");
            if cancel {
                assert_eq!(result.outcome, DownloadOutcome::Cancelled);
                assert!(!destination.exists());
                assert_eq!(fs::metadata(&part).unwrap().len(), 16);
            } else {
                assert!(resumed.load(Ordering::SeqCst));
                assert!(started.elapsed() >= Duration::from_millis(100));
                assert_eq!(result.outcome, DownloadOutcome::Completed);
                assert_eq!(fs::read(&destination).unwrap(), expected_tiff);
                assert!(!part.exists());
            }
            let _ = fs::remove_file(destination);
            let _ = fs::remove_file(part);
        }
    }

    #[derive(Default)]
    struct SeasatRecordingBackend {
        requests: Mutex<Vec<(PathBuf, Option<u64>)>>,
    }

    impl TransferBackend for SeasatRecordingBackend {
        fn kind(&self) -> TransferBackendKind {
            TransferBackendKind::NativeSingleStream
        }

        fn is_available(&self) -> bool {
            true
        }

        fn download_legacy(
            &self,
            request: &TransferRequest,
            _auth: &TransferAuth,
            _cancelled: &(dyn Fn() -> bool + Sync),
            _paused: &(dyn Fn() -> bool + Sync),
            _progress: &mut dyn FnMut(TransferUpdate),
        ) -> TransferResult {
            self.requests
                .lock()
                .unwrap()
                .push((request.destination.clone(), request.expected_size));
            TransferResult {
                outcome: DownloadOutcome::Completed,
                bytes_written: request.expected_size.unwrap_or_default(),
                attempt_count: 1,
                failure: None,
            }
        }

        fn resolved_session(&self) -> ResolvedTransferSession {
            ResolvedTransferSession::new(1).unwrap()
        }

        fn download_resolved(
            &self,
            _request: &TransferRequest,
            _asset: &ResolvedAsset,
            _policy: AuthRefreshPolicyId,
            _session: &mut ResolvedTransferSession,
            _cancelled: &(dyn Fn() -> bool + Sync),
            _paused: &(dyn Fn() -> bool + Sync),
            _progress: &mut dyn FnMut(TransferUpdate),
        ) -> ResolvedTransferResult {
            panic!("SEASAT legacy-primary test must not enter resolver execution")
        }
    }

    #[test]
    fn production_worker_accepts_direct_cdse_and_system_bhoonidhi_without_losing_session_sharing() {
        let direct = ProviderNetworkPolicy::direct();
        let system = ProviderNetworkPolicy::system();
        let cdse = Arc::new(CdseNetworkFoundation::new(direct).oauth_session().unwrap());
        let endpoint = Arc::new(crate::ReqwestBhoonidhiTokenEndpoint::new(system).unwrap());
        let bhoonidhi = Arc::new(
            crate::BhoonidhiSession::new(system, endpoint, Arc::new(SystemClock)).unwrap(),
        );
        let provider = Arc::new(
            crate::BhoonidhiSessionTokenProvider::new(Arc::clone(&bhoonidhi)).with_desktop_login(),
        );
        let worker = ProductionDownloadWorker::new_with_cdse_and_bhoonidhi_provider(
            TransferOptions::default(),
            cdse,
            direct,
            Arc::clone(&bhoonidhi),
            provider,
            Duration::from_secs(45),
            TransferAdmissionController::compatibility(),
        )
        .expect("both independently routed sessions should share one worker");
        assert!(worker.shares_bhoonidhi_session(&bhoonidhi));
        assert_eq!(worker.resolver_runtime.network_policy(), direct);
        assert_eq!(
            worker.resolver_runtime.bhoonidhi_network_policy(),
            Some(system)
        );
        assert_eq!(
            worker.bhoonidhi_transfer.as_ref().unwrap().proxy_policy(),
            ProxyPolicy::System
        );
        assert_eq!(
            worker.cdse_transfer.as_ref().unwrap().proxy_policy(),
            ProxyPolicy::Direct
        );
    }

    #[test]
    fn desktop_direct_policy_routes_ordinary_transfer_without_changing_bhoonidhi() {
        let direct = ProviderNetworkPolicy::direct();
        let system = ProviderNetworkPolicy::system();
        let cdse = Arc::new(CdseNetworkFoundation::new(direct).oauth_session().unwrap());
        let endpoint = Arc::new(crate::ReqwestBhoonidhiTokenEndpoint::new(system).unwrap());
        let bhoonidhi = Arc::new(
            crate::BhoonidhiSession::new(system, endpoint, Arc::new(SystemClock)).unwrap(),
        );
        let provider = Arc::new(
            crate::BhoonidhiSessionTokenProvider::new(Arc::clone(&bhoonidhi)).with_desktop_login(),
        );
        let worker = ProductionDownloadWorker::new_with_cdse_and_bhoonidhi_provider(
            TransferOptions::default(),
            cdse,
            direct,
            bhoonidhi,
            provider,
            Duration::from_secs(45),
            TransferAdmissionController::compatibility(),
        )
        .expect("production worker");
        assert_eq!(worker.transfer.proxy_policy(), ProxyPolicy::Direct);
        assert_eq!(
            worker.bhoonidhi_transfer.as_ref().unwrap().proxy_policy(),
            ProxyPolicy::System
        );
    }

    #[test]
    fn desktop_manual_policy_routes_every_ordinary_transfer_through_the_proxy() {
        let manual =
            ProviderNetworkPolicy::manual(insar_core::ManualProxy::new("127.0.0.1", 3128).unwrap());
        let system = ProviderNetworkPolicy::system();
        let cdse = Arc::new(CdseNetworkFoundation::new(manual).oauth_session().unwrap());
        let endpoint = Arc::new(crate::ReqwestBhoonidhiTokenEndpoint::new(system).unwrap());
        let bhoonidhi = Arc::new(
            crate::BhoonidhiSession::new(system, endpoint, Arc::new(SystemClock)).unwrap(),
        );
        let provider = Arc::new(
            crate::BhoonidhiSessionTokenProvider::new(Arc::clone(&bhoonidhi)).with_desktop_login(),
        );
        let worker = ProductionDownloadWorker::new_with_cdse_and_bhoonidhi_provider(
            TransferOptions::default(),
            cdse,
            manual,
            bhoonidhi,
            provider,
            Duration::from_secs(45),
            TransferAdmissionController::compatibility(),
        )
        .expect("production worker");
        assert_eq!(worker.transfer.proxy_policy(), manual.route());
        assert_eq!(worker.mpc_transfer.proxy_policy(), manual.route());
        assert_eq!(
            worker.cdse_transfer.as_ref().unwrap().proxy_policy(),
            manual.route()
        );
        assert_eq!(
            worker.bhoonidhi_transfer.as_ref().unwrap().proxy_policy(),
            ProxyPolicy::System
        );
    }

    #[test]
    fn nisar_subset_transfer_selects_bhoonidhi_route_without_altering_earthdata() {
        let direct = ProviderNetworkPolicy::direct();
        let system = ProviderNetworkPolicy::system();
        let cdse = Arc::new(CdseNetworkFoundation::new(direct).oauth_session().unwrap());
        let endpoint = Arc::new(crate::ReqwestBhoonidhiTokenEndpoint::new(system).unwrap());
        let bhoonidhi = Arc::new(
            crate::BhoonidhiSession::new(system, endpoint, Arc::new(SystemClock)).unwrap(),
        );
        let provider = Arc::new(
            crate::BhoonidhiSessionTokenProvider::new(Arc::clone(&bhoonidhi)).with_desktop_login(),
        );
        let worker = ProductionDownloadWorker::new_with_cdse_and_bhoonidhi_provider(
            TransferOptions::default(),
            cdse,
            direct,
            bhoonidhi,
            provider,
            Duration::from_secs(45),
            TransferAdmissionController::compatibility(),
        )
        .expect("production worker");

        assert_eq!(
            worker
                .nisar_subset_transfer(insar_provider::catalog::BHOONIDHI_PROVIDER_ID)
                .proxy_policy(),
            ProxyPolicy::System
        );
        assert_eq!(
            worker
                .nisar_subset_transfer(insar_provider::catalog::NISAR_PROVIDER_ID)
                .proxy_policy(),
            ProxyPolicy::Direct
        );

        let manual =
            ProviderNetworkPolicy::manual(insar_core::ManualProxy::new("127.0.0.1", 7897).unwrap());
        let manual_cdse = Arc::new(CdseNetworkFoundation::new(manual).oauth_session().unwrap());
        let manual_endpoint = Arc::new(crate::ReqwestBhoonidhiTokenEndpoint::new(manual).unwrap());
        let manual_bhoonidhi = Arc::new(
            crate::BhoonidhiSession::new(manual, manual_endpoint, Arc::new(SystemClock)).unwrap(),
        );
        let manual_provider = Arc::new(
            crate::BhoonidhiSessionTokenProvider::new(Arc::clone(&manual_bhoonidhi))
                .with_desktop_login(),
        );
        let manual_worker = ProductionDownloadWorker::new_with_cdse_and_bhoonidhi_provider(
            TransferOptions::default(),
            manual_cdse,
            manual,
            manual_bhoonidhi,
            manual_provider,
            Duration::from_secs(45),
            TransferAdmissionController::compatibility(),
        )
        .expect("manual production worker");
        assert_eq!(
            manual_worker
                .nisar_subset_transfer(insar_provider::catalog::BHOONIDHI_PROVIDER_ID)
                .proxy_policy(),
            manual.route()
        );
        assert_eq!(
            manual_worker
                .nisar_subset_transfer(insar_provider::catalog::NISAR_PROVIDER_ID)
                .proxy_policy(),
            manual.route()
        );
    }

    #[test]
    fn new_with_bhoonidhi_session_initializes_bhoonidhi_subset_transfer() {
        let manual =
            ProviderNetworkPolicy::manual(insar_core::ManualProxy::new("127.0.0.1", 7897).unwrap());
        let endpoint = Arc::new(crate::ReqwestBhoonidhiTokenEndpoint::new(manual).unwrap());
        let bhoonidhi = Arc::new(
            crate::BhoonidhiSession::new(manual, endpoint, Arc::new(SystemClock)).unwrap(),
        );
        let worker = ProductionDownloadWorker::new_with_bhoonidhi_session(
            TransferOptions::default(),
            bhoonidhi,
            Duration::from_secs(45),
            TransferAdmissionController::compatibility(),
        )
        .expect("bhoonidhi-session worker");
        assert_eq!(
            worker.bhoonidhi_transfer.as_ref().unwrap().proxy_policy(),
            manual.route()
        );
        assert_eq!(
            worker
                .nisar_subset_transfer(insar_provider::catalog::BHOONIDHI_PROVIDER_ID)
                .proxy_policy(),
            manual.route()
        );
        assert!(std::ptr::eq(
            worker.resolver_transfer(crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1)
                as *const dyn TransferBackend as *const (),
            Arc::as_ptr(worker.bhoonidhi_transfer.as_ref().unwrap()) as *const (),
        ));
    }

    #[test]
    fn new_with_cdse_and_bhoonidhi_sessions_initializes_both_routes() {
        let direct = ProviderNetworkPolicy::direct();
        let system = ProviderNetworkPolicy::system();
        let cdse = Arc::new(CdseNetworkFoundation::new(direct).oauth_session().unwrap());
        let endpoint = Arc::new(crate::ReqwestBhoonidhiTokenEndpoint::new(system).unwrap());
        let bhoonidhi = Arc::new(
            crate::BhoonidhiSession::new(system, endpoint, Arc::new(SystemClock)).unwrap(),
        );
        let worker = ProductionDownloadWorker::new_with_cdse_and_bhoonidhi_sessions(
            TransferOptions::default(),
            cdse,
            direct,
            bhoonidhi,
            Duration::from_secs(45),
            TransferAdmissionController::compatibility(),
        )
        .expect("sessions worker");
        assert_eq!(worker.transfer.proxy_policy(), ProxyPolicy::Direct);
        assert_eq!(
            worker.cdse_transfer.as_ref().unwrap().proxy_policy(),
            ProxyPolicy::Direct
        );
        assert_eq!(
            worker
                .nisar_subset_transfer(insar_provider::catalog::BHOONIDHI_PROVIDER_ID)
                .proxy_policy(),
            ProxyPolicy::System
        );
    }

    #[test]
    fn cdse_worker_uses_the_selected_route_without_changing_asf_transfer_routing() {
        for policy in [
            ProviderNetworkPolicy::system(),
            ProviderNetworkPolicy::direct(),
        ] {
            let session = Arc::new(
                CdseNetworkFoundation::new(policy)
                    .oauth_session()
                    .expect("empty runtime CDSE session"),
            );
            let worker = ProductionDownloadWorker::new_with_cdse_session(
                TransferOptions::default(),
                session,
                policy,
            )
            .expect("production worker with shared CDSE session");

            assert_eq!(worker.transfer.proxy_policy(), ProxyPolicy::System);
            assert_eq!(
                worker
                    .cdse_transfer
                    .as_ref()
                    .expect("CDSE transfer is configured")
                    .proxy_policy(),
                policy.route()
            );
            assert_eq!(worker.resolver_runtime.network_policy(), policy);
        }
    }

    #[test]
    fn planetary_computer_sas_uses_direct_data_plane_without_changing_other_routes() {
        for policy in [
            ProviderNetworkPolicy::system(),
            ProviderNetworkPolicy::direct(),
        ] {
            let session = Arc::new(
                CdseNetworkFoundation::new(policy)
                    .oauth_session()
                    .expect("empty runtime CDSE session"),
            );
            let worker = ProductionDownloadWorker::new_with_cdse_session(
                TransferOptions::default(),
                session,
                policy,
            )
            .expect("production worker with CDSE route plus MPC data plane");

            assert_eq!(worker.transfer.proxy_policy(), ProxyPolicy::System);
            assert_eq!(worker.mpc_transfer.proxy_policy(), ProxyPolicy::Direct);
            assert_eq!(
                worker
                    .cdse_transfer
                    .as_ref()
                    .expect("CDSE transfer is configured")
                    .proxy_policy(),
                policy.route()
            );
            assert_eq!(
                worker
                    .resolver_transfer(crate::AuthRefreshPolicyId::PlanetaryComputerSasV1)
                    .kind(),
                TransferBackendKind::NativeSingleStream
            );
        }
    }

    #[test]
    fn landsat_resolver_is_registered_only_with_the_injected_shared_usgs_session() {
        let session = Arc::new(
            UsgsM2mSession::new(ProviderNetworkPolicy::system(), Duration::from_secs(1))
                .expect("offline USGS session construction"),
        );
        let standard = ProductionDownloadWorker::new(TransferOptions::default())
            .expect("standard production worker");
        assert!(!standard.shares_usgs_session(&session));

        let worker = ProductionDownloadWorker::new_with_usgs_session(
            TransferOptions::default(),
            Arc::clone(&session),
        )
        .expect("worker with shared USGS session");
        assert!(worker.shares_usgs_session(&session));
        let composed = ProductionDownloadWorker::new_with_cdse_session(
            TransferOptions::default(),
            Arc::new(
                CdseNetworkFoundation::new(ProviderNetworkPolicy::system())
                    .oauth_session()
                    .unwrap(),
            ),
            ProviderNetworkPolicy::system(),
        )
        .unwrap()
        .with_usgs_session(Arc::clone(&session))
        .unwrap();
        assert!(composed.shares_usgs_session(&session));
        assert!(composed.cdse_transfer.is_some());
    }

    #[test]
    fn usgs_signed_url_transfer_uses_the_session_route_without_changing_native_transfer() {
        for policy in [
            ProviderNetworkPolicy::system(),
            ProviderNetworkPolicy::direct(),
        ] {
            let session = Arc::new(
                UsgsM2mSession::new(policy, Duration::from_secs(1))
                    .expect("offline USGS session construction"),
            );
            let worker = ProductionDownloadWorker::new_with_usgs_session(
                TransferOptions::default(),
                session,
            )
            .expect("worker with USGS data-plane route");

            assert_eq!(worker.transfer.proxy_policy(), ProxyPolicy::System);
            assert_eq!(
                worker
                    .usgs_transfer
                    .as_ref()
                    .expect("USGS transfer is configured")
                    .proxy_policy(),
                policy.route()
            );
            assert_eq!(
                worker
                    .resolver_transfer(crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1)
                    .kind(),
                TransferBackendKind::NativeSingleStream
            );
        }
    }

    #[test]
    fn routes_downloads_by_provider_instead_of_directory_layout() {
        assert_eq!(
            DownloadProviderKind::from_provider_id("asf.sentinel1"),
            Some(DownloadProviderKind::AsfSentinel1)
        );
        assert_eq!(
            DownloadProviderKind::from_provider_id("asf.catalog"),
            Some(DownloadProviderKind::AsfCatalog)
        );
        assert_eq!(
            DownloadProviderKind::from_provider_id("esa.sentinel1_orbit"),
            Some(DownloadProviderKind::Orbit)
        );
        assert_eq!(
            DownloadProviderKind::from_provider_id("sentinel1.orbit"),
            Some(DownloadProviderKind::Orbit)
        );
        assert_eq!(DownloadProviderKind::from_provider_id("unknown"), None);
    }

    fn historical_work(dataset: &str) -> DownloadWork {
        let item: SnapshotItem = serde_json::from_value(json!({
            "item_id": format!("{dataset}-TEST"),
            "provider_id": "asf.catalog",
            "display_name": format!("{dataset}-TEST.zip"),
            "metadata": {"dataset": dataset},
            "payload": {
                "download_url": "https://datapool.asf.alaska.edu/test/product.zip",
                "file_name": "product.zip"
            }
        }))
        .unwrap();
        let task: FrozenTaskSnapshot = serde_json::from_value(json!({
            "schema_version": 1,
            "task_id": format!("{}-task", dataset.to_ascii_lowercase()),
            "batch_id": format!("{}-batch", dataset.to_ascii_lowercase()),
            "provider_id": "asf.catalog",
            "created_at": "2026-08-24T00:00:00Z",
            "aoi_name": "Global",
            "aoi_geojson": null,
            "query": {"dataset": dataset},
            "candidates": [item.clone()],
            "initial_selected_item_ids": [item.item_id.clone()],
            "output_root": ".",
            "layout": "direct"
        }))
        .unwrap();
        DownloadWork { task, item }
    }

    fn seasat_work(payload: serde_json::Value, assets: serde_json::Value) -> DownloadWork {
        let item: SnapshotItem = serde_json::from_value(json!({
            "item_id": "ASF::SEASAT::SS_01502_STD_F2536",
            "provider_id": "asf.catalog",
            "display_name": "SS_01502_STD_F2536.h5",
            "metadata": {
                "dataset": "SEASAT",
                "checksum": "fd99f89b380163d20c50b4a26fa0db12",
                "assets": assets
            },
            "payload": payload
        }))
        .unwrap();
        let task: FrozenTaskSnapshot = serde_json::from_value(json!({
            "schema_version": 1,
            "task_id": "seasat-task",
            "batch_id": "seasat-batch",
            "provider_id": "asf.catalog",
            "created_at": "2026-08-25T00:00:00Z",
            "aoi_name": "Global",
            "aoi_geojson": null,
            "query": {"dataset": "SEASAT"},
            "candidates": [item.clone()],
            "initial_selected_item_ids": [item.item_id.clone()],
            "output_root": std::env::temp_dir().join("insar-seasat-worker-test"),
            "layout": "direct"
        }))
        .unwrap();
        DownloadWork { task, item }
    }

    fn valid_seasat_assets() -> serde_json::Value {
        json!([
            {
                "role": "primary",
                "url": "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.h5",
                "file_name": "SS_01502_STD_F2536.h5",
                "media_type": "application/x-hdf5",
                "size_bytes": 425004910
            },
            {
                "role": "derived",
                "url": "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.tif",
                "file_name": "SS_01502_STD_F2536.tif",
                "media_type": "image/tiff",
                "size_bytes": 287076789
            },
            {
                "role": "metadata",
                "url": "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.xml",
                "file_name": "SS_01502_STD_F2536.xml"
            },
            {
                "role": "browse",
                "url": "https://cumulus.asf.earthdatacloud.nasa.gov/BROWSE/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.jpg",
                "file_name": "SS_01502_STD_F2536.jpg",
                "media_type": "image/jpeg"
            },
            {
                "role": "auxiliary",
                "url": "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.qc_report",
                "file_name": "SS_01502_STD_F2536.qc_report"
            }
        ])
    }

    #[test]
    fn seasat_worker_dispatches_exactly_the_frozen_hdf5_primary() {
        let work = seasat_work(
            json!({
                "download_url": "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.h5",
                "file_name": "SS_01502_STD_F2536.h5",
                "file_size_remote": 425004910
            }),
            valid_seasat_assets(),
        );
        assert_eq!(
            seasat_resolution_plan(&work).unwrap(),
            SeasatResolutionPlan {
                url: "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.h5".to_string(),
                file_name: "SS_01502_STD_F2536.h5".to_string(),
                expected_size: Some(425004910),
                md5: "fd99f89b380163d20c50b4a26fa0db12".to_string(),
            }
        );

        let backend = Arc::new(SeasatRecordingBackend::default());
        let mut worker = ProductionDownloadWorker::new(TransferOptions::default()).unwrap();
        worker.transfer_backends = crate::TransferBackendSelector::with_preferred_for_test(
            worker.transfer.clone(),
            backend.clone(),
        );
        worker.bypass_earthdata_auth_for_test = true;
        let result = worker.execute(work, WorkerControl::default(), &mut |_| {});

        assert_eq!(result.outcome, DownloadOutcome::Completed);
        let requests = backend.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0]
            .0
            .to_string_lossy()
            .ends_with("SS_01502_STD_F2536.h5"));
        assert_eq!(requests[0].1, Some(425004910));
    }

    #[test]
    fn seasat_missing_hdf5_primary_fails_before_any_transfer() {
        let work = seasat_work(
            json!({
                "download_url": "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.tif",
                "file_name": "SS_01502_STD_F2536.tif",
                "file_size_remote": 287076789
            }),
            json!([{
                "role": "derived",
                "url": "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.tif",
                "file_name": "SS_01502_STD_F2536.tif",
                "media_type": "image/tiff",
                "size_bytes": 287076789
            }]),
        );
        let backend = Arc::new(SeasatRecordingBackend::default());
        let mut worker = ProductionDownloadWorker::new(TransferOptions::default()).unwrap();
        worker.transfer_backends = crate::TransferBackendSelector::with_preferred_for_test(
            worker.transfer.clone(),
            backend.clone(),
        );
        worker.bypass_earthdata_auth_for_test = true;

        let result = worker.execute(work, WorkerControl::default(), &mut |_| {});

        assert_eq!(result.outcome, DownloadOutcome::Failed);
        assert_eq!(result.failure.unwrap().code, "ASF_PRIMARY_ASSET_MISSING");
        assert!(backend.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn seasat_competing_primary_fails_before_any_transfer() {
        let mut assets = valid_seasat_assets();
        assets.as_array_mut().unwrap().push(json!({
            "role": "primary",
            "url": "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536-copy.h5",
            "file_name": "SS_01502_STD_F2536-copy.h5",
            "media_type": "application/x-hdf5",
            "size_bytes": 425004910
        }));
        let work = seasat_work(
            json!({
                "download_url": "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.h5",
                "file_name": "SS_01502_STD_F2536.h5",
                "file_size_remote": 425004910
            }),
            assets,
        );
        let backend = Arc::new(SeasatRecordingBackend::default());
        let mut worker = ProductionDownloadWorker::new(TransferOptions::default()).unwrap();
        worker.transfer_backends = crate::TransferBackendSelector::with_preferred_for_test(
            worker.transfer.clone(),
            backend.clone(),
        );
        worker.bypass_earthdata_auth_for_test = true;

        let result = worker.execute(work, WorkerControl::default(), &mut |_| {});

        assert_eq!(result.outcome, DownloadOutcome::Failed);
        assert_eq!(result.failure.unwrap().code, "ASF_PRIMARY_ASSET_AMBIGUOUS");
        assert!(backend.requests.lock().unwrap().is_empty());
    }

    fn auth_rejection() -> Option<DownloadFailure> {
        Some(DownloadFailure {
            code: "DL004".to_string(),
            reason_zh: "remote rejected".to_string(),
            retryable: false,
        })
    }

    #[test]
    fn jers_remote_auth_rejection_becomes_dataset_access_restricted() {
        let mut failure = auth_rejection();

        classify_asf_dataset_access_restriction(&historical_work("JERS-1"), &mut failure);

        let failure = failure.unwrap();
        assert_eq!(failure.code, "ASF_DATASET_ACCESS_RESTRICTED");
        // ASF removed JERS-1 from its searchable interfaces (Owner evidence
        // 2026-10-31), so the removal reason leads.
        assert_eq!(failure.reason_zh, "ASF 已从检索接口下线该数据集");
        assert!(!failure.retryable);
    }

    #[test]
    fn asf_removed_legacy_auth_failures_use_the_removal_reason() {
        // ERS, JERS-1 and RADARSAT-1 were removed from ASF's searchable
        // interfaces (Owner evidence 2026-10-31): an auth failure on any of
        // them is reclassified with the ASF-removed reason. JERS-1 keeps the
        // research-agreement reason because its restriction predates the
        // removal.
        for dataset in ["ERS", "RADARSAT-1"] {
            let mut failure = auth_rejection();
            classify_asf_dataset_access_restriction(&historical_work(dataset), &mut failure);
            let failure = failure.unwrap();
            assert_eq!(failure.code, "ASF_DATASET_ACCESS_RESTRICTED", "{dataset}");
            assert_eq!(
                failure.reason_zh, "ASF 已从检索接口下线该数据集",
                "{dataset}"
            );
            assert!(!failure.retryable);
        }
    }

    #[test]
    fn non_auth_failures_and_open_missions_are_not_reclassified() {
        let mut jers_network = Some(DownloadFailure {
            code: "DL005".to_string(),
            reason_zh: "network".to_string(),
            retryable: true,
        });
        let mut seasat_rejection = auth_rejection();

        classify_asf_dataset_access_restriction(&historical_work("JERS-1"), &mut jers_network);
        classify_asf_dataset_access_restriction(&historical_work("SEASAT"), &mut seasat_rejection);

        assert_eq!(jers_network.unwrap().code, "DL005");
        assert_eq!(seasat_rejection.unwrap().code, "DL004");
    }

    #[test]
    fn successful_jers_result_remains_successful() {
        let mut failure = None;

        classify_asf_dataset_access_restriction(&historical_work("JERS-1"), &mut failure);

        assert!(failure.is_none());
    }

    #[derive(Default)]
    struct BurstRouteRecordingBackend {
        legacy_calls: AtomicUsize,
        resolved_calls: AtomicUsize,
    }

    impl TransferBackend for BurstRouteRecordingBackend {
        fn kind(&self) -> TransferBackendKind {
            TransferBackendKind::NativeSingleStream
        }

        fn is_available(&self) -> bool {
            true
        }

        fn download_legacy(
            &self,
            _request: &TransferRequest,
            _auth: &TransferAuth,
            _cancelled: &(dyn Fn() -> bool + Sync),
            _paused: &(dyn Fn() -> bool + Sync),
            _progress: &mut dyn FnMut(TransferUpdate),
        ) -> TransferResult {
            self.legacy_calls.fetch_add(1, Ordering::SeqCst);
            TransferResult {
                outcome: DownloadOutcome::Completed,
                bytes_written: 1_024,
                attempt_count: 1,
                failure: None,
            }
        }

        fn resolved_session(&self) -> ResolvedTransferSession {
            ResolvedTransferSession::new(1).unwrap()
        }

        fn download_resolved(
            &self,
            _request: &TransferRequest,
            _asset: &ResolvedAsset,
            _policy: AuthRefreshPolicyId,
            _session: &mut ResolvedTransferSession,
            _cancelled: &(dyn Fn() -> bool + Sync),
            _paused: &(dyn Fn() -> bool + Sync),
            _progress: &mut dyn FnMut(TransferUpdate),
        ) -> ResolvedTransferResult {
            self.resolved_calls.fetch_add(1, Ordering::SeqCst);
            ResolvedTransferResult::Finished(TransferResult {
                outcome: DownloadOutcome::Completed,
                bytes_written: 1_024,
                attempt_count: 1,
                failure: None,
            })
        }
    }

    #[test]
    fn direct_nisar_v2_execute_requires_scheduler_authority_before_any_backend() {
        let mut item = burst_worker_item(AssetRole::Primary);
        item.metadata.insert(
            "delivery_kind".to_owned(),
            serde_json::Value::String(crate::NISAR_SUBSET_V2_DELIVERY_KIND.to_owned()),
        );
        let output_root = std::env::temp_dir().join(format!(
            "insar-direct-v2-authority-required-{}",
            std::process::id()
        ));
        let task = burst_worker_task(std::slice::from_ref(&item), &output_root);
        let backend = Arc::new(BurstRouteRecordingBackend::default());
        let mut worker = ProductionDownloadWorker::new(TransferOptions::default()).unwrap();
        worker.transfer_backends = crate::TransferBackendSelector::with_preferred_for_test(
            worker.transfer.clone(),
            backend.clone(),
        );
        worker.bypass_earthdata_auth_for_test = true;
        let mut progress_count = 0_usize;

        let result = worker.execute(
            DownloadWork { task, item },
            WorkerControl::default(),
            &mut |_| progress_count += 1,
        );

        assert_eq!(result.outcome, DownloadOutcome::Failed);
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.code.as_str()),
            Some("NISAR_SUBSET_V2_AUTHORITY_REQUIRED")
        );
        assert!(!result.failure.unwrap().retryable);
        assert_eq!(progress_count, 0);
        assert_eq!(backend.legacy_calls.load(Ordering::SeqCst), 0);
        assert_eq!(backend.resolved_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn burst_non_resolver_classifications_fail_closed_before_any_backend() {
        let mut observations = Vec::new();
        for case in ["missing-policy", "explicit-legacy", "invalid-policy"] {
            let mut item = burst_worker_item(AssetRole::Primary);
            item.execution_policy = match case {
                "missing-policy" => None,
                "explicit-legacy" => Some(AssetExecutionPolicy {
                    authority: AssetExecutionAuthority::Legacy,
                    policy_id: "legacy.v1".to_owned(),
                    collection_profile_id: Some(ASF_BURST_COLLECTION_PROFILE_ID.to_owned()),
                }),
                "invalid-policy" => Some(AssetExecutionPolicy {
                    authority: AssetExecutionAuthority::ResolverAuthoritative,
                    policy_id: "asf.sentinel-1.burst.unknown.v1".to_owned(),
                    collection_profile_id: Some(ASF_BURST_COLLECTION_PROFILE_ID.to_owned()),
                }),
                _ => unreachable!(),
            };
            let output_root = std::env::temp_dir()
                .join(format!("insar-burst-route-{case}-{}", std::process::id()));
            let task = burst_worker_task(std::slice::from_ref(&item), &output_root);
            let backend = Arc::new(BurstRouteRecordingBackend::default());
            let mut worker = ProductionDownloadWorker::new(TransferOptions::default()).unwrap();
            worker.transfer_backends = crate::TransferBackendSelector::with_preferred_for_test(
                worker.transfer.clone(),
                backend.clone(),
            );
            worker.bypass_earthdata_auth_for_test = true;

            let result = worker.execute(
                DownloadWork { task, item },
                WorkerControl::default(),
                &mut |_| {},
            );
            observations.push((
                case,
                result.outcome,
                result.failure.map(|failure| failure.code),
                backend.legacy_calls.load(Ordering::SeqCst),
                backend.resolved_calls.load(Ordering::SeqCst),
            ));
        }

        assert_eq!(
            observations,
            [
                (
                    "missing-policy",
                    DownloadOutcome::Failed,
                    Some("ASF_BURST_ASSET_INVALID".to_owned()),
                    0,
                    0,
                ),
                (
                    "explicit-legacy",
                    DownloadOutcome::Failed,
                    Some("ASF_BURST_ASSET_INVALID".to_owned()),
                    0,
                    0,
                ),
                (
                    "invalid-policy",
                    DownloadOutcome::Failed,
                    Some("ASF_BURST_ASSET_INVALID".to_owned()),
                    0,
                    0,
                ),
            ]
        );
    }

    fn sentinel_work(
        payload: serde_json::Value,
        metadata: serde_json::Value,
        query: serde_json::Value,
    ) -> DownloadWork {
        let item: SnapshotItem = serde_json::from_value(json!({
            "item_id": "S1A_TEST_SCENE",
            "provider_id": "asf.sentinel1",
            "display_name": "S1A_TEST_SCENE",
            "metadata": metadata,
            "payload": payload
        }))
        .unwrap();
        let task: FrozenTaskSnapshot = serde_json::from_value(json!({
            "schema_version": 1,
            "task_id": "sentinel-task",
            "batch_id": "sentinel-batch",
            "provider_id": "asf.sentinel1",
            "created_at": "2026-07-15T00:00:00Z",
            "aoi_name": "AOI",
            "aoi_geojson": {"type": "Polygon", "coordinates": []},
            "query": query,
            "candidates": [item.clone()],
            "initial_selected_item_ids": [item.item_id.clone()],
            "output_root": ".",
            "layout": "direct"
        }))
        .unwrap();
        DownloadWork { task, item }
    }

    #[test]
    fn sentinel_resolution_prefers_the_frozen_official_asset() {
        let work = sentinel_work(
            json!({
                "download_url": "https://datapool.asf.alaska.edu/SLC/SA/S1A_TEST.zip",
                "file_name": "S1A_TEST.zip",
                "file_size_remote": 1234
            }),
            json!({"processing_level": "RAW"}),
            json!({"processing_levels": ["SLC"]}),
        );

        assert_eq!(
            sentinel_resolution_plan(&work).unwrap(),
            SentinelResolutionPlan::Frozen {
                url: "https://datapool.asf.alaska.edu/SLC/SA/S1A_TEST.zip".to_string(),
                file_name: "S1A_TEST.zip".to_string(),
                expected_size: Some(1234),
            }
        );
    }

    #[test]
    fn legacy_sentinel_resolution_uses_item_processing_level_before_query() {
        let work = sentinel_work(
            json!({"expected_filename": "S1A_TEST.raw"}),
            json!({"processing_level": "RAW"}),
            json!({"processing_levels": ["SLC"]}),
        );

        assert_eq!(
            sentinel_resolution_plan(&work).unwrap(),
            SentinelResolutionPlan::Metadata {
                file_name: "S1A_TEST.raw".to_string(),
                product_type: "RAW".to_string(),
            }
        );
    }

    #[test]
    fn legacy_sentinel_resolution_uses_unambiguous_processing_levels_query() {
        let work = sentinel_work(
            json!({"expected_filename": "S1A_TEST.nc"}),
            json!({}),
            json!({"processing_levels": ["OCN"]}),
        );

        assert_eq!(
            sentinel_resolution_plan(&work).unwrap(),
            SentinelResolutionPlan::Metadata {
                file_name: "S1A_TEST.nc".to_string(),
                product_type: "OCN".to_string(),
            }
        );
    }

    #[test]
    fn legacy_sentinel_resolution_never_silently_defaults_to_slc() {
        let work = sentinel_work(
            json!({"expected_filename": "S1A_TEST.zip"}),
            json!({}),
            json!({"processing_levels": ["SLC", "RAW"]}),
        );

        let error = sentinel_resolution_plan(&work).unwrap_err();

        assert_eq!(error.code, "DL005");
        assert!(error.reason_zh.contains("产品类型"));
        assert!(!error.reason_zh.contains("SLC"));
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn nisar_science_parallel_range_workers_stays_within_bhoonidhi_limit() {
        let bhoonidhi_workers = super::nisar_science_parallel_range_workers(
            insar_provider::catalog::BHOONIDHI_PROVIDER_ID,
        );
        assert!(bhoonidhi_workers >= 1);
        assert!(bhoonidhi_workers <= crate::BHOONIDHI_MAX_CONCURRENT_DOWNLOADS);
        assert_eq!(bhoonidhi_workers, 1);
        assert_eq!(super::nisar_science_parallel_range_workers("asf.nisar"), 4);
    }
}
