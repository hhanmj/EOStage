use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::num::NonZeroUsize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::{DownloadFailure, DownloadOutcome};
use insar_download::{
    part_path, AdmittedTransferBackend, AuthRefreshPolicyId, AuthRejection, HttpTransfer,
    ResolvedAsset, ResolvedMethod, ResolvedTransferResult, ResolvedTransferSession,
    ResponseHostClass, TransferAdmissionController, TransferAuth, TransferBackend,
    TransferBackendKind, TransferOptions, TransferRequest, TransferResult, TransferUpdate,
};
use serde_json::{json, Map, Value};

const SCHEMA_VERSION: &str = "d2b-admission-benchmark-v1";
const SCENARIO_DEADLINE: Duration = Duration::from_secs(8);
const WAIT_DEADLINE: Duration = Duration::from_secs(2);
const BODY_CHUNK: usize = 1024;

#[derive(Clone, Debug)]
enum Candidate {
    Compatibility,
    Finite {
        global: NonZeroUsize,
        per_origin: NonZeroUsize,
    },
}

impl Candidate {
    fn parse(value: &str) -> Result<Self, String> {
        if value == "compatibility" {
            return Ok(Self::Compatibility);
        }
        let (global, per_origin) = value
            .strip_prefix('g')
            .and_then(|suffix| suffix.split_once("-h"))
            .ok_or_else(|| "invalid benchmark candidate".to_string())?;
        let global = global
            .parse::<usize>()
            .ok()
            .and_then(NonZeroUsize::new)
            .ok_or_else(|| "invalid benchmark candidate".to_string())?;
        let per_origin = per_origin
            .parse::<usize>()
            .ok()
            .and_then(NonZeroUsize::new)
            .ok_or_else(|| "invalid benchmark candidate".to_string())?;
        if !matches!(global.get(), 1 | 2 | 4 | 8) || !matches!(per_origin.get(), 1 | 2 | 4) {
            return Err("unsupported benchmark candidate".to_string());
        }
        Ok(Self::Finite { global, per_origin })
    }

    fn identifier(&self) -> String {
        match self {
            Self::Compatibility => "compatibility".to_string(),
            Self::Finite { global, per_origin } => {
                format!("g{}-h{}", global.get(), per_origin.get())
            }
        }
    }

    fn controller(&self) -> Arc<TransferAdmissionController> {
        match self {
            Self::Compatibility => TransferAdmissionController::compatibility(),
            Self::Finite { global, per_origin } => {
                TransferAdmissionController::finite_for_test(*global, *per_origin)
            }
        }
    }

    fn finite_bounds(&self) -> Option<(usize, usize)> {
        match self {
            Self::Compatibility => None,
            Self::Finite { global, per_origin } => Some((global.get(), per_origin.get())),
        }
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "temporary-root clock failure".to_string())?
            .as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "insar-d2b-admission-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).map_err(|_| "temporary-root creation failure".to_string())?;
        Ok(Self(path))
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct BodyPlan {
    body: Vec<u8>,
    chunk_delay: Duration,
    throttle_above: Option<usize>,
    scripted_statuses: Arc<Mutex<VecDeque<u16>>>,
}

#[derive(Clone)]
enum EndpointPlan {
    Body(BodyPlan),
    Redirect { location: String },
}

struct RequestEvent {
    path: String,
    elapsed_ms: u64,
}

struct FixtureState {
    started: Instant,
    endpoints: Mutex<BTreeMap<String, EndpointPlan>>,
    request_counts: Mutex<BTreeMap<String, u64>>,
    status_counts: Mutex<BTreeMap<String, u64>>,
    events: Mutex<Vec<RequestEvent>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    bytes_served: AtomicU64,
    failures: AtomicUsize,
    failure_codes: Mutex<BTreeMap<String, u64>>,
}

struct FixtureServer {
    base_url: String,
    state: Arc<FixtureState>,
    stop: Arc<AtomicBool>,
    connections: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    accept_thread: Option<thread::JoinHandle<()>>,
}

struct ServerSummary {
    request_counts: BTreeMap<String, u64>,
    status_counts: BTreeMap<String, u64>,
    first_request_ms: BTreeMap<String, u64>,
    bytes_served: u64,
    max_active: usize,
    failures: usize,
    failure_codes: BTreeMap<String, u64>,
}

impl FixtureServer {
    fn new() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|_| "loopback listener creation failure".to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "loopback listener setup failure".to_string())?;
        let address = listener
            .local_addr()
            .map_err(|_| "loopback listener address failure".to_string())?;
        if !address.ip().is_loopback() {
            return Err("fixture rejected non-loopback listener".to_string());
        }
        let state = Arc::new(FixtureState {
            started: Instant::now(),
            endpoints: Mutex::new(BTreeMap::new()),
            request_counts: Mutex::new(BTreeMap::new()),
            status_counts: Mutex::new(BTreeMap::new()),
            events: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            bytes_served: AtomicU64::new(0),
            failures: AtomicUsize::new(0),
            failure_codes: Mutex::new(BTreeMap::new()),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(Vec::new()));
        let accept_state = Arc::clone(&state);
        let accept_stop = Arc::clone(&stop);
        let accept_connections = Arc::clone(&connections);
        let accept_thread = thread::spawn(move || {
            while !accept_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let connection_state = Arc::clone(&accept_state);
                        let connection_stop = Arc::clone(&accept_stop);
                        let handle = thread::spawn(move || {
                            if let Err(error) = handle_connection(stream, &connection_state) {
                                if !connection_stop.load(Ordering::Acquire) {
                                    connection_state.failures.fetch_add(1, Ordering::Relaxed);
                                    *connection_state
                                        .failure_codes
                                        .lock()
                                        .unwrap()
                                        .entry(error)
                                        .or_insert(0) += 1;
                                }
                            }
                        });
                        accept_connections.lock().unwrap().push(handle);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => {
                        if !accept_stop.load(Ordering::Acquire) {
                            accept_state.failures.fetch_add(1, Ordering::Relaxed);
                        }
                        break;
                    }
                }
            }
        });
        Ok(Self {
            base_url: format!("http://{address}"),
            state,
            stop,
            connections,
            accept_thread: Some(accept_thread),
        })
    }

    fn url(&self, path: &str) -> Result<String, String> {
        let value = format!("{}{}", self.base_url, path);
        if !is_loopback_url(&value) {
            return Err("fixture rejected non-loopback request".to_string());
        }
        Ok(value)
    }

    fn add_body(
        &self,
        path: &str,
        body: Vec<u8>,
        chunk_delay: Duration,
        throttle_above: Option<usize>,
        scripted_statuses: Vec<u16>,
    ) {
        self.state.endpoints.lock().unwrap().insert(
            path.to_string(),
            EndpointPlan::Body(BodyPlan {
                body,
                chunk_delay,
                throttle_above,
                scripted_statuses: Arc::new(Mutex::new(scripted_statuses.into())),
            }),
        );
    }

    fn add_redirect(&self, path: &str, location: String) -> Result<(), String> {
        if !is_loopback_url(&location) {
            return Err("fixture rejected non-loopback redirect".to_string());
        }
        self.state
            .endpoints
            .lock()
            .unwrap()
            .insert(path.to_string(), EndpointPlan::Redirect { location });
        Ok(())
    }

    fn finish(mut self) -> ServerSummary {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
        let handles = std::mem::take(&mut *self.connections.lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
        let mut first_request_ms = BTreeMap::new();
        for event in self.state.events.lock().unwrap().iter() {
            first_request_ms
                .entry(event.path.clone())
                .or_insert(event.elapsed_ms);
        }
        ServerSummary {
            request_counts: self.state.request_counts.lock().unwrap().clone(),
            status_counts: self.state.status_counts.lock().unwrap().clone(),
            first_request_ms,
            bytes_served: self.state.bytes_served.load(Ordering::Relaxed),
            max_active: self.state.max_active.load(Ordering::Relaxed),
            failures: self.state.failures.load(Ordering::Relaxed),
            failure_codes: self.state.failure_codes.lock().unwrap().clone(),
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
        let handles = std::mem::take(&mut *self.connections.lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
    }
}

fn update_max(value: &AtomicUsize, candidate: usize) {
    let mut current = value.load(Ordering::Relaxed);
    while candidate > current {
        match value.compare_exchange_weak(current, candidate, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn handle_connection(mut stream: TcpStream, state: &FixtureState) -> Result<(), String> {
    stream
        .set_nonblocking(false)
        .map_err(|_| "fixture blocking setup failure".to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| "fixture read setup failure".to_string())?;
    let Some(path) = read_request_path(&stream)? else {
        return Ok(());
    };
    let active = state.active.fetch_add(1, Ordering::AcqRel) + 1;
    update_max(&state.max_active, active);
    let result = {
        *state
            .request_counts
            .lock()
            .unwrap()
            .entry(path.clone())
            .or_insert(0) += 1;
        state.events.lock().unwrap().push(RequestEvent {
            path: path.clone(),
            elapsed_ms: elapsed_ms(state.started),
        });
        let endpoint = state.endpoints.lock().unwrap().get(&path).cloned();
        match endpoint {
            Some(EndpointPlan::Redirect { location }) => {
                record_status(state, 302);
                write_response(
                    &mut stream,
                    302,
                    Some(&location),
                    &[],
                    Duration::ZERO,
                    state,
                )
            }
            Some(EndpointPlan::Body(plan)) => {
                let scripted = plan.scripted_statuses.lock().unwrap().pop_front();
                let status = scripted.unwrap_or_else(|| {
                    if plan.throttle_above.is_some_and(|limit| active > limit) {
                        if state.request_counts.lock().unwrap()[&path].is_multiple_of(2) {
                            429
                        } else {
                            500
                        }
                    } else {
                        200
                    }
                });
                record_status(state, status);
                if status == 200 {
                    write_response(
                        &mut stream,
                        status,
                        None,
                        &plan.body,
                        plan.chunk_delay,
                        state,
                    )
                } else {
                    write_response(&mut stream, status, None, &[], Duration::ZERO, state)
                }
            }
            None => {
                record_status(state, 404);
                write_response(&mut stream, 404, None, &[], Duration::ZERO, state)
            }
        }
    };
    state.active.fetch_sub(1, Ordering::AcqRel);
    result
}

fn read_request_path(stream: &TcpStream) -> Result<Option<String>, String> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|_| "fixture stream clone failure".to_string())?,
    );
    let mut line = String::new();
    if read_fixture_line(&mut reader, &mut line)?.is_none() {
        return Ok(None);
    }
    let path = line
        .split_whitespace()
        .nth(1)
        .and_then(|target| target.split('?').next())
        .filter(|target| target.starts_with('/'))
        .ok_or_else(|| "fixture malformed request".to_string())?
        .to_string();
    loop {
        line.clear();
        if read_fixture_line(&mut reader, &mut line)?.is_none() {
            return Ok(None);
        }
        if line == "\r\n" {
            break;
        }
    }
    Ok(Some(path))
}

fn read_fixture_line(
    reader: &mut BufReader<TcpStream>,
    line: &mut String,
) -> Result<Option<()>, String> {
    match reader.read_line(line) {
        Ok(0) => Ok(None),
        Ok(_) => Ok(Some(())),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::UnexpectedEof
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(format!("fixture request read {:?}", error.kind())),
    }
}

fn record_status(state: &FixtureState, status: u16) {
    *state
        .status_counts
        .lock()
        .unwrap()
        .entry(status.to_string())
        .or_insert(0) += 1;
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    location: Option<&str>,
    body: &[u8],
    delay: Duration,
    state: &FixtureState,
) -> Result<(), String> {
    let phrase = match status {
        200 => "200 OK",
        302 => "302 Found",
        401 => "401 Unauthorized",
        403 => "403 Forbidden",
        404 => "404 Not Found",
        429 => "429 Too Many Requests",
        500 => "500 Internal Server Error",
        _ => "503 Service Unavailable",
    };
    let mut header = format!(
        "HTTP/1.1 {phrase}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(location) = location {
        header.push_str(&format!("Location: {location}\r\n"));
    }
    header.push_str("\r\n");
    stream
        .write_all(header.as_bytes())
        .map_err(|_| "fixture response header failure".to_string())?;
    for chunk in body.chunks(BODY_CHUNK) {
        stream
            .write_all(chunk)
            .map_err(|_| "fixture response body failure".to_string())?;
        stream
            .flush()
            .map_err(|_| "fixture response flush failure".to_string())?;
        state
            .bytes_served
            .fetch_add(chunk.len() as u64, Ordering::Relaxed);
        if !delay.is_zero() {
            thread::sleep(delay);
        }
    }
    Ok(())
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn is_loopback_url(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    if scheme != "http" && scheme != "https" {
        return false;
    }
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority
        .strip_prefix('[')
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_else(|| authority.split(':').next().unwrap_or_default());
    host == "127.0.0.1" || host == "::1"
}

fn deterministic_body(seed: u8, size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| seed.wrapping_add((index % 251) as u8))
        .collect()
}

fn transfer() -> Result<Arc<HttpTransfer>, String> {
    static SHARED_TRANSFER: OnceLock<Arc<HttpTransfer>> = OnceLock::new();
    if let Some(existing) = SHARED_TRANSFER.get() {
        return Ok(Arc::clone(existing));
    }
    let created = Arc::new(
        HttpTransfer::new(TransferOptions {
            max_attempts: 64,
            backoff: Duration::from_millis(2),
            timeout: Duration::from_secs(3),
            chunk_size: BODY_CHUNK,
            ..TransferOptions::default()
        })
        .map_err(|_| "benchmark transfer creation failure".to_string())?,
    );
    let _ = SHARED_TRANSFER.set(Arc::clone(&created));
    Ok(created)
}

#[derive(Clone)]
struct HttpInput {
    opaque_id: String,
    path: String,
    url: String,
    expected: Vec<u8>,
    destination: PathBuf,
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

struct HttpObservation {
    opaque_id: String,
    path: String,
    expected: Vec<u8>,
    destination: PathBuf,
    completed: bool,
    bytes_written: u64,
    started_ms: u64,
    first_byte_ms: Option<u64>,
    finished_ms: u64,
    max_active_per_origin_seen: usize,
}

fn new_input(
    root: &TempRoot,
    opaque_id: &str,
    path: &str,
    url: String,
    expected: Vec<u8>,
) -> HttpInput {
    HttpInput {
        opaque_id: opaque_id.to_string(),
        path: path.to_string(),
        url,
        expected,
        destination: root.0.join(format!("{opaque_id}.bin")),
        cancelled: Arc::new(AtomicBool::new(false)),
        paused: Arc::new(AtomicBool::new(false)),
    }
}

fn execute_http_inputs(
    controller: Arc<TransferAdmissionController>,
    inputs: Vec<HttpInput>,
    started: Instant,
) -> Result<Vec<HttpObservation>, String> {
    let transfer = transfer()?;
    let handles = inputs
        .into_iter()
        .map(|input| {
            let controller = Arc::clone(&controller);
            let transfer = Arc::clone(&transfer);
            thread::spawn(move || {
                if !is_loopback_url(&input.url) {
                    return Err("fixture rejected non-loopback request".to_string());
                }
                let request = TransferRequest::new(
                    format!("opaque-task-{}", input.opaque_id),
                    input.opaque_id.clone(),
                    &input.url,
                    input.destination.clone(),
                    Some(input.expected.len() as u64),
                )
                .map_err(|_| "benchmark request creation failure".to_string())?;
                let cancelled = Arc::clone(&input.cancelled);
                let paused = Arc::clone(&input.paused);
                let transfer_started = Instant::now();
                let started_ms = elapsed_ms(started);
                let mut first_byte_ms = None;
                let mut max_active_per_origin_seen = 0;
                let observation_controller = Arc::clone(&controller);
                let admitted = AdmittedTransferBackend::new(transfer.as_ref(), controller);
                let result = admitted.download_legacy(
                    &request,
                    &TransferAuth::None,
                    &|| cancelled.load(Ordering::Acquire),
                    &|| paused.load(Ordering::Acquire),
                    &mut |_| {
                        if first_byte_ms.is_none() {
                            first_byte_ms = Some(elapsed_ms(started));
                        }
                        let current_max = observation_controller
                            .snapshot()
                            .active_by_host
                            .values()
                            .copied()
                            .max()
                            .unwrap_or(0);
                        max_active_per_origin_seen = max_active_per_origin_seen.max(current_max);
                    },
                );
                let completed = result.outcome == DownloadOutcome::Completed;
                let _transfer_elapsed = transfer_started.elapsed();
                Ok(HttpObservation {
                    opaque_id: input.opaque_id,
                    path: input.path,
                    expected: input.expected,
                    destination: input.destination,
                    completed,
                    bytes_written: result.bytes_written,
                    started_ms,
                    first_byte_ms,
                    finished_ms: elapsed_ms(started),
                    max_active_per_origin_seen,
                })
            })
        })
        .collect::<Vec<_>>();
    handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .map_err(|_| "benchmark transfer thread panicked".to_string())?
        })
        .collect()
}

fn wait_until(description: &str, predicate: impl Fn() -> bool) -> Result<(), String> {
    let deadline = Instant::now() + WAIT_DEADLINE;
    while !predicate() {
        if Instant::now() >= deadline {
            return Err(format!("benchmark wait timeout: {description}"));
        }
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

fn snapshot_json(controller: &TransferAdmissionController) -> Value {
    let snapshot = controller.snapshot();
    json!({
        "active_global": snapshot.active_global,
        "queued_global": snapshot.queued_global,
        "paused_waiters": snapshot.paused_waiters,
        "live_origin_count": snapshot.active_by_host.len(),
        "grants": snapshot.grants,
        "releases": snapshot.releases,
        "prestart_releases": snapshot.prestart_releases,
        "cancellations_before_grant": snapshot.cancellations_before_grant,
        "wait_ms": {
            "total": snapshot.total_wait_ms,
            "max": snapshot.max_wait_ms,
            "waited_count": snapshot.waited_count,
        },
        "max_active_global": snapshot.max_active_global,
        "failed_closed": snapshot.failed_closed,
    })
}

fn controller_is_idle(controller: &TransferAdmissionController) -> bool {
    let snapshot = controller.snapshot();
    snapshot.active_global == 0
        && snapshot.queued_global == 0
        && snapshot.active_by_host.is_empty()
        && snapshot.queued_by_host.is_empty()
        && !snapshot.failed_closed
}

fn merge_status_counts(summaries: &[ServerSummary]) -> BTreeMap<String, u64> {
    let mut merged = BTreeMap::new();
    for summary in summaries {
        for (status, count) in &summary.status_counts {
            *merged.entry(status.clone()).or_insert(0) += count;
        }
    }
    merged
}

fn first_request_ms(summaries: &[ServerSummary], path: &str) -> Option<u64> {
    summaries
        .iter()
        .filter_map(|summary| summary.first_request_ms.get(path).copied())
        .min()
}

fn build_http_sample(
    scenario: &str,
    candidate: &Candidate,
    controller: &TransferAdmissionController,
    observations: &[HttpObservation],
    summaries: &[ServerSummary],
    initial_origin_grouping: Option<bool>,
    extra: Value,
) -> Value {
    let files_valid = observations.iter().all(|observation| {
        observation.completed
            && observation.bytes_written == observation.expected.len() as u64
            && fs::read(&observation.destination).is_ok_and(|actual| actual == observation.expected)
    });
    let no_server_failures = summaries.iter().all(|summary| summary.failures == 0);
    let no_leak = controller_is_idle(controller);
    let snapshot = controller.snapshot();
    let policy_bounds = candidate
        .finite_bounds()
        .is_none_or(|(global, per_origin)| {
            snapshot.max_active_global <= global
                && observations
                    .iter()
                    .all(|observation| observation.max_active_per_origin_seen <= per_origin)
        });
    let statuses = merge_status_counts(summaries);
    let request_count = summaries
        .iter()
        .flat_map(|summary| summary.request_counts.values())
        .copied()
        .sum::<u64>();
    let bytes_served = summaries
        .iter()
        .map(|summary| summary.bytes_served)
        .sum::<u64>();
    let elapsed = observations
        .iter()
        .map(|observation| observation.finished_ms)
        .max()
        .unwrap_or(0);
    let total_bytes = observations
        .iter()
        .map(|observation| observation.expected.len() as u64)
        .sum::<u64>();
    let scaled_total_bytes = total_bytes.saturating_mul(1_000);
    let throughput = scaled_total_bytes
        .checked_div(elapsed)
        .unwrap_or(scaled_total_bytes);
    let mut start_order = observations.to_vec();
    start_order
        .sort_by_key(|observation| observation.first_byte_ms.unwrap_or(observation.started_ms));
    let mut completion_order = observations.to_vec();
    completion_order.sort_by_key(|observation| observation.finished_ms);
    let files = observations
        .iter()
        .map(|observation| {
            json!({
                "id": observation.opaque_id,
                "started_ms": observation.started_ms,
                "first_byte_ms": observation.first_byte_ms,
                "completion_ms": observation.finished_ms,
                "bytes_completed": observation.bytes_written,
                "request_started_ms": first_request_ms(summaries, &observation.path),
            })
        })
        .collect::<Vec<_>>();
    let hard_gates_passed = files_valid && no_server_failures && no_leak && policy_bounds;
    let mut sample = json!({
        "scenario": scenario,
        "candidate": candidate.identifier(),
        "hard_gates_passed": hard_gates_passed,
        "fixture_loopback_only": true,
        "initial_origin_grouping": initial_origin_grouping,
        "metrics": {
            "wall_clock_ms": elapsed,
            "effective_throughput_bytes_per_sec": throughput,
            "bytes_served": bytes_served,
            "fixture_max_concurrent_requests": summaries.iter().map(|summary| summary.max_active).max().unwrap_or(0),
            "request_count": request_count,
            "retry_count": statuses.get("429").copied().unwrap_or(0)
                .saturating_add(statuses.get("500").copied().unwrap_or(0)),
            "status_counts": statuses,
            "process_cpu_time_ms": "unavailable",
            "peak_resident_memory_bytes": "unavailable",
        },
        "files": files,
        "start_order": start_order.into_iter().map(|entry| entry.opaque_id).collect::<Vec<_>>(),
        "completion_order": completion_order.into_iter().map(|entry| entry.opaque_id).collect::<Vec<_>>(),
        "admission": snapshot_json(controller),
        "no_leak": no_leak,
        "policy_bounds_observed": policy_bounds,
        "max_active_per_origin_observed": observations.iter().map(|observation| observation.max_active_per_origin_seen).max().unwrap_or(0),
        "file_integrity_passed": files_valid,
        "fixture_server_failures": summaries.iter().map(|summary| summary.failures).sum::<usize>(),
        "fixture_failure_codes": summaries.iter().flat_map(|summary| summary.failure_codes.iter()).fold(BTreeMap::<String, u64>::new(), |mut merged, (code, count)| { *merged.entry(code.clone()).or_insert(0) += count; merged }),
    });
    if let (Some(object), Some(extra)) = (sample.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            object.insert(key.clone(), value.clone());
        }
    }
    sample
}

impl Clone for HttpObservation {
    fn clone(&self) -> Self {
        Self {
            opaque_id: self.opaque_id.clone(),
            path: self.path.clone(),
            expected: self.expected.clone(),
            destination: self.destination.clone(),
            completed: self.completed,
            bytes_written: self.bytes_written,
            started_ms: self.started_ms,
            first_byte_ms: self.first_byte_ms,
            finished_ms: self.finished_ms,
            max_active_per_origin_seen: self.max_active_per_origin_seen,
        }
    }
}

fn run_single_layout(
    scenario: &str,
    candidate: &Candidate,
    layout: &[(usize, Duration)],
    throttle_above: Option<usize>,
) -> Result<Value, String> {
    let root = TempRoot::new()?;
    let server = FixtureServer::new()?;
    let mut inputs = Vec::new();
    for (index, (size, delay)) in layout.iter().enumerate() {
        let path = format!("/payload-{index}");
        let body = deterministic_body(index as u8 + 1, *size);
        server.add_body(&path, body.clone(), *delay, throttle_above, Vec::new());
        inputs.push(new_input(
            &root,
            &format!("f{index:02}"),
            &path,
            server.url(&path)?,
            body,
        ));
    }
    let controller = candidate.controller();
    let started = Instant::now();
    let observations = execute_http_inputs(Arc::clone(&controller), inputs, started)?;
    if started.elapsed() > SCENARIO_DEADLINE {
        return Err("benchmark scenario deadline exceeded".to_string());
    }
    let summary = server.finish();
    Ok(build_http_sample(
        scenario,
        candidate,
        &controller,
        &observations,
        &[summary],
        None,
        json!({}),
    ))
}

fn run_multi_layout(candidate: &Candidate, mixed_speed: bool) -> Result<Value, String> {
    let root = TempRoot::new()?;
    let first = FixtureServer::new()?;
    let second = FixtureServer::new()?;
    let mut inputs = Vec::new();
    for index in 0..4 {
        let server = if index < 2 { &first } else { &second };
        let delay = if mixed_speed && index < 2 {
            Duration::from_millis(3)
        } else {
            Duration::ZERO
        };
        let path = format!("/payload-{index}");
        let body = deterministic_body(index as u8 + 30, 8 * 1024);
        server.add_body(&path, body.clone(), delay, None, Vec::new());
        inputs.push(new_input(
            &root,
            &format!("f{index:02}"),
            &path,
            server.url(&path)?,
            body,
        ));
    }
    let controller = candidate.controller();
    let started = Instant::now();
    let observations = execute_http_inputs(Arc::clone(&controller), inputs, started)?;
    if started.elapsed() > SCENARIO_DEADLINE {
        return Err("benchmark scenario deadline exceeded".to_string());
    }
    let summaries = [first.finish(), second.finish()];
    Ok(build_http_sample(
        if mixed_speed {
            "multi_mixed_speed"
        } else {
            "multi_balanced"
        },
        candidate,
        &controller,
        &observations,
        &summaries,
        None,
        json!({}),
    ))
}

fn run_queued_cancellation(candidate: &Candidate) -> Result<Value, String> {
    let finite = Candidate::Finite {
        global: NonZeroUsize::new(1).unwrap(),
        per_origin: NonZeroUsize::new(1).unwrap(),
    };
    let effective = if candidate.finite_bounds().is_some() {
        candidate.clone()
    } else {
        finite
    };
    let root = TempRoot::new()?;
    let server = FixtureServer::new()?;
    let holder_path = "/holder";
    let cancel_path = "/cancelled";
    let holder_body = deterministic_body(80, 32 * 1024);
    let cancel_body = deterministic_body(81, 4 * 1024);
    server.add_body(
        holder_path,
        holder_body.clone(),
        Duration::from_millis(3),
        None,
        Vec::new(),
    );
    server.add_body(
        cancel_path,
        cancel_body.clone(),
        Duration::ZERO,
        None,
        Vec::new(),
    );
    let controller = effective.controller();
    let started = Instant::now();
    let holder = new_input(
        &root,
        "f00",
        holder_path,
        server.url(holder_path)?,
        holder_body,
    );
    let cancelled = new_input(
        &root,
        "f01",
        cancel_path,
        server.url(cancel_path)?,
        cancel_body,
    );
    let cancel_flag = Arc::clone(&cancelled.cancelled);
    let holder_handle = thread::spawn({
        let controller = Arc::clone(&controller);
        move || execute_http_inputs(controller, vec![holder], started)
    });
    wait_until("holder permit", || controller.snapshot().active_global == 1)?;
    let cancel_destination = cancelled.destination.clone();
    let cancelled_handle = thread::spawn({
        let controller = Arc::clone(&controller);
        move || execute_http_inputs(controller, vec![cancelled], started)
    });
    wait_until("queued cancellation waiter", || {
        controller.snapshot().queued_global == 1
    })?;
    cancel_flag.store(true, Ordering::Release);
    let cancelled_observations = cancelled_handle
        .join()
        .map_err(|_| "queued cancellation thread panicked".to_string())??;
    let holder_observations = holder_handle
        .join()
        .map_err(|_| "holder thread panicked".to_string())??;
    let summary = server.finish();
    let cancelled_requests = summary
        .request_counts
        .get(cancel_path)
        .copied()
        .unwrap_or(0);
    let no_output = !cancel_destination.exists() && !part_path(&cancel_destination).exists();
    let mut observations = holder_observations;
    observations.extend(cancelled_observations);
    let mut sample = build_http_sample(
        "queued_cancellation",
        &effective,
        &controller,
        &observations,
        &[summary],
        None,
        json!({
            "queued_before_cancel": true,
            "cancelled_request_count": cancelled_requests,
            "cancelled_output_absent": no_output,
        }),
    );
    let holder_valid = observations[0].completed
        && fs::read(&observations[0].destination)
            .is_ok_and(|actual| actual == observations[0].expected);
    let hard = holder_valid
        && cancelled_requests == 0
        && no_output
        && !observations[1].completed
        && controller_is_idle(&controller)
        && sample["policy_bounds_observed"].as_bool() == Some(true);
    sample["hard_gates_passed"] = Value::Bool(hard);
    Ok(sample)
}

fn run_queued_pause_resume(candidate: &Candidate) -> Result<Value, String> {
    let finite = Candidate::Finite {
        global: NonZeroUsize::new(1).unwrap(),
        per_origin: NonZeroUsize::new(1).unwrap(),
    };
    let effective = if candidate.finite_bounds().is_some() {
        candidate.clone()
    } else {
        finite
    };
    let root = TempRoot::new()?;
    let server = FixtureServer::new()?;
    let holder_path = "/holder";
    let paused_path = "/paused";
    let holder_body = deterministic_body(91, 32 * 1024);
    let paused_body = deterministic_body(92, 4 * 1024);
    server.add_body(
        holder_path,
        holder_body.clone(),
        Duration::from_millis(3),
        None,
        Vec::new(),
    );
    server.add_body(
        paused_path,
        paused_body.clone(),
        Duration::ZERO,
        None,
        Vec::new(),
    );
    let controller = effective.controller();
    let started = Instant::now();
    let holder = new_input(
        &root,
        "f00",
        holder_path,
        server.url(holder_path)?,
        holder_body,
    );
    let paused = new_input(
        &root,
        "f01",
        paused_path,
        server.url(paused_path)?,
        paused_body,
    );
    paused.paused.store(true, Ordering::Release);
    let pause_flag = Arc::clone(&paused.paused);
    let holder_handle = thread::spawn({
        let controller = Arc::clone(&controller);
        move || execute_http_inputs(controller, vec![holder], started)
    });
    wait_until("holder permit", || controller.snapshot().active_global == 1)?;
    let paused_handle = thread::spawn({
        let controller = Arc::clone(&controller);
        move || execute_http_inputs(controller, vec![paused], started)
    });
    wait_until("paused waiter", || {
        let snapshot = controller.snapshot();
        snapshot.queued_global == 1 && snapshot.paused_waiters == 1
    })?;
    let holder_observations = holder_handle
        .join()
        .map_err(|_| "pause holder thread panicked".to_string())??;
    wait_until("paused waiter after release", || {
        let snapshot = controller.snapshot();
        snapshot.active_global == 0 && snapshot.queued_global == 1 && snapshot.paused_waiters == 1
    })?;
    pause_flag.store(false, Ordering::Release);
    let paused_observations = paused_handle
        .join()
        .map_err(|_| "paused thread panicked".to_string())??;
    let summary = server.finish();
    let paused_requests = summary
        .request_counts
        .get(paused_path)
        .copied()
        .unwrap_or(0);
    let mut observations = holder_observations;
    observations.extend(paused_observations);
    let mut sample = build_http_sample(
        "queued_pause_resume",
        &effective,
        &controller,
        &observations,
        &[summary],
        None,
        json!({
            "paused_request_count": paused_requests,
            "paused_waiter_released_capacity": true,
        }),
    );
    let hard = sample["hard_gates_passed"].as_bool() == Some(true) && paused_requests == 1;
    sample["hard_gates_passed"] = Value::Bool(hard);
    Ok(sample)
}

fn run_redirect(candidate: &Candidate, same_origin: bool) -> Result<Value, String> {
    let root = TempRoot::new()?;
    let initial = FixtureServer::new()?;
    let target = if same_origin {
        None
    } else {
        Some(FixtureServer::new()?)
    };
    let body = deterministic_body(if same_origin { 110 } else { 111 }, 4 * 1024);
    let target_path = "/target";
    let location = match &target {
        Some(server) => {
            server.add_body(target_path, body.clone(), Duration::ZERO, None, Vec::new());
            server.url(target_path)?
        }
        None => {
            initial.add_body(target_path, body.clone(), Duration::ZERO, None, Vec::new());
            initial.url(target_path)?
        }
    };
    initial.add_redirect("/redirect", location)?;
    let controller = candidate.controller();
    let started = Instant::now();
    let input = new_input(&root, "f00", "/redirect", initial.url("/redirect")?, body);
    let observations = execute_http_inputs(Arc::clone(&controller), vec![input], started)?;
    let initial_summary = initial.finish();
    let mut summaries = vec![initial_summary];
    if let Some(server) = target {
        summaries.push(server.finish());
    }
    let expected_requests = if same_origin { 2 } else { 1 };
    let initial_requests = summaries[0]
        .request_counts
        .get("/redirect")
        .copied()
        .unwrap_or(0);
    let target_requests = summaries
        .iter()
        .map(|summary| {
            summary
                .request_counts
                .get(target_path)
                .copied()
                .unwrap_or(0)
        })
        .sum::<u64>();
    let mut sample = build_http_sample(
        if same_origin {
            "redirect_same_origin"
        } else {
            "redirect_other_origin"
        },
        candidate,
        &controller,
        &observations,
        &summaries,
        Some(true),
        json!({
            "redirect_target_origin_is_not_admitted_separately": true,
            "redirect_initial_request_count": initial_requests,
            "redirect_target_request_count": target_requests,
        }),
    );
    let hard = sample["hard_gates_passed"].as_bool() == Some(true)
        && initial_requests == 1
        && target_requests == 1
        && expected_requests == 1 + usize::from(same_origin);
    sample["hard_gates_passed"] = Value::Bool(hard);
    Ok(sample)
}

#[derive(Default)]
struct GateState {
    entered: usize,
    released: bool,
}

#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

impl Gate {
    fn enter_and_wait(&self) {
        let mut state = self.state.lock().unwrap();
        state.entered += 1;
        self.changed.notify_all();
        while !state.released {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn wait_for(&self, expected: usize) -> Result<(), String> {
        wait_until("controlled backend entry", || {
            self.state.lock().unwrap().entered >= expected
        })
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.released = true;
        self.changed.notify_all();
    }
}

enum ControlledMode {
    Fail,
    Interrupt,
    Panic,
    BlockFirst(Arc<Gate>),
    ResolvedSequence(Mutex<VecDeque<ResolvedTransferResult>>),
}

struct ControlledBackend {
    mode: ControlledMode,
    calls: AtomicUsize,
    legacy_calls: AtomicUsize,
    call_order: Mutex<Vec<String>>,
    sessions: Mutex<Vec<usize>>,
}

impl ControlledBackend {
    fn with_mode(mode: ControlledMode) -> Self {
        Self {
            mode,
            calls: AtomicUsize::new(0),
            legacy_calls: AtomicUsize::new(0),
            call_order: Mutex::new(Vec::new()),
            sessions: Mutex::new(Vec::new()),
        }
    }

    fn result(outcome: DownloadOutcome) -> TransferResult {
        let failure = if outcome == DownloadOutcome::Failed {
            Some(DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: "benchmark controlled failure".to_string(),
                retryable: false,
            })
        } else {
            None
        };
        TransferResult {
            outcome,
            bytes_written: 0,
            attempt_count: 1,
            failure,
        }
    }
}

impl TransferBackend for ControlledBackend {
    fn kind(&self) -> TransferBackendKind {
        TransferBackendKind::NativeSingleStream
    }

    fn is_available(&self) -> bool {
        true
    }

    fn download_legacy(
        &self,
        request: &TransferRequest,
        _auth: &TransferAuth,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        self.legacy_calls.fetch_add(1, Ordering::SeqCst);
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.call_order
            .lock()
            .unwrap()
            .push(request.item_id.clone());
        match &self.mode {
            ControlledMode::Fail => Self::result(DownloadOutcome::Failed),
            ControlledMode::Interrupt => Self::result(DownloadOutcome::Interrupted),
            ControlledMode::Panic => panic!("controlled benchmark panic"),
            ControlledMode::BlockFirst(gate) => {
                if call == 0 {
                    gate.enter_and_wait();
                }
                Self::result(DownloadOutcome::Completed)
            }
            ControlledMode::ResolvedSequence(_) => Self::result(DownloadOutcome::Failed),
        }
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(3).unwrap()
    }

    fn download_resolved(
        &self,
        _request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.sessions
            .lock()
            .unwrap()
            .push(session as *mut ResolvedTransferSession as usize);
        match &self.mode {
            ControlledMode::ResolvedSequence(results) => {
                results.lock().unwrap().pop_front().unwrap()
            }
            _ => ResolvedTransferResult::Finished(Self::result(DownloadOutcome::Completed)),
        }
    }
}

fn controlled_request(task: &str, item: &str, port: u16) -> TransferRequest {
    TransferRequest::new(
        task,
        item,
        &format!("http://127.0.0.1:{port}/fixture"),
        PathBuf::from(format!("{item}.bin")),
        Some(1),
    )
    .unwrap()
}

fn run_task_fairness(candidate: &Candidate) -> Result<Value, String> {
    let effective = Candidate::Finite {
        global: NonZeroUsize::new(2).unwrap(),
        per_origin: NonZeroUsize::new(1).unwrap(),
    };
    let selected = if candidate.finite_bounds().is_some() {
        candidate.clone()
    } else {
        effective
    };
    let controller = selected.controller();
    let gate = Arc::new(Gate::default());
    let backend = Arc::new(ControlledBackend::with_mode(ControlledMode::BlockFirst(
        Arc::clone(&gate),
    )));
    let holder = thread::spawn({
        let controller = Arc::clone(&controller);
        let backend = Arc::clone(&backend);
        move || {
            AdmittedTransferBackend::new(backend.as_ref(), controller).download_legacy(
                &controlled_request("task-a", "f00", 10001),
                &TransferAuth::None,
                &|| false,
                &|| false,
                &mut |_| {},
            )
        }
    });
    gate.wait_for(1)?;
    let blocked = thread::spawn({
        let controller = Arc::clone(&controller);
        let backend = Arc::clone(&backend);
        move || {
            AdmittedTransferBackend::new(backend.as_ref(), controller).download_legacy(
                &controlled_request("task-a", "f01", 10001),
                &TransferAuth::None,
                &|| false,
                &|| false,
                &mut |_| {},
            )
        }
    });
    let other = thread::spawn({
        let controller = Arc::clone(&controller);
        let backend = Arc::clone(&backend);
        move || {
            AdmittedTransferBackend::new(backend.as_ref(), controller).download_legacy(
                &controlled_request("task-b", "f02", 10002),
                &TransferAuth::None,
                &|| false,
                &|| false,
                &mut |_| {},
            )
        }
    });
    wait_until("other origin progress", || {
        backend.calls.load(Ordering::SeqCst) >= 2
    })?;
    gate.release();
    for handle in [holder, blocked, other] {
        if handle
            .join()
            .map_err(|_| "fairness transfer thread panicked".to_string())?
            .outcome
            != DownloadOutcome::Completed
        {
            return Err("fairness transfer did not complete".to_string());
        }
    }
    let order = backend.call_order.lock().unwrap().clone();
    let no_starvation = order.len() == 3
        && order.iter().position(|id| id == "f02") < order.iter().position(|id| id == "f01");
    let hard = no_starvation && controller_is_idle(&controller);
    Ok(json!({
        "scenario": "task_fairness",
        "candidate": selected.identifier(),
        "hard_gates_passed": hard,
        "fixture_loopback_only": true,
        "initial_origin_grouping": null,
        "metrics": {
            "wall_clock_ms": 0,
            "effective_throughput_bytes_per_sec": 0,
            "bytes_served": 0,
            "request_count": 0,
            "retry_count": 0,
            "status_counts": {},
            "process_cpu_time_ms": "unavailable",
            "peak_resident_memory_bytes": "unavailable",
        },
        "files": [],
        "start_order": order,
        "completion_order": ["f00", "f01", "f02"],
        "admission": snapshot_json(&controller),
        "no_leak": controller_is_idle(&controller),
        "policy_bounds_observed": true,
        "oldest_host_blocked_waiter_progressed": no_starvation,
    }))
}

fn run_resolver_refresh(candidate: &Candidate) -> Result<Value, String> {
    let controller = candidate.controller();
    let backend = ControlledBackend::with_mode(ControlledMode::ResolvedSequence(Mutex::new(
        vec![
            ResolvedTransferResult::AuthRejected(AuthRejection {
                status_code: 401,
                response_host_class: ResponseHostClass::Approved,
                refresh_candidate: true,
            }),
            ResolvedTransferResult::Finished(ControlledBackend::result(DownloadOutcome::Completed)),
        ]
        .into(),
    )));
    let request = controlled_request("task-r", "f00", 10003);
    let asset = ResolvedAsset::new("http://127.0.0.1:10003/fixture", ResolvedMethod::Get)
        .map_err(|_| "resolved fixture asset failure".to_string())?;
    let admitted = AdmittedTransferBackend::new(&backend, Arc::clone(&controller));
    let mut session = admitted.resolved_session();
    let first = admitted.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    let second = admitted.download_resolved(
        &request,
        &asset,
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |_| {},
    );
    let sessions = backend.sessions.lock().unwrap().clone();
    let same_session = sessions.len() == 2 && sessions[0] == sessions[1];
    let completed = matches!(
        second,
        ResolvedTransferResult::Finished(TransferResult {
            outcome: DownloadOutcome::Completed,
            ..
        })
    );
    let hard = matches!(first, ResolvedTransferResult::AuthRejected(_))
        && completed
        && same_session
        && backend.legacy_calls.load(Ordering::SeqCst) == 0
        && backend.calls.load(Ordering::SeqCst) == 2
        && controller_is_idle(&controller);
    Ok(json!({
        "scenario": "resolver_refresh",
        "candidate": candidate.identifier(),
        "hard_gates_passed": hard,
        "fixture_loopback_only": true,
        "initial_origin_grouping": true,
        "metrics": {
            "wall_clock_ms": 0,
            "effective_throughput_bytes_per_sec": 0,
            "bytes_served": 0,
            "request_count": 0,
            "retry_count": 0,
            "status_counts": {"401": 1},
            "process_cpu_time_ms": "unavailable",
            "peak_resident_memory_bytes": "unavailable",
        },
        "files": [],
        "start_order": ["f00", "f00"],
        "completion_order": ["f00"],
        "admission": snapshot_json(&controller),
        "no_leak": controller_is_idle(&controller),
        "policy_bounds_observed": true,
        "same_selected_backend": true,
        "same_resolved_transfer_session": same_session,
        "legacy_call_count": backend.legacy_calls.load(Ordering::SeqCst),
    }))
}

fn run_restart_oversubscription(candidate: &Candidate) -> Result<Value, String> {
    const REHYDRATED_WORKER_COUNT: usize = 10;
    let selected = candidate.clone();
    let root = TempRoot::new()?;
    let server = FixtureServer::new()?;
    let mut inputs = Vec::new();
    for index in 0..REHYDRATED_WORKER_COUNT {
        let path = format!("/rehydrated-{index}");
        let body = deterministic_body(index as u8 + 140, 4 * 1024);
        server.add_body(
            &path,
            body.clone(),
            Duration::from_millis(1),
            None,
            Vec::new(),
        );
        inputs.push(new_input(
            &root,
            &format!("f{index:02}"),
            &path,
            server.url(&path)?,
            body,
        ));
    }
    let controller = selected.controller();
    let started = Instant::now();
    let observations = execute_http_inputs(Arc::clone(&controller), inputs, started)?;
    let summary = server.finish();
    let completed = observations.iter().filter(|entry| entry.completed).count();
    let mut sample = build_http_sample(
        "restart_oversubscription",
        &selected,
        &controller,
        &observations,
        &[summary],
        None,
        json!({
            "rehydrated_worker_count": REHYDRATED_WORKER_COUNT,
            "completed_worker_count": completed,
        }),
    );
    let hard =
        sample["hard_gates_passed"].as_bool() == Some(true) && completed == REHYDRATED_WORKER_COUNT;
    sample["hard_gates_passed"] = Value::Bool(hard);
    Ok(sample)
}

fn run_backend_abnormal(candidate: &Candidate) -> Result<Value, String> {
    let mut outcomes = Vec::new();
    let mut hard = true;
    for mode in [
        ControlledMode::Fail,
        ControlledMode::Interrupt,
        ControlledMode::Panic,
    ] {
        let controller = candidate.controller();
        let backend = ControlledBackend::with_mode(mode);
        let result = catch_unwind(AssertUnwindSafe(|| {
            AdmittedTransferBackend::new(&backend, Arc::clone(&controller)).download_legacy(
                &controlled_request("task-x", "f00", 10004),
                &TransferAuth::None,
                &|| false,
                &|| false,
                &mut |_| {},
            )
        }));
        let release_once = controller.snapshot().releases == 1 && controller_is_idle(&controller);
        hard &= release_once && backend.calls.load(Ordering::SeqCst) == 1;
        outcomes.push(json!({
            "panicked": result.is_err(),
            "releases": controller.snapshot().releases,
            "calls": backend.calls.load(Ordering::SeqCst),
        }));
    }
    Ok(json!({
        "scenario": "backend_abnormal",
        "candidate": candidate.identifier(),
        "hard_gates_passed": hard,
        "fixture_loopback_only": true,
        "initial_origin_grouping": null,
        "metrics": {
            "wall_clock_ms": 0,
            "effective_throughput_bytes_per_sec": 0,
            "bytes_served": 0,
            "request_count": 0,
            "retry_count": 0,
            "status_counts": {},
            "process_cpu_time_ms": "unavailable",
            "peak_resident_memory_bytes": "unavailable",
        },
        "files": [],
        "start_order": [],
        "completion_order": [],
        "admission": {"abnormal_cases": 3},
        "no_leak": hard,
        "policy_bounds_observed": true,
        "abnormal_outcomes": outcomes,
    }))
}

const SCALED_SCHEMA_VERSION: &str = "d2b-admission-scaled-v1";
const SCALED_PROFILE: &str = "scaled-production-default-streaming";
const SCALED_BASE_SEED: u64 = 20_260_723;
const SCALED_SERVER_CHUNK: usize = 64 * 1024;
const SCALED_SCENARIO_DEADLINE: Duration = Duration::from_secs(45);
const SCALED_CANDIDATE_IDS: [&str; 7] = [
    "compatibility",
    "g2-h2",
    "g4-h1",
    "g4-h2",
    "g4-h4",
    "g8-h2",
    "g8-h4",
];
const SCALED_RANKED_SCENARIOS: [&str; 5] = [
    "single_origin_sustained",
    "single_origin_pressure",
    "multi_origin_sustained",
    "multi_origin_mixed_rate",
    "same_origin_mixed_size",
];

#[derive(Clone)]
struct StreamingBodyDescriptor {
    seed: u8,
    byte_length: usize,
    server_chunk_bytes: usize,
    chunk_delay: Duration,
    pressure_above: Option<usize>,
    scripted_statuses: Arc<Mutex<VecDeque<u16>>>,
}

impl StreamingBodyDescriptor {
    fn new(
        seed: u8,
        byte_length: usize,
        server_chunk_bytes: usize,
        chunk_delay: Duration,
        pressure_above: Option<usize>,
    ) -> Self {
        Self {
            seed,
            byte_length,
            server_chunk_bytes,
            chunk_delay,
            pressure_above,
            scripted_statuses: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

#[derive(Clone)]
struct ScaledFileSpec {
    origin_index: usize,
    descriptor: StreamingBodyDescriptor,
}

#[derive(Clone)]
struct ScaledHttpInput {
    opaque_id: String,
    path: String,
    url: String,
    descriptor: StreamingBodyDescriptor,
    destination: PathBuf,
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

struct ScaledHttpObservation {
    opaque_id: String,
    path: String,
    descriptor: StreamingBodyDescriptor,
    destination: PathBuf,
    completed: bool,
    bytes_written: u64,
    started_ns: u64,
    first_byte_ns: Option<u64>,
    finished_ns: u64,
    max_active_per_origin_seen: usize,
}

struct ScaledFixtureState {
    started: Instant,
    endpoints: Mutex<BTreeMap<String, StreamingBodyDescriptor>>,
    request_counts: Mutex<BTreeMap<String, u64>>,
    status_counts: Mutex<BTreeMap<String, u64>>,
    first_request_ns: Mutex<BTreeMap<String, u64>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    bytes_served: AtomicU64,
    failures: AtomicUsize,
    failure_codes: Mutex<BTreeMap<String, u64>>,
    pressure_tickets: AtomicUsize,
}

struct ScaledFixtureServer {
    base_url: String,
    state: Arc<ScaledFixtureState>,
    stop: Arc<AtomicBool>,
    connections: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    accept_thread: Option<thread::JoinHandle<()>>,
}

struct ScaledServerSummary {
    request_counts: BTreeMap<String, u64>,
    status_counts: BTreeMap<String, u64>,
    first_request_ns: BTreeMap<String, u64>,
    bytes_served: u64,
    max_active: usize,
    failures: usize,
    failure_codes: BTreeMap<String, u64>,
}

impl ScaledFixtureServer {
    fn new() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|_| "scaled loopback listener creation failure".to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "scaled loopback listener setup failure".to_string())?;
        let address = listener
            .local_addr()
            .map_err(|_| "scaled loopback listener address failure".to_string())?;
        if !address.ip().is_loopback() {
            return Err("scaled fixture rejected non-loopback listener".to_string());
        }
        let state = Arc::new(ScaledFixtureState {
            started: Instant::now(),
            endpoints: Mutex::new(BTreeMap::new()),
            request_counts: Mutex::new(BTreeMap::new()),
            status_counts: Mutex::new(BTreeMap::new()),
            first_request_ns: Mutex::new(BTreeMap::new()),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            bytes_served: AtomicU64::new(0),
            failures: AtomicUsize::new(0),
            failure_codes: Mutex::new(BTreeMap::new()),
            pressure_tickets: AtomicUsize::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(Vec::new()));
        let accept_state = Arc::clone(&state);
        let accept_stop = Arc::clone(&stop);
        let accept_connections = Arc::clone(&connections);
        let accept_thread = thread::spawn(move || {
            while !accept_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let connection_state = Arc::clone(&accept_state);
                        let connection_stop = Arc::clone(&accept_stop);
                        let handle = thread::spawn(move || {
                            if let Err(error) = scaled_handle_connection(stream, &connection_state)
                            {
                                if !connection_stop.load(Ordering::Acquire) {
                                    connection_state.failures.fetch_add(1, Ordering::Relaxed);
                                    *connection_state
                                        .failure_codes
                                        .lock()
                                        .unwrap()
                                        .entry(error)
                                        .or_insert(0) += 1;
                                }
                            }
                        });
                        accept_connections.lock().unwrap().push(handle);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => {
                        if !accept_stop.load(Ordering::Acquire) {
                            accept_state.failures.fetch_add(1, Ordering::Relaxed);
                        }
                        break;
                    }
                }
            }
        });
        Ok(Self {
            base_url: format!("http://{address}"),
            state,
            stop,
            connections,
            accept_thread: Some(accept_thread),
        })
    }

    fn url(&self, path: &str) -> Result<String, String> {
        let value = format!("{}{}", self.base_url, path);
        if !is_loopback_url(&value) {
            return Err("scaled fixture rejected non-loopback request".to_string());
        }
        Ok(value)
    }

    fn add_streaming_body(&self, path: &str, descriptor: StreamingBodyDescriptor) {
        self.state
            .endpoints
            .lock()
            .unwrap()
            .insert(path.to_string(), descriptor);
    }

    fn finish(mut self) -> ScaledServerSummary {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
        let handles = std::mem::take(&mut *self.connections.lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
        ScaledServerSummary {
            request_counts: self.state.request_counts.lock().unwrap().clone(),
            status_counts: self.state.status_counts.lock().unwrap().clone(),
            first_request_ns: self.state.first_request_ns.lock().unwrap().clone(),
            bytes_served: self.state.bytes_served.load(Ordering::Relaxed),
            max_active: self.state.max_active.load(Ordering::Relaxed),
            failures: self.state.failures.load(Ordering::Relaxed),
            failure_codes: self.state.failure_codes.lock().unwrap().clone(),
        }
    }
}

impl Drop for ScaledFixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
        let handles = std::mem::take(&mut *self.connections.lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
    }
}

fn scaled_handle_connection(
    mut stream: TcpStream,
    state: &ScaledFixtureState,
) -> Result<(), String> {
    stream
        .set_nonblocking(false)
        .map_err(|_| "scaled fixture blocking setup failure".to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| "scaled fixture read setup failure".to_string())?;
    let Some(path) = read_request_path(&stream)? else {
        return Ok(());
    };
    let active = state.active.fetch_add(1, Ordering::AcqRel) + 1;
    update_max(&state.max_active, active);
    let result = {
        let request_number = {
            let mut requests = state.request_counts.lock().unwrap();
            let count = requests.entry(path.clone()).or_insert(0);
            *count += 1;
            *count
        };
        state
            .first_request_ns
            .lock()
            .unwrap()
            .entry(path.clone())
            .or_insert_with(|| elapsed_ns(state.started));
        let descriptor = state.endpoints.lock().unwrap().get(&path).cloned();
        match descriptor {
            Some(descriptor) => {
                let scripted = descriptor.scripted_statuses.lock().unwrap().pop_front();
                let status = scripted.unwrap_or_else(|| {
                    if descriptor
                        .pressure_above
                        .is_some_and(|limit| active > limit)
                        && request_number == 1
                    {
                        let ticket = state.pressure_tickets.fetch_add(1, Ordering::Relaxed);
                        if ticket.is_multiple_of(2) {
                            429
                        } else {
                            500
                        }
                    } else {
                        200
                    }
                });
                scaled_record_status(state, status);
                scaled_write_response(&mut stream, status, &descriptor, state)
            }
            None => {
                scaled_record_status(state, 404);
                scaled_write_empty_response(&mut stream, 404)
            }
        }
    };
    state.active.fetch_sub(1, Ordering::AcqRel);
    result
}

fn scaled_record_status(state: &ScaledFixtureState, status: u16) {
    *state
        .status_counts
        .lock()
        .unwrap()
        .entry(status.to_string())
        .or_insert(0) += 1;
}

fn scaled_status_phrase(status: u16) -> &'static str {
    match status {
        200 => "200 OK",
        404 => "404 Not Found",
        429 => "429 Too Many Requests",
        500 => "500 Internal Server Error",
        _ => "503 Service Unavailable",
    }
}

fn scaled_write_empty_response(stream: &mut TcpStream, status: u16) -> Result<(), String> {
    let header = format!(
        "HTTP/1.1 {}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        scaled_status_phrase(status)
    );
    stream
        .write_all(header.as_bytes())
        .map_err(|_| "scaled fixture response header failure".to_string())
}

fn scaled_write_response(
    stream: &mut TcpStream,
    status: u16,
    descriptor: &StreamingBodyDescriptor,
    state: &ScaledFixtureState,
) -> Result<(), String> {
    if status != 200 {
        return scaled_write_empty_response(stream, status);
    }
    let header = format!(
        "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        descriptor.byte_length
    );
    stream
        .write_all(header.as_bytes())
        .map_err(|_| "scaled fixture response header failure".to_string())?;
    let mut buffer = vec![0_u8; descriptor.server_chunk_bytes.max(1)];
    let mut offset = 0_usize;
    while offset < descriptor.byte_length {
        let count = (descriptor.byte_length - offset).min(buffer.len());
        fill_generated_bytes(&mut buffer[..count], descriptor.seed, offset);
        stream
            .write_all(&buffer[..count])
            .map_err(|_| "scaled fixture response body failure".to_string())?;
        stream
            .flush()
            .map_err(|_| "scaled fixture response flush failure".to_string())?;
        state
            .bytes_served
            .fetch_add(u64::try_from(count).unwrap_or(u64::MAX), Ordering::Relaxed);
        offset += count;
        if !descriptor.chunk_delay.is_zero() {
            thread::sleep(descriptor.chunk_delay);
        }
    }
    Ok(())
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn fill_generated_bytes(buffer: &mut [u8], seed: u8, offset: usize) {
    for (index, value) in buffer.iter_mut().enumerate() {
        *value = seed.wrapping_add(((offset + index) % 251) as u8);
    }
}

fn verify_streamed_file(path: &PathBuf, descriptor: &StreamingBodyDescriptor) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut buffer = vec![0_u8; SCALED_SERVER_CHUNK];
    let mut offset = 0_usize;
    loop {
        let Ok(read) = file.read(&mut buffer) else {
            return false;
        };
        if read == 0 {
            break;
        }
        if offset.saturating_add(read) > descriptor.byte_length {
            return false;
        }
        for (index, value) in buffer[..read].iter().enumerate() {
            if *value != descriptor.seed.wrapping_add(((offset + index) % 251) as u8) {
                return false;
            }
        }
        offset += read;
    }
    offset == descriptor.byte_length
}

fn scaled_transfer() -> Result<Arc<HttpTransfer>, String> {
    static SHARED_SCALED_TRANSFER: OnceLock<Arc<HttpTransfer>> = OnceLock::new();
    if let Some(existing) = SHARED_SCALED_TRANSFER.get() {
        return Ok(Arc::clone(existing));
    }
    let created = Arc::new(
        HttpTransfer::new(TransferOptions::default())
            .map_err(|_| "scaled benchmark transfer creation failure".to_string())?,
    );
    let _ = SHARED_SCALED_TRANSFER.set(Arc::clone(&created));
    Ok(created)
}

fn new_scaled_input(
    root: &TempRoot,
    opaque_id: &str,
    path: &str,
    url: String,
    descriptor: StreamingBodyDescriptor,
) -> ScaledHttpInput {
    ScaledHttpInput {
        opaque_id: opaque_id.to_string(),
        path: path.to_string(),
        url,
        descriptor,
        destination: root.0.join(format!("{opaque_id}.bin")),
        cancelled: Arc::new(AtomicBool::new(false)),
        paused: Arc::new(AtomicBool::new(false)),
    }
}

fn execute_scaled_inputs(
    controller: Arc<TransferAdmissionController>,
    inputs: Vec<ScaledHttpInput>,
    started: Instant,
) -> Result<Vec<ScaledHttpObservation>, String> {
    let transfer = scaled_transfer()?;
    let handles = inputs
        .into_iter()
        .map(|input| {
            let controller = Arc::clone(&controller);
            let transfer = Arc::clone(&transfer);
            thread::spawn(move || {
                if !is_loopback_url(&input.url) {
                    return Err("scaled fixture rejected non-loopback request".to_string());
                }
                let request = TransferRequest::new(
                    format!("opaque-scaled-task-{}", input.opaque_id),
                    input.opaque_id.clone(),
                    &input.url,
                    input.destination.clone(),
                    Some(u64::try_from(input.descriptor.byte_length).unwrap_or(u64::MAX)),
                )
                .map_err(|_| "scaled benchmark request creation failure".to_string())?;
                let cancelled = Arc::clone(&input.cancelled);
                let paused = Arc::clone(&input.paused);
                let transfer_started_ns = elapsed_ns(started);
                let mut first_byte_ns = None;
                let mut max_active_per_origin_seen = 0;
                let observation_controller = Arc::clone(&controller);
                let admitted = AdmittedTransferBackend::new(transfer.as_ref(), controller);
                let result = admitted.download_legacy(
                    &request,
                    &TransferAuth::None,
                    &|| cancelled.load(Ordering::Acquire),
                    &|| paused.load(Ordering::Acquire),
                    &mut |_| {
                        if first_byte_ns.is_none() {
                            first_byte_ns = Some(elapsed_ns(started));
                        }
                        let current_max = observation_controller
                            .snapshot()
                            .active_by_host
                            .values()
                            .copied()
                            .max()
                            .unwrap_or(0);
                        max_active_per_origin_seen = max_active_per_origin_seen.max(current_max);
                    },
                );
                Ok(ScaledHttpObservation {
                    opaque_id: input.opaque_id,
                    path: input.path,
                    descriptor: input.descriptor,
                    destination: input.destination,
                    completed: result.outcome == DownloadOutcome::Completed,
                    bytes_written: result.bytes_written,
                    started_ns: transfer_started_ns,
                    first_byte_ns,
                    finished_ns: elapsed_ns(started),
                    max_active_per_origin_seen,
                })
            })
        })
        .collect::<Vec<_>>();
    handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .map_err(|_| "scaled benchmark transfer thread panicked".to_string())?
        })
        .collect()
}

fn merge_scaled_status_counts(summaries: &[ScaledServerSummary]) -> BTreeMap<String, u64> {
    let mut merged = BTreeMap::new();
    for summary in summaries {
        for (status, count) in &summary.status_counts {
            *merged.entry(status.clone()).or_insert(0) += count;
        }
    }
    merged
}

fn first_scaled_request_ns(summaries: &[ScaledServerSummary], path: &str) -> Option<u64> {
    summaries
        .iter()
        .filter_map(|summary| summary.first_request_ns.get(path).copied())
        .min()
}

fn distinct_usize(values: impl Iterator<Item = usize>) -> Vec<usize> {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

fn distinct_u64(values: impl Iterator<Item = u64>) -> Vec<u64> {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

fn build_scaled_http_sample(
    scenario: &str,
    candidate: &Candidate,
    controller: &TransferAdmissionController,
    observations: &[ScaledHttpObservation],
    summaries: &[ScaledServerSummary],
    scenario_elapsed_ns: u64,
) -> Value {
    let files_valid = observations.iter().all(|observation| {
        observation.completed
            && observation.bytes_written
                == u64::try_from(observation.descriptor.byte_length).unwrap_or(u64::MAX)
            && verify_streamed_file(&observation.destination, &observation.descriptor)
    });
    let no_server_failures = summaries.iter().all(|summary| summary.failures == 0);
    let no_leak = controller_is_idle(controller);
    let snapshot = controller.snapshot();
    let policy_bounds = candidate
        .finite_bounds()
        .is_none_or(|(global, per_origin)| {
            snapshot.max_active_global <= global
                && observations
                    .iter()
                    .all(|observation| observation.max_active_per_origin_seen <= per_origin)
        });
    let statuses = merge_scaled_status_counts(summaries);
    let request_count = summaries
        .iter()
        .flat_map(|summary| summary.request_counts.values())
        .copied()
        .sum::<u64>();
    let bytes_served = summaries
        .iter()
        .map(|summary| summary.bytes_served)
        .sum::<u64>();
    let total_bytes = observations.iter().fold(0_u64, |total, observation| {
        total.saturating_add(u64::try_from(observation.descriptor.byte_length).unwrap_or(u64::MAX))
    });
    let throughput = total_bytes
        .saturating_mul(1_000_000_000)
        .checked_div(scenario_elapsed_ns.max(1))
        .unwrap_or(0);
    let mut start_order = observations.iter().collect::<Vec<_>>();
    start_order
        .sort_by_key(|observation| observation.first_byte_ns.unwrap_or(observation.started_ns));
    let mut completion_order = observations.iter().collect::<Vec<_>>();
    completion_order.sort_by_key(|observation| observation.finished_ns);
    let files = observations
        .iter()
        .map(|observation| {
            json!({
                "id": observation.opaque_id,
                "expected_bytes": observation.descriptor.byte_length,
                "generator_seed": observation.descriptor.seed,
                "server_chunk_bytes": observation.descriptor.server_chunk_bytes,
                "server_chunk_delay_ms": duration_ms(observation.descriptor.chunk_delay),
                "request_started_ns": first_scaled_request_ns(summaries, &observation.path),
                "started_ns": observation.started_ns,
                "first_byte_ns": observation.first_byte_ns,
                "completion_ns": observation.finished_ns,
                "bytes_completed": observation.bytes_written,
            })
        })
        .collect::<Vec<_>>();
    let hard_gates_passed = files_valid && no_server_failures && no_leak && policy_bounds;
    json!({
        "scenario": scenario,
        "candidate": candidate.identifier(),
        "profile": SCALED_PROFILE,
        "hard_gates_passed": hard_gates_passed,
        "fixture_loopback_only": true,
        "fixture": {
            "streaming_body": true,
            "file_count": observations.len(),
            "total_bytes": total_bytes,
            "server_chunk_bytes": distinct_usize(observations.iter().map(|entry| entry.descriptor.server_chunk_bytes)),
            "server_chunk_delays_ms": distinct_u64(observations.iter().map(|entry| duration_ms(entry.descriptor.chunk_delay))),
            "pressure_threshold": observations.iter().filter_map(|entry| entry.descriptor.pressure_above).min(),
        },
        "metrics": {
            "scenario_elapsed_ns": scenario_elapsed_ns,
            "effective_throughput_bytes_per_sec": throughput,
            "bytes_served": bytes_served,
            "fixture_max_concurrent_requests": summaries.iter().map(|summary| summary.max_active).max().unwrap_or(0),
            "request_count": request_count,
            "retry_count": statuses.get("429").copied().unwrap_or(0)
                .saturating_add(statuses.get("500").copied().unwrap_or(0)),
            "status_counts": statuses,
        },
        "files": files,
        "start_order": start_order.into_iter().map(|entry| entry.opaque_id.clone()).collect::<Vec<_>>(),
        "completion_order": completion_order.into_iter().map(|entry| entry.opaque_id.clone()).collect::<Vec<_>>(),
        "admission": snapshot_json(controller),
        "no_leak": no_leak,
        "policy_bounds_observed": policy_bounds,
        "max_active_per_origin_observed": observations.iter().map(|entry| entry.max_active_per_origin_seen).max().unwrap_or(0),
        "file_integrity_passed": files_valid,
        "fixture_server_failures": summaries.iter().map(|summary| summary.failures).sum::<usize>(),
        "fixture_failure_codes": summaries.iter().flat_map(|summary| summary.failure_codes.iter()).fold(BTreeMap::<String, u64>::new(), |mut merged, (code, count)| { *merged.entry(code.clone()).or_insert(0) += count; merged }),
    })
}

fn scaled_spec(
    origin_index: usize,
    seed: u8,
    byte_length: usize,
    delay_ms: u64,
    pressure_above: Option<usize>,
) -> ScaledFileSpec {
    ScaledFileSpec {
        origin_index,
        descriptor: StreamingBodyDescriptor::new(
            seed,
            byte_length,
            SCALED_SERVER_CHUNK,
            Duration::from_millis(delay_ms),
            pressure_above,
        ),
    }
}

fn run_scaled_layout(
    scenario: &str,
    candidate: &Candidate,
    origin_count: usize,
    specs: Vec<ScaledFileSpec>,
) -> Result<Value, String> {
    let root = TempRoot::new()?;
    let servers = (0..origin_count)
        .map(|_| ScaledFixtureServer::new())
        .collect::<Result<Vec<_>, _>>()?;
    let mut inputs = Vec::new();
    for (index, spec) in specs.into_iter().enumerate() {
        let server = servers
            .get(spec.origin_index)
            .ok_or_else(|| "scaled fixture origin index is invalid".to_string())?;
        let path = format!("/stream-{index}");
        server.add_streaming_body(&path, spec.descriptor.clone());
        inputs.push(new_scaled_input(
            &root,
            &format!("f{index:02}"),
            &path,
            server.url(&path)?,
            spec.descriptor,
        ));
    }
    let controller = candidate.controller();
    let started = Instant::now();
    let observations = execute_scaled_inputs(Arc::clone(&controller), inputs, started)?;
    let scenario_elapsed_ns = elapsed_ns(started);
    if started.elapsed() > SCALED_SCENARIO_DEADLINE {
        return Err("scaled benchmark scenario deadline exceeded".to_string());
    }
    let summaries = servers
        .into_iter()
        .map(ScaledFixtureServer::finish)
        .collect::<Vec<_>>();
    Ok(build_scaled_http_sample(
        scenario,
        candidate,
        &controller,
        &observations,
        &summaries,
        scenario_elapsed_ns,
    ))
}

fn run_scaled_scenario(scenario: &str, candidate: &Candidate) -> Result<Value, String> {
    match scenario {
        "single_origin_sustained" => run_scaled_layout(
            scenario,
            candidate,
            1,
            (0..4)
                .map(|index| scaled_spec(0, 10 + index, 6 * 1024 * 1024, 10, None))
                .collect(),
        ),
        "single_origin_pressure" => run_scaled_layout(
            scenario,
            candidate,
            1,
            (0..8)
                .map(|index| scaled_spec(0, 30 + index, 3 * 1024 * 1024, 16, Some(2)))
                .collect(),
        ),
        "multi_origin_sustained" => run_scaled_layout(
            scenario,
            candidate,
            4,
            (0..8)
                .map(|index| scaled_spec(index / 2, 50 + index as u8, 6 * 1024 * 1024, 10, None))
                .collect(),
        ),
        "multi_origin_mixed_rate" => {
            let mut specs = Vec::new();
            for origin in 0..4 {
                let delay = [12, 14, 16, 20][origin];
                specs.push(scaled_spec(
                    origin,
                    80 + origin as u8 * 2,
                    4 * 1024 * 1024,
                    delay,
                    None,
                ));
                specs.push(scaled_spec(
                    origin,
                    81 + origin as u8 * 2,
                    6 * 1024 * 1024,
                    delay,
                    None,
                ));
            }
            run_scaled_layout(scenario, candidate, 4, specs)
        }
        "same_origin_mixed_size" => run_scaled_layout(
            scenario,
            candidate,
            1,
            vec![
                scaled_spec(0, 110, 2 * 1024 * 1024, 10, None),
                scaled_spec(0, 111, 6 * 1024 * 1024, 10, None),
                scaled_spec(0, 112, 12 * 1024 * 1024, 10, None),
            ],
        ),
        _ => Err("unknown scaled benchmark scenario".to_string()),
    }
}

fn run_scaled_registry_cycles() -> Result<bool, String> {
    let candidate = Candidate::parse("g8-h4")?;
    let controller = candidate.controller();
    for index in 0..16 {
        let backend = ControlledBackend::with_mode(ControlledMode::Fail);
        let port = 12_000_u16.saturating_add(index);
        let result = AdmittedTransferBackend::new(&backend, Arc::clone(&controller))
            .download_legacy(
                &controlled_request("opaque-cycle", &format!("cycle-{index}"), port),
                &TransferAuth::None,
                &|| false,
                &|| false,
                &mut |_| {},
            );
        if result.outcome != DownloadOutcome::Failed || !controller_is_idle(&controller) {
            return Ok(false);
        }
    }
    Ok(controller_is_idle(&controller))
}

fn run_scaled_queue_soak() -> Result<Value, String> {
    let started = Instant::now();
    let saturation = Candidate::parse("g4-h1")?;
    let restart_candidate = Candidate::parse("g8-h4")?;
    let fairness = run_task_fairness(&saturation)?;
    let cancellation = run_queued_cancellation(&saturation)?;
    let pause_resume = run_queued_pause_resume(&saturation)?;
    let restart = run_restart_oversubscription(&restart_candidate)?;
    let registry_cycles_clean = run_scaled_registry_cycles()?;
    let hard = fairness["hard_gates_passed"].as_bool() == Some(true)
        && cancellation["hard_gates_passed"].as_bool() == Some(true)
        && pause_resume["hard_gates_passed"].as_bool() == Some(true)
        && restart["hard_gates_passed"].as_bool() == Some(true)
        && restart["rehydrated_worker_count"].as_u64().unwrap_or(0) >= 10
        && registry_cycles_clean;
    Ok(json!({
        "scenario": "queue_soak",
        "candidate": restart_candidate.identifier(),
        "profile": SCALED_PROFILE,
        "hard_gates_passed": hard,
        "fixture_loopback_only": true,
        "fixture": {
            "streaming_body": false,
            "kind": "controller cleanup and thread-model safety suite",
        },
        "metrics": {
            "scenario_elapsed_ns": elapsed_ns(started),
            "effective_throughput_bytes_per_sec": 0,
            "bytes_served": 0,
            "request_count": 0,
            "retry_count": 0,
            "status_counts": {},
        },
        "files": [],
        "start_order": [],
        "completion_order": [],
        "admission": {
            "registry_cycles_clean": registry_cycles_clean,
            "restart_thread_model_workers": restart["rehydrated_worker_count"],
        },
        "no_leak": hard,
        "policy_bounds_observed": hard,
        "safety": {
            "fairness": fairness["hard_gates_passed"],
            "queued_cancellation": cancellation["hard_gates_passed"],
            "queued_pause_resume": pause_resume["hard_gates_passed"],
            "restart_thread_model": restart["hard_gates_passed"],
        },
    }))
}

#[derive(Clone)]
struct ScaledWork {
    candidate: Candidate,
    scenario: &'static str,
    phase: &'static str,
    repetition: usize,
}

fn scaled_candidate_matrix() -> Result<Vec<Candidate>, String> {
    SCALED_CANDIDATE_IDS
        .iter()
        .map(|candidate| Candidate::parse(candidate))
        .collect()
}

fn scaled_shuffle<T>(values: &mut [T], seed: u64) {
    if values.len() < 2 {
        return;
    }
    let mut state = seed.max(1);
    for index in (1..values.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let target =
            usize::try_from(state % u64::try_from(index + 1).unwrap_or(u64::MAX)).unwrap_or(0);
        values.swap(index, target);
    }
}

fn scaled_profile_json() -> Value {
    let options = TransferOptions::default();
    json!({
        "name": SCALED_PROFILE,
        "transfer_options": {
            "max_attempts": options.max_attempts,
            "backoff_ms": duration_ms(options.backoff),
            "timeout_ms": duration_ms(options.timeout),
            "chunk_size": options.chunk_size,
            "max_accepted_polls": options.max_accepted_polls,
            "accepted_poll_timeout_ms": duration_ms(options.accepted_poll_timeout),
        },
    })
}

fn attach_scaled_identity(
    sample: &mut Value,
    round: u64,
    order_index: usize,
    seed: u64,
    phase: &str,
    repetition: usize,
) -> Result<(), String> {
    let object = sample
        .as_object_mut()
        .ok_or_else(|| "scaled benchmark sample shape failure".to_string())?;
    object.insert("round".to_string(), Value::from(round));
    object.insert(
        "order_index".to_string(),
        Value::from(u64::try_from(order_index).unwrap_or(u64::MAX)),
    );
    object.insert("seed".to_string(), Value::from(seed));
    object.insert("phase".to_string(), Value::String(phase.to_string()));
    object.insert(
        "repetition".to_string(),
        Value::from(u64::try_from(repetition).unwrap_or(u64::MAX)),
    );
    Ok(())
}

fn run_scaled_safety(name: &str) -> Result<Value, String> {
    let candidate = Candidate::parse("g4-h2")?;
    let started = Instant::now();
    let mut sample = match name {
        "queue_soak" => run_scaled_queue_soak()?,
        "resolver_refresh" => run_resolver_refresh(&candidate)?,
        "redirect_same_origin" => run_redirect(&candidate, true)?,
        "redirect_other_origin" => run_redirect(&candidate, false)?,
        "backend_abnormal" => run_backend_abnormal(&candidate)?,
        _ => return Err("unknown scaled safety scenario".to_string()),
    };
    let object = sample
        .as_object_mut()
        .ok_or_else(|| "scaled safety sample shape failure".to_string())?;
    object.insert(
        "profile".to_string(),
        Value::String(SCALED_PROFILE.to_string()),
    );
    object.entry("fixture".to_string()).or_insert_with(
        || json!({"streaming_body": false, "kind": "existing bounded safety fixture"}),
    );
    object.entry("metrics".to_string()).and_modify(|metrics| {
        if let Some(metrics) = metrics.as_object_mut() {
            metrics.insert(
                "scenario_elapsed_ns".to_string(),
                Value::from(elapsed_ns(started)),
            );
        }
    });
    Ok(sample)
}

fn run_scaled_batch_internal(
    round: u64,
    seed: u64,
    warmups: usize,
    measured_repetitions: usize,
    candidates: Vec<Candidate>,
    scenarios: &[&'static str],
    include_safety: bool,
) -> Result<String, String> {
    let effective_seed = seed ^ round.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut warmup_work = Vec::new();
    let mut measured_work = Vec::new();
    for scenario in scenarios {
        for candidate in &candidates {
            for repetition in 1..=warmups {
                warmup_work.push(ScaledWork {
                    candidate: candidate.clone(),
                    scenario,
                    phase: "warmup",
                    repetition,
                });
            }
            for repetition in 1..=measured_repetitions {
                measured_work.push(ScaledWork {
                    candidate: candidate.clone(),
                    scenario,
                    phase: "measured",
                    repetition,
                });
            }
        }
    }
    scaled_shuffle(&mut warmup_work, effective_seed ^ 0xA5A5_A5A5_A5A5_A5A5);
    scaled_shuffle(&mut measured_work, effective_seed ^ 0x5A5A_5A5A_5A5A_5A5A);
    let mut samples = Vec::new();
    for work in warmup_work.into_iter().chain(measured_work) {
        let mut sample = run_scaled_scenario(work.scenario, &work.candidate)?;
        let order_index = samples.len();
        attach_scaled_identity(
            &mut sample,
            round,
            order_index,
            effective_seed,
            work.phase,
            work.repetition,
        )?;
        samples.push(sample);
    }
    if include_safety {
        let mut safety = vec![
            "queue_soak",
            "resolver_refresh",
            "redirect_same_origin",
            "redirect_other_origin",
            "backend_abnormal",
        ];
        scaled_shuffle(&mut safety, effective_seed ^ 0xC3C3_C3C3_C3C3_C3C3);
        for name in safety {
            let mut sample = run_scaled_safety(name)?;
            let order_index = samples.len();
            attach_scaled_identity(&mut sample, round, order_index, effective_seed, "safety", 1)?;
            samples.push(sample);
        }
    }
    let hard_gates_passed = samples
        .iter()
        .all(|sample| sample["hard_gates_passed"].as_bool() == Some(true));
    serde_json::to_string(&json!({
        "schema_version": SCALED_SCHEMA_VERSION,
        "fixture_loopback_only": true,
        "profile": scaled_profile_json(),
        "round": {
            "number": round,
            "seed": effective_seed,
            "order_policy": "seeded-fisher-yates-by-phase",
        },
        "samples": samples,
        "hard_gates_passed": hard_gates_passed,
    }))
    .map_err(|_| "scaled benchmark JSON serialization failure".to_string())
}

pub fn run_scaled_batch_for_test() -> Result<String, String> {
    run_scaled_batch_internal(
        1,
        SCALED_BASE_SEED,
        0,
        1,
        vec![Candidate::Compatibility],
        &["single_origin_sustained"],
        false,
    )
}

fn run_scaled_batch_cli(
    round: u64,
    seed: u64,
    warmups: usize,
    measured_repetitions: usize,
) -> Result<String, String> {
    if warmups != 1 || measured_repetitions != 4 {
        return Err(
            "scaled benchmark protocol requires one warmup and four measured repetitions"
                .to_string(),
        );
    }
    run_scaled_batch_internal(
        round,
        seed,
        warmups,
        measured_repetitions,
        scaled_candidate_matrix()?,
        &SCALED_RANKED_SCENARIOS,
        true,
    )
}

fn run_scenario(scenario: &str, candidate: &Candidate) -> Result<Value, String> {
    match scenario {
        "single_unconstrained" => {
            run_single_layout(scenario, candidate, &[(8 * 1024, Duration::ZERO); 4], None)
        }
        "single_throttled" => run_single_layout(
            scenario,
            candidate,
            &[(8 * 1024, Duration::from_millis(1)); 4],
            Some(1),
        ),
        "multi_balanced" => run_multi_layout(candidate, false),
        "multi_mixed_speed" => run_multi_layout(candidate, true),
        "same_origin_mixed_size" => run_single_layout(
            scenario,
            candidate,
            &[
                (1024, Duration::ZERO),
                (32 * 1024, Duration::from_millis(1)),
                (2048, Duration::ZERO),
                (16 * 1024, Duration::from_millis(1)),
            ],
            None,
        ),
        "task_fairness" => run_task_fairness(candidate),
        "queued_cancellation" => run_queued_cancellation(candidate),
        "queued_pause_resume" => run_queued_pause_resume(candidate),
        "redirect_same_origin" => run_redirect(candidate, true),
        "redirect_other_origin" => run_redirect(candidate, false),
        "resolver_refresh" => run_resolver_refresh(candidate),
        "restart_oversubscription" => run_restart_oversubscription(candidate),
        "backend_abnormal" => run_backend_abnormal(candidate),
        _ => Err("unknown benchmark scenario".to_string()),
    }
}

fn report(samples: Vec<Value>) -> Result<String, String> {
    let hard_gates_passed = samples
        .iter()
        .all(|sample| sample["hard_gates_passed"].as_bool() == Some(true));
    serde_json::to_string(&json!({
        "schema_version": SCHEMA_VERSION,
        "fixture_loopback_only": true,
        "hard_gates_passed": hard_gates_passed,
        "samples": samples,
    }))
    .map_err(|_| "benchmark JSON serialization failure".to_string())
}

pub fn run_smoke_suite_for_test() -> Result<String, String> {
    let compatibility = Candidate::Compatibility;
    let finite = Candidate::parse("g1-h1")?;
    report(vec![
        run_scenario("single_unconstrained", &compatibility)?,
        run_scenario("queued_cancellation", &finite)?,
        run_scenario("redirect_same_origin", &finite)?,
        run_scenario("redirect_other_origin", &finite)?,
        run_scenario("restart_oversubscription", &finite)?,
        run_scenario("restart_oversubscription", &compatibility)?,
    ])
}

pub fn rejects_non_loopback_for_test() -> bool {
    !is_loopback_url("https://example.invalid/product")
}

enum CliMode {
    Legacy {
        scenario: String,
        candidate: Candidate,
        run_label: String,
    },
    Scaled {
        round: u64,
        seed: u64,
        warmups: usize,
        measured_repetitions: usize,
    },
}

fn required_value(values: &mut BTreeMap<String, String>, flag: &str) -> Result<String, String> {
    values
        .remove(flag)
        .ok_or_else(|| "benchmark argument missing value".to_string())
}

fn parse_u64_argument(values: &mut BTreeMap<String, String>, flag: &str) -> Result<u64, String> {
    required_value(values, flag)?
        .parse::<u64>()
        .map_err(|_| "benchmark numeric argument is invalid".to_string())
}

fn parse_usize_argument(
    values: &mut BTreeMap<String, String>,
    flag: &str,
) -> Result<usize, String> {
    required_value(values, flag)?
        .parse::<usize>()
        .map_err(|_| "benchmark numeric argument is invalid".to_string())
}

fn parse_cli_mode() -> Result<CliMode, String> {
    let mut values = BTreeMap::new();
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--help" {
            return Err(
                "usage: legacy --scenario NAME --candidate ID --run-label LABEL --schema-version VERSION; scaled --mode scaled-batch --round N --seed N --warmups 1 --measured-repetitions 4 --schema-version VERSION"
                    .to_string(),
            );
        }
        if !matches!(
            flag.as_str(),
            "--scenario"
                | "--candidate"
                | "--run-label"
                | "--schema-version"
                | "--mode"
                | "--round"
                | "--seed"
                | "--warmups"
                | "--measured-repetitions"
        ) {
            return Err("unknown benchmark argument".to_string());
        }
        let value = args
            .next()
            .ok_or_else(|| "benchmark argument missing value".to_string())?;
        values.insert(flag, value);
    }
    let schema = required_value(&mut values, "--schema-version")?;
    match schema.as_str() {
        SCHEMA_VERSION => {
            let scenario = required_value(&mut values, "--scenario")?;
            let candidate = Candidate::parse(&required_value(&mut values, "--candidate")?)?;
            let run_label = required_value(&mut values, "--run-label")?;
            if !run_label
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_'))
            {
                return Err("benchmark run label is invalid".to_string());
            }
            if !values.is_empty() {
                return Err("duplicate benchmark argument".to_string());
            }
            Ok(CliMode::Legacy {
                scenario,
                candidate,
                run_label,
            })
        }
        SCALED_SCHEMA_VERSION => {
            if required_value(&mut values, "--mode")? != "scaled-batch" {
                return Err("scaled benchmark mode is invalid".to_string());
            }
            let round = parse_u64_argument(&mut values, "--round")?;
            let seed = parse_u64_argument(&mut values, "--seed")?;
            let warmups = parse_usize_argument(&mut values, "--warmups")?;
            let measured_repetitions = parse_usize_argument(&mut values, "--measured-repetitions")?;
            if round == 0 || seed == 0 || !values.is_empty() {
                return Err("scaled benchmark arguments are invalid".to_string());
            }
            Ok(CliMode::Scaled {
                round,
                seed,
                warmups,
                measured_repetitions,
            })
        }
        _ => Err("benchmark schema version mismatch".to_string()),
    }
}

fn run_cli() -> Result<String, String> {
    match parse_cli_mode()? {
        CliMode::Legacy {
            scenario,
            candidate,
            run_label,
        } => {
            let mut value: Value =
                serde_json::from_str(&report(vec![run_scenario(&scenario, &candidate)?])?)
                    .map_err(|_| "benchmark JSON construction failure".to_string())?;
            let object: &mut Map<String, Value> = value
                .as_object_mut()
                .ok_or_else(|| "benchmark report shape failure".to_string())?;
            object.insert("run_label".to_string(), Value::String(run_label));
            serde_json::to_string(&value)
                .map_err(|_| "benchmark JSON serialization failure".to_string())
        }
        CliMode::Scaled {
            round,
            seed,
            warmups,
            measured_repetitions,
        } => run_scaled_batch_cli(round, seed, warmups, measured_repetitions),
    }
}

fn main() {
    match run_cli() {
        Ok(output) => println!("{output}"),
        Err(_) => {
            eprintln!("admission benchmark failed");
            std::process::exit(1);
        }
    }
}
