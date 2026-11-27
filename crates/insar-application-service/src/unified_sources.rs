use std::collections::{BTreeSet, HashMap};

use insar_core::{
    AccountId, CredentialRequirement, FrozenSearchBatch, ResourceDescriptor, ResourceRegistry,
    SearchFilterSchemaCatalog, SourceAvailability, SourceDescriptor, SourceFamily,
    SourceNetworkDefault, SourceSearchMode, StaticCapabilityLifecycle,
};

use insar_store::{StoreBatchRouteHeader, StoreSourceView};

use crate::{ApplicationService, ApplicationServiceError};

fn active(lifecycle: &StaticCapabilityLifecycle) -> bool {
    !matches!(
        lifecycle,
        StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
    )
}

/// ASF removed the legacy missions (ERS, RADARSAT-1, JERS-1) from its
/// searchable interfaces (Owner evidence 2026-10-31), so a retired
/// lifecycle for those resources must say why instead of the generic
/// not-yet-open copy.
pub(crate) const ASF_RETIRED_REASON_ZH: &str = "ASF 已从检索接口下线该数据集";

fn availability_reason_zh(
    resource_id: &str,
    lifecycle: &StaticCapabilityLifecycle,
) -> Option<String> {
    match lifecycle {
        StaticCapabilityLifecycle::Retired if asf_retired_legacy_resource(resource_id) => {
            Some(ASF_RETIRED_REASON_ZH.to_owned())
        }
        StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired => {
            Some("该数据源尚未开放".to_owned())
        }
        _ => None,
    }
}

/// The user-facing Chinese reason a search request against an unavailable
/// (reserved or retired) resource is rejected with. Shared by the unified
/// search admission so the dropdown hint and the typed rejection agree.
pub(crate) fn availability_reason_for_resource(
    resource_id: &str,
    lifecycle: &StaticCapabilityLifecycle,
) -> Option<String> {
    availability_reason_zh(resource_id, lifecycle)
}

fn asf_retired_legacy_resource(resource_id: &str) -> bool {
    matches!(
        resource_id,
        "science.ers-1-2.sar.v1" | "science.jers-1.sar.v1" | "science.radarsat-1.sar.v1"
    )
}

fn account_for_provider(provider: &str) -> Option<AccountId> {
    match provider {
        "asf.nasa.earthdata" | "asf.catalog" => Some(AccountId::Earthdata),
        "nisar-dem.asf.earthdata"
        | "opera.lpdaac.earthdata"
        | "nasa.laads.earthdata"
        | "nasa.lpdaac.earthdata" => Some(AccountId::Earthdata),
        "copernicus.dataspace.odata" => Some(AccountId::Cdse),
        "usgs.eros.m2m" => Some(AccountId::UsgsM2m),
        "microsoft.planetary-computer" => Some(AccountId::Mpc),
        "isro.nrsc.bhoonidhi" => Some(AccountId::Bhoonidhi),
        "opentopography" => Some(AccountId::Opentopography),
        "gacos.webform" => Some(AccountId::Gacos),
        _ => None,
    }
}

fn account_rank(account: &AccountId) -> u8 {
    match account {
        AccountId::Cdse => 0,
        AccountId::Earthdata => 1,
        AccountId::UsgsM2m => 2,
        AccountId::Bhoonidhi => 3,
        AccountId::Mpc => 4,
        AccountId::Opentopography => 5,
        AccountId::Gacos => 6,
    }
}

pub(crate) fn project_sources() -> Result<Vec<SourceDescriptor>, String> {
    let registry = ResourceRegistry::embedded_ref()?;
    let schemas = SearchFilterSchemaCatalog::embedded_ref()?;
    let mut sources = Vec::with_capacity(registry.resources.len());
    for resource in &registry.resources {
        let common = resource.common();
        let (family, search_mode, schema) = match resource {
            ResourceDescriptor::ScienceDataset { science, .. } => {
                let is_sar = science.sar_search.is_some() || science.mission == "ENVISAT";
                let schema = science
                    .search_schema_id
                    .as_deref()
                    .and_then(|id| schemas.schema(id));
                // Optical schemas are reviewed here; SAR schemas are described by
                // the Registry's separate SAR selector/filter projection.
                if !is_sar && science.search_schema_id.is_some() && schema.is_none() {
                    return Err(format!(
                        "unknown source filter schema for {}",
                        common.resource_id
                    ));
                }
                (
                    if is_sar {
                        SourceFamily::Sar
                    } else {
                        SourceFamily::Optical
                    },
                    if is_sar {
                        SourceSearchMode::Sar
                    } else {
                        SourceSearchMode::CatalogueIntent
                    },
                    schema,
                )
            }
            ResourceDescriptor::Orbit { .. } => {
                (SourceFamily::Orbit, SourceSearchMode::Auxiliary, None)
            }
            ResourceDescriptor::Dem { .. } => {
                (SourceFamily::Dem, SourceSearchMode::Auxiliary, None)
            }
            ResourceDescriptor::Atmospheric { .. } => {
                (SourceFamily::Atmospheric, SourceSearchMode::Auxiliary, None)
            }
        };
        let mut accounts = Vec::new();
        let mut seen_routes = BTreeSet::new();
        let mut bhoonidhi = false;
        for capability in common.capabilities.iter().filter(|c| active(&c.lifecycle)) {
            for route_id in &capability.provider_route_ids {
                if !seen_routes.insert(route_id.as_str()) {
                    continue;
                }
                let route = common
                    .provider_routes
                    .iter()
                    .find(|route| &route.route_id == route_id)
                    .ok_or_else(|| format!("unknown source route {}", route_id))?;
                if !active(&route.lifecycle) {
                    continue;
                }
                if route.provider_id == "isro.nrsc.bhoonidhi" {
                    bhoonidhi = true;
                }
                if route.credential_requirement == CredentialRequirement::Required {
                    let account = account_for_provider(&route.provider_id).ok_or_else(|| {
                        format!("unmapped credential provider {}", route.provider_id)
                    })?;
                    if !accounts.contains(&account) {
                        accounts.push(account);
                    }
                }
            }
        }
        accounts.sort_by_key(account_rank);
        let unavailable = !active(&common.lifecycle);
        let availability_reason_zh = availability_reason_zh(&common.resource_id, &common.lifecycle);
        sources.push(SourceDescriptor {
            resource_id: common.resource_id.clone(),
            group_id: if common.resource_id.starts_with("science.nisar.") {
                "nisar".to_owned()
            } else {
                common.resource_id.clone()
            },
            display_name_zh: common.display_name.clone(),
            family,
            search_mode,
            filter_schema_id: schema.map(|s| s.schema_id.clone()),
            filter_schema_revision: schema.map(|s| u64::from(s.schema_revision)),
            // #22 B slice 3(c): the same default target search_start uses.
            result_cap: crate::unified_search::unified_default_target(registry, resource),
            accounts,
            network_default: if bhoonidhi {
                SourceNetworkDefault::SystemProxy
            } else {
                SourceNetworkDefault::Direct
            },
            availability: if unavailable {
                SourceAvailability::Unavailable
            } else {
                SourceAvailability::Available
            },
            availability_reason_zh,
        });
    }
    sources.sort_by(|a, b| a.resource_id.cmp(&b.resource_id));
    Ok(sources)
}

fn source_for_route(
    provider_id: &str,
    logical_dataset: Option<&str>,
    native_collection_id: Option<&str>,
    dataset: Option<&str>,
) -> Option<String> {
    let registry = ResourceRegistry::embedded_ref().ok()?;
    if let Some(logical_dataset) = logical_dataset {
        return registry
            .catalogue_search_resource_for_source(
                logical_dataset,
                provider_id,
                native_collection_id?,
            )
            .ok()
            .flatten()
            .map(|resource| resource.common().resource_id.clone())
            .or_else(|| {
                let native = native_collection_id?;
                registry.resources.iter().find_map(|resource| {
                    let ResourceDescriptor::ScienceDataset { common, science } = resource else {
                        return None;
                    };
                    let sar = science.sar_search.as_ref()?;
                    if sar.logical_dataset.as_deref() != Some(logical_dataset) {
                        return None;
                    }
                    let route = common
                        .provider_routes
                        .iter()
                        .find(|route| route.route_id == sar.provider_route_id)?;
                    (route.provider_id == provider_id
                        && route.native_collection_id.as_deref() == Some(native)
                        && active(&route.lifecycle))
                    .then(|| common.resource_id.clone())
                })
            });
    }
    registry
        .resource_for_provider_selector(provider_id, dataset?)
        .map(|resource| resource.common().resource_id.clone())
}

pub(crate) fn source_for_batch(batch: &FrozenSearchBatch) -> Option<String> {
    let text = |key: &str| batch.query.get(key).and_then(serde_json::Value::as_str);
    source_for_route(
        &batch.provider_id,
        text("logical_dataset"),
        text("native_collection_id"),
        text("dataset"),
    )
}

fn source_for_header(header: &StoreBatchRouteHeader) -> Option<String> {
    source_for_route(
        &header.provider_id,
        header.logical_dataset.as_deref(),
        header.native_collection_id.as_deref(),
        header.dataset.as_deref(),
    )
}

fn newer(header: &StoreBatchRouteHeader, view: Option<&StoreSourceView>) -> bool {
    match view.and_then(|view| Some((view.searched_at.as_deref()?, view.batch_id.as_deref()?))) {
        None => true,
        Some((searched_at, batch_id)) => {
            (header.created_at.as_str(), header.batch_id.as_str()) > (searched_at, batch_id)
        }
    }
}

pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl ApplicationService {
    /// Read-only Registry + schema projection. No Store, network or credentials are opened.
    pub fn list_sources(&self) -> Result<Vec<SourceDescriptor>, ApplicationServiceError> {
        project_sources().map_err(|reason| ApplicationServiceError::InvalidRequest { reason })
    }

    /// Persisted per-source views (schema 33). A newer frozen batch for a
    /// source, from the legacy search entry or a crash between publication and
    /// the view update, advances the pointer; it never moves backward. Pending
    /// requests that are not alive in this process (restart) are cleared.
    pub fn source_views(&self) -> Result<Vec<insar_core::SourceView>, ApplicationServiceError> {
        use insar_core::SourceView;
        let sources = self.list_sources()?;
        let store = self.store_handle()?;
        let now = now_rfc3339();
        let (headers, mut rows) = {
            let store = store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            (
                store.latest_frozen_batch_headers_per_route()?,
                store
                    .load_source_views()?
                    .into_iter()
                    .map(|view| (view.resource_id.clone(), view))
                    .collect::<HashMap<_, _>>(),
            )
        };
        let mut latest = HashMap::<String, &StoreBatchRouteHeader>::new();
        for header in &headers {
            let Some(resource_id) = source_for_header(header) else {
                continue;
            };
            let entry = latest.entry(resource_id).or_insert(header);
            if (header.created_at.as_str(), header.batch_id.as_str())
                > (entry.created_at.as_str(), entry.batch_id.as_str())
            {
                *entry = header;
            }
        }
        let live = {
            let mut progress = self
                .unified_searches
                .progress
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            progress.prune(std::time::Instant::now());
            progress
                .values()
                .filter(|item| !item.terminal)
                .map(|item| (item.request_id.clone(), item.resource_id.clone()))
                .collect::<Vec<_>>()
        };
        let mut repairs = Vec::new();
        for (resource_id, header) in &latest {
            if newer(header, rows.get(resource_id)) {
                let row = rows
                    .entry(resource_id.clone())
                    .or_insert_with(|| StoreSourceView {
                        resource_id: resource_id.clone(),
                        batch_id: None,
                        item_count: 0,
                        searched_at: None,
                        pending_request_id: None,
                        last_error: None,
                        updated_at: now.clone(),
                    });
                row.batch_id = Some(header.batch_id.clone());
                row.searched_at = Some(header.created_at.clone());
                row.item_count = header.item_count;
                repairs.push((resource_id.clone(), Some(header.batch_id.clone()), None));
            }
        }
        for row in rows.values_mut() {
            if let Some(request_id) = row.pending_request_id.clone() {
                if !live.iter().any(|(id, _)| id == &request_id)
                    && !self.unified_searches.has_active_continuation(&request_id)
                {
                    row.pending_request_id = None;
                    repairs.push((row.resource_id.clone(), None, Some(request_id)));
                }
            }
        }
        if !repairs.is_empty() {
            // Best effort: the projection above is already correct for this read.
            match store.lock() {
                Ok(mut store) => {
                    for (resource_id, batch_id, request_id) in repairs {
                        let repair_result = match (batch_id, request_id) {
                            (Some(batch_id), _) => store.record_source_view_success(
                                &resource_id,
                                &batch_id,
                                None,
                                &now,
                            ),
                            (None, Some(request_id)) => {
                                store.clear_source_view_pending(&resource_id, &request_id, &now)
                            }
                            (None, None) => Ok(()),
                        };
                        if let Err(err) = repair_result {
                            eprintln!(
                                "[unified_sources] best-effort source view repair failed: code={}",
                                err.code()
                            );
                        }
                    }
                }
                Err(_) => {
                    eprintln!("[unified_sources] best-effort source view repair skipped: store lock poisoned");
                }
            }
        }
        Ok(sources
            .into_iter()
            .map(|source| {
                let row = rows.get(&source.resource_id);
                let running = row
                    .and_then(|row| row.pending_request_id.clone())
                    .or_else(|| {
                        live.iter()
                            .find(|(_, resource_id)| resource_id == &source.resource_id)
                            .map(|(request_id, _)| request_id.clone())
                    });
                SourceView {
                    batch_id: row.and_then(|row| row.batch_id.clone()),
                    revision: row.and_then(|row| row.batch_id.as_ref()).map(|_| 0),
                    item_count: row.map_or(0, |row| row.item_count),
                    searched_at: row.and_then(|row| row.searched_at.clone()),
                    pending_request_id: running,
                    last_error: row.and_then(|row| row.last_error.clone()),
                    resource_id: source.resource_id,
                }
            })
            .collect())
    }

    /// Persist a source-view transition from the search worker. Failure to
    /// record the view never fails the search itself.
    pub(crate) fn update_source_view(
        &self,
        update: impl FnOnce(&mut insar_store::TaskStore, &str) -> Result<(), insar_store::StoreError>,
    ) {
        match self.store_handle() {
            Ok(store) => match store.lock() {
                Ok(mut store) => {
                    if let Err(err) = update(&mut store, &now_rfc3339()) {
                        eprintln!(
                            "[unified_sources] best-effort update_source_view failed: code={}",
                            err.code()
                        );
                    }
                }
                Err(_) => {
                    eprintln!("[unified_sources] best-effort update_source_view skipped: store lock poisoned");
                }
            },
            Err(err) => {
                eprintln!(
                    "[unified_sources] best-effort update_source_view skipped: code={}",
                    err.code()
                );
            }
        }
    }
}
