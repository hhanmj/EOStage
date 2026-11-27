use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, DownloadLayout, FrozenSearchBatch,
    SearchWorkbenchState, SnapshotItem, StableAssetReference, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskStateCommandKind,
};
use insar_download::{
    AssetResolver, AssetResolverRegistry, Clock, CredentialMaterial, CredentialProvider,
    CredentialProviderError, DownloadScheduler, DownloadWorker, HttpTransfer,
    ProductionDownloadWorker, ResolveContext, ResolveError, ResolvedAsset, ResolvedChecksum,
    ResolvedMethod, ResolverExecutionRuntime, ShadowAccessIntent, ShadowResolveContext,
    TransferBackend, TransferOptions, EARTHDATA_CREDENTIAL_PROFILE,
};
use insar_store::{StoreTaskSummary, TaskStore};
use serde_json::{json, Map};
use sha2::{Digest, Sha256};

const TASK_ID: &str = "telemetry-production";
const ITEM_ID: &str = "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::fixture-product";

struct TempFixture {
    root: PathBuf,
    database: PathBuf,
    output: PathBuf,
}

impl TempFixture {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "insar-production-telemetry-{}-{nonce}-{seq}",
            std::process::id()
        ));
        let output = root.join("output");
        fs::create_dir_all(&output).expect("create fixture output");
        Self {
            database: root.join("task-store.sqlite3"),
            root,
            output,
        }
    }

    fn database(&self) -> &Path {
        &self.database
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Clone, Default)]
struct PauseResumeStreamGates {
    pause_confirmed: Arc<AtomicBool>,
    resumed_body: Arc<AtomicBool>,
    completion: Arc<AtomicBool>,
}

struct StreamingServer {
    url: String,
    total_bytes: u64,
    checksum: String,
    stop: Arc<AtomicBool>,
    wake_address: SocketAddr,
    pause_resume_gates: Option<PauseResumeStreamGates>,
    worker: Option<thread::JoinHandle<()>>,
}

impl StreamingServer {
    fn new(chunk_count: usize, chunk_size: usize, chunk_delay: Duration) -> Self {
        Self::start(chunk_count, chunk_size, chunk_delay, None)
    }

    fn new_pause_resume(chunk_count: usize, chunk_size: usize, chunk_delay: Duration) -> Self {
        Self::start(
            chunk_count,
            chunk_size,
            chunk_delay,
            Some(PauseResumeStreamGates::default()),
        )
    }

    fn start(
        chunk_count: usize,
        chunk_size: usize,
        chunk_delay: Duration,
        pause_resume_gates: Option<PauseResumeStreamGates>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local HTTP fixture");
        listener
            .set_nonblocking(true)
            .expect("set fixture listener nonblocking");
        let address = listener.local_addr().expect("fixture address");
        let chunks = (0..chunk_count)
            .map(|index| {
                let byte = b'a' + u8::try_from(index % 26).expect("fixture alphabet index");
                vec![byte; chunk_size]
            })
            .collect::<Vec<_>>();
        let total_bytes = chunks.iter().try_fold(0_u64, |total, chunk| {
            total.checked_add(u64::try_from(chunk.len()).ok()?)
        });
        let total_bytes = total_bytes.expect("fixture byte count");
        let mut digest = Sha256::new();
        for chunk in &chunks {
            digest.update(chunk);
        }
        let checksum = format!("{:x}", digest.finalize());
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_pause_resume_gates = pause_resume_gates.clone();
        let worker = thread::spawn(move || {
            let mut stream = loop {
                if worker_stop.load(Ordering::Acquire) {
                    return;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        if worker_stop.load(Ordering::Acquire) {
                            return;
                        }
                        break stream;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return,
                }
            };
            if stream.set_nonblocking(false).is_err()
                || stream
                    .set_read_timeout(Some(Duration::from_millis(50)))
                    .is_err()
                || stream
                    .set_write_timeout(Some(Duration::from_millis(50)))
                    .is_err()
                || !read_request(&mut stream, worker_stop.as_ref())
                || worker_stop.load(Ordering::Acquire)
            {
                return;
            }
            if write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {total_bytes}\r\nConnection: close\r\n\r\n"
            )
            .and_then(|()| stream.flush())
            .is_err()
            {
                return;
            }
            for chunk in chunks {
                if let Some(gates) = worker_pause_resume_gates.as_ref() {
                    if gates.pause_confirmed.load(Ordering::Acquire)
                        && !wait_for_fixture_gate(gates.resumed_body.as_ref(), worker_stop.as_ref())
                    {
                        return;
                    }
                }
                if worker_stop.load(Ordering::Acquire)
                    || stream.write_all(&chunk).is_err()
                    || stream.flush().is_err()
                {
                    break;
                }
                let completion_released = worker_pause_resume_gates
                    .as_ref()
                    .is_some_and(|gates| gates.completion.load(Ordering::Acquire));
                if !completion_released && wait_for_stop(worker_stop.as_ref(), chunk_delay) {
                    break;
                }
            }
        });
        Self {
            url: format!("http://{address}/telemetry.bin"),
            total_bytes,
            checksum,
            stop,
            wake_address: address,
            pause_resume_gates,
            worker: Some(worker),
        }
    }

    fn hold_body_until_resume(&self) {
        self.pause_resume_gates
            .as_ref()
            .expect("pause/resume fixture gates")
            .pause_confirmed
            .store(true, Ordering::Release);
    }

    fn release_resumed_body(&self) {
        self.pause_resume_gates
            .as_ref()
            .expect("pause/resume fixture gates")
            .resumed_body
            .store(true, Ordering::Release);
    }

    fn release_completion(&self) {
        self.pause_resume_gates
            .as_ref()
            .expect("pause/resume fixture gates")
            .completion
            .store(true, Ordering::Release);
    }

    fn finish(mut self) {
        self.signal_stop();
        self.worker.take().expect("fixture worker").join().unwrap();
    }

    fn signal_stop(&self) {
        self.stop.store(true, Ordering::Release);
        if let Some(gates) = self.pause_resume_gates.as_ref() {
            gates.pause_confirmed.store(true, Ordering::Release);
            gates.resumed_body.store(true, Ordering::Release);
            gates.completion.store(true, Ordering::Release);
        }
        let _ = TcpStream::connect_timeout(&self.wake_address, Duration::from_millis(50));
    }
}

impl Drop for StreamingServer {
    fn drop(&mut self) {
        self.signal_stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_request(stream: &mut TcpStream, stop: &AtomicBool) -> bool {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return false,
            Ok(_) if line == "\r\n" => return true,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if stop.load(Ordering::Acquire) {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
}

fn wait_for_fixture_gate(gate: &AtomicBool, stop: &AtomicBool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !gate.load(Ordering::Acquire) {
        if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
    !stop.load(Ordering::Acquire)
}

fn wait_for_stop(stop: &AtomicBool, duration: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < duration {
        if stop.load(Ordering::Acquire) {
            return true;
        }
        thread::sleep(
            duration
                .saturating_sub(started.elapsed())
                .min(Duration::from_millis(10)),
        );
    }
    stop.load(Ordering::Acquire)
}

struct FixtureClock;

impl Clock for FixtureClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

struct FixtureCredentials;

impl CredentialProvider for FixtureCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        if profile != EARTHDATA_CREDENTIAL_PROFILE {
            return Err(CredentialProviderError::Missing);
        }
        CredentialMaterial::bearer("FIXTURE_TOKEN_PRODUCTION_TELEMETRY")
            .map_err(|_| CredentialProviderError::Failed)
    }
}

struct LocalHttpResolver {
    url: String,
    total_bytes: u64,
    checksum: String,
}

impl AssetResolver for LocalHttpResolver {
    fn resolver_id(&self) -> &'static str {
        "earthdata_https"
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::EarthdataHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.scheme == AssetAccessScheme::EarthdataHttps
    }

    fn resolve(
        &self,
        _reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        context.check_cancelled()?;
        let profile = descriptor
            .credential_profile
            .as_deref()
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let credential = context
            .credential_provider()
            .resolve(profile)
            .map_err(|_| ResolveError::credentials_missing())?;
        let mut asset = ResolvedAsset::new(&self.url, ResolvedMethod::Get)?
            .with_suggested_filename(Some("telemetry.bin".to_owned()))
            .with_expected_size(Some(self.total_bytes))
            .with_checksum(Some(ResolvedChecksum::new("sha256", &self.checksum)?));
        credential.apply_to(&mut asset)?;
        Ok(asset)
    }

    fn plan_shadow(
        &self,
        _frozen: &insar_core::FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::failed())
    }
}

fn resolver_runtime(server: &StreamingServer) -> ResolverExecutionRuntime {
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(LocalHttpResolver {
            url: server.url.clone(),
            total_bytes: server.total_bytes,
            checksum: server.checksum.clone(),
        }))
        .expect("register local production resolver");
    ResolverExecutionRuntime::with_dependencies(
        registry,
        Arc::new(FixtureCredentials),
        Arc::new(FixtureClock),
    )
}

fn seed_task(database: &Path, output: &Path, total_bytes: u64, checksum: &str) {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asset_execution_policy/sentinel1_standard_cases.json");
    let mut item = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(fixture_path).expect("read resolver policy fixture"),
    )
    .expect("parse resolver policy fixture")["base_item"]
        .clone();
    item["display_name"] = json!("telemetry.bin");
    item["asset_access"]["file_name"] = json!("telemetry.bin");
    item["asset_access"]["expected_size_bytes"] = json!(total_bytes);
    item["asset_access"]["checksum_value"] = json!(checksum);
    let item: SnapshotItem = serde_json::from_value(item).expect("production resolver item");
    let mut store = TaskStore::open(database).expect("open production telemetry store");
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "telemetry-production-batch".to_owned(),
                provider_id: "asf.catalog".to_owned(),
                created_at: "2026-08-20T00:00:00Z".to_owned(),
                aoi_name: "Telemetry Fixture".to_owned(),
                aoi_geojson: Some(
                    json!({
                        "type": "Polygon",
                        "coordinates": [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]]
                    })
                    .as_object()
                    .expect("fixture polygon")
                    .clone(),
                ),
                query: Map::new(),
                search_stats: None,
                candidates: vec![item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "telemetry-production-batch".to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 100,
                revision: 0,
            },
        })
        .expect("save production telemetry batch");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: TASK_ID.to_owned(),
            batch_id: "telemetry-production-batch".to_owned(),
            selected_item_ids: vec![ITEM_ID.to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 100,
            output_root: output.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-20T00:00:01Z".to_owned(),
        })
        .expect("create production telemetry task");
}

fn restored_summary(database: &Path) -> StoreTaskSummary {
    TaskStore::open(database)
        .expect("open telemetry store")
        .core_restore(false)
        .expect("restore telemetry store")
        .tasks
        .into_iter()
        .find(|task| task.task_id == TASK_ID)
        .expect("production telemetry summary")
}

fn wait_until(label: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("telemetry condition did not become true: {label}");
}

fn wait_for_initial_telemetry(database: &Path) -> StoreTaskSummary {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = None;
    while Instant::now() < deadline {
        let task = restored_summary(database);
        if task.transferred_bytes > 0
            && task.bytes_per_second > 0.0
            && task.transfer_active_millis > 0
        {
            return task;
        }
        if matches!(
            task.status,
            TaskLifecycleStatus::Completed
                | TaskLifecycleStatus::Failed
                | TaskLifecycleStatus::Cancelled
        ) {
            panic!(
                "production transfer became terminal before telemetry was durable: status={:?}, bytes={}, rate={}, active_millis={}",
                task.status,
                task.transferred_bytes,
                task.bytes_per_second,
                task.transfer_active_millis,
            );
        }
        last = Some((
            task.status,
            task.transferred_bytes,
            task.bytes_per_second,
            task.transfer_active_millis,
        ));
        thread::sleep(Duration::from_millis(20));
    }
    panic!("real bytes/rate were visible but the transfer clock was not durable: {last:?}");
}

fn apply_task_command(database: &Path, kind: TaskStateCommandKind, event_id: &str) -> (u64, u64) {
    let mut store = TaskStore::open(database).expect("open store for task command");
    let task = store
        .load_task(TASK_ID)
        .expect("load production task")
        .expect("production task exists");
    let expected_revision = task.runtime.revision;
    let updated = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: TASK_ID.to_owned(),
            expected_revision,
            command: StoreTaskCommand {
                kind,
                item_ids: vec![],
            },
            event_id: event_id.to_owned(),
            occurred_at: "2026-08-20T00:00:02Z".to_owned(),
        })
        .expect("apply production task command");
    (expected_revision, updated.runtime.revision)
}

fn wait_for_resume_activation(database: &Path, paused: &StoreTaskSummary) -> StoreTaskSummary {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = None;
    while Instant::now() < deadline {
        let task = restored_summary(database);
        if task.status == TaskLifecycleStatus::Running
            && task.active_count == 1
            && task.paused_count == 0
            && task.transfer_active_millis > paused.transfer_active_millis
        {
            return task;
        }
        if matches!(
            task.status,
            TaskLifecycleStatus::Completed
                | TaskLifecycleStatus::Failed
                | TaskLifecycleStatus::Cancelled
        ) {
            panic!(
                "production transfer became terminal before resume activation: status={:?}, bytes={}, rate={}, active_millis={}",
                task.status,
                task.transferred_bytes,
                task.bytes_per_second,
                task.transfer_active_millis,
            );
        }
        last = Some((
            task.revision,
            task.status,
            task.active_count,
            task.paused_count,
            task.transfer_active_millis,
        ));
        thread::sleep(Duration::from_millis(20));
    }
    panic!("scheduler did not reactivate the paused production body: {last:?}");
}

fn wait_for_resumed_telemetry(
    database: &Path,
    before_pause: &StoreTaskSummary,
    paused: &StoreTaskSummary,
    after_resume: StoreTaskSummary,
    pause_revisions: (u64, u64),
    resume_revisions: (u64, u64),
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = after_resume;
    while Instant::now() < deadline {
        if last.transfer_active_millis > paused.transfer_active_millis
            && last.transferred_bytes > paused.transferred_bytes
            && last.bytes_per_second > 0.0
        {
            return;
        }
        thread::sleep(Duration::from_millis(20));
        last = restored_summary(database);
    }
    let scheduler_task = TaskStore::open(database)
        .expect("open telemetry store for scheduler diagnostic")
        .load_task(TASK_ID)
        .expect("load task diagnostic")
        .expect("task diagnostic exists");
    panic!(
        "resumed telemetry did not advance: before_pause=(revision={},status={:?},bytes={},active_millis={},rate={},active={},paused={}); pause_command={:?}; after_pause=(revision={},status={:?},bytes={},active_millis={},rate={},active={},paused={}); resume_command={:?}; timeout=(revision={},status={:?},bytes={},active_millis={},rate={},active={},paused={},queued={},completed={}); advanced=(active_millis={},bytes={},positive_rate={}); scheduler_runtime=(revision={},status={:?},active_items={:?},paused_items={:?},queued_items={:?})",
        before_pause.revision,
        before_pause.status,
        before_pause.transferred_bytes,
        before_pause.transfer_active_millis,
        before_pause.bytes_per_second,
        before_pause.active_count,
        before_pause.paused_count,
        pause_revisions,
        paused.revision,
        paused.status,
        paused.transferred_bytes,
        paused.transfer_active_millis,
        paused.bytes_per_second,
        paused.active_count,
        paused.paused_count,
        resume_revisions,
        last.revision,
        last.status,
        last.transferred_bytes,
        last.transfer_active_millis,
        last.bytes_per_second,
        last.active_count,
        last.paused_count,
        last.queued_count,
        last.completed_count,
        last.transfer_active_millis > paused.transfer_active_millis,
        last.transferred_bytes > paused.transferred_bytes,
        last.bytes_per_second > 0.0,
        scheduler_task.runtime.revision,
        scheduler_task.runtime.status,
        scheduler_task.runtime.active_item_ids,
        scheduler_task.runtime.paused_item_ids,
        scheduler_task.runtime.queued_item_ids,
    );
}

fn wait_for_frozen_transfer_time(database: &Path, worker: &dyn DownloadWorker) -> StoreTaskSummary {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_sample = None;
    while Instant::now() < deadline {
        if let Some(sample) = worker.task_transfer_sample(TASK_ID) {
            last_sample = Some(sample);
            if sample.active_count == 0 {
                let persistence_deadline = Instant::now() + Duration::from_secs(5);
                let mut last = restored_summary(database);
                while Instant::now() < persistence_deadline {
                    last = restored_summary(database);
                    if last.transfer_active_millis >= sample.active_millis
                        && last.transferred_bytes >= sample.network_bytes_received
                    {
                        return last;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                panic!(
                    "paused worker activity froze but durable telemetry did not catch up: worker={sample:?}, durable=(bytes={},active_millis={},rate={})",
                    last.transferred_bytes,
                    last.transfer_active_millis,
                    last.bytes_per_second,
                );
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("paused worker activity did not freeze: {last_sample:?}");
}

#[test]
fn native_production_telemetry_survives_pause_resume_and_reopen() {
    let fixture = TempFixture::new();
    let server = StreamingServer::new_pause_resume(256, 16 * 1024, Duration::from_millis(60));
    let total_bytes = server.total_bytes;
    seed_task(
        fixture.database(),
        &fixture.output,
        server.total_bytes,
        &server.checksum,
    );
    let options = TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(3),
        chunk_size: 16 * 1024,
        ..TransferOptions::default()
    };
    let native: Arc<dyn TransferBackend> =
        Arc::new(HttpTransfer::new(options).expect("native single-stream backend"));
    let worker = Arc::new(
        ProductionDownloadWorker::new(options)
            .expect("production worker")
            .with_resolver_runtime_and_transfer(resolver_runtime(&server), native),
    );
    let scheduler_worker: Arc<dyn DownloadWorker> = worker.clone();
    let scheduler = DownloadScheduler::start(
        fixture.database(),
        scheduler_worker,
        Duration::from_millis(20),
    )
    .expect("production scheduler");
    scheduler.wake();

    let before_pause = wait_for_initial_telemetry(fixture.database());

    let pause_revisions = apply_task_command(
        fixture.database(),
        TaskStateCommandKind::PauseTask,
        "pause-production-telemetry",
    );
    scheduler.wake();
    let paused = wait_for_frozen_transfer_time(fixture.database(), worker.as_ref());
    server.hold_body_until_resume();
    assert_eq!(paused.status, TaskLifecycleStatus::Paused);
    assert_eq!(paused.active_count, 0);
    assert_eq!(paused.paused_count, 1);
    assert_eq!(paused.bytes_per_second, 0.0);
    thread::sleep(Duration::from_millis(250));
    let paused_again = restored_summary(fixture.database());
    assert_eq!(
        paused.transfer_active_millis,
        paused_again.transfer_active_millis
    );
    assert_eq!(
        paused.transferred_bytes, paused_again.transferred_bytes,
        "paused native body must not continue advancing durable bytes"
    );

    let resume_revisions = apply_task_command(
        fixture.database(),
        TaskStateCommandKind::ResumeTask,
        "resume-production-telemetry",
    );
    scheduler.wake();
    let after_resume = wait_for_resume_activation(fixture.database(), &paused);
    server.release_resumed_body();
    wait_for_resumed_telemetry(
        fixture.database(),
        &before_pause,
        &paused,
        after_resume,
        pause_revisions,
        resume_revisions,
    );
    server.release_completion();
    wait_until("native production download completes", || {
        restored_summary(fixture.database()).status == TaskLifecycleStatus::Completed
    });
    scheduler.shutdown();
    server.finish();

    let reopened = restored_summary(fixture.database());
    assert_eq!(reopened.status, TaskLifecycleStatus::Completed);
    assert_eq!(reopened.transferred_bytes, total_bytes);
    assert!(reopened.transfer_active_millis > paused.transfer_active_millis);
    assert_eq!(reopened.bytes_per_second, 0.0);
}

#[test]
fn loopback_fixture_shutdown_is_bounded_when_a_client_stalls_mid_request() {
    let server = StreamingServer::new(1, 1, Duration::ZERO);
    let address = server
        .url
        .strip_prefix("http://")
        .and_then(|value| value.split('/').next())
        .expect("fixture socket address")
        .to_owned();
    let mut client = TcpStream::connect(address).expect("connect stalled fixture client");
    client
        .write_all(b"GET /telemetry.bin HTTP/1.1\r\nHost: fixture\r\n")
        .expect("write partial request");
    client.flush().expect("flush partial request");
    let stalled_client = thread::spawn(move || {
        thread::sleep(Duration::from_secs(2));
        drop(client);
    });

    let started = Instant::now();
    let shutdown = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| server.finish()));
    let elapsed = started.elapsed();
    stalled_client.join().expect("stalled client cleanup");

    assert!(shutdown.is_ok(), "fixture shutdown worker panicked");
    assert!(
        elapsed < Duration::from_millis(500),
        "fixture shutdown blocked on a stalled client for {elapsed:?}"
    );
}
