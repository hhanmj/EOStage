//! Desktop Agent host state: one `AgentHost` over the one ApplicationService.
//! Model channel metadata lives in a settings file under the app data
//! directory; model API keys live only in the system credential store and no
//! command projects them back.
use crate::error::ShellError;
pub use insar_agent::session::{
    AgentHostNotification, AgentHostNotifier, SessionTruncateResult, SessionWorkspaceState,
};
use insar_agent::{
    executor::ErrorClass,
    host::UnifiedApplication,
    session::{
        AgentHost, HostError, ModelSelection, ModelSource, OpenedModel, StoreAccess, TaskStore,
    },
};
use insar_agent_model::{
    agent::KeyedModel,
    block_on,
    channels::{
        is_reserved_channel_id, ChannelConfig, ChannelError, ChannelKeys, ChannelMemory,
        ChannelModel, ChannelStore,
    },
    transport::{ModelClient, ModelProxy},
    CancellationToken, Message, ModelError, Protocol, ReasoningMode, ToolDescription, Zeroizing,
};
use insar_application_service::{
    AgentApplication, NisarConfirmSubsetBatchItemV2, NisarConfirmSubsetBatchTaskV2Request,
    NisarPreflightSubsetBatchV2Request, NisarPrepareSubsetBatchV2Request,
};
use insar_core::{DownloadLayout, StoreBatchPageRequest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

pub type DesktopAgentHost = AgentHost<DesktopGeeAgentService, DesktopModels>;

const GEE_AGENT_SYSTEM_PROMPT: &str = "\
GEE 云端计算与分块下载套件（内置算子免 Python；若已设置工作区 AOI 可省略 `bbox`，勿输出本机路径或内部编号）：\n\
- 目录配数：`gee_catalog_search`（检索官方/社区目录）、`gee_dataset_advisor`（recommend 配数 / compare / verify）\n\
- 预检与计算：`gee_check_availability`（景数与分块）、`gee_export_preflight`（离线预算/可分块判定，PCA/RSEI 禁止独立分块）、`gee_cloud_analyze`（composite / index / zonal_stats / change_detect / time_series，自动生成主地图预览）、`gee_quota_status`\n\
- 分块导出与脚本：`gee_chunked_download`（<=9 块直接导出 Float32 GeoTIFF，10..=64 块需用户确认，>64 块拒绝且禁止静默降采样）、`gee_script_inspect`（自定义 `ee` 脚本审查与批准运行）\n\
- 本地 GIS 联动：`gis_toolchains_status` / `gis_local_process` / `gis_external_execute`（内置 GDAL/PROJ 或受控外部工程包）";

#[derive(Clone)]
pub struct DesktopGeeAgentService {
    pub inner: AgentApplication,
}

impl DesktopGeeAgentService {
    fn resolve_working_bbox(&self) -> Option<[f64; 4]> {
        let val = self.inner.application_working_aoi().ok()?;
        extract_bbox_from_aoi_json(&val)
    }
}

fn collect_lon_lat_points(val: &Value, pts: &mut Vec<(f64, f64)>) {
    if let Some(arr) = val.as_array() {
        if let (Some(lon), Some(lat)) = (
            arr.first().and_then(Value::as_f64),
            arr.get(1).and_then(Value::as_f64),
        ) {
            pts.push((lon, lat));
            return;
        }
        for item in arr {
            collect_lon_lat_points(item, pts);
        }
    } else if let Some(obj) = val.as_object() {
        for key in [
            "bbox",
            "coordinates",
            "geometry",
            "geojson",
            "aoi_geojson",
            "working_aoi",
        ] {
            if let Some(sub) = obj.get(key) {
                collect_lon_lat_points(sub, pts);
            }
        }
    }
}

fn extract_bbox_from_aoi_json(val: &Value) -> Option<[f64; 4]> {
    if let Some(arr) = val.get("bbox").and_then(Value::as_array) {
        if arr.len() == 4 {
            if let (Some(w), Some(s), Some(e), Some(n)) = (
                arr[0].as_f64(),
                arr[1].as_f64(),
                arr[2].as_f64(),
                arr[3].as_f64(),
            ) {
                if e > w && n > s {
                    return Some([w, s, e, n]);
                }
            }
        }
    }
    let mut pts = Vec::new();
    collect_lon_lat_points(val, &mut pts);
    if pts.is_empty() {
        return None;
    }
    let mut w = f64::INFINITY;
    let mut s = f64::INFINITY;
    let mut e = f64::NEG_INFINITY;
    let mut n = f64::NEG_INFINITY;
    for (lon, lat) in pts {
        w = w.min(lon);
        s = s.min(lat);
        e = e.max(lon);
        n = n.max(lat);
    }
    (e > w && n > s).then_some([w, s, e, n])
}

fn sum_assets_bytes(assets: &Value) -> Option<u64> {
    let mut total = 0u64;
    let mut any = false;
    let mut inspect = |entry: &Value| {
        for key in ["size_bytes", "bytes", "file_size", "size"] {
            if let Some(n) = entry.get(key).and_then(Value::as_u64) {
                total = total.saturating_add(n);
                any = true;
                break;
            }
        }
    };
    match assets {
        Value::Array(arr) => {
            for entry in arr {
                inspect(entry);
            }
        }
        Value::Object(map) => {
            for entry in map.values() {
                inspect(entry);
            }
        }
        _ => {}
    }
    any.then_some(total)
}

/// Strips multi-tens-of-kilobytes download asset manifests (`metadata.assets`, `asset_access`,
/// dense GeoJSON rings) from Agent-only `workbench_page` / `batch_page` projections while
/// preserving batch identity, total counts, `size_bytes`, and all scientific summary fields.
fn slim_agent_page_value(mut val: Value) -> Value {
    const KEPT_PROPERTIES: &[&str] = &[
        "beamModeType",
        "frameNumber",
        "orbit",
        "pathNumber",
        "polarization",
        "processingLevel",
        "platform",
        "sensor",
        "startTime",
        "stopTime",
        "flightDirection",
        "lookDirection",
        "cloudCover",
        "eo:cloud_cover",
        "sar:polarizations",
        "sar:instrument_mode",
        "sat:orbit_state",
        "sat:relative_orbit",
    ];

    if let Some(items) = val.get_mut("items").and_then(Value::as_array_mut) {
        for entry in items.iter_mut() {
            let target = if entry.get("item").is_some() {
                entry.get_mut("item")
            } else {
                Some(entry)
            };
            let Some(item_obj) = target.and_then(Value::as_object_mut) else {
                continue;
            };

            item_obj.remove("asset_access");
            item_obj.remove("execution_policy");

            if let Some(fp) = item_obj.remove("footprint_geojson") {
                if !item_obj.contains_key("bbox") {
                    if let Some([w, s, e, n]) = extract_bbox_from_aoi_json(&fp) {
                        item_obj.insert(
                            "bbox".into(),
                            json!([
                                (w * 1e5).round() / 1e5,
                                (s * 1e5).round() / 1e5,
                                (e * 1e5).round() / 1e5,
                                (n * 1e5).round() / 1e5,
                            ]),
                        );
                    }
                }
            }

            let payload_assets_bytes = item_obj
                .get_mut("payload")
                .and_then(Value::as_object_mut)
                .and_then(|p| p.remove("assets"))
                .and_then(|a| sum_assets_bytes(&a));

            if let Some(meta_obj) = item_obj.get_mut("metadata").and_then(Value::as_object_mut) {
                let meta_assets_bytes =
                    meta_obj.remove("assets").and_then(|a| sum_assets_bytes(&a));
                if !meta_obj.contains_key("size_bytes") {
                    if let Some(bytes) = meta_assets_bytes.or(payload_assets_bytes) {
                        meta_obj.insert("size_bytes".into(), json!(bytes));
                    }
                }
                if let Some(props) = meta_obj
                    .get_mut("properties")
                    .and_then(Value::as_object_mut)
                {
                    if props.len() > 12 {
                        props.retain(|k, _| KEPT_PROPERTIES.contains(&k.as_str()));
                    }
                }
            }
        }
    }
    val
}

impl StoreAccess for DesktopGeeAgentService {
    fn with_store<T>(&self, operation: impl FnOnce(&mut TaskStore) -> T) -> Result<T, HostError> {
        self.inner.with_store(operation)
    }
}

impl UnifiedApplication for DesktopGeeAgentService {
    fn application_service_readiness(&self) -> Result<Value, ErrorClass> {
        self.inner.application_service_readiness()
    }
    fn application_provider_capabilities(&self) -> Result<Value, ErrorClass> {
        self.inner.application_provider_capabilities()
    }
    fn application_working_aoi(&self) -> Result<Value, ErrorClass> {
        self.inner.application_working_aoi()
    }
    fn application_workbench_page(
        &self,
        page_index: u64,
        page_size: u64,
    ) -> Result<Value, ErrorClass> {
        self.inner
            .application_workbench_page(page_index, page_size)
            .map(slim_agent_page_value)
    }
    fn application_download_task_page(
        &self,
        task_id: &str,
        page_index: u64,
        page_size: u64,
    ) -> Result<Value, ErrorClass> {
        self.inner
            .application_download_task_page(task_id, page_index, page_size)
    }
    fn application_download_task_event_page(
        &self,
        task_id: &str,
        page_size: u64,
    ) -> Result<Value, ErrorClass> {
        self.inner
            .application_download_task_event_page(task_id, page_size)
    }
    fn application_credential_categories(&self) -> Result<Value, ErrorClass> {
        self.inner.application_credential_categories()
    }
    fn application_list_sources(&self) -> Result<Value, ErrorClass> {
        self.inner.application_list_sources()
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
        self.inner.application_search_start(
            resource_id,
            aoi,
            aoi_name,
            time_range,
            filters,
            request_id,
        )
    }
    fn application_search_progress(&self, request_id: &str) -> Result<Value, ErrorClass> {
        self.inner.application_search_progress(request_id)
    }
    fn application_batch_page(
        &self,
        batch_id: &str,
        revision: u64,
        page: u32,
        page_size: u32,
    ) -> Result<Value, ErrorClass> {
        self.inner
            .application_batch_page(batch_id, revision, page, page_size)
            .map(slim_agent_page_value)
    }
    fn application_create_task(
        &self,
        batch_id: &str,
        revision: u64,
        item_ids: &[String],
        options: &Value,
    ) -> Result<Value, ErrorClass> {
        self.inner
            .application_create_task(batch_id, revision, item_ids, options)
    }
    fn application_task_action(
        &self,
        task_id: &str,
        revision: u64,
        action: &str,
        event_id: &str,
    ) -> Result<Value, ErrorClass> {
        self.inner
            .application_task_action(task_id, revision, action, event_id)
    }
    fn application_update_download_task(&self, request: &Value) -> Result<Value, ErrorClass> {
        self.inner.application_update_download_task(request)
    }
    fn application_task_overview(&self) -> Result<Value, ErrorClass> {
        self.inner.application_task_overview()
    }
    fn application_account_session_statuses(&self) -> Result<Value, ErrorClass> {
        self.inner.application_account_session_statuses()
    }
    fn application_task_event_state(
        &self,
        task_id: &str,
        event_id: &str,
    ) -> Result<Option<Value>, ErrorClass> {
        self.inner.application_task_event_state(task_id, event_id)
    }
    fn application_default_output_root(&self) -> Result<Option<String>, ErrorClass> {
        self.inner.application_default_output_root()
    }
    fn application_search_batch(&self, request_id: &str) -> Result<Option<String>, ErrorClass> {
        self.inner.application_search_batch(request_id)
    }
    fn application_selection_accounts_ready(
        &self,
        batch_id: &str,
        item_ids: &[String],
    ) -> Result<bool, ErrorClass> {
        self.inner
            .application_selection_accounts_ready(batch_id, item_ids)
    }
    fn application_admin_boundary_search(&self, input: &Value) -> Result<Value, ErrorClass> {
        self.inner.application_admin_boundary_search(input)
    }
    fn application_preview_working_aoi_set(&self, input: &Value) -> Result<Value, ErrorClass> {
        self.inner.application_preview_working_aoi_set(input)
    }
    fn application_set_working_aoi(&self, input: &Value) -> Result<Value, ErrorClass> {
        self.inner.application_set_working_aoi(input)
    }
    fn application_restore_working_aoi(&self, snapshot: &Value) -> Result<Value, ErrorClass> {
        self.inner.application_restore_working_aoi(snapshot)
    }
    fn application_extension_system_prompt(&self) -> Option<String> {
        Some(build_desktop_extension_system_prompt(
            self.inner.data_root().as_deref(),
        ))
    }
    fn application_extension_tools(&self) -> Vec<ToolDescription> {
        let app_data_dir = self.inner.data_root();
        let gee_active = is_gee_plugin_active(app_data_dir.as_deref());
        let mut tools = Vec::with_capacity(18);
        if gee_active {
            tools.extend([
            ToolDescription {
                name: "gee_catalog_search".into(),
                description: "中英文检索 Google Earth Engine 官方(900+)与社区(4400+)数据集目录，支持来源·类型·分辨率过滤与退役排除，返回标准 asset_id、置信度与核验规则".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "query": { "type": "string", "description": "中文或英文关键词，如 哨兵2、夜间灯光、土地覆盖、ERA5、MODIS NDVI" },
                        "include_community": { "type": "boolean", "description": "是否同时检索社区数据集目录，默认 true" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 50 },
                        "source": { "type": "string", "enum": ["official", "community", "curated"], "description": "按目录来源筛选" },
                        "provider": { "type": "string", "description": "按数据发布机构关键词筛选" },
                        "kind": { "type": "string", "enum": ["image_collection", "image", "table"], "description": "按资产类型筛选" },
                        "max_resolution_m": { "type": "number", "minimum": 0.1, "description": "要求已知空间分辨率不粗于该值（米）" },
                        "include_deprecated": { "type": "boolean", "description": "是否包含已退役数据集，默认 false" }
                    },
                    "required": ["query"]
                }),
            },
            ToolDescription {
                name: "gee_dataset_advisor".into(),
                description: "按科研任务角色打包推荐 GEE 数据集组合（recommend）、多候选并排对比（compare）或核验资产目录与可选实时状态（verify）".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "mode": { "type": "string", "enum": ["recommend", "compare", "verify"], "description": "顾问工作模式：recommend 按任务角色配数 · compare 并排对比 · verify 核验资产" },
                        "task": { "type": "string", "description": "mode=recommend 时的中英文分析目标，如 洪涝风险评估、植被长势、城市热岛" },
                        "ids": { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 12, "description": "mode=compare 时待对比的至少 2 个数据集 ID" },
                        "asset_id": { "type": "string", "description": "mode=verify 时待核验的数据集 ID" },
                        "limit_per_role": { "type": "integer", "minimum": 1, "maximum": 5, "description": "recommend 模式下每个角色返回的最多候选数" },
                        "include_community": { "type": "boolean", "description": "是否包含社区目录候选，默认 true" },
                        "live": { "type": "boolean", "description": "mode=verify 时是否尝试联网检查公开 STAC 与账号可访问性，默认 false" }
                    },
                    "required": ["mode"]
                }),
            },
            ToolDescription {
                name: "gee_export_preflight".into(),
                description: "零配额离线导出预演门禁：核算研究区像元与分块规模、判定算法是否可安全分块（如 PCA 与 RSEI 禁止独立分块）、给出推荐导出路径与数据血缘摘要".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "asset_id": { "type": "string", "description": "GEE 数据集资产 ID" },
                        "bbox": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4, "description": "[west, south, east, north] 研究区范围；未传时尝试读取当前工作区 AOI" },
                        "scale_m": { "type": "number", "minimum": 1, "description": "目标空间分辨率（米）" },
                        "bands": { "type": "array", "items": { "type": "string" }, "description": "拟导出或分析的波段列表" },
                        "index_name": { "type": "string", "description": "可选光谱或雷达指数名称" },
                        "algorithm_kind": { "type": "string", "enum": ["per_pixel_index", "composite", "trend", "ml_inference", "band_export", "pca_rsei", "global_zonal_stats", "segmentation", "connected_components", "ml_training"], "description": "算法类型，用于判定是否适合独立网格分块" },
                        "destination": { "type": "string", "enum": ["local", "drive", "cloud_storage", "ee_asset"], "description": "计划导出目的地，默认 local" },
                        "start_date": { "type": "string", "description": "起始日期 YYYY-MM-DD" },
                        "end_date": { "type": "string", "description": "结束日期 YYYY-MM-DD" },
                        "max_cloud": { "type": "number", "description": "最大云量百分比" },
                        "reducer": { "type": "string", "description": "时间合成方式，如 median · mean · max" },
                        "qa_mask": { "type": "string", "description": "拟采用的去云或质量掩膜方式说明" },
                        "target_variable": { "type": "string", "description": "目标物理量或研究变量说明" },
                        "crs": { "type": "string", "description": "目标坐标系，默认 EPSG:4326" }
                    },
                    "required": ["asset_id"]
                }),
            },
            ToolDescription {
                name: "gee_check_availability".into(),
                description: "预检指定 GEE 数据集在研究区与时间段内的总景数、云量过滤后可用景数与分块规模".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "asset_id": { "type": "string", "description": "GEE 数据集 ID，如 COPERNICUS/S2_SR_HARMONIZED 或 LANDSAT/LC09/C02/T1_L2" },
                        "bbox": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4, "description": "[west, south, east, north] 经纬度范围；若工作台已有 AOI 可省略" },
                        "start_date": { "type": "string", "description": "起始日期 YYYY-MM-DD" },
                        "end_date": { "type": "string", "description": "结束日期 YYYY-MM-DD" },
                        "max_cloud": { "type": "number", "description": "最大云量百分比 0..100，如 20" },
                        "scale_m": { "type": "number", "description": "目标空间分辨率（米）" }
                    },
                    "required": ["asset_id"]
                }),
            },
            ToolDescription {
                name: "gee_cloud_analyze".into(),
                description: "执行零环境 GEE 云端遥感计算（无云合成、光谱/雷达指数、区域统计与阈值面积 km²、前后两期变化检测、时间序列曲线），并自动渲染对话预览图与主地图叠加层".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "asset_id": { "type": "string", "description": "GEE 数据集 ID，如 COPERNICUS/S2_SR_HARMONIZED" },
                        "operation": { "type": "string", "enum": ["composite", "index", "zonal_stats", "change_detect", "time_series"], "description": "计算类型" },
                        "index_name": { "type": "string", "enum": ["NDVI", "EVI", "NDWI", "MNDWI", "NDBI", "NBR", "BSI", "SAR_DIFF"], "description": "内置物理定标指数名称（index / change_detect / time_series / zonal_stats 可用）" },
                        "reducer": { "type": "string", "enum": ["median", "mean", "min", "max", "mosaic"], "description": "时间合成方式，默认 median" },
                        "bbox": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4, "description": "[west, south, east, north]；若已有 AOI 可省略" },
                        "start_date": { "type": "string", "description": "起始日期 YYYY-MM-DD（单期计算起点；若未提供 before_start_date，则亦作为 change_detect 前一期起点）" },
                        "end_date": { "type": "string", "description": "结束日期 YYYY-MM-DD（单期计算终点；若未提供 before_end_date，则亦作为 change_detect 前一期终点）" },
                        "before_start_date": { "type": "string", "description": "change_detect 前一期起始日期 YYYY-MM-DD（可选，未填时回退使用 start_date）" },
                        "before_end_date": { "type": "string", "description": "change_detect 前一期结束日期 YYYY-MM-DD（可选，未填时回退使用 end_date）" },
                        "after_start_date": { "type": "string", "description": "change_detect 后一期起始日期 YYYY-MM-DD" },
                        "after_end_date": { "type": "string", "description": "change_detect 后一期结束日期 YYYY-MM-DD" },
                        "max_cloud": { "type": "number", "description": "最大云量百分比，默认 30" },
                        "scale_m": { "type": "number", "description": "计算分辨率（米）" },
                        "threshold": { "type": "number", "description": "可选阈值（如统计 NDWI > 0.1 的水体面积 km²，或 NDVI > 0.4 的植被面积 km²）" },
                        "threshold_op": { "type": "string", "enum": ["gt", "lt"], "description": "阈值比较方向：gt(大于) 或 lt(小于)" },
                        "step_days": { "type": "integer", "minimum": 1, "maximum": 365, "description": "time_series 步长天数（按起止日期换算为 2..=24 个分段窗口）" },
                        "step_count": { "type": "integer", "minimum": 2, "maximum": 24, "description": "time_series 分段窗口数（未指定 step_days 时使用，默认 6）" }
                    },
                    "required": ["asset_id", "operation"]
                }),
            },
            ToolDescription {
                name: "gee_chunked_download".into(),
                description: "免 Google Drive 中转，按网格分块拉取 GEE 原始浮点数值并在本机无缝拼接为标准 Float32 GeoTIFF（<=9 块自动全分辨率；10..=64 块提示确认全分辨率或轻量预览分辨率；>64 块需缩小 AOI 或启用 use_preview_scale）".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "asset_id": { "type": "string", "description": "GEE 数据集 ID" },
                        "index_name": { "type": "string", "description": "可选：直接下载定标后的指数单波段（如 NDVI, NDWI）；留空则下载合成多波段" },
                        "bands": { "type": "array", "items": { "type": "string" }, "description": "可选：指定波段列表，如 [\"B4\", \"B3\", \"B2\"]" },
                        "reducer": { "type": "string", "enum": ["median", "mean", "min", "max", "mosaic"] },
                        "bbox": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 },
                        "start_date": { "type": "string" },
                        "end_date": { "type": "string" },
                        "max_cloud": { "type": "number" },
                        "scale_m": { "type": "number", "description": "请求空间分辨率（米），默认使用数据集原生分辨率" },
                        "confirm_full_resolution": { "type": "boolean", "description": "仅当分块数在 10..=64 且用户明确要求按原生全分辨率拼接时设为 true（超过 64 块仍会报错拒绝，绝不静默降采样）" },
                        "use_preview_scale": { "type": "boolean", "description": "仅当用户明确选择推荐预览分辨率时设为 true" }
                    },
                    "required": ["asset_id"]
                }),
            },
            ToolDescription {
                name: "gee_script_inspect".into(),
                description: "检查自定义 Python Earth Engine (`ee`) 脚本的安全性与遥感物理量纲，并在界面生成待用户确认运行的脚本卡片".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "script": { "type": "string", "description": "使用已初始化的 `ee`、`PROJECT_ID`、`AOI` (ee.Geometry.Rectangle)、`AOI_BBOX` / `BBOX` ([west, south, east, north])，可选设置 `PREVIEW_IMAGE` (ee.Image) 与 `VIS_PARAMS` 或直接将 PNG 写入 `PREVIEW_PNG_PATH`，并设置 `RESULT = {...}` 的 Python 脚本代码" },
                        "bbox": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 }
                    },
                    "required": ["script"]
                }),
            },
            ToolDescription {
                name: "gee_quota_status".into(),
                description: "查询当前 Google Earth Engine 项目的真实账号档位、月度 EECU-hours 算力上限、本机当月实测 EECU 账本，以及开启 Cloud Billing 后的云端全局已用与剩余算力".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "refresh": { "type": "boolean", "description": "是否跳过 120 秒内存缓存强制向 Google Cloud 实时刷新配额状态，默认 false" }
                    }
                }),
            },
            ]);
        }
        tools.extend([
            ToolDescription {
                name: "toolset_activate".into(),
                description: "按需激活专域工具包（gee / nisar / local_gis / mcp），避免首轮注入大量无关长 Schema；收到确认后下一轮即可直接调用解锁工具。".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["toolsets"],
                    "properties": {
                        "toolsets": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 4,
                            "items": {
                                "type": "string",
                                "enum": ["gee", "nisar", "local_gis", "mcp"]
                            },
                            "description": "要解锁的专域工具包名称列表"
                        }
                    }
                }),
            },
            ToolDescription {
                name: "gis_toolchains_status".into(),
                description: "探查内置 GDAL/PROJ 引擎、外部 GIS 工具链（RustGIS / GeoLibre / QGIS / ArcGIS Pro）、本机 Conda 科学环境以及 GIS/Conda 日志脱水组件与共享 RTK 状态".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "probe_versions": { "type": "boolean", "description": "是否探查各外部工具与 Conda Python 实时版本号（默认 false，快速静态检测）" }
                    }
                }),
            },
            ToolDescription {
                name: "gis_local_process".into(),
                description: "使用内置 GDAL/PROJ 引擎执行保精度栅格裁剪/重投影/镶嵌（支持 GeoTIFF 与 ENVI+.hdr）以及局部等距投影公制矢量缓冲/格式转换，结果隔离保存至受控输出目录".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "domain": { "type": "string", "enum": ["raster", "vector"], "description": "处理域：raster 或 vector" },
                        "operation": { "type": "string", "description": "栅格支持 clip/translate/reproject/mosaic/slope/aspect/hillshade/tpi/tri/roughness/ndvi/ndwi/nbr/band_math；矢量支持 buffer/convert" },
                        "sources": { "type": "array", "items": { "type": "string" }, "description": "栅格输入路径数组（mosaic/ndvi 可传多个）" },
                        "input_path": { "type": "string", "description": "矢量输入路径（GeoJSON/Shapefile/KML/KMZ）" },
                        "output_filename": { "type": "string", "description": "输出文件名（仅文件名，自动隔离保存到 gis_outputs 目录）" },
                        "crop_bbox": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 },
                        "target_crs": { "type": "string", "description": "目标坐标系，如 EPSG:32650 或 EPSG:4326" },
                        "resampling": { "type": "string", "enum": ["near", "bilinear", "cubic", "lanczos", "average", "mode"] },
                        "output_format": { "type": "string", "enum": ["GTiff", "ENVI"] },
                        "preserve_dtype": { "type": "boolean", "description": "是否保持输入栅格像元数据类型（clip/translate/reproject/mosaic 默认 true）" },
                        "band_a": { "type": "integer", "minimum": 1 },
                        "band_b": { "type": "integer", "minimum": 1 },
                        "z_factor": { "type": "number" },
                        "azimuth_deg": { "type": "number" },
                        "altitude_deg": { "type": "number" },
                        "distance_m": { "type": "number", "description": "矢量缓冲半径（单位：米，基于局部等距方位投影计算）" },
                        "quad_segs": { "type": "integer", "minimum": 1, "maximum": 64 }
                    },
                    "required": ["domain", "operation", "output_filename"]
                }),
            },
            ToolDescription {
                name: "gis_external_execute".into(),
                description: "按受控无头模式调用本机 RustGIS、GeoLibre、QGIS (qgis_process)、ArcGIS Pro (Windows isolated arcpy) 或用户 Conda 科学环境（package_probe / raster_inspect / raster_band_stats / vector_inspect / hdf5_inspect），支持一键生成 .qgs 交付包并自动对长日志执行脱水压缩".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "toolchain": { "type": "string", "enum": ["rustgis", "geolibre", "qgis", "arcgis_pro", "conda"], "description": "目标外部 GIS 或 Conda 科学工具链" },
                        "action": { "type": "string", "enum": ["version", "list_algorithms", "run_algorithm", "deliver_project"], "description": "执行动作" },
                        "algorithm": { "type": "string", "description": "算法标识（run_algorithm 必填）" },
                        "conda_env": { "type": "string", "description": "当 toolchain 为 conda 时可选指定环境名或 ID" },
                        "params": { "type": "object", "description": "算法键值参数对象（标量或简单数组）" },
                        "input_paths": { "type": "array", "items": { "type": "string" }, "description": "用于工程交付或 Conda 检视的输入栅格/矢量路径列表" },
                        "output_filename": { "type": "string", "description": "可选输出文件或交付目录名（隔离保存于 gis_outputs 目录）" },
                        "open_gui": { "type": "boolean", "description": "deliver_project 完成后是否尝试打开对应桌面软件（默认 false）" }
                    },
                    "required": ["toolchain", "action"]
                }),
            },
            ToolDescription {
                name: "skill_read".into(),
                description: "按需读取内置地学专业技能、教材级 Cookbook、自定义技能或已启用插件技能的完整 SKILL.md、references/*.md，或按关键词检索片段。执行专业遥感/InSAR/灾害任务前请优先加载对应技能。".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["skill_id"],
                    "properties": {
                        "skill_id": { "type": "string", "description": "技能 ID（如 gee-textbook-cookbook、sar-insar-textbook、nisar-rslc-subset、remote-sensing-analysis 等，与 <available_skills> 中的 id 一致）" },
                        "file_path": { "type": "string", "description": "单个相对文件路径，默认为 SKILL.md，也可传入如 references/authoritative-sources.md" },
                        "paths": {
                            "type": "array",
                            "items": { "type": "string" },
                            "maxItems": 5,
                            "description": "一次批量读取最多 5 个相对路径文件，节省交互轮次"
                        },
                        "query": { "type": "string", "description": "关键词（跨 SKILL.md 与 references/*.md 搜索匹配片段与文件名）" }
                    }
                }),
            },
            ToolDescription {
                name: "nisar_subset_preflight".into(),
                description: "对已检索到的 NISAR 数据批次（GCOV/GUNW/RSLC 等）执行客户端 HTTP Byte-Range V2 按研究区裁剪预检：校验空间相交、可用频段(Frequency)与极化(Polarization)组合，并计算全景体积 vs 研究区局部裁剪字节估算与节省比例。".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["source_batch_id"],
                    "properties": {
                        "source_batch_id": { "type": "string", "description": "search_start 返回的检索批次 ID（batch_id）" },
                        "source_item_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "可选：待预检的 NISAR 景 ID 列表；省略时自动取批次前 25 景"
                        },
                        "bbox": {
                            "type": "array",
                            "items": { "type": "number" },
                            "minItems": 4,
                            "maxItems": 4,
                            "description": "可选：[west, south, east, north] 研究区外包框；省略时自动读取当前工作区 AOI"
                        },
                        "aoi_geojson": { "type": "object", "description": "可选：GeoJSON Polygon/MultiPolygon/Feature 研究区多边形（优先级高于 bbox）" },
                        "selected_frequency": { "type": "string", "description": "可选：目标频段（如 A 或 B）；省略时自动选用公共可用频段" },
                        "selected_polarizations": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "可选：目标极化列表（如 [\"HH\", \"HV\"]）；省略时自动选用对应频段公共可用极化"
                        }
                    }
                }),
            },
            ToolDescription {
                name: "nisar_create_subset_task".into(),
                description: "提交真实 NISAR HTTP Byte-Range V2 按研究区局部裁剪下载任务（只拉取相交空间块与指定频段/极化 HDF5 数据集，避免整景下载）。提交前请确保研究区与参数已确认。".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["source_batch_id"],
                    "properties": {
                        "source_batch_id": { "type": "string", "description": "search_start 返回的检索批次 ID" },
                        "source_item_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "可选：指定裁剪下载的景 ID 列表；省略时使用批次中通过相交校验的景（最多 25 景）"
                        },
                        "bbox": {
                            "type": "array",
                            "items": { "type": "number" },
                            "minItems": 4,
                            "maxItems": 4,
                            "description": "可选：[west, south, east, north]；省略时自动读取当前工作区 AOI"
                        },
                        "aoi_geojson": { "type": "object", "description": "可选：GeoJSON Polygon/MultiPolygon/Feature" },
                        "selected_frequency": { "type": "string", "description": "可选：频段（如 A 或 B）；省略时自动选择公共可用频段" },
                        "selected_polarizations": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "可选：极化通道列表；省略时自动选择该频段公共极化"
                        },
                        "concurrency": { "type": "integer", "minimum": 1, "maximum": 8, "description": "并发连接数（1-8，默认 4）" },
                        "auto_start": { "type": "boolean", "description": "创建后是否立即启动真实 Range V2 下载（默认 true）" }
                    }
                }),
            },
            ToolDescription {
                name: "global_disaster_search".into(),
                description: "免账号实时检索联合国/欧盟 GDACS 全球活动灾害事件（地震 EQ、洪水 FL、台风/气旋 TC、火山 VO、野火 WF、干旱 DR），直接返回标准 WGS84 受灾包围盒 bbox、警报等级以及推荐的灾前/灾后影像检索时间窗。".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "event_types": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "可选：灾种代码或中英文名称列表，如 [\"EQ\", \"FL\"] 或 [\"地震\", \"洪水\", \"山火\"]；省略时默认查询 EQ/FL/TC/VO/WF"
                        },
                        "keyword": {
                            "type": "string",
                            "description": "可选：按事件名称、国家或地区关键词过滤（不区分大小写），如 China、Japan、California、Sichuan"
                        },
                        "min_alert_level": {
                            "type": "string",
                            "enum": ["green", "orange", "red"],
                            "description": "可选：最低警报等级（green 全部 / orange 橙色及以上 / red 仅红色重大灾害），默认 green"
                        },
                        "bbox": {
                            "type": "array",
                            "items": { "type": "number" },
                            "minItems": 4,
                            "maxItems": 4,
                            "description": "可选：[west, south, east, north] 空间相交过滤范围"
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 30,
                            "description": "最多返回事件条数（1-30，默认 10）"
                        }
                    }
                }),
            },
            ToolDescription {
                name: "aoi_validate_boundary".into(),
                description: "对候选地名边界或包围盒做双源交叉体检：校验几何是否退化、等积面积是否超出单次检索安全上限（默认 150000 km²），以及与参考地理坐标点的质心偏离（防止同名异地误框），不合规时返回安全回退包围盒与中文建议。".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "bbox": {
                            "type": "array",
                            "items": { "type": "number" },
                            "minItems": 4,
                            "maxItems": 4,
                            "description": "候选研究区 [west, south, east, north] 外包框；省略时自动检查当前工作区 AOI"
                        },
                        "reference_lat": {
                            "type": "number",
                            "description": "可选：参考地名或灾害震中纬度（WGS84，-90..90），用于同名异地质心漂移校验"
                        },
                        "reference_lon": {
                            "type": "number",
                            "description": "可选：参考地名或灾害震中经度（WGS84，-180..180）"
                        },
                        "max_area_km2": {
                            "type": "number",
                            "description": "可选：最大允许面积阈值（km²，默认 150000）"
                        },
                        "max_centroid_drift_km": {
                            "type": "number",
                            "description": "可选：质心与参考坐标最大允许偏差（km，默认 120）"
                        }
                    }
                }),
            },
            ToolDescription {
                name: "open_geodata_query".into(),
                description: "免账号直连开放地学数据源检索与下载工具，仅支持以下白名单公开数据源：AWS Open Data（aws_sentinel1_grd 哨兵1号 GRD COG、noaa_aws_weather NOAA GFS 0.25°/GOES 气象、overture_maps Overture 全球矢量 GeoParquet）、open_dem_tiles、whu_clcd、tum_eot20、gebco_2026、noaa_etopo_2022".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["dataset"],
                    "properties": {
                        "dataset": {
                            "type": "string",
                            "description": "白名单数据源标识：aws_sentinel1_grd / noaa_aws_weather / overture_maps / open_dem_tiles / whu_clcd / tum_eot20 / gebco_2026 / noaa_etopo_2022"
                        },
                        "bbox": {
                            "type": "array",
                            "items": { "type": "number" },
                            "minItems": 4,
                            "maxItems": 4,
                            "description": "可选：[west, south, east, north] WGS84 空间范围"
                        },
                        "start_date": { "type": "string", "description": "可选：起始日期 YYYY-MM-DD" },
                        "end_date": { "type": "string", "description": "可选：截止日期 YYYY-MM-DD" },
                        "keyword": { "type": "string", "description": "可选：检索筛选关键词（如极化 VV/VH、GFS 变量 TMP/UGRD、Overture 主题 places/buildings）" },
                        "dem_product": { "type": "string", "description": "可选：子产品或模式（如 gfs / goes19 / buildings / places / cop30）" },
                        "osm_tag": { "type": "string", "description": "可选：下载指定条目 item_id" },
                        "tool_name": { "type": "string", "description": "可选：动作类型（默认检索；download / crop / full / export_geojson 时触发落盘下载）" },
                        "output_path": { "type": "string", "description": "可选：建议输出文件名（宿主机将受控落盘于工作区开放数据目录内）" }
                    }
                }),
            },
        ]);
        if let Some(dir) = app_data_dir.as_deref() {
            let rt_plugins = crate::application::discover_runtime_plugins_dir();
            let connectors =
                crate::application::load_effective_mcp_connectors_at(dir, &rt_plugins);
            if !connectors.is_empty() {
                let mut mcp_tools = crate::mcp_client::cached_active_extension_tools(&connectors);
                for td in &mut mcp_tools {
                    td.description = insar_agent::history::outbound_text(&td.description);
                }
                mcp_tools.sort_by(|a, b| a.name.cmp(&b.name));
                tools.extend(mcp_tools);
            }
        }
        tools
    }
    fn application_call_extension_tool(
        &self,
        tool_id: &str,
        input: &Value,
    ) -> Option<Result<Value, ErrorClass>> {
        if tool_id.starts_with("mcp__") {
            let dir = self.inner.data_root()?;
            let rt_plugins = crate::application::discover_runtime_plugins_dir();
            let connectors =
                crate::application::load_effective_mcp_connectors_at(&dir, &rt_plugins);
            return crate::mcp_client::call_connector_tool(&connectors, tool_id, input).map(Ok);
        }
        if tool_id == "skill_read" {
            let app_data_dir = self.inner.data_root();
            let result = execute_skill_read(app_data_dir.as_deref(), input);
            return Some(Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(
                &result,
            )));
        }
        if tool_id == "global_disaster_search" {
            let result = self.execute_global_disaster_search(input);
            return Some(Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(
                &result,
            )));
        }
        if tool_id == "aoi_validate_boundary" {
            let result = self.execute_aoi_validate_boundary(input);
            return Some(Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(
                &result,
            )));
        }
        if tool_id == "nisar_subset_preflight" {
            let result = self.execute_nisar_subset_preflight(input);
            return Some(Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(
                &result,
            )));
        }
        if tool_id == "nisar_create_subset_task" {
            let Some(app_data_dir) = self.inner.data_root() else {
                return Some(Err(ErrorClass::Internal));
            };
            let result = self.execute_nisar_create_subset_task(&app_data_dir, input);
            return Some(Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(
                &result,
            )));
        }
        if tool_id == "open_geodata_query" {
            let Some(app_data_dir) = self.inner.data_root() else {
                return Some(Err(ErrorClass::Internal));
            };
            return Some(Ok(
                self.execute_open_geodata_query_extension(&app_data_dir, input)
            ));
        }
        if matches!(
            tool_id,
            "gis_toolchains_status" | "gis_local_process" | "gis_external_execute"
        ) {
            return Some(self.call_gis_extension_tool(tool_id, input));
        }
        if !matches!(
            tool_id,
            "gee_catalog_search"
                | "gee_dataset_advisor"
                | "gee_export_preflight"
                | "gee_check_availability"
                | "gee_cloud_analyze"
                | "gee_chunked_download"
                | "gee_script_inspect"
                | "gee_quota_status"
        ) {
            return None;
        }
        let Some(app_data_dir) = self.inner.data_root() else {
            return Some(Err(ErrorClass::Internal));
        };
        if !is_gee_plugin_active(Some(&app_data_dir)) {
            return Some(Ok(json!({
                "ok": false,
                "tool_id": tool_id,
                "error": "AGENT_PLUGIN_DISABLED",
                "message": "Google Earth Engine 插件尚未安装或已停用，请在「扩展 -> 插件」中安装并启用 eostage-gee-plugin 后重试。",
            })));
        }
        let is_download_tool = matches!(tool_id, "gee_chunked_download" | "gee_download");
        let sanitized_input = if is_download_tool {
            strip_agent_supplied_output_fields(input)
        } else {
            input.clone()
        };
        let host_output_path = if is_download_tool {
            let configured_root = self.inner.application_default_output_root().ok().flatten();
            let asset_id = sanitized_input
                .get("asset_id")
                .and_then(Value::as_str)
                .unwrap_or("gee_mosaic");
            let index_name = sanitized_input.get("index_name").and_then(Value::as_str);
            let unique_token = next_gee_agent_download_token();
            let cand = allocate_safe_gee_agent_download_path(
                configured_root.as_deref(),
                &app_data_dir,
                asset_id,
                index_name,
                &unique_token,
            );
            if cand
                .parent()
                .and_then(|p| std::fs::symlink_metadata(p).ok())
                .is_some_and(|m| m.file_type().is_symlink())
                || std::fs::symlink_metadata(&app_data_dir)
                    .ok()
                    .is_some_and(|m| m.file_type().is_symlink())
            {
                return Some(Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(
                    &json!({
                        "ok": false,
                        "tool_id": tool_id,
                        "error": "UNSAFE_GEOTIFF_OUTPUT_PATH: 拒绝通过符号链接目录写入目标路径",
                    }),
                )));
            }
            Some(cand)
        } else {
            None
        };
        let host_output_path_str = host_output_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());

        let input_bbox = sanitized_input
            .get("bbox")
            .and_then(|v| serde_json::from_value::<[f64; 4]>(v.clone()).ok());
        let effective_bbox = input_bbox.or_else(|| self.resolve_working_bbox());
        let start_date = sanitized_input.get("start_date").and_then(Value::as_str);
        let end_date = sanitized_input.get("end_date").and_then(Value::as_str);

        let started = std::time::Instant::now();
        let raw_outcome = crate::gee_cloud_suite::dispatch_gee_geodata_query(
            &app_data_dir,
            tool_id,
            None,
            effective_bbox,
            start_date,
            end_date,
            host_output_path_str.as_deref(),
            Some(&sanitized_input),
        );

        let trace_safe = match raw_outcome {
            Ok(mut val) => {
                if is_download_tool {
                    let record_req = crate::geodata_tools::OpenGeodataQueryRequest {
                        dataset: tool_id.to_owned(),
                        bbox: effective_bbox,
                        start_date: start_date.map(str::to_owned),
                        end_date: end_date.map(str::to_owned),
                        output_path: None,
                        gee_params: Some(sanitized_input),
                        ..Default::default()
                    };
                    crate::geodata_tools::maybe_record_geodata_download_task_on_service(
                        &self.inner.0,
                        &record_req,
                        &mut val,
                        started.elapsed(),
                    );
                }
                crate::gee_cloud_suite::sanitize_for_agent_trace(&val)
            }
            Err(err_msg) => crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                "ok": false,
                "tool_id": tool_id,
                "error": err_msg,
            })),
        };
        Some(Ok(trace_safe))
    }
}

const BUILTIN_SKILLS_MANIFEST_JSON: &str = include_str!("../../ui/public/skills/manifest.json");
include!(concat!(env!("OUT_DIR"), "/embedded_builtin_skills.rs"));

fn embedded_builtin_skill_files(skill_id: &str) -> Vec<String> {
    EMBEDDED_BUILTIN_SKILLS
        .iter()
        .filter(|(id, _, _)| *id == skill_id)
        .map(|(_, rel, _)| (*rel).to_owned())
        .collect()
}

fn read_embedded_builtin_skill_file(skill_id: &str, rel_path: &str) -> Option<&'static str> {
    EMBEDDED_BUILTIN_SKILLS
        .iter()
        .find(|(id, rel, _)| *id == skill_id && *rel == rel_path)
        .map(|(_, _, content)| *content)
}

#[derive(Debug, Deserialize)]
struct BuiltinSkillsManifest {
    #[serde(default)]
    skills: Vec<BuiltinSkillManifestEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuiltinSkillManifestEntry {
    id: String,
    #[serde(default)]
    name_zh: String,
    #[serde(default)]
    slash_command: String,
    #[serde(default)]
    files: Vec<BuiltinSkillManifestFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuiltinSkillManifestFile {
    path: String,
}

fn builtin_skill_entries() -> Vec<BuiltinSkillManifestEntry> {
    serde_json::from_str::<BuiltinSkillsManifest>(BUILTIN_SKILLS_MANIFEST_JSON)
        .map(|m| m.skills)
        .unwrap_or_default()
}

pub fn is_gee_plugin_active(app_data_dir: Option<&Path>) -> bool {
    let Some(data_dir) = app_data_dir else {
        return false;
    };
    let plugin_dir = data_dir.join("plugins").join("eostage-gee-plugin");
    plugin_dir.join("plugin.json").is_file() && !plugin_dir.join(".disabled").exists()
}

fn build_desktop_extension_system_prompt(app_data_dir: Option<&Path>) -> String {
    let mut prompt = String::with_capacity(16384);
    let gee_active = is_gee_plugin_active(app_data_dir);
    if gee_active {
        prompt.push_str(GEE_AGENT_SYSTEM_PROMPT);
        prompt.push_str("\n\n");
    }
    prompt.push_str(
        "<available_toolsets>\n\
         专域工具按需解锁（命中关键词自动开启，或调用 `toolset_activate`）：\n\
         - `nisar`: `nisar_subset_preflight` / `nisar_create_subset_task`\n\
         - `local_gis`: `gis_toolchains_status` / `gis_local_process` / `gis_external_execute`\n\
         - `mcp`: `mcp__*`\n",
    );
    if gee_active {
        prompt.push_str("- `gee`: GEE 算子已就绪\n");
    } else {
        prompt.push_str("- `gee`: 需在「扩展 -> 插件」启用 `eostage-gee-plugin`\n");
    }
    prompt.push_str("</available_toolsets>\n\n");
    prompt.push_str(
        "## NISAR Range V2 规则\n\
         - `nisar_gcov`、`nisar_gunw`、`nisar_rslc`、`nisar_gslc` 支持 `remote_range` 按研究区局部裁剪下载；严禁声称无法裁剪。\n\
         - `nisar_subset_preflight` 仅返回 `pre_hdf5_metadata_estimate`（`hdf5_chunk_extraction_verified: false`），不得夸大为已验证分块；先 `search_start` 再预检，并调用 `nisar_create_subset_task`。\n\n",
    );
    let gee_flood_hint = if gee_active {
        " 或 `gee_cloud_analyze`"
    } else {
        ""
    };
    prompt.push_str(&format!(
        "## Disaster Response Playbooks\n\
         - 灾害定位：未知坐标时先用 `global_disaster_search`，过大/重名行政区先 `aoi_validate_boundary` 再 `working_aoi_set`。\n\
         - 地震/洪涝/山火/滑坡：按场景组合 Sentinel-1/NISAR（走 Range V2）、Sentinel-2/Landsat{gee_flood_hint} 与 DEM。\n\n"
    ));
    let disabled_skills: std::collections::HashSet<String> = app_data_dir
        .map(|d| {
            load_disabled_items_at(d)
                .disabled_skills
                .into_iter()
                .collect()
        })
        .unwrap_or_default();
    // Deduplicate skills deterministically with strict precedence:
    // custom (2) > plugin (1, sorted by plugin.id ascending) > builtin (0).
    // Iterating the BTreeMap emits exactly one authoritative entry per skill_id
    // in stable byte-identical order to maximize prompt-cache hit rate.
    let mut resolved_skills: std::collections::BTreeMap<String, (u8, String)> =
        std::collections::BTreeMap::new();
    for skill in builtin_skill_entries() {
        if disabled_skills.contains(&skill.id) {
            continue;
        }
        let line = format!(
            "- `{}` ({}, {})\n",
            skill.id, skill.name_zh, skill.slash_command
        );
        resolved_skills.insert(skill.id.clone(), (0, line));
    }
    if let Some(data_dir) = app_data_dir {
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        if let Ok(mut status) = crate::application::inspect_plugins_status_at(data_dir, &rt_plugins)
        {
            status.installed_plugins.sort_by(|a, b| a.id.cmp(&b.id));
            for plugin in status.installed_plugins {
                if !plugin.enabled {
                    continue;
                }
                let mut bundled = plugin.bundled_skills;
                bundled.sort_by(|a, b| a.id.cmp(&b.id));
                for skill in bundled {
                    if disabled_skills.contains(&skill.id) {
                        continue;
                    }
                    // First plugin (alphabetically by plugin.id) wins among plugins,
                    // matching resolve_skill_dir_and_source.
                    if resolved_skills
                        .get(&skill.id)
                        .is_some_and(|(prio, _)| *prio >= 1)
                    {
                        continue;
                    }
                    let safe_name = insar_agent::history::outbound_text(skill.name.trim());
                    let safe_desc = insar_agent::history::outbound_text(skill.description.trim());
                    let line = format!(
                        "- `{}` ({}, resolved_source=plugin:{}): {}\n",
                        skill.id, safe_name, plugin.id, safe_desc
                    );
                    resolved_skills.insert(skill.id, (1, line));
                }
            }
        }
        let mut custom_skills = crate::application::load_custom_skills_at(data_dir);
        custom_skills.sort_by(|a, b| a.id.cmp(&b.id));
        for custom in custom_skills {
            if !custom.enabled || disabled_skills.contains(&custom.id) {
                continue;
            }
            let safe_name = insar_agent::history::outbound_text(custom.name.trim());
            let safe_slash = insar_agent::history::outbound_text(custom.slash_command.trim());
            let safe_desc = insar_agent::history::outbound_text(custom.description.trim());
            let line = format!(
                "- `{}` ({}, {}, resolved_source=custom): {}\n",
                custom.id, safe_name, safe_slash, safe_desc
            );
            resolved_skills.insert(custom.id, (2, line));
        }
    }
    prompt.push_str("<available_skills>\n");
    prompt.push_str("需要专业遥感/InSAR/GIS/灾害规程时先调用 `skill_read(skill_id)`：\n");
    for (_, (_, line)) in resolved_skills {
        prompt.push_str(&line);
    }
    prompt.push_str("</available_skills>");
    prompt
}

fn is_safe_skill_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.contains("..")
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains('\0')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn sanitize_skill_rel_path(raw: Option<&str>) -> Result<PathBuf, &'static str> {
    let trimmed = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("SKILL.md");
    if trimmed.contains('\0') || trimmed.contains('\\') || trimmed.starts_with('/') {
        return Err("INVALID_SKILL_FILE_PATH");
    }
    let mut rel = PathBuf::new();
    for comp in Path::new(trimmed).components() {
        match comp {
            std::path::Component::Normal(seg) => rel.push(seg),
            _ => return Err("INVALID_SKILL_FILE_PATH"),
        }
    }
    if rel.as_os_str().is_empty() {
        return Err("INVALID_SKILL_FILE_PATH");
    }
    Ok(rel)
}

fn candidate_builtin_skills_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    roots.push(manifest_dir.join("../ui/public/skills"));
    roots.push(manifest_dir.join("../ui/dist/skills"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.join("skills"));
            roots.push(parent.join("resources").join("skills"));
            roots.push(parent.join("../Resources/skills"));
        }
    }
    roots
}

const MAX_SKILL_READ_BYTES: usize = 128 * 1024;

fn truncate_skill_utf8(raw: &[u8]) -> String {
    let truncated = raw.len() > MAX_SKILL_READ_BYTES;
    let slice = if truncated {
        let mut end = MAX_SKILL_READ_BYTES.min(raw.len());
        while end > 0 && (raw[end] & 0b1100_0000) == 0b1000_0000 {
            end -= 1;
        }
        &raw[..end]
    } else {
        raw
    };
    let text = String::from_utf8_lossy(slice).into_owned();
    if truncated {
        format!("{text}\n\n[truncated at {MAX_SKILL_READ_BYTES} bytes]")
    } else {
        text
    }
}

fn read_contained_utf8_file(base_dir: &Path, rel_path: &Path) -> Result<Option<String>, String> {
    let joined = base_dir.join(rel_path);
    if !joined.exists() {
        return Ok(None);
    }
    let meta = std::fs::symlink_metadata(&joined).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("UNSAFE_SKILL_FILE_TYPE".to_owned());
    }
    let canon_base = std::fs::canonicalize(base_dir).map_err(|e| e.to_string())?;
    let canon_target = std::fs::canonicalize(&joined).map_err(|e| e.to_string())?;
    if !canon_target.starts_with(&canon_base) {
        return Err("SKILL_PATH_TRAVERSAL_BLOCKED".to_owned());
    }
    let raw = std::fs::read(&canon_target).map_err(|e| e.to_string())?;
    Ok(Some(truncate_skill_utf8(&raw)))
}

fn collect_relative_skill_files(base_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(base: &Path, cur: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(cur) else {
            return;
        };
        let mut sorted: Vec<_> = entries.flatten().collect();
        sorted.sort_by_key(|e| e.file_name());
        for entry in sorted {
            let p = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if p.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with('.') {
                    walk(base, &p, out);
                }
            } else if p.is_file() {
                if let Ok(rel) = p.strip_prefix(base) {
                    let s = rel.to_string_lossy().replace('\\', "/");
                    if s != "skill.json" {
                        out.push(s);
                    }
                }
            }
        }
    }
    walk(base_dir, base_dir, &mut out);
    out
}

/// Prepares Markdown snippets/contents from `skill_read` for inclusion in an outbound
/// `tool` result JSON string without collapsing LaTeX/escape backslashes or URL lines
/// followed by Markdown headings (`\n\n###`) into `"[local]"`.
fn sanitize_skill_markdown_for_tool_result(raw: &str) -> String {
    let scrubbed = insar_agent::history::outbound_text(raw);
    if insar_agent::outbound::nested_value(&json!(scrubbed)) != json!("[local]") {
        return scrubbed;
    }
    let mut normalized = String::with_capacity(scrubbed.len() + 64);
    for (idx, line) in scrubbed.lines().enumerate() {
        if idx > 0 {
            normalized.push('\n');
        }
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                while chars.peek() == Some(&'\\') {
                    chars.next();
                }
                if chars
                    .peek()
                    .is_some_and(|next| next.is_ascii_alphanumeric())
                {
                    continue;
                }
                normalized.push(' ');
                continue;
            }
            normalized.push(c);
        }
        // Keep trailing space after lines containing `://` so JSON-escaped `\n###` cannot
        // glue onto the preceding URL word and trip `run::safe_value`'s `#fragment` check.
        if line.contains("://") && !normalized.ends_with(' ') {
            normalized.push(' ');
        }
    }
    let re_scrubbed = insar_agent::history::outbound_text(&normalized);
    if insar_agent::outbound::nested_value(&json!(re_scrubbed)) != json!("[local]") {
        return re_scrubbed;
    }
    normalized
}

fn resolve_skill_text_content(
    skill_dir: &Path,
    source_label: &str,
    skill_id: &str,
    rel_path: &Path,
    rel_str: &str,
    inline_fallback: Option<&str>,
) -> Result<Option<String>, String> {
    if skill_dir.is_dir() {
        if let Some(content) = read_contained_utf8_file(skill_dir, rel_path)? {
            return Ok(Some(content));
        }
    }
    if source_label == "builtin" {
        if let Some(embedded) = read_embedded_builtin_skill_file(skill_id, rel_str) {
            return Ok(Some(truncate_skill_utf8(embedded.as_bytes())));
        }
    }
    if rel_str == "SKILL.md" {
        if let Some(content) = inline_fallback {
            return Ok(Some(truncate_skill_utf8(content.as_bytes())));
        }
    }
    Ok(None)
}

fn search_skill_matches(
    skill_dir: &Path,
    source_label: &str,
    skill_id: &str,
    files: &[String],
    inline_fallback: Option<&str>,
    query: &str,
) -> Vec<Value> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for rel_str in files {
        let Ok(rel_path) = sanitize_skill_rel_path(Some(rel_str)) else {
            continue;
        };
        let Ok(Some(text)) = resolve_skill_text_content(
            skill_dir,
            source_label,
            skill_id,
            &rel_path,
            rel_str,
            inline_fallback,
        ) else {
            continue;
        };
        for (idx, line) in text.lines().enumerate() {
            if line.to_ascii_lowercase().contains(&needle) {
                let raw_snippet: String = line.trim().chars().take(220).collect();
                let snippet = sanitize_skill_markdown_for_tool_result(&raw_snippet);
                hits.push(json!({
                    "file_path": rel_str,
                    "line": idx + 1,
                    "snippet": snippet,
                }));
                if hits.len() >= 8 {
                    return hits;
                }
            }
        }
    }
    if hits.is_empty() {
        if let Some(inline_md) = inline_fallback {
            for (idx, line) in inline_md.lines().enumerate() {
                if line.to_ascii_lowercase().contains(&needle) {
                    let raw_snippet: String = line.trim().chars().take(220).collect();
                    hits.push(json!({
                        "file_path": "SKILL.md",
                        "line": idx + 1,
                        "snippet": sanitize_skill_markdown_for_tool_result(&raw_snippet),
                    }));
                    if hits.len() >= 8 {
                        break;
                    }
                }
            }
        }
    }
    hits
}

fn resolve_skill_dir_and_source(
    app_data_dir: Option<&Path>,
    skill_id: &str,
) -> Option<(PathBuf, String, Vec<String>, Option<String>)> {
    if let Some(data_dir) = app_data_dir {
        if let Some(custom) = crate::application::load_custom_skills_at(data_dir)
            .into_iter()
            .find(|s| s.id == skill_id && s.enabled)
        {
            let dir = data_dir.join("skills_custom").join(&custom.id);
            let files = if dir.is_dir() {
                let collected = collect_relative_skill_files(&dir);
                if collected.is_empty() {
                    vec!["SKILL.md".to_owned()]
                } else {
                    collected
                }
            } else {
                vec!["SKILL.md".to_owned()]
            };
            return Some((
                dir,
                "custom".to_owned(),
                files,
                Some(format!(
                    "# {}\n\n{}\n\n{}\n",
                    custom.name, custom.description, custom.prompt
                )),
            ));
        }
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        if let Ok(mut status) = crate::application::inspect_plugins_status_at(data_dir, &rt_plugins)
        {
            status.installed_plugins.sort_by(|a, b| a.id.cmp(&b.id));
            for plugin in status.installed_plugins {
                if !plugin.enabled {
                    continue;
                }
                if let Some(bundled) = plugin.bundled_skills.iter().find(|s| s.id == skill_id) {
                    let skill_dir = PathBuf::from(&plugin.install_path)
                        .join("skills")
                        .join(&bundled.id);
                    let files = if skill_dir.is_dir() {
                        collect_relative_skill_files(&skill_dir)
                    } else {
                        vec!["SKILL.md".to_owned()]
                    };
                    return Some((
                        skill_dir,
                        format!("plugin:{}", plugin.id),
                        files,
                        (!bundled.prompt.trim().is_empty()).then(|| bundled.prompt.clone()),
                    ));
                }
            }
        }
    }

    let manifest_entry = builtin_skill_entries()
        .into_iter()
        .find(|s| s.id == skill_id);
    let manifest_files: Vec<String> = manifest_entry
        .as_ref()
        .map(|m| m.files.iter().map(|f| f.path.clone()).collect())
        .unwrap_or_default();
    for root in candidate_builtin_skills_roots() {
        let skill_dir = root.join(skill_id);
        if skill_dir.is_dir() {
            let files = if manifest_files.is_empty() {
                collect_relative_skill_files(&skill_dir)
            } else {
                manifest_files
            };
            return Some((skill_dir, "builtin".to_owned(), files, None));
        }
    }
    let embedded_files = embedded_builtin_skill_files(skill_id);
    if !embedded_files.is_empty() {
        let files = if manifest_files.is_empty() {
            embedded_files
        } else {
            manifest_files
        };
        return Some((PathBuf::new(), "builtin".to_owned(), files, None));
    }
    None
}

fn execute_skill_read(app_data_dir: Option<&Path>, input: &Value) -> Value {
    let skill_id = input
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if !is_safe_skill_id(skill_id) {
        return json!({
            "ok": false,
            "error": "INVALID_SKILL_ID",
            "message": "skill_id 无效或包含非法路径字符",
        });
    }
    if let Some(data_dir) = app_data_dir {
        let disabled = load_disabled_items_at(data_dir);
        if disabled.disabled_skills.iter().any(|id| id == skill_id) {
            return json!({
                "ok": false,
                "skill_id": skill_id,
                "error": "SKILL_DISABLED",
                "message": "该技能已在设置或技能中心中停用",
            });
        }
    }

    let Some((skill_dir, source_label, available_files, inline_fallback)) =
        resolve_skill_dir_and_source(app_data_dir, skill_id)
    else {
        if let Some(data_dir) = app_data_dir {
            let rt_plugins = crate::application::discover_runtime_plugins_dir();
            let plugins =
                crate::application::load_installed_plugin_packages_at(data_dir, &rt_plugins);
            if let Some(disabled_owner) = plugins.iter().find(|plugin| {
                !plugin.enabled && plugin.bundled_skills.iter().any(|sk| sk.id == skill_id)
            }) {
                return json!({
                    "ok": false,
                    "skill_id": skill_id,
                    "plugin_id": disabled_owner.id,
                    "error": "PLUGIN_DISABLED",
                    "message": format!("技能 `{skill_id}` 所属插件 `{}` 已停用，请先启用该插件", disabled_owner.name),
                });
            }
        }
        return json!({
            "ok": false,
            "skill_id": skill_id,
            "error": "SKILL_FILE_NOT_FOUND",
            "message": format!("未找到技能 `{skill_id}`"),
        });
    };

    if let Some(query) = input
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let matches = search_skill_matches(
            &skill_dir,
            &source_label,
            skill_id,
            &available_files,
            inline_fallback.as_deref(),
            query,
        );
        return json!({
            "ok": true,
            "skill_id": skill_id,
            "source": source_label,
            "query": query,
            "available_files": available_files,
            "matches": matches,
        });
    }

    if let Some(paths_arr) = input
        .get("paths")
        .and_then(Value::as_array)
        .filter(|arr| !arr.is_empty())
    {
        let mut files_out = Vec::new();
        let mut missing_files = Vec::new();
        for item in paths_arr.iter().take(5) {
            let Some(raw_p) = item.as_str() else {
                return json!({
                    "ok": false,
                    "skill_id": skill_id,
                    "error": "INVALID_SKILL_FILE_PATH",
                    "available_files": available_files,
                });
            };
            let rel_path = match sanitize_skill_rel_path(Some(raw_p)) {
                Ok(r) => r,
                Err(code) => {
                    return json!({
                        "ok": false,
                        "skill_id": skill_id,
                        "error": code,
                        "available_files": available_files,
                    });
                }
            };
            let rel_str = rel_path.to_string_lossy().replace('\\', "/");
            match resolve_skill_text_content(
                &skill_dir,
                &source_label,
                skill_id,
                &rel_path,
                &rel_str,
                inline_fallback.as_deref(),
            ) {
                Ok(Some(content)) => {
                    let bytes = content.len();
                    files_out.push(json!({
                        "file_path": rel_str,
                        "path": rel_str,
                        "bytes": bytes,
                        "content": sanitize_skill_markdown_for_tool_result(&content),
                    }));
                }
                Err(err) => {
                    return json!({
                        "ok": false,
                        "skill_id": skill_id,
                        "error": err,
                        "available_files": available_files,
                    });
                }
                Ok(None) => {
                    missing_files.push(rel_str);
                }
            }
        }
        if files_out.is_empty() {
            return json!({
                "ok": false,
                "skill_id": skill_id,
                "source": source_label,
                "error": "SKILL_FILE_NOT_FOUND",
                "missing_files": missing_files,
                "available_files": available_files,
                "message": format!("未在技能 `{skill_id}` 中找到请求的任何文件"),
            });
        }
        let mut resp = json!({
            "ok": true,
            "skill_id": skill_id,
            "source": source_label,
            "available_files": available_files,
            "files": files_out,
        });
        if !missing_files.is_empty() {
            resp["missing_files"] = json!(missing_files);
        }
        return resp;
    }

    let raw_file_path = input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(Value::as_str);
    let rel_path = match sanitize_skill_rel_path(raw_file_path) {
        Ok(rel) => rel,
        Err(code) => {
            return json!({
                "ok": false,
                "skill_id": skill_id,
                "error": code,
            });
        }
    };
    let rel_str = rel_path.to_string_lossy().replace('\\', "/");

    match resolve_skill_text_content(
        &skill_dir,
        &source_label,
        skill_id,
        &rel_path,
        &rel_str,
        inline_fallback.as_deref(),
    ) {
        Ok(Some(content)) => {
            return json!({
                "ok": true,
                "skill_id": skill_id,
                "source": source_label,
                "file_path": rel_str,
                "available_files": available_files,
                "content": sanitize_skill_markdown_for_tool_result(&content),
            });
        }
        Err(err) => {
            return json!({
                "ok": false,
                "skill_id": skill_id,
                "error": err,
            });
        }
        Ok(None) => {}
    }

    json!({
        "ok": false,
        "skill_id": skill_id,
        "file_path": rel_str,
        "available_files": available_files,
        "error": "SKILL_FILE_NOT_FOUND",
        "message": format!("未找到技能 `{skill_id}` 的文件 `{rel_str}`"),
    })
}

fn bbox_to_geojson_polygon_map(bbox: [f64; 4]) -> serde_json::Map<String, Value> {
    let [w, s, e, n] = bbox;
    let mut map = serde_json::Map::new();
    map.insert("type".to_owned(), Value::String("Polygon".to_owned()));
    map.insert(
        "coordinates".to_owned(),
        json!([[[w, s], [e, s], [e, n], [w, n], [w, s]]]),
    );
    map
}

fn geojson_map_bbox(map: &serde_json::Map<String, Value>) -> Option<[f64; 4]> {
    extract_bbox_from_aoi_json(&Value::Object(map.clone()))
}

fn bbox_area_deg2(b: [f64; 4]) -> f64 {
    ((b[2] - b[0]).max(0.0)) * ((b[3] - b[1]).max(0.0))
}

fn snapshot_item_full_bytes(item: &insar_core::SnapshotItem) -> u64 {
    if let Some(assets) = item.payload.get("assets").and_then(Value::as_object) {
        let mut sum = 0_u64;
        for asset in assets.values() {
            if let Some(sz) = asset
                .get("size_bytes")
                .or_else(|| asset.get("file:size"))
                .and_then(Value::as_u64)
            {
                sum = sum.saturating_add(sz);
            }
        }
        if sum > 0 {
            return sum;
        }
    }
    if let Some(sz) = item
        .asset_access
        .as_ref()
        .and_then(|a| a.expected_size_bytes)
        .filter(|sz| *sz > 0)
    {
        return sz;
    }
    item.metadata
        .get("size_bytes")
        .or_else(|| item.payload.get("size_bytes"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

impl DesktopGeeAgentService {
    fn resolve_subset_draft_geometry_map(
        &self,
        input: &Value,
    ) -> Result<(serde_json::Map<String, Value>, &'static str), &'static str> {
        if let Some(obj) = input.get("aoi_geojson").and_then(Value::as_object) {
            if obj.get("type").and_then(Value::as_str) == Some("Feature") {
                if let Some(geom) = obj.get("geometry").and_then(Value::as_object) {
                    return Ok((geom.clone(), "polygon"));
                }
            }
            if obj.contains_key("type") && obj.contains_key("coordinates") {
                return Ok((obj.clone(), "polygon"));
            }
        }
        if let Some(bbox) = input
            .get("bbox")
            .and_then(|v| serde_json::from_value::<[f64; 4]>(v.clone()).ok())
        {
            if bbox[2] > bbox[0] && bbox[3] > bbox[1] {
                return Ok((bbox_to_geojson_polygon_map(bbox), "bbox_rectangle"));
            }
            return Err("INVALID_BBOX");
        }
        self.resolve_working_bbox()
            .map(|b| (bbox_to_geojson_polygon_map(b), "bbox_rectangle"))
            .ok_or("MISSING_WORKING_AOI")
    }

    fn resolve_batch_workbench_revision(&self, source_batch_id: &str) -> u64 {
        self.inner
            .0
            .core_restore(false)
            .ok()
            .and_then(|core| core.core.workbench)
            .filter(|wb| wb.batch_id == source_batch_id)
            .map(|wb| wb.revision)
            .unwrap_or(0)
    }

    fn collect_batch_items(
        &self,
        batch_id: &str,
        explicit_item_ids: Option<&[String]>,
    ) -> Result<Vec<insar_core::SnapshotItem>, String> {
        let page = self
            .inner
            .0
            .batch_page(&StoreBatchPageRequest {
                batch_id: batch_id.to_owned(),
                revision: 0,
                page: 0,
                page_size: 25,
            })
            .map_err(|e| format!("{e:?}"))?;
        let mut all_items = page.items;
        let total = page.total_count as usize;
        let mut page_idx = 1_u64;
        while all_items.len() < total && page_idx < 10 {
            if let Ok(next_page) = self.inner.0.batch_page(&StoreBatchPageRequest {
                batch_id: batch_id.to_owned(),
                revision: 0,
                page: page_idx,
                page_size: 25,
            }) {
                if next_page.items.is_empty() {
                    break;
                }
                all_items.extend(next_page.items);
            } else {
                break;
            }
            page_idx += 1;
        }
        if let Some(ids) = explicit_item_ids {
            all_items.retain(|it| ids.iter().any(|id| id == &it.item_id));
        } else {
            all_items.truncate(25);
        }
        Ok(all_items)
    }

    fn resolve_provider_proxy_policy(&self) -> insar_core::ProxyPolicy {
        self.inner
            .0
            .get_network_settings()
            .map(|settings| proxy_policy_from_network_settings(&settings))
            .unwrap_or(insar_core::ProxyPolicy::Direct)
    }

    fn execute_global_disaster_search(&self, input: &Value) -> Value {
        let query: insar_provider::gdacs::GdacsQuery = match serde_json::from_value(input.clone()) {
            Ok(q) => q,
            Err(err) => {
                return json!({
                    "ok": false,
                    "tool_id": "global_disaster_search",
                    "error": format!("INVALID_PARAMETERS: {err}"),
                });
            }
        };
        let proxy_policy = self.resolve_provider_proxy_policy();
        match insar_provider::gdacs::search_gdacs_events(&query, proxy_policy) {
            Ok(resp) => json!({
                "ok": true,
                "tool_id": "global_disaster_search",
                "source": resp.source,
                "source_url": resp.source_url,
                "queried_event_types": resp.queried_event_types,
                "total_matched": resp.total_matched,
                "returned_count": resp.events.len(),
                "events": resp.events,
            }),
            Err(err) => json!({
                "ok": false,
                "tool_id": "global_disaster_search",
                "error": format!("GDACS_QUERY_FAILED: {err}"),
                "hint_zh": "GDACS 公共灾害接口暂时不可达；请稍后重试，或直接提供目标地区名称与灾前/灾后日期范围。",
            }),
        }
    }

    fn execute_aoi_validate_boundary(&self, input: &Value) -> Value {
        let candidate_bbox = input
            .get("bbox")
            .and_then(|v| serde_json::from_value::<[f64; 4]>(v.clone()).ok())
            .or_else(|| self.resolve_working_bbox());
        let reference = match (
            input.get("reference_lat").and_then(Value::as_f64),
            input.get("reference_lon").and_then(Value::as_f64),
        ) {
            (Some(lat), Some(lon)) => Some((lat, lon)),
            _ => None,
        };
        let mut config = insar_agent::aoi::AoiValidationConfig::default();
        if let Some(max_area) = input
            .get("max_area_km2")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v > 0.0)
        {
            config.max_area_km2 = max_area;
        }
        if let Some(max_drift) = input
            .get("max_centroid_drift_km")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v > 0.0)
        {
            config.max_centroid_drift_km = max_drift;
        }
        let report = insar_agent::aoi::validate_place_aoi(None, candidate_bbox, reference, &config);
        json!({
            "ok": true,
            "tool_id": "aoi_validate_boundary",
            "validation": report.to_json(),
        })
    }

    fn execute_nisar_subset_preflight(&self, input: &Value) -> Value {
        let Some(source_batch_id) = input
            .get("source_batch_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return json!({
                "ok": false,
                "error": "MISSING_SOURCE_BATCH_ID",
            });
        };
        let (subset_draft_geometry, geometry_mode) =
            match self.resolve_subset_draft_geometry_map(input) {
                Ok(g) => g,
                Err(code) => {
                    return json!({
                        "ok": false,
                        "source_batch_id": source_batch_id,
                        "error": code,
                        "message": "请先绑定研究区 AOI 或传入有效 bbox / aoi_geojson",
                    });
                }
            };
        let requested_ids: Option<Vec<String>> = input
            .get("source_item_ids")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .filter(|v: &Vec<String>| !v.is_empty());

        let batch_items = match self.collect_batch_items(source_batch_id, requested_ids.as_deref())
        {
            Ok(items) if !items.is_empty() => items,
            Ok(_) => {
                return json!({
                    "ok": false,
                    "source_batch_id": source_batch_id,
                    "error": "EMPTY_NISAR_BATCH_ITEMS",
                });
            }
            Err(err) => {
                return json!({
                    "ok": false,
                    "source_batch_id": source_batch_id,
                    "error": err,
                });
            }
        };
        let source_item_ids: Vec<String> = requested_ids
            .unwrap_or_else(|| batch_items.iter().map(|it| it.item_id.clone()).collect());
        let expected_workbench_revision = self.resolve_batch_workbench_revision(source_batch_id);

        let preflight = match self.inner.0.preflight_nisar_subset_batch_v2(
            &NisarPreflightSubsetBatchV2Request {
                source_batch_id: source_batch_id.to_owned(),
                source_item_ids: source_item_ids.clone(),
                expected_workbench_revision,
                subset_draft_geometry: subset_draft_geometry.clone(),
            },
        ) {
            Ok(p) => p,
            Err(err) => {
                return json!({
                    "ok": false,
                    "source_batch_id": source_batch_id,
                    "error": format!("{err:?}"),
                });
            }
        };

        let chosen_freq = input
            .get("selected_frequency")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                preflight
                    .common_selections
                    .iter()
                    .find(|s| s.frequency.eq_ignore_ascii_case("A"))
                    .or_else(|| preflight.common_selections.first())
                    .map(|s| s.frequency.clone())
            })
            .or_else(|| {
                preflight
                    .eligible_items
                    .first()
                    .and_then(|e| e.available_selections.first())
                    .map(|s| s.frequency.clone())
            });
        let chosen_pols: Option<Vec<String>> = input
            .get("selected_polarizations")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .or_else(|| {
                let freq = chosen_freq.as_deref()?;
                preflight
                    .common_selections
                    .iter()
                    .find(|s| s.frequency.eq_ignore_ascii_case(freq))
                    .or_else(|| {
                        preflight.eligible_items.first().and_then(|e| {
                            e.available_selections
                                .iter()
                                .find(|s| s.frequency.eq_ignore_ascii_case(freq))
                        })
                    })
                    .map(|s| s.polarizations.clone())
            });

        let items_by_id: std::collections::HashMap<&str, &insar_core::SnapshotItem> = batch_items
            .iter()
            .map(|it| (it.item_id.as_str(), it))
            .collect();

        let mut total_full_bytes = 0_u64;
        let mut total_est_subset_bytes = 0_u64;
        let mut item_summaries = Vec::with_capacity(preflight.eligible_items.len());
        for eligible in &preflight.eligible_items {
            let item = items_by_id.get(eligible.source_item_id.as_str()).copied();
            let full_bytes = item.map(snapshot_item_full_bytes).unwrap_or(0);
            let source_bbox = item
                .and_then(|it| it.footprint_geojson.as_ref())
                .and_then(geojson_map_bbox);
            let eff_bbox = geojson_map_bbox(&eligible.effective_geometry.canonical_geojson);
            let area_ratio = match (source_bbox, eff_bbox) {
                (Some(src), Some(eff)) if bbox_area_deg2(src) > 0.0 => {
                    (bbox_area_deg2(eff) / bbox_area_deg2(src)).clamp(0.005, 1.0)
                }
                _ => 0.35,
            };
            let total_avail_pols: usize = eligible
                .available_selections
                .iter()
                .map(|s| s.polarizations.len().max(1))
                .sum::<usize>()
                .max(1);
            let selected_pol_count = chosen_pols
                .as_ref()
                .map(|v| v.len())
                .unwrap_or(total_avail_pols);
            let band_ratio = (selected_pol_count as f64 / total_avail_pols as f64).clamp(0.15, 1.0);
            let combined_ratio = (area_ratio * band_ratio).clamp(0.005, 0.98);
            let est_bytes = if full_bytes > 0 {
                ((full_bytes as f64) * combined_ratio).round().max(1.0) as u64
            } else {
                0
            };
            total_full_bytes = total_full_bytes.saturating_add(full_bytes);
            total_est_subset_bytes = total_est_subset_bytes.saturating_add(est_bytes);
            item_summaries.push(json!({
                "source_item_id": eligible.source_item_id,
                "effective_bbox": eff_bbox,
                "full_scene_bytes": full_bytes,
                "estimated_subset_bytes": est_bytes,
                "area_ratio": (area_ratio * 10000.0).round() / 10000.0,
                "available_selections": eligible.available_selections,
            }));
        }

        let prepared_count = match (&chosen_freq, &chosen_pols) {
            (Some(freq), Some(pols)) if !preflight.eligible_items.is_empty() => self
                .inner
                .0
                .prepare_nisar_subset_batch_v2(&NisarPrepareSubsetBatchV2Request {
                    source_batch_id: source_batch_id.to_owned(),
                    source_item_ids: preflight
                        .eligible_items
                        .iter()
                        .map(|e| e.source_item_id.clone())
                        .collect(),
                    expected_workbench_revision,
                    subset_draft_geometry,
                    selected_frequency: freq.clone(),
                    selected_polarizations: pols.clone(),
                })
                .map(|p| p.intents.len())
                .unwrap_or(0),
            _ => 0,
        };

        let saved_bytes = total_full_bytes.saturating_sub(total_est_subset_bytes);
        let savings_pct = if total_full_bytes > 0 {
            ((saved_bytes as f64) / (total_full_bytes as f64) * 1000.0).round() / 10.0
        } else {
            0.0
        };

        let spatial_intersection_verified = !preflight.eligible_items.is_empty();
        json!({
            "ok": true,
            "nisar_aoi_subset_supported": true,
            "transfer_mode": "remote_range",
            "source_batch_id": source_batch_id,
            "expected_workbench_revision": expected_workbench_revision,
            "selected_frequency": chosen_freq,
            "selected_polarizations": chosen_pols,
            "prepared_intent_count": prepared_count,
            "eligible_items": item_summaries,
            "excluded_items": preflight.excluded_items,
            "common_selections": preflight.common_selections,
            "full_scene_bytes": total_full_bytes,
            "estimated_subset_bytes": total_est_subset_bytes,
            "estimated_saved_bytes": saved_bytes,
            "estimated_savings_pct": savings_pct,
            "spatial_intersection_verified": spatial_intersection_verified,
            "geometry_mode": geometry_mode,
            "byte_estimate_mode": "pre_hdf5_metadata_estimate",
            "hdf5_chunk_extraction_verified": false,
            "estimate_basis": "pre_metadata_aoi_intersection_and_band_ratio",
        })
    }

    fn execute_nisar_create_subset_task(&self, app_data_dir: &Path, input: &Value) -> Value {
        let Some(source_batch_id) = input
            .get("source_batch_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return json!({
                "ok": false,
                "error": "MISSING_SOURCE_BATCH_ID",
            });
        };
        let preflight_val = self.execute_nisar_subset_preflight(input);
        if preflight_val.get("ok").and_then(Value::as_bool) != Some(true) {
            return preflight_val;
        }
        let (subset_draft_geometry, _geometry_mode) =
            match self.resolve_subset_draft_geometry_map(input) {
                Ok(g) => g,
                Err(code) => {
                    return json!({
                        "ok": false,
                        "source_batch_id": source_batch_id,
                        "error": code,
                    });
                }
            };
        let selected_frequency = preflight_val
            .get("selected_frequency")
            .and_then(Value::as_str)
            .unwrap_or("A")
            .to_owned();
        let selected_polarizations: Vec<String> = preflight_val
            .get("selected_polarizations")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or_else(|| vec!["HH".to_owned()]);
        let eligible_ids: Vec<String> = preflight_val
            .get("eligible_items")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|it| it.get("source_item_id").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if eligible_ids.is_empty() {
            return json!({
                "ok": false,
                "source_batch_id": source_batch_id,
                "error": "NO_ELIGIBLE_NISAR_ITEMS_IN_AOI",
            });
        }
        let expected_workbench_revision = self.resolve_batch_workbench_revision(source_batch_id);
        let prepared =
            match self
                .inner
                .0
                .prepare_nisar_subset_batch_v2(&NisarPrepareSubsetBatchV2Request {
                    source_batch_id: source_batch_id.to_owned(),
                    source_item_ids: eligible_ids.clone(),
                    expected_workbench_revision,
                    subset_draft_geometry,
                    selected_frequency: selected_frequency.clone(),
                    selected_polarizations: selected_polarizations.clone(),
                }) {
                Ok(p) => p,
                Err(err) => {
                    return json!({
                        "ok": false,
                        "source_batch_id": source_batch_id,
                        "error": format!("{err:?}"),
                    });
                }
            };

        let configured_root = self
            .inner
            .application_default_output_root()
            .ok()
            .flatten()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| {
                p.is_absolute()
                    && !p
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                    && !std::fs::symlink_metadata(p)
                        .ok()
                        .is_some_and(|m| m.file_type().is_symlink())
            })
            .unwrap_or_else(|| app_data_dir.join("downloads"));
        if std::fs::symlink_metadata(&configured_root)
            .ok()
            .is_some_and(|m| m.file_type().is_symlink())
        {
            return json!({
                "ok": false,
                "error": "UNSAFE_OUTPUT_ROOT_SYMLINK",
            });
        }
        if let Err(err) = std::fs::create_dir_all(&configured_root) {
            return json!({
                "ok": false,
                "error": format!("OUTPUT_ROOT_CREATE_FAILED: {err}"),
            });
        }

        let task_id = format!("nisar_sub_{}", next_gee_agent_download_token());
        let created_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let concurrency = input
            .get("concurrency")
            .and_then(Value::as_u64)
            .unwrap_or(4)
            .clamp(1, 8);
        let confirm_items: Vec<NisarConfirmSubsetBatchItemV2> = prepared
            .intents
            .iter()
            .map(|entry| NisarConfirmSubsetBatchItemV2 {
                source_item_id: entry.source_item_id.clone(),
                prepared_intent: entry.intent.clone(),
            })
            .collect();
        let included_ids: Vec<String> = confirm_items
            .iter()
            .map(|it| it.source_item_id.clone())
            .collect();

        if let Err(err) =
            self.inner
                .0
                .confirm_nisar_subset_batch_task_v2(&NisarConfirmSubsetBatchTaskV2Request {
                    source_batch_id: source_batch_id.to_owned(),
                    expected_workbench_revision,
                    items: confirm_items,
                    task_id: task_id.clone(),
                    output_root: configured_root,
                    layout: DownloadLayout::Direct,
                    concurrency,
                    created_at,
                })
        {
            return json!({
                "ok": false,
                "source_batch_id": source_batch_id,
                "error": format!("{err:?}"),
            });
        }

        let auto_start = input
            .get("auto_start")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let (started, status, start_warning) = if auto_start {
            match self.inner.0.start_newly_confirmed_task(&task_id) {
                Ok(_) => (true, "running", Value::Null),
                Err(err) => (
                    false,
                    "paused",
                    Value::String(format!("TASK_CREATED_PENDING_START: {err:?}")),
                ),
            }
        } else {
            (false, "paused", Value::Null)
        };

        json!({
            "ok": true,
            "task_id": task_id,
            "status": status,
            "started": started,
            "start_warning": start_warning,
            "transfer_mode": "remote_range",
            "source_batch_id": source_batch_id,
            "selected_frequency": selected_frequency,
            "selected_polarizations": selected_polarizations,
            "included_item_ids": included_ids,
            "excluded_items": prepared.excluded_items,
            "full_scene_bytes": preflight_val["full_scene_bytes"],
            "estimated_subset_bytes": preflight_val["estimated_subset_bytes"],
            "estimated_saved_bytes": preflight_val["estimated_saved_bytes"],
            "estimated_savings_pct": preflight_val["estimated_savings_pct"],
        })
    }
}

impl DesktopGeeAgentService {
    fn call_gis_extension_tool(&self, tool_id: &str, input: &Value) -> Result<Value, ErrorClass> {
        let Some(app_data_dir) = self.inner.data_root() else {
            return Err(ErrorClass::Internal);
        };
        let rt_root = crate::application::discover_runtime_root();
        match tool_id {
            "gis_toolchains_status" => {
                let probe_versions = input
                    .get("probe_versions")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let engine = crate::geodata_tools::discover_native_gis_engine();
                match crate::external_gis_bridge::inspect_external_gis_toolchains(
                    &rt_root,
                    &app_data_dir,
                    probe_versions,
                ) {
                    Ok(report) => {
                        let summaries: Vec<Value> = report
                            .toolchains
                            .into_iter()
                            .map(|t| {
                                json!({
                                    "id": t.id,
                                    "display_name": t.display_name,
                                    "installed": t.installed,
                                    "source": t.source,
                                    "version": t.version,
                                    "mcp_capable": t.mcp_capable,
                                    "headless_capable": t.headless_capable,
                                    "gui_capable": t.gui_capable,
                                    "capabilities": t.capabilities,
                                })
                            })
                            .collect();
                        let conda_summaries: Vec<Value> = report
                            .conda_environments
                            .into_iter()
                            .map(|c| {
                                json!({
                                    "id": c.id,
                                    "name": c.name,
                                    "source": c.source,
                                    "python_version": c.python_version,
                                    "geo_packages": c.geo_packages,
                                    "verified": c.verified,
                                })
                            })
                            .collect();
                        let dehydrator_summary = json!({
                            "builtin_ready": report.log_dehydrator.builtin_ready,
                            "mode": report.log_dehydrator.mode,
                            "shared_rtk_installed": report.log_dehydrator.shared_rtk.installed,
                            "shared_rtk_source": report.log_dehydrator.shared_rtk.source,
                            "shared_rtk_version": report.log_dehydrator.shared_rtk.version,
                        });
                        Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                            "ok": true,
                            "tool_id": "gis_toolchains_status",
                            "platform": report.platform,
                            "builtin_engine": {
                                "available": engine.is_discovered(),
                                "engine": "GDAL/PROJ",
                            },
                            "preferred_toolchain": report.prefs.preferred_toolchain,
                            "preferred_conda_env": report.prefs.preferred_conda_env,
                            "toolchains": summaries,
                            "conda_environments": conda_summaries,
                            "log_dehydrator": dehydrator_summary,
                        })))
                    }
                    Err(err) => Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                        "ok": false,
                        "tool_id": "gis_toolchains_status",
                        "error": err,
                    }))),
                }
            }
            "gis_local_process" => {
                let domain = input
                    .get("domain")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or("");
                let operation = input
                    .get("operation")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or("");
                let requested_name = input
                    .get("output_filename")
                    .and_then(Value::as_str)
                    .unwrap_or("output");
                let configured_root = self.inner.application_default_output_root().ok().flatten();
                let default_ext = if domain == "vector" {
                    "geojson"
                } else if input
                    .get("output_format")
                    .and_then(Value::as_str)
                    .is_some_and(|f| f.eq_ignore_ascii_case("ENVI"))
                {
                    "dat"
                } else {
                    "tif"
                };
                let safe_out = match allocate_safe_gis_agent_output_path(
                    configured_root.as_deref(),
                    &app_data_dir,
                    requested_name,
                    default_ext,
                ) {
                    Ok(p) => p,
                    Err(err) => {
                        return Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                            "ok": false,
                            "tool_id": "gis_local_process",
                            "error": err,
                        })));
                    }
                };
                let out_name = safe_out
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("output")
                    .to_string();

                let result = if domain == "raster" {
                    let sources: Vec<String> = input
                        .get("sources")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default();
                    let crop_bbox = input
                        .get("crop_bbox")
                        .and_then(|v| serde_json::from_value::<[f64; 4]>(v.clone()).ok());
                    let req = insar_gis::RasterProcessRequest {
                        sources,
                        output_path: safe_out,
                        operation: operation.to_owned(),
                        crop_bbox,
                        band_a: input
                            .get("band_a")
                            .and_then(Value::as_u64)
                            .map(|n| n as usize),
                        band_b: input
                            .get("band_b")
                            .and_then(Value::as_u64)
                            .map(|n| n as usize),
                        z_factor: input.get("z_factor").and_then(Value::as_f64),
                        azimuth_deg: input.get("azimuth_deg").and_then(Value::as_f64),
                        altitude_deg: input.get("altitude_deg").and_then(Value::as_f64),
                        target_crs: input
                            .get("target_crs")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        resampling: input
                            .get("resampling")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        output_format: input
                            .get("output_format")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        preserve_dtype: input.get("preserve_dtype").and_then(Value::as_bool),
                    };
                    crate::geodata_tools::execute_gis_raster_process(&req)
                } else if domain == "vector" {
                    let input_path = input
                        .get("input_path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    let req = insar_gis::VectorProcessRequest {
                        input_path: PathBuf::from(input_path),
                        output_path: safe_out,
                        operation: operation.to_owned(),
                        distance_m: input.get("distance_m").and_then(Value::as_f64),
                        quad_segs: input
                            .get("quad_segs")
                            .and_then(Value::as_i64)
                            .map(|n| n as i32),
                    };
                    crate::geodata_tools::execute_gis_vector_process(&req)
                } else {
                    Err(format!(
                        "不支持的 domain '{domain}'，仅支持 raster 或 vector"
                    ))
                };

                Ok(match result {
                    Ok(val) => crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                        "ok": true,
                        "tool_id": "gis_local_process",
                        "domain": domain,
                        "operation": operation,
                        "artifact_name": out_name,
                        "result": val,
                    })),
                    Err(err) => crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                        "ok": false,
                        "tool_id": "gis_local_process",
                        "error": err,
                    })),
                })
            }
            "gis_external_execute" => {
                let toolchain = input
                    .get("toolchain")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                let action = input
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                let algorithm = input
                    .get("algorithm")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let mut params = std::collections::BTreeMap::new();
                if let Some(obj) = input.get("params").and_then(Value::as_object) {
                    for (k, v) in obj {
                        let lower_key = k.trim().to_ascii_lowercase();
                        let is_output_override = matches!(
                            lower_key.as_str(),
                            "output"
                                | "output_path"
                                | "output_dir"
                                | "output_root"
                                | "output_file"
                                | "output_folder"
                                | "output_raster"
                                | "output_rasterdataset"
                                | "output_vector"
                                | "output_layer"
                                | "output_table"
                                | "output_dataset"
                                | "output_feature_class"
                                | "output_workspace"
                                | "output_location"
                                | "output_name"
                                | "out_feature_class"
                                | "out_raster"
                                | "out_rasterdataset"
                                | "out_raster_dataset"
                                | "out_dataset"
                                | "out_table"
                                | "out_file"
                                | "out_folder"
                                | "out_workspace"
                                | "out_path"
                                | "out_name"
                                | "out_dir"
                                | "out_location"
                                | "raster_dataset_name_with_extension"
                                | "out_layer"
                                | "out_polygon_features"
                                | "out_point_features"
                                | "out_polyline_features"
                                | "out_fc"
                                | "out_shp"
                                | "out_tif"
                                | "out_tiff"
                                | "out_geojson"
                        );
                        if is_output_override {
                            return Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                                "ok": false,
                                "tool_id": "gis_external_execute",
                                "error": format!(
                                    "GIS_EXTERNAL_OUTPUT_PARAM_FORBIDDEN: 参数 '{k}' 不允许直接指定输出路径，请使用 output_filename 写入受控 gis_outputs 目录。"
                                ),
                            })));
                        }
                        params.insert(k.clone(), v.clone());
                    }
                }
                let input_paths: Vec<String> = input
                    .get("input_paths")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let configured_root = self.inner.application_default_output_root().ok().flatten();
                let output_path = if let Some(raw_name) = input
                    .get("output_filename")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let ext = if action == "deliver_project" {
                        "qgs"
                    } else {
                        "geojson"
                    };
                    match allocate_safe_gis_agent_output_path(
                        configured_root.as_deref(),
                        &app_data_dir,
                        raw_name,
                        ext,
                    ) {
                        Ok(p) => Some(p.to_string_lossy().into_owned()),
                        Err(err) => {
                            return Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                                "ok": false,
                                "tool_id": "gis_external_execute",
                                "error": err,
                            })));
                        }
                    }
                } else {
                    None
                };
                let open_gui = input
                    .get("open_gui")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let conda_env = input
                    .get("conda_env")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let req = crate::external_gis_bridge::ExternalGisExecuteRequest {
                    toolchain,
                    action,
                    algorithm,
                    conda_env,
                    params,
                    input_paths,
                    output_path,
                    open_gui,
                    timeout_seconds: None,
                };
                match crate::external_gis_bridge::execute_external_gis_action(
                    &rt_root,
                    &app_data_dir,
                    &req,
                ) {
                    Ok(res) => Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                        "ok": res.ok,
                        "tool_id": "gis_external_execute",
                        "toolchain": res.toolchain,
                        "action": res.action,
                        "exit_code": res.exit_code,
                        "stdout": res.stdout,
                        "stderr": res.stderr,
                        "gui_launched": res.gui_launched,
                        "artifact_count": res.artifacts.len(),
                        "detail": res.detail,
                        "dehydration": res.dehydration,
                    }))),
                    Err(err) => Ok(crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                        "ok": false,
                        "tool_id": "gis_external_execute",
                        "error": err,
                    }))),
                }
            }
            _ => Err(ErrorClass::Internal),
        }
    }

    fn execute_open_geodata_query_extension(&self, app_data_dir: &Path, input: &Value) -> Value {
        let mut req: crate::geodata_tools::OpenGeodataQueryRequest =
            match serde_json::from_value(input.clone()) {
                Ok(parsed) => parsed,
                Err(err) => {
                    return crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                        "ok": false,
                        "tool_id": "open_geodata_query",
                        "error": format!("INVALID_OPEN_GEODATA_ARGS: {err}"),
                    }));
                }
            };
        let dataset_norm = req.dataset.trim().to_ascii_lowercase();
        let allowlisted = matches!(
            dataset_norm.as_str(),
            "aws_sentinel1_grd"
                | "sentinel1_grd_aws"
                | "noaa_aws_weather"
                | "noaa_gfs"
                | "noaa_goes"
                | "overture_maps"
                | "overture"
                | "open_dem_tiles"
                | "dem_tiles"
                | "whu_clcd"
                | "tum_eot20"
                | "gebco_2026"
                | "noaa_etopo_2022"
        );
        if !allowlisted {
            return crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                "ok": false,
                "tool_id": "open_geodata_query",
                "dataset": req.dataset,
                "error": format!("UNSUPPORTED_AGENT_OPEN_GEODATA_DATASET: `{}` 不在开放地学 Agent 白名单中", req.dataset),
            }));
        }
        if let Some(gp_obj) = req.gee_params.as_mut().and_then(Value::as_object_mut) {
            gp_obj.remove("output_path");
            gp_obj.remove("output_dir");
            gp_obj.remove("output_root");
        }
        let keyword_has_download = req.keyword.as_deref().is_some_and(|kw| {
            kw.split(['|', ',', ';'])
                .any(|tok| tok.trim().eq_ignore_ascii_case("download"))
        });
        let wants_download = req
            .output_path
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
            || req.tool_name.as_deref().map(str::trim).is_some_and(|t| {
                !t.is_empty()
                    && !t.eq_ignore_ascii_case("search")
                    && !t.eq_ignore_ascii_case("preview")
            })
            || keyword_has_download
            || req
                .gee_params
                .as_ref()
                .and_then(|v| v.get("download"))
                .and_then(|v| v.as_bool())
                == Some(true);
        if wants_download {
            match confine_open_geodata_agent_output_path(
                app_data_dir,
                &req.dataset,
                req.output_path.as_deref(),
            ) {
                Ok(confined) => {
                    req.output_path = Some(confined.to_string_lossy().into_owned());
                }
                Err(err) => {
                    return crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                        "ok": false,
                        "tool_id": "open_geodata_query",
                        "dataset": req.dataset,
                        "error": err,
                    }));
                }
            }
        } else {
            req.output_path = None;
            req.tool_name = Some("search".to_owned());
            if matches!(
                dataset_norm.as_str(),
                "whu_clcd" | "tum_eot20" | "gebco_2026" | "noaa_etopo_2022"
            ) {
                let next_kw = match req.keyword.as_deref().map(str::trim) {
                    Some(k) if !k.is_empty() => format!("{k}|search"),
                    _ => "search".to_owned(),
                };
                req.keyword = Some(next_kw);
            }
        }
        let started = std::time::Instant::now();
        match crate::geodata_tools::execute_open_geodata_query(app_data_dir, &req) {
            Ok(mut val) => {
                if wants_download {
                    crate::geodata_tools::maybe_record_geodata_download_task_on_service(
                        &self.inner.0,
                        &req,
                        &mut val,
                        started.elapsed(),
                    );
                }
                crate::gee_cloud_suite::sanitize_for_agent_trace(&val)
            }
            Err(err) => crate::gee_cloud_suite::sanitize_for_agent_trace(&json!({
                "ok": false,
                "tool_id": "open_geodata_query",
                "dataset": req.dataset,
                "error": err,
            })),
        }
    }
}

fn confine_open_geodata_agent_output_path(
    app_data_dir: &Path,
    dataset: &str,
    requested_output_path: Option<&str>,
) -> Result<PathBuf, String> {
    if std::fs::symlink_metadata(app_data_dir)
        .ok()
        .is_some_and(|m| m.file_type().is_symlink())
    {
        return Err("UNSAFE_OUTPUT_DIR: 拒绝写入符号链接应用数据目录".to_owned());
    }
    let dl_root = app_data_dir.join("geodata_downloads");
    if std::fs::symlink_metadata(&dl_root)
        .ok()
        .is_some_and(|m| m.file_type().is_symlink())
    {
        return Err("UNSAFE_OUTPUT_DIR: 拒绝写入符号链接 geodata_downloads 目录".to_owned());
    }
    let safe_dataset: String = dataset
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .take(32)
        .collect();
    let ds_dir = dl_root.join(if safe_dataset.is_empty() {
        "OPEN_GEODATA"
    } else {
        safe_dataset.as_str()
    });
    if std::fs::symlink_metadata(&ds_dir)
        .ok()
        .is_some_and(|m| m.file_type().is_symlink())
    {
        return Err("UNSAFE_OUTPUT_DIR: 拒绝写入符号链接数据集输出目录".to_owned());
    }
    std::fs::create_dir_all(&ds_dir).map_err(|err| format!("CREATE_OUTPUT_DIR_FAILED: {err}"))?;

    if let Some(raw) = requested_output_path
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(file_name) = Path::new(raw).file_name().and_then(|s| s.to_str()) {
            let ext_ok = Path::new(file_name)
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "tif" | "tiff" | "grib2" | "grb2" | "nc" | "parquet" | "geojson" | "json"
                    )
                });
            if ext_ok {
                let clean_name: String = file_name
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                if !clean_name.trim_matches('.').is_empty() {
                    return Ok(ds_dir.join(clean_name));
                }
            }
        }
    }
    Ok(ds_dir)
}

fn allocate_safe_gis_agent_output_path(
    configured_default_output_root: Option<&str>,
    app_data_dir: &Path,
    requested_filename: &str,
    default_ext: &str,
) -> Result<PathBuf, String> {
    if std::fs::symlink_metadata(app_data_dir)
        .ok()
        .is_some_and(|m| m.file_type().is_symlink())
    {
        return Err("UNSAFE_GIS_OUTPUT_PATH: 拒绝写入符号链接应用数据目录".to_owned());
    }
    let base_dir = configured_default_output_root
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Path::new)
        .filter(|p| {
            p.is_absolute()
                && !p
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                && !std::fs::symlink_metadata(p)
                    .ok()
                    .is_some_and(|m| m.file_type().is_symlink())
        })
        .map(|p| p.join("gis_outputs"))
        .unwrap_or_else(|| app_data_dir.join("gis_outputs"));

    if std::fs::symlink_metadata(&base_dir)
        .ok()
        .is_some_and(|m| m.file_type().is_symlink())
    {
        return Err("UNSAFE_GIS_OUTPUT_PATH: 拒绝写入符号链接 gis_outputs 目录".to_owned());
    }
    std::fs::create_dir_all(&base_dir)
        .map_err(|err| format!("创建受控 GIS 输出目录失败: {err}"))?;

    let raw_name = Path::new(requested_filename)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("gis_output");
    let raw_stem = Path::new(raw_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("gis_output");
    let stem_clean: String = raw_stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(48)
        .collect();
    let stem = if stem_clean.trim_matches(|c| c == '_' || c == '-').is_empty() {
        "gis_output".to_owned()
    } else {
        stem_clean
    };
    let ext_clean = Path::new(raw_name)
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|ext| {
            matches!(
                ext.as_str(),
                "tif" | "tiff" | "dat" | "hdr" | "geojson" | "shp" | "qgs" | "json"
            )
        })
        .unwrap_or_else(|| default_ext.to_owned());
    let token = next_gee_agent_download_token();
    Ok(base_dir.join(format!("{stem}_{token}.{ext_clean}")))
}

fn strip_agent_supplied_output_fields(input: &Value) -> Value {
    match input {
        Value::Object(map) => {
            let mut clean = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                if k.eq_ignore_ascii_case("output_path")
                    || k.eq_ignore_ascii_case("output_root")
                    || k.eq_ignore_ascii_case("outputpath")
                    || k.eq_ignore_ascii_case("outputroot")
                {
                    continue;
                }
                clean.insert(k.clone(), v.clone());
            }
            Value::Object(clean)
        }
        other => other.clone(),
    }
}

fn sanitize_gee_agent_filename_stem(asset_id: &str, index_name: Option<&str>) -> String {
    let raw = match index_name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(idx) => format!("{asset_id}_{idx}"),
        None => asset_id.to_owned(),
    };
    let mut out = String::with_capacity(raw.len().min(64));
    let mut prev_sep = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
            prev_sep = false;
        } else if !prev_sep && !out.is_empty() {
            out.push('_');
            prev_sep = true;
        }
        if out.len() >= 64 {
            break;
        }
    }
    let trimmed = out.trim_matches(|c| c == '_' || c == '-');
    if trimmed.is_empty() {
        "gee_mosaic".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn next_gee_agent_download_token() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{ms}_{seq}")
}

pub(crate) fn allocate_safe_gee_agent_download_path(
    configured_default_output_root: Option<&str>,
    app_data_dir: &Path,
    asset_id: &str,
    index_name: Option<&str>,
    unique_suffix: &str,
) -> PathBuf {
    let base_dir = configured_default_output_root
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Path::new)
        .filter(|p| {
            p.is_absolute()
                && !p
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                && !std::fs::symlink_metadata(p)
                    .ok()
                    .is_some_and(|m| m.file_type().is_symlink())
        })
        .map(|p| p.join("gee_downloads"))
        .unwrap_or_else(|| app_data_dir.join("gee_downloads"));

    let stem = sanitize_gee_agent_filename_stem(asset_id, index_name);
    let suffix_clean: String = unique_suffix
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(40)
        .collect();
    let suffix_final = if suffix_clean.is_empty() {
        "0"
    } else {
        suffix_clean.as_str()
    };

    for idx in 0..=1024_u32 {
        let filename = if idx == 0 {
            format!("{stem}_{suffix_final}.tif")
        } else {
            format!("{stem}_{suffix_final}_{idx}.tif")
        };
        let candidate = base_dir.join(filename);
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    base_dir.join(format!("{stem}_{suffix_final}_{}.tif", std::process::id()))
}

pub fn agent_error(error: HostError) -> ShellError {
    ShellError::with_code(error.code(), "agent.error")
}

/// Opened lazily on the first Agent command (inside a background task): it
/// touches the Store to mark runs a previous process left `running`.
pub struct AgentHostState {
    application: AgentApplication,
    root: Option<PathBuf>,
    host: Mutex<Option<Arc<DesktopAgentHost>>>,
    notifier: Mutex<Option<AgentHostNotifier>>,
    search_sync_registered: std::sync::atomic::AtomicBool,
}

impl AgentHostState {
    pub fn new(application: AgentApplication) -> Self {
        let root = application.data_root();
        Self {
            application,
            root,
            host: Mutex::new(None),
            notifier: Mutex::new(None),
            search_sync_registered: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn set_notifier(&self, notifier: AgentHostNotifier) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(Arc::clone(&notifier));
        }
        if !self
            .search_sync_registered
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            let notifier_for_sync = Arc::clone(&notifier);
            self.application
                .0
                .register_search_sync_listener(Arc::new(move |payload| {
                    notifier_for_sync(AgentHostNotification::WorkspaceSync {
                        session_id: String::new(),
                        run_id: String::new(),
                        payload,
                    });
                }));
        }
        if let Ok(slot) = self.host.lock() {
            if let Some(host) = slot.as_ref() {
                host.set_notifier(notifier);
            }
        }
    }

    pub(crate) fn application(&self) -> &AgentApplication {
        &self.application
    }

    fn root(&self) -> Result<PathBuf, HostError> {
        self.root.clone().ok_or(HostError::StoreUnavailable)
    }

    pub fn host(&self) -> Result<Arc<DesktopAgentHost>, ShellError> {
        let mut slot = self
            .host
            .lock()
            .map_err(|_| agent_error(HostError::StoreUnavailable))?;
        if let Some(host) = slot.as_ref() {
            return Ok(Arc::clone(host));
        }
        let root = self.root().map_err(agent_error)?;
        let host = AgentHost::open(
            root.clone(),
            DesktopGeeAgentService {
                inner: self.application.clone(),
            },
            DesktopModels {
                root,
                application: self.application.clone(),
            },
        )
        .map_err(agent_error)?;
        host.set_max_steps(16);
        if let Ok(notifier_slot) = self.notifier.lock() {
            if let Some(notifier) = notifier_slot.as_ref() {
                host.set_notifier(Arc::clone(notifier));
            }
        }
        *slot = Some(Arc::clone(&host));
        Ok(host)
    }

    fn channels(&self) -> Result<ChannelStore, HostError> {
        Ok(ChannelStore::new(self.root()?.join("agent")))
    }

    /// Verifies that `channel_id` is a valid non-reserved LLM channel ID that
    /// currently exists in `agent-channels.json`, preventing keyring lookups
    /// against non-LLM namespaces such as `gee-credentials` or `gee-*`.
    pub fn has_saved_channel(&self, channel_id: &str) -> Result<bool, HostError> {
        let trimmed = channel_id.trim();
        if !is_valid_channel_id(trimmed) {
            return Ok(false);
        }
        Ok(self
            .channels()?
            .saved(trimmed)
            .map_err(channel_error)?
            .is_some())
    }

    pub fn channels_get(&self) -> Result<Vec<AgentChannelView>, HostError> {
        let channels = self.channels()?;
        let default = channels.default_id().map_err(channel_error)?;
        Ok(channels
            .list(&DesktopKeys)
            .map_err(channel_error)?
            .into_iter()
            .map(|(channel, key_ready)| {
                let is_official = crate::eostage_account::is_official_channel_id(&channel.id);
                let mut models = channel.effective_models();
                if !is_official {
                    for m in &mut models {
                        m.is_free = None;
                    }
                }
                AgentChannelView {
                    is_default: default.as_deref() == Some(channel.id.as_str()),
                    models,
                    id: channel.id,
                    protocol: channel.protocol,
                    base_url: channel.base_url,
                    model: channel.model,
                    reasoning_mode: channel.reasoning_mode,
                    reasoning_switch_unsupported: channel.reasoning_switch_unsupported,
                    key_ready,
                }
            })
            .collect())
    }

    pub fn disabled_items_get(&self) -> Result<AgentDisabledItems, HostError> {
        let root = self.root()?;
        Ok(load_disabled_items_at(&root))
    }

    pub fn disabled_items_set(
        &self,
        items: AgentDisabledItems,
    ) -> Result<AgentDisabledItems, HostError> {
        let root = self.root()?;
        save_disabled_items_at(&root, &items)?;
        Ok(items)
    }

    /// The "current channel" new runs use; without one, the first channel
    /// with a saved key is used.
    pub fn channels_set_default(
        &self,
        channel_id: &str,
    ) -> Result<Vec<AgentChannelView>, HostError> {
        self.channels()?
            .set_default(channel_id)
            .map_err(channel_error)?;
        self.channels_get()
    }

    /// Removes a channel: its saved key, its metadata entry, and the default
    /// pointer when it named this channel; returns the refreshed list.
    pub fn channels_delete(&self, channel_id: &str) -> Result<Vec<AgentChannelView>, HostError> {
        delete_channel(&self.channels()?, channel_id, &DesktopKeys)?;
        self.channels_get()
    }

    /// Atomically renames a channel in-process without round-tripping keys
    /// through the UI: verifies target ID validity and non-collision, migrates
    /// any saved credential under `new_id` first (aborting immediately if read
    /// or write fails), updates `agent-channels.json` and default pointer, and
    /// deletes `old_id`'s credential entry last.
    pub fn channels_rename(
        &self,
        old_id: &str,
        new_id: &str,
        api_key: Option<String>,
    ) -> Result<Vec<AgentChannelView>, HostError> {
        rename_channel(
            &self.channels()?,
            old_id,
            new_id,
            api_key.as_deref(),
            &DesktopKeys,
        )?;
        self.channels_get()
    }

    pub fn channels_save(
        &self,
        save: AgentChannelSave,
    ) -> Result<Vec<AgentChannelView>, HostError> {
        save_channel(&self.channels()?, save, &DesktopKeys)?;
        self.channels_get()
    }

    fn channel(&self, channel_id: &str) -> Result<ChannelConfig, HostError> {
        self.channels()?
            .list(&DesktopKeys)
            .map_err(channel_error)?
            .into_iter()
            .map(|(channel, _)| channel)
            .find(|channel| channel.id == channel_id)
            .ok_or(HostError::NotFound)
    }

    /// Bounded diagnostic: one authenticated model-list request. The key is
    /// borrowed inside the credential closure and never returned or logged.
    pub fn channels_models(&self, channel_id: &str) -> Result<Vec<String>, HostError> {
        self.fetch_models(channel_id)
            .map_err(|_| HostError::ModelUnavailable)
    }

    fn fetch_models(&self, channel_id: &str) -> Result<Vec<String>, AgentChannelTest> {
        let channel = self.channel(channel_id).map_err(AgentChannelTest::host)?;
        let route = channel
            .validated_route()
            .map_err(|_| AgentChannelTest::code("AGENT_CHANNEL_INVALID"))?;
        let client = model_client(&self.application, Duration::from_secs(20))
            .map_err(|_| AgentChannelTest::code("AGENT_MODEL_UNAVAILABLE"))?;
        let mut attempt_res = insar_download::with_desktop_agent_model_key(&channel.id, |key| {
            let (resolved_key, resolved_route) =
                crate::oauth_login::resolve_runtime_secret_and_route(
                    &self.application,
                    &channel.id,
                    key,
                    route.clone(),
                );
            block_on(client.models_with_headers(
                &resolved_route.endpoint,
                resolved_key.as_str(),
                &resolved_route.extra_headers,
                &CancellationToken::new(),
            ))
        })
        .map_err(|_| AgentChannelTest::code("CREDENTIAL_STORE_UNAVAILABLE"))?
        .ok_or_else(|| AgentChannelTest::code("AGENT_CHANNEL_KEY_MISSING"))?;
        if matches!(&attempt_res, Err(err) if err.status == Some(401)) {
            if let Some((refreshed_key, refreshed_route)) =
                crate::oauth_login::force_refresh_or_reload_codex_oauth(
                    &self.application,
                    &channel.id,
                    route.clone(),
                )
            {
                attempt_res = block_on(client.models_with_headers(
                    &refreshed_route.endpoint,
                    refreshed_key.as_str(),
                    &refreshed_route.extra_headers,
                    &CancellationToken::new(),
                ));
            }
        }
        attempt_res.map_err(AgentChannelTest::model)
    }

    pub fn channels_test(&self, channel_id: &str) -> Result<AgentChannelTest, HostError> {
        let channels = self.channels()?;
        let saved = channels.saved(channel_id).map_err(channel_error)?;
        let Some(saved) = saved else {
            return Ok(AgentChannelTest::code("AGENT_CHANNEL_NOT_FOUND"));
        };
        let protocol = saved
            .effective_models()
            .into_iter()
            .find(|m| m.id == saved.model)
            .and_then(|m| m.protocol)
            .unwrap_or(saved.protocol);
        self.channels_test_draft(AgentChannelDraft {
            id: Some(saved.id),
            protocol,
            base_url: saved.base_url,
            api_key: None,
            model: Some(saved.model),
        })
    }

    pub fn channels_test_draft(
        &self,
        draft: AgentChannelDraft,
    ) -> Result<AgentChannelTest, HostError> {
        let channels = self.channels()?;
        let id = draft.id.as_deref().filter(|id| !id.is_empty());
        let model = match draft.model.filter(|model| !model.trim().is_empty()) {
            Some(model) => model,
            None => {
                return Ok(AgentChannelTest {
                    ok: false,
                    code: Some("AGENT_CHANNEL_INVALID".into()),
                    status: None,
                    model_count: None,
                    latency_ms: Some(0),
                    message: Some("请先添加或选择模型后再测试连接".into()),
                });
            }
        };
        let is_https = draft.base_url.starts_with("https://");
        let is_chat_completions = draft.protocol == "chat_completions";
        let channel = ChannelConfig {
            id: id.unwrap_or("draft").to_owned(),
            protocol: draft.protocol,
            base_url: draft.base_url,
            model: model.clone(),
            reasoning_mode: ReasoningMode::ProviderDefault,
            reasoning_switch_unsupported: false,
            models: vec![],
        };
        let start = std::time::Instant::now();
        let (resolved_route, resolved_key) = match (|| -> Result<(insar_agent_model::transport::ModelRoute, Zeroizing<String>), AgentChannelTest> {
            let route = channel
                .validated_route()
                .map_err(|_| AgentChannelTest::code("AGENT_CHANNEL_INVALID"))?
                .with_max_output_tokens(Some(64));
            if let Some(key) = draft
                .api_key
                .as_deref()
                .filter(|key| !key.trim().is_empty())
            {
                let (k, r) = crate::oauth_login::resolve_runtime_secret_and_route(
                    &self.application,
                    &channel.id,
                    key,
                    route,
                );
                return Ok((r, k));
            }
            let saved_id = id.ok_or_else(|| AgentChannelTest::code("AGENT_CHANNEL_KEY_MISSING"))?;
            let saved = channels
                .saved(saved_id)
                .map_err(|_| AgentChannelTest::code("AGENT_CHANNEL_INVALID"))?
                .ok_or_else(|| AgentChannelTest::code("AGENT_CHANNEL_KEY_MISSING"))?;
            validate_saved_probe_endpoint(&saved, &channel)?;
            let key = insar_download::with_desktop_agent_model_key(saved_id, |key| key.to_owned())
                .map_err(|_| AgentChannelTest::code("CREDENTIAL_STORE_UNAVAILABLE"))?
                .ok_or_else(|| AgentChannelTest::code("AGENT_CHANNEL_KEY_MISSING"))?;
            let (k, r) = crate::oauth_login::resolve_runtime_secret_and_route(
                &self.application,
                saved_id,
                &key,
                route,
            );
            Ok((r, k))
        })() {
            Ok(pair) => pair,
            Err(failure) => return Ok(failure),
        };

        let client = match model_client(&self.application, Duration::from_secs(45)) {
            Ok(c) => c,
            Err(_) => return Ok(AgentChannelTest::code("AGENT_MODEL_UNAVAILABLE")),
        };

        let messages = [Message {
            role: "user".into(),
            content: "ping".into(),
        }];

        let mut completion_res = block_on(client.send(
            &resolved_route,
            resolved_key.as_str(),
            &messages,
            &[],
            &CancellationToken::new(),
            |_| {},
        ));
        if matches!(&completion_res, Err(err) if err.status == Some(401))
            && draft.api_key.as_deref().is_none_or(|k| k.trim().is_empty())
        {
            if let Some(saved_id) = id {
                if let Some((refreshed_key, refreshed_route)) =
                    crate::oauth_login::force_refresh_or_reload_codex_oauth(
                        &self.application,
                        saved_id,
                        resolved_route.clone(),
                    )
                {
                    completion_res = block_on(client.send(
                        &refreshed_route,
                        refreshed_key.as_str(),
                        &messages,
                        &[],
                        &CancellationToken::new(),
                        |_| {},
                    ));
                }
            }
        }
        if matches!(&completion_res, Err(err) if err.status == Some(400)) && is_https {
            let fallback_route = resolved_route
                .clone()
                .with_omit_stream_options(is_chat_completions)
                .with_max_output_tokens(None);
            completion_res = block_on(client.send(
                &fallback_route,
                resolved_key.as_str(),
                &messages,
                &[],
                &CancellationToken::new(),
                |_| {},
            ));
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;

        Ok(match completion_res {
            Ok(completion) => {
                let preview = completion
                    .text
                    .chars()
                    .take(120)
                    .collect::<String>()
                    .trim()
                    .to_owned();
                AgentChannelTest {
                    ok: true,
                    code: None,
                    status: Some(200),
                    model_count: None,
                    latency_ms: Some(elapsed_ms),
                    message: Some(if preview.is_empty() {
                        "ok".to_string()
                    } else {
                        preview
                    }),
                }
            }
            Err(error) => {
                let mut test = AgentChannelTest::model(error);
                test.latency_ms = Some(elapsed_ms);
                test
            }
        })
    }

    pub fn channels_probe_draft(
        &self,
        draft: AgentChannelDraft,
    ) -> Result<AgentChannelProbe, HostError> {
        let channels = self.channels()?;
        let id = draft.id.as_deref().filter(|id| !id.is_empty());
        let channel = ChannelConfig {
            id: id.unwrap_or("draft").to_owned(),
            protocol: draft.protocol,
            base_url: draft.base_url,
            model: draft
                .model
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| "draft".to_owned()),
            reasoning_mode: ReasoningMode::Off,
            reasoning_switch_unsupported: false,
            models: vec![],
        };
        let start = std::time::Instant::now();
        let result = (|| {
            let route = channel
                .validated_route()
                .map_err(|_| AgentChannelTest::code("AGENT_CHANNEL_INVALID"))?;
            let client = model_client(&self.application, Duration::from_secs(20))
                .map_err(|_| AgentChannelTest::code("AGENT_MODEL_UNAVAILABLE"))?;
            if let Some(key) = draft
                .api_key
                .as_deref()
                .filter(|key| !key.trim().is_empty())
            {
                let (resolved_key, resolved_route) =
                    crate::oauth_login::resolve_runtime_secret_and_route(
                        &self.application,
                        &channel.id,
                        key,
                        route.clone(),
                    );
                return block_on(client.models_with_headers(
                    &resolved_route.endpoint,
                    resolved_key.as_str(),
                    &resolved_route.extra_headers,
                    &CancellationToken::new(),
                ))
                .map_err(AgentChannelTest::model);
            }
            let saved_id = id.ok_or_else(|| AgentChannelTest::code("AGENT_CHANNEL_KEY_MISSING"))?;
            let saved = channels
                .saved(saved_id)
                .map_err(|_| AgentChannelTest::code("AGENT_CHANNEL_INVALID"))?
                .ok_or_else(|| AgentChannelTest::code("AGENT_CHANNEL_KEY_MISSING"))?;
            validate_saved_probe_endpoint(&saved, &channel)?;
            let mut attempt_res = insar_download::with_desktop_agent_model_key(saved_id, |key| {
                let (resolved_key, resolved_route) =
                    crate::oauth_login::resolve_runtime_secret_and_route(
                        &self.application,
                        saved_id,
                        key,
                        route.clone(),
                    );
                block_on(client.models_with_headers(
                    &resolved_route.endpoint,
                    resolved_key.as_str(),
                    &resolved_route.extra_headers,
                    &CancellationToken::new(),
                ))
            })
            .map_err(|_| AgentChannelTest::code("CREDENTIAL_STORE_UNAVAILABLE"))?
            .ok_or_else(|| AgentChannelTest::code("AGENT_CHANNEL_KEY_MISSING"))?;
            if matches!(&attempt_res, Err(err) if err.status == Some(401)) {
                if let Some((refreshed_key, refreshed_route)) =
                    crate::oauth_login::force_refresh_or_reload_codex_oauth(
                        &self.application,
                        saved_id,
                        route.clone(),
                    )
                {
                    attempt_res = block_on(client.models_with_headers(
                        &refreshed_route.endpoint,
                        refreshed_key.as_str(),
                        &refreshed_route.extra_headers,
                        &CancellationToken::new(),
                    ));
                }
            }
            attempt_res.map_err(AgentChannelTest::model)
        })();
        let elapsed_ms = start.elapsed().as_millis() as u64;
        Ok(match result {
            Ok(models) => AgentChannelProbe {
                result: channel_test_result(Ok(models.clone()), elapsed_ms),
                models,
            },
            Err(mut error) => {
                error.latency_ms = Some(elapsed_ms);
                AgentChannelProbe {
                    result: error,
                    models: vec![],
                }
            }
        })
    }
}

fn validate_saved_probe_endpoint(
    saved: &ChannelConfig,
    draft: &ChannelConfig,
) -> Result<(), AgentChannelTest> {
    let valid_protocol = matches!(
        draft.protocol.as_str(),
        "chat_completions" | "responses" | "anthropic_messages" | "google_gemini"
    );
    if !valid_protocol
        || saved.base_url.trim_end_matches('/') != draft.base_url.trim_end_matches('/')
    {
        return Err(AgentChannelTest::code("AGENT_CHANNEL_KEY_ENDPOINT_CHANGED"));
    }
    Ok(())
}

fn channel_test_result(
    result: Result<Vec<String>, AgentChannelTest>,
    elapsed_ms: u64,
) -> AgentChannelTest {
    match result {
        Ok(models) => AgentChannelTest {
            ok: true,
            code: None,
            status: Some(200),
            model_count: Some(models.len() as u64),
            latency_ms: Some(elapsed_ms),
            message: None,
        },
        Err(mut failure) => {
            failure.latency_ms = Some(elapsed_ms);
            failure
        }
    }
}

fn delete_channel(
    channels: &ChannelStore,
    channel_id: &str,
    keys: &impl ChannelKeys,
) -> Result<(), HostError> {
    if crate::eostage_account::is_official_channel_id(channel_id) {
        return Err(HostError::Invalid);
    }
    channels.delete(channel_id, keys).map_err(channel_error)
}

/// Saves one channel. An absent or null `reasoning_mode` keeps the saved
/// choice (forms older than contract 3.31.0 do not send it); a new channel
/// starts with reasoning off.
fn save_channel(
    channels: &ChannelStore,
    save: AgentChannelSave,
    keys: &impl ChannelKeys,
) -> Result<(), HostError> {
    let AgentChannelSave {
        mut id,
        protocol,
        mut base_url,
        model,
        reasoning_mode,
        models,
        mut api_key,
    } = save;
    let trimmed_id = id.trim();
    let is_official = crate::eostage_account::is_official_channel_id(trimmed_id);
    let saved = channels.saved(trimmed_id).map_err(channel_error)?;
    if is_official {
        if trimmed_id != crate::eostage_account::OFFICIAL_CHANNEL_ID {
            return Err(HostError::Invalid);
        }
        if saved.is_none() {
            return Err(HostError::Invalid);
        }
        if api_key.as_deref().is_some_and(|k| !k.trim().is_empty()) {
            return Err(HostError::Invalid);
        }
        let official_base = crate::eostage_account::proxy_base_url();
        let incoming_base = base_url.trim().trim_end_matches('/');
        if !incoming_base.is_empty() && incoming_base != official_base.trim_end_matches('/') {
            return Err(HostError::Invalid);
        }
        id = trimmed_id.to_owned();
        base_url = official_base;
        api_key = None;
    }
    let reasoning_mode = match reasoning_mode {
        Some(mode) => mode,
        None => saved
            .as_ref()
            .map(|channel| channel.reasoning_mode)
            .unwrap_or_default(),
    };
    // An absent list keeps the saved one (forms before contract 3.36.0).
    // `is_free` is strictly server/admin-authoritative for the official channel and
    // never user-editable or permitted on third-party/custom channels.
    let server_free_by_id: std::collections::HashMap<String, Option<bool>> = saved
        .as_ref()
        .filter(|_| is_official)
        .map(|channel| {
            channel
                .models
                .iter()
                .map(|m| (m.id.clone(), m.is_free))
                .collect()
        })
        .unwrap_or_default();
    let mut models = match models {
        Some(models) => models,
        None => saved
            .as_ref()
            .map(|channel| channel.models.clone())
            .unwrap_or_default(),
    };
    for m in &mut models {
        m.is_free = if is_official {
            server_free_by_id.get(&m.id).copied().flatten()
        } else {
            None
        };
    }
    let model = if model.trim().is_empty() && !models.is_empty() {
        models[0].id.clone()
    } else {
        model
    };
    channels
        .save(
            ChannelConfig {
                id,
                protocol,
                base_url,
                model,
                reasoning_mode,
                // Kept or cleared by the store from the saved channel.
                reasoning_switch_unsupported: false,
                models,
            },
            api_key.as_deref().filter(|key| !key.is_empty()),
            keys,
        )
        .map_err(channel_error)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentDisabledItems {
    #[serde(default)]
    pub disabled_providers: Vec<String>,
    #[serde(default)]
    pub disabled_models: Vec<String>,
    #[serde(default)]
    pub disabled_skills: Vec<String>,
}

impl AgentDisabledItems {
    fn is_provider_disabled(&self, channel_id: &str) -> bool {
        self.disabled_providers.iter().any(|id| id == channel_id)
    }

    fn is_model_disabled(&self, channel_id: &str, model_id: &str) -> bool {
        let qualified = format!("{channel_id}::{model_id}");
        self.disabled_models
            .iter()
            .any(|id| id == model_id || id == &qualified)
    }
}

pub fn load_disabled_items_at(app_data_dir: &Path) -> AgentDisabledItems {
    let path = app_data_dir.join("agent").join("disabled-items.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return AgentDisabledItems::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save_disabled_items_at(
    app_data_dir: &Path,
    items: &AgentDisabledItems,
) -> Result<(), HostError> {
    let dir = app_data_dir.join("agent");
    std::fs::create_dir_all(&dir).map_err(|_| HostError::StoreUnavailable)?;
    let data = serde_json::to_vec_pretty(items).map_err(|_| HostError::Invalid)?;
    let tmp = dir.join("disabled-items.tmp");
    std::fs::write(&tmp, data).map_err(|_| HostError::StoreUnavailable)?;
    std::fs::rename(&tmp, dir.join("disabled-items.json")).map_err(|_| HostError::StoreUnavailable)
}

fn channel_error(error: ChannelError) -> HostError {
    match error {
        ChannelError::Invalid => HostError::Invalid,
        ChannelError::Unavailable => HostError::ModelUnavailable,
    }
}

/// In-process key transfer capability used exclusively by atomic channel rename
/// so saved credentials migrate inside Rust without exposing secret values.
trait ChannelKeyTransfer: ChannelKeys {
    fn load_key(&self, channel_id: &str) -> Result<Option<Zeroizing<String>>, ChannelError>;
}

static CHANNEL_RENAME_LOCK: Mutex<()> = Mutex::new(());

fn write_channel_configs_atomic(
    root: &std::path::Path,
    values: &[ChannelConfig],
) -> Result<(), ChannelError> {
    std::fs::create_dir_all(root).map_err(|_| ChannelError::Unavailable)?;
    let data = serde_json::to_vec(values).map_err(|_| ChannelError::Invalid)?;
    let tmp = root.join("agent-channels.tmp");
    std::fs::write(&tmp, data).map_err(|_| ChannelError::Unavailable)?;
    std::fs::rename(&tmp, root.join("agent-channels.json")).map_err(|_| ChannelError::Unavailable)
}

fn is_valid_channel_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !is_reserved_channel_id(id)
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn rename_channel(
    channels: &ChannelStore,
    old_id: &str,
    new_id: &str,
    api_key_override: Option<&str>,
    keys: &impl ChannelKeyTransfer,
) -> Result<(), HostError> {
    let _guard = CHANNEL_RENAME_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let old_trimmed = old_id.trim();
    let new_trimmed = new_id.trim();
    if old_trimmed.is_empty()
        || is_reserved_channel_id(old_trimmed)
        || crate::eostage_account::is_official_channel_id(old_trimmed)
        || crate::eostage_account::is_official_channel_id(new_trimmed)
        || !is_valid_channel_id(new_trimmed)
    {
        return Err(HostError::Invalid);
    }
    let override_key = api_key_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if override_key.is_some_and(|value| value.len() > 16_384) {
        return Err(HostError::Invalid);
    }

    let list = channels.list(keys).map_err(channel_error)?;
    let Some((old_config, old_key_ready)) =
        list.iter().find(|(ch, _)| ch.id == old_trimmed).cloned()
    else {
        return Err(HostError::NotFound);
    };

    if old_trimmed == new_trimmed {
        if let Some(secret) = override_key {
            keys.save_key(old_trimmed, secret).map_err(channel_error)?;
        }
        return Ok(());
    }

    if list.iter().any(|(ch, _)| ch.id == new_trimmed) {
        return Err(HostError::Invalid);
    }

    let mut validated_target = old_config;
    validated_target.id = new_trimmed.to_owned();
    for m in &mut validated_target.models {
        m.is_free = None;
    }
    validated_target.validated_route().map_err(channel_error)?;

    // Resolve secret to write under `new_id` BEFORE modifying any file or key.
    // If `old_id` had a saved key (`old_key_ready`) and reading it fails or
    // returns empty, abort immediately so neither `agent-channels.json` nor
    // `old_id`'s credential entry is touched.
    let secret_to_write: Option<Zeroizing<String>> = if let Some(secret) = override_key {
        Some(Zeroizing::new(secret.to_owned()))
    } else if old_key_ready {
        let loaded = keys.load_key(old_trimmed).map_err(channel_error)?;
        let Some(secret) = loaded.filter(|value| !value.is_empty()) else {
            return Err(HostError::ModelUnavailable);
        };
        Some(secret)
    } else {
        None
    };

    if let Some(ref secret) = secret_to_write {
        keys.save_key(new_trimmed, secret.as_str())
            .map_err(channel_error)?;
        match keys.has_key(new_trimmed) {
            Ok(true) => {}
            _ => {
                let _ = keys.delete_key(new_trimmed);
                return Err(HostError::ModelUnavailable);
            }
        }
    }

    let was_default = channels.default_id().map_err(channel_error)?.as_deref() == Some(old_trimmed);

    let updated: Vec<ChannelConfig> = list
        .into_iter()
        .map(|(mut ch, _)| {
            if ch.id == old_trimmed {
                ch.id = new_trimmed.to_owned();
            }
            ch
        })
        .collect();

    if let Err(err) = write_channel_configs_atomic(channels.root(), &updated) {
        if secret_to_write.is_some() {
            let _ = keys.delete_key(new_trimmed);
        }
        return Err(channel_error(err));
    }

    if was_default {
        channels.set_default(new_trimmed).map_err(channel_error)?;
    }

    if old_key_ready {
        let _ = keys.delete_key(old_trimmed);
    }

    Ok(())
}

/// Keys go to the system credential store through the existing credential
/// module; this type never reads a key back except to answer "is one saved".
pub struct DesktopKeys;
impl ChannelKeys for DesktopKeys {
    fn save_key(&self, channel_id: &str, key: &str) -> Result<(), ChannelError> {
        insar_download::store_desktop_agent_model_key(channel_id, key)
            .map_err(|_| ChannelError::Unavailable)
    }
    fn has_key(&self, channel_id: &str) -> Result<bool, ChannelError> {
        insar_download::desktop_agent_model_key_ready(channel_id)
            .map_err(|_| ChannelError::Unavailable)
    }
    fn delete_key(&self, channel_id: &str) -> Result<bool, ChannelError> {
        insar_download::delete_desktop_agent_model_key(channel_id)
            .map_err(|_| ChannelError::Unavailable)
    }
}
impl ChannelKeyTransfer for DesktopKeys {
    fn load_key(&self, channel_id: &str) -> Result<Option<Zeroizing<String>>, ChannelError> {
        insar_download::with_desktop_agent_model_key(channel_id, |key| {
            Zeroizing::new(key.to_owned())
        })
        .map_err(|_| ChannelError::Unavailable)
    }
}

fn proxy_policy_from_network_settings(
    settings: &insar_application_service::ApplicationNetworkSettings,
) -> insar_core::ProxyPolicy {
    use insar_application_service::NetworkProxyMode;
    match settings.proxy_mode() {
        NetworkProxyMode::Direct => insar_core::ProxyPolicy::Direct,
        NetworkProxyMode::Auto => insar_core::ProxyPolicy::System,
        NetworkProxyMode::Manual => {
            let raw = settings.proxy_url.trim();
            if let Ok(url) = reqwest::Url::parse(raw) {
                if let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) {
                    if let Ok(manual) = insar_core::ManualProxy::new(host, port) {
                        return insar_core::ProxyPolicy::Manual(manual);
                    }
                }
            }
            let authority = raw
                .strip_prefix("http://")
                .or_else(|| raw.strip_prefix("https://"))
                .unwrap_or(raw)
                .split('/')
                .next()
                .unwrap_or("");
            if let Some((host, port_str)) = authority.rsplit_once(':') {
                if let Ok(port) = port_str.parse::<u16>() {
                    if let Ok(manual) = insar_core::ManualProxy::new(host, port) {
                        return insar_core::ProxyPolicy::Manual(manual);
                    }
                }
            }
            insar_core::ProxyPolicy::Direct
        }
    }
}

/// The default channel (or, without one, the first with a saved key) drives
/// new runs. A refused reasoning switch is written back to that channel
/// (W3o), so later runs send none.
fn model_client(
    application: &AgentApplication,
    timeout: Duration,
) -> Result<ModelClient, ModelError> {
    use insar_application_service::NetworkProxyMode;
    let settings = application
        .0
        .get_network_settings()
        .map_err(|_| ModelError::new(insar_agent_model::ErrorKind::Parameter, None))?;
    let proxy = match settings.proxy_mode() {
        NetworkProxyMode::Auto => ModelProxy::System,
        NetworkProxyMode::Direct => ModelProxy::Direct,
        NetworkProxyMode::Manual => ModelProxy::Manual(settings.proxy_url.trim()),
    };
    ModelClient::with_proxy(timeout, proxy)
}

pub struct DesktopModels {
    root: PathBuf,
    application: AgentApplication,
}
fn session_prompt_cache_key(session_id: &str) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(session_id.as_bytes());
    let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!("eostage-{hex}")
}

impl DesktopModels {
    fn open_internal(
        &self,
        session_id: Option<&str>,
        selection: Option<&ModelSelection>,
        prefer_reasoning_off: bool,
    ) -> Result<OpenedModel, HostError> {
        let store = ChannelStore::new(self.root.join("agent"));
        let disabled = load_disabled_items_at(&self.root);
        let (channel, auto_model_id) = match selection {
            Some(selection) => {
                if disabled.is_provider_disabled(&selection.channel_id)
                    || disabled.is_model_disabled(&selection.channel_id, &selection.model_id)
                {
                    return Err(HostError::ModelUnavailable);
                }
                let ch = store
                    .list(&DesktopKeys)
                    .map_err(|_| HostError::ModelUnavailable)?
                    .into_iter()
                    .find(|(channel, key_ready)| *key_ready && channel.id == selection.channel_id)
                    .map(|(channel, _)| channel)
                    .ok_or(HostError::ModelUnavailable)?;
                (ch, None)
            }
            None => {
                let default_id = store
                    .default_id()
                    .map_err(|_| HostError::ModelUnavailable)?;
                let ready_list = store
                    .list(&DesktopKeys)
                    .map_err(|_| HostError::ModelUnavailable)?;
                let pick_enabled_model = |ch: &ChannelConfig| -> Option<String> {
                    if !ch.model.trim().is_empty() && !disabled.is_model_disabled(&ch.id, &ch.model)
                    {
                        return Some(ch.model.clone());
                    }
                    ch.effective_models()
                        .into_iter()
                        .find(|m| {
                            !m.id.trim().is_empty() && !disabled.is_model_disabled(&ch.id, &m.id)
                        })
                        .map(|m| m.id)
                };
                let picked = if let Some(ref def_id) = default_id {
                    ready_list
                        .into_iter()
                        .find(|(ch, ready)| {
                            *ready && &ch.id == def_id && !disabled.is_provider_disabled(&ch.id)
                        })
                        .and_then(|(ch, _)| pick_enabled_model(&ch).map(|mid| (ch, mid)))
                } else {
                    ready_list
                        .into_iter()
                        .filter(|(ch, ready)| *ready && !disabled.is_provider_disabled(&ch.id))
                        .find_map(|(ch, _)| pick_enabled_model(&ch).map(|mid| (ch, mid)))
                }
                .ok_or(HostError::ModelUnavailable)?;
                (picked.0, Some(picked.1))
            }
        };
        let (route, model_id) = match selection {
            Some(selection) => {
                let effective_reasoning = if prefer_reasoning_off {
                    let is_auto = selection
                        .reasoning_level
                        .as_deref()
                        .map(str::trim)
                        .is_none_or(|lvl| {
                            lvl.is_empty()
                                || lvl.eq_ignore_ascii_case("auto")
                                || lvl.eq_ignore_ascii_case("default")
                        });
                    if is_auto {
                        Some("off")
                    } else {
                        selection.reasoning_level.as_deref()
                    }
                } else {
                    selection.reasoning_level.as_deref()
                };
                (
                    channel.route_for(&selection.model_id, effective_reasoning),
                    selection.model_id.clone(),
                )
            }
            None => {
                let target_model = auto_model_id.unwrap_or_else(|| channel.model.clone());
                let effective_reasoning = prefer_reasoning_off.then_some("off");
                (
                    channel.route_for(&target_model, effective_reasoning),
                    target_model,
                )
            }
        };
        let route = route.map_err(|_| HostError::ModelUnavailable)?;
        let raw_key = insar_download::with_desktop_agent_model_key(&channel.id, |key| {
            Zeroizing::new(key.to_owned())
        })
        .map_err(|_| HostError::ModelUnavailable)?
        .ok_or(HostError::ModelUnavailable)?;
        let (key, route) = crate::oauth_login::resolve_runtime_secret_and_route(
            &self.application,
            &channel.id,
            raw_key.as_str(),
            route,
        );
        let cache_key = session_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(session_prompt_cache_key);
        let route = route.with_prompt_cache_key(cache_key);
        // Idle read timeout (not total wall-clock cap): slow reasoning models
        // streaming chunks over several minutes will never be cut mid-thought.
        let client = model_client(&self.application, Duration::from_secs(300))
            .map_err(|_| HostError::ModelUnavailable)?;
        let effective_protocol = match route.endpoint.protocol() {
            Protocol::ChatCompletions => "chat_completions",
            Protocol::Responses => "responses",
            Protocol::AnthropicMessages => "anthropic_messages",
            Protocol::GoogleGemini => "google_gemini",
        };
        let route_name = format!("{effective_protocol}/{model_id}");
        // A refused switch is remembered for the channel's own model only.
        let mut model = KeyedModel::new(client, route, key);
        if model_id == channel.model {
            model = model.remembering(ChannelMemory::new(store, channel));
        }
        Ok(OpenedModel {
            model: Box::new(model),
            route: route_name,
        })
    }
}

impl ModelSource for DesktopModels {
    fn open(&self) -> Result<OpenedModel, HostError> {
        self.open_internal(None, None, false)
    }

    /// The composer's choice of channel, model and thinking level; without
    /// one, the default channel and its model.
    fn open_selected(&self, selection: Option<&ModelSelection>) -> Result<OpenedModel, HostError> {
        self.open_internal(None, selection, false)
    }

    fn open_for_session(
        &self,
        session_id: &str,
        selection: Option<&ModelSelection>,
    ) -> Result<OpenedModel, HostError> {
        self.open_internal(Some(session_id), selection, false)
    }

    fn open_for_turn(
        &self,
        session_id: &str,
        selection: Option<&ModelSelection>,
        prefer_reasoning_off: bool,
    ) -> Result<OpenedModel, HostError> {
        let cache_session_id = if prefer_reasoning_off {
            None
        } else {
            Some(session_id)
        };
        self.open_internal(cache_session_id, selection, prefer_reasoning_off)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentChannelView {
    pub id: String,
    pub protocol: String,
    pub base_url: String,
    pub model: String,
    pub reasoning_mode: ReasoningMode,
    /// The provider refused the reasoning switch for this channel's current
    /// settings (W3o); cleared when protocol, base URL or model change.
    pub reasoning_switch_unsupported: bool,
    /// The channel's models (at least its default `model`).
    pub models: Vec<ChannelModel>,
    pub key_ready: bool,
    pub is_default: bool,
}

/// Save request. `api_key` is write-only: absent or empty keeps the saved key.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentChannelSave {
    pub id: String,
    pub protocol: String,
    pub base_url: String,
    pub model: String,
    /// Absent or null keeps the saved value; a new channel starts `off`.
    #[serde(default)]
    pub reasoning_mode: Option<ReasoningMode>,
    /// Absent or null keeps the saved list; an empty list offers `model` only.
    #[serde(default)]
    pub models: Option<Vec<ChannelModel>>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentChannelDraft {
    pub id: Option<String>,
    pub protocol: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: Option<String>,
}

#[derive(Serialize)]
pub struct AgentChannelProbe {
    pub result: AgentChannelTest,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentChannelTest {
    pub ok: bool,
    pub code: Option<String>,
    pub status: Option<u16>,
    pub model_count: Option<u64>,
    pub latency_ms: Option<u64>,
    pub message: Option<String>,
}
impl AgentChannelTest {
    fn code(code: &str) -> Self {
        Self {
            ok: false,
            code: Some(code.to_owned()),
            status: None,
            model_count: None,
            latency_ms: None,
            message: None,
        }
    }
    fn host(error: HostError) -> Self {
        Self::code(error.code())
    }
    fn model(error: ModelError) -> Self {
        let scrubbed_msg =
            insar_agent_model::sanitize_diagnostic_text(&error.sanitized_summary_zh());
        Self {
            ok: false,
            code: Some(if error.transport_timeout {
                "model_timeout".to_owned()
            } else {
                format!("model_{:?}", error.kind).to_lowercase()
            }),
            status: error.status,
            model_count: None,
            latency_ms: None,
            message: (!scrubbed_msg.is_empty()).then_some(scrubbed_msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoKeys;
    impl ChannelKeys for NoKeys {
        fn save_key(&self, _: &str, _: &str) -> Result<(), ChannelError> {
            Ok(())
        }
        fn has_key(&self, _: &str) -> Result<bool, ChannelError> {
            Ok(false)
        }
        fn delete_key(&self, _: &str) -> Result<bool, ChannelError> {
            Ok(false)
        }
    }

    fn save(id: &str, reasoning_mode: Option<ReasoningMode>) -> AgentChannelSave {
        AgentChannelSave {
            id: id.into(),
            protocol: "chat_completions".into(),
            base_url: "https://example.invalid/v1".into(),
            model: "glm-5.3-flash".into(),
            reasoning_mode,
            models: None,
            api_key: None,
        }
    }

    fn mode(channels: &ChannelStore, id: &str) -> ReasoningMode {
        channels.saved(id).unwrap().unwrap().reasoning_mode
    }

    #[test]
    fn model_client_rejects_invalid_network_settings() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-agent-network-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let application = AgentApplication(Arc::new(
            insar_application_service::ApplicationService::new(
                insar_application_service::ApplicationServiceConfig::store(
                    root.join("fixture.sqlite"),
                ),
            ),
        ));
        assert!(model_client(&application, Duration::from_secs(1)).is_ok());
        std::fs::write(root.join("network-settings.json"), "invalid fixture").unwrap();
        assert!(model_client(&application, Duration::from_secs(1)).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saved_key_fallback_requires_same_endpoint_and_valid_protocol() {
        let saved = ChannelConfig {
            id: "saved".into(),
            protocol: "chat_completions".into(),
            base_url: "https://example.invalid/v1".into(),
            model: "fixture".into(),
            reasoning_mode: ReasoningMode::Off,
            reasoning_switch_unsupported: false,
            models: vec![],
        };
        let mut draft = saved.clone();
        draft.base_url.push('/');
        assert!(validate_saved_probe_endpoint(&saved, &draft).is_ok());
        // Per-model protocol override on the exact same base_url is allowed.
        draft.protocol = "responses".into();
        assert!(validate_saved_probe_endpoint(&saved, &draft).is_ok());
        // Invalid protocol name or changed base_url must still be rejected.
        draft.protocol = "unknown_protocol".into();
        assert!(validate_saved_probe_endpoint(&saved, &draft).is_err());
        draft.protocol = "chat_completions".into();
        draft.base_url = "https://other.invalid/v1".into();
        assert_eq!(
            validate_saved_probe_endpoint(&saved, &draft)
                .unwrap_err()
                .code
                .as_deref(),
            Some("AGENT_CHANNEL_KEY_ENDPOINT_CHANGED")
        );
    }

    #[test]
    fn disabled_items_persist_and_exclude_skills_and_empty_oauth_channel_saves() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-disabled-items-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let channels = ChannelStore::new(root.join("agent"));
        let empty_save = AgentChannelSave {
            id: "fresh-oauth".into(),
            protocol: "google_gemini".into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            model: String::new(),
            reasoning_mode: None,
            models: Some(vec![]),
            api_key: None,
        };
        save_channel(&channels, empty_save, &NoKeys).unwrap();
        let loaded = channels.saved("fresh-oauth").unwrap().unwrap();
        assert!(loaded.model.is_empty());
        assert!(loaded.effective_models().is_empty());

        let items = AgentDisabledItems {
            disabled_providers: vec!["fresh-oauth".into()],
            disabled_models: vec!["other::gpt-4o".into()],
            disabled_skills: vec!["sar-coregistration".into()],
        };
        save_disabled_items_at(&root, &items).unwrap();
        assert_eq!(load_disabled_items_at(&root), items);
        let prompt = build_desktop_extension_system_prompt(Some(&root));
        assert!(!prompt.contains("`sar-coregistration`"));
        let skill_res = execute_skill_read(Some(&root), &json!({"skill_id": "sar-coregistration"}));
        assert_eq!(skill_res["error"], "SKILL_DISABLED");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn saving_a_channel_keeps_the_reasoning_choice_when_omitted_and_starts_new_ones_off() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-agent-channels-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let channels = ChannelStore::new(&root);
        // New channel without a choice: off.
        save_channel(&channels, save("fresh", None), &NoKeys).unwrap();
        assert_eq!(mode(&channels, "fresh"), ReasoningMode::Off);
        // Explicit choice is saved.
        save_channel(
            &channels,
            save("fresh", Some(ReasoningMode::ProviderDefault)),
            &NoKeys,
        )
        .unwrap();
        assert_eq!(mode(&channels, "fresh"), ReasoningMode::ProviderDefault);
        // An older form omits it: the saved choice is preserved.
        save_channel(&channels, save("fresh", None), &NoKeys).unwrap();
        assert_eq!(mode(&channels, "fresh"), ReasoningMode::ProviderDefault);
        // An explicit off overrides it.
        save_channel(&channels, save("fresh", Some(ReasoningMode::Off)), &NoKeys).unwrap();
        assert_eq!(mode(&channels, "fresh"), ReasoningMode::Off);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// W3o: the settings form never carries the remembered refusal; saving
    /// the same endpoint keeps it, a new base URL or model clears it.
    #[test]
    fn saving_a_channel_keeps_a_remembered_refusal_only_for_the_same_endpoint() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-agent-channels-refusal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let channels = ChannelStore::new(&root);
        let refused = |channels: &ChannelStore| {
            channels
                .saved("glm")
                .unwrap()
                .unwrap()
                .reasoning_switch_unsupported
        };
        save_channel(&channels, save("glm", None), &NoKeys).unwrap();
        let used = channels.saved("glm").unwrap().unwrap();
        assert!(channels
            .remember_reasoning_switch_unsupported(&used)
            .unwrap());
        save_channel(&channels, save("glm", None), &NoKeys).unwrap();
        assert!(refused(&channels));
        let mut moved = save("glm", None);
        moved.base_url = "https://other.invalid/v1".into();
        save_channel(&channels, moved, &NoKeys).unwrap();
        assert!(!refused(&channels));
        save_channel(&channels, save("glm", None), &NoKeys).unwrap();
        channels
            .remember_reasoning_switch_unsupported(&used)
            .unwrap();
        let mut renamed = save("glm", None);
        renamed.model = "glm-4.6".into();
        save_channel(&channels, renamed, &NoKeys).unwrap();
        assert!(!refused(&channels));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn channels_test_draft_succeeds_on_real_inference_response() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{}/v1", addr);

        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let len = socket.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..len]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&request);
            assert!(head.starts_with("POST /v1/chat/completions"));
            let frame = "data: {\"choices\":[{\"delta\":{\"content\":\"pong\"}}]}

data: [DONE]

";
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                frame.len(),
                frame
            );
            socket.write_all(reply.as_bytes()).unwrap();
        });

        let root = std::env::temp_dir().join(format!(
            "sceneport-test-channels-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let application = AgentApplication(Arc::new(
            insar_application_service::ApplicationService::new(
                insar_application_service::ApplicationServiceConfig::store(
                    root.join("fixture.sqlite"),
                ),
            ),
        ));
        let state = AgentHostState {
            application,
            root: Some(root.clone()),
            host: Mutex::new(None),
            notifier: Mutex::new(None),
            search_sync_registered: std::sync::atomic::AtomicBool::new(false),
        };

        let result = state
            .channels_test_draft(AgentChannelDraft {
                id: None,
                protocol: "chat_completions".into(),
                base_url,
                api_key: Some("dummy-key".into()),
                model: Some("gpt-4o".into()),
            })
            .unwrap();

        assert!(result.ok);
        assert_eq!(result.status, Some(200));
        assert!(result.latency_ms.is_some());
        assert_eq!(result.message.as_deref(), Some("pong"));
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn channels_test_draft_returns_classified_error_on_http_failure() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{}/v1", addr);

        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let len = socket.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..len]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let reply =
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            socket.write_all(reply.as_bytes()).unwrap();
        });

        let root = std::env::temp_dir().join(format!(
            "sceneport-test-channels-fail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let application = AgentApplication(Arc::new(
            insar_application_service::ApplicationService::new(
                insar_application_service::ApplicationServiceConfig::store(
                    root.join("fixture.sqlite"),
                ),
            ),
        ));
        let state = AgentHostState {
            application,
            root: Some(root.clone()),
            host: Mutex::new(None),
            notifier: Mutex::new(None),
            search_sync_registered: std::sync::atomic::AtomicBool::new(false),
        };

        let result = state
            .channels_test_draft(AgentChannelDraft {
                id: None,
                protocol: "chat_completions".into(),
                base_url,
                api_key: Some("dummy-key".into()),
                model: Some("gpt-4o".into()),
            })
            .unwrap();

        assert!(!result.ok);
        assert_eq!(result.status, Some(401));
        assert_eq!(result.code.as_deref(), Some("model_authentication"));
        assert!(result.latency_ms.is_some());
        assert!(!result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("dummy-key"));
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn channels_test_draft_fails_fast_when_model_is_empty() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-test-channels-empty-model-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let application = AgentApplication(Arc::new(
            insar_application_service::ApplicationService::new(
                insar_application_service::ApplicationServiceConfig::store(
                    root.join("fixture.sqlite"),
                ),
            ),
        ));
        let state = AgentHostState {
            application,
            root: Some(root.clone()),
            host: Mutex::new(None),
            notifier: Mutex::new(None),
            search_sync_registered: std::sync::atomic::AtomicBool::new(false),
        };

        let result = state
            .channels_test_draft(AgentChannelDraft {
                id: None,
                protocol: "chat_completions".into(),
                base_url: "http://127.0.0.1:8080/v1".into(),
                api_key: Some("dummy-key".into()),
                model: None,
            })
            .unwrap();

        assert!(!result.ok);
        assert_eq!(result.code.as_deref(), Some("AGENT_CHANNEL_INVALID"));
        assert_eq!(
            result.message.as_deref(),
            Some("请先添加或选择模型后再测试连接")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[derive(Default)]
    struct RecordedKeys {
        entries: Mutex<std::collections::HashMap<String, String>>,
        ops: Mutex<Vec<String>>,
        fail_load: bool,
    }
    impl ChannelKeys for RecordedKeys {
        fn save_key(&self, id: &str, key: &str) -> Result<(), ChannelError> {
            self.ops.lock().unwrap().push(format!("save:{id}"));
            self.entries
                .lock()
                .unwrap()
                .insert(id.to_owned(), key.to_owned());
            Ok(())
        }
        fn has_key(&self, id: &str) -> Result<bool, ChannelError> {
            Ok(self.entries.lock().unwrap().contains_key(id))
        }
        fn delete_key(&self, id: &str) -> Result<bool, ChannelError> {
            self.ops.lock().unwrap().push(format!("delete:{id}"));
            Ok(self.entries.lock().unwrap().remove(id).is_some())
        }
    }
    impl ChannelKeyTransfer for RecordedKeys {
        fn load_key(&self, id: &str) -> Result<Option<Zeroizing<String>>, ChannelError> {
            self.ops.lock().unwrap().push(format!("load:{id}"));
            if self.fail_load {
                return Err(ChannelError::Unavailable);
            }
            Ok(self
                .entries
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .map(Zeroizing::new))
        }
    }

    #[test]
    fn renaming_channel_migrates_key_preserves_order_and_default_and_deletes_old_key_last() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-channel-rename-ok-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let channels = ChannelStore::new(&root);
        let keys = RecordedKeys::default();
        let mut first = save("alpha", Some(ReasoningMode::ProviderDefault));
        first.api_key = Some("sk-alpha-secret".into());
        save_channel(&channels, first, &keys).unwrap();
        save_channel(&channels, save("beta", None), &keys).unwrap();
        channels.set_default("alpha").unwrap();
        keys.ops.lock().unwrap().clear();

        rename_channel(&channels, "alpha", "alpha-pro", None, &keys).unwrap();

        let listed = channels.list(&keys).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0.id, "alpha-pro");
        assert_eq!(listed[0].0.reasoning_mode, ReasoningMode::ProviderDefault);
        assert!(listed[0].1, "new channel has migrated key");
        assert_eq!(listed[1].0.id, "beta");
        assert_eq!(channels.default_id().unwrap().as_deref(), Some("alpha-pro"));
        assert_eq!(
            keys.entries
                .lock()
                .unwrap()
                .get("alpha-pro")
                .map(String::as_str),
            Some("sk-alpha-secret")
        );
        assert!(!keys.entries.lock().unwrap().contains_key("alpha"));
        let ops = keys.ops.lock().unwrap().clone();
        assert_eq!(
            ops,
            vec!["load:alpha", "save:alpha-pro", "delete:alpha"],
            "old credential is deleted strictly after new credential save succeeds"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn renaming_channel_aborts_without_data_loss_when_key_read_fails() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-channel-rename-abort-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let channels = ChannelStore::new(&root);
        let mut keys = RecordedKeys::default();
        let mut first = save("prod", None);
        first.api_key = Some("sk-prod-must-survive".into());
        save_channel(&channels, first, &keys).unwrap();
        channels.set_default("prod").unwrap();
        keys.fail_load = true;
        keys.ops.lock().unwrap().clear();

        let err = rename_channel(&channels, "prod", "prod-renamed", None, &keys).unwrap_err();
        assert_eq!(err.code(), "AGENT_MODEL_UNAVAILABLE");

        // Original channel metadata, default pointer, and credential remain 100% intact!
        assert!(channels.saved("prod").unwrap().is_some());
        assert!(channels.saved("prod-renamed").unwrap().is_none());
        assert_eq!(channels.default_id().unwrap().as_deref(), Some("prod"));
        assert_eq!(
            keys.entries.lock().unwrap().get("prod").map(String::as_str),
            Some("sk-prod-must-survive")
        );
        let ops = keys.ops.lock().unwrap().clone();
        assert_eq!(ops, vec!["load:prod"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn renaming_channel_rejects_duplicate_or_invalid_target_id_without_touching_keys() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-channel-rename-dup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let channels = ChannelStore::new(&root);
        let keys = RecordedKeys::default();
        let mut first = save("one", None);
        first.api_key = Some("sk-one".into());
        save_channel(&channels, first, &keys).unwrap();
        save_channel(&channels, save("two", None), &keys).unwrap();
        keys.ops.lock().unwrap().clear();

        assert_eq!(
            rename_channel(&channels, "one", "two", None, &keys)
                .unwrap_err()
                .code(),
            "AGENT_INVALID_REQUEST"
        );
        assert_eq!(
            rename_channel(&channels, "one", "bad id!", None, &keys)
                .unwrap_err()
                .code(),
            "AGENT_INVALID_REQUEST"
        );
        for reserved in ["gee", "gee-credentials", "gee-proj-demo", "gee_token"] {
            assert_eq!(
                rename_channel(&channels, "one", reserved, None, &keys)
                    .unwrap_err()
                    .code(),
                "AGENT_INVALID_REQUEST"
            );
            assert_eq!(
                rename_channel(&channels, reserved, "three", None, &keys)
                    .unwrap_err()
                    .code(),
                "AGENT_INVALID_REQUEST"
            );
            assert_eq!(
                save_channel(&channels, save(reserved, None), &keys)
                    .unwrap_err()
                    .code(),
                "AGENT_INVALID_REQUEST"
            );
        }
        assert!(keys.ops.lock().unwrap().is_empty());
        assert_eq!(
            keys.entries.lock().unwrap().get("one").map(String::as_str),
            Some("sk-one")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn official_channel_save_and_delete_lock_base_url_key_and_creation() {
        let root = std::env::temp_dir().join(format!(
            "eostage-official-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let channels = ChannelStore::new(&root);
        let keys = RecordedKeys::default();

        // 1. Logged-out: cannot forge official channel via save_channel
        let mut forge = save(crate::eostage_account::OFFICIAL_CHANNEL_ID, None);
        forge.base_url = "https://evil.example/v1".into();
        assert_eq!(
            save_channel(&channels, forge, &keys).unwrap_err().code(),
            "AGENT_INVALID_REQUEST"
        );
        assert_eq!(
            save_channel(&channels, save("eostage-plan", None), &keys)
                .unwrap_err()
                .code(),
            "AGENT_INVALID_REQUEST"
        );

        // 2. Logged-in official channel seeded by eostage_account
        let official_url = crate::eostage_account::proxy_base_url();
        channels
            .save(
                ChannelConfig {
                    id: crate::eostage_account::OFFICIAL_CHANNEL_ID.into(),
                    protocol: "chat_completions".into(),
                    base_url: official_url.clone(),
                    model: "glm-5.3-flash".into(),
                    reasoning_mode: ReasoningMode::ProviderDefault,
                    reasoning_switch_unsupported: false,
                    models: vec![],
                },
                Some("eos_sess_secret"),
                &keys,
            )
            .unwrap();

        // 3. Reject overwriting base_url or api_key on official channel
        let mut bad_url = save(crate::eostage_account::OFFICIAL_CHANNEL_ID, None);
        bad_url.base_url = "https://attacker.example/v1".into();
        assert_eq!(
            save_channel(&channels, bad_url, &keys).unwrap_err().code(),
            "AGENT_INVALID_REQUEST"
        );

        let mut bad_key = save(crate::eostage_account::OFFICIAL_CHANNEL_ID, None);
        bad_key.base_url = official_url.clone();
        bad_key.api_key = Some("sk-override".into());
        assert_eq!(
            save_channel(&channels, bad_key, &keys).unwrap_err().code(),
            "AGENT_INVALID_REQUEST"
        );

        // 4. Reject deleting official channel via generic channels_delete
        assert_eq!(
            delete_channel(
                &channels,
                crate::eostage_account::OFFICIAL_CHANNEL_ID,
                &keys
            )
            .unwrap_err()
            .code(),
            "AGENT_INVALID_REQUEST"
        );
        assert_eq!(
            delete_channel(&channels, "eostage-plan", &keys)
                .unwrap_err()
                .code(),
            "AGENT_INVALID_REQUEST"
        );

        // 5. Allow updating model/reasoning while keeping official base_url & session key intact
        let mut ok_update = save(
            crate::eostage_account::OFFICIAL_CHANNEL_ID,
            Some(ReasoningMode::ProviderDefault),
        );
        ok_update.base_url = official_url.clone();
        ok_update.model = "glm-6-preview".into();
        save_channel(&channels, ok_update, &keys).unwrap();
        let saved = channels
            .saved(crate::eostage_account::OFFICIAL_CHANNEL_ID)
            .unwrap()
            .unwrap();
        assert_eq!(saved.base_url, official_url);
        assert_eq!(saved.model, "glm-6-preview");
        assert_eq!(
            keys.entries
                .lock()
                .unwrap()
                .get(crate::eostage_account::OFFICIAL_CHANNEL_ID)
                .map(String::as_str),
            Some("eos_sess_secret")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn save_channel_locks_is_free_to_official_server_truth() {
        let root = std::env::temp_dir().join(format!(
            "eostage-free-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let channels = ChannelStore::new(&root);
        let keys = RecordedKeys::default();

        // 1. Custom channel save MUST strip client-supplied `is_free: Some(true)` -> None
        let mut custom_save = save("custom-openai", None);
        custom_save.model = "gpt-free-spoof".into();
        custom_save.models = Some(vec![ChannelModel {
            id: "gpt-free-spoof".into(),
            is_free: Some(true),
            ..ChannelModel::default()
        }]);
        custom_save.api_key = Some("sk-custom".into());
        save_channel(&channels, custom_save, &keys).unwrap();
        let saved_custom = channels.saved("custom-openai").unwrap().unwrap();
        assert_eq!(saved_custom.models[0].is_free, None);

        // 2. Official channel preserves server-synced `is_free`, ignores client tampering,
        //    and clears `is_free` to None on any locally appended model ID not synced by server.
        let official_url = crate::eostage_account::proxy_base_url();
        channels
            .save(
                ChannelConfig {
                    id: crate::eostage_account::OFFICIAL_CHANNEL_ID.into(),
                    protocol: "chat_completions".into(),
                    base_url: official_url.clone(),
                    model: "neohorse-1.5-flash-preview".into(),
                    reasoning_mode: ReasoningMode::ProviderDefault,
                    reasoning_switch_unsupported: false,
                    models: vec![
                        ChannelModel {
                            id: "neohorse-1.5-flash-preview".into(),
                            is_free: Some(true),
                            ..ChannelModel::default()
                        },
                        ChannelModel {
                            id: "glm-5.3-flash".into(),
                            is_free: Some(false),
                            ..ChannelModel::default()
                        },
                    ],
                },
                Some("eos_sess_secret"),
                &keys,
            )
            .unwrap();

        let mut official_tamper = save(
            crate::eostage_account::OFFICIAL_CHANNEL_ID,
            Some(ReasoningMode::ProviderDefault),
        );
        official_tamper.base_url = official_url;
        official_tamper.model = "neohorse-1.5-flash-preview".into();
        official_tamper.models = Some(vec![
            ChannelModel {
                id: "neohorse-1.5-flash-preview".into(),
                is_free: Some(false), // Attempt to flip true -> false ignored
                ..ChannelModel::default()
            },
            ChannelModel {
                id: "glm-5.3-flash".into(),
                is_free: Some(true), // Attempt to spoof paid model as free ignored
                ..ChannelModel::default()
            },
            ChannelModel {
                id: "local-added-model".into(),
                is_free: Some(true), // Locally added model cannot self-mark free
                ..ChannelModel::default()
            },
        ]);
        save_channel(&channels, official_tamper, &keys).unwrap();

        let saved_official = channels
            .saved(crate::eostage_account::OFFICIAL_CHANNEL_ID)
            .unwrap()
            .unwrap();
        assert_eq!(saved_official.models[0].is_free, Some(true));
        assert_eq!(saved_official.models[1].is_free, Some(false));
        assert_eq!(saved_official.models[2].is_free, None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn gee_extension_tools_dispatch_and_pass_run_safe_value() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-gee-agent-ext-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let gee_plugin_dir = root.join("plugins").join("eostage-gee-plugin");
        let _ = std::fs::create_dir_all(&gee_plugin_dir);
        let _ = std::fs::write(
            gee_plugin_dir.join("plugin.json"),
            r#"{"id":"eostage-gee-plugin","name":"GEE","version":"1.0.0","enabled":true}"#,
        );
        let svc_inner = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::business_store(
                root.join("fixture.sqlite"),
            ),
        );
        svc_inner.startup().expect("business_store startup");
        let application = AgentApplication(Arc::new(svc_inner));
        let svc = DesktopGeeAgentService { inner: application };
        let tools = svc.application_extension_tools();
        // 8 built-in GEE tools + 1 toolset_activate + 3 GIS tools + 3 skill/NISAR tools + 2 disaster/AOI-guard tools + 1 open_geodata_query
        assert_eq!(tools.len(), 18);
        assert!(svc.application_extension_system_prompt().is_some());
        assert!(tools.iter().any(|t| t.name == "global_disaster_search"));
        assert!(tools.iter().any(|t| t.name == "aoi_validate_boundary"));

        let aoi_check = svc
            .application_call_extension_tool(
                "aoi_validate_boundary",
                &json!({
                    "bbox": [97.35, 26.05, 108.55, 34.32],
                    "reference_lat": 30.67,
                    "reference_lon": 104.06
                }),
            )
            .expect("aoi_validate_boundary handled")
            .expect("aoi_validate_boundary ok");
        assert_eq!(
            insar_agent::run::safe_value(&aoi_check)
                .expect("aoi_validate_boundary passes safe_value"),
            aoi_check
        );
        assert_eq!(aoi_check["ok"], true);
        assert_eq!(aoi_check["validation"]["status"], "oversized_warning");
        assert_eq!(aoi_check["validation"]["fallback_applied"], true);

        let quota_tool = tools
            .iter()
            .find(|t| t.name == "gee_quota_status")
            .expect("gee_quota_status registered");
        assert!(quota_tool.parameters["properties"].get("refresh").is_some());

        let quota_res = svc
            .application_call_extension_tool("gee_quota_status", &json!({"refresh": true}))
            .expect("gee_quota_status handled")
            .expect("gee_quota_status ok");
        assert_eq!(
            insar_agent::run::safe_value(&quota_res).expect("gee_quota_status passes safe_value"),
            quota_res
        );
        assert_eq!(quota_res["ok"], true);
        assert_eq!(quota_res["configured"], false);
        assert_eq!(quota_res["account_tier"], "unconfigured");

        let analyze_tool = tools
            .iter()
            .find(|t| t.name == "gee_cloud_analyze")
            .expect("gee_cloud_analyze registered");
        let analyze_props = &analyze_tool.parameters["properties"];
        assert!(analyze_props.get("before_start_date").is_some());
        assert!(analyze_props.get("before_end_date").is_some());
        assert!(analyze_props.get("after_start_date").is_some());
        assert!(analyze_props.get("after_end_date").is_some());
        assert!(analyze_props.get("step_days").is_some());

        let script_tool = tools
            .iter()
            .find(|t| t.name == "gee_script_inspect")
            .expect("gee_script_inspect registered");
        let script_desc = script_tool.parameters["properties"]["script"]["description"]
            .as_str()
            .unwrap_or("");
        assert!(script_desc.contains("AOI_BBOX") && script_desc.contains("BBOX"));
        assert!(script_desc.contains("PREVIEW_IMAGE") && script_desc.contains("PREVIEW_PNG_PATH"));

        let cat_res = svc
            .application_call_extension_tool("gee_catalog_search", &json!({"query": "哨兵2"}))
            .expect("gee_catalog_search handled")
            .expect("catalog search ok");
        insar_agent::run::safe_value(&cat_res).expect("catalog result passes safe_value");

        let status_res = svc
            .application_call_extension_tool(
                "gis_toolchains_status",
                &json!({"probe_versions": false}),
            )
            .expect("gis_toolchains_status handled")
            .expect("gis_toolchains_status ok");
        insar_agent::run::safe_value(&status_res).expect("gis_toolchains_status passes safe_value");
        assert_eq!(status_res["ok"], true);
        assert_eq!(
            status_res["toolchains"].as_array().map(|a| a.len()),
            Some(4)
        );
        assert_eq!(status_res["log_dehydrator"]["builtin_ready"], true);
        assert!(status_res["conda_environments"].is_array());
        let ext_tool = tools
            .iter()
            .find(|t| t.name == "gis_external_execute")
            .expect("gis_external_execute registered");
        let toolchain_enum = ext_tool.parameters["properties"]["toolchain"]["enum"]
            .as_array()
            .expect("toolchain enum");
        assert!(toolchain_enum.iter().any(|v| v == "conda"));

        let forbidden_out = svc
            .application_call_extension_tool(
                "gis_external_execute",
                &json!({
                    "toolchain": "qgis",
                    "action": "run_algorithm",
                    "algorithm": "native:filedownloader",
                    "params": { "OUTPUT": "/tmp/arbitrary.bin" }
                }),
            )
            .expect("gis_external_execute handled")
            .expect("gis_external_execute returns sanitized error");
        assert_eq!(forbidden_out["ok"], false);
        assert!(forbidden_out["error"]
            .as_str()
            .unwrap_or("")
            .contains("GIS_EXTERNAL_OUTPUT_PARAM_FORBIDDEN"));

        let forbidden_arcpy_out = svc
            .application_call_extension_tool(
                "gis_external_execute",
                &json!({
                    "toolchain": "arcgis_pro",
                    "action": "run_algorithm",
                    "algorithm": "management.CopyRaster",
                    "params": { "out_rasterdataset": "/tmp/arbitrary.tif" }
                }),
            )
            .expect("gis_external_execute handled")
            .expect("gis_external_execute returns sanitized error");
        assert_eq!(forbidden_arcpy_out["ok"], false);
        assert!(forbidden_arcpy_out["error"]
            .as_str()
            .unwrap_or("")
            .contains("GIS_EXTERNAL_OUTPUT_PARAM_FORBIDDEN"));

        let allowed_non_path_opt = svc
            .application_call_extension_tool(
                "gis_external_execute",
                &json!({
                    "toolchain": "arcgis_pro",
                    "action": "run_algorithm",
                    "algorithm": "management.Project",
                    "output_filename": "hefei_wgs84.shp",
                    "params": { "out_coor_system": "EPSG:4326", "out_cell_size": 30 }
                }),
            )
            .expect("gis_external_execute handled")
            .expect("gis_external_execute returns structured result");
        assert!(
            !allowed_non_path_opt["error"]
                .as_str()
                .unwrap_or("")
                .contains("GIS_EXTERNAL_OUTPUT_PARAM_FORBIDDEN"),
            "non-path options like out_coor_system must not be blocked: {allowed_non_path_opt}"
        );

        let src_geojson = root.join("agent_pt.geojson");
        std::fs::write(
            &src_geojson,
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"id":1},"geometry":{"type":"Point","coordinates":[117.28,31.86]}}]}"#,
        )
        .unwrap();
        let vec_res = svc
            .application_call_extension_tool(
                "gis_local_process",
                &json!({
                    "domain": "vector",
                    "operation": "buffer",
                    "input_path": src_geojson.to_string_lossy().to_string(),
                    "output_filename": "../../escaped/hefei_buf.geojson",
                    "distance_m": 500.0
                }),
            )
            .expect("gis_local_process handled")
            .expect("gis_local_process ok");
        insar_agent::run::safe_value(&vec_res).expect("gis_local_process passes safe_value");
        if crate::geodata_tools::discover_native_gis_engine().is_discovered() {
            assert_eq!(vec_res["ok"], true);
            let produced_name = vec_res["artifact_name"].as_str().expect("artifact_name");
            assert!(produced_name.starts_with("hefei_buf_"));
            assert!(root.join("gis_outputs").join(produced_name).is_file());
        } else {
            assert_eq!(vec_res["ok"], false);
            assert!(vec_res["error"]
                .as_str()
                .unwrap_or("")
                .contains("GIS_ENGINE_UNAVAILABLE"));
        }
        assert!(!root.join("escaped").exists());
        assert!(cat_res["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|it| it["id"] == "COPERNICUS/S2_SR_HARMONIZED"));

        let modis_res = svc
            .application_call_extension_tool("gee_catalog_search", &json!({"query": "MOD11A1"}))
            .expect("gee_catalog_search MOD11A1 handled")
            .expect("MOD11A1 catalog search ok");
        insar_agent::run::safe_value(&modis_res).expect("MOD11A1 result passes safe_value");
        assert!(modis_res["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|it| it["id"] == "MODIS/061/MOD11A1"));

        let era5_res = svc
            .application_call_extension_tool("gee_catalog_search", &json!({"query": "ERA5"}))
            .expect("gee_catalog_search ERA5 handled")
            .expect("ERA5 catalog search ok");
        insar_agent::run::safe_value(&era5_res).expect("ERA5 result passes safe_value");
        assert!(era5_res["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|it| it["id"] == "ECMWF/ERA5_LAND/DAILY_AGGR"));

        let script_res = svc
            .application_call_extension_tool(
                "gee_script_inspect",
                &json!({"script": "col = ee.ImageCollection('COPERNICUS/S2_SR_HARMONIZED').filterBounds(AOI)\nRESULT = {'count': col.size().getInfo()}"}),
            )
            .expect("gee_script_inspect handled")
            .expect("script inspect ok");
        insar_agent::run::safe_value(&script_res).expect("script inspect passes safe_value");
        assert!(script_res["approval_id"]
            .as_str()
            .unwrap_or("")
            .starts_with("gee-appr-"));

        assert!(svc
            .application_call_extension_tool("list_sources", &json!({}))
            .is_none());

        let _ = svc.inner.0.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn gee_extension_gis_local_process_buffers_geojson_with_native_gis() {
        assert!(
            crate::geodata_tools::discover_native_gis_engine().is_discovered(),
            "native GIS runtime must be configured for ignored strict GIS test"
        );
        let root = std::env::temp_dir().join(format!(
            "sceneport-gee-gis-strict-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let gee_plugin_dir = root.join("plugins").join("eostage-gee-plugin");
        let _ = std::fs::create_dir_all(&gee_plugin_dir);
        let _ = std::fs::write(
            gee_plugin_dir.join("plugin.json"),
            r#"{"id":"eostage-gee-plugin","name":"GEE","version":"1.0.0","enabled":true}"#,
        );
        let svc_inner = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::business_store(
                root.join("fixture.sqlite"),
            ),
        );
        svc_inner.startup().expect("business_store startup");
        let svc = DesktopGeeAgentService {
            inner: AgentApplication(Arc::new(svc_inner)),
        };

        let src_geojson = root.join("agent_pt.geojson");
        std::fs::write(
            &src_geojson,
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"id":1},"geometry":{"type":"Point","coordinates":[117.28,31.86]}}]}"#,
        )
        .unwrap();
        let vec_res = svc
            .application_call_extension_tool(
                "gis_local_process",
                &json!({
                    "domain": "vector",
                    "operation": "buffer",
                    "input_path": src_geojson.to_string_lossy().to_string(),
                    "output_filename": "../../escaped/hefei_buf.geojson",
                    "distance_m": 500.0
                }),
            )
            .expect("gis_local_process handled")
            .expect("gis_local_process ok");
        insar_agent::run::safe_value(&vec_res).expect("gis_local_process passes safe_value");
        assert_eq!(vec_res["ok"], true);
        let produced_name = vec_res["artifact_name"].as_str().expect("artifact_name");
        assert!(produced_name.starts_with("hefei_buf_"));
        assert!(root.join("gis_outputs").join(produced_name).is_file());
        assert!(!root.join("escaped").exists());

        let _ = svc.inner.0.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn agent_gee_chunked_download_records_task_in_store_and_passes_safe_value() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let root = std::env::temp_dir().join(format!(
            "sceneport-gee-agent-dl-task-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let gee_plugin_dir = root.join("plugins").join("eostage-gee-plugin");
        let _ = std::fs::create_dir_all(&gee_plugin_dir);
        let _ = std::fs::write(
            gee_plugin_dir.join("plugin.json"),
            r#"{"id":"eostage-gee-plugin","name":"GEE","version":"1.0.0","enabled":true}"#,
        );
        let svc_inner = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::business_store(
                root.join("fixture.sqlite"),
            ),
        );
        svc_inner.startup().expect("business_store startup");
        let application = AgentApplication(Arc::new(svc_inner));
        let svc = DesktopGeeAgentService { inner: application };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");

        // Configure isolated local vault + loopback api_base_url inside `root`
        let vault_doc = json!({
            "eostage:agent-model:gee-credentials": {
                "gee-credentials": json!({
                    "project_id": "ee-agent-dl-test",
                    "auth_mode": "oauth_token",
                    "secret": "agent-dl-token-xyz",
                }).to_string()
            }
        });
        std::fs::write(
            root.join("credentials-vault.v1.json"),
            serde_json::to_vec_pretty(&vault_doc).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("network-settings.json"),
            r#"{"proxy_enabled":false,"proxy_url":"","test_loopback":true}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("gee_credentials.json"),
            json!({
                "configured": true,
                "project_id": "ee-agent-dl-test",
                "auth_mode": "oauth_token",
                "api_base_url": base_url,
            })
            .to_string(),
        )
        .unwrap();

        // 1. Preflight confirmation (> 9 chunks) MUST NOT record a task in TaskStore
        let preflight = svc
            .application_call_extension_tool(
                "gee_chunked_download",
                &json!({
                    "asset_id": "COPERNICUS/S2_SR_HARMONIZED",
                    "index_name": "NDVI",
                    "bbox": [117.10, 31.70, 117.31, 31.90],
                    "scale_m": 10.0,
                }),
            )
            .expect("handled")
            .expect("preflight ok");
        insar_agent::run::safe_value(&preflight).expect("preflight passes safe_value");
        assert_eq!(preflight["quota_confirm_required"], true);
        assert!(preflight.get("task_id").is_none());
        let tasks_before = svc
            .application_task_overview()
            .expect("task_overview before download");
        assert_eq!(
            tasks_before["history"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(0),
            0
        );

        // 2. Small AOI single-chunk download writes stitched GeoTIFF, records TaskStore task, strips paths, and passes safe_value
        let srv = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
            while std::time::Instant::now() < deadline {
                if let Ok((mut s, _)) = listener.accept() {
                    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        match s.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                data.extend_from_slice(&buf[..n]);
                                if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                                    let hdr_txt = String::from_utf8_lossy(&data[..pos]);
                                    let clen = hdr_txt
                                        .lines()
                                        .find_map(|ln| {
                                            let lower = ln.to_ascii_lowercase();
                                            lower
                                                .strip_prefix("content-length:")
                                                .and_then(|v| v.trim().parse::<usize>().ok())
                                        })
                                        .unwrap_or(0);
                                    if data.len() >= pos + 4 + clen {
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let pos = data.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
                    let req_json: Value = serde_json::from_slice(&data[pos + 4..]).unwrap();
                    let w = req_json["grid"]["dimensions"]["width"].as_u64().unwrap() as usize;
                    let h = req_json["grid"]["dimensions"]["height"].as_u64().unwrap() as usize;

                    // Build valid NPY v1.0 structured float32 array [('NDVI', '<f4')] of shape (h, w)
                    let dict = format!(
                        "{{'descr': [('NDVI', '<f4')], 'fortran_order': False, 'shape': ({h}, {w}), }}"
                    );
                    let mut hdr_body = dict.into_bytes();
                    while !(10 + hdr_body.len() + 1).is_multiple_of(64) {
                        hdr_body.push(b' ');
                    }
                    hdr_body.push(b'\n');
                    let mut npy = Vec::new();
                    npy.extend_from_slice(b"\x93NUMPY\x01\x00");
                    npy.extend_from_slice(&(hdr_body.len() as u16).to_le_bytes());
                    npy.extend_from_slice(&hdr_body);
                    for i in 0..(w * h) {
                        let val = 0.25_f32 + ((i % 5) as f32) * 0.1_f32;
                        npy.extend_from_slice(&val.to_le_bytes());
                    }

                    let resp_hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        npy.len()
                    );
                    let _ = s.write_all(resp_hdr.as_bytes());
                    let _ = s.write_all(&npy);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
        });

        let dl_res = svc
            .application_call_extension_tool(
                "gee_chunked_download",
                &json!({
                    "asset_id": "COPERNICUS/S2_SR_HARMONIZED",
                    "index_name": "NDVI",
                    "bbox": [117.250, 31.820, 117.255, 31.825],
                    "scale_m": 100.0,
                }),
            )
            .expect("handled")
            .expect("chunked download ok");
        let _ = srv.join();

        insar_agent::run::safe_value(&dl_res)
            .expect("chunked download sanitized result must pass safe_value");
        assert_eq!(dl_res["ok"], true);
        assert!(
            dl_res.get("output_path").is_none(),
            "output_path must be stripped"
        );
        assert!(
            dl_res.get("output_root").is_none(),
            "output_root must be stripped"
        );
        let task_id = dl_res["task_id"]
            .as_str()
            .expect("task_id must be recorded and returned in Agent tool output");
        assert!(
            task_id.starts_with("geodata-gee_chunked_download-"),
            "unexpected task_id: {task_id}"
        );

        let task_page = svc
            .application_download_task_page(task_id, 0, 20)
            .expect("download_task_page after download");
        assert_eq!(task_page["task_id"], task_id);
        assert_eq!(task_page["total_count"], 1);
        let tasks_after = svc
            .application_task_overview()
            .expect("task_overview after download");
        assert!(tasks_after["history"]
            .as_array()
            .map(|arr| arr.iter().any(|t| {
                t["task_id"] == task_id
                    && t["provider_id"] == "open_geodata.gee_chunked_download"
                    && t["status"] == "completed"
            }))
            .unwrap_or(false));

        let _ = svc.inner.0.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn gee_agent_chunked_download_strips_adversarial_output_path_and_confines_to_default_output_root(
    ) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let uniq = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("sceneport-gee-agent-confinement-{uniq}"));
        let configured_dl_root = root.join("user_configured_output_root");
        let evil_tmp_target = std::env::temp_dir().join(format!("evil_target_{uniq}.tif"));
        let _ = std::fs::remove_file(&evil_tmp_target);
        let _ = std::fs::create_dir_all(&configured_dl_root);
        let gee_plugin_dir = root.join("plugins").join("eostage-gee-plugin");
        let _ = std::fs::create_dir_all(&gee_plugin_dir);
        let _ = std::fs::write(
            gee_plugin_dir.join("plugin.json"),
            r#"{"id":"eostage-gee-plugin","name":"GEE","version":"1.0.0","enabled":true}"#,
        );

        let svc_inner = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::business_store(
                root.join("fixture.sqlite"),
            ),
        );
        svc_inner.startup().expect("business_store startup");
        let mut policy = svc_inner
            .get_sar_download_policy_settings()
            .expect("get policy");
        policy.default_output_root = configured_dl_root.to_string_lossy().into_owned();
        svc_inner
            .save_sar_download_policy_settings(&policy)
            .expect("save default_output_root");

        let application = AgentApplication(Arc::new(svc_inner));
        let svc = DesktopGeeAgentService { inner: application };

        // 1. Verify all advertised GEE extension schemas enforce additionalProperties: false
        //    and neither gee_script_preview nor gee_script_execute is exposed to the LLM.
        let tools = svc.application_extension_tools();
        assert!(!tools.is_empty());
        for td in &tools {
            assert_eq!(
                td.parameters["additionalProperties"],
                json!(false),
                "tool {} schema must declare additionalProperties: false",
                td.name
            );
            assert_ne!(td.name, "gee_script_preview");
            assert_ne!(td.name, "gee_script_execute");
        }
        assert!(svc
            .application_call_extension_tool("gee_script_preview", &json!({"approval_id": "x"}))
            .is_none());
        assert!(svc
            .application_call_extension_tool("gee_script_execute", &json!({"approval_id": "x"}))
            .is_none());

        // 2. Verify collision numbering never overwrites an existing file on disk
        let gee_dl_dir = configured_dl_root.join("gee_downloads");
        std::fs::create_dir_all(&gee_dl_dir).unwrap();
        let pre_existing = gee_dl_dir.join("COPERNICUS_S2_SR_HARMONIZED_NDVI_fixedtok.tif");
        std::fs::write(&pre_existing, b"EXISTING_SENTINEL_BYTES").unwrap();
        let collided = allocate_safe_gee_agent_download_path(
            Some(configured_dl_root.to_str().unwrap()),
            &root,
            "COPERNICUS/S2_SR_HARMONIZED",
            Some("NDVI"),
            "fixedtok",
        );
        assert_eq!(
            collided,
            gee_dl_dir.join("COPERNICUS_S2_SR_HARMONIZED_NDVI_fixedtok_1.tif")
        );
        assert_eq!(
            std::fs::read(&pre_existing).unwrap(),
            b"EXISTING_SENTINEL_BYTES"
        );

        // 3. Spin up loopback GEE server serving 2 computePixels calls
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");

        let vault_doc = json!({
            "eostage:agent-model:gee-credentials": {
                "gee-credentials": json!({
                    "project_id": "ee-confinement-test",
                    "auth_mode": "oauth_token",
                    "secret": "confinement-token-xyz",
                }).to_string()
            }
        });
        std::fs::write(
            root.join("credentials-vault.v1.json"),
            serde_json::to_vec_pretty(&vault_doc).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("network-settings.json"),
            r#"{"proxy_enabled":false,"proxy_url":"","test_loopback":true}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("gee_credentials.json"),
            json!({
                "configured": true,
                "project_id": "ee-confinement-test",
                "auth_mode": "oauth_token",
                "api_base_url": base_url,
            })
            .to_string(),
        )
        .unwrap();

        let srv = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut served = 0;
            while served < 2 && std::time::Instant::now() < deadline {
                if let Ok((mut s, _)) = listener.accept() {
                    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        match s.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                data.extend_from_slice(&buf[..n]);
                                if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                                    let hdr_txt = String::from_utf8_lossy(&data[..pos]);
                                    let clen = hdr_txt
                                        .lines()
                                        .find_map(|ln| {
                                            ln.to_ascii_lowercase()
                                                .strip_prefix("content-length:")
                                                .and_then(|v| v.trim().parse::<usize>().ok())
                                        })
                                        .unwrap_or(0);
                                    if data.len() >= pos + 4 + clen {
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let pos = data.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
                    let req_json: Value = serde_json::from_slice(&data[pos + 4..]).unwrap();
                    let w = req_json["grid"]["dimensions"]["width"].as_u64().unwrap() as usize;
                    let h = req_json["grid"]["dimensions"]["height"].as_u64().unwrap() as usize;
                    let dict = format!(
                        "{{'descr': [('NDVI', '<f4')], 'fortran_order': False, 'shape': ({h}, {w}), }}"
                    );
                    let mut hdr_body = dict.into_bytes();
                    while !(10 + hdr_body.len() + 1).is_multiple_of(64) {
                        hdr_body.push(b' ');
                    }
                    hdr_body.push(b'\n');
                    let mut npy = Vec::new();
                    npy.extend_from_slice(b"\x93NUMPY\x01\x00");
                    npy.extend_from_slice(&(hdr_body.len() as u16).to_le_bytes());
                    npy.extend_from_slice(&hdr_body);
                    for i in 0..(w * h) {
                        let val = 0.3_f32 + ((i % 4) as f32) * 0.05_f32;
                        npy.extend_from_slice(&val.to_le_bytes());
                    }
                    let resp_hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        npy.len()
                    );
                    let _ = s.write_all(resp_hdr.as_bytes());
                    let _ = s.write_all(&npy);
                    served += 1;
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
            }
        });

        // Adversarial Attempt A: absolute path `/tmp/evil_target_<uniq>.tif` + `output_root`
        let res_abs = svc
            .application_call_extension_tool(
                "gee_chunked_download",
                &json!({
                    "asset_id": "COPERNICUS/S2_SR_HARMONIZED",
                    "index_name": "NDVI",
                    "bbox": [117.250, 31.820, 117.255, 31.825],
                    "scale_m": 100.0,
                    "output_path": evil_tmp_target.to_string_lossy(),
                    "output_root": "/tmp",
                }),
            )
            .expect("handled")
            .expect("abs injection safely redirected");
        assert_eq!(res_abs["ok"], true);
        assert!(
            !evil_tmp_target.exists(),
            "adversarial absolute output_path MUST NOT be created: {evil_tmp_target:?}"
        );

        // Adversarial Attempt B: relative traversal `../../evil.tif` + stem sanitization check
        assert_eq!(
            sanitize_gee_agent_filename_stem("../../evil/pwn", Some("../../../etc/passwd")),
            "evil_pwn_etc_passwd"
        );
        let res_rel = svc
            .application_call_extension_tool(
                "gee_chunked_download",
                &json!({
                    "asset_id": "COPERNICUS/S2_SR_HARMONIZED",
                    "index_name": "NDVI",
                    "bbox": [117.250, 31.820, 117.255, 31.825],
                    "scale_m": 100.0,
                    "output_path": "../../evil.tif",
                }),
            )
            .expect("handled")
            .expect("traversal injection safely redirected");
        let _ = srv.join();
        assert_eq!(res_rel["ok"], true);
        assert!(!root.join("evil.tif").exists());
        assert!(!root
            .parent()
            .map(|p| p.join("evil.tif").exists())
            .unwrap_or(false));
        assert_eq!(
            std::fs::read(&pre_existing).unwrap(),
            b"EXISTING_SENTINEL_BYTES",
            "pre-existing file in gee_downloads must never be overwritten"
        );

        // Verify strictly 3 files inside `<default_output_root>/gee_downloads/` (1 pre-existing + 2 new .tif files)
        let written_tifs: Vec<PathBuf> = std::fs::read_dir(&gee_dl_dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|x| x.path()))
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("tif"))
            .collect();
        assert_eq!(
            written_tifs.len(),
            3,
            "expected pre-existing + 2 confined GeoTIFFs inside {gee_dl_dir:?}, got {written_tifs:?}"
        );
        for p in &written_tifs {
            assert!(p.starts_with(&gee_dl_dir));
            if p != &pre_existing {
                let bytes = std::fs::read(p).unwrap();
                assert_eq!(&bytes[0..4], b"II*\0", "valid little-endian GeoTIFF header");
            }
        }
        assert!(
            !root.join("gee_downloads").exists(),
            "when default_output_root is set, app_data_dir/gee_downloads must not be used"
        );

        // Adversarial Attempt C (Unix): dangling symlink at candidate slot must advance to `_1.tif`,
        // and symlinked `gee_downloads` leaf directory must fail closed without writing outside!
        #[cfg(unix)]
        {
            let dangling_slot = gee_dl_dir.join("COPERNICUS_S2_SR_HARMONIZED_NDVI_danglingtok.tif");
            std::os::unix::fs::symlink("/nonexistent_sceneport_target", &dangling_slot).unwrap();
            let next_after_dangling = allocate_safe_gee_agent_download_path(
                Some(configured_dl_root.to_str().unwrap()),
                &root,
                "COPERNICUS/S2_SR_HARMONIZED",
                Some("NDVI"),
                "danglingtok",
            );
            assert_eq!(
                next_after_dangling,
                gee_dl_dir.join("COPERNICUS_S2_SR_HARMONIZED_NDVI_danglingtok_1.tif"),
                "dangling symlink must be treated as occupied via symlink_metadata"
            );

            // Empty default_output_root falls back to `app_data_dir/gee_downloads`
            let fallback_alloc = allocate_safe_gee_agent_download_path(
                Some("   "),
                &root,
                "COPERNICUS/S2_SR_HARMONIZED",
                Some("NDVI"),
                "emptytok",
            );
            assert_eq!(
                fallback_alloc,
                root.join("gee_downloads")
                    .join("COPERNICUS_S2_SR_HARMONIZED_NDVI_emptytok.tif")
            );

            // Symlinked `<app_data_dir>/gee_downloads -> outside_escape_dir` when default_output_root is cleared
            let outside_escape_dir = root.join("outside_symlink_escape");
            std::fs::create_dir_all(&outside_escape_dir).unwrap();
            let mut cleared_policy = svc
                .inner
                .0
                .get_sar_download_policy_settings()
                .expect("get policy");
            cleared_policy.default_output_root = String::new();
            svc.inner
                .0
                .save_sar_download_policy_settings(&cleared_policy)
                .expect("clear default_output_root");
            std::os::unix::fs::symlink(&outside_escape_dir, root.join("gee_downloads")).unwrap();

            let res_symlink = svc
                .application_call_extension_tool(
                    "gee_chunked_download",
                    &json!({
                        "asset_id": "COPERNICUS/S2_SR_HARMONIZED",
                        "index_name": "NDVI",
                        "bbox": [117.250, 31.820, 117.255, 31.825],
                        "scale_m": 100.0,
                    }),
                )
                .expect("handled")
                .expect("trace json returned");
            assert_eq!(res_symlink["ok"], false);
            assert!(
                res_symlink["error"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("UNSAFE_GEOTIFF_OUTPUT_PATH"),
                "expected UNSAFE_GEOTIFF_OUTPUT_PATH, got {res_symlink:?}"
            );
            let escaped_files: Vec<_> = std::fs::read_dir(&outside_escape_dir).unwrap().collect();
            assert!(
                escaped_files.is_empty(),
                "symlinked gee_downloads directory must NEVER receive written files: {escaped_files:?}"
            );
        }

        let _ = svc.inner.0.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn gee_agent_script_inspect_scrubbed_by_outbound_text_roundtrips_via_preview_and_executes() {
        let uniq = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("sceneport-gee-agent-script-{uniq}"));
        let _ = std::fs::create_dir_all(&root);
        let gee_plugin_dir = root.join("plugins").join("eostage-gee-plugin");
        let _ = std::fs::create_dir_all(&gee_plugin_dir);
        let _ = std::fs::write(
            gee_plugin_dir.join("plugin.json"),
            r#"{"id":"eostage-gee-plugin","name":"GEE","version":"1.0.0","enabled":true}"#,
        );

        let svc_inner = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::business_store(
                root.join("fixture.sqlite"),
            ),
        );
        svc_inner.startup().expect("startup");
        let svc = DesktopGeeAgentService {
            inner: AgentApplication(Arc::new(svc_inner)),
        };

        let vault_doc = json!({
            "eostage:agent-model:gee-credentials": {
                "gee-credentials": json!({
                    "project_id": "ee-script-agent-test",
                    "auth_mode": "oauth_token",
                    "secret": "script-agent-token",
                }).to_string()
            }
        });
        std::fs::write(
            root.join("credentials-vault.v1.json"),
            serde_json::to_vec_pretty(&vault_doc).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("gee_credentials.json"),
            json!({
                "configured": true,
                "project_id": "ee-script-agent-test",
                "auth_mode": "oauth_token",
            })
            .to_string(),
        )
        .unwrap();

        let Some(sys_py) = ["python3", "python"].into_iter().find(|cand| {
            std::process::Command::new(cand)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }) else {
            let _ = svc.inner.0.shutdown();
            let _ = std::fs::remove_dir_all(&root);
            return;
        };

        let venv_dir = root.join("gee_python_env");
        let venv_bin = if cfg!(windows) {
            venv_dir.join("Scripts")
        } else {
            venv_dir.join("bin")
        };
        std::fs::create_dir_all(&venv_bin).unwrap();
        std::fs::write(venv_dir.join(".ee_ready_v1"), b"ok\n").unwrap();
        let stub_pkg = venv_dir.join("stub_pkgs");
        std::fs::create_dir_all(stub_pkg.join("google").join("oauth2")).unwrap();
        std::fs::write(stub_pkg.join("google").join("__init__.py"), b"").unwrap();
        std::fs::write(
            stub_pkg.join("google").join("oauth2").join("__init__.py"),
            b"",
        )
        .unwrap();
        std::fs::write(
            stub_pkg.join("google").join("oauth2").join("credentials.py"),
            b"class Credentials:\n    def __init__(self, token=None):\n        self.token = token\n",
        )
        .unwrap();
        std::fs::write(stub_pkg.join("requests.py"), b"").unwrap();
        std::fs::write(
            stub_pkg.join("ee.py"),
            b"class _Geom:\n    @staticmethod\n    def Rectangle(b):\n        return {'rect': b}\nGeometry = _Geom\ndef Initialize(credentials=None, project=None):\n    pass\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let wrapper = venv_bin.join("python");
            std::fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nPYTHONPATH=\"{}\" exec {sys_py} \"$@\"\n",
                    stub_pkg.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let original_script =
            "# 光学/雷达融合\nb8 = 0.62\nb4 = 0.14\ndenom = b8 + b4\nndvi = (b8 - b4)/denom\nRESULT = {'ndvi': round(ndvi, 4), 'aoi': AOI}\n";
        let inspect_out = svc
            .application_call_extension_tool(
                "gee_script_inspect",
                &json!({
                    "script": original_script,
                    "bbox": [117.2, 31.8, 117.3, 31.9],
                }),
            )
            .expect("handled")
            .expect("inspect ok");
        assert_eq!(inspect_out["ok"], true);
        let approval_id = inspect_out["approval_id"].as_str().unwrap().to_owned();

        // Simulate AGENT-REG-002 session.rs:5829-5835 outbound_text scrubbing on ext_args["script"]
        let scrubbed_preview = insar_agent::history::outbound_text(original_script);
        assert!(
            scrubbed_preview.contains("[local-reference]"),
            "outbound_text should scrub slash-containing comment/formula: {scrubbed_preview}"
        );

        // Local desktop IPC `gee_script_preview` returns the exact uncorrupted script for human review
        let preview_ipc = crate::geodata_tools::execute_open_geodata_query(
            &root,
            &crate::geodata_tools::OpenGeodataQueryRequest {
                dataset: "gee_script_preview".to_owned(),
                gee_params: Some(json!({ "approval_id": approval_id })),
                ..Default::default()
            },
        )
        .expect("gee_script_preview ipc ok");
        assert_eq!(preview_ipc["ok"], true);
        assert_eq!(
            preview_ipc["script"].as_str().unwrap(),
            original_script.trim()
        );

        // Even if UI fallback sends scrubbed_preview, verify_and_consume_approval executes canonical script cleanly
        let exec_ipc = crate::geodata_tools::execute_open_geodata_query(
            &root,
            &crate::geodata_tools::OpenGeodataQueryRequest {
                dataset: "gee_script_execute".to_owned(),
                bbox: Some([117.2, 31.8, 117.3, 31.9]),
                gee_params: Some(json!({
                    "approval_id": approval_id,
                    "script": scrubbed_preview,
                })),
                ..Default::default()
            },
        )
        .expect("gee_script_execute must succeed without GEE_SCRIPT_HASH_MISMATCH");
        assert_eq!(exec_ipc["ok"], true);
        let ndvi = exec_ipc["result"]["ndvi"].as_f64().unwrap();
        assert!((ndvi - 0.6316).abs() < 1e-3, "expected ~0.6316, got {ndvi}");

        let _ = svc.inner.0.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn extension_prompt_contains_22_skills_playbooks_and_nisar_range_v2_and_skill_read_works() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-skills-prompt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let gee_plugin_dir = root.join("plugins").join("eostage-gee-plugin");
        let _ = std::fs::create_dir_all(&gee_plugin_dir);
        let _ = std::fs::write(
            gee_plugin_dir.join("plugin.json"),
            r#"{"id":"eostage-gee-plugin","name":"GEE","version":"1.0.0","enabled":true}"#,
        );
        let svc_inner = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::business_store(
                root.join("fixture.sqlite"),
            ),
        );
        svc_inner.startup().expect("business_store startup");
        let svc = DesktopGeeAgentService {
            inner: AgentApplication(Arc::new(svc_inner)),
        };

        let prompt = svc
            .application_extension_system_prompt()
            .expect("extension system prompt present");
        assert!(prompt.contains("<available_skills>"));
        assert!(prompt.contains("<available_toolsets>"));
        assert!(prompt.contains("remote_range"));
        assert!(prompt.contains("nisar_subset_preflight"));
        assert!(prompt.contains("nisar_create_subset_task"));
        assert!(prompt.contains("Disaster Response Playbooks"));

        let manifest_skills = builtin_skill_entries();
        assert_eq!(manifest_skills.len(), 29);
        for skill in &manifest_skills {
            assert!(
                prompt.contains(&format!("`{}`", skill.id)),
                "missing skill `{}` in <available_skills>",
                skill.id
            );
        }

        let read_res = svc
            .application_call_extension_tool(
                "skill_read",
                &json!({ "skill_id": "nisar-rslc-subset" }),
            )
            .expect("skill_read handled")
            .expect("skill_read ok");
        assert_eq!(read_res["ok"], true);
        assert_eq!(read_res["source"], "builtin");
        assert!(
            !read_res["content"].as_str().unwrap_or("").trim().is_empty(),
            "SKILL.md content should be non-empty"
        );

        let traversal_res = svc
            .application_call_extension_tool(
                "skill_read",
                &json!({ "skill_id": "../secrets", "file_path": "../../etc/passwd" }),
            )
            .expect("skill_read traversal handled")
            .expect("skill_read traversal safe");
        assert_eq!(traversal_res["ok"], false);
        assert_eq!(traversal_res["error"], "INVALID_SKILL_ID");

        let _ = svc.inner.0.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn slim_agent_page_value_strips_bulky_asset_manifests_and_preserves_size_bytes() {
        let bulky_assets: Vec<Value> = (0..60)
            .map(|i| {
                json!({
                    "name": format!("NISAR_L1_PR_RSLC_SUBASSET_{i:02}.h5"),
                    "href": format!("https://datapool.asf.alaska.edu/NISAR/RSLC/very/long/signed/path/{i:02}/product_subswath_metadata_manifest_payload.h5?token=placeholder"),
                    "size_bytes": 250_000_000u64,
                })
            })
            .collect();
        let raw_page = json!({
            "batch_id": "batch-unified-38ce5bac-0b71-4391-8033-bbb9fbb7cf7c",
            "search_batch_id": "batch-unified-38ce5bac-0b71-4391-8033-bbb9fbb7cf7c",
            "revision": 0,
            "page_index": 0,
            "page_size": 5,
            "total_count": 5,
            "checked_count": 0,
            "items": (0..5).map(|idx| json!({
                "checked": false,
                "item": {
                    "item_id": format!("G424025711{idx}-ASF"),
                    "display_name": format!("NISAR_L1_PR_RSLC_00{idx}"),
                    "acquisition_datetime": "2026-10-20T12:00:00Z",
                    "provider_id": "asf.nasa.earthdata",
                    "asset_access": { "url": "https://datapool.asf.alaska.edu/heavy" },
                    "execution_policy": { "mode": "stream" },
                    "footprint_geojson": {
                        "type": "Polygon",
                        "coordinates": [[[103.1234567, 31.1234567], [104.9876543, 31.1234567], [104.9876543, 32.6543219], [103.1234567, 32.6543219], [103.1234567, 31.1234567]]]
                    },
                    "metadata": {
                        "dataset": "NISAR",
                        "flight_direction": "ASCENDING",
                        "processing_level": "RSLC",
                        "assets": bulky_assets,
                    },
                    "payload": {}
                }
            })).collect::<Vec<_>>()
        });

        let raw_len = serde_json::to_string(&raw_page).unwrap().len();
        assert!(
            raw_len > 50_000,
            "expected bulky raw page >50KB, got {raw_len}"
        );

        let slimmed = slim_agent_page_value(raw_page);
        let slim_len = serde_json::to_string(&slimmed).unwrap().len();
        assert!(
            slim_len < 3_000,
            "expected slimmed page <3KB, got {slim_len} (raw was {raw_len})"
        );
        assert_eq!(
            slimmed["search_batch_id"],
            "batch-unified-38ce5bac-0b71-4391-8033-bbb9fbb7cf7c"
        );
        assert_eq!(slimmed["items"].as_array().unwrap().len(), 5);
        let first_item = &slimmed["items"][0]["item"];
        assert!(first_item.get("asset_access").is_none());
        assert!(first_item.get("footprint_geojson").is_none());
        assert!(first_item.get("bbox").is_some());
        assert!(first_item["metadata"].get("assets").is_none());
        assert_eq!(
            first_item["metadata"]["size_bytes"],
            json!(15_000_000_000u64)
        );
        assert_eq!(first_item["metadata"]["flight_direction"], "ASCENDING");
    }

    #[test]
    fn uninstalled_or_disabled_gee_plugin_gates_gee_tools_and_prompt() {
        let uniq = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("sceneport-gee-gate-{uniq}"));
        let _ = std::fs::create_dir_all(&root);
        let svc_inner = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::business_store(
                root.join("fixture.sqlite"),
            ),
        );
        svc_inner.startup().expect("startup");
        let svc = DesktopGeeAgentService {
            inner: AgentApplication(Arc::new(svc_inner)),
        };

        // 1. Fresh dir has no installed eostage-gee-plugin -> 10 non-GEE tools (including disaster + AOI guard + open_geodata_query)
        let tools_without = svc.application_extension_tools();
        assert_eq!(tools_without.len(), 10, "{tools_without:?}");
        assert!(!tools_without.iter().any(|t| t.name.starts_with("gee_")));
        assert!(tools_without.iter().any(|t| t.name == "toolset_activate"));
        assert!(tools_without
            .iter()
            .any(|t| t.name == "global_disaster_search"));
        assert!(tools_without
            .iter()
            .any(|t| t.name == "aoi_validate_boundary"));
        assert!(tools_without.iter().any(|t| t.name == "open_geodata_query"));
        assert!(svc
            .application_extension_system_prompt()
            .unwrap()
            .contains("global_disaster_search"));

        let call_gated = svc
            .application_call_extension_tool("gee_catalog_search", &json!({"query": "NDVI"}))
            .expect("handled")
            .expect("ok envelope");
        assert_eq!(call_gated["ok"], false);
        assert_eq!(call_gated["error"], "AGENT_PLUGIN_DISABLED");

        let call_gated_advisor = svc
            .application_call_extension_tool(
                "gee_dataset_advisor",
                &json!({"mode": "recommend", "task": "flood risk"}),
            )
            .expect("handled")
            .expect("ok envelope");
        assert_eq!(call_gated_advisor["error"], "AGENT_PLUGIN_DISABLED");

        let call_gated_preflight = svc
            .application_call_extension_tool(
                "gee_export_preflight",
                &json!({"asset_id": "COPERNICUS/S2_SR_HARMONIZED"}),
            )
            .expect("handled")
            .expect("ok envelope");
        assert_eq!(call_gated_preflight["error"], "AGENT_PLUGIN_DISABLED");

        // 2. Install then disable plugin -> also gated
        let plugin_dir = root.join("plugins").join("eostage-gee-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"id":"eostage-gee-plugin","name":"GEE","version":"1.0.0","enabled":true}"#,
        )
        .unwrap();
        assert_eq!(svc.application_extension_tools().len(), 18);

        let adv_ok = svc
            .application_call_extension_tool(
                "gee_dataset_advisor",
                &json!({"mode": "recommend", "task": "洪涝风险评估"}),
            )
            .expect("advisor handled")
            .expect("advisor ok");
        assert_eq!(
            insar_agent::run::safe_value(&adv_ok).expect("advisor passes safe_value"),
            adv_ok
        );
        assert_eq!(adv_ok["recipe"], "flood-risk");

        let pre_ok = svc
            .application_call_extension_tool(
                "gee_export_preflight",
                &json!({
                    "asset_id": "COPERNICUS/S2_SR_HARMONIZED",
                    "bbox": [117.24, 31.81, 117.29, 31.86],
                    "start_date": "2025-05-01",
                    "end_date": "2025-09-01",
                    "scale_m": 10.0
                }),
            )
            .expect("preflight handled")
            .expect("preflight ok");
        assert_eq!(
            insar_agent::run::safe_value(&pre_ok).expect("preflight passes safe_value"),
            pre_ok
        );
        assert_eq!(pre_ok["ok"], true);
        assert_eq!(pre_ok["ready"], true);

        std::fs::write(plugin_dir.join(".disabled"), b"disabled").unwrap();
        assert_eq!(svc.application_extension_tools().len(), 10);
        assert!(!svc
            .application_extension_system_prompt()
            .unwrap()
            .contains("gee_cloud_analyze"));

        let _ = svc.inner.0.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_geodata_query_tool_advertised_rejects_unknown_and_confines_output_path() {
        let uniq = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("sceneport-open-geodata-agent-{uniq}"));
        let _ = std::fs::create_dir_all(&root);
        let svc_inner = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::business_store(
                root.join("fixture.sqlite"),
            ),
        );
        svc_inner.startup().expect("startup");
        let svc = DesktopGeeAgentService {
            inner: AgentApplication(Arc::new(svc_inner)),
        };

        let tools = svc.application_extension_tools();
        let og = tools
            .iter()
            .find(|t| t.name == "open_geodata_query")
            .expect("open_geodata_query advertised");
        assert!(og.description.contains("aws_sentinel1_grd"));
        assert!(og.description.contains("noaa_aws_weather"));
        assert!(og.description.contains("overture_maps"));

        let confined = confine_open_geodata_agent_output_path(
            &root,
            "aws_sentinel1_grd",
            Some("/tmp/evil/../escape.tif"),
        )
        .expect("confined path");
        assert!(confined.starts_with(root.join("geodata_downloads")));
        assert!(confined.ends_with("escape.tif"));

        for denied_ds in [
            "unknown_aws_dummy",
            "gee_account_save",
            "whitebox",
            "duckdb",
            "xyz_satellite_tiles",
            "nisar_hdf5_subset",
        ] {
            let res = svc
                .application_call_extension_tool(
                    "open_geodata_query",
                    &json!({ "dataset": denied_ds }),
                )
                .expect("handled")
                .expect("ok envelope");
            assert_eq!(res["ok"], false, "{denied_ds}");
            assert!(
                res["error"]
                    .as_str()
                    .is_some_and(|s| s.contains("UNSUPPORTED_AGENT_OPEN_GEODATA_DATASET")),
                "expected allowlist error for {denied_ds}: {res}"
            );
            assert_eq!(insar_agent::run::safe_value(&res).expect("safe_value"), res);
        }

        let _ = svc.inner.0.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_provider_proxy_policy_matches_network_proxy_mode() {
        use insar_application_service::ApplicationNetworkSettings;
        use insar_core::ProxyPolicy;

        let direct = ApplicationNetworkSettings {
            proxy_enabled: false,
            proxy_url: "http://127.0.0.1:7890".into(),
            ..Default::default()
        };
        assert_eq!(
            proxy_policy_from_network_settings(&direct),
            ProxyPolicy::Direct
        );

        let auto = ApplicationNetworkSettings {
            proxy_enabled: true,
            proxy_url: String::new(),
            ..Default::default()
        };
        assert_eq!(
            proxy_policy_from_network_settings(&auto),
            ProxyPolicy::System
        );

        let manual = ApplicationNetworkSettings {
            proxy_enabled: true,
            proxy_url: "http://127.0.0.1:7890".into(),
            ..Default::default()
        };
        assert!(matches!(
            proxy_policy_from_network_settings(&manual),
            ProxyPolicy::Manual(_)
        ));
    }

    #[test]
    fn skill_read_supports_query_search_and_batch_paths() {
        let res = execute_skill_read(
            None,
            &json!({
                "skill_id": "remote-sensing-analysis",
                "query": "BOA_ADD_OFFSET",
            }),
        );
        assert_eq!(res["ok"], true, "{res}");
        assert!(!res["matches"].as_array().unwrap().is_empty(), "{res}");

        let batch = execute_skill_read(
            None,
            &json!({
                "skill_id": "remote-sensing-analysis",
                "paths": ["SKILL.md", "references/authoritative-sources.md"],
            }),
        );
        assert_eq!(batch["ok"], true, "{batch}");
        assert_eq!(batch["files"].as_array().unwrap().len(), 2, "{batch}");
    }

    #[test]
    fn disabled_plugin_suppresses_skill_read_and_extension_mcp_tools() {
        let uniq = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("eostage-disabled-plugin-{uniq}"));
        let rt_plugins = root.join("empty-rt");
        std::fs::create_dir_all(&rt_plugins).unwrap();

        let payload = r#"{
            "id": "eostage-test-owned-plugin",
            "name": "Owned Test Plugin",
            "version": "1.0.0",
            "runtime_compatibility": "eostage_native",
            "skill": {
                "id": "owned-test-skill",
                "name": "Owned Test Skill",
                "content": "---\nname: owned-test-skill\ndescription: test\n---\n# Owned Skill\nRun step 1."
            },
            "mcp_servers": [
                {
                    "id": "owned-srv",
                    "command": "",
                    "args": [],
                    "env": {},
                    "url": "https://owned.example.invalid/mcp",
                    "transport": "http"
                }
            ]
        }"#;
        crate::application::install_plugin_package_at(
            &root,
            &rt_plugins,
            &crate::application::PluginInstallRequest {
                plugin_id: None,
                source_path: None,
                raw_content: Some(payload.to_string()),
                fallback_name: None,
            },
        )
        .expect("install owned plugin");

        // Enabled: prompt lists skill, skill_read succeeds, effective connectors include owned-srv.
        let enabled_prompt = build_desktop_extension_system_prompt(Some(&root));
        assert!(enabled_prompt.contains("owned-test-skill"));
        let read_ok = execute_skill_read(Some(&root), &json!({ "skill_id": "owned-test-skill" }));
        assert_eq!(read_ok["ok"], true, "{read_ok}");
        assert_eq!(read_ok["source"], "plugin:eostage-test-owned-plugin");
        assert!(
            crate::application::load_effective_mcp_connectors_at(&root, &rt_plugins)
                .iter()
                .any(|c| c.id == "plugin-mcp-owned-srv")
        );

        // Disabled: skill suppressed in prompt, skill_read returns PLUGIN_DISABLED, connector suppressed.
        crate::application::set_plugin_enabled_at(
            &root,
            &rt_plugins,
            "eostage-test-owned-plugin",
            false,
        )
        .expect("disable plugin");
        let disabled_prompt = build_desktop_extension_system_prompt(Some(&root));
        assert!(!disabled_prompt.contains("owned-test-skill"));
        let read_disabled =
            execute_skill_read(Some(&root), &json!({ "skill_id": "owned-test-skill" }));
        assert_eq!(read_disabled["ok"], false, "{read_disabled}");
        assert_eq!(read_disabled["error"], "PLUGIN_DISABLED", "{read_disabled}");
        assert!(
            !crate::application::load_effective_mcp_connectors_at(&root, &rt_plugins)
                .iter()
                .any(|c| c.id == "plugin-mcp-owned-srv")
        );

        // Re-enabled: both skill and connector are restored.
        crate::application::set_plugin_enabled_at(
            &root,
            &rt_plugins,
            "eostage-test-owned-plugin",
            true,
        )
        .expect("re-enable plugin");
        let restored_read =
            execute_skill_read(Some(&root), &json!({ "skill_id": "owned-test-skill" }));
        assert_eq!(restored_read["ok"], true, "{restored_read}");
        assert!(
            crate::application::load_effective_mcp_connectors_at(&root, &rt_plugins)
                .iter()
                .any(|c| c.id == "plugin-mcp-owned-srv")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn desktop_extension_prompt_and_tools_pass_outbound_safe_value_across_all_protocols() {
        use insar_agent_model::{transport::ModelRoute, Message, Protocol};

        let uniq = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("sceneport-outbound-prompt-{uniq}"));
        let custom_dir = root.join("skills_custom").join("path-heavy-skill");
        std::fs::create_dir_all(&custom_dir).unwrap();
        std::fs::write(
            custom_dir.join("skill.json"),
            json!({
                "id": "path-heavy-skill",
                "name": "COG Streamer",
                "slash_command": "/path-heavy-skill",
                "description": "使用 `/vsicurl/` 从 `/tmp/demo.tif` 读取示例并提取特征",
                "enabled": true,
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            custom_dir.join("SKILL.md"),
            "# Example\nRun `gdal_translate /vsicurl/https://example.invalid/a.tif /tmp/out.tif`.\n",
        )
        .unwrap();

        let prompt = build_desktop_extension_system_prompt(Some(&root));
        assert!(!prompt.contains("/tmp/demo.tif"));

        let read_res = execute_skill_read(Some(&root), &json!({ "skill_id": "path-heavy-skill" }));
        assert_eq!(read_res["ok"], true, "{read_res}");
        let read_content = read_res["content"].as_str().unwrap();
        assert_ne!(
            insar_agent::outbound::nested_value(&json!(read_content)),
            json!("[local]")
        );

        let svc_inner = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::business_store(
                root.join("fixture.sqlite"),
            ),
        );
        svc_inner.startup().expect("startup");
        let svc = DesktopGeeAgentService {
            inner: AgentApplication(Arc::new(svc_inner)),
        };
        let tools = svc.application_extension_tools();

        let msgs = vec![
            Message {
                role: "system".into(),
                content: prompt,
            },
            Message {
                role: "user".into(),
                content: "请直接输出公式 $a/b$ 与结果说明。".into(),
            },
        ];

        for (protocol, url) in [
            (Protocol::ChatCompletions, "https://example.invalid/v1"),
            (Protocol::Responses, "https://example.invalid/v1"),
            (Protocol::AnthropicMessages, "https://example.invalid"),
            (
                Protocol::GoogleGemini,
                "https://generativelanguage.googleapis.com/v1beta",
            ),
        ] {
            let route = ModelRoute::new(protocol, url, "test-model")
                .expect("route")
                .with_max_output_tokens(Some(16384));
            let (_, body) = route.request(&msgs, &tools).expect("request body");
            assert_eq!(
                insar_agent::run::safe_value(&body)
                    .unwrap_or_else(|e| panic!("outbound blocked on {:?}: {e}", protocol)),
                body
            );
        }

        // Also prove updated GEE prompt + all 18 tools (including advisor, preflight, disaster, AOI guard & open_geodata_query) pass all 4 protocols
        let gee_plugin_dir = root.join("plugins").join("eostage-gee-plugin");
        std::fs::create_dir_all(&gee_plugin_dir).unwrap();
        std::fs::write(
            gee_plugin_dir.join("plugin.json"),
            r#"{"id":"eostage-gee-plugin","name":"GEE","version":"1.0.0","enabled":true}"#,
        )
        .unwrap();
        let gee_prompt = build_desktop_extension_system_prompt(Some(&root));
        let gee_tools = svc.application_extension_tools();
        assert_eq!(gee_tools.len(), 18);
        let gee_msgs = vec![
            Message {
                role: "system".into(),
                content: gee_prompt,
            },
            Message {
                role: "user".into(),
                content: "请评估洪涝风险并预演导出规模。".into(),
            },
        ];
        for (protocol, url) in [
            (Protocol::ChatCompletions, "https://example.invalid/v1"),
            (Protocol::Responses, "https://example.invalid/v1"),
            (Protocol::AnthropicMessages, "https://example.invalid"),
            (
                Protocol::GoogleGemini,
                "https://generativelanguage.googleapis.com/v1beta",
            ),
        ] {
            let route = ModelRoute::new(protocol, url, "test-model")
                .expect("route")
                .with_max_output_tokens(Some(16384));
            let (_, body) = route
                .request(&gee_msgs, &gee_tools)
                .expect("gee request body");
            assert_eq!(
                insar_agent::run::safe_value(&body)
                    .unwrap_or_else(|e| panic!("gee outbound blocked on {:?}: {e}", protocol)),
                body
            );
        }

        let _ = svc.inner.0.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn six_textbook_skills_pass_outbound_nested_value_without_collapsing() {
        let entries = builtin_skill_entries();
        assert_eq!(entries.len(), 29, "expected all 29 public builtin skills");
        for entry in &entries {
            let skill_id = entry.id.as_str();
            let res = execute_skill_read(None, &json!({ "skill_id": skill_id }));
            assert_eq!(res["ok"], true, "{skill_id}: {res}");
            let files = res["available_files"].as_array().unwrap();
            assert!(!files.is_empty(), "{skill_id} missing files");
            for f in files {
                let rel = f.as_str().unwrap();
                if !(rel.ends_with(".md") || rel.ends_with(".txt")) {
                    continue;
                }
                let file_res =
                    execute_skill_read(None, &json!({ "skill_id": skill_id, "file_path": rel }));
                assert_eq!(file_res["ok"], true, "{skill_id}/{rel}: {file_res}");
                let content = file_res["content"].as_str().unwrap();
                let nested = insar_agent::outbound::nested_value(&json!(content));
                assert_ne!(
                    nested,
                    json!("[local]"),
                    "skill {skill_id}/{rel} collapsed to [local] (len={})",
                    content.len()
                );
            }
        }
    }

    #[test]
    fn newly_bundled_recipes_and_methodology_skills_are_embedded_and_searchable() {
        for skill_id in [
            "eo-analysis-recipes",
            "eo-agent-capability-routing",
            "eo-model-explanation-review",
            "eo-model-transfer-evaluation",
            "eo-workbench-concept",
        ] {
            let embedded_files = embedded_builtin_skill_files(skill_id);
            assert!(
                embedded_files.iter().any(|f| f == "SKILL.md"),
                "embedded files missing SKILL.md for {skill_id}: {embedded_files:?}"
            );
            let embedded_skill = read_embedded_builtin_skill_file(skill_id, "SKILL.md")
                .expect("embedded SKILL.md present");
            assert!(!embedded_skill.trim().is_empty());

            let res = execute_skill_read(None, &json!({ "skill_id": skill_id }));
            assert_eq!(res["ok"], true, "{skill_id}: {res}");
            assert_eq!(res["source"], "builtin");
        }

        let recipe_files = embedded_builtin_skill_files("eo-analysis-recipes");
        assert_eq!(
            recipe_files.len(),
            100,
            "expected SKILL.md + 99 recipes in eo-analysis-recipes, got {}",
            recipe_files.len()
        );
        let textbook_files = embedded_builtin_skill_files("gee-textbook-cookbook");
        assert_eq!(
            textbook_files.len(),
            16,
            "expected SKILL.md + 5 handbook chapters + 10 practice cards in gee-textbook-cookbook, got {}",
            textbook_files.len()
        );
        let radar_files = embedded_builtin_skill_files("radar-satellite-analysis");
        assert_eq!(
            radar_files.len(),
            3,
            "expected SKILL.md + 2 pairing branches in radar-satellite-analysis, got {}",
            radar_files.len()
        );
        for (sid, rel_files) in [
            ("eo-analysis-recipes", recipe_files),
            ("gee-textbook-cookbook", textbook_files),
            ("radar-satellite-analysis", radar_files),
        ] {
            for rel in rel_files {
                let raw = read_embedded_builtin_skill_file(sid, &rel)
                    .unwrap_or_else(|| panic!("missing embedded file {sid}/{rel}"));
                assert!(!raw.trim().is_empty(), "empty embedded file {sid}/{rel}");
                let served =
                    execute_skill_read(None, &json!({ "skill_id": sid, "file_path": rel }));
                assert_eq!(served["ok"], true, "{sid}/{rel}: {served}");
                insar_agent::run::safe_value(&served)
                    .unwrap_or_else(|e| panic!("safe_value rejected served {sid}/{rel}: {e}"));
                let served_content = served["content"].as_str().unwrap_or_default();
                assert_ne!(
                    insar_agent::outbound::nested_value(&json!(served_content)),
                    json!("[local]"),
                    "nested_value collapsed served {sid}/{rel} to [local]"
                );
            }
        }

        let core_rule_probe =
            execute_skill_read(None, &json!({ "skill_id": "builtin-insar-copilot-core" }));
        assert_eq!(
            core_rule_probe["ok"], false,
            "System rules must not be exposed via skill_read: {core_rule_probe}"
        );

        for (sid, kw) in [
            ("eo-analysis-recipes", "modis_annual_gpp"),
            ("eo-analysis-recipes", "landsat-rsei"),
            ("eo-analysis-recipes", "hurst-rescaled-range"),
            ("eo-analysis-recipes", "CLOUD_SCORE_PLUS"),
            ("eo-analysis-recipes", "DYNAMICWORLD"),
            ("eo-analysis-recipes", "LandTrendr"),
            ("eo-analysis-recipes", "Ccdc"),
            ("eo-analysis-recipes", "Otsu"),
            ("gee-textbook-cookbook", "tileScale"),
            ("gee-textbook-cookbook", "EECU"),
            ("radar-satellite-analysis", "ALOS-2"),
            ("radar-satellite-analysis", "Bperp"),
        ] {
            let query_res = execute_skill_read(
                None,
                &json!({
                    "skill_id": sid,
                    "query": kw
                }),
            );
            assert_eq!(query_res["ok"], true, "{sid}/{kw}: {query_res}");
            assert!(
                !query_res["matches"].as_array().unwrap().is_empty(),
                "expected query '{kw}' match in {sid}: {query_res}"
            );
        }

        let s2_res = execute_skill_read(
            None,
            &json!({
                "skill_id": "eo-analysis-recipes",
                "file_path": "references/s2-ndvi.md"
            }),
        );
        assert_eq!(s2_res["ok"], true, "{s2_res}");
        assert!(s2_res["content"]
            .as_str()
            .unwrap()
            .contains("COPERNICUS/S2_SR_HARMONIZED"));
    }

    #[test]
    fn skill_read_empty_paths_falls_back_to_file_path_and_missing_paths_error() {
        // 1. `paths: []` with `file_path: "SKILL.md"` MUST fall back to reading SKILL.md
        //    instead of silently returning `{ ok: true, files: [] }`.
        let fallback_explicit = execute_skill_read(
            None,
            &json!({
                "skill_id": "nisar-rslc-subset",
                "file_path": "SKILL.md",
                "paths": []
            }),
        );
        assert_eq!(fallback_explicit["ok"], true, "{fallback_explicit}");
        assert!(
            !fallback_explicit["content"]
                .as_str()
                .unwrap_or("")
                .trim()
                .is_empty(),
            "empty paths with file_path must return actual content: {fallback_explicit}"
        );

        // 2. `paths: []` with omitted `file_path` defaults to reading SKILL.md.
        let fallback_default = execute_skill_read(
            None,
            &json!({
                "skill_id": "nisar-rslc-subset",
                "paths": []
            }),
        );
        assert_eq!(fallback_default["ok"], true, "{fallback_default}");
        assert_eq!(fallback_default["file_path"], "SKILL.md");
        assert!(
            !fallback_default["content"]
                .as_str()
                .unwrap_or("")
                .trim()
                .is_empty(),
            "{fallback_default}"
        );

        // 3. Non-empty `paths` whose files do not exist MUST return `ok: false`
        //    with `SKILL_FILE_NOT_FOUND`, `missing_files`, and `available_files`.
        let missing = execute_skill_read(
            None,
            &json!({
                "skill_id": "nisar-rslc-subset",
                "paths": ["does-not-exist.md"]
            }),
        );
        assert_eq!(missing["ok"], false, "{missing}");
        assert_eq!(missing["error"], "SKILL_FILE_NOT_FOUND");
        assert_eq!(missing["missing_files"], json!(["does-not-exist.md"]));
        assert!(
            missing["available_files"]
                .as_array()
                .is_some_and(|a| !a.is_empty()),
            "missing-file error must include available_files: {missing}"
        );
    }

    #[test]
    fn skill_catalog_deduplicates_with_custom_over_plugin_over_builtin_precedence() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-skill-dedup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);

        // Install two plugins bundling the same builtin skill_id (`sar-insar-textbook`)
        // plus a shared plugin-only skill (`overlap-plugin-skill`).
        for (plugin_id, desc_tag) in [
            ("z-plugin-second", "FROM_Z_PLUGIN"),
            ("a-plugin-first", "FROM_A_PLUGIN"),
        ] {
            let pdir = root.join("plugins").join(plugin_id);
            let _ = std::fs::create_dir_all(&pdir);
            let manifest = json!({
                "id": plugin_id,
                "name": plugin_id,
                "version": "1.0.0",
                "enabled": true,
                "bundledSkills": [
                    {
                        "id": "sar-insar-textbook",
                        "name": "Plugin SAR",
                        "description": format!("Plugin override {desc_tag}"),
                        "prompt": format!("# {desc_tag}")
                    },
                    {
                        "id": "overlap-plugin-skill",
                        "name": "Overlap Skill",
                        "description": format!("Overlap {desc_tag}"),
                        "prompt": format!("# {desc_tag}")
                    }
                ]
            });
            std::fs::write(
                pdir.join("plugin.json"),
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();
        }

        // Also create a custom skill overriding `sar-insar-textbook`
        let custom_dir = root.join("skills_custom").join("sar-insar-textbook");
        std::fs::create_dir_all(&custom_dir).unwrap();
        let custom_meta = json!({
            "id": "sar-insar-textbook",
            "name": "Custom SAR Override",
            "description": "CUSTOM_WINS_DESC",
            "prompt": "# Custom SAR prompt",
            "slash_command": "/sar-insar-textbook",
            "enabled": true
        });
        std::fs::write(
            custom_dir.join("skill.json"),
            serde_json::to_vec_pretty(&custom_meta).unwrap(),
        )
        .unwrap();

        let prompt_1 = build_desktop_extension_system_prompt(Some(&root));
        let prompt_2 = build_desktop_extension_system_prompt(Some(&root));
        assert_eq!(
            prompt_1, prompt_2,
            "extension system prompt must be 100% byte-identical across calls"
        );

        let catalog_slice = prompt_1
            .split("<available_skills>")
            .nth(1)
            .and_then(|s| s.split("</available_skills>").next())
            .expect("available_skills block");

        // Each skill_id appears EXACTLY ONCE in <available_skills>
        let sar_matches: Vec<&str> = catalog_slice
            .lines()
            .filter(|l| l.starts_with("- `sar-insar-textbook` "))
            .collect();
        assert_eq!(
            sar_matches.len(),
            1,
            "sar-insar-textbook must appear once: {sar_matches:?}"
        );
        assert!(
            sar_matches[0].contains("resolved_source=custom")
                && sar_matches[0].contains("CUSTOM_WINS_DESC"),
            "custom must beat plugin & builtin: {}",
            sar_matches[0]
        );

        let overlap_matches: Vec<&str> = catalog_slice
            .lines()
            .filter(|l| l.starts_with("- `overlap-plugin-skill` "))
            .collect();
        assert_eq!(
            overlap_matches.len(),
            1,
            "overlap-plugin-skill must appear once: {overlap_matches:?}"
        );
        assert!(
            overlap_matches[0].contains("resolved_source=plugin:a-plugin-first")
                && overlap_matches[0].contains("FROM_A_PLUGIN"),
            "alphabetically first plugin must win deterministically: {}",
            overlap_matches[0]
        );

        // Runtime resolve_skill_dir_and_source must agree with catalog precedence!
        let (_, sar_src, _, _) =
            resolve_skill_dir_and_source(Some(&root), "sar-insar-textbook").unwrap();
        assert_eq!(sar_src, "custom");
        let (_, overlap_src, _, _) =
            resolve_skill_dir_and_source(Some(&root), "overlap-plugin-skill").unwrap();
        assert_eq!(overlap_src, "plugin:a-plugin-first");

        // Removing custom skill falls back cleanly to plugin:a-plugin-first (still deduplicated)
        let _ = std::fs::remove_dir_all(&custom_dir);
        let prompt_no_custom = build_desktop_extension_system_prompt(Some(&root));
        let catalog_no_custom = prompt_no_custom
            .split("<available_skills>")
            .nth(1)
            .and_then(|s| s.split("</available_skills>").next())
            .expect("available_skills block");
        let sar_after: Vec<&str> = catalog_no_custom
            .lines()
            .filter(|l| l.starts_with("- `sar-insar-textbook` "))
            .collect();
        assert_eq!(sar_after.len(), 1);
        assert!(sar_after[0].contains("resolved_source=plugin:a-plugin-first"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nisar_subset_preflight_emits_honesty_fields_for_bbox_and_polygon() {
        let root = std::env::temp_dir().join(format!(
            "sceneport-nisar-honesty-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let db_path = root.join("fixture.sqlite");

        let svc_inner = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::business_store(db_path),
        );
        svc_inner.startup().expect("startup");
        let svc = DesktopGeeAgentService {
            inner: AgentApplication(Arc::new(svc_inner)),
        };

        // Seed a real NISAR L-band RSLC search batch directly via StoreAccess
        let batch_id = "nisar-honesty-batch-1";
        let item_id =
            "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000011-ASF";
        {
            let footprint = json!({
                "type": "Polygon",
                "coordinates": [[[105.0, 38.0], [107.0, 38.0], [107.0, 39.5], [105.0, 39.5], [105.0, 38.0]]]
            })
            .as_object()
            .unwrap()
            .clone();
            let item = insar_core::SnapshotItem {
                item_id: item_id.to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                display_name: "NISAR_L1_PR_RSLC_G9900000011_P05023_N_F_J_001".to_owned(),
                acquisition_datetime: Some("2026-07-26T15:58:03Z".to_owned()),
                footprint_geojson: Some(footprint.clone()),
                metadata: serde_json::Map::from_iter([
                    (
                        "logical_dataset".to_owned(),
                        json!("sar.nisar.l.rslc.provisional.v1"),
                    ),
                    ("band".to_owned(), json!("L")),
                    ("processing_level".to_owned(), json!("RSLC")),
                    ("science_level".to_owned(), json!("L1")),
                    ("production_configuration".to_owned(), json!("PR")),
                    ("crid".to_owned(), json!("P05023")),
                    ("product_specification_version".to_owned(), json!("1.5.0")),
                    (
                        "collection_profile_id".to_owned(),
                        json!("asf.nisar.l.rslc.provisional.v1"),
                    ),
                    ("quality_status".to_owned(), json!("Provisional")),
                    ("frequency_a_polarization".to_owned(), json!("HH+HV")),
                    ("frequency_b_polarization".to_owned(), json!("HH")),
                ]),
                payload: serde_json::Map::new(),
                asset_access: Some(insar_core::FrozenAssetAccess {
                    reference: insar_core::StableAssetReference {
                        provider_id: "asf.nasa.earthdata".to_owned(),
                        collection_id: "C2853145197-ASF".to_owned(),
                        provider_product_id: "G9900000011-ASF".to_owned(),
                        provider_product_key: item_id.to_owned(),
                        canonical_product_key: None,
                        asset_key: "science-rslc-hdf5".to_owned(),
                        stable_asset_key: format!("{item_id}::ASSET::science-rslc-hdf5"),
                    },
                    descriptor: insar_core::AssetAccessDescriptor {
                        resolver_id: "earthdata_https".to_owned(),
                        scheme: insar_core::AssetAccessScheme::EarthdataHttps,
                        stable_href: None,
                        requires_credentials: false,
                        credential_profile: None,
                        resolver_parameters: insar_core::AssetResolverParameters {},
                    },
                    role: insar_core::AssetRole::Primary,
                    file_name: Some("G9900000011-ASF.h5".to_owned()),
                    expected_size_bytes: Some(12_000_000_000),
                    checksum_algorithm: Some("sha256".to_owned()),
                    checksum_value: Some("e".repeat(64)),
                }),
                execution_policy: Some(insar_core::AssetExecutionPolicy {
                    authority: insar_core::AssetExecutionAuthority::ResolverAuthoritative,
                    policy_id: "asf.nisar.earthdata-complete-hdf5.v2".to_owned(),
                    collection_profile_id: Some("asf.nisar.l.rslc.provisional.v1".to_owned()),
                }),
            };
            svc.inner
                .with_store(|store| {
                    store
                        .save_search_batch(&insar_core::StoreSearchBatchRequest {
                            batch: insar_core::FrozenSearchBatch {
                                schema_version: 1,
                                batch_id: batch_id.to_owned(),
                                provider_id: "asf.nasa.earthdata".to_owned(),
                                created_at: "2026-11-19T00:00:00Z".to_owned(),
                                aoi_name: "银川市".to_owned(),
                                aoi_geojson: Some(footprint),
                                query: serde_json::Map::new(),
                                search_stats: None,
                                candidates: vec![item],
                            },
                            workbench: insar_core::SearchWorkbenchState {
                                batch_id: batch_id.to_owned(),
                                checked_item_ids: vec![item_id.to_owned()],
                                page_index: 0,
                                page_size: 24,
                                revision: 0,
                            },
                            activate_workbench: true,
                        })
                        .expect("save NISAR batch");
                })
                .expect("with_store");
        }

        // 1. BBox input -> geometry_mode = "bbox_rectangle"
        let bbox_res = svc
            .application_call_extension_tool(
                "nisar_subset_preflight",
                &json!({
                    "source_batch_id": batch_id,
                    "bbox": [105.8, 38.2, 106.5, 38.8]
                }),
            )
            .expect("handled")
            .expect("ok");
        assert_eq!(bbox_res["ok"], true, "{bbox_res}");
        assert_eq!(bbox_res["spatial_intersection_verified"], true);
        assert_eq!(bbox_res["geometry_mode"], "bbox_rectangle");
        assert_eq!(bbox_res["byte_estimate_mode"], "pre_hdf5_metadata_estimate");
        assert_eq!(bbox_res["hdf5_chunk_extraction_verified"], false);

        // 2. Explicit GeoJSON polygon input -> geometry_mode = "polygon"
        let poly_res = svc
            .application_call_extension_tool(
                "nisar_subset_preflight",
                &json!({
                    "source_batch_id": batch_id,
                    "aoi_geojson": {
                        "type": "Polygon",
                        "coordinates": [[[105.9, 38.3], [106.4, 38.3], [106.5, 38.7], [105.8, 38.6], [105.9, 38.3]]]
                    }
                }),
            )
            .expect("handled")
            .expect("ok");
        assert_eq!(poly_res["ok"], true, "{poly_res}");
        assert_eq!(poly_res["spatial_intersection_verified"], true);
        assert_eq!(poly_res["geometry_mode"], "polygon");
        assert_eq!(poly_res["byte_estimate_mode"], "pre_hdf5_metadata_estimate");
        assert_eq!(poly_res["hdf5_chunk_extraction_verified"], false);

        let _ = svc.inner.0.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn session_prompt_cache_key_is_deterministic_and_secret_free() {
        let k1 = session_prompt_cache_key("sess-yinchuan-1");
        let k2 = session_prompt_cache_key("sess-yinchuan-1");
        let k3 = session_prompt_cache_key("sess-yinchuan-2");
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
        assert!(k1.starts_with("eostage-"));
        assert_eq!(k1.len(), "eostage-".len() + 24);
        assert!(insar_agent::run::safe_value(&json!(k1)).is_ok());
    }
}
