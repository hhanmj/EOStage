//! Agent tool adapter over the one desktop ApplicationService. It shares the
//! service's Store lease and scheduler; it never opens a Store of its own.
use std::sync::Arc;

use insar_agent::{
    executor::ErrorClass,
    host::UnifiedApplication,
    session::{HostError, StoreAccess},
};
use insar_core::{
    AgentWorkingAoiSetInput, StoreTaskCommand, StoreTaskPageRequest, StoreUpdateTaskRequest,
    TaskStateCommandKind, TypedError, UnifiedSearchRequest,
};
use insar_store::{StoreTaskEventPageRequest, StoreWorkbenchPageRequest, TaskStore};
use serde_json::{json, Map, Value};

use crate::{
    AdminBoundaryResolveRequest, AdminBoundaryResult, AdminBoundarySearchRequest,
    ApplicationService, ApplicationServiceError, ApplicationSetWorkingAoiRequest,
    GeoBoundaryResolveRequest, StoreBatchPageRequest, UnifiedCreateTaskOptions, UnifiedTaskAction,
};

fn code_class(code: &str) -> ErrorClass {
    if code.contains("CREDENTIAL")
        || code.contains("UNAUTHORIZED")
        || code.contains("AUTH")
        || code.contains("SESSION")
    {
        ErrorClass::Authentication
    } else if code.contains("NETWORK") || code.contains("TRANSPORT") || code.contains("TIMEOUT") {
        ErrorClass::Transient
    } else if code.contains("INVALID") || code.contains("NOT_FOUND") {
        ErrorClass::InvalidParameters
    } else if code.contains("UNSUPPORTED") || code.contains("UNAVAILABLE") {
        ErrorClass::Unsupported
    } else {
        ErrorClass::Business
    }
}
fn service_error(error: ApplicationServiceError) -> ErrorClass {
    code_class(error.code())
}
fn typed_error(error: TypedError) -> ErrorClass {
    code_class(&error.code)
}
fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, ErrorClass> {
    serde_json::to_value(value).map_err(|_| ErrorClass::Internal)
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Desktop composition of the Agent `HostPort`. Cloning shares the same service.
#[derive(Clone)]
pub struct AgentApplication(pub Arc<ApplicationService>);

impl AgentApplication {
    /// Application data directory (the Store's parent); Agent sessions and run
    /// records live under `agent/` beside the Store.
    pub fn data_root(&self) -> Option<std::path::PathBuf> {
        self.0
            .config
            .store_path()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf)
    }

    /// Short Store critical section for Agent plan/run rows. Never call back into
    /// the service from `operation`: the service takes the same lock.
    pub fn with_task_store<T>(
        &self,
        operation: impl FnOnce(&mut TaskStore) -> T,
    ) -> Result<T, ApplicationServiceError> {
        let store = self.0.store_handle()?;
        let mut guard = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        Ok(operation(&mut guard))
    }

    fn task_revision(&self, task_id: &str) -> Result<u64, ErrorClass> {
        self.with_task_store(|store| store.load_task_record(task_id))
            .map_err(service_error)?
            .map_err(|_| ErrorClass::InvalidParameters)?
            .map(|task| task.runtime.revision)
            .ok_or(ErrorClass::InvalidParameters)
    }

    fn taiwan_city_for_adcode(code: &str) -> Option<&'static str> {
        match code {
            "710100" => Some("台北市"),
            "710200" => Some("新北市"),
            "710300" => Some("桃园市"),
            "710400" => Some("台中市"),
            "710500" => Some("台南市"),
            "710600" => Some("高雄市"),
            "710700" => Some("基隆市"),
            "710800" => Some("南投县"),
            "710900" => Some("新竹市"),
            "711000" => Some("嘉义市"),
            "711100" => Some("新竹县"),
            "711200" => Some("苗栗县"),
            "711300" => Some("彰化县"),
            "711400" => Some("云林县"),
            "711500" => Some("嘉义县"),
            "711600" => Some("屏东县"),
            "711700" => Some("宜兰县"),
            "711800" => Some("花莲县"),
            "711900" => Some("台东县"),
            "712000" => Some("澎湖县"),
            "712100" => Some("金门县"),
            "712200" => Some("连江县"),
            _ => None,
        }
    }

    fn resolve_admin_boundary_code(
        &self,
        raw_code: &str,
    ) -> Result<AdminBoundaryResult, ErrorClass> {
        let code = raw_code
            .trim()
            .strip_prefix("datav:")
            .unwrap_or(raw_code.trim())
            .trim();
        if code.is_empty() {
            return Err(ErrorClass::InvalidParameters);
        }
        if code.contains('|') {
            return self
                .0
                .geo_boundary_resolve(&GeoBoundaryResolveRequest {
                    provider_object_id: code.to_owned(),
                })
                .map_err(service_error);
        }
        if let Some(nominatim_id) = code.strip_prefix("nominatim:") {
            return self
                .0
                .admin_boundary_resolve(&AdminBoundaryResolveRequest {
                    provider: "nominatim".to_owned(),
                    provider_object_id: nominatim_id.trim().to_owned(),
                })
                .map_err(service_error);
        }
        if code.starts_with("relation:")
            || code.starts_with("way:")
            || code.starts_with("node:")
            || code.starts_with("osm:")
        {
            let clean_id = code.strip_prefix("osm:").unwrap_or(code);
            return self
                .0
                .admin_boundary_resolve(&AdminBoundaryResolveRequest {
                    provider: "nominatim".to_owned(),
                    provider_object_id: clean_id.to_owned(),
                })
                .map_err(service_error);
        }
        let search_req = if code == "710000" {
            AdminBoundarySearchRequest {
                province: "台湾省".to_owned(),
                provider_object_id: Some("710000".to_owned()),
                limit: Some(1),
                ..Default::default()
            }
        } else if let Some(tw_city) = Self::taiwan_city_for_adcode(code) {
            AdminBoundarySearchRequest {
                province: "台湾省".to_owned(),
                city: tw_city.to_owned(),
                provider_object_id: Some(code.to_owned()),
                limit: Some(1),
                ..Default::default()
            }
        } else if code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()) {
            AdminBoundarySearchRequest {
                provider_object_id: Some(code.to_owned()),
                limit: Some(1),
                ..Default::default()
            }
        } else {
            AdminBoundarySearchRequest {
                query: code.to_owned(),
                limit: Some(1),
                ..Default::default()
            }
        };
        let search = self
            .0
            .admin_boundary_search(&search_req)
            .map_err(service_error)?;
        let candidate = search
            .results
            .into_iter()
            .next()
            .ok_or(ErrorClass::InvalidParameters)?;
        if candidate.geometry.is_some() {
            return Ok(candidate);
        }
        if candidate.provider == "nominatim" {
            return self
                .0
                .admin_boundary_resolve(&AdminBoundaryResolveRequest {
                    provider: "nominatim".to_owned(),
                    provider_object_id: candidate.provider_object_id,
                })
                .map_err(service_error);
        }
        Err(ErrorClass::InvalidParameters)
    }

    fn prepare_working_aoi_set(
        &self,
        input: &Value,
    ) -> Result<Option<ApplicationSetWorkingAoiRequest>, ErrorClass> {
        let parsed: AgentWorkingAoiSetInput =
            serde_json::from_value(input.clone()).map_err(|_| ErrorClass::InvalidParameters)?;
        parsed
            .validate()
            .map_err(|_| ErrorClass::InvalidParameters)?;
        if parsed.clear {
            if parsed.admin_code.is_some() || parsed.bbox.is_some() {
                return Err(ErrorClass::InvalidParameters);
            }
            return Ok(None);
        }
        if let Some(admin_code) = parsed
            .admin_code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if parsed.bbox.is_some() {
                return Err(ErrorClass::InvalidParameters);
            }
            let resolved = self.resolve_admin_boundary_code(admin_code)?;
            let geometry = resolved
                .geometry
                .and_then(|g| g.as_object().cloned())
                .ok_or(ErrorClass::InvalidParameters)?;
            let resolved_leaf = {
                let mut s = resolved.name.trim();
                for suffix in [
                    "特别行政区",
                    "维吾尔自治区",
                    "壮族自治区",
                    "回族自治区",
                    "自治区",
                    "自治州",
                    "自治县",
                    "地区",
                    "省",
                    "市",
                    "区",
                    "县",
                    "盟",
                    "旗",
                ] {
                    if let Some(stripped) = s.strip_suffix(suffix) {
                        if stripped.chars().count() >= 2 {
                            s = stripped;
                        }
                        break;
                    }
                }
                s
            };
            let display_name = parsed
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|s| {
                    !s.is_empty()
                        && (resolved_leaf.chars().count() < 2
                            || s.contains(resolved_leaf)
                            || resolved.display_name.contains(*s))
                })
                .map(str::to_owned)
                .unwrap_or(resolved.display_name);
            return Ok(Some(ApplicationSetWorkingAoiRequest {
                source_kind: "administrative_boundary".to_owned(),
                geometry,
                bbox: vec![
                    resolved.bbox.west,
                    resolved.bbox.south,
                    resolved.bbox.east,
                    resolved.bbox.north,
                ],
                crs: "EPSG:4326".to_owned(),
                display_name,
            }));
        }
        if let Some(bbox) = &parsed.bbox {
            let (geometry, norm_bbox) =
                bbox_polygon_geometry(bbox).ok_or(ErrorClass::InvalidParameters)?;
            let display_name = parsed
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    format!(
                        "{:.4}, {:.4}, {:.4}, {:.4}",
                        norm_bbox[0], norm_bbox[1], norm_bbox[2], norm_bbox[3]
                    )
                });
            return Ok(Some(ApplicationSetWorkingAoiRequest {
                source_kind: "manual_bbox".to_owned(),
                geometry,
                bbox: norm_bbox,
                crs: "EPSG:4326".to_owned(),
                display_name,
            }));
        }
        Err(ErrorClass::InvalidParameters)
    }
}

fn bbox_polygon_geometry(bbox: &[f64]) -> Option<(Map<String, Value>, Vec<f64>)> {
    if bbox.len() != 4 || bbox.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let (w, s, e, n) = (bbox[0], bbox[1], bbox[2], bbox[3]);
    if !((-180.0..=180.0).contains(&w)
        && (-180.0..=180.0).contains(&e)
        && w < e
        && (-90.0..=90.0).contains(&s)
        && (-90.0..=90.0).contains(&n)
        && s < n)
    {
        return None;
    }
    let geom = json!({
        "type": "Polygon",
        "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]],
    });
    Some((geom.as_object()?.clone(), vec![w, s, e, n]))
}

fn reorder_s1_polarizations_array(arr: &[Value]) -> Vec<Value> {
    let preferred = ["VV", "VV+VH", "HH", "HH+HV"];
    let existing: Vec<String> = arr
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    if existing.is_empty() {
        return preferred.iter().map(|s| json!(s)).collect();
    }
    let mut out: Vec<Value> = Vec::with_capacity(existing.len());
    for target in preferred {
        if existing
            .iter()
            .any(|item| item.eq_ignore_ascii_case(target))
        {
            out.push(json!(target));
        }
    }
    for item in &existing {
        if !preferred
            .iter()
            .any(|target| item.eq_ignore_ascii_case(target))
        {
            out.push(json!(item));
        }
    }
    out
}

pub(crate) fn normalize_agent_s1_capabilities(capabilities: &mut Value) {
    let is_s1 = |text: &str| {
        let lower = text.to_ascii_lowercase();
        lower.contains("sentinel-1") || lower.contains("sentinel1") || lower.starts_with("s1_")
    };
    if let Some(datasets) = capabilities
        .get_mut("datasets")
        .and_then(Value::as_array_mut)
    {
        for dataset in datasets {
            let matches_s1 = dataset
                .get("dataset")
                .and_then(Value::as_str)
                .is_some_and(is_s1)
                || dataset
                    .get("display_name")
                    .and_then(Value::as_str)
                    .is_some_and(is_s1);
            if !matches_s1 {
                continue;
            }
            if let Some(obj) = dataset.as_object_mut() {
                let reordered = obj
                    .get("polarizations")
                    .and_then(Value::as_array)
                    .map(|arr| reorder_s1_polarizations_array(arr))
                    .unwrap_or_else(|| {
                        vec![json!("VV"), json!("VV+VH"), json!("HH"), json!("HH+HV")]
                    });
                obj.insert("polarizations".to_owned(), Value::Array(reordered));
                obj.insert(
                    "land_default_polarizations".to_owned(),
                    json!(["VV", "VV+VH"]),
                );
                obj.insert(
                    "polar_maritime_only_polarizations".to_owned(),
                    json!(["HH", "HH+HV"]),
                );
            }
        }
    }
    if let Some(sources) = capabilities
        .get_mut("sources")
        .and_then(Value::as_array_mut)
    {
        for source in sources {
            let matches_s1 = source
                .get("selector")
                .and_then(Value::as_str)
                .is_some_and(is_s1)
                || source
                    .get("logical_dataset")
                    .and_then(Value::as_str)
                    .is_some_and(is_s1)
                || source
                    .get("display_name")
                    .and_then(Value::as_str)
                    .is_some_and(is_s1);
            if !matches_s1 {
                continue;
            }
            if let Some(obj) = source.as_object_mut() {
                if let Some(filters) = obj.get_mut("filters").and_then(Value::as_object_mut) {
                    if let Some(pol_val) = filters.get_mut("polarizations") {
                        if let Some(arr) = pol_val.as_array() {
                            *pol_val = Value::Array(reorder_s1_polarizations_array(arr));
                        } else if let Some(pol_obj) = pol_val.as_object_mut() {
                            for key in ["options", "values", "allowed"] {
                                if let Some(arr) = pol_obj.get(key).and_then(Value::as_array) {
                                    let next = reorder_s1_polarizations_array(arr);
                                    pol_obj.insert(key.to_owned(), Value::Array(next));
                                }
                            }
                        }
                    }
                }
                obj.insert(
                    "land_default_polarizations".to_owned(),
                    json!(["VV", "VV+VH"]),
                );
                obj.insert(
                    "polar_maritime_only_polarizations".to_owned(),
                    json!(["HH", "HH+HV"]),
                );
            }
        }
    }
}

pub(crate) fn append_agent_optical_capabilities(capabilities: &mut Value) {
    let Ok(catalog) = insar_core::SearchFilterSchemaCatalog::embedded_ref() else {
        return;
    };
    let Some(root) = capabilities.as_object_mut() else {
        return;
    };
    let optical: Vec<Value> = catalog
        .schemas
        .iter()
        .map(|schema| {
            let mut filter_specs = Vec::new();
            for filter in &schema.filters {
                match filter {
                    insar_core::SearchFilter::MultiEnum {
                        query_key,
                        label,
                        options,
                        default,
                        empty_selection,
                        ..
                    } => {
                        filter_specs.push(json!({
                            "query_key": query_key,
                            "label": label,
                            "type": "array<string>",
                            "options": options.iter().map(|o| &o.value).collect::<Vec<_>>(),
                            "default_when_omitted": match empty_selection {
                                insar_core::EmptySelection::RestoreAll => {
                                    Value::Array(default.iter().map(|d| json!(d)).collect())
                                }
                                insar_core::EmptySelection::Omit => Value::Null,
                            },
                        }));
                    }
                    insar_core::SearchFilter::Number {
                        query_key,
                        label,
                        minimum,
                        maximum,
                        unit,
                        ..
                    } => {
                        filter_specs.push(json!({
                            "query_key": query_key,
                            "label": label,
                            "type": "number",
                            "minimum": minimum,
                            "maximum": maximum,
                            "unit": unit,
                        }));
                    }
                    insar_core::SearchFilter::Fixed {
                        query_key: Some(query_key),
                        label,
                        options,
                        ..
                    } => {
                        filter_specs.push(json!({
                            "query_key": query_key,
                            "label": label,
                            "type": "fixed_string",
                            "default_when_omitted": options
                                .first()
                                .map(|o| json!(o.value))
                                .unwrap_or(Value::Null),
                        }));
                    }
                    insar_core::SearchFilter::Fixed {
                        query_key: None, ..
                    }
                    | insar_core::SearchFilter::DateRange { .. } => {}
                }
            }
            json!({
                "resource_id": schema.resource_id,
                "dataset_label": schema.dataset_label,
                "schema_id": schema.schema_id,
                "filters": filter_specs,
            })
        })
        .collect();
    root.insert("optical_datasets".to_owned(), Value::Array(optical));
}

impl UnifiedApplication for AgentApplication {
    fn application_service_readiness(&self) -> Result<Value, ErrorClass> {
        to_value(&self.0.startup().map_err(service_error)?)
    }
    fn application_provider_capabilities(&self) -> Result<Value, ErrorClass> {
        let mut value = to_value(&self.0.provider_capabilities().map_err(service_error)?)?;
        normalize_agent_s1_capabilities(&mut value);
        append_agent_optical_capabilities(&mut value);
        Ok(value)
    }
    fn application_working_aoi(&self) -> Result<Value, ErrorClass> {
        Ok(match self.0.working_aoi().map_err(service_error)? {
            Some(aoi) => json!({
                "state": aoi.state,
                "source_kind": aoi.source_kind,
                "display_name": aoi.display_name,
                "bbox": aoi.bbox,
                "crs": aoi.crs,
                "geometry": aoi.geometry,
            }),
            None => json!({"state": "none"}),
        })
    }
    fn application_workbench_page(
        &self,
        page_index: u64,
        page_size: u64,
    ) -> Result<Value, ErrorClass> {
        let workbench = self
            .with_task_store(|store| store.core_restore(false))
            .map_err(service_error)?
            .map_err(|_| ErrorClass::Internal)?
            .workbench;
        let Some(workbench) = workbench else {
            return Ok(json!({"workbench": null}));
        };
        let page = self
            .0
            .workbench_page(&StoreWorkbenchPageRequest {
                batch_id: workbench.batch_id,
                expected_revision: workbench.revision,
                page_index,
                page_size,
            })
            .map_err(service_error)?;
        let mut value = to_value(&page.page)?;
        if let Some(map) = value.as_object_mut() {
            map.insert("search_batch_id".into(), json!(page.page.batch_id));
        }
        Ok(value)
    }
    fn application_download_task_page(
        &self,
        task_id: &str,
        page_index: u64,
        page_size: u64,
    ) -> Result<Value, ErrorClass> {
        let expected_revision = self.task_revision(task_id)?;
        to_value(
            &self
                .0
                .task_page(&StoreTaskPageRequest {
                    task_id: task_id.to_owned(),
                    expected_revision,
                    page_index,
                    page_size,
                })
                .map_err(service_error)?,
        )
    }
    fn application_download_task_event_page(
        &self,
        task_id: &str,
        page_size: u64,
    ) -> Result<Value, ErrorClass> {
        let expected_revision = self.task_revision(task_id)?;
        to_value(
            &self
                .0
                .task_event_page(&StoreTaskEventPageRequest {
                    task_id: task_id.to_owned(),
                    expected_revision,
                    before_sequence: None,
                    page_size: u32::try_from(page_size)
                        .map_err(|_| ErrorClass::InvalidParameters)?,
                })
                .map_err(service_error)?,
        )
    }
    fn application_credential_categories(&self) -> Result<Value, ErrorClass> {
        // Reuse account states; the executor projects them to ready/failed.
        let mut categories = serde_json::Map::new();
        for status in self.0.account_session_statuses().map_err(service_error)? {
            let category = match status.state {
                crate::AccountState::LoggedIn
                | crate::AccountState::LoginRequired
                | crate::AccountState::NotRequired => "configured",
                crate::AccountState::CredentialsMissing => "none",
                crate::AccountState::ServiceUnavailable => "unavailable",
            };
            categories.insert(status.provider, json!(category));
        }
        Ok(Value::Object(categories))
    }
    fn application_list_sources(&self) -> Result<Value, ErrorClass> {
        to_value(&self.0.list_sources().map_err(service_error)?)
    }
    fn application_search_start(
        &self,
        resource_id: &str,
        aoi: &Value,
        aoi_name: &str,
        time_range: &Value,
        filters: &Value,
        request_id: &str,
    ) -> Result<Value, ErrorClass> {
        let request: UnifiedSearchRequest = serde_json::from_value(json!({
            "resource_id": resource_id,
            "aoi": aoi,
            "aoi_name": aoi_name,
            "time_range": time_range,
            "filters": filters,
            "request_id": request_id,
        }))
        .map_err(|_| ErrorClass::InvalidParameters)?;
        let started = self.0.search_start(request).map_err(service_error)?;
        if !started.terminal {
            return self.application_search_progress(&started.request_id);
        }
        to_value(&started)
    }
    fn application_search_progress(&self, request_id: &str) -> Result<Value, ErrorClass> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
        let progress = loop {
            let current = self.0.search_progress(request_id);
            match &current {
                None => break current,
                Some(p) if p.terminal => break current,
                Some(_) if std::time::Instant::now() >= deadline => break current,
                Some(_) => std::thread::sleep(std::time::Duration::from_millis(120)),
            }
        };
        to_value(&progress)
    }
    fn application_batch_page(
        &self,
        batch_id: &str,
        revision: u64,
        page: u32,
        page_size: u32,
    ) -> Result<Value, ErrorClass> {
        let page = self
            .0
            .batch_page(&StoreBatchPageRequest {
                batch_id: batch_id.to_owned(),
                revision,
                page: u64::from(page),
                page_size: u64::from(page_size),
            })
            .map_err(service_error)?;
        // Provider payloads stay in the Store; the model sees identity and state.
        Ok(json!({
            // Citation evidence key for a frozen batch the run actually read.
            "search_batch_id": page.batch_id,
            "batch_id": page.batch_id,
            "revision": page.revision,
            "page": page.page,
            "page_size": page.page_size,
            "total_count": page.total_count,
            "items": page.items.iter().map(|item| json!({
                "item_id": item.item_id,
                "display_name": item.display_name,
                "acquisition_datetime": item.acquisition_datetime,
            })).collect::<Vec<_>>(),
            "rows": page.rows,
        }))
    }
    fn application_create_task(
        &self,
        batch_id: &str,
        revision: u64,
        item_ids: &[String],
        options: &Value,
    ) -> Result<Value, ErrorClass> {
        let options: UnifiedCreateTaskOptions =
            serde_json::from_value(options.clone()).map_err(|_| ErrorClass::InvalidParameters)?;
        to_value(
            &self
                .0
                .create_task(batch_id.to_owned(), revision, item_ids.to_vec(), options)
                .map_err(typed_error)?,
        )
    }
    fn application_task_action(
        &self,
        task_id: &str,
        revision: u64,
        action: &str,
        event_id: &str,
    ) -> Result<Value, ErrorClass> {
        let kind = match action {
            "pause" => TaskStateCommandKind::PauseTask,
            // update_download_task re-checks free space before a resume
            // (DISK_LOW); the Agent's permission mode plays no part in it.
            "resume" => TaskStateCommandKind::ResumeTask,
            "retry" => TaskStateCommandKind::RetryTask,
            "cancel" => TaskStateCommandKind::CancelTask,
            "archive" => {
                return to_value(
                    &self
                        .0
                        .task_action(task_id.to_owned(), revision, UnifiedTaskAction::Archive)
                        .map_err(typed_error)?,
                );
            }
            _ => return Err(ErrorClass::InvalidParameters),
        };
        // Same Store command as the unified action, but with the plan's frozen
        // event id so an unknown outcome can be read back by that id.
        let snapshot = self
            .0
            .update_download_task(&StoreUpdateTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision: revision,
                command: StoreTaskCommand {
                    kind,
                    item_ids: vec![],
                },
                event_id: event_id.to_owned(),
                occurred_at: now(),
            })
            .map_err(service_error)?;
        let task = snapshot
            .core
            .tasks
            .into_iter()
            .find(|task| task.task_id == task_id)
            .ok_or(ErrorClass::ResultUnknown)?;
        Ok(
            json!({"task_id": task.task_id, "revision": task.revision, "status": task.status, "created": false}),
        )
    }
    fn application_update_download_task(&self, request: &Value) -> Result<Value, ErrorClass> {
        let request: StoreUpdateTaskRequest =
            serde_json::from_value(request.clone()).map_err(|_| ErrorClass::InvalidParameters)?;
        let snapshot = self
            .0
            .update_download_task(&request)
            .map_err(service_error)?;
        let task = snapshot
            .core
            .tasks
            .into_iter()
            .find(|task| task.task_id == request.task_id)
            .ok_or(ErrorClass::ResultUnknown)?;
        Ok(json!({"task_id": task.task_id, "revision": task.revision, "status": task.status}))
    }
    fn application_task_overview(&self) -> Result<Value, ErrorClass> {
        let overview = self.0.task_overview().map_err(service_error)?;
        // The overview rows are generated contract types since W2 batch 4 (flat
        // fields on both the queue task and the history item).
        let queue: Vec<Value> = overview
            .queue
            .iter()
            .map(|task| {
                json!({
                    "task_id": task.task_id,
                    "batch_id": task.batch_id,
                    "provider_id": task.provider_id,
                    "status": task.status,
                    "revision": task.revision,
                    "selected_count": task.selected_count,
                    "updated_at": task.updated_at,
                })
            })
            .collect();
        let history: Vec<Value> = overview
            .history_page
            .items
            .iter()
            .map(|item| {
                json!({
                    "task_id": item.task_id,
                    "batch_id": item.batch_id,
                    "provider_id": item.provider_id,
                    "status": item.status,
                    "revision": item.revision,
                    "selected_count": item.selected_count,
                    "updated_at": item.updated_at,
                })
            })
            .collect();
        Ok(json!({ "queue": queue, "history": history }))
    }
    fn application_account_session_statuses(&self) -> Result<Value, ErrorClass> {
        to_value(&self.0.account_session_statuses().map_err(service_error)?)
    }
    fn application_task_event_state(
        &self,
        task_id: &str,
        event_id: &str,
    ) -> Result<Option<Value>, ErrorClass> {
        self.with_task_store(|store| {
            let Some(task) = store.load_task_record(task_id)? else {
                return Ok(None);
            };
            let landed = store
                .list_task_events(task_id)?
                .iter()
                .any(|event| event.event_id == event_id);
            Ok(landed.then(|| {
                json!({
                    "task_id": task_id,
                    "last_event_id": event_id,
                    "status": task.runtime.status,
                    "revision": task.runtime.revision,
                })
            }))
        })
        .map_err(service_error)?
        .map_err(|_: insar_store::StoreError| ErrorClass::Internal)
    }
    /// The storage root setting (SAR download policy `default_output_root`).
    fn application_default_output_root(&self) -> Result<Option<String>, ErrorClass> {
        let settings = self
            .0
            .get_sar_download_policy_settings()
            .map_err(service_error)?;
        let root = settings.default_output_root.trim();
        Ok((!root.is_empty()).then(|| root.to_owned()))
    }
    fn application_search_batch(&self, request_id: &str) -> Result<Option<String>, ErrorClass> {
        Ok(self.0.search_batch_id(request_id))
    }
    fn application_selection_accounts_ready(
        &self,
        batch_id: &str,
        item_ids: &[String],
    ) -> Result<bool, ErrorClass> {
        self.0
            .selection_accounts_ready(batch_id, item_ids)
            .map_err(service_error)
    }
    fn application_admin_boundary_search(&self, input: &Value) -> Result<Value, ErrorClass> {
        let str_field = |key: &str| {
            input
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_owned()
        };
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).clamp(1, 10))
            .unwrap_or(8);
        let request = AdminBoundarySearchRequest {
            query: str_field("query"),
            province: str_field("province"),
            city: str_field("city"),
            district: str_field("district"),
            provider_object_id: input
                .get("provider_object_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            limit: Some(limit),
        };
        let response = self
            .0
            .admin_boundary_search(&request)
            .map_err(service_error)?;
        let candidates: Vec<Value> = response
            .results
            .into_iter()
            .take(10)
            .map(|item| {
                let admin_code = if item.provider == "nominatim"
                    && !item.provider_object_id.starts_with("nominatim:")
                {
                    format!("nominatim:{}", item.provider_object_id)
                } else {
                    item.provider_object_id.clone()
                };
                json!({
                    "admin_code": admin_code,
                    "code": item.id,
                    "name": item.name,
                    "full_name": item.display_name,
                    "display_name": item.display_name,
                    "level": item.level,
                    "country_code": item.country_code,
                    "provider": item.provider,
                    "bbox": [item.bbox.west, item.bbox.south, item.bbox.east, item.bbox.north],
                })
            })
            .collect();
        Ok(json!({
            "ok": response.ok,
            "provider": response.provider,
            "warning": response.warning,
            "count": candidates.len(),
            "candidates": candidates,
        }))
    }
    fn application_preview_working_aoi_set(&self, input: &Value) -> Result<Value, ErrorClass> {
        Ok(match self.prepare_working_aoi_set(input)? {
            Some(req) => json!({
                "state": "working",
                "source_kind": req.source_kind,
                "display_name": req.display_name,
                "bbox": req.bbox,
                "crs": req.crs,
                "geometry": req.geometry,
            }),
            None => json!({"state": "none"}),
        })
    }
    fn application_set_working_aoi(&self, input: &Value) -> Result<Value, ErrorClass> {
        match self.prepare_working_aoi_set(input)? {
            Some(req) => {
                let saved = self.0.set_working_aoi(&req).map_err(service_error)?;
                Ok(json!({
                    "state": saved.state,
                    "source_kind": saved.source_kind,
                    "display_name": saved.display_name,
                    "bbox": saved.bbox,
                    "crs": saved.crs,
                    "geometry": saved.geometry,
                }))
            }
            None => {
                self.0.clear_working_aoi().map_err(service_error)?;
                Ok(json!({"state": "none"}))
            }
        }
    }
    fn application_restore_working_aoi(&self, snapshot: &Value) -> Result<Value, ErrorClass> {
        if snapshot.get("state").and_then(Value::as_str) == Some("none")
            || snapshot.get("geometry").is_none()
        {
            self.0.clear_working_aoi().map_err(service_error)?;
            return Ok(json!({"state": "none"}));
        }
        let raw_source_kind = snapshot
            .get("source_kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        let source_kind = match raw_source_kind {
            "manual_bbox" | "manual_polygon" | "uploaded_geometry" | "administrative_boundary" => {
                raw_source_kind.to_owned()
            }
            "bbox" => "manual_bbox".to_owned(),
            _ => "administrative_boundary".to_owned(),
        };
        let display_name = snapshot
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or("工作区")
            .to_owned();
        let crs = snapshot
            .get("crs")
            .and_then(Value::as_str)
            .unwrap_or("EPSG:4326")
            .to_owned();
        let bbox = snapshot
            .get("bbox")
            .and_then(Value::as_array)
            .and_then(|arr| arr.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>())
            .ok_or(ErrorClass::InvalidParameters)?;
        let geometry = snapshot
            .get("geometry")
            .and_then(Value::as_object)
            .cloned()
            .ok_or(ErrorClass::InvalidParameters)?;
        let saved = self
            .0
            .set_working_aoi(&ApplicationSetWorkingAoiRequest {
                source_kind,
                geometry,
                bbox,
                crs,
                display_name,
            })
            .map_err(service_error)?;
        Ok(json!({
            "state": saved.state,
            "source_kind": saved.source_kind,
            "display_name": saved.display_name,
            "bbox": saved.bbox,
            "crs": saved.crs,
            "geometry": saved.geometry,
        }))
    }
}

impl StoreAccess for AgentApplication {
    fn with_store<T>(&self, operation: impl FnOnce(&mut TaskStore) -> T) -> Result<T, HostError> {
        self.with_task_store(operation)
            .map_err(|_| HostError::StoreUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_agent_s1_capabilities_puts_vv_first_and_tags_land_vs_polar() {
        let mut raw = json!({
            "provider_id": "asf",
            "available": true,
            "source_version": "v1",
            "datasets": [
                {
                    "dataset": "SENTINEL-1",
                    "display_name": "Sentinel-1",
                    "polarizations": ["HH", "HH+HV", "VV", "VV+VH"]
                },
                {
                    "dataset": "ALOS",
                    "display_name": "ALOS PALSAR",
                    "polarizations": ["HH", "HV"]
                }
            ],
            "sources": [
                {
                    "selector": "sentinel1_slc",
                    "display_name": "Sentinel-1 SLC",
                    "logical_dataset": "SENTINEL-1",
                    "filters": {
                        "polarizations": ["HH", "HH+HV", "VV", "VV+VH"]
                    }
                }
            ]
        });
        normalize_agent_s1_capabilities(&mut raw);
        assert_eq!(
            raw["datasets"][0]["polarizations"],
            json!(["VV", "VV+VH", "HH", "HH+HV"])
        );
        assert_eq!(
            raw["datasets"][0]["land_default_polarizations"],
            json!(["VV", "VV+VH"])
        );
        assert_eq!(
            raw["datasets"][0]["polar_maritime_only_polarizations"],
            json!(["HH", "HH+HV"])
        );
        assert_eq!(raw["datasets"][1]["polarizations"], json!(["HH", "HV"]));
        assert_eq!(
            raw["sources"][0]["filters"]["polarizations"],
            json!(["VV", "VV+VH", "HH", "HH+HV"])
        );
        append_agent_optical_capabilities(&mut raw);
        let optical = raw["optical_datasets"]
            .as_array()
            .expect("optical_datasets array");
        let landsat = optical
            .iter()
            .find(|item| item["resource_id"] == "science.landsat.c2-l2.v1")
            .expect("Landsat C2 L2 in optical_datasets");
        let filter_keys: Vec<&str> = landsat["filters"]
            .as_array()
            .expect("filters")
            .iter()
            .filter_map(|f| f["query_key"].as_str())
            .collect();
        assert_eq!(
            filter_keys,
            vec!["spacecraft", "processing_levels", "cloud_cover_lte"]
        );
    }

    #[test]
    fn bbox_polygon_geometry_builds_closed_wgs84_ring() {
        let (geom, bbox) = bbox_polygon_geometry(&[100.0, 28.0, 105.0, 32.0]).expect("valid bbox");
        assert_eq!(bbox, vec![100.0, 28.0, 105.0, 32.0]);
        assert_eq!(geom.get("type").and_then(Value::as_str), Some("Polygon"));
        let ring = geom["coordinates"][0].as_array().expect("outer ring");
        assert_eq!(ring.len(), 5);
        assert_eq!(ring[0], ring[4]);
        assert!(bbox_polygon_geometry(&[105.0, 28.0, 100.0, 32.0]).is_none());
    }

    #[test]
    fn working_aoi_set_and_restore_persist_valid_source_kinds_in_task_store() {
        use crate::ApplicationServiceConfig;
        use insar_provider::boundary::{
            ChinaAdminOptions, ChinaBoundaryProvider, GlobalBoundaryProvider,
            ProviderBoundaryCandidate, ProviderBoundaryFeature,
        };
        use std::time::{SystemTime, UNIX_EPOCH};

        struct StubGlobal;
        impl GlobalBoundaryProvider for StubGlobal {
            fn search(
                &self,
                _query: &str,
                _limit: usize,
            ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
                Ok(vec![])
            }
            fn discover(
                &self,
                _query: &str,
                _limit: usize,
            ) -> insar_core::ProviderResult<Vec<ProviderBoundaryCandidate>> {
                Ok(vec![])
            }
            fn discover_china(
                &self,
                _query: &str,
                _limit: usize,
            ) -> insar_core::ProviderResult<Vec<ProviderBoundaryCandidate>> {
                Ok(vec![])
            }
            fn resolve(
                &self,
                _provider_object_id: &str,
            ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
                unreachable!("global resolve not called for China adcode")
            }
        }

        struct StubSichuanChina;
        impl ChinaBoundaryProvider for StubSichuanChina {
            fn options(
                &self,
                _province: &str,
                _city: &str,
            ) -> insar_core::ProviderResult<ChinaAdminOptions> {
                Ok(ChinaAdminOptions::default())
            }
            fn resolve_exact_adcode(
                &self,
                adcode: &str,
            ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
                assert_eq!(adcode, "510000");
                Ok(ProviderBoundaryFeature {
                    provider_object_id: "510000".to_owned(),
                    name: "四川省".to_owned(),
                    display_name: "四川省".to_owned(),
                    country_code: Some("CN".to_owned()),
                    admin_level: Some("province".to_owned()),
                    geometry: json!({
                        "type": "Polygon",
                        "coordinates": [[[97.35, 26.05], [108.54, 26.05], [108.54, 34.31], [97.35, 34.31], [97.35, 26.05]]]
                    }),
                    geometry_kind: "polygon".to_owned(),
                    bbox: [97.35, 26.05, 108.54, 34.31],
                    source: "DataV areas_v3".to_owned(),
                    attribution: None,
                })
            }
            fn search(
                &self,
                _province: &str,
                _city: &str,
                _district: &str,
                _query: &str,
                _limit: usize,
            ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
                Ok(vec![self.resolve_exact_adcode("510000")?])
            }
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let store_path = std::env::temp_dir().join(format!(
            "insar-agent-tools-working-aoi-{}-{nonce}.sqlite3",
            std::process::id()
        ));

        let service = Arc::new(ApplicationService::with_boundary_providers(
            ApplicationServiceConfig::business_store(&store_path),
            Arc::new(StubGlobal),
            Arc::new(StubSichuanChina),
        ));
        let app = AgentApplication(service);

        // 1. admin_code ("510000") persists as "administrative_boundary" in TaskStore
        let admin_saved = app
            .application_set_working_aoi(&json!({
                "admin_code": "510000",
                "country_code": "CN",
                "display_name": "四川省"
            }))
            .expect("admin_code working_aoi_set must succeed against TaskStore");
        assert_eq!(admin_saved["source_kind"], "administrative_boundary");
        assert_eq!(admin_saved["display_name"], "四川省");

        // 2. "datav:510000" prefix is stripped and persists cleanly
        let datav_saved = app
            .application_set_working_aoi(&json!({
                "admin_code": "datav:510000",
                "display_name": "四川省"
            }))
            .expect("datav:510000 working_aoi_set must strip prefix and succeed");
        assert_eq!(datav_saved["source_kind"], "administrative_boundary");

        // 3. bbox persists as "manual_bbox" in TaskStore
        let bbox_saved = app
            .application_set_working_aoi(&json!({
                "bbox": [103.5, 30.2, 104.6, 31.1],
                "display_name": "成都平原框选"
            }))
            .expect("bbox working_aoi_set must succeed against TaskStore");
        assert_eq!(bbox_saved["source_kind"], "manual_bbox");
        assert_eq!(bbox_saved["display_name"], "成都平原框选");

        // 4. restore normalizes legacy/non-canonical source_kind ("restored") to "administrative_boundary"
        let restored = app
            .application_restore_working_aoi(&json!({
                "state": "working",
                "source_kind": "restored",
                "display_name": "西藏自治区",
                "crs": "EPSG:4326",
                "bbox": [78.39, 26.85, 99.12, 36.48],
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[[78.39, 26.85], [99.12, 26.85], [99.12, 36.48], [78.39, 36.48], [78.39, 26.85]]]
                }
            }))
            .expect("restore_working_aoi with legacy source_kind must succeed against TaskStore");
        assert_eq!(restored["source_kind"], "administrative_boundary");
        assert_eq!(restored["display_name"], "西藏自治区");

        // 5. clear removes working AOI cleanly
        let cleared = app
            .application_set_working_aoi(&json!({ "clear": true }))
            .expect("clear working_aoi_set must succeed");
        assert_eq!(cleared["state"], "none");

        drop(app);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", store_path.display(), suffix));
        }
    }
}
