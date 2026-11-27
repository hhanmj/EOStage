//! Fixed Agent-to-ApplicationService operations. The desktop implements
//! `UnifiedApplication` on its one ApplicationService; no generic Tauri command
//! name or model-supplied credential crosses this port.
use crate::{
    catalog,
    executor::{ErrorClass, ToolPort, ToolResult},
};
use serde::Deserialize;
use serde_json::{json, Value};

/// C1 method signatures: all identities refer to the existing Store/ResourceRegistry.
/// The desktop implementation must use its already-owned ApplicationService,
/// never open a second Store connection in a sidecar.
pub trait UnifiedApplication: Send {
    fn application_service_readiness(&self) -> Result<Value, ErrorClass>;
    fn application_provider_capabilities(&self) -> Result<Value, ErrorClass>;
    fn application_working_aoi(&self) -> Result<Value, ErrorClass>;
    fn application_workbench_page(
        &self,
        page_index: u64,
        page_size: u64,
    ) -> Result<Value, ErrorClass>;
    fn application_download_task_page(
        &self,
        task_id: &str,
        page_index: u64,
        page_size: u64,
    ) -> Result<Value, ErrorClass>;
    fn application_download_task_event_page(
        &self,
        task_id: &str,
        page_size: u64,
    ) -> Result<Value, ErrorClass>;
    fn application_credential_categories(&self) -> Result<Value, ErrorClass>;
    fn application_list_sources(&self) -> Result<Value, ErrorClass>;
    fn application_search_start(
        &self,
        resource_id: &str,
        aoi: &Value,
        aoi_name: &str,
        time_range: &Value,
        filters: &Value,
        request_id: &str,
    ) -> Result<Value, ErrorClass>;
    fn application_search_progress(&self, request_id: &str) -> Result<Value, ErrorClass>;
    fn application_batch_page(
        &self,
        batch_id: &str,
        revision: u64,
        page: u32,
        page_size: u32,
    ) -> Result<Value, ErrorClass>;
    fn application_create_task(
        &self,
        batch_id: &str,
        revision: u64,
        item_ids: &[String],
        options: &Value,
    ) -> Result<Value, ErrorClass>;
    /// `event_id` is the plan's frozen idempotency key; the service must persist
    /// it so an unknown outcome can be read back instead of replayed.
    fn application_task_action(
        &self,
        task_id: &str,
        revision: u64,
        action: &str,
        event_id: &str,
    ) -> Result<Value, ErrorClass>;
    fn application_update_download_task(&self, request: &Value) -> Result<Value, ErrorClass>;
    fn application_task_overview(&self) -> Result<Value, ErrorClass>;
    fn application_account_session_statuses(&self) -> Result<Value, ErrorClass>;
    /// Store readback for one event; `None` when the event never landed.
    fn application_task_event_state(
        &self,
        task_id: &str,
        event_id: &str,
    ) -> Result<Option<Value>, ErrorClass>;
    /// The default download directory (the storage root setting), `None`
    /// when unset. The defaults below are the conservative answers: no
    /// directory, no known batch, accounts not ready (each sends a plan to
    /// the desktop card).
    fn application_default_output_root(&self) -> Result<Option<String>, ErrorClass> {
        Ok(None)
    }
    /// The batch a search started through `application_search_start`
    /// froze, by its request id (kept in memory by the service).
    fn application_search_batch(&self, _request_id: &str) -> Result<Option<String>, ErrorClass> {
        Ok(None)
    }
    /// Whether every account a frozen selection needs (the create_task
    /// account preflight's set) has a saved credential and a reachable
    /// service, so admission needs no user action.
    fn application_selection_accounts_ready(
        &self,
        _batch_id: &str,
        _item_ids: &[String],
    ) -> Result<bool, ErrorClass> {
        Ok(false)
    }
    fn application_admin_boundary_search(&self, _input: &Value) -> Result<Value, ErrorClass> {
        Err(ErrorClass::Unsupported)
    }
    /// Resolves the AOI state and geometry that `application_set_working_aoi`
    /// would persist without mutating the Store yet, so `propose` can stage
    /// `working_aoi` across steps in a single plan.
    fn application_preview_working_aoi_set(&self, _input: &Value) -> Result<Value, ErrorClass> {
        Err(ErrorClass::Unsupported)
    }
    fn application_set_working_aoi(&self, _input: &Value) -> Result<Value, ErrorClass> {
        Err(ErrorClass::Unsupported)
    }
    fn application_restore_working_aoi(&self, _snapshot: &Value) -> Result<Value, ErrorClass> {
        Err(ErrorClass::Unsupported)
    }
    /// Optional desktop-owned extension system prompt instructions (e.g. GEE Cloud Suite).
    fn application_extension_system_prompt(&self) -> Option<String> {
        None
    }
    /// Optional desktop-owned extension tools offered alongside `model_tools()`.
    fn application_extension_tools(&self) -> Vec<insar_agent_model::ToolDescription> {
        Vec::new()
    }
    /// Dispatch a desktop-owned extension tool. Returns `None` if `tool_id` is not an extension tool.
    fn application_call_extension_tool(
        &self,
        _tool_id: &str,
        _input: &Value,
    ) -> Option<Result<Value, ErrorClass>> {
        None
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentAdminBoundarySearchInput {
    pub query: String,
    pub province: String,
    pub city: String,
    pub district: String,
    pub country_code: Option<String>,
    pub level: Option<String>,
    pub provider_object_id: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentBatchPageInput {
    pub batch_id: String,
    pub revision: u64,
    pub page: u32,
    pub page_size: u32,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSearchProgressInput {
    pub request_id: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCreateTaskInput {
    pub batch_id: String,
    pub revision: u64,
    pub item_ids: Vec<String>,
    pub options: Value,
    pub event_id: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskActionInput {
    pub task_id: String,
    pub revision: u64,
    pub action: String,
    pub event_id: String,
}

fn scalar(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

/// The single bounded-argument gate shared by the executor and host port.
pub fn bounded_input(tool_id: &str, input: &Value) -> bool {
    input_problem(tool_id, input).is_none()
}

/// Why `bounded_input` refuses an input (the deserializer's or validator's
/// own text), `None` when it is accepted. The text may quote the input, so a
/// caller recording it must pass it through `run::safe_value` first.
pub fn input_problem(tool_id: &str, input: &Value) -> Option<String> {
    fn parse<T: serde::de::DeserializeOwned>(input: &Value) -> Result<T, String> {
        serde_json::from_value(input.clone()).map_err(|error| error.to_string())
    }
    fn require(ok: bool, problem: &str) -> Result<(), String> {
        if ok {
            Ok(())
        } else {
            Err(problem.to_owned())
        }
    }
    const SCALAR: &str =
        "must be a non-empty string without surrounding spaces or control characters";
    let checked: Result<(), String> = match tool_id {
        "service_readiness"
        | "capabilities_get"
        | "working_aoi_get"
        | "credential_readiness"
        | "source_list"
        | "account_readiness"
        | "task_overview" => require(
            input.as_object().is_some_and(serde_json::Map::is_empty),
            "input must be an empty object",
        ),
        "workbench_page" => parse::<insar_core::AgentWorkbenchPageInput>(input)
            .and_then(|request| request.validate()),
        "task_page" => {
            parse::<insar_core::AgentTaskPageInput>(input).and_then(|request| request.validate())
        }
        "task_event_page" => parse::<insar_core::AgentTaskEventPageInput>(input)
            .and_then(|request| request.validate()),
        "batch_page" => parse::<AgentBatchPageInput>(input).and_then(|r| {
            require(scalar(&r.batch_id, 128), &format!("batch_id {SCALAR}"))?;
            require(
                (1..=100).contains(&r.page_size),
                "page_size must be 1 to 100",
            )
        }),
        "search_progress" => parse::<AgentSearchProgressInput>(input)
            .and_then(|r| require(scalar(&r.request_id, 128), &format!("request_id {SCALAR}"))),
        "search_start" => parse::<insar_core::UnifiedSearchRequest>(input).and_then(|r| {
            r.validate()?;
            require(scalar(&r.request_id, 128), &format!("request_id {SCALAR}"))?;
            require(
                scalar(&r.resource_id, 128),
                &format!("resource_id {SCALAR}"),
            )?;
            require(!r.aoi_name.trim().is_empty(), "aoi_name must not be blank")
        }),
        "create_task" => parse::<AgentCreateTaskInput>(input).and_then(|r| {
            require(scalar(&r.batch_id, 128), &format!("batch_id {SCALAR}"))?;
            require(scalar(&r.event_id, 128), &format!("event_id {SCALAR}"))?;
            require(
                (1..=250).contains(&r.item_ids.len()),
                "item_ids must name 1 to 250 products",
            )?;
            require(
                r.item_ids.iter().all(|id| scalar(id, 256)),
                &format!("every item_id {SCALAR}"),
            )?;
            require(r.options.is_object(), "options must be an object")
        }),
        "task_action" => parse::<AgentTaskActionInput>(input).and_then(|r| {
            require(scalar(&r.task_id, 128), &format!("task_id {SCALAR}"))?;
            require(scalar(&r.event_id, 128), &format!("event_id {SCALAR}"))?;
            require(
                matches!(
                    r.action.as_str(),
                    "pause" | "resume" | "retry" | "cancel" | "archive"
                ),
                "action must be pause, resume, retry, cancel or archive",
            )
        }),
        "update_download_task" => parse::<insar_core::StoreUpdateTaskRequest>(input).map(|_| ()),
        "admin_boundary_search" => parse::<AgentAdminBoundarySearchInput>(input).and_then(|r| {
            let has_scope = !r.query.trim().is_empty()
                || !r.province.trim().is_empty()
                || !r.city.trim().is_empty()
                || !r.district.trim().is_empty()
                || r.provider_object_id
                    .as_deref()
                    .is_some_and(|id| !id.trim().is_empty());
            require(
                has_scope,
                "admin_boundary_search requires query, province, city, district or provider_object_id",
            )?;
            for (name, text) in [
                ("query", r.query.as_str()),
                ("province", r.province.as_str()),
                ("city", r.city.as_str()),
                ("district", r.district.as_str()),
            ] {
                if !text.is_empty() {
                    require(
                        text.chars().count() <= 160 && !text.chars().any(char::is_control),
                        &format!("{name} must be at most 160 characters without control characters"),
                    )?;
                }
            }
            if let Some(id) = &r.provider_object_id {
                require(scalar(id, 160), &format!("provider_object_id {SCALAR}"))?;
            }
            if let Some(code) = &r.country_code {
                require(scalar(code, 16), &format!("country_code {SCALAR}"))?;
            }
            if let Some(level) = &r.level {
                require(scalar(level, 32), &format!("level {SCALAR}"))?;
            }
            if let Some(limit) = r.limit {
                require((1..=20).contains(&limit), "limit must be 1 to 20")?;
            }
            Ok(())
        }),
        "working_aoi_set" => parse::<insar_core::AgentWorkingAoiSetInput>(input).and_then(|r| {
            r.validate()?;
            if r.clear {
                require(
                    r.admin_code.is_none() && r.bbox.is_none(),
                    "clear cannot be combined with admin_code or bbox",
                )?;
            } else if let Some(code) = &r.admin_code {
                require(r.bbox.is_none(), "admin_code cannot be combined with bbox")?;
                require(scalar(code, 160), &format!("admin_code {SCALAR}"))?;
            } else if let Some(bbox) = &r.bbox {
                require(
                    bbox.len() == 4 && bbox.iter().all(|v| v.is_finite()),
                    "bbox must contain 4 finite numbers [west, south, east, north]",
                )?;
                let (w, s, e, n) = (bbox[0], bbox[1], bbox[2], bbox[3]);
                require(
                    (-180.0..=180.0).contains(&w)
                        && (-180.0..=180.0).contains(&e)
                        && w < e
                        && (-90.0..=90.0).contains(&s)
                        && (-90.0..=90.0).contains(&n)
                        && s < n,
                    "bbox must satisfy -180 <= west < east <= 180 and -90 <= south < north <= 90",
                )?;
            } else {
                return Err("working_aoi_set requires admin_code, bbox or clear".to_owned());
            }
            if let Some(code) = &r.country_code {
                require(scalar(code, 16), &format!("country_code {SCALAR}"))?;
            }
            if let Some(name) = &r.display_name {
                require(
                    !name.trim().is_empty()
                        && name.chars().count() <= 160
                        && !name.chars().any(char::is_control),
                    "display_name must be 1 to 160 printable characters",
                )?;
            }
            Ok(())
        }),
        _ => Err("tool is not in the catalog".to_owned()),
    };
    checked.err()
}

/// Input fields of a plan step that the host fills itself (idempotency and
/// request keys, the proposal time). A model value is overwritten, so they
/// are not offered in the schema the model sees and not reported as unknown.
pub fn host_filled(tool_id: &str) -> &'static [&'static str] {
    match tool_id {
        "search_start" => &["request_id"],
        "create_task" | "task_action" => &["event_id"],
        "update_download_task" => &["event_id", "occurred_at"],
        _ => &[],
    }
}

/// The input a model fills for one `propose_plan` step, for the tools a plan
/// may name: the fields of each tool's input type above, without
/// `host_filled` ones. `None` for any other tool.
pub fn plan_input_schema(tool_id: &str) -> Option<Value> {
    let (required, properties) = match tool_id {
        "search_start" => (
            json!(["resource_id", "aoi"]),
            json!({
                "resource_id": {"type": "string", "minLength": 1, "maxLength": 128,
                    "description": "数据源编号：source_list 结果里的 resource_id 原样填写"},
                "aoi": {"type": "object", "required": ["ref"], "additionalProperties": false,
                    "properties": {"ref": {"type": "string", "enum": [crate::aoi::CONTEXT_REF, crate::aoi::WORKING_REF]}},
                    "description": "检索区域引用：context_aoi 为用户本条消息附带的区域，working_aoi 为当前工作区；不提供几何"},
                "aoi_name": {"type": "string", "minLength": 1, "maxLength": 160,
                    "description": "可省略；省略时用该区域自带的名称，没有名称时为「工作区」"},
                "time_range": {"type": "object", "additionalProperties": false,
                    "properties": {
                        "start": {"type": "string", "format": "date-time"},
                        "end": {"type": "string", "format": "date-time"},
                    },
                    "description": "获取时间范围：start 与 end 为 RFC 3339 UTC 时间，例如 2026-07-26T00:00:00Z 与 2026-10-26T23:59:59Z"},
                "filters": {"type": "object",
                    "description": "数据源筛选条件，可省略；键名以 capabilities_get 为准。SAR（Sentinel-1 等）各键的取值类型：flight_direction 是单个字符串，取 ASCENDING 或 DESCENDING，不是数组；processing_levels、beam_modes、polarizations、platforms 是字符串数组，只有一个值也写成数组，例如 processing_levels 写成只含 SLC 一个元素的数组；relative_orbit、frame、path 是整数。Sentinel-1 陆地常规区域（含中国各省及中低纬度大陆）主力采集 IW 模式的 VV+VH 双极化（ASF 检索若只填单个 VV 会仅匹配罕见单极化而漏掉全部 VV+VH 常规产品），HH 与 HH+HV 仅用于极地冰盖和海冰海域；除非用户明确要求单极化或 HH，陆地常规检索请省略 polarizations，或同时填写 VV+VH 与 VV"},
            }),
        ),
        "working_aoi_set" => (
            json!([]),
            json!({
                "admin_code": {"type": "string", "minLength": 1, "maxLength": 160,
                    "description": "行政区代码或边界标识：优先填写 admin_boundary_search 返回的 admin_code（例如四川省为 510000，或 Nominatim 与 GeoBoundaries 候选标识），主机将解析真实行政区多边形边界并设为当前工作区"},
                "country_code": {"type": "string", "minLength": 2, "maxLength": 16,
                    "description": "可选国家或地区代码，例如 CN 或 CHN"},
                "bbox": {"type": "array", "minItems": 4, "maxItems": 4,
                    "items": {"type": "number"},
                    "description": "仅在无行政区代码、按经纬度矩形设置工作区时填写四个数值 west、south、east、north（WGS84 度）；有 admin_code 时不要填写 bbox"},
                "display_name": {"type": "string", "minLength": 1, "maxLength": 160,
                    "description": "工作区显示名称，例如 四川省；使用 bbox 时建议填写"},
                "clear": {"type": "boolean",
                    "description": "设为 true 时清空当前工作区，此时不要同时填写 admin_code 或 bbox"},
            }),
        ),
        "create_task" => (
            json!(["batch_id", "revision", "item_ids", "options"]),
            json!({
                "batch_id": {"type": "string", "minLength": 1, "maxLength": 128},
                "revision": {"type": "integer", "minimum": 0},
                "item_ids": {"type": "array", "minItems": 1, "maxItems": 250,
                    "items": {"type": "string", "minLength": 1, "maxLength": 256}},
                "options": {"type": "object", "additionalProperties": false,
                    "required": ["layout", "concurrency"],
                    "properties": {
                        // Plain text only: the schema is also returned inside a
                        // refusal result, where a quoted fragment would be
                        // escaped and read as a local path by the outbound check.
                        "output_root": {"anyOf": [
                                {"type": "string"},
                                {"type": "object", "required": ["ref"], "additionalProperties": false,
                                    "properties": {"ref": {"type": "string", "enum": [crate::message_dir::REF]}}},
                            ],
                            "description": "下载目录。用户消息里有本机目录时（消息中显示为 local-reference:message_dir），写成只含 ref 的对象，ref 取 message_dir；消息里没有本机目录时省略，使用默认下载目录。不要写出任何路径"},
                        "layout": {"type": "string", "enum": ["direct", "sentinel1_slc", "sentinel_orbit_aux_poeorb", "sar_dataset_processing"]},
                        "concurrency": {"type": "integer", "minimum": 1},
                        "include_precise_orbit": {"type": "boolean",
                            "description": "是否同时创建配套精密轨道（POEORB）下载任务。省略时按数据源策略：SLC 与 Burst 默认创建，GRD 默认不创建"},
                    }},
            }),
        ),
        "task_action" => (
            json!(["task_id", "revision", "action"]),
            json!({
                "task_id": {"type": "string", "minLength": 1, "maxLength": 128},
                "revision": {"type": "integer", "minimum": 0},
                "action": {"type": "string", "enum": ["pause", "resume", "retry", "cancel", "archive"]},
            }),
        ),
        "update_download_task" => (
            json!(["task_id", "expected_revision", "command"]),
            json!({
                "task_id": {"type": "string", "minLength": 1},
                "expected_revision": {"type": "integer", "minimum": 0},
                "command": {"type": "object", "required": ["kind", "item_ids"], "additionalProperties": false,
                    "properties": {
                        "kind": {"type": "string", "enum": ["pause_task", "resume_task", "retry_task", "cancel_task",
                            "activate_items", "pause_items", "resume_items", "retry_items",
                            "mark_completed", "mark_failed", "mark_skipped", "cancel_items",
                            "reacquire_items"]},
                        "item_ids": {"type": "array", "items": {"type": "string", "minLength": 1}},
                    }},
            }),
        ),
        _ => return None,
    };
    Some(json!({
        "type": "object",
        "required": required,
        "properties": properties,
        "additionalProperties": false,
    }))
}

/// Top-level fields of a model's step input that `plan_input_schema` does
/// not name (host-filled ones excepted) and required fields it left out,
/// each sorted. Both empty when the tool has no plan schema.
pub fn input_field_diff(tool_id: &str, input: &Value) -> (Vec<String>, Vec<String>) {
    let (Some(schema), Some(given)) = (plan_input_schema(tool_id), input.as_object()) else {
        return (vec![], vec![]);
    };
    let known = schema["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let filled = host_filled(tool_id);
    let mut unknown: Vec<String> = given
        .keys()
        .filter(|key| !known.contains_key(*key) && !filled.contains(&key.as_str()))
        .cloned()
        .collect();
    let mut missing: Vec<String> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|field| !given.contains_key(*field))
        .map(str::to_owned)
        .collect();
    unknown.sort();
    missing.sort();
    (unknown, missing)
}

/// Reduce a service projection to what a run trace may retain: sensitive keys
/// are dropped and local paths or signed strings become a fixed marker, so a
/// single output path in a page does not make the whole read unusable.
pub fn sanitize(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| {
                    let mut probe = serde_json::Map::new();
                    probe.insert((*key).clone(), Value::Null);
                    crate::run::safe_value(&Value::Object(probe)).is_ok()
                })
                .map(|(key, entry)| (key.clone(), sanitize(entry)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(sanitize).collect()),
        Value::String(_) => crate::run::safe_value(value).unwrap_or_else(|_| json!("[local]")),
        _ => value.clone(),
    }
}

/// Account readiness is categories only: provider id and one fixed state word.
pub fn account_categories(raw: &Value) -> Value {
    let mut output = serde_json::Map::new();
    for entry in raw.as_array().into_iter().flatten() {
        let (Some(provider), Some(state)) = (
            entry.get("provider").and_then(Value::as_str),
            entry.get("state").and_then(Value::as_str),
        ) else {
            continue;
        };
        if !provider
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            continue;
        }
        let state = match state {
            "logged_in" | "login_required" | "credentials_missing" | "not_required" => state,
            _ => "service_unavailable",
        };
        output.insert(provider.to_owned(), json!(state));
    }
    Value::Object(output)
}

/// Only tools registered by the catalog are executable. Approval is enforced by
/// `Executor` before `submit` is reached; this port checks catalog tier and
/// bounded arguments again so it is never a generic command passthrough.
pub struct HostPort<S> {
    service: S,
}
impl<S: UnifiedApplication> HostPort<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }
    pub fn into_service(self) -> S {
        self.service
    }
    pub fn service(&self) -> &S {
        &self.service
    }
}

fn observed_now() -> Result<String, ErrorClass> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ErrorClass::Internal)?
        .as_secs()
        .to_string())
}

impl<S: UnifiedApplication> ToolPort for HostPort<S> {
    fn read(&mut self, target_command: &str, input: &Value) -> Result<ToolResult, ErrorClass> {
        let tool = catalog::catalog()
            .tools
            .into_iter()
            .find(|tool| {
                tool.target_command == target_command
                    && tool.confirmation == "none"
                    && tool.effect == "read"
            })
            .ok_or(ErrorClass::Unsupported)?;
        if !bounded_input(&tool.tool_id, input) {
            return Err(ErrorClass::InvalidParameters);
        }
        let field = |name: &str| input.get(name).and_then(Value::as_str).unwrap_or("");
        let number = |name: &str| input.get(name).and_then(Value::as_u64).unwrap_or(0);
        let summary = match tool.tool_id.as_str() {
            "service_readiness" => {
                let raw = self.service.application_service_readiness()?;
                json!({"category": if raw.get("status").and_then(Value::as_str) == Some("store_ready") { "ready" } else { "failed" }})
            }
            "capabilities_get" => self.service.application_provider_capabilities()?,
            // Bounding box, area and a reference only; the geometry stays local.
            "working_aoi_get" => {
                crate::aoi::working_summary(&self.service.application_working_aoi()?)
            }
            "admin_boundary_search" => normalize_boundary_search_output(
                self.service.application_admin_boundary_search(input)?,
            ),
            "credential_readiness" => self.service.application_credential_categories()?,
            "workbench_page" => self
                .service
                .application_workbench_page(number("page_index"), number("page_size"))?,
            "task_page" => self.service.application_download_task_page(
                field("task_id"),
                number("page_index"),
                number("page_size"),
            )?,
            "task_event_page" => self
                .service
                .application_download_task_event_page(field("task_id"), number("page_size"))?,
            "source_list" => compact_source_list(self.service.application_list_sources()?),
            "account_readiness" => {
                account_categories(&self.service.application_account_session_statuses()?)
            }
            "task_overview" => self.service.application_task_overview()?,
            "batch_page" => {
                let request: AgentBatchPageInput = serde_json::from_value(input.clone())
                    .map_err(|_| ErrorClass::InvalidParameters)?;
                self.service.application_batch_page(
                    &request.batch_id,
                    request.revision,
                    request.page,
                    request.page_size,
                )?
            }
            "search_progress" => self
                .service
                .application_search_progress(field("request_id"))?,
            _ => return Err(ErrorClass::Unsupported),
        };
        let safe = if tool.tool_id == "credential_readiness" {
            catalog::credential_categories(&summary)
        } else {
            crate::run::safe_value(&sanitize(&summary)).map_err(|_| ErrorClass::Permission)?
        };
        Ok(ToolResult {
            summary: safe,
            observed_at: observed_now()?,
            source: "application_service".into(),
        })
    }
    fn submit(&mut self, target_command: &str, input: &Value) -> Result<ToolResult, ErrorClass> {
        let tool = catalog::catalog()
            .tools
            .into_iter()
            .find(|tool| {
                tool.target_command == target_command
                    && matches!(
                        (tool.effect.as_str(), tool.confirmation.as_str()),
                        ("read_external", "disclose") | ("commit", "plan_confirm")
                    )
            })
            .ok_or(ErrorClass::Permission)?;
        if !bounded_input(&tool.tool_id, input) {
            return Err(ErrorClass::InvalidParameters);
        }
        let summary = match tool.tool_id.as_str() {
            "search_start" => {
                let request: insar_core::UnifiedSearchRequest =
                    serde_json::from_value(input.clone())
                        .map_err(|_| ErrorClass::InvalidParameters)?;
                self.service.application_search_start(
                    &request.resource_id,
                    &Value::Object(request.aoi),
                    &request.aoi_name,
                    &serde_json::to_value(&request.time_range)
                        .map_err(|_| ErrorClass::InvalidParameters)?,
                    &Value::Object(request.filters),
                    &request.request_id,
                )?
            }
            "working_aoi_set" => {
                let fallback_name = input
                    .get("display_name")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let raw = self.service.application_set_working_aoi(input)?;
                normalize_working_aoi_output(
                    crate::aoi::working_summary(&raw),
                    fallback_name.as_deref(),
                )
            }
            "create_task" => {
                let request: AgentCreateTaskInput = serde_json::from_value(input.clone())
                    .map_err(|_| ErrorClass::InvalidParameters)?;
                self.service.application_create_task(
                    &request.batch_id,
                    request.revision,
                    &request.item_ids,
                    &request.options,
                )?
            }
            "task_action" => {
                let request: AgentTaskActionInput = serde_json::from_value(input.clone())
                    .map_err(|_| ErrorClass::InvalidParameters)?;
                self.service.application_task_action(
                    &request.task_id,
                    request.revision,
                    &request.action,
                    &request.event_id,
                )?
            }
            "update_download_task" => self.service.application_update_download_task(input)?,
            _ => return Err(ErrorClass::Permission),
        };
        Ok(ToolResult {
            summary: crate::run::safe_value(&sanitize(&summary))
                .map_err(|_| ErrorClass::Permission)?,
            observed_at: observed_now()?,
            source: "application_service".into(),
        })
    }
    fn task_state(&mut self, task_id: &str, event_id: &str) -> Result<Option<Value>, ErrorClass> {
        if !scalar(task_id, 128) || !scalar(event_id, 192) {
            return Err(ErrorClass::InvalidParameters);
        }
        Ok(self
            .service
            .application_task_event_state(task_id, event_id)?
            .map(|value| sanitize(&value)))
    }
}

fn non_empty_str<'a>(obj: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn parse_bbox_4(val: Option<&Value>) -> Option<Value> {
    let arr = val?.as_array()?;
    if arr.len() != 4 {
        return None;
    }
    let nums: Vec<f64> = arr.iter().map(Value::as_f64).collect::<Option<Vec<_>>>()?;
    if nums.iter().all(|n| n.is_finite()) {
        Some(serde_json::json!(nums))
    } else {
        None
    }
}

fn parse_nominatim_boundingbox(val: Option<&Value>) -> Option<Value> {
    let arr = val?.as_array()?;
    if arr.len() != 4 {
        return None;
    }
    let parse_one = |v: &Value| -> Option<f64> {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
    };
    let s = parse_one(&arr[0])?;
    let n = parse_one(&arr[1])?;
    let w = parse_one(&arr[2])?;
    let e = parse_one(&arr[3])?;
    if [w, s, e, n].iter().all(|v| v.is_finite()) {
        Some(serde_json::json!([w, s, e, n]))
    } else {
        None
    }
}

fn normalize_boundary_item(item: &mut Value) {
    let Some(obj) = item.as_object_mut() else {
        return;
    };
    if non_empty_str(obj, &["display_name"]).is_none() {
        if let Some(name) =
            non_empty_str(obj, &["full_name", "name", "admin_name", "label"]).map(str::to_owned)
        {
            obj.insert("display_name".into(), Value::String(name));
        }
    }
    if parse_bbox_4(obj.get("bbox")).is_none() {
        let computed = obj
            .get("geometry")
            .and_then(Value::as_object)
            .and_then(|g| {
                let b = crate::aoi::bbox(Some(g));
                (!b.is_null()).then_some(b)
            })
            .or_else(|| parse_nominatim_boundingbox(obj.get("boundingbox")))
            .or_else(|| {
                let bounds = obj.get("bounds")?.as_object()?;
                let w = bounds.get("west").and_then(Value::as_f64)?;
                let s = bounds.get("south").and_then(Value::as_f64)?;
                let e = bounds.get("east").and_then(Value::as_f64)?;
                let n = bounds.get("north").and_then(Value::as_f64)?;
                Some(serde_json::json!([w, s, e, n]))
            });
        if let Some(bbox) = computed {
            obj.insert("bbox".into(), bbox);
        }
    }
    if let Some(bbox_arr) = parse_bbox_4(obj.get("bbox")).and_then(|v| {
        let arr = v.as_array()?;
        Some([
            arr.first()?.as_f64()?,
            arr.get(1)?.as_f64()?,
            arr.get(2)?.as_f64()?,
            arr.get(3)?.as_f64()?,
        ])
    }) {
        let area = obj
            .get("area_km2")
            .and_then(Value::as_f64)
            .or_else(|| crate::aoi::bbox_area_km2(bbox_arr));
        if let Some(area_km2) = area {
            if obj.get("area_km2").is_none_or(Value::is_null) {
                obj.insert("area_km2".into(), json!(area_km2));
            }
            if area_km2 > crate::aoi::DEFAULT_MAX_AOI_AREA_KM2 && !obj.contains_key("aoi_guard") {
                obj.insert(
                    "aoi_guard".into(),
                    json!({
                        "status": "oversized_warning",
                        "area_km2": area_km2,
                        "max_recommended_area_km2": crate::aoi::DEFAULT_MAX_AOI_AREA_KM2,
                        "warning_zh": format!(
                            "该边界覆盖面积约 {area_km2:.0} km²（超过建议单次检索上限 {:.0} km²），建议确认是否缩小到具体市/县或局部范围",
                            crate::aoi::DEFAULT_MAX_AOI_AREA_KM2
                        ),
                    }),
                );
            }
        }
    }
}

fn normalize_boundary_search_output(mut val: Value) -> Value {
    match &mut val {
        Value::Array(items) => {
            for item in items {
                normalize_boundary_item(item);
            }
        }
        Value::Object(_) => {
            normalize_boundary_item(&mut val);
            if let Some(map) = val.as_object_mut() {
                for key in ["items", "candidates", "results", "matches"] {
                    if let Some(Value::Array(items)) = map.get_mut(key) {
                        for item in items {
                            normalize_boundary_item(item);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    val
}

fn normalize_working_aoi_output(mut val: Value, fallback_name: Option<&str>) -> Value {
    normalize_boundary_item(&mut val);
    if let Some(obj) = val.as_object_mut() {
        if non_empty_str(obj, &["display_name"]).is_none() {
            if let Some(name) = fallback_name.map(str::trim).filter(|s| !s.is_empty()) {
                obj.insert("display_name".into(), Value::String(name.to_owned()));
            }
        }
    }
    val
}

fn compact_source_entry(item: Value) -> Value {
    let Value::Object(mut obj) = item else {
        return item;
    };
    let mut out = serde_json::Map::new();
    for key in [
        "resource_id",
        "display_name_zh",
        "display_name",
        "family",
        "availability",
    ] {
        if let Some(v) = obj.remove(key) {
            if !v.is_null() {
                out.insert(key.to_owned(), v);
            }
        }
    }
    if let Some(v) = obj.remove("availability_reason_zh") {
        if v.as_str().is_some_and(|s| !s.trim().is_empty()) {
            out.insert("availability_reason_zh".to_owned(), v);
        }
    }
    if let Some(v) = obj.remove("accounts") {
        if v.as_array().is_some_and(|a| !a.is_empty()) {
            out.insert("accounts".to_owned(), v);
        }
    }
    Value::Object(out)
}

fn compact_source_list(val: Value) -> Value {
    match val {
        Value::Array(items) => Value::Array(items.into_iter().map(compact_source_entry).collect()),
        Value::Object(mut obj) => {
            if let Some(Value::Array(items)) = obj.remove("sources") {
                Value::Array(items.into_iter().map(compact_source_entry).collect())
            } else {
                Value::Object(obj)
            }
        }
        other => other,
    }
}
