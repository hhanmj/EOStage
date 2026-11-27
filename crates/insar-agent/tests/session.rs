//! End-to-end Agent host loop: recorded model replay -> tier-0 reads -> plan
//! row + hash -> desktop confirm card -> approved execution through the host
//! port -> results back to the model -> cited conclusion -> sealed run record.
use insar_agent::{
    executor::ErrorClass,
    host::UnifiedApplication,
    permission::{PermissionMode, StopAllResult},
    session::{
        AgentHost, HostError, ModelSource, OpenedModel, SendAttachment, SendContext, StoreAccess,
        StreamEvent,
    },
};
use insar_agent_model::{
    agent::Model, replay, transport::ModelRoute, CancellationToken, Completion, Message,
    ModelError, ModelEvent, Protocol, ReasoningMode, ToolDescription,
};
use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskStateCommandKind,
};
use insar_store::{StoreBatchPageRequest, TaskStore};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
struct Fake {
    store: Arc<Mutex<TaskStore>>,
    submits: Arc<AtomicUsize>,
    /// Makes the next Store critical sections fail as if the Store were gone.
    store_down: Arc<std::sync::atomic::AtomicBool>,
    /// Runs once at the first failing Store access (e.g. a concurrent cancel).
    on_store_down: Arc<Mutex<Option<StoreDownHook>>>,
    /// Runs once at the start of the next Store access.
    before_store: Arc<Mutex<Option<StoreDownHook>>>,
    /// `create_task` admits like the service (reusing a live task of the same
    /// products with `created=false`) instead of answering Unsupported.
    create_ok: Arc<std::sync::atomic::AtomicBool>,
    /// Every `create_task` request the service received, as sent.
    created: Arc<Mutex<Vec<Value>>>,
    /// Tasks whose cancel the service always refuses.
    fail_cancel: Arc<Mutex<std::collections::HashSet<String>>>,
    /// Tasks whose next cancel loses a revision race: another writer pauses
    /// the task first (revision + 1) and the cancel is refused.
    conflict_once: Arc<Mutex<std::collections::HashSet<String>>>,
    /// The default download directory the service reports (storage root).
    default_output: Arc<Mutex<Option<String>>>,
    /// When set, `search_start` succeeds and freezes this batch under the
    /// host-minted request id (the service's in-memory lookup below).
    next_search_batch: Arc<Mutex<Option<String>>>,
    search_batches: Arc<Mutex<std::collections::HashMap<String, String>>>,
    /// The lookup forgets an entry once read (as a restart would).
    forget_search_batch: Arc<std::sync::atomic::AtomicBool>,
    /// The request id of the last started search; `search_progress` of
    /// `req-latest` reports it (a replayed turn cannot know the minted id).
    last_search: Arc<Mutex<Option<String>>>,
    accounts_ready: Arc<std::sync::atomic::AtomicBool>,
    /// What `list_sources` returns (C1 SourceDescriptor rows).
    sources: Arc<Mutex<Value>>,
    /// Active working AOI reported and mutated by `working_aoi_get` / `working_aoi_set`.
    working_aoi: Arc<Mutex<Value>>,
    /// Every `application_search_start` call received by the fake service.
    searches: Arc<Mutex<Vec<Value>>>,
    /// `items_kept` reported by `application_search_progress` (default 3).
    search_items_kept: Arc<AtomicUsize>,
    /// When true, `application_search_start` returns a terminal `"done"` summary
    /// directly (matching `AgentApplication::application_search_start`).
    search_start_terminal: Arc<std::sync::atomic::AtomicBool>,
    extension_tools: Arc<Mutex<Vec<insar_agent_model::ToolDescription>>>,
    extension_calls: Arc<Mutex<Vec<(String, Value)>>>,
}
type StoreDownHook = Box<dyn FnOnce() + Send>;
impl StoreAccess for Fake {
    fn with_store<T>(&self, operation: impl FnOnce(&mut TaskStore) -> T) -> Result<T, HostError> {
        let before = self.before_store.lock().unwrap().take();
        if let Some(hook) = before {
            hook();
        }
        if self.store_down.load(Ordering::SeqCst) {
            let hook = self.on_store_down.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
            return Err(HostError::StoreUnavailable);
        }
        Ok(operation(&mut self.store.lock().unwrap()))
    }
}
impl UnifiedApplication for Fake {
    fn application_service_readiness(&self) -> Result<Value, ErrorClass> {
        Ok(json!({"status":"store_ready"}))
    }
    fn application_provider_capabilities(&self) -> Result<Value, ErrorClass> {
        Ok(json!({}))
    }
    fn application_working_aoi(&self) -> Result<Value, ErrorClass> {
        Ok(self.working_aoi.lock().unwrap().clone())
    }
    fn application_workbench_page(&self, _: u64, _: u64) -> Result<Value, ErrorClass> {
        Err(ErrorClass::Unsupported)
    }
    fn application_download_task_page(&self, _: &str, _: u64, _: u64) -> Result<Value, ErrorClass> {
        Err(ErrorClass::Unsupported)
    }
    fn application_download_task_event_page(&self, _: &str, _: u64) -> Result<Value, ErrorClass> {
        Err(ErrorClass::Unsupported)
    }
    fn application_credential_categories(&self) -> Result<Value, ErrorClass> {
        Ok(json!({}))
    }
    fn application_list_sources(&self) -> Result<Value, ErrorClass> {
        Ok(self.sources.lock().unwrap().clone())
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
        self.submits.fetch_add(1, Ordering::SeqCst);
        self.searches.lock().unwrap().push(json!({
            "resource_id": resource_id,
            "aoi": aoi,
            "aoi_name": aoi_name,
            "time_range": time_range,
            "filters": filters,
            "request_id": request_id,
        }));
        let Some(batch) = self.next_search_batch.lock().unwrap().clone() else {
            return Err(ErrorClass::Unsupported);
        };
        self.search_batches
            .lock()
            .unwrap()
            .insert(request_id.into(), batch);
        *self.last_search.lock().unwrap() = Some(request_id.into());
        if self.search_start_terminal.load(Ordering::SeqCst) {
            return self.application_search_progress(request_id);
        }
        Ok(json!({
            "request_id": request_id, "resource_id": resource_id,
            "stage": "planning", "requests_done": 0, "items_kept": 0, "items_cap": 250,
            "elapsed_ms": 0, "message_zh": "正在准备检索", "terminal": false, "error": null,
        }))
    }
    /// A finished search (the service's own C1 `SearchProgress` shape);
    /// `req-running` is still searching and `req-local` reports a message
    /// that names a local path.
    fn application_search_progress(&self, request_id: &str) -> Result<Value, ErrorClass> {
        let latest = self.last_search.lock().unwrap().clone().unwrap_or_default();
        let request_id = if request_id == "req-latest" {
            latest.as_str()
        } else {
            request_id
        };
        let kept = self.search_items_kept.load(Ordering::SeqCst);
        let default_msg = format!("检索完成，保留 {kept} 景");
        let (stage, message, terminal) = match request_id {
            "req-running" => ("searching", "正在检索", false),
            "req-local" => ("done", "已保存到 C:\\Users\\fixture\\out", true),
            _ => ("done", default_msg.as_str(), true),
        };
        Ok(json!({
            "request_id": request_id, "resource_id": "science.sentinel-1.slc.v1",
            "stage": stage, "requests_done": 1, "items_kept": kept, "items_cap": 250,
            "elapsed_ms": 5, "message_zh": message, "terminal": terminal, "error": null,
        }))
    }
    /// Same projection as the desktop adapter (`agent_tools.rs`), from the Store.
    fn application_batch_page(
        &self,
        batch_id: &str,
        revision: u64,
        page: u32,
        page_size: u32,
    ) -> Result<Value, ErrorClass> {
        let page = self
            .store
            .lock()
            .unwrap()
            .load_batch_page(&StoreBatchPageRequest {
                batch_id: batch_id.into(),
                revision,
                page: page.into(),
                page_size: page_size.into(),
            })
            .map_err(|_| ErrorClass::InvalidParameters)?;
        Ok(json!({
            "search_batch_id": page.batch_id, "batch_id": page.batch_id,
            "revision": page.revision, "page": page.page, "page_size": page.page_size,
            "total_count": page.total_count,
            "items": page.items.iter().map(|item| json!({"item_id": item.item_id})).collect::<Vec<_>>(),
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
        let n = self.submits.fetch_add(1, Ordering::SeqCst);
        self.created.lock().unwrap().push(json!({
            "batch_id": batch_id, "revision": revision, "item_ids": item_ids, "options": options,
        }));
        if !self.create_ok.load(Ordering::SeqCst) {
            return Err(ErrorClass::Unsupported);
        }
        let mut store = self.store.lock().unwrap();
        if let Some(task) = store
            .active_task_for_products(batch_id, item_ids)
            .map_err(|_| ErrorClass::Internal)?
        {
            let id = task.snapshot.task_id;
            return Ok(json!({"task_id": id, "revision": 0, "status": "queued", "created": false}));
        }
        let task_id = format!("agent-task-{n}");
        let output = options
            .get("output_root")
            .and_then(Value::as_str)
            .unwrap_or("agent-out");
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: task_id.clone(),
                batch_id: batch_id.into(),
                selected_item_ids: item_ids.to_vec(),
                checked_item_ids: item_ids.to_vec(),
                page_index: 0,
                page_size: 24,
                output_root: PathBuf::from(output),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-10-26T00:00:00Z".into(),
            })
            .map_err(|_| ErrorClass::Business)?;
        Ok(json!({"task_id": task_id, "revision": 0, "status": "queued", "created": true}))
    }
    fn application_task_action(
        &self,
        task_id: &str,
        revision: u64,
        action: &str,
        event_id: &str,
    ) -> Result<Value, ErrorClass> {
        self.submits.fetch_add(1, Ordering::SeqCst);
        if action == "cancel" {
            if self.fail_cancel.lock().unwrap().contains(task_id) {
                return Err(ErrorClass::Business);
            }
            if self.conflict_once.lock().unwrap().remove(task_id) {
                self.store
                    .lock()
                    .unwrap()
                    .update_task(&StoreUpdateTaskRequest {
                        task_id: task_id.into(),
                        expected_revision: revision,
                        command: StoreTaskCommand {
                            kind: TaskStateCommandKind::PauseTask,
                            item_ids: vec![],
                        },
                        event_id: format!("other-writer-{event_id}"),
                        occurred_at: "2026-10-26T00:00:00Z".into(),
                    })
                    .map_err(|_| ErrorClass::Internal)?;
                return Err(ErrorClass::Business);
            }
        }
        let kind = match action {
            "pause" => TaskStateCommandKind::PauseTask,
            "resume" => TaskStateCommandKind::ResumeTask,
            "cancel" => TaskStateCommandKind::CancelTask,
            _ => return Err(ErrorClass::Unsupported),
        };
        let task = self
            .store
            .lock()
            .unwrap()
            .update_task(&StoreUpdateTaskRequest {
                task_id: task_id.into(),
                expected_revision: revision,
                command: StoreTaskCommand {
                    kind,
                    item_ids: vec![],
                },
                event_id: event_id.into(),
                occurred_at: "2026-10-24T00:00:00Z".into(),
            })
            .map_err(|_| ErrorClass::Business)?;
        Ok(json!({"task_id":task_id,"revision":task.runtime.revision,"status":task.runtime.status}))
    }
    fn application_update_download_task(&self, _: &Value) -> Result<Value, ErrorClass> {
        self.submits.fetch_add(1, Ordering::SeqCst);
        Err(ErrorClass::Unsupported)
    }
    fn application_task_overview(&self) -> Result<Value, ErrorClass> {
        let tasks = self
            .store
            .lock()
            .unwrap()
            .core_restore(false)
            .unwrap()
            .tasks;
        Ok(
            json!({"queue": tasks.iter().map(|t| json!({"task_id":t.task_id,"revision":t.revision,"status":t.status})).collect::<Vec<_>>()}),
        )
    }
    fn application_account_session_statuses(&self) -> Result<Value, ErrorClass> {
        Ok(json!([]))
    }
    fn application_task_event_state(
        &self,
        task_id: &str,
        event_id: &str,
    ) -> Result<Option<Value>, ErrorClass> {
        let store = self.store.lock().unwrap();
        let landed = store
            .list_task_events(task_id)
            .map_err(|_| ErrorClass::Internal)?
            .iter()
            .any(|e| e.event_id == event_id);
        Ok(landed.then(|| json!({"task_id":task_id,"last_event_id":event_id})))
    }
    fn application_default_output_root(&self) -> Result<Option<String>, ErrorClass> {
        Ok(self.default_output.lock().unwrap().clone())
    }
    fn application_search_batch(&self, request_id: &str) -> Result<Option<String>, ErrorClass> {
        let mut batches = self.search_batches.lock().unwrap();
        Ok(if self.forget_search_batch.load(Ordering::SeqCst) {
            batches.remove(request_id)
        } else {
            batches.get(request_id).cloned()
        })
    }
    fn application_selection_accounts_ready(
        &self,
        _: &str,
        _: &[String],
    ) -> Result<bool, ErrorClass> {
        Ok(self.accounts_ready.load(Ordering::SeqCst))
    }
    fn application_extension_tools(&self) -> Vec<insar_agent_model::ToolDescription> {
        self.extension_tools.lock().unwrap().clone()
    }
    fn application_call_extension_tool(
        &self,
        tool_id: &str,
        input: &Value,
    ) -> Option<Result<Value, ErrorClass>> {
        let registered = self
            .extension_tools
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.name == tool_id);
        if !registered {
            return None;
        }
        self.extension_calls
            .lock()
            .unwrap()
            .push((tool_id.to_owned(), input.clone()));
        Some(Ok(json!({
            "ok": true,
            "tool_id": tool_id,
            "task_id": "task-ext-1",
            "message_zh": "扩展下载任务已创建"
        })))
    }
    fn application_admin_boundary_search(&self, input: &Value) -> Result<Value, ErrorClass> {
        let query = input.get("query").and_then(Value::as_str).unwrap_or("");
        let items = if query.contains("四川") || query == "510000" {
            json!([{
                "admin_code": "510000",
                "display_name": "四川省",
                "level": "province",
                "country_code": "CN",
                "bbox": [97.35, 26.05, 108.55, 34.32]
            }])
        } else if query.contains("西藏") || query == "540000" {
            json!([{
                "admin_code": "540000",
                "display_name": "西藏自治区",
                "level": "province",
                "country_code": "CN",
                "bbox": [78.39, 26.85, 99.12, 36.48]
            }])
        } else {
            json!([])
        };
        Ok(json!({"query": query, "items": items}))
    }
    fn application_preview_working_aoi_set(&self, input: &Value) -> Result<Value, ErrorClass> {
        if input.get("clear") == Some(&json!(true)) {
            return Ok(json!({"state": "none"}));
        }
        if let Some(code) = input.get("admin_code").and_then(Value::as_str) {
            let (name, [w, s, e, n]) = match code {
                "510000" => ("四川省", [97.35, 26.05, 108.55, 34.32]),
                "540000" => ("西藏自治区", [78.39, 26.85, 99.12, 36.48]),
                _ => return Err(ErrorClass::Business),
            };
            let label = input
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or(name);
            return Ok(json!({
                "state": "active",
                "source_kind": "china_admin",
                "display_name": label,
                "bbox": [w, s, e, n],
                "crs": "EPSG:4326",
                "geometry": {
                    "type": "MultiPolygon",
                    "coordinates": [[[[w, s], [e, s], [e, n], [w, n], [w, s]]]]
                }
            }));
        }
        if let Some(arr) = input.get("bbox").and_then(Value::as_array) {
            if arr.len() == 4 {
                let coords: Vec<f64> = arr.iter().filter_map(Value::as_f64).collect();
                if coords.len() == 4 {
                    let [w, s, e, n] = [coords[0], coords[1], coords[2], coords[3]];
                    let label = input
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or("自定义范围");
                    return Ok(json!({
                        "state": "active",
                        "source_kind": "drawn",
                        "display_name": label,
                        "bbox": [w, s, e, n],
                        "crs": "EPSG:4326",
                        "geometry": {
                            "type": "Polygon",
                            "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]
                        }
                    }));
                }
            }
        }
        Err(ErrorClass::InvalidParameters)
    }
    fn application_set_working_aoi(&self, input: &Value) -> Result<Value, ErrorClass> {
        let next = self.application_preview_working_aoi_set(input)?;
        *self.working_aoi.lock().unwrap() = next.clone();
        Ok(next)
    }
    fn application_restore_working_aoi(&self, snapshot: &Value) -> Result<Value, ErrorClass> {
        *self.working_aoi.lock().unwrap() = snapshot.clone();
        Ok(snapshot.clone())
    }
}

/// Recorded Chat Completions SSE turns, decoded by the production parser.
struct Replay {
    turns: Mutex<VecDeque<String>>,
    seen: Mutex<Vec<Vec<Message>>>,
    /// Tool names offered on each model call.
    tools_seen: Mutex<Vec<Vec<String>>>,
    /// Simulates a slow system credential read when a run opens its model.
    open_delay_ms: std::sync::atomic::AtomicU64,
    /// Runs once when the next model call returns its turn.
    on_complete: Mutex<Option<StoreDownHook>>,
    /// The model is a GLM route with reasoning off: its body carries the
    /// reasoning switch.
    reasoning_switch: std::sync::atomic::AtomicBool,
    /// The next call with the switch is refused (HTTP 400) before any turn.
    refuse_switch_once: std::sync::atomic::AtomicBool,
    /// The channel file's remembered refusal (W3o): set by
    /// `remember_reasoning_switch_unsupported`, read when the next run opens
    /// its model. The stand-in keeps no memory inside a run (that is the
    /// production model's, tested on `KeyedModel`).
    switch_unsupported: std::sync::atomic::AtomicBool,
    /// Every call that carries the switch is refused (HTTP 400).
    refuse_switch_always: std::sync::atomic::AtomicBool,
    /// How often the host asked the model to remember a refusal.
    remembered: AtomicUsize,
    /// The request body carries an envelope field the outbound check refuses
    /// (a body the host cannot repair).
    unsafe_envelope: std::sync::atomic::AtomicBool,
}
/// The first bool is true for the resend model, whose switch was removed;
/// the second is the channel's remembered refusal as read when the run
/// opened its model.
struct ReplayModel(Arc<Replay>, bool, bool);
impl ReplayModel {
    fn switch(&self) -> bool {
        self.0.reasoning_switch.load(Ordering::SeqCst)
    }
    fn unsupported(&self) -> bool {
        self.switch() && self.2
    }
}
impl Model for ReplayModel {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [ToolDescription],
        cancel: &'a CancellationToken,
        emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ModelError>> + Send + 'a>> {
        if self.switch()
            && !self.1
            && !self.unsupported()
            && (self.0.refuse_switch_always.load(Ordering::SeqCst)
                || self.0.refuse_switch_once.swap(false, Ordering::SeqCst))
        {
            return Box::pin(async {
                Err(ModelError::new(
                    insar_agent_model::ErrorKind::ReasoningSwitchRefused,
                    Some(400),
                ))
            });
        }
        self.0.seen.lock().unwrap().push(messages.to_vec());
        self.0
            .tools_seen
            .lock()
            .unwrap()
            .push(tools.iter().map(|tool| tool.name.clone()).collect());
        let frames = self.0.turns.lock().unwrap().pop_front();
        let hook = self.0.on_complete.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        Box::pin(async move {
            let frames = frames.expect("a recorded turn for every model call");
            replay(Protocol::ChatCompletions, &frames, cancel, |event| {
                emit(event)
            })
        })
    }
    /// Same production request builder a networked channel uses.
    fn request_body(
        &self,
        messages: &[Message],
        tools: &[ToolDescription],
    ) -> Option<Result<Value, ModelError>> {
        // As the production resend model: the same route, known to refuse
        // the switch (no switch, the larger budget).
        let (model, unsupported) = match (self.switch(), self.1) {
            (false, _) => ("replay-fixture", false),
            (true, false) => ("glm-replay", self.unsupported()),
            (true, true) => ("glm-replay", true),
        };
        let unsafe_envelope = self.0.unsafe_envelope.load(Ordering::SeqCst);
        Some(
            ModelRoute::new(Protocol::ChatCompletions, "http://127.0.0.1:9/v1", model)
                .and_then(|route| {
                    route
                        .with_reasoning(ReasoningMode::Off)
                        .with_reasoning_switch_unsupported(unsupported)
                        .request(messages, tools)
                })
                .map(|(_, mut body)| {
                    if unsafe_envelope {
                        body["metadata"] = json!({"note": "C:/fixture/envelope"});
                    }
                    body
                }),
        )
    }
    fn without_reasoning_switch(&self) -> Option<Box<dyn Model + Send + '_>> {
        (self.switch() && !self.1 && !self.unsupported()).then(|| {
            Box::new(ReplayModel(Arc::clone(&self.0), true, true)) as Box<dyn Model + Send>
        })
    }
    fn reasoning_switch_unsupported(&self) -> bool {
        self.unsupported()
    }
    fn remember_reasoning_switch_unsupported(&self) {
        self.0.remembered.fetch_add(1, Ordering::SeqCst);
        self.0.switch_unsupported.store(true, Ordering::SeqCst);
    }
}
struct Source(Arc<Replay>);
impl ModelSource for Source {
    fn open(&self) -> Result<OpenedModel, HostError> {
        std::thread::sleep(Duration::from_millis(
            self.0.open_delay_ms.load(Ordering::SeqCst),
        ));
        Ok(OpenedModel {
            model: Box::new(ReplayModel(
                Arc::clone(&self.0),
                false,
                self.0.switch_unsupported.load(Ordering::SeqCst),
            )),
            route: "chat_completions/replay-fixture".into(),
        })
    }
}
fn frame(delta: Value) -> String {
    format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"delta":delta}]})
    )
}
fn tool_turn(id: &str, name: &str, arguments: Value) -> String {
    frame(
        json!({"tool_calls":[{"index":0,"id":id,"function":{"name":name,"arguments":arguments.to_string()}}]}),
    )
}
fn text_turn(answer: Value) -> String {
    frame(json!({"content": answer.to_string()}))
}

struct Fixture {
    root: PathBuf,
    fake: Fake,
    replay: Arc<Replay>,
    host: Arc<AgentHost<Fake, Source>>,
    session: String,
}
fn fixture(label: &str, turns: Vec<String>) -> Fixture {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "agent-session-{}-{label}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let mut store = TaskStore::open(root.join("store.db")).unwrap();
    seed_task(&mut store, &root);
    let fake = Fake {
        store: Arc::new(Mutex::new(store)),
        submits: Arc::new(AtomicUsize::new(0)),
        store_down: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        on_store_down: Arc::new(Mutex::new(None)),
        before_store: Arc::new(Mutex::new(None)),
        create_ok: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        created: Arc::new(Mutex::new(vec![])),
        fail_cancel: Arc::new(Mutex::new(Default::default())),
        conflict_once: Arc::new(Mutex::new(Default::default())),
        default_output: Arc::new(Mutex::new(Some(
            root.join("default-out").to_string_lossy().into_owned(),
        ))),
        next_search_batch: Arc::new(Mutex::new(None)),
        search_batches: Arc::new(Mutex::new(Default::default())),
        forget_search_batch: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        last_search: Arc::new(Mutex::new(None)),
        accounts_ready: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        sources: Arc::new(Mutex::new(json!([]))),
        working_aoi: Arc::new(Mutex::new(json!({"state":"none"}))),
        searches: Arc::new(Mutex::new(vec![])),
        search_items_kept: Arc::new(AtomicUsize::new(3)),
        search_start_terminal: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        extension_tools: Arc::new(Mutex::new(Vec::new())),
        extension_calls: Arc::new(Mutex::new(Vec::new())),
    };
    let replay = Arc::new(Replay {
        turns: Mutex::new(turns.into()),
        seen: Mutex::new(vec![]),
        tools_seen: Mutex::new(vec![]),
        open_delay_ms: std::sync::atomic::AtomicU64::new(0),
        on_complete: Mutex::new(None),
        reasoning_switch: std::sync::atomic::AtomicBool::new(false),
        refuse_switch_once: std::sync::atomic::AtomicBool::new(false),
        switch_unsupported: std::sync::atomic::AtomicBool::new(false),
        refuse_switch_always: std::sync::atomic::AtomicBool::new(false),
        remembered: AtomicUsize::new(0),
        unsafe_envelope: std::sync::atomic::AtomicBool::new(false),
    });
    let host = AgentHost::open(root.clone(), fake.clone(), Source(Arc::clone(&replay))).unwrap();
    let session = host
        .session_create(Some("暂停任务".into()))
        .unwrap()
        .session_id;
    Fixture {
        root,
        fake,
        replay,
        host,
        session,
    }
}
fn seed_task(store: &mut TaskStore, root: &Path) {
    let item = SnapshotItem {
        item_id: "ITEM-A".into(),
        provider_id: "asf.sentinel1".into(),
        display_name: "A.zip".into(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Default::default(),
        payload: json!({"file_name":"A.zip"}).as_object().unwrap().clone(),
        asset_access: None,
        execution_policy: None,
    };
    store.save_search_batch(&StoreSearchBatchRequest{activate_workbench:true,batch:FrozenSearchBatch{schema_version:1,batch_id:"batch-a".into(),provider_id:"asf.sentinel1".into(),created_at:"2026-10-23T09:00:00Z".into(),aoi_name:"fixture".into(),aoi_geojson:Some(json!({"type":"Polygon","coordinates":[[[100,30],[101,30],[101,31],[100,31],[100,30]]]}).as_object().unwrap().clone()),query:Default::default(),search_stats:None,candidates:vec![item]},workbench:SearchWorkbenchState{batch_id:"batch-a".into(),checked_item_ids:vec!["ITEM-A".into()],page_index:0,page_size:24,revision:0}}).unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-a".into(),
            batch_id: "batch-a".into(),
            selected_item_ids: vec!["ITEM-A".into()],
            checked_item_ids: vec!["ITEM-A".into()],
            page_index: 0,
            page_size: 24,
            output_root: root.join("out"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-23T09:00:01Z".into(),
        })
        .unwrap();
}
impl Fixture {
    /// Fault injection through a second connection to the same Store file
    /// (e.g. a trigger that makes one table's writes fail).
    fn sql(&self, statement: &str) {
        rusqlite::Connection::open(self.root.join("store.db"))
            .unwrap()
            .execute_batch(statement)
            .unwrap();
    }
    fn revision(&self) -> u64 {
        self.fake
            .store
            .lock()
            .unwrap()
            .load_task_record("task-a")
            .unwrap()
            .unwrap()
            .runtime
            .revision
    }
    fn wait_for(&self, kind: &str) -> StreamEvent {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let page = self.host.stream_poll(&self.session, 0).unwrap();
            if let Some(event) = page.events.into_iter().find(|e| e.kind == kind) {
                return event;
            }
            assert!(Instant::now() < deadline, "no {kind} event");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn plan_status(&self, plan_id: &str) -> String {
        self.fake
            .store
            .lock()
            .unwrap()
            .agent_plan(plan_id)
            .unwrap()
            .unwrap()
            .status
    }
    fn run_status(&self, run_id: &str) -> String {
        self.fake
            .store
            .lock()
            .unwrap()
            .agent_run(run_id)
            .unwrap()
            .unwrap()
            .status
    }
}
fn pause_plan(revision: u64) -> Value {
    json!({"steps":[{"tool_id":"task_action","input":{"task_id":"task-a","revision":revision,"action":"pause","event_id":"model-chosen"},"label_zh":"暂停任务"}]})
}
fn answer() -> Value {
    json!({"text":"已暂停任务","claims":[
        {"statement":"task-a 已暂停","refs":["task:task-a"]},
        {"statement":"网上的说法","refs":["web:https://example.org/x"]},
        {"statement":"没有引用的说法","refs":[]}
    ]})
}

#[test]
fn approved_plan_executes_once_and_conclusion_is_cited_and_sealed() {
    let f = fixture("full", vec![]);
    let revision = f.revision();
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "task_overview", json!({})),
        tool_turn("c2", "propose_plan", pause_plan(revision)),
        text_turn(answer()),
    ]);
    let run_id = f
        .host
        .send(&f.session, "请暂停 task-a", SendContext::default())
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    let plan_card = f.wait_for("plan_card").payload;
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    assert_eq!(plan_card["plan_hash"], card["plan_hash"]);
    assert_eq!(plan_card["max_tier"], 2);
    assert_eq!(plan_card["steps"][0]["tool_id"], "task_action");
    assert_eq!(card["snapshot"]["source"], "asf.sentinel1");
    assert_eq!(card["snapshot"]["range"]["aoi_name"], "fixture");
    assert!(card["snapshot"]["range"]["area_km2"].as_f64().unwrap() > 9000.0);
    // A task control card: no search result and no direction in the frozen task.
    assert_eq!(card["snapshot"]["orbit_directions"], json!([]));
    assert!(card["snapshot"]["result_count"].is_null());
    assert!(card["snapshot"]["already_queued_count"].is_null());
    // Not approved: nothing reached the service and no approval row exists.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    assert_eq!(f.plan_status(&plan_id), "proposed");
    assert!(f
        .fake
        .store
        .lock()
        .unwrap()
        .agent_approval(&plan_id)
        .unwrap()
        .is_none());

    f.host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .unwrap();
    let conclusion = f.wait_for("conclusion").payload;
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 1);
    assert_eq!(conclusion["text"], "已暂停任务");
    let claims = conclusion["claims"].as_array().unwrap();
    assert_eq!(claims[0]["verified"], true);
    assert_eq!(claims[0]["refs"][0]["kind"], "task");
    assert_eq!(claims[0]["refs"][0]["id"], "task-a");
    assert_eq!(claims[1]["verified"], false, "web claims are unverified");
    assert_eq!(claims[1]["refs"][0]["kind"], "web");
    assert_eq!(
        claims[2]["verified"], false,
        "a claim without refs is unverified"
    );

    let store = f.fake.store.lock().unwrap();
    let approval = store.agent_approval(&plan_id).unwrap().unwrap();
    assert_eq!(approval.decided_via, "desktop_card");
    assert_eq!(
        serde_json::from_str::<Value>(&approval.snapshot_json).unwrap(),
        card["snapshot"]
    );
    assert_eq!(
        store.agent_plan(&plan_id).unwrap().unwrap().status,
        "approved"
    );
    let run = store.agent_run(&run_id).unwrap().unwrap();
    assert_eq!(run.status, "completed");
    assert!(run.trace_sha256.is_some());
    let events = store.list_task_events("task-a").unwrap();
    assert!(
        events.iter().any(|e| e.event_id == format!("{plan_id}-0")),
        "the host mints the event id, not the model"
    );
    assert!(!events.iter().any(|e| e.event_id == "model-chosen"));
    drop(store);
    assert!(f
        .root
        .join(format!("agent/runs/{run_id}/journal.md"))
        .is_file());
    let detail = f.host.run_open(&run_id).unwrap();
    let task_step = detail
        .events
        .iter()
        .rev()
        .find(|e| e.kind == "tool_result" && e.payload["summary"]["task_id"] == "task-a")
        .expect("the task control result is in the run record")
        .sequence;
    assert_eq!(claims[0]["refs"][0]["trace_sequence"], task_step);
    assert!(claims[0]["refs"][0]["response_count"].is_null());
    for kind in [
        "request",
        "tool_result",
        "plan",
        "approval",
        "execution",
        "answer",
    ] {
        assert!(
            detail.events.iter().any(|e| e.kind == kind),
            "trace lacks {kind}"
        );
    }
    assert_eq!(f.host.runs_list(&f.session).unwrap()[0].status, "completed");
    // The model saw the execution result as a tool message before answering.
    let seen = f.replay.seen.lock().unwrap();
    let last = seen.last().unwrap().last().unwrap();
    assert_eq!(last.role, "tool");
    assert!(last.content.contains("\"done\""));
    let page = f.host.stream_poll(&f.session, 0).unwrap();
    assert!(f
        .host
        .stream_poll(&f.session, page.cursor)
        .unwrap()
        .events
        .is_empty());
}

#[test]
fn a_different_card_or_changed_store_revision_never_executes() {
    let f = fixture("mismatch", vec![]);
    let revision = f.revision();
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "propose_plan", pause_plan(revision)),
        text_turn(json!({"text":"计划已失效","claims":[]})),
    ]);
    f.host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    let mut forged = card["snapshot"].clone();
    forged["output_dir"] = json!("elsewhere");
    assert!(matches!(
        f.host.plan_approve(&plan_id, forged, None),
        Err(HostError::Expired)
    ));
    f.wait_for("conclusion");
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    assert_eq!(f.plan_status(&plan_id), "expired");

    // Store revision changes between the card and the approval.
    let g = fixture("revision", vec![]);
    let revision = g.revision();
    g.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "propose_plan", pause_plan(revision)),
        text_turn(json!({"text":"计划已失效","claims":[]})),
    ]);
    g.host
        .send(&g.session, "暂停", SendContext::default())
        .unwrap();
    let card = g.wait_for("confirm_card").payload;
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    g.fake
        .application_task_action("task-a", revision, "pause", "outside-writer")
        .unwrap();
    assert!(matches!(
        g.host
            .plan_approve(&plan_id, card["snapshot"].clone(), None),
        Err(HostError::Expired)
    ));
    g.wait_for("conclusion");
    assert_eq!(
        g.fake.submits.load(Ordering::SeqCst),
        1,
        "only the outside writer"
    );
    assert_eq!(g.plan_status(&plan_id), "expired");
}

#[test]
fn store_revision_change_after_approval_expires_the_remaining_steps() {
    let f = fixture("after", vec![]);
    let revision = f.revision();
    // Step 0 changes the task revision; step 1 still names the approved one.
    let plan = json!({"steps":[
        {"tool_id":"task_action","input":{"task_id":"task-a","revision":revision,"action":"pause","event_id":"x"}},
        {"tool_id":"task_action","input":{"task_id":"task-a","revision":revision,"action":"resume","event_id":"y"}}
    ]});
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "propose_plan", plan),
        text_turn(json!({"text":"第二步已失效","claims":[]})),
    ]);
    f.host
        .send(&f.session, "暂停再继续", SendContext::default())
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    f.host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .unwrap();
    f.wait_for("conclusion");
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 1);
    assert_eq!(f.plan_status(&plan_id), "expired");
    let events = f.host.stream_poll(&f.session, 0).unwrap().events;
    assert!(events.iter().any(|e| e.kind == "progress_card"
        && e.payload["step_index"] == 1
        && e.payload["status"] == "expired"));
    assert!(events
        .iter()
        .any(|e| e.kind == "error" && e.payload["code"] == "plan_expired"));
}

#[test]
fn cancel_while_waiting_for_the_card_cancels_the_run_without_executing() {
    let f = fixture("cancel", vec![]);
    let revision = f.revision();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(tool_turn("c1", "propose_plan", pause_plan(revision)));
    let run_id = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    assert!(f.host.run_cancel(&run_id).unwrap());
    let error = f.wait_for("error").payload;
    assert_eq!(error["code"], "cancelled");
    // A user cancel is `cancelled`; `interrupted` is reserved for runs a
    // crash or restart stopped.
    assert_eq!(error["status"], "cancelled");
    assert_eq!(f.run_status(&run_id), "cancelled");
    assert_eq!(f.host.runs_list(&f.session).unwrap()[0].status, "cancelled");
    assert_eq!(f.plan_status(&plan_id), "expired");
    assert!(matches!(
        f.host
            .plan_approve(&plan_id, card["snapshot"].clone(), None),
        Err(HostError::NotFound)
    ));
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
}

#[test]
fn cancel_seals_the_run_even_when_the_plan_cannot_be_expired() {
    let f = fixture("cancel-expire-fails", vec![]);
    let revision = f.revision();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(tool_turn("c1", "propose_plan", pause_plan(revision)));
    let run_id = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    f.wait_for("confirm_card");
    f.sql(
        "CREATE TRIGGER inject_plan_fail BEFORE UPDATE ON agent_plans \
         BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    assert!(f.host.run_cancel(&run_id).unwrap());
    let error = f.wait_for("error").payload;
    assert_eq!(error["code"], "cancelled");
    assert_eq!(f.run_status(&run_id), "cancelled");
    // No running row and no busy session are left behind.
    f.sql("DROP TRIGGER inject_plan_fail;");
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text":"好","claims":[]})));
    assert!(f
        .host
        .send(&f.session, "再来", SendContext::default())
        .is_ok());
}

#[test]
fn deleting_a_session_keeps_tasks_and_leaves_a_content_free_marker() {
    let f = fixture("delete", vec![]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "task_overview", json!({})),
        text_turn(json!({"text":"队列里有一个任务","claims":[{"statement":"有任务","refs":["task:task-a"]}]})),
    ]);
    let run_id = f
        .host
        .send(&f.session, "我的私人问题", SendContext::default())
        .unwrap();
    assert_eq!(
        f.wait_for("conclusion").payload["claims"][0]["verified"],
        true
    );
    assert_eq!(f.host.sessions_list().unwrap().len(), 1);
    f.host.session_delete(&f.session).unwrap();
    assert!(f.host.sessions_list().unwrap().is_empty());
    assert!(matches!(
        f.host.stream_poll(&f.session, 0),
        Err(HostError::NotFound)
    ));
    let marker =
        fs::read_to_string(f.root.join(format!("agent/sessions/{}.deleted", f.session))).unwrap();
    let marker: Value = serde_json::from_str(&marker).unwrap();
    assert_eq!(marker.as_object().unwrap().len(), 2);
    assert!(!marker.to_string().contains("私人"));
    assert!(!f.root.join(format!("agent/runs/{run_id}")).exists());
    let store = f.fake.store.lock().unwrap();
    assert!(store.agent_run(&run_id).unwrap().is_none());
    assert!(store.load_task_record("task-a").unwrap().is_some());
}

#[test]
fn deleting_a_session_removes_its_plans_and_approvals_but_not_tasks_files_or_batches() {
    let f = fixture("delete-plans", vec![]);
    let revision = f.revision();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(tool_turn("c1", "propose_plan", pause_plan(revision)));
    let run_id = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let plan_id = f.wait_for("confirm_card").payload["plan_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // A plan of this session whose run is not listed in the session summary
    // (e.g. the summary write failed after the plan landed) must also go.
    let orphan = {
        let mut store = f.fake.store.lock().unwrap();
        let row = insar_agent::plan::build_plan(
            insar_store::AgentPlanRow {
                plan_id: "plan-orphan".into(),
                session_id: f.session.clone(),
                run_id: "run-orphan".into(),
                created_at: "2026-10-24T00:00:00Z".into(),
                model_route: "fixture".into(),
                plan_json: String::new(),
                plan_hash: String::new(),
                max_tier: 2,
                status: "proposed".into(),
                expires_at: "2999-01-01T00:00:00Z".into(),
                store_revision_seen: 0,
            },
            &json!({"steps":[{"tool_id":"task_page","input":{"task_id":"task-a"}}],"card_snapshot":{"output_dir":"D:/私人/输出","aoi_name":"私人区域"}}),
        )
        .unwrap();
        store.save_agent_plan(&row).unwrap();
        store
            .decide_agent_plan(
                &insar_store::AgentApprovalRow {
                    approval_id: "approval-orphan".into(),
                    plan_id: row.plan_id.clone(),
                    plan_hash_seen: row.plan_hash.clone(),
                    snapshot_json: "{\"output_dir\":\"D:/私人/输出\"}".into(),
                    decision: "approved".into(),
                    decided_at: "2026-10-24T00:00:01Z".into(),
                    decided_via: "desktop_card".into(),
                    mode: None,
                    rewritten_from_plan_hash: None,
                },
                &row.plan_hash,
                0,
                "2026-10-24T00:00:01Z",
            )
            .unwrap();
        assert!(store.agent_approval("plan-orphan").unwrap().is_some());
        row.plan_id
    };
    let kept = f.root.join("out").join("keep.bin");
    fs::create_dir_all(kept.parent().unwrap()).unwrap();
    fs::write(&kept, b"downloaded").unwrap();

    f.host.session_delete(&f.session).unwrap();

    let store = f.fake.store.lock().unwrap();
    assert!(store.agent_run(&run_id).unwrap().is_none());
    for plan in [&plan_id, &orphan] {
        assert!(
            store.agent_plan(plan).unwrap().is_none(),
            "plan {plan} left"
        );
        assert!(
            store.agent_approval(plan).unwrap().is_none(),
            "approval of {plan} left"
        );
    }
    assert!(store.load_task_record("task-a").unwrap().is_some());
    assert!(store.load_search_batch("batch-a").unwrap().is_some());
    assert_eq!(fs::read(&kept).unwrap(), b"downloaded");
    let marker: Value = serde_json::from_str(
        &fs::read_to_string(f.root.join(format!("agent/sessions/{}.deleted", f.session))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        marker
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["deleted_at", "session_id"]
    );
}

#[test]
fn deleting_a_session_removes_runs_its_summary_does_not_list() {
    let f = fixture("delete-unlisted", vec![]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "task_overview", json!({})),
        text_turn(json!({"text":"有任务","claims":[]})),
    ]);
    let run_id = f
        .host
        .send(&f.session, "私人问题", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    // The summary lost the run id (e.g. its write failed after the run row
    // landed): the Store's session index still finds the run.
    let path = f
        .root
        .join(format!("agent/sessions/{}/session.json", f.session));
    let mut summary: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    summary["run_ids"] = json!([]);
    fs::write(&path, serde_json::to_vec(&summary).unwrap()).unwrap();
    f.host.session_delete(&f.session).unwrap();
    assert!(f
        .fake
        .store
        .lock()
        .unwrap()
        .agent_run(&run_id)
        .unwrap()
        .is_none());
    assert!(!f.root.join(format!("agent/runs/{run_id}")).exists());
}

#[test]
fn a_plan_proposed_while_its_session_is_deleted_is_not_saved() {
    let f = fixture("delete-during-propose", vec![]);
    let revision = f.revision();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(tool_turn("c1", "propose_plan", pause_plan(revision)));
    // After the model returns propose_plan, the session is deleted at the
    // first Store access of the proposal, before the plan would be saved.
    let host = Arc::downgrade(&f.host);
    let before = Arc::clone(&f.fake.before_store);
    let session = f.session.clone();
    *f.replay.on_complete.lock().unwrap() = Some(Box::new(move || {
        *before.lock().unwrap() = Some(Box::new(move || {
            host.upgrade().unwrap().session_delete(&session).unwrap();
        }));
    }));
    f.host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let marker = f.root.join(format!("agent/sessions/{}.deleted", f.session));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "session not deleted");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(300));
    let plans: i64 = rusqlite::Connection::open(f.root.join("store.db"))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM agent_plans WHERE session_id=?1",
            [&f.session],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(plans, 0, "a plan outlived its deleted session");
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
}

#[test]
fn every_model_call_archives_its_scrubbed_request_body_on_the_trace_chain() {
    use sha2::Digest;
    let f = fixture("outbound", vec![]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "task_overview", json!({})),
        text_turn(json!({"text":"队列里有一个任务","claims":[]})),
    ]);
    let run_id = f
        .host
        .send(
            &f.session,
            "看看 C:\\Users\\someone\\private.tif 和 password=not-a-real-secret",
            SendContext::default(),
        )
        .unwrap();
    f.wait_for("conclusion");
    let detail = f.host.run_open(&run_id).unwrap();
    let calls: Vec<_> = detail
        .events
        .iter()
        .filter(|event| event.kind == "model_call")
        .collect();
    assert_eq!(calls.len(), 2, "one model_call event per model call");
    let dir = f.root.join(format!("agent/runs/{run_id}/outbound"));
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files.len(), 2, "one archived body per model call");
    for (call, file) in calls.iter().zip(&files) {
        assert_eq!(call.payload["file"], file.as_str());
        let bytes = fs::read(dir.join(file)).unwrap();
        assert_eq!(
            call.payload["sha256"],
            format!("{:x}", sha2::Sha256::digest(&bytes)),
            "the event hashes the archived bytes"
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.contains("\"messages\""),
            "the archive is the request body"
        );
        assert!(!text.contains("private.tif"));
        assert!(!text.contains("not-a-real-secret"));
    }
    let second = fs::read_to_string(dir.join(&files[1])).unwrap();
    assert!(
        second.contains("\"role\":\"tool\""),
        "the second body carries the tool result"
    );
}

#[test]
fn two_concurrent_sends_in_one_session_start_exactly_one_run() {
    let f = fixture("busy", vec![]);
    f.replay.turns.lock().unwrap().extend([
        text_turn(json!({"text":"一","claims":[]})),
        text_turn(json!({"text":"二","claims":[]})),
    ]);
    f.replay.open_delay_ms.store(300, Ordering::SeqCst);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let host = Arc::clone(&f.host);
            let session = f.session.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                host.send(&session, "并发", SendContext::default())
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(HostError::Busy)))
            .count(),
        1
    );
    f.wait_for("conclusion");
    assert_eq!(f.host.runs_list(&f.session).unwrap().len(), 1);
    // The placeholder is released once the run ends: a later send works.
    f.replay.open_delay_ms.store(0, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match f.host.send(&f.session, "再来", SendContext::default()) {
            Ok(_) => break,
            Err(HostError::Busy) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => panic!("{error:?}"),
        }
    }
}

#[test]
fn a_store_failure_during_approval_keeps_the_run_parked_and_approvable() {
    let f = fixture("decide-store", vec![]);
    let revision = f.revision();
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "propose_plan", pause_plan(revision)),
        text_turn(answer()),
    ]);
    let run_id = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    f.fake.store_down.store(true, Ordering::SeqCst);
    assert!(matches!(
        f.host
            .plan_approve(&plan_id, card["snapshot"].clone(), None),
        Err(HostError::StoreUnavailable)
    ));
    f.fake.store_down.store(false, Ordering::SeqCst);
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    assert_eq!(f.plan_status(&plan_id), "proposed");
    assert_eq!(f.run_status(&run_id), "running");
    // The parked run was not dropped: the same card can still be approved.
    f.host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .unwrap();
    f.wait_for("conclusion");
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 1);
    assert_eq!(f.run_status(&run_id), "completed");
}

#[test]
fn a_store_write_error_while_deciding_is_retryable_not_an_expiry() {
    let f = fixture("decide-write", vec![]);
    let revision = f.revision();
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "propose_plan", pause_plan(revision)),
        text_turn(answer()),
    ]);
    let run_id = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    f.sql(
        "CREATE TRIGGER inject_approval_fail BEFORE INSERT ON agent_approvals \
         BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    let error = f
        .host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .unwrap_err();
    assert!(matches!(error, HostError::Store(_)), "{error:?}");
    assert_eq!(error.code(), "AGENT_STORE_UNAVAILABLE");
    std::thread::sleep(Duration::from_millis(100));
    let events = f.host.stream_poll(&f.session, 0).unwrap().events;
    assert!(
        !events
            .iter()
            .any(|e| e.kind == "error" || e.kind == "progress_card"),
        "{events:?}"
    );
    assert_eq!(f.plan_status(&plan_id), "proposed");
    assert_eq!(f.run_status(&run_id), "running");
    f.sql("DROP TRIGGER inject_approval_fail;");
    f.host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .unwrap();
    f.wait_for("conclusion");
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 1);
    assert_eq!(f.run_status(&run_id), "completed");
}

#[test]
fn a_cancel_during_a_failed_decision_seals_the_run_and_says_so() {
    let f = fixture("decide-cancel", vec![]);
    let revision = f.revision();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(tool_turn("c1", "propose_plan", pause_plan(revision)));
    let run_id = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    // The user cancels while the decision holds the parked run; the Store
    // comes back right after the decision failed.
    let host = Arc::downgrade(&f.host);
    let down = Arc::clone(&f.fake.store_down);
    let cancelled = run_id.clone();
    *f.fake.on_store_down.lock().unwrap() = Some(Box::new(move || {
        assert!(host.upgrade().unwrap().run_cancel(&cancelled).unwrap());
        down.store(false, Ordering::SeqCst);
    }));
    f.fake.store_down.store(true, Ordering::SeqCst);
    assert!(f
        .host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .is_err());
    assert_eq!(f.run_status(&run_id), "cancelled");
    let error = f.wait_for("error").payload;
    assert_eq!(error["code"], "cancelled");
    assert_eq!(error["status"], "cancelled");
    assert_eq!(error["run_id"], run_id.as_str());
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
}

#[test]
fn a_restart_marks_a_waiting_run_interrupted_not_cancelled() {
    let f = fixture("restart", vec![]);
    let revision = f.revision();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(tool_turn("c1", "propose_plan", pause_plan(revision)));
    let run_id = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    f.wait_for("confirm_card");
    assert_eq!(f.run_status(&run_id), "running");
    // A new process opens the host over the same Store and data root.
    let _reopened = AgentHost::open(
        f.root.clone(),
        f.fake.clone(),
        Source(Arc::clone(&f.replay)),
    )
    .unwrap();
    assert_eq!(f.run_status(&run_id), "interrupted");
}

#[test]
fn the_model_sees_an_aoi_summary_and_reference_never_the_geometry() {
    let f = fixture("aoi", vec![]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "c1",
            "propose_plan",
            json!({"steps":[{"tool_id":"search_start","input":{
                "resource_id":"science.sentinel-1.slc.v1","aoi":{"ref":"context_aoi"},
                "aoi_name":"我的区域","time_range":null,"filters":{}}}]}),
        ),
        text_turn(json!({"text":"已提出检索","claims":[]})),
    ]);
    let polygon = json!({"type":"Polygon","coordinates":[[[100.123457,30.5],[101.5,30.5],[101.5,31.25],[100.123457,31.25],[100.123457,30.5]]]});
    f.host
        .send(
            &f.session,
            "在这个区域找数据",
            SendContext {
                aoi: Some(json!({"name":"我的区域","geometry":polygon})),
                time_range: None,
                source: None,
                model: None,
                ..SendContext::default()
            },
        )
        .unwrap();
    let card = f.wait_for("plan_card").payload;
    let user = f.replay.seen.lock().unwrap()[0][1].content.clone();
    assert!(!user.contains("100.123457"), "{user}");
    assert!(!user.contains("coordinates"), "{user}");
    assert!(user.contains("context_aoi"), "{user}");
    assert!(user.contains("我的区域"), "{user}");
    assert!(user.contains("bbox") && user.contains("area_km2"), "{user}");
    // The frozen local plan resolves the reference to the actual geometry.
    let plan = f
        .fake
        .store
        .lock()
        .unwrap()
        .agent_plan(card["plan_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    let intent: Value = serde_json::from_str(&plan.plan_json).unwrap();
    assert_eq!(intent["steps"][0]["input"]["aoi"], polygon);
}

#[test]
fn an_oversized_send_context_is_rejected() {
    let f = fixture("big-context", vec![]);
    let big = json!({"name":"大","geometry":{"type":"Polygon","coordinates":[(0..40_000).map(|i| json!([i as f64 * 1e-4, 1.0])).collect::<Vec<_>>()]}});
    assert!(matches!(
        f.host.send(
            &f.session,
            "大区域",
            SendContext {
                aoi: Some(big),
                time_range: None,
                source: None,
                model: None,
                ..SendContext::default()
            },
        ),
        Err(HostError::Invalid)
    ));
    assert!(f.host.runs_list(&f.session).unwrap().is_empty());
}

/// Minimal JSON-schema walk over the v3 contract: `$ref` (+ x-insar-nullable),
/// enum, type, required, additionalProperties=false and array items.
fn conforms(schema: &Value, node: &Value, value: &Value, path: &str) {
    if let Some(name) = node.get("$ref").and_then(Value::as_str) {
        if value.is_null() && node.get("x-insar-nullable") == Some(&json!(true)) {
            return;
        }
        let def = name.trim_start_matches("#/$defs/");
        let target = &schema["$defs"][def];
        assert!(target.is_object(), "{path}: missing contract def {def}");
        return conforms(schema, target, value, path);
    }
    if let Some(choices) = node.get("enum").and_then(Value::as_array) {
        assert!(
            choices.contains(value),
            "{path}: {value} not in {choices:?}"
        );
        return;
    }
    let types: Vec<&str> = match node.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    let actual = match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_u64() || n.is_i64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    assert!(
        types.is_empty()
            || types.contains(&actual)
            || (actual == "integer" && types.contains(&"number")),
        "{path}: {actual} is not {types:?}"
    );
    if let (Value::Object(map), Some(props)) = (value, node.get("properties")) {
        for key in node["required"].as_array().into_iter().flatten() {
            let key = key.as_str().unwrap();
            assert!(map.contains_key(key), "{path}: missing {key}");
        }
        for (key, entry) in map {
            match props.get(key) {
                Some(child) => conforms(schema, child, entry, &format!("{path}.{key}")),
                None => assert!(
                    node.get("additionalProperties") != Some(&json!(false)),
                    "{path}: unexpected field {key}"
                ),
            }
        }
    }
    if let (Value::Array(items), Some(item)) = (value, node.get("items")) {
        for (index, entry) in items.iter().enumerate() {
            conforms(schema, item, entry, &format!("{path}[{index}]"));
        }
    }
}
fn check(def: &str, value: &Value) {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../contracts/v3/provider-contract.schema.json"
    ))
    .unwrap();
    conforms(
        &schema,
        &json!({"$ref": format!("#/$defs/{def}")}),
        value,
        def,
    );
}

#[test]
fn stream_cards_and_command_payloads_match_the_v3_contract() {
    let f = fixture("contract", vec![]);
    let revision = f.revision();
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "task_overview", json!({})),
        tool_turn("c2", "propose_plan", pause_plan(revision)),
        text_turn(answer()),
    ]);
    let run_id = f
        .host
        .send(
            &f.session,
            "请暂停",
            SendContext {
                aoi: None,
                time_range: Some(json!({"start":"2026-01-01","end":null})),
                source: Some("science.sentinel-1.slc.v1".into()),
                model: None,
                ..SendContext::default()
            },
        )
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    f.host
        .plan_approve(
            card["plan_id"].as_str().unwrap(),
            card["snapshot"].clone(),
            None,
        )
        .unwrap();
    f.wait_for("conclusion");
    // A second run that is cancelled at the card produces an error event.
    f.replay.turns.lock().unwrap().push_back(tool_turn(
        "c3",
        "propose_plan",
        pause_plan(f.revision()),
    ));
    let second = f
        .host
        .send(&f.session, "再暂停", SendContext::default())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while f
        .host
        .stream_poll(&f.session, 0)
        .unwrap()
        .events
        .iter()
        .filter(|e| e.kind == "confirm_card")
        .count()
        < 2
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    f.host.run_cancel(&second).unwrap();
    f.wait_for("error");

    let page = f.host.stream_poll(&f.session, 0).unwrap();
    check("AgentStreamPage", &serde_json::to_value(&page).unwrap());
    let mut kinds = std::collections::BTreeSet::new();
    for event in &page.events {
        let def = match event.kind.as_str() {
            "message" => "AgentMessagePayload",
            "tool_round" => "AgentToolRoundPayload",
            "plan_card" => "AgentPlanCard",
            "confirm_card" => "AgentConfirmCard",
            "progress_card" => "AgentProgressCard",
            "conclusion" => "AgentConclusion",
            "error" => "AgentErrorPayload",
            "stop_all" => "AgentStopAllResult",
            "activity_changed" => "AgentActivityStatus",
            "thinking" => "AgentThinkingPayload",
            "question_card" => "AgentQuestionCardPayload",
            "workspace_sync" => "AgentWorkspaceSyncPayload",
            other => panic!("unexpected event kind {other}"),
        };
        check(def, &event.payload);
        kinds.insert(event.kind.clone());
    }
    assert_eq!(kinds.len(), 8, "every event kind was exercised: {kinds:?}");
    for session in f.host.sessions_list().unwrap() {
        check(
            "AgentSessionSummary",
            &serde_json::to_value(session).unwrap(),
        );
    }
    for run in f.host.runs_list(&f.session).unwrap() {
        check("AgentRunSummary", &serde_json::to_value(run).unwrap());
    }
    check(
        "AgentRunDetail",
        &serde_json::to_value(f.host.run_open(&run_id).unwrap()).unwrap(),
    );
    check(
        "AgentSendContext",
        &serde_json::to_value(SendContext::default()).unwrap(),
    );
}

fn product(item_id: &str, metadata: Value) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.into(),
        provider_id: "asf.sentinel1".into(),
        display_name: format!("{item_id}.zip"),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: metadata.as_object().unwrap().clone(),
        payload: json!({"file_name": format!("{item_id}.zip")})
            .as_object()
            .unwrap()
            .clone(),
        asset_access: None,
        execution_policy: None,
    }
}
fn sar_products() -> Vec<SnapshotItem> {
    vec![
        product("B1", json!({"flight_direction": "ASCENDING"})),
        product("B2", json!({"flight_direction": "DESCENDING"})),
        product("B3", json!({})),
    ]
}
/// A frozen batch named "西宁北"; `queued` products go into one live task.
fn seed_batch(
    f: &Fixture,
    batch_id: &str,
    query: Value,
    items: Vec<SnapshotItem>,
    queued: &[&str],
) {
    let mut store = f.fake.store.lock().unwrap();
    let ids: Vec<String> = items.iter().map(|item| item.item_id.clone()).collect();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.into(),
                provider_id: "asf.sentinel1".into(),
                created_at: "2026-10-24T09:00:00Z".into(),
                aoi_name: "西宁北".into(),
                aoi_geojson: None,
                query: query.as_object().unwrap().clone(),
                search_stats: None,
                candidates: items,
            },
            workbench: SearchWorkbenchState {
                batch_id: batch_id.into(),
                checked_item_ids: ids,
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    if !queued.is_empty() {
        let queued: Vec<String> = queued.iter().map(|id| (*id).to_owned()).collect();
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: format!("task-{batch_id}"),
                batch_id: batch_id.into(),
                selected_item_ids: queued.clone(),
                checked_item_ids: queued,
                page_index: 0,
                page_size: 24,
                output_root: f.root.join("out-queued"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-10-24T09:00:01Z".into(),
            })
            .unwrap();
    }
}
fn tool_rounds(f: &Fixture) -> Vec<Value> {
    f.host
        .stream_poll(&f.session, 0)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "tool_round")
        .map(|event| event.payload)
        .collect()
}
fn is_rfc3339_utc(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[10] == b'T'
        && bytes[19] == b'Z'
        && text[..4].bytes().all(|b| b.is_ascii_digit())
}

#[test]
fn tool_rounds_report_start_time_duration_and_search_result_count() {
    let f = fixture("tool-timing", vec![]);
    seed_batch(&f, "batch-b", json!({}), sar_products(), &[]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "search_progress", json!({"request_id": "req-1"})),
        tool_turn(
            "c2",
            "batch_page",
            json!({"batch_id": "batch-b", "revision": 0, "page": 0, "page_size": 2}),
        ),
        tool_turn("c3", "task_overview", json!({})),
        text_turn(json!({"text": "找到 3 景", "claims": []})),
    ]);
    f.host
        .send(&f.session, "检索结果", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let rounds = tool_rounds(&f);
    let round = |call: &str, status: &str| {
        rounds
            .iter()
            .find(|r| r["call_id"] == call && r["status"] == status)
            .cloned()
            .unwrap_or_else(|| panic!("no {status} round for {call}: {rounds:?}"))
    };
    for call in ["c1", "c2", "c3"] {
        let running = round(call, "running");
        let done = round(call, "done");
        let started = running["started_at"]
            .as_str()
            .unwrap_or_else(|| panic!("{call}: running round lacks started_at: {running}"));
        assert!(is_rfc3339_utc(started), "{call}: {started}");
        assert_eq!(done["started_at"], running["started_at"], "{call}");
        assert!(running["duration_ms"].is_null(), "{call}: {running}");
        assert!(done["duration_ms"].as_u64().is_some(), "{call}: {done}");
        assert!(running["result_count"].is_null(), "{call}: {running}");
    }
    assert_eq!(
        round("c1", "done")["result_count"],
        3,
        "items the search kept"
    );
    assert_eq!(round("c1", "done")["summary_zh"], "检索完成，保留 3 景");
    assert_eq!(
        round("c2", "done")["result_count"],
        2,
        "items this page returned"
    );
    assert!(round("c2", "done")["summary_zh"].is_null());
    assert!(
        round("c3", "done")["result_count"].is_null(),
        "task_overview is not a search tool"
    );
}

/// Proposes `create_task` for `items` of `batch_id` and returns the card.
fn propose_create(f: &Fixture, batch_id: &str, items: &[&str]) -> Value {
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "p1",
            "propose_plan",
            json!({"steps": [{"tool_id": "create_task", "input": {
                "batch_id": batch_id, "revision": 0, "item_ids": items, "options": {}}}]}),
        ),
        text_turn(json!({"text": "已提出下载", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载这些", SendContext::default())
        .unwrap();
    f.wait_for("confirm_card").payload
}

#[test]
fn confirm_card_counts_the_selection_its_queued_products_and_orbit_directions() {
    let f = fixture("card-facts", vec![]);
    seed_batch(&f, "batch-b", json!({}), sar_products(), &["B1"]);
    let card = propose_create(&f, "batch-b", &["B1", "B2", "B3"]);
    let snapshot = &card["snapshot"];
    assert_eq!(snapshot["range"]["aoi_name"], "西宁北");
    assert_eq!(snapshot["result_count"], 3, "{snapshot}");
    assert_eq!(snapshot["already_queued_count"], 1, "{snapshot}");
    assert_eq!(
        snapshot["orbit_directions"],
        json!(["Ascending", "Descending"]),
        "{snapshot}"
    );
    // The desktop echoes the card verbatim; the new facts are part of it.
    f.host
        .plan_approve(card["plan_id"].as_str().unwrap(), snapshot.clone(), None)
        .unwrap();
    f.wait_for("conclusion");
}

/// A direction filter in the frozen query is not a fact about the products:
/// with no product recording a direction the card says none (shown unknown).
#[test]
fn confirm_card_orbit_directions_never_come_from_the_frozen_filter() {
    let f = fixture("card-orbit", vec![]);
    seed_batch(
        &f,
        "batch-c",
        json!({"flight_direction": "DESCENDING"}),
        vec![product("C1", json!({}))],
        &[],
    );
    let snapshot = propose_create(&f, "batch-c", &["C1"])["snapshot"].clone();
    assert_eq!(snapshot["orbit_directions"], json!([]), "{snapshot}");
    assert_eq!(snapshot["result_count"], 1, "{snapshot}");
    assert_eq!(snapshot["already_queued_count"], 0, "{snapshot}");
}

#[test]
fn claim_refs_name_the_run_record_step_and_its_response_count() {
    let f = fixture("claim-steps", vec![]);
    seed_batch(&f, "batch-b", json!({}), sar_products(), &[]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "c1",
            "batch_page",
            json!({"batch_id": "batch-b", "revision": 0, "page": 0, "page_size": 10}),
        ),
        text_turn(json!({"text": "批次有 3 景", "claims": [
            {"statement": "批次有 3 景", "refs": ["batch:batch-b"]},
            {"statement": "网上的说法", "refs": ["web:https://example.org/x"]}
        ]})),
    ]);
    let run_id = f
        .host
        .send(&f.session, "看看批次", SendContext::default())
        .unwrap();
    let conclusion = f.wait_for("conclusion").payload;
    let detail = f.host.run_open(&run_id).unwrap();
    let step = detail
        .events
        .iter()
        .rev()
        .find(|e| e.kind == "tool_result" && e.payload["summary"]["search_batch_id"] == "batch-b")
        .expect("the batch read is in the run record")
        .sequence;
    let batch_ref = &conclusion["claims"][0]["refs"][0];
    assert_eq!(conclusion["claims"][0]["verified"], true);
    assert_eq!(batch_ref["trace_sequence"], step, "{batch_ref}");
    assert_eq!(batch_ref["response_count"], 3, "{batch_ref}");
    let web_ref = &conclusion["claims"][1]["refs"][0];
    assert!(web_ref["trace_sequence"].is_null(), "{web_ref}");
    assert!(web_ref["response_count"].is_null(), "{web_ref}");
}

/// Review fix: the card follows create_task admission, which refuses a
/// selection with duplicates or products outside the frozen batch.
#[test]
fn a_create_task_card_refuses_duplicate_or_unknown_products() {
    for (label, items) in [("dup", ["B1", "B1"]), ("unknown", ["B1", "ZZ"])] {
        let f = fixture(&format!("card-selection-{label}"), vec![]);
        seed_batch(&f, "batch-b", json!({}), sar_products(), &[]);
        f.replay.turns.lock().unwrap().extend([
            tool_turn(
                "p1",
                "propose_plan",
                json!({"steps": [{"tool_id": "create_task", "input": {
                    "batch_id": "batch-b", "revision": 0, "item_ids": items, "options": {}}}]}),
            ),
            text_turn(json!({"text": "无法提出", "claims": []})),
        ]);
        f.host
            .send(&f.session, "下载", SendContext::default())
            .unwrap();
        f.wait_for("conclusion");
        let events = f.host.stream_poll(&f.session, 0).unwrap().events;
        assert!(
            !events
                .iter()
                .any(|e| e.kind == "confirm_card" || e.kind == "plan_card"),
            "{label}: no card for a selection the service would refuse"
        );
        let seen = f.replay.seen.lock().unwrap();
        let reply = seen.last().unwrap().last().unwrap();
        assert_eq!(reply.role, "tool");
        assert!(
            reply.content.contains("plan_refused")
                && reply.content.contains("AGENT_INVALID_REQUEST"),
            "{label}: {}",
            reply.content
        );
        assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    }
}

/// Review fix: a failed round has a duration but no counts; an unfinished
/// search has no result count; a message the sanitizer replaced is not shown.
#[test]
fn tool_rounds_leave_counts_and_summary_null_when_the_result_does_not_state_them() {
    let f = fixture("tool-unknown", vec![]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "c1",
            "workbench_page",
            json!({"page_index": 0, "page_size": 10}),
        ),
        tool_turn(
            "c2",
            "search_progress",
            json!({"request_id": "req-running"}),
        ),
        tool_turn("c3", "search_progress", json!({"request_id": "req-local"})),
        text_turn(json!({"text": "完成", "claims": []})),
    ]);
    f.host
        .send(&f.session, "看看进度", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let rounds = tool_rounds(&f);
    let round = |call: &str, status: &str| {
        rounds
            .iter()
            .find(|r| r["call_id"] == call && r["status"] == status)
            .cloned()
            .unwrap_or_else(|| panic!("no {status} round for {call}: {rounds:?}"))
    };
    let failed = round("c1", "failed");
    assert!(failed["duration_ms"].as_u64().is_some(), "{failed}");
    assert!(failed["result_count"].is_null(), "{failed}");
    assert!(failed["summary_zh"].is_null(), "{failed}");
    let running = round("c2", "done");
    assert!(running["result_count"].is_null(), "{running}");
    assert_eq!(running["summary_zh"], "正在检索");
    let local = round("c3", "done");
    assert_eq!(local["result_count"], 3, "{local}");
    assert!(
        local["summary_zh"].is_null(),
        "a message the sanitizer replaced is not shown: {local}"
    );
}
/// Payloads of every `kind` event so far, oldest first; waits for `count`.
/// `events_of` through a given host (one reopened on the fixture's root).
fn events_via(
    host: &AgentHost<Fake, Source>,
    session: &str,
    kind: &str,
    count: usize,
) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let found: Vec<Value> = host
            .stream_poll(session, 0)
            .unwrap()
            .events
            .into_iter()
            .filter(|event| event.kind == kind)
            .map(|event| event.payload)
            .collect();
        if found.len() >= count {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "{} of {count} {kind} events",
            found.len()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn events_of(f: &Fixture, kind: &str, count: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let found: Vec<Value> = f
            .host
            .stream_poll(&f.session, 0)
            .unwrap()
            .events
            .into_iter()
            .filter(|event| event.kind == kind)
            .map(|event| event.payload)
            .collect();
        if found.len() >= count {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "{} of {count} {kind} events",
            found.len()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn permission_mode_defaults_to_confirm_changes_and_ignores_the_global_file() {
    let f = fixture("mode-store", vec![]);
    let global = f.root.join("agent/agent-permission-mode.json");
    let old = b"\"full_access\"";
    fs::write(&global, old).unwrap();
    let view = f.host.permission_mode_get(None).unwrap();
    assert_eq!(view.effective, PermissionMode::ConfirmChanges);
    assert_eq!(view.global_default, PermissionMode::ConfirmChanges);
    assert!(view.session_override.is_none());
    let other = f
        .host
        .session_create(Some("其他".into()))
        .unwrap()
        .session_id;

    assert!(f
        .host
        .permission_mode_set_global(PermissionMode::FullAccess)
        .is_err());
    assert_eq!(fs::read(&global).unwrap(), old);
    let view = f
        .host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    assert_eq!(view.effective, PermissionMode::AutoApprove);
    assert_eq!(view.session_override, Some(PermissionMode::AutoApprove));
    assert_eq!(view.global_default, PermissionMode::ConfirmChanges);
    check(
        "AgentPermissionModeView",
        &serde_json::to_value(view).unwrap(),
    );
    assert_eq!(
        f.host.permission_mode_get(Some(&other)).unwrap().effective,
        PermissionMode::ConfirmChanges
    );

    // The session setting survives a restart; the session list carries it.
    let reopened = AgentHost::open(
        f.root.clone(),
        f.fake.clone(),
        Source(Arc::clone(&f.replay)),
    )
    .unwrap();
    assert_eq!(
        reopened.permission_mode_get(Some(&f.session)).unwrap(),
        view
    );
    let listed = reopened
        .sessions_list()
        .unwrap()
        .into_iter()
        .find(|session| session.session_id == f.session)
        .unwrap();
    assert_eq!(listed.permission_mode, Some(PermissionMode::AutoApprove));
    check(
        "AgentSessionSummary",
        &serde_json::to_value(&listed).unwrap(),
    );

    let view = reopened
        .permission_mode_set_session(&f.session, None)
        .unwrap();
    assert_eq!(view.effective, PermissionMode::ConfirmChanges);
    assert!(view.session_override.is_none());
    assert!(matches!(
        reopened.permission_mode_get(Some("session-missing")),
        Err(HostError::NotFound)
    ));
}

/// Session overrides retain legacy decoding; the retired global file is ignored.
#[test]
fn stored_w3e_modes_are_read_under_the_new_names_and_written_as_plain_names() {
    let f = fixture("mode-legacy", vec![]);
    for (mode, name) in [
        (PermissionMode::ConfirmChanges, "confirm_changes"),
        (PermissionMode::AutoApprove, "auto_approve"),
        (PermissionMode::FullAccess, "full_access"),
    ] {
        check("AgentPermissionMode", &json!(name));
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(name));
    }
    let global = f.root.join("agent/agent-permission-mode.json");
    fs::write(
        &global,
        json!({"kind": "within_quota", "max_bytes": 20_000_000_000u64, "max_tasks": 10,
            "output_dir": f.root.join("out").to_string_lossy()})
        .to_string(),
    )
    .unwrap();
    let session_path = f
        .root
        .join("agent/sessions")
        .join(&f.session)
        .join("session.json");
    let write_session_mode = |mode: Value| {
        let mut record: Value = serde_json::from_slice(&fs::read(&session_path).unwrap()).unwrap();
        record["permission_mode"] = mode;
        fs::write(&session_path, record.to_string()).unwrap();
    };
    write_session_mode(json!({"kind": "per_item"}));
    let view = f.host.permission_mode_get(Some(&f.session)).unwrap();
    assert_eq!(view.global_default, PermissionMode::ConfirmChanges);
    assert_eq!(view.session_override, Some(PermissionMode::ConfirmChanges));
    assert_eq!(view.effective, PermissionMode::ConfirmChanges);
    check(
        "AgentPermissionModeView",
        &serde_json::to_value(view).unwrap(),
    );
    write_session_mode(json!({"kind": "full_access"}));
    assert_eq!(
        f.host
            .permission_mode_get(Some(&f.session))
            .unwrap()
            .effective,
        PermissionMode::FullAccess
    );
    write_session_mode(json!({"kind": "auto"}));
    assert_eq!(
        f.host
            .permission_mode_get(Some(&f.session))
            .unwrap()
            .session_override,
        Some(PermissionMode::ConfirmChanges),
        "an unknown stored value tightens, never follows the global default"
    );
    assert!(f
        .host
        .sessions_list()
        .unwrap()
        .iter()
        .any(|session| session.session_id == f.session));

    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    let record: Value = serde_json::from_slice(&fs::read(&session_path).unwrap()).unwrap();
    assert_eq!(record["permission_mode"], "full_access");
    assert!(f
        .host
        .permission_mode_set_global(PermissionMode::ConfirmChanges)
        .is_err());
    let stored: Value = serde_json::from_slice(&fs::read(&global).unwrap()).unwrap();
    assert_eq!(stored["kind"], "within_quota");
    // An unreadable retired file also has no effect.
    fs::write(&global, "{not json").unwrap();
    assert_eq!(
        f.host.permission_mode_get(None).unwrap().global_default,
        PermissionMode::ConfirmChanges
    );
}

#[test]
fn stop_all_cancels_live_runs_and_the_tasks_agent_runs_created() {
    let f = fixture("stop-all", vec![]);
    assert_eq!(f.host.stop_all().unwrap(), StopAllResult::default());
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-b", json!({}), sar_products(), &[]);
    // Run 1 creates a task through the desktop card.
    let card = propose_create(&f, "batch-b", &["B1", "B2"]);
    f.host
        .plan_approve(
            card["plan_id"].as_str().unwrap(),
            card["snapshot"].clone(),
            None,
        )
        .unwrap();
    events_of(&f, "conclusion", 1);
    let created: Vec<String> = f
        .fake
        .store
        .lock()
        .unwrap()
        .core_restore(false)
        .unwrap()
        .tasks
        .iter()
        .map(|task| task.task_id.clone())
        .filter(|id| id.starts_with("agent-task-"))
        .collect();
    assert_eq!(created.len(), 1, "{created:?}");
    // Run 2's create_task reopens the user's own live task (created=false):
    // the Agent did not create it, so stop-all leaves it alone.
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "p2",
            "propose_plan",
            json!({"steps": [{"tool_id": "create_task", "input": {
                "batch_id": "batch-a", "revision": 0, "item_ids": ["ITEM-A"], "options": {}}}]}),
        ),
        text_turn(json!({"text": "已在队列", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载 A", SendContext::default())
        .unwrap();
    let card = events_of(&f, "confirm_card", 2).pop().unwrap();
    f.host
        .plan_approve(
            card["plan_id"].as_str().unwrap(),
            card["snapshot"].clone(),
            None,
        )
        .unwrap();
    events_of(&f, "conclusion", 2);
    // Run 3 waits for its card.
    f.replay.turns.lock().unwrap().push_back(tool_turn(
        "p3",
        "propose_plan",
        pause_plan(f.revision()),
    ));
    let waiting = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    events_of(&f, "confirm_card", 3);
    let task_a_before = f.revision();

    let result = f.host.stop_all().unwrap();
    assert_eq!(result.cancelled_runs, vec![waiting.clone()]);
    assert_eq!(result.cancelled_tasks, created);
    assert!(result.failed_tasks.is_empty(), "{result:?}");
    check(
        "AgentStopAllResult",
        &serde_json::to_value(&result).unwrap(),
    );
    assert_eq!(f.run_status(&waiting), "cancelled");
    let store = f.fake.store.lock().unwrap();
    let agent_task = store.load_task_record(&created[0]).unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&agent_task.runtime.status).unwrap(),
        "cancelled"
    );
    let user_task = store.load_task_record("task-a").unwrap().unwrap();
    assert_ne!(
        serde_json::to_value(&user_task.runtime.status).unwrap(),
        "cancelled"
    );
    assert_eq!(user_task.runtime.revision, task_a_before);
    drop(store);
    let recorded = events_of(&f, "stop_all", 1);
    assert_eq!(recorded, vec![serde_json::to_value(&result).unwrap()]);
    // Nothing left: a second press stops nothing and records nothing.
    assert_eq!(
        f.host.stop_all().unwrap(),
        StopAllResult {
            cancelled_runs: vec![],
            failed_runs: vec![],
            cancelled_tasks: vec![],
            failed_tasks: vec![],
        }
    );
    assert_eq!(events_of(&f, "stop_all", 1).len(), 1);
}

const GB: u64 = 1_000_000_000;

fn sized(item_id: &str, bytes: u64) -> SnapshotItem {
    product(item_id, json!({"size_bytes": bytes}))
}
fn create_turn(call: &str, batch_id: &str, items: &[&str], options: Value) -> String {
    tool_turn(
        call,
        "propose_plan",
        json!({"steps": [{"tool_id": "create_task", "input": {
            "batch_id": batch_id, "revision": 0, "item_ids": items, "options": options}}]}),
    )
}
fn search_turn(call: &str) -> String {
    tool_turn(
        call,
        "propose_plan",
        json!({"steps": [{"tool_id": "search_start", "input": {
            "resource_id": "science.sentinel-1.slc.v1", "aoi": {"ref": "context_aoi"},
            "aoi_name": "区域", "time_range": null, "filters": {}}}]}),
    )
}
/// One run's search (which freezes `batch_id` in the Fake), the model
/// reading its finished progress, then a create_task from that batch.
fn search_then_create(
    f: &Fixture,
    call: &str,
    batch_id: &str,
    items: &[&str],
    options: Value,
) -> Vec<String> {
    *f.fake.next_search_batch.lock().unwrap() = Some(batch_id.into());
    vec![
        search_turn(&format!("{call}-s")),
        tool_turn(
            &format!("{call}-p"),
            "search_progress",
            json!({"request_id": "req-latest"}),
        ),
        create_turn(&format!("{call}-c"), batch_id, items, options),
    ]
}
fn with_aoi() -> SendContext {
    SendContext {
        aoi: Some(json!({"name": "区域", "geometry": {"type": "Polygon",
            "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]}})),
        time_range: None,
        source: None,
        model: None,
        ..SendContext::default()
    }
}
fn output(f: &Fixture, name: &str) -> Value {
    json!({"output_root": f.root.join("out").join(name).to_string_lossy()})
}
fn approval_of(card: &Value) -> Value {
    card["approval"].clone()
}
fn manual(mode: &str, reasons: Value) -> Value {
    json!({"decision": "manual", "mode": mode, "card_reasons": reasons})
}
fn auto(mode: &str) -> Value {
    json!({"decision": "auto_by_mode", "mode": mode, "card_reasons": []})
}
fn default_dir(f: &Fixture) -> String {
    f.root.join("default-out").to_string_lossy().into_owned()
}

/// Matrix rows under confirm_changes: a create_task waits for the card (the
/// card still names the facts that would keep it from auto_approve); a
/// search runs without a click and its plan card discloses source and
/// conditions.
#[test]
fn confirm_changes_waits_for_the_card_on_tier_2_and_runs_a_disclosed_search_without_a_click() {
    let f = fixture("gate-confirm", vec![]);
    seed_batch(&f, "batch-b", json!({}), vec![sized("S1", GB)], &[]);
    f.replay.turns.lock().unwrap().extend([
        create_turn("p1", "batch-b", &["S1"], output(&f, "a")),
        text_turn(json!({"text": "已下载", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载", SendContext::default())
        .unwrap();
    let plan = events_of(&f, "plan_card", 1).remove(0);
    let card = events_of(&f, "confirm_card", 1).remove(0);
    let expected = manual(
        "confirm_changes",
        json!(["output_dir_unverified", "selection_not_from_run"]),
    );
    for shown in [&plan, &card] {
        assert_eq!(approval_of(shown), expected, "{shown}");
        assert_eq!(shown["auto_executed"], false, "{shown}");
    }
    assert_eq!(card["snapshot"]["output_dir_source"], "model_suggested");
    check("AgentPlanCard", &plan);
    check("AgentConfirmCard", &card);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        f.fake.submits.load(Ordering::SeqCst),
        0,
        "nothing runs unclicked"
    );
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    f.host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .unwrap();
    events_of(&f, "conclusion", 1);
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 1);
    let approval = f
        .fake
        .store
        .lock()
        .unwrap()
        .agent_approval(&plan_id)
        .unwrap()
        .unwrap();
    assert_eq!(approval.decided_via, "desktop_card");
    assert_eq!(approval.mode.as_deref(), Some("confirm_changes"));

    // Tier 1 alone (a provider search) runs without a click and is disclosed.
    *f.fake.sources.lock().unwrap() = json!([
        {"resource_id": "science.sentinel-1.grd.v1", "display_name_zh": "Sentinel-1 GRD"},
        {"resource_id": "science.sentinel-1.slc.v1", "display_name_zh": "Sentinel-1 SLC"},
    ]);
    f.replay.turns.lock().unwrap().extend([
        search_turn("p2"),
        text_turn(json!({"text": "已检索", "claims": []})),
    ]);
    f.host.send(&f.session, "检索", with_aoi()).unwrap();
    events_of(&f, "conclusion", 2);
    let search = events_of(&f, "plan_card", 2).remove(1);
    assert_eq!(search["max_tier"], 1);
    assert_eq!(approval_of(&search), auto("confirm_changes"));
    assert_eq!(search["auto_executed"], true);
    assert_eq!(
        search["steps"][0]["disclosure"],
        json!({"resource_id": "science.sentinel-1.slc.v1", "aoi_name": "区域", "time_range": null,
            "display_name_zh": "Sentinel-1 SLC"})
    );
    check("AgentPlanCard", &search);
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 2);
    let approval = f
        .fake
        .store
        .lock()
        .unwrap()
        .agent_approval(search["plan_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(approval.decided_via, "auto_by_mode");
    assert_eq!(approval.mode.as_deref(), Some("confirm_changes"));
}

#[test]
fn full_access_runs_tier_1_and_2_plans_without_a_click_and_records_the_approval_by_mode() {
    let f = fixture("gate-full", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-b", json!({}), vec![sized("S1", GB)], &[]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        create_turn("p1", "batch-b", &["S1"], json!({})),
        search_turn("p2"),
        text_turn(json!({"text": "已下载并检索", "claims": []})),
    ]);
    let run_id = f.host.send(&f.session, "下载并检索", with_aoi()).unwrap();
    events_of(&f, "conclusion", 1);
    let plans = events_of(&f, "plan_card", 2);
    let card = events_of(&f, "confirm_card", 1).remove(0);
    for shown in plans.iter().chain([&card]) {
        assert_eq!(approval_of(shown), auto("full_access"), "{shown}");
        assert_eq!(shown["auto_executed"], true, "{shown}");
    }
    check("AgentPlanCard", &plans[0]);
    check("AgentConfirmCard", &card);
    assert_eq!(
        f.fake.submits.load(Ordering::SeqCst),
        2,
        "create_task and search_start ran without a click"
    );
    let store = f.fake.store.lock().unwrap();
    for shown in &plans {
        let plan_id = shown["plan_id"].as_str().unwrap();
        let approval = store.agent_approval(plan_id).unwrap().unwrap();
        assert_eq!(approval.decided_via, "auto_by_mode");
        assert_eq!(approval.mode.as_deref(), Some("full_access"));
        assert_eq!(approval.decision, "approved");
        assert!(
            is_rfc3339_utc(&approval.decided_at),
            "{}",
            approval.decided_at
        );
        assert_eq!(
            approval.plan_hash_seen,
            shown["plan_hash"].as_str().unwrap()
        );
        assert_eq!(
            store.agent_plan(plan_id).unwrap().unwrap().status,
            "approved"
        );
    }
    let first = store
        .agent_approval(plans[0]["plan_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&first.snapshot_json).unwrap(),
        card["snapshot"],
        "the approval snapshot is the card the user is shown"
    );
    drop(store);
    let detail = f.host.run_open(&run_id).unwrap();
    let approvals: Vec<&Value> = detail
        .events
        .iter()
        .filter(|e| e.kind == "approval")
        .map(|e| &e.payload)
        .collect();
    assert_eq!(approvals.len(), 2, "{approvals:?}");
    for payload in approvals {
        assert_eq!(payload["decision"], "approved", "{payload}");
        assert_eq!(payload["via"], "auto_by_mode", "{payload}");
        assert_eq!(payload["mode"], "full_access", "{payload}");
    }
    assert!(detail
        .events
        .iter()
        .any(|e| e.kind == "permission" && e.payload["verdict"] == "auto_by_mode"));
    for kind in ["plan", "execution", "answer"] {
        assert!(
            detail.events.iter().any(|e| e.kind == kind),
            "trace lacks {kind}"
        );
    }
    assert_eq!(f.run_status(&run_id), "completed");
}

/// Matrix row "create_task, complete" under auto_approve: the batch was
/// frozen by this run's search, sizes are recorded, nothing is queued, the
/// accounts are ready and the default download directory is set.
#[test]
fn auto_approve_runs_a_complete_create_task_from_this_runs_search_without_a_click() {
    let f = fixture("auto-complete", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-s", json!({}), vec![sized("S1", GB)], &[]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    let mut turns = search_then_create(&f, "a", "batch-s", &["S1"], json!({}));
    turns.push(text_turn(json!({"text": "已检索并下载", "claims": []})));
    f.replay.turns.lock().unwrap().extend(turns);
    let run_id = f.host.send(&f.session, "检索并下载", with_aoi()).unwrap();
    events_of(&f, "conclusion", 1);
    let plans = events_of(&f, "plan_card", 2);
    assert_eq!(approval_of(&plans[0]), auto("auto_approve"));
    assert_eq!(
        plans[0]["steps"][0]["disclosure"]["resource_id"],
        "science.sentinel-1.slc.v1"
    );
    assert_eq!(approval_of(&plans[1]), auto("auto_approve"));
    assert_eq!(plans[1]["auto_executed"], true);
    let card = events_of(&f, "confirm_card", 1).remove(0);
    assert_eq!(card["snapshot"]["output_dir"], default_dir(&f));
    assert_eq!(card["snapshot"]["output_dir_source"], "default");
    assert_eq!(card["snapshot"]["already_queued_count"], 0);
    check("AgentPlanCard", &plans[1]);
    check("AgentConfirmCard", &card);
    let created = f.fake.created.lock().unwrap().clone();
    assert_eq!(created.len(), 1, "{created:?}");
    assert_eq!(
        created[0]["options"]["output_root"],
        json!(default_dir(&f)),
        "the default directory the card showed is frozen in the plan"
    );
    let approval = f
        .fake
        .store
        .lock()
        .unwrap()
        .agent_approval(plans[1]["plan_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(approval.decided_via, "auto_by_mode");
    assert_eq!(approval.mode.as_deref(), Some("auto_approve"));
    let detail = f.host.run_open(&run_id).unwrap();
    assert!(
        detail
            .events
            .iter()
            .any(|e| e.kind == "search_batch" && e.payload["batch_id"] == "batch-s"),
        "the batch the run's search froze is in the run's own record"
    );
}

/// Matrix row "create_task, incomplete" under auto_approve: each missing
/// fact sends the plan to the card, which names it.
#[test]
fn auto_approve_sends_an_incomplete_create_task_to_the_card_naming_the_reason() {
    let f = fixture("auto-incomplete", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(
        &f,
        "batch-s1",
        json!({}),
        vec![product("N1", json!({}))],
        &[],
    );
    seed_batch(&f, "batch-s2", json!({}), vec![sized("Q1", GB)], &["Q1"]);
    seed_batch(&f, "batch-s3", json!({}), vec![sized("A1", GB)], &[]);
    seed_batch(&f, "batch-s4", json!({}), vec![sized("F1", GB)], &[]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    let cases = [
        ("size_unknown", "batch-s1", "N1", true),
        ("reopens_existing_task", "batch-s2", "Q1", true),
        ("account_not_ready", "batch-s3", "A1", true),
        ("selection_not_from_run", "batch-s4", "F1", false),
    ];
    for (index, (reason, batch, item, searched)) in cases.into_iter().enumerate() {
        f.fake
            .accounts_ready
            .store(reason != "account_not_ready", Ordering::SeqCst);
        let mut turns = if searched {
            search_then_create(&f, &format!("c{index}"), batch, &[item], json!({}))
        } else {
            vec![create_turn(&format!("c{index}"), batch, &[item], json!({}))]
        };
        turns.push(text_turn(json!({"text": "未执行", "claims": []})));
        f.replay.turns.lock().unwrap().extend(turns);
        f.host.send(&f.session, "检索并下载", with_aoi()).unwrap();
        let card = events_of(&f, "confirm_card", index + 1).remove(index);
        assert_eq!(
            approval_of(&card),
            manual("auto_approve", json!([reason])),
            "{reason}"
        );
        assert_eq!(card["auto_executed"], false, "{reason}");
        check("AgentConfirmCard", &card);
        f.host
            .plan_reject(card["plan_id"].as_str().unwrap())
            .unwrap();
        events_of(&f, "conclusion", index + 1);
    }
    assert!(f.fake.created.lock().unwrap().is_empty());
}

/// The run record, not the service's memory, says which batches the run's
/// searches froze; the service lookup is only the fallback for a search
/// whose progress the model never read.
#[test]
fn a_selection_is_from_the_run_by_its_record_even_after_the_service_forgot_it() {
    let f = fixture("auto-run-batches", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(
        &f,
        "batch-s",
        json!({}),
        vec![sized("S1", GB), sized("S2", GB), sized("S3", GB)],
        &[],
    );
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    // Run 1: the progress read records the batch; the service then forgets.
    f.fake.forget_search_batch.store(true, Ordering::SeqCst);
    let mut turns = search_then_create(&f, "r1", "batch-s", &["S1"], json!({}));
    turns.push(text_turn(json!({"text": "一", "claims": []})));
    f.replay.turns.lock().unwrap().extend(turns);
    let first = f.host.send(&f.session, "检索并下载", with_aoi()).unwrap();
    events_of(&f, "conclusion", 1);
    assert!(f.fake.search_batches.lock().unwrap().is_empty());
    assert_eq!(
        approval_of(&events_of(&f, "plan_card", 2)[1]),
        auto("auto_approve")
    );
    let recorded = f.host.run_open(&first).unwrap();
    assert_eq!(
        recorded
            .events
            .iter()
            .filter(|e| e.kind == "search_batch")
            .count(),
        1
    );
    // Run 2: no progress read; the service lookup answers.
    f.fake.forget_search_batch.store(false, Ordering::SeqCst);
    *f.fake.next_search_batch.lock().unwrap() = Some("batch-s".into());
    f.replay.turns.lock().unwrap().extend([
        search_turn("r2-s"),
        create_turn("r2-c", "batch-s", &["S2"], json!({})),
        text_turn(json!({"text": "二", "claims": []})),
    ]);
    let second = f.host.send(&f.session, "检索并下载", with_aoi()).unwrap();
    events_of(&f, "conclusion", 2);
    assert_eq!(
        approval_of(&events_of(&f, "plan_card", 4)[3]),
        auto("auto_approve")
    );
    assert!(!f
        .host
        .run_open(&second)
        .unwrap()
        .events
        .iter()
        .any(|e| e.kind == "search_batch"));
    // Run 3: the run's search froze another batch.
    *f.fake.next_search_batch.lock().unwrap() = Some("batch-other".into());
    f.replay.turns.lock().unwrap().extend([
        search_turn("r3-s"),
        create_turn("r3-c", "batch-s", &["S3"], json!({})),
        text_turn(json!({"text": "三", "claims": []})),
    ]);
    f.host.send(&f.session, "检索并下载", with_aoi()).unwrap();
    let card = events_of(&f, "confirm_card", 3).remove(2);
    assert_eq!(
        approval_of(&card),
        manual("auto_approve", json!(["selection_not_from_run"]))
    );
    f.host
        .plan_reject(card["plan_id"].as_str().unwrap())
        .unwrap();
    events_of(&f, "conclusion", 3);
    assert_eq!(f.fake.created.lock().unwrap().len(), 2);
}

/// Matrix: "目录未知仍弹卡（任何模式）". Without a directory the plan cannot
/// be approved; a directory chosen on the card is written into the plan.
#[test]
fn an_unknown_output_directory_needs_the_card_in_every_mode() {
    for (label, mode, name) in [
        ("confirm", PermissionMode::ConfirmChanges, "confirm_changes"),
        ("auto", PermissionMode::AutoApprove, "auto_approve"),
        ("full", PermissionMode::FullAccess, "full_access"),
    ] {
        let f = fixture(&format!("no-dir-{label}"), vec![]);
        *f.fake.default_output.lock().unwrap() = None;
        f.fake.create_ok.store(true, Ordering::SeqCst);
        seed_batch(&f, "batch-b", json!({}), vec![sized("S1", GB)], &[]);
        f.host
            .permission_mode_set_session(&f.session, Some(mode))
            .unwrap();
        f.replay.turns.lock().unwrap().extend([
            create_turn("p1", "batch-b", &["S1"], json!({})),
            text_turn(json!({"text": "完成", "claims": []})),
        ]);
        f.host
            .send(&f.session, "下载", SendContext::default())
            .unwrap();
        let card = events_of(&f, "confirm_card", 1).remove(0);
        assert_eq!(card["approval"]["decision"], "manual", "{label}");
        assert_eq!(card["approval"]["mode"], name, "{label}");
        assert_eq!(
            card["approval"]["card_reasons"][0], "output_dir_unknown",
            "{label}: {card}"
        );
        assert!(card["snapshot"]["output_dir"].is_null(), "{label}");
        assert!(card["snapshot"]["output_dir_source"].is_null(), "{label}");
        check("AgentConfirmCard", &card);
        let plan_id = card["plan_id"].as_str().unwrap().to_owned();
        for missing in [None, Some("relative/out".to_owned())] {
            assert!(
                matches!(
                    f.host
                        .plan_approve(&plan_id, card["snapshot"].clone(), missing),
                    Err(HostError::Invalid)
                ),
                "{label}"
            );
        }
        assert_eq!(
            f.plan_status(&plan_id),
            "proposed",
            "{label}: still waiting"
        );
        let chosen = f.root.join("chosen").to_string_lossy().into_owned();
        f.host
            .plan_approve(&plan_id, card["snapshot"].clone(), Some(chosen.clone()))
            .unwrap();
        events_of(&f, "conclusion", 1);
        let created = f.fake.created.lock().unwrap().clone();
        assert_eq!(
            created[0]["options"]["output_root"],
            json!(chosen),
            "{label}"
        );
    }
}

/// Review ruling 2026-10-26: a directory in the plan came from the model and
/// is never "known" — the user's message says 下载到默认目录, the model
/// invents another — so it is shown as 模型建议的目录 and confirmed on the
/// card in every mode; the card may keep it or choose another.
#[test]
fn a_model_supplied_directory_is_confirmed_on_the_card_in_every_mode() {
    for (label, mode, name) in [
        ("confirm", PermissionMode::ConfirmChanges, "confirm_changes"),
        ("auto", PermissionMode::AutoApprove, "auto_approve"),
        ("full", PermissionMode::FullAccess, "full_access"),
    ] {
        let f = fixture(&format!("model-dir-{label}"), vec![]);
        f.fake.create_ok.store(true, Ordering::SeqCst);
        seed_batch(&f, "batch-s", json!({}), vec![sized("S1", GB)], &[]);
        f.host
            .permission_mode_set_session(&f.session, Some(mode))
            .unwrap();
        let invented = f.root.join("invented").to_string_lossy().into_owned();
        let mut turns = search_then_create(
            &f,
            "m",
            "batch-s",
            &["S1"],
            json!({"output_root": invented}),
        );
        turns.push(text_turn(json!({"text": "完成", "claims": []})));
        f.replay.turns.lock().unwrap().extend(turns);
        f.host
            .send(&f.session, "检索并下载到默认目录", with_aoi())
            .unwrap();
        let card = events_of(&f, "confirm_card", 1).remove(0);
        assert_eq!(
            approval_of(&card),
            manual(name, json!(["output_dir_unverified"])),
            "{label}: the only reason is the model's directory"
        );
        assert_eq!(card["auto_executed"], false, "{label}");
        assert_eq!(card["snapshot"]["output_dir"], json!(invented), "{label}");
        assert_eq!(
            card["snapshot"]["output_dir_source"], "model_suggested",
            "{label}"
        );
        check("AgentConfirmCard", &card);
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            f.fake.created.lock().unwrap().is_empty(),
            "{label}: nothing ran"
        );
        // Approving the card as shown keeps the suggested directory.
        f.host
            .plan_approve(
                card["plan_id"].as_str().unwrap(),
                card["snapshot"].clone(),
                None,
            )
            .unwrap();
        events_of(&f, "conclusion", 1);
        let created = f.fake.created.lock().unwrap().clone();
        assert_eq!(
            created[0]["options"]["output_root"],
            json!(invented),
            "{label}"
        );
    }
    // A model directory that is not a local drive-letter path is refused
    // outright (UNC here); nothing is planned.
    let f = fixture("model-dir-unc", vec![]);
    seed_batch(&f, "batch-b", json!({}), vec![sized("S1", GB)], &[]);
    f.replay.turns.lock().unwrap().extend([
        create_turn(
            "p1",
            "batch-b",
            &["S1"],
            json!({"output_root": r"\\server\share\out"}),
        ),
        text_turn(json!({"text": "未建", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    assert!(f
        .host
        .stream_poll(&f.session, 0)
        .unwrap()
        .events
        .iter()
        .all(|e| e.kind != "plan_card"));
}

/// Coordinator 2026-10-26 (audit): a directory chosen on the card rewrites
/// the plan in the approving transaction; the approval keeps the shown
/// plan's hash and the run record a plan_rewritten event without paths.
#[test]
fn a_directory_chosen_on_the_card_rewrites_the_plan_and_keeps_the_audit_trail() {
    let f = fixture("rewrite-dir", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-b", json!({}), vec![sized("S1", GB)], &[]);
    f.replay.turns.lock().unwrap().extend([
        create_turn("p1", "batch-b", &["S1"], json!({})),
        text_turn(json!({"text": "完成", "claims": []})),
    ]);
    let run_id = f
        .host
        .send(&f.session, "下载", SendContext::default())
        .unwrap();
    let card = events_of(&f, "confirm_card", 1).remove(0);
    assert_eq!(card["snapshot"]["output_dir_source"], "default");
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    let shown_hash = card["plan_hash"].as_str().unwrap().to_owned();
    let picked = f.root.join("picked-dir").to_string_lossy().into_owned();
    f.host
        .plan_approve(&plan_id, card["snapshot"].clone(), Some(picked.clone()))
        .unwrap();
    events_of(&f, "conclusion", 1);
    let created = f.fake.created.lock().unwrap().clone();
    assert_eq!(created[0]["options"]["output_root"], json!(picked));

    let store = f.fake.store.lock().unwrap();
    let approval = store.agent_approval(&plan_id).unwrap().unwrap();
    let plan = store.agent_plan(&plan_id).unwrap().unwrap();
    drop(store);
    assert_eq!(
        approval.rewritten_from_plan_hash.as_deref(),
        Some(shown_hash.as_str())
    );
    assert_ne!(approval.plan_hash_seen, shown_hash);
    assert_eq!(plan.plan_hash, approval.plan_hash_seen);
    let mut expected = card["snapshot"].clone();
    expected["output_dir"] = json!(picked);
    expected["output_dir_source"] = json!("chosen_on_card");
    assert_eq!(
        serde_json::from_str::<Value>(&approval.snapshot_json).unwrap(),
        expected,
        "only the directory and its source changed"
    );

    let detail = f.host.run_open(&run_id).unwrap();
    let rewritten = detail
        .events
        .iter()
        .find(|e| e.kind == "plan_rewritten")
        .expect("a plan_rewritten event")
        .payload
        .clone();
    let sha = |text: &str| insar_agent::session::path_digest(&run_id, text);
    let unsalted = |text: &str| {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(text.as_bytes()))
    };
    assert_ne!(sha(&picked), unsalted(&picked), "the digest is salted");
    assert_ne!(
        sha(&picked),
        insar_agent::session::path_digest("other-run", &picked),
        "per run"
    );
    assert_eq!(
        rewritten,
        json!({"plan_id": plan_id, "field": "output_dir",
            "from": {"source": "default", "sha256": sha(&default_dir(&f))},
            "to": {"source": "chosen_on_card", "sha256": sha(&picked)},
            "from_plan_hash": shown_hash, "to_plan_hash": approval.plan_hash_seen})
    );
    let decided = detail
        .events
        .iter()
        .find(|e| e.kind == "approval" && e.payload["plan_id"] == json!(plan_id))
        .unwrap();
    assert_eq!(
        decided.payload["rewritten_from_plan_hash"],
        json!(shown_hash)
    );
    let trace = serde_json::to_string(&detail.events).unwrap();
    assert!(!trace.contains("picked-dir") && !trace.contains("default-out"));

    // An echo that is not the shown card expires the plan; nothing runs.
    let g = fixture("rewrite-forged", vec![]);
    g.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&g, "batch-b", json!({}), vec![sized("S1", GB)], &[]);
    g.replay.turns.lock().unwrap().extend([
        create_turn("p1", "batch-b", &["S1"], json!({})),
        text_turn(json!({"text": "失效", "claims": []})),
    ]);
    g.host
        .send(&g.session, "下载", SendContext::default())
        .unwrap();
    let card = events_of(&g, "confirm_card", 1).remove(0);
    let mut forged = card["snapshot"].clone();
    forged["estimated_bytes"] = json!(1);
    let picked = g.root.join("picked-dir").to_string_lossy().into_owned();
    assert!(matches!(
        g.host
            .plan_approve(card["plan_id"].as_str().unwrap(), forged, Some(picked)),
        Err(HostError::Expired)
    ));
    events_of(&g, "conclusion", 1);
    assert_eq!(g.plan_status(card["plan_id"].as_str().unwrap()), "expired");
    assert!(g.fake.created.lock().unwrap().is_empty());
}

#[test]
fn the_forbidden_tier_is_refused_even_under_full_access() {
    let f = fixture("gate-forbidden", vec![]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    let revision = f.revision();
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "c1",
            "propose_plan",
            json!({"steps": [
                {"tool_id": "credential_save_cdse", "input": {"user": "x"}},
                {"tool_id": "task_action", "input": {"task_id": "task-a", "revision": revision,
                    "action": "pause", "event_id": "e"}}
            ]}),
        ),
        text_turn(json!({"text": "不能保存账号", "claims": []})),
    ]);
    let run_id = f
        .host
        .send(&f.session, "保存账号并暂停", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    let error = events_of(&f, "error", 1).remove(0);
    assert_eq!(error["code"], "plan_refused_forbidden");
    assert!(error["plan_id"].is_null());
    assert_eq!(
        error["approval"],
        json!({"decision": "refused_forbidden", "mode": "full_access", "card_reasons": []})
    );
    check("AgentErrorPayload", &error);
    let events = f.host.stream_poll(&f.session, 0).unwrap().events;
    assert!(!events.iter().any(|e| e.kind == "plan_card"));
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    let plans: i64 = rusqlite::Connection::open(f.root.join("store.db"))
        .unwrap()
        .query_row("SELECT count(*) FROM agent_plans", [], |r| r.get(0))
        .unwrap();
    assert_eq!(plans, 0, "nothing was planned");
    {
        let seen = f.replay.seen.lock().unwrap();
        let reply = seen.last().unwrap().last().unwrap();
        assert!(
            reply.content.contains("AGENT_TOOL_FORBIDDEN"),
            "{}",
            reply.content
        );
    }
    let detail = f.host.run_open(&run_id).unwrap();
    assert!(detail.events.iter().any(|e| e.kind == "approval"
        && e.payload["decision"] == "refused_forbidden"
        && e.payload["mode"] == "full_access"));
}

#[test]
fn a_session_mode_overrides_a_retired_global_file_at_the_gate() {
    let f = fixture("gate-override", vec![]);
    fs::write(
        f.root.join("agent/agent-permission-mode.json"),
        "\"full_access\"",
    )
    .unwrap();
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::ConfirmChanges))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "propose_plan", pause_plan(f.revision())),
        text_turn(json!({"text": "已暂停", "claims": []})),
    ]);
    f.host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let card = events_of(&f, "confirm_card", 1).remove(0);
    assert_eq!(
        approval_of(&card)["mode"],
        "confirm_changes",
        "the session setting wins"
    );
    assert_eq!(approval_of(&card)["decision"], "manual");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    f.host
        .plan_approve(
            card["plan_id"].as_str().unwrap(),
            card["snapshot"].clone(),
            None,
        )
        .unwrap();
    events_of(&f, "conclusion", 1);
    // Clearing the session setting restores the confirm_changes default.
    f.host
        .permission_mode_set_session(&f.session, None)
        .unwrap();
    let resume = json!({"steps": [{"tool_id": "task_action", "input": {"task_id": "task-a",
        "revision": f.revision(), "action": "resume", "event_id": "e"}}]});
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c2", "propose_plan", resume),
        text_turn(json!({"text": "已继续", "claims": []})),
    ]);
    f.host
        .send(&f.session, "继续", SendContext::default())
        .unwrap();
    let second = events_of(&f, "confirm_card", 2).remove(1);
    assert_eq!(approval_of(&second)["mode"], "confirm_changes");
    assert_eq!(approval_of(&second)["decision"], "manual");
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 1);
    f.host
        .plan_approve(
            second["plan_id"].as_str().unwrap(),
            second["snapshot"].clone(),
            None,
        )
        .unwrap();
    events_of(&f, "conclusion", 2);
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 2);
}

#[test]
fn a_mode_never_changes_the_tools_the_model_sees() {
    let mut seen = Vec::new();
    for (label, mode) in [
        ("confirm", PermissionMode::ConfirmChanges),
        ("auto", PermissionMode::AutoApprove),
        ("full", PermissionMode::FullAccess),
    ] {
        let f = fixture(&format!("tools-{label}"), vec![]);
        f.host
            .permission_mode_set_session(&f.session, Some(mode))
            .unwrap();
        f.replay
            .turns
            .lock()
            .unwrap()
            .push_back(text_turn(json!({"text": "好", "claims": []})));
        f.host
            .send(&f.session, "帮我查下工作区数据", SendContext::default())
            .unwrap();
        f.wait_for("conclusion");
        let tools = f.replay.tools_seen.lock().unwrap().last().unwrap().clone();
        seen.push(tools);
    }
    assert!(seen.iter().all(|tools| *tools == seen[0]), "{seen:?}");
    assert!(seen[0].contains(&"propose_plan".to_owned()));
    assert!(!seen[0]
        .iter()
        .any(|name| name.starts_with("credential_save")
            || name.contains("delete")
            || name.contains("shell")));
}

/// Item 5: the mode only decides who approves. What the service receives is
/// the frozen plan input in every mode, so the service's own admission
/// (storage-root check, NISAR writer disk-space admission, account
/// preflight) applies unchanged.
#[test]
fn a_mode_never_changes_what_the_service_receives() {
    let mut received = Vec::new();
    for (label, mode) in [
        ("manual", PermissionMode::ConfirmChanges),
        ("auto", PermissionMode::FullAccess),
    ] {
        let f = fixture(&format!("service-input-{label}"), vec![]);
        f.fake.create_ok.store(true, Ordering::SeqCst);
        seed_batch(&f, "batch-b", json!({}), vec![sized("S1", GB)], &[]);
        f.host
            .permission_mode_set_session(&f.session, Some(mode))
            .unwrap();
        let options = json!({"layout": "direct", "concurrency": 1});
        f.replay.turns.lock().unwrap().extend([
            create_turn("p1", "batch-b", &["S1"], options.clone()),
            text_turn(json!({"text": "完成", "claims": []})),
        ]);
        f.host
            .send(&f.session, "下载", SendContext::default())
            .unwrap();
        let card = events_of(&f, "confirm_card", 1).remove(0);
        if label == "manual" {
            f.host
                .plan_approve(
                    card["plan_id"].as_str().unwrap(),
                    card["snapshot"].clone(),
                    None,
                )
                .unwrap();
        }
        events_of(&f, "conclusion", 1);
        let created = f.fake.created.lock().unwrap().clone();
        assert_eq!(created.len(), 1, "{label}");
        let mut frozen = options.clone();
        frozen["output_root"] = json!(default_dir(&f));
        assert_eq!(created[0]["options"], frozen, "{label}: options as frozen");
        let mut request = created[0].clone();
        request["options"]["output_root"] = json!("<root>/out");
        received.push(request);
    }
    assert_eq!(received[0], received[1]);
}

#[test]
fn download_and_service_crates_never_see_the_permission_mode() {
    fn scan(dir: &Path, hits: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                scan(&path, hits);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = fs::read_to_string(&path).unwrap();
                for needle in [
                    "PermissionMode",
                    "permission_mode",
                    "auto_by_mode",
                    "confirm_changes",
                    "auto_approve",
                    "full_access",
                ] {
                    if text.contains(needle) {
                        hits.push(format!("{}: {needle}", path.display()));
                    }
                }
            }
        }
    }
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut hits = Vec::new();
    for name in ["insar-download", "insar-application-service"] {
        scan(&crates.join(name).join("src"), &mut hits);
    }
    assert!(hits.is_empty(), "{hits:?}");
}

fn events_in(f: &Fixture, session: &str, kind: &str, count: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let found: Vec<Value> = f
            .host
            .stream_poll(session, 0)
            .unwrap()
            .events
            .into_iter()
            .filter(|event| event.kind == kind)
            .map(|event| event.payload)
            .collect();
        if found.len() >= count {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "{} of {count} {kind} events in {session}",
            found.len()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn task_revision(f: &Fixture, task_id: &str) -> u64 {
    f.fake
        .store
        .lock()
        .unwrap()
        .load_task_record(task_id)
        .unwrap()
        .unwrap()
        .runtime
        .revision
}
fn task_status(f: &Fixture, task_id: &str) -> Value {
    let task = f
        .fake
        .store
        .lock()
        .unwrap()
        .load_task_record(task_id)
        .unwrap()
        .unwrap();
    serde_json::to_value(&task.runtime.status).unwrap()
}
fn update_turn(call: &str, task_id: &str, revision: u64, kind: &str) -> String {
    tool_turn(
        call,
        "propose_plan",
        json!({"steps": [{"tool_id": "update_download_task", "input": {"task_id": task_id,
            "expected_revision": revision, "command": {"kind": kind, "item_ids": []},
            "event_id": "e", "occurred_at": "2026-10-26T00:00:00Z"}}]}),
    )
}
fn control_turn(call: &str, task_id: &str, revision: u64, action: &str) -> String {
    tool_turn(
        call,
        "propose_plan",
        json!({"steps": [{"tool_id": "task_action", "input": {"task_id": task_id,
            "revision": revision, "action": action, "event_id": "e"}}]}),
    )
}

/// Review item 1 (W3e): a session whose record cannot be read decides
/// confirm_changes, never by the global default.
#[test]
fn an_unreadable_session_decides_confirm_changes_not_by_the_global_default() {
    let f = fixture("mode-unreadable", vec![]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    let path = f
        .root
        .join("agent/sessions")
        .join(&f.session)
        .join("session.json");
    *f.replay.on_complete.lock().unwrap() = Some(Box::new(move || {
        fs::write(&path, "{not json").unwrap();
    }));
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "propose_plan", pause_plan(f.revision())),
        text_turn(json!({"text": "已暂停", "claims": []})),
    ]);
    f.host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let card = events_of(&f, "confirm_card", 1).remove(0);
    assert_eq!(
        approval_of(&card),
        manual("confirm_changes", json!(["targets_user_task"]))
    );
    assert_eq!(card["auto_executed"], false);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    f.host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .unwrap();
    events_of(&f, "conclusion", 1);
    let approval = f
        .fake
        .store
        .lock()
        .unwrap()
        .agent_approval(&plan_id)
        .unwrap()
        .unwrap();
    assert_eq!(approval.decided_via, "desktop_card");
    assert_eq!(approval.mode.as_deref(), Some("confirm_changes"));
}

/// Review item 2: one task whose cancel fails does not stop the others; a
/// revision race is retried once; the event says what actually happened.
#[test]
fn stop_all_continues_past_a_failing_task_and_retries_a_revision_race_once() {
    let f = fixture("stop-all-partial", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(
        &f,
        "batch-b",
        json!({}),
        vec![sized("X1", GB), sized("X2", GB), sized("X3", GB)],
        &[],
    );
    let steps: Vec<Value> = ["X1", "X2", "X3"]
        .iter()
        .map(|id| {
            json!({"tool_id": "create_task", "input": {"batch_id": "batch-b",
            "revision": 0, "item_ids": [id], "options": output(&f, "a")}})
        })
        .collect();
    f.replay.turns.lock().unwrap().extend([
        tool_turn("p1", "propose_plan", json!({"steps": steps})),
        text_turn(json!({"text": "三个任务", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载三个", SendContext::default())
        .unwrap();
    let card = events_of(&f, "confirm_card", 1).remove(0);
    f.host
        .plan_approve(
            card["plan_id"].as_str().unwrap(),
            card["snapshot"].clone(),
            None,
        )
        .unwrap();
    events_of(&f, "conclusion", 1);
    let tasks = ["agent-task-0", "agent-task-1", "agent-task-2"];
    for task in tasks {
        assert_eq!(task_status(&f, task), "queued", "{task}");
    }
    f.fake
        .fail_cancel
        .lock()
        .unwrap()
        .insert("agent-task-1".into());
    f.fake
        .conflict_once
        .lock()
        .unwrap()
        .insert("agent-task-2".into());

    let result = f.host.stop_all().unwrap();
    assert_eq!(result.cancelled_tasks, ["agent-task-0", "agent-task-2"]);
    assert_eq!(result.failed_tasks, ["agent-task-1"]);
    assert!(result.cancelled_runs.is_empty() && result.failed_runs.is_empty());
    assert_eq!(task_status(&f, "agent-task-0"), "cancelled");
    assert_eq!(task_status(&f, "agent-task-1"), "queued");
    assert_eq!(
        task_status(&f, "agent-task-2"),
        "cancelled",
        "the cancel lost one revision race and was retried"
    );
    let recorded = events_of(&f, "stop_all", 1);
    assert_eq!(recorded, vec![serde_json::to_value(&result).unwrap()]);
    check("AgentStopAllResult", &recorded[0]);
}

/// Review item 2: with the Store unavailable stop_all still answers, lists
/// what it could not stop and records that.
#[test]
fn stop_all_reports_runs_and_tasks_it_could_not_stop_instead_of_failing() {
    let f = fixture("stop-all-store-down", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-b", json!({}), vec![sized("X1", GB)], &[]);
    let card = propose_create(&f, "batch-b", &["X1"]);
    f.host
        .plan_approve(
            card["plan_id"].as_str().unwrap(),
            card["snapshot"].clone(),
            None,
        )
        .unwrap();
    events_of(&f, "conclusion", 1);
    f.replay.turns.lock().unwrap().push_back(tool_turn(
        "p2",
        "propose_plan",
        pause_plan(f.revision()),
    ));
    let waiting = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    events_of(&f, "confirm_card", 2);
    f.fake.store_down.store(true, Ordering::SeqCst);
    let result = f.host.stop_all().unwrap();
    f.fake.store_down.store(false, Ordering::SeqCst);
    assert_eq!(result.failed_runs, vec![waiting]);
    assert_eq!(result.failed_tasks, ["agent-task-0"]);
    assert!(result.cancelled_runs.is_empty() && result.cancelled_tasks.is_empty());
    let recorded = events_of(&f, "stop_all", 1);
    assert_eq!(recorded, vec![serde_json::to_value(&result).unwrap()]);
}

/// Review item 3: without a permission record the plan is not approved by
/// mode; it waits for the card.
#[test]
fn an_unrecorded_permission_decision_sends_the_plan_to_the_card() {
    let f = fixture("auto-unrecorded", vec![]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    f.host.inject_trace_fault("permission");
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "propose_plan", pause_plan(f.revision())),
        text_turn(json!({"text": "已暂停", "claims": []})),
    ]);
    f.host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let card = events_of(&f, "confirm_card", 1).remove(0);
    assert_eq!(approval_of(&card), manual("full_access", json!([])));
    assert_eq!(card["auto_executed"], false);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    assert!(f
        .fake
        .store
        .lock()
        .unwrap()
        .agent_approval(&plan_id)
        .unwrap()
        .is_none());
    f.host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .unwrap();
    events_of(&f, "conclusion", 1);
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 1);
}

/// Review item 3: when the run record cannot follow an approval by mode
/// already in the Store, the session says so with an error event.
#[test]
fn a_run_record_failure_after_an_approval_by_mode_is_reported() {
    let f = fixture("auto-record-failed", vec![]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    f.host.inject_trace_fault("approval");
    f.replay.turns.lock().unwrap().push_back(tool_turn(
        "c1",
        "propose_plan",
        pause_plan(f.revision()),
    ));
    let run_id = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let error = events_of(&f, "error", 1).remove(0);
    let plan_id = error["plan_id"]
        .as_str()
        .expect("names the plan")
        .to_owned();
    assert_eq!(
        error,
        json!({"run_id": run_id, "code": "record_failed", "status": "failed", "plan_id": plan_id})
    );
    check("AgentErrorPayload", &error);
    assert_eq!(f.run_status(&run_id), "failed");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0, "nothing ran");
    let approval = f
        .fake
        .store
        .lock()
        .unwrap()
        .agent_approval(&plan_id)
        .unwrap()
        .unwrap();
    assert_eq!(approval.decided_via, "auto_by_mode");
}

/// Matrix rows for task controls under auto_approve: an Agent task's pause
/// runs without a click; the user's own task and an archive wait for the
/// card naming why; full access runs them all.
#[test]
fn auto_approve_controls_agent_tasks_but_not_user_tasks_or_archives() {
    let f = fixture("auto-control", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-b", json!({}), vec![sized("X1", GB)], &[]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        create_turn("p1", "batch-b", &["X1"], json!({})),
        text_turn(json!({"text": "已建", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    let cases = [
        (
            "task-a",
            "pause",
            manual("auto_approve", json!(["targets_user_task"])),
        ),
        (
            "agent-task-0",
            "archive",
            manual("auto_approve", json!(["archive_requires_confirm"])),
        ),
        ("agent-task-0", "pause", auto("auto_approve")),
        // Review ruling: update_download_task steers without a click, but a
        // ledger edit waits for the card.
        (
            "agent-task-0",
            "update:mark_completed",
            manual("auto_approve", json!(["ledger_edit_requires_confirm"])),
        ),
        (
            "agent-task-0",
            "update:reacquire_items",
            manual("auto_approve", json!(["ledger_edit_requires_confirm"])),
        ),
        ("agent-task-0", "update:resume_task", auto("auto_approve")),
    ];
    let mut runs = 1;
    for (index, (task, action, expected)) in cases.into_iter().enumerate() {
        let call = format!("c{index}");
        let revision = task_revision(&f, task);
        let turn = match action.strip_prefix("update:") {
            Some(kind) => update_turn(&call, task, revision, kind),
            None => control_turn(&call, task, revision, action),
        };
        f.replay
            .turns
            .lock()
            .unwrap()
            .extend([turn, text_turn(json!({"text": "控制", "claims": []}))]);
        f.host
            .send(&f.session, "控制任务", SendContext::default())
            .unwrap();
        let plan = events_of(&f, "plan_card", runs + 1).remove(runs);
        assert_eq!(approval_of(&plan), expected, "{task} {action}");
        if expected["decision"] == "manual" {
            f.host
                .plan_reject(plan["plan_id"].as_str().unwrap())
                .unwrap();
        }
        runs += 1;
        events_of(&f, "conclusion", runs);
    }
    assert_eq!(task_status(&f, "agent-task-0"), "paused");
    assert_ne!(task_status(&f, "task-a"), "paused");
    // Full access: the user's task and an archive run without a click.
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    for (index, (task, action)) in [("task-a", "pause"), ("agent-task-0", "archive")]
        .into_iter()
        .enumerate()
    {
        f.replay.turns.lock().unwrap().extend([
            control_turn(&format!("f{index}"), task, task_revision(&f, task), action),
            text_turn(json!({"text": "控制", "claims": []})),
        ]);
        f.host
            .send(&f.session, "控制任务", SendContext::default())
            .unwrap();
        runs += 1;
        events_of(&f, "conclusion", runs);
        let plan = events_of(&f, "plan_card", runs).remove(runs - 1);
        assert_eq!(approval_of(&plan), auto("full_access"), "{task} {action}");
    }
    assert_eq!(task_status(&f, "task-a"), "paused");
}

/// Review item 6: a create_task step that created a task names it on its
/// progress card, so the card's cancel can target it.
#[test]
fn a_done_create_step_names_the_task_it_created() {
    let f = fixture("created-ids", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-b", json!({}), vec![sized("X1", GB)], &[]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        create_turn("p1", "batch-b", &["X1"], json!({})),
        text_turn(json!({"text": "完成", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    let done = events_of(&f, "progress_card", 1)
        .into_iter()
        .find(|card| card["status"] == "done")
        .expect("a done step");
    assert_eq!(done["created_task_ids"], json!(["agent-task-0"]), "{done}");
    check("AgentProgressCard", &done);
}

/// Review item 10: each session's own mode decides its plans.
#[test]
fn the_gate_reads_each_sessions_own_mode() {
    let f = fixture("gate-sessions", vec![]);
    let other = f.host.session_create(Some("B".into())).unwrap().session_id;
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "propose_plan", pause_plan(f.revision())),
        text_turn(json!({"text": "已暂停", "claims": []})),
    ]);
    f.host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    let first = events_of(&f, "plan_card", 1).remove(0);
    assert_eq!(approval_of(&first)["decision"], "auto_by_mode");
    f.replay.turns.lock().unwrap().extend([
        control_turn("c2", "task-a", f.revision(), "resume"),
        text_turn(json!({"text": "已继续", "claims": []})),
    ]);
    f.host.send(&other, "继续", SendContext::default()).unwrap();
    let card = events_in(&f, &other, "confirm_card", 1).remove(0);
    assert_eq!(
        approval_of(&card),
        manual("confirm_changes", json!(["targets_user_task"])),
        "the other session follows the global default, not session A"
    );
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 1);
    f.host
        .plan_reject(card["plan_id"].as_str().unwrap())
        .unwrap();
    events_in(&f, &other, "conclusion", 1);
}

#[test]
fn a_refused_reasoning_switch_is_archived_then_resent_once_without_it() {
    use sha2::Digest;
    let f = fixture("reasoning-resend", vec![]);
    f.replay.reasoning_switch.store(true, Ordering::SeqCst);
    f.replay.refuse_switch_once.store(true, Ordering::SeqCst);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text":"队列里有一个任务","claims":[]})));
    let run_id = f
        .host
        .send(&f.session, "看看队列", SendContext::default())
        .unwrap();
    let conclusion = f.wait_for("conclusion").payload;
    check("AgentConclusion", &conclusion);
    assert_eq!(conclusion["reasoning_off_refused"], true);
    assert_eq!(conclusion["text"], "队列里有一个任务");

    let detail = f.host.run_open(&run_id).unwrap();
    let kinds: Vec<_> = detail
        .events
        .iter()
        .map(|event| event.kind.as_str())
        .filter(|kind| kind.starts_with("model_"))
        .collect();
    assert_eq!(
        kinds,
        ["model_call", "model_resend", "model_call", "model_reply"],
        "archive, refusal, archived resend, flagged reply"
    );
    let resend = detail
        .events
        .iter()
        .find(|event| event.kind == "model_resend")
        .unwrap();
    assert_eq!(
        resend.payload,
        json!({"resent_without": "reasoning_switch", "status": 400})
    );
    let reply = detail
        .events
        .iter()
        .find(|event| event.kind == "model_reply")
        .unwrap();
    assert_eq!(reply.payload["reasoning_off_refused"], true);
    assert_eq!(reply.payload["truncated"], false);

    let calls: Vec<_> = detail
        .events
        .iter()
        .filter(|event| event.kind == "model_call")
        .collect();
    let dir = f.root.join(format!("agent/runs/{run_id}/outbound"));
    let bodies: Vec<Value> = calls
        .iter()
        .map(|call| {
            let bytes = fs::read(dir.join(call.payload["file"].as_str().unwrap())).unwrap();
            assert_eq!(
                call.payload["sha256"],
                format!("{:x}", sha2::Sha256::digest(&bytes))
            );
            serde_json::from_slice(&bytes).unwrap()
        })
        .collect();
    assert_eq!(bodies.len(), 2, "both bodies archived before sending");
    assert_eq!(bodies[0]["thinking"], json!({"type": "disabled"}));
    assert!(bodies[1].get("thinking").is_none());
    let mut without = bodies[0].clone();
    without.as_object_mut().unwrap().remove("thinking");
    // W3o: the resend lets the model think, so it has the larger budget.
    assert_eq!(bodies[0]["max_tokens"], 4096);
    assert_eq!(bodies[1]["max_tokens"], 8192);
    without["max_tokens"] = json!(8192);
    assert_eq!(
        without, bodies[1],
        "the resend differs only by the switch and the budget"
    );
    // One recorded turn served one model call: the refusal consumed none.
    assert_eq!(f.replay.seen.lock().unwrap().len(), 1);
    // The successful resend is remembered for the channel, once.
    assert_eq!(f.replay.remembered.load(Ordering::SeqCst), 1);
}

/// W3o item 1 (L4 r9 P2-1: conv1 sent every one of its 5 model calls twice,
/// a 400 then the resend): once a resend succeeded, the channel remembers
/// the refusal. The next run sends no switch and makes no resend, so it
/// does not say the switch was refused (L4 r10 P2-8: every later run said
/// 「该渠道无法关闭思考」 though none sent the switch).
#[test]
fn a_refused_reasoning_switch_is_remembered_and_not_sent_again() {
    let f = fixture("reasoning-remembered", vec![]);
    f.replay.reasoning_switch.store(true, Ordering::SeqCst);
    f.replay.refuse_switch_once.store(true, Ordering::SeqCst);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text":"队列里有一个任务","claims":[]})));
    f.host
        .send(&f.session, "看看队列", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    assert_eq!(f.replay.remembered.load(Ordering::SeqCst), 1);

    // A provider that would refuse again: nothing may reach it with a switch.
    f.replay.refuse_switch_once.store(true, Ordering::SeqCst);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text":"好","claims":[]})));
    let second = f
        .host
        .send(&f.session, "再看看", SendContext::default())
        .unwrap();
    let conclusions = events_of(&f, "conclusion", 2);
    assert_eq!(
        conclusions[0]["reasoning_off_refused"], true,
        "the resent run"
    );
    assert_eq!(
        conclusions[1]["reasoning_off_refused"], false,
        "no switch sent"
    );
    assert!(
        f.replay.refuse_switch_once.load(Ordering::SeqCst),
        "no request carried the switch"
    );
    let detail = f.host.run_open(&second).unwrap();
    let kinds: Vec<_> = detail
        .events
        .iter()
        .map(|event| event.kind.as_str())
        .filter(|kind| kind.starts_with("model_"))
        .collect();
    assert_eq!(kinds, ["model_call"], "no model_resend, no flagged reply");
    let body = first_outbound(&f, &second);
    assert!(body.get("thinking").is_none(), "{body}");
    assert_eq!(body["max_tokens"], 8192, "{body}");
    assert!(!detail.run.reasoning_off_refused);
    let run = serde_json::to_value(
        f.host
            .runs_list(&f.session)
            .unwrap()
            .into_iter()
            .find(|run| run.run_id == second)
            .unwrap(),
    )
    .unwrap();
    check("AgentRunSummary", &run);
    assert_eq!(run["reasoning_off_refused"], false, "{run}");
    assert_eq!(f.replay.remembered.load(Ordering::SeqCst), 1, "not again");
}

/// W3o patch: a run with several model calls on a provider that refuses
/// every switch. The stand-in keeps no memory inside the run, so each call
/// is refused and resent once and the refusal is remembered after each
/// resend; the next run opens with the remembered refusal and makes no
/// resend at all. (Inside one run the production model stops sending the
/// switch after the first resend: `KeyedModel`, insar-agent-model
/// channels tests.)
#[test]
fn several_calls_in_one_run_each_resend_once_and_the_next_run_resends_none() {
    let f = fixture("reasoning-multi-call", vec![]);
    f.replay.reasoning_switch.store(true, Ordering::SeqCst);
    f.replay.refuse_switch_always.store(true, Ordering::SeqCst);
    {
        let mut turns = f.replay.turns.lock().unwrap();
        turns.push_back(tool_turn("c1", "task_overview", json!({})));
        turns.push_back(text_turn(json!({"text":"队列里有一个任务","claims":[]})));
    }
    let first = f
        .host
        .send(&f.session, "看看队列", SendContext::default())
        .unwrap();
    let conclusions = events_of(&f, "conclusion", 1);
    assert_eq!(conclusions[0]["reasoning_off_refused"], true);
    let kinds = |run_id: &str| -> Vec<String> {
        f.host
            .run_open(run_id)
            .unwrap()
            .events
            .iter()
            .map(|event| event.kind.clone())
            .filter(|kind| kind.starts_with("model_"))
            .collect()
    };
    assert_eq!(
        kinds(&first),
        [
            "model_call",
            "model_resend",
            "model_call",
            "model_reply",
            "model_call",
            "model_resend",
            "model_call",
            "model_reply"
        ],
        "each refused call is archived, resent once and flagged"
    );
    assert_eq!(f.replay.remembered.load(Ordering::SeqCst), 2);
    assert_eq!(f.replay.seen.lock().unwrap().len(), 2, "two answered calls");

    {
        let mut turns = f.replay.turns.lock().unwrap();
        turns.push_back(tool_turn("c2", "task_overview", json!({})));
        turns.push_back(text_turn(json!({"text":"好","claims":[]})));
    }
    let second = f
        .host
        .send(&f.session, "再看看", SendContext::default())
        .unwrap();
    let conclusions = events_of(&f, "conclusion", 2);
    assert_eq!(conclusions[1]["reasoning_off_refused"], false);
    assert_eq!(
        kinds(&second),
        ["model_call", "model_call"],
        "the remembered refusal: no switch, no resend, no flagged reply"
    );
    assert_eq!(f.replay.remembered.load(Ordering::SeqCst), 2, "not again");
}

/// W3o item 2 (L4 r9 P1-2): a reply cut off on a channel known not to turn
/// thinking off had the larger budget; its message is the plain one. The
/// run sent no switch, so it is not flagged as refused (L4 r10 P2-8).
#[test]
fn a_truncated_reply_on_a_channel_that_cannot_turn_off_thinking_says_so() {
    let f = fixture(
        "truncated-thinking",
        vec![format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":"队列里"},"finish_reason":"length"}]})
        )],
    );
    f.replay.reasoning_switch.store(true, Ordering::SeqCst);
    f.replay.switch_unsupported.store(true, Ordering::SeqCst);
    let run_id = f
        .host
        .send(&f.session, "看看队列", SendContext::default())
        .unwrap();
    let error = f.wait_for("error").payload;
    check("AgentErrorPayload", &error);
    assert_eq!(error["code"], "model_truncated");
    // The desktop adds its own note from reasoning_off_refused (W5-9); the
    // message is not repeated here.
    assert_eq!(
        error["message_zh"],
        "模型回复超出长度上限，本次运行未生成结论。"
    );
    let body = first_outbound(&f, &run_id);
    assert_eq!(body["max_tokens"], 8192, "{body}");
    let detail = f.host.run_open(&run_id).unwrap();
    assert!(detail.run.truncated);
    assert!(!detail.run.reasoning_off_refused);
}

#[test]
fn a_truncated_reply_fails_the_run_instead_of_concluding() {
    let f = fixture(
        "truncated",
        vec![format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":"队列里"},"finish_reason":"length"}]})
        )],
    );
    let run_id = f
        .host
        .send(&f.session, "看看队列", SendContext::default())
        .unwrap();
    let error = f.wait_for("error").payload;
    check("AgentErrorPayload", &error);
    assert_eq!(error["code"], "model_truncated");
    assert_eq!(error["status"], "failed");
    assert_eq!(
        error["message_zh"],
        "模型回复超出长度上限，本次运行未生成结论。"
    );
    let page = f.host.stream_poll(&f.session, 0).unwrap();
    assert!(
        !page.events.iter().any(|event| event.kind == "conclusion"),
        "a cut-off answer is never a conclusion"
    );
    let detail = f.host.run_open(&run_id).unwrap();
    assert_eq!(detail.run.status, "failed");
    let reply = detail
        .events
        .iter()
        .find(|event| event.kind == "model_reply")
        .expect("the truncated reply is recorded");
    assert_eq!(reply.payload["truncated"], true);
    assert!(!detail.events.iter().any(|event| event.kind == "answer"));
}

/// W5-2g hand-back: the run list says, per run, whether a provider refused
/// the reasoning-off switch and whether a reply was truncated, from the run
/// row the host sealed (the list never reads run records).
#[test]
fn run_summaries_say_whether_reasoning_off_was_refused_or_a_reply_was_truncated() {
    let f = fixture("run-flags", vec![]);
    f.replay.reasoning_switch.store(true, Ordering::SeqCst);
    f.replay.refuse_switch_once.store(true, Ordering::SeqCst);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text":"队列里有一个任务","claims":[]})));
    let resent = f
        .host
        .send(&f.session, "看看队列", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    f.replay.reasoning_switch.store(false, Ordering::SeqCst);
    f.replay.turns.lock().unwrap().push_back(format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"delta":{"content":"队列里"},"finish_reason":"length"}]})
    ));
    let cut = f
        .host
        .send(&f.session, "再看看", SendContext::default())
        .unwrap();
    events_of(&f, "error", 1);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text":"好","claims":[]})));
    let plain = f
        .host
        .send(&f.session, "好的", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);

    let runs: Vec<Value> = f
        .host
        .runs_list(&f.session)
        .unwrap()
        .into_iter()
        .map(|run| serde_json::to_value(run).unwrap())
        .collect();
    assert_eq!(runs.len(), 3);
    for run in &runs {
        check("AgentRunSummary", run);
    }
    let flags = |run_id: &str| {
        let run = runs.iter().find(|run| run["run_id"] == run_id).unwrap();
        (
            run["reasoning_off_refused"].clone(),
            run["truncated"].clone(),
        )
    };
    assert_eq!(flags(&resent), (json!(true), json!(false)), "resent");
    assert_eq!(flags(&cut), (json!(false), json!(true)), "truncated");
    assert_eq!(flags(&plain), (json!(false), json!(false)), "plain");
    let detail = serde_json::to_value(f.host.run_open(&cut).unwrap()).unwrap();
    assert_eq!(detail["run"]["truncated"], true);
    check("AgentRunDetail", &detail);
    // The list answers from the Store row, not from the run record.
    fs::remove_file(f.root.join(format!("agent/runs/{resent}/trace.jsonl"))).unwrap();
    let again = serde_json::to_value(f.host.runs_list(&f.session).unwrap()).unwrap();
    let row = again
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["run_id"] == resent.as_str())
        .unwrap()
        .clone();
    assert_eq!(row["reasoning_off_refused"], true);
}

/// A run a restart interrupted keeps the flag its record shows.
#[test]
fn an_interrupted_run_keeps_its_reasoning_off_refused_flag() {
    let f = fixture("run-flags-restart", vec![]);
    f.replay.reasoning_switch.store(true, Ordering::SeqCst);
    f.replay.refuse_switch_once.store(true, Ordering::SeqCst);
    let revision = f.revision();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(tool_turn("c1", "propose_plan", pause_plan(revision)));
    let run_id = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    f.wait_for("confirm_card");
    let _reopened = AgentHost::open(
        f.root.clone(),
        f.fake.clone(),
        Source(Arc::clone(&f.replay)),
    )
    .unwrap();
    assert_eq!(f.run_status(&run_id), "interrupted");
    let run = serde_json::to_value(&f.host.runs_list(&f.session).unwrap()[0]).unwrap();
    assert_eq!(run["run_id"], run_id.as_str());
    assert_eq!(run["reasoning_off_refused"], true, "{run}");
    assert_eq!(run["truncated"], false, "{run}");
}

/// W5-2f hand-back: the state of the 全部停止 button across sessions. The
/// status counts live runs (running or waiting for a card) and the still
/// active tasks Agent runs created, without reading run records on each call;
/// each change is announced with an `activity_changed` event in the sessions
/// it concerns and in every session that has, or just stopped having, a live
/// run. Reading the status announces nothing.
#[test]
fn activity_status_counts_live_runs_and_active_agent_tasks_and_announces_changes() {
    let f = fixture("activity", vec![]);
    let status = |f: &Fixture| serde_json::to_value(f.host.activity_status().unwrap()).unwrap();
    let idle = json!({"live_runs": 0, "agent_active_tasks": 0, "sessions_with_live_runs": []});
    assert_eq!(status(&f), idle);
    check("AgentActivityStatus", &status(&f));
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-b", json!({}), sar_products(), &[]);
    // Run 1 waits for its create_task card, then creates the task.
    let card = propose_create(&f, "batch-b", &["B1", "B2"]);
    let waiting_here =
        json!({"live_runs": 1, "agent_active_tasks": 0, "sessions_with_live_runs": [f.session]});
    assert_eq!(status(&f), waiting_here);
    f.host
        .plan_approve(
            card["plan_id"].as_str().unwrap(),
            card["snapshot"].clone(),
            None,
        )
        .unwrap();
    events_of(&f, "conclusion", 1);
    let one_task = json!({"live_runs": 0, "agent_active_tasks": 1, "sessions_with_live_runs": []});
    assert_eq!(status(&f), one_task, "the user's own task-a is not counted");
    let announced = events_of(&f, "activity_changed", 1);
    for payload in &announced {
        check("AgentActivityStatus", payload);
    }
    assert_eq!(announced.first(), Some(&waiting_here), "{announced:?}");
    assert_eq!(announced.last(), Some(&one_task), "{announced:?}");
    assert!(
        announced.windows(2).all(|pair| pair[0] != pair[1]),
        "{announced:?}"
    );
    status(&f);
    assert_eq!(events_of(&f, "activity_changed", 1).len(), announced.len());

    // Run 2, in another session, waits for its card.
    let other = f.host.session_create(Some("B".into())).unwrap().session_id;
    f.replay.turns.lock().unwrap().push_back(tool_turn(
        "p2",
        "propose_plan",
        pause_plan(f.revision()),
    ));
    f.host.send(&other, "暂停", SendContext::default()).unwrap();
    events_in(&f, &other, "confirm_card", 1);
    let busy = json!({"live_runs": 1, "agent_active_tasks": 1, "sessions_with_live_runs": [other]});
    assert_eq!(status(&f), busy);
    assert_eq!(
        events_in(&f, &other, "activity_changed", 1).last(),
        Some(&busy)
    );
    // The task is counted from what the host collected when it landed, not
    // by reading run records again.
    let creator = f.host.runs_list(&f.session).unwrap()[0].run_id.clone();
    fs::remove_file(f.root.join(format!("agent/runs/{creator}/trace.jsonl"))).unwrap();
    assert_eq!(status(&f), busy);

    // Stop-all: both sessions hear that nothing is left.
    f.host.stop_all().unwrap();
    assert_eq!(status(&f), idle);
    assert_eq!(events_of(&f, "activity_changed", 1).last(), Some(&idle));
    assert_eq!(
        events_in(&f, &other, "activity_changed", 1).last(),
        Some(&idle)
    );
}

/// W5-2g hand-back: the plan card names the search's source from the
/// service's source list, so it never shows 未知数据源 while the UI's own
/// list is loading; a source the list does not name (or a list that cannot
/// be read) leaves the name null.
#[test]
fn a_search_disclosure_names_its_source_from_the_source_list() {
    let f = fixture("disclosure-name", vec![]);
    *f.fake.sources.lock().unwrap() = json!([
        {"resource_id": "science.sentinel-1.slc.v1", "display_name_zh": "Sentinel-1 SLC"},
    ]);
    f.replay.turns.lock().unwrap().extend([
        search_turn("n1"),
        text_turn(json!({"text": "已检索", "claims": []})),
    ]);
    f.host.send(&f.session, "检索", with_aoi()).unwrap();
    events_of(&f, "conclusion", 1);
    *f.fake.sources.lock().unwrap() = json!([]);
    f.replay.turns.lock().unwrap().extend([
        search_turn("n2"),
        text_turn(json!({"text": "已检索", "claims": []})),
    ]);
    f.host.send(&f.session, "再检索", with_aoi()).unwrap();
    events_of(&f, "conclusion", 2);
    let plans = events_of(&f, "plan_card", 2);
    for plan in &plans {
        check("AgentPlanCard", plan);
    }
    assert_eq!(
        plans[0]["steps"][0]["disclosure"]["display_name_zh"],
        "Sentinel-1 SLC"
    );
    let unnamed = &plans[1]["steps"][0]["disclosure"];
    assert!(
        unnamed.get("display_name_zh").is_some_and(Value::is_null),
        "{unnamed}"
    );
    assert_eq!(unnamed["resource_id"], "science.sentinel-1.slc.v1");
}

/// W3g: a directory the user names in the message stays on this machine.
/// The host takes it out before the request is built, the model sees
/// `[local-reference:message_dir]` and plans `{"ref": "message_dir"}`; the
/// host resolves it (`output_dir_source = from_message`, known to the gate)
/// and no outbound body, run record or journal holds the path: the run
/// record names it by its salted hash only.
#[test]
fn a_directory_named_in_the_message_stays_local_and_is_resolved_for_the_plan() {
    let f = fixture("message-dir", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-s", json!({}), vec![sized("S1", GB)], &[]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    let dir = f.root.join("message-out").to_string_lossy().into_owned();
    assert!(!dir.contains(char::is_whitespace), "{dir}");
    let mut turns = search_then_create(
        &f,
        "md",
        "batch-s",
        &["S1"],
        json!({"output_root": {"ref": "message_dir"}}),
    );
    turns.push(text_turn(json!({"text": "已开始下载", "claims": []})));
    f.replay.turns.lock().unwrap().extend(turns);
    let run_id = f
        .host
        .send(&f.session, &format!("检索并下载到 {dir} 里"), with_aoi())
        .unwrap();
    events_of(&f, "conclusion", 1);

    // The model saw the placeholder, never the directory.
    let seen = f.replay.seen.lock().unwrap().clone();
    let first_user = seen[0].iter().find(|m| m.role == "user").unwrap();
    assert!(
        first_user
            .content
            .starts_with("检索并下载到 [local-reference:message_dir] 里"),
        "{}",
        first_user.content
    );
    // The card shows the directory from the message; the gate knows it, so
    // auto_approve created the task without a click.
    let card = events_of(&f, "confirm_card", 1).remove(0);
    check("AgentConfirmCard", &card);
    assert_eq!(card["snapshot"]["output_dir"], json!(dir), "{card}");
    assert_eq!(card["snapshot"]["output_dir_source"], "from_message");
    assert_eq!(approval_of(&card), auto("auto_approve"), "{card}");
    let created = f.fake.created.lock().unwrap().clone();
    assert_eq!(created[0]["options"]["output_root"], json!(dir));

    // No outbound body, run record, journal or run row holds the path.
    let escaped = dir.replace('\\', "\\\\");
    let run_dir = f.root.join(format!("agent/runs/{run_id}"));
    let mut files = vec![
        run_dir.join("trace.jsonl"),
        run_dir.join("journal.md"),
        run_dir.join("run.json"),
    ];
    let mut outbound: Vec<_> = fs::read_dir(run_dir.join("outbound"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    outbound.sort();
    assert!(outbound.len() >= 2, "{outbound:?}");
    files.extend(outbound);
    for file in &files {
        let text = fs::read_to_string(file).unwrap();
        assert!(
            !text.contains(&dir) && !text.contains(&escaped) && !text.contains("message-out"),
            "{} holds the message directory",
            file.display()
        );
    }
    let body = fs::read_to_string(&files[3]).unwrap();
    assert!(body.contains("[local-reference:message_dir]"), "{body}");
    // The run record names it by its salted hash.
    let request = f
        .host
        .run_open(&run_id)
        .unwrap()
        .events
        .into_iter()
        .find(|event| event.kind == "request")
        .unwrap();
    assert_eq!(
        request.payload["message_dir_sha256"],
        json!(insar_agent::session::path_digest(&run_id, &dir))
    );
}

/// Review 2026-11-02: a Unix path in the message is replaced before the
/// request is built, on every platform. On Unix it is also taken as the
/// run's output directory (`from_message`, known to the gate); on Windows
/// it is not a usable output directory, so the card falls back and the
/// create_task never carries it.
#[test]
fn a_unix_path_in_the_message_is_replaced_and_the_run_goes_on() {
    let f = fixture("message-dir-unix", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-u", json!({}), vec![sized("S1", GB)], &[]);
    // There is no global default since B6; the mode is set per session.
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    let mut turns = search_then_create(
        &f,
        "mu",
        "batch-u",
        &["S1"],
        json!({"output_root": {"ref": "message_dir"}}),
    );
    turns.push(text_turn(json!({"text": "已开始下载", "claims": []})));
    f.replay.turns.lock().unwrap().extend(turns);
    let run_id = f
        .host
        .send(&f.session, "检索并下载到 /home/me/数据 里", with_aoi())
        .unwrap();
    // The first model request (the card that follows it waits on the plan).
    let card = events_of(&f, "confirm_card", 1).remove(0);

    // Whatever the platform, the model never saw the path.
    let seen = f.replay.seen.lock().unwrap().clone();
    for request in &seen {
        let text = request
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("/home/me"), "{text}");
    }
    let first_user = seen[0].iter().find(|m| m.role == "user").unwrap();
    assert!(
        first_user
            .content
            .starts_with("检索并下载到 [local-reference:message_dir] 里\n\n[context]"),
        "{}",
        first_user.content
    );
    #[cfg(unix)]
    {
        events_of(&f, "conclusion", 1);
        // The gate knows the directory, so auto_approve ran without a click.
        assert_eq!(card["snapshot"]["output_dir"], json!("/home/me/数据"));
        assert_eq!(card["snapshot"]["output_dir_source"], "from_message");
        assert_eq!(approval_of(&card), auto("auto_approve"), "{card}");
        let created = f.fake.created.lock().unwrap().clone();
        assert_eq!(created[0]["options"]["output_root"], json!("/home/me/数据"));
        // No outbound body, run record or journal holds the path.
        let run_dir = f.root.join(format!("agent/runs/{run_id}"));
        let mut files = vec![
            run_dir.join("trace.jsonl"),
            run_dir.join("journal.md"),
            run_dir.join("run.json"),
        ];
        let mut outbound: Vec<_> = fs::read_dir(run_dir.join("outbound"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        outbound.sort();
        files.extend(outbound);
        for file in &files {
            let text = fs::read_to_string(file).unwrap();
            assert!(
                !text.contains("/home/me"),
                "{} holds the path",
                file.display()
            );
        }
    }
    #[cfg(windows)]
    {
        // A Unix path is not a usable output directory here: the card asks
        // for one and nothing ran.
        check("AgentConfirmCard", &card);
        assert_eq!(
            approval_of(&card),
            manual("auto_approve", json!(["output_dir_unknown"])),
            "{card}"
        );
        assert!(card["snapshot"]["output_dir"].is_null(), "{card}");
        assert!(card["snapshot"]["output_dir_source"].is_null(), "{card}");
        std::thread::sleep(Duration::from_millis(50));
        assert!(f.fake.created.lock().unwrap().is_empty(), "nothing ran");
        // The run record still names the directory by its salted hash (the
        // syntax was recognised); only its adoption as the output directory
        // is refused here.
        let request = f
            .host
            .run_open(&run_id)
            .unwrap()
            .events
            .into_iter()
            .find(|event| event.kind == "request")
            .unwrap();
        assert_eq!(
            request.payload["message_dir_sha256"],
            json!(insar_agent::session::path_digest(&run_id, "/home/me/数据"))
        );
    }
}

/// Review 2026-11-02: a Unix path glued to Chinese text is taken whole (the
/// model still sees the question around it), on every platform.
#[test]
fn a_path_glued_to_chinese_text_keeps_the_question() {
    let f = fixture("message-dir-glued", vec![]);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好的", "claims": []})));
    f.host
        .send(
            &f.session,
            "下载到/home/me/数据里，谢谢",
            SendContext::default(),
        )
        .unwrap();
    events_of(&f, "conclusion", 1);
    let seen = f.replay.seen.lock().unwrap().clone();
    let first_user = seen[0].iter().find(|m| m.role == "user").unwrap();
    assert!(
        first_user
            .content
            .starts_with("下载到[local-reference:message_dir]，谢谢\n\n[context]"),
        "{}",
        first_user.content
    );
    #[cfg(unix)]
    {
        let run_id = f.host.runs_list(&f.session).unwrap()[0].run_id.clone();
        let request = f
            .host
            .run_open(&run_id)
            .unwrap()
            .events
            .into_iter()
            .find(|event| event.kind == "request")
            .unwrap();
        assert_eq!(
            request.payload["message_dir_sha256"],
            json!(insar_agent::session::path_digest(
                &run_id,
                "/home/me/数据里"
            ))
        );
    }
}

/// A `{"ref": "message_dir"}` with no directory in the message is not a
/// directory: the card must choose one, in every mode, even when a default
/// download directory exists.
#[test]
fn a_message_dir_reference_without_a_directory_in_the_message_needs_the_card() {
    let f = fixture("message-dir-none", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-s", json!({}), vec![sized("S1", GB)], &[]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    let mut turns = search_then_create(
        &f,
        "mn",
        "batch-s",
        &["S1"],
        json!({"output_root": {"ref": "message_dir"}}),
    );
    turns.push(text_turn(json!({"text": "完成", "claims": []})));
    f.replay.turns.lock().unwrap().extend(turns);
    f.host
        .send(&f.session, "检索并下载到我说的目录", with_aoi())
        .unwrap();
    let card = events_of(&f, "confirm_card", 1).remove(0);
    check("AgentConfirmCard", &card);
    assert_eq!(
        approval_of(&card),
        manual("full_access", json!(["output_dir_unknown"])),
        "{card}"
    );
    assert!(card["snapshot"]["output_dir"].is_null(), "{card}");
    assert!(card["snapshot"]["output_dir_source"].is_null(), "{card}");
    std::thread::sleep(Duration::from_millis(50));
    assert!(f.fake.created.lock().unwrap().is_empty(), "nothing ran");
    let request = f
        .host
        .run_open(&f.host.runs_list(&f.session).unwrap()[0].run_id)
        .unwrap()
        .events
        .into_iter()
        .find(|event| event.kind == "request")
        .unwrap();
    assert!(request.payload.get("message_dir_sha256").is_none());
}

/// The result of the last tool call the model saw (the loop wraps it as
/// `{"id", "result"}`).
fn last_tool_result(f: &Fixture) -> Value {
    let seen = f.replay.seen.lock().unwrap();
    let reply = seen.last().unwrap().last().unwrap();
    assert_eq!(reply.role, "tool");
    let wrapped: Value = serde_json::from_str(&reply.content).unwrap();
    wrapped["result"].clone()
}
/// The first request body archived for a run: what left the machine.
fn first_outbound(f: &Fixture, run_id: &str) -> Value {
    let dir = f.root.join(format!("agent/runs/{run_id}/outbound"));
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    serde_json::from_slice(&fs::read(&files[0]).unwrap()).unwrap()
}
fn sorted_strings(value: &Value) -> Vec<String> {
    let mut found: Vec<String> = value
        .as_array()
        .unwrap_or_else(|| panic!("not an array: {value}"))
        .iter()
        .map(|entry| entry.as_str().unwrap().to_owned())
        .collect();
    found.sort();
    found
}
fn plan_refused_events(f: &Fixture, run_id: &str) -> Vec<Value> {
    f.host
        .run_open(run_id)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "plan_refused")
        .map(|event| event.payload)
        .collect()
}
fn context_named(name: &str) -> SendContext {
    SendContext {
        aoi: Some(json!({"name": name, "geometry": {"type": "Polygon",
            "coordinates": [[[103.6195, 31.97], [103.7396, 31.97], [103.7396, 32.0702],
                [103.6195, 32.0702], [103.6195, 31.97]]]}})),
        time_range: None,
        source: None,
        model: None,
        ..SendContext::default()
    }
}
/// `message_zh` is one plain sentence for the user; the correction guidance
/// for the model is `hint_zh`.
fn assert_plain_refusal(label: &str, result: &Value) {
    let message = result["message_zh"].as_str().unwrap_or_default();
    assert!(!message.is_empty(), "{label}: {result}");
    for word in [
        "主机",
        "expected_input",
        "propose_plan",
        "unknown_fields",
        "reason",
    ] {
        assert!(!message.contains(word), "{label}: {word} in {message}");
    }
    assert!(
        result["hint_zh"]
            .as_str()
            .is_some_and(|hint| !hint.is_empty()),
        "{label}: {result}"
    );
}
fn frozen_input(f: &Fixture, plan_id: &str) -> Value {
    let row = f
        .fake
        .store
        .lock()
        .unwrap()
        .agent_plan(plan_id)
        .unwrap()
        .unwrap();
    let intent: Value = serde_json::from_str(&row.plan_json).unwrap();
    intent["steps"][0]["input"].clone()
}

/// L4 r6 (2026-10-26, glm-5.3-flash): the model, shown only `input: object`,
/// invented search fields. The host refuses for a stated reason the model can
/// act on, records it, and never shows a card or reaches the service.
#[test]
fn the_r6_search_plan_is_refused_with_a_structured_reason_and_recorded() {
    let f = fixture("r6-plan-refused", vec![]);
    // propose-plan-call.json, `propose_plan_arguments_parsed`, verbatim.
    let r6 = json!({"steps":[{"input":{"aoi":{"ref":"context_aoi"},"beam_mode":"IW",
        "end":"2026-10-26","flight_direction":"ASCENDING","processing_level":"SLC",
        "product_type":"SLC","sort_by":"acquisition_date","sort_order":"desc",
        "source":"science.sentinel-1.standard.v1","start":"2026-07-26"},
        "label_zh":"检索茂县叠溪区域 2026-07-26 至 2026-10-26 Sentinel-1 SLC 升轨影像（按获取时间倒序，取最新一景）",
        "tool_id":"search_start"}]});
    f.replay.turns.lock().unwrap().extend([
        tool_turn("call_1bf1de2dd60240e49cf0243b", "propose_plan", r6),
        text_turn(json!({"text": "检索条件格式不对，未能发起检索", "claims": []})),
    ]);
    let run_id = f
        .host
        .send(
            &f.session,
            "茂县叠溪 2026-07-26 至 2026-10-26 Sentinel-1 SLC 升轨最新一景",
            context_named("地图绘制矩形"),
        )
        .unwrap();
    f.wait_for("conclusion");
    let result = last_tool_result(&f);
    assert_eq!(result["error"], "plan_refused", "{result}");
    assert_eq!(result["code"], "AGENT_INVALID_REQUEST", "{result}");
    assert_eq!(result["reason"], "input_invalid", "{result}");
    assert_eq!(result["step"], 0, "{result}");
    assert_eq!(result["tool_id"], "search_start", "{result}");
    assert_eq!(
        sorted_strings(&result["unknown_fields"]),
        [
            "beam_mode",
            "end",
            "flight_direction",
            "processing_level",
            "product_type",
            "sort_by",
            "sort_order",
            "source",
            "start"
        ],
        "{result}"
    );
    assert_eq!(
        sorted_strings(&result["missing_fields"]),
        ["resource_id"],
        "{result}"
    );
    assert_plain_refusal("r6", &result);
    assert!(
        result["hint_zh"]
            .as_str()
            .is_some_and(|hint| hint.contains("expected_input")),
        "{result}"
    );
    let expected = &result["expected_input"]["properties"];
    for field in ["resource_id", "aoi", "aoi_name", "time_range", "filters"] {
        assert!(expected.get(field).is_some(), "{field}: {result}");
    }
    assert!(expected.get("request_id").is_none(), "{result}");
    // The run record says why; nothing was planned, shown or submitted.
    let recorded = plan_refused_events(&f, &run_id);
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0]["reason"], "input_invalid");
    assert_eq!(recorded[0]["code"], "AGENT_INVALID_REQUEST");
    assert_eq!(recorded[0]["tool_id"], "search_start");
    let events = f.host.stream_poll(&f.session, 0).unwrap().events;
    assert!(!events
        .iter()
        .any(|e| e.kind == "plan_card" || e.kind == "confirm_card"));
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    let plans: i64 = rusqlite::Connection::open(f.root.join("store.db"))
        .unwrap()
        .query_row("SELECT count(*) FROM agent_plans", [], |r| r.get(0))
        .unwrap();
    assert_eq!(plans, 0);
}

/// The same request filled by the schema the model is shown (no aoi_name;
/// filters given or left out) is a disclosed tier-1 search in every mode.
/// A legacy ASF source gets its registry dataset in the frozen filters; a
/// typed catalogue source keeps exactly what the model gave.
#[test]
fn a_search_filled_by_its_schema_runs_disclosed_in_every_mode() {
    let range = json!({"start": "2026-07-26T00:00:00Z", "end": "2026-10-26T23:59:59Z"});
    let sar = json!({"flight_direction": "ASCENDING", "processing_levels": ["SLC"],
        "beam_modes": ["IW"]});
    let mut sar_frozen = sar.clone();
    sar_frozen["dataset"] = json!("SENTINEL-1");
    let cases = [
        (
            "sar",
            "science.sentinel-1.standard.v1",
            Some(sar),
            sar_frozen,
        ),
        (
            "none",
            "science.sentinel-1.standard.v1",
            None,
            json!({"dataset": "SENTINEL-1"}),
        ),
        (
            "typed",
            "science.nisar.l.rslc.provisional.v1",
            None,
            json!({}),
        ),
    ];
    for (label, mode) in [
        ("confirm", PermissionMode::ConfirmChanges),
        ("auto", PermissionMode::AutoApprove),
        ("full", PermissionMode::FullAccess),
    ] {
        for (variant, resource, filters, frozen) in &cases {
            let f = fixture(&format!("schema-search-{label}-{variant}"), vec![]);
            f.host
                .permission_mode_set_session(&f.session, Some(mode))
                .unwrap();
            let mut input = json!({"resource_id": resource, "aoi": {"ref": "context_aoi"},
                "time_range": range});
            if let Some(filters) = filters {
                input["filters"] = filters.clone();
            }
            f.replay.turns.lock().unwrap().extend([
                tool_turn(
                    "p1",
                    "propose_plan",
                    json!({"steps": [{"tool_id": "search_start", "input": input}]}),
                ),
                text_turn(json!({"text": "已检索", "claims": []})),
            ]);
            f.host
                .send(&f.session, "检索", context_named("地图绘制矩形"))
                .unwrap();
            f.wait_for("conclusion");
            let events = f.host.stream_poll(&f.session, 0).unwrap().events;
            let plan = events
                .iter()
                .find(|e| e.kind == "plan_card")
                .unwrap_or_else(|| {
                    panic!("{label}/{variant}: no plan card; {}", last_tool_result(&f))
                })
                .payload
                .clone();
            assert_eq!(plan["max_tier"], 1, "{label}/{variant}: {plan}");
            assert_eq!(plan["auto_executed"], true, "{label}/{variant}: {plan}");
            // The fixture's source list is empty, so the card names no source.
            assert_eq!(
                plan["steps"][0]["disclosure"],
                json!({"resource_id": resource, "aoi_name": "地图绘制矩形", "time_range": range,
                    "display_name_zh": null}),
                "{label}/{variant}"
            );
            assert_eq!(
                f.fake.submits.load(Ordering::SeqCst),
                1,
                "{label}/{variant}"
            );
            let input = frozen_input(&f, plan["plan_id"].as_str().unwrap());
            assert_eq!(&input["filters"], frozen, "{label}/{variant}");
        }
    }
}

/// Every refusal point names its reason in the tool result and the run record.
#[test]
fn each_plan_refusal_names_its_reason() {
    let search = |aoi: Value| {
        json!({"steps": [{"tool_id": "search_start", "input": {
            "resource_id": "science.sentinel-1.standard.v1", "aoi": aoi}}]})
    };
    let create = |options: Value| {
        json!({"steps": [{"tool_id": "create_task", "input": {
            "batch_id": "batch-b", "revision": 0, "item_ids": ["B1"], "options": options}}]})
    };
    let cases: Vec<(&str, Value, &str, &str)> = vec![
        (
            "steps",
            json!({"steps": []}),
            "steps_invalid",
            "AGENT_INVALID_REQUEST",
        ),
        (
            "read-tool",
            json!({"steps": [{"tool_id": "task_overview", "input": {}}]}),
            "tool_not_plannable",
            "AGENT_INVALID_REQUEST",
        ),
        (
            "forbidden",
            json!({"steps": [{"tool_id": "credential_save_cdse", "input": {"user": "x"}}]}),
            "tool_forbidden",
            "AGENT_TOOL_FORBIDDEN",
        ),
        (
            "no-working-aoi",
            search(json!({"ref": "working_aoi"})),
            "aoi_unavailable",
            "AGENT_INVALID_REQUEST",
        ),
        (
            "raw-aoi",
            json!({"steps": [{"tool_id": "search_start", "input": {
                "resource_id": "science.sentinel-1.standard.v1", "aoi_name": "模型给的多边形",
                "aoi": {"type": "Polygon", "coordinates":
                    [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 30.0]]]}}}]}),
            "input_invalid",
            "AGENT_INVALID_REQUEST",
        ),
        (
            "selection",
            json!({"steps": [{"tool_id": "create_task", "input": {
                "batch_id": "batch-b", "revision": 0, "item_ids": ["B1", "B1"], "options": {}}}]}),
            "selection_invalid",
            "AGENT_INVALID_REQUEST",
        ),
        (
            "output-dir",
            create(json!({"output_root": "relative/out"})),
            "output_dir_invalid",
            "AGENT_INVALID_REQUEST",
        ),
        (
            "options-not-object",
            create(json!("direct")),
            "input_invalid",
            "AGENT_INVALID_REQUEST",
        ),
        (
            "no-task",
            json!({"steps": [{"tool_id": "task_action", "input": {
                "task_id": "task-zz", "revision": 0, "action": "pause"}}]}),
            "task_not_found",
            "AGENT_INVALID_REQUEST",
        ),
    ];
    for (label, plan, reason, code) in cases {
        let f = fixture(&format!("refusal-{label}"), vec![]);
        seed_batch(&f, "batch-b", json!({}), sar_products(), &[]);
        f.replay.turns.lock().unwrap().extend([
            tool_turn("p1", "propose_plan", plan),
            text_turn(json!({"text": "计划未能执行", "claims": []})),
        ]);
        let run_id = f.host.send(&f.session, "执行", with_aoi()).unwrap();
        f.wait_for("conclusion");
        let result = last_tool_result(&f);
        assert_eq!(result["error"], "plan_refused", "{label}: {result}");
        assert_eq!(result["reason"], reason, "{label}: {result}");
        assert_eq!(result["code"], code, "{label}: {result}");
        assert_plain_refusal(label, &result);
        if reason == "output_dir_invalid" {
            // W3g: the hint names the reference, never a path, and survives
            // the nested-result check.
            let hint = result["hint_zh"].as_str().unwrap_or("");
            assert!(hint.contains("message_dir"), "{label}: {result}");
        }
        assert!(result["unknown_fields"].is_array(), "{label}: {result}");
        assert!(result["missing_fields"].is_array(), "{label}: {result}");
        assert!(result["expected_input"].is_object(), "{label}: {result}");
        let recorded = plan_refused_events(&f, &run_id);
        assert_eq!(recorded.len(), 1, "{label}: {recorded:?}");
        assert_eq!(recorded[0]["reason"], reason, "{label}");
        assert_eq!(recorded[0]["code"], code, "{label}");
        assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0, "{label}");
        let plans: i64 = rusqlite::Connection::open(f.root.join("store.db"))
            .unwrap()
            .query_row("SELECT count(*) FROM agent_plans", [], |r| r.get(0))
            .unwrap();
        assert_eq!(plans, 0, "{label}");
    }
}

/// A refusal detail quoting the model's own value (`"a:"`) would read as a
/// local path once nested in the tool message; it is dropped and the run
/// still reaches its conclusion with the reason.
#[test]
fn a_refusal_quoting_the_models_value_still_reaches_the_model() {
    let f = fixture("refusal-quoted", vec![]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "p1",
            "propose_plan",
            json!({"steps": [{"tool_id": "search_start", "input": {
                "resource_id": "science.sentinel-1.standard.v1", "aoi": {"ref": "context_aoi"},
                "time_range": "a:"}}]}),
        ),
        text_turn(json!({"text": "检索条件格式不对", "claims": []})),
    ]);
    let run_id = f.host.send(&f.session, "检索", with_aoi()).unwrap();
    f.wait_for("conclusion");
    let result = last_tool_result(&f);
    assert_eq!(result["reason"], "input_invalid", "{result}");
    assert!(result["detail"].is_null(), "{result}");
    assert_eq!(f.run_status(&run_id), "completed");
}

/// When the run record refuses the full refusal payload, it still records
/// the reason and code.
#[test]
fn a_refusal_the_run_record_cannot_take_still_records_its_reason() {
    let f = fixture("refusal-trace-fault", vec![]);
    f.host.inject_trace_fault("plan_refused");
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "p1",
            "propose_plan",
            json!({"steps": [{"tool_id": "task_overview", "input": {}}]}),
        ),
        text_turn(json!({"text": "计划未能执行", "claims": []})),
    ]);
    let run_id = f
        .host
        .send(&f.session, "执行", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    assert_eq!(last_tool_result(&f)["reason"], "tool_not_plannable");
    let recorded = plan_refused_events(&f, &run_id);
    assert_eq!(
        recorded,
        vec![json!({"reason": "tool_not_plannable", "code": "AGENT_INVALID_REQUEST"})]
    );
}

/// `occurred_at`, like `event_id`, is the host's: a model value is replaced
/// by the proposal time.
#[test]
fn an_update_plan_takes_its_occurred_at_from_the_host() {
    let f = fixture("update-occurred-at", vec![]);
    let revision = f.revision();
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "p1",
            "propose_plan",
            json!({"steps": [{"tool_id": "update_download_task", "input": {
                "task_id": "task-a", "expected_revision": revision,
                "command": {"kind": "pause_task", "item_ids": []},
                "occurred_at": "2000-01-01T00:00:00Z"}}]}),
        ),
        text_turn(json!({"text": "等待确认", "claims": []})),
    ]);
    f.host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    let input = frozen_input(&f, card["plan_id"].as_str().unwrap());
    let occurred = input["occurred_at"].as_str().unwrap();
    assert_ne!(occurred, "2000-01-01T00:00:00Z");
    assert!(is_rfc3339_utc(occurred), "{occurred}");
    assert_eq!(
        input["event_id"],
        format!("{}-0", card["plan_id"].as_str().unwrap())
    );
}

/// Every node of a schema: no `"type"` arrays, and every enum is typed as
/// a string (providers differ in what JSON Schema they accept).
fn assert_compatible_schema(path: &str, node: &Value) {
    match node {
        Value::Object(map) => {
            if let Some(kind) = map.get("type") {
                assert!(kind.is_string(), "{path}: type {kind}");
            }
            if map.contains_key("enum") {
                assert_eq!(map.get("type"), Some(&json!("string")), "{path}: {node}");
            }
            for (key, child) in map {
                assert_compatible_schema(&format!("{path}.{key}"), child);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                assert_compatible_schema(&format!("{path}[{index}]"), child);
            }
        }
        _ => {}
    }
}

/// The propose_plan schema that leaves the machine carries each plan
/// tool's own input fields; host-minted identities are not offered.
#[test]
fn the_propose_plan_schema_the_model_receives_carries_each_tools_input_fields() {
    let f = fixture("plan-schema", vec![]);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run_id = f
        .host
        .send(&f.session, "帮我查下工作区数据", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let body = first_outbound(&f, &run_id);
    let propose = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["function"]["name"] == "propose_plan")
        .expect("propose_plan is offered")["function"]["parameters"]
        .clone();
    assert_compatible_schema("propose_plan", &propose);
    let variants = propose["properties"]["steps"]["items"]["anyOf"]
        .as_array()
        .unwrap_or_else(|| panic!("steps.items has no anyOf: {propose}"))
        .clone();
    let expected: [(&str, &[&str]); 5] = [
        (
            "search_start",
            &["resource_id", "aoi", "aoi_name", "time_range", "filters"],
        ),
        (
            "working_aoi_set",
            &[
                "admin_code",
                "country_code",
                "bbox",
                "display_name",
                "clear",
            ],
        ),
        (
            "create_task",
            &["batch_id", "revision", "item_ids", "options"],
        ),
        ("task_action", &["task_id", "revision", "action"]),
        (
            "update_download_task",
            &["task_id", "expected_revision", "command"],
        ),
    ];
    assert_eq!(variants.len(), expected.len(), "{propose}");
    for (tool_id, fields) in expected {
        let variant = variants
            .iter()
            .find(|v| v["properties"]["tool_id"]["enum"] == json!([tool_id]))
            .unwrap_or_else(|| panic!("no step variant for {tool_id}: {propose}"));
        let input = &variant["properties"]["input"]["properties"];
        for field in fields {
            assert!(input.get(*field).is_some(), "{tool_id}.{field}: {variant}");
        }
        for minted in ["request_id", "event_id", "occurred_at"] {
            assert!(input.get(minted).is_none(), "{tool_id}.{minted}: {variant}");
        }
    }
    let search = &variants
        .iter()
        .find(|v| v["properties"]["tool_id"]["enum"] == json!(["search_start"]))
        .unwrap()["properties"]["input"];
    assert_eq!(
        search["properties"]["aoi"]["properties"]["ref"]["enum"],
        json!(["context_aoi", "working_aoi"])
    );
    assert_eq!(sorted_strings(&search["required"]), ["aoi", "resource_id"]);
    for end in ["start", "end"] {
        assert_eq!(
            search["properties"]["time_range"]["properties"][end]["format"],
            "date-time"
        );
    }
}

/// The system prompt says a `plan_refused` result is the host refusing for
/// its reason (never the user) and names each plan tool's input fields.
#[test]
fn the_system_prompt_explains_plan_refused_and_the_plan_tool_fields() {
    let f = fixture("plan-prompt", vec![]);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run_id = f
        .host
        .send(&f.session, "帮我查下工作区数据", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let body = first_outbound(&f, &run_id);
    let system = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "system")
        .and_then(|message| message["content"].as_str())
        .expect("a system prompt")
        .to_owned();
    for needle in [
        "plan_refused",
        "不是用户拒绝",
        "rejected",
        "reason",
        "message_zh",
        "hint_zh",
        "expected_input",
        "resource_id",
        "source_list",
        "capabilities_get",
        "RFC 3339",
        "2026-07-26T00:00:00Z",
        "time_range",
        "filters",
        "request_id",
        "expected_revision",
        "「工作区」",
    ] {
        assert!(system.contains(needle), "{needle}: {system}");
    }
    assert!(!system.contains("occurred_at"), "{system}");
}

/// One Agent task (`agent-task-0`) created by a full-access run of the
/// fixture's session.
fn create_agent_task(f: &Fixture) {
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(f, "batch-r", json!({}), vec![sized("R1", GB)], &[]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        create_turn("r1", "batch-r", &["R1"], json!({})),
        text_turn(json!({"text": "已建", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载", SendContext::default())
        .unwrap();
    events_of(f, "conclusion", 1);
}
fn reopen(f: &Fixture) -> Arc<AgentHost<Fake, Source>> {
    AgentHost::open(
        f.root.clone(),
        f.fake.clone(),
        Source(Arc::clone(&f.replay)),
    )
    .unwrap()
}

/// Review: after a restart the host finds the tasks earlier Agent runs
/// created from their run records (once, on first use): the status counts
/// them, the gate knows a control of one targets an Agent task, and
/// stop-all cancels it.
#[test]
fn a_reopened_host_rediscovers_the_tasks_agent_runs_created() {
    let f = fixture("rediscover", vec![]);
    create_agent_task(&f);
    let host = reopen(&f);
    assert_eq!(host.activity_status().unwrap().agent_active_tasks, 1);
    // The gate fact: under auto_approve a pause of an Agent task runs
    // without a click (a user task would wait with targets_user_task).
    host.permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        control_turn(
            "r2",
            "agent-task-0",
            task_revision(&f, "agent-task-0"),
            "pause",
        ),
        text_turn(json!({"text": "已暂停", "claims": []})),
    ]);
    host.send(&f.session, "暂停", SendContext::default())
        .unwrap();
    // Read through the host that writes: a second host on the same session
    // directory does not share its file lock and can read a line half written.
    events_via(&host, &f.session, "conclusion", 2);
    let plan = events_via(&host, &f.session, "plan_card", 2).remove(1);
    assert_eq!(approval_of(&plan), auto("auto_approve"), "{plan}");
    assert_eq!(task_status(&f, "agent-task-0"), "paused");
    let result = host.stop_all().unwrap();
    assert_eq!(result.cancelled_tasks, ["agent-task-0"]);
    assert_eq!(task_status(&f, "agent-task-0"), "cancelled");
    assert_eq!(host.activity_status().unwrap().agent_active_tasks, 0);
}

/// Review: a first scan that could not read a run record is not kept, so
/// the next call reads the records again instead of missing the task for
/// the rest of the process.
#[test]
fn a_first_scan_with_an_unreadable_run_record_is_not_cached() {
    let f = fixture("rediscover-unreadable", vec![]);
    create_agent_task(&f);
    let run_id = f.host.runs_list(&f.session).unwrap()[0].run_id.clone();
    let trace = f.root.join(format!("agent/runs/{run_id}/trace.jsonl"));
    let aside = trace.with_extension("aside");
    let host = reopen(&f);
    fs::rename(&trace, &aside).unwrap();
    assert_eq!(host.activity_status().unwrap().agent_active_tasks, 0);
    fs::rename(&aside, &trace).unwrap();
    assert_eq!(host.activity_status().unwrap().agent_active_tasks, 1);
}

/// Review: once the deletion marker is written the session is deleted, so
/// its tasks leave the host's list even when a later step of the deletion
/// fails (here the session directory holds a file the host does not own).
#[test]
fn a_deleted_sessions_tasks_leave_the_list_even_if_the_deletion_fails_later() {
    let f = fixture("delete-cache", vec![]);
    create_agent_task(&f);
    assert_eq!(f.host.activity_status().unwrap().agent_active_tasks, 1);
    fs::write(
        f.root
            .join("agent/sessions")
            .join(&f.session)
            .join("foreign.txt"),
        b"x",
    )
    .unwrap();
    assert!(f.host.session_delete(&f.session).is_err());
    assert!(f
        .root
        .join("agent/sessions")
        .join(format!("{}.deleted", f.session))
        .is_file());
    assert_eq!(f.host.activity_status().unwrap().agent_active_tasks, 0);
}

/// Review: a run whose start fails after another announcement counted it
/// (its placeholder was registered) is announced away, so no session is
/// left believing it is live.
#[test]
fn a_run_that_fails_to_start_is_announced_away() {
    let f = fixture("start-fails", vec![]);
    let other = f.host.session_create(Some("B".into())).unwrap().session_id;
    f.replay.open_delay_ms.store(600, Ordering::SeqCst);
    let starting = {
        let host = Arc::clone(&f.host);
        let session = f.session.clone();
        std::thread::spawn(move || host.send(&session, "暂停", SendContext::default()))
    };
    std::thread::sleep(Duration::from_millis(150));
    f.replay.open_delay_ms.store(0, Ordering::SeqCst);
    // Session B starts a run that waits for its card; its announcement
    // counts A's run, still opening its model.
    f.replay.turns.lock().unwrap().push_back(tool_turn(
        "s1",
        "propose_plan",
        pause_plan(f.revision()),
    ));
    f.host.send(&other, "暂停", SendContext::default()).unwrap();
    let mut live_sessions = vec![f.session.clone(), other.clone()];
    live_sessions.sort();
    let both = json!({"live_runs": 2, "agent_active_tasks": 0,
        "sessions_with_live_runs": live_sessions});
    assert_eq!(
        events_in(&f, &other, "activity_changed", 1).last(),
        Some(&both)
    );
    // A's session disappears while its model opens: its start fails.
    fs::write(
        f.root
            .join("agent/sessions")
            .join(format!("{}.deleted", f.session)),
        b"{}",
    )
    .unwrap();
    assert!(starting.join().unwrap().is_err());
    let only_b = json!({"live_runs": 1, "agent_active_tasks": 0,
        "sessions_with_live_runs": [other]});
    assert_eq!(
        serde_json::to_value(f.host.activity_status().unwrap()).unwrap(),
        only_b
    );
    assert_eq!(
        events_in(&f, &other, "activity_changed", 1).last(),
        Some(&only_b)
    );
}

/// Every request body archived for a run, in order.
fn outbound_bodies(f: &Fixture, run_id: &str) -> Vec<Value> {
    let dir = f.root.join(format!("agent/runs/{run_id}/outbound"));
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
        .iter()
        .map(|file| serde_json::from_slice(&fs::read(file).unwrap()).unwrap())
        .collect()
}
fn message_texts(body: &Value) -> Vec<(String, String)> {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| {
            (
                message["role"].as_str().unwrap_or("").to_owned(),
                message["content"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

/// L4 r7 P1-1 (updated for append-only KV-prefix preservation): the second
/// message of a session replays the exact Turn-1 append-only transcript
/// (`user -> assistant_tool_calls -> tool -> ... -> assistant`) verbatim so
/// the provider's prompt prefix stays byte-identical across turns while
/// carrying the first turn's search batch, read products, plan id, and conclusion.
#[test]
fn a_second_message_carries_the_first_turn_and_its_run_record() {
    let f = fixture("history-two-turns", vec![]);
    seed_batch(&f, "batch-s", json!({}), sar_products(), &[]);
    *f.fake.next_search_batch.lock().unwrap() = Some("batch-s".into());
    f.replay.turns.lock().unwrap().extend([
        search_turn("s1"),
        tool_turn("p1", "search_progress", json!({"request_id": "req-latest"})),
        tool_turn(
            "b1",
            "batch_page",
            json!({"batch_id": "batch-s", "revision": 0, "page": 0, "page_size": 20}),
        ),
        text_turn(json!({"text": "找到 3 景，升轨最新一景是 B1", "claims": []})),
    ]);
    let first = "茂县叠溪这块，先把最新一景 Sentinel-1 SLC 找出来，Ascending。";
    let first_run = f.host.send(&f.session, first, with_aoi()).unwrap();
    events_of(&f, "conclusion", 1);
    let plan_id = events_of(&f, "plan_card", 1)[0]["plan_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好的", "claims": []})));
    let second = f
        .host
        .send(&f.session, "好的，把最新这一景建下载任务。", with_aoi())
        .unwrap();
    events_of(&f, "conclusion", 2);

    let run1_last = outbound_bodies(&f, &first_run).pop().unwrap();
    let run2_body = &outbound_bodies(&f, &second)[0];
    let run1_msgs = run1_last["messages"].as_array().unwrap();
    let run2_msgs = run2_body["messages"].as_array().unwrap();
    assert!(run2_msgs.len() >= run1_msgs.len() + 2);
    // Byte-identical prefix invariance across turns for KV prompt cache
    assert_eq!(
        &run2_msgs[..run1_msgs.len()],
        run1_msgs.as_slice(),
        "Turn 2 must keep Turn 1 outbound prefix byte-identical"
    );

    let messages = message_texts(run2_body);
    let roles: Vec<&str> = messages.iter().map(|(role, _)| role.as_str()).collect();
    assert_eq!(
        roles,
        [
            "system",
            "user",
            "assistant",
            "tool",
            "assistant",
            "tool",
            "assistant",
            "tool",
            "assistant",
            "user"
        ],
        "{messages:?}"
    );
    assert!(messages[1].1.contains(first), "{messages:?}");
    let combined = run2_body["messages"].to_string();
    for fact in [
        "batch-s",
        "B1",
        plan_id.as_str(),
        "找到 3 景，升轨最新一景是 B1",
    ] {
        assert!(combined.contains(fact), "{fact}: {combined}");
    }
    assert!(
        messages.last().unwrap().1.contains("把最新这一景"),
        "{messages:?}"
    );
}

/// Legacy compatibility guard: when a historical run lacks `turn_messages.json`
/// (created before append-only transcripts), Turn 2 falls back seamlessly to
/// the host run-record digest (including Store-enriched orbit directions and approval).
#[test]
fn a_legacy_run_without_turn_messages_falls_back_to_run_record_digest() {
    let f = fixture("history-legacy-digest-fallback", vec![]);
    seed_batch(&f, "batch-s", json!({}), sar_products(), &[]);
    *f.fake.next_search_batch.lock().unwrap() = Some("batch-s".into());
    f.replay.turns.lock().unwrap().extend([
        search_turn("s1"),
        tool_turn("p1", "search_progress", json!({"request_id": "req-latest"})),
        tool_turn(
            "b1",
            "batch_page",
            json!({"batch_id": "batch-s", "revision": 0, "page": 0, "page_size": 20}),
        ),
        text_turn(json!({"text": "找到 3 景，升轨最新一景是 B1", "claims": []})),
    ]);
    let first = "茂县叠溪这块，先把最新一景 Sentinel-1 SLC 找出来，Ascending。";
    let first_run = f.host.send(&f.session, first, with_aoi()).unwrap();
    events_of(&f, "conclusion", 1);
    let plan_id = events_of(&f, "plan_card", 1)[0]["plan_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Simulate a pre-upgrade run on disk without turn_messages.json
    fs::remove_file(
        f.root
            .join("agent")
            .join("runs")
            .join(&first_run)
            .join("turn_messages.json"),
    )
    .unwrap();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好的", "claims": []})));
    let second = f
        .host
        .send(&f.session, "好的，把最新这一景建下载任务。", with_aoi())
        .unwrap();
    events_of(&f, "conclusion", 2);
    let messages = message_texts(&outbound_bodies(&f, &second)[0]);
    let roles: Vec<&str> = messages.iter().map(|(role, _)| role.as_str()).collect();
    assert_eq!(
        roles,
        ["system", "user", "assistant", "user"],
        "{messages:?}"
    );
    let digest = &messages[2].1;
    for fact in [
        "batch-s",
        "B1",
        "ASCENDING",
        "DESCENDING",
        plan_id.as_str(),
        "已批准",
        "找到 3 景，升轨最新一景是 B1",
    ] {
        assert!(digest.contains(fact), "{fact}: {digest}");
    }
}

/// #22 B slice 2: bounded batch reads keep named products in Turn 2 without
/// scanning all 300 candidates, both in append-only mode and legacy digest fallback.
#[test]
fn a_run_record_digest_of_a_large_batch_reads_only_the_named_products() {
    let f = fixture("history-bounded-batch", vec![]);
    let items: Vec<SnapshotItem> = (0..300)
        .map(|index| {
            let mut item = product(
                &format!("P{index:03}"),
                json!({"flight_direction": "ASCENDING"}),
            );
            item.acquisition_datetime = Some(format!("2026-08-{:02}T00:00:00Z", (index % 28) + 1));
            item
        })
        .collect();
    seed_batch(&f, "batch-big", json!({}), items, &[]);
    *f.fake.next_search_batch.lock().unwrap() = Some("batch-big".into());
    f.replay.turns.lock().unwrap().extend([
        search_turn("s1"),
        tool_turn("p1", "search_progress", json!({"request_id": "req-big"})),
        tool_turn(
            "b1",
            "batch_page",
            json!({"batch_id": "batch-big", "revision": 0, "page": 0, "page_size": 20}),
        ),
        text_turn(json!({"text": "已读取前 20 景", "claims": []})),
    ]);
    f.host.send(&f.session, "检索这个区域", with_aoi()).unwrap();
    events_of(&f, "conclusion", 1);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好的", "claims": []})));
    let second = f
        .host
        .send(&f.session, "继续", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);
    let body = &outbound_bodies(&f, &second)[0];
    let messages = message_texts(body);
    let history_serialized: String = messages[1..messages.len() - 1]
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        history_serialized.contains("batch-big"),
        "{history_serialized}"
    );
    assert!(history_serialized.contains("P000"), "{history_serialized}");
    assert!(!history_serialized.contains("P299"), "{history_serialized}");
    let history: usize = messages[1..messages.len() - 1]
        .iter()
        .map(|(_, text)| text.chars().count())
        .sum();
    assert!(history <= 24_000 + 64, "{history}");
}

/// The earlier turns are bounded by hysteresis compaction (`96_000 -> 24_000`):
/// older turns compact once past the trigger while keeping the newest turn intact,
/// omitting verbatim old answers, and ensuring no local directory ever leaves the machine.
#[test]
fn earlier_turns_are_bounded_and_keep_the_newest() {
    let f = fixture("history-budget", vec![]);
    const EARLIER: usize = 20;
    for index in 0..EARLIER {
        f.replay.turns.lock().unwrap().push_back(text_turn(
            json!({"text": format!("第{index}轮的回答"), "claims": []}),
        ));
        let text = if index == EARLIER - 1 {
            r"把结果下载到 D:\EOStage-Test\r7 这个目录".to_owned()
        } else {
            format!("第{index}轮 {}", "长".repeat(7_000))
        };
        f.host
            .send(&f.session, &text, SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", index + 1);
    }
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let last = f
        .host
        .send(&f.session, "继续", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", EARLIER + 1);
    let body = &outbound_bodies(&f, &last)[0];
    let messages = message_texts(body);
    let history: usize = messages[1..messages.len() - 1]
        .iter()
        .map(|(_, text)| text.chars().count())
        .sum();
    assert!(
        history <= insar_agent::history::APPEND_HISTORY_COMPACT_TRIGGER_CHARS + 256,
        "{history}"
    );
    assert!(
        messages[messages.len() - 2].1.contains("第19轮的回答"),
        "the newest turn is kept"
    );
    assert!(
        messages[1].1.contains("已归档压缩并省略原始细节"),
        "{}",
        messages[1].1
    );
    assert!(!messages
        .iter()
        .any(|(_, text)| text.contains("第0轮的回答")));
    assert!(!body.to_string().contains("ScenePort-L4"));
}

fn tool_denied_events(f: &Fixture, run_id: &str) -> Vec<Value> {
    f.host
        .run_open(run_id)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "tool_denied")
        .map(|event| event.payload)
        .collect()
}

/// L4 r7 P1-2: workbench_page with `{}` and batch_page with a task's
/// revision came back as a bare `{"error":"tool_denied"}` and left nothing in
/// the run record, so the model guessed and retried. Each denial now names
/// its reason (with what to fix) and is a `tool_denied` run event.
#[test]
fn a_read_tool_denial_names_its_reason_and_is_recorded() {
    let cases: Vec<(&str, &str, Value, &str, &str)> = vec![
        (
            "workbench-empty",
            "workbench_page",
            json!({}),
            "input_invalid",
            "AGENT_INVALID_REQUEST",
        ),
        (
            "batch-task-revision",
            "batch_page",
            json!({"batch_id": "batch-b", "revision": 5, "page": 0, "page_size": 10}),
            "service_invalid_input",
            "AGENT_SERVICE_REFUSED",
        ),
        (
            "task-page-service-down",
            "task_page",
            json!({"task_id": "task-a", "page_index": 0, "page_size": 10}),
            "service_unavailable",
            "AGENT_SERVICE_REFUSED",
        ),
    ];
    for (label, tool, input, reason, code) in cases {
        let f = fixture(&format!("denied-{label}"), vec![]);
        seed_batch(&f, "batch-b", json!({}), sar_products(), &[]);
        f.replay.turns.lock().unwrap().extend([
            tool_turn("r1", tool, input),
            text_turn(json!({"text": "读取未成功", "claims": []})),
        ]);
        let run_id = f
            .host
            .send(&f.session, "看看结果", SendContext::default())
            .unwrap();
        f.wait_for("conclusion");
        let result = last_tool_result(&f);
        assert_eq!(result["error"], "tool_denied", "{label}: {result}");
        assert_eq!(result["reason"], reason, "{label}: {result}");
        assert_eq!(result["code"], code, "{label}: {result}");
        assert_eq!(result["tool_id"], tool, "{label}: {result}");
        assert_plain_refusal(label, &result);
        let recorded = tool_denied_events(&f, &run_id);
        assert_eq!(recorded.len(), 1, "{label}: {recorded:?}");
        assert_eq!(recorded[0]["reason"], reason, "{label}");
        assert_eq!(recorded[0]["code"], code, "{label}");
        assert_eq!(recorded[0]["tool_id"], tool, "{label}");
        match label {
            "workbench-empty" => {
                assert_eq!(
                    sorted_strings(&result["missing_fields"]),
                    ["page_index", "page_size"],
                    "{result}"
                );
                assert_eq!(
                    sorted_strings(&result["expected_input"]["required"]),
                    ["page_index", "page_size"],
                    "{result}"
                );
            }
            "batch-task-revision" => {
                let hint = result["hint_zh"].as_str().unwrap();
                assert!(hint.contains("revision"), "{result}");
            }
            _ => {}
        }
        assert_eq!(f.run_status(&run_id), "completed", "{label}");
    }
}

/// L4 r7 P1-2: an executed search the service refused came back as
/// `{"code":"denied"}` with no reason and no run-record event.
#[test]
fn an_executed_step_the_service_refuses_names_its_reason_and_is_recorded() {
    let f = fixture("denied-step", vec![]);
    // No batch to freeze: the Fake service refuses the search (Unsupported).
    f.replay.turns.lock().unwrap().extend([
        search_turn("s1"),
        text_turn(json!({"text": "检索未成功", "claims": []})),
    ]);
    let run_id = f.host.send(&f.session, "检索", with_aoi()).unwrap();
    f.wait_for("conclusion");
    let result = last_tool_result(&f);
    assert_eq!(result["status"], "failed", "{result}");
    let step = &result["steps"][0];
    assert_eq!(step["status"], "failed", "{result}");
    assert_eq!(step["error"], "tool_denied", "{result}");
    assert_eq!(step["reason"], "service_unavailable", "{result}");
    assert_eq!(step["code"], "AGENT_SERVICE_REFUSED", "{result}");
    assert_plain_refusal("step", step);
    let recorded = tool_denied_events(&f, &run_id);
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0]["reason"], "service_unavailable");
    assert_eq!(recorded[0]["tool_id"], "search_start");
    assert_eq!(recorded[0]["step"], 0);
    assert_eq!(recorded[0]["plan_id"], result["plan_id"]);
}

/// L4 r7: the full-access run proposed search_start with
/// `flight_direction: ["ASCENDING"]` and then `processing_levels: "SLC"`;
/// both were approved and refused by the service as `denied`. For a legacy
/// ASF source the host now applies the service's own filter type at plan
/// time and refuses with the reason; the accepted shape is planned.
#[test]
fn legacy_search_filters_of_the_wrong_shape_are_refused_at_plan_time() {
    let range = json!({"end": "2026-10-26T23:59:59Z", "start": "2026-07-26T00:00:00Z"});
    let cases = [
        (
            "r7-array-direction",
            json!({"flight_direction": ["ASCENDING"], "processing_levels": ["SLC"]}),
            false,
        ),
        (
            "r7-string-levels",
            json!({"flight_direction": "ASCENDING", "processing_levels": "SLC"}),
            false,
        ),
        (
            "accepted",
            json!({"flight_direction": "ASCENDING", "processing_levels": ["SLC"]}),
            true,
        ),
    ];
    for (label, filters, planned) in cases {
        let f = fixture(&format!("legacy-filters-{label}"), vec![]);
        // run-1a17a678556-15, outbound 000054, propose_plan arguments.
        let plan = json!({"steps": [{"input": {"aoi": {"ref": "context_aoi"}, "aoi_name": "茂县叠溪",
            "filters": filters, "resource_id": "science.sentinel-1.standard.v1", "time_range": range},
            "label_zh": "检索茂县叠溪区域 Sentinel-1 SLC 升轨影像", "tool_id": "search_start"}]});
        f.replay.turns.lock().unwrap().extend([
            tool_turn("p1", "propose_plan", plan),
            text_turn(json!({"text": "完成", "claims": []})),
        ]);
        let run_id = f
            .host
            .send(&f.session, "检索", context_named("地图绘制矩形"))
            .unwrap();
        f.wait_for("conclusion");
        let events = f.host.stream_poll(&f.session, 0).unwrap().events;
        let carded = events.iter().any(|e| e.kind == "plan_card");
        assert_eq!(carded, planned, "{label}: {}", last_tool_result(&f));
        if !planned {
            let result = last_tool_result(&f);
            assert_eq!(result["error"], "plan_refused", "{label}: {result}");
            assert_eq!(result["reason"], "input_invalid", "{label}: {result}");
            assert!(
                result["detail"]
                    .as_str()
                    .is_some_and(|detail| detail.starts_with("filters")),
                "{label}: {result}"
            );
            assert_eq!(plan_refused_events(&f, &run_id).len(), 1, "{label}");
            assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0, "{label}");
        }
    }
}

/// The read tools' schemas name their required fields and bounds, and the
/// system prompt explains a `tool_denied` result.
#[test]
fn read_tool_schemas_name_required_fields_and_the_prompt_explains_tool_denied() {
    let f = fixture("read-schemas", vec![]);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run_id = f
        .host
        .send(&f.session, "帮我查下工作区数据", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let body = first_outbound(&f, &run_id);
    let parameters = |name: &str| {
        body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["function"]["name"] == name)
            .unwrap_or_else(|| panic!("{name} is offered"))["function"]["parameters"]
            .clone()
    };
    for (tool, required) in [
        ("workbench_page", vec!["page_index", "page_size"]),
        ("task_page", vec!["page_index", "page_size", "task_id"]),
        ("task_event_page", vec!["page_size", "task_id"]),
        (
            "batch_page",
            vec!["batch_id", "page", "page_size", "revision"],
        ),
        ("search_progress", vec!["request_id"]),
    ] {
        let schema = parameters(tool);
        assert_compatible_schema(tool, &schema);
        assert_eq!(sorted_strings(&schema["required"]), required, "{tool}");
    }
    assert_eq!(
        parameters("workbench_page")["properties"]["page_size"]["maximum"],
        100
    );
    let system = body["messages"][0]["content"].as_str().unwrap();
    for needle in ["tool_denied", "reason", "hint_zh"] {
        assert!(system.contains(needle), "{needle}: {system}");
    }
}

/// A raw model reply (the r7 replies were prose plus JSON, not a bare object).
fn raw_turn(text: &str) -> String {
    frame(json!({"content": text}))
}
fn trace_kinds(f: &Fixture, run_id: &str) -> Vec<String> {
    f.host
        .run_open(run_id)
        .unwrap()
        .events
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

/// L4 r7 P1-3: all five conclusions had `claims: []` and C2 showed the raw
/// JSON as its text. The r7 reply of conversation C (prose, then a fenced
/// `{text, claims}`) now becomes a conclusion whose text is the answer and
/// whose claims are checked against the run record.
#[test]
fn the_r7_fenced_conclusion_is_cited_against_the_run_record() {
    let f = fixture("conclusion-r7-fenced", vec![]);
    let batch = "batch-unified-50757268-fbb4-4679-91c6-ece9c8687443";
    seed_batch(&f, batch, json!({}), sar_products(), &[]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "b1",
            "batch_page",
            json!({"batch_id": batch, "revision": 0, "page": 0, "page_size": 20}),
        ),
        raw_turn(include_str!(
            "../../insar-agent-model/tests/fixtures/l4r7-c1-fenced.txt"
        )),
    ]);
    let run_id = f
        .host
        .send(&f.session, "找最新一景", SendContext::default())
        .unwrap();
    let conclusion = f.wait_for("conclusion").payload;
    let text = conclusion["text"].as_str().unwrap();
    assert!(text.starts_with("检索完成：茂县叠溪区域"), "{text}");
    assert!(
        !text.contains("```") && !text.contains("\"claims\""),
        "{text}"
    );
    let claims = conclusion["claims"].as_array().unwrap();
    assert_eq!(claims.len(), 2, "{conclusion}");
    let read = f
        .host
        .run_open(&run_id)
        .unwrap()
        .events
        .into_iter()
        .find(|e| e.kind == "tool_result" && e.payload["tool_id"] == "batch_page")
        .unwrap()
        .sequence;
    for claim in claims {
        assert_eq!(claim["verified"], true, "{claim}");
        assert_eq!(claim["refs"][0]["kind"], "batch", "{claim}");
        assert_eq!(claim["refs"][0]["id"], batch, "{claim}");
        assert_eq!(claim["refs"][0]["trace_sequence"], read, "{claim}");
        assert_eq!(claim["refs"][0]["response_count"], 3, "{claim}");
    }
    check("AgentConclusion", &conclusion);
    assert!(!trace_kinds(&f, &run_id).contains(&"conclusion_unparsed".to_owned()));
}

/// A broken JSON reply (starting with `{` or ```` ```json ````) is shown as raw
/// text with no claims and recorded as `conclusion_unparsed` (sizes only),
/// whereas natural Chinese Markdown replies succeed first-class without
/// recording `conclusion_unparsed`.
#[test]
fn an_unparsed_conclusion_shows_the_prose_and_is_recorded() {
    let f = fixture("conclusion-unparsed", vec![]);
    f.replay.turns.lock().unwrap().push_back(raw_turn(
        "<think>先想想</think>{\"text\": \"检索完成，共 8 景，但 JSON 未闭合",
    ));
    let run_id = f
        .host
        .send(&f.session, "找最新一景", SendContext::default())
        .unwrap();
    let conclusion = f.wait_for("conclusion").payload;
    assert_eq!(
        conclusion["text"],
        "{\"text\": \"检索完成，共 8 景，但 JSON 未闭合"
    );
    assert_eq!(conclusion["claims"], json!([]));
    let unparsed: Vec<Value> = f
        .host
        .run_open(&run_id)
        .unwrap()
        .events
        .into_iter()
        .filter(|e| e.kind == "conclusion_unparsed")
        .map(|e| e.payload)
        .collect();
    assert_eq!(unparsed.len(), 1, "{unparsed:?}");
    assert!(unparsed[0]["chars"].as_u64().unwrap() > 0, "{unparsed:?}");
    assert!(unparsed[0].get("text").is_none(), "{unparsed:?}");
    assert_eq!(f.run_status(&run_id), "completed");
}

/// Error events of the fixture's session, waiting until there is one.
fn first_error(f: &Fixture) -> Value {
    f.wait_for("error").payload
}

/// L4 r7 P1-4 + Agent 2.0: the step budget (`max_steps = 8`) ends a run that
/// never concludes as `step_limit` with a Chinese message for the user.
#[test]
fn the_step_budget_ends_a_run_as_step_limit() {
    let f = fixture("step-limit", vec![]);
    let turns: Vec<String> = (0..8)
        .map(|index| {
            let tool = if index % 2 == 0 {
                "service_readiness"
            } else {
                "task_overview"
            };
            tool_turn(&format!("c{index}"), tool, json!({}))
        })
        .collect();
    f.replay.turns.lock().unwrap().extend(turns);
    let run_id = f
        .host
        .send(&f.session, "一直查", SendContext::default())
        .unwrap();
    let error = first_error(&f);
    assert_eq!(error["code"], "step_limit", "{error}");
    assert_eq!(error["status"], "failed");
    assert!(
        error["message_zh"].as_str().is_some_and(|m| !m.is_empty()),
        "{error}"
    );
    assert!(
        error.get("field_class").is_none_or(Value::is_null),
        "{error}"
    );
    check("AgentErrorPayload", &error);
    assert_eq!(f.replay.seen.lock().unwrap().len(), 8);
    assert_eq!(f.run_status(&run_id), "failed");
}

/// The same tool calls twice in a row end the run as repeated_tool_call.
#[test]
fn identical_consecutive_tool_calls_end_as_repeated_tool_call() {
    let f = fixture("repeated-call", vec![]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "task_overview", json!({})),
        tool_turn("c2", "task_overview", json!({})),
    ]);
    f.host
        .send(&f.session, "查", SendContext::default())
        .unwrap();
    let error = first_error(&f);
    assert_eq!(error["code"], "repeated_tool_call", "{error}");
    assert!(
        error["message_zh"].as_str().is_some_and(|m| !m.is_empty()),
        "{error}"
    );
    check("AgentErrorPayload", &error);
}

/// A local path in the user's message is replaced in the body that is sent
/// (it never leaves the machine) and the run goes on instead of failing.
#[test]
fn a_local_path_in_the_message_is_replaced_and_the_run_goes_on() {
    let f = fixture("outbound-redacted", vec![]);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text":"看过了","claims":[]})));
    let run_id = f
        .host
        .send(&f.session, "看看 C:/Users/x/a.tif", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let body = first_outbound(&f, &run_id).to_string();
    assert!(!body.contains("C:/Users"), "{body}");
    assert!(body.contains("看看 [local-reference]"), "{body}");
    assert_eq!(f.run_status(&run_id), "completed");
}

/// A service result whose text reads as a Windows path only once nested in
/// the tool message (`x:"y"` escapes to `x:\"y\"`, `a:` + line break to
/// `a:\n`) no longer stops the run: the host spaces the colon before the
/// check, and the text reaches the model.
#[test]
fn a_tool_result_that_reads_as_a_path_once_nested_still_reaches_the_model() {
    let f = fixture("outbound-nested", vec![]);
    *f.fake.sources.lock().unwrap() = json!([{"resource_id": "r1",
        "display_name_zh": "错误:\"未授权\"", "availability_reason_zh": "第一行:\n第二行"}]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "source_list", json!({})),
        text_turn(json!({"text":"有一个源","claims":[]})),
    ]);
    let run_id = f
        .host
        .send(&f.session, "有哪些源", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let result = last_tool_result(&f);
    assert_eq!(result[0]["display_name_zh"], "错误: \"未授权\"", "{result}");
    assert_eq!(
        result[0]["availability_reason_zh"], "第一行: \n第二行",
        "{result}"
    );
    assert_eq!(f.run_status(&run_id), "completed");
}

/// A body the host cannot repair is not sent: the run fails with
/// outbound_blocked, the part of the body by class (no content), a message
/// for the user, and an outbound_blocked run event.
#[test]
fn a_body_the_host_cannot_repair_is_blocked_with_its_field_class() {
    let f = fixture("outbound-blocked", vec![]);
    f.replay.unsafe_envelope.store(true, Ordering::SeqCst);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text":"不应被调用","claims":[]})));
    let run_id = f
        .host
        .send(&f.session, "你好", SendContext::default())
        .unwrap();
    let error = first_error(&f);
    assert_eq!(error["code"], "outbound_blocked", "{error}");
    assert_eq!(error["field_class"], "request_envelope", "{error}");
    let message = error["message_zh"].as_str().unwrap_or_default();
    assert!(!message.is_empty() && !message.contains("C:/"), "{error}");
    check("AgentErrorPayload", &error);
    assert!(
        f.replay.seen.lock().unwrap().is_empty(),
        "the model was called"
    );
    let recorded: Vec<Value> = f
        .host
        .run_open(&run_id)
        .unwrap()
        .events
        .into_iter()
        .filter(|e| e.kind == "outbound_blocked")
        .map(|e| e.payload)
        .collect();
    assert_eq!(recorded, vec![json!({"field_class": "request_envelope"})]);
    let outbound = f.root.join(format!("agent/runs/{run_id}/outbound"));
    assert_eq!(fs::read_dir(outbound).unwrap().count(), 0);
}

/// An ASF product as the service freezes it (L4 r7 profile, batch
/// batch-unified-50757268-...): the size is `payload.file_size_remote`.
fn asf_product(scene: &str, bytes: Option<u64>) -> SnapshotItem {
    let mut payload = json!({
        "download_url": format!("https://datapool.asf.alaska.edu/SLC/SD/{scene}.zip"),
        "file_name": format!("{scene}.zip"),
    });
    if let Some(bytes) = bytes {
        payload["file_size_remote"] = json!(bytes);
    }
    SnapshotItem {
        item_id: format!("PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::{scene}-SLC"),
        // seed_batch freezes under the fixture provider; the payload is the
        // service's ASF shape.
        provider_id: "asf.sentinel1".into(),
        display_name: format!("{scene}.zip"),
        acquisition_datetime: Some("2026-10-18T11:08:31Z".into()),
        footprint_geojson: None,
        metadata: json!({"flight_direction": "ASCENDING", "processing_level": "SLC",
            "beam_mode": "IW", "platform": "Sentinel-1D"})
        .as_object()
        .unwrap()
        .clone(),
        payload: payload.as_object().unwrap().clone(),
        asset_access: None,
        execution_policy: None,
    }
}

/// L4 r7 P1-5: the ASF create_task card always said 预计大小 未知 and
/// auto_approve always carded with size_unknown, although the search showed
/// 4.66 GB: the host read sizes only from metadata keys, while the service
/// freezes an ASF product's size as payload.file_size_remote.
#[test]
fn an_asf_selection_card_sums_the_frozen_sizes_and_auto_approve_runs_it() {
    let scenes = [
        "S1D_IW_SLC__1SDV_20261018T110831_20261018T110858_005069_009970_518E",
        "S1D_IW_SLC__1SDV_20261006T110831_20261006T110858_004894_0093E1_2C4A",
    ];
    let f = fixture("asf-sizes", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(
        &f,
        "batch-asf",
        json!({}),
        vec![
            asf_product(scenes[0], Some(4_664_020_214)),
            asf_product(scenes[1], Some(4_661_000_000)),
        ],
        &[],
    );
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    let ids: Vec<String> = scenes
        .iter()
        .map(|scene| format!("PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::{scene}-SLC"))
        .collect();
    let items: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut turns = search_then_create(&f, "a", "batch-asf", &items, json!({}));
    turns.push(text_turn(json!({"text": "已建", "claims": []})));
    f.replay.turns.lock().unwrap().extend(turns);
    f.host.send(&f.session, "下载这两景", with_aoi()).unwrap();
    events_of(&f, "conclusion", 1);
    let card = events_of(&f, "confirm_card", 1).remove(0);
    assert_eq!(
        card["snapshot"]["estimated_bytes"],
        4_664_020_214u64 + 4_661_000_000,
        "{card}"
    );
    assert_eq!(approval_of(&card), auto("auto_approve"), "{card}");
    assert_eq!(f.fake.created.lock().unwrap().len(), 1);
}

/// A selection with one product of unknown size keeps the card's size
/// unknown, and auto_approve sends it to the card naming size_unknown.
#[test]
fn an_asf_selection_with_an_unsized_product_stays_size_unknown() {
    let scenes = ["S1D_A_SIZED", "S1D_B_UNSIZED"];
    let f = fixture("asf-size-unknown", vec![]);
    seed_batch(
        &f,
        "batch-asf",
        json!({}),
        vec![
            asf_product(scenes[0], Some(4_664_020_214)),
            asf_product(scenes[1], None),
        ],
        &[],
    );
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    let ids: Vec<String> = scenes
        .iter()
        .map(|scene| format!("PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::{scene}-SLC"))
        .collect();
    let items: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut turns = search_then_create(&f, "a", "batch-asf", &items, json!({}));
    turns.push(text_turn(json!({"text": "等待确认", "claims": []})));
    f.replay.turns.lock().unwrap().extend(turns);
    f.host.send(&f.session, "下载这两景", with_aoi()).unwrap();
    let card = events_of(&f, "confirm_card", 1).remove(0);
    assert!(card["snapshot"]["estimated_bytes"].is_null(), "{card}");
    assert_eq!(
        approval_of(&card),
        manual("auto_approve", json!(["size_unknown"])),
        "{card}"
    );
}

/// Review of 07866b13 (P0-1): a credential typed in a message must not reach
/// the model, neither in this run's request body nor in the next run's
/// history digest (nor in the local outbound archive of either).
#[test]
fn a_credential_in_a_message_never_leaves_now_or_in_later_turns() {
    let f = fixture("credential-redacted", vec![]);
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl";
    let basic = "dXNlcjpwYXNzd29yZA==";
    f.replay.turns.lock().unwrap().extend([
        text_turn(json!({"text": "不要把凭据发给我", "claims": []})),
        text_turn(json!({"text": "好", "claims": []})),
    ]);
    let first = f
        .host
        .send(
            &f.session,
            &format!(
                "这个接口要 Authorization: Bearer {jwt}\n另一个要 Authorization: Basic {basic}"
            ),
            SendContext::default(),
        )
        .unwrap();
    events_of(&f, "conclusion", 1);
    let second = f
        .host
        .send(&f.session, "继续", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);
    for run_id in [&first, &second] {
        for body in outbound_bodies(&f, run_id) {
            let text = body.to_string();
            assert!(
                !text.contains(jwt) && !text.contains(basic),
                "{run_id}: {text}"
            );
        }
    }
    let history = message_texts(&outbound_bodies(&f, &second)[0]);
    assert!(history[1].1.contains("这个接口要"), "{history:?}");
}

/// Synthetic netrc fragments must be scrubbed in both actual requests and titles.
#[test]
fn multiline_netrc_is_redacted_in_messages_and_titles() {
    let f = fixture("netrc-multiline", vec![]);
    let cases = [
        "配置\nmachine x\n login\n FAKE_USER_A\n password\n FAKE_PASS_A\n请检查",
        "配置\ndefault\n account\n FAKE_USER_B\n password\n FAKE_PASS_B\n请检查",
        "machine host.invalid login fakeuser password fakepass 请检查",
        "default login\n fakeuser password\n fakepass 请检查",
    ];
    for (index, text) in cases.iter().enumerate() {
        let title = f.host.session_create(Some((*text).to_owned())).unwrap();
        assert!(
            !title.title.to_ascii_lowercase().contains("fakeuser"),
            "{}",
            title.title
        );
        assert!(
            !title.title.to_ascii_lowercase().contains("fakepass"),
            "{}",
            title.title
        );
        assert!(!title.title.contains("FAKE_USER"), "{}", title.title);
        assert!(!title.title.contains("FAKE_PASS"), "{}", title.title);
        f.replay
            .turns
            .lock()
            .unwrap()
            .push_back(text_turn(json!({"text": "好", "claims": []})));
        let run = f
            .host
            .send(&f.session, text, SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", index + 1);
        let messages = message_texts(&first_outbound(&f, &run));
        let user = messages
            .iter()
            .rev()
            .find(|(role, _)| role == "user")
            .unwrap();
        assert!(
            !user.1.to_ascii_lowercase().contains("fakeuser"),
            "{:?}",
            user
        );
        assert!(
            !user.1.to_ascii_lowercase().contains("fakepass"),
            "{:?}",
            user
        );
        assert!(!user.1.contains("FAKE_USER"), "{:?}", user);
        assert!(!user.1.contains("FAKE_PASS"), "{:?}", user);
        assert!(user.1.contains("请检查"), "{:?}", user);
    }
    for text in [
        "machine learning is useful",
        "login page 打不开",
        "machine learning helps the login page",
        "default settings open the login page",
        "default block: login page stays visible",
        "请打开 default 设置，再看 login page",
    ] {
        let title = f.host.session_create(Some(text.to_owned())).unwrap();
        assert_eq!(title.title, text);
    }
}

/// An unambiguous prose description is preserved, while short values in an
/// actual default block remain protected even without a password field.
#[test]
fn default_netrc_prose_and_short_values_in_messages_and_titles() {
    let f = fixture("default-netrc-boundary", vec![]);
    let cases = [
        (
            "default settings open the login page",
            "default settings open the login page",
        ),
        (
            "default block: login page stays visible",
            "default block: login page stays visible",
        ),
        ("default login alice", "default login [local-reference]"),
        ("default account page", "default account [local-reference]"),
        ("default password is", "default password [local-reference]"),
        (
            "machine learning login page",
            "machine learning login [local-reference]",
        ),
    ];
    for (index, (text, expected)) in cases.iter().enumerate() {
        let title = f.host.session_create(Some((*text).to_owned())).unwrap();
        assert_eq!(title.title, *expected);
        f.replay
            .turns
            .lock()
            .unwrap()
            .push_back(text_turn(json!({"text": "好", "claims": []})));
        let run = f
            .host
            .send(&f.session, text, SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", index + 1);
        let messages = message_texts(&first_outbound(&f, &run));
        let user = messages
            .iter()
            .rev()
            .find(|(role, _)| role == "user")
            .unwrap();
        assert_eq!(
            user.1.split("\n\n[context] ").next(),
            Some(*expected),
            "unexpected outbound user text: {:?}",
            user
        );
    }
}

#[test]
fn netrc_unicode_and_quoted_values_do_not_leave_messages_or_titles() {
    let f = fixture("netrc-unicode-quoted", vec![]);
    let cases = [
        (
            "machine example.invalid login 王五 password 密码串 请检查",
            vec!["王五", "密码串"],
        ),
        (
            "default login \"my name\" password \"secret phrase\" 请检查",
            vec!["my", "name", "secret", "phrase"],
        ),
        (
            "machine example.invalid\nlogin\n\"my name\"\npassword\n\"secret phrase\"\n请检查",
            vec!["my", "name", "secret", "phrase"],
        ),
        ("login alice\npassword bob\n请检查", vec!["alice", "bob"]),
        (
            "default login [redacted] password 密码串 请检查",
            vec!["密码串"],
        ),
        (
            "default login \"用户 名\" password \"口令 串\" 请检查",
            vec!["用户", "口令", "串"],
        ),
    ];
    for (index, (text, secrets)) in cases.iter().enumerate() {
        let title = f.host.session_create(Some((*text).to_owned())).unwrap();
        f.replay
            .turns
            .lock()
            .unwrap()
            .push_back(text_turn(json!({"text":"好","claims":[]})));
        let run = f
            .host
            .send(&f.session, text, SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", index + 1);
        let messages = message_texts(&first_outbound(&f, &run));
        let user = messages
            .iter()
            .rev()
            .find(|(role, _)| role == "user")
            .unwrap();
        for secret in secrets {
            assert!(
                !title.title.contains(secret),
                "title leaked synthetic credential fragment"
            );
            assert!(
                !user.1.contains(secret),
                "message leaked synthetic credential fragment"
            );
        }
        assert!(user.1.contains("请检查"));
    }
}

#[test]
fn netrc_quote_variants_and_placeholder_following_password_are_redacted() {
    let f = fixture("netrc-quote-variants", vec![]);
    let cases = [
        (
            "machine host login “FAKE SMART USER” password “FAKE SMART SECRET” 请检查",
            vec!["FAKE", "SMART", "USER", "SECRET"],
        ),
        (
            "default login ＂FAKE FULLWIDTH USER＂ password ＂FAKE FULLWIDTH SECRET＂ 请检查",
            vec!["FAKE", "FULLWIDTH", "USER", "SECRET"],
        ),
        ("login [redacted] password bob 请检查", vec!["bob"]),
    ];
    for (index, (text, secrets)) in cases.iter().enumerate() {
        let title = f.host.session_create(Some((*text).to_owned())).unwrap();
        f.replay
            .turns
            .lock()
            .unwrap()
            .push_back(text_turn(json!({"text":"好","claims":[]})));
        let run = f
            .host
            .send(&f.session, text, SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", index + 1);
        let user = message_texts(&first_outbound(&f, &run))
            .into_iter()
            .rev()
            .find(|(role, _)| role == "user")
            .unwrap();
        for secret in secrets {
            assert!(
                !title.title.contains(secret),
                "title leaked {secret}: {}",
                title.title
            );
            assert!(
                !user.1.contains(secret),
                "message leaked {secret}: {}",
                user.1
            );
        }
        assert!(user.1.contains("请检查"));
    }
}

#[test]
fn netrc_fullwidth_quotes_hide_all_credential_words() {
    let f = fixture("netrc-fullwidth-quotes", vec![]);
    let text = "default login ＂FAKE FULLWIDTH USER＂ password ＂FAKE FULLWIDTH SECRET＂ 请检查";
    let title = f.host.session_create(Some(text.to_owned())).unwrap();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run = f
        .host
        .send(&f.session, text, SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    let user = message_texts(&first_outbound(&f, &run))
        .into_iter()
        .rev()
        .find(|(role, _)| role == "user")
        .unwrap();
    for fragment in ["FAKE", "FULLWIDTH", "USER", "SECRET"] {
        assert!(
            !title.title.contains(fragment),
            "title leaked synthetic fragment"
        );
        assert!(
            !user.1.contains(fragment),
            "message leaked synthetic fragment"
        );
    }
    assert!(user.1.contains("请检查"));
}

#[test]
fn netrc_headerless_placeholder_keeps_following_short_password_private() {
    let f = fixture("netrc-placeholder-password", vec![]);
    let text = "login [redacted] password bob 请检查";
    let title = f.host.session_create(Some(text.to_owned())).unwrap();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run = f
        .host
        .send(&f.session, text, SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    let user = message_texts(&first_outbound(&f, &run))
        .into_iter()
        .rev()
        .find(|(role, _)| role == "user")
        .unwrap();
    assert!(
        !title.title.contains("bob"),
        "short password leaked from title"
    );
    assert!(
        !user.1.contains("bob"),
        "short password leaked from message"
    );
    assert!(
        user.1.contains("[redacted]"),
        "existing placeholder must stay"
    );
    assert!(user.1.contains("请检查"), "the user's question must stay");
}

#[test]
fn netrc_empty_quoted_value_does_not_swallow_following_fields() {
    let f = fixture("netrc-empty-quoted", vec![]);
    let text = "default login \"\" password FAKE_EMPTY_SECRET 请检查";
    let title = f.host.session_create(Some(text.to_owned())).unwrap();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text":"好","claims":[]})));
    let run = f
        .host
        .send(&f.session, text, SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    let user = message_texts(&first_outbound(&f, &run))
        .into_iter()
        .rev()
        .find(|(role, _)| role == "user")
        .unwrap();
    assert!(!title.title.contains("FAKE_EMPTY_SECRET"));
    assert!(!user.1.contains("FAKE_EMPTY_SECRET"));
    assert!(user.1.contains("请检查"));
}

#[test]
fn netrc_empty_quote_preserves_question_in_titles_and_messages() {
    let f = fixture("netrc-empty-value", vec![]);
    for (index, text) in [
        "machine host login \"\" 请检查",
        "machine host login '' 请检查",
        "default login \"\" account \"\" 请检查",
    ]
    .iter()
    .enumerate()
    {
        let title = f.host.session_create(Some((*text).to_owned())).unwrap();
        f.replay
            .turns
            .lock()
            .unwrap()
            .push_back(text_turn(json!({"text": "好", "claims": []})));
        let run = f
            .host
            .send(&f.session, text, SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", index + 1);
        let user = message_texts(&first_outbound(&f, &run))
            .into_iter()
            .rev()
            .find(|(role, _)| role == "user")
            .unwrap();
        assert!(
            title.title.contains("请检查"),
            "empty quoted value swallowed title question"
        );
        assert!(
            user.1.contains("请检查"),
            "empty quoted value swallowed user question"
        );
    }
}

#[test]
fn netrc_closed_quote_preserves_same_token_suffix_in_titles_and_messages() {
    let f = fixture("netrc-closed-quote-suffix", vec![]);
    let mut count = 0;
    for (opening, closing) in [
        ('"', '"'),
        ('\'', '\''),
        ('“', '”'),
        ('‘', '’'),
        ('＂', '＂'),
        ('＇', '＇'),
        ('`', '`'),
    ] {
        for suffix in ["请检查", "pleaseCheck"] {
            let text = format!("default login {opening}FAKE NAME{closing}{suffix}");
            let title = f.host.session_create(Some(text.clone())).unwrap();
            f.replay
                .turns
                .lock()
                .unwrap()
                .push_back(text_turn(json!({"text": "好", "claims": []})));
            let run = f
                .host
                .send(&f.session, &text, SendContext::default())
                .unwrap();
            count += 1;
            events_of(&f, "conclusion", count);
            let user = message_texts(&first_outbound(&f, &run))
                .into_iter()
                .rev()
                .find(|(role, _)| role == "user")
                .unwrap();
            let expected = format!("default login [local-reference]{suffix}");
            assert_eq!(title.title, expected, "{text:?}");
            assert_eq!(
                user.1.split("\n\n[context] ").next(),
                Some(expected.as_str())
            );
            for sent in [&title.title, &user.1] {
                assert!(!sent.contains("FAKE"), "{text:?} -> {sent:?}");
                assert!(!sent.contains("NAME"), "{text:?} -> {sent:?}");
            }
        }
    }
}

#[test]
fn netrc_closed_quote_suffix_still_redacts_credentials_and_paths() {
    let f = fixture("netrc-closed-quote-private-tail", vec![]);
    let cases = [
        (
            "default login \"fake\"请检查tok_FAKE9876543210abc",
            vec!["fake", "tok_FAKE9876543210abc"],
        ),
        (
            "default login “fake”password=bob 请检查",
            vec!["fake", "bob"],
        ),
        (
            "default login ＂fake＂password \"bob phrase\"请检查",
            vec!["fake", "bob", "phrase"],
        ),
        ("login `fake`password bob 请检查", vec!["fake", "bob"]),
        (
            "default login \"fake\"C:\\fixture\\after.txt 请检查",
            vec!["fake", "fixture", "after.txt"],
        ),
    ];
    for (index, (text, secrets)) in cases.iter().enumerate() {
        let title = f.host.session_create(Some((*text).to_owned())).unwrap();
        f.replay
            .turns
            .lock()
            .unwrap()
            .push_back(text_turn(json!({"text": "好", "claims": []})));
        let run = f
            .host
            .send(&f.session, text, SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", index + 1);
        let user = message_texts(&first_outbound(&f, &run))
            .into_iter()
            .rev()
            .find(|(role, _)| role == "user")
            .unwrap();
        for sent in [&title.title, &user.1] {
            assert!(sent.contains("请检查"), "{text:?} -> {sent:?}");
            for secret in secrets {
                assert!(!sent.contains(secret), "{text:?} -> {sent:?}");
            }
        }
    }
}

/// An unterminated quoted netrc value is masked through the available text.
/// This intentionally accepts over-redaction because choosing a newline or
/// prose boundary would risk exposing the real multi-line credential.
#[test]
fn netrc_unterminated_quote_prefers_over_redaction_to_leakage() {
    let f = fixture("netrc-unterminated-quote", vec![]);
    let text = "machine host login “UNTERMINATED SECRET 请勿泄露\n后续散文";
    let title = f.host.session_create(Some(text.to_owned())).unwrap();
    assert!(!title.title.contains("UNTERMINATED"));
    assert!(!title.title.contains("SECRET"));
}

/// The same fail-closed boundary must hold for the request that actually
/// leaves the host, not only for the stored session title.
#[test]
fn netrc_unterminated_quote_is_redacted_in_title_and_outbound_user_message() {
    let f = fixture("netrc-unterminated-outbound", vec![]);
    let text = "machine host login “UNTERMINATED SECRET 请勿泄露\n后续散文";
    let title = f.host.session_create(Some(text.to_owned())).unwrap();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run = f
        .host
        .send(&f.session, text, SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    let user = message_texts(&first_outbound(&f, &run))
        .into_iter()
        .rev()
        .find(|(role, _)| role == "user")
        .unwrap();
    for fragment in ["UNTERMINATED", "SECRET", "请勿泄露", "后续散文"] {
        assert!(!title.title.contains(fragment), "title leaked {fragment:?}");
        assert!(
            !user.1.contains(fragment),
            "outbound user message leaked {fragment:?}: {user:?}"
        );
    }
}

#[test]
fn netrc_placeholder_between_fields_does_not_expose_short_password() {
    let f = fixture("netrc-placeholder-between-fields", vec![]);
    for (index, text) in [
        "machine host login alice [redacted] password bob 请检查",
        "default login alice\n[redacted]\npasswd bob\n请检查",
        "machine host account alice [local-reference] password bob 请检查",
    ]
    .iter()
    .enumerate()
    {
        let title = f.host.session_create(Some((*text).to_owned())).unwrap();
        f.replay
            .turns
            .lock()
            .unwrap()
            .push_back(text_turn(json!({"text": "好", "claims": []})));
        let run = f
            .host
            .send(&f.session, text, SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", index + 1);
        let user = message_texts(&first_outbound(&f, &run))
            .into_iter()
            .rev()
            .find(|(role, _)| role == "user")
            .unwrap();
        assert!(!title.title.contains("alice"));
        assert!(!title.title.contains("bob"));
        assert!(!user.1.contains("alice"));
        assert!(!user.1.contains("bob"));
        assert!(user.1.contains("请检查"));
        assert!(user.1.contains("[redacted]") || user.1.contains("[local-reference]"));
    }
}

#[test]
fn netrc_isolated_quoted_password_hides_all_words() {
    let f = fixture("netrc-isolated-quoted-password", vec![]);
    for (index, text) in [
        "password \"fake phrase\" 请检查",
        "passwd ‘fake\nphrase’ 请检查",
        "password `fake phrase` 请检查",
    ]
    .iter()
    .enumerate()
    {
        let title = f.host.session_create(Some((*text).to_owned())).unwrap();
        f.replay
            .turns
            .lock()
            .unwrap()
            .push_back(text_turn(json!({"text": "好", "claims": []})));
        let run = f
            .host
            .send(&f.session, text, SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", index + 1);
        let user = message_texts(&first_outbound(&f, &run))
            .into_iter()
            .rev()
            .find(|(role, _)| role == "user")
            .unwrap();
        for fragment in ["fake", "phrase"] {
            assert!(!title.title.contains(fragment));
            assert!(!user.1.contains(fragment));
        }
        assert!(user.1.contains("请检查"));
    }
}

#[test]
fn netrc_backtick_quoted_values_hide_all_words() {
    let f = fixture("netrc-backtick-values", vec![]);
    let text = "machine host login `FAKE BACKTICK USER` password `FAKE BACKTICK SECRET` 请检查";
    let title = f.host.session_create(Some(text.to_owned())).unwrap();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run = f
        .host
        .send(&f.session, text, SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    let user = message_texts(&first_outbound(&f, &run))
        .into_iter()
        .rev()
        .find(|(role, _)| role == "user")
        .unwrap();
    for fragment in ["FAKE", "BACKTICK", "USER", "SECRET"] {
        assert!(!title.title.contains(fragment));
        assert!(!user.1.contains(fragment));
    }
    assert!(user.1.contains("请检查"));
}

/// L4 r8 D4: the user chose the directory on the card, yet the conclusion
/// said 「下载到你在消息中指定的目录」: the create_task result never said
/// where the directory came from. It now carries output_dir_source and a
/// sentence the model can repeat (never the path itself).
#[test]
fn a_create_task_result_tells_the_model_where_its_directory_came_from() {
    // A directory the message names, as this host writes one (review
    // 2026-11-02: the syntax is recognised everywhere; adoption is the
    // host platform's to decide).
    #[cfg(windows)]
    let named = r"下载到 D:\w3l-message\out 这个目录";
    #[cfg(not(windows))]
    let named = "下载到 /w3l/message/out 这个目录";
    // (label, message, the model's options, choose on the card, source)
    let cases = [
        ("chosen", "下载", "none", true, "chosen_on_card"),
        ("default", "下载", "none", false, "default"),
        ("message", named, "message_dir", false, "from_message"),
        ("model", "下载", "model", false, "model_suggested"),
    ];
    for (label, message, options, choose, source) in cases {
        let f = fixture(&format!("dir-source-{label}"), vec![]);
        f.fake.create_ok.store(true, Ordering::SeqCst);
        seed_batch(&f, "batch-b", json!({}), vec![sized("S1", GB)], &[]);
        let options = match options {
            "message_dir" => json!({"output_root": {"ref": "message_dir"}}),
            "model" => output(&f, "model-dir"),
            _ => json!({}),
        };
        f.replay.turns.lock().unwrap().extend([
            create_turn("p1", "batch-b", &["S1"], options),
            text_turn(json!({"text": "完成", "claims": []})),
        ]);
        f.host
            .send(&f.session, message, SendContext::default())
            .unwrap();
        let card = events_of(&f, "confirm_card", 1).remove(0);
        let plan_id = card["plan_id"].as_str().unwrap().to_owned();
        let picked = f.root.join("picked-dir").to_string_lossy().into_owned();
        f.host
            .plan_approve(
                &plan_id,
                card["snapshot"].clone(),
                choose.then(|| picked.clone()),
            )
            .unwrap();
        events_of(&f, "conclusion", 1);
        let result = last_tool_result(&f);
        let step = &result["steps"][0];
        assert_eq!(step["status"], "done", "{label}: {result}");
        assert_eq!(step["output_dir_source"], source, "{label}: {result}");
        let note = step["output_dir_note_zh"].as_str().unwrap_or_default();
        assert!(note.contains("目录"), "{label}: {result}");
        if source == "from_message" {
            assert!(note.contains("消息里指定"), "{label}: {note}");
        } else {
            assert!(note.contains("不是"), "{label}: {note}");
        }
        for path in ["w3l-message", "w3l/message", "model-dir"] {
            assert!(!result.to_string().contains(path), "{label}: {result}");
        }
        assert!(
            !result.to_string().contains("picked-dir"),
            "{label}: {result}"
        );
        assert!(
            !result.to_string().contains("default-out"),
            "{label}: {result}"
        );
    }
}

/// L4 r8 C6/C12: 5 of 6 runs first wrote search filters in the wrong shape
/// (flight_direction as an array, processing_levels as a string) and fixed
/// them only after a plan_refused, one or two calls each time. The schema
/// the model sees and the system prompt now state each field's type.
#[test]
fn the_search_filter_types_are_stated_where_the_model_reads_them() {
    let f = fixture("filter-types", vec![]);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run_id = f
        .host
        .send(&f.session, "帮我查下工作区数据", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let body = first_outbound(&f, &run_id);
    let propose = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["function"]["name"] == "propose_plan")
        .unwrap()["function"]["parameters"]
        .clone();
    let search = propose["properties"]["steps"]["items"]["anyOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["properties"]["tool_id"]["enum"] == json!(["search_start"]))
        .unwrap()
        .clone();
    let filters = search["properties"]["input"]["properties"]["filters"]["description"]
        .as_str()
        .unwrap()
        .to_owned();
    let system = body["messages"][0]["content"].as_str().unwrap().to_owned();
    for text in [&filters, &system] {
        for needle in [
            "flight_direction 是单个字符串",
            "ASCENDING",
            "processing_levels",
            "字符串数组",
            "SLC",
        ] {
            assert!(text.contains(needle), "{needle}: {text}");
        }
    }
    // Plain text: the schema is also nested in refusal results.
    assert!(!filters.contains('"'), "{filters}");
}

/// L4 r8 G1: the history list (and session.json) kept the first message's
/// credential and local path in the session title. The title goes through
/// the same outbound redaction before it is stored.
#[test]
fn a_session_title_keeps_no_credential_or_local_path() {
    let f = fixture("title-redacted", vec![]);
    let cases = [
        (
            "我的令牌是 Authorization: Bearer abcDEF123456…",
            "我的令牌是 [local-reference]…",
            "abcDEF",
        ),
        (
            "把结果下载到 D:\\EOStage-Test\\r8 这个目录，茂县叠溪这块，把最…",
            "把结果下载到 [local-reference] 这个目录，茂县叠溪这块，把最…",
            "ScenePort-L4",
        ),
        (
            "茂县叠溪这块，先把最新一景 Sentinel-1 SLC 找出来，Ascendi…",
            "茂县叠溪这块，先把最新一景 Sentinel-1 SLC 找出来，Ascendi…",
            "",
        ),
        // W3n (L4 r9 C15): bare secrets with no header, as the desktop sends
        // the title (the first 40 characters and `…`, which cuts the JWT).
        (
            "我的令牌是 `tok_FAKE9876543210abc`，配置在 C:\\Use…",
            "我的令牌是 `[local-reference]`，配置在 [local-reference]…",
            "tok_FAKE",
        ),
        (
            "令牌【eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmYWtl…",
            "令牌【[local-reference]…",
            "eyJhbGci",
        ),
        (
            "令牌【eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmYWtlLXVzZXIifQ.FAKEsigABC123xyz】，脚本在…",
            "令牌【[local-reference]】，脚本在…",
            "FAKEsig",
        ),
    ];
    for (title, expected, gone) in cases {
        let created = f.host.session_create(Some(title.into())).unwrap();
        assert_eq!(created.title, expected, "{title}");
        let stored = fs::read_to_string(
            f.root
                .join("agent/sessions")
                .join(&created.session_id)
                .join("session.json"),
        )
        .unwrap();
        if !gone.is_empty() {
            assert!(!stored.contains(gone), "{stored}");
        }
        let listed = f
            .host
            .sessions_list()
            .unwrap()
            .into_iter()
            .find(|s| s.session_id == created.session_id)
            .unwrap();
        assert_eq!(listed.title, expected);
    }
}

/// The system prompt names every output_dir_source a create_task result may
/// carry, model_suggested included (W3l review P3).
#[test]
fn the_system_prompt_names_every_output_dir_source() {
    let f = fixture("prompt-dir-sources", vec![]);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run_id = f
        .host
        .send(&f.session, "帮我查下工作区数据", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let body = first_outbound(&f, &run_id);
    let system = body["messages"][0]["content"].as_str().unwrap();
    for source in [
        "default",
        "chosen_on_card",
        "from_message",
        "model_suggested",
    ] {
        assert!(system.contains(source), "{source}: {system}");
    }
}

/// W3o item 3 (L4 r9): the answer is written for the user in Chinese, with no
/// internal ids (task-…, run-…, plan-…) and no English status words.
#[test]
fn the_system_prompt_asks_for_chinese_without_internal_ids_or_status_words() {
    let f = fixture("prompt-no-ids", vec![]);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run_id = f
        .host
        .send(&f.session, "帮我查下工作区数据", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let body = first_outbound(&f, &run_id);
    let system = body["messages"][0]["content"].as_str().unwrap();
    for part in [
        "不要复述内部编号",
        "task-",
        "run-",
        "plan-",
        "英文状态词",
        "queued",
        "running",
        "排队中",
        "传输中",
        "已暂停",
        "已完成",
        "已失败",
        "已取消",
        "已中断",
    ] {
        assert!(system.contains(part), "{part}: {system}");
    }
}

/// L4 r10 P2-5 (C2-b): a create_task plan naming `{"ref": "message_dir"}`
/// in a message without a directory was rejected on the card, and the model
/// said the directory "指向你指定的本机目录". A rejected (or expired) plan's
/// result now says nothing was created and where the plan's directory came
/// from, as an executed create_task result does.
#[test]
fn a_rejected_create_task_plan_says_where_its_directory_came_from() {
    let f = fixture("rejected-dir-note", vec![]);
    seed_batch(&f, "batch-b", json!({}), vec![sized("S1", GB)], &[]);
    f.replay.turns.lock().unwrap().extend([
        create_turn(
            "p1",
            "batch-b",
            &["S1"],
            json!({"output_root": {"ref": "message_dir"}}),
        ),
        text_turn(json!({"text": "计划被拒绝", "claims": []})),
    ]);
    f.host
        .send(&f.session, "再把那一景建下载任务。", SendContext::default())
        .unwrap();
    let card = events_of(&f, "confirm_card", 1).remove(0);
    f.host
        .plan_reject(card["plan_id"].as_str().unwrap())
        .unwrap();
    events_of(&f, "conclusion", 1);
    let seen = f.replay.seen.lock().unwrap();
    let tool = seen
        .last()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message.role == "tool")
        .expect("the plan result reached the model");
    let result: Value = serde_json::from_str(&tool.content).unwrap();
    let result = &result["result"];
    assert_eq!(result["status"], "rejected", "{result}");
    assert!(result.get("output_dir_source").is_some(), "{result}");
    assert_ne!(result["output_dir_source"], "from_message", "{result}");
    let note = result["output_dir_note_zh"].as_str().unwrap_or_default();
    assert!(note.contains("没有建任务"), "{result}");
    assert!(note.contains("不要说成用户在消息里指定的目录"), "{result}");
}

/// L4 r10 P2-4 (probes 3, 4, 8, 10): the model repeated `[local-reference]`
/// in its answer (「令牌【[local-reference]]」). The system prompt says the
/// placeholders are the host's and asks for a Chinese description instead.
#[test]
fn the_system_prompt_says_not_to_repeat_local_placeholders() {
    let f = fixture("prompt-placeholders", vec![]);
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let run_id = f
        .host
        .send(&f.session, "帮我查下工作区数据", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let body = first_outbound(&f, &run_id);
    let system = body["messages"][0]["content"].as_str().unwrap();
    for part in [
        "[local-reference]",
        "[local]",
        "[已省略]",
        "本机占位符",
        "不要复述这些标记",
        "你提供的本机文件",
    ] {
        assert!(system.contains(part), "{part}: {system}");
    }
}

/// W3m item 1: the confirm card snapshot names its source for the user
/// (`source_display_name_zh`, the same source-list name the search plan
/// card discloses); null when the frozen batch maps to no listed source.
/// The field is part of the frozen snapshot, so the card echoed back on
/// approval still matches.
#[test]
fn a_confirm_card_snapshot_names_its_source() {
    let f = fixture("snapshot-source-name", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    *f.fake.sources.lock().unwrap() = json!([
        {"resource_id": "science.sentinel-1.standard.v1", "display_name_zh": "Sentinel-1 标准产品"},
    ]);
    {
        let item = SnapshotItem {
            item_id: "S1-A".into(),
            provider_id: "asf.catalog".into(),
            display_name: "S1-A.zip".into(),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Default::default(),
            payload: json!({"file_name": "S1-A.zip", "file_size_remote": 10})
                .as_object()
                .unwrap()
                .clone(),
            asset_access: None,
            execution_policy: None,
        };
        f.fake
            .store
            .lock()
            .unwrap()
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: true,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "batch-asf".into(),
                    provider_id: "asf.catalog".into(),
                    created_at: "2026-10-27T09:00:00Z".into(),
                    aoi_name: "叠溪".into(),
                    aoi_geojson: None,
                    query: json!({"dataset": "SENTINEL-1"})
                        .as_object()
                        .unwrap()
                        .clone(),
                    search_stats: None,
                    candidates: vec![item],
                },
                workbench: SearchWorkbenchState {
                    batch_id: "batch-asf".into(),
                    checked_item_ids: vec!["S1-A".into()],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            })
            .unwrap();
    }
    seed_batch(&f, "batch-other", json!({}), vec![sized("O1", GB)], &[]);
    f.replay.turns.lock().unwrap().extend([
        create_turn("p1", "batch-asf", &["S1-A"], json!({})),
        text_turn(json!({"text": "完成", "claims": []})),
        create_turn("p2", "batch-other", &["O1"], json!({})),
        text_turn(json!({"text": "完成", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载", SendContext::default())
        .unwrap();
    let card = events_of(&f, "confirm_card", 1).remove(0);
    assert_eq!(
        card["snapshot"]["source_display_name_zh"], "Sentinel-1 标准产品",
        "{card}"
    );
    check("AgentConfirmCard", &card);
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    f.host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .unwrap();
    events_of(&f, "conclusion", 1);
    assert_eq!(
        f.fake.created.lock().unwrap().len(),
        1,
        "the echoed card matched"
    );

    f.host
        .send(&f.session, "再下载", SendContext::default())
        .unwrap();
    let other = events_of(&f, "confirm_card", 2).remove(1);
    assert!(
        other["snapshot"]["source_display_name_zh"].is_null(),
        "{other}"
    );
    check("AgentConfirmCard", &other);
}

/// A replayed turn whose stream ends with the provider's usage chunk
/// (OpenAI-compatible `stream_options.include_usage`).
fn turn_with_usage(turn: Value, prompt: u64, completion: u64) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices": [{"delta": turn}]}),
        json!({"choices": [], "usage": {"prompt_tokens": prompt, "completion_tokens": completion,
            "total_tokens": prompt + completion}})
    )
}

/// W3m item 3 (L4 r8 P2 「Token 不上报」): the usage a channel reports for each
/// reply is kept in the run record, on the model_reply event after that
/// call's model_call (model_call is written before the request is sent, so
/// it cannot hold the reply's usage). A reply without usage adds nothing.
#[test]
fn each_reply_s_usage_is_kept_in_the_run_record() {
    let f = fixture("usage-record", vec![]);
    f.replay.turns.lock().unwrap().extend([
        turn_with_usage(
            json!({"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "task_overview", "arguments": "{}"}}]}),
            100,
            10,
        ),
        turn_with_usage(json!({"content": json!({"text": "好", "claims": []}).to_string()}), 120, 30),
    ]);
    let run_id = f
        .host
        .send(&f.session, "看看队列", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let usages: Vec<Value> = f
        .host
        .run_open(&run_id)
        .unwrap()
        .events
        .into_iter()
        .filter(|e| e.kind == "model_reply")
        .map(|e| e.payload["usage"].clone())
        .collect();
    assert_eq!(
        usages,
        vec![
            json!({"prompt": 100, "completion": 10, "total": 110}),
            json!({"prompt": 120, "completion": 30, "total": 150}),
        ]
    );
    // A channel that reports no usage: no model_reply event for it.
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好", "claims": []})));
    let plain = f
        .host
        .send(&f.session, "好的", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);
    assert!(!f
        .host
        .run_open(&plain)
        .unwrap()
        .events
        .iter()
        .any(|e| e.kind == "model_reply"));
    // The run list sums each run's usage from its sealed row (schema 37).
    let runs: Vec<Value> = f
        .host
        .runs_list(&f.session)
        .unwrap()
        .into_iter()
        .map(|run| serde_json::to_value(run).unwrap())
        .collect();
    for run in &runs {
        check("AgentRunSummary", run);
    }
    let total =
        |id: &str| runs.iter().find(|run| run["run_id"] == id).unwrap()["usage_total"].clone();
    assert_eq!(
        total(&run_id),
        json!({"prompt": 220, "completion": 40, "total": 260})
    );
    assert!(total(&plain).is_null(), "{runs:?}");
    let opened = serde_json::to_value(f.host.run_open(&run_id).unwrap()).unwrap();
    assert_eq!(opened["run"]["usage_total"]["total"], 260);
    // The list answers from the Store row, not from the run record.
    fs::remove_file(f.root.join(format!("agent/runs/{run_id}/trace.jsonl"))).unwrap();
    let again = serde_json::to_value(f.host.runs_list(&f.session).unwrap()).unwrap();
    let row = again
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["run_id"] == run_id.as_str())
        .unwrap()
        .clone();
    assert_eq!(row["usage_total"]["total"], 260);
}

/// W3m review: usage a channel reports is not trusted; a count of 2^63 or
/// more never fails the reply, the seal or the run list (sums saturate and
/// the Store keeps the largest value it holds).
#[test]
fn an_untrusted_usage_count_never_fails_the_run() {
    let f = fixture("usage-huge", vec![]);
    f.replay.turns.lock().unwrap().push_back(format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices": [{"delta": {"content": json!({"text": "好", "claims": []}).to_string()}}]}),
        json!({"choices": [], "usage": {"prompt_tokens": 9223372036854775813u64,
            "completion_tokens": 9223372036854775808u64}})
    ));
    let run_id = f
        .host
        .send(&f.session, "看看", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    assert_eq!(f.run_status(&run_id), "completed");
    let run = serde_json::to_value(&f.host.runs_list(&f.session).unwrap()[0]).unwrap();
    let max = i64::MAX as u64;
    assert_eq!(
        run["usage_total"],
        json!({"prompt": max, "completion": max, "total": max * 2}),
        "{run}"
    );
}

/// W3m review: a run a restart interrupted keeps the usage its record shows
/// (mark_interrupted -> set_interrupted_trace_hash), as it keeps its flags.
#[test]
fn an_interrupted_run_keeps_its_usage() {
    let f = fixture("usage-restart", vec![]);
    let revision = f.revision();
    f.replay.turns.lock().unwrap().push_back(turn_with_usage(
        json!({"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "propose_plan",
            "arguments": pause_plan(revision).to_string()}}]}),
        50,
        5,
    ));
    let run_id = f
        .host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    f.wait_for("confirm_card");
    let _reopened = AgentHost::open(
        f.root.clone(),
        f.fake.clone(),
        Source(Arc::clone(&f.replay)),
    )
    .unwrap();
    assert_eq!(f.run_status(&run_id), "interrupted");
    let run = serde_json::to_value(&f.host.runs_list(&f.session).unwrap()[0]).unwrap();
    assert_eq!(run["run_id"], run_id.as_str());
    assert_eq!(
        run["usage_total"],
        json!({"prompt": 50, "completion": 5, "total": 55}),
        "{run}"
    );
}

/// W3m review: the card's source name for a catalogue (STAC) batch, found by
/// provider, logical dataset and collection, and for a task control card
/// (the task's frozen snapshot).
#[test]
fn catalogue_and_task_control_cards_name_their_source() {
    let f = fixture("snapshot-source-catalogue", vec![]);
    *f.fake.sources.lock().unwrap() = json!([
        {"resource_id": "science.sentinel-2.l2a.v1", "display_name_zh": "Sentinel-2 L2A"},
    ]);
    let item = SnapshotItem {
        item_id: "S2-A".into(),
        provider_id: "microsoft.planetary-computer".into(),
        display_name: "S2-A".into(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Default::default(),
        payload: json!({"file_name": "S2-A.tif", "file_size_remote": 10})
            .as_object()
            .unwrap()
            .clone(),
        asset_access: None,
        execution_policy: None,
    };
    {
        let mut store = f.fake.store.lock().unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: true,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "batch-s2".into(),
                    provider_id: "microsoft.planetary-computer".into(),
                    created_at: "2026-10-27T09:00:00Z".into(),
                    aoi_name: "叠溪".into(),
                    aoi_geojson: None,
                    query: json!({"logical_dataset": "optical.sentinel-2.l2a",
                        "native_collection_id": "sentinel-2-l2a"})
                    .as_object()
                    .unwrap()
                    .clone(),
                    search_stats: None,
                    candidates: vec![item],
                },
                workbench: SearchWorkbenchState {
                    batch_id: "batch-s2".into(),
                    checked_item_ids: vec!["S2-A".into()],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            })
            .unwrap();
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: "task-s2".into(),
                batch_id: "batch-s2".into(),
                selected_item_ids: vec!["S2-A".into()],
                checked_item_ids: vec!["S2-A".into()],
                page_index: 0,
                page_size: 24,
                output_root: f.root.join("out-s2"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-10-27T09:00:01Z".into(),
            })
            .unwrap();
    }
    let revision = task_revision(&f, "task-s2");
    f.replay.turns.lock().unwrap().extend([
        create_turn("p1", "batch-s2", &["S2-A"], json!({})),
        text_turn(json!({"text": "等待", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载", SendContext::default())
        .unwrap();
    let card = events_of(&f, "confirm_card", 1).remove(0);
    assert_eq!(
        card["snapshot"]["source_display_name_zh"], "Sentinel-2 L2A",
        "{card}"
    );
    check("AgentConfirmCard", &card);
    f.host
        .plan_reject(card["plan_id"].as_str().unwrap())
        .unwrap();
    events_of(&f, "conclusion", 1);
    f.replay.turns.lock().unwrap().extend([
        control_turn("p2", "task-s2", revision, "pause"),
        text_turn(json!({"text": "等待", "claims": []})),
    ]);
    f.host
        .send(&f.session, "暂停", SendContext::default())
        .unwrap();
    let control = events_of(&f, "confirm_card", 2).remove(1);
    assert_eq!(
        control["snapshot"]["source_display_name_zh"], "Sentinel-2 L2A",
        "{control}"
    );
    check("AgentConfirmCard", &control);
}

#[test]
fn sessions_are_renamed_pinned_archived_and_listed_pinned_first() {
    let f = fixture("organize", vec![]);
    let other = f.host.session_create(Some("第二个".into())).unwrap();
    let renamed = f
        .host
        .session_rename(&f.session, "Token: abcDEF123456 叠溪")
        .unwrap();
    assert!(!renamed.title.contains("abcDEF"), "{}", renamed.title);
    assert!(matches!(
        f.host.session_rename(&f.session, "   "),
        Err(HostError::Invalid)
    ));

    f.host.session_set_pinned(&f.session, true).unwrap();
    let listed = f.host.sessions_list().unwrap();
    assert_eq!(listed[0].session_id, f.session);
    assert!(listed[0].pinned_at.is_some());

    f.host
        .session_set_archived(&other.session_id, true)
        .unwrap();
    let ids = |sessions: Vec<insar_agent::session::SessionSummary>| {
        sessions
            .into_iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>()
    };
    assert!(!ids(f.host.sessions_list().unwrap()).contains(&other.session_id));
    assert!(ids(f.host.sessions_list_with(true).unwrap()).contains(&other.session_id));
    f.host
        .session_set_archived(&other.session_id, false)
        .unwrap();
    f.host.session_set_pinned(&f.session, false).unwrap();
    assert!(ids(f.host.sessions_list().unwrap()).contains(&other.session_id));
    assert!(f.host.sessions_list().unwrap()[0].pinned_at.is_none());
}

#[test]
fn workspaces_group_sessions_and_deleting_one_unfiles_them() {
    let f = fixture("workspaces", vec![]);
    let folder = f.root.join("ws-folder").to_string_lossy().into_owned();
    let workspace = f
        .host
        .workspace_create("叠溪滑坡", Some(folder.clone()))
        .unwrap();
    assert_eq!(workspace.folder.as_deref(), Some(folder.as_str()));
    assert!(matches!(
        f.host
            .workspace_create("相对路径", Some("relative/dir".into())),
        Err(HostError::Invalid)
    ));
    let filed = f
        .host
        .session_create_in(
            Some("在工作区里".into()),
            Some(workspace.workspace_id.clone()),
        )
        .unwrap();
    assert_eq!(
        filed.workspace_id.as_deref(),
        Some(workspace.workspace_id.as_str())
    );
    f.host
        .session_move(&f.session, Some(&workspace.workspace_id))
        .unwrap();
    assert!(matches!(
        f.host.session_move(&f.session, Some("workspace-missing")),
        Err(HostError::NotFound)
    ));
    let renamed = f
        .host
        .workspace_update(&workspace.workspace_id, Some("叠溪"), Some(None))
        .unwrap();
    assert_eq!(renamed.name, "叠溪");
    assert!(renamed.folder.is_none());
    assert_eq!(f.host.workspaces_list().unwrap(), vec![renamed]);

    f.host.workspace_delete(&workspace.workspace_id).unwrap();
    assert!(f.host.workspaces_list().unwrap().is_empty());
    for session in f.host.sessions_list().unwrap() {
        assert!(session.workspace_id.is_none(), "{}", session.session_id);
    }
    assert!(Path::new(&f.root).exists());
}

#[test]
fn a_session_exports_as_json_and_markdown_without_writing_a_file() {
    let f = fixture("export", vec![]);
    f.host
        .session_rename(&f.session, "叠溪 最新 SLC?*")
        .unwrap();
    let json_export = f
        .host
        .session_export(&f.session, insar_agent::session::SessionExportFormat::Json)
        .unwrap();
    assert_eq!(json_export.file_name, "叠溪 最新 SLC__.json");
    let parsed: Value = serde_json::from_str(&json_export.content).unwrap();
    assert_eq!(parsed["session"]["session_id"], f.session.as_str());
    assert!(parsed["events"].is_array());
    let markdown = f
        .host
        .session_export(
            &f.session,
            insar_agent::session::SessionExportFormat::Markdown,
        )
        .unwrap();
    assert!(markdown.file_name.ends_with(".md"));
    assert!(markdown.content.starts_with("# 叠溪 最新 SLC?*"));
    assert!(!f.root.join("叠溪 最新 SLC__.json").exists());
}

#[test]
fn an_export_written_to_a_chosen_folder_never_overwrites_a_file() {
    let f = fixture("export-to", vec![]);
    f.host.session_rename(&f.session, "叠溪").unwrap();
    let folder = f.root.join("exports");
    fs::create_dir_all(&folder).unwrap();
    let folder_text = folder.to_string_lossy().into_owned();
    let format = insar_agent::session::SessionExportFormat::Markdown;
    let first = f
        .host
        .session_export_to(&f.session, format, &folder_text)
        .unwrap();
    let second = f
        .host
        .session_export_to(&f.session, format, &folder_text)
        .unwrap();
    assert!(first.ends_with("叠溪.md"), "{first}");
    assert!(second.ends_with("叠溪 (2).md"), "{second}");
    assert!(fs::read_to_string(&first).unwrap().starts_with("# 叠溪"));
    assert!(matches!(
        f.host.session_export_to(&f.session, format, "relative"),
        Err(HostError::Invalid)
    ));
}

#[test]
fn a_sessions_work_lists_each_run_with_its_request_plans_and_created_tasks() {
    let f = fixture("work-view", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-w", json!({}), vec![sized("W1", GB)], &[]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::FullAccess))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        create_turn("pw", "batch-w", &["W1"], json!({})),
        text_turn(json!({"text": "完成", "claims": []})),
        text_turn(json!({"text": "第二次只回答", "claims": []})),
    ]);
    f.host
        .send(&f.session, "下载叠溪最新一景", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    f.host
        .send(&f.session, "只问一句", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);
    let work = f.host.session_work(&f.session).unwrap();
    assert_eq!(work.len(), 2);
    // Newest run first.
    assert_eq!(work[0].request.as_deref(), Some("只问一句"));
    assert!(work[0].plan_ids.is_empty() && work[0].created_task_ids.is_empty());
    assert_eq!(work[1].request.as_deref(), Some("下载叠溪最新一景"));
    assert_eq!(work[1].plan_ids.len(), 1);
    assert_eq!(work[1].created_task_ids, vec!["agent-task-0".to_owned()]);
    check("AgentWorkItem", &serde_json::to_value(&work[1]).unwrap());
}

#[test]
fn natural_chinese_markdown_conclusion_with_thinking_stream_and_auto_provenance() {
    use insar_agent::session::AgentHostNotification;

    let f = fixture("agent2-thinking-md", vec![]);
    seed_batch(&f, "batch-md-1", json!({}), sar_products(), &[]);
    let pushed = Arc::new(Mutex::new(Vec::<AgentHostNotification>::new()));
    let pushed_sink = Arc::clone(&pushed);
    f.host
        .set_notifier(Arc::new(move |note| pushed_sink.lock().unwrap().push(note)));

    let batch_args = json!({"batch_id": "batch-md-1", "revision": 0, "page": 0, "page_size": 20});
    let step_one = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"delta":{"content":"<think>先读取批次确认景数</think>我先核对检索结果。"}}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"b1","function":{"name":"batch_page","arguments":batch_args.to_string()}}]}}]}),
    );
    f.replay.turns.lock().unwrap().extend([
        step_one,
        raw_turn("<think>核对完毕，共 3 景</think>### 检索总结\n\n共命中 **3 景** Sentinel-1 影像，可直接创建下载任务。"),
    ]);

    let run_id = f
        .host
        .send(&f.session, "总结当前批次", SendContext::default())
        .unwrap();
    let conclusion = f.wait_for("conclusion").payload;
    assert_eq!(
        conclusion["text"],
        "### 检索总结\n\n共命中 **3 景** Sentinel-1 影像，可直接创建下载任务。"
    );
    assert_eq!(conclusion["thinking_text"], "核对完毕，共 3 景");
    assert!(conclusion["thinking_ms"].as_u64().is_some(), "{conclusion}");
    let claims = conclusion["claims"].as_array().unwrap();
    assert_eq!(claims.len(), 1, "{conclusion}");
    assert_eq!(claims[0]["verified"], true, "{conclusion}");
    assert_eq!(claims[0]["refs"][0]["kind"], "batch");
    assert_eq!(claims[0]["refs"][0]["id"], "batch-md-1");
    check("AgentConclusion", &conclusion);
    assert!(!trace_kinds(&f, &run_id).contains(&"conclusion_unparsed".to_owned()));

    let page = f.host.stream_poll(&f.session, 0).unwrap();
    let thinking_events: Vec<&StreamEvent> = page
        .events
        .iter()
        .filter(|e| e.kind == "thinking")
        .collect();
    assert!(thinking_events.len() >= 4, "{thinking_events:?}");
    for ev in &thinking_events {
        check("AgentThinkingPayload", &ev.payload);
    }
    assert_eq!(thinking_events[0].payload["status"], "thinking");
    assert_eq!(thinking_events[1].payload["status"], "done");
    assert_eq!(thinking_events[1].payload["text"], "先读取批次确认景数");

    let assistant_msgs: Vec<&StreamEvent> = page
        .events
        .iter()
        .filter(|e| e.kind == "message" && e.payload["role"] == "assistant")
        .collect();
    assert_eq!(assistant_msgs.len(), 1, "{assistant_msgs:?}");
    assert_eq!(assistant_msgs[0].payload["text"], "我先核对检索结果。");
    assert_eq!(
        assistant_msgs[0].payload["thinking_text"],
        "先读取批次确认景数"
    );
    check("AgentMessagePayload", &assistant_msgs[0].payload);

    let notes = pushed.lock().unwrap();
    assert!(notes.iter().any(|n| matches!(
        n,
        AgentHostNotification::LiveDelta { kind, .. } if kind == "thinking_start"
    )));
    assert!(notes.iter().any(|n| matches!(
        n,
        AgentHostNotification::LiveDelta { kind, delta, .. } if kind == "thinking" && delta.contains("核对完毕")
    )));
    assert!(notes.iter().any(|n| matches!(
        n,
        AgentHostNotification::LiveDelta { kind, .. } if kind == "thinking_end"
    )));
}

#[test]
fn single_plan_stages_working_aoi_set_before_search_start_without_stale_aoi() {
    use insar_agent::session::AgentHostNotification;

    let f = fixture("agent2-staged-aoi", vec![]);
    // Active AOI starts as Tibet (540000); user now asks for Sichuan (510000).
    f.fake
        .application_set_working_aoi(&json!({"admin_code": "540000"}))
        .unwrap();
    seed_batch(&f, "batch-sc", json!({}), sar_products(), &[]);
    *f.fake.next_search_batch.lock().unwrap() = Some("batch-sc".into());

    let pushed = Arc::new(Mutex::new(Vec::<AgentHostNotification>::new()));
    let pushed_sink = Arc::clone(&pushed);
    f.host
        .set_notifier(Arc::new(move |note| pushed_sink.lock().unwrap().push(note)));

    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "a1",
            "admin_boundary_search",
            json!({"query": "四川省", "limit": 5}),
        ),
        tool_turn(
            "p1",
            "propose_plan",
            json!({
                "steps": [
                    {
                        "tool_id": "working_aoi_set",
                        "input": {"admin_code": "510000"}
                    },
                    {
                        "tool_id": "search_start",
                        "input": {
                            "resource_id": "science.sentinel-1.slc.v1",
                            "aoi": {"ref": "working_aoi"},
                            "time_range": {"start": "2026-09-01T00:00:00Z", "end": "2026-10-01T00:00:00Z"},
                            "filters": {"polarizations": ["VV", "VV+VH"]}
                        }
                    }
                ]
            }),
        ),
        tool_turn("s1", "search_progress", json!({"request_id": "req-latest"})),
        raw_turn("已完成四川省 Sentinel-1 检索，共保留 **3 景**。"),
    ]);

    // Default ConfirmChanges mode: disclosed [working_aoi_set, search_start] must auto-execute!
    f.host
        .send(
            &f.session,
            "检索四川省 2026 年 9 月的 Sentinel-1 数据",
            SendContext::default(),
        )
        .unwrap();
    let conclusion = f.wait_for("conclusion").payload;
    assert!(conclusion["text"].as_str().unwrap().contains("四川省"));

    let searches = f.fake.searches.lock().unwrap();
    assert_eq!(searches.len(), 1, "{searches:?}");
    assert_eq!(searches[0]["aoi_name"], "四川省");
    assert_eq!(searches[0]["aoi"]["type"], "MultiPolygon");
    assert_eq!(
        searches[0]["aoi"]["coordinates"][0][0][0],
        json!([97.35, 26.05]),
        "search_start must resolve staged Sichuan geometry, not stale Tibet: {searches:?}"
    );

    let page = f.host.stream_poll(&f.session, 0).unwrap();
    assert!(
        !page.events.iter().any(|e| e.kind == "confirm_card"),
        "disclosed AOI + search plan must not wait for confirm_card"
    );
    let syncs: Vec<&StreamEvent> = page
        .events
        .iter()
        .filter(|e| e.kind == "workspace_sync")
        .collect();
    assert!(
        syncs
            .iter()
            .any(|e| e.payload["action"] == "aoi_changed" && e.payload["display_name"] == "四川省"),
        "{syncs:?}"
    );
    assert!(
        syncs
            .iter()
            .any(|e| e.payload["action"] == "search_updated" && e.payload["batch_id"] == "batch-sc"),
        "{syncs:?}"
    );
    for ev in syncs {
        check("AgentWorkspaceSyncPayload", &ev.payload);
    }
    let notes = pushed.lock().unwrap();
    assert!(notes.iter().any(|n| matches!(
        n,
        AgentHostNotification::AoiChanged { display_name: Some(name), .. } if name == "四川省"
    )));
    assert!(notes.iter().any(|n| matches!(
        n,
        AgentHostNotification::SearchUpdated { batch_id: Some(id), .. } if id == "batch-sc"
    )));
}

#[test]
fn sentinel1_continental_land_hh_guardrail_and_search_circuit_breaker() {
    // 1. Unprompted HH over continental land (Sichuan) is refused at propose_plan.
    {
        let f = fixture("agent2-s1-hh-unprompted", vec![]);
        f.fake
            .application_set_working_aoi(&json!({"admin_code": "510000"}))
            .unwrap();
        f.replay.turns.lock().unwrap().extend([
            tool_turn(
                "p1",
                "propose_plan",
                json!({
                    "steps": [{
                        "tool_id": "search_start",
                        "input": {
                            "resource_id": "science.sentinel-1.slc.v1",
                            "aoi": {"ref": "working_aoi"},
                            "filters": {"polarizations": ["HH"]}
                        }
                    }]
                }),
            ),
            raw_turn("陆地默认不支持 HH，请改用 VV 或 VV+VH。"),
        ]);
        let run_id = f
            .host
            .send(
                &f.session,
                "检索四川省 Sentinel-1 数据",
                SendContext::default(),
            )
            .unwrap();
        f.wait_for("conclusion");
        assert!(f.fake.searches.lock().unwrap().is_empty());
        let refused = plan_refused_events(&f, &run_id);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(refused[0]["reason"], "s1_land_hh_unavailable");
    }

    // 2. User-requested HH over Sichuan runs once, reports S1_LAND_HH_UNAVAILABLE on 0 items,
    //    and halts widening time_range retries via SearchCircuitBreaker.
    {
        let f = fixture("agent2-s1-hh-explicit", vec![]);
        f.fake
            .application_set_working_aoi(&json!({"admin_code": "510000"}))
            .unwrap();
        f.fake.search_items_kept.store(0, Ordering::SeqCst);
        seed_batch(&f, "batch-empty", json!({}), vec![], &[]);
        *f.fake.next_search_batch.lock().unwrap() = Some("batch-empty".into());

        f.replay.turns.lock().unwrap().extend([
            tool_turn(
                "p1",
                "propose_plan",
                json!({
                    "steps": [{
                        "tool_id": "search_start",
                        "input": {
                            "resource_id": "science.sentinel-1.slc.v1",
                            "aoi": {"ref": "working_aoi"},
                            "time_range": {"start": "2026-09-01T00:00:00Z", "end": "2026-10-01T00:00:00Z"},
                            "filters": {"polarizations": ["HH"]}
                        }
                    }]
                }),
            ),
            tool_turn("s1", "search_progress", json!({"request_id": "req-latest"})),
            tool_turn(
                "p2",
                "propose_plan",
                json!({
                    "steps": [{
                        "tool_id": "search_start",
                        "input": {
                            "resource_id": "science.sentinel-1.slc.v1",
                            "aoi": {"ref": "working_aoi"},
                            "time_range": {"start": "2025-01-01T00:00:00Z", "end": "2026-10-01T00:00:00Z"},
                            "filters": {"polarizations": ["HH"]}
                        }
                    }]
                }),
            ),
            raw_turn("四川省位于大陆区域，Sentinel-1 无 HH 极化数据，建议改查 VV 或 VV+VH。"),
        ]);
        let run_id = f
            .host
            .send(
                &f.session,
                "查四川省 Sentinel-1 HH 极化数据",
                SendContext::default(),
            )
            .unwrap();
        f.wait_for("conclusion");
        assert_eq!(f.fake.searches.lock().unwrap().len(), 1);
        let refused = plan_refused_events(&f, &run_id);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(refused[0]["reason"], "circuit_breaker_tripped");
        assert!(trace_kinds(&f, &run_id).contains(&"search_circuit_breaker".to_owned()));
        // Check that the model received zero_result_diagnostics in search_progress output.
        let seen = f.replay.seen.lock().unwrap();
        let all_tool_msgs: String = seen
            .iter()
            .flatten()
            .filter(|m| m.role == "tool")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_tool_msgs.contains("S1_LAND_HH_UNAVAILABLE"),
            "{all_tool_msgs}"
        );
    }

    // 3. General 0-result search allows 1 retry and trips circuit_breaker_tripped on attempt 3
    //    even when time_range is widened each time.
    {
        let f = fixture("agent2-zero-breaker", vec![]);
        f.fake
            .application_set_working_aoi(&json!({"admin_code": "510000"}))
            .unwrap();
        f.fake.search_items_kept.store(0, Ordering::SeqCst);
        seed_batch(&f, "batch-z", json!({}), vec![], &[]);
        *f.fake.next_search_batch.lock().unwrap() = Some("batch-z".into());

        let search_step = |id: &str, start: &str| {
            tool_turn(
                id,
                "propose_plan",
                json!({
                    "steps": [{
                        "tool_id": "search_start",
                        "input": {
                            "resource_id": "science.sentinel-1.slc.v1",
                            "aoi": {"ref": "working_aoi"},
                            "time_range": {"start": start, "end": "2026-10-01T00:00:00Z"},
                            "filters": {"polarizations": ["VV"]}
                        }
                    }]
                }),
            )
        };
        f.replay.turns.lock().unwrap().extend([
            search_step("p1", "2026-09-15T00:00:00Z"),
            tool_turn("s1", "search_progress", json!({"request_id": "req-latest"})),
            search_step("p2", "2026-06-01T00:00:00Z"),
            tool_turn("s2", "search_progress", json!({"request_id": "req-latest"})),
            search_step("p3", "2025-01-01T00:00:00Z"),
            raw_turn("重试一次扩大时间窗口后仍为 0 景，已停止重试。"),
        ]);
        let run_id = f
            .host
            .send(&f.session, "查四川省 VV 数据", SendContext::default())
            .unwrap();
        f.wait_for("conclusion");
        assert_eq!(f.fake.searches.lock().unwrap().len(), 2);
        let refused = plan_refused_events(&f, &run_id);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(refused[0]["reason"], "circuit_breaker_tripped");
    }
}

#[test]
fn synthetic_question_card_and_workspace_sync_tools_emit_typed_events() {
    use insar_agent::session::AgentHostNotification;

    let f = fixture("agent2-synthetic-tools", vec![]);
    let pushed = Arc::new(Mutex::new(Vec::<AgentHostNotification>::new()));
    let pushed_sink = Arc::clone(&pushed);
    f.host
        .set_notifier(Arc::new(move |note| pushed_sink.lock().unwrap().push(note)));

    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "m1",
            "map_workspace_action",
            json!({
                "action": "fit_bounds",
                "display_name": "四川省",
                "bbox": [97.35, 26.05, 108.55, 34.32]
            }),
        ),
        tool_turn(
            "q1",
            "ask_user_question",
            json!({
                "question_zh": "请选择要改用的极化方式：",
                "options": [
                    {"label_zh": "改查 VV+VH（推荐）", "value": "请改用 VV+VH 极化检索", "recommended": true},
                    {"label_zh": "改查单极化 VV", "value": "请改用 VV 极化检索", "recommended": false}
                ]
            }),
        ),
        raw_turn("已为您聚焦四川省范围，请点击上方选项继续。"),
    ]);
    let run_id = f
        .host
        .send(&f.session, "帮我定位四川并给建议", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");

    let body = first_outbound(&f, &run_id);
    for tool_name in [
        "admin_boundary_search",
        "ask_user_question",
        "map_workspace_action",
        "goal_manifest",
    ] {
        let schema = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["function"]["name"] == tool_name)
            .unwrap_or_else(|| panic!("{tool_name} must be offered"))["function"]["parameters"]
            .clone();
        assert_compatible_schema(tool_name, &schema);
    }

    let page = f.host.stream_poll(&f.session, 0).unwrap();
    let q_card = page
        .events
        .iter()
        .find(|e| e.kind == "question_card")
        .expect("question_card event emitted");
    check("AgentQuestionCardPayload", &q_card.payload);
    assert_eq!(q_card.payload["options"][0]["recommended"], true);

    let ws_sync = page
        .events
        .iter()
        .find(|e| e.kind == "workspace_sync")
        .expect("workspace_sync event emitted");
    check("AgentWorkspaceSyncPayload", &ws_sync.payload);
    assert_eq!(ws_sync.payload["action"], "fit_bounds");

    let notes = pushed.lock().unwrap();
    assert!(notes.iter().any(|n| matches!(
        n,
        AgentHostNotification::WorkspaceSync { payload, .. } if payload["action"] == "fit_bounds"
    )));
}

#[test]
fn session_truncate_removes_later_turns_and_restores_pre_turn_working_aoi() {
    let f = fixture("agent2-truncate", vec![]);
    f.fake
        .application_set_working_aoi(&json!({"admin_code": "540000"}))
        .unwrap();

    f.replay.turns.lock().unwrap().extend([
        raw_turn("当前工作区是西藏自治区。"),
        tool_turn(
            "p1",
            "propose_plan",
            json!({
                "steps": [{
                    "tool_id": "working_aoi_set",
                    "input": {"admin_code": "510000"}
                }]
            }),
        ),
        raw_turn("已将工作区切换为四川省。"),
    ]);

    let run1 = f
        .host
        .send(&f.session, "当前工作区是什么", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    assert_eq!(
        f.fake.application_working_aoi().unwrap()["display_name"],
        "西藏自治区"
    );

    let run2 = f
        .host
        .send(&f.session, "切换到四川省", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);
    assert_eq!(
        f.fake.application_working_aoi().unwrap()["display_name"],
        "四川省"
    );

    let page = f.host.stream_poll(&f.session, 0).unwrap();
    let turn2_user_seq = page
        .events
        .iter()
        .find(|e| {
            e.kind == "message"
                && e.payload["role"] == "user"
                && e.payload["text"] == "切换到四川省"
        })
        .expect("second user message")
        .seq;

    let truncated = f
        .host
        .session_truncate(&f.session, turn2_user_seq, true)
        .unwrap();
    assert!(truncated.cursor < turn2_user_seq, "{truncated:?}");
    assert_eq!(truncated.session.run_ids.len(), 1, "{truncated:?}");
    assert!(truncated.aoi_restored, "{truncated:?}");
    check(
        "AgentSessionSummary",
        &serde_json::to_value(&truncated.session).unwrap(),
    );

    // Truncated run directory and SQLite run row are cleanly deleted while kept run survives.
    assert!(f.root.join("agent/runs").join(&run1).is_dir());
    assert!(!f.root.join("agent/runs").join(&run2).exists());
    assert_eq!(
        f.fake
            .store
            .lock()
            .unwrap()
            .agent_session_run_ids(&f.session)
            .unwrap(),
        vec![run1]
    );

    // Working AOI is restored back to Tibet (pre-Turn-2 snapshot)!
    assert_eq!(
        f.fake.application_working_aoi().unwrap()["display_name"],
        "西藏自治区"
    );

    let after = f.host.stream_poll(&f.session, 0).unwrap();
    assert!(
        after.events.iter().all(|e| e.seq < turn2_user_seq),
        "{after:?}"
    );
    assert_eq!(f.host.runs_list(&f.session).unwrap().len(), 1);
}

#[test]
fn terminal_search_start_records_batch_emits_workspace_sync_and_feeds_circuit_breaker() {
    let f = fixture("terminal-search-start", vec![]);
    f.fake.search_start_terminal.store(true, Ordering::SeqCst);
    *f.fake.next_search_batch.lock().unwrap() = Some("batch-terminal-s1".into());
    f.fake.search_items_kept.store(250, Ordering::SeqCst);
    f.replay
        .turns
        .lock()
        .unwrap()
        .extend([search_turn("s1"), raw_turn("检索完成，命中 250 景")]);
    let run_id = f.host.send(&f.session, "查四川省", with_aoi()).unwrap();
    f.wait_for("conclusion");

    let run = f.host.run_open(&run_id).unwrap();
    assert!(
        run.events
            .iter()
            .any(|e| e.kind == "search_batch" && e.payload["batch_id"] == "batch-terminal-s1"),
        "terminal search_start must record search_batch even when model does not call search_progress"
    );
    let page = f.host.stream_poll(&f.session, 0).unwrap();
    assert!(
        page.events.iter().any(|e| {
            e.kind == "workspace_sync"
                && e.payload["action"] == "search_updated"
                && e.payload["batch_id"] == "batch-terminal-s1"
        }),
        "terminal search_start must emit workspace_sync(search_updated)"
    );
}

#[test]
fn unknown_tool_call_returns_recoverable_denial_and_logs_tool_denied_instead_of_aborting() {
    let f = fixture("recoverable-unknown-tool", vec![]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn("u1", "search_sentinel1_direct", json!({"region": "四川省"})),
        tool_turn("u2", "source_list", json!({})),
        raw_turn("已改用可用工具完成查询"),
    ]);
    let run_id = f
        .host
        .send(&f.session, "查数据源", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    assert_eq!(f.run_status(&run_id), "completed");

    let run = f.host.run_open(&run_id).unwrap();
    assert!(
        run.events.iter().any(|e| {
            e.kind == "tool_denied"
                && e.payload["reason"] == "tool_unknown"
                && e.payload["tool_id"] == "search_sentinel1_direct"
        }),
        "unknown tool call must record tool_denied with tool_id in trace"
    );
}

#[test]
fn send_context_working_aoi_ref_hydrates_large_working_polygon_without_exceeding_ipc_budget() {
    let f = fixture("working-aoi-ref-hydration", vec![]);
    let mut ring: Vec<Value> = (0..18_000)
        .map(|i| json!([103.5 + (i as f64) * 1e-5, 30.5 + ((i % 97) as f64) * 1e-4]))
        .collect();
    ring.push(ring[0].clone());
    let large_geom = json!({
        "type": "Polygon",
        "coordinates": [ring],
    });
    assert!(
        serde_json::to_vec(&large_geom).unwrap().len() > 256 * 1024,
        "test polygon must exceed 256 KiB inline SendContext limit"
    );
    *f.fake.working_aoi.lock().unwrap() = json!({
        "state": "active",
        "display_name": "成都市",
        "geometry": large_geom.clone(),
    });
    f.fake.search_start_terminal.store(true, Ordering::SeqCst);
    *f.fake.next_search_batch.lock().unwrap() = Some("batch-chengdu-large-poly".into());
    f.replay
        .turns
        .lock()
        .unwrap()
        .extend([search_turn("s-cd"), raw_turn("成都市高精度多边形检索完成")]);

    let run_id = f
        .host
        .send(
            &f.session,
            "检索成都市",
            SendContext {
                aoi: Some(json!({
                    "name": "成都市",
                    "geometry": { "ref": "working_aoi" }
                })),
                time_range: None,
                source: None,
                model: None,
                ..SendContext::default()
            },
        )
        .expect("lightweight working_aoi ref must be accepted");
    f.wait_for("conclusion");
    assert_eq!(f.run_status(&run_id), "completed");

    let searches = f.fake.searches.lock().unwrap();
    assert_eq!(searches.len(), 1);
    assert_eq!(
        searches[0]["aoi"]["coordinates"][0]
            .as_array()
            .unwrap()
            .len(),
        18_001,
        "hydrated context_aoi must preserve all 18,001 vertices of the real working polygon"
    );
}

#[test]
fn premature_action_promise_and_guessed_admin_code_are_caught_and_recover_cleanly() {
    let f = fixture("premature-promise-and-admin-guard", vec![]);
    f.replay.turns.lock().unwrap().extend([
        // Step 0: Flash model emits premature action-promise prose without tool_calls!
        raw_turn("按你的要求，下面一次性提交把研究区切换到四川省的计划，不再单独征求确认。"),
        // Step 1: Model guesses wrong admin_code 540000 (西藏自治区) when user asked for 四川省!
        tool_turn(
            "p-wrong",
            "propose_plan",
            json!({
                "steps": [{
                    "tool_id": "working_aoi_set",
                    "input": {"admin_code": "540000"}
                }]
            }),
        ),
        // Step 2: Host refuses with admin_code_unverified -> model calls admin_boundary_search!
        tool_turn(
            "adm-sc",
            "admin_boundary_search",
            json!({"query": "四川省", "limit": 5}),
        ),
        // Step 3: Model submits verified admin_code 510000!
        tool_turn(
            "p-right",
            "propose_plan",
            json!({
                "steps": [{
                    "tool_id": "working_aoi_set",
                    "input": {"admin_code": "510000"}
                }]
            }),
        ),
        // Step 4: Final clean conclusion!
        raw_turn("已核实行政区代码并将当前研究区切换为四川省。"),
    ]);

    let run_id = f
        .host
        .send(
            &f.session,
            "请把当前研究区切换到四川省",
            SendContext::default(),
        )
        .unwrap();
    let conclusion = f.wait_for("conclusion").payload;
    assert!(conclusion["text"].as_str().unwrap().contains("四川省"));
    assert_eq!(f.run_status(&run_id), "completed");
    assert_eq!(
        f.fake.application_working_aoi().unwrap()["display_name"],
        "四川省"
    );

    // Ensure the discarded Step 0 premature promise text was NOT leaked as an assistant message event!
    let stream = f.host.stream_poll(&f.session, 0).unwrap();
    assert!(
        !stream.events.iter().any(|e| {
            e.kind == "message"
                && e.payload.get("role").and_then(Value::as_str) == Some("assistant")
                && e.payload
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.contains("下面一次性提交"))
        }),
        "discarded premature action-promise text must not leak into session events"
    );
}

#[test]
fn session_workspace_state_isolates_working_aoi_across_workbench_changes() {
    let f = fixture("workspace-isolation", vec![]);
    // Global Map Workbench starts on Tibet (540000).
    f.fake
        .application_set_working_aoi(&json!({"admin_code": "540000"}))
        .unwrap();

    // Unstarted session C reports uninitialized workspace state.
    let session_c = f
        .host
        .session_create(Some("unstarted".into()))
        .unwrap()
        .session_id;
    let state_c = f.host.session_workspace_state(&session_c).unwrap();
    assert!(!state_c.initialized);
    assert!(state_c.working_aoi.is_null());

    // Session A runs its first turn -> snapshots 540000 into <session_dir>/workspace_state.json.
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(raw_turn("已记录工作区。"));
    f.host
        .send(&f.session, "你好", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let state_a = f.host.session_workspace_state(&f.session).unwrap();
    assert!(state_a.initialized);
    assert_eq!(state_a.working_aoi["display_name"], "西藏自治区");

    // Global Map Workbench now switches to Sichuan (510000).
    f.fake
        .application_set_working_aoi(&json!({"admin_code": "510000"}))
        .unwrap();

    // Session A Turn 2 calls working_aoi_get and searches {"ref": "working_aoi"}:
    // it MUST still use Tibet (540000), not Sichuan (510000)!
    f.replay.turns.lock().unwrap().extend([
        tool_turn("g1", "working_aoi_get", json!({})),
        tool_turn(
            "p1",
            "propose_plan",
            json!({
                "steps": [{
                    "tool_id": "search_start",
                    "input": {
                        "resource_id": "science.sentinel-1.slc.v1",
                        "aoi": {"ref": "working_aoi"}
                    }
                }]
            }),
        ),
        tool_turn(
            "sp1",
            "search_progress",
            json!({"request_id": "req-latest"}),
        ),
        raw_turn("已在西藏自治区完成检索。"),
    ]);
    let run_a2 = f
        .host
        .send(&f.session, "查当前工作区", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);
    assert_eq!(f.run_status(&run_a2), "completed");
    let searches = f.fake.searches.lock().unwrap();
    assert_eq!(searches.len(), 1);
    assert_eq!(searches[0]["aoi_name"], "西藏自治区");
    drop(searches);

    // Session B starts after global switch -> snapshots Sichuan (510000).
    let session_b = f
        .host
        .session_create(Some("sichuan".into()))
        .unwrap()
        .session_id;
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(raw_turn("新会话已就绪。"));
    let _ = f
        .host
        .send(&session_b, "开始", SendContext::default())
        .unwrap();
    events_via(&f.host, &session_b, "conclusion", 1);
    let state_b = f.host.session_workspace_state(&session_b).unwrap();
    assert_eq!(state_b.working_aoi["display_name"], "四川省");

    // session_set_working_aoi updates Session A without mutating global workbench AOI.
    let updated_a = f
        .host
        .session_set_working_aoi(
            &f.session,
            Some(json!({
                "source_kind": "draw",
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[[117.1, 31.7], [117.4, 31.7], [117.4, 31.9], [117.1, 31.9], [117.1, 31.7]]]
                },
                "bbox": [117.1, 31.7, 117.4, 31.9],
                "crs": "EPSG:4326",
                "display_name": "合肥测试区"
            })),
            None,
            Some("batch-manual".into()),
            Some("science.sentinel-1.slc.v1".into()),
            Some(2),
        )
        .unwrap();
    assert_eq!(updated_a.working_aoi["display_name"], "合肥测试区");
    assert_eq!(updated_a.active_batch_id.as_deref(), Some("batch-manual"));
    assert_eq!(updated_a.active_batch_revision, Some(2));
    assert_eq!(
        f.fake.application_working_aoi().unwrap()["display_name"],
        "四川省"
    );
}

#[test]
fn multi_turn_search_then_create_task_and_auto_approve_twenty_gb_cap() {
    let f = fixture("multi-turn-and-20gb-cap", vec![]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();

    // Small batch: 1 GB (< 20 GB) -> auto-approves in Turn 2 without search_start in Turn 2.
    let small_item = sized("SMALL-1", GB);
    seed_batch(&f, "batch-small", json!({}), vec![small_item], &[]);
    // Large batch: 25 GB (>= 20 GB) -> requires manual card in AutoApprove, FullAccess still runs.
    let large_items = vec![
        sized("LARGE-1", 10 * GB),
        sized("LARGE-2", 10 * GB),
        sized("LARGE-3", 5 * GB),
    ];
    seed_batch(&f, "batch-large", json!({}), large_items, &[]);

    // Turn 1 freezes batch-small.
    *f.fake.next_search_batch.lock().unwrap() = Some("batch-small".into());
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "ps1",
            "propose_plan",
            json!({
                "steps": [{
                    "tool_id": "search_start",
                    "input": {
                        "resource_id": "science.sentinel-1.slc.v1",
                        "aoi": {"ref": "working_aoi"}
                    }
                }]
            }),
        ),
        tool_turn(
            "sp1",
            "search_progress",
            json!({"request_id": "req-latest"}),
        ),
        raw_turn("检索完成，请确认。"),
    ]);
    f.fake
        .application_set_working_aoi(&json!({"admin_code": "510000"}))
        .unwrap();
    f.host
        .send(&f.session, "检索数据", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");

    // Turn 2: NO search_start in Turn 2, creates task from Turn 1's batch-small (< 20 GB).
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "pt-small",
            "propose_plan",
            json!({
                "steps": [{
                    "tool_id": "create_task",
                    "input": {
                        "batch_id": "batch-small",
                        "revision": 0,
                        "item_ids": ["SMALL-1"],
                        "options": {}
                    }
                }]
            }),
        ),
        raw_turn("小体积下载任务已自动创建。"),
    ]);
    let run2 = f
        .host
        .send(&f.session, "下载第一个小产品", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);
    assert_eq!(f.run_status(&run2), "completed");
    assert!(
        plan_refused_events(&f, &run2).is_empty(),
        "Turn 2 create_task on Turn 1 batch must not fail with selection_not_from_run"
    );
    assert_eq!(f.fake.created.lock().unwrap().len(), 1);
    let cards_so_far = events_of(&f, "confirm_card", 1);
    assert_eq!(cards_so_far[0]["auto_executed"], json!(true));
    assert_eq!(
        cards_so_far[0]["snapshot"]["selected_items_preview"][0]["scene_id"],
        json!("SMALL-1")
    );

    // Turn 3: search_start batch-large in Turn 3, then create_task 25 GB -> must park for card!
    *f.fake.next_search_batch.lock().unwrap() = Some("batch-large".into());
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "ps2",
            "propose_plan",
            json!({
                "steps": [{
                    "tool_id": "search_start",
                    "input": {
                        "resource_id": "science.sentinel-1.slc.v1",
                        "aoi": {"ref": "working_aoi"}
                    }
                }]
            }),
        ),
        tool_turn(
            "sp2",
            "search_progress",
            json!({"request_id": "req-latest"}),
        ),
        tool_turn(
            "pt-large",
            "propose_plan",
            json!({
                "steps": [{
                    "tool_id": "create_task",
                    "input": {
                        "batch_id": "batch-large",
                        "revision": 0,
                        "item_ids": ["LARGE-1", "LARGE-2", "LARGE-3"],
                        "options": {}
                    }
                }]
            }),
        ),
    ]);
    let _run3 = f
        .host
        .send(&f.session, "搜索并下载大文件", SendContext::default())
        .unwrap();
    let cards = events_of(&f, "confirm_card", 2);
    let large_card = &cards[1];
    assert_eq!(large_card["auto_executed"], json!(false));
    assert_eq!(large_card["approval"]["decision"], json!("manual"));
    assert_eq!(
        large_card["snapshot"]["selected_items_preview"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn confirm_card_snapshot_populates_nisar_resource_filters_datetime_assets_and_selected_items_preview(
) {
    let f = fixture("rich-confirm-card", vec![]);
    let mut item = product(
        "NISAR-ITEM-1",
        json!({
            "scene_id": "NISAR_L1_PR_RSLC_001",
            "acquisition_time": "2026-01-15T06:00:00Z",
            "flight_direction": "ASCENDING",
            "relative_orbit": 42
        }),
    );
    item.payload = json!({
        "assets": {
            "h5": { "href": "https://example.invalid/nisar.h5", "size_bytes": 123_456 },
            "xml": { "href": "https://example.invalid/nisar.xml", "size_bytes": 444 }
        }
    })
    .as_object()
    .unwrap()
    .clone();
    seed_batch(
        &f,
        "batch-nisar",
        json!({
            "resource_id": "science.nisar.l.rslc.provisional.v1",
            "filters": { "datetime": "2026-01-01T00:00:00Z/2026-02-01T00:00:00Z" }
        }),
        vec![item],
        &[],
    );
    f.replay.turns.lock().unwrap().push_back(tool_turn(
        "p-nisar",
        "propose_plan",
        json!({
            "steps": [{
                "tool_id": "create_task",
                "input": {
                    "batch_id": "batch-nisar",
                    "revision": 0,
                    "item_ids": ["NISAR-ITEM-1"],
                    "options": {}
                }
            }]
        }),
    ));
    let _run = f
        .host
        .send(&f.session, "创建 NISAR 任务", SendContext::default())
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    let snap = &card["snapshot"];
    assert_eq!(
        snap["source_resource_id"],
        json!("science.nisar.l.rslc.provisional.v1")
    );
    assert_eq!(
        snap["range"]["time_range"],
        json!({"start": "2026-01-01T00:00:00Z", "end": "2026-02-01T00:00:00Z"})
    );
    assert_eq!(snap["estimated_bytes"], json!(123_900));
    let preview = snap["selected_items_preview"].as_array().unwrap();
    assert_eq!(preview.len(), 1);
    assert_eq!(preview[0]["scene_id"], json!("NISAR_L1_PR_RSLC_001"));
    assert_eq!(
        preview[0]["acquisition_time"],
        json!("2026-01-15T06:00:00Z")
    );
    assert_eq!(preview[0]["orbit_direction"], json!("ASCENDING"));
    assert_eq!(preview[0]["relative_orbit"], json!(42));
    assert_eq!(preview[0]["size_bytes"], json!(123_900));
}

#[test]
fn sessions_list_reports_live_and_persisted_last_run_status() {
    let f = fixture("last-run-status", vec![]);
    let revision = f.revision();

    let find_session = |host: &AgentHost<Fake, Source>, session_id: &str| {
        let list = host.sessions_list().unwrap();
        let found = list
            .into_iter()
            .find(|s| s.session_id == session_id)
            .expect("session should exist in sessions_list");
        check(
            "AgentSessionSummary",
            &serde_json::to_value(&found).unwrap(),
        );
        found
    };

    // 1. Fresh session with no runs -> None.
    assert_eq!(find_session(&f.host, &f.session).last_run_status, None);

    // 2. Active run while model opening is delayed -> Some("running").
    f.replay.open_delay_ms.store(250, Ordering::SeqCst);
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c-pause", "propose_plan", pause_plan(revision)),
        text_turn(answer()),
    ]);
    let run_completed = f
        .host
        .send(&f.session, "暂停任务并报告", SendContext::default())
        .unwrap();
    assert_eq!(
        find_session(&f.host, &f.session).last_run_status.as_deref(),
        Some("running")
    );
    f.replay.open_delay_ms.store(0, Ordering::SeqCst);

    // 3. Parked waiting for approval card -> Some("awaiting_approval").
    let card = f.wait_for("confirm_card").payload;
    let plan_id = card["plan_id"].as_str().unwrap().to_owned();
    assert_eq!(
        find_session(&f.host, &f.session).last_run_status.as_deref(),
        Some("awaiting_approval")
    );

    // 4. Approved and finished -> Some("completed"), preserved across rename.
    f.host
        .plan_approve(&plan_id, card["snapshot"].clone(), None)
        .unwrap();
    let _ = f.wait_for("conclusion");
    let deadline = Instant::now() + Duration::from_secs(5);
    while f.run_status(&run_completed) == "running" {
        assert!(Instant::now() < deadline, "run did not seal in time");
        std::thread::sleep(Duration::from_millis(10));
    }
    let completed_wait_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let current = find_session(&f.host, &f.session);
        if current.last_run_status.as_deref() == Some("completed") {
            break;
        }
        assert!(
            Instant::now() < completed_wait_deadline,
            "expected completed last_run_status, got {:?}",
            current.last_run_status
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let renamed = f.host.session_rename(&f.session, "已完成的会话").unwrap();
    assert_eq!(renamed.last_run_status.as_deref(), Some("completed"));

    // 5. Failed run (truncated model turn) -> Some("failed").
    let failed_session = f.host.session_create(Some("失败会话".into())).unwrap();
    assert_eq!(failed_session.last_run_status, None);
    f.replay.turns.lock().unwrap().push_back(format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"delta":{"content":"截断"},"finish_reason":"length"}]})
    ));
    let failed_run_id = f
        .host
        .send(
            &failed_session.session_id,
            "触发失败",
            SendContext::default(),
        )
        .unwrap();
    let failed_deadline = Instant::now() + Duration::from_secs(5);
    while f.run_status(&failed_run_id) == "running" {
        assert!(Instant::now() < failed_deadline, "failed run did not seal");
        std::thread::sleep(Duration::from_millis(10));
    }
    let failed_wait_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let current = find_session(&f.host, &failed_session.session_id);
        if current.last_run_status.as_deref() == Some("failed") {
            break;
        }
        assert!(
            Instant::now() < failed_wait_deadline,
            "expected failed last_run_status, got {:?}",
            current.last_run_status
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // 5b. Explicitly cancelled run -> Some("cancelled").
    let cancelled_session = f.host.session_create(Some("取消会话".into())).unwrap();
    f.replay.turns.lock().unwrap().push_back(tool_turn(
        "c-cancel",
        "propose_plan",
        pause_plan(revision),
    ));
    let cancel_run_id = f
        .host
        .send(
            &cancelled_session.session_id,
            "等待确认后取消",
            SendContext::default(),
        )
        .unwrap();
    let cancel_park_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if find_session(&f.host, &cancelled_session.session_id)
            .last_run_status
            .as_deref()
            == Some("awaiting_approval")
        {
            break;
        }
        assert!(
            Instant::now() < cancel_park_deadline,
            "cancelled session never parked"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(f.host.run_cancel(&cancel_run_id).unwrap());
    let cancel_seal_deadline = Instant::now() + Duration::from_secs(5);
    while f.run_status(&cancel_run_id) == "running" {
        assert!(
            Instant::now() < cancel_seal_deadline,
            "cancelled run did not seal"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let cancel_wait_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let current = find_session(&f.host, &cancelled_session.session_id);
        if current.last_run_status.as_deref() == Some("cancelled") {
            break;
        }
        assert!(
            Instant::now() < cancel_wait_deadline,
            "expected cancelled last_run_status, got {:?}",
            current.last_run_status
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // 6. Interrupted run after reopen -> maps to Some("cancelled").
    let interrupted_session = f.host.session_create(Some("中断会话".into())).unwrap();
    f.replay.turns.lock().unwrap().push_back(tool_turn(
        "c-int",
        "propose_plan",
        pause_plan(revision),
    ));
    let _int_run_id = f
        .host
        .send(
            &interrupted_session.session_id,
            "等待确认时重启",
            SendContext::default(),
        )
        .unwrap();
    let int_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if find_session(&f.host, &interrupted_session.session_id)
            .last_run_status
            .as_deref()
            == Some("awaiting_approval")
        {
            break;
        }
        assert!(
            Instant::now() < int_deadline,
            "interrupted session never parked"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let reopened = AgentHost::open(
        f.root.clone(),
        f.fake.clone(),
        Source(Arc::clone(&f.replay)),
    )
    .unwrap();
    assert_eq!(
        find_session(&reopened, &interrupted_session.session_id)
            .last_run_status
            .as_deref(),
        Some("cancelled")
    );
}

#[test]
fn parked_confirm_card_is_superseded_by_new_user_turn_and_aoi_change() {
    let f = fixture("parked-supersede", vec![]);
    let revision = f.revision();

    // 1. Park turn 1 on a manual confirm card, then send a new user turn directly.
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c-pause-1", "propose_plan", pause_plan(revision)),
        raw_turn("已按最新要求为您切换检索方案。"),
    ]);
    let run1 = f
        .host
        .send(&f.session, "暂停任务", SendContext::default())
        .unwrap();
    let card1 = f.wait_for("confirm_card").payload;
    let plan1 = card1["plan_id"].as_str().unwrap().to_owned();
    assert_eq!(card1["run_id"], json!(run1));
    assert!(matches!(
        f.host
            .sessions_list()
            .unwrap()
            .into_iter()
            .find(|s| s.session_id == f.session)
            .and_then(|s| s.last_run_status)
            .as_deref(),
        Some("awaiting_approval")
    ));

    let run2 = f
        .host
        .send(&f.session, "改查成都市 Sentinel-2", SendContext::default())
        .expect("sending while parked on confirm card must supersede instead of returning Busy");
    let conc = f.wait_for("conclusion");
    assert_eq!(conc.payload["run_id"], json!(run2));
    assert!(matches!(
        f.host.plan_approve(&plan1, card1["snapshot"].clone(), None),
        Err(HostError::NotFound)
    ));
    assert_eq!(f.plan_status(&plan1), "expired");
    assert_eq!(f.run_status(&run1), "cancelled");

    let page1 = f.host.stream_poll(&f.session, 0).unwrap();
    let superseded1 = page1
        .events
        .iter()
        .find(|e| e.kind == "progress_card" && e.payload["plan_id"] == json!(plan1))
        .expect("progress_card must be emitted for superseded plan");
    check("AgentProgressCard", &superseded1.payload);
    assert_eq!(superseded1.payload["status"], json!("expired"));
    assert_eq!(
        superseded1.payload["code"],
        json!("plan_superseded_by_new_turn")
    );
    let second_user_msg = page1
        .events
        .iter()
        .find(|e| {
            e.kind == "message" && e.payload["run_id"] == json!(run2) && e.payload["role"] == "user"
        })
        .expect("second user message must be recorded");
    assert!(
        superseded1.seq < second_user_msg.seq,
        "supersession progress_card must precede new user turn event"
    );
    assert!(
        !page1
            .events
            .iter()
            .any(|e| e.kind == "error" && e.payload["run_id"] == json!(run1)),
        "superseded confirm card must not emit an error card"
    );

    // 2. Park turn 3 on a confirm card, then change the session's working AOI.
    f.replay.turns.lock().unwrap().push_back(tool_turn(
        "c-pause-2",
        "propose_plan",
        pause_plan(revision),
    ));
    let run3 = f
        .host
        .send(&f.session, "再次暂停任务", SendContext::default())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let card3 = loop {
        let p = f.host.stream_poll(&f.session, 0).unwrap();
        if let Some(ev) = p
            .events
            .into_iter()
            .find(|e| e.kind == "confirm_card" && e.payload["run_id"] == json!(run3))
        {
            break ev.payload;
        }
        assert!(
            Instant::now() < deadline,
            "second confirm_card never arrived"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let plan3 = card3["plan_id"].as_str().unwrap().to_owned();

    f.host
        .session_set_working_aoi(
            &f.session,
            Some(json!({
                "source_kind": "draw",
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[[103.5, 30.2], [104.5, 30.2], [104.5, 31.1], [103.5, 31.1], [103.5, 30.2]]]
                },
                "bbox": [103.5, 30.2, 104.5, 31.1],
                "crs": "EPSG:4326",
                "display_name": "成都测试区"
            })),
            None,
            None,
            None,
            None,
        )
        .unwrap();

    assert!(matches!(
        f.host.plan_approve(&plan3, card3["snapshot"].clone(), None),
        Err(HostError::NotFound)
    ));
    assert_eq!(f.plan_status(&plan3), "expired");
    assert_eq!(f.run_status(&run3), "cancelled");
    let page2 = f.host.stream_poll(&f.session, 0).unwrap();
    let superseded3 = page2
        .events
        .iter()
        .find(|e| e.kind == "progress_card" && e.payload["plan_id"] == json!(plan3))
        .expect("progress_card must be emitted when AOI change supersedes confirm card");
    check("AgentProgressCard", &superseded3.payload);
    assert_eq!(superseded3.payload["status"], json!("expired"));
    assert_eq!(
        superseded3.payload["code"],
        json!("plan_superseded_by_aoi_change")
    );
    assert!(
        !page2
            .events
            .iter()
            .any(|e| e.kind == "error" && e.payload["run_id"] == json!(run3)),
        "AOI supersession must not emit an error card"
    );
    assert_eq!(
        f.host
            .sessions_list()
            .unwrap()
            .into_iter()
            .find(|s| s.session_id == f.session)
            .and_then(|s| s.last_run_status)
            .as_deref(),
        Some("cancelled")
    );
}

#[test]
fn goal_manifest_enforces_store_evidence_and_discloses_missing_items() {
    let f = fixture("goal-manifest-guard", vec![]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "g1",
            "goal_manifest",
            json!({
                "action": "define",
                "title_zh": "成都多源配数",
                "items": [
                    {"item_id": "s1", "label_zh": "Sentinel-1 影像"},
                    {"item_id": "dem", "label_zh": "30米高程 DEM"},
                    {
                        "item_id": "orbit",
                        "label_zh": "精密轨道 POEORB",
                        "status": "capability_missing",
                        "note_zh": "当前仅提供影像检索，精密轨道暂需后续接入"
                    }
                ]
            }),
        ),
        tool_turn(
            "g2",
            "goal_manifest",
            json!({
                "action": "bind",
                "item_id": "s1",
                "status": "ready",
                "batch_id": "batch-nonexistent"
            }),
        ),
        tool_turn(
            "g3",
            "goal_manifest",
            json!({
                "action": "bind",
                "item_id": "s1",
                "status": "ready",
                "batch_id": "batch-a"
            }),
        ),
        raw_turn("已为您全部配齐数据。"),
    ]);

    let run_id = f
        .host
        .send(
            &f.session,
            "帮我配齐成都影像、DEM与精密轨道",
            SendContext::default(),
        )
        .unwrap();
    let conc = f.wait_for("conclusion").payload;
    assert_eq!(conc["run_id"], json!(run_id));
    let conc_text = conc["text"].as_str().unwrap_or("");
    assert!(conc_text.contains("已为您全部配齐数据。"), "{conc_text}");
    assert!(conc_text.contains("交付清单核对"), "{conc_text}");
    assert!(conc_text.contains("Sentinel-1 影像"), "{conc_text}");
    assert!(conc_text.contains("30米高程 DEM"), "{conc_text}");
    assert!(conc_text.contains("待补齐"), "{conc_text}");
    assert!(conc_text.contains("精密轨道 POEORB"), "{conc_text}");
    assert!(conc_text.contains("当前能力未接入"), "{conc_text}");

    let seen = f.replay.seen.lock().unwrap();
    let tool_msgs: Vec<String> = seen
        .iter()
        .flatten()
        .filter(|m| m.role == "tool")
        .map(|m| m.content.clone())
        .collect();
    assert!(
        tool_msgs
            .iter()
            .any(|m| m.contains("goal_evidence_not_found")),
        "missing batch_id must be refused by goal_manifest: {tool_msgs:?}"
    );
    assert!(
        tool_msgs
            .iter()
            .any(|m| m.contains("\"ready_count\":1") && m.contains("\"all_ready\":false")),
        "real Store batch_id must bind as ready while keeping all_ready=false: {tool_msgs:?}"
    );

    let detail = f.host.run_open(&run_id).unwrap();
    assert_eq!(detail.run.status, "completed");
}

#[test]
fn tool_rounds_emit_target_and_summaries_and_dedup_identical_read_calls() {
    let f = fixture("tool-summaries-dedup", vec![]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "admin_boundary_search", json!({"query": "四川"})),
        tool_turn("c3", "admin_boundary_search", json!({"query": "510000"})),
        tool_turn("c2", "admin_boundary_search", json!({"query": "四川"})),
        tool_turn(
            "c4",
            "search_progress",
            json!({"request_id": "req-running"}),
        ),
        tool_turn(
            "c5",
            "search_progress",
            json!({"request_id": "req-running"}),
        ),
        text_turn(json!({"text": "检索核对完成", "claims": []})),
    ]);
    f.host
        .send(&f.session, "查行政区与检索进度", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");

    let rounds = tool_rounds(&f);
    let round = |call: &str, status: &str| {
        rounds
            .iter()
            .find(|r| r["call_id"] == call && r["status"] == status)
            .cloned()
            .unwrap_or_else(|| panic!("no {status} round for {call}: {rounds:?}"))
    };

    let c1_done = round("c1", "done");
    assert_eq!(c1_done["target_label"], "四川");
    assert!(
        c1_done["input_summary"]
            .as_str()
            .unwrap_or("")
            .contains("query: 四川"),
        "{c1_done}"
    );
    assert!(
        !c1_done["output_summary"].as_str().unwrap_or("").is_empty(),
        "{c1_done}"
    );
    assert!(
        !c1_done["output_summary"]
            .as_str()
            .unwrap_or("")
            .contains("复用本轮相同参数缓存"),
        "{c1_done}"
    );

    let c2_done = round("c2", "done");
    assert_eq!(c2_done["target_label"], "四川");
    assert!(
        c2_done["output_summary"]
            .as_str()
            .unwrap_or("")
            .starts_with("复用本轮相同参数缓存"),
        "{c2_done}"
    );

    let c3_done = round("c3", "done");
    assert_eq!(c3_done["target_label"], "510000");
    assert!(
        !c3_done["output_summary"]
            .as_str()
            .unwrap_or("")
            .starts_with("复用本轮相同参数缓存"),
        "{c3_done}"
    );

    let c5_done = round("c5", "done");
    assert_eq!(c5_done["target_label"], "req-running");
    assert!(
        !c5_done["output_summary"]
            .as_str()
            .unwrap_or("")
            .starts_with("复用本轮相同参数缓存"),
        "search_progress polling must never be cached: {c5_done}"
    );
}

#[test]
fn progressive_tool_disclosure_defers_extension_schemas_until_activated_or_matched() {
    let f = fixture("progressive-disclosure", vec![]);
    *f.fake.extension_tools.lock().unwrap() = vec![
        insar_agent_model::ToolDescription {
            name: "skill_read".into(),
            description: "skill".into(),
            parameters: json!({"type": "object"}),
        },
        insar_agent_model::ToolDescription {
            name: "toolset_activate".into(),
            description: "activate".into(),
            parameters: json!({"type": "object"}),
        },
        insar_agent_model::ToolDescription {
            name: "gee_catalog_search".into(),
            description: "gee".into(),
            parameters: json!({"type": "object"}),
        },
        insar_agent_model::ToolDescription {
            name: "gis_local_process".into(),
            description: "gis".into(),
            parameters: json!({"type": "object"}),
        },
        insar_agent_model::ToolDescription {
            name: "mcp__demo__ping".into(),
            description: "mcp".into(),
            parameters: json!({"type": "object"}),
        },
    ];
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c1", "toolset_activate", json!({"toolsets": ["gee"]})),
        text_turn(json!({"text": "已按需解锁 GEE 工具包。", "claims": []})),
    ]);
    f.host
        .send(&f.session, "查看当前任务概况", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");

    let seen = f.replay.tools_seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    // Step 0 (generic prompt): core + skill_read + toolset_activate visible; gee_*/gis_*/mcp__* deferred
    assert!(seen[0].iter().any(|n| n == "skill_read"));
    assert!(seen[0].iter().any(|n| n == "toolset_activate"));
    assert!(!seen[0].iter().any(|n| n == "gee_catalog_search"));
    assert!(!seen[0].iter().any(|n| n == "gis_local_process"));
    assert!(!seen[0].iter().any(|n| n == "mcp__demo__ping"));

    // Step 1 (after toolset_activate(["gee"])): gee_catalog_search unlocked; gis_*/mcp__* remain deferred
    assert!(seen[1].iter().any(|n| n == "gee_catalog_search"));
    assert!(!seen[1].iter().any(|n| n == "gis_local_process"));
    assert!(!seen[1].iter().any(|n| n == "mcp__demo__ping"));
}

#[test]
fn passive_workspace_context_does_not_unlock_nisar_tools_or_trigger_actions_on_greeting() {
    let f = fixture("passive-workspace-hi", vec![]);
    *f.fake.extension_tools.lock().unwrap() = vec![
        insar_agent_model::ToolDescription {
            name: "nisar_subset_preflight".into(),
            description: "preflight".into(),
            parameters: json!({"type": "object"}),
        },
        insar_agent_model::ToolDescription {
            name: "nisar_create_subset_task".into(),
            description: "create".into(),
            parameters: json!({"type": "object"}),
        },
    ];
    f.replay.turns.lock().unwrap().push_back(text_turn(
        json!({"text": "你好！有什么我可以帮你的？", "claims": []}),
    ));

    let yinchuan_ctx = SendContext {
        aoi: Some(json!({
            "name": "银川市",
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[105.8, 38.1], [106.8, 38.1], [106.8, 38.9], [105.8, 38.9], [105.8, 38.1]]]
            }
        })),
        time_range: Some(json!({"start": "2026-01-01", "end": "2026-06-01"})),
        source: Some("nisar".into()),
        model: None,
        ..SendContext::default()
    };
    f.host.send(&f.session, "hi", yinchuan_ctx).unwrap();
    f.wait_for("conclusion");

    let seen_msgs = f.replay.seen.lock().unwrap();
    let sys_prompt = &seen_msgs[0][0].content;
    assert!(
        sys_prompt.contains("passive_workspace_snapshot"),
        "system prompt must declare passive context separation"
    );
    let user_msg = &seen_msgs[0][1].content;
    assert!(
        user_msg.contains("\"intent_mode\":\"passive_workspace_snapshot\""),
        "injected [context] must carry intent_mode=passive_workspace_snapshot: {user_msg}"
    );
    let tools_seen = f.replay.tools_seen.lock().unwrap();
    assert!(
        !tools_seen[0].iter().any(|t| t == "nisar_subset_preflight"),
        "passive [context] source=nisar must NOT unlock nisar tools on greeting: {:?}",
        tools_seen[0]
    );
    assert!(
        !tools_seen[0]
            .iter()
            .any(|t| t == "nisar_create_subset_task"),
        "passive [context] source=nisar must NOT unlock nisar download tools on greeting"
    );
    assert!(
        tool_rounds(&f).is_empty(),
        "greeting must execute 0 tool calls"
    );
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    assert!(f.fake.searches.lock().unwrap().is_empty());
}

#[test]
fn yinchuan_architecture_eval_interrupted_resumes_without_repeating_checks_or_starting_downloads() {
    let f = fixture("yinchuan-resume-checkpoint", vec![]);
    let yinchuan_ctx = || SendContext {
        aoi: Some(json!({
            "name": "银川市",
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[105.8, 38.1], [106.8, 38.1], [106.8, 38.9], [105.8, 38.9], [105.8, 38.1]]]
            }
        })),
        time_range: Some(json!({"start": "2026-01-01", "end": "2026-06-01"})),
        source: Some("nisar".into()),
        model: None,
        ..SendContext::default()
    };

    // Turn 1: User asks to evaluate Agent architecture; model performs 1 read-only check
    // (`task_overview`), then hits a mid-loop upstream SSE stream error before answer delivery.
    f.replay.turns.lock().unwrap().extend([
        tool_turn("c-arch-1", "task_overview", json!({})),
        "data: {\"error\":{\"code\":\"server_error\",\"message\":\"Upstream gateway reset mid-stream\"}}\n\n"
            .to_owned(),
    ]);
    let run1 = f
        .host
        .send(
            &f.session,
            "帮我客观评估当前 Agent 架构的稳定性与缓存命中率",
            yinchuan_ctx(),
        )
        .unwrap();
    let err_ev = f.wait_for("error");
    assert_eq!(err_ev.payload["run_id"], run1);
    let err_msg = err_ev.payload["message_zh"].as_str().unwrap_or("");
    assert!(
        err_msg.contains("Upstream gateway reset mid-stream"),
        "Item 11: real sanitized upstream detail must survive to message_zh: {err_msg}"
    );
    assert!(
        !err_msg.contains("返回未知错误"),
        "must not fall back to generic unknown error: {err_msg}"
    );

    // Verify structured resume_checkpoint.json persisted on disk with the completed read step
    let cp_path = f
        .root
        .join("agent")
        .join("sessions")
        .join(&f.session)
        .join("resume_checkpoint.json");
    assert!(
        cp_path.is_file(),
        "resume_checkpoint.json must persist on interrupt"
    );
    let cp_json: Value = serde_json::from_slice(&fs::read(&cp_path).unwrap()).unwrap();
    assert_eq!(cp_json["answer_delivered"], false);
    assert!(cp_json["original_user_goal"]
        .as_str()
        .unwrap_or("")
        .contains("评估当前 Agent 架构"));
    assert_eq!(cp_json["authorized_actions"], json!([]));
    let steps = cp_json["completed_tool_steps"].as_array().unwrap();
    assert_eq!(
        steps.len(),
        1,
        "expected 1 completed read-only check: {cp_json}"
    );
    assert_eq!(steps[0]["tool_id"], "task_overview");
    assert_eq!(steps[0]["ok"], true);
    assert!(
        !steps[0]["output_summary"].as_str().unwrap_or("").is_empty(),
        "completed step must capture real scrubbed evidence"
    );

    let rounds_before_resume = tool_rounds(&f).len();
    assert!(rounds_before_resume >= 2); // running + done for c-arch-1

    // Turn 2: User sends "继续" with the same Yinchuan/NISAR workspace snapshot.
    // Model uses <resume_checkpoint> directly and delivers final answer with 0 repeated checks.
    f.replay.turns.lock().unwrap().push_back(text_turn(json!({
        "text": "基于已核实的运行状态，当前 Agent 架构改进评估如下：1. 保持静态前缀一致以提升缓存命中率；2. 中断后通过结构化检查点直接续跑。",
        "claims": []
    })));
    let run2 = f.host.send(&f.session, "继续", yinchuan_ctx()).unwrap();
    let concl = f.wait_for("conclusion");
    assert_eq!(concl.payload["run_id"], run2);
    assert!(concl.payload["text"]
        .as_str()
        .unwrap_or("")
        .contains("当前 Agent 架构改进评估"));

    // Assert byte-identical static system prompt across Turn 1 and Turn 2 (cache prefix!)
    let seen = f.replay.seen.lock().unwrap();
    let turn1_sys = &seen[0][0].content;
    let turn2_msgs = seen.last().unwrap();
    assert_eq!(
        &turn2_msgs[0].content, turn1_sys,
        "system prompt must stay byte-identical across turns for prompt cache"
    );
    let turn2_user = &turn2_msgs.last().unwrap().content;
    assert!(
        turn2_user.contains("<resume_checkpoint>"),
        "resume turn must inject structured <resume_checkpoint>: {turn2_user}"
    );
    assert!(
        turn2_user.contains("task_overview") && turn2_user.contains("评估当前 Agent 架构"),
        "checkpoint must include original goal & completed tool evidence: {turn2_user}"
    );

    // Assert via actual tool-call log & task store:
    // 0 repeated checks in run2, 0 new searches, 0 new downloads, answer delivered, checkpoint cleared
    let run2_rounds: Vec<Value> = tool_rounds(&f)
        .into_iter()
        .filter(|r| r["run_id"] == run2)
        .collect();
    assert!(
        run2_rounds.is_empty(),
        "resume must execute 0 repeated tool checks: {run2_rounds:?}"
    );
    assert!(
        f.fake.searches.lock().unwrap().is_empty(),
        "resume must trigger 0 new searches"
    );
    assert_eq!(
        f.fake.submits.load(Ordering::SeqCst),
        0,
        "resume must create 0 new download tasks"
    );
    assert!(
        f.fake.created.lock().unwrap().is_empty(),
        "task store must remain untouched"
    );
    assert!(
        !cp_path.exists(),
        "resume_checkpoint.json must be cleared once answer is delivered"
    );
}

#[test]
fn extension_download_tools_obey_session_permission_mode_parity_with_create_task() {
    let f = fixture("ext-download-permission-parity", vec![]);
    *f.fake.extension_tools.lock().unwrap() = vec![
        insar_agent_model::ToolDescription {
            name: "nisar_create_subset_task".into(),
            description: "nisar subset download".into(),
            parameters: json!({"type": "object"}),
        },
        insar_agent_model::ToolDescription {
            name: "gee_chunked_download".into(),
            description: "gee chunked download".into(),
            parameters: json!({"type": "object"}),
        },
    ];

    // 1. Default mode = ConfirmChanges: both extension download tools MUST be refused
    //    without invoking the underlying executor or creating any task.
    assert_eq!(
        f.host
            .permission_mode_get(Some(&f.session))
            .unwrap()
            .effective,
        PermissionMode::ConfirmChanges
    );
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "c-nisar-confirm",
            "nisar_create_subset_task",
            json!({"source_batch_id": "batch-1"}),
        ),
        tool_turn(
            "c-gee-confirm",
            "gee_chunked_download",
            json!({"dataset_id": "COPERNICUS/S2_SR_HARMONIZED"}),
        ),
        text_turn(json!({"text": "已等待用户确认后下载。", "claims": []})),
    ]);
    f.host
        .send(
            &f.session,
            "请下载 NISAR 和 GEE 数据",
            SendContext::default(),
        )
        .unwrap();
    f.wait_for("conclusion");

    assert!(
        f.fake.extension_calls.lock().unwrap().is_empty(),
        "ConfirmChanges MUST NEVER execute nisar_create_subset_task or gee_chunked_download"
    );
    assert_eq!(f.fake.submits.load(Ordering::SeqCst), 0);
    let rounds = tool_rounds(&f);
    for call_id in ["c-nisar-confirm", "c-gee-confirm"] {
        let failed = rounds
            .iter()
            .find(|r| r["call_id"] == call_id && r["status"] == "failed")
            .unwrap_or_else(|| panic!("expected failed tool_round for {call_id}: {rounds:?}"));
        let out_summary = failed["output_summary"].as_str().unwrap_or("");
        assert!(
            out_summary.contains("变更前确认")
                || out_summary.contains("requires_user_confirmation"),
            "Item 9: failed tool_round must include concrete denial reason: {failed}"
        );
    }

    // 2. AutoApprove mode: model-suggested custom output_root is STILL blocked
    //    (parity with create_task OutputDir::ModelSuggested), while default dir is allowed!
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "c-nisar-custom-dir",
            "nisar_create_subset_task",
            json!({"source_batch_id": "batch-1", "output_root": "/tmp/model-suggested"}),
        ),
        tool_turn(
            "c-nisar-auto-ok",
            "nisar_create_subset_task",
            json!({"source_batch_id": "batch-1"}),
        ),
        text_turn(json!({"text": "已在自动批准模式下创建 NISAR 子集任务。", "claims": []})),
    ]);
    f.host
        .send(&f.session, "执行 NISAR 裁剪下载", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);

    let ext_calls = f.fake.extension_calls.lock().unwrap();
    assert_eq!(
        ext_calls.len(),
        1,
        "only the default-dir AutoApprove call should execute: {ext_calls:?}"
    );
    assert_eq!(ext_calls[0].0, "nisar_create_subset_task");
}

#[test]
fn gis_external_execute_obeys_confirm_changes_and_blocks_output_override_in_auto_approve() {
    let f = fixture("gis-external-permission-gate", vec![]);
    *f.fake.extension_tools.lock().unwrap() = vec![insar_agent_model::ToolDescription {
        name: "gis_external_execute".into(),
        description: "external gis execution".into(),
        parameters: json!({"type": "object"}),
    }];

    // 1. ConfirmChanges MUST refuse gis_external_execute without invoking the executor.
    assert_eq!(
        f.host
            .permission_mode_get(Some(&f.session))
            .unwrap()
            .effective,
        PermissionMode::ConfirmChanges
    );
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "c-gis-confirm",
            "gis_external_execute",
            json!({
                "toolchain": "qgis",
                "action": "run_algorithm",
                "algorithm": "native:buffer",
                "output_filename": "buf.geojson",
                "params": {"DISTANCE": 500}
            }),
        ),
        text_turn(json!({"text": "需先经用户确认后才能调用本地 GIS 工具。", "claims": []})),
    ]);
    f.host
        .send(&f.session, "用 QGIS 做缓冲区分析", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");

    assert!(
        f.fake.extension_calls.lock().unwrap().is_empty(),
        "ConfirmChanges MUST NEVER execute gis_external_execute"
    );
    let rounds = tool_rounds(&f);
    let failed = rounds
        .iter()
        .find(|r| r["call_id"] == "c-gis-confirm" && r["status"] == "failed")
        .unwrap_or_else(|| panic!("expected failed tool_round for c-gis-confirm: {rounds:?}"));
    let out_summary = failed["output_summary"].as_str().unwrap_or("");
    assert!(
        out_summary.contains("变更前确认")
            || out_summary.contains("requires_user_confirmation")
            || out_summary.contains("AGENT_APPROVAL_REQUIRED"),
        "expected confirmation denial reason: {failed}"
    );

    // 2. AutoApprove still blocks params OUTPUT override, while allowing clean output_filename.
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "c-gis-bad-out",
            "gis_external_execute",
            json!({
                "toolchain": "qgis",
                "action": "run_algorithm",
                "algorithm": "native:filedownloader",
                "params": {"OUTPUT": "/tmp/evil.bin"}
            }),
        ),
        tool_turn(
            "c-gis-auto-ok",
            "gis_external_execute",
            json!({
                "toolchain": "qgis",
                "action": "run_algorithm",
                "algorithm": "native:buffer",
                "output_filename": "buf.geojson",
                "params": {"DISTANCE": 500}
            }),
        ),
        text_turn(json!({"text": "已在自动批准模式下执行受控缓冲分析。", "claims": []})),
    ]);
    f.host
        .send(&f.session, "执行缓冲分析", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);

    let ext_calls = f.fake.extension_calls.lock().unwrap();
    assert_eq!(
        ext_calls.len(),
        1,
        "only the clean AutoApprove call should execute: {ext_calls:?}"
    );
    assert_eq!(ext_calls[0].0, "gis_external_execute");
}

#[test]
fn open_geodata_query_allows_readonly_search_but_blocks_downloads_under_confirm_changes() {
    let f = fixture("open-geodata-confirm-gate", vec![]);
    *f.fake.extension_tools.lock().unwrap() = vec![insar_agent_model::ToolDescription {
        name: "open_geodata_query".into(),
        description: "open geodata search/download".into(),
        parameters: json!({"type": "object"}),
    }];
    let denial_cases = [
        (
            "c-og-dl-verb",
            json!({"dataset": "aws_sentinel1_grd", "tool_name": "download"}),
        ),
        (
            "c-og-dl-kw",
            json!({"dataset": "whu_clcd", "keyword": "2020..2023|download"}),
        ),
        (
            "c-og-dl-out",
            json!({"dataset": "overture_maps", "output_path": "out.parquet"}),
        ),
        (
            "c-og-dl-gee",
            json!({"dataset": "aws_sentinel1_grd", "gee_params": {"download": true}}),
        ),
        (
            "c-og-blocked-gee-save",
            json!({"dataset": "gee_account_save"}),
        ),
        (
            "c-og-blocked-xyz",
            json!({"dataset": "xyz_satellite_tiles"}),
        ),
        (
            "c-og-blocked-nisar",
            json!({"dataset": "nisar_hdf5_subset"}),
        ),
        ("c-og-blocked-whitebox", json!({"dataset": "whitebox"})),
        (
            "c-og-blocked-duckdb",
            json!({"dataset": "duckdb", "sql": "COPY(SELECT 1) TO '/tmp/x.csv'"}),
        ),
        ("c-og-blocked-era5", json!({"dataset": "open_meteo_era5"})),
    ];

    // Turn 1: one allowed read-only search + first two denials (<3 consecutive failure wrap-up limit).
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "c-og-search",
            "open_geodata_query",
            json!({"dataset": "aws_sentinel1_grd", "keyword": "vv|search"}),
        ),
        tool_turn(
            denial_cases[0].0,
            "open_geodata_query",
            denial_cases[0].1.clone(),
        ),
        tool_turn(
            denial_cases[1].0,
            "open_geodata_query",
            denial_cases[1].1.clone(),
        ),
        text_turn(json!({"text": "检索完成，下载操作已按规则等待确认。", "claims": []})),
    ]);
    f.host
        .send(&f.session, "检索开放数据并尝试下载", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");

    let mut conclusion_count = 1;
    // Remaining pairs stay below the 3-consecutive-failure wrap-up threshold cleanly.
    for pair in denial_cases[2..].chunks(2) {
        for (cid, args) in pair {
            f.replay.turns.lock().unwrap().push_back(tool_turn(
                cid,
                "open_geodata_query",
                args.clone(),
            ));
        }
        f.replay.turns.lock().unwrap().push_back(text_turn(
            json!({"text": "未授权调用均已拒绝。", "claims": []}),
        ));
        f.host
            .send(&f.session, "继续验证权限门禁", SendContext::default())
            .unwrap();
        conclusion_count += 1;
        events_of(&f, "conclusion", conclusion_count);
    }

    let ext_calls = f.fake.extension_calls.lock().unwrap().clone();
    assert_eq!(
        ext_calls.len(),
        1,
        "only read-only open_geodata_query search may run under ConfirmChanges: {ext_calls:?}"
    );
    assert_eq!(ext_calls[0].0, "open_geodata_query");

    let rounds = tool_rounds(&f);
    for (call_id, _) in denial_cases {
        let failed = rounds
            .iter()
            .find(|r| r["call_id"] == call_id && r["status"] == "failed")
            .unwrap_or_else(|| panic!("expected denied tool_round for {call_id}: {rounds:?}"));
        let summary = failed["output_summary"].as_str().unwrap_or("");
        assert!(
            summary.contains("变更前确认") || summary.contains("requires_user_confirmation"),
            "expected AGENT_APPROVAL_REQUIRED reason for {call_id}: {failed}"
        );
    }

    // Even under AutoApprove, gee_params.output_path custom paths must be denied!
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    f.replay.turns.lock().unwrap().extend([
        tool_turn(
            "c-og-auto-gee-escape",
            "open_geodata_query",
            json!({"dataset": "aws_sentinel1_grd", "gee_params": {"download": true, "output_path": "/tmp/escape.tif"}}),
        ),
        text_turn(json!({"text": "已拦截未授权的自定义输出路径。", "claims": []})),
    ]);
    f.host
        .send(&f.session, "测试自定义输出路径拦截", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", conclusion_count + 1);
    assert_eq!(
        f.fake.extension_calls.lock().unwrap().len(),
        1,
        "AutoApprove must NEVER pass gee_params.output_path custom dir to open_geodata_query"
    );
}

#[test]
fn multimodal_image_and_text_attachments_flow_through_send_and_reject_spoofed_images() {
    let f = fixture("multimodal-attachments", vec![]);
    let valid_png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let fake_secret_b64 = "c2stcHJvai1mYWtlMTIzNDU2Nzg5MA==";

    // 1. Spoofed secret disguised as image/png is rejected upfront at `send()`.
    let bad_ctx = SendContext {
        attachments: vec![SendAttachment {
            kind: "image".into(),
            name: "fake.png".into(),
            mime_type: "image/png".into(),
            data_base64: Some(fake_secret_b64.into()),
            text_content: None,
            size_bytes: Some(22),
        }],
        ..SendContext::default()
    };
    assert!(matches!(
        f.host.send(&f.session, "", bad_ctx),
        Err(HostError::Invalid)
    ));

    // 2. Empty text + valid PNG + text file attachment with secret succeeds;
    //    secret is scrubbed while valid PNG travels in `user_multimodal`.
    f.replay.turns.lock().unwrap().push_back(text_turn(
        json!({"text": "已收到图片与附件并完成分析。", "claims": []}),
    ));
    let ok_ctx = SendContext {
        attachments: vec![
            SendAttachment {
                kind: "image".into(),
                name: "aoi_preview.png".into(),
                mime_type: "image/png".into(),
                data_base64: Some(format!("data:image/png;base64,{valid_png_b64}")),
                text_content: None,
                size_bytes: Some(68),
            },
            SendAttachment {
                kind: "text".into(),
                name: "notes.txt".into(),
                mime_type: "text/plain".into(),
                data_base64: None,
                text_content: Some("研究区在银川，注意 sk-proj-FAKE1234567890abc 不得泄露".into()),
                size_bytes: Some(64),
            },
        ],
        ..SendContext::default()
    };
    f.host.send(&f.session, "", ok_ctx).unwrap();
    f.wait_for("conclusion");

    let seen = f.replay.seen.lock().unwrap();
    let sent_user = &seen[0][1];
    assert_eq!(sent_user.role, "user_multimodal");
    assert!(sent_user.content.contains(valid_png_b64));
    assert!(sent_user.content.contains("研究区在银川"));
    assert!(!sent_user.content.contains("sk-proj-FAKE1234567890abc"));
    drop(seen);

    // 3. Text attachment of 50,000 chars (<= 65,536 UI contract limit) is accepted
    //    and clipped cleanly to the 32,000-char outbound snippet cap; > 65,536 is rejected.
    let too_large_ctx = SendContext {
        attachments: vec![SendAttachment {
            kind: "text".into(),
            name: "oversized.txt".into(),
            mime_type: "text/plain".into(),
            data_base64: None,
            text_content: Some("a".repeat(65_537)),
            size_bytes: Some(65_537),
        }],
        ..SendContext::default()
    };
    assert!(matches!(
        f.host.send(&f.session, "", too_large_ctx),
        Err(HostError::Invalid)
    ));

    f.replay.turns.lock().unwrap().push_back(text_turn(
        json!({"text": "已接收 50k 字符文本附件。", "claims": []}),
    ));
    let large_ok_ctx = SendContext {
        attachments: vec![SendAttachment {
            kind: "text".into(),
            name: "large.geojson".into(),
            mime_type: "application/geo+json".into(),
            data_base64: None,
            text_content: Some("x".repeat(50_000)),
            size_bytes: Some(50_000),
        }],
        ..SendContext::default()
    };
    f.host
        .send(&f.session, "分析该 GeoJSON", large_ok_ctx)
        .unwrap();
    events_of(&f, "conclusion", 2);
    let seen_after = f.replay.seen.lock().unwrap();
    let last_user_text = &seen_after.last().unwrap().last().unwrap().content;
    assert!(last_user_text.contains("[附件：large.geojson (application/geo+json)]"));
    assert!(
        last_user_text.chars().count() < 34_000,
        "outbound snippet must remain clipped near 32k chars, got {}",
        last_user_text.chars().count()
    );
}

#[test]
fn unfiled_session_remembers_message_dir_across_turns_without_leaking_path() {
    let f = fixture("remember-message-dir", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-r1", json!({}), vec![sized("S1", GB)], &[]);
    seed_batch(&f, "batch-r2", json!({}), vec![sized("S2", GB)], &[]);
    f.host
        .permission_mode_set_session(&f.session, Some(PermissionMode::AutoApprove))
        .unwrap();
    let dir_a = f
        .root
        .join("remembered-from-msg")
        .to_string_lossy()
        .into_owned();
    assert!(!dir_a.contains(char::is_whitespace), "{dir_a}");

    // Turn 1: user specifies dir_a in the message; model plans with message_dir ref.
    let mut turns_1 = search_then_create(
        &f,
        "r1",
        "batch-r1",
        &["S1"],
        json!({"output_root": {"ref": "message_dir"}}),
    );
    turns_1.push(text_turn(json!({"text": "第一轮完成", "claims": []})));
    f.replay.turns.lock().unwrap().extend(turns_1);

    let run1 = f
        .host
        .send(&f.session, &format!("把影像保存到 {dir_a}"), with_aoi())
        .unwrap();
    events_of(&f, "conclusion", 1);

    // Turn 2: user omits directory completely; model plans without output_root.
    let mut turns_2 = search_then_create(&f, "r2", "batch-r2", &["S2"], json!({}));
    turns_2.push(text_turn(json!({"text": "第二轮完成", "claims": []})));
    f.replay.turns.lock().unwrap().extend(turns_2);

    let run2 = f
        .host
        .send(&f.session, "再帮我下载第二个区域", with_aoi())
        .unwrap();
    events_of(&f, "conclusion", 2);

    let created = f.fake.created.lock().unwrap().clone();
    assert_eq!(created.len(), 2, "{created:?}");
    assert_eq!(created[0]["options"]["output_root"], json!(dir_a));
    assert_eq!(created[1]["options"]["output_root"], json!(dir_a));

    // Verify AGENT-REG-002 outbound & run-record scrubbing on both turns.
    for messages in f.replay.seen.lock().unwrap().iter() {
        for m in messages {
            assert!(
                !m.content.contains(&dir_a),
                "outbound leaked dir_a: {}",
                m.content
            );
        }
    }
    for run_id in [run1, run2] {
        let run_dir = f.root.join(format!("agent/runs/{run_id}"));
        let trace = std::fs::read_to_string(run_dir.join("trace.jsonl")).unwrap();
        assert!(!trace.contains(&dir_a), "trace leaked dir_a in {run_id}");
    }
}

fn events_of_session(f: &Fixture, session_id: &str, kind: &str, count: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let found: Vec<Value> = f
            .host
            .stream_poll(session_id, 0)
            .unwrap()
            .events
            .into_iter()
            .filter(|event| event.kind == kind)
            .map(|event| event.payload)
            .collect();
        if found.len() >= count {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "{} of {count} {kind} events on {session_id}",
            found.len()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn unfiled_session_remembers_chosen_on_card_directory_across_turns() {
    let f = fixture("remember-card-dir", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    seed_batch(&f, "batch-c1", json!({}), vec![sized("S1", GB)], &[]);
    seed_batch(&f, "batch-c2", json!({}), vec![sized("S2", GB)], &[]);

    let chosen_dir = f
        .root
        .join("chosen-card-dir")
        .to_string_lossy()
        .into_owned();
    let mut turns_1 = search_then_create(&f, "c1", "batch-c1", &["S1"], json!({}));
    turns_1.push(text_turn(json!({"text": "第一轮完成", "claims": []})));
    let mut turns_2 = search_then_create(&f, "c2", "batch-c2", &["S2"], json!({}));
    turns_2.push(text_turn(json!({"text": "第二轮完成", "claims": []})));
    f.replay.turns.lock().unwrap().extend(turns_1);
    f.replay.turns.lock().unwrap().extend(turns_2);

    f.host
        .send(&f.session, "帮我检索并下载", with_aoi())
        .unwrap();
    let card1 = events_of(&f, "confirm_card", 1).remove(0);
    f.host
        .plan_approve(
            card1["plan_id"].as_str().unwrap(),
            card1["snapshot"].clone(),
            Some(chosen_dir.clone()),
        )
        .unwrap();
    events_of(&f, "conclusion", 1);

    // Persisted on SessionSummary locally.
    let listed = f.host.sessions_list().unwrap();
    let sess = listed.iter().find(|s| s.session_id == f.session).unwrap();
    assert_eq!(
        sess.remembered_output_dir.as_deref(),
        Some(chosen_dir.as_str())
    );

    f.host
        .send(&f.session, "继续下载下一个批次", with_aoi())
        .unwrap();
    let card2 = events_of(&f, "confirm_card", 2).remove(1);
    assert_eq!(
        card2["snapshot"]["output_dir"],
        json!(chosen_dir),
        "{card2}"
    );
}

#[test]
fn fresh_workspace_session_message_dir_ref_still_fails_closed() {
    let f = fixture("ws-fail-closed", vec![]);
    let ws_dir = f.root.join("ws-folder").to_string_lossy().into_owned();
    let ws = f.host.workspace_create("实验工作区", Some(ws_dir)).unwrap();
    let ws_sess = f
        .host
        .session_create_in(Some("工作区会话".into()), Some(ws.workspace_id))
        .unwrap();
    f.host
        .permission_mode_set_session(&ws_sess.session_id, Some(PermissionMode::FullAccess))
        .unwrap();
    seed_batch(&f, "batch-ws-fc", json!({}), vec![sized("S1", GB)], &[]);
    f.replay.turns.lock().unwrap().extend(search_then_create(
        &f,
        "wfc",
        "batch-ws-fc",
        &["S1"],
        json!({"output_root": {"ref": "message_dir"}}),
    ));
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "完成", "claims": []})));

    f.host
        .send(&ws_sess.session_id, "放到我刚写的那个目录里", with_aoi())
        .unwrap();
    let cards: Vec<Value> = events_of_session(&f, &ws_sess.session_id, "confirm_card", 1);
    let card = &cards[0];
    assert_eq!(
        approval_of(card),
        manual("full_access", json!(["output_dir_unknown"])),
        "{card}"
    );
    assert!(card["snapshot"]["output_dir"].is_null(), "{card}");
}

#[test]
fn workspace_session_defaults_to_workspace_folder_until_session_remembers_override() {
    let f = fixture("ws-default-dir", vec![]);
    f.fake.create_ok.store(true, Ordering::SeqCst);
    let ws_dir = f.root.join("ws-dedicated").to_string_lossy().into_owned();
    let override_dir = f.root.join("ws-override").to_string_lossy().into_owned();
    let ws = f
        .host
        .workspace_create("青藏滑坡工作区", Some(ws_dir.clone()))
        .unwrap();
    let ws_sess = f
        .host
        .session_create_in(Some("青藏监测".into()), Some(ws.workspace_id))
        .unwrap();
    f.host
        .permission_mode_set_session(&ws_sess.session_id, Some(PermissionMode::AutoApprove))
        .unwrap();
    seed_batch(&f, "batch-ws1", json!({}), vec![sized("S1", GB)], &[]);
    seed_batch(&f, "batch-ws2", json!({}), vec![sized("S2", GB)], &[]);
    seed_batch(&f, "batch-ws3", json!({}), vec![sized("S3", GB)], &[]);

    // Turn 1: omits dir -> uses workspace.folder instead of global default_output_root.
    let mut t1 = search_then_create(&f, "w1", "batch-ws1", &["S1"], json!({}));
    t1.push(text_turn(json!({"text": "t1", "claims": []})));
    f.replay.turns.lock().unwrap().extend(t1);
    f.host
        .send(&ws_sess.session_id, "检索并下载第一批", with_aoi())
        .unwrap();
    events_of_session(&f, &ws_sess.session_id, "conclusion", 1);

    // Turn 2: user specifies override_dir in message -> overrides workspace.folder.
    let mut t2 = search_then_create(
        &f,
        "w2",
        "batch-ws2",
        &["S2"],
        json!({"output_root": {"ref": "message_dir"}}),
    );
    t2.push(text_turn(json!({"text": "t2", "claims": []})));
    f.replay.turns.lock().unwrap().extend(t2);
    f.host
        .send(
            &ws_sess.session_id,
            &format!("这一批改放到 {override_dir}"),
            with_aoi(),
        )
        .unwrap();
    events_of_session(&f, &ws_sess.session_id, "conclusion", 2);

    // Turn 3: omits dir again -> keeps per-session remembered override_dir.
    let mut t3 = search_then_create(&f, "w3", "batch-ws3", &["S3"], json!({}));
    t3.push(text_turn(json!({"text": "t3", "claims": []})));
    f.replay.turns.lock().unwrap().extend(t3);
    f.host
        .send(&ws_sess.session_id, "第三批也继续下", with_aoi())
        .unwrap();
    events_of_session(&f, &ws_sess.session_id, "conclusion", 3);

    let created = f.fake.created.lock().unwrap().clone();
    assert_eq!(created.len(), 3, "{created:?}");
    assert_eq!(created[0]["options"]["output_root"], json!(ws_dir));
    assert_eq!(created[1]["options"]["output_root"], json!(override_dir));
    assert_eq!(created[2]["options"]["output_root"], json!(override_dir));
}

#[test]
fn fact_verifier_appends_honest_correction_on_hallucinated_counts_in_conclude() {
    let f = fixture(
        "fact-verifier-conclude",
        vec![
            tool_turn("sp1", "search_progress", json!({"request_id": "req-1"})),
            raw_turn(
                "检索已完成，共找到 99 景影像，已创建 2 个下载任务，高质量占比 3/8（42.5%）。",
            ),
        ],
    );
    let run_id = f
        .host
        .send(&f.session, "帮我查一下最近影像", SendContext::default())
        .unwrap();
    let conc = f.wait_for("conclusion").payload;
    assert_eq!(conc["run_id"], json!(run_id));
    let text = conc["text"].as_str().unwrap_or("");
    assert!(text.contains("本轮数据核对提示"), "{text}");
    assert!(
        text.contains("0 景")
            || text.contains("1 景")
            || text.contains("2 景")
            || text.contains("实测"),
        "{text}"
    );
    assert!(text.contains("尚未创建任何下载任务"), "{text}");
    assert!(text.contains("37.5%"), "{text}");
    assert_eq!(f.run_status(&run_id), "completed");
}

#[test]
fn turn_0_pure_casual_greeting_uses_fast_path_zero_tools_and_short_prompt() {
    for (idx, greeting) in ["hi", "你好！"].into_iter().enumerate() {
        let f = fixture(
            &format!("casual-greeting-{idx}"),
            vec![raw_turn("你好！有什么可以帮你的？")],
        );
        let run_id = f
            .host
            .send(&f.session, greeting, SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", 1);
        let body = first_outbound(&f, &run_id);
        let tools = body["tools"].as_array().cloned().unwrap_or_default();
        assert!(
            tools.is_empty(),
            "turn-0 `{greeting}` must not send tool schemas: {tools:?}"
        );
        let system = body["messages"][0]["content"].as_str().unwrap_or_default();
        assert!(
            system.chars().count() < 400,
            "casual system prompt should stay compact, got {} chars",
            system.chars().count()
        );
        let tools_seen = f
            .replay
            .tools_seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default();
        assert!(tools_seen.is_empty());

        // Turn 1 greeting MUST NOT take turn-0 fast-path once history exists.
        f.replay.turns.lock().unwrap().push_back(text_turn(
            json!({"text": "仍在的，请问需要什么数据？", "claims": []}),
        ));
        f.host
            .send(&f.session, "你好", SendContext::default())
            .unwrap();
        events_of(&f, "conclusion", 2);
        let tools_turn1 = f
            .replay
            .tools_seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default();
        assert!(
            !tools_turn1.is_empty(),
            "turn-1 follow-up must restore full tools"
        );
    }
}

#[test]
fn turn_0_data_request_uses_full_prompt_and_tools() {
    let f = fixture(
        "turn0-data-query",
        vec![text_turn(
            json!({"text": "收到，我来帮你看工作区数据。", "claims": []}),
        )],
    );
    let run_id = f
        .host
        .send(&f.session, "帮我检索哨兵一号数据", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let body = first_outbound(&f, &run_id);
    let tools = body["tools"].as_array().unwrap();
    assert!(tools
        .iter()
        .any(|t| t["function"]["name"] == "propose_plan"));
    let system = body["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("EOStage 的数据准备助手"));
}

#[test]
fn compact_source_list_array_is_preserved_and_summarized() {
    let f = fixture("compact-source-list", vec![]);
    *f.fake.sources.lock().unwrap() = json!([
        {
            "resource_id": "science.sentinel-1.slc.v1",
            "display_name_zh": "Sentinel-1 SLC",
            "family": "sar",
            "availability": "available",
            "availability_reason_zh": null,
            "accounts": [],
            "filter_schema_id": "legacy.sar",
            "filter_schema_revision": 3,
            "group_id": "sar.s1",
            "network_default": "direct"
        },
        {
            "resource_id": "science.landsat.c2l2.v1",
            "display_name_zh": "Landsat C2 L2",
            "family": "optical",
            "availability": "available",
            "availability_reason_zh": "公开访问",
            "accounts": ["mpc"]
        }
    ]);
    f.replay.turns.lock().unwrap().extend([
        tool_turn("s1", "source_list", json!({})),
        text_turn(json!({"text": "共有两个数据源", "claims": []})),
    ]);
    let run_id = f
        .host
        .send(&f.session, "有哪些数据源", SendContext::default())
        .unwrap();
    f.wait_for("conclusion");
    let result = last_tool_result(&f);
    assert!(result.is_array(), "{result}");
    assert_eq!(result[0].get("filter_schema_id"), None);
    assert_eq!(result[0].get("network_default"), None);
    assert_eq!(result[0].get("accounts"), None);
    assert_eq!(result[0]["resource_id"], "science.sentinel-1.slc.v1");
    assert_eq!(result[1]["accounts"], json!(["mpc"]));
    assert_eq!(f.run_status(&run_id), "completed");
}

#[test]
fn multi_turn_append_only_prefix_is_byte_identical_across_turns_and_after_compaction() {
    let f = fixture("append-only-and-compaction", vec![]);
    let sess_dir = f.root.join("agent/sessions").join(&f.session);

    // Turns 1..3 stay below 96k trigger -> Turn N's outbound messages must be a strict
    // byte-identical prefix of Turn N+1's outbound messages!
    let mut prev_body: Option<Value> = None;
    for i in 0..3usize {
        f.replay.turns.lock().unwrap().extend([
            tool_turn(&format!("t-{i}"), "task_overview", json!({})),
            text_turn(
                json!({"text": format!("第{i}轮已完成检查：{}", "答".repeat(2_000)), "claims": []}),
            ),
        ]);
        let run_id = f
            .host
            .send(
                &f.session,
                &format!("轮次{i} {}", "问".repeat(7_500)),
                SendContext::default(),
            )
            .unwrap();
        events_of(&f, "conclusion", i + 1);
        let bodies = outbound_bodies(&f, &run_id);
        let first_body = &bodies[0];
        if let Some(ref prev) = prev_body {
            let prev_msgs = prev["messages"].as_array().unwrap();
            let curr_msgs = first_body["messages"].as_array().unwrap();
            assert_eq!(
                &curr_msgs[..prev_msgs.len()],
                prev_msgs.as_slice(),
                "Turn {i} must preserve Turn {}'s exact message prefix",
                i - 1
            );
        }
        prev_body = Some(bodies.last().unwrap().clone());
        assert!(!sess_dir.join("compaction.json").exists());
    }

    // Push past 96,000 chars to trigger one-shot hysteresis compaction.
    for i in 3..8usize {
        f.replay.turns.lock().unwrap().push_back(text_turn(
            json!({"text": format!("第{i}轮回复 {}", "文".repeat(12_000)), "claims": []}),
        ));
        f.host
            .send(
                &f.session,
                &format!("轮次{i} {}", "题".repeat(7_500)),
                SendContext::default(),
            )
            .unwrap();
        events_of(&f, "conclusion", i + 1);
    }

    // Trigger check on next turn (compaction writes compaction.json)
    f.replay.turns.lock().unwrap().push_back(text_turn(
        json!({"text": "压缩后首轮正常应答", "claims": []}),
    ));
    let post_compact = f
        .host
        .send(&f.session, "压缩后继续", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 9);
    assert!(
        sess_dir.join("compaction.json").is_file(),
        "compaction.json must persist once trigger threshold is exceeded"
    );
    let post_compact_body = outbound_bodies(&f, &post_compact).pop().unwrap();

    // Next turn after compaction appends verbatim against frozen compaction prefix!
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "再续一轮", "claims": []})));
    let after_post_compact = f
        .host
        .send(&f.session, "再续一轮", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 10);
    let after_body = &outbound_bodies(&f, &after_post_compact)[0];
    let pc_msgs = post_compact_body["messages"].as_array().unwrap();
    let after_msgs = after_body["messages"].as_array().unwrap();
    assert_eq!(
        &after_msgs[..pc_msgs.len()],
        pc_msgs.as_slice(),
        "turns following compaction must stay byte-identical on the compacted prefix"
    );

    // Truncating before the compacted prefix invalidates compaction.json cleanly
    let events = f.host.stream_poll(&f.session, 0).unwrap().events;
    let first_user_seq = events
        .iter()
        .find(|e| e.kind == "message" && e.payload["role"] == "user")
        .unwrap()
        .seq;
    let second_user_seq = events
        .iter()
        .filter(|e| e.kind == "message" && e.payload["role"] == "user")
        .nth(1)
        .unwrap()
        .seq;
    assert!(second_user_seq > first_user_seq);
    f.host
        .session_truncate(&f.session, second_user_seq, false)
        .unwrap();
    assert!(
        !sess_dir.join("compaction.json").exists(),
        "stale compaction.json must be invalidated on session_truncate"
    );

    // Deleting the session cleanly removes compaction/toolset/resume files and the directory
    f.host.session_delete(&f.session).unwrap();
    assert!(!sess_dir.exists());
}

#[test]
fn parked_plan_resume_and_next_turn_retain_activated_toolsets_and_wrap_up_keeps_tools() {
    let f = fixture("sticky-toolset-and-wrap-up", vec![]);
    *f.fake.extension_tools.lock().unwrap() = vec![
        insar_agent_model::ToolDescription {
            name: "toolset_activate".into(),
            description: "activate".into(),
            parameters: json!({"type": "object"}),
        },
        insar_agent_model::ToolDescription {
            name: "gee_catalog_search".into(),
            description: "gee".into(),
            parameters: json!({"type": "object"}),
        },
    ];
    f.fake.working_aoi.lock().unwrap().clone_from(&json!({
        "state": "polygon",
        "name": "测试区域",
        "bbox": [100.0, 30.0, 101.0, 31.0],
        "geometry": {"type":"Polygon","coordinates":[[[100.0,30.0],[101.0,30.0],[101.0,31.0],[100.0,31.0],[100.0,30.0]]]}
    }));
    let revision = f.revision();
    // Step 0: activate gee
    // Step 1: propose_plan(pause_plan -> tier=2 -> parks on confirm_card under confirm_changes!) -> resume
    // Steps 2..6: alternating read calls (no RepeatedCall!)
    // Step 7: wrap_up (step_index=7, max_steps=8) -> model receives wrap-up prompt AND SAME tools -> concludes!
    f.replay.turns.lock().unwrap().extend([
        tool_turn("a1", "toolset_activate", json!({"toolsets": ["gee"]})),
        tool_turn("p1", "propose_plan", pause_plan(revision)),
        tool_turn("r2", "task_overview", json!({})),
        tool_turn("r3", "service_readiness", json!({})),
        tool_turn("r4", "task_overview", json!({})),
        tool_turn("r5", "service_readiness", json!({})),
        tool_turn("r6", "task_overview", json!({})),
        text_turn(json!({"text": "已到步数上限，总结完毕。", "claims": []})),
    ]);
    let run1 = f
        .host
        .send(&f.session, "帮我看看当前任务情况", SendContext::default())
        .unwrap();
    let card = f.wait_for("confirm_card").payload;
    f.wait_for("plan_card");
    f.host
        .plan_approve(
            card["plan_id"].as_str().unwrap(),
            card["snapshot"].clone(),
            None,
        )
        .unwrap();
    let conc1 = f.wait_for("conclusion");
    assert_eq!(conc1.payload["run_id"], run1);

    let tools_seen = f.replay.tools_seen.lock().unwrap().clone();
    assert_eq!(tools_seen.len(), 8, "{tools_seen:?}");
    assert!(!tools_seen[0].iter().any(|n| n == "gee_catalog_search"));
    assert!(tools_seen[1].iter().any(|n| n == "gee_catalog_search"));
    // Resumed steps (2..6) AND wrap-up step (7) keep the exact same toolset!
    for step_idx in 2..8 {
        assert_eq!(
            tools_seen[step_idx], tools_seen[1],
            "step {step_idx} must preserve exact tool array for KV cache"
        );
    }
    // Verify wrap-up prompt was injected on step 7 while keeping tools intact
    let seen_msgs = f.replay.seen.lock().unwrap();
    assert!(
        seen_msgs[7]
            .last()
            .unwrap()
            .content
            .contains("已达单轮工具调用上限")
            || outbound_bodies(&f, &run1).last().unwrap()["messages"]
                .to_string()
                .contains("已达单轮工具调用上限")
    );
    drop(seen_msgs);

    // Reopen AgentHost (simulating desktop restart) and send Turn 2 WITHOUT any GEE keyword:
    // sticky toolset_state.json keeps gee_catalog_search active from step 0!
    let reopened = AgentHost::open(
        f.root.clone(),
        f.fake.clone(),
        Source(Arc::clone(&f.replay)),
    )
    .unwrap();
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "好的", "claims": []})));
    reopened
        .send(&f.session, "继续下一步", SendContext::default())
        .unwrap();
    events_of_session_host(&reopened, &f.session, "conclusion", 2);
    let tools_seen_after = f.replay.tools_seen.lock().unwrap();
    assert_eq!(
        tools_seen_after.last().unwrap(),
        &tools_seen[1],
        "reopened Turn 2 must preserve sticky active toolsets without keyword repeat"
    );
}

fn events_of_session_host(
    host: &AgentHost<Fake, Source>,
    session_id: &str,
    kind: &str,
    count: usize,
) -> Vec<Value> {
    for _ in 0..200 {
        let found: Vec<Value> = host
            .stream_poll(session_id, 0)
            .unwrap()
            .events
            .into_iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.payload)
            .collect();
        if found.len() >= count {
            return found;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {count} {kind} events");
}

#[test]
fn interrupted_and_partial_parallel_tool_turns_persist_valid_transcript_and_scope_resume_checkpoint(
) {
    let f = fixture("partial-parallel-and-scoped-resume", vec![]);
    // Turn 1 completes 1 tool call (service_readiness) cleanly.
    f.replay.turns.lock().unwrap().extend([
        tool_turn("t1-call", "service_readiness", json!({})),
        text_turn(json!({"text": "Turn 1 OK", "claims": []})),
    ]);
    f.host
        .send(&f.session, "先检查服务状态", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);

    // Turn 2 emits parallel tool calls [c1, c2], where c1 runs (task_overview) and
    // c2 is propose_plan (which parks, or if a model error / cancel happens before c2 finishes,
    // wait: test partial parallel by having c1=task_overview and c2=propose_plan under cancel!).
    // Even simpler: c1=task_overview, c2=propose_plan -> parked -> run_cancel!
    // That leaves Turn 2's checkpoint with [..., assistant_tool_calls([c1, c2]), tool(c1)] (incomplete parallel tail!).
    let parallel_frame = frame(json!({
        "content": "准备执行:\n第一步查看",
        "tool_calls": [
            {"index": 0, "id": "p-read", "function": {"name": "task_overview", "arguments": "{}"}},
            {"index": 1, "id": "p-plan", "function": {"name": "propose_plan", "arguments": pause_plan(f.revision()).to_string()}}
        ]
    }));
    f.replay.turns.lock().unwrap().push_back(parallel_frame);
    let run2 = f
        .host
        .send(&f.session, "暂停任务", SendContext::default())
        .unwrap();
    let _card = f.wait_for("confirm_card");
    f.wait_for("plan_card");
    assert!(f.host.run_cancel(&run2).unwrap());
    f.wait_for("error");

    // Verify Turn 2's persisted turn_messages.json dropped BOTH incomplete assistant_tool_calls AND orphan tool(p-read)
    let tm2_path = f
        .root
        .join("agent/runs")
        .join(&run2)
        .join("turn_messages.json");
    let tm2: Vec<insar_agent_model::Message> =
        serde_json::from_slice(&fs::read(&tm2_path).unwrap()).unwrap();
    assert_eq!(
        tm2.len(),
        2,
        "incomplete parallel tail must be peeled: {tm2:?}"
    );
    assert_eq!(tm2[0].role, "user");
    assert_eq!(tm2[1].role, "assistant");

    // Turn 3 fails immediately at step 0 (HTTP 500/stream error) after Turn 1 had tools:
    // Turn 3 must NOT panic on empty checkpoint slice AND must NOT absorb Turn 1's tools into resume_checkpoint!
    f.replay.turns.lock().unwrap().push_back(
        "data: {\"error\":{\"code\":\"server_error\",\"message\":\"step0 boom\"}}\n\n".to_owned(),
    );
    let run3 = f
        .host
        .send(&f.session, "第三轮请求", SendContext::default())
        .unwrap();
    let err3 = events_of(&f, "error", 2);
    let err3_payload = err3.last().unwrap();
    assert_eq!(err3_payload["run_id"], run3);
    assert!(
        !err3_payload["message_zh"]
            .as_str()
            .unwrap_or("")
            .contains("已完成"),
        "step-0 error must not falsely claim completed tools from Turn 1: {err3_payload}"
    );
    let cp_json: Value = serde_json::from_slice(
        &fs::read(
            f.root
                .join("agent/sessions")
                .join(&f.session)
                .join("resume_checkpoint.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        cp_json["completed_tool_steps"].as_array().unwrap().len(),
        0,
        "resume_checkpoint must be scoped to current turn only: {cp_json}"
    );

    // Turn 4 succeeds cleanly (confirming Turn 2 + Turn 3 sanitized transcripts replay without error)
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "恢复正常", "claims": []})));
    f.host
        .send(&f.session, "第四轮正常问", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);
}

#[test]
fn outbound_messages_is_byte_identical_idempotent_and_colon_newline_in_tool_calls_persists() {
    use insar_agent::outbound;
    use insar_agent_model::Message;

    let f = fixture("colon-newline-and-multimodal", vec![]);
    let valid_png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    // 1. Verify outbound::messages is byte-for-byte idempotent across roles
    let sample = vec![
        Message {
            role: "system".into(),
            content: "静态系统提示词".into(),
        },
        Message {
            role: "user".into(),
            content: "请查 D:\\temp\\demo 后继续".into(),
        },
        Message {
            role: "assistant_tool_calls".into(),
            content: json!({
                "text": "参数如下:\n请等待核验",
                "tool_calls": [{"id": "c1", "name": "task_overview", "arguments": {"note": "x:\ny"}}]
            })
            .to_string(),
        },
        Message {
            role: "tool".into(),
            content: json!({"id": "c1", "result": {"ok": true, "summary": "结果:\n10条"}})
                .to_string(),
        },
        Message {
            role: "assistant".into(),
            content: "结论:\n已完成".into(),
        },
    ];
    let pass1 = outbound::messages(&sample);
    let pass2 = outbound::messages(&pass1);
    for (a, b) in pass1.iter().zip(pass2.iter()) {
        assert_eq!(a.role, b.role);
        assert_eq!(
            a.content, b.content,
            "outbound::messages must be idempotent"
        );
    }

    // 2. End-to-end: Chinese ":\n" inside assistant_tool_calls text + multimodal image in Turn 1
    //    must persist valid turn_messages.json (downgrading image) and replay cleanly in Turn 2.
    let tool_with_colon_nl = frame(json!({
        "content": "准备调用工具:\n查询中",
        "tool_calls": [{"index": 0, "id": "c-cn", "function": {"name": "task_overview", "arguments": "{}"}}]
    }));
    f.replay.turns.lock().unwrap().extend([
        tool_with_colon_nl,
        text_turn(json!({"text": "已看完截图并核对队列", "claims": []})),
    ]);
    let mut ctx1 = SendContext::default();
    ctx1.attachments.push(insar_agent::session::SendAttachment {
        kind: "image".into(),
        name: "preview.png".into(),
        mime_type: "image/png".into(),
        data_base64: Some(valid_png_b64.into()),
        text_content: None,
        size_bytes: Some(68),
    });
    let run1 = f.host.send(&f.session, "看看这张图并查状态", ctx1).unwrap();
    events_of(&f, "conclusion", 1);

    let tm1_path = f
        .root
        .join("agent/runs")
        .join(&run1)
        .join("turn_messages.json");
    assert!(
        tm1_path.is_file(),
        "assistant_tool_calls with Chinese ':\\n' must not be rejected by per-role safe_value"
    );
    let tm1_raw = fs::read_to_string(&tm1_path).unwrap();
    assert!(
        !tm1_raw.contains(valid_png_b64),
        "raw base64 must be downgraded"
    );
    assert!(tm1_raw.contains("附带 1 张图片"));

    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "第二轮收到", "claims": []})));
    let run2 = f
        .host
        .send(&f.session, "继续", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 2);
    let body2 = outbound_bodies(&f, &run2).pop().unwrap();
    assert!(body2["messages"]
        .to_string()
        .contains("准备调用工具:\\n查询中"));
}

#[test]
fn compacted_toolsets_survive_session_truncate_while_truncated_toolsets_drop() {
    let f = fixture("compacted-toolsets-truncate", vec![]);
    let sess_dir = f.root.join("agent/sessions").join(&f.session);
    *f.fake.extension_tools.lock().unwrap() = vec![
        insar_agent_model::ToolDescription {
            name: "toolset_activate".into(),
            description: "activate".into(),
            parameters: json!({"type": "object"}),
        },
        insar_agent_model::ToolDescription {
            name: "gee_catalog_search".into(),
            description: "gee".into(),
            parameters: json!({"type": "object"}),
        },
        insar_agent_model::ToolDescription {
            name: "nisar_subset_preflight".into(),
            description: "nisar".into(),
            parameters: json!({"type": "object"}),
        },
    ];

    // Turn 0 activates GEE (inside what will become the compacted prefix).
    f.replay.turns.lock().unwrap().extend([
        tool_turn("a-gee", "toolset_activate", json!({"toolsets": ["gee"]})),
        text_turn(json!({"text": format!("GEE 已完成：{}", "详".repeat(12_000)), "claims": []})),
    ]);
    let run0 = f
        .host
        .send(
            &f.session,
            &format!("首轮分析 {}", "字".repeat(7_500)),
            SendContext::default(),
        )
        .unwrap();
    events_of(&f, "conclusion", 1);

    // Push past 96,000 chars so Turn 0 compacts into compaction.json.
    for i in 1..8usize {
        f.replay.turns.lock().unwrap().push_back(text_turn(
            json!({"text": format!("第{i}轮 {}", "文".repeat(12_000)), "claims": []}),
        ));
        f.host
            .send(
                &f.session,
                &format!("轮次{i} {}", "题".repeat(7_500)),
                SendContext::default(),
            )
            .unwrap();
        events_of(&f, "conclusion", i + 1);
    }

    // Turn 8 triggers compaction while keeping tail and adds a neutral reply.
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "触发压缩", "claims": []})));
    f.host
        .send(&f.session, "压缩总结", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 9);
    assert!(sess_dir.join("compaction.json").is_file());

    // Turn 9 activates NISAR AFTER compaction, then we truncate back to before Turn 9.
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "已关注 NISAR", "claims": []})));
    let truncated_run = f
        .host
        .send(&f.session, "再帮我看看 NISAR 数据", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 10);
    let nisar_tools = f.replay.tools_seen.lock().unwrap().last().unwrap().clone();
    assert!(nisar_tools.iter().any(|n| n == "nisar_subset_preflight"));

    let turn9_seq = f
        .host
        .stream_poll(&f.session, 0)
        .unwrap()
        .events
        .into_iter()
        .find(|e| {
            e.kind == "message"
                && e.payload["role"] == "user"
                && e.payload["run_id"] == truncated_run
        })
        .unwrap()
        .seq;
    f.host
        .session_truncate(&f.session, turn9_seq, false)
        .unwrap();
    assert!(sess_dir.join("compaction.json").is_file());
    assert!(!f.root.join("agent/runs").join(&truncated_run).exists());
    assert!(f.root.join("agent/runs").join(&run0).is_dir());

    // Turn 10 (after truncation) has no keyword: must preserve GEE from compaction.compacted_toolsets
    // while dropping NISAR from the truncated turn!
    f.replay
        .turns
        .lock()
        .unwrap()
        .push_back(text_turn(json!({"text": "继续", "claims": []})));
    f.host
        .send(&f.session, "继续下一步", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 10);
    let post_truncate_tools = f.replay.tools_seen.lock().unwrap().last().unwrap().clone();
    assert!(
        post_truncate_tools
            .iter()
            .any(|n| n == "gee_catalog_search"),
        "compacted GEE toolset must survive session_truncate: {post_truncate_tools:?}"
    );
    assert!(
        !post_truncate_tools
            .iter()
            .any(|n| n == "nisar_subset_preflight"),
        "truncated turn's NISAR toolset must NOT leak past session_truncate: {post_truncate_tools:?}"
    );
}

#[test]
fn turn_0_casual_greeting_then_turn_1_tool_query_then_turn_2_preserves_cache_prefix_and_toolsets() {
    let f = fixture(
        "casual-then-tools",
        vec![raw_turn("你好！请问有什么可以帮您？")],
    );
    let sess_dir = f.root.join("agent/sessions").join(&f.session);
    *f.fake.extension_tools.lock().unwrap() = vec![
        insar_agent_model::ToolDescription {
            name: "toolset_activate".into(),
            description: "activate".into(),
            parameters: json!({"type": "object"}),
        },
        insar_agent_model::ToolDescription {
            name: "gee_catalog_search".into(),
            description: "gee".into(),
            parameters: json!({"type": "object"}),
        },
    ];

    // Turn 0: casual greeting uses fast-path (compact system prompt, 0 tools, no toolset_state.json).
    let run0 = f
        .host
        .send(&f.session, "hi", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 1);
    let body0 = first_outbound(&f, &run0);
    assert!(body0["tools"].as_array().is_none_or(Vec::is_empty));
    assert!(!sess_dir.join("toolset_state.json").exists());
    let turn0_msgs: Vec<insar_agent_model::Message> = serde_json::from_slice(
        &std::fs::read(f.root.join(format!("agent/runs/{run0}/turn_messages.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(turn0_msgs.len(), 2);
    assert_eq!(turn0_msgs[0].role, "user");
    assert_eq!(turn0_msgs[1].role, "assistant");

    // Turn 1: geospatial tool query restores full system prompt + tools + GEE toolset.
    f.replay.turns.lock().unwrap().extend([
        tool_turn("a-gee", "toolset_activate", json!({"toolsets": ["gee"]})),
        tool_turn("g-1", "gee_catalog_search", json!({"query": "Sentinel-2"})),
        text_turn(json!({"text": "已为您检索到 GEE 目录结果。", "claims": []})),
    ]);
    let run1 = f
        .host
        .send(
            &f.session,
            "请用 GEE 检索 Sentinel-2",
            SendContext::default(),
        )
        .unwrap();
    events_of(&f, "conclusion", 2);
    let bodies1 = outbound_bodies(&f, &run1);
    assert!(bodies1[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("EOStage 的数据准备助手"));
    assert!(!bodies1[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("日常问候阶段"));
    assert!(sess_dir.join("toolset_state.json").is_file());

    // Turn 2: follow-up reuses byte-identical message prefix from Turn 0 + Turn 1 and retains toolsets.
    f.replay.turns.lock().unwrap().push_back(text_turn(
        json!({"text": "好的，这是进一步对比说明。", "claims": []}),
    ));
    let run2 = f
        .host
        .send(&f.session, "简短对比一下结果", SendContext::default())
        .unwrap();
    events_of(&f, "conclusion", 3);
    let body2 = first_outbound(&f, &run2);
    let last_step_turn1 = bodies1.last().unwrap()["messages"].as_array().unwrap();
    let msgs2 = body2["messages"].as_array().unwrap();
    assert_eq!(
        &msgs2[..last_step_turn1.len()],
        last_step_turn1.as_slice(),
        "Turn 2 must replay Turn 0 + Turn 1 transcript as a byte-identical prefix"
    );
    assert!(f
        .replay
        .tools_seen
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .iter()
        .any(|t| t == "gee_catalog_search"));
}

#[test]
fn final_step_guard_allows_read_only_open_geodata_query_and_blocks_download() {
    let f = fixture("final-step-open-geodata", vec![]);
    f.host.set_max_steps(6);
    *f.fake.extension_tools.lock().unwrap() = vec![insar_agent_model::ToolDescription {
        name: "open_geodata_query".into(),
        description: "query open geodata".into(),
        parameters: json!({"type": "object"}),
    }];
    // Burn steps 0..4 with distinct read-only queries, then step 5 (final step) tests read vs download.
    let mut turns = vec![
        tool_turn(
            "ro-0",
            "open_geodata_query",
            json!({"dataset": "aws_sentinel1_grd", "tool_name": "search", "keyword": "k0"}),
        ),
        tool_turn(
            "ro-1",
            "open_geodata_query",
            json!({"dataset": "noaa_aws_weather", "tool_name": "search", "keyword": "k1"}),
        ),
        tool_turn(
            "ro-2",
            "open_geodata_query",
            json!({"dataset": "overture_maps", "tool_name": "search", "keyword": "k2"}),
        ),
        tool_turn(
            "ro-3",
            "open_geodata_query",
            json!({"dataset": "aws_sentinel1_grd", "tool_name": "search", "keyword": "k3"}),
        ),
        tool_turn(
            "ro-4",
            "open_geodata_query",
            json!({"dataset": "noaa_aws_weather", "tool_name": "search", "keyword": "k4"}),
        ),
        // Step 5 (final step: current_step + 1 == 6 == max_steps):
        // 1) Read-only allowlisted query MUST execute (not blocked by final_step_wrap_up).
        // 2) Download query MUST be blocked with error == "final_step_wrap_up".
        frame(json!({
            "tool_calls": [
                {
                    "index": 0,
                    "id": "ro-5",
                    "function": {
                        "name": "open_geodata_query",
                        "arguments": json!({"dataset": "overture_maps", "tool_name": "search", "keyword": "k5"}).to_string()
                    }
                },
                {
                    "index": 1,
                    "id": "dl-5",
                    "function": {
                        "name": "open_geodata_query",
                        "arguments": json!({"dataset": "aws_sentinel1_grd", "tool_name": "download"}).to_string()
                    }
                }
            ]
        })),
    ];
    f.replay.turns.lock().unwrap().extend(turns.drain(..));
    let run_id = f
        .host
        .send(&f.session, "执行最终步开放数据核验", SendContext::default())
        .unwrap();
    let errs = events_of(&f, "error", 1);
    assert_eq!(errs[0]["code"], "step_limit");
    assert_eq!(f.run_status(&run_id), "failed");
    let ext_calls = f.fake.extension_calls.lock().unwrap().clone();
    assert_eq!(
        ext_calls.len(),
        6,
        "all 6 read-only calls should execute: {ext_calls:?}"
    );
    let tm_str = std::fs::read_to_string(
        f.root
            .join(format!("agent/runs/{run_id}/turn_messages.json")),
    )
    .unwrap();
    assert!(
        tm_str.contains("final_step_wrap_up"),
        "download call on final step must return final_step_wrap_up: {tm_str}"
    );
}
