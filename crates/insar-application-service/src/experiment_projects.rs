use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::Utc;
use insar_core::{RegionAoiState, TaskLifecycleStatus};
use insar_store::{
    ExperimentProjectItemRecord, ExperimentProjectRecord, ExperimentProjectStore,
    ExperimentProjectUpsertItem, StoreError, StoreTaskHistoryPageRequest, StoreTaskSummary,
    DEFAULT_EXPERIMENT_PROJECT_ID, MAX_EXPERIMENT_PROJECT_ITEMS, MAX_TASK_HISTORY_PAGE_SIZE,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{ApplicationService, ApplicationServiceError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExperimentProjectItemView {
    pub item_id: String,
    pub task_id: String,
    pub provider_id: String,
    pub dataset_id: String,
    pub task_type: Option<String>,
    pub output_root: String,
    pub primary_artifact_path: String,
    pub provenance_path: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub integrity_status: String,
    pub license_id: String,
    pub is_noncommercial: bool,
    pub bbox: Option<[f64; 4]>,
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExperimentProjectSnapshot {
    pub project_id: String,
    pub title: String,
    pub max_items: usize,
    pub item_count: usize,
    pub has_noncommercial_restricted_data: bool,
    pub aoi_display_name: Option<String>,
    pub aoi_bbox: Option<[f64; 4]>,
    pub items: Vec<ExperimentProjectItemView>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExperimentProjectExportResult {
    pub project_id: String,
    pub export_dir: String,
    pub manifest_path: String,
    pub aoi_geojson_path: String,
    pub footprints_geojson_path: String,
    pub readme_path: String,
    pub verified_count: usize,
    pub missing_or_modified_count: usize,
    pub has_noncommercial_restricted_data: bool,
}

#[derive(Debug, Clone)]
struct ActiveAoiContext {
    display_name: Option<String>,
    bbox: Option<[f64; 4]>,
    geometry_json: Option<String>,
}

fn map_experiment_store_error(err: StoreError) -> ApplicationServiceError {
    match err {
        StoreError::InvalidSnapshot { reason } => {
            ApplicationServiceError::InvalidRequest { reason }
        }
        other => ApplicationServiceError::Store(other),
    }
}

fn now_utc_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn parse_bbox4(slice: &[f64]) -> Option<[f64; 4]> {
    if slice.len() == 4 && slice.iter().all(|v| v.is_finite()) {
        Some([slice[0], slice[1], slice[2], slice[3]])
    } else {
        None
    }
}

fn parse_bbox_json(raw: Option<&str>) -> Option<[f64; 4]> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    serde_json::from_str::<[f64; 4]>(raw).ok()
}

fn is_noncommercial_license(provider_id: &str, dataset_id: &str, license_id: &str) -> bool {
    let p = provider_id.to_ascii_lowercase();
    let d = dataset_id.to_ascii_lowercase();
    let l = license_id.to_ascii_lowercase();
    p.contains("fabdem") || d.contains("fabdem") || l.contains("by-nc")
}

fn infer_license_id(provider_id: &str, dataset_id: &str) -> String {
    let p = provider_id.to_ascii_lowercase();
    let d = dataset_id.to_ascii_lowercase();
    if p.contains("fabdem") || d.contains("fabdem") {
        "CC BY-NC-SA 4.0".to_owned()
    } else {
        "CC-BY-4.0 / Open".to_owned()
    }
}

fn compute_file_sha256(path: &Path) -> Result<(u64, String), ApplicationServiceError> {
    let mut file = fs::File::open(path).map_err(|err| ApplicationServiceError::InvalidRequest {
        reason: format!("无法读取数据产物文件 {}: {err}", path.display()),
    })?;
    let meta = file
        .metadata()
        .map_err(|err| ApplicationServiceError::InvalidRequest {
            reason: format!("无法获取数据产物元信息 {}: {err}", path.display()),
        })?;
    let size_bytes = meta.len();
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|err| ApplicationServiceError::InvalidRequest {
                reason: format!("读取文件校验摘要失败 {}: {err}", path.display()),
            })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        let _ = write!(&mut hex, "{:02x}", b);
    }
    Ok((size_bytes, hex))
}

fn verify_item_on_disk(record: &ExperimentProjectItemRecord) -> ExperimentProjectItemView {
    let artifact_path = Path::new(&record.primary_artifact_path);
    let integrity_status = if artifact_path.is_file() {
        match fs::metadata(artifact_path) {
            Ok(meta) if meta.len() == record.size_bytes && record.size_bytes > 0 => {
                match compute_file_sha256(artifact_path) {
                    Ok((disk_size, disk_sha))
                        if disk_size == record.size_bytes && disk_sha == record.sha256 =>
                    {
                        "verified".to_owned()
                    }
                    _ => "missing_or_modified".to_owned(),
                }
            }
            _ => "missing_or_modified".to_owned(),
        }
    } else {
        "missing_or_modified".to_owned()
    };

    ExperimentProjectItemView {
        item_id: record.item_id.clone(),
        task_id: record.task_id.clone(),
        provider_id: record.provider_id.clone(),
        dataset_id: record.dataset_id.clone(),
        task_type: record.task_type.clone(),
        output_root: record.output_root.clone(),
        primary_artifact_path: record.primary_artifact_path.clone(),
        provenance_path: record.provenance_path.clone(),
        size_bytes: record.size_bytes,
        sha256: record.sha256.clone(),
        integrity_status,
        license_id: record.license_id.clone(),
        is_noncommercial: is_noncommercial_license(
            &record.provider_id,
            &record.dataset_id,
            &record.license_id,
        ),
        bbox: parse_bbox_json(record.bbox_json.as_deref()),
        added_at: record.added_at.clone(),
    }
}

fn is_ignored_auxiliary_file(name_lower: &str) -> bool {
    name_lower.starts_with('.')
        || name_lower.ends_with(".part")
        || name_lower.ends_with(".tmp")
        || name_lower.ends_with(".lineage_source_index.tif")
        || name_lower.ends_with(".provenance.json")
        || name_lower == "catalog.geojson"
        || name_lower == "readme_数据说明.md"
        || name_lower == "readme_experiment.md"
        || name_lower == "experiment-project.v1.json"
        || name_lower == "study_area.aoi.geojson"
        || name_lower == "scene_footprints.geojson"
}

fn deliverable_extension_priority(name_lower: &str) -> u8 {
    if name_lower.ends_with(".tif")
        || name_lower.ends_with(".tiff")
        || name_lower.ends_with(".nc")
        || name_lower.ends_with(".h5")
        || name_lower.ends_with(".he5")
        || name_lower.ends_with(".img")
        || name_lower.ends_with(".jp2")
    {
        3
    } else if name_lower.ends_with(".zip")
        || name_lower.ends_with(".tar")
        || name_lower.ends_with(".tgz")
        || name_lower.ends_with(".tar.gz")
    {
        2
    } else {
        1
    }
}

fn discover_primary_artifact(
    summary: &StoreTaskSummary,
) -> Result<(PathBuf, Option<PathBuf>), ApplicationServiceError> {
    let output_root = &summary.output_root;
    let mut best: Option<(u8, u64, PathBuf)> = None;
    let mut first_provenance: Option<PathBuf> = None;

    if output_root.is_file() {
        let len = output_root.metadata().map(|m| m.len()).unwrap_or(0);
        if len > 0 {
            best = Some((3, len, output_root.clone()));
        }
    } else if output_root.is_dir() {
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
        queue.push_back((output_root.clone(), 0));
        let mut visited_entries: usize = 0;

        while let Some((dir, depth)) = queue.pop_front() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if visited_entries >= 256 {
                    break;
                }
                visited_entries += 1;
                let path = entry.path();
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let name_lower = name_str.to_ascii_lowercase();
                if path.is_dir() {
                    if !name_str.starts_with('.')
                        && !name_lower.starts_with("experiment_bundle_")
                        && depth < 3
                    {
                        queue.push_back((path, depth + 1));
                    }
                    continue;
                }
                if !path.is_file() {
                    continue;
                }
                if name_lower.ends_with(".provenance.json") && first_provenance.is_none() {
                    first_provenance = Some(path.clone());
                    continue;
                }
                if is_ignored_auxiliary_file(&name_lower) {
                    continue;
                }
                let Ok(meta) = fs::metadata(&path) else {
                    continue;
                };
                if meta.len() == 0 {
                    continue;
                }
                let candidate = (
                    deliverable_extension_priority(&name_lower),
                    meta.len(),
                    path,
                );
                let replace = match &best {
                    None => true,
                    Some(prev) => {
                        candidate.0 > prev.0
                            || (candidate.0 == prev.0
                                && (candidate.1 > prev.1
                                    || (candidate.1 == prev.1 && candidate.2 < prev.2)))
                    }
                };
                if replace {
                    best = Some(candidate);
                }
            }
            if visited_entries >= 256 {
                break;
            }
        }
    }

    if let Some((_, _, primary_path)) = best {
        let adj_prov = {
            let stem_prov = primary_path.with_extension("provenance.json");
            if stem_prov.is_file() {
                Some(stem_prov)
            } else {
                let raw_name = primary_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| primary_path.with_file_name(format!("{n}.provenance.json")));
                raw_name.filter(|p| p.is_file()).or(first_provenance)
            }
        };
        return Ok((primary_path, adj_prov));
    }

    if let Some(input_path) = summary
        .operation
        .as_ref()
        .and_then(|op| op.input_path.as_deref())
        .map(PathBuf::from)
    {
        if input_path.is_file() && input_path.metadata().map(|m| m.len()).unwrap_or(0) > 0 {
            return Ok((input_path, first_provenance));
        }
    }

    Err(ApplicationServiceError::InvalidRequest {
        reason: "未在任务输出目录找到有效落盘的数据产物文件，无法纳入实验工程".to_owned(),
    })
}

fn find_task_summary_in_store(
    store: &insar_store::TaskStore,
    task_id: &str,
) -> Result<(StoreTaskSummary, ActiveAoiContext), ApplicationServiceError> {
    let core = store.core_restore(true)?;
    let aoi_ctx = if let Some(region) = core.region_aoi.as_ref() {
        extract_region_aoi_context(region)
    } else if let Some(standalone) = store.load_standalone_aoi()? {
        ActiveAoiContext {
            display_name: Some(standalone.display_name),
            bbox: parse_bbox4(&standalone.bbox),
            geometry_json: serde_json::to_string(&standalone.geometry).ok(),
        }
    } else {
        ActiveAoiContext {
            display_name: None,
            bbox: None,
            geometry_json: None,
        }
    };

    if let Some(found) = core.tasks.into_iter().find(|t| t.task_id == task_id) {
        return Ok((found, aoi_ctx));
    }

    let mut after_epoch: Option<i64> = None;
    let mut after_task_id: Option<String> = None;
    for _ in 0..50 {
        let page = store.load_task_history_page(&StoreTaskHistoryPageRequest {
            page_size: MAX_TASK_HISTORY_PAGE_SIZE,
            after_archived_at_epoch_ms: after_epoch,
            after_task_id: after_task_id.clone(),
        })?;
        if let Some(item) = page.items.into_iter().find(|i| i.task.task_id == task_id) {
            return Ok((item.task, aoi_ctx));
        }
        if !page.has_more {
            break;
        }
        let Some(cursor) = page.next_cursor else {
            break;
        };
        after_epoch = Some(cursor.archived_at_epoch_ms);
        after_task_id = Some(cursor.task_id);
    }

    Err(ApplicationServiceError::InvalidRequest {
        reason: format!("未找到任务 {task_id} 的持久化记录"),
    })
}

fn load_active_aoi_from_store(
    store: &insar_store::TaskStore,
) -> Result<ActiveAoiContext, ApplicationServiceError> {
    let core = store.core_restore(false)?;
    if let Some(region) = core.region_aoi.as_ref() {
        return Ok(extract_region_aoi_context(region));
    }
    if let Some(standalone) = store.load_standalone_aoi()? {
        return Ok(ActiveAoiContext {
            display_name: Some(standalone.display_name),
            bbox: parse_bbox4(&standalone.bbox),
            geometry_json: serde_json::to_string(&standalone.geometry).ok(),
        });
    }
    Ok(ActiveAoiContext {
        display_name: None,
        bbox: None,
        geometry_json: None,
    })
}

fn extract_region_aoi_context(region: &RegionAoiState) -> ActiveAoiContext {
    ActiveAoiContext {
        display_name: Some(region.display_name.clone()),
        bbox: parse_bbox4(&region.bbox),
        geometry_json: serde_json::to_string(&region.geometry).ok(),
    }
}

fn bbox_polygon_geometry(bbox: [f64; 4]) -> Value {
    let [w, s, e, n] = bbox;
    json!({
        "type": "Polygon",
        "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]
    })
}

fn write_file_atomic(dest: &Path, content: &[u8]) -> Result<(), ApplicationServiceError> {
    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact");
    let tmp = dest.with_file_name(format!(".{file_name}.tmp.{}", Uuid::new_v4()));
    fs::write(&tmp, content).map_err(|err| ApplicationServiceError::InvalidRequest {
        reason: format!("写入临时导出文件失败 {}: {err}", tmp.display()),
    })?;
    if let Err(err) = fs::rename(&tmp, dest) {
        let _ = fs::remove_file(&tmp);
        return Err(ApplicationServiceError::InvalidRequest {
            reason: format!("原子落盘导出文件失败 {}: {err}", dest.display()),
        });
    }
    Ok(())
}

fn build_snapshot(
    project: ExperimentProjectRecord,
    records: Vec<ExperimentProjectItemRecord>,
) -> ExperimentProjectSnapshot {
    let items: Vec<ExperimentProjectItemView> = records.iter().map(verify_item_on_disk).collect();
    let has_noncommercial_restricted_data = items.iter().any(|item| item.is_noncommercial);
    let item_count = items.len();
    ExperimentProjectSnapshot {
        project_id: project.project_id,
        title: project.title,
        max_items: MAX_EXPERIMENT_PROJECT_ITEMS,
        item_count,
        has_noncommercial_restricted_data,
        aoi_display_name: project.aoi_display_name,
        aoi_bbox: parse_bbox_json(project.aoi_bbox_json.as_deref()),
        items,
        updated_at: project.updated_at,
    }
}

fn resolve_default_export_dir(
    store_path: &Path,
    project: &ExperimentProjectRecord,
    items: &[ExperimentProjectItemRecord],
) -> PathBuf {
    let base_parent = items
        .first()
        .and_then(|item| Path::new(&item.output_root).parent().map(Path::to_path_buf))
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| store_path.parent().map(Path::to_path_buf))
        .unwrap_or_else(std::env::temp_dir);
    base_parent.join(format!("experiment_bundle_{}", project.project_id))
}

impl ApplicationService {
    fn experiment_store_path(&self) -> Result<&Path, ApplicationServiceError> {
        self.config
            .store_path()
            .ok_or(ApplicationServiceError::StorePathRequired)
    }

    pub fn experiment_project_get(
        &self,
    ) -> Result<ExperimentProjectSnapshot, ApplicationServiceError> {
        let task_store_path = self.experiment_store_path()?;
        let aoi_ctx = {
            let store = self.store_handle()?;
            let guard = store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            load_active_aoi_from_store(&guard)?
        };
        let now = now_utc_iso();
        let db_path = ExperimentProjectStore::sibling_path_for_task_store(task_store_path);
        let mut exp_store = ExperimentProjectStore::open(&db_path)?;
        if aoi_ctx.bbox.is_some() || aoi_ctx.geometry_json.is_some() {
            let bbox_json = aoi_ctx.bbox.and_then(|b| serde_json::to_string(&b).ok());
            exp_store
                .update_project_aoi(
                    DEFAULT_EXPERIMENT_PROJECT_ID,
                    aoi_ctx.display_name.as_deref(),
                    bbox_json.as_deref(),
                    aoi_ctx.geometry_json.as_deref(),
                    &now,
                )
                .map_err(map_experiment_store_error)?;
        }
        let (project, items) = exp_store
            .load_or_init_default(&now)
            .map_err(map_experiment_store_error)?;
        Ok(build_snapshot(project, items))
    }

    pub fn experiment_project_add_task(
        &self,
        task_id: &str,
    ) -> Result<ExperimentProjectSnapshot, ApplicationServiceError> {
        let task_id = task_id.trim();
        if task_id.is_empty() {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "task_id 不能为空".to_owned(),
            });
        }
        let task_store_path = self.experiment_store_path()?;
        let (summary, aoi_ctx) = {
            let store = self.store_handle()?;
            let guard = store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            find_task_summary_in_store(&guard, task_id)?
        };
        if summary.status != TaskLifecycleStatus::Completed {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "仅支持将已完成的任务纳入实验工程".to_owned(),
            });
        }

        let (primary_artifact_path, provenance_path) = discover_primary_artifact(&summary)?;
        let (size_bytes, sha256) = compute_file_sha256(&primary_artifact_path)?;
        if size_bytes == 0 {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "数据产物为空文件，无法纳入实验工程".to_owned(),
            });
        }

        let dataset_id = summary
            .dataset_id
            .clone()
            .unwrap_or_else(|| summary.provider_id.clone());
        let license_id = infer_license_id(&summary.provider_id, &dataset_id);
        let now = now_utc_iso();
        let db_path = ExperimentProjectStore::sibling_path_for_task_store(task_store_path);
        let mut exp_store = ExperimentProjectStore::open(&db_path)?;

        let aoi_bbox_json = aoi_ctx.bbox.and_then(|b| serde_json::to_string(&b).ok());
        if aoi_bbox_json.is_some() || aoi_ctx.geometry_json.is_some() {
            exp_store
                .update_project_aoi(
                    DEFAULT_EXPERIMENT_PROJECT_ID,
                    aoi_ctx.display_name.as_deref(),
                    aoi_bbox_json.as_deref(),
                    aoi_ctx.geometry_json.as_deref(),
                    &now,
                )
                .map_err(map_experiment_store_error)?;
        }

        let (project, items) = exp_store
            .upsert_item(&ExperimentProjectUpsertItem {
                item_id: Uuid::new_v4().to_string(),
                project_id: DEFAULT_EXPERIMENT_PROJECT_ID.to_owned(),
                task_id: summary.task_id,
                provider_id: summary.provider_id,
                dataset_id,
                task_type: summary.task_type,
                output_root: summary.output_root.to_string_lossy().to_string(),
                primary_artifact_path: primary_artifact_path.to_string_lossy().to_string(),
                provenance_path: provenance_path.map(|p| p.to_string_lossy().to_string()),
                size_bytes,
                sha256,
                bbox_json: aoi_bbox_json,
                license_id,
                timestamp: now,
            })
            .map_err(map_experiment_store_error)?;

        Ok(build_snapshot(project, items))
    }

    pub fn experiment_project_remove_item(
        &self,
        item_id: &str,
    ) -> Result<ExperimentProjectSnapshot, ApplicationServiceError> {
        let item_id = item_id.trim();
        if item_id.is_empty() {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "item_id 不能为空".to_owned(),
            });
        }
        let task_store_path = self.experiment_store_path()?;
        let now = now_utc_iso();
        let db_path = ExperimentProjectStore::sibling_path_for_task_store(task_store_path);
        let mut exp_store = ExperimentProjectStore::open(&db_path)?;
        let (project, items) = exp_store
            .remove_item(DEFAULT_EXPERIMENT_PROJECT_ID, item_id, &now)
            .map_err(map_experiment_store_error)?;
        Ok(build_snapshot(project, items))
    }

    pub fn experiment_project_export(
        &self,
        export_dir: Option<PathBuf>,
    ) -> Result<ExperimentProjectExportResult, ApplicationServiceError> {
        let task_store_path = self.experiment_store_path()?;
        let aoi_ctx = {
            let store = self.store_handle()?;
            let guard = store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            load_active_aoi_from_store(&guard)?
        };
        let now = now_utc_iso();
        let db_path = ExperimentProjectStore::sibling_path_for_task_store(task_store_path);
        let mut exp_store = ExperimentProjectStore::open(&db_path)?;
        if aoi_ctx.bbox.is_some() || aoi_ctx.geometry_json.is_some() {
            let bbox_json = aoi_ctx.bbox.and_then(|b| serde_json::to_string(&b).ok());
            exp_store
                .update_project_aoi(
                    DEFAULT_EXPERIMENT_PROJECT_ID,
                    aoi_ctx.display_name.as_deref(),
                    bbox_json.as_deref(),
                    aoi_ctx.geometry_json.as_deref(),
                    &now,
                )
                .map_err(map_experiment_store_error)?;
        }
        let (project, records) = exp_store
            .load_or_init_default(&now)
            .map_err(map_experiment_store_error)?;

        let target_dir = match export_dir.filter(|p| !p.to_string_lossy().trim().is_empty()) {
            Some(dir) => dir,
            None => resolve_default_export_dir(task_store_path, &project, &records),
        };
        fs::create_dir_all(&target_dir).map_err(|err| ApplicationServiceError::InvalidRequest {
            reason: format!("创建实验工程导出目录失败 {}: {err}", target_dir.display()),
        })?;

        let items: Vec<ExperimentProjectItemView> =
            records.iter().map(verify_item_on_disk).collect();
        let verified_count = items
            .iter()
            .filter(|i| i.integrity_status == "verified")
            .count();
        let missing_or_modified_count = items.len().saturating_sub(verified_count);
        let has_noncommercial_restricted_data = items.iter().any(|i| i.is_noncommercial);
        let project_bbox = parse_bbox_json(project.aoi_bbox_json.as_deref());

        let manifest_path = target_dir.join("experiment-project.v1.json");
        let aoi_geojson_path = target_dir.join("study_area.aoi.geojson");
        let footprints_geojson_path = target_dir.join("scene_footprints.geojson");
        let readme_path = target_dir.join("README_EXPERIMENT.md");

        let manifest_json = json!({
            "schema_version": "experiment-project.v1",
            "exported_at": now,
            "project_id": project.project_id,
            "title": project.title,
            "max_items": MAX_EXPERIMENT_PROJECT_ITEMS,
            "item_count": items.len(),
            "verified_count": verified_count,
            "missing_or_modified_count": missing_or_modified_count,
            "has_noncommercial_restricted_data": has_noncommercial_restricted_data,
            "aoi": {
                "display_name": project.aoi_display_name,
                "bbox": project_bbox,
            },
            "items": items,
        });
        let manifest_bytes = serde_json::to_vec_pretty(&manifest_json).map_err(|err| {
            ApplicationServiceError::InvalidRequest {
                reason: format!("序列化实验工程清单失败: {err}"),
            }
        })?;
        write_file_atomic(&manifest_path, &manifest_bytes)?;

        let mut aoi_features: Vec<Value> = Vec::new();
        let aoi_geom_val: Option<Value> = project
            .aoi_geojson
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Map<String, Value>>(raw).ok())
            .map(Value::Object)
            .or_else(|| project_bbox.map(bbox_polygon_geometry));
        if let Some(geom) = aoi_geom_val {
            aoi_features.push(json!({
                "type": "Feature",
                "id": "study-area-aoi",
                "geometry": geom,
                "properties": {
                    "display_name": project.aoi_display_name,
                    "bbox": project_bbox,
                    "crs": "EPSG:4326"
                }
            }));
        }
        let aoi_fc = json!({
            "type": "FeatureCollection",
            "name": "study_area_aoi",
            "features": aoi_features,
        });
        let aoi_bytes = serde_json::to_vec_pretty(&aoi_fc).map_err(|err| {
            ApplicationServiceError::InvalidRequest {
                reason: format!("序列化 AOI GeoJSON 失败: {err}"),
            }
        })?;
        write_file_atomic(&aoi_geojson_path, &aoi_bytes)?;

        let mut footprint_features: Vec<Value> = Vec::new();
        for item in &items {
            if let Some(bbox) = item.bbox.or(project_bbox) {
                footprint_features.push(json!({
                    "type": "Feature",
                    "id": item.item_id,
                    "geometry": bbox_polygon_geometry(bbox),
                    "properties": {
                        "item_id": item.item_id,
                        "task_id": item.task_id,
                        "provider_id": item.provider_id,
                        "dataset_id": item.dataset_id,
                        "license_id": item.license_id,
                        "is_noncommercial": item.is_noncommercial,
                        "integrity_status": item.integrity_status,
                        "size_bytes": item.size_bytes,
                        "sha256": item.sha256,
                        "primary_artifact_path": item.primary_artifact_path,
                    }
                }));
            }
        }
        let footprints_fc = json!({
            "type": "FeatureCollection",
            "name": "scene_footprints",
            "features": footprint_features,
        });
        let footprints_bytes = serde_json::to_vec_pretty(&footprints_fc).map_err(|err| {
            ApplicationServiceError::InvalidRequest {
                reason: format!("序列化影像范围图斑失败: {err}"),
            }
        })?;
        write_file_atomic(&footprints_geojson_path, &footprints_bytes)?;

        let mut readme = String::new();
        let _ = writeln!(&mut readme, "# EOStage Reproducible Experiment Bundle\n");
        let _ = writeln!(&mut readme, "- Project ID: `{}`", project.project_id);
        let _ = writeln!(&mut readme, "- Exported At (UTC): `{now}`");
        let _ = writeln!(
            &mut readme,
            "- Managed Assets: `{}` / `{MAX_EXPERIMENT_PROJECT_ITEMS}` (Verified: `{verified_count}`, Missing/Modified: `{missing_or_modified_count}`)",
            items.len()
        );
        if let Some(name) = project.aoi_display_name.as_deref() {
            let _ = writeln!(&mut readme, "- Study Area AOI: `{name}`");
        }
        if let Some(bbox) = project_bbox {
            let _ = writeln!(
                &mut readme,
                "- Study Area BBox (WGS84): `[{:.6}, {:.6}, {:.6}, {:.6}]`",
                bbox[0], bbox[1], bbox[2], bbox[3]
            );
        }
        let _ = writeln!(&mut readme);

        if has_noncommercial_restricted_data {
            let _ = writeln!(&mut readme, "> [!WARNING]");
            let _ = writeln!(
                &mut readme,
                "> **CC BY-NC-SA 4.0 Non-Commercial License Notice:** This experiment bundle includes **FABDEM V1-2** elevation products licensed under **CC BY-NC-SA 4.0** (non-commercial research / educational use only; derivative redistribution must share alike and carry attribution).\n"
            );
        }

        let _ = writeln!(
            &mut readme,
            "## Artifact Integrity Verification (SHA-256)\n"
        );
        let _ = writeln!(
            &mut readme,
            "| Item ID | Task ID | Provider | Dataset | License | Status | Size (Bytes) | SHA-256 |"
        );
        let _ = writeln!(
            &mut readme,
            "| --- | --- | --- | --- | --- | --- | ---: | --- |"
        );
        for item in &items {
            let _ = writeln!(
                &mut readme,
                "| `{}` | `{}` | `{}` | `{}` | `{}` | `{}` | {} | `{}` |",
                item.item_id,
                item.task_id,
                item.provider_id,
                item.dataset_id,
                item.license_id,
                item.integrity_status,
                item.size_bytes,
                item.sha256
            );
        }
        let _ = writeln!(&mut readme, "\n## QGIS Quick Import\n");
        let _ = writeln!(
            &mut readme,
            "1. Drag `study_area.aoi.geojson` and `scene_footprints.geojson` into QGIS to inspect AOI & product coverage."
        );
        let _ = writeln!(
            &mut readme,
            "2. Refer to `experiment-project.v1.json` for exact local raster paths and SHA-256 digests."
        );
        write_file_atomic(&readme_path, readme.as_bytes())?;

        Ok(ExperimentProjectExportResult {
            project_id: project.project_id,
            export_dir: target_dir.to_string_lossy().to_string(),
            manifest_path: manifest_path.to_string_lossy().to_string(),
            aoi_geojson_path: aoi_geojson_path.to_string_lossy().to_string(),
            footprints_geojson_path: footprints_geojson_path.to_string_lossy().to_string(),
            readme_path: readme_path.to_string_lossy().to_string(),
            verified_count,
            missing_or_modified_count,
            has_noncommercial_restricted_data,
        })
    }
}
