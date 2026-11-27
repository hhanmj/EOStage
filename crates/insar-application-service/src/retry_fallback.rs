//! C1 `application_task_retry_fallback`: re-run a failed task's frozen search
//! and rebuild its download items inside the same task ID.
//!
//! The original batch and task snapshot stay immutable. The re-search freezes
//! a new batch (never activating the legacy Workbench slot); Store then
//! records the matching items as a new task snapshot revision and points the
//! task at it before retrying. Products missing from the new batch require
//! user confirmation bound to that exact batch (the latest one frozen for the
//! failure, recorded in the task's snapshot head); new products are never added.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;

use insar_core::{
    AccountId, CatalogSearchQuery, FrozenSearchBatch, FrozenTaskSnapshot, SarRemoteSearchRequest,
    SarSearchQuery, SearchWorkbenchState, SnapshotItem, StoreSearchBatchRequest,
    TaskLifecycleStatus, TaskRef, TypedError,
};
use insar_provider::catalog::RouteUse;
use insar_store::{
    is_replayable_optical_delivery_kind, StoreRetryFallbackRequest, TaskStore,
    RETRY_FALLBACK_BATCH_PREFIX,
};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    build_planetary_computer_reviewed_cog_item, frozen_landsat_archive_source_evidence,
    frozen_landsat_scene_evidence, frozen_reviewed_mpc_source_evidence,
    frozen_sentinel2_analysis_source, reviewed_mpc_asset_keys_allowed, ApplicationService,
    ApplicationServiceError, BusinessRuntimeFactory, PlanetaryComputerAssetSigner,
    PlanetaryComputerReviewedAssetEvidence, Sentinel2ArchiveAdmissionOutcome,
    Sentinel2ArchiveBatchLookupContext, Sentinel2BandEvidenceRuntime,
    SENTINEL2_ARCHIVE_BATCH_LOOKUP_CONCURRENCY,
};

/// C1 response (contracts/v3 `TaskRetryFallbackOutcome`): `task` is the
/// original task (`created` is always false); `refreshed_batch_id` is passed
/// back as the confirmation; `added_product_ids` is the refreshed batch minus
/// the original batch, reported only and never added to the task.
pub use insar_core::TaskRetryFallbackOutcome;

fn typed(code: &str, message_zh: &str, retryable: bool) -> TypedError {
    TypedError {
        code: code.to_owned(),
        message_zh: message_zh.to_owned(),
        retryable,
        needs_account: None,
        needs_user_confirmation: false,
    }
}

fn unsupported() -> TypedError {
    typed(
        "RETRY_FALLBACK_UNSUPPORTED",
        "该任务的数据源或类型暂不支持重检兜底，请使用普通重试",
        false,
    )
}

fn task_not_failed() -> TypedError {
    typed(
        "RETRY_FALLBACK_TASK_NOT_FAILED",
        "只有失败的任务可以重检兜底",
        false,
    )
}

fn confirmation_mismatch() -> TypedError {
    typed(
        "RETRY_FALLBACK_CONFIRMATION_MISMATCH",
        "确认的重检结果已过期或不属于本次重检，请重新发起重检兜底",
        false,
    )
}

fn account_for_provider(provider_hint: Option<&str>) -> Option<(AccountId, &'static str)> {
    let hint = provider_hint.unwrap_or_default();
    if hint.contains("asf") || hint.contains("earthdata") {
        Some((
            AccountId::Earthdata,
            "重检需要有效的 Earthdata 登录，请登录后再试；原任务未改动",
        ))
    } else if hint.contains("usgs") || hint.contains("landsat_c2_l2") {
        Some((
            AccountId::UsgsM2m,
            "重检需要有效的 USGS EarthExplorer 登录，请登录后再试；原任务未改动",
        ))
    } else if hint.contains("copernicus") || hint.contains("cdse") {
        Some((
            AccountId::Cdse,
            "重检需要有效的 Copernicus Dataspace 登录，请登录后再试；原任务未改动",
        ))
    } else {
        None
    }
}

/// Retry-fallback errors have their own wording: nothing here creates a
/// task, and every failure leaves the original task unchanged.
fn service_error(error: ApplicationServiceError, provider_hint: Option<&str>) -> TypedError {
    let code = error.code();
    let (message_zh, retryable, needs_account) = match code {
        "RETRY_FALLBACK_UNSUPPORTED" => return unsupported(),
        "RETRY_FALLBACK_TASK_NOT_FAILED" => return task_not_failed(),
        "RETRY_FALLBACK_CONFIRMATION_MISMATCH" => return confirmation_mismatch(),
        "RETRY_FALLBACK_ITEM_NOT_RETRYABLE" => (
            "只有失败或已取消的产品可以重检重建，原任务未改动",
            false,
            None,
        ),
        _ if ["CREDENTIAL", "UNAUTHORIZED", "AUTH", "SESSION"]
            .iter()
            .any(|term| code.contains(term)) =>
        {
            if let Some((account, msg)) = account_for_provider(provider_hint) {
                (msg, false, Some(account))
            } else {
                (
                    "重检认证失败，请检查账号或网络设置后再试；原任务未改动",
                    false,
                    None,
                )
            }
        }
        _ if ["NETWORK", "TRANSPORT", "TIMEOUT"]
            .iter()
            .any(|term| code.contains(term)) =>
        {
            (
                "网络暂不可用，重检未完成，原任务未改动，请稍后再试",
                true,
                None,
            )
        }
        _ if code.contains("REVISION_CONFLICT") => (
            "任务状态已变化，请刷新任务后再试重检兜底；原任务未改动",
            false,
            None,
        ),
        _ if code.contains("NOT_FOUND") => (
            "任务或重检结果已不存在，请刷新后再试；原任务未改动",
            false,
            None,
        ),
        _ if code.contains("POISONED")
            || code.contains("UNAVAILABLE")
            || code.contains("NOT_READY")
            || code.contains("BUSY") =>
        {
            ("暂时无法读取任务记录，原任务未改动，请稍后再试", true, None)
        }
        _ if code.starts_with("V3_PROVIDER") => {
            ("重检未完成，原任务未改动，请稍后再试", true, None)
        }
        _ => ("重检兜底未完成，原任务未改动，请稍后再试", false, None),
    };
    TypedError {
        code: code.to_owned(),
        message_zh: message_zh.to_owned(),
        retryable,
        needs_account,
        needs_user_confirmation: false,
    }
}

enum ReplayPlan {
    Sar(Box<SarSearchQuery>),
    Catalog {
        query: Box<CatalogSearchQuery>,
    },
    DerivedOptical {
        source_batch: Box<FrozenSearchBatch>,
        source_query: Box<CatalogSearchQuery>,
        delivery_kind: String,
    },
}

fn classify_replay_plan_internal(
    snapshot: &FrozenTaskSnapshot,
    is_explicit: bool,
    store: &TaskStore,
) -> Result<Option<ReplayPlan>, insar_store::StoreError> {
    if is_explicit || snapshot.operation.is_some() || snapshot.orbit_association.is_some() {
        return Ok(None);
    }

    if snapshot.query.contains_key("derived_from_batch_id")
        || snapshot.query.contains_key("delivery_kind")
    {
        let Some(source_batch_id) = snapshot
            .query
            .get("derived_from_batch_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(None);
        };
        let Some(delivery_kind) = snapshot
            .query
            .get("delivery_kind")
            .and_then(Value::as_str)
            .filter(|kind| is_replayable_optical_delivery_kind(kind))
        else {
            return Ok(None);
        };
        let Some(source_batch) = store.load_search_batch(source_batch_id)? else {
            return Ok(None);
        };
        let Ok(source_query) =
            serde_json::from_value::<CatalogSearchQuery>(Value::Object(source_batch.query.clone()))
        else {
            return Ok(None);
        };
        if source_query.validate().is_err()
            || source_batch.provider_id != source_query.provider_id
            || source_batch.aoi_geojson != source_query.aoi_geojson
        {
            return Ok(None);
        }
        return Ok(Some(ReplayPlan::DerivedOptical {
            source_batch: Box::new(source_batch),
            source_query: Box::new(source_query),
            delivery_kind: delivery_kind.to_owned(),
        }));
    }

    if snapshot.candidates.iter().any(|item| {
        item.metadata.contains_key("source_item_id") || item.metadata.contains_key("delivery_kind")
    }) {
        return Ok(None);
    }

    if snapshot.provider_id == "asf.catalog" {
        if snapshot.query.contains_key("logical_dataset") {
            return Ok(None);
        }
        let Ok(query) =
            serde_json::from_value::<SarSearchQuery>(Value::Object(snapshot.query.clone()))
        else {
            return Ok(None);
        };
        if query.validate().is_err() {
            return Ok(None);
        }
        return Ok(Some(ReplayPlan::Sar(Box::new(query))));
    }

    let Ok(query) =
        serde_json::from_value::<CatalogSearchQuery>(Value::Object(snapshot.query.clone()))
    else {
        return Ok(None);
    };
    if query.validate().is_err()
        || snapshot.provider_id != query.provider_id
        || snapshot.aoi_geojson != query.aoi_geojson
    {
        return Ok(None);
    }
    Ok(Some(ReplayPlan::Catalog {
        query: Box::new(query),
    }))
}

fn classify_replay_plan(
    snapshot: &FrozenTaskSnapshot,
    is_explicit: bool,
    store: &TaskStore,
) -> Result<ReplayPlan, TypedError> {
    match classify_replay_plan_internal(snapshot, is_explicit, store) {
        Ok(Some(plan)) => Ok(plan),
        Ok(None) => Err(unsupported()),
        Err(error) => Err(service_error(error.into(), Some(&snapshot.provider_id))),
    }
}

fn same_frozen_conditions(task: &FrozenTaskSnapshot, batch: &FrozenSearchBatch) -> bool {
    task.provider_id == batch.provider_id
        && task.query == batch.query
        && task.aoi_geojson == batch.aoi_geojson
        && task.batch_id != batch.batch_id
}

fn instant(value: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(value).ok()
}

fn now_after_rfc3339(baseline: &str) -> String {
    let now = chrono::Utc::now();
    let candidate = instant(baseline)
        .map(|base| base.with_timezone(&chrono::Utc) + chrono::TimeDelta::milliseconds(1))
        .filter(|next| *next > now)
        .unwrap_or(now);
    candidate.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn asset_key_from_derived_item(item: &SnapshotItem) -> Option<&str> {
    item.metadata
        .get("delivery_asset_key")
        .and_then(Value::as_str)
        .or_else(|| {
            item.asset_access
                .as_ref()
                .map(|access| access.reference.asset_key.as_str())
        })
}

fn find_refreshed_source_item<'a>(
    source_batch: &FrozenSearchBatch,
    refreshed: &'a [SnapshotItem],
    source_item_id: &str,
    fallback_display_name: Option<&str>,
) -> Option<&'a SnapshotItem> {
    if let Some(found) = refreshed
        .iter()
        .find(|candidate| candidate.item_id == source_item_id)
    {
        return Some(found);
    }
    let display_name = fallback_display_name.or_else(|| {
        source_batch
            .candidates
            .iter()
            .find(|candidate| candidate.item_id == source_item_id)
            .map(|candidate| candidate.display_name.as_str())
    })?;
    refreshed
        .iter()
        .find(|candidate| candidate.display_name == display_name)
}

/// Group selected derived items by `(source_item_id, ordered unique asset_keys)`.
fn collect_derived_asset_groups(
    task: &FrozenTaskSnapshot,
    source_batch: &FrozenSearchBatch,
) -> Vec<(String, Option<String>, Vec<String>)> {
    let mut order: Vec<String> = Vec::new();
    let mut keys_by_source: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut display_by_source: BTreeMap<String, Option<String>> = BTreeMap::new();
    let default_source_id = task
        .query
        .get("delivery_source_item_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let default_display_name = task
        .query
        .get("delivery_safe_name")
        .and_then(Value::as_str)
        .map(str::to_owned);

    for item in &task.candidates {
        let source_id = item
            .metadata
            .get("source_item_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| default_source_id.clone())
            .or_else(|| {
                source_batch
                    .candidates
                    .iter()
                    .find(|cand| cand.display_name == item.display_name)
                    .map(|cand| cand.item_id.clone())
            })
            .unwrap_or_else(|| item.item_id.clone());

        let fallback_display = default_display_name.clone().or_else(|| {
            source_batch
                .candidates
                .iter()
                .find(|cand| cand.item_id == source_id)
                .map(|cand| cand.display_name.clone())
        });

        let keys = keys_by_source.entry(source_id.clone()).or_insert_with(|| {
            order.push(source_id.clone());
            Vec::new()
        });
        display_by_source
            .entry(source_id)
            .or_insert(fallback_display);
        if let Some(asset_key) = asset_key_from_derived_item(item) {
            if !keys.iter().any(|existing| existing == asset_key) {
                keys.push(asset_key.to_owned());
            }
        }
    }

    order
        .into_iter()
        .map(|source_id| {
            let keys = keys_by_source.remove(&source_id).unwrap_or_default();
            let display = display_by_source.remove(&source_id).flatten();
            (source_id, display, keys)
        })
        .collect()
}

impl ApplicationService {
    /// Inspect whether each requested `task_id` owns a replayable search query
    /// (legacy ASF `SarSearchQuery`, unified `CatalogSearchQuery`, or derived
    /// optical delivery batch whose underlying source search batch still exists).
    pub fn task_retry_fallback_eligibility(
        &self,
        task_ids: &[String],
    ) -> Result<BTreeMap<String, bool>, ApplicationServiceError> {
        self.startup()?;
        let store = self.store_handle()?;
        let store = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let mut eligibility = BTreeMap::new();
        for task_id in task_ids {
            if task_id.trim().is_empty() {
                continue;
            }
            let eligible = match store.load_task_record(task_id)? {
                Some(task) => {
                    let is_explicit = store.is_task_explicit(task_id)?;
                    classify_replay_plan_internal(&task.snapshot, is_explicit, &store)?.is_some()
                }
                None => false,
            };
            eligibility.insert(task_id.clone(), eligible);
        }
        Ok(eligibility)
    }

    pub fn task_retry_fallback(
        &self,
        task_id: String,
        confirm_refreshed_batch_id: Option<String>,
    ) -> Result<TaskRetryFallbackOutcome, TypedError> {
        if task_id.trim().is_empty() {
            return Err(typed(
                "V3_APPLICATION_INVALID_REQUEST",
                "任务编号无效",
                false,
            ));
        }
        let store = self
            .store_handle()
            .map_err(|error| service_error(error, None))?;
        let (task, is_explicit, replay_plan, source_ids) = {
            let store = store
                .lock()
                .map_err(|_| service_error(ApplicationServiceError::StatePoisoned, None))?;
            let task = store
                .load_task_record(&task_id)
                .map_err(|error| service_error(error.into(), None))?
                .ok_or_else(|| typed("V3_STORE_NOT_FOUND", "任务不存在，请刷新任务列表", false))?;
            let provider_hint = task.snapshot.provider_id.clone();
            let is_explicit = store
                .is_task_explicit(&task_id)
                .map_err(|error| service_error(error.into(), Some(&provider_hint)))?;
            let replay_plan = classify_replay_plan(&task.snapshot, is_explicit, &store)?;
            let source_ids = store
                .load_search_batch(&task.snapshot.batch_id)
                .map_err(|error| service_error(error.into(), Some(&provider_hint)))?
                .map(|batch| {
                    batch
                        .candidates
                        .into_iter()
                        .map(|item| item.item_id)
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_else(|| {
                    task.snapshot
                        .candidates
                        .iter()
                        .map(|item| item.item_id.clone())
                        .collect()
                });
            (task, is_explicit, replay_plan, source_ids)
        };
        let _ = is_explicit;
        if task.runtime.status != TaskLifecycleStatus::Failed {
            return Err(task_not_failed());
        }

        let refreshed = match &confirm_refreshed_batch_id {
            Some(batch_id) => self.offered_fallback_batch(&task, batch_id)?,
            None => match replay_plan {
                ReplayPlan::Sar(query) => self.refresh_sar_batch(
                    &task.snapshot,
                    &task.runtime.updated_at,
                    task.runtime.revision,
                    *query,
                )?,
                ReplayPlan::Catalog { query } => self.refresh_catalog_batch(
                    &task.snapshot,
                    &task.runtime.updated_at,
                    task.runtime.revision,
                    &query,
                )?,
                ReplayPlan::DerivedOptical {
                    source_batch,
                    source_query,
                    delivery_kind,
                } => self.refresh_derived_optical_batch(
                    &task.snapshot,
                    &task.runtime.updated_at,
                    task.runtime.revision,
                    &source_batch,
                    &source_query,
                    &delivery_kind,
                )?,
            },
        };

        let refreshed_ids = refreshed
            .candidates
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<BTreeSet<_>>();
        // Only failed and cancelled products are retried; completed or skipped
        // products neither need the refreshed batch nor count as missing.
        let retryable = task
            .runtime
            .failed_item_ids
            .iter()
            .chain(&task.runtime.cancelled_item_ids)
            .collect::<Vec<_>>();
        let missing_product_ids = retryable
            .iter()
            .filter(|id| !refreshed_ids.contains(id.as_str()))
            .map(|id| (*id).clone())
            .collect::<Vec<_>>();
        let original_ids = source_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let added_product_ids = crate::unified_tasks::added_products(&original_ids, &refreshed_ids);
        let present = retryable
            .iter()
            .filter(|id| refreshed_ids.contains(id.as_str()))
            .map(|id| (*id).clone())
            .collect::<Vec<_>>();
        if present.is_empty() {
            return Err(typed(
                "RETRY_FALLBACK_NO_PRODUCTS",
                "重检结果中没有原任务的任何产品",
                false,
            ));
        }
        let current = TaskRef {
            task_id: task.snapshot.task_id.clone(),
            revision: task.runtime.revision,
            status: task.runtime.status.clone(),
            warning: None,
            created: false,
        };
        if !missing_product_ids.is_empty() && confirm_refreshed_batch_id.is_none() {
            return Ok(TaskRetryFallbackOutcome {
                task: current,
                refreshed_batch_id: refreshed.batch_id,
                rebuilt_item_ids: Vec::new(),
                missing_product_ids,
                added_product_ids,
                needs_user_confirmation: true,
            });
        }
        let request = StoreRetryFallbackRequest {
            task_id: task.snapshot.task_id.clone(),
            expected_revision: task.runtime.revision,
            refreshed_batch_id: refreshed.batch_id.clone(),
            item_ids: present.clone(),
            event_id: format!("retry-fallback-{}", Uuid::new_v4()),
            occurred_at: crate::unified_sources::now_rfc3339(),
        };
        let provider_hint = task.snapshot.provider_id.clone();
        let rebuilt = self
            .with_business_runtime(|runtime| {
                let task = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?
                    .rebind_task_items_and_retry(&request)?;
                if let Some(scheduler) = runtime.scheduler.as_ref() {
                    scheduler.wake();
                }
                Ok(task)
            })
            .map_err(|error| service_error(error, Some(&provider_hint)))?;
        Ok(TaskRetryFallbackOutcome {
            task: TaskRef {
                task_id: rebuilt.snapshot.task_id,
                revision: rebuilt.runtime.revision,
                status: rebuilt.runtime.status,
                warning: None,
                created: false,
            },
            refreshed_batch_id: refreshed.batch_id,
            rebuilt_item_ids: present,
            missing_product_ids,
            added_product_ids,
            needs_user_confirmation: false,
        })
    }

    /// The confirmation must name the batch most recently frozen for this
    /// failed task revision: a fallback re-search created after the failure,
    /// with the task's frozen conditions. Any older batch, even one with the
    /// same conditions, is refused.
    fn offered_fallback_batch(
        &self,
        task: &insar_store::StoredTask,
        batch_id: &str,
    ) -> Result<FrozenSearchBatch, TypedError> {
        let provider_hint = task.snapshot.provider_id.clone();
        let store = self
            .store_handle()
            .map_err(|error| service_error(error, Some(&provider_hint)))?;
        let (offer, batch) = {
            let store = store.lock().map_err(|_| {
                service_error(ApplicationServiceError::StatePoisoned, Some(&provider_hint))
            })?;
            (
                store
                    .retry_fallback_offer(&task.snapshot.task_id)
                    .map_err(|error| service_error(error.into(), Some(&provider_hint)))?,
                store
                    .load_search_batch(batch_id)
                    .map_err(|error| service_error(error.into(), Some(&provider_hint)))?,
            )
        };
        let batch = batch.ok_or_else(confirmation_mismatch)?;
        let after_failure = instant(&batch.created_at)
            .zip(instant(&task.runtime.updated_at))
            .is_some_and(|(created, failed)| created > failed);
        if offer != Some((batch_id.to_owned(), task.runtime.revision))
            || !batch.batch_id.starts_with(RETRY_FALLBACK_BATCH_PREFIX)
            || !after_failure
            || !same_frozen_conditions(&task.snapshot, &batch)
        {
            return Err(confirmation_mismatch());
        }
        Ok(batch)
    }

    fn freeze_fallback_batch(
        &self,
        task: &FrozenTaskSnapshot,
        task_revision: u64,
        batch: FrozenSearchBatch,
    ) -> Result<FrozenSearchBatch, TypedError> {
        let provider_hint = task.provider_id.clone();
        if !same_frozen_conditions(task, &batch) {
            return Err(unsupported());
        }
        let request = StoreSearchBatchRequest {
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: 100,
                revision: 0,
            },
            batch: batch.clone(),
            activate_workbench: false,
        };
        self.with_business_runtime(|runtime| {
            runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?
                .offer_retry_fallback_batch(&task.task_id, task_revision, &request)
                .map_err(ApplicationServiceError::from)
        })
        .map_err(|error| service_error(error, Some(&provider_hint)))?;
        Ok(batch)
    }

    /// Replay the frozen SAR query directly against the provider (no result
    /// cache, which could answer with the same stale items) and freeze the
    /// answer as a new immutable batch without activating the Workbench.
    fn refresh_sar_batch(
        &self,
        task: &FrozenTaskSnapshot,
        task_updated_at: &str,
        task_revision: u64,
        query: SarSearchQuery,
    ) -> Result<FrozenSearchBatch, TypedError> {
        let provider_hint = task.provider_id.as_str();
        let dispatcher = self
            .dispatcher_handle()
            .map_err(|error| service_error(error, Some(provider_hint)))?;
        let request_id = format!("retry-fallback-{}", Uuid::new_v4());
        let response = dispatcher
            .search_sar_for_application_with_progress(
                &request_id,
                &SarRemoteSearchRequest { query },
                &|_| {},
            )
            .map_err(|error| {
                service_error(
                    ApplicationServiceError::Provider(error),
                    Some(provider_hint),
                )
            })?;
        if !response.errors.is_empty()
            || response
                .stats
                .mapping_error_count
                .is_some_and(|count| count > 0)
        {
            return Err(typed(
                "RETRY_FALLBACK_SEARCH_INCOMPLETE",
                "重检结果不完整，原任务未改动，请稍后再试",
                true,
            ));
        }
        let count = response.products.len();
        let stats = crate::normalize_legacy_asf_search_stats(&response.stats, count);
        let candidates = response
            .products
            .into_iter()
            .map(crate::map_sar_product)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| service_error(error, Some(provider_hint)))?;
        if candidates.is_empty() {
            return Err(typed(
                "RETRY_FALLBACK_NO_PRODUCTS",
                "重检结果中没有原任务的任何产品",
                false,
            ));
        }
        let provider_id = crate::sar_candidate_provider_id(&candidates)
            .map_err(|error| service_error(error, Some(provider_hint)))?;
        let batch = FrozenSearchBatch {
            schema_version: 1,
            batch_id: format!("{RETRY_FALLBACK_BATCH_PREFIX}{}", Uuid::new_v4()),
            provider_id,
            created_at: now_after_rfc3339(task_updated_at),
            aoi_name: task.aoi_name.clone(),
            aoi_geojson: task.aoi_geojson.clone(),
            query: task.query.clone(),
            search_stats: Some(stats),
            candidates,
        };
        self.freeze_fallback_batch(task, task_revision, batch)
    }

    fn execute_live_catalog_query(
        &self,
        query: &CatalogSearchQuery,
    ) -> Result<(Vec<SnapshotItem>, RouteUse), TypedError> {
        let provider_hint = query.provider_id.as_str();
        let dispatcher = self
            .dispatcher_handle()
            .map_err(|error| service_error(error, Some(provider_hint)))?;
        let route_use = dispatcher.catalogue_route_use().map_err(|error| {
            service_error(
                ApplicationServiceError::Provider(error),
                Some(provider_hint),
            )
        })?;
        let fixture = self
            .offline_stac_fixture
            .as_deref()
            .filter(|admission| admission.targets(query));
        let use_direct_cdse_public_odata = fixture.is_none()
            && matches!(
                self.business_factory.as_ref(),
                Some(BusinessRuntimeFactory::Production { .. })
            )
            && matches!(
                query.logical_dataset.as_str(),
                "optical.sentinel-2.l1c" | "optical.sentinel-3.standard"
            )
            && query.provider_id == "copernicus.dataspace.odata";

        let request_id = format!("retry-fallback-{}", Uuid::new_v4());
        let (response, effective_query) = if use_direct_cdse_public_odata {
            self.apply_pending_network_policy();
            let network_policy = self
                .cdse_session_runtime()
                .map_err(|error| service_error(error, Some(provider_hint)))?
                .network_policy;
            let client =
                insar_provider::cdse::catalog_client::CdseODataCatalogClient::with_network_policy(
                    Duration::from_secs(45),
                    network_policy,
                )
                .map_err(|error| {
                    service_error(
                        ApplicationServiceError::Provider(error),
                        Some(provider_hint),
                    )
                })?;
            let response = client.search_all(query, &|| false).map_err(|error| {
                service_error(
                    ApplicationServiceError::Provider(error),
                    Some(provider_hint),
                )
            })?;
            (response, query.clone())
        } else {
            let operation = dispatcher
                .search_catalog_for_application(&request_id, query)
                .map_err(|error| {
                    service_error(
                        ApplicationServiceError::Provider(error),
                        Some(provider_hint),
                    )
                })?;
            if operation.query.provider_id != query.provider_id {
                return Err(unsupported());
            }
            (operation.response, operation.query)
        };

        if let Some(admission) = fixture {
            admission
                .validate_response(&effective_query, &response, route_use)
                .map_err(|error| {
                    service_error(
                        ApplicationServiceError::Provider(error),
                        Some(provider_hint),
                    )
                })?;
        } else {
            crate::validate_complete_catalogue_response(
                &response,
                &effective_query,
                route_use,
                self.catalogue_live_acceptance_candidate,
            )
            .map_err(|error| service_error(error, Some(provider_hint)))?;
        }

        let mut items = Vec::with_capacity(response.products.len());
        for product in response.products {
            let mapped = match fixture {
                Some(admission) => crate::stac_fixture::map_product(admission, product, route_use)
                    .map_err(|error| service_error(error, Some(provider_hint)))?,
                None => crate::map_catalogue_product(product, &effective_query, route_use)
                    .map_err(|error| service_error(error, Some(provider_hint)))?,
            };
            items.push(mapped);
        }
        Ok((items, route_use))
    }

    fn refresh_catalog_batch(
        &self,
        task: &FrozenTaskSnapshot,
        task_updated_at: &str,
        task_revision: u64,
        query: &CatalogSearchQuery,
    ) -> Result<FrozenSearchBatch, TypedError> {
        let (candidates, _) = self.execute_live_catalog_query(query)?;
        if candidates.is_empty() {
            return Err(typed(
                "RETRY_FALLBACK_NO_PRODUCTS",
                "重检结果中没有原任务的任何产品",
                false,
            ));
        }
        let batch = FrozenSearchBatch {
            schema_version: 1,
            batch_id: format!("{RETRY_FALLBACK_BATCH_PREFIX}{}", Uuid::new_v4()),
            provider_id: task.provider_id.clone(),
            created_at: now_after_rfc3339(task_updated_at),
            aoi_name: task.aoi_name.clone(),
            aoi_geojson: task.aoi_geojson.clone(),
            query: task.query.clone(),
            search_stats: None,
            candidates,
        };
        self.freeze_fallback_batch(task, task_revision, batch)
    }

    fn refresh_derived_optical_batch(
        &self,
        task: &FrozenTaskSnapshot,
        task_updated_at: &str,
        task_revision: u64,
        source_batch: &FrozenSearchBatch,
        source_query: &CatalogSearchQuery,
        delivery_kind: &str,
    ) -> Result<FrozenSearchBatch, TypedError> {
        let provider_hint = task.provider_id.as_str();
        let (fresh_source_candidates, route_use) = self.execute_live_catalog_query(source_query)?;
        let refreshed_source_batch = FrozenSearchBatch {
            schema_version: source_batch.schema_version,
            batch_id: source_batch.batch_id.clone(),
            provider_id: source_batch.provider_id.clone(),
            created_at: now_after_rfc3339(task_updated_at),
            aoi_name: source_batch.aoi_name.clone(),
            aoi_geojson: source_batch.aoi_geojson.clone(),
            query: source_batch.query.clone(),
            search_stats: None,
            candidates: fresh_source_candidates,
        };
        let cancelled = AtomicBool::new(false);
        let groups = collect_derived_asset_groups(task, source_batch);
        let mut refreshed_items = Vec::new();

        match delivery_kind {
            "landsat_c2_l2_asset_batch"
            | "landsat_c2_l2_complete_asset_set"
            | "landsat_c2_l2_asset_selection" => {
                for (source_id, fallback_display, asset_keys) in groups {
                    let Some(source_item) = find_refreshed_source_item(
                        source_batch,
                        &refreshed_source_batch.candidates,
                        &source_id,
                        fallback_display.as_deref(),
                    ) else {
                        continue;
                    };
                    let evidence =
                        frozen_landsat_scene_evidence(&refreshed_source_batch, 100, source_item)
                            .map_err(|error| service_error(error, Some(provider_hint)))?;
                    let prepared = self
                        .prepare_landsat_asset_items(
                            &evidence,
                            source_item,
                            &asset_keys,
                            &cancelled,
                        )
                        .map_err(|error| service_error(error, Some(provider_hint)))?;
                    refreshed_items.extend(prepared.items);
                }
            }
            "landsat_c2_l2_bundle_batch"
            | "landsat_c2_l2_full_bundle"
            | "landsat_c2_l2_matched_full_bundle" => {
                for (source_id, fallback_display, _) in groups {
                    let Some(source_item) = find_refreshed_source_item(
                        source_batch,
                        &refreshed_source_batch.candidates,
                        &source_id,
                        fallback_display.as_deref(),
                    ) else {
                        continue;
                    };
                    let evidence = frozen_landsat_archive_source_evidence(
                        &refreshed_source_batch,
                        100,
                        source_item,
                    )
                    .map_err(|error| service_error(error, Some(provider_hint)))?;
                    let (prepared, selected_id) = self
                        .prepare_landsat_archive_source(&evidence, &source_item.item_id, &cancelled)
                        .map_err(|error| service_error(error, Some(provider_hint)))?;
                    if let Some(mut archive) = prepared
                        .candidates
                        .into_iter()
                        .find(|candidate| candidate.item_id == selected_id)
                    {
                        if delivery_kind == "landsat_c2_l2_bundle_batch" {
                            archive.metadata.insert(
                                "source_item_id".to_owned(),
                                Value::String(source_item.item_id.clone()),
                            );
                        }
                        refreshed_items.push(archive);
                    }
                }
            }
            "hls_multiband_batch" => {
                for (source_id, fallback_display, asset_keys) in groups {
                    let Some(source_item) = find_refreshed_source_item(
                        source_batch,
                        &refreshed_source_batch.candidates,
                        &source_id,
                        fallback_display.as_deref(),
                    ) else {
                        continue;
                    };
                    let prepared = crate::hls_catalogue::prepare_hls_band_items(
                        source_item,
                        &asset_keys,
                        &cancelled,
                    )
                    .map_err(|error| service_error(error, Some(provider_hint)))?;
                    refreshed_items.extend(prepared);
                }
            }
            "sentinel2_l2a_analysis_band_batch" | "sentinel2_l2a_analysis_band_set" => {
                self.apply_pending_network_policy();
                let network_policy = self
                    .cdse_session_runtime()
                    .map_err(|error| service_error(error, Some(provider_hint)))?
                    .network_policy;
                let evidence_port =
                    Sentinel2BandEvidenceRuntime::new(network_policy).map_err(|error| {
                        service_error(
                            ApplicationServiceError::Provider(error),
                            Some(provider_hint),
                        )
                    })?;
                for (source_id, fallback_display, asset_keys) in groups {
                    let Some(source_item) = find_refreshed_source_item(
                        source_batch,
                        &refreshed_source_batch.candidates,
                        &source_id,
                        fallback_display.as_deref(),
                    ) else {
                        continue;
                    };
                    let source_identity =
                        frozen_sentinel2_analysis_source(&refreshed_source_batch, source_item)
                            .map_err(|error| service_error(error, Some(provider_hint)))?;
                    let (directory, mut items) =
                        crate::sentinel2_band_delivery::prepare_sentinel2_band_items(
                            source_item,
                            &source_identity,
                            &asset_keys,
                            &evidence_port,
                            &cancelled,
                        )
                        .map_err(|error| service_error(error, Some(provider_hint)))?;
                    if delivery_kind == "sentinel2_l2a_analysis_band_batch" {
                        for asset in &mut items {
                            asset.metadata.insert(
                                "source_item_id".to_owned(),
                                Value::String(source_item.item_id.clone()),
                            );
                            asset.metadata.insert(
                                "delivery_product_directory".to_owned(),
                                Value::String(directory.clone()),
                            );
                        }
                    }
                    refreshed_items.extend(items);
                }
            }
            "sentinel2_l1c_band_batch" => {
                self.apply_pending_network_policy();
                let network_policy = self
                    .cdse_session_runtime()
                    .map_err(|error| service_error(error, Some(provider_hint)))?
                    .network_policy;
                for (source_id, fallback_display, asset_keys) in groups {
                    let Some(source_item) = find_refreshed_source_item(
                        source_batch,
                        &refreshed_source_batch.candidates,
                        &source_id,
                        fallback_display.as_deref(),
                    ) else {
                        continue;
                    };
                    let (_, items) =
                        crate::sentinel2_band_delivery::prepare_sentinel2_l1c_band_items(
                            source_item,
                            &asset_keys,
                            network_policy,
                            &cancelled,
                        )
                        .map_err(|error| service_error(error, Some(provider_hint)))?;
                    refreshed_items.extend(items);
                }
            }
            "sentinel2_cdse_safe_archive_batch" | "sentinel2_cdse_safe_archive" => {
                if source_batch.provider_id == "copernicus.dataspace.odata" {
                    for (source_id, fallback_display, _) in groups {
                        if let Some(fresh_item) = find_refreshed_source_item(
                            source_batch,
                            &refreshed_source_batch.candidates,
                            &source_id,
                            fallback_display.as_deref(),
                        ) {
                            refreshed_items.push(fresh_item.clone());
                        }
                    }
                } else {
                    let dispatcher = self
                        .dispatcher_handle()
                        .map_err(|error| service_error(error, Some(provider_hint)))?;
                    let cancelled_arc = Arc::new(AtomicBool::new(false));
                    let request_prefix =
                        format!("rf-{}", &Uuid::new_v4().simple().to_string()[..12]);
                    let resolved_inputs = groups
                        .iter()
                        .enumerate()
                        .filter_map(|(index, (source_id, fallback_display, _))| {
                            let fresh_source = find_refreshed_source_item(
                                source_batch,
                                &refreshed_source_batch.candidates,
                                source_id,
                                fallback_display.as_deref(),
                            )?;
                            Some((index, source_id.clone(), fresh_source.clone()))
                        })
                        .collect::<Vec<_>>();
                    for chunk in resolved_inputs.chunks(SENTINEL2_ARCHIVE_BATCH_LOOKUP_CONCURRENCY)
                    {
                        let outcomes = std::thread::scope(|scope| {
                            let mut handles = Vec::with_capacity(chunk.len());
                            for (index, source_id, fresh_source) in chunk {
                                let context = Sentinel2ArchiveBatchLookupContext {
                                    dispatcher: Arc::clone(&dispatcher),
                                    route_use,
                                    live_acceptance_candidate: self
                                        .catalogue_live_acceptance_candidate,
                                    source_query: source_query.clone(),
                                    lookup_request_id: format!("{request_prefix}-{index}"),
                                    cancelled: Arc::clone(&cancelled_arc),
                                };
                                let source_id = source_id.clone();
                                let fresh_source = fresh_source.clone();
                                handles.push(scope.spawn(move || {
                                    Self::resolve_sentinel2_archive_batch_item(
                                        context,
                                        source_id,
                                        Some(fresh_source),
                                    )
                                }));
                            }
                            let mut results = Vec::with_capacity(handles.len());
                            for handle in handles {
                                let joined = handle
                                    .join()
                                    .map_err(|_| ApplicationServiceError::StatePoisoned)??;
                                results.push(joined);
                            }
                            Ok::<_, ApplicationServiceError>(results)
                        })
                        .map_err(|error| service_error(error, Some(provider_hint)))?;
                        for (source_id, outcome) in outcomes {
                            if let Sentinel2ArchiveAdmissionOutcome::Admitted(mut delivery_item) =
                                outcome
                            {
                                if delivery_kind == "sentinel2_cdse_safe_archive_batch" {
                                    delivery_item.metadata.insert(
                                        "source_item_id".to_owned(),
                                        Value::String(source_id),
                                    );
                                }
                                refreshed_items.push(*delivery_item);
                            }
                        }
                    }
                }
            }
            "planetary_computer_reviewed_cog_assets" => {
                self.apply_pending_network_policy();
                let network_policy = self
                    .cdse_session_runtime()
                    .map_err(|error| service_error(error, Some(provider_hint)))?
                    .network_policy;
                let evidence_port =
                    PlanetaryComputerAssetSigner::new(network_policy).map_err(|error| {
                        service_error(
                            ApplicationServiceError::Provider(error),
                            Some(provider_hint),
                        )
                    })?;
                for (source_id, fallback_display, asset_keys) in groups {
                    let Some(source_item) = find_refreshed_source_item(
                        source_batch,
                        &refreshed_source_batch.candidates,
                        &source_id,
                        fallback_display.as_deref(),
                    ) else {
                        continue;
                    };
                    let source = frozen_reviewed_mpc_source_evidence(
                        &refreshed_source_batch,
                        100,
                        source_item,
                    )
                    .map_err(|error| service_error(error, Some(provider_hint)))?;
                    if !reviewed_mpc_asset_keys_allowed(&source, &asset_keys) {
                        return Err(unsupported());
                    }
                    for asset_key in asset_keys {
                        let resolved = evidence_port
                            .resolve_reviewed_asset(
                                source.class,
                                &source.provider_product_id,
                                &asset_key,
                                &|| false,
                            )
                            .map_err(|error| {
                                service_error(
                                    ApplicationServiceError::Provider(error),
                                    Some(provider_hint),
                                )
                            })?;
                        let frozen_evidence = PlanetaryComputerReviewedAssetEvidence {
                            file_name: resolved.file_name,
                            expected_size_bytes: resolved.expected_size_bytes,
                        };
                        let item = build_planetary_computer_reviewed_cog_item(
                            &source.source_item,
                            source.class,
                            &asset_key,
                            &resolved.stable_href,
                            &frozen_evidence,
                        )
                        .map_err(|_| {
                            service_error(
                                ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid,
                                Some(provider_hint),
                            )
                        })?;
                        refreshed_items.push(item);
                    }
                }
            }
            _ => return Err(unsupported()),
        }

        if refreshed_items.is_empty() {
            return Err(typed(
                "RETRY_FALLBACK_NO_PRODUCTS",
                "重检结果中没有原任务的任何产品",
                false,
            ));
        }

        let batch = FrozenSearchBatch {
            schema_version: 1,
            batch_id: format!("{RETRY_FALLBACK_BATCH_PREFIX}{}", Uuid::new_v4()),
            provider_id: task.provider_id.clone(),
            created_at: now_after_rfc3339(task_updated_at),
            aoi_name: task.aoi_name.clone(),
            aoi_geojson: task.aoi_geojson.clone(),
            query: task.query.clone(),
            search_stats: None,
            candidates: refreshed_items,
        };
        self.freeze_fallback_batch(task, task_revision, batch)
    }
}

#[cfg(test)]
mod eligibility_tests {
    use super::classify_replay_plan_internal;
    use crate::{ApplicationService, ApplicationServiceConfig};
    use insar_core::{
        FrozenSearchBatch, FrozenTaskSnapshot, SearchWorkbenchState, StoreSearchBatchRequest,
    };
    use serde_json::json;

    #[test]
    fn eligibility_distinguishes_ineligible_tasks_from_store_errors() {
        let root = std::env::temp_dir().join(format!(
            "insar-retry-elig-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db_path = root.join("tasks.sqlite3");
        {
            let mut store = insar_store::TaskStore::open(&db_path).unwrap();
            let batch_id = "batch-explicit-1";
            store
                .save_search_batch(&StoreSearchBatchRequest {
                    batch: FrozenSearchBatch {
                        schema_version: 1,
                        batch_id: batch_id.to_owned(),
                        provider_id: "asf.catalog".to_owned(),
                        created_at: "2026-11-22T00:00:00Z".to_owned(),
                        aoi_name: "Hefei".to_owned(),
                        aoi_geojson: Some(
                            json!({
                                "type": "Polygon",
                                "coordinates": [[[117.0, 31.5], [117.5, 31.5], [117.5, 32.0], [117.0, 32.0], [117.0, 31.5]]]
                            })
                            .as_object()
                            .unwrap()
                            .clone(),
                        ),
                        query: json!({"dataset": "SENTINEL-1"}).as_object().unwrap().clone(),
                        search_stats: None,
                        candidates: Vec::new(),
                    },
                    workbench: SearchWorkbenchState {
                        batch_id: batch_id.to_owned(),
                        checked_item_ids: Vec::new(),
                        page_index: 0,
                        page_size: 20,
                        revision: 0,
                    },
                    activate_workbench: true,
                })
                .unwrap();

            let derived_snapshot = FrozenTaskSnapshot {
                schema_version: 1,
                task_id: "task-derived-1".to_owned(),
                batch_id: batch_id.to_owned(),
                provider_id: "microsoft.planetary-computer".to_owned(),
                created_at: "2026-11-22T00:00:00Z".to_owned(),
                output_root: root.join("out"),
                layout: insar_core::DownloadLayout::Direct,
                aoi_name: "Hefei".to_owned(),
                aoi_geojson: None,
                query: json!({
                    "derived_from_batch_id": "missing-or-unreadable-batch",
                    "delivery_kind": "sentinel2_l2a_analysis_band_batch"
                })
                .as_object()
                .unwrap()
                .clone(),
                candidates: Vec::new(),
                initial_selected_item_ids: Vec::new(),
                operation: None,
                orbit_association: None,
            };
            assert!(
                classify_replay_plan_internal(&derived_snapshot, true, &store)
                    .unwrap()
                    .is_none(),
                "explicit task must be ineligible Ok(None)"
            );
            assert!(
                classify_replay_plan_internal(&derived_snapshot, false, &store)
                    .unwrap()
                    .is_none(),
                "missing source batch must stay Ok(None)"
            );
        }

        let service = ApplicationService::new(ApplicationServiceConfig::business_store(&db_path));
        service.startup().unwrap();

        // Verify direct StoreError propagation from classify_replay_plan_internal
        {
            let store = insar_store::TaskStore::open(&db_path).unwrap();
            let broken_conn = rusqlite::Connection::open(&db_path).unwrap();
            broken_conn
                .execute_batch("PRAGMA foreign_keys = OFF; DROP TABLE search_batches;")
                .unwrap();
            drop(broken_conn);
            let err_snapshot = FrozenTaskSnapshot {
                schema_version: 1,
                task_id: "task-derived-err".to_owned(),
                batch_id: "batch-explicit-1".to_owned(),
                provider_id: "microsoft.planetary-computer".to_owned(),
                created_at: "2026-11-22T00:00:00Z".to_owned(),
                output_root: root.join("out"),
                layout: insar_core::DownloadLayout::Direct,
                aoi_name: "Hefei".to_owned(),
                aoi_geojson: None,
                query: json!({
                    "derived_from_batch_id": "any-batch",
                    "delivery_kind": "sentinel2_l2a_analysis_band_batch"
                })
                .as_object()
                .unwrap()
                .clone(),
                candidates: Vec::new(),
                initial_selected_item_ids: Vec::new(),
                operation: None,
                orbit_association: None,
            };
            let db_err = classify_replay_plan_internal(&err_snapshot, false, &store)
                .err()
                .expect("broken search_batches table must propagate StoreError");
            assert!(!format!("{db_err}").is_empty());
        }

        // Propagate DB corruption error from task_retry_fallback_eligibility
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF; DROP TABLE task_tombstones;")
            .unwrap();
        drop(conn);
        let err = service
            .task_retry_fallback_eligibility(&["any-task".to_owned()])
            .expect_err(
                "SQLite schema error must propagate instead of silently returning Ok(false)",
            );
        assert!(!format!("{err}").is_empty());

        let _ = service.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }
}
