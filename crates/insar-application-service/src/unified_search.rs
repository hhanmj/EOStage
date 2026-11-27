use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use insar_core::{
    ApplicationSearchProgress, ApplicationSearchProgressStage, CapabilityKind, CatalogSearchQuery,
    FrozenSearchStats, ResourceDescriptor, ResourceRegistry, SarRemoteSearchRequest,
    SarSearchProjectionKind, SarSearchQuery, SearchFilterSchemaCatalog, SearchProgress,
    SearchProgressStage, SourceSearchMode, StaticCapabilityLifecycle, TypedError,
    UnifiedSearchRequest,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    catalogue_intent::CatalogSearchIntent, ApplicationCatalogIntentSearchAndFreezeRequest,
    ApplicationCatalogSearchAndFreezeRequest, ApplicationSarSearchAndFreezeRequest,
    ApplicationService, ApplicationServiceError, CoreServiceSnapshot,
};

pub(crate) const SAR_BACKGROUND_CONTINUATION_CAP: u64 = 2_000;
const SAR_FIRST_PAGE_SIZE: u64 = 250;
pub(crate) const OPTICAL_BACKGROUND_CONTINUATION_CAP: u64 = 2_000;
const OPTICAL_FIRST_PAGE_SIZE: u64 = insar_store::PROGRESSIVE_PUBLICATION_ROWS;

pub type SearchBatchSyncListener = Arc<dyn Fn(Value) + Send + Sync>;

pub(crate) struct SarContinuationPlan {
    pub(crate) request_id: String,
    pub(crate) batch_id: String,
    pub(crate) created_at: String,
    pub(crate) base_query: SarSearchQuery,
    pub(crate) loaded_count: u64,
    pub(crate) remote_total: Option<u64>,
    pub(crate) oldest_dt: String,
    pub(crate) seen_ids: HashSet<String>,
    pub(crate) stats: FrozenSearchStats,
    pub(crate) continuation_cap: u64,
}

pub(crate) enum SarFirstPageOutcome {
    Sealed(CoreServiceSnapshot),
    StagedForContinuation(SarContinuationPlan),
}

const TERMINAL_RETENTION: Duration = Duration::from_secs(30 * 60);
const TERMINAL_CAP: usize = 256;

struct ProgressEntry {
    value: SearchProgress,
    completed_at: Option<Instant>,
    order: u64,
}

#[derive(Default)]
pub(crate) struct ProgressEntries {
    entries: HashMap<String, ProgressEntry>,
    order: u64,
}

impl ProgressEntries {
    pub(crate) fn prune(&mut self, now: Instant) {
        self.entries.retain(|_, item| {
            !item.value.terminal
                || item
                    .completed_at
                    .is_some_and(|at| now.saturating_duration_since(at) <= TERMINAL_RETENTION)
        });
        let mut terminal = self
            .entries
            .iter()
            .filter(|(_, item)| item.value.terminal)
            .map(|(id, item)| (id.clone(), item.order))
            .collect::<Vec<_>>();
        let excess = terminal.len().saturating_sub(TERMINAL_CAP);
        if excess == 0 {
            return;
        }
        terminal.sort_by(|(left_id, left_order), (right_id, right_order)| {
            left_order
                .cmp(right_order)
                .then_with(|| left_id.cmp(right_id))
        });
        for (id, _) in terminal.into_iter().take(excess) {
            self.entries.remove(&id);
        }
    }

    fn insert_at(&mut self, id: String, value: SearchProgress, now: Instant) {
        let previous = self.entries.get(&id);
        let completed_at = if value.terminal {
            previous.and_then(|entry| entry.completed_at).or(Some(now))
        } else {
            None
        };
        self.order = self.order.wrapping_add(1);
        self.entries.insert(
            id,
            ProgressEntry {
                value,
                completed_at,
                order: self.order,
            },
        );
        self.prune(now);
    }

    fn get(&self, id: &str) -> Option<&SearchProgress> {
        self.entries.get(id).map(|entry| &entry.value)
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &SearchProgress> {
        self.entries.values().map(|entry| &entry.value)
    }

    fn contains_key(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }
}

#[derive(Default)]
pub(crate) struct UnifiedSearchRegistry {
    pub(crate) progress: Mutex<ProgressEntries>,
    /// Provider facts of a running unified catalogue search, taken when the
    /// search ends. Internal only: they are neither persisted nor returned.
    catalogue_outcomes: Mutex<HashMap<String, CatalogueOutcome>>,
    /// Batches the latest searches froze, by request id (newest last, at
    /// most TERMINAL_CAP). In memory only: an Agent run record keeps its own
    /// copy, and this is only its fallback.
    published: Mutex<VecDeque<(String, String)>>,
    cancelled: Mutex<HashSet<String>>,
    active_continuations: Mutex<HashSet<String>>,
    sync_listeners: Mutex<Vec<SearchBatchSyncListener>>,
}

/// What the provider response said about the cap, for the terminal message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CatalogueOutcome {
    pub(crate) remote_exhausted: bool,
    /// More than one spatial part means tiles were merged by product ID, so
    /// the kept set is not newest-first even if each part asked for it.
    pub(crate) transport_part_count: u64,
}

impl UnifiedSearchRegistry {
    /// Called by the catalogue freeze for every provider response; only a
    /// request started through `search_start` keeps its facts.
    pub(crate) fn record_catalogue_outcome(
        &self,
        request_id: &str,
        stats: &insar_core::CatalogSearchStats,
    ) {
        let registered = self
            .progress
            .lock()
            .is_ok_and(|progress| progress.contains_key(request_id));
        if !registered {
            return;
        }
        if let Ok(mut outcomes) = self.catalogue_outcomes.lock() {
            outcomes.insert(
                request_id.to_owned(),
                CatalogueOutcome {
                    remote_exhausted: stats.remote_exhausted,
                    transport_part_count: stats.transport_part_count,
                },
            );
        }
    }

    fn record_published(&self, request_id: &str, batch_id: &str) {
        if let Ok(mut published) = self.published.lock() {
            published.retain(|(request, _)| request != request_id);
            published.push_back((request_id.to_owned(), batch_id.to_owned()));
            while published.len() > TERMINAL_CAP {
                published.pop_front();
            }
        }
    }

    pub(crate) fn published_batch(&self, request_id: &str) -> Option<String> {
        self.published.lock().ok().and_then(|published| {
            published
                .iter()
                .rev()
                .find(|(request, _)| request == request_id)
                .map(|(_, batch)| batch.clone())
        })
    }

    fn clear_published(&self, request_id: &str) {
        if let Ok(mut published) = self.published.lock() {
            published.retain(|(request, _)| request != request_id);
        }
    }

    fn take_catalogue_outcome(&self, request_id: &str) -> Option<CatalogueOutcome> {
        self.catalogue_outcomes
            .lock()
            .ok()
            .and_then(|mut outcomes| outcomes.remove(request_id))
    }

    fn mark_cancelled(&self, request_id: &str) {
        let should_track = self
            .progress
            .lock()
            .is_ok_and(|progress| progress.get(request_id).is_some_and(|item| !item.terminal))
            || self.has_active_continuation(request_id);
        if !should_track {
            return;
        }
        if let Ok(mut set) = self.cancelled.lock() {
            set.insert(request_id.to_owned());
        }
    }

    fn prune_stale_cancellations(&self) {
        let Ok(progress) = self.progress.lock() else {
            return;
        };
        let Ok(active) = self.active_continuations.lock() else {
            return;
        };
        if let Ok(mut cancelled) = self.cancelled.lock() {
            cancelled.retain(|id| {
                progress.get(id).is_some_and(|item| !item.terminal) || active.contains(id)
            });
        }
    }

    fn is_cancelled(&self, request_id: &str) -> bool {
        self.cancelled
            .lock()
            .is_ok_and(|set| set.contains(request_id))
    }

    fn clear_cancelled(&self, request_id: &str) {
        if let Ok(mut set) = self.cancelled.lock() {
            set.remove(request_id);
        }
    }

    pub(crate) fn mark_continuation_active(&self, request_id: &str) {
        if let Ok(mut set) = self.active_continuations.lock() {
            set.insert(request_id.to_owned());
        }
    }

    pub(crate) fn finish_continuation(&self, request_id: &str) {
        if let Ok(mut set) = self.active_continuations.lock() {
            set.remove(request_id);
        }
    }

    pub(crate) fn has_active_continuation(&self, request_id: &str) -> bool {
        self.active_continuations
            .lock()
            .is_ok_and(|set| set.contains(request_id))
    }

    fn add_sync_listener(&self, listener: SearchBatchSyncListener) {
        if let Ok(mut listeners) = self.sync_listeners.lock() {
            listeners.push(listener);
        }
    }

    fn emit_sync(&self, payload: Value) {
        let listeners = self
            .sync_listeners
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        for listener in listeners {
            listener(payload.clone());
        }
    }
}

impl ApplicationService {
    pub fn register_search_sync_listener(&self, listener: SearchBatchSyncListener) {
        self.unified_searches.add_sync_listener(listener);
    }

    pub(crate) fn emit_search_batch_sync(&self, payload: Value) {
        self.unified_searches.emit_sync(payload);
    }
}

struct ContinuationGuard {
    service: Arc<ApplicationService>,
    request_id: String,
    resource_id: String,
    completed: bool,
}

impl Drop for ContinuationGuard {
    fn drop(&mut self) {
        self.service
            .unified_searches
            .finish_continuation(&self.request_id);
        self.service
            .unified_searches
            .clear_cancelled(&self.request_id);
        if self.completed {
            return;
        }
        let err = ApplicationServiceError::BusinessRuntimeUnavailable;
        let typed = search_error(&err);
        self.service.update_source_view(|store, at| {
            store.record_source_view_failure(&self.resource_id, &self.request_id, &typed, at)
        });
        if let Ok(mut progress) = self.service.unified_searches.progress.lock() {
            if let Some(existing) = progress.get(&self.request_id).cloned() {
                if !existing.terminal {
                    progress.insert_at(
                        self.request_id.clone(),
                        SearchProgress {
                            stage: SearchProgressStage::Failed,
                            terminal: true,
                            message_zh: typed.message_zh.clone(),
                            error: Some(typed),
                            possibly_incomplete: true,
                            ..existing
                        },
                        Instant::now(),
                    );
                }
            }
        }
    }
}

struct SearchWorkerGuard {
    service: Arc<ApplicationService>,
    registry: Arc<UnifiedSearchRegistry>,
    request_id: String,
    resource_id: String,
    handed_off: bool,
    completed: bool,
}

impl Drop for SearchWorkerGuard {
    fn drop(&mut self) {
        if self.handed_off {
            return;
        }
        self.registry.finish_continuation(&self.request_id);
        self.registry.clear_cancelled(&self.request_id);
        if self.completed {
            return;
        }
        let err = ApplicationServiceError::BusinessRuntimeUnavailable;
        let typed = search_error(&err);
        self.service.update_source_view(|store, at| {
            store.record_source_view_failure(&self.resource_id, &self.request_id, &typed, at)
        });
        if let Ok(mut progress) = self.registry.progress.lock() {
            if let Some(existing) = progress.get(&self.request_id).cloned() {
                if !existing.terminal {
                    progress.insert_at(
                        self.request_id.clone(),
                        SearchProgress {
                            stage: SearchProgressStage::Failed,
                            terminal: true,
                            message_zh: typed.message_zh.clone(),
                            error: Some(typed),
                            possibly_incomplete: false,
                            ..existing
                        },
                        Instant::now(),
                    );
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn finalize_sar_continuation(
    service: &ApplicationService,
    resource_id: &str,
    request_id: &str,
    batch_id: &str,
    created_at: &str,
    continuation_cap: u64,
    loaded_count: u64,
    remote_total: Option<u64>,
    exhausted: bool,
    continuation_error: Option<ApplicationServiceError>,
    mut final_stats: insar_core::FrozenSearchStats,
    continuation_guard: Option<ContinuationGuard>,
) {
    let was_cancelled = service.unified_searches.is_cancelled(request_id);
    let terminal_state = if was_cancelled {
        insar_core::SearchBatchProgressState::Cancelled
    } else if continuation_error.is_some() {
        insar_core::SearchBatchProgressState::Failed
    } else {
        insar_core::SearchBatchProgressState::Complete
    };
    if matches!(
        terminal_state,
        insar_core::SearchBatchProgressState::Cancelled
            | insar_core::SearchBatchProgressState::Failed
    ) {
        final_stats.truncated = true;
    }
    let seal_result =
        service.seal_sar_continuation_staging(request_id, created_at, final_stats, terminal_state);
    if let Err(ref seal_err) = seal_result {
        service.unified_searches.clear_published(request_id);
        if !matches!(
            seal_err,
            ApplicationServiceError::BusinessRuntimeUnavailable
                | ApplicationServiceError::ShuttingDown
        ) {
            let _ = service.abort_catalogue_staging(request_id);
        }
    }

    let (stage, error, sealed_batch_id, possibly_incomplete, message_zh, emit_complete) =
        if was_cancelled {
            if seal_result.is_ok() {
                service.update_source_view(|store, at| {
                    store.record_source_view_success(resource_id, batch_id, Some(request_id), at)
                });
                (
                    SearchProgressStage::Cancelled,
                    None,
                    Some(batch_id.to_owned()),
                    true,
                    format!("检索已取消，已到 {loaded_count} 景结果已保留，可能不完整"),
                    Some(false),
                )
            } else {
                service.update_source_view(|store, at| {
                    store.clear_source_view_pending(resource_id, request_id, at)
                });
                (
                    SearchProgressStage::Cancelled,
                    None,
                    None,
                    true,
                    format!("检索已取消，已到 {loaded_count} 景结果未能保存"),
                    None,
                )
            }
        } else if continuation_error.is_some() || seal_result.is_err() {
            match seal_result {
                Ok(()) => {
                    let err = continuation_error
                        .expect("checked continuation_error.is_some() when seal_result is Ok");
                    let typed = search_error(&err);
                    service.update_source_view(|store, at| {
                        store.record_source_view_success(
                            resource_id,
                            batch_id,
                            Some(request_id),
                            at,
                        )
                    });
                    (
                        SearchProgressStage::Failed,
                        Some(typed),
                        Some(batch_id.to_owned()),
                        true,
                        format!("检索中途失败，已到 {loaded_count} 景结果已保留，可能不完整"),
                        Some(false),
                    )
                }
                Err(seal_err) => {
                    let err = continuation_error.unwrap_or(seal_err);
                    let typed = search_error(&err);
                    service.update_source_view(|store, at| {
                        store.record_source_view_failure(resource_id, request_id, &typed, at)
                    });
                    (
                        SearchProgressStage::Failed,
                        Some(typed),
                        None,
                        true,
                        format!("检索封存失败，已到 {loaded_count} 景结果未能保存"),
                        None,
                    )
                }
            }
        } else {
            service.update_source_view(|store, at| {
                store.record_source_view_success(resource_id, batch_id, Some(request_id), at)
            });
            let incomplete = !exhausted
                && (loaded_count >= continuation_cap
                    || remote_total.is_some_and(|total| total > loaded_count));
            (
                SearchProgressStage::Done,
                None,
                Some(batch_id.to_owned()),
                incomplete,
                format!("已完成检索，共载入 {loaded_count} 景"),
                Some(true),
            )
        };

    if let Ok(mut progress) = service.unified_searches.progress.lock() {
        if let Some(existing) = progress.get(request_id).cloned() {
            progress.insert_at(
                request_id.to_owned(),
                SearchProgress {
                    stage,
                    terminal: true,
                    items_kept: loaded_count,
                    loaded_count: Some(loaded_count),
                    remote_total,
                    error,
                    batch_id: sealed_batch_id,
                    possibly_incomplete,
                    message_zh,
                    ..existing
                },
                Instant::now(),
            );
        }
    }
    if let Some(mut guard) = continuation_guard {
        guard.completed = true;
        drop(guard);
    }
    service.unified_searches.finish_continuation(request_id);
    service.unified_searches.clear_cancelled(request_id);
    if let Some(complete) = emit_complete {
        service.emit_search_batch_sync(json!({
            "action": "search_updated",
            "request_id": request_id,
            "batch_id": batch_id,
            "resource_id": resource_id,
            "loaded_count": loaded_count,
            "total_count": remote_total,
            "complete": complete,
        }));
    }
}

fn run_sar_background_continuation(
    service: Arc<ApplicationService>,
    resource_id: String,
    mut plan: SarContinuationPlan,
) {
    let continuation_guard = ContinuationGuard {
        service: Arc::clone(&service),
        request_id: plan.request_id.clone(),
        resource_id: resource_id.clone(),
        completed: false,
    };
    let mut loaded_count = plan.loaded_count;
    let mut remote_total = plan.remote_total;
    let mut cursor_dt = plan.oldest_dt.clone();
    let mut exhausted = false;
    let mut interrupted = false;
    let mut continuation_error: Option<ApplicationServiceError> = None;

    while loaded_count < plan.continuation_cap {
        if service.unified_searches.is_cancelled(&plan.request_id) {
            interrupted = true;
            break;
        }
        if service
            .shutting_down
            .load(std::sync::atomic::Ordering::Acquire)
        {
            continuation_error = Some(ApplicationServiceError::ShuttingDown);
            interrupted = true;
            break;
        }
        let remaining = plan.continuation_cap.saturating_sub(loaded_count);
        let page_limit = remaining.min(SAR_FIRST_PAGE_SIZE);
        let page_result = service.fetch_next_sar_continuation_page(
            &plan.request_id,
            &plan.base_query,
            &cursor_dt,
            page_limit,
            &mut plan.seen_ids,
        );
        let (new_items, next_oldest, page_total, page_may_have_more) = match page_result {
            Ok(tuple) => tuple,
            Err(err) => {
                continuation_error = Some(err);
                interrupted = true;
                break;
            }
        };
        if let Some(total) = page_total {
            remote_total = Some(remote_total.map_or(total, |prev| prev.max(total)));
        }
        if new_items.is_empty() {
            exhausted = true;
            break;
        }
        let added = u64::try_from(new_items.len()).unwrap_or(0);
        if let Err(err) = service.append_sar_continuation_candidates(&plan.request_id, &new_items) {
            continuation_error = Some(err);
            interrupted = true;
            break;
        }
        loaded_count = loaded_count.saturating_add(added);
        if let Ok(mut progress) = service.unified_searches.progress.lock() {
            if let Some(existing) = progress.get(&plan.request_id).cloned() {
                progress.insert_at(
                    plan.request_id.clone(),
                    SearchProgress {
                        items_kept: loaded_count,
                        loaded_count: Some(loaded_count),
                        remote_total,
                        message_zh: format!("已加载 {loaded_count} 景，正在后台继续拉取..."),
                        ..existing
                    },
                    Instant::now(),
                );
            }
        }
        service.emit_search_batch_sync(json!({
            "action": "search_updated",
            "request_id": plan.request_id,
            "batch_id": plan.batch_id,
            "resource_id": resource_id,
            "loaded_count": loaded_count,
            "total_count": remote_total,
            "complete": false,
        }));
        let Some(next_cursor) = next_oldest else {
            exhausted = true;
            break;
        };
        if next_cursor == cursor_dt || !page_may_have_more {
            exhausted = !page_may_have_more;
            break;
        }
        cursor_dt = next_cursor;
    }

    let mut final_stats = plan.stats;
    final_stats.frozen_candidate_count = loaded_count;
    final_stats.fetched_record_count = Some(loaded_count);
    final_stats.mapped_record_count = Some(loaded_count);
    final_stats.provider_returned_count = loaded_count;
    final_stats.matched_count = remote_total;
    final_stats.truncated = interrupted
        || (!exhausted && loaded_count >= plan.continuation_cap)
        || remote_total.is_some_and(|total| total > loaded_count);

    finalize_sar_continuation(
        &service,
        &resource_id,
        &plan.request_id,
        &plan.batch_id,
        &plan.created_at,
        plan.continuation_cap,
        loaded_count,
        remote_total,
        exhausted,
        continuation_error,
        final_stats,
        Some(continuation_guard),
    );
}

fn invalid(reason: impl Into<String>) -> ApplicationServiceError {
    ApplicationServiceError::InvalidRequest {
        reason: reason.into(),
    }
}

fn asf_query_invalid_guidance(source: &insar_core::ContractError) -> Option<String> {
    let field = source
        .debug_context
        .get("field")
        .and_then(Value::as_str)
        .unwrap_or("");
    let field_label = match field {
        "relative_orbit" | "path" => "Path / 轨道号",
        "frame" => "Frame / 帧号",
        _ => "检索参数",
    };
    let reason = source.debug_context.get("reason").and_then(Value::as_str);
    match reason {
        Some("exact_width") => {
            let width = source
                .debug_context
                .get("exact_width")
                .and_then(Value::as_u64)?;
            Some(format!("{field_label}需为 {width} 位数字"))
        }
        Some("out_of_range") => {
            let min = source.debug_context.get("min").and_then(Value::as_u64);
            let max = source.debug_context.get("max").and_then(Value::as_u64);
            match (min, max) {
                (Some(low), Some(high)) => {
                    Some(format!("{field_label}超出合法范围（允许 {low}–{high}）"))
                }
                (Some(low), None) => Some(format!("{field_label}超出合法范围（需不小于 {low}）")),
                (None, Some(high)) => Some(format!("{field_label}超出合法范围（需不超过 {high}）")),
                (None, None) => None,
            }
        }
        _ => None,
    }
}

fn search_error(error: &ApplicationServiceError) -> TypedError {
    let code = error.code();
    let provider_guidance: Option<String> = match error {
        ApplicationServiceError::Provider(source)
            if source.code == "BHOONIDHI_QUERY_INVALID"
                && matches!(
                    source.debug_context.get("reason").and_then(Value::as_str),
                    Some("aoi_too_complex" | "aoi_too_deep")
                ) =>
        {
            Some(
                "研究区多边形边界顶点过多（超过 Bhoonidhi 接口上限），请改用矩形或简化多边形框选"
                    .to_owned(),
            )
        }
        ApplicationServiceError::Provider(source) if source.code == "ASF_QUERY_INVALID" => {
            asf_query_invalid_guidance(source)
        }
        ApplicationServiceError::Provider(source) if source.code == "BHOONIDHI_HTTP_ERROR" => {
            source
                .debug_context
                .get("status")
                .and_then(Value::as_u64)
                .map(|status| format!("http_status:{status}"))
        }
        _ => None,
    };
    let account = code.contains("CREDENTIAL")
        || code.contains("UNAUTHORIZED")
        || code.contains("AUTH_REJECTED")
        || code.contains("AUTH_UNAVAILABLE")
        || code.contains("SESSION")
        || code.contains("ACCESS_DENIED")
        || code.contains("FORBIDDEN");
    let network = code.contains("NETWORK")
        || code.contains("TRANSPORT")
        || code.contains("TIMEOUT")
        || code.ends_with("_SERVER_ERROR")
        || code == "CATALOGUE_HTTP_SERVER_FAILURE";
    let message_zh = if let Some(guidance) = provider_guidance {
        guidance
    } else if account {
        "账号或登录会话不可用，请检查该数据源的账号状态".to_owned()
    } else if network {
        "网络或服务暂不可用，请稍后重试检索".to_owned()
    } else if code.contains("REVISION_CONFLICT") || code.ends_with("_STALE") {
        "检索状态修订冲突，请刷新后重试".to_owned()
    } else if code.contains("NOT_FOUND") {
        "检索数据未找到，请核对条件后重试".to_owned()
    } else {
        "检索失败，请查看错误码与诊断".to_owned()
    };
    TypedError {
        code: code.to_owned(),
        message_zh,
        retryable: network && !account,
        needs_account: None,
        needs_user_confirmation: false,
    }
}

/// Orbit, DEM and GACOS have no catalogue batch: they are reached through
/// their dedicated task entries. The unified entry answers with a typed,
/// terminal "unsupported" progress instead of an untyped request error.
fn auxiliary_unsupported(
    request: &UnifiedSearchRequest,
) -> Result<Option<SearchProgress>, ApplicationServiceError> {
    request.validate().map_err(invalid)?;
    if !super::valid_catalogue_opaque_id(&request.request_id) {
        return Err(invalid("request_id must be a bounded opaque identifier"));
    }
    let registry = ResourceRegistry::embedded_ref()
        .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable)?;
    let message_zh = match (
        registry.resource(&request.resource_id),
        request.resource_id.as_str(),
    ) {
        (_, "event.usgs.earthquake.v1") => {
            "USGS 地震事件目录无需在影像工作台检索，请在地震事件面板中触发同震配对计算"
        }
        (_, "geodesy.ngl.gnss.v1") => {
            "NGL 全球 GNSS 基准站位移序列无需在影像工作台检索，请在参考点面板中自动检索与匹配"
        }
        (_, "atmospheric.era5.reanalysis.v1") => {
            "ERA5 大气延迟场无需在影像工作台检索，请在气象校正面板中按影像过境时刻与范围提取"
        }
        (_, "ionospheric.igs.gim.v1") => {
            "IGS GIM 电离层格网无需在影像工作台检索，请在电离层校正面板中按影像日期与年积日提取"
        }
        (Some(ResourceDescriptor::Orbit { .. }), _) => {
            "精密轨道无需在这里检索，会按 SAR 影像时间自动匹配；请在 SAR 结果或任务中创建轨道任务"
        }
        (Some(ResourceDescriptor::Dem { .. }), _) => {
            "DEM 无需在这里检索，请按当前范围直接创建 DEM 下载任务"
        }
        (Some(ResourceDescriptor::Atmospheric { .. }), _) => {
            "GACOS 无需在这里检索，请在 GACOS 入口按影像日期提交申请"
        }
        _ => return Ok(None),
    };
    Ok(Some(SearchProgress {
        request_id: request.request_id.clone(),
        resource_id: request.resource_id.clone(),
        stage: SearchProgressStage::Failed,
        requests_done: 0,
        items_kept: 0,
        items_cap: 0,
        elapsed_ms: 0,
        message_zh: message_zh.to_owned(),
        terminal: true,
        error: Some(TypedError {
            code: "SEARCH_SOURCE_AUXILIARY_UNSUPPORTED".to_owned(),
            message_zh: message_zh.to_owned(),
            retryable: false,
            needs_account: None,
            needs_user_confirmation: false,
        }),
        possibly_incomplete: false,
        batch_id: None,
        loaded_count: None,
        remote_total: None,
    }))
}

fn recognized_request(
    request: &UnifiedSearchRequest,
) -> Result<(SourceSearchMode, u64), ApplicationServiceError> {
    request.validate().map_err(invalid)?;
    if !super::valid_catalogue_opaque_id(&request.request_id) {
        return Err(invalid("request_id must be a bounded opaque identifier"));
    }
    if insar_core::validated_authoritative_polygonal_geojson(&request.aoi).is_none() {
        return Err(invalid(
            "aoi must be an authoritative Polygon or MultiPolygon",
        ));
    }
    let registry = ResourceRegistry::embedded_ref()
        .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable)?;
    let resource = registry
        .resource(&request.resource_id)
        .ok_or_else(|| invalid("unknown resource_id"))?;
    if matches!(
        resource.common().lifecycle,
        StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
    ) {
        return Err(invalid(
            super::unified_sources::availability_reason_for_resource(
                &resource.common().resource_id,
                &resource.common().lifecycle,
            )
            .unwrap_or_else(|| "resource is not available".to_owned()),
        ));
    }
    let ResourceDescriptor::ScienceDataset { science, .. } = resource else {
        return Err(invalid(
            "auxiliary source has no catalogue batch; use the dedicated task entry",
        ));
    };
    if let Some(search) = &science.sar_search {
        // Legacy ASF SAR keeps its 250 default; a typed route may rise.
        let mut target = LEGACY_DEFAULT_TARGET;
        match search.projection_kind {
            SarSearchProjectionKind::TypedCatalogProfile => {
                target = unified_default_target(registry, resource);
                let route = registry
                    .provider_route_candidates(&request.resource_id, CapabilityKind::Search)
                    .map_err(invalid)?
                    .into_iter()
                    .find(|candidate| candidate.route.route_id == search.provider_route_id)
                    .ok_or_else(|| invalid("reviewed SAR route unavailable"))?;
                if route.route.native_collection_id.is_none() || search.logical_dataset.is_none() {
                    return Err(invalid("reviewed SAR route has incomplete source identity"));
                }
                let mut filters = request.filters.clone();
                if let Some(range) = &request.time_range {
                    filters.insert(
                        "datetime".to_owned(),
                        serde_json::to_value(range).map_err(|_| invalid("invalid time_range"))?,
                    );
                }
                let query = CatalogSearchQuery {
                    logical_dataset: search.logical_dataset.clone().unwrap_or_default(),
                    provider_id: route.route.provider_id.clone(),
                    native_collection_id: route
                        .route
                        .native_collection_id
                        .clone()
                        .unwrap_or_default(),
                    aoi_geojson: Some(request.aoi.clone()),
                    cursor: None,
                    result_limit: Some(target),
                    filters,
                };
                let allowed = search
                    .source_filters
                    .as_ref()
                    .ok_or_else(|| invalid("SAR filters unavailable"))?;
                if query.filters.keys().any(|key| {
                    !allowed.supported_keys.contains(key) && !allowed.fixed.contains_key(key)
                }) {
                    return Err(invalid("unreviewed SAR catalogue filter"));
                }
                for (key, value) in &allowed.fixed {
                    let expected = serde_json::to_value(value)
                        .map_err(|_| invalid("invalid fixed SAR filter"))?;
                    if query
                        .filters
                        .get(key)
                        .is_some_and(|actual| actual != &expected)
                    {
                        return Err(invalid("SAR fixed filter conflicts with source"));
                    }
                }
                query.validate().map_err(invalid)?;
            }
            SarSearchProjectionKind::LegacyAsfSearch => {
                let mut query: SarSearchQuery =
                    serde_json::from_value(Value::Object(request.filters.clone()))
                        .map_err(|_| invalid("invalid SAR filter shape"))?;
                query.dataset = search.selector.clone();
                query.provider = "ASF".to_owned();
                query.aoi_geojson = Some(request.aoi.clone());
                query.max_results = Some(LEGACY_DEFAULT_TARGET);
                if let Some(range) = &request.time_range {
                    query.start = range.start.clone();
                    query.end = range.end.clone();
                }
                query.validate().map_err(invalid)?;
            }
        }
        Ok((SourceSearchMode::Sar, target))
    } else if science.search_schema_id.is_some() {
        let schemas = SearchFilterSchemaCatalog::embedded_ref()
            .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable)?;
        let schema = schemas
            .for_resource(&request.resource_id)
            .ok_or_else(|| invalid("missing optical filter schema"))?;
        let mut filters = request.filters.clone();
        if let Some(range) = &request.time_range {
            filters.insert(
                "datetime".to_owned(),
                serde_json::to_value(range).map_err(|_| invalid("invalid time_range"))?,
            );
        }
        apply_default_optical_filters(schema, &mut filters);
        schema.validate_query_filters(&filters).map_err(invalid)?;
        Ok((
            SourceSearchMode::CatalogueIntent,
            unified_default_target(registry, resource),
        ))
    } else {
        Err(invalid("source has no reviewed search route"))
    }
}

fn apply_default_optical_filters(
    schema: &insar_core::SearchFilterSchema,
    filters: &mut serde_json::Map<String, Value>,
) {
    for filter in &schema.filters {
        match filter {
            insar_core::SearchFilter::Fixed {
                query_key: Some(key),
                options,
                ..
            } => {
                if let Some(expected) = options.first().map(|option| option.value.clone()) {
                    if filters.get(key).is_none_or(Value::is_null) {
                        filters.insert(key.clone(), Value::String(expected));
                    }
                }
            }
            insar_core::SearchFilter::MultiEnum {
                query_key,
                default,
                empty_selection: insar_core::EmptySelection::RestoreAll,
                ..
            } => {
                let needs_default = match filters.get(query_key) {
                    None | Some(Value::Null) => true,
                    Some(Value::Array(items)) => items.is_empty(),
                    _ => false,
                };
                if needs_default && !default.is_empty() {
                    filters.insert(
                        query_key.clone(),
                        Value::Array(default.iter().cloned().map(Value::String).collect()),
                    );
                }
            }
            insar_core::SearchFilter::MultiEnum {
                query_key,
                empty_selection: insar_core::EmptySelection::Omit,
                ..
            } => {
                let should_omit = match filters.get(query_key) {
                    Some(Value::Null) => true,
                    Some(Value::Array(items)) => items.is_empty(),
                    _ => false,
                };
                if should_omit {
                    filters.remove(query_key);
                }
            }
            insar_core::SearchFilter::Number { query_key, .. } => {
                if matches!(filters.get(query_key), Some(Value::Null)) {
                    filters.remove(query_key);
                }
            }
            insar_core::SearchFilter::Fixed {
                query_key: None, ..
            }
            | insar_core::SearchFilter::DateRange { .. } => {}
        }
    }
}

/// The pre-3(c) default target of a legacy ASF SAR search and of a typed SAR
/// route that does not stream.
const LEGACY_DEFAULT_TARGET: u64 = 250;
/// The default target of a non-streaming optical catalogue route. Owner
/// decision 2026-11-13: optical and other catalogue sources default to the
/// same 250 scenes as SAR.
const CATALOGUE_DEFAULT_TARGET: u64 = 250;

/// #22 B slice 3(c): whether a source that streams on every eligible route
/// defaults to the 10,000 ceiling. Held off pending a decision: the existing
/// unified-search and Agent suites pin the HLS default at 100
/// (`tests/unified_search.rs` items_cap and default-target-met cases,
/// `tests/agent_tools.rs` fixture), and this slice may not edit those
/// assertions. Turning this on is the whole switch; the eligibility below is
/// already covered by unit tests.
const STREAMING_SOURCES_DEFAULT_TO_CEILING: bool = false;

/// #22 B slice 3(c): the default target of a unified search. A source whose
/// every eligible search route streams page by page into staging (progressive
/// delivery: page 1 first, the rest in the background) may default to the
/// 10,000 safety ceiling ([`STREAMING_SOURCES_DEFAULT_TO_CEILING`]); any route
/// that still fetches a whole response keeps the bounded default (250 SAR
/// and optical), so no fallback ever loads 10,000 products into memory at
/// once. Legacy ASF SAR always keeps 250. The source descriptor's
/// `result_cap` reports the same value.
pub(crate) fn unified_default_target(
    registry: &ResourceRegistry,
    resource: &ResourceDescriptor,
) -> u64 {
    match streaming_default_eligibility(registry, resource) {
        Some((true, _)) if STREAMING_SOURCES_DEFAULT_TO_CEILING => insar_core::SEARCH_SAFETY_CAP,
        Some((_, fallback_target)) => fallback_target,
        None => 0,
    }
}

/// Whether every eligible search route of `resource` streams, with the
/// pre-3(c) default target; `None` for a resource that is not searchable
/// science data.
fn streaming_default_eligibility(
    registry: &ResourceRegistry,
    resource: &ResourceDescriptor,
) -> Option<(bool, u64)> {
    let ResourceDescriptor::ScienceDataset { science, .. } = resource else {
        return None;
    };
    let resource_id = &resource.common().resource_id;
    let (logical_dataset, fallback_target, only_route) = match &science.sar_search {
        Some(search) => match search.projection_kind {
            SarSearchProjectionKind::LegacyAsfSearch => {
                return Some((false, LEGACY_DEFAULT_TARGET))
            }
            SarSearchProjectionKind::TypedCatalogProfile => (
                search.logical_dataset.as_deref(),
                LEGACY_DEFAULT_TARGET,
                Some(search.provider_route_id.as_str()),
            ),
        },
        None => (
            science.catalogue_logical_dataset.as_deref(),
            CATALOGUE_DEFAULT_TARGET,
            None,
        ),
    };
    let Some(logical_dataset) = logical_dataset else {
        return Some((false, fallback_target));
    };
    let Ok(candidates) = registry.provider_route_candidates(resource_id, CapabilityKind::Search)
    else {
        return Some((false, fallback_target));
    };
    let routes: Vec<_> = candidates
        .iter()
        .filter(|candidate| only_route.is_none_or(|route_id| candidate.route.route_id == route_id))
        .collect();
    let streams = !routes.is_empty()
        && routes.iter().all(|candidate| {
            candidate
                .route
                .native_collection_id
                .as_deref()
                .is_some_and(|collection| {
                    insar_service::catalogue_route_streams(
                        logical_dataset,
                        &candidate.route.provider_id,
                        collection,
                    )
                })
        });
    Some((streams, fallback_target))
}

/// True when an optical/catalogue source's priority-1 reviewed Search route
/// supports page-by-page streaming, enabling 250-first progressive display
/// followed by background continuation up to [`OPTICAL_BACKGROUND_CONTINUATION_CAP`].
fn optical_primary_route_streams(registry: &ResourceRegistry, resource_id: &str) -> bool {
    let Some(ResourceDescriptor::ScienceDataset { science, .. }) = registry.resource(resource_id)
    else {
        return false;
    };
    if science.sar_search.is_some() {
        return false;
    }
    let Some(logical_dataset) = science.catalogue_logical_dataset.as_deref() else {
        return false;
    };
    let Ok(candidates) = registry.provider_route_candidates(resource_id, CapabilityKind::Search)
    else {
        return false;
    };
    candidates.first().is_some_and(|candidate| {
        candidate
            .route
            .native_collection_id
            .as_deref()
            .is_some_and(|collection| {
                insar_service::catalogue_route_streams(
                    logical_dataset,
                    &candidate.route.provider_id,
                    collection,
                )
            })
    })
}

/// True only when `sortby` newest-first was actually sent and decided the cap:
/// a dated request, a reviewed sort-capable source, one spatial part (no
/// tiling merged by product ID), the cap filled, and the provider still had a
/// continuation. A cache hit has no provider outcome and makes no claim.
fn newest_first_cut(
    dated: bool,
    provider_id: Option<&str>,
    outcome: Option<CatalogueOutcome>,
    count: u64,
    cap: u64,
) -> bool {
    dated
        && count > 0
        && count >= cap
        && provider_id.is_some_and(insar_provider::catalog::stac_sorts_newest_first)
        && outcome
            .is_some_and(|outcome| outcome.transport_part_count == 1 && !outcome.remote_exhausted)
}

/// Whether the batch just frozen is possibly incomplete. A cache hit keeps
/// the flag it was frozen with; a batch without statistics claims nothing.
fn frozen_possibly_incomplete(workbench: Option<&insar_store::StoreWorkbenchSummary>) -> bool {
    workbench
        .and_then(|workbench| workbench.search_stats.as_ref())
        .is_some_and(|stats| stats.truncated)
}

/// #22 B slice 3(c): the provider reports more matches than the 10,000
/// safety ceiling keeps, so the user is asked to narrow the area or time.
fn over_ceiling_message(remote_total: Option<u64>, count: u64) -> Option<String> {
    remote_total
        .filter(|total| *total > insar_core::SEARCH_SAFETY_CAP)
        .map(|total| {
            format!("共 {total} 景，超过 10,000 景上限，已纳入 {count} 景，请缩小范围或时间")
        })
}

/// Terminal text for a frozen search; it never estimates records beyond N.
fn completed_message(count: u64, newest_first_cut: bool) -> String {
    if newest_first_cut {
        format!("已完成检索，按时间最新优先纳入前 {count} 景")
    } else {
        format!("已完成检索，纳入 {count} 条")
    }
}

fn map_progress(resource_id: &str, update: &ApplicationSearchProgress, cap: u64) -> SearchProgress {
    let stage =
        match update.stage {
            ApplicationSearchProgressStage::Validating
            | ApplicationSearchProgressStage::Planning => SearchProgressStage::Planning,
            ApplicationSearchProgressStage::WaitingProvider
            | ApplicationSearchProgressStage::Fetching => SearchProgressStage::Requesting,
            ApplicationSearchProgressStage::Filtering => SearchProgressStage::Filtering,
            ApplicationSearchProgressStage::Freezing => SearchProgressStage::Freezing,
            ApplicationSearchProgressStage::Completed => SearchProgressStage::Done,
            ApplicationSearchProgressStage::Failed => SearchProgressStage::Failed,
            ApplicationSearchProgressStage::Cancelling
            | ApplicationSearchProgressStage::Cancelled => SearchProgressStage::Cancelled,
        };
    SearchProgress {
        request_id: update.request_id.clone(),
        resource_id: resource_id.to_owned(),
        stage,
        requests_done: update.provider_request_count.unwrap_or(0),
        items_kept: update.admitted_count.unwrap_or(0),
        items_cap: update.requested_limit.unwrap_or(cap),
        elapsed_ms: update.elapsed_ms,
        message_zh: update.message.clone(),
        terminal: update.terminal,
        error: None,
        possibly_incomplete: false,
        batch_id: update.batch_id.clone(),
        loaded_count: update.loaded_count,
        remote_total: update.remote_total,
    }
}

impl ApplicationService {
    pub fn search_start(
        self: &Arc<Self>,
        request: UnifiedSearchRequest,
    ) -> Result<SearchProgress, ApplicationServiceError> {
        self.search_start_with_options(request, false)
    }

    pub fn search_start_with_options(
        self: &Arc<Self>,
        request: UnifiedSearchRequest,
        bypass_cache: bool,
    ) -> Result<SearchProgress, ApplicationServiceError> {
        if let Some(unsupported) = auxiliary_unsupported(&request)? {
            let mut progress = self
                .unified_searches
                .progress
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            if progress.contains_key(&request.request_id) {
                return Err(invalid("request_id already exists"));
            }
            progress.insert_at(request.request_id, unsupported.clone(), Instant::now());
            return Ok(unsupported);
        }
        let (mode, cap) = recognized_request(&request)?;
        let is_sar_mode = matches!(mode, SourceSearchMode::Sar);
        let is_optical_streaming = matches!(mode, SourceSearchMode::CatalogueIntent)
            && ResourceRegistry::embedded_ref()
                .ok()
                .is_some_and(|reg| optical_primary_route_streams(reg, &request.resource_id));
        let is_progressive_mode = is_sar_mode || is_optical_streaming;
        let effective_cap = if is_sar_mode {
            cap.max(SAR_BACKGROUND_CONTINUATION_CAP)
        } else if is_optical_streaming {
            cap.max(OPTICAL_BACKGROUND_CONTINUATION_CAP)
        } else {
            cap
        };
        let progressive_first_page_size = if is_sar_mode {
            SAR_FIRST_PAGE_SIZE
        } else {
            OPTICAL_FIRST_PAGE_SIZE
        };
        let registry = Arc::clone(&self.unified_searches);
        let pending = SearchProgress {
            request_id: request.request_id.clone(),
            resource_id: request.resource_id.clone(),
            stage: SearchProgressStage::Planning,
            requests_done: 0,
            items_kept: 0,
            items_cap: effective_cap,
            elapsed_ms: 0,
            message_zh: "正在准备检索".to_owned(),
            terminal: false,
            error: None,
            possibly_incomplete: false,
            batch_id: None,
            loaded_count: None,
            remote_total: None,
        };
        {
            let mut progress = registry
                .progress
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            if progress.contains_key(&request.request_id) {
                return Err(invalid("request_id already exists"));
            }
            progress.prune(Instant::now());
            progress.insert_at(request.request_id.clone(), pending.clone(), Instant::now());
        }
        registry.prune_stale_cancellations();
        let resource_id_for_fallback = request.resource_id.clone();
        let request_id_for_fallback = request.request_id.clone();
        self.update_source_view(|store, at| {
            store.mark_source_view_pending(&request.resource_id, &request.request_id, at)
        });
        let service = Arc::clone(self);
        let worker_registry = Arc::clone(&registry);
        let pending_for_worker = pending.clone();
        let spawn_result = std::thread::Builder::new()
            .name("insar-unified-search".to_owned())
            .spawn(move || {
                let pending = pending_for_worker;
                let resource_id = request.resource_id.clone();
                let batch_id = format!("batch-unified-{}", Uuid::new_v4());
                let published_batch_id = batch_id.clone();
                let progressive_sar_published = Arc::new(Mutex::new(false));
                let last_synced_sar_loaded = Arc::new(Mutex::new(0_u64));
                let request_id = request.request_id.clone();
                let mut worker_guard = SearchWorkerGuard {
                    service: Arc::clone(&service),
                    registry: Arc::clone(&worker_registry),
                    request_id: request_id.clone(),
                    resource_id: resource_id.clone(),
                    handed_off: false,
                    completed: false,
                };
                let registry = worker_registry;
                let publish = |update: ApplicationSearchProgress| {
                    // The worker publishes the one terminal itself, after the
                    // frozen batch is read back (#22 B slice 3(a)); an earlier
                    // terminal here would let a poller stop on a Done without it,
                    // or (#22 B slice 3(c)) on a Failed/Cancelled without its
                    // typed error and kept batch.
                    if update.terminal {
                        return;
                    }
                    let next = map_progress(&resource_id, &update, effective_cap);
                    if is_progressive_mode {
                        if let Ok(Some(bp)) = service.search_batch_progress(&published_batch_id) {
                            if bp.loaded_count >= progressive_first_page_size {
                                let first_publish = progressive_sar_published
                                    .lock()
                                    .map(|mut flag| {
                                        let prev = *flag;
                                        *flag = true;
                                        !prev
                                    })
                                    .unwrap_or(false);
                                if first_publish {
                                    registry.record_published(&request_id, &published_batch_id);
                                }
                                let should_emit_sync = last_synced_sar_loaded
                                    .lock()
                                    .map(|mut prev| {
                                        if bp.loaded_count > *prev {
                                            *prev = bp.loaded_count;
                                            true
                                        } else {
                                            false
                                        }
                                    })
                                    .unwrap_or(false);
                                if let Ok(mut progress) = registry.progress.lock() {
                                    if !progress.get(&request_id).is_some_and(|c| {
                                        c.terminal && c.stage == SearchProgressStage::Cancelled
                                    }) {
                                        progress.insert_at(
                                            request_id.clone(),
                                            SearchProgress {
                                                stage: SearchProgressStage::Requesting,
                                                items_kept: bp.loaded_count,
                                                items_cap: effective_cap,
                                                terminal: false,
                                                possibly_incomplete: false,
                                                batch_id: Some(published_batch_id.clone()),
                                                loaded_count: Some(bp.loaded_count),
                                                remote_total: bp.remote_total.or(next.remote_total),
                                                message_zh: format!(
                                                    "已优先载入前 {} 景，正在后台继续拉取...",
                                                    bp.loaded_count
                                                ),
                                                ..next.clone()
                                            },
                                            Instant::now(),
                                        );
                                    }
                                }
                                if should_emit_sync {
                                    service.emit_search_batch_sync(json!({
                                        "action": "search_updated",
                                        "request_id": request_id,
                                        "batch_id": published_batch_id,
                                        "resource_id": resource_id,
                                        "loaded_count": bp.loaded_count,
                                        "total_count": bp.remote_total,
                                        "complete": false,
                                    }));
                                }
                                return;
                            }
                        }
                    }
                    if let Ok(mut progress) = registry.progress.lock() {
                        // A cancellation should not be overwritten by a delayed planning update.
                        if !progress
                            .get(&request_id)
                            .is_some_and(|current| current.terminal)
                        {
                            progress.insert_at(request_id.clone(), next, Instant::now());
                        }
                    }
                };
                let created_at =
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                let mut sar_continuation_plan: Option<SarContinuationPlan> = None;
                let result = match mode {
                    SourceSearchMode::CatalogueIntent => {
                        let schemas = SearchFilterSchemaCatalog::embedded_ref()
                            .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable);
                        schemas.and_then(|schemas| {
                            let schema = schemas
                                .for_resource(&resource_id)
                                .ok_or_else(|| invalid("missing optical filter schema"))?;
                            let mut filters = request.filters.clone();
                            if let Some(range) = &request.time_range {
                                filters.insert(
                                    "datetime".to_owned(),
                                    serde_json::to_value(range)
                                        .map_err(|_| invalid("invalid time_range"))?,
                                );
                            }
                            apply_default_optical_filters(schema, &mut filters);
                            service.catalog_intent_search_and_freeze_with_progress_and_options(
                                &ApplicationCatalogIntentSearchAndFreezeRequest {
                                    request_id: request_id.clone(),
                                    intent: CatalogSearchIntent {
                                        resource_id: resource_id.clone(),
                                        search_schema_id: schema.schema_id.clone(),
                                        schema_revision: schema.schema_revision,
                                        aoi_geojson: Some(request.aoi.clone()),
                                        result_limit: Some(effective_cap),
                                        filters,
                                    },
                                    batch_id,
                                    created_at,
                                    aoi_name: request.aoi_name.clone(),
                                    aoi_geojson: Some(request.aoi.clone()),
                                    workbench_page_size: 100,
                                },
                                bypass_cache,
                                publish,
                            )
                        })
                    }
                    SourceSearchMode::Sar => {
                        let registry = ResourceRegistry::embedded_ref()
                            .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable);
                        registry.and_then(|registry| {
                            let ResourceDescriptor::ScienceDataset { science, .. } = registry
                                .resource(&resource_id)
                                .ok_or_else(|| invalid("unknown SAR resource"))?
                            else {
                                return Err(invalid("not a SAR source"));
                            };
                            let selector = science
                                .sar_search
                                .as_ref()
                                .ok_or_else(|| invalid("SAR selector unavailable"))?;
                            if matches!(
                                selector.projection_kind,
                                SarSearchProjectionKind::TypedCatalogProfile
                            ) {
                                let routes = registry
                                    .provider_route_candidates(&resource_id, CapabilityKind::Search)
                                    .map_err(invalid)?;
                                let route = routes
                                    .into_iter()
                                    .find(|candidate| {
                                        candidate.route.route_id == selector.provider_route_id
                                    })
                                    .ok_or_else(|| invalid("SAR catalogue route unavailable"))?
                                    .route;
                                let logical_dataset = selector
                                    .logical_dataset
                                    .clone()
                                    .ok_or_else(|| invalid("SAR logical dataset unavailable"))?;
                                let native_collection_id = route
                                    .native_collection_id
                                    .clone()
                                    .ok_or_else(|| invalid("SAR collection unavailable"))?;
                                let mut filters = request.filters.clone();
                                if let Some(range) = &request.time_range {
                                    filters.insert(
                                        "datetime".to_owned(),
                                        serde_json::to_value(range)
                                            .map_err(|_| invalid("invalid time_range"))?,
                                    );
                                }
                                service.catalog_search_and_freeze_with_progress_and_options(
                                    &ApplicationCatalogSearchAndFreezeRequest {
                                        request_id: request_id.clone(),
                                        search: CatalogSearchQuery {
                                            logical_dataset,
                                            provider_id: route.provider_id.clone(),
                                            native_collection_id,
                                            aoi_geojson: Some(request.aoi.clone()),
                                            cursor: None,
                                            result_limit: Some(effective_cap),
                                            filters,
                                        },
                                        batch_id,
                                        created_at,
                                        aoi_name: request.aoi_name.clone(),
                                        aoi_geojson: Some(request.aoi.clone()),
                                        workbench_page_size: 100,
                                    },
                                    bypass_cache,
                                    publish,
                                )
                            } else {
                                // Provider and dataset are bound by the Registry, not UI filters.
                                let mut query: SarSearchQuery =
                                    serde_json::from_value(Value::Object(request.filters.clone()))
                                        .map_err(|_| invalid("invalid SAR filter shape"))?;
                                query.dataset = selector.selector.clone();
                                query.provider = "ASF".to_owned();
                                query.aoi_geojson = Some(request.aoi.clone());
                                query.max_results = Some(SAR_FIRST_PAGE_SIZE);
                                if let Some(range) = &request.time_range {
                                    query.start = range.start.clone();
                                    query.end = range.end.clone();
                                }
                                let app_req = ApplicationSarSearchAndFreezeRequest {
                                    request_id: request_id.clone(),
                                    search: SarRemoteSearchRequest { query },
                                    batch_id,
                                    created_at,
                                    aoi_name: request.aoi_name.clone(),
                                    aoi_geojson: Some(request.aoi.clone()),
                                    page_size: 100,
                                };
                                let started = Instant::now();
                                match service
                                    .sar_search_first_page_or_stage_for_continuation_with_options(
                                        &app_req,
                                        SAR_BACKGROUND_CONTINUATION_CAP,
                                        bypass_cache,
                                        &|mut update| {
                                            update.elapsed_ms =
                                                u64::try_from(started.elapsed().as_millis())
                                                    .unwrap_or(u64::MAX);
                                            publish(update);
                                        },
                                    )? {
                                    SarFirstPageOutcome::Sealed(snapshot) => Ok(snapshot),
                                    SarFirstPageOutcome::StagedForContinuation(plan) => {
                                        sar_continuation_plan = Some(plan);
                                        service.core_restore(false)
                                    }
                                }
                            }
                        })
                    }
                    SourceSearchMode::Auxiliary => {
                        Err(invalid("auxiliary source is not searchable"))
                    }
                };
                if let Some(plan) = sar_continuation_plan {
                    registry.mark_continuation_active(&request_id);
                    registry.record_published(&request_id, &published_batch_id);
                    let current = registry
                        .progress
                        .lock()
                        .ok()
                        .and_then(|progress| progress.get(&request_id).cloned())
                        .unwrap_or(pending.clone());
                    let first_loaded = plan.loaded_count;
                    let remote_total = plan.remote_total;
                    if let Ok(mut progress) = registry.progress.lock() {
                        progress.insert_at(
                            request_id.clone(),
                            SearchProgress {
                                stage: SearchProgressStage::Requesting,
                                items_kept: first_loaded,
                                items_cap: effective_cap,
                                terminal: false,
                                possibly_incomplete: false,
                                batch_id: Some(published_batch_id.clone()),
                                loaded_count: Some(first_loaded),
                                remote_total,
                                message_zh: format!(
                                    "已优先载入前 {first_loaded} 景，正在后台继续拉取更多结果..."
                                ),
                                ..current
                            },
                            Instant::now(),
                        );
                    }
                    service.emit_search_batch_sync(json!({
                        "action": "search_updated",
                        "request_id": request_id,
                        "batch_id": published_batch_id,
                        "resource_id": resource_id,
                        "loaded_count": first_loaded,
                        "total_count": remote_total,
                        "complete": false,
                    }));
                    let bg_service = Arc::clone(&service);
                    let bg_resource = resource_id.clone();
                    let first_stats = plan.stats.clone();
                    let fallback_created_at = plan.created_at.clone();
                    let fallback_cap = plan.continuation_cap;
                    let fallback_batch_id = plan.batch_id.clone();
                    let cont_spawn = std::thread::Builder::new()
                        .name("insar-sar-continuation".to_owned())
                        .spawn(move || {
                            run_sar_background_continuation(bg_service, bg_resource, plan);
                        });
                    if cont_spawn.is_ok() {
                        worker_guard.handed_off = true;
                    } else {
                        let mut truncated_stats = first_stats;
                        truncated_stats.frozen_candidate_count = first_loaded;
                        truncated_stats.fetched_record_count = Some(first_loaded);
                        truncated_stats.mapped_record_count = Some(first_loaded);
                        truncated_stats.provider_returned_count = first_loaded;
                        truncated_stats.matched_count = remote_total;
                        truncated_stats.truncated = true;
                        finalize_sar_continuation(
                            &service,
                            &resource_id,
                            &request_id,
                            &fallback_batch_id,
                            &fallback_created_at,
                            fallback_cap,
                            first_loaded,
                            remote_total,
                            false,
                            Some(ApplicationServiceError::BusinessRuntimeUnavailable),
                            truncated_stats,
                            None,
                        );
                        worker_guard.completed = true;
                    }
                    return;
                }
                let prior_cancellation = registry.progress.lock().ok().and_then(|progress| {
                    progress
                        .get(&request_id)
                        .filter(|current| {
                            current.terminal && current.stage == SearchProgressStage::Cancelled
                        })
                        .cloned()
                });
                // #22 B slice 3(c): a search cancelled or failed after its first
                // full page keeps the arrived rows as a sealed batch; the view
                // moves to it like a frozen search, and the terminal names it.
                let kept = match &result {
                    Ok(_) => None,
                    Err(_) => service
                        .search_batch_progress(&published_batch_id)
                        .ok()
                        .flatten()
                        .filter(|progress| {
                            progress.request_id == request_id
                                && progress.state != insar_core::SearchBatchProgressState::Loading
                        }),
                };
                // The view moves only after the batch is frozen; a failure keeps
                // the previous batch and persists only the typed error.
                if result.is_ok() || kept.is_some() {
                    registry.record_published(&request_id, &published_batch_id);
                }
                match &result {
                    Err(_) if kept.is_some() => service.update_source_view(|store, at| {
                        store.record_source_view_success(
                            &resource_id,
                            &published_batch_id,
                            Some(&request_id),
                            at,
                        )
                    }),
                    Ok(_) => service.update_source_view(|store, at| {
                        store.record_source_view_success(
                            &resource_id,
                            &published_batch_id,
                            Some(&request_id),
                            at,
                        )
                    }),
                    Err(error) if super::sar_search_error_is_cancelled(error) => service
                        .update_source_view(|store, at| {
                            store.clear_source_view_pending(&resource_id, &request_id, at)
                        }),
                    Err(error) => service.update_source_view(|store, at| {
                        store.record_source_view_failure(
                            &resource_id,
                            &request_id,
                            &search_error(error),
                            at,
                        )
                    }),
                }
                let outcome = registry.take_catalogue_outcome(&request_id);
                let current = registry
                    .progress
                    .lock()
                    .ok()
                    .and_then(|progress| progress.get(&request_id).cloned())
                    .unwrap_or(pending.clone());
                let terminal = match result {
                    Ok(snapshot) => {
                        let workbench = snapshot.core.workbench.as_ref();
                        let count =
                            workbench.map_or(0, |workbench| workbench.candidate_count as u64);
                        let cut = newest_first_cut(
                            request.time_range.is_some(),
                            workbench.map(|workbench| workbench.provider_id.as_str()),
                            outcome,
                            count,
                            effective_cap,
                        );
                        let remote_total = workbench
                            .and_then(|workbench| workbench.search_stats.as_ref())
                            .and_then(|stats| stats.matched_count)
                            .or(current.remote_total);
                        SearchProgress {
                            stage: SearchProgressStage::Done,
                            items_kept: count,
                            terminal: true,
                            message_zh: over_ceiling_message(remote_total, count)
                                .unwrap_or_else(|| completed_message(count, cut)),
                            // #22 B slice 3(a): the frozen batch's own flag, so
                            // the done view and a later restore agree.
                            possibly_incomplete: frozen_possibly_incomplete(workbench),
                            // #22 B slice 3(c): the frozen batch and its count.
                            batch_id: Some(published_batch_id.clone()),
                            loaded_count: Some(count),
                            remote_total,
                            ..current
                        }
                    }
                    Err(error) => {
                        let cancelled = super::sar_search_error_is_cancelled(&error);
                        let message_zh = match (&kept, cancelled) {
                            (Some(_), true) => "检索已取消，已到结果已保留，可能不完整".to_owned(),
                            (Some(_), false) => {
                                "检索中途失败，已到结果已保留，可能不完整".to_owned()
                            }
                            (None, true) => "检索已取消".to_owned(),
                            (None, false) => search_error(&error).message_zh,
                        };
                        SearchProgress {
                            stage: if cancelled {
                                SearchProgressStage::Cancelled
                            } else {
                                SearchProgressStage::Failed
                            },
                            terminal: true,
                            message_zh,
                            error: (!cancelled).then(|| search_error(&error)),
                            // #22 B slice 3(c): rows kept by a cancel or a mid-way
                            // failure are possibly incomplete; without a kept
                            // batch nothing is readable and no batch is named.
                            possibly_incomplete: kept.is_some(),
                            items_kept: kept
                                .as_ref()
                                .map_or(current.items_kept, |kept| kept.loaded_count),
                            batch_id: kept.as_ref().map(|kept| kept.batch_id.clone()),
                            loaded_count: kept.as_ref().map(|kept| kept.loaded_count),
                            remote_total: kept
                                .as_ref()
                                .and_then(|kept| kept.remote_total)
                                .or(current.remote_total),
                            ..current
                        }
                    }
                };
                if let Ok(mut progress) = registry.progress.lock() {
                    progress.insert_at(
                        request_id.clone(),
                        prior_cancellation.unwrap_or_else(|| terminal.clone()),
                        Instant::now(),
                    );
                }
                if is_progressive_mode && terminal.stage == SearchProgressStage::Done {
                    service.emit_search_batch_sync(json!({
                        "action": "search_updated",
                        "request_id": request_id,
                        "batch_id": published_batch_id,
                        "resource_id": resource_id,
                        "loaded_count": terminal.loaded_count.unwrap_or(terminal.items_kept),
                        "total_count": terminal.remote_total,
                        "complete": true,
                    }));
                }
                worker_guard.completed = true;
            });
        if spawn_result.is_err() {
            let err = ApplicationServiceError::BusinessRuntimeUnavailable;
            let typed = search_error(&err);
            self.update_source_view(|store, at| {
                store.clear_source_view_pending(
                    &resource_id_for_fallback,
                    &request_id_for_fallback,
                    at,
                )
            });
            if let Ok(mut progress) = registry.progress.lock() {
                if let Some(existing) = progress.get(&request_id_for_fallback).cloned() {
                    progress.insert_at(
                        request_id_for_fallback.clone(),
                        SearchProgress {
                            stage: SearchProgressStage::Failed,
                            terminal: true,
                            message_zh: typed.message_zh.clone(),
                            error: Some(typed),
                            possibly_incomplete: false,
                            ..existing
                        },
                        Instant::now(),
                    );
                }
            }
            registry.clear_cancelled(&request_id_for_fallback);
            return Err(err);
        }
        Ok(pending)
    }

    pub fn search_progress(&self, request_id: &str) -> Option<SearchProgress> {
        self.unified_searches
            .progress
            .lock()
            .ok()
            .and_then(|mut progress| {
                progress.prune(Instant::now());
                progress.get(request_id).cloned()
            })
    }

    /// The batch a search started through `search_start` froze; `None`
    /// before it finishes, after a failure, and after a restart.
    pub fn search_batch_id(&self, request_id: &str) -> Option<String> {
        self.unified_searches.published_batch(request_id)
    }

    pub fn search_cancel(&self, request_id: &str) -> Result<bool, ApplicationServiceError> {
        self.unified_searches.mark_cancelled(request_id);
        self.cancel_request(request_id)
    }
}

#[cfg(test)]
mod default_target_tests {
    use super::{streaming_default_eligibility, unified_default_target};
    use insar_core::ResourceRegistry;

    fn eligibility(resource_id: &str) -> Option<(bool, u64)> {
        let registry = ResourceRegistry::embedded_ref().unwrap();
        streaming_default_eligibility(registry, registry.resource(resource_id).unwrap())
    }

    /// #22 B slice 3(c): only a source whose every route streams page by page
    /// is eligible for the 10,000 default; the others keep 250.
    #[test]
    fn only_sources_that_stream_on_every_route_are_eligible_for_the_ceiling() {
        assert_eq!(eligibility("science.hls.l30.v2"), Some((true, 250)));
        assert_eq!(eligibility("science.hls.s30.v2"), Some((true, 250)));
        // Sentinel-2 L2A streams across CDSE OData, MPC STAC, and Earth Search.
        assert_eq!(eligibility("science.sentinel-2.l2a.v1"), Some((true, 250)));
        // Landsat still has whole-response USGS M2M fallback routes in the Registry.
        assert_eq!(eligibility("science.landsat.c2-l2.v1"), Some((false, 250)));
        assert_eq!(
            eligibility("science.nisar.l.rslc.provisional.v1"),
            Some((false, 250))
        );
        // Legacy ASF SAR always keeps 250.
        assert_eq!(
            eligibility("science.sentinel-1.slc-burst.v1"),
            Some((false, 250))
        );
        assert_eq!(eligibility("dem.cop30.v1"), None);
    }

    #[test]
    fn the_default_target_never_lowers_a_pre_3c_default() {
        let registry = ResourceRegistry::embedded_ref().unwrap();
        for resource in &registry.resources {
            if let Some((_, fallback)) = streaming_default_eligibility(registry, resource) {
                assert!(unified_default_target(registry, resource) >= fallback);
            }
        }
    }
}

#[cfg(test)]
mod error_tests {
    use super::{
        completed_message, newest_first_cut, search_error, CatalogueOutcome, SearchProgress,
        SearchProgressStage,
    };
    use crate::ApplicationServiceError;
    use insar_store::StoreError;

    #[test]
    fn newest_first_suffix_needs_a_sent_sort_one_part_and_a_remaining_page() {
        let mpc = Some("microsoft.planetary-computer");
        let cut = Some(CatalogueOutcome {
            remote_exhausted: false,
            transport_part_count: 1,
        });
        assert!(newest_first_cut(true, mpc, cut, 100, 100));
        assert!(newest_first_cut(
            true,
            Some("element84.earth-search"),
            cut,
            250,
            250
        ));
        assert_eq!(
            completed_message(100, true),
            "已完成检索，按时间最新优先纳入前 100 景"
        );
        assert_eq!(completed_message(100, false), "已完成检索，纳入 100 条");

        // Undated: no sortby was sent.
        assert!(!newest_first_cut(false, mpc, cut, 100, 100));
        // Spatial fallback tiling: parts merged by product ID.
        let tiled = Some(CatalogueOutcome {
            remote_exhausted: false,
            transport_part_count: 4,
        });
        assert!(!newest_first_cut(true, mpc, tiled, 100, 100));
        // Remote exhausted: nothing was cut.
        let exhausted = Some(CatalogueOutcome {
            remote_exhausted: true,
            transport_part_count: 1,
        });
        assert!(!newest_first_cut(true, mpc, exhausted, 100, 100));
        // Below the cap, a source without the sort extension, or a cache hit.
        assert!(!newest_first_cut(true, mpc, cut, 99, 100));
        assert!(!newest_first_cut(
            true,
            Some("asf.sentinel1"),
            cut,
            250,
            250
        ));
        assert!(!newest_first_cut(true, mpc, None, 100, 100));
    }

    /// W3f: the Agent host's fallback lookup of the batch a search froze.
    #[test]
    fn published_batches_are_found_by_request_and_bounded() {
        let registry = super::UnifiedSearchRegistry::default();
        assert_eq!(registry.published_batch("r-0"), None);
        registry.record_published("r-0", "batch-0");
        registry.record_published("r-0", "batch-0b");
        assert_eq!(registry.published_batch("r-0").as_deref(), Some("batch-0b"));
        for n in 1..=super::TERMINAL_CAP {
            registry.record_published(&format!("r-{n}"), &format!("batch-{n}"));
        }
        assert_eq!(
            registry.published_batch("r-0"),
            None,
            "the oldest is dropped"
        );
        assert_eq!(registry.published_batch("r-1").as_deref(), Some("batch-1"));
        assert_eq!(
            registry.published.lock().unwrap().len(),
            super::TERMINAL_CAP
        );
    }

    #[test]
    fn catalogue_outcomes_are_kept_only_for_unified_requests() {
        let registry = super::UnifiedSearchRegistry::default();
        let stats: insar_core::CatalogSearchStats = serde_json::from_value(serde_json::json!({
            "requested_limit": 100, "matched_count": null, "request_count": 1,
            "transport_part_count": 1, "fetched_record_count": 100, "mapped_record_count": 100,
            "profile_filtered_record_count": 0, "exact_duplicate_record_count": 0,
            "unique_provider_product_count": 100, "exact_matched_count": 100,
            "returned_count": 100, "remote_exhausted": false, "working_set_satisfied": true,
            "elapsed_ms": 1, "truncated": false
        }))
        .unwrap();
        registry.record_catalogue_outcome("legacy-request", &stats);
        assert_eq!(registry.take_catalogue_outcome("legacy-request"), None);
        registry.progress.lock().unwrap().insert_at(
            "unified-request".to_owned(),
            SearchProgress {
                request_id: "unified-request".to_owned(),
                resource_id: "science.hls.l30.v2".to_owned(),
                stage: SearchProgressStage::Requesting,
                requests_done: 0,
                items_kept: 0,
                items_cap: 100,
                elapsed_ms: 0,
                message_zh: "fixture".to_owned(),
                terminal: false,
                error: None,
                possibly_incomplete: false,
                batch_id: None,
                loaded_count: None,
                remote_total: None,
            },
            std::time::Instant::now(),
        );
        registry.record_catalogue_outcome("unified-request", &stats);
        assert_eq!(
            registry.take_catalogue_outcome("unified-request"),
            Some(CatalogueOutcome {
                remote_exhausted: false,
                transport_part_count: 1,
            })
        );
        assert_eq!(registry.take_catalogue_outcome("unified-request"), None);
    }

    #[test]
    fn typed_store_errors_keep_revision_and_not_found_distinct() {
        for (store, word) in [
            (StoreError::RevisionConflict, "修订"),
            (StoreError::NotFound, "未找到"),
        ] {
            let typed = search_error(&ApplicationServiceError::from(store));
            assert!(typed.message_zh.contains(word));
            assert!(!typed.retryable);
        }
    }

    #[test]
    fn terminal_cache_evicts_oldest_without_dropping_running_requests() {
        let service =
            crate::ApplicationService::new(crate::ApplicationServiceConfig::liveness_only());
        let registry = &service.unified_searches;
        let done = |id: &str, terminal: bool| SearchProgress {
            request_id: id.to_owned(),
            resource_id: "science.hls.l30.v2".to_owned(),
            stage: if terminal {
                SearchProgressStage::Done
            } else {
                SearchProgressStage::Requesting
            },
            requests_done: 0,
            items_kept: 0,
            items_cap: 100,
            elapsed_ms: 0,
            message_zh: "fixture".to_owned(),
            terminal,
            error: None,
            possibly_incomplete: false,
            batch_id: None,
            loaded_count: None,
            remote_total: None,
        };
        {
            let mut progress = registry.progress.lock().unwrap();
            let now = std::time::Instant::now();
            progress.insert_at(
                "expired".to_owned(),
                done("expired", true),
                now - std::time::Duration::from_secs(31 * 60),
            );
            progress.insert_at(
                "running".to_owned(),
                done("running", false),
                now - std::time::Duration::from_secs(32 * 60),
            );
            for i in 0..260 {
                let id = format!("done-{i}");
                progress.insert_at(
                    id.clone(),
                    done(&id, true),
                    now + std::time::Duration::from_millis(i),
                );
            }
        }
        assert!(service.search_progress("expired").is_none());
        assert!(service.search_progress("done-0").is_none());
        assert!(service.search_progress("done-259").is_some());
        assert!(service.search_progress("running").is_some());
        assert!(registry.progress.lock().unwrap().entries.len() <= 257);
    }

    #[test]
    fn search_error_surfaces_bhoonidhi_complex_polygon_guidance() {
        let mut ctx = serde_json::Map::new();
        ctx.insert(
            "reason".to_owned(),
            serde_json::Value::String("aoi_too_complex".to_owned()),
        );
        let provider_err = insar_core::ContractError::new(
            "BHOONIDHI_QUERY_INVALID",
            "研究区多边形边界顶点过多（超过 Bhoonidhi 接口上限），请改用矩形或简化多边形框选",
            ctx,
        )
        .expect("safe provider error");
        let typed = search_error(&ApplicationServiceError::Provider(provider_err));
        assert_eq!(typed.code, "BHOONIDHI_QUERY_INVALID");
        assert!(typed.message_zh.contains("研究区多边形边界顶点过多"));

        let generic_err = insar_core::ContractError::new(
            "BHOONIDHI_QUERY_INVALID",
            "other",
            serde_json::Map::new(),
        )
        .expect("safe generic error");
        let generic_typed = search_error(&ApplicationServiceError::Provider(generic_err));
        assert_eq!(generic_typed.message_zh, "检索失败，请查看错误码与诊断");
    }

    #[test]
    fn sar_continuation_seal_activates_over_prior_optical_batch_and_updates_source_views() {
        static NEXT_DB_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        struct TempDb(std::path::PathBuf);
        impl Drop for TempDb {
            fn drop(&mut self) {
                for suffix in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
                }
            }
        }

        let db = TempDb(std::env::temp_dir().join(format!(
            "sar-cont-seal-{}-{}.sqlite3",
            std::process::id(),
            NEXT_DB_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )));
        let aoi: serde_json::Map<String, serde_json::Value> = serde_json::json!({
            "type": "Polygon",
            "coordinates": [[[91.0, 29.5], [91.5, 29.5], [91.5, 30.0], [91.0, 30.0], [91.0, 29.5]]]
        })
        .as_object()
        .unwrap()
        .clone();

        {
            let mut store = insar_store::TaskStore::open(&db.0).unwrap();
            store
                .save_search_batch(&insar_core::StoreSearchBatchRequest {
                    batch: insar_core::FrozenSearchBatch {
                        schema_version: 1,
                        batch_id: "batch-landsat-prior".to_owned(),
                        provider_id: "microsoft.planetary-computer".to_owned(),
                        created_at: "2026-11-17T00:00:00Z".to_owned(),
                        aoi_name: "合肥市".to_owned(),
                        aoi_geojson: Some(aoi.clone()),
                        query: serde_json::json!({
                            "logical_dataset": "optical.landsat.c2.l2",
                            "native_collection_id": "landsat-c2-l2"
                        })
                        .as_object()
                        .unwrap()
                        .clone(),
                        search_stats: None,
                        candidates: vec![insar_core::SnapshotItem {
                            item_id: "landsat-scene-1".to_owned(),
                            provider_id: "microsoft.planetary-computer".to_owned(),
                            display_name: "Landsat Scene 1".to_owned(),
                            acquisition_datetime: Some("2026-11-01T00:00:00Z".to_owned()),
                            footprint_geojson: None,
                            metadata: serde_json::Map::new(),
                            payload: serde_json::Map::new(),
                            asset_access: None,
                            execution_policy: None,
                        }],
                    },
                    workbench: insar_core::SearchWorkbenchState {
                        batch_id: "batch-landsat-prior".to_owned(),
                        checked_item_ids: vec![],
                        page_index: 0,
                        page_size: 50,
                        revision: 0,
                    },
                    activate_workbench: true,
                })
                .unwrap();
            store
                .record_source_view_success(
                    "science.landsat.c2-l2.v1",
                    "batch-landsat-prior",
                    None,
                    "2026-11-17T00:00:00Z",
                )
                .unwrap();
            assert_eq!(
                store.load_workbench().unwrap().unwrap().state.batch_id,
                "batch-landsat-prior"
            );
        }

        let service =
            crate::ApplicationService::new(crate::ApplicationServiceConfig::business_store(&db.0));
        let stage_sar_batch = |req_id: &str, batch_id: &str, created_at: &str, count: usize| {
            service.update_source_view(|store, _| {
                store.mark_source_view_pending("science.sentinel-1.standard.v1", req_id, created_at)
            });
            service.unified_searches.mark_continuation_active(req_id);
            service
                .with_business_runtime(|runtime| {
                    let mut store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    store.begin_search_batch_staging(
                        &insar_store::StoreSearchBatchStagingRequest {
                            request_id: req_id.to_owned(),
                            batch_header: insar_core::FrozenSearchBatch {
                                schema_version: 1,
                                batch_id: batch_id.to_owned(),
                                provider_id: "asf.catalog".to_owned(),
                                created_at: created_at.to_owned(),
                                aoi_name: "拉萨市".to_owned(),
                                aoi_geojson: Some(aoi.clone()),
                                query: serde_json::json!({ "dataset": "SENTINEL-1" })
                                    .as_object()
                                    .unwrap()
                                    .clone(),
                                search_stats: None,
                                candidates: Vec::new(),
                            },
                            workbench: insar_core::SearchWorkbenchState {
                                batch_id: batch_id.to_owned(),
                                checked_item_ids: Vec::new(),
                                page_index: 0,
                                page_size: 100,
                                revision: 0,
                            },
                            max_candidates: 2_000,
                        },
                    )?;
                    let items: Vec<insar_core::SnapshotItem> = (0..count)
                        .map(|idx| insar_core::SnapshotItem {
                            item_id: format!("{batch_id}-item-{idx:03}"),
                            provider_id: "asf.catalog".to_owned(),
                            display_name: format!("S1 Scene {idx}"),
                            acquisition_datetime: Some("2024-09-01T00:00:00Z".to_owned()),
                            footprint_geojson: None,
                            metadata: serde_json::Map::new(),
                            payload: serde_json::Map::new(),
                            asset_access: None,
                            execution_policy: None,
                        })
                        .collect();
                    store.append_search_batch_staging_candidates(req_id, &items)?;
                    store.publish_search_batch_staging_progressive(req_id, Some(count as u64))?;
                    Ok(())
                })
                .unwrap();
        };

        stage_sar_batch("req-s1-cont", "batch-s1-cont", "2026-11-17T00:01:00Z", 254);
        {
            let mut progress = service.unified_searches.progress.lock().unwrap();
            progress.insert_at(
                "req-s1-cont".to_owned(),
                SearchProgress {
                    request_id: "req-s1-cont".to_owned(),
                    resource_id: "science.sentinel-1.standard.v1".to_owned(),
                    stage: SearchProgressStage::Requesting,
                    requests_done: 1,
                    items_kept: 250,
                    items_cap: 2_000,
                    elapsed_ms: 100,
                    message_zh: "已优先载入前 250 景，正在后台继续拉取更多结果...".to_owned(),
                    terminal: false,
                    error: None,
                    possibly_incomplete: false,
                    batch_id: Some("batch-s1-cont".to_owned()),
                    loaded_count: Some(250),
                    remote_total: Some(254),
                },
                std::time::Instant::now(),
            );
        }

        // During active continuation: SearchProgress remains non-terminal with the
        // readable staged batch_id, and source_views keeps pending_request_id alive
        // until sealing completes.
        let live_progress = service.search_progress("req-s1-cont").unwrap();
        assert!(!live_progress.terminal);
        assert_eq!(live_progress.stage, SearchProgressStage::Requesting);
        assert_eq!(live_progress.batch_id.as_deref(), Some("batch-s1-cont"));
        let while_loading = service
            .source_views()
            .unwrap()
            .into_iter()
            .find(|v| v.resource_id == "science.sentinel-1.standard.v1")
            .unwrap();
        assert_eq!(
            while_loading.pending_request_id.as_deref(),
            Some("req-s1-cont")
        );

        let stats_254 = insar_core::FrozenSearchStats {
            matched_count: Some(254),
            fetched_record_count: Some(254),
            mapped_record_count: Some(254),
            provider_returned_count: 254,
            frozen_candidate_count: 254,
            truncated: false,
            source_attempts: vec![],
            source: "TYPED_CATALOG".to_owned(),
        };
        service
            .seal_sar_continuation_staging(
                "req-s1-cont",
                "2026-11-17T00:01:00Z",
                stats_254.clone(),
                insar_core::SearchBatchProgressState::Complete,
            )
            .unwrap();
        service.update_source_view(|store, at| {
            store.record_source_view_success(
                "science.sentinel-1.standard.v1",
                "batch-s1-cont",
                Some("req-s1-cont"),
                at,
            )
        });
        service.unified_searches.finish_continuation("req-s1-cont");
        {
            let mut progress = service.unified_searches.progress.lock().unwrap();
            let existing = progress.get("req-s1-cont").cloned().unwrap();
            progress.insert_at(
                "req-s1-cont".to_owned(),
                SearchProgress {
                    stage: SearchProgressStage::Done,
                    terminal: true,
                    items_kept: 254,
                    loaded_count: Some(254),
                    remote_total: Some(254),
                    message_zh: "已完成检索，共载入 254 景".to_owned(),
                    ..existing
                },
                std::time::Instant::now(),
            );
        }
        let terminal_progress = service.search_progress("req-s1-cont").unwrap();
        assert!(terminal_progress.terminal);
        assert_eq!(terminal_progress.stage, SearchProgressStage::Done);
        assert_eq!(terminal_progress.loaded_count, Some(254));

        let after_seal = service
            .source_views()
            .unwrap()
            .into_iter()
            .find(|v| v.resource_id == "science.sentinel-1.standard.v1")
            .unwrap();
        assert_eq!(after_seal.batch_id.as_deref(), Some("batch-s1-cont"));
        assert_eq!(after_seal.item_count, 254);
        assert_eq!(after_seal.pending_request_id, None);

        service
            .with_business_runtime(|runtime| {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                assert_eq!(
                    store.load_workbench()?.unwrap().state.batch_id,
                    "batch-s1-cont"
                );
                let headers = store.latest_frozen_batch_headers_per_route()?;
                assert!(
                    headers
                        .iter()
                        .any(|h| h.batch_id == "batch-s1-cont" && h.item_count == 254),
                    "sealed SAR continuation batch must be view_eligible=1: {headers:?}"
                );

                // Concurrent newer search guard: a newer batch in workbench_state must not
                // be overwritten by an older continuation seal, yet the older sealed batch
                // still stays view_eligible=1.
                store.save_search_batch(&insar_core::StoreSearchBatchRequest {
                    batch: insar_core::FrozenSearchBatch {
                        schema_version: 1,
                        batch_id: "batch-newer".to_owned(),
                        provider_id: "microsoft.planetary-computer".to_owned(),
                        created_at: "2026-11-17T00:03:00Z".to_owned(),
                        aoi_name: "拉萨市".to_owned(),
                        aoi_geojson: Some(aoi.clone()),
                        query: serde_json::json!({
                            "logical_dataset": "optical.landsat.c2.l2",
                            "native_collection_id": "landsat-c2-l2"
                        })
                        .as_object()
                        .unwrap()
                        .clone(),
                        search_stats: None,
                        candidates: vec![insar_core::SnapshotItem {
                            item_id: "landsat-newer-1".to_owned(),
                            provider_id: "microsoft.planetary-computer".to_owned(),
                            display_name: "Landsat Newer".to_owned(),
                            acquisition_datetime: Some("2026-11-02T00:00:00Z".to_owned()),
                            footprint_geojson: None,
                            metadata: serde_json::Map::new(),
                            payload: serde_json::Map::new(),
                            asset_access: None,
                            execution_policy: None,
                        }],
                    },
                    workbench: insar_core::SearchWorkbenchState {
                        batch_id: "batch-newer".to_owned(),
                        checked_item_ids: vec![],
                        page_index: 0,
                        page_size: 50,
                        revision: 0,
                    },
                    activate_workbench: true,
                })?;
                Ok(())
            })
            .unwrap();

        stage_sar_batch(
            "req-s1-older",
            "batch-s1-older",
            "2026-11-17T00:02:00Z",
            254,
        );
        service
            .seal_sar_continuation_staging(
                "req-s1-older",
                "2026-11-17T00:02:00Z",
                stats_254,
                insar_core::SearchBatchProgressState::Complete,
            )
            .unwrap();
        service.unified_searches.finish_continuation("req-s1-older");

        service
            .with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                assert_eq!(
                    store.load_workbench()?.unwrap().state.batch_id,
                    "batch-newer"
                );
                let headers = store.latest_frozen_batch_headers_per_route()?;
                assert!(
                    headers
                        .iter()
                        .any(|h| h.batch_id == "batch-s1-older" && h.item_count == 254),
                    "older sealed SAR batch must still be view_eligible=1: {headers:?}"
                );
                Ok(())
            })
            .unwrap();
        service.shutdown().unwrap();
    }

    #[test]
    fn mark_cancelled_ignores_unknown_requests_and_continuation_error_reports_failed_not_done() {
        use super::finalize_sar_continuation;
        use crate::ApplicationService;
        use serde_json::{json, Map, Value};
        use std::sync::{Arc, Mutex};
        use std::time::Instant;

        static NEXT_ERR_DB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        struct TempDb(std::path::PathBuf);
        impl Drop for TempDb {
            fn drop(&mut self) {
                for suffix in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
                }
            }
        }

        let db = TempDb(std::env::temp_dir().join(format!(
            "sar-cont-err-{}-{}.sqlite3",
            std::process::id(),
            NEXT_ERR_DB.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )));
        let _store = insar_store::TaskStore::open(&db.0).unwrap();
        let service = Arc::new(ApplicationService::new(
            crate::ApplicationServiceConfig::business_store(&db.0),
        ));

        // Unknown request IDs must not accumulate in `cancelled`.
        assert!(!service.search_cancel("unknown-request-id").unwrap());
        assert!(service
            .unified_searches
            .cancelled
            .lock()
            .unwrap()
            .is_empty());

        let sync_events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sync_capture = Arc::clone(&sync_events);
        service.register_search_sync_listener(Arc::new(move |payload| {
            if let Ok(mut list) = sync_capture.lock() {
                list.push(payload);
            }
        }));

        // Stage a SAR batch whose first page arrived (251) and then background continuation hit an error.
        let req_id = "req-s1-midfail";
        let batch_id = "batch-s1-midfail";
        let created_at = "2026-11-22T01:00:00Z";
        let aoi: Map<String, Value> = json!({
            "type": "Polygon",
            "coordinates": [[[91.0, 29.5], [91.5, 29.5], [91.5, 30.0], [91.0, 30.0], [91.0, 29.5]]]
        })
        .as_object()
        .unwrap()
        .clone();
        service.update_source_view(|store, _| {
            store.mark_source_view_pending("science.sentinel-1.standard.v1", req_id, created_at)
        });
        service.unified_searches.mark_continuation_active(req_id);
        service
            .with_business_runtime(|runtime| {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                store.begin_search_batch_staging(&insar_store::StoreSearchBatchStagingRequest {
                    request_id: req_id.to_owned(),
                    batch_header: insar_core::FrozenSearchBatch {
                        schema_version: 1,
                        batch_id: batch_id.to_owned(),
                        provider_id: "asf.catalog".to_owned(),
                        created_at: created_at.to_owned(),
                        aoi_name: "Lhasa".to_owned(),
                        aoi_geojson: Some(aoi),
                        query: json!({ "dataset": "SENTINEL-1" })
                            .as_object()
                            .unwrap()
                            .clone(),
                        search_stats: None,
                        candidates: Vec::new(),
                    },
                    workbench: insar_core::SearchWorkbenchState {
                        batch_id: batch_id.to_owned(),
                        checked_item_ids: Vec::new(),
                        page_index: 0,
                        page_size: 100,
                        revision: 0,
                    },
                    max_candidates: 2_000,
                })?;
                let items: Vec<insar_core::SnapshotItem> = (0..251)
                    .map(|idx| insar_core::SnapshotItem {
                        item_id: format!("item-{idx}"),
                        provider_id: "asf.catalog".to_owned(),
                        display_name: format!("S1 {idx}"),
                        acquisition_datetime: Some("2024-09-01T00:00:00Z".to_owned()),
                        footprint_geojson: None,
                        metadata: Map::new(),
                        payload: Map::new(),
                        asset_access: None,
                        execution_policy: None,
                    })
                    .collect();
                store.append_search_batch_staging_candidates(req_id, &items)?;
                store.publish_search_batch_staging_progressive(req_id, Some(251))?;
                Ok(())
            })
            .unwrap();
        {
            let mut progress = service.unified_searches.progress.lock().unwrap();
            progress.insert_at(
                req_id.to_owned(),
                SearchProgress {
                    request_id: req_id.to_owned(),
                    resource_id: "science.sentinel-1.standard.v1".to_owned(),
                    stage: SearchProgressStage::Requesting,
                    requests_done: 1,
                    items_kept: 251,
                    items_cap: 2_000,
                    elapsed_ms: 5,
                    message_zh: "continuing".to_owned(),
                    terminal: false,
                    error: None,
                    possibly_incomplete: false,
                    batch_id: Some(batch_id.to_owned()),
                    loaded_count: Some(251),
                    remote_total: Some(500),
                },
                Instant::now(),
            );
        }

        finalize_sar_continuation(
            &service,
            "science.sentinel-1.standard.v1",
            req_id,
            batch_id,
            created_at,
            2_000,
            251,
            Some(500),
            false,
            Some(ApplicationServiceError::Provider(
                insar_core::ContractError::new("V3_PROVIDER_TIMEOUT", "timeout", Map::new())
                    .unwrap(),
            )),
            insar_core::FrozenSearchStats {
                matched_count: Some(500),
                fetched_record_count: Some(251),
                mapped_record_count: Some(251),
                provider_returned_count: 251,
                frozen_candidate_count: 251,
                truncated: true,
                source: "TYPED_CATALOG".to_owned(),
                source_attempts: Vec::new(),
            },
            None,
        );

        let prog = service.search_progress(req_id).unwrap();
        assert_eq!(prog.stage, SearchProgressStage::Failed);
        assert!(prog.terminal);
        assert!(prog.error.is_some());
        assert!(prog.possibly_incomplete);
        assert_eq!(prog.batch_id.as_deref(), Some(batch_id));
        let emitted = sync_events.lock().unwrap();
        assert_eq!(
            emitted
                .last()
                .and_then(|v| v.get("complete"))
                .and_then(Value::as_bool),
            Some(false)
        );
        drop(emitted);
        assert!(!service.unified_searches.has_active_continuation(req_id));

        // Major #1 verification: SQLite `search_batch_progress.state` must be `Failed`, and
        // `preview_all_match_task` must reject it with `V3_TASK_BATCH_INCOMPLETE`.
        let failed_db_progress = service.search_batch_progress(batch_id).unwrap().unwrap();
        assert_eq!(
            failed_db_progress.state,
            insar_core::SearchBatchProgressState::Failed
        );
        let all_match_err = service
            .preview_all_match_task(crate::BatchAllMatchPreviewRequest {
                batch_id: batch_id.to_owned(),
                revision: 0,
                predicate_version: crate::ALL_MATCH_PREDICATE_VERSION_V1,
                predicate: Default::default(),
                exclusions: Vec::new(),
            })
            .expect_err("incomplete failed continuation batch must reject all-match");
        assert_eq!(all_match_err.code, "V3_TASK_BATCH_INCOMPLETE");

        // Helper closure to stage 251 progressive SAR candidates for additional continuation cases.
        let stage_prog = |req: &str, batch: &str| {
            let aoi_map: Map<String, Value> = json!({
                "type": "Polygon",
                "coordinates": [[[91.0, 29.5], [91.5, 29.5], [91.5, 30.0], [91.0, 30.0], [91.0, 29.5]]]
            })
            .as_object()
            .unwrap()
            .clone();
            service.update_source_view(|store, _| {
                store.mark_source_view_pending("science.sentinel-1.standard.v1", req, created_at)
            });
            service.unified_searches.mark_continuation_active(req);
            service.unified_searches.record_published(req, batch);
            service
                .with_business_runtime(|runtime| {
                    let mut store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    store.begin_search_batch_staging(
                        &insar_store::StoreSearchBatchStagingRequest {
                            request_id: req.to_owned(),
                            batch_header: insar_core::FrozenSearchBatch {
                                schema_version: 1,
                                batch_id: batch.to_owned(),
                                provider_id: "asf.catalog".to_owned(),
                                created_at: created_at.to_owned(),
                                aoi_name: "Lhasa".to_owned(),
                                aoi_geojson: Some(aoi_map),
                                query: json!({ "dataset": "SENTINEL-1" })
                                    .as_object()
                                    .unwrap()
                                    .clone(),
                                search_stats: None,
                                candidates: Vec::new(),
                            },
                            workbench: insar_core::SearchWorkbenchState {
                                batch_id: batch.to_owned(),
                                checked_item_ids: Vec::new(),
                                page_index: 0,
                                page_size: 100,
                                revision: 0,
                            },
                            max_candidates: 2_000,
                        },
                    )?;
                    let items: Vec<insar_core::SnapshotItem> = (0..251)
                        .map(|idx| insar_core::SnapshotItem {
                            item_id: format!("item-{req}-{idx}"),
                            provider_id: "asf.catalog".to_owned(),
                            display_name: format!("S1 {idx}"),
                            acquisition_datetime: Some("2024-09-01T00:00:00Z".to_owned()),
                            footprint_geojson: None,
                            metadata: Map::new(),
                            payload: Map::new(),
                            asset_access: None,
                            execution_policy: None,
                        })
                        .collect();
                    store.append_search_batch_staging_candidates(req, &items)?;
                    store.publish_search_batch_staging_progressive(req, Some(251))?;
                    Ok(())
                })
                .unwrap();
            let mut progress = service.unified_searches.progress.lock().unwrap();
            progress.insert_at(
                req.to_owned(),
                SearchProgress {
                    request_id: req.to_owned(),
                    resource_id: "science.sentinel-1.standard.v1".to_owned(),
                    stage: SearchProgressStage::Requesting,
                    requests_done: 1,
                    items_kept: 251,
                    items_cap: 2_000,
                    elapsed_ms: 5,
                    message_zh: "continuing".to_owned(),
                    terminal: false,
                    error: None,
                    possibly_incomplete: false,
                    batch_id: Some(batch.to_owned()),
                    loaded_count: Some(251),
                    remote_total: Some(500),
                },
                Instant::now(),
            );
        };

        // Case A1: cancelled continuation seals SQLite state as `Cancelled` and rejects all-match.
        stage_prog("req-s1-cancel", "batch-s1-cancel");
        // No in-flight HTTP request in dispatcher, yet `mark_cancelled` tracks active continuation.
        assert!(!service.search_cancel("req-s1-cancel").unwrap());
        assert!(service.unified_searches.is_cancelled("req-s1-cancel"));
        finalize_sar_continuation(
            &service,
            "science.sentinel-1.standard.v1",
            "req-s1-cancel",
            "batch-s1-cancel",
            created_at,
            2_000,
            251,
            Some(500),
            false,
            None,
            insar_core::FrozenSearchStats {
                matched_count: Some(500),
                fetched_record_count: Some(251),
                mapped_record_count: Some(251),
                provider_returned_count: 251,
                frozen_candidate_count: 251,
                truncated: false, // finalize_sar_continuation must enforce truncated=true for Cancelled
                source: "TYPED_CATALOG".to_owned(),
                source_attempts: Vec::new(),
            },
            None,
        );
        let cancel_prog = service.search_progress("req-s1-cancel").unwrap();
        assert_eq!(cancel_prog.stage, SearchProgressStage::Cancelled);
        assert!(cancel_prog.terminal);
        assert!(cancel_prog.possibly_incomplete);
        let cancel_db = service
            .search_batch_progress("batch-s1-cancel")
            .unwrap()
            .unwrap();
        assert_eq!(
            cancel_db.state,
            insar_core::SearchBatchProgressState::Cancelled
        );
        let cancel_all_match_err = service
            .preview_all_match_task(crate::BatchAllMatchPreviewRequest {
                batch_id: "batch-s1-cancel".to_owned(),
                revision: 0,
                predicate_version: crate::ALL_MATCH_PREDICATE_VERSION_V1,
                predicate: Default::default(),
                exclusions: Vec::new(),
            })
            .expect_err("cancelled continuation batch must reject all-match");
        assert_eq!(cancel_all_match_err.code, "V3_TASK_BATCH_INCOMPLETE");

        // Case B2: non-shutdown seal failure (mismatched candidate count) withdraws `'loading'` and clears `published`.
        stage_prog("req-s1-sealfail", "batch-s1-sealfail");
        assert_eq!(
            service.search_batch_id("req-s1-sealfail").as_deref(),
            Some("batch-s1-sealfail")
        );
        finalize_sar_continuation(
            &service,
            "science.sentinel-1.standard.v1",
            "req-s1-sealfail",
            "batch-s1-sealfail",
            created_at,
            2_000,
            251,
            Some(500),
            false,
            None,
            insar_core::FrozenSearchStats {
                matched_count: Some(500),
                fetched_record_count: Some(999),
                mapped_record_count: Some(999),
                provider_returned_count: 999,
                frozen_candidate_count: 999, // intentional count mismatch -> seal fails
                truncated: false,
                source: "TYPED_CATALOG".to_owned(),
                source_attempts: Vec::new(),
            },
            None,
        );
        let sealfail_prog = service.search_progress("req-s1-sealfail").unwrap();
        assert_eq!(sealfail_prog.stage, SearchProgressStage::Failed);
        assert!(sealfail_prog.terminal);
        assert_eq!(sealfail_prog.batch_id, None);
        assert_eq!(service.search_batch_id("req-s1-sealfail"), None);
        assert!(service
            .search_batch_progress("batch-s1-sealfail")
            .unwrap()
            .is_none());

        // Minor #2 & prune_stale_cancellations: early drop of ContinuationGuard marks terminal Failed & cleans sets.
        service
            .unified_searches
            .mark_continuation_active("req-s1-guard");
        {
            let mut progress = service.unified_searches.progress.lock().unwrap();
            progress.insert_at(
                "req-s1-guard".to_owned(),
                SearchProgress {
                    request_id: "req-s1-guard".to_owned(),
                    resource_id: "science.sentinel-1.standard.v1".to_owned(),
                    stage: SearchProgressStage::Requesting,
                    requests_done: 1,
                    items_kept: 251,
                    items_cap: 2_000,
                    elapsed_ms: 5,
                    message_zh: "active".to_owned(),
                    terminal: false,
                    error: None,
                    possibly_incomplete: false,
                    batch_id: None,
                    loaded_count: Some(251),
                    remote_total: None,
                },
                Instant::now(),
            );
        }
        assert!(!service.search_cancel("req-s1-guard").unwrap());
        assert!(service.unified_searches.is_cancelled("req-s1-guard"));
        {
            let _guard = super::ContinuationGuard {
                service: Arc::clone(&service),
                request_id: "req-s1-guard".to_owned(),
                resource_id: "science.sentinel-1.standard.v1".to_owned(),
                completed: false,
            };
        }
        assert!(!service
            .unified_searches
            .has_active_continuation("req-s1-guard"));
        assert!(!service.unified_searches.is_cancelled("req-s1-guard"));
        let guard_prog = service.search_progress("req-s1-guard").unwrap();
        assert_eq!(guard_prog.stage, SearchProgressStage::Failed);
        assert!(guard_prog.terminal);
        assert!(guard_prog.possibly_incomplete);

        service
            .unified_searches
            .cancelled
            .lock()
            .unwrap()
            .insert("stale-orphan-id".to_owned());
        service.unified_searches.prune_stale_cancellations();
        assert!(!service.unified_searches.is_cancelled("stale-orphan-id"));

        let range_err = ApplicationServiceError::Provider(
            insar_core::ContractError::new(
                "ASF_QUERY_INVALID",
                "Filter value is outside the dataset capability range",
                serde_json::Map::from_iter([
                    (
                        "field".to_owned(),
                        Value::String("relative_orbit".to_owned()),
                    ),
                    (
                        "reason".to_owned(),
                        Value::String("out_of_range".to_owned()),
                    ),
                    ("min".to_owned(), Value::from(1)),
                    ("max".to_owned(), Value::from(175)),
                ]),
            )
            .unwrap(),
        );
        let mapped_err = super::search_error(&range_err);
        assert_eq!(mapped_err.code, "ASF_QUERY_INVALID");
        assert_eq!(
            mapped_err.message_zh,
            "Path / 轨道号超出合法范围（允许 1–175）"
        );

        service.shutdown().unwrap();
    }
}
