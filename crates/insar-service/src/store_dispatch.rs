use std::path::PathBuf;
use std::sync::Mutex;

use insar_core::{
    DownloadSnapshotRequest, SidecarOperation, SidecarResponse, StoreAppendTaskRequest,
    StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreDeleteHistoryRequest,
    StoreMutationResponse, StoreRestoreRequest, StoreSearchBatchRequest, StoreUpdateTaskRequest,
    StoreUpdateTaskViewRequest, StoreUpdateWorkbenchRequest,
};
use insar_store::{StoreError, TaskStore};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::dispatcher::{
    error_response, invalid_payload_response, success_response, typed_success_response,
};

pub(crate) struct LazyTaskStore {
    path: PathBuf,
    store: Option<TaskStore>,
}

pub(crate) fn dispatch_download_snapshot(
    lazy: &Mutex<LazyTaskStore>,
    request_id: String,
    payload: &Map<String, Value>,
) -> SidecarResponse {
    let request: DownloadSnapshotRequest = match decode(payload) {
        Ok(request) => request,
        Err(reason) => return invalid_payload_response(request_id, reason),
    };
    let mut guard = match lazy.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                request_id,
                "V3_STORE_LOCK_POISONED",
                "Rust 下载状态存储锁异常",
                Map::new(),
            )
        }
    };
    match guard
        .get()
        .and_then(|store| store.load_download_snapshot(&request.task_id))
    {
        Ok(snapshot) => typed_success_response(request_id, &snapshot),
        Err(error) => store_error_response(request_id, &error),
    }
}

impl LazyTaskStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path, store: None }
    }

    pub(crate) fn get(&mut self) -> Result<&mut TaskStore, StoreError> {
        if self.store.is_none() {
            self.store = Some(TaskStore::open(&self.path)?);
        }
        Ok(self.store.as_mut().expect("store was initialized"))
    }
}

pub(crate) fn dispatch_startup_health(
    lazy: &Mutex<LazyTaskStore>,
    request_id: String,
) -> SidecarResponse {
    let mut guard = match lazy.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                request_id,
                "V3_STORE_LOCK_POISONED",
                "Rust 任务存储锁状态异常",
                Map::new(),
            );
        }
    };
    match guard.get().and_then(|store| store.health()) {
        Ok(_) => success_response(
            request_id,
            Map::from_iter([("status".to_string(), json!("healthy"))]),
        ),
        Err(error) => store_error_response(request_id, &error),
    }
}

pub(crate) fn default_store_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("APPDATA")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            #[cfg(target_os = "macos")]
            {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
                    .map(|home| home.join("Library").join("Application Support"))
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                std::env::var_os("XDG_DATA_HOME")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
                    .or_else(|| {
                        std::env::var_os("HOME")
                            .filter(|v| !v.is_empty())
                            .map(PathBuf::from)
                            .map(|home| home.join(".local").join("share"))
                    })
            }
            #[cfg(not(unix))]
            {
                None
            }
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("EOStage").join("v3").join("tasks.sqlite3")
}

pub(crate) fn dispatch_store(
    lazy: &Mutex<LazyTaskStore>,
    operation: SidecarOperation,
    request_id: String,
    payload: &Map<String, Value>,
) -> SidecarResponse {
    let mut guard = match lazy.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                request_id,
                "V3_STORE_LOCK_POISONED",
                "Rust 任务存储锁状态异常",
                Map::new(),
            );
        }
    };
    let store = match guard.get() {
        Ok(store) => store,
        Err(error) => return store_error_response(request_id, &error),
    };
    match operation {
        SidecarOperation::StoreSearchBatch => {
            let request: StoreSearchBatchRequest = match decode(payload) {
                Ok(request) => request,
                Err(reason) => return invalid_payload_response(request_id, reason),
            };
            match store.save_search_batch(&request) {
                Ok(()) => success_response(
                    request_id,
                    Map::from_iter([
                        ("batch_id".to_string(), json!(request.batch.batch_id)),
                        ("revision".to_string(), json!(request.workbench.revision)),
                    ]),
                ),
                Err(error) => store_error_response(request_id, &error),
            }
        }
        SidecarOperation::StoreUpdateWorkbench => {
            let request: StoreUpdateWorkbenchRequest = match decode(payload) {
                Ok(request) => request,
                Err(reason) => return invalid_payload_response(request_id, reason),
            };
            match store.update_workbench_request(&request) {
                Ok(()) => success_response(
                    request_id,
                    Map::from_iter([
                        ("batch_id".to_string(), json!(request.state.batch_id)),
                        ("revision".to_string(), json!(request.state.revision)),
                    ]),
                ),
                Err(error) => store_error_response(request_id, &error),
            }
        }
        SidecarOperation::StoreCreateTask => {
            let request: StoreCreateTaskRequest = match decode(payload) {
                Ok(request) => request,
                Err(reason) => return invalid_payload_response(request_id, reason),
            };
            match store.create_task(&request) {
                Ok(task) => {
                    mutation_response(request_id, task.runtime.task_id, task.runtime.revision)
                }
                Err(error) => store_error_response(request_id, &error),
            }
        }
        SidecarOperation::StoreUpdateTask => {
            let request: StoreUpdateTaskRequest = match decode(payload) {
                Ok(request) => request,
                Err(reason) => return invalid_payload_response(request_id, reason),
            };
            match store.update_task(&request) {
                Ok(task) => {
                    mutation_response(request_id, task.runtime.task_id, task.runtime.revision)
                }
                Err(error) => store_error_response(request_id, &error),
            }
        }
        SidecarOperation::StoreUpdateTaskView => {
            let request: StoreUpdateTaskViewRequest = match decode(payload) {
                Ok(request) => request,
                Err(reason) => return invalid_payload_response(request_id, reason),
            };
            match store.update_task_view(&request) {
                Ok(task) => {
                    mutation_response(request_id, task.runtime.task_id, task.runtime.revision)
                }
                Err(error) => store_error_response(request_id, &error),
            }
        }
        SidecarOperation::StoreAppendTask => {
            let request: StoreAppendTaskRequest = match decode(payload) {
                Ok(request) => request,
                Err(reason) => return invalid_payload_response(request_id, reason),
            };
            match store.append_task_items(&request) {
                Ok(task) => {
                    mutation_response(request_id, task.runtime.task_id, task.runtime.revision)
                }
                Err(error) => store_error_response(request_id, &error),
            }
        }
        SidecarOperation::StoreArchiveTask => {
            let request: StoreArchiveTaskRequest = match decode(payload) {
                Ok(request) => request,
                Err(reason) => return invalid_payload_response(request_id, reason),
            };
            match store.archive_task(&request) {
                Ok(history) => mutation_response(
                    request_id,
                    history.task.runtime.task_id,
                    history.task.runtime.revision,
                ),
                Err(error) => store_error_response(request_id, &error),
            }
        }
        SidecarOperation::StoreDeleteHistory => {
            let request: StoreDeleteHistoryRequest = match decode(payload) {
                Ok(request) => request,
                Err(reason) => return invalid_payload_response(request_id, reason),
            };
            match store.delete_history(&request) {
                Ok(()) => success_response(
                    request_id,
                    Map::from_iter([
                        ("task_id".to_string(), json!(request.task_id)),
                        ("deleted".to_string(), json!(true)),
                    ]),
                ),
                Err(error) => store_error_response(request_id, &error),
            }
        }
        SidecarOperation::StoreRestore => {
            let request: StoreRestoreRequest = match decode(payload) {
                Ok(request) => request,
                Err(reason) => return invalid_payload_response(request_id, reason),
            };
            match store.restore(&request) {
                Ok(response) => match serde_json::to_value(response)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                {
                    Some(value) => success_response(request_id, value),
                    None => error_response(
                        request_id,
                        "V3_STORE_CORRUPT_ROW",
                        "Rust 任务存储恢复响应无效",
                        Map::new(),
                    ),
                },
                Err(error) => store_error_response(request_id, &error),
            }
        }
        _ => error_response(
            request_id,
            "V3_PROTOCOL_INVALID_REQUEST",
            "Sidecar 任务存储操作无效",
            Map::new(),
        ),
    }
}

fn decode<T: DeserializeOwned>(payload: &Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(payload.clone())).map_err(|error| error.to_string())
}

fn mutation_response(request_id: String, task_id: String, revision: u64) -> SidecarResponse {
    typed_success_response(request_id, &StoreMutationResponse { task_id, revision })
}

fn store_error_response(request_id: String, error: &StoreError) -> SidecarResponse {
    error_response(
        request_id,
        error.code(),
        "Rust 任务存储操作失败",
        Map::new(),
    )
}
