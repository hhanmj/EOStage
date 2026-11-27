use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use insar_application_service::{
    ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    CdseNetworkMode, Sentinel2CreateBandBatchTaskRequest,
};
use insar_core::{CatalogSearchQuery, DownloadLayout, DownloadOutcome};
use insar_download::{
    evaluate_resolver_cutover_eligibility, AsfCollectionProfileRegistry,
    AssetExecutionPolicyRegistry, ExecutionPolicyError, ResolverCutoverEligibility,
};
use insar_store::StoreWorkbenchPageRequest;
use serde_json::{json, Map, Value};

const LIVE_ROOT_ENV: &str = "INSAR_SENTINEL2_MPC_LIVE_ROOT";
const SOURCE_BATCH_ID: &str = "sentinel2-mpc-live-source";
const TASK_ID: &str = "sentinel2-mpc-live-task";
const ITEM_ID: &str = "S2A_MSIL2A_20261002T053241_R105_T44RKV_20261002T101803";
const SAFE_NAME: &str = "S2A_MSIL2A_20261002T053241_N0513_R105_T44RKV_20261002T101803.SAFE";
const ASSET_KEYS: [&str; 3] = ["B02", "B03", "B04"];
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const POLL: Duration = Duration::from_millis(250);

struct Workspace {
    root: PathBuf,
    store: PathBuf,
    output: PathBuf,
    clean_on_drop: bool,
}

impl Workspace {
    fn create() -> Result<Self, String> {
        let root = std::env::var_os(LIVE_ROOT_ENV)
            .map(PathBuf::from)
            .ok_or_else(|| "S2_MPC_LIVE_ROOT_REQUIRED".to_owned())?;
        if !root.is_absolute() || root.exists() {
            return Err("S2_MPC_LIVE_ROOT_MUST_BE_ABSENT_ABSOLUTE".to_owned());
        }
        let output = root.join("output");
        fs::create_dir_all(&output).map_err(|_| "S2_MPC_LIVE_ROOT_UNAVAILABLE".to_owned())?;
        Ok(Self {
            store: root.join("tasks.sqlite3"),
            root,
            output,
            clean_on_drop: false,
        })
    }

    fn mark_success(&mut self) {
        self.clean_on_drop = true;
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if self.clean_on_drop {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
#[ignore = "requires explicit opt-in and public Sentinel-2 MPC network access"]
fn sentinel2_mpc_three_band_live_acceptance() {
    match run_live() {
        Ok((bytes, part_observed)) => {
            eprintln!("S2_MPC_LIVE outcome=completed");
            eprintln!("scene_id={ITEM_ID}");
            eprintln!("safe_name={SAFE_NAME}");
            eprintln!("asset_count=3");
            eprintln!("actual_bytes={bytes}");
            eprintln!("part_observed={part_observed}");
            eprintln!("atomic_final=true");
            eprintln!("store_restore_completed=true");
            eprintln!("credential_input_count=0");
            eprintln!("workspace_cleaned=true");
        }
        Err(code) => {
            eprintln!("S2_MPC_LIVE outcome=stopped");
            eprintln!("code={code}");
            panic!("S2_MPC_LIVE_STOPPED");
        }
    }
}

fn run_live() -> Result<(u64, bool), String> {
    let mut workspace = Workspace::create()?;
    let service =
        ApplicationService::new(ApplicationServiceConfig::business_store(&workspace.store));
    service
        .set_cdse_network_policy(CdseNetworkMode::Direct)
        .map_err(|error| format!("S2_MPC_NETWORK_POLICY_{}", error.code()))?;

    let request = live_search_request()?;
    let frozen = service
        .catalog_search_and_freeze(&request)
        .map_err(|error| format!("S2_MPC_SEARCH_{}", error.code()))?;
    let workbench = frozen
        .core
        .workbench
        .as_ref()
        .ok_or_else(|| "S2_MPC_WORKBENCH_MISSING".to_owned())?;
    let page = service
        .workbench_page(&StoreWorkbenchPageRequest {
            batch_id: SOURCE_BATCH_ID.to_owned(),
            expected_revision: workbench.revision,
            page_index: 0,
            page_size: 10,
        })
        .map_err(|error| format!("S2_MPC_WORKBENCH_PAGE_{}", error.code()))?;
    let candidate = page
        .page
        .items
        .iter()
        .find(|entry| entry.item.display_name == SAFE_NAME)
        .ok_or_else(|| "S2_MPC_EXACT_SCENE_NOT_FOUND".to_owned())?;

    let created = service
        .create_sentinel2_band_batch_task(&Sentinel2CreateBandBatchTaskRequest {
            request_id: "sentinel2-mpc-live-create".to_owned(),
            source_batch_id: SOURCE_BATCH_ID.to_owned(),
            expected_workbench_revision: workbench.revision,
            item_ids: vec![candidate.item.item_id.clone()],
            asset_keys: ASSET_KEYS.iter().map(|value| (*value).to_owned()).collect(),
            task_id: TASK_ID.to_owned(),
            output_root: workspace.output.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 3,
            created_at: "2026-10-05T00:00:00Z".to_owned(),
        })
        .map_err(|error| format!("S2_MPC_CREATE_{}", error.code()))?;
    if created.admission.admitted_count != 1 || created.admission.rejected_count != 0 {
        return Err("S2_MPC_ADMISSION_INVALID".to_owned());
    }

    let frozen_snapshot = service
        .snapshot(true)
        .map_err(|error| format!("S2_MPC_SNAPSHOT_{}", error.code()))?;
    let task = frozen_snapshot
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == TASK_ID)
        .ok_or_else(|| "S2_MPC_TASK_MISSING".to_owned())?;
    if task.candidates.len() != ASSET_KEYS.len() {
        return Err("S2_MPC_FROZEN_ASSET_COUNT_INVALID".to_owned());
    }
    for item in &task.candidates {
        match evaluate_resolver_cutover_eligibility(
            item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ) {
            ResolverCutoverEligibility::Eligible(_) => {}
            ResolverCutoverEligibility::Ineligible(reason) => {
                return Err(format!("S2_MPC_POLICY_INELIGIBLE_{}", reason.code()));
            }
            ResolverCutoverEligibility::Invalid(ExecutionPolicyError::IdentityMismatch(field)) => {
                return Err(format!("S2_MPC_POLICY_IDENTITY_{field}"));
            }
            ResolverCutoverEligibility::Invalid(error) => {
                return Err(format!("S2_MPC_POLICY_INVALID_{}", error.code()));
            }
        }
    }
    let expected = task
        .candidates
        .iter()
        .map(|item| {
            let size = item
                .asset_access
                .as_ref()
                .and_then(|access| access.expected_size_bytes)
                .filter(|size| *size > 0)
                .ok_or_else(|| "S2_MPC_EXPECTED_SIZE_MISSING".to_owned())?;
            Ok((item.item_id.clone(), size))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;

    let deadline = Instant::now() + TRANSFER_TIMEOUT;
    let mut part_observed = false;
    let final_snapshot = loop {
        part_observed |= contains_part(&workspace.output);
        let snapshot = service
            .download_snapshot(TASK_ID)
            .map_err(|error| format!("S2_MPC_DOWNLOAD_SNAPSHOT_{}", error.code()))?;
        if let Some(failed) = snapshot
            .results
            .iter()
            .find(|result| result.outcome != DownloadOutcome::Completed)
        {
            let (code, retryable, class) = failed
                .failure
                .as_ref()
                .map(|failure| {
                    let class = match failure.reason_zh.as_str() {
                        "冻结的 Resolver 执行策略无效，未回退到 Legacy 下载。" => {
                            "POLICY"
                        }
                        "无法解析或传输冻结的下载资产，请稍后重试。" => {
                            "RUNTIME"
                        }
                        _ => "OTHER",
                    };
                    (failure.code.as_str(), failure.retryable, class)
                })
                .unwrap_or(("NO_FAILURE_DETAIL", false, "NONE"));
            return Err(format!(
                "S2_MPC_TRANSFER_{code}_{}_{}",
                if retryable { "RETRYABLE" } else { "TERMINAL" },
                class
            ));
        }
        if snapshot.results.len() == ASSET_KEYS.len()
            && snapshot
                .results
                .iter()
                .all(|result| result.outcome == DownloadOutcome::Completed)
        {
            break snapshot;
        }
        if Instant::now() >= deadline {
            return Err("S2_MPC_TRANSFER_TIMEOUT".to_owned());
        }
        thread::sleep(POLL);
    };

    if !part_observed {
        return Err("S2_MPC_PART_NOT_OBSERVED".to_owned());
    }
    let mut total = 0_u64;
    for result in &final_snapshot.results {
        let expected_size = *expected
            .get(&result.item_id)
            .ok_or_else(|| "S2_MPC_RESULT_IDENTITY_INVALID".to_owned())?;
        let actual = fs::metadata(&result.destination)
            .map_err(|_| "S2_MPC_FINAL_FILE_MISSING".to_owned())?
            .len();
        if actual != expected_size
            || actual != result.bytes_written
            || part_path(&result.destination).exists()
        {
            return Err("S2_MPC_FINAL_SIZE_INVALID".to_owned());
        }
        total = total
            .checked_add(actual)
            .ok_or_else(|| "S2_MPC_TOTAL_OVERFLOW".to_owned())?;
    }

    service
        .shutdown()
        .map_err(|_| "S2_MPC_STORE_CLOSE_FAILED".to_owned())?;
    let reopened =
        ApplicationService::new(ApplicationServiceConfig::business_store(&workspace.store));
    let restored = reopened
        .download_snapshot(TASK_ID)
        .map_err(|_| "S2_MPC_STORE_REOPEN_FAILED".to_owned())?;
    if restored.results.len() != ASSET_KEYS.len()
        || restored
            .results
            .iter()
            .any(|result| result.outcome != DownloadOutcome::Completed)
    {
        return Err("S2_MPC_STORE_RESTORE_INVALID".to_owned());
    }
    reopened
        .shutdown()
        .map_err(|_| "S2_MPC_REOPEN_CLOSE_FAILED".to_owned())?;

    workspace.mark_success();
    Ok((total, part_observed))
}

fn live_search_request() -> Result<ApplicationCatalogSearchAndFreezeRequest, String> {
    let aoi_geojson: Map<String, Value> = serde_json::from_value(json!({
        "type": "Polygon",
        "coordinates": [[
            [77.8381884, 30.6071543],
            [79.015281, 30.6071543],
            [79.015281, 31.6197962],
            [77.8381884, 31.6197962],
            [77.8381884, 30.6071543]
        ]]
    }))
    .map_err(|_| "S2_MPC_AOI_INVALID".to_owned())?;
    let search: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l2a",
        "provider_id": "copernicus.dataspace.odata",
        "native_collection_id": "SENTINEL-2",
        "aoi_geojson": aoi_geojson,
        "result_limit": 10,
        "filters": {
            "datetime": {
                "start": "2026-10-02T05:30:00Z",
                "end": "2026-10-02T05:35:00Z"
            },
            "product_type": "S2MSI2A",
            "cloud_cover_lte": 100.0
        }
    }))
    .map_err(|_| "S2_MPC_QUERY_INVALID".to_owned())?;
    search
        .validate()
        .map_err(|_| "S2_MPC_QUERY_INVALID".to_owned())?;
    Ok(ApplicationCatalogSearchAndFreezeRequest {
        request_id: "sentinel2-mpc-live-search".to_owned(),
        search,
        batch_id: SOURCE_BATCH_ID.to_owned(),
        created_at: "2026-10-05T00:00:00Z".to_owned(),
        aoi_name: "Sentinel-2 T44RKV accepted scene bbox".to_owned(),
        aoi_geojson: Some(aoi_geojson),
        workbench_page_size: 10,
    })
}

fn contains_part(root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            contains_part(&path)
        } else {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.ends_with(".part"))
        }
    })
}

fn part_path(final_path: &Path) -> PathBuf {
    let file_name = final_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    final_path.with_file_name(format!("{file_name}.part"))
}
