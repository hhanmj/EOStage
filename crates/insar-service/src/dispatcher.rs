use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

use insar_core::{
    asf_catalog_capabilities, decide_asf_cache, match_orbits, normalize_asf_features,
    parse_orbit_filenames, AsfCacheDecisionRequest, AsfCachedSearchRequest, AsfNormalizeRequest,
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogCollectionDescriptor,
    CatalogSearchQuery, CatalogSearchResponse, CatalogSearchStats, ContractError,
    OrbitMatchRequest, OrbitParseRequest, ProviderNetworkPolicy, ProviderProduct, ProviderResult,
    ResourceRegistry, SarRemoteSearchRequest, SarRemoteSearchResponse, SarSearchCountRequest,
    SarSearchCountResponse, SarSourceCapability, SarSourceSearchKind, SidecarOperation,
    SidecarRequest, SidecarResponse, SidecarServiceInfo, V3_CONTRACT_VERSION, V3_PROTOCOL_MAJOR,
    V3_PROTOCOL_MINOR, V3_PROTOCOL_MIN_MINOR,
};
use insar_download::{
    resolve_earthdata_search_token, DownloadScheduler, ProductionDownloadWorker, TransferOptions,
};
use insar_provider::asf::catalog_client::{AsfCatalogClient, CatalogSearchProviderProgress};
use insar_provider::asf::client::AsfHttpClient;
use insar_provider::asf::search::{AsfProvider, SarSearchProviderProgress};
use insar_provider::bhoonidhi::{BhoonidhiCatalogClient, BhoonidhiCatalogProgress};
use insar_provider::catalog::StacOfflineFixtureAdmission;
use insar_provider::catalog::{
    CollectionProfile, CollectionProfileRegistry, DatasetRoutePolicyRegistry,
    EarthSearchLandsatCatalogClient, EarthSearchSentinel2CatalogClient, MpcHlsCatalogClient,
    MpcHlsProduct, MpcReviewedCatalogClient, MpcSentinel2CatalogClient, RouteUse,
    ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROVIDER_ID,
    BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROVIDER_ID,
    EARTH_SEARCH_LANDSAT_COLLECTION_ID, EARTH_SEARCH_LANDSAT_PROVIDER_ID,
    EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID, EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
    LANDSAT_C2_L2_LOGICAL_DATASET, LANDSAT_C2_L2_NATIVE_COLLECTION_ID, LANDSAT_C2_L2_PROVIDER_ID,
    MPC_COPDEM_GLO30_COLLECTION_ID, MPC_COPDEM_GLO30_LOGICAL_DATASET, MPC_COPDEM_GLO30_PROFILE_ID,
    MPC_SENTINEL1_GRD_COLLECTION_ID, MPC_SENTINEL1_GRD_LOGICAL_DATASET,
    MPC_SENTINEL1_GRD_PROFILE_ID, MPC_SENTINEL2_L2A_COLLECTION_ID, MPC_SENTINEL2_L2A_PROVIDER_ID,
    NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID, NISAR_PROVIDER_ID,
    PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use insar_provider::cdse::catalog_client::CdseODataCatalogClient;
use insar_provider::cdse::loopback_client::LoopbackODataClient;
use insar_provider::planetary_computer::PlanetaryComputerLandsatClient;
use insar_provider::usgs_m2m::UsgsM2mSceneSearchRouter;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::asf_cached_search::execute_cached_asf_search;
use crate::store_dispatch::{
    default_store_path, dispatch_download_snapshot, dispatch_startup_health, dispatch_store,
    LazyTaskStore,
};
use crate::{CancellationRegistry, CatalogueRequestLease, CatalogueRequestRegistry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchClass {
    Control,
    Work,
}

pub trait RequestDispatcher: Sync {
    fn dispatch(&self, request: SidecarRequest) -> SidecarResponse;
    fn classify(&self, operation: &SidecarOperation) -> DispatchClass;
    fn is_shutting_down(&self) -> bool;
}

pub trait LiveAsfSearch: Send + Sync {
    fn search(
        &self,
        request: &AsfRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse>;

    fn search_sar(
        &self,
        _request: &SarRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        Err(ContractError::new(
            "V3_SIDECAR_CAPABILITY_UNAVAILABLE",
            "褰撳墠 ASF Provider 鏈疄鐜伴€氱敤 SAR 妫€绱?",
            Map::new(),
        )
        .expect("empty generic SAR capability context is safe"))
    }

    fn search_sar_with_progress(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        _progress: &dyn Fn(SarSearchProviderProgress),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.search_sar(request, cancelled)
    }

    fn count_sar(
        &self,
        _request: &SarSearchCountRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarSearchCountResponse> {
        Err(ContractError::new(
            "V3_SIDECAR_CAPABILITY_UNAVAILABLE",
            "当前 ASF Provider 未实现通用 SAR 候选计数",
            Map::new(),
        )
        .expect("empty generic SAR count capability context is safe"))
    }
}

/// #22 B slice 3(b): a streaming catalogue search sink. Products arrive in
/// provider order, in bounded chunks. `attempt` is called before each source
/// attempt with that attempt's exact query, so a stateful sink can reset any
/// partial state from a failed fallback source.
pub trait CatalogSearchStreamSink {
    /// Called once per provider chunk of already-mapped products.
    fn accept(&mut self, products: &[ProviderProduct]) -> ProviderResult<()>;

    /// Called before each source attempt. The default suits stateless sinks.
    fn attempt(&mut self, _query: &CatalogSearchQuery) -> ProviderResult<()> {
        Ok(())
    }
}

/// A sink that ignores all products. Used by callers that only need the
/// final statistics.
pub struct CatalogSearchNoopSink;

impl CatalogSearchStreamSink for CatalogSearchNoopSink {
    fn accept(&mut self, _products: &[ProviderProduct]) -> ProviderResult<()> {
        Ok(())
    }
}

/// #22 B slice 3(b): the streaming counterpart of a whole
/// [`CatalogSearchResponse`]. Either the provider streamed its products to
/// the sink in bounded chunks and only the validated statistics and
/// collection remain, or the provider has not migrated and the whole
/// response is returned (its products were still handed to the sink as one
/// chunk, so the caller's chunk handling runs for every provider).
#[derive(Debug, Clone, PartialEq)]
pub enum CatalogSearchStreamOutcome {
    Streamed {
        collection: CatalogCollectionDescriptor,
        stats: CatalogSearchStats,
    },
    WholeResponse {
        response: CatalogSearchResponse,
    },
}

/// Private application-only catalogue search boundary. It is deliberately not
/// represented by a Sidecar operation or provider capability.
pub trait LiveCatalogSearch: Send + Sync {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse>;

    fn search_catalog_with_progress(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        _progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search_catalog(query, cancelled)
    }

    /// #22 B slice 3(b): stream the search's mapped products to `sink` in
    /// bounded provider-order chunks instead of accumulating the whole
    /// response. `sink.attempt(query)` is called first so a stateful sink can
    /// reset per-source state. Unmigrated providers keep the whole-response
    /// method; the default implementation here still hands its products to
    /// the sink as one chunk, so the caller's chunk handling is exercised for
    /// every provider. A provider that cannot stream its route (for example
    /// a multi-part spatial fallback whose cap is only decidable after all
    /// parts) may return [`CatalogSearchStreamOutcome::WholeResponse`].
    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamOutcome> {
        // #22 B slice 3(b) review fix 4: unmigrated providers keep their
        // whole-response fetch, but their progress overrides (for example
        // ASF and Bhoonidhi search_catalog_with_progress) still run, so the
        // Fetching progress events match the pre-streaming sequence.
        sink.attempt(query)?;
        let response = self.search_catalog_with_progress(query, cancelled, progress)?;
        sink.accept(&response.products)?;
        Ok(CatalogSearchStreamOutcome::WholeResponse { response })
    }
}

/// #22 B slice 3(b): forward closure used to adapt the trait sink to the
/// provider clients' FnMut-based streaming signatures.
fn streaming_sink_forwarder(
    sink: &mut dyn CatalogSearchStreamSink,
) -> impl FnMut(&[ProviderProduct]) -> ProviderResult<()> + '_ {
    move |products: &[ProviderProduct]| sink.accept(products)
}

fn streaming_sink_forwarder_with_progress<'a>(
    sink: &'a mut dyn CatalogSearchStreamSink,
    progress: &'a dyn Fn(CatalogSearchProviderProgress),
    requested_limit: Option<u64>,
) -> impl FnMut(&[ProviderProduct]) -> ProviderResult<()> + 'a {
    let started = std::time::Instant::now();
    let mut requests = 0_u64;
    let mut streamed = 0_u64;
    move |products: &[ProviderProduct]| {
        sink.accept(products)?;
        requests = requests.saturating_add(1);
        streamed = streamed.saturating_add(products.len() as u64);
        let admitted = requested_limit.map_or(streamed, |limit| streamed.min(limit));
        progress(CatalogSearchProviderProgress {
            provider_request_count: requests,
            transport_part_count: 1,
            raw_fetched_count: streamed,
            mapped_count: streamed,
            unique_admitted_count: admitted,
            exact_matched_count: streamed,
            requested_limit,
            remote_total: None,
            elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        });
        Ok(())
    }
}

/// #22 B slice 3(c): whether the production router streams this catalogue
/// route page by page into the sink (Earth Search Landsat, Planetary
/// Computer HLS, and the reviewed Planetary Computer Sentinel-1 GRD and
/// Copernicus DEM routes), instead of fetching the whole response before
/// handing it over. Only such routes may default to the 10,000 ceiling: a
/// whole-response route would hold every product in memory first. Keep in
/// step with the arms of `ProductionCatalogRouter::search_catalog_streaming`.
pub fn catalogue_route_streams(
    logical_dataset: &str,
    provider_id: &str,
    native_collection_id: &str,
) -> bool {
    matches!(
        (logical_dataset, provider_id, native_collection_id),
        (
            MPC_SENTINEL1_GRD_LOGICAL_DATASET,
            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
            MPC_SENTINEL1_GRD_COLLECTION_ID,
        ) | (
            MPC_COPDEM_GLO30_LOGICAL_DATASET,
            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
            MPC_COPDEM_GLO30_COLLECTION_ID,
        ) | (
            LANDSAT_C2_L2_LOGICAL_DATASET,
            EARTH_SEARCH_LANDSAT_PROVIDER_ID,
            EARTH_SEARCH_LANDSAT_COLLECTION_ID,
        ) | (
            "optical.sentinel-2.l2a",
            EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
            EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
        ) | (
            "optical.sentinel-2.l2a",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
        ) | (
            "optical.sentinel-2.l2a",
            MPC_SENTINEL2_L2A_PROVIDER_ID,
            MPC_SENTINEL2_L2A_COLLECTION_ID,
        ) | (
            LANDSAT_C2_L2_LOGICAL_DATASET,
            insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID,
            insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID,
        )
    ) || MpcHlsProduct::for_source(logical_dataset, provider_id, native_collection_id).is_some()
}

/// The typed fail-closed reason a streaming transport reports when the
/// result cap cannot be decided page-by-page (undated query, or a
/// continuation that states a different sort mid-search).
pub fn is_streaming_cap_order(error: &ContractError) -> bool {
    error.code == "STAC_RESPONSE_INVALID"
        && error.debug_context.get("reason").and_then(Value::as_str) == Some("streaming_cap_order")
}

/// Retry the identical query on the whole-response path and hand its
/// products to the sink as one chunk. This is the bounded fallback for the
/// typed `streaming_cap_order` failure, not a second network attempt after
/// an unknown state: the transport failed closed before flushing any page.
/// #22 B slice 3(b) review fix 3: the fail-closed can fire on a LATER page,
/// after earlier pages were already flushed into the sink. The retry must
/// call `sink.attempt(query)` first so a stateful sink drops the staged rows
/// from the failed stream; otherwise re-staging the same items collides
/// (`AlreadyExists` at the Store).
pub fn retry_whole_response_streamed<T: LiveCatalogSearch>(
    provider: &T,
    query: &CatalogSearchQuery,
    cancelled: &(dyn Fn() -> bool + Sync),
    sink: &mut dyn CatalogSearchStreamSink,
    progress: &dyn Fn(CatalogSearchProviderProgress),
) -> ProviderResult<CatalogSearchStreamOutcome> {
    sink.attempt(query)?;
    let response = provider.search_catalog_with_progress(query, cancelled, progress)?;
    sink.accept(&response.products)?;
    Ok(CatalogSearchStreamOutcome::WholeResponse { response })
}

impl LiveCatalogSearch for LoopbackODataClient {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search_all(query, cancelled)
    }
}

impl LiveCatalogSearch for CdseODataCatalogClient {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search_all(query, cancelled)
    }

    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let streamed = {
            let mut forwarder = streaming_sink_forwarder(sink);
            self.search_streaming(query, cancelled, &mut forwarder, &progress)
        };
        match streamed {
            Ok((collection, stats)) => {
                Ok(CatalogSearchStreamOutcome::Streamed { collection, stats })
            }
            Err(error) if is_streaming_cap_order(&error) => {
                retry_whole_response_streamed(self, query, cancelled, sink, progress)
            }
            Err(error) => Err(error),
        }
    }
}

impl LiveCatalogSearch for UsgsM2mSceneSearchRouter {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        UsgsM2mSceneSearchRouter::search_catalog(self, query, cancelled).map_err(|error| {
            let mut context = Map::from_iter([(
                "endpoint_class".to_owned(),
                Value::String(error.endpoint().as_str().to_owned()),
            )]);
            if let Some(detail) = error.detail() {
                context.insert("parser_stage".to_owned(), Value::String(detail.to_owned()));
            }
            if let Some(status) = error.http_status() {
                context.insert("http_status".to_owned(), Value::Number(status.into()));
            }
            if let Some(seconds) = error.retry_after_seconds() {
                context.insert(
                    "retry_after_seconds".to_owned(),
                    Value::Number(seconds.into()),
                );
            }
            ContractError::new(
                error.code_str(),
                "USGS Landsat scene search failed",
                context,
            )
            .expect("fixed USGS scene-search context is safe")
        })
    }
}

impl LiveCatalogSearch for PlanetaryComputerLandsatClient {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search_catalog(query, cancelled).map_err(|error| {
            let mut context = Map::from_iter([(
                "endpoint_class".to_owned(),
                Value::String(error.endpoint().as_str().to_owned()),
            )]);
            if let Some(status) = error.http_status() {
                context.insert("http_status".to_owned(), Value::Number(status.into()));
            }
            ContractError::new(
                error.code_str(),
                "Planetary Computer 公开 Landsat 目录请求失败",
                context,
            )
            .expect("fixed Planetary Computer catalogue context is safe")
        })
    }

    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let streamed = {
            let mut forwarder = streaming_sink_forwarder(sink);
            PlanetaryComputerLandsatClient::search_catalog_streaming(
                self,
                query,
                cancelled,
                &mut forwarder,
                &progress,
            )
        };
        match streamed {
            Ok((collection, stats)) => {
                Ok(CatalogSearchStreamOutcome::Streamed { collection, stats })
            }
            Err(error) if is_streaming_cap_order(&error) => {
                retry_whole_response_streamed(self, query, cancelled, sink, progress)
            }
            Err(error) => Err(error),
        }
    }
}

impl LiveCatalogSearch for EarthSearchLandsatCatalogClient {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        EarthSearchLandsatCatalogClient::search_catalog(self, query, cancelled)
    }

    /// #22 B slice 3(b): true page-by-page streaming. An undated or
    /// mid-search re-sorted query cannot decide its cap while streaming and
    /// fails closed with the typed `streaming_cap_order` reason; the whole
    /// response is then fetched and handed to the sink as one chunk.
    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let streamed = {
            let mut forwarder =
                streaming_sink_forwarder_with_progress(sink, progress, query.result_limit);
            EarthSearchLandsatCatalogClient::search_catalog_streaming(
                self,
                query,
                cancelled,
                &mut forwarder,
            )
        };
        match streamed {
            Ok((collection, stats)) => {
                Ok(CatalogSearchStreamOutcome::Streamed { collection, stats })
            }
            Err(error) if is_streaming_cap_order(&error) => {
                retry_whole_response_streamed(self, query, cancelled, sink, progress)
            }
            Err(error) => Err(error),
        }
    }
}

impl LiveCatalogSearch for EarthSearchSentinel2CatalogClient {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        EarthSearchSentinel2CatalogClient::search_catalog(self, query, cancelled)
    }

    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let streamed = {
            let mut forwarder =
                streaming_sink_forwarder_with_progress(sink, progress, query.result_limit);
            EarthSearchSentinel2CatalogClient::search_catalog_streaming(
                self,
                query,
                cancelled,
                &mut forwarder,
            )
        };
        match streamed {
            Ok((collection, stats)) => {
                Ok(CatalogSearchStreamOutcome::Streamed { collection, stats })
            }
            Err(error) if is_streaming_cap_order(&error) => {
                retry_whole_response_streamed(self, query, cancelled, sink, progress)
            }
            Err(error) => Err(error),
        }
    }
}

impl LiveCatalogSearch for MpcSentinel2CatalogClient {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        MpcSentinel2CatalogClient::search_catalog(self, query, cancelled)
    }

    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let streamed = {
            let mut forwarder =
                streaming_sink_forwarder_with_progress(sink, progress, query.result_limit);
            MpcSentinel2CatalogClient::search_catalog_streaming(
                self,
                query,
                cancelled,
                &mut forwarder,
            )
        };
        match streamed {
            Ok((collection, stats)) => {
                Ok(CatalogSearchStreamOutcome::Streamed { collection, stats })
            }
            Err(error) if is_streaming_cap_order(&error) => {
                retry_whole_response_streamed(self, query, cancelled, sink, progress)
            }
            Err(error) => Err(error),
        }
    }
}

impl LiveCatalogSearch for MpcHlsCatalogClient {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        MpcHlsCatalogClient::search_catalog(self, query, cancelled)
    }

    /// #22 B slice 3(b): true page-by-page streaming with the same
    /// `streaming_cap_order` whole-response retry as Earth Search.
    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let streamed = {
            let mut forwarder =
                streaming_sink_forwarder_with_progress(sink, progress, query.result_limit);
            MpcHlsCatalogClient::search_catalog_streaming(self, query, cancelled, &mut forwarder)
        };
        match streamed {
            Ok((collection, stats)) => {
                Ok(CatalogSearchStreamOutcome::Streamed { collection, stats })
            }
            Err(error) if is_streaming_cap_order(&error) => {
                retry_whole_response_streamed(self, query, cancelled, sink, progress)
            }
            Err(error) => Err(error),
        }
    }
}

impl LiveCatalogSearch for MpcReviewedCatalogClient {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        MpcReviewedCatalogClient::search_catalog(self, query, cancelled)
    }

    /// #22 B slice 3(b): true page-by-page streaming with the same
    /// `streaming_cap_order` whole-response retry as Earth Search.
    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let streamed = {
            let mut forwarder =
                streaming_sink_forwarder_with_progress(sink, progress, query.result_limit);
            MpcReviewedCatalogClient::search_catalog_streaming(
                self,
                query,
                cancelled,
                &mut forwarder,
            )
        };
        match streamed {
            Ok((collection, stats)) => {
                Ok(CatalogSearchStreamOutcome::Streamed { collection, stats })
            }
            Err(error) if is_streaming_cap_order(&error) => {
                retry_whole_response_streamed(self, query, cancelled, sink, progress)
            }
            Err(error) => Err(error),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AsfTypedCatalogueRoute {
    Alos2,
    Nisar,
}

fn resolve_asf_typed_catalogue_profile(
    query: &CatalogSearchQuery,
) -> ProviderResult<(AsfTypedCatalogueRoute, &'static CollectionProfile)> {
    let route = match (
        query.logical_dataset.as_str(),
        query.provider_id.as_str(),
        query.native_collection_id.as_str(),
    ) {
        (ALOS2_LOGICAL_DATASET, ALOS2_PROVIDER_ID, ALOS2_NATIVE_COLLECTION_ID) => {
            AsfTypedCatalogueRoute::Alos2
        }
        (NISAR_LOGICAL_DATASET, NISAR_PROVIDER_ID, NISAR_NATIVE_COLLECTION_ID) => {
            AsfTypedCatalogueRoute::Nisar
        }
        _ => {
            return Err(ContractError::new(
                "CATALOGUE_ROUTE_UNAVAILABLE",
                "ASF typed 目录请求不匹配已启用的数据集路由",
                Map::new(),
            )
            .expect("empty ASF typed route context is safe"))
        }
    };
    let profile = CollectionProfileRegistry
        .for_source(
            &query.logical_dataset,
            &query.provider_id,
            &query.native_collection_id,
        )
        .ok_or_else(|| {
            ContractError::new(
                "CATALOGUE_PROFILE_UNAVAILABLE",
                "ASF typed 目录请求没有匹配的精确来源 collection profile",
                Map::from_iter([
                    (
                        "logical_dataset".to_owned(),
                        Value::String(query.logical_dataset.clone()),
                    ),
                    (
                        "provider_id".to_owned(),
                        Value::String(query.provider_id.clone()),
                    ),
                    (
                        "native_collection_id".to_owned(),
                        Value::String(query.native_collection_id.clone()),
                    ),
                ]),
            )
            .expect("exact ASF typed profile context is safe")
        })?;
    Ok((route, profile))
}

impl LiveCatalogSearch for AsfCatalogClient {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let (route, profile) = resolve_asf_typed_catalogue_profile(query)?;
        match route {
            AsfTypedCatalogueRoute::Alos2 => self.search_alos2(query, profile, cancelled),
            AsfTypedCatalogueRoute::Nisar => self.search_nisar(query, profile, cancelled),
        }
    }

    fn search_catalog_with_progress(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchResponse> {
        let (route, profile) = resolve_asf_typed_catalogue_profile(query)?;
        match route {
            AsfTypedCatalogueRoute::Alos2 => {
                self.search_alos2_with_progress(query, profile, cancelled, progress)
            }
            AsfTypedCatalogueRoute::Nisar => {
                self.search_nisar_with_progress(query, profile, cancelled, progress)
            }
        }
    }
}

#[cfg(test)]
mod typed_asf_catalogue_profile_tests {
    use super::*;

    fn typed_query(
        logical_dataset: &str,
        provider_id: &str,
        native_collection_id: &str,
    ) -> CatalogSearchQuery {
        CatalogSearchQuery {
            logical_dataset: logical_dataset.to_owned(),
            provider_id: provider_id.to_owned(),
            native_collection_id: native_collection_id.to_owned(),
            aoi_geojson: None,
            cursor: None,
            result_limit: Some(1),
            filters: Map::new(),
        }
    }

    #[test]
    fn asf_typed_profile_resolution_is_exact_source_bound() {
        for (expected_route, logical_dataset, provider_id, native_collection_id) in [
            (
                AsfTypedCatalogueRoute::Alos2,
                ALOS2_LOGICAL_DATASET,
                ALOS2_PROVIDER_ID,
                ALOS2_NATIVE_COLLECTION_ID,
            ),
            (
                AsfTypedCatalogueRoute::Nisar,
                NISAR_LOGICAL_DATASET,
                NISAR_PROVIDER_ID,
                NISAR_NATIVE_COLLECTION_ID,
            ),
        ] {
            let query = typed_query(logical_dataset, provider_id, native_collection_id);
            let (route, profile) = resolve_asf_typed_catalogue_profile(&query)
                .expect("registered exact ASF typed source must resolve");
            assert_eq!(route, expected_route);
            assert_eq!(profile.logical_dataset, logical_dataset);
            assert_eq!(profile.provider_id, provider_id);
            assert_eq!(profile.native_collection_id, native_collection_id);
        }

        for forged in [
            typed_query(
                ALOS2_LOGICAL_DATASET,
                "asf.generic",
                ALOS2_NATIVE_COLLECTION_ID,
            ),
            typed_query(
                ALOS2_LOGICAL_DATASET,
                ALOS2_PROVIDER_ID,
                NISAR_NATIVE_COLLECTION_ID,
            ),
            typed_query(
                NISAR_LOGICAL_DATASET,
                NISAR_PROVIDER_ID,
                ALOS2_NATIVE_COLLECTION_ID,
            ),
        ] {
            let error = resolve_asf_typed_catalogue_profile(&forged)
                .expect_err("forged ASF source tuple must fail before profile selection");
            assert_eq!(error.code, "CATALOGUE_ROUTE_UNAVAILABLE");
        }
    }
}

impl LiveCatalogSearch for BhoonidhiCatalogClient {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search(query, cancelled)
    }

    fn search_catalog_with_progress(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search_with_progress(query, cancelled, &|update: BhoonidhiCatalogProgress| {
            progress(CatalogSearchProviderProgress {
                provider_request_count: update.provider_request_count,
                transport_part_count: update.transport_part_count,
                raw_fetched_count: update.raw_fetched_count,
                mapped_count: update.mapped_count,
                unique_admitted_count: update.unique_admitted_count,
                exact_matched_count: update.exact_matched_count,
                requested_limit: update.requested_limit,
                remote_total: update.remote_total,
                elapsed_ms: update.elapsed_ms,
            })
        })
    }
}

/// Closed production catalogue router for the approved optical, ASF, and
/// Bhoonidhi typed datasets. Dataset policy validation still runs in `Dispatcher` before this
/// port, while this second check prevents a direct caller from crossing
/// Provider boundaries.
pub struct ProductionCatalogRouter {
    cdse: Arc<dyn LiveCatalogSearch>,
    mpc_sentinel2: Option<Arc<dyn LiveCatalogSearch>>,
    earth_search_sentinel2: Option<Arc<dyn LiveCatalogSearch>>,
    mpc_reviewed: Option<Arc<dyn LiveCatalogSearch>>,
    mpc_hls: Option<Arc<dyn LiveCatalogSearch>>,
    usgs: Option<Arc<dyn LiveCatalogSearch>>,
    planetary_computer: Option<Arc<dyn LiveCatalogSearch>>,
    earth_search: Option<Arc<dyn LiveCatalogSearch>>,
    asf: Arc<dyn LiveCatalogSearch>,
    bhoonidhi: Option<Arc<dyn LiveCatalogSearch>>,
}

impl ProductionCatalogRouter {
    pub fn new(
        cdse: Arc<dyn LiveCatalogSearch>,
        usgs: Arc<dyn LiveCatalogSearch>,
        asf: Arc<dyn LiveCatalogSearch>,
    ) -> Self {
        Self {
            cdse,
            mpc_sentinel2: None,
            earth_search_sentinel2: None,
            mpc_reviewed: None,
            mpc_hls: None,
            usgs: Some(usgs),
            planetary_computer: None,
            earth_search: None,
            asf,
            bhoonidhi: None,
        }
    }

    pub fn without_landsat(
        cdse: Arc<dyn LiveCatalogSearch>,
        asf: Arc<dyn LiveCatalogSearch>,
    ) -> Self {
        Self {
            cdse,
            mpc_sentinel2: None,
            earth_search_sentinel2: None,
            mpc_reviewed: None,
            mpc_hls: None,
            usgs: None,
            planetary_computer: None,
            earth_search: None,
            asf,
            bhoonidhi: None,
        }
    }

    /// Inject the authenticated Bhoonidhi catalogue boundary for the exact
    /// NISAR S-band SSAR RSLC route. The default constructors intentionally
    /// leave this route unavailable until a runtime client is supplied.
    pub fn with_bhoonidhi(mut self, bhoonidhi: Arc<dyn LiveCatalogSearch>) -> Self {
        self.bhoonidhi = Some(bhoonidhi);
        self
    }

    /// Native Microsoft identity must enter its own port. It is never placed
    /// behind the USGS tuple, even when both channels describe Landsat scenes.
    pub fn with_planetary_computer(mut self, provider: Arc<dyn LiveCatalogSearch>) -> Self {
        self.planetary_computer = Some(provider);
        self
    }

    pub fn with_earth_search(mut self, provider: Arc<dyn LiveCatalogSearch>) -> Self {
        self.earth_search = Some(provider);
        self
    }

    pub fn with_mpc_sentinel2(mut self, provider: Arc<dyn LiveCatalogSearch>) -> Self {
        self.mpc_sentinel2 = Some(provider);
        self
    }

    pub fn with_earth_search_sentinel2(mut self, provider: Arc<dyn LiveCatalogSearch>) -> Self {
        self.earth_search_sentinel2 = Some(provider);
        self
    }

    pub fn with_mpc_reviewed(mut self, provider: Arc<dyn LiveCatalogSearch>) -> Self {
        self.mpc_reviewed = Some(provider);
        self
    }

    /// Issue #3 R2 B5: search-only HLS v2.0 L30/S30 on Microsoft.
    pub fn with_mpc_hls(mut self, provider: Arc<dyn LiveCatalogSearch>) -> Self {
        self.mpc_hls = Some(provider);
        self
    }
}

impl LiveCatalogSearch for ProductionCatalogRouter {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        match (
            query.logical_dataset.as_str(),
            query.provider_id.as_str(),
            query.native_collection_id.as_str(),
        ) {
            ("optical.sentinel-2.l2a", "copernicus.dataspace.odata", "SENTINEL-2") => {
                self.cdse.search_catalog(query, cancelled)
            }
            (
                "optical.sentinel-2.l2a",
                MPC_SENTINEL2_L2A_PROVIDER_ID,
                MPC_SENTINEL2_L2A_COLLECTION_ID,
            ) => self
                .mpc_sentinel2
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "Microsoft Sentinel-2 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("fixed Microsoft Sentinel-2 route error is safe")
                })?
                .search_catalog(query, cancelled),
            (
                "optical.sentinel-2.l2a",
                EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
                EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
            ) => self
                .earth_search_sentinel2
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "Earth Search Sentinel-2 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("fixed Earth Search Sentinel-2 route error is safe")
                })?
                .search_catalog(query, cancelled),
            (
                MPC_SENTINEL1_GRD_LOGICAL_DATASET,
                PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
                MPC_SENTINEL1_GRD_COLLECTION_ID,
            )
            | (
                MPC_COPDEM_GLO30_LOGICAL_DATASET,
                PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
                MPC_COPDEM_GLO30_COLLECTION_ID,
            ) => self
                .mpc_reviewed
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "Planetary Computer 审核 COG 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("fixed reviewed MPC route error is safe")
                })?
                .search_catalog(query, cancelled),
            (logical_dataset, provider_id, native_collection_id)
                if MpcHlsProduct::for_source(
                    logical_dataset,
                    provider_id,
                    native_collection_id,
                )
                .is_some() =>
            {
                self.mpc_hls
                    .as_ref()
                    .ok_or_else(|| {
                        ContractError::new(
                            "CATALOGUE_ROUTE_UNAVAILABLE",
                            "Planetary Computer HLS 目录路由尚未注入",
                            Map::new(),
                        )
                        .expect("fixed HLS route error is safe")
                    })?
                    .search_catalog(query, cancelled)
            }
            (
                LANDSAT_C2_L2_LOGICAL_DATASET,
                insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID,
                insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID,
            ) => self
                .planetary_computer
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "微软 Landsat 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("fixed Microsoft route error is safe")
                })?
                .search_catalog(query, cancelled),
            (
                LANDSAT_C2_L2_LOGICAL_DATASET,
                EARTH_SEARCH_LANDSAT_PROVIDER_ID,
                EARTH_SEARCH_LANDSAT_COLLECTION_ID,
            ) => self
                .earth_search
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "Earth Search Landsat 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("fixed Earth Search route error is safe")
                })?
                .search_catalog(query, cancelled),
            (
                LANDSAT_C2_L2_LOGICAL_DATASET,
                LANDSAT_C2_L2_PROVIDER_ID,
                LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
            ) => self
                .usgs
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "Landsat typed 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("empty Landsat route context is safe")
                })?
                .search_catalog(query, cancelled),
            (ALOS2_LOGICAL_DATASET, ALOS2_PROVIDER_ID, ALOS2_NATIVE_COLLECTION_ID)
            | (NISAR_LOGICAL_DATASET, NISAR_PROVIDER_ID, NISAR_NATIVE_COLLECTION_ID) => {
                self.asf.search_catalog(query, cancelled)
            }
            (BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_PROVIDER_ID, BHOONIDHI_NATIVE_COLLECTION_ID) => {
                self.bhoonidhi
                    .as_ref()
                    .ok_or_else(|| {
                        ContractError::new(
                            "CATALOGUE_ROUTE_UNAVAILABLE",
                            "Bhoonidhi typed 目录路由尚未注入",
                            Map::new(),
                        )
                        .expect("empty Bhoonidhi route context is safe")
                    })?
                    .search_catalog(query, cancelled)
            }
            _ => Err(ContractError::new(
                "CATALOGUE_ROUTE_UNAVAILABLE",
                "目录请求不匹配已启用的数据集路由",
                Map::new(),
            )
            .expect("empty production catalogue route context is safe")),
        }
    }

    /// #22 B slice 3(b) review fix 1: the streaming counterpart of
    /// [`Self::search_catalog`], with identical match arms. Without this
    /// override the trait default would run the whole-response path and the
    /// per-client streaming overrides (Earth Search, MPC HLS, MPC reviewed)
    /// would never execute in production.
    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamOutcome> {
        match (
            query.logical_dataset.as_str(),
            query.provider_id.as_str(),
            query.native_collection_id.as_str(),
        ) {
            ("optical.sentinel-2.l2a", "copernicus.dataspace.odata", "SENTINEL-2") => self
                .cdse
                .search_catalog_streaming(query, cancelled, sink, progress),
            (
                "optical.sentinel-2.l2a",
                MPC_SENTINEL2_L2A_PROVIDER_ID,
                MPC_SENTINEL2_L2A_COLLECTION_ID,
            ) => self
                .mpc_sentinel2
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "Microsoft Sentinel-2 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("fixed Microsoft Sentinel-2 route error is safe")
                })?
                .search_catalog_streaming(query, cancelled, sink, progress),
            (
                "optical.sentinel-2.l2a",
                EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
                EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
            ) => self
                .earth_search_sentinel2
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "Earth Search Sentinel-2 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("fixed Earth Search Sentinel-2 route error is safe")
                })?
                .search_catalog_streaming(query, cancelled, sink, progress),
            (
                MPC_SENTINEL1_GRD_LOGICAL_DATASET,
                PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
                MPC_SENTINEL1_GRD_COLLECTION_ID,
            )
            | (
                MPC_COPDEM_GLO30_LOGICAL_DATASET,
                PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
                MPC_COPDEM_GLO30_COLLECTION_ID,
            ) => self
                .mpc_reviewed
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "Planetary Computer 审核 COG 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("fixed reviewed MPC route error is safe")
                })?
                .search_catalog_streaming(query, cancelled, sink, progress),
            (logical_dataset, provider_id, native_collection_id)
                if MpcHlsProduct::for_source(
                    logical_dataset,
                    provider_id,
                    native_collection_id,
                )
                .is_some() =>
            {
                self.mpc_hls
                    .as_ref()
                    .ok_or_else(|| {
                        ContractError::new(
                            "CATALOGUE_ROUTE_UNAVAILABLE",
                            "Planetary Computer HLS 目录路由尚未注入",
                            Map::new(),
                        )
                        .expect("fixed HLS route error is safe")
                    })?
                    .search_catalog_streaming(query, cancelled, sink, progress)
            }
            (
                LANDSAT_C2_L2_LOGICAL_DATASET,
                insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID,
                insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID,
            ) => self
                .planetary_computer
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "微软 Landsat 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("fixed Microsoft route error is safe")
                })?
                .search_catalog_streaming(query, cancelled, sink, progress),
            (
                LANDSAT_C2_L2_LOGICAL_DATASET,
                EARTH_SEARCH_LANDSAT_PROVIDER_ID,
                EARTH_SEARCH_LANDSAT_COLLECTION_ID,
            ) => self
                .earth_search
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "Earth Search Landsat 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("fixed Earth Search route error is safe")
                })?
                .search_catalog_streaming(query, cancelled, sink, progress),
            (
                LANDSAT_C2_L2_LOGICAL_DATASET,
                LANDSAT_C2_L2_PROVIDER_ID,
                LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
            ) => self
                .usgs
                .as_ref()
                .ok_or_else(|| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "Landsat typed 目录路由尚未注入",
                        Map::new(),
                    )
                    .expect("empty Landsat route context is safe")
                })?
                .search_catalog_streaming(query, cancelled, sink, progress),
            (ALOS2_LOGICAL_DATASET, ALOS2_PROVIDER_ID, ALOS2_NATIVE_COLLECTION_ID)
            | (NISAR_LOGICAL_DATASET, NISAR_PROVIDER_ID, NISAR_NATIVE_COLLECTION_ID) => self
                .asf
                .search_catalog_streaming(query, cancelled, sink, progress),
            (BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_PROVIDER_ID, BHOONIDHI_NATIVE_COLLECTION_ID) => {
                self.bhoonidhi
                    .as_ref()
                    .ok_or_else(|| {
                        ContractError::new(
                            "CATALOGUE_ROUTE_UNAVAILABLE",
                            "Bhoonidhi typed 目录路由尚未注入",
                            Map::new(),
                        )
                        .expect("empty Bhoonidhi route context is safe")
                    })?
                    .search_catalog_streaming(query, cancelled, sink, progress)
            }
            _ => Err(ContractError::new(
                "CATALOGUE_ROUTE_UNAVAILABLE",
                "目录请求不匹配已启用的数据集路由",
                Map::new(),
            )
            .expect("empty production catalogue route context is safe")),
        }
    }

    fn search_catalog_with_progress(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchResponse> {
        match (
            query.logical_dataset.as_str(),
            query.provider_id.as_str(),
            query.native_collection_id.as_str(),
        ) {
            (ALOS2_LOGICAL_DATASET, ALOS2_PROVIDER_ID, ALOS2_NATIVE_COLLECTION_ID)
            | (NISAR_LOGICAL_DATASET, NISAR_PROVIDER_ID, NISAR_NATIVE_COLLECTION_ID) => self
                .asf
                .search_catalog_with_progress(query, cancelled, progress),
            (BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_PROVIDER_ID, BHOONIDHI_NATIVE_COLLECTION_ID) => {
                self.bhoonidhi
                    .as_ref()
                    .ok_or_else(|| {
                        ContractError::new(
                            "CATALOGUE_ROUTE_UNAVAILABLE",
                            "Bhoonidhi typed 目录路由尚未注入",
                            Map::new(),
                        )
                        .expect("empty Bhoonidhi route context is safe")
                    })?
                    .search_catalog_with_progress(query, cancelled, progress)
            }
            _ => self.search_catalog(query, cancelled),
        }
    }
}

const MAX_CATALOGUE_SOURCE_ATTEMPTS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogSourceFallbackReason {
    TransientNetwork,
    RateLimited,
    ServerError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogSourceAttemptOutcome {
    Fallback(CatalogSourceFallbackReason),
    Succeeded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogSourceAttempt {
    pub provider_id: String,
    pub native_collection_id: String,
    pub outcome: CatalogSourceAttemptOutcome,
}

pub struct CatalogSearchForApplication {
    pub response: CatalogSearchResponse,
    /// Exact reviewed source query that produced `response`. Application/Store
    /// freeze this identity; a fallback never rewrites it to the preferred route.
    pub query: CatalogSearchQuery,
    /// Bounded, secret-free execution provenance for this live provider request.
    /// It is intentionally not a raw error log and is never synthesized from cache reuse.
    pub source_attempts: Vec<CatalogSourceAttempt>,
    pub lease: CatalogueRequestLease,
}

/// Owned registration produced while the ApplicationService policy gate is
/// held. Executing it performs provider/network work after that gate is gone.
pub struct RegisteredCatalogSearchForApplication {
    catalog_search: Arc<dyn LiveCatalogSearch>,
    queries: Vec<CatalogSearchQuery>,
    request_id: String,
    lease: CatalogueRequestLease,
}

/// #22 B slice 3(b): the streaming counterpart of
/// [`CatalogSearchForApplication`], returned by
/// [`RegisteredCatalogSearchForApplication::execute_candidates_streaming`].
pub struct CatalogSearchStreamedOutcome {
    pub outcome: CatalogSearchStreamOutcome,
    /// The exact reviewed source query that produced the outcome.
    pub query: CatalogSearchQuery,
    /// Bounded, secret-free execution provenance for this live provider request.
    pub source_attempts: Vec<CatalogSourceAttempt>,
    pub lease: CatalogueRequestLease,
}

impl RegisteredCatalogSearchForApplication {
    pub fn execute(self) -> ProviderResult<CatalogSearchForApplication> {
        self.execute_with_progress(&|_| {})
    }

    /// A nested lookup keeps its own publication lease but must also stop when
    /// the creating operation is cancelled. Share the existing parent signal;
    /// do not register an alias that could cancel an unrelated request.
    pub fn execute_with_parent_cancellation(
        self,
        parent_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchForApplication> {
        self.execute_candidates_with_cancellation(&|_| None, &|_| {}, parent_cancelled)
    }

    pub fn execute_with_progress(
        self,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchForApplication> {
        self.execute_candidates_with_progress(&|_| None, progress)
    }

    /// Execute reviewed source candidates under one cancellation lease. Only
    /// Provider/network errors accepted by `allow_fallback` can advance to the
    /// next source; validation/mapping/Store failures happen outside this loop.
    pub fn execute_candidates_with_progress(
        self,
        fallback_reason: &dyn Fn(&ContractError) -> Option<CatalogSourceFallbackReason>,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchForApplication> {
        self.execute_candidates_with_cancellation(fallback_reason, progress, &|| false)
    }

    /// #22 B slice 3(b): the same reviewed fallback/cancellation loop as
    /// [`Self::execute_candidates_with_progress`], but the winning source
    /// streams its products to `sink` in bounded chunks instead of
    /// accumulating the whole response. The sink's `attempt` hook runs before
    /// each source, letting it drop partial state a failed fallback source
    /// staged. Unmigrated providers stream their whole response as one chunk
    /// through the trait's default method.
    pub fn execute_candidates_streaming(
        self,
        fallback_reason: &dyn Fn(&ContractError) -> Option<CatalogSourceFallbackReason>,
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamedOutcome> {
        self.execute_candidates_streaming_with_cancellation(
            fallback_reason,
            sink,
            progress,
            &|| false,
        )
    }

    /// #22 B slice 3(b) review fix 8: the streaming counterpart of
    /// [`Self::execute_candidates_with_cancellation`]. A nested lookup keeps
    /// its own publication lease but must also stop when the creating
    /// operation is cancelled; share the parent signal like the classic path.
    pub fn execute_candidates_streaming_with_cancellation(
        self,
        fallback_reason: &dyn Fn(&ContractError) -> Option<CatalogSourceFallbackReason>,
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
        parent_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchStreamedOutcome> {
        let Self {
            catalog_search,
            queries,
            request_id,
            lease,
        } = self;
        let cancelled = || lease.is_cancelled() || parent_cancelled();
        let mut source_attempts = Vec::with_capacity(queries.len());
        for (index, query) in queries.iter().enumerate() {
            if cancelled() {
                return Err(catalogue_cancelled("before_provider"));
            }
            catalogue_diagnostic(&request_id, "provider_execute_streaming");
            match catalog_search.search_catalog_streaming(query, &cancelled, sink, progress) {
                Ok(outcome) => {
                    catalogue_diagnostic(&request_id, "provider_returned_streaming");
                    if cancelled() || parent_cancelled() {
                        return Err(catalogue_cancelled("after_provider"));
                    }
                    source_attempts.push(CatalogSourceAttempt {
                        provider_id: query.provider_id.clone(),
                        native_collection_id: query.native_collection_id.clone(),
                        outcome: CatalogSourceAttemptOutcome::Succeeded,
                    });
                    return Ok(CatalogSearchStreamedOutcome {
                        outcome,
                        query: query.clone(),
                        source_attempts,
                        lease,
                    });
                }
                Err(error) => {
                    catalogue_diagnostic(&request_id, "provider_failed");
                    if cancelled()
                        || parent_cancelled()
                        || error.code == "CATALOGUE_REQUEST_CANCELLED"
                    {
                        return Err(catalogue_cancelled("provider_cancelled"));
                    }
                    if index + 1 < queries.len() {
                        if let Some(reason) = fallback_reason(&error) {
                            source_attempts.push(CatalogSourceAttempt {
                                provider_id: query.provider_id.clone(),
                                native_collection_id: query.native_collection_id.clone(),
                                outcome: CatalogSourceAttemptOutcome::Fallback(reason),
                            });
                            catalogue_diagnostic(&request_id, "provider_fallback");
                            continue;
                        }
                    }
                    return Err(error);
                }
            }
        }
        Err(ContractError::new(
            "CATALOGUE_ROUTE_UNAVAILABLE",
            "没有可执行的数据来源",
            Map::new(),
        )
        .expect("empty provider fallback context is safe"))
    }

    fn execute_candidates_with_cancellation(
        self,
        fallback_reason: &dyn Fn(&ContractError) -> Option<CatalogSourceFallbackReason>,
        progress: &dyn Fn(CatalogSearchProviderProgress),
        parent_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchForApplication> {
        let Self {
            catalog_search,
            queries,
            request_id,
            lease,
        } = self;
        let mut source_attempts = Vec::with_capacity(queries.len());
        for (index, query) in queries.iter().enumerate() {
            if lease.is_cancelled() || parent_cancelled() {
                return Err(catalogue_cancelled("before_provider"));
            }
            catalogue_diagnostic(&request_id, "provider_execute");
            match catalog_search.search_catalog_with_progress(
                query,
                &|| lease.is_cancelled() || parent_cancelled(),
                progress,
            ) {
                Ok(response) => {
                    catalogue_diagnostic(&request_id, "provider_returned");
                    if lease.is_cancelled() || parent_cancelled() {
                        return Err(catalogue_cancelled("after_provider"));
                    }
                    source_attempts.push(CatalogSourceAttempt {
                        provider_id: query.provider_id.clone(),
                        native_collection_id: query.native_collection_id.clone(),
                        outcome: CatalogSourceAttemptOutcome::Succeeded,
                    });
                    return Ok(CatalogSearchForApplication {
                        response,
                        query: query.clone(),
                        source_attempts,
                        lease,
                    });
                }
                Err(error) => {
                    catalogue_diagnostic(&request_id, "provider_failed");
                    if lease.is_cancelled()
                        || parent_cancelled()
                        || error.code == "CATALOGUE_REQUEST_CANCELLED"
                    {
                        return Err(catalogue_cancelled("provider_cancelled"));
                    }
                    if index + 1 < queries.len() {
                        if let Some(reason) = fallback_reason(&error) {
                            source_attempts.push(CatalogSourceAttempt {
                                provider_id: query.provider_id.clone(),
                                native_collection_id: query.native_collection_id.clone(),
                                outcome: CatalogSourceAttemptOutcome::Fallback(reason),
                            });
                            catalogue_diagnostic(&request_id, "provider_fallback");
                            continue;
                        }
                    }
                    return Err(error);
                }
            }
        }
        Err(ContractError::new(
            "CATALOGUE_ROUTE_UNAVAILABLE",
            "没有可执行的数据来源",
            Map::new(),
        )
        .expect("empty provider fallback context is safe"))
    }
}

fn catalogue_cancelled(reason: &str) -> ContractError {
    ContractError::new(
        "CATALOGUE_REQUEST_CANCELLED",
        "目录请求已取消",
        Map::from_iter([("reason".to_string(), Value::String(reason.to_string()))]),
    )
    .expect("bounded catalogue cancellation context is safe")
}

#[derive(Clone, Copy, Debug)]
struct ServiceRuntimeCapabilities {
    downloads_enabled: bool,
    execution_policy_supported: bool,
    resolver_runtime_ready: bool,
}

impl LiveAsfSearch for AsfProvider<AsfHttpClient> {
    fn search(
        &self,
        request: &AsfRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        AsfProvider::search(self, request, cancelled)
    }

    fn search_sar(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        AsfProvider::search_sar(self, request, cancelled)
    }

    fn search_sar_with_progress(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(SarSearchProviderProgress),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        AsfProvider::search_sar_with_progress(self, request, cancelled, progress)
    }

    fn count_sar(
        &self,
        request: &SarSearchCountRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarSearchCountResponse> {
        AsfProvider::count_sar(self, request, cancelled)
    }
}

pub struct Dispatcher {
    cancellations: CancellationRegistry,
    catalogue_requests: Arc<CatalogueRequestRegistry>,
    shutting_down: AtomicBool,
    asf_search: Arc<dyn LiveAsfSearch>,
    catalog_search: Option<Arc<dyn LiveCatalogSearch>>,
    catalogue_route_use: Option<RouteUse>,
    offline_stac_fixture: Option<Arc<StacOfflineFixtureAdmission>>,
    bhoonidhi_catalogue_enabled: bool,
    earth_search_catalogue_enabled: bool,
    mpc_sentinel2_catalogue_enabled: bool,
    earth_search_sentinel2_catalogue_enabled: bool,
    mpc_reviewed_catalogue_enabled: bool,
    mpc_hls_catalogue_enabled: bool,
    catalogue_live_acceptance_candidate: bool,
    store: Option<Mutex<LazyTaskStore>>,
    downloads: Option<DownloadScheduler>,
    runtime_capabilities: ServiceRuntimeCapabilities,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatcher {
    pub fn new() -> Self {
        Self::try_new().expect("production ASF Provider initialization is static and valid")
    }

    /// Production ASF provider with the Earthdata bearer-token seam for the
    /// restricted legacy SAR missions (ERS-1/2, RADARSAT-1, JERS-1). Token
    /// resolution is attempted per legacy search; failures surface as the
    /// provider's typed login-required error.
    fn production_asf_provider(
        network_policy: insar_core::ProviderNetworkPolicy,
    ) -> ProviderResult<AsfProvider<AsfHttpClient>> {
        let token_provider = Arc::new(|| resolve_earthdata_search_token().ok());
        AsfProvider::production_with_legacy_cmr_token(network_policy, token_provider)
    }

    pub fn try_new() -> ProviderResult<Self> {
        Self::try_new_with_store_path(default_store_path())
    }

    pub fn try_new_with_store_path(path: std::path::PathBuf) -> ProviderResult<Self> {
        let mut dispatcher = Self::with_asf_search_and_store_path(
            Arc::new(Self::production_asf_provider(
                insar_core::ProviderNetworkPolicy::system(),
            )?),
            path,
        );
        dispatcher.enable_production_catalogue()?;
        Ok(dispatcher)
    }

    pub fn try_new_with_downloads(path: std::path::PathBuf) -> ProviderResult<Self> {
        let worker = ProductionDownloadWorker::new(TransferOptions::default()).map_err(|_| {
            ContractError::new(
                "V3_DOWNLOAD_INITIALIZATION_FAILED",
                "无法初始化 Rust 下载引擎",
                Map::new(),
            )
            .expect("empty download initialization context is safe")
        })?;
        let resolver_runtime_ready = worker.resolver_runtime_ready();
        let downloads = DownloadScheduler::start(
            &path,
            Arc::new(worker),
            std::time::Duration::from_millis(100),
        )
        .map_err(|_| {
            ContractError::new(
                "V3_DOWNLOAD_INITIALIZATION_FAILED",
                "无法打开 Rust 下载任务存储",
                Map::new(),
            )
            .expect("empty download initialization context is safe")
        })?;
        let mut dispatcher = Self::with_asf_search_and_store_path(
            Arc::new(Self::production_asf_provider(
                insar_core::ProviderNetworkPolicy::system(),
            )?),
            path,
        );
        dispatcher.enable_production_catalogue()?;
        dispatcher.downloads = Some(downloads);
        dispatcher.runtime_capabilities.downloads_enabled = true;
        dispatcher.runtime_capabilities.resolver_runtime_ready = resolver_runtime_ready;
        Ok(dispatcher)
    }

    fn enable_production_catalogue(&mut self) -> ProviderResult<()> {
        let cdse = Arc::new(CdseODataCatalogClient::with_network_policy(
            std::time::Duration::from_secs(45),
            ProviderNetworkPolicy::system(),
        )?);
        let asf = Arc::new(AsfCatalogClient::production()?);
        let mpc_reviewed: Arc<dyn LiveCatalogSearch> = Arc::new(MpcReviewedCatalogClient::new(
            ProviderNetworkPolicy::system(),
        )?);
        let mpc_hls: Arc<dyn LiveCatalogSearch> =
            Arc::new(MpcHlsCatalogClient::new(ProviderNetworkPolicy::system())?);
        self.catalog_search = Some(Arc::new(
            ProductionCatalogRouter::without_landsat(cdse, asf)
                .with_mpc_reviewed(mpc_reviewed)
                .with_mpc_hls(mpc_hls),
        ));
        self.catalogue_route_use = Some(RouteUse::Production);
        self.mpc_reviewed_catalogue_enabled = true;
        self.mpc_hls_catalogue_enabled = true;
        self.offline_stac_fixture = None;
        Ok(())
    }

    #[doc(hidden)]
    pub fn with_asf_search(asf_search: Arc<dyn LiveAsfSearch>) -> Self {
        Self::with_asf_search_and_store_path(asf_search, default_store_path())
    }

    #[doc(hidden)]
    pub fn with_asf_search_and_store_path(
        asf_search: Arc<dyn LiveAsfSearch>,
        path: std::path::PathBuf,
    ) -> Self {
        Self {
            cancellations: CancellationRegistry::default(),
            catalogue_requests: Arc::new(CatalogueRequestRegistry::default()),
            shutting_down: AtomicBool::new(false),
            asf_search,
            catalog_search: None,
            catalogue_route_use: None,
            offline_stac_fixture: None,
            bhoonidhi_catalogue_enabled: false,
            earth_search_catalogue_enabled: false,
            mpc_sentinel2_catalogue_enabled: false,
            earth_search_sentinel2_catalogue_enabled: false,
            mpc_reviewed_catalogue_enabled: false,
            mpc_hls_catalogue_enabled: false,
            catalogue_live_acceptance_candidate: false,
            store: Some(Mutex::new(LazyTaskStore::new(path))),
            downloads: None,
            runtime_capabilities: ServiceRuntimeCapabilities {
                downloads_enabled: false,
                execution_policy_supported: true,
                resolver_runtime_ready: false,
            },
        }
    }

    /// Construct only the Provider/cancellation part of the Dispatcher for an
    /// owning ApplicationService. The service supplies the sole Store lease and
    /// scheduler, so this dispatcher deliberately has no Store path to open.
    #[doc(hidden)]
    pub fn with_asf_search_for_application(asf_search: Arc<dyn LiveAsfSearch>) -> Self {
        Self {
            cancellations: CancellationRegistry::default(),
            catalogue_requests: Arc::new(CatalogueRequestRegistry::default()),
            shutting_down: AtomicBool::new(false),
            asf_search,
            catalog_search: None,
            catalogue_route_use: None,
            offline_stac_fixture: None,
            bhoonidhi_catalogue_enabled: false,
            earth_search_catalogue_enabled: false,
            mpc_sentinel2_catalogue_enabled: false,
            earth_search_sentinel2_catalogue_enabled: false,
            mpc_reviewed_catalogue_enabled: false,
            mpc_hls_catalogue_enabled: false,
            catalogue_live_acceptance_candidate: false,
            store: None,
            downloads: None,
            runtime_capabilities: ServiceRuntimeCapabilities {
                downloads_enabled: false,
                execution_policy_supported: true,
                resolver_runtime_ready: false,
            },
        }
    }

    /// Build the production Provider/cancellation runtime without creating a
    /// Store or download scheduler. The ApplicationService owns those resources.
    pub fn try_new_for_application() -> ProviderResult<Self> {
        Self::try_new_for_application_with_network_policy(ProviderNetworkPolicy::system())
    }

    pub fn try_new_for_application_with_network_policy(
        network_policy: ProviderNetworkPolicy,
    ) -> ProviderResult<Self> {
        let cdse = Arc::new(CdseODataCatalogClient::with_network_policy(
            std::time::Duration::from_secs(45),
            network_policy,
        )?);
        let asf = Arc::new(AsfCatalogClient::production()?);
        let mpc_reviewed: Arc<dyn LiveCatalogSearch> =
            Arc::new(MpcReviewedCatalogClient::new(network_policy)?);
        let mpc_hls: Arc<dyn LiveCatalogSearch> =
            Arc::new(MpcHlsCatalogClient::new(network_policy)?);
        let catalogue = Arc::new(
            ProductionCatalogRouter::without_landsat(cdse, asf)
                .with_mpc_reviewed(mpc_reviewed)
                .with_mpc_hls(mpc_hls),
        );
        let mut dispatcher = Self::with_production_catalogue_for_application(
            Arc::new(Self::production_asf_provider(
                insar_core::ProviderNetworkPolicy::system(),
            )?),
            catalogue,
        );
        dispatcher.mpc_reviewed_catalogue_enabled = true;
        dispatcher.mpc_hls_catalogue_enabled = true;
        Ok(dispatcher)
    }

    pub fn try_new_for_application_with_landsat_catalogue(
        network_policy: ProviderNetworkPolicy,
        landsat_catalogue: Arc<dyn LiveCatalogSearch>,
    ) -> ProviderResult<Self> {
        let cdse = Arc::new(CdseODataCatalogClient::with_network_policy(
            std::time::Duration::from_secs(45),
            network_policy,
        )?);
        let asf = Arc::new(AsfCatalogClient::production()?);
        let mpc_reviewed: Arc<dyn LiveCatalogSearch> =
            Arc::new(MpcReviewedCatalogClient::new(network_policy)?);
        let mpc_hls: Arc<dyn LiveCatalogSearch> =
            Arc::new(MpcHlsCatalogClient::new(network_policy)?);
        let catalogue = Arc::new(
            ProductionCatalogRouter::new(cdse, landsat_catalogue, asf)
                .with_mpc_reviewed(mpc_reviewed)
                .with_mpc_hls(mpc_hls),
        );
        let mut dispatcher = Self::with_production_catalogue_for_application(
            Arc::new(Self::production_asf_provider(
                insar_core::ProviderNetworkPolicy::system(),
            )?),
            catalogue,
        );
        dispatcher.mpc_reviewed_catalogue_enabled = true;
        dispatcher.mpc_hls_catalogue_enabled = true;
        Ok(dispatcher)
    }

    /// Construct the production application router with the authenticated
    /// Bhoonidhi S-RSLC route injected alongside the existing optical, Landsat,
    /// and ASF routes.
    pub fn try_new_for_application_with_landsat_and_bhoonidhi_catalogue(
        network_policy: ProviderNetworkPolicy,
        landsat_catalogue: Arc<dyn LiveCatalogSearch>,
        bhoonidhi_catalogue: Arc<dyn LiveCatalogSearch>,
    ) -> ProviderResult<Self> {
        let cdse = Arc::new(CdseODataCatalogClient::with_network_policy(
            std::time::Duration::from_secs(45),
            network_policy,
        )?);
        let asf = Arc::new(AsfCatalogClient::production()?);
        let mpc_reviewed: Arc<dyn LiveCatalogSearch> =
            Arc::new(MpcReviewedCatalogClient::new(network_policy)?);
        let mpc_hls: Arc<dyn LiveCatalogSearch> =
            Arc::new(MpcHlsCatalogClient::new(network_policy)?);
        let catalogue = Arc::new(
            ProductionCatalogRouter::new(cdse, landsat_catalogue, asf)
                .with_mpc_reviewed(mpc_reviewed)
                .with_mpc_hls(mpc_hls)
                .with_bhoonidhi(bhoonidhi_catalogue),
        );
        let mut dispatcher = Self::with_production_catalogue_for_application(
            Arc::new(Self::production_asf_provider(
                insar_core::ProviderNetworkPolicy::system(),
            )?),
            catalogue,
        );
        dispatcher.bhoonidhi_catalogue_enabled = true;
        dispatcher.mpc_reviewed_catalogue_enabled = true;
        dispatcher.mpc_hls_catalogue_enabled = true;
        Ok(dispatcher)
    }

    /// Construct the ordinary public Landsat runtime without enabling a USGS
    /// network fallback. Existing USGS injection constructors remain separate.
    pub fn try_new_for_application_with_planetary_and_bhoonidhi_catalogue(
        network_policy: ProviderNetworkPolicy,
        planetary_computer: Arc<dyn LiveCatalogSearch>,
        bhoonidhi_catalogue: Arc<dyn LiveCatalogSearch>,
    ) -> ProviderResult<Self> {
        let cdse = Arc::new(CdseODataCatalogClient::with_network_policy(
            std::time::Duration::from_secs(45),
            network_policy,
        )?);
        let asf = Arc::new(AsfCatalogClient::production_with_network_policy(
            network_policy,
        )?);
        let earth_search: Arc<dyn LiveCatalogSearch> =
            Arc::new(EarthSearchLandsatCatalogClient::new(network_policy)?);
        let mpc_sentinel2: Arc<dyn LiveCatalogSearch> =
            Arc::new(MpcSentinel2CatalogClient::new(network_policy)?);
        let earth_search_sentinel2: Arc<dyn LiveCatalogSearch> =
            Arc::new(EarthSearchSentinel2CatalogClient::new(network_policy)?);
        let mpc_reviewed: Arc<dyn LiveCatalogSearch> =
            Arc::new(MpcReviewedCatalogClient::new(network_policy)?);
        let mpc_hls: Arc<dyn LiveCatalogSearch> =
            Arc::new(MpcHlsCatalogClient::new(network_policy)?);
        let catalogue = Arc::new(
            ProductionCatalogRouter::without_landsat(cdse, asf)
                .with_planetary_computer(planetary_computer)
                .with_earth_search(earth_search)
                .with_mpc_sentinel2(mpc_sentinel2)
                .with_earth_search_sentinel2(earth_search_sentinel2)
                .with_mpc_reviewed(mpc_reviewed)
                .with_mpc_hls(mpc_hls)
                .with_bhoonidhi(bhoonidhi_catalogue),
        );
        let mut dispatcher = Self::with_production_catalogue_for_application(
            Arc::new(Self::production_asf_provider(network_policy)?),
            catalogue,
        );
        dispatcher.bhoonidhi_catalogue_enabled = true;
        dispatcher.earth_search_catalogue_enabled = true;
        dispatcher.mpc_sentinel2_catalogue_enabled = true;
        dispatcher.earth_search_sentinel2_catalogue_enabled = true;
        dispatcher.mpc_reviewed_catalogue_enabled = true;
        dispatcher.mpc_hls_catalogue_enabled = true;
        Ok(dispatcher)
    }

    /// Build the production application provider boundary. Catalogue dispatch is
    /// deliberately closed to the one reviewed route; no generic provider or
    /// Legacy fallback is available from this port.
    #[doc(hidden)]
    pub fn with_production_catalogue_for_application(
        asf_search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
    ) -> Self {
        let mut dispatcher = Self::with_asf_search_for_application(asf_search);
        dispatcher.catalog_search = Some(catalog_search);
        dispatcher.catalogue_route_use = Some(RouteUse::Production);
        dispatcher
    }

    #[cfg(feature = "usgs-m2m-live-acceptance")]
    #[doc(hidden)]
    pub fn with_live_acceptance_catalogue_for_application(
        asf_search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
    ) -> Self {
        let mut dispatcher =
            Self::with_production_catalogue_for_application(asf_search, catalog_search);
        dispatcher.catalogue_live_acceptance_candidate = true;
        dispatcher
    }

    /// Fixture-only application construction that retains the normal ASF port
    /// while injecting the otherwise disabled catalogue port.
    #[doc(hidden)]
    pub fn with_search_ports_for_application(
        asf_search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
    ) -> Self {
        let mut dispatcher = Self::with_asf_search_for_application(asf_search);
        dispatcher.catalog_search = Some(catalog_search);
        dispatcher.catalogue_route_use = Some(RouteUse::OfflineFixture);
        dispatcher
    }

    /// Process-local admission is accepted only by this offline constructor.
    /// Production construction and protocol requests cannot install it.
    #[doc(hidden)]
    pub fn with_offline_stac_fixture_for_application(
        asf_search: Arc<dyn LiveAsfSearch>,
        catalog_search: Arc<dyn LiveCatalogSearch>,
        admission: Arc<StacOfflineFixtureAdmission>,
    ) -> Self {
        let mut dispatcher = Self::with_search_ports_for_application(asf_search, catalog_search);
        dispatcher.offline_stac_fixture = Some(admission);
        dispatcher
    }

    pub fn catalogue_route_use(&self) -> ProviderResult<RouteUse> {
        self.catalogue_route_use.ok_or_else(|| {
            ContractError::new(
                "CATALOGUE_CAPABILITY_UNAVAILABLE",
                "当前运行时未注入目录检索能力",
                Map::new(),
            )
            .expect("empty catalogue capability context is safe")
        })
    }

    pub fn provider_capabilities(&self) -> insar_core::ProviderCapabilitiesResponse {
        let mut capabilities = asf_catalog_capabilities();
        let mut sources = std::mem::take(&mut capabilities.sources);
        // ASF owns the compatibility projection, while provider-specific typed
        // routes must be projected by their owning application boundary.
        sources.extend(
            ResourceRegistry::embedded_ref()
                .expect("embedded Resource Registry must be valid")
                .sar_source_capabilities_for_providers(&[BHOONIDHI_PROVIDER_ID]),
        );
        capabilities.sources = sar_source_capabilities(
            sources,
            capabilities.available,
            capabilities.reason.as_deref(),
            self.catalogue_route_use,
            self.bhoonidhi_catalogue_enabled,
        );
        capabilities
    }

    /// Publish the ASF/CDSE catalogue bindings that this concrete Dispatcher
    /// actually owns. This is deliberately keyed by Registry binding IDs; it
    /// does not turn the Dispatcher into a product-policy or admission owner.
    pub fn capability_runtime_observations(
        &self,
    ) -> Vec<(String, insar_core::RuntimeReadinessState)> {
        let ready = insar_core::RuntimeReadinessState::Ready;
        let mut observations = [
            "asf.alos-2.palsar-2.scansar.l1.1.v1",
            "asf.alos-palsar.v2",
            "asf.ers.v1",
            "asf.jers-1.v1",
            "asf.nisar.l.rslc.provisional.v1",
            "asf.radarsat-1.v1",
            "asf.seasat.v1",
            "asf.sentinel-1.burst.v1",
            "asf.sentinel-1.standard.v1",
        ]
        .into_iter()
        .map(|binding| (binding.to_owned(), ready))
        .collect::<Vec<_>>();
        if self.catalogue_route_use == Some(RouteUse::Production) {
            observations.extend(
                [
                    "cdse.sentinel-2.l2a.archive-resolver-policy.v1",
                    "cdse.sentinel-2.l2a.collection.v1",
                    "planetary-computer.landsat-c2-l2.v1",
                ]
                .into_iter()
                .map(|binding| (binding.to_owned(), ready)),
            );
        }
        if self.bhoonidhi_catalogue_enabled {
            observations.push(("isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.v1".to_owned(), ready));
        }
        if self.earth_search_catalogue_enabled {
            observations.push(("element84.earth-search.landsat-c2-l2.v1".to_owned(), ready));
        }
        if self.mpc_sentinel2_catalogue_enabled {
            observations.push(("planetary-computer.sentinel-2-l2a.v1".to_owned(), ready));
        }
        if self.earth_search_sentinel2_catalogue_enabled {
            observations.push(("element84.earth-search.sentinel-2-l2a.v1".to_owned(), ready));
        }
        if self.mpc_reviewed_catalogue_enabled {
            observations.extend(
                [MPC_SENTINEL1_GRD_PROFILE_ID, MPC_COPDEM_GLO30_PROFILE_ID]
                    .into_iter()
                    .map(|binding| (binding.to_owned(), ready)),
            );
        }
        if self.mpc_hls_catalogue_enabled {
            observations.extend(
                [
                    "planetary-computer.hls2-l30.v1",
                    "planetary-computer.hls2-s30.v1",
                ]
                .into_iter()
                .map(|binding| (binding.to_owned(), ready)),
            );
        }
        observations
    }

    pub fn cancel_request(&self, request_id: &str) -> bool {
        let asf_cancelled = self.cancellations.cancel(request_id);
        let catalogue_cancelled = self.catalogue_requests.cancel(request_id);
        asf_cancelled || catalogue_cancelled
    }

    /// True while a catalogue lease is held from request registration through
    /// publication. Callers use this to avoid replacing the owning Dispatcher
    /// while its cancellation state is still live.
    pub fn has_active_catalogue_requests(&self) -> bool {
        self.catalogue_requests.has_active_requests()
    }

    pub fn search_sar_for_application(
        &self,
        request_id: &str,
        request: &SarRemoteSearchRequest,
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.search_sar_for_application_with_progress(request_id, request, &|_| {})
    }

    pub fn search_sar_for_application_with_progress(
        &self,
        request_id: &str,
        request: &SarRemoteSearchRequest,
        progress: &dyn Fn(SarSearchProviderProgress),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        let cancellation = self.cancellations.try_register(request_id).ok_or_else(|| {
            ContractError::new(
                "ASF_REQUEST_DUPLICATE",
                "ASF 检索请求已在运行",
                Map::from_iter([(
                    "reason".to_owned(),
                    Value::String("duplicate_request_id".to_owned()),
                )]),
            )
            .expect("bounded duplicate ASF request context is safe")
        })?;
        let result = self.asf_search.search_sar_with_progress(
            request,
            &|| cancellation.load(Ordering::SeqCst),
            progress,
        );
        self.cancellations.finish(request_id);
        result
    }

    pub fn count_sar_for_application(
        &self,
        request_id: &str,
        request: &SarSearchCountRequest,
    ) -> ProviderResult<SarSearchCountResponse> {
        let cancellation = self.cancellations.try_register(request_id).ok_or_else(|| {
            ContractError::new(
                "ASF_REQUEST_DUPLICATE",
                "ASF 计数请求已在运行",
                Map::from_iter([(
                    "reason".to_owned(),
                    Value::String("duplicate_request_id".to_owned()),
                )]),
            )
            .expect("bounded duplicate ASF count context is safe")
        })?;
        let result = self
            .asf_search
            .count_sar(request, &|| cancellation.load(Ordering::SeqCst));
        self.cancellations.finish(request_id);
        result
    }

    pub fn register_catalog_search_for_application(
        &self,
        request_id: &str,
        query: &CatalogSearchQuery,
    ) -> ProviderResult<RegisteredCatalogSearchForApplication> {
        self.register_catalog_search_candidates_for_application(
            request_id,
            std::slice::from_ref(query),
        )
    }

    /// Validate every candidate before acquiring one request lease. Production
    /// fallback can therefore never introduce an unreviewed provider after the
    /// first network failure, and cancellation remains active between attempts.
    pub fn register_catalog_search_candidates_for_application(
        &self,
        request_id: &str,
        queries: &[CatalogSearchQuery],
    ) -> ProviderResult<RegisteredCatalogSearchForApplication> {
        catalogue_diagnostic(request_id, "dispatcher_enter");
        if queries.is_empty() {
            return Err(ContractError::new(
                "CATALOGUE_ROUTE_UNAVAILABLE",
                "没有可执行的数据来源",
                Map::new(),
            )
            .expect("empty provider candidate context is safe"));
        }
        if queries.len() > MAX_CATALOGUE_SOURCE_ATTEMPTS {
            return Err(ContractError::new(
                "CATALOGUE_ROUTE_UNAVAILABLE",
                "可执行数据来源数量超过单次请求上限",
                Map::new(),
            )
            .expect("bounded provider candidate limit context is safe"));
        }
        let catalog_search = self.catalog_search.as_ref().ok_or_else(|| {
            ContractError::new(
                "CATALOGUE_CAPABILITY_UNAVAILABLE",
                "当前运行时未注入目录检索能力",
                Map::new(),
            )
            .expect("empty catalogue capability context is safe")
        })?;
        let route_use = self.catalogue_route_use()?;
        if queries.len() > 1
            && (self.offline_stac_fixture.is_some() || self.catalogue_live_acceptance_candidate)
        {
            return Err(ContractError::new(
                "CATALOGUE_ROUTE_UNAVAILABLE",
                "验收或测试路由不允许自动换源",
                Map::new(),
            )
            .expect("fixture fallback rejection is safe"));
        }
        for query in queries {
            if let Some(admission) = self
                .offline_stac_fixture
                .as_deref()
                .filter(|a| a.targets(query))
            {
                admission.validate_query(query, route_use)?;
            } else {
                let route = if self.catalogue_live_acceptance_candidate {
                    #[cfg(feature = "usgs-m2m-live-acceptance")]
                    {
                        DatasetRoutePolicyRegistry.resolve_live_acceptance_candidate(
                            &query.logical_dataset,
                            &query.provider_id,
                            &query.native_collection_id,
                        )
                    }
                    #[cfg(not(feature = "usgs-m2m-live-acceptance"))]
                    {
                        unreachable!("live acceptance candidate is feature-gated")
                    }
                } else {
                    DatasetRoutePolicyRegistry.resolve(
                        &query.logical_dataset,
                        &query.provider_id,
                        &query.native_collection_id,
                        route_use,
                    )
                };
                route.map_err(|_| {
                    ContractError::new(
                        "CATALOGUE_ROUTE_UNAVAILABLE",
                        "目录请求不匹配已启用的数据集路由",
                        Map::new(),
                    )
                    .expect("empty catalogue route context is safe")
                })?;
            }
        }
        catalogue_diagnostic(request_id, "route_validated");
        let lease = self.catalogue_requests.register(request_id)?;
        catalogue_diagnostic(request_id, "lease_registered");
        Ok(RegisteredCatalogSearchForApplication {
            catalog_search: Arc::clone(catalog_search),
            queries: queries.to_vec(),
            request_id: request_id.to_owned(),
            lease,
        })
    }

    pub fn search_catalog_for_application(
        &self,
        request_id: &str,
        query: &CatalogSearchQuery,
    ) -> ProviderResult<CatalogSearchForApplication> {
        self.register_catalog_search_for_application(request_id, query)?
            .execute()
    }

    pub fn dispatch(&self, request: SidecarRequest) -> SidecarResponse {
        if let Err(reason) = request.validate() {
            return error_response(
                safe_request_id(&request.request_id),
                "V3_PROTOCOL_INVALID_REQUEST",
                "Sidecar 请求格式无效",
                Map::from_iter([("reason".to_string(), json!(reason))]),
            );
        }
        if request.protocol_major != V3_PROTOCOL_MAJOR
            || !(V3_PROTOCOL_MIN_MINOR..=V3_PROTOCOL_MINOR).contains(&request.protocol_minor)
        {
            return error_response(
                request.request_id,
                "V3_PROTOCOL_VERSION_UNSUPPORTED",
                "Sidecar 协议版本不兼容",
                Map::from_iter([
                    ("protocol_major".to_string(), json!(request.protocol_major)),
                    ("protocol_minor".to_string(), json!(request.protocol_minor)),
                ]),
            );
        }

        let is_asf_operation = matches!(
            &request.operation,
            SidecarOperation::AsfNormalize | SidecarOperation::AsfCacheDecide
        );
        if is_asf_operation && request.protocol_minor < 3 {
            return error_response(
                request.request_id,
                "V3_SIDECAR_CAPABILITY_UNAVAILABLE",
                "当前协商的 Sidecar 协议不支持 ASF Rust 能力",
                Map::new(),
            );
        }
        if request.operation == SidecarOperation::AsfSearch && request.protocol_minor < 4 {
            return error_response(
                request.request_id,
                "V3_SIDECAR_CAPABILITY_UNAVAILABLE",
                "当前协商的 Sidecar 协议不支持 ASF 实时检索",
                Map::new(),
            );
        }
        if request.operation == SidecarOperation::AsfSearchCached && request.protocol_minor < 7 {
            return error_response(
                request.request_id,
                "V3_SIDECAR_CAPABILITY_UNAVAILABLE",
                "当前协商的 Sidecar 协议不支持 ASF 持久缓存检索",
                Map::new(),
            );
        }
        if matches!(
            request.operation,
            SidecarOperation::SarSearch | SidecarOperation::ProviderCapabilities
        ) && request.protocol_minor < 8
        {
            return capability_unavailable(request.request_id);
        }
        if matches!(
            request.operation,
            SidecarOperation::StoreSearchBatch
                | SidecarOperation::StoreCreateTask
                | SidecarOperation::StoreUpdateTask
                | SidecarOperation::StoreAppendTask
                | SidecarOperation::StoreArchiveTask
                | SidecarOperation::StoreDeleteHistory
                | SidecarOperation::StoreRestore
        ) && request.protocol_minor < 5
        {
            return error_response(
                request.request_id,
                "V3_SIDECAR_CAPABILITY_UNAVAILABLE",
                "当前协商的 Sidecar 协议不支持 SQLite 任务存储",
                Map::new(),
            );
        }
        if request.operation == SidecarOperation::StoreUpdateWorkbench && request.protocol_minor < 6
        {
            return capability_unavailable(request.request_id);
        }
        if request.operation == SidecarOperation::StoreUpdateTaskView && request.protocol_minor < 10
        {
            return capability_unavailable(request.request_id);
        }
        if matches!(
            request.operation,
            SidecarOperation::DownloadWake | SidecarOperation::DownloadSnapshot
        ) && request.protocol_minor < 5
        {
            return capability_unavailable(request.request_id);
        }

        let negotiated_minor = request.protocol_minor;
        let carries_asf_scene_metadata = matches!(
            &request.operation,
            SidecarOperation::AsfNormalize
                | SidecarOperation::AsfCacheDecide
                | SidecarOperation::AsfSearch
                | SidecarOperation::AsfSearchCached
        );
        let carries_existing_sar_asset_wire =
            matches!(&request.operation, SidecarOperation::SarSearch);
        let mut response = match request.operation {
            SidecarOperation::Handshake => success_response(
                request.request_id,
                service_info(request.protocol_minor, self.runtime_capabilities),
            ),
            SidecarOperation::Health
                if request.payload.get("probe").and_then(Value::as_str) == Some("startup")
                    && request.protocol_minor >= 16
                    && request
                        .payload
                        .get("require_store")
                        .and_then(Value::as_bool)
                        == Some(true) =>
            {
                match self.store.as_ref() {
                    Some(store) => dispatch_startup_health(store, request.request_id),
                    None => capability_unavailable(request.request_id),
                }
            }
            SidecarOperation::Health => success_response(
                request.request_id,
                Map::from_iter([("status".to_string(), json!("healthy"))]),
            ),
            SidecarOperation::Cancel => self.cancel(request.request_id, &request.payload),
            SidecarOperation::OrbitParse => orbit_parse(request.request_id, &request.payload),
            SidecarOperation::OrbitMatch => orbit_match(request.request_id, &request.payload),
            SidecarOperation::AsfNormalize => {
                self.asf_normalize(request.request_id, &request.payload)
            }
            SidecarOperation::AsfCacheDecide => {
                self.asf_cache_decide(request.request_id, &request.payload)
            }
            SidecarOperation::AsfSearch => self.asf_search(request.request_id, &request.payload),
            SidecarOperation::AsfSearchCached => {
                self.asf_search_cached(request.request_id, &request.payload)
            }
            SidecarOperation::SarCount => self.sar_count(request.request_id, &request.payload),
            SidecarOperation::SarSearch => self.sar_search(request.request_id, &request.payload),
            SidecarOperation::ProviderCapabilities => {
                typed_success_response(request.request_id, &self.provider_capabilities())
            }
            SidecarOperation::DownloadWake => match &self.downloads {
                Some(downloads) => {
                    downloads.wake();
                    success_response(
                        request.request_id,
                        Map::from_iter([("status".to_string(), json!("woken"))]),
                    )
                }
                None => capability_unavailable(request.request_id),
            },
            SidecarOperation::DownloadSnapshot => {
                match (self.downloads.as_ref(), self.store.as_ref()) {
                    (Some(_), Some(store)) => {
                        dispatch_download_snapshot(store, request.request_id, &request.payload)
                    }
                    _ => capability_unavailable(request.request_id),
                }
            }
            SidecarOperation::StoreSearchBatch
            | SidecarOperation::StoreUpdateWorkbench
            | SidecarOperation::StoreCreateTask
            | SidecarOperation::StoreUpdateTask
            | SidecarOperation::StoreUpdateTaskView
            | SidecarOperation::StoreAppendTask
            | SidecarOperation::StoreArchiveTask
            | SidecarOperation::StoreDeleteHistory
            | SidecarOperation::StoreRestore => match self.store.as_ref() {
                Some(store) => dispatch_store(
                    store,
                    request.operation,
                    request.request_id,
                    &request.payload,
                ),
                None => capability_unavailable(request.request_id),
            },
            SidecarOperation::Shutdown => {
                self.shutting_down.store(true, Ordering::SeqCst);
                success_response(
                    request.request_id,
                    Map::from_iter([("status".to_string(), json!("shutting_down"))]),
                )
            }
        };
        if carries_asf_scene_metadata && negotiated_minor < 15 {
            strip_asf_preview_metadata(&mut response.result);
        }
        if carries_existing_sar_asset_wire {
            strip_catalogue_only_asset_fields(&mut response.result);
        }
        response
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    fn cancel(&self, request_id: String, payload: &Map<String, Value>) -> SidecarResponse {
        let Some(target_id) = payload.get("request_id").and_then(Value::as_str) else {
            return error_response(
                request_id,
                "V3_PROTOCOL_INVALID_REQUEST",
                "取消请求缺少 request_id",
                Map::new(),
            );
        };
        let cancelled = self.cancellations.cancel(target_id);
        success_response(
            request_id,
            Map::from_iter([
                ("request_id".to_string(), json!(target_id)),
                ("cancelled".to_string(), json!(cancelled)),
            ]),
        )
    }

    fn asf_normalize(&self, request_id: String, payload: &Map<String, Value>) -> SidecarResponse {
        let request: AsfNormalizeRequest = match decode_payload(payload) {
            Ok(request) => request,
            Err(reason) => return invalid_payload_response(request_id, reason),
        };
        let cancellation = self.cancellations.register(&request_id);
        let result = if cancellation.load(Ordering::SeqCst) {
            Err(cancelled_error())
        } else {
            normalize_asf_features(&request)
        };
        self.cancellations.finish(&request_id);
        typed_provider_response(request_id, result)
    }

    fn asf_cache_decide(
        &self,
        request_id: String,
        payload: &Map<String, Value>,
    ) -> SidecarResponse {
        let request: AsfCacheDecisionRequest = match decode_payload(payload) {
            Ok(request) => request,
            Err(reason) => return invalid_payload_response(request_id, reason),
        };
        let cancellation = self.cancellations.register(&request_id);
        let result = if cancellation.load(Ordering::SeqCst) {
            Err(cancelled_error())
        } else {
            decide_asf_cache(&request)
        };
        self.cancellations.finish(&request_id);
        typed_provider_response(request_id, result)
    }

    fn asf_search(&self, request_id: String, payload: &Map<String, Value>) -> SidecarResponse {
        let request: AsfRemoteSearchRequest = match decode_payload(payload) {
            Ok(request) => request,
            Err(reason) => return invalid_payload_response(request_id, reason),
        };
        let cancellation = self.cancellations.register(&request_id);
        let result = self
            .asf_search
            .search(&request, &|| cancellation.load(Ordering::SeqCst));
        self.cancellations.finish(&request_id);
        typed_provider_response(request_id, result)
    }

    fn asf_search_cached(
        &self,
        request_id: String,
        payload: &Map<String, Value>,
    ) -> SidecarResponse {
        let request: AsfCachedSearchRequest = match decode_payload(payload) {
            Ok(request) => request,
            Err(reason) => return invalid_payload_response(request_id, reason),
        };
        let cancellation = self.cancellations.register(&request_id);
        let result = match self.store.as_ref() {
            Some(store) => {
                execute_cached_asf_search(self.asf_search.as_ref(), store, &request, &|| {
                    cancellation.load(Ordering::SeqCst)
                })
            }
            None => Err(ContractError::new(
                "V3_SIDECAR_CAPABILITY_UNAVAILABLE",
                "当前运行时不提供持久化 ASF 缓存检索",
                Map::new(),
            )
            .expect("empty capability context is safe")),
        };
        self.cancellations.finish(&request_id);
        typed_provider_response(request_id, result)
    }

    fn sar_search(&self, request_id: String, payload: &Map<String, Value>) -> SidecarResponse {
        let request: SarRemoteSearchRequest = match decode_payload(payload) {
            Ok(request) => request,
            Err(reason) => return invalid_payload_response(request_id, reason),
        };
        let result = self.search_sar_for_application(&request_id, &request);
        typed_provider_response(request_id, result)
    }

    fn sar_count(&self, request_id: String, payload: &Map<String, Value>) -> SidecarResponse {
        let request: SarSearchCountRequest = match decode_payload(payload) {
            Ok(request) => request,
            Err(reason) => return invalid_payload_response(request_id, reason),
        };
        let result = self.count_sar_for_application(&request_id, &request);
        typed_provider_response(request_id, result)
    }
}

fn catalogue_diagnostic(request_id: &str, stage: &str) {
    if std::env::var_os("INSAR_DIAGNOSTIC_CATALOGUE").is_some() {
        eprintln!("INSAR_CATALOGUE_DIAGNOSTIC request_id={request_id} stage={stage}");
    }
}

const CATALOGUE_ROUTE_UNAVAILABLE_REASON: &str = "CATALOGUE_ROUTE_UNAVAILABLE";

fn sar_source_capabilities(
    mut sources: Vec<SarSourceCapability>,
    provider_available: bool,
    provider_reason: Option<&str>,
    route_use: Option<RouteUse>,
    bhoonidhi_available: bool,
) -> Vec<SarSourceCapability> {
    for source in &mut sources {
        match source.search_kind {
            SarSourceSearchKind::LegacyAsfSearch => {
                source.available = provider_available;
                source.disabled_reason = provider_reason.map(str::to_owned);
            }
            SarSourceSearchKind::TypedCatalogProfile => {
                let route_available = route_use.is_some_and(|use_case| {
                    source
                        .logical_dataset
                        .as_deref()
                        .zip(source.provider_id.as_deref())
                        .zip(source.native_collection_id.as_deref())
                        .is_some_and(|((logical_dataset, source_provider_id), collection_id)| {
                            DatasetRoutePolicyRegistry
                                .resolve(
                                    logical_dataset,
                                    source_provider_id,
                                    collection_id,
                                    use_case,
                                )
                                .is_ok()
                        })
                });
                let profile_matches = source
                    .logical_dataset
                    .as_deref()
                    .zip(source.provider_id.as_deref())
                    .zip(source.native_collection_id.as_deref())
                    .zip(source.collection_profile_id.as_deref())
                    .is_some_and(
                        |(((logical_dataset, source_provider_id), collection_id), profile_id)| {
                            CollectionProfileRegistry
                                .for_source(logical_dataset, source_provider_id, collection_id)
                                .is_some_and(|profile| {
                                    profile.profile_id == profile_id
                                        && profile.provider_id == source_provider_id
                                        && profile.native_collection_id == collection_id
                                })
                        },
                    );
                let source_provider_available =
                    if source.provider_id.as_deref() == Some(BHOONIDHI_PROVIDER_ID) {
                        bhoonidhi_available
                    } else {
                        provider_available
                    };
                source.available = source_provider_available && route_available && profile_matches;
                source.disabled_reason =
                    (!source.available).then(|| CATALOGUE_ROUTE_UNAVAILABLE_REASON.to_owned());
            }
        }
    }
    sources
}

impl RequestDispatcher for Dispatcher {
    fn dispatch(&self, request: SidecarRequest) -> SidecarResponse {
        Dispatcher::dispatch(self, request)
    }

    fn classify(&self, operation: &SidecarOperation) -> DispatchClass {
        match operation {
            SidecarOperation::Handshake
            | SidecarOperation::Health
            | SidecarOperation::Cancel
            | SidecarOperation::Shutdown => DispatchClass::Control,
            SidecarOperation::OrbitParse
            | SidecarOperation::OrbitMatch
            | SidecarOperation::AsfNormalize
            | SidecarOperation::AsfCacheDecide
            | SidecarOperation::AsfSearch
            | SidecarOperation::AsfSearchCached
            | SidecarOperation::SarCount
            | SidecarOperation::SarSearch
            | SidecarOperation::ProviderCapabilities
            | SidecarOperation::StoreSearchBatch
            | SidecarOperation::StoreUpdateWorkbench
            | SidecarOperation::StoreCreateTask
            | SidecarOperation::StoreUpdateTask
            | SidecarOperation::StoreUpdateTaskView
            | SidecarOperation::StoreAppendTask
            | SidecarOperation::StoreArchiveTask
            | SidecarOperation::StoreDeleteHistory
            | SidecarOperation::StoreRestore => DispatchClass::Work,
            SidecarOperation::DownloadWake | SidecarOperation::DownloadSnapshot => {
                DispatchClass::Work
            }
        }
    }

    fn is_shutting_down(&self) -> bool {
        Dispatcher::is_shutting_down(self)
    }
}

fn cancelled_error() -> ContractError {
    ContractError::new("DL001", "ASF 请求已取消", Map::new())
        .expect("empty cancellation context is safe")
}

fn orbit_parse(request_id: String, payload: &Map<String, Value>) -> SidecarResponse {
    let request: OrbitParseRequest = match decode_payload(payload) {
        Ok(request) => request,
        Err(reason) => return invalid_payload_response(request_id, reason),
    };
    if let Err(reason) = request.validate() {
        return invalid_payload_response(request_id, reason);
    }
    typed_success_response(request_id, &parse_orbit_filenames(&request))
}

fn orbit_match(request_id: String, payload: &Map<String, Value>) -> SidecarResponse {
    let request: OrbitMatchRequest = match decode_payload(payload) {
        Ok(request) => request,
        Err(reason) => return invalid_payload_response(request_id, reason),
    };
    if let Err(reason) = request.validate() {
        return invalid_payload_response(request_id, reason);
    }
    typed_success_response(request_id, &match_orbits(&request))
}

fn decode_payload<T: DeserializeOwned>(payload: &Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(payload.clone())).map_err(|error| error.to_string())
}

pub(crate) fn invalid_payload_response(request_id: String, reason: String) -> SidecarResponse {
    error_response(
        request_id,
        "V3_PROTOCOL_INVALID_REQUEST",
        "Sidecar 请求载荷无效",
        Map::from_iter([("reason".to_string(), Value::String(reason))]),
    )
}

pub(crate) fn typed_success_response<T: Serialize>(
    request_id: String,
    result: &T,
) -> SidecarResponse {
    let result = serde_json::to_value(result)
        .expect("generated sidecar result serializes")
        .as_object()
        .cloned()
        .expect("generated sidecar result is an object");
    success_response(request_id, result)
}

fn typed_provider_response<T: Serialize>(
    request_id: String,
    result: ProviderResult<T>,
) -> SidecarResponse {
    match result {
        Ok(result) => typed_success_response(request_id, &result),
        Err(error) => SidecarResponse {
            protocol_major: V3_PROTOCOL_MAJOR,
            protocol_minor: V3_PROTOCOL_MINOR,
            request_id,
            ok: false,
            result: Map::new(),
            error: Some(error),
            terminal: true,
        },
    }
}

pub(crate) fn safe_request_id(value: &str) -> String {
    if !value.is_empty() && value.chars().count() <= 128 && !value.chars().any(char::is_control) {
        value.to_string()
    } else {
        "unknown".to_string()
    }
}

pub(crate) fn success_response(request_id: String, result: Map<String, Value>) -> SidecarResponse {
    SidecarResponse {
        protocol_major: V3_PROTOCOL_MAJOR,
        protocol_minor: V3_PROTOCOL_MINOR,
        request_id,
        ok: true,
        result,
        error: None,
        terminal: true,
    }
}

pub(crate) fn error_response(
    request_id: String,
    code: &str,
    user_message: &str,
    debug_context: Map<String, Value>,
) -> SidecarResponse {
    SidecarResponse {
        protocol_major: V3_PROTOCOL_MAJOR,
        protocol_minor: V3_PROTOCOL_MINOR,
        request_id,
        ok: false,
        result: Map::new(),
        error: Some(ContractError {
            code: code.to_string(),
            user_message: user_message.to_string(),
            debug_context,
        }),
        terminal: true,
    }
}

fn service_info(negotiated_minor: u64, runtime: ServiceRuntimeCapabilities) -> Map<String, Value> {
    let mut capabilities = vec!["orbit_parse".to_string(), "orbit_match".to_string()];
    if negotiated_minor >= 3 {
        capabilities.extend(["asf_normalize".to_string(), "asf_cache_decide".to_string()]);
    }
    if negotiated_minor >= 4 {
        capabilities.push("asf_search".to_string());
    }
    if negotiated_minor >= 5 {
        capabilities.extend([
            "store_search_batch".to_string(),
            "store_create_task".to_string(),
            "store_update_task".to_string(),
            "store_append_task".to_string(),
            "store_archive_task".to_string(),
            "store_delete_history".to_string(),
            "store_restore".to_string(),
        ]);
        if runtime.downloads_enabled {
            capabilities.extend(["download_wake".to_string(), "download_snapshot".to_string()]);
        }
    }
    if negotiated_minor >= 6 {
        capabilities.push("store_update_workbench".to_string());
    }
    if negotiated_minor >= 7 {
        capabilities.push("asf_search_cached".to_string());
    }
    if negotiated_minor >= 8 {
        capabilities.extend([
            "sar_search".to_string(),
            "provider_capabilities".to_string(),
        ]);
    }
    if negotiated_minor >= 9 {
        capabilities.push("sar_count".to_string());
    }
    if negotiated_minor >= 10 {
        capabilities.push("store_update_task_view".to_string());
    }
    if negotiated_minor >= 14 && runtime.execution_policy_supported {
        capabilities.push("asset_execution_policy_v1".to_string());
    }
    if negotiated_minor >= 14 && runtime.resolver_runtime_ready {
        capabilities.push("resolved_asset_transfer_v1".to_string());
    }
    if negotiated_minor >= 15 {
        capabilities.push("asf_cached_preview_metadata_v1".to_string());
    }
    if negotiated_minor >= 16 {
        capabilities.push("store_startup_readiness_v1".to_string());
    }
    if negotiated_minor >= 19 {
        capabilities.extend([
            "supports_catalog_result_limit_v1".to_string(),
            "supports_application_search_progress_v2".to_string(),
        ]);
    }
    let info = SidecarServiceInfo {
        service_version: env!("CARGO_PKG_VERSION").to_string(),
        contract_version: V3_CONTRACT_VERSION.to_string(),
        protocol_major: V3_PROTOCOL_MAJOR,
        protocol_minor: V3_PROTOCOL_MINOR,
        minimum_protocol_minor: V3_PROTOCOL_MIN_MINOR,
        build_id: option_env!("INSAR_BUILD_ID")
            .unwrap_or("development")
            .to_string(),
        capabilities,
    };
    serde_json::to_value(info)
        .expect("generated service info serializes")
        .as_object()
        .cloned()
        .expect("service info is an object")
}

fn strip_asf_preview_metadata(value: &mut Map<String, Value>) {
    const NEW_FIELDS: [&str; 10] = [
        "provider_product_id",
        "file_name",
        "processing_level",
        "acquisition_stop",
        "browse_url",
        "thumbnail_url",
        "checksum_algorithm",
        "checksum_value",
        "center_latitude",
        "center_longitude",
    ];

    fn strip(value: &mut Value) {
        match value {
            Value::Object(object) => {
                for field in NEW_FIELDS {
                    object.remove(field);
                }
                for child in object.values_mut() {
                    strip(child);
                }
            }
            Value::Array(values) => {
                for child in values {
                    strip(child);
                }
            }
            _ => {}
        }
    }

    for child in value.values_mut() {
        strip(child);
    }
}

fn strip_catalogue_only_asset_fields(value: &mut Map<String, Value>) {
    let Some(products) = value.get_mut("products").and_then(Value::as_array_mut) else {
        return;
    };
    for product in products {
        let Some(product) = product.as_object_mut() else {
            continue;
        };
        let Some(assets) = product.get_mut("assets").and_then(Value::as_array_mut) else {
            continue;
        };
        for asset in assets {
            if let Some(asset) = asset.as_object_mut() {
                asset.remove("native_roles");
            }
        }
    }
}

fn capability_unavailable(request_id: String) -> SidecarResponse {
    error_response(
        request_id,
        "V3_SIDECAR_CAPABILITY_UNAVAILABLE",
        "Rust 下载引擎尚未启用",
        Map::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bhoonidhi_source_is_projected_and_requires_its_runtime_route() {
        let sources = ResourceRegistry::embedded_ref()
            .expect("embedded Resource Registry must be valid")
            .sar_source_capabilities_for_providers(&[BHOONIDHI_PROVIDER_ID]);
        assert!(sources.iter().any(|source| {
            source.provider_id.as_deref() == Some(BHOONIDHI_PROVIDER_ID)
                && source.selector == "NISAR_SSAR"
        }));

        let unavailable = sar_source_capabilities(
            sources.clone(),
            true,
            None,
            Some(RouteUse::Production),
            false,
        );
        let source = unavailable
            .iter()
            .find(|source| source.provider_id.as_deref() == Some(BHOONIDHI_PROVIDER_ID))
            .expect("Bhoonidhi source is present");
        assert!(!source.available);
        assert_eq!(
            source.disabled_reason.as_deref(),
            Some(CATALOGUE_ROUTE_UNAVAILABLE_REASON)
        );

        let available =
            sar_source_capabilities(sources, true, None, Some(RouteUse::Production), true);
        assert!(available
            .iter()
            .find(|source| source.provider_id.as_deref() == Some(BHOONIDHI_PROVIDER_ID))
            .is_some_and(|source| source.available));
    }
}
