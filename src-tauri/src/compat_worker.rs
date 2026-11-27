use insar_application_service::CapabilityRuntimeObservations;
use insar_core::{BuildCapabilityState, RuntimeReadinessState};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PROTOCOL: &str = "eostage.compat-worker.v1";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(50);
const COMPONENT_CONTRACT_JSON: &str =
    include_str!("../../contracts/compat-worker-component.v1.json");
const TEST_WORKER_OVERRIDE_ENV: &str = "EOSTAGE_COMPAT_WORKER_TEST_OVERRIDE";
const TEST_COMPONENT_MANIFEST_ENV: &str = "EOSTAGE_COMPAT_WORKER_COMPONENT_MANIFEST";

const ALLOWED_OPERATIONS: &[&str] = &[
    "worker_capabilities",
    "aoi_import",
    "dem_inspect",
    "dem_convert",
    "dem_prepare_sarscape",
];

const SIDE_EFFECT_OPERATIONS: &[&str] = &["dem_convert", "dem_prepare_sarscape"];

#[derive(Debug, Clone, Deserialize)]
struct ComponentManifest {
    component_id: String,
    protocol: String,
    relative_path: String,
    executable_name: String,
    version: String,
    sha256: String,
    supported_operations: Vec<String>,
    offline_only: bool,
    store_access: bool,
    credential_access: bool,
    download_queue_access: bool,
    lifecycle_owner: String,
    task_id_owner: String,
}

#[derive(Debug, Clone)]
struct VerifiedComponent {
    path: PathBuf,
    manifest: ComponentManifest,
}

#[derive(Debug, Clone)]
struct ComponentIdentity {
    component_id: String,
    version: String,
    sha256: String,
    path: PathBuf,
}

impl VerifiedComponent {
    fn identity(&self) -> ComponentIdentity {
        ComponentIdentity {
            component_id: self.manifest.component_id.clone(),
            version: self.manifest.version.clone(),
            sha256: self.manifest.sha256.clone(),
            path: self.path.clone(),
        }
    }
}

#[derive(Debug)]
struct PendingRequest {
    operation: String,
    side_effect: bool,
    generation: u64,
}

#[derive(Debug)]
enum WorkerFrame {
    Response(Value),
    ReadError(&'static str),
    Eof,
}

#[derive(Debug, PartialEq, Eq)]
enum ShutdownState {
    Running,
    Shutdown,
}

/// The single process/session owner. Calls hold the outer state mutex while
/// sending and awaiting one response, which makes the queue serial and keeps
/// writes and request/response pairing deterministic without replaying work.
struct WorkerManager {
    child: Option<Child>,
    stdin: Option<std::process::ChildStdin>,
    stdout_reader: Option<JoinHandle<()>>,
    responses: Option<Receiver<WorkerFrame>>,
    next_request_id: u64,
    pending: HashMap<String, PendingRequest>,
    process_generation: u64,
    shutdown_state: ShutdownState,
    restart_count: u8,
    restart_pending: bool,
    restart_blocked: bool,
    verified_component_identity: Option<ComponentIdentity>,
}

impl Default for WorkerManager {
    fn default() -> Self {
        Self {
            child: None,
            stdin: None,
            stdout_reader: None,
            responses: None,
            next_request_id: 1,
            pending: HashMap::new(),
            process_generation: 0,
            shutdown_state: ShutdownState::Running,
            restart_count: 0,
            restart_pending: false,
            restart_blocked: false,
            verified_component_identity: None,
        }
    }
}

pub struct CompatWorkerState {
    shutdown_started: AtomicBool,
    timeout: Duration,
    shutdown_timeout: Duration,
    component: Option<VerifiedComponent>,
    startup_error: Option<&'static str>,
    smoke_mode: bool,
    manager: Mutex<WorkerManager>,
}

impl CompatWorkerState {
    pub fn from_environment() -> Self {
        let timeout = std::env::var("EOSTAGE_COMPAT_WORKER_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
            .filter(|value| !value.is_zero())
            .unwrap_or(DEFAULT_TIMEOUT);
        let (component, startup_error) = match resolve_component() {
            Ok(component) => (Some(component), None),
            Err(code) => (None, Some(code)),
        };
        Self {
            shutdown_started: AtomicBool::new(false),
            timeout,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
            component,
            startup_error,
            smoke_mode: false,
            manager: Mutex::new(WorkerManager::default()),
        }
    }

    /// Deterministic offline Worker boundary used only by the packaged UI
    /// smoke. It exercises the same allowlist and ApplicationService port as
    /// the managed component, but never starts a child process or touches a
    /// provider. Store, credentials, and download scheduling remain outside
    /// this boundary.
    pub fn ui_smoke() -> Self {
        Self {
            shutdown_started: AtomicBool::new(false),
            timeout: DEFAULT_TIMEOUT,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
            component: None,
            startup_error: None,
            smoke_mode: true,
            manager: Mutex::new(WorkerManager::default()),
        }
    }

    pub fn allowed_operations() -> &'static [&'static str] {
        ALLOWED_OPERATIONS
    }

    pub fn capabilities(&self) -> Value {
        json!({
            "protocol": PROTOCOL,
            "operations": Self::allowed_operations(),
            "offline_only": true,
            "public_requests": 0,
            "webview": false,
            "store_access": false,
            "download_queue_access": false,
            "credential_access": false,
            "task_id_owner": "application_service",
            "lifecycle_owner": "tauri",
            "production_remote_subset": false,
        })
    }

    /// Project only verified compatibility-component readiness. The Python
    /// worker does not own Registry product facts, Store state, credentials,
    /// or download scheduling; it only reports that its declared DEM bridge
    /// operations are available for the Rust capability projector.
    pub fn capability_runtime_observations(&self) -> CapabilityRuntimeObservations {
        let dem_bridge_ready = self.smoke_mode
            || self.component.as_ref().is_some_and(|component| {
                component.manifest.offline_only
                    && component
                        .manifest
                        .supported_operations
                        .iter()
                        .any(|operation| operation == "dem_inspect")
                    && component
                        .manifest
                        .supported_operations
                        .iter()
                        .any(|operation| operation == "dem_prepare_sarscape")
                    && component
                        .manifest
                        .supported_operations
                        .iter()
                        .any(|operation| operation == "dem_convert")
            });
        if !dem_bridge_ready {
            return CapabilityRuntimeObservations::default();
        }
        [
            "dem-provider.copernicus-aws.v1",
            "dem-provider.opentopography.v1",
            "dem-provider.user-local.v1",
        ]
        .into_iter()
        .fold(
            CapabilityRuntimeObservations::default(),
            |observations, binding| {
                observations
                    .with_build(binding, BuildCapabilityState::Available)
                    .with_runtime(binding, RuntimeReadinessState::Ready)
            },
        )
    }

    pub fn call(&self, operation: &str, payload: Value) -> Result<Value, crate::ShellError> {
        if self.shutdown_started.load(Ordering::Acquire) {
            return Err(worker_error("COMPAT_WORKER_SHUTDOWN"));
        }
        if !ALLOWED_OPERATIONS.contains(&operation) {
            return Err(worker_error("COMPAT_WORKER_OPERATION_UNSUPPORTED"));
        }
        if !payload.is_object() {
            return Err(worker_error("COMPAT_WORKER_INVALID_PAYLOAD"));
        }
        if self.smoke_mode {
            return Ok(ui_smoke_result(operation, payload));
        }
        if self.component.is_none() {
            return Err(worker_error(
                self.startup_error.unwrap_or("COMPAT_WORKER_MISSING"),
            ));
        }
        let mut manager = self
            .manager
            .lock()
            .map_err(|_| worker_error("COMPAT_WORKER_STATE_POISONED"))?;
        if self.shutdown_started.load(Ordering::Acquire)
            || manager.shutdown_state == ShutdownState::Shutdown
        {
            return Err(worker_error("COMPAT_WORKER_SHUTDOWN"));
        }
        if manager.restart_blocked {
            return Err(worker_error("COMPAT_WORKER_RESTART_BLOCKED"));
        }
        if manager.child.is_none() {
            if manager.restart_pending {
                if manager.restart_count >= 1 {
                    return Err(worker_error("COMPAT_WORKER_RESTART_LIMIT"));
                }
                manager.restart_count += 1;
                manager.restart_pending = false;
            }
            self.start_process(&mut manager)?;
        }
        let request_id = format!("tauri-{}", manager.next_request_id);
        manager.next_request_id = manager.next_request_id.saturating_add(1);
        self.send_request(&mut manager, request_id, operation, payload)
    }

    /// Return the live PID for offline lifecycle evidence. It is intentionally
    /// not exposed as a WebView command.
    pub fn process_id(&self) -> Option<u32> {
        self.manager
            .lock()
            .ok()
            .and_then(|manager| manager.child.as_ref().map(Child::id))
    }

    pub fn lifecycle_snapshot(&self) -> Value {
        self.manager
            .lock()
            .map(|manager| {
                json!({
                    "running": manager.child.is_some(),
                    "process_id": manager.child.as_ref().map(Child::id),
                    "process_generation": manager.process_generation,
                    "restart_count": manager.restart_count,
                    "pending_count": manager.pending.len(),
                    "shutdown": manager.shutdown_state == ShutdownState::Shutdown,
                    "verified_component": manager.verified_component_identity.as_ref().map(|identity| json!({
                        "component_id": identity.component_id,
                        "version": identity.version,
                        "sha256": identity.sha256,
                        "path": identity.path.file_name().and_then(|value| value.to_str()),
                    })),
                })
            })
            .unwrap_or_else(|_| json!({"state": "poisoned"}))
    }

    pub fn shutdown(&self) -> Result<(), crate::ShellError> {
        if self.shutdown_started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let mut manager = self
            .manager
            .lock()
            .map_err(|_| worker_error("COMPAT_WORKER_STATE_POISONED"))?;
        manager.shutdown_state = ShutdownState::Shutdown;
        manager.pending.clear();
        self.terminate_session(&mut manager, false);
        Ok(())
    }

    fn start_process(&self, manager: &mut WorkerManager) -> Result<(), crate::ShellError> {
        // Re-verify immediately before every process creation. The startup
        // preflight is useful for a clear missing-component state, but it must
        // not become a stale approval if the installed file changes later.
        let component = resolve_component().map_err(worker_error)?;
        let mut child = compat_worker_command(&component.path)
            .spawn()
            .map_err(|_| worker_error("COMPAT_WORKER_START_FAILED"))?;
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                reap_child(&mut child, self.shutdown_timeout);
                return Err(worker_error("COMPAT_WORKER_STDOUT_UNAVAILABLE"));
            }
        };
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                reap_child(&mut child, self.shutdown_timeout);
                return Err(worker_error("COMPAT_WORKER_STDIN_UNAVAILABLE"));
            }
        };
        let (sender, receiver) = mpsc::channel();
        let reader = thread::spawn(move || read_worker_frames(stdout, sender));
        manager.child = Some(child);
        manager.stdin = Some(stdin);
        manager.stdout_reader = Some(reader);
        manager.responses = Some(receiver);
        manager.process_generation = manager.process_generation.saturating_add(1);

        let request_id = format!("tauri-{}", manager.next_request_id);
        manager.next_request_id = manager.next_request_id.saturating_add(1);
        let capabilities =
            match self.send_request(manager, request_id, "worker_capabilities", json!({})) {
                Ok(value) => value,
                Err(error) => {
                    if error.code == "COMPAT_WORKER_RESPONSE_MISMATCH"
                        || error.code == "COMPAT_WORKER_INVALID_RESPONSE"
                        || error.code == "COMPAT_WORKER_PROTOCOL_MISMATCH"
                    {
                        self.terminate_session(manager, false);
                    }
                    return Err(error);
                }
            };
        if let Err(error) = validate_worker_capabilities(&capabilities, &component.manifest) {
            self.terminate_session(manager, false);
            return Err(error);
        }
        manager.verified_component_identity = Some(component.identity());
        Ok(())
    }

    fn send_request(
        &self,
        manager: &mut WorkerManager,
        request_id: String,
        operation: &str,
        payload: Value,
    ) -> Result<Value, crate::ShellError> {
        let request = json!({
            "protocol": PROTOCOL,
            "request_id": request_id,
            "operation": operation,
            "payload": payload,
        });
        let encoded = serde_json::to_vec(&request)
            .map_err(|_| worker_error("COMPAT_WORKER_SERIALIZE_FAILED"))?;
        let pending = PendingRequest {
            operation: operation.to_owned(),
            side_effect: SIDE_EFFECT_OPERATIONS.contains(&operation),
            generation: manager.process_generation,
        };
        manager.pending.insert(request_id.clone(), pending);
        let child_exited = match manager.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
            None => true,
        };
        if child_exited {
            let pending = manager.pending.remove(&request_id);
            self.terminate_session(manager, true);
            return Err(worker_error(failure_code(
                pending.as_ref(),
                "COMPAT_WORKER_CRASHED",
            )));
        }
        let write_result = (|| {
            let stdin = manager
                .stdin
                .as_mut()
                .ok_or("COMPAT_WORKER_STDIN_UNAVAILABLE")?;
            stdin
                .write_all(&encoded)
                .map_err(|_| "COMPAT_WORKER_WRITE_FAILED")?;
            stdin
                .write_all(b"\n")
                .map_err(|_| "COMPAT_WORKER_WRITE_FAILED")?;
            stdin.flush().map_err(|_| "COMPAT_WORKER_WRITE_FAILED")
        })();
        if let Err(code) = write_result {
            let pending = manager.pending.remove(&request_id);
            self.terminate_session(manager, true);
            return Err(worker_error(failure_code(pending.as_ref(), code)));
        }

        let started = Instant::now();
        loop {
            if self.shutdown_started.load(Ordering::Acquire) {
                let pending = manager.pending.remove(&request_id);
                self.terminate_session(manager, false);
                return Err(worker_error(failure_code(
                    pending.as_ref(),
                    "COMPAT_WORKER_SHUTDOWN",
                )));
            }
            let elapsed = started.elapsed();
            if elapsed >= self.timeout {
                let pending = manager.pending.remove(&request_id);
                // A timeout is terminal for this manager instance. It is not an
                // automatic restart trigger and the operation is never replayed.
                manager.restart_blocked = true;
                self.terminate_session(manager, false);
                return Err(worker_error(failure_code(
                    pending.as_ref(),
                    "COMPAT_WORKER_TIMEOUT",
                )));
            }
            let wait_for = (self.timeout - elapsed).min(FRAME_POLL_INTERVAL);
            let frame = match manager.responses.as_ref() {
                Some(receiver) => receiver.recv_timeout(wait_for),
                None => return Err(worker_error("COMPAT_WORKER_STDOUT_UNAVAILABLE")),
            };
            match frame {
                Ok(WorkerFrame::Response(response)) => {
                    let _pending = manager.pending.remove(&request_id);
                    match validate_response(&request_id, response) {
                        Ok(value) => return Ok(value),
                        Err(error) => {
                            if error.code == "COMPAT_WORKER_RESPONSE_MISMATCH"
                                || error.code == "COMPAT_WORKER_INVALID_RESPONSE"
                                || error.code == "COMPAT_WORKER_PROTOCOL_MISMATCH"
                            {
                                self.terminate_session(manager, true);
                            }
                            return Err(error);
                        }
                    }
                }
                Ok(WorkerFrame::ReadError(code)) => {
                    let pending = manager.pending.remove(&request_id);
                    self.terminate_session(manager, true);
                    return Err(worker_error(failure_code(pending.as_ref(), code)));
                }
                Ok(WorkerFrame::Eof) | Err(RecvTimeoutError::Disconnected) => {
                    let pending = manager.pending.remove(&request_id);
                    self.terminate_session(manager, true);
                    return Err(worker_error(failure_code(
                        pending.as_ref(),
                        "COMPAT_WORKER_CRASHED",
                    )));
                }
                Err(RecvTimeoutError::Timeout) => continue,
            }
        }
    }

    fn terminate_session(&self, manager: &mut WorkerManager, unexpected: bool) {
        manager.pending.clear();
        manager.stdin.take();
        manager.responses.take();
        if let Some(mut child) = manager.child.take() {
            reap_child(&mut child, self.shutdown_timeout);
        }
        if let Some(reader) = manager.stdout_reader.take() {
            let _ = reader.join();
        }
        manager.verified_component_identity = None;
        if unexpected && manager.shutdown_state == ShutdownState::Running {
            manager.restart_pending = true;
        }
    }
}

fn ui_smoke_result(operation: &str, payload: Value) -> Value {
    match operation {
        "dem_inspect" | "dem_convert" | "dem_prepare_sarscape" => {
            // Keep the smoke operation observable long enough for the real
            // scheduler control route to exercise pause/resume semantics.
            // The packaged loopback must expose a deterministic control
            // window so the real Scheduler can persist pause/resume commands
            // before this offline operation completes. This delay is confined
            // to the smoke Worker; production compatibility operations retain
            // their real bounded execution semantics.
            thread::sleep(Duration::from_millis(2_500));
            json!({
                "ok": true,
                "offline": true,
                "bytes_written": 0,
                "operation": operation,
                "input_path": payload.get("input_path").cloned().unwrap_or(Value::Null),
                "output_root": payload.get("output_root").cloned().unwrap_or(Value::Null),
            })
        }
        _ => json!({"ok": true, "offline": true}),
    }
}

impl Drop for CompatWorkerState {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl Default for CompatWorkerState {
    fn default() -> Self {
        Self::from_environment()
    }
}

fn worker_error(code: &str) -> crate::ShellError {
    crate::ShellError::with_code(code, "compat_worker.operation_failed")
}

fn failure_code(pending: Option<&PendingRequest>, code: &str) -> &'static str {
    if pending.is_some_and(|item| {
        let _ = (&item.operation, item.generation);
        item.side_effect
    }) {
        "COMPAT_WORKER_SIDE_EFFECT_INDETERMINATE"
    } else {
        match code {
            "COMPAT_WORKER_SHUTDOWN" => "COMPAT_WORKER_SHUTDOWN",
            "COMPAT_WORKER_TIMEOUT" => "COMPAT_WORKER_TIMEOUT",
            "COMPAT_WORKER_PROTOCOL_MISMATCH" => "COMPAT_WORKER_PROTOCOL_MISMATCH",
            "COMPAT_WORKER_RESPONSE_MISMATCH" => "COMPAT_WORKER_RESPONSE_MISMATCH",
            "COMPAT_WORKER_INVALID_RESPONSE" => "COMPAT_WORKER_INVALID_RESPONSE",
            "COMPAT_WORKER_WRITE_FAILED" => "COMPAT_WORKER_WRITE_FAILED",
            "COMPAT_WORKER_READ_FAILED" => "COMPAT_WORKER_READ_FAILED",
            "COMPAT_WORKER_STDOUT_UNAVAILABLE" => "COMPAT_WORKER_STDOUT_UNAVAILABLE",
            "COMPAT_WORKER_CRASHED" => "COMPAT_WORKER_CRASHED",
            _ => "COMPAT_WORKER_OPERATION_FAILED",
        }
    }
}

fn read_worker_frames(stdout: std::process::ChildStdout, sender: Sender<WorkerFrame>) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                let _ = sender.send(WorkerFrame::Eof);
                return;
            }
            Ok(_) => match serde_json::from_str::<Value>(&line) {
                Ok(response) => {
                    if sender.send(WorkerFrame::Response(response)).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = sender.send(WorkerFrame::ReadError("COMPAT_WORKER_INVALID_RESPONSE"));
                    return;
                }
            },
            Err(_) => {
                let _ = sender.send(WorkerFrame::ReadError("COMPAT_WORKER_READ_FAILED"));
                return;
            }
        }
    }
}

fn validate_response(request_id: &str, response: Value) -> Result<Value, crate::ShellError> {
    let object = response
        .as_object()
        .ok_or_else(|| worker_error("COMPAT_WORKER_INVALID_RESPONSE"))?;
    if object.get("protocol").and_then(Value::as_str) != Some(PROTOCOL) {
        return Err(worker_error("COMPAT_WORKER_PROTOCOL_MISMATCH"));
    }
    if object.get("request_id").and_then(Value::as_str) != Some(request_id) {
        return Err(worker_error("COMPAT_WORKER_RESPONSE_MISMATCH"));
    }
    if object.get("ok").and_then(Value::as_bool) == Some(true) {
        return object
            .get("result")
            .cloned()
            .ok_or_else(|| worker_error("COMPAT_WORKER_INVALID_RESPONSE"));
    }
    let code = object
        .get("error")
        .and_then(Value::as_object)
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("COMPAT_WORKER_OPERATION_FAILED");
    Err(worker_error(code))
}

fn validate_worker_capabilities(
    capabilities: &Value,
    manifest: &ComponentManifest,
) -> Result<(), crate::ShellError> {
    if capabilities.get("protocol").and_then(Value::as_str) != Some(PROTOCOL) {
        return Err(worker_error("COMPAT_WORKER_PROTOCOL_MISMATCH"));
    }
    let expected_operations: Vec<Value> = manifest
        .supported_operations
        .iter()
        .map(|operation| Value::String(operation.clone()))
        .collect();
    if capabilities.get("operations") != Some(&Value::Array(expected_operations))
        || capabilities.get("offline_only") != Some(&Value::Bool(true))
        || capabilities.get("public_requests") != Some(&Value::from(0))
        || capabilities.get("webview") != Some(&Value::Bool(false))
        || capabilities.get("store_access") != Some(&Value::Bool(false))
        || capabilities.get("credential_access") != Some(&Value::Bool(false))
        || capabilities.get("download_queue_access") != Some(&Value::Bool(false))
        || capabilities.get("lifecycle_owner") != Some(&Value::String("tauri".to_owned()))
        || capabilities.get("task_id_owner")
            != Some(&Value::String("application_service".to_owned()))
        || capabilities.get("production_remote_subset") != Some(&Value::Bool(false))
    {
        return Err(worker_error("COMPAT_WORKER_CAPABILITY_MISMATCH"));
    }
    Ok(())
}

fn parse_component_contract() -> Result<ComponentManifest, &'static str> {
    serde_json::from_str(COMPONENT_CONTRACT_JSON).map_err(|_| "COMPAT_WORKER_CAPABILITY_MISMATCH")
}

fn resolve_component() -> Result<VerifiedComponent, &'static str> {
    let expected = parse_component_contract()?;
    let executable = std::env::current_exe().map_err(|_| "COMPAT_WORKER_MISSING")?;
    let exe_dir = executable
        .parent()
        .ok_or("COMPAT_WORKER_MISSING")?
        .to_path_buf();
    let macos_resources_dir = exe_dir
        .parent()
        .map(|parent| parent.join("Resources"))
        .filter(|res| {
            res.join("runtime")
                .join("compat-worker")
                .join("component-manifest.json")
                .is_file()
        });
    let install_root = if exe_dir
        .join("runtime")
        .join("compat-worker")
        .join("component-manifest.json")
        .is_file()
    {
        exe_dir
    } else if let Some(res_dir) = macos_resources_dir {
        res_dir
    } else if cfg!(debug_assertions) {
        crate::application::discover_runtime_root()
    } else {
        exe_dir
    };
    let test_override = if cfg!(debug_assertions) {
        std::env::var_os(TEST_WORKER_OVERRIDE_ENV).map(PathBuf::from)
    } else {
        None
    };
    let manifest_path = if test_override.is_some() {
        std::env::var_os(TEST_COMPONENT_MANIFEST_ENV)
            .map(PathBuf::from)
            .or_else(|| {
                test_override.as_ref().and_then(|path| {
                    path.parent()
                        .map(|parent| parent.join("component-manifest.json"))
                })
            })
            .ok_or("COMPAT_WORKER_MISSING")?
    } else {
        install_root
            .join("runtime")
            .join("compat-worker")
            .join("component-manifest.json")
    };
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|_| "COMPAT_WORKER_MISSING")?;
    let manifest: ComponentManifest =
        serde_json::from_str(&manifest_text).map_err(|_| "COMPAT_WORKER_CAPABILITY_MISMATCH")?;
    validate_manifest_metadata(&expected, &manifest)?;

    let worker_path = if let Some(path) = test_override {
        path
    } else {
        let relative = Path::new(&manifest.relative_path);
        if relative.is_absolute() {
            return Err("COMPAT_WORKER_INTEGRITY_FAILED");
        }
        install_root.join(relative)
    };
    if worker_path.file_name().and_then(|value| value.to_str())
        != Some(manifest.executable_name.as_str())
    {
        return Err("COMPAT_WORKER_CAPABILITY_MISMATCH");
    }
    let canonical_root = install_root
        .canonicalize()
        .map_err(|_| "COMPAT_WORKER_MISSING")?;
    let canonical_worker = worker_path
        .canonicalize()
        .map_err(|_| "COMPAT_WORKER_MISSING")?;
    if !test_override_path_allowed(&canonical_root, &canonical_worker, test_override_path()) {
        return Err("COMPAT_WORKER_INTEGRITY_FAILED");
    }
    if !canonical_worker.is_file() {
        return Err("COMPAT_WORKER_MISSING");
    }
    let actual_sha256 = sha256_file(&canonical_worker)?;
    verify_sha256(&manifest.sha256, &actual_sha256)?;
    Ok(VerifiedComponent {
        path: canonical_worker,
        manifest,
    })
}

fn test_override_path() -> Option<PathBuf> {
    std::env::var_os(TEST_WORKER_OVERRIDE_ENV).map(PathBuf::from)
}

fn test_override_path_allowed(root: &Path, worker: &Path, override_path: Option<PathBuf>) -> bool {
    override_path.is_some() || worker.strip_prefix(root).is_ok()
}

fn validate_manifest_metadata(
    expected: &ComponentManifest,
    actual: &ComponentManifest,
) -> Result<(), &'static str> {
    if actual.protocol != expected.protocol {
        return Err("COMPAT_WORKER_PROTOCOL_MISMATCH");
    }
    if actual.component_id != expected.component_id
        || actual.relative_path != expected.relative_path
        || actual.executable_name != expected.executable_name
        || actual.version != expected.version
        || actual.supported_operations != expected.supported_operations
        || actual.offline_only != expected.offline_only
        || actual.store_access != expected.store_access
        || actual.credential_access != expected.credential_access
        || actual.download_queue_access != expected.download_queue_access
        || actual.lifecycle_owner != expected.lifecycle_owner
        || actual.task_id_owner != expected.task_id_owner
    {
        return Err("COMPAT_WORKER_CAPABILITY_MISMATCH");
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn verify_sha256(expected: &str, actual: &str) -> Result<(), &'static str> {
    if !is_sha256(expected) || !expected.eq_ignore_ascii_case(actual) {
        return Err("COMPAT_WORKER_INTEGRITY_FAILED");
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, &'static str> {
    let mut file = File::open(path).map_err(|_| "COMPAT_WORKER_MISSING")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "COMPAT_WORKER_INTEGRITY_FAILED")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Build the Compatibility Worker `Command` with the reviewed spawn boundary:
/// the child must never inherit hostile HDF5 startup variables from the host.
/// The host sanitises its own CRT environment at startup; this boundary also
/// explicitly removes the four variables so a hostile value present in the
/// parent process can never leak into the Worker. No WorkerManager or
/// protocol state is involved.
fn compat_worker_command(component_path: &std::path::Path) -> Command {
    let mut command = Command::new(component_path);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_remove("HDF5_PLUGIN_PATH")
        .env_remove("HDF5_VOL_CONNECTOR")
        .env_remove("HDF5_DRIVER")
        .env_remove("HDF5_DRIVER_CONFIG");
    command
}

fn reap_child(child: &mut Child, timeout: Duration) {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if started.elapsed() < timeout => thread::sleep(FRAME_POLL_INTERVAL),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{
        compat_worker_command, is_sha256, validate_manifest_metadata, validate_worker_capabilities,
        verify_sha256, CapabilityRuntimeObservations, CompatWorkerState, ComponentManifest,
        VerifiedComponent, ALLOWED_OPERATIONS,
    };

    fn manifest() -> ComponentManifest {
        serde_json::from_str(include_str!(
            "../../contracts/compat-worker-component.v1.json"
        ))
        .expect("component contract parses")
    }

    #[test]
    fn worker_allowlist_is_explicit_and_offline() {
        assert_eq!(CompatWorkerState::allowed_operations(), ALLOWED_OPERATIONS);
        let capabilities = CompatWorkerState::default().capabilities();
        assert_eq!(capabilities["public_requests"], 0);
        assert_eq!(capabilities["store_access"], false);
        assert_eq!(capabilities["download_queue_access"], false);
        assert_eq!(capabilities["credential_access"], false);
    }

    #[test]
    fn ui_smoke_worker_returns_only_the_heavy_dem_engine_shape() {
        let worker = CompatWorkerState::ui_smoke();
        assert_ne!(
            worker.capability_runtime_observations(),
            CapabilityRuntimeObservations::default()
        );
        let dem = worker
            .call(
                "dem_prepare_sarscape",
                serde_json::json!({"input_path": "source.tif", "output_root": "out"}),
            )
            .unwrap();
        assert_eq!(dem["ok"], true);
        assert_eq!(dem["offline"], true);
        let error = worker
            .call("gacos_plan", serde_json::json!({"scenes": []}))
            .expect_err("GACOS is an ApplicationService route");
        assert_eq!(error.code, "COMPAT_WORKER_OPERATION_UNSUPPORTED");
    }

    #[test]
    fn ui_smoke_dem_operation_keeps_a_real_scheduler_control_window() {
        let worker = CompatWorkerState::ui_smoke();
        let started = Instant::now();
        let result = worker
            .call(
                "dem_prepare_sarscape",
                serde_json::json!({"input_path": "source.tif", "output_root": "out"}),
            )
            .expect("offline DEM smoke operation");
        assert_eq!(result["ok"], true);
        assert!(
            started.elapsed() >= Duration::from_millis(500),
            "the formal DEM smoke task must remain controllable long enough for pause/resume"
        );
    }

    #[test]
    fn unknown_operation_fails_before_process_start() {
        let state = CompatWorkerState::default();
        let error = state
            .call("not_reviewed", serde_json::json!({}))
            .unwrap_err();
        assert_eq!(error.code, "COMPAT_WORKER_OPERATION_UNSUPPORTED");
    }

    #[test]
    fn capability_validation_rejects_protocol_and_capability_drift() {
        let contract = manifest();
        let mut capabilities = serde_json::json!({
            "protocol": "eostage.compat-worker.v1",
            "operations": contract.supported_operations,
            "offline_only": true,
            "public_requests": 0,
            "webview": false,
            "store_access": false,
            "credential_access": false,
            "download_queue_access": false,
            "lifecycle_owner": "tauri",
            "task_id_owner": "application_service",
            "production_remote_subset": false
        });
        assert!(validate_worker_capabilities(&capabilities, &contract).is_ok());
        capabilities["protocol"] = serde_json::json!("other.protocol");
        assert_eq!(
            validate_worker_capabilities(&capabilities, &contract)
                .unwrap_err()
                .code,
            "COMPAT_WORKER_PROTOCOL_MISMATCH"
        );
        capabilities["protocol"] = serde_json::json!("eostage.compat-worker.v1");
        capabilities["store_access"] = serde_json::json!(true);
        assert_eq!(
            validate_worker_capabilities(&capabilities, &contract)
                .unwrap_err()
                .code,
            "COMPAT_WORKER_CAPABILITY_MISMATCH"
        );
    }

    #[test]
    fn sha256_shape_is_strict() {
        assert!(is_sha256(&"a".repeat(64)));
        assert!(!is_sha256(&"a".repeat(63)));
        assert!(!is_sha256(&format!("{}g", "a".repeat(63))));
        assert_eq!(
            verify_sha256(&"a".repeat(64), &"b".repeat(64)).unwrap_err(),
            "COMPAT_WORKER_INTEGRITY_FAILED"
        );
    }

    #[test]
    fn manifest_protocol_and_capability_drift_have_distinct_errors() {
        let expected = manifest();
        let mut actual = manifest();
        actual.protocol = "other.protocol".to_owned();
        assert_eq!(
            validate_manifest_metadata(&expected, &actual).unwrap_err(),
            "COMPAT_WORKER_PROTOCOL_MISMATCH"
        );
        actual.protocol = expected.protocol.clone();
        actual.store_access = true;
        assert_eq!(
            validate_manifest_metadata(&expected, &actual).unwrap_err(),
            "COMPAT_WORKER_CAPABILITY_MISMATCH"
        );
    }

    #[test]
    fn verified_dem_bridge_projects_runtime_without_copying_product_facts() {
        let mut state = CompatWorkerState::default();
        state.component = Some(VerifiedComponent {
            path: PathBuf::from("runtime/compat-worker/test.exe"),
            manifest: manifest(),
        });
        let observations = state.capability_runtime_observations();
        let service = insar_application_service::ApplicationService::new(
            insar_application_service::ApplicationServiceConfig::liveness_only()
                .with_capability_runtime_observations(observations),
        );
        let status = service
            .effective_capability("capability.dem.cop30.aws-download.v1")
            .unwrap()
            .expect("Registry owns the Copernicus DEM declaration");
        assert_eq!(
            status.inputs.build,
            Some(insar_core::BuildCapabilityState::Available)
        );
        assert_eq!(
            status.inputs.runtime,
            Some(insar_core::RuntimeReadinessState::Ready)
        );
        assert_eq!(
            status.projection.state,
            insar_core::EffectiveCapabilityState::Executable
        );
    }

    fn require_configured_runtime_state() -> CompatWorkerState {
        assert!(
            std::env::var_os("EOSTAGE_COMPAT_WORKER_TEST_OVERRIDE").is_some(),
            "EOSTAGE_COMPAT_WORKER_TEST_OVERRIDE must be set to run ignored compat-worker tests"
        );
        CompatWorkerState::from_environment()
    }

    fn kill_managed_child(state: &CompatWorkerState) {
        let mut manager = state.manager.lock().expect("Worker manager lock");
        if let Some(child) = manager.child.as_mut() {
            let _ = child.kill();
        }
        drop(manager);
        thread::sleep(Duration::from_millis(100));
    }

    #[test]
    #[ignore = "requires optional compat-worker executable (EOSTAGE_COMPAT_WORKER_TEST_OVERRIDE)"]
    fn persistent_worker_serial_queue_keeps_concurrent_utf8_responses_paired() {
        let state = require_configured_runtime_state();
        let state = Arc::new(state);
        let mut handles = Vec::new();
        for index in 0..8 {
            let state = Arc::clone(&state);
            handles.push(thread::spawn(move || {
                let file_name = format!("区{index}.geojson");
                let payload = serde_json::json!({
                    "source": "text",
                    "file_name": file_name,
                    "text": "{\"type\":\"FeatureCollection\",\"features\":[{\"type\":\"Feature\",\"properties\":{\"name\":\"测试区\"},\"geometry\":{\"type\":\"Polygon\",\"coordinates\":[[[100,30],[101,30],[101,31],[100,31],[100,30]]]}}]}",
                });
                let result = state
                    .call("aoi_import", payload)
                    .expect("concurrent Worker call succeeds");
                assert_eq!(result["file_name"], file_name);
            }));
        }
        for handle in handles {
            handle.join().expect("concurrent Worker thread joins");
        }
        let lifecycle = state.lifecycle_snapshot();
        assert_eq!(lifecycle["process_generation"], 1);
        assert_eq!(lifecycle["restart_count"], 0);
        assert_eq!(lifecycle["pending_count"], 0);
        state.shutdown().expect("Worker shuts down");
        assert_eq!(state.process_id(), None);
    }

    #[test]
    #[ignore = "requires optional compat-worker executable (EOSTAGE_COMPAT_WORKER_TEST_OVERRIDE)"]
    fn unexpected_exit_allows_only_one_next_request_restart() {
        let state = require_configured_runtime_state();
        state
            .call("worker_capabilities", serde_json::json!({}))
            .expect("initial Worker call succeeds");
        kill_managed_child(&state);
        assert_eq!(
            state
                .call("worker_capabilities", serde_json::json!({}))
                .unwrap_err()
                .code,
            "COMPAT_WORKER_CRASHED"
        );
        state
            .call("worker_capabilities", serde_json::json!({}))
            .expect("the next new request performs one controlled restart");
        assert_eq!(state.lifecycle_snapshot()["process_generation"], 2);
        assert_eq!(state.lifecycle_snapshot()["restart_count"], 1);
        kill_managed_child(&state);
        assert_eq!(
            state
                .call("worker_capabilities", serde_json::json!({}))
                .unwrap_err()
                .code,
            "COMPAT_WORKER_CRASHED"
        );
        assert_eq!(
            state
                .call("worker_capabilities", serde_json::json!({}))
                .unwrap_err()
                .code,
            "COMPAT_WORKER_RESTART_LIMIT"
        );
        state
            .shutdown()
            .expect("Worker shuts down after restart test");
    }

    #[test]
    #[ignore = "requires optional compat-worker executable (EOSTAGE_COMPAT_WORKER_TEST_OVERRIDE)"]
    fn side_effect_failure_is_indeterminate_and_is_never_replayed() {
        let state = require_configured_runtime_state();
        state
            .call("worker_capabilities", serde_json::json!({}))
            .expect("initial Worker call succeeds");
        kill_managed_child(&state);
        assert_eq!(
            state
                .call("dem_prepare_sarscape", serde_json::json!({}))
                .unwrap_err()
                .code,
            "COMPAT_WORKER_SIDE_EFFECT_INDETERMINATE"
        );
        assert_eq!(state.lifecycle_snapshot()["process_generation"], 1);
        state
            .call("worker_capabilities", serde_json::json!({}))
            .expect("the next request may restart after the crash, without replaying the write");
        assert_eq!(state.lifecycle_snapshot()["process_generation"], 2);
        assert_eq!(state.lifecycle_snapshot()["restart_count"], 1);
        state
            .shutdown()
            .expect("Worker shuts down after side-effect test");
    }
    /// Regression: the Worker spawn boundary must never inherit hostile HDF5
    /// startup variables. This uses the same `compat_worker_command` builder
    /// the runtime uses, pointed at a probe that prints its inherited
    /// environment, and asserts the four hostile variables are absent in the
    /// child while present in the parent. Windows uses cmd `set`; Unix uses
    /// `/bin/sh -c env`; the probe output is captured via the piped stdout the
    /// builder already sets.
    #[test]
    fn worker_command_does_not_inherit_hostile_hdf5_variables() {
        use std::io::Read;
        use std::process::Command;
        use std::sync::atomic::{AtomicU64, Ordering};

        const SUBPROCESS_MARKER: &str = "INSAR_COMPAT_WORKER_ENV_SUBPROCESS_PROBE";
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let hostile = format!("insar-hostile-path-{}-{}", std::process::id(), id);
        let hostile_vars = [
            ("HDF5_PLUGIN_PATH", hostile.as_str()),
            ("HDF5_VOL_CONNECTOR", "nonexistent_external_connector"),
            ("HDF5_DRIVER", "nonexistent_external_driver"),
            ("HDF5_DRIVER_CONFIG", "hostile"),
        ];

        // Never call `std::env::set_var` / `remove_var` inside the multi-threaded
        // unit-test runner: native C libraries (`libgdal`, `libhdf5`, `CoreFoundation`)
        // call `getenv()` on sibling test threads, which is UB / SIGSEGV on POSIX if
        // `setenv`/`unsetenv` reallocates `environ`. Re-exec this exact test in a
        // dedicated single-threaded subprocess whose initial `envp` carries the
        // hostile HDF5 variables from process start.
        if std::env::var_os(SUBPROCESS_MARKER).is_none() {
            let output = Command::new(std::env::current_exe().expect("current test binary"))
                .env(SUBPROCESS_MARKER, "1")
                .envs(hostile_vars)
                .args([
                    "--exact",
                    "compat_worker::tests::worker_command_does_not_inherit_hostile_hdf5_variables",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .output()
                .expect("isolated env probe subprocess spawns");
            assert!(
                output.status.success(),
                "isolated env probe subprocess failed ({}): stdout={}, stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        for (name, _) in hostile_vars {
            assert!(
                std::env::var_os(name).is_some_and(|v| !v.is_empty()),
                "subprocess must start with hostile {name} populated in parent environment"
            );
        }

        #[cfg(windows)]
        let mut command = {
            let mut command = compat_worker_command(std::path::Path::new("cmd.exe"));
            command.args(["/c", "set"]);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = compat_worker_command(std::path::Path::new("/bin/sh"));
            command.args(["-c", "env"]);
            command
        };

        let mut child = command.spawn().expect("env probe spawns");
        let mut stdout = Vec::new();
        child
            .stdout
            .take()
            .expect("env probe stdout")
            .read_to_end(&mut stdout)
            .expect("env probe stdout readable");
        let status = child.wait().expect("env probe exits");
        assert!(status.success(), "env probe failed: {status}");

        let lower = String::from_utf8_lossy(&stdout).to_ascii_lowercase();
        for (name, _) in hostile_vars {
            let needle = format!("{name}=").to_ascii_lowercase();
            assert!(
                !lower.contains(&needle),
                "child inherited hostile {name}: {}",
                String::from_utf8_lossy(&stdout)
            );
        }
    }
}
